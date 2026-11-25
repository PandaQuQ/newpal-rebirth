//! Offline helper for smooth-animation mods (writes OUTSIDE the repo; output is derived from your game data,
//! never commit or ship it). Subcommands:
//!
//! * `export <game> <act> <slot> <action> <dir>` - original frames of one action, padded to one canvas
//!   (bottom-centre anchor, like the engine) -> `<dir>/f000.png..` + `frames.json {durations_ms}`
//! * `crossfade <in> <mod_action_dir> <n>` - CPU in-betweens: n blended frames between each pair (loops)
//!   -> mod frames + `anim.toml` (no GPU; a cheap baseline)
//! * `prep-rife <in> <work>` - split frames for RIFE: `<work>/rgb` (premultiplied on black) and
//!   `<work>/alpha` (alpha as grey), first frame repeated at the end so the loop seam is interpolated too
//! * `merge-rife <work> <mod_action_dir> <orig_total_ms>` - recombine RIFE outputs `<work>/rgb_out`,
//!   `<work>/alpha_out` (sorted by name, last frame = loop duplicate dropped) -> mod frames + `anim.toml`
use newpal_assets::act;
use newpal_assets::image::RgbaImage;
use newpal_assets::vfs::Vfs;
use std::path::{Path, PathBuf};

const TICK_MS: f32 = 35.0;

fn pngs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"))).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn load(p: &Path) -> RgbaImage {
    RgbaImage::from_png(&std::fs::read(p).unwrap()).unwrap()
}

fn durations(dir: &Path, n: usize) -> Vec<f32> {
    std::fs::read_to_string(dir.join("frames.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["durations_ms"].as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect::<Vec<_>>()))
        .filter(|d| d.len() == n)
        .unwrap_or_else(|| vec![TICK_MS * 2.0; n])
}

/// Premultiplied-alpha lerp (avoids dark fringes).
fn blend(a: &RgbaImage, b: &RgbaImage, k: f32) -> RgbaImage {
    let mut o = RgbaImage::new(a.width, a.height);
    for i in 0..(a.width * a.height) as usize {
        let (pa, pb) = (&a.pixels[i * 4..i * 4 + 4], &b.pixels[i * 4..i * 4 + 4]);
        let (aa, ab) = (pa[3] as f32 / 255.0, pb[3] as f32 / 255.0);
        let al = aa * (1.0 - k) + ab * k;
        let mut px = [0u8; 4];
        if al > 0.0 {
            for c in 0..3 {
                let v = (pa[c] as f32 * aa * (1.0 - k) + pb[c] as f32 * ab * k) / al;
                px[c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
        px[3] = (al * 255.0).round() as u8;
        o.pixels[i * 4..i * 4 + 4].copy_from_slice(&px);
    }
    o
}

fn write_manifest(dir: &Path, count: usize, durs: &[f32], note: &str) {
    let d = durs.iter().map(|d| format!("{d:.2}")).collect::<Vec<_>>().join(", ");
    let text = format!("# {note}\ncount = {count}\ndurations_ms = [{d}]\n");
    std::fs::write(dir.join("anim.toml"), text).unwrap();
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("export") if a.len() == 6 => {
            let vfs = Vfs::new(PathBuf::from(&a[1]), Vec::new());
            let (actf, slot, action, out) = (&a[2], a[3].parse::<usize>().unwrap(), a[4].parse::<usize>().unwrap(), PathBuf::from(&a[5]));
            let acts = vfs.act(actf, slot).unwrap();
            let act = acts.get(action).and_then(|x| x.as_ref()).expect("no such action");
            let mut frames = Vec::new();
            for s in act::steps(act) {
                let Some(id) = s.sprite.filter(|&id| id < 3000) else { continue };
                let tsw = actf.replace(".act", "");
                if let Ok(t) = vfs.sprite_frame(&tsw, id, s.frame as usize) {
                    frames.push((t.image, s.delay as f32 * TICK_MS));
                }
            }
            let (w, h) = frames.iter().fold((0, 0), |m, (i, _)| (m.0.max(i.width), m.1.max(i.height)));
            std::fs::create_dir_all(&out).unwrap();
            for (k, (img, _)) in frames.iter().enumerate() {
                let mut c = RgbaImage::new(w, h);
                c.blit(img, ((w - img.width) / 2) as i32, (h - img.height) as i32);
                c.save_png(&out.join(format!("f{k:03}.png"))).unwrap();
            }
            let d: Vec<f32> = frames.iter().map(|f| f.1).collect();
            std::fs::write(out.join("frames.json"), serde_json::json!({ "durations_ms": d, "canvas": [w, h] }).to_string()).unwrap();
            println!("exported {} frames {w}x{h} durations {d:?} -> {}", frames.len(), out.display());
        }
        Some("crossfade") if a.len() == 4 => {
            let (inp, out, n) = (PathBuf::from(&a[1]), PathBuf::from(&a[2]), a[3].parse::<usize>().unwrap());
            let src: Vec<RgbaImage> = pngs(&inp).iter().map(|p| load(p)).collect();
            let durs = durations(&inp, src.len());
            std::fs::create_dir_all(&out).unwrap();
            let (mut k, mut od) = (0, Vec::new());
            for i in 0..src.len() {
                let next = &src[(i + 1) % src.len()];
                for j in 0..=n {
                    let img = if j == 0 { src[i].clone() } else { blend(&src[i], next, j as f32 / (n + 1) as f32) };
                    img.save_png(&out.join(format!("f{k:03}.png"))).unwrap();
                    od.push(durs[i] / (n + 1) as f32);
                    k += 1;
                }
            }
            write_manifest(&out, k, &od, &format!("cross-fade in-betweens x{} (anim_tool crossfade)", n + 1));
            println!("{k} frames -> {}", out.display());
        }
        Some("prep-rife") if a.len() == 3 => {
            let (inp, work) = (PathBuf::from(&a[1]), PathBuf::from(&a[2]));
            let mut src: Vec<RgbaImage> = pngs(&inp).iter().map(|p| load(p)).collect();
            src.push(src[0].clone());
            for d in ["rgb", "alpha"] {
                std::fs::create_dir_all(work.join(d)).unwrap();
            }
            for (k, img) in src.iter().enumerate() {
                let (mut rgb, mut al) = (RgbaImage::new(img.width, img.height), RgbaImage::new(img.width, img.height));
                for i in 0..(img.width * img.height) as usize {
                    let p = &img.pixels[i * 4..i * 4 + 4];
                    let m = p[3] as f32 / 255.0;
                    rgb.pixels[i * 4..i * 4 + 4].copy_from_slice(&[(p[0] as f32 * m) as u8, (p[1] as f32 * m) as u8, (p[2] as f32 * m) as u8, 255]);
                    al.pixels[i * 4..i * 4 + 4].copy_from_slice(&[p[3], p[3], p[3], 255]);
                }
                rgb.save_png(&work.join("rgb").join(format!("{k:08}.png"))).unwrap();
                al.save_png(&work.join("alpha").join(format!("{k:08}.png"))).unwrap();
            }
            println!("{} frames (incl. loop seam) -> {}\\rgb, alpha", src.len(), work.display());
        }
        Some("merge-rife") if a.len() == 4 => {
            let (work, out, total) = (PathBuf::from(&a[1]), PathBuf::from(&a[2]), a[3].parse::<f32>().unwrap());
            let (rgb, al) = (pngs(&work.join("rgb_out")), pngs(&work.join("alpha_out")));
            assert_eq!(rgb.len(), al.len(), "rgb_out / alpha_out frame counts differ");
            let n = rgb.len().saturating_sub(1); // drop loop duplicate
            std::fs::create_dir_all(&out).unwrap();
            for k in 0..n {
                let (c, m) = (load(&rgb[k]), load(&al[k]));
                let mut o = RgbaImage::new(c.width, c.height);
                for i in 0..(c.width * c.height) as usize {
                    let a = m.pixels[i * 4];
                    let f = if a == 0 { 0.0 } else { 255.0 / a as f32 };
                    let px = [0, 1, 2].map(|j| (c.pixels[i * 4 + j] as f32 * f).round().clamp(0.0, 255.0) as u8);
                    o.pixels[i * 4..i * 4 + 4].copy_from_slice(&[px[0], px[1], px[2], if a < 8 { 0 } else { a }]);
                }
                o.save_png(&out.join(format!("f{k:03}.png"))).unwrap();
            }
            let d = vec![total / n.max(1) as f32; n];
            write_manifest(&out, n, &d, "RIFE in-betweens (anim_tool merge-rife)");
            println!("{n} frames -> {}", out.display());
        }
        _ => eprintln!("usage: anim_tool export <game> <act> <slot> <action> <dir> | crossfade <in> <out> <n> | prep-rife <in> <work> | merge-rife <work> <out> <total_ms>"),
    }
}