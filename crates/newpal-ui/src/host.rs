//! What the UI needs from the engine. Implemented by the game crate (`newpal-game/src/shell.rs`), so this crate
//! stays free of asset / VM dependencies and can be tested headless.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct DirCandidate {
    pub path: PathBuf,
    /// "steam" | "config" | "env" | "picked"
    pub source: String,
}

#[derive(Clone, Debug, Default)]
pub struct DirCheck {
    pub path: PathBuf,
    pub usable: bool,
    /// identical to the known Steam build (sizes / hashes)
    pub exact: bool,
    pub ok_files: usize,
    pub total_files: usize,
    pub missing: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HdState {
    /// HD stills present in the local cache
    Ready { items: usize },
    /// no cache; `patch_items` > 0 if a residual patch package was found next to the game / in the data root
    Missing { patch_items: usize, patch_bytes: u64 },
    Installing { done: usize, total: usize, eta_s: Option<f32>, simulated: bool },
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct SaveSlot {
    pub id: u32,
    pub title: String,
    pub location: String,
    pub play_time_s: u64,
    pub saved_at: String,
}

#[derive(Clone, Debug)]
pub struct ModEntry {
    pub name: String,
    pub enabled: bool,
    pub priority: i32,
    pub dir: String,
}

#[derive(Clone, Debug)]
pub struct LangEntry {
    pub tag: String,
    pub name: String,
}

pub trait Host {
    /// Candidates from config / env / Steam libraries (each still needs [Host::check_dir]).
    fn detect_game_dirs(&mut self) -> Vec<DirCandidate>;
    fn check_dir(&mut self, dir: &Path) -> DirCheck;
    /// Native folder picker (rfd). `None` if cancelled or headless.
    fn pick_dir(&mut self) -> Option<PathBuf>;
    /// Currently configured game folder (after [crate::scene::Cmd::UseGameDir]).
    fn game_dir(&self) -> Option<PathBuf>;
    fn hd_state(&mut self) -> HdState;
    fn start_hd_install(&mut self);
    fn cancel_hd_install(&mut self);
    /// Save slots, newest first. Empty until the save system exists (interface reserved).
    fn saves(&mut self) -> Vec<SaveSlot>;
    fn mods(&mut self) -> Vec<ModEntry>;
    fn mods_dir(&self) -> Option<PathBuf>;
    fn languages(&mut self) -> Vec<LangEntry>;
    /// Window sizes offered for windowed / exclusive mode.
    fn resolutions(&self) -> Vec<(u32, u32)>;
    /// Highest refresh rate of the current display, if known.
    fn display_hz(&self) -> Option<u32>;
    fn version(&self) -> String;
    /// Party money for the system menu's money box (`None` while the VM does not expose it).
    fn money(&self) -> Option<i64> {
        None
    }
}

/// Inert host for tests and previews.
#[derive(Default)]
pub struct NullHost {
    pub dir: Option<PathBuf>,
}

impl Host for NullHost {
    fn detect_game_dirs(&mut self) -> Vec<DirCandidate> {
        Vec::new()
    }
    fn check_dir(&mut self, dir: &Path) -> DirCheck {
        DirCheck { path: dir.to_path_buf(), ..Default::default() }
    }
    fn pick_dir(&mut self) -> Option<PathBuf> {
        None
    }
    fn game_dir(&self) -> Option<PathBuf> {
        self.dir.clone()
    }
    fn hd_state(&mut self) -> HdState {
        HdState::Missing { patch_items: 0, patch_bytes: 0 }
    }
    fn start_hd_install(&mut self) {}
    fn cancel_hd_install(&mut self) {}
    fn saves(&mut self) -> Vec<SaveSlot> {
        Vec::new()
    }
    fn mods(&mut self) -> Vec<ModEntry> {
        Vec::new()
    }
    fn mods_dir(&self) -> Option<PathBuf> {
        None
    }
    fn languages(&mut self) -> Vec<LangEntry> {
        ["zh-Hant", "zh-Hans", "en"].into_iter().map(|t| LangEntry { tag: t.into(), name: t.into() }).collect()
    }
    fn resolutions(&self) -> Vec<(u32, u32)> {
        vec![(1280, 720), (1600, 900), (1920, 1080), (2560, 1440), (3840, 2160)]
    }
    fn display_hz(&self) -> Option<u32> {
        None
    }
    fn version(&self) -> String {
        "dev".into()
    }
}
