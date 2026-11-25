//! The original-locale text source: turns the player's own install (talk scripts, `g*.dat` tables, the
//! `<locale>/StringDB.txt` string tables) into stable keys with UTF-8 / markup values. Read-only, in place,
//! lazily; nothing is copied and nothing is written next to the game.
//!
//! Two consumers:
//! * [`LegacySource`] - the runtime [`TextSource`] behind [`I18n`] (`battle:1006`, `menu:1619`... resolve to the
//!   original or to the translation the install ships for the locale);
//! * [`original_entries`] - everything translatable, for `newpal-i18n export` / `check`.
use super::stringdb::{self, StringDb};
use crate::g7f::Record;
use crate::i18n::export::Entry;
use crate::i18n::{self, key, markup, TextSource, ORIGINAL_LOCALE, TABLE_PREFIXES};
use crate::tables::Table;
use crate::talk;
use crate::vfs::Vfs;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Runtime text source over the player's game folder.
pub struct LegacySource {
    game: PathBuf,
    dbs: Mutex<HashMap<String, Option<Arc<StringDb>>>>,
    names: OnceLock<HashMap<String, u32>>,
    build: OnceLock<super::build::BuildInfo>,
}

impl LegacySource {
    pub fn new(game: PathBuf) -> Self {
        Self { game, dbs: Mutex::default(), names: OnceLock::new(), build: OnceLock::new() }
    }

    /// The string table of locale `tag` (folder name matched by canonical tag), cached.
    pub fn db(&self, tag: &str) -> Option<Arc<StringDb>> {
        let want = i18n::canonical(tag);
        let mut g = self.dbs.lock().unwrap();
        g.entry(want.clone())
            .or_insert_with(|| {
                let rd = std::fs::read_dir(&self.game).ok()?;
                let dir = rd.filter_map(|e| e.ok().map(|e| e.path())).find(|p| p.is_dir() && p.file_name().is_some_and(|n| i18n::canonical(&n.to_string_lossy()) == want) && p.join("StringDB.txt").is_file())?;
                StringDb::load(&dir.file_name()?.to_string_lossy(), &dir).map(Arc::new)
            })
            .clone()
    }

    pub fn build(&self) -> &super::build::BuildInfo {
        self.build.get_or_init(|| super::build::detect(&self.game))
    }

    /// original character name text -> StringDB id (name category)
    fn names(&self) -> &HashMap<String, u32> {
        self.names.get_or_init(|| {
            let mut m = HashMap::new();
            if let Some(db) = self.db(ORIGINAL_LOCALE) {
                for (id, raw) in &db.entries {
                    if stringdb::category(*id) == "name" {
                        m.entry(raw.trim().to_string()).or_insert(*id);
                    }
                }
            }
            m
        })
    }
}

/// Shipped text only counts as a translation when it differs from the original (the Steam `en` table is a copy
/// of `zh-Hant`) or carries no CJK text at all (symbols, key labels).
fn is_translation(shipped_raw: &str, original_raw: Option<&str>) -> bool {
    match original_raw {
        Some(o) if o == shipped_raw => !shipped_raw.chars().any(|c| c as u32 >= 0x2e80),
        _ => true,
    }
}

impl TextSource for LegacySource {
    fn original(&self, k: &str) -> Option<String> {
        let id = stringdb::parse_key(k)?;
        self.db(ORIGINAL_LOCALE)?.message(id)
    }

    fn shipped(&self, locale: &str, k: &str) -> Option<String> {
        if i18n::canonical(locale) == ORIGINAL_LOCALE {
            return None;
        }
        let id = stringdb::parse_key(k)?;
        let db = self.db(locale)?;
        let raw = db.entries.get(&id)?;
        let orig = self.db(ORIGINAL_LOCALE);
        is_translation(raw, orig.as_ref().and_then(|o| o.entries.get(&id)).map(String::as_str)).then(|| stringdb::to_message(raw))
    }

    fn shipped_name(&self, locale: &str, original: &str) -> Option<String> {
        let id = *self.names().get(original.trim())?;
        self.shipped(locale, &stringdb::key(id))
    }

    fn locales(&self) -> Vec<(String, String)> {
        let Ok(rd) = std::fs::read_dir(&self.game) else { return Vec::new() };
        let mut v: Vec<(String, String)> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir() && p.join("StringDB.txt").is_file())
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .map(|n| (i18n::canonical(&n), format!("{n} (shipped with the game)")))
            .collect();
        v.sort();
        v
    }

    fn notes(&self) -> Vec<String> {
        let b = self.build();
        let mut n = b.notes.clone();
        if let Some(k) = b.known {
            n.insert(0, format!("game build: {}", k.label));
        }
        n
    }
}

/// Some text ops carry binary parameters in front of the text; those must not become speaker keys.
fn plausible_name(s: &str) -> bool {
    !s.is_empty() && s.chars().count() <= 16 && !s.chars().any(|c| c.is_control() || c == '\u{fffd}' || c == '%' || c == '<' || c == '{')
}

/// Dialogue strings of one script: (instruction position, op, markup). `full` also sweeps the code that is only
/// reachable by jumps (everything after an end marker).
pub fn script_dialogue(vfs: &Vfs, n: u32, full: bool) -> Option<Vec<(usize, u16, String)>> {
    let bytes = vfs.read_raw(&talk::file_for(n)).ok()?;
    let s = talk::script_from_file(&bytes, n).ok()?;
    if s.code.len() < 2 || s.start >= bytes.len() {
        return None;
    }
    let d = if full { talk::disasm_full(&s.code) } else { talk::disasm(&s.code) };
    let mut out = Vec::new();
    for i in d.instrs {
        if let talk::Instr::Op { pos, op, text: Some(t), .. } = i {
            if talk::is_dialogue(op) {
                out.push((pos, op, t));
            }
        }
    }
    Some(out)
}

/// Script numbers the talk files can hold (`talk1..8.dat`, 1000 slots each).
fn script_ids() -> impl Iterator<Item = u32> {
    (1..=8u32).flat_map(|k| (1..=1000u32).map(move |slot| (k - 1) * 1000 + slot % 1000))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DialogueStats {
    pub scripts: usize,
    /// dialogue instructions found by the old sweep (stops at the first end marker)
    pub linear: usize,
    /// ... by the full sweep (past end markers)
    pub full: usize,
    /// per talk file (`talk1.dat` = index 0): (linear, full)
    pub per_file: BTreeMap<u32, (usize, usize)>,
}

pub fn dialogue_stats(vfs: &Vfs) -> DialogueStats {
    let mut st = DialogueStats::default();
    for n in script_ids() {
        let Some(f) = script_dialogue(vfs, n, true) else { continue };
        let l = script_dialogue(vfs, n, false).map(|v| v.len()).unwrap_or(0);
        st.scripts += 1;
        st.linear += l;
        st.full += f.len();
        let e = st.per_file.entry(n / 1000 + 1).or_default();
        e.0 += l;
        e.1 += f.len();
    }
    st
}

/// Counts of original control codes found in the dialogue (legacy letter -> uses), from the converted markup's
/// tags: shows which codes the data really uses.
pub fn tag_histogram(vfs: &Vfs) -> BTreeMap<String, usize> {
    let mut h: BTreeMap<String, usize> = BTreeMap::new();
    for n in script_ids() {
        for (_, _, t) in script_dialogue(vfs, n, true).unwrap_or_default() {
            for tag in markup::signature(&t) {
                let name = tag.trim_start_matches('/').split('=').next().unwrap_or("").to_string();
                if !tag.starts_with('{') {
                    *h.entry(if name == "ctl" { tag.clone() } else { name }).or_default() += 1;
                }
            }
        }
    }
    h
}

fn table_entries(prefix: &str, r: &Record, out: &mut Vec<Entry>) {
    let Some(id) = r.id else { return };
    for (f, v) in [("name", &r.name), ("desc", &r.desc)] {
        if let Some(v) = v.as_ref().filter(|v| !v.is_empty()) {
            out.push(Entry { key: key::field(prefix, id, f), text: v.clone(), note: String::new(), group: "tables".into() });
        }
    }
}

/// Every translatable string of the install, original language, as (key, markup / message text): dialogue of all
/// scripts (full sweep), speaker names, table names / descriptions and the `StringDB.txt` entries.
pub fn original_entries(vfs: &Vfs) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut names: BTreeMap<String, usize> = BTreeMap::new();
    for n in script_ids() {
        let Some(ds) = script_dialogue(vfs, n, true) else { continue };
        for (pos, op, text) in ds {
            if let Some(sp) = markup::parse(&text).speaker.filter(|s| plausible_name(s)) {
                *names.entry(sp).or_default() += 1;
            }
            out.push(Entry { key: key::talk(n, pos), text, note: format!("op {op:#x}"), group: format!("talk_{}", n / 1000 + 1) });
        }
    }
    for (n, c) in names {
        out.push(Entry { key: key::name(&n), text: n, note: format!("speaker, {c} lines"), group: "names".into() });
    }
    for (file, prefix) in TABLE_PREFIXES {
        let Ok(b) = vfs.read_raw(&format!("{file}.dat")) else { continue };
        let t = Table::from_dat(file, &b);
        for r in &t.records {
            table_entries(prefix, r, &mut out);
        }
    }
    out.extend(stringdb_entries(vfs.game_dir()));
    out
}

/// `StringDB.txt` entries of the original language (everything but SDL key labels), grouped by category.
pub fn stringdb_entries(game: &Path) -> Vec<Entry> {
    let src = LegacySource::new(game.to_path_buf());
    let Some(db) = src.db(ORIGINAL_LOCALE) else { return Vec::new() };
    db.entries
        .iter()
        .filter(|(id, raw)| stringdb::translatable(**id) && !raw.trim().is_empty())
        .map(|(id, _)| {
            let text = db.message(*id).unwrap_or_default();
            let slots = markup::signature(&text).iter().filter(|s| s.starts_with('{')).count();
            Entry { key: stringdb::key(*id), text, note: if slots > 0 { format!("original id {id}, {slots} placeholder(s)") } else { format!("original id {id}") }, group: stringdb::category(*id).into() }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("newpal-src-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (f, body) in [
            ("zh-Hant", "1350 \u{9673}\n1006 \u{653b}\u{64ca}\n1007 \u{8f14}\u{52a9}\n150 NONUSHASH\n"),
            ("zh-Hans", "1350 \u{9648}\n1006 \u{653b}\u{51fb}\n1007 \u{8f85}\u{52a9}\n150 NONUSHASH\n"),
            ("en", "1350 \u{9673}\n1006 \u{653b}\u{64ca}\n150 NONUSHASH\n"),
        ] {
            std::fs::create_dir_all(d.join(f)).unwrap();
            std::fs::write(d.join(f).join("StringDB.txt"), body).unwrap();
        }
        d
    }

    #[test]
    fn original_shipped_and_untranslated_copies() {
        let d = game("a");
        let s = LegacySource::new(d.clone());
        assert_eq!(s.original("battle:1006").as_deref(), Some("\u{653b}\u{64ca}"));
        assert_eq!(s.shipped("zh-Hans", "battle:1006").as_deref(), Some("\u{653b}\u{51fb}"));
        // the English table is a copy of the original: not a translation
        assert_eq!(s.shipped("en", "battle:1006"), None);
        // symbols without CJK text are fine to share
        assert_eq!(s.shipped("en", "key:150").as_deref(), Some("NONUSHASH"));
        assert_eq!(s.shipped("zh-Hant", "battle:1006"), None);
        // speaker names come from the name table
        assert_eq!(s.shipped_name("zh-Hans", "\u{9673}").as_deref(), Some("\u{9648}"));
        assert_eq!(s.shipped_name("en", "\u{9673}"), None);
        let tags: Vec<String> = s.locales().into_iter().map(|l| l.0).collect();
        assert_eq!(tags, vec!["en", "zh-Hans", "zh-Hant"]);
        assert!(s.original("ui:dialogue.name_sep").is_none());
        let es: Vec<String> = stringdb_entries(&d).into_iter().map(|e| e.key).collect();
        assert_eq!(es, vec!["battle:1006", "battle:1007", "name:1350"]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn missing_game_folder_is_graceful() {
        let s = LegacySource::new(std::env::temp_dir().join("newpal-no-such-game-dir"));
        assert!(s.original("battle:1006").is_none() && s.locales().is_empty());
        assert!(stringdb_entries(Path::new("/definitely/not/here")).is_empty());
    }

    #[test]
    fn binary_junk_is_not_a_speaker() {
        assert!(plausible_name("A"));
        assert!(!plausible_name("a\0b") && !plausible_name("a\u{fffd}") && !plausible_name("a\nb") && !plausible_name(&"x".repeat(40)));
    }
}
