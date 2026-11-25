[English](architecture.md) | [简体中文](../zh-CN/architecture.md) · [Back to README](../../README.md)

# Architecture

## Crates

```
newpal-game      the `newpal` binary: winit window, wgpu renderer, game loop, audio, CLI
├── newpal-vm        talk-script VM (ScriptBackend trait, per-opcode handlers)
├── newpal-prepare   offline tools: HD batch, newpal-patch, newpal-i18n, optional RIFE / flow prototypes
└── newpal-assets    original-format readers, mod VFS, game-folder detection, i18n catalog
```

`newpal-assets` is the only crate that knows the original file formats. Everything above it works with decoded
frames, tables and `String`s.

## Reading the original data

- **Index archives** (`.tsw`, `.act`, `.snd`): slot tables with LZO1X-compressed payloads.
- **Sprites** are RLE images. **Maps** (`huge.lmf`, `huge2.lmf`) hold a composed background, object cut-outs for
  depth occlusion and a walk grid.
- **ACT** files are animation tables; **talk scripts** are bytecode, disassembled and run by `newpal-vm`.
- `maps.dat` gives the scene table and NPC placement. `ghero / gitem / gmagic / gmon / glevel` are value tables.
- Everything is little-endian. Readers work on the bytes of your install, in place, through a read-only handle.

## Virtual file system and mods

Lookup order for any asset: **mods > local HD cache > original**. A mod file overrides an original by a stable
logical name (`sprites/all_char/187/f4.png`, `maps/huge/30/bg.png`, ...). See [modding.md](modding.md).

## Game loop and frame pacing

- Logic runs on a **fixed 35 ms tick** (about 28.6 Hz), the original main-loop rate. ACT `DS` delays count ticks.
- Rendering runs at the display rate (vsync, or uncapped, or `--fps-cap`) and interpolates the hero, the camera, the
  ambient clock and dialogue fades between the last two ticks.
- Walk frames are picked by distance walked, so feet do not slide when you are blocked.
- The camera is an eased exponential follow, clamped to the map and widescreen-aware.
- Interpolation only samples between keyframes. It never changes keyframe times, so animation durations match the
  original. See [hd-patch.md](hd-patch.md).

## Rendering

Sprites and map backgrounds are bitmaps drawn by a wgpu sprite renderer with depth occlusion in a **logical 480 px
high** space, so the HD stills (4x) render at the same logical size and 4K or ultrawide only change the camera.
Upscaling has three modes, cycled with F9: original (nearest), a realtime shader (Anime4K-style or xBR), and HD
stills restored from a residual patch.

## Text

All text is UTF-8 and addressed by stable keys through the i18n catalog. The original's Big5, `%`-codes and printf
formats are converted in one import layer (`newpal-assets/src/legacy`), which a policy test (`tests/text_policy.rs`)
keeps isolated. See [i18n.md](i18n.md).

## Offline tools (`newpal-prepare`)

| tool | what it does |
|---|---|
| `newpal-patch build / apply / cmp` | build, apply and compare residual HD patches. See [hd-patch.md](hd-patch.md). |
| `newpal-i18n info / export / check / ui-list / migrate` | inspect your install's text tables and manage language packs. |
| `hd_batch` (example) | local 4x Real-ESRGAN batch that produces the HD cache a patch is built from. |

## The `modern-ui` branch

The `modern-ui` branch is a work in progress and is **not merged into `main`**. It adds the `newpal-ui` crate: a
launcher, title menu, settings, load and pause screens and an original-style dialogue box and system menu, all drawn
as **vectors** with `vello` on the engine's own wgpu device (glyph outlines laid out with `skrifa`, no bitmap text).
It also bundles a subset of the open-licensed LXGW WenKai font (SIL OFL 1.1, with its licence file) so the launcher
can render text before any game files are found. The UI is resolution independent and talks to the engine only
through a small `Host` / `TextSource` / `Cmd` interface. Its own notes are in `docs/modern-architecture.md` on that
branch.
