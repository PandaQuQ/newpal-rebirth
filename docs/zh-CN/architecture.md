[English](../en/architecture.md) | [简体中文](architecture.md) · [返回 README](../../README.zh-CN.md)

# 架构概览

## crate 划分

```
newpal-game      `newpal` 可执行文件：winit 窗口、wgpu 渲染、游戏主循环、音频、命令行
├── newpal-vm        对话脚本虚拟机（ScriptBackend trait，按操作码划分的处理函数）
├── newpal-prepare   离线工具：高清批处理、newpal-patch、newpal-i18n、可选的 RIFE / 光流原型
└── newpal-assets    原版格式读取器、模组 VFS、游戏目录检测、i18n 目录
```

`newpal-assets` 是唯一了解原版文件格式的 crate，它之上的代码只处理解码后的帧、数值表和 `String`。

## 读取原版数据

- **索引包**（`.tsw`、`.act`、`.snd`）：槽位表，载荷用 LZO1X 压缩。
- **精灵图**是 RLE 图像。**地图**（`huge.lmf`、`huge2.lmf`）包含合成好的背景、用于深度遮挡的物件抠图和行走网格。
- **ACT** 是动画表；**对话脚本**是字节码，由 `newpal-vm` 反汇编并执行。
- `maps.dat` 提供场景表和 NPC 摆放。`ghero / gitem / gmagic / gmon / glevel` 是数值表。
- 全部小端序。读取器通过只读句柄，直接在你的安装文件上原地工作。

## 虚拟文件系统与模组

任何资源的查找顺序是：**模组 > 本地高清缓存 > 原版**。模组文件按稳定的逻辑名覆盖原版资源
（`sprites/all_char/187/f4.png`、`maps/huge/30/bg.png` 等）。见 [modding.md](modding.md)。

## 主循环与帧率

- 逻辑以**固定 35 ms 一拍**运行（约 28.6 Hz），即原版主循环的速率。ACT 的 `DS` 延迟以拍计数。
- 渲染按显示器刷新率进行（垂直同步、不限帧或 `--fps-cap`），在最近两拍之间对主角、镜头、环境时钟和对话淡入淡出做插值。
- 行走帧按走过的距离选取，被挡住时脚不会滑动。
- 镜头是缓动的指数跟随，限制在地图范围内，并支持宽屏。
- 插值只在关键帧之间取样，从不改变关键帧时间，所以动画时长与原版一致。见 [hd-patch.md](hd-patch.md)。

## 渲染

精灵图和地图背景是位图，由带深度遮挡的 wgpu 精灵渲染器在**逻辑高度 480 像素**的空间里绘制，
所以 4 倍高清静帧按同样的逻辑尺寸显示，4K 或超宽屏只影响镜头。放大有三种模式，用 F9 循环切换：
原图（最近邻）、实时着色器（Anime4K 风格或 xBR）、由残差补丁还原的高清静帧。

## 文本

所有文本都是 UTF-8，通过 i18n 目录用稳定的键访问。原版的 Big5、`%` 控制码和 printf 格式在一个导入层
（`newpal-assets/src/legacy`）里转换，并由一个策略测试（`tests/text_policy.rs`）保证它与其他代码隔离。见 [i18n.md](i18n.md)。

## 离线工具（`newpal-prepare`）

| 工具 | 作用 |
|---|---|
| `newpal-patch build / apply / cmp` | 构建、应用、比较残差高清补丁。见 [hd-patch.md](hd-patch.md)。 |
| `newpal-i18n info / export / check / ui-list / migrate` | 查看你安装里的文本表，管理语言包。 |
| `hd_batch`（example） | 本地 4 倍 Real-ESRGAN 批处理，生成构建补丁所用的高清缓存。 |

## `modern-ui` 分支

`modern-ui` 分支仍在开发中，**没有合并进 `main`**。它新增 `newpal-ui` crate：启动器、标题菜单、设置、读档和暂停界面，
以及原版风格的对话框和系统菜单，全部用 `vello` 在引擎自己的 wgpu 设备上以**矢量**方式绘制
（字形轮廓由 `skrifa` 排版，没有位图文字）。分支里还内置了开源字体“霞鹜文楷”（LXGW WenKai）的子集
（SIL OFL 1.1，附许可证文件），让启动器在找到游戏文件之前就能渲染文字。界面与分辨率无关，只通过一个很小的
`Host` / `TextSource` / `Cmd` 接口和引擎交互。该分支上的 `docs/modern-architecture.md` 有更详细的说明。
