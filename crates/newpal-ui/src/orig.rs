//! Vector re-drawings of the original game's UI materials (measured from the player's own `Resource/*.png` at
//! analysis time; no pixels are copied): burlap plaques with charred edges, cream scroll paper, wooden rods with
//! bamboo ties, the U-shaped menu frame, brush-label states (normal: dark-brown ink with a pale rim; focused:
//! vermilion with a white glow; disabled: grey), the gold-ingot money box.
//!
//! Everything is laid out in the original's 640x480 reference space; [`Ref`] maps it to the window
//! (scale by height, centred horizontally - like the game view).
use crate::anim::hash;
use crate::text::{self, Align, Fonts, Style};
use vello::kurbo::{Affine, BezPath, Circle, Point, Rect, RoundedRect, Stroke};
use vello::peniko::{Color, Fill, Gradient};
use vello::Scene;

pub const C_INK: Color = Color::from_rgba8(62, 34, 18, 255);
pub const C_RIM: Color = Color::from_rgba8(252, 232, 186, 255);
pub const C_VERMILION: Color = Color::from_rgba8(222, 30, 18, 255);
pub const C_PAPER: Color = Color::from_rgba8(240, 232, 215, 255);
pub const C_PAPER_EDGE: Color = Color::from_rgba8(206, 186, 142, 255);
pub const C_BURLAP_HI: Color = Color::from_rgba8(244, 198, 100, 255);
pub const C_BURLAP_LO: Color = Color::from_rgba8(206, 130, 50, 255);
pub const C_CHAR: Color = Color::from_rgba8(74, 40, 18, 255);
pub const C_ROD_D: Color = Color::from_rgba8(96, 66, 44, 255);
pub const C_ROD: Color = Color::from_rgba8(130, 100, 75, 255);
pub const C_ROD_HI: Color = Color::from_rgba8(182, 143, 107, 255);

fn ca(c: Color, a: f32) -> Color {
    c.multiply_alpha(a.clamp(0.0, 1.0))
}

/// Reference space -> window.
#[derive(Clone, Copy, Debug)]
pub struct Ref {
    pub s: f32,
    pub ox: f32,
}

impl Ref {
    pub fn new(w: f32, h: f32) -> Self {
        let s = h / 480.0;
        Self { s, ox: (w - 640.0 * s) / 2.0 }
    }
    pub fn p(&self, x: f32, y: f32) -> Point {
        Point::new((self.ox + x * self.s) as f64, (y * self.s) as f64)
    }
    pub fn r(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new((self.ox + x * self.s) as f64, (y * self.s) as f64, (self.ox + (x + w) * self.s) as f64, ((y + h) * self.s) as f64)
    }
    /// window point -> reference point
    pub fn inv(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.ox) / self.s, y / self.s)
    }
}

/// Rectangle outline with a hand-made, slightly torn edge (deterministic per `seed`).
fn torn(r: Rect, amp: f64, step: f64, seed: u32) -> BezPath {
    let mut b = BezPath::new();
    let mut i = seed;
    let mut jit = || {
        i = i.wrapping_add(1);
        (hash(i) as f64 - 0.5) * 2.0 * amp
    };
    let n = |len: f64| ((len / step).ceil() as usize).max(2);
    let (nx, ny) = (n(r.width()), n(r.height()));
    b.move_to((r.x0 + jit(), r.y0 + jit()));
    for k in 1..=nx {
        b.line_to((r.x0 + r.width() * k as f64 / nx as f64, r.y0 + jit()));
    }
    for k in 1..=ny {
        b.line_to((r.x1 + jit(), r.y0 + r.height() * k as f64 / ny as f64));
    }
    for k in 1..=nx {
        b.line_to((r.x1 - r.width() * k as f64 / nx as f64, r.y1 + jit()));
    }
    for k in 1..ny {
        b.line_to((r.x0 + jit(), r.y1 - r.height() * k as f64 / ny as f64));
    }
    b.close_path();
    b
}

/// Burlap plaque (menu buttons, option panels). `state`: 0 normal, 1 focused, 2 disabled. `glow` 0..1 animates focus.
pub fn plaque(sc: &mut Scene, r: Rect, s: f32, seed: u32, disabled: bool, glow: f32, alpha: f32) {
    let s64 = s as f64;
    let (hi, lo) = if disabled {
        (Color::from_rgba8(176, 170, 160, 255), Color::from_rgba8(136, 130, 122, 255))
    } else {
        (crate::screens::lerp_c(C_BURLAP_HI, Color::from_rgba8(246, 214, 150, 255), glow * 0.6), C_BURLAP_LO)
    };
    // charred rim
    sc.fill(Fill::NonZero, Affine::IDENTITY, ca(C_CHAR, 0.95 * alpha), None, &torn(r, 1.3 * s64, 5.0 * s64, seed));
    let inner = r.inset(-2.4 * s64);
    let g = Gradient::new_radial(inner.center(), (inner.width().max(inner.height()) * 0.62) as f32).with_stops([ca(hi, alpha), ca(lo, alpha)]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &torn(inner, 0.9 * s64, 4.0 * s64, seed ^ 0x9e37));
    // weave
    sc.push_layer(Fill::NonZero, vello::peniko::Mix::Normal, alpha, Affine::IDENTITY, &inner);
    let step = (2.0 * s64).max(1.5);
    let thread = Stroke::new((0.55 * s64).max(0.5));
    let mut y = inner.y0;
    let mut k = seed;
    while y < inner.y1 {
        k = k.wrapping_add(7);
        let c = ca(C_CHAR, 0.04 + hash(k) * 0.07);
        sc.stroke(&thread, Affine::IDENTITY, c, None, &vello::kurbo::Line::new((inner.x0, y), (inner.x1, y)));
        y += step;
    }
    let mut x = inner.x0;
    while x < inner.x1 {
        k = k.wrapping_add(13);
        let c = ca(Color::from_rgba8(255, 236, 180, 255), 0.015 + hash(k) * 0.03);
        sc.stroke(&thread, Affine::IDENTITY, c, None, &vello::kurbo::Line::new((x, inner.y0), (x, inner.y1)));
        x += step;
    }
    // burnt vignette towards the edges
    for i in 0..5 {
        let d = i as f64 * 1.6 * s64;
        let a = 0.22 - i as f32 * 0.04;
        sc.stroke(&Stroke::new(2.0 * s64), Affine::IDENTITY, ca(C_CHAR, a), None, &inner.inset(-d));
    }
    sc.pop_layer();
}

/// Brush label on a plaque: dark ink with a pale rim; focused -> vermilion with a white glow; disabled -> grey.
/// Four CJK characters are set 2 + 2 on two lines like the original buttons.
pub fn label(sc: &mut Scene, fonts: &Fonts, txt: &str, r: Rect, s: f32, disabled: bool, glow: f32, alpha: f32) {
    let chars: Vec<char> = txt.chars().collect();
    let cjk = chars.iter().all(|c| (*c as u32) >= 0x2E80);
    let lines: Vec<String> = if cjk && chars.len() == 4 { vec![chars[..2].iter().collect(), chars[2..].iter().collect()] } else { vec![txt.to_string()] };
    let n = lines.len() as f32;
    let mut size = if n > 1.0 { (r.height() as f32 / n) * 0.90 } else { r.height() as f32 * 0.66 };
    let gap = if cjk { size * if n > 1.0 { 0.06 } else { 0.30 } } else { 0.0 };
    // fit width
    for l in &lines {
        let st = Style::new(size, C_INK).spacing(gap).title();
        let w = text::measure(fonts, &st, l);
        let maxw = r.width() as f32 * 0.84;
        if w > maxw {
            size *= maxw / w;
        }
    }
    let fill = if disabled { Color::from_rgba8(92, 90, 86, 255) } else { crate::screens::lerp_c(C_INK, C_VERMILION, glow) };
    let rim = if disabled { Color::from_rgba8(190, 186, 178, 255) } else { crate::screens::lerp_c(C_RIM, Color::WHITE, glow) };
    let lh = size * 1.02;
    let top = r.center().y as f32 - lh * n / 2.0;
    for (i, l) in lines.iter().enumerate() {
        let st = Style::new(size, fill).spacing(gap).title().align(Align::Center);
        let line = text::shape(fonts, &st, l);
        let base = top + lh * i as f32 + (lh + line.ascent + line.descent) / 2.0;
        let cx = r.center().x as f32;
        let x0 = cx - line.width / 2.0;
        let xf = Affine::translate((x0 as f64, base as f64));
        let chars = text::place_chars(fonts, &st, l).into_iter().map(|c| (c, 0.0)).collect::<Vec<_>>();
        if glow > 0.01 && !disabled {
            let halo = Stroke::new((size * 0.30 * glow) as f64).with_join(vello::kurbo::Join::Round);
            text::paint_chars(sc, fonts, &st, &chars, &vello::peniko::Brush::Solid(ca(Color::WHITE, 0.35 * glow * alpha)), xf, Some(&halo));
        }
        let rimw = Stroke::new((size * (0.20 + 0.06 * glow)) as f64).with_join(vello::kurbo::Join::Round);
        text::paint_chars(sc, fonts, &st, &chars, &vello::peniko::Brush::Solid(ca(rim, alpha)), xf, Some(&rimw));
        // brush weight: the original labels are heavy calligraphy, so embolden the face a little
        let bold = Stroke::new((size * 0.045) as f64).with_join(vello::kurbo::Join::Round);
        text::paint_chars(sc, fonts, &st, &chars, &vello::peniko::Brush::Solid(ca(fill, alpha)), xf, Some(&bold));
        text::paint_chars(sc, fonts, &st, &chars, &vello::peniko::Brush::Solid(ca(fill, alpha)), xf, None);
        let _ = s;
    }
}

/// Vertical wooden rod with bamboo ties (from `y0` to `y1` at centre `x`), window pixels.
pub fn rod(sc: &mut Scene, x: f64, y0: f64, y1: f64, s: f64, alpha: f32, ties: bool) {
    let w = 7.0 * s;
    let r = RoundedRect::new(x - w / 2.0, y0, x + w / 2.0, y1, w / 2.0);
    let g = Gradient::new_linear((x - w / 2.0, 0.0), (x + w / 2.0, 0.0)).with_stops([ca(C_ROD_D, alpha), ca(C_ROD_HI, alpha), ca(C_ROD, alpha), ca(C_ROD_D, alpha)]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &r);
    if ties {
        let mut y = y0 + 22.0 * s;
        while y < y1 - 16.0 * s {
            let t = Stroke::new(1.1 * s);
            let c = ca(Color::from_rgba8(232, 214, 180, 255), 0.9 * alpha);
            sc.stroke(&t, Affine::IDENTITY, c, None, &vello::kurbo::Line::new((x - w * 0.55, y - 2.2 * s), (x + w * 0.55, y + 2.2 * s)));
            sc.stroke(&t, Affine::IDENTITY, c, None, &vello::kurbo::Line::new((x - w * 0.55, y + 2.2 * s), (x + w * 0.55, y - 2.2 * s)));
            y += 74.0 * s;
        }
    }
}

/// The original's U-shaped menu frame: two rods joined by a curved bottom rail.
pub fn u_frame(sc: &mut Scene, r: Rect, s: f64, alpha: f32) {
    let rad = 26.0 * s;
    rod(sc, r.x0, r.y0, r.y1 - rad, s, alpha, true);
    rod(sc, r.x1, r.y0, r.y1 - rad, s, alpha, true);
    let mut b = BezPath::new();
    b.move_to((r.x0, r.y1 - rad - 2.0 * s));
    b.quad_to((r.x0, r.y1), (r.x0 + rad, r.y1));
    b.line_to((r.x1 - rad, r.y1));
    b.quad_to((r.x1, r.y1), (r.x1, r.y1 - rad - 2.0 * s));
    sc.stroke(&Stroke::new(7.0 * s).with_caps(vello::kurbo::Cap::Round), Affine::IDENTITY, ca(C_ROD_D, alpha), None, &b);
    sc.stroke(&Stroke::new(3.4 * s).with_caps(vello::kurbo::Cap::Round), Affine::IDENTITY, ca(C_ROD_HI, alpha), None, &b);
    // brass knuckles at the joints and the bottom centre
    for p in [Point::new(r.x0, r.y1 - rad), Point::new(r.x1, r.y1 - rad), Point::new(r.center().x, r.y1)] {
        sc.fill(Fill::NonZero, Affine::IDENTITY, ca(Color::from_rgba8(214, 176, 112, 255), alpha), None, &Circle::new(p, 3.4 * s));
        sc.stroke(&Stroke::new(1.0 * s), Affine::IDENTITY, ca(C_ROD_D, alpha), None, &Circle::new(p, 3.4 * s));
    }
    // rod caps
    for x in [r.x0, r.x1] {
        let cap = RoundedRect::new(x - 5.0 * s, r.y0 - 4.0 * s, x + 5.0 * s, r.y0 + 4.0 * s, 2.0 * s);
        sc.fill(Fill::NonZero, Affine::IDENTITY, ca(Color::from_rgba8(204, 164, 98, 255), alpha), None, &cap);
    }
}

/// Cream scroll paper (dialogue / list backgrounds) with wooden rods on the left and right.
pub fn scroll(sc: &mut Scene, r: Rect, s: f64, alpha: f32, fill_a: f32) {
    let g = Gradient::new_linear((r.x0, r.y0), (r.x0, r.y1)).with_stops([
        ca(C_PAPER_EDGE, fill_a * alpha),
        ca(C_PAPER, fill_a * alpha),
        ca(C_PAPER, fill_a * alpha),
        ca(Color::from_rgba8(226, 212, 184, 255), fill_a * alpha),
    ]);
    let body = Rect::new(r.x0 + 4.0 * s, r.y0, r.x1 - 4.0 * s, r.y1);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &body);
    // rolled top edge shadow + fibres
    let band = Gradient::new_linear((0.0, r.y0), (0.0, r.y0 + 12.0 * s)).with_stops([ca(Color::from_rgba8(150, 118, 70, 255), 0.45 * alpha), ca(C_PAPER, 0.0)]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &band, None, &Rect::new(body.x0, r.y0, body.x1, r.y0 + 12.0 * s));
    let mut k = 77u32;
    for _ in 0..((body.width() / (9.0 * s)) as usize).min(400) {
        k = k.wrapping_add(31);
        let x = body.x0 + hash(k) as f64 * body.width();
        let y = body.y0 + hash(k ^ 0x55) as f64 * body.height();
        let l = (6.0 + hash(k ^ 0xaa) as f64 * 14.0) * s;
        sc.stroke(&Stroke::new(0.5 * s), Affine::IDENTITY, ca(Color::from_rgba8(170, 150, 110, 255), 0.18 * alpha * fill_a), None, &vello::kurbo::Line::new((x, y), (x + l, y + (hash(k ^ 0x33) as f64 - 0.5) * 3.0 * s)));
    }
    rod(sc, r.x0 + 3.0 * s, r.y0 - 3.0 * s, r.y1 + 3.0 * s, s, alpha, false);
    rod(sc, r.x1 - 3.0 * s, r.y0 - 3.0 * s, r.y1 + 3.0 * s, s, alpha, false);
}

/// Gold ingot icon in a square gold tile + dark fading bar (the original money box).
pub fn money_box(sc: &mut Scene, fonts: &Fonts, r: Rect, s: f64, amount: &str, alpha: f32) {
    let bar = Gradient::new_linear((r.x0, 0.0), (r.x1, 0.0)).with_stops([ca(Color::from_rgba8(40, 40, 46, 255), 0.9 * alpha), ca(Color::from_rgba8(40, 40, 46, 255), 0.0)]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &bar, None, &Rect::new(r.x0 + 14.0 * s, r.y0 + 6.0 * s, r.x1, r.y1 - 6.0 * s));
    let t = Rect::new(r.x0 + 2.0 * s, r.y0 + 4.0 * s, r.x0 + 34.0 * s, r.y0 + 36.0 * s);
    let g = Gradient::new_linear((t.x0, t.y0), (t.x1, t.y1)).with_stops([ca(Color::from_rgba8(252, 220, 110, 255), alpha), ca(Color::from_rgba8(196, 128, 24, 255), alpha)]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &RoundedRect::from_rect(t, 4.0 * s));
    sc.stroke(&Stroke::new(1.2 * s), Affine::IDENTITY, ca(Color::from_rgba8(120, 70, 10, 255), alpha), None, &RoundedRect::from_rect(t, 4.0 * s));
    // yuanbao: boat body + dome
    let c = t.center();
    let mut b = BezPath::new();
    b.move_to((c.x - 11.0 * s, c.y - 1.0 * s));
    b.quad_to((c.x, c.y + 3.0 * s), (c.x + 11.0 * s, c.y - 1.0 * s));
    b.quad_to((c.x + 8.0 * s, c.y + 9.0 * s), (c.x, c.y + 9.0 * s));
    b.quad_to((c.x - 8.0 * s, c.y + 9.0 * s), (c.x - 11.0 * s, c.y - 1.0 * s));
    b.close_path();
    sc.fill(Fill::NonZero, Affine::IDENTITY, ca(Color::from_rgba8(255, 236, 150, 255), alpha), None, &b);
    sc.stroke(&Stroke::new(1.0 * s), Affine::IDENTITY, ca(Color::from_rgba8(140, 84, 10, 255), alpha), None, &b);
    let dome = vello::kurbo::Ellipse::new((c.x, c.y - 2.0 * s), (5.5 * s, 4.5 * s), 0.0);
    sc.fill(Fill::NonZero, Affine::IDENTITY, ca(Color::from_rgba8(255, 214, 90, 255), alpha), None, &dome);
    sc.stroke(&Stroke::new(1.0 * s), Affine::IDENTITY, ca(Color::from_rgba8(140, 84, 10, 255), alpha), None, &dome);
    let st = Style::new((18.0 * s) as f32, ca(C_PAPER, alpha)).dialog_font();
    text::draw_mid(sc, fonts, &st, amount, (t.x1 + 10.0 * s) as f32, c.y as f32);
}

/// The original menu covers the world with a sepia ink painting on burlap; this is a procedural stand-in:
/// warm orange-brown wash, a lighter glow upper-middle, dark ink blotches, faint vertical weave.
pub fn sepia_veil(sc: &mut Scene, w: f64, h: f64, t: f32, alpha: f32) {
    let g = Gradient::new_linear((0.0, 0.0), (w * 0.3, h)).with_stops([ca(Color::from_rgba8(150, 76, 30, 255), 0.99 * alpha), ca(Color::from_rgba8(176, 98, 42, 255), 0.985 * alpha), ca(Color::from_rgba8(96, 44, 18, 255), 0.99 * alpha)]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &Rect::new(0.0, 0.0, w, h));
    let glow = Rect::new(w * 0.25, -h * 0.15, w * 0.75, h * 0.45);
    sc.draw_blurred_rounded_rect(Affine::IDENTITY, glow, ca(Color::from_rgba8(240, 170, 80, 255), 0.35 * alpha), h * 0.3, h * 0.12);
    for i in 0..18u32 {
        let x = hash(i * 3 + 1) as f64 * w;
        let y = hash(i * 3 + 2) as f64 * h;
        let r = (0.06 + hash(i * 3 + 3) as f64 * 0.2) * h;
        let drift = (t * 0.05 + i as f32).sin() as f64 * 0.006 * w;
        let ink = if i % 3 == 0 { Color::from_rgba8(226, 150, 70, 255) } else { Color::from_rgba8(58, 24, 8, 255) };
        sc.draw_blurred_rounded_rect(Affine::IDENTITY, Rect::new(x - r * 1.4 + drift, y - r * 0.5, x + r * 1.4 + drift, y + r * 0.5), ca(ink, 0.34 * alpha), r * 0.5, r * 0.25);
    }
    let step = (h / 240.0).max(2.0);
    let mut x = 0.0;
    let mut k = 5u32;
    while x < w {
        k = k.wrapping_add(17);
        sc.fill(Fill::NonZero, Affine::IDENTITY, ca(Color::from_rgba8(40, 16, 4, 255), (0.04 + hash(k) * 0.05) * alpha), None, &Rect::new(x, 0.0, x + step * 0.5, h));
        x += step;
    }
}
