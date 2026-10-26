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

use std::collections::{HashMap, HashSet};

use crate::Color;
use crate::anim::{lerp, lerp_angle, lerp_bounds, lerp_color};
use crate::pathmorph::{keyed_runs, match_runs, mix_runs, reveal_runs};
use crate::scene::{
    ArcMark, AxisMap, Base, MarkLabel, PointMark, Polyline, Rect, Rule, Scene, Text, TickRef,
};
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
pub(crate) const OLD_SUFFIX: &str = "\u{1}old";

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
    /// Lines, their markers and pie slices. They are tessellated on the CPU, so the renderer
    /// is handed [`Plan::marks_at`] for each frame instead of start and end states.
    pub polylines: Vec<Pair<Polyline>>,
    pub points: Vec<Pair<PointMark>>,
    pub arcs: Vec<Pair<ArcMark>>,
    /// The old scene's axes, to interpolate the axes of a sampled scene.
    from_axes: Vec<AxisMap>,
    max_delay_ms: f32,
}

/// What happened to the item at an index.
pub(crate) enum Change {
    Update { old: usize, new: usize },
    Enter { new: usize },
    Exit { old: usize },
}

/// Matches `old` and `new` by key: every new item (in order) is an update or an enter, then
/// every unmatched old item is an exit. When a key occurs more than once (3D scenes reuse a few
/// names) the n-th new item with it matches the n-th old one, and an old item is matched at
/// most once, so nothing can move to two places.
pub(crate) fn diff_keys<T>(old: &[T], new: &[T], key: impl Fn(&T) -> &str) -> Vec<Change> {
    const NONE: usize = usize::MAX;
    // The old items of each key as a chain, in order: `head[key]` is the first one not yet
    // matched and `next[i]` the one after `i`.
    let mut head: HashMap<&str, usize> = HashMap::with_capacity(old.len());
    let mut next = vec![NONE; old.len()];
    for (i, item) in old.iter().enumerate().rev() {
        next[i] = head.insert(key(item), i).unwrap_or(NONE);
    }
    let mut used = vec![false; old.len()];
    let mut out = Vec::with_capacity(new.len());
    for (n, item) in new.iter().enumerate() {
        match head.get_mut(key(item)) {
            Some(first) if *first != NONE => {
                let o = *first;
                *first = next[o];
                used[o] = true;
                out.push(Change::Update { old: o, new: n });
            }
            _ => out.push(Change::Enter { new: n }),
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

pub(crate) fn transparent(c: Color) -> Color {
    Color { a: 0.0, ..c }
}

/// How far `tick` would move, in pixels (or world units) along its axis, if the scale of the
/// axis list `other` placed it instead of the one in `own`. `None` when `other` has no such
/// axis. The flag says whether the axis runs along y.
pub(crate) fn tick_shift(
    tick: Option<TickRef>,
    own: &[AxisMap],
    other: &[AxisMap],
) -> Option<(bool, f32)> {
    let tick = tick?;
    let own = own.get(tick.axis)?;
    let theirs = other
        .iter()
        .find(|a| a.id == own.id && a.vertical == own.vertical)?;
    let delta = theirs.px_unclamped(tick.value) - own.px_unclamped(tick.value);
    Some((own.vertical, delta as f32))
}

/// The pixel at which a bar's baseline edge would be on the scale of `other`.
pub(crate) fn base_px(base: &Base, own: &[AxisMap], other: &[AxisMap]) -> Option<f32> {
    let own = own.get(base.axis)?;
    let theirs = other.iter().find(|a| a.id == own.id && a.vertical)?;
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
        None => r
            .base
            .as_ref()
            .and_then(|b| base_px(b, &scene.axes, &other.axes))
            .unwrap_or_else(|| own_baseline(r)),
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
            polylines: Vec::new(),
            points: Vec::new(),
            arcs: Vec::new(),
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

        // Lines draw on, fade out, or morph; their markers scale and fade; pie slices sweep
        // open from, and close onto, the boundary of their neighbours. The stagger runs along
        // the new scene's markers and slices (bars, then points, then slices make up the mark
        // indices the value labels refer to).
        let mut mark_delays = delays.clone();
        // Where the vertices (and so the markers) that appear or disappear on a line that stays
        // start from or end at: on the neighbouring point that stays.
        let mut anchors: HashMap<&str, [f32; 2]> = HashMap::new();
        // The points of lines whose vertices could not be matched by point (their markers
        // cross-fade instead of moving, since the line does not move with them).
        let mut loose: HashSet<&str> = HashSet::new();
        for change in diff_keys(&from.polylines, &to.polylines, |l| &l.key) {
            plan.polylines.push(match change {
                Change::Update { old, new } => {
                    let (a, b) = keyed_runs(
                        (&from.polylines[old].runs, &from.polylines[old].keys),
                        (&to.polylines[new].runs, &to.polylines[new].keys),
                        &mut anchors,
                    )
                    .unwrap_or_else(|| {
                        let both = from.polylines[old]
                            .keys
                            .iter()
                            .chain(&to.polylines[new].keys);
                        loose.extend(both.map(String::as_str));
                        match_runs(&from.polylines[old].runs, &to.polylines[new].runs)
                    });
                    Pair {
                        from: Polyline {
                            runs: a,
                            ..from.polylines[old].clone()
                        },
                        to: Polyline {
                            runs: b,
                            ..to.polylines[new].clone()
                        },
                        delay_ms: 0.0,
                        phase: Phase::Update,
                    }
                }
                Change::Enter { new } => Pair {
                    from: Polyline {
                        reveal: 0.0,
                        ..to.polylines[new].clone()
                    },
                    to: to.polylines[new].clone(),
                    delay_ms: 0.0,
                    phase: Phase::Enter,
                },
                Change::Exit { old } => Pair {
                    from: from.polylines[old].clone(),
                    to: Polyline {
                        color: transparent(from.polylines[old].color),
                        ..from.polylines[old].clone()
                    },
                    delay_ms: 0.0,
                    phase: Phase::Exit,
                },
            });
        }
        let count = to.points.len();
        let mut leaving: Vec<Pair<PointMark>> = Vec::new();
        for change in diff_keys(&from.points, &to.points, |p| &p.key) {
            plan.points.push(match change {
                Change::Update { old, new } if loose.contains(to.points[new].key.as_str()) => {
                    // Fades out where it was (as a marker of its own, listed after the rest)
                    // while the new one fades in where it is now.
                    let mut gone = vanished(&from.points[old], &anchors);
                    (gone.x, gone.y) = (from.points[old].x, from.points[old].y);
                    gone.key.push_str(OLD_SUFFIX);
                    leaving.push(Pair {
                        from: PointMark {
                            key: gone.key.clone(),
                            ..from.points[old].clone()
                        },
                        to: gone,
                        delay_ms: 0.0,
                        phase: Phase::Exit,
                    });
                    Pair {
                        from: vanished(&to.points[new], &anchors),
                        to: to.points[new].clone(),
                        delay_ms: timing.delay_of(new, count),
                        phase: Phase::Enter,
                    }
                }
                Change::Update { old, new } => Pair {
                    from: from.points[old].clone(),
                    to: to.points[new].clone(),
                    delay_ms: timing.delay_of(new, count),
                    phase: Phase::Update,
                },
                Change::Enter { new } => Pair {
                    from: vanished(&to.points[new], &anchors),
                    to: to.points[new].clone(),
                    delay_ms: timing.delay_of(new, count),
                    phase: Phase::Enter,
                },
                Change::Exit { old } => Pair {
                    from: from.points[old].clone(),
                    to: vanished(&from.points[old], &anchors),
                    delay_ms: 0.0,
                    phase: Phase::Exit,
                },
            });
        }
        // (The exits already in the list stay after the new ones, so mark indices still agree.)
        mark_delays.extend(
            plan.points
                .iter()
                .filter(|p| p.phase != Phase::Exit)
                .map(|p| p.delay_ms),
        );
        plan.points.extend(leaving);
        let count = to.arcs.len();
        let changes = diff_keys(&from.arcs, &to.arcs, |a| &a.key);
        let mut new_to_old = vec![None; count];
        let mut old_to_new = vec![None; from.arcs.len()];
        for change in &changes {
            if let Change::Update { old, new } = *change {
                new_to_old[new] = Some(old);
                old_to_new[old] = Some(new);
            }
        }
        for change in changes {
            plan.arcs.push(match change {
                Change::Update { old, new } => Pair {
                    from: from.arcs[old].clone(),
                    to: to.arcs[new].clone(),
                    delay_ms: timing.delay_of(new, count),
                    phase: Phase::Update,
                },
                Change::Enter { new } => {
                    // Opens from the old boundary where the slice would have been squeezed in.
                    let at = open_seam(new, &angles(&to.arcs), &new_to_old, &angles(&from.arcs));
                    Pair {
                        from: closed(&to.arcs[new], at),
                        to: to.arcs[new].clone(),
                        delay_ms: timing.delay_of(new, count),
                        phase: Phase::Enter,
                    }
                }
                Change::Exit { old } => {
                    // Closes onto the new boundary of the slices that stay on either side.
                    let at = close_seam(old, &angles(&from.arcs), &old_to_new, &angles(&to.arcs));
                    Pair {
                        from: from.arcs[old].clone(),
                        to: closed(&from.arcs[old], at),
                        delay_ms: 0.0,
                        phase: Phase::Exit,
                    }
                }
            });
        }
        mark_delays.extend(
            plan.arcs
                .iter()
                .filter(|p| p.phase != Phase::Exit)
                .map(|p| p.delay_ms),
        );

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
                    if let Some((vertical, d)) = tick_shift(start.tick, &to.axes, &from.axes) {
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
                    if let Some((vertical, d)) = tick_shift(end.tick, &from.axes, &to.axes) {
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
                    if let Some((vertical, d)) = tick_shift(pair.to.tick, &to.axes, &from.axes) {
                        if vertical {
                            pair.from.y += d;
                        } else {
                            pair.from.x += d;
                        }
                    }
                }
                Phase::Exit => {
                    if let Some((vertical, d)) = tick_shift(pair.from.tick, &from.axes, &to.axes) {
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
        let label_delay = |i: usize| mark_delays.get(to.labels[i].mark).copied().unwrap_or(0.0);
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

        let delays = (plan.labels.iter().map(|p| p.delay_ms))
            .chain(plan.points.iter().map(|p| p.delay_ms))
            .chain(plan.arcs.iter().map(|p| p.delay_ms));
        for d in delays {
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
        calm(&self.rects)
            && calm(&self.rules)
            && calm(&self.texts)
            && calm(&self.labels)
            && calm(&self.polylines)
            && calm(&self.points)
            && calm(&self.arcs)
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
        self.fill_marks(&mut scene, elapsed_ms);
        // The scales slide too (all at the undelayed pace), so the next diff starts from where
        // the axes visibly are.
        scene.axes = lerp_axes(&self.from_axes, &self.target.axes, t(0.0));
        scene
    }
}

/// The axes `e` of the way from those of the old scene to those of the new one; an axis only
/// the new scene has is as it is.
pub(crate) fn lerp_axes(from: &[AxisMap], to: &[AxisMap], e: f32) -> Vec<AxisMap> {
    to.iter()
        .map(|to| match from.iter().find(|a| a.id == to.id) {
            Some(from) if from.vertical == to.vertical => {
                let l = |a: f64, b: f64| f64::from(lerp(a as f32, b as f32, e));
                AxisMap {
                    id: to.id.clone(),
                    vertical: to.vertical,
                    domain: (l(from.domain.0, to.domain.0), l(from.domain.1, to.domain.1)),
                    range: (l(from.range.0, to.range.0), l(from.range.1, to.range.1)),
                }
            }
            _ => to.clone(),
        })
        .collect()
}

impl Plan {
    /// Whether there are lines, markers or slices to move: they are redrawn each frame from
    /// [`marks_at`](Self::marks_at) because the CPU tessellates them.
    pub fn has_marks(&self) -> bool {
        !(self.polylines.is_empty() && self.points.is_empty() && self.arcs.is_empty())
    }

    /// A scene holding only the lines, markers and slices as they are `elapsed_ms` into the
    /// transition (the rest empty): what the renderer re-tessellates each frame.
    pub fn marks_at(&self, elapsed_ms: f32) -> Scene {
        let mut scene = Scene::empty();
        scene.width = self.target.width;
        scene.height = self.target.height;
        scene.background = self.target.background;
        self.fill_marks(&mut scene, elapsed_ms);
        scene
    }

    fn fill_marks(&self, scene: &mut Scene, elapsed_ms: f32) {
        let t = |delay: f32| self.timing.eased(elapsed_ms, delay);
        scene.polylines = self
            .polylines
            .iter()
            .map(|p| morph_polyline(p, t(p.delay_ms)))
            .collect();
        scene.points = self
            .points
            .iter()
            .map(|p| morph_point(p, t(p.delay_ms)))
            .collect();
        scene.arcs = self
            .arcs
            .iter()
            .map(|p| morph_arc(p, t(p.delay_ms)))
            .collect();
    }
}

/// A marker with no size and no opacity: what one scales and fades from or to. It sits at its
/// anchor (the neighbouring point of its line that stays, see [`keyed_runs`]), else where it is.
fn vanished(p: &PointMark, anchors: &HashMap<&str, [f32; 2]>) -> PointMark {
    let [x, y] = anchors.get(p.key.as_str()).copied().unwrap_or([p.x, p.y]);
    PointMark {
        x,
        y,
        size: 0.0,
        color: transparent(p.color),
        ..p.clone()
    }
}

/// The angle a slice that is not in the old scene opens from. Between two slices that stay
/// (X before it, Y after it) the old gap is `[X.end, Y.start]` and the new one `[X.end', Y.start']`;
/// the slices that enter fill the new gap as it opens, so they open from the *old* end of the
/// gap, `Y.start`: the first enters at `lerp(Y.start, X.end')`, the last ends at `lerp(Y.start,
/// Y.start')`, which is where Y is then. Slices that enter next to each other open from the
/// same angle, so they unfold as one piece, and together with the ones that leave they tile the
/// gap at every moment. With no slice after it, the gap ends where the circle does; with no old
/// slices at all, everything opens from the first angle.
///
/// `counterpart[j]` is the index in `old` of slice `j` of `new`, if it has one.
pub(crate) fn open_seam(
    i: usize,
    new: &[(f32, f32)],
    counterpart: &[Option<usize>],
    old: &[(f32, f32)],
) -> f32 {
    counterpart[i..]
        .iter()
        .find_map(|c| c.map(|c| old[c].0))
        .or_else(|| old.last().map(|o| o.1))
        .unwrap_or(new[0].0)
}

/// The angle a slice that is not in the new scene closes onto: where the slice before it that
/// stays ends in the new layout (the mirror of [`open_seam`]: leaving slices collapse onto the
/// start of the new gap). With none before it, the start of the circle.
///
/// `counterpart[j]` is the index in `new` of slice `j` of `old`, if it has one.
pub(crate) fn close_seam(
    i: usize,
    old: &[(f32, f32)],
    counterpart: &[Option<usize>],
    new: &[(f32, f32)],
) -> f32 {
    counterpart[..i]
        .iter()
        .rev()
        .find_map(|c| c.map(|c| new[c].1))
        .or_else(|| new.first().map(|n| n.0))
        .unwrap_or(old[0].0)
}

fn angles(arcs: &[ArcMark]) -> Vec<(f32, f32)> {
    arcs.iter().map(|a| (a.start, a.end)).collect()
}

/// The slice squeezed to nothing at angle `at`.
fn closed(slice: &ArcMark, at: f32) -> ArcMark {
    ArcMark {
        start: at,
        end: at,
        ..slice.clone()
    }
}

pub fn morph_polyline(p: &Pair<Polyline>, e: f32) -> Polyline {
    let mut l = p.to.clone();
    l.runs = mix_runs(&p.from.runs, &p.to.runs, e);
    l.color = lerp_color(p.from.color, p.to.color, e);
    l.width = lerp(p.from.width, p.to.width, e);
    let reveal = lerp(p.from.reveal, p.to.reveal, e);
    if reveal < 1.0 {
        l.runs = reveal_runs(&l.runs, reveal);
    }
    l.reveal = 1.0;
    l
}

pub fn morph_point(p: &Pair<PointMark>, e: f32) -> PointMark {
    let mut m = p.to.clone();
    m.x = lerp(p.from.x, p.to.x, e);
    m.y = lerp(p.from.y, p.to.y, e);
    m.size = lerp(p.from.size, p.to.size, e).max(0.0);
    m.color = lerp_color(p.from.color, p.to.color, e);
    m
}

pub fn morph_arc(p: &Pair<ArcMark>, e: f32) -> ArcMark {
    let mut a = p.to.clone();
    let l = |f: f32, t: f32| lerp(f, t, e);
    a.center = [
        l(p.from.center[0], p.to.center[0]),
        l(p.from.center[1], p.to.center[1]),
    ];
    a.inner = l(p.from.inner, p.to.inner).max(0.0);
    a.outer = l(p.from.outer, p.to.outer).max(0.0);
    a.start = lerp_angle(p.from.start, p.to.start, e);
    a.end = lerp_angle(p.from.end, p.to.end, e).max(a.start);
    a.pad = l(p.from.pad, p.to.pad);
    a.color = lerp_color(p.from.color, p.to.color, e);
    a
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
    use crate::pathmorph::subdivide;
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

    fn line(key: &str, points: &[[f32; 2]]) -> Polyline {
        Polyline {
            key: key.into(),
            color: Color::linear(0.2, 0.4, 0.6, 1.0),
            width: 2.0,
            runs: vec![points.to_vec()],
            layer: 1,
            reveal: 1.0,
            keys: Vec::new(),
        }
    }

    /// A data line whose vertices are the points `a`, `b`, ... at the given positions.
    fn keyed_line(points: &[(&str, [f32; 2])]) -> Polyline {
        Polyline {
            keys: points.iter().map(|p| p.0.to_owned()).collect(),
            ..line("l", &points.iter().map(|p| p.1).collect::<Vec<_>>())
        }
    }

    fn marker(key: &str, at: [f32; 2]) -> PointMark {
        PointMark {
            key: key.into(),
            x: at[0],
            y: at[1],
            color: Color::linear(1.0, 0.0, 0.0, 1.0),
            size: 8.0,
            visible: true,
            layer: 1,
            row: 0,
        }
    }

    fn line_scene(points: &[(&str, [f32; 2])]) -> Scene {
        let mut s = scene(vec![], (0.0, 1.0));
        s.polylines.push(keyed_line(points));
        s.points = points.iter().map(|p| marker(p.0, p.1)).collect();
        s
    }

    #[test]
    fn a_point_that_is_added_to_a_line_grows_out_of_its_neighbour_and_its_marker_with_it() {
        let old = line_scene(&[
            ("a", [0.0, 100.0]),
            ("b", [100.0, 50.0]),
            ("c", [200.0, 80.0]),
        ]);
        let new = line_scene(&[
            ("a", [0.0, 90.0]),
            ("b", [80.0, 40.0]),
            ("c", [160.0, 70.0]),
            ("d", [240.0, 10.0]),
        ]);
        let plan = Plan::diff(&old, &new, timing());
        let pair = &plan.polylines[0];
        assert_eq!(pair.from.runs[0].len(), 4);
        // At the start the line is the old one with the new vertex sitting on its last point.
        assert_eq!(pair.from.runs[0][3], [200.0, 80.0]);
        assert_eq!(
            &plan.sample(400.0).polylines[0].runs,
            &new.polylines[0].runs
        );
        // The marker of the new point starts on the same spot as the vertex does, and every
        // marker that stays is on the line at every moment.
        for step in 0..=8 {
            let s = plan.sample(step as f32 * 50.0);
            let line = &s.polylines[0].runs[0];
            for m in &s.points {
                assert!(
                    line.iter()
                        .any(|v| (v[0] - m.x).abs() < 1e-3 && (v[1] - m.y).abs() < 1e-3),
                    "marker {} off the line at step {step}: {m:?} vs {line:?}",
                    m.key
                );
            }
        }
    }

    #[test]
    fn a_point_that_is_removed_closes_onto_its_neighbour() {
        let old = line_scene(&[
            ("a", [0.0, 100.0]),
            ("b", [100.0, 50.0]),
            ("c", [200.0, 80.0]),
        ]);
        let new = line_scene(&[("a", [0.0, 100.0]), ("c", [200.0, 80.0])]);
        let plan = Plan::diff(&old, &new, timing());
        let pair = &plan.polylines[0];
        assert_eq!(pair.from.runs[0], old.polylines[0].runs[0]);
        // b ends on a.
        assert_eq!(pair.to.runs[0][1], [0.0, 100.0]);
        for step in 0..=8 {
            let s = plan.sample(step as f32 * 50.0);
            let line = &s.polylines[0].runs[0];
            for m in &s.points {
                assert!(
                    line.iter()
                        .any(|v| (v[0] - m.x).abs() < 1e-3 && (v[1] - m.y).abs() < 1e-3)
                );
            }
        }
        assert_eq!(
            &plan.sample(400.0).polylines[0].runs,
            &new.polylines[0].runs
        );
    }

    #[test]
    fn lines_whose_points_changed_order_fall_back_to_resampling() {
        let old = line_scene(&[
            ("a", [0.0, 100.0]),
            ("b", [100.0, 50.0]),
            ("c", [200.0, 80.0]),
        ]);
        let new = line_scene(&[
            ("c", [0.0, 10.0]),
            ("b", [100.0, 20.0]),
            ("a", [200.0, 30.0]),
        ]);
        let plan = Plan::diff(&old, &new, timing());
        let pair = &plan.polylines[0];
        assert_eq!(pair.from.runs[0].len(), pair.to.runs[0].len());
        assert_eq!(
            &plan.sample(400.0).polylines[0].runs,
            &new.polylines[0].runs
        );
    }

    #[test]
    fn subdividing_keeps_every_vertex_and_the_shape() {
        let run = [[0.0, 0.0], [10.0, 0.0], [10.0, 30.0]];
        let more = subdivide(&run, 7);
        assert_eq!(more.len(), 7);
        // The original points are still there, in order.
        let mut at = 0;
        for p in run {
            at += more[at..].iter().position(|&q| q == p).unwrap();
        }
        // The new ones lie on the old line, mostly on the longer segment.
        for q in &more {
            let on_first = q[1] == 0.0 && (0.0..=10.0).contains(&q[0]);
            let on_second = q[0] == 10.0 && (0.0..=30.0).contains(&q[1]);
            assert!(on_first || on_second, "{q:?}");
        }
        assert_eq!(more.iter().filter(|q| q[0] == 10.0).count(), 5);
        assert_eq!(subdivide(&run, 3), run);
        assert_eq!(subdivide(&[[1.0, 2.0]], 3), vec![[1.0, 2.0]; 3]);
    }

    #[test]
    fn a_line_morphs_between_different_point_counts_without_a_jump() {
        let a = line("l", &[[0.0, 100.0], [50.0, 50.0], [100.0, 80.0]]);
        let b = line(
            "l",
            &[
                [0.0, 90.0],
                [25.0, 40.0],
                [50.0, 60.0],
                [75.0, 30.0],
                [100.0, 20.0],
            ],
        );
        let mut old = scene(vec![], (0.0, 1.0));
        old.polylines.push(a.clone());
        let mut new = scene(vec![], (0.0, 1.0));
        new.polylines.push(b.clone());
        let plan = Plan::diff(&old, &new, timing());
        let pair = &plan.polylines[0];
        assert_eq!(pair.from.runs[0].len(), pair.to.runs[0].len());
        // First frame: the old line (its original vertices among the extra points).
        let first = &plan.sample(0.0).polylines[0].runs[0];
        for p in &a.runs[0] {
            assert!(first.contains(p), "{p:?} missing in {first:?}");
        }
        assert_eq!(&plan.sample(400.0).polylines[0].runs, &b.runs);
        // Midway the line is in between: every point between its two ends.
        let mid = &plan.sample(200.0).polylines[0].runs[0];
        for (m, (f, t)) in mid
            .iter()
            .zip(pair.from.runs[0].iter().zip(&pair.to.runs[0]))
        {
            assert!((m[1] - (f[1] + t[1]) / 2.0).abs() < 1e-3);
        }
    }

    #[test]
    fn a_new_line_is_drawn_on_from_its_start_and_an_old_one_fades_out() {
        let l = line("l", &[[0.0, 100.0], [100.0, 100.0], [100.0, 200.0]]);
        let mut with = scene(vec![], (0.0, 1.0));
        with.polylines.push(l.clone());
        let without = scene(vec![], (0.0, 1.0));
        let plan = Plan::diff(&without, &with, timing());
        assert_eq!(plan.polylines[0].phase, Phase::Enter);
        assert!(
            plan.sample(0.0).polylines[0].runs.is_empty()
                || plan.sample(0.0).polylines[0].runs[0].len() == 1
        );
        // A quarter of the way: a quarter of the length (50 of 200 px), still on the first leg.
        let quarter = &plan.sample(100.0).polylines[0].runs[0];
        assert_eq!(quarter.as_slice(), &[[0.0, 100.0], [50.0, 100.0]]);
        let three_quarters = &plan.sample(300.0).polylines[0].runs[0];
        assert_eq!(
            three_quarters.as_slice(),
            &[[0.0, 100.0], [100.0, 100.0], [100.0, 150.0]]
        );
        assert_eq!(&plan.sample(400.0).polylines[0].runs, &l.runs);
        // Leaving: the same line, fading.
        let plan = Plan::diff(&with, &without, timing());
        assert_eq!(plan.polylines[0].phase, Phase::Exit);
        let mid = &plan.sample(200.0).polylines[0];
        assert_eq!(mid.runs, l.runs);
        assert!((mid.color.a - 0.5).abs() < 1e-6);
        assert!(plan.sample(400.0).polylines.is_empty());
    }

    #[test]
    fn markers_scale_and_fade_in_and_out() {
        let marker = |key: &str, x: f32| PointMark {
            key: key.into(),
            x,
            y: 10.0,
            color: Color::linear(1.0, 0.0, 0.0, 1.0),
            size: 8.0,
            visible: true,
            layer: 1,
            row: 0,
        };
        let mut a = scene(vec![], (0.0, 1.0));
        a.points = vec![marker("a", 10.0), marker("b", 20.0)];
        let mut b = scene(vec![], (0.0, 1.0));
        b.points = vec![marker("b", 30.0), marker("c", 40.0)];
        let plan = Plan::diff(&a, &b, timing());
        let mid = plan.sample(200.0);
        let by_key = |k: &str| mid.points.iter().find(|p| p.key == k).unwrap();
        assert_eq!(by_key("b").x, 25.0);
        assert_eq!(by_key("b").size, 8.0);
        assert_eq!(by_key("c").size, 4.0); // entering: half grown
        assert!((by_key("c").color.a - 0.5).abs() < 1e-6);
        assert_eq!(by_key("a").size, 4.0); // leaving: half shrunk
        assert_eq!(by_key("c").x, 40.0); // in place
        assert_eq!(plan.sample(400.0).points.len(), 2);
    }

    fn slices(spec: &[(&str, f32)]) -> Scene {
        let total: f32 = spec.iter().map(|s| s.1).sum();
        let mut at = 0.0;
        let mut s = Scene::empty();
        for (key, v) in spec {
            let end = at + v / total * std::f32::consts::TAU;
            s.arcs.push(ArcMark {
                key: (*key).into(),
                color: Color::linear(0.5, 0.5, 0.5, 1.0),
                center: [100.0, 100.0],
                inner: 20.0,
                outer: 80.0,
                start: at,
                end,
                pad: 0.0,
                layer: 0,
                row: 0,
                sum: None,
            });
            at = end;
        }
        s
    }

    /// The slices of a sampled scene, in angular order, never overlap and never leave a gap
    /// bigger than float noise: they tile the circle at every moment.
    fn assert_tiles(s: &Scene, what: &str) {
        let mut arcs: Vec<&ArcMark> = s.arcs.iter().collect();
        arcs.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.end.total_cmp(&b.end)));
        for pair in arcs.windows(2) {
            assert!(
                pair[0].end <= pair[1].start + 1e-3,
                "{what}: {} ends at {} after {} starts at {}",
                pair[0].key,
                pair[0].end,
                pair[1].key,
                pair[1].start
            );
        }
        for a in &arcs {
            assert!(a.end >= a.start - 1e-6, "{what}: {} inverted", a.key);
        }
    }

    #[test]
    fn pie_slices_open_and_close_along_their_neighbours_and_never_overlap() {
        let abc = slices(&[("a", 3.0), ("b", 2.0), ("c", 1.0)]);
        let ac = slices(&[("a", 3.0), ("c", 1.0)]);
        let abdc = slices(&[("a", 3.0), ("b", 2.0), ("d", 4.0), ("c", 1.0)]);
        let empty = slices(&[]);
        let all = [&abc, &ac, &abdc, &empty];
        for from in all {
            for to in all {
                let plan = Plan::diff(from, to, timing());
                for step in 0..=20 {
                    assert_tiles(&plan.sample(step as f32 * 20.0), &format!("step {step}"));
                }
            }
        }
        // Removing b: it closes onto the seam between a and c in the new layout.
        let plan = Plan::diff(&abc, &ac, timing());
        let exit = plan.arcs.iter().find(|p| p.phase == Phase::Exit).unwrap();
        assert_eq!(exit.to.start, exit.to.end);
        assert_eq!(exit.to.start, ac.arcs[0].end);
        // Adding d between b and c: it opens from where the old c began (= where b ended).
        let plan = Plan::diff(&abc, &abdc, timing());
        let enter = plan.arcs.iter().find(|p| p.phase == Phase::Enter).unwrap();
        assert_eq!(enter.from.start, abc.arcs[2].start);
        assert_eq!(enter.from.start, enter.from.end);
        // A whole new pie sweeps open from its first angle.
        let plan = Plan::diff(&empty, &abc, timing());
        assert!(plan.arcs.iter().all(|p| p.from.start == abc.arcs[0].start));
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod arc_properties {
        use super::*;
        use proptest::prelude::*;

        fn pie() -> impl Strategy<Value = Vec<(String, f32)>> {
            proptest::collection::btree_map("[a-f]", 0.5f32..10.0, 0..6)
                .prop_map(|m| m.into_iter().collect())
        }

        proptest! {
            /// Whatever slices appear and disappear, the pie tiles at every moment, and the
            /// first and last frames are exactly the old and the new pie.
            #[test]
            fn slices_tile_at_every_moment(a in pie(), b in pie(), t in 0.0f32..400.0) {
                let as_scene = |p: &Vec<(String, f32)>| {
                    slices(&p.iter().map(|(k, v)| (k.as_str(), *v)).collect::<Vec<_>>())
                };
                let (from, to) = (as_scene(&a), as_scene(&b));
                let plan = Plan::diff(&from, &to, timing());
                assert_tiles(&plan.sample(t), "random");
                prop_assert_eq!(&plan.sample(400.0).arcs, &to.arcs);
                let first = plan.sample(0.0);
                for arc in &from.arcs {
                    let shown = first.arcs.iter().find(|x| x.key == arc.key).unwrap();
                    prop_assert_eq!((shown.start, shown.end), (arc.start, arc.end));
                }
            }

            #[test]
            fn subdividing_is_exact_and_monotone(
                pts in proptest::collection::vec((0.0f32..500.0, 0.0f32..500.0), 2..12),
                extra in 0usize..20,
            ) {
                let run: Vec<[f32; 2]> = pts.iter().map(|p| [p.0, p.1]).collect();
                let more = subdivide(&run, run.len() + extra);
                prop_assert_eq!(more.len(), run.len() + extra);
                prop_assert_eq!(more[0], run[0]);
                prop_assert_eq!(more[more.len() - 1], run[run.len() - 1]);
                let mut at = 0;
                for p in &run {
                    let found = more[at..].iter().position(|q| q == p);
                    prop_assert!(found.is_some());
                    at += found.unwrap();
                }
            }

            /// Revealing more shows more: the length shown never goes down, and the whole line
            /// is drawn at 1.
            #[test]
            fn reveal_grows_with_the_fraction(
                pts in proptest::collection::vec((0.0f32..500.0, 0.0f32..500.0), 2..12),
            ) {
                let run: Vec<[f32; 2]> = pts.iter().map(|p| [p.0, p.1]).collect();
                let length = |runs: &[Vec<[f32; 2]>]| -> f32 {
                    runs.iter().flat_map(|r| r.windows(2)).map(|w| (w[1][0]-w[0][0]).hypot(w[1][1]-w[0][1])).sum()
                };
                let mut last = -1.0;
                for i in 0..=10 {
                    let shown = length(&reveal_runs(std::slice::from_ref(&run), i as f32 / 10.0));
                    prop_assert!(shown >= last - 1e-3);
                    last = shown;
                }
                prop_assert_eq!(reveal_runs(std::slice::from_ref(&run), 1.0), vec![run]);
            }
        }
    }
}
