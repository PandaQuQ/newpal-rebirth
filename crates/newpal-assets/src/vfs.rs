//! Virtual file system: mods first (by priority), then the original game data (read-only, in place).
//!
//! ## Logical names (stable, used by mods)
//! | logical name | original source |
//! |---|---|
//! | `sprites/<tsw>/<id>/f<k>.png`        | `<tsw>.tsw` entry id (1-based = slot+1), frame k |
//! | `maps/<lmf>/<id>/bg.png`             | `<lmf>.lmf` map id background |
//! | `maps/<lmf>/<id>/obj_<oid>.png`      | object (background cut-out) `oid` of that map |
//! | `audio/snd/<slot>.wav`               | `all.snd` slot (0-based; script SFX id = slot + 1) |
//! | `music/<stem>.ogg|mp3|wav|flac`      | `Music/<stem>.mp3` (stem from the maps.dat music table, e.g. `m027`) |
//! | `act/<act>/<slot>/<action>/anim.toml` | in-between frames for an action (see [`crate::smooth`]) |
//! | `text/talk/<n>.json`                 | text overrides for talk script n: `{ "<hex pos>": "new text" }` (locale independent; see also `lang/<locale>/*.toml`, [`crate::i18n`]) |
//! | `lang/<locale>/*.toml`               | language pack inside a mod (keys: see [`crate::i18n`]) |
//! | `scripts/talk/<n>.bin`               | full replacement bytecode for talk script n |
//! | `tables/<name>.json` / `.toml`       | patch for `<name>.dat` value table (see [`crate::tables`]) |
//! | `fonts/dialog.ttf`                   | dialogue font (else game `Resource/CT.ttf`, else system CJK font) |
//! | `raw/<relative path>`                | any original file, byte-for-byte replacement |
//!
//! ## HD replacements
//! An image may be supplied at higher resolution. It always renders at the original **logical** size:
//! * explicit: `f2@2x.png`, `bg@4x.png` (logical = native / N)
//! * implicit: `f2.png` larger than the original -> logical = original size
//!
//! Mods live in `<mods_dir>/<name>/` with an optional `mod.toml` (`name`, `priority`, `enabled`).
use crate::act::{parse_act, Action};
use crate::image::RgbaImage;
use crate::lmf::{Lmf, LmfMap};
use crate::pak::IndexArchive;
use crate::tables::Table;
use crate::talk::{file_for, script_from_file, TalkScript};
use crate::tsw::SpriteEntry;
use crate::{Error, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct ModInfo {
    pub name: String,
    pub dir: PathBuf,
    pub priority: i32,
}

#[derive(Deserialize, Default)]
struct ModToml {
    name: Option<String>,
    #[serde(default)]
    priority: i32,
    enabled: Option<bool>,
}

/// Where an asset came from.
#[derive(Clone, Debug, PartialEq)]
pub enum Origin {
    Game,
    Mod(String),
    HdCache,
}

/// An image plus the size it occupies in logical (original-pixel) units.
#[derive(Clone, Debug)]
pub struct Texture {
    pub image: RgbaImage,
    pub logical_w: f32,
    pub logical_h: f32,
    pub origin: Origin,
}

impl Texture {
    pub fn native(image: RgbaImage, origin: Origin) -> Self {
        let (w, h) = (image.width as f32, image.height as f32);
        Self { image, logical_w: w, logical_h: h, origin }
    }
    /// Native pixels per logical pixel (1.0 = original resolution).
    pub fn scale(&self) -> f32 {
        if self.logical_w > 0.0 { self.image.width as f32 / self.logical_w } else { 1.0 }
    }
}

pub struct Vfs {
    game: PathBuf,
    mods: Vec<ModInfo>,
    paks: Mutex<HashMap<String, Arc<IndexArchive>>>,
    lmfs: Mutex<HashMap<String, Arc<Lmf>>>,
    sprites: Mutex<HashMap<(String, usize), Arc<SpriteEntry>>>,
    /// prepared smooth-animation cache (launcher prepare step); used after mods, before original frames
    smooth: Option<(Arc<crate::smoothpak::SmoothPak>, u32)>,
    /// prepared HD stills (launcher --hd-prepare); used after mods, before original frames
    hd_dir: Option<PathBuf>,
    hd_on: AtomicBool,
    i18n: Arc<crate::i18n::I18n>,
}

/// Discover mods in `mods_dir`. `enabled` (if non-empty) selects and orders them (first = highest priority).
pub fn discover_mods(mods_dir: &Path, enabled: &[String]) -> Vec<ModInfo> {
    let mut found = Vec::new();
    if let Ok(rd) = std::fs::read_dir(mods_dir) {
        for e in rd.flatten() {
            let dir = e.path();
            if !dir.is_dir() {
                continue;
            }
            let folder = e.file_name().to_string_lossy().to_string();
            let mt: ModToml = std::fs::read_to_string(dir.join("mod.toml")).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default();
            if mt.enabled == Some(false) && enabled.is_empty() {
                continue;
            }
            found.push(ModInfo { name: mt.name.unwrap_or_else(|| folder.clone()), dir, priority: mt.priority });
        }
    }
    if enabled.is_empty() {
        found.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.name.cmp(&b.name)));
        found
    } else {
        enabled
            .iter()
            .filter_map(|n| found.iter().find(|m| &m.name == n || m.dir.file_name().map(|f| f.to_string_lossy() == n.as_str()).unwrap_or(false)).cloned())
            .collect()
    }
}

const SCALES: &[(&str, f32)] = &[("@4x", 4.0), ("@3x", 3.0), ("@2x", 2.0), ("", 0.0)];

impl Vfs {
    fn default_i18n(game: &Path) -> Arc<crate::i18n::I18n> {
        let i = crate::i18n::I18n::default();
        i.set_source(Arc::new(crate::legacy::source::LegacySource::new(game.to_path_buf())));
        Arc::new(i)
    }

    pub fn new(game: PathBuf, mods: Vec<ModInfo>) -> Self {
        let i18n = Self::default_i18n(&game);
        Self { game, mods, paks: Mutex::default(), lmfs: Mutex::default(), sprites: Mutex::default(), smooth: None, hd_dir: None, hd_on: AtomicBool::new(true), i18n }
    }

    /// Attach the prepared cache; only entries generated with `factor` are used.
    pub fn set_smooth_cache(&mut self, pak: Option<Arc<crate::smoothpak::SmoothPak>>, factor: u32) {
        self.smooth = pak.map(|p| (p, factor));
    }

    pub fn smooth_cache(&self) -> Option<&Arc<crate::smoothpak::SmoothPak>> {
        self.smooth.as_ref().map(|s| &s.0)
    }

    /// Attach the language layer (default: original language, no packs).
    pub fn set_i18n(&mut self, i: Arc<crate::i18n::I18n>) {
        // the catalog reads the original text / the install's own translations from this game folder
        if !i.has_source() {
            i.set_source(Arc::new(crate::legacy::source::LegacySource::new(self.game.clone())));
        }
        self.i18n = i;
    }

    pub fn i18n(&self) -> &Arc<crate::i18n::I18n> {
        &self.i18n
    }

    pub fn game_dir(&self) -> &Path {
        &self.game
    }

    pub fn mods(&self) -> &[ModInfo] {
        &self.mods
    }

    /// Find a mod file for `logical` (highest priority first). Returns (path, mod name, explicit scale or 0).
    pub fn find_override(&self, logical: &str) -> Option<(PathBuf, String, f32)> {
        let rel = Path::new(logical);
        let stem = rel.file_stem()?.to_string_lossy().to_string();
        let ext = rel.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        for m in &self.mods {
            for (suffix, scale) in SCALES {
                let p = m.dir.join(rel.with_file_name(format!("{stem}{suffix}{ext}")));
                if p.is_file() {
                    return Some((p, m.name.clone(), *scale));
                }
            }
        }
        None
    }

    fn override_texture(&self, logical: &str, orig_size: Option<(u32, u32)>) -> Result<Option<Texture>> {
        let Some((path, name, scale)) = self.find_override(logical) else { return Ok(None) };
        let bytes = std::fs::read(&path).map_err(|e| Error::io(e, path.display().to_string()))?;
        let image = RgbaImage::from_png(&bytes)?;
        let (lw, lh) = if scale > 0.0 {
            (image.width as f32 / scale, image.height as f32 / scale)
        } else if let Some((w, h)) = orig_size {
            (w as f32, h as f32)
        } else {
            (image.width as f32, image.height as f32)
        };
        Ok(Some(Texture { image, logical_w: lw, logical_h: lh, origin: Origin::Mod(name) }))
    }

    /// Read an original file (or its `raw/` override). Read-only.
    pub fn read_raw(&self, rel: &str) -> Result<Vec<u8>> {
        if let Some((p, _, _)) = self.find_override(&format!("raw/{rel}")) {
            return std::fs::read(&p).map_err(|e| Error::io(e, p.display().to_string()));
        }
        let p = self.game.join(rel);
        std::fs::read(&p).map_err(|e| Error::io(e, p.display().to_string()))
    }

    pub fn pak(&self, file: &str) -> Result<Arc<IndexArchive>> {
        let mut g = self.paks.lock().unwrap();
        if let Some(a) = g.get(file) {
            return Ok(a.clone());
        }
        let a = Arc::new(IndexArchive::open(&self.game.join(file))?);
        g.insert(file.to_string(), a.clone());
        Ok(a)
    }

    pub fn lmf(&self, file: &str) -> Result<Arc<Lmf>> {
        let mut g = self.lmfs.lock().unwrap();
        if let Some(a) = g.get(file) {
            return Ok(a.clone());
        }
        let a = Arc::new(Lmf::open(&self.game.join(file))?);
        g.insert(file.to_string(), a.clone());
        Ok(a)
    }

    /// `tsw` without extension, e.g. "all_char".
    pub fn sprite_entry(&self, tsw: &str, slot: usize) -> Result<Arc<SpriteEntry>> {
        let key = (tsw.to_string(), slot);
        if let Some(e) = self.sprites.lock().unwrap().get(&key) {
            return Ok(e.clone());
        }
        let e = Arc::new(SpriteEntry::parse(self.pak(&format!("{tsw}.tsw"))?.read(slot)?)?);
        self.sprites.lock().unwrap().insert(key, e.clone());
        Ok(e)
    }

    /// Sprite frame by tsw entry id (1-based, as used by ACT `FR`).
    pub fn sprite_frame(&self, tsw: &str, id: u32, frame: usize) -> Result<Texture> {
        let logical = format!("sprites/{tsw}/{id}/f{frame}.png");
        let slot = id.checked_sub(1).ok_or_else(|| Error::not_found("sprite id 0"))? as usize;
        let entry = self.sprite_entry(tsw, slot).ok();
        let orig = entry.as_ref().and_then(|e| e.descs.get(frame).map(|d| (d.width(), d.height())));
        if let Some(t) = self.override_texture(&logical, orig)? {
            return Ok(t);
        }
        if self.hd_enabled() {
            if let Some(t) = self.hd_texture(&logical, orig)? {
                return Ok(t);
            }
        }
        let entry = entry.ok_or_else(|| Error::not_found(logical.clone()))?;
        Ok(Texture::native(entry.frame_image(frame)?, Origin::Game))
    }

    pub fn map(&self, lmf: &str, id: u32) -> Result<LmfMap> {
        self.lmf(&format!("{lmf}.lmf"))?.load_map(id)
    }

    pub fn map_bg(&self, lmf: &str, map: &LmfMap) -> Result<Texture> {
        let logical = format!("maps/{lmf}/{}/bg.png", map.id);
        if let Some(t) = self.override_texture(&logical, Some(map.pixel_size()))? {
            return Ok(t);
        }
        if self.hd_enabled() {
            if let Some(t) = self.hd_texture(&logical, Some(map.pixel_size()))? {
                return Ok(t);
            }
        }
        Ok(Texture::native(map.render_bg(), Origin::Game))
    }

    pub fn map_object(&self, lmf: &str, map: &LmfMap, oid: u16) -> Result<Option<Texture>> {
        let logical = format!("maps/{lmf}/{}/obj_{oid}.png", map.id);
        let orig = self.lmf(&format!("{lmf}.lmf"))?.object_image(map, oid)?;
        if let Some(t) = self.override_texture(&logical, orig.as_ref().map(|i| (i.width, i.height)))? {
            return Ok(Some(t));
        }
        if self.hd_enabled() {
            if let Some(t) = self.hd_texture(&logical, orig.as_ref().map(|i| (i.width, i.height)))? {
                return Ok(Some(t));
            }
        }
        Ok(orig.map(|i| Texture::native(i, Origin::Game)))
    }

    /// Prepared HD stills (`cache/hd`, 4x PNGs named `f0@4x.png` / `bg@4x.png`). Mods still win.
    pub fn set_hd_cache(&mut self, dir: Option<PathBuf>) {
        self.hd_dir = dir;
    }

    pub fn hd_dir(&self) -> Option<&Path> {
        self.hd_dir.as_deref()
    }

    pub fn set_hd_enabled(&self, on: bool) {
        self.hd_on.store(on, Ordering::Relaxed);
    }

    pub fn hd_enabled(&self) -> bool {
        self.hd_on.load(Ordering::Relaxed) && self.hd_dir.is_some()
    }

    fn hd_path(&self, logical: &str) -> Option<PathBuf> {
        let dir = self.hd_dir.as_ref()?;
        let rel = Path::new(logical);
        let stem = rel.file_stem()?.to_string_lossy();
        let ext = rel.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_else(|| ".png".into());
        let p = match rel.parent() {
            Some(parent) => dir.join(parent).join(format!("{stem}@4x{ext}")),
            None => dir.join(format!("{stem}@4x{ext}")),
        };
        p.is_file().then_some(p)
    }

    fn hd_texture(&self, logical: &str, orig_size: Option<(u32, u32)>) -> Result<Option<Texture>> {
        let Some(path) = self.hd_path(logical) else { return Ok(None) };
        let bytes = std::fs::read(&path).map_err(|e| Error::io(e, path.display().to_string()))?;
        let image = RgbaImage::from_png(&bytes)?;
        let (lw, lh) = if let Some((w, h)) = orig_size {
            (w as f32, h as f32)
        } else {
            (image.width as f32 / 4.0, image.height as f32 / 4.0)
        };
        Ok(Some(Texture { image, logical_w: lw, logical_h: lh, origin: Origin::HdCache }))
    }

    /// Original frame plus an HD cache still if one exists (for the F9 compare toggle).
    pub fn sprite_frame_pair(&self, tsw: &str, id: u32, frame: usize) -> Result<(Texture, Option<Texture>)> {
        let logical = format!("sprites/{tsw}/{id}/f{frame}.png");
        let slot = id.checked_sub(1).ok_or_else(|| Error::not_found("sprite id 0"))? as usize;
        let entry = self.sprite_entry(tsw, slot).ok();
        let orig_size = entry.as_ref().and_then(|e| e.descs.get(frame).map(|d| (d.width(), d.height())));
        if let Some(t) = self.override_texture(&logical, orig_size)? {
            return Ok((t, None));
        }
        let entry = entry.ok_or_else(|| Error::not_found(logical.clone()))?;
        let orig = Texture::native(entry.frame_image(frame)?, Origin::Game);
        let hd = self.hd_texture(&logical, orig_size)?;
        Ok((orig, hd))
    }

    pub fn map_bg_pair(&self, lmf: &str, map: &LmfMap) -> Result<(Texture, Option<Texture>)> {
        let logical = format!("maps/{lmf}/{}/bg.png", map.id);
        let size = Some(map.pixel_size());
        if let Some(t) = self.override_texture(&logical, size)? {
            return Ok((t, None));
        }
        let orig = Texture::native(map.render_bg(), Origin::Game);
        let hd = self.hd_texture(&logical, size)?;
        Ok((orig, hd))
    }

    /// Object cut-out at a placement. HD path crops the upscaled background so seams stay consistent.
    pub fn map_object_pair(&self, lmf: &str, map: &LmfMap, oid: u16, x0: u16, y0: u16) -> Result<Option<(Texture, Option<Texture>)>> {
        let logical = format!("maps/{lmf}/{}/obj_{oid}.png", map.id);
        let raw = self.lmf(&format!("{lmf}.lmf"))?.object_image(map, oid)?;
        let Some(img) = raw else { return Ok(None) };
        let size = Some((img.width, img.height));
        if let Some(t) = self.override_texture(&logical, size)? {
            return Ok(Some((t, None)));
        }
        let orig = Texture::native(img, Origin::Game);
        let hd = if let Some(t) = self.hd_texture(&logical, size)? {
            Some(t)
        } else {
            self.hd_object_from_bg(lmf, map, x0, y0, &orig.image)?
        };
        Ok(Some((orig, hd)))
    }

    fn hd_object_from_bg(&self, lmf: &str, map: &LmfMap, x0: u16, y0: u16, orig: &RgbaImage) -> Result<Option<Texture>> {
        let Some(path) = self.hd_path(&format!("maps/{lmf}/{}/bg.png", map.id)) else { return Ok(None) };
        let bg = RgbaImage::from_png(&std::fs::read(&path).map_err(|e| Error::io(e, path.display().to_string()))?)?;
        let (mw, mh) = map.pixel_size();
        if mw == 0 || mh == 0 || bg.width < mw || bg.height < mh { return Ok(None); }
        let sx = bg.width / mw;
        let sy = bg.height / mh;
        if sx == 0 || sy == 0 { return Ok(None); }
        let (cw, ch) = (orig.width * sx, orig.height * sy);
        let mut cut = RgbaImage::new(cw, ch);
        let a = orig.scale_nearest(sx);
        let ox = x0 as u32 * sx;
        let oy = y0 as u32 * sy;
        for y in 0..ch {
            for x in 0..cw {
                let alpha = if x < a.width && y < a.height {
                    a.pixels[((y * a.width + x) * 4 + 3) as usize]
                } else { 0 };
                let gx = ox + x;
                let gy = oy + y;
                if alpha == 0 || gx >= bg.width || gy >= bg.height {
                    continue;
                }
                let i = ((gy * bg.width + gx) * 4) as usize;
                cut.put(x, y, [bg.pixels[i], bg.pixels[i + 1], bg.pixels[i + 2], alpha]);
            }
        }
        Ok(Some(Texture { image: cut, logical_w: orig.width as f32, logical_h: orig.height as f32, origin: Origin::HdCache }))
    }


    /// `act` without extension, e.g. "all_char".
    pub fn act(&self, act: &str, slot: usize) -> Result<Vec<Option<Action>>> {
        parse_act(&self.pak(&format!("{act}.act"))?.read(slot)?)
    }

    pub fn talk(&self, n: u32) -> Result<TalkScript> {
        if let Some((p, _, _)) = self.find_override(&format!("scripts/talk/{n}.bin")) {
            let code = std::fs::read(&p).map_err(|e| Error::io(e, p.display().to_string()))?;
            return Ok(TalkScript::standalone(n, p.display().to_string(), code));
        }
        script_from_file(&self.read_raw(&file_for(n))?, n)
    }

    /// Text overrides for script `n`: instruction position -> replacement dialogue text.
    /// Priority: mod `text/talk/<n>.json` > language packs (see [`crate::i18n`]).
    pub fn talk_text_overrides(&self, n: u32) -> Result<HashMap<usize, String>> {
        let mut out = self.i18n.talk_overrides(n);
        let Some((p, _, _)) = self.find_override(&format!("text/talk/{n}.json")) else { return Ok(out) };
        let s = std::fs::read_to_string(&p).map_err(|e| Error::io(e, p.display().to_string()))?;
        let m: HashMap<String, String> = serde_json::from_str(&s).map_err(|e| Error::Config(format!("{}: {e}", p.display())))?;
        for (k, v) in m {
            let pos = usize::from_str_radix(k.trim_start_matches("0x"), 16).map_err(|_| Error::Config(format!("bad position key {k:?}")))?;
            // mod files written for the old control codes (`%C6`, `Name:%N`) are upgraded to markup
            let v = if crate::legacy::markup::looks_legacy(&v) { crate::legacy::markup::from_legacy(&v, true) } else { v };
            out.insert(pos, v);
        }
        Ok(out)
    }

    /// `maps.dat` scene + role (NPC) tables (`raw/maps.dat` can override).
    /// Sound effect by script id (1-based, like the original sound-effect lookup). `None` = no such sound.
    pub fn sfx(&self, id: u16) -> Result<Option<Vec<u8>>> {
        if id == 0 || id == 0xFFFF {
            return Ok(None);
        }
        let slot = id as usize - 1;
        if let Some((p, _, _)) = self.find_override(&format!("audio/snd/{slot}.wav")) {
            return std::fs::read(&p).map(Some).map_err(|e| Error::io(e, p.display().to_string()));
        }
        let snd = self.pak("all.snd")?;
        if slot >= snd.entries.len() {
            return Ok(None);
        }
        let raw = snd.read(slot)?;
        if raw.len() < 18 {
            return Ok(None);
        }
        if raw.starts_with(b"RIFF") {
            return Ok(Some(raw));
        }
        crate::pak::snd_to_wav(&raw).map(Some)
    }

    /// Background music file by maps.dat name (e.g. "M027.mp3"; the original swaps any extension for the
    /// files in `Music/`). Returns (bytes, extension). Mods: `music/<stem>.<ogg|mp3|wav|flac>`.
    pub fn music(&self, name: &str) -> Result<Option<(Vec<u8>, String)>> {
        let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        if stem.is_empty() || stem == "h" {
            return Ok(None);
        }
        for ext in ["ogg", "mp3", "wav", "flac"] {
            if let Some((p, _, _)) = self.find_override(&format!("music/{stem}.{ext}")) {
                return std::fs::read(&p).map(|b| Some((b, ext.to_string()))).map_err(|e| Error::io(e, p.display().to_string()));
            }
        }
        for cand in [format!("{stem}.mp3"), name.to_lowercase()] {
            let p = self.game.join("Music").join(&cand);
            if p.is_file() {
                let ext = Path::new(&cand).extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
                return std::fs::read(&p).map(|b| Some((b, ext))).map_err(|e| Error::io(e, p.display().to_string()));
            }
        }
        Ok(None)
    }

    /// Smooth-animation override for `<act>.act` entry `slot` (0-based), action index `action`:
    /// manifest + frame textures (logical size = png / scale). `None` when no mod provides it.
    pub fn anim_override(&self, act: &str, slot: usize, action: usize) -> Result<Option<(crate::smooth::AnimManifest, Vec<Texture>)>> {
        let Some((path, name, _)) = self.find_override(&format!("act/{act}/{slot}/{action}/anim.toml")) else {
            return self.prepared_anim(act, slot, action);
        };
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(e, path.display().to_string()))?;
        let m = crate::smooth::AnimManifest::parse(&text)?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let scale = m.scale.filter(|s| *s > 0.0).unwrap_or(1.0);
        let mut frames = Vec::new();
        for f in m.files() {
            let p = dir.join(&f);
            let bytes = std::fs::read(&p).map_err(|e| Error::io(e, p.display().to_string()))?;
            let image = RgbaImage::from_png(&bytes)?;
            let (lw, lh) = (image.width as f32 / scale, image.height as f32 / scale);
            frames.push(Texture { image, logical_w: lw, logical_h: lh, origin: Origin::Mod(name.clone()) });
        }
        Ok(Some((m, frames)))
    }

    /// Prepared cache lookup (no mod override): manifest built from the cache entry.
    pub fn prepared_anim(&self, act: &str, slot: usize, action: usize) -> Result<Option<(crate::smooth::AnimManifest, Vec<Texture>)>> {
        let Some((pak, factor)) = &self.smooth else { return Ok(None) };
        let Some(e) = pak.get(&crate::smoothpak::key(act, slot, action)) else { return Ok(None) };
        if e.factor != *factor {
            return Ok(None);
        }
        let frames = pak.frames(e)?;
        let m = crate::smooth::AnimManifest { count: Some(frames.len()), durations_ms: e.durations_ms.clone(), ..Default::default() };
        let tex = frames.into_iter().map(|i| Texture::native(i, Origin::Mod(format!("prepared:{}", e.backend)))).collect();
        Ok(Some((m, tex)))
    }

    pub fn maps_dat(&self) -> Result<crate::mapsdat::MapsDat> {
        crate::mapsdat::MapsDat::parse(&self.read_raw("maps.dat")?)
    }

    /// Value table `name` (e.g. "ghero") with mod patches applied (lowest priority first).
    pub fn table(&self, name: &str) -> Result<Table> {
        let mut t = Table::from_dat(name, &self.read_raw(&format!("{name}.dat"))?);
        for m in self.mods.iter().rev() {
            for (ext, is_toml) in [("json", false), ("toml", true)] {
                let p = m.dir.join("tables").join(format!("{name}.{ext}"));
                if let Ok(s) = std::fs::read_to_string(&p) {
                    t.apply(&Table::parse_override(&s, is_toml)?)?;
                }
            }
        }
        self.i18n.localize_table(&mut t);
        Ok(t)
    }

    /// Dialogue font bytes: mod `fonts/dialog.ttf` -> game `Resource/CT.ttf` -> system CJK font.
    pub fn font(&self) -> Result<(Vec<u8>, String)> {
        if let Some((p, n, _)) = self.find_override("fonts/dialog.ttf") {
            return Ok((std::fs::read(&p).map_err(|e| Error::io(e, p.display().to_string()))?, format!("mod {n}")));
        }
        if let Ok(b) = self.read_raw("Resource/CT.ttf") {
            return Ok((b, "game Resource/CT.ttf".into()));
        }
        for p in ["C:\\Windows\\Fonts\\msjh.ttc", "C:\\Windows\\Fonts\\mingliu.ttc", "C:\\Windows\\Fonts\\msyh.ttc", "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"] {
            if let Ok(b) = std::fs::read(p) {
                return Ok((b, p.to_string()));
            }
        }
        Err(Error::not_found("no CJK font"))
    }
}