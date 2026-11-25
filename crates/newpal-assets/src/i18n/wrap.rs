//! Line layout / wrapping for dialogue text. Pure logic (glyph advances come from a callback), so it is unit
//! tested without a font.
//!
//! * [`WrapMode::Char`]: every character is a break opportunity (CJK; the original behaviour).
//! * [`WrapMode::Word`]: break at spaces; CJK characters inside the text are still individually breakable;
//!   a word wider than the line is split by character. Spaces never start a wrapped line.
//!
//! Both modes apply a minimal *kinsoku* rule: closing punctuation is never pushed to the start of a line
//! (it hangs past the right edge instead). TODO: full UAX #14 line breaking, hyphenation, bidi / RTL shaping.
use crate::talk::Span;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WrapMode {
    #[default]
    Char,
    Word,
}

impl WrapMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "char" | "cjk" => Some(Self::Char),
            "word" | "space" => Some(Self::Word),
            _ => None,
        }
    }
}

/// One positioned character.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub ch: char,
    pub color: u8,
    /// pen x within the line (px)
    pub x: f32,
    pub line: usize,
}

/// Characters that must not start a line.
const NO_START: &str = ",.;:!?)]}>%\u{3001}\u{3002}\u{ff0c}\u{ff0e}\u{ff1a}\u{ff1b}\u{ff01}\u{ff1f}\u{ff09}\u{3011}\u{300d}\u{300f}\u{300b}\u{3009}\u{2026}\u{2014}\u{ff5e}\u{30fb}\u{201d}\u{2019}";

fn is_wide(c: char) -> bool {
    let u = c as u32;
    (0x2E80..=0xD7FF).contains(&u) || (0xF900..=0xFAFF).contains(&u) || (0xFE30..=0xFE4F).contains(&u) || (0xFF00..=0xFFEF).contains(&u) || (0x20000..=0x3FFFF).contains(&u)
}

/// Lay out `lines` (hard line breaks = the outer Vec) into `max_w` pixels. Returns the glyphs and the number of
/// visual lines. `adv(ch)` is the advance width of a character.
pub fn layout(lines: &[Vec<Span>], max_w: f32, mode: WrapMode, adv: &mut dyn FnMut(char) -> f32) -> (Vec<Placed>, usize) {
    let mut out = Vec::new();
    let mut line = 0usize;
    for l in lines {
        // units: (chars with colour, width, is_space)
        let mut units: Vec<(Vec<(char, u8)>, f32, bool)> = Vec::new();
        let mut cur: Vec<(char, u8)> = Vec::new();
        let mut cur_w = 0.0f32;
        for span in l {
            for ch in span.text.chars() {
                let w = adv(ch);
                let flush = |units: &mut Vec<(Vec<(char, u8)>, f32, bool)>, cur: &mut Vec<(char, u8)>, cur_w: &mut f32| {
                    if !cur.is_empty() {
                        units.push((std::mem::take(cur), *cur_w, false));
                        *cur_w = 0.0;
                    }
                };
                if mode == WrapMode::Char {
                    units.push((vec![(ch, span.color)], w, false));
                } else if ch == ' ' || ch == '\t' {
                    flush(&mut units, &mut cur, &mut cur_w);
                    units.push((vec![(ch, span.color)], w, true));
                } else if is_wide(ch) {
                    flush(&mut units, &mut cur, &mut cur_w);
                    units.push((vec![(ch, span.color)], w, false));
                } else {
                    cur.push((ch, span.color));
                    cur_w += w;
                    if ch == '-' {
                        flush(&mut units, &mut cur, &mut cur_w);
                    }
                }
            }
        }
        if !cur.is_empty() {
            units.push((cur, cur_w, false));
        }
        let mut x = 0.0f32;
        let mut line_has = false;
        for (chars, w, space) in units {
            if space {
                if !line_has {
                    continue; // no leading spaces after a wrap
                }
                for (ch, color) in chars {
                    out.push(Placed { ch, color, x, line });
                    x += adv(ch);
                }
                continue;
            }
            let hangs = chars.len() == 1 && NO_START.contains(chars[0].0);
            if line_has && x + w > max_w && !hangs {
                line += 1;
                x = 0.0;
                line_has = false;
            }
            if w > max_w && chars.len() > 1 {
                // over-long word: split by character
                for (ch, color) in chars {
                    let cw = adv(ch);
                    if line_has && x + cw > max_w {
                        line += 1;
                        x = 0.0;
                    }
                    out.push(Placed { ch, color, x, line });
                    x += cw;
                    line_has = true;
                }
            } else {
                for (ch, color) in chars {
                    out.push(Placed { ch, color, x, line });
                    x += adv(ch);
                }
                line_has = true;
            }
        }
        line += 1;
    }
    (out, line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(t: &str) -> Vec<Span> {
        vec![Span { color: 0, text: t.to_string() }]
    }
    fn adv(c: char) -> f32 {
        if is_wide(c) { 20.0 } else { 10.0 }
    }
    fn lines_of(p: &[Placed], n: usize) -> Vec<String> {
        (0..n).map(|i| p.iter().filter(|g| g.line == i).map(|g| g.ch).collect()).collect()
    }

    #[test]
    fn char_wrap_cjk() {
        let t = "\u{4e00}\u{4e8c}\u{4e09}\u{56db}\u{4e94}"; // 5 wide chars = 100px
        let (p, n) = layout(&[sp(t)], 60.0, WrapMode::Char, &mut adv);
        assert_eq!(n, 2);
        assert_eq!(lines_of(&p, n)[0].chars().count(), 3);
    }

    #[test]
    fn word_wrap_latin() {
        let (p, n) = layout(&[sp("hello big world")], 100.0, WrapMode::Word, &mut adv);
        assert_eq!(lines_of(&p, n), vec!["hello big ", "world"]);
        // no leading space on the wrapped line
        assert_eq!(p.iter().find(|g| g.line == 1).unwrap().x, 0.0);
    }

    #[test]
    fn overlong_word_splits() {
        let (p, n) = layout(&[sp("abcdefghijkl")], 50.0, WrapMode::Word, &mut adv);
        assert_eq!(n, 3);
        assert!(p.iter().all(|g| g.x + 10.0 <= 50.0 + 0.01));
    }

    #[test]
    fn kinsoku_closing_punct_hangs() {
        // 3 wide chars fill 60px; the following full-width comma must stay on line 0
        let t = "\u{4e00}\u{4e8c}\u{4e09}\u{ff0c}\u{56db}";
        let (p, n) = layout(&[sp(t)], 60.0, WrapMode::Char, &mut adv);
        let l = lines_of(&p, n);
        assert!(l[0].ends_with('\u{ff0c}'), "{l:?}");
        assert_eq!(l[1], "\u{56db}");
    }

    #[test]
    fn colors_and_hard_breaks_survive() {
        let l = vec![vec![Span { color: 6, text: "ab".into() }, Span { color: 0, text: "cd".into() }], sp("e")];
        let (p, n) = layout(&l, 500.0, WrapMode::Word, &mut adv);
        assert_eq!(n, 2);
        assert_eq!(p[0].color, 6);
        assert_eq!(p[3].color, 0);
        assert_eq!(p[4].line, 1);
    }
}
