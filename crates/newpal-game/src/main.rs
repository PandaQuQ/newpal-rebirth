//! `newpal` — New PAL reborn client. Runs on the player's own original game data (never bundled).
mod app;
mod audio;
mod prepare;
mod render;
mod text;
mod ui_sheet;
mod timing;
mod world;
mod hd_compare;
mod rife_flow_proto;

use newpal_assets::gamedir::{self, Config};
use newpal_assets::vfs::{discover_mods, Vfs};
use std::path::PathBuf;

const HELP: &str = "newpal - New PAL reborn (needs your own copy of the original game)

USAGE: newpal [options]
  --game-dir <path>     original game folder (else: NEWPAL_GAME_DIR, newpal.toml, Steam auto-detect)
  --config <file>       config file (default: newpal.toml next to exe, else %APPDATA%\\NewPAL-Reborn\\newpal.toml)
  --pick                choose the game folder with a dialog and save it to the config
  --validate            check the game folder and exit
  --mods-dir <path>     mods folder (default: <exe dir>\\mods)
  --map <lmf>:<id>      start map, e.g. huge:2 (default) or huge2:277
  --script <n>          talk script to start (default 104 = opening); 0 = none
  --pos <x>,<y>         hero start position (map pixels)
  --view-height <px>    logical view height (480 = original); width follows window aspect
  --size <w>x<h>        window / screenshot size (default 1280x720)
  --screenshot <png>    render one frame offscreen to a PNG and exit
  --walk-to <x>,<y>     (screenshot) click-to-walk target, simulated before the frame
  --sim <seconds>       (screenshot) simulate this long before rendering (default 0)
  --interact            (screenshot) press the action key after the simulation
  --fps-cap <hz>        frame cap (0 = none; default 0 = display rate with vsync)
  --no-vsync / --vsync  present mode (default vsync)
  --no-interp           no interpolation / camera easing / UI fades (original stepping)
  --no-smooth-frames    ignore mod in-between frames
  --crossfade           cross-fade original frames (cheap smoothing without mods)
  --original            = --no-interp --no-smooth-frames, no cross-fade (A/B compare)
  --logic-hz <hz>       logic tick rate (default original 28.6 = 35 ms)
  --frames <n>          (screenshot) save n consecutive frames <png>_000.png ..
  --render-hz <hz>      (screenshot) simulated display rate (default 144)
  --prepare             launcher asset preparation: generate in-between frames from your data into the
                        local cache (<data_root>\\cache\\smooth.pak), then exit
                        (or continue to --screenshot). Runs automatically on launch when the cache is missing.
  --regenerate          like --prepare but recompute everything
  --smooth-backend <b>  auto (RIFE on a Vulkan GPU, else CPU) | gpu | cpu | off
  --smooth-quality <n>  2 or 4 (in-betweens per original frame)
  --workers <n>         parallel prepare workers (default 2)
  --prepare-only <f>    limit to all_char[:slot[:action]]   --prepare-limit <n>  first n animations
  --smooth-cache <file> cache file (default above)
  --prepare-tools       only download + verify the pinned rife-ncnn-vulkan (no GPU work), then exit
  --hd-prepare          run full 4x static HD stills (resumable) then exit
  --data-root <path>    project data root (cache/tools/output); else NEWPAL_DATA_ROOT or auto-detect
  --compare-hd          after HD cache is ready, write frame strips + APNG + metrics (GPU mesh-warp)
  --rife-flow-proto     RIFE 4x + flow-warp prototype on the hero walk (8 dirs); then exit
  --compare-out <dir>   compare output dir (default: <data_root>\\output\\screenshots\\hd-proto)
  --hd-cache <dir>      HD cache dir (default: <data_root>\\cache\\hd)
  --lang <tag>          language: zh-Hant (original), zh-Hans, en ... (else newpal.toml `language`, NEWPAL_LANG)
  --lang-dir <dir>      folder with language packs <dir>/<tag>/*.toml (also: `lang` next to the exe, [mods]/<mod>/lang/<tag>)
  --list-langs          list the selectable languages and exit
  --lang-then <tag>     (screenshot) switch language at runtime after the first frame is set up
  --advance <n>         (screenshot) advance the dialogue n lines before the frame
  --lang-missing <file> (screenshot) write the keys that fell back to the original text
  --ui-sheet <png>      render the game's own menu / battle / shop strings of the selected language to a PNG and exit
  --ui-cat <name>       (ui-sheet) sys | ui | menu | battle | shop | msg | name | err (default battle)
  --ui-from <id> --ui-count <n>   (ui-sheet) first id / how many strings (default 0 / 48)
  --mute                no audio output (screenshots are always silent; requests are logged)
  --debug-nav           overlay blocked cells / regions / path (F3 toggles in game)
Keys: F12 next language, F3 nav overlay, F4 interpolation, F5 in-between frames, F6 cross-fade, F7 fps cap (vsync/30/60/144/240), F8 vsync, F9 HD stills, WASD/arrows walk (8 dirs), left click = walk to, Enter/Space/click = next line, T = restart script, Esc = quit";

pub struct Args {
    pub game_dir: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub pick: bool,
    pub validate: bool,
    pub mods_dir: Option<PathBuf>,
    pub lmf: String,
    pub map_id: u32,
    pub script: u32,
    pub pos: Option<(f32, f32)>,
    pub view_height: Option<u32>,
    pub size: (u32, u32),
    pub screenshot: Option<PathBuf>,
    pub walk_to: Option<(f32, f32)>,
    pub sim: f32,
    pub debug_nav: bool,
    pub interact: bool,
    pub mute: bool,
    /// video settings from the CLI (override newpal.toml [video]); merged into `video`
    pub video_cli: gamedir::VideoConfig,
    pub video: Video,
    /// (screenshot) save this many consecutive frames (name_000.png ..) instead of one
    pub frames: u32,
    /// (screenshot) simulated display rate for --frames / --sim
    pub render_hz: f32,
    pub prepare: bool,
    pub regenerate: bool,
    pub smooth_backend: Option<String>,
    pub smooth_quality: Option<u32>,
    pub workers: Option<usize>,
    pub prepare_only: Option<String>,
    pub prepare_limit: Option<usize>,
    pub smooth_cache: Option<PathBuf>,
    pub prepare_tools: bool,
    pub hd_prepare: bool,
    pub compare_hd: bool,
    pub rife_flow_proto: bool,
    pub compare_out: Option<PathBuf>,
    pub hd_cache: Option<PathBuf>,
    pub data_root: Option<PathBuf>,
    pub no_hd: bool,
    pub lang: Option<String>,
    pub lang_dir: Option<PathBuf>,
    pub list_langs: bool,
    pub lang_then: Option<String>,
    pub advance: u32,
    pub lang_missing: Option<PathBuf>,
    pub ui_sheet: Option<PathBuf>,
    pub ui_cat: String,
    pub ui_from: u32,
    pub ui_count: usize,
}

/// Resolved frame pacing / smoothing settings.
#[derive(Clone, Copy, Debug)]
pub struct Video {
    pub vsync: bool,
    pub fps_cap: u32,
    pub look: world::Look,
    pub logic_hz: Option<f32>,
}

impl Video {
    pub fn resolve(cli: &gamedir::VideoConfig, cfg: &gamedir::VideoConfig) -> Self {
        let d = world::Look::default();
        Video {
            vsync: cli.vsync.or(cfg.vsync).unwrap_or(true),
            fps_cap: cli.fps_cap.or(cfg.fps_cap).unwrap_or(0),
            look: world::Look {
                interpolate: cli.interpolate.or(cfg.interpolate).unwrap_or(d.interpolate),
                smooth_frames: cli.smooth_frames.or(cfg.smooth_frames).unwrap_or(d.smooth_frames),
                crossfade: cli.crossfade.or(cfg.crossfade).unwrap_or(d.crossfade),
                present: cli.present.as_deref().or(cfg.present.as_deref()).and_then(world::Present::parse).unwrap_or(d.present),
                shader: cli.shader.as_deref().or(cfg.shader.as_deref()).and_then(world::ShaderKind::parse).unwrap_or(d.shader),
                flow: cli.flow.or(cfg.flow).unwrap_or(d.flow),
            },
            logic_hz: cli.logic_hz.or(cfg.logic_hz),
        }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        game_dir: None, config: None, pick: false, validate: false, mods_dir: None, lmf: "huge".into(), map_id: 2,
        script: 104, pos: None, view_height: None, size: (1280, 720), screenshot: None, walk_to: None, sim: 0.0, debug_nav: false, interact: false, mute: false,
        video_cli: Default::default(), video: Video::resolve(&Default::default(), &Default::default()), frames: 0, render_hz: 144.0,
        prepare: false, regenerate: false, smooth_backend: None, smooth_quality: None, workers: None, prepare_only: None, prepare_limit: None, smooth_cache: None, prepare_tools: false, hd_prepare: false, compare_hd: false, rife_flow_proto: false, compare_out: None, hd_cache: None, data_root: None, no_hd: false,
        lang: None, lang_dir: None, list_langs: false, lang_then: None, advance: 0, lang_missing: None, ui_sheet: None, ui_cat: "battle".into(), ui_from: 0, ui_count: 48,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{k} needs a value"));
        match k.as_str() {
            "--game-dir" => a.game_dir = Some(val()?.into()),
            "--config" => a.config = Some(val()?.into()),
            "--pick" => a.pick = true,
            "--validate" => a.validate = true,
            "--mods-dir" => a.mods_dir = Some(val()?.into()),
            "--map" => {
                let v = val()?;
                let (l, i) = v.split_once(':').ok_or("--map expects <lmf>:<id>")?;
                a.lmf = l.trim_end_matches(".lmf").to_string();
                a.map_id = i.parse().map_err(|_| "bad map id")?;
            }
            "--script" => a.script = val()?.parse().map_err(|_| "bad script id")?,
            "--pos" => {
                let v = val()?;
                let (x, y) = v.split_once(',').ok_or("--pos expects x,y")?;
                a.pos = Some((x.parse().map_err(|_| "bad x")?, y.parse().map_err(|_| "bad y")?));
            }
            "--view-height" => a.view_height = Some(val()?.parse().map_err(|_| "bad view height")?),
            "--size" => {
                let v = val()?;
                let (w, h) = v.split_once('x').ok_or("--size expects WxH")?;
                a.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
            }
            "--screenshot" => a.screenshot = Some(val()?.into()),
            "--walk-to" => {
                let v = val()?;
                let (x, y) = v.split_once(',').ok_or("--walk-to expects x,y")?;
                a.walk_to = Some((x.parse().map_err(|_| "bad x")?, y.parse().map_err(|_| "bad y")?));
            }
            "--sim" => a.sim = val()?.parse().map_err(|_| "bad --sim")?,
            "--debug-nav" => a.debug_nav = true,
            "--interact" => a.interact = true,
            "--mute" => a.mute = true,
            "--lang" => a.lang = Some(val()?),
            "--lang-dir" => a.lang_dir = Some(val()?.into()),
            "--list-langs" => a.list_langs = true,
            "--lang-then" => a.lang_then = Some(val()?),
            "--advance" => a.advance = val()?.parse().map_err(|_| "bad --advance")?,
            "--lang-missing" => a.lang_missing = Some(val()?.into()),
            "--ui-sheet" => a.ui_sheet = Some(val()?.into()),
            "--ui-cat" => a.ui_cat = val()?,
            "--ui-from" => a.ui_from = val()?.parse().map_err(|_| "bad --ui-from")?,
            "--ui-count" => a.ui_count = val()?.parse().map_err(|_| "bad --ui-count")?,
            "--fps-cap" => a.video_cli.fps_cap = Some(val()?.parse().map_err(|_| "bad fps cap")?),
            "--no-vsync" => a.video_cli.vsync = Some(false),
            "--vsync" => a.video_cli.vsync = Some(true),
            "--no-interp" => a.video_cli.interpolate = Some(false),
            "--interp" => a.video_cli.interpolate = Some(true),
            "--no-smooth-frames" => a.video_cli.smooth_frames = Some(false),
            "--crossfade" => a.video_cli.crossfade = Some(true),
            "--logic-hz" => a.video_cli.logic_hz = Some(val()?.parse().map_err(|_| "bad logic hz")?),
            "--original" => {
                // compare: original pacing (no interpolation / easing / extra frames)
                a.video_cli.interpolate = Some(false);
                a.video_cli.smooth_frames = Some(false);
                a.video_cli.crossfade = Some(false);
            }
            "--prepare" => a.prepare = true,
            "--prepare-tools" => a.prepare_tools = true,
            "--hd-prepare" => a.hd_prepare = true,
            "--compare-hd" => a.compare_hd = true,
            "--rife-flow-proto" => a.rife_flow_proto = true,
            "--compare-out" => a.compare_out = Some(val()?.into()),
            "--hd-cache" => a.hd_cache = Some(val()?.into()),
            "--data-root" => a.data_root = Some(val()?.into()),
            "--present" => {
            let v = it.next().ok_or("--present needs original|shader|hd")?;
            a.video_cli.present = Some(v);
        }
        "--shader" => {
            let v = it.next().ok_or("--shader needs anime4k|xbr")?;
            a.video_cli.shader = Some(v);
        }
        "--no-flow" => a.video_cli.flow = Some(false),
        "--no-hd" => a.no_hd = true,
            "--hd" => a.no_hd = false,
            "--regenerate" => { a.prepare = true; a.regenerate = true }
            "--smooth-backend" => a.smooth_backend = Some(val()?),
            "--smooth-quality" => a.smooth_quality = Some(val()?.parse().map_err(|_| "bad quality")?),
            "--workers" => a.workers = Some(val()?.parse().map_err(|_| "bad workers")?),
            "--prepare-only" => a.prepare_only = Some(val()?),
            "--prepare-limit" => a.prepare_limit = Some(val()?.parse().map_err(|_| "bad limit")?),
            "--smooth-cache" => a.smooth_cache = Some(val()?.into()),
            "--frames" => a.frames = val()?.parse().map_err(|_| "bad frame count")?,
            "--render-hz" => a.render_hz = val()?.parse().map_err(|_| "bad render hz")?,
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown option {other}\n\n{HELP}")),
        }
    }
    Ok(a)
}

/// First-run launcher step: folder picker -> validate -> save to config.
fn pick_game_dir(cfg: &mut Config, cfg_path: &std::path::Path, lang: Option<&str>) -> Result<PathBuf, String> {
    let tag = lang.map(str::to_string).or(cfg.language.clone()).or_else(|| std::env::var("NEWPAL_LANG").ok()).unwrap_or_else(|| newpal_assets::i18n::ORIGINAL_LOCALE.into());
    let dir = rfd::FileDialog::new()
        .set_title(newpal_assets::i18n::builtin::text(&tag, "app.pick_folder"))
        .pick_folder()
        .ok_or("no folder selected")?;
    let v = gamedir::validate(&dir);
    print!("{}", v.report());
    if !v.usable() {
        return Err(format!("{} is not a usable New PAL install (required files missing)", dir.display()));
    }
    cfg.game_dir = Some(dir.clone());
    cfg.save_to(cfg_path).map_err(|e| e.to_string())?;
    println!("saved game_dir to {}", cfg_path.display());
    Ok(dir)
}

/// Language layer: pack folders (`--lang-dir`, config, `lang` next to the exe, per-user), mod packs, selected locale.
fn make_i18n(args: &Args, cfg: &Config, mods: &[newpal_assets::vfs::ModInfo]) -> std::sync::Arc<newpal_assets::i18n::I18n> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    dirs.extend(args.lang_dir.clone());
    dirs.extend(cfg.lang_dir.clone());
    dirs.extend(gamedir::exe_dir().map(|d| d.join("lang")));
    dirs.push(gamedir::user_config_dir().join("lang"));
    dirs.dedup();
    let i = newpal_assets::i18n::I18n::new(dirs, mods.iter().map(|m| (m.name.clone(), m.dir.clone())).collect());
    let want = args.lang.clone().or(cfg.language.clone()).or_else(|| std::env::var("NEWPAL_LANG").ok());
    if let Some(tag) = want {
        match i.set_locale(&tag) {
            Ok(l) => eprintln!("language: {} ({})", l.tag, l.name),
            Err(e) => eprintln!("warning: {e}; using the original language"),
        }
        for w in i.warnings() {
            eprintln!("language pack warning: {w}");
        }
    }
    std::sync::Arc::new(i)
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let cfg_path = args.config.clone().unwrap_or_else(Config::default_path);
    let mut cfg = Config::load_from(&cfg_path).map_err(|e| e.to_string())?;

    let game = if args.pick {
        pick_game_dir(&mut cfg, &cfg_path, args.lang.as_deref())?
    } else {
        match gamedir::resolve(args.game_dir.as_deref(), &cfg, &cfg_path) {
            Ok(r) => {
                eprintln!("game data: {} (via {:?})", r.dir.display(), r.source);
                if !r.validation.exact_build() {
                    eprintln!("warning: files differ from the known Steam build; continuing\n{}", r.validation.report());
                }
                r.dir
            }
            Err(e) if args.validate || args.screenshot.is_some() => return Err(e.to_string()),
            Err(e) => {
                eprintln!("{e}\nopening folder picker...");
                pick_game_dir(&mut cfg, &cfg_path, args.lang.as_deref())?
            }
        }
    };
    if args.validate {
        let v = gamedir::validate(&game);
        print!("{}", v.report());
        return if v.usable() { Ok(()) } else { Err("not usable".into()) };
    }

    let mods_dir = args.mods_dir.clone().or(cfg.mods_dir.clone()).or_else(|| gamedir::exe_dir().map(|d| d.join("mods")));
    let mods = mods_dir.as_deref().map(|d| discover_mods(d, &cfg.mods)).unwrap_or_default();
    for m in &mods {
        eprintln!("mod: {} (priority {}) {}", m.name, m.priority, m.dir.display());
    }
    if args.prepare_tools {
        let (exe, model) = newpal_prepare::rife::ensure(&newpal_prepare::default_tools(), &mut |s| eprintln!("[tools] {s}"))?;
        eprintln!("[tools] rife: {} model: {} | Vulkan GPU: {:?}", exe.display(), model.display(), prepare::vulkan_gpu());
        return Ok(());
    }
    if args.compare_hd {
        let hd_cache = args.hd_cache.clone().unwrap_or_else(newpal_prepare::hd::default_dir);
        let compare_out = args.compare_out.clone().unwrap_or_else(|| {
            newpal_assets::gamedir::output_dir().join("screenshots").join("hd-proto")
        });
        let vfs = Vfs::new(game.clone(), mods.clone());
        hd_compare::run(&vfs, &hd_cache, &compare_out, false, args.regenerate)?;
        if args.screenshot.is_none() { return Ok(()); }
    }
    if args.rife_flow_proto {
        let hd_cache = args.hd_cache.clone().unwrap_or_else(newpal_prepare::hd::default_dir);
        let out = args.compare_out.clone().unwrap_or_else(|| {
            newpal_assets::gamedir::output_dir().join("screenshots").join("interp")
        });
        let tools = newpal_prepare::default_tools();
        let vfs = Vfs::new(game.clone(), mods.clone());
        eprintln!("[rife-flow] HD {} -> {}", hd_cache.display(), out.display());
        rife_flow_proto::run(&vfs, &hd_cache, &tools, &out)?;
        return Ok(());
    }
    if args.hd_prepare && args.screenshot.is_none() && !args.compare_hd {
        let opts = newpal_prepare::hd_batch::HdBatchOpts {
            game: game.clone(),
            out: args.hd_cache.clone().unwrap_or_else(newpal_prepare::hd_batch::HdBatchOpts::default_out),
            tools: newpal_prepare::hd_batch::HdBatchOpts::default_tools(),
            skip_maps: false,
            only: None,
            regenerate: args.regenerate,
        };
        eprintln!("[hd] preparing static 4x stills -> {}", opts.out.display());
        newpal_prepare::hd_batch::run(opts)?;
        return Ok(());
    }

    // launcher step 2: asset preparation (local, from the player's own data)
    let mut sopts = prepare::SmoothOpts::resolve(&args, &cfg);
    if let Some(c) = &args.smooth_cache {
        sopts.cache = c.clone();
    }
    let auto = !args.validate && args.screenshot.is_none() && prepare::needs_prepare(&sopts);
    if args.prepare || auto {
        if auto && !args.prepare {
            eprintln!("smooth-animation cache missing or outdated -> preparing (p = pause, q = skip for now)");
        }
        prepare::run(&game, &sopts, args.regenerate, args.prepare_only.clone(), args.prepare_limit)?;
        if args.prepare && args.screenshot.is_none() {
            return Ok(());
        }
    }
    let i18n = make_i18n(&args, &cfg, &mods);
    if args.list_langs {
        for l in i18n.available() {
            println!("{:<10} {}{}", l.tag, l.name, if l.tag == i18n.locale() { "  (selected)" } else { "" });
        }
        return Ok(());
    }
    let mut vfs = Vfs::new(game, mods);
    vfs.set_i18n(i18n);
    for n in vfs.i18n().source_notes() {
        eprintln!("game data: {n}");
    }
    if let Some(out) = &args.ui_sheet {
        return ui_sheet::run(&vfs, &args, out);
    }
    let hd_cache = args.hd_cache.clone().unwrap_or_else(newpal_prepare::hd::default_dir);
    if hd_cache.join("sprites").is_dir() || hd_cache.join("maps").is_dir() || hd_cache.join("manifest.json").is_file() {
        eprintln!("hd cache: {}", hd_cache.display());
        vfs.set_hd_cache(Some(hd_cache));
        vfs.set_hd_enabled(!args.no_hd);
    }
    let pak = prepare::open(&sopts);
    if let Some(p) = &pak {
        eprintln!("smooth cache: {} ({} animations, backend {})", p.path.display(), p.manifest.entries.len(), p.manifest.info.get("backend").cloned().unwrap_or_default());
    }
    vfs.set_smooth_cache(pak, sopts.quality);
    let view_h = args.view_height.or(cfg.view_height).unwrap_or(480) as f32;
    let mut args = args;
    args.video = Video::resolve(&args.video_cli, &cfg.video);
    if args.no_hd { args.video.look.present = world::Present::Shader; }
    if let Some(out) = &args.screenshot {
        return app::screenshot(&vfs, &args, view_h, out);
    }
    app::run(vfs, args, view_h)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
}