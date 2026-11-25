//! Full static 4x HD stills: dump unique sprite frames + composed map backgrounds,
//! run realesrgan-ncnn-vulkan in directory mode, merge alpha, write the per-user cache.
//!
//! Releases ship no game art. Output: `%LOCALAPPDATA%/NewPAL-Reborn/cache/hd`.
//!
//! Resumable: dest `@4x.png` files that already exist are skipped (unless `regenerate`).
//! Already-upscaled plates under `hd-work/out` are merged without re-running ESRGAN (partial batches).
//! A `manifest.json` is updated after each merged batch.
//! Merge runs on a background thread so the next ESRGAN job can start immediately.
//! A lock file prevents a second copy from starting while one is alive.
use newpal_assets::image::RgbaImage;
use newpal_assets::tsw::SpriteEntry;
use newpal_assets::vfs::Vfs;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const SCALE: u32 = 4;
pub const MODEL: &str = "realesrgan-x4plus-anime";
pub const ALGO: u32 = 1;
pub const TSW: &[&str] = &["all_char", "all_item", "all_magic", "all_map1", "all_map2", "all_sys"];
pub const LMF: &[&str] = &["huge", "huge2"];
const BATCH: usize = 1500;
const MAX_JOBS: usize = 2;
const MAX_CHILDREN: usize = 3;
const BLEED_R: u32 = 3;

#[derive(Clone, Debug)]
pub struct HdBatchOpts {
    pub game: PathBuf,
    pub out: PathBuf,
    pub tools: PathBuf,
    pub skip_maps: bool,
    pub only: Option<String>,
    pub regenerate: bool,
}

impl HdBatchOpts {
    pub fn default_out() -> PathBuf {
        newpal_assets::gamedir::user_data_dir().join("cache").join("hd")
    }

    pub fn default_tools() -> PathBuf {
        newpal_assets::gamedir::user_data_dir()
            .join("tools")
            .join("realesrgan-ncnn-vulkan-20220424-windows")
    }
}

/// Parse `hd_batch` / `--hd-prepare` CLI flags (`--game-dir` required for the example).
pub fn opts_from_args<I: Iterator<Item = String>>(mut it: I) -> Result<HdBatchOpts, String> {
    let mut game = None;
    let mut out = None;
    let mut tools = None;
    let mut skip_maps = false;
    let mut only = None;
    let mut regenerate = false;
    while let Some(k) = it.next() {
        match k.as_str() {
            "--game-dir" => game = Some(PathBuf::from(it.next().ok_or("--game-dir")?)),
            "--out" | "--hd-cache" => out = Some(PathBuf::from(it.next().ok_or("--out")?)),
            "--data-root" => {
                let root = PathBuf::from(it.next().ok_or("--data-root")?);
                std::env::set_var("NEWPAL_DATA_ROOT", &root);
                if out.is_none() {
                    out = Some(root.join("cache").join("hd"));
                }
                if tools.is_none() {
                    tools = Some(root.join("tools").join("realesrgan-ncnn-vulkan-20220424-windows"));
                }
            }
            "--tools" => tools = Some(PathBuf::from(it.next().ok_or("--tools")?)),
            "--skip-maps" => skip_maps = true,
            "--only" => only = Some(it.next().ok_or("--only")?),
            "--regenerate" => regenerate = true,
            other => return Err(format!("unknown {other}")),
        }
    }
    let game = game.ok_or("--game-dir required")?;
    Ok(HdBatchOpts {
        game,
        out: out.unwrap_or_else(HdBatchOpts::default_out),
        tools: tools.unwrap_or_else(HdBatchOpts::default_tools),
        skip_maps,
        only,
        regenerate,
    })
}

pub fn run_from_env() -> Result<(), String> {
    run(opts_from_args(std::env::args().skip(1))?)
}

pub fn run(opts: HdBatchOpts) -> Result<(), String> {
    let exe = opts.tools.join("realesrgan-ncnn-vulkan.exe");
    let models = opts.tools.join("models");
    if !exe.is_file() {
        return Err(format!("missing {}", exe.display()));
    }
    let work = opts.out.parent().unwrap_or(&opts.out).join("hd-work");
    std::fs::create_dir_all(&opts.out).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;

    let _lock = acquire_lock(&work)?;

    let t0 = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));

    let jobs = Arc::new(Mutex::new(VecDeque::<Job>::new()));
    let running = Arc::new(Mutex::new(Vec::<Running>::new()));
    let done_up = Arc::new(AtomicUsize::new(0));
    let skipped = Arc::new(AtomicUsize::new(0));
    let launched = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(Mutex::new(Vec::<String>::new()));
    let inflight_merge = Arc::new(AtomicUsize::new(0));

    let manifest_path = opts.out.join("manifest.json");
    let manifest = Arc::new(Mutex::new(Manifest::load_or_new(&manifest_path, &opts)));

    let (merge_tx, merge_rx) = mpsc::channel::<MergeTask>();
    let work_m = work.clone();
    let out_m = opts.out.clone();
    let done_m = done_up.clone();
    let failed_m = failed.clone();
    let inflight_m = inflight_merge.clone();
    let manifest_m = manifest.clone();
    let manifest_path_m = manifest_path.clone();
    let merger = std::thread::spawn(move || {
        while let Ok(task) = merge_rx.recv() {
            let (label, res, batch_id) = match task {
                MergeTask::Batch { id, kind, label } => {
                    (label.clone(), merge_batch(&work_m, &out_m, id, kind), Some(id))
                }
                MergeTask::ReuseSprite { name, rgb, al } => {
                    (name.clone(), merge_reuse_sprite(&out_m, &name, &rgb, &al), None)
                }
                MergeTask::ReuseMap { name, path } => {
                    (name.clone(), merge_reuse_map(&out_m, &name, &path), None)
                }
            };
            match res {
                Ok(n) => {
                    done_m.fetch_add(n, Ordering::Relaxed);
                    eprintln!("[esrgan] {} merged {n} -> {}", label, out_m.display());
                    if let Ok(mut man) = manifest_m.lock() {
                        if let Some(id) = batch_id {
                            man.note_batch(id, Kind::Sprite, n);
                        }
                        man.merged = done_m.load(Ordering::Relaxed);
                        let _ = man.save(&manifest_path_m);
                    }
                }
                Err(e) => {
                    eprintln!("[esrgan] {label} merge: {e}");
                    failed_m.lock().unwrap().push(format!("{label} merge: {e}"));
                }
            }
            inflight_m.fetch_sub(1, Ordering::Relaxed);
        }
    });

    let existing = index_existing_outs(&work);
    eprintln!(
        "[hd-batch] resume index: dest will be checked per file; leftover plates rgb={} alpha={} maps={}",
        existing.rgb.len(),
        existing.alpha.len(),
        existing.maps.len()
    );

    let vfs_game = opts.game.clone();
    let work2 = work.clone();
    let out2 = opts.out.clone();
    let jobs2 = jobs.clone();
    let stop2 = stop.clone();
    let skip_maps2 = opts.skip_maps;
    let only2 = opts.only.clone();
    let regen2 = opts.regenerate;
    let skipped2 = skipped.clone();
    let reused2 = Arc::new(AtomicUsize::new(0));
    let reused_d = reused2.clone();
    let merge_tx_d = merge_tx.clone();
    let inflight_d = inflight_merge.clone();
    let existing_d = existing;
    let dumper = std::thread::spawn(move || -> Result<(usize, usize), String> {
        let vfs = Vfs::new(vfs_game, Vec::new());
        let mut n_spr = 0usize;
        let mut n_map = 0usize;
        let mut batch: Vec<Item> = Vec::new();
        let mut bid = 0u32;
        let flush = |batch: &mut Vec<Item>, bid: &mut u32, jobs: &Mutex<VecDeque<Job>>| -> Result<(), String> {
            if batch.is_empty() {
                return Ok(());
            }
            let id = *bid;
            *bid += 1;
            let kind = batch[0].kind;
            let n = batch.len();
            write_batch(&work2, id, kind, batch)?;
            jobs.lock().unwrap().push_back(Job { id, kind, n });
            eprintln!("[dump] batch {id} {:?} {n} images queued", kind);
            batch.clear();
            Ok(())
        };

        if only2.as_deref() != Some("maps") {
            for tsw in TSW {
                if stop2.load(Ordering::Relaxed) {
                    break;
                }
                if let Some(o) = only2.as_deref() {
                    if o != *tsw && o != "sprites" {
                        continue;
                    }
                }
                eprintln!("[dump] tsw {tsw}");
                let pak = vfs.pak(&format!("{tsw}.tsw")).map_err(|e| e.to_string())?;
                for e in &pak.entries {
                    if stop2.load(Ordering::Relaxed) {
                        break;
                    }
                    if e.size == 0 {
                        continue;
                    }
                    let Ok(raw) = pak.read(e.slot) else { continue };
                    let Ok(spr) = SpriteEntry::parse(raw) else { continue };
                    let id = if e.id == 0 { e.slot as u32 + 1 } else { e.id };
                    for fi in 0..spr.frame_count() {
                        let Ok(img) = spr.frame_image(fi) else { continue };
                        if img.width == 0 || img.height == 0 {
                            continue;
                        }
                        let name = format!("s__{tsw}__{id}__{fi:04}.png");
                        let dest = dest_from_name(&out2, &name)?;
                        if !regen2 && dest.is_file() {
                            skipped2.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        if !regen2 {
                            if let (Some(rgb), Some(al)) =
                                (existing_d.rgb.get(&name), existing_d.alpha.get(&name))
                            {
                                inflight_d.fetch_add(1, Ordering::Relaxed);
                                if merge_tx_d
                                    .send(MergeTask::ReuseSprite {
                                        name: name.clone(),
                                        rgb: rgb.clone(),
                                        al: al.clone(),
                                    })
                                    .is_err()
                                {
                                    inflight_d.fetch_sub(1, Ordering::Relaxed);
                                } else {
                                    reused_d.fetch_add(1, Ordering::Relaxed);
                                }
                                continue;
                            }
                        }
                        batch.push(Item { name, img, kind: Kind::Sprite });
                        n_spr += 1;
                        if batch.len() >= BATCH {
                            flush(&mut batch, &mut bid, &jobs2)?;
                        }
                    }
                }
                eprintln!(
                    "[dump] {tsw} running total sprites {n_spr} skipped {}",
                    skipped2.load(Ordering::Relaxed)
                );
            }
            if !batch.is_empty() {
                flush(&mut batch, &mut bid, &jobs2)?;
            }
        }

        if !skip_maps2 && only2.as_deref() != Some("sprites") {
            for lmf in LMF {
                if stop2.load(Ordering::Relaxed) {
                    break;
                }
                if let Some(o) = only2.as_deref() {
                    if o != *lmf && o != "maps" {
                        continue;
                    }
                }
                eprintln!("[dump] lmf {lmf}");
                let arc = vfs.lmf(&format!("{lmf}.lmf")).map_err(|e| e.to_string())?;
                let mut ids: Vec<u32> = arc.maps.keys().copied().collect();
                ids.sort_unstable();
                for mid in ids {
                    if stop2.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(map) = arc.load_map(mid) else { continue };
                    let img = map.render_bg();
                    if img.width == 0 || img.height == 0 {
                        continue;
                    }
                    let name = format!("m__{lmf}__{mid}__bg.png");
                    let dest = dest_from_name(&out2, &name)?;
                    if !regen2 && dest.is_file() {
                        skipped2.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    if !regen2 {
                        if let Some(path) = existing_d.maps.get(&name) {
                            inflight_d.fetch_add(1, Ordering::Relaxed);
                            if merge_tx_d
                                .send(MergeTask::ReuseMap {
                                    name: name.clone(),
                                    path: path.clone(),
                                })
                                .is_err()
                            {
                                inflight_d.fetch_sub(1, Ordering::Relaxed);
                            } else {
                                reused_d.fetch_add(1, Ordering::Relaxed);
                            }
                            continue;
                        }
                    }
                    batch.push(Item { name, img, kind: Kind::Map });
                    n_map += 1;
                    if batch.len() >= 8 {
                        flush(&mut batch, &mut bid, &jobs2)?;
                    }
                }
                eprintln!("[dump] {lmf} maps so far {n_map}");
            }
            if !batch.is_empty() {
                flush(&mut batch, &mut bid, &jobs2)?;
            }
        }
        Ok((n_spr, n_map))
    });

    loop {
        reap(
            &mut running.lock().unwrap(),
            &merge_tx,
            &inflight_merge,
            &failed,
        );
        let nrun = running.lock().unwrap().len();
        let nchild: usize = running.lock().unwrap().iter().map(|r| r.children.len()).sum();
        let peek = jobs
            .lock()
            .unwrap()
            .front()
            .map(|j| if matches!(j.kind, Kind::Sprite) { 2 } else { 1 })
            .unwrap_or(0);
        if nrun < MAX_JOBS && peek > 0 && nchild + peek <= MAX_CHILDREN {
            if let Some(job) = jobs.lock().unwrap().pop_front() {
                match spawn_job(&exe, &models, &work, &job) {
                    Ok(r) => {
                        launched.fetch_add(1, Ordering::Relaxed);
                        eprintln!(
                            "[esrgan] start batch {} {:?} n={} (jobs {} children {})",
                            job.id,
                            job.kind,
                            job.n,
                            nrun + 1,
                            nchild + r.children.len()
                        );
                        running.lock().unwrap().push(r);
                    }
                    Err(e) => {
                        eprintln!("[esrgan] spawn batch {}: {e}", job.id);
                        failed.lock().unwrap().push(format!("b{:04} spawn: {e}", job.id));
                    }
                }
                continue;
            }
        }
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if dumper.is_finished()
            && jobs.lock().unwrap().is_empty()
            && running.lock().unwrap().is_empty()
            && inflight_merge.load(Ordering::Relaxed) == 0
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    let dump_res = dumper.join().unwrap_or_else(|_| Err("dumper panic".into()));
    // last reap + drain merges
    let tjoin = Instant::now();
    while tjoin.elapsed() < Duration::from_secs(86400) {
        reap(
            &mut running.lock().unwrap(),
            &merge_tx,
            &inflight_merge,
            &failed,
        );
        if running.lock().unwrap().is_empty() && inflight_merge.load(Ordering::Relaxed) == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    drop(merge_tx);
    let _ = merger.join();

    let (n_spr, n_map) = dump_res?;
    let n_fail = failed.lock().unwrap().len();
    let n_skip = skipped.load(Ordering::Relaxed);
    let n_reuse = reused2.load(Ordering::Relaxed);
    {
        let mut man = manifest.lock().unwrap();
        man.sprites_queued = n_spr;
        man.maps_queued = n_map;
        man.skipped = n_skip;
        man.merged = done_up.load(Ordering::Relaxed);
        man.failed = failed.lock().unwrap().clone();
        man.finished = true;
        man.elapsed_s = t0.elapsed().as_secs_f32();
        man.touch();
        man.save(&manifest_path)?;
    }
    eprintln!(
        "[hd-batch] sprites {n_spr} maps {n_map} skipped {n_skip} reused {n_reuse} launched {} merged {} failed {n_fail} in {:.1}s -> {}",
        launched.load(Ordering::Relaxed),
        done_up.load(Ordering::Relaxed),
        t0.elapsed().as_secs_f32(),
        opts.out.display()
    );
    if n_fail > 0 {
        return Err(format!("{n_fail} batch(es) failed; see log and {}", manifest_path.display()));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Sprite,
    Map,
}

struct Item {
    name: String,
    img: RgbaImage,
    kind: Kind,
}
struct Job {
    id: u32,
    kind: Kind,
    n: usize,
}
struct Running {
    id: u32,
    kind: Kind,
    children: Vec<std::process::Child>,
    label: String,
}
enum MergeTask {
    Batch { id: u32, kind: Kind, label: String },
    ReuseSprite { name: String, rgb: PathBuf, al: PathBuf },
    ReuseMap { name: String, path: PathBuf },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    algo: u32,
    scale: u32,
    model: String,
    updated_unix: u64,
    sprites_queued: usize,
    maps_queued: usize,
    skipped: usize,
    merged: usize,
    elapsed_s: f32,
    finished: bool,
    batches_done: Vec<u32>,
    failed: Vec<String>,
}

impl Manifest {
    fn load_or_new(path: &Path, opts: &HdBatchOpts) -> Self {
        if !opts.regenerate {
            if let Ok(b) = std::fs::read(path) {
                if let Ok(m) = serde_json::from_slice::<Manifest>(&b) {
                    if m.algo == ALGO && m.model == MODEL && m.scale == SCALE {
                        return m;
                    }
                }
            }
        }
        Self {
            version: 1,
            algo: ALGO,
            scale: SCALE,
            model: MODEL.to_string(),
            updated_unix: now_unix(),
            sprites_queued: 0,
            maps_queued: 0,
            skipped: 0,
            merged: 0,
            elapsed_s: 0.0,
            finished: false,
            batches_done: Vec::new(),
            failed: Vec::new(),
        }
    }

    fn note_batch(&mut self, id: u32, _kind: Kind, _n: usize) {
        if !self.batches_done.contains(&id) {
            self.batches_done.push(id);
        }
        self.touch();
    }

    fn touch(&mut self) {
        self.updated_unix = now_unix();
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        let s = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, s).map_err(|e| e.to_string())
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

struct LockGuard {
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn acquire_lock(work: &Path) -> Result<LockGuard, String> {
    let path = work.join("hd_batch.lock");
    if path.is_file() {
        if let Ok(s) = std::fs::read_to_string(&path) {
            if let Ok(pid) = s.trim().parse::<u32>() {
                if pid_alive(pid) {
                    return Err(format!(
                        "hd_batch already running (pid {pid}); not starting a second copy"
                    ));
                }
            }
        }
    }
    if esrgan_running() {
        return Err("realesrgan-ncnn-vulkan is already running; not starting a second copy".into());
    }
    std::fs::write(&path, std::process::id().to_string()).map_err(|e| e.to_string())?;
    Ok(LockGuard { path })
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let out = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output();
        match out {
            Ok(o) => {
                let s = String::from_utf8_lossy(&o.stdout);
                s.contains(&pid.to_string()) && !s.to_ascii_lowercase().contains("no tasks")
            }
            Err(_) => true,
        }
    }
    #[cfg(not(windows))]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
}

fn esrgan_running() -> bool {
    let out = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq realesrgan-ncnn-vulkan.exe", "/FO", "CSV", "/NH"])
        .output();
    match out {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).to_ascii_lowercase();
            s.contains("realesrgan-ncnn-vulkan.exe") && !s.contains("no tasks")
        }
        Err(_) => false,
    }
}

fn bleed(img: &RgbaImage, radius: u32) -> RgbaImage {
    let mut o = img.clone();
    for _ in 0..radius {
        let src = o.clone();
        for y in 0..src.height {
            for x in 0..src.width {
                let i = ((y * src.width + x) * 4) as usize;
                if src.pixels[i + 3] != 0 {
                    continue;
                }
                let mut acc = [0u32; 3];
                let mut n = 0u32;
                for (dx, dy) in [(-1i32, 0), (1, 0), (0, -1), (0, 1)] {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= src.width as i32 || ny >= src.height as i32 {
                        continue;
                    }
                    let j = ((ny as u32 * src.width + nx as u32) * 4) as usize;
                    if src.pixels[j + 3] == 0 {
                        continue;
                    }
                    for c in 0..3 {
                        acc[c] += src.pixels[j + c] as u32;
                    }
                    n += 1;
                }
                if n > 0 {
                    for c in 0..3 {
                        o.pixels[i + c] = (acc[c] / n) as u8;
                    }
                }
            }
        }
    }
    for i in 0..(img.width * img.height) as usize {
        o.pixels[i * 4 + 3] = 255;
    }
    o
}

fn alpha_plate(img: &RgbaImage) -> RgbaImage {
    let mut o = RgbaImage::new(img.width, img.height);
    for i in 0..(img.width * img.height) as usize {
        let a = img.pixels[i * 4 + 3];
        o.pixels[i * 4..i * 4 + 4].copy_from_slice(&[a, a, a, 255]);
    }
    o
}

fn write_batch(work: &Path, id: u32, kind: Kind, batch: &[Item]) -> Result<(), String> {
    let tag = format!("b{id:04}");
    match kind {
        Kind::Sprite => {
            let rgb = work.join("in").join(format!("{tag}_rgb"));
            let al = work.join("in").join(format!("{tag}_a"));
            std::fs::create_dir_all(&rgb).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&al).map_err(|e| e.to_string())?;
            for it in batch {
                bleed(&it.img, BLEED_R)
                    .save_png(&rgb.join(&it.name))
                    .map_err(|e| e.to_string())?;
                alpha_plate(&it.img)
                    .save_png(&al.join(&it.name))
                    .map_err(|e| e.to_string())?;
            }
        }
        Kind::Map => {
            let rgb = work.join("in").join(format!("{tag}_map"));
            std::fs::create_dir_all(&rgb).map_err(|e| e.to_string())?;
            for it in batch {
                let mut p = it.img.clone();
                for i in 0..(p.width * p.height) as usize {
                    p.pixels[i * 4 + 3] = 255;
                }
                p.save_png(&rgb.join(&it.name)).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn spawn_esrgan(exe: &Path, models: &Path, input: &Path, output: &Path) -> Result<std::process::Child, String> {
    std::fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let mut c = Command::new(exe);
    c.args([
        "-i",
        &input.to_string_lossy(),
        "-o",
        &output.to_string_lossy(),
        "-n",
        MODEL,
        "-s",
        "4",
        "-m",
        &models.to_string_lossy(),
        "-g",
        "0",
        "-j",
        "2:4:4",
        // Auto tile (-t 0) hung realesrgan-ncnn-vulkan 20220424 on a 10x15
        // sprite in folder mode (all_sys/46/f0, 2026-11-25). Explicit 256 is
        // safe for both tiny sprites and large map plates.
        "-t",
        "256",
        "-f",
        "png",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const HIGH_PRIORITY_CLASS: u32 = 0x0000_0080;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(HIGH_PRIORITY_CLASS | CREATE_NO_WINDOW);
    }
    c.spawn().map_err(|e| format!("spawn {}: {e}", exe.display()))
}

fn spawn_job(exe: &Path, models: &Path, work: &Path, job: &Job) -> Result<Running, String> {
    let tag = format!("b{:04}", job.id);
    let mut children = Vec::new();
    match job.kind {
        Kind::Sprite => {
            children.push(spawn_esrgan(
                exe,
                models,
                &work.join("in").join(format!("{tag}_rgb")),
                &work.join("out").join(format!("{tag}_rgb")),
            )?);
            children.push(spawn_esrgan(
                exe,
                models,
                &work.join("in").join(format!("{tag}_a")),
                &work.join("out").join(format!("{tag}_a")),
            )?);
        }
        Kind::Map => {
            children.push(spawn_esrgan(
                exe,
                models,
                &work.join("in").join(format!("{tag}_map")),
                &work.join("out").join(format!("{tag}_map")),
            )?);
        }
    }
    Ok(Running {
        id: job.id,
        kind: job.kind,
        children,
        label: tag,
    })
}

fn reap(
    running: &mut Vec<Running>,
    merge_tx: &mpsc::Sender<MergeTask>,
    inflight: &AtomicUsize,
    failed: &Mutex<Vec<String>>,
) {
    let mut i = 0;
    while i < running.len() {
        let finished = running[i]
            .children
            .iter_mut()
            .all(|c| matches!(c.try_wait(), Ok(Some(_))));
        if !finished {
            i += 1;
            continue;
        }
        let r = running.remove(i);
        let mut bad = Vec::new();
        for mut c in r.children {
            if let Ok(Some(st)) = c.try_wait() {
                if !st.success() {
                    bad.push(format!("{st}"));
                }
            }
        }
        if !bad.is_empty() {
            let msg = format!("{} FAILED {}", r.label, bad.join(","));
            eprintln!("[esrgan] {msg}");
            failed.lock().unwrap().push(msg);
            continue;
        }
        inflight.fetch_add(1, Ordering::Relaxed);
        if merge_tx
            .send(MergeTask::Batch {
                id: r.id,
                kind: r.kind,
                label: r.label.clone(),
            })
            .is_err()
        {
            inflight.fetch_sub(1, Ordering::Relaxed);
            failed.lock().unwrap().push(format!("{} merge queue closed", r.label));
        }
    }
}

fn merge_batch(work: &Path, out: &Path, id: u32, kind: Kind) -> Result<usize, String> {
    let tag = format!("b{id:04}");
    let mut n = 0usize;
    match kind {
        Kind::Sprite => {
            let rgb_dir = work.join("out").join(format!("{tag}_rgb"));
            let a_dir = work.join("out").join(format!("{tag}_a"));
            let names = list_png(&rgb_dir)?;
            for name in names {
                let rgb = RgbaImage::from_png(
                    &std::fs::read(rgb_dir.join(&name)).map_err(|e| format!("{name}: {e}"))?,
                )
                .map_err(|e| e.to_string())?;
                let al = RgbaImage::from_png(
                    &std::fs::read(a_dir.join(&name)).map_err(|e| format!("alpha {name}: {e}"))?,
                )
                .map_err(|e| e.to_string())?;
                let merged = merge(&rgb, &al);
                let dest = dest_from_name(out, &name)?;
                if let Some(p) = dest.parent() {
                    std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
                }
                merged.save_png(&dest).map_err(|e| e.to_string())?;
                n += 1;
            }
        }
        Kind::Map => {
            let dir = work.join("out").join(format!("{tag}_map"));
            for name in list_png(&dir)? {
                let img = RgbaImage::from_png(
                    &std::fs::read(dir.join(&name)).map_err(|e| format!("{name}: {e}"))?,
                )
                .map_err(|e| e.to_string())?;
                let dest = dest_from_name(out, &name)?;
                if let Some(p) = dest.parent() {
                    std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
                }
                img.save_png(&dest).map_err(|e| e.to_string())?;
                n += 1;
            }
        }
    }
    Ok(n)
}

/// Merge one already-upscaled work batch into the HD cache (no realesrgan).
pub fn merge_job(work: &Path, out: &Path, id: u32, kind: Kind) -> Result<usize, String> {
    merge_batch(work, out, id, kind)
}

fn merge(rgb: &RgbaImage, al: &RgbaImage) -> RgbaImage {
    let w = rgb.width.min(al.width);
    let h = rgb.height.min(al.height);
    let mut o = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * rgb.width + x) * 4) as usize;
            let j = ((y * al.width + x) * 4) as usize;
            let k = ((y * w + x) * 4) as usize;
            let a = al.pixels[j];
            o.pixels[k] = rgb.pixels[i];
            o.pixels[k + 1] = rgb.pixels[i + 1];
            o.pixels[k + 2] = rgb.pixels[i + 2];
            o.pixels[k + 3] = if a < 8 { 0 } else { a };
        }
    }
    o
}


struct OutIndex {
    rgb: HashMap<String, PathBuf>,
    alpha: HashMap<String, PathBuf>,
    maps: HashMap<String, PathBuf>,
}

fn index_existing_outs(work: &Path) -> OutIndex {
    let mut idx = OutIndex {
        rgb: HashMap::new(),
        alpha: HashMap::new(),
        maps: HashMap::new(),
    };
    let root = work.join("out");
    let Ok(rd) = std::fs::read_dir(&root) else {
        return idx;
    };
    for e in rd.flatten() {
        let path = e.path();
        if !path.is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(files) = list_png(&path) else { continue };
        if name.ends_with("_rgb") {
            for f in files {
                idx.rgb.insert(f.clone(), path.join(&f));
            }
        } else if name.ends_with("_a") {
            for f in files {
                idx.alpha.insert(f.clone(), path.join(&f));
            }
        } else if name.ends_with("_map") {
            for f in files {
                idx.maps.insert(f.clone(), path.join(&f));
            }
        }
    }
    idx
}

fn merge_reuse_sprite(out: &Path, name: &str, rgb_p: &Path, al_p: &Path) -> Result<usize, String> {
    let rgb = RgbaImage::from_png(&std::fs::read(rgb_p).map_err(|e| format!("{name} rgb: {e}"))?)
        .map_err(|e| e.to_string())?;
    let al = RgbaImage::from_png(&std::fs::read(al_p).map_err(|e| format!("{name} alpha: {e}"))?)
        .map_err(|e| e.to_string())?;
    let merged = merge(&rgb, &al);
    let dest = dest_from_name(out, name)?;
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    merged.save_png(&dest).map_err(|e| e.to_string())?;
    Ok(1)
}

fn merge_reuse_map(out: &Path, name: &str, src: &Path) -> Result<usize, String> {
    let img = RgbaImage::from_png(&std::fs::read(src).map_err(|e| format!("{name}: {e}"))?)
        .map_err(|e| e.to_string())?;
    let dest = dest_from_name(out, name)?;
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    img.save_png(&dest).map_err(|e| e.to_string())?;
    Ok(1)
}

pub fn dest_from_name(out: &Path, name: &str) -> Result<PathBuf, String> {
    let stem = name.trim_end_matches(".png");
    let p: Vec<&str> = stem.split("__").collect();
    match p.as_slice() {
        ["s", tsw, id, fi] => {
            let fi: usize = fi.parse().map_err(|_| format!("bad frame {fi}"))?;
            Ok(out
                .join("sprites")
                .join(tsw)
                .join(id)
                .join(format!("f{fi}@4x.png")))
        }
        ["m", lmf, id, "bg"] => Ok(out.join("maps").join(lmf).join(id).join("bg@4x.png")),
        _ => Err(format!("bad name {name}")),
    }
}

fn list_png(dir: &Path) -> Result<Vec<String>, String> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let n = e.file_name().to_string_lossy().to_string();
        if n.to_ascii_lowercase().ends_with(".png") {
            v.push(n);
        }
    }
    v.sort();
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dest_paths() {
        let o = PathBuf::from("hd");
        assert_eq!(
            dest_from_name(&o, "s__all_char__1__0000.png").unwrap(),
            o.join("sprites").join("all_char").join("1").join("f0@4x.png")
        );
        assert_eq!(
            dest_from_name(&o, "m__huge__2__bg.png").unwrap(),
            o.join("maps").join("huge").join("2").join("bg@4x.png")
        );
    }
}
