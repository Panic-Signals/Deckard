# Per-Note Ratchet Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A note can play as 1–16 hits inside its own length, velocities flat, falling to a floor or rising to a ceiling, set per note in the window and on the MK3.

**Architecture:** `Note` carries a small `Ratchet` value. One pure function, `Ratchet::hits`, turns a note into its hits; the engine, MIDI export and all drawing use it. In memory the ratchet rides on the note's tree event as a private field (as note expression does); on save it is stripped and written to a new section of the DECKARD block.

**Tech Stack:** Rust workspace (`deckard-model`, `deckard-doc`, `deckard-engine`, `deckard-surface` with Lua pages, `deckard` viewer on egui). Run everything through `mise exec -- cargo …`; format with `mise exec -- cargo fmt`.

**Spec:** `docs/superpowers/specs/2026-11-15-note-ratchet-design.md`

## Global Constraints

- Repeats 1–16 (default 1), Shape Flat/Down/Up (default Flat), Floor 0–127 (default 0), Ceiling 0–127 (default 127).
- A note with the default ratchet sounds and exports exactly as today; a project with nothing in the DECKARD block saves with no block.
- Hit velocities are rounded and clamped to 1–127. Hit 0 is always the note's velocity.
- One probability roll per note: its hits play or drop together.
- No new MK3 button. No ratchet on MIDI events, audio cues or live Note Repeat.
- Clean room: nothing from NI's code. Comments say what the code does, in the repo's plain style.
- Engine tests play audio on the user's machine: run only the named tests, never the whole workspace, unless the user says tests are fine.
- Commit messages end with:
  `Co-Authored-By: Claude Fable 5.2 <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01BtPYgGmp4tyz6P8hgvvDjL`

## Review Focus

1. A ratcheted note hanging over the pattern's end (looping): hits at or past the end must not fire into the next pass. (Task 3 test.)
2. Copy/paste, duplicate, double, quantize, nudge, transpose, undo/redo of a ratcheted note keep its ratchet. (Task 2 test.)
3. A 3-tick note with 16 repeats: 3 hits, no zero-length hit, Up still ends on the ceiling. (Task 1 test.)
4. Opening a version 12 project, and one with no block: no ratchets, no error; saving a ratchet-free project that had no block writes no block. (Task 2 test.)
5. A selection mixing Flat, Down and Up notes on the MK3: knob 4 reads TARGET and moves only the Down floors and Up ceilings. (Task 6 test.)

---

### Task 1: The ratchet value and its hits (`deckard-model`)

**Files:**
- Create: `crates/model/src/ratchet.rs`
- Modify: `crates/model/src/lib.rs` (`pub mod ratchet;`, `Note`, the note reader near where `expression::EXPR_FIELD` is read)
- Modify (mechanical): every `Note { … }` literal that names all fields (about 200, in `apps/viewer`, `crates/doc`, `crates/engine`, `crates/lua-host`, `crates/surface`)

**Interfaces — Produces:**

```rust
// crates/model/src/ratchet.rs
pub const FIELD: &str = "deckard:ratchet";          // private field on a note's tree event
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Shape { #[default] Flat, Down, Up }        // Shape::ALL, label(): "Flat" | "Down" | "Up"
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ratchet { pub repeats: u8, pub shape: Shape, pub floor: u8, pub ceiling: u8 }
impl Default for Ratchet                             // { 1, Flat, 0, 127 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit { pub offset: i64, pub length: i64, pub velocity: i64 }
impl Ratchet {
    pub fn is_plain(self) -> bool;                   // repeats <= 1
    pub fn hits(self, length: i64, velocity: i64) -> Vec<Hit>;
    pub fn to_int(self) -> i64;                      // repeats | shape<<8 | floor<<16 | ceiling<<24
    pub fn from_int(v: i64) -> Ratchet;              // clamps each part into range
    pub fn target(self) -> Option<u8>;               // Down: floor, Up: ceiling, Flat: None
    pub fn with_target(self, v: u8) -> Ratchet;      // sets floor (Down) or ceiling (Up); Flat unchanged
}
// crates/model/src/lib.rs
pub struct Note { …, pub ratchet: ratchet::Ratchet }
```

- [ ] **Step 1: Write the failing tests** in `ratchet.rs`:

```rust
fn r(repeats: u8, shape: Shape, floor: u8, ceiling: u8) -> Ratchet { Ratchet { repeats, shape, floor, ceiling } }
fn vels(h: &[Hit]) -> Vec<i64> { h.iter().map(|h| h.velocity).collect() }

#[test]
fn a_plain_note_is_one_hit_of_itself() {
    assert_eq!(Ratchet::default().hits(240, 100), [Hit { offset: 0, length: 240, velocity: 100 }]);
}
#[test]
fn the_hits_split_the_length_evenly_and_add_up_to_it() {
    let h = r(4, Shape::Flat, 0, 127).hits(960, 90);
    assert_eq!(h.iter().map(|h| (h.offset, h.length)).collect::<Vec<_>>(), [(0, 240), (240, 240), (480, 240), (720, 240)]);
    let h = r(3, Shape::Flat, 0, 127).hits(100, 90);
    assert_eq!(h.iter().map(|h| h.length).sum::<i64>(), 100);
    assert_eq!(h[2].offset + h[2].length, 100);
}
#[test]
fn down_falls_toward_the_floor_and_up_ends_on_the_ceiling() {
    assert_eq!(vels(&r(4, Shape::Down, 0, 127).hits(960, 100)), [100, 75, 50, 25]);
    assert_eq!(vels(&r(4, Shape::Down, 60, 127).hits(960, 100)), [100, 90, 80, 70]);
    assert_eq!(vels(&r(4, Shape::Up, 0, 127).hits(960, 67)), [67, 87, 107, 127]);
    assert_eq!(vels(&r(3, Shape::Up, 0, 100).hits(960, 40)), [40, 70, 100]);
}
#[test]
fn a_target_on_the_wrong_side_is_flat_and_no_hit_is_silent() {
    assert_eq!(vels(&r(3, Shape::Down, 120, 127).hits(960, 100)), [100, 100, 100]);
    assert_eq!(vels(&r(3, Shape::Up, 0, 20).hits(960, 100)), [100, 100, 100]);
    assert!(vels(&r(16, Shape::Down, 0, 127).hits(960, 2)).iter().all(|&v| (1..=127).contains(&v)));
}
#[test]
fn a_note_shorter_than_its_repeats_plays_a_hit_a_tick() {
    let h = r(16, Shape::Up, 0, 127).hits(3, 27);
    assert_eq!(h.iter().map(|h| (h.offset, h.length)).collect::<Vec<_>>(), [(0, 1), (1, 1), (2, 1)]);
    assert_eq!(vels(&h), [27, 77, 127]);
    assert_eq!(r(4, Shape::Flat, 0, 127).hits(0, 90).len(), 1);
}
#[test]
fn it_packs_into_one_number_and_back() {
    let x = r(7, Shape::Up, 12, 99);
    assert_eq!(Ratchet::from_int(x.to_int()), x);
    assert_eq!(Ratchet::from_int(0).repeats, 1);
    assert_eq!(Ratchet::from_int(200 | 9 << 8).repeats, 16);
    assert_eq!((x.target(), x.with_target(50).ceiling, r(2, Shape::Down, 5, 9).with_target(50).floor, r(2, Shape::Flat, 5, 9).with_target(50)), (Some(99), 50, 50, r(2, Shape::Flat, 5, 9)));
}
```

- [ ] **Step 2:** Run `mise exec -- cargo test -p deckard-model ratchet` — expect compile failure.
- [ ] **Step 3: Implement `ratchet.rs`.** `hits`: `n = repeats.clamp(1,16).min(length.max(1))`; hit k offset `k*length/n`, length to the next offset (last: to `length`); Down `v + (floor.min(v) - v) * k / n`, Up `v + (ceiling.max(v) - v) * k / (n-1)` (n = 1: v), rounded, clamped 1–127.
- [ ] **Step 4: Add `ratchet` to `Note`** and read it where the model reads a note's `EXPR_FIELD` from its event object (`FIELD` as an Int through `from_int`; absent: default). Then fix every full `Note { … }` literal by inserting `ratchet: Default::default(), ` (a scripted replace of `Note { expr:` → `Note { ratchet: Default::default(), expr:` covers most; fix the rest by the compiler's list). Literals using `..n` need nothing.
- [ ] **Step 5:** Run `mise exec -- cargo test -p deckard-model` and `mise exec -- cargo build --workspace --all-targets` — expect pass and a clean build.
- [ ] **Step 6: Commit** `Model: a note's ratchet (repeats, shape, floor, ceiling) and its hits`.

### Task 2: Keeping and saving it (`deckard-doc`)

**Files:**
- Modify: `crates/doc/src/notes.rs` (the note reader ~line 273 and writer ~line 358), `crates/doc/src/expression.rs` (`set_expr_field`'s sibling, `strip_deckard_fields`), `crates/doc/src/block.rs`, `docs/deckard-block.md`
- Test: `crates/doc/src/block.rs` tests, `crates/doc/tests/notes.rs`

**Interfaces:**
- Consumes: `deckard_model::ratchet::{FIELD, Ratchet}`, `Note.ratchet`.
- Produces: `pub(crate) fn set_ratchet_field(t: &mut Obj, r: Ratchet)` (removes the field when `r` is default); block `VERSION = 13` with `Block.ratchets: Vec<RatchetEntry>`, `pub(crate) struct RatchetEntry { group: u32, pattern: u32, sound: u32, position: i64, pitch: i32, ordinal: u32, ratchet: u32 }`.

Block section, appended after the Audio offsets: `u32 count`, then per entry `u32 group, u32 pattern, u32 sound, i64 position, i32 pitch, u32 ordinal, u32 ratchet` (`Ratchet::to_int`). Only non-default ratchets are written. Decoding versions 1–12 gives no entries. On load, each entry sets the field on the matching note event, found as expression entries are (`with_ordinals`). Clip patterns use `2^24 + k` as elsewhere in the block.

- [ ] **Step 1: Write the failing tests.**

```rust
// block.rs tests
#[test]
fn a_ratchet_round_trips_through_a_save() {
    // new project, add note A (default) and note B { ratchet: Ratchet { repeats: 4, shape: Shape::Down, floor: 20, ceiling: 127 } }
    // at the same sound/position/pitch as a third plain note C placed first (ordinal 1 for B)
    // to_bytes → Document::from_bytes → the pattern's notes equal the originals, B's ratchet intact, A and C default.
}
#[test]
fn no_ratchet_no_block_and_an_older_block_reads() {
    // new project with one plain note: to_bytes has no DKRD item (as `no_expression_no_block` checks).
    // a payload encoded with VERSION 12 layout (encode, then truncate the ratchet section and patch the version to 12) decodes with ratchets empty.
}
// tests/notes.rs
#[test]
fn edits_keep_a_notes_ratchet() {
    // a ratcheted note through change_note (move), edit_notes (remove+add copy at +960), duplicate/double of the pattern,
    // the note_tools quantize and transpose helpers, then undo and redo: every resulting copy still has the ratchet;
    // after undo to the start the original has it.
}
```

Write them out fully against the helpers those test modules already have (`doc_with`, `note(…)`).

- [ ] **Step 2:** Run `mise exec -- cargo test -p deckard-doc ratchet` — expect failures.
- [ ] **Step 3: Implement.** Reader: `ratchet` from the event's `FIELD`. Writer: `set_ratchet_field` beside `set_expr_field`. `strip_deckard_fields` removes `FIELD` and `block_of` collects the entries. `encode`/`decode` gain the section; bump `VERSION` to 13 and update the version checks and the existing "block bytes" tests the bump moves.
- [ ] **Step 4: Update `docs/deckard-block.md`:** "Payload, version 13", the version paragraph, and the new section's layout.
- [ ] **Step 5:** Run `mise exec -- cargo test -p deckard-doc` — expect pass.
- [ ] **Step 6: Commit** `Doc: a note's ratchet survives edits and saves in the DECKARD block (version 13)`.

### Task 3: Playing it (`deckard-engine`)

**Files:**
- Modify: `crates/engine/src/lib.rs` (`NoteEvent`, the pattern-notes loop ~line 566, `chance` call sites ~3624 and ~5031)
- Test: `crates/engine/src/lib.rs` tests

**Interfaces:**
- Consumes: `Ratchet::hits`.
- Produces: `NoteEvent.roll: Option<u64>` — when `Some(k)`, `chance` is keyed by `k` instead of the event's index. Each ratcheted note instance in a timeline gets one `k` (`(1 << 40) + a counter`), shared by its hits. Every other `NoteEvent` literal gets `roll: None`.

Expansion, in the notes loop: a plain note is pushed as now. A ratcheted one pushes an event per hit with `tick = swung(p, g, n.sound, start + rep + pos) + hit.offset` (swing on the note's start only), `velocity = hit.velocity`, `length = hit.length`, and skips a hit when `pos + hit.offset >= plen` or `rep + pos + hit.offset >= length`. `expr` goes on every hit unchanged.

- [ ] **Step 1: Write the failing tests** (beside the existing timeline tests, using their project helpers):

```rust
#[test]
fn a_ratcheted_note_plays_its_hits() {
    // one 1/4 note (length PPQ) at 0, velocity 100, Ratchet { repeats: 4, shape: Down, floor: 0, ceiling: 127 }:
    // the timeline's note events for it are at ticks 0, PPQ/4, PPQ/2, 3*PPQ/4, lengths PPQ/4, velocities [100, 75, 50, 25], one shared Some(roll).
    // the same note with the default ratchet gives one event equal to today's (roll None).
}
#[test]
fn hits_past_the_patterns_end_do_not_play() {
    // a 1-bar pattern; a note starting one 1/4 before the end, length 2 × PPQ, 4 repeats (hits every PPQ/2):
    // only the hits at offsets 0 and PPQ/2 are in the timeline.
}
#[test]
fn a_ratchets_hits_take_one_roll() {
    // probability 50, 8 repeats: over passes 0..64, for each pass chance(...) with the events' roll gives the same answer for all 8 hits,
    // and both answers occur across the passes.
}
```

- [ ] **Step 2:** Run `mise exec -- cargo test -p deckard-engine --lib ratchet` (and the three names) — expect failures.
- [ ] **Step 3: Implement** as above; at both `chance` call sites pass `e.roll.map_or(idx, |k| k as usize)` as the key.
- [ ] **Step 4:** Run `mise exec -- cargo test -p deckard-engine --lib` — expect pass (plays audio: only with the user's leave).
- [ ] **Step 5: Commit** `Engine: a ratcheted note plays as its hits, on one roll`.

### Task 4: Export MIDI (`apps/viewer/src/midi_file.rs`)

**Files:** Modify `apps/viewer/src/midi_file.rs` (`track_events` ~line 68) and its tests.

- [ ] **Step 1: Failing test** `a_ratchet_exports_as_its_hits`: export one note (position 0, length 480, pitch 60, velocity 100, ratchet 4 × Down to 0) with the test module's existing export-then-`import` round trip; the imported notes are four, at 0/120/240/360, length 120, velocities [100, 75, 50, 25], all with the default ratchet.
- [ ] **Step 2:** Run `mise exec -- cargo test -p deckard midi_file` — expect failure.
- [ ] **Step 3:** In `track_events`, write a note-on/off pair per `n.ratchet.hits(n.length, n.velocity)`.
- [ ] **Step 4:** Run it again — pass. **Commit** `Export MIDI writes a ratchet's hits`.

### Task 5: The window (`apps/viewer/src/layout.rs`, `main.rs`)

**Files:**
- Modify: `apps/viewer/src/layout.rs` — `LaneKind`/`Lane` (~7490–7560: add `Ratchet`, name "Ratchet", `get` = repeats, `set` = repeats, range 1–16), the lane picker list, note drawing in the drum grid and piano roll, the Velocity lane's stems, the note context menu
- Modify: `apps/viewer/src/main.rs` — an `Act` for the menu
- Test: `apps/viewer/src/main.rs` tests (the egui_kittest harness tests near `copy_paste_and_the_event_menu`)

**Interfaces:**
- Produces: `Act::Ratchet(RatchetEdit)` with `pub enum RatchetEdit { Repeats(u8), Shape(Shape), Floor(u8), Ceiling(u8) }`, applied to the selected notes (the right-clicked note if it isn't selected) in one undo step named "Ratchet" via `edit_notes`.

Drawing: a ratcheted note gets a 1 px tick in `black(120)` at each hit's start after the first, in both views. In the Velocity lane a ratcheted note draws a stem per hit at the hit's x and velocity; the first as now, the rest at half opacity and not draggable. The Ratchet lane's bar is `repeats/16` of the lane's height with the number beside it when it's above 1.

Menu, under the note menu's existing items: `Ratchet ▸` with a repeats row (`DragValue` 1–16), `Flat / Down / Up` as radio items, and `Floor` and `Ceiling` `DragValue`s 0–127, both always shown.

- [ ] **Step 1: Failing tests:**

```rust
#[test]
fn the_ratchet_lane_and_menu_set_a_notes_ratchet() {
    // project with two selected notes; apply Act::Ratchet(Repeats(4)), then Shape(Down), then Floor(30):
    // both notes read Ratchet { repeats: 4, shape: Down, floor: 30, ceiling: 127 }; one undo step each, undo ×3 restores defaults.
    // Lane::Ratchet: lane value of a note is its repeats; setting 99 clamps to 16 and 0 to 1.
}
```

- [ ] **Step 2:** Run `mise exec -- cargo test -p deckard the_ratchet_lane` — expect failure.
- [ ] **Step 3: Implement** the lane, the act, the menu, and the drawing.
- [ ] **Step 4:** Run the test, then render and look: extend `desktop`-style shot test in `apps/viewer/src/manual_shots.rs` with `ratchet_shots` (ignored): a pattern with a 1/4 note at 4 × Down and the Ratchet lane open, saved as `ratchet-window.png`; open it with Read and check ticks, stems and lane.
- [ ] **Step 5: Commit** `Pattern editor: a Ratchet lane, a note's Ratchet menu, and its hits drawn`.

### Task 6: The MK3 (`crates/surface`, `apps/viewer/src/surface_events.rs`)

**Files:**
- Modify: `crates/surface/src/api.rs` (note tables ~1235: add `ratchet`, `ratchet_shape` (0–2), `ratchet_floor`, `ratchet_ceiling`; the `deckard.events` name list ~771: add `"ratchet"`, `"ratchet_shape"`, `"ratchet_target"`), the `View`'s note type if it lists fields
- Modify: `crates/surface/lua/lib/notes.lua` (`notes.ratchet_params(s)`, `notes.ratchet_knob(n, steps)`), `pages/step.lua`, `pages/step_grid.lua`, `pages/events.lua` (knob page 2), `lib/pattern_editor.lua` (ticks)
- Modify: `apps/viewer/src/surface_events.rs` (~line 91, beside `"probability"`)
- Test: `crates/surface/src/tests.rs`, `apps/viewer/src/surface_events_tests.rs`

**Interfaces:**
- Consumes: `Note.ratchet`, `Ratchet::{target, with_target}`.
- Produces (viewer, for `Action::Events(name, n)`):
  - `"ratchet"`: repeats `+ n`, clamped 1–16.
  - `"ratchet_shape"`: shape index `+ n`, clamped Flat..Up (no wrap).
  - `"ratchet_target"`: `with_target(target + n)` clamped 0–127 on Down and Up notes; Flat notes untouched.

Page 2 params: `{ PROBABILITY, RATCHET, SHAPE, <fourth> }`. RATCHET's value is the count, SHAPE's "Flat"/"Down"/"Up", both through `notes.value` so mixed selections show as other mixed values do. The fourth is `FLOOR` when every selected note is Down, `CEILING` when every one is Up, absent when all are Flat or nothing is selected, else `TARGET` with the mixed-value text. Knobs 2–4 on page 2 call `notes.ratchet_knob`; knob 1 stays probability. The pattern editor draws a ratcheted note's hit ticks in the theme's `on_colour`.

- [ ] **Step 1: Failing tests:**

```rust
// crates/surface/src/tests.rs, beside the knob-page-2 probability test (~line 1750)
// on Step and Events pages, page 2: Knob(1) → [Action::Events("ratchet", 1)], Knob(2) → [Action::Events("ratchet_shape", 1)],
// Knob(3) → [Action::Events("ratchet_target", 1)]; the drawn params read "RATCHET" "4", "SHAPE" "Down", "FLOOR" "20" for a selected
// note with that ratchet, "CEILING" for an Up note, no fourth name for Flat, "TARGET" for a Down + Up selection.

// apps/viewer/src/surface_events_tests.rs
// three selected notes Flat / Down(floor 10) / Up(ceiling 100): Events("ratchet", 3) → repeats 4 each;
// Events("ratchet_target", 5) → floors/ceilings (0 untouched, 15, 105); Events("ratchet_shape", 1) → Down, Up, Up;
// Events("ratchet", -99) → repeats 1. Each is one undo step.
```

- [ ] **Step 2:** Run `mise exec -- cargo test -p deckard-surface ratchet` and `mise exec -- cargo test -p deckard ratchet` — expect failures.
- [ ] **Step 3: Implement** the state fields, names, Lua params/knobs/ticks, and the viewer handlers.
- [ ] **Step 4:** Run both again, then `mise exec -- cargo test -p deckard-surface`; render the screens in `ratchet_shots` (Task 5) with the `mk3(...)` helper after pressing to the Step page's knob page 2, save `mk3-ratchet.png`, and look at it.
- [ ] **Step 5: Commit** `MK3: RATCHET, SHAPE and FLOOR / CEILING on the Step and Events pages' second knob page`.

### Task 7: Wrap up

- [ ] `mise exec -- cargo fmt`, `mise run lint`, `mise exec -- cargo clippy --workspace --all-targets` — clean.
- [ ] `mise exec -- cargo test -p deckard-model -p deckard-doc -p deckard-surface -p deckard` and the engine's `--lib` tests (with the user's leave for audio). `tests::channel_properties` already fails on `main`; anything else must pass.
- [ ] Note in the pull request's **Manual** section that `differences-from-maschine.md` and `maschine-mk3.md` need a Ratchet section (non-Maschine feature, new knob page).
- [ ] Commit any leftovers.
