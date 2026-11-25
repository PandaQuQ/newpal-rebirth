//! Collect every translatable string from the player's own install, and write translation templates.
//!
//! The output is **local only**: it may contain original text (`with_source`), so [`write_template`] refuses to
//! write inside the game folder and drops a `.gitignore` (`*`) next to source-bearing output. Never commit it.
use crate::vfs::Vfs;
use crate::{Error, Result};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    /// original text (empty for engine UI keys: see `default`)
    pub text: String,
    /// short note for translators
    pub note: String,
    /// output file group: `talk_1` .. `talk_8`, `names`, `tables`, `ui`
    pub group: String,
}

/// Everything translatable: the original install's text (see [`crate::legacy::source::original_entries`]) plus
/// the engine's own UI strings.
pub fn collect(vfs: &Vfs) -> Vec<Entry> {
    let mut out = crate::legacy::source::original_entries(vfs);
    for (k, t) in super::builtin::keys() {
        out.push(Entry { key: k, text: t, note: "engine UI (English default shown)".into(), group: "ui".into() });
    }
    out
}

fn comment_line(s: &str) -> String {
    s.replace('\r', "").replace('\n', "\\n")
}

/// TOML basic-string key.
fn toml_key(k: &str) -> String {
    format!("\"{}\"", k.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Write `<out>/<locale>/{lang.toml, <group>.toml...}`. Values are empty (= untranslated). With `with_source`
/// the original text is put in a comment above each key.
pub fn write_template(entries: &[Entry], out: &Path, locale: &str, with_source: bool, game_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(g) = game_dir {
        let (a, b) = (std::fs::canonicalize(out).unwrap_or_else(|_| out.to_path_buf()), std::fs::canonicalize(g).unwrap_or_else(|_| g.to_path_buf()));
        if a.starts_with(&b) {
            return Err(Error::Config(format!("refusing to write inside the game folder ({})", g.display())));
        }
    }
    let dir = out.join(super::canonical(locale));
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(e, dir.display().to_string()))?;
    if with_source {
        let gi = out.join(".gitignore");
        if !gi.exists() {
            let _ = std::fs::write(&gi, "# contains text extracted from the original game: never commit\n*\n");
        }
    }
    let loc = super::canonical(locale);
    let head = format!(
        "[meta]\nname = \"{loc}\"\n# inherits = \"en\"\n# fonts = [\"fonts/MyFont.ttf\", \"system:segoeui.ttf\"]\n# wrap = \"word\"      # \"char\" for CJK\n# convert = \"t2s.tsv\" # char map applied to untranslated original text\nmarkup = \"1\"\n"
    );
    std::fs::write(dir.join("lang.toml"), head).map_err(|e| Error::io(e, dir.display().to_string()))?;
    let mut groups: BTreeMap<&str, String> = BTreeMap::new();
    for e in entries {
        let s = groups.entry(e.group.as_str()).or_insert_with(|| "[strings]\n# empty value = not translated yet (falls back). Keep the <tags> and {placeholders} of the source text.\n".to_string());
        if !e.note.is_empty() {
            let _ = writeln!(s, "# {}", comment_line(&e.note));
        }
        if with_source && !e.text.is_empty() {
            let _ = writeln!(s, "#> {}", comment_line(&e.text));
        }
        let _ = writeln!(s, "{} = \"\"", toml_key(&e.key));
    }
    for (g, body) in groups {
        let p = dir.join(format!("{g}.toml"));
        std::fs::write(&p, body).map_err(|e| Error::io(e, p.display().to_string()))?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::key;

    #[test]
    fn template_roundtrips_through_the_loader_and_is_empty() {
        let d = std::env::temp_dir().join(format!("newpal-i18n-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let es = vec![
            Entry { key: key::talk(1, 2), text: "x\ny".into(), note: "op 0x1".into(), group: "talk_1".into() },
            Entry { key: key::ui("a"), text: String::new(), note: String::new(), group: "ui".into() },
        ];
        let dir = write_template(&es, &d, "en", true, None).unwrap();
        let p = super::super::catalog::load_pack_dir(&dir).unwrap();
        assert!(p.strings.is_empty(), "templates carry no translations");
        assert!(p.warnings.is_empty());
        let t = std::fs::read_to_string(dir.join("talk_1.toml")).unwrap();
        assert!(t.contains("#> x\\ny") && t.contains("\"talk:1:0002\" = \"\""));
        assert!(d.join(".gitignore").exists());
        // refuses the game folder
        assert!(write_template(&es, &d, "en", true, Some(&d)).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
