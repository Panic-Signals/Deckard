# Maschine's Cymbal: parameters and DSP

A clean-room description of Maschine 3's internal **Cymbal** drum synth
(`NI::MASCHINE::DATA::Cymbal` → `NI::MASCHINE::RT::DrumSynthWrapper<NI::DSP::CymbalProcessor>`
→ `NI::DSP::CrashCymbalCore` / `NI::DSP::RideCymbalCore`). The DSP is NI's modal-drum kit:
an attack generator (`AttackCore`), a 512-partial modal body with an "air" noise layer
(`CymbalBodyCore`), a bank of noise/crispel resonators (`AttachedBodiesCore`), a 15-line
feedback delay network (`FdbckDelayNetwork`), a macro layer mapping the knobs onto all of
them (`RealTimeModulation`), and output shelving filters.

It was written from Ghidra output and arm64 disassembly of the **VST3 plug-in's** arm64
slice (`/Library/Audio/Plug-Ins/VST3/Maschine 3.vst3`, VM addresses below are in that slice;
the data-model functions were read in `analysis/maschine_arm64`, the app). The DECKARD port
is `crates/engine/src/synth/cymbal.rs` with submodules in `crates/engine/src/synth/cymbal/`.

**Confidence.** **[exact]**: formula and constants read from code or assembly and confirmed by
the null tests. **[inferred]**: reconstructed from partial evidence. **[unknown]**: not
recovered.

Notation: `v` a stored parameter value; `sr` the sample rate; all arithmetic `f32` unless
marked `f64`; "fused" means a single-rounding multiply-add (`fmadd`, `f32::mul_add`).
`E(x)` is Maschine's exp table (`NI::AB::ExpTable`, docs/fx-eq.md §2.1),
`E(x) ≈ 2^((x − 1200)/60)`; `K = 9.965784` (semitones → table steps).

---

## 1. Parameters

### 1.1 Module

Module name "Cymbal" (`Module::Module(L"Cymbal")`). Two engines (Engine enum): **Crash**
(0) and **Ride** (1) **[exact]** (`__GLOBAL__sub_I_Modules_DrumSynth_Objects_ALL.cpp`
builds the option vectors at 0x10e305c60 and 0x10e305c78). Pages come from
`DrumSynth::getPagesNamesStatic()` (0x1019c4a74): **Main**, **Advanced**, **Modulation**,
8 slots each.

### 1.2 Parameters in constructor order

`Cymbal::Cymbal()` (0x1019d5dac) creates each parameter with
`TransactionObjectContainer::insert(label, default, min, max)` **[exact]**. Every numeric
parameter's display converter (+0x140) is
`DefaultConverters::ScaleNormalizedToPercentOneDecimal` (`v·100`, one decimal, `%`).

| obj off | mxprj field | label | section | default | range | RT route |
|---|---|---|---|---|---|---|
| 0xa8 | `engine` | **Engine** (enum: Crash, Ride) | "Main" | 0 | 0..1 | shared 0 (via `setParamModeAndTuneSteps`) |
| 0xb0 | `tune` | **Tune** | | 0.3 | 0..1 | shared 1 |
| 0xb8 | *(not stored)* | **Tune Stepped** | | 0.3 | 0..1 | mirrors Tune (20 semitone steps) |
| 0xc0 | `decay` | **Decay** | | 0.7 | 0..1 | shared 2 |
| 0xc8 | `impact` | **Impact** | | 0.7 | 0..1 | shared 3 |
| 0xd0 | `width` | **Width** | | 0.5 | 0..1 | shared 4 |
| 0xd8 | `velocity` | **Velocity** (bipolar) | "Scale" | 1.0 | −1..1 | shared 5 (raw value) |
| 0xe0 | `density` | **Density** | "Color" (span 3) | 1.0 | 0..1 | core 0 (Crash), param 2 |
| 0xe8 | `tone` | **Tone** | | 0.5 | 0..1 | core 0, param 3 |
| 0xf0 | `crash_mode` | **Mode** (enum: Metallic, Normal, Soft) | | 0 | 0..2 | core 0, param 6 (enum) |
| 0xf8 | `edge` | **Edge** | "Strike" (span 2) | 0.5 | 0..1 | core 1 (Ride), param 2 |
| 0x100 | `bell` | **Bell** | | 0.0 | 0..1 | core 1, param 3 |
| 0x108 | `character` | **Character** | "Color" (span 2) | 0.2 | 0..1 | core 1, param 5 |
| 0x110 | `tail` | **Tail** | | 1.0 | 0..1 | core 1, param 6 |

Tune Stepped (0xb8) is not serialized: `serialize` writes 0xa8, 0xb0, then 0xc0…0x110
**[exact]**. Tune and Tune Stepped mirror each other (`onTuneParameterChanged` /
`onTuneSteppedParameterChanged`); the engine selector installs a wheel handler with
`computeNumberSemitonesActualEngine() = 20` steps and a fine resolution of 0.01.
Tune's display is a percentage, so the QML shows Tune as `normalized·100` with two decimals
and its small knob without the "NOTE" label.

### 1.3 Knob layout (`getParameterByPosition`, 0x1019d7700)

| pos | page | Crash | Ride |
|---|---|---|---|
| 0 | Main | Engine | Engine |
| 1 | Main | Tune | Tune |
| 2 | Main | Decay | Decay |
| 3 | Main | Impact | Impact |
| 4 | Main | Width | Width |
| 5 | Main | Density | Edge |
| 6 | Main | Tone | Bell |
| 7 | Main | Mode | – |
| 8 | Advanced | – | Character |
| 9 | Advanced | – | Tail |
| 16 | Modulation | Velocity | Velocity |

Section headings as set in the constructor: slot 0 "Main", Density "Color" (3 wide),
Edge "Strike" (2 wide), Character "Color" (2 wide), Velocity "Scale".

### 1.4 The QML editor

`Modules/DrumSynth/Cymbal/CymbalEditor.qml` (`DrumSynthModuleScaffold`, secondary name
"CYMBAL") with `CymbalMainTab.qml` (`DrumSynthMainTabBase`: TUNE numeric area + small knob,
the engine cycler with `cymbal.svg`, VELOCITY bipolar slider, DECAY visualizer + PERCENT)
and `CymbalModulationTab.qml`:
- **Crash**: MAIN: Impact, Width; COLOR: Density, Tone, Mode (option cycler).
- **Ride**: MAIN: Impact, Width; STRIKE: Edge, Bell; COLOR: Character, Tail.

### 1.5 Value → engine

`DrumSynthWrapper::processSharedParameter(idx, offset, v)` (0x1e17714) calls
`CymbalProcessor::setParameter(idx, v)` at once (offset 0) or queues it; likewise
`processCoreParameter(core, idx, offset, v)` → `CymbalProcessor::setCoreParameter`, and
`processCoreEnumParameter` converts the enum to float first **[exact]**.

`CymbalProcessor::setParameter(i, v)` stores `v` at +0x80+4i, then:
- 0 Mode: pending engine = `(u32)v` (+0xa4); if it differs from the current engine (+0xa0),
  the current core is choked (fade out). The new engine takes over at the next note-on.
- 1 Tune → core param 0, 2 Decay → 1, 3 Impact → 4, 4 Width → Crash 5 / Ride 7,
  5 Velocity scale → Crash 7 / Ride 8, each sent to the *current* core.

`setCoreParameter(core, i, v)` sends to Crash (core 0) or Ride (core 1) regardless of the
current engine.

---

## 6. The noise recordings

Both engines play two compiled-in `WAVE` resources (`ni::compiled_resources`, symbols
`__ZL20resource_data_WAVE_<id>`, decoded once by
`UTIL::loadSampleFromResources(id, block)`, 0x1018b1944 in the app):

| id | file | samples | RIFF bytes | VST3 arm64 slice addr | read by |
|---|---|---|---|---|---|
| 3 | `ModalNoises_23x131072.wav` | 23 × 131072 | 6029430 | 0x56ccf26 | `AttachedBodiesCore` noise layer (`NoiseSampler`) |
| 4 | `ModalPinkNoise_1x131072.wav` | 131072 | 262188 | 0x5c8cfa4 | `UTIL::Air` in the body (`PinkSampler`, resampled once for sr ≠ 44.1k) |

Mono 16-bit PCM, 44.1 kHz, decoded as `max(n·f32(1/32767), −1)` **[exact]** (checked
against Maschine's decoded blocks). WAVE 3 carries a LIST/INFO chunk "2011-11-07",
"Sony Sound Forge 9.0 (libsndfile-1.0.24)": both are authored audio files, and no Maschine
code generates them, so DECKARD can neither ship nor regenerate them. DECKARD plays without
them (those two layers silent), so the Cymbal cannot null against Maschine.

## 7. Port status

Not finished (stopped on the data question above). Ported and verified bit-exact against
the plug-in's own functions (oracle tests, `#[ignore]`): `FdbckDelayNetwork` (fdn.rs),
`AttachedBodiesCore` both variants (attached.rs), `AttackCore` both variants (attack.rs),
`MemoEnvelope` (memo.rs), the modal bank `ModModalSynth::processAudio_optimized` (body/modal.rs).
Written, not yet verified: `RealTimeModulation` Crash/Ride knob functions (rtmod.rs),
the CymbalProcessor/wrapper layer (cymbal.rs). Partial, not compiled: body/air.rs,
body/exciter.rs, body/src.rs. Not started: BodyCore/CymbalBodyCore, the Crash and Ride
cores (initPreset, setParameter, note-on, control/modulation rate, mixing, output shelves).

