# Steps, events and patterns on the controller

Status: **working** for the MK3 (October 2026). NI's own Step, Events and Pattern pages
run unchanged; DECKARD implements the note calls they make
(`crates/lua-host/src/api/events.rs`) and draws the pattern editor widget they show
(`crates/lua-host/src/pattern_view.rs`). Behaviour follows the MK3 manual's "Working with
Patterns and Clips" chapter.

## What works

- **Step mode (STEP).** Pads are the 16 steps of the focused sound, a page at a time
  (Left/Right; FOLLOW pages along with the playhead). An unlit pad adds a one-step note at
  the sound's base key and the pad's velocity (with FIXED VEL on, the fixed velocity:
  0–127, 100 in a new project); a lit pad removes it.
  The new step is selected. Pads light for steps with notes; the playing step flashes.
  Pads don't play sounds here.
  - Knobs 1–4 move, transpose, change the velocity of and lengthen the selected steps (by
    the step grid; SHIFT for fine).
  - DOUBLE (button 3) doubles the pattern, its notes copied into the new half.
  - KEYBOARD (button 4) switches the pads to the focused sound's pitches, and the right
    display to a piano roll drawn as the app's pattern editor draws it: two octaves of
    rows (knob 8 scrolls them) with the keys beside them, each C named, the pitches
    outside the scale tinted red (keys more than rows), the keys sounding lit.
  - SHIFT + pads: 1 undo, 2 redo, 5/6 quantize (all the way / 50 %), 7/8 nudge, 9 clear,
    11 copy, 12 paste, 13/14 a semitone down/up, 15/16 an octave down/up.
- **Events (EVENTS).** Two modes, as in 3.7.1's scripts (the manual describes older ones):
  - *Edit* (the default): knobs 1–4 move, transpose, change the velocity of and lengthen
    the selected notes (only the selection, as the manual says); GRID switches the step
    grid off (edits then move by the smallest step); QUANTIZE (SHIFT: 50 %); CLEAR ALL;
    PROB LOCK.
  - *Select* (button 2): START and END pick the notes of the focused sound in a time
    range, EVENT the next or previous note; ALL and NONE; UP/DOWN pick the sound; a pad
    selects all its sound's notes (again deselects them), with ERASE held deletes them.
  - Left/Right switch the knob pages; page 2's knob 1 is PROBABILITY.
  - Knobs 5–7 zoom and scroll the right display and double its lane height.
  - The 4-D encoder: turn to step through the notes, press and turn to lengthen them,
    left/right to nudge (round the pattern's ends), up/down to pick the sound. Nudging,
    transposing, quantizing, clearing and copying work on what's shown when nothing is
    selected (in Keyboard mode the focused sound's notes, else the group's).
- **Double tap to reset.** On the Step and Events pages, touching the VELOCITY knob
  twice quickly (within 0.4 s) sets the selected notes' velocity to 100, and the
  PROBABILITY knob (the knobs' page 2) their probability to 100 %. One undo step.
- **Patterns (PATTERN).** Pads pick the bank's patterns (an empty pad makes one); CREATE,
  DELETE, DOUBLE, DUPLICATE, and the knobs for position, start and length.
- **Displays.** The left display shows the whole pattern, a row per sound, with a frame
  around what the right display shows. The right display shows the 16 steps (the focused
  sound's notes in a row, underneath each a thin stem topped by a dot for its velocity,
  or with PROBABILITY on its probability) in Step mode, else a lane per sound. In
  Keyboard mode both show the focused sound's notes by pitch. The right display is
  zoomed and scrolled through Maschine's own pattern-editor parameters
  (`crates/lua-host/src/api/pattern_editor.rs`), so the Step page's 16-step pages and
  the Events page's knobs move the same view. Velocity is the notes'
  opacity, selected notes have a white border, and the playhead is a white line. Sizes and
  colours are from the style sheet.
- **Shared with the app.** The selection is the app's pattern editor selection, both ways,
  and the step grid is the app's Grid.

## How Maschine decides the pad mode

No script asks for Step or Pattern pad mode: Maschine picks it from the page on top
(`NHLController:getPadMode()` with no mode set). DECKARD does the same: the Step pages
give `PAD_MODE_STEP`, the Pattern page `PAD_MODE_PATTERN`. Pads play sounds in neither,
nor in `PAD_MODE_NONE` (held MUTE/SOLO, page navigation, Events SELECT).

## Not yet

- The smallest step (SHIFT on the knobs, GRID off) is DECKARD's guess, a 128th note.
- DOUBLE is two undo steps (the length, then the copied notes).
- Audio loops: DECKARD's sounds have none, so the scripts' audio-loop branches never run.
