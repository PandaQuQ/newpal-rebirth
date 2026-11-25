//! In-game pause menu (Esc / Start): the world freezes behind a soft ink veil; same visual language as the title.
use super::{hints, Anims, ConfirmScreen};
use crate::anim::{approach, ease_out_cubic};
use crate::input::{Action, Input};
use crate::scene::{Cmd, Ctx, DrawCx, Nav, Screen};
use crate::text::{self, Style};
use crate::theme::{self, a, GOLD, INK, JADE, JADE_L, PAPER, PAPER_DIM};
use vello::kurbo::{Affine, Point, Rect};
use vello::peniko::{Fill, Gradient};
use vello::Scene;

const ITEMS: [&str; 5] = ["pause.resume", "menu.settings", "menu.load", "pause.to_title", "pause.quit"];

pub struct PauseScreen {
    focus: usize,
    anims: Anims,
    hl: f32,
    t: f32,
}

impl Default for PauseScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl PauseScreen {
    pub fn new() -> Self {
        Self { focus: 0, anims: Anims::default(), hl: -1.0, t: 0.0 }
    }
    fn item_y(i: usize, u: f32, h: f32) -> f32 {
        h / 2.0 - 120.0 * u + i as f32 * 62.0 * u
    }
    fn item_rect(i: usize, u: f32, h: f32) -> Rect {
        let y = Self::item_y(i, u, h);
        Rect::new((100.0 * u) as f64, (y - 27.0 * u) as f64, (560.0 * u) as f64, (y + 27.0 * u) as f64)
    }
    fn activate(&self, cx: &mut Ctx) -> Nav {
        match ITEMS[self.focus] {
            "pause.resume" => {
                cx.cmds.push(Cmd::Resume);
                Nav::Pop
            }
            "menu.settings" => Nav::Push(Box::new(super::SettingsScreen::new())),
            "menu.load" => Nav::Push(Box::new(super::LoadScreen::new())),
            "pause.to_title" => Nav::Push(Box::new(ConfirmScreen::new("confirm.to_title", Cmd::ToTitle))),
            _ => Nav::Push(Box::new(ConfirmScreen::new("confirm.quit", Cmd::Quit))),
        }
    }
}

impl Screen for PauseScreen {
    fn name(&self) -> &'static str {
        "pause"
    }
    fn update(&mut self, cx: &mut Ctx, inp: &Input, dt: f32) -> Nav {
        self.t += dt;
        let n = ITEMS.len();
        let mut nav = Nav::Stay;
        for act in &inp.pressed {
            match act {
                Action::Up => self.focus = (self.focus + n - 1) % n,
                Action::Down => self.focus = (self.focus + 1) % n,
                Action::Back | Action::Menu => {
                    cx.cmds.push(Cmd::Resume);
                    return Nav::Pop;
                }
                Action::Confirm => nav = self.activate(cx),
                _ => {}
            }
        }
        for i in 0..n {
            if super::hit(Self::item_rect(i, cx.u, cx.h), inp.mouse) {
                if inp.mouse_moved {
                    self.focus = i;
                }
                if inp.click {
                    self.focus = i;
                    nav = self.activate(cx);
                }
            }
        }
        self.anims.tick(n, Some(self.focus), dt);
        let ty = Self::item_y(self.focus, cx.u, cx.h);
        if self.hl < 0.0 {
            self.hl = ty;
        }
        approach(&mut self.hl, ty, 16.0, dt);
        nav
    }
    fn draw(&self, sc: &mut Scene, cx: &DrawCx) {
        let (w, h, u) = (cx.w, cx.h, cx.u);
        // veil: dark on the left, lighter on the right so the frozen scene stays readable
        let g = Gradient::new_linear((0.0, 0.0), (w as f64, 0.0)).with_stops([a(INK, 0.92), a(INK, 0.7), a(INK, 0.35)]);
        sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &cx.full());
        theme::mist(sc, w, h, cx.time, 0.8);
        let x = 120.0 * u;
        let ty = h / 2.0 - 230.0 * u;
        text::draw(sc, cx.fonts, &Style::new(54.0 * u, GOLD).title().spacing(8.0 * u), &cx.s.t("pause.title"), x, ty);
        theme::diamond(sc, Point::new(x as f64, (ty + 34.0 * u) as f64), 5.0 * u, GOLD);
        let rg = Gradient::new_linear((x as f64, 0.0), ((x + 420.0 * u) as f64, 0.0)).with_stops([a(GOLD, 0.8), a(GOLD, 0.0)]);
        sc.fill(Fill::NonZero, Affine::IDENTITY, &rg, None, &Rect::new((x + 14.0 * u) as f64, (ty + 33.4 * u) as f64, (x + 420.0 * u) as f64, (ty + 34.6 * u) as f64));
        let hf = self.anims.get(self.focus);
        for (i, key) in ITEMS.iter().enumerate() {
            let k = ease_out_cubic(((self.t - i as f32 * 0.04) / 0.3).clamp(0.0, 1.0));
            let y = Self::item_y(i, u, h);
            let f = self.anims.get(i);
            let label = cx.s.t(key);
            let st = Style::new(32.0 * u, super::lerp_c(PAPER_DIM, PAPER, f)).spacing(2.0 * u).alpha(k);
            let tw = text::measure(cx.fonts, &st, &label);
            if i == self.focus {
                theme::brush_stroke(sc, x - 30.0 * u, self.hl, tw + 110.0 * u, 38.0 * u, hf, a(JADE, 0.55), a(JADE_L, 0.18));
            }
            text::draw_mid(sc, cx.fonts, &st, &label, x + 18.0 * u + f * 14.0 * u, y);
        }
        hints(sc, cx, &[(Action::Up, cx.s.t("hint.move")), (Action::Confirm, cx.s.t("hint.select")), (Action::Back, cx.s.t("hint.back"))], 1.0);
    }
}
