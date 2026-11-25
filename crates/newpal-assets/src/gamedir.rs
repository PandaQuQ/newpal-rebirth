//! Locating and validating the player's own original game install.
//!
//! The engine ships with **no** original data. The game directory is resolved in this order:
//! 1. CLI `--game-dir <path>`
//! 2. env `NEWPAL_GAME_DIR`
//! 3. `game_dir` in `newpal.toml` (portable: next to the exe; otherwise `%APPDATA%\NewPAL-Reborn\newpal.toml`)
//! 4. Steam auto-detect: Steam root (registry / default paths) -> `steamapps/libraryfolders.vdf`
//!    -> `appmanifest_695630.acf` -> `steamapps/common/<installdir>`
//!
//! Every candidate is validated (required files present; sizes / SHA-256 of key files compared against
//! the known Steam 2017 build). Data is only ever opened read-only.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const STEAM_APP_ID: u32 = 695630;
pub const CONFIG_FILE: &str = "newpal.toml";
pub const ENV_GAME_DIR: &str = "NEWPAL_GAME_DIR";
pub const ENV_DATA_ROOT: &str = "NEWPAL_DATA_ROOT";

/// User configuration (`newpal.toml`). Contains paths and preferences only, never game data.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// Original game folder (read-only).
    pub game_dir: Option<PathBuf>,
    /// Mods folder (default: `mods` next to the exe).
    pub mods_dir: Option<PathBuf>,
    /// Enabled mods in priority order (first = highest). Empty = all mods found, by their `priority`.
    #[serde(default)]
    pub mods: Vec<String>,
    /// Logical view height in original pixels (480 = original). Width follows the window aspect.
    pub view_height: Option<u32>,
    /// Window size in physical pixels.
    pub window_size: Option<[u32; 2]>,
    /// UI / dialogue language tag, e.g. `zh-Hant` (original), `zh-Hans`, `en` (see `i18n`). Default: original.
    pub language: Option<String>,
    /// Extra folder with language packs (`<lang_dir>/<locale>/*.toml`); `lang` next to the exe is always searched.
    pub lang_dir: Option<PathBuf>,
    /// Frame pacing / smoothing (`[video]` table).
    #[serde(default)]
    pub video: VideoConfig,
    /// Launcher asset preparation (`[smooth]` table).
    #[serde(default)]
    pub smooth: SmoothConfig,
    /// Cache / tools / output root (`[paths]` table).
    #[serde(default)]
    pub paths: PathsConfig,
}

/// `[smooth]`: locally generated in-between frames (prepared cache).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SmoothConfig {
    /// 2 or 4 (default 4)
    pub quality: Option<u32>,
    /// `auto` (GPU RIFE if Vulkan works, else CPU) | `gpu` | `cpu` | `off`  (default auto)
    pub backend: Option<String>,
    /// parallel workers (default 2)
    pub workers: Option<usize>,
    /// use this rife-ncnn-vulkan.exe instead of downloading the pinned release
    pub rife_path: Option<PathBuf>,
}

/// Project or per-user data root (caches, downloaded tools, output).
///
/// Resolution: `NEWPAL_DATA_ROOT` > discovered project root (LAYOUT.md or `cache/`+`tools/`)
/// > `%LOCALAPPDATA%\\NewPAL-Reborn`.
pub fn user_data_dir() -> PathBuf {
    if let Ok(a) = std::env::var(ENV_DATA_ROOT) {
        if !a.is_empty() {
            return PathBuf::from(a);
        }
    }
    if let Some(p) = discover_project_root() {
        return p;
    }
    if let Some(a) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(a).join("NewPAL-Reborn");
    }
    if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(x).join("newpal-reborn");
    }
    user_config_dir()
}

/// Cache, tools and screenshot output under [`user_data_dir`].
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PathsConfig {
    pub data_root: Option<PathBuf>,
    pub hd_cache: Option<PathBuf>,
    pub tools: Option<PathBuf>,
    pub output: Option<PathBuf>,
}

pub fn is_project_root(p: &Path) -> bool {
    p.join("LAYOUT.md").is_file() || (p.join("cache").is_dir() && p.join("tools").is_dir())
}

/// Walk from the exe and cwd to find the project layout.
pub fn discover_project_root() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(d) = exe_dir() {
        let mut cur = d;
        cands.push(cur.clone());
        for _ in 0..5 {
            if let Some(p) = cur.parent() {
                cands.push(p.to_path_buf());
                cur = p.to_path_buf();
            } else {
                break;
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        cands.push(cwd.clone());
        if let Some(p) = cwd.parent() {
            cands.push(p.to_path_buf());
        }
    }
    for c in cands {
        if is_project_root(&c) {
            return Some(c);
        }
    }
    None
}

pub fn cache_dir() -> PathBuf {
    user_data_dir().join("cache")
}
pub fn tools_dir() -> PathBuf {
    user_data_dir().join("tools")
}
pub fn output_dir() -> PathBuf {
    user_data_dir().join("output")
}

/// `[video]` settings; every field is optional (CLI flags override, hotkeys toggle at runtime).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct VideoConfig {
    /// present with vsync (default true)
    pub vsync: Option<bool>,
    /// frame cap in Hz, 0 = none (default 0; with vsync the display rate caps it)
    pub fps_cap: Option<u32>,
    /// interpolate positions / camera / UI between logic ticks (default true)
    pub interpolate: Option<bool>,
    /// play mod in-between frames (`act/<act>/<slot>/<action>/anim.toml`) when present (default true)
    pub smooth_frames: Option<bool>,
    /// cheap CPU-free fallback: cross-fade original frames (default false)
    pub crossfade: Option<bool>,
    /// logic tick rate in Hz (default = original 1000/35 ~ 28.6 Hz)
    pub logic_hz: Option<f32>,
    /// present mode: original | shader | hd (default hd; missing HD stills use shader)
    pub present: Option<String>,
    /// realtime upscale shader: anime4k | xbr (default anime4k)
    pub shader: Option<String>,
    /// warp between keyframes at display rate (default true when flow cache exists)
    pub flow: Option<bool>,
}

impl Config {
    /// `newpal.toml` next to the exe if it exists (portable mode), else the per-user location.
    pub fn default_path() -> PathBuf {
        if let Some(p) = exe_dir().map(|d| d.join(CONFIG_FILE)) {
            if p.exists() {
                return p;
            }
        }
        if let Some(p) = discover_project_root().map(|d| d.join(CONFIG_FILE)) {
            if p.exists() {
                return p;
            }
        }
        user_config_dir().join(CONFIG_FILE)
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| Error::Config(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(Error::io(e, path.display().to_string())),
        }
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| Error::io(e, d.display().to_string()))?;
        }
        let s = toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        std::fs::write(path, s).map_err(|e| Error::io(e, path.display().to_string()))
    }
}

pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf))
}

pub fn user_config_dir() -> PathBuf {
    if let Some(a) = std::env::var_os("APPDATA") {
        return PathBuf::from(a).join("NewPAL-Reborn");
    }
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(x).join("newpal-reborn");
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config").join("newpal-reborn")).unwrap_or_else(|| PathBuf::from("."))
}

/// A file the engine expects in the original install.
pub struct KeyFile {
    pub name: &'static str,
    pub required: bool,
    pub size: Option<u64>,
    pub sha256: Option<&'static str>,
}

/// Known Steam 2017 build (v2.17.102.0). Hashes are only computed for files < 2 MiB.
pub const KEY_FILES: &[KeyFile] = &[
    KeyFile { name: "ghero.dat", required: true, size: Some(4084), sha256: Some("a81e724aff054ee400fae176d5b8e31924df69110f4f4c89160d589199c81b30") },
    KeyFile { name: "geindex.dat", required: true, size: Some(75420), sha256: Some("5eb03ef72cb0bacb238c0815c3b9f208f71e07bf86c4cfa28628ca8bc82f481e") },
    KeyFile { name: "gmagic.dat", required: true, size: Some(54173), sha256: Some("421d03f7d80a2f25c1afbb8fc2c1249dc5c2a0dd41684add7b7d8f073628708f") },
    KeyFile { name: "gmon.dat", required: true, size: Some(67587), sha256: Some("beb32f2ed688ecce7900d806807f79cfd0431622419941c523af59d2f862326d") },
    KeyFile { name: "gitem.dat", required: true, size: Some(198814), sha256: None },
    KeyFile { name: "glevel.dat", required: true, size: Some(250410), sha256: None },
    KeyFile { name: "talk1.dat", required: true, size: Some(322250), sha256: Some("8a9891679877976e888c475d631f832dd7824f72c57866089f266c8e325176a6") },
    KeyFile { name: "all_char.act", required: true, size: Some(776742), sha256: Some("6c5105c5ea5f509cb4497a36d58b5dcc7d96a0bb8d336b3e47f44b171e7d6801") },
    KeyFile { name: "all_char.tsw", required: true, size: Some(213093149), sha256: None },
    KeyFile { name: "huge.lmf", required: true, size: Some(594549270), sha256: None },
    KeyFile { name: "huge2.lmf", required: true, size: Some(264098766), sha256: None },
    KeyFile { name: "all.snd", required: false, size: Some(50972930), sha256: None },
    KeyFile { name: "Resource/CT.ttf", required: false, size: Some(6699252), sha256: None },
    KeyFile { name: "NewPAL_Release.exe", required: false, size: Some(1429504), sha256: Some("1301e72bf46008eb74a0f49ab81d9894ab92d037d63c63d86c906aa067c961dc") },
];

#[derive(Clone, Debug)]
pub struct FileCheck {
    pub name: String,
    pub required: bool,
    pub present: bool,
    pub size: Option<u64>,
    pub size_ok: Option<bool>,
    pub sha_ok: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct Validation {
    pub dir: PathBuf,
    pub files: Vec<FileCheck>,
}

impl Validation {
    /// All required files exist -> the engine can try to run.
    pub fn usable(&self) -> bool {
        self.files.iter().all(|f| !f.required || f.present)
    }
    /// Every checked size/hash matches the known build.
    pub fn exact_build(&self) -> bool {
        self.files.iter().all(|f| f.present && f.size_ok != Some(false) && f.sha_ok != Some(false))
    }
    pub fn report(&self) -> String {
        let mut s = format!("game dir: {}\n", self.dir.display());
        for f in &self.files {
            let st = if !f.present {
                if f.required { "MISSING (required)" } else { "missing (optional)" }
            } else if f.sha_ok == Some(false) || f.size_ok == Some(false) {
                "present, differs from known build"
            } else {
                "ok"
            };
            s.push_str(&format!("  {:<20} {}\n", f.name, st));
        }
        s.push_str(&format!("usable: {}  exact build: {}\n", self.usable(), self.exact_build()));
        s
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Validate a candidate game directory (read-only).
pub fn validate(dir: &Path) -> Validation {
    let files = KEY_FILES
        .iter()
        .map(|k| {
            let p = dir.join(k.name);
            let md = std::fs::metadata(&p).ok().filter(|m| m.is_file());
            let size = md.as_ref().map(|m| m.len());
            let size_ok = match (size, k.size) {
                (Some(a), Some(b)) => Some(a == b),
                _ => None,
            };
            let sha_ok = match (k.sha256, size) {
                (Some(h), Some(sz)) if sz < 2 * 1024 * 1024 => std::fs::read(&p).ok().map(|b| sha256_hex(&b) == h),
                _ => None,
            };
            FileCheck { name: k.name.to_string(), required: k.required, present: md.is_some(), size, size_ok, sha_ok }
        })
        .collect();
    Validation { dir: dir.to_path_buf(), files }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Cli,
    Env,
    Config(PathBuf),
    Steam,
}

#[derive(Clone, Debug)]
pub struct Resolved {
    pub dir: PathBuf,
    pub source: Source,
    pub validation: Validation,
}

/// Resolve the game directory (see module docs). Returns the first *usable* candidate,
/// or an error listing everything that was tried.
pub fn resolve(cli: Option<&Path>, cfg: &Config, cfg_path: &Path) -> Result<Resolved> {
    let mut cands: Vec<(PathBuf, Source)> = Vec::new();
    if let Some(p) = cli {
        cands.push((p.to_path_buf(), Source::Cli));
    }
    if let Some(p) = std::env::var_os(ENV_GAME_DIR) {
        cands.push((PathBuf::from(p), Source::Env));
    }
    if let Some(p) = &cfg.game_dir {
        cands.push((p.clone(), Source::Config(cfg_path.to_path_buf())));
    }
    for p in find_steam_installs(STEAM_APP_ID) {
        cands.push((p, Source::Steam));
    }
    let mut tried = Vec::new();
    for (dir, source) in cands {
        let v = validate(&dir);
        if v.usable() {
            return Ok(Resolved { dir, source, validation: v });
        }
        tried.push(format!("{} ({:?})", dir.display(), source));
    }
    Err(Error::not_found(format!(
        "original game data not found. Tried: [{}]. Pass --game-dir, set {ENV_GAME_DIR}, or set game_dir in newpal.toml",
        tried.join(", ")
    )))
}

// ---------------- Steam auto-detect ----------------

/// Candidate Steam root folders.
pub fn steam_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        use winreg::RegKey;
        if let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Valve\\Steam") {
            if let Ok(p) = k.get_value::<String, _>("SteamPath") {
                v.push(PathBuf::from(p.replace('/', "\\")));
            }
        }
        for key in ["SOFTWARE\\WOW6432Node\\Valve\\Steam", "SOFTWARE\\Valve\\Steam"] {
            if let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(key) {
                if let Ok(p) = k.get_value::<String, _>("InstallPath") {
                    v.push(PathBuf::from(p));
                }
            }
        }
        v.push(PathBuf::from("C:\\Program Files (x86)\\Steam"));
        v.push(PathBuf::from("C:\\Program Files\\Steam"));
    }
    if let Some(h) = std::env::var_os("HOME") {
        let h = PathBuf::from(h);
        v.push(h.join(".steam/steam"));
        v.push(h.join(".local/share/Steam"));
        v.push(h.join("Library/Application Support/Steam"));
    }
    let mut seen = Vec::new();
    v.retain(|p| {
        let key = p.to_string_lossy().to_lowercase();
        let keep = p.is_dir() && !seen.contains(&key);
        seen.push(key);
        keep
    });
    v
}

/// Tokenize Valve KeyValues text into quoted strings (handles `\\` and `\"`).
pub fn vdf_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = text.chars();
    while let Some(c) = it.next() {
        if c != '"' {
            continue;
        }
        let mut s = String::new();
        while let Some(c) = it.next() {
            match c {
                '\\' => {
                    if let Some(n) = it.next() {
                        s.push(n);
                    }
                }
                '"' => break,
                _ => s.push(c),
            }
        }
        out.push(s);
    }
    out
}

/// Value following the first key `key` (case-insensitive) in a KeyValues file.
pub fn vdf_values(text: &str, key: &str) -> Vec<String> {
    let t = vdf_tokens(text);
    t.windows(2).filter(|w| w[0].eq_ignore_ascii_case(key)).map(|w| w[1].clone()).collect()
}

/// All installs of `app_id` across Steam libraries.
pub fn find_steam_installs(app_id: u32) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in steam_roots() {
        let mut libs = vec![root.clone()];
        if let Ok(t) = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) {
            libs.extend(vdf_values(&t, "path").into_iter().map(PathBuf::from));
        }
        for lib in libs {
            let man = lib.join("steamapps").join(format!("appmanifest_{app_id}.acf"));
            if let Ok(t) = std::fs::read_to_string(&man) {
                if let Some(dir) = vdf_values(&t, "installdir").into_iter().next() {
                    let p = lib.join("steamapps").join("common").join(dir);
                    if p.is_dir() && !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vdf_parse() {
        let t = r#""libraryfolders" { "0" { "path" "C:\\Program Files (x86)\\Steam" "apps" { "228980" "1" } } "1" { "path" "D:\\SteamLibrary" "apps" { "695630" "123" } } }"#;
        assert_eq!(vdf_values(t, "path"), vec!["C:\\Program Files (x86)\\Steam".to_string(), "D:\\SteamLibrary".to_string()]);
        let m = r#""AppState" { "appid" "695630" "installdir" "NewPAL" }"#;
        assert_eq!(vdf_values(m, "installdir"), vec!["NewPAL".to_string()]);
    }
    #[test]
    fn config_roundtrip() {
        let c = Config { game_dir: Some("F:/x".into()), mods: vec!["hd".into()], ..Default::default() };
        let s = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.game_dir, c.game_dir);
        assert_eq!(back.mods, c.mods);
    }
}