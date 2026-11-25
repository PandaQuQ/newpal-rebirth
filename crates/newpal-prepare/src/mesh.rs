//! Motion mesh for a looping sprite animation.
//!
//! A regular grid is laid over the sprite. Each vertex is tracked to the next frame with opaque-only
//! SAD block-match plus sub-pixel Lucas-Kanade on luma; verts/flow stay in logical pixels. Warp error = mean absolute
//! difference (0..1) between frame B and frame A inverse-warped by the mesh, on opaque pixels.
//! Segments above [`ERROR_FADE`] fall back to a cross-fade at runtime.
use newpal_assets::image::RgbaImage;
use serde::{Deserialize, Serialize};

pub const ERROR_FADE: f32 = 0.18;
const WIN: i32 = 5;
const LK_ITERS: i32 = 8;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnimMesh {
    pub scale: u32,
    pub logical_w: u32,
    pub logical_h: u32,
    pub grid: [u32; 2],
    pub durations_ms: Vec<f32>,
    /// triangle indices into the grid (2 tris per cell), shared by every frame
    pub indices: Vec<u32>,
    pub frames: Vec<FrameMesh>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameMesh {
    /// grid vertices in *logical* pixels (origin = top-left of the sprite)
    pub verts: Vec<[f32; 2]>,
    /// motion of each vertex toward the next frame, in logical pixels
    pub flow: Vec<[f32; 2]>,
    pub error: f32,
    pub fallback: bool,
}

impl AnimMesh {
    pub fn cells(&self) -> (u32, u32) {
        (self.grid[0].saturating_sub(1), self.grid[1].saturating_sub(1))
    }

    /// Vertex positions at blend `t` in 0..1 (forward warp of this frame).
    pub fn verts_at(frame: &FrameMesh, t: f32) -> Vec<[f32; 2]> {
        frame.verts.iter().zip(&frame.flow).map(|(v, f)| [v[0] + f[0] * t, v[1] + f[1] * t]).collect()
    }
}

pub fn grid_size(w: u32, h: u32) -> [u32; 2] {
    let gx = ((w / 6).clamp(4, 16)) | 1;
    let gy = ((h / 6).clamp(6, 24)) | 1;
    [gx.max(4), gy.max(6)]
}

fn luma(img: &RgbaImage, x: i32, y: i32) -> f32 {
    let x = x.clamp(0, img.width as i32 - 1) as u32;
    let y = y.clamp(0, img.height as i32 - 1) as u32;
    let i = ((y * img.width + x) * 4) as usize;
    let a = img.pixels[i + 3] as f32 / 255.0;
    (0.299 * img.pixels[i] as f32 + 0.587 * img.pixels[i + 1] as f32 + 0.114 * img.pixels[i + 2] as f32) / 255.0 * a
}

fn alpha(img: &RgbaImage, x: i32, y: i32) -> f32 {
    let x = x.clamp(0, img.width as i32 - 1) as u32;
    let y = y.clamp(0, img.height as i32 - 1) as u32;
    img.pixels[((y * img.width + x) * 4 + 3) as usize] as f32 / 255.0
}

/// Translational Lucas-Kanade in a window, one pyramid level. Returns flow in this level's pixels.
fn lk_level(a: &RgbaImage, b: &RgbaImage, x: f32, y: f32, guess: [f32; 2]) -> [f32; 2] {
    let (mut gx, mut gy) = (guess[0], guess[1]);
    for _ in 0..LK_ITERS {
        let mut a11 = 0.0f32;
        let mut a12 = 0.0;
        let mut a22 = 0.0;
        let mut b1 = 0.0;
        let mut b2 = 0.0;
        let mut n = 0u32;
        for oy in -WIN..=WIN {
            for ox in -WIN..=WIN {
                let (px, py) = (x + ox as f32, y + oy as f32);
                let (qx, qy) = (px + gx, py + gy);
                if px < 1.0 || py < 1.0 || qx < 1.0 || qy < 1.0 { continue; }
                if px >= a.width as f32 - 2.0 || py >= a.height as f32 - 2.0 { continue; }
                if qx >= b.width as f32 - 2.0 || qy >= b.height as f32 - 2.0 { continue; }
                let ix = (luma(a, px as i32 + 1, py as i32) - luma(a, px as i32 - 1, py as i32)) * 0.5;
                let iy = (luma(a, px as i32, py as i32 + 1) - luma(a, px as i32, py as i32 - 1)) * 0.5;
                let it = luma(b, qx as i32, qy as i32) - luma(a, px as i32, py as i32);
                a11 += ix * ix;
                a12 += ix * iy;
                a22 += iy * iy;
                b1 -= it * ix;
                b2 -= it * iy;
                n += 1;
            }
        }
        if n < 8 { break; }
        let det = a11 * a22 - a12 * a12;
        if det.abs() < 1e-4 { break; }
        gx += (a22 * b1 - a12 * b2) / det;
        gy += (-a12 * b1 + a11 * b2) / det;
    }
    [gx, gy]
}

/// Block-match + sub-pixel LK. Only pixels opaque in A contribute to SAD, so empty
/// background cannot invent a displacement. Returns None when the window has too little signal.
pub fn track(a: &RgbaImage, b: &RgbaImage, x: f32, y: f32) -> Option<[f32; 2]> {
    const R: i32 = 24;
    let mut opaque = 0u32;
    for oy in -WIN..=WIN {
        for ox in -WIN..=WIN {
            if alpha(a, (x + ox as f32) as i32, (y + oy as f32) as i32) >= 0.05 { opaque += 1; }
        }
    }
    if opaque < 8 { return None; }
    let mut best = (0i32, 0i32, f32::MAX);
    for dy in -R..=R {
        for dx in -R..=R {
            let mut sad = 0.0f32;
            let mut n = 0u32;
            for oy in -WIN..=WIN {
                for ox in -WIN..=WIN {
                    let (ax, ay) = (x + ox as f32, y + oy as f32);
                    if ax < 0.0 || ay < 0.0 || ax >= a.width as f32 || ay >= a.height as f32 { continue; }
                    if alpha(a, ax as i32, ay as i32) < 0.05 { continue; }
                    let (bx, by) = (ax + dx as f32, ay + dy as f32);
                    if bx < 0.0 || by < 0.0 || bx >= b.width as f32 || by >= b.height as f32 { continue; }
                    sad += (luma(a, ax as i32, ay as i32) - luma(b, bx as i32, by as i32)).abs();
                    n += 1;
                }
            }
            if n < 8 { continue; }
            let s = sad / n as f32;
            if s < best.2 { best = (dx, dy, s); }
        }
    }
    if best.2 == f32::MAX { return None; }
    Some(lk_level(a, b, x, y, [best.0 as f32, best.1 as f32]))
}

fn fill_and_smooth(flow: &mut [[f32; 2]], mut valid: Vec<bool>, gx: u32, gy: u32) {
    for _ in 0..gx.max(gy) {
        let src = flow.to_vec();
        let src_v = valid.clone();
        let mut any = false;
        for y in 0..gy as i32 {
            for x in 0..gx as i32 {
                let i = (y as u32 * gx + x as u32) as usize;
                if src_v[i] { continue; }
                let mut acc = [0.0f32; 2];
                let mut n = 0.0f32;
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let nx = x + dx;
                    let ny = y + dy;
                    if nx >= 0 && ny >= 0 && nx < gx as i32 && ny < gy as i32 {
                        let j = (ny as u32 * gx + nx as u32) as usize;
                        if src_v[j] {
                            acc[0] += src[j][0];
                            acc[1] += src[j][1];
                            n += 1.0;
                        }
                    }
                }
                if n > 0.0 {
                    flow[i] = [acc[0] / n, acc[1] / n];
                    valid[i] = true;
                    any = true;
                }
            }
        }
        if !any { break; }
    }
    for _ in 0..2 {
        let src = flow.to_vec();
        for y in 0..gy {
            for x in 0..gx {
                let i = (y * gx + x) as usize;
                if !valid[i] { continue; }
                let mut acc = src[i];
                let mut n = 1.0f32;
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx >= 0 && ny >= 0 && nx < gx as i32 && ny < gy as i32 {
                        let j = (ny as u32 * gx + nx as u32) as usize;
                        if valid[j] {
                            acc[0] += src[j][0];
                            acc[1] += src[j][1];
                            n += 1.0;
                        }
                    }
                }
                flow[i] = [acc[0] / n, acc[1] / n];
            }
        }
    }
}

fn make_grid(w: f32, h: f32, gx: u32, gy: u32) -> (Vec<[f32; 2]>, Vec<u32>) {
    let mut verts = Vec::with_capacity((gx * gy) as usize);
    for iy in 0..gy {
        for ix in 0..gx {
            let u = ix as f32 / (gx - 1) as f32;
            let v = iy as f32 / (gy - 1) as f32;
            verts.push([u * (w - 1.0).max(0.0), v * (h - 1.0).max(0.0)]);
        }
    }
    let mut idx = Vec::new();
    for iy in 0..gy - 1 {
        for ix in 0..gx - 1 {
            let i = iy * gx + ix;
            idx.extend_from_slice(&[i, i + 1, i + gx, i + 1, i + gx + 1, i + gx]);
        }
    }
    (verts, idx)
}

/// Inverse-warp `src` onto the destination grid (identity at rest, `flow` at t=1) and compare with `dst`.
pub fn warp_error(src: &RgbaImage, dst: &RgbaImage, verts: &[[f32; 2]], flow: &[[f32; 2]], gx: u32, gy: u32) -> f32 {
    let (w, h) = (dst.width, dst.height);
    let mut acc = 0.0f32;
    let mut n = 0u32;
    // For each dest pixel, locate the identity grid cell and interpolate source from the inverse map:
    // dest p = v + flow * 1, so we search warped quads.
    for cy in 0..gy - 1 {
        for cx in 0..gx - 1 {
            let i = (cy * gx + cx) as usize;
            let corners = [i, i + 1, i + gx as usize + 1, i + gx as usize]; // TL TR BR BL
            let d: Vec<[f32; 2]> = corners.iter().map(|&k| [verts[k][0] + flow[k][0], verts[k][1] + flow[k][1]]).collect();
            let s: Vec<[f32; 2]> = corners.iter().map(|&k| verts[k]).collect();
            let minx = d.iter().map(|p| p[0]).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
            let miny = d.iter().map(|p| p[1]).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
            let maxx = d.iter().map(|p| p[0]).fold(0.0f32, f32::max).ceil().min(w as f32 - 1.0) as i32;
            let maxy = d.iter().map(|p| p[1]).fold(0.0f32, f32::max).ceil().min(h as f32 - 1.0) as i32;
            for y in miny..=maxy {
                for x in minx..=maxx {
                    if let Some((u, v)) = inv_bilinear([x as f32, y as f32], d[0], d[1], d[2], d[3]) {
                        let sx = s[0][0] * (1.0 - u) * (1.0 - v) + s[1][0] * u * (1.0 - v) + s[2][0] * u * v + s[3][0] * (1.0 - u) * v;
                        let sy = s[0][1] * (1.0 - u) * (1.0 - v) + s[1][1] * u * (1.0 - v) + s[2][1] * u * v + s[3][1] * (1.0 - u) * v;
                        let da = alpha(dst, x, y);
                        let sa = sample_a(src, sx, sy);
                        if da < 0.05 && sa < 0.05 { continue; }
                        let (sr, sg, sb) = sample_rgb(src, sx, sy);
                        let di = ((y as u32 * w + x as u32) * 4) as usize;
                        let (dr, dg, db) = (dst.pixels[di] as f32, dst.pixels[di + 1] as f32, dst.pixels[di + 2] as f32);
                        acc += ((sr - dr).abs() + (sg - dg).abs() + (sb - db).abs()) / (3.0 * 255.0);
                        n += 1;
                    }
                }
            }
        }
    }
    if n == 0 { 1.0 } else { acc / n as f32 }
}

fn sample_a(img: &RgbaImage, x: f32, y: f32) -> f32 { alpha(img, x.round() as i32, y.round() as i32) }
fn sample_rgb(img: &RgbaImage, x: f32, y: f32) -> (f32, f32, f32) {
    let x = x.round().clamp(0.0, img.width as f32 - 1.0) as u32;
    let y = y.round().clamp(0.0, img.height as f32 - 1.0) as u32;
    let i = ((y * img.width + x) * 4) as usize;
    (img.pixels[i] as f32, img.pixels[i + 1] as f32, img.pixels[i + 2] as f32)
}

/// Inverse bilinear: find (u,v) in 0..1 such that lerp of TL,TR,BR,BL equals p. None if outside.
fn inv_bilinear(p: [f32; 2], tl: [f32; 2], tr: [f32; 2], br: [f32; 2], bl: [f32; 2]) -> Option<(f32, f32)> {
    // Newton on f(u,v) = (1-u)(1-v) TL + u(1-v) TR + uv BR + (1-u)v BL - p
    let mut u = 0.5f32;
    let mut v = 0.5f32;
    for _ in 0..8 {
        let (iu, iv) = (1.0 - u, 1.0 - v);
        let x = iu * iv * tl[0] + u * iv * tr[0] + u * v * br[0] + iu * v * bl[0] - p[0];
        let y = iu * iv * tl[1] + u * iv * tr[1] + u * v * br[1] + iu * v * bl[1] - p[1];
        // df/du, df/dv
        let dxdu = -iv * tl[0] + iv * tr[0] + v * br[0] - v * bl[0];
        let dydu = -iv * tl[1] + iv * tr[1] + v * br[1] - v * bl[1];
        let dxdv = -iu * tl[0] - u * tr[0] + u * br[0] + iu * bl[0];
        let dydv = -iu * tl[1] - u * tr[1] + u * br[1] + iu * bl[1];
        let det = dxdu * dydv - dxdv * dydu;
        if det.abs() < 1e-8 { break; }
        u -= (x * dydv - y * dxdv) / det;
        v -= (-x * dydu + y * dxdu) / det;
    }
    if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) { Some((u, v)) } else { None }
}

/// Build a motion mesh for a loop of same-size frames (logical / original pixels).
pub fn build(frames: &[RgbaImage], durations_ms: &[f32]) -> Result<AnimMesh, String> {
    if frames.len() < 2 { return Err("need at least 2 frames".into()); }
    let (w, h) = (frames[0].width, frames[0].height);
    if frames.iter().any(|f| f.width != w || f.height != h) { return Err("frame size mismatch".into()); }
    let [gx, gy] = grid_size(w, h);
    let (verts, indices) = make_grid(w as f32, h as f32, gx, gy);
    let n = frames.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (a, b) = (&frames[i], &frames[(i + 1) % n]);
        let mut valid = Vec::with_capacity(verts.len());
        let mut flow = verts.iter().map(|&p| {
            match track(a, b, p[0], p[1]) {
                Some(f) => { valid.push(true); f }
                None => { valid.push(false); [0.0, 0.0] }
            }
        }).collect::<Vec<_>>();
        fill_and_smooth(&mut flow, valid, gx, gy);
        let error = warp_error(a, b, &verts, &flow, gx, gy);
        out.push(FrameMesh { verts: verts.clone(), flow, error, fallback: error > ERROR_FADE });
    }
    Ok(AnimMesh {
        scale: 1,
        logical_w: w,
        logical_h: h,
        grid: [gx, gy],
        durations_ms: durations_ms.to_vec(),
        indices,
        frames: out,
    })
}

/// Scale a mesh built on original frames up to HD coordinates (UVs stay 0..1 via logical size).
pub fn scale_to(mut m: AnimMesh, scale: u32) -> AnimMesh {
    m.scale = scale;
    // verts/flow stay in logical pixels so the game camera (original units) can use them as-is.
    m
}

/// CPU forward-warp of `src` by t * flow (for tests / APNG fallback). Holes stay transparent.
pub fn warp_forward(src: &RgbaImage, verts: &[[f32; 2]], flow: &[[f32; 2]], gx: u32, gy: u32, t: f32) -> RgbaImage {
    let mut o = RgbaImage::new(src.width, src.height);
    for cy in 0..gy - 1 {
        for cx in 0..gx - 1 {
            let i = (cy * gx + cx) as usize;
            let corners = [i, i + 1, i + gx as usize + 1, i + gx as usize];
            let s: Vec<[f32; 2]> = corners.iter().map(|&k| verts[k]).collect();
            let d: Vec<[f32; 2]> = corners.iter().map(|&k| [verts[k][0] + flow[k][0] * t, verts[k][1] + flow[k][1] * t]).collect();
            let minx = d.iter().map(|p| p[0]).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
            let miny = d.iter().map(|p| p[1]).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
            let maxx = d.iter().map(|p| p[0]).fold(0.0f32, f32::max).ceil().min(src.width as f32 - 1.0) as i32;
            let maxy = d.iter().map(|p| p[1]).fold(0.0f32, f32::max).ceil().min(src.height as f32 - 1.0) as i32;
            for y in miny..=maxy {
                for x in minx..=maxx {
                    if let Some((u, v)) = inv_bilinear([x as f32, y as f32], d[0], d[1], d[2], d[3]) {
                        let sx = s[0][0] * (1.0 - u) * (1.0 - v) + s[1][0] * u * (1.0 - v) + s[2][0] * u * v + s[3][0] * (1.0 - u) * v;
                        let sy = s[0][1] * (1.0 - u) * (1.0 - v) + s[1][1] * u * (1.0 - v) + s[2][1] * u * v + s[3][1] * (1.0 - u) * v;
                        let (sr, sg, sb) = sample_rgb(src, sx, sy);
                        let sa = (sample_a(src, sx, sy) * 255.0).round() as u8;
                        let oi = ((y as u32 * src.width + x as u32) * 4) as usize;
                        o.pixels[oi] = sr.round() as u8;
                        o.pixels[oi + 1] = sg.round() as u8;
                        o.pixels[oi + 2] = sb.round() as u8;
                        o.pixels[oi + 3] = sa;
                    }
                }
            }
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dot(w: u32, h: u32, x: u32, y: u32, s: u32) -> RgbaImage {
        let mut i = RgbaImage::new(w, h);
        for dy in 0..s {
            for dx in 0..s {
                i.put(x + dx, y + dy, [200, 40, 40, 255]);
            }
        }
        i
    }

    #[test]
    fn tracks_a_translating_square() {
        let a = dot(48, 32, 8, 8, 12);
        let b = dot(48, 32, 16, 10, 12); // +8, +2
        let m = build(&[a.clone(), b], &[100.0, 100.0]).unwrap();
        assert_eq!(m.frames.len(), 2);
        let mut acc = [0.0f32; 2];
        let mut n = 0.0;
        for (v, f) in m.frames[0].verts.iter().zip(&m.frames[0].flow) {
            let (x, y) = (v[0].round() as i32, v[1].round() as i32);
            if alpha(&a, x, y) > 0.5 {
                acc[0] += f[0];
                acc[1] += f[1];
                n += 1.0;
            }
        }
        assert!(n > 0.0, "no verts on the square");
        let (dx, dy) = (acc[0] / n, acc[1] / n);
        assert!((dx - 8.0).abs() < 2.5, "dx={dx} n={n}");
        assert!((dy - 2.0).abs() < 2.5, "dy={dy} n={n}");
        assert!(m.frames[0].error < 0.15, "error={}", m.frames[0].error);
    }

    #[test]
    fn fade_flag_on_teleport() {
        let a = dot(48, 48, 2, 2, 6);
        let b = dot(48, 48, 40, 40, 6); // jump beyond search radius
        let m = build(&[a, b], &[100.0, 100.0]).unwrap();
        assert!(m.frames[0].error > 0.05, "error={}", m.frames[0].error);
    }
}
