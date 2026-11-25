//! Screens + shared widgets.
pub mod confirm;
pub mod launcher;
pub mod load;
pub mod mods;
pub mod pause;
pub mod settings;
pub mod sysmenu;
pub mod title;

pub use confirm::ConfirmScreen;
pub use launcher::LauncherScreen;
pub use load::LoadScreen;
pub use mods::ModsScreen;
pub use pause::PauseScreen;
pub use settings::SettingsScreen;
pub use sysmenu::SysMenuScreen;
pub use title::TitleScreen;

use crate::anim::{approach, lerp};
use crate::input::{key_label, Action, Device};
use crate::scene::DrawCx;
use crate::text::{self, Align, Fonts, Style};
use crate::theme::{self, a, GOLD, INK, JADE, JADE_D, JADE_L, MUTED, PAPER, PAPER_DIM};
use vello::kurbo::{Affine, Point, Rect, RoundedRect, Stroke};
use vello::peniko::{Fill, Gradient};
use vello::Scene;

pub fn hit(r: Rect, m: (f32, f32)) -> bool {
    r.contains(Point::new(m.0 as f64, m.1 as f64))
}

pub fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64)
}

/// Per-item hover / focus animation values.
#[derive(Default, Clone)]
pub struct Anims {
    pub v: Vec<f32>,
}

impl Anims {
    pub fn tick(&mut self, n: usize, focus: Option<usize>, dt: f32) {
        self.v.resize(n, 0.0);
        for (i, v) in self.v.iter_mut().enumerate() {
            approach(v, if Some(i) == focus { 1.0 } else { 0.0 }, 14.0, dt);
        }
    }
    pub fn get(&self, i: usize) -> f32 {
        self.v.get(i).copied().unwrap_or(0.0)
    }
}

/// A button: ink pill, jade rim, brush-light when focused.
#[derive(Clone)]
pub struct Btn {
    pub id: &'static str,
    pub label: String,
    pub primary: bool,
    pub enabled: bool,
}

impl Btn {
    pub fn new(id: &'static str, label: String) -> Self {
        Self { id, label, primary: false, enabled: true }
    }
    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }
    pub fn enabled(mut self, e: bool) -> Self {
        self.enabled = e;
        self
    }
}

pub fn btn_style(u: f32) -> Style {
    Style::new(24.0 * u, PAPER).spacing(1.0 * u)
}

/// Right-aligned row of buttons ending at `right`, vertically centred on `cy`.
pub fn layout_buttons(fonts: &Fonts, btns: &[Btn], right: f32, cy: f32, u: f32) -> Vec<Rect> {
    let st = btn_style(u);
    let h = 58.0 * u;
    let gap = 18.0 * u;
    let widths: Vec<f32> = btns.iter().map(|b| text::measure(fonts, &st, &b.label) + 64.0 * u).map(|w| w.max(170.0 * u)).collect();
    let total: f32 = widths.iter().sum::<f32>() + gap * (btns.len().saturating_sub(1)) as f32;
    let mut x = right - total;
    widths
        .iter()
        .map(|w| {
            let r = rect(x, cy - h / 2.0, *w, h);
            x += w + gap;
            r
        })
        .collect()
}

pub fn draw_button(sc: &mut Scene, fonts: &Fonts, r: Rect, b: &Btn, f: f32, u: f32, alpha: f32) {
    let rad = r.height() / 2.0;
    let rr = RoundedRect::from_rect(r, rad);
    let en = if b.enabled { 1.0 } else { 0.4 };
    if b.primary && b.enabled {
        let g = Gradient::new_linear((r.x0, r.y0), (r.x1, r.y1)).with_stops([a(JADE_D, 0.95 * alpha), a(theme::rgba(42, 120, 102, 1.0), 0.95 * alpha)]);
        sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &rr);
    } else {
        sc.fill(Fill::NonZero, Affine::IDENTITY, a(INK, 0.7 * alpha), None, &rr);
    }
    if f > 0.01 {
        sc.draw_blurred_rounded_rect(Affine::IDENTITY, r, a(JADE, 0.35 * f * alpha * en), rad, (14.0 * u) as f64);
    }
    let rim = a(lerp_c(JADE, JADE_L, f), (0.45 + 0.55 * f) * alpha * en);
    sc.stroke(&Stroke::new(((1.2 + 0.8 * f) * u) as f64), Affine::IDENTITY, rim, None, &rr);
    let st = btn_style(u).align(Align::Center);
    let col = if b.enabled { PAPER } else { MUTED };
    text::draw_mid(sc, fonts, &Style { color: a(col, alpha), ..st }, &b.label, r.center().x as f32, r.center().y as f32);
}

pub fn lerp_c(c0: vello::peniko::Color, c1: vello::peniko::Color, t: f32) -> vello::peniko::Color {
    let (x, y) = (c0.components, c1.components);
    vello::peniko::Color::new([lerp(x[0], y[0], t), lerp(x[1], y[1], t), lerp(x[2], y[2], t), lerp(x[3], y[3], t)])
}

/// Section header: diamond + title + fading gold rule.
pub fn header(sc: &mut Scene, fonts: &Fonts, title: &str, x: f32, y: f32, w: f32, u: f32, alpha: f32) {
    theme::diamond(sc, Point::new(x as f64, y as f64), 7.0 * u, a(GOLD, alpha));
    let tw = text::draw_mid(sc, fonts, &Style::new(40.0 * u, a(PAPER, alpha)).title().spacing(4.0 * u), title, x + 22.0 * u, y);
    let lx = x + 22.0 * u + tw + 24.0 * u;
    let line = Rect::new(lx as f64, (y - 0.6 * u) as f64, (x + w) as f64, (y + 0.6 * u) as f64);
    let g = Gradient::new_linear((lx as f64, 0.0), ((x + w) as f64, 0.0)).with_stops([a(GOLD, 0.6 * alpha), a(GOLD, 0.0)]);
    sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &line);
}

/// Bottom-right input hints (key caps for keyboard / mouse, face buttons for gamepads).
pub fn hints(sc: &mut Scene, cx: &DrawCx, items: &[(Action, String)], alpha: f32) {
    let u = cx.u;
    let cy = cx.h - 46.0 * u;
    let st = Style::new(18.0 * u, a(PAPER_DIM, alpha));
    // measure right-to-left
    let mut x = cx.w - 56.0 * u;
    for (act, label) in items.iter().rev() {
        let lw = text::measure(cx.fonts, &st, label);
        x -= lw;
        text::draw_mid(sc, cx.fonts, &st, label, x, cy);
        x -= 10.0 * u;
        let cap = match cx.device {
            Device::Gamepad => act.pad_label().to_string(),
            _ => cx.settings.bindings.keys.get(act).and_then(|v| v.first()).map(|k| key_label(*k)).unwrap_or_default(),
        };
        let cw = estimate_cap(cx, &cap);
        x -= cw;
        if cx.device == Device::Gamepad {
            theme::pad_button(sc, cx.fonts, x, cy, &cap, u, alpha);
        } else {
            theme::keycap(sc, cx.fonts, x, cy, &cap, u, alpha);
        }
        x -= 30.0 * u;
    }
}

fn estimate_cap(cx: &DrawCx, label: &str) -> f32 {
    let u = cx.u;
    if cx.device == Device::Gamepad {
        if label.chars().count() > 1 { text::measure(cx.fonts, &Style::new(13.0 * u, PAPER), label) + 18.0 * u } else { 26.0 * u }
    } else {
        (text::measure(cx.fonts, &Style::new(15.0 * u, PAPER), label) + 16.0 * u).max(26.0 * u)
    }
}

pub fn dim(sc: &mut Scene, cx: &DrawCx, k: f32) {
    theme::fill_rect(sc, cx.full(), a(INK, k));
}

pub fn fmt_bytes(b: u64) -> String {
    let gb = b as f64 / 1e9;
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", b as f64 / 1e6) }
}

pub fn fmt_secs(s: f32) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 { format!("{}h {:02}m", s / 3600, (s % 3600) / 60) } else if s >= 60 { format!("{}m {:02}s", s / 60, s % 60) } else { format!("{s}s") }
}

/// Middle-ellipsize a long path for display.
pub fn short_path(fonts: &Fonts, st: &Style, p: &str, max_w: f32) -> String {
    if text::measure(fonts, st, p) <= max_w {
        return p.to_string();
    }
    let chars: Vec<char> = p.chars().collect();
    let mut keep = chars.len();
    while keep > 8 {
        keep -= 1;
        let head: String = chars[..keep / 3].iter().collect();
        let tail: String = chars[chars.len() - (keep - keep / 3)..].iter().collect();
        let s = format!("{head}…{tail}");
        if text::measure(fonts, st, &s) <= max_w {
            return s;
        }
    }
    p.to_string()
}
