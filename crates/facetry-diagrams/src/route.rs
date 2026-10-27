//! Edge geometry once nodes are placed: where an edge leaves and enters a node's outline,
//! the smooth curve through its control points, and self-loops.

/// Where the ray from `from` toward `to` leaves the convex polygon `outline` (absolute
/// coordinates, `from` inside it). `from` itself if the ray never leaves (degenerate input).
pub fn clip(outline: &[[f64; 2]], from: [f64; 2], to: [f64; 2]) -> [f64; 2] {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    if dx.hypot(dy) < 1e-9 {
        return from;
    }
    let mut best: Option<f64> = None;
    let n = outline.len();
    for i in 0..n {
        let (a, b) = (outline[i], outline[(i + 1) % n]);
        let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
        let denom = dx * ey - dy * ex;
        if denom.abs() < 1e-12 {
            continue;
        }
        // from + t d = a + s e
        let (fx, fy) = (a[0] - from[0], a[1] - from[1]);
        let t = (fx * ey - fy * ex) / denom;
        let s = (fx * dy - fy * dx) / denom;
        // From inside a convex polygon the ray crosses its border once going out (and, on
        // the border already, once more at t = 0): the largest t is the exit.
        if t >= 0.0 && (-1e-9..=1.0 + 1e-9).contains(&s) {
            best = Some(best.map_or(t, |b: f64| b.max(t)));
        }
    }
    match best {
        Some(t) => [from[0] + dx * t, from[1] + dy * t],
        None => from,
    }
}

/// Samples per cubic segment of a smoothed edge.
const SAMPLES: usize = 8;

/// The uniform cubic B-spline through `points` that starts and ends exactly on the first and
/// last point: d3's `curveBasis`, which Mermaid draws flowchart edges with. It passes near,
/// not through, the inner points, which turns a dummy chain's corners into gentle bends.
/// Returned as a polyline.
pub fn basis(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    let lerp6 = |a: [f64; 2], b: [f64; 2], wa: f64, wb: f64| {
        [(wa * a[0] + wb * b[0]) / 6.0, (wa * a[1] + wb * b[1]) / 6.0]
    };
    let cubic =
        |out: &mut Vec<[f64; 2]>, p0: [f64; 2], c1: [f64; 2], c2: [f64; 2], p3: [f64; 2]| {
            for k in 1..=SAMPLES {
                let t = k as f64 / SAMPLES as f64;
                let u = 1.0 - t;
                let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                out.push([
                    a * p0[0] + b * c1[0] + c * c2[0] + d * p3[0],
                    a * p0[1] + b * c1[1] + c * c2[1] + d * p3[1],
                ]);
            }
        };
    // d3: after the first two points, line to (5 p0 + p1) / 6; then one Bézier per point.
    let (mut x0, mut x1) = (points[0], points[1]);
    out.push(lerp6(x0, x1, 5.0, 1.0));
    let mut current = *out.last().unwrap_or(&points[0]);
    let bez = |out: &mut Vec<[f64; 2]>,
               current: &mut [f64; 2],
               x0: [f64; 2],
               x1: [f64; 2],
               x: [f64; 2]| {
        let c1 = [(2.0 * x0[0] + x1[0]) / 3.0, (2.0 * x0[1] + x1[1]) / 3.0];
        let c2 = [(x0[0] + 2.0 * x1[0]) / 3.0, (x0[1] + 2.0 * x1[1]) / 3.0];
        let end = [
            (x0[0] + 4.0 * x1[0] + x[0]) / 6.0,
            (x0[1] + 4.0 * x1[1] + x[1]) / 6.0,
        ];
        cubic(out, *current, c1, c2, end);
        *current = end;
    };
    for &x in &points[2..] {
        bez(&mut out, &mut current, x0, x1, x);
        x0 = x1;
        x1 = x;
    }
    // lineEnd: one more segment toward the last point, then a line onto it.
    bez(&mut out, &mut current, x0, x1, x1);
    out.push(x1);
    dedup(out)
}

fn dedup(points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    for p in points {
        if !out
            .last()
            .is_some_and(|q| (q[0] - p[0]).hypot(q[1] - p[1]) < 1e-6)
        {
            out.push(p);
        }
    }
    out
}

/// Control points of a self-loop on a node centred at `c`, `w` by `h`, bulging out of its
/// right side (`right`) or its bottom.
pub fn self_loop(c: [f64; 2], w: f64, h: f64, right: bool) -> Vec<[f64; 2]> {
    let reach = 22.0;
    if right {
        let x = c[0] + w / 2.0;
        vec![
            [c[0], c[1] - h / 4.0],
            [x + reach * 0.6, c[1] - h / 2.0 - 4.0],
            [x + reach, c[1]],
            [x + reach * 0.6, c[1] + h / 2.0 + 4.0],
            [c[0], c[1] + h / 4.0],
        ]
    } else {
        let y = c[1] + h / 2.0;
        vec![
            [c[0] - w / 4.0, c[1]],
            [c[0] - w / 2.0 - 4.0, y + reach * 0.6],
            [c[0], y + reach],
            [c[0] + w / 2.0 + 4.0, y + reach * 0.6],
            [c[0] + w / 4.0, c[1]],
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: [[f64; 2]; 4] = [[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0]];

    #[test]
    fn clipping_finds_the_border() {
        assert_eq!(clip(&SQUARE, [0.0, 0.0], [0.0, 50.0]), [0.0, 10.0]);
        assert_eq!(clip(&SQUARE, [0.0, 0.0], [100.0, 0.0]), [10.0, 0.0]);
        let p = clip(&SQUARE, [0.0, 0.0], [30.0, 30.0]);
        assert!((p[0] - 10.0).abs() < 1e-9 && (p[1] - 10.0).abs() < 1e-9);
        // A diamond.
        let diamond = [[0.0, -10.0], [20.0, 0.0], [0.0, 10.0], [-20.0, 0.0]];
        let p = clip(&diamond, [0.0, 0.0], [0.0, -40.0]);
        assert_eq!(p, [0.0, -10.0]);
    }

    #[test]
    fn the_basis_curve_keeps_its_ends_and_smooths_corners() {
        let pts = [[0.0, 0.0], [0.0, 50.0], [50.0, 100.0], [50.0, 150.0]];
        let c = basis(&pts);
        assert_eq!(c[0], pts[0]);
        assert_eq!(*c.last().unwrap(), pts[3]);
        // Monotone downwards and inside the hull.
        for w in c.windows(2) {
            assert!(w[1][1] >= w[0][1] - 1e-9);
        }
        assert!(c.iter().all(|p| (0.0..=50.0).contains(&p[0])));
        // Two points stay a straight line.
        assert_eq!(basis(&pts[..2]), pts[..2].to_vec());
    }
}
