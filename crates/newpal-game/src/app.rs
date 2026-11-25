//! winit window + event loop, and the headless screenshot path.
use crate::audio::Audio;
use crate::render::Renderer;
use crate::timing::{FixedStep, Pacer, CAPS};
use crate::world::World;
use crate::Args;
use newpal_assets::vfs::Vfs;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const CLEAR: wgpu::Color = wgpu::Color { r: 0.02, g: 0.02, b: 0.03, a: 1.0 };

pub(crate) fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle())
}

pub(crate) async fn device(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), String> {
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: surface,
            force_fallback_adapter: false,
            ..Default::default()
        })
        .await
        .map_err(|e| format!("no GPU adapter: {e}"))?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor { label: Some("newpal"), ..Default::default() })
        .await
        .map_err(|e| format!("request_device: {e}"))?;
    Ok((adapter, device, queue))
}

/// Scene music + script audio requests of this frame.
pub(crate) fn sync_audio(vfs: &Vfs, a: &mut Audio, w: &mut World) {
    if std::mem::take(&mut w.scene_changed) {
        a.enter_scene(vfs, w.city);
    }
    a.run(vfs, std::mem::take(&mut w.audio_cmds));
    a.tick();
}

fn setup_world(vfs: &Vfs, r: &mut Renderer, args: &Args, view_h: f32) -> Result<World, String> {
    let mut w = World::load(vfs, r, &args.lmf, args.map_id, view_h)?;
    w.debug_nav = args.debug_nav;
    w.look = args.video.look;
    let (x, y) = args.pos.unwrap_or((w.hero.x, w.hero.y));
    w.place_hero(x, y);
    if args.script != 0 {
        if let Err(e) = w.start_script(vfs, args.script) {
            eprintln!("script {}: {e}", args.script);
        }
    }
    Ok(w)
}

/// Print / write the keys that fell back to the original text.
fn report_i18n(vfs: &Vfs, args: &Args) {
    let i = vfs.i18n();
    let missing = i.missing();
    eprintln!("i18n: locale {} | {} key(s) fell back to the original text", i.locale(), missing.len());
    if let Some(p) = &args.lang_missing {
        let _ = std::fs::write(p, missing.join("\n"));
    }
}

/// Render one frame offscreen and save it as PNG (no window).
pub fn screenshot(vfs: &Vfs, args: &Args, view_h: f32, out: &Path) -> Result<(), String> {
    let inst = instance();
    let (_a, device, queue) = pollster::block_on(device(&inst, None))?;
    let mut r = Renderer::new(device, queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    let mut w = setup_world(vfs, &mut r, args, view_h)?;
    for _ in 0..args.advance {
        w.advance_dialogue();
    }
    if let Some(tag) = &args.lang_then {
        match w.set_locale(vfs, tag) {
            Ok(t) => eprintln!("language switched at runtime -> {t}"),
            Err(e) => eprintln!("language switch failed: {e}"),
        }
    }
    if let Some((x, y)) = args.walk_to {
        // headless click-to-walk: dismiss dialogue, path, simulate
        while w.dialogue.is_some() { w.advance_dialogue(); }
        w.walk_to(x, y);
    }
    let steps = (args.sim / 0.02).round() as i32;
    eprintln!("look {:?}, logic tick {:.1} ms, anims with in-between frames: {}", w.look, FixedStep::from_hz(args.video.logic_hz).step * 1000.0, w.smooth_anims());
    let mut audio = Audio::silent();
    sync_audio(vfs, &mut audio, &mut w);
    // same pacing as the window: fixed logic steps, rendering at render_hz with interpolation
    let mut fixed = FixedStep::from_hz(args.video.logic_hz);
    let frame_dt = 1.0 / args.render_hz.max(1.0);
    let mut run = |w: &mut World, r: &mut Renderer, audio: &mut Audio| {
        for _ in 0..fixed.advance(frame_dt) {
            w.step(vfs, r, fixed.step);
            sync_audio(vfs, audio, w);
        }
        w.alpha = fixed.alpha();
    };
    let _ = steps;
    for _ in 0..(args.sim * args.render_hz).round() as i32 {
        run(&mut w, &mut r, &mut audio);
    }
    if args.interact {
        w.interact();
        w.tick(vfs, &mut r, 0.0);
        sync_audio(vfs, &mut audio, &mut w);
    }
    if args.frames > 0 {
        let (sw, sh) = args.size;
        let stem = out.with_extension("");
        let mut last = None;
        for i in 0..args.frames {
            run(&mut w, &mut r, &mut audio);
            let (hx, hy, _, _) = w.hero_render();
            let cam = w.camera(sw, sh).origin;
            let d = last.map(|(lx, ly): (f32, f32)| (hx - lx).hypot(hy - ly)).unwrap_or(0.0);
            eprintln!("frame {i:03}: alpha {:.2} hero ({hx:.2},{hy:.2}) moved {d:.2}px cam ({:.2},{:.2})", w.alpha, cam[0], cam[1]);
            last = Some((hx, hy));
            w.draw(&mut r, sw, sh);
            let img = r.render_to_image(sw, sh, CLEAR);
            img.save_png(&std::path::PathBuf::from(format!("{}_{i:03}.png", stem.display()))).map_err(|e| e.to_string())?;
        }
        eprintln!("saved {} frames -> {}_NNN.png", args.frames, stem.display());
        report_i18n(vfs, args);
        return Ok(());
    }
    w.settle_ui();
    for l in &audio.log {
        eprintln!("audio: {l}");
    }
    w.update(0.0);
    eprintln!("hero at ({:.0},{:.0}) path left {} status: {}", w.hero.x, w.hero.y, w.hero.path.len(), w.status);
    let (sw, sh) = args.size;
    w.draw(&mut r, sw, sh);
    let img = r.render_to_image(sw, sh, CLEAR);
    img.save_png(out).map_err(|e| e.to_string())?;
    eprintln!("screenshot {}x{} -> {}", sw, sh, out.display());
    report_i18n(vfs, args);
    Ok(())
}

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    r: Renderer,
    world: World,
}

struct App {
    vfs: Vfs,
    args: Args,
    view_h: f32,
    gfx: Option<Gfx>,
    keys: [bool; 4], // up down left right
    cursor: (f32, f32),
    last: Instant,
    error: Option<String>,
    audio: Audio,
    fixed: FixedStep,
    pacer: Pacer,
    vsync: bool,
}

impl App {
    fn init(&mut self, el: &ActiveEventLoop) -> Result<(), String> {
        let (w, h) = self.args.size;
        let attrs = Window::default_attributes()
            .with_title(self.vfs.i18n().ui("app.title"))
            .with_inner_size(winit::dpi::PhysicalSize::new(w, h));
        let window = Arc::new(el.create_window(attrs).map_err(|e| e.to_string())?);
        let inst = instance();
        let surface = inst.create_surface(window.clone()).map_err(|e| e.to_string())?;
        let (adapter, device, queue) = pollster::block_on(device(&inst, Some(&surface)))?;
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).ok_or("surface unsupported")?;
        config.format = format;
        config.present_mode = if self.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        surface.configure(&device, &config);
        let mut r = Renderer::new(device, queue, format);
        let world = setup_world(&self.vfs, &mut r, &self.args, self.view_h)?;
        self.gfx = Some(Gfx { window, surface, config, r, world });
        Ok(())
    }

    fn redraw(&mut self) {
        let Some(g) = self.gfx.as_mut() else { return };
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.25);
        self.last = now;
        self.pacer.frame(dt);
        let k = self.keys;
        g.world.input = ((k[3] as i32 - k[2] as i32) as f32, (k[1] as i32 - k[0] as i32) as f32);
        // fixed logic steps (original 35 ms tick), rendering at the display rate with interpolation
        for _ in 0..self.fixed.advance(dt) {
            g.world.step(&self.vfs, &mut g.r, self.fixed.step);
            sync_audio(&self.vfs, &mut self.audio, &mut g.world);
        }
        g.world.alpha = self.fixed.alpha();
        let frame = match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                g.surface.configure(&g.r.device, &g.config);
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        g.world.draw(&mut g.r, g.config.width, g.config.height);
        g.r.render(&view, CLEAR);
        g.r.queue.present(frame);
        let h = &g.world.hero;
        g.window.set_title(&format!(
            "{} \u{2014} {}:{} ({:.0},{:.0}) {:.0} fps [cap {} vsync {} interp {} frames {} xfade {} {} {} {}] {}",
            self.vfs.i18n().ui("app.title"), g.world.lmf, g.world.map_id, h.x, h.y, self.pacer.fps,
            if self.pacer.cap == 0 { "-".to_string() } else { self.pacer.cap.to_string() },
            on(self.vsync), on(g.world.look.interpolate), on(g.world.look.smooth_frames), on(g.world.look.crossfade), g.world.look.present.as_str(), g.world.look.shader.as_str(), if g.world.look.flow { "flow" } else { "cut" }, g.world.status
        ));
    }
}

fn on(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_none() {
            if let Err(e) = self.init(el) {
                self.error = Some(e);
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                if let Some(g) = self.gfx.as_mut() {
                    g.config.width = s.width.max(1);
                    g.config.height = s.height.max(1);
                    g.surface.configure(&g.r.device, &g.config);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::CursorMoved { position, .. } => self.cursor = (position.x as f32, position.y as f32),
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let Some(g) = self.gfx.as_mut() {
                    if g.world.dialogue.is_some() {
                        g.world.advance_dialogue();
                    } else {
                        let t = g.world.screen_to_world(g.config.width, g.config.height, self.cursor.0, self.cursor.1);
                        g.world.walk_to(t.0, t.1);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let down = event.state == ElementState::Pressed;
                let PhysicalKey::Code(code) = event.physical_key else { return };
                let idx = match code {
                    KeyCode::KeyW | KeyCode::ArrowUp => Some(0),
                    KeyCode::KeyS | KeyCode::ArrowDown => Some(1),
                    KeyCode::KeyA | KeyCode::ArrowLeft => Some(2),
                    KeyCode::KeyD | KeyCode::ArrowRight => Some(3),
                    _ => None,
                };
                if let Some(i) = idx {
                    self.keys[i] = down;
                }
                if !down || event.repeat {
                    return;
                }
                match code {
                    KeyCode::Escape => el.exit(),
                    KeyCode::Enter | KeyCode::Space | KeyCode::NumpadEnter => {
                        if let Some(g) = self.gfx.as_mut() {
                            if g.world.dialogue.is_none() {
                                g.world.interact();
                            }
                            g.world.advance_dialogue();
                        }
                    }
                    KeyCode::F4 | KeyCode::F5 | KeyCode::F6 => {
                        if let Some(g) = self.gfx.as_mut() {
                            let l = &mut g.world.look;
                            match code {
                                KeyCode::F4 => l.interpolate = !l.interpolate,
                                KeyCode::F5 => l.smooth_frames = !l.smooth_frames,
                                _ => l.crossfade = !l.crossfade,
                            }
                            g.world.snap_visuals();
                        }
                    }
                    KeyCode::F7 => {
                        let i = CAPS.iter().position(|&c| c == self.pacer.cap).map(|i| (i + 1) % CAPS.len()).unwrap_or(0);
                        self.pacer.cap = CAPS[i];
                    }
                    KeyCode::F8 => {
                        self.vsync = !self.vsync;
                        if let Some(g) = self.gfx.as_mut() {
                            g.config.present_mode = if self.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
                            g.surface.configure(&g.r.device, &g.config);
                        }
                    }
                    KeyCode::F3 => {
                        if let Some(g) = self.gfx.as_mut() {
                            g.world.debug_nav = !g.world.debug_nav;
                        }
                    }
                    KeyCode::F9 => {
                        if let Some(g) = self.gfx.as_mut() {
                            g.world.look.present = g.world.look.present.cycle();
                            g.world.snap_visuals();
                        }
                    }
                    KeyCode::F10 => {
                        if let Some(g) = self.gfx.as_mut() {
                            g.world.look.shader = g.world.look.shader.cycle();
                            g.world.snap_visuals();
                        }
                    }
                    KeyCode::F11 => {
                        if let Some(g) = self.gfx.as_mut() {
                            g.world.look.flow = !g.world.look.flow;
                            g.world.snap_visuals();
                        }
                    }
                    KeyCode::F12 => {
                        if let Some(g) = self.gfx.as_mut() {
                            let next = self.vfs.i18n().next_locale();
                            g.world.status = match g.world.set_locale(&self.vfs, &next) {
                                Ok(t) => format!("language: {t}"),
                                Err(e) => e,
                            };
                        }
                    }
                    KeyCode::KeyT => {
                        if let Some(g) = self.gfx.as_mut() {
                            if let Err(e) = g.world.start_script(&self.vfs, self.args.script.max(1)) {
                                g.world.status = e;
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if let Some(g) = &self.gfx {
            g.window.request_redraw();
        }
        // vsync paces in present(); a cap waits until the next slot; otherwise render as fast as possible
        match self.pacer.deadline(Instant::now()) {
            Some(t) => el.set_control_flow(ControlFlow::WaitUntil(t)),
            None => el.set_control_flow(ControlFlow::Poll),
        }
    }
}

pub fn run(vfs: Vfs, args: Args, view_h: f32) -> Result<(), String> {
    let el = EventLoop::new().map_err(|e| e.to_string())?;
    let v = args.video;
    let mut app = App { vfs, args, view_h, gfx: None, keys: [false; 4], cursor: (0.0, 0.0), last: Instant::now(), error: None, audio: Audio::silent(), fixed: FixedStep::from_hz(v.logic_hz), pacer: Pacer::new(v.fps_cap), vsync: v.vsync };
    if !app.args.mute {
        app.audio = Audio::open();
    }
    el.run_app(&mut app).map_err(|e| e.to_string())?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}