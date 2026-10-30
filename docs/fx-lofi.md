# Maschine's Lo-Fi: parameters and DSP

A clean-room description of Maschine 3's built-in **Lo-Fi** effect
(`NI::MASCHINE::DATA::LoFi` → `NI::MASCHINE::RT::LoFiProcessor` →
`NI::MASCHINE::FXCORE::LoFiCore`). `LoFiCore` is a compiled Reaktor-Core kernel (a flat
struct of inputs, outputs and state, with `beginInit(sr)`, `endInit()`, `processAudio()` (one
sample per call) and one `processEventIn*()` per parameter). Unlike Chorus's kernel, this one
is vectorized: the five ramped parameters live in 4-lane vectors and both channels go through
the crusher and the hold stage as lanes of one vector.

Written from arm64 disassembly of `analysis/maschine_arm64`: the kernel is in
`out/decomp/lofi-core.s` (0x101a1d754–0x101a1e0a4), the data class, processor and SR
converter in `out/decomp/lofi-data.s` (0x101a48a6c–0x101a49604). (A Ghidra run was started
with `out/decomp/lofi-names.txt` and stopped: the assembly was enough.) Data were read from
`__DATA`/`__TEXT` constants (§3.1). The DECKARD port is `crates/engine/src/fx/lofi.rs`. It
nulls **bit-exactly** against Maschine (§5).

**Confidence.** **[exact]**: formula and constants read from code or assembly.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation:
- `v`: a parameter's normalized value (0..1) as stored in the project.
- `sr`: sample rate (`f32` in the kernel).
- All arithmetic is `f32`. `fma(a,b,c)` is `a·b + c` with one rounding (`fmadd`); every other
  product or sum is rounded on its own.
- `rnd(x)`: add `+0.5` (when `x ≥ 0`) or `−0.5` to `x` **in `f64`** and truncate (`fcvtzs`).
- `frinta`: round half away from zero (Rust `f32::round`).
- `exp2f`: the system libm function (`_exp2f` stub 0x10489ce98).

---

## 1. Parameters

### 1.1 Module

Module name "LoFi" (mxprj class `NI::MASCHINE::DATA::LoFi`); the chain shows it as "Lo-Fi".
One page: `s_aPageNames = { "Main" }` (`__GLOBAL__sub_I_Modules_Effects_LoFi_ALL.cpp`,
0x101a4b304; string at 0x104d2f3b8) **[exact]**.

### 1.2 Parameters in constructor order

`LoFi::LoFi()` (0x101a48a6c) makes each parameter with
`TransactionObjectContainer::insert(label, default, min, max)` **[exact]**:

| obj off | mxprj field | label | section | default | range | RT idx | display converter |
|---|---|---|---|---|---|---|---|
| 0xa8 | `sample_rate` | **SR** | "Resample" | 0.12975679 (`0x3e04def7`) | 0..1 | 0 | `aSRConverter` (`LofiSRDisplayConverter`, §1.3) |
| 0xb0 | `bits` | **Bits** | "Bitcrush" | 0.185 (`0x3e3d70a4`) | 0..1 | 1 | `FloatLinearConverter(12, 1, "%.1f")` |
| 0xb8 | `smooth` | **Smooth** | (Bitcrush) | 1.0 | 0..1 | 2 | `ScaleNormalizedToPercentOneDecimal` |
| 0xc0 | `stereo` | **Stereo** | (Bitcrush) | 1.0 | 0..1 | 3 | `ScaleNormalizedToPercentOneDecimal` |
| 0xc8 | `mix` | **Mix** | "Output" | 1.0 | 0..1 | 4 | `ScaleNormalizedToPercentOneDecimal` |

All five are plain `MaschineNumericParameter<float>`: no enums or switches. Labels are wide
strings at 0x104d2f3cc ("SR"), 0x3d8, 0x3ec, 0x408, 0x424; section names at 0x104d2f434
("Resample", set on SR), 0x458 ("Bitcrush", set on Bits) and 0x490 ("Output", set on Mix).

### 1.3 Display converters

- **SR**: `convertTo(v) = powf(1.05946 (0x3f879c63 = 2^(1/12)), v·(−127)) · 44100`, printed by
  `StringUnits::hzToString` (`"%.1f kHz"` of Hz/1000 from 1000 Hz up, `"%.1f Hz"` below,
  `"%.2f Hz"` below 10; see docs/fx-filter.md §1.3) **[exact]**. So the knob reads
  44.1 kHz at 0, 17.0 kHz at the default, 1.1 kHz at 50 %, 28.8 Hz at 100 %: 127 semitones
  down. **This is not the rate the DSP uses**: the hold clock follows the `SR_HZ` table
  (§3.1), 44100 Hz … 99.5 Hz (e.g. ≈ 20.0 kHz at the default, 2.1 kHz at 50 %). The display
  is Maschine's, and DECKARD should show the same text. `convertFrom(Hz)` is the inverse:
  `clamp(log(Hz·0.022676)/0.057762·(−0.0094787), 0, 1)` (the log stub at 0x10489d45c was not identified) **[inferred]**.
- **Bits**: `12 + (1 − 12)·v` = `12 − 11·v` bits, `"%.1f"` (no unit) **[exact]**; default
  shows "10.0".
- **Smooth, Stereo, Mix**: `v·100` with one decimal and `%` **[exact]**.

### 1.4 Knob layout (`getParameterByPosition`, 0x101a49214; jump table 0x104d2f3ad)

| pos | page | parameter | section heading |
|---|---|---|---|
| 0 | Main | SR | "Resample" |
| 1 | Main | Bits | "Bitcrush" |
| 2 | Main | Smooth | |
| 3 | Main | Stereo | |
| 4 | Main | Mix | "Output" |
| 5–7 | Main | – (empty) | |

Every parameter is always shown (no mode-dependent knobs).

### 1.5 Serialization

`iserializer<…, LoFi>::load_object_data` (0x101a497e4) loads the Module base, then the objects
at **0xa8, 0xb8, 0xb0, 0xc0, 0xc8** in that order **[exact]**: so the stored order is
SR, Smooth, Bits, Stereo, Mix. These are the `crates/mxprj` fields `sample_rate`, `smooth`,
`bits`, `stereo`, `mix` (formerly `unknown_a8/b8/b0/c0/c8`).

### 1.6 Value → engine (`LoFiProcessor::processFloatParameter`, 0x101a490c0)

Each parameter is bound with `setAndCallFuncAutomate` to `processFloatParameter(idx, …, v)`,
which stores `v` into a kernel input and calls its event handler **[exact]**:

| idx | parameter | kernel input | handler |
|---|---|---|---|
| 0 | SR | +0x00 | `processEventInSR` |
| 1 | Bits | +0x04 | `processEventInBits` |
| 2 | Smooth | +0x08 | `processEventInSm` |
| 3 | Stereo | +0x0c | `processEventInSt` |
| 4 | Mix | +0x10 | `processEventInMix` |

---

## 2. Processor (`LoFiProcessor`)

- **Constructor** (0x101a49324): stereo in, stereo out; allocates the 0x180-byte kernel
  (128-byte aligned) and runs `LoFiCore()` (0x101a1d754), which zeroes most state but **not
  the noise register at +0x170** and not the inputs **[exact]**. `beginInit` does not clear
  +0x170 either, so the kernel never seeds its noise generator. Every Maschine render we made
  matches a register starting at 0 (§5), which makes the "noise" a constant −1 (§3.4)
  **[inferred]**: if the allocation ever reused dirty memory, the dither would differ.
- **`onSamplerateChanged(sr)`** only stores `sr`; it takes effect at the next `resume`.
- **`resume()`** (0x101a49498): saves the five inputs, calls `beginInit(sr)`, restores them,
  sets all five event flags (+0x14..+0x18) and calls `endInit()` **[exact]**.
- **`process()`** (0x101a494f8): per sample, in L → +0x1c, in R → +0x20, `processAudio()`,
  +0x24 → out L, +0x28 → out R. In place, no block-level state.

---

## 3. DSP (`FXCORE::LoFiCore`)

### 3.1 Constants and tables

| addr | content |
|---|---|
| 0x104d297d0 | two `i32`: block length 128 (+0x178) and the counter's start 129 (+0x17c) |
| 0x104d299f0 | `f32` 0.0025, 0.0266: the dither envelope's attack/release times (s) |
| 0x104d299f8 | `SR_HZ[128]`: hold-clock rate, 44100, 42034.7, … 99.516 Hz (≈ ×0.953 per step, with 6-digit ripple; copied as numbers) |
| 0x104d29bf8 | `SR_GAIN[128]`: 1.0 for 0..83, then rising to 1.90312 (copied) |
| 0x104d29df8 | `CRUSH_GAINS[128]`: 0.999929 … 0.222489 (copied) |
| 0x104d14600 | `(1, −1, 1, −1)`: the dither's sign per lane |

Other constants: `log2(0.01)` = `0xc0d49a78`; `1e-30` (`0x0da24260`); 63095.9 (`0x477677e6`,
= 10^(96/20)); `2^−14` (`0x38800000`); 126.99999 (`0x42fdffff`); the snap threshold `1e-15`
(`f64`, `0x3cd203af9ee75616`). All tables print as 6-significant-digit decimals, and the
port lists those decimals.

### 3.2 Init and events

`beginInit(sr)` (0x101a1d780) **[exact]**:
```
block = 128;  counter = 129            // the first processAudio runs a control block
inv_sr = 1/sr                          // +0x30   (also 2π/sr at +0x174, unused)
n_k   = t_k·sr + (−1)                  // t = (0.0025, 0.0266)
c_k   = 1 − exp2f(n_k > 1 ? log2(0.01)/n_k : log2(0.01))     // +0x48 attack, +0x4c release
zero: ramps (current, step, target), flags, ramp counter, envelope, both hold phases,
      the held/previous samples and fractions; sr_hz (+0x50) = 0.5
```
`endInit()` (0x101a1d884) **[exact]**:
```
steps  = max(1, (i32)((sr·0.02)/128 + 0.5))      // 7 at 44.1 k, 8 at 48 k, 15 at 96 k: ~20 ms
ramp_k = 1/(f32)(steps·128)
SR, Bits events (below); Stereo → target[3], Smooth → target[4], Mix → target[2]
current = target (all 8 lanes); changed = 1
```
`processEventInSR` (0x101a1da3c) **[exact]**:
```
x = SR·126.99999;  i = rnd(x − 0.5);  f = x − i
sr_hz   = fma(f, SR_HZ[i+1] − SR_HZ[i], SR_HZ[i])
sr_gain = fma(f, SR_GAIN[i+1] − SR_GAIN[i], SR_GAIN[i])
```
At SR = 0 exactly, `i = −1` and the kernel reads the float before each table (0.0266 and
`SR_HZ[127]`); harmless, because SR = 0 also bypasses the hold stage (§3.5). The port
reproduces it. These two values are **not ramped**: they jump.

`processEventInBits` (0x101a1dac4) **[exact]**:
```
bits = fma(Bits, −11, 12)                          // 12 … 1
target[0] = levels = exp2f(bits + (−1))            // 2048 … 1
target[1] = CRUSH_GAINS[rnd(Bits·127)]
changed = 1
```
`processEventIn{Sm,St,Mix}` set target[4], target[3], target[2] to the raw value and
`changed = 1`.

### 3.3 Parameter ramps (every sample, control every 128)

Lanes: 0 levels, 1 crush gain, 2 Mix, 3 Stereo, 4 Smooth (5–7 unused, always 0).
Per sample **[exact]**:
```
current += step                                      // all lanes
counter += 1
if counter ≥ 128:
    counter = 0
    if changed:
        changed = 0
        for each lane: step = (target − current)·ramp_k
                       if (f64)(step·step) < 1e-15: step = 0, current = target
        if any lane kept a step: ramp_left = steps
    else:
        ramp_left −= 1  (wrapping i32)
        if ramp_left == 0: current = target, step = 0 for all lanes
```
So a change ramps linearly over `steps·128` samples (896 at 44.1 k, ≈ 20 ms) starting at the
next 128-sample boundary, and snaps exactly to the target at the end. The kernel also keeps
a state word (+0x100: 1/2/3/4/0) that nothing reads.

### 3.4 Bit crusher (when `levels ≤ 2047`, i.e. Bits > 0)

With `x = (xL, xR)` and the ramped `levels`, `cg` (crush gain) **[exact]**:
```
inv = 1/levels
pk  = |xL| < |xR| ? |xR| : |xL|
d   = (pk − env) + 1e-30
env = fma(pk > env ? c_att : c_rel, d, env)          // peak follower, 2.5 ms / 26.6 ms to 1 %
s   = (s << 1) | (((s >> 15) ^ (s >> 6)) & 1)        // 32-bit shift register (+0x170)
n   = fma((f32)(s & 0x3fff), 2^−14, −0.5);  n = n + n     // [−1, 1)
dith = (inv·0.25)·(env·n)
for each side k (sign = +1 for L, −1 for R):
    c   = ceil(x_k·levels)       // as: t = |v| < 2^24 ? (f32)(i32)v : v;  if v > t: t += 1
    y_k = ((c + (−0.5))·inv + sign·dith)·cg
    if 0.5 > |x_k·63095.9|: y_k = 0                    // gate below −102 dBFS
```
The quantizer is a mid-riser: `levels` steps per unit, so `bits` bits over −1..1. With the
register at 0 (§2) the noise term is always −1, so the "dither" is a constant offset of
`∓0.25·env/levels` (L −, R +), a quarter step scaled by the input's envelope.
At Bits = 0 (12 bits, 2048 levels) the crusher is skipped entirely and `y = x`; the envelope
and the shift register then don't advance.

### 3.5 Sample and hold (when the SR **input** is not exactly 0)

With `st` = ramped Stereo, `sm` = ramped Smooth; per sample **[exact]**:
```
p    = fma(sr_hz, inv_sr, phase_L)
p2   = st·0.5 + p
wrapL = p ≥ 0.5;  phase_L = p − (wrapL ? frinta(p) : 0)
if p2 ≥ 0.5: w = p2 − frinta(p2); wrapR = (w ≤ phase_R); phase_R = w
else:        wrapR = false;  phase_R = p2
h = held
if wrapL: prev_L = held_L; h_L = y_L
if wrapR: prev_R = held_R; h_R = y_R
for each side: interp = prev + frac·(h − prev)       // frac = last sample's phase + 0.5
               y = h + (sr_gain·interp − h)·sm
frac = (phase_L + 0.5, phase_R + 0.5);  held = h
```
The left clock samples the crushed signal `sr_hz` times per second; the right clock runs at
the same rate offset by `Stereo/2` of a period. Smooth 0 gives the raw staircase; Smooth 1
gives a linear interpolation between the last two held samples (one hold period late),
scaled by `SR_GAIN`. At SR = 0 exactly the stage is bypassed (`y` = crusher output); any
SR > 0 runs it, even at 44100 Hz.

### 3.6 Output

```
out_k = x_k + (y_k − x_k)·mix          // mix = ramped Mix
```
Mix 0 gives the input bit for bit. Bits 0 with SR 0 is also the identity.

---

## 4. Implementation notes

- `LoFi::set` runs `resume()` on the first call. Later calls send only the events of values
  that changed (re-sending an unchanged value would only start a zero-length ramp check),
  keeping the ramps, envelope, phases and held samples.
- The port keeps the kernel's operation order, its `fma`s as `mul_add`, and computes both
  sides lane by lane; NaN-handling of two comparisons was simplified (`!(a > b)` → `a ≤ b`).
- Not verified against Maschine: parameter changes during a render (`maschine-ref` has no
  automation). The ramp code follows the assembly; a unit test checks the ramp lands exactly.
- No side-chain, tempo or meters.

## 5. Null tests

Source project: `/Users/Shared/Maschine 2 Library/Projects/Bar Upstairs.mxprj`, sound 0/1
slot 2 (stored sample_rate 0.04 / smooth 1 / bits 0 / stereo 1 / mix 0.72), made with
`maschine-ref make … 0 1 2 [field=value …]`, rendered 3 s at −12 dBFS with `--preroll 0`
unless noted, compared against `fx-null` on the captured input. Residuals are L / R;
**−240 dB means bit-identical**. Scratch scripts: `nt.sh`, `run.sh`, `pad.py` in the
agent's scratch dir.

| case | input | sr | SR / Bits / Smooth / Stereo / Mix | residual |
|---|---|---|---|---|
| project values | noise | 44.1 k | 0.04 / 0 / 1 / 1 / 0.72 | −240 / −240 |
| defaults | noise | 44.1 k | 0.1298 / 0.185 / 1 / 1 / 1 | −240 / −240 |
| defaults | noise | 48 k | 0.1298 / 0.185 / 1 / 1 / 1 | −240 / −240 |
| bits mid | noise | 44.1 k | 0.04 / 0.5 / 1 / 1 / 0.72 | −240 / −240 |
| bits max (1 bit) | noise | 44.1 k | 0.04 / 1 / 1 / 1 / 1 | −240 / −240 |
| | sine 440 | 48 k | 0.3 / 0.7 / 1 / 1 / 1 | −240 / −240 |
| SR min (stage off) | noise | 44.1 k | 0 / 0.5 / 1 / 1 / 1 | −240 / −240 |
| SR max | noise | 44.1 k | 1 / 0.3 / 1 / 1 / 1 | −240 / −240 |
| SR max, no crush | noise | 44.1 k | 1 / 0 / 1 / 1 / 1 | −240 / −240 |
| SR mid | sweep | 44.1 k | 0.5 / 0.4 / 1 / 1 / 1 | −240 / −240 |
| odd values | noise | 48 k | 0.663 / 0.123 / 1 / 1 / 1 | −240 / −240 |
| smooth min | noise | 44.1 k | 0.5 / 0 / 0 / 1 / 1 | −240 / −240 |
| smooth mid | noise | 44.1 k | 0.8 / 0 / 0.5 / 1 / 1 | −240 / −240 |
| stereo min | noise | 44.1 k | 0.6 / 0 / 1 / 0 / 1 | −240 / −240 |
| stereo mid | noise | 48 k | 0.6 / 0 / 1 / 0.5 / 1 | −240 / −240 |
| stereo max | noise | 44.1 k | 0.9 / 0.6 / 1 / 1 / 1 | −240 / −240 |
| mix min | noise | 44.1 k | 0.5 / 0.8 / 1 / 1 / 0 | −240 / −240 |
| mix mid | noise | 44.1 k | 0.5 / 0.8 / 1 / 1 / 0.5 | −240 / −240 |
| | impulse | 44.1 k | 0.7 / 0.9 / 1 / 1 / 1 | −240 / −240 |
| all mixed | sine 1000 | 48 k | 0.95 / 0.95 / 0.3 / 0.7 / 0.8 | −240 / −240 |
| | sweep | 48 k | 0.2 / 0.9 / 0 / 0 / 1 | −240 / −240 |
| ramp 15 blocks | noise | 96 k | 0.4 / 0.5 / 1 / 1 / 1 | −240 / −240 |
| | noise | 22.05 k | 0.4 / 0.5 / 1 / 1 / 1 | −240 / −240 |
| gate (−100 dBFS) | sweep | 44.1 k | 0.5 / 0.8 / 1 / 1 / 1 | −240 / −240 |
| 0 dBFS | noise | 48 k | 0.3 / 0.6 / 1 / 1 / 1 | −240 / −240 |
| `--preroll 1`, DECKARD fed 44100 zeros first | noise | 44.1 k | 0.55 / 0.7 / 0.4 / 0.6 / 1 | −240 / −240 |
| `--preroll 1`, input fed as is | noise | 44.1 k | same | −16.3 / −16.1 (hold phase differs) |

The bit-exact crusher results also confirm that Maschine's noise register starts at 0 (§2).
