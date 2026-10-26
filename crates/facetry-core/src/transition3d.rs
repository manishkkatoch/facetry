//! Transitions between two 3D scenes: the twin of [`crate::transition`] for [`Scene3d`].
//!
//! The same idea: [`Plan3::diff`] matches the items of the scene on screen and of the new one
//! by key and turns each into a [`Pair`] of start and end state, [`Plan3::sample`] mixes them
//! for any moment (which is also how an interrupting update finds out what is on screen). The
//! renderer mixes the boxes and the text on the GPU, and redoes everything it tessellates (grid
//! and axis lines, pie slices, data lines, curtains, markers) from [`Plan3::marks_at`] each
//! frame.
//!
//! What is different from 2D:
//!
//! * the plot box can change size (a category more or less) and the legend can change what is
//!   left of the canvas, so both are interpolated too and the camera frames the box as it is
//!   at that moment: nothing in the picture jumps when the layout does;
//! * the legend is a 2D overlay, animated by a 2D [`Plan`] of its rules and texts;
//! * a world label (an axis label, a value label) hangs from a point in space and is moved,
//!   faded and cross-faded like text in 2D. Which labels are *shown* is decided per frame
//!   from the camera by the label planner; during a transition it works on the labels of the
//!   **target** scene, see `facetry-render`'s `labels` module.

use std::collections::{HashMap, HashSet};

use crate::anim::{lerp, lerp_angle, lerp_color};
use crate::pathmorph::{keyed_runs, match_runs, mix_runs, mix_runs_into, reveal_runs};
use crate::scene::{AxisMap, Base, Scene};
use crate::scene3d::{
    BoxMark, Line3, MarkLabel3, Overlay, PointMark3, Polyline3, Scene3d, SliceMark, Text3,
};
use crate::spec::Timing;
use crate::transition::{
    Change, OLD_SUFFIX, Pair, Phase, Plan, base_px, close_seam, diff_keys, lerp_axes, open_seam,
    tick_shift, transparent,
};

/// A transition between two 3D scenes. Items are listed in the new scene's order followed by
/// the exits, so index `i` of `boxes` is box `i` of `target` (hover and click address the
/// target), and likewise for slices and points.
#[derive(Debug, Clone)]
pub struct Plan3 {
    pub timing: Timing,
    pub target: Scene3d,
    pub boxes: Vec<Pair<BoxMark>>,
    pub lines: Vec<Pair<Line3>>,
    pub texts: Vec<Pair<Text3>>,
    pub labels: Vec<Pair<MarkLabel3>>,
    pub slices: Vec<Pair<SliceMark>>,
    pub polylines: Vec<Pair<Polyline3>>,
    pub points: Vec<Pair<PointMark3>>,
    /// The legend: a 2D plan of the overlay's rules and texts.
    pub overlay: Plan,
    /// The old plot box and the old part of the canvas it was framed in.
    pub from_size: [f32; 3],
    pub from_plot: Option<[f32; 4]>,
    from_axes: Vec<AxisMap>,
    max_delay_ms: f32,
}

/// The legend overlay as a 2D scene, so that a 2D [`Plan`] can animate it.
fn overlay_scene(scene: &Scene3d) -> Scene {
    let mut s = Scene::empty();
    s.width = scene.width;
    s.height = scene.height;
    s.background = scene.background;
    s.rules.clone_from(&scene.overlay.rules);
    s.texts.clone_from(&scene.overlay.texts);
    s
}

/// The edge of a box that faces away from its baseline: its top if it grows up from it.
fn far_edge(b: &BoxMark) -> f32 {
    match &b.base {
        Some(base) if !base.from_bottom => b.min[1],
        _ => b.max[1],
    }
}

/// Where a box's baseline edge is in its own scene.
fn own_baseline(b: &BoxMark) -> f32 {
    match &b.base {
        Some(base) if !base.from_bottom => b.max[1],
        _ => b.min[1],
    }
}

/// The box squashed onto the plane at height `y`.
fn flattened(b: &BoxMark, y: f32) -> BoxMark {
    let mut c = b.clone();
    c.min[1] = y;
    c.max[1] = y;
    c
}

/// The box of the same stack that sits directly under box `i`, if it is a stacked segment and
/// not the first of its stack (see `segment_below` in `transition.rs`).
fn segment_below(scene: &Scene3d, i: usize) -> Option<usize> {
    let b = &scene.boxes[i];
    let base = b.base.as_ref().filter(|b| b.y0 != 0.0)?;
    let edge = own_baseline(b);
    let same = |a: f32, c: f32| (a - c).abs() < 1e-4;
    scene.boxes.iter().position(|o| {
        !std::ptr::eq(o, b)
            && o.layer == b.layer
            && same(o.min[0], b.min[0])
            && same(o.max[0], b.max[0])
            && same(o.min[2], b.min[2])
            && same(o.max[2], b.max[2])
            && o.base
                .as_ref()
                .is_some_and(|ob| ob.from_bottom == base.from_bottom)
            && (far_edge(o) - edge).abs() < 1e-3
    })
}

/// The height a box with no counterpart in `other` is flat at: where the segment under it is
/// (see `flat_edge` in `transition.rs`), or its baseline on the scale of `other`.
fn flat_edge(
    i: usize,
    scene: &Scene3d,
    other: &Scene3d,
    counterpart: &[Option<usize>],
    memo: &mut [Option<f32>],
) -> f32 {
    if let Some(y) = memo[i] {
        return y;
    }
    let b = &scene.boxes[i];
    memo[i] = Some(own_baseline(b));
    let y = match segment_below(scene, i) {
        Some(j) => match counterpart[j] {
            Some(c) => far_edge(&other.boxes[c]),
            None => flat_edge(j, scene, other, counterpart, memo),
        },
        None => b
            .base
            .as_ref()
            .and_then(|base: &Base| base_px(base, &scene.axes, &other.axes))
            .unwrap_or_else(|| own_baseline(b)),
    };
    memo[i] = Some(y);
    y
}

/// A marker with no size and no opacity (see `vanished` in `transition.rs`), sitting at its
/// anchor when it has one.
fn vanished(p: &PointMark3, anchors: &HashMap<&str, [f32; 3]>) -> PointMark3 {
    PointMark3 {
        pos: anchors.get(p.key.as_str()).copied().unwrap_or(p.pos),
        size_px: 0.0,
        color: transparent(p.color),
        ..p.clone()
    }
}

fn angles(slices: &[SliceMark]) -> Vec<(f32, f32)> {
    slices.iter().map(|s| (s.start, s.end)).collect()
}

/// Shifts a 3D tick along its axis as `tick_shift` says (only the y axis has ticks that do).
fn shift_y(p: &mut [f32; 3], d: f32) {
    p[1] += d;
}

/// The text a changed string cross-fades into (see `text_pairs` in `transition.rs`).
fn text3_pairs(old: &[Text3], new: &[Text3], delay_of: &dyn Fn(usize) -> f32) -> Vec<Pair<Text3>> {
    let faded = |t: &Text3| Text3 {
        color: transparent(t.color),
        ..t.clone()
    };
    let mut out = Vec::new();
    for change in diff_keys(old, new, |t| &t.key) {
        match change {
            Change::Update { old: o, new: n } if old[o].text == new[n].text => out.push(Pair {
                from: old[o].clone(),
                to: new[n].clone(),
                delay_ms: delay_of(n),
                phase: Phase::Update,
            }),
            Change::Update { old: o, new: n } => {
                let mut gone = old[o].clone();
                gone.key.push_str(OLD_SUFFIX);
                out.push(Pair {
                    from: faded(&new[n]),
                    to: new[n].clone(),
                    delay_ms: delay_of(n),
                    phase: Phase::Enter,
                });
                out.push(Pair {
                    from: gone.clone(),
                    to: faded(&gone),
                    delay_ms: 0.0,
                    phase: Phase::Exit,
                });
            }
            Change::Enter { new: n } => out.push(Pair {
                from: faded(&new[n]),
                to: new[n].clone(),
                delay_ms: delay_of(n),
                phase: Phase::Enter,
            }),
            Change::Exit { old: o } => out.push(Pair {
                from: old[o].clone(),
                to: faded(&old[o]),
                delay_ms: 0.0,
                phase: Phase::Exit,
            }),
        }
    }
    out
}

/// An empty scene with the box, canvas and background of `scene`: what a chart that animates
/// in starts from (so the camera's framing is final from the first frame).
pub fn blank_like(scene: &Scene3d) -> Scene3d {
    scene.clone_shell()
}

impl Plan3 {
    /// The transition from the scene on screen to its replacement.
    pub fn diff(from: &Scene3d, to: &Scene3d, timing: Timing) -> Plan3 {
        let mut plan = Plan3 {
            timing,
            target: to.clone(),
            boxes: Vec::new(),
            lines: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
            slices: Vec::new(),
            polylines: Vec::new(),
            points: Vec::new(),
            overlay: Plan::diff(&overlay_scene(from), &overlay_scene(to), timing),
            from_size: from.size,
            from_plot: from.plot,
            from_axes: from.axes.clone(),
            max_delay_ms: 0.0,
        };

        // Boxes grow from and shrink to their baseline, stacks as one piece.
        let count = to.boxes.len();
        let mut box_delays = vec![0.0; count];
        let changes = diff_keys(&from.boxes, &to.boxes, |b| &b.key);
        let mut new_to_old = vec![None; count];
        let mut old_to_new = vec![None; from.boxes.len()];
        for change in &changes {
            if let Change::Update { old, new } = *change {
                new_to_old[new] = Some(old);
                old_to_new[old] = Some(new);
            }
        }
        let (mut enter_memo, mut exit_memo) = (vec![None; count], vec![None; from.boxes.len()]);
        for change in changes {
            plan.boxes.push(match change {
                Change::Update { old, new } => {
                    box_delays[new] = timing.delay_of(new, count);
                    Pair {
                        from: from.boxes[old].clone(),
                        to: to.boxes[new].clone(),
                        delay_ms: box_delays[new],
                        phase: Phase::Update,
                    }
                }
                Change::Enter { new } => {
                    let y = flat_edge(new, to, from, &new_to_old, &mut enter_memo);
                    box_delays[new] = timing.delay_of(new, count);
                    Pair {
                        from: flattened(&to.boxes[new], y),
                        to: to.boxes[new].clone(),
                        delay_ms: box_delays[new],
                        phase: Phase::Enter,
                    }
                }
                Change::Exit { old } => {
                    let y = flat_edge(old, from, to, &old_to_new, &mut exit_memo);
                    Pair {
                        from: from.boxes[old].clone(),
                        to: flattened(&from.boxes[old], y),
                        delay_ms: 0.0,
                        phase: Phase::Exit,
                    }
                }
            });
        }
        let mut mark_delays = box_delays;

        // Pie slices open and close along their neighbours, as in 2D.
        let count = to.slices.len();
        let changes = diff_keys(&from.slices, &to.slices, |s| &s.key);
        let mut new_to_old = vec![None; count];
        let mut old_to_new = vec![None; from.slices.len()];
        for change in &changes {
            if let Change::Update { old, new } = *change {
                new_to_old[new] = Some(old);
                old_to_new[old] = Some(new);
            }
        }
        let closed = |s: &SliceMark, at: f32| SliceMark {
            start: at,
            end: at,
            ..s.clone()
        };
        for change in changes {
            plan.slices.push(match change {
                Change::Update { old, new } => Pair {
                    from: from.slices[old].clone(),
                    to: to.slices[new].clone(),
                    delay_ms: timing.delay_of(new, count),
                    phase: Phase::Update,
                },
                Change::Enter { new } => {
                    let at =
                        open_seam(new, &angles(&to.slices), &new_to_old, &angles(&from.slices));
                    Pair {
                        from: closed(&to.slices[new], at),
                        to: to.slices[new].clone(),
                        delay_ms: timing.delay_of(new, count),
                        phase: Phase::Enter,
                    }
                }
                Change::Exit { old } => {
                    let at =
                        close_seam(old, &angles(&from.slices), &old_to_new, &angles(&to.slices));
                    Pair {
                        from: from.slices[old].clone(),
                        to: closed(&from.slices[old], at),
                        delay_ms: 0.0,
                        phase: Phase::Exit,
                    }
                }
            });
        }
        mark_delays.extend(
            plan.slices
                .iter()
                .filter(|p| p.phase != Phase::Exit)
                .map(|p| p.delay_ms),
        );

        // Data lines: draw on, fade out, or morph vertex by vertex; markers follow.
        let mut anchors: HashMap<&str, [f32; 3]> = HashMap::new();
        // The points of lines whose vertices could not be matched by point: their markers
        // cross-fade instead of moving, since the line does not move with them.
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
                        from: Polyline3 {
                            runs: a,
                            ..from.polylines[old].clone()
                        },
                        to: Polyline3 {
                            runs: b,
                            ..to.polylines[new].clone()
                        },
                        delay_ms: 0.0,
                        phase: Phase::Update,
                    }
                }
                Change::Enter { new } => Pair {
                    from: Polyline3 {
                        reveal: 0.0,
                        ..to.polylines[new].clone()
                    },
                    to: to.polylines[new].clone(),
                    delay_ms: 0.0,
                    phase: Phase::Enter,
                },
                Change::Exit { old } => Pair {
                    from: from.polylines[old].clone(),
                    to: Polyline3 {
                        color: transparent(from.polylines[old].color),
                        ..from.polylines[old].clone()
                    },
                    delay_ms: 0.0,
                    phase: Phase::Exit,
                },
            });
        }
        let count = to.points.len();
        let mut leaving: Vec<Pair<PointMark3>> = Vec::new();
        for change in diff_keys(&from.points, &to.points, |p| &p.key) {
            plan.points.push(match change {
                Change::Update { old, new } if loose.contains(to.points[new].key.as_str()) => {
                    let mut gone = vanished(&from.points[old], &anchors);
                    gone.pos = from.points[old].pos;
                    gone.key.push_str(OLD_SUFFIX);
                    leaving.push(Pair {
                        from: PointMark3 {
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
        mark_delays.extend(
            plan.points
                .iter()
                .filter(|p| p.phase != Phase::Exit)
                .map(|p| p.delay_ms),
        );
        plan.points.extend(leaving);

        // Grid, axis and tick lines. A tick of the value axis enters from where the old scale
        // puts it and leaves toward where the new one does, fading.
        for change in diff_keys(&from.lines, &to.lines, |l| &l.key) {
            plan.lines.push(match change {
                Change::Update { old, new } => Pair {
                    from: from.lines[old].clone(),
                    to: to.lines[new].clone(),
                    delay_ms: 0.0,
                    phase: Phase::Update,
                },
                Change::Enter { new } => {
                    let mut start = to.lines[new].clone();
                    start.color = transparent(start.color);
                    if let Some((_, d)) = tick_shift(start.tick, &to.axes, &from.axes) {
                        shift_y(&mut start.a, d);
                        shift_y(&mut start.b, d);
                    }
                    Pair {
                        from: start,
                        to: to.lines[new].clone(),
                        delay_ms: 0.0,
                        phase: Phase::Enter,
                    }
                }
                Change::Exit { old } => {
                    let mut end = from.lines[old].clone();
                    end.color = transparent(end.color);
                    if let Some((_, d)) = tick_shift(end.tick, &from.axes, &to.axes) {
                        shift_y(&mut end.a, d);
                        shift_y(&mut end.b, d);
                    }
                    Pair {
                        from: from.lines[old].clone(),
                        to: end,
                        delay_ms: 0.0,
                        phase: Phase::Exit,
                    }
                }
            });
        }

        // World text.
        plan.texts = text3_pairs(&from.texts, &to.texts, &|_| 0.0);
        for pair in &mut plan.texts {
            match pair.phase {
                Phase::Enter => {
                    if let Some((_, d)) = tick_shift(pair.to.tick, &to.axes, &from.axes) {
                        shift_y(&mut pair.from.anchor, d);
                    }
                }
                Phase::Exit => {
                    if let Some((_, d)) = tick_shift(pair.from.tick, &from.axes, &to.axes) {
                        shift_y(&mut pair.to.anchor, d);
                    }
                }
                Phase::Update => {}
            }
        }
        // Value labels follow their mark's delay.
        let old_texts: Vec<Text3> = from.labels.iter().map(|l| l.text.clone()).collect();
        let new_texts: Vec<Text3> = to.labels.iter().map(|l| l.text.clone()).collect();
        let label_delay = |i: usize| mark_delays.get(to.labels[i].mark).copied().unwrap_or(0.0);
        plan.labels = text3_pairs(&old_texts, &new_texts, &label_delay)
            .into_iter()
            .map(|p| {
                let source = match p.phase {
                    Phase::Exit => from.labels.iter().find(|l| l.text.key == p.from.key),
                    _ => to.labels.iter().find(|l| l.text.key == p.to.key),
                };
                let (mark, show) =
                    source.map_or((usize::MAX, crate::spec::LabelShow::Always), |l| {
                        (
                            if p.phase == Phase::Exit {
                                usize::MAX
                            } else {
                                l.mark
                            },
                            l.show,
                        )
                    });
                Pair {
                    from: MarkLabel3 {
                        mark,
                        show,
                        text: p.from,
                    },
                    to: MarkLabel3 {
                        mark,
                        show,
                        text: p.to,
                    },
                    delay_ms: p.delay_ms,
                    phase: p.phase,
                }
            })
            .collect();

        let delays = (plan.boxes.iter().map(|p| p.delay_ms))
            .chain(plan.labels.iter().map(|p| p.delay_ms))
            .chain(plan.slices.iter().map(|p| p.delay_ms))
            .chain(plan.points.iter().map(|p| p.delay_ms));
        for d in delays {
            plan.max_delay_ms = plan.max_delay_ms.max(d);
        }
        plan
    }

    /// How long the whole transition lasts, stagger included.
    pub fn total_ms(&self) -> f32 {
        self.timing.total_ms(self.max_delay_ms)
    }

    /// Whether anything differs between the two scenes.
    pub fn is_static(&self) -> bool {
        fn calm<T: PartialEq>(pairs: &[Pair<T>]) -> bool {
            pairs
                .iter()
                .all(|p| p.phase == Phase::Update && p.from == p.to)
        }
        calm(&self.boxes)
            && calm(&self.lines)
            && calm(&self.texts)
            && calm(&self.labels)
            && calm(&self.slices)
            && calm(&self.polylines)
            && calm(&self.points)
            && self.overlay.is_static()
            && self.from_size == self.target.size
            && self.from_plot == self.target.plot
    }

    /// Whether anything is tessellated on the CPU (everything but boxes and text).
    pub fn has_marks(&self) -> bool {
        !(self.lines.is_empty()
            && self.slices.is_empty()
            && self.polylines.is_empty()
            && self.points.is_empty())
    }

    /// The plot box at this moment: it grows or shrinks to the new size with the same easing
    /// as the axes, so the camera's framing of it changes smoothly.
    pub fn size_at(&self, elapsed_ms: f32) -> [f32; 3] {
        let e = self.timing.eased(elapsed_ms, 0.0);
        let to = self.target.size;
        std::array::from_fn(|i| lerp(self.from_size[i], to[i], e))
    }

    /// The part of the canvas the plot is framed in at this moment.
    pub fn plot_at(&self, elapsed_ms: f32) -> Option<[f32; 4]> {
        let e = self.timing.eased(elapsed_ms, 0.0);
        let canvas = |s: &Scene3d| s.plot_rect();
        // A plot that fills the canvas on one side and not on the other is interpolated from
        // the whole canvas, so adding a legend shrinks it gradually.
        let (from, to) = (
            self.from_plot.unwrap_or([
                0.0,
                0.0,
                self.target.width as f32,
                self.target.height as f32,
            ]),
            canvas(&self.target),
        );
        if self.from_plot.is_none() && self.target.plot.is_none() {
            return None;
        }
        Some(std::array::from_fn(|i| lerp(from[i], to[i], e)))
    }

    /// A scene with the scalar fields of the target (the box and the plot at this moment) and
    /// none of its items.
    fn shell(&self, elapsed_ms: f32) -> Scene3d {
        Scene3d {
            plot: self.plot_at(elapsed_ms),
            size: self.size_at(elapsed_ms),
            ..self.target.clone_shell()
        }
    }

    /// A scene holding only what the CPU tessellates (lines, slices, data lines, markers), as
    /// it is `elapsed_ms` into the transition: what the renderer redoes each frame.
    pub fn marks_at(&self, elapsed_ms: f32) -> Scene3d {
        let mut scene = self.shell(elapsed_ms);
        self.fill_marks(&mut scene, elapsed_ms);
        scene
    }

    /// Fills `scene`'s lines, slices, data lines and markers with their state `elapsed_ms` in.
    /// When `scene` already holds this plan's (from an earlier call) only the numbers are
    /// rewritten, so a frame allocates next to nothing (keys are not copied again).
    pub fn fill_marks(&self, scene: &mut Scene3d, elapsed_ms: f32) {
        let t = |delay: f32| self.timing.eased(elapsed_ms, delay);
        if scene.lines.len() == self.lines.len() {
            for (l, p) in scene.lines.iter_mut().zip(&self.lines) {
                let e = t(p.delay_ms);
                l.a = std::array::from_fn(|i| lerp(p.from.a[i], p.to.a[i], e));
                l.b = std::array::from_fn(|i| lerp(p.from.b[i], p.to.b[i], e));
                l.width_px = lerp(p.from.width_px, p.to.width_px, e);
                l.color = lerp_color(p.from.color, p.to.color, e);
            }
        } else {
            scene.lines = (self.lines.iter())
                .map(|p| morph_line(p, t(p.delay_ms)))
                .collect();
        }
        scene.slices = (self.slices.iter())
            .map(|p| morph_slice(p, t(p.delay_ms)))
            .collect();
        if scene.polylines.len() == self.polylines.len() {
            for (l, p) in scene.polylines.iter_mut().zip(&self.polylines) {
                let e = t(p.delay_ms);
                mix_runs_into(&mut l.runs, &p.from.runs, &p.to.runs, e);
                l.color = lerp_color(p.from.color, p.to.color, e);
                l.width_px = lerp(p.from.width_px, p.to.width_px, e);
                l.area = match (p.from.area, p.to.area) {
                    (Some(a), Some(b)) => Some(lerp(a, b, e)),
                    (_, b) => b.or(p.from.area),
                };
                let reveal = lerp(p.from.reveal, p.to.reveal, e);
                if reveal < 1.0 {
                    l.runs = reveal_runs(&l.runs, reveal);
                }
            }
        } else {
            scene.polylines = (self.polylines.iter())
                .map(|p| morph_polyline(p, t(p.delay_ms)))
                .collect();
        }
        if scene.points.len() == self.points.len() {
            for (m, p) in scene.points.iter_mut().zip(&self.points) {
                let e = t(p.delay_ms);
                m.pos = std::array::from_fn(|i| lerp(p.from.pos[i], p.to.pos[i], e));
                m.size_px = lerp(p.from.size_px, p.to.size_px, e).max(0.0);
                m.color = lerp_color(p.from.color, p.to.color, e);
            }
        } else {
            scene.points = (self.points.iter())
                .map(|p| morph_point(p, t(p.delay_ms)))
                .collect();
        }
    }

    /// The scene as it looks `elapsed_ms` into the transition: every item mixed between its two
    /// states, exits still drawn. At or after [`total_ms`](Self::total_ms) it is the target
    /// exactly.
    pub fn sample(&self, elapsed_ms: f32) -> Scene3d {
        if elapsed_ms >= self.total_ms() {
            return self.target.clone();
        }
        let mut scene = self.shell(elapsed_ms);
        let t = |delay: f32| self.timing.eased(elapsed_ms, delay);
        scene.boxes = self
            .boxes
            .iter()
            .map(|p| morph_box(p, t(p.delay_ms)))
            .collect();
        scene.texts = self
            .texts
            .iter()
            .map(|p| morph_text(p, t(p.delay_ms)))
            .collect();
        scene.labels = self
            .labels
            .iter()
            .map(|p| MarkLabel3 {
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
        let overlay = self.overlay.sample(elapsed_ms);
        scene.overlay = Overlay {
            rules: overlay.rules,
            texts: overlay.texts,
            hits: self.target.overlay.hits.clone(),
        };
        // The value axis slides with the transition, so the next diff starts where it is.
        scene.axes = lerp_axes(&self.from_axes, &self.target.axes, t(0.0));
        scene
    }
}

pub fn morph_box(p: &Pair<BoxMark>, e: f32) -> BoxMark {
    let mut b = p.to.clone();
    b.min = std::array::from_fn(|i| lerp(p.from.min[i], p.to.min[i], e));
    b.max = std::array::from_fn(|i| lerp(p.from.max[i], p.to.max[i], e));
    // An overshooting easing must not turn a box inside out.
    for i in 0..3 {
        if b.max[i] < b.min[i] {
            let mid = (b.max[i] + b.min[i]) / 2.0;
            (b.min[i], b.max[i]) = (mid, mid);
        }
    }
    b.color = lerp_color(p.from.color, p.to.color, e);
    b
}

pub fn morph_line(p: &Pair<Line3>, e: f32) -> Line3 {
    let mut l = p.to.clone();
    l.a = std::array::from_fn(|i| lerp(p.from.a[i], p.to.a[i], e));
    l.b = std::array::from_fn(|i| lerp(p.from.b[i], p.to.b[i], e));
    l.width_px = lerp(p.from.width_px, p.to.width_px, e);
    l.color = lerp_color(p.from.color, p.to.color, e);
    l
}

pub fn morph_text(p: &Pair<Text3>, e: f32) -> Text3 {
    let mut t = p.to.clone();
    t.anchor = std::array::from_fn(|i| lerp(p.from.anchor[i], p.to.anchor[i], e));
    t.offset_px = std::array::from_fn(|i| lerp(p.from.offset_px[i], p.to.offset_px[i], e));
    t.size_px = lerp(p.from.size_px, p.to.size_px, e);
    t.color = lerp_color(p.from.color, p.to.color, e);
    t
}

pub fn morph_slice(p: &Pair<SliceMark>, e: f32) -> SliceMark {
    let mut s = p.to.clone();
    let l = |f: f32, t: f32| lerp(f, t, e);
    s.center = [
        l(p.from.center[0], p.to.center[0]),
        l(p.from.center[1], p.to.center[1]),
    ];
    s.inner = l(p.from.inner, p.to.inner).max(0.0);
    s.outer = l(p.from.outer, p.to.outer).max(0.0);
    s.height = l(p.from.height, p.to.height).max(0.0);
    s.start = lerp_angle(p.from.start, p.to.start, e);
    s.end = lerp_angle(p.from.end, p.to.end, e).max(s.start);
    s.pad = l(p.from.pad, p.to.pad);
    s.color = lerp_color(p.from.color, p.to.color, e);
    s
}

pub fn morph_polyline(p: &Pair<Polyline3>, e: f32) -> Polyline3 {
    let mut l = p.to.clone();
    l.runs = mix_runs(&p.from.runs, &p.to.runs, e);
    l.color = lerp_color(p.from.color, p.to.color, e);
    l.width_px = lerp(p.from.width_px, p.to.width_px, e);
    // The curtain's baseline follows when both ends have one.
    l.area = match (p.from.area, p.to.area) {
        (Some(a), Some(b)) => Some(lerp(a, b, e)),
        (_, b) => b.or(p.from.area),
    };
    let reveal = lerp(p.from.reveal, p.to.reveal, e);
    if reveal < 1.0 {
        l.runs = reveal_runs(&l.runs, reveal);
    }
    l.reveal = 1.0;
    l
}

pub fn morph_point(p: &Pair<PointMark3>, e: f32) -> PointMark3 {
    let mut m = p.to.clone();
    m.pos = std::array::from_fn(|i| lerp(p.from.pos[i], p.to.pos[i], e));
    m.size_px = lerp(p.from.size_px, p.to.size_px, e).max(0.0);
    m.color = lerp_color(p.from.color, p.to.color, e);
    m
}
