**English** | [简体中文](CONTRIBUTING.zh-CN.md)

# Contributing

Thanks for looking. This is a small, early project, so the rules are short.

## The one hard rule: no original assets

This repository must never contain anything from the original game, or anything derived from it:

- sprites, maps, backgrounds, audio, music, movies, palettes, fonts (including `CT.ttf`), scripts, string tables
  (`StringDB.txt`), dialogue dumps, Big5 dumps;
- screenshots that show original art, upscaled or restored frames (`@4x.png` and the like), residual patches
  (`.nres`, `manifest.json`);
- translations *with the original text attached* (for example `newpal-i18n export --with-source` output).

Pull requests that include any of these will be closed, and the history will have to be cleaned. If you are unsure
whether something is OK, ask in an issue first. Tests that need the game must read it from the player's install and
skip themselves when it is missing (see `crates/newpal-assets/tests/golden.rs`).
The optional local file `crates/newpal-assets/tests/golden.json` holds hashes used by those tests; generate it
from your own install with `NEWPAL_WRITE_GOLDEN=1 cargo test -p newpal-assets --test golden write_golden` and
never commit it (it is gitignored).

Please also keep your own paths, tokens and personal data out of commits. `newpal.toml` is ignored on purpose.

## Setup

```powershell
git clone https://github.com/PandaQuQ/newpal-rebirth
cd newpal-rebirth
cargo build -p newpal-game
cargo test
cargo clippy --workspace --all-targets
cargo fmt --all
```

You need your own copy of the game for the golden tests and for running the engine. See
[docs/en/getting-started.md](docs/en/getting-started.md).

## Code

- Rust stable. Keep `cargo test` green; add a test for a new reader or opcode where you can.
- **Text policy:** no Big5, no codec use and no CJK characters in code outside `crates/newpal-assets/src/legacy/`.
  All user-visible strings go through i18n keys. `tests/text_policy.rs` enforces this.
- Do not modify or write to the game folder, and do not touch the game process. The folder is read-only input.
- Keep game timing intact: logic ticks at 35 ms. Rendering and interpolation must not change durations.
- Prefer small, focused pull requests with a clear description of what changed and why.

## Translations and language packs

Packs are plain TOML. See [docs/en/i18n.md](docs/en/i18n.md). Contribute your own wording only. Do not paste original
text into a pack you want to publish. Untranslated keys can be left empty.

## Docs

English and Simplified Chinese docs are kept side by side (`docs/en`, `docs/zh-CN`, `README.md`,
`README.zh-CN.md`). If you change one, update the other or note in the PR that the translation is still needed.

## Licence

By contributing you agree that your contribution is licensed under **MIT OR Apache-2.0**, the same as the project.
