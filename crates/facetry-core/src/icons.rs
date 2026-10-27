//! The built-in icons: simple line drawings (drawn for Facetry, in the spirit of open line-icon
//! sets) that a diagram prints on a surface, like the icons of a printed infographic. Each is a
//! list of strokes in a 24 by 24 box (y down), written as a small path language:
//!
//! * `M x y` starts a stroke, `L x y` continues it, `Z` closes it;
//! * `O cx cy r` is a circle; `E cx cy rx ry` an ellipse;
//! * `A cx cy r a0 a1` an arc and `B cx cy rx ry a0 a1` an elliptical one, from angle `a0` to
//!   `a1` in degrees (0 to the right, 90 down, as y points down);
//! * `S cx cy r0 r1 n` is `n` spokes from radius `r0` to `r1` (a gear's teeth);
//! * `P cx cy r0 r1 n` is a star of `n` points, outer radius `r0`, inner `r1`.
//!
//! Names follow common icon sets, and Mermaid-style prefixed names (`fa:lightbulb`) are read by
//! dropping the prefix and looking the rest up among the names and aliases.

/// `(name, path)`, sorted by name.
const ICONS: &[(&str, &str)] = &[
    (
        "alert",
        "M 12 3 L 21.5 20 L 2.5 20 Z M 12 9 L 12 14 O 12 17 0.6",
    ),
    (
        "bars",
        "M 4 20 L 20 20 M 7 20 L 7 13 M 12 20 L 12 6 M 17 20 L 17 10",
    ),
    (
        "bolt",
        "M 13 2.5 L 5 14 L 11.5 14 L 10.5 21.5 L 19 9.5 L 12.5 9.5 Z",
    ),
    (
        "bulb",
        "A 12 9.5 6 140 400 M 7.4 13.4 L 9.5 15.5 L 9.5 18 L 14.5 18 L 14.5 15.5 L 16.6 13.4 M 10 20.5 L 14 20.5",
    ),
    (
        "calendar",
        "M 3.5 5 L 20.5 5 L 20.5 20.5 L 3.5 20.5 Z M 3.5 9.5 L 20.5 9.5 M 8 3 L 8 7 M 16 3 L 16 7",
    ),
    (
        "cart",
        "M 2.5 4 L 5.5 4 L 8 15 L 18 15 L 20.5 7 L 6.5 7 O 9.5 19 1.5 O 16.5 19 1.5",
    ),
    ("chart", "O 12 12 8.5 M 12 12 L 12 3.5 M 12 12 L 19.4 16.2"),
    ("check", "O 12 12 9 M 7.8 12.3 L 10.8 15.3 L 16.5 9"),
    ("clock", "O 12 12 9 M 12 6.5 L 12 12 L 15.8 14.2"),
    (
        "cloud",
        "A 7.5 14.5 3.5 90 270 A 12.5 11 5.2 195 345 A 17 14.5 3.5 270 450 M 7.5 18 L 17 18",
    ),
    (
        "code",
        "M 8 7 L 3 12 L 8 17 M 16 7 L 21 12 L 16 17 M 14 5 L 10 19",
    ),
    (
        "database",
        "E 12 6 7 2.5 M 5 6 L 5 18 M 19 6 L 19 18 B 12 12 7 2.5 0 180 B 12 18 7 2.5 0 180",
    ),
    (
        "disk",
        "E 12 8 8 3 M 4 8 L 4 16 M 20 8 L 20 16 B 12 16 8 3 0 180 O 16.5 13.5 0.6",
    ),
    (
        "file",
        "M 6 3 L 14 3 L 18 7 L 18 21 L 6 21 Z M 14 3 L 14 7 L 18 7 M 9 12 L 15 12 M 9 16 L 15 16",
    ),
    ("flag", "M 5 21 L 5 3 M 5 4 L 18 4 L 15 8 L 18 12 L 5 12"),
    ("gear", "O 12 12 3 O 12 12 6.5 S 12 12 6.5 9.5 8"),
    ("globe", "O 12 12 9 E 12 12 4 9 M 3 12 L 21 12"),
    (
        "heart",
        "A 8.3 9 4.3 150 360 A 15.7 9 4.3 180 390 M 4.6 11.2 L 12 20 L 19.4 11.2",
    ),
    (
        "home",
        "M 3 11 L 12 3.5 L 21 11 M 5.5 9 L 5.5 20 L 18.5 20 L 18.5 9 M 10 20 L 10 14 L 14 14 L 14 20",
    ),
    (
        "lock",
        "M 6 11 L 18 11 L 18 20.5 L 6 20.5 Z A 12 8 4 180 360 M 8 8 L 8 11 M 16 8 L 16 11 O 12 15.5 1",
    ),
    ("mail", "M 3 6 L 21 6 L 21 18 L 3 18 Z M 3 6 L 12 13 L 21 6"),
    (
        "money",
        "O 12 12 9 M 15 8.5 L 10.5 8.5 L 9.5 9.5 L 9.5 11 L 10.5 12 L 13.5 12 L 14.5 13 L 14.5 14.5 L 13.5 15.5 L 9 15.5 M 12 6.5 L 12 17.5",
    ),
    (
        "phone",
        "M 7 2.5 L 17 2.5 L 17 21.5 L 7 21.5 Z M 10.5 18.5 L 13.5 18.5",
    ),
    (
        "rocket",
        "M 12 2.5 L 15.5 7.5 L 15.5 16 L 8.5 16 L 8.5 7.5 Z M 8.5 11.5 L 5.5 16 L 8.5 16 M 15.5 11.5 L 18.5 16 L 15.5 16 M 10.5 18 L 12 21.5 L 13.5 18 O 12 10 1.5",
    ),
    ("search", "O 10.5 10.5 6.5 M 15.3 15.3 L 20.5 20.5"),
    (
        "server",
        "M 4 4 L 20 4 L 20 10 L 4 10 Z M 4 14 L 20 14 L 20 20 L 4 20 Z O 7.5 7 0.8 O 7.5 17 0.8",
    ),
    (
        "shield",
        "M 12 2.5 L 20 5.5 L 20 11.5 L 12 21.5 L 4 11.5 L 4 5.5 Z M 8.5 11.5 L 11 14 L 15.5 9",
    ),
    ("star", "P 12 12.5 9.5 4 5"),
    ("target", "O 12 12 9 O 12 12 5.5 O 12 12 2"),
    (
        "truck",
        "M 2.5 6 L 14.5 6 L 14.5 16.5 L 2.5 16.5 Z M 14.5 9.5 L 18.5 9.5 L 21.5 13 L 21.5 16.5 L 14.5 16.5 O 7 18.5 2 O 17.5 18.5 2",
    ),
    ("user", "O 12 8 4 A 12 21 7.5 180 360 M 4.5 21 L 19.5 21"),
    (
        "users",
        "O 9 8 3.5 A 9 20 6.5 180 360 M 2.5 20 L 15.5 20 O 16.5 8.5 2.8 A 17 19 5 210 340",
    ),
];

/// Other names for the icons (as common icon sets spell them).
const ALIASES: &[(&str, &str)] = &[
    ("bar-chart", "bars"),
    ("calendar-days", "calendar"),
    ("chart-bar", "bars"),
    ("chart-pie", "chart"),
    ("circle-check", "check"),
    ("cog", "gear"),
    ("dollar", "money"),
    ("document", "file"),
    ("envelope", "mail"),
    ("exclamation-triangle", "alert"),
    ("gears", "gear"),
    ("hard-drive", "disk"),
    ("house", "home"),
    ("idea", "bulb"),
    ("internet", "globe"),
    ("lightbulb", "bulb"),
    ("lightning", "bolt"),
    ("magnifying-glass", "search"),
    ("pie", "chart"),
    ("settings", "gear"),
    ("shopping-cart", "cart"),
    ("time", "clock"),
    ("triangle-exclamation", "alert"),
    ("warning", "alert"),
];

/// The names of all built-in icons, sorted.
pub fn names() -> impl Iterator<Item = &'static str> {
    ICONS.iter().map(|(n, _)| *n)
}

/// The built-in icon a name stands for (dropping an `fa:`-style prefix and following aliases).
pub fn resolve(name: &str) -> Option<&'static str> {
    let bare = name.rsplit(':').next().unwrap_or(name).trim();
    let bare = ALIASES
        .iter()
        .find(|(a, _)| *a == bare)
        .map_or(bare, |(_, n)| n);
    ICONS.iter().find(|(n, _)| *n == bare).map(|(n, _)| *n)
}

/// Degrees between samples of a curve.
const STEP_DEGREES: f64 = 15.0;

/// The digits of a standing numeral, drawn like the icons as centre lines of strokes
/// [`DIGIT_STROKE`] thick, in a box [`DIGIT_WIDTH`] wide and 24 high (y down): a rounded monoline
/// face, `0` to `9`.
const DIGITS: [&str; 10] = [
    "E 7 12 5.3 10.3",
    "M 3.2 5.6 L 7.6 1.7 L 7.6 22.3",
    "B 7 6.9 5.3 5.2 190 380 L 1.7 22.3 L 12.3 22.3",
    "B 7 6.5 4.9 4.8 200 450 B 7 16.6 5.3 5.7 270 520",
    "M 10.2 22.3 L 10.2 1.7 L 1.7 15.8 L 12.3 15.8",
    "M 12.3 1.7 L 3 1.7 L 2.4 11.6 B 7 15.6 5.3 6.7 228 495",
    "E 7 15.6 5.3 6.7 B 7 12 5.3 10.3 300 165",
    "M 1.7 1.7 L 12.3 1.7 L 5 22.3",
    "E 7 6.4 4.6 4.7 E 7 16.4 5.3 5.9",
    "E 7 8.4 5.3 6.7 B 7 12 5.3 10.3 120 -15",
];

/// A digit's width in its 24-high box, and its strokes' thickness.
pub const DIGIT_WIDTH: f64 = 14.0;
pub const DIGIT_STROKE: f64 = 3.4;

/// The strokes of digit `d` (`'0'..='9'`; nothing for anything else), in its
/// [`DIGIT_WIDTH`] by 24 box.
pub fn digit_strokes(d: char) -> Vec<Vec<[f64; 2]>> {
    d.to_digit(10)
        .map_or_else(Vec::new, |k| path_strokes(DIGITS[k as usize]))
}

/// The strokes of icon `name` (already resolved), in its 24 by 24 box.
pub fn strokes(name: &str) -> Vec<Vec<[f64; 2]>> {
    ICONS
        .iter()
        .find(|(n, _)| *n == name)
        .map_or_else(Vec::new, |(_, path)| path_strokes(path))
}

/// The strokes a path in the language above draws.
fn path_strokes(path: &str) -> Vec<Vec<[f64; 2]>> {
    let tokens: Vec<&str> = path.split_whitespace().collect();
    let mut out: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut i = 0;
    let num = |k: usize| {
        tokens
            .get(k)
            .and_then(|t| t.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let arc = |cx: f64, cy: f64, rx: f64, ry: f64, a0: f64, a1: f64| {
        let n = (((a1 - a0).abs() / STEP_DEGREES).ceil() as usize).max(2);
        (0..=n)
            .map(|k| {
                let a = (a0 + (a1 - a0) * k as f64 / n as f64).to_radians();
                [cx + rx * a.cos(), cy + ry * a.sin()]
            })
            .collect::<Vec<_>>()
    };
    while i < tokens.len() {
        match tokens[i] {
            "M" => {
                out.push(vec![[num(i + 1), num(i + 2)]]);
                i += 3;
            }
            "L" => {
                if let Some(last) = out.last_mut() {
                    last.push([num(i + 1), num(i + 2)]);
                }
                i += 3;
            }
            "Z" => {
                if let Some(last) = out.last_mut() {
                    if let Some(&first) = last.first() {
                        last.push(first);
                    }
                }
                i += 1;
            }
            "O" => {
                let r = num(i + 3);
                out.push(arc(num(i + 1), num(i + 2), r, r, 0.0, 360.0));
                i += 4;
            }
            "E" => {
                out.push(arc(
                    num(i + 1),
                    num(i + 2),
                    num(i + 3),
                    num(i + 4),
                    0.0,
                    360.0,
                ));
                i += 5;
            }
            "A" => {
                let r = num(i + 3);
                out.push(arc(num(i + 1), num(i + 2), r, r, num(i + 4), num(i + 5)));
                i += 6;
            }
            "B" => {
                out.push(arc(
                    num(i + 1),
                    num(i + 2),
                    num(i + 3),
                    num(i + 4),
                    num(i + 5),
                    num(i + 6),
                ));
                i += 7;
            }
            "S" => {
                let (cx, cy, r0, r1) = (num(i + 1), num(i + 2), num(i + 3), num(i + 4));
                let n = num(i + 5).max(1.0) as usize;
                for k in 0..n {
                    let a = (360.0 * k as f64 / n as f64).to_radians();
                    out.push(vec![
                        [cx + r0 * a.cos(), cy + r0 * a.sin()],
                        [cx + r1 * a.cos(), cy + r1 * a.sin()],
                    ]);
                }
                i += 6;
            }
            "P" => {
                let (cx, cy, r0, r1) = (num(i + 1), num(i + 2), num(i + 3), num(i + 4));
                let n = num(i + 5).max(2.0) as usize;
                let mut star: Vec<[f64; 2]> = (0..2 * n)
                    .map(|k| {
                        let r = if k % 2 == 0 { r0 } else { r1 };
                        let a = (-90.0 + 180.0 * k as f64 / n as f64).to_radians();
                        [cx + r * a.cos(), cy + r * a.sin()]
                    })
                    .collect();
                star.push(star[0]);
                out.push(star);
                i += 6;
            }
            _ => i += 1,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_draws_inside_its_box() {
        assert!(ICONS.windows(2).all(|w| w[0].0 < w[1].0), "sorted by name");
        for (name, _) in ICONS {
            let s = strokes(name);
            assert!(!s.is_empty(), "{name}");
            for stroke in &s {
                assert!(stroke.len() >= 2, "{name}");
                for p in stroke {
                    assert!(
                        (0.0..=24.0).contains(&p[0]) && (0.0..=24.0).contains(&p[1]),
                        "{name}: {p:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_digit_draws_inside_its_box_with_its_stroke() {
        let half = DIGIT_STROKE / 2.0 - 0.05;
        for d in '0'..='9' {
            let s = digit_strokes(d);
            assert!(!s.is_empty(), "{d}");
            for p in s.iter().flatten() {
                assert!(
                    (half..=DIGIT_WIDTH - half).contains(&p[0])
                        && (half..=24.0 - half).contains(&p[1]),
                    "{d}: {p:?}"
                );
            }
        }
        assert!(digit_strokes('x').is_empty());
        // Each digit has a face of its own.
        for (i, a) in DIGITS.iter().enumerate() {
            assert!(DIGITS[i + 1..].iter().all(|b| a != b), "digit {i}");
        }
    }

    #[test]
    fn names_aliases_and_prefixes_resolve() {
        assert_eq!(resolve("bulb"), Some("bulb"));
        assert_eq!(resolve("fa:lightbulb"), Some("bulb"));
        assert_eq!(resolve("mdi:cog"), Some("gear"));
        assert_eq!(resolve("idea"), Some("bulb"));
        assert_eq!(resolve("unicorn"), None);
        for (alias, target) in ALIASES {
            assert_eq!(resolve(alias), Some(*target), "{alias}");
        }
    }
}
