//! Built-in data-driven themes (TOML). Our own measurements of the original layout, no game data.

/// Dialogue box in the original's style (see [`crate::dialogue::DialogueTheme`]).
pub const ORIGINAL_DIALOGUE: &str = include_str!("../themes/original-dialogue.toml");

#[cfg(test)]
mod tests {
    #[test]
    fn original_dialogue_parses() {
        let th = crate::dialogue::DialogueTheme::from_toml(super::ORIGINAL_DIALOGUE).expect("theme");
        assert_eq!(th.style, "parchment");
        assert_eq!(th.palette.len(), 10);
        assert!((th.font_px + th.char_gap - 22.0).abs() < 0.01, "22 px CJK cell");
    }
}
