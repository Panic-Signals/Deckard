# Slices to pads and to files (sampler roadmap, phase 3 part 2)

Go-ahead 2026-11-19 ("Let's go on and start building it"), on the outline given in the
conversation with its recommended choices. The choices below marked *(my call)* were not
discussed.

## Why

APPLY puts every slice on a key of one sound. That is right for playing chops on a
keyboard, but someone who wants each chop on its own pad (its own effects, mute, choke
and level) has to build that group by hand, and someone who wants the chops as files has
no way to get them. The counts also stop at 32.

## What exists already

- APPLY puts any number of slices on keys from C1 in one sound.
- APPLY's **Create Pattern** and **Replace Pattern** write one note per slice, in order.
- On the MK3's SLICE page the pads show the 16 slices around the selected one.

## What it adds

- **64.** Split, Find and Random gain a count of 64.
- **BANK** on the MK3's SLICE page: SHIFT + display button 7 steps the pads to the next
  16 slices (round to the first), when there are more than 16.
- **Apply to pads.** APPLY gains **TO**: *Keys* (as now) or *Pads*. Pads makes new groups
  at the end of the group list, 16 slices each: every slice is a copy of the sliced sound
  (its effects and settings, not its notes) with the slice as its only zone, over all
  keys with its root at C3. The group is named after the sound (a number added from the
  second group on), the sounds "<name> <slice number>". The current group and the
  slicer's slices are left as they are, and the focus stays. One undo step.
  - With **Create Pattern** or **Replace Pattern**, each new group gets a pattern with a
    note per slice at the slice's time, so the groups' patterns played together play the
    sample. *(my call: Replace means the same as Create here, there being nothing to
    replace.)*
  - **EACH PLAYS TO** applies as it does for keys.
  - A zone's tempo and play mode come along unchanged (a Sync slice stays in Sync); a
    zone with none plays Repitch, as with Key Shift. *(my call)*
- **ORDER** in APPLY: *In order* (as now) or *Shuffled*: the notes keep their times and
  each plays another slice. The shuffle is the slicer's seed's, so the same until
  RE-ROLL. *(my call: one shuffle, not the Octatrack's lock types.)*
- **EXPORT** (software only): a button on SLICE asks for a folder and writes each slice
  as a 24-bit stereo WAV named "<sample> 01.wav", "<sample> 02.wav" and so on. It asks
  before replacing files that are there. *(my call: no export from the MK3, which has no
  way to choose a folder.)*

## Non-goals

Dragging slices out of the window; slices as references with their own loop points;
Slice Motion, Tail Length and Beat Munger (part 3).

## Anti-goal

APPLY to pads changing or replacing anything in the group being sliced.

## Constraints

Nothing new is saved in the project: TO and ORDER live with the slicer's other settings.

## Approaches

1. **New groups at the end (chosen).** Non-destructive, and any count fits.
2. Fill the current group's pads from pad 1: overwrites what's there and stops at 16.
3. One group with the chops as zones on 16 sounds' keys: neither one thing nor the other.
