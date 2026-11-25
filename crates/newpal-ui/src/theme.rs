//! Visual language: ink wash (水墨) + jade (玉) + gold leaf, drawn procedurally as vector paths.
//! Nothing here is a bitmap; every helper takes `u` = layout unit (1.0 at 1920x1080) so it scales cleanly.
use crate::anim::hash;
use crate::text::{self, Align, Fonts, Style};
use vello::kurbo::{Affine, BezPath, Circle, Point, Rect, RoundedRect, Stroke};
use vello::peniko::{Color, Fill, Gradient, Mix};
use vello::Scene;

pub fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::from_rgba8(r, g, b, (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

pub const INK: Color = Color::from_rgba8(12, 17, 20, 255);
pub const INK_2: Color = Color::from_rgba8(20, 29, 32, 255);
pub const JADE: Color = Color::from_rgba8(86, 186, 158, 255);
pub const JADE_L: Color = Color::from_rgba8(165, 222, 204, 255);
pub const JADE_D: Color = Color::from_rgba8(31, 94, 82, 255);
pub const GOLD: Color = Color::from_rgba8(226, 192, 122, 255);
pub const GOLD_D: Color = Color::from_rgba8(160, 126, 70, 255);
pub const PAPER: Color = Color::from_rgba8(239, 232, 216, 255);
pub const PAPER_DIM: Color = Color::from_rgba8(178, 176, 164, 255);
pub const MUTED: Color = Color::from_rgba8(118, 132, 128, 255);
pub const SEAL: Color = Color::from_rgba8(181, 69, 47, 255);
pub const OK: Color = Color::from_rgba8(108, 199, 157, 255);
pub const WARN: Color = Color::from_rgba8(226, 172, 84, 255);
pub const BAD: Color = Color::from_rgba8(217, 100, 74, 255);

pub fn a(c: Color, alpha: f32) -> Color {
    c.multiply_alpha(alpha.clamp(0.0, 1.0))
}

pub fn fill_rect(sc: &mut Scene, r: Rect, c: Color) {
    sc.fill(Fill::NonZero, Affine::IDENTITY, c, None, &r);
}

/// Begin a group drawn at `alpha` (everything until [end] fades together).
pub fn begin(sc: &mut Scene, alpha: f32, clip: Rect) {
    sc.push_layer(Fill::NonZero, Mix::Normal, alpha.clamp(0.0, 1.0), Affine::IDENTITY, &clip);
}
pub fn end(sc: &mut Scene) {
    sc.pop_layer();
}

/// Full-screen ink background (used before the player's own map is available).
pub fn ink_backdrop(sc: &mut Scene, w: f32, h: f32, t: f32) {
    let full = Rect::new(0.0, 0.0, w as f64, h as f64);
    let g = Gradient::new_radial((w as f64 * 0.68, h as f64 * 0.32), (w.max(h) * 0.9) as f32)
        .with_stops([rgba(36, 56, 58, 1.0), rgba(16, 24, 27, 1.0), rgba(8, 11, 13, 1.0)]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &full);
    mist(sc, w, h, t, 0.55);
}

/// Darken the left side + bottom so menus stay legible over any background image.
pub fn legibility_scrim(sc: &mut Scene, w: f32, h: f32, strength: f32) {
    let full = Rect::new(0.0, 0.0, w as f64, h as f64);
    let gl = Gradient::new_linear((0.0, 0.0), (w as f64 * 0.62, 0.0)).with_stops([
        a(INK, 0.88 * strength),
        a(INK, 0.55 * strength),
        a(INK, 0.0),
    ]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &gl, None, &full);
    let gb = Gradient::new_linear((0.0, h as f64), (0.0, h as f64 * 0.55)).with_stops([a(INK, 0.85 * strength), a(INK, 0.0)]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &gb, None, &full);
    let gt = Gradient::new_linear((0.0, 0.0), (0.0, h as f64 * 0.18)).with_stops([a(INK, 0.45 * strength), a(INK, 0.0)]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &gt, None, &full);
}

/// A ridge line of ink mountains: smooth random peaks (deterministic per `seed`).
fn ridge(w: f32, base: f32, amp: f32, seed: u32, shift: f32) -> BezPath {
    let mut p = BezPath::new();
    let n = 9;
    let step = w * 1.3 / n as f32;
    let x0 = -w * 0.15 + shift;
    let pts: Vec<(f32, f32)> = (0..=n)
        .map(|i| {
            let hgt = amp * (0.35 + 0.65 * hash(seed * 31 + i as u32));
            (x0 + i as f32 * step, base - hgt)
        })
        .collect();
    p.move_to((x0 as f64, (base + amp * 3.0) as f64));
    p.line_to((pts[0].0 as f64, pts[0].1 as f64));
    for i in 0..n {
        let (xa, ya) = pts[i];
        let (xb, yb) = pts[i + 1];
        let xm = (xa + xb) / 2.0;
        // asymmetric "brush" peaks: steep on one side, long slope on the other
        let peak = hash(seed * 77 + i as u32) > 0.5;
        let (c1, c2) = if peak { ((xa + step * 0.15, ya - amp * 0.25), (xm - step * 0.1, yb.min(ya) - amp * 0.35)) } else { ((xm, ya + amp * 0.1), (xb - step * 0.3, yb + amp * 0.05)) };
        p.curve_to((c1.0 as f64, c1.1 as f64), (c2.0 as f64, c2.1 as f64), (xb as f64, yb as f64));
    }
    let xe = pts[n].0;
    p.line_to((xe as f64, (base + amp * 3.0) as f64));
    p.close_path();
    p
}

/// Layered ink mountains with mouse parallax (`par` in [-1,1]^2) - far layers move less.
pub fn mountains(sc: &mut Scene, w: f32, h: f32, t: f32, par: (f32, f32), alpha: f32) {
    let layers = [(0.70, 0.16, 3u32, 0.10, rgba(70, 104, 104, 0.30)), (0.80, 0.14, 7, 0.25, rgba(34, 56, 58, 0.55)), (0.92, 0.12, 11, 0.5, rgba(12, 18, 20, 0.92))];
    for (i, (base, amp, seed, depth, col)) in layers.iter().enumerate() {
        let drift = (t * 4.0 * (i as f32 + 1.0)).sin() * 2.0;
        let shift = -par.0 * 40.0 * depth + drift;
        let dy = -par.1 * 18.0 * depth;
        let path = ridge(w, h * base + dy, h * amp, *seed, shift);
        let top = h * (base - amp * 1.4);
        let g = Gradient::new_linear((0.0, top as f64), (0.0, h as f64)).with_stops([a(*col, alpha), a(*col, alpha * 0.6), a(*col, alpha)]);
        sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &path);
    }
}

/// Drifting mist bands (blurred rounded rects).
pub fn mist(sc: &mut Scene, w: f32, h: f32, t: f32, alpha: f32) {
    for i in 0..5u32 {
        let speed = 6.0 + 10.0 * hash(i * 13 + 1);
        let span = w * (0.5 + 0.5 * hash(i * 7 + 2));
        let x = ((hash(i * 3 + 5) * w * 2.0 + t * speed) % (w + span * 2.0)) - span;
        let y = h * (0.45 + 0.4 * hash(i * 11 + 3));
        let hh = h * (0.04 + 0.05 * hash(i * 17 + 4));
        let r = Rect::new(x as f64, y as f64, (x + span) as f64, (y + hh) as f64);
        sc.draw_blurred_rounded_rect(Affine::IDENTITY, r, a(JADE_L, 0.06 * alpha), (hh / 2.0) as f64, (hh * 0.9) as f64);
    }
}

/// Moon with halo.
pub fn moon(sc: &mut Scene, c: Point, r: f32, alpha: f32) {
    let rr = r as f64;
    let halo = Rect::new(c.x - rr, c.y - rr, c.x + rr, c.y + rr);
    sc.draw_blurred_rounded_rect(Affine::IDENTITY, halo, a(rgba(236, 228, 200, 1.0), 0.28 * alpha), rr, rr * 0.9);
    let g = Gradient::new_radial((c.x - rr * 0.25, c.y - rr * 0.25), r * 1.2).with_stops([a(rgba(250, 244, 222, 1.0), 0.95 * alpha), a(rgba(214, 204, 172, 1.0), 0.9 * alpha)]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &Circle::new(c, rr));
}

/// Fireflies / spirit lights drifting upwards.
pub fn spirits(sc: &mut Scene, w: f32, h: f32, t: f32, u: f32, alpha: f32) {
    for i in 0..28u32 {
        let sp = 12.0 + 26.0 * hash(i * 5 + 9);
        let x0 = hash(i * 19 + 1) * w;
        let y = h - ((hash(i * 23 + 2) * h + t * sp * u) % (h * 1.1));
        let x = x0 + (t * 0.6 + i as f32).sin() * 18.0 * u;
        let tw = 0.5 + 0.5 * (t * (1.2 + hash(i) * 2.0) + i as f32).sin();
        let r = (1.4 + 2.2 * hash(i * 29 + 3)) * u;
        let fade = (y / h).clamp(0.0, 1.0);
        let col = if i % 3 == 0 { GOLD } else { JADE_L };
        let rc = Rect::new((x - r * 3.0) as f64, (y - r * 3.0) as f64, (x + r * 3.0) as f64, (y + r * 3.0) as f64);
        sc.draw_blurred_rounded_rect(Affine::IDENTITY, rc, a(col, 0.5 * tw * fade * alpha), (r * 3.0) as f64, (r * 1.6) as f64);
        sc.fill(Fill::NonZero, Affine::IDENTITY, a(col, 0.9 * tw * fade * alpha), None, &Circle::new((x as f64, y as f64), (r * 0.6) as f64));
    }
}

/// Frosted ink panel with soft shadow, jade hairline and gold corner ornaments.
pub fn panel(sc: &mut Scene, r: Rect, u: f32, alpha: f32) {
    let rad = (14.0 * u) as f64;
    sc.draw_blurred_rounded_rect(Affine::translate((0.0, 10.0 * u as f64)), r, a(Color::BLACK, 0.55 * alpha), rad, (28.0 * u) as f64);
    let rr = RoundedRect::from_rect(r, rad);
    let g = Gradient::new_linear((r.x0, r.y0), (r.x1, r.y1)).with_stops([a(rgba(24, 36, 38, 1.0), 0.93 * alpha), a(rgba(12, 18, 21, 1.0), 0.95 * alpha)]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &rr);
    sc.stroke(&Stroke::new((1.2 * u) as f64), Affine::IDENTITY, a(JADE, 0.35 * alpha), None, &rr);
    let inner = RoundedRect::from_rect(r.inset(-(6.0 * u) as f64), rad * 0.6);
    sc.stroke(&Stroke::new((0.8 * u) as f64), Affine::IDENTITY, a(GOLD, 0.12 * alpha), None, &inner);
    let s = 26.0 * u;
    for (p, dx, dy) in [(Point::new(r.x0, r.y0), 1.0, 1.0), (Point::new(r.x1, r.y0), -1.0, 1.0), (Point::new(r.x0, r.y1), 1.0, -1.0), (Point::new(r.x1, r.y1), -1.0, -1.0)] {
        corner(sc, p, dx, dy, s, u, a(GOLD, 0.75 * alpha));
    }
}

/// 回纹-style corner bracket.
pub fn corner(sc: &mut Scene, p: Point, dx: f32, dy: f32, s: f32, u: f32, c: Color) {
    let o = 10.0 * u;
    let (x, y) = (p.x as f32 + dx * o, p.y as f32 + dy * o);
    let mut b = BezPath::new();
    b.move_to(((x + dx * s) as f64, y as f64));
    b.line_to((x as f64, y as f64));
    b.line_to((x as f64, (y + dy * s) as f64));
    b.move_to(((x + dx * s * 0.45) as f64, (y + dy * s * 0.22) as f64));
    b.line_to(((x + dx * s * 0.22) as f64, (y + dy * s * 0.22) as f64));
    b.line_to(((x + dx * s * 0.22) as f64, (y + dy * s * 0.45) as f64));
    sc.stroke(&Stroke::new((1.6 * u) as f64), Affine::IDENTITY, c, None, &b);
}

/// Horizontal brush stroke (selection highlight): tapered ends, slight wobble, `grow` 0..1 = draw-on.
pub fn brush_stroke(sc: &mut Scene, x: f32, y: f32, w: f32, th: f32, grow: f32, c0: Color, c1: Color) {
    if grow <= 0.001 {
        return;
    }
    let w = w * grow;
    let mut p = BezPath::new();
    let (x, y, w, th) = (x as f64, y as f64, w as f64, th as f64);
    p.move_to((x, y + th * 0.1));
    p.curve_to((x + w * 0.2, y - th * 0.55), (x + w * 0.7, y - th * 0.45), (x + w, y - th * 0.12));
    p.curve_to((x + w + th * 0.3, y), (x + w + th * 0.1, y + th * 0.3), (x + w * 0.96, y + th * 0.42));
    p.curve_to((x + w * 0.6, y + th * 0.55), (x + w * 0.25, y + th * 0.5), (x - th * 0.1, y + th * 0.35));
    p.close_path();
    let g = Gradient::new_linear((x, y), (x + w, y)).with_stops([c0, c1, a(c1, 0.0)]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &p);
}

pub fn diamond(sc: &mut Scene, c: Point, r: f32, col: Color) {
    let r = r as f64;
    let mut p = BezPath::new();
    p.move_to((c.x, c.y - r));
    p.line_to((c.x + r, c.y));
    p.line_to((c.x, c.y + r));
    p.line_to((c.x - r, c.y));
    p.close_path();
    sc.fill(Fill::NonZero, Affine::IDENTITY, col, None, &p);
}

/// Red seal stamp (印章) with text, slightly rotated, rough inner border.
pub fn seal(sc: &mut Scene, fonts: &Fonts, c: Point, size: f32, label: &str, alpha: f32) {
    let s = size as f64;
    let xf = Affine::translate((c.x, c.y)) * Affine::rotate(-0.06);
    let rr = RoundedRect::new(-s / 2.0, -s / 2.0, s / 2.0, s / 2.0, s * 0.08);
    sc.fill(Fill::NonZero, xf, a(SEAL, 0.92 * alpha), None, &rr);
    let inner = RoundedRect::new(-s * 0.42, -s * 0.42, s * 0.42, s * 0.42, s * 0.04);
    sc.stroke(&Stroke::new(s * 0.03), xf, a(PAPER, 0.85 * alpha), None, &inner);
    let n = label.chars().count().max(1);
    let st = Style::new(size * if n <= 2 { 0.36 } else { 0.26 }, a(PAPER, 0.95 * alpha)).title().align(Align::Center);
    // vertical stack, right-to-left columns like a real seal
    let chars: Vec<char> = label.chars().collect();
    let cols = if n > 2 { 2 } else { 1 };
    let per = n.div_ceil(cols);
    for (i, ch) in chars.iter().enumerate() {
        let col = i / per;
        let row = i % per;
        let cx = if cols == 2 { if col == 0 { s * 0.18 } else { -s * 0.18 } } else { 0.0 };
        let step = s * 0.72 / per as f64;
        let cy = -s * 0.36 + step * (row as f64 + 0.5);
        let line = text::shape(fonts, &st, &ch.to_string());
        let base = cy + ((line.ascent + line.descent) / 2.0) as f64;
        text::paint(sc, fonts, &st, &line, cx as f32, base as f32, &vello::peniko::Brush::Solid(st.color), xf);
    }
}

/// Circular progress ring.
pub fn ring(sc: &mut Scene, c: Point, r: f32, th: f32, frac: f32, t: f32, alpha: f32) {
    let rr = r as f64;
    sc.stroke(&Stroke::new(th as f64), Affine::IDENTITY, a(JADE_D, 0.6 * alpha), None, &Circle::new(c, rr));
    let f = frac.clamp(0.0, 1.0) as f64;
    if f > 0.0 {
        let arc = vello::kurbo::Arc::new(c, (rr, rr), -std::f64::consts::FRAC_PI_2, f * std::f64::consts::TAU, 0.0);
        let g = Gradient::new_sweep(c, 0.0, std::f32::consts::TAU).with_stops([JADE, JADE_L, GOLD, JADE]);
        sc.stroke(&Stroke::new(th as f64).with_caps(vello::kurbo::Cap::Round), Affine::IDENTITY, &g, None, &arc);
    }
    // orbiting spark
    let ang = t as f64 * 2.2;
    let sp = Point::new(c.x + rr * ang.cos(), c.y + rr * ang.sin());
    let rc = Rect::new(sp.x - th as f64 * 2.0, sp.y - th as f64 * 2.0, sp.x + th as f64 * 2.0, sp.y + th as f64 * 2.0);
    sc.draw_blurred_rounded_rect(Affine::IDENTITY, rc, a(JADE_L, 0.5 * alpha), th as f64 * 2.0, th as f64);
}

/// Thin progress bar with shimmer.
pub fn bar(sc: &mut Scene, r: Rect, frac: f32, t: f32, u: f32, alpha: f32) {
    let h = r.height();
    sc.fill(Fill::NonZero, Affine::IDENTITY, a(JADE_D, 0.45 * alpha), None, &RoundedRect::from_rect(r, h / 2.0));
    let fw = r.width() * frac.clamp(0.0, 1.0) as f64;
    if fw > 0.5 {
        let fr = Rect::new(r.x0, r.y0, r.x0 + fw, r.y1);
        let g = Gradient::new_linear((r.x0, 0.0), (r.x0 + fw, 0.0)).with_stops([JADE_D, JADE, JADE_L]);
        sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &RoundedRect::from_rect(fr, h / 2.0));
        let sx = r.x0 + ((t as f64 * 0.6).fract()) * fw;
        let sh = Rect::new(sx - 30.0 * u as f64, r.y0, sx + 30.0 * u as f64, r.y1);
        sc.push_layer(Fill::NonZero, Mix::Normal, 1.0, Affine::IDENTITY, &RoundedRect::from_rect(fr, h / 2.0));
        sc.draw_blurred_rounded_rect(Affine::IDENTITY, sh, a(Color::WHITE, 0.35 * alpha), h / 2.0, 12.0 * u as f64);
        sc.pop_layer();
    }
}

// ---- icons (vector) ----

pub fn icon_check(sc: &mut Scene, c: Point, s: f32, col: Color) {
    let s = s as f64;
    sc.fill(Fill::NonZero, Affine::IDENTITY, a(col, 0.18), None, &Circle::new(c, s));
    let mut p = BezPath::new();
    p.move_to((c.x - s * 0.45, c.y + s * 0.02));
    p.line_to((c.x - s * 0.1, c.y + s * 0.36));
    p.line_to((c.x + s * 0.5, c.y - s * 0.32));
    sc.stroke(&Stroke::new(s * 0.18).with_caps(vello::kurbo::Cap::Round).with_join(vello::kurbo::Join::Round), Affine::IDENTITY, col, None, &p);
}

pub fn icon_cross(sc: &mut Scene, c: Point, s: f32, col: Color) {
    let s = s as f64;
    sc.fill(Fill::NonZero, Affine::IDENTITY, a(col, 0.18), None, &Circle::new(c, s));
    let mut p = BezPath::new();
    p.move_to((c.x - s * 0.35, c.y - s * 0.35));
    p.line_to((c.x + s * 0.35, c.y + s * 0.35));
    p.move_to((c.x + s * 0.35, c.y - s * 0.35));
    p.line_to((c.x - s * 0.35, c.y + s * 0.35));
    sc.stroke(&Stroke::new(s * 0.18).with_caps(vello::kurbo::Cap::Round), Affine::IDENTITY, col, None, &p);
}

pub fn icon_warn(sc: &mut Scene, c: Point, s: f32, col: Color) {
    let s = s as f64;
    let mut tri = BezPath::new();
    tri.move_to((c.x, c.y - s * 0.9));
    tri.line_to((c.x + s, c.y + s * 0.75));
    tri.line_to((c.x - s, c.y + s * 0.75));
    tri.close_path();
    sc.stroke(&Stroke::new(s * 0.14).with_join(vello::kurbo::Join::Round), Affine::IDENTITY, col, None, &tri);
    let mut l = BezPath::new();
    l.move_to((c.x, c.y - s * 0.3));
    l.line_to((c.x, c.y + s * 0.2));
    sc.stroke(&Stroke::new(s * 0.16).with_caps(vello::kurbo::Cap::Round), Affine::IDENTITY, col, None, &l);
    sc.fill(Fill::NonZero, Affine::IDENTITY, col, None, &Circle::new((c.x, c.y + s * 0.48), s * 0.09));
}

pub fn icon_folder(sc: &mut Scene, c: Point, s: f32, col: Color) {
    let s = s as f64;
    let mut p = BezPath::new();
    p.move_to((c.x - s, c.y - s * 0.55));
    p.line_to((c.x - s * 0.35, c.y - s * 0.55));
    p.line_to((c.x - s * 0.2, c.y - s * 0.35));
    p.line_to((c.x + s, c.y - s * 0.35));
    p.line_to((c.x + s, c.y + s * 0.65));
    p.line_to((c.x - s, c.y + s * 0.65));
    p.close_path();
    sc.stroke(&Stroke::new(s * 0.12).with_join(vello::kurbo::Join::Round), Affine::IDENTITY, col, None, &p);
}

/// Keyboard key cap with a label (e.g. "Enter", "Esc", "W").
pub fn keycap(sc: &mut Scene, fonts: &Fonts, x: f32, cy: f32, label: &str, u: f32, alpha: f32) -> f32 {
    let st = Style::new(15.0 * u, a(PAPER, alpha));
    let tw = text::measure(fonts, &st, label);
    let w = (tw + 16.0 * u).max(26.0 * u);
    let h = 26.0 * u;
    let r = RoundedRect::new(x as f64, (cy - h / 2.0) as f64, (x + w) as f64, (cy + h / 2.0) as f64, (5.0 * u) as f64);
    sc.fill(Fill::NonZero, Affine::IDENTITY, a(INK_2, 0.9 * alpha), None, &r);
    sc.stroke(&Stroke::new((1.2 * u) as f64), Affine::IDENTITY, a(PAPER_DIM, 0.6 * alpha), None, &r);
    text::draw_mid(sc, fonts, &st.align(Align::Center), label, x + w / 2.0, cy);
    w
}

/// Gamepad face button (Xbox layout colours, shape only).
pub fn pad_button(sc: &mut Scene, fonts: &Fonts, x: f32, cy: f32, label: &str, u: f32, alpha: f32) -> f32 {
    let r = 13.0 * u;
    let col = match label {
        "A" => rgba(96, 178, 86, 1.0),
        "B" => rgba(206, 78, 64, 1.0),
        "X" => rgba(70, 128, 214, 1.0),
        "Y" => rgba(222, 180, 60, 1.0),
        _ => PAPER_DIM,
    };
    let c = Point::new((x + r) as f64, cy as f64);
    if label.len() > 1 {
        let st = Style::new(13.0 * u, a(PAPER, alpha)).align(Align::Center);
        let w = text::measure(fonts, &st, label) + 18.0 * u;
        let rr = RoundedRect::new(x as f64, (cy - r) as f64, (x + w) as f64, (cy + r) as f64, r as f64);
        sc.stroke(&Stroke::new((1.4 * u) as f64), Affine::IDENTITY, a(PAPER_DIM, 0.8 * alpha), None, &rr);
        text::draw_mid(sc, fonts, &st, label, x + w / 2.0, cy);
        return w;
    }
    sc.fill(Fill::NonZero, Affine::IDENTITY, a(INK_2, 0.9 * alpha), None, &Circle::new(c, r as f64));
    sc.stroke(&Stroke::new((1.6 * u) as f64), Affine::IDENTITY, a(col, alpha), None, &Circle::new(c, r as f64));
    text::draw_mid(sc, fonts, &Style::new(14.0 * u, a(col, alpha)).align(Align::Center), label, x + r, cy);
    2.0 * r
}

/// Ink-wipe mask for scene transitions: an organic blot growing from `origin`, `t` 0..1 covers the screen.
pub fn ink_blot(w: f32, h: f32, origin: Point, t: f32) -> BezPath {
    let maxr = ((w * w + h * h) as f64).sqrt() * 1.05;
    let r = maxr * crate::anim::ease_in_out(t) as f64;
    let mut p = BezPath::new();
    let n = 48;
    for i in 0..=n {
        let ang = i as f64 / n as f64 * std::f64::consts::TAU;
        let k = (i % n) as u32;
        let wob = 1.0 + 0.10 * (hash(k * 7 + 1) as f64 - 0.5) + 0.06 * (ang * 5.0 + t as f64 * 4.0).sin();
        let rr = r * wob;
        let pt = (origin.x + rr * ang.cos(), origin.y + rr * ang.sin());
        if i == 0 { p.move_to(pt) } else { p.line_to(pt) }
    }
    p.close_path();
    p
}
