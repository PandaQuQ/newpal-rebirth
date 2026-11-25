//! Original-style dialogue box, drawn as vectors with live font text (no bitmap text, no baked PNG words).
//!
//! * Layout, frame, palette and typewriter timing come from a data-driven [`DialogueTheme`] in the original's
//!   reference resolution (640x480 for New PAL); it is scaled to the window by height and centred horizontally,
//!   like the game view (`view-height 480`, width follows the aspect).
//! * Markup cues (already parsed by the engine's i18n layer, see `newpal_assets::i18n::markup`) drive the
//!   renderer: `<speed=N/>`, `<instant/>`, `<page/>`, `<color=N>`, `<br/>` (= lines), `<speaker>`, `<glyph=N/>`.
//! * The typewriter runs on wall-clock time in the render loop; it never touches the 35 ms logic tick.
use crate::text::{self, Fonts, PlacedChar, Style};
use serde::Deserialize;
use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRect, Stroke};
use vello::peniko::{Brush, Color, Fill, Gradient};
use vello::Scene;

/// Closing punctuation that must not start a line (kinsoku): fullwidth comma, ideographic full stop / comma,
/// fullwidth ! ? ; :, closing corner brackets, fullwidth paren, double angle bracket, ASCII , . ! ? ; :
const NO_LINE_START: &str = "\u{ff0c}\u{3002}\u{3001}\u{ff01}\u{ff1f}\u{ff1b}\u{ff1a}\u{300d}\u{300f}\u{ff09}\u{300b},.!?;:";

#[derive(Clone, Debug, PartialEq)]
pub struct DlgSpan {
    pub color: u8,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DlgCue {
    Page,
    Instant,
    Speed(u8),
    Glyph(u8),
    /// an original control code not modelled yet (kept so it can be logged)
    Ctl(char, Option<u8>),
}

/// One dialogue line as the engine hands it over: speaker, styled hard lines, cues at (line, column).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DlgText {
    pub speaker: Option<String>,
    pub lines: Vec<Vec<DlgSpan>>,
    pub cues: Vec<(usize, usize, DlgCue)>,
}

fn rgb(c: [u8; 4]) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}

/// Everything that makes the box look like the original. Coordinates are reference pixels (`ref_h` tall).
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct DialogueTheme {
    /// frame material: "parchment" (cream scroll paper between wooden rods, like the original menus) or
    /// "modern" (dark glass panel with a gold rim)
    pub style: String,
    pub ref_w: f32,
    pub ref_h: f32,
    /// x, y, w, h of the box
    pub rect: [f32; 4],
    /// text inset: left, top, right, bottom
    pub pad: [f32; 4],
    pub corner: f32,
    pub font_px: f32,
    /// baseline-to-baseline distance
    pub line_h: f32,
    /// extra space after each character (the original uses fixed cells)
    pub char_gap: f32,
    pub lines_per_page: usize,
    /// speaker on its own first line ("Name:"), coloured with `name_color`
    pub name_line: bool,
    pub name_color: u8,
    pub body_color: u8,
    /// `<color=N>` palette, RGBA
    pub palette: Vec<[u8; 4]>,
    pub outline_px: f32,
    pub outline: [u8; 4],
    pub shadow: [f32; 2],
    pub shadow_color: [u8; 4],
    pub fill_top: [u8; 4],
    pub fill_bottom: [u8; 4],
    pub border_outer: [u8; 4],
    pub border_inner: [u8; 4],
    pub border_px: f32,
    /// draw the cloud-scroll corner ornaments
    pub ornaments: bool,
    pub ornament: [u8; 4],
    /// ms per character for `<speed=0..9/>`
    pub speed_ms: [f32; 10],
    pub default_speed: u8,
    /// font scale for `<glyph=0..9/>` (meaning in the original unconfirmed; 1.0 = unchanged)
    pub glyph_scale: [f32; 10],
    /// open animation (s) and slide distance (ref px)
    pub open_s: f32,
    pub slide: f32,
    /// "more" marker, relative to the box's bottom-right corner
    pub more_at: [f32; 2],
    pub more_color: [u8; 4],
}

impl Default for DialogueTheme {
    fn default() -> Self {
        Self {
            style: "modern".into(),
            ref_w: 640.0,
            ref_h: 480.0,
            rect: [24.0, 352.0, 592.0, 116.0],
            pad: [22.0, 18.0, 22.0, 14.0],
            corner: 6.0,
            font_px: 19.0,
            line_h: 26.0,
            char_gap: 1.0,
            lines_per_page: 3,
            name_line: true,
            name_color: 6,
            body_color: 0,
            palette: vec![
                [242, 236, 220, 255], // 0 body (paper white)
                [168, 168, 168, 255], // 1 grey
                [232, 88, 72, 255],   // 2 red
                [112, 200, 120, 255], // 3 green
                [104, 160, 240, 255], // 4 blue
                [236, 220, 160, 255], // 5 pale gold
                [244, 196, 92, 255],  // 6 gold (names)
                [150, 140, 120, 255], // 7 hint
                [200, 140, 230, 255], // 8 violet
                [120, 220, 220, 255], // 9 cyan
            ],
            outline_px: 2.2,
            outline: [12, 10, 18, 230],
            shadow: [1.2, 1.6],
            shadow_color: [0, 0, 0, 150],
            fill_top: [22, 30, 52, 214],
            fill_bottom: [8, 12, 26, 228],
            border_outer: [196, 160, 96, 255],
            border_inner: [92, 72, 44, 255],
            border_px: 2.0,
            ornaments: true,
            ornament: [214, 178, 108, 255],
            speed_ms: [0.0, 17.5, 35.0, 52.5, 70.0, 87.5, 105.0, 140.0, 175.0, 210.0],
            default_speed: 2,
            glyph_scale: [1.0; 10],
            open_s: 0.18,
            slide: 10.0,
            more_at: [-20.0, -14.0],
            more_color: [244, 196, 92, 255],
        }
    }
}

impl DialogueTheme {
    pub fn from_toml(src: &str) -> Result<Self, String> {
        toml::from_str(src).map_err(|e| e.to_string())
    }
    pub fn color(&self, i: u8) -> Color {
        rgb(*self.palette.get(i as usize).or(self.palette.first()).unwrap_or(&[255, 255, 255, 255]))
    }
}

#[derive(Clone, Debug)]
struct G {
    c: PlacedChar,
    /// visual line on the page, x in ref px (scaled by the glyph's own size)
    line: usize,
    x: f32,
    color: u8,
    scale: f32,
    /// ms before this character appears (after the previous one)
    delay: f32,
}

#[derive(Clone, Debug, Default)]
struct Page {
    glyphs: Vec<G>,
    /// first glyph that is shown without typing (an `<instant/>` cue)
    instant_from: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DlgEvent {
    None,
    /// the last page was dismissed: the engine moves the script on
    Done,
}

/// A dialogue on screen: pages, typewriter progress, open animation.
pub struct DialogueView {
    pub theme: DialogueTheme,
    pub text: DlgText,
    pages: Vec<Page>,
    pub page: usize,
    /// characters shown on the current page
    shown: usize,
    acc_ms: f32,
    open: f32,
    t: f32,
    pub unknown_cues: Vec<String>,
}

impl DialogueView {
    pub fn new(fonts: &Fonts, theme: DialogueTheme, text: DlgText, name_sep: &str) -> Self {
        let mut v = Self { pages: Vec::new(), page: 0, shown: 0, acc_ms: 0.0, open: 0.0, t: 0.0, unknown_cues: Vec::new(), theme, text };
        v.layout(fonts, name_sep);
        v
    }

    fn style(&self, scale: f32) -> Style {
        let mut st = Style::new(self.theme.font_px * scale, Color::WHITE).spacing(self.theme.char_gap * scale);
        st.dialog = true;
        st
    }

    /// Re-layout (fonts or locale changed). Keeps the page index if still valid.
    pub fn layout(&mut self, fonts: &Fonts, name_sep: &str) {
        let th = self.theme.clone();
        let inner_w = th.rect[2] - th.pad[0] - th.pad[2];
        let mut pages = vec![Page::default()];
        let mut line = 0usize;
        let mut level = th.default_speed.min(9);
        let mut scale = 1.0f32;
        let mut unknown = Vec::new();
        // speaker line
        let mut src: Vec<(Vec<DlgSpan>, Option<usize>)> = Vec::new();
        if th.name_line {
            if let Some(n) = &self.text.speaker {
                src.push((vec![DlgSpan { color: th.name_color, text: format!("{n}{name_sep}") }], None));
            }
        }
        for (i, l) in self.text.lines.iter().enumerate() {
            src.push((l.clone(), Some(i)));
        }
        for (spans, li) in src {
            let mut x = 0.0f32;
            let mut col = 0usize;
            let mut last_space: Option<usize> = None;
            let cues: Vec<&(usize, usize, DlgCue)> = li.map(|li| self.text.cues.iter().filter(|c| c.0 == li).collect()).unwrap_or_default();
            let apply = |col: usize, pages: &mut Vec<Page>, line: &mut usize, x: &mut f32, level: &mut u8, scale: &mut f32, unknown: &mut Vec<String>| {
                for c in cues.iter().filter(|c| c.1 == col) {
                    match &c.2 {
                        DlgCue::Page => {
                            if !pages.last().unwrap().glyphs.is_empty() {
                                pages.push(Page::default());
                            }
                            *line = 0;
                            *x = 0.0;
                        }
                        DlgCue::Instant => {
                            let p = pages.last_mut().unwrap();
                            p.instant_from.get_or_insert(p.glyphs.len());
                        }
                        DlgCue::Speed(n) => *level = (*n).min(9),
                        DlgCue::Glyph(n) => *scale = th.glyph_scale[(*n).min(9) as usize],
                        DlgCue::Ctl(k, d) => unknown.push(format!("%{k}{}", d.map(|d| d.to_string()).unwrap_or_default())),
                    }
                }
            };
            for sp in spans {
                for ch in sp.text.chars() {
                    apply(col, &mut pages, &mut line, &mut x, &mut level, &mut scale, &mut unknown);
                    let st = self.style(scale);
                    let mut s = [0u8; 4];
                    let Some(pc) = text::place_chars(fonts, &st, ch.encode_utf8(&mut s)).into_iter().next() else {
                        col += 1;
                        continue;
                    };
                    let adv = pc.adv + st.spacing;
                    if x > 0.0 && x + pc.adv > inner_w {
                        // wrap: Latin moves the current word down, CJK breaks anywhere; no line-initial closing punctuation
                        let p = pages.last_mut().unwrap();
                        let carry_from = if ch == ' ' {
                            None
                        } else if ch.is_alphanumeric() && (ch as u32) < 0x2E80 {
                            last_space.map(|i| i + 1)
                        } else if NO_LINE_START.contains(ch) {
                            p.glyphs.len().checked_sub(1)
                        } else {
                            None
                        };
                        line += 1;
                        let mut carried: Vec<G> = match carry_from {
                            Some(i) if i < p.glyphs.len() && p.glyphs[i].line == line - 1 => p.glyphs.split_off(i),
                            _ => Vec::new(),
                        };
                        x = 0.0;
                        if line >= th.lines_per_page {
                            pages.push(Page::default());
                            line = 0;
                        }
                        let p = pages.last_mut().unwrap();
                        for mut g in carried.drain(..) {
                            g.line = line;
                            g.x = x;
                            x += g.c.adv + th.char_gap * g.scale;
                            p.glyphs.push(g);
                        }
                        last_space = None;
                        if ch == ' ' {
                            col += 1;
                            continue;
                        }
                    }
                    let p = pages.last_mut().unwrap();
                    if ch == ' ' {
                        last_space = Some(p.glyphs.len());
                    }
                    let delay = if ch == ' ' { 0.0 } else { th.speed_ms[level as usize] };
                    p.glyphs.push(G { c: pc, line, x, color: sp.color, scale, delay });
                    x += adv;
                    col += 1;
                }
            }
            apply(col, &mut pages, &mut line, &mut x, &mut level, &mut scale, &mut unknown);
            if x > 0.0 || li.is_none() {
                line += 1;
            }
            if line >= th.lines_per_page {
                pages.push(Page::default());
                line = 0;
            }
        }
        while pages.len() > 1 && pages.last().is_some_and(|p| p.glyphs.is_empty()) {
            pages.pop();
        }
        // body colour 0 maps to the theme's body colour
        for p in &mut pages {
            for g in &mut p.glyphs {
                if g.color == 0 {
                    g.color = th.body_color;
                }
            }
        }
        self.page = self.page.min(pages.len().saturating_sub(1));
        self.pages = pages;
        self.unknown_cues = unknown;
    }

    pub fn pages(&self) -> usize {
        self.pages.len()
    }

    fn cur(&self) -> &Page {
        &self.pages[self.page]
    }

    pub fn page_complete(&self) -> bool {
        self.shown >= self.cur().glyphs.len()
    }

    /// Advance the typewriter by `dt` seconds of real time.
    pub fn update(&mut self, dt: f32) {
        self.t += dt;
        let was = self.open;
        self.open = (self.open + dt / self.theme.open_s.max(1e-3)).min(1.0);
        if was < 1.0 {
            return;
        }
        let p = &self.pages[self.page];
        self.acc_ms += dt * 1000.0;
        while self.shown < p.glyphs.len() {
            if p.instant_from.is_some_and(|i| self.shown >= i) {
                self.shown = p.glyphs.len();
                break;
            }
            let d = p.glyphs[self.shown].delay;
            if d > self.acc_ms {
                break;
            }
            self.acc_ms -= d;
            self.shown += 1;
        }
        if self.shown >= p.glyphs.len() {
            self.acc_ms = 0.0;
        }
    }

    /// Confirm (Enter / Space / click / A): finish the page, else next page, else [`DlgEvent::Done`].
    pub fn confirm(&mut self) -> DlgEvent {
        if !self.page_complete() {
            self.shown = self.cur().glyphs.len();
            return DlgEvent::None;
        }
        if self.page + 1 < self.pages.len() {
            self.page += 1;
            self.shown = 0;
            self.acc_ms = 0.0;
            return DlgEvent::None;
        }
        DlgEvent::Done
    }

    /// Scale and offset of the reference space in a `w` x `h` window.
    pub fn frame(&self, w: f32, h: f32) -> (f32, f32) {
        let s = h / self.theme.ref_h;
        (s, (w - self.theme.ref_w * s) / 2.0)
    }

    pub fn box_rect(&self, w: f32, h: f32) -> Rect {
        let (s, ox) = self.frame(w, h);
        let r = self.theme.rect;
        Rect::new((ox + r[0] * s) as f64, (r[1] * s) as f64, (ox + (r[0] + r[2]) * s) as f64, ((r[1] + r[3]) * s) as f64)
    }

    pub fn draw(&self, sc: &mut Scene, fonts: &Fonts, w: f32, h: f32, alpha: f32) {
        let th = &self.theme;
        let k = crate::anim::ease_out_cubic(self.open) * alpha;
        if k <= 0.0 {
            return;
        }
        let (s, _) = self.frame(w, h);
        let lift = (1.0 - k) * th.slide * s;
        let r = self.box_rect(w, h) + vello::kurbo::Vec2::new(0.0, lift as f64);
        sc.push_layer(Fill::NonZero, vello::peniko::Mix::Normal, k, Affine::IDENTITY, &r.inflate(40.0, 40.0));
        draw_frame(sc, th, r, s);
        // text
        let p = self.cur();
        let x0 = r.x0 as f32 + th.pad[0] * s;
        let y0 = r.y0 as f32 + th.pad[1] * s;
        let (asc, _) = text::metrics(fonts, &self.style(s));
        let n = self.shown.min(p.glyphs.len());
        // partial fade of the newest glyph for a softer typewriter
        let frac = if n < p.glyphs.len() && p.glyphs[n].delay > 0.0 { (self.acc_ms / p.glyphs[n].delay).clamp(0.0, 1.0) } else { 0.0 };
        let vis: Vec<(usize, f32)> = (0..n).map(|i| (i, 1.0)).chain((frac > 0.05 && n < p.glyphs.len()).then_some((n, frac))).collect();
        // group by (scale, colour, alpha)
        let outline = Stroke::new((th.outline_px * s) as f64).with_join(vello::kurbo::Join::Round);
        for pass in 0..3 {
            let mut i = 0;
            while i < vis.len() {
                let g0 = &p.glyphs[vis[i].0];
                let key = (g0.scale, g0.color, vis[i].1);
                let mut j = i;
                let mut run: Vec<(PlacedChar, f32)> = Vec::new();
                while j < vis.len() {
                    let g = &p.glyphs[vis[j].0];
                    if (g.scale, g.color, vis[j].1) != key {
                        break;
                    }
                    let mut c = g.c;
                    c.x = g.x * s;
                    run.push((c, g.line as f32 * th.line_h * s));
                    j += 1;
                }
                let st = self.style(s * key.0);
                let a = key.2;
                let base = Affine::translate((x0 as f64, (y0 + asc) as f64));
                match pass {
                    0 if th.shadow_color[3] > 0 => {
                        let b = Brush::Solid(rgb(th.shadow_color).multiply_alpha(a));
                        text::paint_chars(sc, fonts, &st, &run, &b, Affine::translate(((th.shadow[0] * s) as f64, (th.shadow[1] * s) as f64)) * base, None);
                    }
                    1 if th.outline_px > 0.0 => {
                        let b = Brush::Solid(rgb(th.outline).multiply_alpha(a));
                        text::paint_chars(sc, fonts, &st, &run, &b, base, Some(&outline));
                    }
                    2 => {
                        let b = Brush::Solid(th.color(key.1).multiply_alpha(a));
                        text::paint_chars(sc, fonts, &st, &run, &b, base, None);
                    }
                    _ => {}
                }
                i = j;
            }
        }
        // "more" marker once the page is out
        if self.page_complete() {
            let bob = (self.t * 4.0).sin() * 2.0 * s;
            let c = Point::new(r.x1 + (th.more_at[0] * s) as f64, r.y1 + (th.more_at[1] * s + bob) as f64);
            let z = (4.5 * s) as f64;
            let mut tri = BezPath::new();
            tri.move_to((c.x - z, c.y - z * 0.6));
            tri.line_to((c.x + z, c.y - z * 0.6));
            tri.line_to((c.x, c.y + z * 0.7));
            tri.close_path();
            sc.fill(Fill::NonZero, Affine::IDENTITY, rgb(th.more_color), None, &tri);
        }
        sc.pop_layer();
    }
}

/// Box frame: vertical gradient fill, double border, cloud-scroll corner ornaments.
pub fn draw_frame(sc: &mut Scene, th: &DialogueTheme, r: Rect, s: f32) {
    if th.style == "parchment" {
        crate::orig::scroll(sc, r, s as f64, 1.0, th.fill_top[3] as f32 / 255.0);
        if th.ornaments {
            for (cx, cy, dx, dy) in [(r.x0 + 8.0 * s as f64, r.y0, 1.0, 1.0), (r.x1 - 8.0 * s as f64, r.y0, -1.0, 1.0), (r.x0 + 8.0 * s as f64, r.y1, 1.0, -1.0), (r.x1 - 8.0 * s as f64, r.y1, -1.0, -1.0)] {
                cloud_corner(sc, Point::new(cx, cy), dx, dy, s as f64, rgb(th.ornament));
            }
        }
        return;
    }
    let rr = RoundedRect::from_rect(r, (th.corner * s) as f64);
    let g = Gradient::new_linear((r.x0, r.y0), (r.x0, r.y1)).with_stops([rgb(th.fill_top), rgb(th.fill_bottom)]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &rr);
    let bw = (th.border_px * s) as f64;
    sc.stroke(&Stroke::new(bw), Affine::IDENTITY, rgb(th.border_outer), None, &rr);
    let inner = RoundedRect::from_rect(r.inset(-(bw * 2.0)), ((th.corner * s) as f64 - bw * 2.0).max(0.0));
    sc.stroke(&Stroke::new(bw * 0.5), Affine::IDENTITY, rgb(th.border_inner), None, &inner);
    if th.ornaments {
        for (cx, cy, dx, dy) in [(r.x0, r.y0, 1.0, 1.0), (r.x1, r.y0, -1.0, 1.0), (r.x0, r.y1, 1.0, -1.0), (r.x1, r.y1, -1.0, -1.0)] {
            cloud_corner(sc, Point::new(cx, cy), dx, dy, s as f64, rgb(th.ornament));
        }
    }
}

/// A small cloud-scroll (xiangyun) hook in a corner, pointing inwards.
fn cloud_corner(sc: &mut Scene, p: Point, dx: f64, dy: f64, s: f64, c: Color) {
    let m = |x: f64, y: f64| Point::new(p.x + dx * x * s, p.y + dy * y * s);
    let mut b = BezPath::new();
    b.move_to(m(3.0, 16.0));
    b.line_to(m(3.0, 6.0));
    b.quad_to(m(3.0, 3.0), m(6.0, 3.0));
    b.line_to(m(16.0, 3.0));
    sc.stroke(&Stroke::new(1.6 * s).with_caps(vello::kurbo::Cap::Round), Affine::IDENTITY, c, None, &b);
    let mut sp = BezPath::new();
    sp.move_to(m(9.0, 9.0));
    sp.curve_to(m(9.0, 5.5), m(14.0, 5.5), m(14.0, 9.0));
    sp.curve_to(m(14.0, 11.5), m(10.5, 12.0), m(10.5, 9.5));
    sc.stroke(&Stroke::new(1.2 * s).with_caps(vello::kurbo::Cap::Round), Affine::IDENTITY, c, None, &sp);
    sc.fill(Fill::NonZero, Affine::IDENTITY, c, None, &vello::kurbo::Circle::new(m(5.5, 5.5), 1.4 * s));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> Fonts {
        Fonts::load(Vec::new())
    }
    fn txt(lines: &[&str], cues: Vec<(usize, usize, DlgCue)>) -> DlgText {
        DlgText { speaker: Some("Ann".into()), lines: lines.iter().map(|l| vec![DlgSpan { color: 0, text: l.to_string() }]).collect(), cues }
    }

    #[test]
    fn typewriter_and_pages() {
        let f = fonts();
        let th = DialogueTheme::default();
        let mut v = DialogueView::new(&f, th.clone(), txt(&["abc", "def"], vec![(1, 0, DlgCue::Page)]), ": ");
        assert_eq!(v.pages(), 2, "page cue splits");
        v.update(1.0); // open
        assert!(v.shown <= 1);
        // speaker "Ann: " (5 chars, 1 space without delay) + "abc" at 35 ms each
        v.update(0.035 * 7.0 + 0.001);
        assert!(v.page_complete());
        assert_eq!(v.confirm(), DlgEvent::None);
        assert_eq!(v.page, 1);
        assert!(!v.page_complete());
        assert_eq!(v.confirm(), DlgEvent::None, "first confirm finishes the page");
        assert!(v.page_complete());
        assert_eq!(v.confirm(), DlgEvent::Done);
    }

    #[test]
    fn instant_and_speed() {
        let f = fonts();
        let mut v = DialogueView::new(&f, DialogueTheme::default(), txt(&["abcdef"], vec![(0, 2, DlgCue::Instant)]), ": ");
        v.update(1.0);
        v.update(0.035 * 7.0);
        assert!(v.page_complete(), "instant shows the rest");
        let mut v = DialogueView::new(&f, DialogueTheme::default(), txt(&["abcdef"], vec![(0, 0, DlgCue::Speed(0))]), ": ");
        v.update(1.0);
        v.update(0.035 * 5.0);
        assert!(v.page_complete(), "speed 0 = no delay");
    }

    #[test]
    fn wraps_and_auto_pages() {
        let f = fonts();
        let long = "word ".repeat(80);
        let v = DialogueView::new(&f, DialogueTheme::default(), txt(&[&long], vec![]), ": ");
        assert!(v.pages() >= 2, "{}", v.pages());
        for p in &v.pages {
            assert!(p.glyphs.iter().all(|g| g.line < 3));
            let maxx = p.glyphs.iter().map(|g| g.x + g.c.adv).fold(0.0, f32::max);
            assert!(maxx <= 592.0 - 44.0 + 0.5, "{maxx}");
        }
    }

    #[test]
    fn theme_toml_overrides() {
        let t = DialogueTheme::from_toml("font_px = 22.0\nlines_per_page = 4\n").unwrap();
        assert_eq!(t.lines_per_page, 4);
        assert_eq!(t.palette.len(), 10);
    }
}
