# Maschine's Snare drum synth: parameters, processor and DSP

A clean-room description of Maschine 3's internal **Snare** instrument (a drum synth that
sits in a sound's first slot): `NI::MASCHINE::DATA::Snare` (parameters, persistence, display)
→ `NI::MASCHINE::RT::DrumSynthWrapper<NI::DSP::SnareProcessor>` (real-time wrapper) →
`NI::DSP::SnareProcessor` → one of eight DSP cores. Written from Ghidra output and objdump
listings of `analysis/maschine_arm64`; addresses are arm64 VM addresses. The DECKARD port is
`crates/engine/src/synth/snare.rs` (processor and wrapper) and `crates/engine/src/synth/snare/*.rs`
(one file per core).

Decompiled sources (`INDEX.tsv` in each maps file → address → symbol):

| dir | contents |
|---|---|
| `out/decomp/snare/` | `DATA::Snare` (ctor 0088/0089, serialize 0086/0087, `getParameterByPosition` 0111, `getTuneValueString` 0110, `getBendValueString` 0109, `connectParameters` 0079, …), `SnareProcessor` (0002–0030, 0093–0099), `ClassicSnareCore` (0031–0062), `DrumSynthWrapper<SnareProcessor>` (0070–0077), `DSPCoreWrapper` (0063–0069), `__GLOBAL__sub_I_Modules_DrumSynth_*` (0114/0115: page names, enum option strings) |
| `out/decomp/snare-cores/` | the eight cores and every NI::DSP callee three levels deep (BodyCore, BodyTableCore, AttackCore, AttachedBodiesCore, MainOscAttack, ParallelNoise, FreeEnvelope, PhaseOsc, *SDRealTimeMod, Air, BiQuad, …) |

**Confidence tags.** **[exact]**: read from code or assembly. **[inferred]**: reconstructed
from partial evidence or measurement. **[unknown]**: not recovered.

Notation: `v` is a parameter's normalized value (0..1); arithmetic is `f32` unless stated;
`fma(a,b,c)` is a fused multiply-add (AArch64 `fmadd`), required where marked for bit-exact
output. `midi/127` is computed as `midi · 0.007874016`.

---

## 1. Parameters (`DATA::Snare`)

Module name "Snare" (`s_aModuleInfo` = 3). Pages `DrumSynth::getPagesNamesStatic()` =
**Main**, **Advanced**, **Modulation** (wide strings 0x104d1c390/3a4/3c8) **[exact]**.

### 1.1 Stored parameters (constructor order, `Snare::Snare()` 0x1019cf53c)

Each is made by `TransactionObjectContainer::insert(label, default, min, max)`; enums by
`insert(label, default, options)`. The serializer (`serialize<iarchive>` 0x1019cf18c) loads
them in this order but **skips Tune Stepped** (+0xb8), which it sets from Tune after loading.
All floats except Tune/Tune Stepped/Bend/Velocity display with
`DefaultConverters::ScaleNormalizedToPercentOneDecimal` (`v·100`, one decimal, "%") **[exact]**.

| member | mxprj field | label | section | default | range | → processor | display |
|---|---|---|---|---|---|---|---|
| +0xa8 | `mode` | **Engine** (enum) | "Main" | 0 (Volt) | Volt, Bit, Pow, Sharp, Airy, Vintage, Chrome, Iron, Clap, Breaker | `setParameter(0)` = `setParamMode` (via `setParamModeAndTuneSteps`) | option name |
| +0xb0 | `tune` | **Tune** | – | 0.5 | 0..1 | `setParameter(1)` | `getTuneValueString` (§1.3) |
| +0xb8 | (not stored) | **Tune Stepped** | – | 0.5 | 0..1 | (mirrors Tune) | as Tune; dragger steps (§1.3) |
| +0xc0 | `decay` | **Decay** | – | 0.5 | 0..1 | `setParameter(2)` | % |
| +0xc8 | `skin_tune` | **Skin Tune** | – | 0.5 | 0..1 | `setParameter(4)` | % |
| +0xd0 | `impact` | **Impact** | – | 0.75 | 0..1 | `setParameter(5)` | % |
| +0xd8 | `body` | **Body** | – | 0.75 | 0..1 | `setParameter(6)` | % |
| +0xe0 | `spectra_mode` | **Mode** (enum) | "Spectra" (span 4, not automatable) | 0 | A, B | `setParameter(7)` (NoiseMode) | option |
| +0xe8 | `spectra_tune` | **Tune** | – | 0.5 | 0..1 | `setParameter(8)` (NoiseTune) | % |
| +0xf0 | `spectra_amount` | **Amount** | – | 0.75 | 0..1 | `setParameter(9)` (NoiseAmount) | % |
| +0xf8 | `spectra_decay` | **Decay** | – | 0.4 | 0..1 | `setParameter(10)` (NoiseDecay) | % |
| +0x100 | `velocity` | **Velocity** | "Scale" | 1.0 | −1..1, bipolar | `setParameter(11)` | % (`v·100`) |
| +0x108 | `osc_mode` | **Osc Mode** (enum) | (span 2, not automatable) | 0 | Tonal, Punchy | `setParameter(3)` (AnalogSnareMode) | option |
| +0x110 | `noise_color` | **Color** | "Noise" | 0.5 | 0..1 | `setCoreParameter(0, 6)` | % |
| +0x118 | `noise_amount` | **Amount** | – | 0.25 | 0..1 | `setCoreParameter(0, 7)` | % |
| +0x120 | `punch` | **Punch** | – | 0.25 | 0..1 | `setCoreParameter(0, 4)` | % |
| +0x128 | `osc_mix` | **Osc Mix** | – | 0.5 | 0..1 | `setCoreParameter(0, 3)` | % |
| +0x130 | `grit` | **Grit** | – | 0.3 | 0..1 | `setCoreParameter(0, 5)` | % |
| +0x138 | `gate` | **Gate** (bool) | – | false | off/on | `setCoreParameter(0, 8)` (1.0/0.0) | on/off |
| +0x140 | `bend` | **Bend** | – | 0.5 | 0..1, bipolar | `setCoreParameter(1, 4)` | `getBendValueString`: `(2v−1)·100` % |
| +0x148 | `room` | **Room** | – | 0.5 | 0..1 | `setCoreParameter(1, 3)` | % |
| +0x150 | `focus` | **Focus** (QML: "SHARP") | – | 0.5 | 0..1 | `setCoreParameter(1, 6)` | % |
| +0x158 | `noise` | **Noise** | – | 0.4 | 0..1 | `setCoreParameter(6, 5)` | % |

- Enum option strings are built in `__GLOBAL__sub_I_Modules_DrumSynth_Objects_ALL.cpp`
  (vectors at 0x10e305be8 engine, 0x10e305c00 spectra mode, 0x10e305c18 osc mode) **[exact]**.
- Engine and Osc Mode are not automatable and force a display update when changed
  (`setParameterForcesDisplayUpdate`) **[exact]**.
- The engine change callback (`onModeParameterChanged`) and `setParamModeAndTuneSteps` set Tune
  Stepped's dragger steps (§1.3) and Tune's wheel handler (`processWheelStepsAndFineRes`,
  fine step 0.01) **[exact]**.
- Old files (class version 0) stored the engine in another order; the loader remaps
  `[0..8] → [4, 1, 6, 8, 7, 2, 3, 5, 0]` (old 0 = new Airy, …, old 8 = new Volt) **[exact]**.
  Current files are version 1 (`crates/mxprj/src/types/classes.rs`).
- The processor receives `NumericParameter::getNormalized(value)` for the 0..1 floats
  (`setAndCallFuncAutomate` binds it), the enum index as a float, 1.0/0.0 for Gate, and
  **the raw −1..1 value for Velocity** (bound without `getNormalized`, like the enums;
  confirmed by null tests) **[exact]**.

### 1.2 Hardware/knob pages (`getParameterByPosition` 0x1019d1db0)

Positions 0–7 are page **Main**, 8–15 **Advanced**, 16–23 **Modulation**. Which parameter sits
in a slot depends on the engine **[exact]**:

| pos | Volt | Bit | Pow | Clap | Sharp | Airy / Vintage / Chrome / Breaker | Iron |
|---|---|---|---|---|---|---|---|
| 0 | Engine | Engine | Engine | Engine | Engine | Engine | Engine |
| 1 | Tune | Tune | Tune | Tune | Tune | Tune | Tune |
| 2 | Decay | Decay | Decay | Decay | Decay | Decay | Decay |
| 3 | Gate | Gate | Bend | Room | Skin Tune | Skin Tune | Skin Tune |
| 4 | Osc Mode | Grit | Focus | Focus | Impact | Impact | Impact |
| 5 | Punch (Osc Mode Punchy) / Osc Mix (Tonal) | Color | – | – | Body | – | Body |
| 6 | Color | Amount (noise) | – | – | Noise | – | – |
| 7 | Amount (noise) | – | – | – | – | – | – |
| 8 | – | – | – | – | – | Spectra Mode | Spectra Mode |
| 9 | – | – | – | – | – | Spectra Tune | – |
| 10 | – | – | – | – | – | Spectra Decay | Spectra Decay |
| 11 | – | – | – | – | – | Spectra Amount | Spectra Amount |
| 16 | Velocity | Velocity | Velocity | Velocity | Velocity | Velocity | Velocity |

Section headings as set in the constructor: Engine "Main", Color "Noise" (Volt/Bit), Spectra
Mode "Spectra", Velocity "Scale".

The software editor (QML `Modules/DrumSynth/Snare/SnareEditor.qml`, `SnareMainTab.qml`,
`SnareModulationTab.qml`, with `DrumSynthMainTabBase.qml`/`DrumSynthModuleScaffold.qml`) has two
tabs, Main and Modulation. Main: TUNE (numeric, + NOTE small knob on Tune Stepped unless the
display is a percentage), the engine selector with a snare/clap picture, VELOCITY slider, DECAY
(curve + PERCENT, GATE toggle in Volt and Bit only). Modulation: per engine, headings and knobs —
Volt: MAIN [Osc Mode selector, Punch (Punchy) or Osc Mix (Tonal)] | NOISE [Color, Amount];
Bit: MAIN [Grit] | NOISE [Color, Amount]; Pow: MAIN [BEND, SHARP]; Clap: MAIN [ROOM, SHARP];
Sharp: MAIN [Skin Tune, Impact, Body, Noise]; Iron: MAIN [Skin Tune, Impact, Body] | SPECTRA
[Mode A/B, Decay, Amount]; Airy/Vintage/Chrome/Breaker: MAIN [Skin Tune, Impact] | SPECTRA
[Mode A/B, Tune, Decay, Amount] **[exact]**.

### 1.3 Tune display (`getTuneValueString` 0x1019d0178)

Bit, Pow, Sharp and Clap show Tune as a percentage (`v·100`, one decimal). The other engines
show a MIDI note number with `"%.2f"` **[exact]**:

| engine | note |
|---|---|
| Volt | `fma(v, 65−41, 41) + 12` = 53 + 24v |
| Airy | `fma(v, 24, 52) − 12 + 12` = 52 + 24v |
| Vintage | `fma(v, 18, 58) − 9 + 12` = 61 + 18v |
| Chrome | 60 + 24v |
| Iron | 46 + 24v |
| Breaker | 61 + 24v |

(`*SDCore::s_fDEFAULTTUNESEMITONES`: Airy 52, Chrome 60, Iron 46, Breaker 61; Vintage 58.)
Tune Stepped's dragger: 100 steps for Bit/Pow/Sharp/Clap, otherwise
`computeNumberSemitonesActualEngine()` steps of 1 (Volt 24, Airy/Chrome/Iron/Breaker 24,
Vintage 18) **[exact]**.

### 1.4 A new Snare

`DATA::Snare::Snare()` defaults, as DECKARD creates one (`snare::DEFAULTS`): mode 0 (Volt),
tune 0.5, decay 0.5, skin_tune 0.5, impact 0.75, body 0.75, spectra_mode 0 (A), spectra_tune
0.5, spectra_amount 0.75, spectra_decay 0.4, velocity 1.0, osc_mode 0 (Tonal), noise_color 0.5,
noise_amount 0.25, punch 0.25, osc_mix 0.5, grit 0.3, gate false, bend 0.5, room 0.5, focus 0.5,
noise 0.4 **[exact]**.

---

## 2. Processor (`DrumSynthWrapper<SnareProcessor>`, `SnareProcessor`)

### 2.1 Wrapper (`DrumSynthWrapper::process` 0x1019e497c, `processAudio` 0x1019e51e4)

- Splits the host block at MIDI/automation event offsets. Note-on (0x9n, velocity > 0) →
  `setNoteOn(vel·0.007874016, note)`; a second note-on **at the same offset is ignored**.
  Note-off (0x8n, or 0x9n with velocity 0) → `setNoteOff(note)`. CC 120 → `onChoke()`.
  Automation events → `setParameter` / `setCoreParameter` **[exact]**.
- `processAudio(offset, n)`: before each piece, if the control counter is 0 it runs
  `SnareProcessor::processControlRate()` and reloads 128; pieces are at most 16 samples and
  never cross a control-rate boundary (both counters persist across calls and start at 0, so
  the first step runs before the first sample) **[exact]**. Output: 2 channels.
- `hasOutputSignal()` of the active core only sets the block's silence flag **[exact]**.

### 2.2 Engines and cores

`SnareProcessor` holds all eight cores (ctor 0x1019f73fc) and stores the shared parameters
at +0x80 + 4·idx (all 0 at construction). `setParamMode` (0x1019f79e4) maps the engine to a
core (+0xb4, "pending") **[exact]**:

| engine | core id | core | core mode param |
|---|---|---|---|
| Volt | 0 | `ClassicSnareCore` | param 2 = 0 (Osc Mode Tonal) / 1 (Punchy) |
| Bit | 0 | `ClassicSnareCore` | param 2 = 2 |
| Pow | 1 | `ClapCore` | param 1 = 0 |
| Clap | 1 | `ClapCore` | param 1 = 1 |
| Iron | 2 | `OrangeSDCore` | – |
| Vintage | 3 | `OldieSDCore` | – |
| Chrome | 4 | `WhippySDCore` | – |
| Airy | 5 | `AirySDCore` | – |
| Sharp | 6 | `DiscoSDCore` | – |
| Breaker | 7 | `BreakerSDCore` | – |

When the engine changes, the **playing** core (+0xb0) is told: Classic/Clap get `onChoke()`
(even when the new engine uses the same core, which then gets its mode parameter at once);
a modal core whose engine is left starts its switch fade (state 1 → 2, gain 1.0). The new core
takes over only at the **next note-on**: `setNoteOn` copies pending → active, gives a Classic/
Clap core its mode parameter, then always calls `updateGlobalCoreParameters()` (all shared
values into the active core) before the core's `setNoteOn` **[exact]**. Audio, control rate,
note-off and choke go to the active core only.

### 2.3 Shared parameter → core parameter (`updateGlobalCoreParameters` 0x1019f8c40)

`setParameter(idx, v)` stores `v` and forwards it to the active core only (`setParamTune` etc.
inline the core's `setParameter`). The core indices **[exact]**:

| shared idx | 1 Tune | 2 Decay | 4 Skin Tune | 5 Impact | 6 Body | 7 Sp. Mode | 8 Sp. Tune | 9 Sp. Amount | 10 Sp. Decay | 11 Velocity |
|---|---|---|---|---|---|---|---|---|---|---|
| Classic | 0 | 1 | – | – | – | – | – | – | – | – |
| Clap | 0 | 5, then 2 | – | – | – | – | – | – | – | – |
| Orange (Iron) | 1 | 2 | 0 | 3 | 6 | 7 | – | 5 | 4 | 8 |
| Oldie / Whippy / Airy | 1 | 2 | 0 | 3 | – | 7 | 6 | 4 | 5 | 8 |
| Disco (Sharp) | 1 | 2 | 0 | 3 | 4 | – | – | – | – | 6 |
| Breaker | 0 | 1 | 2 | 3 | – | 7 | 4 | 6 | 5 | 8 |

Core parameters (`setCoreParameter(core, idx, v)`, 0x1019f8ae8) go straight to that core,
active or not.

### 2.4 Velocity (`setNoteOn` 0x1019f9104)

Modal cores get the raw `vel` (= midi/127) and the Velocity parameter as a core parameter.
Classic and Clap get `x³` with `x = fma(−w, |s|, 1)`, `w = s < 0 ? vel : 1 − vel`, `s` = the
stored Velocity value (−1..1, raw) **[exact]**, null-tested: s = 1 → vel³, s = 0 → 1, s = −1 →
(1 − vel)³.

### 2.5 Osc Mode (`setParamAnalogSnareMode` 0x1019f824c)

Shared index 3 is not sent through `updateGlobalCoreParameters`. When it is set while the
engine is Volt (shared[0] = 0) and the Classic core is active, the core gets param 2 =
(int)v (0 Tonal, 1 Punchy; other values ignored) **[exact]**.

### 2.6 End of the sound (`DrumSynthWrapper::processAudio`)

After a ≤16-sample piece in which the active core's `hasOutputSignal()` is false, the wrapper
flags the engine block's `AlignedAudioBlock` silent (+0x10); the next piece `bzero`s the whole
block buffer before rendering. So the 128-frame engine block in which the sound ends comes out
as zeros (except its last piece) **[exact]**. DECKARD reproduces this within one `process`
call (a 128-block split across two calls keeps its first part).

### 2.7 MIDI

Pitch: every core gets the MIDI note at note-on (cores decide how to use it; Classic and
Clap follow it, see §3). Note-off goes to the active core (only Volt/Bit with GATE use it).
The wrapper chokes on CC 120, but a host CC 120 never reaches it in Maschine (a render with
`--cc 0.1:120:0` equals one without): DECKARD ignores it. Pitch bend/mod wheel are not mapped.
Mono: a new note-on retriggers the core (with a de-click crossfade in Classic).

---

## 3. `ClassicSnareCore` (Volt, Bit) — `snare/classic.rs`, bit-exact

Parameters: 0 tune, 1 decay, 2 mode (0 Volt Tonal, 1 Volt Punchy, 2 Bit), 3 osc mix, 4
punch, 5 grit, 6 noise colour, 7 noise amount, 8 gate **[exact]**. Per sample **[exact]**:

- Two sines with a common pitch `f = note2hz(base + depth·penv)` (`penv` a one-pole decay
  to 0, time from tune/punch). Each is a triangle phase through a 7th-order sine polynomial
  (osc 1 at `1.75129·f`, osc 2 at `f`); osc 2 is crossfaded towards `1.4·velocity` by a 1 ms
  click envelope. `osc = amp·(amp²·mix·sin1 + (1−mix)·sin2')`, `amp` = the amp FreeEnvelope ×
  velocity peak. A retrigger crossfade continues the previous note's slope.
- Noise: an LCG through a 9 kHz one-pole low-pass, two TPT SVFs in series (k = 1.6) at
  `note2hz(base + 53 + colour)`; `bp2 + colfac·srfac·0.4·hp2`, ×20, × the noise envelope
  (FreeEnvelope × velocity, max'd with a decaying peak hold, faded out over 15 ms below 1e-4),
  hard-clipped to ±4 and softly saturated. The LCG restarts from its fixed seed at reset, so
  renders are deterministic.
- Volt: `osc + amount·(0.65 + 0.35·vel)·noise`. Bit: noise+osc through a dead zone, a comb
  (delay `1/note2hz(colour)`), a sample-and-hold "bit crusher" pair at grit-dependent rates and
  a third SVF (k = 1.4): `0.75·amount·bp3 + S&H(osc)`.
- Optional gate envelope (GATE, released by note-off), a one-pole high-pass at
  `note2hz(base − 5)`, × 0.55015 (Volt) / 0.47367 (Bit) × the choke envelope.
- All 16 smoother targets ramp over `round(0.02·sr/128)` control steps. Maschine compiles 16
  variants of the loop (`processAudioImpl<bit, gate, ramp, dirty>`); the non-dirty ones skip
  coefficient updates for values that do not ramp, and the port reproduces that.

Full description in the file's doc comments (with `classic/env.rs`: FreeEnvelope).

## 4. `ClapCore` (Pow, Clap) — `snare/clap.rs`, bit-exact

Parameters: 0 tune, 1 mode (0 Pow, 1 Clap), 2/5 decay, 3 room, 4 bend, 6 focus **[exact]**.

- Pow: LCG noise through a resonant band-pass whose pitch follows a bend envelope, a `7000·t`
  low-pass, then a `500·t` band-pass gated by envelope A and a soft-clipped tone low-pass
  opened by a short hit envelope; a low shelf at the smoothed tune, a 100 Hz high-pass and an
  `atan` saturator.
- Clap: the same band-pass noise (envelope A) plus a second noise layer (envelope B, `6300·t`
  band-pass); the hit envelope retriggers three times (the "hands"); the sum passes a bell,
  the low shelf and a second bell.
- `processAudioImpl<clap, ramp, first>`: the first block after a note-on and blocks while
  parameters ramp recompute the coefficients that depend on smoothed parameters.

Full description in `clap.rs` / `clap/dsp.rs` (BiQuad, FreeEnvelope).

## 5. The modal cores (Sharp, Airy, Vintage, Chrome, Iron, Breaker) — not finished

`DiscoSDCore`, `AirySDCore`, `OldieSDCore`, `WhippySDCore`, `OrangeSDCore`, `BreakerSDCore`
are compiled modal-synthesis kernels (≈0x42000 bytes of state each) built from shared
templates: `AttackCore` (FluffyOsc click, `MainOscAttack` phase-coupled oscillator, filtered
`ParallelNoise`), `BodyCore`/`BodyTableCore` (a bank of up to 32 resonators,
`UTIL::ModModalSynth`, struck by `BodyExciter<true>`, optional `UTIL::Air` and feedback
damping), `AttachedBodiesCore` (the snare wires / "Spectra": resonators fed by sampled noise)
and a `*SDRealTimeMod` that maps parameters and velocity onto modulation targets
(`sModulationValues` lo/hi tables, `sVelocityValues` curves); `initPreset` writes the modal
frequency/amplitude/decay tables. A mode switch away from such a core starts a 15 ms fade
(state 1 → 2).

Status in DECKARD: Sharp (`snare/disco.rs`) has the attack layer, the realtime mod, the
smoother and the output mix ported; its body and attached bodies are stubs, so it plays only
the attack (residual −43.6 dB vs Maschine). Airy, Vintage, Chrome, Iron and Breaker are
silent stubs. Partial, not yet wired work exists in `snare/oldie/` (BodyCore<Double, …>
and AttackCore as a byte-image port) and `snare/orange/` (AttackCore, MemoEnvelope); those
directories are not compiled (their parent files do not declare them).

### 5.1 NI noise data the modal cores play (not reproducible)

`UTIL::loadSampleFromResources(id)` (0x1018b1944) decodes compiled-in WAVE resources
(`ni::compiled_resources`, type "WAVE", registered in
`register_resource_MaschineHeadlessResources_yaml()`):

| id | resource | data address (arm64) | format | used by |
|---|---|---|---|---|
| 3 | `ModalNoises_23x131072.wav` | 0x1057517a9 (6 029 438 bytes) | mono 16-bit 44.1 kHz, 23 × 131072 samples back to back | `AttachedBodiesCore` ctor (0x101963d38, 0x101966000): the Spectra / Noise layers of all six modal snares (and modal kicks, toms, perc, cymbals, hi-hats) |
| 4 | `ModalPinkNoise_1x131072.wav` | 0x105d11827 (262 188 bytes) | mono 16-bit 44.1 kHz, 131072 samples | `UTIL::Air` ctor (0x1018987b4), built in every `BodyCore`; played where `UseAir` = 1 (Sharp, Iron) |

The 23 noises are differently coloured (lag-1 autocorrelation 0.58–0.999), each peak-
normalized to full scale: offline-rendered data, not a PRNG stream DECKARD could regenerate.
DECKARD must run without NI software, so it neither reads nor embeds them: the layers that
play them stay silent in a port, and those engines cannot null at nonzero Spectra Amount /
Noise.

---

## 6. Null tests (Maschine 3 VST3 vs DECKARD `synth-null`, `--preroll 0`)

Source Lightspeed 0/1 with the fields set as listed. Residual = RMS of the difference.

| case | options | residual |
|---|---|---|
| Volt Tonal defaults | note 60:100, 48k | −240 dB (bit-exact) |
| Volt Punchy | osc_mode=1 | −240 dB |
| Volt tune 0 / 1, decay 0 / 1, colour 1, retrigger, choke, 44.1k, 96k, vel 127 | | −240 dB |
| Volt velocity field 0, two notes 64/127, 44.1k | | −240 dB |
| Volt / Bit GATE on (note-off) | gate=1 | −126…−137 dB (one 128-frame tail block) |
| Bit defaults, colour 0/1, decay 1, 44.1k, 96k | mode=1 | −240 dB (≤ 1e-38 denormal diffs) |
| Pow: defaults, tune/decay/room/focus/bend 0/0.5/1, vel 1/64, velocity −1/−0.5/0, notes 36–96, retrigger, choke, 44.1k, 96k | mode=2 | −240 dB |
| Clap: same matrix | mode=8 | −240 dB (≤ 1.1e-36 denormal diffs) |
| engine switch Pow → Clap between notes | | −240 dB |
| Sharp defaults / noise 0 | mode=3 | −43.6 dB (body not ported) |
| Airy, Vintage, Chrome, Iron, Breaker | | DECKARD's own designs, not null-tested (docs/drum-engines-own.md) |

Pitch: the Classic and Clap engines follow the MIDI note (tested 36–96).
