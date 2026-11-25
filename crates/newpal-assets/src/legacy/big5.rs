//! The only place that knows the original data is Big5. Everything else works on UTF-8 `String`s.

/// Decode original text bytes (no BOM sniffing), replacing invalid sequences with U+FFFD.
pub fn decode_text(b: &[u8]) -> String {
    encoding_rs::BIG5.decode_without_bom_handling(b).0.into_owned()
}

/// Decode a NUL-terminated / NUL-padded original text field.
pub fn decode_cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    decode_text(&b[..end])
}

/// Some dialogue instructions carry binary operand bytes in front of the text and one of them happens to be `%`,
/// so the text start is detected too early. When the decoded text contains U+FFFD, return how many leading bytes
/// are operand noise (everything up to the last control byte / invalid Big5 sequence, plus the spaces that follow);
/// 0 when the text is clean or no clean remainder exists.
pub fn binary_prefix(b: &[u8]) -> usize {
    if !decode_text(b).contains('\u{FFFD}') {
        return 0;
    }
    let (mut last, mut i) = (None, 0usize);
    while i < b.len() {
        let c = b[i];
        if c < 0x20 {
            last = Some(i);
            i += 1;
        } else if c >= 0x81 {
            let ok = i + 1 < b.len() && !encoding_rs::BIG5.decode_without_bom_handling(&b[i..i + 2]).1;
            if ok {
                i += 2;
            } else {
                last = Some(i);
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    let Some(j) = last else { return 0 };
    let rest = &b[j + 1..];
    if rest.is_empty() || decode_text(rest).contains('\u{FFFD}') {
        return 0;
    }
    j + 1 + rest.iter().take_while(|&&c| c == b' ').count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_big5_and_cstr() {
        // "\u{4f60}\u{597d}" in Big5
        assert_eq!(decode_text(&[0xa7, 0x41, 0xa6, 0x6e]), "\u{4f60}\u{597d}");
        assert_eq!(decode_cstr(&[0xa7, 0x41, 0, 0x41, 0]), "\u{4f60}");
        assert_eq!(decode_cstr(b"abc"), "abc");
    }

    #[test]
    fn binary_prefix_is_cut_only_when_the_remainder_is_clean() {
        assert_eq!(binary_prefix(b"%Qhello"), 0);
        // noise, control byte, spaces, then clean text
        let mut v = b"%a\x01\xff".to_vec();
        v.extend_from_slice(b"\x02  abc");
        assert_eq!(binary_prefix(&v), v.len() - 3);
        // no clean remainder: leave untouched
        assert_eq!(binary_prefix(&[0x25, 0x01, 0xff]), 0);
    }
}
