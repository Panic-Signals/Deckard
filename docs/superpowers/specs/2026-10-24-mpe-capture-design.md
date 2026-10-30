# MPE capture and keep: design (expression, part 1 of 4)

October 2026. Part 1 of the expression work: capture and keep. Part 2 sends it out (MIDI
and plug-ins), part 3 is the Ableton-style Expression editor, part 4 is DECKARD's own
sounds and macros responding per voice. Each part gets its own design.

## Why

The user plays a Hydrasynth (MPE, and poly aftertouch outside MPE mode), a Launchpad
(poly aftertouch) and the MK3 pads (pressure while a pad is held). They want what they
play recorded, kept with the notes, and saved, so later parts can send it to the
Hydrasynth, edit it like Ableton's Note Expression, and use it to modulate DECKARD sounds.
Maschine drops poly aftertouch and records no MPE, so this goes beyond parity.

Today DECKARD also records no CC or pitch bend at all, only notes (`Live::ControlIn`
is not recorded). The user wants those recorded too, as part of this.

## Goals

- Record per-note expression with each note, in Ableton's five dimensions:
  - **Pitch:** per-note bend, in semitones.
  - **Slide:** per-note Y, CC74.
  - **Pressure:** poly aftertouch, MPE per-note pressure, or MK3 pad pressure.
  - **Velocity:** already kept.
  - **Release velocity:** the note-off velocity.
- Record channel-wide controllers into the sound's lanes:
  - CC;
  - pitch bend;
  - **channel pressure**, which stays a channel-wide lane like a CC and is never spread
    onto notes (the user's decision).
- Save all of it with the project, and load it back.
- A project DECKARD saves still opens and plays in real Maschine 3. Real Maschine
  ignores the extra data and loses it if it re-saves.

## Non-goals (later parts)

- Sending expression out to MIDI or plug-ins (part 2). Until then, captured expression
  can't be heard.
- Editing curves, expression lanes, or pitch drawn on notes (part 3).
- Per-voice response in DECKARD's sampler and synths, and pressure or slide as macro
  sources (part 4).
- Expression in exported group or sound files (.mxgrp, .mxsnd) and in MIDI file export.

## Anti-goals

- A DECKARD-saved project that real Maschine refuses or misreads. Projects with no
  expression must save byte-for-byte as they do today.
- Writing NI's database, libraries or favourites. Tests write no library folders and
  no settings file.
- Porting NI code. Format facts come from files and decompiles only. The DECKARD block
  is our own format.

## What was established

- **The DECKARD block.** Real Maschine 3 loads a project with an extra item at the top
  level of the hsin tree and ignores it. On 2026-10-24 the user opened a Velvet Lounge
  copy with a root child of domain `DKRD`, item id `EXPR`: one frame, whose fourcc id
  is `DKRD` over the terminator, carrying about 2 KB. It loaded fine. The same block
  placed inside the project item was not reported on.
- **Maschine's lane types.** The MidiAutomator `mode` enum is `Not assigned, Note,
  Velocity, Pitchwheel, Controller, Program change, Aftertouch, Poly-Aftertouch,
  Probability`, i.e. 0–8 (docs/channel-software.md §4.4). DECKARD already reads and
  writes 2, 3, 4 and 8.
- **MK3 pads.** DECKARD decodes `Event::PadPressure(pad, 0..1)` for held pads
  (crates/mk3), and both `mk3_io.rs` and `main.rs` currently drop it. The MK3 reports
  pressure only when it changes.

## Behaviour

### Inputs

- **MPE mode** is a new per-input-port setting under Preferences ▸ MIDI. It is off by
  default and kept in DECKARD's settings.
  - When on, the port uses the lower zone: channel 1 is the manager channel, channels
    2–16 are members.
  - The member pitch-bend range defaults to ±48 semitones and can be set.
  - The manager channel's bend range is ±2, as in the MPE spec.
- **Message routing with MPE on:**

  | Message | Channel | Goes to |
  |---|---|---|
  | Note on/off | member | a note bound to that channel until its note-off |
  | Pitch bend | member | that note's Pitch |
  | CC74 | member | that note's Slide |
  | Channel pressure | member | that note's Pressure |
  | Anything | manager | channel-wide: bend, CC and channel-pressure lanes |
  | Other CCs | member | the channel-wide CC lanes |

  Expression sent on a member channel before its note-on is that note's initial value,
  as MPE requires.
- **MPE off, the normal case:**
  - Poly aftertouch (0xA0) on a key is the Pressure of the held note with that pitch
    on that channel.
  - Channel pressure, CC and pitch bend are channel-wide.
- **MK3 pads:** a held pad's pressure is the Pressure of the note that pad is playing,
  in drum and keyboard modes alike. It goes through the pad sensitivity curve, as hit
  velocity does. *My call.*
- **Release velocity:** taken from note-off (0x80 velocity). A 0x90 with velocity 0
  gives none. The MK3 pads give none.
- **Routing:** expression for a note follows the note to the sound it reaches through
  the existing `MidiTable`. Messages that today reach no sound are still dropped.
- **MPE routing** *(my call):* an MPE zone acts as one channel. A sound or group set
  to the manager channel (1), or to all channels, receives notes from every member
  channel. A sound set to a channel from 2 to 16 doesn't take member notes from an MPE
  port.
- **Live, before part 2** (the user's decision): per-note MPE (a member channel's bend,
  CC74 or pressure) is captured only. It isn't passed on live, so one note's bend never
  bends every voice of a sound. Channel-wide messages reach sounds as they do today.
- **Note Repeat and Arp** (the user's decision): pressure on a held pad or key changes
  the velocity of the notes it generates, as Maschine does. Those generated notes
  carry no curves.
- **Which sound records a channel-wide message** *(my call):*
  - the focused sound, when it is one of the sounds the message reaches;
  - otherwise each sound it reaches whose own channel setting matches;
  - never every pad of a keyboard-mode group.

### Recording

- **Per-note curves:**
  - Each note recorded with expression gets up to three curves (Pitch, Slide, Pressure)
    and an optional release velocity.
  - A curve is a list of breakpoints `(offset from the note's start in ticks, value)`:
    - Pitch is in semitones;
    - Slide and Pressure are 0–1.
  - A curve covers the note from its start to its note-off. Values after the note-off
    are not kept. *My call.*
  - A note with no incoming expression gets no curves. An absent curve means "no
    expression", not zero.
- **Thinning.** Recorded curves are simplified to keep the shape within a tolerance:
  - 0.05 semitone for Pitch;
  - 0.5 % of range for Slide and Pressure;
  - consecutive equal values collapse.

  So a long held lean becomes a few dozen points, not thousands. *My call on the
  algorithm and tolerances.*
- **Segment shape.** Breakpoints are joined by straight lines. The stored format leaves
  room for a per-segment curve amount, which part 3 needs for ⌥-drag curving; every
  value is 0 in part 1.
- **Channel-wide recording:**
  - CC, pitch bend and channel pressure are recorded as the sound's lane events while
    recording.
  - Duplicates are dropped, and there is at most one event per tick per lane.
  - A lane that receives events is added to the sound's lanes, so it shows in the
    editor.
  - **Overwrite as you write** (the user's decision): recording a controller over
    existing data replaces it and never doubles it, loop pass after loop pass.
    - The moment a lane receives a value, it goes into write mode.
    - While it's writing, that lane's existing events are cleared as the playhead
      passes them, and the new values are written in their place.
    - When the loop wraps while still writing, the previous pass's events are
      cleared in the same way.
    - Lanes the controller doesn't touch keep their data. A pass that never moves
      the mod wheel leaves the mod-wheel lane alone.
    - Write mode ends when recording stops, or after the controller has been idle
      for a short time (touch-style, about 250 ms; the user chose touch over latch).
      From then on, the lane's remaining events play as before.
- **Channel pressure storage.** It goes in Maschine's own data (mode 6) if real Maschine
  accepts that. This is the first step of the plan:
  - build a test project with a channel-pressure lane in Maschine's format, as we did
    for the DECKARD block;
  - the user opens it in real Maschine;
  - if it loads and shows the lane, use Maschine's format;
  - otherwise store channel pressure in the DECKARD block.

  CC and pitch bend use Maschine's existing event kinds (4 and 3).

  *My call on sequencing:* channel pressure goes in the DECKARD block first, so the
  plan doesn't wait. Maschine's format replaces it only once all of these hold:
  - the event layout is known, from a decompile (reading is allowed);
  - the user confirms that real Maschine shows the lane **and** plays the recorded
    values correctly;
  - DECKARD reads Maschine-saved values back unchanged.
- **Undo.** A recording pass stays one undo step (`begin_take`/`end_take`), expression
  and lane events included.

### Keeping expression with notes

Notes have no id: the doc matches them by sound, position and pitch. Expression must
follow its note through the note edits that already exist:

- **Follows the note:** move, nudge, transpose, quantize, length changes (curves are
  cut at the new end), copy/paste and duplicate (the copy carries a copy of the
  curves), and pattern duplicate.
- **Other edits** *(my call):*
  - split slices the curves at the split;
  - join joins them, with the second note's curves offset;
  - Double Pattern and duplicating or doubling a clip copy them;
  - Groove Variation and Generate keep a note's curves on the note they move;
  - copying, pasting or resetting a sound carries or clears its notes' expression,
    with its notes;
  - Convert to Clip carries it.
- **Clip patterns** are covered too. The block keys patterns as the doc does,
  including the `CLIP + k` list.
- **Removed:** deleting a note deletes its expression. So do Clear Pattern and deleting
  a pattern.
- **Undo/redo** restore it.

How it's held in memory is the builder's choice. One rule: `Note` stays a plain value
that the existing code and Maschine's archive keep using, and expression sits beside it.

**Before part 3:** a note carrying expression gets a small mark in the pattern grid, so
recorded expression is visible. *My call.*

### Saving and loading

- **On save,** if any note in the project has expression (or, per the test above, the
  channel pressure goes in our block), DECKARD writes one `DKRD`/`EXPR` child at the end
  of the root's child list. Nothing else in the file changes.
- **Projects without expression** save with no block, identical to today's output.
- **The payload format** is DECKARD's own, versioned, little-endian, and documented in
  `docs/deckard-block.md`. It holds, per pattern (group index, pattern index), the
  entries for each note:
  - the note's key: sound, position, pitch;
  - for duplicate keys, the ordinal among notes with that key, in note order;
  - release velocity, if any;
  - each present curve's breakpoints and segment curve amounts.

  It is built from the project as it stands when saved, so group and pattern
  reordering need nothing special.
- **On load,** entries whose note isn't found are dropped silently. A block with an
  unknown version, or that doesn't parse, is ignored with a log line, and the project
  still opens.
- **The mxprj crate** needs a way to add a root child on re-save. The existing code
  only swaps frame payloads. With no block to add, the output must stay byte-identical
  (the round-trip tool proves it).
- **Projects re-saved by real Maschine** come back without the block. Their notes load
  without expression. Nothing warns about this. *My call.*

## Left to the builder

- In-memory representation of expression and how it follows note edits.
- The thinning algorithm, within the tolerances above.
- The Preferences UI layout for the MPE setting.
- The engine's path from input to `Recorded`: extending `InMsg`, `Live` and
  `Recorded`, and per-channel note binding.
- The grid mark's look.

## Tests the plan should carry

- **Parsing:** 0xA0, 0xD0 and note-off velocity, and the MPE member/manager routing in
  the table above.
- **Recording:**
  - a recorded MPE note gets Pitch, Slide and Pressure curves with its release
    velocity;
  - MK3 `PadPressure` becomes Pressure on the pad's note;
  - with Note Repeat on, pad pressure changes the repeats' velocity and stores no
    curves;
  - a member channel's bend doesn't reach the sound live;
  - poly aftertouch reaches the right note when two notes are held;
  - CC, bend and channel pressure become lane events;
  - recording over a lane clears its old events as the playhead passes, and a
    second loop pass replaces the first rather than doubling it;
  - an untouched lane is kept;
  - one undo step per pass.
- **Thinning:** a ramp collapses to its end points, and a sine keeps its shape within
  tolerance.
- **Edits:** expression follows move, transpose, quantize, copy and duplicate; delete
  removes it; undo restores it.
- **Save and load:**
  - a project with expression saves a DKRD child, and loading it gives the same curves;
  - without expression the save is byte-identical to today;
  - a bad block is ignored.
- **Real Maschine:** the user opens a DECKARD-saved project with expression, and the
  channel-pressure test project, in Maschine 3. This is a manual check, done once.
