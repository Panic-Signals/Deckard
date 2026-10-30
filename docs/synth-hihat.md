# Maschine's Hi-hat: parameters and DSP

A clean-room description of Maschine 3's internal **Hi-hat** drum synth
(`NI::MASCHINE::DATA::Hihat` → `NI::MASCHINE::RT::DrumSynthWrapper<NI::DSP::HihatProcessor>`).
`HihatProcessor` owns three DSP cores and runs one at a time:

| engine (`mode`) | core | DECKARD |
|---|---|---|
| 0 Silver | `NI::DSP::Hihat606Core` (Silver flag on) | `synth/hihat/c606.rs`, bit-exact |
| 1 Circuit | `NI::DSP::Hihat606Core` (Silver flag off) | `synth/hihat/c606.rs`, bit-exact |
| 2 Memory | `NI::DSP::IFFTCymbalCore` | `synth/hihat/memory.rs`, bit-exact (needs NI recordings, §5) |
| 3 Hybrid | `NI::DSP::HybridHihatCore` | DECKARD's own design (docs/drum-engines-own.md) |

Written from Ghidra output in `out/decomp/hihat/` and `out/decomp/hihat2/` (`INDEX.tsv`
maps files to functions) and from arm64 disassembly of `analysis/maschine_arm64`;
addresses below are VM addresses there. The QML editor is
`Modules/DrumSynth/Hihat/HihatEditor.qml` (scaffold), `HihatMainTab.qml` (the shared
drum-synth main tab: Tune/Decay/Gate/mode picture) and `HihatModulationTab.qml` (the
per-engine knobs), plus `DrumSynth/DrumSynthMainTabBase.qml`.

**Confidence.** **[exact]**: read from code and confirmed by a bit-exact null.
**[inferred]**: reconstructed. **[unknown]**: not recovered.

---

## 1. Parameters

### 1.1 Module

Module name "Hi-hat" (`s_aModuleInfo`, wide string built in
`__GLOBAL__sub_I_Modules_DrumSynth_Objects_ALL.cpp` @0x1019f14f8). Pages
(`DrumSynth::getPagesNamesStatic` @0x1019c4a74, shared by all drum synths):
**Main** (positions 0–7), **Advanced** (8–15), **Modulation** (16–23) **[exact]**.
Class version 2; version 1 projects lack the last five objects (§1.4).

### 1.2 Parameters in constructor order (`Hihat::Hihat()` @0x1019c4fd4)

Every parameter is made by `TransactionObjectContainer::insert(label, default, min, max)`
**[exact]**. "RT" is where `connectParameters` (@0x1019c5d30) sends it: `shared n` =
`HihatProcessor::setParameter(n)` (applies to the active core, and is re-sent to the core
at every note), `core c,i` = `setCoreParameter(c, i)` → that core's `setParameter(i)`
(always, active or not).

| obj off | mxprj field | label | section | default | range | RT | display |
|---|---|---|---|---|---|---|---|
| 0xa8 | `mode` | **Engine** | "Main" | 0 | Silver, Circuit, Memory, Hybrid | shared 0 (via `setParamModeAndTuneSteps`) | enum; not automatable |
| 0xb0 | `tune` | **Tune** | | 0.5 | 0..1 | shared 1 | percent, 1 decimal; fine/coarse step 0.1; wheel steps from `computeNumberSemitonesActualEngine` = 20 |
| 0xb8 | `decay` | **Decay** | | 0.65 | 0..1 | shared 2 | percent |
| 0xc0 | `color` | **Color** | "Tone" | 0.715 | 0..1 | shared 4 | `getColorValueString`: Silver/Circuit the band-pass frequency `calculateColorFreq(v, 0)` as `"%.2f Hz"` (< 10), `"%.1f Hz"` (< 1000), `"%.1f kHz"`; Memory/Hybrid percent |
| 0xc8 | `impact` | **Strike** | | 0.8 | 0..1 | shared 5 | percent (v2 only) |
| 0xd0 | `gate` | **Gate** | | false | bool | shared 3 | on/off |
| 0xd8 | `velocity_scaling` | **Velocity** | "Scale" | 1.0 | −1..1, bipolar | shared 6 | percent |
| 0xe0 | `hihat606_sat` | **Saturate** | | 0.19 | 0..1 | core 0,3 | percent |
| 0xe8 | `hihat606_noise` | **Noise** | | 0.1 | 0..1 | core 0,5 | percent |
| 0xf0 | `hihat606_seed` | **Seed** | | 0.2 | 0..1, 31 steps | core 0,6 | `getSeedValueString`: the character `"ABCDEFGHIJKLMNOPQRSTUVWXYZ!?&#*"[(u32)(v·30)]` |
| 0xf8 | `hihat606_random` | **Dissonance** | | 0.25 | 0..1 | core 0,7 | percent (QML label "Random") |
| 0x100 | `cymbal_strike` | **Strike** | | 0.75 | 0..1 | core 1,2 | percent; overridden by `impact` at every note (§4.2), so it has no audible effect |
| 0x108 | `cymbal_sample` | **Source** | | 0 | A, B, C, D, E, F | core 1,3 (enum) | enum; not automatable |
| 0x110 | `cymbal_distress` | **Distress** | | 0 | 0..1 | core 1,5 | percent |
| 0x118 | `hybrid_metallic` | **Metallic** | | 0 | 0..1 | core 2,3 | percent (v2) |
| 0x120 | `hybrid_bend` | **Bend** | "Hybrid" | 0.5 | 0..1, bipolar | core 2,5 | `getHybridBendValueString`: percent of `2v − 1` (−100 %…+100 %) (v2) |
| 0x128 | `hybrid_rattle` | **Rattle** | | 0.5 | 0..1 | core 2,6 | percent (v2) |
| 0x130 | `hybrid_character` | **Character** | | 0.25 | 0..1 | core 2,7 | percent (v2) |

Field names are Maschine's `HihatModel` properties (`getXBinding` getters @0x1019… read
the same offsets − 0x50). A new Hi-hat starts with exactly the defaults above.

### 1.3 Knob layout (`getParameterByPosition` @0x1019c7238) **[exact]**

| pos | Silver (0) | Circuit (1) | Memory (2) | Hybrid (3) |
|---|---|---|---|---|
| 0 Main | Engine | Engine | Engine | Engine |
| 1 | Tune | Tune | Tune | Tune |
| 2 | Decay | Decay | Decay | Decay |
| 3 | Gate | Gate | Gate | – |
| 4 | Color | Color | Source | Color |
| 5 | Saturate | Saturate | Color | Metallic |
| 6 | Noise | Seed | Strike (`impact`) | Strike (`impact`) |
| 7 | – | Dissonance | Distress | Character |
| 8 Advanced | – | – | – | Bend |
| 9 | – | – | – | Rattle |
| 16 Modulation | Velocity | Velocity | Velocity | Velocity |

Section headings: "Main" on Engine, "Tone" on Color, "Scale" on Velocity, "Hybrid" on
Bend. The QML modulation tab groups them the same way: Silver/Circuit "TONE"
(Color, Saturate, Noise or Seed, Random); Memory "MAIN" (Source option cycler) | "TONE"
(Color, Strike, Distress); Hybrid "HYBRID" (Bend, Rattle) | "TONE" (Color, Metallic,
Strike, Character). The Gate switch shows only for Silver, Circuit and Memory.

### 1.4 Serialization

`serialize<portable_binary_iarchive>` @0x1019c4c50 loads, after the Module base:
0xa8, 0xb0, 0xb8, 0xc0, 0xd0, 0xd8, 0xe0, 0xe8, 0xf0, 0xf8, 0x100, 0x108, 0x110, then for
version > 1: 0xc8, 0x118, 0x130, 0x120, 0x128 (`crates/mxprj` `module.rs`).

---

## 2. The processor and wrapper **[exact]**

- **Events** (`DrumSynthWrapper::process` @0x1019db7b8): note-on velocity `v = vel·(1/127)`
  (f32 `0x3c010204`); a second note-on at the same sample is dropped; note-off
  releases the gate whatever the note; parameter events at or before a note's time are
  applied first. CC 120 is decoded as "choke" by the wrapper but never reaches it from a
  sound: CCs, pitch bend and the mod wheel have no effect (null-tested, §7).
- **Blocks** (`processAudio` @0x1019dc020): `processControlRate` every 128 samples and
  `processAudio` in pieces of at most 16 samples, both counters running on across calls
  from the start of the engine. After a piece, if the active core reports no signal,
  the output block is marked silent; a piece that starts on a silent block clears the
  whole block first. Maschine's blocks are 256 samples here, so the end of a voice can
  erase earlier samples of its 256-block (seen with Gate, §7).
- **Mode** (`setParamMode`): `pending = (m == 3) ? 2 : (m == 2)`. If the active core is
  the 606 and the new engine is Silver/Circuit, it chokes the voice (5 ms) and sets the
  core's Silver flag (taken at the next note); any other change chokes the old core's
  voice. The engine switches at the next note-on.
- **Note-on** (`HihatProcessor::setNoteOn` @0x1019f4218): switch core if pending, then
  `updateGlobalCoreParameters` re-sends Tune, Decay, Gate, Color (and Strike to Memory),
  then the core's `setNoteOn(v′, note)` with velocity scaling
  `v′ = (1 − |V|·(V ≥ 0 ? 1 − v : v))³` (`fmsub`, then `a·(a·a)`). It is monophonic:
  a new note restarts the one voice.
- `DSPCoreWrapper` constructor runs `onSetSampleRate(44100, 128)`, the engine then
  `onSetSampleRate(sr)`; `resume` calls each core's `onReset`.

### 2.1 Shared building blocks

- `E(x)`: `NI::AB::ExpTable` (docs/fx-eq.md §2.1, `fx::exp_e`); pitch `p` → Hz is
  `E(fma(p + 70, 5, 1381.8816))`, dB → gain `E(fma(dB, 9.965784, 1200))`, a decay time in
  dB `E(fma(dB, 9.965784, 602.0529))` = `0.001·10^(dB/20)` s.
- `tan3(w) = w + fma(w⁵, 0x3e088886, w³·0x3eaaaaaa)`, `w = (1/sr)·(f·π)`.
- SVF: `g = tan3(…)`, `h = 1/fma(g, k + g, 1)`; per sample
  `hp = x − h·fma(x, fma(g,k,g²), fma(2ic1, k, fma(ic2, 2, g·2ic1)))`,
  `v1 = fma(g, hp, ic1)`, `bp = ic1 + v1`, `v2 = fma(g, bp, ic2)`. Cutoff clamped to
  `[0, 0.49·sr]`, recomputed only when it changes.
- TPT one-pole: `y = a·(x − b·z)` (`fmsub`), `a = 1/(g+1)`, `b = g − 1`; low-pass
  `g·(z + y)`, high-pass `y − z`.
- `FreeEnvelope<off, breakpoint, lin, log, lin>` (`synth/hihat/env.rs`): an `f64` ramp
  `x` 1 → 0; attack `x −= (1/sr)/att`, level `1 − x`; decay `x ·= exp(−ln8/(T·sr))`,
  level `fma(1−sus, (x − 1e-4)·1.000100010001, sus)`; ends when `x ≤ 1e-4` or the stage
  time is over; release is forced to 0 s (instant). The hi-hat cores use sustain 0 and
  no velocity sensitivity.
- `SmoothingToolbox<N>`: a parameter change sets targets; the next control tick sets
  increments `(target − current)/(n·128)` with `n = round(0.02·sr/128)` (20 ms), drops
  those with `inc² < 1e-15` (in `f64`), and ramps per sample while the cores' "C"
  template variants run; after `n` ticks the values snap to the targets.
- `processAudioImpl<…, C, D>` template flags: **D** ("fresh", set by note-on, Gate,
  sample-rate and reset) recomputes every coefficient each sample for one block; **C**
  (smoothing) recomputes only those whose increment is nonzero. Otherwise the stored
  coefficients are used, so stale coefficients after a ramp are part of the sound.
- The retrigger crossfade: at note-on the previous level is held and decays by
  `2^(−3/(sr·τ))` (cubic `2^x`), `τ` = 0.4·decay (606) / min(decay/2, 1) (Memory); the
  level is `max(old, new)`. Below `1e-4` a cubic fade of 15 ms closes the voice tail.
- Gate (1 ms) and choke (5 ms) envelopes: `l ·= exp(−ln2/(t·sr))`, off below `1e-4`.

---

## 3. Silver and Circuit (`Hihat606Core`) **[exact]**

Core parameters (`setParameter` @0x1019053dc): 0 Tune → pitch
`st = clamp(fma(t, 48, −24) + note − 60, ±24)`, frequency `E(pitch st)`;
1 Decay → `25 + 40·d` dB; 2 Color → band-pass `E(pitch clamp(fma(c, 50, 82) + note − 60,
82, 132))` and high-pass at 0.85× that; 3 Saturate → `dB = fma(s, 24, −6)`, drive
`E(dB)`, make-up `E(10·clamp((dB−2)/13) − dB)`; 4 Silver flag; 5 Noise; 6 Seed → six
increments from an LCG (`r = r·0x7d3adeb + 0x12aa0131`, value
`fma(fma(r, 2^-32, 0.5), 0.3, 0.1)`) seeded by `(int)(v·30)`; 7 Dissonance; 8 Gate.
Values 0–3, 5, 7 are smoothed (8 slots).

Per sample:

- **Silver source**: six pulse oscillators at 1, 1.25, 1.49, 1.7, 1.79, 2.55 × f
  (phase `a = wrap(φ + inc)`, `b = wrap(φ + inc + 1.525)`, out `a − b`, `wrap(t) = (t+10)
  − (int)(t+10.5)`), summed, through an 8 kHz one-pole low-pass, cross-faded with white
  noise (LCG `n·0x41c64e6d + 0x3039`, scaled `√(sr/44100)·2^-31`) by Noise:
  `fma((1−x)(2−(1−x)), osc, x(2−x)·noise)`. Above Noise 0.999 the oscillators stop.
- **Circuit source**: a clock at `30·f/sr`; each tick updates six "counters"
  `c = wrap(fma(a, c, r_i))` (`a = fma(Dissonance, 0.63, 0.57)`, `r_i` from Seed) as a
  binary divider (counter i steps every 2^i ticks); output the previous counters' sum
  `·0.4·3`.
- Band-pass SVF (k ≈ 0.2) at Color; envelope follower (1 ms / 10 ms) normalises it:
  `q = max(clamp(bp/max(env, 0.01), ±1), −0.1)`.
- Two envelopes (attack 1.3 ms): a click with decay `min(T, 30 ms)` (level 0.7) and the
  main decay `T = 0.001·10^((Decay dB + 3(v′−½))/20)` (level 0.5, minus 0.15):
  `level = v′·(e1 + max(e2 − 0.15, 0))`, through the retrigger crossfade.
- High-pass SVF (k = 1.6) at 0.85·Color, drive, the saturator
  `sign(y)·(0.7 − 0.49/(|y| + 0.7))`, make-up gain, gate (when on) and choke, ×1.292219
  (Silver) or ×1.219929 (Circuit). Mono, both channels equal.

## 4. Memory (`IFFTCymbalCore`) **[exact]**

### 4.1 Parameters

Core parameters: 0 Tune, 1 Decay, 2 Strike, 3 Source, 4 Color, 5 Distress, 6 Gate.
Twelve smoothed values: tune st, PM depth, strike gain, decay T, decay mix, EQ gain dB,
EQ frequency, decimator mix, Distress mix, Distress pitch, shelf gain, loop rate.

- Tune: `st = clamp(fma(t, 24, −12) + note − 60, ±12)`; loop rate `2^(st/12)·44100/sr`;
  decimator mix `0.85·min(max(st/−5, 0), 1)`; pitch
  `f = E(pitch(PITCH[src] + 70 + st))` with `PITCH = −8, −7, 4, 11, −7, 3`.
- Decay: `d = clamp(fma(|v|, −0.2, 1.2)·v)`, `dB = fma(d, 34, 24)`, `T = E(dB as time)`,
  mix `clamp((dB−42)/−7)`, PM-envelope decay `fma(c, 0.12, 0.08)`
  (`c = clamp((dB−41)/−5)`).
- Strike: gain `E(10·s dB)`, attack `fma(clamp(10s), −0.005, 0.006)` s (from the next
  note), PM depth `4.46431e-5/(√f·0.00681) · (PM_AMT[src]·s) · fma(c, −0.4, 2.4)`
  (`PM_AMT = 1, 1.3, 0.6, 0.7, 1.5, 1.3`).
- Color: EQ frequency `4700·2^(st′/12)` (`st′ = 24·min(c/0.715,1) − 24` up to 0.715,
  then `12·min((c−0.715)/0.285, 1)`), EQ gain `max(−24·2c, −24)` dB.
- Distress `x`: second decimator mix `0.9·x(2−x)`, its pitch `st″ + 125 − 2x(2−x)`
  (`st″ = st` or `st/2` when ≤ 0), shelf gain `3·x(2−x)` dB.

### 4.2 Voice

`setNoteOn`: Tune again with the note; targets jump to the values (all of them for a new
voice; only some while the voice still sounds); the four envelopes restart; Strike's
attack applies to the next note; the noise band-pass moves to `f`. `onReset` (at
`resume` and when a voice ends) clears the filters and draws the loop start
`rand()·2^17·2^-31` from libc `rand()` (Park–Miller, seed 1, process-global).

### 4.3 Per sample

1. White noise (LCG) → band-pass SVF at `f` (k 0.4); times the PM envelope (attack one
   sample, decay above, level 0.7) and the PM depth: `pm`.
2. Loop playback: position `+= rate`; read at `pos + 44100·pm` with 4-point Hermite
   interpolation from the 131072-sample recording (index masked to 17 bits).
3. Decimator (rate `E(pitch fma(st, 0.9, 135.06))/sr`), 250 Hz one-pole high-pass.
4. With Distress > 0: second decimator, `BiQuad` shelf (5 kHz, q 0.2, gain above), 600
   Hz high-pass.
5. Peaking EQ 1 at 4700 Hz (`k = clamp(10.316/(A·3.364), 0.01, 100)`, `A = 2^(dB/12)`,
   gain `(A²−1)·k`), peaking EQ 2 at the EQ frequency (k 0.76669, gain 11.5), output gain
   `E(−dB/2)`.
6. 6 kHz band-pass (k 1.6) → envelope follower (not fused: `env += d·c`) → normaliser
   `3/env` (300 below 0.01).
7. Envelopes: e1 decay `min(T, 70 ms)` (level 0.5), e2 `min(T, 120 ms)` (level
   `fma(strike gain, 0.3, 0.6)`), e3 `T` (level 0.4): level
   `v′·fma(e2 − s, mix, s)`, `s = fma(strike gain, e1, max(e3 − 0.03, 0))`, through the
   retrigger crossfade.
8. Drive `E(8·e1 dB)`: `y = fma(drive − 1 + 0.7, hp8k(x·norm), x·norm)·level`, clip at
   ±12, knee `y·fma(−|y|/4, 1/6, 1)`, 200 Hz high-pass, gate, choke, ×0.0307669.

The voice runs while any of e1–e3 runs (and the gate, when on) and the choke is on.

## 5. NI data

Memory plays Native Instruments' recordings: `loadSampleFromResources(5…10)` decodes
compiled-in `WAVE` resources (`resource_data_WAVE_5`…`_10`, mono 16-bit 44.1 kHz, 131072
samples each). DECKARD does not ship them: `synth/hihat/res.rs` (copied from the Cymbal
port's loader and extended) reads them by symbol from the user's Maschine 3 binary (VST3
or app; `MASCHINE_BINARY` overrides). Without an install, Memory is silent. Hybrid would
also need WAVE 3 and 4 (the modal noises).

## 6. Hybrid (`HybridHihatCore`) — not ported; DECKARD plays its own design (docs/drum-engines-own.md) **[unknown]**

`HybridHihatCore` (state ≈ 0x83b00 bytes) is a variant of the Cymbal's
`CrashCymbalCore`: it constructs and runs a `CrashCymbalCore`-like voice (`BodyCore`,
`AttackCore`, `AttachedBodiesCore` reading WAVE 3, `FdbckDelayNetwork`, `ModModalSynth`,
`UTIL::Air` reading WAVE 4) with its own `initPreset` (@0x1019440b8, ~10 KB of preset
values), `HybridHihatRealTimeMod` (`setTune`, `setDecay`, `setColor`, `setMetallic`,
`computePerformance`) and `computeTune/Metallic/VelocityModulations`. Core parameters:
0 Tune, 1 Decay, 2 Color, 3 Metallic, 4 Strike, 5 Bend, 6 Rattle, 7 Character; Tune
target pitches `60 ∓ 14·t` (two integer notes). In DECKARD it is silent; porting it means
reusing the Cymbal engine's components (being ported in `synth/cymbal/`).

---

## 7. Null tests

`maschine-ref make <src> made.mxprj G S 0 field=value…`, `render` and `synth-null` with
the same options; "exact" = every sample equal (residual −240 dB). Mirage 0/3 unless
noted; note 60, velocity 90–100, 48 kHz.

| case | result |
|---|---|
| Silver, Circuit: defaults; velocities 1, 64, 127; notes 36, 60, 84 | exact |
| Silver, Circuit: Tune 0/1, Decay 0/1, Color 0/1, Saturate 0/1, Noise 0/0.5/1, Seed 0/1, Dissonance 0/1, Velocity −1/−0.5/0, Gate on (note-off at 50 ms, retrigger) | exact |
| Silver, Circuit: three overlapping notes (retrigger), two notes in one block | exact |
| 44.1 kHz, 96 kHz, 0.37 s preroll | exact |
| Ten Years Off 1/5, Mirage 0/3 with Decay 0.8 | exact |
| Lightspeed 2/0 | −53 dB: the project automates or modulates Decay (with Decay 1 it is exact) |
| Memory: defaults; Sources B–F; Distress 0.5/1; Decay 0/1; Color 0.5/1; Strike 0/1; Tune 0/1; Gate; Velocity 0; notes 36, 84; velocity 1; three notes (voice ends and restarts: second `rand()` draw); 44.1/96 kHz | exact, except samples below 1.2e-38 in the last decay tail (Maschine flushes denormals in some 256-blocks and not in others) |
| Memory, Color exactly 0 (or below ≈ 2e-5) | −136 dB (cause not found) |
| CC 1, CC 120, pitch bend during a note | no effect in Maschine; exact |
| Hybrid | DECKARD's own design, not null-tested |
