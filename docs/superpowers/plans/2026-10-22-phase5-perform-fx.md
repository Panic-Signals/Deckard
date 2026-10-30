# Phase 5 — Perform FX

> Roadmap: `docs/superpowers/plans/2026-10-22-parity-roadmap.md`. Inline, test-first; the
> software and the MK3.

**Goal:** Maschine's eight Perform FX in a group's chain, played from the Smart Strip
(engage on touch, position) in the software and on the MK3, recordable as modulation.

**Clean room:** the module's parameters (names, ranges, defaults, choices) and its
file layout come from Maschine's data and decompiled data code; the sound is DECKARD's
own DSP, built from the effects DECKARD already has. No NI DSP is read or ported.

## Findings
- **One module, `NI::MASCHINE::DATA::PerformanceFX`**, in a group's chain (slot 0 in the
  projects seen), `Group:getPerformanceFX()`; `SongAccess.loadPerformanceFX(App, hold)`
  puts one in. `Mode` picks the effect: 0 Filter, 1 Flanger, 2 Burst Echo, 3 Reso
  Echo, 4 Ring, 5 Stutter, 6 Tremolo, 7 Scratcher.
- **Its 56 parameters** (constructor `PerformanceFX::PerformanceFX`, by object offset;
  all stored but Touched; defaults as Maschine 3 saves them; every float 0–1):

  | offset | effect | parameter | default / choices |
  |---|---|---|---|
  | a8 | — | Mode | 0 (the list above) |
  | b0 / b8 / c0 | — | Frequency (strip position) / Engage / Touched | 0.5 / off / off |
  | c8–f0 | Filter | Type (Low pass, Band pass, High pass), Min. Freq, Max. Freq, Resonance, Saturation, Out Level | 0, 0, 1, 0.333, 0.3, 0.74 |
  | f8–120 | Flanger | Decay, Stereo, Tone (Flanger, Phaser), Saturation, Wet Level, Polarity (Negative, Positive) | 0.65, 1, 0, 0.7, 0.45, 0 |
  | 128–150 | Ring | Spread, Rev Mix, Rev Time, Mix Mode (Mix, Wet only), Dry/Wet, Glide | 0.3, 0.5, 0.1, 0, 0.35, 0 |
  | 158–190 | Burst Echo | Time, Feedback, Time Mode (Straight, Dotted, Triplet, Free), TS Assign (Feedback, Time), Filter, Feed, Mix Mode (Mix, Replace, Wet only), Wet Level | 0.5, 0.95, 1, 0, 0.35, 0.3, 0, 0.6 |
  | 198–1c8 | Reso Echo | Resonance, Feedback, Time, Saturation, Pump, Wet Level, Mix Mode (Mix, Insert, Wet only) | 0.45, 0.75, 0.25, 0.5, 0.5, 0.35, 0 |
  | 1d0–200 | Stutter | TS Assign (Length, Pitch), Length, Pitch, Direction (Forward, Reverse, Both), Quantize, Gate, Retrigger | 0, 0.4, 0.5, 2, on, 1, on |
  | 208–220 | Scratcher | Delay Time, Feedback, Freq Shift, Wet Level | 0.55, 0.7, 0.5, 0.5 |
  | 228–260 | Tremolo | Trem/Vibr, Vibr Mode (Varispeed, Freq Shift), Attk/Rel, Stereo, Depth min, Depth max, Rate min, Rate max | 0.3, 0, 0.5, 0.4, 0.1, 0.4, 0.6, 0.4 |

- **Pages** (manual pp. 381–390): each effect has Main (Mode; the strip's Engage and
  Frequency/Position; its own section) and Setup. Displays: Ring Glide Off below 1 %,
  else 10^((40x+30)/20) ms; Burst Echo Wet Level ×1.5; Scratcher Freq Shift
  (2x−1)·(x<½ ? 100 : 50) Hz.
- **The strip:** on the MK3, PERFORM sets the touch strip to TS_MODE_PERFORM; touching it
  engages, its position is Frequency. In the software, Engage and Frequency are the
  module's first knobs. Both are parameters, so Auto-write records them as modulation
  (Maschine's templates hold modulation on `touchstrip_on_off`).

## Tasks
1. **Names** (`mxprj` decoder): the fields get their parameters' names (`mode`,
   `touchstrip_pos`, `touchstrip_on_off`, `filter_type` …); a new Perform FX built from
   scratch with the defaults (`doc`: `add_perform_fx(g)`, slot 0). Tests: the user's and
   the library's projects read with the names; a new one round-trips.
2. **The DSP** (`engine/src/fx/perform/`): `PerformFx` (an `Effect`) runs the mode's
   effect while engaged, passes the input through while not (short crossfades on
   engage, release and mode change); one file per effect. Tests per effect: bypass when
   disengaged; Filter cuts highs; Flanger/Reso Echo/Burst Echo delay; Ring adds
   sidebands; Stutter repeats; Tremolo modulates level; Scratcher slows to a stop.
3. **Software:** the group's chain shows Perform FX with its Main / Setup pages per
   mode; Add Perform FX in the group chain's menu; Engage and Frequency record as
   modulation (existing Auto-write). Harness tests.
4. **MK3:** `Group:getPerformanceFX`, `loadPerformanceFX`, `canAddPerformanceFX`; the
   strip in PERFORM mode engages and moves it; its LEDs; the Control pages. Script tests.
5. Docs: PARITY row 2, this plan's results; the full check.

## Results
- Tasks 1–5 done. The decoder already read all 55 stored fields; they now have the
  parameters' names. `Document::add_perform_fx`; `fx::perform::PerformFx` (one file per
  effect, `kit.rs` for the shared parts); the pages; `App::perform_strip`; the host's
  `PerformanceFX` object, `canAddPerformanceFX`, `loadPerformanceFX` and the strip in
  PERFORM mode (`ControlRequest::PerformStrip`, `ShowPerformFx`).

## Rulings
- The DSP is DECKARD's own design; the ranges it maps 0–1 to (cutoff 20 Hz–20 kHz, echo
  times, Ring's 30 Hz–3 kHz, Tremolo's 0.1–20 Hz…) are DECKARD's choices beyond the few
  display conversions read from Maschine (Ring Glide, Burst Echo's wet level ×1.5,
  Scratcher's Freq Shift). If wrong: the effects sound and respond differently from
  Maschine's, though every parameter is there and saved the same.
- Engage is recorded as single points (on at the touch, off at the release) and the
  position as a knob's movement. If wrong: Maschine may thin or shape these otherwise.
- Engaging in the software is a toggle (ENGAGE); Maschine's software may make it
  momentary. If wrong: one click engages until the next instead of while held.
