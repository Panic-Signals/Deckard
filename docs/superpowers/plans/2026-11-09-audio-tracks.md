# Audio Tracks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make DECKARD's audio recording and Audio plug-in playback match Maschine, and add one thing Maschine lacks: dragging a pattern's audio along the grid.

**Architecture:** A per-pattern `offset` on `AudioCue` (ticks from 1.1.1), stored as a DECKARD-only field on the pattern's `AudioEvent` (stripped from Maschine's archive, saved in the DECKARD block). The engine's cue carries the shift from the pass start to the take's first frame, so playback follows Maschine's formula plus the offset. The app's recording finish follows Maschine's rules; the pattern editor draws from the offset and drags it.

**Tech Stack:** Rust workspace; packages `deckard-model`, `deckard-doc`, `deckard-engine`, `deckard-lua-host`, `deckard` (the app, egui/eframe 0.36, tests with `egui_kittest`).

**Spec:** `docs/superpowers/specs/2026-11-09-audio-tracks-design.md`

## Global Constraints

- Everything matches Maschine except the drag (spec, "Decided by the user").
- New takes start at offset 0 (Maschine's placement). Nothing places a take automatically where it was recorded.
- No single-sound internal source; the INPUT list stays Master and groups.
- A project saved by DECKARD opens in Maschine unchanged: no `deckard:` field in Maschine's archive, no audio rotated.
- Offsets are ticks, 960 per beat (`deckard_model::PPQ`), measured from 1.1.1 like note positions.
- Undo label for a drag: "Move Audio".
- Targeted tests only (`cargo test -p <package> <name>`); the full workspace suite runs once at the end of the branch.
- Comments and docs in the repo's plain style; public docs never mention reverse-engineering tools.
- Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. A drag on an Audio plug-in row in pad view in Gate mode, where the row has notes: dragging a note moves the note; dragging empty space moves the audio (not a rubber-band selection). Pinned in Task 6.
2. Undo after a drag puts the waveform and playback back in the same step. Pinned in Task 6.
3. Pattern Start not at 1.1.1 (e.g. 3.2.1) with offset 0: the take plays from the place Maschine plays it, not from its first frame. Pinned in Task 3.
4. An old project (block version 10, no offsets) opens with every offset 0 and saves as version 11. Pinned in Task 2.
5. A Sync recording onto an Audio plug-in sound while the transport plays is heard before the pattern comes round. Pinned in Tasks 4 and 5.

---

### Task 1: `AudioCue::offset` in the model

**Files:**
- Modify: `crates/model/src/lib.rs` (`AudioCue` at ~789; the `AudioEvent` read at ~2325)
- Modify: every `AudioCue { sound, take }` literal (`crates/doc/src/audio.rs:217,287,289`, `crates/engine/src/lib.rs:7581`, `apps/viewer/src/main.rs:14360,14383`)

**Interfaces:**
- Produces: `pub struct AudioCue { pub sound: usize, pub take: usize, pub offset: i64 }` (add `Default`); `pub const AUDIO_OFFSET: &str = "deckard:audio_offset";` in `deckard_model`, an `Int` field on the `NI::MASCHINE::DATA::AudioEvent` object.

- [ ] **Step 1: Failing test** in `crates/model/src/lib.rs` tests: `an_audio_events_offset_is_read`. Build a project tree with one AudioEvent carrying `deckard:audio_offset = 2880` (reuse whichever fixture the existing AudioEvent parse test uses; if none, write one via `deckard_doc` in Task 2 instead and make this step a compile-only change). Assert `pat.audio == [AudioCue { sound: s, take: 0, offset: 2880 }]`, and an event without the field reads `offset: 0`.
- [ ] **Step 2:** `cargo test -p deckard-model an_audio_events_offset_is_read`: fails (no field).
- [ ] **Step 3:** Add the field and constant; read it from the AudioEvent object (`ev.get(AUDIO_OFFSET).and_then(Value::as_int).unwrap_or(0)`). Add `offset: 0` to the literals listed.
- [ ] **Step 4:** `cargo test -p deckard-model an_audio_events_offset_is_read` passes; `cargo check --workspace --tests` builds.
- [ ] **Step 5:** Commit "Model: a pattern's audio offset".

### Task 2: Setting, saving and reopening the offset (document)

**Files:**
- Modify: `crates/doc/src/audio.rs` (new method), `crates/doc/src/notes.rs` (`audio_event_ops` keeps an existing offset when it rebuilds the event for the same take: re-switching a take keeps it only if the take is unchanged; a new take starts at 0)
- Modify: `crates/doc/src/block.rs` (`VERSION = 11`; a new list `audio_offsets: Vec<(usize, usize, usize, i64)>`, written after the version-10 view text, read when `version >= 11`, applied in `read_block`)
- Modify: `docs/deckard-block.md` (version 11)

**Interfaces:**
- Consumes: `AudioCue::offset`, `AUDIO_OFFSET` (Task 1).
- Produces: `Document::set_audio_offset(&mut self, g: usize, p: usize, s: usize, ticks: i64, label: &str) -> Result<(), String>`; errors when the pattern has no AudioEvent for `s`. Writes the field on the AudioEvent object (removes it at 0). One undo step.

- [ ] **Step 1: Failing tests** in `crates/doc/src/audio.rs` tests (use the existing `check` setup: sound 3 of group 0 with one take on in pattern 0):
  - `an_audio_offset_is_set_and_undone`: `set_audio_offset(0, 0, 3, 1920, "Move Audio")` → `patterns[0].audio[0].offset == 1920`; `undo()` → 0; label of the step is "Move Audio".
  - `an_audio_offset_survives_save_and_reopen`: set 1920, save to a temp `.mxprj`, reopen → 1920; Maschine's archive in the saved file has no `deckard:` field (as `crates/doc/src/lfos.rs:222` checks).
  - `a_version_10_block_reads_with_no_offsets`: encode a block at version 10 (as the existing old-version tests in `block.rs` do), decode → `audio_offsets` empty; a project from it saves at version 11.
  - `switching_the_take_resets_the_offset`: offset 1920 on take 0, `set_audio_in(.., Some(1), ..)` → offset 0.
- [ ] **Step 2:** `cargo test -p deckard-doc audio_offset` and `version_10_block` and `switching_the_take`: fail.
- [ ] **Step 3:** Implement as listed. The block's old-version truncation tests get the new length.
- [ ] **Step 4:** Same commands pass; `cargo test -p deckard-doc block` passes.
- [ ] **Step 5:** Commit "Document: set, save and reopen a pattern's audio offset".

### Task 3: Playback from the pattern's Start and the offset (engine)

**Files:**
- Modify: `crates/engine/src/lib.rs` (`Cue` at ~97; the pass loop at ~518–538; `cue_take` ~393; `recue` ~3285; `trigger_with` ~5100), `crates/engine/src/audio_loop.rs` (doc comment only)

**Interfaces:**
- Consumes: `AudioCue::offset`.
- Produces: `pub enum Cue { On(usize, i64), Off }`, the `i64` the shift in ticks from the cue's tick to the take's first frame: `offset − pat.start`. Every place turning a cue into `(take, origin_beats)` uses `origin = (e.tick + shift) / PPQ`. `LoopRt::place` is unchanged (`(beats − origin) mod take_beats`), so before the origin the take wraps round.

- [ ] **Step 1: Failing tests** in `crates/engine/src/lib.rs` tests, using `audio_project` (~7560; its loop has a mark at its first frame, `marks()` finds it):
  - `a_loop_follows_the_patterns_start`: pattern `start = 2 * PPQ`, `length = 8 * PPQ`, take 4 beats, offset 0. Render from the song's start: the first mark is at beat 2 of the pass (pattern time 4 beats − 2 → the take's first frame falls 2 beats into the pass), not at the pass start. Assert the first mark's frame within 512 frames of `2 beats` at the project tempo.
  - `a_loop_starts_at_its_offset`: start 0, offset `PPQ` (one beat). First mark one beat into the pass; the next mark one take-length later.
  - `an_offset_past_the_take_wraps`: offset `5 * PPQ` with a 4-beat take: marks as with offset `PPQ`.
- [ ] **Step 2:** `cargo test -p deckard-engine a_loop_follows_the_patterns_start a_loop_starts_at_its_offset an_offset_past_the_take_wraps` (run each): fail.
- [ ] **Step 3:** Implement: the pass loop pushes `Cue::On(c.take, c.offset - pat.start)`; `cue_take` returns `Option<(usize, i64)>`; `recue`, `trigger_with` and the ahead-player path (`ahead::Ev::Cue`) use `origin = (e.tick + shift) as f64 / PPQ`. Clip patterns (arranger) use the same.
- [ ] **Step 4:** The three pass; `cargo test -p deckard-engine loop` passes (existing loop tests unchanged).
- [ ] **Step 5:** Commit "Engine: a loop follows its pattern's Start and offset, as Maschine's".

### Task 4: A new loop is heard at once (engine)

**Files:**
- Modify: `crates/engine/src/lib.rs` (where a new project/timeline is taken while playing: call `recue()` for sounds whose cue changed)

**Interfaces:**
- Consumes: Task 3's cue.
- Produces: no new API. When the project given to a playing engine switches a take on in the pattern now playing (or changes its take or offset), that sound's loop is cued at once at its place, not at the next pass.

- [ ] **Step 1: Failing test** `a_loop_switched_on_while_playing_is_heard_at_once`: start `audio_project` with the cue removed, play 2 beats, hand the engine the project with the cue on (the same call the app uses after an edit), render one more beat: the output is not silent.
- [ ] **Step 2:** `cargo test -p deckard-engine a_loop_switched_on_while_playing_is_heard_at_once`: fails.
- [ ] **Step 3:** Implement: after the timeline is rebuilt for a new project while playing, run `recue()` (it already finds the last cue before the playhead for each Audio sound); `adopt` keeps a sound whose cue didn't change in place.
- [ ] **Step 4:** Passes; `cargo test -p deckard-engine loop` passes.
- [ ] **Step 5:** Commit "Engine: a loop switched on while playing is heard at once".

### Task 5: Recording finishes as Maschine's (app)

**Files:**
- Modify: `apps/viewer/src/recorder.rs` (`StartWith` gets `pub focus: (usize, usize)`; `Finished::Take` gets `focus: (usize, usize)`, the group and sound focused at Start)
- Modify: `apps/viewer/src/main.rs` (`start_recording` passes `(self.group, self.sound)`; `finish_recording`, `finish_loop`, `next_free_sound`, `load_take`)

**Interfaces:**
- Consumes: `Document::add_take`, `set_audio_in`, `audio_only` (existing).
- Produces: the rules, from the spec's "What DECKARD changes" 1–3:
  - The destination's group and sound come from `focus`, not from the focus at Stop.
  - Detect or Sync onto a sound whose `instrument == "AudioModule"`: `add_take` (selected), `set_audio_in` on the focused pattern, pattern grown to hold it (doubling, as Loop does), sound renamed if its name isn't the user's (as `load_take` decides). Otherwise unchanged (`load_take`).
  - A Detect take's Length in beats is its `ticks` rounded to the nearest sixteenth (`PPQ / 4`), at least one sixteenth; Sync and Loop keep their whole bars.
  - `next_free_sound(g)` falls back to the last sound of the group whose instrument is `AudioModule`; with none, the group's last sound (unchanged).

- [ ] **Step 1: Failing tests** in `apps/viewer/src/main.rs`, next to `recordings_land_where_maschine_puts_them` (reuse its `tone` and `free` helpers):
  - `a_sync_take_on_an_audio_sound_is_a_new_take`: make sound s an Audio plug-in with one take (finish a Loop/Take recording first), then `Finished::Take { mode: 1, focus: (0, s), .. }` → `instrument` still `AudioModule`, 2 takes, focused 1, the pattern's cue on take 1, no Sampler.
  - `a_detect_take_on_an_audio_sound_is_rounded_to_a_sixteenth`: `mode: 0, ticks: 4.0 * PPQ + 70.0` onto an Audio sound → the new take's `beats == 4.0` (70 ticks is under half a sixteenth, 120).
  - `a_detect_take_on_an_empty_sound_is_a_sampler_zone`: `mode: 0` on an empty sound → Sampler with one zone, no `audio` (Maschine's rule, unchanged).
  - `the_take_goes_to_the_sound_focused_at_start`: `focus: (0, 2)` while `h.state().sound == 5` → the take is on sound 2.
  - `with_no_free_sound_left_a_take_goes_to_the_last_audio_sound`: fill every sound of group 0 after the focus, with sound 9 the last `AudioModule` and sound 15 a Sampler; Target Sound → a new take on sound 9.
- [ ] **Step 2:** `cargo test -p deckard <name>` for each: fail.
- [ ] **Step 3:** Implement as listed; existing `Finished::Take` constructions in tests get `focus`.
- [ ] **Step 4:** The five pass, and `cargo test -p deckard recordings_land_where_maschine_puts_them` and `cargo test -p deckard recorder` pass.
- [ ] **Step 5:** Commit "Recording: Maschine's rules for audio takes and the destination sound".

### Task 6: Draw and drag the audio in the pattern editor (app)

**Files:**
- Modify: `apps/viewer/src/layout.rs` (the loop drawing at ~6009–6057; the pointer handling around the double-click at ~6389; `Act` at ~594 gets `MoveAudio { g: usize, p: usize, s: usize, offset: i64 }`)
- Modify: `apps/viewer/src/main.rs` (`Act::MoveAudio` → `set_audio_offset(.., "Move Audio")`)

**Interfaces:**
- Consumes: `AudioCue::offset`, `Document::set_audio_offset`.
- Produces: drawing and dragging:
  - Drawing: at pattern tick `t` (from 1.1.1), the take's place is `(t − offset).rem_euclid(loop_ticks)`; repeat index `((t − offset) / loop_ticks).floor()`, the first pass (index 0) bright, any other index (negative too) darker, as now.
  - Drag: in pad view on an Audio plug-in row (`instrument == "AudioModule"`, the pattern has its cue), or in the piano roll when the focused sound is one, a press on empty space (no note under it) not in draw mode starts a drag. While dragging the waveform is drawn at the dragged offset; release emits `MoveAudio` with `offset = (old + Δticks).rem_euclid(loop_ticks)` snapped to the editor's step (`step` ticks), or to the tick with SHIFT held. A tooltip shows the offset as bars.beats.sixteenths (the editor's existing position format). No drag on a row whose loop is off in the pattern. Double-click (on/off) is unchanged.

- [ ] **Step 1: Failing tests** in `apps/viewer/src/main.rs` (kittest harness, as `timeline_drags_set_pattern_start_and_length` ~12793 drives the timeline):
  - `dragging_a_loop_moves_its_audio`: a pattern with an Audio sound's take on; drag on its row (pad view) right by one beat of grid → the cue's offset is `PPQ`; `undo` → 0, one step labelled "Move Audio".
  - `a_loop_drag_snaps_unless_shift`: drag by 1.3 steps → offset one step; with SHIFT → within a tick of 1.3 steps.
  - `dragging_a_note_on_a_gate_row_moves_the_note`: Gate-mode Audio sound with a note; drag the note → the note moves, the offset stays 0; drag empty space on that row → the offset moves.
  - `the_piano_roll_drags_the_focused_loop`: piano roll, focused Audio sound, drag empty space → offset moves.
- [ ] **Step 2:** `cargo test -p deckard <name>` each: fail.
- [ ] **Step 3:** Implement as listed.
- [ ] **Step 4:** The four pass; `cargo test -p deckard loop` and `cargo test -p deckard timeline_drags` pass.
- [ ] **Step 5:** Commit "Pattern editor: drag a pattern's audio along the grid".

### Task 7: The MK3's lanes and the docs

**Files:**
- Modify: `crates/lua-host/src/pattern_view.rs` (~193, ~315: the `ClipAudioLane` drawing places the waveform with the cue's offset, by the same rule as Task 6's drawing)
- Modify: `docs/audio-plugin.md` ("How DECKARD does it": the offset, the drag, the cue's shift, Maschine's recording rules; "Not confirmed": `offset` and `unknown_98` are the editor's Scroll X and FocusFrame), `docs/sampling-layout.md` (DECKARD's recorder notes: the destination sound from the focus at Start; Detect/Sync on an Audio sound)

**Interfaces:**
- Consumes: `AudioCue::offset`.

- [ ] **Step 1: Failing test** in `crates/lua-host` beside the existing `ClipAudioLane` test: `the_mk3_lane_draws_the_loop_from_its_offset`: a cue with offset one beat → the lane's first bright column is one beat in (compare the pixel columns the existing test reads).
- [ ] **Step 2:** `cargo test -p deckard-lua-host the_mk3_lane_draws_the_loop_from_its_offset`: fails.
- [ ] **Step 3:** Implement; update the docs.
- [ ] **Step 4:** Passes; `cargo test -p deckard-lua-host pattern_view` passes.
- [ ] **Step 5:** Commit "MK3: draw the loop from its offset; document audio offsets and recording".

### Finish

- [ ] Run the full suite once: `cargo test --workspace` (~45 min). Known environmental failure on this Mac: `deckard-doc sounds::tests::clear_double_duplicate_and_delete_patterns`.
- [ ] `cargo clippy --workspace --tests` and `cargo fmt --check`.
