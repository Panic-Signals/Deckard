# Maschine's Percussion: parameters and DSP

A clean-room description of Maschine 3's internal **Percussion** drum synth
(`NI::MASCHINE::DATA::Perc` → `NI::MASCHINE::RT::DrumSynthWrapper<NI::DSP::PercProcessor>`).
`PercProcessor` owns three DSP cores and runs one at a time:

| engine (`mode`) | core | DECKARD |
|---|---|---|
| 0 Fractal | `NI::DSP::PercFBMatrixCore` (at `PercProcessor` +0xb80) | `synth/perc/fractal.rs`, bit-exact |
| 1 Kettle | `NI::DSP::KettlePercCore` (+0x1280) | `synth/perc/kettle.rs`: DECKARD's own design (docs/drum-engines-own.md) |
| 2 Shaker | `NI::DSP::ShakerCore` + `ShakerPerformer` (+0x180) | `synth/perc/shaker.rs`, bit-exact (all three envelope modes) |

The engine list is confirmed by the `mode` enum options in `Perc::Perc()` and by
`PercMainTab.qml` / `PercModulationTab.qml`, which know exactly these three labels.

Written from Ghidra output in `out/decomp/perc/`, `out/decomp/perc-data/` and
`out/decomp/perc-fe/` and from arm64 disassembly of `analysis/maschine_arm64`
(addresses below are VM addresses there). The QML editor is
`Modules/DrumSynth/Perc/PercEditor.qml` (a `DrumSynthModuleScaffold`, secondary name
"PERCUSSION", tabs **Main** / **Modulation**), `PercMainTab.qml` (the shared
`DrumSynthMainTabBase`: mode picture, Tune with the Fractal-only HOLD switch, Mode,
Velocity, Decay visualiser) and `PercModulationTab.qml` (the per-engine knobs).

**Confidence.** **[exact]**: read from code and confirmed by a bit-exact null.
**[inferred]**: reconstructed, not null-tested.

---

## 1. Parameters

### 1.1 Module

Pages (`DrumSynth::getPagesNamesStatic` @0x1019c4a74, shared by all drum synths):
**Main** (positions 0–7), **Advanced** (8–15), **Modulation** (16–23) **[exact]**.

### 1.2 Parameters in constructor order (`Perc::Perc()`)

`TransactionObjectContainer::insert(name, default, min, max)` **[exact]**. "RT" is where
the value goes: `shared n` = `PercProcessor::setParameter(n)` (applies to the engine
playing, and is re-sent to it at every note), `core c,i` =
`PercProcessor::setCoreParameter(c, i)` (always, playing or not; the Shaker's copy is
first shifted or velocity-scaled, §2.3).

| obj off | mxprj field | label | section | default | range | RT | display |
|---|---|---|---|---|---|---|---|
| 0xa8 | `mode` | **Engine** | "Main" | 0 | Fractal, Kettle, Shaker | shared 0 | enum; not automatable |
| 0xb0 | `tune` | **Tune** | | 0.5 | 0..1 | shared 1 | `getTuneValueString`: Fractal `60 + 48·v` as `"%.2f"` (a note number), Kettle `33 + 24·v`, Shaker percent |
| 0xb8 | — | Tune Stepped | | 0.5 | 0..1 | (sets Tune) | not serialized; the NOTE wheel |
| 0xc0 | `decay` | **Decay** | | 0.5 | 0..1 | shared 2 | percent, 1 decimal |
| 0xc8 | `impact` | **Impact** | | 0.6 | 0..1 | shared 3 | percent |
| 0xd0 | `velocity` | **Velocity** | "Scale" | 1.0 | −1..1, bipolar | shared 4 | percent |
| 0xd8 | `shaker_grain` | **Grain** | | 0.5 | 0..1 | core 2,1 | percent |
| 0xe0 | `shaker_filter` | **Filter** | | 0.7 | 0..1 | core 2,2 | percent |
| 0xe8 | `shaker_env_mode` | **Mode** | "Envelope" | 1 | Machine, Realistic, Performer | core 2,3 | enum |
| 0xf0 | `shaker_attack` | **Attack** | | 0.4 | 0..1 | core 2,4 | percent |
| 0xf8 | `shaker_hold` | **Hold** | | 0 | 0..1 | core 2,5 | percent |
| 0x100 | `shaker_release` | **Release** | | 0.35 | 0..1 | core 2,6 | percent |
| 0x108 | `shaker_accent` | **Accent** | | 0.7 | 0..1 | core 2,7 | percent |
| 0x110 | `shaker_fill` | **Fill** | | 1 | Triple, None, Double | core 2,9 | enum |
| 0x118 | `shaker_rate` | **Rate** | | 4 | 1/4, 1/4T, 1/8, 1/8T, 1/16, 1/16T, 1/32 | core 2,10 | enum |
| 0x120 | `shaker_swing` | **Swing** | | 0 | 0..1 | core 2,11 | percent |
| 0x128 | `shaker_twist` | **Twist** | | 0 | 0..1 | core 2,12 | percent |
| 0x130 | `shaker_length` | **Length** | | 3 | "1" … "8" | core 2,13 | enum |
| 0x138 | `shaker_offset` | **Offset** | | 0 | −1..1, bipolar, 8 steps | core 2,14 | `"%.2f"` of 4·v |
| 0x140 | `shaker_sync` | **Sync** | "Envelope" | 1 | Retrig, Lock | core 2,16 | enum |
| 0x148 | `fractal_freq_a` | **Freq A** | | 0.5 | 0..1 | core 0,5 | percent |
| 0x150 | `fractal_freq_b` | **Freq B** | | 0.4997 | 0..1 | core 0,6 | percent |
| 0x158 | `fractal_freq_c` | **Freq C** | | 0.2906 | 0..1 | core 0,7 | percent |
| 0x160 | `fractal_fm` | **FM** | | 0.3436 | 0..1 | core 0,8 | percent |
| 0x168 | `fractal_am` | **AM** | | 0.1857 | 0..1 | core 0,9 | percent |
| 0x170 | `fractal_tune_hold` | **Tune Hold** | | false | bool | core 0,0 | on/off (QML "HOLD") |
| 0x178 | `fractal_keytrack` | **KTr. Mode** | "Harmonics" | 0 | Harmonic, Dissonant | core 0,1 | enum |
| 0x180 | `fractal_glide` | **Glide** | | 0 | 0..1 | core 0,4 | `getFBMatrixGlideValueString` (`calculatePortaDecayForGUI`): "None", `"%.2f ms"`, `"%.1f ms"`, `"%.1f s"` |
| 0x188 | `kettle_puff` | **Puff** | | 0.25 | 0..1 | core 1,3 | percent |
| 0x190 | `kettle_damp` | **Damp** | | 0 | 0..1 | core 1,4 | percent |

A new Percussion module therefore starts as a Fractal with exactly these defaults
(`synth::perc::FIELDS` lists them in this order).

### 1.3 Knob layout (`Perc::getParameterByPosition` @0x1019cea68) **[exact]**

| page | slot | Fractal | Kettle | Shaker Machine | Shaker Realistic | Shaker Performer |
|---|---|---|---|---|---|---|
| Main | 0 | Engine | Engine | Engine | Engine | Engine |
| | 1 | Tune | Tune | Tune | Tune | Tune |
| | 2 | Decay | Decay | Filter | Filter | Filter |
| | 3 | Tune Hold | Puff | Grain | Grain | Grain |
| | 4 | Glide | Damp | Mode | Mode | Mode |
| | 5 | Impact | Impact | Attack | Attack | Accent |
| | 6 | — | — | Hold | — | Fill |
| | 7 | — | — | Release | Release | Release |
| Advanced | 8 | KTr. Mode | — | — | — | Sync |
| | 9 | Freq A | — | — | — | Rate |
| | 10 | Freq B | — | — | — | Length |
| | 11 | Freq C | — | — | — | Offset |
| | 12 | FM | — | — | — | Swing |
| | 13 | AM | — | — | — | Twist |
| | 14–15 | — | — | — | — | — |
| Modulation | 16 | Velocity | Velocity | Velocity | Velocity | Velocity |
| | 17–23 | — | — | — | — | — |

The software editor shows a subset: the Main tab has Tune (+ HOLD for Fractal), Mode,
Velocity and Decay for every engine; the Modulation tab has, per engine: Fractal —
"MAIN" Glide, Impact; "HARMONICS" Freq A, Freq B, Freq C, AM, FM, KTr. Mode cycler;
Kettle — "MAIN" Puff, Damp, Impact; Shaker — "MAIN" Grain, "ENVELOPE" with the Mode
cycler and Attack, Hold, Release (Machine), Attack, Release (Realistic) or Accent,
Release, Sync, Rate, Offset, Fill, Length, Swing, Twist (Performer). The Shaker's
Filter is only on the hardware page.

---

## 2. The processor and wrapper **[exact]**

### 2.1 Blocks

`DrumSynthWrapper<PercProcessor>::process` (@0x1019e1da0) runs once per 128-frame
block Maschine's engine hands it (its `AlignedAudioBlock`). It first calls
`PercProcessor::onBPMChanged(tempo)` and `onPositionChanged(song position in quarters,
playing)` — both reach only the Shaker, and only while the Shaker is the engine playing
(`+0x10c == 2`) — then splits the block at its MIDI events. `processAudio`
(@0x1019e262c) cuts each piece into chunks of at most 16 frames, calls
`processControlRate` every 128 frames (counter at +0x43d00, kept across calls; the first
call runs it), and after every chunk checks the core's `hasOutputSignal`: when it is
false the block's *silent* flag is set. A chunk that starts with the flag set first
clears the whole block buffer; a block that ends with the flag set is silence as a
whole. So a note's tail is cut at the start of the 128-frame block in which the core
went quiet.

### 2.2 Notes

One note-on per time stamp. Velocity `v = vel · f32(1/127)`. `PercProcessor::setNoteOn`
switches to the engine selected (`+0x110` → `+0x10c`; an engine change only chokes the
old one), computes the velocity scaling `a = (1 − x·|s|)³` with `s` = Velocity and
`x = v` (`s < 0`) or `1 − v`, re-sends the shared parameters
(`updateGlobalCoreParameters`), then starts the core: Fractal and Shaker get `a`, the
Kettle gets `v` and the Velocity value as its parameter 5. Note-off reaches only the
Shaker (the Performer counts held notes). CC 120 would choke (`onChoke`), but Maschine
delivers no controller or pitch bend to the module at all (null-tested, §8), so DECKARD
ignores them.

### 2.3 The Shaker's parameter copy

`setCoreParameter(2, i, v)`: Grain, Filter and Release follow the note's velocity
(`clamp01(v + (a − 1)·(−s))` for Grain, `+ (a − 1)·s·0.2362` for Filter, `·0.4173` for
Release, which also sets the Performer's Spread) unless the envelope mode is Performer;
Fill arrives as `v − 1`, Length as `v + 1`, Offset as `round(4v)` (parameter 14) and its
fraction (15); setting Env Mode chokes and re-applies the velocity modulation.

---

## 3. Shared building blocks (`synth/perc/dsp.rs`) **[exact]**

- `NI::AB::ExpTable`: 2401 entries `f32(2^(k/60 − 20))` with slopes, read with
  `fcvtzs`; dB (`x·9.965784 + 1200`), dB60 (`+602.0529`) and Hz (`x·5 + 1381.8816`)
  lookups.
- NEON `frecpe`/`frecps` emulated in software (checked against the hardware).
- Polynomial `2^x` (round, then `0.056/0.242/0.693`), `log2`, `tan(w) ≈ w + w³/3 +
  2w⁵/15`.
- `AB::SmoothingToolbox<N>`: at each control tick a started lane ramps linearly to its
  target over `round(sr·0.02/128)` ticks (`0.05` for the Performer's values).
- `FreeEnvelope<…>` in the three variants the cores use (Fractal lin/log, Shaker
  Machine lin/shape, Shaker Realistic inverted/log).
- The TPT state-variable filter.

---

## 4. Fractal (`PercFBMatrixCore`) **[exact]**

Four parabolic sine oscillators in a feedback matrix: each lane's pitch is bent by the
other lanes' SVF-filtered outputs (FM) and its level modulated by them (AM). Lane A is
the output, mixed with a soft-clipped copy (Impact) and a band-passed noise burst,
shaped by two `FreeEnvelope`s, a pitch envelope, a declick crossfade and a 10 Hz
high-pass. Tune = `48·v − 24` semitones around the note (key 60 = root), Decay =
`30·v + 15`, Freq A/B/C = `96·v + 24` (note numbers), Glide a smoothed portamento
(`1 / Hz(72 − 82·g')`, `g' = (1.8 − 0.8|g|)·g`). Tune Hold freezes the lanes' pitches
at the note-on value; KTr. Mode Harmonic adds the base pitch to Freq B/C, Dissonant
scales it by 0.73/1.12. Maschine picks one of several template instances
(`processAudioImpl<Glide, Hold, Keytrack, ramp, first>`); the port branches at run
time on the same conditions.

## 5. Kettle (`KettlePercCore`) **[exact, with the recordings]**

An `AttackCore<FluffyOsc off, white noise, MainOsc 1, …>` strike, a
`BodyTableCore<Exciter 1, Air off, FeedbackDamping off>` modal body (a
`ModModalSynth` bank, with an `UTIL::Air` layer that plays the pink-noise recording),
`AttachedBodiesCore<AM off, Noise 2, Crispel off, PitchEnv on>` (resonators fed by the
"ModalNoises" recording) and `KettlePercRealTimeMod` (preset morph tables scanned by
Tune, Decay, Impact, Puff, Damp). Tune spans notes `60 − 24·t … 60 + 24·(1 − t)`; the
note-on maps the key into that range.

DECKARD does not port this core. `synth/perc/kettle.rs` is DECKARD's own design, tuned
against Maschine's renders (docs/drum-engines-own.md). An earlier translation of Maschine's
machine code was removed as not clean-room.

## 6. Shaker (`ShakerCore` + `ShakerPerformer`) **[exact]**

Noise (LCG `x·0x41c64e6d + 0x3039`, Maschine's seeds) through a band-pass, mixed with
four random grains (phasors restarting at a random rate around 90/80/75/70 Hz with a
random decaying level), three SVFs in series (cutoffs from Tune and Filter), an envelope
follower that normalises the level, a 12 kHz high-pass, and one of three envelopes:

- **Machine**: `FreeEnvelope` linear attack / hold / shaped release, times a decaying
  level block.
- **Realistic**: inverted-shape attack, log decay (curve `f32(−0.6)`), linear release.
- **Performer**: a tempo-synced shake pattern (`ShakerPerformer::processSampleImpl`):
  Rate (steps per bar), Length (bars), Fill (Triple ×1.5 / Double ×2), Swing and Twist
  (alternate step lengths), Accent and Spread (velocity-dependent dB gain), Offset
  (pattern start), Sync Lock (the pattern follows the song position) or Retrig. It plays
  while a note is held.

Maschine runs one of `processAudioImpl<mode, ramp, first>`: the first block after a
reset recomputes every coefficient from the smoothed values; a ramp block recomputes
only those whose smoother lane is moving (its increment non-zero); other blocks none.
The port does the same.

---

## 7. NI data

The Kettle reads two recordings compiled into Maschine as `WAVE` resources: 3,
"ModalNoises_23x131072.wav" (attached bodies) and 4, "ModalPinkNoise_1x131072.wav"
(Air), mono 16-bit at 44.1 kHz. They are recorded audio, so DECKARD has no copy: in
DECKARD those two layers are silent (`kettle/noises.rs`), and the Kettle differs from
Maschine by the missing noise (about −65 dB residual against a −31 dB note). Oracle
tests can supply decoded copies through `PERC_RES_NOISES` / `PERC_RES_PINK`
(`cfg(test)` only). Fractal and Shaker use no NI data.

---

## 8. Null tests

`maschine-ref make "Dev Traps.mxprj" … 8 0 0 <fields>` then `render … --preroll 0`,
against `synth-null … --tempo 120` (maschine-ref's host tempo). 48 kHz unless noted,
note 60 velocity 100 unless noted, 2 s (note-off at 1.5 s). Kettle rows use a build
that reads the recordings (§7).

NULL_TABLE

Sources: Dev Traps 8/0 and Lightspeed 0/2 load and play; Exosphere 0/1 renders silence
in Maschine for every engine, so it was not used.
