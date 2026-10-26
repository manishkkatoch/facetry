//! Hiding series from the legend: the data transforms see only what is shown (stacks restack,
//! shares renormalise, pies re-divide, axes refit), colours never move, and the legend keeps
//! every entry, dimmed, with a rectangle to click.
#![cfg(not(target_arch = "wasm32"))]

use facetry_core::scene::{RuleKind, Scene};
use facetry_core::{
    ChartSpec, build_scene, build_scene_3d, parse_spec_json, series_names, set_series_visible,
    settle_hidden, validate,
};
use facetry_font::FontMetrics;

fn example(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../../examples/specs/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// An example spec with `legend` set to the given JSON.
fn with_legend(name: &str, legend: &str) -> ChartSpec {
    let mut value: serde_json::Value = serde_json::from_str(&example(name)).unwrap();
    value["legend"] = serde_json::from_str(legend).unwrap();
    let spec = parse_spec_json(&value.to_string()).unwrap();
    assert_eq!(validate(&spec), vec![], "{name} {legend}");
    spec
}

fn scene(spec: &ChartSpec) -> Scene {
    build_scene(spec, (800, 500), 1.0, FontMetrics::inter())
}

fn hidden_scene(name: &str, hidden: &[&str]) -> Scene {
    let list: Vec<String> = hidden.iter().map(|h| format!("\"{h}\"")).collect();
    scene(&with_legend(
        name,
        &format!(r#"{{ "hidden": [{}] }}"#, list.join(",")),
    ))
}

fn rect<'a>(sc: &'a Scene, key: &str) -> &'a facetry_core::scene::Rect {
    sc.rects
        .iter()
        .find(|r| r.key == key)
        .unwrap_or_else(|| panic!("no bar {key}"))
}

#[test]
fn a_hidden_series_has_no_marks_and_the_stack_closes_the_gap() {
    let all = hidden_scene("bar-stacked", &[]);
    assert_eq!(all.rects.len(), 12);
    let sc = hidden_scene("bar-stacked", &["Software"]);
    assert_eq!(sc.rects.len(), 8);
    assert!(sc.rects.iter().all(|r| !r.key.contains("Software")));
    // Services used to sit on top of Software; now it sits directly on Hardware.
    let (hw, sv) = (
        rect(&sc, "layer0/Q1/Hardware"),
        rect(&sc, "layer0/Q1/Services"),
    );
    assert_eq!(
        sv.bounds.y + sv.bounds.h,
        hw.bounds.y,
        "Services starts where Hardware ends"
    );
    assert_ne!(
        sv.bounds,
        rect(&all, "layer0/Q1/Services").bounds,
        "it moved"
    );
}

#[test]
fn the_axis_refits_to_the_shown_data() {
    // The tallest stack was 280 (Q3: 140 + 110 + 30), so the axis reached 300; without
    // Software the tallest is 170 (Q3: 140 + 30), which needs only 200.
    let top_label = |sc: &Scene| {
        sc.texts
            .iter()
            .filter(|t| t.key.starts_with("axis:y/label:"))
            .map(|t| t.text.replace(',', "").parse::<f64>().unwrap())
            .fold(0.0, f64::max)
    };
    let all = top_label(&hidden_scene("bar-stacked", &[]));
    let some = top_label(&hidden_scene("bar-stacked", &["Software"]));
    assert!(
        all >= 280.0 && some < all && some >= 170.0,
        "{all} -> {some}"
    );
}

#[test]
fn colours_do_not_move_when_a_series_is_hidden() {
    let all = hidden_scene("bar-stacked", &[]);
    let sc = hidden_scene("bar-stacked", &["Hardware"]);
    for key in ["layer0/Q2/Software", "layer0/Q2/Services"] {
        assert_eq!(rect(&sc, key).color, rect(&all, key).color, "{key}");
    }
}

#[test]
fn normalised_columns_still_fill_the_axis() {
    let sc = hidden_scene("bar-normalized", &["Software"]);
    let plot_top = sc.rects.iter().map(|r| r.bounds.y).fold(f32::MAX, f32::min);
    for q in ["Q1", "Q2", "Q3", "Q4"] {
        let top = sc
            .rects
            .iter()
            .filter(|r| r.key.starts_with(&format!("layer0/{q}/")))
            .map(|r| r.bounds.y)
            .fold(f32::MAX, f32::min);
        assert_eq!(top, plot_top, "{q} reaches 100%");
    }
}

#[test]
fn grouped_bars_give_a_hidden_series_its_slot_back() {
    let all = hidden_scene("bar-grouped", &[]);
    let sc = hidden_scene("bar-grouped", &["Software"]);
    let width = |sc: &Scene, key: &str| rect(sc, key).bounds.w;
    // Two series share a band that three used to share: the bars are wider.
    assert!(width(&sc, "layer0/Q1/Hardware") > width(&all, "layer0/Q1/Hardware") * 1.3);
    // And they sit side by side without a hole where Software was.
    let (a, b) = (
        rect(&sc, "layer0/Q1/Hardware"),
        rect(&sc, "layer0/Q1/Services"),
    );
    assert!(b.bounds.x - (a.bounds.x + a.bounds.w) < a.bounds.w * 0.3);
}

#[test]
fn a_pie_divides_the_circle_among_the_slices_that_are_left() {
    let all = hidden_scene("pie", &[]);
    let sc = hidden_scene("pie", &["Social", "Email"]);
    assert_eq!((all.arcs.len(), sc.arcs.len()), (6, 4));
    let sweep: f32 = sc.arcs.iter().map(|a| a.end - a.start).sum();
    assert!((sweep - std::f32::consts::TAU).abs() < 1e-3, "{sweep}");
    // Search is 420 of the remaining 420 + 190 + 20 + 12 = 642.
    let search = &sc.arcs[0];
    let share = (search.end - search.start) / std::f32::consts::TAU;
    assert!((share - 420.0 / 642.0).abs() < 1e-3, "{share}");
    // Percent labels follow.
    let labels: Vec<&str> = sc.labels.iter().map(|l| l.text.text.as_str()).collect();
    assert!(labels.contains(&"65.4%"), "{labels:?}");
}

#[test]
fn lines_of_hidden_series_vanish_and_the_axes_refit() {
    let all = hidden_scene("line-basic", &[]);
    assert_eq!(all.polylines.len(), 2);
    let first = series_names(&with_legend("line-basic", "true"))[0].clone();
    let sc = hidden_scene("line-basic", &[first.as_str()]);
    assert_eq!(sc.polylines.len(), 1);
    assert!(sc.points.iter().all(|p| !p.key.contains(&first)));
}

#[test]
fn the_legend_keeps_hidden_entries_dimmed() {
    let all = hidden_scene("bar-stacked", &[]);
    let sc = hidden_scene("bar-stacked", &["Software"]);
    let swatch = |sc: &Scene, name: &str| {
        sc.rules
            .iter()
            .find(|r| r.kind == RuleKind::Swatch && r.key.ends_with(name))
            .unwrap()
            .clone()
    };
    assert_eq!(
        swatch(&sc, "Software").bounds,
        swatch(&all, "Software").bounds
    );
    assert!(swatch(&sc, "Software").color.a < 0.5);
    assert_eq!(
        swatch(&sc, "Hardware").color,
        swatch(&all, "Hardware").color
    );
    let text = |sc: &Scene, name: &str| {
        sc.texts
            .iter()
            .find(|t| t.key == format!("legend/label:{name}"))
            .unwrap()
            .color
            .a
    };
    assert!(text(&sc, "Software") < 0.5 && text(&sc, "Hardware") == 1.0);
}

#[test]
fn legend_entries_can_be_hit_where_they_are_drawn() {
    let sc = hidden_scene("bar-stacked", &[]);
    assert_eq!(sc.legend_hits.len(), 3);
    for name in ["Hardware", "Software", "Services"] {
        let swatch = sc
            .rules
            .iter()
            .find(|r| r.key == format!("legend/swatch:{name}"))
            .unwrap();
        let b = swatch.bounds;
        // The swatch, and the label beside it, are both the entry.
        assert_eq!(sc.legend_pick(b.x + b.w / 2.0, b.y + b.h / 2.0), Some(name));
        let label = sc
            .texts
            .iter()
            .find(|t| t.key == format!("legend/label:{name}"))
            .unwrap();
        assert_eq!(sc.legend_pick(label.x + 3.0, label.y - 3.0), Some(name));
        let [cx, cy] = sc.legend_center(name).unwrap();
        assert_eq!(sc.legend_pick(cx, cy), Some(name));
    }
    // Entries do not overlap, and the plot is not an entry.
    for (i, a) in sc.legend_hits.iter().enumerate() {
        for b in &sc.legend_hits[i + 1..] {
            let apart =
                a.bounds.y + a.bounds.h <= b.bounds.y || b.bounds.y + b.bounds.h <= a.bounds.y;
            assert!(apart, "{} and {} overlap", a.series, b.series);
        }
    }
    assert_eq!(sc.legend_pick(100.0, 100.0), None);
    // A hidden entry is hit like any other (that is how it comes back).
    let hidden = hidden_scene("bar-stacked", &["Software"]);
    let [cx, cy] = hidden.legend_center("Software").unwrap();
    assert_eq!(hidden.legend_pick(cx, cy), Some("Software"));
}

#[test]
fn hits_follow_the_legend_on_every_side() {
    for orient in ["right", "left", "top", "bottom"] {
        let spec = with_legend("bar-stacked", &format!(r#"{{ "orient": "{orient}" }}"#));
        let sc = scene(&spec);
        assert_eq!(sc.legend_hits.len(), 3, "{orient}");
        for h in &sc.legend_hits {
            let r = sc
                .rules
                .iter()
                .find(|r| r.key == format!("legend/swatch:{}", h.series))
                .unwrap();
            assert!(
                h.bounds.contains(r.bounds.x + 1.0, r.bounds.y + 1.0),
                "{orient} {}",
                h.series
            );
        }
    }
}

#[test]
fn a_legend_that_is_not_interactive_has_nothing_to_click() {
    let off = scene(&with_legend("bar-stacked", r#"{ "interactive": false }"#));
    assert!(off.legend_hits.is_empty());
    assert_eq!(
        off.rules
            .iter()
            .filter(|r| r.kind == RuleKind::Swatch)
            .count(),
        3
    );
    assert!(
        scene(&with_legend("bar-stacked", r#"{ "interactive": true }"#))
            .legend_hits
            .len()
            == 3
    );
    assert!(
        scene(&with_legend("bar-stacked", "false"))
            .legend_hits
            .is_empty()
    );
    // Hiding still works through `hidden`: only the clicking is off.
    let both = scene(&with_legend(
        "bar-stacked",
        r#"{ "interactive": false, "hidden": ["Software"] }"#,
    ));
    assert_eq!(both.rects.len(), 8);
}

#[test]
fn the_pie_legend_is_clickable_too() {
    let sc = scene(&with_legend("pie", "true"));
    assert_eq!(sc.legend_hits.len(), 6);
    let [x, y] = sc.legend_center("Direct").unwrap();
    assert_eq!(sc.legend_pick(x, y), Some("Direct"));
}

#[test]
fn three_d_stacks_restack_and_the_overlay_has_the_hits() {
    let spec = with_legend("bar3d-stacked", r#"{ "hidden": ["Software"] }"#);
    let sc = build_scene_3d(&spec, (800, 500), 1.0, FontMetrics::inter());
    assert_eq!(sc.boxes.len(), 24);
    assert!(sc.boxes.iter().all(|b| !b.key.contains("Software")));
    // Services sits straight on Hardware in every stack.
    let b = |k: &str| sc.boxes.iter().find(|b| b.key == k).unwrap();
    assert_eq!(
        b("layer0/Q1/Services/North").min[1],
        b("layer0/Q1/Hardware/North").max[1]
    );
    assert_eq!(sc.overlay.hits.len(), 3);
    let [x, y] = sc.legend_center("Software").unwrap();
    assert_eq!(sc.legend_pick(x, y), Some("Software"));
    assert_eq!(sc.legend_pick(5.0, 250.0), None);
    // The legend is still fully listed, dimmed.
    assert_eq!(sc.overlay.rules.len(), 3);
    assert!(sc.overlay.rules.iter().any(|r| r.color.a < 0.5));
}

#[test]
fn a_3d_depth_row_only_a_hidden_series_used_leaves_the_axis() {
    // Colour and z on the same field: every series has its own row.
    let json = r#"{
      "dimension": "3d",
      "data": { "t": { "x": ["a","b","a","b"], "s": ["u","u","v","v"], "y": [1,2,3,4] } },
      "axes": [ {"id":"x","orient":"bottom"}, {"id":"y","orient":"left"}, {"id":"z","orient":"depth"} ],
      "layers": [ { "mark": "bar", "data": "t",
        "encoding": { "x": {"field":"x"}, "y": {"field":"y"}, "z": {"field":"s"}, "color": {"field":"s"} } } ],
      "legend": { "hidden": ["v"] } }"#;
    let spec = parse_spec_json(json).unwrap();
    assert_eq!(validate(&spec), vec![]);
    let sc = build_scene_3d(&spec, (800, 500), 1.0, FontMetrics::inter());
    assert_eq!(sc.boxes.len(), 2);
    // One row left: the box is one cell deep.
    assert!(
        (sc.size[2] - sc.size[0] / 2.0).abs() < 1e-4,
        "{:?}",
        sc.size
    );
}

// --- the hidden set ---------------------------------------------------------------------

fn names(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

#[test]
fn series_are_the_colour_categories_in_legend_order() {
    let spec = with_legend("bar-stacked", "true");
    assert_eq!(series_names(&spec), ["Hardware", "Software", "Services"]);
    assert_eq!(
        series_names(&parse_spec_json(&example("bar-basic")).unwrap()),
        Vec::<String>::new()
    );
}

#[test]
fn toggling_hides_and_shows_and_keeps_legend_order() {
    let mut spec = with_legend("bar-stacked", "true");
    assert!(set_series_visible(&mut spec, "Services", false));
    assert!(set_series_visible(&mut spec, "Hardware", false));
    assert_eq!(names(spec.hidden_series()), ["Hardware", "Services"]);
    // Already hidden / already shown: no change.
    assert!(!set_series_visible(&mut spec, "Hardware", false));
    assert!(!set_series_visible(&mut spec, "Software", true));
    assert!(set_series_visible(&mut spec, "Hardware", true));
    assert_eq!(names(spec.hidden_series()), ["Services"]);
}

#[test]
fn the_last_visible_series_cannot_be_hidden_and_unknown_names_do_nothing() {
    let mut spec = with_legend("bar-stacked", "true");
    assert!(set_series_visible(&mut spec, "Hardware", false));
    assert!(set_series_visible(&mut spec, "Software", false));
    assert!(
        !set_series_visible(&mut spec, "Services", false),
        "that would leave nothing"
    );
    assert_eq!(names(spec.hidden_series()), ["Hardware", "Software"]);
    assert!(!set_series_visible(&mut spec, "Nope", false));
    assert!(!set_series_visible(&mut spec, "Nope", true));
    assert_eq!(scene(&spec).rects.len(), 4);
}

#[test]
fn an_update_keeps_what_the_viewer_hid_for_series_that_still_exist() {
    let mut old = with_legend("bar-stacked", "true");
    settle_hidden(&mut old, None);
    assert!(old.hidden_series().is_empty());
    set_series_visible(&mut old, "Software", false);
    set_series_visible(&mut old, "Services", false);

    // The same spec again: both stay hidden.
    let mut next = with_legend("bar-stacked", "true");
    settle_hidden(&mut next, Some(old.hidden_series()));
    assert_eq!(names(next.hidden_series()), ["Software", "Services"]);

    // New data without Services: only Software stays hidden.
    let json = example("bar-stacked").replace("Services", "Hardware");
    let mut fewer = parse_spec_json(&json).unwrap();
    settle_hidden(&mut fewer, Some(old.hidden_series()));
    assert_eq!(names(fewer.hidden_series()), ["Software"]);

    // New data that is only hidden series: everything is shown rather than nothing.
    let json = example("bar-stacked")
        .replace("Hardware", "Software")
        .replace("Services", "Software");
    let mut only = parse_spec_json(&json).unwrap();
    settle_hidden(&mut only, Some(old.hidden_series()));
    assert!(only.hidden_series().is_empty());
}

#[test]
fn legend_hidden_in_the_new_spec_wins_over_the_viewers_choice() {
    let mut old = with_legend("bar-stacked", "true");
    set_series_visible(&mut old, "Software", false);
    let mut next = with_legend("bar-stacked", r#"{ "hidden": ["Hardware"] }"#);
    settle_hidden(&mut next, Some(old.hidden_series()));
    assert_eq!(names(next.hidden_series()), ["Hardware"]);
    // An explicit empty list shows everything again.
    let mut shown = with_legend("bar-stacked", r#"{ "hidden": [] }"#);
    settle_hidden(&mut shown, Some(old.hidden_series()));
    assert!(shown.hidden_series().is_empty());
}

#[test]
fn unknown_names_in_legend_hidden_are_ignored_and_hiding_everything_is_an_issue() {
    let spec = with_legend("bar-stacked", r#"{ "hidden": ["Nope", "Software"] }"#);
    assert_eq!(scene(&spec).rects.len(), 8);
    let mut settled = spec.clone();
    settle_hidden(&mut settled, None);
    assert_eq!(
        names(settled.hidden_series()),
        ["Software"],
        "unknown names are dropped"
    );

    let all = r#"{ "hidden": ["Hardware", "Software", "Services"] }"#;
    let mut value: serde_json::Value = serde_json::from_str(&example("bar-stacked")).unwrap();
    value["legend"] = serde_json::from_str(all).unwrap();
    let spec = parse_spec_json(&value.to_string()).unwrap();
    let issues = validate(&spec);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].path, "/legend/hidden");
}

#[test]
fn the_legend_options_are_checked_for_type_and_typos() {
    for bad in [
        r#"{ "interactive": "yes" }"#,
        r#"{ "hidden": "Software" }"#,
        r#"{ "hidden": [1] }"#,
        r#"{ "hide": [] }"#,
    ] {
        let mut value: serde_json::Value = serde_json::from_str(&example("bar-stacked")).unwrap();
        value["legend"] = serde_json::from_str(bad).unwrap();
        assert!(parse_spec_json(&value.to_string()).is_err(), "{bad}");
    }
}

#[test]
fn hiding_is_deterministic_and_does_not_touch_the_spec_data() {
    let spec = with_legend("bar-stacked", r#"{ "hidden": ["Software"] }"#);
    assert_eq!(scene(&spec), scene(&spec));
    let before = spec.clone();
    let _ = scene(&spec);
    assert_eq!(spec, before);
}
