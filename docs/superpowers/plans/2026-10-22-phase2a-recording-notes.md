# Phase 2a — Recording notes (design and plan)

> Roadmap: `docs/superpowers/plans/2026-10-22-parity-roadmap.md`. Inline, test-first.
> Sources: MK3 manual ("Recording Patterns in Real Time", count-in, Note Repeat
> chapter's "output is recorded"), software manual ("Recording Patterns with Real-
> time Recording", "Using your computer keyboard as a MIDI keyboard").

## Behaviour
- **REC** (software header, MK3 REC button / `TransportAccess.startEventRecording`):
  while the song plays, notes played live — pads, MIDI input, the computer keyboard —
  are written into the pattern playing for their group, at the song position they were
  played, overdubbing what's there. REC again (or STOP) ends the take: one undo step
  ("Record").
- **Count-in:** REC while stopped starts playback after a count-in (Preferences ▸
  General ▸ Count-in: Off, 1, 2, 4 bars; default 1) during which only the metronome
  clicks; recording starts at the downbeat.
- **Input quantize** (Preferences ▸ General ▸ Input Quantize: Off, Grid; default Off):
  note starts snap to the pattern editor's grid as they're written.
- **Auto-grow:** recording into an empty pattern lets it grow: it plays as a long
  pattern (32 bars) during the take and is trimmed at the end to the whole bars
  recorded (at least one).
- **Computer keyboard (⌘K / Keys):** two rows play 18 keys, C to F an octave and a
  half (A W S E D F T G Y H U J K O L P ; '), from C3 by default; Z / X octave down /
  up; C / V velocity down / up (steps of 16, 1–127, default 100); an overlay shows
  the range and velocity. The notes play the focused sound at their pitch (as
  keyboard mode). While Keys is on, those letters don't reach other shortcuts.

## How
- **Engine:** `Command::Record(bool)`; while recording and the sequencer runs, each
  live note on and release (`play_live`) is noted with the song tick it played at
  (exact, from the frame). The audio thread hands them over with its telemetry;
  `Output::take_recorded()` gives them to the app. Count-in: `Command::Start { count_in:
  frames }` clicks the metronome that long before the sequencer moves.
- **App:** pairs ons and offs into notes; maps the song tick to the group's pattern
  (the pattern playing in that scene or section, the position within it modulo its
  length); quantizes; writes them with `edit_notes` inside a document take
  (`Document::begin_take` / `end_take`: every edit in between is one undo step).
- **Document:** `begin_take(label)`, `end_take()`.
- **MK3:** `TransportAccess.startEventRecording` / stopping set the app's recording
  (the REC LED follows); the pads record as they play.

## Steps (test-first)
1. Document takes: edits between `begin_take` and `end_take` undo as one step.
2. Engine: live notes while recording come back with their song ticks (and nothing
   when not recording or stopped).
3. Engine count-in: clicks for N bars, sequencer at 0 when it ends.
4. App: recorded notes land in the playing pattern at the right step (pattern-relative,
   wrapping), with lengths; quantize; one undo step per take.
5. App: auto-grow trims an empty pattern to the bars recorded.
6. Computer keyboard: ⌘K toggles; keys play pitches; Z/X/C/V; recorded like pads.
7. MK3: REC from the controller records; LED.
8. Preferences: Count-in, Input Quantize. Docs; full check.
