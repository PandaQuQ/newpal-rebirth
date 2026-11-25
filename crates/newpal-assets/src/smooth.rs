//! Smooth-animation mods: extra in-between frames for an ACT action.
//!
//! Logical name: `act/<act>/<slot>/<action>/anim.toml` (slot = 0-based entry of `<act>.act`, e.g.
//! `act/all_char/0/8/anim.toml` = hero walk north). Frames are PNGs next to the manifest, drawn with
//! the same bottom-centre anchor as the original frames.
//!
//! ```toml
//! count = 24            # frames f000.png .. f023.png   (or: frames = ["a.png", "b.png"])
//! fps = 60              # optional: playback rate; default = spread over the original loop length
//! scale = 2             # optional: pixel density (HD); logical size = png size / scale
//! durations_ms = [..]   # optional: explicit per-frame durations (overrides fps)
//! ```
use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct AnimManifest {
    #[serde(default)]
    pub count: Option<usize>,
    #[serde(default)]
    pub frames: Vec<String>,
    #[serde(default)]
    pub fps: Option<f32>,
    #[serde(default)]
    pub scale: Option<f32>,
    #[serde(default)]
    pub durations_ms: Vec<f32>,
}

impl AnimManifest {
    pub fn parse(text: &str) -> crate::Result<Self> {
        toml::from_str(text).map_err(|e| crate::Error::corrupt(format!("anim.toml: {e}")))
    }

    /// Frame file names in play order.
    pub fn files(&self) -> Vec<String> {
        if !self.frames.is_empty() {
            return self.frames.clone();
        }
        (0..self.count.unwrap_or(0)).map(|i| format!("f{i:03}.png")).collect()
    }

    /// Per-frame durations (seconds) for `n` frames; `orig_total` = original loop length (seconds).
    pub fn durations(&self, n: usize, orig_total: f32) -> Vec<f32> {
        if n == 0 {
            return Vec::new();
        }
        if self.durations_ms.len() == n {
            return self.durations_ms.iter().map(|d| d.max(1.0) / 1000.0).collect();
        }
        let each = match self.fps {
            Some(f) if f > 0.0 => 1.0 / f,
            _ if orig_total > 0.0 => orig_total / n as f32,
            _ => 1.0 / 30.0,
        };
        vec![each; n]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_defaults_and_timing() {
        let m = AnimManifest::parse("count = 4").unwrap();
        assert_eq!(m.files(), vec!["f000.png", "f001.png", "f002.png", "f003.png"]);
        // spread over the original loop (0.28 s)
        assert!(m.durations(4, 0.28).iter().all(|d| (d - 0.07).abs() < 1e-6));
        let m = AnimManifest::parse("frames = [\"a.png\", \"b.png\"]\nfps = 50\nscale = 2").unwrap();
        assert_eq!(m.files().len(), 2);
        assert_eq!(m.durations(2, 1.0), vec![0.02, 0.02]);
        assert_eq!(m.scale, Some(2.0));
        let m = AnimManifest::parse("count = 2\ndurations_ms = [10, 30]").unwrap();
        assert_eq!(m.durations(2, 1.0), vec![0.01, 0.03]);
        assert!(AnimManifest::parse("count = \"x\"").is_err());
    }
}