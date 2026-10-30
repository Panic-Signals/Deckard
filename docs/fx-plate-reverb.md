# Maschine's Reverb (Room / Hall / Plate): parameters and DSP

A clean-room description of Maschine 3's built-in **Reverb** module, class
`NI::MASCHINE::DATA::PlateReverb` (mxprj module name "PlateReverb", display name "Reverb")
→ `NI::MASCHINE::RT::PlateReverbProcessor` → three engines:

- **Room** and **Hall**: two instances of `NI::DSP::GaloisReverb` (`GaloisReverb(false)` and
  `GaloisReverb(true)`), a 31-line feedback delay network with a circulant feedback matrix.
- **Plate**: `NI::DSP::PlateReverb`, a hand-vectorised (SSE-style, compiled to NEON) 16-line
  network with early reflections.

The DECKARD port is `crates/engine/src/fx/plate_reverb.rs` (processor, parameters) with
`plate_reverb/galois.rs` and `plate_reverb/plate.rs`. All three modes null **bit-exactly**
against Maschine at 44.1 and 48 kHz, including parameter ramps (§6).

It was written from Ghidra output in `out/decomp/plate_reverb/` (DATA, RT, Plate) and
`out/decomp/plate_reverb-galois/` (Galois, EQ, smoothing), each with an `INDEX.tsv`, and from
arm64 disassembly: `out/decomp/plate_reverb-impl.s` (`PlateReverb::processAudioImpl<false>`),
`-damp.s` (`generateDampingFiltersForAllDelays`), `-init.s` (`initBuffersAndPointers`),
`-galois2.s`, `-galois3.s`, `-galois6.s`, `-galois7.s` (`GaloisReverb::processAudio<2/3/6/7>`)
and `-gemv.s` (Eigen's matrix–vector kernel). Where Ghidra and the assembly disagree (fused
multiply-adds, SIMD lanes, libm arguments), this spec follows the assembly. Addresses are arm64
VM addresses in `analysis/maschine_arm64`.

**Confidence.** **[exact]**: read from code or assembly and confirmed by null tests.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation: `v` is a stored (normalised 0..1) value; arithmetic is `f32` unless marked `f64`;
`fma(a,b,c) = a·b + c` with one rounding; `sr` the sample rate.

---

## 1. Parameters

### 1.1 Module

`Module(L"Reverb")`, `s_aModuleInfo` id 0x19 (`__GLOBAL__sub_I_Modules_Effects_PlateReverb_ALL.cpp`,
0x101a610a4). Pages `s_aPageNames = { "Main", "EQ" }`. Mode values
`s_aModeValueList = { "Room", "Hall", "Plate" }` **[exact]**.

### 1.2 Fields in constructor order

`DATA::PlateReverb::PlateReverb()` at 0x101a5c1f4 (`0052.c`). Each field is
`insert(label, default, min, max)` **[exact]**:

| obj off | mxprj field | label | section | default | range | RT idx | display |
|---|---|---|---|---|---|---|---|
| 0xa8 | `mode` | **Mode** (enum) | "Room" | 0 (Room) | Room, Hall, Plate | `setMode` | value string; not automatable, forces a display update |
| 0xb0 | `room_rev_time` | **Reverb Time** | | 0.2 | 0..1 | 0 | `"%.1f s"` of `exp(3.7·v)·0.5` (0.5 … 20.2 s) |
| 0xb8 | `room_size` | **Room Size** | | 0.2 | 0..1 | 1 | percent, 1 decimal |
| 0xc0 | `room_damp` | **Damping** | "Color" | 0.5 | 0..1 | 2 | percent |
| 0xc8 | `room_mod` | **Modulation** | | 0.2 | 0..1 | 3 | percent |
| 0xd0 | `room_diff` | **Diffusion** | | 0.8 | 0..1 | 4 | percent |
| 0xd8 | `room_pre_delay` | **Pre Delay** | | 0 | 0..1 | 5 | `FloatLinearConverter(0, 250, "%.1f ms")`: `250·v` ms |
| 0xe0 | `room_high_cut` | **High Cut** | | 0 | 0..1 | 6 | `FloatLinearConverter(20, 2, "%.1f kHz")`: `20 − 18·v` kHz |
| 0xe8 | `room_low_shelf` | **Low Shelf** | "EQ" | 0 | 0..1 | 7 | `FloatLinearConverter(0, −12, "%.1f dB")`: `−12·v` dB |
| 0xf0 | `hall_rev_time` | **Reverb Time** | | 0.4 | 0..1 | 9 | as Room |
| 0xf8 | `hall_size` | **Room Size** | | 0.1 | 0..1 | 10 | percent |
| 0x100 | `hall_damp` | **Damping** | "Color" | 0.5 | 0..1 | 11 | percent |
| 0x108 | `hall_mod` | **Modulation** | | 0.4 | 0..1 | 12 | percent |
| 0x110 | `hall_diff` | **Softness** | | 0.9 | 0..1 | 13 | percent |
| 0x118 | `hall_pre_delay` | **Pre Delay** | | 0 | 0..1 | 14 | `250·v` ms |
| 0x120 | `hall_high_cut` | **High Cut** | | 0 | 0..1 | 15 | `20 − 18·v` kHz |
| 0x128 | `hall_low_shelf` | **Low Shelf** | "EQ" | 0 | 0..1 | 16 | `−12·v` dB |
| 0x130 | `plate_pre_delay` | **Pre Delay** | | 0 | 0..1 | 0x12 | `StringUnits::msToString(250·v)`: "0.00 ms" below 0.005, `"%.2f ms"` below 1, `"%.1f ms"` |
| 0x138 | `plate_decay` | **Decay** | | 0.3 | 0..1 | 0x13 | percent |
| 0x140 | `mix` | **Mix** | "Output" | 0.5 | 0..1 | 0x14 | percent (shared by all modes) |
| 0x148 | `plate_low_shelf` | **Low Shelf** | "Color" | 0 | 0..1 | 0x15 | percent |
| 0x150 | `plate_high_damp` | **Hi Damp** | | 0 | 0..1 | 0x16 | percent |
| 0x158 | `wet_gain` | **Wet Gain** | | 3.5 | 0.1..4 | 0x17 | none; not automatable, on no page |

### 1.3 Serialization (`serialize<portable_binary_iarchive>`, 0x101a5bdf0)

Version 0 stores only the Plate fields: `0x130, 0x138, 0x140, 0x148, 0x150`; on load Mode is
set to **2 (Plate)** and Wet Gain to **1.0**. Version ≥ 1 continues with `0xa8` (Mode), `0x158`
(Wet Gain), then `0xb0 … 0x128` **[exact]**. This is the `|d, v|` block in `module.rs`; the
renamed fields are `mode` (was `unknown_a8`), `wet_gain` (`unknown_158`) and `hall_low_shelf`
(`unknown_tail`). DECKARD treats a parameter list without `mode` as version 0.

### 1.4 Knob layout (`getParameterByPosition`, 0x101a5e5ac)

Positions 0–7 are page "Main", 8–15 page "EQ"; the layout depends on Mode **[exact]**:

| pos | Room | Hall | Plate |
|---|---|---|---|
| 0 | Mode | Mode | Mode |
| 1 | Reverb Time | Reverb Time | Decay |
| 2 | Room Size | Room Size | – |
| 3 | Damping | Damping | – |
| 4 | Modulation | Modulation | Low Shelf |
| 5 | Diffusion | Softness | Hi Damp |
| 6 | Pre Delay | Pre Delay | Pre Delay |
| 7 | Mix | Mix | Mix |
| 8 (EQ) | Low Shelf | Low Shelf | – |
| 9 (EQ) | High Cut | High Cut | – |

Section headings: pos 0 "Room" (set once on the Mode parameter), Damping / Plate Low Shelf
"Color", Room/Hall Low Shelf "EQ", Mix "Output".

---

## 2. Processor (`RT::PlateReverbProcessor`)

- **Constructor** (0x101a5ea20): `PlateReverb` (0x163500 bytes), `GaloisReverb(false)` (Room),
  `GaloisReverb(true)` (Hall); each gets `onInit()` and `onSetSampleRate(44100, 128)`.
- **`processParameter(idx, v)`** (0x101a5da9c): idx 0–7 → Room `setParameter(idx)`, 9–16 →
  Hall `setParameter(idx − 9)`; 0x12 → Plate 0 (pre-delay), 0x13 → Plate 1 (decay), 0x14 (Mix)
  → Room 8, Hall 8 and Plate 7, 0x15 → Plate 9 (low shelf), 0x16 → Plate 10 (hi damp), 0x17 →
  Plate 8 (wet gain) **[exact]**. Every engine gets its parameters whatever the mode.
- **`setMode(m)`** (0x101a5da60): stores the mode; the chosen engine is cleared (Plate:
  `onReset()`; Room/Hall: the same clearing as `GaloisReverb::onReset`, inlined).
- **`onSamplerateChanged(sr)`**: `onSetSampleRate(sr, 128)` on all three. **`resume()`**:
  Plate `onReset`, Hall and Room `onReset`, Room and Hall `onResume`, Plate `onResume`.
- **`process()`** (0x101a5ed88): runs only the selected engine, in place, in chunks of at most
  128 samples: a counter (shared by the modes, starts at 0) calls the engine's
  `processControlRate()` whenever it reaches 0 and is reset to 128.

DECKARD reproduces the parameter order of `DATA::PlateReverb::connectParameters` (Mode first,
then Plate, Mix, Wet Gain, Room, Hall), then `onSamplerateChanged` and `resume` on the first
`set()`. Later `set()` calls send only changed fields.

---

## 3. Room and Hall (`NI::DSP::GaloisReverb`)

### 3.1 Parameters and smoothed values

`setParameter(idx, v)` (0x102608b5c) stores `v` and sets a target in one of two
`SmoothingToolbox`es **[exact]**:

| idx | target (toolbox A, 11 lanes) |
|---|---|
| 0 Reverb Time | `T = expf(v·3.7)·0.5` (s) → A[1]; `gain = sqrt(1 − exp10f(−4/T))·0.5` → A[0] |
| 1 Size | `f = fma(v, 3, 1)`; line lengths (§3.2) → toolbox B; `A[6] = len[1] − len[0]` |
| 2 Damping | `powf(v, 0.25)` → A[2] |
| 3 Modulation | `v·10` → A[3] (LFO depth in samples) |
| 4 Diffusion | `v·0.5` → A[4] |
| 5 Pre Delay | `(v·0.25)·44100` → A[5] (samples; 250 ms at 44.1 kHz) |
| 6 High Cut | `fma(1 − v, 19000, 2000)` Hz → A[7] |
| 7 Low Shelf | `v·−12` dB → A[8] |
| 8 Mix | `v` → A[9] |
| 9 Width | `v` → A[10] (not sent by Maschine: stays 1.0) |

`exp10f` is libm's `__exp10f`; `expf`, `powf`, `logf`, `exp` (f64) and `__sincos(f)_stret`
are the system's, so the port calls the same functions on macOS.

Before the first `onResume` (flag `+0x8`) every `setParameter` ends in
`setVarsToFinalSmoothingVals()` (§3.5): values jump. Afterwards they ramp (§3.6).

### 3.2 Line lengths

31 lines. A permutation `perm` starts as the identity and gets swaps `(k, s[k])`: Room
`s = [0,1,7,8,20,21]`, Hall `s = [0,1,6,7]`. With per-line offsets
`off = [4,3,5,5,5,5,2,1,1,1,4,5,1,5,1,4,3,4,2,4,1,2,2,5,4,2,3,1,3,0,3]` **[exact]**:

- Room: `idx = int(fma(f·10, perm[k], 90) + off[k])`
- Hall: `idx = int(fma((f·23)·32, powf(perm[k]·0.03125, 1.61), 90) + off[k])`

`len[k] = prime[idx]`, the table `GaloisReverb::PrimeNums::prime_nums` (0x10c34dd3c; the first
3041 primes from 2, generated in the port). Line 0 is always 499 samples. The lines' buffers
(and their power-of-two masks) are sized once in the constructor for `f = 4` (largest size).

### 3.3 Fixed matrices (constructor, `initFDNoutputMatrix`)

- **Feedback** `M` (32×32, column-major, lane 31 unused): `M[r][j] = v[(j − r) mod 31]` with
  `v` the sequence `AABAAABABABBBBABBABAABBAAAAABBB`, `A = 0xbe503656` (−0.20333),
  `B = 0x3e19d390` (0.15022) **[exact]**, verified against an oracle dump (§6.2).
- **Input** (32×2): left feeds lines 0, 2, 4 (+1), 6 (−1), 10 (−1), 14, 18 (+1), 22, 26, 28
  (−1); right feeds 1, 3, 5 (+1), 7, 11, 15, 19, 23 (−1), 27, 29 (+1). Hall clears lines 4–7
  (to 0 / −0).
- **Output** (2×32): Room taps lines 0, 4, 16, 24 (+L), 3, 8, 12, 20 (−L), 1, 5, 17, 25 (+R),
  2, 9, 13, 21 (−R). Hall blends two matrices by Diffusion `d` (A[4]):
  `O = 2d·O_b + (1 − 2d)·O_a`, `O_a` taps (±1) lines 0–3, 6–9, 12/13, 16/17, 20/21, 24/25, and
  `O_b` the same taps scaled by 0.447 (`0x3ee4f92e`, first four) and 1.126 (`0x3f900f2b`).

### 3.4 Per-sample processing (`processAudio<2>` Room, `<3>` Hall; stereo, no ramp)

1. **LFO**, f64 phasor rotated by `4π/sr` (2 Hz): `re' = fma(c, re, −(s·im)) + 0`,
   `im' = fma(s, re, c·im)`; `(s, c) = __sincos_stret(4π/sr)`, reset to (1, 0) by
   `onSetSampleRate`. `mre = A[3]·(f32)re`, `mim = A[3]·(f32)im`.
2. **Pre-delays** (2 × 16384): write input, read with linear interpolation
   `fma(x[n], 1 − f, f·x[n+1])` at `A[5] + A[6]` (left) and `A[5]` (right) samples.
3. **Feedback**: `y = M·x` as sequential fused dot products (Eigen's NEON kernel: `acc =
   fma(M[r][j], x[j], acc)`, j = 0…31, then `+ 0`).
4. **Injection** `inj[i] = fma(In_R[i], pdR, In_L[i]·pdL)`.
5. **Lengths**: `dlen[k] = (k odd ? mre : mim) + B.cur[k]` for k = 1…30; `dlen[0] = 499`.
6. **Lines** k = 0…30: write `y[k]` at `w`; add `inj[k]` at `(w − len[0] + 50) & mask`; read
   with linear interpolation at `dlen[k] − 1` → `x[k]`; `w = (w + 1) & mask`.
7. **Damping** (32 lanes): `x = a·x + b·s; s = x` (§3.5).
8. **Output**: `o = O·x` (fused dot products), then per channel:
   - **diffusers** `AllPassNested<3>` (Room 4, Hall 2): three delay lines z0..z2 (lengths Room
     [31,79,41], [43,73,53], [59,61,67], [71,47,37]; Hall [73,181,97], [103,179,127]),
     `u0 = fma(a20, z2, fma(a10, z1, a00·z0)) + b0·x` (same for u1),
     `u2 = a02·z0 + (a12·z1 + a22·z2) + x·b2`, each written to its line and read back
     `len − 1` samples later; output `fma(cz, z0_old, x·cx)`. Coefficients from Diffusion
     (`setDiffuserCoeffs`, §3.5);
   - **low shelf** `NI::DSP::EQ` (Simper SVF): `v3 = s − ic2`, `v1 = fma(a1, ic1, v3·a2)`,
     `v2 = ic2 + fma(a2, ic1, v3·a3)`, `ic = 2v − ic`, `y = fma(m2, v2, fma(m0, s, v1·m1))`;
   - **high cut** one-pole `y = fma(c, state − y, y)`.
9. **Mix**: `(sn, cs) = __sincosf_stret(A[9]·π·0.5)`, `g = A[0]·sn`, `mid = (oL + oR)·0.5`,
   `out = fma(cs, in, g·fma(o − mid, A[10], mid))`.

Above 48 kHz Maschine resamples (§3.7).

### 3.5 Coefficients (`setVarsToFinalSmoothingVals`, 0x102607054)

- High cut: `c = (f32) exp((f64)(A[7]·−2)·π / 44100)`.
- Shelf: `EQ::setParams(200 Hz, Q 1, A[8] dB, 44100, LowShelf)`: `A = exp10f(dB/40)`,
  `w = (f32)(200π/44100)`, `g = fma(w⁵, 0.1333, fma(w³, 1/3, w))/sqrt(A)`,
  `a1 = 1/fma(g, 1 + g, 1)`, `a2 = g·a1`, `a3 = g·a2`, `m = [1, A − 1, fma(A, A, −1)]`.
- Diffusion `d = A[4]`: `f2 = −0.9d`, `f6 = fma(f2, d, 1)`, `f7 = f2·d`, `f8 = 0.9d`,
  `f5 = fma(−d, f8, 1)`, `f9 = −d·f8`. Even stages: columns `[f9,f9,f2]`, `[f5,f9,f2]`,
  `[0,f5,f2]`, `b = [d, d, 1]`, `cz = f5`, `cx = d`; odd stages: `[f7,f7,f8]`, `[f6,f7,f8]`,
  `[0,f6,f8]`, `b = [−d, −d, 1]`, `cz = f6`, `cx = −d`. Hall also re-blends `O` (§3.3).
- Two-band T60 (`T = A[1]`, `D = A[2]`): `L = logf(2T)·0.27027`,
  `T_hi = T·fma(L, fma(D, −0.95, 1), fma(D, −0.5, 1)·(1 − L))`, `g = exp10f(−3/(T·44100))`,
  `ρ = exp10f(−3/(T_hi·44100))/g`. Per line: `a = powf(g, len)`, `r = 1/powf(ρ, len)²`,
  `b = r == 1 ? 0 : ((−0.48137906 − r) + sqrt(fma(r, 2.962758, −0.7682742)))/(1 − r)`, then
  `a ·= 1 − b`.

The constants 44100 are fixed: at 48 kHz decay times and corner frequencies scale by 48/44.1, in
Maschine as well.

### 3.6 Ramps (`SmoothingToolbox`, `processControlRate`, `processAudio<6>/<7>`)

Toolbox A (12 lanes) ramps over `round(sr·0.05/128)` ticks, B (lengths, 32 lanes) over
`round(sr·0.2/128)`. `startSmooth`: `start = cur`, `end = target`, `delta = end − start`; a lane
whose `|delta|` is below one ulp of `|cur|` jumps. Per sample (while either toolbox ramps):
`count += 1`, `cur = start + delta·(count·1/(steps·128))`; then high cut when `delta[7] ≠ 0`,
shelf when `delta[8] ≠ 0`, every 8th sample of the call diffusion when `delta[4] ≠ 0`, every
32nd the full T60 when `delta[1]` or `delta[2] ≠ 0`, else the line damping when B ramps. At the
end of a ramp the values jump to the end without recomputing coefficients **[exact]**
(state machine 2 → 3 → 4 → 0, as in the binary; see `Ramp` in `galois.rs`).

### 3.7 Rates above 48 kHz **[not ported]**

`onSetSampleRate` sets an internal rate `sr/k`, `k = ⌊sr/44100⌋`, and wraps the engine in two
`NI::AB::SampleRateConverter`s. DECKARD does not port the converter: it runs the network at
`sr/k` on averages of `k` input frames and interpolates the output linearly. The decay matches
(96 kHz impulse: energy envelope within 0.1–0.6 dB of Maschine) but the signal does not null.

### 3.8 State not cleared by `onReset`

The LFO phase, the shelf's and high cut's states and all write indices survive `onReset` and
`setMode`; only the buffers, the network vectors and the diffusers' states are cleared.

---

## 4. Plate (`NI::DSP::PlateReverb`)

### 4.1 Parameters (`setParameter`, 0x10262bc00)

| idx | target / effect |
|---|---|
| 0 Pre Delay | `fma(v, 250, 0)` ms → ramp lane 0 |
| 1 Decay | lane 1 = v; T60s from the **current** lane value (below) |
| 7 Mix | lane 2 |
| 8 Wet Gain | stored (`+0xc0`), no ramp |
| 9 Low Shelf | lane 3 = `1 − v`; shelf from the current lane |
| 10 Hi Damp | lane 4 = `1 − v`; high shelf from the current lane |
| 13 Width | lane 5 (not sent by Maschine: 1.0) |

Coefficients use the *current* (not target) lane, so a call computes with the previous value
**[exact]**; the processor's `onSamplerateChanged` call re-applies 0, 1, 9, 10 with current ==
target, which makes the start consistent. Before `onResume` targets are copied to the current
values at once.

- Decay `d`: T60s (ms) `[fma(6400, d, 556), fma(1070, d, 173), fma(2993, d, 1007)]` (mid, low,
  high band) and crossover `fma(757, 1 − d, 968)` Hz; input high shelf 1 gain
  `exp10f(fma(1 − d, 3, 0)/20)`.
- Low shelf `x`: gain `exp10f(fma(x, 34.375, −28.75)/20)` at 160 Hz.
- Hi damp `x`: crossover `fma(x, 4851, 6000)` Hz, high shelf 2 gain `exp10f(fma(x, 4, −4)/20)`
  at 5.6 kHz.

### 4.2 Input chain

Mono `x = 0.5·((0 + inL) + inR)`, 20 one-sample allpasses (`v = fma(s, 0.9, x)`,
`x = fma(v, −0.9, s)`, `s = v`), then four first-order filters as a 4-lane pipeline (one sample
per stage): high shelf 1 (1 kHz, `K = 1.6596`), low shelf (160 Hz), a 7.5 kHz lowpass
(`tan(π·7500/sr)` bilinear) and high shelf 2 (5.6 kHz); `X' = [Y1, Y2, Y3, x·gain]`,
`Y' = (X' + a·X) − Y·b`. Each shelf: `(g, a, b)` from `compute*FilterCoeffs` (sinf of
`w/2 ∓ π/4`, `w = min(f/sr, 1)·π`). Quirk: `onReset` sets each filter's own coefficients to
unity but keeps the packed ones; the next coefficient change mixes them **[exact]**.

### 4.3 Early reflections

The pipeline output is written to a 24000-sample buffer; 32 taps at `int(sr·t_k)` (table
`EARLY_SECS`) feed the lines, a second set 5 ms later, weighted by `0.8913·w_k`, sums into a
direct 4-lane early signal.

### 4.4 Network

16 lines of `int(sr·t) − 1` samples (21.1 … 55.1 ms). Per sample: each line's output goes
through its damping biquad, `u = 0.25·(y + ±early)`, a 16-point Hadamard transform (exact sum
order in `hadamard16`), written back. The damping biquad per line is two first-order shelves
(crossovers from the Decay crossover and Hi Damp) whose gains come from the three T60s through
`powe_0p02prec` (a cubic 2^x) and the `sin_rx5` polynomial (`generateDampingFiltersForAllDelays`).

### 4.5 Output

`wet = (Σ lines + early)` → stereo through a "width" matrix of `sin_rx5`/`cos_rx5` of
`[0, 0, π/2, π/2]` (width 1), into a 96100-sample pre-delay per channel (read
`max(1, ms·sr/1000)` samples back). The dry signal goes through the same matrix.
`out = fma(mix·wetgain·0.04217, wet_delayed, (1 − mix)·0.50118·dry)`.

### 4.6 Ramps

`processControlRate` (every 128 samples) starts 8-lane linear ramps over
`round(sr·0.02/128)` ticks (lanes whose step² < 1e−15 jump). While ramping, every sample
recomputes the affected coefficients (decay, low shelf, hi damp, pre-delay index, width). At
the end, values jump to the targets without recomputing **[exact]**.

---

## 5. Implementation notes

- The port follows the binary's operation order; `mul_add` exactly where it has `fmadd`/`fmla`;
  vector code lane by lane. Matrix products are sequential fused dot products.
- libm: `__exp10f`, `__sincos_stret` and `__sincosf_stret` are called directly on macOS
  (`f32::powf/exp/ln/sin/tan`, `f64::exp` map to the same libSystem functions). Elsewhere the
  port falls back to Rust's functions and is not bit-exact.
- Only the selected engine runs; parameter changes to the others wait (their ramps run when
  they are selected, as in Maschine).
- Shared code with the other reverb ports: none. Metaverb, Iceverb and Reverb (legacy) are
  Reaktor-Core kernels; Galois and Plate are separate `NI::DSP` classes.

## 6. Null tests

### 6.1 Against Maschine (`maschine-ref`)

Source: `/Users/Shared/Chromatic Fire Library/Projects/Dev Traps.mxprj`, group 12, sound 0
("Reverb room", slot 0) unless noted, `maschine-ref make … 12 0 0 [field=value …]`, rendered
with `--preroll 0` at −12 dBFS, compared with `fx-null` on the captured input. −240 dB means
bit-identical (peak difference 0).

| case | input | sr | settings | residual L / R |
|---|---|---|---|---|
| Room, project values | noise 3 s | 48 k | mode 0 | −240 / −240 |
| Room | noise 4 s | 44.1 k | mode 0 | −240 / −240 |
| Room | impulse 8 s | 44.1 k | mode 0 | −240 / −240 |
| Room min | noise 4 s | 44.1 k | all 0, mix 0 | −240 / −240 |
| Room max | noise 4 s | 48 k | all 1, mix 1 | −240 / −240 |
| Room mid | noise 4 s | 44.1 k | 0.63/0.47/0.21/0.77/0.35/0.4/0.6/0.5, mix 0.7 | −240 / −240 |
| Room | sine 440 4 s | 48 k | mode 0 | −240 / −240 |
| Room long tail | impulse 12 s | 48 k | time 1, size 1, mix 1 | −240 / −240 |
| Hall | noise 4 s | 48 k | mode 1 | −240 / −240 |
| Hall, project values (sound 12/1) | noise 4 s | 48 k | | −240 / −240 |
| Hall | impulse 8 s | 44.1 k | mode 1 | −240 / −240 (see below) |
| Hall min | noise 4 s | 48 k | all 0, mix 0 | −240 / −240 |
| Hall max | noise 4 s | 44.1 k | all 1, mix 1 | −240 / −240 |
| Hall mid | noise 4 s | 48 k | 0.81/0.66/0.13/0.29/0.55/0.2/0.3/0.9, mix 0.35 | −240 / −240 |
| Hall | sweep 4 s | 44.1 k | mode 1 | −240 / −240 |
| Hall long tail | impulse 12 s | 44.1 k | time 1, size 1, mix 1 | −240 / −240 |
| Plate | noise 4 s | 48 k | mode 2 | −240 / −240 |
| Plate | noise 4 s | 44.1 k | mode 2 | −240 / −240 |
| Plate | impulse 8 s | 48 k | mode 2 | −240 / −240 |
| Plate max | noise 4 s | 44.1 k | decay/pre/shelf/damp/mix 1, wet gain 4 | −240 / −240 |
| Plate mid | noise 4 s | 48 k | 0.77/0.3/0.5/0.6, mix 0.25, wet gain 0.1 | −240 / −240 |
| Plate min | noise 4 s | 44.1 k | decay 0, mix 0 | −240 / −240 |
| Plate | sweep / sine 1 k | 44.1 / 48 k | | −240 / −240 |
| Plate long tail | impulse 12 s | 48 k | decay 1, mix 1 | −240 / −240 |
| Plate | noise 3 s | 96 k | mode 2 | −240 / −240 |
| Room / Hall | noise 3 s | 96 k | | ≈ −18 dB (resampler not ported, §3.7) |

Hall impulse: Maschine is itself not deterministic below the smallest normal float: of two
identical renders, one equals DECKARD bit for bit, the other differs in 24 samples by values
under 1e−34 (some intermediate subnormals are flushed, apparently depending on the worker
thread's FPCR). Normal-range output is identical.

### 6.2 Against an oracle (parameter changes)

`maschine-ref` cannot automate, so ramps were checked against Maschine's own engine code: a
scratch harness (`scratchpad/fx-plate_reverb/oracle/oracle2.c`) loads the Maschine 3 VST3
binary, constructs `NI::DSP::GaloisReverb` / `NI::DSP::PlateReverb` and calls their methods
(`setParameter`, `processControlRate` every 128 samples, `processAudio`) exactly as the
processor does. The oracle itself nulls bit-exactly against `maschine-ref` renders. Two seconds
of noise then silence, changes at block boundaries; all bit-identical (0 differing samples):

| engine | changes |
|---|---|
| Room | mix 0.9; low shelf 0.8; high cut 0.7; diffusion 0.3; time 0.7; damping 0.9; mod 0.9; pre-delay 0.5; size 0.8; size 0.8 then time 0.1 |
| Hall | diffusion 0.2; size 0.9 then 0.0; time 1 and damping 0 one block apart; diffusion/high cut/low shelf one block apart; mod 0 + pre-delay 1 |
| Plate | decay 0.9; pre-delay 0.8; mix 0.1; low shelf 0.9; hi damp 0.95; wet gain 0.5; decay 0.9 → 0.1 one block apart, then low shelf 0 |
