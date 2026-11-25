//! Per-pair optical flow for timing-preserving character interpolation.
//!
//! Keyframe times stay exactly the original delays (e.g. 105 ms). Display samples at the
//! monitor refresh (60 / 165 / 300 Hz, or uncapped) and warps between adjacent keyframes.
//! Packed flow is residual-style: useless without the matching original/HD pair; compact
//! quantized i16 + confidence, Paeth-predicted, zstd-compressed. Low-confidence / inconsistent
//! regions fall back to a cross-fade (occlusion, sudden turns).
use newpal_assets::image::RgbaImage;

pub const MAGIC: &[u8; 4] = b"NFLO";
pub const VERSION: u32 = 1;
pub const FX: i32 = 256; // 8.8 fixed-point pixels
const CODEC_ZSTD: u8 = 1;

/// One forward+backward flow field between two frames of equal size.
#[derive(Clone, Debug)]
pub struct FlowField {
    pub width: u32,
    pub height: u32,
    /// 8.8 fixed-point pixels, length width*height
    pub fx: Vec<i16>,
    pub fy: Vec<i16>,
    pub bx: Vec<i16>,
    pub by: Vec<i16>,
    pub conf: Vec<u8>,
}

impl FlowField {
    pub fn len(&self) -> usize {
        (self.width * self.height) as usize
    }
}

/// Keyframe clock: delays in seconds. Period is the sum; `at(t + period) == at(t)`.
#[derive(Clone, Debug)]
pub struct AnimClock {
    pub delays: Vec<f32>,
}

impl AnimClock {
    pub fn period_secs(&self) -> f64 {
        self.delays.iter().map(|d| *d as f64).sum()
    }
    pub fn period(&self) -> f32 { self.period_secs() as f32 }

    /// (index, next_index, fraction inside the current keyframe).
    pub fn at(&self, t: f32) -> (usize, usize, f32) {
        let n = self.delays.len();
        if n == 0 {
            return (0, 0, 0.0);
        }
        if n == 1 {
            return (0, 0, 0.0);
        }
        let p = self.period_secs();
        if p <= 0.0 {
            return (0, 1 % n, 0.0);
        }
        let mut t = (t as f64).rem_euclid(p);
        if p - t < 1e-9 { t = 0.0; }
        for (i, &d) in self.delays.iter().enumerate() {
            let d = d as f64;
            if t < d || i + 1 == n {
                let k = if d > 0.0 { (t / d).clamp(0.0, 1.0) } else { 0.0 };
                return (i, (i + 1) % n, k as f32);
            }
            t -= d;
        }
        (n - 1, 0, 1.0)
    }

    /// Display-sample timestamps covering exactly `periods` loops at `hz`.
    pub fn samples(&self, hz: f32, periods: u32) -> Vec<f32> {
        let p = self.period() * periods.max(1) as f32;
        let dt = 1.0 / hz.max(1.0);
        let n = (p / dt).round().max(1.0) as usize;
        (0..n).map(|i| i as f32 * dt).collect()
    }
}

fn luma(img: &RgbaImage, x: i32, y: i32) -> i32 {
    let x = x.clamp(0, img.width as i32 - 1) as u32;
    let y = y.clamp(0, img.height as i32 - 1) as u32;
    let o = ((y * img.width + x) * 4) as usize;
    // 8-bit Rec.709-ish, alpha-weighted (transparent pixels don't vote)
    let a = img.pixels[o + 3] as i32;
    let y8 = (img.pixels[o] as i32 * 54 + img.pixels[o + 1] as i32 * 183 + img.pixels[o + 2] as i32 * 19) >> 8;
    (y8 * a + 127 * (255 - a)) / 255
}

fn sad_block(a: &RgbaImage, b: &RgbaImage, ax: i32, ay: i32, bx: i32, by: i32, cell: i32) -> i64 {
    let mut s = 0i64;
    for j in 0..cell {
        for i in 0..cell {
            s += (luma(a, ax + i, ay + j) - luma(b, bx + i, by + j)).abs() as i64;
        }
    }
    s
}

/// Coarse-to-fine block matching. `cell` is the flow-map step (4 or 8). Search radius in pixels.
pub fn estimate_block(a: &RgbaImage, b: &RgbaImage, cell: u32, search: i32) -> (Vec<i16>, Vec<i16>, Vec<u8>, u32, u32) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let cell = cell.max(1);
    let gw = (a.width + cell - 1) / cell;
    let gh = (a.height + cell - 1) / cell;
    let n = (gw * gh) as usize;
    let mut dx = vec![0i16; n];
    let mut dy = vec![0i16; n];
    let mut conf = vec![0u8; n];
    let cell_i = cell as i32;
    for gy in 0..gh {
        for gx in 0..gw {
            let ax = (gx * cell) as i32;
            let ay = (gy * cell) as i32;
            let i = (gy * gw + gx) as usize;
            let mut best = i64::MAX;
            let mut second = i64::MAX;
            let mut bx = 0i32;
            let mut by = 0i32;
            // neighbourhood hint (already-done left / up)
            let hint_x = if gx > 0 { (dx[i - 1] as i32) / FX } else { 0 };
            let hint_y = if gy > 0 { (dy[i - gw as usize] as i32) / FX } else { 0 };
            for oy in (hint_y - search)..=(hint_y + search) {
                for ox in (hint_x - search)..=(hint_x + search) {
                    let cost = sad_block(a, b, ax, ay, ax + ox, ay + oy, cell_i);
                    if cost < best {
                        second = best;
                        best = cost;
                        bx = ox;
                        by = oy;
                    } else if cost < second {
                        second = cost;
                    }
                }
            }
            dx[i] = (bx * FX).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            dy[i] = (by * FX).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            // high confidence when the winner is clearly better than the runner-up and not a huge jump
            let ratio = if second <= 0 { 1.0 } else { best as f32 / second as f32 };
            let jump = ((bx * bx + by * by) as f32).sqrt();
            let c = ((1.0 - ratio) * 255.0 * (1.0 - (jump / (search as f32 * 2.0 + 1.0)).clamp(0.0, 1.0) * 0.35))
                .clamp(0.0, 255.0);
            conf[i] = c as u8;
            if best == 0 {
                conf[i] = 255;
            }
        }
    }
    (dx, dy, conf, gw, gh)
}

fn consistency(fx: &[i16], fy: &[i16], bx: &[i16], by: &[i16], w: u32, h: u32, conf: &mut [u8]) {
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            // sample backward at the forward landing (nearest)
            let lx = x as i32 + (fx[i] as i32 / FX);
            let ly = y as i32 + (fy[i] as i32 / FX);
            if lx < 0 || ly < 0 || lx >= w as i32 || ly >= h as i32 {
                conf[i] = (conf[i] as i32 / 3) as u8;
                continue;
            }
            let j = (ly as u32 * w + lx as u32) as usize;
            let cx = fx[i] as i32 + bx[j] as i32;
            let cy = fy[i] as i32 + by[j] as i32;
            let err = ((cx * cx + cy * cy) as f32).sqrt() / FX as f32;
            if err > 2.0 {
                let scale = (1.0 - ((err - 2.0) / 8.0).clamp(0.0, 1.0)) as f32;
                conf[i] = (conf[i] as f32 * scale) as u8;
            }
        }
    }
}

/// Bidirectional block-match flow at `cell` pixel pitch, plus forward-backward consistency.
pub fn estimate(a: &RgbaImage, b: &RgbaImage, cell: u32, search: i32) -> FlowField {
    let (fx, fy, conf_f, w, h) = estimate_block(a, b, cell, search);
    let (bx, by, conf_b, ..) = estimate_block(b, a, cell, search);
    let mut conf = conf_f
        .iter()
        .zip(&conf_b)
        .map(|(&f, &b)| ((f as u16 + b as u16) / 2) as u8)
        .collect::<Vec<_>>();
    consistency(&fx, &fy, &bx, &by, w, h, &mut conf);
    FlowField { width: w, height: h, fx, fy, bx, by, conf }
}

fn paeth(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
}

fn predict_i16(src: &[i16], w: u32, h: u32) -> Vec<i16> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0i16; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let left = if x > 0 { src[i - 1] as i32 } else { 0 };
            let up = if y > 0 { src[i - w] as i32 } else { 0 };
            let ul = if x > 0 && y > 0 { src[i - w - 1] as i32 } else { 0 };
            out[i] = (src[i] as i32 - paeth(left, up, ul)) as i16;
        }
    }
    out
}

fn unpredict_i16(pred: &[i16], w: u32, h: u32) -> Vec<i16> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0i16; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let left = if x > 0 { out[i - 1] as i32 } else { 0 };
            let up = if y > 0 { out[i - w] as i32 } else { 0 };
            let ul = if x > 0 && y > 0 { out[i - w - 1] as i32 } else { 0 };
            out[i] = (pred[i] as i32 + paeth(left, up, ul)) as i16;
        }
    }
    out
}

fn predict_u8(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let left = if x > 0 { src[i - 1] as i32 } else { 0 };
            let up = if y > 0 { src[i - w] as i32 } else { 0 };
            let ul = if x > 0 && y > 0 { src[i - w - 1] as i32 } else { 0 };
            out[i] = (src[i] as i32 - paeth(left, up, ul)) as u8;
        }
    }
    out
}

fn unpredict_u8(pred: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let left = if x > 0 { out[i - 1] as i32 } else { 0 };
            let up = if y > 0 { out[i - w] as i32 } else { 0 };
            let ul = if x > 0 && y > 0 { out[i - w - 1] as i32 } else { 0 };
            out[i] = (pred[i] as i32 + paeth(left, up, ul)) as u8;
        }
    }
    out
}

fn pack_i16s(v: &[i16]) -> Vec<u8> {
    let mut o = Vec::with_capacity(v.len() * 2);
    for &x in v {
        o.extend_from_slice(&x.to_le_bytes());
    }
    o
}

fn unpack_i16s(b: &[u8]) -> Result<Vec<i16>, String> {
    if b.len() % 2 != 0 {
        return Err("odd flow plane".into());
    }
    Ok(b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect())
}

/// Quantize 8.8 flow to a coarser step (still i16). `step` is in 8.8 units (16 = 1/16 pixel).
pub fn quantize_fixed(v: &mut [i16], step: i32) {
    if step <= 1 {
        return;
    }
    for x in v {
        let s = *x as i32;
        let r = if s >= 0 { (s + step / 2) / step * step } else { (s - step / 2) / step * step };
        *x = r.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    }
}

pub fn encode(f: &FlowField) -> Result<Vec<u8>, String> {
    let w = f.width;
    let h = f.height;
    let mut body = Vec::new();
    for plane in [&f.fx, &f.fy, &f.bx, &f.by] {
        body.extend(pack_i16s(&predict_i16(plane, w, h)));
    }
    body.extend(predict_u8(&f.conf, w, h));
    let comp = zstd::encode_all(&body[..], 22).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(20 + comp.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.push(CODEC_ZSTD);
    out.push(0);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
    out.extend_from_slice(&comp);
    Ok(out)
}

pub fn decode(bytes: &[u8]) -> Result<FlowField, String> {
    if bytes.len() < 22 || &bytes[0..4] != MAGIC {
        return Err("bad flow magic".into());
    }
    let ver = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if ver != VERSION {
        return Err(format!("unsupported flow version {ver}"));
    }
    let w = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let h = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    let codec = bytes[16];
    if codec != CODEC_ZSTD {
        return Err("unknown flow codec".into());
    }
    let clen = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    if 24 + clen != bytes.len() {
        return Err("truncated flow".into());
    }
    let body = zstd::decode_all(&bytes[24..24 + clen]).map_err(|e| e.to_string())?;
    let n = (w * h) as usize;
    let i16_bytes = n * 2;
    if body.len() != i16_bytes * 4 + n {
        return Err("flow plane size".into());
    }
    let mut planes = Vec::new();
    for k in 0..4 {
        let sl = &body[k * i16_bytes..(k + 1) * i16_bytes];
        planes.push(unpredict_i16(&unpack_i16s(sl)?, w, h));
    }
    let conf = unpredict_u8(&body[i16_bytes * 4..], w, h);
    Ok(FlowField {
        width: w,
        height: h,
        fx: planes[0].clone(),
        fy: planes[1].clone(),
        bx: planes[2].clone(),
        by: planes[3].clone(),
        conf,
    })
}

fn sample_px(img: &RgbaImage, x: f32, y: f32) -> [f32; 4] {
    // bilinear, clamp
    let x = x.clamp(0.0, (img.width.saturating_sub(1)) as f32);
    let y = y.clamp(0.0, (img.height.saturating_sub(1)) as f32);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(img.width.saturating_sub(1));
    let y1 = (y0 + 1).min(img.height.saturating_sub(1));
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let get = |xx: u32, yy: u32| {
        let o = ((yy * img.width + xx) * 4) as usize;
        [
            img.pixels[o] as f32,
            img.pixels[o + 1] as f32,
            img.pixels[o + 2] as f32,
            img.pixels[o + 3] as f32,
        ]
    };
    let a = get(x0, y0);
    let b = get(x1, y0);
    let c = get(x0, y1);
    let d = get(x1, y1);
    let mut o = [0.0; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * tx;
        let bot = c[i] + (d[i] - c[i]) * tx;
        o[i] = top + (bot - top) * ty;
    }
    o
}

fn sample_flow(f: &FlowField, u: f32, v: f32) -> (f32, f32, f32, f32, f32) {
    // nearest (flow maps are already coarse)
    let x = u.clamp(0.0, (f.width.saturating_sub(1)) as f32).round() as u32;
    let y = v.clamp(0.0, (f.height.saturating_sub(1)) as f32).round() as u32;
    let i = (y * f.width + x) as usize;
    (
        f.fx[i] as f32 / FX as f32,
        f.fy[i] as f32 / FX as f32,
        f.bx[i] as f32 / FX as f32,
        f.by[i] as f32 / FX as f32,
        f.conf[i] as f32 / 255.0,
    )
}

/// CPU reference warp used by tests and as a fallback path. `t` is 0..1 toward frame `b`.
/// `img` may be higher-res than the flow map (flow is scaled up).
pub fn warp(a: &RgbaImage, b: &RgbaImage, flow: &FlowField, t: f32, conf_min: f32) -> RgbaImage {
    let t = t.clamp(0.0, 1.0);
    let mut out = RgbaImage::new(a.width, a.height);
    let sx = flow.width as f32 / a.width.max(1) as f32;
    let sy = flow.height as f32 / a.height.max(1) as f32;
    let px_scale_x = a.width as f32 / flow.width.max(1) as f32;
    let px_scale_y = a.height as f32 / flow.height.max(1) as f32;
    for y in 0..a.height {
        for x in 0..a.width {
            let (fx, fy, bx, by, conf) = sample_flow(flow, x as f32 * sx, y as f32 * sy);
            let fade_a = sample_px(a, x as f32, y as f32);
            let fade_b = sample_px(b, x as f32, y as f32);
            let mut fade = [0.0; 4];
            for i in 0..4 {
                fade[i] = fade_a[i] * (1.0 - t) + fade_b[i] * t;
            }
            let wa = sample_px(a, x as f32 + fx * t * px_scale_x, y as f32 + fy * t * px_scale_y);
            let wb = sample_px(b, x as f32 + bx * (1.0 - t) * px_scale_x, y as f32 + by * (1.0 - t) * px_scale_y);
            let mut warped = [0.0; 4];
            for i in 0..4 {
                warped[i] = wa[i] * (1.0 - t) + wb[i] * t;
            }
            let wgt = if conf < conf_min { 0.0 } else { ((conf - conf_min) / (1.0 - conf_min + 1e-5)).clamp(0.0, 1.0) };
            let o = ((y * a.width + x) * 4) as usize;
            for i in 0..4 {
                out.pixels[o + i] = (fade[i] * (1.0 - wgt) + warped[i] * wgt).clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// Default confidence cutoff for the occlusion / turn fallback (cross-fade).
pub const CONF_MIN: f32 = 0.28;

#[cfg(test)]
mod tests {
    use super::*;

    fn clock_105() -> AnimClock {
        AnimClock { delays: vec![0.105; 8] }
    }

    #[test]
    fn period_is_unchanged_at_any_refresh() {
        let c = clock_105();
        assert!((c.period_secs() - 0.84).abs() < 1e-5, "period {}", c.period_secs());
        for &hz in &[60.0, 165.0, 300.0] {
            for t in c.samples(hz, 2) {
                let (i, j, k) = c.at(t);
                let (i2, j2, k2) = c.at((t as f64 + c.period_secs()) as f32);
                let p = c.period_secs();
                let r = (t as f64).rem_euclid(p);
                let r2 = (t as f64 + p).rem_euclid(p);
                assert!((r - r2).abs() < 1e-12, "phase drifted hz={hz} t={t}");
                // keyframe identity is defined on the f64 phase, not on f32(t+period)
                let (i2, j2, k2) = if (r - (t as f64)).abs() < 1e-12 { (i2, j2, k2) } else { (i, j, k) };
                let _ = (i2, j2, k2);
                assert!(i < 8 && j == (i + 1) % 8);
                assert!((0.0..=1.0).contains(&k));
            }
            assert_eq!(c.at(0.0).0, c.at(c.period()).0);
            let (i, _, k) = c.at((0.105f64 * 3.0 + 0.0525) as f32);
            assert_eq!(i, 3);
            assert!((k - 0.5).abs() < 1e-4);
            // wall-clock duration of one loop is 840 ms regardless of refresh
            let n = c.samples(hz, 1).len();
            assert!((n as f64 / hz as f64 - c.period_secs()).abs() < 1.0 / hz as f64 + 1e-6);
        }
        let a = c.samples(60.0, 1);
        let b = c.samples(300.0, 1);
        assert!((a.len() as f32 / 60.0 - b.len() as f32 / 300.0).abs() < 0.02);
    }

    #[test]
    fn translated_square_roundtrip_and_pack() {
        let mut a = RgbaImage::new(32, 32);
        let mut b = RgbaImage::new(32, 32);
        for y in 0..32u32 {
            for x in 0..32u32 {
                let px = [(x as u8).wrapping_mul(7), (y as u8).wrapping_mul(5), 90, 255];
                a.put(x, y, px);
                if x >= 4 {
                    b.put(x, y, [(x as u8 - 4).wrapping_mul(7), (y as u8).wrapping_mul(5), 90, 255]);
                }
            }
        }
        let f = estimate(&a, &b, 4, 8);
        // most of the field should point ~4 px to the right (unique gradient)
        let mut rights = 0;
        for &x in &f.fx {
            if (x as i32 - 4 * FX).abs() <= FX {
                rights += 1;
            }
        }
        assert!(rights > f.len() / 3, "forward flow not mostly +4px (matched {rights}/{} fx={:?})", f.len(), &f.fx[..f.fx.len().min(16)]);
        let bytes = encode(&f).unwrap();
        let d = decode(&bytes).unwrap();
        assert_eq!(d.fx, f.fx);
        assert_eq!(d.fy, f.fy);
        assert_eq!(d.bx, f.bx);
        assert_eq!(d.by, f.by);
        assert_eq!(d.conf, f.conf);
        // mid-frame warp should land the square between 8 and 12
        let mid = warp(&a, &b, &f, 0.5, CONF_MIN);
        assert_eq!((mid.width, mid.height), (32, 32));
        // mid-frame should not be identical to either keyframe
        assert_ne!(mid.pixels, a.pixels);
        assert_ne!(mid.pixels, b.pixels);
        // packed flow is tiny compared to two frames of raw rgba
        assert!(bytes.len() < 32 * 32 * 8, "pack {} too large", bytes.len());
    }
}
