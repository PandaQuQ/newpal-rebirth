//! Frame-rate independent animation helpers (work the same at 60, 165 or 300 Hz).

/// Exponential approach: moves `cur` toward `target`; `rate` = 1/seconds (higher = snappier).
pub fn approach(cur: &mut f32, target: f32, rate: f32, dt: f32) {
    let k = 1.0 - (-rate * dt).exp();
    *cur += (target - *cur) * k;
    if (*cur - target).abs() < 1e-4 {
        *cur = target;
    }
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Deterministic hash noise in [0, 1) (particles, brush jitter) - no RNG state, same every run.
pub fn hash(i: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approach_is_rate_independent() {
        let (mut a, mut b) = (0.0f32, 0.0f32);
        for _ in 0..60 { approach(&mut a, 1.0, 8.0, 1.0 / 60.0); }
        for _ in 0..300 { approach(&mut b, 1.0, 8.0, 1.0 / 300.0); }
        assert!((a - b).abs() < 1e-3, "{a} vs {b}");
    }
    #[test]
    fn hash_range() {
        for i in 0..1000 { let h = hash(i); assert!((0.0..1.0).contains(&h)); }
    }
}
