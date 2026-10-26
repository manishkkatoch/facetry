//! Transitions between two 2D scenes: what enters, what moves and what leaves.
//!
//! [`Plan::diff`] compares the scene on screen (`from`) with the one that replaces it (`to`) by
//! **key**. A key only in `to` is an *enter*, in both an *update*, only in `from` an *exit*.
//! Every item then becomes a [`Pair`]: the state it starts in and the state it ends in. An
//! entering bar starts as a zero-height bar on its baseline, an exiting one ends as one; an
//! entering label starts as the same label with no opacity. So whatever kind of item it is, the
//! frame at time `t` is just `morph(pair.from, pair.to, ease(t))`, which is what
//! [`Plan::sample`] computes on the CPU and what the GPU does for the instanced marks (it is
//! handed the `from` and `to` of each pair and mixes them in the vertex shader).
//!
//! **Interruption.** [`Plan::sample`] is also how an update that arrives mid-transition avoids
//! a jump: the sampled scene (the state on screen right now, exits included) is the `from` of
//! the next diff.
//!
//! Pure and deterministic: no clock, no GPU, which is why it is unit-tested natively.

use std::collections::HashMap;

use crate::Color;
use crate::anim::{lerp, lerp_bounds, lerp_color};
use crate::scene::{AxisMap, MarkLabel, Rect, Rule, Scene, Text, TickRef};
use crate::spec::Timing;

/// One item's journey: the state it starts in, the state it ends in, and how long after the
/// transition begins it starts (the `stagger`).
#[derive(Debug, Clone, PartialEq)]
pub struct Pair<T> {
    pub from: T,
    pub to: T,
    pub delay_ms: f32,
    pub phase: Phase,
}

/// Which of the three things happens to an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Only in the new scene.
    Enter,
    /// In both.
    Update,
    /// Only in the old scene; it is removed when the transition ends.
    Exit,
}

/// Suffix of the key an old text gets when its string changed under the same key: it fades out
/// as its own item while the new one fades in.
const OLD_SUFFIX: &str = "\u{1}old";

/// A transition between two scenes. Items are listed in the new scene's order followed by the
/// exits, so index `i` of `rects` is bar `i` of `target` for `i < target.rects.len()` (hover
/// and click address the target).
#[derive(Debug, Clone)]
pub struct Plan {
    pub timing: Timing,
    /// Where the transition ends: what picking, snapshots and the next layout see.
    pub target: Scene,
    pub rects: Vec<Pair<Rect>>,
    pub rules: Vec<Pair<Rule>>,
    pub texts: Vec<Pair<Text>>,
    pub labels: Vec<Pair<MarkLabel>>,
    /// The old scene's axes, to interpolate the axes of a sampled scene.
    from_axes: Vec<AxisMap>,
    max_delay_ms: f32,
}

/// What happened to the item at an index.
enum Change {
    Update { old: usize, new: usize },
    Enter { new: usize },
    Exit { old: usize },
}

/// Matches `old` and `new` by key: every new item (in order) is an update or an enter, then
/// every unmatched old item is an exit. An old item is matched at most once, so duplicate keys
/// cannot make one item move to two places.
fn diff_keys<T>(old: &[T], new: &[T], key: impl Fn(&T) -> &str) -> Vec<Change> {
    let mut index: HashMap<&str, usize> = HashMap::with_capacity(old.len());
    for (i, item) in old.iter().enumerate() {
        index.entry(key(item)).or_insert(i);
    }
    let mut used = vec![false; old.len()];
    let mut out = Vec::with_capacity(new.len());
    for (n, item) in new.iter().enumerate() {
        match index.get(key(item)).copied().filter(|&o| !used[o]) {
            Some(o) => {
                used[o] = true;
                out.push(Change::Update { old: o, new: n });
            }
            None => out.push(Change::Enter { new: n }),
        }
    }
    out.extend(
        used.iter()
            .enumerate()
            .filter(|&(_, &u)| !u)
            .map(|(old, _)| Change::Exit { old }),
    );
    out
}

fn transparent(c: Color) -> Color {
    Color { a: 0.0, ..c }
}

/// The axis of `scene` that `tick` refers to, with the one of the same id in `other`: the
/// scale that puts the tick where it is, and the scale that would put it somewhere else.
fn matching_axes<'a>(
    tick: TickRef,
    scene: &'a Scene,
    other: &'a Scene,
) -> Option<(&'a AxisMap, &'a AxisMap)> {
    let own = scene.axes.get(tick.axis)?;
    let theirs = other
        .axes
        .iter()
        .find(|a| a.id == own.id && a.vertical == own.vertical)?;
    Some((own, theirs))
}

/// How far `tick` would move, in pixels along its axis, if the scale of `other` placed it
/// instead of the one of `scene`. `None` when `other` has no such axis.
fn tick_shift(tick: Option<TickRef>, scene: &Scene, other: &Scene) -> Option<(bool, f32)> {
    let tick = tick?;
    let (own, theirs) = matching_axes(tick, scene, other)?;
    let delta = theirs.px_unclamped(tick.value) - own.px_unclamped(tick.value);
    Some((own.vertical, delta as f32))
}

/// The pixel at which a bar's baseline edge would be on the scale of `other`.
fn base_px(rect: &Rect, scene: &Scene, other: &Scene) -> Option<f32> {
    let base = rect.base.as_ref()?;
    let own = scene.axes.get(base.axis)?;
    let theirs = other.axes.iter().find(|a| a.id == own.id && a.vertical)?;
    Some(theirs.px(base.y0) as f32)
}

/// The edge of a bar that faces away from its baseline: its top if it grows up from it.
fn far_edge(rect: &Rect) -> f32 {
    match &rect.base {
        Some(b) if !b.from_bottom => rect.bounds.y + rect.bounds.h,
        _ => rect.bounds.y,
    }
}

/// The segment of the same stack that sits directly under bar `i` of `scene` (its top edge is
/// where `i` begins), if `i` is a stacked segment and not the first of its stack.
fn segment_below(scene: &Scene, i: usize) -> Option<usize> {
    let r = &scene.rects[i];
    let base = r.base.as_ref().filter(|b| b.y0 != 0.0)?;
    let edge = own_baseline(r);
    scene.rects.iter().position(|o| {
        !std::ptr::eq(o, r)
            && o.layer == r.layer
            && (o.bounds.x - r.bounds.x).abs() < 0.5
            && (o.bounds.w - r.bounds.w).abs() < 0.5
            && o.base
                .as_ref()
                .is_some_and(|ob| ob.from_bottom == base.from_bottom)
            && (far_edge(o) - edge).abs() < 0.75
    })
}

/// The pixel row a bar that has no counterpart in `other` is flat on while it grows or after it
/// shrank. A bar on a baseline is flat on the baseline, where `other`'s scale puts it. A stacked
/// segment is flat where the segment under it is: at the top it has in `other` if it has a
/// counterpart there (the two then move together and stay glued), else on the same line it is
/// flat on. So a whole new stack grows from its baseline as one piece, and a segment appearing
/// in an existing stack opens up between its neighbours.
///
/// `counterpart[i]` is the index in `other` of bar `i` of `scene`, if it has one.
fn flat_edge(
    i: usize,
    scene: &Scene,
    other: &Scene,
    counterpart: &[Option<usize>],
    memo: &mut [Option<f32>],
) -> f32 {
    if let Some(y) = memo[i] {
        return y;
    }
    let r = &scene.rects[i];
    // A placeholder so a (pathological) cycle of touching zero-height bars ends.
    memo[i] = Some(own_baseline(r));
    let y = match segment_below(scene, i) {
        Some(j) => match counterpart[j] {
            Some(c) => far_edge(&other.rects[c]),
            None => flat_edge(j, scene, other, counterpart, memo),
        },
        None => base_px(r, scene, other).unwrap_or_else(|| own_baseline(r)),
    };
    memo[i] = Some(y);
    y
}

/// The bar squashed onto the line `y`.
fn flattened(rect: &Rect, y: f32) -> Rect {
    let mut r = rect.clone();
    r.bounds.y = y;
    r.bounds.h = 0.0;
    r
}

/// Where a bar's baseline edge is in its own scene.
fn own_baseline(rect: &Rect) -> f32 {
    match &rect.base {
        Some(b) if !b.from_bottom => rect.bounds.y,
        _ => rect.bounds.y + rect.bounds.h,
    }
}

impl Plan {
    /// The transition from the scene on screen to its replacement.
    pub fn diff(from: &Scene, to: &Scene, timing: Timing) -> Plan {
        let mut plan = Plan {
            timing,
            target: to.clone(),
            rects: Vec::new(),
            rules: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
            from_axes: from.axes.clone(),
            max_delay_ms: 0.0,
        };

        // Bars. The stagger runs along the new scene's bars in drawing order.
        let count = to.rects.len();
        let mut delays = vec![0.0; count];
        let changes = diff_keys(&from.rects, &to.rects, |r| &r.key);
        // Which bar of the other scene each one matches, for finding what an entering or
        // leaving segment sits on.
        let mut new_to_old = vec![None; count];
        let mut old_to_new = vec![None; from.rects.len()];
        for change in &changes {
            if let Change::Update { old, new } = *change {
                new_to_old[new] = Some(old);
                old_to_new[old] = Some(new);
            }
        }
        let mut enter_memo = vec![None; count];
        let mut exit_memo = vec![None; from.rects.len()];
        for change in changes {
            match change {
                Change::Update { old, new } => {
                    let delay = timing.delay_of(new, count);
                    delays[new] = delay;
                    plan.max_delay_ms = plan.max_delay_ms.max(delay);
                    plan.rects.push(Pair {
                        from: from.rects[old].clone(),
                        to: to.rects[new].clone(),
                        delay_ms: delay,
                        phase: Phase::Update,
                    });
                }
                Change::Enter { new } => {
                    let y = flat_edge(new, to, from, &new_to_old, &mut enter_memo);
                    let delay = timing.delay_of(new, count);
                    delays[new] = delay;
                    plan.max_delay_ms = plan.max_delay_ms.max(delay);
                    plan.rects.push(Pair {
                        from: flattened(&to.rects[new], y),
                        to: to.rects[new].clone(),
                        delay_ms: delay,
                        phase: Phase::Enter,
                    });
                }
                Change::Exit { old } => {
                    let y = flat_edge(old, from, to, &old_to_new, &mut exit_memo);
                    plan.rects.push(Pair {
                        from: from.rects[old].clone(),
                        to: flattened(&from.rects[old], y),
                        delay_ms: 0.0,
                        phase: Phase::Exit,
                    });
                }
            }
        }

        // Rules: gridlines, axis lines, ticks and legend swatches. A tick that enters starts
        // where the old scale would have put it, one that leaves ends where the new scale puts
        // it, and both fade: the way d3 animates an axis.
        for change in diff_keys(&from.rules, &to.rules, |r| &r.key) {
            plan.rules.push(match change {
                Change::Update { old, new } => Pair {
                    from: from.rules[old].clone(),
                    to: to.rules[new].clone(),
                    delay_ms: 0.0,
                    phase: Phase::Update,
                },
                Change::Enter { new } => {
                    let mut start = to.rules[new].clone();
                    start.color = transparent(start.color);
                    if let Some((vertical, d)) = tick_shift(start.tick, to, from) {
                        if vertical {
                            start.bounds.y += d;
                        } else {
                            start.bounds.x += d;
                        }
                    }
                    Pair {
                        from: start,
                        to: to.rules[new].clone(),
                        delay_ms: 0.0,
                        phase: Phase::Enter,
                    }
                }
                Change::Exit { old } => {
                    let mut end = from.rules[old].clone();
                    end.color = transparent(end.color);
                    if let Some((vertical, d)) = tick_shift(end.tick, from, to) {
                        if vertical {
                            end.bounds.y += d;
                        } else {
                            end.bounds.x += d;
                        }
                    }
                    Pair {
                        from: from.rules[old].clone(),
                        to: end,
                        delay_ms: 0.0,
                        phase: Phase::Exit,
                    }
                }
            });
        }

        // Text, the same way.
        let text_pairs = |old: &[Text], new: &[Text], delay_of: &dyn Fn(usize) -> f32| {
            let mut out: Vec<Pair<Text>> = Vec::new();
            for change in diff_keys(old, new, |t| &t.key) {
                match change {
                    Change::Update { old: o, new: n } if old[o].text == new[n].text => {
                        out.push(Pair {
                            from: old[o].clone(),
                            to: new[n].clone(),
                            delay_ms: delay_of(n),
                            phase: Phase::Update,
                        });
                    }
                    // The string changed under the same key: cross-fade instead of morphing
                    // glyphs that are not the same letters.
                    Change::Update { old: o, new: n } => {
                        let mut gone = old[o].clone();
                        gone.key.push_str(OLD_SUFFIX);
                        out.push(Pair {
                            from: new[n].clone().with_color(transparent(new[n].color)),
                            to: new[n].clone(),
                            delay_ms: delay_of(n),
                            phase: Phase::Enter,
                        });
                        out.push(Pair {
                            from: gone.clone(),
                            to: gone.clone().with_color(transparent(gone.color)),
                            delay_ms: 0.0,
                            phase: Phase::Exit,
                        });
                    }
                    Change::Enter { new: n } => {
                        let mut start = new[n].clone();
                        start.color = transparent(start.color);
                        out.push(Pair {
                            from: start,
                            to: new[n].clone(),
                            delay_ms: delay_of(n),
                            phase: Phase::Enter,
                        });
                    }
                    Change::Exit { old: o } => {
                        let mut end = old[o].clone();
                        end.color = transparent(end.color);
                        out.push(Pair {
                            from: old[o].clone(),
                            to: end,
                            delay_ms: 0.0,
                            phase: Phase::Exit,
                        });
                    }
                }
            }
            out
        };
        plan.texts = text_pairs(&from.texts, &to.texts, &|_| 0.0);
        // Axis ticks slide with their scale: the same shift as the rules.
        for pair in &mut plan.texts {
            match pair.phase {
                Phase::Enter => {
                    if let Some((vertical, d)) = tick_shift(pair.to.tick, to, from) {
                        if vertical {
                            pair.from.y += d;
                        } else {
                            pair.from.x += d;
                        }
                    }
                }
                Phase::Exit => {
                    if let Some((vertical, d)) = tick_shift(pair.from.tick, from, to) {
                        if vertical {
                            pair.to.y += d;
                        } else {
                            pair.to.x += d;
                        }
                    }
                }
                Phase::Update => {}
            }
        }

        // Value labels follow their bar's delay.
        let old_texts: Vec<Text> = from.labels.iter().map(|l| l.text.clone()).collect();
        let new_texts: Vec<Text> = to.labels.iter().map(|l| l.text.clone()).collect();
        let label_delay = |i: usize| delays.get(to.labels[i].mark).copied().unwrap_or(0.0);
        let label_pairs = text_pairs(&old_texts, &new_texts, &label_delay);
        plan.labels = label_pairs
            .into_iter()
            .map(|p| {
                // The pair's text came from `to.labels` when it ends visible, else from
                // `from.labels`; look the label up again by key for `mark` and `show`.
                let source = match p.phase {
                    Phase::Exit => from.labels.iter().find(|l| l.text.key == p.from.key),
                    _ => to.labels.iter().find(|l| l.text.key == p.to.key),
                };
                let (mark, show) =
                    source.map_or((usize::MAX, crate::spec::LabelShow::Always), |l| {
                        let mark = if p.phase == Phase::Exit {
                            usize::MAX
                        } else {
                            l.mark
                        };
                        (mark, l.show)
                    });
                Pair {
                    from: MarkLabel {
                        mark,
                        show,
                        text: p.from,
                    },
                    to: MarkLabel {
                        mark,
                        show,
                        text: p.to,
                    },
                    delay_ms: p.delay_ms,
                    phase: p.phase,
                }
            })
            .collect();

        for d in plan.labels.iter().map(|p| p.delay_ms) {
            plan.max_delay_ms = plan.max_delay_ms.max(d);
        }
        plan
    }

    /// How long the whole transition lasts, stagger included.
    pub fn total_ms(&self) -> f32 {
        self.timing.total_ms(self.max_delay_ms)
    }

    /// Whether anything at all differs between the two scenes. If not there is nothing to
    /// animate and the new scene can be shown at once.
    pub fn is_static(&self) -> bool {
        fn calm<T: PartialEq>(pairs: &[Pair<T>]) -> bool {
            pairs
                .iter()
                .all(|p| p.phase == Phase::Update && p.from == p.to)
        }
        calm(&self.rects) && calm(&self.rules) && calm(&self.texts) && calm(&self.labels)
    }

    /// The scene as it looks `elapsed_ms` into the transition: every item mixed between its
    /// two states, exits still drawn. At or after [`total_ms`](Self::total_ms) it is the target
    /// exactly (without the exits).
    pub fn sample(&self, elapsed_ms: f32) -> Scene {
        if elapsed_ms >= self.total_ms() {
            return self.target.clone();
        }
        let mut scene = self.target.clone();
        let t = |delay: f32| self.timing.eased(elapsed_ms, delay);
        scene.rects = self
            .rects
            .iter()
            .map(|p| morph_rect(p, t(p.delay_ms)))
            .collect();
        scene.rules = self
            .rules
            .iter()
            .map(|p| morph_rule(p, t(p.delay_ms)))
            .collect();
        scene.texts = self
            .texts
            .iter()
            .map(|p| morph_text(p, t(p.delay_ms)))
            .collect();
        scene.labels = self
            .labels
            .iter()
            .map(|p| MarkLabel {
                mark: p.to.mark,
                show: p.to.show,
                text: morph_text(
                    &Pair {
                        from: p.from.text.clone(),
                        to: p.to.text.clone(),
                        delay_ms: p.delay_ms,
                        phase: p.phase,
                    },
                    t(p.delay_ms),
                ),
            })
            .collect();
        // The scales slide too (all at the undelayed pace), so the next diff starts from where
        // the axes visibly are.
        let e = t(0.0);
        scene.axes = self
            .target
            .axes
            .iter()
            .map(|to| match self.from_axes.iter().find(|a| a.id == to.id) {
                Some(from) if from.vertical == to.vertical => AxisMap {
                    id: to.id.clone(),
                    vertical: to.vertical,
                    domain: (
                        f64::from(lerp(from.domain.0 as f32, to.domain.0 as f32, e)),
                        f64::from(lerp(from.domain.1 as f32, to.domain.1 as f32, e)),
                    ),
                    range: (
                        f64::from(lerp(from.range.0 as f32, to.range.0 as f32, e)),
                        f64::from(lerp(from.range.1 as f32, to.range.1 as f32, e)),
                    ),
                },
                _ => to.clone(),
            })
            .collect();
        scene
    }
}

impl Text {
    fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
}

/// `pair` at eased progress `e`. Everything that is not geometry or colour is the end state's
/// (for an exit that is the start state with the colour faded or the bar flattened).
pub fn morph_rect(p: &Pair<Rect>, e: f32) -> Rect {
    let mut r = p.to.clone();
    r.bounds = lerp_bounds(p.from.bounds, p.to.bounds, e);
    r.color = lerp_color(p.from.color, p.to.color, e);
    r
}

pub fn morph_rule(p: &Pair<Rule>, e: f32) -> Rule {
    let mut r = p.to.clone();
    r.bounds = lerp_bounds(p.from.bounds, p.to.bounds, e);
    r.color = lerp_color(p.from.color, p.to.color, e);
    r
}

pub fn morph_text(p: &Pair<Text>, e: f32) -> Text {
    let mut t = p.to.clone();
    t.x = lerp(p.from.x, p.to.x, e);
    t.y = lerp(p.from.y, p.to.y, e);
    t.size_px = lerp(p.from.size_px, p.to.size_px, e);
    t.color = lerp_color(p.from.color, p.to.color, e);
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Anchor, Base, Bounds, Rotation, RuleKind};
    use crate::spec::Easing;

    fn timing() -> Timing {
        Timing {
            duration_ms: 400.0,
            easing: Easing::Linear,
            stagger_ms: 0.0,
        }
    }

    fn bar(key: &str, x: f64, top: f64, bottom: f64) -> Rect {
        Rect {
            key: key.into(),
            bounds: Bounds::new(x, top, 20.0, bottom - top),
            color: Color::linear(0.2, 0.4, 0.6, 1.0),
            layer: 0,
            row: 0,
            sum: None,
            base: Some(Base {
                axis: 0,
                y0: 0.0,
                from_bottom: true,
            }),
        }
    }

    fn y_axis(domain: (f64, f64)) -> AxisMap {
        AxisMap {
            id: "y".into(),
            vertical: true,
            domain,
            range: (300.0, 100.0),
        }
    }

    fn scene(rects: Vec<Rect>, domain: (f64, f64)) -> Scene {
        let mut s = Scene::empty();
        s.width = 400;
        s.height = 400;
        s.rects = rects;
        s.axes = vec![y_axis(domain)];
        s
    }

    fn tick(key: &str, value: f64, domain_axis: usize) -> (Rule, Text) {
        let tick = Some(TickRef {
            axis: domain_axis,
            value,
        });
        let c = Color::linear(0.0, 0.0, 0.0, 1.0);
        (
            Rule {
                key: format!("axis:y/tick:{key}"),
                kind: RuleKind::Tick,
                bounds: Bounds::new(40.0, 0.0, 6.0, 1.0),
                color: c,
                tick,
            },
            Text {
                key: format!("axis:y/label:{key}"),
                text: key.into(),
                x: 38.0,
                y: 0.0,
                size_px: 12.0,
                color: c,
                anchor: Anchor::End,
                rotation: Rotation::None,
                tick,
            },
        )
    }

    /// Puts a tick at the pixel its value has on the scene's own y axis.
    fn with_tick(mut s: Scene, value: f64) -> Scene {
        let (mut rule, mut text) = tick(&format!("{value}"), value, 0);
        let y = s.axes[0].px_unclamped(value) as f32;
        rule.bounds.y = y;
        text.y = y;
        s.rules.push(rule);
        s.texts.push(text);
        s
    }

    fn pair_of<T>(pairs: &[Pair<T>], phase: Phase) -> Vec<&Pair<T>> {
        pairs.iter().filter(|p| p.phase == phase).collect()
    }

    #[test]
    fn keys_classify_enter_update_exit() {
        let old = scene(
            vec![bar("a", 0.0, 200.0, 300.0), bar("b", 30.0, 150.0, 300.0)],
            (0.0, 100.0),
        );
        let new = scene(
            vec![bar("b", 30.0, 100.0, 300.0), bar("c", 60.0, 250.0, 300.0)],
            (0.0, 100.0),
        );
        let plan = Plan::diff(&old, &new, timing());
        let key = |p: &Pair<Rect>| p.to.key.clone();
        assert_eq!(
            pair_of(&plan.rects, Phase::Update)
                .into_iter()
                .map(key)
                .collect::<Vec<_>>(),
            ["b"]
        );
        assert_eq!(
            pair_of(&plan.rects, Phase::Enter)
                .into_iter()
                .map(key)
                .collect::<Vec<_>>(),
            ["c"]
        );
        assert_eq!(
            pair_of(&plan.rects, Phase::Exit)
                .into_iter()
                .map(key)
                .collect::<Vec<_>>(),
            ["a"]
        );
        // The new scene's bars come first, in order, then the exits.
        let order: Vec<&str> = plan.rects.iter().map(|p| p.to.key.as_str()).collect();
        assert_eq!(order, ["b", "c", "a"]);
    }

    #[test]
    fn duplicate_keys_do_not_match_one_old_item_twice() {
        let old = scene(vec![bar("a", 0.0, 200.0, 300.0)], (0.0, 100.0));
        let new = scene(
            vec![bar("a", 0.0, 100.0, 300.0), bar("a", 30.0, 100.0, 300.0)],
            (0.0, 100.0),
        );
        let plan = Plan::diff(&old, &new, timing());
        assert_eq!(pair_of(&plan.rects, Phase::Update).len(), 1);
        assert_eq!(pair_of(&plan.rects, Phase::Enter).len(), 1);
        assert_eq!(pair_of(&plan.rects, Phase::Exit).len(), 0);
    }

    #[test]
    fn entering_and_exiting_bars_move_along_their_baseline() {
        let old = scene(vec![bar("a", 0.0, 200.0, 300.0)], (0.0, 100.0));
        let new = scene(vec![bar("c", 60.0, 150.0, 300.0)], (0.0, 100.0));
        let plan = Plan::diff(&old, &new, timing());
        let enter = &pair_of(&plan.rects, Phase::Enter)[0];
        assert_eq!(enter.from.bounds.h, 0.0);
        assert_eq!(enter.from.bounds.y, 300.0); // y = 0 on the old scale
        assert_eq!(enter.from.bounds.x, enter.to.bounds.x);
        let exit = &pair_of(&plan.rects, Phase::Exit)[0];
        assert_eq!(exit.to.bounds.h, 0.0);
        assert_eq!(exit.to.bounds.y, 300.0);
    }

    #[test]
    fn a_stacked_segment_grows_from_the_top_of_the_one_below() {
        // Value 40 is at pixel 220 on the old scale and at 260 on a taller new one.
        let mut seg = bar("s2", 0.0, 100.0, 220.0);
        seg.base.as_mut().unwrap().y0 = 40.0;
        let old = scene(vec![], (0.0, 100.0));
        let new = scene(vec![seg], (0.0, 200.0));
        let plan = Plan::diff(&old, &new, timing());
        let enter = &plan.rects[0];
        assert_eq!(enter.from.bounds.y, 220.0);
        // And leaving, it shrinks onto where the *new* scale puts 40.
        let plan = Plan::diff(&new, &old, timing());
        assert_eq!(plan.rects[0].to.bounds.y, 220.0);
        assert_eq!(plan.rects[0].to.bounds.h, 0.0);
    }

    /// A stack of three segments, `tops` the pixel row of each one's top, starting at 300.
    fn stack(x: f64, tops: [f64; 3]) -> Vec<Rect> {
        let mut out = Vec::new();
        let mut bottom = 300.0;
        for (i, top) in tops.into_iter().enumerate() {
            let mut r = bar(&format!("s{i}/{x}"), x, top, bottom);
            r.base.as_mut().unwrap().y0 = (300.0 - bottom) / 2.0;
            out.push(r);
            bottom = top;
        }
        out
    }

    #[test]
    fn a_new_stack_grows_from_its_baseline_as_one_piece() {
        let old = scene(vec![], (0.0, 100.0));
        let new = scene(stack(60.0, [250.0, 200.0, 150.0]), (0.0, 100.0));
        let plan = Plan::diff(&old, &new, timing());
        // Every segment starts flat on the baseline, so the stack stays glued while growing.
        assert!(
            plan.rects
                .iter()
                .all(|p| p.from.bounds.y == 300.0 && p.from.bounds.h == 0.0)
        );
        let mid = plan.sample(200.0);
        for pair in mid.rects.windows(2) {
            assert_eq!(
                pair[0].bounds.y,
                pair[1].bounds.y + pair[1].bounds.h,
                "segments are glued"
            );
        }
    }

    #[test]
    fn a_segment_entering_an_existing_stack_opens_between_its_neighbours() {
        // The first two segments stay (the second grows a little), the third is new.
        let old = scene(
            stack(60.0, [250.0, 200.0, 200.0])[..2].to_vec(),
            (0.0, 100.0),
        );
        let new = scene(stack(60.0, [250.0, 190.0, 150.0]), (0.0, 100.0));
        let plan = Plan::diff(&old, &new, timing());
        let enter = pair_of(&plan.rects, Phase::Enter);
        assert_eq!(enter.len(), 1);
        // Flat on the old top of the segment under it, which moves from 200 to 190 with it.
        assert_eq!(enter[0].from.bounds.y, 200.0);
        let mid = plan.sample(200.0);
        assert_eq!(
            mid.rects[1].bounds.y,
            mid.rects[2].bounds.y + mid.rects[2].bounds.h
        );
    }

    #[test]
    fn a_leaving_stack_shrinks_onto_its_baseline_as_one_piece() {
        let old = scene(stack(60.0, [250.0, 200.0, 150.0]), (0.0, 100.0));
        let new = scene(vec![], (0.0, 100.0));
        let plan = Plan::diff(&old, &new, timing());
        assert!(
            plan.rects
                .iter()
                .all(|p| p.to.bounds.y == 300.0 && p.to.bounds.h == 0.0)
        );
        let mid = plan.sample(200.0);
        for pair in mid.rects.windows(2) {
            assert_eq!(pair[0].bounds.y, pair[1].bounds.y + pair[1].bounds.h);
        }
    }

    #[test]
    fn negative_bars_hang_from_their_top_edge() {
        let mut neg = bar("n", 0.0, 300.0, 340.0);
        neg.base.as_mut().unwrap().from_bottom = false;
        let new = scene(vec![neg], (0.0, 100.0));
        let old = Scene {
            axes: vec![],
            ..scene(vec![], (0.0, 100.0))
        };
        let plan = Plan::diff(&old, &new, timing());
        // No old axis to ask: its own baseline edge, the top.
        assert_eq!(plan.rects[0].from.bounds.y, 300.0);
        assert_eq!(plan.rects[0].from.bounds.h, 0.0);
    }

    #[test]
    fn sampling_mixes_the_two_states_and_ends_on_the_target() {
        let old = scene(vec![bar("a", 0.0, 200.0, 300.0)], (0.0, 100.0));
        let new = scene(vec![bar("a", 10.0, 100.0, 300.0)], (0.0, 100.0));
        let plan = Plan::diff(&old, &new, timing());
        let start = plan.sample(0.0);
        assert_eq!(start.rects[0].bounds, old.rects[0].bounds);
        let mid = plan.sample(200.0);
        assert_eq!(mid.rects[0].bounds.x, 5.0);
        assert_eq!(mid.rects[0].bounds.y, 150.0);
        assert_eq!(mid.rects[0].bounds.h, 150.0);
        let end = plan.sample(400.0);
        assert_eq!(end.rects, new.rects);
        assert_eq!(plan.sample(10_000.0).rects, new.rects);
    }

    #[test]
    fn exits_are_drawn_until_the_end_then_gone() {
        let old = scene(vec![bar("a", 0.0, 200.0, 300.0)], (0.0, 100.0));
        let new = scene(vec![], (0.0, 100.0));
        let plan = Plan::diff(&old, &new, timing());
        let mid = plan.sample(200.0);
        assert_eq!(mid.rects.len(), 1);
        assert_eq!(mid.rects[0].bounds.h, 50.0);
        assert!(plan.sample(400.0).rects.is_empty());
    }

    #[test]
    fn stagger_delays_bars_in_drawing_order_and_extends_the_total() {
        let old = scene(vec![], (0.0, 100.0));
        let new = scene(
            vec![
                bar("a", 0.0, 100.0, 300.0),
                bar("b", 30.0, 100.0, 300.0),
                bar("c", 60.0, 100.0, 300.0),
            ],
            (0.0, 100.0),
        );
        let t = Timing {
            stagger_ms: 200.0,
            ..timing()
        };
        let plan = Plan::diff(&old, &new, t);
        let delays: Vec<f32> = plan.rects.iter().map(|p| p.delay_ms).collect();
        assert_eq!(delays, [0.0, 100.0, 200.0]);
        assert_eq!(plan.total_ms(), 600.0);
        // At 200 ms the first bar is half grown, the second a quarter, the third not started.
        let s = plan.sample(200.0);
        let h: Vec<f32> = s.rects.iter().map(|r| r.bounds.h).collect();
        assert_eq!(h, [100.0, 50.0, 0.0]);
    }

    #[test]
    fn identical_scenes_have_nothing_to_animate() {
        let s = scene(vec![bar("a", 0.0, 200.0, 300.0)], (0.0, 100.0));
        let s = with_tick(s, 50.0);
        assert!(Plan::diff(&s, &s, timing()).is_static());
        let mut moved = s.clone();
        moved.rects[0].bounds.h += 1.0;
        assert!(!Plan::diff(&s, &moved, timing()).is_static());
        assert!(!Plan::diff(&Scene::empty(), &s, timing()).is_static());
    }

    #[test]
    fn new_ticks_start_where_the_old_scale_puts_them() {
        // Old axis 0-100 over 300..100 px; the new one 0-200, so 150 is only on the new one.
        let old = with_tick(scene(vec![], (0.0, 100.0)), 50.0);
        let new = with_tick(with_tick(scene(vec![], (0.0, 200.0)), 50.0), 150.0);
        let plan = Plan::diff(&old, &new, timing());
        let enter = &pair_of(&plan.rules, Phase::Enter)[0];
        assert_eq!(enter.to.key, "axis:y/tick:150");
        // Old scale: 150 would be at 300 - 1.5 * 200 = 0; new scale: 300 - 0.75 * 200 = 150.
        assert_eq!(enter.from.bounds.y, 0.0);
        assert_eq!(enter.to.bounds.y, 150.0);
        assert_eq!(enter.from.color.a, 0.0);
        let label = &pair_of(&plan.texts, Phase::Enter)[0];
        assert_eq!(label.from.y, 0.0);
        assert_eq!(label.from.color.a, 0.0);
        // The tick that stays moves with the scale: 50 is at 200 px on the old one and 250 on
        // the new one.
        let stay = &pair_of(&plan.rules, Phase::Update)[0];
        assert_eq!((stay.from.bounds.y, stay.to.bounds.y), (200.0, 250.0));
    }

    #[test]
    fn old_ticks_slide_to_where_the_new_scale_puts_them_while_fading() {
        let old = with_tick(scene(vec![], (0.0, 100.0)), 100.0);
        let new = scene(vec![], (0.0, 200.0));
        let plan = Plan::diff(&old, &new, timing());
        let exit = &pair_of(&plan.rules, Phase::Exit)[0];
        assert_eq!(exit.from.bounds.y, 100.0);
        assert_eq!(exit.to.bounds.y, 200.0); // 100 on the new scale
        assert_eq!(exit.to.color.a, 0.0);
        let mid = plan.sample(200.0);
        assert_eq!(mid.rules[0].bounds.y, 150.0);
        assert!((mid.rules[0].color.a - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_label_whose_text_changed_cross_fades() {
        let c = Color::linear(0.0, 0.0, 0.0, 1.0);
        let label = |text: &str| MarkLabel {
            mark: 0,
            show: crate::spec::LabelShow::Always,
            text: Text {
                key: "layer0/a/label".into(),
                text: text.into(),
                x: 10.0,
                y: 20.0,
                size_px: 12.0,
                color: c,
                anchor: Anchor::Middle,
                rotation: Rotation::None,
                tick: None,
            },
        };
        let mut old = scene(vec![bar("a", 0.0, 200.0, 300.0)], (0.0, 100.0));
        old.labels.push(label("100"));
        let mut new = scene(vec![bar("a", 0.0, 150.0, 300.0)], (0.0, 100.0));
        new.labels.push(label("150"));
        let plan = Plan::diff(&old, &new, timing());
        assert_eq!(plan.labels.len(), 2);
        let enter = &pair_of(&plan.labels, Phase::Enter)[0];
        assert_eq!(enter.to.text.text, "150");
        assert_eq!(enter.from.text.color.a, 0.0);
        assert_eq!(enter.to.mark, 0);
        let exit = &pair_of(&plan.labels, Phase::Exit)[0];
        assert_eq!(exit.from.text.text, "100");
        assert!(exit.from.text.key.ends_with(OLD_SUFFIX));
        let mid = plan.sample(200.0);
        let alphas: Vec<f32> = mid.labels.iter().map(|l| l.text.color.a).collect();
        assert!(alphas.iter().all(|a| (a - 0.5).abs() < 1e-6), "{alphas:?}");
    }

    /// An update that arrives while a transition is running starts from what is on screen.
    #[test]
    fn an_interrupting_update_starts_from_the_displayed_state() {
        let a = scene(vec![bar("a", 0.0, 200.0, 300.0)], (0.0, 100.0));
        let b = scene(vec![bar("a", 0.0, 100.0, 300.0)], (0.0, 100.0));
        let c = scene(
            vec![bar("a", 0.0, 250.0, 300.0), bar("z", 40.0, 200.0, 300.0)],
            (0.0, 100.0),
        );
        let first = Plan::diff(&a, &b, timing());
        let shown = first.sample(100.0); // a quarter of the way: top at 175
        assert_eq!(shown.rects[0].bounds.y, 175.0);
        let second = Plan::diff(&shown, &c, timing());
        let start = second.sample(0.0);
        assert_eq!(start.rects[0].bounds, shown.rects[0].bounds);
        // The new bar enters; the old one continues from 175 to 250, no jump anywhere.
        assert_eq!(second.sample(400.0).rects.len(), 2);
        let mid = second.sample(200.0);
        assert_eq!(mid.rects[0].bounds.y, 212.5);
    }

    #[test]
    fn an_exit_that_is_interrupted_can_come_back() {
        let a = scene(vec![bar("a", 0.0, 200.0, 300.0)], (0.0, 100.0));
        let gone = scene(vec![], (0.0, 100.0));
        let first = Plan::diff(&a, &gone, timing());
        let shown = first.sample(200.0); // half shrunk
        let back = Plan::diff(&shown, &a, timing());
        assert_eq!(back.rects.len(), 1);
        assert_eq!(back.rects[0].phase, Phase::Update);
        assert_eq!(back.sample(0.0).rects[0].bounds.h, 50.0);
        assert_eq!(back.sample(400.0).rects[0].bounds, a.rects[0].bounds);
    }

    #[test]
    fn sampled_axes_slide_with_the_transition() {
        let old = scene(vec![], (0.0, 100.0));
        let new = scene(vec![], (0.0, 200.0));
        let plan = Plan::diff(&old, &new, timing());
        assert_eq!(plan.sample(200.0).axes[0].domain, (0.0, 150.0));
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod properties {
        use super::*;
        use proptest::prelude::*;

        fn rect_strategy() -> impl Strategy<Value = Rect> {
            (0.0f64..700.0, 0.0f64..400.0, 0.0f64..300.0)
                .prop_map(|(x, top, h)| bar("k", x, top, top + h))
        }

        proptest! {
            /// The first frame is the old scene and the last the new one, whatever the bars.
            #[test]
            fn endpoints_are_exact(a in rect_strategy(), b in rect_strategy(), easing in 0usize..4) {
                let easing = [Easing::Linear, Easing::CubicInOut, Easing::CubicOut, Easing::BackOut][easing];
                let t = Timing { easing, ..timing() };
                let (old, new) = (scene(vec![a.clone()], (0.0, 1.0)), scene(vec![b.clone()], (0.0, 1.0)));
                let plan = Plan::diff(&old, &new, t);
                prop_assert_eq!(&plan.sample(0.0).rects[0].bounds, &a.bounds);
                prop_assert_eq!(&plan.sample(t.duration_ms).rects[0].bounds, &b.bounds);
            }

            /// With linear easing every edge moves monotonically from its start to its end:
            /// a bar never overshoots, flips or goes negative on the way.
            #[test]
            fn linear_motion_is_monotone(a in rect_strategy(), b in rect_strategy()) {
                let (old, new) = (scene(vec![a.clone()], (0.0, 1.0)), scene(vec![b.clone()], (0.0, 1.0)));
                let plan = Plan::diff(&old, &new, timing());
                let mut last = a.bounds;
                for step in 1..=20 {
                    let r = plan.sample(step as f32 * 20.0).rects[0].bounds;
                    for (prev, now, from, to) in [
                        (last.x, r.x, a.bounds.x, b.bounds.x),
                        (last.y, r.y, a.bounds.y, b.bounds.y),
                        (last.w, r.w, a.bounds.w, b.bounds.w),
                        (last.h, r.h, a.bounds.h, b.bounds.h),
                    ] {
                        let moved = now - prev;
                        let wanted = to - from;
                        prop_assert!(moved * wanted >= -1e-3, "went the wrong way: {prev} -> {now} (want {from} -> {to})");
                    }
                    prop_assert!(r.w >= 0.0 && r.h >= 0.0);
                    last = r;
                }
            }

            /// Interrupting at any moment shows exactly what was on screen.
            #[test]
            fn interruption_never_jumps(a in rect_strategy(), b in rect_strategy(), c in rect_strategy(), at in 0.0f32..400.0) {
                let (sa, sb, sc) = (
                    scene(vec![a], (0.0, 1.0)),
                    scene(vec![b], (0.0, 1.0)),
                    scene(vec![c], (0.0, 1.0)),
                );
                let shown = Plan::diff(&sa, &sb, timing()).sample(at);
                let again = Plan::diff(&shown, &sc, timing()).sample(0.0);
                prop_assert_eq!(&again.rects, &shown.rects);
            }
        }
    }
}
