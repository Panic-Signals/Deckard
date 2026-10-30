# Expression out: design (expression, part 2 of 4)

Status: proposed, 2026-10-24. Follows part 1, capture and keep
(`docs/superpowers/specs/2026-10-24-mpe-capture-design.md`), and Pad Touch
(`docs/superpowers/specs/2026-10-24-pad-touch-design.md`).

## Why

The user records per-note expression today (Pad Touch Poly AT on the MK3, the
Launchpad's poly aftertouch, an MPE controller): each note gets a dot and keeps pitch,
slide and pressure curves. Nothing plays them back, so a take sounds like plain notes
and the user had no way to tell the recording worked. This part makes recorded and live
expression reach the synths: hardware over MIDI first (the Hydrasynth, in its normal
mode with poly aftertouch, or in MPE mode), then any instrument or plug-in parameter.

## Parts, in order (the user agreed)

- **2a, MIDI out:** per-note curves play out to MIDI ports, in a normal or an MPE
  format chosen per output port.
- **2b, expression as a source:** Pressure, Slide and Pitch can be assigned to any
  instrument, effect or plug-in parameter, like a macro. Needs plug-in parameters to be
  reachable as targets first (today they can't be learned or made macros; PARITY row 3).

2a is planned and built first; 2b gets its own plan after, from this design.

## Goals

- A take recorded with expression sounds like it was played, out of a MIDI port.
- The same take plays to a synth in either mode: what's recorded doesn't depend on the
  output format (my call; the user's Hydrasynth is in poly AT mode now and will be
  switched to MPE for testing).
- Pressure (and slide, pitch) can drive any parameter of a sound's instruments and
  plug-ins, live and on playback (2b).

## Non-goals

- Seeing or editing curves (part 3, the Expression editor). Until then, playback is how
  a take is checked.
- DECKARD's sampler and synths responding per voice (part 4). 2b drives a parameter
  for the whole instrument, not per voice.
- Curves on notes made by Note Repeat or the Arp (part 1: they carry none).
- MIDI file export of expression.
- Per-note expression to hosted plug-ins that take MPE (the user's decision: none to
  test with). A take sent to a plug-in plays as plain notes, apart from 2b's
  parameter assignments.

## Anti-goals

- A stuck synth: a bend, slide or pressure left non-neutral after notes end, playback
  stops, or a loop wraps.
- Floods: sending values that didn't change, or more than the curve's detail needs.
- Changing what's recorded or saved; projects save exactly as today.

## 2a: MIDI out

### Per output port, a format (Preferences ▸ MIDI)

Next to the MPE input setting per input port, each **output** port gets an expression
format (my call: the Hydrasynth's MPE mode is a setting of the whole synth, so the port
is where it belongs). Saved in DECKARD's settings like the input setting.

- **Normal** (default): notes go on the sound's channel as today.
  - Pressure: poly aftertouch on the note (0xA0), from its pressure curve.
  - Pitch and slide: not sent (they have no per-note form here).
  - Release velocity: the note-off's velocity.
- **MPE** (lower zone, manager channel 1, member channels 2–16; a per-port pitch-bend
  range, default 48 semitones, the MPE default and what the input setting offers):
  - **DECKARD sets the synth up** (the user's decision): an MPE Configuration Message
    (RPN 6: lower zone, 15 member channels) and the port's pitch-bend range on the
    member channels (RPN 0) go out when the port is set to MPE, when it connects, and
    when its bend range changes; the manager channel's range is set to ±2. A global
    setting, **Send MPE setup to synths** (Preferences ▸ MIDI), on by default (the
    user's decision), turns this off for synths set up by hand.
  - A recorded bend beyond the port's range is clipped to it.
  - Each note gets its own member channel for its life.
  - Pitch: pitch bend on the member channel, from the pitch curve (semitones scaled by
    the port's bend range).
  - Slide: CC74 on the member channel (0–1 → 0–127).
  - Pressure: channel pressure on the member channel.
  - Release velocity: the note-off's velocity.
  - **Before each note-on**, its starting pitch bend, CC74 and pressure are sent on its
    member channel, so a synth reading slide relatively (the Hydrasynth's MPE-Yrel)
    starts from the note's own value and absolute readers (MPE-X pitch bend, MPE-Yabs
    CC74) start in the right place. A note without a curve starts neutral: bend
    centre, CC74 64, pressure 0.
  - The sound's own channel setting is replaced by the member channels for its notes;
    channel-wide lanes (CC, pitch bend, Aftertouch lanes, live controllers) go on the
    manager channel.
  - Member channels are shared by every sound sending to that port: a free channel is
    picked (the one free longest); with all 15 busy, the oldest sounding note on the
    port is ended and its channel reused (my call).
  - When the note ends, its channel is released; the next note to take the channel
    gets its own starting values first, so nothing carries over.

### What's sent and when

- **Recorded notes:** while a note plays from a pattern (or the song), its curves are
  read at the playhead and sent when the value as sent (the 7 or 14 bit number)
  changes, no more often than every 5 ms per channel and dimension (my call; enough for
  smooth bends without flooding a DIN port). Between breakpoints values move straight
  (the curve's own interpolation), so a long slow press plays as a smooth ramp.
- **Live:** what's played reaches the port the same way:
  - The MK3's Pad Touch Poly AT and the Launchpad's poly aftertouch already go out as
    poly aftertouch (2026-10-24); on an MPE port they go as the note's channel pressure
    instead.
  - Per-note MPE from an MPE input (a member channel's bend, CC74, pressure): part 1
    kept these off live so one note's bend wouldn't bend every voice. Now, to a sound
    whose MIDI output is an **MPE port**, they go out per note on that port (my call:
    the reason they were held back doesn't apply there). To a normal port, pressure goes
    as poly aftertouch; pitch and slide stay off, as before.
- **Neutral at the end:** a member channel whose note ends is left as it is (its next
  note resets it). When playback stops, a loop wraps with notes cut, or a sound's output
  port or the port's format changes, every member channel DECKARD used gets its bend
  centred, CC74 64 and pressure 0 (my call; MPE synths otherwise hold the last value
  into the next note on some engines).
- **Several sounds, one channel, normal port:** poly aftertouch is per note, so they
  don't fight.

## 2b: expression as a source: the modulation matrix's base

Revised 2026-10-24: the user chose to build 2b as the first part of the modulation
matrix (`docs/FEATURES.md` #1), so knobs, macros and LFOs (#9) can join as sources later
without a redo.

- **A route:** source → target, with **Low** and **High** (where on the target knob's
  travel, 0–100 %, the source's 0 and 1 put it; Low above High inverts), a **Curve**
  (as Pad Touch: 50 straight) and, for Pitch, a **Range** (± semitones, default 2).
  Routes are a sound's: its notes are the source, its own chain the targets (the
  instrument and every effect, plug-ins included). Several routes can share a source.
- **Sources (2b):** Pressure, Slide and Pitch of the sound's notes, live and recorded,
  whatever plays them (the Sampler, a drum synth, a plug-in instrument, a MIDI port).
- **One value from many notes:**
  - Pressure: the hardest-pressed held note wins (the user's decision).
  - Slide and Pitch: the most recently started held note (my call).
  - A note without that dimension (a plain note, a pad without Pad Touch) doesn't count
    for it (my call: otherwise every hit would throw the parameter to a neutral value).
  - With no note counted the source is 0: Pressure and Slide rest the parameter at Low;
    Pitch rests at centre (halfway between Low and High; −Range is Low, +Range High).
- **The route owns the parameter while it exists** (absolute, as the matrix's low/high
  ranges are): the knob's own value is where it goes back to when the route is removed.
- **Targets:** any continuous parameter of the sound's Sampler, drum synth or internal
  effect (the knob's scale maps travel to stored values), and any parameter of a hosted
  plug-in, instrument or effect (normalized 0–1). A plug-in parameter is reached by its
  slot and id, so the matrix doesn't wait for plug-in parameters to become macro or
  MIDI Learn targets (still PARITY row 3; my call).
- **Following edits:** a route names its chain slot; adding, removing and moving slots
  keep it on its module, removing the module removes its routes, and a route whose slot
  now holds another module does nothing.
- **Assigning (software):** the Assignment area gains a **MOD** tab: under each knob of
  the page, its route (source and range) or "Assign"; a click edits it (source, Low,
  High, Curve, Range, remove), without closing until clicked outside. A **Routes** list
  shows every route of the sound in one place (source, target, range), as the matrix
  should.
- **MK3:** assigning from the controller comes after (my call: it needs DECKARD pages of
  its own on the MK3, like Pad Touch's); routes made in the software play from the MK3.
- **Saving:** routes are DECKARD's: a `deckard:mod_routes` field on the sound (copy,
  paste and undo carry it), in the DECKARD block (version 3); Maschine ignores it.

## Testing with the user

- Hydrasynth in poly AT mode: record pad Poly AT, play the pattern back: the
  Hydrasynth's aftertouch moves as recorded; the MIDI Monitor's Out list shows Poly AT.
- Hydrasynth in MPE mode, port set to MPE: the same take plays as channel pressure on
  member channels; a mod-matrix route from MPE-X, MPE-Yabs or MPE-Yrel follows a take
  recorded from an MPE controller.

## Left to the builder

- How the engine reaches a note's expression while playing (NoteEvent today has none)
  and how the per-port channel allocation is kept.
- The Preferences ▸ MIDI layout for the output format and bend range.
- Where the 5 ms limit lives (per port, per channel).
