# Maschine's Freq Shifter: parameters and DSP

A clean-room description of Maschine 3's built-in **Freq Shifter** effect
(`NI::MASCHINE::DATA::FreqShifter` → `NI::MASCHINE::RT::FreqShifterProcessor` →
`NI::MASCHINE::FXCORE::FreqShifterCore`). Like Chorus, `FreqShifterCore` is a compiled
Reaktor-Core kernel: a flat struct of inputs, outputs and state, with `endInit()`,
`processAudio()` (one sample per call), one `processEventIn*()` per parameter, and
`processEventInBfrE()` (sent at the start of each block).

It was written from Ghidra output in `out/decomp/freq_shifter/` (`INDEX.tsv` maps files to
functions) and from the arm64 disassembly of the whole kernel, saved as
`out/decomp/freq_shifter-core.s` (0x101a12300–0x101a13f04: `processAudio`, the event
handlers, `endInit`). Where Ghidra and the assembly differ, this spec follows the assembly.
Addresses are arm64 VM addresses in `analysis/maschine_arm64`. The DECKARD port is
`crates/engine/src/fx/freq_shifter.rs`. It nulls **bit-exactly** against Maschine (§5).

(`NI::DSP::FreqShifterProcessor`, 0x10261ee30, is a different, unrelated processor used by
the performance FX; it is not part of this module.)

**Confidence.** **[exact]**: formula and constants read from code or assembly.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation: `v` is a parameter's stored value (0..1), `sr` the sample rate (`f32` in the
kernel). All arithmetic is `f32` unless marked `f64`, and there are **no fused
multiply-adds**. `rnd(x)` adds ±0.5 (−0.5 when `x < 0`) to `x` **in `f64`** and truncates
(`fcvtzs`), as in Chorus. `tiny = 1e-18` (`0x219392ef`), `tiny_d = 1e-100`.

---

## 1. Parameters

### 1.1 Module

Module name "Freq Shifter" (`s_aModuleInfo`, built in `__GLOBAL__sub_I_Modules_Effects_
FreqShifter_ALL.cpp`, 0x101a31438). One page: `s_aPageNames = { "Main" }` **[exact]**. The
RT processor calls itself "FrequencyShifter".

### 1.2 Parameters in constructor order

`FreqShifter::FreqShifter()` at 0x101a2e870 (`0015.c`, checked against the assembly).
Each is `TransactionObjectContainer::insert(label, default, min, max)` **[exact]**; labels are
wide strings at 0x104d2b880 … 0x104d2b90c.

| obj off | mxprj field | label | section | default | range | RT idx | display converter |
|---|---|---|---|---|---|---|---|
| 0xa8 | `coarse` | **Coarse** | "Freq" | 0.5 | 0..1 | float 0 | `ScaleNormalizedToPercentOneDecimal` |
| 0xb0 | `fine` | **Fine** | (Freq) | 0.0 | 0..1 | float 1 | `ScaleNormalizedToPercentOneDecimal` |
| 0xb8 | `feedback` | **Feedback** | "Output" | 0.75 | 0..1 | float 2 | `ScaleNormalizedToPercentOneDecimal` |
| 0xc0 | `stereo` | **Stereo** | (Output) | 0.75 | 0..1 | float 3 | `ScaleNormalizedToPercentOneDecimal` |
| 0xc8 | `invert` | **Invert** | (Output) | **true** | bool | bool 0 | (on/off) |
| 0xd0 | `mix` | **Mix** | (Output) | 0.75 | 0..1 | float 4 | `ScaleNormalizedToPercentOneDecimal` |

Five `MaschineNumericParameter<float>` and one `MaschineNumericParameter<bool>` (Invert).
There are no enums. Every float displays as `v·100` with one decimal and `%`; Maschine
never shows Hz. Coarse and Fine are stored 0..1 (not bipolar): Fine's default is 0.

### 1.3 Knob layout (`getParameterByPosition`, 0x101a2ef58)

| pos | page | parameter |
|---|---|---|
| 0 | Main | Coarse (section "Freq") |
| 1 | Main | Fine |
| 2 | Main | Feedback (section "Output") |
| 3 | Main | Stereo |
| 4 | Main | Invert (bool; `+0x158` instead of `+0x160`, the bool parameter's base) |
| 5 | Main | Mix |
| 6–7 | Main | – (empty) |

### 1.4 Serialization

`iserializer<…, FreqShifter>::load_object_data` (0x101a2f550) loads the Module base, then
0xa8, 0xb0, 0xb8, 0xc0, 0xc8, 0xd0 in that order **[exact]**. These are the `crates/mxprj`
fields `coarse`, `fine`, `feedback`, `stereo`, `invert`, `mix` (were `unknown_a8…d0`).

### 1.5 Value → engine

Each parameter is bound with `setAndCallFuncAutomate`. `processFloatParameter`
(0x101a2edec) stores `v` into a kernel input and tail-calls the handler;
`processBoolParameter` (0x101a2ee50) stores `(float)b` **[exact]**:

| parameter | kernel input | handler | flags set |
|---|---|---|---|
| Coarse | +0x18 | `processEventInCrs` | 0x7 |
| Fine | +0x20 | `processEventInFine` | 0x7 |
| Feedback | +0x30 | `processEventInFB` | 0x6 |
| Stereo | +0x40 | `processEventInSt` | 0x8 |
| Invert | +0x28 | `processEventInInv` (inlined) | 0x10 |
| Mix | +0x10 | `processEventInMix` | 0x20 |

---

## 2. Processor (`FreqShifterProcessor`)

- **Constructor** (0x101a2f014): stereo in/out; kernel of 0x300 bytes, zeroed, with input
  presets Coarse 0.7008, Feedback 0.78, Stereo 0.89, Invert 1, Mix 1. The stored values
  replace them before audio runs.
- **`onSamplerateChanged(sr)`** stores `sr` (+0x48); used at the next `resume`.
- **`resume()`** (0x101a2f1b0): saves the six inputs, **zeroes the whole kernel**, writes
  `sr`, the inputs and their event flags (1.0), the init flag (+0x50 = 1.0), and runs
  `endInit` (inlined).
- **`process()`** (0x101a2f260): calls **`processEventInBfrE()` once at the start of every
  block**, then per sample `in L → +0xc`, `in R → +0x3c`, `processAudio()`,
  `+0x38 → out L`, `+0x48 → out R` **[exact]**.

The oscillator free-runs from `resume` (phase −0.5), not tied to the transport, so a render
with `--preroll S` must be matched by feeding DECKARD `S·sr` zeros first (§5).

---

## 3. DSP (`FXCORE::FreqShifterCore`)

### 3.1 Hilbert pair

Per channel, the input plus feedback `u = x + fb` goes through two cascades "A" and "B" of six
first-order all-passes. Stage `k` **[exact]**:
```
y_k = s[k] + c_k·(y_{k−1} − s[k+1]);   s[k+1] = y_k + tiny        (y_0 = u)
s[0] = u                                                            (after the cascade)
```
`s[0]` is the cascade's previous input; `s[k]` (k ≥ 1) holds stage k's previous output plus
`tiny`, and doubles as stage k+1's previous input. Coefficients (computed in `endInit`,
shared by both channels) **[exact]**:
```
w   = 47.1239 / sr          (0x423c7ee0 = 15π)
c_k = −(1 − w·K_k) / (w·K_k + 1)
K_A = 1.2524, 5.5671, 22.3423, 89.6271, 364.791, 2770.11
K_B = 0.3609, 2.7412, 11.1573, 44.7581, 179.624, 798.458
```
(exact `f32` bit patterns in the port). So the all-pass break frequencies sit at about
`15·K` Hz: 5.4 Hz … 41.6 kHz, independent of `sr` apart from the bilinear warp.

### 3.2 `endInit()` (0x101a12e74) **[exact]**

```
steps   = rnd(sr / 9800)         (f32 division; 5 at 44.1/48 k, 10 at 96 k)
period  = steps − 1              (+0x58);  div = steps > 1 ? steps : 1
ctl_hz  = rnd(sr) / div;  per_ms = (f64)(f32)ctl_hz / 1000
L       = (0.310295·0.9602356 + 0.311434·0.039764404) − 61        (as in Chorus)
c100    = 1 − (f64)exp2((f32)(L / max(100·per_ms − 1, 1)))   Mix, Feedback, gain, frequency
c250    = 1 − (f64)exp2((f32)(L / max(250·per_ms − 1, 1)))   Invert, Stereo
note_hi = ((0.184142·0.7727356 + 0.185385·0.2272644) + 2)·12 + 69      = 95.213
note_lo = ((0.803514·0.8909302 + 0.804324·0.109069824) − 8)·12 + 69    = −17.357
range   = note_hi − note_lo   (+0xb8);  note_lo (+0xb4);  inv_sr = 1/sr
```
`exp2` is Chorus's table routine (the 1023-entry table `s_0303B510` at 0x10dc202fc is
byte-identical to Chorus's). The four Coarse-range values are entries of a log table
(around 0x10dc1f524 and 0x10dc1fedc) read at fixed positions with fixed weights; the port
carries their bits.

Then every parameter is set as its event handler would (§3.3), each smoothed value starts
at its target, the counter starts at `div` (so the control block runs at once), the phase is
−0.5, and one `processAudio` step runs on silent input (output dropped). After it the
all-pass states hold `tiny` offsets, every smoother has taken one step, and the counter is 0.

### 3.3 Events **[exact]**

```
Coarse: crs_note = note_lo + Coarse·range
Fine:   fine_st  = Fine·12
both:   x  = (crs_note + fine_st)·0.0833292
        n  = rnd(x);  f = x − n
        Hz = ((f·(f·(f·0.056 + 0.242) + 0.693) + 1)·2^n)·8.17742    (2^n via exponent bits)
        r  = (1000/Hz)·1.5
        fb_lim  = min((Hz·0.333333)·0.02 + 0.93, 1)
        gain_hz = r < 1 ? max(r, 0.707) : 1          (= clamp(1500/Hz, 0.707, 1))
        freq_t = (f64)Hz;  fb_t = (f64)(fb_scaled·fb_lim);  gain_t = (f64)((gain_hz·gain_fb)·0.7078445)
Feedback: fb_scaled = Feedback·0.7
        gain_fb = Feedback > 0.35 ? ((Feedback − 0.35)·1.53846)²·(−0.35) + 1 : 1
        fb_t, gain_t as above
Stereo: st_t  = (f64)((St·(St·St))·0.5 + 0)        (cycles of phase offset, 0 … 0.5)
Invert: inv_t = (f64)Invert (0 or 1)
Mix:    mix_t = (f64)Mix
```
So Coarse sweeps the shift **logarithmically from 3.0 Hz (0 %) through 77.5 Hz (50 %) to
2000 Hz (100 %)**, and Fine adds 0 … 12 semitones (×1 … ×2, up to 4 kHz). Feedback is
`0.7·Feedback`, reduced below ~10 Hz (×0.93 at 0 Hz). The output gain is −3 dB, a further up
to −3 dB above 1.5 kHz (`1500/Hz`, floor 0.707 from 2.1 kHz), and up to −3.7 dB more at
Feedback 100 %.

### 3.4 Control block and `BfrE`

`processAudio` first runs the all-passes, then the control counter **[exact]**:
```
if ctr < period: ctr += 1
else: ctr = 0; if flags ≥ 1:
    each smoother: s += c·((t − s) + tiny_d)   (f64); value = (f32)s
    wet = (2 − m)·m;  dry = (1 − m)·(2 − (1 − m))      m = Mix value
    inc = inv_sr · Hz value
```
`processEventInBfrE` (0x101a12900) clears each smoother's flag when `|(f64)value − t| ≤ 1e-4`
(bits 0 Hz, 1 Feedback, 2 gain, 3 Stereo, 4 Invert, 5 Mix; bits 6–7 belong to two smoothers
towards 0 that feed nothing and always clear). Once every flag is clear the control block
stops, and the smoothed values freeze within 1e-4 of their targets. With constant parameters
every value equals its target from `endInit` on, so this only matters after a change; then
the exact freeze point depends on the block boundaries.

The time constants: 2^L ≈ −365 dB after 100 ms (Mix, Feedback, gain, Hz) or 250 ms (Invert,
Stereo); the 1/e time is about T/42 (2.4 ms / 6 ms). Invert therefore crossfades between the
two directions rather than switching.

### 3.5 Oscillator and output (every sample) **[exact]**

```
p   = phase + inc;  pL = p − rnd(p);  phase = pL           (left phase in [−0.5, 0.5])
q   = St + pL                                              (right phase, St cycles ahead)
para(v) = v·(8 − |v|·16)                                   (parabolic sine of a wrapped phase)
sinR = para(q − rnd(q));      cosR = para(q' − rnd(q')),  q' = q − 0.25
sinL = para(pL);              cosL = para(p' − rnd(p')),  p' = pL − 0.25
per channel:
  b = B_out·sin;  a = A_out·cos
  sum = b + a;  out = sum + Inv·((b − a) − sum)            (Inv 0: b + a, Inv 1: b − a)
  y   = x·dry + wet·(gain·out)
  fb  = Feedback_value·out + tiny                          (into the next sample's u)
```
Invert **off shifts up** (the input frequency F comes out at F + Hz), Invert **on** (the
default) shifts **down** (F − Hz, reflecting through 0 for F < Hz). The two channels are
independent except for the shared oscillator. Mix 0 gives `dry = 1`, `wet = 0`: the output
is bit-identical to the input.

---

## 4. Implementation notes

- `FreqShifter::set` resumes on the first call and sends events for changed values later;
  `process` sends `BfrE` at the start of each call, as Maschine does per block.
- Operation order, `f32`/`f64` and the `tiny` offsets follow the assembly; no `mul_add`.
- Not verified against Maschine: parameter changes during a render (`maschine-ref` has no
  automation). After a change the smoothers freeze at the first block start where all are
  within 1e-4, so DECKARD and Maschine can differ by < 1e-4 in a smoothed value if their
  block sizes differ.
- Feedback is stable over the tested range; very low shifts (3 Hz) with full feedback on
  periodic input can build up short bursts (in Maschine as well, since the port is exact).

## 5. Null tests

Source project: `~/Documents/Native Instruments/Maschine 2/Projects/4-6 show.mxprj`, sound
3/15 slot 1 (Freq Shifter, stored coarse 0.04 / fine 0 / feedback 0.08 / stereo 0.62 /
invert 1 / mix 1), made with `maschine-ref make … 3 15 1 [field=…]`, rendered 3 s at −12 dBFS
with `--preroll 0`, compared with `fx-null` on the captured input (scratch script `nt.sh`).
Unlisted fields keep the project values. **−240 dB = bit-identical.** (Several library
projects with a Freq Shifter, e.g. *Concrete Dubs*, fail to load in the plug-in with
"unregistered class" even as a pass-through, so they were not used.)

| case | input | sr | params changed | residual L / R |
|---|---|---|---|---|
| project values | noise | 44.1 k | – | −240 / −240 |
| module defaults | noise | 44.1 k | 0.5 / 0 / 0.75 / 0.75 / on / 0.75 | −240 / −240 |
| module defaults | sweep | 48 k | same | −240 / −240 |
| project values | sweep | 44.1 k | – | −240 / −240 |
| project values | sine 440 | 48 k | – | −240 / −240 |
| coarse min (3 Hz) | noise | 44.1 k | coarse 0 | −240 / −240 |
| coarse max (2 kHz) | noise | 44.1 k | coarse 1 | −240 / −240 |
| coarse mid | noise | 48 k | coarse 0.5 | −240 / −240 |
| fine max | noise | 44.1 k | fine 1 | −240 / −240 |
| fine mid | noise | 44.1 k | fine 0.5, coarse 0.7 | −240 / −240 |
| feedback 0 / 1 | noise | 44.1 k | feedback 0, 1 | −240 / −240 |
| feedback knee | noise | 44.1 k | feedback 0.35, coarse 0.8 | −240 / −240 |
| feedback mid | noise | 48 k | feedback 0.6, coarse 0.3 | −240 / −240 |
| stereo 0 / 0.5 / 1 | noise | 44.1 k | stereo 0, 0.5, 1 | −240 / −240 |
| invert off (up) | noise | 44.1 k | invert 0 | −240 / −240 |
| invert on | sweep | 48 k | invert 1, coarse 0.6 | −240 / −240 |
| mix 0 / 0.5 | noise | 44.1 k | mix 0, 0.5 | −240 / −240 |
| impulse, full feedback | impulse | 44.1 k | coarse 0.5, feedback 1, mix 1 | −240 / −240 |
| odd values | noise | 48 k | 0.123 / 0.877 / 0.333 / 0.456 / off / 0.789 | −240 / −240 |
| control rate 10 | noise | 96 k | 0.9 / 1 / 1 / 1 / off / 1 | −240 / −240 |
| control rate 2 | noise | 22.05 k | coarse 0.9, fine 0.3, feedback 0.9 | −240 / −240 |
| control rate 9 | sweep | 88.2 k | coarse 1, fine 1, feedback 1 | −240 / −240 |
| control rate 20 | noise | 192 k | coarse 0.4, feedback 0.8 | −240 / −240 |
| `--preroll 1`, input as is | noise | 44.1 k | coarse 0.6, feedback 0.9, stereo 0.7 | −16.1 / −16.0 (phase differs) |
| `--preroll 1`, DECKARD fed `sr` zeros first | noise | 44.1 k | same | −240 / −240 |
| `--preroll 1`, DECKARD fed `sr` zeros first | noise | 48 k | 0.3 / 0.5 / 1 / 1 / off / 1 | −240 / −240 |
