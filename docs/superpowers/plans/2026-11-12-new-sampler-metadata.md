# New Sampler skeleton and sample metadata (roadmap 1b) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** a "New Sampler" instrument you can add to a sound, whose zones learn their tempo
and root note in the background (cached by file content) and show them.

**Architecture:** the New Sampler is Maschine's Sampler module marked DECKARD's by a
`deckard:new_sampler` field on the sound, so it reuses zones, files, the engine's
`SamplerRt` and the Sampling page; in Maschine it opens as a plain Sampler (the field is
stripped). Each zone's analysis is a `deckard:zone` field on the zone object, naming the
file it describes (a stale one is ignored). Both are saved in DECKARD block version 13.
The app analyses New Sampler zones on a thread, through an on-disk cache keyed by the
file's SHA-256, and writes the result without an undo step. The engine plays it as the
Sampler does (Repitch) until 1c.

**Tech Stack:** Rust; crates model, doc, analysis (+ `sha2`), apps/viewer (egui).

**Spec:** `docs/superpowers/plans/2026-11-11-sampler-roadmap.md` (Architecture, 1b, 1c's
Auto rule); 1a: `2026-11-12-sample-analysis.md`.

## Global Constraints

- Clean room. `deckard:` fields are stripped from Maschine's archive (existing `strip_deckard_fields`).
- Maschine still opens the project; the New Sampler is a Sampler there.
- Analysis never runs on the audio or UI thread; writing its result is not an undo step.
- Auto rule (Serato's): Sync for a zone with a tempo, unless under 3 s, or under 10 s with "808" in its name; else Repitch.
- Cargo through mise; `mise run lint` and the touched crates' tests at the end.

## Review Focus

1. A zone whose sample changes (edit, replace, undo): its old analysis isn't shown; it is analysed again (Task 1 `meta_for_another_file_is_ignored`, Task 4).
2. Undo across an analysis result: undo still steps through the user's edits, and isn't blocked by analysis writing (Task 2 `zone_meta_is_no_undo_step`).
3. A missing or unreadable file: no panic, no endless re-analysis (Task 4: a failure is remembered for that file).
4. Old projects (block v12) still read; v13 round-trips both fields (Task 2).
5. Replacing the New Sampler's slot with another instrument: it's no longer shown as a New Sampler (Task 1 `only_a_sampler_is_a_new_sampler`).

---

### Task 1: The model

**Files:** Create `crates/model/src/new_sampler.rs`; modify `crates/model/src/lib.rs` (`Sound.new_sampler`, `Zone.meta`, `pub mod new_sampler`).

**Interfaces — Produces:**
```rust
pub const FIELD: &str = "deckard:new_sampler";      // on the sound
pub const ZONE_FIELD: &str = "deckard:zone";         // on a zone object
pub enum StretchEngine { Rendered, Live }            // default Rendered
pub struct NewSampler { pub stretch: StretchEngine } // Default
pub enum PlayMode { Repitch, Sync, Keys }
pub enum BpmSource { Name, Loop, Audio, Manual }
pub struct ZoneMeta { pub file: String, pub mode: PlayMode, pub bpm: Option<f64>,
  pub bpm_source: BpmSource, pub beats: Option<f64>, pub grid_offset: u64,
  pub root: Option<u8>, pub cents: f32, pub root_locked: bool }
impl NewSampler { fn to_text(&self) -> String; fn from_text(&str) -> NewSampler }
impl ZoneMeta { fn to_text(&self) -> String; fn from_text(&str) -> Option<ZoneMeta> }
Sound.new_sampler: Option<NewSampler>  // Some only when the field is there AND instrument == "Sampler"
Zone.meta: Option<ZoneMeta>            // None when absent or its `file` isn't the zone's sample path
```
Text: `key value` lines (`v 1`, `stretch rendered`; zone: `file <path…>` last line so
paths keep spaces); unknown keys skipped.

- [ ] Tests: `settings_round_trip_as_text`, `zone_meta_round_trips` (path with spaces, every Option None and Some), `unknown_lines_are_skipped`, `meta_for_another_file_is_ignored`, `only_a_sampler_is_a_new_sampler`.
- [ ] `mise exec -- cargo test -p deckard-model new_sampler` PASS; commit.

### Task 2: The document

**Files:** Create `crates/doc/src/new_sampler.rs`; modify `crates/doc/src/block.rs` (v13), `crates/doc/src/lib.rs` (mod).

**Interfaces — Produces:** `Document::set_new_sampler(g, s, Option<&NewSampler>) -> Result<(), String>` (one undo step: "New Sampler" / "Convert to Sampler"); `Document::note_zone_meta(g, s, z, &ZoneMeta) -> Result<(), String>` (no undo step; the document's revision goes up so the engine and views follow); `pub(crate) fn new_sampler_ops`, `zone_meta_ops`. Block v13: count, then (g, s, text) per New Sampler sound; count, then (g, s, z, text) per zone meta.

- [ ] Tests: `marks_and_unmarks_a_sound` (+ undo), `zone_meta_is_no_undo_step` (an edit, then meta, then undo undoes the edit and keeps the meta), `both_round_trip_through_a_save` (to_bytes → from_bytes), `version_12_blocks_still_read`, `maschine_sees_a_sampler` (the archive without the block has no `deckard:` fields and the sound's instrument is "Sampler").
- [ ] `mise exec -- cargo test -p deckard-doc new_sampler block` PASS; commit.

### Task 3: Analysis results and their cache

**Files:** Create `crates/analysis/src/cache.rs`; modify `crates/analysis/{Cargo.toml (sha2 0.10), src/lib.rs}`.

**Interfaces — Produces:**
```rust
pub struct Analysis { pub tempo: Option<Tempo>, pub root: Option<Root>, pub seconds: f64 }
pub fn analyse(frames: &[[f32; 2]], rate: u32, name: &str) -> Analysis  // tempo hint from tempo_in_name, root_with_name
pub struct Cache { dir: PathBuf }
impl Cache { pub fn new(dir) -> Cache; pub fn key(bytes: &[u8]) -> String /* sha256 hex */;
  pub fn get(&self, key: &str) -> Option<Analysis>; pub fn put(&self, key: &str, a: &Analysis) -> io::Result<()> }
```
One text file per key (`<key>.txt`, `v 1` first; written to a temp name, then renamed).

- [ ] Tests: `analysis_round_trips_through_the_cache` (tempdir), `a_missing_or_bad_entry_is_none`, `the_key_is_the_content` (same bytes, same key; one byte off, another), `the_name_gives_the_hint` (120 BPM drums named "x 100bpm.wav" → bpm 100, Hint).
- [ ] `mise exec -- cargo test -p deckard-analysis cache` PASS; commit.

### Task 4: The app

**Files:** Create `apps/viewer/src/analysis.rs`; modify `apps/viewer/Cargo.toml`, `apps/viewer/src/main.rs` (job polling, after edits), `apps/viewer/src/layout.rs` (`AddWhat::NewSampler`, the + menu, slot label), `plugin_choices`, `apps/viewer/src/layout/sampling.rs` (readout).

**Interfaces — Produces:** `analysis::meta_from(a: &Analysis, file: &str, name: &str, keep: Option<&ZoneMeta>) -> ZoneMeta` (Auto rule; a kept meta's locked root and Manual bpm win); `analysis::Jobs` (`want(path)`, `poll() -> Vec<(PathBuf, Result<Analysis, String>)>`; one worker thread; a path asked twice is queued once; failures remembered); `App` asks for every New Sampler zone whose `meta` is None after each edit, and writes results with `note_zone_meta` to every zone still holding that file. "New Sampler" in the + menu after "Sampler" (adds a Sampler, then marks it, one step "Add New Sampler"); the chain shows "New Sampler". The Sampling page shows, for a New Sampler zone, "120.00 BPM · 16 beats · Root A2 −12 ct · SYNC" (or "Analysing…").

- [ ] Tests: `the_auto_rule` (4 s loop with tempo → Sync; 2 s → Repitch; 8 s "808 Long.wav" → Repitch; no tempo → Repitch), `a_locked_root_is_kept`, `jobs_analyse_once_and_remember_failures` (tempdir wav; a missing file).
- [ ] `mise exec -- cargo test -p deckard --bin deckard analysis` PASS; commit.

### Task 5: Finish

- [ ] Roadmap: 1b built; Architecture: the New Sampler opens as a Sampler in Maschine. `docs/deckard-block.md`: v13. FEATURES #18: skeleton built.
- [ ] `mise run lint`; tests for model, doc, analysis, viewer; release build.
