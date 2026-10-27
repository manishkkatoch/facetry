//! Filling closed outlines with triangles, for filled icons: SVG's fill rules on any number of
//! contours, holes and all. The plane is cut into bands at every vertex's height; inside a band
//! no edge starts or ends, so the edges crossing it are ordered left to right, the fill rule
//! picks the spans between them that are inside, and each span is a trapezoid (two triangles).
//! Simple and robust (no ear clipping, no special cases for holes or touching contours); edges
//! that cross each other inside a band (a self-intersecting outline) are drawn slightly off,
//! which icons do not do.

pub use crate::spec::FillRule;

/// Bands thinner than this are skipped (they hold no area worth drawing).
const EPS: f64 = 1e-9;

/// Triangles covering the inside of `contours` (each closed back to its start), by `rule`.
pub fn triangulate(contours: &[Vec<[f64; 2]>], rule: FillRule) -> Vec<[[f64; 2]; 3]> {
    // Every non-horizontal edge, from its upper end to its lower end, with its direction.
    struct Edge {
        top: [f64; 2],
        bottom: [f64; 2],
        winding: i32,
    }
    let mut edges: Vec<Edge> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    for contour in contours {
        let n = contour.len();
        if n < 3 {
            continue;
        }
        for i in 0..n {
            let (a, b) = (contour[i], contour[(i + 1) % n]);
            if !(a[0].is_finite() && a[1].is_finite()) {
                continue;
            }
            ys.push(a[1]);
            if (a[1] - b[1]).abs() <= EPS {
                continue;
            }
            let (top, bottom, winding) = if a[1] < b[1] { (a, b, 1) } else { (b, a, -1) };
            edges.push(Edge {
                top,
                bottom,
                winding,
            });
        }
    }
    sort(&mut ys, |y| *y);
    ys.dedup_by(|a, b| (*a - *b).abs() <= EPS);
    let x_at = |e: &Edge, y: f64| {
        let t = (y - e.top[1]) / (e.bottom[1] - e.top[1]);
        e.top[0] + (e.bottom[0] - e.top[0]) * t
    };
    let mut out = Vec::new();
    let mut crossing: Vec<(f64, usize)> = Vec::new();
    for band in ys.windows(2) {
        let (y0, y1) = (band[0], band[1]);
        if y1 - y0 <= EPS {
            continue;
        }
        let mid = (y0 + y1) / 2.0;
        // Every edge is looked at in every band: icons are small, and this keeps the code
        // (and the wasm) small.
        crossing.clear();
        crossing.extend(
            edges
                .iter()
                .enumerate()
                .filter(|(_, e)| e.top[1] < mid && e.bottom[1] > mid)
                .map(|(i, e)| (x_at(e, mid), i)),
        );
        sort(&mut crossing, |c| c.0);
        let mut winding = 0;
        for pair in crossing.windows(2) {
            let (left, right) = (&edges[pair[0].1], &edges[pair[1].1]);
            winding += left.winding;
            let inside = match rule {
                FillRule::NonZero => winding != 0,
                FillRule::EvenOdd => winding % 2 != 0,
            };
            if !inside {
                continue;
            }
            let (a, b) = ([x_at(left, y0), y0], [x_at(right, y0), y0]);
            let (c, d) = ([x_at(right, y1), y1], [x_at(left, y1), y1]);
            out.push([a, b, c]);
            out.push([a, c, d]);
        }
    }
    out
}

/// Sorts `items` by `key` in place: an insertion sort, which is quick on the short or nearly
/// ordered lists here and far smaller in wasm than the standard library's sorts.
fn sort<T: Copy>(items: &mut [T], key: impl Fn(&T) -> f64) {
    for i in 1..items.len() {
        let item = items[i];
        let k = key(&item);
        let mut j = i;
        while j > 0 && key(&items[j - 1]) > k {
            items[j] = items[j - 1];
            j -= 1;
        }
        items[j] = item;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(triangles: &[[[f64; 2]; 3]]) -> f64 {
        triangles
            .iter()
            .map(|[a, b, c]| {
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
            })
            .sum()
    }

    fn square(x: f64, y: f64, s: f64, clockwise: bool) -> Vec<[f64; 2]> {
        let mut p = vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]];
        if !clockwise {
            p.reverse();
        }
        p
    }

    #[test]
    fn a_square_is_filled_whole() {
        let t = triangulate(&[square(0.0, 0.0, 10.0, true)], FillRule::NonZero);
        assert!((area(&t) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn a_hole_wound_the_other_way_is_left_open_by_either_rule() {
        let ring = [square(0.0, 0.0, 10.0, true), square(3.0, 3.0, 4.0, false)];
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            assert!(
                (area(&triangulate(&ring, rule)) - 84.0).abs() < 1e-9,
                "{rule:?}"
            );
        }
        // Wound the same way, nonzero fills the hole and even-odd keeps it.
        let same = [square(0.0, 0.0, 10.0, true), square(3.0, 3.0, 4.0, true)];
        assert!((area(&triangulate(&same, FillRule::NonZero)) - 100.0).abs() < 1e-9);
        assert!((area(&triangulate(&same, FillRule::EvenOdd)) - 84.0).abs() < 1e-9);
    }

    #[test]
    fn slanted_and_round_outlines_keep_their_area() {
        let triangle = vec![[0.0, 0.0], [10.0, 4.0], [3.0, 9.0]];
        let t = triangulate(&[triangle], FillRule::NonZero);
        assert!((area(&t) - 39.0).abs() < 1e-9, "{}", area(&t));
        let n = 64;
        let circle: Vec<[f64; 2]> = (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * f64::from(k) / f64::from(n);
                [5.0 * a.cos(), 5.0 * a.sin()]
            })
            .collect();
        let polygon = 0.5 * f64::from(n) * 25.0 * (std::f64::consts::TAU / f64::from(n)).sin();
        assert!((area(&triangulate(&[circle], FillRule::EvenOdd)) - polygon).abs() < 1e-9);
    }

    #[test]
    fn nothing_to_fill_gives_nothing() {
        assert!(triangulate(&[], FillRule::NonZero).is_empty());
        assert!(triangulate(&[vec![[0.0, 0.0], [5.0, 5.0]]], FillRule::NonZero).is_empty());
        let flat = vec![[0.0, 1.0], [5.0, 1.0], [9.0, 1.0]];
        assert!(triangulate(&[flat], FillRule::NonZero).is_empty());
    }
}
