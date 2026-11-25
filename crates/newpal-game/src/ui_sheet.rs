//! `--ui-sheet <png>`: render the original game's own strings (battle, menu, shop, messages ...) exactly as the
//! engine resolves them for the selected language, as a contact sheet. The menu / battle / shop *screens* are not
//! implemented in the engine yet; this proves the catalog -> markup -> font stack -> pixels path for those keys and
//! gives translators a visual check. Placeholders (`{1}`) are shown verbatim.
use crate::Args;
use newpal_assets::i18n::{markup, Source};
use newpal_assets::image::RgbaImage;
use newpal_assets::legacy::source;
use newpal_assets::talk::Span;
use newpal_assets::vfs::Vfs;
use std::collections::BTreeMap;
use std::path::Path;

fn over(dst: &mut RgbaImage, src: &RgbaImage, dx: i32, dy: i32) {
    for y in 0..src.height as i32 {
        for x in 0..src.width as i32 {
            let (tx, ty) = (dx + x, dy + y);
            if tx < 0 || ty < 0 || tx >= dst.width as i32 || ty >= dst.height as i32 {
                continue;
            }
            let s = ((y as u32 * src.width + x as u32) * 4) as usize;
            let a = src.pixels[s + 3] as u32;
            if a == 0 {
                continue;
            }
            let o = ((ty as u32 * dst.width + tx as u32) * 4) as usize;
            for c in 0..3 {
                dst.pixels[o + c] = ((src.pixels[s + c] as u32 * a + dst.pixels[o + c] as u32 * (255 - a)) / 255) as u8;
            }
        }
    }
}

pub fn run(vfs: &Vfs, args: &Args, out: &Path) -> Result<(), String> {
    let i18n = vfs.i18n();
    let text = crate::world::build_text_renderer(vfs).ok_or("no usable font")?;
    let (sw, sh) = args.size;
    let mut sheet = RgbaImage::new(sw, sh);
    for p in sheet.pixels.chunks_exact_mut(4) {
        p.copy_from_slice(&[16, 14, 26, 255]);
    }
    let cat = args.ui_cat.as_str();
    let entries: Vec<_> = source::stringdb_entries(vfs.game_dir()).into_iter().filter(|e| e.group == cat).filter_map(|e| e.key.split(':').nth(1).and_then(|n| n.parse::<u32>().ok()).map(|id| (id, e.key))).filter(|(id, _)| *id >= args.ui_from).take(args.ui_count).collect();
    if entries.is_empty() {
        return Err(format!("no strings in category {cat:?} from id {} (categories: sys ui menu battle shop msg name err)", args.ui_from));
    }
    let mut tally: BTreeMap<&'static str, usize> = BTreeMap::new();
    let px = 20.0f32;
    let (margin, colw) = (16u32, ((sw - 16) / 2).saturating_sub(24).max(200));
    let head_h = 40u32;
    let (mut x, mut y) = (margin, head_h);
    let mut shown = 0usize;
    for (id, key) in &entries {
        let r = i18n.resolve(key, None).ok_or_else(|| format!("{key} did not resolve"))?;
        let label = match &r.source {
            Source::Pack(_) => "pack",
            Source::Builtin(_) => "builtin",
            Source::Shipped(_) => "shipped",
            Source::Converted => "converted",
            Source::Original => "original",
        };
        *tally.entry(label).or_default() += 1;
        let parsed = markup::parse(&i18n.t_fmt(key, &[]));
        let mut lines = parsed.lines;
        if lines.is_empty() {
            lines.push(vec![Span { color: 7, text: "(empty)".into() }]);
        }
        lines[0].insert(0, Span { color: 7, text: format!("{id}  ") });
        let img = text.render(&lines, px, colw);
        if y + img.height + 4 > sh {
            x += colw + 24;
            y = head_h;
        }
        if x + colw > sw {
            break;
        }
        over(&mut sheet, &img, x as i32, y as i32);
        y += img.height + 4;
        shown += 1;
    }
    let head = format!("{}  |  {cat}  |  {shown} strings  |  {tally:?}", i18n.locale());
    let h = text.render(&[vec![Span { color: 6, text: head }]], px, sw - 2 * margin);
    over(&mut sheet, &h, margin as i32, 8);
    sheet.save_png(out).map_err(|e| e.to_string())?;
    eprintln!("ui sheet: {shown} {cat} strings, locale {}, sources {tally:?} -> {}", i18n.locale(), out.display());
    Ok(())
}
