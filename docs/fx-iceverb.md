# Maschine's Ice (Iceverb): parameters and DSP

A clean-room description of Maschine 3's built-in **Ice** effect
(`NI::MASCHINE::DATA::Iceverb` → `NI::MASCHINE::RT::IceverbProcessor` →
`NI::MASCHINE::FXCORE::IceverbCore`). `IceverbCore` is a small compiled Reaktor-Core kernel:
a flat struct of inputs, outputs and state, with `beginInit(sr)`, `endInit()`,
`processAudio()` (one sample per call) and one `processEventIn*()` per parameter.

It was written from the arm64 disassembly of the kernel (`out/decomp/iceverb-core.s`,
0x101a15998–0x101a1692c) and of the data/processor classes (`out/decomp/iceverb-data.s`).
Ghidra was not needed: the project was locked by another run, and the kernel is short
enough to read in assembly. Addresses are arm64 VM addresses in `analysis/maschine_arm64`.
The DECKARD port is `crates/engine/src/fx/iceverb.rs`. It nulls **bit-exactly** against
Maschine (§5).

**Confidence.** **[exact]**: formula and constants read from the assembly.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation:
- `v`: a parameter's normalized value (0..1) as stored in the project.
- `sr`: sample rate (`f32` in the kernel).
- All arithmetic is `f32` unless the text says `f64`. Unlike Chorus, this kernel **uses fused
  multiply-adds**: `fma(a, b, c)` = `a·b + c` with one rounding (`fmadd`/`fmla`).
- `rnd(x)`: the kernel's rounding. It adds `+0.5` (when `x ≥ 0`) or `−0.5` to `x` **in
  `f64`** and truncates toward zero (`fcvtzs`). So halves round away from zero.

---

## 1. Parameters

### 1.1 Module

Class `Iceverb`, mxprj module name "Iceverb". `getInfo()` (0x101a420fc) returns the display
name **"Ice"** (id 21). One page: `s_aPageNames = { "Main" }`
(`__GLOBAL__sub_I_Modules_Effects_Iceverb_ALL.cpp`, 0x101a43dfc; string at 0x104d2e1f0)
**[exact]**.

### 1.2 Parameters in constructor order

The constructor is `Iceverb::Iceverb()` at 0x101a4154c. Each parameter is made by
`TransactionObjectContainer::insert(label, default, min, max)` **[exact]**. Labels are wide
strings at 0x104d2e214 / 0x104d2e204 / 0x104d2e22c / 0x104d2e240.

| obj off | mxprj field | label | section | default | range | RT idx | display converter |
|---|---|---|---|---|---|---|---|
| 0xa8 | `color` | **Color** | "Room" | 0.75 | 0..1 | 0 | `ScaleNormalizedToPercentOneDecimal` |
| 0xb0 | `ice` | **Ice** | (Room) | 0.75 | 0..1 | 1 | `ScaleNormalizedToPercentOneDecimal` |
| 0xb8 | `size` | **Size** | (Room) | 0.5 | 0..1 | 2 | `ScaleNormalizedToPercentOneDecimal` |
| 0xc0 | `mix` | **Mix** | "Output" | 0.5 | 0..1 | 3 | `ScaleNormalizedToPercentOneDecimal` |

All four are plain `MaschineNumericParameter<float>`: no enums or switches. The converter
(0x10dce2238) is set on all four; the display is `v·100` with one decimal and a `%` sign.
The section names are set with `setSectionName` on Color ("Room", 0x104d2e250) and Mix
("Output", 0x104d2e264).

### 1.3 Knob layout (`getParameterByPosition`, 0x101a41c50)

| pos | page | parameter |
|---|---|---|
| 0 | Main | Color |
| 1 | Main | Ice |
| 2 | Main | Size |
| 3 | Main | Mix |
| 4–7 | Main | – (empty) |

Section headings: slot 0 "Room", slot 3 "Output" **[exact]**.

### 1.4 Serialization

`iserializer<…, Iceverb>::load_object_data` (0x101a422f4) loads the Module base, then the
objects at 0xa8, 0xb0, 0xb8, 0xc0 in that order **[exact]**. These are the `crates/mxprj`
fields `color`, `ice`, `size`, `mix` (were `unknown_a8/b0/b8/c0`).

### 1.5 Value → engine (`IceverbProcessor::processFloatParameter`, 0x101a41a74)

`connectParameters` binds each parameter to `processFloatParameter(idx, …, v)`, which stores
`v` as is into a kernel input and calls its handler **[exact]**:

| idx | parameter | kernel input | handler |
|---|---|---|---|
| 0 | Color | +0x0 | `processEventInColor` (0x101a15d50) |
| 1 | Ice | +0x4 | `processEventInIce` (0x101a160d0) |
| 2 | Size | +0x8 | `processEventInSize` (0x101a15ce4) |
| 3 | Mix | +0xc | `processEventInMix` (0x101a161f4) |

---

## 2. Processor (`IceverbProcessor`)

- **Constructor** (0x101a41dec) **[exact]**: stereo in, stereo out. It allocates the kernel
  (0x180 bytes, plus seven 64 KiB delay lines) and presets the inputs to Color 0.677165,
  Ice 0.9, Size 0.66, Mix 0.5 (0x104d2e1d0). The module's stored values replace these
  before audio runs.
- **`onSamplerateChanged(sr)`** only stores `sr` (+0x48), used at the next `resume`.
- **`resume()`** (0x101a41fc0) **[exact]**: saves the four inputs, calls
  `beginInit(sr)`, restores the inputs, sets the four "event" flags (+0x10..+0x13) and calls
  `endInit()`.
- **`process()`** (0x101a4200c): per sample, `in L → +0x14`, `in R → +0x18`,
  `processAudio()`, then `+0x1c → out L`, `+0x20 → out R`. In place, no block state.

---

## 3. DSP (`FXCORE::IceverbCore`)

### 3.1 Signal flow

```
in L + in R ─► ×gain ─► band-pass ─► band-pass ─► AP1 ─► AP2 ─►(+)─► AP3 ─► AP4 ─► AP5 ─┬─► AP6 ─► delay ─┬─► ×1.2 ─► wet R
  (mono sum)          (Color, Ice: two equal biquads)               ▲                       │                   │
                                                                      └──── ×feedback ◄───────┼───────────────────┘
                                                                                             └─► wet L
out L = in L·dry + wet·(AP5 output);   out R = in R·dry + wet·1.2·(delay output)
```

Seven delay lines of 16384 samples (`+0x88..+0xb8`), indices masked with 16383, reads
clamped to `[0, 16382]` samples **[exact]**.

### 3.2 Targets (event handlers, `beginInit`, `endInit`)

There are eight ramped values (§3.3), each with a target (+0x130..+0x14c). **[exact]**:

| # | value | target |
|---|---|---|
| 0 | dry | `dm = 1 − Mix`; `dm > 0.5 ? 1 : dm + dm` |
| 1 | wet | `Mix > 0.5 ? 1.4125375 : (Mix + Mix)·1.4125375` (0x3fb4ce07, +3 dB) |
| 2 | input gain | `boost · lerp(GAIN, pitch)` (below) |
| 3 | ω | `(2π'/sr) · clamp(E(fma(pitch, 5, 1381.8816)), sr/24576, sr/2.125)` |
| 4 | damping | `1 − fma(Ice, 0.8, 0.1)` |
| 5 | feedback | `fma(Ice·(2 − Ice), 0.66, 0.2)` |
| 6 | loop factor | `fma(Size·(2 − Size), 98, 12)` |
| 7 | delay unit | `(fma(Size, 0.9, 0.1)·sr)·0.001` (samples per "unit" = 0.1…1 ms) |

- `pitch = fma(Color, 90, 30)` (+0x34): a MIDI note from 30 to 120.
- `E(x)` is `NI::AB::ExpTable` (the EQ's table, `fx::exp_e`, docs/fx-eq.md §2.1),
  `≈ 2^((x − 1200)/60)`; with `x = 5·pitch + 1381.8816` it is `440·2^((pitch − 69)/12)` Hz.
  So Color spans 46.2 Hz (0 %) … 2283 Hz (75 %) … 8372 Hz (100 %).
- `2π' = 6.2831898` (0x40c90fe4, slightly above 2π).
- `boost = Ice² ≤ 0.8 ? 1.12 : (f32)((f64)(((Ice² − 0.8)·5.0000005)²·0.66) + 1.12)`.
- `lerp(GAIN, p)`: `i = rnd(p − 0.5)`, `f = p − i`, `fma(G[i+1] − G[i], f, G[i])` over
  `IceverbCore::s_fGainTable` (0x10dc23b08, 128 floats, copied as bits). It rises from
  0.346 (index 0) to ≈ 1 at notes 73–105, then falls to 0.83 at 121.
- Color's handler sets ω and the input gain; Ice's sets damping, feedback and the input
  gain; Size's sets the loop factor and unit; Mix's sets dry and wet. Each handler sets the
  "changed" flag (+0x150).

`beginInit(sr)` (0x101a15b10) zeroes the biquad state, all-pass state, the seven lines,
the ramps and the write index; sets the "coefficients" flag (+0x28); counter = 128;
`steps = max(1, (i32)(sr·0.1/128 + 0.5))` (34 at 44.1 k, 38 at 48 k);
`k = 1/(steps·128)`; the frequency clamps and `2π'/sr`; then computes all targets.
`endInit()` (0x101a15ebc) recomputes them and **copies all targets into the current values**,
so the effect starts at its settings with no ramp **[exact]**.

### 3.3 Ramps (control block every 128 samples)

`processAudio` increments the counter; when it reaches 128 it resets it and **[exact]**:
```
if changed:
    changed = false
    for each i: inc[i] = (tgt[i] − cur[i])·k
                if (f64)(inc[i]·inc[i]) < 1e-15: inc[i] = 0, cur[i] = tgt[i]
    if any inc ≠ 0: ramp_ctr = steps, ramping = true
else:
    ramp_ctr −= 1
    if ramp_ctr == 0: cur = tgt, inc = 0, ramping = false
```
Then, every sample, `cur[i] += inc[i]` for all eight. So a change glides linearly over
`steps·128` samples (≈ 99–100 ms) and then snaps to the target. (A status word at +0x160
is also updated; nothing reads it in the audio path.)

### 3.4 Band-pass coefficients

Computed on the first sample after `beginInit`, and on every sample while `ramping` with
`inc[ω] ≠ 0` or `inc[damping] ≠ 0` **[exact]**:
```
w2   = ω·ω
sin  = ω · fma(w2, fma(w2, fma(w2, fma(w2, fma(w2, −2.39e-8, 2.7525e-6), −1.98409e-4), 8.33333e-3), −0.166667), 1)
cos  = fma(w2, fma(w2, fma(w2, fma(w2, −2.605e-7, 2.47609e-5), −1.38884e-3), 4.16666e-2), −0.499923), 1)
alpha = damping·sin;   a = 1/(alpha + 1)
c0 = alpha·a;  c1 = (−alpha)·a;  c2 = (cos + cos)·a;  c3 = (−(1 − alpha))·a
```
(The sine polynomial is `fma` chained from the innermost constant: 0xb2cd4ca8, 0x3638b797,
0xb9500c03, 0x3c088885, 0xbe2aaac1, 1; the cosine: 0xb48bdad9, 0x37cfb5a1, 0xbab609bd,
0x3d2aaa99, 0xbefff5e8, 1.) This is the RBJ constant-peak band-pass with
`1/(2Q) = damping`: Q ≈ 0.56 at Ice 0, 1.67 at 75 %, 5 at 100 %.

### 3.5 Per sample

**[exact]**, with `DC = 1e-24` (0x179abe15):
```
x  = (inL + inR)·gain
y1 = fma(c3, y1[n−2], fma(c2, y1[n−1], fma(c0, x,  c1·x[n−2])))  + DC
y2 = fma(c3, y2[n−2], fma(c2, y2[n−1], fma(c0, y1, c1·y1[n−2]))) + DC
w  = (w + 1) & 16383                                   // write index
AP_k(in):  out = fma(in, 0.5, z_k) + DC
           line_k[w] = fma(out, −0.5, in)
           z_k = line_k[w − rnd(clamp(L_k·unit, 0, 16382))]   // used on the next sample
o1 = AP1(y2); o2 = AP2(o1)
o3 = AP3(fma(z_7, feedback, o2))                       // z_7: last loop-delay read
o4 = AP4(o3); o5 = AP5(o4); o6 = AP6(o5)
line_7[w] = o6;  z_7 = line_7[w − rnd(clamp(loop·unit, 0, 16382))]
outL = fma(inL, dry, o5·wet)
outR = fma(inR, dry, (z_7·wet)·1.2)                     // 1.2 = 0x3f99999a
```
All-pass lengths `L_k` = 27, 8, 78, 27, 13, 29 units. The reads come after the write, so a
read of `n` samples is used one sample later (a delay of `n + 1`). Physical sizes: the unit
is 0.1 ms (Size 0) … 0.55 ms (50 %) … 1 ms (100 %); the loop delay is `12…110` units
(1.2 ms at Size 0, 47.0 ms at 50 %, 110 ms at 100 %), and the loop also holds AP3–AP6
(147 units). With the loop gain 0.2 (Ice 0) … 0.82 (75 %) … 0.86 (100 %), Ice sets both
the resonance of the band-pass and the decay time.

Mix 0 gives dry 1 and wet 0, so the output is bit-identical to the input.
At 192 kHz, Size 100 % (loop 21120 samples) exceeds the 16382-sample clamp, in Maschine as well.

---

## 4. Implementation notes

- `Iceverb::set` does `resume()` on the first call. Later calls send the handler of each
  parameter whose value changed (a repeated event would restart a running ramp from its
  current position, so unchanged values are not re-sent).
- The port keeps the kernel's operation order, `mul_add` exactly where the kernel has
  `fmadd`/`fmla`, and `f64` only where it has doubles (the boost, the ramp snap test,
  rounding).
- Not verified against Maschine: parameter changes during a render (`maschine-ref` has no
  automation). The ramp code follows the assembly; a unit test checks the glide and that
  re-sending the same values changes nothing.
- No side-chain, tempo, transport or meters.

## 5. Null tests

Source project: `/Users/Shared/Deep Matter Library/Projects/Sonnenallee.mxprj`, sound 0/1
slot 1 (Iceverb, stored color 0.39 / ice 1 / size 0.09 / mix 0.16), made with
`maschine-ref make … 0 1 1 [color=… ice=… size=… mix=…]`, rendered with `--preroll 0` at
−12 dBFS. Compared against `fx-null` on the captured input. Residuals are L / R;
**−240 dB means bit-identical** (peak difference 0). Scratch script: `nt.sh` in the
agent's scratch directory.

| case | input | sr | color / ice / size / mix | len | residual |
|---|---|---|---|---|---|
| project values | impulse | 48 k | 0.39 / 1 / 0.09 / 0.16 | 4 s | −240 / −240 |
| project values | impulse | 44.1 k | 0.39 / 1 / 0.09 / 0.16 | 6 s | −240 / −240 |
| project values | noise | 44.1 k | 0.39 / 1 / 0.09 / 0.16 | 4 s | −240 / −240 |
| project values | sweep | 48 k | 0.39 / 1 / 0.09 / 0.16 | 4 s | −240 / −240 |
| project values | sine 440 | 48 k | 0.39 / 1 / 0.09 / 0.16 | 3 s | −240 / −240 |
| defaults | noise | 44.1 k | 0.75 / 0.75 / 0.5 / 0.5 | 4 s | −240 / −240 |
| defaults | impulse | 48 k | 0.75 / 0.75 / 0.5 / 0.5 | 6 s | −240 / −240 |
| color min | noise | 44.1 k | 0 / 0.75 / 0.5 / 1 | 3 s | −240 / −240 |
| color max | noise | 44.1 k | 1 / 0.75 / 0.5 / 1 | 3 s | −240 / −240 |
| color mid | noise | 48 k | 0.5 / 0.75 / 0.5 / 1 | 3 s | −240 / −240 |
| ice min | noise | 44.1 k | 0.6 / 0 / 0.5 / 1 | 3 s | −240 / −240 |
| ice max, size max (longest tail) | impulse | 44.1 k | 0.6 / 1 / 1 / 1 | 8 s | −240 / −240 |
| ice boost region | noise | 44.1 k | 0.6 / 0.9 / 0.5 / 1 | 3 s | −240 / −240 |
| size min | noise | 44.1 k | 0.6 / 0.6 / 0 / 1 | 3 s | −240 / −240 |
| size max | noise | 48 k | 0.6 / 0.6 / 1 / 1 | 3 s | −240 / −240 |
| mix min | noise | 44.1 k | 0.6 / 0.6 / 0.5 / 0 | 3 s | −240 / −240 |
| mix max | noise | 44.1 k | 0.6 / 0.6 / 0.5 / 1 | 3 s | −240 / −240 |
| mix 0.3 | noise | 44.1 k | 0.6 / 0.6 / 0.5 / 0.3 | 3 s | −240 / −240 |
| mix 0.7 | sweep | 44.1 k | 0.6 / 0.6 / 0.5 / 0.7 | 3 s | −240 / −240 |
| odd values | noise | 48 k | 0.123 / 0.877 / 0.333 / 0.55 | 3 s | −240 / −240 |
| | noise | 96 k | 0.9 / 0.95 / 1 / 1 | 3 s | −240 / −240 |
| | noise | 22.05 k | 1 / 0.5 / 0.7 / 1 | 3 s | −240 / −240 |
| delay clamp | impulse | 192 k | 0.2 / 1 / 1 / 1 | 3 s | −240 / −240 |
| `--preroll 1`, input fed as is | impulse | 44.1 k | 0.6 / 1 / 1 / 1 | 8 s | −240 / −240 (peak 3.9e-22) |

With a preroll, Maschine's lines fill with the `1e-24` guard offsets during the silent
second, which leaves a difference of at most 4e-22 (−427 dB); otherwise the preroll changes
nothing, since the effect has no free-running modulation.
