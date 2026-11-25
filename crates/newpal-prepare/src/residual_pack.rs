//! Packed residual: predicted signed i16 + zstd/brotli, alpha separate.
use crate::residual::psnr;
use newpal_assets::image::RgbaImage;
use rayon::prelude::*;
use std::io::Write;

pub const MAGIC: &[u8; 4] = b"NPR1";
pub const CODEC_ZSTD: u8 = 1;
pub const CODEC_BROTLI: u8 = 2;
pub const FLAG_QUANT: u8 = 1;
pub const FLAG_PAETH: u8 = 2;
pub const FLAG_ALPHA_SEP: u8 = 4;
/// v3: image is cut into independent horizontal stripes (parallel encode/decode).
pub const FLAG_STRIPED: u8 = 8;
/// Images with at least this many pixels use the striped v3 layout; smaller ones keep v2.
pub const STRIPE_MIN_PIXELS: u64 = 1 << 22;
pub const STRIPE_ROWS: u32 = 128;

#[derive(Clone, Copy, Debug)]
pub struct PackOpts {
    pub codec: u8,
    pub psnr_target: Option<f64>,
}

impl PackOpts {
    pub fn zstd() -> Self { Self { codec: CODEC_ZSTD, psnr_target: None } }
    pub fn brotli() -> Self { Self { codec: CODEC_BROTLI, psnr_target: None } }
    pub fn zstd_psnr(p: f64) -> Self { Self { codec: CODEC_ZSTD, psnr_target: Some(p) } }
    pub fn brotli_psnr(p: f64) -> Self { Self { codec: CODEC_BROTLI, psnr_target: Some(p) } }
}

fn paeth(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
}

/// Predict-and-delta a plane of i16 (row-major). Inverse is also here.
fn predict_plane(src: &[i16], w: u32, h: u32) -> Vec<i16> {
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

fn unpredict_plane(pred: &[i16], w: u32, h: u32) -> Vec<i16> {
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

fn quantize_plane(src: &[i16], q: i32) -> Vec<i16> {
    if q <= 1 {
        return src.to_vec();
    }
    src.iter()
        .map(|&d| {
            let s = d as i32;
            let r = if s >= 0 { (s + q / 2) / q * q } else { (s - q / 2) / q * q };
            r as i16
        })
        .collect()
}

fn planes_from_residual(hd: &RgbaImage, base: &RgbaImage) -> (Vec<i16>, Vec<i16>, Vec<i16>, Vec<i16>) {
    let n = (hd.width * hd.height) as usize;
    let mut r = vec![0i16; n];
    let mut g = vec![0i16; n];
    let mut b = vec![0i16; n];
    let mut a = vec![0i16; n];
    for i in 0..n {
        r[i] = hd.pixels[i * 4] as i16 - base.pixels[i * 4] as i16;
        g[i] = hd.pixels[i * 4 + 1] as i16 - base.pixels[i * 4 + 1] as i16;
        b[i] = hd.pixels[i * 4 + 2] as i16 - base.pixels[i * 4 + 2] as i16;
        a[i] = hd.pixels[i * 4 + 3] as i16 - base.pixels[i * 4 + 3] as i16;
    }
    (r, g, b, a)
}

fn apply_planes(base: &RgbaImage, r: &[i16], g: &[i16], b: &[i16], a: &[i16]) -> RgbaImage {
    let mut out = RgbaImage::new(base.width, base.height);
    let n = (base.width * base.height) as usize;
    for i in 0..n {
        out.pixels[i * 4] = (base.pixels[i * 4] as i16 + r[i]).clamp(0, 255) as u8;
        out.pixels[i * 4 + 1] = (base.pixels[i * 4 + 1] as i16 + g[i]).clamp(0, 255) as u8;
        out.pixels[i * 4 + 2] = (base.pixels[i * 4 + 2] as i16 + b[i]).clamp(0, 255) as u8;
        out.pixels[i * 4 + 3] = (base.pixels[i * 4 + 3] as i16 + a[i]).clamp(0, 255) as u8;
    }
    out
}

fn psnr_planes(hd: &RgbaImage, rec: &RgbaImage) -> f64 {
    psnr(hd, rec).unwrap_or(0.0)
}

fn choose_q(hd: &RgbaImage, base: &RgbaImage, r: &[i16], g: &[i16], b: &[i16], a: &[i16], target: f64) -> i32 {
    let mut best = 1i32;
    for q in 1..=32 {
        let rec = apply_planes(
            base,
            &quantize_plane(r, q),
            &quantize_plane(g, q),
            &quantize_plane(b, q),
            &quantize_plane(a, q),
        );
        let p = psnr_planes(hd, &rec);
        // keep raising q while we still meet the target (including a perfect q=1)
        if p < target {
            break;
        }
        best = q;
    }
    best
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
        return Err("odd residual".into());
    }
    Ok(b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect())
}

fn compress(codec: u8, data: &[u8]) -> Result<Vec<u8>, String> {
    match codec {
        CODEC_ZSTD => zstd::encode_all(data, 22).map_err(|e| e.to_string()),
        CODEC_BROTLI => {
            let mut out = Vec::new();
            let mut params = brotli::enc::BrotliEncoderParams::default();
            params.quality = 11;
            params.lgwin = 22;
            let mut w = brotli::CompressorWriter::with_params(&mut out, 4096, &params);
            w.write_all(data).map_err(|e| e.to_string())?;
            w.flush().map_err(|e| e.to_string())?;
            drop(w);
            Ok(out)
        }
        _ => Err("unknown codec".into()),
    }
}

fn decompress(codec: u8, data: &[u8]) -> Result<Vec<u8>, String> {
    match codec {
        CODEC_ZSTD => zstd::decode_all(data).map_err(|e| e.to_string()),
        CODEC_BROTLI => {
            let mut out = Vec::new();
            let mut d = brotli::Decompressor::new(data, 4096);
            std::io::copy(&mut d, &mut out).map_err(|e| e.to_string())?;
            Ok(out)
        }
        _ => Err("unknown codec".into()),
    }
}

pub fn encode_packed(hd: &RgbaImage, base: &RgbaImage, opts: PackOpts) -> Result<(Vec<u8>, i32, f64), String> {
    if hd.width != base.width || hd.height != base.height {
        return Err("size mismatch".into());
    }
    if hd.width as u64 * hd.height as u64 >= STRIPE_MIN_PIXELS {
        return encode_packed_striped(hd, base, opts);
    }
    encode_packed_legacy(hd, base, opts)
}

fn encode_packed_legacy(hd: &RgbaImage, base: &RgbaImage, opts: PackOpts) -> Result<(Vec<u8>, i32, f64), String> {
    let (mut r, mut g, mut b, mut a) = planes_from_residual(hd, base);
    let q = if let Some(t) = opts.psnr_target {
        choose_q(hd, base, &r, &g, &b, &a, t)
    } else {
        1
    };
    if q > 1 {
        r = quantize_plane(&r, q);
        g = quantize_plane(&g, q);
        b = quantize_plane(&b, q);
        a = quantize_plane(&a, q);
    }
    let rec = apply_planes(base, &r, &g, &b, &a);
    let p = psnr_planes(hd, &rec);
    let pr = predict_plane(&r, hd.width, hd.height);
    let pg = predict_plane(&g, hd.width, hd.height);
    let pb = predict_plane(&b, hd.width, hd.height);
    let pa = predict_plane(&a, hd.width, hd.height);
    let mut rgb = pack_i16s(&pr);
    rgb.extend(pack_i16s(&pg));
    rgb.extend(pack_i16s(&pb));
    let al = pack_i16s(&pa);
    let rgb_c = compress(opts.codec, &rgb)?;
    let a_c = compress(opts.codec, &al)?;
    let mut out = Vec::with_capacity(24 + rgb_c.len() + a_c.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&2u32.to_le_bytes()); // version
    out.push(opts.codec);
    let mut flags = FLAG_PAETH | FLAG_ALPHA_SEP;
    if q > 1 {
        flags |= FLAG_QUANT;
    }
    out.push(flags);
    out.extend_from_slice(&(q as u16).to_le_bytes());
    out.extend_from_slice(&hd.width.to_le_bytes());
    out.extend_from_slice(&hd.height.to_le_bytes());
    out.extend_from_slice(&(rgb_c.len() as u32).to_le_bytes());
    out.extend_from_slice(&(a_c.len() as u32).to_le_bytes());
    out.extend_from_slice(&rgb_c);
    out.extend_from_slice(&a_c);
    Ok((out, q, p))
}

/// v2 (non-striped) encoder, kept callable for tests and for sprites.
pub fn encode_packed_v2(hd: &RgbaImage, base: &RgbaImage, opts: PackOpts) -> Result<(Vec<u8>, i32, f64), String> {
    encode_packed_legacy(hd, base, opts)
}

pub fn decode_packed(bytes: &[u8]) -> Result<(u32, u32, Vec<i16>, Vec<i16>, Vec<i16>, Vec<i16>), String> {
    if bytes.len() < 28 || &bytes[0..4] != MAGIC {
        return Err("bad magic".into());
    }
    let ver = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if ver == 3 {
        return decode_packed_striped(bytes);
    }
    if ver != 2 {
        return Err(format!("unsupported residual version {ver}"));
    }
    let codec = bytes[8];
    let flags = bytes[9];
    if flags & FLAG_PAETH == 0 || flags & FLAG_ALPHA_SEP == 0 {
        return Err("unexpected flags".into());
    }
    let w = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    let h = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let rgb_len = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let a_len = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
    if 28 + rgb_len + a_len != bytes.len() {
        return Err("truncated residual".into());
    }
    let rgb = decompress(codec, &bytes[28..28 + rgb_len])?;
    let al = decompress(codec, &bytes[28 + rgb_len..28 + rgb_len + a_len])?;
    let n = (w * h) as usize;
    let rgb16 = unpack_i16s(&rgb)?;
    if rgb16.len() != n * 3 {
        return Err("rgb plane size".into());
    }
    let a16 = unpack_i16s(&al)?;
    if a16.len() != n {
        return Err("alpha plane size".into());
    }
    let r = unpredict_plane(&rgb16[0..n], w, h);
    let g = unpredict_plane(&rgb16[n..2 * n], w, h);
    let b = unpredict_plane(&rgb16[2 * n..3 * n], w, h);
    let a = unpredict_plane(&a16, w, h);
    Ok((w, h, r, g, b, a))
}

pub fn apply_packed(base: &RgbaImage, bytes: &[u8]) -> Result<RgbaImage, String> {
    if bytes.len() >= 8 && &bytes[0..4] == MAGIC && u32::from_le_bytes(bytes[4..8].try_into().unwrap()) == 3 {
        return apply_packed_striped(base, bytes);
    }
    let (w, h, r, g, b, a) = decode_packed(bytes)?;
    if w != base.width || h != base.height {
        return Err("size mismatch".into());
    }
    Ok(apply_planes(base, &r, &g, &b, &a))
}


// ---------------------------------------------------------------------------------------------
// v3 striped layout: header (28 B) | stripe table (nstripes x {rgb_len u32, a_len u32}) | stripes.
//   0  "NPR1"  4 u32 version=3  8 u8 codec  9 u8 flags  10 u16 q  12 u32 w  16 u32 h
//   20 u32 stripe_rows  24 u32 nstripes
// Each stripe is rows [i*stripe_rows, min(h, (i+1)*stripe_rows)); Paeth prediction restarts at the
// stripe's first row; rgb blob = pred(r)|pred(g)|pred(b), alpha blob = pred(a); each its own stream.
// Output is independent of thread count: stripes are collected in index order.
// ---------------------------------------------------------------------------------------------

const HIST_N: usize = 4 * 256 * 511;

#[inline]
fn quant1(d: i32, q: i32) -> i32 {
    if q <= 1 {
        d
    } else if d >= 0 {
        (d + q / 2) / q * q
    } else {
        (d - q / 2) / q * q
    }
}

/// Joint histogram of (channel, base value, residual) over the whole image.
fn residual_hist(hd: &RgbaImage, base: &RgbaImage) -> Vec<u32> {
    let row = hd.width as usize * 4;
    hd.pixels
        .par_chunks(row)
        .zip(base.pixels.par_chunks(row))
        .fold(
            || vec![0u32; HIST_N],
            |mut h, (hr, br)| {
                for (i, (&a, &b)) in hr.iter().zip(br.iter()).enumerate() {
                    let c = i & 3;
                    let d = (a as i32 - b as i32 + 255) as usize;
                    h[(c * 256 + b as usize) * 511 + d] += 1;
                }
                h
            },
        )
        .reduce(
            || vec![0u32; HIST_N],
            |mut a, b| {
                for (x, y) in a.iter_mut().zip(b.iter()) {
                    *x += *y;
                }
                a
            },
        )
}

/// Exact sum of squared error between hd and clamp(base + quant(hd-base, q)).
fn hist_sse(h: &[u32], q: i32) -> u64 {
    let tbl: Vec<i32> = (-255..=255).map(|d| quant1(d, q)).collect();
    let mut sse = 0u64;
    for c in 0..4 {
        for b in 0..256usize {
            let off = (c * 256 + b) * 511;
            for di in 0..511usize {
                let n = h[off + di];
                if n == 0 {
                    continue;
                }
                let d = di as i32 - 255;
                let rec = (b as i32 + tbl[di]).clamp(0, 255);
                let e = (b as i32 + d - rec) as i64;
                sse += n as u64 * (e * e) as u64;
            }
        }
    }
    sse
}

fn psnr_of_sse(sse: u64, len: usize) -> f64 {
    if sse == 0 {
        return f64::INFINITY;
    }
    let mse = sse as f64 / len as f64;
    10.0 * (255.0 * 255.0 / mse).log10()
}

fn encode_stripe(hd: &RgbaImage, base: &RgbaImage, y0: usize, y1: usize, q: i32, codec: u8) -> Result<(Vec<u8>, Vec<u8>), String> {
    let w = hd.width as usize;
    let rows = y1 - y0;
    let n = w * rows;
    let mut r = vec![0i16; n];
    let mut g = vec![0i16; n];
    let mut b = vec![0i16; n];
    let mut a = vec![0i16; n];
    let o0 = y0 * w * 4;
    for i in 0..n {
        let o = o0 + i * 4;
        r[i] = quant1(hd.pixels[o] as i32 - base.pixels[o] as i32, q) as i16;
        g[i] = quant1(hd.pixels[o + 1] as i32 - base.pixels[o + 1] as i32, q) as i16;
        b[i] = quant1(hd.pixels[o + 2] as i32 - base.pixels[o + 2] as i32, q) as i16;
        a[i] = quant1(hd.pixels[o + 3] as i32 - base.pixels[o + 3] as i32, q) as i16;
    }
    let mut rgb = pack_i16s(&predict_plane(&r, w as u32, rows as u32));
    rgb.extend(pack_i16s(&predict_plane(&g, w as u32, rows as u32)));
    rgb.extend(pack_i16s(&predict_plane(&b, w as u32, rows as u32)));
    let al = pack_i16s(&predict_plane(&a, w as u32, rows as u32));
    Ok((compress(codec, &rgb)?, compress(codec, &al)?))
}

fn encode_packed_striped(hd: &RgbaImage, base: &RgbaImage, opts: PackOpts) -> Result<(Vec<u8>, i32, f64), String> {
    let t0 = std::time::Instant::now();
    let (w, h) = (hd.width as usize, hd.height as usize);
    let len = w * h * 4;
    let hist = residual_hist(hd, base);
    let q = if let Some(target) = opts.psnr_target {
        let mut best = 1i32;
        for q in 1..=32 {
            if psnr_of_sse(hist_sse(&hist, q), len) < target {
                break;
            }
            best = q;
        }
        best
    } else {
        1
    };
    let p = psnr_of_sse(hist_sse(&hist, q), len);
    let t_q = t0.elapsed().as_secs_f32();
    let rows = STRIPE_ROWS as usize;
    let nst = h.div_ceil(rows);
    let parts: Vec<Result<(Vec<u8>, Vec<u8>), String>> = (0..nst)
        .into_par_iter()
        .map(|s| encode_stripe(hd, base, s * rows, ((s + 1) * rows).min(h), q, opts.codec))
        .collect();
    let mut stripes = Vec::with_capacity(nst);
    for pr in parts {
        stripes.push(pr?);
    }
    let body: usize = stripes.iter().map(|(a, b)| a.len() + b.len()).sum();
    let mut out = Vec::with_capacity(28 + nst * 8 + body);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&3u32.to_le_bytes());
    out.push(opts.codec);
    let mut flags = FLAG_PAETH | FLAG_ALPHA_SEP | FLAG_STRIPED;
    if q > 1 {
        flags |= FLAG_QUANT;
    }
    out.push(flags);
    out.extend_from_slice(&(q as u16).to_le_bytes());
    out.extend_from_slice(&hd.width.to_le_bytes());
    out.extend_from_slice(&hd.height.to_le_bytes());
    out.extend_from_slice(&STRIPE_ROWS.to_le_bytes());
    out.extend_from_slice(&(nst as u32).to_le_bytes());
    for (a, b) in &stripes {
        out.extend_from_slice(&(a.len() as u32).to_le_bytes());
        out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    }
    for (a, b) in &stripes {
        out.extend_from_slice(a);
        out.extend_from_slice(b);
    }
    if std::env::var_os("NEWPAL_PATCH_TRACE").is_some() {
        eprintln!(
            "[patch]   striped {w}x{h}: hist+q {t_q:.1}s (q={q}, psnr {p:.2}), stripes {nst} done {:.1}s, {} bytes",
            t0.elapsed().as_secs_f32(),
            out.len()
        );
    }
    Ok((out, q, p))
}

struct StripedHdr {
    codec: u8,
    w: usize,
    h: usize,
    rows: usize,
    /// (rgb_off, rgb_len, a_off, a_len) per stripe, absolute offsets into the file
    tab: Vec<(usize, usize, usize, usize)>,
}

fn parse_striped(bytes: &[u8]) -> Result<StripedHdr, String> {
    if bytes.len() < 28 || &bytes[0..4] != MAGIC {
        return Err("bad magic".into());
    }
    let codec = bytes[8];
    let flags = bytes[9];
    if flags & FLAG_PAETH == 0 || flags & FLAG_ALPHA_SEP == 0 || flags & FLAG_STRIPED == 0 {
        return Err("unexpected flags".into());
    }
    let w = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let h = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
    let rows = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let nst = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
    if rows == 0 || nst != h.div_ceil(rows) {
        return Err("bad stripe table".into());
    }
    let tab_end = 28 + nst * 8;
    if bytes.len() < tab_end {
        return Err("truncated stripe table".into());
    }
    let mut tab = Vec::with_capacity(nst);
    let mut off = tab_end;
    for s in 0..nst {
        let rl = u32::from_le_bytes(bytes[28 + s * 8..32 + s * 8].try_into().unwrap()) as usize;
        let al = u32::from_le_bytes(bytes[32 + s * 8..36 + s * 8].try_into().unwrap()) as usize;
        tab.push((off, rl, off + rl, al));
        off += rl + al;
    }
    if off != bytes.len() {
        return Err("truncated residual".into());
    }
    Ok(StripedHdr { codec, w, h, rows, tab })
}

/// Decode stripe `s` into 4 planes (r,g,b,a), each w*rows i16.
fn decode_stripe(bytes: &[u8], hd: &StripedHdr, s: usize) -> Result<[Vec<i16>; 4], String> {
    let rows = hd.rows.min(hd.h - s * hd.rows);
    let n = hd.w * rows;
    let (ro, rl, ao, al) = hd.tab[s];
    let rgb = unpack_i16s(&decompress(hd.codec, &bytes[ro..ro + rl])?)?;
    let a = unpack_i16s(&decompress(hd.codec, &bytes[ao..ao + al])?)?;
    if rgb.len() != n * 3 || a.len() != n {
        return Err("stripe plane size".into());
    }
    let (w32, r32) = (hd.w as u32, rows as u32);
    Ok([
        unpredict_plane(&rgb[0..n], w32, r32),
        unpredict_plane(&rgb[n..2 * n], w32, r32),
        unpredict_plane(&rgb[2 * n..3 * n], w32, r32),
        unpredict_plane(&a, w32, r32),
    ])
}

fn decode_packed_striped(bytes: &[u8]) -> Result<(u32, u32, Vec<i16>, Vec<i16>, Vec<i16>, Vec<i16>), String> {
    let hd = parse_striped(bytes)?;
    let parts: Vec<Result<[Vec<i16>; 4], String>> = (0..hd.tab.len()).into_par_iter().map(|s| decode_stripe(bytes, &hd, s)).collect();
    let n = hd.w * hd.h;
    let (mut r, mut g, mut b, mut a) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
    for p in parts {
        let [pr, pg, pb, pa] = p?;
        r.extend(pr);
        g.extend(pg);
        b.extend(pb);
        a.extend(pa);
    }
    Ok((hd.w as u32, hd.h as u32, r, g, b, a))
}

fn apply_packed_striped(base: &RgbaImage, bytes: &[u8]) -> Result<RgbaImage, String> {
    let hd = parse_striped(bytes)?;
    if hd.w as u32 != base.width || hd.h as u32 != base.height {
        return Err("size mismatch".into());
    }
    let mut out = RgbaImage::new(base.width, base.height);
    let row_bytes = hd.w * 4 * hd.rows;
    out.pixels
        .par_chunks_mut(row_bytes)
        .zip(base.pixels.par_chunks(row_bytes))
        .enumerate()
        .try_for_each(|(s, (o, bpx))| -> Result<(), String> {
            let [r, g, b, a] = decode_stripe(bytes, &hd, s)?;
            for i in 0..r.len() {
                let j = i * 4;
                o[j] = (bpx[j] as i16 + r[i]).clamp(0, 255) as u8;
                o[j + 1] = (bpx[j + 1] as i16 + g[i]).clamp(0, 255) as u8;
                o[j + 2] = (bpx[j + 2] as i16 + b[i]).clamp(0, 255) as u8;
                o[j + 3] = (bpx[j + 3] as i16 + a[i]).clamp(0, 255) as u8;
            }
            Ok(())
        })?;
    Ok(out)
}

#[cfg(test)]
mod striped_tests {
    use super::*;

    fn img(w: u32, h: u32, seed: u32) -> RgbaImage {
        let mut i = RgbaImage::new(w, h);
        let mut s = seed;
        for p in i.pixels.iter_mut() {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            *p = (s >> 24) as u8;
        }
        i
    }

    #[test]
    fn striped_roundtrip_matches_v2_reconstruction() {
        // 2100 x 2100 > STRIPE_MIN_PIXELS, odd stripe tail (2100 % 128 != 0)
        let base = img(2100, 2100, 7);
        let mut hd = base.clone();
        let mut s = 99u32;
        for p in hd.pixels.iter_mut() {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            let d = ((s >> 16) % 9) as i32 - 4;
            *p = (*p as i32 + d).clamp(0, 255) as u8;
        }
        let opts = PackOpts::brotli_psnr(45.0);
        let (v3, q3, p3) = encode_packed_striped(&hd, &base, opts).unwrap();
        // v2 path on identical data (call the legacy body by temporarily forcing it via a small helper)
        let (v2, q2, p2) = encode_packed_v2(&hd, &base, opts).unwrap();
        assert_eq!(q3, q2);
        assert!((p3 - p2).abs() < 1e-9 || (p3.is_infinite() && p2.is_infinite()), "psnr {p3} vs {p2}");
        let r3 = apply_packed(&base, &v3).unwrap();
        let r2 = apply_packed(&base, &v2).unwrap();
        assert_eq!(r3.pixels, r2.pixels, "v3 reconstruction must equal v2");
        let (_, _, a, b, c, d) = decode_packed(&v3).unwrap();
        assert_eq!(a.len(), 2100 * 2100);
        let _ = (b, c, d);
    }
}
