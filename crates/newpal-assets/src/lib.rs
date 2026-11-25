//! # newpal-assets
//!
//! Readers for the original *New Chinese Paladin* (新仙劍奇俠傳, Steam app 695630, SWD3 engine) data files,
//! plus a virtual file system (VFS) that lets mods override any asset by a stable logical name.
//!
//! This crate ships **no game data**. Everything is read in place, read-only, from the player's own install
//! (see [`gamedir`]). Byte layouts are documented in `FINDINGS.md`; all values are little-endian.

pub mod act;
pub mod bytes;
pub mod error;
pub mod g7f;
pub mod gamedir;
pub mod i18n;
pub mod image;
pub mod legacy;
pub mod lmf;
pub mod lzo;
pub mod mapsdat;
pub mod nav;
pub mod smooth;
pub mod smoothpak;
pub mod pak;
pub mod tables;
pub mod talk;
mod talk_ops_gen;
pub mod tsw;
pub mod vfs;

pub use error::{Error, Result};