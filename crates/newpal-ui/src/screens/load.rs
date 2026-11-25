//! Load screen: save slots from [crate::host::Host::saves]. The save system itself is not rebuilt yet, so
//! the slots show as empty and the note says so - the interface (slot id, title, location, play time) is fixed.
use super::{dim, hints, rect, Anims};
use crate::anim::ease_out_cubic;
use crate::host::SaveSlot;
use crate::input::{Action, Input};
use crate::scene::{Cmd, Ctx, DrawCx, Nav, Screen};
use crate::text::{self, Align, Style};
use crate::theme::{self, a, GOLD, INK_2, JADE, JADE_L, MUTED, PAPER, PAPER_DIM};
use vello::kurbo::{Affine, Point, Rect, RoundedRect, Stroke};
use vello::peniko::Fill;
use vello::Scene;

const SLOTS: usize = 6;

pub struct LoadScreen {
    focus: usize,
    anims: Anims,
    saves: Vec<SaveSlot>,
    queried: bool,
    t: f32,
}

impl Default for LoadScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl LoadScreen {
    pub fn new() -> Self {
        Self { focus: 0, anims: Anims::default(), saves: Vec::new(), queried: false, t: 0.0 }
    }
    fn panel(w: f32, h: f32, u: f32) -> Rect {
        let pw = (1400.0 * u).min(w - 60.0 * u);
        let ph = (820.0 * u).min(h - 60.0 * u);
        rect((w - pw) / 2.0, (h - ph) / 2.0, pw, ph)
    }
    fn slot_rect(i: usize, p: Rect, u: f32) -> Rect {
        let (col, row) = (i % 3, i / 3);
        let gap = 28.0 * u;
        let sw = (p.width() as f32 - 120.0 * u - 2.0 * gap) / 3.0;
        let sh = 230.0 * u;
        rect(p.x0 as f32 + 60.0 * u + col as f32 * (sw + gap), p.y0 as f32 + 150.0 * u + row as f32 * (sh + gap), sw, sh)
    }
    fn slot(&self, id: u32) -> Option<&SaveSlot> {
        self.saves.iter().find(|s| s.id == id)
    }
}

impl Screen for LoadScreen {
    fn name(&self) -> &'static str {
        "load"
    }
    fn update(&mut self, cx: &mut Ctx, inp: &Input, dt: f32) -> Nav {
        self.t += dt;
        if !self.queried {
            self.queried = true;
            self.saves = cx.host.saves();
        }
        let p = Self::panel(cx.w, cx.h, cx.u);
        let mut activate = false;
        for act in &inp.pressed {
            match act {
                Action::Back => return Nav::Pop,
                Action::Left if self.focus % 3 > 0 => self.focus -= 1,
                Action::Right if self.focus % 3 < 2 => self.focus += 1,
                Action::Up if self.focus >= 3 => self.focus -= 3,
                Action::Down if self.focus + 3 < SLOTS => self.focus += 3,
                Action::Confirm => activate = true,
                _ => {}
            }
        }
        for i in 0..SLOTS {
            if super::hit(Self::slot_rect(i, p, cx.u), inp.mouse) {
                if inp.mouse_moved {
                    self.focus = i;
                }
                if inp.click {
                    self.focus = i;
                    activate = true;
                }
            }
        }
        if inp.right_click {
            return Nav::Pop;
        }
        if activate && self.slot(self.focus as u32 + 1).is_some() {
            cx.cmds.push(Cmd::LoadSlot(self.focus as u32 + 1));
        }
        self.anims.tick(SLOTS, Some(self.focus), dt);
        Nav::Stay
    }
    fn draw(&self, sc: &mut Scene, cx: &DrawCx) {
        let u = cx.u;
        dim(sc, cx, 0.55);
        let p = Self::panel(cx.w, cx.h, u);
        theme::panel(sc, p, u, 1.0);
        super::header(sc, cx.fonts, &cx.s.t("load.title"), p.x0 as f32 + 60.0 * u, p.y0 as f32 + 70.0 * u, p.width() as f32 - 120.0 * u, u, 1.0);
        for i in 0..SLOTS {
            let k = ease_out_cubic(((self.t - i as f32 * 0.04) / 0.35).clamp(0.0, 1.0));
            let r = Self::slot_rect(i, p, u);
            let r = r.with_origin((r.x0, r.y0 + ((1.0 - k) * 18.0 * u) as f64));
            let f = self.anims.get(i);
            let rr = RoundedRect::from_rect(r, (12.0 * u) as f64);
            let slot = self.slot(i as u32 + 1);
            sc.fill(Fill::NonZero, Affine::IDENTITY, a(INK_2, (0.55 + 0.25 * f) * k), None, &rr);
            if slot.is_some() {
                sc.stroke(&Stroke::new(((1.0 + f) * u) as f64), Affine::IDENTITY, a(super::lerp_c(MUTED, JADE_L, f), k), None, &rr);
            } else {
                // dashed outline for empty slots
                sc.stroke(&Stroke::new(((1.0 + f) * u) as f64).with_dashes(0.0, [8.0 * u as f64, 6.0 * u as f64]), Affine::IDENTITY, a(super::lerp_c(MUTED, JADE, f), 0.7 * k), None, &rr);
            }
            let (x, y) = (r.x0 as f32 + 26.0 * u, r.y0 as f32);
            text::draw(sc, cx.fonts, &Style::new(18.0 * u, GOLD).alpha(k), &cx.s.f("load.slot", &[("n", (i + 1).to_string())]), x, y + 42.0 * u);
            match slot {
                Some(s) => {
                    text::draw(sc, cx.fonts, &Style::new(26.0 * u, PAPER).alpha(k), &s.title, x, y + 96.0 * u);
                    text::draw(sc, cx.fonts, &Style::new(19.0 * u, PAPER_DIM).alpha(k), &s.location, x, y + 134.0 * u);
                    let pt = format!("{}:{:02}  ·  {}", s.play_time_s / 3600, (s.play_time_s % 3600) / 60, s.saved_at);
                    text::draw(sc, cx.fonts, &Style::new(17.0 * u, MUTED).alpha(k), &pt, x, r.y1 as f32 - 24.0 * u);
                }
                None => {
                    theme::diamond(sc, Point::new(r.center().x, r.center().y + (10.0 * u) as f64), 8.0 * u, a(super::lerp_c(MUTED, JADE, f), 0.6 * k));
                    text::draw(sc, cx.fonts, &Style::new(20.0 * u, MUTED).align(Align::Center).alpha(k), &cx.s.t("load.empty"), r.center().x as f32, r.center().y as f32 + 56.0 * u);
                }
            }
        }
        if self.saves.is_empty() {
            let st = Style::new(19.0 * u, PAPER_DIM);
            theme::diamond(sc, Point::new((p.x0 + 66.0 * u as f64) as f64, p.y1 - (66.0 * u) as f64), 5.0 * u, JADE);
            text::draw_mid(sc, cx.fonts, &st, &cx.s.t("load.note"), p.x0 as f32 + 84.0 * u, p.y1 as f32 - 66.0 * u);
        }
        hints(sc, cx, &[(Action::Confirm, cx.s.t("hint.select")), (Action::Back, cx.s.t("hint.back"))], 1.0);
    }
}
