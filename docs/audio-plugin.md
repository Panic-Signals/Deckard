# The Audio plug-in

Maschine's Audio plug-in plays audio loops in time with the project: a loop at 94 BPM in
a 120 BPM song plays at 120 BPM, its pitch kept (the Stretch and Formant engines) or
moving with the speed (Re-pitch). This is the spec DECKARD builds it from.

Sources: the Maschine 3 software manual and the MK3 manual (2026 editions; "Using the
Audio Plug-in", "Working with Plug-ins"), Maschine's own projects that use it (the
expansion libraries' demo projects: 66 Audio modules in 80 projects, read with
`mxprj-dump --decode`), the strings in Maschine's binary, and NI's controller scripts.
Nothing of NI's code is used.

## What it does (the manual)

- **Mode: Loop or Gate.**
  - Loop (default): the loop plays with the pattern, without notes, from the pattern's
    start, repeated to fill the pattern (a shorter pattern starts it over early). Its
    waveform shows in the pattern editor; double-clicking it switches the loop off in
    that pattern, and on again. A loop loaded or recorded is on in the pattern.
  - Gate: the loop runs with the song but is heard only during notes; with Stretch the
    note's pitch transposes it (C3 the original pitch, C4 an octave up). FADE (Gate only)
    fades each gate in and out against clicks.
- **Tune** −36…+36 semitones (SHIFT: cents). **Pitchbend** range −12…+12 semitones,
  relative to Tune, for a controller's pitch wheel.
- **Engine:**
  - **Re-pitch:** the loop's speed follows the tempo, and its pitch with it (a record
    played faster). Sharp transients, little CPU. In Gate mode notes only gate.
  - **Stretch:** tempo and pitch independent; polyphonic material; notes in Gate mode
    pitch it.
  - **Formant:** as Stretch, keeping the formants (voices don't go "cartoon" when
    pitched).
- **Source: Tempo and Length**, the loop's original tempo and length (in beats), set on
  loading ("Maschine will seamlessly determine tempo information from the Sample"),
  editable if it guessed wrong. They describe the take in focus.
- **Takes / Audio Pool:** an Audio module holds takes (recordings, loaded files); each
  pattern plays one ("changing the selected take only affects the current Pattern").
- **Sample Editor:** the RECORD and EDIT tabs (no SLICE or ZONE); EDIT's play range
  start and end mark the part of the file that loops; destructive edits as the Sampler's.
- **Loading:** a sample tagged as a loop dragged onto an *empty* sound loads into an
  Audio plug-in, placed in the pattern, tempo-detected and stretched to the project.
  Loop recording loads takes into it; stems (EDIT ▸ STEMS) and Sample to Sound
  ("rendered samples are loaded as Takes in the Audio Plug-in") do too.

## How Maschine stores it

`NI::MASCOMMON::DATA::AudioModule` (class version 7; `crates/mxprj/src/types/module.rs`):

| field | type | meaning | evidence |
|---|---|---|---|
| `takes` | `ObjectVector<Take>` | the takes, and the focused one | |
| `mode` | enum | 0 Loop, 1 Gate | sounds with mode 1 have notes (35 of 40), mode 0 not (1 of 26) |
| `fade` | float | Gate's fade, normalized; 0.1222 new | default in 62 of 66 |
| `tune` | float | semitones | values −23…22 |
| `pitchbend` | float | normalized; 0.5 new | always 0.5 |
| `engine` | enum | 0 Re-Pitch, 1 Stretch, 2 Formant | the binary's "Set AudioModule Engine Mode: 0=Re-Pitch, 1=Stretch, 2=Formant"; always 1 |

`NI::MASCOMMON::DATA::Take` (version 2):

| field | meaning |
|---|---|
| `file` | the sample (`FileLocation`) |
| `length` | Source Length in beats (`QuantizedFloatParameter`; the binary: "Num. beats (double)"). The source tempo is `length × 60 × rate / frames`: 16 beats of 450 372 frames at 44.1 kHz = 94.0 BPM ("Bass[94] Amber.wav"); every take checked gives its file name's tempo |
| `play_range` | frames played (low, high) |
| `selection_range` | as a Zone's |
| `offset`, `unknown_98` | the sample editor's view state: `offset` is Scroll X and `unknown_98` the FocusFrame, where the view looks. They don't move the audio in a pattern (checked against Maschine 3) |
| `zoom_factor`, `quick_browse` | |

Per pattern, the sound's sequence holds an `NI::MASCHINE::DATA::AudioEvent` (a
`LengthEvent`: position 0, length `i64::MAX`, and the `take` it plays). No event: the
loop is off in that pattern. In Gate mode the same sequence holds the notes.

Maschine puts a take's first beat at the pattern's 1.1.1 and cannot move it: in a pass the
read position is `(time since the pass + the pattern's Start) mod the pattern's length`,
wrapped in the take's play range. A Maschine project always plays from 1.1.1.

## Not confirmed

- How FADE and PITCHBEND's normalized values map to milliseconds and semitones (the
  manual gives Pitchbend's range only).

## How DECKARD does it

- **Model** (`crates/model`): `Sound::audio` (`AudioLoop`: takes, the focused one, mode,
  fade, tune, pitchbend, engine), `Take` (sample, beats, play range; `tempo()` from its
  frames), and `Pattern::audio` (`AudioCue { sound, take, offset }`: a sound's take on in the pattern, and
  where it sits).
- **Offset (DECKARD only).** Maschine can't slide a take; DECKARD can. `AudioCue::offset`
  is in ticks (960 per beat) from 1.1.1: the pattern time at which the take's first frame
  plays. 0 is Maschine's placement and every new take starts there. Each pattern places the
  same take on its own. It is saved as `deckard:audio_offset` on the AudioEvent in the DECKARD
  block (version 11), when it isn't 0, and kept out of Maschine's archive, so Maschine plays
  the take from 1.1.1.
- **The drag.** In the pattern editor, drag a sound's waveform (pad view: the sound's row;
  piano roll: the focused sound's waveform, anywhere there is no note) to set the offset. The
  waveform follows the pointer and the offset shows as bars.beats.sixteenths. It snaps to the
  editor's grid; hold SHIFT to move to the tick. The offset wraps in the take's length. One
  undo step, "Move Audio". The controller shows the offset but doesn't set it.
- **Document** (`crates/doc/src/audio.rs`): `audio_only` (a lone Audio plug-in),
  `set_takes`, `add_take`, `set_take_sample` (an edit: the beats scaled with the frames, so
  the tempo stays), `set_audio_in` (AudioEvents on or off in patterns), and
  `set_audio_offset` (the cue's offset, one undo step, "Move Audio"), and
  `Target::Take(g, s, k)` for a take's fields; the module's are `Target::Module(g, s, 0)`.
  Written as Maschine writes them (a take's `length.t1` its duration in beats at 120).
- **Engine** (`crates/engine/src/audio_loop.rs`): the player inside the sound's Sampler
  runtime. The timeline cues it at each pattern instance (`Cue::On(take, shift)`, where
  shift is the cue's offset minus the pattern's Start) and stops it at the end. Its place in
  the take is `(t - offset) mod take length`, with t counted from 1.1.1 (Start included), as
  Maschine's formula at offset 0. Before the offset the take wraps round, so a loop still
  fills the pattern. A take switched on while the song plays is heard at once, at the
  matching place; while the transport is stopped it waits for play. Re-pitch reads it at the rate the tempo asks;
  Stretch and Formant go through `crates/stretch` (Signalsmith Stretch 1.3.1, MIT,
  vendored: `seek` pre-roll and the output latency dropped, so it's on the beat; it starts
  over where the song is when it drifts or jumps). Gate mode opens on notes (FADE: 0–100
  ms), each transposed from C3. Tune and the pitch wheel move Stretch's and Formant's
  pitch; Re-pitch's pitch is its speed. Sounds rendered ahead get the cues too; a seek
  recues every loop; an edit's player adopts the old one's place.
- **Loading:** a sample dropped on a sound with a Sampler is a zone more of it; on another
  sound it's an Audio plug-in's loop (`App::load_loop`), its length in beats from a tempo
  in its name ("[94]", "94bpm"; `apps/viewer/src/loops.rs`) or else the beats nearest the
  song's tempo, on in the focused pattern, which grows to hold it. Stems (EDIT ▸ STEMS) are
  Audio plug-ins in their new group, at the source's beats, playing in the focused scene.
  Loop recording (Target Take: one take more; Sound; Pattern) loads into an Audio plug-in;
  a sound with a Sampler keeps it. Edits on an Audio sound's take (EDIT) point the take at
  the edited copy.
- **Recording** (`apps/viewer/src/main.rs`), by Maschine's rules for audio takes:
  - The destination is the sound that was focused when the recording started, not when it
    stopped.
  - Detect and Sync onto a sound that already has an Audio plug-in add a take to it, selected
    and on in the pattern; they don't swap in a Sampler. Detect's take length is the
    recording's beats rounded to the nearest sixteenth, so it repeats on the grid.
  - Loop with Target Sound and no free sound left adds a take to the last sound of the group
    that has an Audio plug-in. Auto keeps the group's last sound.
  - Every new take is at offset 0. External inputs are not latency-compensated: drag the
    waveform to put a late take right.
- **Software:** the plug-in's page (`audio_ui.rs`: PLAYBACK MODE, FADE, TUNE, PITCHBEND;
  ENGINE; SOURCE TEMPO, LENGTH). The pattern editor draws each loop on its sound's row
  (the focused sound's behind its notes in the piano roll), placed by beats from 1.1.1 and the
  cue's offset, so its transients sit on the steps whatever the tempo: the first pass in the sound's colour,
  repeats darker (before the offset too), dim where it's off; double-clicking a Loop-mode row switches it on or
  off in the pattern.
- **Controller:** the Control page shows the same page. The pattern editors draw the loops
  in their lanes as Maschine's `ClipAudioLane` style has them (`HW/MaschineStudio/
  ArrangerPage.txt`: 10 pixels of a 12-pixel lane, 18 of 24; the sound's colour, repeats in
  palette 19, greys muted or off; Gate lit under its notes; the piano roll's palettes 7 and
  6), from peaks the app hands over (`State::loop_peaks`), placed by the app's rule: place
  in the take `(t - offset) mod loop length`, the first pass bright, every other pass darker. `hasAudioModuleInLoopMode`,
  `isAudioLoopEnabled`, `getAudioLoopLength`, `toggleAudioLoop` and `Sound:getAudioModule`
  work, so STEP lights a loop's steps while it's on and a step pad switches it.
