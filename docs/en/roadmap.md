[English](roadmap.md) | [简体中文](../zh-CN/roadmap.md) · [Back to README](../../README.md)

# Roadmap and status

Early preview. Things are rough, and several details are still guesses. This page tries to be honest about which.

## Works today (`main`)

- Reading the original data in place: index paks, LZO, RLE sprites, LMF maps, ACT tables, talk scripts, value tables.
- Game-folder detection (flag, env var, config, Steam auto-detect, folder picker) and validation.
- Maps with depth occlusion, 8-direction walking, A* click-to-walk, region triggers, scene changes.
- NPCs from `maps.dat` with their talk scripts, a dialogue box, story flags, variables and jumps in the VM.
- Scene music and script sound effects through `rodio`.
- 35 ms logic tick with display-rate rendering and interpolation; HD stills or shader upscale.
- Mod VFS, UTF-8 i18n with language packs and runtime switching, `newpal-i18n` tooling.
- Residual patch build and apply tooling (`newpal-patch`).

## In progress (`modern-ui` branch, not merged)

- Vector UI with `vello`: launcher, title, settings, load, pause, original-style dialogue box and system menu.

## Not done yet

- **No save system.** Continue / Load exist in the UI branch but have nothing to load.
- **No battle system, no inventory** (scripts currently see an empty bag), no shops.
- System menu sub-screens: items, equipment, magic, status and save only show a "not ready" notice.
- Battle and shop screens.
- Dialogue placement: the original puts the box next to the speaking character and sizes it to the text. Ours is
  fixed at the bottom of the screen. Dialogue frame sprites, portraits, the exact colour palette and the
  typewriter speed levels are approximations. The meaning of the `<glyph>` code is unconfirmed.
- NPC movement and hidden roles (op 0x11), the "previous scene" target, and many cutscene opcodes.
- Menu background is an ink-wash stand-in drawn by this project, not the original painting.
- Volume changes only apply when playback starts. There is no "keep these settings?" countdown after a resolution
  change. `Config::save_to` drops the `[display]`, `[audio]` and `[input]` tables (known bug on `modern-ui`).
- Full timing and full PSNR verification of the complete HD patch.
- Only Windows (GNU toolchain) has been tested.

## Guesses worth knowing about

Steps onto scripts >= 6000 fire on contact while others need the action key; actors >= 699 are invisible; music mode
2 means play once; the hero is anchored bottom-centre. These match the original in testing, but they came from reading
behaviour, not from a spec.

## Later

- A complete patch release process, once the legal question in [legal.md](legal.md) is resolved.
- More languages and CLDR plural rules, complete traditional/simplified conversion by default.
- Gamepad and rebinding on `main`, a proper launcher, an optional Lua / Rhai script backend.
