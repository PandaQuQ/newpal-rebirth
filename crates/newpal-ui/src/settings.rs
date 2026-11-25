//! Player settings, stored in `newpal.toml` next to the existing engine keys.
//!
//! Only the keys below are touched; every other key / table already in the file is preserved
//! (the file is merged as a TOML table, not re-serialized from a struct).
//!
//! ```toml
//! game_dir = "..."            # shared with the engine (gamedir.rs)
//! language = "zh-Hans"        # shared (i18n)
//! window_size = [1920, 1080]  # shared
//! [video]  vsync, fps_cap, present ("hd" | "shader" | "original")   # shared
//! [display] mode = "windowed" | "borderless" | "fullscreen", refresh = 0 | 60 | 120 | 144 | 165 | 240 | 300, ui_scale
//! [audio]  master, music, sfx (0..1)
//! [input]  up = ["KeyW", "ArrowUp"], ...  (winit KeyCode names)
//! ```
use crate::input::{key_from_name, key_name, Action, Bindings};
use std::path::{Path, PathBuf};
use toml::{Table, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowMode {
    Windowed,
    Borderless,
    Fullscreen,
}

impl WindowMode {
    pub const ALL: [WindowMode; 3] = [WindowMode::Windowed, WindowMode::Borderless, WindowMode::Fullscreen];
    pub fn id(self) -> &'static str {
        match self {
            WindowMode::Windowed => "windowed",
            WindowMode::Borderless => "borderless",
            WindowMode::Fullscreen => "fullscreen",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == s)
    }
}

/// Frame targets offered in the UI (0 = follow the display). The logic tick stays 35 ms; frames in between
/// are interpolated, so a higher target never changes game speed or animation length.
pub const REFRESH_TARGETS: [u32; 7] = [0, 60, 120, 144, 165, 240, 300];
pub const PRESENTS: [&str; 3] = ["hd", "shader", "original"];

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub game_dir: Option<PathBuf>,
    pub language: Option<String>,
    pub window_mode: WindowMode,
    pub resolution: (u32, u32),
    pub refresh: u32,
    pub vsync: bool,
    pub present: String,
    pub ui_scale: f32,
    pub master: f32,
    pub music: f32,
    pub sfx: f32,
    pub bindings: Bindings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            game_dir: None,
            language: None,
            window_mode: WindowMode::Windowed,
            resolution: (1600, 900),
            refresh: 0,
            vsync: true,
            present: "hd".into(),
            ui_scale: 1.0,
            master: 0.8,
            music: 0.8,
            sfx: 0.9,
            bindings: Bindings::default(),
        }
    }
}

fn f(t: Option<&Table>, k: &str) -> Option<f32> {
    t.and_then(|t| t.get(k)).and_then(|v| v.as_float().map(|x| x as f32).or_else(|| v.as_integer().map(|x| x as f32)))
}

impl Settings {
    pub fn from_table(t: &Table) -> Self {
        let mut s = Self::default();
        s.game_dir = t.get("game_dir").and_then(|v| v.as_str()).map(PathBuf::from);
        s.language = t.get("language").and_then(|v| v.as_str()).map(String::from);
        if let Some(a) = t.get("window_size").and_then(|v| v.as_array()) {
            if let (Some(w), Some(h)) = (a.first().and_then(|v| v.as_integer()), a.get(1).and_then(|v| v.as_integer())) {
                s.resolution = (w.max(320) as u32, h.max(240) as u32);
            }
        }
        let video = t.get("video").and_then(|v| v.as_table());
        if let Some(v) = video.and_then(|v| v.get("vsync")).and_then(|v| v.as_bool()) {
            s.vsync = v;
        }
        if let Some(v) = video.and_then(|v| v.get("present")).and_then(|v| v.as_str()) {
            s.present = match v { "orig" | "original" | "pixel" | "nearest" => "original".into(), "shader" | "upscale" => "shader".into(), _ => "hd".into() };
        }
        let display = t.get("display").and_then(|v| v.as_table());
        if let Some(m) = display.and_then(|d| d.get("mode")).and_then(|v| v.as_str()).and_then(WindowMode::parse) {
            s.window_mode = m;
        }
        let fps_cap = video.and_then(|v| v.get("fps_cap")).and_then(|v| v.as_integer()).map(|x| x as u32);
        s.refresh = display.and_then(|d| d.get("refresh")).and_then(|v| v.as_integer()).map(|x| x as u32).or(fps_cap).unwrap_or(0);
        s.ui_scale = f(display, "ui_scale").unwrap_or(1.0).clamp(0.75, 1.5);
        let audio = t.get("audio").and_then(|v| v.as_table());
        s.master = f(audio, "master").unwrap_or(s.master).clamp(0.0, 1.0);
        s.music = f(audio, "music").unwrap_or(s.music).clamp(0.0, 1.0);
        s.sfx = f(audio, "sfx").unwrap_or(s.sfx).clamp(0.0, 1.0);
        if let Some(inp) = t.get("input").and_then(|v| v.as_table()) {
            for a in Action::ALL {
                if let Some(arr) = inp.get(a.id()).and_then(|v| v.as_array()) {
                    let keys: Vec<_> = arr.iter().filter_map(|v| v.as_str()).filter_map(key_from_name).collect();
                    if !keys.is_empty() {
                        s.bindings.keys.insert(a, keys);
                    }
                }
            }
        }
        s
    }

    pub fn load(path: &Path) -> Self {
        let t = std::fs::read_to_string(path).ok().and_then(|s| s.parse::<Table>().ok()).unwrap_or_default();
        Self::from_table(&t)
    }

    /// Write our keys into `t`, leaving everything else alone.
    pub fn merge_into(&self, t: &mut Table) {
        match &self.game_dir {
            Some(p) => {
                t.insert("game_dir".into(), Value::String(p.display().to_string()));
            }
            None => {}
        }
        if let Some(l) = &self.language {
            t.insert("language".into(), Value::String(l.clone()));
        }
        t.insert("window_size".into(), Value::Array(vec![Value::Integer(self.resolution.0 as i64), Value::Integer(self.resolution.1 as i64)]));
        let sub = |t: &mut Table, k: &str| -> Table { t.get(k).and_then(|v| v.as_table()).cloned().unwrap_or_default() };
        let mut video = sub(t, "video");
        video.insert("vsync".into(), Value::Boolean(self.vsync));
        video.insert("fps_cap".into(), Value::Integer(self.refresh as i64));
        video.insert("present".into(), Value::String(self.present.clone()));
        t.insert("video".into(), Value::Table(video));
        let mut display = sub(t, "display");
        display.insert("mode".into(), Value::String(self.window_mode.id().into()));
        display.insert("refresh".into(), Value::Integer(self.refresh as i64));
        display.insert("ui_scale".into(), Value::Float((self.ui_scale as f64 * 100.0).round() / 100.0));
        t.insert("display".into(), Value::Table(display));
        let mut audio = sub(t, "audio");
        for (k, v) in [("master", self.master), ("music", self.music), ("sfx", self.sfx)] {
            audio.insert(k.into(), Value::Float((v as f64 * 100.0).round() / 100.0));
        }
        t.insert("audio".into(), Value::Table(audio));
        let mut input = sub(t, "input");
        for (a, keys) in &self.bindings.keys {
            input.insert(a.id().into(), Value::Array(keys.iter().filter_map(|k| key_name(*k)).map(|n| Value::String(n.into())).collect()));
        }
        t.insert("input".into(), Value::Table(input));
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let existing = std::fs::read_to_string(path).unwrap_or_default();
        let mut t: Table = existing.parse().map_err(|e| format!("{}: {e}", path.display()))?;
        self.merge_into(&mut t);
        let mut out = String::new();
        // keep the leading comment block of the file
        for l in existing.lines() {
            if l.starts_with('#') {
                out.push_str(l);
                out.push('\n');
            } else {
                break;
            }
        }
        if out.is_empty() {
            out.push_str("# New PAL Reborn - paths and preferences only, never original game data.\n");
        }
        out.push('\n');
        out.push_str(&toml::to_string_pretty(&t).map_err(|e| e.to_string())?);
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, out).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_preserves_foreign_keys() {
        let src = "# head\n[paths]\ndata_root = \"F:\\\\x\"\n[video]\ninterpolate = true\n[smooth]\nquality = 4\n";
        let dir = std::env::temp_dir().join(format!("newpal-ui-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("newpal.toml");
        std::fs::write(&p, src).unwrap();
        let mut s = Settings::load(&p);
        s.window_mode = WindowMode::Borderless;
        s.refresh = 165;
        s.master = 0.5;
        s.language = Some("en".into());
        s.bindings.rebind(Action::Confirm, winit::keyboard::KeyCode::KeyJ);
        s.save(&p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("# head"));
        let t: Table = text.parse().unwrap();
        assert_eq!(t["paths"]["data_root"].as_str(), Some("F:\\x"));
        assert_eq!(t["video"]["interpolate"].as_bool(), Some(true));
        assert_eq!(t["smooth"]["quality"].as_integer(), Some(4));
        let back = Settings::load(&p);
        assert_eq!(back, s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
