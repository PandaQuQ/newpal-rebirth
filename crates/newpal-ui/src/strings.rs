//! UI text by key. Every visible string of the front end is `ui:<name>`. The texts live in the engine's i18n
//! catalog (`newpal-assets/src/i18n/builtin/<locale>.toml`, overridable by language packs and mods); this crate
//! only sees them through [`TextSource`], so it stays free of asset dependencies and holds no display text.
use std::sync::Arc;

/// The engine's i18n layer (implemented in the game crate on top of `newpal_assets::i18n::I18n`).
pub trait TextSource: Send + Sync {
    /// Text of `ui:<name>` in the current locale (with the catalog's own fallbacks).
    fn text(&self, name: &str) -> Option<String>;
    /// Same, with message arguments (`{n}`, `{n, plural, ...}`).
    fn text_fmt(&self, name: &str, args: &[(&str, String)]) -> Option<String>;
    fn locale(&self) -> String;
}

#[derive(Clone)]
pub struct Strings {
    src: Option<Arc<dyn TextSource>>,
    fixed: String,
}

fn subst(mut s: String, args: &[(&str, String)]) -> String {
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

impl Strings {
    /// No catalog: every text is its key name (tests, previews).
    pub fn keys_only(locale: &str) -> Self {
        Self { src: None, fixed: locale.to_string() }
    }
    pub fn with_source(src: Arc<dyn TextSource>) -> Self {
        Self { src: Some(src), fixed: String::new() }
    }
    pub fn locale(&self) -> String {
        self.src.as_ref().map(|s| s.locale()).unwrap_or_else(|| self.fixed.clone())
    }
    /// `ui:<name>`, or the name itself when nothing provides it.
    pub fn t(&self, name: &str) -> String {
        self.src.as_ref().and_then(|s| s.text(name)).unwrap_or_else(|| name.to_string())
    }
    /// `ui:<name>` formatted with `args`.
    pub fn f(&self, name: &str, args: &[(&str, String)]) -> String {
        match &self.src {
            Some(s) => s.text_fmt(name, args).unwrap_or_else(|| subst(name.to_string(), args)),
            None => subst(name.to_string(), args),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Pack;
    impl TextSource for Pack {
        fn text(&self, k: &str) -> Option<String> {
            (k == "launcher.found").then(|| "Found {n}".to_string())
        }
        fn text_fmt(&self, k: &str, a: &[(&str, String)]) -> Option<String> {
            self.text(k).map(|t| subst(t, a))
        }
        fn locale(&self) -> String {
            "en-GB".into()
        }
    }
    #[test]
    fn source_then_key() {
        let s = Strings::with_source(Arc::new(Pack));
        assert_eq!(s.f("launcher.found", &[("n", "2".into())]), "Found 2");
        assert_eq!(s.t("menu.new"), "menu.new");
        assert_eq!(Strings::keys_only("en").t("menu.quit"), "menu.quit");
    }
}
