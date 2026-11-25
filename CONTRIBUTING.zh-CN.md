[English](CONTRIBUTING.md) | **简体中文**

# 参与贡献

感谢关注。这是个早期的小项目，所以规矩不多。

## 唯一的硬规矩：不要原版素材

本仓库绝不能包含来自原版游戏的任何东西，也不能包含由它派生出来的任何东西：

- 精灵图、地图、背景、音频、音乐、视频、调色板、字体（包括 `CT.ttf`）、脚本、字符串表（`StringDB.txt`）、
  对话转储、Big5 转储；
- 含有原版美术的截图，放大或还原后的帧（`@4x.png` 之类），残差补丁（`.nres`、`manifest.json`）；
- *附带原文的*翻译（例如 `newpal-i18n export --with-source` 的输出）。

包含以上任何一种内容的 pull request 会被关闭，历史记录也得清理。拿不准能不能放，请先开 issue 问。
需要游戏的测试必须从玩家自己的安装里读取，找不到时自行跳过（参见 `crates/newpal-assets/tests/golden.rs`）。
可选的本地文件 `crates/newpal-assets/tests/golden.json` 保存这些测试用的哈希；用
`NEWPAL_WRITE_GOLDEN=1 cargo test -p newpal-assets --test golden write_golden` 从你自己的安装生成，不要提交
（已加入 .gitignore）。

也请不要把你自己的路径、令牌和个人信息提交进来。`newpal.toml` 是特意被忽略的。

## 环境

```powershell
git clone https://github.com/PandaQuQ/newpal-rebirth
cd newpal-rebirth
cargo build -p newpal-game
cargo test
cargo clippy --workspace --all-targets
cargo fmt --all
```

跑 golden 测试和运行引擎都需要你自己的游戏副本。见 [docs/zh-CN/getting-started.md](docs/zh-CN/getting-started.md)。

## 代码

- Rust stable。保持 `cargo test` 通过；新增读取器或操作码时尽量补测试。
- **文本策略：** `crates/newpal-assets/src/legacy/` 之外的代码里不得出现 Big5、编解码器和中日韩字符。
  所有用户可见的字符串都走 i18n 键。`tests/text_policy.rs` 会检查。
- 不要修改或写入游戏目录，也不要触碰游戏进程。该目录只是只读输入。
- 保持游戏时长不变：逻辑 35 ms 一拍。渲染和插值不得改变时长。
- 优先提交小而专注的 pull request，并写清楚改了什么、为什么改。

## 翻译与语言包

语言包就是普通的 TOML，见 [docs/zh-CN/i18n.md](docs/zh-CN/i18n.md)。只贡献你自己的译文。
想公开发布的语言包里不要贴原文。没翻译的键可以留空。

## 文档

英文和简体中文文档并排维护（`docs/en`、`docs/zh-CN`、`README.md`、`README.zh-CN.md`）。改了一边，请同步另一边，
或在 PR 里注明翻译还待补。

## 许可

提交贡献即表示你同意你的贡献以 **MIT OR Apache-2.0** 授权，与项目一致。
