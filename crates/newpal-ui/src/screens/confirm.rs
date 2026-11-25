//! Yes / No dialog.
use super::{dim, draw_button, layout_buttons, rect, Anims, Btn};
use crate::input::{Action, Input};
use crate::scene::{Cmd, Ctx, DrawCx, Nav, Screen, Transition};
use crate::text::{self, Align, Style};
use crate::theme::{self, PAPER};
use vello::kurbo::Rect;
use vello::Scene;

pub struct ConfirmScreen {
    body: &'static str,
    cmd: Cmd,
    focus: usize,
    anims: Anims,
}

impl ConfirmScreen {
    pub fn new(body: &'static str, cmd: Cmd) -> Self {
        Self { body, cmd, focus: 0, anims: Anims::default() }
    }
    fn panel(w: f32, h: f32, u: f32) -> Rect {
        rect(w / 2.0 - 400.0 * u, h / 2.0 - 140.0 * u, 800.0 * u, 280.0 * u)
    }
    fn btns(&self, s: &crate::strings::Strings) -> Vec<Btn> {
        vec![Btn::new("no", s.t("confirm.no")), Btn::new("yes", s.t("confirm.yes")).primary()]
    }
}

impl Screen for ConfirmScreen {
    fn name(&self) -> &'static str {
        "confirm"
    }
    fn transition(&self) -> Transition {
        Transition::Fade
    }
    fn update(&mut self, cx: &mut Ctx, inp: &Input, dt: f32) -> Nav {
        let p = Self::panel(cx.w, cx.h, cx.u);
        let btns = self.btns(cx.s);
        let rects = layout_buttons(cx.fonts, &btns, p.x1 as f32 - 40.0 * cx.u, p.y1 as f32 - 58.0 * cx.u, cx.u);
        let mut pick = None;
        for act in &inp.pressed {
            match act {
                Action::Left => self.focus = 0,
                Action::Right => self.focus = 1,
                Action::Back => return Nav::Pop,
                Action::Confirm => pick = Some(self.focus),
                _ => {}
            }
        }
        for (i, r) in rects.iter().enumerate() {
            if super::hit(*r, inp.mouse) {
                if inp.mouse_moved {
                    self.focus = i;
                }
                if inp.click {
                    pick = Some(i);
                }
            }
        }
        self.anims.tick(2, Some(self.focus), dt);
        match pick {
            Some(1) => {
                cx.cmds.push(self.cmd.clone());
                Nav::Pop
            }
            Some(_) => Nav::Pop,
            None => Nav::Stay,
        }
    }
    fn draw(&self, sc: &mut Scene, cx: &DrawCx) {
        let u = cx.u;
        dim(sc, cx, 0.45);
        let p = Self::panel(cx.w, cx.h, u);
        theme::panel(sc, p, u, 1.0);
        let st = Style::new(24.0 * u, PAPER).align(Align::Center);
        for (i, l) in text::wrap(cx.fonts, &st, &cx.s.t(self.body), p.width() as f32 - 120.0 * u).iter().take(2).enumerate() {
            text::draw(sc, cx.fonts, &st, l, p.center().x as f32, p.y0 as f32 + 80.0 * u + i as f32 * 36.0 * u);
        }
        let btns = self.btns(cx.s);
        let rects = layout_buttons(cx.fonts, &btns, p.x1 as f32 - 40.0 * u, p.y1 as f32 - 58.0 * u, u);
        for (i, (b, r)) in btns.iter().zip(rects.iter()).enumerate() {
            draw_button(sc, cx.fonts, *r, b, self.anims.get(i), u, 1.0);
        }
    }
}
