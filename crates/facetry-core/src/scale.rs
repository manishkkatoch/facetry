//! Scales map data values to pixels (or colours). Each axis owns one, which is what makes
//! several y-axes with independent domains cheap.
//!
//! All scale maths is in `f64`; layout converts to `f32` pixels at the edge.

use std::collections::HashMap;

use crate::Color;

/// d3's "1-2-5" thresholds: the geometric midpoints between 1, 2, 5 and 10.
const E10: f64 = 7.071_067_811_865_475_5; // sqrt(50)
const E5: f64 = 3.162_277_660_168_379_5; // sqrt(10)
const E2: f64 = std::f64::consts::SQRT_2;

/// The tick step for a domain, as a signed "increment" (d3's trick to dodge float noise):
/// positive `n` means the step is `n` (1, 2, 5, 10, 20, ...); negative `-n` means the step
/// is `1 / n` (so 0.1 is `-10`). Ticks are then `i * n` or `i / n` for integer `i`, which,
/// unlike repeated `+= 0.1`, never accumulates error: `3 / 10` is exactly the double nearest 0.3.
fn tick_increment(start: f64, stop: f64, count: usize) -> f64 {
    let step = (stop - start) / count.max(1) as f64;
    if !step.is_finite() || step <= 0.0 {
        return 0.0;
    }
    let power = step.log10().floor();
    let error = step / 10f64.powf(power);
    let factor = if error >= E10 {
        10.0
    } else if error >= E5 {
        5.0
    } else if error >= E2 {
        2.0
    } else {
        1.0
    };
    if power >= 0.0 {
        factor * 10f64.powf(power)
    } else {
        -(10f64.powf(-power)) / factor
    }
}

/// Tick positions plus how many decimals the labels need.
#[derive(Debug, Clone, PartialEq)]
pub struct Ticks {
    pub values: Vec<f64>,
    /// Decimal places shared by every label of this axis (`0.5` steps -> 1).
    pub decimals: usize,
}

impl Ticks {
    /// The labels, formatted with [`format_number`].
    pub fn labels(&self) -> Vec<String> {
        self.values
            .iter()
            .map(|&v| format_number(v, self.decimals, true))
            .collect()
    }
}

impl Ticks {
    /// The labels as percentages of a `0..=1` axis: `0.25` reads `25%`. The decimals follow
    /// the step (two fewer than the fraction needs), so `0.005` steps read `0.5%`.
    pub fn labels_percent(&self) -> Vec<String> {
        let decimals = self.decimals.saturating_sub(2);
        self.values
            .iter()
            .map(|&v| format!("{}%", format_number(v * 100.0, decimals, true)))
            .collect()
    }
}

/// Decimals needed to print multiples of the step encoded by `increment`.
fn decimals_for(increment: f64) -> usize {
    if increment >= 0.0 {
        return 0;
    }
    // Step is 1/n for a whole n (1, 2, 5, 10, 20, ...): the smallest d with n | 10^d.
    let n = (-increment).round() as u64;
    let mut pow = 1u64;
    for d in 0..=15 {
        if n != 0 && pow.is_multiple_of(n) {
            return d;
        }
        pow = pow.saturating_mul(10);
    }
    15
}

/// Formats `v` with a fixed number of decimals, optionally with `,` thousands separators.
///
/// Chosen format (documented here because it is a product decision): `1,200`, `0.25`,
/// `-1,500.5`. No `k`/`M` suffixes: they lose precision and need locale rules. Negative zero
/// and values that round to zero print as `0`.
pub fn format_number(v: f64, decimals: usize, thousands: bool) -> String {
    let mut s = format!("{v:.decimals$}");
    if s.parse::<f64>() == Ok(0.0) {
        s = format!("{:.decimals$}", 0.0);
    }
    if !thousands {
        return s;
    }
    let (sign, rest) = s.strip_prefix('-').map_or(("", s.as_str()), |r| ("-", r));
    let (int, frac) = rest.split_once('.').map_or((rest, ""), |(i, f)| (i, f));
    // Only plain digits get separators ("inf"/"NaN" pass through untouched).
    if !int.bytes().all(|b| b.is_ascii_digit()) {
        return s;
    }
    let mut out = String::from(sign);
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    out
}

/// Text for a data value on a bar label: the axis number style (`1,200`) with `decimals`
/// digits, or by default as few as the value needs (up to 4: `1200` is `1,200`, `0.25` is
/// `0.25`, `2.5` is `2.5`).
pub fn format_value(v: f64, decimals: Option<u32>) -> String {
    let decimals = decimals.map_or_else(
        || {
            (0..4)
                .find(|&d| {
                    let p = 10f64.powi(d);
                    ((v * p).round() / p - v).abs() <= 1e-9 * v.abs().max(1.0)
                })
                .unwrap_or(4) as usize
        },
        |d| d as usize,
    );
    format_number(v, decimals, true)
}

/// A share (`0.25`) as a percentage label (`25%`). By default one decimal only when the
/// value needs it (`33.3%`, `50%`); `decimals` fixes the count.
pub fn format_percent(share: f64, decimals: Option<u32>) -> String {
    let pct = share * 100.0;
    let decimals = decimals.map_or_else(
        || usize::from(((pct * 10.0).round() / 10.0 - pct.round()).abs() > 1e-9),
        |d| d as usize,
    );
    format!("{}%", format_number(pct, decimals, true))
}

/// A continuous numeric scale: `domain` (data) to `range` (pixels), linearly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Linear {
    domain: (f64, f64),
    range: (f64, f64),
}

impl Linear {
    pub fn new(domain: (f64, f64), range: (f64, f64)) -> Self {
        Self { domain, range }
    }

    pub fn domain(&self) -> (f64, f64) {
        self.domain
    }

    pub fn range(&self) -> (f64, f64) {
        self.range
    }

    pub fn set_range(&mut self, range: (f64, f64)) {
        self.range = range;
    }

    /// Maps a data value to the range. Values outside the domain extrapolate.
    pub fn map(&self, v: f64) -> f64 {
        let (d0, d1) = self.domain;
        if d1 == d0 {
            return self.range.0;
        }
        self.range.0 + (v - d0) / (d1 - d0) * (self.range.1 - self.range.0)
    }

    /// Rounds the domain out so it starts and ends on tick values for about `count` ticks.
    /// Iterates because rounding can change which step is the nice one (a port of d3's `nice`).
    /// Use `count >= 2`; for 1 the iteration may not settle (it stops after 10 rounds).
    pub fn nice(&mut self, count: usize) {
        let (mut start, mut stop) = self.domain;
        if !(start.is_finite() && stop.is_finite()) || start >= stop {
            return;
        }
        let mut prev = f64::NAN;
        for _ in 0..10 {
            let inc = tick_increment(start, stop, count);
            if inc == prev {
                break;
            }
            if inc > 0.0 {
                start = (start / inc).floor() * inc;
                stop = (stop / inc).ceil() * inc;
            } else if inc < 0.0 {
                // Step is 1/n: round out to multiples of 1/n.
                let n = -inc;
                start = (start * n).floor() / n;
                stop = (stop * n).ceil() / n;
            } else {
                break;
            }
            prev = inc;
        }
        self.domain = (start, stop);
    }

    /// About `count` tick values inside the domain, on 1-2-5 steps, ascending.
    pub fn ticks(&self, count: usize) -> Ticks {
        let (start, stop) = self.domain;
        let none = Ticks {
            values: Vec::new(),
            decimals: 0,
        };
        if !(start.is_finite() && stop.is_finite()) || count == 0 || start > stop {
            return none;
        }
        if start == stop {
            return Ticks {
                values: vec![start],
                decimals: 0,
            };
        }
        let inc = tick_increment(start, stop, count);
        if inc == 0.0 {
            return none;
        }
        // Integer multiples of the step that fall inside [start, stop].
        let (scale, invert) = if inc > 0.0 {
            (inc, false)
        } else {
            (-inc, true)
        };
        let at = |i: f64| if invert { i / scale } else { i * scale };
        let mut first = if invert { start * scale } else { start / scale }.round();
        let mut last = if invert { stop * scale } else { stop / scale }.round();
        if at(first) < start {
            first += 1.0;
        }
        if at(last) > stop {
            last -= 1.0;
        }
        let values = (0..)
            .map(|k| first + f64::from(k))
            .take_while(|&i| i <= last)
            .map(at)
            .collect();
        Ticks {
            values,
            decimals: decimals_for(inc),
        }
    }
}

/// Evenly spaced bands for categories, with padding (d3's `scaleBand`).
///
/// With `n` bands over a range of length `L`: `step = L / (n - inner + 2 * outer)`, a band is
/// `step * (1 - inner)` wide, and the leftover is split evenly so the bands are centred.
#[derive(Debug, Clone, PartialEq)]
pub struct Band {
    domain: Vec<String>,
    index: HashMap<String, usize>,
    start: f64,
    step: f64,
    bandwidth: f64,
    first: f64,
}

impl Band {
    /// `padding_inner`/`padding_outer` are fractions of a step in `0..=1`.
    pub fn new(
        domain: Vec<String>,
        range: (f64, f64),
        padding_inner: f64,
        padding_outer: f64,
    ) -> Self {
        let n = domain.len() as f64;
        let inner = padding_inner.clamp(0.0, 1.0);
        let outer = padding_outer.clamp(0.0, 1.0);
        let len = range.1 - range.0;
        let step = len / (n - inner + 2.0 * outer).max(1.0);
        let bandwidth = step * (1.0 - inner);
        let first = range.0 + (len - step * (n - inner)) * 0.5;
        let index = domain
            .iter()
            .enumerate()
            .map(|(i, c)| (c.clone(), i))
            .collect();
        Self {
            domain,
            index,
            start: range.0,
            step,
            bandwidth,
            first,
        }
    }

    pub fn domain(&self) -> &[String] {
        &self.domain
    }

    /// Distance between the starts of neighbouring bands.
    pub fn step(&self) -> f64 {
        self.step
    }

    pub fn bandwidth(&self) -> f64 {
        self.bandwidth
    }

    pub fn index_of(&self, category: &str) -> Option<usize> {
        self.index.get(category).copied()
    }

    /// Left edge of band `i`.
    pub fn position(&self, i: usize) -> f64 {
        self.first + self.step * i as f64
    }

    pub fn center(&self, i: usize) -> f64 {
        self.position(i) + self.bandwidth / 2.0
    }

    /// The range's start, for callers that need the plot edge.
    pub fn range_start(&self) -> f64 {
        self.start
    }
}

/// Categories to colours: the n-th distinct category gets the n-th colour, cycling.
#[derive(Debug, Clone, PartialEq)]
pub struct Ordinal {
    index: HashMap<String, usize>,
    range: Vec<Color>,
}

impl Ordinal {
    /// An empty `range` falls back to opaque black rather than panicking on a modulo by zero.
    pub fn new(domain: &[String], range: Vec<Color>) -> Self {
        let index = domain
            .iter()
            .enumerate()
            .map(|(i, c)| (c.clone(), i))
            .collect();
        Self { index, range }
    }

    pub fn color(&self, category: &str) -> Color {
        let i = self.index.get(category).copied().unwrap_or(0);
        if self.range.is_empty() {
            return Color::linear(0.0, 0.0, 0.0, 1.0);
        }
        self.range[i % self.range.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lin(d0: f64, d1: f64) -> Linear {
        Linear::new((d0, d1), (0.0, 100.0))
    }

    fn labels(d0: f64, d1: f64, count: usize) -> Vec<String> {
        lin(d0, d1).ticks(count).labels()
    }

    #[test]
    fn value_labels_use_the_axis_style_with_natural_or_fixed_decimals() {
        assert_eq!(format_value(1200.0, None), "1,200");
        assert_eq!(format_value(0.25, None), "0.25");
        assert_eq!(format_value(-2.5, None), "-2.5");
        assert_eq!(format_value(0.1 + 0.2, None), "0.3");
        assert_eq!(format_value(1.23456789, None), "1.2346");
        assert_eq!(format_value(1200.0, Some(2)), "1,200.00");
        assert_eq!(format_value(2.567, Some(1)), "2.6");
        assert_eq!(format_value(7.0, Some(0)), "7");
    }

    #[test]
    fn linear_maps_and_inverts_ranges() {
        let s = Linear::new((0.0, 10.0), (100.0, 0.0));
        assert_eq!(s.map(0.0), 100.0);
        assert_eq!(s.map(10.0), 0.0);
        assert_eq!(s.map(2.5), 75.0);
    }

    #[test]
    fn degenerate_domain_maps_to_range_start() {
        assert_eq!(Linear::new((3.0, 3.0), (7.0, 9.0)).map(3.0), 7.0);
    }

    #[test]
    fn ticks_follow_the_1_2_5_rule() {
        assert_eq!(labels(0.0, 10.0, 5), ["0", "2", "4", "6", "8", "10"]);
        assert_eq!(labels(0.0, 100.0, 5), ["0", "20", "40", "60", "80", "100"]);
        assert_eq!(labels(0.0, 1.0, 2), ["0.0", "0.5", "1.0"]);
        assert_eq!(
            labels(0.0, 140.0, 7),
            ["0", "20", "40", "60", "80", "100", "120", "140"]
        );
    }

    #[test]
    fn ticks_have_no_float_noise() {
        // 0.1 + 0.2 style accumulation would give 0.30000000000000004.
        assert_eq!(
            labels(0.0, 0.5, 5),
            ["0.0", "0.1", "0.2", "0.3", "0.4", "0.5"]
        );
        let t = lin(0.0, 0.5).ticks(5);
        assert_eq!(t.values[3], 0.3);
    }

    #[test]
    fn thousands_separators() {
        assert_eq!(labels(0.0, 1500.0, 3), ["0", "500", "1,000", "1,500"]);
        assert_eq!(format_number(1_234_567.0, 0, true), "1,234,567");
        assert_eq!(format_number(-1234.5, 1, true), "-1,234.5");
        assert_eq!(format_number(999.0, 0, true), "999");
        assert_eq!(format_number(1200.0, 0, false), "1200");
    }

    #[test]
    fn negative_zero_prints_as_zero() {
        assert_eq!(format_number(-0.0, 0, true), "0");
        assert_eq!(format_number(-0.001, 1, true), "0.0");
        assert_eq!(labels(-100.0, 100.0, 4), ["-100", "-50", "0", "50", "100"]);
    }

    #[test]
    fn non_finite_values_pass_through() {
        assert_eq!(format_number(f64::INFINITY, 0, true), "inf");
        assert_eq!(format_number(f64::NAN, 2, true), "NaN");
    }

    #[test]
    fn nice_rounds_the_domain_out() {
        let mut s = lin(0.0, 137.0);
        s.nice(8);
        assert_eq!(s.domain(), (0.0, 140.0));
        let mut s = lin(-3.2, 96.4);
        s.nice(5);
        assert_eq!(s.domain(), (-20.0, 100.0));
        let mut s = lin(0.013, 0.97);
        s.nice(5);
        assert_eq!(s.domain(), (0.0, 1.0));
    }

    #[test]
    fn nice_leaves_degenerate_and_non_finite_domains_alone() {
        for (a, b) in [(1.0, 1.0), (f64::NAN, 1.0), (0.0, f64::INFINITY)] {
            let mut s = lin(a, b);
            s.nice(5);
            let (x, y) = s.domain();
            assert!(x.to_bits() == a.to_bits() && y.to_bits() == b.to_bits());
        }
    }

    #[test]
    fn single_value_domain_has_one_tick() {
        assert_eq!(labels(5.0, 5.0, 5), ["5"]);
    }

    #[test]
    fn zero_count_or_bad_domain_has_no_ticks() {
        assert!(lin(0.0, 1.0).ticks(0).values.is_empty());
        assert!(lin(f64::NAN, 1.0).ticks(5).values.is_empty());
        assert!(lin(2.0, 1.0).ticks(5).values.is_empty());
    }

    #[test]
    fn tiny_and_huge_steps_format_without_exponents() {
        assert_eq!(labels(0.0, 0.0005, 5).last().unwrap(), "0.0005");
        assert_eq!(labels(0.0, 5e9, 5).last().unwrap(), "5,000,000,000");
    }

    #[test]
    fn band_matches_d3_for_the_documented_example() {
        // 4 bands over 0..400 with inner 0.2, outer 0.1: step = 400 / (4 - 0.2 + 0.2) = 100.
        let b = Band::new(
            ["a", "b", "c", "d"].map(String::from).to_vec(),
            (0.0, 400.0),
            0.2,
            0.1,
        );
        assert!((b.step() - 100.0).abs() < 1e-9);
        assert!((b.bandwidth() - 80.0).abs() < 1e-9);
        assert!((b.position(0) - 10.0).abs() < 1e-9);
        assert!((b.position(3) - 310.0).abs() < 1e-9);
        assert!((b.center(1) - 150.0).abs() < 1e-9);
        assert_eq!(b.index_of("c"), Some(2));
        assert_eq!(b.index_of("z"), None);
    }

    #[test]
    fn bands_are_centred_in_the_range() {
        let b = Band::new(vec!["a".into(), "b".into()], (10.0, 210.0), 0.5, 0.0);
        let left = b.position(0) - 10.0;
        let right = 210.0 - (b.position(1) + b.bandwidth());
        assert!((left - right).abs() < 1e-9);
    }

    #[test]
    fn empty_band_domain_does_not_divide_by_zero() {
        let b = Band::new(vec![], (0.0, 100.0), 0.2, 0.1);
        assert!(b.step().is_finite() && b.bandwidth().is_finite());
    }

    #[test]
    fn ordinal_cycles_through_its_range() {
        let range = vec![
            Color::linear(1.0, 0.0, 0.0, 1.0),
            Color::linear(0.0, 1.0, 0.0, 1.0),
        ];
        let domain: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let s = Ordinal::new(&domain, range.clone());
        assert_eq!(s.color("a"), range[0]);
        assert_eq!(s.color("b"), range[1]);
        assert_eq!(s.color("c"), range[0]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod properties {
        use super::*;
        use proptest::prelude::*;

        // `count` starts at 2: with a request for a single tick, d3's `nice` can bounce between
        // two domains that straddle zero (layout always asks for 2 or more).

        /// A finite domain with a span that is neither absurdly small nor huge relative to
        /// its position, so `f64` has the precision the properties assume.
        fn domain() -> impl Strategy<Value = (f64, f64)> {
            (-1e6f64..1e6, 1e-3f64..1e6).prop_map(|(start, span)| (start, start + span))
        }

        proptest! {
            #[test]
            fn ticks_are_ascending_and_inside_the_domain((a, b) in domain(), count in 2usize..=20) {
                let t = lin(a, b).ticks(count);
                let eps = (b - a) * 1e-9;
                for w in t.values.windows(2) {
                    prop_assert!(w[0] < w[1], "{:?}", t.values);
                }
                for &v in &t.values {
                    prop_assert!(v >= a - eps && v <= b + eps, "{v} outside [{a}, {b}]");
                }
            }

            #[test]
            fn nice_domain_contains_the_original_and_ends_on_ticks((a, b) in domain(), count in 2usize..=20) {
                let mut s = lin(a, b);
                s.nice(count);
                let (n0, n1) = s.domain();
                let eps = (b - a) * 1e-9;
                prop_assert!(n0 <= a + eps && n1 >= b - eps, "[{n0}, {n1}] does not cover [{a}, {b}]");
                let t = s.ticks(count);
                prop_assert!(t.values.len() >= 2, "{:?}", t.values);
                prop_assert!((t.values[0] - n0).abs() <= eps.max(1e-9 * n0.abs()), "{:?} vs {n0}", t.values);
                prop_assert!((t.values[t.values.len() - 1] - n1).abs() <= eps.max(1e-9 * n1.abs()));
            }

            #[test]
            fn tick_count_is_in_the_ballpark_of_the_request((a, b) in domain(), count in 2usize..=20) {
                // The 1-2-5 rule can be off by up to a factor of ~2.5 either way (and rounding
                // the domain out adds up to two steps).
                let n = lin(a, b).ticks(count).values.len();
                prop_assert!(n <= 3 * count + 2, "{n} ticks for a request of {count}");
                let mut s = lin(a, b);
                s.nice(count);
                let n = s.ticks(count).values.len();
                prop_assert!(n >= 2 && n <= 3 * count + 2, "{n} ticks for a request of {count}");
            }

            #[test]
            fn nice_is_idempotent((a, b) in domain(), count in 2usize..=20) {
                let mut once = lin(a, b);
                once.nice(count);
                let mut twice = once;
                twice.nice(count);
                prop_assert_eq!(once.domain(), twice.domain());
            }

            #[test]
            fn labels_are_clean_and_round_trip((a, b) in domain(), count in 2usize..=20) {
                let t = lin(a, b).ticks(count);
                for (&v, label) in t.values.iter().zip(t.labels()) {
                    prop_assert!(!label.contains('e') && !label.contains("NaN"), "{label}");
                    let digits = label.split_once('.').map_or(0, |(_, f)| f.len());
                    prop_assert_eq!(digits, t.decimals);
                    let parsed: f64 = label.replace(',', "").parse().unwrap();
                    let half_unit = 0.5 * 10f64.powi(-(t.decimals as i32));
                    prop_assert!((parsed - v).abs() <= half_unit * 1.000_001, "{label} vs {v}");
                }
            }

            #[test]
            fn linear_map_is_monotonic((a, b) in domain(), x in 0.0f64..1.0, y in 0.0f64..1.0) {
                let s = Linear::new((a, b), (0.0, 500.0));
                let (lo, hi) = if x <= y { (x, y) } else { (y, x) };
                prop_assert!(s.map(a + lo * (b - a)) <= s.map(a + hi * (b - a)));
            }
        }
    }
}
