//! # Messages: placeholders and plurals (an ICU MessageFormat subset)
//!
//! ```text
//! {name}                         named argument            {1}   positional argument (1-based)
//! {n:4}  {n:-4}  {n:04}          min width, left-aligned, zero-padded (numbers)
//! {n, plural, =0 {none} one {# item} other {# items}}      CLDR plural, `#` = the number
//! {g, select, f {her} m {his} other {their}}
//! {{  }}                         literal braces
//! ```
//! Plural categories follow CLDR (`zero one two few many other`) for the *integer* rules of the locale in
//! [`plural_category`]; a language without a table uses `one`/`other` like English. A translator may add any
//! branch the language needs; `other` is the required fallback. Tags such as `<color=6>` are not touched here.
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Int(i64),
    Str(String),
}

impl From<i64> for Arg {
    fn from(v: i64) -> Self {
        Arg::Int(v)
    }
}
impl From<i32> for Arg {
    fn from(v: i32) -> Self {
        Arg::Int(v as i64)
    }
}
impl From<u32> for Arg {
    fn from(v: u32) -> Self {
        Arg::Int(v as i64)
    }
}
impl From<usize> for Arg {
    fn from(v: usize) -> Self {
        Arg::Int(v as i64)
    }
}
impl From<&str> for Arg {
    fn from(v: &str) -> Self {
        Arg::Str(v.to_string())
    }
}
impl From<String> for Arg {
    fn from(v: String) -> Self {
        Arg::Str(v)
    }
}

impl Arg {
    fn text(&self) -> String {
        match self {
            Arg::Int(i) => i.to_string(),
            Arg::Str(s) => s.clone(),
        }
    }
    fn int(&self) -> Option<i64> {
        match self {
            Arg::Int(i) => Some(*i),
            Arg::Str(s) => s.trim().parse().ok(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plural {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

impl Plural {
    pub fn name(self) -> &'static str {
        match self {
            Plural::Zero => "zero",
            Plural::One => "one",
            Plural::Two => "two",
            Plural::Few => "few",
            Plural::Many => "many",
            Plural::Other => "other",
        }
    }
}

/// CLDR plural category of integer `n` in `locale` (BCP-47 tag; only the language subtag matters).
/// Covered: no-plural CJK/SEA languages, English-like (default), French/Portuguese, Russian/Ukrainian, Polish,
/// Czech/Slovak, Arabic. Add a language by adding an arm.
pub fn plural_category(locale: &str, n: i64) -> Plural {
    let lang = locale.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    let n = n.unsigned_abs();
    let (m10, m100) = (n % 10, n % 100);
    match lang.as_str() {
        "zh" | "ja" | "ko" | "vi" | "th" | "id" | "ms" | "lo" | "km" | "my" => Plural::Other,
        "fr" | "pt" => {
            if n <= 1 {
                Plural::One
            } else if n != 0 && n % 1_000_000 == 0 {
                Plural::Many
            } else {
                Plural::Other
            }
        }
        "ru" | "uk" | "be" => {
            if m10 == 1 && m100 != 11 {
                Plural::One
            } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                Plural::Few
            } else {
                Plural::Many
            }
        }
        "pl" => {
            if n == 1 {
                Plural::One
            } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                Plural::Few
            } else {
                Plural::Many
            }
        }
        "cs" | "sk" => match n {
            1 => Plural::One,
            2..=4 => Plural::Few,
            _ => Plural::Other,
        },
        "ar" => match n {
            0 => Plural::Zero,
            1 => Plural::One,
            2 => Plural::Two,
            _ if (3..=10).contains(&m100) => Plural::Few,
            _ if (11..=99).contains(&m100) => Plural::Many,
            _ => Plural::Other,
        },
        _ => {
            if n == 1 {
                Plural::One
            } else {
                Plural::Other
            }
        }
    }
}

/// Index of the `}` matching the `{` at `open` (nesting aware).
fn matching(cs: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < cs.len() {
        match cs[i] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split `a, b, rest` on the first two top-level commas.
fn split_head(inner: &str) -> (String, Option<String>, Option<String>) {
    let mut parts = inner.splitn(3, ',');
    let a = parts.next().unwrap_or("").trim().to_string();
    (a, parts.next().map(|s| s.trim().to_string()), parts.next().map(str::to_string))
}

/// `one {..} other {..}` -> [(selector, body)]
fn branches(spec: &str) -> Vec<(String, String)> {
    let cs: Vec<char> = spec.chars().collect();
    let (mut i, mut out) = (0, Vec::new());
    while i < cs.len() {
        while i < cs.len() && cs[i].is_whitespace() {
            i += 1;
        }
        let s0 = i;
        while i < cs.len() && !cs[i].is_whitespace() && cs[i] != '{' {
            i += 1;
        }
        let sel: String = cs[s0..i].iter().collect();
        while i < cs.len() && cs[i].is_whitespace() {
            i += 1;
        }
        if i >= cs.len() || cs[i] != '{' || sel.is_empty() {
            break;
        }
        let Some(e) = matching(&cs, i) else { break };
        out.push((sel, cs[i + 1..e].iter().collect()));
        i = e + 1;
    }
    out
}

fn lookup<'a>(args: &'a [(&str, Arg)], name: &str) -> Option<&'a Arg> {
    args.iter().find(|(n, _)| *n == name).map(|(_, a)| a)
}

fn pad(s: String, spec: &str, numeric: bool) -> String {
    let left = spec.starts_with('-');
    let spec = spec.trim_start_matches('-');
    let zero = numeric && spec.starts_with('0') && spec.len() > 1;
    let w: usize = spec.parse().unwrap_or(0);
    let len = s.chars().count();
    if len >= w {
        return s;
    }
    let fill = w - len;
    if left {
        format!("{s}{}", " ".repeat(fill))
    } else if zero {
        let (sign, digits) = s.strip_prefix('-').map(|d| ("-", d)).unwrap_or(("", &s));
        format!("{sign}{}{digits}", "0".repeat(fill))
    } else {
        format!("{}{s}", " ".repeat(fill))
    }
}

fn fmt_into(t: &str, args: &[(&str, Arg)], locale: &str, hash: Option<&str>, out: &mut String) {
    let cs: Vec<char> = t.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '{' if cs.get(i + 1) == Some(&'{') => {
                out.push('{');
                i += 2;
            }
            '}' if cs.get(i + 1) == Some(&'}') => {
                out.push('}');
                i += 2;
            }
            '{' => match matching(&cs, i) {
                Some(e) => {
                    let inner: String = cs[i + 1..e].iter().collect();
                    eval(&inner, args, locale, out);
                    i = e + 1;
                }
                None => {
                    out.push('{');
                    i += 1;
                }
            },
            '#' if hash.is_some() => {
                out.push_str(hash.unwrap());
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
}

fn eval(inner: &str, args: &[(&str, Arg)], locale: &str, out: &mut String) {
    let (head, kind, rest) = split_head(inner);
    match kind.as_deref() {
        None => {
            let (name, spec) = head.split_once(':').map(|(n, s)| (n.trim(), s.trim())).unwrap_or((head.as_str(), ""));
            match lookup(args, name) {
                Some(a) => out.push_str(&if spec.is_empty() { a.text() } else { pad(a.text(), spec, matches!(a, Arg::Int(_))) }),
                None => {
                    let _ = write!(out, "{{{inner}}}");
                }
            }
        }
        Some(k @ ("plural" | "select")) => {
            let bs = branches(rest.as_deref().unwrap_or(""));
            let Some(a) = lookup(args, &head) else {
                let _ = write!(out, "{{{inner}}}");
                return;
            };
            let (pick, hash) = if k == "plural" {
                let n = a.int().unwrap_or(0);
                let cat = plural_category(locale, n).name();
                let exact = format!("={n}");
                (bs.iter().find(|(s, _)| *s == exact).or_else(|| bs.iter().find(|(s, _)| s == cat)).or_else(|| bs.iter().find(|(s, _)| s == "other")), Some(n.to_string()))
            } else {
                let v = a.text();
                (bs.iter().find(|(s, _)| *s == v).or_else(|| bs.iter().find(|(s, _)| s == "other")), None)
            };
            if let Some((_, body)) = pick {
                fmt_into(body, args, locale, hash.as_deref(), out);
            }
        }
        Some(other) => {
            let _ = write!(out, "{{{inner}}}");
            let _ = other;
        }
    }
}

/// Resolve placeholders and plurals of `template` for `locale`.
pub fn format(template: &str, args: &[(&str, Arg)], locale: &str) -> String {
    if !template.contains('{') && !template.contains('}') {
        return template.to_string();
    }
    let mut out = String::with_capacity(template.len() + 8);
    fmt_into(template, args, locale, None, &mut out);
    out
}

/// Names of every argument a template references (for translation checks), sorted, de-duplicated.
pub fn placeholders(template: &str) -> Vec<String> {
    fn walk(t: &str, out: &mut Vec<String>) {
        let cs: Vec<char> = t.chars().collect();
        let mut i = 0;
        while i < cs.len() {
            if cs[i] == '{' && cs.get(i + 1) == Some(&'{') {
                i += 2;
            } else if cs[i] == '{' {
                let Some(e) = matching(&cs, i) else { break };
                let inner: String = cs[i + 1..e].iter().collect();
                let (head, kind, rest) = split_head(&inner);
                let name = head.split(':').next().unwrap_or("").trim().to_string();
                out.push(name);
                if kind.is_some() {
                    for (_, body) in branches(rest.as_deref().unwrap_or("")) {
                        walk(&body, out);
                    }
                }
                i = e + 1;
            } else {
                i += 1;
            }
        }
    }
    let mut v = Vec::new();
    walk(template, &mut v);
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(t: &str, a: &[(&str, Arg)], l: &str) -> String {
        format(t, a, l)
    }

    #[test]
    fn simple_and_padded() {
        assert_eq!(f("{x}+{1}", &[("x", 5.into()), ("1", "a".into())], "en"), "5+a");
        assert_eq!(f("[{1:4}][{1:-4}][{1:04}]", &[("1", 42.into())], "en"), "[  42][42  ][0042]");
        assert_eq!(f("{{x}} {y}", &[], "en"), "{x} {y}");
        assert_eq!(f("{n:03}", &[("n", (-5).into())], "en"), "-05");
    }

    #[test]
    fn english_plural() {
        let t = "{n, plural, =0 {no items} one {# item} other {# items}}";
        assert_eq!(f(t, &[("n", 0.into())], "en"), "no items");
        assert_eq!(f(t, &[("n", 1.into())], "en-GB"), "1 item");
        assert_eq!(f(t, &[("n", 7.into())], "en"), "7 items");
    }

    #[test]
    fn cldr_rules() {
        assert_eq!(plural_category("zh-Hans", 1), Plural::Other);
        assert_eq!(plural_category("fr", 0), Plural::One);
        assert_eq!(plural_category("ru", 21), Plural::One);
        assert_eq!(plural_category("ru", 23), Plural::Few);
        assert_eq!(plural_category("ru", 12), Plural::Many);
        assert_eq!(plural_category("pl", 22), Plural::Few);
        assert_eq!(plural_category("ar", 105), Plural::Few);
        assert_eq!(plural_category("de", 2), Plural::Other);
        let t = "{n, plural, one {# \u{44f}\u{431}\u{43b}\u{43e}\u{43a}\u{43e}} few {# \u{44f}\u{431}\u{43b}\u{43e}\u{43a}\u{430}} other {# \u{44f}\u{431}\u{43b}\u{43e}\u{43a}}}";
        assert!(f(t, &[("n", 3.into())], "ru").ends_with('\u{430}'));
        // CJK: `other` only
        assert_eq!(f("{n, plural, one {# a} other {# b}}", &[("n", 1.into())], "zh-Hant"), "1 b");
    }

    #[test]
    fn select_nested_and_missing() {
        let t = "{g, select, f {{n, plural, one {her #} other {her # x}}} other {their}}";
        assert_eq!(f(t, &[("g", "f".into()), ("n", 1.into())], "en"), "her 1");
        assert_eq!(f(t, &[("g", "q".into())], "en"), "their");
        assert_eq!(f("hi {nobody}", &[], "en"), "hi {nobody}");
        assert_eq!(f("{broken", &[], "en"), "{broken");
        assert_eq!(f("keeps <color=6>tags</color> and #", &[], "en"), "keeps <color=6>tags</color> and #");
    }

    #[test]
    fn placeholder_names() {
        assert_eq!(placeholders("{b} {a} {a:3} {n, plural, one {# {c}} other {x}} {{lit}}"), vec!["a", "b", "c", "n"]);
    }
}
