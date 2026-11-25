//! Original control codes -> inline markup ([`crate::i18n::markup`]). Pure `&str -> String`, no Big5 here.
//!
//! Codes the original text renderer understands (its draw loop and its measuring twin):
//! `%C<d>` colour, `%N` newline, `%P` page (draw + wait), `%K` instant text (disables the typewriter and makes
//! later `%S` a no-op), `%S<d>` typewriter delay, `%G<d>` glyph draw mode (`0 -> 4`, `1 -> 16`, else `d`),
//! `%D<d>` second (shadow) colour, `%L` line end, `%B..`/`%A..` inline resource markers, `%Q` end of text.
//! Codes without a modelled meaning (`%L %D %B %A`) are kept as `<ctl=X/>` so nothing is lost.

#[derive(Clone, Debug, PartialEq)]
enum T {
    Text(String),
    Color(u8),
    Br,
    Page,
    Key,
    Speed(u8),
    Glyph(u8),
    Ctl(char, Option<u8>),
    End,
}

fn tokenize(s: &str) -> Vec<T> {
    let cs: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    let digit = |i: usize| cs.get(i).and_then(|c| c.to_digit(10)).map(|d| d as u8);
    while i < cs.len() {
        if cs[i] != '%' || i + 1 >= cs.len() {
            text.push(cs[i]);
            i += 1;
            continue;
        }
        let c = cs[i + 1];
        let (tok, used) = match c {
            'C' => match digit(i + 2) {
                Some(d) => (Some(T::Color(d)), 3),
                None => (Some(T::Ctl('C', None)), 2),
            },
            'N' => (Some(T::Br), 2),
            'P' => (Some(T::Page), 2),
            'K' | 'k' => (Some(T::Key), 2),
            'S' => match digit(i + 2) {
                Some(d) => (Some(T::Speed(d)), 3),
                None => (Some(T::Ctl('S', None)), 2),
            },
            'G' => match digit(i + 2) {
                Some(d) => (Some(T::Glyph(d)), 3),
                None => (Some(T::Ctl('G', None)), 2),
            },
            'D' => match digit(i + 2) {
                Some(d) => (Some(T::Ctl('D', Some(d))), 3),
                None => (Some(T::Ctl('D', None)), 2),
            },
            'L' | 'B' | 'b' | 'A' | 'a' => (Some(T::Ctl(c, None)), 2),
            'Q' => (Some(T::End), 2),
            '%' => {
                text.push('%');
                (None, 2)
            }
            _ => {
                text.push('%');
                (None, 1)
            }
        };
        i += used;
        if let Some(t) = tok {
            if !text.is_empty() {
                out.push(T::Text(std::mem::take(&mut text)));
            }
            let end = t == T::End;
            out.push(t);
            if end {
                return out;
            }
        }
    }
    if !text.is_empty() {
        out.push(T::Text(text));
    }
    out
}

/// Does `s` contain original control codes (a pack / mod file written for the old text format)?
pub fn looks_legacy(s: &str) -> bool {
    let b = s.as_bytes();
    b.windows(2).any(|w| w[0] == b'%' && matches!(w[1], b'C' | b'N' | b'S' | b'G' | b'K' | b'k' | b'P' | b'Q' | b'L' | b'D'))
}

/// Convert original text to markup. With `speaker`, a first line `Name:` / `Name\u{ff1a}` followed by `%N` and more
/// text becomes `<speaker>Name</speaker>` + the rest (what the dialogue box wants).
pub fn from_legacy(raw: &str, speaker: bool) -> String {
    let toks = tokenize(raw);
    let (mut start, mut init, mut name) = (0usize, 0u8, None::<String>);
    if speaker {
        if let Some(br) = toks.iter().position(|t| matches!(t, T::Br)) {
            let head = &toks[..br];
            if head.iter().all(|t| matches!(t, T::Text(_) | T::Color(_))) && toks[br + 1..].iter().any(|t| !matches!(t, T::Br | T::End)) {
                let plain: String = head.iter().filter_map(|t| if let T::Text(x) = t { Some(x.as_str()) } else { None }).collect();
                let tt = plain.trim();
                if tt.ends_with(['\u{ff1a}', ':']) {
                    let n = tt.trim_end_matches(['\u{ff1a}', ':']).trim();
                    if !n.is_empty() {
                        name = Some(n.to_string());
                        init = head.iter().rev().find_map(|t| if let T::Color(c) = t { Some(*c) } else { None }).unwrap_or(0);
                        start = br + 1;
                    }
                }
            }
        }
    }
    let mut out = String::with_capacity(raw.len() + 16);
    if let Some(n) = &name {
        out.push_str("<speaker>");
        out.push_str(&crate::i18n::markup::escape(n));
        out.push_str("</speaker>");
    }
    let mut cur = 0u8;
    let set = |out: &mut String, cur: &mut u8, new: u8| {
        if new == *cur {
            return;
        }
        if *cur != 0 {
            out.push_str("</color>");
        }
        if new != 0 {
            out.push_str(&format!("<color={new}>"));
        }
        *cur = new;
    };
    // colours are applied lazily, right before the next visible text: no empty `<color>` pairs
    let mut want = init;
    for t in &toks[start..] {
        match t {
            T::Text(x) => {
                set(&mut out, &mut cur, want);
                out.push_str(&crate::i18n::markup::escape(x));
            }
            T::Color(d) => want = *d,
            T::Br => out.push_str("<br/>"),
            T::Page => out.push_str("<page/>"),
            T::Key => out.push_str("<instant/>"),
            T::Speed(d) => out.push_str(&format!("<speed={d}/>")),
            T::Glyph(d) => out.push_str(&format!("<glyph={d}/>")),
            T::Ctl(c, d) => out.push_str(&format!("<ctl={c}{}/>", d.map(|d| d.to_string()).unwrap_or_default())),
            T::End => break,
        }
    }
    set(&mut out, &mut cur, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::markup::{parse, CueKind};

    #[test]
    fn speaker_split_and_colors() {
        assert_eq!(from_legacy("A:%N%C6hi%C0 there%Q", true), "<speaker>A</speaker><color=6>hi</color> there");
        // colour set before the break carries over to the body; fullwidth colon
        assert_eq!(from_legacy("%C6A\u{ff1a}%Nhi", true), "<speaker>A</speaker><color=6>hi</color>");
        assert_eq!(from_legacy("%C6A\u{ff1a}%C0%Nhi", true), "<speaker>A</speaker>hi");
        // colour changes without text in between leave no empty tags
        assert_eq!(from_legacy("%C6A:%C5%N%C5hi%C0", true), "<speaker>A</speaker><color=5>hi</color>");
        assert_eq!(from_legacy("a%C6%C0b", false), "ab");
        // not a speaker: no body / no colon / disabled
        assert_eq!(from_legacy("A:%N", true), "A:<br/>");
        assert_eq!(from_legacy("no colon%Nx", true), "no colon<br/>x");
        assert_eq!(from_legacy("A:%Nx", false), "A:<br/>x");
    }

    #[test]
    fn s_g_k_p_and_unknown_codes() {
        let m = from_legacy("%K%S0%G1%C6hi%P%L%D2x%%y", false);
        assert_eq!(m, "<instant/><speed=0/><glyph=1/><color=6>hi<page/><ctl=L/><ctl=D2/>x%y</color>");
        let p = parse(&m);
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        assert_eq!(p.cues[0].kind, CueKind::Instant);
        assert_eq!(p.cues[3].kind, CueKind::Page);
        assert_eq!(p.cues[5].kind, CueKind::Ctl('D', Some(2)));
    }

    #[test]
    fn braces_and_angle_brackets_are_escaped_and_percent_stays() {
        assert_eq!(from_legacy("a<b {c} 50%x", false), "a<<b {{c}} 50%x");
        assert_eq!(from_legacy("100%", false), "100%");
    }

    #[test]
    fn detects_legacy() {
        assert!(looks_legacy("%C6a"));
        assert!(looks_legacy("a%Nb"));
        assert!(!looks_legacy("<br/> 100% sure"));
    }
}
