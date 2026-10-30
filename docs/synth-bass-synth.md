# Maschine's Bass Synth: parameters and DSP

A clean-room description of Maschine 3's internal **Bass Synth** instrument. The class
chain is `NI::MASCHINE::DATA::BassSynth` → `RT::DrumSynthWrapper<DSP::BassSynthProcessor>`
→ `DSP::BassSynthCore`. The core uses four Reaktor-Core-style kernels
(`DSP::UTIL::RX::RxOsc4Wave` ×2, `RxEnvAHD` and `RxDrive`), the nonlinear filter
`DSP::TSKFilterNLA<float,1>`, `MASCOMMON::SAMPLER::Glide` (the held-key list) and an
`NI::AB::SampleRateConverter`.

Sources:
- Ghidra output in `out/decomp/bass/` (DATA, processor, wrapper, core) and
  `out/decomp/bass2/` (kernels, Glide); `INDEX.tsv` maps files to functions.
- arm64 disassembly in `out/decomp/bass-core.s` (BassSynthCore), `bass-rx-osc.s`,
  `bass-rx-envdrive.s`, `bass-tsk.s` and `bass-glide.s`.

Where Ghidra and the assembly disagree, this spec follows the assembly. Ghidra drops f64
steps, and it hides fused multiply-adds and the `frintx` rounding. Addresses are arm64 VM
addresses in `analysis/maschine_arm64`.

The DECKARD port is `crates/engine/src/synth/bass_synth.rs`, with its kernels in
`bass_synth/{osc,env,filter,drive,src,rx}.rs`. It nulls **bit-exactly** against Maschine
(§6).

**Confidence.** **[exact]**: read from code or assembly and confirmed by the null tests.
**[inferred]**: reconstructed from partial evidence.

---

## 1. Parameters

### 1.1 Stored parameters (constructor `BassSynth::BassSynth`, 0x1019d7e5c) **[exact]**

Module name "Bass Synth". Each parameter is created by
`TransactionObjectContainer::insert(label, default, min, max)`.

| obj off | mxprj field | label | section | default | range | RT idx | display |
|---|---|---|---|---|---|---|---|
| 0xa8 | `shape` | **Shape** | "Osc1" | 0.66 | 0..1 | 0 | default float format `%.2f` |
| 0xb0 | `octave` | **Octave** | – | −2 | −3..3 (int, bipolar) | 1 | `%d` |
| 0xb8 | `shape_2` | **Shape 2** | "Osc2" | 0.66 | 0..1 | 2 | `%.2f` |
| 0xc0 | `octave_2` | **Octave 2** | – | −2 | −3..3 (int, bipolar) | 3 | `%d` |
| 0xc8 | `detune` | **Detune** | – | 0 | 0..1 | 4 | `%.2f` |
| 0xd0 | `osc_mix` | **Osc Mix** | "Osc Mix" | 0 | 0..1 | 5 | `%.2f` |
| 0xe8 | `cutoff` | **Cutoff** | "Filter" | 0.2 | 0..1 | 6 | `hzToString(8.1758·2^((10+128v)/12))`, 14.6 Hz..24.6 kHz (default ≈ 64 Hz) |
| 0xf0 | `resonance` | **Resonance** | – | 0.75 | 0..1 | 7 | `ScaleNormalizedToPercentOneDecimal(1.2·v)`: 0.0..120.0 % |
| 0xd8 | `mod_amount` | **Mod. Amt.** | "Envelope" | 0.5 | 0..1 | 8 | `v·100`, one decimal, `%` |
| 0xe0 | `decay` | **Decay** | – | 0.5 | 0..1 | 9 | `v·100`, one decimal, `%` |
| 0x108 | `drive` | **Drive** | "Tone" | 0 | −1..1 (bipolar) | 10 | `v·100`, one decimal, `%` |
| 0xf8 | `glide` | **Glide** | "Note" | 0 | 0..1 (int) | 11 | `%d` (a two-step knob) |
| 0x100 | `glide_time` | **Glide Time** | "Note" | 0.0909091 | 0..1 | 12 | `FloatLinearConverter(10, 1000, "%.f ms")`: 10 + 990·v ms (default 100 ms) |

"RT idx" is the `BassSynthProcessor` parameter index, assigned in `connectParameters`
order. The float display format with no converter is
`MaschineNumericParameter<float>::getDefaultFormat()` = `"%.2f"`; for ints it is `"%d"`.

### 1.2 Pages and knob layout (`getParameterByPosition` 0x1019d93ec, `s_pageNames`) **[exact]**

There are two pages, "Main" and "Advanced". The section heading is shown where the section
changes.

| page | slot 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |
|---|---|---|---|---|---|---|---|---|
| Main | SHAPE (Osc1) | OCTAVE | SHAPE 2 (Osc2) | OCTAVE 2 | DETUNE | OSC MIX (Osc Mix) | CUTOFF (Filter) | RESONANCE |
| Advanced | MOD. AMT. (Envelope) | DECAY | DRIVE (Tone) | GLIDE TIME (Note) | GLIDE | – | – | – |

There are no enum selectors. Glide is an integer 0/1. The QML editor
`Modules/BassSynthEditor.qml` (title "BASS SYNTH") lays the controls out differently:
- Row 1: Octave, Shape (waveform knob), Osc Mix, Shape 2 (waveform knob), Octave 2, Detune.
- Row 2: Mod. Amt., Decay, Cutoff, Resonance, Glide, Glide Time, Drive.

### 1.3 Serialization (`serialize<iarchive>` 0x1019d7a94) **[exact]**

Class version 2. The load order is:
- Module, a8, b0.
- Only for v > 1: b8, c0, c8, d0.
- e8, f0, d8, e0, 108.
- f8: an int (v ≥ 1), or for v0 a `bool` "Glide" that is loaded into it (mxprj
  `legacy_glide`).
- 100.

Projects saved at v1 lack `shape_2`/`octave_2`/`detune`/`osc_mix`, which keep their
defaults (Maschine resaves them at v2 with the defaults).

A new Bass Synth starts with the defaults in the table above.

### 1.4 Value → processor **[exact]**

Each `connectParameters` lambda sends `(f32)((1/(max − min))_f64 · (f64)(v − min))` to
`BassSynthProcessor::setParameter(idx, ·)`:
- (v − min) is computed in `f32`; for the ints it is an integer subtraction.
- The values are sent only on change.
- The processor stores them, and maps idx 0..12 to core indices 0,1,2,3,4,5,7,8,9,10,11,12,13
  (core index 6 is unused).

`BassSynthCore::setParameter` (0x10195a5b0):

| core idx | effect |
|---|---|
| 0 / 2 | osc 1/2 Shape: the input +0x08, and starts the oscillator's 100 ms shape glide (+0x24 = 1, +0x28 = 0, +0x30 = current +0x2c, +0x34 = v − current) |
| 1 / 3 | octave = `round_half_away(fma(v, 6, −3))` (f32), written to osc +0x14 and +0x4c |
| 4 | detune d = v (semitones subtracted from osc 2's pitch at the next note) |
| 5 | osc mix m = v |
| 7 | cutoff pitch = `(f32)fma((f64)clamp01(v), 128, 10)` → ramp 0; forces a filter retune (`+0xec = −1`); re-aims ramp 1 at the stored resonance (`+0xf0 = −1`) |
| 8 | resonance r = v·1.2 → ramp 1, `+0xf0 = −1` |
| 9 | mod amount → ramp 2 |
| 10 | envelope D = `fma(v, 40, 50)` → `RxEnvAHD` +0x38, `processEventInD` |
| 11 | `setParamDrv` (§3.4) |
| 12 | glide g (0/1): both pitch glides' "on" and env +0x50 → `processEventInGlide` |
| 13 | glide time = `(f32)fma((f64)clamp01(v), 9.9f, 0.1f)` seconds·10 |

A ramp is one of five linear 20 ms glides (cutoff, resonance, mod amount, drive input gain,
drive output gain):
- Its length is `len = sr_os·0.001f·20` and `n = fcvtas(len)`.
- `aim(x)`: target = x; if cur ≠ x, then step = (x − cur)/len, count = n, and pending.
- Per sample, when |target − cur| ≤ 0, or when the count runs out (count < 2 before the
  decrement), it snaps if pending; otherwise cur += step.

---

## 2. Processor and wrapper **[exact]**

- **Startup.** `DSPCoreWrapper` constructs the processor (whose params are 0), calls
  `onSetSampleRate(44100)`, then the module's 13 values arrive, then
  `onSamplerateChanged(sr)` and `resume` (= `onReset`). With this order the stored values
  apply at once, with no shape glide.
- **Events.** `DrumSynthWrapper::process` splits the block at events. A note-on calls
  `setNoteOn(vel·0.007874016, key & 0x7f)`; the wrapper takes **only one note-on per
  timestamp**, and a velocity-0 note-on counts as a note-off. A note-off calls
  `setNoteOff(key)`. **CC 120** (All Sound Off) calls `onChoke`. Nothing else is read: pitch
  bend, the mod wheel and sustain do not reach the instrument (null-tested, §6). Automation
  events call `setParameter`.
- **Segments.** The audio runs in segments of at most 16 frames on a running counter that
  starts at construction. They are also cut at events and at 128-frame control ticks; the
  control tick is a no-op here.
- **`setNoteOn`** first re-sends all 13 parameters to the core (`updateGlobalCoreParameters`),
  then runs the core's `setNoteOn`:
  1. If vel ∉ [0,1] it does nothing.
  2. `Glide::onNotePress(key)`.
  3. If the voice is off, every pending ramp snaps.
  4. The `dirty` flag is set.
  5. Pitch glide 1 goes to `key`, pitch glide 2 to `key − d`.
  6. env +0x28 = 1, `processEventInNoteOn`; env +0x08 = vel, `processEventInNoteVel`.
  7. state = 1.
- **`setNoteOff`**: `Glide::onNoteRelease(key)`.
  - **If Glide is 0, nothing else happens**: the envelope is a one-shot attack–hold–decay
    that ignores note length.
  - With Glide on and keys still held, both pitch glides go to the most recently pressed
    held key (legato return).
  - With Glide on and no keys left: env +0x48 = 0, `processEventInNoteOff` (the decay starts).
- **`Glide`** (0x10185e6f0…) keeps the held keys, most recent first, with a 128-node free
  list.
- **`onChoke`** sets the last-envelope value to 0. If the state is 1, it becomes 2 (fading)
  with amp = 1.
- **`hasOutputSignal`** is `state ≠ 0`. The state only returns to 0 when a choke fade ends,
  so after its first note **the instrument runs continuously**: its oscillators free-run
  and later notes start from their phases. DECKARD's `voices()` mirrors this.
- **`processAudio`** (0x10195fe4c): if state = 0, it calls `onReset` once if `dirty` and
  outputs nothing. Otherwise it calls `processAudioImpl<os, ramps, first>`:
  - `os`: the rate is below 88 kHz.
  - `ramps`: some ramp is pending at the call's start (only then are the ramps ticked).
  - `first`: the first call after `onReset`. In that call the resonance is rechecked on
    every sample. Every later call rechecks it **only while ramp 1 is pending**, so a
    resonance glide's final snap does not retune the filter. This quirk is ported.

### 2.1 `onSetSampleRate(sr)` (0x10195a17c) and `onReset` (0x101959c1c) **[exact]**

`onSetSampleRate` refuses sr < 44100. Otherwise:
1. os = 2 if sr < 88000, else 1, and sr_os = os·sr.
2. The SRC is set up (os·sr → sr, 2 channels, quality 2).
3. Each kernel is cleared except its inputs, which are re-sent with event flags = 1.0, and
   its `endInit` runs.
4. The pitch glides restart at their targets.
5. The filter period is 1/sr_os (`setFreq` again if it changed).
6. The choke fade is `min(1, expf((1/sr_os)·(−0.6931472)/0.05))` (it halves every 50 ms).
7. The ramp lengths are set, then `onReset`.

`onReset`:
- clears the SRC and the filter states (+0x30..+0x44);
- forces a filter retune;
- restarts the oscillators, envelope and drive as above;
- sets the state and amp to 0, and snaps the pending ramps;
- sets **ramp 4** (drive output gain) to cur = target = 1.0 until the drive is next set.
  The next note-on re-sends it, and with the voice off it snaps.

Constructor constants:
- Envelope: A = 13, H = 30, Ax = 3, Dx = −10, VelS = 1. Its mode inputs +0x58 and +0x60 are 0.
- Drive: output gain 1, gain range 20 dB.
- Filter: saturator scales 4, 0.25, 3.4, 1/3.4 and α = 0.8.

---

## 3. DSP (`processAudioImpl`, per sample at sr_os) **[exact]**

1. Tick the five ramps (if this call ticks them).
2. Envelope `RxEnvAHD::processAudio`; e = its output (+0x68).
3. Pitch glides. tg = glide_time·0.1f. If active: d = max(tg·on, 1e-4f), then
   pos += 1/(d·sr_os) (at 1 it stops), v = pos·Δ + start, and if on > 0 the pitch = v.
   The glide is linear in semitones over 10..1000 ms; with Glide off the pitch jumps, but
   the interpolator still runs, so later glides start where it got to.
4. Oscillator 1 (pitch1) → a = out·0.4f. Oscillator 2 (pitch2 = key − d) → b.
   If m ≠ 0: a = fma(√(1−m), a, √m·(0.4f·b)), an equal-power mix.
5. **Filter cutoff.** Retune when the cutoff ramp or the mod ramp changed, or when
   mod ≠ 0 and e changed:
   - p = min(fma(e·mod, 128, cut), 138)
   - f = (f32)exp2(fma((f64)p, 1/12, 3.0313597)) (libm `exp2`), i.e. 8.1758·2^(p/12) Hz
   - then `TSKFilterNLA::setFreq(f)`.

   So the envelope sweeps the cutoff by up to 128 semitones times Mod. Amt.
6. **Resonance.** On a change (gated as in §2): k = r < 2 ? 2r : 4, then `setFreq` with the
   stored f. The coefficients are:
   - g = atan(min(π·f/sr_os, 1.50845)) (Reaktor's polynomial)
   - c24 = g(g+1)/fma(g, (2−k)+g, 1), c28 = g/(g+1), c2c = 1/g.
7. Filter input a·(drive input gain); `processInput`; y = its s2 state (+0x34).
8. Drive: input 0.7·y, `RxDrive::processAudio`, giving dout (+0x24).
9. amp = 0 (state 0), 1 (state 1), or for state 2 amp ·= fade, which ends at
   `(f64)amp < 1e-4` (state 0).
10. out = ((e·dout)·amp)·(drive output gain), the same on both channels.

Then the converter halves the rate at os = 2 (§3.5).

### 3.1 Oscillator `RxOsc4Wave` (0x1018b6c8c, endInit 0x1018b69ec)

Inputs and output:
- Inputs: +0x00 sr, +0x08 Shape (flag +0x0c), +0x10 pitch (MIDI), +0x14 octave (flag +0x18;
  +0x4c is the copy in use). Output at +0x1c.
- Frequency = 8.1758·2^((pitch + 12·oct)/12) via the shared octave table.
- The increment is the mean of this and the previous frequency over sr, clamped to ±0.5.
- The phase (+0x44, in ±0.5) runs downwards and wraps by ±1 in f64.

There are four waveforms from one phase, each one sample late:
- sine: the folded phase through a degree-7 odd polynomial;
- triangle: 2|φ| − 0.5;
- saw: 2φ;
- square: ±0.5, always 50 %.

Each discontinuity arms a two-sample polynomial step correction:
- saw/square: a degree-7 polynomial;
- triangle corners: a degree-9 polynomial times the increment.

Shape morphs linearly in thirds: ≤ 0.3333 sine→triangle, ≤ 0.6666 triangle→saw, above that
saw→square. Each is a crossfade at 3·(s − region start). The region is recomputed only while
the 100 ms shape glide runs.

One quirk is kept: going from the saw→square region straight back to the first region never
clears the square flag. The kernel has no fused operations, but some steps are f64.

### 3.2 Envelope `RxEnvAHD`

A segment generator. Its stage table at +0x120 has 6 doubles per stage: start, target, time
(s), curvature (±10), flags, next.

| stage | target | time | curve | flags |
|---|---|---|---|---|
| 0 attack | peak = 1 + (vel−1)·VelS = vel | A | Ax = 3 | input +0x60 |
| 1 hold | current level | H | 0 | 4 \| 2·Glide (sustain while Glide is on) |
| 2 decay | 0 | D | Dx = −10 | 2 |

Times:
- time = 2^((x−60)·0.166096) ≈ 10^((x−60)/20) s, capped at 11184.8 s.
- A = 13 → 4.5 ms; H = 30 → 31.6 ms; D = 50 + 40·Decay → 0.32 s (Decay 0) .. 31.6 s (Decay 1).
- n = max(1, rne(t·sr)) samples.

Per sample: out = (f32)y, then y = y·mul + add + 1e-100, with:
- mul = 2^(c/n) (a double exp2 from 6-digit tables and a degree-5 polynomial);
- add = (mul−1)/(2^c−1)·(target − y·2^c);
- below |c/n| ≈ 2^−26 the step is linear instead.

Behaviour on notes and edits:
- Note-on restarts the attack from the current level (no reset to 0).
- With Glide on, the hold sustains until note-off, which starts the decay.
- Changing D while it runs re-times the segment proportionally.
- The output is exactly 0 only when y < the smallest `f32`.

### 3.3 Filter `TSKFilterNLA<float,1>` (setFreq 0x1019c3e54, processInput 0x1019c410c)

A nonlinear two-pole Sallen-Key low-pass with two trapezoidal integrators (s1 at +0x30,
s2 at +0x34 = output).

The saturator is scale·tanh(arg/scale′):
- linear below |arg| = 0.001 (compared in f64);
- ±scale from |arg| = 10;
- otherwise e = 2^(arg·2.88539), using `frinti` and a fused cubic, and (e−1)/(e+1)·scale.

Each sample:
1. A feedback estimate (an f64 fused α-blend).
2. The input minus k·feedback is saturated.
3. A predictor pass through both stages.
4. A refined feedback.
5. The state updates use `g.mul_add(…)`.

13 `f32` and 2 `f64` operations are fused; all others are rounded separately.

### 3.4 Drive `RxDrive` and `setParamDrv` (0x10195a888)

`setParamDrv(v)`:
- x = fma(v, 2, −1) is the drive in −1..1.
- RxDrive's amount = max(x, 0), with its own 100 ms glide.
- Input gain (ramp 3) = 10^((x < 0 ? −20x : −0)·0.05) (libm `__exp10`), i.e. up to +20 dB
  for negative drive.
- Output gain (ramp 4) = 10^(((x ≥ 0) ? −7 : −10)·min(|x|,1)·0.05).

RxDrive (no fused operations):
- A 30 Hz one-pole splits off the lows; a 2800 Hz one-pole splits the rest.
- Only the 30..2800 Hz band is driven: d = band·10^(amount·20/20).
- Shaper: d − d³/3 for |d| ≤ 0.05; tanh via the table for |d| ≤ 20; ±1 beyond.
- Output = (lows + highs + shaped)·output gain. The filter states carry a 1e-18 offset.

### 3.5 Converter

At os = 2 the converter is `SampleRateConverter` quality 2 at integer ratio 2, the same as
the Distortion's (docs/fx-distortion.md §5):
- every other input enters an 8-deep delay and emits (d + fir)·½;
- the inputs between feed a 16-tap windowed-sinc phase at ½.

At 88 kHz and up there is no converter (os = 1).

### 3.6 Tables

All the tables are computed by code or are small constants. No NI data is embedded beyond
6-digit decimal constants:
- The octave table `2^(i/1021 − 0.5)` to 6 digits, shared with the Chorus.
- RxEnvAHD's `2^(i−128)` (257 entries) and `2^((k−8)/16)` (17 entries), as 6-digit decimals.
- Two entries of a further envelope table (0 and 0x3ab91dee).

---

## 4. DECKARD implementation notes

- **Kernel state layout.** The kernels keep Maschine's state layout (word arrays addressed
  by byte offset, `rx::Mem`). They were verified word for word against Maschine's own
  functions, which were called from the VST3 binary in a scratch harness.
- **`Synth::set`.** It applies only changed values, as Maschine's parameters do. The first
  call applies them like a project load: values, then the real rate, then a reset.
- **Event timing.** One note-on is taken per `process` call (per timestamp). Each event
  starts a new segment, and segments also follow the wrapper's 16-frame grid. This matters
  only at the end of a choke fade: an ended voice is reset (cutting the converter's tail)
  at the next segment start.
- **`control`.** It handles CC 120 as a choke. `stop()` is the same choke.

## 5. Open points

- **Parameter modulation.** Maschine applies a sound's per-parameter **modulation offset**
  (the mxprj `MaschineParameter` modulation flag/amount) to the value the instrument
  receives, even without the transport running. In *Orange Ten* 1/0 the cutoff carries
  flag = 1 and amount −0.27, so the cutoff sent is clamp(0.15 − 0.27) = 0. DECKARD's
  engine passes the stored values only.
  - The Bass Synth itself nulls exactly once given the modulated value (checked with the
    harness).
  - This belongs to the engine/doc layer for every module, not to this file.
- **Choke tail.** At the end of a choke fade Maschine went silent one 16-frame segment
  earlier than the wrapper model. This leaves 32 differing samples at about −160 dBFS
  (residual −172 dB).
- **Flush to zero.** In very long tails (Decay 0, after ~2.8 s) Maschine flushes values
  near 1e-38 to zero. DECKARD does not, which leaves a residual of −755 dB.

## 6. Null tests

`maschine-ref make … 0 [fields]` + `render --preroll 0` against `synth-null` with the same
options. Note lists use `T:N:V:D`. N =
`0:36:100:0.6,0.7:48:64:0.3,1.0:60:127:0.8,1.9:30:1:0.5` (3 s). Sources were resaved by
Maschine (`roundtrip`) so that all v2 fields exist. All tests are at 48 kHz unless noted.

| case | source | settings | residual |
|---|---|---|---|
| project values, one note | Exosphere 3/0 | – (44.1 / 48 / 96 kHz) | bit-exact ×3 |
| project values, N | Jakobi Woodnine 11/0 | 44.1 / 48 / 96 kHz | bit-exact ×3 |
| project values, N | Orange Ten 1/0 | cutoff modulated (§5) | −9.8 dB (modulation; exact with the modulated value) |
| defaults, N | Exosphere | all defaults | bit-exact |
| each of shape, shape_2, detune, osc_mix, cutoff, resonance, mod_amount, decay, glide_time at 0 / 0.5 / 1 | Exosphere, osc_mix 0.5 | N | bit-exact ×26; decay 0: −755 dB (§5) |
| octave, octave_2 at −3 / 0 / 3 | Exosphere | N | bit-exact ×6 |
| drive −1 / −0.5 / 0.5 / 1 | Exosphere | N | bit-exact ×4 |
| legato glide (4 overlapping notes) | Exosphere, glide 1 | time 0.3 at 48k/44.1k, 1 at 96k, 0 | bit-exact ×4 |
| velocity 1 / 64 / 127 | Exosphere | key 40 | bit-exact ×3 |
| key 100 (octave 3), key 12 (octave −3) | Exosphere | – | bit-exact ×2 |
| CC 120 during a note | Exosphere | – | −172 dB (§5) |
| CC 120, then notes again | Exosphere | – | bit-exact |
| CC 123, pitch bend 0/16383, mod wheel 127, sustain 127 | Exosphere | – | bit-exact (none reach the synth) |
| 88.2 kHz, 192 kHz | Exosphere | N | bit-exact ×2 |

Harness against Maschine's own `BassSynthProcessor`, driven like the wrapper:
- 6,600 random scenes (all parameters, 44.1/48/88.2/96/192 kHz, overlapping notes,
  note-offs, chokes, mid-note edits, DECKARD block sizes 1..4096) and 60 hand-made ones:
  all bit-exact.
- Each kernel was checked word for word against Maschine's function from live snapshots
  (oscillator, every envelope event, filter, drive).
