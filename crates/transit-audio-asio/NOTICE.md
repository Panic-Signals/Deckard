# Vendored from transit

`transit-audio-asio` and `transit-audio-backend` are copied from
https://github.com/transit-audio/transit (`crates/audio-asio`, `crates/audio-backend`) at
commit `bfdce66` (v0.7.2), under the MIT or Apache-2.0 licence, at the user's option
(`LICENSE-MIT`, `LICENSE-APACHE` beside each crate). transit began as a fork of SoundCraft:
"Copyright (c) 2026 ArtCraft Team and the SoundCraft contributors".

Changes from upstream: the package manifests (no workspace inheritance), and the tests that
need transit's `transit-rt` crate (`stream_tests.rs`, `release_tests.rs`) are left out. The
source is otherwise unchanged, so a newer upstream can be copied over it.

## Keeping up with upstream

Pinned at `bfdce66` (v0.7.2, 2026-11-14). transit is alpha and its authors have bench-tested two
interfaces, so check it for fixes now and then, and when working on ASIO or audio drivers:
`git log --oneline bfdce66..origin/main -- crates/audio-asio crates/audio-backend` in a clone of
transit lists what changed. The commit messages say which are driver fixes (resets, stalls,
buffer sizes, sample types) or API changes.

To update:

1. Clone transit into a scratch folder (never run its code from inside the clone).
2. Copy its `crates/audio-asio/src` and `crates/audio-backend/src` over the `src/` of the two
   crates here. Leave out `stream_tests.rs` and `release_tests.rs` (they need `transit-rt`), and
   drop their `mod` lines in `lib.rs` and `host.rs`. Keep this repository's `Cargo.toml`s and
   `rustfmt.toml`s (transit formats at width 150, DECKARD at 140).
3. Change the commit above and in `../transit-audio-backend/NOTICE.md`.
4. Build with and without `--features asio`, and run `cargo test -p transit-audio-asio`.

Don't patch these files for DECKARD: send the fix to transit first. If it can't wait, make it
its own commit and say so in it, so the next update doesn't lose it. What DECKARD needs on top
belongs in `crates/engine/src/native_asio.rs`.
