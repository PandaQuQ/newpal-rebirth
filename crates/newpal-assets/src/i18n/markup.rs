//! # Inline markup v1
//!
//! Every translatable string is UTF-8 text with a small, language-neutral inline markup. Language packs only
//! ever contain this; the original game's control codes (`%C6`, `%N`...) are converted by
//! [`crate::legacy::markup`] when the original text is imported.
//!
//! | markup | meaning | original code |
//! |---|---|---|
//! | `<color=N>..</color>` | text colour (palette index 0-9); nests, `</color>` returns to the previous colour | `%C<d>` / `$<d>` |
//! | `<br/>` | line break | `%N` |
//! | `<page/>` | wait for a key, then continue on a fresh page | `%P` |
//! | `<instant/>` | show the rest of the text without the typewriter effect | `%K` |
//! | `<speed=N/>` | typewriter delay level (0-9) | `%S<d>` |
//! | `<glyph=N/>` | glyph draw mode / size selector passed to the text blitter (0-9; meaning unconfirmed) | `%G<d>` |
//! | `<speaker>Name</speaker>` | speaker of a dialogue; shown by the UI as `name` + `ui:dialogue.name_sep` | first line `Name：%N` |
//! | `<ctl=X>` / `<ctl=XN>` | an original control code the engine does not model yet, kept verbatim | `%X` / `%X<d>` |
//!
//! Escapes: `<<` = literal `<`, `{{` / `}}` = literal braces (the latter two belong to [`super::message`]).
//! Placeholders (`{1}`, `{name}`, plurals) are resolved by [`super::message::format`] *before* parsing.
pub use crate::talk::Span;

#[derive(Clone, Debug, PartialEq)]
pub enum CueKind {
    Page,
    Instant,
    Speed(u8),
    Glyph(u8),
    /// original code not modelled yet: letter + optional digit
    Ctl(char, Option<u8>),
}

/// A non-visual instruction at a text position (`col` = characters already placed on `line`).
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub line: usize,
    pub col: usize,
    pub kind: CueKind,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    pub speaker: Option<String>,
    /// styled runs per line (empty lines are dropped)
    pub lines: Vec<Vec<Span>>,
    pub cues: Vec<Cue>,
    pub warnings: Vec<String>,
}

/// `<` -> `<<` and braces doubled: makes arbitrary text safe to embed in a markup/message string.
pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => o.push_str("<<"),
            '{' => o.push_str("{{"),
            '}' => o.push_str("}}"),
            c => o.push(c),
        }
    }
    o
}

/// One lexical item of a markup string.
#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Text(String),
    /// `name`, optional `=value`, self-closing?, closing?
    Tag { name: String, value: Option<String>, close: bool },
}

fn lex(s: &str, warnings: &mut Vec<String>) -> Vec<Tok> {
    let cs: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] != '<' {
            text.push(cs[i]);
            i += 1;
            continue;
        }
        if cs.get(i + 1) == Some(&'<') {
            text.push('<');
            i += 2;
            continue;
        }
        // a tag: <name>, <name=v>, <name/>, <name=v/>, </name>
        let end = cs[i + 1..].iter().position(|&c| c == '>' || c == '<').map(|e| i + 1 + e);
        let parsed = end.filter(|&e| cs[e] == '>').and_then(|e| {
            let body: String = cs[i + 1..e].iter().collect();
            let (close, body) = match body.strip_prefix('/') {
                Some(b) => (true, b.to_string()),
                None => (false, body),
            };
            let body = body.trim_end_matches('/').to_string();
            let (name, value) = match body.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (body, None),
            };
            (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphabetic())).then_some((Tok::Tag { name, value, close }, e))
        });
        match parsed {
            Some((t, e)) => {
                if !text.is_empty() {
                    out.push(Tok::Text(std::mem::take(&mut text)));
                }
                out.push(t);
                i = e + 1;
            }
            None => {
                warnings.push(format!("stray '<' at char {i} (write '<<' for a literal)"));
                text.push('<');
                i += 1;
            }
        }
    }
    if !text.is_empty() {
        out.push(Tok::Text(text));
    }
    out
}

fn digit(v: &Option<String>) -> Option<u8> {
    v.as_deref()?.parse::<u8>().ok().filter(|d| *d <= 9)
}

/// Parse markup into speaker, styled lines and cues. Unknown tags are kept as text with a warning.
pub fn parse(s: &str) -> Parsed {
    let mut p = Parsed::default();
    let toks = lex(s, &mut p.warnings);
    let mut lines: Vec<Vec<Span>> = vec![vec![]];
    let mut colors: Vec<u8> = vec![0];
    let mut speaker: Option<String> = None;
    let mut in_speaker = false;
    let mut col_chars = 0usize;
    for t in toks {
        match t {
            Tok::Text(x) => {
                if in_speaker {
                    speaker.get_or_insert_with(String::new).push_str(&x);
                } else {
                    col_chars += x.chars().count();
                    let color = *colors.last().unwrap();
                    let line = lines.last_mut().unwrap();
                    match line.last_mut() {
                        Some(sp) if sp.color == color => sp.text.push_str(&x),
                        _ => line.push(Span { color, text: x }),
                    }
                }
            }
            Tok::Tag { name, value, close } => {
                let cue = |kind: CueKind, lines: &Vec<Vec<Span>>, col: usize| Cue { line: lines.len() - 1, col, kind };
                match (name.as_str(), close) {
                    ("color", false) => match digit(&value) {
                        Some(d) => colors.push(d),
                        None => p.warnings.push(format!("<color={}>: expected a palette index 0-9", value.unwrap_or_default())),
                    },
                    ("color", true) => {
                        if colors.len() > 1 {
                            colors.pop();
                        } else {
                            p.warnings.push("</color> without <color>".into());
                        }
                    }
                    ("speaker", false) => in_speaker = true,
                    ("speaker", true) => in_speaker = false,
                    ("br", false) => {
                        lines.push(vec![]);
                        col_chars = 0;
                    }
                    ("page", false) => p.cues.push(cue(CueKind::Page, &lines, col_chars)),
                    ("instant", false) => p.cues.push(cue(CueKind::Instant, &lines, col_chars)),
                    ("speed", false) => match digit(&value) {
                        Some(d) => p.cues.push(cue(CueKind::Speed(d), &lines, col_chars)),
                        None => p.warnings.push("<speed=N/>: N must be 0-9".into()),
                    },
                    ("glyph", false) => match digit(&value) {
                        Some(d) => p.cues.push(cue(CueKind::Glyph(d), &lines, col_chars)),
                        None => p.warnings.push("<glyph=N/>: N must be 0-9".into()),
                    },
                    ("ctl", false) => {
                        let v = value.unwrap_or_default();
                        let mut it = v.chars();
                        match it.next() {
                            Some(c) if c.is_ascii_alphabetic() => {
                                let rest: String = it.collect();
                                p.cues.push(cue(CueKind::Ctl(c, rest.parse().ok()), &lines, col_chars));
                            }
                            _ => p.warnings.push("<ctl=X/>: X must be a letter".into()),
                        }
                    }
                    (n, c) => p.warnings.push(format!("unknown tag <{}{n}>", if c { "/" } else { "" })),
                }
            }
        }
    }
    lines.retain(|l| !l.is_empty());
    p.lines = lines;
    p.speaker = speaker.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    p
}

/// Visible text only (tags dropped, `<br/>` -> `\n`, speaker omitted).
pub fn plain(s: &str) -> String {
    let p = parse(s);
    p.lines.iter().map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n")
}

/// Structure of a string for translation checks: the sorted multiset of tags (`color=6`, `/color`, `br`, ...) and
/// `{placeholder}` names. A translation must keep it equal to the original's (reordering is allowed).
pub fn signature(s: &str) -> Vec<String> {
    let mut w = Vec::new();
    let mut sig: Vec<String> = lex(s, &mut w)
        .into_iter()
        .filter_map(|t| match t {
            Tok::Tag { name, value, close } => Some(format!("{}{}{}", if close { "/" } else { "" }, name, value.map(|v| format!("={v}")).unwrap_or_default())),
            Tok::Text(_) => None,
        })
        .collect();
    sig.extend(super::message::placeholders(s).into_iter().map(|p| format!("{{{p}}}")));
    sig.sort();
    sig
}

/// Apply `f` to the text between tags, leaving tags untouched (used by script converters).
pub fn map_text(s: &str, f: &dyn Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(s.len());
    let mut seg = String::new();
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '<' && cs.get(i + 1) != Some(&'<') {
            if let Some(e) = cs[i..].iter().position(|&c| c == '>') {
                let tag: String = cs[i..i + e + 1].iter().collect();
                if tag[1..tag.len() - 1].chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '=')) {
                    out.push_str(&f(&seg));
                    seg.clear();
                    out.push_str(&tag);
                    i += e + 1;
                    continue;
                }
            }
        }
        if cs[i] == '<' && cs.get(i + 1) == Some(&'<') {
            seg.push_str("<<");
            i += 2;
            continue;
        }
        seg.push(cs[i]);
        i += 1;
    }
    out.push_str(&f(&seg));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colors_lines_and_cues() {
        let p = parse("<speaker>A</speaker><instant/><speed=0/><color=6>hi</color> x<br/>y<glyph=1/>z<page/>");
        assert_eq!(p.speaker.as_deref(), Some("A"));
        assert_eq!(p.lines.len(), 2);
        assert_eq!(p.lines[0], vec![Span { color: 6, text: "hi".into() }, Span { color: 0, text: " x".into() }]);
        assert_eq!(p.lines[1][0].text, "yz");
        assert_eq!(p.cues.iter().map(|c| c.kind.clone()).collect::<Vec<_>>(), vec![CueKind::Instant, CueKind::Speed(0), CueKind::Glyph(1), CueKind::Page]);
        assert_eq!((p.cues[2].line, p.cues[2].col), (1, 1));
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn nested_colors_and_escapes() {
        let p = parse("<color=1>a<color=2>b</color>c</color>d <<tag>");
        let t: Vec<(u8, &str)> = p.lines[0].iter().map(|s| (s.color, s.text.as_str())).collect();
        assert_eq!(t, vec![(1, "a"), (2, "b"), (1, "c"), (0, "d <tag>")]);
        assert_eq!(escape("a<b {c}"), "a<<b {{c}}");
        assert_eq!(plain("<color=3>x</color><br/>y"), "x\ny");
    }

    #[test]
    fn bad_markup_warns_but_never_panics() {
        let p = parse("a < b <bogus/> <color=x>c</color></color>");
        assert!(p.warnings.len() >= 3, "{:?}", p.warnings);
        assert!(p.lines[0].iter().any(|s| s.text.contains("a ")));
    }

    #[test]
    fn signature_ignores_order_and_text() {
        assert_eq!(signature("<color=6>a</color>{1}<br/>"), signature("{1}<br/><color=6>zzz</color>"));
        assert_ne!(signature("<br/>"), signature("<br/><br/>"));
        assert_ne!(signature("<color=6>a</color>"), signature("<color=5>a</color>"));
    }

    #[test]
    fn map_text_skips_tags() {
        let up = |s: &str| s.to_uppercase();
        assert_eq!(map_text("a<color=6>b</color>c<<d", &up), "A<color=6>B</color>C<<D");
    }
}
