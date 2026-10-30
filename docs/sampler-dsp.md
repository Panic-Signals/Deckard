# Maschine's Sampler: DSP specification

A clean-room description of how Maschine 3 plays its built-in Sampler
(`NI::MASCOMMON::DATA::Sampler` → `NI::MASCOMMON::RT::SamplerProcessor` →
`NI::MASCOMMON::SAMPLER::Voice`), written from decompiled code in
`out/decomp/{sampler,sp,sp2,sp3}` (Ghidra, arm64 slice) and constants read from the binary.
Lookup tables are in `crates/engine/src/sampler_tables.rs`. Addresses are arm64 VM
addresses.

**Confidence.** **[exact]**: formula and constants read directly from the code.
**[inferred]**: reconstructed from partial code, or a reasonable reading of it.
**[unknown]**: not recovered.

Notation:
- `lerp(T, v)`: table lookup used throughout. `i = min(v·127, 127)`, then interpolate
  linearly between `T[⌊i⌋]` and `T[⌊i⌋+1]`; the last entry is not interpolated.
  `sampler_tables::lookup`.
- `sq(x) = |x|·x`: signed square. Almost every modulation amount is squared this way.
- `sr`: the host sample rate.

---

## 1. Parameters as stored

Field names are those of `crates/mxprj/src/types/module.rs`. Each parameter is created
by `TransactionObjectContainer::insert(label, default, min, max)` in
`Sampler::Sampler()` (0x10186b214), `AHDSREnvelopeObject::AHDSREnvelopeObject`,
`LFOObject::LFOObject` and `Zone::Zone()`. `connectParameters` (0x10186dd28) routes each
one to a `SamplerProcessor::process*Parameter`. **[exact]**

### 1.1 Sampler (module)

| field (mxprj) | label | type | default | range | processor (what it stores) |
|---|---|---|---|---|---|
| editor_tab | EditorTab | enum | 0 | Main, Zone | – (UI) |
| polyphony | Polyphony | enum | 4 ("8") | Legato, 1, 2, 4, 8, 16, 32, 64 | `processVoicesParameter`: index |
| glide | Glide | float | 0 | 0..1 | `processGlideParameter`: raw |
| pitchbend | Pitchbend | float | 0.5 | 0..1 | `processPitchbendRange`: `sq(v)` (§6.3) |
| mode | Mode | enum | 0 | Standard, MP60, S1200, S1200 L, S1200 LM, S1200 HM, S1200 H | `processEngineModeParameter` (§7) |
| envelope_type | Type | enum (`SamplerEnvelopeMode`) | 0 | One-shot, AHD, ADSR | `processSamplerModeParameter`; all voices fade out over 10 ms on change |
| reverse | Reverse | bool | false | | `processReverseParameter` |
| tune | Tune | float | 0 | −1..1 | `processTuneParameter`: `v·3` (octaves, so ±36 semitones) |
| start | Start | float | 0 | 0..1 | `processStartParameter`: `sq(v)` (§6.2) |
| (not saved) | Scroll X / Scroll Y / Zoom X / Vertical Zone Editor Zoom | | | | UI only |
| comp | Comp | float | 0 | 0..1 | `processCompressorParameter` → ShaperDrum (§8.2) |
| drive | Drive | float | 0 | 0..1 | `processDriveParameter` → ShaperEnhanced (§8.3) |
| sample_rate | SR | float | 0 | 0..1 | `processLofiSampleRateParameter` (§8.4) |
| bits | Bits | float | 0 | 0..1 | `processLofiBitRateParameter` (§8.4) |
| filter | Filter | enum | 0 | Off, LP2, BP2, HP2, EQ | `processFilterModeParameter` (§8.5) |
| cutoff | Cutoff | float | 0.5 | 0..1 | `processFilterCutoffParameter` |
| resonance | Resonance | float | 0 | 0..1 | `processFilterResonanceParameter` |
| eq_frequency | Frequency | float | 0.5 | 0..1 | `processEQFrequencyParameter` |
| eq_width | Width | float | 0.25 | 0..1 | `processEQBandWidthParameter` |
| eq_gain | Gain | float | 0 | −1..1 | `processEQGainParameter` |
| velocity_start | (Velocity →) Start | float | 0 | −1..1 | `processVelocityToStartParameter`: `sq` |
| velocity_decay | (Velocity →) Decay | float | 0 | −1..1 | `processVelocityToDecayParameter`: `sq` |
| velocity_cutoff | (Velocity →) Cutoff | float | 0 | −1..1 | `processVelocityToCutoffParameter`: `sq` |
| velocity_volume | (Velocity →) Volume | float | **1** | −1..1 | `processVelocityToVolumeParameter`: `sq` |
| modwheel_start | (Mod wheel →) Start | float | 0 | −1..1 | `processModWheelToStartParameter`: `sq` |
| modwheel_cutoff | (Mod wheel →) Cutoff | float | 0 | −1..1 | `processModWheelToCutoffParameter`: `sq` |
| modwheel_lfo | (Mod wheel →) LFO Depth | float | 0 | 0..1 | `processModWheelToLFOParameter`: `sq` |
| modwheel_pan | (Mod wheel →) Pan | float | 0 | −1..1 | `processModWheelToPanParameter`: `sq` |
| volume_envelope | (AHDSREnvelopeObject) | | | | amp envelope (§5) |
| mod_envelope | (AHDSREnvelopeObject) | | | | modulation envelope (§5.3) |
| lfo | (LFOObject) | | | | LFO (§4) |

The eight bipolar amounts (named `mod_*` here before this spec) are, in order: **velocity → start, decay, cutoff, volume**
and **mod wheel → start, cutoff, LFO depth, pan**. The page names
(`Sampler::s_aPageNames`) are "Voice Settings / Engine", "Pitch / Envelope",
"FX / Filter", "Modulation" and "Velocity / Modwheel". **[exact]**

### 1.2 AHDSREnvelopeObject (volume_envelope and mod_envelope)

| field | default | range | volume env | mod env |
|---|---|---|---|---|
| attack | 0 | 0..1 | `processVolumeAttackParameter` | `processModulationAttackParameter` |
| hold | 0.5 | 0..1 | …Hold | …Hold |
| decay | 0.5 | 0..1 | …Decay | …Decay |
| sustain | 1 | 0..1 | …Sustain (`v³`) | …Sustain (`v³`) |
| release | 0.5 | 0..1 | …Release | …Release |
| pitch | 0 | −1..1, bipolar | (unused) | `processEnvAmountPitchParameter`: `sq` |
| cutoff | 0 | −1..1 | (unused) | `processEnvAmountCutoffParameter`: `sq` |
| drive | 0 | −1..1 | (unused) | `processEnvAmountDriveParameter`: `sq` |
| pan | 0 | −1..1, PanningConverter | (unused) | `processEnvAmountPanParameter`: `sq` |

Only the modulation envelope's amounts are connected. **[exact]**

### 1.3 LFOObject

| field (order) | label | default | range | processor |
|---|---|---|---|---|
| 0 | Type | 0 | Sine, Tri, Rect, Saw, Random | `processLfoTypeParameter` |
| 1 | Speed | 0.5 | 0..1 | `processLfoSpeedParameter` (§4) |
| 2 | Phase | 0 | −0.5..0.5 | `processLfoPhaseParameter`: raw (cycles) |
| 3 | Sync | 0 | Free, Retrig, Lock | `processLfoSyncParameter` |
| 4..7 | Pitch, Cutoff, Drive, Pan | 0 | −1..1 | `processLfoAmount*Parameter`: `sq` |

### 1.4 Zone

| field | label | default | range |
|---|---|---|---|
| loop_active | Active | false | |
| x_fade | X-Fade | 0 | 0..100000 frames |
| attack | Attack | 0 | frames (zone fade-in, §6.5) |
| decay | Decay | 0 | frames (zone fade-out) |
| root_key | Root Key | 60 | 0..127 |
| key_range, velocity_range | Key Range / Velocity Range | | inclusive ranges |
| gain | Gain | 0.75 | 0..1 (0.75 = unity, §6.6) |
| pan | Pan | 0 | −1..1 |
| tune | Tune | 0 | −36..36 semitones |
| play_range, loop_range | | | sample frames (inclusive) |

---

## 2. Signal flow of one voice

`Voice::processAudio` (0x10185f504) renders in blocks of ≤128 frames. **[exact]** (order)

```
sample stream (AudioStreamCreator: play range, loop, x-fade, zone fades, reverse)
  → engine resampler (Standard / MPC60 / SP1200), pitch from renderPitchModulation (§6.1)
  → processVoiceEffects:
       ShaperDrum   ("Comp",  §8.2)
       ShaperEnhanced ("Drive", §8.3; modulated by env·drive + lfo·drive)
       SamplerLofi  (§8.4)
       Filter2PoleLP / BandPass / Filter2PoleHP (only the selected one is active; cutoff modulated, §8.5)
       BiquadFilter (EQ peak band, when the filter mode is EQ)
  → processVolumeEnvelope (amp AHDSR, §5)
  → processPanModulation (zone gain/pan, velocity→volume, pan modulation, §6.6)
```

Per-voice modulation sources: the mod envelope `env` (0..1), the LFO `lfo` (−1..1), the
velocity `vel = velocity/127` and the mod wheel `mw` (0..1). A note-on with velocity 0
counts as velocity 64; the scaling is `·0.007874016`. The LFO is scaled by the mod wheel
everywhere it is used:

```
lfoScale = (1 − mwToLfo) + mw · mwToLfo        // mwToLfo = sq(modwheel_lfo)
```

So with Mod wheel → LFO Depth at 0 the LFO acts at full depth. At 1 its depth follows the
mod wheel. **[exact]**

---

## 3. Time constants (envelopes)

The processor turns knob values into frame counts (`SamplerProcessor::process*`):

```
attack_ms  = exp(lerp(ATTACK_CURVE, v)        · 8.922792 + ln 2) − 2
hold_ms    = exp(lerp(HOLD_CURVE, v)          · 8.922792 + ln 2) − 2
decay_ms   = exp(lerp(DECAY_RELEASE_CURVE, v) · 9.433563 + ln 2) − 2      // decay and release
frames     = (long)(sr · ms · 0.001)
sustain    = v³
```

At v = 1 these give an attack of about 7.7 s, a hold of about 7.7 s and a decay or
release of about 12.4 s. At v = 0 attack and hold are 0 ms and decay/release are about
2.9 ms (the table starts at 0.095). **[exact]**

**Velocity → decay** (amp envelope only, at trigger time, when the amount `a = sq(param)`
is non-zero):

```
t   = lerp(VELOCITY_DECAY_CURVE, decay_knob)          // table 0x104d122c4 (same values as DECAY_RELEASE_CURVE)
u   = a ≥ 0 ? 1 − vel : vel
ms  = exp((1 − |a|·u) · t · 8.922792 + ln 2) − 2      // NB: factor 8.922792, not 9.433563
decay_frames = sr · 0.001 · ms
```

**[exact]**

---

## 4. LFO

`NI::MASCOMMON::LFO`: the phase is in cycles (0..1). `inc = freq / sr` per frame.
Rendered per block. **[exact]**

| type | value at phase φ |
|---|---|
| 0 Sine | `cos(2πφ)` (polynomial cosine) |
| 1 Tri | with `φ' = frac(φ + 0.5)`: `φ' ≤ 0.5 ? 4φ' − 1 : 1 − 4(φ' − 0.5)` |
| 2 Rect | `φ < 0.5 ? +1 : −1` |
| 3 Saw | `1 − 2φ` (falling) |
| 4 Random | sample and hold, a new value at each phase wrap: `seed = seed·0xBB38435 + 0x3619636B`; value = the float with bits `(seed & 0x7FFFFF) \| 0x40000000`, minus 3 (uniform on [−1, 1)) |

**Speed.** The LFO helper (`SyncableLFOHelperBase`) uses a table of `{num, den}` note
lengths in whole notes: `SamplerProcessor::s_arrRateDivisions` (0x10dc16e48, n = 12):
16, 8, 4, 2, 1, 1/2, 3/8, 1/4, 3/16, 1/8, 1/16, 1/32 (`sampler_tables::LFO_RATE_DIVISIONS`).
**[exact]**
- Free-running Hz: `x = speed·(n−1)`; `i = clamp(round(x − 0.5), 0, n−2)`; `f = x − i`;
  `len = f·num[i+1]/den[i+1] + (1−f)·num[i]/den[i]`; `Hz = 1/(2·len)`. That runs from
  1/32 Hz (speed 0) through 1 Hz (speed ≈ 0.45, len 1/2) to 16 Hz (speed 1). The default
  0.5 sits between 1/2 and 3/8 (about 1.1 Hz).
- Sync division: `j = clamp(round(speed·(n−1)), 0, n−1)`; `ratio = den[j]/num[j] · 0.25`
  (cycles per beat).

**Sync modes**, applied when a voice starts (`triggerVoiceWithZoneAndMidiEvent`):
- **0 Free.** Frequency = the free-running Hz. Phase = the processor's free-running
  global LFO phase + Phase + the offset of the event in the block.
- **1 Retrig.** Frequency = `BPM/60 · ratio`. Phase = the Phase parameter (restarts on
  each note).
- **2 Lock.** Frequency = `BPM/60 · ratio`. Phase =
  `Phase + ratio · fmod(songPositionBeats, 1/ratio)` + the in-block offset (locked to the
  song).

**Destinations:** pitch (§6.1), cutoff (§8.5), drive (§8.3), pan (§6.6). Each is
`sq(amount) · lfoScale · lfo`.

---

## 5. Envelopes

### 5.1 Amp envelope modes

`provideEnvelopeParameterToVoice` (§3 gives the frame counts). Segments are 0 attack,
1 hold, 2 decay, 3 sustain, 4 release. **[exact]**

| Type | enabled | attack | hold | decay | release | sustain level |
|---|---|---|---|---|---|---|
| 0 One-shot | no (the envelope is off; the sample plays to its end, or loops) | | | | | |
| 1 AHD | yes, gated off | A | H | D | 0 | 0 (decays to silence, ignores note-off) |
| 2 ADSR | yes, gated | A | 0 | D | R | `sustain³` |

The start level is 1.0 (field `+0x10c`). A voice is freed once its envelope has finished
and the level is below 0.0001. In One-shot mode a looped zone (`loop_active`) is not
looped: the voice flag that allows looping is set only when the envelope is enabled
(`Voice+0x171 = env.enabled && zone.loop_active`). **[exact]**

### 5.2 Segment shapes

`AHDSREnvelope` (0x10185dc58 render, 0x10185daac start, 0x10185d9a8 `EnvelopeCurve::setCurving`).
Each segment runs from level `s` to level `e` in `N` frames, along an exponential set
by a curvature `C ∈ [−1, 1]`. **[exact]**

```
c     = exp(L + (H − L)·(1 − |C|))      // L = −9.903487552536127 (ln 5e−5), H = 3.2188758248682006 (ln 25)
ratio = C > 0 ? c/(c+1) : (c+1)/c
m     = ratio^(1/N)                      // per-frame multiplier (double precision)
x0    = C > 0 ? c + 1 : c
d     = C > 0 ? −(e − s) : (e − s)
off   = s − d·x0
frame k:  y = off + d·x;  x *= m          // x starts at x0, so y(0) = s and y(N) = e
```

Closed form, `t = k/N`:
- `C > 0`: `y = s + (e − s)·(c + 1)·(1 − (c/(c+1))^t)`. Fast at first (an exponential
  approach).
- `C ≤ 0`: `y = s + (e − s)·c·(((c+1)/c)^t − 1)`. Slow at first.

Default curvatures (`resetToDefault`):

| segment | C | c | shape |
|---|---|---|---|
| 0 attack, 0 → 1 | −0.332 | 0.3205 | slow start, e.g. 33 % of the way at half time |
| 1 hold, 1 → 1 | 0 | 25 | (flat) |
| 2 decay, 1 → sustain | +0.4427 (0x3ee2a84c) | 0.0750 | fast drop, 79 % done at half time |
| 3 sustain | 0 | | held until note-off (ADSR) |
| 4 release, level → 0 | +0.4427 | 0.0750 | as decay |

**Note-off** (`Voice::releaseWithCountDown`). Only for gated envelopes (ADSR), and for
both the amp and the mod envelope: the release segment restarts from the **current
level** and goes to 0 over the release frames, with the release curve. A voice is
removed when its amp envelope has run out and its level is below 0.0001.

**Fades.** `Voice::fadeOutWithCountDown(ms)` is a linear fade to 0 over `ms`. It is used
with 10 ms for voice stealing (`VoiceManager::fadeOutOldestVoice`: `sr·0.01` frames), for
CC 120 (all sound off) and when Type changes.

### 5.3 Modulation envelope

The same AHDSR as the amp envelope. It is always enabled, uses the mod-envelope knobs,
and is gated (sustain, release) only when Type is ADSR; otherwise it is AHD with a
sustain of 0. Its output (0..1) is scaled by `sq(pitch|cutoff|drive|pan amount)`.
**[exact]**

---

## 6. Voice start, pitch, sample playback

### 6.1 Pitch

At trigger (`triggerVoiceWithZoneAndMidiEvent`):

```
voice.tune      = processor.tune            // = Tune·3 octaves
voice.keyOffset = (note − zone.root_key) / 12   // octaves
voice.pbAmount  = pitchbendWheel · sq(Pitchbend)   // fields 0x24c · 0x250
```

`renderPitchCurve` and `renderPitchModulation` (0x101860650, 0x101860510) build a
per-frame pitch in **octaves**. All amounts are in octaves too. **[exact]**

```
octaves = zone.tune/12 + Tune·3 + (note − root)/12 + glide(t)
        + sq(lfoPitch)·lfoScale·lfo + sq(envPitch)·env + bend·sq(Pitchbend)
ratio   = 2^octaves · (zoneSampleRate / sr)          // ni::ae pow2; setSampleRateRatio
```

So the mod envelope or LFO pitch amount at ±1 is ±1 octave.

**Glide** (Legato only; `Glide`): `glideFrames = round(Glide² · sr)` (Glide 1 = 1 s).
The glide offset `glide(t)` ramps linearly from its current value to the target
`(lastPressedNote − firstNote)/12` over `glideFrames`. The target is updated on every
note-on and note-off while another note is held.

### 6.2 Sample start

```
start = clamp(sq(Start) + vel·sq(velToStart) + mw·sq(mwToStart), 0, 1)     // fraction of the play range
Voice::startWithCountDown(start, offsetInBlock)
```

**[exact]**

### 6.3 Pitch bend

The wheel position is `bend = value14/8192 − 1` (−1..1; `processEventBuffers`). The bend
in octaves is `bend · sq(Pitchbend)`, so the range is `Pitchbend²` octaves: the default
0.5 gives ±0.25 octave (±3 semitones), and 1.0 gives ±1 octave. **[exact]**

**Mod wheel:** CC 1 (with 14-bit MSB/LSB merging) gives `mw = value/127`, used by every
voice at once. **Sustain pedal:** CC 64 ≥ 64 is down. While it is down, a note-off only
marks its voices as sustained; releasing the pedal releases them. **CC 120** fades out
all voices over 10 ms and resets glide.

### 6.4 Reverse, play range, loop

`Voice+0x170 = !reverse`. The `AudioStreamCreator` gets `setPlayRange(play_range)` and
`setLoopRange(loop_range)` (inclusive, converted to start/length). `loop_active`
enables looping, but only when the amp envelope is enabled (§5.1). While the voice
plays, the loop range follows edits to the zone, clamped into the play range
(`processAudio` head). **[exact]**

### 6.5 Zone fades

`setZoneLevelEnvelopeAttack(zone.attack)` and `setZoneLevelEnvelopeDecay(zone.decay)`
(frames) apply `ZoneLevelEnvelope::applyToAudioBlock`: a **linear** fade-in over the first
`attack` frames of the play range and a linear fade-out over the last `decay` frames.
**[exact]**

**Loop crossfade** (`AudioStreamCreator::process`). With looping on and `x_fade = X > 0`,
the last `X' = min(X, loopLength, framesAvailableBeforeLoopStart)` frames before the loop
end are mixed with the `X'` frames that precede the loop start. The tail fades 1 → 0
and the pre-start material fades 0 → 1 (`AudioBlockAmplifier`, linear), so the jump back
to the loop start is seamless. Reverse playback reads the same ranges backward.
**[inferred]** from the control flow; the amplitude ramps are **[exact]**.

### 6.6 Output gain and pan

`processPanModulation`. **[exact]**

```
g      = 2.3948107·gain⁵ + 0.767467·gain²          // zone gain; 0.75 → 1.0
a      = sq(velToVolume)                            // default 1
u      = a ≥ 0 ? 1 − vel : vel
g     *= (1 − |a|·u)³                               // velocity sensitivity; a = 1 gives g·vel³
L     *= (1 − 0.70711·p − 0.29289·p²) · g           // p = zone pan
R     *= (1 + 0.70711·p − 0.29289·p²) · g
panMod = sq(lfoPan)·lfoScale·lfo + sq(envPan)·env + sq(mwToPan)·mw
panMod: one-pole smoothed per frame, y = 0.9·y + 0.1·x, then ni::ae::AlignedAudioBlock::applyPanModulation
```

`panModulationVec_Generic` (0x102557640) is a linear balance law with
`p = panMod[n]`: right gain `= p > 0 ? 1 : 1 + p`, left gain `= p < 0 ? 1 : 1 − p`
(the left side is by symmetry). It is not clamped. This is applied after the static zone
pan above.

---

## 7. Engine modes (Mode) and resampling

**Standard** (`EngineStandard::process`, 0x10185ce34). **[exact]**
- The position is a 16.16 fixed-point accumulator. Per output frame,
  `inc = round(ratio·65536)`, where ratio = the pitch ratio × the zone/host rate ratio.
- Interpolation is an 8-point polynomial (`ResamplerImpl::scaleFreqMonoX` /
  `scaleFreqStereoX`, 0x101861…). `t = 1 − frac/65536`; the output is
  `Σ w_k(t)·x[n+k]` for k = −3..4, with quintic weights (Horner form, highest power
  first, each ending in `·t`):

  | tap | polynomial in t |
  |---|---|
  | x[n−3] | ((((−0.020964433t + 0.075699046)t − 0.061509572)t + 0.007359662)t − 0.00058470765)t |
  | x[n−2] | ((((0.14338401t − 0.38360944)t + 0.18016043)t + 0.03248506)t + 0.027579939)t |
  | x[n−1] | ((((−0.3720349t + 0.90449077)t − 0.08861077)t − 0.25935876)t − 0.18448636)t |
  | x[n]   | ((((0.6183112t − 1.51671)t − 0.032761108)t + 1.1424075)t + 0.78875244)t |
  | x[n+1] | ((((−0.6183112t + 1.574846)t − 0.0835108)t − 1.873024)t + 0)t + 1  (no trailing ·t: this is the 1 at t = 0) |
  | x[n+2] | ((((0.3720349t − 0.95568377)t + 0.1909967)t + 1.1814046)t − 0.78875244)t |
  | x[n+3] | ((((−0.14338401t + 0.3333106)t − 0.07956275)t − 0.2948502)t + 0.18448636)t |
  | x[n+4] | ((((0.020964433t − 0.02912311)t − 0.031642303)t + 0.06738092)t − 0.027579939)t |

  At t = 1 (frac 0) the output is x[n]; at t → 0 it is x[n+1]. The tap order is
  reconstructed from the shift-register update (newest sample in lane 0 of the first
  register). It is confirmed numerically: the weights sum to 1 for every t, and at
  t = 0.5 they are symmetric (x[n] = x[n+1] = 0.6004, x[n−1] = x[n+2] = −0.1233, …).

**MP60** (`EngineMPC60`). **[inferred]**: only the structure and constants were read.
- The pitch ratio is held for each 32-frame chunk and clamped at 1.49831.
- The input is quantized to 12 bits: `trunc(x·2048)/2048`.
- 16-sample ring buffer interpolation, and an `AAFilterMPC60` IIR anti-alias filter with
  coefficients `fAAa`/`fAAb` (`sampler_tables::MPC60_AA_A/B`).
- Output gain 0.8637621. There is a separate code path when the host rate is not
  44100 (`processInternal<true>`).

**S1200** (`EngineSP1200`, modes 2..6). **[inferred]**: structure only.
- Built around an internal clock of 104000 Hz (constants 104000, 724.07733 and
  1/724.077), with 12-bit quantization.
- A 24-coefficient interpolation table is inlined (values 0.63..1.41).
- An `AAOutputFilterSP1200` 6-pole anti-alias filter (`cfAA2b0..b6`, `cfAA2a1..a6` →
  `SP1200_AA_B/A`).
- For modes 3..6, a per-channel output filter `OutputFilterSP1200::m_sCoeffCh{3..6}Out_{a,b}`
  (`SP1200_CH*_OUT_*`) and a gain constant, as below.

`Voice::setEngineMode`: 0 Standard (`EngineStandard`), 1 MP60 (`EngineMPC60` with the
`AAFilterMPC60` anti-alias filter), 2..6 SP1200 (`EngineSP1200`). For 3..6 an output
filter is selected: `OutputFilterSP1200::m_sCoeffCh{3,4,5,6}Out_{a,b}` and a gain
constant (`0x3a150e48`, `0x3ab3a772`, `0x3b4886a5`, `0x3bab2eb5`). These are channel
models: S1200 L, LM, HM, H (by option order: S1200 = 2 (no output filter), S1200 L = 3,
S1200 LM = 4, S1200 HM = 5, S1200 H = 6). Details in §10. **[exact]** for the mapping.

---

## 8. Effects

All of these run per voice, in the order of §2.

### 8.1 Parameter smoothing

The LP, BP and HP cutoff ramps are recomputed every 32 frames. Ramp length is
`max(1, round(sr·0.02/32))·32` frames (about 20 ms). `Voice::setSampleRate`. **[exact]**

### 8.2 Comp (ShaperDrum)

A table-driven compressor and shaper. **[exact]**
- Target amount `A = clamp(comp, 0, 1)·50`, re-read every 32 frames. Bypassed when
  `comp ≤ 0.0001`.
- Amount smoothing: `cur += (A − cur)·c50`. When `|step| < 1e−4` it jumps to A.
- Per frame: `peak = max_ch |x|`; `env = peak ≥ env ? peak : env + cRel·(peak − env) + 1e−12`;
  `lev = min(env + 1e−5, 1)`.
- `curve(a, lev)`: bilinear interpolation in `ShaperDrum::tableCompressCurves`
  (0x10dc0f898, 52 curves × 128 points, row stride 128). Row = `⌊cur⌋` (and `+1`),
  column = `lev·127`.
- Gain target `= curve / lev`. Smoothed: `g += cGain·(target − g) + 1e−12`.
- Output `x · g · lerp(tableCompressGainCorrection, cur)` (0x10dc16098, 129 entries,
  indexed by `cur`).
- Coefficients: `c(ms) = 1 − 2^(−log2(100) / (sr·ms·0.001 − 1))`, i.e. 99 % in `ms`
  milliseconds. `c50` uses 50 ms, `cGain` 2.67874 ms, `cRel` 25.4764 ms.

### 8.3 Drive (ShaperEnhanced)

**[exact]**
- Every 32 frames: `d = clamp(drive + mod[firstFrame], 0, 1)` with
  `mod = sq(envDrive)·env + sq(lfoDrive)·lfoScale·lfo`. Target `T = d·127`, smoothed
  with c50. Bypassed when `d ≤ 0.0001`.
- `amount = lerp(tableDriveAmount, cur)` (0x10dc1629c),
  `mix = lerp(tableDriveMix, cur)` (0x10dc1649c),
  `corr = lerp(tableDriveGainCorrection, cur)` (0x10dc1669c). Each table has 128
  entries, indexed directly by `cur` (0..127).
- Per sample: `u = clamp(x·amount, −8, 8)`; `y = x + mix·(corr·u·(1 − |u|/16) − x)`.

### 8.4 Lo-fi (SamplerLofi)

`Voice::setLofiParametersNormalized(bits, sr)`. **[exact]**
- `b = lerp(LOFI_BITS_CURVE, bits)`. When `b < 0.99999`, bit reduction is on:
  `levels = 2^max(1, 16·b)` (fast `pow2`). Per sample: `y = trunc(x·levels) / levels`
  (truncation toward zero).
- `r = lerp(LOFI_RATE_CURVE, sr_knob)`. When `r < 0.99999`, rate reduction is on:
  `target = max(50, r·sr)`, `hold = max(1, sr/target)`.
  Per frame: `if c ≥ hold { c −= hold; held = x } else { c += 1 }; y = held`.
  (The decompiled loop takes the new sample when `c ≥ hold`, otherwise outputs the held
  sample and adds 1.)
- Display: rate "Off" when r ≥ 0.99999, else `max(50, r·sr)` in Hz/kHz.

### 8.5 Filter

Mode 1 LP2, 2 BP2, 3 HP2, 4 EQ, 0 Off. Every filter is processed, but each one bypasses
itself unless it is the selected mode (`setFilterMode`). **[exact]**

**Cutoff input:** `c = lerp(CUTOFF_CURVE, clamp(cutoff, 0, 1))`, then per 32-frame chunk:

```
c' = c + mod[firstFrameOfChunk]
mod = sq(envCutoff)·env + sq(lfoCutoff)·lfoScale·lfo + sq(velToCutoff)·vel + sq(mwToCutoff)·mw
```

`c'` is ramped over about 20 ms (§8.1), and the coefficients are recomputed from the
ramped value every 32 frames.

**Cutoff to normalized frequency** (shared by the three filters):

```
x  = clamp(c', 0, 1) · 6.15
e  = exp(x) − 1                       // 9th-order Taylor series in the code
f  = min(max(e/388.67682, 0), 1) + 0.002
f  = clamp(f · (44100/sr) · 0.998004, 0, 1)
```

Display Hz: `f · 22050 · k` with k = 0.99 (LP), 0.82 (BP), 0.92² (HP).

**Biquad form** (`BiquadFilter::processInternal`, direct form I with `+1e−28` against
denormals). Coefficient struct `Coef2Pole {b1, b2, a1, a2, b0, 1}`:

```
y[n] = b0·x[n] + b1·x[n−1] + b2·x[n−2] + a1·y[n−1] + a2·y[n−2]      // a's have feedback sign
```

**Prototype.** A 2-pole Butterworth low pass at 1 rad/sample, P = `{b1 = 0.28820994,
b2 = 0.14410497, a1 = 0.67749089, a2 = −0.25391075, b0 = 0.14410497}`. The `Voice`
constructor writes it once into `BiquadFilter::s_aUnityCoef` (0x10dc0f880): 16 bytes
from 0x104d0ba80, plus `b0 = 0x3e139041`. The filters are frequency transforms of this
prototype (Constantinides). The resonance `r` gives `q = clamp(1 − (1 − r)², 0, 1)`.

**LP2** (`Filter2PoleLP::calculateFilterCoefficientsInternal`):

```
w  = f · π/2 · 0.99 · 0.9987809
k  = sin(0.5 − w) / sin(0.5 + w)          // LP→LP all-pass coefficient, θc = 1 rad
A1 = P.a1                                  // (q·0 + 1)·a1
A2 = P.a2 · (1 + 2.687·q)                  // resonance raises the pole radius
d  = 1 / ((k·A1 + 1) − k²·A2)
a1 = (A1 + 2k + k²·A1 − 2k·A2) · d
a2 = (A2 − A1·k − k²) · d
B0 = (P.b0 − k·P.b1 + k²·P.b2) · d
B1 = (P.b1 − 2k·P.b0 + k²·P.b1 − 2k·P.b2) · d
B2 = (P.b2 − k·P.b1 + k²·P.b0) · d
g  = ((1 − q)·0.55 + 0.45) · (1 − (a1 + a2)) / (B0 + B1 + B2)    // unity DC gain × resonance compensation
coef = {b1 = g·B1, b2 = g·B2, a1, a2, b0 = g·B0, 1}
```

(Checked term by term against `out/decomp/sp2/0018.c`. `P.b1` is `s_aUnityCoef[0]`, `P.b2` is `[1]`, `P.a1` is `[2]`, `P.a2` is `[3]` and `P.b0` is `[4]`.)

**HP2** (`Filter2PoleHP`, 0x1018579c0): the same, with
`w = f·π/2·0.92·0.9971332` and the LP→HP transform
`k = −cos(0.5 + w)/cos(0.5 − w)` (polynomial cosine). Then
`a1 = −(A1 + 2k + k²A1 − 2kA2)·d`, `a2 = (A2 − A1k − k²)·d`, `b1` negated, and the gain
normalized at Nyquist: `g = ((1−q)·0.55 + 0.45)·((a1 − a2) + 1)/(B0 + B1 + B2)`.

**BP2** (`BandPass`, 0x1018562b0): two cascaded first-order sections built from a
first-order prototype at 1 rad (`BandPass::s_aUnityCoef` at 0x10dc0f860, written by the
Voice constructor: `b1 = 0.26186`, `a1 = 0.29340`, `b0 = 0.26186`; from `0x3e861406` and
`0x3e963993`). No resonance.

```
w   = f · π/2 · 0.82
k1  = sin(0.5 − w·0.9987809)/sin(0.5 + w·0.9987809)        // LP section
d1  = 1/(k1·a1 + 1);  B0 = d1(b0 − k1·b1);  B1 = d1(b1 − k1·b0);  A = d1(a1 + k1)
g1  = (1 − A)/(B0 + B1)          → section 1 = {b1: g1·B1, a1: A, b0: g1·B0}
k2  = −0.99 · cos(0.5 + w·0.9971332)/cos(0.5 − w·0.9971332)  // HP section
d2  = 1/(k2·a1 + 1);  C0 = d2(b0 − k2·b1);  C1 = d2(b1 − k2·b0);  A' = d2(k2 + a1)
g2  = (1 − A')/(C0 + C1)         → section 2 = {b1: −g2·C1, a1: −A', b0: g2·C0}
```

**EQ** (`Voice::setEQParametersNormalized(freq, width, gain)`): an RBJ peaking biquad.

```
Hz    = 20 · 1000^freq           // exp(freq·6.9077554 + 2.9957323), 20 Hz..20 kHz (round trip through fast log/exp)
w0    = min(Hz/sr, 0.49) · 2π
bw    = 2^(width·8/3 + 1/3)      // 1.26 .. 8
alpha = sin(w0) · (bw − 1/bw) / 2
dB    = gain · 18                // gain −1..1 → ±18 dB
A     = 2^(dB/6)                 // fast pow2 ≈ 10^(dB/20)
a0    = 1 + alpha/A
b0 = (1 + alpha·A)/a0;  b1 = −2cos(w0)/a0;  b2 = (1 − alpha·A)/a0
a1 = 2cos(w0)/a0 (feedback sign);  a2 = (alpha/A − 1)/a0
```

The EQ is not modulated. Note that with `A = 2^(dB/6)` (≈ 10^(dB/20), not RBJ's
10^(dB/40)) the band's gain at its centre is `A²`: Gain at ±1 (shown as ±18 dB) boosts or
cuts about ±36 dB there. That is what `Voice::setEQParametersNormalized` (0x101860f2c)
computes; it has not been checked by ear against Maschine.

---

## 9. Note handling

`processEventBuffers` → `processNoteOn` (0x1018856c8) / `processNoteOff`. **[exact]**

**Note-on.** Velocity is the event's velocity; 0 counts as 64 (MIDI 1 "note on, vel 0"
is a note-off elsewhere). For 32-bit MIDI 2 events velocity is `v >> 25`.
1. **Legato** (Polyphony index 0): `Glide::onNotePress(note)` and every voice gets the
   new glide target and time. If another note was already held, **no new voice starts**:
   the sounding voice glides. Otherwise all voices fade out (10 ms) and step 2 runs.
2. **Zone selection:** every zone with `key_range.lo ≤ note ≤ key_range.hi` and
   `velocity_range.lo ≤ vel ≤ velocity_range.hi` starts a voice. Overlapping zones
   layer. The voices of one note share one `VoiceLimitCounter` item (they count as one
   note for polyphony).
3. **Voice limit:** for Polyphony "2".."64" (index 2..7), while the number of notes
   sounding is above the limit (2, 4, 8, 16, 32, 64), the oldest note fades out over
   10 ms. For "Legato" and "1", the limit is 1 (`while count ≥ 2`).
4. **Trigger** (`triggerVoiceWithZoneAndMidiEvent`): set the pitch (§6.1), the envelopes
   (§5, including velocity → decay), the LFO frequency and phase by sync mode (§4), the
   mod-envelope amounts, the zone fades, the engine mode and filter, EQ, drive, comp and
   lo-fi settings. Then start at `start` (§6.2) at the event's frame offset in the block.

**Note-off.** `Glide::onNoteRelease(note)`.
- With the sustain pedal down: mark the matching voices as sustained.
- **Legato:** retarget the glide to the latest note still held. If none is held,
  release every voice.
- **Other modes:** release (§5.2) the voices whose note and note ID/channel match.

**Velocity:** `vel = velocity/127` (·0.007874016). It drives start, decay, cutoff and
volume, each through `sq(amount)` and the `u = a ≥ 0 ? 1 − vel : vel` form (cutoff and
start are additive: `vel·sq(amount)`).

---

## 10. Open points

- **MP60 / S1200 engines.** The per-sample algorithms (ring buffers, interpolation,
  where quantization happens, how the output filters are applied) were not fully
  reconstructed. The constants and coefficient tables are complete in
  `sampler_tables.rs`; the code is `out/decomp/sp/0079.c`–`0090.c`.
  `AAOutputFilterSP1200::process` timed out in Ghidra.
- **Bit-crush versus rate-hold order** inside `SamplerLofi::processInternal<true,true>`
  is not certain. The bit reduction is assumed to come first.
- **The ShaperDrum row stride** (`+0x20`) is assumed to be 128 (the table is 52 × 128).
- **Display strings** were not all transcribed. Cutoff: §8.5. Lo-fi rate: §8.4.
  LFO rate: free Hz, or `num/den` in sync modes.
- **Pitch-bend range display** (semitones) was not checked against Maschine's UI.
- The mxprj field names follow this spec: `velocity_*`, `modwheel_*`, `lfo`
  (`pan_amount` its fourth amount).

## 11. Implementation order (suggested)

1. Parameter mapping (§1, §3) and the AHDSR with curves (§5). This makes envelopes and
   times match.
2. Velocity → volume and the zone gain/pan law (§6.6). It changes loudness a lot: the
   default velocity curve is `vel³`.
3. Filter LP2/BP2/HP2/EQ (§8.5), then drive, comp and lo-fi (§8.2–8.4).
4. The modulation envelope and LFO (§4, §5.3), mod wheel and pitch bend (§6.3).
5. Polyphony, legato and glide (§9), loop crossfade (§6.5), the 8-point interpolator (§7).
6. The MP60 and S1200 engines (§7, §10).
