[English](../en/modding.md) | [简体中文](modding.md) · [返回 README](../../README.zh-CN.md)

# 模组

模组放在 exe 旁边的 `mods/<名称>/` 里（或 `--mods-dir` / `mods_dir` 指定的目录）。`mod.toml` 可选：

```toml
name = "my-mod"
priority = 10        # 越大越优先
enabled = true
```

`newpal.toml` 里的 `mods = [...]` 可以显式指定启用哪些模组（靠前优先级高）。留空则使用找到的全部模组，按 `priority` 排序。

模组文件通过稳定的**逻辑名**替换原版资源。你没提供的文件仍然来自原版。模组叠在高清缓存和原版数据之上。

| 逻辑名 | 替换的内容 |
|---|---|
| `sprites/<tsw>/<id>/f<k>.png` | tsw 条目 id 的第 k 帧（从 1 开始，与 ACT 的 `FR` 一致），如 `sprites/all_char/187/f4.png` |
| `maps/<lmf>/<id>/bg.png` | 地图背景，如 `maps/huge/30/bg.png` |
| `maps/<lmf>/<id>/obj_<oid>.png` | 地图物件（深度抠图） |
| `audio/snd/<slot>.wav` | 音效（`all.snd` 槽位，从 0 开始；脚本里的 id = 槽位 + 1） |
| `music/<stem>.ogg` / `.mp3` / `.wav` / `.flac` | 背景音乐，如 `music/m027.ogg` 替换原版的 `m027.mp3` |
| `act/<act>/<slot>/<action>/anim.toml` + `f000.png ...` | 某个动作的补间帧 |
| `text/talk/<n>.json` | 按指令位置给出对话文本：`{ "0024": "新的一行" }` |
| `lang/<locale>/*.toml` | 语言包，见 [i18n.md](i18n.md) |
| `scripts/talk/<n>.bin` | 脚本 n 的完整字节码 |
| `tables/<name>.json` / `.toml` | 给 `ghero / gitem / gmagic / gmon / glevel` 记录打补丁（按 `id` 匹配；新 id 需要 `kind`） |
| `fonts/dialog.ttf` | 对话字体 |
| `ui/dialogue.toml` | 对话框主题（`modern-ui` 分支） |
| `raw/<路径>` | 任意原版文件，逐字节替换 |

**高清和缩放美术：** `f4@2x.png`、`bg@4x.png` 等按“逻辑尺寸 = 原生尺寸 / N”显示。比原图大的普通 `f4.png`
按原图的逻辑尺寸显示。渲染器以逻辑单位工作（480 像素 = 原版视图高度），所以任何分辨率都能用。

**脚本：** 脚本虚拟机提供 `Vm::register(op, handler)` 和 `ScriptBackend` trait。以后可以加入 Lua 或 Rhai 后端，
不必改动主循环。

## 发布模组的规则

- 不要包含原版游戏文件，也不要包含从中提取或转换出的任何东西，包括重新编码的音频、放大的帧和文本转储。
- 欢迎自己写的翻译、新美术、新音乐和脚本，许可证由你自己选。
- 用 `newpal-i18n export --with-source` 导出的语言包会把原文放在注释里，请只留在本地。
  工具会在旁边写一个 `.gitignore` 就是为了这个。
