//! Unified input: keyboard, mouse and gamepads become abstract [Action]s. Keyboard bindings are rebindable
//! (saved in `newpal.toml` `[input]`); gamepad uses the standard layout (A confirm, B back, Start menu,
//! D-pad / left stick move, LB/RB switch tabs).
use std::collections::{BTreeMap, HashSet};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
    Menu,
    PrevTab,
    NextTab,
}

impl Action {
    pub const ALL: [Action; 9] = [Action::Up, Action::Down, Action::Left, Action::Right, Action::Confirm, Action::Back, Action::Menu, Action::PrevTab, Action::NextTab];
    pub fn id(self) -> &'static str {
        match self {
            Action::Up => "up",
            Action::Down => "down",
            Action::Left => "left",
            Action::Right => "right",
            Action::Confirm => "confirm",
            Action::Back => "back",
            Action::Menu => "menu",
            Action::PrevTab => "prev_tab",
            Action::NextTab => "next_tab",
        }
    }
    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == s)
    }
    /// gamepad button label shown in hints / settings
    pub fn pad_label(self) -> &'static str {
        match self {
            Action::Up => "D-Pad ↑",
            Action::Down => "D-Pad ↓",
            Action::Left => "D-Pad ←",
            Action::Right => "D-Pad →",
            Action::Confirm => "A",
            Action::Back => "B",
            Action::Menu => "Start",
            Action::PrevTab => "LB",
            Action::NextTab => "RB",
        }
    }
}

macro_rules! keys {
    ($($k:ident),* $(,)?) => {
        /// Supported key names (winit `KeyCode` variant names) for `newpal.toml`.
        pub fn key_name(k: KeyCode) -> Option<&'static str> {
            match k { $(KeyCode::$k => Some(stringify!($k)),)* _ => None }
        }
        pub fn key_from_name(s: &str) -> Option<KeyCode> {
            match s { $(stringify!($k) => Some(KeyCode::$k),)* _ => None }
        }
    };
}
keys!(
    KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT,
    KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ, Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
    ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Enter, NumpadEnter, Space, Escape, Backspace, Tab, ShiftLeft, ShiftRight,
    ControlLeft, ControlRight, AltLeft, AltRight, Numpad0, Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7,
    Numpad8, Numpad9, PageUp, PageDown, Home, End, Insert, Delete, Comma, Period, Slash, Semicolon, Quote, BracketLeft,
    BracketRight, Minus, Equal, Backquote, Backslash,
);

/// Short label for a key cap.
pub fn key_label(k: KeyCode) -> String {
    let n = key_name(k).unwrap_or("?");
    match n {
        "ArrowUp" => "↑".into(),
        "ArrowDown" => "↓".into(),
        "ArrowLeft" => "←".into(),
        "ArrowRight" => "→".into(),
        "Escape" => "Esc".into(),
        "Enter" | "NumpadEnter" => "Enter".into(),
        "Backspace" => "⌫".into(),
        "ShiftLeft" | "ShiftRight" => "Shift".into(),
        "ControlLeft" | "ControlRight" => "Ctrl".into(),
        "AltLeft" | "AltRight" => "Alt".into(),
        _ => n.trim_start_matches("Key").trim_start_matches("Digit").to_string(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bindings {
    pub keys: BTreeMap<Action, Vec<KeyCode>>,
}

impl Default for Bindings {
    fn default() -> Self {
        use KeyCode::*;
        let mut keys = BTreeMap::new();
        keys.insert(Action::Up, vec![KeyW, ArrowUp]);
        keys.insert(Action::Down, vec![KeyS, ArrowDown]);
        keys.insert(Action::Left, vec![KeyA, ArrowLeft]);
        keys.insert(Action::Right, vec![KeyD, ArrowRight]);
        keys.insert(Action::Confirm, vec![Enter, Space, NumpadEnter]);
        keys.insert(Action::Back, vec![Escape, Backspace]);
        keys.insert(Action::Menu, vec![Escape, Tab]);
        keys.insert(Action::PrevTab, vec![KeyQ]);
        keys.insert(Action::NextTab, vec![KeyE]);
        Self { keys }
    }
}

impl Bindings {
    pub fn actions_for(&self, k: KeyCode) -> Vec<Action> {
        self.keys.iter().filter(|(_, v)| v.contains(&k)).map(|(a, _)| *a).collect()
    }
    /// Make `k` the primary key of `a` (keeps the old primary as secondary).
    pub fn rebind(&mut self, a: Action, k: KeyCode) {
        let v = self.keys.entry(a).or_default();
        v.retain(|x| *x != k);
        v.insert(0, k);
        v.truncate(2);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Device {
    #[default]
    Keyboard,
    Mouse,
    Gamepad,
}

/// Input of one frame.
#[derive(Clone, Debug, Default)]
pub struct Input {
    /// actions triggered this frame (key down + auto-repeat)
    pub pressed: Vec<Action>,
    pub held: HashSet<Action>,
    pub mouse: (f32, f32),
    pub mouse_moved: bool,
    pub click: bool,
    pub mouse_down: bool,
    pub right_click: bool,
    pub scroll: f32,
    pub device: Device,
    /// raw key pressed this frame (for rebinding capture)
    pub raw_key: Option<KeyCode>,
    /// analog move vector from the left stick (-1..1)
    pub stick: (f32, f32),
}

impl Input {
    pub fn hit(&self, a: Action) -> bool {
        self.pressed.contains(&a)
    }
}

pub struct InputMapper {
    pub bindings: Bindings,
    gilrs: Option<gilrs::Gilrs>,
    cur: Input,
    held_keys: HashSet<KeyCode>,
    pad_held: HashSet<Action>,
    stick: (f32, f32),
    stick_dir: Option<Action>,
    stick_repeat: f32,
    pub gamepad_name: Option<String>,
}

impl InputMapper {
    pub fn new(bindings: Bindings) -> Self {
        let gilrs = gilrs::Gilrs::new().ok();
        let gamepad_name = gilrs.as_ref().and_then(|g| g.gamepads().next().map(|(_, p)| p.name().to_string()));
        Self { bindings, gilrs, cur: Input::default(), held_keys: HashSet::new(), pad_held: HashSet::new(), stick: (0.0, 0.0), stick_dir: None, stick_repeat: 0.0, gamepad_name }
    }

    /// Headless / tests: no gamepad backend.
    pub fn headless(bindings: Bindings) -> Self {
        Self { bindings, gilrs: None, cur: Input::default(), held_keys: HashSet::new(), pad_held: HashSet::new(), stick: (0.0, 0.0), stick_dir: None, stick_repeat: 0.0, gamepad_name: None }
    }

    pub fn press(&mut self, a: Action) {
        self.cur.pressed.push(a);
    }

    pub fn on_key(&mut self, code: KeyCode, down: bool, repeat: bool) {
        self.cur.device = Device::Keyboard;
        if down {
            self.held_keys.insert(code);
            if !repeat {
                self.cur.raw_key = Some(code);
            }
            for a in self.bindings.actions_for(code) {
                // movement keys repeat (menu navigation); confirm/back/menu fire once
                if !repeat || matches!(a, Action::Up | Action::Down | Action::Left | Action::Right) {
                    self.cur.pressed.push(a);
                }
            }
        } else {
            self.held_keys.remove(&code);
        }
    }

    pub fn on_window_event(&mut self, ev: &WindowEvent) {
        match ev {
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.on_key(code, event.state == ElementState::Pressed, event.repeat);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cur.mouse = (position.x as f32, position.y as f32);
                self.cur.mouse_moved = true;
                self.cur.device = Device::Mouse;
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.cur.device = Device::Mouse;
                match (button, state) {
                    (MouseButton::Left, ElementState::Pressed) => {
                        self.cur.click = true;
                        self.cur.mouse_down = true;
                    }
                    (MouseButton::Left, ElementState::Released) => self.cur.mouse_down = false,
                    (MouseButton::Right, ElementState::Pressed) => self.cur.right_click = true,
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.cur.scroll += match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
            }
            _ => {}
        }
    }

    fn pad_action(b: gilrs::Button) -> Option<Action> {
        use gilrs::Button::*;
        Some(match b {
            DPadUp => Action::Up,
            DPadDown => Action::Down,
            DPadLeft => Action::Left,
            DPadRight => Action::Right,
            South => Action::Confirm,
            East => Action::Back,
            Start => Action::Menu,
            LeftTrigger => Action::PrevTab,
            RightTrigger => Action::NextTab,
            _ => return None,
        })
    }

    fn poll_gamepad(&mut self, dt: f32) {
        let Some(g) = self.gilrs.as_mut() else { return };
        while let Some(ev) = g.next_event() {
            match ev.event {
                gilrs::EventType::ButtonPressed(b, _) => {
                    if let Some(a) = Self::pad_action(b) {
                        self.cur.pressed.push(a);
                        self.pad_held.insert(a);
                        self.cur.device = Device::Gamepad;
                    }
                }
                gilrs::EventType::ButtonRepeated(b, _) => {
                    if let Some(a) = Self::pad_action(b) {
                        if matches!(a, Action::Up | Action::Down | Action::Left | Action::Right) {
                            self.cur.pressed.push(a);
                        }
                    }
                }
                gilrs::EventType::ButtonReleased(b, _) => {
                    if let Some(a) = Self::pad_action(b) {
                        self.pad_held.remove(&a);
                    }
                }
                gilrs::EventType::AxisChanged(ax, v, _) => match ax {
                    gilrs::Axis::LeftStickX => self.stick.0 = v,
                    gilrs::Axis::LeftStickY => self.stick.1 = -v,
                    _ => {}
                },
                gilrs::EventType::Connected => {
                    self.gamepad_name = Some(g.gamepad(ev.id).name().to_string());
                }
                _ => {}
            }
        }
        // stick -> discrete navigation with repeat (0.35 s first, then every 0.11 s)
        let (x, y) = self.stick;
        let dir = if x.abs().max(y.abs()) < 0.5 {
            None
        } else if x.abs() > y.abs() {
            Some(if x > 0.0 { Action::Right } else { Action::Left })
        } else {
            Some(if y > 0.0 { Action::Down } else { Action::Up })
        };
        if dir != self.stick_dir {
            self.stick_dir = dir;
            if let Some(a) = dir {
                self.cur.pressed.push(a);
                self.cur.device = Device::Gamepad;
            }
            self.stick_repeat = 0.35;
        } else if let Some(a) = dir {
            self.stick_repeat -= dt;
            if self.stick_repeat <= 0.0 {
                self.cur.pressed.push(a);
                self.stick_repeat = 0.11;
            }
        }
    }

    /// Collect this frame's input and reset the per-frame fields.
    pub fn frame(&mut self, dt: f32) -> Input {
        self.poll_gamepad(dt);
        let mut held: HashSet<Action> = self.held_keys.iter().flat_map(|k| self.bindings.actions_for(*k)).collect();
        held.extend(self.pad_held.iter().copied());
        if let Some(a) = self.stick_dir {
            held.insert(a);
        }
        let mut out = std::mem::take(&mut self.cur);
        out.held = held;
        out.stick = self.stick;
        // keep persistent fields
        self.cur.mouse = out.mouse;
        self.cur.mouse_down = out.mouse_down;
        self.cur.device = out.device;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_names_roundtrip() {
        for k in [KeyCode::KeyW, KeyCode::ArrowUp, KeyCode::Enter, KeyCode::Escape, KeyCode::Digit5] {
            assert_eq!(key_from_name(key_name(k).unwrap()), Some(k));
        }
    }
    #[test]
    fn rebind_keeps_secondary() {
        let mut b = Bindings::default();
        b.rebind(Action::Up, KeyCode::KeyI);
        assert_eq!(b.keys[&Action::Up], vec![KeyCode::KeyI, KeyCode::KeyW]);
    }
    #[test]
    fn keyboard_maps_to_actions() {
        let mut m = InputMapper::headless(Bindings::default());
        m.on_key(KeyCode::ArrowDown, true, false);
        let f = m.frame(0.016);
        assert!(f.hit(Action::Down));
        assert!(f.held.contains(&Action::Down));
        m.on_key(KeyCode::ArrowDown, false, false);
        let f = m.frame(0.016);
        assert!(!f.held.contains(&Action::Down) && f.pressed.is_empty());
    }
}
