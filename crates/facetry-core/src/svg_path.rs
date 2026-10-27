//! SVG path data (the `d` attribute) read into polylines, for icons people bring: every
//! command (`M L H V C S Q T A Z`, absolute and relative) with curves and arcs cut into short
//! straight steps. Numbers are read here rather than by `str::parse`, which would link the
//! whole standard float parser into the diagrams add-on, and because path data packs numbers
//! the standard parser does not split (`1.5.5` is two numbers, `1-2` too, arc flags `11`).

/// Straight steps per Bézier curve.
const CURVE_STEPS: usize = 10;
/// Degrees of an arc per straight step, at most.
const ARC_STEP_DEGREES: f64 = 15.0;
/// The most points a path may come to: an icon is a small drawing.
pub const MAX_POINTS: usize = 4000;

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn skip(&mut self) {
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b',')
        {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn error(&self, what: &str) -> String {
        if self.pos >= self.bytes.len() {
            format!("{what} at the end of the path")
        } else {
            format!("{what} at character {} of the path", self.pos + 1)
        }
    }

    fn number(&mut self) -> Result<f64, String> {
        self.skip();
        let digits = |r: &mut Self, mantissa: &mut u64, scale: &mut i32, fraction: bool| {
            let mut any = false;
            while let Some(b) = r.peek().filter(u8::is_ascii_digit) {
                any = true;
                // Past 18 digits the rest only round; keep the magnitude.
                if *mantissa < 100_000_000_000_000_000 {
                    *mantissa = *mantissa * 10 + u64::from(b - b'0');
                    if fraction {
                        *scale -= 1;
                    }
                } else if !fraction {
                    *scale += 1;
                }
                r.pos += 1;
            }
            any
        };
        let start = self.pos;
        let negative = match self.peek() {
            Some(b'-') => {
                self.pos += 1;
                true
            }
            Some(b'+') => {
                self.pos += 1;
                false
            }
            _ => false,
        };
        let (mut mantissa, mut scale) = (0u64, 0i32);
        let mut any = digits(self, &mut mantissa, &mut scale, false);
        if self.peek() == Some(b'.') {
            self.pos += 1;
            any |= digits(self, &mut mantissa, &mut scale, true);
        }
        if !any {
            self.pos = start;
            return Err(self.error("expected a number"));
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let back = self.pos;
            self.pos += 1;
            let minus = match self.peek() {
                Some(b'-') => {
                    self.pos += 1;
                    true
                }
                Some(b'+') => {
                    self.pos += 1;
                    false
                }
                _ => false,
            };
            let mut exponent = 0i32;
            let mut seen = false;
            while let Some(b) = self.peek().filter(u8::is_ascii_digit) {
                seen = true;
                exponent = (exponent * 10 + i32::from(b - b'0')).min(1000);
                self.pos += 1;
            }
            if seen {
                scale += if minus { -exponent } else { exponent };
            } else {
                self.pos = back;
            }
        }
        let value = mantissa as f64 * 10f64.powi(scale.clamp(-330, 330));
        if !value.is_finite() {
            return Err(self.error("number out of range"));
        }
        Ok(if negative { -value } else { value })
    }

    /// An arc's flag: a lone `0` or `1`, which may run into what follows.
    fn flag(&mut self) -> Result<bool, String> {
        self.skip();
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.pos += 1;
                Ok(true)
            }
            _ => Err(self.error("expected an arc flag (0 or 1)")),
        }
    }
}

/// The polylines `d` draws, in its own coordinates, or what is wrong with it.
pub fn flatten(d: &str) -> Result<Vec<Vec<[f64; 2]>>, String> {
    let mut r = Reader {
        bytes: d.as_bytes(),
        pos: 0,
    };
    let mut out: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut run: Vec<[f64; 2]> = Vec::new();
    let mut points = 0usize;
    let mut at = [0.0f64; 2];
    let mut start = [0.0f64; 2];
    // The last curve's second control point, for the smooth commands' reflection.
    let mut cubic: Option<[f64; 2]> = None;
    let mut quad: Option<[f64; 2]> = None;
    let mut command: Option<u8> = None;
    let finish = |run: &mut Vec<[f64; 2]>, out: &mut Vec<Vec<[f64; 2]>>| {
        if run.len() >= 2 {
            out.push(std::mem::take(run));
        } else {
            run.clear();
        }
    };
    loop {
        r.skip();
        let Some(b) = r.peek() else { break };
        let letter = b.is_ascii_alphabetic();
        if letter {
            if !b"MmLlHhVvCcSsQqTtAaZz".contains(&b) {
                return Err(r.error(&format!("unknown path command `{}`", b as char)));
            }
            if command.is_none() && !matches!(b, b'M' | b'm') {
                return Err(r.error("a path must start with `M`"));
            }
            r.pos += 1;
            command = Some(b);
        } else if command.is_none() {
            return Err(r.error("a path must start with `M`"));
        }
        let c = command.unwrap_or(b'M');
        if matches!(c, b'Z' | b'z') {
            if !letter {
                return Err(r.error("expected a path command"));
            }
            if !run.is_empty() {
                run.push(start);
            }
            finish(&mut run, &mut out);
            at = start;
            cubic = None;
            quad = None;
            continue;
        }
        let rel = c.is_ascii_lowercase();
        let base = if rel { at } else { [0.0, 0.0] };
        let point = |r: &mut Reader| -> Result<[f64; 2], String> {
            Ok([base[0] + r.number()?, base[1] + r.number()?])
        };
        let mut steps: Vec<[f64; 2]> = Vec::new();
        let (mut next_cubic, mut next_quad) = (None, None);
        match c.to_ascii_uppercase() {
            b'M' => {
                let p = point(&mut r)?;
                finish(&mut run, &mut out);
                run.push(p);
                points += 1;
                at = p;
                start = p;
                // Further pairs after a move are lines.
                command = Some(if rel { b'l' } else { b'L' });
                cubic = None;
                quad = None;
                continue;
            }
            b'L' => steps.push(point(&mut r)?),
            b'H' => {
                let x = r.number()? + if rel { at[0] } else { 0.0 };
                steps.push([x, at[1]]);
            }
            b'V' => {
                let y = r.number()? + if rel { at[1] } else { 0.0 };
                steps.push([at[0], y]);
            }
            b'C' | b'S' => {
                let c1 = if c.eq_ignore_ascii_case(&b'C') {
                    point(&mut r)?
                } else {
                    cubic.map_or(at, |q| [2.0 * at[0] - q[0], 2.0 * at[1] - q[1]])
                };
                let c2 = point(&mut r)?;
                let end = point(&mut r)?;
                for k in 1..=CURVE_STEPS {
                    let t = k as f64 / CURVE_STEPS as f64;
                    let u = 1.0 - t;
                    let (a, b2, c3, d4) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    steps.push([
                        a * at[0] + b2 * c1[0] + c3 * c2[0] + d4 * end[0],
                        a * at[1] + b2 * c1[1] + c3 * c2[1] + d4 * end[1],
                    ]);
                }
                next_cubic = Some(c2);
            }
            b'Q' | b'T' => {
                let c1 = if c.eq_ignore_ascii_case(&b'Q') {
                    point(&mut r)?
                } else {
                    quad.map_or(at, |q| [2.0 * at[0] - q[0], 2.0 * at[1] - q[1]])
                };
                let end = point(&mut r)?;
                for k in 1..=CURVE_STEPS {
                    let t = k as f64 / CURVE_STEPS as f64;
                    let u = 1.0 - t;
                    let (a, b2, c3) = (u * u, 2.0 * u * t, t * t);
                    steps.push([
                        a * at[0] + b2 * c1[0] + c3 * end[0],
                        a * at[1] + b2 * c1[1] + c3 * end[1],
                    ]);
                }
                next_quad = Some(c1);
            }
            _ => {
                // `A`: the arc to `end` on the ellipse of radii `rx`, `ry` turned `phi`
                // degrees, the large or small way round, either way (SVG 1.1, F.6.5).
                let (rx, ry) = (r.number()?.abs(), r.number()?.abs());
                let phi = r.number()?.to_radians();
                let (large, sweep) = (r.flag()?, r.flag()?);
                let end = point(&mut r)?;
                arc(at, rx, ry, phi, large, sweep, end, &mut steps);
            }
        }
        points += steps.len();
        if points > MAX_POINTS {
            return Err(format!(
                "the path is too detailed (over {MAX_POINTS} points once curves are cut into steps)"
            ));
        }
        if run.is_empty() {
            run.push(at);
        }
        if let Some(&last) = steps.last() {
            at = last;
        }
        run.extend(steps);
        cubic = next_cubic;
        quad = next_quad;
    }
    finish(&mut run, &mut out);
    if out.is_empty() {
        return Err("the path draws nothing".into());
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn arc(
    from: [f64; 2],
    rx: f64,
    ry: f64,
    phi: f64,
    large: bool,
    sweep: bool,
    to: [f64; 2],
    out: &mut Vec<[f64; 2]>,
) {
    if rx < 1e-12 || ry < 1e-12 || (from[0] - to[0]).hypot(from[1] - to[1]) < 1e-12 {
        out.push(to);
        return;
    }
    let (sin, cos) = phi.sin_cos();
    let (dx, dy) = ((from[0] - to[0]) / 2.0, (from[1] - to[1]) / 2.0);
    let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    let (mut rx, mut ry) = (rx, ry);
    let lambda = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut coef = (num / den).max(0.0).sqrt();
    if large == sweep {
        coef = -coef;
    }
    let (cx1, cy1) = (coef * rx * y1 / ry, -coef * ry * x1 / rx);
    let (cx, cy) = (
        cos * cx1 - sin * cy1 + (from[0] + to[0]) / 2.0,
        sin * cx1 + cos * cy1 + (from[1] + to[1]) / 2.0,
    );
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let (ux, uy) = ((x1 - cx1) / rx, (y1 - cy1) / ry);
    let (vx, vy) = ((-x1 - cx1) / rx, (-y1 - cy1) / ry);
    let theta = angle(1.0, 0.0, ux, uy);
    let mut delta = angle(ux, uy, vx, vy);
    let tau = std::f64::consts::TAU;
    if !sweep && delta > 0.0 {
        delta -= tau;
    } else if sweep && delta < 0.0 {
        delta += tau;
    }
    let n = ((delta.abs().to_degrees() / ARC_STEP_DEGREES).ceil() as usize).max(1);
    for k in 1..n {
        let t = theta + delta * k as f64 / n as f64;
        let (st, ct) = t.sin_cos();
        out.push([
            cx + rx * cos * ct - ry * sin * st,
            cy + rx * sin * ct + ry * cos * st,
        ]);
    }
    out.push(to);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
    }

    #[test]
    fn numbers_are_read_the_way_path_data_packs_them() {
        let runs = flatten("M1.5.5L-2-3e1 l+1,1E0 h.25V1e-1").unwrap();
        let p = &runs[0];
        assert!(close(p[0], [1.5, 0.5]));
        assert!(close(p[1], [-2.0, -30.0]));
        assert!(close(p[2], [-1.0, -29.0]));
        assert!(close(p[3], [-0.75, -29.0]));
        assert!(close(p[4], [-0.75, 0.1]));
    }

    #[test]
    fn moves_start_runs_implicit_pairs_are_lines_and_z_closes() {
        let runs = flatten("M0 0 10 0 10 10z m5 5 h2").unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(
            runs[0],
            vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 0.0]]
        );
        // After `z` the pen is back at the run's start, so `m5 5` is from (0, 0).
        assert_eq!(runs[1], vec![[5.0, 5.0], [7.0, 5.0]]);
    }

    #[test]
    fn curves_end_where_they_should_and_smooth_ones_reflect() {
        let runs = flatten("M0 0 C0 10 10 10 10 0 S20 -10 20 0 Q25 5 30 0 T40 0").unwrap();
        let p = &runs[0];
        assert_eq!(p.len(), 1 + 4 * CURVE_STEPS);
        assert!(close(p[CURVE_STEPS], [10.0, 0.0]));
        assert!(close(p[2 * CURVE_STEPS], [20.0, 0.0]));
        // The reflected control point pulls the second curve below the axis.
        assert!(p[CURVE_STEPS + CURVE_STEPS / 2][1] < 0.0);
        assert!(close(p[4 * CURVE_STEPS], [40.0, 0.0]));
    }

    #[test]
    fn arcs_follow_their_circle_with_packed_flags() {
        // Half a circle of radius 5 from (0, 0) to (10, 0), over the top (sweep 1, y down).
        let runs = flatten("M0 0a5 5 0 0110 0").unwrap();
        let p = &runs[0];
        assert!(close(*p.last().unwrap(), [10.0, 0.0]));
        for q in p {
            assert!(((q[0] - 5.0).hypot(q[1]) - 5.0).abs() < 1e-9, "{q:?}");
            assert!(q[1] <= 1e-9, "the top half, y up the page: {q:?}");
        }
        // A radius too small to reach is scaled up until it does.
        let p = &flatten("M0 0 A1 1 0 0 0 10 0").unwrap()[0];
        assert!(close(*p.last().unwrap(), [10.0, 0.0]));
        assert!(
            p.iter()
                .all(|q| ((q[0] - 5.0).hypot(q[1]) - 5.0).abs() < 1e-9)
        );
    }

    #[test]
    fn broken_paths_say_what_and_where() {
        let err = |d: &str| flatten(d).unwrap_err();
        assert_eq!(
            err("L 1 2"),
            "a path must start with `M` at character 1 of the path"
        );
        assert_eq!(
            err("M 1 2 K 3"),
            "unknown path command `K` at character 7 of the path"
        );
        assert_eq!(err("M 1 2 L 3"), "expected a number at the end of the path");
        assert_eq!(
            err("M 1 2 L 3 x"),
            "expected a number at character 11 of the path"
        );
        assert_eq!(
            err("M0 0 A1 1 0 2 0 1 1"),
            "expected an arc flag (0 or 1) at character 13 of the path"
        );
        assert_eq!(err("M 1 2"), "the path draws nothing");
        assert!(err("M0 0 Z 1 2").starts_with("expected a path command"));
        let huge = format!("M0 0{}", " C1 1 2 2 3 3".repeat(500));
        assert!(err(&huge).starts_with("the path is too detailed"));
    }
}
