//! # legacy: the original-game import layer
//!
//! The original game stores text as hard-coded **Big5** bytes with printf / control-code mini-languages
//! (`%C6`, `%N`, `%S0`, `%d`, `$1`...). This module is the *only* code allowed to know that. It turns the
//! original text into the engine's modern form:
//!
//! * UTF-8 `String`s ([`decode_text`]),
//! * structured inline markup (`<color=6>`, `<br/>`, `<speaker>`...) and message placeholders
//!   (`{1}`, `{n, plural, ...}`), see [`markup`] and [`stringdb`],
//! * stable keys (`talk:`, `name:`, `item:`..., `battle:`, `menu:`, `shop:` ...), see [`source`].
//!
//! Nothing outside `legacy/` may decode Big5 or hold hard-coded display text: `tests/text_policy.rs` enforces it.
mod big5;
pub mod build;
pub mod markup;
pub mod source;
pub mod stringdb;

pub use big5::{binary_prefix, decode_cstr, decode_text};
