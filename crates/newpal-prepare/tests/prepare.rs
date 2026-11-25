//! Prepare step on a tiny sample of the player's own data (skipped without an install). CPU / mock only.
use newpal_assets::gamedir::{self, Config};
use newpal_assets::smoothpak::SmoothPak;
use newpal_assets::vfs::Vfs;
use newpal_prepare::*;
use std::sync::atomic::Ordering;

fn vfs() -> Option<Vfs> {
    let p = Config::default_path();
    let cfg = Config::load_from(&p).unwrap_or_default();
    gamedir::resolve(None, &cfg, &p).ok().map(|r| Vfs::new(r.dir, Vec::new()))
}

fn settings(dir: &std::path::Path, factor: u32) -> Settings {
    Settings { cache: dir.join("cache").join("smooth.pak"), factor, workers: 2, regenerate: false, filter: Some("all_char:0".into()), limit: Some(3) }
}

#[test]
fn sample_prepare_incremental_resume_cancel() {
    let Some(v) = vfs() else { eprintln!("SKIP: no game"); return };
    let dir = std::env::temp_dir().join(format!("np-prep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ctl = Control::default();
    let mut log = Vec::new();
    let s = settings(&dir, 2);
    let r = prepare(&v, &s, &CpuCrossfade, &ctl, &mut |p| log.push(format!("{p:?}"))).unwrap();
    eprintln!("first: {r:?}");
    assert_eq!((r.total, r.generated, r.reused, r.failed), (3, 3, 0, 0));
    let pak = SmoothPak::open(&s.cache).unwrap();
    let e = pak.get("all_char/0/8").expect("hero walk N prepared");
    assert_eq!(e.frames.len(), 16); // 8 frames x2
    assert!((e.durations_ms.iter().sum::<f32>() - 840.0).abs() < 0.1, "loop length kept");
    assert_eq!(pak.manifest.info.get("complete").map(String::as_str), Some("0"), "sample run is not complete");
    drop(pak);
    // runtime lookup through the VFS: cache used, factor must match
    let mut v2 = Vfs::new(v.game_dir().to_path_buf(), Vec::new());
    v2.set_smooth_cache(Some(std::sync::Arc::new(SmoothPak::open(&s.cache).unwrap())), 2);
    let (m, tex) = v2.anim_override("all_char", 0, 8).unwrap().unwrap();
    assert_eq!(tex.len(), 16);
    assert_eq!(m.durations(16, 0.84).len(), 16);
    v2.set_smooth_cache(Some(std::sync::Arc::new(SmoothPak::open(&s.cache).unwrap())), 4);
    assert!(v2.anim_override("all_char", 0, 8).unwrap().is_none());
    // second run: everything reused (incremental)
    let r = prepare(&v, &s, &CpuCrossfade, &ctl, &mut |_| {}).unwrap();
    assert_eq!((r.generated, r.reused), (0, 3));
    // resume: a staged part from a killed run is picked up
    let stage = s.cache.with_file_name("staging");
    let src = source(&v, &Job { act: "all_char".into(), slot: 0, action: 11 }).unwrap();
    let part = generate(&src, &CpuCrossfade, 2).unwrap();
    newpal_assets::smoothpak::write(&stage.join("all_char_0_11.part"), &[part], Default::default()).unwrap();
    let mut s4 = settings(&dir, 2);
    s4.limit = Some(4);
    let r = prepare(&v, &s4, &CpuCrossfade, &ctl, &mut |_| {}).unwrap();
    assert_eq!((r.generated, r.reused), (0, 4), "{r:?}");
    // factor change / regenerate -> recompute
    let r = prepare(&v, &settings(&dir, 4), &CpuCrossfade, &ctl, &mut |_| {}).unwrap();
    assert_eq!(r.generated, 3);
    let mut sr = settings(&dir, 4);
    sr.regenerate = true;
    assert_eq!(prepare(&v, &sr, &CpuCrossfade, &ctl, &mut |_| {}).unwrap().generated, 3);
    // cancel: no work, pack still written with previous entries
    ctl.cancel.store(true, Ordering::Relaxed);
    let r = prepare(&v, &settings(&dir, 4), &CpuCrossfade, &ctl, &mut |_| {}).unwrap();
    assert!(r.cancelled && r.generated == 0 && r.entries >= 4, "{r:?}");
    assert!(log.iter().any(|l| l.contains("Start")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mock_rife_backend_on_real_anim() {
    let Some(v) = vfs() else { return };
    let dir = std::env::temp_dir().join(format!("np-prep-rife-{}", std::process::id()));
    let be = Rife { exe: "rife-ncnn-vulkan.exe".into(), model_dir: dir.join("rife-v4.6"), gpu: 0, work: dir.join("work"), runner: MockRifeRunner, version: "mock-rife".into() };
    let mut s = settings(&dir, 4);
    s.filter = Some("all_char:0:8".into());
    let r = prepare(&v, &s, &be, &Control::default(), &mut |_| {}).unwrap();
    assert_eq!(r.generated, 1);
    let pak = SmoothPak::open(&s.cache).unwrap();
    let e = pak.get("all_char/0/8").unwrap();
    assert_eq!((e.frames.len(), e.backend.as_str()), (32, "mock-rife/rife-v4.6"));
    drop(pak);
    let _ = std::fs::remove_dir_all(&dir);
}