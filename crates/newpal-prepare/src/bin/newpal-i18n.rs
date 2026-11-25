//! `newpal-i18n`: translation tooling. Reads the player's own original install (read-only) and writes local
//! templates / reports. Output may contain original text: keep it out of git (the tool adds a `.gitignore`).
//!
//! ```text
//! newpal-i18n export  --out <dir> [--locale en] [--with-source] [--game-dir <p>] [--only talk,names,tables,ui,battle,menu,shop,msg,sys,err]
//! newpal-i18n check   --lang-dir <dir> --locale en [--game-dir <p>]
//! newpal-i18n ui-list --locale <tag> [--cat battle] [--from <id>] [--count <n>] [--lang-dir <dir>] [--game-dir <p>]
//! newpal-i18n info    [--game-dir <p>]
//! newpal-i18n migrate --from <dir> --out <dir>
//! ```
use newpal_assets::gamedir::{self, Config};
use newpal_assets::i18n::{self, catalog, export, markup};
use newpal_assets::legacy::{self, source, stringdb};
use newpal_assets::vfs::Vfs;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const HELP: &str = "newpal-i18n - translation tooling for your own New PAL install (read-only)

  export --out <dir> [--locale <tag>] [--with-source] [--only talk,names,tables,ui,battle,menu,shop,msg,sys,err] [--game-dir <path>]
        writes <dir>/<tag>/lang.toml + talk_1..8.toml, names.toml, tables.toml, ui.toml, battle.toml, menu.toml,
        shop.toml ... with EMPTY values (empty = untranslated). Covers ALL dialogue (also the code that is only
        reachable by jumps), names, tables and the game's own string table (menus, battle, shop, messages).
        --with-source adds the original text as `#>` comments: LOCAL USE ONLY, never commit it (a .gitignore is
        written). Refuses to write inside the game folder.
  check  --lang-dir <dir> --locale <tag> [--game-dir <path>]
        coverage per group, unknown keys, and markup / placeholder mismatches of a language pack.
  ui-list --locale <tag> [--cat <category>] [--from <id>] [--count <n>] [--lang-dir <dir>] [--game-dir <path>]
        the game's own strings of a category as the engine would show them in <tag>, with where each came from.
  info   [--game-dir <path>]
        game build identification (exe SHA-256), shipped string tables, dialogue / key counts, markup usage.
  migrate --from <dir> --out <dir>
        rewrites language packs (*.toml) and mod text overrides (*.json) written for the old %C / %N control codes
        to inline markup v1 (and adds markup = \"1\" to the pack's [meta]).
";

fn arg(args: &[String], k: &str) -> Option<String> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned()
}

fn game_vfs(args: &[String]) -> Result<(Vfs, PathBuf), String> {
    let p = Config::default_path();
    let cfg = Config::load_from(&p).unwrap_or_default();
    let cli = arg(args, "--game-dir").map(PathBuf::from);
    let r = gamedir::resolve(cli.as_deref(), &cfg, &p).map_err(|e| e.to_string())?;
    Ok((Vfs::new(r.dir.clone(), Vec::new()), r.dir))
}

fn group_of(key: &str) -> &str {
    key.split(':').next().unwrap_or("?")
}

fn find_pack_dir(lang_dir: &Path, locale: &str) -> Result<PathBuf, String> {
    std::fs::read_dir(lang_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.is_dir() && p.file_name().is_some_and(|n| i18n::canonical(&n.to_string_lossy()) == locale))
        .ok_or_else(|| format!("no <lang-dir>/{locale} folder"))
}

// ---- migrate ----------------------------------------------------------------------------------------------

/// Decode a one-line TOML basic / literal string.
fn toml_string(rhs: &str) -> Option<String> {
    let r = rhs.trim();
    if r.starts_with("\"\"\"") || r.starts_with("'''") {
        return None;
    }
    if let Some(l) = r.strip_prefix('\'') {
        return l.strip_suffix('\'').map(str::to_string);
    }
    let body = r.strip_prefix('"')?.strip_suffix('"')?;
    let cs: Vec<char> = body.chars().collect();
    let mut o = String::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] != '\\' {
            o.push(cs[i]);
            i += 1;
            continue;
        }
        i += 1;
        match cs.get(i)? {
            'n' => o.push('\n'),
            't' => o.push('\t'),
            'r' => o.push('\r'),
            '"' => o.push('"'),
            '\\' => o.push('\\'),
            'u' | 'U' => {
                let len = if cs[i] == 'u' { 4 } else { 8 };
                let hex: String = cs.get(i + 1..i + 1 + len)?.iter().collect();
                o.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                i += len;
            }
            _ => return None,
        }
        i += 1;
    }
    Some(o)
}

fn toml_quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Rewrite one pack file; returns (new text, converted strings, strings left alone).
fn migrate_toml(text: &str) -> (String, usize, usize) {
    let (mut out, mut n, mut skipped) = (String::new(), 0, 0);
    for line in text.split_inclusive('\n') {
        let t = line.trim_start().trim_end_matches(['\r', '\n']);
        let eol = &line[line.trim_end_matches(['\r', '\n']).len()..];
        let body = line.trim_end_matches(['\r', '\n']);
        if t.starts_with('"') {
            // key may contain escaped quotes
            let kb = t.as_bytes();
            let mut j = 1;
            while j < kb.len() && kb[j] != b'"' {
                j += if kb[j] == b'\\' { 2 } else { 1 };
            }
            if let Some(rest) = t.get(j + 1..).and_then(|r| r.trim_start().strip_prefix('=')) {
                let key_end = body.len() - rest.len();
                match toml_string(rest) {
                    Some(v) if legacy::markup::looks_legacy(&v) => {
                        out.push_str(&body[..key_end]);
                        out.push(' ');
                        out.push_str(&toml_quote(&legacy::markup::from_legacy(&v, true)));
                        out.push_str(eol);
                        n += 1;
                        continue;
                    }
                    None if rest.trim().starts_with("\"\"\"") => skipped += 1,
                    _ => {}
                }
            }
        }
        out.push_str(line);
    }
    (out, n, skipped)
}

fn ensure_markup_meta(text: &str) -> String {
    if text.lines().any(|l| l.trim_start().starts_with("markup")) {
        return text.to_string();
    }
    match text.find("[meta]") {
        Some(p) => {
            let eol = text[p..].find('\n').map(|e| p + e + 1).unwrap_or(text.len());
            let nl = if eol == text.len() && !text.ends_with('\n') { "\n" } else { "" };
            format!("{}{nl}markup = \"1\"\n{}", &text[..eol], &text[eol..])
        }
        None => format!("[meta]\nmarkup = \"1\"\n\n{text}"),
    }
}

fn migrate_dir(from: &Path, to: &Path, stats: &mut (usize, usize, usize)) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let mut has_meta_file = false;
    let mut is_pack = false;
    for e in std::fs::read_dir(from).map_err(|e| e.to_string())?.filter_map(|e| e.ok()) {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            migrate_dir(&p, &to.join(&name), stats)?;
        } else if name.ends_with(".toml") {
            is_pack = true;
            let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            let (mut new, n, sk) = migrate_toml(&text);
            if name == "lang.toml" {
                has_meta_file = true;
                new = ensure_markup_meta(&new);
            }
            stats.0 += n;
            stats.1 += sk;
            std::fs::write(to.join(&name), new).map_err(|e| e.to_string())?;
        } else if name.ends_with(".json") {
            let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            let mut m: BTreeMap<String, String> = match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(_) => {
                    std::fs::copy(&p, to.join(&name)).map_err(|e| e.to_string())?;
                    continue;
                }
            };
            for v in m.values_mut() {
                if legacy::markup::looks_legacy(v) {
                    *v = legacy::markup::from_legacy(v, true);
                    stats.2 += 1;
                }
            }
            std::fs::write(to.join(&name), serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        } else {
            std::fs::copy(&p, to.join(&name)).map_err(|e| e.to_string())?;
        }
    }
    if is_pack && !has_meta_file {
        std::fs::write(to.join("lang.toml"), "[meta]\nmarkup = \"1\"\n").map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---- commands ---------------------------------------------------------------------------------------------

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("export") => {
            let out: PathBuf = arg(&args, "--out").ok_or("--out <dir> is required")?.into();
            let locale = arg(&args, "--locale").unwrap_or_else(|| "en".into());
            let with_source = args.iter().any(|a| a == "--with-source");
            let only: Option<Vec<String>> = arg(&args, "--only").map(|s| s.split(',').map(|x| x.trim().to_string()).collect());
            let (vfs, game) = game_vfs(&args)?;
            let mut entries = export::collect(&vfs);
            if let Some(o) = &only {
                entries.retain(|e| {
                    let g = if e.group.starts_with("talk_") { "talk" } else { e.group.as_str() };
                    o.iter().any(|x| x == g)
                });
            }
            let dir = export::write_template(&entries, &out, &locale, with_source, Some(&game)).map_err(|e| e.to_string())?;
            let mut by: BTreeMap<&str, usize> = BTreeMap::new();
            for e in &entries {
                *by.entry(group_of(&e.key)).or_default() += 1;
            }
            println!("wrote {} keys to {} {:?}", entries.len(), dir.display(), by);
            let st = source::dialogue_stats(&vfs);
            println!("dialogue: {} scripts, {} lines (old linear sweep {}, +{} only reachable by jumps)", st.scripts, st.full, st.linear, st.full - st.linear);
            if with_source {
                println!("note: the files contain original text; they are local only (.gitignore written)");
            }
            Ok(())
        }
        Some("check") => {
            let lang_dir: PathBuf = arg(&args, "--lang-dir").ok_or("--lang-dir <dir> is required")?.into();
            let locale = i18n::canonical(&arg(&args, "--locale").ok_or("--locale <tag> is required")?);
            let dir = find_pack_dir(&lang_dir, &locale)?;
            let pack = catalog::load_pack_dir(&dir).map_err(|e| e.to_string())?;
            for w in &pack.warnings {
                println!("warning: {w}");
            }
            let (vfs, _) = game_vfs(&args)?;
            let entries = export::collect(&vfs);
            let known: BTreeMap<&str, &export::Entry> = entries.iter().map(|e| (e.key.as_str(), e)).collect();
            let mut total: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
            for e in &entries {
                let t = total.entry(group_of(&e.key)).or_default();
                t.0 += 1;
                if pack.strings.contains_key(&e.key) {
                    t.1 += 1;
                }
            }
            println!("pack {} ({} strings)", dir.display(), pack.strings.len());
            for (g, (n, done)) in &total {
                println!("  {g:<8} {done}/{n} ({:.1}%)", 100.0 * *done as f64 / (*n).max(1) as f64);
            }
            let (mut unknown, mut mismatch, mut bad_markup) = (0, 0, 0);
            for (k, v) in &pack.strings {
                // SDL key labels are valid keys but not exported
                let is_key_label = stringdb::parse_key(k).is_some_and(|id| !stringdb::translatable(id));
                match known.get(k.as_str()) {
                    None if is_key_label => {}
                    None => {
                        unknown += 1;
                        if unknown <= 20 {
                            println!("unknown key: {k}");
                        }
                    }
                    Some(e) => {
                        let p = markup::parse(&i18n::message::format(v, &[], &locale));
                        if !p.warnings.is_empty() {
                            bad_markup += 1;
                            if bad_markup <= 20 {
                                println!("markup problem at {k}: {}", p.warnings.join("; "));
                            }
                        }
                        let (a, b) = (markup::signature(&e.text), markup::signature(v));
                        if k.starts_with("talk:") || stringdb::parse_key(k).is_some() || k.starts_with("ui:") {
                            // `<speaker>` may legitimately be dropped / added by a translation of a speakerless line
                            let strip = |s: Vec<String>| s.into_iter().filter(|t| !t.contains("speaker")).collect::<Vec<_>>();
                            if strip(a.clone()) != strip(b.clone()) {
                                mismatch += 1;
                                if mismatch <= 20 {
                                    println!("markup / placeholders differ at {k}: original {a:?} vs translation {b:?}");
                                }
                            }
                        }
                    }
                }
            }
            println!("unknown keys: {unknown}, markup mismatches: {mismatch}, markup problems: {bad_markup}");
            if unknown + mismatch + bad_markup > 0 {
                std::process::exit(1);
            }
            Ok(())
        }
        Some("ui-list") => {
            let locale = i18n::canonical(&arg(&args, "--locale").unwrap_or_else(|| i18n::ORIGINAL_LOCALE.into()));
            let cat = arg(&args, "--cat");
            let from: u32 = arg(&args, "--from").and_then(|v| v.parse().ok()).unwrap_or(0);
            let count: usize = arg(&args, "--count").and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
            let (vfs, _) = game_vfs(&args)?;
            let mut dirs: Vec<PathBuf> = arg(&args, "--lang-dir").map(PathBuf::from).into_iter().collect();
            dirs.push(gamedir::user_config_dir().join("lang"));
            let i = i18n::I18n::new(dirs, Vec::new());
            i.set_source(std::sync::Arc::new(source::LegacySource::new(vfs.game_dir().to_path_buf())));
            i.set_quiet(true);
            i.set_locale(&locale)?;
            let mut shown = 0usize;
            let mut by_source: BTreeMap<String, usize> = BTreeMap::new();
            let src = source::LegacySource::new(vfs.game_dir().to_path_buf());
            let db = src.db(i18n::ORIGINAL_LOCALE).ok_or("the game folder has no zh-Hant/StringDB.txt")?;
            for id in db.entries.keys().copied().filter(|id| *id >= from) {
                let k = stringdb::key(id);
                if cat.as_deref().is_some_and(|c| c != stringdb::category(id)) {
                    continue;
                }
                if shown >= count {
                    break;
                }
                let r = i.resolve(&k, None).ok_or("unresolved key")?;
                let tag = match &r.source {
                    i18n::Source::Pack(_) => "pack",
                    i18n::Source::Builtin(_) => "builtin",
                    i18n::Source::Shipped(_) => "shipped",
                    i18n::Source::Converted => "converted",
                    i18n::Source::Original => "original",
                };
                *by_source.entry(tag.into()).or_default() += 1;
                println!("{k:<14} [{tag:<9}] {}", r.text.replace('\n', "\\n"));
                shown += 1;
            }
            println!("-- {shown} string(s), locale {locale}, sources {by_source:?}");
            Ok(())
        }
        Some("info") => {
            let (vfs, game) = game_vfs(&args)?;
            let src = source::LegacySource::new(game.clone());
            let b = src.build();
            println!("game folder: {}", game.display());
            println!("exe: {} sha256 {}", b.exe.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "-".into()), b.exe_sha256.as_deref().unwrap_or("-"));
            println!("build: {}", b.known.map(|k| k.label).unwrap_or("UNKNOWN (text is read as is)"));
            for n in &b.notes {
                println!("note: {n}");
            }
            let tr = |s: &dyn i18n::TextSource| s.locales();
            for (tag, name) in tr(&src) {
                let n = src.db(&tag).map(|d| (d.entries.len(), d.warnings.len())).unwrap_or((0, 0));
                println!("string table {tag}: {} entries, {} parse warning(s) ({name})", n.0, n.1);
            }
            let mut cats: BTreeMap<String, usize> = BTreeMap::new();
            for e in source::stringdb_entries(&game) {
                *cats.entry(group_of(&e.key).to_string()).or_default() += 1;
            }
            println!("exported string-table keys by category: {cats:?}");
            let st = source::dialogue_stats(&vfs);
            println!("dialogue: {} scripts, {} lines; old linear sweep found {}, jump-only +{}", st.scripts, st.full, st.linear, st.full - st.linear);
            for (f, (l, full)) in &st.per_file {
                println!("  talk{f}.dat: linear {l}, full {full} (+{})", full - l);
            }
            println!("markup tags in dialogue: {:?}", source::tag_histogram(&vfs));
            Ok(())
        }
        Some("migrate") => {
            let from: PathBuf = arg(&args, "--from").ok_or("--from <dir> is required")?.into();
            let to: PathBuf = arg(&args, "--out").ok_or("--out <dir> is required")?.into();
            if std::fs::canonicalize(&from).ok() == std::fs::canonicalize(&to).ok() {
                return Err("--out must be a different folder than --from".into());
            }
            let mut stats = (0usize, 0usize, 0usize);
            migrate_dir(&from, &to, &mut stats)?;
            println!("migrated {} pack string(s) and {} mod override(s) to markup v1 -> {} ({} multi-line string(s) left as is)", stats.0, stats.2, to.display(), stats.1);
            Ok(())
        }
        _ => {
            println!("{HELP}");
            Ok(())
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_string_roundtrip() {
        assert_eq!(toml_string(r#""a\"b\n\u00e9""#).as_deref(), Some("a\"b\n\u{e9}"));
        assert_eq!(toml_string("'lit\\'").as_deref(), Some("lit\\"));
        assert_eq!(toml_string("\"\"\"ml\"\"\""), None);
        assert_eq!(toml_string(&toml_quote("x\"y\\z\n")).as_deref(), Some("x\"y\\z\n"));
    }

    #[test]
    fn migrates_only_legacy_values_and_keeps_comments() {
        let src = "[strings]\n# note %C6\n\"talk:1:0000\" = \"Li: %N%C6Hi%C0\"\n\"ui:x\" = \"plain 100% sure\"\n\"talk:1:0002\" = \"\"\n";
        let (out, n, _) = migrate_toml(src);
        assert_eq!(n, 1);
        assert!(out.contains("# note %C6\n"));
        assert!(out.contains("\"talk:1:0000\" = \"<speaker>Li</speaker><color=6>Hi</color>\"\n"), "{out}");
        assert!(out.contains("\"ui:x\" = \"plain 100% sure\""));
        assert!(out.contains("\"talk:1:0002\" = \"\""));
    }

    #[test]
    fn meta_gets_markup_version() {
        assert_eq!(ensure_markup_meta("[meta]\nname = \"E\"\n"), "[meta]\nmarkup = \"1\"\nname = \"E\"\n");
        assert_eq!(ensure_markup_meta("[strings]\n"), "[meta]\nmarkup = \"1\"\n\n[strings]\n");
        assert_eq!(ensure_markup_meta("[meta]\nmarkup = \"1\"\n"), "[meta]\nmarkup = \"1\"\n");
    }
}
