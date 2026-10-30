# Maschine's Compressor: parameters and DSP

A clean-room description of Maschine 3's built-in **Compressor** effect
(`NI::MASCHINE::DATA::Compressor` → `NI::MASCHINE::RT::CompressorProcessor` →
`NI::MASCHINE::FXCORE::CompressorCore` ("Classic") and
`NI::MASCHINE::FXCORE::FBCompressorCore` ("Feedback"), with
`NI::MASCHINE::FXCORE::SidechainFilterCore` on the side-chain input).

It was written from Ghidra output in `out/decomp/comp/` (`INDEX.tsv` maps files to
functions). The dense loops (`CompressorCore::processAudio`, `FBCompressorCore::processAudio`,
`FBCompressorCore::onInit`, `SidechainFilterCore::calcFilterCoeffs`, and the exp-table
builder) were checked against arm64 disassembly, saved as `out/decomp/comp/asm-*.s`. Ghidra
loses SIMD lanes and the arguments of libm calls in those functions; where the two disagree,
this spec follows the assembly. Hopper (REA bridge) returned "unreadable result" during this
session, so it was not used for the cross-check. Addresses are arm64 VM addresses in
`analysis/maschine_arm64`.

**Confidence.** **[exact]**: formula and constants read from code or assembly.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation:
- `v`: a parameter's normalized value (0..1) as stored in the project.
- `sr`: sample rate. `d = 1.0e-18` (`0x219392ef`) is the Feedback core's denormal offset.
- All arithmetic is `f32` unless the text says `f64`.
- `E(x)`: the shared exponential lookup table (§3.1). `dB→lin(x) = E(x·9.965784 + 1200)`.
  `exp(x) ≈ E(x·86.5617 + 1200)`.

---

## 1. Parameters

### 1.1 Stored fields, in constructor order

The constructor is `Compressor::Compressor()` at 0x101a05a38 (`0039.c`). Each parameter is
created by `TransactionObjectContainer::insert(label, default, min, max)` **[exact]**. Labels
are wide-string literals (Mode 0x104d27b3c … Gain 0x104d27be4).

| obj off | label | type | default (norm) | range | display converter | section |
|---|---|---|---|---|---|---|
| 0xa8 | **Mode** | EnumParameter | 0 | `Classic`, `Feedback` | – | "Mode". Not automatable; changing it forces a display update |
| 0xb0 | **Threshold** | float | 1/6 (`0x3e2aaaab`) | 0..1 | `FloatLinearConverter(0, −60, "%.1f dB")`: `−60·v` dB (default −10 dB) | "Depth" |
| 0xb8 | **Knee** | float | 0.5 | 0..1 | `ScaleNormalizedToPercentOneDecimal` | (Depth) |
| 0xc0 | **Amount** | float | 0.75 | 0..1 | `ScaleNormalizedToPercentOneDecimal` | (Depth) |
| 0xc8 | **Attack** | float | `FloatPowerConverter::convertFrom(0.25)` = (0.25/8)^(1/3) ≈ 0.31498 | 0..1 | `FloatPowerConverter(0, 8, 3, "%.2f ms")`: `8·v³` ms (default 0.25 ms) | "Time" |
| 0xd0 | **Release** | float | 0.5 | 0..1 | `FloatExponentialConverter(40, 251, 1.1220185, …, "%.1f ms")`: `clamp(1.1220185^(16v+32), 40, 251)` ms (default 100 ms) | (Time) |
| 0xd8 | **Gain** | float | 0.9166667 (`0x3f6aaaab`) | 0..1 | `FloatLinearConverter(−36, 0, "%.1f dB")`: `36·v − 36` dB (default −3 dB) | "Output" |
| 0xe0 | (side-chain) | `SideChainFilter` | see §1.3 | | | |

`FloatPowerConverter::convertTo(v) = min + (max−min)·v^exp` and
`convertFrom(x) = clamp(((x−min)/(max−min))^(1/exp), 0, 1)` (0x1024ae17c / 0x1024ae1a4)
**[exact]**. The Release display matches `CompressorCore::setRelease` exactly. The order of
the middle three `FloatExponentialConverter` arguments is garbled in Ghidra **[inferred]**.

Encoder behaviour (`setFuncProcessWheel`): Threshold uses `processWheelDeltaGeneric` with
steps (−0.5, −0.1), so turning right lowers the threshold; Attack uses fine-only 0.01; Release
uses fine-only 0.1; Gain uses `processWheelDeltaGeneric` with (0.5, 0.1) **[exact]**.

Module info: name "Compressor", `s_aModuleInfo` id 8. Pages (`s_aPageNames`):
**"Main"** and **"Side-Chain Input"** **[exact]** (`0064.c`).

`getParameterByPosition` (0x101a06910) gives the page layout **[exact]**:

| pos | page | parameter |
|---|---|---|
| 0 | Main | Mode |
| 1 | Main | Threshold |
| 2 | Main | Knee |
| 3 | Main | Amount |
| 4 | Main | Attack |
| 5 | Main | Release |
| 6 | Main | Gain |
| 7 | Main | – (empty) |
| 8 | Side-Chain Input | Source |
| 9 | Side-Chain Input | Gain (SC) |
| 10 | Side-Chain Input | Filter |
| 11 | Side-Chain Input | Center Freq |
| 12 | Side-Chain Input | Width |

### 1.2 mxprj serialization order and field mapping

`Compressor::serialize<portable_binary_iarchive>` (0x101a05800, `0035.c`) reads, in order:
Module base, then **Mode** (only when class version ≠ 0), then Threshold, **Amount**,
**Knee**, Attack, Release, Gain, and SideChainFilter **[exact]**. This matches the order in
`crates/mxprj/src/types/module.rs:356`. The fields map as follows:

| module.rs field | offset | label |
|---|---|---|
| unknown_a8 (version ≥ 1) | 0xa8 | Mode (0 Classic, 1 Feedback) |
| unknown_b0 | 0xb0 | Threshold |
| unknown_c0 | 0xc0 | Amount |
| unknown_b8 | 0xb8 | Knee |
| unknown_c8 | 0xc8 | Attack |
| unknown_d0 | 0xd0 | Release |
| unknown_d8 | 0xd8 | Gain |
| unknown_tail | 0xe0 | SideChainFilter (§1.3) |

Version-0 files have no Mode field, so Mode takes its default of 0 (Classic).

### 1.3 SideChainFilter (`NI::MASCHINE::DATA::SideChainFilter`, shared by several FX)

The constructor is at 0x10184755c (`0004.c`, checked against the asm). The serialization
order is the same as the insertion order (`0001.c`), and maps to module.rs:1005
`SideChainFilter` fields `unknown_0, unknown_8, unknown_10, unknown_18, unknown_tail`.

| field | label | type | default | range | DSP mapping (SidechainFilterCore) | section |
|---|---|---|---|---|---|---|
| unknown_0 | **Source** | `SideChainInputParameter` (value type `SideChainInputParameterValue`) | default-constructed | routing choice | none; selects what feeds input pin 1 | "Input". Not automatable |
| unknown_8 | **Gain** | float | **1.0** | 0..1 | `setParamGain`: `dB→lin(40v − 20)`, so −20..+20 dB | (Input) |
| unknown_10 | **Center Freq** | float | 0.5 | 0..1 | `fc(n) = E((n·119.58942 + 15.48682)·5 + 1381.8816)` = **20·1000ⁿ Hz** (20 Hz..20 kHz; 632 Hz at 0.5) | (Filter) |
| unknown_18 | **Width** | float | 0.5 | 0..1 | % display; see §4 | (Filter) |
| unknown_tail | **Filter** | bool | false | | `setParamFilterOn` | "Filter" |

The Center Freq display (`CenterFreqDisplayConverter::getValueAsString`, 0x101847d60) prints
`"%.2f Hz"` below 10 Hz, `"%.1f Hz"` below 1000 Hz, `"%.1f kHz"` (÷1000) above that, and
"0.00 Hz" when |f| < 0.005 **[exact]**.

The default SC Gain of 1.0 means **+20 dB** under `setParamGain`'s formula (asm 0x1018475b4
stores 1.0 as the default) **[exact]**. This looks surprising, but it is what the code does.
See §6.

### 1.4 Parameter routing to the processor

`setAndCallFuncAutomate` binds each float to
`CompressorProcessor::processFloatParameter(tFloatParameters idx, …, v)` with
**Threshold 0, Amount 1, Knee 2, Attack 3, Release 4, Gain 5** **[exact]**. `0039.c` binds
fields 0xb0→0, 0xc0→1, 0xb8→2, 0xc8→3, 0xd0→4, 0xd8→5. Mode is bound to `setMode(idx)`,
which stores the index at processor+0xa0.

`processFloatParameter` (0x101a066fc) calls the matching `CompressorCore::setX(v)` and then
tail-calls `FBCompressorCore::setParameter(idx, v)` (0x101a113dc). **Both cores always
receive every parameter**, whatever the mode **[exact]**.

---

## 2. Processor (`CompressorProcessor`)

`CompressorProcessor::CompressorProcessor` (0x101a06a80) **[exact]**:
- It has 2 audio input pins × 2 channels (pin 0 is main, pin 1 is side-chain) and 1 output
  pin × 2 channels.
- It owns a `CompressorCore` (0x180 bytes) and an `FBCompressorCore` (0x200 bytes). It holds
  a reference to the `SidechainFilterCore` owned by the module's `SideChainFilter`.
- It calls `FB.onInit()` and then `FB.onSetSampleRate(44100, 128)`.

`onSamplerateChanged(sr)` (0x101a06cc8) calls `Classic.setSampleRate(sr)`,
`FB.onSetSampleRate(sr, 128)`, and `SidechainFilterCore::setSampleRate(sr)` (inlined).

`resume()` calls `Classic.resume()`, `FB.onReset()`, `FB.onResume()` and `SCF.resume()`. It
zeroes the meters (+0xc0) and the FB control-rate counter (+0xc8). `suspend()` calls
`Classic.suspend()` and `FB.onSuspend()`.

`process()` (0x101a06d84) **[exact]** unless marked:
1. Main L/R are `in[0]`, `in[0]+stride`, processed in place. **Side-chain is active** when
   input pin 1 exists and its channel count is > 1. In that case SC L/R are pin 1's buffers;
   otherwise the "SC" pointers alias the main L/R buffers.
2. `Classic.setSidechain(active)` and `FB.setParameter(8, active ? 1 : 0)` are called every
   block.
3. When SC is active, `SidechainFilterCore::processAudio(scL, scR, n)` filters and scales the
   SC buffers in place (§4).
4. Input meter: `getLevelVecStereo(mainL, mainR, n)` is taken **before** processing.
5. **Mode 0 (Classic)**, per sample: the core's `in = (mainL, mainR)` and
   `sc = (scL, scR)`; then `CompressorCore::processAudio()`; then `mainL/R = core.out`.
6. **Mode ≠ 0 (Feedback)**: the block is split at a 128-sample control-rate grid that is
   continuous across host blocks (counter at +0xc8, reset to 0 by `resume`, so the first
   sample triggers a tick). At each grid point `FB.processControlRate()` runs. Each chunk is
   processed by `FB.processAudio({mainL, mainR, scL, scR}, {mainL, mainR}, len)`.
7. Meters at +0xc0/+0xc4 are peak-hold with decay. `decay = (1 − 4·info[+0x10])^n` (the field
   is probably 1/sr, **[inferred]**), and `meter = max(meter·decay + 1e-36, level)`. +0xc0 is
   the input meter (`getInputMeterValue`) and +0xc4 the output meter (`getOutputMeterValue`)
   **[inferred]**: Ghidra lost the second return lane.
8. GR meter (`Compressor::getGRMeterValue`, 0x101a06a58):
   `clamp(20·log10(g), −60, 0) / −60`, where `g` is Classic+0x12c (Classic) or FB+0x50
   (Feedback). It is 0 while suspended.

Switching modes only changes +0xa0. Neither core is reset, so the inactive core's state
stays frozen and resumes when it is selected again **[exact]**. There is **no dry/wet mix**
in either mode.

---

## 3. Shared primitives

### 3.1 `NI::AB::ExpTable::m_fExpTable` (0x10e2ee580)

It is built by `__GLOBAL__sub_I_unity_0_cxx.cxx` (0x102668b44, `asm-102668b44.s`) **[exact]**:

```
val[i]   = (f32) exp2( (f64)i / 60.0 − 20.0 )   for i = 0..=2400   (val[0] stored as 2^-20 = 0x35800000)
slope[i] = val[i+1] − val[i]   (f32 subtraction),   slope[2400] = 0
memory: interleaved pairs { val[i], slope[i] } at 0x10e2ee580 + 8·i
```

Lookup, used everywhere:

```
E(x): i = (i32) x            // fcvtzs: truncation toward zero
      if i >= 2400 → val[2400] (= 2^20)
      if i < 0     → 0.0
      else         → val[i] + slope[i]·(x − (f32)i)
```

So `E(1200 + 60·log2(y)) ≈ y`. The constants are `9.965784 = 60/(20·log10 2)` (dB) and
`86.5617 = 60/ln 2` (natural log). Note that x ∈ (−1, 0) truncates to i = 0 and extrapolates
with slope[0].

### 3.2 Parameter smoothing (both cores and the SC filter)

All three cores use the same scheme **[exact]**:
- Every core keeps `target[]`, `current[]` and `inc[]`. A setter writes the target and raises
  a "dirty" flag.
- Control ticks come every 128 samples. On the first tick after dirty:
  `inc = (target − current)·rampInv`. For any lane with `inc² < 1e-15` (compared in f64),
  `current = target` and `inc = 0`. If any lane still moves, a countdown `N` starts;
  otherwise state = 4.
- `current += inc` every sample.
- When the countdown hits 0 on a later tick, `current = target` and `inc = 0`.
- `N = max(1, (i32)(sr·0.02/128 + 0.5))` and `rampInv = 1/(N·128)`. This gives a **linear
  ramp of ≈ 20 ms, quantized to whole 128-sample ticks**.

---

## 4. Side-chain filter (`SidechainFilterCore`)

**[exact]** from `calcFilterCoeffs` (0x101a207cc, asm), `setSampleRate` (0x101a20770),
`setParamCenterFreq` / `setParamWidth` (0x101a20bd0 / 0x101a20c04), and `processAudio`
(inlined in `0011.c`):

- `hiN = clamp(center + width/2, 0, 1)` and `loN = clamp(center − width/2, 0, 1)`.
- `f_hi = fc(hiN)` and `f_lo = fc(loN)` (§1.3). For each, `w = 2π·f/sr`, `(s, c) = sincosf(w)`,
  `α = s·0.70710677` (Q = 1/√2), and `a0inv = 1/(1 + α)`.
- LP at f_hi: `A1h = 2c_hi·a0inv_hi`, `A2h = −(1 − α_hi)·a0inv_hi`,
  `b0LP = (1 − c_hi)·0.5·a0inv_hi`.
- HP at f_lo: `A1l = 2c_lo·a0inv_lo`, `A2l = −(1 − α_lo)·a0inv_lo`,
  `b0HP = (1 + c_lo)·0.5·a0inv_lo`.
- Scale target (both channels):
  - Filter off: `G = gainLin`.
  - Filter on: `G = (E(width·−20·0.70710677·86.5617 + 1200) + 1)·gainLin·b0LP·b0HP`, i.e.
    `(1 + e^(−14.142·width))·gainLin·b0LP·b0HP`.
- `G` is smoothed per §3.2 (target +0x80/0x84, current +0x60/0x64). A second smoothed pair at
  +0x88/0x8c stays at 1.0 and acts as the post-LP gain.

Per sample, per channel:

```
filter off:  sc = sc·G
filter on:   u   = sc·G
             lp  = u + 2·u1 + u2 + A1h·lp1 + A2h·lp2          (u1,u2 / lp1,lp2: previous inputs / outputs)
             hin = lp1·post                                   (HP input = LP output of the PREVIOUS sample)
             hp  = hin + (−2)·hin1 + hin2 + A1l·hp1 + A2l·hp2
             sc  = hp
```

The result is a 2nd-order Butterworth LP (f_hi) feeding a 2nd-order Butterworth HP (f_lo),
with the numerator b0's folded into G. The HP input is the LP output one sample late, so the
filtered SC has one extra sample of delay **[exact per Ghidra]**.

`resume()` clears the biquad states and snaps current = target.

---

## 5. DSP

### 5.1 Mode "Classic" (`CompressorCore`)

#### Setters
All at 0x101a18e98..0x101a18fd0 **[exact]**. Each sets dirty (+0xe0) and state 1.

| param | stored | target lane |
|---|---|---|
| Threshold | `T = −60·v` dB | +0xc0 |
| Amount | `S = v·(2 − v)` (compression slope, 0..1; ratio = 1/(1 − S)) | +0xd0 |
| Knee | `K2 = v·0.5` | +0xd4 |
| Attack | `Ta = max(8·v³, 1e-10)` ms; lanes +0xc4 = Ta, +0xc8 = 1/Ta | |
| Release | `Tr = clamp(powf(1.1220185, 16v + 32), 40, 251)` ms; lane +0xcc = 1/Tr | |
| Gain | `Gdb = (v − 1)·36` dB | +0xd8 |
| setSidechain | flags +0x1c and +0x6c, applied immediately with no smoothing | |

`setSampleRate` stores sr and 1/sr. `resume()` (0x101a18d68) does the following:
- copies the parameters into target and current;
- sets `N` and `rampInv` (§3.2);
- clears the in/sc/out/delay registers (+0x28..+0x47) and all detector states
  (+0x108..+0x130 = 0);
- sets the sample counters to (129, 5), so the first sample runs a control tick and a latch.

#### Per sample
`processAudio()`, 0x101a19064, `asm-101a19064.s`. All **[exact]** from asm.

1. **Smoothing**: `current += inc` (8 lanes). Tick counter +0x144: tick when the incremented
   count ≥ 128 (§3.2).
2. **Latch**: counter +0x140 holds the pre-increment value `k`. When `k ≥ 4`, latch and reset
   the counter to 0, which makes it **every 5 samples** (k: 0,1,2,3,4→latch). The latched
   block is: `T` (+0x50), `Ta` (+0x54), `1/Ta` (+0x58), `1/Tr` (+0x5c), `K2` (+0x60),
   `S` (+0x64), `Gdb` (+0x68), all taken from current.
3. **Detector input**: `(xL, xR) = sidechain ? sc : in`, and
   `p = max(|xL|, |xR|)` (stereo-linked peak).
4. **Overshoot (dB)**: `thrLin = dB→lin(T)`.
   - If `p > thrLin`: `over = fastlog2(p)·6.0205302 − T`, where
     `m = bits(p) & 0x807fffff | 0x3f800000` as float, `m −= 1`, `e = exponent(p) − 127`, and
     `fastlog2 = m·(m·(−0.34624699) + 1.34625006) + e`.
   - Otherwise `over = 0`.
5. **Ballistics** on `env` (+0x108), in the dB-overshoot domain:
   - `inv = (over > env) ? 1/Ta : 1/Tr`
   - `q = inv·(1/sr)·1000`
   - `coef = (q < 1) ? q·0.693147 : 0.693147`
   - `env_new = over·coef + (1 − coef)·env + 1e-30`

   The old `env` is still used in step 6.
6. **Soft knee** (quadratic, entirely above threshold):
   - `Wraw = −(K2·T)`, stored to +0x120.
   - If the **previous** `Wraw` ≤ 0: `y = env_old` (hard knee) and `W = 0.001`.
   - Otherwise: `W = max(Wraw, 0.001)` and `x = env_old·Winv_prev`, where `Winv_prev` (+0x110)
     is the reciprocal of the previous sample's W. Then:
     - `x > 1`: `y = (2x − 1)·(W_prev/2)` (= env − W/2)
     - `1e-16 < x ≤ 1`: `y = x·x·(W_prev/2)` (= env²/(2W))
     - else `y = 0`
   - Store `env_new`, `Wraw` and `W` (+0x124).
7. **Gain-reduction meter value**: `grLin = E(S·y·(−9.965784) + 1200)`, stored to +0x130. The
   previous +0x130 is kept for step 10.
8. **Output gain (dB)**, auto makeup included:
   ```
   s  = (Ta > 4) ? 1 : Ta·0.25 ;  h = s(2−s) ;  h2 = h(2−h)
   Mdb = Gdb + h2·(S·18·0.016666668·T) + (S·0.96)·(K2·0.5·T − T) − S·y
   G   = dB→lin(Mdb)            → +0x128
   ```
   T is negative, so the 0.96 term adds makeup and the `h2` term (fast attacks only)
   subtracts some.
9. **Output safety limiter**:
   - `pk = max(|D_L|, |D_R|)`, where `D` (+0x118) is the delayed, gained sample from the
     previous call.
   - `lim = (pk > lim) ? pk : lim + (pk − lim)·coefA + 1e-30`. `coefA` is built like step 5
     but always from **1/Ta**, so the limiter releases at the attack time.
   - Store `lim` (+0x10c), then `c = clamp(lim, 1, 1000)`.
   - Reciprocals of `(c, c, W)` use `frecpe` plus two `frecps` Newton steps (NEON estimate,
     not IEEE division). The W lane becomes +0x110.
10. **Outputs**:
    - `D_new = prevIn·G`, where `prevIn` (+0x38) is the main input of the previous call. Then
      `prevIn = in`.
    - `out = D_old·(1/c)`.
    - Meter +0x12c = `grLin_prev·(1/c)`.

Signal latency is **2 samples**: `out[n] = in[n−2]·G[n−1]/c[n]`. The detector at sample n sets
the gain for `in[n−1]`, so the gain effectively leads the audio by one sample. This latency
is not compensated anywhere visible.

`getGainMeter` (0x101a18ffc) returns `clamp(20·log10(+0x12c), −60, 0)/−60`.

### 5.2 Mode "Feedback" (`FBCompressorCore`)

This is a feedback (output-sensing) design. The gain law is applied to the gained level and
iterated 4× per sample. RC-style time constants come from analog component values
(C = 0.22 µF).

#### Coefficients
From `setParameter` (0x101a113dc; Ghidra shows it inside `0042.c`), `onInit` (0x101a10bc8,
asm) and `onSetSampleRate` (0x101a10fdc). All **[exact]**.

| param | computation | smoothed target lane |
|---|---|---|
| Threshold | `T = −60·v` (+0x7c); `thrInv = E(−T·9.965784 + 1200)` = 10^(−T/20) | +0x164 |
| Amount | `x = v·19 + 1 − 1`; `R = 1 + 2.3974712·x + 6.456394e-05·powf(x, 4)` (+0x6c; 1 at v=0, 37.83 at 0.75, 54.97 at 1) | +0x14c |
| makeup (Threshold or Amount) | `Mk = E((T/R − T)·0.5·9.965784 + 1200)` = 10^((T/R−T)/40) | +0x144 |
| Knee | `k = 4.5·v + 0.5` dB (+0x68); `kH = dB→lin(k)` (+0x64); `kL = dB→lin(−k)` (+0x74); +0x70 = −k | kL → +0x140, kH → +0x150 |
| knee shape (Knee or Amount) | `L = logf(kH)`; `c = (f32)((f64)(1−R) / (pow((f64)L, 3)·4))`; `e = expf(L·(1−R)·0.25)`; `sel = (c < −0.1) ? −0.011512913/c : 0.11512913`; `kLx = kL·expf(powf(sel, 0.25))` | c → +0x154, e → +0x158, kLx → +0x148 |
| Attack | `τa = (v³·49530 + 470)·2.2e-7` s (0.1034..11.0 ms); `atk = 1 − E(−0.25/(τa·sr)·86.5617 + 1200)` | +0x15c |
| Release | `r = clamp(E((16v + 32)·9.965784 + 1200), 40, ·)`; `xr = (r ≤ 251) ? (r − 40)/211 : 1`; `τr = (xr·1320909.1 + 270000)·2.2e-7` s (59.4..350 ms); `rel = 1 − E(−0.25/(τr·sr)·86.5617 + 1200)` | +0x160 |
| Gain | `gLin = dB→lin(36v − 36)` | +0x168 |
| on/off | 1 on resume and init, 0 on suspend | +0x16c |
| param 7 | auto-makeup flag +0x3c. **Never sent**; the constructor sets it to 1, so makeup is always on | – |
| param 8 | side-chain flag +0x3d (immediate) | – |

`kLx` is the level where the knee curve reaches −0.1 dB of gain reduction:
`c·u⁴ = −0.0115 = ln(10^(−0.1/20))`. It is the floor the release pulls toward.

Fixed coefficients (`onSetSampleRate`):
- Averager `cA = cB = 1 − E(−0.25/(sr·0.484)·86.5617 + 1200)` (+0x80/+0x84; τ = 0.484 s). If
  the table index is negative, +0x80 becomes 1.0 instead.
- Output LP (1st order):
  - `t = tanf(atanf(20000/(2sr)))` (≈ 10000/sr)
  - `W = t·251327.42`, `n = 1/(W + 20000)`
  - `b0 = b1 = W·n` (+0x88/0x8c) and `a1 = (W − 20000)·n` (+0x94)
  - This is a bilinear LP with K = 4π·10⁴/sr, so fc ≈ 17.3 kHz at 44.1 kHz and 18.4 kHz at
    48 kHz.
- Detector HP (DC block): `K = tanf(31.415928/sr)` (fc = 10 Hz), `h0 = 1/(K + 1)` (+0xa8),
  `h1 = −h0` (+0xac), `ha = (K − 1)·h0` (+0xb0).

Every coefficient change raises the ramp flag (+0x170, state +0x180 = 1).
`processControlRate()` (0x101a11b68) implements §3.2 on 12 lanes: current +0xe0..+0x10c,
inc +0x110.., target +0x140... If the core is suspended (+0x3e set), `setParameter` copies
targets to current directly.

#### Per sample
`processAudio`, 0x101a11be4, `asm-101a11be4.s`. All **[exact]** from asm.

```
current += inc                                   // 12 lanes
U  = kL·kH                                       // ≈ 1.0 (upper knee edge = threshold); stored +0x60
if R == 1.0:  g = 1; m = A = B = (−k)            // +0x70 value reused as a level (sic)
x  = d + in (L,R)                                // stored +0x40
s  = sidechainFlag ? d + sc : x                  // detector source
hp = h1·s1 + h0·s − ha·hp1 ;  s1 = d + s ; hp1 = d + hp       // per channel, 10 Hz HP
v  = thrInv · max(|hpL|, |hpR|)                  // level relative to threshold (1.0 = threshold)

g, m, A, B ← state (+0x50, +0x54, +0x58, +0x5c)
repeat 4 times:                                  // quarter-sample sub-steps
    p = v·g                                      // detector sees the OUTPUT level (feedback)
    if p >= B:  B = B + atk·(p − B)              // attack: peak follower
    A = d + A + cB·(m − A)
    B = d + B + rel·(kLx − B) + cA·(m − B)       // release toward kLx, slow leak toward m
    m = (A + B)·0.5
    if m <= kL:            g = 1
    elif m <= U:           t = logf(m/kL);  g = E((t²)·(c·t²)·86.5617 + 1200)
    else:                  g = e·E((1−R)·logf(m/U)·86.5617 + 1200)
    g = d + g
store g, m, A, B

y   = b0·(x·g) + b1·xg1 − a1·y1 ;  xg1 = d + x·g ;  y1 = d + y      // per channel
out = y · gLin · on · (makeupFlag ? Mk : 1)                          // stored +0x48/+0x4c
```

Notes:
- The gain law is a quartic soft knee in the log domain between kL (k dB below threshold) and
  threshold, then a constant log slope of (1−R) above it. It is continuous in value and slope
  at U.
- In a feedback loop the steady state is `g = x^(1/R − 1)`, so R acts as the true ratio.
- There is no lookahead and no extra latency beyond the 1st-order LP.
- `getGainMeter` (0x101a11b0c) returns `clamp(20·log10(g), −60, 0)/−60`, or 0 while
  suspended.

---

## 6. Implementation notes and open points

1. **Bit exactness.**
   - Use the `E()` table (§3.1), including f64 `exp2`, f32 slope subtraction and truncating
     conversion.
   - Use libm `logf`/`expf`/`powf`/`sincosf`/`tanf`/`atanf` (Apple libm results may differ in
     the last ulp from Rust's).
   - Use FMA where the asm uses `fmadd`/`fmsub`/`fnmsub`/`fmla`.
   - Classic's `1/c` and `1/W` use NEON `frecpe` + 2×`frecps`. On aarch64 use
     `vrecpeq_f32`/`vrecpsq_f32`; elsewhere emulate the ARM FRECPE table.
2. **Classic latch period is 5 samples**, not 4 (counter compares the pre-increment value).
   Reproduce it as-is.
3. **Classic knee uses the previous sample's env and W** (one-sample stale), and the
   hard-knee decision uses the previous `Wraw`. With Knee = 0 the knee is hard
   (W = 0.001 floor).
4. **Feedback reset quirk**: when Amount = 0 (R exactly 1) the states are re-initialized
   every sample to `g = 1, m = A = B = −k`, a dB value used as a linear level. With R = 1 the
   gain law yields g ≈ 1 anyway.
5. **Side-chain Gain default 1.0 = +20 dB.** It is not certain whether the UI or display maps
   this differently; no display converter is set on that parameter. **[unknown]** whether
   NI intends this; reproduce it.
6. **SideChainFilter::Processor** (0x101847b3c) is a separate processor that runs the same
   `SidechainFilterCore::processAudio`. `CompressorProcessor` also calls it directly on the SC
   buffers. Whether the module graph also inserts `SideChainFilter::Processor` in the
   compressor's SC path is **[unknown]**. If it did, the SC would be filtered twice; the
   Compressor's own path is the one documented here.
7. **Side-chain Source options** (`SideChainInputParameter`) are a routing concern (which
   group or sound feeds pin 1) and are not decoded here **[unknown]**. SC is active only when
   pin 1 has more than 1 channel.
8. Meter decay field `ProcessInfo+0x10` is **[inferred]** to be 1/sr.
9. `FloatExponentialConverter` argument order for the Release display is **[inferred]**. The
   display values equal the DSP's clamped `1.1220185^(16v+32)` ms.
10. The Feedback LP corner (≈17–18 kHz) comes from the odd `tan(atan(20000/2sr))·2π·40000`
    construction, which is probably unintended. Reproduce it as written.
11. The QML editor (`Modules/FX/CompressorEditor.qml`, `CompressorModel`) exposes
    Threshold, Amount, Knee, Attack, Release, Gain, Mode and the input, output and GR meters.
    Its layout was not decoded.
