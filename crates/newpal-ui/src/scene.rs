//! Screen stack with animated transitions. The launcher, title menu, settings, pause menu ... are all screens
//! on one stack inside the game window, so going from "first run" to "playing" never leaves the window.
use crate::anim::{ease_in_out, ease_out_cubic};
use crate::host::Host;
use crate::input::{Device, Input};
use crate::settings::Settings;
use crate::strings::Strings;
use crate::text::Fonts;
use crate::theme;
use std::path::PathBuf;
use vello::kurbo::{Affine, Point, Rect};
use vello::peniko::{Fill, Mix};
use vello::Scene;

/// Requests from screens to the engine shell.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    /// validated folder chosen in the launcher -> save to config, open the VFS
    UseGameDir(PathBuf),
    /// launcher finished -> load the title background (player's own map) and show the title menu
    EnterTitle,
    NewGame,
    Continue,
    LoadSlot(u32),
    /// settings changed: apply live (window mode, size, vsync, frame target, look, volume, bindings, ui scale)
    ApplySettings,
    SaveSettings,
    SetLanguage(String),
    ToggleMod(String),
    ToTitle,
    Resume,
    Quit,
}

pub struct Ctx<'a> {
    pub s: &'a Strings,
    pub fonts: &'a Fonts,
    pub host: &'a mut dyn Host,
    pub settings: &'a mut Settings,
    pub w: f32,
    pub h: f32,
    /// layout unit: 1.0 at 1920x1080 (x ui_scale)
    pub u: f32,
    pub time: f32,
    pub in_game: bool,
    pub cmds: &'a mut Vec<Cmd>,
}

pub struct DrawCx<'a> {
    pub s: &'a Strings,
    pub fonts: &'a Fonts,
    pub settings: &'a Settings,
    pub w: f32,
    pub h: f32,
    pub u: f32,
    pub time: f32,
    pub device: Device,
    /// the player's map is rendered behind the UI
    pub has_world: bool,
    pub in_game: bool,
    pub mouse: (f32, f32),
}

impl DrawCx<'_> {
    pub fn full(&self) -> Rect {
        Rect::new(0.0, 0.0, self.w as f64, self.h as f64)
    }
    /// mouse parallax in [-1, 1]
    pub fn parallax(&self) -> (f32, f32) {
        ((self.mouse.0 / self.w.max(1.0) - 0.5) * 2.0, (self.mouse.1 / self.h.max(1.0) - 0.5) * 2.0)
    }
}

/// Layout unit for a target size: designs are authored for 1920x1080.
pub fn layout_unit(w: f32, h: f32, ui_scale: f32) -> f32 {
    (w / 1920.0).min(h / 1080.0) * ui_scale
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transition {
    Fade,
    /// fade + rise
    Slide,
    /// organic ink blot grows from a point
    Ink(Point),
}

impl Transition {
    fn secs(self) -> f32 {
        match self {
            Transition::Fade => 0.28,
            Transition::Slide => 0.32,
            Transition::Ink(_) => 0.95,
        }
    }
}

pub enum Nav {
    Stay,
    Push(Box<dyn Screen>),
    Pop,
    Replace(Box<dyn Screen>, Transition),
    /// remove every screen (fade out) - e.g. entering gameplay
    Clear,
}

pub trait Screen {
    fn name(&self) -> &'static str;
    fn update(&mut self, cx: &mut Ctx, inp: &Input, dt: f32) -> Nav;
    fn draw(&self, sc: &mut Scene, cx: &DrawCx);
    /// hides everything below once fully visible
    fn opaque(&self) -> bool {
        false
    }
    fn transition(&self) -> Transition {
        Transition::Slide
    }
}

struct Entry {
    s: Box<dyn Screen>,
    vis: f32,
    target: f32,
    trans: Transition,
    /// replaced: stays drawn (no input) until the screen above is fully visible
    doomed: bool,
}

#[derive(Default)]
pub struct SceneStack {
    entries: Vec<Entry>,
}

impl SceneStack {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn top_name(&self) -> Option<&'static str> {
        self.entries.iter().rev().find(|e| e.target > 0.0 && !e.doomed).map(|e| e.s.name())
    }
    pub fn names(&self) -> Vec<&'static str> {
        self.entries.iter().map(|e| e.s.name()).collect()
    }
    /// any screen visible or animating
    pub fn active(&self) -> bool {
        !self.entries.is_empty()
    }
    pub fn push(&mut self, s: Box<dyn Screen>) {
        let trans = s.transition();
        self.entries.push(Entry { s, vis: 0.0, target: 1.0, trans, doomed: false });
    }
    pub fn push_instant(&mut self, s: Box<dyn Screen>) {
        let trans = s.transition();
        self.entries.push(Entry { s, vis: 1.0, target: 1.0, trans, doomed: false });
    }
    pub fn replace(&mut self, s: Box<dyn Screen>, trans: Transition) {
        if let Some(top) = self.entries.iter_mut().rev().find(|e| !e.doomed && e.target > 0.0) {
            top.doomed = true;
        }
        self.entries.push(Entry { s, vis: 0.0, target: 1.0, trans, doomed: false });
    }
    pub fn clear(&mut self) {
        for e in &mut self.entries {
            e.target = 0.0;
            e.trans = Transition::Fade;
        }
    }

    pub fn update(&mut self, cx: &mut Ctx, inp: &Input, dt: f32) {
        // input only for the top live, fully-arrived-or-arriving screen
        let idx = self.entries.iter().rposition(|e| !e.doomed && e.target > 0.0);
        let empty = Input { mouse: inp.mouse, device: inp.device, ..Default::default() };
        let mut nav = Nav::Stay;
        for (i, e) in self.entries.iter_mut().enumerate() {
            let live = Some(i) == idx && e.vis > 0.35;
            let n = e.s.update(cx, if live { inp } else { &empty }, dt);
            if live {
                nav = n;
            }
        }
        match nav {
            Nav::Stay => {}
            Nav::Push(s) => self.push(s),
            Nav::Pop => {
                if let Some(i) = idx {
                    self.entries[i].target = 0.0;
                }
            }
            Nav::Replace(s, t) => self.replace(s, t),
            Nav::Clear => self.clear(),
        }
        for e in &mut self.entries {
            let step = dt / e.trans.secs();
            if e.vis < e.target {
                e.vis = (e.vis + step).min(e.target);
            } else if e.vis > e.target {
                e.vis = (e.vis - step).max(e.target);
            }
        }
        // drop finished: popped ones at 0, doomed ones once something above them is fully visible
        let mut keep = Vec::with_capacity(self.entries.len());
        let n = self.entries.len();
        let arrived: Vec<bool> = self.entries.iter().map(|e| e.vis >= 1.0 && e.target >= 1.0).collect();
        for (i, e) in self.entries.drain(..).enumerate() {
            let covered = e.doomed && (i + 1..n).any(|j| arrived[j]);
            if (e.target <= 0.0 && e.vis <= 0.0) || covered {
                continue;
            }
            keep.push(e);
        }
        self.entries = keep;
    }

    pub fn draw(&self, sc: &mut Scene, cx: &DrawCx) {
        // start from the highest fully visible opaque screen
        let start = self.entries.iter().rposition(|e| e.s.opaque() && e.vis >= 1.0 && !e.doomed).unwrap_or(0);
        let full = cx.full();
        // an ink wipe in progress: screens below it are cut away inside the blot (so the game frame under the
        // whole UI shows through), the incoming screen is drawn only inside it
        let ink = self.entries.iter().rposition(|e| matches!(e.trans, Transition::Ink(_)) && e.vis < 1.0 && e.target > 0.0);
        let blot = ink.map(|k| match self.entries[k].trans {
            Transition::Ink(o) => theme::ink_blot(cx.w, cx.h, o, self.entries[k].vis),
            _ => unreachable!(),
        });
        for (i, e) in self.entries.iter().enumerate().skip(start) {
            if e.vis <= 0.0 {
                continue;
            }
            let mut sub = Scene::new();
            e.s.draw(&mut sub, cx);
            let below_ink = matches!(ink, Some(k) if i < k);
            if below_ink {
                let mut inv = vello::kurbo::BezPath::from_vec(vello::kurbo::Shape::path_elements(&full, 0.1).collect());
                inv.extend(blot.as_ref().unwrap().elements().iter().copied());
                sc.push_layer(Fill::EvenOdd, Mix::Normal, 1.0, Affine::IDENTITY, &inv);
            }
            match e.trans {
                Transition::Fade => {
                    sc.push_layer(Fill::NonZero, Mix::Normal, ease_in_out(e.vis), Affine::IDENTITY, &full);
                    sc.append(&sub, None);
                    sc.pop_layer();
                }
                Transition::Slide => {
                    let k = ease_out_cubic(e.vis);
                    let dy = (1.0 - k) * 28.0 * cx.u;
                    sc.push_layer(Fill::NonZero, Mix::Normal, k, Affine::IDENTITY, &full);
                    sc.append(&sub, Some(Affine::translate((0.0, dy as f64))));
                    sc.pop_layer();
                }
                Transition::Ink(_) => {
                    if Some(i) == ink {
                        let b = blot.as_ref().unwrap();
                        sc.push_layer(Fill::NonZero, Mix::Normal, 1.0, Affine::IDENTITY, b);
                        sc.append(&sub, None);
                        sc.pop_layer();
                        // wet ink rim
                        sc.stroke(&vello::kurbo::Stroke::new((10.0 * cx.u) as f64), Affine::IDENTITY, theme::a(theme::INK, 0.55 * (1.0 - e.vis)), None, b);
                    } else {
                        sc.append(&sub, None);
                    }
                }
            }
            if below_ink {
                sc.pop_layer();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::NullHost;

    struct T(&'static str, bool);
    impl Screen for T {
        fn name(&self) -> &'static str {
            self.0
        }
        fn update(&mut self, _cx: &mut Ctx, inp: &Input, _dt: f32) -> Nav {
            if self.1 && inp.hit(crate::input::Action::Back) { Nav::Pop } else { Nav::Stay }
        }
        fn draw(&self, _sc: &mut Scene, _cx: &DrawCx) {}
    }

    fn run(st: &mut SceneStack, inp: &Input, frames: usize) {
        let strings = Strings::keys_only("en");
        let fonts = Fonts::load(Vec::new());
        let mut host = NullHost::default();
        let mut settings = Settings::default();
        let mut cmds = Vec::new();
        for _ in 0..frames {
            let mut cx = Ctx { s: &strings, fonts: &fonts, host: &mut host, settings: &mut settings, w: 1920.0, h: 1080.0, u: 1.0, time: 0.0, in_game: false, cmds: &mut cmds };
            st.update(&mut cx, inp, 1.0 / 60.0);
        }
    }

    #[test]
    fn push_pop_replace() {
        let mut st = SceneStack::new();
        st.push(Box::new(T("a", false)));
        st.push(Box::new(T("b", true)));
        run(&mut st, &Input::default(), 30);
        assert_eq!(st.names(), vec!["a", "b"]);
        let back = Input { pressed: vec![crate::input::Action::Back], ..Default::default() };
        run(&mut st, &back, 1);
        run(&mut st, &Input::default(), 30);
        assert_eq!(st.names(), vec!["a"]);
        st.replace(Box::new(T("c", false)), Transition::Ink(Point::new(0.0, 0.0)));
        run(&mut st, &Input::default(), 90);
        assert_eq!(st.names(), vec!["c"]);
        st.clear();
        run(&mut st, &Input::default(), 30);
        assert!(st.is_empty());
    }
}
