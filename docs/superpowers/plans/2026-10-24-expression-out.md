# Expression out, part 2a (MIDI out) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Recorded and live per-note expression plays out of MIDI ports: poly aftertouch on normal ports, per-note pitch/slide/pressure on member channels on MPE ports, with DECKARD setting the synth up.

**Architecture:** The document's expression table rides on `Project` (as `Arc`s) into each `Timeline`; timeline notes carry their `ExprId`. The engine keeps each sounding MIDI note in one place (a `MidiNote` record replacing the `MidiOff` tuple), reads its curves at the playhead each block, and sends changed values (≤ every 5 ms). Per-port MPE state (member channels, bend range) lives in the engine, set from the settings through `Output` like the MIDI sender; the synth set-up messages (RPN) are sent by `midi_io`.

**Tech Stack:** Rust (mise-pinned), egui 0.36 / egui_kittest, midir.

**Spec:** `docs/superpowers/specs/2026-10-24-expression-out-design.md` (2a only; 2b gets its own plan).

## Global Constraints

- Clean room: no NI code ported; formats from files and decompiles only.
- Cargo only via mise: `mise exec -- cargo …`; lint `mise run lint`.
- Tests write no library folders and no settings file.
- Projects with no expression save byte-for-byte as before; nothing here changes saving.
- Neutral values: pitch bend 8192 (centre), CC74 64, pressure 0.
- MPE: lower zone, manager channel 1 (status low nibble 0), member channels 2–16 (nibbles 1–15); default bend range 48.
- Values sent only when the sent number changes, at most every 5 ms per note and dimension.

## Review Focus

1. A note stolen, released, cut by stop, or retriggered on the same key: its member channel is freed and its curve stops; no expression is sent for a note no longer sounding.
2. Changing a port's format (Normal ↔ MPE) or its bend range while notes sound: no stuck notes (they end on the channel they started on) and used member channels go neutral.
3. A pattern note with an `ExprId` the timeline's table doesn't have (table older than the timeline, or a corrupt id): it plays as a plain note, no panic.
4. Two sounds on one MPE port, and a sound passing notes to another sound (`OutRoute::Sound`): channels are shared per port; no double allocation.
5. Live keys held on an MPE port while the pattern also plays notes there: live and timeline notes share the port's channels without one ending the other's.

---

### Task 1: Expressions reach the engine

**Files:**
- Modify: `crates/model/src/lib.rs` (Project), `crates/doc/src/lib.rs` (expressions storage, refresh), `crates/doc/src/expression.rs`, `crates/engine/src/lib.rs` (NoteEvent, Timeline, timeline building, every `NoteEvent {` literal), `crates/engine/src/pads.rs`, `crates/engine/src/ahead.rs` if it builds NoteEvents
- Test: `crates/doc/src/expression.rs` tests, `crates/engine/src/lib.rs` tests

**Interfaces:**
- Produces: `Project.expressions: Vec<std::sync::Arc<deckard_model::Expression>>` (index = `ExprId.0`; empty from `Project::read`); the document's table becomes `Vec<Arc<Expression>>` and `project().expressions` always equals it. `NoteEvent.expr: Option<ExprId>`. `Timeline.exprs: Vec<Arc<Expression>>`. `Engine::expression(&self, id: ExprId) -> Option<Arc<Expression>>` (from the playing timeline).

- [ ] **Step 1: Write the failing tests**
  - doc: `project_carries_the_expression_table`: `add_expression(e)` then `project().expressions[id.0]` is `e`; after `edit_notes` and `undo` still there; a fresh `Document::new_project()` has none.
  - engine: `timeline_notes_carry_their_expression`: project with one pattern note `expr: Some(ExprId(0))` and `p.expressions = vec![Arc::new(e)]`; `timeline(&p, mode)` has that note's event with `expr == Some(ExprId(0))`, `t.exprs.len() == 1`; an engine built from it: `e.expression(ExprId(0))` is `Some`, `e.expression(ExprId(9))` is `None`.
- [ ] **Step 2: Run** `mise exec -- cargo test -p deckard-doc --lib project_carries` and `mise exec -- cargo test -p deckard-engine --lib timeline_notes_carry` — Expected: compile errors (no `expressions` / `expr`).
- [ ] **Step 3: Implement.** `Rc` → `Arc` in the doc's table; `refresh` and `add_expression` keep `project.expressions` equal to it. Add `expr` to `NoteEvent` (pattern notes take `n.expr`; every other literal `expr: None`); `timeline()` fills `exprs` from `project.expressions`.
- [ ] **Step 4: Run** both tests, then `mise exec -- cargo test -p deckard-engine --lib` and `-p deckard-doc --lib` — Expected: PASS.

### Task 2: Normal ports play recorded pressure and release velocity

**Files:**
- Modify: `crates/engine/src/lib.rs` (`MidiOff` → `MidiNote`, `trigger_with`, `fire_note_offs`, `stop`, `note_release`, live pressure arm, a per-block expression pass)
- Test: `crates/engine/src/lib.rs` tests

**Interfaces:**
- Consumes: Task 1's `NoteEvent.expr`, `Engine::expression`.
- Produces: `struct MidiNote { at: u64, port: Arc<str>, channel: u8, note: u8, held: (usize, usize, i64), release: u8, expr: Option<Arc<Expression>>, started: u64, sent: [Option<u16>; 3], sent_at: [u64; 3] }` (one per sounding note; `release` the note-off velocity, 0 when none); `fn end_midi_note(&mut self, n: MidiNote)` — the only place a note-off is sent (Task 3 frees MPE channels there); `fn play_expression(&mut self)` called each block before note-offs.

- [ ] **Step 1: Write the failing tests** (engine at 1000 Hz, 120 BPM, a sound to port "Synth" channel 2, as `sounds_send_midi`):
  - `recorded_pressure_plays_as_poly_aftertouch`: a note at tick 0, length one beat, pressure curve 0 → 1 straight over the beat; process 500 frames: poly AT `0xa2` on its key, values rising, first ≥ 0 and last 127, strictly increasing, and nothing after its note-off.
  - `expression_sends_changes_at_most_every_5_ms`: same note with a constant pressure 0.5: exactly one `0xa2 … 64` message; a curve 0 → 1 over 10 ticks: no two `0xa2` within 5 frames (5 ms) of each other (record frame of each via processing 1 frame at a time).
  - `release_velocity_on_the_note_off`: `release_velocity: Some(40)` → note-off `[0x82, note, 40]`; without → `[0x82, note, 0]` as today.
  - `pitch_and_slide_stay_off_normal_ports`: a note with only pitch and slide curves sends note-on and note-off only.
  - `sounds_send_midi` and `pressure_on_a_held_note_goes_out_as_poly_aftertouch` unchanged and still green.
- [ ] **Step 2: Run** `mise exec -- cargo test -p deckard-engine --lib -- recorded_pressure expression_sends release_velocity pitch_and_slide` — Expected: FAIL (no aftertouch sent / velocity 0).
- [ ] **Step 3: Implement.** Replace the tuple with `MidiNote`; all removals go through `end_midi_note`; `play_expression` reads each note's pressure at `(time − started) · ticks per sample` and sends `0xa0 | channel` when the 0–127 value changed and ≥ 5 ms since its last send for that dimension.
- [ ] **Step 4: Run** the four tests and `mise exec -- cargo test -p deckard-engine --lib` — Expected: PASS.

### Task 3: MPE ports in the engine

**Files:**
- Modify: `crates/engine/src/lib.rs`, `crates/engine/src/midi.rs` (MPE port state), `crates/engine/src/output.rs` (`Command::SetMpeOut`, `Output::set_mpe_out`, applied in `play` like `midi_tx`)
- Test: `crates/engine/src/lib.rs` tests, `crates/engine/src/midi.rs` tests

**Interfaces:**
- Consumes: Task 2's `MidiNote`, `end_midi_note`, `play_expression`.
- Produces: `Engine::set_mpe_out(&mut self, ports: BTreeMap<String, u8>)` (port name → bend range; absent = Normal); `Output::set_mpe_out(&self, ports: &BTreeMap<String, u8>)` (no-op if unchanged); `midi::MpePort { range: u8, busy: [bool; 15], free_since: [u64; 15], used: u16 }` with `fn take(&mut self, now: u64) -> Option<u8>` (member nibble 1–15, the one free longest) and `fn free(&mut self, ch: u8, now: u64)`; `fn bend_14(semitones: f32, range: u8) -> u16` (8192 + st/range·8192, clamped 0–16383).

- [ ] **Step 1: Write the failing tests** (port "Synth" set to MPE, range 48):
  - `mpe_notes_get_channels_of_their_own`: three overlapping notes go out on channels 2, 3, 4 (status `0x91`, `0x92`, `0x93`), each note-off on its own channel; the sound's channel setting is ignored.
  - `mpe_channels_are_taken_free_longest_first` (midi.rs, `MpePort`): free 2 then 1 → next take is the one freed first.
  - `mpe_notes_start_with_their_values`: before a note's note-on, on its channel: pitch bend from the curve's first value (+12 st at range 48 → `bend_14(12.0, 48)` = 10240), CC74 from slide, channel pressure from pressure; a note without expression starts at 8192 / 64 / 0.
  - `mpe_curves_play_on_the_member_channel`: pressure → `0xd?`, slide → `0xb? 74`, pitch → `0xe?` on that note's channel while it plays; a bend past the range is clipped (±60 st at 48 → 16383 / 0).
  - `lanes_go_on_the_manager_channel`: a CC lane event and a pitch-bend lane event on an MPE port go out on channel 1 (`0xb0`, `0xe0`).
  - `the_sixteenth_note_steals_the_oldest`: 16 overlapping notes: the 16th ends the first (its note-off sent) and takes its channel.
  - `stop_resets_used_member_channels`: after notes on channels 2 and 3, `stop()` sends on 2 and 3 only: bend 8192, CC74 64, pressure 0.
  - `changing_a_ports_format_resets_it`: `set_mpe_out` removing the port while a note sounds: the note ends on its member channel; used channels reset; later notes go on the sound's channel.
- [ ] **Step 2: Run** `mise exec -- cargo test -p deckard-engine --lib -- mpe_ lanes_go the_sixteenth stop_resets changing_a_ports` — Expected: FAIL (no `set_mpe_out`).
- [ ] **Step 3: Implement** per the spec's 2a MPE section; `end_midi_note` frees the channel; `play_expression` sends all three dimensions on MPE notes. Notes a loop wrap cuts end through `end_midi_note` too; the next note on that channel sends its own starting values, which is the spec's reset for that case.
- [ ] **Step 4: Run** the tests and `mise exec -- cargo test -p deckard-engine --lib` — Expected: PASS.

### Task 4: Live expression out

**Files:**
- Modify: `crates/engine/src/lib.rs` (`play_live`'s `Live::Expression` arms)
- Test: `crates/engine/src/lib.rs` tests

**Interfaces:**
- Consumes: Tasks 2–3 (`MidiNote`, MPE ports, `bend_14`).
- Produces: nothing new.

- [ ] **Step 1: Write the failing tests:**
  - `live_mpe_expression_goes_out_per_note`: on an MPE port, `note_in` thru then `Live::Expression` Pressure 0.5 / Slide 0.25 / Pitch 2.0 for that note: `0xd? 64`, `0xb? 74 32`, `0xe?` `bend_14(2.0, 48)` on its member channel.
  - `live_pitch_and_slide_stay_off_normal_ports`: same on a normal port: only the pressure, as poly AT.
  - `live_release_velocity_reaches_the_note_off`: `Live::Expression { kind: ExprKind::Release, value: 0.5 }` then `NoteRelease`: note-off velocity 64.
- [ ] **Step 2: Run** `mise exec -- cargo test -p deckard-engine --lib -- live_mpe live_pitch live_release` — Expected: FAIL.
- [ ] **Step 3: Implement** (Release sets the held `MidiNote.release`).
- [ ] **Step 4: Run** them and `mise exec -- cargo test -p deckard-engine --lib` — Expected: PASS.

### Task 5: Settings, synth set-up, Preferences

**Files:**
- Modify: `apps/viewer/src/settings.rs` (`mpe_out: BTreeMap<String, u8>`, `mpe_setup: bool` default true; lines `mpe_out = <range> <port>` and `mpe_setup = false`), `crates/engine/src/midi.rs` (`mpe_setup`), `apps/viewer/src/midi_io.rs` (`set_mpe_out`, send on connect), `apps/viewer/src/main.rs` (Preferences ▸ MIDI outputs: an MPE checkbox and range per output port like inputs; a "Send MPE setup to synths" checkbox; wiring next to `self.midi.set_mpe` → `self.midi.set_mpe_out(...)` and `out.set_mpe_out(...)`)
- Test: `settings.rs`, `midi.rs`, `midi_io.rs` tests

**Interfaces:**
- Consumes: Task 3's `Output::set_mpe_out`.
- Produces: `midi::mpe_setup(range: u8) -> Vec<[u8; 3]>`; `MidiIo::set_mpe_out(&self, ports: &BTreeMap<String, u8>, send_setup: bool)`; `midi_io::setup_due(old: &BTreeMap<String, u8>, new: &BTreeMap<String, u8>) -> Vec<(String, u8)>` (ports newly MPE or with a changed range).

- [ ] **Step 1: Write the failing tests:**
  - `mpe_out_settings_round_trip`: `mpe_out = 48 Hydrasynth Explorer` and `mpe_setup = false` parse and write back; defaults: `mpe_out` empty, `mpe_setup` true.
  - `mpe_setup_configures_the_lower_zone`: `mpe_setup(48)` starts `[0xb0,0x65,0],[0xb0,0x64,6],[0xb0,0x06,15]` (MCM, 15 members); contains `[0xb0,0x06,2]` after `[0xb0,0x64,0]` (manager ±2) and, for channels `0xb1` and `0xbf`, `0x65 0`, `0x64 0`, `0x06 48`, `0x26 0`; each channel's RPN closed with `0x65 0x7f`, `0x64 0x7f`.
  - `setup_is_due_for_new_and_changed_ports`: `setup_due({}, {A:48})` = `[(A,48)]`; `({A:48},{A:48})` = `[]`; `({A:48},{A:24})` = `[(A,24)]`; a removed port: `[]`.
- [ ] **Step 2: Run** `mise exec -- cargo test -p deckard --bin deckard -- mpe_out_settings setup_is_due` and `mise exec -- cargo test -p deckard-engine --lib mpe_setup` — Expected: FAIL.
- [ ] **Step 3: Implement.** `set_mpe_out` sends `mpe_setup` for `setup_due` ports when `send_setup`; `send_loop` sends it right after connecting a port that is in the MPE map (when `send_setup`), before the message that caused the connection. Preferences as above (hover text: "MPE synth: notes on channels 2–16, each with its own bend, slide and pressure").
- [ ] **Step 4: Run** the tests, then `mise exec -- cargo test -p deckard --bin deckard -- settings midi_io` — Expected: PASS.

### Task 6: Finish

- [ ] **Step 1:** `docs/PARITY.md` row 10: part 2a done (normal ports poly AT, MPE ports per note, set-up); part 2b, 3, 4 still missing. 
- [ ] **Step 2: Run** `mise run lint` — Expected: PASS. Run `mise exec -- cargo test --workspace --no-fail-fast > <workspace>/suite.txt 2>&1` and read its tail — Expected: every binary `ok`.
- [ ] **Step 3:** Hand the user: rebuild/restart; (a) Hydrasynth in poly AT mode, port Normal: play back a pad Poly AT take — the MIDI Monitor's Out list shows Poly AT on the notes, the Hydrasynth's aftertouch moves; (b) Hydrasynth in MPE mode, port set to MPE in Preferences ▸ MIDI: the Out list shows the set-up on connect, then notes on channels 2–16 with channel pressure; route MPE-X / MPE-Yabs / MPE-Yrel in the mod matrix to hear a take from an MPE controller.
