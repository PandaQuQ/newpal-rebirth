**English** | [简体中文](README.zh-CN.md)

# NewPAL Rebirth (新仙剑奇侠传：重生)

A from-scratch Rust engine for *New Chinese Paladin* (新仙劍奇俠傳, "NewPAL", the PAL98 remake by Softstar / 大宇,
Steam app 695630). It runs on **your own, legally owned copy** of the game and reads that copy in place.

> **This repository contains no game data.** No sprites, maps, audio, text, palettes, fonts or HD frames from the
> original game are included, and none will be accepted. You need to own the game. See [docs/en/legal.md](docs/en/legal.md).

Status: **early preview**. You can walk around the inn and the town, talk to NPCs and run the story scripts, with
HD stills and music. There is no battle system, no inventory and no save system yet. Details in the
[roadmap](docs/en/roadmap.md).

## What is in it

- **A new engine, not a patched exe.** Rust, `wgpu` for rendering, `winit` for windows and input, `rodio` for audio,
  `vello` for the vector UI (on the `modern-ui` branch).
- **Your game folder is read in place.** A launcher, a first-run folder picker or Steam auto-detect finds the
  install. The folder is validated, then read. It is never copied, modified or written to.
- **HD without shipping art.** The 4x HD stills are not distributed. A *residual patch* stores only the difference
  between an HD image and a deterministic bicubic upscale of the original frame, so it is useless without your files.
  Details in [docs/en/hd-patch.md](docs/en/hd-patch.md). A realtime shader upscaler is the fallback.
- **Timing stays original.** Game logic ticks at the original 35 ms. Rendering runs at your display rate and only
  interpolates between ticks, so a 165 Hz monitor does not change how long anything takes.
- **Modern text pipeline.** All engine text is UTF-8 behind stable keys, with TOML language packs and an ICU-style
  message subset. The original's Big5 exists only in one import layer. See [docs/en/i18n.md](docs/en/i18n.md).
- **Mods.** Drop files into `mods/<name>/` to override sprites, maps, audio, text, tables and fonts by logical name.
  See [docs/en/modding.md](docs/en/modding.md).

## Quick start

You need Rust (stable) and your own copy of the game, for example from Steam.

```powershell
git clone https://github.com/PandaQuQ/newpal-rebirth
cd newpal-rebirth
cargo build --release -p newpal-game
target\release\newpal.exe --validate            # find and check the game folder
target\release\newpal.exe                       # start in the inn, opening scene
```

The game folder is picked in this order: `--game-dir`, `NEWPAL_GAME_DIR`, `game_dir` in `newpal.toml`, Steam
auto-detect, then a folder picker. More in [docs/en/getting-started.md](docs/en/getting-started.md).

## Documentation

| English | 简体中文 |
|---|---|
| [Getting started](docs/en/getting-started.md) | [快速上手](docs/zh-CN/getting-started.md) |
| [Architecture](docs/en/architecture.md) | [架构概览](docs/zh-CN/architecture.md) |
| [Localisation (i18n)](docs/en/i18n.md) | [本地化 (i18n)](docs/zh-CN/i18n.md) |
| [Modding](docs/en/modding.md) | [模组](docs/zh-CN/modding.md) |
| [HD patches and frame pacing](docs/en/hd-patch.md) | [高清补丁与帧率](docs/zh-CN/hd-patch.md) |
| [Roadmap and status](docs/en/roadmap.md) | [路线图与现状](docs/zh-CN/roadmap.md) |
| [Legal notes](docs/en/legal.md) | [法律声明](docs/zh-CN/legal.md) |
| [Contributing](CONTRIBUTING.md) | [参与贡献](CONTRIBUTING.zh-CN.md) |

## Workspace

| crate | role |
|---|---|
| `crates/newpal-assets` | Readers for the original formats (index paks, LZO1X, RLE sprites, LMF maps, ACT animation tables, talk scripts, value tables), the mod VFS, game-folder detection and validation, and the i18n layer. |
| `crates/newpal-vm` | The talk-script VM, with a `ScriptBackend` trait and per-opcode handlers. |
| `crates/newpal-prepare` | Offline tools: the HD batch, `newpal-patch` (build / apply / verify residual patches), `newpal-i18n` (export and check language packs). |
| `crates/newpal-game` | The `newpal` binary: winit + wgpu client, map rendering with depth occlusion, walking, dialogue, audio. |

## Legal

Not affiliated with, endorsed by or sponsored by Softstar (大宇資訊) or any rights holder of *Chinese Paladin*.
All trademarks, names and original game content belong to their owners. This project is for people who own the
game and want to play it on a modern engine. Engine code is licensed under **MIT OR Apache-2.0**, at your choice
([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)). The licence covers this repository only, not the game.
