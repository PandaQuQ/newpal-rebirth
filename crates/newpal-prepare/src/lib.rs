//! # newpal-prepare
//!
//! Launcher "asset preparation": after the player's game folder is validated, generate in-between
//! sprite frames **locally** from their own data and pack them into a per-user cache
//! (`%LOCALAPPDATA%\NewPAL-Reborn\cache\smooth.pak`). Releases contain only the engine and tools; no
//! original or generated art is distributed.
//!
//! * incremental: an entry is reused when (source hash, backend id, factor, [`ALGO`]) match
//! * resumable: every finished animation is staged in `cache/staging/*.part` at once; a killed run picks
//!   them up next time; a cancelled run still writes the pack with what is done
//! * pausable / cancelable through [`Control`]; bounded worker threads
pub mod backend;
pub mod rife;
pub mod esrgan;
pub mod mesh;
pub mod hd;
pub mod hd_batch;
pub mod residual;
pub mod residual_pack;
pub mod flow;

use backend::{Backend, Result};
use newpal_assets::act;
use newpal_assets::image::RgbaImage;
use newpal_assets::smoothpak::{self, Entry, OwnedEntry, SmoothPak};
use newpal_assets::vfs::Vfs;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// Generator algorithm version: bump to force regeneration of every entry.
pub const ALGO: u32 = 1;
/// Original main-loop frame (ms) = ACT `DS` unit.
pub const TICK_MS: f32 = 35.0;
/// Act files whose animations are prepared (map walking / NPCs use all_char).
pub const ACTS: &[&str] = &["all_char"];

#[derive(Default)]
pub struct Control {
    pub paused: AtomicBool,
    pub cancel: AtomicBool,
}

/// One source animation: frames padded to a shared canvas (bottom-centre, like the engine draws them).
pub struct Source {
    pub key: String,
    pub frames: Vec<RgbaImage>,
    pub durations_ms: Vec<f32>,
    pub hash: String,
}

/// Animation to prepare.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub act: String,
    pub slot: usize,
    pub action: usize,
}

impl Job {
    pub fn key(&self) -> String {
        smoothpak::key(&self.act, self.slot, self.action)
    }
}

/// List animations with >= 2 frames. `filter`: "all_char", "all_char:0" or "all_char:0:8".
pub fn jobs(vfs: &Vfs, filter: Option<&str>) -> Vec<Job> {
    let f: Vec<&str> = filter.map(|s| s.split(':').collect()).unwrap_or_default();
    let mut out = Vec::new();
    for &actf in ACTS {
        if f.first().is_some_and(|a| *a != actf) {
            continue;
        }
        let Ok(pak) = vfs.pak(&format!("{actf}.act")) else { continue };
        for slot in 0..pak.entries.len() {
            if f.get(1).is_some_and(|s| s.parse::<usize>().ok() != Some(slot)) {
                continue;
            }
            let Ok(acts) = vfs.act(actf, slot) else { continue };
            for (i, a) in acts.iter().enumerate() {
                if f.get(2).is_some_and(|s| s.parse::<usize>().ok() != Some(i)) {
                    continue;
                }
                let Some(a) = a else { continue };
                let n = act::steps(a).iter().filter(|s| s.sprite.is_some_and(|id| id < 3000)).count();
                if n >= 2 {
                    out.push(Job { act: actf.to_string(), slot, action: i });
                }
            }
        }
    }
    out
}

/// Load the original frames of a job (game data only; mods are not baked into the cache).
pub fn source(vfs: &Vfs, j: &Job) -> Result<Source> {
    let acts = vfs.act(&j.act, j.slot).map_err(|e| e.to_string())?;
    let a = acts.get(j.action).and_then(|a| a.as_ref()).ok_or("no action")?;
    let tsw = j.act.clone();
    let mut fr = Vec::new();
    for s in act::steps(a) {
        let Some(id) = s.sprite.filter(|&id| id < 3000) else { continue };
        let t = vfs.sprite_frame(&tsw, id, s.frame as usize).map_err(|e| e.to_string())?;
        fr.push((t.image, s.delay.max(1) as f32 * TICK_MS));
    }
    let (w, h) = fr.iter().fold((0, 0), |m, (i, _)| (m.0.max(i.width), m.1.max(i.height)));
    let mut hs = Sha256::new();
    hs.update(format!("{w}x{h}").as_bytes());
    let mut frames = Vec::new();
    let mut durations_ms = Vec::new();
    for (img, d) in fr {
        let mut c = RgbaImage::new(w, h);
        c.blit(&img, ((w - img.width) / 2) as i32, (h - img.height) as i32);
        hs.update(&c.pixels);
        hs.update(d.to_le_bytes());
        frames.push(c);
        durations_ms.push(d);
    }
    let hash = hs.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok(Source { key: j.key(), frames, durations_ms, hash })
}

/// Generate one entry.
pub fn generate(src: &Source, be: &dyn Backend, factor: u32) -> Result<OwnedEntry> {
    let distinct = src.frames.windows(2).any(|w| w[0].pixels != w[1].pixels);
    let frames = if distinct { be.interpolate(&src.frames, factor)? } else { src.frames.clone() };
    let f = if distinct { factor } else { 1 };
    let durations_ms = src.durations_ms.iter().flat_map(|d| std::iter::repeat(d / f as f32).take(f as usize)).collect::<Vec<_>>();
    if frames.len() != durations_ms.len() {
        return Err(format!("{}: backend returned {} frames, expected {}", src.key, frames.len(), durations_ms.len()));
    }
    let pngs = frames.iter().map(|i| i.to_png().map_err(|e| e.to_string())).collect::<Result<Vec<_>>>()?;
    let (w, h) = src.frames.first().map(|i| (i.width, i.height)).unwrap_or((0, 0));
    let meta = Entry { src: src.hash.clone(), backend: be.id(), factor, algo: ALGO, canvas: [w, h], durations_ms, frames: Vec::new() };
    Ok(OwnedEntry { key: src.key.clone(), meta, pngs })
}

pub struct Settings {
    pub cache: PathBuf,
    pub factor: u32,
    pub workers: usize,
    pub regenerate: bool,
    pub filter: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug)]
pub enum Progress {
    /// `all` = every animation in scope before `limit`
    Start { total: usize, all: usize, reusable: usize, backend: String },
    Item { key: String, generated: bool, done: usize, total: usize, ms: u128 },
    Failed { key: String, err: String },
}

#[derive(Debug, Default)]
pub struct Report {
    pub total: usize,
    pub generated: usize,
    pub reused: usize,
    pub failed: usize,
    pub cancelled: bool,
    pub bytes: u64,
    pub entries: usize,
    pub secs: f32,
    pub backend: String,
}

fn valid(e: &Entry, src: &str, be: &str, factor: u32) -> bool {
    e.src == src && e.backend == be && e.factor == factor && e.algo == ALGO
}

fn part_path(stage: &Path, key: &str) -> PathBuf {
    stage.join(format!("{}.part", key.replace('/', "_")))
}

/// Run the prepare step.
pub fn prepare(vfs: &Vfs, s: &Settings, be: &dyn Backend, ctl: &Control, on: &mut dyn FnMut(Progress)) -> Result<Report> {
    let t0 = Instant::now();
    let stage = s.cache.with_file_name("staging");
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let old = if s.cache.is_file() { SmoothPak::open(&s.cache).ok() } else { None };
    let mut jobs = jobs(vfs, s.filter.as_deref());
    let all = jobs.len();
    if let Some(l) = s.limit {
        jobs.truncate(l);
    }
    let total = jobs.len();
    let bid = be.id();
    let reusable = old.as_ref().map(|o| jobs.iter().filter(|j| o.get(&j.key()).is_some_and(|e| e.backend == bid && e.factor == s.factor && e.algo == ALGO)).count()).unwrap_or(0);
    on(Progress::Start { total, all, reusable: if s.regenerate { 0 } else { reusable }, backend: bid.clone() });
    let queue = Mutex::new(jobs.into_iter().collect::<VecDeque<_>>());
    let (tx, rx) = mpsc::channel::<(String, std::result::Result<(OwnedEntry, bool), String>, u128)>();
    let done = AtomicUsize::new(0);
    let mut results: HashMap<String, OwnedEntry> = HashMap::new();
    let mut rep = Report { total, backend: bid.clone(), ..Default::default() };
    std::thread::scope(|sc| {
        for _ in 0..s.workers.max(1) {
            let tx = tx.clone();
            let (queue, old, stage, bid) = (&queue, &old, &stage, &bid);
            sc.spawn(move || loop {
                while ctl.paused.load(Ordering::Relaxed) && !ctl.cancel.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(200));
                }
                if ctl.cancel.load(Ordering::Relaxed) {
                    break;
                }
                let Some(j) = queue.lock().unwrap().pop_front() else { break };
                let t = Instant::now();
                let key = j.key();
                let r = (|| -> std::result::Result<(OwnedEntry, bool), String> {
                    let src = source(vfs, &j)?;
                    if !s.regenerate {
                        if let Some(o) = old.as_ref().filter(|o| o.get(&key).is_some_and(|e| valid(e, &src.hash, bid, s.factor))) {
                            return Ok((OwnedEntry::from_pak(o, &key).map_err(|e| e.to_string())?, false));
                        }
                        let pp = part_path(stage, &key);
                        if let Ok(p) = SmoothPak::open(&pp) {
                            if p.get(&key).is_some_and(|e| valid(e, &src.hash, bid, s.factor)) {
                                return Ok((OwnedEntry::from_pak(&p, &key).map_err(|e| e.to_string())?, false));
                            }
                        }
                    }
                    let e = generate(&src, be, s.factor)?;
                    smoothpak::write(&part_path(stage, &key), std::slice::from_ref(&e), BTreeMap::new()).map_err(|e| e.to_string())?;
                    Ok((e, true))
                })();
                let _ = tx.send((key, r, t.elapsed().as_millis()));
            });
        }
        drop(tx);
        for (key, r, ms) in rx {
            match r {
                Ok((e, generated)) => {
                    let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                    if generated { rep.generated += 1 } else { rep.reused += 1 }
                    on(Progress::Item { key: key.clone(), generated, done: d, total, ms });
                    results.insert(key, e);
                }
                Err(err) => {
                    rep.failed += 1;
                    on(Progress::Failed { key, err });
                }
            }
        }
    });
    rep.cancelled = ctl.cancel.load(Ordering::Relaxed);
    // keep old entries that were not part of this run (other filters / not reached before cancel)
    if let Some(o) = &old {
        for k in o.manifest.entries.keys() {
            if !results.contains_key(k) {
                if let Ok(e) = OwnedEntry::from_pak(o, k) {
                    results.insert(k.clone(), e);
                }
            }
        }
    }
    let mut all: Vec<OwnedEntry> = results.into_values().collect();
    all.sort_by(|a, b| a.key.cmp(&b.key));
    let mut info = BTreeMap::new();
    info.insert("backend".into(), bid);
    info.insert("factor".into(), s.factor.to_string());
    info.insert("algo".into(), ALGO.to_string());
    // complete = a full-scope run finished without errors (a sample / cancelled run keeps the old flag)
    let full = s.filter.is_none() && s.limit.is_none();
    let was = old.as_ref().is_some_and(|o| o.manifest.info.get("complete").map(String::as_str) == Some("1"));
    let complete = if full { !rep.cancelled && rep.failed == 0 } else { was };
    info.insert("complete".into(), if complete { "1" } else { "0" }.into());
    drop(old);
    rep.entries = all.len();
    rep.bytes = smoothpak::write(&s.cache, &all, info).map_err(|e| e.to_string())?;
    for e in &all {
        let _ = std::fs::remove_file(part_path(&stage, &e.key));
    }
    rep.secs = t0.elapsed().as_secs_f32();
    Ok(rep)
}

/// Default cache location.
pub fn default_cache() -> PathBuf {
    newpal_assets::gamedir::user_data_dir().join("cache").join("smooth.pak")
}
pub fn default_tools() -> PathBuf {
    newpal_assets::gamedir::user_data_dir().join("tools")
}

pub use backend::{CpuCrossfade, MockRifeRunner, ProcessRunner, Rife};

/// Build the RIFE backend from an installed / given exe.
pub fn rife_backend(exe: PathBuf, model_dir: PathBuf, work: PathBuf) -> Rife<ProcessRunner> {
    Rife { exe, model_dir, gpu: 0, work, runner: ProcessRunner, version: rife::VERSION.into() }
}

#[allow(dead_code)]
fn _assert_sync() {
    fn s<T: Sync>() {}
    s::<Vfs>();
    s::<Arc<SmoothPak>>();
}