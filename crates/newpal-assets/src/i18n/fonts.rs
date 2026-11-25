//! Font stack for a locale.
//!
//! The original game ships `Resource/CT.ttf` (Traditional Chinese TrueType; dialogue is already rendered with
//! TrueType, not with a bitmap font). Other languages need glyphs CT.ttf may not have, so the renderer takes a
//! *stack*: for every character the first font that has a glyph is used.
//!
//! Order: language pack `fonts` -> mod `fonts/dialog.ttf` -> (zh-Hant: game CT.ttf) -> system fonts for the
//! script of the locale -> game CT.ttf (CJK glyph fallback) -> generic CJK system fonts. At most
//! [`MAX_FONTS`] fonts are loaded. Fonts are never copied anywhere; the game font is read in place.
//!
//! TODO: bundle one permissively licensed open font (e.g. Noto Sans) so Latin text looks the same on every
//! machine; per-glyph shaping (ligatures, RTL, Indic) needs a shaper such as `rustybuzz`.
use crate::vfs::Vfs;
use crate::{Error, Result};
use std::path::{Path, PathBuf};

pub const MAX_FONTS: usize = 5;

/// A font named by a language pack.
#[derive(Clone, Debug, PartialEq)]
pub enum FontEntry {
    File(PathBuf),
    /// file name looked up in the system font folders (`system:segoeui.ttf`)
    System(String),
}

impl FontEntry {
    pub fn resolve(spec: &str, pack_dir: Option<&Path>) -> Self {
        if let Some(n) = spec.strip_prefix("system:") {
            return Self::System(n.trim().to_string());
        }
        let p = PathBuf::from(spec);
        if p.is_absolute() {
            Self::File(p)
        } else {
            Self::File(pack_dir.map(|d| d.join(&p)).unwrap_or(p))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptClass {
    Hans,
    Hant,
    Ja,
    Ko,
    Latin,
}

pub fn script_class(tag: &str) -> ScriptClass {
    let t = super::canonical(tag);
    let lang = t.split('-').next().unwrap_or("");
    match (lang, t.as_str()) {
        ("zh", t) if t.contains("Hant") => ScriptClass::Hant,
        ("zh", _) => ScriptClass::Hans,
        ("ja", _) => ScriptClass::Ja,
        ("ko", _) => ScriptClass::Ko,
        _ => ScriptClass::Latin,
    }
}

/// System font file names to try, per script (first existing wins; all are kept up to the limit).
pub fn system_candidates(c: ScriptClass) -> &'static [&'static str] {
    match c {
        ScriptClass::Hans => &["msyh.ttc", "msyh.ttf", "simsun.ttc", "simhei.ttf", "NotoSansCJK-Regular.ttc", "NotoSansSC-Regular.otf"],
        ScriptClass::Hant => &["msjh.ttc", "mingliu.ttc", "NotoSansCJK-Regular.ttc", "NotoSansTC-Regular.otf"],
        ScriptClass::Ja => &["YuGothR.ttc", "meiryo.ttc", "msgothic.ttc", "NotoSansCJK-Regular.ttc"],
        ScriptClass::Ko => &["malgun.ttf", "NotoSansCJK-Regular.ttc"],
        ScriptClass::Latin => &["segoeui.ttf", "arial.ttf", "calibri.ttf", "NotoSans-Regular.ttf", "DejaVuSans.ttf"],
    }
}

fn system_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(w) = std::env::var_os("WINDIR") {
        v.push(PathBuf::from(w).join("Fonts"));
    }
    v.push(PathBuf::from(r"C:\Windows\Fonts"));
    for d in ["/usr/share/fonts/opentype/noto", "/usr/share/fonts/truetype/noto", "/usr/share/fonts/truetype/dejavu", "/usr/share/fonts/truetype/liberation", "/System/Library/Fonts", "/Library/Fonts"] {
        v.push(PathBuf::from(d));
    }
    v
}

fn find_system(name: &str) -> Option<PathBuf> {
    system_dirs().into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

impl Vfs {
    /// Font stack for the current locale of this VFS (see the module docs). `(bytes, description)` each.
    pub fn font_stack(&self) -> Result<Vec<(Vec<u8>, String)>> {
        let i18n = self.i18n();
        let class = script_class(&i18n.locale());
        let mut paths: Vec<(PathBuf, String)> = Vec::new();
        let mut bytes: Vec<(Vec<u8>, String)> = Vec::new();
        let add_path = |p: PathBuf, why: &str, paths: &mut Vec<(PathBuf, String)>| {
            if p.is_file() && !paths.iter().any(|(q, _)| *q == p) {
                paths.push((p, why.to_string()));
            }
        };
        for e in i18n.font_entries() {
            match e {
                FontEntry::File(p) => add_path(p, "language pack", &mut paths),
                FontEntry::System(n) => {
                    if let Some(p) = find_system(&n) {
                        add_path(p, "language pack (system)", &mut paths)
                    }
                }
            }
        }
        if let Some((p, n, _)) = self.find_override("fonts/dialog.ttf") {
            add_path(p, &format!("mod {n}"), &mut paths);
        }
        let ct = |bytes: &mut Vec<(Vec<u8>, String)>, this: &Vfs| {
            if bytes.iter().any(|(_, d)| d.contains("CT.ttf")) {
                return;
            }
            if let Ok(b) = this.read_raw("Resource/CT.ttf") {
                bytes.push((b, "game Resource/CT.ttf".into()));
            }
        };
        // files so far
        for (p, why) in &paths {
            if let Ok(b) = std::fs::read(p) {
                bytes.push((b, format!("{why}: {}", p.display())));
            }
        }
        if class == ScriptClass::Hant {
            ct(&mut bytes, self);
        }
        let mut sys: Vec<PathBuf> = Vec::new();
        for n in system_candidates(class) {
            if let Some(p) = find_system(n) {
                sys.push(p);
            }
        }
        for p in sys.into_iter().filter(|p| !paths.iter().any(|(q, _)| q == p)).take(2) {
            if let Ok(b) = std::fs::read(&p) {
                bytes.push((b, format!("system: {}", p.display())));
            }
        }
        ct(&mut bytes, self);
        if class != ScriptClass::Hant {
            for n in system_candidates(ScriptClass::Hans).iter().chain(system_candidates(ScriptClass::Hant)) {
                if bytes.len() >= MAX_FONTS {
                    break;
                }
                if let Some(p) = find_system(n) {
                    if !bytes.iter().any(|(_, d)| d.ends_with(&p.display().to_string())) {
                        if let Ok(b) = std::fs::read(&p) {
                            bytes.push((b, format!("system: {}", p.display())));
                            break; // one generic CJK fallback is enough
                        }
                    }
                }
            }
        }
        bytes.truncate(MAX_FONTS);
        if bytes.is_empty() {
            return Err(Error::not_found("no usable font (set `fonts` in the language pack or install a CJK font)"));
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_and_entries() {
        assert_eq!(script_class("zh-Hant"), ScriptClass::Hant);
        assert_eq!(script_class("zh_TW"), ScriptClass::Hant);
        assert_eq!(script_class("zh-CN"), ScriptClass::Hans);
        assert_eq!(script_class("en-GB"), ScriptClass::Latin);
        assert_eq!(FontEntry::resolve("system:arial.ttf", None), FontEntry::System("arial.ttf".into()));
        assert_eq!(FontEntry::resolve("f/a.ttf", Some(Path::new("/p"))), FontEntry::File(PathBuf::from("/p").join("f/a.ttf")));
    }
}
