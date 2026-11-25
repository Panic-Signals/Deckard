# Chop by Playing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Catch slices at the playhead, slice from the playhead, a Random slicer mode with kept slices, and Key Shift of one slice onto a new sound.

**Architecture:** The slicer's list of start frames (`SamplingState::slices`) stays what APPLY reads. Pure helpers in a new `apps/viewer/src/chop.rs` decide where a caught slice goes, where Split and Grid start, and what Random picks; `App` methods in a new `apps/viewer/src/chop_app.rs` apply them. Find learns to keep given starts (`deckard-analysis`). The MK3 page (`crates/surface/lua/pages/sampling.lua`) asks through new `SamplingCall`s.

**Tech Stack:** Rust (egui viewer, `deckard-analysis`, `deckard-doc`), Lua (surface page).

**Spec:** `docs/superpowers/specs/2026-11-18-chop-by-playing-design.md`

## Global Constraints

- Catching only in Manual mode (`slice_mode == 3`).
- Catch snaps to 0.5 beats (a 1/8 note) of the zone's `BeatMap`; free with SHIFT.
- Random's and Find's counts are `FIND_COUNTS` (4, 8, 16, 32).
- Nothing new is saved in the project.
- Key Shift: one zone, keys 0..=127, root 60, on the group's first sound with no slots after none found → status line, no change.
- Run with `mise exec -- cargo …`; tests play audio (allowed).

## Review Focus

- Catch with no playhead, or a playhead outside the slicing range: nothing happens.
- A kept slice outside the range after the range changes: it is dropped, not kept forever.
- Random with fewer candidates than the count: as many as there are, no panic, no loop.
- Key Shift with no empty sound in the group: a status message, the document untouched.
- A re-roll with every slice kept: the slices stay as they are.

---

### Task 1: Find keeps given starts; the pure chop helpers

**Files:**
- Modify: `crates/analysis/src/find.rs`, `crates/analysis/src/lib.rs`
- Create: `apps/viewer/src/chop.rs` (declared in `apps/viewer/src/main.rs` beside `mod beat;`)

**Interfaces (produces):**
- `deckard_analysis::find_samples_keeping(frames: &[[f32; 2]], rate: u32, range: (usize, usize), lines: &[usize], count: usize, kept: &[usize]) -> Vec<usize>`: the kept starts inside `range` verbatim, plus up to `count - kept` found ones chosen apart from and unlike the kept; sorted, deduped. `find_samples` calls it with `&[]`.
- `chop::catch_at(playhead: usize, map: Option<&BeatMap>, free: bool, range: (usize, usize)) -> Option<usize>`
- `chop::slicer_range(range: (usize, usize), playhead: Option<usize>, selection: Option<(usize, usize)>) -> (usize, usize)`
- `chop::random_picks(cands: &[usize], kept: &[usize], count: usize, seed: u64, near: usize) -> Vec<usize>`: kept plus `count - kept.len()` of the candidates farther than `near` frames from every kept one, by a splitmix64 shuffle of `seed`; sorted.
- `chop::heard(start: std::time::Instant, now: std::time::Instant, (a, b): (usize, usize), rate: u32) -> Option<usize>`

- [ ] **Step 1: failing tests.** In `find.rs`: `kept_starts_stay_and_the_rest_are_found` (drums of 16 bars, `kept = [3 * bar + 100]`, count 4 → result has 4 starts, contains `3 * bar + 100`, none other within a second of it; `kept` outside the range is left out; 5 kept with count 4 → those 5). In `chop.rs`: `a_caught_slice_snaps_to_an_eighth` (map 120 bpm at 48 kHz: playhead 13_000 → 12_000; free → 13_000; playhead outside range → None; at `range.0` exactly → None, the first slice is there already), `the_slicer_starts_at_the_playhead_or_the_selection` ((0,1000) + playhead 300 → (300,1000); selection (200,400) → (200,1000); selection (0,400) or outside → (0,1000); playhead outside → falls to selection), `random_picks_keep_and_fill` (same seed same picks, another seed differs for 32 candidates, kept present, len == count, candidates within `near` of a kept one not picked, 3 candidates count 8 → 3), `the_heard_place_moves_with_the_clock`.
- [ ] **Step 2:** run `mise exec -- cargo test -p deckard-analysis find` and `mise exec -- cargo test -p deckard-viewer chop::` → FAIL (not defined).
- [ ] **Step 3:** implement. In `find_samples_keeping` the kept frames become candidates of their own, exempt from the level floor, taken before the scoring loop; their returned frame is the kept frame itself.
- [ ] **Step 4:** run both → PASS.
- [ ] **Step 5:** commit "Find keeps given starts; where a caught slice goes".

### Task 2: The slicer: Random, kept slices, from the playhead, catching

**Files:**
- Modify: `apps/viewer/src/layout/sampling.rs` (state, acts, consts), `apps/viewer/src/main.rs` (`sampling_act` arms, `compute_slices`, `update_sampling`, `sampling_call` Prehear), `apps/viewer/src/sampling_hw.rs` (mode tables to six, `Slice.Random` knob, `Apply::RandomCount`)
- Create: `apps/viewer/src/chop_app.rs` (`impl App`), tests in `apps/viewer/src/chop_tests.rs`

**Interfaces:**
- Consumes Task 1's functions.
- Produces: `SLICE_MODES: [&str; 6]` ending `"Random"`; `SamplingState { kept: Vec<usize>, random: usize (default 2), seed: u64 }`; `SamplingAct::{Catch(bool), RandomCount(usize), Reroll, KeepSlice(usize)}` (`KeepSlice(i)` toggles slice `i`; only in modes 4 and 5); `App::prehear_from: Option<(Instant, usize, usize)>`; `App::catch_slice(&mut self, free: bool)`; `App::slice_candidates(&self, range, count) -> Vec<usize>`.

- [ ] **Step 1: failing tests** (`chop_tests.rs`, with the `app()` helper pattern of `surface_sampling_tests.rs`, made `pub(crate)` there and reused):
  - `catching_drops_a_slice_at_the_playhead_in_manual_mode`: mode 3, `sampling.playhead = Some(20_000)`, `Catch(true)` → slices `[0, 20_000]`; again same place → unchanged; mode 0 → `Catch` changes nothing; playhead `None` → nothing.
  - `split_starts_at_the_playhead`: playhead 11_025, `SliceMode(0)`, `SplitCount` of 4 → first slice 11_025, 4 slices.
  - `random_rerolls_and_keeps`: `SliceMode(5)` → up to 16 slices; `KeepSlice(1)`; note frame; `Reroll` twice → frame still in slices and in `kept`; `RandomCount(0)` → 4 slices (or fewer) containing it; `MoveSlice(i, f)` of the kept one moves its mark; `RemoveSliceAt` in it removes the mark; `SliceMode(0)` clears `kept`.
  - `a_slice_pad_gives_a_playhead`: `sampling_call(Prehear(0, 44_100))` then `update_sampling()` → `sampling.playhead.is_some()`.
- [ ] **Step 2:** run `mise exec -- cargo test -p deckard-viewer chop_tests` → FAIL.
- [ ] **Step 3:** implement. `compute_slices`: modes 0 and 1 use `chop::slicer_range(r, playhead, selection)`; mode 4 calls `find_samples_keeping` with the kept in range; mode 5 = `random_picks(slice_candidates(r, count), kept, count, seed, rate / 25)`, `[r.0]` when empty; any mode but 4 and 5 clears `kept`; `kept` is retained to frames inside `r`. `slice_candidates`: Find's lines (as mode 4 builds them) through `find_samples(.., usize::MAX)`. `update_sampling`: when the zone has no position, `sampling.playhead = chop::heard(..)` from `prehear_from` (cleared once past its end). `ClearSlices` clears `kept`; `set_sample` on another file clears it.
- [ ] **Step 4:** run → PASS; `mise exec -- cargo test -p deckard-viewer sampling` → PASS.
- [ ] **Step 5:** commit "The slicer catches slices at the playhead, picks at random and keeps slices".

### Task 3: Key Shift

**Files:**
- Modify: `crates/doc/src/sounds.rs` (`SoundClip::without_notes(self) -> SoundClip`), `apps/viewer/src/chop_app.rs`, `apps/viewer/src/layout/sampling.rs` (`SamplingAct::KeyShift(usize)`), `apps/viewer/src/main.rs` (arm), tests in `apps/viewer/src/chop_tests.rs`

**Interfaces (produces):** `App::key_shift(&mut self, slice: usize)`; `SamplingAct::KeyShift(usize)`.

- [ ] **Step 1: failing tests:** `key_shift_puts_a_slice_on_the_next_empty_sound`: slices `[0, 11_025, 22_050]` (Manual), `KeyShift(1)` → the group's first formerly-empty sound has a Sampler with one zone: `play_range == (11_025, 22_049)`, `key_range == (0, 127)`, `root_key == 60`, `new_sampler.is_some()`, no notes of it in any pattern, focus unchanged, status names the pad; one `undo()` removes it. `key_shift_of_a_sync_zone_plays_in_keys`: source sound a New Sampler whose zone meta is Sync with a bpm → the new zone's meta mode is `Keys`, same bpm. `key_shift_without_an_empty_sound_says_so`: every sound given a Sampler → status set, document's undo label unchanged.
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3:** implement as one `transaction("Key Shift", …)`: `copy_sound(g, s)?.without_notes()`, `paste_sounds(g, target, …)`, `set_zones(g, target, [ZoneSpec { from: Zone(focused zone), set: play range, key 0..127, root 60, velocity 0..127, loop off }])`, `set_new_sampler(.., Some(&existing or default))`, `set_zone_meta` with `mode: Keys` when the source meta is Sync (else the meta as it is, when there is one), rename to "<name> keys".
- [ ] **Step 4:** run → PASS.
- [ ] **Step 5:** commit "Key Shift: one slice over all the keys of a new sound".

### Task 4: The SLICE tab in the software

**Files:**
- Modify: `apps/viewer/src/layout/sampling.rs` (`slice_tab`, `wave`), tests beside the existing layout tests using the egui harness (`apps/viewer/src/chop_tests.rs`)

- [ ] **Step 1: failing tests** (harness, as `the_slice_tab…` tests in `main.rs` do): `the_slice_tab_shows_cue_and_reroll`: SLICE tab, Manual → a "CUE" label is drawn; Random → "RE-ROLL" and a "SAMPLES" field; pressing Enter in Manual with a playhead adds a slice.
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3:** implement: **CUE** tool button after DELETE ALL (enabled: Manual and a playhead; SHIFT-click free), Enter does the same while the tab shows and no text field has focus; mode 5 footer: SAMPLES choice (`RandomCount`) and a **RE-ROLL** tool button; a kept slice's marker has a filled square at its top; right-click inside the waveform on SLICE opens a menu for the slice under the pointer: "Keep slice" / "Don't keep slice" (modes 4, 5) and "Key Shift to new sound".
- [ ] **Step 4:** run → PASS.
- [ ] **Step 5:** commit "SLICE: CUE, RE-ROLL, kept slices and Key Shift in the software".

### Task 5: The MK3 page

**Files:**
- Modify: `crates/lua-host/src/state.rs` (`SamplingCall::{CatchSlice(bool), KeepSlice(usize), KeyShift(usize), Reroll}`), `crates/surface/src/api.rs` (`"catch_slice"` (a = 1: free), `"keep_slice"`, `"key_shift"`, `"reroll"`; view field `kept`), `crates/model/src/sampling_view.rs` (`SampleView::kept: Vec<usize>`, slice indices), `apps/viewer/src/sampling_hw.rs`, `apps/viewer/src/main.rs` (`sampling_call` arms), `crates/surface/lua/pages/sampling.lua`, `crates/surface/src/sampling_tests.rs`, `docs/sampling-controller.md`, the roadmap's Phase 3 status.

- [ ] **Step 1: failing test** `the_slice_pads_catch_keep_and_key_shift`: mode knob text "Manual", `playhead: Some(5000)`: pad 9 → `[SamplingCall(CatchSlice(false))]`, with SHIFT → `CatchSlice(true)`; no playhead → nothing; mode "Split" → nothing. Mode "Find": SHIFT + pad 2 → `KeepSlice(1)` (no prehear). SHIFT held: display 5 reads "KEY SHIFT" → `KeyShift(1)` for the focused slice; mode "Random": display 6 reads "RE-ROLL" → `Reroll`.
- [ ] **Step 2:** run `mise exec -- cargo test -p deckard-surface sampling` → FAIL.
- [ ] **Step 3:** implement; kept slices' pads are lit bright; Random's page 1 shows `Slice.Random`.
- [ ] **Step 4:** run → PASS; then `mise exec -- cargo test --workspace --no-fail-fast` (expected: only `tests::channel_properties` fails, as on main), `mise run lint`, `scripts/check-platforms.sh`.
- [ ] **Step 5:** commit "MK3: catch slices on empty pads, keep them, Key Shift, RE-ROLL".
