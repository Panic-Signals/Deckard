# MPE Capture and Keep Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** record per-note MPE expression (pitch, slide, pressure, release velocity) and
channel-wide CC, pitch bend and channel pressure. Keep the expression with its notes
through edits and undo, and save it in a DECKARD block that real Maschine ignores.

**Architecture:**
- A note carries an optional `ExprId`. It is stored as a DECKARD-only field on the
  note's `TimedEvent` in the document tree. That field is stripped before Maschine's
  archive is written. Because the id lives in the tree, every existing edit, copy and
  undo carries it for free.
- The `Expression` values sit in an append-only table in the `Document`, so ids never
  change meaning.
- On save, the referenced expressions go into a root-level `DKRD`/`EXPR` hsin child. On
  load they come back out of it.
- Input:
  - MIDI and the MK3 become a new `Live::Expression` event.
  - The engine records it into its own stream; it doesn't play it (capture only).
  - The viewer's `Take` builds each note's curves.
  - A new `ControlWriter` records channel-wide lanes, overwriting as it writes.

**Tech Stack:** Rust (workspace crates `mxprj`, `deckard_model`, `deckard_doc`,
`deckard_engine`, `deckard_mk3`, app `apps/viewer`, egui), midir.

**Spec:** `docs/superpowers/specs/2026-10-24-mpe-capture-design.md`

## Global Constraints

- **Cargo:** always through mise: `mise exec -- cargo …`, `mise run test`,
  `mise run lint`, `mise run check-platforms`.
- **Tests while building:** run only the task's targeted tests. Run the full check
  (`mise run test` and `mise run lint`) in Task 11, and before any commit.
- **Commits:** none unless the user asks. Commit messages end with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never commit the manual
  PDFs, `out/`, `analysis/` or `.superpowers/`.
- **Clean room:**
  - no NI code ported;
  - Maschine format facts come from files and decompiles;
  - the DECKARD block is our own format.
- **Never write** NI's database, libraries or favourites. Tests write no library folders
  and no settings file. Don't restart the user's running DECKARD.
- **Byte-identity:** a project with no expression (and no channel pressure) saves
  byte-identically to today.
- **Real Maschine 3** must still open every project DECKARD saves.
- **Values:**
  - Pitch is in semitones (f32);
  - Slide and Pressure are 0–1 (f32);
  - release velocity is 1–127 (u8);
  - MPE member bend range defaults to ±48, manager ±2;
  - thinning tolerance is 0.05 semitone for Pitch and 0.005 for Slide and Pressure;
  - the touch release is 250 ms.

## Review Focus

1. **A file that already has a DKRD block,** opened and re-saved. It must end up with
   exactly one block, holding the new data. Saving with no expression left must remove
   it. Tests in Tasks 1 and 4.
2. **An MPE controller reusing a member channel** for a new note before the old note's
   note-off arrives. The new note takes the channel, and the old note's curve stops
   there. Test in Task 7.
3. **A note held across the pattern's loop end while recording.** Curve offsets stay
   relative to its start and increase. Nothing is written at negative offsets. Test in
   Task 9.
4. **Pressure, bend or slide for a note that isn't held** (it arrives after the release,
   or a released MK3 pad springs back). It's ignored, never attached to another note.
   Tests in Tasks 7 and 8.
5. **A take with expression, undone, then saved.** Unreferenced expressions aren't
   written, so the block is absent or holds only live notes. Test in Task 4.

---

### Task 1: Root-level child items in the mxprj container

**Files:**
- Modify: `crates/mxprj/src/container.rs`
- Test: `crates/mxprj/src/container.rs` (`mod tests`)

**Interfaces:**
- Produces:
  - `pub fn root_child(file: &[u8], domain: [u8; 4], item: [u8; 4]) -> Option<Vec<u8>>`
    returns the payload of that child's first frame (its body above the terminator).
  - `pub fn set_root_child(file: &[u8], domain: [u8; 4], item: [u8; 4], frame_id: u32, payload: Option<&[u8]>) -> Result<Vec<u8>, String>`
    removes any existing child with that domain and item. When `payload` is given, it
    appends one item at the end of the root's child list and updates the root's child
    count and size.

The layout is in `docs/mxprj-format.md`:
- An item is `u64 size, u32 1, "hsin", u32 1, u32 0, uuid[16], frame chain, u32 list_version, u32 n, n × {u32 tag, u32 domain, u32 item_id, item}`.
- A frame is `u64 size, "DSIN", u32 id, u32 version, body`.
- The child item's chain is one `frame_id` frame over the terminator frame (id 1,
  body `u32 1`). Its own child list is empty.
- Domain and item ids are stored as the little-endian u32 of the big-endian fourcc,
  exactly as for `MAS2`.
- The uuid is random, from the existing uuid helper used by `new_file`, or 16 bytes
  from `getrandom` if there's none.

- [ ] **Step 1: Write the failing tests**
  - `set_root_child_adds_one_child`: on `new_file(PROJ…)` bytes,
    `set_root_child(.., *b"DKRD", *b"EXPR", u32::from_be_bytes(*b"DKRD"), Some(b"hello"))`
    gives a file whose root parses with `Item::parse` and has 2 children. The last
    child's domain is `DKRD`, and `root_child(..)` gives `b"hello"`.
  - `set_root_child_replaces`: setting twice (`b"one"`, then `b"two"`) leaves 2 root
    children, and `root_child` gives `b"two"`.
  - `set_root_child_none_restores_the_file`: add, then set `None`, gives bytes equal to
    the original.
  - `set_root_child_none_on_a_plain_file_is_identity`: `None` on a file without the
    child returns the input bytes unchanged.
  - `the_project_still_decodes_with_a_child`: `Project::parse` and `decode` of the
    file with the child give the same tree as without.
- [ ] **Step 2: Run them and see them fail**
  - Run: `mise exec -- cargo test -p mxprj set_root_child the_project_still_decodes`
  - Expected: FAIL to compile (`set_root_child` not found).
- [ ] **Step 3: Implement** `root_child` and `set_root_child` (byte-level, with no
  borrow of `Item`).
- [ ] **Step 4: Run them and see them pass.** Then run the round-trip tool on the test
  folder from the design check, which should report 0 differ:
  `mise exec -- cargo run -q -p mxprj --bin mxprj-roundtrip -- <dir>`.

### Task 2: The expression model

**Files:**
- Create: `crates/model/src/expression.rs`
- Modify:
  - `crates/model/src/lib.rs`: `pub mod expression;`, `Note.expr`, and
    `MidiKind::ChannelPressure`;
  - every `Note { … }` literal without `..` (53 sites): add `expr: None`;
  - every exhaustive `match` on `MidiKind`.
- Test: `crates/model/src/expression.rs` (`mod tests`)

**Interfaces:**
- Produces:
  - `pub struct ExprId(pub u32)`, with `#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]`.
  - `Note.expr: Option<ExprId>`. `Note` stays `Copy` and `Default`.
  - `pub enum Dim { Pitch, Slide, Pressure }` and
    `pub enum ExprKind { Dim(Dim), Release }`.
  - `pub struct Point { pub at: i64, pub value: f32, pub curve: f32 }`. `at` is in
    ticks from the note's start. `curve` is 0 (straight) in part 1.
  - `pub struct Curve { pub points: Vec<Point> }` with:
    - `fn value_at(&self, at: i64) -> f32`: linear between points, held before the
      first and after the last;
    - `fn slice(&self, from: i64, to: i64) -> Curve`: re-based so that `from` becomes
      0, with points added at both edges;
    - `fn join(a: &Curve, a_len: i64, b: &Curve) -> Curve`.
  - `pub struct Expression { pub pitch: Option<Curve>, pub slide: Option<Curve>, pub pressure: Option<Curve>, pub release_velocity: Option<u8> }`
    with `fn curve(&self, Dim) -> Option<&Curve>` and `fn is_empty(&self) -> bool`.
  - `pub fn thin(samples: &[(i64, f32)], tolerance: f32) -> Vec<Point>`:
    Ramer–Douglas–Peucker on (tick, value). It keeps the first and last samples and
    collapses equal runs.
  - `MidiKind::ChannelPressure(u8)`: stored as kind 6 (Task 5).

- [ ] **Step 1: Write the failing tests**
  - `a_ramp_thins_to_its_ends`: 1000 samples from (0, 0.0) to (999, 1.0) become
    2 points.
  - `a_sine_keeps_its_shape`: 2000 samples of `0.5 + 0.5·sin` become fewer than 100
    points. `value_at` of the thinned curve is within 0.005 of every sample.
  - `equal_runs_collapse`: `[(0,.3),(10,.3),(20,.3),(30,.8)]` becomes ticks `[0,20,30]`
    or fewer, and still reproduces every value.
  - `value_at_holds_and_interpolates`: points (10, 0), (20, 1). `value_at(0)` is 0,
    `value_at(15)` is 0.5 and `value_at(99)` is 1.
  - `slice_rebases`: slicing that curve at 15..20 gives points at 0 (0.5) and 5 (1.0).
  - `join_offsets_the_second`: `join(a, 100, b)` places b's point 0 at 100.
- [ ] **Step 2: Run them and see them fail**
  - Run: `mise exec -- cargo test -p deckard-model expression`
  - Expected: FAIL to compile.
- [ ] **Step 3: Implement.** Add `expr: None` at the literal sites, and the
  `ChannelPressure` arms: route it as a controller would, and in the engine's
  `control` ignore it. `mise exec -- cargo check --workspace --tests` must be clean.
- [ ] **Step 4: Run them and see them pass.**

### Task 3: Expression follows its note (Document)

**Files:**
- Modify:
  - `crates/doc/src/lib.rs`: the `expressions` table, `add_expression`, `expression`,
    and stripping in `to_bytes`;
  - `crates/doc/src/notes.rs`: `new_event` writes the field;
  - `crates/model/src/lib.rs`: the `TimedEvent` reader reads the field into
    `Note.expr`;
  - `crates/doc/src/note_tools.rs`: split slices, join joins.
- Create: `crates/doc/src/expression.rs` (the doc-side helpers)
- Test: `crates/doc/src/expression.rs` (`mod tests`)

**Interfaces:**
- Consumes: Task 2's types.
- Produces:
  - `const EXPR_FIELD: &str = "deckard:expr"`: an `Int` field on a note's
    `TimedEvent`, not on the `NoteEvent`, which can be shared by reference.
  - `Document::add_expression(&mut self, e: Expression) -> ExprId`: appends to
    `expressions: Vec<Rc<Expression>>`. It is never undone; ids are stable.
  - `Document::expression(&self, id: ExprId) -> Option<&Expression>`.
  - `fn strip_deckard_fields(v: &mut Value)`: removes every field whose name starts
    with `deckard:`. `to_bytes` calls it on its clone of the root before encoding.
- Rules:
  - `new_event` sets `EXPR_FIELD` from `n.expr`, removing any copied from the template.
  - Clones of whole `TimedEvent`s (pattern duplicate, Double, copy sound, convert to
    clip) keep the field as they are.
  - Split: each part gets `add_expression` of the curves sliced to its span.
  - Join: the joined note gets the joined curves.
  - Length changes leave the curve alone. Values past the note's length are ignored
    when read and when saved.

- [ ] **Step 1: Write the failing tests.** Each starts from a new project with a pattern
  holding one note `n` whose `expr` is the id of an expression with a pressure ramp.
  - `expression_survives_move_transpose_quantize`: `edit_notes` removing `n` and adding
    it moved, then transposed. `project()` shows the moved note with the same `expr`,
    and `expression(id)` still gives the ramp.
  - `expression_survives_copy_and_duplicate`: copy/paste a note, duplicate the pattern,
    Double Pattern, copy and paste the sound, Convert to Clip. Every resulting note
    has the `expr`.
  - `split_slices_and_join_joins`: splitting at the middle gives two notes whose curves
    are each half. Joining them back gives one curve equal to the original at 9
    sample points.
  - `delete_drops_and_undo_restores`: deleting the note leaves no note with that
    `expr`. `undo()` brings back the note with `expr`.
  - `saving_without_expression_is_byte_identical`: a project opened from a copy of a
    test fixture and saved unchanged equals the input.
  - `deckard_fields_never_reach_the_archive`: a project with `expr` saves, and decoding
    the saved archive finds no field named `deckard:*`.
- [ ] **Step 2: Run them and see them fail**
  - Run: `mise exec -- cargo test -p deckard-doc expression`
  - Expected: FAIL to compile (`add_expression` not found).
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them and see them pass.** Then run
  `mise exec -- cargo test -p deckard-doc notes note_tools` to check no note-editing
  test broke.

### Task 4: The DECKARD block: saving and loading expression

**Files:**
- Create:
  - `crates/doc/src/block.rs` (the payload encoder and decoder);
  - `docs/deckard-block.md` (the format).
- Modify: `crates/doc/src/lib.rs`: `to_bytes` writes the block, `from_bytes` reads it.
- Test: `crates/doc/src/block.rs` (`mod tests`)

**Interfaces:**
- Consumes: Task 1's `set_root_child`/`root_child`, and Task 3's table and field.
- Produces:
  - `pub(crate) fn encode_block(p: &Project, exprs: &dyn Fn(ExprId) -> Option<&Expression>) -> Option<Vec<u8>>`
    returns `None` when nothing has expression.
  - `pub(crate) fn decode_block(bytes: &[u8]) -> Result<Block, String>`.
- **Payload v1**, little-endian, documented in `docs/deckard-block.md`:
  - `u32 version = 1`, then `u32 pattern_count`;
  - per pattern: `u32 group, u32 pattern` (clip patterns as `CLIP + k`, as the doc
    numbers them), then `u32 entries`;
  - per entry:
    - `u32 sound, i64 position, i32 pitch, u32 ordinal` (the ordinal among the
      pattern's notes with the same sound, position and pitch, in `Pattern.notes`
      order);
    - `u8 release_velocity` (0 = none);
    - `u8 dims_mask` (bit 0 Pitch, 1 Slide, 2 Pressure);
    - per present dim, `u32 n` then n × `{i64 at, f32 value, f32 curve}`;
  - then a channel-pressure section (Task 5): `u32 lane_count`, and per lane
    `u32 group, u32 pattern, u32 sound, u32 n, n × {i64 position, u8 value}`. In v1 it
    is written as 0 lanes until Task 5.
  - Frame id `DKRD`, domain `DKRD`, item `EXPR`.
- **Save:** clip curves to each note's length. Only expressions referenced by notes in
  the project are written. `set_root_child(…, None)` runs when there's nothing, so
  stale blocks go.
- **Load:**
  - For each entry, find the note by its key and ordinal, `add_expression`, and set
    `EXPR_FIELD` on its `TimedEvent` directly. That isn't an undo step, so the project
    opens clean. Then `refresh()`.
  - Entries without a note are dropped.
  - A block that fails to parse, or whose version isn't 1, is logged with `eprintln!`
    and ignored.

- [ ] **Step 1: Write the failing tests**
  - `expression_round_trips_through_a_save`: a note with all three curves and release
    velocity 64 survives `to_bytes`, then `from_bytes`, with equal curves and
    `release_velocity == Some(64)`.
  - `the_block_is_a_dkrd_root_child`: `root_child(saved, DKRD, EXPR)` is `Some`.
  - `resave_keeps_one_block`: save, open, save again. There's 1 DKRD child.
  - `no_expression_no_block`: a project whose only expressed note was deleted saves
    with `root_child` `None`.
  - `undone_take_writes_nothing`: add a note with `expr` in a take, undo it, and save.
    There's no block.
  - `duplicate_keys_use_the_ordinal`: two notes at the same sound, position and pitch
    with different expressions each get their own back.
  - `a_bad_block_is_ignored`: a file with `set_root_child(.., Some(b"junk"))` opens,
    with no expression and no error.
  - `orphan_entries_are_dropped`: a block naming a missing note opens, and the other
    entries still apply.
- [ ] **Step 2: Run them and see them fail**
  - Run: `mise exec -- cargo test -p deckard-doc block`
  - Expected: FAIL to compile.
- [ ] **Step 3: Implement,** and write `docs/deckard-block.md`.
- [ ] **Step 4: Run them and see them pass.** Then rerun Task 3's tests.
- [ ] **Step 5: Manual check (user).** Build a test project and give its full path, but
  don't open it in the user's Maschine:
  - Open a copy of a Velvet Lounge project in the doc.
  - Add a note with an expression.
  - Save it to the scratchpad.
  - Ask the user to open it in real Maschine 3, after saving any unsaved work. Ledger
    the result.

### Task 5: Channel pressure lanes and their storage

**Files:**
- Modify:
  - `crates/doc/src/notes.rs`: `midi_kind` and `midi_fields` map kind 6 to
    `ChannelPressure` (value in `i20`, `i18` −1);
  - `crates/doc/src/block.rs`: the channel-pressure section;
  - `crates/doc/src/lib.rs`: `to_bytes`;
  - `apps/viewer/src/layout.rs`: the `Lane` enum gets `ChannelPressure`, named
    "Aftertouch", with `max` 127. `Lane::of`, `from_ref`/`to_ref` map it to
    `LaneRef { kind: 6 }`, and it appears in the MIDI Lanes dialog.
- Test: `crates/doc/src/block.rs`, and `apps/viewer/src/layout.rs` (lane mapping).

**Interfaces:**
- Consumes: Task 2's `MidiKind::ChannelPressure`, and Task 4's block.
- Produces: `edit_midi` accepts `ChannelPressure` events. In the tree they are
  MidiEvents of kind 6.

The design says channel pressure goes in the DECKARD block until real Maschine is shown
to read kind 6 correctly.

**Ruling (plan):** the in-memory form is the same either way (kind-6 MidiEvents in the
tree). Storage differs only at save. `to_bytes` works on its clone of the document:
- it removes every `ChannelPressure` event, using the existing `event_ops` removal on
  the clone;
- then it writes those events into the block's channel-pressure section;
- on load they're re-added to the tree with `event_ops`, without an undo step.

Switching to native storage later means deleting the strip.

- [ ] **Step 1: Write the failing tests**
  - `channel_pressure_round_trips_in_the_block`: three `ChannelPressure` events save
    and load back equal. The saved archive has no kind-6 MidiEvent.
  - `aftertouch_lane_maps_to_kind_6` (viewer): `Lane::of(&MidiKind::ChannelPressure(0))`
    is `Lane::ChannelPressure`, and `to_ref`/`from_ref` round-trip it.
- [ ] **Step 2: Run them and see them fail**
  - Run: `mise exec -- cargo test -p deckard-doc channel_pressure` and
    `mise exec -- cargo test -p deckard aftertouch_lane`
  - Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them and see them pass.**
- [ ] **Step 5: Native test file (user).** Build a scratchpad copy of a Velvet Lounge
  project with a kind-6 MidiEvent lane of a rising ramp in Maschine's own format, with
  no strip. Ask the user whether real Maschine shows an Aftertouch lane with the ramp,
  and plays it. Ledger the answer. Native storage replaces the block only if the user
  says yes **and** a decompile confirms the field layout. That would be a follow-up,
  not part of this plan.

### Task 6: The engine: expression events, recording streams, repeat velocity

**Files:**
- Modify:
  - `crates/engine/src/midi.rs`: `InMsg`;
  - `crates/engine/src/lib.rs`: `Live`, `play_live`, the recorded streams and
    `take_recorded_*`;
  - `crates/engine/src/output.rs`: the take functions;
  - `crates/engine/src/repeat.rs`: `Repeater::pressure`.
- Test: `crates/engine/src/midi.rs` and `crates/engine/src/lib.rs` (`mod tests`)

**Interfaces:**
- Consumes: Task 2's `ExprKind` and `MidiKind::ChannelPressure`.
- Produces:
  - `InMsg::NoteOff { ch, note, velocity: u8 }`. A 0x90 with velocity 0 gives
    velocity 0.
  - `InMsg::PolyPressure { ch, note, value }` (0xA0).
  - `InMsg::ChannelPressure { ch, value }` (0xD0).
  - `Live::Expression { group, sound, pitch, kind: ExprKind, value: f32 }`. It is
    recorded when recording and sequencing. It never reaches a sound. Exception: a
    `Pressure` for a note held in the `Repeater` calls
    `Repeater::pressure(group, sound, pitch, velocity: i64)` with
    `velocity = (value * 127).round().clamp(1, 127)`, and the notes it fires after
    that use that velocity.
  - `Live::ControlIn { group, sound, kind, thru, record: bool }`. When `record` is
    true, recording and sequencing, it's pushed to the control stream.
  - `pub struct RecordedExpr { pub group, pub sound, pub pitch: i64, pub kind: ExprKind, pub value: f32, pub tick: f64, pub mode: Mode }`
    and
    `pub struct RecordedControl { pub group, pub sound, pub kind: MidiKind, pub tick: f64, pub mode: Mode }`,
    each in a ring of `RECORDED_ROOM`.
  - `Output::take_recorded_expr() -> Vec<RecordedExpr>` and
    `Output::take_recorded_controls() -> Vec<RecordedControl>`, next to
    `take_recorded`.

- [ ] **Step 1: Write the failing tests**
  - `parses_pressure_and_release_velocity`:
    - `[0xA2,60,90]` gives `PolyPressure{ch:3,note:60,value:90}`;
    - `[0xD0,40]` gives `ChannelPressure{ch:1,value:40}`;
    - `[0x80,60,70]` gives `NoteOff{ch:1,note:60,velocity:70}`;
    - `[0x90,60,0]` gives `NoteOff{velocity:0}`.
  - `expression_is_recorded_not_played`: while recording, a `Live::Expression`
    lands in `take_recorded_expr`, and no sound's state changes. Check with the
    Sampler's bend, which stays 0.
  - `control_in_records_only_when_asked`: `record: true` lands in
    `take_recorded_controls`, and `record: false` doesn't.
  - `pressure_changes_repeat_velocity`: with Note Repeat on and a held pad at
    velocity 100, `Pressure` 0.3 makes the next fired notes velocity 38.
- [ ] **Step 2: Run them and see them fail**
  - Run: `mise exec -- cargo test -p deckard-engine parses_pressure expression_is_recorded control_in_records pressure_changes_repeat`
  - Expected: FAIL to compile.
- [ ] **Step 3: Implement.** Update the existing `NoteOff` and `ControlIn` sites:
  `record: false` everywhere except the MIDI path (Task 7).
- [ ] **Step 4: Run them and see them pass,** plus `mise exec -- cargo test -p deckard-engine repeat midi`.

### Task 7: MIDI input: MPE mode, poly and channel pressure, which sound records

**Files:**
- Modify:
  - `apps/viewer/src/midi_io.rs`: a pure router and the per-port MPE state;
  - `apps/viewer/src/settings.rs`: `mpe` per port;
  - `apps/viewer/src/main.rs` (~1717): the Preferences MIDI inputs list gets an "MPE"
    checkbox and a bend-range value per input;
  - `crates/engine/src/midi.rs`: `MidiTable::exact_channel(g, s, ch) -> bool` and
    `MidiTable::focused() -> Option<(usize, usize)>`, if the table doesn't already
    know the focus.
- Test: `apps/viewer/src/midi_io.rs` (`mod tests`), and `apps/viewer/src/settings.rs`

**Interfaces:**
- Consumes: Task 6's `InMsg` and `Live`.
- Produces:
  - `Settings.mpe: BTreeMap<String, u8>` (port name → member bend range). A port
    present means MPE on. It is saved as lines `mpe = <range> <port>`.
  - `pub(crate) struct Mpe { bend_range: f32, members: [Vec<(usize, usize, i64)>; 16], initial: [[Option<f32>; 3]; 16] }`,
    one per MPE port.
  - `pub(crate) fn route(t: &MidiTable, mpe: Option<&mut Mpe>, port: &str, msg: InMsg) -> Vec<Live>`.
    `handle()` calls it and sends each `Live`, keeping the `Pending` path for notes.
- **Rules:**
  - **MPE on:**
    - A member note-on is routed as channel 1 and bound to its channel.
    - Member bend gives Pitch = `value / 8192 · range`.
    - Member CC74 gives Slide = `v / 127`.
    - Member channel pressure gives Pressure = `v / 127`.
    - Expression that arrives before the note-on is stored as `initial` and sent right
      after the note-on.
    - A member note-off gives `Expression{Release, velocity/127}` (if velocity > 0),
      then `NoteRelease`, then unbinds.
    - A new note-on on a bound channel rebinds it.
    - Other member CCs, and everything on channel 1, are channel-wide.
  - **MPE off:**
    - Poly pressure gives Pressure on the notes `route_note(port, ch, note)` gives.
    - Channel pressure gives `ControlIn{ChannelPressure}`.
    - A note-off with velocity gives Release, then `NoteRelease`.
    - Expression for a note not held (no binding, or no held note at that pitch) is
      dropped. Held notes are tracked per port in `route`.
  - **`record` on `ControlIn`:** true for the focused sound when it's among
    `route_control`'s sounds. Otherwise it's true for sounds whose
    `exact_channel(g, s, ch)` holds. A keyboard-mode group's pads are never all
    flagged.

- [ ] **Step 1: Write the failing tests** (a `MidiTable` with one sound on "Hydra",
  channel 0, and one group in keyboard mode)
  - `mpe_member_note_gets_its_expression`: an MPE port receives:
    - bend on ch 2 before the note-on;
    - note-on ch 2;
    - CC74 64 on ch 2;
    - pressure 100 on ch 2;
    - note-off ch 2 with velocity 50.

    The output is `NoteIn`, then Pitch (the initial), then Slide ≈0.504, then
    Pressure ≈0.787, then Release ≈0.394, then `NoteRelease`, all on that sound and
    pitch.
  - `two_member_notes_stay_apart`: notes on ch 2 and ch 3. Bend on ch 3 goes only to
    the second note.
  - `a_reused_member_channel_rebinds`: (Review Focus 2) note A on ch 2, then note B
    on ch 2 without A's note-off. Bend goes to B only.
  - `manager_channel_is_channel_wide`: CC1 on ch 1 gives `ControlIn`, not
    `Expression`.
  - `poly_pressure_reaches_the_held_note`: MPE off, notes 60 and 64 held. Poly
    pressure on 64 gives Pressure for 64 only.
  - `pressure_for_a_released_note_is_dropped`: (Review Focus 4) after 64's note-off,
    poly pressure on 64 gives nothing.
  - `channel_pressure_is_a_lane`: MPE off, 0xD0 gives
    `ControlIn{ChannelPressure, record: true}` for the focused sound.
  - `only_the_focused_pad_records`: CC on a keyboard-mode group gives `record: true`
    for one sound only.
  - `mpe_setting_round_trips` (settings.rs): `mpe = 48 Hydrasynth` survives saving
    and parsing the settings text (strings only; nothing written to disk).
- [ ] **Step 2: Run them and see them fail**
  - Run: `mise exec -- cargo test -p deckard midi_io:: mpe_setting`
  - Expected: FAIL to compile.
- [ ] **Step 3: Implement,** including the Preferences UI.
- [ ] **Step 4: Run them and see them pass,** plus `mise exec -- cargo test -p deckard sync_tests remote`.

### Task 8: MK3 pad pressure

**Files:**
- Modify: `apps/viewer/src/mk3_io.rs`
- Test: `apps/viewer/src/mk3_io.rs` (`mod tests`)

**Interfaces:**
- Consumes: Task 6's `Live::Expression`.
- Produces: `fn pad_pressure(slot: Option<(usize, usize, i64)>, pressure: f64, sensitivity: f64) -> Option<Live>`.
  - It gives `Live::Expression { kind: Dim(Pressure), value: pad_velocity(pressure, sensitivity) as f32, … }`
    for the pad's sounding note.
  - It gives `None` when the pad isn't sounding.
  - The event loop's `Event::PadPressure(p, v)` arm calls it with `sounding[p-1]`.
  - The scripts' `Event::PadPressure` stays dropped (`main.rs:2759`).

- [ ] **Step 1: Write the failing tests**
  - `pad_pressure_goes_to_the_sounding_note`: slot `Some((0, 3, 60))` and pressure 0.5
    give an Expression on group 0, sound 3, pitch 60, with value equal to
    `pad_velocity(0.5, s)`.
  - `pressure_on_a_silent_pad_is_dropped`: slot `None` gives `None`.
- [ ] **Step 2: Run them and see them fail**
  - Run: `mise exec -- cargo test -p deckard pad_pressure pressure_on_a_silent`
  - Expected: FAIL to compile.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them and see them pass.**

### Task 9: Recording in the app: curves on notes, overwrite-as-you-write lanes

**Files:**
- Modify:
  - `apps/viewer/src/record.rs`: `Take` collects expression, and the new
    `ControlWriter`;
  - `apps/viewer/src/main.rs`: each frame, `take_in_recorded` also takes in the expr
    and control streams and calls `ControlWriter::tick` with the play position;
    write mode ends when recording stops.
- Test: `apps/viewer/src/expression_record_tests.rs` (new, `#[cfg(test)] mod` in
  `main.rs`, in the style of `auto_length_tests.rs`)

**Interfaces:**
- Consumes: Tasks 2, 3 and 6.
- Produces:
  - `Take::add_expr(&mut self, r: RecordedExpr)` keeps raw samples
    `(tick − note start, value)` per held key `(group, sound, pitch)` and dim.
    Samples at negative offsets are dropped. A note held across the loop wrap adds the
    pattern length to its ticks, so offsets keep increasing (Review Focus 3).
    `Release` is kept as the release velocity (`value · 127`).
  - `Take::add` on a release returns the `Note`, plus
    `Option<Expression>` (thinned with the tolerances in the Global Constraints).
    `write_recorded` then calls `doc.add_expression` and sets `n.expr`.
  - `pub struct ControlWriter`. Per `(group, pattern, sound, lane)` it tracks
    `writing_since: Instant` and `cleared_to: i64`.
    - `fn add(&mut self, doc: &mut Document, r: RecordedControl, now: Instant)`
      enters write mode. It removes that lane's events between `cleared_to` and the
      event's pattern position, wrapping at the loop end, and adds the event (at most
      one per tick per lane, duplicates dropped).
    - `fn tick(&mut self, doc: &mut Document, position: (usize, i64) per group, now: Instant)`
      clears up to the playhead for lanes still writing. A lane idle for 250 ms or more
      leaves write mode.
    - Edits go through `edit_midi("Record", …)` inside the open take, so a pass stays
      one undo step.
    - A lane receiving events is added to the sound's lanes (`crates/doc/src/lanes.rs`).

- [ ] **Step 1: Write the failing tests** (they drive `take_in_recorded*` and
  `ControlWriter` directly, without audio)
  - `an_mpe_note_records_its_curves`: an on, then 200 Pressure samples rising 0 to 1,
    then Slide and Pitch samples, then Release 0.5, then the release. The note has an
    `expr`. Its pressure curve has 2 to 10 points ending at 1.0, and its release
    velocity is 64.
  - `two_held_notes_keep_their_own_pressure`: pressure for 60 and 64 interleaved.
    Each note's curve has only its own values.
  - `a_note_held_across_the_loop_keeps_increasing_offsets`: (Review Focus 3) the on
    is at the last beat, the samples wrap past the loop end, then the release. All
    offsets are ≥ 0 and increasing.
  - `a_second_loop_pass_replaces_the_first`: CC1 written over a 1-bar loop on pass 1,
    then different values on pass 2 over the same span. The lane holds only pass 2's
    values. Its count equals pass 2's distinct ticks.
  - `writing_clears_ahead_as_the_playhead_passes`: the lane has old events at ticks
    100 to 900. A CC arrives at 100, and `tick` advances to 500 within 250 ms. Events
    in 100..500 are gone except the new one, and 500..900 remain.
  - `an_idle_controller_stops_overwriting`: after 300 ms without values, `tick` to the
    end leaves the old events after that point.
  - `an_untouched_lane_is_kept`: recording CC1 leaves the CC74 lane unchanged.
  - `a_pass_is_one_undo_step`: notes with expression and CC events in one take. One
    `undo()` removes all of them.
- [ ] **Step 2: Run them and see them fail**
  - Run: `mise exec -- cargo test -p deckard expression_record_tests`
  - Expected: FAIL to compile.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them and see them pass,** plus
  `mise exec -- cargo test -p deckard auto_length_tests record`.

### Task 10: The grid mark for notes with expression

**Files:**
- Modify: `apps/viewer/src/layout.rs` (the note drawing in `group_panel`)
- Test: `apps/viewer/src/expression_record_tests.rs`

**Interfaces:**
- Consumes: `Note.expr`.
- Produces: `pub(crate) fn has_expression_mark(n: &Note) -> bool` (`n.expr.is_some()`).
  It is used to draw a small dot, 3 px, in the note's top-left corner, coloured
  white at 70 % alpha. The look is the builder's choice.

- [ ] **Step 1: Write the failing test** `notes_with_expression_are_marked`: the
  harness draws a pattern with one note that has `expr` and one that doesn't. The
  function and draw path produce a mark for one note only. Record mark rects with
  `note_rect`-style temp data, as `header_rect` does, and count them.
- [ ] **Step 2: Run it and see it fail**
  - Run: `mise exec -- cargo test -p deckard notes_with_expression`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run it and see it pass.**

### Task 11: Docs and the full check

**Files:**
- Modify:
  - `docs/PARITY.md`: a row for expression capture, beyond parity;
  - `docs/mk3-usb.md`: under "Not done yet", pad pressure is now captured (sending it
    is part 2);
  - `docs/note-repeat-arp.md`: aftertouch velocity is now done;
  - this plan's Results section.

- [ ] **Step 1:** Write the docs.
- [ ] **Step 2: The full check.**
  - Run: `mise run test` then `mise run lint` (and `mise run check-platforms` if the
    engine's MIDI code touched platform `cfg`s).
  - Expected: all pass. Every failure is reported by name.
- [ ] **Step 3:** Run `mxprj-roundtrip` over `/Users/Shared` (read-only).
  - Expected: 0 differ.
- [ ] **Step 4:** Hand the user Task 4's test project and Task 5's native test file to
  open in Maschine 3, and ledger their answers.

## Results

- All 11 tasks built inline; rulings are in the ledger and the final message.
- `cargo test --workspace --no-fail-fast`: every crate passes (the viewer 261, the engine
  428, the doc 91). `mise run lint` passes after `cargo fmt --all`.
  `tests::song_view_section_click_sets_the_start_and_queues_a_jump` failed once under
  the full run's load (no playhead yet from the audio device) and passes alone.
- `mxprj-roundtrip /Users/Shared`: 3,681 documents identical, 0 differ. The new
  `crates/doc/examples/doc-roundtrip.rs` (open as a Document and save, in memory):
  117 projects identical, 0 differ, so saving a project with nothing for the DECKARD
  block is unchanged, and none of them has Maschine-made channel pressure to move.
- Maschine 3 check files (Task 4's expression block, Task 5's native aftertouch lane):
  handed to the user; their answers are in the ledger.
