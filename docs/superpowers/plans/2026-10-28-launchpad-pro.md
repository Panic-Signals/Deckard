# Launchpad Pro [MK3] Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Launchpad Pro [MK3] as a Lua controller script: a Session grid of the
Ideas view, scene launch, and Print to Clip into the focused sound's pattern; and with
it, MIDI controllers as a kind of device anyone can script.

**Architecture:** The controller layer (`crates/surface`) gains MIDI devices: a script
folder `devices/<id>/` with a `device.lua` manifest (ports by name) and a `main.lua` of
handlers, each device in its own Lua state, fed the same `View` as the MK3 and returning
the same `Action`s plus `MidiOut`, `LaunchScene` and `PrintNotes`. The app
(`apps/viewer/src/midi_devices.rs`) opens a device when its ports appear, routes bytes
both ways, and keeps its exclusive ports out of note input.

**Tech Stack:** Rust, mlua (Lua 5.4), midir, egui app.

**Spec:** `docs/superpowers/specs/2026-10-28-launchpad-pro-design.md`

## Global Constraints

- Scripts are DECKARD's own; nothing copied from Novation, Ableton or the GPL Bitwig
  script (`~/code/Launchpad-Pro-Mk3-for-Bitwig`, read for behaviour only).
- SysEx header `F0 00 20 29 02 0E`; Session pads `10·row + col` (row 1 bottom), CCs per
  the spec's table; Print to Clip times `value / 500` beats, 7 bits per byte MSB first,
  packet id 8 × 4 bits.
- `PPQ` = 960 ticks per beat.
- A device script error never stops the MK3 layer or note input.
- Tests don't open real MIDI hardware or write the user's settings or library folders;
  MIDI port tests use virtual ports (as `midi_io.rs` tests do) and return early when
  the platform can't make them.
- Run cargo via mise; targeted tests while building; full suite + `mise run lint`
  before any commit; commit only when asked.

## Review Focus

1. Unplugging the Launchpad mid-play: the device closes cleanly, no panic, no stuck
   notes, the port list refreshes; plugging it back in reopens it.
2. A Print to Clip with a missing packet: nothing written, a status message; the next
   print works.
3. A project with more than 8 groups or scenes: arrows scroll, a cell press focuses the
   right group/scene (offset by the scroll).
4. The device script failing to load or throwing in `tick`: the error shows in the
   status bar; the MK3 and notes keep working; fixing the file reloads it.
5. The Launchpad's DAW port must not play notes (its pad presses in Session would play
   the focused sound otherwise).

---

### Task 1: MIDI devices in the controller layer

**Files:**
- Modify: `crates/surface/src/sources.rs` (device-aware lookup; shipped device files)
- Modify: `crates/surface/src/lib.rs` (`Surface::for_device`, `midi`, `open`/`close`,
  `Action::MidiOut`, `Action::LaunchScene`, `Action::PrintNotes`, manifest parsing)
- Modify: `crates/surface/src/api.rs` (`deckard.midi_send`, `deckard.launch_scene`,
  `deckard.print_notes`)
- Create: `crates/surface/src/device_tests.rs`

**Interfaces:**
- Produces:
  - `pub struct DeviceManifest { pub id: String, pub name: String, pub ports: Vec<(String, Vec<String>)>, pub exclusive: Vec<String> }`
  - `pub fn devices(user: Option<&Path>) -> Vec<DeviceManifest>` — every `devices/<id>/device.lua`, shipped and user, except `mk3`.
  - `impl Surface { pub fn for_device(id: &str, user: Option<PathBuf>) -> Surface; pub fn open(&mut self, v: &View, now: Instant) -> Vec<Action>; pub fn close(&mut self) -> Vec<Action>; pub fn midi(&mut self, role: &str, bytes: &[u8], v: &View, now: Instant) -> Vec<Action> }` (`tick`, `poll`, `reload`, `error` as for the MK3).
  - `Action::MidiOut { role: String, bytes: Vec<u8> }`, `Action::LaunchScene(usize)`, `Action::PrintNotes { length: f64, notes: Vec<PrintedNote> }` with `pub struct PrintedNote { pub pitch: u8, pub start: f64, pub length: f64, pub velocity: u8 }` (beats).

- [ ] **Step 1: Write the failing tests** in `device_tests.rs` (a temp user folder with a test device):
  - `a_device_manifest_is_found`: `devices/test/device.lua` returning `{ name = "Test", ports = { daw = {"Test DAW"} }, exclusive = {"daw"} }` → `devices(Some(dir))` has id `"test"`, port `("daw", ["Test DAW"])`, exclusive `["daw"]`; `mk3` never listed.
  - `midi_in_reaches_the_script_and_sends_back`: `main.lua` with `midi = function(role, b) deckard.midi_send(role, { 0x90, b[2], 5 }) end` → `s.midi("daw", &[0x90, 11, 127], ..)` returns `[Action::MidiOut { role: "daw", bytes: vec![0x90, 11, 5] }]`.
  - `open_and_close_run`: `open` sends `F0 00 20 29 02 0E 10 01 F7`, `close` sends `… 10 00 F7`; asserted as `MidiOut`.
  - `a_device_never_loads_mk3_pages`: `require("pages.control")` from the device errors; `require("lib.util")` works.
  - `launch_and_print_actions`: `deckard.launch_scene(2)` → `Action::LaunchScene(2)`; `deckard.print_notes{ length = 4, notes = {{60, 0.5, 0.25, 100}} }` → `Action::PrintNotes { length: 4.0, notes: vec![PrintedNote { pitch: 60, start: 0.5, length: 0.25, velocity: 100 }] }`.
  - `a_failing_device_script_reports_and_keeps_the_last`: a syntax error after a working load → `s.error()` is `Some`, handlers still the old ones.
- [ ] **Step 2: Run** `mise exec -- cargo test -p deckard-surface device_tests` — Expected: FAIL (no `for_device`).
- [ ] **Step 3: Implement.** Lookup for a device: `devices/<id>/<path>` user then shipped, then `lib/<path>` user then shipped; nothing else. `deckard.device()` returns the id. Indices from Lua are 0-based as the MK3 API's are (`launch_scene(2)` is the third scene; check `api.rs`'s `indexed!` and follow it).
- [ ] **Step 4: Run** the same tests, then the whole surface crate: `mise exec -- cargo test -p deckard-surface` — Expected: PASS (MK3 tests unchanged).
- [ ] **Step 5: Commit** (when asked).

### Task 2: The app runs MIDI devices

**Files:**
- Create: `apps/viewer/src/midi_devices.rs`
- Modify: `apps/viewer/src/main.rs` (own a `MidiDevices`; in `midi_sync` leave exclusive ports out of `ins`; each frame tick devices with the controller `View`; route `Action::MidiOut`; apply the other actions with `surface_act`)
- Modify: `apps/viewer/src/main.rs` `surface_act`: `A::LaunchScene(s) => self.launch_scene(s)`; `A::PrintNotes` → Task 3.

**Interfaces:**
- Consumes: Task 1's `devices`, `Surface::for_device`, `open/close/midi/tick`.
- Produces: `pub struct MidiDevices`; `fn sync(&mut self, inputs: &[String], outputs: &[String], user: Option<&Path>)` (open devices whose every port role matches an input and an output by name fragment, close those whose ports went); `fn exclusive_ports(&self) -> Vec<String>`; `fn poll(&mut self, v: &View, now: Instant) -> Vec<Action>` (drain input bytes from each device's midir thread, call `midi`, `tick`; send `MidiOut` itself and return the rest); `fn close_all(&mut self)` (on quit).

- [ ] **Step 1: Write the failing tests** (virtual ports, early return if they can't be made):
  - `a_device_opens_when_its_ports_appear`: a temp user folder with device `test` (ports `daw = {"DECKARD Test DAW"}`); create virtual ports `DECKARD Test DAW` in and out; `sync` → open; the virtual input receives the bytes `open()` sent.
  - `its_presses_reach_the_script`: send `90 0B 7F` into the device; `poll` returns the action the test script makes for it.
  - `exclusive_ports_are_not_note_inputs`: after open, `exclusive_ports()` contains the port name; `midi_sync`'s filter (factor it as `fn note_inputs(all: &[String], off: &BTreeSet<String>, exclusive: &[String]) -> Vec<String>`) leaves it out.
  - `unplugging_closes_the_device`: drop the virtual ports, `sync` with the new lists → closed, no panic; recreate → reopened.
- [ ] **Step 2: Run** `mise exec -- cargo test -p deckard --bin deckard midi_devices` — Expected: FAIL.
- [ ] **Step 3: Implement.** One midir input per device role, its callback pushing `Vec<u8>` into a channel (SysEx kept whole: `ignore(Ignore::None)`); outputs held for `MidiOut`. Re-sync when the port list changes (the app already refreshes it). Errors from a device show in the status bar the way the MK3 layer's do.
- [ ] **Step 4: Run** the tests above and `midi_io::` — Expected: PASS.
- [ ] **Step 5: Commit** (when asked).

### Task 3: Print to Clip into the focused pattern

**Files:**
- Modify: `apps/viewer/src/main.rs` (`fn print_notes(&mut self, length: f64, notes: &[PrintedNote])`, called from `surface_act`)
- Test: `apps/viewer/src/print_tests.rs` (new, `#[cfg(test)] mod` in main.rs)

**Interfaces:**
- Consumes: `deckard_doc` `edit_notes(label, g, p, remove, add)`, `add_pattern`, the scene-pattern setter the Ideas view uses, `PatternLength`.
- Produces: the behaviour in the spec's "Print to Clip into DECKARD".

- [ ] **Step 1: Write the failing tests** (an in-memory project, no files):
  - `printing_replaces_the_focused_sounds_notes`: pattern with sound 0 notes and sound 1 notes, focus sound 1; print 2 notes → sound 1 has exactly those (pitch, `start·960`, `length·960`, velocity), sound 0's untouched; one undo restores all.
  - `printing_makes_a_pattern_when_there_is_none`: focused scene has no pattern for the group → a new one in the scene with the notes, length `length·960`.
  - `printing_lengthens_the_pattern`: 1-bar pattern, print `length = 8` beats → pattern 2 bars; a shorter print leaves it.
  - `a_note_past_the_end_wraps`: length 4, a note at 3.5 lasting 1 → two notes: start 3.5 length 0.5, and start 0 length 0.5 (same pitch and velocity).
- [ ] **Step 2: Run** `mise exec -- cargo test -p deckard --bin deckard print_tests` — Expected: FAIL.
- [ ] **Step 3: Implement** as one labelled edit ("Print to Clip").
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** (when asked).

### Task 4: The Launchpad Pro [MK3] script

**Files:**
- Create: `crates/surface/lua/devices/launchpad_pro_mk3/device.lua`, `main.lua`, `lib/print.lua` (packet decoding), registered in `sources.rs`'s `SHIPPED`
- Test: `crates/surface/src/launchpad_tests.rs`

**Interfaces:**
- Consumes: Task 1's device API; the `View`'s `groups` (`color`, `patterns`, `scene_pattern`), `scenes` (`patterns` per group, `color`), `scene`, `group`, playing and queued scene fields (add `queued_scene: Option<usize>` to `View` if it isn't there, filled where the app fills the view).
- Produces: the behaviour in the spec's "The Launchpad script".

- [ ] **Step 1: Write the failing tests** (`tests_support::view()` with 2 groups × 3 scenes, scene 1 lacking group 1's pattern, scene 0 playing):
  - `open_enters_daw_mode_and_session`: `open` sends `10 01`, `00 00 00 00`, `18 01` SysEx in that order.
  - `the_grid_shows_the_ideas_view`: after `tick`, the lights sent (decode the `03` SysEx specs into a map LED → colour) are: pad 81 (group 0, scene 0) pulsing group 0's palette colour; pad 82 dim RGB of group 0; pad 72 off (no pattern); pads 13–88 beyond 2×3 off; 101–103 lit, 101 green.
  - `only_changes_are_sent`: a second `tick` with the same view sends nothing.
  - `a_pad_focuses_group_and_scene`: note 72 on → `FocusGroup(1)`, `FocusScene(1)`.
  - `the_bottom_row_launches`: CC 102 127 → `LaunchScene(1)`.
  - `arrows_scroll`: a view with 10 scenes; CC 92 → the grid's column 1 is scene 8; pad 81 → `FocusScene(8)`.
  - `not_in_session_draws_nothing`: device reports layout `04` (Note) → `tick` sends nothing; layout `00` → redraws everything.
  - `print_to_clip_decodes`: begin, a data packet with length `00 0F 50` (2000 = 4 beats), a data packet with offset `00 00 00` and notes `(00 00, 00 7D, 3C, 64)` and `(03 74, 00 7D, 3E, 50)`, end (ids 0, 1, 2, 3) → `PrintNotes { length: 4.0, notes: [(60, 0.0, 0.25, 100), (62, 1.0, 0.25, 80)] }` (`03 74` = 3·128 + 116 = 500 = 1 beat; `00 7D` = 125 = ¼ beat).
  - `a_skipped_packet_drops_the_print`: ids 0, 2, … → no `PrintNotes`, a status message action.
  - `close_leaves_daw_mode`: `10 00`.
- [ ] **Step 2: Run** `mise exec -- cargo test -p deckard-surface launchpad_tests` — Expected: FAIL.
- [ ] **Step 3: Implement** the scripts (palette colours for flashing/pulsing: nearest of the 128 by RGB, a table in `main.lua`; RGB for static).
- [ ] **Step 4: Run** — Expected: PASS; then `mise exec -- cargo test -p deckard-surface`.
- [ ] **Step 5: Commit** (when asked).

### Task 5: Docs and the hardware checklist

**Files:**
- Modify: `docs/controller-api.md` (a "MIDI controllers" section: the device folder, manifest, handlers, `midi_send`, `launch_scene`, `print_notes`, how to add your own; the Launchpad as the example)
- Modify: `docs/FEATURES.md` (#5 → Done when the checklist passes; until then a note "built, awaiting hardware")
- Create: `docs/launchpad-pro-mk3.md` (user-facing: what each control does; the checklist below)

- [ ] **Step 1:** Write the docs.
- [ ] **Step 2: Hardware checklist** (with the user and a connected Launchpad): ports found on macOS; Session lights match the Ideas view, colours readable; pad and bottom row; arrows with 9+ scenes; Note mode plays the focused sound; the sequencer follows DECKARD's clock (Sync: Send to `LPProMK3 MIDI`); Print to Clip writes the pattern; unplug/replug; quitting leaves the Launchpad in standalone.
- [ ] **Step 3:** Full suite + lint; commit (when asked).
