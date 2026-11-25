[English](legal.md) | [简体中文](../zh-CN/legal.md) · [Back to README](../../README.md)

# Legal notes

**This is not legal advice.**

## What this project is

An independent, unofficial engine. It is **not affiliated with, endorsed by or sponsored by** Softstar Entertainment
(大宇資訊) or any other rights holder of *Chinese Paladin* / 仙劍奇俠傳 / 新仙劍奇俠傳. Those names, the game's
characters, story, art, music, text and all related trademarks belong to their respective owners. They are mentioned
here only to say which game this engine runs.

## You must own the game

The engine does nothing useful without the original game files, and it is meant for people who own a legal copy
(for example the Steam release). It does not help you obtain the game.

## What the repository contains

- Engine source code, tools, tests and documentation, written for this project.
- Short UI strings in English and Chinese written for this project (not taken from the game).
- On the `modern-ui` branch: a subset of the LXGW WenKai font (SIL Open Font License 1.1, licence file included,
  renamed as the licence requires).

## What it never contains

No sprites, maps, audio, scripts, dialogue or string tables, palettes, fonts or screenshots from the original game, and
no HD upscales of them. `.gitignore` blocks the original formats, media files and local working folders. Please keep
it that way: do not open pull requests or issues that attach original assets.

## How data is handled

The game folder is validated and **read in place, read-only**. The engine never copies it, never modifies it and never
touches the game process. HD stills, caches and screenshots are generated on the player's machine from the player's
own files and stay there.

## Residual HD patches

A residual patch stores the difference between an AI-upscaled image and a deterministic upscale of the original frame.
It contains no original pixels and no upscaled pixels, and cannot be used without the matching original frames. That
is the design intent. Whether *distributing* such patches is acceptable is a copyright question this project has not
had answered by a lawyer. No patch is published here. If you plan to distribute one, get your own advice first.

## Licence

The engine source code and documentation are dual-licensed under **MIT OR Apache-2.0**, at your option:
[LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE). The licence applies to this repository's
own content only. It grants no rights to the original game. Third-party components keep their own licences (crates
from crates.io, the optional `realesrgan-ncnn-vulkan` and `rife-ncnn-vulkan` tools, the LXGW WenKai font subset).

Rights holders who believe something in this repository should not be here can open an issue or contact the maintainer
and it will be removed promptly.
