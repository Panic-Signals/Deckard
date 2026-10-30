# Maschine's Tom drum synth: parameters, processor and DSP

A clean-room description of Maschine 3's internal **Tom** drum synth
(`NI::MASCHINE::DATA::Tom` → `NI::MASCHINE::RT::DrumSynthWrapper<NI::DSP::TomProcessor>`).
It has four engines, each its own core:

| mode | engine | core |
|---|---|---|
| 0 | **Tronic** | `NI::DSP::DustyTomCore` |
| 1 | **Fractal** | `NI::DSP::TomFBMatrixCore` |
| 2 | **Floor** | `NI::DSP::FloorTomCore` |
| 3 | **High** | `NI::DSP::HighTomCore` |

Written from Ghidra output and arm64 disassembly of the app's arm64 slice
(`analysis/maschine_arm64`; addresses below are its VM addresses) and checked against the
VST3 plug-in by null tests (§7). The DECKARD port is `crates/engine/src/synth/tom.rs`,
with its engines in `crates/engine/src/synth/tom/`.

Sources:

| dir | contents |
|---|---|
| `out/decomp/tom/` | Ghidra C (`INDEX.tsv`): all of `HighTomCore` and `FloorTomCore`, `DustyTomCore::setParameter`, `TomProcessor::setParameter` and the processor |
| `out/decomp/tom2/` | `RealTimeModulation`, `TomFBMatrixCore`, the DATA side (`Tom::Tom`, `getParameterByPosition`, display converters) |
| `out/decomp/tom-env/` | `FreeEnvelope` instantiations used by Tronic and Fractal |

**Confidence.** **[exact]**: read from code or disassembly and confirmed by the bit-exact
null tests. **[inferred]**: a reasonable reading. **[unknown]**: not recovered.

Notation: `v` is a stored parameter value; `sr` the sample rate; all arithmetic is `f32`
unless marked `f64`. `fma(a,b,c)` = a·b + c with one rounding. `E(x)` is Maschine's exp
table (`NI::AB::ExpTable`, docs/fx-eq.md §2.1): `t[0] = 2^-20`, `t[i] = exp2(i/60 − 20)`,
linear interpolation, so `E(x) ≈ 2^((x − 1200)/60)`. `P(st) = E(fma(st, 5, 1381.8816))` is
the frequency of MIDI pitch `st` (Hz), and `D(db) = E(fma(db, 9.965784, 602.0529))` is a
level in dB as a factor ·1000 (Maschine uses it for times in ms).

---

## 1. Parameters (`DATA::Tom`)

### 1.1 Module

The module is named "Tom". Engine enum: **Tronic**, **Fractal**, **Floor**, **High**
(the option vector is built in `__GLOBAL__sub_I_Modules_DrumSynth_Objects_ALL.cpp`).
**KTr. Mode** enum: **Harmonic**, **Dissonant**. Pages come from
`DrumSynth::getPagesNamesStatic()`: **Main** (slots 0–7), **Advanced** (8–15),
**Modulation** (16–23). **[exact]**

### 1.2 Stored parameters (constructor order)

`Tom::Tom()` inserts each parameter with `(label, default, min, max)` **[exact]**:

| obj off | mxprj field | label | default | range | section | display |
|---|---|---|---|---|---|---|
| 0xa8 | `engine` | **Engine** (enum, not automatable) | 0 (Tronic) | 0..3 | "Main" | enum name |
| 0xb0 | `tune` | **Tune** | 0 | 0..1 | | per engine, §1.5 |
| 0xb8 | *(not stored)* | **Tune Stepped** | – | 0..1 | | mirrors Tune |
| 0xc0 | `decay` | **Decay** | 0.5 | 0..1 | | percent |
| 0xc8 | `bend` | **Bend** (bipolar display) | 0.5 | 0..1 | | `(2v − 1)·100` % |
| 0xd0 | `impact` | **Impact** | 0.8 | 0..1 | | percent |
| 0xd8 | `velocity` | **Velocity** (bipolar) | 1.0 | −1..1 | "Scale" | percent |
| 0xe0 | `interval` | **Interval** | 0.5 | 0..1 | "Harmonics" | `"%.2f"` of `14v − 0.5` |
| 0xe8 | `fm_freq` | **FM Freq** | 0.5 | 0..1 | | percent |
| 0xf0 | `fm_amount` | **FM Amount** | 0.1 | 0..1 | | percent |
| 0xf8 | `fm_decay` | **FM Decay** | 0.35 | 0..1 | | percent |
| 0x100 | `keytrack_mode` | **KTr. Mode** (enum, not automatable) | 0 (Harmonic) | 0..1 | "Harmonics" | enum name |
| 0x108 | `mix` | **Mix** | 0.05 | 0..1 | | percent |
| 0x110 | `glide` | **Glide** | 0.1 | 0..1 | | time, §1.5 |
| 0x118 | `transpose` | **Transpose** | 0.5 | 0..1 | | `"%.2f"` of `24v − 12` |
| 0x120 | `freq_a` | **Freq A** | 1.0 | 0..1 | | percent |
| 0x128 | `freq_b` | **Freq B** | 0.75 | 0..1 | | percent |
| 0x130 | `fm` | **FM** | 0.5 | 0..1 | | percent |
| 0x138 | `am` | **AM** | 0.5 | 0..1 | | percent |
| 0x140 | `color` | **Color** | 0.5 | 0..1 | | percent |
| 0x148 | `tension` | **Tension** | 1.0 | 0..1 | | percent |
| 0x150 | `flex` | **Flex** | 0.3 | 0..1 | | percent |
| 0x158 | `air` | **Air** | 0.5 | 0..1 | | percent |
| 0x160 | `skin` | **Skin** | 0.0 | 0..1 | "Mute" | percent |

"percent" is `DefaultConverters::ScaleNormalizedToPercentOneDecimal` (`v·100`, `"%.1f %"`).

Serialization writes the Module base, then 0xa8, 0xb0, 0xc0 … 0x150, **0x160 (Skin)**,
**0x158 (Air)**: Tune Stepped is not stored, and Skin comes before Air. The mxprj field
names in `crates/mxprj/src/types/module.rs` follow this order: `engine, tune, decay, bend,
impact, velocity, interval, fm_freq, fm_amount, fm_decay, keytrack_mode, mix, glide,
transpose, freq_a, freq_b, fm, am, color, tension, flex, skin, air`. **[exact]**

### 1.3 Knob layout (`getParameterByPosition`)

| pos | page | Tronic | Fractal | Floor | High |
|---|---|---|---|---|---|
| 0 | Main | Engine | Engine | Engine | Engine |
| 1 | Main | Tune | Tune | Tune | Tune |
| 2 | Main | Decay | Decay | Decay | Decay |
| 3 | Main | Bend | Bend | Tension | Tension |
| 4 | Main | Impact | Impact | Impact | Impact |
| 5 | Main | – | Color | Flex | – |
| 6 | Main | – | Glide | Skin | – |
| 7 | Main | – | – | Air | – |
| 8 | Advanced | Interval | KTr. Mode | – | – |
| 9 | Advanced | FM Freq | Mix | – | – |
| 10 | Advanced | FM Decay | Transpose | – | – |
| 11 | Advanced | FM Amount | Freq A | – | – |
| 12 | Advanced | – | Freq B | – | – |
| 13 | Advanced | – | FM | – | – |
| 14 | Advanced | – | AM | – | – |
| 16 | Modulation | Velocity | Velocity | Velocity | Velocity |

**[exact]**

### 1.4 The QML editor

`Modules/DrumSynth/Tom/TomEditor.qml` (`DrumSynthModuleScaffold`, secondary name "TOM",
tabs Main and Modulation), `TomMainTab.qml` and `TomModulationTab.qml`. The main tab
(`DrumSynthMainTabBase`) has the TUNE number area with its small NOTE knob, the engine
cycler, the VELOCITY slider (bipolar) and the DECAY visualizer. A HOLD toggle shows only
for Fractal (`fbMatrixParamLock`). Below that, per engine:

- **Tronic**: MAIN: Bend, Impact | HARMONICS: Interval, FM Freq, FM Decay, FM Amount.
- **Fractal**: MAIN: Glide, Color, Bend, Impact, Mode (KTr. Mode cycler) | HARMONICS:
  Mix, Freq A, AM, Freq B, FM, Transpose.
- **Floor**: MAIN: Tension, Impact, Flex | MUTE: Skin (labelled "Skin Tune"), Air.
- **High**: MAIN: Tension, Impact.

### 1.5 Display of Tune and Glide

The Tune text is `"%.2f"` of a note number **[exact]**:

| engine | shown | Tune Stepped steps |
|---|---|---|
| Tronic | `24v + 24 + 12` (36 … 60) | 24 |
| Fractal | `24v + 48 − 12 + 12` (48 … 72) | 24 |
| Floor | `43 + 14v − 7 + 12` (48 … 62) | 14 |
| High | `52 + 14v − 7 + 12` (57 … 71) | 14 |

Glide shows "None" at `v ≤ 0`, otherwise `t = D(40v)·3.5` ms, formatted `"0.00 ms"`,
`"%.2f ms"`, `"%.1f ms"` or `"%.1f s"` by size.

### 1.6 A new Tom

A new Tom has the defaults above: Tronic, Tune 0, Decay 50 %, Bend 0 % (stored 0.5),
Impact 80 %, Velocity 100 %, Interval 6.50, FM Freq 50 %, FM Amount 10 %, FM Decay 35 %,
KTr. Mode Harmonic, Mix 5 %, Glide 0.1, Transpose 0.00, Freq A 100 %, Freq B 75 %, FM 50 %,
AM 50 %, Color 50 %, Tension 100 %, Flex 30 %, Air 50 %, Skin 0 %.

---

## 2. Processor (`DrumSynthWrapper<TomProcessor>`)

### 2.1 Wrapper

- Control rate every 128 frames (`processControlRate` of the current core, before the
  frames). Audio runs in chunks of at most 16 frames. **[exact]**
- One note-on per frame position. A note-on with velocity 0 is a note-off. Note-offs
  matter only to Fractal (its held-note list for glide). **[exact]**
- The velocity reaching `TomProcessor::setNoteOn` is `vel·(1/127)` in `f32`
  (`0x3c010204`). **[exact]**
- **Silence gating**: when the current core reports no output (`hasOutputSignal()` false)
  at the end of a 128-frame control block, the wrapper zeroes the output of that whole
  block. **[exact]** (measured on real Maschine)
- Host MIDI CC 120 (all sound off) does not reach the module in the plug-in (tested). Pitch
  bend and the mod wheel are not mapped and have no effect (tested). **[exact]**

### 2.2 `TomProcessor`

Four cores live side by side; only the current one (`+0x9c`) runs. Shared parameters 0..6
(stored at `+0x80`) are mode, tune, decay, bend, impact, tension and velocity scale.
`setParameter(0, v)` stores the pending mode (`+0xa0`) and chokes the current core if
the mode differs. The pending mode becomes current at the next note-on, followed by
`updateGlobalCoreParameters`, which sends every shared value to the new core. **[exact]**

Each `setParam*` is a tail call to the current core's `setParameter` **[exact]**:

| shared | Tronic | Fractal | Floor | High |
|---|---|---|---|---|
| tune | 0 | 1 | 1 | 0 |
| decay | 1 | 3 | 4 | 1 |
| bend | 4 | 4 | – | – |
| impact | 3 | 6 | 3 | 3 |
| tension | – | – | 2 | 2 |
| velocity | – | – | 7 | 4 |

The velocity scale goes to the core at once only for Floor and High; Tronic and Fractal
read it at note-on.

`setCoreParameter(core, idx, v)` reaches a core whether or not it is current **[exact]**:

- Tronic: Interval → 2, FM Freq → 5, FM Amount → 6, FM Decay → 7.
- Fractal: Glide → 0, KTr. Mode → 2, Mix → 5, Transpose → 7, Freq A → 8, Freq B → 9,
  FM → 10, AM → 11, Color → 12.
- Floor: Flex → 0, Air → 5, Skin → 6.

### 2.3 Velocity (`setNoteOn`)

For Tronic and Fractal, with the Velocity knob `s` (−1..1) and `x = s < 0 ? vel : 1 − vel`:
`v = fma(−x, |s|, 1)`, then `v·(v·v)`. A result outside 0..1 means no note. Floor and High
get the raw `vel` and apply the scale themselves. **[exact]**

---

## 3. Tronic (`DustyTomCore`) — `tom/tronic.rs`, bit-exact

Three triangle oscillators, a noise burst and four envelopes.

- **Pitch** (`calcPitchParams`): `p = tune_st + (note − 60)` with `tune_st = 24v + 24`.
  Oscillator 0 plays `p`, oscillator 1 plays `p + interval` (`interval = 14v − 0.5`), and
  oscillator 2 a partial 17.51 semitones up (`0x418c147b`). Each frequency is
  `(bend_offset + 2^(st/12 + 3.0313597)) · (1 + 0.33·drop)`. `2^x` uses the vector
  polynomial (round to nearest, a cubic for the remainder). **[exact]**
- **Bend** (`fma(v, −20, 10)` semitones) bends the pitch along envelope 3; a "drop"
  decay (time `0.03·period`) raises the pitch by up to 33 % at the start. **[exact]**
- **FM**: an FM triangle at `p + fm_st` (`fm_st = 70v`) with amount `60v / P(0.8·f)`.
  Its decay time is `D(fma(1.2·decay_db − 10, v, 10))`. It offsets the oscillator phases
  before the triangle shaping. **[exact]**
- **Soft clip** per oscillator: a knee/width/sign table per half-wave and a sine-series
  shaper (`x − x³/6 + …` on `π/4`-scaled input), then `x·(0.85|x| + 0.15)`. **[exact]**
- **Envelopes**: four `UTIL::FreeEnvelope`s, stepped in `f64`:
  - 0 and 1 are the amplitudes of oscillators 0 and 1. Their attacks are 20 ms and Impact.
  - 2 is the partial. Its attack is 0.5 ms and its decay `0.02·period`.
  - 3 is the bend shape.

  The decay time is `D(decay_db + 3·per − 2)` with `decay_db = 22v + 34`. **[exact]**
- **Impact** sets five targets: `v/0.3`, `(v − 0.4)/0.6`, `v/0.9`, a click attack, and a
  noise envelope `1 + 14·clamp((v − 0.2)/0.8)`. **[exact]**
- **Noise**: a 32-bit LCG (`·0x41c64e6d + 0x3039`), scaled by `√(sr/44100)`, through two
  3 kHz one-pole low-passes, a 5 kHz high-pass and a 650 Hz SVF. It is added to
  oscillator 1 under a body decay, and as a click (0.5 ms, after a 2.2 ms wait) scaled by
  velocity. **[exact]**
- **Output**: oscillator sum, plus the cross-fade from a voice a retrigger cut. A DC
  blocker at `6·P(p − 40)`, the noise click, and `·0.037973`. Mono (L = R). **[exact]**
- **Smoothing**: 16 lanes ramp to their targets over `round(0.02·sr/128)` control ticks.
  A retrigger jumps lanes 0–4 and 6–14 to their targets. **[exact]**
- **Choke**: a 5 ms exponential fade (factor `exp(−ln2/(0.005·sr))` per frame). The voice ends below
  1e-4. **[exact]**

## 4. Fractal (`TomFBMatrixCore`) — `tom/fractal.rs`, bit-exact

Four parabolic-sine oscillators frequency-modulate each other through a feedback matrix:

- Each lane's pitch is pushed by the sum of the other lanes' filter states, scaled by FM
  (Freq A/Freq B set the lane ratios **[inferred]**).
- Each lane's level is modulated through AM: lane 0 multiplies, the others ring around 1.
- Each lane runs through a 2-pole SVF.
- Lane 0, soft-clipped and crossfaded by Color, is the "fractal" voice.

Mix crossfades that voice with a classic tom: a pitch-swept triangle→sine with a click
envelope (`FreeEnvelope`). A noise burst through a pitch-tracking band-pass carries Impact.
A 10 Hz one-pole high-pass and a fixed gain finish the voice, which is mono. **[exact]**

Parameters (`setParameter`, values clamped to 0..1) **[exact]**:

| idx | knob | effect |
|---|---|---|
| 0 | Glide | `D(40v)·0.001` s one-pole on the pitch (1e-4 at 0) |
| 1 | Tune | `36v − 18` semitones |
| 2 | KTr. Mode | `v > 0`: Dissonant key tracking of the matrix |
| 3 | Decay | `x = 25v + 40`: pitch sweep `D(x − 5)`, oscillators `D(0.74x)`, feedback `D(0.74x − 3)`, body `D(max(x − 28, 28))` |
| 4 | Bend | `48v − 24` semitones of pitch sweep |
| 5 | Mix | `v·(1.8 − 0.8|v|)` |
| 6 | Impact | noise band `E(…(5v − 55)…)`, `2v`, `0.2 + 0.5·clamp(v/0.2)`, `15v` |
| 7 | Transpose | `24v − 12` semitones (matrix) |
| 8 | Freq A | `30v + 40 − 48` |
| 9 | Freq B | `90v + 30 − 48` and that − 5 |
| 10/11/12 | FM / AM / Color | stored as is |

Notes held together glide (`MASCOMMON::SAMPLER::Glide` keeps up to 128 held notes); a
legato note does not retrigger the envelopes. Note-on velocity outside 0..1 silences the
voice. Choke is a 5 ms fade. **[exact]**

## 5. Floor and High — `tom/modal.rs`, bit-exact

### 5.1 Structure

Both are NI's modal-drum kit (the same building blocks as the Cymbal and the Snare's
modal engines) **[exact]** (from the call graph):

- `AttackCore<…MainOscType 1, PhaseCoupling 1, PitchEnv 1…>`: the attack oscillator,
  with a phase oscillator, pitch envelope and filtered noise.
- `BodyCore` / `BodyTableCore` + `UTIL::ModModalSynth`: the bank of modal resonators,
  driven by a `BodyExciter`. For High the body has an air layer, `UTIL::Air`, which plays
  the looped pink-noise recording (`PinkSampler`). Floor's body has no air layer.
- `AttachedBodiesCore` (High `<AM 0, Noise 1, …>`, Floor `<AM 1, Noise 1, …>`): a noise
  layer, `NoiseSampler`, reading noise 12 of the "ModalNoises" recording (§6).
- `RealTimeModulation` (`UTIL::HighTomRealTimeMod`, and Floor's): the macro layer. Each
  knob sets several targets as `lo + k·(hi − lo)` from the `initPreset` tables, summed
  over the knobs that share a target.
- `MemoEnvelope`s, `ParallelNoise`/`NoiseGen`, `FluffyOsc`, and the core's control-rate
  modulation (`computeTuneModulations`, `computeVelocityModulations`).

The output is **stereo** (L ≠ R).

Core parameters **[exact]**:

- Floor `setParameter`: 0 Flex, 1 Tune (an integer pitch pair `60 − 14v`,
  `60 + 14(1 − v)`, re-tuned at the next control tick **[inferred]**), 2 Tension, 3 Impact (applied at
  note-on), 4 Decay, 5 Air, 6 Skin, 7 velocity scale.
- High `setParameter`: 0 Tune, 1 Decay, 2 Tension, 3 Impact, 4 velocity scale.

### 5.2 DECKARD's version

Floor and High are not ported. DECKARD plays its own designs of them (`synth/tom/modal.rs`,
docs/drum-engines-own.md), tuned against Maschine's renders. An earlier translation of
Maschine's machine code was removed as not clean-room.

## 6. NI recorded data Tom needs

Floor and High play two compiled-in `WAVE` resources (`ni::compiled_resources`, decoded
once by `UTIL::loadSampleFromResources(id, block)`). They are mono 16-bit PCM at 44.1 kHz,
decoded as `max(n·f32(1/32767), −1)` (`f32` bits `0x38000100`). **[exact]**

| id | file | frames | file bytes | app arm64 VM | app file offset | VST3 file offsets (x86_64 copy, arm64 copy) | used by |
|---|---|---|---|---|---|---|---|
| 3 | `ModalNoises_23x131072.wav` | 23 × 131072 | 6029438 (RIFF 6029430) | 0x1057517a9 | 0x5e095a0 | 0x6024790, 0x17cdcf26 | Floor and High: **only noise 12** (frames 1572864 … 1703935), read by the `AttachedBodiesCore` `NoiseSampler` |
| 4 | `ModalPinkNoise_1x131072.wav` | 131072 | 262188 | 0x105d11827 | 0x63c9620 | 0x65e4810, 0x1829cfa4 | **High only**: `UTIL::Air` (`PinkSampler`), resampled once when sr ≠ 44.1 kHz |

- WAVE 3 carries a LIST/INFO chunk dated 2011-11-07 (Sound Forge / libsndfile). Both
  files are recorded audio; no Maschine code generates them.
- Fingerprints (FNV-1a-64 of the first 4096 data bytes): pink `0x83b682f39f29faa2`,
  noises `0x9bf3a1b4d7973439`.
- They are the same two resources the Cymbal plays (docs/synth-cymbal.md §6).
- "Only noise 12" was measured: rendering with every noise but one zeroed changes the
  output only for noise 12, for both engines and several notes and velocities. Both
  cores' images also hold their noise pointers at byte offset 0x600000 (= 12·131072·4).
- Tronic and Fractal use no recorded data (their noise is an LCG).

**DECKARD has no copy.** `tom/samples.rs` returns silence in place of both, so in a normal
build the layers that read them are silent:
- Floor's attached-body noise is silent. It sits about 10 dB below the full sound at the
  defaults; the residual is −43 dB against a −33 dB signal.
- High's attached-body noise and air are silent. Together they sit about 8 dB below the
  full sound; the residual is −37 dB against −29 dB.

Everything else in Floor and High is computed exactly.

For null tests only, `cfg(test)` code loads decoded copies from raw little-endian `f32`
files named by `TOM_RES_NOISES` and `TOM_RES_PINK`. The scratch harness writes them from a
local Maschine install; nothing else reads them.

## 7. Null tests (Maschine 3 VST3 vs DECKARD)

Tronic and Fractal were rendered by `synth-null` (the normal build), and Floor and High
by the `#[ignore]` oracle test `synth::tom::oracle::render` (with `TOM_RES_*` set).
`maschine-ref` used `--preroll 0`, 3 s, 48 kHz unless noted, and note 60 velocity 100
held 1 s unless noted. The residual is the RMS of the difference; −240 dB means
bit-identical, both channels.

| engine | case | ref RMS | residual |
|---|---|---|---|
| Tronic | tune 0 / 0.5 / 1 | −35.6 / −37.7 / −39.1 dB | −240 dB |
| Tronic | decay 0 / 0.5 / 1 | −37.1 / −33.5 / −28.7 | −240 |
| Tronic | bend 0 / 0.5 / 1 | −35.7 / −35.6 / −35.6 | −240 |
| Tronic | impact 0 / 0.5 / 1 | −37.6 / −35.7 / −35.6 | −240 |
| Tronic | interval 0 / 0.5 / 1 | −33.4 / −35.6 / −35.4 | −240 |
| Tronic | fm_freq 0 / 0.5 / 1 | −35.6 / −35.6 / −35.4 | −240 |
| Tronic | fm_amount 0 / 0.5 / 1 | −35.4 / −35.9 / −35.2 | −240 |
| Tronic | fm_decay 0 / 0.5 / 1 | −35.5 / −35.6 / −35.6 | −240 |
| Tronic | velocity knob −1 / 0 / 1 | −69.7 / −29.4 / −35.6 | −240 |
| Tronic | note velocity 1 / 64 / 127 | −155.6 / −47.3 / −29.4 | −240 |
| Tronic | notes 24 / 36 / 84 / 108 | −19.9 / −25.7 / −39.1 / −40.1 | −240 |
| Tronic | retrigger (60, 60 after 0.1 s, 72 after 0.13 s) | −32.9 | −240 |
| Tronic | 44.1 kHz / 96 kHz | −35.6 / −35.6 | −240 |
| Tronic | 88.2 kHz, decay 0.8, fm_amount 0.7 | −30.7 | −240 |
| Tronic | all knobs 1, note 48 vel 127 / all knobs 0 | −24.1 / −27.1 | −240 |
| Tronic | pitch bend + mod wheel | −35.6 | −240 |
| Fractal | tune, decay, bend, impact, mix, glide, transpose, freq_a, freq_b, fm, am, color at 0 / 0.5 / 1 (36 cases) | −30.3 … −47.6 | −240 |
| Fractal | KTr. Mode Dissonant | −37.3 | −240 |
| Fractal | velocity knob −1 / 0 | −71.4 / −31.0 | −240 |
| Fractal | note velocity 1 / 64 / 127 | −157.3 / −48.9 / −31.0 | −240 |
| Fractal | notes 24 / 36 / 84 / 108 | −39.1 / −37.6 / −37.3 / −37.4 | −240 |
| Fractal | retrigger, legato glide (overlapping notes) | −34.1 | −240 |
| Fractal | 44.1 kHz; 96 kHz with glide 0.7, Dissonant | −37.3 / −37.8 | −240 |
| Fractal | all knobs 1 / all knobs 0, note 48 vel 127 | −33.1 / −37.1 | −240 |
| Floor | tune, decay, impact, tension, flex, skin, air at 0 / 0.5 / 1 | −34.0 … −38.6 | −240 |
| Floor / High | the rest of the matrix: velocity knob, note velocity, notes, retrigger, 32/44.1/88.2/96 kHz, all knobs 0 / 1 | see §8 | |
| High | tune, decay, impact, tension at 0 / 0.5 / 1 | see §8 | |

## 8. Port status

- **Tronic** and **Fractal**: complete, and bit-exact against Maschine in every test
  above.
- **Floor** and **High**: complete, and bit-exact against Maschine given NI's two noise
  recordings (§6). In a normal build those layers are silent, so Floor and High do not
  null without them. They are otherwise exact.
- Not modelled: host CC 120, which Maschine also ignores here.
- When the current engine is silent, the `voices()==0` skip in the sampler stops calling
  `process`. A later note then starts on a control grid that can differ in phase from
  Maschine's, which runs its 128-frame grid continuously. The null tests start at frame 0
  and are unaffected.
- `resample.rs` lacks the `SampleRateConverter` path for ratios below 1/2, which is
  reached only at sample rates under 22.05 kHz.
