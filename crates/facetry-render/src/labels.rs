//! Per-frame placement of the 3D axis labels.
//!
//! Which axis labels are readable depends on the camera: look along an axis and its labels
//! land on top of each other; look from the side and they are spread out. The scene cannot
//! know (it is built once, whatever the camera does), so each frame the renderer asks
//! [`LabelPlan::states`] for a tiny table with one entry per label: *draw it or not*, and *by
//! how many pixels to nudge it*. The vertex shader reads its entry and acts on it
//! (`text3d.wgsl`). The glyph instances themselves are never rewritten.
//!
//! Why on the CPU: it takes a few dozen labels, a handful of projections and rectangle tests
//! (microseconds), and the rules ("does this label's rectangle touch another?") are easy to
//! write and to unit-test in Rust but awkward in a shader, which cannot see its neighbours.
//! The table is 4 KiB, written only when the camera or viewport changed.
//!
//! The rules, for the labels of the sector the camera is in:
//!
//! 1. **Clear of the box.** A label is moved away from the plot box's projected centre, in
//!    screen space, just far enough that its rectangle no longer touches the box's projected
//!    outline (the six faces' outlines, whose union is the box's silhouette). Edges seen nearly end-on run *inside* that outline (the far end of the edge is
//!    hidden behind the near bars), so their labels would otherwise be drawn over bars.
//! 2. **Never on top of each other.** Labels are visited in priority order (titles, then
//!    y ticks, then the other axes' ticks; within an axis the first and last label, then the
//!    middle one, then the middles of each half, ...) and one is hidden if its rectangle
//!    touches one already kept. An axis seen end-on therefore keeps its first and last label
//!    (if they are apart) or just one, and always its title.

use facetry_core::scene::Anchor;
use facetry_core::scene3d::{AXIS_X, AXIS_Y, AXIS_Z, AzimuthRange, LabelRole, Scene3d, VAlign};
use facetry_font::FontMetrics;
use glam::{Mat4, Vec2, Vec3};

use crate::pick::{ray_box, ray_through_pixel};
use crate::text::NO_SLOT;

/// Entries in the label-state table (matches `LabelStates` in `text3d.wgsl`): the first half
/// for axis labels (numbered within each azimuth sector), the second for value labels that
/// hide while their box is out of sight.
pub const SLOTS: usize = 512;
/// Axis labels use slots `0..SECTOR_SLOTS`; the last of those is never visible: labels beyond
/// the capacity point at it.
const SECTOR_SLOTS: usize = SLOTS / 2;
const OVERFLOW_SLOT: usize = SECTOR_SLOTS - 1;
/// Value labels on boxes use `VALUE_SLOTS..SLOTS - 1`; the very last slot is the overflow.
const VALUE_SLOTS: usize = SLOTS / 2;
/// How far (world units) in front of a point a box may start and still leave the point
/// visible.
pub const POINT_SLACK: f32 = 0.02;
/// How far (pixels) a label may be pushed to clear the box before it is given up on.
const MAX_PUSH: i32 = 240;

/// One entry per slot: `[visible (0 or 1), nudge x, nudge y, spare]`, nudge in pixels.
pub type LabelStates = [[f32; 4]; SLOTS];

/// A label the plan manages.
#[derive(Debug, Clone)]
struct Entry {
    slot: usize,
    /// Order of consideration: smaller goes first (and so wins an overlap).
    priority: (u8, u32),
    range: Option<AzimuthRange>,
    anchor: Vec3,
    offset: Vec2,
    align: Anchor,
    valign: VAlign,
    width: f32,
    cap: f32,
    descent: f32,
}

impl Entry {
    /// The screen rectangle `[x0, y0, x1, y1]` the text covers when its anchor is at `anchor_px`.
    fn rect(&self, anchor_px: Vec2) -> [f32; 4] {
        let (ax, ay) = (anchor_px.x + self.offset.x, anchor_px.y + self.offset.y);
        let x0 = match self.align {
            Anchor::Start => ax,
            Anchor::Middle => ax - self.width / 2.0,
            Anchor::End => ax - self.width,
        };
        let (top, bottom) = match self.valign {
            VAlign::Top => (ay, ay + self.cap + self.descent),
            VAlign::Middle => (ay - self.cap / 2.0, ay + self.cap / 2.0 + self.descent),
            VAlign::Baseline => (ay - self.cap, ay + self.descent),
        };
        [x0, top, x0 + self.width, bottom]
    }
}

/// What [`LabelPlan::place`] decided for one label (slot, verdict, nudge, where it ends up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub slot: usize,
    pub visible: bool,
    pub push: [f32; 2],
    /// Screen rectangle `[x0, y0, x1, y1]` in pixels, with the nudge applied.
    pub rect: [f32; 4],
}

/// The managed labels of a 3D scene.
#[derive(Debug, Clone)]
pub struct LabelPlan {
    entries: Vec<Entry>,
    size: Vec3,
    /// Clear space kept between labels, device pixels.
    gap: f32,
    /// Value labels that are only visible while their box is (see [`LabelRole::OnMark`]),
    /// the boxes they test against, and the slot of every one of `scene.labels` ([`NO_SLOT`]
    /// for the rest).
    on_boxes: Vec<OnBox>,
    boxes: Vec<(Vec3, Vec3)>,
    label_slots: Vec<f32>,
}

/// A value label that is shown only while its anchor can be seen: a label printed on a box
/// (`mark` is that box: it must be the first thing the camera sees there) or one hanging from
/// a line's data point (`mark` is `None`: no box may be in front of the point).
#[derive(Debug, Clone)]
struct OnBox {
    slot: usize,
    mark: Option<usize>,
    anchor: Vec3,
    /// A point's label also gives way to one already shown (see [`LabelPlan::states`]), so it
    /// carries its size.
    entry: Entry,
}

impl LabelPlan {
    /// Collects the scene's per-frame labels and numbers them. Returns the plan and, for each
    /// of `scene.texts`, the slot to put in its glyph instances ([`NO_SLOT`] for labels that
    /// always show).
    ///
    /// Slots are numbered within each azimuth sector (only one sector is ever on screen), so
    /// the table has to hold one sector's labels, not all eight's. A sector with more than
    /// `SLOTS - 1` labels shows none of the excess.
    pub fn new(scene: &Scene3d, font: &FontMetrics) -> (Self, Vec<f32>) {
        // Next free slot per sector (index 8: labels that belong to no sector).
        let mut next = [0usize; 9];
        let mut entries = Vec::new();
        let mut slots = Vec::with_capacity(scene.texts.len());
        for t in &scene.texts {
            let (class, rank) = match t.role {
                // Always shown here (an `OnMark` label is a value label, planned below).
                LabelRole::Fixed | LabelRole::OnMark(_) | LabelRole::OnPoint => {
                    slots.push(NO_SLOT);
                    continue;
                }
                LabelRole::Title { group } => (0, u32::from(group_order(group))),
                LabelRole::Tick {
                    group,
                    index,
                    count,
                } => (
                    1 + group_order(group),
                    bisect_rank(usize::from(index), usize::from(count)) as u32,
                ),
            };
            let sector = &mut next[t
                .azimuths
                .map_or(8, |r| (r.from / 45.0).round() as usize % 8)];
            let slot = (*sector).min(OVERFLOW_SLOT);
            if slot < OVERFLOW_SLOT {
                entries.push(Entry {
                    slot,
                    priority: (class, rank),
                    range: t.azimuths,
                    anchor: Vec3::from(t.anchor),
                    offset: Vec2::from(t.offset_px),
                    align: t.align,
                    valign: t.valign,
                    width: font.text_width(&t.text, t.size_px),
                    cap: font.cap_height * t.size_px,
                    descent: -font.descender * t.size_px,
                });
            }
            *sector += 1;
            slots.push(slot as f32);
        }
        // Value labels on boxes: one slot each, past the axis labels'.
        let mut on_boxes = Vec::new();
        let mut label_slots = Vec::with_capacity(scene.labels.len());
        for l in &scene.labels {
            let mark = match l.text.role {
                LabelRole::OnMark(mark) => Some(Some(mark as usize)),
                LabelRole::OnPoint => Some(None),
                _ => None,
            };
            match mark {
                Some(mark) if l.show == facetry_core::spec::LabelShow::Always => {
                    let slot = (VALUE_SLOTS + on_boxes.len()).min(SLOTS - 1);
                    if slot < SLOTS - 1 {
                        on_boxes.push(OnBox {
                            slot,
                            mark,
                            anchor: Vec3::from(l.text.anchor),
                            entry: Entry {
                                slot,
                                priority: (0, 0),
                                range: None,
                                anchor: Vec3::from(l.text.anchor),
                                offset: Vec2::from(l.text.offset_px),
                                align: l.text.align,
                                valign: l.text.valign,
                                width: font.text_width(&l.text.text, l.text.size_px),
                                cap: font.cap_height * l.text.size_px,
                                descent: -font.descender * l.text.size_px,
                            },
                        });
                    }
                    label_slots.push(slot as f32);
                }
                _ => label_slots.push(NO_SLOT),
            }
        }
        let boxes = if on_boxes.is_empty() {
            Vec::new()
        } else {
            scene
                .boxes
                .iter()
                .map(|b| (Vec3::from(b.min), Vec3::from(b.max)))
                .collect()
        };
        let plan = Self {
            entries,
            size: Vec3::from(scene.size),
            gap: 4.0 * scene.halo_px,
            on_boxes,
            boxes,
            label_slots,
        };
        (plan, slots)
    }

    /// The slot for each of `scene.labels` ([`NO_SLOT`] where the label always shows).
    pub fn label_slots(&self) -> &[f32] {
        &self.label_slots
    }

    /// Decides every label of the sector `azimuth` (degrees) is in, for the given
    /// world-to-clip matrix and viewport (pixels).
    pub fn place(&self, view_proj: Mat4, viewport: [f32; 2], azimuth: f32) -> Vec<Placed> {
        // The same projection the vertex shader does, down to rounding the anchor to a pixel.
        let project = |p: Vec3| {
            let c = view_proj * p.extend(1.0);
            Vec2::new(
                (c.x / c.w * 0.5 + 0.5) * viewport[0],
                (0.5 - c.y / c.w * 0.5) * viewport[1],
            )
        };
        let silhouette = box_faces(self.size.to_array(), project);
        let centre = project(self.size / 2.0);
        let margin = self.gap / 2.0;

        let mut active: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| e.range.is_none_or(|r| r.contains(azimuth)))
            .collect();
        active.sort_by_key(|e| e.priority);

        let mut kept: Vec<[f32; 4]> = Vec::new();
        let mut out = Vec::with_capacity(active.len());
        for e in active {
            let anchor = project(e.anchor).round();
            let away = (anchor - centre).try_normalize().unwrap_or(Vec2::X);
            // The smallest nudge that takes the label's rectangle off the box's outline.
            let pushed = (0..=MAX_PUSH).find_map(|t| {
                let push = (away * t as f32).round();
                (!hits_any(inflate(e.rect(anchor + push), margin), &silhouette)).then_some(push)
            });
            let (push, rect) = match pushed {
                Some(push) => (push, e.rect(anchor + push)),
                None => (Vec2::ZERO, e.rect(anchor)),
            };
            let visible = pushed.is_some()
                && !kept
                    .iter()
                    .any(|k| rects_overlap(inflate(*k, margin), inflate(rect, margin)));
            if visible {
                kept.push(rect);
            }
            out.push(Placed {
                slot: e.slot,
                visible,
                push: push.to_array(),
                rect,
            });
        }
        out
    }

    /// The table the shader reads: entry `slot` is `[1, nudge x, nudge y, 0]` for a label to
    /// draw, zeros for everything else.
    pub fn states(&self, view_proj: Mat4, viewport: [f32; 2], azimuth: f32) -> LabelStates {
        let mut table = [[0.0; 4]; SLOTS];
        for p in self.place(view_proj, viewport, azimuth) {
            if p.visible {
                table[p.slot] = [1.0, p.push[0], p.push[1], 0.0];
            }
        }
        // A label printed on a box shows if the first box the camera sees through its anchor's
        // pixel is that box: the same ray test as picking. A label hanging from a line's data
        // point shows if no box is in front of the point, and then only if it does not touch
        // one already shown, the points nearest the camera first (points of different series
        // often land on the same pixels, and two numbers on top of each other say nothing).
        let mut points: Vec<(f32, &OnBox, Vec2)> = Vec::new();
        for label in &self.on_boxes {
            let c = view_proj * label.anchor.extend(1.0);
            if c.w <= 0.0 {
                continue;
            }
            let px = (
                (c.x / c.w * 0.5 + 0.5) * viewport[0],
                (0.5 - c.y / c.w * 0.5) * viewport[1],
            );
            let Some(ray) = ray_through_pixel(view_proj, (viewport[0], viewport[1]), px) else {
                continue;
            };
            let first = self
                .boxes
                .iter()
                .enumerate()
                .filter_map(|(i, (min, max))| Some((i, ray_box(&ray, *min, *max)?)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            match label.mark {
                Some(mark) => {
                    if first.is_some_and(|(i, _)| i == mark) {
                        table[label.slot] = [1.0, 0.0, 0.0, 0.0];
                    }
                }
                // A point is seen unless a box is entered before the camera reaches it
                // (a hair of slack, so a point resting on a box's face is not "inside" it).
                None => {
                    let distance = (label.anchor - ray.origin).dot(ray.dir);
                    if first.is_none_or(|(_, t)| t >= distance - POINT_SLACK) {
                        points.push((distance, label, Vec2::new(px.0, px.1).round()));
                    }
                }
            }
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        let margin = self.gap / 2.0;
        let mut kept: Vec<[f32; 4]> = Vec::new();
        for (_, label, anchor_px) in points {
            let rect = inflate(label.entry.rect(anchor_px), margin);
            if !kept.iter().any(|k| rects_overlap(*k, rect)) {
                kept.push(rect);
                table[label.slot] = [1.0, 0.0, 0.0, 0.0];
            }
        }
        table
    }
}

/// Titles and ticks are considered axis by axis in this order: y (the scale readers rely
/// on), then x, then z.
fn group_order(group: u8) -> u8 {
    match group {
        AXIS_Y => 0,
        AXIS_X => 1,
        AXIS_Z => 2,
        _ => 3,
    }
}

/// Where `index` falls in the order "first, last, the middle, the middle of each half, ..." for
/// `n` labels. Keeping labels in this order and dropping any that overlap leaves an evenly
/// spread subset however many fit.
fn bisect_rank(index: usize, n: usize) -> usize {
    bisect_order(n)
        .iter()
        .position(|&i| i == index)
        .unwrap_or(n)
}

fn bisect_order(n: usize) -> Vec<usize> {
    let mut out = Vec::with_capacity(n);
    if n == 0 {
        return out;
    }
    out.push(0);
    if n > 1 {
        out.push(n - 1);
    }
    // Breadth first over the halves: a queue that only ever grows, read from the front.
    let mut halves = vec![(0, n - 1)];
    let mut next = 0;
    while next < halves.len() {
        let (lo, hi) = halves[next];
        next += 1;
        if hi - lo >= 2 {
            let mid = (lo + hi) / 2;
            out.push(mid);
            halves.push((lo, mid));
            halves.push((mid, hi));
        }
    }
    out
}

fn inflate(r: [f32; 4], by: f32) -> [f32; 4] {
    [r[0] - by, r[1] - by, r[2] + by, r[3] + by]
}

fn rects_overlap(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// The six faces of the box `[0, size]` as screen-space quads. A convex solid's silhouette is
/// the union of its faces' outlines, and each (perspective-projected) face is itself convex,
/// which is all the overlap test below needs; no hull has to be computed.
fn box_faces(size: [f32; 3], project: impl Fn(Vec3) -> Vec2) -> [[Vec2; 4]; 6] {
    let corner = |i: usize| {
        project(Vec3::new(
            if i & 1 == 0 { 0.0 } else { size[0] },
            if i & 2 == 0 { 0.0 } else { size[1] },
            if i & 4 == 0 { 0.0 } else { size[2] },
        ))
    };
    // Corner `i` has bit 0 for x, bit 1 for y, bit 2 for z; each face lists its corners in order
    // around it.
    [
        [0, 1, 3, 2], // z = 0
        [4, 5, 7, 6], // z = max
        [0, 1, 5, 4], // y = 0
        [2, 3, 7, 6], // y = max
        [0, 2, 6, 4], // x = 0
        [1, 3, 7, 5], // x = max
    ]
    .map(|face| face.map(corner))
}

fn hits_any(r: [f32; 4], polygons: &[[Vec2; 4]]) -> bool {
    polygons.iter().any(|p| rect_hits_polygon(r, p))
}

/// Whether the rectangle and the convex polygon share any area (separating-axis test: the
/// shapes are apart if some line, among the rectangle's axes and the polygon's edge normals,
/// has them on opposite sides).
fn rect_hits_polygon(r: [f32; 4], poly: &[Vec2]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for p in poly {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    if r[2] < lo.x || r[0] > hi.x || r[3] < lo.y || r[1] > hi.y {
        return false;
    }
    let corners = [
        Vec2::new(r[0], r[1]),
        Vec2::new(r[2], r[1]),
        Vec2::new(r[2], r[3]),
        Vec2::new(r[0], r[3]),
    ];
    for i in 0..poly.len() {
        let edge = poly[(i + 1) % poly.len()] - poly[i];
        let axis = Vec2::new(-edge.y, edge.x);
        let span = |pts: &[Vec2]| {
            pts.iter()
                .map(|p| axis.dot(*p))
                .fold((f32::MAX, f32::MIN), |(lo, hi), d| (lo.min(d), hi.max(d)))
        };
        let (plo, phi) = span(poly);
        let (rlo, rhi) = span(&corners);
        if rhi < plo || rlo > phi {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use facetry_core::{build_scene_3d, parse_spec_json};

    use super::*;
    use crate::camera::Camera;

    fn scene(json: &str) -> Scene3d {
        let spec = parse_spec_json(json).expect("parses");
        build_scene_3d(&spec, (800, 500), 1.0, FontMetrics::inter())
    }

    fn grid() -> Scene3d {
        scene(include_str!("../../../examples/specs/bar3d-grid.json"))
    }

    /// A chart with 12 categories on each floor axis, so thinning has work to do.
    fn busy() -> Scene3d {
        let quoted =
            |items: Vec<String>| items.iter().map(|s| format!("{s:?}")).collect::<Vec<_>>();
        let (mut x, mut z, mut v) = (vec![], vec![], vec![]);
        for xi in 0..12 {
            for zi in 0..12 {
                x.push(format!("Cat {xi}"));
                z.push(format!("Row {zi}"));
                v.push((10 + (xi * 12 + zi) % 7).to_string());
            }
        }
        scene(&format!(
            r#"{{"dimension":"3d","data":{{"t":{{"x":[{}],"z":[{}],"v":[{}]}}}},
            "axes":[{{"id":"x","orient":"bottom","title":"X axis"}},
                    {{"id":"y","orient":"left","title":"Value"}},
                    {{"id":"z","orient":"depth","title":"Z axis"}}],
            "layers":[{{"mark":"bar","data":"t","encoding":
                {{"x":{{"field":"x"}},"y":{{"field":"v"}},"z":{{"field":"z"}}}}}}]}}"#,
            quoted(x).join(","),
            quoted(z).join(","),
            v.join(",")
        ))
    }

    fn camera(azimuth: f32, elevation: f32) -> Camera {
        let mut c = Camera::default();
        c.set(Some(azimuth), Some(elevation), None);
        c
    }

    fn placed(scene: &Scene3d, plan: &LabelPlan, c: &Camera) -> Vec<Placed> {
        let vp = c.plot_view_proj(scene.size, (800.0, 500.0));
        plan.place(vp, [800.0, 500.0], c.azimuth)
    }

    fn faces_of(scene: &Scene3d, c: &Camera) -> [[Vec2; 4]; 6] {
        let vp = c.plot_view_proj(scene.size, (800.0, 500.0));
        box_faces(scene.size, |p| {
            let c = vp * p.extend(1.0);
            Vec2::new(
                (c.x / c.w * 0.5 + 0.5) * 800.0,
                (0.5 - c.y / c.w * 0.5) * 500.0,
            )
        })
    }

    #[test]
    fn bisect_order_visits_the_ends_first_then_halves() {
        assert_eq!(bisect_order(0), Vec::<usize>::new());
        assert_eq!(bisect_order(1), [0]);
        assert_eq!(bisect_order(2), [0, 1]);
        assert_eq!(bisect_order(5), [0, 4, 2, 1, 3]);
        for n in 1..60 {
            let mut o = bisect_order(n);
            o.sort_unstable();
            assert_eq!(o, (0..n).collect::<Vec<_>>(), "n = {n}");
        }
    }

    #[test]
    fn rectangle_and_polygon_overlap_basics() {
        let square = [
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        assert!(rect_hits_polygon([8.0, 8.0, 20.0, 20.0], &square));
        assert!(rect_hits_polygon([2.0, 2.0, 3.0, 3.0], &square)); // fully inside
        assert!(!rect_hits_polygon([11.0, 0.0, 20.0, 10.0], &square));
        // A diamond: its bounding box contains the point, the diamond does not.
        let diamond = [
            Vec2::new(5.0, 0.0),
            Vec2::new(10.0, 5.0),
            Vec2::new(5.0, 10.0),
            Vec2::new(0.0, 5.0),
        ];
        assert!(!rect_hits_polygon([8.5, 0.0, 12.0, 1.5], &diamond));
        assert!(rect_hits_polygon([4.0, 4.0, 6.0, 6.0], &diamond));
    }

    #[test]
    fn slots_are_numbered_per_sector_and_fixed_labels_have_none() {
        let s = grid();
        let (plan, slots) = LabelPlan::new(&s, FontMetrics::inter());
        assert_eq!(slots.len(), s.texts.len());
        assert_eq!(plan.entries.len(), 8 * 15);
        // 4 + 3 + 5 ticks and 3 titles in every sector, numbered 0..15 there.
        for sector in 0..8 {
            let mut seen: Vec<f32> = s
                .texts
                .iter()
                .zip(&slots)
                .filter(|(t, _)| t.azimuths.is_some_and(|r| r.from as usize == sector * 45))
                .map(|(_, &slot)| slot)
                .collect();
            seen.sort_by(f32::total_cmp);
            assert_eq!(seen, (0..15).map(|i| i as f32).collect::<Vec<_>>());
        }
    }

    #[test]
    fn at_the_default_camera_every_label_of_the_example_shows() {
        let s = grid();
        let (plan, _) = LabelPlan::new(&s, FontMetrics::inter());
        let p = placed(&s, &plan, &Camera::default());
        assert_eq!(p.len(), 15);
        assert!(p.iter().all(|p| p.visible), "{p:#?}");
    }

    #[test]
    fn visible_labels_never_overlap_each_other_or_the_box_at_any_view() {
        for s in [grid(), busy()] {
            let (plan, _) = LabelPlan::new(&s, FontMetrics::inter());
            for elevation in [5.0, 10.0, 25.0, 45.0, 70.0, 85.0] {
                for step in 0..96 {
                    let c = camera(step as f32 * 3.75, elevation);
                    let faces = faces_of(&s, &c);
                    let shown: Vec<_> = placed(&s, &plan, &c)
                        .into_iter()
                        .filter(|p| p.visible)
                        .collect();
                    for (i, a) in shown.iter().enumerate() {
                        assert!(
                            !hits_any(a.rect, &faces),
                            "label {a:?} is over the box at {c:?}"
                        );
                        for b in &shown[i + 1..] {
                            assert!(!rects_overlap(a.rect, b.rect), "{a:?} / {b:?} at {c:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn titles_always_show_in_the_example() {
        let s = grid();
        let (plan, _) = LabelPlan::new(&s, FontMetrics::inter());
        for elevation in [10.0, 25.0, 70.0] {
            for step in 0..72 {
                let c = camera(step as f32 * 5.0, elevation);
                let n = placed(&s, &plan, &c).iter().filter(|p| p.visible).count();
                assert!(n >= 3, "only {n} labels at {c:?}");
            }
        }
    }

    #[test]
    fn an_axis_with_too_many_labels_keeps_an_evenly_spread_subset_from_any_side() {
        // 12 labels along each floor axis do not fit along either axis, end-on or broadside:
        // some are dropped, the first and last survive when they are far apart, and what is
        // kept is the same labels' bisection order (ends first), never an arbitrary pick.
        let s = busy();
        let (plan, slots) = LabelPlan::new(&s, FontMetrics::inter());
        for (az, group) in [(0.0, AXIS_X), (0.0, AXIS_Z), (90.0, AXIS_X), (90.0, AXIS_Z)] {
            let c = camera(az, 25.0);
            let visible: Vec<usize> = placed(&s, &plan, &c)
                .iter()
                .filter(|p| p.visible)
                .map(|p| p.slot)
                .collect();
            let mut indices: Vec<u16> = s
                .texts
                .iter()
                .zip(&slots)
                .filter_map(|(t, slot)| match t.role {
                    LabelRole::Tick {
                        group: g, index, ..
                    } if g == group
                        && t.azimuths.is_some_and(|r| r.contains(az))
                        && visible.contains(&(*slot as usize)) =>
                    {
                        Some(index)
                    }
                    _ => None,
                })
                .collect();
            indices.sort_unstable();
            assert!(
                (2..12).contains(&indices.len()),
                "az {az} group {group}: {indices:?}"
            );
            assert_eq!(indices[0], 0, "az {az} group {group}: {indices:?}");
        }
    }

    #[test]
    fn labels_over_the_box_are_pushed_outward_not_hidden() {
        // The grid at azimuth 88: the x axis is almost end-on and its far labels would sit
        // inside the box outline. They are nudged out (to the right), not dropped.
        let s = grid();
        let (plan, _) = LabelPlan::new(&s, FontMetrics::inter());
        let p = placed(&s, &plan, &camera(88.0, 25.0));
        assert!(
            p.iter().any(|p| p.visible && p.push != [0.0, 0.0]),
            "{p:#?}"
        );
        assert!(p.iter().all(|p| p.visible), "{p:#?}");
    }

    #[test]
    fn states_table_marks_exactly_the_visible_slots() {
        let s = grid();
        let (plan, _) = LabelPlan::new(&s, FontMetrics::inter());
        let c = camera(40.0, 25.0);
        let vp = c.plot_view_proj(s.size, (800.0, 500.0));
        let table = plan.states(vp, [800.0, 500.0], c.azimuth);
        let want = placed(&s, &plan, &c);
        for p in &want {
            assert_eq!(table[p.slot][0], if p.visible { 1.0 } else { 0.0 });
            if p.visible {
                assert_eq!([table[p.slot][1], table[p.slot][2]], p.push);
            }
        }
        assert_eq!(table[OVERFLOW_SLOT], [0.0; 4]);
    }

    #[test]
    fn a_sector_with_too_many_labels_hides_the_excess() {
        // 450 categories: build3d caps its candidates, so craft the overflow by hand.
        let mut s = grid();
        let template = s.texts[0].clone();
        s.texts.clear();
        for i in 0..400 {
            let mut t = template.clone();
            t.role = LabelRole::Tick {
                group: AXIS_X,
                index: i,
                count: 400,
            };
            t.azimuths = Some(AzimuthRange::new(0.0, 45.0));
            s.texts.push(t);
        }
        let (plan, slots) = LabelPlan::new(&s, FontMetrics::inter());
        assert_eq!(plan.entries.len(), OVERFLOW_SLOT);
        assert_eq!(slots[OVERFLOW_SLOT + 10], OVERFLOW_SLOT as f32);
    }

    /// Which of a stacked chart's value labels are shown from `camera`: the keys of their boxes.
    fn shown_value_labels(scene: &Scene3d, camera: Camera) -> Vec<String> {
        let (plan, _) = LabelPlan::new(scene, FontMetrics::inter());
        let viewport = (800.0, 500.0);
        let table = plan.states(
            camera.plot_view_proj(scene.size, viewport),
            [viewport.0, viewport.1],
            camera.azimuth,
        );
        plan.label_slots()
            .iter()
            .zip(&scene.labels)
            .filter(|(slot, _)| table[**slot as usize][0] == 1.0)
            .map(|(_, l)| {
                scene.boxes[match l.text.role {
                    LabelRole::OnMark(m) => m as usize,
                    other => panic!("{other:?}"),
                }]
                .key
                .clone()
            })
            .collect()
    }

    fn stacked_with_labels() -> Scene3d {
        let json = include_str!("../../../examples/specs/bar3d-stacked.json").replace(
            r#""encoding": {"#,
            r#""label": { "show": "always" }, "encoding": {"#,
        );
        let s = scene(&json);
        assert!(
            s.labels
                .iter()
                .all(|l| matches!(l.text.role, LabelRole::OnMark(_)))
        );
        s
    }

    #[test]
    fn labels_on_boxes_hide_behind_other_boxes() {
        let s = stacked_with_labels();
        // 36 segments; the thinnest ones (Q4 Services) have no room for a label at all.
        assert!(s.labels.len() >= 30, "{}", s.labels.len());
        // From almost straight above only the top segment of each stack is in view: the
        // segments below are hidden under it.
        let mut top = Camera::default();
        top.set(Some(0.0), Some(85.0), None);
        let shown = shown_value_labels(&s, top);
        assert!(!shown.is_empty());
        assert!(
            shown.iter().all(|k| k.contains("/Services/")),
            "only the topmost segments: {shown:?}"
        );
        // From the front at a low elevation the nearest row of every stack shows all three
        // segments, and the rows behind them are hidden.
        let mut front = Camera::default();
        front.set(Some(0.0), Some(8.0), None);
        let shown = shown_value_labels(&s, front);
        assert!(
            shown.contains(&"layer0/Q1/Hardware/North".to_owned()),
            "{shown:?}"
        );
        assert!(shown.contains(&"layer0/Q1/Software/North".to_owned()));
        assert!(!shown.iter().any(|k| k.ends_with("/West")), "{shown:?}");
        // Whatever the view, something is shown and nothing is shown twice.
        for az in (0..360).step_by(20) {
            for el in [10.0, 30.0, 60.0] {
                let mut c = Camera::default();
                c.set(Some(az as f32), Some(el), None);
                let shown = shown_value_labels(&s, c);
                assert!(!shown.is_empty(), "{az}/{el}");
                let mut unique = shown.clone();
                unique.sort();
                unique.dedup();
                assert_eq!(unique.len(), shown.len());
            }
        }
    }

    /// `line3d` with value labels on, optionally with the line values replaced.
    fn labelled_lines(values: Option<&str>) -> Scene3d {
        let json = include_str!("../../../examples/specs/line3d.json").replace(
            r#""point": true,"#,
            r#""point": true, "label": { "show": "always" },"#,
        );
        let json = match values {
            Some(v) => {
                let open = json.find(r#""visits": ["#).unwrap() + r#""visits": ["#.len();
                let close = open + json[open..].find(']').unwrap();
                format!("{}{v}{}", &json[..open], &json[close..])
            }
            None => json,
        };
        scene(&json)
    }

    /// The screen rectangles of the point labels shown from `camera`.
    fn shown_point_rects(scene: &Scene3d, camera: Camera) -> Vec<[f32; 4]> {
        let (plan, _) = LabelPlan::new(scene, FontMetrics::inter());
        let viewport = (800.0, 500.0);
        let vp = camera.plot_view_proj(scene.size, viewport);
        let table = plan.states(vp, [viewport.0, viewport.1], camera.azimuth);
        plan.on_boxes
            .iter()
            .filter(|l| table[l.slot][0] == 1.0)
            .map(|l| {
                let c = vp * l.anchor.extend(1.0);
                let px = Vec2::new(
                    (c.x / c.w * 0.5 + 0.5) * viewport.0,
                    (0.5 - c.y / c.w * 0.5) * viewport.1,
                );
                l.entry.rect(px.round())
            })
            .collect()
    }

    #[test]
    fn point_labels_never_overlap_each_other_from_any_side() {
        let s = labelled_lines(None);
        assert_eq!(s.labels.len(), 18);
        let mut most = 0;
        for az in (0..360).step_by(15) {
            for el in [10.0, 30.0, 60.0, 85.0] {
                let rects = shown_point_rects(&s, camera(az as f32, el));
                most = most.max(rects.len());
                for (i, a) in rects.iter().enumerate() {
                    for b in &rects[i + 1..] {
                        assert!(!rects_overlap(*a, *b), "{az}/{el}: {a:?} {b:?}");
                    }
                }
            }
        }
        // With room to spare, most of them show.
        assert!(most >= 14, "{most}");
    }

    #[test]
    fn point_labels_hide_while_a_bar_is_in_front_of_the_point() {
        // The line's points are above every bar: all labels can show (minus overlaps).
        let json = include_str!("../../../examples/specs/combo3d-bar-line.json").replace(
            r#""point": true,"#,
            r#""point": true, "label": { "show": "always" },"#,
        );
        let above = scene(&json);
        assert!(
            above
                .labels
                .iter()
                .all(|l| l.text.role == LabelRole::OnPoint)
        );
        assert!(!shown_point_rects(&above, camera(30.0, 30.0)).is_empty());
        // Points inside the bars (values far below them): from the front at a low elevation
        // every point is in the bar of its own cell or behind another one, so none shows.
        let open = json.find("\"target\": [").unwrap() + "\"target\": [".len();
        let close = open + json[open..].find(']').unwrap();
        let low = format!(
            "{}{}{}",
            &json[..open],
            ["5"; 12].join(", "),
            &json[close..]
        );
        let inside = scene(&low);
        for (az, el) in [(0.0, 10.0), (30.0, 30.0), (200.0, 20.0), (90.0, 45.0)] {
            assert!(
                shown_point_rects(&inside, camera(az, el)).is_empty(),
                "{az}/{el}"
            );
        }
    }

    #[test]
    fn point_slots_stay_clear_of_the_axis_label_slots() {
        let s = labelled_lines(None);
        let (plan, text_slots) = LabelPlan::new(&s, FontMetrics::inter());
        let axis_max = text_slots
            .iter()
            .filter(|s| **s >= 0.0)
            .fold(0.0_f32, |a, b| a.max(*b));
        let point_min = plan.label_slots().iter().fold(f32::MAX, |a, b| a.min(*b));
        assert!((axis_max as usize) < SLOTS / 2 && (point_min as usize) >= SLOTS / 2);
    }

    #[test]
    fn value_slots_do_not_collide_with_axis_label_slots() {
        let s = stacked_with_labels();
        let (plan, text_slots) = LabelPlan::new(&s, FontMetrics::inter());
        let axis_max = text_slots
            .iter()
            .filter(|s| **s >= 0.0)
            .fold(0.0_f32, |a, b| a.max(*b));
        let value_min = plan.label_slots().iter().fold(f32::MAX, |a, b| a.min(*b));
        assert!((axis_max as usize) < SLOTS / 2 && (value_min as usize) >= SLOTS / 2);
    }
}
