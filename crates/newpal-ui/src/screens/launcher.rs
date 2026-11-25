//! First-run launcher, inside the game window: find the original game (Steam auto-detect, saved setting or a
//! folder picker), verify it, offer the HD assets, then ink-wipe straight into the title screen.
use super::{draw_button, fmt_bytes, fmt_secs, layout_buttons, rect, short_path, Anims, Btn};
use crate::anim::{approach, ease_out_cubic};
use crate::host::{DirCandidate, DirCheck, HdState};
use crate::input::{Action, Input};
use crate::scene::{Cmd, Ctx, DrawCx, Nav, Screen, Transition};
use crate::text::{self, Align, Style};
use crate::theme::{self, a, BAD, GOLD, INK_2, JADE, JADE_D, JADE_L, MUTED, OK, PAPER, PAPER_DIM, WARN};
use vello::kurbo::{Affine, Circle, Point, Rect, RoundedRect, Stroke};
use vello::peniko::Fill;
use vello::Scene;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Step {
    Scan,
    Dir,
    Hd,
}

pub struct LauncherScreen {
    step: Step,
    t_step: f32,
    t: f32,
    cands: Vec<(DirCandidate, DirCheck)>,
    sel: usize,
    focus: usize,
    card_anims: Anims,
    btn_anims: Anims,
    hd: HdState,
    hd_poll: f32,
    /// opened from settings: pop instead of entering the title
    hd_only: bool,
    shown_frac: f32,
}

impl Default for LauncherScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl LauncherScreen {
    pub fn new() -> Self {
        Self { step: Step::Scan, t_step: 0.0, t: 0.0, cands: Vec::new(), sel: 0, focus: 0, card_anims: Anims::default(), btn_anims: Anims::default(), hd: HdState::Missing { patch_items: 0, patch_bytes: 0 }, hd_poll: 0.0, hd_only: false, shown_frac: 0.0 }
    }
    /// Only the HD step (settings -> game data -> HD assets).
    pub fn hd_only() -> Self {
        let mut s = Self::new();
        s.step = Step::Hd;
        s.hd_only = true;
        s
    }

    fn panel(w: f32, h: f32, u: f32) -> Rect {
        let pw = 1180.0 * u;
        let ph = 760.0 * u;
        rect((w - pw) / 2.0, (h - ph) / 2.0, pw, ph)
    }

    fn buttons(&self, s: &crate::strings::Strings) -> Vec<Btn> {
        match self.step {
            Step::Scan => vec![],
            Step::Dir => {
                if self.cands.is_empty() {
                    vec![Btn::new("quit", s.t("launcher.quit")), Btn::new("browse", s.t("launcher.browse_first")).primary()]
                } else {
                    let usable = self.cands.get(self.sel).is_some_and(|c| c.1.usable);
                    vec![Btn::new("quit", s.t("launcher.quit")), Btn::new("browse", s.t("launcher.browse")), Btn::new("use", s.t("launcher.use")).primary().enabled(usable)]
                }
            }
            Step::Hd => match &self.hd {
                HdState::Ready { .. } => vec![Btn::new("enter", s.t("launcher.enter")).primary()],
                HdState::Missing { .. } | HdState::Failed(_) => vec![Btn::new("skip", s.t("hd.skip")), Btn::new("install", s.t("hd.install")).primary()],
                HdState::Installing { .. } => vec![Btn::new("cancel", s.t("hd.cancel"))],
            },
        }
    }

    fn button_rects(&self, cx_fonts: &crate::text::Fonts, btns: &[Btn], w: f32, h: f32, u: f32) -> Vec<Rect> {
        let p = Self::panel(w, h, u);
        layout_buttons(cx_fonts, btns, (p.x1 as f32) - 56.0 * u, (p.y1 as f32) - 64.0 * u, u)
    }

    fn card_rect(i: usize, w: f32, h: f32, u: f32) -> Rect {
        let p = Self::panel(w, h, u);
        rect(p.x0 as f32 + 56.0 * u, p.y0 as f32 + 250.0 * u + i as f32 * 118.0 * u, p.width() as f32 - 112.0 * u, 104.0 * u)
    }

    fn scan(&mut self, cx: &mut Ctx) {
        let mut found = cx.host.detect_game_dirs();
        found.dedup_by(|a, b| a.path == b.path);
        self.cands = found.into_iter().take(3).map(|c| {
            let chk = cx.host.check_dir(&c.path);
            (c, chk)
        }).collect();
        self.sel = self.cands.iter().position(|c| c.1.usable).unwrap_or(0);
    }

    fn set_step(&mut self, s: Step) {
        self.step = s;
        self.t_step = 0.0;
        self.focus = usize::MAX;
    }

    fn finish(&mut self, cx: &mut Ctx) -> Nav {
        if self.hd_only {
            return Nav::Pop;
        }
        cx.cmds.push(Cmd::EnterTitle);
        Nav::Replace(Box::new(super::TitleScreen::new()), Transition::Ink(Point::new(cx.w as f64 * 0.5, cx.h as f64 * 0.55)))
    }

    fn press(&mut self, id: &str, cx: &mut Ctx) -> Nav {
        match id {
            "quit" => {
                cx.cmds.push(Cmd::Quit);
                Nav::Stay
            }
            "browse" => {
                if let Some(p) = cx.host.pick_dir() {
                    let chk = cx.host.check_dir(&p);
                    self.cands.retain(|c| c.0.path != p);
                    self.cands.insert(0, (DirCandidate { path: p, source: "picked".into() }, chk));
                    self.cands.truncate(3);
                    self.sel = 0;
                }
                Nav::Stay
            }
            "use" => {
                if let Some((c, chk)) = self.cands.get(self.sel) {
                    if chk.usable {
                        cx.cmds.push(Cmd::UseGameDir(c.path.clone()));
                        self.hd = cx.host.hd_state();
                        self.set_step(Step::Hd);
                    }
                }
                Nav::Stay
            }
            "install" => {
                cx.host.start_hd_install();
                self.hd = cx.host.hd_state();
                Nav::Stay
            }
            "cancel" => {
                cx.host.cancel_hd_install();
                self.hd = cx.host.hd_state();
                Nav::Stay
            }
            "skip" => {
                if cx.settings.present == "hd" {
                    cx.settings.present = "shader".into();
                    cx.cmds.push(Cmd::ApplySettings);
                }
                self.finish(cx)
            }
            "enter" => {
                if cx.settings.present != "hd" && !self.hd_only {
                    cx.settings.present = "hd".into();
                    cx.cmds.push(Cmd::ApplySettings);
                }
                self.finish(cx)
            }
            _ => Nav::Stay,
        }
    }
}

impl Screen for LauncherScreen {
    fn name(&self) -> &'static str {
        "launcher"
    }
    fn opaque(&self) -> bool {
        !self.hd_only
    }
    fn transition(&self) -> Transition {
        Transition::Fade
    }

    fn update(&mut self, cx: &mut Ctx, inp: &Input, dt: f32) -> Nav {
        self.t += dt;
        self.t_step += dt;
        if self.step == Step::Scan && self.t_step > 0.25 && self.cands.is_empty() && self.t_step < 10.0 {
            self.scan(cx);
            self.t_step = 10.0; // scanned
        }
        if self.step == Step::Scan && self.t_step > 10.0 + 0.9 {
            self.set_step(Step::Dir);
        }
        if self.step == Step::Hd {
            self.hd_poll -= dt;
            if self.hd_poll <= 0.0 {
                self.hd = cx.host.hd_state();
                self.hd_poll = 0.25;
            }
            if let HdState::Installing { done, total, .. } = &self.hd {
                let f = *done as f32 / (*total).max(1) as f32;
                approach(&mut self.shown_frac, f, 6.0, dt);
            }
        }
        let btns = self.buttons(cx.s);
        if self.focus == usize::MAX || self.focus >= btns.len() {
            self.focus = btns.iter().rposition(|b| b.enabled && b.primary).or_else(|| btns.iter().rposition(|b| b.enabled)).unwrap_or(0);
        }
        let rects = self.button_rects(cx.fonts, &btns, cx.w, cx.h, cx.u);
        let mut nav = Nav::Stay;
        let mut pressed: Option<&'static str> = None;
        for act in &inp.pressed {
            match act {
                Action::Left if self.focus > 0 => self.focus -= 1,
                Action::Right if self.focus + 1 < btns.len() => self.focus += 1,
                Action::Up if self.step == Step::Dir && self.sel > 0 => self.sel -= 1,
                Action::Down if self.step == Step::Dir && self.sel + 1 < self.cands.len() => self.sel += 1,
                Action::Confirm => {
                    if let Some(b) = btns.get(self.focus) {
                        if b.enabled {
                            pressed = Some(b.id);
                        }
                    }
                }
                Action::Back if self.hd_only => nav = Nav::Pop,
                _ => {}
            }
        }
        for (i, r) in rects.iter().enumerate() {
            if super::hit(*r, inp.mouse) && btns[i].enabled {
                if inp.mouse_moved {
                    self.focus = i;
                }
                if inp.click {
                    self.focus = i;
                    pressed = Some(btns[i].id);
                }
            }
        }
        if self.step == Step::Dir {
            for i in 0..self.cands.len() {
                if super::hit(Self::card_rect(i, cx.w, cx.h, cx.u), inp.mouse) && inp.click {
                    self.sel = i;
                }
            }
        }
        if let Some(id) = pressed {
            nav = self.press(id, cx);
        }
        self.card_anims.tick(self.cands.len(), Some(self.sel), dt);
        self.btn_anims.tick(btns.len(), Some(self.focus), dt);
        nav
    }

    fn draw(&self, sc: &mut Scene, cx: &DrawCx) {
        let (w, h, u, t) = (cx.w, cx.h, cx.u, cx.time);
        if self.hd_only {
            super::dim(sc, cx, 0.6);
        } else {
            theme::ink_backdrop(sc, w, h, t);
            theme::mountains(sc, w, h, t, cx.parallax(), 0.7);
            theme::spirits(sc, w, h, t, u, 0.6);
        }
        let p = Self::panel(w, h, u);
        let appear = ease_out_cubic((self.t / 0.6).min(1.0));
        theme::panel(sc, p, u, appear);
        let (px, py) = (p.x0 as f32, p.y0 as f32);
        let pw = p.width() as f32;

        // header: small title + seal, welcome line
        let tst = Style::new(46.0 * u, PAPER).title().spacing(4.0 * u);
        let tw = text::draw(sc, cx.fonts, &tst, &cx.s.t(if self.hd_only { "launcher.step.hd" } else { "launcher.title" }), px + 56.0 * u, py + 92.0 * u);
        theme::seal(sc, cx.fonts, Point::new((px + 56.0 * u + tw + 44.0 * u) as f64, (py + 76.0 * u) as f64), 44.0 * u, &cx.s.t("title.seal"), 1.0);
        let sub = Style::new(20.0 * u, PAPER_DIM);
        for (i, l) in text::wrap(cx.fonts, &sub, &cx.s.t("launcher.subtitle"), pw - 112.0 * u).iter().take(2).enumerate() {
            text::draw(sc, cx.fonts, &sub, l, px + 56.0 * u, py + 136.0 * u + i as f32 * 30.0 * u);
        }

        // stepper (top right)
        if !self.hd_only {
            let steps = ["launcher.step.dir", "launcher.step.check", "launcher.step.hd", "launcher.step.play"];
            let cur = match self.step {
                Step::Scan => 0,
                Step::Dir => 1,
                Step::Hd => 2,
            };
            let sx = px + pw - 56.0 * u - 3.0 * 120.0 * u;
            let sy = py + 74.0 * u;
            for i in 0..4 {
                let x = sx + i as f32 * 120.0 * u;
                if i < 3 {
                    let col = if i < cur { JADE } else { a(MUTED, 0.5) };
                    theme::fill_rect(sc, Rect::new((x + 14.0 * u) as f64, (sy - 1.0 * u) as f64, (x + 106.0 * u) as f64, (sy + 1.0 * u) as f64), col);
                }
                let c = Point::new(x as f64, sy as f64);
                if i < cur {
                    theme::icon_check(sc, c, 11.0 * u, JADE_L);
                } else if i == cur {
                    let pulse = 0.5 + 0.5 * (t * 3.0).sin();
                    sc.draw_blurred_rounded_rect(Affine::IDENTITY, Rect::new(c.x - 16.0 * u as f64, c.y - 16.0 * u as f64, c.x + 16.0 * u as f64, c.y + 16.0 * u as f64), a(JADE, 0.4 * pulse), 16.0 * u as f64, 8.0 * u as f64);
                    sc.fill(Fill::NonZero, Affine::IDENTITY, JADE, None, &Circle::new(c, (8.0 * u) as f64));
                } else {
                    sc.stroke(&Stroke::new((1.5 * u) as f64), Affine::IDENTITY, a(MUTED, 0.8), None, &Circle::new(c, (7.0 * u) as f64));
                }
                let col = if i == cur { PAPER } else if i < cur { PAPER_DIM } else { MUTED };
                text::draw(sc, cx.fonts, &Style::new(16.0 * u, col).align(Align::Center), &cx.s.t(steps[i]), x, sy + 34.0 * u);
            }
        }

        let body_y = py + 250.0 * u;
        let k = ease_out_cubic((self.t_step / 0.4).min(1.0));
        match self.step {
            Step::Scan => {
                let c = Point::new((px + pw / 2.0) as f64, (body_y + 120.0 * u) as f64);
                theme::ring(sc, c, 46.0 * u, 5.0 * u, ((t * 0.7) % 1.0) * 0.8 + 0.1, t * 2.0, 1.0);
                text::draw(sc, cx.fonts, &Style::new(24.0 * u, PAPER).align(Align::Center), &cx.s.t("launcher.scanning"), px + pw / 2.0, body_y + 230.0 * u);
            }
            Step::Dir => {
                if self.cands.is_empty() {
                    let c = Point::new((px + pw / 2.0) as f64, (body_y + 70.0 * u) as f64);
                    theme::icon_warn(sc, c, 34.0 * u, a(WARN, k));
                    text::draw(sc, cx.fonts, &Style::new(28.0 * u, PAPER).align(Align::Center).alpha(k), &cx.s.t("launcher.not_found"), px + pw / 2.0, body_y + 160.0 * u);
                    text::draw(sc, cx.fonts, &Style::new(20.0 * u, PAPER_DIM).align(Align::Center).alpha(k), &cx.s.t("launcher.not_found_hint"), px + pw / 2.0, body_y + 200.0 * u);
                } else {
                    text::draw(sc, cx.fonts, &Style::new(20.0 * u, GOLD).alpha(k), &cx.s.f("launcher.found", &[("n", self.cands.len().to_string())]), px + 56.0 * u, body_y - 18.0 * u);
                    for (i, (cand, chk)) in self.cands.iter().enumerate() {
                        let kk = ease_out_cubic(((self.t_step - i as f32 * 0.08) / 0.4).clamp(0.0, 1.0));
                        let r = Self::card_rect(i, w, h, u);
                        let r = r.with_origin((r.x0, r.y0 + ((1.0 - kk) * 16.0 * u) as f64));
                        let f = self.card_anims.get(i);
                        let rr = RoundedRect::from_rect(r, (10.0 * u) as f64);
                        sc.fill(Fill::NonZero, Affine::IDENTITY, a(INK_2, (0.7 + 0.2 * f) * kk), None, &rr);
                        sc.stroke(&Stroke::new(((1.0 + f) * u) as f64), Affine::IDENTITY, a(super::lerp_c(MUTED, JADE_L, f), (0.4 + 0.6 * f) * kk), None, &rr);
                        let ic = Point::new(r.x0 + (44.0 * u) as f64, r.center().y);
                        let (status, col) = if chk.exact {
                            theme::icon_check(sc, ic, 18.0 * u, a(OK, kk));
                            (cx.s.t("launcher.check.exact"), OK)
                        } else if chk.usable {
                            theme::icon_warn(sc, ic, 16.0 * u, a(WARN, kk));
                            (cx.s.t("launcher.check.usable"), WARN)
                        } else {
                            theme::icon_cross(sc, ic, 18.0 * u, a(BAD, kk));
                            (cx.s.f("launcher.check.bad", &[("n", chk.missing.len().to_string())]), BAD)
                        };
                        let tx = r.x0 as f32 + 84.0 * u;
                        let pst = Style::new(23.0 * u, PAPER).alpha(kk);
                        let path = short_path(cx.fonts, &pst, &cand.path.display().to_string(), r.width() as f32 - 330.0 * u);
                        text::draw(sc, cx.fonts, &pst, &path, tx, r.y0 as f32 + 44.0 * u);
                        text::draw(sc, cx.fonts, &Style::new(18.0 * u, col).alpha(kk), &status, tx, r.y0 as f32 + 78.0 * u);
                        let files = cx.s.f("launcher.check.files", &[("ok", chk.ok_files.to_string()), ("total", chk.total_files.to_string())]);
                        text::draw(sc, cx.fonts, &Style::new(16.0 * u, MUTED).align(Align::Right).alpha(kk), &files, r.x1 as f32 - 28.0 * u, r.y0 as f32 + 78.0 * u);
                        // source chip
                        let src = cx.s.t(&format!("launcher.source.{}", cand.source));
                        let cst = Style::new(15.0 * u, JADE_L).alpha(kk);
                        let cw = text::measure(cx.fonts, &cst, &src) + 24.0 * u;
                        let chip = rect(r.x1 as f32 - 28.0 * u - cw, r.y0 as f32 + 22.0 * u, cw, 28.0 * u);
                        sc.stroke(&Stroke::new((1.0 * u) as f64), Affine::IDENTITY, a(JADE, 0.7 * kk), None, &RoundedRect::from_rect(chip, (14.0 * u) as f64));
                        text::draw_mid(sc, cx.fonts, &cst.align(Align::Center), &src, chip.center().x as f32, chip.center().y as f32);
                    }
                    let ny = Self::card_rect(self.cands.len(), w, h, u).y0 as f32 + 22.0 * u;
                    theme::icon_folder(sc, Point::new((px + 70.0 * u) as f64, ny as f64), 11.0 * u, a(MUTED, k));
                    text::draw_mid(sc, cx.fonts, &Style::new(17.0 * u, MUTED).alpha(k), &cx.s.t("launcher.readonly"), px + 92.0 * u, ny);
                }
            }
            Step::Hd => self.draw_hd(sc, cx, p, k),
        }

        // buttons
        let btns = self.buttons(cx.s);
        let rects = self.button_rects(cx.fonts, &btns, w, h, u);
        for (i, (b, r)) in btns.iter().zip(rects.iter()).enumerate() {
            draw_button(sc, cx.fonts, *r, b, self.btn_anims.get(i), u, appear);
        }
        let _ = JADE_D;
    }
}

impl LauncherScreen {
    fn draw_hd(&self, sc: &mut Scene, cx: &DrawCx, p: Rect, k: f32) {
        let u = cx.u;
        let (px, pw) = (p.x0 as f32, p.width() as f32);
        let y = p.y0 as f32 + 250.0 * u;
        let x = px + 56.0 * u;
        match &self.hd {
            HdState::Ready { items } => {
                theme::icon_check(sc, Point::new((x + 26.0 * u) as f64, (y + 40.0 * u) as f64), 24.0 * u, a(OK, k));
                text::draw_mid(sc, cx.fonts, &Style::new(30.0 * u, PAPER).alpha(k), &cx.s.f("hd.ready", &[("n", items.to_string())]), x + 70.0 * u, y + 40.0 * u);
                text::draw(sc, cx.fonts, &Style::new(19.0 * u, PAPER_DIM).alpha(k), &cx.s.t("set.refresh.desc"), x + 70.0 * u, y + 100.0 * u);
            }
            HdState::Missing { patch_items, patch_bytes } => {
                text::draw(sc, cx.fonts, &Style::new(30.0 * u, PAPER).alpha(k), &cx.s.t("hd.missing"), x, y + 20.0 * u);
                let st = Style::new(20.0 * u, PAPER_DIM).alpha(k);
                for (i, l) in text::wrap(cx.fonts, &st, &cx.s.t("hd.missing_body"), pw - 112.0 * u).iter().enumerate() {
                    text::draw(sc, cx.fonts, &st, l, x, y + 70.0 * u + i as f32 * 32.0 * u);
                }
                let (txt, col) = if *patch_items > 0 {
                    (cx.s.f("hd.patch_found", &[("n", patch_items.to_string()), ("size", fmt_bytes(*patch_bytes))]), JADE_L)
                } else {
                    (cx.s.t("hd.patch_none"), WARN)
                };
                theme::diamond(sc, Point::new((x + 6.0 * u) as f64, (y + 200.0 * u) as f64), 5.0 * u, a(col, k));
                text::draw_mid(sc, cx.fonts, &Style::new(19.0 * u, col).alpha(k), &txt, x + 22.0 * u, y + 200.0 * u);
            }
            HdState::Installing { done, total, eta_s, simulated } => {
                let c = Point::new((x + 90.0 * u) as f64, (y + 110.0 * u) as f64);
                theme::ring(sc, c, 74.0 * u, 7.0 * u, self.shown_frac, cx.time, k);
                text::draw_mid(sc, cx.fonts, &Style::new(34.0 * u, PAPER).align(Align::Center).alpha(k), &format!("{:.0}%", self.shown_frac * 100.0), c.x as f32, c.y as f32);
                let tx = x + 210.0 * u;
                text::draw(sc, cx.fonts, &Style::new(30.0 * u, PAPER).alpha(k), &cx.s.t("hd.installing"), tx, y + 60.0 * u);
                theme::bar(sc, rect(tx, y + 96.0 * u, pw - 112.0 * u - 210.0 * u, 8.0 * u), self.shown_frac, cx.time, u, k);
                let mut info = cx.s.f("hd.progress", &[("done", done.to_string()), ("total", total.to_string())]);
                if let Some(e) = eta_s {
                    info = format!("{info}  ·  {}", cx.s.f("hd.eta", &[("t", fmt_secs(*e))]));
                }
                text::draw(sc, cx.fonts, &Style::new(20.0 * u, PAPER_DIM).alpha(k), &info, tx, y + 146.0 * u);
                if *simulated {
                    text::draw(sc, cx.fonts, &Style::new(17.0 * u, WARN).alpha(k), &cx.s.t("hd.simulated"), tx, y + 182.0 * u);
                }
            }
            HdState::Failed(e) => {
                theme::icon_cross(sc, Point::new((x + 24.0 * u) as f64, (y + 30.0 * u) as f64), 22.0 * u, a(BAD, k));
                text::draw_mid(sc, cx.fonts, &Style::new(24.0 * u, PAPER).alpha(k), &cx.s.f("hd.failed", &[("e", e.clone())]), x + 64.0 * u, y + 30.0 * u);
            }
        }
    }
}
