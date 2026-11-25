//! Vector text: glyph *outlines* are filled by vello at the final pixel size (no atlas, no bitmap font), so text
//! stays sharp at 720p, 4K and any DPI scale, and can be animated / scaled freely.
//!
//! Font stack, per character (first face that has the glyph wins):
//! 1. bundled UI faces: a subset of LXGW WenKai (SIL OFL 1.1, renamed "NewPal Kai" as the OFL requires for
//!    modified versions) - see `fonts/README.md`;
//! 2. extra faces from the host (e.g. a language pack font, the player's own `Resource/CT.ttf`);
//! 3. system fonts (Microsoft YaHei / SimSun / Segoe UI ...), used only at runtime, never shipped.
use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, MetadataProvider};
use std::sync::Arc;
use vello::kurbo::Affine;
use vello::peniko::{Blob, Brush, Color, Fill, FontData};
use vello::{Glyph, Scene};

pub const BUNDLED_REGULAR: &[u8] = include_bytes!("../fonts/NewPalKai-Regular.ttf");
pub const BUNDLED_MEDIUM: &[u8] = include_bytes!("../fonts/NewPalKai-Medium.ttf");

#[derive(Clone)]
pub struct Face {
    pub data: FontData,
    pub name: String,
}

impl Face {
    pub fn from_bytes(bytes: Vec<u8>, index: u32, name: &str) -> Option<Self> {
        FontRef::from_index(&bytes, index).ok()?;
        Some(Self { data: FontData::new(Blob::new(Arc::new(bytes)), index), name: name.to_string() })
    }
    pub fn from_static(bytes: &'static [u8], name: &str) -> Option<Self> {
        FontRef::from_index(bytes, 0).ok()?;
        Some(Self { data: FontData::new(Blob::new(Arc::new(bytes)), 0), name: name.to_string() })
    }
    fn font(&self) -> FontRef<'_> {
        FontRef::from_index(self.data.data.as_ref(), self.data.index).expect("validated at load")
    }
}

/// One fallback chain.
#[derive(Clone, Default)]
pub struct FontSet {
    pub faces: Vec<Face>,
}

#[derive(Clone)]
pub struct Fonts {
    pub body: FontSet,
    pub title: FontSet,
    /// in-game text (dialogue, original-style menus): the host faces first (the player's `CT.ttf`, language-pack
    /// fonts), then the body chain
    pub dialog: FontSet,
}

/// Windows system fonts tried after the bundled / host faces (runtime only).
const SYSTEM: &[(&str, u32)] = &[
    (r"C:\Windows\Fonts\msyh.ttc", 0),
    (r"C:\Windows\Fonts\msjh.ttc", 0),
    (r"C:\Windows\Fonts\simsun.ttc", 0),
    (r"C:\Windows\Fonts\segoeui.ttf", 0),
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
];

impl Fonts {
    /// Bundled faces + `extra` (host-provided: language pack fonts, the player's CT.ttf) + system fallbacks.
    pub fn load(extra: Vec<Face>) -> Self {
        let mut sys = Vec::new();
        for (p, i) in SYSTEM {
            if let Ok(b) = std::fs::read(p) {
                if let Some(f) = Face::from_bytes(b, *i, p) {
                    sys.push(f);
                }
            }
        }
        let reg = Face::from_static(BUNDLED_REGULAR, "NewPal Kai Regular");
        let med = Face::from_static(BUNDLED_MEDIUM, "NewPal Kai Medium");
        let mut body = FontSet::default();
        body.faces.extend(reg.clone());
        body.faces.extend(extra.iter().cloned());
        body.faces.extend(sys.iter().cloned());
        let mut title = FontSet::default();
        title.faces.extend(med);
        title.faces.extend(reg.clone());
        title.faces.extend(extra.iter().cloned());
        title.faces.extend(sys.iter().cloned());
        let mut dialog = FontSet::default();
        dialog.faces.extend(extra);
        dialog.faces.extend(reg);
        dialog.faces.extend(sys);
        Self { body, title, dialog }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy)]
pub struct Style {
    pub size: f32,
    pub color: Color,
    pub align: Align,
    /// extra advance after every character, in px
    pub spacing: f32,
    pub title: bool,
    /// use [`Fonts::dialog`]
    pub dialog: bool,
}

impl Style {
    pub fn new(size: f32, color: Color) -> Self {
        Self { size, color, align: Align::Left, spacing: 0.0, title: false, dialog: false }
    }
    pub fn align(mut self, a: Align) -> Self {
        self.align = a;
        self
    }
    pub fn spacing(mut self, s: f32) -> Self {
        self.spacing = s;
        self
    }
    pub fn title(mut self) -> Self {
        self.title = true;
        self
    }
    /// use the in-game font chain ([`Fonts::dialog`], the player's CT.ttf first)
    pub fn dialog_font(mut self) -> Self {
        self.dialog = true;
        self
    }
    pub fn alpha(mut self, a: f32) -> Self {
        self.color = self.color.multiply_alpha(a.clamp(0.0, 1.0));
        self
    }
}

struct Run {
    face: usize,
    glyphs: Vec<Glyph>,
}

/// Shaped line: runs per face, total advance, ascent/descent of the primary face.
pub struct Line {
    runs: Vec<Run>,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

fn set<'a>(fonts: &'a Fonts, st: &Style) -> &'a FontSet {
    if st.dialog {
        &fonts.dialog
    } else if st.title {
        &fonts.title
    } else {
        &fonts.body
    }
}

/// One positioned character (for per-glyph effects such as the typewriter).
#[derive(Clone, Copy, Debug)]
pub struct PlacedChar {
    pub ch: char,
    pub face: usize,
    pub gid: u32,
    /// pen position and advance at `size` px
    pub x: f32,
    pub adv: f32,
}

/// Per-character layout with the font fallback of `st` (advance + `st.spacing` after each character).
pub fn place_chars(fonts: &Fonts, st: &Style, text: &str) -> Vec<PlacedChar> {
    let fs = set(fonts, st);
    let size = Size::new(st.size);
    let refs: Vec<FontRef<'_>> = fs.faces.iter().map(|f| f.font()).collect();
    let mut out = Vec::new();
    let mut x = 0.0f32;
    for ch in text.chars() {
        if ch == '\n' {
            continue;
        }
        let hit = refs.iter().enumerate().find_map(|(i, f)| f.charmap().map(ch).filter(|g| g.to_u32() != 0).map(|g| (i, g)));
        let Some((fi, gid)) = hit.or_else(|| refs.first().map(|f| (0, f.charmap().map('?').unwrap_or_default()))) else { continue };
        let adv = if ch == ' ' || ch == '\u{3000}' {
            refs[fi].glyph_metrics(size, LocationRef::default()).advance_width(gid).unwrap_or(st.size * if ch == ' ' { 0.3 } else { 1.0 })
        } else {
            refs[fi].glyph_metrics(size, LocationRef::default()).advance_width(gid).unwrap_or(st.size * 0.5)
        };
        out.push(PlacedChar { ch, face: fi, gid: gid.to_u32(), x, adv });
        x += adv + st.spacing;
    }
    out
}

/// Ascent / descent of the primary face of `st` at `st.size`.
pub fn metrics(fonts: &Fonts, st: &Style) -> (f32, f32) {
    let fs = set(fonts, st);
    fs.faces.first().map(|f| {
        let m = f.font().metrics(Size::new(st.size), LocationRef::default());
        (m.ascent, m.descent)
    }).unwrap_or((st.size * 0.8, -st.size * 0.2))
}

/// Fill (or stroke, for outlines) placed characters of one face set at `size`, translated by `xf`.
pub fn paint_chars(sc: &mut Scene, fonts: &Fonts, st: &Style, chars: &[(PlacedChar, f32)], brush: &Brush, xf: Affine, stroke: Option<&vello::kurbo::Stroke>) {
    let fs = set(fonts, st);
    let mut i = 0;
    while i < chars.len() {
        let face = chars[i].0.face;
        let mut j = i;
        while j < chars.len() && chars[j].0.face == face {
            j += 1;
        }
        let gl = chars[i..j].iter().map(|(c, y)| Glyph { id: c.gid, x: c.x, y: *y });
        let b = sc.draw_glyphs(&fs.faces[face].data).font_size(st.size).transform(xf).hint(false).brush(brush);
        match stroke {
            Some(s) => b.draw(s, gl),
            None => b.draw(Fill::NonZero, gl),
        }
        i = j;
    }
}

/// Simple left-to-right layout (CJK + Latin UI strings need no complex shaping).
pub fn shape(fonts: &Fonts, st: &Style, text: &str) -> Line {
    let fs = set(fonts, st);
    let size = Size::new(st.size);
    let refs: Vec<FontRef<'_>> = fs.faces.iter().map(|f| f.font()).collect();
    let (ascent, descent) = refs.first().map(|f| {
        let m = f.metrics(size, LocationRef::default());
        (m.ascent, m.descent)
    }).unwrap_or((st.size * 0.8, -st.size * 0.2));
    let mut runs: Vec<Run> = Vec::new();
    let mut x = 0.0f32;
    for ch in text.chars() {
        if ch == '\n' {
            continue;
        }
        let mut hit = None;
        for (i, f) in refs.iter().enumerate() {
            if let Some(g) = f.charmap().map(ch) {
                if g.to_u32() != 0 {
                    hit = Some((i, g));
                    break;
                }
            }
        }
        let Some((fi, gid)) = hit.or_else(|| refs.first().map(|f| (0, f.charmap().map('?').unwrap_or_default()))) else { continue };
        let adv = refs[fi].glyph_metrics(size, LocationRef::default()).advance_width(gid).unwrap_or(st.size * 0.5);
        match runs.last_mut() {
            Some(r) if r.face == fi => r.glyphs.push(Glyph { id: gid.to_u32(), x, y: 0.0 }),
            _ => runs.push(Run { face: fi, glyphs: vec![Glyph { id: gid.to_u32(), x, y: 0.0 }] }),
        }
        x += adv + st.spacing;
    }
    if !text.is_empty() {
        x -= st.spacing;
    }
    Line { runs, width: x.max(0.0), ascent, descent }
}

pub fn measure(fonts: &Fonts, st: &Style, text: &str) -> f32 {
    shape(fonts, st, text).width
}

/// Draw `text` with its baseline at `y`; `x` is interpreted per `st.align`. Returns the drawn width.
pub fn draw(sc: &mut Scene, fonts: &Fonts, st: &Style, text: &str, x: f32, y: f32) -> f32 {
    draw_with(sc, fonts, st, text, x, y, &Brush::Solid(st.color), Affine::IDENTITY)
}

/// Draw vertically centred on `cy`.
pub fn draw_mid(sc: &mut Scene, fonts: &Fonts, st: &Style, text: &str, x: f32, cy: f32) -> f32 {
    let line = shape(fonts, st, text);
    let base = cy + (line.ascent + line.descent) / 2.0;
    paint(sc, fonts, st, &line, x, base, &Brush::Solid(st.color), Affine::IDENTITY);
    line.width
}

/// Draw with any brush (gradients) and an extra transform applied around the baseline origin.
pub fn draw_with(sc: &mut Scene, fonts: &Fonts, st: &Style, text: &str, x: f32, y: f32, brush: &Brush, xf: Affine) -> f32 {
    let line = shape(fonts, st, text);
    paint(sc, fonts, st, &line, x, y, brush, xf);
    line.width
}

pub fn paint(sc: &mut Scene, fonts: &Fonts, st: &Style, line: &Line, x: f32, y: f32, brush: &Brush, xf: Affine) {
    let fs = set(fonts, st);
    let x0 = match st.align {
        Align::Left => x,
        Align::Center => x - line.width / 2.0,
        Align::Right => x - line.width,
    };
    let t = xf * Affine::translate((x0 as f64, y as f64));
    for r in &line.runs {
        sc.draw_glyphs(&fs.faces[r.face].data)
            .font_size(st.size)
            .transform(t)
            .hint(false)
            .brush(brush)
            .draw(Fill::NonZero, r.glyphs.iter().copied());
    }
}

/// Greedy wrap to `max_w` (CJK: any character; Latin: at spaces). Returns lines.
pub fn wrap(fonts: &Fonts, st: &Style, text: &str, max_w: f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        let mut word = String::new();
        let flush_word = |cur: &mut String, word: &mut String, out: &mut Vec<String>| {
            let cand = format!("{cur}{word}");
            if measure(fonts, st, cand.trim_end()) <= max_w || cur.is_empty() {
                *cur = cand;
            } else {
                out.push(cur.trim_end().to_string());
                *cur = word.trim_start().to_string();
            }
            word.clear();
        };
        for ch in para.chars() {
            let cjk = (ch as u32) >= 0x2E80;
            if cjk {
                flush_word(&mut cur, &mut word, &mut out);
                word.push(ch);
                // no line may start with closing punctuation
                flush_word(&mut cur, &mut word, &mut out);
            } else if ch == ' ' {
                word.push(ch);
                flush_word(&mut cur, &mut word, &mut out);
            } else {
                word.push(ch);
            }
        }
        flush_word(&mut cur, &mut word, &mut out);
        out.push(cur.trim_end().to_string());
    }
    out
}
