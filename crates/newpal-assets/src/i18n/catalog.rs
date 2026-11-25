//! Language pack files: `lang/<locale>/*.toml`.
//!
//! ```toml
//! [meta]                        # optional; usually in lang.toml
//! name = "English"
//! inherits = "en"               # optional parent locale (default: the tag minus its last subtag)
//! fonts = ["fonts/Body.ttf", "system:segoeui.ttf"]   # tried in order, per-glyph fallback
//! wrap = "word"                 # "char" (CJK) | "word"
//! convert = "t2s.tsv"           # optional char-map applied to *untranslated* original text
//! markup = "1"                  # string syntax version (inline markup v1, see `i18n::markup`); packs written for
//!                               # the old `%C6` / `%N` codes omit it and are upgraded on load (with a warning)
//!
//! [strings]                     # empty value = not translated yet (falls back)
//! "talk:104:0012" = "<speaker>Li</speaker>Hello<br/>there"
//! "ui:dialogue.name_sep" = ": "
//! ```
//! Several `*.toml` files per locale are merged in file-name order (later wins).
use super::wrap::WrapMode;
use crate::{Error, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct LangMeta {
    pub locale: Option<String>,
    pub name: Option<String>,
    pub inherits: Option<String>,
    pub fonts: Vec<String>,
    pub wrap: Option<WrapMode>,
    pub convert: Option<String>,
    pub markup: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Pack {
    pub dir: PathBuf,
    pub meta: LangMeta,
    pub strings: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

/// Parse one pack file's text into (meta, strings) and collect warnings.
pub fn parse_file(text: &str, origin: &str, meta: &mut LangMeta, strings: &mut BTreeMap<String, String>, warnings: &mut Vec<String>) {
    let table: toml::Table = match text.trim_start_matches('\u{feff}').parse() {
        Ok(t) => t,
        Err(e) => {
            warnings.push(format!("{origin}: {e}"));
            return;
        }
    };
    for (k, v) in table {
        match (k.as_str(), v) {
            ("meta", toml::Value::Table(t)) => match toml::Value::Table(t).try_into::<LangMeta>() {
                Ok(m) => merge_meta(meta, m),
                Err(e) => warnings.push(format!("{origin}: [meta]: {e}")),
            },
            ("strings", toml::Value::Table(t)) => {
                for (k, v) in t {
                    put(strings, warnings, origin, k, v);
                }
            }
            (_, v) => put(strings, warnings, origin, k, v),
        }
    }
}

fn put(strings: &mut BTreeMap<String, String>, warnings: &mut Vec<String>, origin: &str, k: String, v: toml::Value) {
    match v {
        toml::Value::String(s) => {
            if !s.is_empty() {
                strings.insert(k, s);
            }
        }
        other => warnings.push(format!("{origin}: key {k:?} is not a string ({})", other.type_str())),
    }
}

fn merge_meta(into: &mut LangMeta, m: LangMeta) {
    macro_rules! take {
        ($f:ident) => {
            if m.$f.is_some() {
                into.$f = m.$f;
            }
        };
    }
    take!(locale);
    take!(name);
    take!(inherits);
    take!(wrap);
    take!(convert);
    take!(markup);
    if !m.fonts.is_empty() {
        into.fonts = m.fonts;
    }
}

/// Load every `*.toml` in a pack directory.
pub fn load_pack_dir(dir: &Path) -> Result<Pack> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| Error::io(e, dir.display().to_string()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml")) && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .collect();
    files.sort();
    let (mut meta, mut strings, mut warnings) = (LangMeta::default(), BTreeMap::new(), Vec::new());
    for f in files {
        let text = std::fs::read_to_string(&f).map_err(|e| Error::io(e, f.display().to_string()))?;
        parse_file(&text, &f.display().to_string(), &mut meta, &mut strings, &mut warnings);
    }
    // packs written for the old control codes (`%C6`, `%N`, `Name:%N`) are upgraded to markup v1
    if meta.markup.as_deref().map(str::trim) != Some("1") {
        let mut n = 0usize;
        for v in strings.values_mut() {
            if crate::legacy::markup::looks_legacy(v) {
                *v = crate::legacy::markup::from_legacy(v, true);
                n += 1;
            }
        }
        if n > 0 {
            warnings.push(format!("{}: {n} string(s) use the old %C/%N control codes and were upgraded to markup v1; run `newpal-i18n migrate` and set markup = \"1\"", dir.display()));
        }
    }
    Ok(Pack { dir: dir.to_path_buf(), meta, strings, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_meta_strings_and_skips_empty() {
        let (mut m, mut s, mut w) = (LangMeta::default(), BTreeMap::new(), Vec::new());
        parse_file(
            "[meta]\nname = \"X\"\nwrap = \"word\"\nfonts = [\"a.ttf\"]\n[strings]\n\"talk:1:0000\" = \"hi\"\n\"talk:1:0002\" = \"\"\n\"bad\" = 3\n",
            "t",
            &mut m,
            &mut s,
            &mut w,
        );
        assert_eq!(m.name.as_deref(), Some("X"));
        assert_eq!(m.wrap, Some(WrapMode::Word));
        assert_eq!(s.len(), 1);
        assert_eq!(s["talk:1:0000"], "hi");
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn broken_toml_is_a_warning_not_a_panic() {
        let (mut m, mut s, mut w) = (LangMeta::default(), BTreeMap::new(), Vec::new());
        parse_file("[strings\nx", "t", &mut m, &mut s, &mut w);
        assert!(s.is_empty());
        assert_eq!(w.len(), 1);
    }
}
