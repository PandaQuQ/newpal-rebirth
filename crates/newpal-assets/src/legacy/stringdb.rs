//! `<locale>/StringDB.txt`: the original game's own string table for menus, battle, shop, messages and errors.
//!
//! (The original exe itself contains no usable text: the Steam build ships one UTF-8 `StringDB.txt` per language
//! folder - `zh-Hant`, `zh-Hans`, `en` - and picks the folder from the Steam language /
//! `GetUserDefaultLangID`: `schinese` or 0x804 -> `zh-Hans`, 0x404 / anything Steam-Chinese -> `zh-Hant`, else `en`.
//! A line-oriented parser then reads the file.)
//!
//! Format: one entry per line, `<id><space or '='><text>`; lines starting with `;` are
//! comments; `\r` is ignored; text runs to the end of the line and keeps the printf format of the C code
//! (`%d %s %-3d`), `$0..$3` colour switches and the literal two-character escape `\r\n`. Our importer turns
//! that into message syntax: [`to_message`].
//!
//! The `en` file of the Steam build is a byte copy of `zh-Hant` (no English text), `zh-Hans` is a real
//! Simplified Chinese translation. [`crate::i18n`] therefore treats an official entry identical to the original
//! as *untranslated* unless it contains no CJK characters.
use std::collections::BTreeMap;
use std::path::Path;

/// Key categories by id range (inclusive). Purely a classification of the original numbering, used to build
/// stable keys `<category>:<id>` and translator-friendly output files. `key` ids are SDL key names (not translated).
const SEGMENTS: &[(u32, u32, &str)] = &[
    (1, 99, "sys"),
    (100, 399, "key"),
    (1001, 1002, "menu"),
    (1003, 1224, "battle"),
    (1225, 1229, "ui"),
    (1230, 1247, "err"),
    (1248, 1254, "menu"),
    (1255, 1275, "ui"),
    (1276, 1308, "err"),
    (1309, 1325, "ui"),
    (1326, 1347, "err"),
    (1350, 1363, "name"),
    (1364, 1435, "err"),
    (1436, 1461, "ui"),
    (1462, 1468, "err"),
    (1469, 1491, "menu"),
    (1492, 1495, "err"),
    (1496, 1531, "menu"),
    (1532, 1533, "err"),
    (1534, 1554, "menu"),
    (1555, 1555, "err"),
    (1556, 1559, "menu"),
    (1560, 1561, "err"),
    (1562, 1568, "menu"),
    (1569, 1569, "err"),
    (1570, 1628, "menu"),
    (1629, 1631, "err"),
    (1632, 1643, "shop"),
    (1644, 1646, "err"),
    (1647, 1652, "menu"),
    (2008, 2014, "err"),
    (2015, 2015, "ui"),
    (2016, 2017, "msg"),
    (2018, 2019, "err"),
    (2020, 2020, "ui"),
    (2021, 2027, "err"),
    (2028, 2047, "msg"),
    (2048, 2048, "ui"),
    (2049, 2065, "err"),
    (2066, 2067, "shop"),
];

/// Every category name in output order.
pub const CATEGORIES: &[&str] = &["sys", "ui", "menu", "battle", "shop", "msg", "name", "err", "key"];

pub fn category(id: u32) -> &'static str {
    SEGMENTS.iter().find(|(a, b, _)| (*a..=*b).contains(&id)).map(|s| s.2).unwrap_or("ui")
}

pub fn key(id: u32) -> String {
    format!("{}:{id}", category(id))
}

/// `battle:1006` -> 1006 (only for the categories above; the prefix must match the id's category).
pub fn parse_key(k: &str) -> Option<u32> {
    let (c, n) = k.split_once(':')?;
    let id: u32 = n.parse().ok()?;
    (CATEGORIES.contains(&c) && category(id) == c).then_some(id)
}

/// Is this id something a translator should see? (`key` names are SDL key labels.)
pub fn translatable(id: u32) -> bool {
    category(id) != "key"
}

#[derive(Clone, Debug, Default)]
pub struct StringDb {
    /// folder name (`zh-Hant`...)
    pub folder: String,
    pub entries: BTreeMap<u32, String>,
    pub warnings: Vec<String>,
}

impl StringDb {
    pub fn parse(folder: &str, text: &str) -> Self {
        let mut db = StringDb { folder: folder.to_string(), ..Default::default() };
        for (n, line) in text.trim_start_matches('\u{feff}').split('\n').enumerate() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            let Some(sep) = line.find([' ', '=']) else {
                db.warnings.push(format!("{folder}:{}: no separator", n + 1));
                continue;
            };
            let Ok(id) = line[..sep].parse::<u32>() else {
                db.warnings.push(format!("{folder}:{}: non-numeric id {:?}", n + 1, &line[..sep]));
                continue;
            };
            if db.entries.insert(id, line[sep + 1..].to_string()).is_some() {
                db.warnings.push(format!("{folder}:{}: duplicate id {id} (later wins)", n + 1));
            }
        }
        db
    }

    /// `<dir>/StringDB.txt` (UTF-8; invalid bytes are replaced). `None` if missing.
    pub fn load(folder: &str, dir: &Path) -> Option<Self> {
        let bytes = std::fs::read(dir.join("StringDB.txt")).ok()?;
        Some(Self::parse(folder, &String::from_utf8_lossy(&bytes)))
    }

    /// Entry `id` converted to message syntax.
    pub fn message(&self, id: u32) -> Option<String> {
        self.entries.get(&id).map(|r| to_message(r))
    }
}

const DIALOGUE_LETTERS: &str = "CQNSGKPLDBA";

/// printf format + `$d` colours + `\r\n` -> message syntax with inline markup:
/// `%d`/`%s`/`%-3d` -> `{1}` / `{2}` / `{3:-3}` (numbered in order), `$1x$0` -> `<color=1>x</color>`,
/// `\r\n` -> `<br/>`, `%%C6..%%Q` (a dialogue code that went through printf) -> `<color=6>..`, `%%` -> `%`.
pub fn to_message(raw: &str) -> String {
    let cs: Vec<char> = raw.chars().collect();
    let mut pre = String::with_capacity(raw.len());
    let mut specs: Vec<String> = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '%' {
            if cs.get(i + 1) == Some(&'%') {
                match cs.get(i + 2) {
                    Some(l) if DIALOGUE_LETTERS.contains(*l) => pre.push('%'),
                    _ => pre.push_str("%%"),
                }
                i += 2;
                continue;
            }
            let mut j = i + 1;
            let mut flags = String::new();
            while j < cs.len() && matches!(cs[j], '-' | '0' | '+' | ' ' | '#') {
                if matches!(cs[j], '-' | '0') && !flags.contains(cs[j]) {
                    flags.push(cs[j]);
                }
                j += 1;
            }
            let w0 = j;
            while j < cs.len() && cs[j].is_ascii_digit() {
                j += 1;
            }
            let width: String = cs[w0..j].iter().collect();
            if cs.get(j) == Some(&'.') {
                j += 1;
                while j < cs.len() && cs[j].is_ascii_digit() {
                    j += 1;
                }
            }
            if cs.get(j).is_some_and(|t| "dsxXcuif".contains(*t)) {
                // `0` as a flag only counts when a width follows
                let f = if width.is_empty() { flags.replace('0', "") } else { flags };
                specs.push(format!("{f}{width}"));
                pre.push('\u{e000}');
                i = j + 1;
                continue;
            }
            pre.push('%');
            i += 1;
            continue;
        }
        if c == '$' && cs.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
            pre.push_str("%C");
            pre.push(cs[i + 1]);
            i += 2;
            continue;
        }
        if c == '\\' && cs.get(i + 1) == Some(&'r') && cs.get(i + 2) == Some(&'\\') && cs.get(i + 3) == Some(&'n') {
            pre.push_str("%N");
            i += 4;
            continue;
        }
        if c == '\\' && cs.get(i + 1) == Some(&'n') {
            pre.push_str("%N");
            i += 2;
            continue;
        }
        pre.push(c);
        i += 1;
    }
    let m = super::markup::from_legacy(&pre, false);
    let mut out = String::with_capacity(m.len() + 8);
    let mut k = 0usize;
    for ch in m.chars() {
        if ch == '\u{e000}' {
            k += 1;
            match specs.get(k - 1).filter(|s| !s.is_empty()) {
                Some(s) => out.push_str(&format!("{{{k}:{s}}}")),
                None => out.push_str(&format!("{{{k}}}")),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_like_the_exe() {
        let db = StringDb::parse("t", "\u{feff}1 one\r\n;comment\n\n2=two words\n3  lead\nx bad\n5\n2 again\n");
        assert_eq!(db.entries[&1], "one");
        assert_eq!(db.entries[&2], "again");
        assert_eq!(db.entries[&3], " lead");
        assert_eq!(db.warnings.len(), 3, "{:?}", db.warnings); // x, 5, duplicate
    }

    #[test]
    fn categories_and_keys() {
        assert_eq!(category(1006), "battle");
        assert_eq!(category(1633), "shop");
        assert_eq!(category(1619), "menu");
        assert_eq!(category(1350), "name");
        assert_eq!(category(104), "key");
        assert_eq!(key(1099), "battle:1099");
        assert_eq!(parse_key("shop:1633"), Some(1633));
        assert_eq!(parse_key("menu:1633"), None);
        assert_eq!(parse_key("ui:dialogue.name_sep"), None);
        assert!(!translatable(150) && translatable(1006));
    }

    #[test]
    fn printf_colors_escapes() {
        assert_eq!(to_message("%s\u{5347}\u{7b2c}%d\u{7d1a}"), "{1}\u{5347}\u{7b2c}{2}\u{7d1a}");
        assert_eq!(to_message("HP:%-3d LV:%02d %4s"), "HP:{1:-3} LV:{2:02} {3:4}");
        assert_eq!(to_message("$1%s$0 up$2!"), "<color=1>{1}</color> up<color=2>!</color>");
        assert_eq!(to_message("a\\r\\nb"), "a<br/>b");
        assert_eq!(to_message("+50%% res"), "+50% res");
        // a dialogue code that passed through printf: %%C6 .. %%C4 .. %%Q
        assert_eq!(to_message("get %%C6%s%%C4 gold%%Q"), "get <color=6>{1}</color><color=4> gold</color>");
        assert_eq!(to_message("{x} <y>"), "{{x}} <<y>");
        assert_eq!(to_message("%2d/%d"), "{1:2}/{2}");
    }
}
