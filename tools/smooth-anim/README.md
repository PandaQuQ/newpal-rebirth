# Smooth animation: in-between frames (hand-made mods)

> The primary path is now the launcher's asset preparation (`newpal --prepare`; see the main README). It writes `%LOCALAPPDATA%\NewPAL-Reborn\cache\smooth.pak` locally. This folder is only for authoring **overrides**: mods win over the cache. The old `mods-test\smooth-hero-xfade` sample is retired.

The original sprites are low-frame-rate. For example, the hero walk cycle has 8 frames × `DS 3` × 35 ms, which is about 9.5 animation fps. The engine already renders positions and camera at the display rate. This pipeline adds **extra sprite frames** as a mod. The original data is never modified.

## Mod format
```
mods/<name>/act/<act>/<slot>/<action>/anim.toml
mods/<name>/act/<act>/<slot>/<action>/f000.png, f001.png, ...
```
- `<act>` is the act file stem (`all_char`).
- `<slot>` is the 0-based entry (0 = the hero).
- `<action>` is the ACT action index (0..7 stand, 8..15 walk).
- Frames are drawn bottom-centre, like the original frames.

```toml
count = 32                # f000.png .. f031.png   (or frames = ["a.png", ...])
fps = 60                  # optional; default = spread over the original loop length
durations_ms = [26.25, ...]   # optional explicit timing (wins over fps)
scale = 2                 # optional HD density (png size / scale = logical size)
```

## Generating
`make-smooth.ps1` uses `anim_tool` (crates/newpal-assets/examples). It writes to `..\extracted\smooth` and `..\mods-test\...`, both outside the repo.

| mode | cost | quality |
|---|---|---|
| `-Mode crossfade -Factor 4` | CPU, a few seconds | blends the frames (ghosting); a baseline / pipeline test |
| `-Mode rife -Factor 4` | **GPU** (Vulkan); run when not gaming | real motion in-betweens |

RIFE setup: download a `rife-ncnn-vulkan` release (Windows zip; it ships `rife-v4.6` and other model folders) and pass `-Rife <path to exe>`.
- RIFE has no alpha channel. The tool therefore splits each frame into premultiplied RGB-on-black and an alpha mask, interpolates both, and merges them again (`prep-rife` / `merge-rife`).
- The first frame is repeated at the end so that the loop seam also gets in-betweens.
- Pixel-art outlines can come out soft. Clean them up by hand or keep factor 2.

## Compare in game
| key | toggles |
|---|---|
| F4 | interpolation (movement / camera / UI) |
| F5 | in-between frames |
| F6 | cross-fade fallback |
| F7 | fps cap (vsync / 30 / 60 / 144 / 240) |
| F8 | vsync |

`--original` starts with everything off. The window title shows fps and the toggles.