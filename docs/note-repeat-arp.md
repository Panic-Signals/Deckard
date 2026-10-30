# Note Repeat and the arpeggiator

Status: **working**, October 2026 (plan: `docs/superpowers/plans/2026-10-22-phase2b-repeat-arp.md`).

- **Note Repeat** (pads in group mode): a held pad retriggers on the rate's grid, each
  note `gate × rate` long at its velocity × dynamic.
- **Arp** (pads in keyboard or chord mode, or ⌘K's keys): the held pitches within
  Min.–Max. Key play in TYPE order over OCTAVES:
  - TYPE: Up, Down, Up & Down, Order Played, or Chord (all together).
  - RETRIGGER restarts the cycle after that many steps, rests counting.
  - REPEAT plays each step that many times.
  - OFFSET rotates the cycle.
  - INVERSION adds rotated passes an octave up.
  - SEQUENCE 1–8 rests on some steps. These are DECKARD's own rhythms, since Maschine's
    aren't documented.
- **Timing:** steps fall on the song's rate grid; with the song stopped they run on the
  engine's own clock from the first pad. HOLD latches, and the next press after all are
  let go starts anew.
- **Recording:** what it plays is recorded (REC) as pad hits are.
- **Input:** only pads and the computer keyboard go through it; MIDI input doesn't (as in
  Maschine).
- **On:**
  - the MK3: while the NOTE REPEAT page shows, or with LOCK (button 2). SHIFT + NOTE
    REPEAT switches the pads to Keyboard mode (the Arp) as Maschine 3's scripts do (the
    MK3 manual says in one place it toggles LOCK, in another this);
  - DECKARD's own controller layer: the same, its `pages/note_repeat.lua`;
  - the software: View ▸ Note Repeat / Arp ▸ On.
- **Settings** (one set per project, saved in it as Maschine saves the song's
  `arpeggiator`): rate (1 BAR … 1/128) and unit (Normal, Triplet, Dotted), stored as
  ticks; type, sequence, octaves, dynamic, gate, hold, retrigger, repeat, offset,
  inversion, min/max key; presets 1/4, 1/8, 1/16, 1/32 on buttons 5–8.

## How
- **Model:** `Project::arp` (`Arp`, `ArpType`), read from the song's `arpeggiator`.
  `pc0` is the rate in ticks; `octaves` and `offset` are stored minus 1 and plus 8.
- **Engine** (`crates/engine/src/repeat.rs`): `Repeater`, between live input and sounds
  (`Engine::set_repeat`, `Command::Repeat`). Held live notes (`NoteOn` held: pads, keys)
  feed it. Each block it queues the notes due as live notes with lengths, so they play,
  and record, sample-accurately. MK3 pads now send held notes (`NoteOn`) instead of MIDI
  input notes.
- **MK3** (`crates/lua-host/src/api/arp.rs`):
  - `NI.DATA.getArpeggiator(App)` and its parameters;
  - the rate, unit and preset parameters;
  - `NI.HW.getArpeggiatorActiveParameter`;
  - `SongAccess.isArpRepeatLocked` / `setArpRepeatLockState`.

  Values and texts come from the app (`State::arp`) and edits go back (`arp_edits`);
  `apps/viewer/src/arp_ui.rs` turns them into the project's fields.

## Not yet
- Maschine's own eight sequence rhythms; the presets' other stored settings
  (`arp_presets`); swing on the output; the chord-mode chord as one "pitch".
- Done since: pressure on a held pad or key (poly pressure, MPE, the MK3's pads) changes
  the repeated notes' velocity (`Repeater::pressure`).
