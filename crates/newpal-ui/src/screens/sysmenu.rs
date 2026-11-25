//! In-game system menu in the original's layout and materials (vector-drawn, see [`crate::orig`]): sepia veil
//! over the frozen world, a U-shaped wooden frame on the left holding seven burlap plaques, the gold-ingot money
//! box top right, and the small option panel (music / sound / quit) next to the Options plaque. "More settings"
//! leads to the modern settings screen, so nothing from the front end is lost.
use super::{Anims, ConfirmScreen};
use crate::anim::{approach, ease_out_cubic};
use crate::input::{Action, Input};
use crate::orig::{self, Ref};
use crate::scene::{Cmd, Ctx, DrawCx, Nav, Screen, Transition};
use crate::text::{self, Align, Style};
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};
use vello::Scene;

pub const ITEMS: [&str; 7] = ["sysmenu.items", "sysmenu.equip", "sysmenu.magic", "sysmenu.status", "sysmenu.load", "sysmenu.save", "sysmenu.option"];
const OPTS: [&str; 4] = ["sysmenu.music", "sysmenu.sfx", "sysmenu.more", "sysmenu.quit"];

// reference-space layout (640x480)
const FRAME: (f32, f32, f32, f32) = (24.0, 34.0, 156.0, 420.0);
const PLAQUE_W: f32 = 110.0;
/// four-character plaques are taller (2 + 2 lines), like the original's 110x75 vs 112x60 buttons
const PLAQUE_H4: f32 = 56.0;
const PLAQUE_H2: f32 = 46.0;
const PLAQUE_GAP: f32 = 6.0;
fn tall(i: usize) -> bool {
    matches!(ITEMS[i], "sysmenu.items" | "sysmenu.load" | "sysmenu.save" | "sysmenu.option")
}
const MONEY: (f32, f32, f32, f32) = (430.0, 20.0, 188.0, 44.0);
const PANEL: (f32, f32, f32, f32) = (186.0, 300.0, 168.0, 140.0);

pub struct SysMenuScreen {
    focus: usize,
    anims: Anims,
    panel: Option<usize>,
    panel_k: f32,
    opt_anims: Anims,
    toast: f32,
    t: f32,
    money: Option<i64>,
}

impl SysMenuScreen {
    pub fn new(money: Option<i64>) -> Self {
        Self { focus: 0, anims: Anims::default(), panel: None, panel_k: 0.0, opt_anims: Anims::default(), toast: 0.0, t: 0.0, money }
    }
    fn plaque_ref(i: usize) -> (f32, f32, f32, f32) {
        let x = FRAME.0 + (FRAME.2 - PLAQUE_W) / 2.0;
        let h = |j: usize| if tall(j) { PLAQUE_H4 } else { PLAQUE_H2 };
        let y = FRAME.1 + 10.0 + (0..i).map(|j| h(j) + PLAQUE_GAP).sum::<f32>();
        (x, y, PLAQUE_W, h(i))
    }
    fn opt_row_ref(i: usize) -> (f32, f32, f32, f32) {
        (PANEL.0 + 10.0, PANEL.1 + 14.0 + i as f32 * 28.0, PANEL.2 - 20.0, 26.0)
    }
    fn activate(&mut self, cx: &mut Ctx) -> Nav {
        let key = ITEMS[self.focus];
        match key {
            "sysmenu.load" => Nav::Push(Box::new(super::LoadScreen::new())),
            "sysmenu.option" => {
                self.panel = Some(0);
                Nav::Stay
            }
            _ => {
                let _ = cx;
                self.toast = 1.6;
                Nav::Stay
            }
        }
    }
    fn vol<'a>(cx: &'a mut Ctx, i: usize) -> &'a mut f32 {
        if i == 0 {
            &mut cx.settings.music
        } else {
            &mut cx.settings.sfx
        }
    }
    fn panel_input(&mut self, cx: &mut Ctx, inp: &Input, sel: usize) -> Nav {
        let n = OPTS.len();
        let mut sel = sel;
        let mut nav = Nav::Stay;
        let mut changed = false;
        for act in &inp.pressed {
            match act {
                Action::Up => sel = (sel + n - 1) % n,
                Action::Down => sel = (sel + 1) % n,
                Action::Left | Action::Right if sel < 2 => {
                    let d = if *act == Action::Left { -0.1 } else { 0.1 };
                    let v = Self::vol(cx, sel);
                    *v = ((*v + d) * 10.0).round().clamp(0.0, 10.0) / 10.0;
                    changed = true;
                }
                Action::Back | Action::Menu => {
                    self.panel = None;
                    return Nav::Stay;
                }
                Action::Confirm => nav = self.panel_activate(sel),
                _ => {}
            }
        }
        let r = Ref::new(cx.w, cx.h);
        for i in 0..n {
            let (x, y, w, h) = Self::opt_row_ref(i);
            if super::hit(r.r(x, y, w, h), inp.mouse) {
                if inp.mouse_moved {
                    sel = i;
                }
                if inp.click {
                    sel = i;
                    if i < 2 {
                        // click on the bar sets the volume directly
                        let (mx, _) = r.inv(inp.mouse.0, inp.mouse.1);
                        let bx = x + 52.0;
                        let bw = w - 58.0;
                        if mx >= bx - 4.0 {
                            *Self::vol(cx, i) = (((mx - bx) / bw).clamp(0.0, 1.0) * 10.0).round() / 10.0;
                            changed = true;
                        }
                    } else {
                        nav = self.panel_activate(i);
                    }
                }
            }
        }
        if inp.click && !super::hit(r.r(PANEL.0, PANEL.1, PANEL.2, PANEL.3), inp.mouse) && !super::hit(Self::plaque_rect(&r, 6), inp.mouse) {
            self.panel = None;
            return nav;
        }
        if changed {
            cx.cmds.push(Cmd::ApplySettings);
            cx.cmds.push(Cmd::SaveSettings);
        }
        if self.panel.is_some() {
            self.panel = Some(sel);
        }
        nav
    }
    fn panel_activate(&mut self, sel: usize) -> Nav {
        match OPTS[sel] {
            "sysmenu.more" => Nav::Push(Box::new(super::SettingsScreen::new())),
            "sysmenu.quit" => Nav::Push(Box::new(ConfirmScreen::new("confirm.to_title", Cmd::ToTitle))),
            _ => Nav::Stay,
        }
    }
    fn plaque_rect(r: &Ref, i: usize) -> Rect {
        let (x, y, w, h) = Self::plaque_ref(i);
        r.r(x, y, w, h)
    }
}

impl Screen for SysMenuScreen {
    fn name(&self) -> &'static str {
        "sysmenu"
    }
    fn transition(&self) -> Transition {
        Transition::Fade
    }
    fn update(&mut self, cx: &mut Ctx, inp: &Input, dt: f32) -> Nav {
        self.t += dt;
        self.toast = (self.toast - dt).max(0.0);
        approach(&mut self.panel_k, if self.panel.is_some() { 1.0 } else { 0.0 }, 14.0, dt);
        if let Some(sel) = self.panel {
            let nav = self.panel_input(cx, inp, sel);
            self.anims.tick(ITEMS.len(), Some(self.focus), dt);
            self.opt_anims.tick(OPTS.len(), self.panel, dt);
            return nav;
        }
        self.opt_anims.tick(OPTS.len(), None, dt);
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
        let r = Ref::new(cx.w, cx.h);
        for i in 0..n {
            if super::hit(Self::plaque_rect(&r, i), inp.mouse) {
                if inp.mouse_moved {
                    self.focus = i;
                }
                if inp.click {
                    self.focus = i;
                    nav = self.activate(cx);
                }
            }
        }
        if inp.right_click {
            cx.cmds.push(Cmd::Resume);
            return Nav::Pop;
        }
        self.anims.tick(n, Some(self.focus), dt);
        nav
    }
    fn draw(&self, sc: &mut Scene, cx: &DrawCx) {
        let r = Ref::new(cx.w, cx.h);
        let s = r.s as f64;
        let k = ease_out_cubic((self.t / 0.35).clamp(0.0, 1.0));
        orig::sepia_veil(sc, cx.w as f64, cx.h as f64, cx.time, 1.0);
        // frame slides down like the original's drop-in
        let drop = (1.0 - k) * -30.0;
        let fr = r.r(FRAME.0, FRAME.1 + drop, FRAME.2, FRAME.3);
        orig::u_frame(sc, fr, s, k);
        for (i, key) in ITEMS.iter().enumerate() {
            let ki = ease_out_cubic(((self.t - 0.05 - i as f32 * 0.035) / 0.25).clamp(0.0, 1.0));
            let (x, y, w, h) = Self::plaque_ref(i);
            let pr = r.r(x, y + drop - (1.0 - ki) * 10.0, w, h);
            let f = self.anims.get(i);
            // entries not built yet keep the normal look (the original greys only unavailable actions) and
            // show a short notice when chosen
            let dis = false;
            orig::plaque(sc, pr, r.s, 17 + i as u32 * 101, dis, f, ki);
            orig::label(sc, cx.fonts, &cx.s.t(key), pr.inset(-4.0 * s), r.s, dis, f, ki);
        }
        // money box
        let money = self.money.map(|m| m.to_string()).unwrap_or_else(|| cx.s.t("sysmenu.money_none"));
        orig::money_box(sc, cx.fonts, r.r(MONEY.0, MONEY.1, MONEY.2, MONEY.3), s, &money, k);
        // option panel
        if self.panel_k > 0.01 {
            let pk = self.panel_k;
            let pr = r.r(PANEL.0 - (1.0 - pk) * 12.0, PANEL.1, PANEL.2, PANEL.3);
            orig::plaque(sc, pr, r.s, 901, false, 0.0, pk);
            for (i, key) in OPTS.iter().enumerate() {
                let (x, y, w, h) = Self::opt_row_ref(i);
                let rr = r.r(x - (1.0 - pk) * 12.0, y, w, h);
                let f = self.opt_anims.get(i);
                let size = (16.5 * r.s).round();
                let fill = super::lerp_c(orig::C_INK, orig::C_VERMILION, f);
                let st = Style::new(size, fill.multiply_alpha(pk)).dialog_font().spacing(1.5 * r.s);
                let label = cx.s.t(key);
                let cy = rr.center().y as f32;
                // pale rim then ink, like the plaque labels
                let chars = text::place_chars(cx.fonts, &st, &label).into_iter().map(|c| (c, 0.0)).collect::<Vec<_>>();
                let line = text::shape(cx.fonts, &st, &label);
                let base = cy + (line.ascent + line.descent) / 2.0;
                let xf = Affine::translate((rr.x0 + 4.0 * s, base as f64));
                let rim = super::lerp_c(orig::C_RIM, Color::WHITE, f).multiply_alpha(pk);
                text::paint_chars(sc, cx.fonts, &st, &chars, &vello::peniko::Brush::Solid(rim), xf, Some(&vello::kurbo::Stroke::new((size * 0.16) as f64).with_join(vello::kurbo::Join::Round)));
                text::paint_chars(sc, cx.fonts, &st, &chars, &vello::peniko::Brush::Solid(fill.multiply_alpha(pk)), xf, None);
                if i < 2 {
                    let v = if i == 0 { cx.settings.music } else { cx.settings.sfx };
                    let bx = rr.x0 + 52.0 * s;
                    let bw = rr.width() - 58.0 * s;
                    // brush-stroke bar like the original's volume sprites: groove + inked part
                    let groove = vello::kurbo::RoundedRect::new(bx, rr.center().y - 2.0 * s, bx + bw, rr.center().y + 2.0 * s, 2.0 * s);
                    sc.fill(Fill::NonZero, Affine::IDENTITY, orig::C_CHAR.multiply_alpha(0.25 * pk), None, &groove);
                    let ink = vello::kurbo::RoundedRect::new(bx, rr.center().y - 3.2 * s, bx + bw * v as f64, rr.center().y + 3.2 * s, 3.2 * s);
                    sc.fill(Fill::NonZero, Affine::IDENTITY, fill.multiply_alpha(0.9 * pk), None, &ink);
                    let knob = vello::kurbo::Circle::new((bx + bw * v as f64, rr.center().y), 4.6 * s);
                    sc.fill(Fill::NonZero, Affine::IDENTITY, orig::C_RIM.multiply_alpha(pk), None, &knob);
                    sc.stroke(&vello::kurbo::Stroke::new(1.2 * s), Affine::IDENTITY, fill.multiply_alpha(pk), None, &knob);
                }
            }
        }
        // short notice for entries not built yet
        if self.toast > 0.0 {
            let a = (self.toast / 0.3).min(1.0);
            let tr = r.r(220.0, 220.0, 200.0, 40.0);
            orig::plaque(sc, tr, r.s, 333, false, 0.0, a);
            let st = Style::new((15.0 * r.s).round(), orig::C_INK.multiply_alpha(a)).dialog_font().align(Align::Center);
            text::draw_mid(sc, cx.fonts, &st, &cx.s.t("sysmenu.todo"), tr.center().x as f32, tr.center().y as f32);
        }
    }
}
