//! Settings: tabs (display / audio / language / controls / game data), every change applies live and is
//! written to `newpal.toml` when the screen closes.
use super::{dim, hints, rect, short_path, Anims};
use crate::anim::{approach, ease_out_cubic};
use crate::host::{DirCheck, HdState, LangEntry};
use crate::input::{key_label, Action, Bindings, Input};
use crate::scene::{Cmd, Ctx, DrawCx, Nav, Screen};
use crate::settings::{WindowMode, PRESENTS, REFRESH_TARGETS};
use crate::strings::Strings;
use crate::text::{self, Align, Style};
use crate::theme::{self, a, GOLD, INK_2, JADE, JADE_D, JADE_L, MUTED, OK, PAPER, PAPER_DIM, WARN};
use std::path::PathBuf;
use vello::kurbo::{Affine, BezPath, Circle, Point, Rect, RoundedRect, Stroke};
use vello::peniko::{Fill, Gradient};
use vello::Scene;
use winit::keyboard::KeyCode;

const TABS: [&str; 5] = ["tab.display", "tab.audio", "tab.language", "tab.controls", "tab.data"];

enum Kind {
    Choice(Vec<String>, usize),
    Slider(f32, String),
    Toggle(bool),
    Bind(Action),
    Button(String),
    Info(String, Option<vello::peniko::Color>),
}

struct Row {
    id: &'static str,
    label: String,
    kind: Kind,
    desc: Option<String>,
}

#[derive(Default)]
struct Snap {
    res: Vec<(u32, u32)>,
    langs: Vec<LangEntry>,
    hz: Option<u32>,
    dir: Option<PathBuf>,
    check: Option<DirCheck>,
    hd: Option<HdState>,
}

pub struct SettingsScreen {
    tab: usize,
    row: usize,
    tab_x: f32,
    row_y: f32,
    anims: Anims,
    snap: Snap,
    snapped: bool,
    capture: Option<Action>,
    capture_armed: bool,
    t: f32,
    dragging: bool,
}

impl Default for SettingsScreen {
    fn default() -> Self {
        Self::new()
    }
}

fn pct(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}

impl SettingsScreen {
    pub fn new() -> Self {
        Self { tab: 0, row: 0, tab_x: -1.0, row_y: -1.0, anims: Anims::default(), snap: Snap::default(), snapped: false, capture: None, capture_armed: false, t: 0.0, dragging: false }
    }
    pub fn with_tab(tab: usize) -> Self {
        let mut s = Self::new();
        s.tab = tab.min(TABS.len() - 1);
        s
    }

    fn panel(w: f32, h: f32, u: f32) -> Rect {
        let pw = (1500.0 * u).min(w - 60.0 * u);
        let ph = (880.0 * u).min(h - 60.0 * u);
        rect((w - pw) / 2.0, (h - ph) / 2.0, pw, ph)
    }
    fn tab_rect(i: usize, p: Rect, u: f32, fonts: &crate::text::Fonts, s: &Strings) -> Rect {
        let st = Style::new(24.0 * u, PAPER);
        let mut x = p.x0 as f32 + 60.0 * u;
        for j in 0..i {
            x += text::measure(fonts, &st, &s.t(TABS[j])) + 56.0 * u;
        }
        let w = text::measure(fonts, &st, &s.t(TABS[i]));
        rect(x - 14.0 * u, p.y0 as f32 + 120.0 * u, w + 28.0 * u, 48.0 * u)
    }
    fn row_rect(i: usize, p: Rect, u: f32) -> Rect {
        rect(p.x0 as f32 + 48.0 * u, p.y0 as f32 + 206.0 * u + i as f32 * 70.0 * u, p.width() as f32 - 96.0 * u, 62.0 * u)
    }
    /// value widget area of a row
    fn value_rect(r: Rect, u: f32) -> Rect {
        Rect::new(r.x1 - (560.0 * u) as f64, r.y0, r.x1 - (20.0 * u) as f64, r.y1)
    }

    fn rows(&self, s: &Strings, st: &crate::settings::Settings) -> Vec<Row> {
        let mut v = Vec::new();
        match self.tab {
            0 => {
                let modes: Vec<String> = WindowMode::ALL.iter().map(|m| s.t(&format!("opt.{}", m.id()))).collect();
                let mi = WindowMode::ALL.iter().position(|m| *m == st.window_mode).unwrap_or(0);
                v.push(Row { id: "mode", label: s.t("set.window_mode"), kind: Kind::Choice(modes, mi), desc: None });
                let mut res = self.snap.res.clone();
                if !res.contains(&st.resolution) {
                    res.push(st.resolution);
                    res.sort();
                }
                let ri = res.iter().position(|r| *r == st.resolution).unwrap_or(0);
                v.push(Row { id: "res", label: s.t("set.resolution"), kind: Kind::Choice(res.iter().map(|(w, h)| format!("{w} × {h}")).collect(), ri), desc: Some(s.t("set.resolution.desc")) });
                let hz: Vec<String> = REFRESH_TARGETS
                    .iter()
                    .map(|&r| if r == 0 { self.snap.hz.map(|h| s.f("opt.refresh_display", &[("hz", h.to_string())])).unwrap_or_else(|| s.t("opt.refresh_display_unknown")) } else { s.f("opt.hz", &[("hz", r.to_string())]) })
                    .collect();
                let hi = REFRESH_TARGETS.iter().position(|r| *r == st.refresh).unwrap_or(0);
                v.push(Row { id: "refresh", label: s.t("set.refresh"), kind: Kind::Choice(hz, hi), desc: Some(s.t("set.refresh.desc")) });
                v.push(Row { id: "vsync", label: s.t("set.vsync"), kind: Kind::Toggle(st.vsync), desc: None });
                let pi = PRESENTS.iter().position(|p| *p == st.present).unwrap_or(0);
                v.push(Row { id: "present", label: s.t("set.present"), kind: Kind::Choice(PRESENTS.iter().map(|p| s.t(&format!("opt.{p}"))).collect(), pi), desc: Some(s.t("set.present.desc")) });
                v.push(Row { id: "ui_scale", label: s.t("set.ui_scale"), kind: Kind::Slider((st.ui_scale - 0.75) / 0.75, pct(st.ui_scale)), desc: None });
            }
            1 => {
                v.push(Row { id: "master", label: s.t("set.master"), kind: Kind::Slider(st.master, pct(st.master)), desc: None });
                v.push(Row { id: "music", label: s.t("set.music"), kind: Kind::Slider(st.music, pct(st.music)), desc: None });
                v.push(Row { id: "sfx", label: s.t("set.sfx"), kind: Kind::Slider(st.sfx, pct(st.sfx)), desc: None });
            }
            2 => {
                let cur = s.locale();
                let li = self.snap.langs.iter().position(|l| l.tag == cur).unwrap_or(0);
                v.push(Row { id: "lang", label: s.t("set.language"), kind: Kind::Choice(self.snap.langs.iter().map(|l| l.name.clone()).collect(), li), desc: Some(s.t("set.language.desc")) });
            }
            3 => {
                for act in Action::ALL {
                    v.push(Row { id: act.id(), label: s.t(&format!("act.{}", act.id())), kind: Kind::Bind(act), desc: None });
                }
                v.push(Row { id: "reset_binds", label: String::new(), kind: Kind::Button(s.t("set.bind.reset")), desc: None });
            }
            _ => {
                let dir = self.snap.dir.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| s.t("set.not_set"));
                v.push(Row { id: "dir", label: s.t("set.game_dir"), kind: Kind::Info(dir, None), desc: Some(s.t("launcher.readonly")) });
                if let Some(c) = &self.snap.check {
                    let (txt, col) = if c.exact { (s.t("launcher.check.exact"), OK) } else if c.usable { (s.t("launcher.check.usable"), WARN) } else { (s.f("launcher.check.bad", &[("n", c.missing.len().to_string())]), theme::BAD) };
                    v.push(Row { id: "verify", label: s.t("set.verify"), kind: Kind::Info(txt, Some(col)), desc: None });
                }
                v.push(Row { id: "change_dir", label: String::new(), kind: Kind::Button(s.t("set.change_dir")), desc: None });
                let hd = match &self.snap.hd {
                    Some(HdState::Ready { items }) => s.f("hd.ready", &[("n", items.to_string())]),
                    Some(HdState::Installing { done, total, .. }) => format!("{} · {}", s.t("hd.installing"), s.f("hd.progress", &[("done", done.to_string()), ("total", total.to_string())])),
                    Some(HdState::Failed(e)) => s.f("hd.failed", &[("e", e.clone())]),
                    _ => s.t("hd.missing"),
                };
                v.push(Row { id: "hd", label: s.t("set.hd"), kind: Kind::Button(hd), desc: Some(s.t("hd.missing_body")) });
            }
        }
        v
    }

    fn refresh_snap(&mut self, cx: &mut Ctx) {
        self.snap.res = cx.host.resolutions();
        self.snap.langs = cx.host.languages();
        self.snap.hz = cx.host.display_hz();
        self.snap.dir = cx.host.game_dir();
        self.snap.check = self.snap.dir.clone().map(|d| cx.host.check_dir(&d));
        self.snap.hd = Some(cx.host.hd_state());
    }

    /// Change a value by `d` (-1 / +1), or activate (d = 0).
    fn adjust(&mut self, cx: &mut Ctx, row: &Row, d: i32) -> Nav {
        let st = &mut *cx.settings;
        let step = |i: usize, n: usize| -> usize {
            if n == 0 {
                0
            } else if d == 0 { (i + 1) % n.max(1) } else { (i as i32 + d).clamp(0, n as i32 - 1) as usize }
        };
        let mut changed = true;
        match (&row.kind, row.id) {
            (Kind::Choice(opts, i), "mode") => st.window_mode = WindowMode::ALL[step(*i, opts.len())],
            (Kind::Choice(opts, i), "res") => {
                let mut res = self.snap.res.clone();
                if !res.contains(&st.resolution) {
                    res.push(st.resolution);
                    res.sort();
                }
                if let Some(r) = res.get(step(*i, opts.len())) {
                    st.resolution = *r;
                }
            }
            (Kind::Choice(opts, i), "refresh") => st.refresh = REFRESH_TARGETS[step(*i, opts.len())],
            (Kind::Choice(opts, i), "present") => st.present = PRESENTS[step(*i, opts.len())].to_string(),
            (Kind::Toggle(b), "vsync") => st.vsync = !*b,
            (Kind::Slider(..), id) => {
                let delta = if d == 0 { 0.0 } else { d as f32 * 0.05 };
                match id {
                    "ui_scale" => st.ui_scale = (st.ui_scale + delta).clamp(0.75, 1.5),
                    "master" => st.master = (st.master + delta).clamp(0.0, 1.0),
                    "music" => st.music = (st.music + delta).clamp(0.0, 1.0),
                    "sfx" => st.sfx = (st.sfx + delta).clamp(0.0, 1.0),
                    _ => {}
                }
            }
            (Kind::Choice(opts, i), "lang") => {
                if let Some(l) = self.snap.langs.get(step(*i, opts.len())) {
                    st.language = Some(l.tag.clone());
                    cx.cmds.push(Cmd::SetLanguage(l.tag.clone()));
                }
            }
            (Kind::Bind(act), _) if d == 0 => {
                self.capture = Some(*act);
                self.capture_armed = false;
                changed = false;
            }
            (Kind::Button(_), "reset_binds") if d == 0 => st.bindings = Bindings::default(),
            (Kind::Button(_), "change_dir") if d == 0 => {
                changed = false;
                if let Some(p) = cx.host.pick_dir() {
                    let chk = cx.host.check_dir(&p);
                    if chk.usable {
                        cx.cmds.push(Cmd::UseGameDir(p));
                    }
                    self.snap.check = Some(chk);
                }
            }
            (Kind::Button(_), "hd") if d == 0 => return Nav::Push(Box::new(super::LauncherScreen::hd_only())),
            _ => changed = false,
        }
        if changed {
            cx.cmds.push(Cmd::ApplySettings);
        }
        Nav::Stay
    }
}

impl Screen for SettingsScreen {
    fn name(&self) -> &'static str {
        "settings"
    }

    fn update(&mut self, cx: &mut Ctx, inp: &Input, dt: f32) -> Nav {
        self.t += dt;
        if !self.snapped {
            self.snapped = true;
            self.refresh_snap(cx);
        }
        // key capture for rebinding
        if let Some(act) = self.capture {
            if self.capture_armed {
                if let Some(k) = inp.raw_key {
                    if k != KeyCode::Escape {
                        cx.settings.bindings.rebind(act, k);
                        cx.cmds.push(Cmd::ApplySettings);
                    }
                    self.capture = None;
                }
            }
            self.capture_armed = true;
            return Nav::Stay;
        }
        let p = Self::panel(cx.w, cx.h, cx.u);
        let rows = self.rows(cx.s, cx.settings);
        if self.row >= rows.len() {
            self.row = rows.len().saturating_sub(1);
        }
        let mut nav = Nav::Stay;
        for act in inp.pressed.clone() {
            match act {
                Action::Back => {
                    cx.cmds.push(Cmd::SaveSettings);
                    return Nav::Pop;
                }
                Action::PrevTab => {
                    self.tab = (self.tab + TABS.len() - 1) % TABS.len();
                    self.row = 0;
                    self.refresh_snap(cx);
                }
                Action::NextTab => {
                    self.tab = (self.tab + 1) % TABS.len();
                    self.row = 0;
                    self.refresh_snap(cx);
                }
                Action::Up if self.row > 0 => self.row -= 1,
                Action::Down if self.row + 1 < rows.len() => self.row += 1,
                Action::Left | Action::Right => {
                    if let Some(r) = rows.get(self.row) {
                        nav = self.adjust(cx, r, if act == Action::Left { -1 } else { 1 });
                    }
                }
                Action::Confirm => {
                    if let Some(r) = rows.get(self.row) {
                        nav = self.adjust(cx, r, 0);
                    }
                }
                _ => {}
            }
        }
        // mouse
        for i in 0..TABS.len() {
            if inp.click && super::hit(Self::tab_rect(i, p, cx.u, cx.fonts, cx.s), inp.mouse) && i != self.tab {
                self.tab = i;
                self.row = 0;
                self.refresh_snap(cx);
                return Nav::Stay;
            }
        }
        if !inp.mouse_down {
            self.dragging = false;
        }
        for (i, r) in rows.iter().enumerate() {
            let rr = Self::row_rect(i, p, cx.u);
            if super::hit(rr, inp.mouse) {
                if inp.mouse_moved {
                    self.row = i;
                }
                let vr = Self::value_rect(rr, cx.u);
                if let Kind::Slider(..) = r.kind {
                    if (inp.click || (self.dragging && inp.mouse_down)) && super::hit(vr, inp.mouse) {
                        self.dragging = true;
                        self.row = i;
                        let bar_x0 = vr.x0 as f32 + 20.0 * cx.u;
                        let bar_w = vr.width() as f32 - 120.0 * cx.u;
                        let f = ((inp.mouse.0 - bar_x0) / bar_w).clamp(0.0, 1.0);
                        let st = &mut *cx.settings;
                        match r.id {
                            "ui_scale" => st.ui_scale = ((0.75 + f * 0.75) * 20.0).round() / 20.0,
                            "master" => st.master = (f * 100.0).round() / 100.0,
                            "music" => st.music = (f * 100.0).round() / 100.0,
                            "sfx" => st.sfx = (f * 100.0).round() / 100.0,
                            _ => {}
                        }
                        cx.cmds.push(Cmd::ApplySettings);
                        continue;
                    }
                }
                if inp.click {
                    self.row = i;
                    let d = if matches!(r.kind, Kind::Choice(..)) && super::hit(vr, inp.mouse) {
                        if (inp.mouse.0 as f64) < vr.center().x { -1 } else { 1 }
                    } else {
                        0
                    };
                    nav = self.adjust(cx, r, d);
                }
            }
        }
        if inp.scroll.abs() > 0.5 {
            if let Some(r) = rows.get(self.row) {
                if matches!(r.kind, Kind::Slider(..)) {
                    nav = self.adjust(cx, r, if inp.scroll > 0.0 { 1 } else { -1 });
                }
            }
        }
        self.anims.tick(rows.len(), Some(self.row), dt);
        let tr = Self::tab_rect(self.tab, p, cx.u, cx.fonts, cx.s);
        if self.tab_x < 0.0 {
            self.tab_x = tr.x0 as f32;
        }
        approach(&mut self.tab_x, tr.x0 as f32, 18.0, dt);
        let ry = Self::row_rect(self.row, p, cx.u).y0 as f32;
        if self.row_y < 0.0 {
            self.row_y = ry;
        }
        approach(&mut self.row_y, ry, 20.0, dt);
        if self.t > 0.0 && (self.t * 2.0).fract() < dt * 2.0 && self.tab == 4 {
            self.snap.hd = Some(cx.host.hd_state());
        }
        nav
    }

    fn draw(&self, sc: &mut Scene, cx: &DrawCx) {
        let u = cx.u;
        dim(sc, cx, 0.55);
        let p = Self::panel(cx.w, cx.h, u);
        theme::panel(sc, p, u, 1.0);
        let (px, py, pw) = (p.x0 as f32, p.y0 as f32, p.width() as f32);
        super::header(sc, cx.fonts, &cx.s.t("settings.title"), px + 60.0 * u, py + 70.0 * u, pw - 120.0 * u, u, 1.0);

        // tabs + sliding indicator
        let tst = Style::new(24.0 * u, PAPER);
        for (i, key) in TABS.iter().enumerate() {
            let r = Self::tab_rect(i, p, u, cx.fonts, cx.s);
            let col = if i == self.tab { PAPER } else { PAPER_DIM };
            text::draw_mid(sc, cx.fonts, &Style { color: col, ..tst }.align(Align::Center), &cx.s.t(key), r.center().x as f32, r.center().y as f32);
        }
        let cur = Self::tab_rect(self.tab, p, u, cx.fonts, cx.s);
        let iw = cur.width() as f32;
        let iy = cur.y1 as f32 + 2.0 * u;
        let g = Gradient::new_linear((self.tab_x as f64, 0.0), ((self.tab_x + iw) as f64, 0.0)).with_stops([a(JADE, 0.0), JADE_L, a(JADE, 0.0)]);
        sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &rect(self.tab_x, iy, iw, 2.5 * u));
        theme::fill_rect(sc, rect(px + 48.0 * u, iy + 1.0 * u, pw - 96.0 * u, 0.8 * u), a(MUTED, 0.35));
        // LB / RB hints beside the tabs
        let last = Self::tab_rect(TABS.len() - 1, p, u, cx.fonts, cx.s);
        if cx.device == crate::input::Device::Gamepad {
            theme::pad_button(sc, cx.fonts, last.x1 as f32 + 30.0 * u, cur.center().y as f32, "LB", u, 0.8);
            theme::pad_button(sc, cx.fonts, last.x1 as f32 + 84.0 * u, cur.center().y as f32, "RB", u, 0.8);
        } else {
            let q = cx.settings.bindings.keys.get(&Action::PrevTab).and_then(|v| v.first()).map(|k| key_label(*k)).unwrap_or_default();
            let e = cx.settings.bindings.keys.get(&Action::NextTab).and_then(|v| v.first()).map(|k| key_label(*k)).unwrap_or_default();
            let w1 = theme::keycap(sc, cx.fonts, last.x1 as f32 + 30.0 * u, cur.center().y as f32, &q, u, 0.7);
            theme::keycap(sc, cx.fonts, last.x1 as f32 + 40.0 * u + w1, cur.center().y as f32, &e, u, 0.7);
        }

        // rows
        let rows = self.rows(cx.s, cx.settings);
        let hr = rect(px + 48.0 * u, self.row_y, pw - 96.0 * u, 62.0 * u);
        let hg = Gradient::new_linear((hr.x0, 0.0), (hr.x1, 0.0)).with_stops([a(JADE_D, 0.55), a(JADE_D, 0.18), a(JADE_D, 0.05)]);
        sc.fill(Fill::NonZero, Affine::IDENTITY, &hg, None, &RoundedRect::from_rect(hr, (8.0 * u) as f64));
        theme::fill_rect(sc, rect(px + 48.0 * u, self.row_y + 10.0 * u, 3.0 * u, 42.0 * u), JADE_L);
        for (i, r) in rows.iter().enumerate() {
            let rr = Self::row_rect(i, p, u);
            let f = self.anims.get(i);
            let k = ease_out_cubic(((self.t - i as f32 * 0.03) / 0.3).clamp(0.0, 1.0));
            let cy = rr.center().y as f32;
            let lst = Style::new(24.0 * u, super::lerp_c(PAPER_DIM, PAPER, f)).alpha(k);
            text::draw_mid(sc, cx.fonts, &lst, &r.label, rr.x0 as f32 + 28.0 * u, cy);
            let vr = Self::value_rect(rr, u);
            self.draw_value(sc, cx, r, vr, f, k);
        }
        // description of the focused row
        if let Some(r) = rows.get(self.row) {
            if let Some(d) = &r.desc {
                let st = Style::new(19.0 * u, PAPER_DIM);
                let y0 = p.y1 as f32 - 128.0 * u;
                theme::fill_rect(sc, rect(px + 48.0 * u, y0 - 26.0 * u, pw - 96.0 * u, 0.8 * u), a(MUTED, 0.3));
                for (i, l) in text::wrap(cx.fonts, &st, d, pw - 140.0 * u).iter().take(2).enumerate() {
                    text::draw(sc, cx.fonts, &st, l, px + 76.0 * u, y0 + 6.0 * u + i as f32 * 28.0 * u);
                }
            }
        }
        if self.capture.is_some() {
            dim(sc, cx, 0.5);
            let bx = rect(cx.w / 2.0 - 360.0 * u, cx.h / 2.0 - 80.0 * u, 720.0 * u, 160.0 * u);
            theme::panel(sc, bx, u, 1.0);
            let pulse = 0.6 + 0.4 * (cx.time * 4.0).sin();
            text::draw_mid(sc, cx.fonts, &Style::new(28.0 * u, a(PAPER, pulse)).align(Align::Center), &cx.s.t("set.bind.press"), bx.center().x as f32, bx.center().y as f32);
        }
        hints(sc, cx, &[(Action::Left, cx.s.t("hint.change")), (Action::Confirm, cx.s.t("hint.select")), (Action::Back, cx.s.t("hint.back"))], 1.0);
    }
}

impl SettingsScreen {
    fn draw_value(&self, sc: &mut Scene, cx: &DrawCx, r: &Row, vr: Rect, f: f32, k: f32) {
        let u = cx.u;
        let cy = vr.center().y as f32;
        match &r.kind {
            Kind::Choice(opts, i) => {
                let v = opts.get(*i).cloned().unwrap_or_default();
                text::draw_mid(sc, cx.fonts, &Style::new(23.0 * u, PAPER).align(Align::Center).alpha(k), &v, vr.center().x as f32, cy);
                for (dir, x) in [(-1.0f32, vr.x0 as f32 + 18.0 * u), (1.0, vr.x1 as f32 - 18.0 * u)] {
                    let en = if dir < 0.0 { *i > 0 } else { *i + 1 < opts.len() };
                    let mut chev = BezPath::new();
                    let s = 8.0 * u;
                    chev.move_to(((x - dir * s * 0.5) as f64, (cy - s) as f64));
                    chev.line_to(((x + dir * s * 0.5) as f64, cy as f64));
                    chev.line_to(((x - dir * s * 0.5) as f64, (cy + s) as f64));
                    let col = if en { super::lerp_c(MUTED, JADE_L, f) } else { a(MUTED, 0.3) };
                    sc.stroke(&Stroke::new((2.2 * u) as f64).with_caps(vello::kurbo::Cap::Round).with_join(vello::kurbo::Join::Round), Affine::IDENTITY, a(col, k), None, &chev);
                }
                // option dots
                let n = opts.len().min(12);
                if n > 1 {
                    let w = (n - 1) as f32 * 12.0 * u;
                    for j in 0..n {
                        let x = vr.center().x as f32 - w / 2.0 + j as f32 * 12.0 * u;
                        let col = if j == *i { JADE_L } else { a(MUTED, 0.5) };
                        sc.fill(Fill::NonZero, Affine::IDENTITY, a(col, k), None, &Circle::new((x as f64, (cy + 22.0 * u) as f64), (2.2 * u) as f64));
                    }
                }
            }
            Kind::Slider(v, label) => {
                let x0 = vr.x0 as f32 + 20.0 * u;
                let bw = vr.width() as f32 - 120.0 * u;
                let track = rect(x0, cy - 3.0 * u, bw, 6.0 * u);
                sc.fill(Fill::NonZero, Affine::IDENTITY, a(INK_2, k), None, &RoundedRect::from_rect(track, (3.0 * u) as f64));
                let fw = bw * v.clamp(0.0, 1.0);
                let g = Gradient::new_linear((x0 as f64, 0.0), ((x0 + bw) as f64, 0.0)).with_stops([JADE_D, JADE, JADE_L]);
                sc.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &RoundedRect::from_rect(rect(x0, cy - 3.0 * u, fw, 6.0 * u), (3.0 * u) as f64));
                let kc = Point::new((x0 + fw) as f64, cy as f64);
                sc.draw_blurred_rounded_rect(Affine::IDENTITY, Rect::new(kc.x - 14.0 * u as f64, kc.y - 14.0 * u as f64, kc.x + 14.0 * u as f64, kc.y + 14.0 * u as f64), a(JADE, 0.5 * f * k), 14.0 * u as f64, 6.0 * u as f64);
                theme::diamond(sc, kc, (9.0 + 3.0 * f) * u, a(super::lerp_c(GOLD, theme::rgba(255, 236, 190, 1.0), f), k));
                text::draw_mid(sc, cx.fonts, &Style::new(21.0 * u, PAPER).align(Align::Right).alpha(k), label, vr.x1 as f32, cy);
            }
            Kind::Toggle(on) => {
                let w = 64.0 * u;
                let h = 32.0 * u;
                let r = rect(vr.x1 as f32 - w, cy - h / 2.0, w, h);
                let rr = RoundedRect::from_rect(r, (h / 2.0) as f64);
                sc.fill(Fill::NonZero, Affine::IDENTITY, a(if *on { JADE_D } else { INK_2 }, k), None, &rr);
                sc.stroke(&Stroke::new((1.4 * u) as f64), Affine::IDENTITY, a(if *on { JADE_L } else { MUTED }, k), None, &rr);
                let kx = if *on { r.x1 as f32 - h / 2.0 } else { r.x0 as f32 + h / 2.0 };
                sc.fill(Fill::NonZero, Affine::IDENTITY, a(if *on { PAPER } else { PAPER_DIM }, k), None, &Circle::new((kx as f64, cy as f64), (h * 0.36) as f64));
                let l = cx.s.t(if *on { "opt.on" } else { "opt.off" });
                text::draw_mid(sc, cx.fonts, &Style::new(21.0 * u, PAPER_DIM).align(Align::Right).alpha(k), &l, r.x0 as f32 - 16.0 * u, cy);
            }
            Kind::Bind(act) => {
                let mut x = vr.x0 as f32 + 40.0 * u;
                let keys = cx.settings.bindings.keys.get(act).cloned().unwrap_or_default();
                for kc in keys.iter().take(2) {
                    x += theme::keycap(sc, cx.fonts, x, cy, &key_label(*kc), u * 1.1, k) + 12.0 * u;
                }
                let pl = act.pad_label();
                let pst = Style::new(13.0 * u, PAPER);
                let pw = if pl.chars().count() > 1 { text::measure(cx.fonts, &pst, pl) + 18.0 * u } else { 26.0 * u };
                theme::pad_button(sc, cx.fonts, vr.x1 as f32 - pw, cy, pl, u, 0.85 * k);
            }
            Kind::Button(label) => {
                let st = Style::new(22.0 * u, super::lerp_c(JADE_L, PAPER, f)).alpha(k);
                let tw = text::measure(cx.fonts, &st, label);
                let bx = rect(vr.x1 as f32 - tw - 56.0 * u, cy - 22.0 * u, tw + 56.0 * u, 44.0 * u);
                sc.stroke(&Stroke::new(((1.0 + f) * u) as f64), Affine::IDENTITY, a(JADE, (0.5 + 0.5 * f) * k), None, &RoundedRect::from_rect(bx, (22.0 * u) as f64));
                text::draw_mid(sc, cx.fonts, &st.align(Align::Center), label, bx.center().x as f32, cy);
            }
            Kind::Info(txt, col) => {
                let st = Style::new(20.0 * u, col.unwrap_or(PAPER_DIM)).align(Align::Right).alpha(k);
                let s = short_path(cx.fonts, &st, txt, vr.width() as f32 + 200.0 * u);
                text::draw_mid(sc, cx.fonts, &st, &s, vr.x1 as f32, cy);
            }
        }
    }
}
