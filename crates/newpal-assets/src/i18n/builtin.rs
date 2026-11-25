//! Engine-owned UI strings (`ui:*`) shipped inside the binary as data files (`builtin/<locale>.toml`): our own
//! words, never original text. They sit *below* language packs and mods in the lookup order and above the
//! player's own `StringDB.txt`. Adding an engine string = add the key to `en.toml` and `zh-Hant.toml`.
use super::catalog;
use std::collections::BTreeMap;
use std::sync::OnceLock;

const FILES: &[(&str, &str)] = &[("en", include_str!("builtin/en.toml")), ("zh-Hant", include_str!("builtin/zh-Hant.toml")), ("zh-Hans", include_str!("builtin/zh-Hans.toml"))];

fn tables() -> &'static Vec<(&'static str, BTreeMap<String, String>)> {
    static T: OnceLock<Vec<(&'static str, BTreeMap<String, String>)>> = OnceLock::new();
    T.get_or_init(|| {
        FILES
            .iter()
            .map(|(tag, src)| {
                let (mut meta, mut strings, mut warnings) = (catalog::LangMeta::default(), BTreeMap::new(), Vec::new());
                catalog::parse_file(src, tag, &mut meta, &mut strings, &mut warnings);
                debug_assert!(warnings.is_empty(), "builtin {tag}: {warnings:?}");
                (*tag, strings)
            })
            .collect()
    })
}

/// Built-in locales.
pub fn locales() -> Vec<&'static str> {
    tables().iter().map(|(t, _)| *t).collect()
}

/// `ui:<name>` of the exact locale `tag` (no fallback).
pub fn get(tag: &str, key: &str) -> Option<&'static str> {
    tables().iter().find(|(t, _)| *t == tag).and_then(|(_, m)| m.get(key)).map(|s| s.as_str())
}

/// Every built-in key with its English text (for export templates), sorted.
pub fn keys() -> Vec<(String, String)> {
    let mut v: BTreeMap<String, String> = BTreeMap::new();
    for (_, m) in tables() {
        for k in m.keys() {
            v.entry(k.clone()).or_default();
        }
    }
    for (_, m) in tables().iter().filter(|(t, _)| *t == "en") {
        for (k, t) in m {
            v.insert(k.clone(), t.clone());
        }
    }
    v.into_iter().collect()
}

/// Text for the engine UI before any [`super::I18n`] exists (e.g. the first-run folder picker).
pub fn text(tag: &str, name: &str) -> String {
    let key = super::key::ui(name);
    let c = super::canonical(tag);
    get(&c, &key).or_else(|| super::parent_tag(&c).and_then(|p| get(&p, &key))).or_else(|| get("en", &key)).unwrap_or(name).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_locale_defines_the_same_keys() {
        let en: Vec<&String> = tables().iter().find(|(t, _)| *t == "en").unwrap().1.keys().collect();
        for (t, m) in tables() {
            assert_eq!(m.keys().collect::<Vec<_>>(), en, "builtin {t} differs from en");
        }
    }

    #[test]
    fn fallbacks() {
        assert_eq!(text("en-GB", "dialogue.name_sep"), ": ");
        assert_eq!(text("zh-Hant", "dialogue.name_sep"), "\u{ff1a}");
        assert_eq!(text("fr", "app.pick_folder"), "Select your original New PAL folder");
        assert_eq!(text("en", "no.such"), "no.such");
    }
}
