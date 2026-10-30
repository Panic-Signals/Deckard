# Maschine's EQ effect: parameter and DSP specification

A clean-room description of Maschine 3's built-in **EQ** insert effect. The chain is
`NI::MASCHINE::DATA::EQ` (parameters, persistence) → `NI::MASCHINE::RT::EQProcessor`
(real-time wrapper) → `NI::MASCHINE::FXCORE::GBXEQCore` (the DSP kernel). It was written
from Ghidra output and objdump listings of the arm64 slice (`analysis/maschine_arm64`).
Addresses are arm64 VM addresses.

Decompiled sources (each directory has an `INDEX.tsv` mapping file → address → symbol):

| dir | contents |
|---|---|
| `out/decomp/eq/` | `DATA::EQ::EQ()` (0001), `getProcessor` (0002), `EQProcessor::EQProcessor` (0003), `processFloatParameter` (0004), `onSamplerateChanged` (0005), `resume` (0006), `process` (0007). 0008/0009 are `NI::DSP::EQ`, a different, unrelated class (not used by this module). |
| `out/decomp/eq2/` | `GBXEQCore::beginInit` (0001), `endInit` (0002), `processAudio` (0003), `processEventIn*` (0004–0014), `FloatExponentialConverter` (0015–0019), `FloatLinearConverter` (0020–0023), `__GLOBAL__sub_I_Modules_Effects_EQ_ALL.cpp` (0024: page names, module info, converters) |
| `out/decomp/eq3/` | `EQ::getParameterByPosition` (0001), `SmoothingToolbox<12>::startSmooth` (0002), `EQ::serialize<iarchive>` (0003), plus objdump listings `processAudio.s`, `init.s`, `processEventIn.s`, `startSmooth.s`, `exptable-init.s` |

**Confidence tags.** **[exact]**: formula and constants read directly from code or
assembly. **[inferred]**: a reasonable reading of partial evidence. **[unknown]**: not
recovered.

Notation: all arithmetic is IEEE `f32` unless stated. `fma(a,b,c)` = fused `a·b+c` with one
rounding (AArch64 `fmadd`). It must be used where marked if you want bit-exact output.
`π/2` = `0x3fc90fdb`, `2π` = `0x40c90fdb` (`6.2831855`).

---

## 1. Parameters

### 1.1 Stored parameters (constructor order)

`DATA::EQ::EQ()` (0x101a0a134, `eq/0001.c`) creates 11 `MaschineNumericParameter<float>`
with `TransactionObjectContainer::insert(label, default, min, max)`. Each one is bound with
`setAndCallFuncAutomate` to `EQProcessor::processFloatParameter(idx, …)` using the index in
the "RT idx" column. **[exact]**

| # | member | mxprj field (`module.rs`) | label | section (setSectionName) | default (normalized) | range | RT idx | display converter |
|---|---|---|---|---|---|---|---|---|
| 1 | +0xa8 | `unknown_a8` | Freq | **Low** | `convertFrom(248 Hz)` = 0.420214 | 0..1 | 0 | `FloatExponentialConverter(20, 8000, "%.1f Hz")` @0x10e305ea8 |
| 2 | +0xb0 | `unknown_b0` | Gain | – | 0 | −1..1 (bipolar) | 1 | `FloatScaleDisplayConverter(20, "%.1f dB")` @`__MergedGlobals` |
| 3 | +0xb8 | `unknown_b8` | Freq | **Low-Mid** | `convertFrom(630 Hz)` = 0.460128 | 0..1 | 2 | `FloatExponentialConverter(40, 16000, "%.1f Hz")` @0x10e305ee8 |
| 4 | +0xc0 | `unknown_c0` | Gain | – | 0 | −1..1 (bipolar) | 3 | ×20 "%.1f dB" |
| 5 | +0xc8 | `unknown_c8` | Width | **Low-Mid** (section span 2) | 0.5 | 0..1 | 8 | `FloatLinearConverter(0.1, 4.0, "%.1f oc")` @0x10e305e78 |
| 6 | +0xd0 | `unknown_d0` | Freq | **High-Mid** | `convertFrom(1970 Hz)` = 0.650410 | 0..1 | 4 | Exp(40, 16000) @0x10e305ee8 |
| 7 | +0xd8 | `unknown_d8` | Gain | – | 0 | −1..1 (bipolar) | 5 | ×20 "%.1f dB" |
| 8 | +0xe0 | `unknown_e0` | Width | **High-Mid** (section span 2) | 0.5 | 0..1 | 9 | Linear(0.1, 4.0, "%.1f oc") |
| 9 | +0xe8 | `unknown_e8` | Freq | **High** | `convertFrom(2610 Hz)` = 0.660119 | 0..1 | 6 | `FloatExponentialConverter(50, 20000, "%.1f Hz")` @0x10e305f28 |
| 10 | +0xf0 | `unknown_f0` | Gain | – | 0 | −1..1 (bipolar) | 7 | ×20 "%.1f dB" |
| 11 | +0xf8 | `unknown_tail` | Gain | **Output** | 0 | −1..1 (bipolar) | 10 | ×20 "%.1f dB" |

- Labels are wide strings: 0x104d28774 = "Freq", 0x104d28788 = "Gain", 0x104d2879c =
  "Width". Section names are inline wide literals in the constructor: "Low", "Low-Mid",
  "High-Mid", "High", "Output". **[exact]**
- All four Gains and Output call `setBipolar(true)` and `setFuncProcessWheel(
  processWheelDeltaGeneric, 0.5, 0.1)`. The two bound floats (0.5, 0.1) are presumably the
  encoder coarse/fine step factors. **[exact]** call / **[inferred]** meaning.
- Width params call `MaschineParameter::setSectionSpan(2)` (Ghidra mislabels the call as
  `SequenceProcessor::setWorkspace`; objdump shows `bl 0x1002dc474 setSectionSpan`). **[exact]**
- No enums. Every parameter is a float.
- The normalized defaults above are from float32 re-evaluation of `convertFrom`
  (§1.3). They round-trip through `convertTo` to 247.99997, 629.99988, 1970.0004 and 2610.0 Hz.
- **Serialization order** (`EQ::serialize`, `eq3/0003.c`): a8, b0, b8, c0, d0, d8, e8, f0,
  c8, e0, f8. That is RT-idx order 0..10, and it matches `module.rs` exactly, where
  `unknown_tail` = +0xf8 = Output. **[exact]**

Suggested names for `module.rs`: a8 `low_freq`, b0 `low_gain`, b8 `low_mid_freq`, c0
`low_mid_gain`, d0 `high_mid_freq`, d8 `high_mid_gain`, e8 `high_freq`, f0 `high_gain`, c8
`low_mid_width`, e0 `high_mid_width`, tail `output_gain`. The Qt model
`GUI::qml_module::EQModel` uses the getter names LowFreq, LowGain, Mid1Freq, Mid1Gain,
Mid1Width, Mid2Freq, Mid2Gain, Mid2Width, HighFreq, HighGain and OutputGain.

### 1.2 Pages

`__GLOBAL__sub_I_Modules_Effects_EQ_ALL.cpp` (`eq2/0024.c`) sets
`EQ::s_aPageNames` = **"Freq / Gain"**, **"Width / Output"**, and sets the module name to "EQ"
(`s_aModuleInfo`). **[exact]**

`Module::getParameter(group, page, slot)` (0x1002dc83c) tail-calls the virtual
`getParameterByPosition(page·8 + slot)`. `EQ::getParameterByPosition` (0x101a0b040,
`eq3/0001.c`) is a jump table (bytes at 0x104d2868b) **[exact]**:

| page | slot 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |
|---|---|---|---|---|---|---|---|---|
| Freq / Gain | Low Freq | Low Gain | Low-Mid Freq | Low-Mid Gain | High-Mid Freq | High-Mid Gain | High Freq | High Gain |
| Width / Output | (empty) | (empty) | Low-Mid Width | (empty) | (empty) | High-Mid Width | (empty) | Output Gain |

Section headings: page 1 shows "Low" over slots 1–2, "Low-Mid" over 3–4, "High-Mid" over
5–6 and "High" over 7–8. A Gain has no section of its own, so it continues the heading of
the slot before it. On page 2, "Low-Mid" (span 2) sits over slot 3, "High-Mid" (span 2)
over slot 6, and "Output" over slot 8. The exact span rendering is **[inferred]**.

### 1.3 Value → internal conversion

`EQProcessor::processFloatParameter(idx, _, v)` (0x101a0addc, `eq/0004.c`) stores the
"engineering" value into the core's parameter block. It then calls the matching
`GBXEQCore::processEventIn*` logic (`eq2/0004–0014.c`, `eq3/processEventIn.s`), which
computes a smoothing target. **[exact]**

| RT idx | core param (offset) | engineering value | smoothing target (offset) |
|---|---|---|---|
| 0 Low Freq | `LoF` (+0x00) | `f = convertTo_LF(v)` | `wLo = max((f·2π)·(1/sr), 1e-5)` (+0x270) |
| 1 Low Gain | `LoG` (+0x04) | `dB = v·20` | `gLo = E(fma(dB·0.25, K, 1200))` (+0x278) |
| 2 Low-Mid Freq | `M1F` (+0x08) | `f = convertTo_MF(v)` | `wM1` (+0x274), same formula |
| 3 Low-Mid Gain | `M1G` (+0x10) | `dB = v·20` | `gM1 = E(fma(dB·0.5, K, 1200))` (+0x27c) |
| 8 Low-Mid Width | `M1W` (+0x0c) | `v` | `bwM1 = fma(v, 3.9, 0.1)` octaves (+0x290) |
| 4 High-Mid Freq | `M2F` (+0x14) | `convertTo_MF(v)` | `wM2` (+0x280) |
| 5 High-Mid Gain | `M2G` (+0x1c) | `v·20` | `gM2 = E(fma(dB·0.5, K, 1200))` (+0x288) |
| 9 High-Mid Width | `M2W` (+0x18) | `v` | `bwM2 = fma(v, 3.9, 0.1)` (+0x294) |
| 6 High Freq | `HiF` (+0x20) | `convertTo_HF(v)` | `wHi` (+0x284) |
| 7 High Gain | `HiG` (+0x24) | `v·20` | `gHi = E(fma(dB·0.25, K, 1200))` (+0x28c) |
| 10 Output | `Lvl` (+0x28) | `v·20` | `gOut = E(fma(dB, K, 1200))` (+0x298) |

`K = 9.965784` (`0x411f73da`) = 3/log10(2), and `E(x)` is the exp-table lookup (§2.1).
So `E(fma(dB·k, K, 1200)) ≈ 10^(dB·k/20)`:

- `gLo`, `gHi` = 10^(dB/80) = √A_RBJ (the shelf code squares it).
- `gM1`, `gM2` = 10^(dB/40) = A_RBJ.
- `gOut` = 10^(dB/20), linear output gain.
- Gain range is therefore ±20 dB for every band and for Output.

Frequency converters (`FloatExponentialConverter(min, max)`, `eq2/0015,0017,0018.c`)
**[exact]**:

```
base = 10;  lmin = log10(min) (f64 log10, stored f32);  range = (f32)log10(max) − lmin
convertTo(v)   = clamp(powf(10, lmin + range·v), min, max)          // min·(max/min)^v
convertFrom(h) = h > 0 ? clamp((log10f(h)/log10f(10) − lmin)/range, 0, 1) : clamp(h,0,1)
```

The RT converters `EQProcessor::s_LFConverter` / `s_MFConverter` / `s_HFConverter` are
built with the same (20, 8000), (40, 16000), (50, 20000) ranges. They are separate objects
from the UI ones (format "%.2f"). **[exact]**

Width: `FloatLinearConverter(0.1, 4.0)` → `0.1 + 3.9·v` octaves, so the default 0.5 gives
2.05 oct. The RT path uses `fma(v, 3.9f, 0.1f)` (`processEventInM1W`). **[exact]**

Display strings: the frequency display prints `"%.1f Hz"` of `convertTo(v)`, never kHz,
and values below `-0.5/10^precision` that are negative print as 0. **[exact]**

---

## 2. DSP

### 2.1 Exp table `E(x)` (NI::AB::ExpTable)

`m_fExpTable` (0x10e2ee580, zero-filled `__common`) is filled at start-up by
`__GLOBAL__sub_I_unity_0_cxx.cxx` @0x102668b44 (`eq3/exptable-init.s`). **[exact]**

```
struct Entry { f32 val; f32 slope; }  table[2401]   // 8-byte stride
table[0].val = 2^-20 (0x35800000)
for i in 1..=2400: table[i].val = (f32) exp2((f64)i / 60.0 − 20.0)       // libm exp2 in double
for i in 0..2400:  table[i].slope = table[i+1].val − table[i].val          // f32 subtract
table[2400].slope = 0
```

Lookup, identical in every caller (e.g. `processEventInLoG`, `processAudio.s` 0x101a1c934) **[exact]**:

```
fn E(x: f32) -> f32 {
    let n = x as i32;                 // fcvtzs: truncate toward zero
    if n >= 2400 { return table[2400].val }        // 2^20
    if n < 0     { return 0.0 }
    fma(x - n as f32, table[n].slope, table[n].val)
}
```

`E(x) ≈ 2^((x−1200)/60)`. (For −1 < x < 0, `n` = 0 and the code interpolates with a
negative fraction.)

### 2.2 Signal flow

`EQProcessor::process` (0x101a0b358, `eq/0007.c`) **[exact]**:

- The processor is stereo-in/stereo-out (`setStereoInStereoOut`) and processes in place.
- It does nothing unless input pin 0 exists and its channel count equals
  `getAudioInputChannelsPerPin(0)` (2).
- Buffers are planar: `L = buf[i]`, `R = buf[i + stride]`.
- If the buffer's "needs clear" flag (+0x10) is set, the buffer is zeroed first, the flag
  is cleared, and processing still runs, so filter tails ring out.
- Then, for each frame: `core.inL = L; core.inR = R; core.processAudio(); L = core.outL; R = core.outR`.

The processing is per sample, with no block processing and no oversampling.

`GBXEQCore::processAudio` (0x101a1c568, `eq2/0003.c`, `eq3/processAudio.s`) runs one
sample. It is a 4-lane SIMD pair of biquad stages **[exact]**:

```
x   = [inL·gOut_s, inR·gOut_s, d_L, d_R]          // d = Low-Mid output of the PREVIOUS sample
st1 = biquad4(x,   [LowShelf, LowShelf, HighShelf, HighShelf])
st2 = biquad4(st1, [Peak1,    Peak1,    Peak2,     Peak2   ])
d   = st2[0..2]                                   // stored at +0x360
out = st2[2..4]                                   // +0x40/+0x44
```

The effective series chain per channel is:

`in → ×Output gain → Low shelf → Low-Mid peak → z⁻¹ → High shelf → High-Mid peak → out`

This gives **1 sample of latency**, a by-product of packing the four filters into two
4-wide stages. The output gain is applied **before** the filters. **[exact]**

Both channels use identical coefficients. There is no M/S mode and no channel linking
logic. **[exact]**

### 2.3 Biquad structure (both stages)

The structure is transposed direct form II with **unnormalized b** and a/a0-normalized
feedback. Per lane, with `s1, s2` state (stage 1 at +0x2e0/+0x2f0, stage 2 at +0x300/+0x310).
All ops are plain mul/add, not fused (`processAudio.s` 0x101a1cec0–0x101a1cf48) **[exact]**:

```
v   = s1 + b0·x                 // stage 2 computes b0·x + s1 (same value; fadd is commutative)
s1' = s2 + (b1·x − (a1/a0)·v)
s2' = b2·x − (a2/a0)·v
y   = v · inv_a0
```

Here `a1/a0` and `a2/a0` are precomputed as `a1·inv_a0` and `a2·inv_a0`. This is
algebraically TDF-II with the state scaled by a0.

### 2.4 Coefficients

All bands use the plain digital ω = 2π·f/sr (clamped ≥ 1e-5). There is **no prewarping**.
sin/cos come from a range-reduced Taylor polynomial, not libm **[exact]**:

```
x  = (w > π/2) ? (π/2 − (w + (−π/2))) : w            // ≈ π − w
x2 = x·x
P  = fma(x2·(−1/12), fma(x2, −1/30, 1), 1)            // consts 0xbdaaaaab, 0xbd088889
Q  = fma(x2·(−0.05), fma(x2, −1/42, 1), 1)            // 0xbd4ccccd, 0xbcc30c30
sin = x · fma(x2·(−1/6), Q, 1)                       // 0xbe2aaaab
cos = (w > π/2) ? −fma(x2·(−0.5), P, 1) : fma(x2·(−0.5), P, 1)   // fnmadd / fmadd
```

(For the peak bands, the "w" used in the bandwidth term is `w' = π/2 − ((π − w) − π/2)` when
w > π/2, which is w after a float round trip, else w.)

**Low shelf** (current `wLo`, `gLo`; recomputed when either changed, or on the first sample
after init). This is RBJ low shelf with `A = gLo²`, shelf slope **S = 0.5**
(α = sin/2·√(A + 1/A + 2)) **[exact]** (`processAudio.s` 0x101a1c69c–0x101a1c808, decoded
lane by lane):

```
A  = g·g
Kt = g · ((sqrt((A + 2) + 1/A) · (sin·0.5)) · −2)      // = −2·√A·α
a0 = (A+1) + (A−1)·cos − Kt      a2 = (A+1) + (A−1)·cos + Kt
b0 = A·((A+1) − (A−1)·cos − Kt)  b2 = A·((A+1) − (A−1)·cos + Kt)
b1 = 2A·((A−1) − (A+1)·cos)      a1 = −2·((A−1) + (A+1)·cos)
inv_a0 = 1/a0
```

**High shelf** (`wHi`, `gHi`) is RBJ high shelf, S = 0.5, with the same Kt (`eq2/0003.c`,
the branch writing +0xf0..+0x10c) **[exact]**:

```
a0 = (A+1) − (A−1)·cos − Kt      a2 = (A+1) − (A−1)·cos + Kt
b0 = A·((A+1) + (A−1)·cos − Kt)  b2 = A·((A+1) + (A−1)·cos + Kt)
a1 = 2·((A−1) − (A+1)·cos)       b1 = −2A·((A−1) + (A+1)·cos)
inv_a0 = 1/a0
```

**Peaks** (Low-Mid: `wM1`, `gM1`, `bwM1`; High-Mid: `wM2`, `gM2`, `bwM2`; recomputed when
w, g or bw changed). This is RBJ peaking EQ with bandwidth in octaves, `A = g`, and a
clamp on α (`processAudio.s` 0x101a1c848–0x101a1ca30) **[exact]**:

```
y     = ((bw · w') · 0.3465736) / sin            // 0x3eb17218 = ln2/2
ep    = E(fma(y,  86.5617, 1200))                // 0x42ad1f97 = 60/ln2 → e^y
em    = E(fma(y, −86.5617, 1200))                //                    → e^−y
alpha = (sin·0.5) · (ep − em)                    // sin(w)·sinh(ln2/2·bw·w/sin w)
alpha = (alpha < bw) ? alpha : bw                // NOTE: clamp α ≤ bw
a0 = 1 + alpha/A    a2 = 1 + (−alpha/A)
b0 = 1 + A·alpha    b2 = 1 + (−(A·alpha))
a1 = b1 = cos·(−2)
inv_a0 = A / (A + alpha)                         // not 1/a0
```

**Coefficient packing.** When either shelf changed, stage-1 arrays are rebuilt as
`[Lo, Lo, Hi, Hi]` lanes: inv_a0, a1·inv_a0, a2·inv_a0, b0, b1, b2. When either peak
changed, stage 2 is rebuilt the same way. Cached "last" values are at +0x1d0..+0x1f4 and
are compared with `!=` on the smoothed values. **[exact]**

### 2.5 Parameter smoothing

`NI::AB::SmoothingToolbox<11>` sits at core+0x200 and holds:

- `cur[11]` at +0x210
- `inc[]` at +0x240
- `tgt[]` at +0x270

The lane order is wLo, wM1, gLo, gM1, wM2, wHi, gM2, gHi, bwM1, bwM2, gOut. Smoothing is
**linear in the internal domain**: ω in radians (not log-frequency) and linear gain.
**[exact]**

- `endInit` (`eq2/0002.c`, `init.s`):
  - `steps = max(1, (int)(sr·0.02/128 + 0.5))` (`fcvtzs`, i.e. truncate)
  - `invLen = 1/(steps·128)`
  - block = 128 (+0x36c)
  - `cur = tgt` (no ramp at start)

  At 44.1 k this gives 7 blocks = 896 samples (20.3 ms); at 48 k, 8 = 1024; at 96 k, 15 = 1920.
- Each parameter event sets the target, a dirty flag (+0x2a0) and state 1.
- In `processAudio`, every sample **first** does `cur += inc` (all lanes), then increments
  the sample counter. At each 128-sample boundary (counter ≥ 128 → 0), or on the first
  sample after init, one of two things happens:
  - **dirty**: clear dirty, `startSmooth()`.
  - **otherwise**: `remaining -= 1`; at 0, snap `cur = tgt`, `inc = 0`.
- `startSmooth` (`eq3/startSmooth.s`, shared `SmoothingToolbox<12>` code):
  - `inc = (tgt − cur)·invLen` per lane.
  - If `inc² < 1e-15` (compared in double), that lane snaps to target with inc 0.
  - If Σinc² > 0, then `remaining = steps` and active = 1.
- Coefficients are then computed from `cur` in the same sample. So a parameter change is
  picked up only at the next 128-sample boundary, and then ramps over `steps·128` samples.

### 2.6 Init / reset / sample rate

- `onSamplerateChanged(sr)` only stores sr (`eq/0005.c`).
- `resume()` (`eq/0006.c`) saves the 11 engineering values and calls
  `beginInit(sr)`. That call zeroes all filter states, the `d` delay, the smoother and the
  counters, and sets the first-sample flag (+0x2c0). `resume()` then restores the values,
  marks every parameter dirty, and recomputes all targets and `cur = tgt` (inline
  `endInit`). **[exact]**
- `EQProcessor::EQProcessor` (`eq/0003.c`) allocates the 0x400-byte, 128-aligned core.
  It runs `beginInit(44100)` / `endInit()`. Its own default values (248/630/1967/2610 Hz,
  width 0.5) are overwritten immediately, because `setAndCallFuncAutomate` pushes the
  DATA defaults through `processFloatParameter`. **[exact]**

### 2.7 Bypass, denormals, gain staging

- **Gain staging**: the output gain (±20 dB) multiplies the input before any filter. There
  is no auto-gain, no saturation and no clipping. **[exact]**
- **Bypass**: `EQProcessor` and `GBXEQCore` have no bypass path. Module on/off is handled
  by the generic module/processor-chain code, which is not covered here. **[unknown]** (out of scope)
- **Denormals**: there is no flush or DC-offset trick in the EQ code. It relies on the
  host's FTZ setting, if any. **[inferred]**
- **Flat bands**: at 0 dB all bands reduce to b = a (unity), but they still run, so the
  output is the input delayed by 1 sample, with float rounding. **[exact]** (from the formulas)

---

## 3. Implementation notes / ambiguities

1. **Bit-exactness.** Use `f32` throughout. Use `mul_add` exactly where marked: the
   polynomial sin/cos, the E-table index `fma(…, K or ±86.5617, 1200)`, the E-table
   interpolation, and `width = fma(v, 3.9, 0.1)`. Everything else is unfused. The table
   values need f64 `exp2` (macOS libm). Rust's `f64::exp2` should agree to ≤1 ulp before
   the f32 cast, but this is unverified.
2. **The 1-sample latency between Low-Mid and High** is real and must be kept for
   sample-accurate output. It also means the High shelf/High-Mid see the Low-Mid output one
   sample late. With all bands flat, the EQ is a pure 1-sample delay.
3. **α clamp** in the peaks (`alpha = min(alpha, bw)`) is unusual, but it is in the
   assembly (`fcmp s4,s2; fcsel mi`). It only matters for wide bands at high ω.
4. **No ω upper clamp.** At sample rates where max frequency > Nyquist (e.g. High Freq
   20 kHz at 32 kHz), `w > π` and the reduced `x = π − w` goes negative. Reproduce as is.
5. **Frequency smoothing is linear in ω**, which is audible as a non-log sweep on long jumps.
6. The order of the ω products differs slightly between `endInit/resume`
   (`(1/sr)·f·2π` in the Ghidra text) and the event handlers (`(f·2π)·(1/sr)`). Float
   rounding can differ by 1 ulp. Use the handler order for live changes and check
   `init.s` for the init order.
7. Page-2 section-span rendering, and the meaning of the `processWheelDeltaGeneric`
   (0.5, 0.1) arguments, are **[inferred]**.
8. Hopper (via REA) could not be opened in this session: the bridge timed out. The dense
   SIMD section was cross-checked against the raw objdump listing instead. The low-shelf
   and peak coefficient blocks and the filter loop were decoded lane by lane from
   `processAudio.s`. The high-shelf block relies on the Ghidra text, which matched the
   other two blocks exactly.
