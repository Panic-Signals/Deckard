# Maschine's Perform FX Scratcher: parameters and DSP

A clean-room description of the **Scratcher** Perform FX (`PerformanceFX` mode 7):
`NI::MASCHINE::RT::PerformanceFXProcessor` → `NI::DSP::JamScratcher`, with an embedded
`NI::DSP::FreqShifterProcessor` (the "swirl") and two `NI::DSP::SineSaturator<5>`.
DECKARD's port is `crates/engine/src/fx/perform/scratcher.rs`.

Written from Ghidra output in `out/decomp/perform-scratcher/c/` (`INDEX.tsv`) and arm64
disassembly (`out/decomp/perform-scratcher/jam.s`, `proc.s` = `processAudio`,
`f_10261ee30.s` = `FreqShifterProcessor::setSampleRate`, `f_10266228c.s` =
`SineSaturator<5>()`, `f_1026678b8.s` = its Eigen sine sum). Where Ghidra and the
assembly disagree (it drops FMAs and NEON lanes and shows some `f64` fractions as `f32`),
this follows the assembly. Addresses are VM addresses in `analysis/maschine_arm64`.

**Confidence.** **[exact]** read from code; **[inferred]** reconstructed;
**[unknown]** not recovered. Arithmetic is `f32` unless marked `f64`. "fma(a,b,c)" is a
fused `a·b + c` (one rounding); everything not written as fma rounds after each step.

---

## 1. Parameters

Stored fields (offsets of `DATA::PerformanceFX`, Maschine 3 defaults) and the core
parameter index each feeds (`connectParameters`: `setFloatParameter(mode 7, index, v)`;
index 0/1 through `setTouchstripPosition`/`setTouchstripOnOff`) **[exact]**:

| field | default | core index | display |
|---|---|---|---|
| `touchstrip_pos` | 0.5 | 0 | — |
| `touchstrip_on_off` | off | 1 | — |
| `scratcher_wet_level` (0x220) | 0.5 | 2 | percent |
| `scratcher_delay_time` (0x208) | 0.55 | 3 | percent |
| `scratcher_feedback` (0x210) | 0.7 | 4 | `convertNormalized2ScratcherFeedback` |
| `scratcher_freq_shift` (0x218) | 0.5 | 5 | (2x−1)·(x<½ ? 100 : 50) Hz |

`setParameter(i, x)` stores the raw `x` (`param[i]`) and, for i ≥ 2, retargets smoothers
with `x' = clamp(x, 0, 1)` (NaN passes) **[exact]**:

- **2 (Wet Level):** `scr` → `f32(fma(x', f64(f32 −0.2), 1.0))`; `swl` → `f32(fma(x',
  f64(f32 0.7), 0.0))`. (The scratched signal falls to 80 %, the swirl rises to 70 %.)
- **3 (Delay Time):** `hi` → `f32(fma(x', 38.0, 12.0))`, `lo` → `f32(fma(x', 30.0,
  −10.0))` (dB of milliseconds, `f64`).
- **4 (Feedback):** `u = clamp((x − 0.1f) / 0.9f, 0, 1)`, `v = clamp(x / 0.1f, 0, 1)`
  (`f32`); `fb` → `f32(x ≥ 0.1f ? fma(u, f64(0.6f), f64(0.4f)) : fma(v, f64(0.4f), 0))
  · 0.99f`.
- **5 (Freq Shift):** `s = f32(fma(x', 2.0, −1.0))`; `hz` → `s · (s > 0 ? 50 : 100)`.
- **0 (position), 1 (on/off):** only mark the strip dirty (handled per sample, §4.1).

### 1.1 Smoothers **[exact]**

Linear ramps: `{cur, target, step, count, steps, dur, pending}`. Retargeting to `t`: if
`t ≠ cur`: `step = (t − cur) / dur` (`f32`), `count = steps`, `pending = 1`. Per sample:
if `|target − cur| > 0` and `count-- ≥ 2`: `cur += step`; otherwise, if pending: `cur =
target`, `step = 0`, `count = 0`, `pending = 0`. So a ramp takes `steps` samples
(`steps − 1` increments, then the target). Durations: `sr·0.001f·5` (position, `pos`)
and `sr·0.001f·20` (all others: `on`, `scr`, `swl`, `fb`, `lo`, `hi`, `hz`); `steps =
fcvtas(dur)` (round to nearest, ties away). Per-sample order: pos, on, scr, swl, fb, lo,
hi, hz.

## 2. Lifecycle **[exact]**

`PerformanceFXProcessor` constructs every core, calls `onInit` and
`onSetSampleRate(44100, 128)`; on a sample-rate change it calls `onSuspend`,
`onSetSampleRate(sr)`, then for the active mode `onReset`, `onResume`. Switching to a mode
sends it the strip position and on/off, then `onReset`, `onResume`. Parameter changes
reach the core immediately off the audio thread, sample-accurately on it.

- **Construction:** `dirty_on = dirty_pos = suspended = braking = 1`, `c8 = 0.5`,
  `prev_pos = 1.0`, everything else 0.
- **onInit:** `sr = 1`, `param[2] = 1`; `scr` → 0.8, `swl` → 0.7 (ramps with duration 0;
  §1.1 makes them snap at the next reset).
- **onSetSampleRate(sr):** ignored (returns false) unless `suspended` and `sr ≥ 44100`.
  So below 44.1 kHz every rate-derived value stays at its 44.1 kHz setting. Sets the ramp
  durations (and current `count = steps`), `isr = 1/f64(sr)`, the buffers (§3), the
  shifter (§5.2), the filters (§5.1), and the two `f64` low-pass SVFs `P` (needle) and `T`
  (swirl delay time): cutoff argument `w = isr · 6π`, §3.1.
- **onReset:** snap a pending `pos` ramp; clear all buffers (positions 0); shifter phase
  `re = 1, im = 0`; clear the Hilbert and filter states; `dirty_on = dirty_pos = 1`,
  `brake_reset = 1`, `prev_on = 0`; `on.cur = on.target = 0` (`step`, `count` 0; its
  pending flag untouched); snap pending `scr…hz`; `n_since = n_rec = 0`, `c0 = 1`,
  `prev_pos = 0`, `travel = 0`; prime `T` (§3.1, 20 iterations at `w = isr·2π·10⁴`, input
  `f64(0.001f)`); run the on/off update (§4.1); apply the position (immediately, since
  suspended); `resetBrakeFilters` (§4.3).
- **onResume:** `suspended = 0`. **onSuspend:** `suspended = 1`, snap a pending `pos`.

## 3. Buffers **[exact]**

Mono pairs (L, R), circular, write position = next slot:

- **A (record):** `floor_u(f64(sr)·0.8)` samples. **C (swirl delay):** same length.
- **B (dry):** `floor_u(f64(sr)·0.05 + 10)` samples.

`read4(buf, base)`: the four samples at `base−1 … base+2` (mod length), where `base = len +
pos + i`. **Cubic (4-point, Catmull-Rom):** with taps `y0…y3` (`f32`) and fraction `t`
(`f64`): `c3 = fma(y3, 0.5, fma(y2, −1.5, fma(y0, −0.5, y1·1.5)))`, `c2 = fma(y3, −0.5,
fma(y2, 2, fma(y1, −2.5, y0)))`, `c1 = fma(y0, −0.5, y2·0.5)` (all `f32`); result =
`f32(fma(f64(c1), t, fma(f64(c3), t³, t²·f64(c2))) + f64(y1))` with `t² = t·t`, `t³ = t·t²`.

### 3.1 The `f64` SVF

`g = fma(w⁵, 0.13333333333333333, fma(w³, 0x3fd555555555554f, w))` (`w² = w·w`, `w³ =
w·w²`, `w⁵ = w²·w³`; a tan series), `a1 = 1/fma(g, g + 2, 1)`, `a2 = g·a1`, `a3 = g·a2`.
Step with input `x`: `v3 = x − ic2`; `v1 = fma(a1, ic1, v3·a2)`; `v2 = ic2 + fma(a2, ic1,
v3·a3)`; `ic1 = 2·v1 − ic1`, `ic2 = 2·v2 − ic2` (fused); output `v2`. "Prime n at w towards
x": from zero state, `n` steps (§ coefficients from `w`), keeping the state.

## 4. Per sample **[exact]**

### 4.1 Strip

1. **Position dirty:** if `!suspended && prev_on ≠ 0`: when touched, retarget `pos` to
   `param[0]` (5 ms ramp) and clear the flag; when not touched leave it dirty. Otherwise
   `pos.cur = pos.target = param[0]` (no ramp) and clear.
2. **On/off dirty**, only when `on` has no ramp pending: `x = param[1]`. If `x > 0.5`:
   if not touched yet, `n_since = n_rec = 0` and copy all of B into the last `len B`
   samples of A (ending at A's write position); `on` → 1. Else, if touched: `on` → 0, and
   a pending `pos` ramp freezes where it is and the position is marked dirty. Then
   `touched = x > 0.5`, clear the flag.
3. Run the smoothers (§1.1). `on = on.cur`. If `on == 0 && prev_on > 0` and `pos` is
   pending, snap it. `prev_on = on`.

### 4.2 Record and dry

- **A** is written (L, R input), and `n_rec += 1`, when `on == 0 && !touched`, or
  otherwise while `fma(f64(sr), −0.055, f64(len A)) > n_rec` (so 0.745 s are taken from
  the touch, then it holds).
- **B** is always written; `dry = B[pos − 4]` (the input 3 samples ago).
- `mix = (1 − on)·dry` (both sides).

### 4.3 The needle (when `on > 0` or touched; else `brake_reset = 1`)

- `p = clamp(f64(pos.cur), 0, 1)`, `s = fma(p, 0.45, −0.05)`, `c8 = max(s, 0.05)`.
- Brake SVF `B` (state `ic1, ic2`): `w = (0.318 / c8)·π·isr`, coefficients §3.1.
- If `brake_reset`: `resetBrakeFilters`, `travel = 0`, `c0 = c8`, `prev_pos = −1`,
  `delta = 1`, `braking = 1`, `brake_reset = 0`, target `x = c8`, `jump = 0`.
  Else if `!braking`: `x = s`, `jump = 0`. Else if `delta ≥ f64(0.001f)` or `n_since < 3`:
  `x = c8`, `jump = 0`. Else `braking = 0`, `x = s`, `jump = (s ≠ c8)`.
- Step `B` with input 0 (§3.1); `travel = fma(isr, v2_B, travel)`; `frac = clamp(travel /
  c0, 0, 1)`.
- If `jump`: `n = fcvtzu(sr·0.001f)`; `P` state = 0, then primed `n` steps at `w = isr·2π·10⁴`
  towards `x`; `B` state = 0.
- Step `P` with input `x`; `pos_s = frac · v2_P`.
- If `pos_s > travel`: `lim = clamp(isr·f64(n_since) − travel, 1e-10, 1e10)`; `pos_s =
  fma(tanh((pos_s − travel)/lim), lim, travel)` (the needle cannot outrun real time).
- `pos_s ·= f64(sr)`; `delta = pos_s − prev_pos`, `prev_pos = pos_s`; if `jump`, `delta = 0`.
- Read A at `r = pos_s − f64(n_rec + 4)`: `i = fcvtzs(r) − (r ≤ 0 ? 1 : 0)`, `t = r − i`,
  cubic from `base = len + pos_A + i`. `gain = f32(min(|delta| / 0.2, 1))`.
  `mix = fma(on·cubic, gain, mix)` (each side). `n_since += 1`.
- **resetBrakeFilters:** `n = fcvtzu(sr·0.0005f)`; `B` state = 0 then primed `n` steps at
  `w = isr·2π·10⁴` towards 1; `P` state = 0 then primed `n` steps towards `c8`.

### 4.4 The swirl

- `shift.hz = hz.cur`; `(sin, cos) = sincos(f64(hz)·2π / f64(sr))`; phase step `(cos,
  sin)` (amplitude 1).
- `ms = f32(fma(clamp(f64(pos.cur),0,1), f64(hi − lo), f64(lo)))`; target `f32(exp10(f64(ms)
  ·0.05)·0.001)` seconds; step `T` (§3.1); `d = sr·f32(v2_T)`.
- Read C at `r = −f64(d)`: `i = fcvtzs(r) − (d ≥ 0 ? 1 : 0)`, `t = r − i`, cubic from
  `base = len + pos_C + i` → `wet` (L, R).
- `x = on · fma(wet, fb.cur, mix)` (each side), then the 15 kHz stage (§5.1 `F15`, output
  `o`), the Hilbert pair and shifter (§5.2), the saturator (§5.3), the 30 Hz stage (§5.1
  `F30`, output `u`), written to C.
- **Output:** `on·(mix·scr + swl·wet) + (1 − on)·dry` as `fma(1 − on, dry, on·(mix·scr +
  wet·swl))`.

## 5. Swirl parts

### 5.1 One-pole stages **[exact]**

`g = fma(x⁵, 0.13333334f, fma(x³, 0.33333334f, x))` with `x = (1/sr)·K` (`f32`; `K =
94.24778f` for 30 Hz, `47123.89f` for 15 kHz), `G = 1/(g + 1)`. Per sample from state `s`:
`u = x + G·fma(−g, x, −2s)` (as `u = x + G·(−2s − g·x)`, the `−g·x` fused), `o = fma(s,
2, g·u)`, `s = fma(g, u, s)`. `F15` passes `o`, `F30` passes `u`.

### 5.2 Hilbert pair and shifter **[exact]**

Four lanes `(L_A, L_B, R_A, R_B)` through seven first-order all-pass stages, each with an
8-slot ring and lane coefficients `k`: `old = ring[(i − D + 1) & 7]`, `t = k·(x − old)`,
`ring[i+1] = x + t`, `y = old + t` (i.e. `(k + z⁻ᴰ)/(1 + k z⁻ᴰ)`). `D = clamp(fcvtzs((sr +
1)/44100), ≤ 8)` (1 at 44.1/48 kHz); rings cleared when `D ≠ 1`. Coefficients (A lanes, B
lanes) = `n · recip(d)`, `recip` = `frecpe` and two `frecps` steps:

| stage | d (A, B) | n (A, B) |
|---|---|---|
| 1 | 0.901254535, 0.821706116 | 0.0291516185, 0.294312894 |
| 2 | 0.767939627, 0.735501528 | 0.473534584, 0.581661582 |
| 3 | 0.716687441, 0.704884887 | 0.644375205, 0.683717072 |
| 4 | 1.09874547, 1.17829394 | −0.629151642, −0.894312918 |
| 5 | 1.23206043, 1.26449847 | −1.07353461, −1.18166161 |
| 6 | 1.28331256, 1.29511511 | −1.24437523, −1.28371716 |
| 7 | 1 | −0.3 |

(Bit patterns in `scratcher.rs`.) The input lanes are `(o_L, o_L, o_R, o_R)`. Phase:
`re' = fma(cos, re, −(sin·im)) + 0`, `im' = fma(sin, re, cos·im)` (`f64`). Output `y_L =
f32(im')·A6_L + f32(re')·B7_L` (A from stage 6, B from stage 7), likewise R.

### 5.3 SineSaturator<5> **[exact]**

`a_k = k·π` (`f32`, k = 1…5); `c` solves `M c = (1/π, 0, 0, 0, 0)` with `M[j][k] =
(−1)ᵏ(k+1)^(2j+1)` as Eigen computes it in `f32` (exactly `(5/3, 10/21, 5/42, 5/252,
1/630)/π`, but the rounded values do not null): `c = M⁻¹[:,0] · f32(1/π)`, with `M⁻¹[:,0]`
from a partial-pivot LU (first largest |pivot|, column scaled by `/pivot`, update `a −=
l·u` unfused), then the permuted unit vector through the unit-lower solve (`x_r =
fma(−x_i, L_ri, x_r)`) and the upper solve (`x_i ·= 1/U_ii`, then `x_r = fma(−x_i, U_ri,
x_r)`). `G = 1/(1 + Σ c_k a_k)`
(lane sums as `(c₁a₁ + c₃a₃) + (c₂a₂ + c₄a₄)`, then `+ c₅a₅`). Per sample: `x = clamp(G·y,
−1, 1)`; out `= x + (((c₁s₁ + c₃s₃) + (c₂s₂ + c₄s₄)) + c₅·sinf(x·a₅))`, `s_k = psin(x·a_k)`
for k ≤ 4 (Eigen 3.4's vectorised sine: reduce by `π/2` with `−1.5707960`,
`−3.1391647e-7`, `−5.3903025e-15` fused, sign and quadrant from the rounded multiple,
sine polynomial `−1.9587841e-4, 8.3321612e-3, −0.16666655`, cosine polynomial
`2.4372266e-5, −1.3886507e-3, 4.1666646e-2, −0.5, 1`, all Horner steps fused).

## 6. Null tests

Through `out/perform-ref-ready.md` (`maschine-ref make … --perform 7`, `render --preroll 0`,
`fx-null`). The project is static: the strip is touched from the first sample and never
moves, so the brake, the drift to the strip's position, the capture window and the swirl
are covered; scratching (strip movement), release and re-touch are not (the code paths
are the same smoothers and equations, from the code only). Results: §6.1.

### 6.1 Results

`scratcher.rs` `tests::nulls_against_maschine` (renders in `out/perform-scratcher/`, 2 s of
noise at −12 dBFS, `--preroll 0`). The core's input is faded in as the processor's load
switch does (§2; `run_faded`), and samples from 20 ms on are compared (the processor
crossfades the output from the old mode before that, which this test doesn't model).

| case | settings | sr | residual |
|---|---|---|---|
| default | Maschine's defaults, engaged | 48k | −240 dB (bit-exact) |
| off | disengaged (input 3 samples late) | 48k | −240 |
| pos0 / pos1 | strip 0 / 1 | 48k | −240 (one 1-ulp difference at ~1e-32 in a tail, pos0) |
| dry | Wet Level 0 | 48k | −240 |
| hot | Wet 1, Feedback 1, Freq Shift 0.9, Delay Time 0.2 | 48k | −240 |
| low | strip 0.3, Freq Shift 0.1, Delay Time 1, Feedback 0.05 | 48k | −240 (one 1-ulp difference at ~1e-32) |
| default | defaults | 44.1k / 96k | −240 / −240 |

The two 1-ulp differences are at levels near 1e-32 (a decaying tail); likely Maschine's
audio thread flushing tiny values (FPCR.FZ), not modelled. Through DECKARD's whole
`PerformFx` (`fx-null`) the residual is −40 dB until mod.rs models the load fade (request
in `out/perform-ref-requests.md`).

## 7. Open points

1. *(Resolved: the Eigen LU, §5.3.)*
2. The load order of parameters relative to `onReset` (DECKARD: parameters, then reset at
   the first sample): confirmed by the null tests.
3. `sinf`, `tanh`, `exp10`, `sincos` are the system's (bit-exact on macOS).
