//! Headless HD prototype compare: original pixel (nearest 4x) | HD static | HD + GPU mesh-warp.
//! Used by `--hd-prepare` / `--compare-hd`. Output is local only (never committed).
use crate::render::{Camera, Renderer, CAM_WORLD};
use newpal_assets::image::RgbaImage;
use newpal_assets::vfs::Vfs;
use newpal_prepare::hd::{self, SCALE};
use newpal_prepare::mesh::AnimMesh;
use std::path::Path;
use std::time::Instant;

const CLEAR: wgpu::Color = wgpu::Color { r: 0.12, g: 0.12, b: 0.14, a: 1.0 };
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const PAD: i32 = 8;
const HZ: f32 = 60.0;

fn headless_gpu() -> Result<Renderer, String> {
    let inst = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(inst.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .map_err(|e| format!("no GPU adapter: {e}"))?;
    eprintln!(
        "[hd] adapter: {} ({:?})",
        adapter.get_info().name,
        adapter.get_info().backend
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("hd-compare"),
        ..Default::default()
    }))
    .map_err(|e| format!("request_device: {e}"))?;
    Ok(Renderer::new(device, queue, wgpu::TextureFormat::Rgba8UnormSrgb))
}

struct ActionSet {
    action: usize,
    orig: Vec<RgbaImage>,
    hd: Vec<RgbaImage>,
    mesh: AnimMesh,
    durations_ms: Vec<f32>,
    tex_orig: Vec<usize>,
    tex_hd: Vec<usize>,
}

fn cycle_samples(durs: &[f32], hz: f32) -> Vec<(usize, f32)> {
    let total: f32 = durs.iter().sum();
    let n = ((total / 1000.0) * hz).round().max(1.0) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut t = (i as f32 / n as f32) * total;
        let mut placed = false;
        for (fi, d) in durs.iter().enumerate() {
            if t < *d {
                out.push((fi, (t / d).clamp(0.0, 1.0)));
                placed = true;
                break;
            }
            t -= *d;
        }
        if !placed {
            out.push((durs.len() - 1, 1.0));
        }
    }
    out
}

fn strip_samples(nframes: usize) -> Vec<(usize, f32)> {
    let mut s = Vec::new();
    for i in 0..nframes {
        for t in [0.0, 0.25, 0.5, 0.75] {
            s.push((i, t));
        }
    }
    s
}

fn compose_row(panels: &[RgbaImage], pad: i32) -> RgbaImage {
    let h = panels.iter().map(|p| p.height).max().unwrap_or(1);
    let w = panels.iter().map(|p| p.width).sum::<u32>() + (pad as u32) * (panels.len() as u32 + 1);
    let mut o = RgbaImage::new(w, h + pad as u32 * 2);
    // opaque background so APNG dispose-to-background stays a solid slate
    for px in o.pixels.chunks_exact_mut(4) {
        px.copy_from_slice(&[30, 30, 36, 255]);
    }
    let mut x = pad;
    for p in panels {
        o.blit(p, x, pad);
        x += p.width as i32 + pad;
    }
    o
}

fn compose_stack(rows: &[RgbaImage], pad: i32) -> RgbaImage {
    let w = rows.iter().map(|p| p.width).max().unwrap_or(1);
    let h = rows.iter().map(|p| p.height).sum::<u32>() + (pad as u32) * (rows.len() as u32 + 1);
    let mut o = RgbaImage::new(w + pad as u32 * 2, h);
    for px in o.pixels.chunks_exact_mut(4) {
        px.copy_from_slice(&[30, 30, 36, 255]);
    }
    let mut y = pad;
    for r in rows {
        o.blit(r, pad, y);
        y += r.height as i32 + pad;
    }
    o
}

fn draw_panel(r: &mut Renderer, set: &ActionSet, kind: Panel, fi: usize, t: f32) -> RgbaImage {
    let (lw, lh) = (set.mesh.logical_w, set.mesh.logical_h);
    r.set_camera(CAM_WORLD, Camera { origin: [0.0, 0.0], size: [lw as f32, lh as f32] });
    let n = set.hd.len();
    let fi = fi.min(n.saturating_sub(1));
    match kind {
        Panel::Orig => {
            r.push(set.tex_orig[fi], CAM_WORLD, 0.0, 0.0, lw as f32, lh as f32, WHITE);
        }
        Panel::HdStatic => {
            r.push(set.tex_hd[fi], CAM_WORLD, 0.0, 0.0, lw as f32, lh as f32, WHITE);
        }
        Panel::HdWarp => {
            // Always draw the mesh warp in the compare (runtime still cross-fades when fallback is set).
            r.push_anim_mesh(set.tex_hd[fi], CAM_WORLD, &set.mesh, fi, t, [0.0, 0.0], WHITE);
        }
    }
    r.render_to_image(lw * SCALE, lh * SCALE, CLEAR)
}

#[derive(Clone, Copy)]
enum Panel { Orig, HdStatic, HdWarp }

fn trio(r: &mut Renderer, set: &ActionSet, fi: usize, t: f32) -> RgbaImage {
    let a = draw_panel(r, set, Panel::Orig, fi, t);
    let b = draw_panel(r, set, Panel::HdStatic, fi, t);
    let c = draw_panel(r, set, Panel::HdWarp, fi, t);
    compose_row(&[a, b, c], PAD)
}

/// Prepare (if needed) then write frame strips, contact sheets, APNG and a metrics text file.
pub fn run(vfs: &Vfs, cache: &Path, out: &Path, prepare: bool, regenerate: bool) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let man_path = cache.join("manifest.json");
    if prepare || regenerate || !man_path.is_file() {
        let t0 = Instant::now();
        eprintln!("[hd] preparing prototype into {}", cache.display());
        let man = hd::prepare_proto(vfs, cache, &mut |s| eprintln!("[hd] {s}"))?;
        eprintln!("[hd] prepare done in {:.1}s, {} actions", t0.elapsed().as_secs_f32(), man.actions.len());
    }
    let man = hd::load_manifest(cache)?;
    let mut r = headless_gpu()?;
    let (actf, slot, actions) = hd::PROTO;
    let mut sets = Vec::new();
    for &action in actions {
        let (orig, durs) = hd::source_frames(vfs, actf, slot, action)?;
        let (hd_frames, mesh) = hd::load_action(cache, actf, slot, action)?;
        if hd_frames.len() != orig.len() || hd_frames.len() != mesh.frames.len() {
            return Err(format!("frame count mismatch on {actf}/{slot}/{action}"));
        }
        let tex_orig: Vec<usize> = orig.iter().map(|im| r.upload(im, false)).collect();
        let tex_hd: Vec<usize> = hd_frames.iter().map(|im| r.upload(im, true)).collect();
        sets.push(ActionSet { action, orig, hd: hd_frames, mesh, durations_ms: durs, tex_orig, tex_hd });
    }

    // metrics
    let mut metrics = String::new();
    metrics.push_str(&format!(
        "HD prototype metrics\nmodel: {}  scale: {}x  algo: {}\ncache: {}\nerror threshold (fade): {}\n\n",
        man.model, man.scale, man.algo, cache.display(), newpal_prepare::mesh::ERROR_FADE
    ));
    metrics.push_str("action  frames  mean_err  max_err  fade_segments\n");
    for a in &man.actions {
        metrics.push_str(&format!(
            "{:<22} {:>4}   {:>7.4}  {:>7.4}  {:>3}/{}\n",
            a.key, a.frames, a.mean_error, a.max_error, a.fade_segments, a.frames
        ));
    }
    let mean_all = man.actions.iter().map(|a| a.mean_error).sum::<f32>() / man.actions.len().max(1) as f32;
    let max_all = man.actions.iter().map(|a| a.max_error).fold(0.0f32, f32::max);
    let fade_all: usize = man.actions.iter().map(|a| a.fade_segments).sum();
    let frames_all: usize = man.actions.iter().map(|a| a.frames).sum();
    metrics.push_str(&format!("\nALL  mean {mean_all:.4}  max {max_all:.4}  fade {fade_all}/{frames_all}\n"));
    let metrics_path = out.join("metrics.txt");
    std::fs::write(&metrics_path, &metrics).map_err(|e| e.to_string())?;
    eprintln!("[hd] {}", metrics.trim());

    // contact sheets at t=0 and mid-segment
    let mut rows0 = Vec::new();
    let mut rows_mid = Vec::new();
    for s in &sets {
        rows0.push(trio(&mut r, s, 0, 0.0));
        rows_mid.push(trio(&mut r, s, 0, 0.5));
    }
    compose_stack(&rows0, PAD).save_png(&out.join("hd_compare_sheet_t0.png")).map_err(|e| e.to_string())?;
    compose_stack(&rows_mid, PAD).save_png(&out.join("hd_compare_sheet_t05.png")).map_err(|e| e.to_string())?;
    eprintln!("[hd] wrote contact sheets");

    // per-direction strips (4 samples per original frame)
    for s in &sets {
        let mut cells_o = Vec::new();
        let mut cells_h = Vec::new();
        let mut cells_w = Vec::new();
        for (fi, t) in strip_samples(s.orig.len()) {
            cells_o.push(draw_panel(&mut r, s, Panel::Orig, fi, t));
            cells_h.push(draw_panel(&mut r, s, Panel::HdStatic, fi, t));
            cells_w.push(draw_panel(&mut r, s, Panel::HdWarp, fi, t));
        }
        let sheet = compose_stack(
            &[compose_row(&cells_o, 2), compose_row(&cells_h, 2), compose_row(&cells_w, 2)],
            PAD,
        );
        let p = out.join(format!("hd_compare_strip_a{}.png", s.action));
        sheet.save_png(&p).map_err(|e| e.to_string())?;
        eprintln!("[hd] wrote {}", p.display());
    }

    // APNG of the 8-dir 3-column grid at 60 Hz for one walk cycle (use first set's timing; all walks share DS=3)
    let samples = cycle_samples(&sets[0].durations_ms, HZ);
    let mut frames = Vec::with_capacity(samples.len());
    for (k, (fi, t)) in samples.iter().enumerate() {
        let rows: Vec<RgbaImage> = sets.iter().map(|s| trio(&mut r, s, *fi, *t)).collect();
        frames.push(compose_stack(&rows, PAD));
        if k % 10 == 0 {
            eprintln!("[hd] apng frame {}/{}", k + 1, samples.len());
        }
    }
    let apng = out.join("hd_compare_walk.apng");
    RgbaImage::save_apng(&apng, &frames, 1, HZ as u16).map_err(|e| e.to_string())?;
    eprintln!("[hd] wrote {} ({} frames @ {} Hz)", apng.display(), frames.len(), HZ);

    // also a lighter APNG of a single representative direction (action 8) 3-column
    if let Some(s) = sets.iter().find(|s| s.action == 8).or(sets.first()) {
        let mut one = Vec::new();
        for (fi, t) in &samples {
            one.push(trio(&mut r, s, *fi, *t));
        }
        let p = out.join(format!("hd_compare_walk_a{}.apng", s.action));
        RgbaImage::save_apng(&p, &one, 1, HZ as u16).map_err(|e| e.to_string())?;
        eprintln!("[hd] wrote {}", p.display());
    }

    let _ = std::fs::copy(&man_path, out.join("manifest.json"));
    eprintln!("[hd] compare output: {}", out.display());
    Ok(())
}
