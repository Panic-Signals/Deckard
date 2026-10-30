# Stem separation (EDIT ▸ STEMS)

Status: **working** (October 2026), in the software and on the MK3.

## What Maschine does

From the Maschine 3 software manual and the MK3 manual ("Audio Editing Functions"):

- STEMS separates the focused zone's sample into four stems: drums (and percussion),
  bass, other (what remains) and vocals.
- It runs in the background; the Stems button shows its progress as a percentage, and
  Cancel stops it. Only one separation runs at a time: STEMS is grayed out for other
  sounds meanwhile.
- When it's done, a new group is created whose first four sounds hold the stems in
  Sampler modules, named after the sample followed by Drums, Bass, Other or Vocal; the
  stems are saved as samples (in the user library).
- On the MK3 (NI's own scripts, unchanged): EDIT, ◀ / ▶ to STEMS, Button 8 APPLY. The
  displays show "Stem Separation in Progress" and a progress bar
  (`FocusSampleAccess.getStemSeparationProgress`); Button 8 becomes CANCEL.

NI's separation is iZotope's (licensed). DECKARD's is open:

## How DECKARD does it

- **Model:** Meta's Demucs v4 `htdemucs` (MIT), the 4-stem model, run in Rust by
  [demucs-rs](https://github.com/nikhilunni/demucs-rs) (Apache-2.0; `demucs-core`,
  pinned by commit in `crates/stems/Cargo.toml`) on Burn: the GPU through Metal (macOS)
  or Vulkan (Linux, Windows), the CPU if that fails. On an Apple Silicon Mac a 3½-minute
  song takes about 80 s; the CPU is ~10× slower.
- **Weights:** `htdemucs.safetensors` (84,030,696 bytes), as Hugging Face's
  `set-soft/audio_separation` serves it, checked against its published SHA-256
  (`deckard_stems::MODEL_SHA256`). Never bundled: installed into DECKARD's settings
  folder's `models` by
  - `scripts/install-demucs.sh` (macOS, Linux) and `scripts/install-demucs.ps1`
    (Windows; Windows PowerShell 5.1 or PowerShell 7);
  - `mise install` (its `postinstall` hook runs `mise run install:demucs`; skipped on CI
    unless `DECKARD_INSTALL_DEMUCS=1`);
  - the release packages carry the installer: beside the program (Linux tarball,
    Windows zip, with a line in their README) and in `DECKARD.app/Contents/Resources`;
  - and DECKARD runs that installer itself the first time STEMS is used without the
    model (the button reads MODEL… meanwhile).
- **The job** (`apps/viewer/src/stems.rs`): a thread per separation, progress per chunk
  of the model's segments, cancelled between chunks. Its result: four WAVs next to the
  project (`Stems/<name> Drums.wav`, …; `~/Music/DECKARD/Stems` without a project), then
  one undo step: a new group named after the sample, its sounds 1–4 Audio plug-ins playing
  them (in time with the song, at the source's length in beats: its take's, or from its
  name or length), named as Maschine names them, on in the group's first pattern, which
  plays in the focused scene; the new group is focused. (Maschine loads them in Samplers;
  DECKARD's are loops, `docs/audio-plugin.md`.)
- **Software:** EDIT's toolbar has STEMS (after STRETCH): while it runs on this sample
  it shows the percentage, and a click cancels; it's off for other sounds meanwhile.
- **Controller:** `api/sampling.rs` answers NI's calls — `executeStemSeparation`,
  `cancelStemSeparation`, `getStemSeparationSource` (the focused sample's transaction
  sample while it's that one, another object while it's another's),
  `getStemSeparationProgress` — from `State::stems`, which the app fills each frame.
- Unlike Maschine, mono samples separate too (as two equal channels).

## Checking it

- `cargo test -p deckard-stems` checks the model file check; with
  `DECKARD_STEMS_MODEL=<folder with the model>` it also separates a short made-up mix
  (the four stems in order, adding up to it; cancelling).
- `DECKARD_STEMS_MODEL=… cargo test -p deckard controller_stems` runs STEMS from the
  controller on the test project's kick and checks the new group.
- `cargo run --release -p deckard-stems --example separate -- MODEL_DIR IN.wav OUT_DIR
  [gpu|cpu]` separates a file, timing it and saying how close the stems add up to it.
