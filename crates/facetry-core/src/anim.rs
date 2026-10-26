//! The arithmetic of animation: easing curves, timing, and interpolation of the values a scene
//! is made of. Pure functions, no clock and no GPU; the caller says how many milliseconds have
//! passed.
//!
//! A transition has one **elapsed time**. Each mark has its own start `delay` (the `stagger`),
//! so each one's progress is `clamp((elapsed - delay) / duration, 0, 1)` run through the easing
//! curve. The GPU shaders compute exactly this for the marks they move (see
//! `facetry-render/src/shaders/ease.wgsl`); the CPU does it for everything it tessellates
//! itself, and to find where a mark is when a new update interrupts a running transition.

use crate::Color;
use crate::scene::Bounds;
use crate::spec::{Easing, Timing};

/// `a` at `t = 0`, `b` at `t = 1`, exactly (the form `a(1-t) + bt` has no rounding error at
/// the ends, unlike `a + (b-a)t`). `t` may leave `0..=1` (an overshooting easing does).
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

/// Linear interpolation of the four channels, which stay in `0..=1` even if an overshooting
/// easing pushes `t` past 1.
pub fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let ch = |a: f32, b: f32| lerp(a, b, t).clamp(0.0, 1.0);
    Color::linear(ch(a.r, b.r), ch(a.g, b.g), ch(a.b, b.b), ch(a.a, b.a))
}

pub fn lerp_bounds(a: Bounds, b: Bounds, t: f32) -> Bounds {
    Bounds {
        x: lerp(a.x, b.x, t),
        y: lerp(a.y, b.y, t),
        w: lerp(a.w, b.w, t).max(0.0),
        h: lerp(a.h, b.h, t).max(0.0),
    }
}

/// Interpolates between two angles (radians) along the way a straight line on the number
/// line goes: slices are described by absolute start and end angles that already add up around
/// the circle, so there is no "shortest way round" to take here (the camera's azimuth has
/// that, see [`lerp_angle_deg`]).
pub fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    lerp(a, b, t)
}

/// Interpolates degrees by the shortest way round the circle, so 350 to 10 passes through 0
/// instead of sweeping back through 180. The result is `a + delta * t` with `delta` in
/// `-180..=180`, and is not wrapped back into `0..360`.
pub fn lerp_angle_deg(a: f32, b: f32, t: f32) -> f32 {
    let delta = (b - a + 180.0).rem_euclid(360.0) - 180.0;
    // At the end the target itself, not an equal angle a turn away.
    if t >= 1.0 { b } else { a + delta * t }
}

impl Easing {
    /// Maps linear progress `t` in `0..=1` to eased progress: 0 at 0 and 1 at 1 for every
    /// curve (`backOut` overshoots 1 in between).
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::CubicInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    let u = -2.0 * t + 2.0;
                    1.0 - u * u * u / 2.0
                }
            }
            Self::CubicOut => {
                let u = 1.0 - t;
                1.0 - u * u * u
            }
            Self::BackOut => {
                const OVERSHOOT: f32 = 1.701_58;
                let u = t - 1.0;
                1.0 + (OVERSHOOT + 1.0) * u * u * u + OVERSHOOT * u * u
            }
        }
    }

    /// The number the shaders switch on; keep in step with `ease.wgsl`.
    pub fn id(self) -> u32 {
        match self {
            Self::Linear => 0,
            Self::CubicInOut => 1,
            Self::CubicOut => 2,
            Self::BackOut => 3,
        }
    }
}

impl Timing {
    /// Eased progress of a mark that starts `delay_ms` after the transition does, `elapsed_ms`
    /// into it.
    pub fn eased(&self, elapsed_ms: f32, delay_ms: f32) -> f32 {
        let p = (elapsed_ms - delay_ms) / self.duration_ms.max(f32::MIN_POSITIVE);
        self.easing.apply(p)
    }

    /// How long the whole transition takes when the last mark starts `max_delay_ms` late.
    pub fn total_ms(&self, max_delay_ms: f32) -> f32 {
        self.duration_ms + max_delay_ms
    }

    /// The delay of mark `index` of `count` when `stagger` is spread over them: the first
    /// starts at once, the last `stagger_ms` late.
    pub fn delay_of(&self, index: usize, count: usize) -> f32 {
        if count < 2 {
            0.0
        } else {
            self.stagger_ms * index as f32 / (count - 1) as f32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Easing; 4] = [
        Easing::Linear,
        Easing::CubicInOut,
        Easing::CubicOut,
        Easing::BackOut,
    ];

    #[test]
    fn every_easing_starts_at_zero_and_ends_at_one() {
        for e in ALL {
            assert_eq!(e.apply(0.0), 0.0, "{e:?}");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-6, "{e:?}");
        }
    }

    #[test]
    fn easings_have_their_known_midpoints() {
        assert_eq!(Easing::Linear.apply(0.5), 0.5);
        assert!((Easing::CubicInOut.apply(0.5) - 0.5).abs() < 1e-6);
        assert!((Easing::CubicOut.apply(0.5) - 0.875).abs() < 1e-6);
        // d3's easeBackOut(0.5) with the default overshoot.
        assert!((Easing::BackOut.apply(0.5) - 1.087_698_7).abs() < 1e-4);
    }

    #[test]
    fn input_outside_the_unit_interval_is_clamped() {
        for e in ALL {
            assert_eq!(e.apply(-3.0), e.apply(0.0));
            assert!((e.apply(7.0) - e.apply(1.0)).abs() < 1e-6);
        }
    }

    #[test]
    fn all_but_back_out_are_monotone_and_back_out_overshoots_then_settles() {
        for e in [Easing::Linear, Easing::CubicInOut, Easing::CubicOut] {
            let mut last = 0.0;
            for i in 0..=100 {
                let v = e.apply(i as f32 / 100.0);
                assert!(v >= last - 1e-6 && v <= 1.0 + 1e-6, "{e:?} at {i}");
                last = v;
            }
        }
        let peak = (0..=100)
            .map(|i| Easing::BackOut.apply(i as f32 / 100.0))
            .fold(0.0, f32::max);
        assert!(peak > 1.05 && peak < 1.15, "{peak}");
    }

    #[test]
    fn lerp_is_exact_at_the_ends() {
        for (a, b) in [(0.1, 0.7), (-3.3, 1e6), (123.456, 123.456)] {
            assert_eq!(lerp(a, b, 0.0), a);
            assert_eq!(lerp(a, b, 1.0), b);
        }
    }

    #[test]
    fn colours_stay_in_range_when_t_overshoots() {
        let a = Color::linear(0.0, 0.5, 1.0, 0.0);
        let b = Color::linear(1.0, 0.5, 0.2, 1.0);
        let c = lerp_color(a, b, 1.1);
        assert!(c.r <= 1.0 && c.a <= 1.0 && c.b >= 0.0);
        assert_eq!(lerp_color(a, b, 0.0), a);
        assert_eq!(lerp_color(a, b, 1.0), b);
    }

    #[test]
    fn degrees_take_the_shortest_way_round() {
        assert!((lerp_angle_deg(350.0, 10.0, 0.5) - 360.0).abs() < 1e-4);
        assert!((lerp_angle_deg(10.0, 350.0, 0.5) - 0.0).abs() < 1e-4);
        assert!((lerp_angle_deg(0.0, 90.0, 0.5) - 45.0).abs() < 1e-4);
        assert_eq!(lerp_angle_deg(350.0, 10.0, 1.0), 10.0);
        assert_eq!(lerp_angle_deg(350.0, 10.0, 0.0), 350.0);
        // Exactly opposite: either way is as short, but it must be consistent.
        assert!((lerp_angle_deg(0.0, 180.0, 0.5).abs() - 90.0).abs() < 1e-3);
    }

    #[test]
    fn stagger_spreads_the_start_over_the_marks() {
        let t = Timing {
            duration_ms: 400.0,
            easing: Easing::Linear,
            stagger_ms: 200.0,
        };
        assert_eq!(t.delay_of(0, 5), 0.0);
        assert_eq!(t.delay_of(4, 5), 200.0);
        assert_eq!(t.delay_of(2, 5), 100.0);
        assert_eq!(t.delay_of(0, 1), 0.0);
        assert_eq!(t.total_ms(200.0), 600.0);
        // The last mark has not started at 100 ms, the first is a quarter of the way.
        assert_eq!(t.eased(100.0, 200.0), 0.0);
        assert_eq!(t.eased(100.0, 0.0), 0.25);
        assert_eq!(t.eased(600.0, 200.0), 1.0);
    }
}
