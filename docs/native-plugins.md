# DECKARD's own instruments and effects

DECKARD can run synth engines (and effects) of its own. Each one is saved in the
project as a plug-in, so projects stay valid Maschine projects.

## How they are stored

A native plug-in is a `PluginHost` module, exactly as for a VST3:

- `plugin_id` = `DECKARD:VST3:INSTR:<class id, 32 hex digits>` (`:FX:` for an effect)
- `plugin_name` = the engine's name
- `chunk` = `[u32 1][u32 1][u32 n]` + a `.vstpreset` whose component state is the
  engine's state (`crates/plugins/src/native.rs` gives the format: magic `DKRD`, a
  version, then (parameter id, value) pairs, then the engine's own bytes).

**Checked against Maschine 3:** `maschine-ref plant` writes a project with an unknown
DECKARD plug-in, and `maschine-ref roundtrip` loads it into Maschine's own engine and
saves it again. Maschine reports the plug-in as missing, but it keeps the id, the name
and the state chunk byte for byte. A Replicant project made in DECKARD
(`native_synth_from_the_add_menu`) passes the same round trip. A plug-in Maschine never
ran comes back with an empty 12-byte chunk, which DECKARD loads as the defaults.

The class id is a VST3 class id. The same engine could later be built as a real VST3
with that id, and Maschine would then load it too.

## Writing an engine

1. Make a file next to `crates/plugins/src/replicant.rs` (the worked example).
2. Implement `native::Engine`. It handles sound only: `set_param`, `note_on`/`note_off`,
   `process` (in place: input in the buffers for an effect, silence for an
   instrument), `reset`, and optionally `text` (how a value reads), `control_change`,
   `pitch_bend` (−1..1), and `save`/`load` for data that isn't a parameter.
   The framework keeps parameter values, saves and loads them, does undo through the
   project, and holds notes for the sustain pedal (CC 64).
3. Describe its `Param`s and `Page`s. Values are normalized 0..1. `steps > 0` makes a
   choice, shown as a selector with arrows. A page has up to eight knobs, and a knob can
   start a section heading.
4. Add a `static CLASS: Class` with a new 16-byte `cid` and list it in
   `native::CLASSES`. It then shows up under "+" › Internal (instruments after Audio,
   effects after Maschine's effects).

**Rules for compatibility:** never change a class id. Never reuse a parameter id for
another meaning. New parameters load at their default in old projects, and parameters
you remove are ignored. Bump the state version only when the engine's own bytes
change format.

## Replicant

Replicant is a subtractive synth: two PolyBLEP oscillators (saw, pulse, triangle,
sine), a sub oscillator, noise, and a resonant state-variable filter (LP, BP or HP)
with its own envelope. It also has an amp envelope, an LFO (to pitch and cutoff), and
the mod wheel for vibrato. It plays 16 voices, or Mono (legato with glide), with
stereo spread and analog drift. Oscillator 1 can be a supersaw (after Roland's
JP-8000): Super Voices (Off, 3, 5, 7) stacks detuned copies of its wave, starting at
random phases on each note; Super Detune spreads them (the JP-8000's curve, as measured
by Adam Szabo: gentle for most of the knob, about ±1.8 semitones at full) and Super Mix
sets the side copies against the centre one. Off by default, so older sounds are
unchanged. Its pages are Oscillators, Filter, Amp, Modulation and Super.
