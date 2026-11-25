[English](hd-patch.md) | [简体中文](../zh-CN/hd-patch.md) · [Back to README](../../README.md)

# HD stills, residual patches and frame pacing

The original game is a 640x480 sprite game. This project wants it to look good on a modern display without
shipping any art. The design has three layers, from the bottom up.

1. **Realtime shader upscale** (always available). Anime4K-style or xBR, selected with F10 or `--shader`.
2. **HD stills** (4x), generated *locally from your own files*. A pinned `realesrgan-ncnn-vulkan`
   (`realesrgan-x4plus-anime`, MIT) upscales every frame of `all_char / all_item / all_magic / all_map1 /
   all_map2 / all_sys.tsw`, and each composed `huge.lmf` / `huge2.lmf` background as a whole map, so no tile seams
   appear. Sprites go through an RGB plate (after a 3 px colour bleed) and a separate alpha plate. This takes hours
   of GPU time, and nothing it produces is distributed.
3. **Residual patches**, so players do not have to run that upscale themselves.

Runtime lookup is `mods > HD cache > original`. A missing HD entry falls back to the shader. Logical size does not
change, so camera and interpolation behave the same with or without HD.

## Residual patch design

A patch stores `HD - baseline(original)` for each frame, **keyed by the SHA-256 of the original frame**.

- The **baseline** is a bit-exact CPU Catmull-Rom bicubic 4x upscale in fixed point (16.16,
  `bicubic-catmull-rom-fp16.16`). It does not depend on the GPU or driver, so building and applying a patch give
  the same bytes on every machine. The realtime shader is *not* the baseline.
- The **residual** is a signed 16-bit difference with per-channel Paeth prediction and a separate alpha plane,
  compressed with brotli (or zstd). The default `brotli-45` quantises the residual just enough to keep
  **PSNR >= 45 dB** against the true HD image. Other encodings: `lossless`, `zstd`, `brotli`, `zstd-45`.
- Large maps are stored in stripes (container version 3, `.nres`), so they can be encoded and decoded in parallel.
- `manifest.json` maps frame hashes to residual files. Identical frames share one file.
- **A patch contains neither original pixels nor HD pixels.** It is the difference between two images, and is
  useless without the matching original frame. Its manifest says so too.

Command line:

```powershell
cargo run --release -p newpal-prepare --bin newpal-patch -- build --game-dir <NewPAL> --hd-cache <hd-cache> --out <patch-dir> --only all_char --encoding brotli-45 --workers 2
cargo run --release -p newpal-prepare --bin newpal-patch -- apply --game-dir <NewPAL> --patch <patch-dir> --hd-cache <new-hd-cache>
newpal-patch cmp --a <dir> --b <dir>        # compare two caches
```

`apply` reads your original frames, computes the baseline, adds the residual, and writes the HD stills into a local
cache. Your game folder is only read.

### Measurements

200 `all_char` frames (the prototype): HD PNGs 9.95 MB; lossless residual 13.4 MB (zstd-22) or 11.7 MB (brotli-11),
about 1.2x to 1.35x the HD PNGs; near-lossless `brotli-45` 5.4 MB (0.54x), apply in 0.56 s, min PSNR 45.00.

Full build on the maintainer's machine (all sprites and maps, `brotli-45`): 20,865 entries, 18,349 unique residual
files, **8.51 GiB** in total, which is 44.8 % of the 19.0 GiB HD cache. The minimum PSNR across the build was 45.00 dB.
A complete timed apply and a full per-class PSNR re-verification are still outstanding.

**No patch is published yet.** Nobody should redistribute one until the legal question in
[legal.md](legal.md) is settled. This is not legal advice.

## Frame pacing and interpolation

- Logic is a fixed 35 ms tick. Rendering interpolates between ticks at the display rate (60 / 165 / 300 Hz or uncapped).
- **Keyframe times never change.** For example the hero walk cycle is 8 frames of 105 ms, 840 ms per loop, at any
  refresh rate.
- Between two keyframes the renderer can warp with a packed optical-flow field (quantised i16 8.8 plus confidence,
  Paeth + zstd). Low-confidence regions (occlusion, sudden turns) fall back to a cross-fade. F11 toggles it and
  `--no-flow` starts with hard cuts.
- Optional RIFE in-between frames (`[smooth] backend`, default `off`) can be generated locally with the pinned
  `rife-ncnn-vulkan` release (MIT, SHA-256 verified on download). They are a local cache, never distributed.
- Lookup order for extra frames: user mods > local cache > original frames.
