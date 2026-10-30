# Recording capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Live notes are written at note-on and grow while held, by a clock that never
wraps; count-in pickup; no doubles; same-pad retrigger ends the earlier note.

**Architecture:** The engine adds `played` (never-wrapping ticks) to `Recorded`,
`RecordedExpr` and the published telemetry, and records the count-in's last 240 ticks.
`record::Take` becomes the pure decision-maker: it takes events and the clock and
returns edits (`Edit { g, pi, remove: Vec<Note>, add: Note }`) the app applies with
`edit_notes` inside the take.

**Tech Stack:** Rust (crates engine; apps/viewer).

**Spec:** `docs/superpowers/specs/2026-10-25-recording-capture-design.md`.

## Global Constraints

- Clean room: behaviour only from the binary; no NI code.
- Cargo through mise; targeted tests while building; lint and the full suite at the end.
- A take stays one undo step.

## Review Focus

1. A pad held across the loop end: one note, its length the time held (Task 2 `a_note_held_over_the_loop_end_keeps_growing`).
2. Expression on a note held across the wrap keeps increasing offsets (Task 2 `expression_over_the_wrap`).
3. A note quantized onto the loop end lands on step 1 (Task 2 `quantized_onto_the_end_wraps_to_the_start`).
4. Count-in: last 1/16 recorded at the start, earlier not (Task 1).
5. The take is one undo step after many growth edits (Task 3).

---

### Task 1: The engine: the never-wrapping clock and count-in pickup

**Files:** `crates/engine/src/lib.rs`, `crates/engine/src/output.rs` (telemetry accessor).

**Produces:** `Recorded.played: f64`, `RecordedExpr.played: f64`, `Telemetry.played: f64` (`time / samples_per_tick`); `const PICKUP: i64 = 240`.

- [ ] Tests: `played_keeps_counting_over_the_loop`, `the_count_ins_last_sixteenth_is_recorded_at_the_start`, `earlier_count_in_notes_are_not_recorded`.
- [ ] `mise exec -- cargo test -p deckard-engine --lib record` PASS.

### Task 2: The take: write at note-on, grow, release

**Files:** `apps/viewer/src/record.rs`.

**Produces:**
```rust
pub struct Placing { pub quantize: Option<i64> }       // Input Quantize's grid, if on
pub struct Edit { pub g: usize, pub pi: usize, pub remove: Vec<Note>, pub add: Note }
impl Take {
    pub fn add(&mut self, p: &Project, r: Recorded, at: Placing) -> Vec<Edit>;
    pub fn add_expr(&mut self, r: RecordedExpr);
    pub fn grow(&mut self, played: f64) -> Vec<Edit>;
    pub fn close(&mut self, played: f64) -> Vec<Edit>;
}
```
Expression goes on the note with its final edit (`add.expr` set by the app from the
returned expression: `Edit` carries `expr: Option<Expression>`).

- [ ] Tests: `a_note_is_written_at_note_on`, `a_held_note_grows_and_its_release_sets_its_length`, `a_note_held_over_the_loop_end_keeps_growing`, `expression_over_the_wrap`, `quantized_onto_the_end_wraps_to_the_start`, `doubles_are_removed`, `the_same_pad_again_ends_the_earlier_note`, `repeat_notes_are_not_quantized`, `notes_held_at_the_end_keep_their_length`.
- [ ] `mise exec -- cargo test -p deckard --bin deckard record::` PASS.

### Task 3: The app

**Files:** `apps/viewer/src/main.rs` (`take_in_all`, `poll_recorded` grows every 100 ms, `set_recording` closes, `write_recorded` replaced by applying `Edit`s), existing rec tests updated to the new timing.

- [ ] Tests: existing `rec_tests`, `expression_record_tests`, `auto_length_tests` green; new `a_held_pad_shows_while_held` (a held note is in the pattern before its release).
- [ ] `mise exec -- cargo test -p deckard --bin deckard rec` PASS.

### Task 4: Finish

- [ ] FEATURES.md: Replace mode (ERASE + REC). BUGS.md: entries for the reported bugs, fixed.
- [ ] Lint, full suite, release build.
