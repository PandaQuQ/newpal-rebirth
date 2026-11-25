//! Data-driven text converter (e.g. Traditional -> Simplified Chinese) without any heavy dependency.
//!
//! File format (UTF-8, `#` comments, one rule per line, first whitespace separates source and target):
//! ```text
//! # t2s.tsv
//! \u{9ad4}    \u{4f53}
//! ```
//! Multi-character sources (phrases) are allowed; the longest match wins. A full conversion table (or an
//! OpenCC-compatible converter behind the [`TextConverter`] trait) can be dropped in later without touching
//! the engine.
use crate::{Error, Result};
use std::collections::HashMap;
use std::path::Path;

/// Anything that can rewrite original text for a derived locale.
pub trait TextConverter: Send + Sync {
    fn convert(&self, s: &str) -> String;
}

#[derive(Clone, Debug, Default)]
pub struct CharMap {
    map: HashMap<Vec<char>, String>,
    max_len: usize,
}

impl CharMap {
    pub fn parse(text: &str) -> Self {
        let mut m = CharMap::default();
        for line in text.lines() {
            let line = line.trim_start_matches('\u{feff}').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((from, to)) = line.split_once(char::is_whitespace) else { continue };
            let from: Vec<char> = from.chars().collect();
            if from.is_empty() {
                continue;
            }
            m.max_len = m.max_len.max(from.len());
            m.map.insert(from, to.trim().to_string());
        }
        m
    }

    pub fn load(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).map_err(|e| Error::io(e, path.display().to_string()))?;
        Ok(Self::parse(&s))
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl TextConverter for CharMap {
    fn convert(&self, s: &str) -> String {
        if self.map.is_empty() {
            return s.to_string();
        }
        // only the text between markup tags is rewritten
        super::markup::map_text(s, &|seg| self.convert_plain(seg))
    }
}

impl CharMap {
    fn convert_plain(&self, s: &str) -> String {
        let cs: Vec<char> = s.chars().collect();
        let mut out = String::with_capacity(s.len());
        let mut i = 0;
        'outer: while i < cs.len() {
            for l in (1..=self.max_len.min(cs.len() - i)).rev() {
                if let Some(to) = self.map.get(&cs[i..i + l]) {
                    out.push_str(to);
                    i += l;
                    continue 'outer;
                }
            }
            out.push(cs[i]);
            i += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_match_and_control_codes() {
        let m = CharMap::parse("# c\n\u{9ad4} \u{4f53}\n\u{9ad4}\u{80b2}\tPE\n");
        assert_eq!(m.convert("\u{9ad4}\u{80b2}\u{9ad4}"), "PE\u{4f53}");
        assert_eq!(m.convert("<color=6>\u{9ad4}</color><br/>"), "<color=6>\u{4f53}</color><br/>");
        assert_eq!(CharMap::default().convert("x"), "x");
    }
}

/// `convert = "opencc:t2s"`: pure-Rust OpenCC (`ferrous-opencc`, optional cargo feature `opencc`).
pub fn opencc(name: &str) -> std::result::Result<std::sync::Arc<dyn TextConverter>, String> {
    #[cfg(feature = "opencc")]
    {
        use ferrous_opencc::config::BuiltinConfig;
        let cfg = match name.to_ascii_lowercase().as_str() {
            "t2s" => BuiltinConfig::T2s,
            "tw2s" => BuiltinConfig::Tw2s,
            "tw2sp" => BuiltinConfig::Tw2sp,
            "hk2s" => BuiltinConfig::Hk2s,
            other => return Err(format!("unknown opencc config {other:?} (t2s, tw2s, tw2sp, hk2s)")),
        };
        struct Oc(ferrous_opencc::OpenCC);
        impl TextConverter for Oc {
            fn convert(&self, s: &str) -> String {
                // only the text between markup tags is converted
                super::markup::map_text(s, &|seg| self.0.convert(seg))
            }
        }
        ferrous_opencc::OpenCC::from_config(cfg).map(|o| std::sync::Arc::new(Oc(o)) as std::sync::Arc<dyn TextConverter>).map_err(|e| e.to_string())
    }
    #[cfg(not(feature = "opencc"))]
    {
        let _ = name;
        Err("this build has no OpenCC support (cargo feature `opencc`)".into())
    }
}
