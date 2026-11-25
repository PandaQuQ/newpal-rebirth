//! RIFE 4x + packed optical-flow prototype on the hero walk cycle (all_char slot 0, actions 8..15).
//! Offline: interpolate HD stills with rife-ncnn-vulkan (below-normal priority). Runtime: wgpu
//! flow-warp at the display rate. Original 8 x 105 ms = 840 ms cycle is unchanged.
use crate::render::{Camera, Renderer, CAM_WORLD};
use newpal_assets::act;
use newpal_assets::image::RgbaImage;
use newpal_assets::vfs::Vfs;
use newpal_prepare::backend::{self, Backend, Result as PResult, Runner};
use newpal_prepare::flow::{self, AnimClock, FlowField};
use newpal_prepare::hd::{self, PROTO, SCALE};
use newpal_prepare::residual;
use newpal_prepare::rife;
use std::path::Path;
use std::time::Instant;

const CLEAR: wgpu::Color = wgpu::Color { r: 0.12, g: 0.12, b: 0.14, a: 1.0 };
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const PAD: i32 = 8;
const FACTOR: u32 = 4;
const FLOW_CELL: u32 = 8;
const FLOW_SEARCH: i32 = 6;
const DIRS: &[&str] = &["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

struct NiceRunner;

impl Runner for NiceRunner {
    fn run(&self, exe: &Path, args: &[String]) -> PResult<()> {
        let mut c = std::process::Command::new(exe);
        c.args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // BELOW_NORMAL | CREATE_NO_WINDOW — leave CPU for the residual patch.
            c.creation_flags(0x0000_4000 | 0x0800_0000);
        }
        let o = c.output().map_err(|e| format!("{}: {e}", exe.display()))?;
        if !o.status.success() {
            return Err(format!(
                "{} failed ({}): {}",
                exe.display(),
                o.status,
                String::from_utf8_lossy(&o.stderr)
                    .lines()
                    .last()
                    .unwrap_or("")
            ));
        }
        Ok(())
    }
}

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
        "[rife-flow] adapter: {} ({:?})",
        adapter.get_info().name,
        adapter.get_info().backend
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("rife-flow"),
        ..Default::default()
    }))
    .map_err(|e| format!("request_device: {e}"))?;
    Ok(Renderer::new(device, queue, wgpu::TextureFormat::Rgba8UnormSrgb))
}

fn load_hd_frames(
    vfs: &Vfs,
    hd: &Path,
    actf: &str,
    slot: usize,
    action: usize,
    logical: (u32, u32),
) -> Result<Vec<RgbaImage>, String> {
    let acts = vfs.act(actf, slot).map_err(|e| e.to_string())?;
    let a = acts
        .get(action)
        .and_then(|a| a.as_ref())
        .ok_or("no action")?;
    let (cw, ch) = (logical.0 * SCALE, logical.1 * SCALE);
    let mut out = Vec::new();
    for s in act::steps(a) {
        let Some(id) = s.sprite.filter(|&id| id < 3000) else {
            continue;
        };
        let p = hd
            .join("sprites")
            .join(actf)
            .join(id.to_string())
            .join(format!("f{}@4x.png", s.frame));
        let img = RgbaImage::from_png(
            &std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?,
        )
        .map_err(|e| e.to_string())?;
        let mut c = RgbaImage::new(cw, ch);
        let x = (cw as i32 - img.width as i32) / 2;
        let y = (ch as i32 - img.height as i32) / 2;
        c.blit(&img, x, y);
        out.push(c);
    }
    Ok(out)
}

fn compose_row(panels: &[RgbaImage], pad: i32) -> RgbaImage {
    let h = panels.iter().map(|p| p.height).max().unwrap_or(1);
    let w = panels.iter().map(|p| p.width).sum::<u32>() + (pad as u32) * (panels.len() as u32 + 1);
    let mut o = RgbaImage::new(w, h + pad as u32 * 2);
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

fn scale_half(img: &RgbaImage) -> RgbaImage {
    let w = (img.width / 2).max(1);
    let h = (img.height / 2).max(1);
    let mut o = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 4];
            for dy in 0..2u32 {
                for dx in 0..2u32 {
                    let px = sample(img, (x * 2 + dx).min(img.width - 1), (y * 2 + dy).min(img.height - 1));
                    for c in 0..4 { acc[c] += px[c] as u32; }
                }
            }
            o.put(x, y, acc.map(|v| ((v + 2) / 4) as u8));
        }
    }
    o
}

fn alpha_coverage(img: &RgbaImage) -> f32 {
    let n = (img.width * img.height) as f32;
    let k = img.pixels.chunks_exact(4).filter(|p| p[3] >= 8).count() as f32;
    k / n.max(1.0)
}

fn sample(img: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
    let o = ((y * img.width + x) * 4) as usize;
    [img.pixels[o], img.pixels[o + 1], img.pixels[o + 2], img.pixels[o + 3]]
}

fn scale_nearest(img: &RgbaImage, sx: u32, sy: u32) -> RgbaImage {
    let mut o = RgbaImage::new(img.width * sx, img.height * sy);
    for y in 0..o.height {
        for x in 0..o.width {
            o.put(x, y, sample(img, x / sx, y / sy));
        }
    }
    o
}

fn png_bytes(img: &RgbaImage) -> Result<u64, String> {
    let dir = std::env::temp_dir().join("np-rife-flow-sz");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let p = dir.join(format!("{:x}.png", img.pixels.len() + img.width as usize));
    img.save_png(&p).map_err(|e| e.to_string())?;
    let n = std::fs::metadata(&p).map_err(|e| e.to_string())?.len();
    let _ = std::fs::remove_file(&p);
    Ok(n)
}

fn clock_ok(c: &AnimClock, label: &str, log: &mut String) -> Result<(), String> {
    let p = c.period_secs();
    if (p - 0.84).abs() >= 1e-5 {
        return Err(format!("{label}: period {p} != 0.84"));
    }
    let mut ok = true;
    for hz in [60.0f32, 165.0, 300.0] {
        let samples = c.samples(hz, 1);
        let wall = samples.len() as f64 / hz as f64;
        let span_ok = (wall - p).abs() < 1.0 / hz as f64 + 1e-6;
        let (i0, j0, k0) = c.at(0.0);
        let (i1, j1, k1) = c.at(c.period());
        let wrap = i0 == i1 && j0 == j1 && (k0 - k1).abs() < 1e-4;
        log.push_str(&format!(
            "  {label} {hz}Hz: samples={} wall={:.4}s period={:.4}s span_ok={span_ok} wrap_ok={wrap}\n",
            samples.len(),
            wall,
            p
        ));
        if !span_ok || !wrap {
            ok = false;
        }
    }
    if !ok {
        return Err(format!("{label}: 840ms cycle check failed"));
    }
    Ok(())
}

fn gpu_ms_per_frame(
    r: &mut Renderer,
    tex_a: usize,
    tex_b: usize,
    flow: usize,
    conf: usize,
    w: u32,
    h: u32,
) -> f64 {
    r.set_camera(
        CAM_WORLD,
        Camera {
            origin: [0.0, 0.0],
            size: [w as f32, h as f32],
        },
    );
    // warmup (includes readback)
    for _ in 0..3 {
        r.push_flow(tex_a, tex_b, flow, conf, CAM_WORLD, 0.0, 0.0, w as f32, h as f32, WHITE, 0.5);
        let _ = r.render_to_image(w, h, CLEAR);
    }
    let n = 20u32;
    let t0 = Instant::now();
    for _ in 0..n {
        r.push_flow(tex_a, tex_b, flow, conf, CAM_WORLD, 0.0, 0.0, w as f32, h as f32, WHITE, 0.5);
        let _ = r.render_to_image(w, h, CLEAR);
    }
    t0.elapsed().as_secs_f64() * 1000.0 / n as f64
}

struct DirSet {
    action: usize,
    name: &'static str,
    orig: Vec<RgbaImage>,
    hd: Vec<RgbaImage>,
    rife: Vec<RgbaImage>,
    flows: Vec<FlowField>,
    packs: Vec<Vec<u8>>,
    durs_ms: Vec<f32>,
}

/// Run the prototype. `out` is the local output folder (also copyable to the share dir).
pub fn run(vfs: &Vfs, hd_cache: &Path, tools: &Path, out: &Path) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let work = out.join("rife-work");
    let (actf, slot, actions) = PROTO;
    let (exe, model) = rife::installed(tools)
        .or_else(|| rife::ensure(tools, &mut |s| eprintln!("[rife-flow] {s}")).ok())
        .ok_or("rife-ncnn-vulkan not installed")?;
    eprintln!(
        "[rife-flow] exe {} model {} factor={FACTOR}",
        exe.display(),
        model.display()
    );
    let be = newpal_prepare::backend::Rife {
        exe,
        model_dir: model,
        gpu: 0,
        work: work.clone(),
        runner: NiceRunner,
        version: rife::VERSION.into(),
    };

    let mut sets = Vec::new();
    let mut sanity_log = String::new();
    let t_rife = Instant::now();
    for (di, &action) in actions.iter().enumerate() {
        let (orig, durs_ms) = hd::source_frames(vfs, actf, slot, action)?;
        let (lw, lh) = orig
            .iter()
            .fold((0u32, 0u32), |m, i| (m.0.max(i.width), m.1.max(i.height)));
        let hd_frames = load_hd_frames(vfs, hd_cache, actf, slot, action, (lw, lh))?;
        if hd_frames.len() != orig.len() {
            return Err(format!(
                "{actf}/{slot}/{action}: hd {} vs orig {}",
                hd_frames.len(),
                orig.len()
            ));
        }
        let sum: f32 = durs_ms.iter().sum();
        eprintln!(
            "[rife-flow] {} action {action}: {} frames, {sum:.1} ms, {}x{} HD {}x{}",
            DIRS[di],
            orig.len(),
            lw,
            lh,
            hd_frames[0].width,
            hd_frames[0].height
        );
        let t0 = Instant::now();
        let mut rife_frames = be.interpolate(&hd_frames, FACTOR)?;
        eprintln!(
            "[rife-flow]   RIFE -> {} frames in {:.1}s",
            rife_frames.len(),
            t0.elapsed().as_secs_f32()
        );
        if rife_frames.len() != orig.len() * FACTOR as usize {
            return Err(format!(
                "expected {} rife frames, got {}",
                orig.len() * FACTOR as usize,
                rife_frames.len()
            ));
        }
        // Sanity:
        // - Keyframes RIFE emits should resemble the HD inputs (stride/channel garbage fails this hard).
        //   rife-ncnn-vulkan is not bit-exact on keys (~25 dB is normal); we pin the HD keys back
        //   into the sequence so duration and source stills stay authoritative.
        // - Mid vs neighbour cross-fade is a soft check: ~20 dB is typical, but a walk cycle with
        //   real motion can sit lower because RIFE places one body, not a dissolve. Stripe noise
        //   falls well below 12 dB and also blows alpha coverage.
        // Dump a contact strip immediately so a later direction failing the gate still leaves proof.
        let nkey = orig.len();
        {
            let mid = FACTOR as usize / 2;
            let next = FACTOR as usize;
            let row = compose_row(
                &[
                    hd_frames[0].clone(),
                    rife_frames[0].clone(),
                    rife_frames[mid].clone(),
                    rife_frames[next].clone(),
                    hd_frames[1].clone(),
                ],
                PAD,
            );
            let sanity_dir = out.join("sanity");
            std::fs::create_dir_all(&sanity_dir).map_err(|e| e.to_string())?;
            row.save_png(&sanity_dir.join(format!("{}_{}_key_mid_key.png", DIRS[di], action)))
                .map_err(|e| e.to_string())?;
        }
        let mut key_psnr_sum = 0.0f64;
        let mut key_psnr_n = 0usize;
        let mut key_psnr_min = f64::INFINITY;
        for i in 0..nkey {
            if let Some(p) = residual::psnr(&rife_frames[i * FACTOR as usize], &hd_frames[i]) {
                let p = if p.is_finite() { p } else { 99.0 };
                key_psnr_sum += p;
                key_psnr_n += 1;
                if p < key_psnr_min {
                    key_psnr_min = p;
                }
            }
        }
        let key_psnr_avg = if key_psnr_n == 0 { 0.0 } else { key_psnr_sum / key_psnr_n as f64 };
        // Pin HD keys over RIFE's slightly drifted copies so the cycle stills stay exact.
        for i in 0..nkey {
            rife_frames[i * FACTOR as usize] = hd_frames[i].clone();
        }
        let cov_key: f32 = (0..nkey).map(|i| alpha_coverage(&rife_frames[i * FACTOR as usize])).sum::<f32>() / nkey as f32;
        let mut psnr_sum = 0.0f64;
        let mut psnr_n = 0usize;
        let mut psnr_min = f64::INFINITY;
        let mut cov_mid_sum = 0.0f32;
        let mut cov_mid_n = 0usize;
        for i in 0..nkey {
            let a = &rife_frames[i * FACTOR as usize];
            let b = &rife_frames[((i + 1) % nkey) * FACTOR as usize];
            for k in 1..FACTOR as usize {
                let mid = &rife_frames[i * FACTOR as usize + k];
                let fade = backend::blend(a, b, k as f32 / FACTOR as f32);
                if let Some(p) = residual::psnr(mid, &fade) {
                    let p = if p.is_finite() { p } else { 99.0 };
                    psnr_sum += p;
                    psnr_n += 1;
                    if p < psnr_min { psnr_min = p; }
                }
                cov_mid_sum += alpha_coverage(mid);
                cov_mid_n += 1;
            }
        }
        let psnr_avg = if psnr_n == 0 { 0.0 } else { psnr_sum / psnr_n as f64 };
        let cov_mid = if cov_mid_n == 0 { 0.0 } else { cov_mid_sum / cov_mid_n as f32 };
        eprintln!(
            "[rife-flow]   sanity key-vs-HD PSNR avg={key_psnr_avg:.2} min={key_psnr_min:.2} dB  mid-vs-crossfade avg={psnr_avg:.2} min={psnr_min:.2} dB  alpha key={:.3} mid={:.3}",
            cov_key, cov_mid
        );
        if key_psnr_avg < 18.0 {
            return Err(format!(
                "{actf}/{slot}/{action}: RIFE key-vs-HD PSNR {key_psnr_avg:.2} dB < 18 (stride/channel garbage?)"
            ));
        }
        if psnr_avg < 12.0 {
            return Err(format!(
                "{actf}/{slot}/{action}: RIFE mid-frame PSNR {psnr_avg:.2} dB < 12 (looks like stride/channel garbage)"
            ));
        }
        if psnr_avg < 20.0 {
            eprintln!(
                "[rife-flow]   note: mid-vs-crossfade {psnr_avg:.2} dB < 20 (OK for large walk motion if keys/alpha pass)"
            );
        }
        if (cov_mid - cov_key).abs() > 0.25 {
            return Err(format!(
                "{actf}/{slot}/{action}: alpha coverage mid={cov_mid:.3} vs key={cov_key:.3} (background filled?)"
            ));
        }
        sanity_log.push_str(&format!(
            "sanity {}:{}  key-vs-HD PSNR avg={key_psnr_avg:.2} min={key_psnr_min:.2} dB  mid-vs-crossfade avg={psnr_avg:.2} min={psnr_min:.2} dB  alpha key={:.3} mid={:.3}\n",
            DIRS[di], action, cov_key, cov_mid
        ));
        let mut flows = Vec::with_capacity(rife_frames.len());
        let mut packs = Vec::with_capacity(rife_frames.len());
        let t1 = Instant::now();
        for i in 0..rife_frames.len() {
            let a = &rife_frames[i];
            let b = &rife_frames[(i + 1) % rife_frames.len()];
            let f = flow::estimate(a, b, FLOW_CELL, FLOW_SEARCH);
            let pack = flow::encode(&f)?;
            let back = flow::decode(&pack)?;
            if back.width != f.width
                || back.height != f.height
                || back.fx != f.fx
                || back.fy != f.fy
                || back.bx != f.bx
                || back.by != f.by
                || back.conf != f.conf
            {
                return Err(format!(
                    "{actf}/{slot}/{action}: NFLO encode/decode mismatch at frame {i}"
                ));
            }
            packs.push(pack);
            flows.push(f);
        }
        eprintln!(
            "[rife-flow]   {} NFLO packs in {:.1}s",
            packs.len(),
            t1.elapsed().as_secs_f32()
        );
        sets.push(DirSet {
            action,
            name: DIRS[di],
            orig,
            hd: hd_frames,
            rife: rife_frames,
            flows,
            packs,
            durs_ms,
        });
    }
    let rife_secs = t_rife.elapsed().as_secs_f32();

    let mut log = String::new();
    log.push_str(&format!(
        "RIFE 4x + flow-warp prototype - hero walk, 8 directions\n\
         model: {}  factor: {FACTOR}  flow cell={FLOW_CELL} search={FLOW_SEARCH}\n\
         HD cache: {}\n\n",
        rife::MODEL,
        hd_cache.display()
    ));

    // 840 ms checks on original clock and on the refined clock
    for s in &sets {
        let orig_clock = AnimClock {
            delays: s.durs_ms.iter().map(|d| *d / 1000.0).collect(),
        };
        clock_ok(&orig_clock, &format!("orig {}:{}", s.name, s.action), &mut log)?;
        let refined_clock = AnimClock {
            delays: s
                .durs_ms
                .iter()
                .flat_map(|d| std::iter::repeat_n(*d / 1000.0 / FACTOR as f32, FACTOR as usize))
                .collect(),
        };
        clock_ok(
            &refined_clock,
            &format!("rife {}:{}", s.name, s.action),
            &mut log,
        )?;
    }
    log.push('\n');
    log.push_str(&sanity_log);
    log.push('\n');

    // sizes
    let mut hd_png = 0u64;
    let mut rife_png = 0u64;
    let mut nflo = 0u64;
    for s in &sets {
        for im in &s.hd {
            hd_png += png_bytes(im)?;
        }
        for im in &s.rife {
            rife_png += png_bytes(im)?;
        }
        for p in &s.packs {
            nflo += p.len() as u64;
        }
        // write packs + a handful of frames to out/data
        let dir = out.join("data").join(format!("a{}", s.action));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for (i, p) in s.packs.iter().enumerate() {
            std::fs::write(dir.join(format!("p{i:02}.nflo")), p).map_err(|e| e.to_string())?;
        }
        for (i, im) in s.rife.iter().enumerate() {
            im.save_png(&dir.join(format!("r{i:02}.png")))
                .map_err(|e| e.to_string())?;
        }
    }
    let extra = rife_png + nflo;
    log.push_str(&format!(
        "size impact (8 dirs, PNG on disk + NFLO):\n\
         HD keyframes (8x8):           {hd_png} bytes ({:.3} MB)\n\
         RIFE refined (8x32):          {rife_png} bytes ({:.3} MB)  ({:.2}x HD keys)\n\
         NFLO packs (8x32):            {nflo} bytes ({:.3} MB)\n\
         NFLO + refined frames:        {extra} bytes ({:.3} MB)  ({:.2}x HD keys)\n\
         RIFE-only extra over keys:    {} bytes\n\n",
        hd_png as f64 / 1e6,
        rife_png as f64 / 1e6,
        rife_png as f64 / hd_png.max(1) as f64,
        nflo as f64 / 1e6,
        extra as f64 / 1e6,
        extra as f64 / hd_png.max(1) as f64,
        rife_png.saturating_sub(hd_png)
    ));

    // GPU warp timing + images
    let mut r = headless_gpu()?;
    let s0 = &sets[0];
    let (fw, fh) = (s0.rife[0].width, s0.rife[0].height);
    let ta = r.upload(&s0.rife[0], true);
    let tb = r.upload(&s0.rife[1], true);
    let (ff, cf) = r.upload_flow(&s0.flows[0], fw as f32, fh as f32);
    let gpu_ms = gpu_ms_per_frame(&mut r, ta, tb, ff, cf, fw, fh);
    log.push_str(&format!(
        "GPU flow-warp (action {}, {}x{}, t=0.5, includes readback): {gpu_ms:.3} ms/frame\n\
         RIFE wall time (8 dirs): {rife_secs:.1}s\n\n",
        s0.action, fw, fh
    ));
    eprintln!("[rife-flow] GPU flow-warp {gpu_ms:.3} ms/frame");

    // strips: each dir, 3 rows (orig nearest-4x, HD keys, RIFE 4 samples/key)
    for s in &sets {
        let orig4: Vec<RgbaImage> = s.orig.iter().map(|im| scale_nearest(im, SCALE, SCALE)).collect();
        let mut rife_samp = Vec::new();
        for i in 0..s.orig.len() {
            let base = i * FACTOR as usize;
            for k in 0..FACTOR as usize {
                rife_samp.push(s.rife[base + k].clone());
            }
        }
        let sheet = compose_stack(
            &[
                compose_row(&orig4, 2),
                compose_row(&s.hd, 2),
                compose_row(&rife_samp, 2),
            ],
            PAD,
        );
        let p = out.join(format!("strip_{}_{}.png", s.name, s.action));
        sheet.save_png(&p).map_err(|e| e.to_string())?;
        eprintln!("[rife-flow] wrote {}", p.display());
    }

    // mid-segment flow-warp vs RIFE vs HD static contact (t=0.5 of first refined pair of each key)
    let mut rows_mid = Vec::new();
    for s in &sets {
        r.set_camera(
            CAM_WORLD,
            Camera {
                origin: [0.0, 0.0],
                size: [fw as f32, fh as f32],
            },
        );
        // Same instant: HD key 0 | RIFE mid of first interval | flow-warp between those two keys at t=0.5
        let mid_i = FACTOR as usize / 2;
        let next_i = FACTOR as usize;
        let key_flow = flow::estimate(&s.rife[0], &s.rife[next_i], FLOW_CELL, FLOW_SEARCH);
        let a = r.upload(&s.rife[0], true);
        let b = r.upload(&s.rife[next_i], true);
        let (ff, cf) = r.upload_flow(&key_flow, fw as f32, fh as f32);
        let hd_id = r.upload(&s.hd[0], true);
        r.push(hd_id, CAM_WORLD, 0.0, 0.0, fw as f32, fh as f32, WHITE);
        let hd0 = r.render_to_image(fw, fh, CLEAR);
        let mid_id = r.upload(&s.rife[mid_i], true);
        r.push(mid_id, CAM_WORLD, 0.0, 0.0, fw as f32, fh as f32, WHITE);
        let mid_rife = r.render_to_image(fw, fh, CLEAR);
        r.push_flow(a, b, ff, cf, CAM_WORLD, 0.0, 0.0, fw as f32, fh as f32, WHITE, 0.5);
        let mid_flow = r.render_to_image(fw, fh, CLEAR);
        rows_mid.push(compose_row(&[hd0, mid_rife, mid_flow], PAD));
    }
    compose_stack(&rows_mid, PAD)
        .save_png(&out.join("sheet_mid_hd_rife_flow.png"))
        .map_err(|e| e.to_string())?;

    // APNGs
    // 8-dir grid at 60 Hz, one cycle; columns = HD-static (hold key) | RIFE | flow-warp
    write_apng_grid(&mut r, &sets, 60.0, out, "walk_8dir_60hz.apng", &mut log)?;
    // single dir (N / action 8) at 60/165/300
    write_apng_dir(&mut r, &sets[0], 60.0, out, "walk_N_60hz.apng", &mut log)?;
    write_apng_dir(&mut r, &sets[0], 165.0, out, "walk_N_165hz.apng", &mut log)?;
    write_apng_dir(&mut r, &sets[0], 300.0, out, "walk_N_300hz.apng", &mut log)?;

    let metrics = out.join("metrics.txt");
    std::fs::write(&metrics, &log).map_err(|e| e.to_string())?;
    eprintln!("[rife-flow] {}", log.trim());
    eprintln!("[rife-flow] wrote {}", metrics.display());
    let _ = std::fs::remove_dir_all(&work);
    Ok(())
}

fn sample_panel(
    r: &mut Renderer,
    s: &DirSet,
    t_secs: f32,
    cache: &PanelCache,
) -> RgbaImage {
    let orig_clock = AnimClock {
        delays: s.durs_ms.iter().map(|d| *d / 1000.0).collect(),
    };
    let refined_clock = AnimClock {
        delays: s
            .durs_ms
            .iter()
            .flat_map(|d| std::iter::repeat_n(*d / 1000.0 / FACTOR as f32, FACTOR as usize))
            .collect(),
    };
    let (ki, _, _) = orig_clock.at(t_secs);
    let (ri, rj, rk) = refined_clock.at(t_secs);
    let (fw, fh) = (s.rife[0].width, s.rife[0].height);
    r.set_camera(
        CAM_WORLD,
        Camera {
            origin: [0.0, 0.0],
            size: [fw as f32, fh as f32],
        },
    );
    let hd = cache.hd[ki];
    r.push(hd, CAM_WORLD, 0.0, 0.0, fw as f32, fh as f32, WHITE);
    let a = r.render_to_image(fw, fh, CLEAR);
    let rf = cache.rife[ri];
    r.push(rf, CAM_WORLD, 0.0, 0.0, fw as f32, fh as f32, WHITE);
    let b = r.render_to_image(fw, fh, CLEAR);
    let (ff, cf) = cache.flow[ri];
    r.push_flow(
        cache.rife[ri],
        cache.rife[rj],
        ff,
        cf,
        CAM_WORLD,
        0.0,
        0.0,
        fw as f32,
        fh as f32,
        WHITE,
        rk,
    );
    let c = r.render_to_image(fw, fh, CLEAR);
    compose_row(&[a, b, c], 2)
}

struct PanelCache {
    hd: Vec<usize>,
    rife: Vec<usize>,
    flow: Vec<(usize, usize)>,
}

fn upload_dir(r: &mut Renderer, s: &DirSet) -> PanelCache {
    let (fw, fh) = (s.rife[0].width, s.rife[0].height);
    let hd: Vec<usize> = s.hd.iter().map(|im| r.upload(im, true)).collect();
    let rife: Vec<usize> = s.rife.iter().map(|im| r.upload(im, true)).collect();
    let flow: Vec<(usize, usize)> = s
        .flows
        .iter()
        .map(|f| r.upload_flow(f, fw as f32, fh as f32))
        .collect();
    PanelCache { hd, rife, flow }
}

fn write_apng_dir(
    r: &mut Renderer,
    s: &DirSet,
    hz: f32,
    out: &Path,
    name: &str,
    log: &mut String,
) -> Result<(), String> {
    let clock = AnimClock {
        delays: s.durs_ms.iter().map(|d| *d / 1000.0).collect(),
    };
    let samples = clock.samples(hz, 1);
    let cache = upload_dir(r, s);
    let mut frames = Vec::with_capacity(samples.len());
    for t in &samples {
        frames.push(sample_panel(r, s, *t, &cache));
    }
    let p = out.join(name);
    let den = hz.round() as u16;
    let frames: Vec<RgbaImage> = frames.iter().map(scale_half).collect();
    RgbaImage::save_apng(&p, &frames, 1, den).map_err(|e| e.to_string())?;
    let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    log.push_str(&format!(
        "APNG {name}: {} frames @ {hz}Hz, {bytes} bytes, cycle {:.3}s\n",
        frames.len(),
        samples.len() as f32 / hz
    ));
    eprintln!("[rife-flow] wrote {} ({} frames @ {hz}Hz)", p.display(), frames.len());
    Ok(())
}

fn write_apng_grid(
    r: &mut Renderer,
    sets: &[DirSet],
    hz: f32,
    out: &Path,
    name: &str,
    log: &mut String,
) -> Result<(), String> {
    let clock = AnimClock {
        delays: sets[0].durs_ms.iter().map(|d| *d / 1000.0).collect(),
    };
    let samples = clock.samples(hz, 1);
    let caches: Vec<PanelCache> = sets.iter().map(|s| upload_dir(r, s)).collect();
    let mut frames = Vec::with_capacity(samples.len());
    for (k, t) in samples.iter().enumerate() {
        let rows: Vec<RgbaImage> = sets
            .iter()
            .zip(&caches)
            .map(|(s, c)| sample_panel(r, s, *t, c))
            .collect();
        frames.push(compose_stack(&rows, PAD));
        if k % 10 == 0 {
            eprintln!("[rife-flow] grid apng {}/{}", k + 1, samples.len());
        }
    }
    let p = out.join(name);
    let frames: Vec<RgbaImage> = frames.iter().map(scale_half).collect();
    RgbaImage::save_apng(&p, &frames, 1, hz.round() as u16).map_err(|e| e.to_string())?;
    let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    log.push_str(&format!(
        "APNG {name}: {} frames @ {hz}Hz, {bytes} bytes, cycle {:.3}s\n",
        frames.len(),
        samples.len() as f32 / hz
    ));
    eprintln!("[rife-flow] wrote {} ({} frames)", p.display(), frames.len());
    Ok(())
}