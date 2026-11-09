# CV outputs and Zhora

Deckard drives a modular synth through a DC-coupled audio interface. The first user setup
is a MOTU UltraLite mk5, whose line outputs are DC-coupled.

Two parts, built in this order, because Part 2 uses what Part 1 adds:

1. **CV routes:** any LFO or envelope can drive one side of a Deckard output as a control
   voltage.
2. **Zhora:** a new instrument that plays a modular voice over CV, listens to it come back,
   and keeps it in tune.

Items marked *(builder's call made in design)* were chosen while writing this and were not
asked for directly. The user reviews them when reading this document.

## Why

The user wants a MIDI note to fire an envelope that opens a VCA in their modular, and an
LFO to sweep a filter, each from one of the MOTU's outputs. Beyond that, they want to play
a modular voice from Deckard like any other instrument: pitch, gate, clock and envelope
out, the finished voice back in, in tune.

**The failure that matters most: notes consistently coming out out of tune.** Everything
in Part 2 is judged against that.

## Goals

- An LFO or envelope drives a chosen side of a chosen Deckard output, at audio rate.
- Zhora plays one modular voice, with pitch, gate, clock, envelope, velocity and mod wheel
  each sent to a Deckard output the user chooses, and glide on the pitch.
- Zhora monitors the modular's return like any other sound in Deckard.
- A one-time Tune measures the oscillator and makes notes land in tune.
- An optional tune-while-playing mode trims drift during held notes.
- A manual tuner is always showing on the right screen.
- All of Zhora is usable from both the hardware controller and the computer screen.

## Non-goals

- Polyphony. One Zhora is one voice. More voices means more Zhoras.
- Aftertouch as CV. *(builder's call made in design)*
- Volt displays or voltage calibration for CV routes. Ranges are percentages.
- CV input (reading the modular's voltages as modulation).
- Capturing the modular in real time during an export.
- Support for interfaces that aren't DC-coupled. They simply won't hold steady voltages.

## Anti-goals

- A wrong note presented as a right one. A note Zhora can't play accurately is clamped and
  flagged, and a failed Tune never replaces a good tuning.
- Any Deckard mixer control changing the pitch CV.
- Audio leaking into a CV output, or CV reaching the monitors by default.
- Tune-while-playing warbling or chasing vibrato, reverb tails or pitch sweeps.

## What the code already has

- Envelopes are a mode of the 16 LFOs (`crates/model/src/lfo.rs`). A sound's notes trigger
  them, live MIDI and sequenced alike (`Engine::trigger_with`), including on an empty
  sound. A muted sound does not fire them.
- `CcRoute` (`crates/model/src/mod_matrix.rs`) sends an LFO to a MIDI CC with a low and
  high range, and is edited on the mod matrix page. It is the pattern to follow.
- `Engine::lfo_value` gives an LFO's value now. CCs read it once per span.
- Deckard has 16 stereo outputs. The audio callback maps each side to an interface channel
  (`routing` in `crates/engine/src/output.rs`), filling `ext[bus]` buffers first.
- Sounds take audio in with `AudioSource::Ext`, but `input_routes` skips a sound whose
  chain starts with an instrument. **Zhora must change that for itself.**
- Native instruments implement `native::Engine` in `crates/plugins` and are listed in
  `native::CLASSES`. **That trait has no way to write to Deckard's outputs or receive live
  analysis of the input, so Zhora needs a new path.**
- Stems and export treat outputs as things to bounce (`crates/engine/src/export.rs`).

## Part 1: CV routes

**CV-only outputs (user's decision):** in Preferences ▸ Audio ▸ Routing ▸ Output, any of
the 32 output channels (Out 1 L … Out 16 R) can be marked CV only. A CV-only channel is
not available as an audio output anywhere in Deckard. It is the only kind of channel the
mod matrix and Zhora can send CV to. The mark is kept per device, alongside that device's
routing and channel names (`Routing` in `apps/viewer/src/settings.rs`).

**Where:** a third kind of route on the mod matrix page, next to modulation and CC routes.

**A route has:**

- **Source:** any LFO or envelope.
- **Destination:** a Deckard output side marked CV only (below), like "Out 2 L", shown by
  its alias if it has one. The user maps that output to a MOTU jack once, in the existing
  audio settings. Routes never name interface channels, so projects survive a change of
  interface. (User's decision.)
- **Low and High:** each -100% to +100% of full scale. The source's 0..1 maps from Low to
  High. A VCA uses 0 to some level. A filter uses, say, -50% to +50% to swing either side
  of its cutoff knob. No separate bipolar option.

**Behaviour:**

- Written sample-smooth. CC-style once-per-span values would click or zipper on a VCA.
- Skips the master, faders, effects and limiters, and the -4..4 monitor clamp must not
  affect it.
- Not included in exports or stems.
- A CV-only channel carries only CV. Audio can never reach it.
- A muted source, or a source that gives no value (`lfo_value` returning `None`), puts the
  output at Low.
- On stop, envelopes release as they do now, so a VCA closes.
- LFOs keep their deliberate restart at the song's start, so playback and exports still
  match. The CV output alone is smoothed for a few milliseconds across that jump so it
  doesn't click. It moves quickly to the restart point. (User's decision.)
- Deckard builds a new `Engine` on each Play (`play_from` in `apps/viewer/src/main.rs`).
  CV outputs must carry on across that swap, with no drop to zero, and Zhora's pitch must
  keep its last value through it.
- With no engine running the outputs are at 0.
- Saved in the Deckard block like CC routes.

**Protections** *(builder's call made in design)*:

- A channel can't be marked CV only while the current project's master or cue plays on
  it, so CV can't reach the monitors or headphones by accident.
- A stereo output with either side marked CV only is unavailable to audio as a whole, and
  the automatic choice of a free output skips it.
- One owner per CV-only channel. One already used by a CV route or a Zhora signal shows
  as in use and can't be picked.
- These hold at runtime, not only when picking. Checked on load and whenever the master,
  cue, routing, marks or routes change:
  - A project sending audio to a CV-only channel (made on another setup, say) has that
    audio dropped and flagged on the strip. This includes the master, which is then
    silent and says why.
  - A CV route or Zhora signal pointing at a channel that isn't marked CV only on this
    machine goes silent and is flagged. It works again once the channel is marked.
  - A plain output channel mapped to the same interface jack as a CV-only channel is
    dropped at that jack, because the callback would otherwise mix the two.
  - Where a duplicate or paste gives one channel two CV owners, the first in project
    order keeps it and the other is flagged.
- A CV-only channel has no stem.

## Part 2: Zhora

A new native instrument named Zhora, added alongside Replicant and Plaits. It goes on a
sound like any instrument.

### Playing

- One voice, newest note wins. *(builder's call made in design)*
- **Pitch:** the note goes through the tuning table to an output value. Pitch bend is
  honoured. *(builder's call made in design)*
- **Gate:** high while a note is held. A new note overlapping the last one drops the gate
  briefly so the modular retriggers. *(builder's call made in design)*
- **Clock:** pulses at Deckard's tempo at a chosen division, default 1/16, only while
  playing. *(division and default are builder's calls made in design)*
- **Envelope:** Zhora's own attack, decay, sustain and release, fired by Zhora's notes,
  separate from the 16 LFO slots.
- **Velocity:** each note's velocity as a steady level, set at the note-on and held until
  the next note. It doesn't scale Zhora's envelope. (User asked for velocity CV; the rest
  is a builder's call made in design.)
- **Mod wheel:** the incoming mod wheel (CC 1) as a level, smoothed so it doesn't step.
  (User asked for mod wheel CV; the smoothing is a builder's call made in design.)
- **Glide:** a time, off by default, that slides the pitch CV to each new note. It slides
  in notes, before the tuning table, so every point of a glide stays in tune. It applies
  to every note change. Tune, Check and Hold ignore it, so measurements land on settled
  pitches. (User asked for glide; the rest is a builder's call made in design.)
- **Unused signals:** any of these outputs can be set to None and is then not sent.
  *(builder's call made in design)*

Gate, clock, envelope, velocity and mod wheel each have a Level, as a percentage of full
scale, default 50%.
Full scale on the mk5 may be well above what a 5 V gate or VCA input expects, and the mk5's
figure isn't known, so the default stays low and the user sets it by ear or with a scope.
*(builder's call made in design)* Gate's and clock's low level is 0, as is the envelope's
at rest. Until Tune has run, pitch uses a rough assumed scaling. Tune starts with small
steps around 0 and widens, so it doesn't depend on that assumption.

Pitch, gate, clock, envelope, velocity and mod wheel each go to a CV-only channel the user
picks. The same rules apply as for CV routes: they skip the master, faders, effects and
limiters, stay out of exports and stems, and hold their channel exclusively. Gate and
envelope go to their low level when stopped; pitch, velocity and mod wheel hold their last
value.

**Timing:** the CV must line up with the output sample-accurately enough that gate and
pitch never arrive out of order. Deckard renders some players ahead of time, so this must
not be computed on a player that runs ahead of the outputs.

CV leaves at the output's time, with the rest of Deckard's audio, and there is no latency
compensation. The modular's return is heard and recorded later by the trip through the
interface, the same as an external synth played over MIDI. (User's decision.)

### Monitoring

The modular's finished voice comes in on an input the user picks, and plays through
Zhora's sound with the same level, effects, mute and recording as any sound.

**Mute** *(builder's call made in design)*: muting Zhora's sound or group behaves like any
muted sound. Its notes don't play, so the gate stays low, and the return is silent. A gate
that is high when the mute lands drops. The tuner, Tune, Check and Hold listen before the
mute, so they keep working while muted. Tune and Check are explicit actions, so they fire
gate and envelope even when muted, and the return is heard during Tune unless it's muted.

### Exports

There is no modular in an offline export, so Zhora's part is left out of mixdowns and
stems. The export warns that Zhora isn't included, so the user can record its return as
audio first if they want it in the mix. (User's decision.)

### Tune (one-time)

The user turns off effects and opens the filter, then presses Tune. Zhora:

1. Opens gate and envelope so it can hear the oscillator.
2. Steps the pitch CV through a range of voltages for a few seconds, waiting for each to
   settle, and measures the pitch.
3. Builds a table from note to output value, finding the lowest and highest notes the
   oscillator and the interface can reach accurately.
4. Replays a spread of notes and measures them. It shows the worst error in cents and
   names any note outside the target.

**Target: every note in the tuned range within 5 cents.** *(builder's call made in design)*

If it can't read a clear pitch at enough steps, it says so and keeps the previous table.

- Notes outside the tuned range clamp to the nearest tuned note and are flagged.
- Before any Tune, pitch uses a nominal scaling and the screen reads "Not tuned".
- The table is saved with Zhora in the project. *(builder's call made in design)*
- Zhora shows when it was last tuned.

### Check

Re-measures a spread of notes in a few seconds and shows the worst error, without
changing the table. It exists because a CueMix output trim, or a turn of the oscillator's
tune knob, changes the pitch without Deckard knowing.

### Tune while playing

An on/off option, off by default. The input is the finished voice after filter, VCA and
effects, so it must be cautious:

- Corrects only during a held note that has settled, ignoring attacks and glides.
- Takes a reading only when pitch confidence is high and the pitch is near the note being
  played. Otherwise it holds the last correction. Delay and reverb tails can carry the
  previous note, so it waits a moment into the note.
- Corrections are slow and limited in size. It trims drift on top of the table and can't
  fix a large error.
- Vibrato and pitch sweeps from the modular survive, because only slow drift is tracked.
- An indicator shows locked on or holding.

### Manual tuner and Hold

The tuner is always live on the right screen, reading whatever comes in on Zhora's input,
at A = 440 Hz, with Deckard's existing note naming.

**Hold** keeps pitch CV and gate on one chosen note, envelope open, so both hands are free
for the oscillator's knob. *(builder's call made in design; user has seen it in the mockup)*

### Screens

**MK3, chosen from mockups (tuner A):**

Left screen:
- Buttons: OUTS, ENV, LEVELS, MOD pages. *(LEVELS replaces the mockup's CLOCK and MOD is
  new, to fit the added signals; builder's call made in design)*
- Knobs on OUTS: Pitch, Gate, Clock, Envelope, each picking from the CV-only channels.
- Knobs on ENV: Attack, Decay, Sustain, Release.
- Knobs on LEVELS: Clock division, Clock level, Gate level, Envelope level.
- Knobs on MOD: Velocity output, Velocity level, Mod wheel output, Mod wheel level.
- Centre: Zhora's name and what it's listening on.

Right screen:
- Buttons: TUNE, CHECK, LIVE, HOLD.
- Bar: last tuned, worst note error and tuned range; what's being held.
- Centre: large note name, cents, and a needle that centres on a green zone.
- Knobs: Input, Hold note, Live on/off, Glide. *(Glide takes the knob the mockup used to
  show the range, which moves to the bar; builder's call made in design)*

**Computer screen:** the same controls and readings in Deckard's plug-in panel, following
how Replicant and Plaits appear there, plus the tuner. *(builder's call made in design;
not mocked up)*

## Left to the builder

- How Zhora reaches the outputs and input: a new engine-level path, or extending
  `native::Engine`.
- Pitch detection method, and where analysis runs. It must not do heavy work or allocate
  on the audio thread.
- Tune's step count, settle time and confidence thresholds.
- Live correction speed and limit.
- Exact mod matrix rows for a CV route, following the CC slot's pattern.
- Clock pulse width and gate retrigger gap.
- Persistence format for CV routes and Zhora's table, following the Deckard block's rules
  (older readers skip what they don't know).

## How we'll know it works

- Unit tests for route mapping, muting, exclusivity, the master protection, and that CV
  stays out of exports and stems.
- A closed-loop test with a simulated oscillator that tracks volts imperfectly: Tune must
  bring every note within 5 cents, and a failed Tune must leave the old table in place.
- Tune-while-playing tests against simulated drift, vibrato and a pitchless signal.
- By hand on the real rig: envelope into a VCA and LFO into a filter with no clicks, then
  Zhora tuned and played against a reference.
