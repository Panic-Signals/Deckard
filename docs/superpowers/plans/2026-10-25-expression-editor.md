# Expression editor (expression part 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** See and edit each note's Pitch, Slide, Pressure and Release velocity in the
pattern editor's lane, Ableton-style, with curved segments that play back as drawn.

**Architecture:** Curve editing is pure model code (`deckard_model::expression`):
`value_at` honours segment curves, and `Curve` gains point and segment edits plus a
freehand `draw`. The document gets `set_expressions` (new table ids put on the notes
through `edit_notes`, one undo step). The viewer gets `layout/expr_lane.rs`: the lane's
value scale, which note an edit goes to, hit tests and the gesture → curve logic, all
testable without egui; `layout.rs` adds `Lane::Expr(Dim)` and `Lane::Release`, draws
and wires them, and overlays pitch on piano-roll notes.

**Tech Stack:** Rust (crates model, doc; apps/viewer egui, egui_kittest).

**Spec:** `docs/superpowers/specs/2026-10-25-expression-editor-design.md`.

## Global Constraints

- Clean room: no NI or Ableton code; expression stays in the DECKARD block.
- Cargo through mise: `mise exec -- cargo …`; targeted tests while building; lint and the full suite at the end.
- Tests don't write library folders or the settings file.

## Review Focus

1. Chords on one sound (piano roll): an edit goes to one note, the selected one first (Task 3 `the_edit_goes_to_the_nearest_curve`, `selected_notes_come_first`).
2. A breakpoint dragged past its neighbour or outside the note: clamped, order kept (Task 1 `a_point_stays_between_its_neighbours`).
3. Removing a curve's last point removes the curve; an emptied expression clears the note's id (Task 1, Task 2 `clearing_everything_clears_the_id`).
4. A curved segment survives save and load, and plays back curved (Task 1 `value_at_follows_the_bend`, Task 2 `edits_round_trip_through_a_save`).
5. Undo of an edit restores the old curve and the note (Task 2 `an_edit_is_one_undo_step`).

---

### Task 1: The model: curved segments and edits

**Files:** modify `crates/model/src/expression.rs`.

**Interfaces — Produces:**
```rust
impl Dim { pub fn rest(self) -> f32 /* 0 */; pub fn name(self) -> &'static str }
impl Curve {
    pub fn value_at(&self, at: i64) -> f32;                       // now follows each segment's curve
    pub fn insert(&mut self, at: i64, value: f32) -> usize;       // replaces a point at the same tick
    pub fn move_point(&mut self, i: usize, at: i64, value: f32, len: i64);
    pub fn move_segment(&mut self, i: usize, dv: f32, lo: f32, hi: f32); // points i and i+1
    pub fn bend(&mut self, i: usize, curve: f32);                 // clamped −1..1
    pub fn remove(&mut self, i: usize);
    pub fn draw(&mut self, samples: &[(i64, f32)], tolerance: f32);// replaces samples' span
}
pub fn bent(t: f32, curve: f32, rising: bool) -> f32;
```
Segment curve: `v0 + (v1 − v0)·t^e`, `e = 4^(−c·sign(v1−v0))`.

- [ ] Tests: `value_at_follows_the_bend` (c 0 straight; c 1 rising above the line, falling above too; flat ignores c), `inserting_keeps_points_in_order`, `a_point_stays_between_its_neighbours` (time clamped strictly between neighbours, 0..=len), `moving_a_segment_moves_both_ends` (clamped lo..hi), `drawing_replaces_the_span` (points outside kept, thinned), `removing_points`.
- [ ] `mise exec -- cargo test -p deckard-model expression` PASS.

### Task 2: The document: setting expression

**Files:** modify `crates/doc/src/expression.rs`.

**Interfaces — Produces:** `Document::set_expressions(&mut self, g: usize, p: usize, edits: &[(Note, Expression)]) -> Result<(), String>`: each note gets a new id for its expression (none when `is_empty`), through one `edit_notes("Edit Expression", …)`.

- [ ] Tests: `an_edit_is_one_undo_step`, `clearing_everything_clears_the_id`, `edits_round_trip_through_a_save` (curve amounts kept).
- [ ] `mise exec -- cargo test -p deckard-doc --lib expression` PASS.

### Task 3: The viewer: lane logic

**Files:** create `apps/viewer/src/layout/expr_lane.rs`.

**Interfaces — Produces:**
```rust
pub struct Scale { pub lo: f32, pub hi: f32 }              // value at the lane's bottom and top
pub fn scale(d: Dim, pitch_range: f32) -> Scale;
pub fn fit_range(curves: impl Iterator<Item = f32>) -> f32; // 2, 12, 24 or 48
pub fn pick(notes: &[(Note, Option<&Curve>)], selected: &[Note], tick: i64, value: f32, d: Dim) -> Option<usize>;
pub enum Hit { Point(usize), Segment(usize), Empty }
pub fn hit(c: &Curve, start_x: f32, px_per_tick: f32, y_of: impl Fn(f32) -> f32, pos: Pos2) -> Hit;
pub fn readout(d: Option<Dim>, v: f32) -> String;          // None: Release
```
- [ ] Tests: `the_edit_goes_to_the_nearest_curve`, `selected_notes_come_first`, `a_note_without_a_curve_is_picked_at_its_rest`, `hits_prefer_points`, `pitch_range_fits_the_curves`, `readouts`.
- [ ] `mise exec -- cargo test -p deckard --bin deckard expr_lane` PASS.

### Task 4: The viewer: lanes drawn and wired

**Files:** modify `apps/viewer/src/layout.rs` (`Lane::Expr(Dim)`, `Lane::Release`, `lane_list`, drawing, gestures with `NoteDrag::Expr`, right-click menu, piano-roll pitch overlay, `Act::SetExpressions`), `apps/viewer/src/main.rs` (harness test).

- [ ] Harness tests: `the_pressure_lane_adds_a_point` (click in a note's span → its pressure curve has a point; undo removes it), `the_release_lane_sets_release_velocity`, update the lane-list order assertion (expression lanes after Probability).
- [ ] `mise exec -- cargo test -p deckard --bin deckard lane` PASS.

### Task 5: Finish

- [ ] PARITY row 10 (part 3 built), FEATURES if listed, spec cross-links.
- [ ] `mise run lint` PASS; `mise exec -- cargo test --workspace --no-fail-fast` all ok; release build.
