//! Map + hero + dialogue state, independent of the window (also used for headless screenshots).
use crate::render::{Camera, Renderer, CAM_SCREEN, CAM_WORLD, UPSCALE_ANIME4K, UPSCALE_LINEAR, UPSCALE_NEAREST, UPSCALE_XBR};
use crate::text::TextRenderer;
use newpal_assets::act;
use newpal_assets::i18n::{key as i18n_key, I18n};
use newpal_assets::lmf::Region;
use newpal_assets::nav::NavGrid;
use newpal_assets::vfs::{Texture, Vfs};
use crate::timing::{lerp, ORIGINAL_TICK};
use newpal_vm::{Dialogue, Event, ScriptBackend, Vm, VmState};

/// ms per ACT `DS` unit = one original main-loop frame (35 ms, the original sleep argument 0x23).
pub const TICK_MS: f32 = ORIGINAL_TICK * 1000.0;
/// Camera follow rate (1/s) for exponential easing when smoothing is on.
pub const CAM_RATE: f32 = 9.0;
/// Dialogue box fade/slide-in time (s).
pub const UI_FADE: f32 = 0.14;
/// Walking speed in logical pixels per second (guess).
pub const WALK_SPEED: f32 = 120.0;
/// all_char.act slot 0 = the hero. Actions 0..8 stand, 8..16 walk; direction order below.
pub const HERO_ACT_SLOT: usize = 0;
/// Region scripts with ids >= this fire when stepped on (doors/exits); smaller ids need the action key (guess).
pub const STEP_TRIGGER_MIN: u32 = 6000;

#[derive(Clone, Copy)]
pub struct Sprite {
    /// original (or only) texture
    pub tex: usize,
    /// HD cache still, if prepared
    pub tex_hd: Option<usize>,
    pub w: f32,
    pub h: f32,
}

impl Sprite {
    pub fn pick(&self, look: Look) -> (usize, u32) {
        match look.present {
            Present::Original => (self.tex, UPSCALE_NEAREST),
            Present::Shader => (self.tex, look.shader.mode()),
            Present::Hd => {
                if let Some(h) = self.tex_hd {
                    (h, UPSCALE_LINEAR)
                } else {
                    (self.tex, look.shader.mode())
                }
            }
        }
    }
}

fn upload(r: &mut Renderer, t: &Texture) -> Sprite {
    upload_pair(r, t, None)
}

fn upload_pair(r: &mut Renderer, lo: &Texture, hi: Option<&Texture>) -> Sprite {
    let tex = r.upload(&lo.image, lo.scale() > 1.0);
    let tex_hd = hi.filter(|h| h.image.width > lo.image.width || h.origin != newpal_assets::vfs::Origin::Game)
        .map(|h| r.upload(&h.image, true));
    Sprite { tex, tex_hd, w: lo.logical_w, h: lo.logical_h }
}

pub struct Anim {
    /// original frames (sprite, duration s)
    pub frames: Vec<(Sprite, f32)>,
    /// mod in-between frames (act/<act>/<slot>/<action>/anim.toml), same loop length unless the mod says otherwise
    pub smooth: Option<Vec<(Sprite, f32)>>,
}

/// How frames are chosen / blended (runtime toggles).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// interpolate positions / camera / UI between logic ticks
    pub interpolate: bool,
    /// use mod in-between frames when present
    pub smooth_frames: bool,
    /// cross-fade original frames (fallback when no in-betweens)
    pub crossfade: bool,
    /// original / realtime shader / HD cache (F9 cycles; missing HD falls back to shader)
    pub present: Present,
    /// Anime4K-style or xBR/ScaleFX (F10)
    pub shader: ShaderKind,
    /// warp between keyframes at display rate; original delays stay unchanged (F11)
    pub flow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Present { Original, Shader, Hd }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderKind { Anime4k, Xbr }

impl Present {
    pub fn cycle(self) -> Self {
        match self { Self::Original => Self::Shader, Self::Shader => Self::Hd, Self::Hd => Self::Original }
    }
    pub fn as_str(self) -> &'static str {
        match self { Self::Original => "orig", Self::Shader => "shader", Self::Hd => "hd" }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "original" | "orig" | "nearest" | "pixel" => Some(Self::Original),
            "shader" | "upscale" => Some(Self::Shader),
            "hd" | "esrgan" => Some(Self::Hd),
            _ => None,
        }
    }
}

impl ShaderKind {
    pub fn cycle(self) -> Self { match self { Self::Anime4k => Self::Xbr, Self::Xbr => Self::Anime4k } }
    pub fn as_str(self) -> &'static str { match self { Self::Anime4k => "anime4k", Self::Xbr => "xbr" } }
    pub fn mode(self) -> u32 {
        match self { Self::Anime4k => UPSCALE_ANIME4K, Self::Xbr => UPSCALE_XBR }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "anime4k" | "anime" | "a4k" => Some(Self::Anime4k),
            "xbr" | "scalefx" | "sfx" => Some(Self::Xbr),
            _ => None,
        }
    }
}

impl Default for Look {
    fn default() -> Self {
        Self { interpolate: true, smooth_frames: false, crossfade: false, present: Present::Hd, shader: ShaderKind::Anime4k, flow: true }
    }
}

/// Frame at time t in a loop: (current, next, fraction through current).
pub fn sample(frames: &[(Sprite, f32)], t: f32) -> Option<(Sprite, Sprite, f32)> {
    let first = frames.first()?.0;
    let total: f32 = frames.iter().map(|f| f.1).sum();
    if total <= 0.0 || frames.len() == 1 {
        return Some((first, first, 0.0));
    }
    let mut t = t.max(0.0) % total;
    for (i, (s, d)) in frames.iter().enumerate() {
        if t < *d {
            let next = frames[(i + 1) % frames.len()].0;
            return Some((*s, next, (t / d).clamp(0.0, 1.0)));
        }
        t -= d;
    }
    let last = frames.last()?.0;
    Some((last, first, 1.0))
}

impl Anim {
    /// Sprites to draw at time t: [(sprite, opacity)] (1 entry, or 2 when cross-fading).
    pub fn draw_at(&self, t: f32, look: Look) -> Vec<(Sprite, f32)> {
        if look.smooth_frames {
            if let Some((s, _, _)) = self.smooth.as_deref().and_then(|f| sample(f, t)) {
                return vec![(s, 1.0)];
            }
        }
        match sample(&self.frames, t) {
            Some((a, b, k)) if look.crossfade && b.pick(look).0 != a.pick(look).0 && k > 0.0 => vec![(a, 1.0), (b, k)],
            Some((a, _, _)) => vec![(a, 1.0)],
            None => Vec::new(),
        }
    }
    pub fn has_smooth(&self) -> bool {
        self.smooth.is_some()
    }
}

impl World {
    /// Number of loaded animations that have mod in-between frames.
    pub fn smooth_anims(&self) -> usize {
        let h = &self.hero;
        h.stand.iter().chain(h.walk.iter()).chain(self.npcs.iter().filter_map(|n| n.anim.as_ref())).filter(|a| a.has_smooth()).count()
    }
}

/// Direction index as used by the ACT tables: 0 N, 1 S, 2 W, 3 E, 4 NW, 5 SE, 6 SW, 7 NE
/// (identified visually from the walk strips).
pub fn dir_from(dx: f32, dy: f32) -> usize {
    let oct = ((dy.atan2(dx) / std::f32::consts::FRAC_PI_4).round() as i32).rem_euclid(8) as usize;
    // octant 0 = E, 1 = SE, 2 = S, 3 = SW, 4 = W, 5 = NW, 6 = N, 7 = NE
    [3, 5, 1, 6, 2, 4, 0, 7][oct]
}

pub struct Hero {
    pub x: f32,
    pub y: f32,
    pub dir: usize,
    pub moving: bool,
    /// time in the current stand/walk state (s)
    pub t: f32,
    /// distance walked (px): drives walk frames, so feet do not slide when blocked
    pub walk_dist: f32,
    /// previous logic state (for interpolation): x, y, t, walk_dist
    pub prev: (f32, f32, f32, f32),
    /// click-to-walk waypoints (A* on the cell grid, smoothed)
    pub path: Vec<(f32, f32)>,
    stand: Vec<Anim>,
    walk: Vec<Anim>,
}

/// A role from maps.dat placed on the current map.
pub struct Npc {
    pub role: u16,
    pub actor: u16,
    pub x: f32,
    pub y: f32,
    pub script: u32,
    anim: Option<Anim>,
}

struct Obj {
    sprite: Sprite,
    x: f32,
    y: f32,
    base_y: f32,
}

pub struct DialogueBox {
    pub d: Dialogue,
    tex: Option<(usize, u32, u32)>,
    built_for: (u32, u32),
}

pub struct World {
    pub lmf: String,
    pub map_id: u32,
    pub size: (f32, f32),
    bg: Sprite,
    objs: Vec<Obj>,
    pub hero: Hero,
    pub view_height: f32,
    pub input: (f32, f32),
    pub vm: Option<Vm>,
    /// story state while no script runs (a running [Vm] owns it)
    pub story: Option<VmState>,
    /// scene change requested by the running script: (city, x cell, y cell)
    pub pending_scene: Option<(u16, u16, u16)>,
    pub dialogue: Option<DialogueBox>,
    text: Option<TextRenderer>,
    /// language layer shared with the VFS (locale can change at runtime)
    pub i18n: std::sync::Arc<I18n>,
    pub status: String,
    pub nav: NavGrid,
    pub regions: Vec<Region>,
    last_region: u32,
    /// talk script queued by a region trigger; started by [World::tick]
    pub pending_script: Option<u32>,
    pub debug_nav: bool,
    /// maps.dat scene id of this map
    pub city: Option<u16>,
    pub npcs: Vec<Npc>,
    /// audio requests from scripts, drained by the app each frame
    pub audio_cmds: Vec<newpal_vm::AudioCmd>,
    /// render blend factor between the previous and current logic tick (set by the app per frame)
    pub alpha: f32,
    pub look: Look,
    /// eased camera centre and its previous tick value
    cam: (f32, f32),
    cam_prev: (f32, f32),
    /// dialogue box fade-in 0..1 (current, previous tick)
    ui_k: (f32, f32),
    /// world clock for ambient animation (current, previous tick)
    clock: (f32, f32),
    /// set when a scene was (re)loaded: the app switches to its music
    pub scene_changed: bool,
}

/// Original frames of an action + mod in-betweens for (act file, slot, action index) if a mod has them.
fn load_anim(vfs: &Vfs, r: &mut Renderer, key: (&str, usize, usize), a: Option<&act::Action>) -> Anim {
    let mut frames = Vec::new();
    if let Some(a) = a {
        for s in act::steps(a) {
            let Some(id) = s.sprite.filter(|&id| id < 3000) else { continue };
            if let Ok((lo, hi)) = vfs.sprite_frame_pair("all_char", id, s.frame as usize) {
                frames.push((upload_pair(r, &lo, hi.as_ref()), s.delay as f32 * TICK_MS / 1000.0));
            }
        }
    }
    let total: f32 = frames.iter().map(|f| f.1).sum();
    let smooth = match vfs.anim_override(key.0, key.1, key.2) {
        Ok(Some((m, tex))) if !tex.is_empty() => {
            let d = m.durations(tex.len(), total);
            Some(tex.iter().zip(d).map(|(t, d)| (upload(r, t), d)).collect())
        }
        Ok(_) => None,
        Err(e) => {
            eprintln!("anim override {}/{}/{}: {e}", key.0, key.1, key.2);
            None
        }
    };
    Anim { frames, smooth }
}

/// Roles of the scene that uses this map. Visible roles block their feet cell.
fn load_npcs(vfs: &Vfs, r: &mut Renderer, lmf: &str, map_id: u32, city: Option<u16>, nav: &mut NavGrid) -> (Option<u16>, Vec<Npc>) {
    let Ok(md) = vfs.maps_dat() else { return (None, Vec::new()) };
    let found = match city { Some(c) => md.city(c), None => md.city_for_map(map_id as u16) };
    let Some(city) = found.filter(|_| newpal_assets::mapsdat::MapsDat::lmf_for(map_id as u16) == lmf) else {
        return (None, Vec::new());
    };
    let mut out = Vec::new();
    for role in md.roles_in(city.id) {
        let (cx, cy) = role.cell();
        if cx == 0 && cy == 0 {
            continue; // placeholder rows (10000/10001)
        }
        let actor = role.actor();
        let anim = if (1..MAX_ACTOR).contains(&actor) {
            vfs.act("all_char", actor as usize - 1).ok().and_then(|acts| {
                let idx = if acts.get(role.action() as usize).is_some_and(|a| a.is_some()) { role.action() as usize } else { acts.iter().position(|a| a.is_some()).unwrap_or(0) };
                let a = load_anim(vfs, r, ("all_char", actor as usize - 1, idx), acts.get(idx).and_then(|a| a.as_ref()));
                (!a.frames.is_empty()).then_some(a)
            })
        } else {
            None
        };
        let (x, y) = NavGrid::center(cx as i32, cy as i32);
        if anim.is_some() {
            if let Some(i) = ((cx as i32) < nav.w && (cy as i32) < nav.h).then(|| (cy as i32 * nav.w + cx as i32) as usize) {
                nav.cells[i] |= 0x4000_0000;
            }
        }
        out.push(Npc { role: role.id(), actor, x, y, script: role.script() as u32, anim });
    }
    (Some(city.id), out)
}

/// all_char actors >= this are engine-special (invisible triggers, hero placeholders) - guess.
pub const MAX_ACTOR: u16 = 699;
/// Action key reach (feet distance, px).
pub const TALK_REACH: f32 = 40.0;

/// Font stack + wrapping of the VFS's current locale.
pub(crate) fn build_text_renderer(vfs: &Vfs) -> Option<TextRenderer> {
    let stack = vfs.font_stack().ok()?;
    let refs: Vec<&[u8]> = stack.iter().map(|(b, _)| b.as_slice()).collect();
    for (i, (_, d)) in stack.iter().enumerate() {
        eprintln!("font {i}: {d}");
    }
    TextRenderer::from_stack(&refs, vfs.i18n().wrap_mode()).ok()
}

impl World {
    pub fn load(vfs: &Vfs, r: &mut Renderer, lmf: &str, map_id: u32, view_height: f32) -> Result<Self, String> {
        let mut w = Self::load_scene(vfs, r, lmf, map_id, None, view_height)?;
        if let Some(c) = w.city {
            w.story_mut().enter_city(c);
        }
        Ok(w)
    }

    /// Load a map; `city` = maps.dat scene (None: first scene using this map).
    pub fn load_scene(vfs: &Vfs, r: &mut Renderer, lmf: &str, map_id: u32, city: Option<u16>, view_height: f32) -> Result<Self, String> {
        let map = vfs.map(lmf, map_id).map_err(|e| format!("map {lmf}:{map_id}: {e}"))?;
        let (pw, ph) = map.pixel_size();
        let nav = NavGrid::from_map(&map);
        let regions = map.regions.clone();
        let (bg_lo, bg_hi) = vfs.map_bg_pair(lmf, &map).map_err(|e| e.to_string())?;
        let bg = upload_pair(r, &bg_lo, bg_hi.as_ref());
        let mut objs = Vec::new();
        for p in &map.placements {
            if let Ok(Some((lo, hi))) = vfs.map_object_pair(lmf, &map, p.id, p.x0, p.y0) {
                objs.push(Obj { sprite: upload_pair(r, &lo, hi.as_ref()), x: p.x0 as f32, y: p.y0 as f32, base_y: p.y1 as f32 });
            }
        }
        objs.sort_by(|a, b| a.base_y.total_cmp(&b.base_y));
        let mut nav = nav;
        let (city, npcs) = load_npcs(vfs, r, lmf, map_id, city, &mut nav);
        let acts = vfs.act("all_char", HERO_ACT_SLOT).map_err(|e| e.to_string())?;
        let stand = (0..8).map(|d| load_anim(vfs, r, ("all_char", HERO_ACT_SLOT, d), acts.get(d).and_then(|a| a.as_ref()))).collect();
        let walk = (8..16).map(|d| load_anim(vfs, r, ("all_char", HERO_ACT_SLOT, d), acts.get(d).and_then(|a| a.as_ref()))).collect();
        let text = build_text_renderer(vfs);
        Ok(Self {
            lmf: lmf.to_string(),
            map_id,
            size: (pw as f32, ph as f32),
            bg,
            objs,
            hero: Hero { x: pw as f32 / 2.0, y: ph as f32 * 0.6, dir: 1, moving: false, t: 0.0, walk_dist: 0.0, prev: (0.0, 0.0, 0.0, 0.0), path: Vec::new(), stand, walk },
            view_height,
            input: (0.0, 0.0),
            vm: None,
            story: Some(VmState::new_game()),
            pending_scene: None,
            dialogue: None,
            text,
            i18n: vfs.i18n().clone(),
            status: String::new(),
            nav,
            regions,
            last_region: 0,
            pending_script: None,
            debug_nav: false,
            city,
            npcs,
            audio_cmds: Vec::new(),
            scene_changed: true,
            alpha: 1.0,
            look: Look::default(),
            cam: (0.0, 0.0),
            cam_prev: (0.0, 0.0),
            ui_k: (0.0, 0.0),
            clock: (0.0, 0.0),
        })
    }

    /// Story state, wherever it currently lives.
    pub fn story_mut(&mut self) -> &mut VmState {
        match self.vm.as_mut() {
            Some(vm) => &mut vm.state,
            None => self.story.get_or_insert_with(VmState::new_game),
        }
    }

    fn take_story(&mut self) -> VmState {
        if let Some(vm) = self.vm.take() {
            return vm.into_state();
        }
        self.story.take().unwrap_or_else(VmState::new_game)
    }

    /// Start a talk script (keeps the story state); shows its first dialogue.
    pub fn start_script(&mut self, vfs: &Vfs, n: u32) -> Result<(), String> {
        let script = vfs.talk(n).map_err(|e| e.to_string())?;
        let ov = vfs.talk_text_overrides(n).map_err(|e| e.to_string())?;
        let mut st = self.take_story();
        if let Some(c) = self.city {
            st.city = c;
        }
        st.roles_present = self.npcs.iter().map(|n| n.role).collect();
        let mut vm = Vm::with_state(&script, ov, st);
        vm.set_text_filter(self.text_filter());
        let mut loader_vfs = Vfs::new(vfs.game_dir().to_path_buf(), vfs.mods().to_vec());
        loader_vfs.set_i18n(vfs.i18n().clone());
        vm.set_loader(Box::new(move |n| {
            let s = loader_vfs.talk(n).ok()?;
            Some((s, loader_vfs.talk_text_overrides(n).unwrap_or_default()))
        }));
        self.vm = Some(vm);
        self.advance_dialogue();
        Ok(())
    }

    /// Filter for non-overridden dialogue text of the current locale (zh-Hant -> zh-Hans etc.).
    fn text_filter(&self) -> Option<newpal_vm::TextFilter> {
        self.i18n.text_filter().map(|c| std::sync::Arc::new(move |s: &str| c.convert(s)) as newpal_vm::TextFilter)
    }

    /// Switch language at runtime: packs, fonts, wrapping, the running script and the visible line.
    pub fn set_locale(&mut self, vfs: &Vfs, tag: &str) -> Result<String, String> {
        let info = vfs.i18n().set_locale(tag)?;
        self.text = build_text_renderer(vfs);
        let filter = self.text_filter();
        if let Some(vm) = self.vm.as_mut() {
            let n = vm.script_id;
            vm.set_overrides(vfs.talk_text_overrides(n).unwrap_or_default());
            vm.set_text_filter(filter);
            if let Some(db) = self.dialogue.as_mut() {
                let (raw, ov) = vm.localized(db.d.pos, &db.d.original);
                db.d.set_text(raw, ov);
                db.tex = None;
                if !ov && !self.i18n.has_converter() {
                    self.i18n.note_missing(&i18n_key::talk(n, db.d.pos));
                }
            }
        }
        Ok(info.tag)
    }

    /// Next dialogue line (Enter / Space / click).
    pub fn advance_dialogue(&mut self) {
        self.dialogue = None;
        if self.pending_scene.is_some() {
            return;
        }
        let Some(vm) = self.vm.as_mut() else { return };
        loop {
            let ev = vm.next_event();
            self.audio_cmds.extend(vm.state.audio.drain(..));
            match ev {
                Event::Dialogue(d) => {
                    if !d.overridden && !self.i18n.has_converter() {
                        self.i18n.note_missing(&i18n_key::talk(vm.script_id, d.pos));
                    }
                    self.dialogue = Some(DialogueBox { d, tex: None, built_for: (0, 0) });
                    return;
                }
                Event::Movie(m) => self.status = self.i18n.ui_fmt("status.movie_skipped", &[("name", m)]),
                Event::ChangeScene { city, x, y, .. } => {
                    self.status = format!("script {} -> scene {city} at ({x},{y})", vm.script_id);
                    eprintln!("{}", self.status);
                    self.pending_scene = Some((city, x, y));
                    return;
                }
                Event::End => {
                    self.status = format!(
                        "script {} ended, {} instrs, unhandled ops {:?}, skipped {} B",
                        vm.script_id, vm.state.executed, vm.state.unhandled.keys().map(|k| format!("{k:x}")).collect::<Vec<_>>(), vm.state.skipped
                    );
                    eprintln!("{}", self.status);
                    let mut st = self.vm.take().unwrap().into_state();
                    st.unhandled.clear();
                    st.executed = 0;
                    st.skipped = 0;
                    self.story = Some(st);
                    return;
                }
            }
        }
    }

    /// Load scene `city` (maps.dat -> LMF map) with the hero at cell (x, y); the running script and story survive.
    pub fn change_scene(&mut self, vfs: &Vfs, r: &mut Renderer, city: u16, x: u16, y: u16) -> Result<(), String> {
        let md = vfs.maps_dat().map_err(|e| e.to_string())?;
        let c = md.city(city).ok_or_else(|| format!("scene {city} not in maps.dat"))?;
        let lmf = newpal_assets::mapsdat::MapsDat::lmf_for(c.map);
        let mut w = World::load_scene(vfs, r, lmf, c.map as u32, Some(city), self.view_height)?;
        w.vm = self.vm.take();
        w.story = self.story.take();
        w.debug_nav = self.debug_nav;
        w.look = self.look;
        w.clock = self.clock;
        w.audio_cmds = std::mem::take(&mut self.audio_cmds);
        w.story_mut().enter_city(city);
        let present: std::collections::HashSet<u16> = w.npcs.iter().map(|n| n.role).collect();
        w.story_mut().roles_present = present;
        let (px, py) = NavGrid::center(x as i32, y as i32);
        w.place_hero(px, py);
        *self = w;
        Ok(())
    }
    /// Place the hero (snapped to the nearest walkable cell) without firing the region under it.
    pub fn place_hero(&mut self, x: f32, y: f32) {
        let (cx, cy) = self.nav.cell_of(x, y);
        let (x, y) = match self.nav.nearest_walkable(cx, cy) {
            Some(c) if c != (cx, cy) => NavGrid::center(c.0, c.1),
            _ => (x, y),
        };
        self.hero.x = x;
        self.hero.y = y;
        self.hero.path.clear();
        self.last_region = self.nav.region_px(x, y);
        self.snap_visuals();
    }

    /// Drop interpolation history (teleports, scene loads): camera and hero jump without easing.
    pub fn snap_visuals(&mut self) {
        let h = &mut self.hero;
        h.prev = (h.x, h.y, h.t, h.walk_dist);
        self.cam = self.cam_target();
        self.cam_prev = self.cam;
    }

    fn cam_target(&self) -> (f32, f32) {
        (self.hero.x, self.hero.y - 40.0)
    }

    /// One fixed logic step: remember the previous state, simulate, then ease the camera / UI.
    pub fn step(&mut self, vfs: &Vfs, r: &mut Renderer, dt: f32) {
        let h = &mut self.hero;
        h.prev = (h.x, h.y, h.t, h.walk_dist);
        self.cam_prev = self.cam;
        self.ui_k.1 = self.ui_k.0;
        self.clock.1 = self.clock.0;
        self.tick(vfs, r, dt);
        self.clock.0 += dt;
        let tgt = self.cam_target();
        let (dx, dy) = (tgt.0 - self.cam.0, tgt.1 - self.cam.1);
        if !self.look.interpolate || dx.hypot(dy) > 320.0 {
            self.cam = tgt;
        } else {
            let k = 1.0 - (-CAM_RATE * dt).exp();
            self.cam = (self.cam.0 + dx * k, self.cam.1 + dy * k);
        }
        let open = self.dialogue.is_some();
        self.ui_k.0 = if !open { 0.0 } else if !self.look.interpolate { 1.0 } else { (self.ui_k.0 + dt / UI_FADE).min(1.0) };
    }

    /// Finish UI transitions now (single headless frames).
    pub fn settle_ui(&mut self) {
        let k = if self.dialogue.is_some() { 1.0 } else { 0.0 };
        self.ui_k = (k, k);
    }

    /// Interpolated render state of the hero: (x, y, anim time, walk distance).
    pub fn hero_render(&self) -> (f32, f32, f32, f32) {
        let h = &self.hero;
        if !self.look.interpolate {
            return (h.x, h.y, h.t, h.walk_dist);
        }
        let a = self.alpha;
        let t = if h.t >= h.prev.2 { lerp(h.prev.2, h.t, a) } else { h.t };
        (lerp(h.prev.0, h.x, a), lerp(h.prev.1, h.y, a), t, lerp(h.prev.3, h.walk_dist, a))
    }

    /// Click-to-walk: A* path to a world point (unreachable -> nearest reachable floor cell).
    pub fn walk_to(&mut self, x: f32, y: f32) -> bool {
        match self.nav.find_path((self.hero.x, self.hero.y), (x, y)) {
            Some(p) => {
                self.hero.path = p;
                true
            }
            None => {
                self.status = self.i18n.ui_fmt("status.no_path", &[("x", format!("{x:.0}")), ("y", format!("{y:.0}"))]);
                false
            }
        }
    }

    /// Action key (Space/Enter outside dialogue): run the region script under the hero.
    pub fn interact(&mut self) -> bool {
        let (hx, hy) = (self.hero.x, self.hero.y);
        let near = self.npcs.iter().filter(|n| n.script != 0).map(|n| ((n.x - hx).hypot(n.y - hy), n)).filter(|(d, _)| *d <= TALK_REACH).min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, n)) = near {
            self.status = format!("talk: role {} actor {} -> script {}", n.role, n.actor, n.script);
            self.hero.dir = dir_from(n.x - hx, n.y - hy);
            self.pending_script = Some(n.script);
            return true;
        }
        let reg = self.nav.region_px(self.hero.x, self.hero.y);
        match self.regions.get((reg as usize).wrapping_sub(1)) {
            Some(r) if r.v[1] != 0 => {
                self.status = format!("action: region {reg} {} -> script {}", r.name, r.v[1]);
                self.pending_script = Some(r.v[1]);
                true
            }
            _ => false,
        }
    }

    /// update + run queued region / NPC scripts and script scene changes.
    pub fn tick(&mut self, vfs: &Vfs, r: &mut Renderer, dt: f32) {
        self.update(dt);
        if let Some((city, x, y)) = self.pending_scene.take() {
            if let Err(e) = self.change_scene(vfs, r, city, x, y) {
                self.status = format!("scene {city}: {e}");
                eprintln!("{}", self.status);
            }
            self.advance_dialogue();
        }
        if let Some(n) = self.pending_script.take() {
            if self.vm.is_none() {
                if let Err(e) = self.start_script(vfs, n) {
                    self.status = format!("trigger script {n}: {e}");
                }
            }
        }
    }
    pub fn update(&mut self, dt: f32) {
        let busy = self.dialogue.is_some();
        let h = &mut self.hero;
        let (mut dx, mut dy) = self.input;
        let keyboard = dx != 0.0 || dy != 0.0;
        if keyboard {
            h.path.clear();
        } else if let Some(&(tx, ty)) = h.path.first() {
            let (ex, ey) = (tx - h.x, ty - h.y);
            let d = (ex * ex + ey * ey).sqrt();
            if d <= (WALK_SPEED * dt).max(1.5) {
                if !busy { h.x = tx; h.y = ty; h.path.remove(0); }
            } else {
                dx = ex / d;
                dy = ey / d;
            }
        }
        let len = (dx * dx + dy * dy).sqrt();
        let was = h.moving;
        h.moving = len > 0.0 && !busy;
        if h.moving {
            h.dir = dir_from(dx, dy);
            let step = WALK_SPEED * dt / len;
            let (nx, ny) = ((h.x + dx * step).clamp(0.0, self.size.0 - 1.0), (h.y + dy * step).clamp(0.0, self.size.1 - 1.0));
            // collision with wall sliding (keyboard); path waypoints are already clear
            let (ox, oy) = (h.x, h.y);
            if self.nav.walkable_px(nx, ny) || !self.nav.walkable_px(h.x, h.y) {
                h.x = nx;
                h.y = ny;
            } else if dx != 0.0 && self.nav.walkable_px(nx, h.y) {
                h.x = nx;
            } else if dy != 0.0 && self.nav.walkable_px(h.x, ny) {
                h.y = ny;
            } else {
                h.moving = false;
                h.path.clear();
            }
            h.walk_dist += (h.x - ox).hypot(h.y - oy);
        }
        if was != h.moving {
            h.t = 0.0;
        }
        h.t += dt;
        // region triggers (index -> Region.v[1] talk script). Guess: scripts >= 6000 are door/exit
        // scripts (short original labels meaning enter / leave / down) and fire on entering; others (items, NPC zones) wait for the action key.
        let reg = self.nav.region_px(h.x, h.y);
        if reg != self.last_region {
            self.last_region = reg;
            if reg != 0 && !busy {
                if let Some(r) = self.regions.get(reg as usize - 1) {
                    self.status = format!("region {reg} {} -> script {}", r.name, r.v[1]);
                    eprintln!("{}", self.status);
                    if r.v[1] >= STEP_TRIGGER_MIN {
                        self.pending_script = Some(r.v[1]);
                    } else {
                        self.status.push_str(" (action key)");
                    }
                }
            }
        }
    }
    /// Visible world rectangle for a surface of `sw`x`sh` physical pixels (aspect-correct, widescreen-aware).
    pub fn camera(&self, sw: u32, sh: u32) -> Camera {
        let vh = self.view_height;
        let vw = vh * sw as f32 / sh.max(1) as f32;
        let fit = |center: f32, view: f32, world: f32| {
            if world <= view { (world - view) / 2.0 } else { (center - view / 2.0).clamp(0.0, world - view) }
        };
        let (cx, cy) = if self.look.interpolate { (lerp(self.cam_prev.0, self.cam.0, self.alpha), lerp(self.cam_prev.1, self.cam.1, self.alpha)) } else { self.cam };
        Camera { origin: [fit(cx, vw, self.size.0), fit(cy, vh, self.size.1)], size: [vw, vh] }
    }

    /// Convert a physical cursor position to world coordinates.
    pub fn screen_to_world(&self, sw: u32, sh: u32, x: f32, y: f32) -> (f32, f32) {
        let c = self.camera(sw, sh);
        (c.origin[0] + x / sw as f32 * c.size[0], c.origin[1] + y / sh as f32 * c.size[1])
    }

    pub fn draw(&mut self, r: &mut Renderer, sw: u32, sh: u32) {
        r.set_camera(CAM_WORLD, self.camera(sw, sh));
        r.set_camera(CAM_SCREEN, Camera { origin: [0.0, 0.0], size: [sw as f32, sh as f32] });
        let white = [1.0; 4];
        let (bg_tex, bg_mode) = self.bg.pick(self.look);
        r.push_ex(bg_tex, CAM_WORLD, 0.0, 0.0, self.bg.w, self.bg.h, white, bg_mode);
        let h = &self.hero;
        let (hx, hy, ht, hd) = self.hero_render();
        // walk frames follow the distance walked (at WALK_SPEED this equals real time), stand frames follow time
        let (anim, phase) = if h.moving { (&h.walk[h.dir], hd / WALK_SPEED) } else { (&h.stand[h.dir], ht) };
        let clock = if self.look.interpolate { lerp(self.clock.1, self.clock.0, self.alpha) } else { self.clock.0 };
        // painter's order by feet / baseline y: objects, NPCs and the hero (stable sort keeps cross-fade pairs ordered)
        let mut items: Vec<(f32, Sprite, f32, f32, f32)> = Vec::new();
        for o in &self.objs {
            items.push((o.base_y, o.sprite, o.x, o.y, 1.0));
        }
        for n in &self.npcs {
            if let Some(a) = n.anim.as_ref() {
                for (s, k) in a.draw_at(clock + n.x * 0.01, self.look) {
                    items.push((n.y, s, n.x - s.w / 2.0, n.y - s.h, k));
                }
            }
        }
        for (s, k) in anim.draw_at(phase, self.look) {
            items.push((hy + 0.01, s, hx - s.w / 2.0, hy - s.h, k));
        }
        items.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, s, x, y, k) in items {
            let (tex, mode) = s.pick(self.look);
            r.push_ex(tex, CAM_WORLD, x, y, s.w, s.h, [1.0, 1.0, 1.0, k], mode);
        }        if self.debug_nav {
            self.draw_nav(r);
        }
        self.draw_dialogue(r, sw, sh);
    }

    /// Debug overlay: blocked cells red, region cells blue, current path green.
    fn draw_nav(&self, r: &mut Renderer) {
        let t = newpal_assets::lmf::TILE as f32;
        for cy in 0..self.nav.h {
            for cx in 0..self.nav.w {
                let c = self.nav.get(cx, cy).unwrap_or(0);
                let col = if c & newpal_assets::nav::BLOCK_MASK != 0 { [1.0, 0.1, 0.1, 0.35] } else if c & newpal_assets::nav::REGION_MASK != 0 { [0.2, 0.4, 1.0, 0.45] } else { continue };
                r.push(r.white, CAM_WORLD, cx as f32 * t + 1.0, cy as f32 * t + 1.0, t - 2.0, t - 2.0, col);
            }
        }
        let (hx, hy, _, _) = self.hero_render();
        let mut prev = (hx, hy);
        for &p in &self.hero.path {
            let n = ((p.0 - prev.0).hypot(p.1 - prev.1) / 4.0).ceil().max(1.0) as i32;
            for i in 0..=n {
                let k = i as f32 / n as f32;
                r.push(r.white, CAM_WORLD, prev.0 + (p.0 - prev.0) * k - 1.5, prev.1 + (p.1 - prev.1) * k - 1.5, 3.0, 3.0, [0.2, 1.0, 0.3, 0.9]);
            }
            prev = p;
        }
    }

    fn draw_dialogue(&mut self, r: &mut Renderer, sw: u32, sh: u32) {
        let k = if self.look.interpolate { lerp(self.ui_k.1, self.ui_k.0, self.alpha) } else { 1.0 };
        // ease-out cubic
        let k = 1.0 - (1.0 - k.clamp(0.0, 1.0)).powi(3);
        let Some(db) = self.dialogue.as_mut() else { return };
        let Some(text) = self.text.as_ref() else { return };
        let ui = sh as f32 / 480.0; // UI scale: original layout at 480p, rendered at native resolution
        let (bw, pad) = ((sw as f32 * 0.86).min(sh as f32 * 1.6), 14.0 * ui);
        let px = (19.0 * ui).max(12.0);
        if db.tex.is_none() || db.built_for != (sw, sh) {
            let mut lines = Vec::new();
            if let Some(n) = &db.d.name {
                let sep = self.i18n.ui("dialogue.name_sep");
                // a translated line carries its own speaker; names are mapped only for untranslated lines
                let shown = if db.d.overridden { n.clone() } else { self.i18n.tr_name(n) };
                lines.push(vec![newpal_assets::talk::Span { color: 6, text: format!("{shown}{sep}") }]);
            }
            lines.extend(db.d.lines.iter().cloned());
            let hint = self.i18n.ui("dialogue.continue");
            if !hint.is_empty() {
                lines.push(vec![newpal_assets::talk::Span { color: 7, text: hint }]);
            }
            let img = text.render(&lines, px, (bw - 2.0 * pad) as u32);
            let id = match db.tex {
                Some((id, _, _)) => {
                    r.replace(id, &img, true);
                    id
                }
                None => r.upload(&img, true),
            };
            db.tex = Some((id, img.width, img.height));
            db.built_for = (sw, sh);
        }
        let (id, tw, th) = db.tex.unwrap();
        let bh = th as f32 + 2.0 * pad;
        let (bx, by) = ((sw as f32 - bw) / 2.0, sh as f32 - bh - 16.0 * ui + (1.0 - k) * 18.0 * ui);
        r.push(r.white, CAM_SCREEN, bx, by, bw, bh, [0.05, 0.04, 0.08, 0.78 * k]);
        r.push(r.white, CAM_SCREEN, bx, by, bw, 2.0 * ui, [0.85, 0.7, 0.4, 0.9 * k]);
        r.push(id, CAM_SCREEN, bx + pad, by + pad, tw as f32, th as f32, [1.0, 1.0, 1.0, k]);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn sp(tex: usize) -> Sprite {
        Sprite { tex, tex_hd: None, w: 10.0, h: 20.0 }
    }

    #[test]
    fn sample_and_blend_modes() {
        let frames = vec![(sp(1), 0.1), (sp(2), 0.1), (sp(3), 0.2)];
        let (a, b, k) = sample(&frames, 0.05).unwrap();
        assert_eq!((a.tex, b.tex), (1, 2));
        assert!((k - 0.5).abs() < 1e-4);
        let (a, b, _) = sample(&frames, 0.35).unwrap();
        assert_eq!((a.tex, b.tex), (3, 1)); // loops
        assert_eq!(sample(&frames, 0.4 + 0.05).unwrap().0.tex, 1);
        let mut an = Anim { frames: frames.clone(), smooth: None };
        let orig = Look { interpolate: false, smooth_frames: false, crossfade: false, present: Present::Original, shader: ShaderKind::Anime4k, flow: false };
        assert_eq!(an.draw_at(0.05, orig).len(), 1);
        let xf = Look { crossfade: true, ..orig };
        let d = an.draw_at(0.05, xf);
        assert_eq!(d.len(), 2);
        assert!((d[1].1 - 0.5).abs() < 1e-4);
        // mod in-betweens win when enabled
        an.smooth = Some((0..8).map(|i| (sp(10 + i), 0.05)).collect());
        let sm = Look { smooth_frames: true, ..xf };
        assert_eq!(an.draw_at(0.12, sm)[0].0.tex, 12);
        assert_eq!(an.draw_at(0.12, xf).len(), 2);
    }
}