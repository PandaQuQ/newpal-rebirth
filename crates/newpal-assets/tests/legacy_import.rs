//! The legacy import layer against the player's own install (skipped when it is not found): everything the
//! engine receives from the original data must be valid UTF-8 markup / message text, and the translation export
//! must cover dialogue that is only reachable by jumps.
use newpal_assets::gamedir::{self, Config};
use newpal_assets::i18n::{markup, message, TextSource, ORIGINAL_LOCALE};
use newpal_assets::legacy::{build, source, stringdb};
use newpal_assets::talk;
use newpal_assets::vfs::Vfs;
use std::path::PathBuf;

fn game() -> Option<PathBuf> {
    let p = Config::default_path();
    let cfg = Config::load_from(&p).unwrap_or_default();
    match gamedir::resolve(None, &cfg, &p) {
        Ok(r) => Some(r.dir),
        Err(e) => {
            eprintln!("SKIP legacy import test: {e}");
            None
        }
    }
}

#[test]
fn dialogue_is_clean_markup_and_the_full_sweep_finds_more() {
    let Some(d) = game() else { return };
    let vfs = Vfs::new(d, Vec::new());
    let st = source::dialogue_stats(&vfs);
    eprintln!("dialogue: {st:?}");
    assert!(st.scripts > 100);
    assert!(st.full >= st.linear && st.linear > 1000, "{st:?}");
    let (mut bad, mut left, mut n, mut junk) = (Vec::new(), 0usize, 0usize, Vec::new());
    for s in 1..=8000u32 {
        for (pos, _, t) in source::script_dialogue(&vfs, s, true).unwrap_or_default() {
            n += 1;
            if t.contains('\u{FFFD}') && junk.len() < 5 {
                junk.push(format!("talk:{s}:{pos:04x}"));
            }
            let p = markup::parse(&t);
            if !p.warnings.is_empty() && bad.len() < 5 {
                bad.push(format!("talk:{s}:{pos:04x}: {:?}", p.warnings));
            }
            if t.contains("%C") || t.contains("%N") || t.contains("%S") || t.contains("%G") || t.contains("%K") || t.contains("%P") {
                left += 1;
            }
        }
    }
    eprintln!("{n} dialogue strings parsed");
    assert!(bad.is_empty(), "markup warnings: {bad:?}");
    assert!(junk.is_empty(), "undecodable bytes in dialogue (operand noise?): {junk:?}");
    assert_eq!(left, 0, "legacy control codes leaked through the import layer");
}

#[test]
fn markup_conversion_matches_the_raw_text() {
    let Some(d) = game() else { return };
    let vfs = Vfs::new(d, Vec::new());
    // raw (original-form) text converted by hand == what the disassembler hands out
    for n in [104u32, 135, 1000, 2001, 3001, 5001, 6001] {
        let Ok(bytes) = vfs.read_raw(&talk::file_for(n)) else { continue };
        let Ok(s) = talk::script_from_file(&bytes, n) else { continue };
        let raw = talk::disasm_raw_text(&s.code);
        let cooked = talk::disasm(&s.code);
        for (a, b) in raw.instrs.iter().zip(cooked.instrs.iter()) {
            if let (talk::Instr::Op { op, text: Some(r), .. }, talk::Instr::Op { text: Some(c), .. }) = (a, b) {
                let expect = if matches!(*op, 0xc2 | 0x60) { r.clone() } else { newpal_assets::legacy::markup::from_legacy(r, true) };
                assert_eq!(&expect, c, "script {n}");
            }
        }
    }
}

#[test]
fn string_tables_convert_and_stay_consistent() {
    let Some(d) = game() else { return };
    let src = source::LegacySource::new(d.clone());
    let hant = src.db(ORIGINAL_LOCALE).expect("zh-Hant StringDB.txt");
    assert!(hant.entries.len() > 500, "{} entries", hant.entries.len());
    let mut problems = Vec::new();
    for (tag, db) in [("zh-Hant", Some(hant.clone())), ("zh-Hans", src.db("zh-Hans")), ("en", src.db("en"))] {
        let Some(db) = db else { continue };
        for id in db.entries.keys() {
            let m = db.message(*id).unwrap();
            let p = markup::parse(&message::format(&m, &[], tag));
            if !p.warnings.is_empty() {
                problems.push(format!("{tag} {id}: {:?}", p.warnings));
            }
        }
    }
    assert!(problems.is_empty(), "{problems:?}");
    // the real Simplified table keeps the placeholders / colours of the original line
    let hans = src.db("zh-Hans").expect("zh-Hans StringDB.txt");
    let mut diff = Vec::new();
    for id in hant.entries.keys().filter(|id| stringdb::translatable(**id)) {
        if let (Some(a), Some(b)) = (hant.message(*id), hans.message(*id)) {
            if markup::signature(&a) != markup::signature(&b) {
                diff.push(*id);
            }
        }
    }
    eprintln!("zh-Hans entries whose placeholders / tags differ from zh-Hant: {diff:?}");
    assert!(diff.len() < 5, "{diff:?}");
    // official translation is picked up, the byte-copy `en` table is not
    assert!(src.shipped("zh-Hans", &stringdb::key(1006)).is_some());
    assert!(src.shipped("en", &stringdb::key(1006)).is_none());
    // every id has a category and a stable key
    for id in hant.entries.keys() {
        assert_eq!(stringdb::parse_key(&stringdb::key(*id)), Some(*id));
    }
}

#[test]
fn build_detection() {
    let Some(d) = game() else { return };
    let b = build::detect(&d);
    eprintln!("build: {:?} notes {:?}", b.known.map(|k| k.label), b.notes);
    assert!(b.exe_sha256.is_some());
}
