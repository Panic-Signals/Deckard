# Chop by playing (sampler roadmap, phase 3 part 1)

Approved 2026-11-18.

## Why

Chopping a sample today means choosing a slicer mode and correcting its markers with
the mouse. People who chop by ear want to play the sample and tap where the pieces
start, to try different sets of starts quickly while keeping the ones they like, and to
play one chop as an instrument.

## What it does

- **Catch.** SLICE, Manual mode, while the sample plays: an empty pad on the MK3 drops a
  slice at the playhead. Pads that have a slice still play it. In the software a **CUE**
  button and the Enter key do the same. The slice snaps to the sample's nearest 1/8 note
  when the zone has a tempo; SHIFT drops it exactly where the playhead is.
- **Slicer.** Split and Grid start from the playhead when the sample is playing, else
  from the selection's start when a part inside the range is selected, else from the
  range's start as now.
- **Random.** A sixth slicer mode with a count (4, 8, 16, 32) and **RE-ROLL**. It picks
  among the places Find weighs (the sample's bars and beats, else its hits).
- **Keep.** In Find and Random a slice can be kept (right-click in the slice; SHIFT + its
  pad). A re-roll or another count leaves kept slices where they are and refills the
  rest. Find's refill takes the kept ones into account (apart from them, unlike them).
- **Key Shift.** A slice's right-click menu and SHIFT + display button 5 on the MK3:
  the slice goes onto the group's next empty sound as one New Sampler zone over all
  keys, rooted at C3, so keyboard mode plays it chromatically. A Sync zone becomes Keys
  there, so its length holds at any pitch; a zone with no tempo plays Repitch.

## Goals, non-goals

- Goal: all of it works from the MK3 and from the software.
- Non-goal: more than 16 slices on pads, exporting slices, Slice Motion (later parts).
- Anti-goal: a pad dropping a slice by surprise. Catching happens only in Manual mode.

## Constraints

- Nothing new is saved in the project: kept marks and the random seed live with the
  slicer's other settings, until APPLY or another sample.
- The slicer's MODE knob on a controller gains Random after Find, as Find was added.

## Approaches

1. **On the slicer as it is (chosen).** The slices stay a list of start frames; catch,
   Random and keep edit that list; Key Shift is a document edit using the sound copy and
   zone operations there already are. Small, and APPLY is untouched.
2. A separate "performance chop" page with its own pads and state. More to learn, and
   two places that slice.
3. Key Shift by re-mapping the current sound's zones. It would take the keys the other
   chops are on.

## Decided details

- The playhead while a slice pad auditions (the prehear) is worked out from when it
  started, since the prehear reports no position; while PLAY plays the zone it is the
  engine's.
- Key Shift copies the sound (its effects and settings come along, its notes don't),
  leaves the focus where it is, and says in the status line which pad it went to.
- Kept marks follow a slice when it is dragged, go when it is removed or all are
  deleted, and when the mode is neither Find nor Random.

## Left to the builder

The look of a kept marker, button placement in the SLICE footer.
