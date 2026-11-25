#!/usr/bin/env python3
"""Build the bundled UI font subsets (crates/newpal-ui/fonts/NewPalKai-*.ttf).

Source: LXGW WenKai v1.530 (SIL Open Font License 1.1), https://github.com/lxgw/LxgwWenKai
The OFL reserves the names '霞鹜', '霞鶩', '落霞孤鹜', '落霞孤鶩' and 'LXGW' for modified versions, and a subset is a
modified version, so the subset is renamed "NewPal Kai". The copyright and license records are kept.

usage: python subset_ui_font.py <LXGWWenKai-Regular.ttf> <LXGWWenKai-Medium.ttf> <out_dir> <text files...>
  e.g. crates/newpal-assets/src/i18n/builtin/*.toml (string values are decoded, so \\u escapes count)
Needs: pip install fonttools
"""
import re, sys, tomllib
from fontTools import subset
from fontTools.ttLib import TTFont

def read(path):
    if path.endswith(".toml"):
        data = tomllib.load(open(path, "rb"))
        vals = []
        def walk(v):
            if isinstance(v, dict):
                for x in v.values():
                    walk(x)
            elif isinstance(v, str):
                vals.append(v)
        walk(data)
        return "".join(vals)
    return open(path, encoding="utf-8").read()

def chars(files):
    text = "".join(read(f) for f in files)
    s = set(text)
    s |= {chr(c) for c in range(0x20, 0x7F)}
    s |= set("‹›←↑→↓⌫…·×—–「」『』《》〈〉（）【】、。，．：；！？“”‘’％＋－／～・〇一二三四五六七八九十百千万萬繁體简体中文日本語한국어")
    return "".join(sorted(c for c in s if ord(c) >= 0x20))

RENAME = [("LXGW WenKai", "NewPal Kai"), ("LXGWWenKai", "NewPalKai"), ("霞鹜文楷", "新仙界面楷"), ("霞鶩文楷", "新仙界面楷"), ("LXGW", "NewPal")]

def build(src, dst, text):
    opts = subset.Options()
    opts.layout_features = ["*"]
    opts.name_IDs = ["*"]
    opts.name_languages = ["*"]
    opts.notdef_outline = True
    opts.glyph_names = False
    f = TTFont(src)
    sub = subset.Subsetter(opts)
    sub.populate(text=text)
    sub.subset(f)
    names = {1, 3, 4, 6, 16, 17, 18, 21, 22, 25}  # font-name records only; copyright, designer, license stay verbatim
    for rec in f["name"].names:
        if rec.nameID not in names:
            continue
        s = rec.toUnicode()
        for a, b in RENAME:
            s = s.replace(a, b)
        rec.string = s
    for rec in f["name"].names:
        if rec.nameID in names:
            assert "LXGW" not in rec.toUnicode() and "霞鹜" not in rec.toUnicode(), rec
    f.save(dst)

if __name__ == "__main__":
    reg, med, out = sys.argv[1:4]
    text = chars(sys.argv[4:])
    build(reg, f"{out}/NewPalKai-Regular.ttf", text)
    build(med, f"{out}/NewPalKai-Medium.ttf", text)
    print(len(text), "characters")
