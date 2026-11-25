//! Text policy (CI check): original-game text handling is confined to `newpal-assets/src/legacy/`.
//!
//! 1. No Big5 / legacy-codec decoding (`encoding_rs`, `BIG5`, `big5`...) anywhere else, neither in code nor in the
//!    manifests of other crates.
//! 2. The VM / game crates never call the legacy decoders: they only see UTF-8 strings and markup.
//! 3. No hard-coded display text: no CJK characters in the code (strings, chars) of any `src/` or `examples/`
//!    file outside `legacy/` (comments are fine), and no string literal passed to a window title. Display text
//!    lives in language packs and `i18n/builtin/*.toml`.
//!
//! Run: `cargo test -p newpal-assets --test text_policy`.
use std::path::{Path, PathBuf};

/// Source with comments removed (strings and chars kept).
fn strip_comments(src: &str) -> String {
    let cs: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let (mut i, mut block) = (0usize, 0usize);
    let mut in_str = false;
    while i < cs.len() {
        let c = cs[i];
        let n = cs.get(i + 1).copied();
        if block > 0 {
            if c == '*' && n == Some('/') {
                block -= 1;
                i += 2;
            } else if c == '/' && n == Some('*') {
                block += 1;
                i += 2;
            } else {
                if c == '\n' {
                    out.push('\n');
                }
                i += 1;
            }
            continue;
        }
        if in_str {
            out.push(c);
            if c == '\\' {
                if let Some(n) = n {
                    out.push(n);
                }
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match (c, n) {
            ('/', Some('/')) => {
                while i < cs.len() && cs[i] != '\n' {
                    i += 1;
                }
            }
            ('/', Some('*')) => {
                block += 1;
                i += 2;
            }
            ('"', _) => {
                in_str = true;
                out.push(c);
                i += 1;
            }
            // char literal '"' / '\'' must not open a string
            ('\'', Some('"')) if cs.get(i + 2) == Some(&'\'') => {
                out.push_str("'\"'");
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x2e80..=0x9fff | 0xf900..=0xfaff | 0xff00..=0xffef | 0x3000..=0x303f)
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.is_dir() {
            if !matches!(p.file_name().and_then(|n| n.to_str()), Some("target" | "target-i18n" | "target-mt" | ".git")) {
                rs_files(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn rel(p: &Path) -> String {
    p.strip_prefix(crates_dir()).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

const CODEC_WORDS: &[&str] = &["encoding_rs", "BIG5", "Big5", "big5", "GBK", "gb18030", "GB18030", "SHIFT_JIS", "EUC_KR"];

/// (file, finding) over a set of (path, source) pairs; `is_legacy` decides which files are exempt.
fn violations(files: &[(String, String)]) -> Vec<String> {
    let mut v = Vec::new();
    for (path, src) in files {
        let legacy = path.starts_with("newpal-assets/src/legacy/");
        let is_policy_test = path.ends_with("tests/text_policy.rs");
        if legacy || is_policy_test {
            continue;
        }
        let code = strip_comments(src);
        for w in CODEC_WORDS {
            if code.contains(w) {
                v.push(format!("{path}: legacy codec `{w}` outside legacy/"));
            }
        }
        let downstream = path.starts_with("newpal-vm/") || path.starts_with("newpal-game/");
        if downstream && (code.contains("legacy::decode_") || code.contains("decode_cstr") || code.contains("decode_text")) {
            v.push(format!("{path}: VM / game code must not call the legacy text decoders"));
        }
        let shipped_src = (path.contains("/src/") || path.contains("/examples/")) && !path.contains("/tests/");
        if shipped_src {
            if let Some((n, line)) = code.lines().enumerate().find(|(_, l)| l.chars().any(is_cjk)) {
                v.push(format!("{path}:{}: hard-coded CJK text in code: {}", n + 1, line.trim().chars().take(60).collect::<String>()));
            }
            for (n, line) in code.lines().enumerate() {
                let t = line.replace(' ', "");
                if t.contains("title(\"") || t.contains("set_title(\"") {
                    v.push(format!("{path}:{}: string literal as window title (use the `ui:app.*` catalog keys)", n + 1));
                }
            }
        }
    }
    v
}

#[test]
fn comment_stripper_keeps_strings_and_drops_comments() {
    let s = "let a = \"x//y\"; // \u{4e2d}\n/* \u{4e2d} /* nested */ */ let b = '\"'; let c = \"\u{4e2d}\";";
    let c = strip_comments(s);
    assert!(c.contains("\"x//y\""));
    assert_eq!(c.matches('\u{4e2d}').count(), 1, "{c}");
}

#[test]
fn checker_flags_each_rule() {
    let f = |p: &str, s: &str| violations(&[(p.to_string(), s.to_string())]);
    assert!(f("newpal-vm/src/a.rs", "use encoding_rs::BIG5;").len() >= 2);
    assert!(f("newpal-game/src/a.rs", "let s = crate::legacy::decode_text(b);").len() == 1);
    assert!(f("newpal-game/src/a.rs", "let s = \"\u{4e2d}\";").len() == 1);
    assert!(f("newpal-game/src/a.rs", "w.with_title(\"hello\")").len() == 1);
    assert!(f("newpal-game/src/a.rs", "w.with_title(t)").is_empty());
    // allowed places
    assert!(f("newpal-assets/src/legacy/x.rs", "use encoding_rs; let s = \"\u{4e2d}\";").is_empty());
    assert!(f("newpal-game/src/a.rs", "// \u{4e2d} big5 only in a comment\nlet a = 1;").is_empty());
    assert!(f("newpal-assets/tests/golden.rs", "assert_eq!(n, \"\u{4e2d}\");").is_empty());
}

#[test]
fn no_original_text_handling_outside_legacy() {
    let root = crates_dir();
    let mut paths = Vec::new();
    rs_files(&root, &mut paths);
    assert!(paths.len() > 20, "found only {} source files under {}", paths.len(), root.display());
    let files: Vec<(String, String)> = paths.iter().map(|p| (rel(p), std::fs::read_to_string(p).unwrap_or_default())).collect();
    let mut v = violations(&files);
    // manifests: only the assets crate (through legacy/) may depend on a text codec crate
    for e in std::fs::read_dir(&root).unwrap().filter_map(|e| e.ok()) {
        let toml = e.path().join("Cargo.toml");
        let name = e.file_name().to_string_lossy().to_string();
        if name != "newpal-assets" && std::fs::read_to_string(&toml).is_ok_and(|t| t.contains("encoding_rs")) {
            v.push(format!("{name}/Cargo.toml: depends on a text codec crate"));
        }
    }
    assert!(v.is_empty(), "text policy violations:\n{}", v.join("\n"));
}
