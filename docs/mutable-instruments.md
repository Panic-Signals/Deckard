# Mutable Instruments: Plaits

DECKARD has Mutable Instruments' Plaits as its own plug-in, built from
Émilie Gillet's open source code (MIT; `crates/mi/vendor/LICENSE`, also in the About
box), so it sounds as the module does:

- **Plaits** (instrument): the macro-oscillator's 24 engines.

## How

- `crates/mi` (`deckard-mi`) builds the module's DSP as its desktop test build does
  (`TEST` defined: no hardware), with a small C bridge (`src/bridge.cc`), and wraps it
  (`Plaits`).
- The vendored code is Plaits' `dsp/` and resources, and stmlib's `dsp/` and `utils/`
  (the commits are in the licence file).
- `crates/plugins/src/plaits.rs` is the native plug-in around it.

## Plaits

- Pages: **Plaits** (Engine, Harmonics, Timbre, Morph; Decay and Colour of the low-pass
  gate; Level; Out/Aux, between the module's two outputs) and **Voice**.
- Harmonics, Timbre and Morph take the names the manual gives them for the engine
  chosen (VA: Detune, Square, Saw; Bass Drum: Attack, Brightness, Decay, …), on screen
  and on the MK3. Decay and Colour read "unused" for the engines with their own
  envelope (six-op FM, String, Modal and the drums), which bypass the low-pass gate.
- A voice per note (Voices: 1, 2, 4 or 8), each a Plaits with its TRIG held while the
  key is (a gate: the six-op engines play their envelopes for as long as it's high).
- Envelope: **LPG** (the module's gate, decaying with Decay) or **Gate** (LEVEL patched
  with an envelope: the note holds, then releases with Release and the gate's tail).
- Plaits runs at 48 kHz, tuned for the module's 47 872 Hz clock (DECKARD corrects the
  pitch); at other rates its output is resampled.

## Not yet

- Plaits' FM, Timbre and Morph modulation inputs aren't used.

Beads (the granular processor) was built here once and removed (2026-10-22): not wanted.
