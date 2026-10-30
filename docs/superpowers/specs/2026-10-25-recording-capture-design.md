# Recording capture: design

October 2026. A fix to live note recording (pads, MIDI, the computer keyboard) after the
user found chords held on the MK3 pads going missing, the first note of a take missing
from step 1, and notes showing only once released.

## What Maschine 3 does

Established by reading the Maschine 3 binary for behaviour (read-only, nothing ported;
scratch notes in `analysis/rec/`, git-ignored). Confidence high unless marked.

- **Written at note-on.** A note goes into the pattern the moment it's played, with a
  placeholder length (1 tick; one grid step with Input Quantize on; a Note Repeat
  note's own step). While held it grows, never shrinks, and on release its length is
  set to exactly the time held (unquantized; it may come out shorter than the
  placeholder). Notes still held when REC stops keep the length they grew to.
- **Two clocks.** A note is *placed* by song position and *measured* by a clock that
  never wraps. A note held over the loop end keeps growing past the end of the
  pattern; it isn't cut, wrapped or recorded again on the next pass. (The note-off
  the user sees at the loop end is playback ending that long note, medium confidence.)
- **Count-in pickup.** During a count-in, a note played in its last 1/16 (240 ticks
  at 960 PPQ) is recorded, placed exactly on the start of recording, its length still
  measured from the real press. Earlier count-in notes are heard, not recorded. With
  REC armed and PLAY pressed there is no pickup: notes from the first block on.
- **Quantize, then wrap.** Input Quantize rounds the start to the nearest grid step
  (half rounds up), then the position wraps by the pattern's length: a note rounded
  up onto the loop end lands on step 1. Starts only; lengths aren't quantized. Note
  Repeat notes aren't input-quantized.
- **No doubles.** Before a note goes in, notes of the same sound and pitch starting
  within its placeholder span are removed: with quantize on, hitting a pad twice on
  one step leaves one note.
- **Same pad again.** A new note-on from a pad or key still held ends its earlier
  note first.
- **A take is one undo step.**

## What DECKARD changes

Today DECKARD writes a note only on its release, measures its length by song position
(so a note held across the wrap becomes 1 tick long), records nothing during a
count-in, and a second press of a held pad loses the first note.

- The engine stamps every recorded note and expression event with `played`, ticks of
  a clock that never wraps (the engine's frame count over the tempo's frames per
  tick), and publishes the clock's current value with the playhead.
- During a count-in, notes (and releases, and expression) in its last 240 ticks are
  recorded, at the song position recording starts from.
- The take writes each note at note-on with its placeholder length, after removing
  doubles; grows held notes about every 100 ms (each growth an edit inside the take's
  one undo step, so the pattern editor shows it stretching); and sets the final
  length (and the expression played on it) on release.
- Note Repeat notes keep their own length and aren't input-quantized.

## Non-goals

- Replace mode (ERASE + REC: erase as the playhead passes, overdub after the wrap).
  Logged in `docs/FEATURES.md`.
- Recording into clips in the Song view beyond what already works.

## Anti-goals

- Porting NI code. The behaviour above is described, not transliterated.
- A take that isn't one undo step.
