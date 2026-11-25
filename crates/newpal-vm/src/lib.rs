//! # newpal-vm
//!
//! A pluggable VM for the original `talk*.dat` bytecode (the original talk-script interpreter).
//!
//! * Executes at byte positions inside the whole talk file, so jumps may leave a script's own slice
//!   (the original seeks to `target + 0x200`). Instructions are decoded on demand with
//!   [`newpal_assets::talk::decode_at`]; undecodable bytes are skipped with the disassembler's resync rule.
//! * Story state ([`VmState`]: flag bits, script variables, current scene, items) outlives a script and is
//!   handed from script to script by the game.
//! * Every opcode can be (re)defined with [`Vm::register`]; unhandled opcodes are counted in
//!   [`VmState::unhandled`] and skipped, so the VM never blocks on unknown semantics.
//! * The engine talks to scripts only through the [`ScriptBackend`] trait (Lua/Rhai can be added later).
//!
//! Implemented semantics (decomp switch labels are `op - 1`):
//! `0x0f` goto u32 · `0x10` jump if role present · `0x15`/`0x16` jump if flag set / clear ·
//! `0x17`/`0x18` jump if all / any flags of a list (0xFF00-terminated) · `0x19`/`0x1a` set / clear flag ·
//! `0x1b` change scene (city, x, y cells, ..) · `0x1d..0x21` / `0xb5..0xb9` var set/add/sub(clamp 0)/
//! jump-if->= / jump-if-<= (i16 / i32 forms) · `0xa1` chain to another script · `0xa3`/`0xa4` jump if
//! current scene != / == · `0xa5`/`0xa6` jump on item count >= / <= · dialogue `SAY*` · `0xc2` movie ·
//! `0x3b`/`0x71` play SFX (all.snd slot w1) · `0x72` play music tracks w1/w2 (maps.dat music ids, w3&3 mode) ·
//! `0x89` back to the scene's music. Audio requests are queued in [`VmState::audio`] (non-blocking).
use newpal_assets::i18n::markup::{self, Cue};
use newpal_assets::talk::{self, Instr, Span, TalkScript};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

/// A dialogue box request.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialogue {
    /// byte position of the instruction relative to the script start (key for mod text overrides)
    pub pos: usize,
    pub op: u16,
    pub speaker: u16,
    /// speaker name from the `<speaker>` markup (original name; the UI maps it through the catalog)
    pub name: Option<String>,
    pub lines: Vec<Vec<Span>>,
    /// non-visual cues (`<page/>`, `<instant/>`, `<speed=N/>`, `<glyph=N/>`, ...) at text positions
    pub cues: Vec<Cue>,
    /// text actually shown (override / converted), as inline markup (see `newpal_assets::i18n::markup`)
    pub raw: String,
    /// text from the player's own data (never changed; key material for translation tools)
    pub original: String,
    /// `raw` came from an override (language pack / mod), not from the original data
    pub overridden: bool,
}

impl Dialogue {
    /// Replace the shown text (runtime language switch) and re-parse speaker, lines and cues.
    pub fn set_text(&mut self, raw: String, overridden: bool) {
        let p = markup::parse(&raw);
        self.name = p.speaker;
        self.lines = p.lines;
        self.cues = p.cues;
        self.raw = raw;
        self.overridden = overridden;
    }
}

/// Rewrites original (non-overridden) dialogue text, e.g. a zh-Hant -> zh-Hans converter.
pub type TextFilter = Arc<dyn Fn(&str) -> String + Send + Sync>;

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Dialogue(Dialogue),
    Movie(String),
    /// op 0x1b: load scene `city` (maps.dat) and put the hero at cell (x, y); `extra` = remaining words
    ChangeScene { city: u16, x: u16, y: u16, extra: [u16; 3] },
    End,
}

/// What a handler wants the VM to do next.
pub enum Flow {
    Continue,
    Emit(Event),
    /// jump to a u32 script target
    Jump(u32),
}

pub type Handler = Box<dyn FnMut(&mut VmState, &Instr) -> Flow + Send>;
/// Loads another script for op 0xa1 (chain).
pub type Loader = Box<dyn FnMut(u32) -> Option<(TalkScript, HashMap<usize, String>)> + Send>;

/// Number of story flag bits (`DAT_0056f7a8`, 0x800 bytes).
pub const FLAGS: usize = 0x800 * 8;

/// Story state shared by all scripts (the original keeps these in globals).
#[derive(Debug, Clone)]
pub struct VmState {
    flags: Vec<u8>,
    /// script variables (`DAT_0056cf00[i]`, i32)
    pub vars: HashMap<u16, i32>,
    /// current scene (`DAT_0053d064`)
    pub city: u16,
    /// item id -> count (inventory is not modelled yet; scripts may test it)
    pub items: HashMap<u32, i32>,
    /// roles present in the current scene (for ops 0x10/0x11)
    pub roles_present: HashSet<u16>,
    /// opcode -> times skipped because no handler exists
    pub unhandled: BTreeMap<u16, usize>,
    pub executed: usize,
    /// bytes skipped by resync
    pub skipped: usize,
    /// audio requests produced by scripts; the engine drains this after each step
    pub audio: Vec<AudioCmd>,
}

/// Script audio request.
#[derive(Clone, Debug, PartialEq)]
pub enum AudioCmd {
    /// all.snd slot (the original mixer call, Mix_PlayChannelTimed)
    Sfx(u16),
    /// music ids (maps.dat music name table); mode 1 = loop, 2 = once (guess)
    Music { a: u16, b: u16, mode: u16 },
    /// return to the current scene's music (op 0x89)
    SceneMusic,
}

impl Default for VmState {
    fn default() -> Self {
        Self {
            flags: vec![0; FLAGS / 8],
            vars: HashMap::new(),
            city: 0,
            items: HashMap::new(),
            roles_present: HashSet::new(),
            unhandled: BTreeMap::new(),
            executed: 0,
            skipped: 0,
            audio: Vec::new(),
        }
    }
}

impl VmState {
    /// New-game state as set by the original initialiser: flags 1,3,4,0x1e,0x46,0x52 set, var[0] = 100.
    pub fn new_game() -> Self {
        let mut s = Self::default();
        for f in [1u32, 3, 4, 0x1e, 0x46, 0x52] {
            s.set_flag(f, true);
        }
        s.vars.insert(0, 100);
        s
    }
    pub fn flag(&self, f: u32) -> bool {
        self.flags.get(f as usize >> 3).is_some_and(|b| b >> (f & 7) & 1 == 1)
    }
    pub fn set_flag(&mut self, f: u32, on: bool) {
        if let Some(b) = self.flags.get_mut(f as usize >> 3) {
            if on { *b |= 1 << (f & 7) } else { *b &= !(1 << (f & 7)) }
        }
    }
    pub fn var(&self, i: u16) -> i32 {
        *self.vars.get(&i).unwrap_or(&0)
    }
    /// Entering a scene (`LoadCity`): visited flag 12000 + city.
    pub fn enter_city(&mut self, city: u16) {
        self.city = city;
        self.set_flag(12000 + city as u32, true);
    }
}

/// Anything that can drive the game's scripted events.
pub trait ScriptBackend {
    fn name(&self) -> &str;
    /// Run until the next event that needs the game (dialogue, movie, scene change) or the end.
    fn next_event(&mut self) -> Event;
}

pub struct Vm {
    pub script_id: u32,
    image: Arc<Vec<u8>>,
    start: usize,
    jump_base: usize,
    pc: usize,
    ended: bool,
    overrides: HashMap<usize, String>,
    text_filter: Option<TextFilter>,
    handlers: HashMap<u16, Handler>,
    loader: Option<Loader>,
    pub state: VmState,
    /// max instructions per `next_event` (guards endless loops waiting on unmodelled state)
    pub budget: usize,
}

fn u32_at(args: &[u16], i: usize) -> Option<u32> {
    Some(*args.get(i)? as u32 | (*args.get(i + 1)? as u32) << 16)
}

impl Vm {
    pub fn new(script: &TalkScript, overrides: HashMap<usize, String>) -> Self {
        Self::with_state(script, overrides, VmState::default())
    }

    pub fn with_state(script: &TalkScript, overrides: HashMap<usize, String>, state: VmState) -> Self {
        let mut vm = Self {
            script_id: script.id,
            image: script.image.clone(),
            start: script.start,
            jump_base: script.jump_base,
            pc: script.start,
            ended: false,
            overrides,
            text_filter: None,
            handlers: HashMap::new(),
            loader: None,
            state,
            budget: 20_000,
        };
        vm.install_defaults();
        vm
    }

    /// Text shown for the dialogue at script-relative position `rel` whose original text is `original`:
    /// override > filter(original) > original. The flag tells whether an override was used.
    pub fn localized(&self, rel: usize, original: &str) -> (String, bool) {
        match self.overrides.get(&rel) {
            Some(o) => (o.clone(), true),
            None => (self.text_filter.as_ref().map(|f| f(original)).unwrap_or_else(|| original.to_string()), false),
        }
    }

    /// Replace the text overrides of the running script (language switch).
    pub fn set_overrides(&mut self, ov: HashMap<usize, String>) {
        self.overrides = ov;
    }

    /// Filter applied to every non-overridden dialogue text.
    pub fn set_text_filter(&mut self, f: Option<TextFilter>) {
        self.text_filter = f;
    }

    pub fn set_loader(&mut self, l: Loader) {
        self.loader = Some(l);
    }

    /// Define or replace the behaviour of an opcode.
    pub fn register(&mut self, op: u16, h: Handler) {
        self.handlers.insert(op, h);
    }

    /// Hand the story state back (when the script ended).
    pub fn into_state(self) -> VmState {
        self.state
    }

    fn install_defaults(&mut self) {
        self.register(0xc2, Box::new(|_, i| match i {
            Instr::Op { text: Some(t), .. } => Flow::Emit(Event::Movie(t.clone())),
            _ => Flow::Continue,
        }));
        for op in [0x402u16, 0x400, 0x401] {
            self.register(op, Box::new(|_, _| Flow::Continue));
        }
        let jump_if = |c: bool, t: Option<u32>| match (c, t) {
            (true, Some(t)) => Flow::Jump(t),
            _ => Flow::Continue,
        };
        self.register(0x0f, Box::new(|_, i| match i {
            Instr::Op { target: Some(t), .. } => Flow::Jump(*t),
            _ => Flow::Continue,
        }));
        self.register(0x10, Box::new(move |s, i| {
            let Instr::Op { args, .. } = i else { return Flow::Continue };
            jump_if(args.first().is_some_and(|r| s.roles_present.contains(r)), u32_at(args, 1))
        }));
        self.register(0x11, Box::new(|_, _| Flow::Continue)); // role hidden: not modelled -> never
        for op in [0x15u16, 0x16] {
            self.register(op, Box::new(move |s, i| {
                let Instr::Op { args, op, .. } = i else { return Flow::Continue };
                let set = args.first().is_some_and(|&f| s.flag(f as u32));
                jump_if(set == (*op == 0x15), u32_at(args, 1))
            }));
        }
        for op in [0x17u16, 0x18] {
            self.register(op, Box::new(move |s, i| {
                let Instr::Op { args, op, .. } = i else { return Flow::Continue };
                let Some(end) = args.iter().position(|&w| w == 0xFF00) else { return Flow::Continue };
                let fl = &args[..end];
                let c = if *op == 0x17 { fl.iter().all(|&f| s.flag(f as u32)) } else { fl.iter().any(|&f| s.flag(f as u32)) };
                jump_if(c, u32_at(args, end + 1))
            }));
        }
        for (op, on) in [(0x19u16, true), (0x1a, false)] {
            self.register(op, Box::new(move |s, i| {
                if let Instr::Op { args, .. } = i {
                    if let Some(&f) = args.first() {
                        s.set_flag(f as u32, on);
                    }
                }
                Flow::Continue
            }));
        }
        self.register(0x1b, Box::new(|s, i| {
            let Instr::Op { args, .. } = i else { return Flow::Continue };
            if args.len() < 6 || args[0] == 1000 || args[0] == s.city {
                return Flow::Continue; // 1000 = "return to previous scene" (not modelled); same scene = no-op
            }
            Flow::Emit(Event::ChangeScene { city: args[0], x: args[1], y: args[2], extra: [args[3], args[4], args[5]] })
        }));
        for op in [0x1du16, 0x1e, 0x1f, 0x20, 0x21, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9] {
            self.register(op, Box::new(move |s, i| {
                let Instr::Op { args, op, .. } = i else { return Flow::Continue };
                let wide = *op > 0x21;
                let Some(&var) = args.first() else { return Flow::Continue };
                let val = if wide { u32_at(args, 1).map(|v| v as i32) } else { args.get(1).map(|&v| v as i16 as i32) };
                let Some(val) = val else { return Flow::Continue };
                let tgt = u32_at(args, if wide { 3 } else { 2 });
                let cur = s.var(var);
                match op & 0xFF {
                    0x1d | 0xb5 => { s.vars.insert(var, val); }
                    0x1e | 0xb6 => { s.vars.insert(var, cur.wrapping_add(val)); }
                    0x1f | 0xb7 => { s.vars.insert(var, cur.wrapping_sub(val).max(0)); }
                    0x20 | 0xb8 => return jump_if(val <= cur, tgt),
                    _ => return jump_if(cur <= val, tgt),
                }
                Flow::Continue
            }));
        }
        for op in [0x3bu16, 0x71] {
            self.register(op, Box::new(|s, i| {
                if let Instr::Op { args, .. } = i {
                    if let Some(&slot) = args.first() {
                        s.audio.push(AudioCmd::Sfx(slot));
                    }
                }
                Flow::Continue
            }));
        }
        self.register(0x72, Box::new(|s, i| {
            if let Instr::Op { args, .. } = i {
                if args.len() >= 3 {
                    let m = args[2] & 3;
                    s.audio.push(AudioCmd::Music { a: args[0], b: args[1], mode: if m == 0 { 1 } else { m } });
                }
            }
            Flow::Continue
        }));
        self.register(0x89, Box::new(|s, _| {
            s.audio.push(AudioCmd::SceneMusic);
            Flow::Continue
        }));
        for op in [0xa3u16, 0xa4] {
            self.register(op, Box::new(move |s, i| {
                let Instr::Op { args, op, .. } = i else { return Flow::Continue };
                let here = args.first().is_some_and(|&c| c == s.city);
                jump_if(here == (*op == 0xa4), u32_at(args, 1))
            }));
        }
        for op in [0xa5u16, 0xa6] {
            self.register(op, Box::new(move |s, i| {
                let Instr::Op { args, op, .. } = i else { return Flow::Continue };
                let (Some(&item), Some(&n)) = (args.first(), args.get(1)) else { return Flow::Continue };
                let have = *s.items.get(&(item as u32)).unwrap_or(&0);
                let n = n as i16 as i32;
                let c = if *op == 0xa5 { have != 0 && n <= have } else { have == 0 || have <= n };
                jump_if(c, u32_at(args, 2))
            }));
        }
    }

    /// Current byte position relative to the script start.
    pub fn pos(&self) -> usize {
        self.pc.wrapping_sub(self.start)
    }

    fn dialogue(&self, i: &Instr) -> Option<Dialogue> {
        let Instr::Op { pos, op, args, text: Some(t), .. } = i else { return None };
        let rel = pos.wrapping_sub(self.start);
        let (raw, overridden) = self.localized(rel, t);
        let p = markup::parse(&raw);
        Some(Dialogue { pos: rel, op: *op, speaker: args.first().copied().unwrap_or(0), name: p.speaker, lines: p.lines, cues: p.cues, raw, original: t.clone(), overridden })
    }

    fn jump(&mut self, t: u32) {
        self.pc = t as usize + self.jump_base;
    }

    fn chain(&mut self, n: u32) -> bool {
        let Some(l) = self.loader.as_mut() else { return false };
        let Some((s, ov)) = l(n) else { return false };
        self.script_id = s.id;
        self.image = s.image.clone();
        self.start = s.start;
        self.jump_base = s.jump_base;
        self.pc = s.start;
        self.overrides = ov;
        true
    }
}

impl ScriptBackend for Vm {
    fn name(&self) -> &str {
        "talk-bytecode"
    }

    fn next_event(&mut self) -> Event {
        let mut budget = self.budget;
        while !self.ended && self.pc < self.image.len() {
            if budget == 0 {
                self.ended = true;
                break;
            }
            budget -= 1;
            let Some(i) = talk::decode_at(&self.image, self.pc) else {
                let q = talk::resync(&self.image, self.pc);
                self.state.skipped += q - self.pc;
                self.pc = q;
                continue;
            };
            self.state.executed += 1;
            let (op, len) = match &i {
                Instr::End { .. } => break,
                Instr::Unknown { .. } => (0, 2),
                Instr::Op { op, len, .. } => (*op, *len),
            };
            self.pc += len.max(2);
            if talk::is_dialogue(op) {
                if let Some(d) = self.dialogue(&i) {
                    return Event::Dialogue(d);
                }
                continue;
            }
            if op == 0xa1 {
                let n = if let Instr::Op { args, .. } = &i { args.first().copied().unwrap_or(0) as u32 } else { 0 };
                if !self.chain(n) {
                    *self.state.unhandled.entry(op).or_default() += 1;
                }
                continue;
            }
            match self.handlers.get_mut(&op) {
                Some(h) => match h(&mut self.state, &i) {
                    Flow::Continue => {}
                    Flow::Jump(t) => self.jump(t),
                    Flow::Emit(e) => return e,
                },
                None => *self.state.unhandled.entry(op).or_default() += 1,
            }
        }
        self.ended = true;
        Event::End
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(code: Vec<u8>) -> TalkScript {
        TalkScript::standalone(1, "test".into(), code)
    }
    fn w(v: &mut Vec<u8>, ws: &[u16]) {
        for x in ws {
            v.extend_from_slice(&x.to_le_bytes());
        }
    }
    fn say(v: &mut Vec<u8>, t: &str) {
        w(v, &[4, 0x2710]);
        v.extend_from_slice(format!("%C5{t}%Q").as_bytes());
    }
    fn text(e: Event) -> String {
        match e {
            Event::Dialogue(d) => d.lines[0][0].text.clone(),
            e => format!("{e:?}"),
        }
    }

    #[test]
    fn dialogue_and_end() {
        // YIELD; SAY2_W speaker 0x2710, hdr "3#", text; END
        let mut c = vec![0x02, 0x04];
        c.extend_from_slice(&[0x04, 0x00, 0x10, 0x27]);
        c.extend_from_slice(b"3#\x00\x00%C6Li:%N%C5Hello%N%C5World%Q");
        c.extend_from_slice(&[0xFF, 0xFF]);
        let mut vm = Vm::new(&script(c), HashMap::new());
        match vm.next_event() {
            Event::Dialogue(d) => {
                assert_eq!(d.speaker, 0x2710);
                assert_eq!(d.name.as_deref(), Some("Li"));
                assert_eq!(d.lines.len(), 2);
                assert_eq!(d.lines[0][0], Span { color: 5, text: "Hello".into() });
            }
            e => panic!("{e:?}"),
        }
        assert_eq!(vm.next_event(), Event::End);
    }

    #[test]
    fn text_filter_and_override_flags() {
        let mut c = vec![0x04, 0x00, 0x10, 0x27];
        c.extend_from_slice(b"%C5Old%Q");
        c.extend_from_slice(&[0x02, 0x04, 0xFF, 0xFF]);
        let mut vm = Vm::new(&script(c), HashMap::new());
        vm.set_text_filter(Some(Arc::new(|s: &str| markup::map_text(s, &|x| x.to_uppercase()))));
        let Event::Dialogue(mut d) = vm.next_event() else { panic!() };
        assert_eq!(d.original, "<color=5>Old</color>"); // the script's %C5 code arrives as markup
        assert_eq!(d.lines[0][0].text, "OLD");
        assert!(!d.overridden);
        // runtime switch of the visible line
        d.set_text("<color=5>Neu</color>".into(), true);
        assert_eq!(d.lines[0][0].text, "Neu");
        assert!(d.overridden);
        let mut ov = HashMap::new();
        ov.insert(0usize, "x".to_string());
        vm.set_overrides(ov);
        assert_eq!(vm.localized(0, "o"), ("x".to_string(), true));
        assert_eq!(vm.localized(2, "o"), ("O".to_string(), false));
    }

    #[test]
    fn text_override_and_custom_handler() {
        let mut c = vec![0x04, 0x00, 0x10, 0x27];
        c.extend_from_slice(b"%C5Old%Q");
        c.extend_from_slice(&[0x02, 0x04, 0xFF, 0xFF]);
        let mut ov = HashMap::new();
        ov.insert(0usize, "<color=5>New</color>".to_string());
        let mut vm = Vm::new(&script(c), ov);
        vm.register(0x402, Box::new(|_, _| Flow::Emit(Event::Movie("yield-hook".into()))));
        match vm.next_event() {
            Event::Dialogue(d) => assert_eq!(d.lines[0][0].text, "New"),
            e => panic!("{e:?}"),
        }
        assert_eq!(vm.next_event(), Event::Movie("yield-hook".into()));
        assert_eq!(vm.next_event(), Event::End);
    }

    /// if flag 7 set -> B, else A; then set flag 7: second run takes branch B.
    #[test]
    fn flag_branch_and_persistence() {
        let mut c = Vec::new();
        w(&mut c, &[0x15, 7, 0, 0]); // target patched below
        say(&mut c, "A");
        w(&mut c, &[0x19, 7, 0xFFFF]);
        let b = c.len() as u32;
        say(&mut c, "B");
        w(&mut c, &[0xFFFF]);
        c[4..8].copy_from_slice(&b.to_le_bytes());
        let s = script(c);
        let mut vm = Vm::new(&s, HashMap::new());
        assert_eq!(text(vm.next_event()), "A");
        assert_eq!(vm.next_event(), Event::End);
        let st = vm.into_state();
        assert!(st.flag(7));
        let mut vm = Vm::with_state(&s, HashMap::new(), st);
        assert_eq!(text(vm.next_event()), "B");
    }

    /// var[3] = 5; var[3] += 2; jump if 7 >= var... (0x20: jump if value <= var)
    #[test]
    fn vars_and_compare() {
        let mut c = Vec::new();
        w(&mut c, &[0x1d, 3, 5, 0x1e, 3, 2]);
        w(&mut c, &[0x20, 3, 7, 0, 0]);
        say(&mut c, "less");
        w(&mut c, &[0xFFFF]);
        let t = c.len() as u32;
        say(&mut c, "ge7");
        w(&mut c, &[0xFFFF]);
        c[18..22].copy_from_slice(&t.to_le_bytes());
        let mut vm = Vm::new(&script(c), HashMap::new());
        assert_eq!(text(vm.next_event()), "ge7");
        assert_eq!(vm.state.var(3), 7);
    }

    #[test]
    fn scene_change_and_city_test() {
        let mut c = Vec::new();
        w(&mut c, &[0xa4, 2, 0, 0]); // in city 2 -> jump to END
        w(&mut c, &[0x1b, 8, 63, 31, 0xFFFF, 0, 6]);
        let end = c.len() as u32;
        w(&mut c, &[0xFFFF]);
        c[4..8].copy_from_slice(&end.to_le_bytes());
        let s = script(c);
        let mut st = VmState::new_game();
        st.enter_city(5);
        let mut vm = Vm::with_state(&s, HashMap::new(), st);
        assert_eq!(vm.next_event(), Event::ChangeScene { city: 8, x: 63, y: 31, extra: [0xFFFF, 0, 6] });
        let mut st = VmState::new_game();
        st.enter_city(2);
        assert!(st.flag(12002) && st.flag(1) && !st.flag(2));
        let mut vm = Vm::with_state(&s, HashMap::new(), st);
        assert_eq!(vm.next_event(), Event::End);
    }

    #[test]
    fn chain_loads_other_script() {
        let mut c = Vec::new();
        w(&mut c, &[0xa1, 42]);
        let mut other = Vec::new();
        say(&mut other, "chained");
        w(&mut other, &[0xFFFF]);
        let mut vm = Vm::new(&script(c), HashMap::new());
        vm.set_loader(Box::new(move |n| (n == 42).then(|| (TalkScript::standalone(42, "o".into(), other.clone()), HashMap::new()))));
        assert_eq!(text(vm.next_event()), "chained");
        assert_eq!(vm.script_id, 42);
    }

    #[test]
    fn audio_ops_queue() {
        let mut c = Vec::new();
        w(&mut c, &[0x3b, 5, 0x72, 3, 4, 2, 0x89, 0x71, 6, 0]);
        say(&mut c, "x");
        w(&mut c, &[0xFFFF]);
        let mut vm = Vm::new(&script(c), HashMap::new());
        assert_eq!(text(vm.next_event()), "x");
        assert_eq!(vm.state.audio, vec![AudioCmd::Sfx(5), AudioCmd::Music { a: 3, b: 4, mode: 2 }, AudioCmd::SceneMusic, AudioCmd::Sfx(6)]);
    }
}