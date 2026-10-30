# Recording notes (REC)

Status: **working**, October 2026 (plan: `docs/superpowers/plans/2026-10-22-phase2a-recording-notes.md`).

- **REC** (the header's ●, the MK3's REC): while the song plays, notes played live are
  written into the pattern playing for their group, where they were played, over what's
  there. The notes come from:
  - the pads;
  - MIDI input;
  - the computer keyboard (⌘K).

  REC again, or STOP, ends the take: one undo step, "Record".
- **From a stop** REC **arms** (lit, nothing plays); PLAY starts the song and the take
  at once. **SHIFT + REC** (shift-click on ●; the MK3's SHIFT + REC) counts in first:
  Preferences ▸ Recording ▸ Count-in (1, 2, 4 bars; default 1, Off counts 1). During the
  count-in only the metronome clicks.
- **Input Quantize** (Preferences ▸ Recording): starts snap to the pattern editor's grid.
- **Auto length** (docs/superpowers/plans/2026-10-23-auto-length-recording.md): a pattern
  no length was chosen for (the default) recorded into from empty starts as **one bar**,
  looping; a note in its last bar adds the next; when the song comes round with the last
  bar empty it **settles** on the bars played (bar 1 played, bar 2 not: it loops bar 1)
  and grows no more. REC off trims it to the bars played (nothing played: as it was).
  Choosing a length (the pattern editor's LENGTH menu, the MK3's Record Prepare: hold
  REC, knob 4, buttons 5–8) turns Auto off; LENGTH ▸ Auto turns it back on. LENGTH and
  Record Prepare show "Auto". Auto is the session's (not saved), as in Maschine.
- **Computer keyboard (⌘K):** A W S E D F T G Y H U J K O L P ; ' play 18 keys from C3
  on the focused sound, held while pressed. Z / X move them an octave; C / V the velocity
  (steps of 16). The status line shows the range and velocity.

## How
- **Engine:** `Command::Record`; while recording and playing, each live note on/release
  (`play_live`) is noted with its exact song position (`Recorded`: tick, mode, length;
  a pad hit has its length, a held note waits for its release). They are handed over in
  the telemetry (`Output::take_recorded`). Count-in: `Engine::count_in(frames)`.
- **App** (`record.rs`): `pattern_at` maps a song position to the group's pattern (the
  pattern alone, the scene's, the section's scene's), wrapping at its length; `Take`
  pairs ons with releases. Notes go in with `edit_notes` inside a document take
  (`Document::begin_take` / `end_take`).
- **MK3:** `TransportAccess.startEventRecording(App, count_in, erase)` /
  `stopEventRecording` set the app's REC (`Request::Recording`); the REC LED follows
  `Workspace:getRecordingParameter()`.

## Not yet
- ERASE + REC (replace recording); recording MIDI controllers and pitch bend; recording
  into clips.
