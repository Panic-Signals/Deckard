# Maschine's Reflex: parameters and DSP

A clean-room description of Maschine 3's built-in **Reflex** effect
(`NI::MASCHINE::DATA::Reflex` → `NI::MASCHINE::RT::ReflexProcessor` →
`NI::MASCHINE::FXCORE::ReflexCore`). Reflex is a small early-reflection reverb.
`ReflexCore` is a compiled Reaktor-Core kernel: a flat struct (0x70280 bytes) of inputs,
outputs and state, with `beginInit(sr)`, `endInit()`, `processAudio()` (one sample per
call) and one `processEventIn*()` per parameter. It shares no code with the other reverbs:
the only shared piece is `NI::AB::ExpTable` (`fx::exp_e`, docs/fx-eq.md §2.1).

It was written from Ghidra output in `out/decomp/reflex/` (`INDEX.tsv` maps files to
functions) and from arm64 disassembly in `out/decomp/reflex-core.s` (`beginInit`,
`endInit`, the event handlers, `processAudio`) and `out/decomp/reflex-proc.s`
(`processFloatParameter`, `resume`, `process`). Where Ghidra and the assembly disagree
(Ghidra hides the cosine that `__sincosf_stret` returns in `s1` and mixes up the SIMD
lanes), this spec follows the assembly. Addresses are arm64 VM addresses in
`analysis/maschine_arm64`. The DECKARD port is `crates/engine/src/fx/reflex.rs`. It nulls
**bit-exactly** against Maschine (§5).

**Confidence.** **[exact]**: formula and constants read from code or assembly.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation:
- `v`: a parameter's normalized value (0..1) as stored in the project.
- `sr`: sample rate (`f32` in the kernel).
- All arithmetic is `f32`. `fma(a, b, c)` = `a·b + c` with one rounding: the kernel
  fuses exactly where this spec writes `fma`, and nowhere else.
- `rnd(x)`: add `+0.5` (when `x ≥ 0`) or `−0.5` to `x` **in `f64`**, truncate toward zero.

---

## 1. Parameters

### 1.1 Module

Module name "Reflex" (`Module::Module(L"Reflex")`, wide string at 0x10dce6770). One page:
`s_aPageNames = { "Main" }` (`__GLOBAL__sub_I_Modules_Effects_Reflex_ALL.cpp`, 0x101a63a10)
**[exact]**.

### 1.2 Parameters in constructor order

The constructor is `Reflex::Reflex()` at 0x101a61458 (`0011.c`). Each parameter is made by
`TransactionObjectContainer::insert(label, default, min, max)` **[exact]**. Labels are wide
strings at 0x104d330b8 / 0x104d330d0 / 0x104d330ec / 0x104d33100.

| obj off | mxprj field | label | section | default | range | RT idx | display converter |
|---|---|---|---|---|---|---|---|
| 0xa8 | `color` | **Color** | "Room" | 0.5 | 0..1 | 0 | `ScaleNormalizedToPercentOneDecimal` |
| 0xb0 | `smooth` | **Smooth** | (Room) | 0.75 | 0..1 | 1 | `ScaleNormalizedToPercentOneDecimal` |
| 0xb8 | `size` | **Size** | (Room) | 0.25 | 0..1 | 2 | `ScaleNormalizedToPercentOneDecimal` |
| 0xc0 | `mix` | **Mix** | "Output" | 0.5 | 0..1 | 3 | `ScaleNormalizedToPercentOneDecimal` |

All four are plain `MaschineNumericParameter<float>`: no enums, no switches. The display is
`v·100` with one decimal and a `%` sign (e.g. "75.0%").

### 1.3 Knob layout (`getParameterByPosition`, 0x101a619c8)

| pos | page | parameter |
|---|---|---|
| 0 | Main | Color |
| 1 | Main | Smooth |
| 2 | Main | Size |
| 3 | Main | Mix |
| 4–7 | Main | – (empty) |

Section headings: slot 0 "Room", slot 3 "Output" **[exact]**.

### 1.4 Serialization

`iserializer<…, Reflex>::load_object_data` (0x101a61f08) loads the Module base, then the
objects at 0xa8, 0xb0, 0xb8, 0xc0 in that order **[exact]**. In `crates/mxprj` they are
`color`, `smooth`, `size`, `mix` (were `unknown_a8/b0/b8/c0`).

### 1.5 Value → engine (`ReflexProcessor::processFloatParameter`, 0x101a61880)

Each parameter is bound with `setAndCallFuncAutomate` to `processFloatParameter(idx, …, v)`,
which stores `v` into a kernel input and runs that input's event handler inline
**[exact]**. Each handler writes glide **targets** (§3.3), sets the "changed" flag (+0xc0)
and a state word (+0xd0) to 1.

| idx | input | handler | targets (lane: value) |
|---|---|---|---|
| 0 | +0x20 Color | `processEventInColr` | 0: `freq`, 1: `gain` (below) |
| 1 | +0x24 Smooth | `processEventInSm` | 2: `damp = v·0.9`, 3: `fbk` (below) |
| 2 | +0x28 Size | `processEventInSize` | 4: `unit = fma(v, 0.9, 0.1)·sr·0.001` |
| 3 | +0x1c Mix | `processEventInMix` | 5: `dry`, 6: `wet` (below) |

Lane 7 is never written (0).

**Color** **[exact]**:
```
k    = v · fma(|v|, −0.3, 1.3)                        // 0 … 1, a gentle curve
freq = E(fma(fma(k, 90, 30), 5, 1381.8816))          // E = exp table, docs/fx-eq.md §2.1
gain = k > 0.66 ? ((k + (−0.66))·1.47059 + 1)·2.51197 : 2.51197
```
`E(5·n + 1381.8816)` is the frequency of MIDI note `n` (8.1758·2^(n/12) Hz), so the band
centre is note `30 + 90k`: **46 Hz** (Color 0) … 920 Hz (50 %) … **8.37 kHz** (100 %).
The input gain is 2.51197 (+8 dB) up to Color ≈ 58.7 % (`k = 0.66`), then rises to
3.768 (+11.5 dB) at 100 %.

**Smooth** **[exact]**:
```
i    = rnd(fma(v, 127, −0.5))                         // −1 at v = 0
fr   = fma(v, 127, −i)
fbk  = fma(T[i+1] − T[i], fr, T[i])                   // T = s_FbkArray
```
`s_FbkArray` (0x10dc23d10) has 128 `f32` entries, 7.94e-15 … 0.628907, all 6-significant
digit decimals (the port carries them). It rises steeply from ~0 to 0.19 over the first
half and flattens to 0.63. At `v = 0` the kernel reads `T[−1]`, the word before the table,
which is 0.0, so `fbk = T[0]`. At `v = 1` it reads `T[128]` (`0x01a1b370`) times a zero
fraction.

**Size**: `unit` is samples per delay unit: `sr·(0.1 … 1.0)/1000`, i.e. 0.1 ms … 1 ms.

**Mix** **[exact]**:
```
dry = (1 − v) > 0.5 ? 1 : (1 − v)·2
wet = v > 0.5 ? 1.41233 : (v + v)·1.41233
```
So the dry signal stays at unity up to 50 %, and the wet one is at full level (+3 dB) from
50 % on. Mix 0 is the identity (§3.5).

---

## 2. Processor (`ReflexProcessor`)

- **Constructor** (0x101a61a5c) **[exact]**: stereo in, stereo out. It allocates the kernel
  (0x70280 bytes, zeroed). There are no preset inputs; the module's stored values arrive
  through `processFloatParameter` before audio runs.
- **`onSamplerateChanged(sr)`** only stores `sr` (+0x48).
- **`resume()`** (0x101a61bd4) **[exact]**: it saves the four inputs, calls
  `beginInit(sr)` (zeroes the whole kernel, stores `sr`, sets the control period to 128,
  the control counter to 129), restores the inputs, sets all four event flags, and calls
  `endInit()` (§3.2).
- **`process()`** (0x101a61c20): per sample, `in L → +0xc`, `in R → +0x10`,
  `processAudio()`, then `+0x14 → out L`, `+0x18 → out R`, in place. No block state.

---

## 3. DSP (`FXCORE::ReflexCore`)

### 3.1 Signal flow

```
xL, xR ─×gain─► BP1 (two lanes) ─► L: nL = x'L − bpL     R: bpR
                                    │                       │
                                   BP2 ◄────────────────────┘ (two lanes, same coefs)
                                    │ zL                     │ zR
 e = 0.0398093·(nL − zL)            │                        ▼
                                    │       AP1(21u,0.7574) → AP2(8u,0.7019) → (+fbk·r6)
                                    │       → AP3(23u,0.7574) → AP4(18u,0.7019) = v4
                                    │       → AP5(43u,0.6613) = v5 → D6(110u) = r6 ─┐
                                    │                                 ▲ feedback ──┘
 wetL = fma(r6 + v5, 0.5, e)            wetR = e + D7(39u)(v4)
 out = dry·x + wet·wet{L,R}
```

The reverb is fed by the **right input only**. The left input reaches the wet signal only
as `e`, 0.04 × the part of it that both band-passes reject, which is quiet (about 10 dB
below the right channel's wet in a white-noise test, and ~0 at the band centre). This is
the kernel as compiled, confirmed by the null tests with left-only and right-only inputs
(§5).

### 3.2 `endInit()` (0x101a1802c) **[exact]**

```
steps = trunc(sr·0.02/128 + 0.5), at least 1        // 7 at 44.1/48 k, 15 at 96 k, 3 at 22.05 k
inv   = 1 / (steps·128)
targets from the four inputs (§1.5), current = targets, changed flag = 1
gain = cur1, dry = cur5, wet = cur6, fbk = cur3, unit = cur4
band-pass coefficients from (cur0, cur2) (§3.4)
```
The delay lines, filter states and write index are zero (from `beginInit`).

### 3.3 Glides (top of `processAudio`) **[exact]**

All eight lanes glide linearly over `steps·128` samples (≈ 20 ms). The kernel keeps
`cur[8]`, `inc[8]`, `tgt[8]`:
```
counter += 1                                          // starts at 129
if counter ≥ 128:
  counter = 0
  if changed:
    changed = 0
    for each lane: inc = (tgt − cur)·inv
                   if (f64)(inc·inc) < 1e-15: inc = 0, cur = tgt   // tiny moves snap
    if any inc ≠ 0: countdown = steps, gliding = 1
  else:
    countdown −= 1
    if countdown == 0: cur = tgt, inc = 0, gliding = 0
every sample:  cur += inc
if gliding:
  if inc0 ≠ 0 or inc2 ≠ 0: coefficients from (cur0, cur2); gain = cur1; fbk = cur3
  if inc5 ≠ 0 or inc6 ≠ 0: dry = cur5; wet = cur6
  if inc4 ≠ 0: unit = cur4
```
Two details matter for exactness: the first sample after `resume` runs the "changed"
branch with nothing to do, and when a glide ends the snapped targets are **not** applied
(the working values keep the last glided sample's values, within float rounding of the
targets). A new event during a glide restarts it from the current values at the next
control tick.

### 3.4 Band-pass ("Color") **[exact]**

Coefficients from frequency `f` and damping `d`:
```
f = max(f, sr·4.0690105e-5);  f = min(f, sr·0.47058824)
(s, c) = __sincosf_stret(f·6.28319 / sr)
t = (1 − d)·s;  n = 1/(t + 1)
b0 = t·n;  b1 = n·0;  b2 = −(t·n);  a1 = (c + c)·n;  a2 = −((1 − t)·n)
```
This is the constant-0-dB-peak band-pass with `α = t`, i.e. `Q = 1/(2(1 − 0.9·Smooth))`:
0.5 at Smooth 0, 1.54 at 75 %, 5 at 100 %. Each stage is direct form I, unfused:
```
y = ((b0·x + b1·x1) + b2·x2) + (a1·y1 + a2·y2)
```
Stage 1 runs on `(gain·xL, gain·xR)`; stage 2 (same coefficients, own state) on
`(gain·xL − y1L, y1R)`.

`__sincosf_stret` matters: its cosine differs from `cosf` in the last bit for ~0.7 % of
`f32` inputs, so the port calls it on Apple platforms (and falls back to `sin`/`cos`
elsewhere, which is then not bit-exact).

### 3.5 All-passes, delays and output **[exact]**

Seven 16384-sample lines (at +0x1d0 + k·0x10000), one write index `w` (masked 0x3fff).
Delays are `n_k = rnd(unit·K_k)` with `K = 21, 8, 23, 18, 43, 110, 39`: at Size 0 they span
2.1 … 11 ms, at Size 100 % 21 … 110 ms (independent of sr). Each all-pass keeps the read
from the previous sample (`r_k`); the read happens right after the write, so a line of
`n` gives `n + 1` samples of delay. Per sample, with `x = zR`:
```
v1 = fma(0.7574, x, r1);   L1[w] = x − v1·0.7574;   r1 = L1[w − n1]
v2 = fma(v1, 0.7019, r2);  L2[w] = v1 − v2·0.7019;  r2 = L2[w − n2]
i3 = fma(fbk, r6, v2)                                  // r6 from the previous sample
v3 = fma(i3, 0.7574, r3);  L3[w] = i3 − v3·0.7574;  r3 = L3[w − n3]
v4 = fma(v3, 0.7019, r4);  L4[w] = v3 − v4·0.7019;  r4 = L4[w − n4]
v5 = fma(v4, 0.6613, r5);  L5[w] = v4 − v5·0.6613;  r5 = L5[w − n5]
L6[w] = v5;  r6 = L6[w − n6]
L7[w] = v4;  r7 = L7[w − n7]
e    = 0.0398093·(nL − zL)
wetL = fma(r6 + v5, 0.5, e)
wetR = e + r7
outL = dry·xL + wet·wetL;   outR = dry·xR + wet·wetR;   w = (w + 1) & 0x3fff
```
(The all-passes are lattice form: output `v = r + g·in`, write `in − g·v`.) The feedback
loop (AP3 → AP5 → D6 → AP3, gain `fbk ≤ 0.63`) sets the decay: at Size 100 % and Smooth
100 % an impulse is down ~70 dB after 3 s (96 kHz unit test).
At 192 kHz and Size > 75 % the 110-unit line exceeds 16384 samples and wraps, in Maschine
as well.

---

## 4. Implementation notes

- `Reflex::set` does `resume()` on the first call. Later calls send an event only for a
  value that changed (as Maschine does), so the glide restarts only then.
- The port keeps the kernel's operation order and uses `mul_add` exactly where it fuses.
- Not verified against Maschine: parameter changes during a render (glides), because
  `maschine-ref` has no automation. The glide code follows the assembly; unit tests check
  that re-sending the same values changes nothing and that Mix glides over 7·128 samples.
- Not used: side-chain, tempo, transport, meters (the processor has none).

## 5. Null tests

Source project: `/Users/Shared/Maschine 2 Library/Projects/Chances.mxprj`, sound 3/0
("HiTom Sepolia") slot 1 (Reflex, stored color 0.49 / smooth 0.86 / size 0.39 / mix 0.5),
made with `maschine-ref make … 3 0 1 [field=value …]`, rendered at −12 dBFS with
`--preroll 0`, compared against `fx-null` on the captured input (scratch script
`null.sh`). Residuals are L / R; **−240 dB means bit-identical** (peak difference 0).
Fields not listed keep the project values.

| case | input | sr | color / smooth / size / mix | s | residual |
|---|---|---|---|---|---|
| project values | noise | 44.1 k | project | 3 | −240 / −240 |
| defaults | noise | 44.1 k | 0.5 / 0.75 / 0.25 / 0.5 | 3 | −240 / −240 |
| defaults | noise | 48 k | 0.5 / 0.75 / 0.25 / 0.5 | 3 | −240 / −240 |
| tail | impulse | 44.1 k | 0.5 / 0.75 / 0.25 / 1 | 4 | −240 / −240 |
| longest tail | impulse | 44.1 k | 0.5 / 1 / 1 / 1 | 6 | −240 / −240 |
| | sweep | 44.1 k | project | 3 | −240 / −240 |
| | sine 440 | 48 k | project | 3 | −240 / −240 |
| color min | noise | 44.1 k | 0 / – / – / – | 3 | −240 / −240 |
| color max | noise | 44.1 k | 1 / – / – / – | 3 | −240 / −240 |
| color above the gain knee | noise | 44.1 k | 0.7 / – / – / – | 3 | −240 / −240 |
| smooth min (`T[−1]`) | noise | 44.1 k | – / 0 / – / – | 3 | −240 / −240 |
| smooth max | noise | 44.1 k | – / 1 / – / – | 3 | −240 / −240 |
| size min | noise | 44.1 k | – / – / 0 / – | 3 | −240 / −240 |
| size max | noise | 44.1 k | – / – / 1 / – | 3 | −240 / −240 |
| mix min | noise | 44.1 k | – / – / – / 0 | 3 | −240 / −240 |
| mix max | noise | 44.1 k | – / – / – / 1 | 3 | −240 / −240 |
| | noise | 44.1 k | – / – / – / 0.3 | 3 | −240 / −240 |
| odd values | noise | 48 k | 0.123 / 0.877 / 0.333 / 0.777 | 3 | −240 / −240 |
| 15 glide steps | noise | 96 k | 0.9 / 0.95 / 0.8 / 0.6 | 3 | −240 / −240 |
| 3 glide steps | noise | 22.05 k | 0.2 / 0.4 / 0.6 / 0.6 | 3 | −240 / −240 |
| left input only | noise L, silent R | 44.1 k | – / – / – / 1 | 3 | −240 / −240 (out −34 dB) |
| right input only | silent L, noise R | 44.1 k | – / – / – / 1 | 3 | −240 / −240 (out −25/−23 dB) |
| `--preroll 1`, `FX_PREROLL=1` | impulse | 48 k | 0.123 / 0.877 / 0.333 / 0.777 | 4 | −240 / −240 |
