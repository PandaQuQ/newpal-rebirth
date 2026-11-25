[English](../en/hd-patch.md) | [简体中文](hd-patch.md) · [返回 README](../../README.zh-CN.md)

# 高清静帧、残差补丁与帧率

原版是 640x480 的精灵游戏。本项目希望它在现代显示器上也好看，同时不发布任何美术。设计分三层，从下往上：

1. **实时着色器放大**（始终可用）。Anime4K 风格或 xBR，用 F10 或 `--shader` 选择。
2. **高清静帧**（4 倍），*在本地用你自己的文件生成*。固定版本的 `realesrgan-ncnn-vulkan`
   （`realesrgan-x4plus-anime`，MIT）放大 `all_char / all_item / all_magic / all_map1 / all_map2 / all_sys.tsw`
   的每一帧，以及 `huge.lmf` / `huge2.lmf` 里合成好的整张地图背景（整图而不是分块，所以不会出现拼缝）。
   精灵图先拆成 RGB 板（做 3 像素颜色外溢后）和独立的 Alpha 板再放大。这需要几个小时的 GPU 时间，
   产物一律不分发。
3. **残差补丁**，让玩家不必自己跑这套放大。

运行时的查找顺序是 `模组 > 高清缓存 > 原版`。缺少的高清条目回退到着色器。逻辑尺寸不变，
所以有没有高清，镜头和插值的表现都一样。

## 残差补丁的设计

补丁为每一帧保存 `高清 - 基线(原帧)`，并**以原帧的 SHA-256 作为索引**。

- **基线**是位精确的 CPU Catmull-Rom 双三次 4 倍放大，使用定点数（16.16，`bicubic-catmull-rom-fp16.16`）。
  它不依赖 GPU 或驱动，所以在任何机器上构建和应用补丁得到的字节都一样。实时着色器*不是*基线。
- **残差**是有符号 16 位差值，逐通道 Paeth 预测，Alpha 平面单独存放，用 brotli（或 zstd）压缩。默认的 `brotli-45`
  会把残差量化到刚好保证与真实高清图的 **PSNR >= 45 dB**。其他编码：`lossless`、`zstd`、`brotli`、`zstd-45`。
- 大地图分条带存储（容器版本 3，`.nres`），可以并行编码和解码。
- `manifest.json` 把帧哈希映射到残差文件，相同的帧共用一个文件。
- **补丁既不含原版像素，也不含高清像素。** 它只是两张图的差，没有对应的原帧就毫无用处，清单里也这样声明。

命令行：

```powershell
cargo run --release -p newpal-prepare --bin newpal-patch -- build --game-dir <NewPAL> --hd-cache <高清缓存> --out <补丁目录> --only all_char --encoding brotli-45 --workers 2
cargo run --release -p newpal-prepare --bin newpal-patch -- apply --game-dir <NewPAL> --patch <补丁目录> --hd-cache <新的高清缓存>
newpal-patch cmp --a <目录> --b <目录>        # 比较两个缓存
```

`apply` 读取你的原帧，算出基线，加上残差，把高清静帧写进本地缓存。你的游戏目录只会被读取。

### 实测数据

200 帧 `all_char`（原型）：高清 PNG 9.95 MB；无损残差 13.4 MB（zstd-22）或 11.7 MB（brotli-11），约为高清 PNG 的 1.2 到 1.35 倍；
近无损 `brotli-45` 为 5.4 MB（0.54 倍），应用耗时 0.56 秒，最低 PSNR 45.00。

维护者机器上的完整构建（全部精灵图和地图，`brotli-45`）：20,865 个条目，18,349 个不重复的残差文件，共 **8.51 GiB**，
是 19.0 GiB 高清缓存的 44.8%。整个构建中最低 PSNR 为 45.00 dB。完整的计时应用和按类别的 PSNR 全量复验还没做完。

**目前没有发布任何补丁。** 在 [legal.md](legal.md) 里的法律问题弄清之前，任何人都不应该再分发补丁。这不是法律意见。

## 帧率与插值

- 逻辑是固定 35 ms 一拍。渲染按显示器刷新率（60 / 165 / 300 Hz 或不限帧）在两拍之间插值。
- **关键帧时间永远不变。** 例如主角的行走是 8 帧、每帧 105 ms，一圈 840 ms，不论刷新率是多少。
- 两个关键帧之间，渲染器可以用打包的光流场做变形（量化的 i16 8.8 加置信度，Paeth + zstd）。
  置信度低的区域（遮挡、突然转向）回退到交叉淡化。F11 开关，`--no-flow` 以硬切启动。
- 可选的 RIFE 补间帧（`[smooth] backend`，默认 `off`）可以用固定版本的 `rife-ncnn-vulkan`（MIT，下载时校验 SHA-256）
  在本地生成。它们只是本地缓存，不会分发。
- 额外帧的查找顺序：用户模组 > 本地缓存 > 原帧。
