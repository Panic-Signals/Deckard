# Slice Motion, Tail and Munge: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** MOTION (a note plays the next or a random slice), TAIL (a fading reverse tail
at a slice's end) and MUNGE (swing, eighth-note mutes and splices, as an edit).

**Architecture:** Motion is a line in the sound's New Sampler field and a counter in
`SamplerRt`; Tail a line in the zone's field and a turn-round in `Stream`. Munge is a pure
function in a new `crates/engine/src/munge.rs`, opened from EDIT as a bar like STRETCH's
and stored through `store_edit`.

**Spec:** `docs/superpowers/specs/2026-11-19-motion-tail-munge-design.md`

## Global Constraints

- A project with none of the three set plays and saves as before.
- Nothing allocates on the audio thread.
- Clean room; no test needs audio hardware.
- Engine tests: `mise exec -- cargo test -p deckard-engine <filter>`; app tests
  `mise exec -- cargo test -p deckard <filter>` (~3 minutes a build: batch).

## Review Focus

1. Motion with zones that can't play (an empty play range) or with one zone: skipped, and
   one zone plays every time. Test in task 1.
2. Tail longer than the slice, and on a reversed zone (it turns forwards from the start).
   Test in task 1.
3. Munge with nothing changed gives the input back exactly, so opening the bar and
   pressing APPLY can't degrade a sample. Test in task 2.
4. Munge with every eighth off, a range shorter than an eighth, or no tempo: nothing
   happens, with a status. Test in tasks 2 and 3.
5. Text of older projects (no `motion`, no `tail`) reads as Off and 0; unknown words read
   as Off. Test in task 1.

---

### Task 1: Motion and Tail in the model and the engine

**Files:** `crates/model/src/new_sampler.rs`, `crates/engine/src/sampler.rs`,
`crates/engine/src/lib.rs`.

**Produces:**
- `pub enum Motion { Off, Next, Random }` (`Default` Off; `ALL`, `label()` "Off" / "Next"
  / "Random"); `NewSampler::motion`; text line `motion next|random` (none when Off).
- `ZoneMeta::tail: f64` (ms, 0 off, read clamped to 0..=2000); text line `tail <ms>` when
  above 0.
- `ZoneRt::tail_ms: f32`; `SamplerRt::motion: Motion` (public, set after `new`), private
  `turn: usize` and `last: usize`, both kept through `adopt`, `turn` zeroed by `fade_all`.
- `note_on` with motion on and at least one playable zone: one voice, of the playable
  zone (in order of `keys.0`, then index) at `turn % count` (Next) or drawn from `seed`,
  never `last` when there are two or more (Random); `make_voice` gets the zone's root as
  the pitch note, the played note stays the voice's `note`.
- `Stream::tail: i64` (frames; `make_voice` sets it from `tail_ms` and the sample's rate
  when the stream doesn't loop) and `tail_left`, `tail_len`: on running off the play
  range with `tail > 0`, turn round at the edge, play `min(tail, range)` frames with a
  linear fade to 0, then `done`.

Tests (engine): `motion_plays_the_slices_in_turn` (three zones on keys 36..38 of a ramp:
three notes of key 60 play the three ranges in order, the fourth the first; an empty zone
is skipped; one zone plays every time; after `fade_all` the first); `random_motion_never_repeats`
(40 notes, no two alike in a row, all three heard); `a_tail_plays_back_from_the_end_and_fades`
(ramp, play 1000..2000, tail 500 frames: frames after the end fall from the end's value
toward 0, silent after 500; tail longer than the slice stops at the slice's length; a
reversed zone's tail rises from the start); `a_project_sets_motion_and_tail` (lib.rs:
meta tail and new_sampler motion reach the `SamplerRt`). Model: round trips, and
`motion sideways` / no lines read as Off and 0.

### Task 2: the munge function

**Files:** `crates/engine/src/munge.rs` (new), `crates/engine/src/lib.rs` (`pub mod munge`).

**Produces:**
```rust
pub struct Munge { pub swing: f64 /* -1..=1 */, pub sixteenths: bool, pub mutes: [bool; 8] /* true: off */, pub grunge_ms: f64, pub comb_ms: f64 }
impl Default for Munge // 0, true, none, 5.0, 30.0
impl Munge { pub fn changes(&self) -> bool /* swing != 0 or a mute */ }
/// `cuts`: the eighths' start frames inside `r`, rising, the first `r.0`; `eighth`: an
/// eighth's even length in frames. None: nothing left to play.
pub fn munge(s: &Sample, r: (usize, usize), cuts: &[usize], eighth: f64, m: &Munge) -> Option<Sample>
```
Pieces are the eighths (halved at their middles with `sixteenths`); eighth `k` is dropped
when `mutes[k % 8]`. Piece `j` of those left gets the length `unit × (1 + swing/3)` (even
`j`) or `unit × (1 − swing/3)` (odd), `unit` the eighth or half of it, ends rounded
cumulatively; a last piece shorter than half a unit keeps its length. Shorter: cut, the
last grunge frames faded out. Longer: its last comb frames repeated to fill, each join
crossfaded over grunge (at most half the comb), the fill's level falling to 0. A piece
after one that was cut, filled or dropped fades in over 1 ms. `!m.changes()` returns the
input's frames unchanged. Audio outside `r` is kept.

Tests: `nothing_changed_is_the_input`; `swing_moves_the_off_beat` (clicks on each eighth
of an even loop: with swing 1 on 1/8 the second click is at 4/3 of an eighth, the length
unchanged; swing −1 on a loop made with +1 puts it back within a few frames);
`a_muted_eighth_shortens_the_bar` (8 eighths, one off: 7/8 of the length, the click gone,
the others on the grid); `all_off_is_none`; `a_longer_piece_is_filled_and_dies_away` (no
frame louder than the source, the last frames near 0); `outside_the_range_is_kept`.

### Task 3: MUNGE on EDIT

**Files:** `apps/viewer/src/layout/sampling.rs`, `apps/viewer/src/main.rs`,
`apps/viewer/src/chop_app.rs` (the app half), `apps/viewer/src/chop_tests.rs`.

**Produces:** `EditOp::Munge` ("MUNGE", after STRETCH; software only);
`SamplingState::munge: Option<Munge>`; `SamplingAct::{MungeOpen(bool), SetMunge(Munge),
MungeApply, MungeHear}`; the bar (APPLY "munge-apply", CANCEL "munge-cancel", HEAR
"munge-hear", box "MUNGE" with SWING, ON, GRUNGE, COMB, and eight toggles "munge-1".."munge-8").
`App::munged(&self) -> Result<(Sample, ZoneMeta, (usize, usize)), String>`: the edit
range cut at the zone's eighths by its `BeatMap` (first cut the range's start), munged;
the meta with `warps` cleared and `beats` set from the new length; the munged range in
the result. Errors: "MUNGE needs the sample's tempo", "nothing left to play". `store_edit`
gains `meta: Option<ZoneMeta>` (written with the new file's path in the same undo step;
a zone's carried lanes win their `stems`). HEAR prehears the munged range.

Tests: `munge_takes_an_eighth_out_of_the_bar` (a 2-bar loop at 120 with a tempo: mute
eighth 8, APPLY: the zone's file is 7/8 as long, its meta keeps the bpm, has 7 beats and
no warps; one undo brings the old file and meta back); `munge_needs_a_tempo` (status, no
edit); `the_munge_bar_shows_its_fields` (harness: MUNGE opens the bar, "munge-1" toggles).

### Task 4: MOTION and TAIL in the zone bar and on the MK3, and the docs

**Files:** `apps/viewer/src/layout/sampling.rs`, `apps/viewer/src/main.rs`,
`apps/viewer/src/sampling_hw.rs`, `docs/sampling-controller.md`, the roadmap's note.

**Produces:** in the zone bar's SLICE box, after PLAYS: TAIL (drag, 10 ms a step, "Off"
at 0; right-click: off) and MOTION (choice); `SamplingAct::Motion(Motion)` (one undo
step "Slice Motion" through `set_new_sampler`); knobs `Play.Tail` ("TAIL") and
`Play.Motion` ("MOTION") on the PLAYBACK page with `Apply::{Tail, Motion}`.

Tests (app): `motion_and_tail_are_set_and_undone` (the act and the knob set them in the
document; undo takes each back; the engine's sampler has them).
