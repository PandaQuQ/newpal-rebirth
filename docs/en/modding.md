[English](modding.md) | [简体中文](../zh-CN/modding.md) · [Back to README](../../README.md)

# Modding

Mods live in `mods/<name>/` next to the exe (or the folder given by `--mods-dir` / `mods_dir`). A `mod.toml` is
optional:

```toml
name = "my-mod"
priority = 10        # higher wins
enabled = true
```

`mods = [...]` in `newpal.toml` selects mods explicitly (first = highest priority). If it is empty, every mod found is
used, ordered by `priority`.

A mod file replaces an original asset by a stable **logical name**. Files you do not provide keep coming from the
original game. Mods are layered above the HD cache and the original data.

| logical name | replaces |
|---|---|
| `sprites/<tsw>/<id>/f<k>.png` | frame k of tsw entry id (1-based, as used by ACT `FR`), e.g. `sprites/all_char/187/f4.png` |
| `maps/<lmf>/<id>/bg.png` | a map background, e.g. `maps/huge/30/bg.png` |
| `maps/<lmf>/<id>/obj_<oid>.png` | a map object (depth cut-out) |
| `audio/snd/<slot>.wav` | a sound effect (`all.snd` slot, 0-based; script id = slot + 1) |
| `music/<stem>.ogg` / `.mp3` / `.wav` / `.flac` | background music, e.g. `music/m027.ogg` replaces the original `m027.mp3` |
| `act/<act>/<slot>/<action>/anim.toml` + `f000.png ...` | in-between frames for an action |
| `text/talk/<n>.json` | dialogue text per instruction position: `{ "0024": "new line" }` |
| `lang/<locale>/*.toml` | language packs, see [i18n.md](i18n.md) |
| `scripts/talk/<n>.bin` | the full bytecode of script n |
| `tables/<name>.json` / `.toml` | patches for `ghero / gitem / gmagic / gmon / glevel` records (match by `id`; new ids need `kind`) |
| `fonts/dialog.ttf` | the dialogue font |
| `ui/dialogue.toml` | dialogue box theme (`modern-ui` branch) |
| `raw/<path>` | any original file, byte for byte |

**HD and scaled art:** `f4@2x.png`, `bg@4x.png` and so on render at logical size = native size / N. A plain `f4.png`
larger than the original is shown at the original's logical size. The renderer works in logical units (480 px = the
original view height), so any resolution works.

**Scripting:** the script VM exposes `Vm::register(op, handler)` and the `ScriptBackend` trait. A Lua or Rhai
backend can be added later without touching the game loop.

## Rules for mods you publish

- Do not include original game files or anything extracted or converted from them. That includes re-encoded audio,
  upscaled frames, and text dumps.
- Translations, new art, new music and scripts you wrote yourself are welcome, under a licence you choose.
- Language packs exported with `newpal-i18n export --with-source` contain the original text as comments. Keep those
  local. The tool writes a `.gitignore` next to them for that reason.
