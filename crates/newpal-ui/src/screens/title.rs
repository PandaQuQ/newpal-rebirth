//! Title screen: the player's own map (rendered by the engine behind the UI) with ink mountains, mist, moon and
//! spirit lights layered on top with mouse parallax; menu on the left with brush-stroke focus.
use super::{hints, Anims};
use crate::anim::{approach, ease_out_cubic};
use crate::input::Action;
use crate::scene::{Cmd, Ctx, DrawCx, Nav, Screen, Transition};
use crate::text::{self, Style};
use crate::theme::{self, a, GOLD, GOLD_D, JADE, JADE_L, MUTED, PAPER, PAPER_DIM};
use vello::kurbo::{Affine, Point, Rect};
use vello::peniko::{Brush, Fill, Gradient};
use vello::Scene;

const ITEMS: [&str; 6] = ["menu.continue", "menu.new", "menu.load", "menu.settings", "menu.mods", "menu.quit"];

pub struct TitleScreen {
    focus: usize,
    anims: Anims,
    hl: f32,
    intro: f32,
    has_save: bool,
    data: String,
    queried: bool,
}

impl Default for TitleScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl TitleScreen {
    pub fn new() -> Self {
        Self { focus: 1, anims: Anims::default(), hl: -1.0, intro: 0.0, has_save: false, data: String::new(), queried: false }
    }

    fn enabled(&self, i: usize) -> bool {
        i != 0 || self.has_save
    }

    fn item_y(i: usize, u: f32, h: f32) -> f32 {
        let top = (h / u).max(900.0) * 0.5 + 40.0;
        (top + i as f32 * 64.0) * u
    }

    fn item_rect(i: usize, u: f32, h: f32) -> Rect {
        let y = Self::item_y(i, u, h);
        Rect::new((120.0 * u) as f64, (y - 28.0 * u) as f64, (620.0 * u) as f64, (y + 28.0 * u) as f64)
    }

    fn activate(&mut self, cx: &mut Ctx) -> Nav {
        match ITEMS[self.focus] {
            "menu.continue" => {
                cx.cmds.push(Cmd::Continue);
                Nav::Stay
            }
            "menu.new" => {
                cx.cmds.push(Cmd::NewGame);
                Nav::Stay
            }
            "menu.load" => Nav::Push(Box::new(super::LoadScreen::new())),
            "menu.settings" => Nav::Push(Box::new(super::SettingsScreen::new())),
            "menu.mods" => Nav::Push(Box::new(super::ModsScreen::new())),
            _ => {
                cx.cmds.push(Cmd::Quit);
                Nav::Stay
            }
        }
    }
}

impl Screen for TitleScreen {
    fn name(&self) -> &'static str {
        "title"
    }
    fn opaque(&self) -> bool {
        false
    }
    fn transition(&self) -> Transition {
        Transition::Fade
    }

    fn update(&mut self, cx: &mut Ctx, inp: &crate::input::Input, dt: f32) -> Nav {
        self.intro += dt;
        if !self.queried {
            self.queried = true;
            self.has_save = !cx.host.saves().is_empty();
            self.focus = if self.has_save { 0 } else { 1 };
            self.data = cx.host.game_dir().map(|p| p.display().to_string()).unwrap_or_default();
        }
        let n = ITEMS.len();
        let mut nav = Nav::Stay;
        for act in &inp.pressed {
            match act {
                Action::Up | Action::Down => {
                    let d = if *act == Action::Up { n - 1 } else { 1 };
                    let mut f = (self.focus + d) % n;
                    while !self.enabled(f) {
                        f = (f + d) % n;
                    }
                    self.focus = f;
                }
                Action::Confirm => nav = self.activate(cx),
                _ => {}
            }
        }
        for i in 0..n {
            if Self::item_rect(i, cx.u, cx.h).contains(Point::new(inp.mouse.0 as f64, inp.mouse.1 as f64)) && self.enabled(i) {
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
        let (w, h, u, t) = (cx.w, cx.h, cx.u, cx.time);
        let par = cx.parallax();
        if !cx.has_world {
            theme::ink_backdrop(sc, w, h, t);
        }
        let intro = ease_out_cubic((self.intro / 1.2).min(1.0));
        theme::moon(sc, Point::new((w * 0.80 - par.0 * 10.0 * u) as f64, (h * 0.22 - par.1 * 6.0 * u) as f64), 74.0 * u, 0.9 * intro);
        theme::mountains(sc, w, h, t, par, 0.9);
        theme::mist(sc, w, h, t, 1.0);
        theme::legibility_scrim(sc, w, h, 1.0);
        theme::spirits(sc, w, h, t, u, 0.9);

        // ---- title block ----
        let x = 120.0 * u;
        let ty = (h / u).max(900.0) * 0.5 * u - 210.0 * u;
        let rise = (1.0 - intro) * 24.0 * u;
        let title = cx.s.t("title.game");
        let st = Style::new(if title.is_ascii() { 92.0 } else { 104.0 } * u, GOLD).title().spacing(if title.is_ascii() { 2.0 } else { 10.0 } * u).alpha(intro);
        let line = text::shape(cx.fonts, &st, &title);
        let base = ty + rise;
        // soft glow behind the title
        let glow = Rect::new((x - 20.0 * u) as f64, (base - line.ascent) as f64, (x + line.width + 20.0 * u) as f64, (base + 10.0 * u) as f64);
        sc.draw_blurred_rounded_rect(Affine::IDENTITY, glow, a(GOLD_D, 0.18 * intro), (30.0 * u) as f64, (40.0 * u) as f64);
        let g = Gradient::new_linear((0.0, (base - line.ascent) as f64), (0.0, base as f64)).with_stops([a(theme::rgba(250, 232, 186, 1.0), intro), a(GOLD, intro), a(GOLD_D, intro)]);
        text::paint(sc, cx.fonts, &st, &line, x, base, &Brush::Gradient(g), Affine::IDENTITY);
        // seal at the end of the title
        theme::seal(sc, cx.fonts, Point::new((x + line.width + 70.0 * u) as f64, (base - line.ascent * 0.45) as f64), 70.0 * u, &cx.s.t("title.seal"), intro);
        // rule + subtitle
        let ry = base + 46.0 * u;
        let rule = Rect::new(x as f64, (ry - 0.7 * u) as f64, (x + 520.0 * u * intro) as f64, (ry + 0.7 * u) as f64);
        let rg = Gradient::new_linear((x as f64, 0.0), ((x + 520.0 * u) as f64, 0.0)).with_stops([a(GOLD, 0.9 * intro), a(GOLD, 0.0)]);
        sc.fill(Fill::NonZero, Affine::IDENTITY, &rg, None, &rule);
        theme::diamond(sc, Point::new(x as f64, ry as f64), 6.0 * u, a(GOLD, intro));
        text::draw(sc, cx.fonts, &Style::new(30.0 * u, PAPER_DIM).spacing(14.0 * u).alpha(intro), &cx.s.t("title.sub"), x + 24.0 * u, ry + 52.0 * u);

        // ---- menu ----
        let hf = self.anims.get(self.focus);
        let hy = self.hl;
        for (i, key) in ITEMS.iter().enumerate() {
            let k = ease_out_cubic(((self.intro - 0.35 - i as f32 * 0.07) / 0.45).clamp(0.0, 1.0));
            if k <= 0.0 {
                continue;
            }
            let y = Self::item_y(i, u, h);
            let f = self.anims.get(i);
            let label = cx.s.t(key);
            let enabled = self.enabled(i);
            let st = Style::new(34.0 * u, if enabled { super::lerp_c(PAPER_DIM, PAPER, f) } else { MUTED }).spacing(3.0 * u).alpha(k * if enabled { 1.0 } else { 0.55 });
            let tw = text::measure(cx.fonts, &st, &label);
            if i == self.focus {
                theme::brush_stroke(sc, x - 34.0 * u, hy, tw + 120.0 * u, 40.0 * u, hf * k, a(JADE, 0.55), a(JADE_L, 0.18));
            }
            let dx = f * 16.0 * u - (1.0 - k) * 30.0 * u;
            if f > 0.02 {
                theme::diamond(sc, Point::new((x - 6.0 * u + dx * 0.3) as f64, y as f64), 6.0 * u * f, a(GOLD, f * k));
            }
            text::draw_mid(sc, cx.fonts, &st, &label, x + 18.0 * u + dx, y);
            if !enabled && i == 0 {
                text::draw_mid(sc, cx.fonts, &Style::new(18.0 * u, MUTED).alpha(k * 0.8), &cx.s.t("menu.continue.none"), x + 36.0 * u + tw + dx, y + 2.0 * u);
            }
        }

        // ---- footer ----
        let fa = ease_out_cubic(((self.intro - 0.9) / 0.6).clamp(0.0, 1.0));
        let look = match cx.settings.present.as_str() {
            "shader" => cx.s.t("menu.look.shader"),
            "original" => cx.s.t("menu.look.original"),
            _ => cx.s.t("menu.look.hd"),
        };
        let fst = Style::new(16.0 * u, MUTED).alpha(fa);
        if !self.data.is_empty() {
            let p = super::short_path(cx.fonts, &fst, &self.data, 520.0 * u);
            text::draw(sc, cx.fonts, &fst, &format!("{}  ·  {}", cx.s.f("menu.data", &[("path", p)]), look), x, h - 40.0 * u);
        }
        hints(sc, cx, &[(Action::Up, cx.s.t("hint.move")), (Action::Confirm, cx.s.t("hint.select"))], fa);
    }
}
