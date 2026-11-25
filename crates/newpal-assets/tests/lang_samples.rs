//! The shipped sample packs load without warnings and only use valid markup / keys.
use newpal_assets::i18n::{catalog, markup, message};
use newpal_assets::legacy::stringdb;
use std::path::Path;

#[test]
fn sample_packs_are_clean() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lang-samples");
    for loc in ["en", "zh-Hans"] {
        let p = catalog::load_pack_dir(&root.join(loc)).unwrap();
        assert!(p.warnings.is_empty(), "{loc}: {:?}", p.warnings);
        assert_eq!(p.meta.markup.as_deref(), Some("1"));
        assert!(!p.strings.is_empty());
        for (k, v) in &p.strings {
            let parsed = markup::parse(&message::format(v, &[("x", "1".into()), ("y", "2".into()), ("name", "m".into()), ("1", 1.into()), ("2", 2.into())], loc));
            assert!(parsed.warnings.is_empty(), "{loc} {k}: {:?}", parsed.warnings);
            let known = k.starts_with("talk:") || k.starts_with("ui:") || stringdb::parse_key(k).is_some();
            assert!(known, "{loc}: unexpected key {k}");
        }
    }
}
