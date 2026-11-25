[English](getting-started.md) | [简体中文](../zh-CN/getting-started.md) · [Back to README](../../README.md)

# Getting started

## What you need

- **Your own copy of New Chinese Paladin** (新仙劍奇俠傳). The engine is developed against the Steam release
  (app 695630, build 2.17.102.0). Other builds may work and only produce a validation warning.
- **Rust**, stable channel. Windows with the `x86_64-pc-windows-gnu` toolchain is what the project is developed on
  (no MSVC and no admin rights needed). Other platforms are untested.
- A GPU with Vulkan, DX12 or Metal support (anything `wgpu` can use).

## Build

```powershell
git clone https://github.com/PandaQuQ/newpal-rebirth
cd newpal-rebirth
cargo build --release -p newpal-game     # produces target\release\newpal.exe
cargo test                               # unit tests; golden tests skip themselves if no game install is found
```

## How the engine finds your game

The game directory is resolved in this order. The first candidate that passes validation wins.

1. `--game-dir <path>`
2. the `NEWPAL_GAME_DIR` environment variable
3. `game_dir` in `newpal.toml`. The file sits next to the exe in portable mode; otherwise it is
   `%APPDATA%\NewPAL-Reborn\newpal.toml`.
4. **Steam auto-detect.** The engine finds the Steam root (registry or default paths), reads
   `steamapps\libraryfolders.vdf` and `appmanifest_695630.acf`, and uses `steamapps\common\<installdir>`.

If nothing is found, a **folder picker** opens. The chosen folder is validated and remembered in `newpal.toml`.
`newpal --pick` opens the picker again at any time.

**Validation** checks that every required file is present (`*.dat`, `all_char.tsw/.act`, `huge*.lmf` and so on),
compares file sizes with the known Steam build, and compares the SHA-256 of each key file under 2 MiB. A different
build only gives a warning. The directory is opened **read-only**. Nothing is copied out of it and nothing is
written into it, and the game process is never touched.

## Config

`newpal.toml` (it is in `.gitignore`, because it contains your own paths):

```toml
game_dir = 'D:\Games\NewPAL'      # path to your own install
mods = ["hd-sprites"]             # enabled mods, first = highest priority (empty = all, by priority)
view_height = 480                 # logical view height; width follows the window aspect
window_size = [1280, 720]
language = "en"                   # BCP-47 tag, see docs/en/i18n.md

[video]
vsync = true
fps_cap = 0                       # 0 = none
interpolate = true
smooth_frames = false
crossfade = false

[smooth]                          # optional local RIFE in-between frames, off by default
backend = "off"                   # off | auto | gpu | cpu
quality = 4
workers = 2
```

Work files (HD cache, downloaded tools, output) go under a *data root*. It is resolved as: `--data-root` /
`[paths] data_root` in `newpal.toml`, then `NEWPAL_DATA_ROOT`, then a project folder found next to the exe or the
current directory (one that contains `LAYOUT.md`, or both `cache/` and `tools/`), then `%LOCALAPPDATA%\NewPAL-Reborn`.
Nothing under the data root is ever committed.

## Running

```powershell
newpal --validate                                   # check the game folder
newpal                                              # map huge:2 (the inn), opening dialogue script 104
newpal --map huge:30 --script 0 --size 1920x1080
newpal --map huge:2 --pos 470,430 --screenshot shot.png        # headless frame -> PNG
newpal --script 0 --walk-to 168,568 --sim 15 --screenshot s.png   # walk out of the inn (scene change)
newpal --mute --debug-nav                           # no audio, navigation overlay on
newpal --lang en                                    # UI language, see the i18n doc
newpal --help                                       # the full list of options
```

Screenshots written by `--screenshot` contain original art. Keep them local and do not publish them.

### Controls

| input | action |
|---|---|
| WASD / arrow keys | walk in 8 directions |
| left click | walk to the clicked point (A* over the map walk grid, around NPCs) |
| Enter / Space / click | advance dialogue; with no dialogue open, talk to the NPC in reach or run the region under the hero |
| step on an exit region | runs its script, for example leaving the inn loads the town |
| F3 | navigation overlay (red blocked, blue regions, green path) |
| F4 | interpolation |
| F5 / F6 | in-between frames / cross-fade |
| F7 / F8 | fps cap / vsync |
| F9 | cycle original / shader upscale / HD stills |
| F10 | cycle the shader: Anime4K-style / xBR |
| F11 | timing-preserving flow warp on/off |
| F12 | switch language |
| T | restart the script |
| Esc | quit |

`--original` starts with every enhancement off, for A/B comparison. The title bar shows fps and the toggles.
