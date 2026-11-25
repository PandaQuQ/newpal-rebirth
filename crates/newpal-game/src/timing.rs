//! Frame pacing: fixed-timestep logic (original tick) decoupled from rendering, plus interpolation helpers.
//!
//! The original main loop sleeps to 35 ms per frame (sleep argument 0x23; 70 ms in some cutscenes),
//! i.e. ~28.6 logic frames/s, and ACT `DS` delays count those frames. We keep that logic rate (so
//! timings match the original) and render at the display rate, blending the last two logic states
//! with `alpha` in [0, 1).
use std::time::{Duration, Instant};

/// Original frame time (seconds).
pub const ORIGINAL_TICK: f32 = 0.035;

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Accumulator for a fixed logic step.
pub struct FixedStep {
    pub step: f32,
    acc: f32,
    /// cap on catch-up steps per frame (avoids a spiral of death after a stall)
    pub max_steps: u32,
}

impl FixedStep {
    pub fn new(step: f32) -> Self {
        Self { step: step.max(0.001), acc: 0.0, max_steps: 8 }
    }
    pub fn from_hz(hz: Option<f32>) -> Self {
        Self::new(hz.filter(|h| *h > 0.0).map(|h| 1.0 / h).unwrap_or(ORIGINAL_TICK))
    }
    /// Add frame time; returns how many logic steps to run now.
    pub fn advance(&mut self, dt: f32) -> u32 {
        self.acc += dt.max(0.0);
        let mut n = 0;
        while self.acc >= self.step && n < self.max_steps {
            self.acc -= self.step;
            n += 1;
        }
        if n == self.max_steps {
            self.acc = self.acc.min(self.step);
        }
        n
    }
    /// Blend factor between the previous and the current logic state.
    pub fn alpha(&self) -> f32 {
        (self.acc / self.step).clamp(0.0, 1.0)
    }
}

/// Optional frame cap (0 = none) and a smoothed FPS counter.
pub struct Pacer {
    pub cap: u32,
    next: Instant,
    pub fps: f32,
}

impl Pacer {
    pub fn new(cap: u32) -> Self {
        Self { cap, next: Instant::now(), fps: 0.0 }
    }
    /// Deadline for the next frame (None = render as fast as vsync / the GPU allows).
    pub fn deadline(&mut self, now: Instant) -> Option<Instant> {
        if self.cap == 0 {
            return None;
        }
        let period = Duration::from_secs_f64(1.0 / self.cap as f64);
        self.next = (self.next + period).max(now.checked_sub(period).unwrap_or(now));
        Some(self.next)
    }
    pub fn frame(&mut self, dt: f32) {
        if dt > 0.0 {
            let f = 1.0 / dt;
            self.fps = if self.fps == 0.0 { f } else { self.fps * 0.95 + f * 0.05 };
        }
    }
}

/// Frame caps cycled by the hotkey (0 = uncapped / vsync).
pub const CAPS: [u32; 5] = [0, 30, 60, 144, 240];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_step_counts_and_alpha() {
        let mut s = FixedStep::new(0.035);
        // 144 Hz rendering: most frames run 0 logic steps, alpha grows smoothly
        let mut steps = 0;
        let mut alphas = Vec::new();
        for _ in 0..144 {
            steps += s.advance(1.0 / 144.0);
            alphas.push(s.alpha());
        }
        assert!((steps as i32 - 28).abs() <= 1, "{steps}");
        assert!(alphas.iter().all(|a| (0.0..=1.0).contains(a)));
        // between two steps alpha is increasing
        let mut s = FixedStep::new(0.035);
        s.advance(0.010);
        let a1 = s.alpha();
        s.advance(0.010);
        assert!(s.alpha() > a1);
        // a stall does not explode
        let mut s = FixedStep::new(0.035);
        assert_eq!(s.advance(5.0), 8);
        assert!(s.alpha() <= 1.0);
        assert_eq!(FixedStep::from_hz(None).step, ORIGINAL_TICK);
        assert!((FixedStep::from_hz(Some(60.0)).step - 1.0 / 60.0).abs() < 1e-6);
    }

    #[test]
    fn pacer_cap() {
        let mut p = Pacer::new(0);
        assert!(p.deadline(Instant::now()).is_none());
        let mut p = Pacer::new(100);
        let now = Instant::now();
        let d = p.deadline(now).unwrap();
        assert!(d <= now + Duration::from_millis(11));
        p.frame(0.01);
        assert!((p.fps - 100.0).abs() < 1.0);
    }
}