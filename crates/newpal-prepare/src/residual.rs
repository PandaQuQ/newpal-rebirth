//! Residual / delta HD patches.
//!
//! A patch stores only `HD - baseline(original)`, keyed by sha256 of the player's own
//! original frame. It contains no original and no HD pixels; it is useless without the
//! matching original data. Baseline is a bit-exact **CPU** Catmull-Rom bicubic 4x
//! (fixed-point, not GPU-dependent) so build and apply produce the same bytes.
use newpal_assets::image::RgbaImage;
use sha2::{Digest, Sha256};
use serde::{Deserialize, Serialize};
use rayon::prelude::*;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

pub const SCALE: u32 = 4;
pub const ALGO: u32 = 1;
pub const BASELINE: &str = "bicubic-catmull-rom-fp16.16";
pub const TSW_DEFAULT: &[&str] = &["all_char", "all_item", "all_magic", "all_map1", "all_map2", "all_sys"];

/// sha256 of little-endian width, height, then raw RGBA8 pixels.
pub fn source_hash(img: &RgbaImage) -> String {
    let mut h = Sha256::new();
    h.update(img.width.to_le_bytes());
    h.update(img.height.to_le_bytes());
    h.update(&img.pixels);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Catmull-Rom cubic weight, `t` and return value in 16.16 fixed point.
fn cubic_w(t: i32) -> i32 {
    // w = (1.5|t|^3 - 2.5|t|^2 + 1)                0 <= |t| < 1
    // w = (-0.5|t|^3 + 2.5|t|^2 - 4|t| + 2)        1 <= |t| < 2
    let x = t.abs();
    let x2 = ((x as i64 * x as i64) >> 16) as i32;
    let x3 = ((x2 as i64 * x as i64) >> 16) as i32;
    if x < (1 << 16) {
        // 1.5 x^3 - 2.5 x^2 + 1
        let a15 = ((x3 as i64 * 3) >> 1) as i32;
        let a25 = ((x2 as i64 * 5) >> 1) as i32;
        a15 - a25 + (1 << 16)
    } else if x < (2 << 16) {
        // -0.5 x^3 + 2.5 x^2 - 4x + 2
        let a05 = x3 >> 1;
        let a25 = ((x2 as i64 * 5) >> 1) as i32;
        -a05 + a25 - (x << 2) + (2 << 16)
    } else {
        0
    }
}

fn src_px(img: &RgbaImage, x: i32, y: i32) -> [u8; 4] {
    let x = x.clamp(0, img.width as i32 - 1) as u32;
    let y = y.clamp(0, img.height as i32 - 1) as u32;
    let o = ((y * img.width + x) * 4) as usize;
    [img.pixels[o], img.pixels[o + 1], img.pixels[o + 2], img.pixels[o + 3]]
}

/// Straightforward reference implementation (slow); `bicubic_4x` must match it byte for byte.
pub fn bicubic_4x_reference(src: &RgbaImage) -> RgbaImage {
    if src.width == 0 || src.height == 0 {
        return RgbaImage::new(0, 0);
    }
    let w = src.width * SCALE;
    let h = src.height * SCALE;
    let mut out = RgbaImage::new(w, h);
    // dest (x+0.5)/4 - 0.5  in 16.16
    // fx = ((x * 2 + 1) * (1/4) - 1/2) = (2x + 1)/4 - 1/2 = (2x + 1 - 2)/4 = (2x - 1)/4
    for y in 0..h {
        for x in 0..w {
            let fx = (((x as i64 * 2 - 1) << 16) / 4) as i32;
            let fy = (((y as i64 * 2 - 1) << 16) / 4) as i32;
            let ix = fx >> 16;
            let iy = fy >> 16;
            let tx = fx & 0xffff;
            let ty = fy & 0xffff;
            let mut acc = [0i64; 4];
            let mut wsum = 0i64;
            for j in -1..3 {
                let wy = cubic_w(((j << 16) - ty) as i32) as i64;
                for i in -1..3 {
                    let wx = cubic_w(((i << 16) - tx) as i32) as i64;
                    let wgt = (wx * wy) >> 16;
                    wsum += wgt;
                    let p = src_px(src, ix + i, iy + j);
                    for c in 0..4 {
                        acc[c] += wgt * p[c] as i64;
                    }
                }
            }
            let o = ((y * w + x) * 4) as usize;
            if wsum == 0 {
                continue;
            }
            for c in 0..4 {
                let v = (acc[c] + (wsum / 2)) / wsum;
                out.pixels[o + c] = v.clamp(0, 255) as u8;
            }
        }
    }
    out
}

/// Bit-exact 4x Catmull-Rom bicubic. Same inputs always yield the same bytes on every CPU.
/// Per-phase weights are precomputed (the fractional offset only takes two values for 4x), output
/// rows are computed in parallel; identical to `bicubic_4x_reference`.
pub fn bicubic_4x(src: &RgbaImage) -> RgbaImage {
    if src.width == 0 || src.height == 0 {
        return RgbaImage::new(0, 0);
    }
    let w = (src.width * SCALE) as usize;
    let h = (src.height * SCALE) as usize;
    let (sw, sh) = (src.width as i32, src.height as i32);
    let mut out = RgbaImage::new(w as u32, h as u32);
    // per output coordinate: integer position and index into the distinct-fraction table
    let pos = |x: usize| -> (i32, i32) {
        let fx = (((x as i64 * 2 - 1) << 16) / 4) as i32;
        (fx >> 16, fx & 0xffff)
    };
    let mut fracs: Vec<i32> = Vec::new();
    let mut fidx = |t: i32| -> usize {
        if let Some(i) = fracs.iter().position(|&f| f == t) {
            i
        } else {
            fracs.push(t);
            fracs.len() - 1
        }
    };
    let xs: Vec<(i32, usize)> = (0..w).map(|x| { let (i, t) = pos(x); (i, fidx(t)) }).collect();
    let ys: Vec<(i32, usize)> = (0..h).map(|y| { let (i, t) = pos(y); (i, fidx(t)) }).collect();
    // taps[f][k] for k in -1..3  (index k+1)
    let taps: Vec<[i64; 4]> = fracs
        .iter()
        .map(|&t| {
            let mut a = [0i64; 4];
            for k in -1..3i32 {
                a[(k + 1) as usize] = cubic_w(((k << 16) - t) as i32) as i64;
            }
            a
        })
        .collect();
    // combined weights and sums per (xfrac, yfrac)
    let nf = fracs.len();
    let mut wgt = vec![[[0i64; 4]; 4]; nf * nf];
    let mut wsum = vec![0i64; nf * nf];
    for fy in 0..nf {
        for fx in 0..nf {
            let mut s = 0i64;
            for j in 0..4 {
                for i in 0..4 {
                    let g = (taps[fx][i] * taps[fy][j]) >> 16;
                    wgt[fy * nf + fx][j][i] = g;
                    s += g;
                }
            }
            wsum[fy * nf + fx] = s;
        }
    }
    // clamped source columns per output x
    let cols: Vec<[usize; 4]> = xs
        .iter()
        .map(|&(ix, _)| {
            let mut c = [0usize; 4];
            for i in 0..4 {
                c[i] = (ix + i as i32 - 1).clamp(0, sw - 1) as usize;
            }
            c
        })
        .collect();
    let sp = &src.pixels;
    let swz = src.width as usize;
    out.pixels.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let (iy, fy) = ys[y];
        let rows: [usize; 4] = std::array::from_fn(|j| (iy + j as i32 - 1).clamp(0, sh - 1) as usize);
        for x in 0..w {
            let fx = xs[x].1;
            let k = fy * nf + fx;
            let ws = wsum[k];
            if ws == 0 {
                continue;
            }
            let wg = &wgt[k];
            let c = &cols[x];
            let mut acc = [0i64; 4];
            for j in 0..4 {
                let base = rows[j] * swz;
                for i in 0..4 {
                    let o = (base + c[i]) * 4;
                    let g = wg[j][i];
                    acc[0] += g * sp[o] as i64;
                    acc[1] += g * sp[o + 1] as i64;
                    acc[2] += g * sp[o + 2] as i64;
                    acc[3] += g * sp[o + 3] as i64;
                }
            }
            let o = x * 4;
            for ch in 0..4 {
                row[o + ch] = ((acc[ch] + (ws / 2)) / ws).clamp(0, 255) as u8;
            }
        }
    });
    out
}

/// Lossless residual: 16-bit RGBA PNG, each channel is `wrapping_add` of i16 (hd - base) as u16.
pub fn encode_residual_lossless(hd: &RgbaImage, base: &RgbaImage) -> Result<Vec<u8>, String> {
    if hd.width != base.width || hd.height != base.height {
        return Err("size mismatch".into());
    }
    let n = (hd.width * hd.height) as usize;
    let mut buf = vec![0u8; n * 8];
    for i in 0..n {
        for c in 0..4 {
            let d = hd.pixels[i * 4 + c] as i16 - base.pixels[i * 4 + c] as i16;
            let u = d as u16;
            let o = i * 8 + c * 2;
            buf[o] = (u >> 8) as u8; // png 16-bit is network (BE) order
            buf[o + 1] = u as u8;
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, hd.width, hd.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Sixteen);
        enc.set_compression(png::Compression::High);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&buf).map_err(|e| e.to_string())?;
        w.finish().map_err(|e| e.to_string())?;
    }
    Ok(out)
}

pub fn decode_residual_lossless(bytes: &[u8]) -> Result<(u32, u32, Vec<i16>), String> {
    let dec = png::Decoder::new(std::io::Cursor::new(bytes));
    // do not expand to 8-bit
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    if info.bit_depth != png::BitDepth::Sixteen || info.color_type != png::ColorType::Rgba {
        return Err(format!("unexpected residual png {:?}", info.color_type));
    }
    let (w, h) = (info.width, info.height);
    let size = reader.output_buffer_size().ok_or("png too large")?;
    let mut buf = vec![0u8; size];
    let frame = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let n = (w * h * 4) as usize;
    let mut d = vec![0i16; n];
    let data = &buf[..frame.buffer_size()];
    for i in 0..n {
        let u = u16::from_be_bytes([data[i * 2], data[i * 2 + 1]]);
        d[i] = u as i16;
    }
    Ok((w, h, d))
}

pub fn apply_residual(base: &RgbaImage, residual: &[i16]) -> Result<RgbaImage, String> {
    let n = (base.width * base.height * 4) as usize;
    if residual.len() != n {
        return Err("residual length mismatch".into());
    }
    let mut out = RgbaImage::new(base.width, base.height);
    for i in 0..n {
        let v = base.pixels[i] as i16 + residual[i];
        out.pixels[i] = v.clamp(0, 255) as u8;
    }
    Ok(out)
}

/// 8-bit near-lossless residual: store (d+128).clamp(0,255).
pub fn encode_residual_offset8(hd: &RgbaImage, base: &RgbaImage) -> Result<Vec<u8>, String> {
    if hd.width != base.width || hd.height != base.height {
        return Err("size mismatch".into());
    }
    let mut img = RgbaImage::new(hd.width, hd.height);
    for i in 0..hd.pixels.len() {
        let d = hd.pixels[i] as i16 - base.pixels[i] as i16;
        img.pixels[i] = (d + 128).clamp(0, 255) as u8;
    }
    let mut buf = Vec::new();
    img.write_png_to(&mut buf)?;
    Ok(buf)
}

pub fn apply_residual_offset8(base: &RgbaImage, res: &RgbaImage) -> Result<RgbaImage, String> {
    if base.width != res.width || base.height != res.height {
        return Err("size mismatch".into());
    }
    let mut out = RgbaImage::new(base.width, base.height);
    for i in 0..base.pixels.len() {
        let v = base.pixels[i] as i16 + res.pixels[i] as i16 - 128;
        out.pixels[i] = v.clamp(0, 255) as u8;
    }
    Ok(out)
}

pub fn psnr(a: &RgbaImage, b: &RgbaImage) -> Option<f64> {
    if a.width != b.width || a.height != b.height || a.pixels.is_empty() {
        return None;
    }
    // exact integer sum (identical to the old f64 sum, which was exact below 2^53)
    let sse: u64 = a
        .pixels
        .par_chunks(1 << 16)
        .zip(b.pixels.par_chunks(1 << 16))
        .map(|(x, y)| {
            let mut s = 0u64;
            for (&p, &q) in x.iter().zip(y.iter()) {
                let d = p as i32 - q as i32;
                s += (d * d) as u64;
            }
            s
        })
        .sum();
    if sse == 0 {
        return Some(f64::INFINITY);
    }
    let mse = sse as f64 / a.pixels.len() as f64;
    Some(10.0 * (255.0 * 255.0 / mse).log10())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatchManifest {
    pub version: u32,
    pub algo: u32,
    pub scale: u32,
    pub baseline: String,
    pub encoding: String,
    pub notes: String,
    pub entries: Vec<PatchEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatchEntry {
    pub key: String,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    pub file: String,
    pub bytes: u64,
}

impl PatchManifest {
    pub fn new(encoding: &str) -> Self {
        Self {
            version: 1,
            algo: ALGO,
            scale: SCALE,
            baseline: BASELINE.into(),
            encoding: encoding.into(),
            notes: "Residual only. Useless without the matching original frame (sha256). Contains no original or HD pixels.".into(),
            entries: Vec::new(),
        }
    }
}

/// Convenience: write a PNG via a cursor (RgbaImage::save_png writes a path).
trait WritePng {
    fn write_png_to(&self, w: &mut Vec<u8>) -> Result<(), String>;
}
impl WritePng for RgbaImage {
    fn write_png_to(&self, w: &mut Vec<u8>) -> Result<(), String> {
        // save to temp is messy; encode here
        let mut enc = png::Encoder::new(w, self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::High);
        let mut wr = enc.write_header().map_err(|e| e.to_string())?;
        wr.write_image_data(&self.pixels).map_err(|e| e.to_string())?;
        wr.finish().map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Measure {
    pub frames: usize,
    pub orig_png_bytes: u64,
    pub hd_png_bytes: u64,
    pub lossless_bytes: u64,
    pub offset8_bytes: u64,
    pub lossless_ok: usize,
    pub offset8_psnr_sum: f64,
    pub offset8_psnr_min: f64,
    pub apply_lossless_ms: f64,
    pub packed_bytes: u64,
    pub packed_ok: usize,
    pub packed_psnr_sum: f64,
    pub packed_psnr_min: f64,
    pub packed_q_sum: f64,
    pub apply_packed_ms: f64,
    pub secs: f32,
}

impl Measure {
    pub fn offset8_psnr_avg(&self) -> f64 {
        if self.frames == 0 {
            0.0
        } else {
            self.offset8_psnr_sum / self.frames as f64
        }
    }
    pub fn packed_psnr_avg(&self) -> f64 {
        if self.packed_ok == 0 { 0.0 } else { self.packed_psnr_sum / self.packed_ok as f64 }
    }
    pub fn packed_q_avg(&self) -> f64 {
        if self.packed_ok == 0 { 0.0 } else { self.packed_q_sum / self.packed_ok as f64 }
    }

    pub fn add(&mut self, o: &Measure) {
        self.frames += o.frames;
        self.orig_png_bytes += o.orig_png_bytes;
        self.hd_png_bytes += o.hd_png_bytes;
        self.lossless_bytes += o.lossless_bytes;
        self.offset8_bytes += o.offset8_bytes;
        self.lossless_ok += o.lossless_ok;
        self.offset8_psnr_sum += o.offset8_psnr_sum;
        if o.offset8_psnr_min < self.offset8_psnr_min {
            self.offset8_psnr_min = o.offset8_psnr_min;
        }
        self.apply_lossless_ms += o.apply_lossless_ms;
        self.packed_bytes += o.packed_bytes;
        self.packed_ok += o.packed_ok;
        self.packed_psnr_sum += o.packed_psnr_sum;
        if o.packed_psnr_min < self.packed_psnr_min {
            self.packed_psnr_min = o.packed_psnr_min;
        }
        self.packed_q_sum += o.packed_q_sum;
        self.apply_packed_ms += o.apply_packed_ms;
        self.secs += o.secs;
    }
}

/// Build or measure patches for a list of (logical key, original, hd) frames.

fn reuse_nres(dir: &Path, sha: &str) -> Option<(String, u64)> {
    let rel = format!("res/{sha}.nres");
    let path = dir.join(&rel);
    let meta = std::fs::metadata(&path).ok()?;
    if meta.len() < 28 {
        return None;
    }
    let mut hdr = [0u8; 4];
    let mut f = std::fs::File::open(&path).ok()?;
    f.read_exact(&mut hdr).ok()?;
    if &hdr != crate::residual_pack::MAGIC {
        return None;
    }
    Some((rel, meta.len()))
}

fn write_atomic(dir: &Path, rel: &str, bytes: &[u8]) -> Result<(), String> {
    static N: AtomicU64 = AtomicU64::new(0);
    let tmp = dir.join(format!(
        "{}.{}.tmp",
        rel,
        N.fetch_add(1, Ordering::Relaxed)
    ));
    if let Some(p) = tmp.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join(rel)).map_err(|e| e.to_string())
}

pub fn process_frames(
    items: &[(String, RgbaImage, RgbaImage)],
    out_dir: Option<&Path>,
    encoding: &str,
    workers: usize,
) -> Result<(PatchManifest, Measure), String> {
    if let Some(d) = out_dir {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(d.join("res")).map_err(|e| e.to_string())?;
    }
    let man = Mutex::new(PatchManifest::new(encoding));
    let meas = Mutex::new(Measure {
        offset8_psnr_min: f64::INFINITY,
        packed_psnr_min: f64::INFINITY,
        ..Measure::default()
    });
    let t0 = Instant::now();
    let workers = workers.max(1);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .map_err(|e| e.to_string())?;
    pool.install(|| {
        items.par_iter().for_each(|(key, orig, hd)| {
            let rec = (|| -> Result<(), String> {
                if orig.width == 0 || hd.width == 0 {
                    return Err(format!("{key}: empty"));
                }
                let sha = source_hash(orig);
                if let Some(dir) = out_dir {
                    if let Some((rel, nbytes)) = reuse_nres(dir, &sha) {
                        if hd.width != 0 {
                            // skip encode if a valid packed residual already exists
                            man.lock().unwrap().entries.push(PatchEntry {
                                key: key.clone(),
                                sha256: sha.clone(),
                                width: hd.width,
                                height: hd.height,
                                file: rel,
                                bytes: nbytes,
                            });
                            let mut m = meas.lock().unwrap();
                            m.frames += 1;
                            m.packed_ok += 1;
                            m.packed_bytes += nbytes;
                            return Ok(());
                        }
                    }
                }
                let t_base = Instant::now();
                let base = bicubic_4x(orig);
                if std::env::var_os("NEWPAL_PATCH_TRACE").is_some() && hd.width as u64 * hd.height as u64 >= crate::residual_pack::STRIPE_MIN_PIXELS {
                    eprintln!("[patch]   {key}: baseline {:.1}s", t_base.elapsed().as_secs_f32());
                }
                if base.width != hd.width || base.height != hd.height {
                    return Err(format!(
                        "size {}x{} vs hd {}x{}",
                        base.width, base.height, hd.width, hd.height
                    ));
                }
                {
                    let mut m = meas.lock().unwrap();
                    m.frames += 1;
                }

                if encoding == "offset8" {
                    let bytes = encode_residual_offset8(hd, &base)?;
                    let recov = apply_residual_offset8(&base, &RgbaImage::from_png(&bytes).map_err(|e| e.to_string())?)?;
                    let p = psnr(hd, &recov).unwrap_or(0.0);
                    if let Some(dir) = out_dir {
                        let rel = format!("res/{sha}.png");
                        write_atomic(dir, &rel, &bytes)?;
                        man.lock().unwrap().entries.push(PatchEntry {
                            key: key.clone(),
                            sha256: sha.clone(),
                            width: hd.width,
                            height: hd.height,
                            file: rel,
                            bytes: bytes.len() as u64,
                        });
                    }
                    let mut m = meas.lock().unwrap();
                    m.offset8_bytes += bytes.len() as u64;
                    m.offset8_psnr_sum += p;
                    if p < m.offset8_psnr_min {
                        m.offset8_psnr_min = p;
                    }
                } else if encoding.starts_with("zstd") || encoding.starts_with("brotli") {
                    let opts = match encoding {
                        "zstd" => crate::residual_pack::PackOpts::zstd(),
                        "brotli" => crate::residual_pack::PackOpts::brotli(),
                        "zstd-45" => crate::residual_pack::PackOpts::zstd_psnr(45.0),
                        "brotli-45" => crate::residual_pack::PackOpts::brotli_psnr(45.0),
                        _ => return Err(format!("unknown packed encoding {encoding}")),
                    };
                    let (bytes, q, p) = crate::residual_pack::encode_packed(hd, &base, opts)?;
                    let t_apply = Instant::now();
                    let recov = crate::residual_pack::apply_packed(&base, &bytes)?;
                    let apply_ms = t_apply.elapsed().as_secs_f64() * 1000.0;
                    if opts.psnr_target.is_none() && recov.pixels != hd.pixels {
                        return Err(format!("{key}: packed lossless roundtrip failed"));
                    }
                    if let Some(min_p) = opts.psnr_target {
                        if p < min_p {
                            return Err(format!("{key}: PSNR {p:.2} < {min_p}"));
                        }
                        // end-to-end check: what a player reconstructs must meet the target too
                        let pr = psnr(hd, &recov).unwrap_or(0.0);
                        if pr < min_p {
                            return Err(format!("{key}: reconstructed PSNR {pr:.2} < {min_p}"));
                        }
                    }
                    if let Some(dir) = out_dir {
                        let rel = format!("res/{sha}.nres");
                        write_atomic(dir, &rel, &bytes)?;
                        man.lock().unwrap().entries.push(PatchEntry {
                            key: key.clone(),
                            sha256: sha.clone(),
                            width: hd.width,
                            height: hd.height,
                            file: rel,
                            bytes: bytes.len() as u64,
                        });
                    }
                    // Large images skip the (slow, serial) PNG size stats; the caller adds file sizes.
                    let big = hd.width as u64 * hd.height as u64 >= crate::residual_pack::STRIPE_MIN_PIXELS;
                    let (hd_png_len, orig_png_len) = if big {
                        (0u64, 0u64)
                    } else {
                        let mut hd_png = Vec::new();
                        hd.write_png_to(&mut hd_png)?;
                        let mut orig_png = Vec::new();
                        orig.write_png_to(&mut orig_png)?;
                        (hd_png.len() as u64, orig_png.len() as u64)
                    };
                    let mut m = meas.lock().unwrap();
                    m.packed_bytes += bytes.len() as u64;
                    m.hd_png_bytes += hd_png_len;
                    m.orig_png_bytes += orig_png_len;
                    m.packed_ok += 1;
                    m.packed_psnr_sum += if p.is_finite() { p } else { 99.0 };
                    if p < m.packed_psnr_min { m.packed_psnr_min = p; }
                    m.packed_q_sum += q as f64;
                    m.apply_packed_ms += apply_ms;
                } else {
                    let bytes = encode_residual_lossless(hd, &base)?;
                    let t_apply = Instant::now();
                    let (w, h, d) = decode_residual_lossless(&bytes)?;
                    let recov = apply_residual(&base, &d)?;
                    let apply_ms = t_apply.elapsed().as_secs_f64() * 1000.0;
                    if recov.pixels != hd.pixels || w != hd.width || h != hd.height {
                        return Err(format!("{key}: lossless roundtrip failed"));
                    }
                    if let Some(dir) = out_dir {
                        let rel = format!("res/{sha}.png");
                        write_atomic(dir, &rel, &bytes)?;
                        man.lock().unwrap().entries.push(PatchEntry {
                            key: key.clone(),
                            sha256: sha.clone(),
                            width: hd.width,
                            height: hd.height,
                            file: rel,
                            bytes: bytes.len() as u64,
                        });
                    }
                    let mut m = meas.lock().unwrap();
                    m.lossless_bytes += bytes.len() as u64;
                    m.lossless_ok += 1;
                    m.apply_lossless_ms += apply_ms;
                }
                Ok(())
            })();
            if let Err(e) = rec {
                eprintln!("[patch] {e}");
            }
        });
    });
        let mut man = man.into_inner().unwrap();
    let mut meas = meas.into_inner().unwrap();
    meas.secs = t0.elapsed().as_secs_f32();
    man.entries.sort_by(|a, b| a.key.cmp(&b.key));
    // Caller writes the merged manifest atomically. Writing it here would
    // clobber a resume manifest with only this batch's entries.
    let _ = out_dir;
    let _ = PathBuf::new();
    Ok((man, meas))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bicubic_fast_matches_reference() {
        let mut src = RgbaImage::new(37, 23);
        let mut s = 12345u32;
        for p in src.pixels.iter_mut() {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            *p = (s >> 24) as u8;
        }
        assert_eq!(bicubic_4x(&src).pixels, bicubic_4x_reference(&src).pixels);
        let one = solid(1, 1, [9, 8, 7, 6]);
        assert_eq!(bicubic_4x(&one).pixels, bicubic_4x_reference(&one).pixels);
        let thin = solid(5, 1, [200, 0, 255, 128]);
        assert_eq!(bicubic_4x(&thin).pixels, bicubic_4x_reference(&thin).pixels);
    }

    fn solid(w: u32, h: u32, px: [u8; 4]) -> RgbaImage {
        let mut i = RgbaImage::new(w, h);
        for p in i.pixels.chunks_exact_mut(4) {
            p.copy_from_slice(&px);
        }
        i
    }

    #[test]
    fn bicubic_deterministic_and_lossless_roundtrip() {
        let mut src = RgbaImage::new(5, 7);
        for y in 0..7u32 {
            for x in 0..5u32 {
                src.put(x, y, [(x * 40) as u8, (y * 30) as u8, 128, if x == 0 { 0 } else { 255 }]);
            }
        }
        let a = bicubic_4x(&src);
        let b = bicubic_4x(&src);
        assert_eq!(a.width, 20);
        assert_eq!(a.height, 28);
        assert_eq!(a.pixels, b.pixels);
        // pretend HD is baseline + a known offset
        let mut hd = a.clone();
        for i in 0..hd.pixels.len() {
            hd.pixels[i] = hd.pixels[i].saturating_add((i % 13) as u8);
        }
        let enc = encode_residual_lossless(&hd, &a).unwrap();
        let (_, _, d) = decode_residual_lossless(&enc).unwrap();
        let rec = apply_residual(&a, &d).unwrap();
        assert_eq!(rec.pixels, hd.pixels);
        assert_eq!(psnr(&hd, &rec), Some(f64::INFINITY));
        // hash is stable
        assert_eq!(source_hash(&src), source_hash(&src));
        assert_ne!(source_hash(&src), source_hash(&solid(5, 7, [1, 2, 3, 255])));
    }

    #[test]
    fn packed_zstd_roundtrip_and_psnr45() {
        let mut src = RgbaImage::new(8, 8);
        for (i, p) in src.pixels.iter_mut().enumerate() { *p = (i * 13) as u8; }
        let base = bicubic_4x(&src);
        let mut hd = base.clone();
        for i in 0..hd.pixels.len() {
            hd.pixels[i] = hd.pixels[i].saturating_add((i % 7) as u8);
        }
        let (bytes, q, _p) = crate::residual_pack::encode_packed(&hd, &base, crate::residual_pack::PackOpts::zstd()).unwrap();
        let rec = crate::residual_pack::apply_packed(&base, &bytes).unwrap();
        assert_eq!(rec.pixels, hd.pixels);
        assert_eq!(q, 1);
        let (b45, q45, p45) = crate::residual_pack::encode_packed(&hd, &base, crate::residual_pack::PackOpts::zstd_psnr(45.0)).unwrap();
        assert!(p45 >= 45.0 || p45.is_infinite(), "psnr {p45}");
        let rec45 = crate::residual_pack::apply_packed(&base, &b45).unwrap();
        let p = psnr(&hd, &rec45).unwrap();
        assert!(p >= 45.0 || p.is_infinite(), "apply psnr {p} q={q45} enc={} vs lossless={}", b45.len(), bytes.len());
    }
}
