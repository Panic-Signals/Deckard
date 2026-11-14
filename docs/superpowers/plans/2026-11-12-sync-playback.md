# Sync and Keys playback, rendered (roadmap 1c) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** a New Sampler zone in Sync plays in time with the song at any tempo, its Tune
moving pitch and not length; in Keys, notes transpose it and keep its length.

**Architecture:** the Rendered stretch engine. Off the audio thread, Signalsmith stretches a
zone's play range to the song's tempo with Tune as pitch (`render::stretched`); the engine
builder (`Engine::new`) swaps that render in for the zone's sample when the
`SampleCache`'s render store has it, and otherwise asks for it and plays what it has. A
voice of a Sync zone reads at `song tempo now / tempo its sample is in time at`, so before
the render arrives (or while the tempo moves) the loop still stays in time, repitched. Keys
asks for one render per note offset played (±24) and falls back to the zone's own render
(transposed by rate, so its length moves) until it arrives. The app runs the render jobs
on a thread and rebuilds the engine when one finishes.

**Tech Stack:** Rust; crates engine (+ `deckard-stretch`), doc, apps/viewer.

**Spec:** `docs/superpowers/plans/2026-11-11-sampler-roadmap.md` (1c, "Root and tuning").

## Global Constraints

- No allocation, locking or stretching on the audio thread.
- Repitch zones, the Sampler and the Audio plug-in sound exactly as before.
- In Sync and Keys the length never changes with Tune or the note.
- The Live stretch engine is not in this plan (it falls back to Rendered).

## Review Focus

1. A tempo change while a Sync loop plays: it stays in time at once (repitched), then the new render swaps in (Task 2 `sync_voices_follow_the_tempo`).
2. A zone without a tempo, or a Repitch zone, in a New Sampler: plays as before (Task 3 `repitch_zones_are_untouched`).
3. Renders pile up (many tempos, many notes): the store is capped (Task 3 `the_render_store_is_capped`).
4. A render finishing for a zone the user has since changed: no crash, the stale render is just unused (keys include the sample, range, tempo and pitch).
5. Undo of a mode change: the zone plays as before (Task 4 `set_zone_meta_is_undoable`).

---

### Task 1: Rendering

**Files:** Create `crates/engine/src/render.rs`; `lib.rs` (`pub mod render`).

**Produces:** `pub fn stretched(s: &Sample, range: (usize, usize), time: f64, semitones: f64) -> Sample`
(output length `round(time · len)`, Signalsmith, latency compensated, short fades at the
ends not added: the zone's own fades apply).

- [x] Tests: `keeps_pitch_and_changes_length` (220 Hz, time 1.25 → length ×1.25, frequency 220 ± 2 %); `moves_pitch_not_length` (+12 → 440 ± 2 %, same length); `starts_on_time` (an impulse train's first click within 2 ms of where it was × time).

### Task 2: Voices that follow the tempo

**Files:** `crates/engine/src/sampler.rs`.

**Produces:** `pub struct ZoneSync { pub tempo: f64, pub keys: bool, pub notes: Vec<(i64, Arc<Sample>)> }`;
`ZoneRt.sync: Option<ZoneSync>`. A Sync voice ignores the note (Tune is in its render); a
Keys voice plays `notes`' render for `note − root` if there, else transposes by rate.
Speed × `tempo / sync.tempo` (the song's tempo from `set_song`).

- [x] Tests: `sync_voices_follow_the_tempo` (a zone in time at 100, song at 120: reads 1.2 frames per frame; song 100: 1.0); `sync_ignores_the_note`; `keys_plays_the_note_render`.

### Task 3: The engine builds Sync zones

**Files:** `crates/engine/src/lib.rs` (`SampleCache.renders`, zone build), `render.rs` (`Renders`, `RenderJob`).

**Produces:** `pub struct RenderJob { pub key: RenderKey, pub sample: Arc<Sample>, pub range, pub time, pub semitones }`;
`SampleCache::wanted_renders(&mut self) -> Vec<RenderJob>`; `SampleCache::add_render(key, Arc<Sample>)`.
For a New Sampler zone with meta mode Sync/Keys and a bpm: target = project tempo, time =
bpm / tempo, semitones = zone tune + sound tune (stored); render key (sample ptr, range,
tempo·100, semitones·100). Have it: play the render (ranges scaled, sync.tempo = project
tempo). Else: ask, play the raw sample with sync.tempo = bpm. Keys asks the root's render
and the notes' renders from the notes it has played (`Engine::keys_wanted`, reported by
the voices): first cut asks ±12 semitones around the root eagerly, nearest first.

- [x] Tests: `a_sync_zone_asks_then_plays_its_render`; `repitch_zones_are_untouched`; `the_render_store_is_capped`.

### Task 4: The app

**Files:** `apps/viewer/src/render_jobs.rs` (new), `main.rs`, `layout/sampling.rs`, `crates/doc/src/new_sampler.rs`.

**Produces:** `Document::set_zone_meta(g, s, z, &ZoneMeta)` (one undo step, "Play Mode");
a render worker (`want(job)`, `poll()`), results into `self.samples`, then a full engine
rebuild. On the Sampling page of a New Sampler zone: REPITCH / SYNC / KEYS buttons, and
"Tune keeps the length" / "Tune changes the speed" beside the readout.

- [x] Tests: `set_zone_meta_is_undoable`; app `a_sync_zone_gets_rendered` (end to end: the job runs, the engine rebuilds with the render).

### Task 5: Finish

- [x] Roadmap 1c: built (Rendered, Sync, Keys first cut); not yet Live, grid-quantized starts, target note / Match key, root lock UI.
- [x] `mise run lint`; engine, doc, viewer tests.
