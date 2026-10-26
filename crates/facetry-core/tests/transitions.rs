//! Transitions between real scenes: what the keys of the example specs say enters, moves and
//! leaves, and that every pair of examples can be animated without a jump at either end.
#![cfg(not(target_arch = "wasm32"))]

use facetry_core::spec::{Easing, Timing};
use facetry_core::transition::{Phase, Plan};
use facetry_core::{ChartSpec, Scene, build_scene, parse_spec_json, set_series_visible, validate};
use facetry_font::FontMetrics;

fn example(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../../examples/specs/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn spec_from(json: &str) -> ChartSpec {
    let spec = parse_spec_json(json).unwrap();
    assert_eq!(validate(&spec), vec![]);
    spec
}

fn scene(spec: &ChartSpec) -> Scene {
    build_scene(spec, (800, 500), 1.0, FontMetrics::inter())
}

fn timing() -> Timing {
    Timing {
        duration_ms: 400.0,
        easing: Easing::Linear,
        stagger_ms: 0.0,
    }
}

/// `bar-stacked` with a fifth quarter.
fn stacked_with_q5() -> ChartSpec {
    let mut v: serde_json::Value = serde_json::from_str(&example("bar-stacked")).unwrap();
    let t = &mut v["data"]["sales"];
    for (product, revenue) in [("Hardware", 150), ("Software", 120), ("Services", 60)] {
        t["quarter"].as_array_mut().unwrap().push("Q5".into());
        t["product"].as_array_mut().unwrap().push(product.into());
        t["revenue"].as_array_mut().unwrap().push(revenue.into());
    }
    spec_from(&v.to_string())
}

fn keys<T>(
    pairs: &[facetry_core::transition::Pair<T>],
    phase: Phase,
    key: fn(&T) -> &str,
) -> Vec<String> {
    pairs
        .iter()
        .filter(|p| p.phase == phase)
        .map(|p| key(if phase == Phase::Exit { &p.from } else { &p.to }).to_owned())
        .collect()
}

#[test]
fn a_new_category_enters_its_bars_labels_and_ticks_and_the_rest_moves() {
    let old = scene(&spec_from(&example("bar-stacked")));
    let new = scene(&stacked_with_q5());
    let plan = Plan::diff(&old, &new, timing());
    assert_eq!(
        keys(&plan.rects, Phase::Enter, |r| &r.key),
        [
            "layer0/Q5/Hardware",
            "layer0/Q5/Software",
            "layer0/Q5/Services"
        ]
    );
    assert_eq!(keys(&plan.rects, Phase::Update, |r| &r.key).len(), 12);
    assert!(keys(&plan.rects, Phase::Exit, |r| &r.key).is_empty());
    // The new category's tick and label come in; the old ticks stay.
    assert_eq!(
        keys(&plan.rules, Phase::Enter, |r| &r.key)
            .iter()
            .filter(|k| k.starts_with("axis:x/tick:"))
            .collect::<Vec<_>>(),
        ["axis:x/tick:Q5"]
    );
    assert!(keys(&plan.texts, Phase::Enter, |t| &t.key).contains(&"axis:x/label:Q5".to_owned()));
    // Its three value labels fade in, and the three bars' labels of an existing category
    // keep their text, so they move rather than cross-fade.
    let entering_labels = keys(&plan.labels, Phase::Enter, |l| &l.text.key);
    assert!(entering_labels.contains(&"layer0/Q5/Hardware/label".to_owned()));
    assert!(
        plan.labels
            .iter()
            .filter(|p| p.phase == Phase::Update)
            .count()
            >= 11
    );
    // The axis got taller: the old 300 is gone or moved, new ticks came in above the old range.
    assert!(!plan.is_static());
}

#[test]
fn a_removed_category_exits_the_same_things() {
    let old = scene(&stacked_with_q5());
    let new = scene(&spec_from(&example("bar-stacked")));
    let plan = Plan::diff(&old, &new, timing());
    assert_eq!(
        keys(&plan.rects, Phase::Exit, |r| &r.key),
        [
            "layer0/Q5/Hardware",
            "layer0/Q5/Software",
            "layer0/Q5/Services"
        ]
    );
    assert!(keys(&plan.rects, Phase::Enter, |r| &r.key).is_empty());
    assert!(keys(&plan.rules, Phase::Exit, |r| &r.key).contains(&"axis:x/tick:Q5".to_owned()));
    // The exiting bars end flat on the baseline (the zero of the new scale).
    let zero = new.axes.iter().find(|a| a.id == "y").unwrap().px(0.0) as f32;
    for p in plan.rects.iter().filter(|p| p.phase == Phase::Exit) {
        assert_eq!(p.to.bounds.h, 0.0);
        assert!(
            (p.to.bounds.y - zero).abs() < 1.0,
            "{} vs {zero}",
            p.to.bounds.y
        );
    }
}

#[test]
fn new_y_ticks_start_where_the_old_scale_would_have_put_them() {
    let old = scene(&spec_from(&example("bar-stacked")));
    let new = scene(&stacked_with_q5());
    let old_y = old.axes.iter().find(|a| a.id == "y").unwrap();
    let new_y = new.axes.iter().find(|a| a.id == "y").unwrap();
    assert_ne!(
        old_y.domain, new_y.domain,
        "the example should refit the axis"
    );
    let plan = Plan::diff(&old, &new, timing());
    let entering: Vec<_> = plan
        .rules
        .iter()
        .filter(|p| p.phase == Phase::Enter && p.to.key.starts_with("axis:y/tick:"))
        .collect();
    assert!(!entering.is_empty());
    for p in entering {
        let v = p.to.tick.unwrap().value;
        let moved = (old_y.px_unclamped(v) - new_y.px_unclamped(v)) as f32;
        assert!(
            (p.from.bounds.y - (p.to.bounds.y + moved)).abs() < 1e-3,
            "{}",
            p.to.key
        );
        assert_eq!(p.from.color.a, 0.0);
    }
    // And the old ticks that stay slide along with their scale.
    for p in plan
        .rules
        .iter()
        .filter(|p| p.phase == Phase::Update && p.to.key.starts_with("axis:y/tick:"))
    {
        let v = p.to.tick.unwrap().value;
        assert!(
            (p.from.bounds.y - (old_y.px_unclamped(v) as f32 - p.to.bounds.h / 2.0).round()).abs()
                < 2.0,
            "{v}"
        );
    }
}

#[test]
fn hiding_a_series_exits_its_segments_and_dims_its_legend_entry() {
    let shown = spec_from(&example("bar-stacked"));
    let mut hidden = shown.clone();
    assert!(set_series_visible(&mut hidden, "Services", false));
    let (old, new) = (scene(&shown), scene(&hidden));
    let plan = Plan::diff(&old, &new, timing());
    assert_eq!(
        keys(&plan.rects, Phase::Exit, |r| &r.key),
        [
            "layer0/Q1/Services",
            "layer0/Q2/Services",
            "layer0/Q3/Services",
            "layer0/Q4/Services"
        ]
    );
    // The legend entry stays and fades to its dimmed colour; it does not leave.
    let swatch = plan
        .rules
        .iter()
        .find(|p| p.to.key == "legend/swatch:Services")
        .unwrap();
    assert_eq!(swatch.phase, Phase::Update);
    assert!(swatch.to.color.a < swatch.from.color.a);
    // Showing it again is the reverse: the segments enter.
    let back = Plan::diff(&new, &old, timing());
    assert_eq!(keys(&back.rects, Phase::Enter, |r| &r.key).len(), 4);
}

/// Every pair of 2D examples: the first frame has everything of the old scene exactly where it
/// was, the last is the new scene exactly, and nothing in between is inverted or off the canvas
/// by much.
#[test]
fn every_pair_of_examples_animates_without_a_jump_at_either_end() {
    let names = [
        "bar-basic",
        "bar-colored",
        "bar-labels",
        "bar-negative",
        "bar-stacked",
        "bar-grouped",
        "bar-normalized",
        "bar-right-axis",
        "bar-empty",
        "line-basic",
        "combo-bar-line",
        "pie",
    ];
    let scenes: Vec<(String, Scene)> = names
        .iter()
        .map(|n| (n.to_string(), scene(&spec_from(&example(n)))))
        .collect();
    for (an, a) in &scenes {
        for (bn, b) in &scenes {
            let plan = Plan::diff(a, b, timing());
            let first = plan.sample(0.0);
            for r in &a.rects {
                let shown = first
                    .rects
                    .iter()
                    .find(|x| x.key == r.key)
                    .unwrap_or_else(|| panic!("{an}->{bn}: {} missing at 0", r.key));
                assert_eq!(shown.bounds, r.bounds, "{an}->{bn}: {}", r.key);
            }
            // Every old text is on screen as it was, under its own key or (when the string
            // changed under the same key and it is cross-fading out) a marked copy of it.
            for t in &a.texts {
                assert!(
                    first
                        .texts
                        .iter()
                        .any(|x| x.text == t.text && (x.x, x.y, x.color) == (t.x, t.y, t.color)),
                    "{an}->{bn}: {}",
                    t.key
                );
            }
            let last = plan.sample(plan.total_ms());
            assert_eq!(last.rects, b.rects, "{an}->{bn}");
            assert_eq!(last.texts, b.texts, "{an}->{bn}");
            assert_eq!(last.rules, b.rules, "{an}->{bn}");
            for step in 0..=8 {
                let mid = plan.sample(plan.total_ms() * step as f32 / 8.0);
                for r in &mid.rects {
                    assert!(
                        r.bounds.w >= 0.0 && r.bounds.h >= 0.0,
                        "{an}->{bn} {}",
                        r.key
                    );
                    assert!(
                        r.bounds.y >= -1.0 && r.bounds.y + r.bounds.h <= 501.0,
                        "{an}->{bn} {}: {:?}",
                        r.key,
                        r.bounds
                    );
                }
            }
        }
    }
}

#[test]
fn a_scene_against_itself_has_nothing_to_animate() {
    for name in [
        "bar-basic",
        "bar-stacked",
        "bar-grouped",
        "line-basic",
        "pie",
        "donut",
        "combo-bar-line",
    ] {
        let s = scene(&spec_from(&example(name)));
        assert!(Plan::diff(&s, &s, timing()).is_static(), "{name}");
    }
}

// --- 3D ----------------------------------------------------------------------------------------

mod three_d {
    use super::*;
    use facetry_core::scene3d::Scene3d;
    use facetry_core::transition3d::Plan3;
    use facetry_core::{ChartSpec, build_scene_3d};

    fn scene3(spec: &ChartSpec) -> Scene3d {
        build_scene_3d(spec, (800, 500), 1.0, FontMetrics::inter())
    }

    fn named(name: &str) -> Scene3d {
        scene3(&spec_from(&example(name)))
    }

    /// An example spec with `edit` applied to its JSON.
    fn edited(name: &str, edit: impl Fn(&mut serde_json::Value)) -> Scene3d {
        let mut v: serde_json::Value = serde_json::from_str(&example(name)).unwrap();
        edit(&mut v);
        scene3(&spec_from(&v.to_string()))
    }

    fn push_row(table: &mut serde_json::Value, row: &[(&str, serde_json::Value)]) {
        for (k, v) in row {
            table[*k].as_array_mut().unwrap().push(v.clone());
        }
    }

    #[test]
    fn a_new_column_of_boxes_grows_from_the_floor_and_the_box_resizes_smoothly() {
        let old = named("bar3d-stacked");
        let new = edited("bar3d-stacked", |v| {
            let t = &mut v["data"]["sales"];
            // A fifth quarter in every region and product.
            let rows: Vec<(String, String, String)> = {
                let (q, r, p) = (
                    t["quarter"].as_array().unwrap().clone(),
                    t["region"].as_array().unwrap().clone(),
                    t["product"].as_array().unwrap().clone(),
                );
                let mut seen = Vec::new();
                for i in 0..q.len() {
                    let key = (
                        r[i].as_str().unwrap().to_owned(),
                        p[i].as_str().unwrap().to_owned(),
                    );
                    if !seen.contains(&key) {
                        seen.push(key);
                    }
                }
                seen.into_iter()
                    .map(|(r, p)| ("Q5".to_owned(), r, p))
                    .collect()
            };
            for (q, r, p) in rows {
                push_row(
                    t,
                    &[
                        ("quarter", q.into()),
                        ("region", r.into()),
                        ("product", p.into()),
                        ("revenue", 77.into()),
                    ],
                );
            }
        });
        let plan = Plan3::diff(&old, &new, timing());
        let entering: Vec<_> = plan
            .boxes
            .iter()
            .filter(|p| p.phase == Phase::Enter)
            .collect();
        assert!(!entering.is_empty());
        assert!(entering.iter().all(|p| p.from.min[1] == p.from.max[1]));
        assert!(plan.boxes.iter().filter(|p| p.phase == Phase::Exit).count() == 0);
        // The box is wider with a fifth column: its size, and so the camera's framing, moves.
        assert!(new.size[0] > old.size[0]);
        assert_eq!(plan.size_at(0.0), old.size);
        assert_eq!(plan.size_at(400.0), new.size);
        let mid = plan.size_at(200.0);
        assert!(mid[0] > old.size[0] && mid[0] < new.size[0]);
        // The first frame is the old picture exactly.
        let first = plan.sample(0.0);
        for b in &old.boxes {
            let shown = first.boxes.iter().find(|x| x.key == b.key).unwrap();
            assert_eq!((shown.min, shown.max), (b.min, b.max), "{}", b.key);
        }
        assert_eq!(plan.sample(plan.total_ms()).boxes, new.boxes);
    }

    #[test]
    fn a_segment_added_to_a_3d_stack_opens_between_its_neighbours() {
        let shown = spec_from(&example("bar3d-stacked"));
        let mut hidden = shown.clone();
        assert!(set_series_visible(&mut hidden, "Services", false));
        let (all, without) = (scene3(&shown), scene3(&hidden));
        let hide = Plan3::diff(&all, &without, timing());
        let exits: Vec<_> = hide
            .boxes
            .iter()
            .filter(|p| p.phase == Phase::Exit)
            .collect();
        assert!(!exits.is_empty());
        assert!(exits.iter().all(|p| p.to.min[1] == p.to.max[1]));
        // The restacked boxes never overlap or leave a gap inside a stack mid-way: every
        // exiting segment is glued to the top of the segment that was under it.
        for step in 0..=8 {
            let s = hide.sample(step as f32 * 50.0);
            for b in &s.boxes {
                assert!(b.max[1] >= b.min[1], "{} inverted at step {step}", b.key);
            }
        }
        let show = Plan3::diff(&without, &all, timing());
        assert_eq!(
            show.boxes
                .iter()
                .filter(|p| p.phase == Phase::Enter)
                .count(),
            exits.len()
        );
    }

    #[test]
    fn extruded_pie_slices_tile_while_they_open_and_close() {
        let old = named("pie3d");
        let mut spec = spec_from(&example("pie3d"));
        let second = facetry_core::series_names(&spec)[1].clone();
        assert!(set_series_visible(&mut spec, &second, false));
        let new = scene3(&spec);
        for (a, b) in [(&old, &new), (&new, &old)] {
            let plan = Plan3::diff(a, b, timing());
            for step in 0..=10 {
                let s = plan.sample(step as f32 * 40.0);
                let mut slices: Vec<_> = s.slices.iter().collect();
                slices.sort_by(|x, y| x.start.total_cmp(&y.start).then(x.end.total_cmp(&y.end)));
                for pair in slices.windows(2) {
                    assert!(
                        pair[0].end <= pair[1].start + 1e-3,
                        "overlap at step {step}"
                    );
                }
            }
            assert_eq!(plan.sample(plan.total_ms()).slices, b.slices);
        }
    }

    #[test]
    fn markers_stay_on_a_3d_line_as_points_come_and_go() {
        let old = named("line3d");
        let new = edited("line3d", |v| {
            let t = v["data"]
                .as_object_mut()
                .unwrap()
                .values_mut()
                .next()
                .unwrap();
            let keys: Vec<String> = t.as_object().unwrap().keys().cloned().collect();
            // Drop the first row of every column.
            for k in keys {
                t[&k].as_array_mut().unwrap().remove(0);
            }
        });
        let plan = Plan3::diff(&old, &new, timing());
        assert!(plan.points.iter().any(|p| p.phase == Phase::Exit));
        for step in 0..=8 {
            let s = plan.sample(step as f32 * 50.0);
            for m in &s.points {
                let on_line = s.polylines.iter().any(|l| {
                    l.runs
                        .iter()
                        .flatten()
                        .any(|v| (0..3).all(|i| (v[i] - m.pos[i]).abs() < 1e-3))
                });
                // A marker that is drawn in full is on a line. (Those whose line could not be
                // matched by point cross-fade in place instead.)
                let full = m.size_px >= 1.0 && m.color.a >= 0.999;
                assert!(
                    on_line || !full,
                    "marker {} is solid and off its line at step {step}",
                    m.key
                );
            }
        }
    }

    #[test]
    fn every_pair_of_3d_examples_animates_without_a_jump_at_either_end() {
        let names = [
            "bar3d-grid",
            "bar3d-row",
            "bar3d-stacked",
            "bar3d-negative",
            "bar3d-labels",
            "pie3d",
            "line3d",
            "combo3d-bar-line",
        ];
        let scenes: Vec<(&str, Scene3d)> = names.iter().map(|n| (*n, named(n))).collect();
        for (an, a) in &scenes {
            for (bn, b) in &scenes {
                let plan = Plan3::diff(a, b, timing());
                let first = plan.sample(0.0);
                for old in &a.boxes {
                    let shown = first.boxes.iter().find(|x| x.key == old.key).unwrap();
                    assert_eq!(
                        (shown.min, shown.max),
                        (old.min, old.max),
                        "{an}->{bn} {}",
                        old.key
                    );
                }
                // (A few lines share a key: look for the same line under any of them.)
                for old in &a.lines {
                    assert!(
                        first
                            .lines
                            .iter()
                            .any(|x| x.key == old.key && (x.a, x.b) == (old.a, old.b)),
                        "{an}->{bn} {}",
                        old.key
                    );
                }
                for old in &a.texts {
                    // Under its own key or, if the string changed, a marked copy.
                    assert!(
                        first.texts.iter().any(|x| x.text == old.text
                            && x.anchor == old.anchor
                            && x.color == old.color),
                        "{an}->{bn} {}",
                        old.key
                    );
                }
                assert_eq!(plan.size_at(0.0), a.size, "{an}->{bn}");
                let last = plan.sample(plan.total_ms());
                assert_eq!(last.boxes, b.boxes, "{an}->{bn}");
                assert_eq!(last.lines, b.lines, "{an}->{bn}");
                assert_eq!(last.texts, b.texts, "{an}->{bn}");
                assert_eq!(last.slices, b.slices, "{an}->{bn}");
                assert_eq!(last.polylines, b.polylines, "{an}->{bn}");
                assert_eq!(last.points, b.points, "{an}->{bn}");
                for step in 0..=8 {
                    let mid = plan.sample(plan.total_ms() * step as f32 / 8.0);
                    for bx in &mid.boxes {
                        assert!(
                            (0..3).all(|i| bx.max[i] >= bx.min[i]),
                            "{an}->{bn} {}",
                            bx.key
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_scene_against_itself_has_nothing_to_animate_in_3d() {
        for name in [
            "bar3d-grid",
            "bar3d-stacked",
            "pie3d",
            "line3d",
            "combo3d-bar-line",
        ] {
            let s = named(name);
            assert!(Plan3::diff(&s, &s, timing()).is_static(), "{name}");
        }
    }

    #[test]
    fn y_ticks_of_a_3d_chart_slide_along_the_old_and_the_new_scale() {
        let old = named("bar3d-grid");
        let new = edited("bar3d-grid", |v| {
            let t = &mut v["data"]["sales"];
            let revenue: Vec<serde_json::Value> = t["revenue"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| (x.as_f64().unwrap() * 3.0).into())
                .collect();
            t["revenue"] = revenue.into();
        });
        let plan = Plan3::diff(&old, &new, timing());
        let (old_y, new_y) = (&old.axes[0], &new.axes[0]);
        assert_ne!(old_y.domain, new_y.domain);
        let moving: Vec<_> = plan
            .lines
            .iter()
            .filter(|p| p.phase == Phase::Update && p.to.tick.is_some())
            .collect();
        assert!(!moving.is_empty());
        let entering: Vec<_> = plan
            .lines
            .iter()
            .filter(|p| p.phase == Phase::Enter && p.to.tick.is_some())
            .collect();
        for p in entering {
            let v = p.to.tick.unwrap().value;
            let d = (old_y.px_unclamped(v) - new_y.px_unclamped(v)) as f32;
            assert!((p.from.a[1] - (p.to.a[1] + d)).abs() < 1e-4, "{}", p.to.key);
            assert_eq!(p.from.color.a, 0.0);
        }
    }
}
