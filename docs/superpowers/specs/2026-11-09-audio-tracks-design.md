# Audio tracks: recording as Maschine does it, and sliding the audio on the grid

**Status:** draft for review, 2026-11-09.
**Area:** Sampling ▸ RECORD (`apps/viewer/src/recorder.rs`, `App::finish_recording` in
`apps/viewer/src/main.rs`), the Audio plug-in (`docs/audio-plugin.md`,
`crates/engine/src/audio_loop.rs`, `crates/doc/src/audio.rs`), and the pattern editor
(`apps/viewer/src/layout.rs`).

## Why

The user recorded a 4-bar loop from inside DECKARD (Sync, 4 bars) in a pattern whose start
was at 3.2.1. Recording stopped after 4 bars and then nothing happened: no waveform in the
pattern editor, nothing playing. The take had gone into the sound's Sampler as a zone.
Maschine does that too for a Sync recording on an empty sound; only Loop mode, or a sound
that already has an Audio plug-in, makes an audio track.

The user's rule for this work: **everything matches Maschine, with one addition: the
audio can be dragged along the pattern grid.** Maschine puts a take's first beat at the
pattern's 1.1.1 and has no way to move it.

## Maschine's behaviour (the spec)

From Maschine 3's software manual ("Using the Audio Plug-in", "Recording audio":
`analysis/rec/Maschine_3_Software_Manual_English.txt` lines 6898–7160 and 19272–19560; the
MK3 manual says the same) and Maschine 3's binary (checked 2026-11-09):

- **Which recordings become audio tracks.** Detect and Sync: if the focused sound has
  neither an Audio nor a Sampler plug-in, the take loads into a Sampler, mapped to one zone
  covering all keys and velocities, existing zones replaced. Loop loads into an Audio
  plug-in "so that you can immediately hear the results"; a sound that already has a
  Sampler keeps it, its takes preserved. A recording in any mode onto a sound that already
  has an Audio plug-in becomes a take of it. Auto makes zones.
- **When recording starts.** Detect: on the threshold, or at once with it OFF. Sync: the
  next bar. Loop: the beginning of the pattern ("Waiting for end of Pattern…"; in Song
  view the grid is a bar).
- **Which sound.** "The recorded audio will be stored in the Sound that was under focus as
  you started the recording." Loop's Target: **Take**, a new take of the focused sound,
  selected for playback; **Sound**, an empty sound of the group, layered on the earlier
  ones, and when none is left a new take of the **last sound of the group that contains an
  Audio plug-in**; **Pattern**, as Sound plus a new pattern where only this take plays (the
  newest is off in the previous pattern).
- **After recording.** The take is appended to the sound's Audio Pool and selected; the
  sound takes the recording's name; existing notes for that sound stay (in Gate mode the
  take plays at their pitch); a Loop take is on in the pattern and plays with it.
- **Internal source.** "The output of any available Group or the Master." No single sound
  (solo it and pick its group). A single sound only in Auto.
- **Playback.** A take is locked to the pattern's 1.1.1: in a pass, its read position is
  `(time since the pass + pattern Start) mod pattern length`, wrapped in the take's play
  range; frames per beat = the file's frames ÷ the take's Length in beats. A shorter take
  repeats to fill the pattern (repeats drawn darker); a shorter pattern starts it over.
  Double-clicking the waveform switches the audio off or on in that pattern.

## What DECKARD changes

DECKARD already does most of this (`docs/audio-plugin.md`, "How DECKARD does it"). The
differences to close:

1. **A Detect or Sync recording onto a sound with an Audio plug-in** replaces the plug-in
   with a Sampler today (`load_take` → `sampler_only`). It must add a take to the Audio
   plug-in, selected and on in the pattern, as Loop's Target Take does.
2. **Target Sound with no empty sound left** goes to the group's last sound today
   (`next_free_sound`). It must go to the last sound of the group with an Audio plug-in.
3. **The sound is the one focused at Start**, not at Stop. Check this holds for every mode;
   fix it where it doesn't.
4. **Playback honours the pattern's Start.** Today the engine puts the take's first beat on
   the pass start, ignoring the pattern's Start. It must follow Maschine's formula above,
   so a take sits where Maschine puts it (and notes and audio move together).
5. **Heard at once.** When a take that is on in the pattern now playing finishes
   recording, the engine is cued straight away at the matching place in the take, rather
   than at the next pass. When the transport is stopped, or another pattern is playing, it
   waits for that pattern to play.

Everything else stays as it is, including the internal source list (Master and groups).

## The addition: drag the audio on the grid

### The offset

- New per-pattern field: `AudioCue::offset`, in ticks (960 per beat), measured from 1.1.1
  like note positions: the pattern time at which the take's first frame plays. 0 is
  Maschine's placement, and every new take starts at 0.
- Playback: `beat_in_take = (pattern_time − offset) mod take_length`, with `pattern_time`
  as in Maschine's formula (counted from 1.1.1, Start included). Before `offset` the take
  wraps round, so a loop still fills the whole pattern; a take longer than the pattern
  still starts over each pass.
- Saved in the DECKARD block (`crates/doc/src/block.rs`, version 11): per group, pattern and
  sound, the offset when it isn't 0. Maschine's `AudioEvent` is unchanged and no audio is
  rotated, so a project opened in Maschine plays the take from 1.1.1, as Maschine would.
- The take a pattern plays and its offset belong to the pattern; the takes belong to the
  sound. Each pattern can place the same take differently.
- Undoable ("Move Audio").

### The drag

- **Pad view** (one row per sound): press on a sound's audio lane and drag left or right.
  The waveform follows the pointer; release sets the offset. It snaps to the editor's step
  grid; hold SHIFT to move freely, to the tick. Double-click still switches the audio off
  or on.
- **Piano roll:** the focused sound's waveform is behind its notes. Dragging on the
  waveform where there's no note slides the audio; dragging a note moves the note, as now.
- The offset wraps within the take's length (sliding by a whole take changes nothing) and
  shows as bars.beats.sixteenths while dragging.
- The pattern editor and the MK3's pattern lanes draw the waveform from the offset. The
  MK3 shows it but doesn't set it.

## Non-goals

- Anything Maschine doesn't do, other than the drag: single sounds as an internal source,
  placing a take automatically where it was recorded, arranger clips, comps of several
  takes in one pattern, punch-in.
- Trimming or editing the take from the pattern editor (EDIT does that).
- Dragging on the MK3.

## Calls made in review (not the user's; check these)

- Detect's take Length is the recording's duration in beats, rounded to the nearest
  sixteenth so it repeats on the grid (Stretch absorbs the difference). A tempo change
  during a recording uses the average tempo over the recorded range, as Maschine's
  `getRecordingTempo` does.
- External inputs are not latency-compensated (unchanged). Dragging the waveform is how a
  late take is put right.
- The pattern grows to hold the take (doubled, as Loop does now); the offset doesn't
  affect growth.

## Left to the builder

- How the cue carries the offset to `audio_loop.rs` (a field on `Cue::On`, or a start
  beat).
- Whether the drag starts anywhere in the lane or only over the waveform.
- How the MK3 lanes get the offset (`State::loop_peaks` or alongside).

## Decided by the user (2026-11-09)

- Match Maschine in everything, except the drag.
- Which recordings become audio tracks: as Maschine.
- No single-sound internal source.
- The drag works in both pattern editor views, pad view and piano roll.
