# Per-note ratchet: design

## Intent

A note can ratchet: play as several quick hits inside its own length, with the hits'
velocities flat, falling or rising. The user sets it per note, in the window and on the
MK3. Because the hits fill the note's length, dragging a note longer keeps it repeating
for as long as it lasts; that is a wanted effect, not a side effect.

Maschine has no such feature. This is DECKARD's own.

## Goals

- Select one or more notes and give them a number of repeats and a velocity shape.
- Edit it in the window's pattern editor and on the MK3's Events and Step pages.
- A ratcheted note stays one note: it moves, copies, duplicates, lengthens and deletes
  as one, and shows its hits.
- Projects stay openable in Maschine, which plays the plain note.

## Non-goals

- Ratchets that run past the note's end, or at a rate set apart from its length.
- Per-hit editing (pitch, length or velocity of one hit).
- Ratchets on recorded MIDI events, audio patterns, or Note Repeat's output as it's played
  live (a recorded note can be given one afterwards like any note).
- Swing or groove applied between a note's hits.

## Anti-goals

- A plain note (repeats 1) must sound and export exactly as it does today. A project
  with nothing in the DECKARD block still saves with no block, byte for byte as today.
  (*Builder's call:* a project that already has a block gets the new version number and
  an empty section, as each earlier version bump did.)
- The MK3 gets no new button; it goes on the knobs.

## Behaviour (decided with the user)

A note's ratchet is four values:

| Value | Range | Default | Meaning |
|---|---|---|---|
| Repeats | 1–16 | 1 | How many hits. 1 is an ordinary note. |
| Shape | Flat, Down, Up | Flat | How the hits' velocities run. |
| Floor | 0–127 | 0 | Down's target velocity. |
| Ceiling | 0–127 | 127 | Up's target velocity. |

**Timing.** With N repeats the note's length L is split into N equal parts. Hit k
(0-based) starts at `position + k·L/N` and lasts `L/N` (the last takes what's left of L,
so the hits add up to L exactly). A note too short to give each hit one tick plays as
many hits as it has ticks, and that smaller count is the N in the velocity lines below
(so Up still ends on the Ceiling).

*Builder's call:* a hit is cut the way a note is today, hit by hit. A hit that would start
at or after the pattern's end (or the pass's end: the loop point, a scene change) doesn't
play, so a ratchet hanging over the end never fires into the next pass; the last hit that
does start rings out as a plain note would. A note that starts before the pattern's start
is left out whole, as now.

**Velocity.** Hit 0 always plays at the note's own velocity V.

- Flat: every hit at V.
- Down: the hits run in a straight line from V toward Floor, which the line reaches at
  the note's end, so hit k plays at `V + (Floor − V)·k/N`. The last hit is above the
  floor and still sounds. A Floor above V is treated as V (flat).
- Up: the hits run in a straight line from V to Ceiling, which the last hit plays:
  hit k is `V + (Ceiling − V)·k/(N−1)`. A Ceiling below V is treated as V (flat).

Velocities are rounded and kept within 1–127, so a hit never goes silent.

**Probability.** One roll for the whole note: all its hits play or none do. PROB LOCK
keeps that roll as it does a plain note's.

**Everything else follows the note.** Mute, the sound's swing (applied to the note's
start; the hits keep their even spacing after it), choke groups, the piano roll's pitch,
note expression (the curves run over the note's whole length, each hit reading its own
stretch), export and bounce (the hits are in the audio and in exported MIDI as separate
notes).

## Approaches

- **A. A ratchet on the note (chosen).** `Note` gains the four values; the engine splits
  the note into hits when it builds what plays. Saved in DECKARD's block. The note stays
  one editable thing, which is what makes "lengthen it and it keeps repeating" work.
- **B. Bake the hits as real notes.** Maschine would play them, but the note is no longer
  one note and can't be lengthened into more repeats. Rejected for that.
- **C. A per-sound ratchet module.** Can't differ per note. Rejected.

The user left the how to the builder; A is the builder's pick.

## How it's built

### Model (`crates/model`)

`Note` gains `ratchet: Ratchet`, a small `Copy` struct `{ repeats: u8, shape: Shape,
floor: u8, ceiling: u8 }` whose `Default` is the table's defaults. A pure function gives
a note's hits, `(offset_ticks, length_ticks, velocity)` for each; the engine, the editor's
drawing and the tests all use that one function.

### Saving (`crates/doc`, `docs/deckard-block.md`)

A new section at the end of the DECKARD block, version 13: per pattern, the notes whose
ratchet isn't the default, each found the way expression entries are (sound, position,
pitch, ordinal), with the four values. The block is written only when something is in
it, as now. Versions 1–12 still read. Edits that move, copy or delete notes carry the
ratchet because it is on the `Note`; the document's edit paths that rebuild notes
(`set_notes` and the like, undo and redo, copy and paste, duplicate, double, quantize,
nudge, transpose) must keep it.

### Engine (`crates/engine`)

Where the timeline's `NoteEvent`s are made from a pattern's notes, a ratcheted note
becomes one event per hit. The hits share one probability roll (the first hit's), so
they play or stay silent together, and PROB LOCK holds for them. Audio export and
bounce go through the same events. Export MIDI writes straight from the notes
(`apps/viewer/src/midi_file.rs`), so it expands each note with the hit function itself.

### The window (`apps/viewer`)

- **A Ratchet lane** beside Velocity and Probability under the pattern editor: a bar per
  note, 1 at the bottom to 16 at the top, dragged like the others, and applying to the
  selection as they do.
- **The note's right-click menu** gains a Ratchet submenu: Shape (Flat, Down, Up), and
  Floor and Ceiling as number fields, both always there (*builder's call*, so a mixed
  selection needs no special case). Each sets that value on every selected note.
- **Drawing:** a ratcheted note shows a thin tick at each hit's start, in both the drum
  grid and the piano roll, and the Velocity lane shows a stem per hit at its velocity
  (the first full, the rest fainter), so the shape can be seen.

### The MK3 (`crates/surface`)

Page 2 of the knobs on the Events page and the Step page (and the Step Grid, which shares
them), where PROBABILITY is alone today:

```
 knob 1         knob 2      knob 3      knob 4
 PROBABILITY    RATCHET     SHAPE       FLOOR / CEILING
   100 %          4          Down         20
```

- RATCHET turns 1–16; SHAPE steps Flat, Down, Up; knob 4 is FLOOR while the shape is
  Down, CEILING while it's Up, and blank while it's Flat.
- They act on the selected notes, as the other knobs do; with notes that differ, the
  value shows as the others show mixed values.
- *Builder's call*, a selection of mixed shapes: knob 4 is named TARGET, and turning it
  moves each note's own target (a Down note's floor, an Up note's ceiling) and leaves
  Flat notes alone. `ratchet_target` means exactly that for any selection.
- The pattern drawn on the screens ticks ratcheted notes as the window does.

The scripts reach it through the existing `deckard.events(...)` calls with new names
(`ratchet`, `ratchet_shape`, `ratchet_target`), and the note values in `deckard.state()`
gain the four fields.

## Left to the builder

- The exact look of the ticks and the lane's bars.
- How the hits share a probability roll inside the engine.
- Whether `Ratchet` is stored inline on `Note` or behind an `Option` (inline is assumed
  above; `Note` is built in many tests, so `..Default`-style construction may be needed).
- The manual: a section in `differences-from-maschine.md` and the MK3 chapter, as a
  manual pull request when this one is opened.

## Tests

- The hit function: counts, offsets summing to the length, each shape's velocities, the
  clamps (floor above V, ceiling below V, a note shorter than its repeats).
- Engine: a ratcheted note sounds N times at the right frames and levels; at 50 %
  probability its hits play or drop together; a plain note's events are unchanged.
- Save: a ratchet round-trips; a project without one saves as before; a version 12 block
  still reads.
- Edits keep it: move, copy and paste, duplicate, double, quantize, undo and redo.
- The window: dragging in the Ratchet lane and the menu set it on the selection.
- The MK3: the three knobs set it, knob 4's name follows the shape, mixed values show.
