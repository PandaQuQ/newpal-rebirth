//! `newpal-ui`: the modern front end of New PAL Reborn.
//!
//! * everything is **vector**: shapes, icons, ornaments and text are paths rendered by vello on the GPU at the
//!   window's physical resolution, so the UI is sharp at any size / DPI (no bitmaps are shipped);
//! * screens live on a [`scene::SceneStack`] with animated transitions (fade / slide / ink wipe);
//! * input goes through [`input::InputMapper`]: keyboard + mouse + gamepad -> abstract [`input::Action`]s,
//!   rebindable and stored in `newpal.toml`;
//! * all visible text is looked up by `ui:` key ([`strings::Strings`]); language packs override the built-in
//!   English / Simplified / Traditional defaults;
//! * engine services (game folder detection, HD resources, saves, mods, languages) are reached through the
//!   [`host::Host`] trait, so this crate does not depend on the asset / game crates.
pub mod anim;
pub mod dialogue;
pub mod orig;
pub mod gpu;
pub mod host;
pub mod input;
pub mod scene;
pub mod screens;
pub mod settings;
pub mod strings;
pub mod text;
pub mod theme;
pub mod themes;

pub use vello;
