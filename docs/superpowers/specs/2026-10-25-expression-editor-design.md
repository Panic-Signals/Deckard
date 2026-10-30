# Expression editor: design (expression, part 3 of 4)

October 2026. Part 1 records each note's pitch, slide, pressure and release velocity
(`2026-10-24-mpe-capture-design.md`); part 2 plays it out and lets it modulate
(`2026-10-24-expression-out-design.md`). Part 3 lets the user see and edit it, as
Ableton's Note Expression does. Part 4 is DECKARD's sounds responding per voice.

## Why

Recorded expression is only visible as a dot on a note today. The user wants to fix a
smeared pressure swell, flatten an accidental bend, or draw slide into notes that were
played without it, then hear it out of the Hydrasynth or through a route. Maschine has
nothing like it; Ableton's Note Expression is the model the user named.

## Goals

- **Four new lanes** in the pattern editor's lane list (under the note area, both the
  drum grid and the piano roll), after Velocity and Probability: **Pitch**, **Slide**,
  **Pressure** and **Release** (release velocity). They show the focused sound's notes.
  *Built as:* they're listed once the focused sound's notes in the pattern have
  expression; until then the "+" dialog offers them (Expression, per note), so drawing
  on a plain sound is one extra click. The "+" stays pinned at the list's bottom.
- **Curves over their notes.** In Pitch, Slide and Pressure each note's curve is drawn
  across the note's span; a note with no curve shows a faint dashed line at its rest
  value (Pitch 0, Slide and Pressure 0), so an empty note is visibly drawable.
  Selected notes are drawn bright, others dimmer.
- **Release** shows one bar per note, like Velocity; a note without one shows a hollow
  stub. Dragging or clicking sets it (1–127).
- **Pitch over the notes:** in the piano roll, each note's pitch curve is also drawn as
  a line over the note, offset by its bend in rows. Read-only there. *My call:* editing
  happens in the Pitch lane, which works in both the drum grid and the piano roll.
- **Editing in select mode (Ableton's gestures):**
  - Click in a note's span: adds a breakpoint there at the pointer's value (creating
    the curve if the note had none).
  - Drag a breakpoint: moves it in time (between its neighbours and inside the note)
    and value.
  - Drag a segment (between two breakpoints, away from a point): moves both its ends'
    values together.
  - ⌥-drag a segment: bends it (the segment curve amount part 1 left room for).
  - Double-click a breakpoint: removes it. Removing the last one removes the curve.
- **Editing in draw mode:** dragging across notes draws freehand: the stretch the
  pointer covers is replaced by what was drawn, thinned with part 1's tolerances.
- **Which note an edit goes to:** the focused sound's notes whose span holds the
  pointer; if any of those is selected, only selected ones; among them the one whose
  curve passes nearest the pointer. *My call* (the piano roll has chords on one sound).
- **The lane's menu** (right-click): Clear Pitch / Slide / Pressure / Release of the
  selected notes (all of the sound's notes when none are selected), and Clear All
  Expression.
- **Value readout** while dragging: "Pressure 72 %", "Slide 40 %", "Pitch +1.25 st",
  "Release 96".
- **Pitch lane scale:** ±2, ±12, ±24 or ±48 semitones, the smallest that fits the shown
  curves, with a small selector at the lane's top right to choose another. Values snap
  to 0.01 semitone. *My call.*
- **Undo:** every gesture (a click, a drag, a menu item) is one undo step,
  "Edit Expression".
- **Playback and routes** use the edited curves, curved segments included.

## Segment curve

A segment from value `v0` to `v1` with curve amount `c` (−1..1, 0 straight) follows
`v0 + (v1 − v0) · t^e`, `e = 4^(−c · sign(v1 − v0))`: a positive amount always bows the
segment upward, a negative one downward; a flat segment can't bend. ⌥-drag changes `c`
by the pointer's vertical travel (100 px is the full range). *My call on the formula.*

## Non-goals

- Editing on the MK3 (later, with the standalone controller layer).
- Editing channel-wide lanes differently (they keep today's editor).
- Moving a note's time from the lane, or Ableton's "show all notes' expression" toggle.
- Quantizing breakpoints to the grid. Breakpoints land on ticks.

## Anti-goals

- An edit that silently changes a note other than the one under the pointer.
- Breaking Maschine compatibility: expression stays in the DECKARD block.
- Porting NI or Ableton code.

## Left to the builder

The lane's pixel layout, hit tolerances, colours, the number of points freehand drawing
keeps (within the tolerances), and how the editor's code is split into files.
