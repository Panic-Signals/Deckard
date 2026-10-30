# Maschine's Grain Delay: parameters and DSP

A clean-room description of Maschine 3's built-in **Grain Delay** effect
(`NI::MASCHINE::DATA::GrainDelay` → `NI::MASCHINE::RT::GrainDelayProcessor` →
`NI::MASCHINE::FXCORE::GrainDelayCore`). `GrainDelayCore` is a compiled Reaktor-Core
kernel: one flat struct (0x3003f8 bytes: inputs, state and four delay lines) with
`endInit()`, `processAudio()` (one sample per call) and two event handlers
(`processEventInms` for Size, `processEventInSpc` for Space). The processor writes the other
parameters straight into the kernel.

Sources: Ghidra C in `out/decomp/grain_delay/` (`INDEX.tsv`; with offsets rewritten in
`out/decomp/grain_delay-x/`) and arm64 disassembly in `out/decomp/grain_delay-all.s`
(everything from the `DATA` constructor to the static initializer),
`out/decomp/grain_delay-ei.s` (`endInit`) and `out/decomp/grain_delay-pa.s`
(`processAudio`). Ghidra gets several sums and one `endInit` formula wrong in this kernel,
so the spec and the port follow the assembly. Addresses are arm64 VM addresses in
`analysis/maschine_arm64`. Port: `crates/engine/src/fx/grain_delay.rs`. It nulls
**bit-exactly** against Maschine (§5).

**Confidence.** **[exact]**: read from code or assembly and confirmed by the null tests.
**[inferred]**: a reasonable reading. **[unknown]**: not recovered.

Notation: `v` is a stored value; `sr` the sample rate (`f32`). All DSP is `f32` unless marked
`f64`. The kernel has **no fused multiply-adds**. `rnd(x)` adds ±0.5 (by the sign of `x`) in
`f64` and truncates, so halves round away from zero. `exp2(x)` and `log2(x)` are the
table approximations of §3.1.

---

## 1. Parameters

### 1.1 Module

- Module name "Grain Delay", module-info id **27** (0x1b) (`s_aModuleInfo`, set in
  `__GLOBAL__sub_I_Modules_Effects_GrainDelay_ALL.cpp`, 0x101a3be14). **[exact]**
- Pages (`s_aPageNames`): **"Main"**, **"Output"**. **[exact]**
- Processor: stereo in, stereo out (`setStereoInStereoOut`), in place, no latency reported.
  **[exact]**

### 1.2 Parameters in constructor order

`GrainDelay::GrainDelay()` (0x101a351f4) makes each parameter with
`TransactionObjectContainer::insert(label, default, min, max)` **[exact]**. Labels are
wide strings at 0x104d2ccac … 0x104d2cd6c.

| obj off | mxprj field | label | type | default | range | RT idx | display | section |
|---|---|---|---|---|---|---|---|---|
| 0xa8 | `pitch` | **Pitch** | float, **bipolar** | 0 | −1 … 1 | float 0 | `FloatLinearConverter(0, 48, "%.1f")`: `48·v` (semitones, no unit), −48.0 … 48.0 | "Grain" |
| 0xb0 | `size` | **Size** | float | 0.25 | 0 … 1 | float 1 | `ScaleNormalizedToPercentOneDecimal` (`v·100`, "%.1f %") | – |
| 0xb8 | `jitter` | **Jitter** | float | 0 | 0 … 1 | float 2 | percent, one decimal | – |
| 0xc0 | `reverse` | **Reverse** | bool | off | on/off | bool 0 | on/off | – |
| 0xc8 | `space` | **Space** | float | 0 | 0 … 1 | float 3 | percent, one decimal | "Cloud" |
| 0xd0 | `density` | **Density** | float | 0.25 | 0 … 1 | float 4 | percent, one decimal | – |
| 0xd8 | `modulation` | **Mod** | float | 0 | 0 … 1 | float 5 | percent, one decimal | – |
| 0xe0 | `mix` | **Mix** | float | 0.75 | 0 … 1 | float 6 | percent, one decimal | – |
| 0xe8 | `stereo` | **Stereo** | float | 0.75 | 0 … 1 | float 7 | percent, one decimal | "Output" |

- Pitch: `setNumSteps(96)` (one semitone per step over −48 … 48),
  `setNumDraggerSteps(4800, 100)`, a fine step 0.0088903 (`+0x1bc` = `0x3c11d14e`), bipolar.
  **[exact]**
- The GUI model (`GUI::qml_module::GrainDelayModel`) binds Pitch, Size, Space, Mix, Jitter,
  Reverse, Density, Modulation and Stereo for the custom editor (`GrainDelayEditor.qml`).
- Serialization (`serialize<iarchive>` 0x101a3501c): Module base, then the nine objects at
  0xa8 … 0xe8 in table order. In `crates/mxprj` they were `unknown_a8` … `unknown_tail`;
  they are now named as above. **[exact]**

### 1.3 Knob layout (`getParameterByPosition`, 0x101a36080)

| page | pos | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
|---|---|---|---|---|---|---|---|---|---|
| Main | 0–7 | PITCH | SIZE | JITTER | REVERSE | SPACE | DENSITY | MOD | MIX |
| Output | 8 | STEREO | – | – | – | – | – | – | – |

Section headings: Main slot 0 "Grain", Main slot 4 "Cloud", Output slot 0 "Output".
No enums and no mode-dependent knobs. **[exact]**

### 1.4 Value → kernel (`GrainDelayProcessor::processFloatParameter` 0x101a35c08, `processBoolParameter` 0x101a35d0c, `setDensity` 0x101a39c34)

Each parameter is bound with `setAndCallFuncAutomate` (`connectParameters` 0x101a357ac).
**[exact]**:

| idx | parameter | kernel input | then | flag |
|---|---|---|---|---|
| 0 | Pitch | +0x28 = `48·v` | pitch target `= (f64)(48·v)` | 0x01 |
| 1 | Size | +0x18 = `v` | `processEventInms` (§3.4) | 0x06 |
| 2 | Jitter | +0x64 = `v` | `j2 = (f64)(v·v)`; ±values `∓ jscale·j2` | – |
| 3 | Space | +0x38 = `v` | `processEventInSpc` (§3.4) | 0x06 |
| 4 | Density | +0x40 = `v` | `setDensity` (§3.4) | 0x0e |
| 5 | Mod | +0x30 = `v` | mod target `= (f64)(a + v·v·(0.1 − a))` | 0x10 |
| 6 | Mix | +0x10 = `v` | mix target `= (f64)v` | 0x40 |
| 7 | Stereo | +0x4c = `v` | stereo target `= (f64)v` | 0x20 |
| bool 0 | Reverse | +0x20 = `(f32)b` | `rev = (rnd(b) ≠ 0)` into both voices (+0xe0, +0x1e4) | – |

The flags (+0x74) say which smoothers move (§3.3).

---

## 2. Processor (`GrainDelayProcessor`)

- **Constructor** (0x101a36280): allocates the kernel (0x3003f8 bytes, zeroed). **[exact]**
- **`onSamplerateChanged(sr)`** stores `sr`; it applies at the next `resume`.
- **`resume()`** (0x101a363ec): saves the nine inputs, **zeroes the whole kernel**
  (delay lines included), writes `sr`, the inputs and an event flag 1.0 for each, then
  `endInit()`. **[exact]**
- **`process()`** (0x101a382f0), per block: for each of the seven smoothers it compares the
  smoothed value (rounded to `f32`) with its target and **clears that flag when
  `|value − target| ≤ 1e-4`**. Then, per sample, in L → +0xc, in R → +0x48,
  `processAudio()`, out L ← +0x54, out R ← +0x58. **[exact]**

So a smoother stops (frozen within 1e-4 of its target) at the first block start after it
arrives, and the smoothing pass runs while any flag is set. After `resume` every smoother
already sits on its target, so with fixed parameters the first block clears all flags and
the output does not depend on the block size. With automation the freeze points depend on
Maschine's block boundaries; the port checks at the start of every `process()` call. The
kernel's internal state is free-running from `resume`: the grain clock, the Mod LFO and the
voices do not follow the transport.

---

## 3. DSP (`FXCORE::GrainDelayCore`)

### 3.1 Tables and approximations

The kernel reads three static tables (`__DATA`):

| symbol | addr | entries | content | port |
|---|---|---|---|---|
| `s_04C30350` | 0x10dc30e7c | 1025 | `2^(i/1021 − 0.5)` for i ≤ 1021, then √2, 1/√2, 1/√2 (6 significant digits; byte-identical to Chorus's table) | formula + 7 overrides |
| `s_04C31388` | 0x10dc31e80 | 1026 | `log2(1 + i/1021)` for i ≤ 1021, then 1, 0, 0, 0 | formula + 9 overrides |
| `s_0694FDF8` | 0x10dc30c7c | 128 | clipper gain against Density's upper half: 1.0 … 0.0946 | `SAT_GAIN`, copied |

Seven exp entries (106, 213, 225, 287, 324, 382, 673) and nine log entries (27, 88, 264,
297, 377, 435, 439, 560, 935) are ties at the 7th digit stored the other way. The entries
after 1021 matter: an exact power of two gives index −1, which reads entries 1023/1024.
**[exact]**

```
exp2(x):  n = rnd(x);  y = ((x − n) + 0.5)·1021;  k = rnd(y − 0.5);  f = y − k;  i = k & 0x3ff
          = (E[i+1]·f + E[i]·(1 − f)) · 2^n          // 2^n: n added to the exponent of 1.0
log2(x):  m = mantissa of x in [1, 2);  e = exponent;  y = (m − 1)·1021;  k = rnd(y − 0.5)
          f = y − k;  i = k & 0x3ff;  = ((1 − f)·L[i] + f·L[i+1]) + e
cbrt(x)  = exp2(log2(x)·0.63093·0.333333·1.58496)             // x > 0, else 0
note(x)  = with y = x·0.0833292, n = rnd(y), f = y − n:
           (f·(f·(f·0.056 + 0.242) + 0.693) + 1)·2^n·8.17742   // Hz of MIDI note x
tan(t)   : u = t + (t³·0.333333 + (t²·t³)·0.133333)   // the one-pole prewarp
window(p)= (x²·P(x²) + 2)·0.5, x = p·6.28318 − 3.14159, P = (((2.47609e-5 − 2.605e-7·x²)·x²
           − 0.00138884)·x² + 0.0416666)·x² − 0.499923    // (1 + cos x)/2: Hann, p ∈ [0, 1]
clip(x)  = c·(1 − |c|·0.0625), c = x limited to ±8
```

Smoothers are `s ← s + c·((t − s) + 1e-100)` in `f64`; the `f32` value is what the DSP uses.

### 3.2 `endInit()` (0x101a364cc)

`resume` flags every input, so `endInit` always takes all its "input changed" paths
**[exact]**:

- Control rate: `steps = rnd(sr/8820)` (f32 division), counter limit `steps − 1`;
  `per_ms = (rnd(sr) / steps)/1000` (integer division; control steps per ms). 5 steps at
  44.1 k and 48 k (`per_ms` 8.82 / 9.6), 11 at 96 k.
- Smoother coefficients `c = 1 − exp2(L/max(T·per_ms − 1, 1))`: with `L = log2(0.01)` from
  the log table (`(L[285]·0.12002563 + L[286]·0.87997437) − 7`), `T` = 250 ms for the space
  delay and 50 ms for the space level (both `f32`); with `L = (L[245]·0.9602356 +
  L[246]·0.039764404) − 61 = −60.69` and `T` in `f64`, 200 ms for Mix and Stereo and 150 ms
  for Pitch, Mod, feedback, Density (upper half) and the LP note.
- Constants: `ratio_k = 1/((E[510]·0.5 + E[511]·0.5)·8.17742)`; `a = 1 − 1/((E[510]·0.5 +
  E[511]·0.5)·65536)` (the Mod base, ≈ 0.99998); `k154 = (L[39]·0.48719406 +
  L[40]·0.51280594) − 16` (≈ −15.945); band limits `lo = sr/24576`, `hi = sr/2.125`;
  `π/sr` with π = 3.14159; HP corners 55 Hz (fed by voice B) and 45 Hz (fed by voice A),
  each taken as `hi` if `hi` < corner else `max(lo, corner)`.
- All targets as in §3.4 (from the inputs), every smoother **starting at its target**; the
  LFO phase is `inc − 0.5` wrapped.
- Then one control step (§3.3) and one sample (§3.5) on the zero input, with these
  differences: the voice toggle stays 0 and voice **B** starts; the jitter value is not
  looked up (it stays 0); the period is `rnd(per_ms·((ms + max(cube, 0))·0.5))`.

### 3.3 Per sample (`processAudio`, 0x101a38578)

```
write in L, in R at w (grain lines, 2^18 each);  w = (w + 1) & 0x3ffff
lfo = wrap(lfo + lfo_inc)                         // [−0.5, 0.5); inc = (1000/ms)·0.03125537/sr
if counter < steps − 1: counter += 1             // no control step
else (control step, every `steps` samples):
  g = grain_counter;  grain_counter += 1
  dly ← dly + c250·((Size·sr/1000 − dly) + 1e-18)        // f32
  if flags ≥ 1: all seven smoothers step; then
     wet = (2 − m)·m, dry = (1 − m)·(2 − (1 − m))          // m = Mix
     depth = Mod − 1 (smoothed)                            // tiny unless Mod > 0
     sat = SAT_GAIN lerp at dh·127, sp_wet = (2 − dh)·dh   // dh = Density upper half
     up = 18.953209·dh + 1,  wid = −0.20555687·dh + 1
     level target = (f32)(gain·fb)
     ratio = ratio_k·(exp2(pitch·0.0833292)·8.17742)       // 2^(semitones/12)
     len = (sr/1000·ms)·(min(ratio·len, 2^18)/(ratio·len))
     offset = −(max(ratio, 1)·len − len);  span± = ±ratio·len
     LP L ← band(note + 10), LP R ← band(note)            // band: limit to [lo, hi]
  if g == 0: the other voice is chosen (toggle ^= 1); if its window < 0.010000365 it
     restarts: phase 0, inc = (1000/ms)/sr, start = offset + w
  if the toggle changed since the last period: jit = ±jscale·Jitter² (+ for toggle 1)
     period = rnd(per_ms·((ms + max(jit + cube, 0))·0.5))
  if period ≤ g + 1: grain_counter = 0
  span− is stored (after the clock); level ← level + c50·((target − level) + 1e-18)
  a restarted voice takes span[Reverse]
```

So the voices alternate every `(Size ± Jitter)/2` and each grain lasts Size ms: two Hann
windows with 50 % overlap. Jitter is not random: the period alternates between
`(ms + j)/2` and `(ms − j)/2` (`j = Jitter²·max(ms, cube)`, plus `cube`, clamped at 0).
The kernel has **no random number generator**. A voice whose window has not yet fallen
below 0.01 is not restarted; its turn is skipped.

Then, every sample **[exact]**:

```
pA, pB = min(phase, 1), phases += inc
tri = lfo·(|lfo|·−8 + 4) + 0.5                    // triangle 0 … 1
dA = dly·(tri·depth + 1), dB = dly·((1 − tri)·depth + 1)
grain(v, line, p):                                // A reads line L, B reads line R
  pos = p·span + start;  h0, h1, h2 = the last three positions
  q1 = (h0·0.273438 + (h1·0.820313 − h2·0.0546875)) − pos·0.0390625
  q2 = (h0·0.5625 + (h1·0.5625 − h2·0.0625)) − pos·0.0625
  q3 = (h0·0.820313 + (h1·0.273438 − h2·0.0390625)) − pos·0.0546875
  lerp(q): i = rnd(q − 0.5); a = line[i−1], b = line[i]; a + (b − a)·((i − q) + 1)
  = window(p)·((lerp(h0) + (lerp(q3) + (lerp(q1) + lerp(q2))))·0.25)
space lines (2^17 each, write first, then read): spR[ws] = fbR·level, spL[ws] = fbL·level
  outB = grainB + lerp of spR at ws − rnd(dB − 0.5);  outA = grainA + lerp of spL at ws − …
sA = wid·outA, sB = outB·wid, mid = (sB + sA)·0.5
out L = inL·dry + wet·(mid + stereo·(sA − mid))
out R = inR·dry + wet·(mid + stereo·(sB − mid))
side(x) for x = outB → fbL, x = outA → fbR:
  y  = x1·a + (y1·b + x·a)           // LP, a = u/(1+u), b = (1−u)/(1+u), u = tan(π/sr·f)
  hp = y1·(−g) + (h1·b' + y·g)       // HP, g = 1/(1+u'), b' = (1−u')/(1+u')
  (x1, y1, h1 = x, y, hp, each + 1e-18)
  n = up·hp;  mid = (u0·0.5625 + (u1·0.5625 − u2·0.0625)) − n·0.0625;  shift u
  fb = hp + sp_wet·((clip(u0_old) + clip(mid))·0.5·sat − hp) + 1e-18
ws = (ws + 1) & 0x1ffff
```

The four sub-positions are the cubic (Lagrange) interpolation of the read position at 1/4,
1/2 and 3/4 of the way from the previous position, so each output averages four linear
reads along the path: an anti-aliasing filter for pitched-up grains. The space part is a
cross-coupled ping-pong: B's output feeds the left space line, which A's output reads, and
the other way round. Mod sweeps the two space delays in anti-phase with a triangle at
1/32 of the grain rate.

### 3.4 Events

`processEventInms` (Size, 0x101a39488) **[exact]**:
```
ms = v·v·928 + 72                                      // 72 … 1000 ms
grain_hz = 1000/ms;  lfo_inc = grain_hz·0.03125537·(1/sr)
space delay target = len = ms·sr/1000; offset, span± from the current ratio (§3.3)
x = ms·min(0.1·2^(ms/20 − 1), 5)   (quadratic 2^: (f·(f·0.248 + 0.708) + 1)·2^n)
root = cbrt(x) (0 if x ≤ 0);  s = root·Space;  cube = s·(s·s)
jscale = max(ms, cube) (f64)
then "space common" below
```
`processEventInSpc` (Space): `cube = (root·Space)³` with the stored root, `jscale = max(ms,
cube)`, then:
```
fb_lo = limit(cube·0.008·0.6, 0, 0.6) (0.6 once cube·0.008 ≥ 1)
lp_amt = limit(cube·0.004, 0, 1);  LP note target = note(lp_amt·lp_k + 96)
feedback target = (f64)(fb_lo + (0.95 − fb_lo)·dens_x)
gain = exp2(k154 / max(max(cube, 0.25)·sr/1000 − 1, 1))   // f64 division
level target = fb·gain;  jit± = ∓jscale·Jitter²;  period recomputed;  flags |= 6
```
`setDensity` (0x101a39c34):
```
lo = min(2v, 1);  dens_hi target = (f64)(2v − lo)          // the upper half, 0 … 1
dens_x = limit(2·lo, 0, 1) = min(4v, 1)
h = 2·(lo − 0.5);  k = 20 if h < 0, 18h + 20 if h < 1, else 38
lp_k = limit(4·(dens_x − 0.75), 0, 1)·k + 84 − 96
feedback and LP note targets as above;  flags |= 0xe
```

What the knobs do: **Size** 72 … 1000 ms grains (quadratic); **Pitch** ±48 semitones;
**Jitter** alternates the grain period by ±Jitter²·Size; **Reverse** reads each new grain
backwards; **Space** sets the space level and lengthens the period (`cube` ms); **Density**
sets the feedback (0 … 0.95 over its lower quarter) and, over its upper half, the clipper
drive, interpolation gain and wet amount of the space feedback and the LP range;
**Mod** sweeps the space delays; **Mix** `dry = 1 − m²`, `wet = 2m − m²`; **Stereo** the
width between the two voices.

---

## 4. Implementation notes

- `GrainDelay::set` resumes on the first call; later calls send only the changed values as
  events, which keeps the delay lines, voices and LFO running.
- Not emulated: the per-block "previous value" copies the processor makes for `endInit`'s
  never-taken paths, and Maschine's own block boundaries under automation (see §2).
- The kernel is about 3 MB (four delay lines).

## 5. Null tests

Source project `/Users/Shared/Chromatic Fire Library/Projects/Dev Traps.mxprj`, sound 10/0
slot 4 (Grain Delay, stored pitch 0, size 0.09, jitter 0.04, space 0.38, density 0.2,
mod 0.28, mix 0.43, stereo 0.75), made with `maschine-ref make … 10 0 4 field=…` (all nine
fields written). Rendered for 3 s at −12 dBFS with `--preroll 0`, compared with `fx-null`
(512-sample blocks) on the captured input. "Odd values" list pitch/size/jitter/reverse/space/density/mod/mix/stereo. **−240 dB = bit-identical** (peak difference 0).

| case | input | sr | settings (else defaults) | residual L / R |
|---|---|---|---|---|
| defaults | noise | 48 k / 44.1 k | – | −240 / −240 |
| project values | noise | 48 k | Dev Traps' own | −240 / −240 |
| pitch | noise | 48 k | −1, −0.25, 0.123, 1 | −240 / −240 each |
| size | noise | 48 k | 0, 1 | −240 / −240 each |
| jitter | noise | 48 k | 1; 0.5 with size 0.6 | −240 / −240 each |
| reverse | noise | 48 k | on; on with pitch 0.5, size 0.8 | −240 / −240 each |
| space | noise | 48 k | 0.5, 1 | −240 / −240 each |
| density | noise | 48 k | 0, 1; 0.5 and 0.75 with space 0.6 | −240 / −240 each |
| mod | noise | 48 k | 1 with space 0.7; 0.5 with space 0.4 | −240 / −240 each |
| mix | noise | 48 k | 0, 1 | −240 / −240 each |
| stereo | noise | 48 k | 0, 1 | −240 / −240 each |
| all max | noise | 48 k | every knob at max, reverse on | −240 / −240 |
| all min | noise | 48 k | pitch −1, others 0, mix 0.001 | −240 / −240 |
| odd values | noise | 48 k | −0.37/0.41/0.33/–/0.77/0.66/0.21/0.58/0.31 | −240 / −240 |
| odd values | noise | 44.1 k | 0.61/0.13/0.71/on/0.29/0.91/0.83/0.9/0.05 | −240 / −240 |
| odd values | noise | 96 k | −0.12/0.52/0.2/–/0.85/0.35/0.6/0.7/0.6 | −240 / −240 |
| | sweep | 48 k | pitch 0.25, space 0.5, size 0.5 | −240 / −240 |
| | sine 440 | 44.1 k | pitch −0.5, space 0.3, jitter 0.5 | −240 / −240 |
| | impulse | 48 k | space 0.9, density 0.8, mix 1 | −240 / −240 |
| 12 s, long feedback | noise | 48 k | space 1, density 1, mod 1, size 1 | −240 / −240 |
| `--preroll 1`, DECKARD fed 48000 zeros first | noise | 48 k | odd values (48 k row) | −240 / −240 |
| oracle: 240 random cases | random ±0.5 and ×20 | 22.05 … 192 k | random settings, blocks 64 … 1000, 1–36 parameter changes between blocks | bit-identical, all |

"Oracle" rows: Maschine's own kernel, called from the VST3 binary
(`/Library/Audio/Plug-Ins/VST3/Maschine 3.vst3`, functions found by symbol offset; scratch
harness `gd.c`), was first checked bit-identical to `maschine-ref` renders, then compared
with DECKARD's port over random settings, sample rates (22.05 … 192 k), block sizes and
parameter changes between blocks (scratch `fuzz.py`).
