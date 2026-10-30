# Maschine's Chorus: parameters and DSP

A clean-room description of Maschine 3's built-in **Chorus** effect
(`NI::MASCHINE::DATA::Chorus` → `NI::MASCHINE::RT::ChorusProcessor` →
`NI::MASCHINE::FXCORE::ChorusCore`). `ChorusCore` is a compiled Reaktor-Core kernel: a flat
struct of inputs, outputs and state, with `endInit()` and `processAudio()` (one sample per
call) and one `processEventIn*()` per parameter.

It was written from Ghidra output in `out/decomp/chorus/` (`INDEX.tsv` maps files to
functions) and from arm64 disassembly of the kernel, saved as `out/decomp/chorus-core.s`
(`processAudio`, the event handlers) and `out/decomp/chorus-endinit.s` (`endInit`). Where
the two disagree (Ghidra reorders some products and shows a double division as float),
this spec follows the assembly. Addresses are arm64 VM addresses in
`analysis/maschine_arm64`. The DECKARD port is `crates/engine/src/fx/chorus.rs`. It nulls
**bit-exactly** against Maschine (§5).

**Confidence.** **[exact]**: formula and constants read from code or assembly.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation:
- `v`: a parameter's normalized value (0..1) as stored in the project.
- `sr`: sample rate (`f32` in the kernel).
- All arithmetic is `f32` unless the text says `f64`. The kernel has **no fused
  multiply-adds**: every product and sum is rounded on its own.
- `rnd(x)`: the kernel's rounding. It adds `+0.5` (when `x ≥ 0`) or `−0.5` to `x` **in
  `f64`** and truncates toward zero (`fcvtzs`). So halves round away from zero.

---

## 1. Parameters

### 1.1 Module

Module name "Chorus" (`Module::Module(L"Chorus")`, string at 0x10dce6070). One page:
`s_aPageNames = { "Main" }` (`__GLOBAL__sub_I_Modules_Effects_Chorus_ALL.cpp`, 0x101a0562c)
**[exact]**.

### 1.2 Parameters in constructor order

The constructor is `Chorus::Chorus()` at 0x101a03120 (`0012.c`). Each parameter is made by
`TransactionObjectContainer::insert(label, default, min, max)` **[exact]**. The labels are
wide strings at 0x104d271e0 / 0x104d271f4 / 0x104d27210.

| obj off | mxprj field | label | section | default | range | RT idx | display converter |
|---|---|---|---|---|---|---|---|
| 0xa8 | `rate` | **Rate** | "Mod" | 0.5 | 0..1 | 0 | `ScaleNormalizedToPercentOneDecimal` |
| 0xb0 | `amount` | **Amount** | (Mod) | 0.5 | 0..1 | 1 | `ScaleNormalizedToPercentOneDecimal` |
| 0xb8 | `mix` | **Mix** | "Output" | 0.5 | 0..1 | 2 | `ScaleNormalizedToPercentOneDecimal` |

All three are plain `MaschineNumericParameter<float>`, with no enums or switches. The
display is `v·100` with one decimal and a `%` sign, the same as Compressor's Knee/Amount.
Maschine shows percentages and not Hz or ms, so the physical values in §3 are never shown.

### 1.3 Knob layout (`getParameterByPosition`, 0x101a035c8)

| pos | page | parameter |
|---|---|---|
| 0 | Main | Rate |
| 1 | Main | Amount |
| 2 | Main | Mix |
| 3–7 | Main | – (empty) |

Section headings: slot 0 "Mod", slot 2 "Output" **[exact]**.

### 1.4 Serialization

`iserializer<…, Chorus>::load_object_data` (0x101a03b3c) loads the Module base, then the
objects at 0xa8, 0xb0, 0xb8 in that order **[exact]**. These map to `crates/mxprj` fields
`rate`, `amount`, `mix`, which were `unknown_a8/b0/b8`.

### 1.5 Value → engine (`ChorusProcessor::processFloatParameter`, 0x101a0349c)

Each parameter is bound with `setAndCallFuncAutomate` to `processFloatParameter(idx, …, v)`.
That function stores `v` as is into a kernel input and calls its event handler **[exact]**:

| idx | parameter | kernel input | handler | effect |
|---|---|---|---|---|
| 0 | Rate | +0x18 | `processEventInRate` | rate target (f64) `= (f64)(v·127)`; flag bit 0 |
| 1 | Amount | +0x20 | `processEventInAmt` | amount target `= (f64)v`; flag bit 1 |
| 2 | Mix | +0x10 | `processEventInMix` | mix target `= (f64)v`; flag bit 2 |

The flags at +0x3c only gate the control-rate block (§3.3). `endInit` always leaves all
three set, and nothing clears them, so the control block runs on every period.

---

## 2. Processor (`ChorusProcessor`)

- **Constructor** (0x101a0363c) **[exact]**: stereo in, stereo out. It allocates the kernel
  (0x13a00 bytes, zeroed) and presets the kernel inputs to Rate 0.45, Amount 0.55 and
  Mix 0.5. The module's stored values replace these through `processFloatParameter` before
  audio runs.
- **`onSamplerateChanged(sr)`** only stores `sr` (+0x48). It takes effect at the next
  `resume`.
- **`resume()`** (0x101a037d4) **[exact]**: it saves the three inputs, **zeroes the whole
  kernel** (state, both delay lines and the LFO), writes `sr`, the three inputs and their
  event flags (1.0), sets +0x34 = 0 and the "init" flag +0x38 = 1.0, and calls `endInit()`.
- **`process()`** (0x101a03854): per sample, `in L → +0xc`, `in R → +0x2c`, then
  `processAudio()`, then `+0x28 → out L`, `+0x30 → out R`. It works in place and has no
  block-level state.

So the LFO **free-runs from `resume`**: it always starts at the same phase (§3.2) when the
engine starts, and it is not tied to the transport. With `maschine-ref render --preroll S`,
Maschine runs `S·sr` silent samples first. During those the LFO advances, and the delay
lines fill with zeros. To match a preroll render, DECKARD must process the same number of
silent samples before the input (§5).

---

## 3. DSP (`FXCORE::ChorusCore`)

### 3.1 Tables

The kernel reads static tables from `__DATA`. All of them hold 6-significant-digit decimal
values (every entry is the `f32` nearest its `%.6g` text) **[exact]**:

| symbol | addr | size | content | port |
|---|---|---|---|---|
| `s_05F8C0B8` | 0x10dc1ccf0 | 128 | **LFO rate in Hz**, 0.0639015 … 16.3543, about ×2^(8/127) per step. It has a small ripple once per octave (an approximate exp2 was used to make it) | `RATE_HZ`, copied as numbers |
| `s_05F8C9D8` | 0x10dc1cef0 | 128 | **LFO smoother**, 1.47501 … 377.499 (≈ 23.08 × rate Hz) | `SMOOTH`, copied |
| `s_05F8C300` | 0x10dc1d0f0 | 128 | **depth in ms**: 2.0 for idx ≤ 31, then ≈ 0.534/rate_Hz, 0.0625 for idx ≥ 112 | `DEPTH_MS`, copied |
| `s_04FB03F0` | 0x10dc1e2f4 | 1023 used | `2^(i/1021 − 0.5)`, i = 0..1021; entry 1022 = 1.41421 | formula (`exp2_entry`) |
| `s_04FB1428` | 0x10dc1d2f0 | 257 | a log-like curve, 0 … 0.322776. Only entries 245/246 (0.310295, 0.311434) are read, at a constant position | the two constants |

No formula reproduces the three 128-entry tables to their printed 6 digits: the rate table
has a per-octave ripple, and the other two drift from a pure exponential by about 1e-5. So
the port carries their values. The 1023-entry octave table follows the formula, rounded to
6 significant digits. Seven entries (106, 213, 225, 287, 324, 382, 673) are exact ties at
the 7th digit and were rounded the other way. The port overrides these seven and entry 1022,
which makes it reproduce the table exactly. (At 48/96/192 kHz the Mix smoother's
coefficient interpolates at entry 381/382.)

`exp2(x)` (used for coefficients only) **[exact]**:
```
n  = rnd(x)                      // (the sign test is "x ≥ 0", so NaN → −0.5)
y  = ((x − n) + 0.5) · 1021
k  = rnd(y − 0.5);  f = y − k;  i = k & 0x3ff
exp2(x) = ((1 − f)·E[i] + f·E[i+1]) · 2^n     // 2^n by adding n<<23 to the bits of 1.0f
```

### 3.2 `endInit()` (0x101a102dc): control rate, coefficients, first state

**[exact]**:
```
steps   = rnd_f64(sr / 9800.0)                 // f64 division; 5 at 44.1 k and 48 k, 10 at 96 k
period  = steps − 1                            // control counter limit (+0x44)
ctl_hz  = rnd_f64(sr) / steps                  // integer division
per_ms  = ctl_hz / 1000.0                      // f64
L       = (0.310295·0.9602356 + 0.311434·0.039764404) + (−61.0)   // f32 = −60.689655
c_fast  = 1 − (f64)exp2((f32)((f64)L / max(50·per_ms − 1, 1)))    // mix smoother
c_slow  = 1 − (f64)exp2((f32)((f64)L / max(500·per_ms − 1, 1)))   // rate & amount smoothers
ms      = sr / 1000                            // samples per ms, used for both channels
k44     = 44100 / sr;   inv_sr = 1 / sr
phase   = −0.5;  lfo = 0;  write index = 0;  control counter = 0
targets: rate_t = (f64)(Rate·127), amt_t = (f64)Amount, mix_t = (f64)Mix
smoothed values start at their target: s = t + c·((t − t) + 1e-100)
```
Then the control block of §3.3 runs once (without the counter), and the per-sample code of
§3.4–3.5 runs once on a **silent input** (its output is dropped). This advances the LFO by
one step and the write index to 1. All three flags end up set (§1.5).

So each smoother reaches 2^L ≈ −365 dB after its nominal time (50 ms for Mix, 500 ms for
Rate and Amount). The 1/e time is about T/42: ≈ 1.2 ms for Mix and ≈ 12 ms for the others.

### 3.3 Control block (every `steps` samples)

`processAudio` first increments the counter. When the counter has reached `period`, it
resets the counter and runs this block, so the first update comes on the 5th sample
(44.1/48 k) after `resume` **[exact]**:
```
mix_s  += c_fast·((mix_t − mix_s) + 1e-100)      // f64
amt_s  += c_slow·((amt_t − amt_s) + 1e-100)
rate_s += c_slow·((rate_t − rate_s) + 1e-100)
m = (f32)mix_s;  a = (f32)amt_s;  r = (f32)rate_s
wet = (2 − m)·m
dry = (1 − m)·(2 − (1 − m))                       // = 1 − m², with this rounding
i  = rnd(r − 0.5);  f = r − i
i0 = (u32)i < 127 ? i : 127                       // unsigned: i = −1 (r = 0) → 127
i1 = (u32)(i+1) ≥ 127 ? 127 : i+1
lerp(T) = T[i0] + (T[i1] − T[i0])·f
depth  = (a·a·25) · lerp(DEPTH_MS)                // ms
lfo_c  = k44 · (lerp(SMOOTH) · 1.0440000e-4)      // 0x38daf153
inc    = inv_sr · lerp(RATE_HZ)                   // cycles per sample
```
At Rate = 0 the index wraps (`i0 = 127`, `i1 = 0`, `f = 1`). The kernel still lands on
entry 0, but through `T[127] + (T[0] − T[127])·1`, so the value is rounded **[exact]**.

Physical ranges: the LFO runs at 0.064 Hz (Rate 0) … 1.02 Hz (50 %) … 16.35 Hz (100 %).
The sweep depth is `25·Amount²·DEPTH_MS(rate)` ms. Up to Rate ≈ 24 % it is at most 50 ms (Amount 100 %). Above that it falls
roughly as 1/rate, down to 1.56 ms at Rate ≥ 88 %.

### 3.4 LFO (every sample)

**[exact]**:
```
p     = phase + inc
k     = rnd(p)
phase = p − k                                      // in [−0.5, 0.5]
shape = phase·(|p − k|·(−8) + 4) + 0.5             // parabolic sine, 0..1
lfo   = lfo + lfo_c·((shape − lfo) + 1e-18)        // 0x219392ef; one-pole on the shape
```
The smoother's cutoff is about `lfo_c·sr/2π ≈ 17 × rate_Hz`. It rounds the corners of the
parabolas, and adds a little lag and attenuation at high rates.

### 3.5 Delay lines and output (every sample)

There are two 8192-sample lines (L at +0xf0, R at +0x9d30). Indices are masked with
`0x1fff`, and the write uses `min(w, 9999)`, which never binds. Per sample **[exact]**:
```
w = write index;  write index = (w + 1) & 0x1fff
bufR[w] = xR;  bufL[w] = xL                        // write first, then read
(control block, LFO)
sweep = depth·lfo
dL = ms·sweep                                      // left:  depth·lfo
dR = ms·(depth − sweep)                            // right: depth·(1 − lfo), anti-phase
tap(buf, d):
  n  = rnd(d − 0.5);  rd = w − n
  A = buf[rd−2], B = buf[rd−1], C = buf[rd−3], E = buf[rd]       // all & 0x1fff
  t  = (n − d) + 1
  c1 = (B − C)·0.5;  c2 = (E − A)·0.5;  ab = A − B
  s  = ab + c1;  q = ab + (c2 + s)
  y  = A + t·(c1 + t·(q·t − (s + q)))             // Catmull-Rom between A and B
outL = xL·dry + wet·tap(bufL, dL)
outR = xR·dry + wet·tap(bufR, dR)
```
The interpolated read is `d + 1` samples behind the input (`d = 0` gives a 1-sample delay).
So the wet signal sweeps from 1 sample to `1 + depth·sr/1000` samples. The two channels are
processed independently: there is no cross-feed and no feedback. Mix 0 gives `dry = 1` and
`wet = 0`, so the output is **bit-identical** to the input. Mix 1 gives the wet signal only.
At 192 kHz a 50 ms sweep (9600 samples) is longer than the 8192-sample line and wraps, in
Maschine as well.

---

## 4. Implementation notes

- `Chorus::set` does `resume()` on the first call and sends the three events on later
  calls. The LFO, delay lines and smoothers keep running, as when Maschine automates a knob.
- The port keeps the kernel's operation order, `f64` where the kernel uses doubles, and no
  `mul_add`.
- Not verified against Maschine: parameter changes during a render, because `maschine-ref`
  has no automation. The event and smoother code follows the assembly, and a unit test
  checks that re-sending the same values changes nothing.
- Not used here: the side-chain, tempo, the transport, and output meters (the processor has
  none).

## 5. Null tests

Source project: `~/Documents/Native Instruments/Maschine 2/Projects/4-6 show.mxprj`, sound
0/15 slot 2 (Chorus, stored rate 0.24 / amount 0.71 / mix 0.5), made with
`maschine-ref make … 0 15 2 [rate=… amount=… mix=…]`. Rendered for 3 s, at −12 dBFS, with
`--preroll 0`. The comparison is against `fx-null` on the captured input. Residuals are L / R,
and **−240 dB means bit-identical** (the peak difference is 0).

| case | input | sr | rate / amount / mix | residual |
|---|---|---|---|---|
| project values | noise | 44.1 k | 0.24 / 0.71 / 0.5 | −240 / −240 |
| defaults | noise | 44.1 k | 0.5 / 0.5 / 0.5 | −240 / −240 |
| defaults | noise | 48 k | 0.5 / 0.5 / 0.5 | −240 / −240 |
| project values | sweep | 44.1 k | 0.24 / 0.71 / 0.5 | −240 / −240 |
| project values | sine 440 | 48 k | 0.24 / 0.71 / 0.5 | −240 / −240 |
| rate min (index wrap) | noise | 44.1 k | 0 / 0.5 / 0.5 | −240 / −240 |
| rate max | noise | 44.1 k | 1 / 0.5 / 0.5 | −240 / −240 |
| | noise | 48 k | 0.25 / 1 / 1 | −240 / −240 |
| | noise | 44.1 k | 0.75 / 0.3 / 0.7 | −240 / −240 |
| amount min | noise | 44.1 k | 0.5 / 0 / 0.5 | −240 / −240 |
| amount max | noise | 44.1 k | 0.5 / 1 / 0.5 | −240 / −240 |
| deepest sweep (50 ms) | noise | 48 k | 0 / 1 / 1 | −240 / −240 |
| mix min | noise | 44.1 k | 0.5 / 0.5 / 0 | −240 / −240 |
| mix max | noise | 44.1 k | 0.5 / 0.5 / 1 | −240 / −240 |
| | impulse | 44.1 k | 0.3 / 1 / 1 | −240 / −240 |
| odd values | noise | 48 k | 0.123 / 0.877 / 0.333 | −240 / −240 |
| | sweep | 44.1 k | 0.05 / 1 / 0.5 | −240 / −240 |
| control rate 10 | noise | 96 k | 0.6 / 0.8 / 0.6 | −240 / −240 |
| control rate 2 | noise | 22.05 k | 0.6 / 0.8 / 0.6 | −240 / −240 |
| `--preroll 1`, input fed as is | noise | 44.1 k | project | −17.1 / −17.2 (LFO phase differs) |
| `--preroll 1`, DECKARD fed 44100 zeros first | noise | 44.1 k | project | −240 / −240 |
| `--preroll 1`, DECKARD fed 48000 zeros first | noise | 48 k | 0.9 / 1 / 1 | −240 / −240 |

For the preroll rows, the captured input was padded with `S·sr` zero frames, and the same
number of frames was trimmed from DECKARD's output (scratch script `pad.py`). The match shows
that Maschine's preroll is exactly `S·sr` processed samples after `resume`, and that the LFO
free-runs through it.
