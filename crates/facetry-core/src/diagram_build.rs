//! A diagram's build-in: the picture assembling itself in the order things flow. Group bases
//! come up first, then the nodes rise rank by rank from the sources, each edge drawing itself
//! out from its tail toward the node it leads to, and its head, label and moving marks
//! appearing as it arrives.

use crate::Color;
use crate::scene3d::{BuildStep, Polyline3, PrismMark, RibbonMark, Scene3d, Text3};
use crate::spec::DiagramSpec;

/// How long a node takes to rise and an edge to draw out, ms.
const RISE_MS: f32 = 420.0;
const DRAW_MS: f32 = 380.0;
/// From one rank's start to the next's: the edge leaves halfway up its node, and the next node
/// starts rising a little before the edge arrives.
const RANK_MS: f32 = RISE_MS * 0.5 + DRAW_MS * 0.8;
/// A group's base comes up this long before its first node.
const GROUP_LEAD_MS: f32 = 160.0;
/// The whole build-in takes at most this long: a deep diagram builds faster.
const MAX_MS: f32 = 3200.0;
/// An edge's head, label and the like grow over this last share of its drawing.
const ARRIVAL: f32 = 0.3;

/// When each node, edge (`edge{i}`) and group of `diagram` builds in.
pub(crate) fn schedule(diagram: &DiagramSpec) -> Vec<BuildStep> {
    let n = diagram.nodes.len();
    // Lists rather than maps throughout: a diagram has a few hundred parts at most, and a
    // hash map would cost the wasm module kilobytes.
    let index = |id: &str| diagram.nodes.iter().position(|node| node.id == id);
    // An edge to or from a group stands for its nodes: the nearest one counts.
    let ends = |id: &str| -> Vec<usize> {
        match index(id) {
            Some(i) => vec![i],
            None => diagram
                .groups
                .iter()
                .find(|g| g.id == id)
                .map(|g| {
                    (0..n)
                        .filter(|&i| inside(diagram, i, g.x, g.y, g.width, g.height))
                        .collect()
                })
                .unwrap_or_default(),
        }
    };
    let links: Vec<(Vec<usize>, Vec<usize>)> = diagram
        .edges
        .iter()
        .map(|e| (ends(&e.from), ends(&e.to)))
        .collect();
    // Ranks by breadth-first search from the nodes nothing leads to (in their order), then from
    // any node still unreached (a cycle).
    let mut rank = vec![usize::MAX; n];
    let mut incoming = vec![false; n];
    for (_, to) in &links {
        for &t in to {
            incoming[t] = true;
        }
    }
    let mut queue: Vec<usize> = (0..n).filter(|&i| !incoming[i]).collect();
    for &i in &queue {
        rank[i] = 0;
    }
    let mut head = 0;
    loop {
        while head < queue.len() {
            let a = queue[head];
            head += 1;
            for (from, to) in &links {
                if from.contains(&a) {
                    for &b in to {
                        if rank[b] == usize::MAX {
                            rank[b] = rank[a] + 1;
                            queue.push(b);
                        }
                    }
                }
            }
        }
        match (0..n).find(|&i| rank[i] == usize::MAX) {
            Some(i) => {
                rank[i] = 0;
                queue.push(i);
            }
            None => break,
        }
    }
    let start = |r: usize| GROUP_LEAD_MS + r as f32 * RANK_MS;
    let mut steps = Vec::new();
    for (i, node) in diagram.nodes.iter().enumerate() {
        steps.push(BuildStep {
            owner: node.id.clone(),
            start_ms: start(rank[i]),
            duration_ms: RISE_MS,
            draw: false,
        });
    }
    for (k, (from, _)) in links.iter().enumerate() {
        let r = from.iter().map(|&i| rank[i]).min().unwrap_or(0);
        steps.push(BuildStep {
            owner: format!("edge{k}"),
            start_ms: start(r) + RISE_MS * 0.5,
            duration_ms: DRAW_MS,
            draw: true,
        });
    }
    for g in &diagram.groups {
        let first = (0..n)
            .filter(|&i| inside(diagram, i, g.x, g.y, g.width, g.height))
            .map(|i| rank[i])
            .min()
            .unwrap_or(0);
        steps.push(BuildStep {
            owner: g.id.clone(),
            start_ms: start(first) - GROUP_LEAD_MS,
            duration_ms: RISE_MS,
            draw: false,
        });
    }
    let total = steps
        .iter()
        .map(|s| s.start_ms + s.duration_ms)
        .fold(0.0, f32::max);
    if total > MAX_MS {
        let k = MAX_MS / total;
        for s in &mut steps {
            s.start_ms *= k;
            s.duration_ms *= k;
        }
    }
    steps
}

/// Whether node `i`'s centre is in the box centred on `(x, y)`.
fn inside(diagram: &DiagramSpec, i: usize, x: f64, y: f64, w: f64, h: f64) -> bool {
    let node = &diagram.nodes[i];
    (node.x - x).abs() <= w / 2.0 && (node.y - y).abs() <= h / 2.0
}

/// Slows down toward the end.
fn ease(p: f32) -> f32 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
}

/// The part of a key that names what it belongs to: a node's or group's id, or `edge{i}`.
fn owner(key: &str) -> &str {
    key.split('/').next().unwrap_or(key)
}

/// How far along an owner is at the time asked for.
#[derive(Clone, Copy)]
struct Progress {
    /// 0 before it starts, 1 once done.
    p: f32,
    draw: bool,
}

impl Progress {
    /// How grown its solids are: a node's or group's as it rises; an edge's head, label and the
    /// like as it arrives.
    fn grown(self) -> f32 {
        if self.draw {
            ease((self.p - (1.0 - ARRIVAL)) / ARRIVAL)
        } else {
            ease(self.p)
        }
    }
}

fn faded(c: Color, k: f32) -> Color {
    let [r, g, b, a] = c.to_array();
    Color::linear(r, g, b, a * k)
}

fn raise(p: &mut PrismMark, k: f32) {
    p.base *= k;
    p.top *= k;
    if let Some(s) = &mut p.stand {
        s.origin[1] *= k;
    }
}

/// `path` cut to its first `keep` (world units, along the floor), with `lift` cut alike.
fn cut(path: &[[f32; 2]], lift: &[f32], keep: f32) -> (Vec<[f32; 2]>, Vec<f32>) {
    let mut out = vec![path[0]];
    let mut lifts = vec![lift.first().copied().unwrap_or(0.0)];
    let mut left = keep;
    for i in 1..path.len() {
        let ([ax, az], [bx, bz]) = (path[i - 1], path[i]);
        let len = ((bx - ax).powi(2) + (bz - az).powi(2)).sqrt();
        let l = |j: usize| lift.get(j).copied().unwrap_or(0.0);
        if len >= left {
            let t = if len > 0.0 { left / len } else { 0.0 };
            out.push([ax + (bx - ax) * t, az + (bz - az) * t]);
            lifts.push(l(i - 1) + (l(i) - l(i - 1)) * t);
            break;
        }
        out.push(path[i]);
        lifts.push(l(i));
        left -= len;
    }
    if lift.is_empty() {
        lifts.clear();
    }
    (out, lifts)
}

fn floor_length(path: &[[f32; 2]]) -> f32 {
    path.windows(2)
        .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
        .sum()
}

/// A polyline's runs cut to their first `keep` world units (along the runs, in order).
fn cut_runs(line: &mut Polyline3, keep: f32) {
    let mut left = keep;
    let mut runs = Vec::new();
    for run in &line.runs {
        if left <= 0.0 || run.len() < 2 {
            break;
        }
        let mut out = vec![run[0]];
        for w in run.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len =
                ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
            if len >= left {
                let t = if len > 0.0 { left / len } else { 0.0 };
                out.push([
                    a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t,
                    a[2] + (b[2] - a[2]) * t,
                ]);
                left = 0.0;
                break;
            }
            out.push(b);
            left -= len;
        }
        runs.push(out);
    }
    line.runs = runs;
}

fn run_length(line: &Polyline3) -> f32 {
    line.runs
        .iter()
        .flat_map(|r| r.windows(2))
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt()
        })
        .sum()
}

impl Scene3d {
    /// How long the build-in takes, ms (0 for a scene without one).
    pub fn build_ms(&self) -> f32 {
        self.build
            .iter()
            .map(|s| s.start_ms + s.duration_ms)
            .fold(0.0, f32::max)
    }

    /// The scene `ms` into its build-in: what has not begun left out, nodes and groups rising
    /// from the floor, edges drawn out part of the way. What belongs to nothing in the
    /// schedule (a floor grid) is there from the start.
    pub fn built_at(&self, ms: f32) -> Scene3d {
        let progress = |key: &str| -> Progress {
            let owner = owner(key);
            match self.build.iter().find(|s| s.owner == owner) {
                Some(s) => Progress {
                    p: ((ms - s.start_ms) / s.duration_ms.max(1e-3)).clamp(0.0, 1.0),
                    draw: s.draw,
                },
                None => Progress {
                    p: 1.0,
                    draw: false,
                },
            }
        };
        let mut out = self.clone();
        out.build.clear();
        // Solids: grown, or left out until they begin.
        let grow = |marks: &mut Vec<PrismMark>| {
            marks.retain_mut(|m| {
                let k = progress(&m.key).grown();
                raise(m, k);
                k > 0.0
            });
        };
        grow(&mut out.prisms);
        grow(&mut out.decor);
        // Edges' bands and lines drawn out along their length; a group's rails rise.
        // Per edge: how much of its bands came before (in order), and their whole length.
        let mut drawn: Vec<(&str, f32, f32)> = Vec::new();
        for r in &self.ribbons {
            let o = owner(&r.key);
            let len = floor_length(&r.path);
            match drawn.iter_mut().find(|d| d.0 == o) {
                Some(d) => d.2 += len,
                None => drawn.push((o, 0.0, len)),
            }
        }
        let mut bands: Vec<RibbonMark> = Vec::new();
        for mut r in std::mem::take(&mut out.ribbons) {
            let pr = progress(&r.key);
            if !pr.draw {
                let k = pr.grown();
                if k <= 0.0 {
                    continue;
                }
                (r.base, r.top) = (r.base * k, r.top * k);
                r.lift.iter_mut().for_each(|l| *l *= k);
                bands.push(r);
                continue;
            }
            let len = floor_length(&r.path);
            let o = owner(&r.key);
            let Some(d) = drawn.iter_mut().find(|d| d.0 == o) else {
                continue;
            };
            let keep = ease(pr.p) * d.2 - d.1;
            d.1 += len;
            if keep <= 0.0 {
                continue;
            }
            if keep < len {
                (r.path, r.lift) = cut(&r.path, &r.lift, keep);
            }
            if r.path.len() >= 2 {
                bands.push(r);
            }
        }
        out.ribbons = bands;
        out.polylines.retain_mut(|line| {
            let pr = progress(&line.key);
            if pr.draw && line.key == owner(&line.key) {
                let total = run_length(line);
                cut_runs(line, ease(pr.p) * total);
                return !line.runs.is_empty();
            }
            let k = pr.grown();
            for run in &mut line.runs {
                for p in run.iter_mut() {
                    p[1] *= k;
                }
            }
            k > 0.0
        });
        // Rims round node tops rise with them.
        out.lines.retain_mut(|l| {
            let k = progress(&l.key).grown();
            (l.a[1], l.b[1]) = (l.a[1] * k, l.b[1] * k);
            k > 0.0
        });
        // Printed text and filled icons rise and fade in with what they are on.
        let show = |t: &mut Text3| {
            let k = progress(&t.key).grown();
            t.anchor[1] *= k;
            t.color = faded(t.color, k);
            k > 0.0
        };
        out.texts.retain_mut(show);
        out.fills.retain_mut(|f| {
            let k = progress(&f.key).grown();
            f.triangles.iter_mut().for_each(|p| p[1] *= k);
            f.color = faded(f.color, k);
            k > 0.0
        });
        out.shadows.retain_mut(|s| {
            let k = progress(&s.key).grown();
            s.color = faded(s.color, k);
            s.lift *= k;
            k > 0.0
        });
        out.signs.retain_mut(|sign| {
            let k = progress(&sign.key).grown();
            for p in &mut sign.prisms {
                raise(p, k);
            }
            for t in &mut sign.texts {
                t.anchor[1] *= k;
                t.color = faded(t.color, k);
            }
            for l in &mut sign.polylines {
                l.runs.iter_mut().flatten().for_each(|p| p[1] *= k);
            }
            for f in &mut sign.fills {
                f.triangles.iter_mut().for_each(|p| p[1] *= k);
                f.color = faded(f.color, k);
            }
            k > 0.0
        });
        // Marks move along an edge once it is there.
        out.flows.retain(|f| progress(&f.key).p >= 1.0);
        out
    }
}
