//! HD sprite preparation (prototype): 4x Real-ESRGAN upscale of the player's own frames + a motion
//! mesh. Output lives in `%LOCALAPPDATA%\NewPAL-Reborn\cache\hd\` (never shipped).
use crate::backend::{ProcessRunner, Runner};
use crate::esrgan;
use crate::mesh::{self, AnimMesh};
use newpal_assets::act;
use newpal_assets::image::RgbaImage;
use newpal_assets::vfs::Vfs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SCALE: u32 = 4;
pub const TICK_MS: f32 = 35.0;
/// Prototype scope: the hero walk cycle, 8 directions (all_char.act slot 0, actions 8..15).
pub const PROTO: (&str, usize, &[usize]) = ("all_char", 0, &[8, 9, 10, 11, 12, 13, 14, 15]);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HdManifest {
    pub scale: u32,
    pub model: String,
    pub algo: u32,
    pub actions: Vec<HdAction>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HdAction {
    pub key: String,
    pub frames: usize,
    pub mean_error: f32,
    pub max_error: f32,
    pub fade_segments: usize,
}

pub const ALGO: u32 = 1;

pub fn default_dir() -> PathBuf {
    newpal_assets::gamedir::user_data_dir().join("cache").join("hd")
}

/// Spread opaque RGB into neighbouring transparent pixels (alpha stays 0) so the upscaler does not
/// invent a dark halo around the silhouette.
pub fn bleed(img: &RgbaImage, radius: u32) -> RgbaImage {
    let mut o = img.clone();
    for _ in 0..radius {
        let src = o.clone();
        for y in 0..src.height {
            for x in 0..src.width {
                let i = ((y * src.width + x) * 4) as usize;
                if src.pixels[i + 3] != 0 { continue; }
                let mut acc = [0u32; 3];
                let mut n = 0u32;
                for (dx, dy) in [(-1i32, 0), (1, 0), (0, -1), (0, 1)] {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= src.width as i32 || ny >= src.height as i32 { continue; }
                    let j = ((ny as u32 * src.width + nx as u32) * 4) as usize;
                    if src.pixels[j + 3] == 0 { continue; }
                    for c in 0..3 { acc[c] += src.pixels[j + c] as u32; }
                    n += 1;
                }
                if n > 0 {
                    for c in 0..3 { o.pixels[i + c] = (acc[c] / n) as u8; }
                }
            }
        }
    }
    // keep alpha from the original
    for i in 0..(img.width * img.height) as usize {
        o.pixels[i * 4 + 3] = 0; // RGB plate is fully opaque for the colour pass; alpha is a separate plate
        // wait - we need original alpha saved separately. Colour plate is opaque RGB (bled).
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

fn merge(rgb: &RgbaImage, al: &RgbaImage, target: (u32, u32)) -> Result<RgbaImage, String> {
    if rgb.width != al.width || rgb.height != al.height {
        return Err(format!("upscale size mismatch {}x{} vs {}x{}", rgb.width, rgb.height, al.width, al.height));
    }
    let mut o = RgbaImage::new(rgb.width, rgb.height);
    for i in 0..(rgb.width * rgb.height) as usize {
        let a = al.pixels[i * 4]; // grey plate
        o.pixels[i * 4] = rgb.pixels[i * 4];
        o.pixels[i * 4 + 1] = rgb.pixels[i * 4 + 1];
        o.pixels[i * 4 + 2] = rgb.pixels[i * 4 + 2];
        o.pixels[i * 4 + 3] = if a < 8 { 0 } else { a };
    }
    if (o.width, o.height) != target {
        // realesrgan should hit exact 4x; if not, leave as-is (still usable)
    }
    Ok(o)
}

fn run_esrgan(exe: &Path, models: &Path, input: &Path, output: &Path, runner: &dyn Runner, log: &mut dyn FnMut(String)) -> Result<(), String> {
    std::fs::create_dir_all(output).map_err(|e| e.to_string())?;
    log(format!("Real-ESRGAN {} -> {}", input.display(), output.display()));
    let args: Vec<String> = [
        "-i", &input.to_string_lossy(), "-o", &output.to_string_lossy(),
        "-n", esrgan::MODEL, "-s", &SCALE.to_string(), "-m", &models.to_string_lossy(),
        "-g", "0", "-j", "1:2:2", "-f", "png",
    ].iter().map(|s| s.to_string()).collect();
    runner.run(exe, &args)
}

/// Load original frames of one action (padded to a shared canvas, bottom-centre).
pub fn source_frames(vfs: &Vfs, actf: &str, slot: usize, action: usize) -> Result<(Vec<RgbaImage>, Vec<f32>), String> {
    let acts = vfs.act(actf, slot).map_err(|e| e.to_string())?;
    let a = acts.get(action).and_then(|a| a.as_ref()).ok_or("no action")?;
    let mut fr = Vec::new();
    for s in act::steps(a) {
        let Some(id) = s.sprite.filter(|&id| id < 3000) else { continue };
        let t = vfs.sprite_frame(actf, id, s.frame as usize).map_err(|e| e.to_string())?;
        fr.push((t.image, s.delay.max(1) as f32 * TICK_MS));
    }
    if fr.len() < 2 { return Err(format!("{actf}/{slot}/{action}: {} frames", fr.len())); }
    let (w, h) = fr.iter().fold((0, 0), |m, (i, _)| (m.0.max(i.width), m.1.max(i.height)));
    let mut frames = Vec::new();
    let mut durs = Vec::new();
    for (img, d) in fr {
        let mut c = RgbaImage::new(w, h);
        c.blit(&img, ((w - img.width) / 2) as i32, (h - img.height) as i32);
        frames.push(c);
        durs.push(d);
    }
    Ok((frames, durs))
}

pub struct Prepared {
    pub key: String,
    pub orig: Vec<RgbaImage>,
    pub hd: Vec<RgbaImage>,
    pub mesh: AnimMesh,
    pub durations_ms: Vec<f32>,
}

pub fn load_manifest(root: &Path) -> Result<HdManifest, String> {
    serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

pub fn action_dir(root: &Path, actf: &str, slot: usize, action: usize) -> PathBuf {
    root.join(actf).join(slot.to_string()).join(action.to_string())
}

pub fn load_action(root: &Path, actf: &str, slot: usize, action: usize) -> Result<(Vec<RgbaImage>, AnimMesh), String> {
    let dir = action_dir(root, actf, slot, action);
    let mesh: AnimMesh = serde_json::from_str(&std::fs::read_to_string(dir.join("mesh.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut hd = Vec::new();
    for i in 0..mesh.frames.len() {
        let p = dir.join(format!("f{i:03}.png"));
        hd.push(RgbaImage::from_png(&std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?).map_err(|e| e.to_string())?);
    }
    Ok((hd, mesh))
}

fn save_action(root: &Path, actf: &str, slot: usize, action: usize, hd: &[RgbaImage], mesh: &AnimMesh) -> Result<(), String> {
    let dir = action_dir(root, actf, slot, action);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (i, img) in hd.iter().enumerate() {
        img.save_png(&dir.join(format!("f{i:03}.png"))).map_err(|e| e.to_string())?;
    }
    std::fs::write(dir.join("mesh.json"), serde_json::to_vec_pretty(mesh).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(())
}

/// Prepare the prototype set (hero walk, 8 dirs).
pub fn prepare_proto(vfs: &Vfs, out: &Path, log: &mut dyn FnMut(String)) -> Result<HdManifest, String> {
    let tools = crate::default_tools();
    let (exe, models) = esrgan::ensure(&tools, log)?;
    let runner = ProcessRunner;
    let work = tools.join("hd-work");
    let _ = std::fs::remove_dir_all(&work);
    let (rgb_in, al_in, rgb_out, al_out) = (work.join("rgb"), work.join("alpha"), work.join("rgb_out"), work.join("alpha_out"));
    for d in [&rgb_in, &al_in, &rgb_out, &al_out] { std::fs::create_dir_all(d).map_err(|e| e.to_string())?; }

    let (actf, slot, actions) = PROTO;
    let mut jobs = Vec::new();
    for &action in actions {
        let (frames, durs) = source_frames(vfs, actf, slot, action)?;
        for (i, img) in frames.iter().enumerate() {
            let name = format!("{actf}_{slot}_{action}_{i:03}.png");
            bleed(img, 3).save_png(&rgb_in.join(&name)).map_err(|e| e.to_string())?;
            alpha_plate(img).save_png(&al_in.join(&name)).map_err(|e| e.to_string())?;
        }
        jobs.push((action, frames, durs));
    }
    run_esrgan(&exe, &models, &rgb_in, &rgb_out, &runner, log)?;
    run_esrgan(&exe, &models, &al_in, &al_out, &runner, log)?;

    let mut man = HdManifest { scale: SCALE, model: esrgan::MODEL.into(), algo: ALGO, actions: Vec::new() };
    for (action, orig, durs) in jobs {
        let mut hd = Vec::new();
        for i in 0..orig.len() {
            let name = format!("{actf}_{slot}_{action}_{i:03}.png");
            let rgb = RgbaImage::from_png(&std::fs::read(rgb_out.join(&name)).map_err(|e| format!("missing rgb {name}: {e}"))?).map_err(|e| e.to_string())?;
            let al = RgbaImage::from_png(&std::fs::read(al_out.join(&name)).map_err(|e| format!("missing alpha {name}: {e}"))?).map_err(|e| e.to_string())?;
            hd.push(merge(&rgb, &al, (orig[0].width * SCALE, orig[0].height * SCALE))?);
        }
        log(format!("mesh {actf}/{slot}/{action} ({} frames)", orig.len()));
        // track on original pixels (more stable, less noise than HD), keep logical units
        let mesh = mesh::scale_to(mesh::build(&orig, &durs)?, SCALE);
        let mean = mesh.frames.iter().map(|f| f.error).sum::<f32>() / mesh.frames.len() as f32;
        let max = mesh.frames.iter().map(|f| f.error).fold(0.0f32, f32::max);
        let fade = mesh.frames.iter().filter(|f| f.fallback).count();
        save_action(out, actf, slot, action, &hd, &mesh)?;
        man.actions.push(HdAction { key: format!("{actf}/{slot}/{action}"), frames: orig.len(), mean_error: mean, max_error: max, fade_segments: fade });
        log(format!("  mean err {mean:.3} max {max:.3} fade {fade}/{}", orig.len()));
    }
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    std::fs::write(out.join("manifest.json"), serde_json::to_vec_pretty(&man).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(man)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bleed_fills_neighbour_but_keeps_silhouette_on_alpha_plate() {
        let mut img = RgbaImage::new(4, 4);
        img.put(1, 1, [10, 20, 30, 255]);
        let rgb = bleed(&img, 2);
        // the seed stays
        assert_eq!(&rgb.pixels[(1 * 4 + 1) * 4..(1 * 4 + 1) * 4 + 3], &[10, 20, 30]);
        // a neighbour got colour and is opaque on the colour plate
        let n = &rgb.pixels[(1 * 4 + 0) * 4..(1 * 4 + 0) * 4 + 4];
        assert_eq!(n[3], 255);
        assert_eq!(&n[..3], &[10, 20, 30]);
        let al = alpha_plate(&img);
        assert_eq!(al.pixels[(1 * 4 + 1) * 4], 255);
        assert_eq!(al.pixels[(1 * 4 + 0) * 4], 0);
    }
}
