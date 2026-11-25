# Modern front end and UI architecture

Status: prototype on branch `modern-ui`. This file covers the layers, the vector renderer, scenes, input, resolution
and refresh handling, the original-style in-game UI, and what is still missing.

## Layers

```
newpal-game (binary)
├── shell.rs            front end: one window, launcher -> title -> game; GameHost; scripted/headless runs
├── app.rs / world.rs   game loop (35 ms logic tick), map, VM, bitmap sprites (HD stills / shader fallback)
├── render.rs           wgpu sprite renderer (game frame)
newpal-ui (library, no asset/VM deps)
├── gpu.rs              vello renderer on the engine's wgpu device + blit over the game frame
├── text.rs             glyph-outline text (skrifa layout, vello fill/stroke); font chains
├── scene.rs            scene stack, transitions, Cmd queue back to the engine
├── input.rs            abstract actions, keyboard/mouse/gamepad (gilrs), rebinding
├── settings.rs         [display] [audio] [input] + shared keys, merged into newpal.toml
├── strings.rs          `ui:` lookups through the engine's i18n (TextSource), no text of its own
├── host.rs             what the UI asks of the engine (detect/validate folder, HD install, saves, mods, langs)
├── dialogue.rs         original-style dialogue box: typewriter, pages, colours, cues (data-driven theme)
├── theme.rs/themes.rs  vector primitives; built-in TOML themes
├── orig.rs             original materials as vectors: burlap plaques, scroll paper, rods, U-frame, money box
└── screens/            launcher, title, settings, load, mods, pause, confirm, sysmenu (original layout)
newpal-assets
└── i18n/               catalog, builtin/<locale>.toml (all `ui:` texts incl. the front end), markup, messages
```

The UI crate talks to the engine only through `Host`, `TextSource` and `Cmd`, so it builds and tests headless and
does not conflict with work on the VM/assets crates.

## Scene stack

`SceneStack` holds screens (`Screen` trait: `update`, `draw`, `nav`, `name`). Push/replace come with a transition:
`Fade`, `Slide`, or `Ink` (an organic ink blot that grows from a point; screens below are drawn with an inverse
even-odd clip so the game frame shows through the blot). Screens never call the engine directly; they push
`Cmd`s (`UseGameDir`, `EnterTitle`, `NewGame`, `ApplySettings`, `SetLanguage`, `ToTitle`, `Resume`, `Quit`, ...)
which `shell.rs` executes after the update. Phases: `Launcher` (no world) -> `Title` (world alive behind the menu,
camera drift + parallax) -> `Game` (world frozen while a menu is open).

## Input

Actions: Up, Down, Left, Right, Confirm, Back, Menu, PrevTab, NextTab. Keyboard bindings are winit `KeyCode`
names in `[input]` (two per action, rebindable in Settings > Keys). Gamepad (gilrs): D-pad/left stick, A confirm,
B back, Start menu, LB/RB tabs, stick auto-repeat 0.35 s then 0.11 s. Mouse: hover focuses, click confirms,
drag on sliders. The last used device decides the button glyphs in the hint bar.

## Vectorisation

Choice: **vello 0.12** (GPU compute 2D renderer, wgpu 30 - the engine's own version) for all UI:
paths, gradients, blur-free soft shapes, strokes, clips/layers, and **glyph outlines** (`draw_glyphs`), laid out
with skrifa. No glyph atlas and no bitmap text: text is filled at the final pixel size, so it is equally sharp at
720p, 4K and any DPI/UI scale, and can be scaled/animated without re-rasterising textures.

Alternatives considered: SDF/MSDF text (needs per-font atlas generation, CJK atlases are large, corners soften at
large sizes); glyphon/cosmic-text (atlas based, good for big text volumes, less flexible for strokes/effects);
tessellation (lyon) - more code for the same result.

Cost, measured on the RTX 4070 Ti build:
- pipelines: ~630-670 ms once at start-up (logged `[ui] vello pipelines ready in N ms`); can move to a
  background thread later.
- per frame: 0.3-1.2 ms UI GPU time at 1280x720 and 3840x2160 (`UiGpu.last_ms`, shown in the window title).
- binary: release `newpal.exe` 22.5 MB -> 30.3 MB (+7.8 MB: vello shaders/compute, gilrs, two bundled font
  subsets of ~249 KB each).

Game graphics stay bitmaps (HD stills from the residual patch, shader upscale fallback); vectors are for UI,
text and effects.

## Fonts

Bundled: LXGW WenKai subset (SIL OFL 1.1), renamed "NewPal Kai" (OFL reserved-name rule), 527 characters, two
weights. Font chains: `body`/`title` (bundled, then host faces, then system); `dialog` (the player's own
`Resource/CT.ttf` first, then language-pack / mod fonts in the i18n layer's order, bundled, system). Original fonts
are read at runtime only.

## Resolution independence

UI layout unit `u = min(w/1920, h/1080) * ui_scale`; every size is a multiple of `u`. The original-style in-game UI
uses the original reference space (640x480): scale = window height / 480, centred horizontally, like the game view
(`view-height 480`, width follows the aspect). `--ui-scale` / Settings > Display > UI scale (0.75-1.5).

## High refresh rate and window modes

Rendering is decoupled from the 35 ms logic tick: the world interpolates between ticks; UI animations and the
dialogue typewriter run on real frame time and never change logic durations. Settings > Display: window mode
(windowed / borderless / exclusive fullscreen), resolution (from the monitor's modes), refresh target
(0 = display, 60, 120, 144, 165, 240, 300), vsync, frame cap. Applied live by `ShellApp::apply_window`.

## Original-style in-game UI (dialogue first)

Panda's rule: no translating words baked into PNGs. In-game UI is redrawn: frames as vectors, all text from fonts
through i18n keys (`talk:`, `menu:`, `battle:`, `shop:`, `ui:`).

- `DialogueTheme` (TOML, reference pixels): box rect, padding, corner, font size, line height, character gap
  (original fixed cells), lines per page, name line, palette for `<color=N>`, outline and shadow, fill gradient,
  borders, corner ornaments, typewriter ms per `<speed=N/>` level, `<glyph=N/>` scales, open animation, "more"
  marker. Built-in: `crates/newpal-ui/themes/original-dialogue.toml`; a mod can ship `ui/dialogue.toml`.
- Markup handled by the renderer: `<speed=N/>`, `<instant/>`, `<page/>`, `<color=N>`, `<br/>` (lines),
  `<speaker>` (name line + `ui:dialogue.name_sep`), `<glyph=N/>` (size scale; meaning in the original still
  unconfirmed); unknown original codes are logged.
- Confirm: first press completes the page, then next page, then the script continues.
- Wrapping: CJK breaks anywhere with kinsoku for closing punctuation; Latin wraps at spaces.

### What the original looks like (read-only analysis of the player's `Resource/*.png` and the exe)

- Materials: burlap / parchment plaques with charred edges; cream paper (240,232,215); wooden rods
  (130,100,75 / 120,88,63, highlight 182,143,107) with bamboo ties; a U-shaped rod frame (156x420) holds the menu
  plaques; sepia ink-painting background; gold-ingot money box (188x44).
- Brush labels: dark-brown ink with a pale rim; hover = vermilion with a white glow; disabled = greyscale. Most
  labels are baked into the PNGs and are not in StringDB, so they are re-typed as `ui:sysmenu.*` keys.
- Dialogue text: half-width cell 11 px in the exe, so CJK cells and lines are 22 px; frame padding 4 (5 with a
  caption). Four frame styles are sprites 9005/9007/9009/9011 (caption 9015) inside a pak; the box is placed next to
  the speaking actor (actor x+0x40, y+0x10) and sized to the text.
- Mapping to vectors: `orig.rs` draws these materials procedurally (no pixels copied). Dialogue `style =
  "parchment"` uses the scroll paper; the system menu (`screens/sysmenu.rs`, opened by Esc / Start in game) uses
  the U-frame, seven plaques, the money box and the small music / sound / quit panel; "More settings" opens the
  modern settings screen.
- Not reproduced yet / guessed: speaker-anchored bubble placement (we keep a bottom box), the exact frame sprites,
  palette and per-level speeds, the meaning of `<glyph=N/>`, portraits (9071-9089) beside the text, money (the VM
  does not expose it yet).

## Settings file

`newpal.toml` is merged as a TOML table so unknown keys survive. Shared keys: `game_dir`, `language`,
`window_size`, `[video]` (vsync, fps_cap, present). New: `[display]` (mode, refresh, ui_scale), `[audio]`
(master, music, sfx), `[input]` (bindings).

## Scripted runs (CI / screenshots)

`--ui-script "wait:2;key:down;key:confirm;shot:menu;quit"` with `--ui-shot-dir <dir>` renders headless and writes
PNGs; steps: `wait`, `key:<action>`, `click:x,y`, `move:x,y`, `pad`, `kbd`, `shot:<name>`, `rec:<s>`, `size:WxH`,
`quit`. `--first-run` forces the launcher, `--classic` keeps the old debug window.

## Open items

- Save system (Continue / Load are wired but empty).
- System menu: items / equip / magic / status / save screens; battle and shop screens; speaker-anchored dialogue.
- `Config::save_to` (used by `--pick`) re-serialises the struct and drops `[display]`/`[audio]`/`[input]`.
- Audio volumes are applied at playback start only.
- Resolution change has no "keep these settings?" countdown yet.
