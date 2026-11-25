# Bundled UI fonts

`NewPalKai-Regular.ttf`, `NewPalKai-Medium.ttf` are **subsets** of
[LXGW WenKai](https://github.com/lxgw/LxgwWenKai) v1.530 (霞鹜文楷), © 2021-2026 LXGW,
licensed under the **SIL Open Font License 1.1** (`OFL.txt`).

* Only the characters used by the builtin `ui:` strings (`newpal-assets/src/i18n/builtin/*.toml`) + ASCII + common CJK punctuation are kept
  (~250 KB each instead of ~26 MB).
* The OFL reserves the names "霞鹜 / 霞鶩 / 落霞孤鹜 / 落霞孤鶩 / LXGW" for modified versions, so the font-name
  records are renamed to **NewPal Kai**. Copyright, designer and license records are unchanged.
* The fonts are embedded in the binary (`include_bytes!`) and are not offered as installable desktop fonts.
* Characters outside the subset (paths, mod names, pack translations) fall back to host fonts (language-pack
  fonts, the player's own `CT.ttf`) and then system fonts at runtime.

Rebuild after changing UI strings (needs `pip install fonttools`):

```
python tools/fonts/subset_ui_font.py LXGWWenKai-Regular.ttf LXGWWenKai-Medium.ttf crates/newpal-ui/fonts crates/newpal-assets/src/i18n/builtin/*.toml
```
