//! Modern front end ("shell"): one window from the first run to gameplay.
//!
//! launcher (detect / verify the player's own game folder, HD assets) -> ink wipe -> title menu over the player's
//! own map -> new game reveals the same map and starts the opening script. Esc / Start opens the original-style system menu.
//! The vector UI (crate `newpal-ui`, vello) is composited over the game frame; the game image stays HD bitmaps.
//!
//! `--classic` (or any tool / screenshot flag) keeps the old direct-to-game path in `app.rs`.
use crate::app::{device, instance, sync_audio};
use crate::audio::Audio;
use crate::render::{Renderer, CAM_WORLD};
use crate::timing::{FixedStep, Pacer};
use crate::world::{Present, World};
use crate::Args;
use newpal_assets::gamedir::{self, Config};
use newpal_assets::i18n::I18n;
use newpal_assets::vfs::{discover_mods, Vfs};
use newpal_ui::gpu::UiGpu;
use newpal_ui::host::{DirCandidate, DirCheck, HdState, Host, LangEntry, ModEntry, SaveSlot};
use newpal_ui::input::{Action, Bindings, Device, Input, InputMapper};
use newpal_ui::scene::{layout_unit, Cmd, Ctx, DrawCx, SceneStack};
use newpal_ui::screens::{LauncherScreen, SysMenuScreen, TitleScreen};
use newpal_ui::settings::{Settings, WindowMode};
use newpal_ui::strings::{Strings, TextSource};
use newpal_ui::dialogue::{DialogueTheme, DialogueView, DlgCue, DlgEvent, DlgSpan, DlgText};
use newpal_ui::text::{Face, Fonts};
use newpal_ui::vello;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowId};

const CLEAR: wgpu::Color = wgpu::Color { r: 0.03, g: 0.045, b: 0.05, a: 1.0 };

// ---------------------------------------------------------------- options (kept out of main.rs' Args)

#[derive(Default, Clone, Debug)]
pub struct ShellOpts {
    pub classic: bool,
    pub first_run: bool,
    /// scripted input: `wait:1.5;key:down;key:confirm;click:0.5,0.5;move:x,y;pad;kbd;shot:name;rec:2;size:WxH;quit`
    pub script: Option<String>,
    /// headless: render the script offscreen and write `shot:` PNGs here
    pub shot_dir: Option<PathBuf>,
    pub ui_scale: Option<f32>,
    pub no_picker: bool,
}

static OPTS: OnceLock<Mutex<ShellOpts>> = OnceLock::new();

pub fn opts() -> ShellOpts {
    OPTS.get().map(|m| m.lock().unwrap().clone()).unwrap_or_default()
}

/// Remove the shell's own flags from the command line (called by `parse_args`).
pub fn strip_shell_args(args: impl Iterator<Item = String>) -> Vec<String> {
    let mut o = ShellOpts::default();
    let mut rest = Vec::new();
    let mut it = args.peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--classic" => o.classic = true,
            "--first-run" => o.first_run = true,
            "--no-picker" => o.no_picker = true,
            "--ui-script" => o.script = it.next(),
            "--ui-shot-dir" => o.shot_dir = it.next().map(PathBuf::from),
            "--ui-scale" => o.ui_scale = it.next().and_then(|v| v.parse().ok()),
            _ => rest.push(a),
        }
    }
    let _ = OPTS.set(Mutex::new(o));
    rest
}

pub const HELP: &str = "
FRONT END (default; one window: launcher -> title menu -> game)
  --classic             old direct-to-game window (debug keys F3..F12)
  --first-run           show the first-run launcher even if a game folder is configured
  --ui-scale <f>        interface scale (overrides [display] ui_scale)
  --ui-script <steps>   scripted input, e.g. \"wait:2;key:down;key:confirm;shot:menu;quit\"
  --ui-shot-dir <dir>   run the script headless and save shot:<name> as <dir>/<name>.png";

/// The shell runs unless a tool / screenshot / classic flag was given.
pub fn enabled(a: &Args) -> bool {
    let o = opts();
    !(o.classic || a.screenshot.is_some() || a.validate || a.pick || a.prepare || a.prepare_tools || a.hd_prepare || a.compare_hd || a.rife_flow_proto || a.list_langs)
}

// ---------------------------------------------------------------- i18n bridge

struct I18nSource(Arc<I18n>);
impl TextSource for I18nSource {
    fn text(&self, name: &str) -> Option<String> {
        Some(self.0.ui(name))
    }
    fn text_fmt(&self, name: &str, args: &[(&str, String)]) -> Option<String> {
        Some(self.0.ui_fmt(name, args))
    }
    fn locale(&self) -> String {
        self.0.locale()
    }
}

// ---------------------------------------------------------------- host: engine services for the UI

enum Install {
    Real { child: Child, total: usize, base: usize, done: Arc<AtomicUsize>, started: Instant, stop: Arc<AtomicUsize> },
    Sim { total: usize, started: Instant },
}

struct PatchInfo {
    dir: PathBuf,
    items: usize,
    bytes: u64,
}

pub struct GameHost {
    cfg_path: PathBuf,
    dir: Option<PathBuf>,
    cli_dir: Option<PathBuf>,
    hd_cache: PathBuf,
    mods_dir: Option<PathBuf>,
    i18n: Arc<I18n>,
    install: Option<Install>,
    hd_count: Option<usize>,
    patch: Option<Option<PatchInfo>>,
    last_fail: Option<String>,
    resolutions: Vec<(u32, u32)>,
    display_hz: Option<u32>,
    headless: bool,
}

fn count_files(dir: &Path) -> usize {
    let mut n = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(_) => {
                    if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("png")) {
                        n += 1;
                    }
                }
                _ => {}
            }
        }
    }
    n
}

/// Residual patch package: `<dir>/manifest.json` from `newpal-patch build` (entries + bytes, scanned without serde).
fn read_patch(dir: &Path) -> Option<PatchInfo> {
    let s = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    let items = s.matches("\"sha256\"").count();
    let mut bytes = 0u64;
    for part in s.split("\"bytes\"").skip(1) {
        let num: String = part.trim_start_matches([':', ' ']).chars().take_while(|c| c.is_ascii_digit()).collect();
        bytes += num.parse::<u64>().unwrap_or(0);
    }
    (items > 0).then(|| PatchInfo { dir: dir.to_path_buf(), items, bytes })
}

fn patch_exe() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("NEWPAL_PATCH_EXE") {
        return Some(PathBuf::from(p));
    }
    let exe = gamedir::exe_dir()?.join(if cfg!(windows) { "newpal-patch.exe" } else { "newpal-patch" });
    exe.is_file().then_some(exe)
}

impl GameHost {
    fn hd_ready(&self) -> bool {
        self.hd_cache.join("manifest.json").is_file() || self.hd_cache.join("sprites").is_dir() || self.hd_cache.join("maps").is_dir()
    }

    fn find_patch(&mut self) -> Option<&PatchInfo> {
        if self.patch.is_none() {
            let mut roots = Vec::new();
            if let Ok(p) = std::env::var("NEWPAL_PATCH_DIR") {
                roots.push(PathBuf::from(p));
            }
            if let Some(d) = gamedir::exe_dir() {
                roots.push(d.join("patches"));
            }
            roots.push(gamedir::output_dir().join("patches"));
            let mut found = None;
            'outer: for r in roots {
                if let Some(p) = read_patch(&r) {
                    found = Some(p);
                    break;
                }
                if let Ok(rd) = std::fs::read_dir(&r) {
                    for e in rd.flatten() {
                        if let Some(p) = read_patch(&e.path()) {
                            found = Some(p);
                            break 'outer;
                        }
                    }
                }
            }
            self.patch = Some(found);
        }
        self.patch.as_ref().unwrap().as_ref()
    }

    fn toggle_mod(&mut self, name: &str) {
        let Some(md) = self.mods_dir.clone() else { return };
        let mut t: toml::Table = std::fs::read_to_string(&self.cfg_path).ok().and_then(|s| s.parse().ok()).unwrap_or_default();
        let enabled: Vec<String> = t.get("mods").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
        let all: Vec<String> = discover_mods(&md, &[]).into_iter().map(|m| m.name).collect();
        let mut on: Vec<String> = if enabled.is_empty() { all.clone() } else { enabled };
        if let Some(i) = on.iter().position(|n| n == name) {
            on.remove(i);
        } else {
            on.push(name.to_string());
        }
        t.insert("mods".into(), toml::Value::Array(on.into_iter().map(toml::Value::String).collect()));
        let _ = std::fs::write(&self.cfg_path, toml::to_string_pretty(&t).unwrap_or_default());
    }
}

impl Host for GameHost {
    fn detect_game_dirs(&mut self) -> Vec<DirCandidate> {
        let mut v = Vec::new();
        if let Some(d) = &self.cli_dir {
            v.push(DirCandidate { path: d.clone(), source: "config".into() });
        }
        if let Ok(d) = std::env::var(gamedir::ENV_GAME_DIR) {
            v.push(DirCandidate { path: d.into(), source: "env".into() });
        }
        if let Some(d) = Config::load_from(&self.cfg_path).ok().and_then(|c| c.game_dir) {
            v.push(DirCandidate { path: d, source: "config".into() });
        }
        for d in gamedir::find_steam_installs(gamedir::STEAM_APP_ID) {
            v.push(DirCandidate { path: d, source: "steam".into() });
        }
        let mut seen = std::collections::HashSet::new();
        v.retain(|c| seen.insert(c.path.to_string_lossy().to_lowercase()));
        v
    }

    fn check_dir(&mut self, dir: &Path) -> DirCheck {
        let v = gamedir::validate(dir);
        let ok = v.files.iter().filter(|f| f.present && f.size_ok != Some(false) && f.sha_ok != Some(false)).count();
        DirCheck {
            path: dir.to_path_buf(),
            usable: v.usable(),
            exact: v.exact_build(),
            ok_files: ok,
            total_files: v.files.len(),
            missing: v.files.iter().filter(|f| f.required && !f.present).map(|f| f.name.clone()).collect(),
        }
    }

    fn pick_dir(&mut self) -> Option<PathBuf> {
        if self.headless || opts().no_picker {
            return None;
        }
        rfd::FileDialog::new().set_title(self.i18n.ui("app.pick_folder")).pick_folder()
    }

    fn game_dir(&self) -> Option<PathBuf> {
        self.dir.clone()
    }

    fn hd_state(&mut self) -> HdState {
        if let Some(inst) = &mut self.install {
            match inst {
                Install::Real { child, total, base, done, started, stop } => {
                    let d = done.load(Ordering::Relaxed).saturating_sub(*base).min(*total);
                    match child.try_wait() {
                        Ok(Some(st)) => {
                            let ok = st.success();
                            stop.store(1, Ordering::Relaxed);
                            self.install = None;
                            self.hd_count = None;
                            if !ok {
                                self.last_fail = Some(format!("newpal-patch exit {st}"));
                            }
                        }
                        _ => {
                            let el = started.elapsed().as_secs_f32();
                            let eta = (d > 20).then(|| el / d as f32 * (*total - d) as f32);
                            return HdState::Installing { done: d, total: *total, eta_s: eta, simulated: false };
                        }
                    }
                }
                Install::Sim { total, started } => {
                    let el = started.elapsed().as_secs_f32();
                    let rate = *total as f32 / 14.0;
                    let d = ((el * rate) as usize).min(*total);
                    if d < *total {
                        return HdState::Installing { done: d, total: *total, eta_s: Some((*total - d) as f32 / rate), simulated: true };
                    }
                    self.install = None;
                }
            }
        }
        if let Some(e) = self.last_fail.take() {
            return HdState::Failed(e);
        }
        if self.hd_ready() {
            if self.hd_count.is_none() {
                self.hd_count = Some(count_files(&self.hd_cache));
            }
            return HdState::Ready { items: self.hd_count.unwrap_or(0) };
        }
        let (items, bytes) = self.find_patch().map(|p| (p.items, p.bytes)).unwrap_or((0, 0));
        HdState::Missing { patch_items: items, patch_bytes: bytes }
    }

    fn start_hd_install(&mut self) {
        if self.install.is_some() {
            return;
        }
        let game = self.dir.clone();
        let hd = self.hd_cache.clone();
        let patch = self.find_patch().map(|p| (p.dir.clone(), p.items));
        match (game, patch, patch_exe()) {
            (Some(game), Some((pdir, total)), Some(exe)) => {
                let workers = std::thread::available_parallelism().map(|n| (n.get() / 2).max(1)).unwrap_or(2);
                eprintln!("[hd] {} apply --patch {} --hd-cache {} --workers {workers}", exe.display(), pdir.display(), hd.display());
                let _ = std::fs::create_dir_all(&hd);
                match Command::new(&exe)
                    .args(["apply", "--game-dir"])
                    .arg(&game)
                    .arg("--patch")
                    .arg(&pdir)
                    .arg("--hd-cache")
                    .arg(&hd)
                    .args(["--workers", &workers.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    Ok(child) => {
                        let base = count_files(&hd);
                        let done = Arc::new(AtomicUsize::new(base));
                        let stop = Arc::new(AtomicUsize::new(0));
                        let (d2, s2, h2) = (done.clone(), stop.clone(), hd.clone());
                        // progress = images written into the cache (apply prints no progress)
                        std::thread::spawn(move || {
                            while s2.load(Ordering::Relaxed) == 0 {
                                d2.store(count_files(&h2), Ordering::Relaxed);
                                std::thread::sleep(std::time::Duration::from_millis(700));
                            }
                        });
                        self.install = Some(Install::Real { child, total, base, done, started: Instant::now(), stop });
                    }
                    Err(e) => self.last_fail = Some(format!("{}: {e}", exe.display())),
                }
            }
            (_, patch, _) => {
                // no package / tool: clearly-labelled simulation so the flow can be tried
                let total = patch.map(|p| p.1).unwrap_or(20865);
                self.install = Some(Install::Sim { total, started: Instant::now() });
            }
        }
    }

    fn cancel_hd_install(&mut self) {
        if let Some(Install::Real { mut child, stop, .. }) = self.install.take() {
            let _ = child.kill();
            let _ = child.wait();
            stop.store(1, Ordering::Relaxed);
        }
        self.install = None;
    }

    fn saves(&mut self) -> Vec<SaveSlot> {
        Vec::new() // TODO(save system): slots + "continue" = newest slot
    }

    fn mods(&mut self) -> Vec<ModEntry> {
        let Some(md) = &self.mods_dir else { return Vec::new() };
        let cfg = Config::load_from(&self.cfg_path).unwrap_or_default();
        discover_mods(md, &[])
            .into_iter()
            .map(|m| ModEntry { enabled: cfg.mods.is_empty() || cfg.mods.contains(&m.name), name: m.name, priority: m.priority, dir: m.dir.display().to_string() })
            .collect()
    }

    fn mods_dir(&self) -> Option<PathBuf> {
        self.mods_dir.clone()
    }

    fn languages(&mut self) -> Vec<LangEntry> {
        self.i18n
            .available()
            .into_iter()
            .map(|l| {
                // autonym from the catalog (`ui:lang.<tag>`), else the pack's own name
                let key = format!("lang.{}", l.tag);
                let mut name = if newpal_assets::i18n::builtin::get("en", &format!("ui:{key}")).is_some() { self.i18n.ui(&key) } else { l.name.clone() };
                if l.tag == newpal_assets::i18n::ORIGINAL_LOCALE {
                    name.push_str(&self.i18n.ui("lang.original_mark"));
                }
                LangEntry { name, tag: l.tag }
            })
            .collect()
    }

    fn resolutions(&self) -> Vec<(u32, u32)> {
        self.resolutions.clone()
    }

    fn display_hz(&self) -> Option<u32> {
        self.display_hz
    }

    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").into()
    }
}

// ---------------------------------------------------------------- core (shared by window + headless)

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Launcher,
    Title,
    Game,
}

pub struct Core {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    r: Renderer,
    ui: UiGpu,
    fonts: Fonts,
    strings: Strings,
    stack: SceneStack,
    pub settings: Settings,
    host: GameHost,
    vfs: Option<Vfs>,
    world: Option<World>,
    phase: Phase,
    audio: Audio,
    fixed: FixedStep,
    args: Args,
    cfg: Config,
    view_h: f32,
    time: f32,
    pub quit: bool,
    /// window mode / size / vsync / frame cap changed
    pub window_dirty: bool,
    device_kind: Device,
    mouse: (f32, f32),
    size: (u32, u32),
    /// vector dialogue box for `world.dialogue`: (identity of the line, view)
    dlg: Option<((usize, String, u64), DialogueView)>,
    dlg_theme: DialogueTheme,
}

impl Core {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat, args: Args, cfg: Config, cfg_path: PathBuf, headless: bool) -> Result<Self, String> {
        let o = opts();
        let mut settings = Settings::load(&cfg_path);
        if let Some(s) = o.ui_scale {
            settings.ui_scale = s;
        }
        let i18n = crate::make_i18n(&args, &cfg, &[]);
        let strings = Strings::with_source(Arc::new(I18nSource(i18n.clone())));
        let t0 = Instant::now();
        let ui = UiGpu::new(&device, format)?;
        eprintln!("[ui] vello pipelines ready in {} ms", t0.elapsed().as_millis());
        let fonts = Fonts::load(Vec::new());
        let r = Renderer::new(device.clone(), queue.clone(), format);
        let mods_dir = args.mods_dir.clone().or(cfg.mods_dir.clone()).or_else(|| gamedir::exe_dir().map(|d| d.join("mods")));
        let host = GameHost {
            cfg_path: cfg_path.clone(),
            dir: None,
            cli_dir: args.game_dir.clone(),
            hd_cache: args.hd_cache.clone().unwrap_or_else(newpal_prepare::hd::default_dir),
            mods_dir,
            i18n,
            install: None,
            hd_count: None,
            patch: None,
            last_fail: None,
            resolutions: vec![(1280, 720), (1600, 900), (1920, 1080), (2560, 1440), (3840, 2160)],
            display_hz: None,
            headless,
        };
        let mut audio = Audio::silent();
        if !args.mute && !headless {
            audio = Audio::open();
        }
        let view_h = args.view_height.or(cfg.view_height).unwrap_or(480) as f32;
        let fixed = FixedStep::from_hz(args.video.logic_hz);
        let mut core = Self {
            device,
            queue,
            r,
            ui,
            fonts,
            strings,
            stack: SceneStack::new(),
            settings,
            host,
            vfs: None,
            world: None,
            phase: Phase::Launcher,
            audio,
            fixed,
            args,
            cfg,
            view_h,
            time: 0.0,
            quit: false,
            window_dirty: false,
            device_kind: Device::Keyboard,
            mouse: (0.0, 0.0),
            size: (1280, 720),
            dlg: None,
            dlg_theme: DialogueTheme::default(),
        };
        core.apply_settings();
        // configured + valid folder -> straight to the title (with a fade); else the first-run launcher
        let known = core.args.game_dir.clone().or(core.settings.game_dir.clone()).or_else(|| std::env::var(gamedir::ENV_GAME_DIR).ok().map(PathBuf::from));
        let valid = known.filter(|d| gamedir::validate(d).usable());
        match (valid, o.first_run) {
            (Some(d), false) => {
                core.open_game(d);
                core.enter_title();
                core.stack.push(Box::new(TitleScreen::new()));
            }
            _ => core.stack.push_instant(Box::new(LauncherScreen::new())),
        }
        Ok(core)
    }

    fn open_game(&mut self, dir: PathBuf) {
        let mods = self.host.mods_dir.as_deref().map(|d| discover_mods(d, &self.cfg.mods)).unwrap_or_default();
        let i18n = crate::make_i18n(&self.args, &self.cfg, &mods);
        if let Some(l) = &self.settings.language {
            let _ = i18n.set_locale(l);
        }
        let mut vfs = Vfs::new(dir.clone(), mods);
        vfs.set_i18n(i18n.clone());
        let hd = self.host.hd_cache.clone();
        if self.host.hd_ready() {
            vfs.set_hd_cache(Some(hd));
            vfs.set_hd_enabled(self.settings.present == "hd");
        }
        self.strings = Strings::with_source(Arc::new(I18nSource(i18n.clone())));
        self.host.i18n = i18n;
        self.host.dir = Some(dir);
        self.dlg_theme = load_dialogue_theme(&vfs);
        self.fonts = Fonts::load(game_faces(&vfs));
        self.vfs = Some(vfs);
    }

    fn load_world(&mut self, script: u32) {
        let Some(vfs) = &self.vfs else { return };
        match World::load(vfs, &mut self.r, &self.args.lmf, self.args.map_id, self.view_h) {
            Ok(mut w) => {
                w.look = self.args.video.look;
                w.look.present = present_of(&self.settings.present);
                let (x, y) = self.args.pos.unwrap_or((w.hero.x, w.hero.y));
                w.place_hero(x, y);
                if script != 0 {
                    if let Err(e) = w.start_script(vfs, script) {
                        eprintln!("script {script}: {e}");
                    }
                }
                w.snap_visuals();
                w.vector_dialogue = true;
                self.world = Some(w);
                self.dlg = None;
            }
            Err(e) => {
                eprintln!("[shell] map {}:{}: {e}", self.args.lmf, self.args.map_id);
                self.world = None;
            }
        }
        self.fixed = FixedStep::from_hz(self.args.video.logic_hz);
    }

    fn enter_title(&mut self) {
        self.phase = Phase::Title;
        self.load_world(0);
    }

    fn apply_settings(&mut self) {
        let p = present_of(&self.settings.present);
        if let Some(w) = &mut self.world {
            w.look.present = p;
            w.snap_visuals();
        }
        if let Some(v) = &self.vfs {
            v.set_hd_enabled(self.settings.present == "hd");
        }
        self.audio.volume = self.settings.master * self.settings.music;
        self.window_dirty = true;
    }

    fn handle(&mut self, c: Cmd) {
        match c {
            Cmd::UseGameDir(p) => {
                self.settings.game_dir = Some(p.clone());
                let _ = self.settings.save(&self.host.cfg_path);
                self.open_game(p);
            }
            Cmd::EnterTitle => self.enter_title(),
            Cmd::NewGame => {
                let s = self.args.script;
                self.load_world(s);
                self.phase = Phase::Game;
                self.stack.clear();
            }
            Cmd::Continue | Cmd::LoadSlot(_) => eprintln!("[shell] save system not implemented yet"),
            Cmd::ApplySettings => self.apply_settings(),
            Cmd::SaveSettings => {
                if let Err(e) = self.settings.save(&self.host.cfg_path) {
                    eprintln!("[shell] settings: {e}");
                }
            }
            Cmd::SetLanguage(tag) => {
                let i = self.host.i18n.clone();
                match i.set_locale(&tag) {
                    Ok(l) => eprintln!("[shell] language {}", l.tag),
                    Err(e) => eprintln!("[shell] language {tag}: {e}"),
                }
                if let (Some(w), Some(v)) = (&mut self.world, &self.vfs) {
                    let _ = w.set_locale(v, &tag);
                }
                self.settings.language = Some(tag);
            }
            Cmd::ToggleMod(name) => self.host.toggle_mod(&name),
            Cmd::ToTitle => {
                self.stack.clear();
                self.enter_title();
                self.stack.push(Box::new(TitleScreen::new()));
            }
            Cmd::Resume => {}
            Cmd::Quit => self.quit = true,
        }
    }

    /// One frame of logic. `w`,`h` = target size in physical pixels.
    pub fn update(&mut self, inp: &Input, dt: f32, w: u32, h: u32) {
        self.time += dt;
        self.size = (w, h);
        self.device_kind = inp.device;
        self.mouse = inp.mouse;
        let (wf, hf) = (w as f32, h as f32);
        // gameplay input when no screen is open
        if self.phase == Phase::Game && !self.stack.active() {
            if inp.hit(Action::Menu) {
                // original-style system menu; money stays hidden until the VM exposes it
                self.stack.push(Box::new(SysMenuScreen::new(None)));
            } else if let Some(wd) = &mut self.world {
                let held = |a| inp.held.contains(&a) as i32 as f32;
                let mut mv = (held(Action::Right) - held(Action::Left), held(Action::Down) - held(Action::Up));
                if mv == (0.0, 0.0) && (inp.stick.0.abs() > 0.3 || inp.stick.1.abs() > 0.3) {
                    mv = inp.stick;
                }
                wd.input = mv;
                if inp.hit(Action::Confirm) || (inp.click && wd.dialogue.is_some()) {
                    match (&mut self.dlg, wd.dialogue.is_some()) {
                        (Some((_, v)), true) => {
                            if v.confirm() == DlgEvent::Done {
                                wd.advance_dialogue();
                            }
                        }
                        (_, true) => wd.advance_dialogue(),
                        (_, false) => {
                            wd.interact();
                            wd.advance_dialogue();
                        }
                    }
                } else if inp.click {
                    {
                        let t = wd.screen_to_world(w, h, inp.mouse.0, inp.mouse.1);
                        wd.walk_to(t.0, t.1);
                    }
                }
            }
        } else if let Some(wd) = &mut self.world {
            wd.input = (0.0, 0.0);
        }
        let u = layout_unit(wf, hf, self.settings.ui_scale);
        let mut cmds = Vec::new();
        {
            let mut cx = Ctx { s: &self.strings, fonts: &self.fonts, host: &mut self.host, settings: &mut self.settings, w: wf, h: hf, u, time: self.time, in_game: self.phase == Phase::Game, cmds: &mut cmds };
            self.stack.update(&mut cx, inp, dt);
        }
        for c in cmds {
            self.handle(c);
        }
        // world: alive on the title, frozen while a menu is open in game
        let run_world = match self.phase {
            Phase::Launcher => false,
            Phase::Title => true,
            Phase::Game => !self.stack.active() || self.stack.top_name().is_none(),
        };
        if run_world {
            if let (Some(wd), Some(vfs)) = (&mut self.world, &self.vfs) {
                for _ in 0..self.fixed.advance(dt) {
                    wd.step(vfs, &mut self.r, self.fixed.step);
                    sync_audio(vfs, &mut self.audio, wd);
                }
                wd.alpha = self.fixed.alpha();
            }
            self.sync_dialogue(dt);
        }
    }

    /// Keep the vector dialogue box in step with the world's current line (typewriter on real time).
    fn sync_dialogue(&mut self, dt: f32) {
        let Some(wd) = &self.world else {
            self.dlg = None;
            return;
        };
        let Some(db) = &wd.dialogue else {
            self.dlg = None;
            return;
        };
        let i18n = &self.host.i18n;
        let id = (db.d.pos, db.d.raw.clone(), i18n.generation());
        if self.dlg.as_ref().map(|(k, _)| k) != Some(&id) {
            let d = &db.d;
            // a translated line carries its own speaker; names are mapped only for untranslated lines
            let speaker = d.name.as_ref().map(|n| if d.overridden { n.clone() } else { i18n.tr_name(n) });
            let lines = d.lines.iter().map(|l| l.iter().map(|sp| DlgSpan { color: sp.color, text: sp.text.clone() }).collect()).collect();
            use newpal_assets::i18n::markup::CueKind;
            let cues = d
                .cues
                .iter()
                .map(|c| {
                    let k = match &c.kind {
                        CueKind::Page => DlgCue::Page,
                        CueKind::Instant => DlgCue::Instant,
                        CueKind::Speed(n) => DlgCue::Speed(*n),
                        CueKind::Glyph(n) => DlgCue::Glyph(*n),
                        CueKind::Ctl(a, b) => DlgCue::Ctl(*a, *b),
                    };
                    (c.line, c.col, k)
                })
                .collect();
            let v = DialogueView::new(&self.fonts, self.dlg_theme.clone(), DlgText { speaker, lines, cues }, &i18n.ui("dialogue.name_sep"));
            if !v.unknown_cues.is_empty() {
                eprintln!("[dialogue] cues not modelled yet: {:?}", v.unknown_cues);
            }
            self.dlg = Some((id, v));
        }
        if let Some((_, v)) = &mut self.dlg {
            v.update(dt);
        }
    }

    /// Draw the game frame (if any) and composite the UI on top.
    pub fn render(&mut self, view: &wgpu::TextureView, w: u32, h: u32) -> Result<(), String> {
        if let Some(wd) = &mut self.world {
            wd.draw(&mut self.r, w, h);
            if self.phase == Phase::Title {
                // slow drift + mouse parallax of the camera behind the title (clamped to the map)
                let mut cam = wd.camera(w, h);
                let par = ((self.mouse.0 / w.max(1) as f32 - 0.5) * 2.0, (self.mouse.1 / h.max(1) as f32 - 0.5) * 2.0);
                let dx = (self.time * 0.07).sin() * 46.0 + par.0 * 10.0;
                let dy = (self.time * 0.05).cos() * 16.0 + par.1 * 6.0;
                cam.origin[0] = (cam.origin[0] + dx).clamp(0.0, (wd.size.0 - cam.size[0]).max(0.0));
                cam.origin[1] = (cam.origin[1] + dy).clamp(0.0, (wd.size.1 - cam.size[1]).max(0.0));
                self.r.set_camera(CAM_WORLD, cam);
            }
        }
        self.r.render(view, CLEAR);
        let dlg = self.dlg.as_ref().filter(|_| self.phase == Phase::Game).map(|(_, v)| v);
        if self.stack.active() || dlg.is_some() {
            let mut sc = vello::Scene::new();
            if let Some(v) = dlg {
                v.draw(&mut sc, &self.fonts, w as f32, h as f32, 1.0);
            }
            let u = layout_unit(w as f32, h as f32, self.settings.ui_scale);
            let cx = DrawCx {
                s: &self.strings,
                fonts: &self.fonts,
                settings: &self.settings,
                w: w as f32,
                h: h as f32,
                u,
                time: self.time,
                device: self.device_kind,
                has_world: self.world.is_some() && self.phase != Phase::Launcher,
                in_game: self.phase == Phase::Game,
                mouse: self.mouse,
            };
            self.stack.draw(&mut sc, &cx);
            self.ui.render(&self.device, &self.queue, &sc, view, w, h)?;
        }
        Ok(())
    }

    pub fn status(&self) -> String {
        format!("{:?} [{}] ui {:.1} ms", self.phase, self.stack.names().join(">"), self.ui.last_ms)
    }
}

/// Dialogue look: the built-in original-style theme, overridable by a mod's `ui/dialogue.toml`.
fn load_dialogue_theme(vfs: &Vfs) -> DialogueTheme {
    let builtin = DialogueTheme::from_toml(newpal_ui::themes::ORIGINAL_DIALOGUE).unwrap_or_default();
    match vfs.find_override("ui/dialogue.toml").and_then(|(p, _, _)| std::fs::read_to_string(p).ok()) {
        Some(src) => DialogueTheme::from_toml(&src).unwrap_or_else(|e| {
            eprintln!("[dialogue] ui/dialogue.toml: {e}");
            builtin
        }),
        None => builtin,
    }
}

/// In-game faces for the UI font chain: the player's `Resource/CT.ttf` first, then language-pack / mod / system
/// fonts in the order the i18n layer chose (read at runtime, never shipped).
fn game_faces(vfs: &Vfs) -> Vec<Face> {
    let mut v: Vec<(Vec<u8>, String)> = vfs.font_stack().unwrap_or_default();
    v.sort_by_key(|(_, d)| !d.contains("CT.ttf"));
    v.into_iter().filter_map(|(b, d)| Face::from_bytes(b, 0, &d)).collect()
}

fn present_of(s: &str) -> Present {
    match s {
        "shader" => Present::Shader,
        "original" => Present::Original,
        _ => Present::Hd,
    }
}

// ---------------------------------------------------------------- scripted input (screenshots / smoke tests)

#[derive(Debug)]
enum Step {
    Wait(f32),
    Key(Action),
    Click(f32, f32),
    Move(f32, f32),
    Device(Device),
    Shot(String),
    Rec(f32),
    Size(u32, u32),
    Quit,
}

fn parse_script(s: &str) -> Result<Vec<Step>, String> {
    let mut v = Vec::new();
    for part in s.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (k, a) = part.split_once(':').unwrap_or((part, ""));
        let xy = |a: &str| -> Result<(f32, f32), String> {
            let (x, y) = a.split_once(',').ok_or(format!("bad xy {a}"))?;
            Ok((x.parse().map_err(|_| format!("bad x {x}"))?, y.parse().map_err(|_| format!("bad y {y}"))?))
        };
        v.push(match k {
            "wait" => Step::Wait(a.parse().map_err(|_| format!("bad wait {a}"))?),
            "key" => Step::Key(Action::from_id(a).ok_or(format!("unknown action {a}"))?),
            "click" => {
                let (x, y) = xy(a)?;
                Step::Click(x, y)
            }
            "move" => {
                let (x, y) = xy(a)?;
                Step::Move(x, y)
            }
            "pad" => Step::Device(Device::Gamepad),
            "kbd" => Step::Device(Device::Keyboard),
            "shot" => Step::Shot(a.to_string()),
            "rec" => Step::Rec(a.parse().map_err(|_| format!("bad rec {a}"))?),
            "size" => {
                let (w, h) = a.split_once('x').ok_or("size WxH")?;
                Step::Size(w.parse().map_err(|_| "bad w")?, h.parse().map_err(|_| "bad h")?)
            }
            "quit" => Step::Quit,
            other => return Err(format!("unknown script step {other}")),
        });
    }
    Ok(v)
}

struct Driver {
    steps: Vec<Step>,
    i: usize,
    wait: f32,
    rec: f32,
    device: Device,
    mouse: (f32, f32),
}

/// What the driver wants this frame.
#[derive(Default)]
struct Want {
    shot: Option<String>,
    rec_frame: bool,
    size: Option<(u32, u32)>,
    quit: bool,
}

impl Driver {
    fn new(script: &str) -> Result<Self, String> {
        Ok(Self { steps: parse_script(script)?, i: 0, wait: 0.0, rec: 0.0, device: Device::Mouse, mouse: (-1.0, -1.0) })
    }
    /// Feed scripted events into `inp` (after real events); returns requests.
    fn frame(&mut self, inp: &mut Input, dt: f32, w: u32, h: u32) -> Want {
        let mut want = Want::default();
        if self.rec > 0.0 {
            self.rec -= dt;
            want.rec_frame = true;
        }
        if self.wait > 0.0 {
            self.wait -= dt;
        } else {
            while self.i < self.steps.len() {
                let st = &self.steps[self.i];
                self.i += 1;
                match st {
                    Step::Wait(s) => {
                        self.wait = *s;
                        break;
                    }
                    Step::Rec(s) => {
                        self.rec = *s;
                        self.wait = *s;
                        break;
                    }
                    Step::Key(a) => {
                        inp.pressed.push(*a);
                        if self.device == Device::Mouse {
                            self.device = Device::Keyboard;
                        }
                        break; // one action per frame, like a real key press
                    }
                    Step::Click(x, y) => {
                        self.mouse = (x * w as f32, y * h as f32);
                        inp.mouse = self.mouse;
                        inp.mouse_moved = true;
                        inp.click = true;
                        self.device = Device::Mouse;
                        break;
                    }
                    Step::Move(x, y) => {
                        self.mouse = (x * w as f32, y * h as f32);
                        inp.mouse = self.mouse;
                        inp.mouse_moved = true;
                        self.device = Device::Mouse;
                    }
                    Step::Device(d) => self.device = *d,
                    Step::Shot(n) => {
                        want.shot = Some(n.clone());
                        break;
                    }
                    Step::Size(w, h) => {
                        want.size = Some((*w, *h));
                        break;
                    }
                    Step::Quit => {
                        want.quit = true;
                        break;
                    }
                }
            }
            if self.i >= self.steps.len() && self.wait <= 0.0 && want.shot.is_none() && want.size.is_none() {
                want.quit = true;
            }
        }
        if self.mouse.0 >= 0.0 {
            inp.mouse = self.mouse;
        }
        inp.device = self.device;
        want
    }
}

// ---------------------------------------------------------------- headless runner

fn readback(device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture, w: u32, h: u32) -> newpal_assets::image::RgbaImage {
    let padded = (4 * w).div_ceil(256) * 256;
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("ui-readback"), size: (padded * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ui-copy") });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let data = slice.get_mapped_range().expect("map readback buffer");
    let mut img = newpal_assets::image::RgbaImage::new(w, h);
    for y in 0..h as usize {
        img.pixels[y * 4 * w as usize..(y + 1) * 4 * w as usize].copy_from_slice(&data[y * padded as usize..y * padded as usize + 4 * w as usize]);
    }
    drop(data);
    buf.unmap();
    img
}

fn headless(args: Args, cfg: Config, cfg_path: PathBuf, script: &str, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let inst = instance();
    let (_a, dev, queue) = pollster::block_on(device(&inst, None))?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let (mut w, mut h) = args.size;
    let mut core = Core::new(dev.clone(), queue.clone(), format, args, cfg, cfg_path, true)?;
    let mut driver = Driver::new(script)?;
    let mut mapper = InputMapper::headless(core.settings.bindings.clone());
    let dt = 1.0 / 60.0;
    let mut rec_n = 0u32;
    let mut frame_i = 0u64;
    let mk = |w: u32, h: u32| {
        let tex = dev.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui-shot"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        (tex, view)
    };
    let (mut tex, mut view) = mk(w, h);
    for _ in 0..60 * 600 {
        mapper.bindings = core.settings.bindings.clone();
        let mut inp = mapper.frame(dt);
        let want = driver.frame(&mut inp, dt, w, h);
        if let Some((nw, nh)) = want.size {
            w = nw;
            h = nh;
            (tex, view) = mk(w, h);
            continue;
        }
        core.update(&inp, dt, w, h);
        frame_i += 1;
        let need = want.shot.is_some() || (want.rec_frame && frame_i % 2 == 0);
        if need || frame_i % 30 == 0 {
            core.render(&view, w, h)?;
        }
        if let Some(n) = &want.shot {
            let t0 = Instant::now();
            let img = readback(&dev, &queue, &tex, w, h);
            let p = dir.join(format!("{n}.png"));
            img.save_rgb_png(&p).map_err(|e| e.to_string())?;
            eprintln!("[shot] {} {w}x{h} {} ({} ms readback)", p.display(), core.status(), t0.elapsed().as_millis());
        } else if want.rec_frame && frame_i % 2 == 0 {
            let img = readback(&dev, &queue, &tex, w, h);
            rec_n += 1;
            let fd = dir.join("frames");
            let _ = std::fs::create_dir_all(&fd);
            img.save_rgb_png(&fd.join(format!("f_{rec_n:05}.png"))).map_err(|e| e.to_string())?;
        }
        if want.quit || core.quit {
            break;
        }
    }
    if rec_n > 0 {
        eprintln!("[rec] {rec_n} frames (30 fps) -> {}", dir.join("frames").display());
    }
    Ok(())
}

// ---------------------------------------------------------------- window

struct Win {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    core: Core,
}

struct ShellApp {
    args: Option<Args>,
    cfg: Option<Config>,
    cfg_path: PathBuf,
    win: Option<Win>,
    mapper: InputMapper,
    driver: Option<Driver>,
    pacer: Pacer,
    last: Instant,
    error: Option<String>,
    title_t: f32,
}

fn apply_window(win: &mut Win, pacer: &mut Pacer) {
    let s = &win.core.settings;
    let w = &win.window;
    match s.window_mode {
        WindowMode::Windowed => {
            if w.fullscreen().is_some() {
                w.set_fullscreen(None);
            }
            let cur = w.inner_size();
            if (cur.width, cur.height) != s.resolution {
                let _ = w.request_inner_size(winit::dpi::PhysicalSize::new(s.resolution.0, s.resolution.1));
            }
        }
        WindowMode::Borderless => {
            if !matches!(w.fullscreen(), Some(Fullscreen::Borderless(_))) {
                w.set_fullscreen(Some(Fullscreen::Borderless(None)));
            }
        }
        WindowMode::Fullscreen => {
            let mode = w.current_monitor().and_then(|m| {
                m.video_modes().filter(|v| (v.size().width, v.size().height) == s.resolution).max_by_key(|v| v.refresh_rate_millihertz())
            });
            match mode {
                Some(m) => w.set_fullscreen(Some(Fullscreen::Exclusive(m))),
                None => w.set_fullscreen(Some(Fullscreen::Borderless(None))),
            }
        }
    }
    let pm = if s.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
    if win.config.present_mode != pm {
        win.config.present_mode = pm;
        win.surface.configure(&win.core.device, &win.config);
    }
    pacer.cap = s.refresh;
}

impl ShellApp {
    fn init(&mut self, el: &ActiveEventLoop) -> Result<(), String> {
        let settings = Settings::load(&self.cfg_path);
        let (w, h) = settings.resolution;
        let attrs = Window::default_attributes().with_title(newpal_assets::i18n::builtin::text(settings.language.as_deref().unwrap_or(newpal_assets::i18n::ORIGINAL_LOCALE), "app.title")).with_inner_size(winit::dpi::PhysicalSize::new(w, h));
        let window = Arc::new(el.create_window(attrs).map_err(|e| e.to_string())?);
        let inst = instance();
        let surface = inst.create_surface(window.clone()).map_err(|e| e.to_string())?;
        let (adapter, dev, queue) = pollster::block_on(device(&inst, Some(&surface)))?;
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).ok_or("surface unsupported")?;
        config.format = format;
        config.present_mode = if settings.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        surface.configure(&dev, &config);
        let mut core = Core::new(dev, queue, format, self.args.take().unwrap(), self.cfg.take().unwrap(), self.cfg_path.clone(), false)?;
        if let Some(m) = window.current_monitor() {
            let mut res: Vec<(u32, u32)> = m.video_modes().map(|v| (v.size().width, v.size().height)).filter(|(w, h)| *w >= 1280 && *h >= 720).collect();
            res.sort();
            res.dedup();
            if !res.is_empty() {
                core.host.resolutions = res;
            }
            core.host.display_hz = m.refresh_rate_millihertz().map(|mhz| (mhz + 500) / 1000);
        }
        self.mapper = InputMapper::new(core.settings.bindings.clone());
        if let Some(n) = &self.mapper.gamepad_name {
            eprintln!("[input] gamepad: {n}");
        }
        let mut win = Win { window, surface, config, core };
        apply_window(&mut win, &mut self.pacer);
        win.core.window_dirty = false;
        self.win = Some(win);
        Ok(())
    }

    fn redraw(&mut self, el: &ActiveEventLoop) {
        let Some(win) = self.win.as_mut() else { return };
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        self.pacer.frame(dt);
        let (w, h) = (win.config.width, win.config.height);
        self.mapper.bindings = win.core.settings.bindings.clone();
        let mut inp = self.mapper.frame(dt);
        if let Some(d) = &mut self.driver {
            let want = d.frame(&mut inp, dt, w, h);
            if want.quit {
                el.exit();
            }
        }
        win.core.update(&inp, dt, w, h);
        if win.core.quit {
            el.exit();
            return;
        }
        if win.core.window_dirty {
            win.core.window_dirty = false;
            apply_window(win, &mut self.pacer);
        }
        let frame = match win.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                win.surface.configure(&win.core.device, &win.config);
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        if let Err(e) = win.core.render(&view, w, h) {
            self.error = Some(e);
            el.exit();
            return;
        }
        win.core.queue.present(frame);
        self.title_t += dt;
        if self.title_t > 0.5 {
            self.title_t = 0.0;
            win.window.set_title(&format!("{} \u{2014} {:.0} fps  {}", win.core.host.i18n.ui("app.title"), self.pacer.fps, win.core.status()));
        }
    }
}

impl ApplicationHandler for ShellApp {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.win.is_none() {
            if let Err(e) = self.init(el) {
                self.error = Some(e);
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        self.mapper.on_window_event(&ev);
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                if let Some(win) = self.win.as_mut() {
                    win.config.width = s.width.max(1);
                    win.config.height = s.height.max(1);
                    win.surface.configure(&win.core.device, &win.config);
                    if win.core.settings.window_mode == WindowMode::Windowed {
                        win.core.settings.resolution = (win.config.width, win.config.height);
                    }
                }
            }
            WindowEvent::RedrawRequested => self.redraw(el),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if let Some(win) = &self.win {
            win.window.request_redraw();
        }
        match self.pacer.deadline(Instant::now()) {
            Some(t) => el.set_control_flow(ControlFlow::WaitUntil(t)),
            None => el.set_control_flow(ControlFlow::Poll),
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        if let Some(win) = &mut self.win {
            win.core.host.cancel_hd_install();
            let _ = win.core.settings.save(&win.core.host.cfg_path);
        }
    }
}

pub fn run(args: Args, cfg: Config, cfg_path: PathBuf) -> Result<(), String> {
    let o = opts();
    if let (Some(dir), Some(script)) = (&o.shot_dir, &o.script) {
        return headless(args, cfg, cfg_path, script, dir);
    }
    let driver = match &o.script {
        Some(s) => Some(Driver::new(s)?),
        None => None,
    };
    let el = EventLoop::new().map_err(|e| e.to_string())?;
    let mut app = ShellApp { args: Some(args), cfg: Some(cfg), cfg_path, win: None, mapper: InputMapper::headless(Bindings::default()), driver, pacer: Pacer::new(0), last: Instant::now(), error: None, title_t: 0.0 };
    el.run_app(&mut app).map_err(|e| e.to_string())?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
