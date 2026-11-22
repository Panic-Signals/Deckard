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

Pinned at `bfdce66`; see `../transit-audio-asio/NOTICE.md` for how to update.
