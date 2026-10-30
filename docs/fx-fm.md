# Maschine's FM: parameters and DSP

A clean-room description of Maschine 3's built-in **FM** effect
(`NI::MASCHINE::DATA::FM` → `NI::MASCHINE::RT::FMProcessor` →
`NI::MASCHINE::FXCORE::FrequencyModulatorCore`). Like Chorus, the core is a compiled
Reaktor-Core kernel: a flat struct of inputs, outputs and state, with `endInit()`,
`processAudio()` (one sample per call) and one `processEventIn*()` per parameter.

It was written from Ghidra output in `out/decomp/fm/` (`INDEX.tsv` maps files to functions;
`0017` `processAudio`, `0018–0021` the events, `0022` `endInit`, `0001` the static init,
`0065` the loader) and from arm64 disassembly: `out/decomp/fm-asm/data-proc.s` (data class
and processor) and `out/decomp/fm-core.s` (the kernel). Where the two disagree on order,
this spec follows the assembly. Addresses are arm64 VM addresses in `analysis/maschine_arm64`.
The DECKARD port is `crates/engine/src/fx/fm.rs`. It nulls **bit-exactly** against Maschine (§5).

**Confidence.** **[exact]**: formula and constants read from code or assembly.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation: `v` is a parameter's stored value (0..1). All arithmetic is `f32` unless the text
says `f64`; the kernel has **no fused multiply-adds**. `rnd(x)` adds ±0.5 (by the sign of
`x`, `x < 0` → −0.5) in `f64` and truncates (`fcvtzs`). `TINY = 0x219392ef` (≈ 1e-18) is the
kernel's anti-denormal offset; `TINY_D = 1e-100`.

---

## 1. Parameters

### 1.1 Module

Module name "FM" (`Module::Module(L"FM")`). One page: `s_aPageNames = { "Main" }`
(`__GLOBAL__sub_I_Modules_Effects_FM_ALL.cpp`, 0x101a0fd28) **[exact]**.

### 1.2 Parameters in constructor order

`FM::FM()` at 0x101a0d75c, `TransactionObjectContainer::insert(label, default, min, max)`.
Labels are wide strings at 0x104d28f6c / f80 / f98 / fb8; section names at 0x104d28fd4 /
fe8 **[exact]**:

| obj off | mxprj field | label | section | default | range | RT idx | display converter |
|---|---|---|---|---|---|---|---|
| 0xa8 | `rate` | **Rate** | "Freq" | 0.25 | 0..1 | 0 | `ScaleNormalizedToPercentOneDecimal` |
| 0xb0 | `split` | **Split** | (Freq) | 0.75 | 0..1 | 1 | `ScaleNormalizedToPercentOneDecimal` |
| 0xb8 | `contour` | **Contour** | "Depth" | 0.75 | 0..1 | 2 | `ScaleNormalizedToPercentOneDecimal` |
| 0xc0 | `amount` | **Amount** | (Depth) | 0.75 | 0..1 | 3 | `ScaleNormalizedToPercentOneDecimal` |

All four are plain `MaschineNumericParameter<float>`; no enums or switches. Display:
`v·100`, one decimal, `%` (converter at 0x10dce2238). The physical values of §3 are never shown.

### 1.3 Knob layout (`getParameterByPosition`, 0x101a0dcc0)

| pos | page | parameter |
|---|---|---|
| 0 | Main | Rate |
| 1 | Main | Split |
| 2 | Main | Contour |
| 3 | Main | Amount |
| 4–7 | Main | – (empty) |

Section headings: slot 0 "Freq", slot 2 "Depth" **[exact]**. (The software editor,
`FMEditor.qml`, may arrange them differently; the hardware/parameter order is the above.)

### 1.4 Serialization

`iserializer<…, FM>::load_object_data` (0x101a0e220) loads the Module base, then 0xa8,
0xb0, 0xb8, 0xc0 **[exact]**. In `crates/mxprj` they were `unknown_a8/b0/b8/c0`, now
`rate`, `split`, `contour`, `amount`.

### 1.5 Value → engine (`FMProcessor::processFloatParameter`, 0x101a0db78)

`v` is stored as is into a kernel input and its event handler is called **[exact]**:

| idx | parameter | input | handler | target (f64, from `f32` math) |
|---|---|---|---|---|
| 0 | Rate | +0x18 | `processEventInRate` (0x101a148d8) | `rate_t = v` |
| 1 | Split | +0x10 | `processEventInSpl` (0x101a148f4) | `split_t = exp2₃((v·112 + 24)·0.0833292)·8.17742` Hz (MIDI note 24 + 112v) |
| 2 | Contour | +0x2c | `processEventInCont` (0x101a149b8) | `cont_t = 0.9 − (v·v)·0.9` |
| 3 | Amount | +0x20 | `processEventInAmt` (0x101a149ec) | `amt_t = v·exp2₃((v·40 − 40)·0.166096) + 0` (= v·10^((40v−40)/20)) |

Each also sets a flag bit in +0x48; the flags only gate the control block, and `endInit`
leaves them non-zero for good.

---

## 2. Processor (`FMProcessor`)

- **Constructor** (0x101a0dd54): stereo in/out, allocates the kernel (0x13a90 bytes,
  zeroed). Unlike Chorus it does not preset any input.
- **`onSamplerateChanged(sr)`** stores `sr` (+0x48), used at the next `resume`.
- **`resume()`** (0x101a0deb0) **[exact]**: saves the four inputs, zeroes the whole kernel,
  writes `sr`, the inputs and their event flags (1.0), and calls `endInit()`.
- **`process()`** (0x101a0df38): per sample `L → +0xc`, `R → +0x28`, `processAudio()`,
  `+0x34 → L`, `+0x38 → R`. In place; no block state.

The oscillator free-runs from `resume` (not tied to the transport). With
`maschine-ref render --preroll S`, DECKARD must be fed the same `S·sr` zeros first (§5).

---

## 3. DSP (`FXCORE::FrequencyModulatorCore`)

### 3.1 Tables and approximations

128-entry tables, all `%.6g` decimal values **[exact]**, copied as numbers into the port:

| symbol | addr | content | port |
|---|---|---|---|
| `s_042F3160` | 0x10dc21300 | **oscillator frequency, Hz**: 46.2475 … 8370.79 (≈ ×2^(1/17) per step) | `RATE_HZ` |
| `s_04306C10` | 0x10dc21500 | **depth factor**: 1.0 up to idx 59, then falling to 0.0625 | `DEPTH` |
| `s_042FF608` | 0x10dc21700 | **peak filter bandwidth term**: 1.0 up to idx 42, falling to 0.1 from idx 99 | `PEAK_Q` |
| `s_0430C3D0` | 0x10dc21900 | **peak filter gain, dB** (×Amount): 0, −0.378 … −24 from idx 64 | `PEAK_DB` |
| `s_0303B510` | 0x10dc22b04 | the 1023-entry octave table of `exp2`, **identical** to Chorus' `s_04FB03F0` | `exp2_entry` (copied from chorus.rs) |

Three `2^x` approximations; all split `x = n + f` with `n = rnd(x)` and build `2^n` by adding
`n<<23` to the bits of 1.0:
- `exp2₃` (events): `(f·(f·(f·0.056 + 0.242) + 0.693) + 1)·2^n` (constants 0x3d656042,
  0x3e77ced9, 0x3f316873).
- `exp2₂` (control block): `(f·(f·0.248 + 0.708) + 1)·2^n` (0x3e7df3b6, 0x3f353f7d).
- `exp2_tab` (coefficients): the Chorus table interpolation (docs/fx-chorus.md §3.1; here
  `n` uses `fcmge`, so NaN → −0.5).

Sin/cos (split filter), series in `z = w²`:
`sin = w·(z·(z·(z·(z·(z·−2.39e-8 + 2.7526e-6) − 1.98409e-4) + 8.33333e-3) − 0.166667) + 1)`,
`cos = z·(z·(z·(z·(z·−2.605e-7 + 2.47609e-5) − 1.38884e-3) + 0.0416666) − 0.499923) + 1`
(bit patterns in fm.rs `SIN`/`COS`). Tan (peak filter): `g = g0 + (g0³·0.333333 + g0⁵·0.133333)`.

### 3.2 `endInit()` (0x101a14ab0)

**[exact]**:
```
steps   = max(rnd_f64(sr/9800), 1)            ; period = steps − 1  (5 at 44.1/48 k)
per_ms  = (rnd(sr) / steps) / 1000            // integer division, then f64
L       = (0.310295·0.9602356 + 0.311434·0.039764404) + (−61)
c_amt   = 1 − (f64)exp2_tab((f32)(L / max(500·per_ms − 1, 1)))      // Amount: 500 ms
c_other = 1 − (f64)exp2_tab((f32)(L / max(100·per_ms − 1, 1)))      // Rate, Split, Contour: 100 ms
ms2     = (sr + sr)/1000                      // half-samples per ms
lo, hi  = sr/24576, sr/2.125                  // Split clamp
w_k = 6.28319/sr;  pi_sr = 3.14159/sr;  inv_sr = 1/sr
Le      = (0.355172·0.120025635 + 0.356276·0.87997437) + (−7)
att     = 1 − exp2_tab(Le / max(12·sr/1000 − 1, 1))       // f32; envelope attack 12 ms
rel     = 1 − exp2_tab(Le / max(70·sr/1000 − 1, 1))       // release 70 ms
targets from the inputs (§1.5); smoothed values start at their targets
phase = −0.5; everything else 0
```
It then runs the control block (§3.3) once without touching the counter, and the per-sample
code once on silence (output dropped). The port does exactly this (`tick(0, 0, init=true)`).

### 3.3 Control block (every `steps` samples)

The counter increments each sample; when it has reached `period` it resets and **[exact]**:
```
each smoother: s += c·((t − s) + 1e-100)       (f64)  — amount, contour, split, rate
amt = (f32)amt_s; cont = (f32)cont_s; f = (f32)split_s; r = (f32)rate_s
i = rnd(127r − 0.5); fr = 127r − i; i0 = min_u(i,127); i1 = (u32)(i+1) ≥ 127 ? 127 : i+1
depth_k = lerp(DEPTH)
e1 = exp2₂(lerp(PEAK_Q)·0.5)
e2 = exp2₂(lerp(PEAK_DB)·amt·0.0830431)       // 10^(dB·amt/40)
k  = (e1·e1 − 1)/(e1·e2);  pk_gain = k·(e2·e2 − 1)
hz = lerp(RATE_HZ);  inc = hz·inv_sr
g  = tan≈(pi_sr·hz);  pk_a2 = (g² + 1) − g·k;  pk_n = 1/((g² + 1) + g·k);  pk_a1 = 2(g² − 1)
f  = clamp(f, lo, hi);  w = f·w_k
a  = 1/(sin + 1);  a2 = −((1 − sin)·a);  a1 = (cos + cos)·a;  b1 = (1 − cos)·a;  b0 = ((1 − cos)·0.5)·a
```
The split filter is a 2nd-order low-pass with `α = sin w`, i.e. **Q = 0.5**.

### 3.4 Per sample: split, oscillator, envelope

**[exact]** (both channels use the same coefficients):
```
w = write index; write index = (w + 2) & 0x1fff     (2 writes per sample)
y  = (((x·b0 + x1·b1) + x2·b0) + y1·a1) + y2·a2      // states hold value + TINY
buf[min(w, 9999)] = (prev_y + y)·0.5;  prev_y = y;  buf[(w+1) & 0x1fff] = y
p = phase + inc;  k = rnd(p);  phase = p − k        // −0.5..0.5
peak = max(|yL|, |yR|);  d = peak − env
env += (d + TINY)·(d > 0 ? att : rel)
tri   = 4·|phase·0.5|                                // 0..1
par   = phase·(|p − k|·(−8) + 4) + 0.5               // parabolic sine 0..1
shape = tri + (par − tri)·0.25
depth = depth_k·((amt·(cont·(env² − 1) + 1))·30)    // ms
lfo   = shape·depth
```
So the modulator is an audio-rate oscillator at `RATE_HZ` (46 Hz at 0 %, ≈ 170 Hz at the
default 25 %, 8.37 kHz at 100 %). Its delay swing is `30·DEPTH(rate)·amt` ms (up to 30 ms
below ≈ 47 % Rate). Contour mixes in the envelope: the factor is `1 − cont_t·(1 − env²)`, so at
Contour 100 % (`cont_t = 0`) the depth is fixed, and at Contour 0 % (`cont_t = 0.9`) it is 10 %
at silence, growing with the low band's level.

### 3.5–3.6 Modulated read (delay line at 2×)

Two 8192-entry lines (L +0x1e0, R +0x9e20) hold the low band at twice the rate. Per channel
**[exact]**:
```
dL = ms2·lfo;           dR = (depth − lfo)·ms2      // anti-phase
avg = (prev_d + d)·0.5; prev_d = d
tap(base, d): n = rnd(d − 0.5); i = base − n; a = buf[i−1], b = buf[i]; a + (b − a)·((n − d) + 1)
v = (tap(w+1, d) + tap(w, avg))·0.5                 // two half-sample reads, averaged
```

### 3.7 Peaking filter and output

**[exact]**:
```
y = pk_n·((g·(v − vx2) − y1·pk_a1) − y2·pk_a2)     // band-pass at hz; states + TINY
out = (x − y_low) + (v + pk_gain·y)
```
`pk_gain·y` turns the band-pass into a peaking cut of `PEAK_DB·amt` dB (0 … −24 dB) at the
oscillator frequency. The part above Split passes untouched; the low band is replaced by its
modulated copy. At Amount 0 the low band only gets a one-sample delay.

---

## 4. Implementation notes

- `Fm::set` does `resume()` on the first call and sends the four events on later calls,
  keeping all state (as Maschine does when a knob is automated).
- The port keeps the kernel's operation order, `f64` smoothers, and no `mul_add`.
- Not verified against Maschine: parameter changes during a render (`maschine-ref` has no
  automation); the event/smoother code follows the assembly.
- The `exp2_tab` table is the Chorus kernel's; fm.rs carries its own copy of the formula.

## 5. Null tests

Source: `~/Documents/Native Instruments/Maschine 2/Projects/20230419-ambient.mxprj`, sound
0/2 slot 2 (FM, stored rate 0.065 / split 0.474 / contour 0.75 / amount 0.75), made with
`maschine-ref make … 0 2 2 [field=…]`, rendered 3 s at −12 dBFS with `--preroll 0`, compared
against `fx-null` on the captured input. Residuals L / R; −240 dB = bit-identical. (Input vs.
Maschine output differs by −30 dB for the project values, so the effect is active.)

| case | input | sr | rate / split / contour / amount | residual |
|---|---|---|---|---|
| project values | noise | 44.1 k | 0.065 / 0.474 / 0.75 / 0.75 | −240 / −240 |
| defaults | noise | 44.1 k | 0.25 / 0.75 / 0.75 / 0.75 | −240 / −240 |
| defaults | noise | 48 k | 0.25 / 0.75 / 0.75 / 0.75 | −240 / −240 |
| project values | sweep | 44.1 k | project | −240 / −240 |
| project values | sine 440 | 48 k | project | −240 / −240 |
| rate min / mid / max | noise | 44.1 k | 0, 0.5, 1 / project | −240 / −240 |
| split min / max | noise | 44.1 k | project / 0, 1 | −240 / −240 |
| split max (clamped) | noise | 48 k | 0.7 / 1 / 0.75 / 0.75 | −240 / −240 |
| contour min / max | noise | 44.1 k | contour 0, 1 | −240 / −240 |
| contour mid | noise | 48 k | contour 0.5, amount 1 | −240 / −240 |
| amount min / mid / max | noise | 44.1 k | amount 0, 0.5, 1 | −240 / −240 |
| odd values | noise | 48 k | 0.123 / 0.877 / 0.333 / 0.618 | −240 / −240 |
| | impulse | 44.1 k | 0.6 / 0.9 / 0 / 1 | −240 / −240 |
| all max | noise | 44.1 k | 1 / 1 / 1 / 1 | −240 / −240 |
| all min | noise | 44.1 k | 0 / 0 / 0 / 0 | −240 / −240 |
| control rate 10 | noise | 96 k | 0.6 / 0.8 / 0.4 / 0.9 | −240 / −240 |
| control rate 2 | noise | 22.05 k | 0.6 / 0.8 / 0.4 / 0.9 | −240 / −240 |
| | sweep | 48 k | 0.9 / 1 / 0.2 / 1 | −240 / −240 |
| `--preroll 1`, input fed as is | noise | 44.1 k | 0.3 / 0.474 / 0.2 / 1 | −35.4 / −35.4 (oscillator phase differs) |
| `--preroll 1`, DECKARD fed `sr` zeros first | noise | 44.1 k / 48 k | 0.3 / 0.474 / 0.2 / 1 | −240 / −240 |

Scratch scripts: `null.sh` (make/render/fx-null/compare) and `pad.py` (pad/trim) in the
session scratch dir `fx-fm/`.
