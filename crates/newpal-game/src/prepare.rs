//! Launcher step 2 (after the game folder is validated): prepare smooth-animation frames locally.
use crate::Args;
use newpal_assets::gamedir::Config;
use newpal_assets::smoothpak::SmoothPak;
use newpal_assets::vfs::Vfs;
use newpal_prepare::{self as prep, backend::Backend, Control, Progress, Settings};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub struct SmoothOpts {
    pub backend: String,
    pub quality: u32,
    pub workers: usize,
    pub rife_path: Option<PathBuf>,
    pub cache: PathBuf,
}

impl SmoothOpts {
    pub fn resolve(args: &Args, cfg: &Config) -> Self {
        let c = &cfg.smooth;
        let q = args.smooth_quality.or(c.quality).unwrap_or(4);
        SmoothOpts {
            backend: args.smooth_backend.clone().or(c.backend.clone()).unwrap_or_else(|| "auto".into()).to_lowercase(),
            quality: if q >= 4 { 4 } else { 2 },
            workers: args.workers.or(c.workers).unwrap_or(2).clamp(1, 16),
            rife_path: c.rife_path.clone(),
            cache: prep::default_cache(),
        }
    }
}

/// A Vulkan GPU that is not a software rasterizer (probe only, no work submitted).
pub fn vulkan_gpu() -> Option<String> {
    if !prep::rife::vulkan_loader_present() {
        return None;
    }
    let mut d = wgpu::InstanceDescriptor::new_without_display_handle();
    d.backends = wgpu::Backends::VULKAN;
    let inst = wgpu::Instance::new(d);
    let a = pollster::block_on(inst.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() })).ok()?;
    let i = a.get_info();
    (i.device_type != wgpu::DeviceType::Cpu).then(|| i.name)
}

fn backend(o: &SmoothOpts, log: &mut dyn FnMut(String)) -> Result<Option<Box<dyn Backend>>, String> {
    let tools = prep::default_tools();
    let rife = |log: &mut dyn FnMut(String)| -> Result<Box<dyn Backend>, String> {
        let (exe, model) = match &o.rife_path {
            Some(p) => (p.clone(), p.parent().unwrap_or(Path::new(".")).join(prep::rife::MODEL)),
            None => prep::rife::ensure(&tools, log)?,
        };
        Ok(Box::new(prep::rife_backend(exe, model, tools.join("work"))))
    };
    Ok(match o.backend.as_str() {
        "off" => None,
        "cpu" => Some(Box::new(prep::CpuCrossfade)),
        // test double with RIFE's CLI contract (no GPU); for checking the pipeline
        "mock-rife" => Some(Box::new(prep::Rife { exe: "mock".into(), model_dir: tools.join(prep::rife::MODEL), gpu: 0, work: tools.join("work"), runner: prep::MockRifeRunner, version: "mock-rife".into() })),
        "gpu" => Some(rife(log)?),
        _ => match vulkan_gpu() {
            Some(name) => {
                log(format!("Vulkan GPU: {name} -> RIFE"));
                match rife(log) {
                    Ok(b) => Some(b),
                    Err(e) => {
                        log(format!("RIFE unavailable ({e}); falling back to CPU cross-fade"));
                        Some(Box::new(prep::CpuCrossfade))
                    }
                }
            }
            None => {
                log("no Vulkan GPU -> CPU cross-fade".into());
                Some(Box::new(prep::CpuCrossfade))
            }
        },
    })
}

/// Is the cache missing or made with other settings / algorithm? (Source changes are found by --prepare.)
pub fn needs_prepare(o: &SmoothOpts) -> bool {
    if o.backend == "off" {
        return false;
    }
    match SmoothPak::open(&o.cache) {
        Ok(p) => p.manifest.info.get("factor").map(String::as_str) != Some(&o.quality.to_string()) || p.manifest.info.get("algo").map(String::as_str) != Some(&prep::ALGO.to_string())
            || p.manifest.info.get("complete").map(String::as_str) != Some("1"),
        Err(_) => true,
    }
}

/// Run preparation with console progress. Type `p`+Enter to pause/resume, `q`+Enter to cancel
/// (a cancelled or killed run resumes where it stopped).
pub fn run(game: &Path, o: &SmoothOpts, regenerate: bool, filter: Option<String>, limit: Option<usize>) -> Result<(), String> {
    let mut log = |s: String| eprintln!("[prepare] {s}");
    let Some(be) = backend(o, &mut log)? else {
        log("smooth backend off".into());
        return Ok(());
    };
    let vfs = Vfs::new(game.to_path_buf(), Vec::new()); // original data only; mods are applied at runtime
    let ctl: &'static Control = Box::leak(Box::default());
    std::thread::spawn(move || {
        let mut line = String::new();
        while std::io::stdin().read_line(&mut line).map(|n| n > 0).unwrap_or(false) {
            match line.trim() {
                "p" => {
                    let p = !ctl.paused.load(Ordering::Relaxed);
                    ctl.paused.store(p, Ordering::Relaxed);
                    eprintln!("\n[prepare] {}", if p { "paused (p = resume)" } else { "resumed" });
                }
                "q" => ctl.cancel.store(true, Ordering::Relaxed),
                _ => {}
            }
            line.clear();
        }
    });
    let s = Settings { cache: o.cache.clone(), factor: o.quality, workers: o.workers, regenerate, filter, limit };
    eprintln!("[prepare] cache {} | x{} | {} workers | backend {} | p=pause q=cancel", o.cache.display(), o.quality, o.workers, be.id());
    let t0 = Instant::now();
    let (mut gen_ms, mut gen_n, mut all) = (0u128, 0usize, 0usize);
    let r = prep::prepare(&vfs, &s, be.as_ref(), ctl, &mut |p| match p {
        Progress::Start { total, all: a, reusable, .. } => {
            all = a;
            eprintln!("[prepare] {total} animations ({a} in scope), {reusable} reusable");
        }
        Progress::Item { key, generated, done, total, ms } => {
            if generated {
                gen_ms += ms;
                gen_n += 1;
            }
            let el = t0.elapsed().as_secs_f32();
            let eta = if done > 0 { el / done as f32 * (total - done) as f32 } else { 0.0 };
            eprint!("\r[prepare] {done:>5}/{total} {:5.1}% {key:<18} {} {ms:>5} ms  ETA {:.0}s   ", done as f32 * 100.0 / total.max(1) as f32, if generated { "gen" } else { "hit" }, eta);
            let _ = std::io::stderr().flush();
        }
        Progress::Failed { key, err } => eprintln!("\n[prepare] {key}: {err}"),
    })?;
    eprintln!("\n[prepare] {} generated, {} reused, {} failed{} in {:.1}s -> {} entries, {:.1} MB",
        r.generated, r.reused, r.failed, if r.cancelled { " (cancelled, resumable)" } else { "" }, r.secs, r.entries, r.bytes as f64 / 1e6);
    if gen_n > 0 && r.entries > 0 {
        let per = gen_ms as f64 / gen_n as f64 / o.workers as f64;
        eprintln!("[prepare] estimate for all {all}: ~{:.0} s with this backend, ~{:.0} MB", per * all as f64 / 1000.0, r.bytes as f64 / r.entries as f64 * all as f64 / 1e6);
    }
    Ok(())
}

/// Open the cache for runtime lookup (user mods > prepared cache > original frames).
pub fn open(o: &SmoothOpts) -> Option<Arc<SmoothPak>> {
    if o.backend == "off" {
        return None;
    }
    SmoothPak::open(&o.cache).ok().map(Arc::new)
}