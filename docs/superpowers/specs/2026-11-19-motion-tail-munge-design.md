# Slice Motion, Tail and Munge (sampler roadmap, phase 3 part 3)

Go-ahead 2026-11-19 ("go ahead and start on part 3"). The roadmap names the three
features; how each works here was not discussed, so every choice below is *my call*.

## Why

Three things a chopped sample still can't do: play another chop on each hit of one pad
(round robin from one file), end a chop cut off mid-sound without a click or a hole, and
change a loop's feel (its swing, its meter) without re-chopping and re-sequencing it.

## What it adds

All three are the New Sampler's (a plain Sampler shows none of them).

- **MOTION** (the sound's; in the zone bar's SLICE box, and a knob on the MK3's PLAYBACK
  page): *Off*, *Next* or *Random*. On, every note plays one slice of the sound, whichever
  key it is: the next in key order (round to the first), or a random one that isn't the
  one before. The slice plays at its own pitch. Stopping the song goes back to the first.
- **TAIL** (a zone's; same box, same page): 0 to 2000 ms. A note that reaches the slice's
  end plays back from there for that long, fading to nothing (the MPC's reverse tail: it
  can't click, the audio being the same at the turn). A slice that loops has no tail. At
  most as long as the slice.
- **MUNGE** (EDIT, software only, like STRETCH): opens a bar with APPLY, CANCEL and HEAR.
  It needs the zone's tempo. It cuts the selection (or the play range) into eighth notes
  by the sample's beats and builds it again:
  - **SWING** −100 to +100 %, **ON** 1/8 or 1/16: each second eighth (or sixteenth) comes
    later (+) or earlier (−); 100 % is a triplet. Negative takes swing out of a loop.
  - **1 to 8**: the eighths of a bar. One switched off is cut out of every bar, so the
    loop gets shorter (4/4 with one off is 7/8).
  - A part made shorter is cut and faded over **GRUNGE** (1 to 30 ms); one made longer is
    filled by repeating its last **COMB** (2 to 100 ms: short buzzes, long echoes), dying
    away.
  - The result is on an even grid at the zone's tempo: its warp markers go, its length in
    beats follows. HEAR plays it without keeping it. APPLY is one undo step, a new file
    as every edit is.

## Non-goals

"Over N slices" for Motion (it goes over all the sound's slices); a tail loop start;
Munge on the MK3; Munge following the song live.

## Anti-goal

A project with none of these set playing or saving any differently.

## Constraints

Motion is a `motion` line in the sound's `deckard:new_sampler` field, Tail a `tail` line
in the zone's `deckard:zone` field: older builds skip unknown lines. Nothing allocates on
the audio thread.

## Approaches

1. **Motion and Tail in the engine, Munge as an edit (chosen).** Motion and Tail are
   heard as played and cost nothing saved but a line; Munge changes the audio, so it is a
   file like Stretch.
2. Tail rendered into the file: every chop would need its own file, and part 2 keeps
   slices as ranges of one.
3. Munge as a live effect: it would have to be a second stretcher; out of proportion.
