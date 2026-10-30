# Maschine's Burst Echo (Perform FX mode 2): parameters and DSP

A clean-room description of Maschine 3's **Burst Echo** Perform FX
(`NI::MASCHINE::DATA::PerformanceFX` → `NI::MASCHINE::RT::PerformanceFXProcessor` →
`NI::DSP::JamBurstEcho` → `NI::DSP::JamBurstEchoCore`, with `JamBurstEchoCore::DelayUnit`,
`NI::DSP::DelayLineVariableSpeed`, `JamBurstEchoCore::Filters`, `NI::DSP::Limiter` and
`NI::DSP::SmoothingToolbox<9>`). DECKARD's implementation is
`crates/engine/src/fx/perform/burst.rs`; it nulls bit-exactly against Maschine (§5).

Written from Ghidra output in `out/decomp/perform-burst/` (`INDEX.tsv`, plus `b2/` Limiter,
SmoothingToolbox and Eigen's `psincos_float`, `b3/` DelayLineVariableSpeed, `b4/`
`DATA::PerformanceFX::connectParameters` and the display converters) and arm64
disassembly in `out/perform-burst/asm/` and `out/perform-burst/core_process.s`. Where the
two disagree (operand order, which operations are fused), this spec follows the assembly.
Addresses are arm64 VM addresses in `analysis/maschine_arm64`; table constants were read
from the binary.

**Confidence.** **[exact]**: read from code or assembly and confirmed by the null tests.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation: all arithmetic is `f32` unless marked `f64`. `fma(a,b,c)` is a fused a·b+c
(arm64 `fmadd`/`fmla`; `fmsub`/`fnmsub` are the fused forms with the obvious signs).
"Ramp" means `JamBurstEcho`'s linear 20 ms smoother (§2.2).

---

## 1. Parameters

### 1.1 Stored fields and core indices **[exact]**

`connectParameters` binds each stored field to `PerformanceFXProcessor::set*Parameter(mode
2, index)`; enums arrive as floats. The strip goes to the *current* effect only (§4).

| index | field | arrives as | default |
|---|---|---|---|
| 0 | `touchstrip_pos` | `setTouchstripPosition(v)` | 0.5 |
| 1 | `touchstrip_on_off` | `setTouchstripOnOff(0/1)` | off |
| 2 | `burst_time` | float | 0.5 |
| 3 | `burst_feedback` | float | 0.95 |
| 4 | `burst_time_mode` | enum × 1/3 (`0x3eaaaaab`): Straight 0, Dotted 1, Triplet 2, Free 3 | Dotted |
| 5 | `burst_filter` | float | 0.35 |
| 6 | `burst_wet_level` | float | 0.6 |
| 7 | `burst_ts_assign` | enum: Feedback 0, Time 1 | Feedback |
| 8 | `burst_feed` | float | 0.3 |
| 9 | `burst_mix_mode` | enum: Mix 0, Replace 1, Wet only 2 | Mix |

Connection order: time, feedback, time mode, TS assign, filter, feed, mix, wet.

### 1.2 What each index does (`JamBurstEcho::setParameter` 0x102632558) **[exact]**

Every call stores the raw value; while suspended (§4) every ramp then lands at once.

- **0 strip:** with TS Assign = Feedback the feedback source becomes the strip:
  `fb = feedback(v, 1.2)`; with Time, `setTime(v)`.
- **1 engage:** `setTouchstripOnOffInternal` (§2.1).
- **2 time:** `setTime(v)` unless TS Assign = Time.
- **3 feedback:** with TS Assign = Time, `fb = feedback(v, 1.05)`.
- **4 time mode:** `mode = (int)(3v ± 0.5)`; core index 6 (synced) = mode ≠ Free; then
  `setTime(strip)` (Assign Time) or `setTime(raw time)`.
- **5 filter:** high pass `exp10f(fma(v, 1.6, 1))` Hz (10–398 Hz) to core 2; shelf
  `(f32)fma((f64)clamp01(v), −16000, 20000)` Hz to core 3.
- **6 wet:** wet ramp target `1.5·v`.
- **7 TS assign:** with Time, the feedback source becomes the stored feedback
  (`feedback(raw3, 1.05)`); with Feedback, `setTime(raw time)` then `feedback(strip, 1.2)`;
  with Time, `setTime(strip)`; then the dry-mute length.
- **8 feed (burst length):** `len = (f32)(exp10((f64)(f32)fma((f64)clamp01(v), 25, 37) ·
  0.05) · 0.001)` s (70.8 ms … 1.26 s); `len = 0` (endless) when `v > 0.99`. If engaged,
  the burst timer is (re)armed, and an endless burst opens the gate ramp to 1. Then the
  dry-mute length and the mix ramps.
- **9 mix:** `mix = (int)(v + 0.5)`; replace ramp → (mix = 1); dry ramp → (mix ≠ 2); dry-mute
  length.

`feedback(v, k) = fma(|v|, −0.8, 1.8) · v · k` (`0xbf4ccccd`, `0x3fe66666`). Released, it is
capped at 0.72 (`0x3f3851ec`); engaged, the strip can reach 1.2 and the knob 1.05.

### 1.3 Echo time (`setTime` 0x1026336e4) **[exact]**

- **Free:** `k = Assign Time ? 1 : −1`; `x = fma(−|v|, k, k+1)·v`; `d = clamp01((f64)x)`;
  ms = `fma(d, 760, 40)` (40–800 ms), or `fma(d, −454, 480)` with Assign Time (strip at 0
  is the longest).
- **Synced:** index range (lo, hi) = Straight (1, 8), Dotted (21, 28), Triplet (11, 18);
  with Assign Time (2, 7), (22, 27), (12, 17) and `v → 1 − v`. `idx = lo +
  (int)(min(v, 0.99) · (hi − lo))`. ms = `mult · (1000 / ((bpm/60) · 2^(base − idx)))` in
  f64, (base, mult) = Straight (5, 1), Dotted (25, 1.5), Triplet (15, 2/3). While ms >
  30 000 the index steps down (stopping at 1 from 1, 11 or 21). Clamped to 0–30 000.
  So Straight runs 1/64 to 2 bars, the default Dotted 0.5 is a dotted 1/8 (375 ms at 120).
- The ms go to core index 5 (§3.1); then the **dry-mute length**: the time in s, kept
  within [0.07 s, burst length] when the burst is finite (`calculateDryMuteLength`).
- **Tempo:** `onBPMChanged(bpm)` stores the tempo and, if synced, re-sends the synced ms to
  core 5 (it does not recompute the dry-mute length). The processor calls it every host
  block with the host tempo (an f32).

---

## 2. `JamBurstEcho` (0x102633b20)

### 2.1 Engaging (`setTouchstripOnOffInternal` 0x102632df8) **[exact]**

- **Released:** dry-mute ramp → 1, burst timer stops, gate ramp → 0, timers cleared.
- **Engaged** (unless already engaged and running): burst timer runs if the burst is
  finite, gate ramp → 1, both timers restart from 0, the dry-mute timer runs.
- Then the feedback is re-sent with the engaged/released cap (§1.2) and the dry-mute length
  recomputed.

### 2.2 Ramps **[exact]**

Five linear ramps (gate, dry, dry mute, wet, replace): length `sr·0.001·20` samples
(float, and its int), threshold 0.001. Setting a target different from the value starts a
step of `(target − value)/len` with the counter at `len`. Each sample: if |target − value|
> 0.001, the counter decrements and, if it was ≥ 2, the value steps; otherwise (or when
the counter runs out) an active ramp lands on its target.

### 2.3 Per sample, in blocks of up to 512 **[exact]**

1. Gate ramp ticks. If the burst timer runs: `t = (f32)(1/sr (f64) + t)`; when `len < t`
   it stops and, for a finite burst, the gate ramps to 0.
2. The core runs on `gate · input` (§3).
3. Dry, dry-mute, wet and replace ramps tick. If the dry-mute timer runs and passes the
   dry-mute length, the dry-mute ramp goes to 0 and the timer stops.
4. `d = dry · fma(replace, mute − 1, 1)`; `out = fma(wet, core_out, d · in)` per channel.

So **Mix** keeps the dry signal, **Replace** fades it out after the first repeat's time
(it returns over 20 ms on release), **Wet only** drops it.

---

## 3. `JamBurstEchoCore` (0x10262262c)

Parameters: 2 high pass Hz, 3 shelf Hz, 4 feedback, 5 time ms, 6 synced, 7 width (0.5,
set once by `onInit`; at 0.5 the M/S stage is the identity). Indices 0, 1 are stored and
unused.

### 3.1 Parameter lanes (`SmoothingToolbox<9>`) **[exact]**

Lanes 4 high pass, 5 shelf, 6 feedback, 7 time. A change sets the pending value and flags
a smooth; at the next control tick (every 128 samples, the processor's block) the glide
starts: start = current, delta = target − current (snapped if |delta| < the current
value's ulp), counter = `steps = max(1, (int)(sr·0.02/128 + 0.5))`. Each sample `t += 1`,
`current = start + delta·(t·rate)`, `rate = 1/(steps·128)`; when the counter runs out the
lanes land. While suspended, values land at once. **Synced** time and the tempo update
write lane 7 directly (no glide). The feedback goes straight in while suspended.

The time then passes through a **one-pole in f64**: if |lane7 − T| > 1e−4 ms, `T =
fma(1 − a, lane7, T·a)`, `a = exp(−5.298317366548035/(sr·150/1000))` (≈ 150 ms to 0.5 %);
within 1e−4 it lands. `setDelayTime` runs whenever T moved. **T starts at 0** on a fresh
core, so a newly loaded Burst Echo's tape spins up from its shortest time over about
0.4 s (Maschine's behaviour, reproduced).

### 3.2 Units and signal flow **[exact]**

Three tape units (§3.4), U0, U1, U2, each L/R. Per sample:

```
U0.write(in);  a = U0.read();  c = U2.read()
m = fb·c + a                (lane 6 feedback)
m = FiltersA(m)            (§3.5)
U1.write(m);   b = FiltersB(U1.read());  U2.write(b)
o = OutLP(b + m)           (§3.6)
M/S width (identity at 0.5), then Limiter(o/2)·2   (§3.7)
```

So the input reaches the output after one tape delay (U0) and again after two (U0 → U1),
and U2 feeds back into the U0 tap.

`setDelayTime`: `t = clamp(T, 0.1, 30 000)` ms; spread `s = min(0.3, 25/t·0.3)`; base =
`sr · (f64)((f32)t · 0.001f)` samples. Unit spreads: U0 `−s`, U1 `(f32)(2s)`, U2
`(f32)(−2s)`; a unit with spread `p` delays L by `base·fma(p, 0.5, 1)` and R by
`base·(1 − p/2)` (fused). So the three units straddle the time a little apart and alternate
sides.

### 3.3 Tape write (`DelayUnit::write` 0x102622f74) **[exact]**

Per channel:

1. **Soft clip with antiderivative anti-aliasing.** `x = gain·in`, `gain = 1/(1 + Σ aₖwₖ)`;
   `f(x) = x + Σ aₖ sin(wₖx)` for |x| ≤ 1, ±1 beyond; `F(x) = x²/2 − Σ bₖ cos(wₖx) − c₂`
   inside, `x + c₁` above 1, `c₁ − x` below −1; `y = (F(x) − F(x₋₁))/(x − x₋₁)`, or `f(clamp(x))`
   when |x − x₋₁| < 1e−4. `wₖ = kπ` (k = 1…4), `a = {0.50930, 0.12732, 0.024252, 0.0022736}`,
   `b = aₖ/wₖ` (hex in burst.rs); `c₂ = −Σbₖ`, `c₁ = (0.5 − Σ bₖ cos wₖ − c₂) − 1`. Sums are
   (0+2)+(1+3). sin/cos are Eigen's `psincos_float` (Cody–Waite reduction by π/2 with three
   constants, degree-4 cos and degree-3 sin polynomials, all fused).
2. **10 kHz one-pole:** `y = fma(c, state − y, y)`, `c = (f32)exp(−2π·10 000/sr)`.
3. **Speed-tracking low pass:** an SVF (k = 1.4) at `g = tan(π/2 · min(0.8·speed,
   0.9999999))` (set by the previous read).
4. **Write into the line** (§3.4).

### 3.4 The line (`DelayLineVariableSpeed`) **[exact]**

A ring of `nextpow2(sr)` floats. The write head moves `speed = sr/delay` slots per sample
(clamped to 50.5); the reader sits a fixed `sr` slots behind it, so the delay is
`delay` samples and changing the time changes the tape speed (pitch glides, as a tape
echo). Per sample the unit **reads** (linear interpolation at the reader, after running a
k = 1.4 SVF low pass at `g = tan(π/2 · clamp(1/|reader step|, 0.2, 1))` over up to 15 slots
the reader is about to cross), then **advances** the head (`pos += speed`), then the next
**write** fills the slots the head crossed with a parabola through the last three
written values at the last speeds (`h0 − t·(B − t·A)`, all fused). tan is the series
`x + x³/3 + 2x⁵/15` (`tan3`).

### 3.5 Filters A and B (`Filters::process` 0x102623864) **[exact]**

Per channel: an SVF **high shelf** at the shelf frequency, `g = √A · tan3(πf/sr)`
(f64 division), k = 1/0.6, `A = exp10f(−0.25)`: `y = fma(1 − A², low, fma(A², x, band·
A(1−A)/0.6))` (−10 dB above f), then an SVF **high pass** (k = 2) at the high-pass
frequency: `y − 2·band − low`.

### 3.6 Output low pass **[exact]**

An SVF low pass (k = 1.2) per channel at `g = tan3((f32)((f64)f·π·0.5))`, `f =
min(0.96, max(0.05, sr/delay_L)·1.2)` of U0 (its L / R delay): long echoes get darker.

### 3.7 Limiter (`NI::DSP::Limiter`, the core's first one) **[exact]**

Gains 0 dB in and out, no side chain, release 3 s. Peak `max(|x_L + 1e−12|, |x_R + 1e−12|)`
in dB (`ln·8.68589`); env₁ attack `1 − 1/(0.005 sr)`, release `1 − 1/(3 sr)`; env₂ attack
`1 + 1/(−15 sr)`, release `1 + 1/(−3 sr)`; level = max; gain = `ExpTable(min(0, −level))`;
output clamped to ±2.5. With the ×½ in front it only acts above 2.0 (+6 dBFS) in the
wet path. The core's second Limiter only receives a release setting and never runs.

---

## 4. The processor around it **[exact]**

- The processor calls the current effect's `processControlRate` every 128 samples and its
  `processAudio` on each block, engaged or not (released, the Burst Echo passes the dry
  signal and lets its echoes ring out with feedback ≤ 0.72).
- Parameters of a non-current effect are set while it is suspended (they land at once).
- **Becoming current** (`switchEffectMode`, at the start of a block): the strip position
  and engage state, `onBPMChanged(host tempo)`, `onReset` (lines, filters and output
  low pass cleared, timers cleared, then the engage logic re-run, so the burst and dry-mute
  timers start from the switch), `onResume`. A project loaded engaged therefore starts its
  burst at the first processed sample.
- The crossfade between effects on a mode change is the shared processor's (see the
  shared Perform FX notes); not part of this effect.

---

## 5. Null tests

Renders: `out/perform-burst/ref/*.mxprj` made with `maschine-ref make … --perform 2
field=value…`, rendered with `--preroll 0.05 --seconds 3` (`MASCHINE_REF_HOME` set to a
private home). Test: `fx::perform::burst::tests::nulls_against_maschine` runs `BurstEcho`
over 0.05 s of silence and the same input. Residual: RMS of L/R difference.

| case | settings | input | sr | tempo | residual |
|---|---|---|---|---|---|
| c1 | defaults, strip 0.5 | noise | 44.1k | 120 | −240 dB (bit-exact) |
| c2 | strip 0.8, feed endless, Replace | noise | 44.1k | 120 | −240 |
| c3 | Free, time 0.3, Wet only, filter 0.8 | noise | 44.1k | 120 | −240 |
| c4 | Assign Time, strip 0.3, feedback 0.6, Triplet, feed 0.7 | noise | 48k | 120 | −240 |
| c5 | strip 0.95, feed endless, feedback 1, filter 0 | noise | 44.1k | 120 | −240 |
| off | released | noise | 44.1k | 120 | −240 |
| c6 | Assign Time, strip 0.7, Triplet, feed endless | noise | 44.1k | 90 | −240 |
| c7 | Assign Time, Free, strip 0.2, Replace, feed 0.5, feedback 0.9 | sweep | 48k | 120 | −240 |
| c8 | strip 1.0 (fb 1.2), feed endless, wet 1, filter 0.1, input 0 dBFS (limiter active) | noise | 44.1k | 120 | −240 |
| c10 | Straight, time 0.05, feed endless, Replace | noise | 44.1k | 137.5 | −240 |
| c9 | strip 0.5, feed endless | noise | **96k** | 120 | **−20 dB** (not matched, §6) |

Also through the whole `PerformFx` with `fx-null` (c1, c5): −240 dB.

Not tested against Maschine: parameter changes mid-stream (strip movement, release,
re-engaging), since maschine-ref renders a fixed project; those paths follow the code.

## 6. Open points

1. **Above 48 kHz** **[known, not reproduced]**: the core runs at `sr / (int)(sr/44100)`
   through two `NI::AB::SampleRateConverter`s (quality 2) around it
   (`JamBurstEchoCore::onSetSampleRate`). DECKARD runs the core at the full rate; at 96 kHz
   the residual is −20 dB.
2. **Below 44.1 kHz** **[inferred]**: `JamBurstEcho::onSetSampleRate` ignores rates below
   44 100 Hz, so the effect keeps its constructor's settings (its sample period is then
   −1, so the burst and dry-mute timers never expire). burst.rs also ignores the rate there,
   but its period stays 0; untested.
3. **Host tempo:** DECKARD's `Ctx` carries a quarter note in samples, from which burst.rs
   recovers the f32 tempo; exact for the tested tempi.
