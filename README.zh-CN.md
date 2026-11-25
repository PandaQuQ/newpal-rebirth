[English](README.md) | **简体中文**

# 新仙剑奇侠传：重生 (NewPAL Rebirth)

一个用 Rust 从零写的《新仙剑奇侠传》（新仙劍奇俠傳，简称 NewPAL，大宇 / Softstar 的 PAL98 重制版，Steam 应用号
695630）引擎。它直接读取**你自己合法购买的游戏副本**，原地读取，不复制。

> **本仓库不含任何游戏数据。** 原版的精灵图、地图、音频、文本、调色板、字体、高清帧一概没有，也不会接收。
> 你必须拥有正版游戏。详见 [docs/zh-CN/legal.md](docs/zh-CN/legal.md)。

现状：**早期预览**。能在客栈和小镇里走动、和 NPC 对话、跑剧情脚本，有高清静帧和音乐。战斗、背包、存档还没有。
详见[路线图](docs/zh-CN/roadmap.md)。

## 有什么

- **新引擎，不是给原版 exe 打补丁。** Rust 编写，`wgpu` 渲染，`winit` 管窗口和输入，`rodio` 播放声音，
  矢量界面用 `vello`（在 `modern-ui` 分支）。
- **原地读取你的游戏目录。** 启动器、首次运行的选择目录窗口，或 Steam 自动检测来找到安装位置。目录会先校验再读取，
  不会被复制、修改或写入。
- **不发布美术的高清化。** 4 倍高清静帧不会分发。*残差补丁*只保存“高清图”与“原帧的确定性双三次放大”之间的差，
  没有你的原版文件就毫无用处。详见 [docs/zh-CN/hd-patch.md](docs/zh-CN/hd-patch.md)。实时着色器放大作为保底。
- **时长保持原样。** 游戏逻辑按原版的 35 ms 一拍。画面按你的显示器刷新率输出，只在两拍之间插值，
  所以 165 Hz 的显示器不会改变任何动作的耗时。
- **现代的文本管线。** 引擎里的文字全是 UTF-8，用稳定的键索引，语言包是 TOML，消息格式是 ICU 的一个子集。
  原版的 Big5 只存在于一个导入层。详见 [docs/zh-CN/i18n.md](docs/zh-CN/i18n.md)。
- **模组。** 把文件放进 `mods/<名称>/`，按逻辑名覆盖精灵图、地图、音频、文本、数值表和字体。
  详见 [docs/zh-CN/modding.md](docs/zh-CN/modding.md)。

## 快速开始

需要 Rust（stable）和你自己的游戏副本，比如来自 Steam。

```powershell
git clone https://github.com/PandaQuQ/newpal-rebirth
cd newpal-rebirth
cargo build --release -p newpal-game
target\release\newpal.exe --validate            # 查找并检查游戏目录
target\release\newpal.exe                       # 从客栈开场进入
```

游戏目录按这个顺序确定：`--game-dir`、环境变量 `NEWPAL_GAME_DIR`、`newpal.toml` 里的 `game_dir`、
Steam 自动检测，最后弹出选择目录窗口。更多见 [docs/zh-CN/getting-started.md](docs/zh-CN/getting-started.md)。

## 文档

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

## 工作区

| crate | 作用 |
|---|---|
| `crates/newpal-assets` | 原版格式读取器（索引包、LZO1X、RLE 精灵、LMF 地图、ACT 动画表、对话脚本、数值表）、模组 VFS、游戏目录检测与校验、i18n 层。 |
| `crates/newpal-vm` | 对话脚本虚拟机，带 `ScriptBackend` trait 和按操作码划分的处理函数。 |
| `crates/newpal-prepare` | 离线工具：高清批处理、`newpal-patch`（构建 / 应用 / 校验残差补丁）、`newpal-i18n`（导出和检查语言包）。 |
| `crates/newpal-game` | `newpal` 可执行文件：winit + wgpu 客户端，带深度遮挡的地图渲染、行走、对话、音频。 |

## 法律

与大宇资讯（Softstar）及《仙剑奇侠传》的任何权利人无关联，未获其认可或赞助。所有商标、名称和原版游戏内容归各自所有者。
本项目面向拥有正版游戏、想在现代引擎上游玩的玩家。引擎代码以 **MIT OR Apache-2.0** 双许可发布，任选其一
（[LICENSE-MIT](LICENSE-MIT)、[LICENSE-APACHE](LICENSE-APACHE)）。许可只覆盖本仓库，不覆盖游戏本身。
