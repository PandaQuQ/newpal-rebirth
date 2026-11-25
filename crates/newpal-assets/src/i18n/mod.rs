//! # i18n
//!
//! All text the engine shows is **UTF-8** and reaches the screen only through this catalog: a stable **key** is
//! resolved through the layers below. The original game's Big5 / control codes are converted once, in
//! [`crate::legacy`] (the only module that may know them); everything here and downstream sees UTF-8 strings
//! with the inline [`markup`] (`<color=6>`, `<br/>`, `<speaker>`) and [`message`] placeholders (`{1}`, plurals).
//!
//! ## Keys
//! | key | text |
//! |---|---|
//! | `talk:<script>:<pos>` | dialogue instruction of talk script `<script>` at byte `<pos>` (4+ hex digits, relative to the script start); markup, see [`markup`]. Same position key as the mod text overrides. |
//! | `name:<original speaker>` | speaker name shown in front of a line |
//! | `item|magic|hero|monster|level:<id>:name` / `:desc` | `gitem` / `gmagic` / `ghero` / `gmon` / `glevel` records |
//! | `ui:<name>` | engine UI string (built in: [`builtin`]); `{x}`-style placeholders |
//! | `sys|ui|menu|battle|shop|msg|name|err:<id>` | the original game's own string table (`StringDB.txt`), see [`crate::legacy::stringdb`]; message syntax |
//!
//! ## Lookup order (first hit wins)
//! 1. mod text overrides `text/talk/<n>.json` (legacy, locale independent)
//! 2. mod language packs `<mod>/lang/<locale>/` (mods by priority; most specific locale first)
//! 3. language packs `<lang_dir>/<locale>/` (same locale order)
//! 4. engine built-ins of the locale chain (`ui:` only, [`builtin`])
//! 5. text the player's own install ships for the locale (`<game>/<locale>/StringDB.txt`; the real Simplified
//!    Chinese translation in the Steam build). An entry identical to the original counts as untranslated.
//! 6. optional converter of the locale (e.g. zh-Hant -> zh-Hans char map) applied to the original text
//! 7. the original text (zh-Hant). A miss in a non-original locale is logged ([`I18n::missing`]).
//!
//! Locale tags are BCP-47: `tag`, then its `inherits` (or the tag minus its region, `en-GB` -> `en`). The original
//! game language is [`ORIGINAL_LOCALE`] (`zh-Hant`). Plural rules follow CLDR ([`message::plural_category`]).
pub mod builtin;
pub mod catalog;
pub mod charmap;
pub mod export;
pub mod fonts;
pub mod markup;
pub mod message;
pub mod wrap;

pub use catalog::{LangMeta, Pack};
pub use charmap::{CharMap, TextConverter};
pub use wrap::WrapMode;

use crate::tables::Table;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// Language of the original game data (Traditional Chinese, Big5 on disk).
pub const ORIGINAL_LOCALE: &str = "zh-Hant";

/// `ui:` names whose empty value is meaningful (shown as nothing instead of a missing-key marker).
pub const EMPTY_OK_UI: &[&str] = &["dialogue.continue"];

/// Original data table file -> key prefix.
pub const TABLE_PREFIXES: &[(&str, &str)] = &[("gitem", "item"), ("gmagic", "magic"), ("ghero", "hero"), ("gmon", "monster"), ("glevel", "level")];

pub fn table_prefix(table: &str) -> Option<&'static str> {
    TABLE_PREFIXES.iter().find(|(t, _)| *t == table).map(|(_, p)| *p)
}

/// Key builders.
pub mod key {
    pub fn talk(script: u32, pos: usize) -> String {
        format!("talk:{script}:{pos:04x}")
    }
    pub fn talk_prefix(script: u32) -> String {
        format!("talk:{script}:")
    }
    pub fn field(prefix: &str, id: i32, field: &str) -> String {
        format!("{prefix}:{id}:{field}")
    }
    pub fn ui(name: &str) -> String {
        format!("ui:{name}")
    }
    pub fn name(speaker: &str) -> String {
        format!("name:{speaker}")
    }
    /// `talk:<script>:<pos>` -> (script, pos)
    pub fn parse_talk(k: &str) -> Option<(u32, usize)> {
        let r = k.strip_prefix("talk:")?;
        let (s, p) = r.split_once(':')?;
        Some((s.parse().ok()?, usize::from_str_radix(p, 16).ok()?))
    }
}

/// Canonical BCP-47-ish tag: `zh_tw` -> `zh-Hant`, `EN-gb` -> `en-GB`, `zh` -> `zh-Hans`.
pub fn canonical(tag: &str) -> String {
    let parts: Vec<String> = tag
        .trim()
        .replace('_', "-")
        .split('-')
        .filter(|p| !p.is_empty())
        .enumerate()
        .map(|(i, p)| {
            if i == 0 {
                p.to_ascii_lowercase()
            } else if p.len() == 4 && p.chars().all(|c| c.is_ascii_alphabetic()) {
                let mut c = p.to_ascii_lowercase();
                c[..1].make_ascii_uppercase();
                c
            } else if p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic()) {
                p.to_ascii_uppercase()
            } else {
                p.to_ascii_lowercase()
            }
        })
        .collect();
    let t = parts.join("-");
    match t.as_str() {
        "zh" | "zh-CN" | "zh-SG" => "zh-Hans".into(),
        "zh-TW" | "zh-HK" | "zh-MO" => "zh-Hant".into(),
        _ => t,
    }
}

/// Implicit parent of a canonical tag: only a trailing *region* is stripped (`en-GB` -> `en`,
/// `zh-Hant-TW` -> `zh-Hant`); a script subtag is not (`zh-Hant` must never fall into `zh-Hans`).
pub fn parent_tag(tag: &str) -> Option<String> {
    let (head, last) = tag.rsplit_once('-')?;
    let region = (last.len() == 2 && last.chars().all(|c| c.is_ascii_uppercase())) || (last.len() == 3 && last.chars().all(|c| c.is_ascii_digit()));
    region.then(|| canonical(head))
}

/// `{name}` placeholder substitution for `ui:` strings (see [`message::format`]; plurals use `locale`).
pub fn format_args(template: &str, args: &[(&str, String)]) -> String {
    let a: Vec<(&str, message::Arg)> = args.iter().map(|(k, v)| (*k, message::Arg::Str(v.clone()))).collect();
    message::format(template, &a, ORIGINAL_LOCALE)
}

/// A locale that can be selected.
#[derive(Clone, Debug, PartialEq)]
pub struct LocaleInfo {
    pub tag: String,
    pub name: String,
    pub dirs: Vec<PathBuf>,
}

/// Where the *original-locale* text and the text the player's install ships come from. Implemented by
/// [`crate::legacy::source::LegacySource`]; tests can supply their own.
pub trait TextSource: Send + Sync {
    /// Original-language text of `key` in markup / message syntax.
    fn original(&self, key: &str) -> Option<String>;
    /// Text the player's install ships for exactly `locale` (a real translation; `None` when it is absent or just
    /// the original again).
    fn shipped(&self, locale: &str, key: &str) -> Option<String>;
    /// Shipped translation of a speaker's original name.
    fn shipped_name(&self, _locale: &str, _original: &str) -> Option<String> {
        None
    }
    /// Locales the install ships text for (tag, display name).
    fn locales(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    /// Notes about the install (build identification ...).
    fn notes(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Where a resolved string came from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// mod / language pack layer (its origin)
    Pack(String),
    Builtin(String),
    /// shipped by the player's install for this locale
    Shipped(String),
    /// original text through the locale's converter
    Converted,
    Original,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub text: String,
    pub source: Source,
}

struct Layer {
    origin: String,
    map: BTreeMap<String, String>,
}

struct State {
    locale: String,
    meta: LangMeta,
    /// directory the font / convert paths of `meta` are relative to
    meta_dir: Option<PathBuf>,
    /// highest priority first
    layers: Vec<Layer>,
    convert: Option<Arc<dyn TextConverter>>,
    warnings: Vec<String>,
    /// locale chain, most specific first (never contains the original locale unless it is the selected one)
    chain: Vec<String>,
}

pub struct I18n {
    pack_dirs: Vec<PathBuf>,
    /// (mod name, mod dir) by priority, highest first
    mods: Vec<(String, PathBuf)>,
    state: RwLock<State>,
    source: RwLock<Option<Arc<dyn TextSource>>>,
    missing: Mutex<BTreeSet<String>>,
    missing_n: AtomicUsize,
    generation: AtomicU64,
    quiet: std::sync::atomic::AtomicBool,
}

impl std::fmt::Debug for I18n {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("I18n").field("locale", &self.locale()).field("packs", &self.pack_dirs).finish()
    }
}

impl Default for I18n {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new())
    }
}

impl I18n {
    /// `pack_dirs`: folders that contain `<locale>/` pack directories (searched in this order);
    /// `mods`: (name, mod dir) by priority, highest first (`<mod>/lang/<locale>/`).
    /// Starts in the original language.
    pub fn new(pack_dirs: Vec<PathBuf>, mods: Vec<(String, PathBuf)>) -> Self {
        let s = Self {
            pack_dirs,
            mods,
            state: RwLock::new(State { locale: ORIGINAL_LOCALE.into(), meta: LangMeta::default(), meta_dir: None, layers: Vec::new(), convert: None, warnings: Vec::new(), chain: vec![ORIGINAL_LOCALE.into()] }),
            source: RwLock::new(None),
            missing: Mutex::default(),
            missing_n: AtomicUsize::new(0),
            generation: AtomicU64::new(0),
            quiet: std::sync::atomic::AtomicBool::new(false),
        };
        let _ = s.set_locale(ORIGINAL_LOCALE);
        s
    }

    /// Do not print missing-key lines to stderr (they are still recorded).
    pub fn set_quiet(&self, q: bool) {
        self.quiet.store(q, Ordering::Relaxed);
    }

    pub fn locale(&self) -> String {
        self.state.read().unwrap().locale.clone()
    }

    pub fn is_original(&self) -> bool {
        self.state.read().unwrap().locale == ORIGINAL_LOCALE
    }

    /// Bumps on every locale switch (cache invalidation for rendered text).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn meta(&self) -> LangMeta {
        self.state.read().unwrap().meta.clone()
    }

    pub fn wrap_mode(&self) -> WrapMode {
        self.state.read().unwrap().meta.wrap.unwrap_or(if self.is_original() { WrapMode::Char } else { WrapMode::Word })
    }

    /// Problems found while loading the current locale (bad TOML, missing files ...).
    pub fn warnings(&self) -> Vec<String> {
        self.state.read().unwrap().warnings.clone()
    }

    pub fn has_converter(&self) -> bool {
        self.state.read().unwrap().convert.is_some()
    }

    /// Converter for non-overridden original text (`None` = show the original as is).
    pub fn text_filter(&self) -> Option<Arc<dyn TextConverter>> {
        self.state.read().unwrap().convert.clone()
    }

    /// Font files named by the language pack (resolved to paths or `system:<file>` entries).
    pub fn font_entries(&self) -> Vec<fonts::FontEntry> {
        let st = self.state.read().unwrap();
        st.meta.fonts.iter().map(|f| fonts::FontEntry::resolve(f, st.meta_dir.as_deref())).collect()
    }

    fn find_dirs(&self, tag: &str) -> (Vec<PathBuf>, Vec<(String, PathBuf)>) {
        let want = canonical(tag);
        let sub = |root: &Path| -> Option<PathBuf> {
            std::fs::read_dir(root).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.is_dir() && p.file_name().is_some_and(|n| canonical(&n.to_string_lossy()) == want))
        };
        let packs = self.pack_dirs.iter().filter_map(|d| sub(d)).collect();
        let mods = self.mods.iter().filter_map(|(n, d)| sub(&d.join("lang")).map(|p| (n.clone(), p))).collect();
        (packs, mods)
    }

    /// Switch the language at runtime. `Err` if no pack of any kind exists for `tag` (state unchanged).
    pub fn set_locale(&self, tag: &str) -> Result<LocaleInfo, String> {
        let tag = canonical(tag);
        // locale chain, most specific first
        let mut chain = vec![tag.clone()];
        let mut metas: Vec<(String, Option<(PathBuf, LangMeta)>)> = Vec::new();
        let mut warnings = Vec::new();
        let mut layers_mod: Vec<Layer> = Vec::new();
        let mut layers_pack: Vec<Layer> = Vec::new();
        let mut found_any = false;
        let mut i = 0;
        while i < chain.len() && i < 4 {
            let c = chain[i].clone();
            let (packs, mods) = self.find_dirs(&c);
            let mut meta_of: Option<(PathBuf, LangMeta)> = None;
            for (name, d) in &mods {
                match catalog::load_pack_dir(d) {
                    Ok(p) => {
                        found_any = true;
                        warnings.extend(p.warnings);
                        meta_of.get_or_insert((p.dir.clone(), p.meta.clone()));
                        layers_mod.push(Layer { origin: format!("mod {name} [{c}]"), map: p.strings });
                    }
                    Err(e) => warnings.push(e.to_string()),
                }
            }
            for d in &packs {
                match catalog::load_pack_dir(d) {
                    Ok(p) => {
                        found_any = true;
                        warnings.extend(p.warnings);
                        // pack metadata wins over mod metadata (mods usually carry strings only)
                        meta_of = Some((p.dir.clone(), p.meta.clone()));
                        layers_pack.push(Layer { origin: format!("pack {}", d.display()), map: p.strings });
                    }
                    Err(e) => warnings.push(e.to_string()),
                }
            }
            let parent = meta_of.as_ref().and_then(|(_, m)| m.inherits.clone()).map(|p| canonical(&p)).or_else(|| parent_tag(&c));
            metas.push((c, meta_of));
            if let Some(p) = parent {
                if p != ORIGINAL_LOCALE && !chain.contains(&p) {
                    chain.push(p);
                }
            }
            i += 1;
        }
        // locales the engine or the player's install provide text for need no pack folder
        let shipped: Vec<String> = self.source.read().unwrap().as_ref().map(|s| s.locales().into_iter().map(|(t, _)| canonical(&t)).collect()).unwrap_or_default();
        if chain.iter().any(|c| shipped.contains(c) || builtin::locales().contains(&c.as_str())) {
            found_any = true;
        }
        if !found_any && tag != ORIGINAL_LOCALE {
            return Err(format!("no language pack for {tag} (searched {} pack folder(s), {} mod(s))", self.pack_dirs.len(), self.mods.len()));
        }
        // merge metadata: most specific wins per field
        let mut meta = LangMeta::default();
        let mut meta_dir = None;
        for (_, m) in metas.iter().rev() {
            if let Some((d, m)) = m {
                let mut merged = meta.clone();
                if m.name.is_some() { merged.name = m.name.clone(); }
                if m.wrap.is_some() { merged.wrap = m.wrap; }
                if m.convert.is_some() { merged.convert = m.convert.clone(); }
                if !m.fonts.is_empty() { merged.fonts = m.fonts.clone(); }
                if m.inherits.is_some() { merged.inherits = m.inherits.clone(); }
                if m.convert.is_some() || !m.fonts.is_empty() { meta_dir = Some(d.clone()); }
                meta = merged;
            }
        }
        // relative paths in `meta` refer to the pack that defined them; we keep one dir (the last definer)
        let convert: Option<Arc<dyn TextConverter>> = if tag != ORIGINAL_LOCALE {
            match (&meta.convert, &meta_dir) {
                (Some(c), _) if c.starts_with("opencc:") => match charmap::opencc(&c["opencc:".len()..]) {
                    Ok(m) => Some(m),
                    Err(e) => {
                        warnings.push(format!("convert {c}: {e}"));
                        None
                    }
                },
                (Some(c), Some(d)) => match CharMap::load(&d.join(c)) {
                    Ok(m) => Some(Arc::new(m)),
                    Err(e) => {
                        warnings.push(format!("convert map: {e}"));
                        None
                    }
                },
                _ => None,
            }
        } else {
            None
        };
        let mut layers = layers_mod;
        layers.extend(layers_pack);
        let info = LocaleInfo { tag: tag.clone(), name: meta.name.clone().unwrap_or_else(|| tag.clone()), dirs: layers.iter().map(|l| PathBuf::from(&l.origin)).collect() };
        {
            let mut st = self.state.write().unwrap();
            *st = State { locale: tag, meta, meta_dir, layers, convert, warnings, chain: chain.clone() };
        }
        self.missing.lock().unwrap().clear();
        self.missing_n.store(0, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Relaxed);
        Ok(info)
    }

    /// Selectable locales: the original plus every pack folder / mod pack found.
    pub fn available(&self) -> Vec<LocaleInfo> {
        let mut found: BTreeMap<String, LocaleInfo> = BTreeMap::new();
        found.insert(ORIGINAL_LOCALE.into(), LocaleInfo { tag: ORIGINAL_LOCALE.into(), name: "original".into(), dirs: Vec::new() });
        let mut roots: Vec<PathBuf> = self.pack_dirs.clone();
        roots.extend(self.mods.iter().map(|(_, d)| d.join("lang")));
        for r in roots {
            let Ok(rd) = std::fs::read_dir(&r) else { continue };
            for e in rd.filter_map(|e| e.ok()) {
                let p = e.path();
                if !p.is_dir() {
                    continue;
                }
                let tag = canonical(&e.file_name().to_string_lossy());
                let name = catalog::load_pack_dir(&p).ok().and_then(|k| k.meta.name).unwrap_or_else(|| tag.clone());
                found.entry(tag.clone()).and_modify(|i| i.dirs.push(p.clone())).or_insert(LocaleInfo { tag, name, dirs: vec![p] });
            }
        }
        let shipped: Vec<(String, String)> = self.source.read().unwrap().as_ref().map(|s| s.locales()).unwrap_or_default();
        for (t, n) in shipped.into_iter().chain(builtin::locales().into_iter().map(|t| (t.to_string(), t.to_string()))) {
            let tag = canonical(&t);
            found.entry(tag.clone()).or_insert(LocaleInfo { tag, name: n, dirs: Vec::new() });
        }
        found.into_values().collect()
    }

    /// Attach the provider of original / shipped text (see [`TextSource`]).
    pub fn set_source(&self, s: Arc<dyn TextSource>) {
        *self.source.write().unwrap() = Some(s);
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn has_source(&self) -> bool {
        self.source.read().unwrap().is_some()
    }

    /// Notes from the text source (build identification ...).
    pub fn source_notes(&self) -> Vec<String> {
        self.source.read().unwrap().as_ref().map(|s| s.notes()).unwrap_or_default()
    }

    /// The locale after the current one in [`I18n::available`] (wraps).
    pub fn next_locale(&self) -> String {
        let av = self.available();
        let cur = self.locale();
        let i = av.iter().position(|l| l.tag == cur).map(|i| (i + 1) % av.len()).unwrap_or(0);
        av[i].tag.clone()
    }

    /// Layered lookup without fallback / logging.
    pub fn lookup(&self, key: &str) -> Option<String> {
        let st = self.state.read().unwrap();
        st.layers.iter().find_map(|l| l.map.get(key)).cloned()
    }

    /// Which layer provides `key` (diagnostics).
    pub fn origin_of(&self, key: &str) -> Option<String> {
        let st = self.state.read().unwrap();
        st.layers.iter().find(|l| l.map.contains_key(key)).map(|l| l.origin.clone())
    }

    /// Record a key that fell back to the original text (no-op in the original locale).
    pub fn note_missing(&self, key: &str) {
        if self.is_original() {
            return;
        }
        if self.missing.lock().unwrap().insert(key.to_string()) {
            let n = self.missing_n.fetch_add(1, Ordering::Relaxed);
            if !self.quiet.load(Ordering::Relaxed) && n < 12 {
                eprintln!("i18n [{}]: no translation for {key}, using the original text{}", self.locale(), if n == 11 { " (further misses are only counted)" } else { "" });
            }
        }
    }

    /// Keys that fell back since the locale was set.
    pub fn missing(&self) -> Vec<String> {
        self.missing.lock().unwrap().iter().cloned().collect()
    }

    /// The locale chain of the current locale, most specific first.
    pub fn chain(&self) -> Vec<String> {
        self.state.read().unwrap().chain.clone()
    }

    /// Resolve `key` through every layer (see the module docs). `original` = the original text when the caller
    /// already holds it (otherwise the [`TextSource`] is asked). `None`: nothing knows the key.
    pub fn resolve(&self, key: &str, original: Option<&str>) -> Option<Resolved> {
        let (chain, conv) = {
            let st = self.state.read().unwrap();
            if let Some(l) = st.layers.iter().find(|l| l.map.contains_key(key)) {
                return Some(Resolved { text: l.map[key].clone(), source: Source::Pack(l.origin.clone()) });
            }
            (st.chain.clone(), st.convert.clone())
        };
        if key.starts_with("ui:") {
            for c in &chain {
                if let Some(t) = builtin::get(c, key) {
                    return Some(Resolved { text: t.to_string(), source: Source::Builtin(c.clone()) });
                }
            }
        }
        let src = self.source.read().unwrap().clone();
        if let Some(src) = &src {
            for c in &chain {
                if let Some(t) = src.shipped(c, key) {
                    return Some(Resolved { text: t, source: Source::Shipped(c.clone()) });
                }
            }
        }
        let orig = original.map(str::to_string).or_else(|| src.as_ref().and_then(|s| s.original(key)));
        match (orig, conv) {
            (Some(o), Some(c)) => Some(Resolved { text: c.convert(&o), source: Source::Converted }),
            (Some(o), None) => Some(Resolved { text: o, source: Source::Original }),
            (None, _) => {
                // engine strings without a text in this locale: the English built-in, flagged as a fallback below
                if key.starts_with("ui:") {
                    builtin::get("en", key).map(|t| Resolved { text: t.to_string(), source: Source::Builtin("en".into()) })
                } else {
                    None
                }
            }
        }
    }

    /// Log keys that were not translated for the current locale.
    fn note_resolved(&self, key: &str, r: &Resolved) {
        let miss = match &r.source {
            Source::Original => true,
            Source::Builtin(t) => !self.state.read().unwrap().chain.contains(t),
            _ => false,
        };
        if miss {
            self.note_missing(key);
        }
    }

    /// Translate `key`; `original` is the text from the player's data.
    pub fn tr(&self, key: &str, original: &str) -> String {
        match self.resolve(key, Some(original)) {
            Some(r) => {
                self.note_resolved(key, &r);
                r.text
            }
            None => original.to_string(),
        }
    }

    /// Text of `key` (message / markup syntax, placeholders unresolved). Unknown keys come back as the key.
    pub fn t(&self, key: &str) -> String {
        match self.resolve(key, None) {
            Some(r) => {
                self.note_resolved(key, &r);
                r.text
            }
            None => {
                self.note_missing(key);
                key.to_string()
            }
        }
    }

    /// [`I18n::t`] with placeholders / plurals resolved for the current locale.
    pub fn t_fmt(&self, key: &str, args: &[(&str, message::Arg)]) -> String {
        message::format(&self.t(key), args, &self.locale())
    }

    /// [`I18n::t_fmt`] parsed into styled lines, speaker and cues.
    pub fn render(&self, key: &str, args: &[(&str, message::Arg)]) -> markup::Parsed {
        markup::parse(&self.t_fmt(key, args))
    }

    /// Speaker name.
    pub fn tr_name(&self, original: &str) -> String {
        let k = key::name(original);
        if let Some(r) = self.resolve(&k, Some(original)) {
            if r.source == Source::Original {
                // the install may ship the character's name (StringDB name table)
                let (chain, src) = (self.chain(), self.source.read().unwrap().clone());
                if let Some(t) = src.and_then(|s| chain.iter().find_map(|c| s.shipped_name(c, original))) {
                    return t;
                }
            }
            self.note_resolved(&k, &r);
            return r.text;
        }
        original.to_string()
    }

    /// Engine UI string (`ui:<name>`).
    pub fn ui(&self, name: &str) -> String {
        let k = key::ui(name);
        match self.resolve(&k, None) {
            Some(r) => {
                self.note_resolved(&k, &r);
                r.text
            }
            None if EMPTY_OK_UI.contains(&name) => String::new(),
            None => {
                self.note_missing(&k);
                name.to_string()
            }
        }
    }

    pub fn ui_fmt(&self, name: &str, args: &[(&str, String)]) -> String {
        let a: Vec<(&str, message::Arg)> = args.iter().map(|(k, v)| (*k, message::Arg::Str(v.clone()))).collect();
        message::format(&self.ui(name), &a, &self.locale())
    }

    /// Replace names / descriptions of `t` (a `gitem`-style table) by translations (originals stay where missing).
    pub fn localize_table(&self, t: &mut Table) {
        let Some(prefix) = table_prefix(&t.name) else { return };
        if self.is_original() && self.state.read().unwrap().layers.is_empty() {
            return;
        }
        for r in &mut t.records {
            let Some(id) = r.id else { continue };
            if let Some(n) = r.name.as_mut() {
                *n = self.tr(&key::field(prefix, id, "name"), n);
            }
            if let Some(d) = r.desc.as_mut() {
                *d = self.tr(&key::field(prefix, id, "desc"), d);
            }
        }
    }

    /// Dialogue overrides of script `n` from the language layers: instruction position -> text.
    pub fn talk_overrides(&self, n: u32) -> HashMap<usize, String> {
        let st = self.state.read().unwrap();
        let prefix = key::talk_prefix(n);
        let mut out = HashMap::new();
        for l in st.layers.iter().rev() {
            for (k, v) in l.map.range(prefix.clone()..) {
                if !k.starts_with(&prefix) {
                    break;
                }
                if let Some((_, pos)) = key::parse_talk(k) {
                    out.insert(pos, v.clone());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("newpal-i18n-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    #[test]
    fn canonical_tags() {
        assert_eq!(canonical("zh_tw"), "zh-Hant");
        assert_eq!(canonical("ZH-cn"), "zh-Hans");
        assert_eq!(canonical("EN-gb"), "en-GB");
        assert_eq!(canonical("zh-hans"), "zh-Hans");
        assert_eq!(canonical("pt-br"), "pt-BR");
    }

    #[test]
    fn implicit_parents() {
        assert_eq!(parent_tag("en-GB").as_deref(), Some("en"));
        assert_eq!(parent_tag("zh-Hant-TW").as_deref(), Some("zh-Hant"));
        assert_eq!(parent_tag("zh-Hant"), None);
        assert_eq!(parent_tag("zh-Hans"), None);
        assert_eq!(parent_tag("en"), None);
    }

    #[test]
    fn original_locale_never_inherits_a_chinese_pack() {
        let root = tmp("hant");
        let packs = root.join("lang");
        write(&packs.join("zh-Hans/a.toml"), "[strings]\n\"talk:1:0000\" = \"simplified\"\n");
        let i = I18n::new(vec![packs], vec![]);
        i.set_locale("zh-Hant").unwrap();
        assert_eq!(i.lookup("talk:1:0000"), None);
        i.set_locale("zh-TW").unwrap();
        assert_eq!(i.lookup("talk:1:0000"), None);
        i.set_locale("zh-Hans").unwrap();
        assert_eq!(i.lookup("talk:1:0000").as_deref(), Some("simplified"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn keys_roundtrip() {
        assert_eq!(key::talk(104, 0x12), "talk:104:0012");
        assert_eq!(key::parse_talk("talk:104:0012"), Some((104, 0x12)));
        assert_eq!(key::parse_talk("ui:x"), None);
        assert_eq!(key::field("item", 5, "name"), "item:5:name");
    }

    struct FakeSource;
    impl TextSource for FakeSource {
        fn original(&self, key: &str) -> Option<String> {
            (key == "battle:1").then(|| "orig {1}".to_string())
        }
        fn shipped(&self, locale: &str, key: &str) -> Option<String> {
            (locale == "zh-Hans" && key == "battle:1").then(|| "shipped {1}".to_string())
        }
        fn shipped_name(&self, locale: &str, original: &str) -> Option<String> {
            (locale == "zh-Hans" && original == "A").then(|| "A-hans".to_string())
        }
        fn locales(&self) -> Vec<(String, String)> {
            vec![("zh-Hans".into(), "Shipped".into())]
        }
    }

    #[test]
    fn shipped_text_beats_converter_and_needs_no_pack() {
        let i = I18n::default();
        i.set_quiet(true);
        i.set_source(Arc::new(FakeSource));
        assert_eq!(i.t("battle:1"), "orig {1}"); // original locale
        i.set_locale("zh-Hans").unwrap(); // no pack folder, but the source ships it
        assert_eq!(i.t("battle:1"), "shipped {1}");
        assert_eq!(i.resolve("battle:1", None).unwrap().source, Source::Shipped("zh-Hans".into()));
        assert_eq!(i.t_fmt("battle:1", &[("1", 7.into())]), "shipped 7");
        assert_eq!(i.tr_name("A"), "A-hans");
        assert_eq!(i.tr_name("B"), "B");
        assert!(i.missing().contains(&"name:B".to_string()));
        // unknown key comes back visibly and is logged
        assert_eq!(i.t("battle:2"), "battle:2");
        i.set_locale("en").unwrap(); // built-in locale, nothing shipped
        assert_eq!(i.t("battle:1"), "orig {1}");
        assert!(i.missing().contains(&"battle:1".to_string()));
        assert_eq!(i.ui("dialogue.name_sep"), ": ");
        assert_eq!(i.ui("dialogue.continue"), "");
    }

    #[test]
    fn builtin_chain_and_english_fallback() {
        let i = I18n::default();
        i.set_quiet(true);
        assert_eq!(i.ui("dialogue.name_sep"), "\u{ff1a}");
        i.set_locale("zh-Hans").unwrap();
        assert!(i.ui("app.title").contains("Reborn"));
        assert!(i.missing().is_empty());
        assert_eq!(i.ui("no.such.key"), "no.such.key");
        assert_eq!(i.ui_fmt("status.no_path", &[("x", "1".into()), ("y", "2".into())]).contains("(1, 2)"), true);
    }

    #[test]
    fn plural_messages_use_the_locale() {
        let root = tmp("plural");
        let packs = root.join("lang");
        write(&packs.join("en/a.toml"), "[strings]\n\"menu:1\" = \"{n, plural, one {# potion} other {# potions}}\"\n");
        let i = I18n::new(vec![packs], vec![]);
        i.set_locale("en").unwrap();
        assert_eq!(i.t_fmt("menu:1", &[("n", 1.into())]), "1 potion");
        assert_eq!(i.t_fmt("menu:1", &[("n", 3.into())]), "3 potions");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn lookup_order_mod_over_pack_with_inheritance_and_fallback() {
        let root = tmp("order");
        let packs = root.join("lang");
        write(&packs.join("en/lang.toml"), "[meta]\nname = \"English\"\nwrap = \"word\"\n[strings]\n\"talk:1:0000\" = \"pack-en\"\n\"ui:dialogue.name_sep\" = \": \"\n\"talk:1:0002\" = \"pack-en-2\"\n");
        write(&packs.join("en-GB/lang.toml"), "[strings]\n\"talk:1:0002\" = \"pack-engb-2\"\n");
        let m = root.join("mods/m1");
        write(&m.join("lang/en/x.toml"), "[strings]\n\"talk:1:0000\" = \"mod-en\"\n");
        let i = I18n::new(vec![packs], vec![("m1".into(), m)]);
        i.set_quiet(true);
        assert!(i.is_original());
        i.set_locale("en-gb").unwrap();
        assert_eq!(i.locale(), "en-GB");
        assert_eq!(i.lookup("talk:1:0000").as_deref(), Some("mod-en")); // mod beats pack
        assert_eq!(i.lookup("talk:1:0002").as_deref(), Some("pack-engb-2")); // specific beats parent
        assert_eq!(i.ui("dialogue.name_sep"), ": "); // parent pack
        assert_eq!(i.wrap_mode(), WrapMode::Word);
        // fallback to the original + logged
        assert_eq!(i.tr("talk:1:0004", "orig"), "orig");
        assert_eq!(i.missing(), vec!["talk:1:0004".to_string()]);
        let ov = i.talk_overrides(1);
        assert_eq!(ov.len(), 2);
        assert_eq!(ov[&0], "mod-en");
        assert_eq!(ov[&2], "pack-engb-2");
        assert!(i.talk_overrides(10).is_empty()); // prefix `talk:1:` must not match `talk:10:`
        // runtime switch back: nothing is missing in the original locale
        let g = i.generation();
        i.set_locale("zh-Hant").unwrap();
        assert!(i.generation() > g);
        assert_eq!(i.tr("talk:1:0004", "orig"), "orig");
        assert!(i.missing().is_empty());
        assert_eq!(i.ui("dialogue.name_sep"), "\u{ff1a}");
        assert!(i.set_locale("xx").is_err());
        assert_eq!(i.locale(), "zh-Hant");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn converter_locale_counts_as_translated() {
        let root = tmp("conv");
        let packs = root.join("lang");
        write(&packs.join("zh-Hans/lang.toml"), "[meta]\nconvert = \"t2s.tsv\"\n[strings]\n\"talk:2:0000\" = \"manual\"\n");
        write(&packs.join("zh-Hans/t2s.tsv"), "\u{9ad4}\t\u{4f53}\n");
        let i = I18n::new(vec![packs], vec![]);
        i.set_locale("zh-hans").unwrap();
        assert!(i.has_converter());
        assert_eq!(i.tr("talk:2:0000", "x"), "manual");
        assert_eq!(i.tr("talk:2:0002", "\u{9ad4}"), "\u{4f53}");
        assert!(i.missing().is_empty());
        assert_eq!(i.available().iter().map(|l| l.tag.as_str()).collect::<Vec<_>>(), vec!["en", "zh-Hans", "zh-Hant"]);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(feature = "opencc")]
    #[test]
    fn opencc_converter_keeps_control_codes() {
        let c = charmap::opencc("t2s").unwrap();
        assert_eq!(c.convert("<color=6>\u{958b}\u{653e}</color><br/>\u{8f49}\u{63db}"), "<color=6>\u{5f00}\u{653e}</color><br/>\u{8f6c}\u{6362}");
    }

    #[test]
    fn table_localization() {
        let root = tmp("tab");
        let packs = root.join("lang");
        write(&packs.join("en/a.toml"), "[strings]\n\"item:7:name\" = \"Herb\"\n");
        let i = I18n::new(vec![packs], vec![]);
        i.set_quiet(true);
        i.set_locale("en").unwrap();
        let mut t = Table { name: "gitem".into(), records: vec![crate::g7f::Record { id: Some(7), name: Some("a".into()), desc: Some("d".into()), ..Default::default() }] };
        i.localize_table(&mut t);
        assert_eq!(t.records[0].name.as_deref(), Some("Herb"));
        assert_eq!(t.records[0].desc.as_deref(), Some("d"));
        assert_eq!(i.missing(), vec!["item:7:desc".to_string()]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn format_placeholders() {
        assert_eq!(format_args("no path to ({x}, {y})", &[("x", "1".into()), ("y", "2".into())]), "no path to (1, 2)");
    }
}
