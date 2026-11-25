//! Mods: list of discovered mod folders with enable toggles (priority order = load order).
use super::{dim, hints, rect, short_path, Anims};
use crate::anim::ease_out_cubic;
use crate::host::ModEntry;
use crate::input::{Action, Input};
use crate::scene::{Cmd, Ctx, DrawCx, Nav, Screen};
use crate::text::{self, Align, Style};
use crate::theme::{self, a, GOLD, INK_2, JADE, JADE_D, JADE_L, MUTED, PAPER, PAPER_DIM};
use vello::kurbo::{Affine, Circle, Point, Rect, RoundedRect, Stroke};
use vello::peniko::Fill;
use vello::Scene;

pub struct ModsScreen {
    focus: usize,
    anims: Anims,
    mods: Vec<ModEntry>,
    dir: String,
    queried: bool,
    t: f32,
}

impl Default for ModsScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl ModsScreen {
    pub fn new() -> Self {
        Self { focus: 0, anims: Anims::default(), mods: Vec::new(), dir: String::new(), queried: false, t: 0.0 }
    }
    fn panel(w: f32, h: f32, u: f32) -> Rect {
        let pw = (1200.0 * u).min(w - 60.0 * u);
        let ph = (800.0 * u).min(h - 60.0 * u);
        rect((w - pw) / 2.0, (h - ph) / 2.0, pw, ph)
    }
    fn row_rect(i: usize, p: Rect, u: f32) -> Rect {
        rect(p.x0 as f32 + 60.0 * u, p.y0 as f32 + 140.0 * u + i as f32 * 92.0 * u, p.width() as f32 - 120.0 * u, 80.0 * u)
    }
}

impl Screen for ModsScreen {
    fn name(&self) -> &'static str {
        "mods"
    }
    fn update(&mut self, cx: &mut Ctx, inp: &Input, dt: f32) -> Nav {
        self.t += dt;
        if !self.queried {
            self.queried = true;
            self.mods = cx.host.mods();
            self.dir = cx.host.mods_dir().map(|d| d.display().to_string()).unwrap_or_default();
        }
        let p = Self::panel(cx.w, cx.h, cx.u);
        let n = self.mods.len();
        let mut toggle = None;
        for act in &inp.pressed {
            match act {
                Action::Back => return Nav::Pop,
                Action::Up if self.focus > 0 => self.focus -= 1,
                Action::Down if self.focus + 1 < n => self.focus += 1,
                Action::Confirm | Action::Left | Action::Right => toggle = Some(self.focus),
                _ => {}
            }
        }
        for i in 0..n {
            if super::hit(Self::row_rect(i, p, cx.u), inp.mouse) {
                if inp.mouse_moved {
                    self.focus = i;
                }
                if inp.click {
                    toggle = Some(i);
                }
            }
        }
        if let Some(i) = toggle {
            if let Some(m) = self.mods.get_mut(i) {
                m.enabled = !m.enabled;
                cx.cmds.push(Cmd::ToggleMod(m.name.clone()));
            }
        }
        self.anims.tick(n, Some(self.focus), dt);
        Nav::Stay
    }
    fn draw(&self, sc: &mut Scene, cx: &DrawCx) {
        let u = cx.u;
        dim(sc, cx, 0.55);
        let p = Self::panel(cx.w, cx.h, u);
        theme::panel(sc, p, u, 1.0);
        super::header(sc, cx.fonts, &cx.s.t("mods.title"), p.x0 as f32 + 60.0 * u, p.y0 as f32 + 70.0 * u, p.width() as f32 - 120.0 * u, u, 1.0);
        if self.mods.is_empty() {
            let c = Point::new(p.center().x, p.center().y - (60.0 * u) as f64);
            // empty state: a small vector "scroll"
            let sr = Rect::new(c.x - 60.0 * u as f64, c.y - 40.0 * u as f64, c.x + 60.0 * u as f64, c.y + 40.0 * u as f64);
            sc.stroke(&Stroke::new((2.0 * u) as f64), Affine::IDENTITY, a(JADE, 0.7), None, &RoundedRect::from_rect(sr, (6.0 * u) as f64));
            for j in 0..3 {
                let y = c.y - 14.0 * u as f64 + j as f64 * 14.0 * u as f64;
                theme::fill_rect(sc, Rect::new(c.x - 36.0 * u as f64, y, c.x + (36.0 - j as f64 * 14.0) * u as f64, y + 2.0 * u as f64), a(PAPER_DIM, 0.6));
            }
            text::draw(sc, cx.fonts, &Style::new(28.0 * u, PAPER).align(Align::Center), &cx.s.t("mods.empty"), c.x as f32, c.y as f32 + 100.0 * u);
            let st = Style::new(19.0 * u, PAPER_DIM).align(Align::Center);
            let d = short_path(cx.fonts, &st, &self.dir, p.width() as f32 - 300.0 * u);
            text::draw(sc, cx.fonts, &st, &cx.s.f("mods.where", &[("dir", d)]), c.x as f32, c.y as f32 + 142.0 * u);
        }
        for (i, m) in self.mods.iter().enumerate() {
            let k = ease_out_cubic(((self.t - i as f32 * 0.04) / 0.35).clamp(0.0, 1.0));
            let r = Self::row_rect(i, p, u);
            let f = self.anims.get(i);
            let rr = RoundedRect::from_rect(r, (10.0 * u) as f64);
            sc.fill(Fill::NonZero, Affine::IDENTITY, a(INK_2, (0.6 + 0.25 * f) * k), None, &rr);
            sc.stroke(&Stroke::new(((1.0 + f) * u) as f64), Affine::IDENTITY, a(super::lerp_c(MUTED, JADE_L, f), (0.4 + 0.6 * f) * k), None, &rr);
            let cy = r.center().y as f32;
            text::draw(sc, cx.fonts, &Style::new(25.0 * u, PAPER).alpha(k), &m.name, r.x0 as f32 + 28.0 * u, cy - 4.0 * u);
            text::draw(sc, cx.fonts, &Style::new(16.0 * u, MUTED).alpha(k), &cx.s.f("mods.priority", &[("n", m.priority.to_string())]), r.x0 as f32 + 28.0 * u, cy + 24.0 * u);
            let tw = 64.0 * u;
            let th = 32.0 * u;
            let tr = rect(r.x1 as f32 - 28.0 * u - tw, cy - th / 2.0, tw, th);
            let trr = RoundedRect::from_rect(tr, (th / 2.0) as f64);
            sc.fill(Fill::NonZero, Affine::IDENTITY, a(if m.enabled { JADE_D } else { INK_2 }, k), None, &trr);
            sc.stroke(&Stroke::new((1.4 * u) as f64), Affine::IDENTITY, a(if m.enabled { JADE_L } else { MUTED }, k), None, &trr);
            let kx = if m.enabled { tr.x1 as f32 - th / 2.0 } else { tr.x0 as f32 + th / 2.0 };
            sc.fill(Fill::NonZero, Affine::IDENTITY, a(PAPER, k), None, &Circle::new((kx as f64, cy as f64), (th * 0.36) as f64));
            let l = cx.s.t(if m.enabled { "mods.enabled" } else { "mods.disabled" });
            text::draw_mid(sc, cx.fonts, &Style::new(19.0 * u, PAPER_DIM).align(Align::Right).alpha(k), &l, tr.x0 as f32 - 16.0 * u, cy);
        }
        theme::diamond(sc, Point::new(p.x0 + (66.0 * u) as f64, p.y1 - (60.0 * u) as f64), 5.0 * u, GOLD);
        text::draw_mid(sc, cx.fonts, &Style::new(18.0 * u, PAPER_DIM), &cx.s.t("mods.note"), p.x0 as f32 + 84.0 * u, p.y1 as f32 - 60.0 * u);
        hints(sc, cx, &[(Action::Confirm, cx.s.t("hint.select")), (Action::Back, cx.s.t("hint.back"))], 1.0);
    }
}
