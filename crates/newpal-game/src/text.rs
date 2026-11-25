//! Dialogue text rasterization with fontdue (crisp at any resolution: rendered in physical pixels).
use newpal_assets::image::RgbaImage;
use newpal_assets::i18n::wrap::{layout, WrapMode};
use newpal_assets::talk::Span;

/// Dialogue renderer over a font *stack*: per character the first font with a glyph is used (language pack
/// font, game CT.ttf, system fonts ... see `newpal_assets::i18n::fonts`). Wrapping follows the locale.
pub struct TextRenderer {
    fonts: Vec<fontdue::Font>,
    wrap: WrapMode,
}

/// `%C<n>` colour codes. Guesses until the palette is located in the exe: 6 = names, 5 = body text.
pub fn palette(c: u8) -> [u8; 3] {
    match c {
        1 => [255, 96, 96],
        2 => [96, 255, 96],
        3 => [120, 160, 255],
        4 => [255, 160, 64],
        6 => [255, 220, 120],
        7 => [200, 200, 200],
        _ => [255, 255, 255],
    }
}

impl TextRenderer {
    #[allow(dead_code)]
    pub fn new(bytes: &[u8]) -> Result<Self, String> {
        Self::from_stack(&[bytes], WrapMode::Char)
    }

    /// `stack`: font file bytes in priority order (unparsable ones are skipped).
    pub fn from_stack(stack: &[&[u8]], wrap: WrapMode) -> Result<Self, String> {
        let mut fonts = Vec::new();
        let mut err = String::from("no fonts");
        for b in stack {
            match fontdue::Font::from_bytes(*b, fontdue::FontSettings { collection_index: 0, scale: 48.0, ..Default::default() }) {
                Ok(f) => fonts.push(f),
                Err(e) => err = e.to_string(),
            }
        }
        if fonts.is_empty() {
            return Err(err);
        }
        Ok(Self { fonts, wrap })
    }

    /// Font index that has a glyph for `ch` (0 when none has).
    fn pick(&self, ch: char) -> usize {
        self.fonts.iter().position(|f| f.lookup_glyph_index(ch) != 0).unwrap_or(0)
    }

    /// Render styled lines, wrapping at `max_w` pixels. Returns an RGBA image (transparent background).
    pub fn render(&self, lines: &[Vec<Span>], px: f32, max_w: u32) -> RgbaImage {
        let font = &self.fonts[0];
        let lm = font.horizontal_line_metrics(px);
        let (ascent, line_h) = match lm {
            Some(m) => (m.ascent, (m.ascent - m.descent + m.line_gap).ceil()),
            None => (px * 0.8, px * 1.2),
        };
        let (placed, line) = layout(lines, max_w as f32, self.wrap, &mut |ch| self.fonts[self.pick(ch)].metrics(ch, px).advance_width);
        let glyphs: Vec<(char, [u8; 3], f32, usize)> = placed.into_iter().map(|g| (g.ch, palette(g.color), g.x, g.line)).collect();
        let h = (line.max(1) as f32 * line_h).ceil() as u32 + 4;
        let mut img = RgbaImage::new(max_w.max(1), h);
        for (ch, col, x, ln) in glyphs {
            let (m, bmp) = self.fonts[self.pick(ch)].rasterize(ch, px);
            let baseline = ln as f32 * line_h + ascent;
            let gx = (x + m.xmin as f32).round() as i32;
            let gy = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
            for yy in 0..m.height {
                for xx in 0..m.width {
                    let a = bmp[yy * m.width + xx];
                    if a == 0 {
                        continue;
                    }
                    let (tx, ty) = (gx + xx as i32, gy + yy as i32);
                    if tx < 0 || ty < 0 || tx >= img.width as i32 || ty >= img.height as i32 {
                        continue;
                    }
                    let o = ((ty as u32 * img.width + tx as u32) * 4) as usize;
                    if a >= img.pixels[o + 3] {
                        img.pixels[o..o + 4].copy_from_slice(&[col[0], col[1], col[2], a]);
                    }
                }
            }
        }
        img
    }
}