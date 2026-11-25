[English](../en/getting-started.md) | [简体中文](getting-started.md) · [返回 README](../../README.zh-CN.md)

# 快速上手

## 准备

- **你自己的《新仙剑奇侠传》**（新仙劍奇俠傳）。引擎基于 Steam 版开发（应用号 695630，构建号 2.17.102.0）。
  其他版本也许能用，校验时只会给出警告。
- **Rust**，stable 通道。项目在 Windows 的 `x86_64-pc-windows-gnu` 工具链上开发（不需要 MSVC，也不需要管理员权限）。
  其他平台没有测试过。
- 支持 Vulkan、DX12 或 Metal 的显卡（`wgpu` 能用就行）。

## 编译

```powershell
git clone https://github.com/PandaQuQ/newpal-rebirth
cd newpal-rebirth
cargo build --release -p newpal-game     # 生成 target\release\newpal.exe
cargo test                               # 单元测试；找不到游戏安装时，golden 测试会自行跳过
```

## 引擎怎么找到你的游戏

游戏目录按下面的顺序确定，第一个通过校验的候选生效。

1. `--game-dir <路径>`
2. 环境变量 `NEWPAL_GAME_DIR`
3. `newpal.toml` 里的 `game_dir`。便携模式下这个文件在 exe 旁边，否则是 `%APPDATA%\NewPAL-Reborn\newpal.toml`。
4. **Steam 自动检测。** 引擎找到 Steam 根目录（注册表或默认路径），读取 `steamapps\libraryfolders.vdf` 和
   `appmanifest_695630.acf`，使用 `steamapps\common\<installdir>`。

都找不到时，会弹出**选择目录窗口**。选好的目录经过校验后记进 `newpal.toml`。随时可以用 `newpal --pick` 重新选择。

**校验**会检查必需文件是否齐全（`*.dat`、`all_char.tsw/.act`、`huge*.lmf` 等），把文件大小和已知的 Steam 版比较，
并比较每个小于 2 MiB 的关键文件的 SHA-256。构建不同只给警告。目录以**只读**方式打开：
不会从里面复制任何东西，也不会往里面写入，更不会碰游戏进程。

## 配置

`newpal.toml`（已在 `.gitignore` 里，因为里面是你自己的路径）：

```toml
game_dir = 'D:\Games\NewPAL'      # 你自己的安装路径
mods = ["hd-sprites"]             # 启用的模组，靠前优先级高（留空 = 全部，按优先级）
view_height = 480                 # 逻辑视图高度，宽度跟随窗口比例
window_size = [1280, 720]
language = "zh-Hans"              # BCP-47 语言标签，见 docs/zh-CN/i18n.md

[video]
vsync = true
fps_cap = 0                       # 0 = 不限
interpolate = true
smooth_frames = false
crossfade = false

[smooth]                          # 可选：本地 RIFE 补帧，默认关闭
backend = "off"                   # off | auto | gpu | cpu
quality = 4
workers = 2
```

工作文件（高清缓存、下载的工具、输出）放在*数据根目录*下，确定顺序：`--data-root` / `newpal.toml` 里的
`[paths] data_root`，然后是 `NEWPAL_DATA_ROOT`，然后是 exe 旁或当前目录里找到的项目文件夹（含 `LAYOUT.md`，或同时含
`cache/` 和 `tools/`），最后是 `%LOCALAPPDATA%\NewPAL-Reborn`。数据根目录里的东西永远不会提交到仓库。

## 运行

```powershell
newpal --validate                                   # 检查游戏目录
newpal                                              # 地图 huge:2（客栈），开场对话脚本 104
newpal --map huge:30 --script 0 --size 1920x1080
newpal --map huge:2 --pos 470,430 --screenshot shot.png        # 无窗口渲染一帧到 PNG
newpal --script 0 --walk-to 168,568 --sim 15 --screenshot s.png   # 走出客栈（切换场景）
newpal --mute --debug-nav                           # 静音，开启导航叠加层
newpal --lang zh-Hans                               # 界面语言，见 i18n 文档
newpal --help                                       # 完整的选项列表
```

`--screenshot` 生成的截图含有原版美术，请只留在本地，不要公开。

### 操作

| 输入 | 动作 |
|---|---|
| WASD / 方向键 | 8 方向行走 |
| 左键点击 | 走到点击处（在地图行走网格上做 A* 寻路，会绕开 NPC） |
| 回车 / 空格 / 点击 | 推进对话；没有对话时，和身边的 NPC 说话，或触发脚下的区域 |
| 踏入出口区域 | 执行其脚本，例如离开客栈会载入小镇 |
| F3 | 导航叠加层（红 = 阻挡，蓝 = 区域，绿 = 路径） |
| F4 | 插值 |
| F5 / F6 | 补间帧 / 交叉淡化 |
| F7 / F8 | 帧率上限 / 垂直同步 |
| F9 | 循环切换：原图 / 着色器放大 / 高清静帧 |
| F10 | 切换着色器：Anime4K 风格 / xBR |
| F11 | 保持时长的光流变形 开/关 |
| F12 | 切换语言 |
| T | 重新开始脚本 |
| Esc | 退出 |

`--original` 会关闭所有增强，方便 A/B 对比。标题栏显示帧率和各开关状态。
