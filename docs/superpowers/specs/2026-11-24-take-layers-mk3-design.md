# Takes onto layers from the MK3, and a visible LAYERS tab

## Why

From the MK3 there is no way to put a recorded take on a layer of a Sampler Preview
sound: RECORD's LAYER knob only says where the *next* take goes, and moving a take is a
right-click in the app. And the MK3's LAYERS tab can't be found: it is display button 4
pressed a second time on ZONE, with nothing on the screen saying so.

## Goals

- Record several takes, then choose from the MK3 which take plays on which layer.
- Nothing in the sound changes until APPLY is pressed.
- The LAYERS tab is visible on the MK3's screen whenever the sound can have layers.

## Non-goals

- Loop and Auto recording (they have targets of their own) and sounds that can't have
  layers (a classic Sampler, an Audio plug-in): unchanged.
- The layers themselves (LAYERS tab's knobs, morph, windows): unchanged.

## Anti-goal

Turning a knob must never replace a layer's sample on the way past it.

## Approaches

1. **Live, passed-over layers restored.** The take moves as the knob turns; layers passed
   get their sample back. Heard at once, but every step edits the project and has to be
   undone again.
2. **Turn to choose, press APPLY (picked by Josh).** The knob only shows a choice; a
   button commits it. One edit, one undo step, the word the rest of the page already uses
   (EDIT's and SLICE's APPLY).
3. **Apply on release of the knob.** No button, but a touch that brushes the knob commits.
   Dropped.

## Design

### Recording (Detect and Sync, a sound that can have layers)

- The first take on a sound with no sample becomes its sample, as today: shown as layer
  1, and still sliceable until a second layer exists.
- Every later take joins the take list, focused, on no layer. The sound plays what it did.

### RECORD's LAYER (the MK3's RECORDING knob page and the app's RECORD footer)

- It is the focused take's layer: `None`, `1`…`n` (the sound's layers), `New` (while it
  has fewer than `MAX_LAYERS`). With no take focused it isn't shown.
- A take's layer is the lane whose sample is the take's file; none: `None`.
- Turning sets a pending choice, kept by the app (`RecorderView`): shown on the knob,
  nothing else changes.
- MK3: display button 8 is APPLY (BANK is gone, see below): lit while the pending choice
  differs from the take's layer, dim otherwise. With SHIFT, 7 and 8 stay PREVIOUS and NEXT.
- App: an APPLY button beside the LAYER field, enabled while the choice is pending.
- APPLY:
  - a layer or `New`: the take onto that lane (`App::take_to_layer`: an existing layer's
    sample is replaced, its settings kept; `New` adds one);
  - `None`, the take being on a layer: that layer is removed (as LAYERS' REMOVE).
  One undo step either way.
- The pending choice is dropped when it is turned back to the take's layer, another take
  is focused, the focused take is deleted, or the focused sound changes.
- The app's right-click "To layer N" stays.

### BANK goes

BANK only did anything with TAKES on and more than 16 takes. Without it, the pads show
the bank of 16 that holds the focused take (SHIFT + PREVIOUS / NEXT walks through them).
[Josh: the button isn't useful]

### The LAYERS tab on the MK3

- On a sound that can have layers and has a sample, display button 4 has both names,
  stacked: the tab showing in the tab row's usual letters, the other small right under
  it. Pressing it goes between the two and the names swap sizes, so it reads as a
  toggle. [Josh] (As now, but it also works on a sound whose one sample isn't a layer
  yet.)
- Other sounds: `ZONE`, as now.

### Controllers run by NI's own scripts

Their Sampling page has no button to spare for APPLY, so RECORD's LAYER knob isn't given
to them; takes are placed from the app. [Claude's call]

## Claude's calls (not asked)

- `None` on a take that is on a layer removes the layer (undoable).
- The first take stays the sound's plain sample (not yet a layer), so SLICE still works.
- ZONE and LAYERS on one button, not LAYERS on button 3: a sound with one sample has both
  SLICE and LAYERS, so button 3 isn't free.

## Left to the builder

- Where the pending choice lives and how the MK3 script learns of it (a field of the
  sampling view next to `take`).
- The manual: RECORD's LAYER and the MK3's LAYERS tab are DECKARD's own, so the manual
  wants a section (the `pull-request` skill asks).
- Tests: in `layers_tests.rs` (takes, pending, APPLY, `None`), `record_knobs.rs` (the
  knob's choices) and the surface's tests (button 8, the tab's label).
