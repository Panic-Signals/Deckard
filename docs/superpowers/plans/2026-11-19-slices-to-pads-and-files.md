# Slices to pads and to files: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** A count of 64, a pad bank button on the MK3's SLICE page, APPLY to pads (new
groups, a sound per slice), a shuffled note order, and slices exported as WAV files.

**Architecture:** The slicer stays a list of start frames. Pure decisions (note order,
file names) go in `apps/viewer/src/chop.rs`; the document edits and the export go in a
new `apps/viewer/src/chop_group.rs` (`impl App`); `apply_slices` in `main.rs` branches on
the new TO setting. Tests in a new `apps/viewer/src/chop_group_tests.rs`.

**Spec:** `docs/superpowers/specs/2026-11-19-slices-to-pads-and-files-design.md`

## Global Constraints

- Nothing new saved in the project.
- APPLY to pads never changes the group being sliced; one undo step.
- Clean room; no audio hardware needed by any test.
- `mise exec -- cargo test -p deckard <filter>` (a build is ~3 minutes: batch red runs).

## Review Focus

1. More slices than fit: 64 slices to pads make four groups; 17 make two (the second with
   one sound). Test in task 2.
2. A failure part-way (no pattern to copy from, a document error) leaves no half-made
   group: the transaction rolls back. Test in task 2 by undo count / group count.
3. Export into a folder that has the files already: nothing replaced unless asked. Test
   in task 3.
4. Sample names with characters a file name can't hold (`/`, `:`). Test in task 3.
5. Shuffled order with two slices, and with one seed, is a permutation and repeatable.
   Test in task 1.

---

### Task 1: counts of 64, note order, TO and ORDER settings

**Files:** `apps/viewer/src/chop.rs`, `apps/viewer/src/layout/sampling.rs`,
`apps/viewer/src/sampling_hw.rs`, `apps/viewer/src/main.rs`,
`apps/viewer/src/chop_group_tests.rs` (new).

**Produces:**
- `chop::note_order(n: usize, shuffled: bool, seed: u64) -> Vec<usize>`: `0..n`, or a
  permutation of it by the seed (Fisher–Yates on the splitmix64 already in `random_picks`,
  factored out as `fn mix(state: &mut u64) -> u64`). Shuffled with `n >= 2` is never the
  identity (rotate by one if it comes out so).
- `SPLIT_COUNTS`, `FIND_COUNTS`: `[4, 8, 16, 32, 64]`; the `.min(3)` clamps in
  `compute_slices` become `.min(FIND_COUNTS.len() - 1)`.
- `SamplingState::{apply_to: usize /* 0 Keys, 1 Pads */, shuffle: bool}`;
  `pub const APPLY_TO: [&str; 2] = ["Keys", "Pads"]`, `NOTE_ORDERS: [&str; 2] = ["In order", "Shuffled"]`;
  `SamplingAct::{ApplyTo(usize), NoteOrder(bool), ExportSlices}`.
- Knobs `Slice.To` ("TO") and `Slice.Order` ("ORDER") with `Apply::{ApplyTo, NoteOrder}`.
- `apply_slices` (keys): note `i` keeps slot `i`'s position and plays slice `order[i]`
  (its pitch and its length).

Tests: `note_order` unit test (identity; permutation; same seed same; n = 2 swaps; n = 0,
1); `split_into_64` (Split with count 64 gives 64 slices; APPLY gives 64 zones on keys
36..=99); `shuffled_notes_keep_their_times` (Create Pattern + Shuffled: positions as in
order, pitches a non-identity permutation of 36..36+n).

### Task 2: APPLY to pads

**Files:** `apps/viewer/src/chop_group.rs` (new), `main.rs` (`mod`, the branch at the top
of `apply_slices`), `chop_group_tests.rs`.

**Produces:** `App::apply_slices_to_pads(&mut self)`, called by `apply_slices` when
`sampling.apply_to == 1`. One transaction "Apply Slices to Pads": per 16 slices
`add_group`, rename, and per slice `copy_sound(..).without_notes()` → `paste_sounds` →
`set_zones` (keys 0..=127, root 60, the slice's play range; to the range's end with EACH
PLAYS TO) → `set_new_sampler` (the source's, if it has one) → `set_zone_meta` (the zone's
own; none: Repitch with the file) → rename; the one-shot envelope fix `apply_slices`
makes; with `pattern_mode < 2`, `add_pattern` "Slices" long enough in whole bars and a
note per slice (pitch 60, sound = its pad, order by `note_order` within all slices: slot
`i`'s time, slice `order[i]`'s pad, in that slice's group). Status: "N slices are on the
pads of <group names>". Slices, tab and focus unchanged.

Tests: `seventeen_slices_make_two_groups` (group count +2, 16 + 1 sounds with slots, each
one zone with the right play range, key range (0,127), root 60; names; the sliced group's
sounds equal to before; slices and focus unchanged; one undo removes both groups);
`sixty_four_make_four`; `pads_get_a_pattern` (Create Pattern: each new group has a
pattern whose notes are at the slices' times, pitch 60, sounds 0..); `a_sync_zone_stays_sync_on_its_pad`.

### Task 3: export

**Files:** `chop.rs`, `chop_group.rs`, `layout/sampling.rs` (EXPORT button, id "export",
left of APPLY's text, enabled with ≥ 1 slice), `main.rs` (act arm), tests.

**Produces:**
- `chop::slice_file_names(stem: &str, n: usize) -> Vec<String>`: "<stem> 01.wav"…, digits
  as many as `n` needs (at least 2); characters `/ \ : * ? " < > |` and control characters
  in the stem become `_`; an empty stem is "Slice".
- `App::export_slices_to(&mut self, dir: &Path, replace: bool) -> Result<usize, String>`:
  Err naming the first file that exists when `!replace`, having written nothing; else
  writes with `deckard_engine::edit::write_wav` and returns the count. Slices as APPLY
  cuts them (`slices_from`, each to the next).
- `SamplingAct::ExportSlices`: `rfd` folder dialog, then if files exist an `rfd`
  confirm, then status "Exported N slices to <dir>".

Tests: names unit test; `export_writes_a_file_per_slice` (read back with `Sample::load`:
counts and lengths; second call without replace errs and leaves file times/sizes;
with replace succeeds).

### Task 4: the MK3 page and the docs

**Files:** `crates/surface/lua/pages/sampling.lua`, `crates/surface/src/sampling_tests.rs`,
`docs/sampling-controller.md`, the roadmap's Phase 3 note.

- Apply knob page: `{ k["Slice.To"], k["Slice.Apply"], k["Slice.Order"], k["Slice.Bpm"] }`
  (Slice.BpmMode dropped from this page; it stays on Grid's).
- SHIFT + display button 7: "BANK n/m", disabled with ≤ 16 slices; sets the selected slice
  to the first of the next bank.

Test: `the_slice_pads_bank` (40 slices: pads light 16; BANK → pads show 17..32; third
bank lights 8; round to the first), and the Apply page lists TO.
