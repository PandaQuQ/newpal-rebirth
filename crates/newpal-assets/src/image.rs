//! Simple RGBA8 image type + PNG load/save (for mod overrides and debug dumps).
use crate::{Error, Result};
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, `width * height * 4` bytes. Transparent pixels are all-zero.
    pub pixels: Vec<u8>,
}

/// Expand a 5-bit channel to 8 bits exactly like the original engine tools do.
#[inline]
pub fn c5(v: u16) -> u8 {
    ((v << 3) | (v >> 2)) as u8
}

/// RGB555 (`0RRRRRGGGGGBBBBB`) to RGBA8 (opaque).
#[inline]
pub fn rgb555(v: u16) -> [u8; 4] {
    [c5((v >> 10) & 31), c5((v >> 5) & 31), c5(v & 31), 255]
}

impl RgbaImage {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, pixels: vec![0; (width * height * 4) as usize] }
    }

    #[inline]
    pub fn put(&mut self, x: u32, y: u32, px: [u8; 4]) {
        if x < self.width && y < self.height {
            let o = ((y * self.width + x) * 4) as usize;
            self.pixels[o..o + 4].copy_from_slice(&px);
        }
    }

    /// Alpha-blit `src` at (dx, dy) with 1-bit alpha (copies non-transparent pixels).
    pub fn blit(&mut self, src: &RgbaImage, dx: i32, dy: i32) {
        for y in 0..src.height as i32 {
            let ty = dy + y;
            if ty < 0 || ty >= self.height as i32 {
                continue;
            }
            for x in 0..src.width as i32 {
                let tx = dx + x;
                if tx < 0 || tx >= self.width as i32 {
                    continue;
                }
                let s = ((y as u32 * src.width + x as u32) * 4) as usize;
                if src.pixels[s + 3] == 0 {
                    continue;
                }
                let o = ((ty as u32 * self.width + tx as u32) * 4) as usize;
                self.pixels[o..o + 4].copy_from_slice(&src.pixels[s..s + 4]);
            }
        }
    }

    /// Decode a PNG (any color type / bit depth) to RGBA8.
    pub fn from_png(bytes: &[u8]) -> Result<Self> {
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = dec.read_info().map_err(|e| Error::corrupt(format!("png: {e}")))?;
        let size = reader.output_buffer_size().ok_or_else(|| Error::corrupt("png: image too large"))?;
        let mut buf = vec![0u8; size];
        let info = reader.next_frame(&mut buf).map_err(|e| Error::corrupt(format!("png: {e}")))?;
        let (w, h) = (info.width, info.height);
        let n = (w * h) as usize;
        let line = info.line_size;
        let mut out = Vec::with_capacity(n * 4);
        // The decompressed buffer is row-padded (`line_size` may exceed width*bpp).
        // Copying the packed prefix (`n * bpp`) would skew later rows (vertical-stripe garbage).
        for y in 0..h as usize {
            let row = &buf[y * line..];
            match info.color_type {
                png::ColorType::Rgba => out.extend_from_slice(&row[..w as usize * 4]),
                png::ColorType::Rgb => {
                    for p in row[..w as usize * 3].chunks_exact(3) {
                        out.extend_from_slice(&[p[0], p[1], p[2], 255]);
                    }
                }
                png::ColorType::GrayscaleAlpha => {
                    for p in row[..w as usize * 2].chunks_exact(2) {
                        out.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
                    }
                }
                png::ColorType::Grayscale => {
                    for &g in &row[..w as usize] {
                        out.extend_from_slice(&[g, g, g, 255]);
                    }
                }
                png::ColorType::Indexed => return Err(Error::corrupt("png: indexed not expanded")),
            }
        }
        Ok(Self { width: w, height: h, pixels: out })
    }

    pub fn to_png(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width, self.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().map_err(|e| Error::corrupt(format!("png: {e}")))?;
            w.write_image_data(&self.pixels).map_err(|e| Error::corrupt(format!("png: {e}")))?;
        }
        Ok(out)
    }

    pub fn save_png(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_png()?).map_err(|e| Error::io(e, path.display().to_string()))
    }

    /// 8-bit RGB PNG (no alpha). rife-ncnn-vulkan mis-reads 32-bit RGBA as tightly packed RGB,
    /// which produces coloured vertical-stripe garbage. Feed it this instead.
    pub fn to_rgb_png(&self) -> Result<Vec<u8>> {
        let mut rgb = Vec::with_capacity((self.width * self.height * 3) as usize);
        for p in self.pixels.chunks_exact(4) {
            rgb.extend_from_slice(&p[..3]);
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width, self.height);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().map_err(|e| Error::corrupt(format!("png: {e}")))?;
            w.write_image_data(&rgb).map_err(|e| Error::corrupt(format!("png: {e}")))?;
        }
        Ok(out)
    }

    pub fn save_rgb_png(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_rgb_png()?).map_err(|e| Error::io(e, path.display().to_string()))
    }

    pub fn scale_nearest(&self, factor: u32) -> Self {
        let f = factor.max(1);
        let mut o = Self::new(self.width * f, self.height * f);
        for y in 0..o.height {
            for x in 0..o.width {
                let s = (((y / f) * self.width + (x / f)) * 4) as usize;
                let d = ((y * o.width + x) * 4) as usize;
                o.pixels[d..d + 4].copy_from_slice(&self.pixels[s..s + 4]);
            }
        }
        o
    }

    /// Looping APNG. `delay_num/delay_den` is the per-frame duration in seconds (den=0 means 100).
    pub fn save_apng(path: &Path, frames: &[RgbaImage], delay_num: u16, delay_den: u16) -> Result<()> {
        if frames.is_empty() {
            return Err(Error::corrupt("apng: no frames"));
        }
        let (w, h) = (frames[0].width, frames[0].height);
        if frames.iter().any(|f| f.width != w || f.height != h) {
            return Err(Error::corrupt("apng: frame size mismatch"));
        }
        let mut file = std::fs::File::create(path).map_err(|e| Error::io(e, path.display().to_string()))?;
        {
            let mut enc = png::Encoder::new(&mut file, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_animated(frames.len() as u32, 0).map_err(|e| Error::corrupt(format!("apng: {e}")))?;
            enc.set_frame_delay(delay_num, delay_den).map_err(|e| Error::corrupt(format!("apng: {e}")))?;
            let _ = enc.set_dispose_op(png::DisposeOp::Background);
            let mut wr = enc.write_header().map_err(|e| Error::corrupt(format!("apng: {e}")))?;
            for (i, f) in frames.iter().enumerate() {
                if i > 0 {
                    wr.set_frame_delay(delay_num, delay_den).map_err(|e| Error::corrupt(format!("apng: {e}")))?;
                    let _ = wr.set_dispose_op(png::DisposeOp::Background);
                }
                wr.write_image_data(&f.pixels).map_err(|e| Error::corrupt(format!("apng: {e}")))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_nearest_quadruples_a_pixel() {
        let mut i = RgbaImage::new(1, 1);
        i.put(0, 0, [10, 20, 30, 255]);
        let o = i.scale_nearest(4);
        assert_eq!((o.width, o.height), (4, 4));
        assert_eq!(&o.pixels[0..4], &[10, 20, 30, 255]);
        assert_eq!(&o.pixels[(3 * 4 + 3) * 4..(3 * 4 + 3) * 4 + 4], &[10, 20, 30, 255]);
    }

    #[test]
    fn rgb_png_roundtrip_odd_width() {
        let mut i = RgbaImage::new(241, 3);
        i.put(0, 0, [10, 20, 30, 255]);
        i.put(240, 2, [1, 2, 3, 255]);
        i.put(100, 1, [9, 8, 7, 128]);
        let img = RgbaImage::from_png(&i.to_rgb_png().unwrap()).unwrap();
        assert_eq!((img.width, img.height), (241, 3));
        assert_eq!(&img.pixels[0..4], &[10, 20, 30, 255]);
        let o = ((2 * 241 + 240) * 4) as usize;
        assert_eq!(&img.pixels[o..o + 4], &[1, 2, 3, 255]);
        let o = ((1 * 241 + 100) * 4) as usize;
        assert_eq!(&img.pixels[o..o + 3], &[9, 8, 7]);
        assert_eq!(img.pixels[o + 3], 255); // rgb png has no alpha
    }
}
