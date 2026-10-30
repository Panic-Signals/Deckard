# Maschine's Saturator: parameters and DSP

A clean-room description of Maschine 3's built-in **Saturator** effect. The chain is
`NI::MASCHINE::DATA::Saturator` (parameters, persistence) → `NI::MASCHINE::RT::SaturatorProcessor`
(real-time wrapper) → one of three DSP cores chosen by Mode:

| Mode | core | DECKARD |
|---|---|---|
| 0 Classic | `NI::MASCHINE::FXCORE::SaturatorCore` | `crates/engine/src/fx/saturator.rs` (`Classic`) |
| 1 Tape | `NI::DSP::Tape` + `NI::AB::SampleRateConverter` (×2) | `crates/engine/src/fx/saturator/tape.rs` |
| 2 Tube | `NI::DSP::WarmTube` (×4 internally) | `crates/engine/src/fx/saturator.rs` (`Tube`) |

It was written from Ghidra output and objdump listings of the arm64 slice
(`analysis/maschine_arm64`; addresses are arm64 VM addresses):

- `out/decomp/saturator/`: data class, processor, `SaturatorCore`, `WarmTube`, `Tape`
  (`INDEX.tsv` maps files to functions). Hand-read assembly is in `asm-saturatorcore.s`,
  `asm-wt-*.s` and `asm-tape-*.s`.
- `out/decomp/saturator-tape/`: the Tape mode's helpers and `SampleRateConverter`, including
  `asm-double-half.s`.

Ghidra drops NEON lanes, `fmaxnm` and fused operations in these functions. Where Ghidra and
the assembly disagree, this spec follows the assembly.

**Confidence.** **[exact]**: formula and constants read from code or assembly. **[inferred]**:
reconstructed from partial evidence. **[unknown]**: not recovered. All three modes null
against Maschine bit for bit (§8), so everything marked [exact] is also confirmed by
measurement.

Notation:
- All arithmetic is `f32` unless the text says `f64`.
- `fma(a,b,c)` is `a·b + c` with one rounding (`fmadd`). Every other product or sum is
  rounded on its own.
- `E(x)` is the shared ExpTable lookup (docs/fx-compressor.md §3.1), and
  `dB→lin(x) = E(x·9.965784 + 1200)`.
- `exp10f` is Apple's `__exp10f`; `powf`, `exp2f` and `tanf` are the system libm functions.

---

## 1. Parameters

### 1.1 Stored fields

`Saturator::Saturator()` (0x101a739a4, `0009.c`) inserts the parameters with
`TransactionObjectContainer::insert(label, default, min, max)` **[exact]**. The serialization
order is the same as the offset order, with no class-version conditions
(`serialize<iarchive>` 0x101a73754).

| off | mxprj field | label | type | default | stored range | display | core idx | notes |
|---|---|---|---|---|---|---|---|---|
| 0xa8 | `mode` | Mode | enum | 0 | Classic, Tape, Tube | – | `setMode` | section **"Main"**, not automatable, forces a display update |
| 0xb0 | `classic_input` | Input | float | 0.25 | 0..1 | `ScaleNormalizedToPercentOneDecimal` (`"%.1f %"` of 100·v) | Classic 0 | |
| 0xb8 | `classic_drive` | Drive | float | 0.5 | 0..1 | percent | Classic 1 | |
| 0xc0 | `classic_contour` | Contour | float | 0.75 | 0..1 | percent | Classic 2 | |
| 0xc8 | `tape_input` | Input | float | 0.2 | 0..1 | `getTapeGainValueString`: `"%.1f dB"` of 30·v | Tape 0 (normalized) | |
| 0xd0 | `tape_drive` | Drive | float | 0.34 | **−1..1** | `getTapeWarmthValueString`: `"%.1f dB"` of 3·v | Tape 1 (normalized (v+1)/2) | bipolar; wheel `processWheelDeltaSnapToZero`; +0x1d0 = 2 |
| 0xd8 | `tape_contour` | Contour | float | 1.0 | 0..1 | `getTapeToneValueString`: `"%.1f k"` of 8·v + 12 | Tape 2 (normalized) | |
| 0xe0 | `tube_drive` | Drive | float | 0.25 | 0..1 | percent | Tube 0 (normalized) | |
| 0xe8 | `tube_charge` | Charge | float | 2.0 | **0..4** | `getTubeChargeValueString`: `"%.1f %%"` of 25·v | Tube 1 (**raw**) | wheel `processWheelOnFineResolutionOnly(0.02)`; +0x1d0 = 4, +0x1c0/+0x1c8 = 0.4 |
| 0xf0 | `tube_bass` | Bass | float | 0.8 | 0..1 | percent | Tube 4 (normalized) | |
| 0xf8 | `tube_treble` | Treble | float | 0.8 | 0..1 | percent | Tube 5 (normalized) | |
| 0x100 | `tube_gain` | Gain | float | 0 | **−12..12** | `getTubeGainValueString`: `"%.1f dB"` of v | Tube 6 (**raw**) | section **"Output"**; bipolar; wheel `processWheelDeltaGeneric(1.0, 0.1)` |
| 0x108 | `tube_overload` | Overload | bool | false | | on/off | Tube 2 (0/1) | not automatable |
| 0x110 | `tube_eq_bypass` | Bypass | bool | false | | on/off | Tube 3 (0/1) | section **"EQ"**, not automatable |

- **Labels** are wide strings at 0x104d34fd8 (Mode), 0x104d34fec (Input), 0x104d35004 (Drive),
  0x104d3501c (Contour), 0x104d3503c (Charge), 0x104d35058 (Bass), 0x104d3506c (Treble),
  0x104d35088 (Gain), 0x104d3509c (Overload), 0x104d350c0 (Bypass). The format strings are
  0x104d350f8 `"%.1f dB"`, 0x104d35118 `"%.1f k"` and 0x104d35134 `"%.1f %%"` **[exact]**.
- **Mode options** (vector at 0x10e306390, built in `__GLOBAL__sub_I_Modules_Effects_Saturator_ALL.cpp`):
  "Classic", "Tape", "Tube" **[exact]**.
- **Module info:** name "Saturator", id 0x20. One page, **"Main"** (`s_aPageNames`) **[exact]**.
- **Binding** (`setAndCallFuncAutomate`):
  - Classic floats call `processClassicParameter(idx, v)`, which stores v and tail-calls
    `processEventInCmp` / `processEventInDrv` / `processEventInCntr`.
  - Tape and Tube parameters go to the core's `setParameter(idx, v)` (vtable +0x68).
  - Tape 0..2 and Tube 0, 2, 3, 4, 5 are passed through `NumericParameter::getNormalized`.
    Tube Charge (1) and Gain (6) are passed raw **[exact]**.
- Every core always receives its own parameters, whatever the mode.

### 1.2 Page layout (`getParameterByPosition`, 0x101a74d40) **[exact]**

The eight slots depend on Mode.

| slot | Classic | Tape | Tube |
|---|---|---|---|
| 0 | Mode | Mode | Mode |
| 1 | Input (`classic_input`) | Input (`tape_input`) | Charge |
| 2 | Contour (`classic_contour`) | Contour (`tape_contour`) | Overload |
| 3 | Drive (`classic_drive`) | Drive (`tape_drive`) | Drive (`tube_drive`) |
| 4 | – | – | Bypass |
| 5 | – | – | Bass |
| 6 | – | – | Treble |
| 7 | – | – | Gain |

Section headings come only from `setSectionName`: "Main" on Mode (slot 0), "EQ" on Bypass
(Tube slot 4), and "Output" on Gain (Tube slot 7).

The QML model (`SaturatorModel`) names the bindings Mode, Drive, Contour, Compress,
TapeGain, TapeTone, TapeWarmth, TubeDrive, TubeCharge, TubeGain, TubeEQLow, TubeEQHigh,
TubeEQBypass and TubeBassOverload. So the hardware "Input" of Classic is the editor's
"Compress", Tape's "Drive" is its "Warmth" and Tape's "Contour" its "Tone". The DSP below
confirms these meanings.

---

## 2. Processor (`SaturatorProcessor`)

### 2.1 Construction and life cycle

**Constructor** (0x101a74fb8) **[exact]**:
- Allocates `SaturatorCore` (0x180 bytes), `Tape` (0x4280 bytes) and `WarmTube` (0x300 bytes).
- Classic gets its defaults (0.25, 0.5, 0.75), then `beginInit(44100)` and `endInit()`.
- Pins are stereo in, stereo out.
- Tape gets `onInit()` and then `setParameter(3, 0.0)`, so its HQ is off.
- WarmTube gets `onInit()`.
- The Tape/Tube control counter (+0xc0) is 0.

The parameters are then sent, and after that the host's sample rate arrives:
- **`onSamplerateChanged(sr)`** stores sr and calls `Tape.onSetSampleRate(sr, 128)` and
  `WarmTube.onSetSampleRate(sr, 128)`. Classic is not touched.
- **`resume()`**:
  - Classic: `beginInit(sr)`, which clears everything including the raw parameters. The
    processor saves and restores the three raw values, sets the three "changed" flags and
    calls `endInit()`.
  - Tape: `onReset()`, then `onResume()`.
  - Tube: `onReset()`, then `onResume()`.
- **`suspend()`** calls `onSuspend()` on Tape and Tube.

### 2.2 `process()` (0x101a7545c) **[exact]**

- **Mode 0 (Classic):** `SaturatorCore::processAudio()` runs once per sample, in place, with
  `(in, out) = (+0x1c/+0x20, +0x24/+0x28)`.
- **Mode 1 or 2:** the block is cut at a 128-sample grid. The counter at +0xc0 runs across
  blocks; it is not reset by `resume`, and it starts at 0, so the first sample ticks.
  - At each grid point the active core's `processControlRate()` runs.
  - Each piece then goes through the core's `processAudio(in, out, len)`.
  - The counter only advances while Mode is 1 or 2.
- **Mode switches** only change +0xa0. No core is reset; an inactive core keeps its state.
- There is **no dry/wet** at processor level and no latency compensation.

### 2.3 Parameter ramps (all cores)

All cores use the same scheme as the Compressor (docs/fx-compressor.md §3.2):
- Targets, currents and increments are kept per lane.
- On the first tick after a change, `inc = (target − current)·inv`. Any lane with
  `(f64)(inc²) < 1e-15` snaps to its target. If any lane still moves, a countdown of N
  ticks starts; when it reaches 0, every lane snaps.
- `current += inc` every sample.
- N = `max(1, (int)(sr·T/128 + 0.5))` and `inv = 1/(N·128)`.
- T = 0.02 s for Classic and Tube, 0.03 s for Tape (§5).

---

## 3. Shared tables

- **`E(x)`:** the ExpTable (docs/fx-compressor.md §3.1).
- **`SaturatorCore::s_GainArray`** (0x10dc23f18, 6529 `f32`): 51 rows × 128 points of a level
  transfer curve `y(x)` at `x = i/127`, plus a trailing 1.0.
  - Row 0 is the identity, and every row ends at 1.
  - Higher rows rise faster; they are close to, but not exactly, `(1 − e^(−kx))/(1 − e^(−k))`.
  - The table was copied from the binary into `saturator/gain_table.rs`, the same way as
    `sampler_tables.rs`. It could not be regenerated from a formula **[exact data / unknown
    origin]**.
- **Tape:** a 4098-entry curve table and the ×2 converter's sinc (§5).

---

## 4. Classic (`FXCORE::SaturatorCore`)

### 4.1 Init (`beginInit` 0x101a1fbc0, `endInit` 0x101a1fd60) **[exact]**

`beginInit(sr)` clears the whole core (0x00..0x16f), stores sr, sets the tick period to 128
(+0xd0) and the counter to 129 (+0xd8; the first sample ticks), then computes, with
`ms = sr·0.001`:

```
c(m, n) = 1 − exp2f( d < 1 ? n : n/d ),   d = fma(ms, m, −1)
c54 = c(10.8388, −3.321928)   // Input envelope release: −20 dB in 10.84 ms
c58 = c(2.67874, −6.643856)   // Input gain smoother: −40 dB in 2.68 ms
c64 = c(2,       −3.321928)   // drive envelope attack (2 ms)
c68 = c(40,      −3.321928)   // drive envelope release (40 ms)
K   = tanf(1727.876/sr);  a0inv = 1/(K + 1);  km1 = K + (−1)
gsm = 0 (+0x5c);  env = 0.1 (+0x60);  env2 = 0;  all filter states 0
```

The constants are 10.8388 = 0x412d6bba and 2.67874 = 0x402b707a; the numerators are
−log2(10) and −2·log2(10). `endInit` sets N and inv for 20 ms, recomputes every parameter
from its stored value (below), copies targets to currents and raises the change flag.

### 4.2 Parameters (`processEventIn*`) **[exact]**

Ramp lanes: 0 `cg`, 1 `Y`, 2 `X`, 3 `Z`, 4 `D`; lanes 5..7 are unused (always 0).

| param | computation |
|---|---|
| Input `v` | `cmp = v` (used directly, not ramped); `q = (1−v)²`; `cg = fma(fma(q²−q, v, q), 0.436367, 0.563633)` → lane 0 |
| Drive `v` | `D = v(2−v)` → lane 4; `s = D > 0 ? (D + (−0.8))·5 : 0`; `Y = fma(s, −0.195, 0.65)` → lane 1; `Z = min(25·D, 1)` → lane 3; and X as for Contour |
| Contour `t` | `X = fma((fma(s, −0.15, 0.9) − D/4), t, D/4)` → lane 2 |

Note that `s` goes negative for D < 0.8.

### 4.3 Per sample (`processAudio`, 0x101a20068) **[exact]**

1. **Ticks and ramps:** the counter is incremented; when it reaches 128 it resets and a
   control tick runs (§2.3). Then `current += inc`.
2. **Input stage** (only when `cmp > 0`): a stereo-linked peak compressor through the gain
   table.
   ```
   p   = max(|L|, |R|)
   env = (p − env > 0) ? p : fma(p − env, c54, env)          // instant attack
   e   = min(env + 5.96046e-8, 1);   xi = e·127
   row = ceil(cmp·50)                                         // fcvtps
   f   = xi + (−0.5);  i = clamp((int)f + 128·row, 0, 6527)
   g   = fma(T[i+1] − T[i], xi − (float)(int)f, T[i]) / e     // curve → gain
   gsm = fma(g − gsm, c58, gsm)
   L, R *= cg·gsm
   ```
3. **Drive envelope:**
   ```
   d    = max(|L|, |R|) − env2
   env2 = fma(d + 1e-10, d > 0 ? c64 : c68, env2)
   m    = min(env2, 1)
   a    = D·fma(1 − m, X, m)
   b    = Y·a
   ```
4. **Band split:** a one-pole per channel, `lp = (lp + (x − lp·km1)·a0inv)·K`. Both bands are
   then driven in dB:
   ```
   ga = E(fma(60a, 9.965784, 1200));  gb = E(fma(60b, 9.965784, 1200))
   u  = [(L − lpL)·gb, (R − lpR)·gb, lpL·ga, lpR·ga]          // high bands, low bands
   ```
5. **Soft clip at ×2:**
   - The half-sample point is `h = ((u2·0.5625 − u3·0.0625) + u1·0.5625) − u·0.0625`, where
     u1..u3 are the previous three `u` vectors.
   - `sat(x) = x·(1 − |x|/16)` after clamping x to ±8.
   - `o = W·((sat(h) + sat(u1))·0.5)`, where `W(v) = (q + (q² − q)·0.65)·0.95001 + 0.04999`
     with `q = (1−v)⁴`. The high lanes use W(b) and the low lanes W(a).
6. **Output:**
   ```
   wet = o_high + o_low  (per channel)
   out = clamp((wet − dry)·Z + dry, −1, 1)
   ```
   `dry` is the previous sample after the Input stage. The wet path is therefore 1 to 1.5
   samples late and the dry path 1 sample late. The output is **hard-clipped at ±1**.

---

## 5. Tape (`NI::DSP::Tape`, 0x10264c6fc..)

All **[exact]** from asm (`asm-tape-*.s`, `saturator-tape/asm-double-half.s`), confirmed by
bit-exact null tests.

### 5.1 Parameters

`setParameter` is at 0x10264df48. Values arrive through `getNormalized`.

| idx | field | DSP |
|---|---|---|
| 0 | `tape_input` v | drive gain `g = 300·E(48v·9.965784 + 1200)` = 300·10^(48v/20) → ramp A lane 0 |
| 1 | `tape_drive` (Warmth) w = (stored + 1)/2 | warmth gain = `exp10f((6w − 3)·0.05)`, i.e. ±3 dB → ramp B lane 0 |
| 2 | `tape_contour` (Tone) t | ramp A lane 1; also sets the "remake tone coefficients" flag |
| 3 | HQ | `v > 0.5` would select ×4. Maschine always sends 0 |

- **onInit defaults:** gain 0, tone 1, warmth 0.5.
- **Ramps** follow §2.3 with 30 ms:
  - Ramp A runs at the oversampled rate: N = max(1, (int)(sr·F·0.03/(F·128) + 0.5)),
    inv = 1/(N·F·128).
  - Ramp B runs at the base rate.
  - `onReset` snaps both ramps.
- **Oversampling factor F:** 2 if sr ≤ 50 kHz, else 1. At F = 1 the converters are type 2
  (copy).

### 5.2 Signal path

Every filter stage is a 4-lane NEON section. Lanes 0/1 are filter A (L/R). Lanes 2/3 are
filter B, whose input is A's output from the previous sample, so each stage is A then B in
series with one extra sample of delay.

1. **Pre-filter** (base rate; coefficients at +0x1430..+0x1470), DF-I:
   - A: a 2nd-order highpass at 12.5 Hz, designed with fs = F·sr. With F = 2 it runs at half
     its design rate, so it is effectively ≈ 6 Hz.
   - B: a shelf at 8.5 Hz, weighting 0.7782794.
2. **Up-sampler** (`SampleRateConverter` type DOUBLE, `init(sr, 2sr, 2 ch, quality 2)`):
   - Even outputs are the input delayed by 8 samples.
   - Odd outputs come from a 16-tap transposed FIR with h(k + ½), k = −8..7:
     `h(t) = sin(πt)/(πt) · I0(8·√(1 − (t/8)²)) / 427.5641467788136`.
   - I0 uses the Abramowitz–Stegun polynomials in `f64` with `fma`.
3. **At F·sr:**
   - **Anti-alias biquad** (+0x1700..): a 2nd-order bilinear lowpass at 13 kHz, 1/Q = 0.5555556,
     DF-II, on all four lanes.
   - **Shelf** (+0x1750..): lanes 0/1 are a 1st-order shelf at 10 kHz with constant 2.981072
     (2nd order if F·sr > 100 kHz); lanes 2/3 pass through.
   - **Tape curve:** `y = sign(x·g)·T(min(|x·g|, 4096))·1395.1892/g`.
     - g is floored at 1e-6, and 1/g is NEON `frecpe` plus one `frecps` step.
     - T is a 4098-entry table (0x10c351998, `f64`, converted to `f32`), linearly
       interpolated. It is not tanh.
     - Samples are processed in pairs with their own g, so the curve output is one
       oversampled sample late.
4. **Tone stage** (F·sr; +0x1a20..):
   - A: a bilinear lowpass at 10000 + 10000·t Hz, 1/Q = 1.4285715.
   - B: a lowpass at 12000 + 8000·t Hz, 1/Q = 1.25.
   - The coefficients are recomputed (tan in `f64`) whenever the ramped tone moves or the flag
     is set.
5. **Fixed stage** (+0x1a70..):
   - A: a 1st-order shelf at 500 Hz.
   - B: a 2nd-order lowpass at 1.6 kHz, 1/Q = 2.5.
6. **Down-sampler** (type HALF, `init(2sr, sr)`): the same 16 taps.
   `out = 0.5·(even input delayed by 8 + FIR over the odd inputs)`.
7. **Output** (base rate; +0x1d40..):
   - A: a 1st-order shelf at 40 Hz, 0.12201846.
   - B: a 1st-order highpass at 10 Hz, scaled by 0.70794576.
   - The Warmth band `z` is a 1st-order lowpass at 27.5 Hz of y with gain 27.183832.
   - `out = y + z·warmth`.

`onResume` and `onSuspend` are empty. `onReset` clears the filter states but **not** the
converters' delay lines. The converters and filters delay the signal: against the dry input,
the best-matching lag at 48 kHz is 86 samples.

---

## 6. Tube (`NI::DSP::WarmTube`, 0x102640dcc..)

### 6.1 Helpers **[exact]**

- **sin/cos polynomials** of `w` (`x = w²`, every step `fma`):
  - `sin = w·(((((x·2.39e-8 + 2.7526e-6)x − 1.98409e-4)x + 8.33333e-3)x − 0.166667)x + 1)`
  - `cos = ((((x·−2.605e-7 + 2.47609e-5)x − 1.38884e-3)x + 0.0416666)x − 0.499923)x + 1`
- **ω(f)** = `(clamp(f, sr/24576, sr/2.125)·6.2831)/sr`.
- **Lowpass(f)** (Q = 1):
  ```
  α = sin/2;  inv = 1/(α + 1)
  a1 = (−2cos)·inv;  a2 = (1 − α)·inv
  b0 = b2 = ((1 − cos)·0.5)·inv;  b1 = (1 − cos)·inv
  ```
- **Low shelf at 80 Hz**, `A = exp10f(dB·0.025)`, `s = sin` (no halving):
  ```
  t   = fma(A−1, cos, A+1);  inv = 1/fma(2√A, s, t)
  a1  = fma(cos, A+1, A−1)·−2
  a2  = fma(−2√A, s, t)
  u   = (A+1) − (A−1)·cos (fused);  v = (A−1) − (A+1)·cos (fused)
  b0  = fma(2√A, s, u)·A;  b1 = (v + v)·A;  b2 = (A·fma(−2√A, s, u))·inv
  ```
  `a1`, `a2`, `b0` and `b1` are multiplied by `inv` afterwards.

### 6.2 Parameters (`setParameter` 0x102641670) **[exact]**

Ramp lanes (§2.3, 20 ms): 0 Charge, 1 Treble Hz, 2 Bass dB, 3 on/off (unused by the audio),
4 drive gain `a`, 5 exponent `k`, 6 makeup, 7 output gain.

| idx | computation |
|---|---|
| 0 Drive v | `d = fma(|v|, −0.33, 1.33)·v`; `a = powf(10^(1/20), fma(d, 36, −24))`; `k = 0.02d`; `m = min(|a|, 1)`; `full = fma(powf(|a|, max′(k)) − a, m(2−m), a)`, where `max′(k) = (f64)k < 0.001 ? 0.001 : k`; makeup = `(f64)full < 0.1 ? 10 : 1/full`; out = `fma(−0.44d, d, 1)·powf(10^(1/20), gain dB)` |
| 1 Charge v (0..4) | lane 0 = v |
| 2 Overload b | highpass on = `b ≤ 0` (Overload **off** puts the input highpasses in) |
| 3 Bypass b | EQ on = `b ≤ 0` |
| 4 Bass v | dB = 12v → lane 2; the shelf is recomputed at once |
| 5 Treble v | Hz = fma(v, 5000, 9000) → lane 1; the lowpass is recomputed at once |
| 6 Gain v (dB) | lane 7 = `powf(10^(1/20), v)·fma(−0.44d, d, 1)` |

**`onInit`:**
- Sets the 12 kHz lowpass and the 80 Hz shelf at 0 dB.
- Sets every ramp's current and target to `[0.5, 9000, 0, 1, 0.70550203, 0.01165, 1.0301, 0.5]`
  (lane 7: 0.5).
- Loads the interpolator and decimator taps.

The parameters sent afterwards only set the **targets**, so after `resume` every lane glides
from these internal defaults to the real values over the first 20 ms. This is reproduced.

**`onSetSampleRate`:**
- N and inv for 20 ms.
- With `w = 6.2831/sr`: `c_env = min(24w, 1)`, `c_dc = min(12w, 1)`, and the highpass
  coefficients `[min(36w,1)×2, min(80w,1)×2]`.
- The 12 kHz lowpass, the shelf at the stored dB, and the treble lowpass at the stored Hz.

**`onResume`** sets lane 3's target to 0. **`onReset`** clears all signal states.

### 6.3 Per sample (`processAudio` 0x102641b60) **[exact]**

1. **Ramps and EQ coefficients:** `current += inc` on 8 lanes. While lane 1 or lane 2 moves
   (inc ≠ 0), the treble lowpass or the bass shelf is recomputed from the current value.
2. **Input highpasses** (Overload off only), four lanes:
   ```
   A += c·H;  H = [L − A0, R − A1, H0old − A2, H1old − A3];  x = H2, H3
   ```
   This is 36 Hz then 80 Hz, one sample late.
3. **Charge bias**, per channel, from the previous output `y`:
   ```
   env = fma(max(|y| − 0.1, 0)·Charge − env, c_env, env)
   e   = clamp(env, −2, 2)
   g   = max(1 − e·fma(−0.25, |e|, 1), 0.1)
   x  *= g
   ```
4. **×4 tube stage**, per channel:
   - The history is `[x[n], x[n−1], x[n−2], x[n−3]]`.
   - Four phases, each `(t0h0 + t2h2) + (t1h1 + t3h3)`, with taps:
     - `[0,1,0,0]`
     - `[−0.0703125, 0.8671875, 0.2265625, −0.0234375]`
     - `[−1/16, 9/16, 9/16, −1/16]`
     - `[−0.0234375, 0.2265625, 0.8671875, −0.0703125]`
   - Each phase value is multiplied by `a` to give `z`, then shaped:
     `s = z + m(2−m)·(sign(z)|z|^k − z)` with `m = min(|z|, 1)` and `k = max′(lane 5)`.
   - `|z|^k` is approximated:
     - `log2 ≈ e + f·(1.34625 − 0.34625f)`, with f from the mantissa and e from the exponent.
     - `2^y = 2^round_even(y)·(1 + f(0.693 + f(0.242 + 0.056f)))`.
     - All products in this approximation are rounded separately.
   - Decimation is a 16-tap FIR over the last four ×4 blocks (newest first):
     - `[−0.005859375, −0.015625, −0.017578125, 0]`
     - `[0.056640625, 0.140625, 0.216796875, 0.25]`
     - `[0.216796875, 0.140625, 0.056640625, −0]`
     - `[−0.017578125, −0.015625, −0.005859375, 0]`
   - Lanes are summed as `((h0·s + h1·p1) + h2·p2) + h3·p3`, then `(l0 + l2) + (l1 + l3)`.
   - `y = makeup·sum`, which becomes the next sample's bias input.
5. **EQ** (skipped when Bypass is on), two 4-lane DF-II passes,
   `w0 = (x − a1w1) − a2w2` and `y = (b1w1 + b0w0) + b2w2`:
   - **Pass 1:** lanes 0/1 are the 12 kHz lowpass of y. Lanes 2/3 are the bass shelf of the
     previous lowpass output.
   - **Pass 2:**
     - Lanes 0/1 are the treble lowpass of the shelf output.
     - Lanes 2/3 are a DC blocker on the previous treble output: a1 = c_dc, a2 = 1, b0 = 0,
       b1 = c_dc, b2 = 1, and its w2 is the previous output.
     - The output is lanes 2/3 of `w0`, which is `x − lowpass(x)`: a one-pole highpass near
       12 Hz (`c_dc ≈ 2π·12/sr`).
6. **Output:** `out = (EQ ? eq : y)·lane 7`. The EQ adds two samples of delay: one between
   the 12 kHz lowpass and the shelf, and one between the treble lowpass and the DC blocker.

---

## 7. DECKARD implementation notes

- `Saturator::new(sr)` builds all three cores as the constructor does.
  - The first `set` applies Tape and Tube parameters, then re-inits Classic at sr (`resume`),
    `Tube.onSetSampleRate`, and `onReset`/`onResume` on Tape and Tube.
  - Later `set` calls send only the changed parameters (ramped), and keep every core's state.
- Tape's Drive is sent as `(v + 1)/2`. Tube's Charge and Gain are sent raw.
- Results match bit for bit on Apple silicon, which uses the system libm (`exp2f`, `tanf`,
  `powf`, `__exp10f`) and NEON `frecpe`/`frecps` (Tape, via intrinsics). Elsewhere `exp10f`
  falls back to `powf` and results may differ in the last bits.
- Denormals: Maschine flushes them to zero, DECKARD does not. On an impulse input the
  residual peak is about 1e-36 (−240 dB), so this does not matter.

---

## 8. Null tests

Setup:
- Source: `/Users/Shared/Chromatic Fire Library/Projects/Exosphere.mxprj`, group 0, sound 0,
  slot 2 (`maschine-ref make … 0 0 2 field=value`).
- `maschine-ref render --preroll 0` (noise at −12 dBFS, 2 s, 48 kHz unless noted) against
  `fx-null` on the same input.
- "Black Earth.mxprj" does not load in the plug-in (an unregistered class, even when passed
  through), so it was not used.
- Pairs with mismatched settings leave −17 to −36 dB of residual, so the tests do tell
  settings apart.

| mode | case | residual |
|---|---|---|
| Classic | defaults (0.25, 0.5, 0.75) | −240 dB (bit-exact), lag 0 |
| Classic | Input 0 / 0.5 / 1 | −240 / −240 / −240 |
| Classic | Drive 0 / 0.3 / 1 | −240 / −240 / −240 |
| Classic | Contour 0 / 1 | −240 / −240 |
| Classic | all 1; +6 dBFS (Input 0.7, Drive 0.8); −50 dBFS (Input 0.9) | −240 each |
| Classic | sweep; impulse; 100 Hz sine at 0 dBFS | −240 each (impulse: denormal-only peak 1.8e-38) |
| Classic | 44.1 kHz and 96 kHz (0.4, 0.6, 0.2) | −240 / −240 |
| Tape | project values (Input 0.25, Drive 0.68, Contour 1) | −240 |
| Tape | Input 0 / 0.5 / 1; Drive −1 / 0 / 1; Contour 0 / 0.5 | −240 each |
| Tape | 44.1 kHz and 96 kHz (0.7, 0.2, 0.3); +6 dBFS all max; sweep; impulse | −240 each |
| Tube | defaults (Drive 0.25, Charge 2, Bass/Treble 0.8, Gain 0) | −240 |
| Tube | Drive 0 / 0.6 / 1 | −240 / −240 / −240 |
| Tube | Charge 0 / 1.3 / 4 | −240 each |
| Tube | Overload on; EQ Bypass on | −240 / −240 |
| Tube | Bass 0 / 1; Treble 0 / 1; Gain −12 / +12 | −240 each |
| Tube | mixed (0.9, 3, Overload, 0.3, 0.2, +4.5 dB); +6 dBFS max drive; sweep; −60 dBFS | −240 each |
| Tube | 44.1 kHz and 96 kHz (Drive 0.6, Bass 0.4, Treble 0.9) | −240 / −240 |
| all | Classic settings with Tape and Tube fields at extremes (mode 0) | −240 |

The Tape fork also measured −240 dB at 44.1, 48 and 96 kHz for every row of its own matrix
(input, drive and contour extremes, +6 dBFS, sweep).

---

## 9. Open points

1. **Parameter changes mid-stream** are ported exactly (ramps, per-sample EQ recompute,
   Tape's tone recompute) but are not null-tested, because `maschine-ref` renders static
   parameters only.
2. **Tape HQ** (idx 3, ×4 oversampling) is never enabled by the Saturator and is not
   implemented. The Tape branch for F·sr > 100 kHz is ported but untested.
3. **Suspend/resume while running** (Tube lane 3, Tape's converter delay lines that survive
   `onReset`) only matters for hosts that suspend the effect. DECKARD never does.
4. The meaning of the s_GainArray rows (how NI generated them) is **[unknown]**. The table
   is used as data.
5. The UI labels on hardware are taken from the parameter labels (Input, Drive, Contour, …).
   Whether the Maschine software shows Tape's slots as "Input / Drive / Contour" or as
   "Gain / Warmth / Tone" is **[inferred]** from the QML binding names only.
