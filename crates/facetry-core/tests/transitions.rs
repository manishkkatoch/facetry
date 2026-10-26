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
