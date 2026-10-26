// Animation clock, shared by the shaders that move marks (prepended to them, see
// `gpu::with_ease`). Needs `globals.anim` = (elapsed ms, duration ms, easing id, spare).
// The curves match `Easing::apply` in facetry-core/src/anim.rs; keep them in step.

fn ease(progress: f32, id: f32) -> f32 {
    let t = clamp(progress, 0.0, 1.0);
    if (id < 0.5) {
        return t;
    }
    if (id < 1.5) {
        // cubicInOut
        if (t < 0.5) {
            return 4.0 * t * t * t;
        }
        let u = -2.0 * t + 2.0;
        return 1.0 - u * u * u / 2.0;
    }
    if (id < 2.5) {
        // cubicOut
        let u = 1.0 - t;
        return 1.0 - u * u * u;
    }
    // backOut: 1.70158 is the classic overshoot.
    let u = t - 1.0;
    return 1.0 + 2.70158 * u * u * u + 1.70158 * u * u;
}

// Eased progress of a mark that starts `delay` milliseconds after the transition does. When
// nothing is animating the clock reads (1, 1): progress 1 exactly, so `mix_to(a, b, t)` is `b`.
fn anim_t(delay: f32) -> f32 {
    return ease((globals.anim.x - delay) / max(globals.anim.y, 0.000001), globals.anim.z);
}
