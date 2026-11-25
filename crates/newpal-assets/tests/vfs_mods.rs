//! VFS / mod layer tests. Synthetic data only, no game install needed.
use newpal_assets::image::RgbaImage;
use newpal_assets::tables::Table;
use newpal_assets::vfs::{discover_mods, Origin, Vfs};
use std::path::PathBuf;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("newpal-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn hd_override_keeps_logical_size_and_priority() {
    let root = tmp("mods");
    let game = tmp("nogame");
    for (m, prio, scale) in [("low", 0, "@2x"), ("high", 10, "@4x")] {
        let d = root.join(m);
        std::fs::create_dir_all(d.join("sprites/all_char/187")).unwrap();
        std::fs::write(d.join("mod.toml"), format!("name = \"{m}\"\npriority = {prio}\n")).unwrap();
        let img = RgbaImage::new(176, 384);
        img.save_png(&d.join(format!("sprites/all_char/187/f4{scale}.png"))).unwrap();
    }
    let mods = discover_mods(&root, &[]);
    assert_eq!(mods[0].name, "high");
    let vfs = Vfs::new(game, mods);
    let t = vfs.sprite_frame("all_char", 187, 4).unwrap();
    assert_eq!(t.origin, Origin::Mod("high".into()));
    assert_eq!((t.logical_w, t.logical_h), (44.0, 96.0)); // 176x384 @4x -> 44x96 logical
    assert_eq!(t.scale(), 4.0);
    // explicit enable list overrides priority
    let vfs = Vfs::new(tmp("nogame2"), discover_mods(&root, &["low".to_string()]));
    let t = vfs.sprite_frame("all_char", 187, 4).unwrap();
    assert_eq!((t.logical_w, t.scale()), (88.0, 2.0));
}

#[test]
fn table_override_json_and_toml() {
    let mut t = Table { name: "ghero".into(), records: vec![] };
    t.records.push(newpal_assets::g7f::Record { kind: 0x7F10_1000, id: Some(2000), name: Some("A".into()), ..Default::default() });
    let j = Table::parse_override(r#"{"records":[{"id":2000,"attrs":{"16":999}}]}"#, false).unwrap();
    t.apply(&j).unwrap();
    assert_eq!(t.get(2000).unwrap().attrs[&16], 999);
    let tm = Table::parse_override("[[records]]\nid = 2100\nkind = 2132807680\nname = \"NewActor\"\n[records.attrs]\n\"29\" = 50\n", true).unwrap();
    t.apply(&tm).unwrap();
    assert_eq!(t.get(2100).unwrap().name.as_deref(), Some("NewActor"));
    assert_eq!(t.get(2100).unwrap().attrs[&29], 50);
}