# Maschine's Metaverb: parameters and DSP

A clean-room description of Maschine 3's built-in **Metaverb** effect
(`NI::MASCHINE::DATA::Metaverb` → `NI::MASCHINE::RT::MetaverbProcessor` →
`NI::MASCHINE::FXCORE::MetaverbCore`). Unlike Chorus, `MetaverbCore` is not a generated
Reaktor-Core kernel. It is hand-vectorised C++: four-lane NEON vectors, fused multiply-adds,
`tanf`, and the shared `NI::AB::ExpTable` and `NI::AB::SmoothingToolbox<15>`. It still has
the Core interface: a flat struct, `beginInit(sr)`/`endInit()`, one `processAudio()` per
sample, and one `processEventIn*()` per parameter.

Sources: Ghidra output in `out/decomp/metaverb/` (`INDEX.tsv`) and arm64 disassembly in
`out/decomp/metaverb-core.s` (kernel, 0x101a1e0a4–0x101a1f250) and
`out/decomp/metaverb-data.s` (DATA and RT classes). `SmoothingToolbox<15>::startSmooth` is
at 0x1019685bc; the same code also serves `<13>`. The spec follows the assembly. Addresses
are arm64 VM addresses in `analysis/maschine_arm64`. The DECKARD port is
`crates/engine/src/fx/metaverb.rs`. It nulls **bit-exactly** against Maschine (§5).

**Confidence.** **[exact]**: read from code or assembly. **[inferred]**: reconstructed.
**[unknown]**: not recovered.

Notation: `v` is a stored parameter value. Arithmetic is `f32`. `fma(a,b,c)` = `a·b + c`
with one rounding (the kernel's `fmadd`/`fmla`); any other product or sum is rounded on its
own. `E(x)` is the ExpTable lookup ≈ 2^((x−1200)/60) (docs/fx-compressor.md §3.1,
`fx::exp_e`). Two forms recur:
- `E(fma(p, 5, 1381.8816))` = 8.1758·2^(p/12) Hz, the frequency of MIDI pitch `p`.
- `E(fma(dB, 9.965784, 1200))` = 10^(dB/20).

---

## 1. Parameters

### 1.1 Module

Module name "Metaverb". One page: `s_aPageNames = { "Main" }`
(`__GLOBAL__sub_I_Modules_Effects_Metaverb_ALL.cpp`, 0x101a50fb4) **[exact]**.

### 1.2 Parameters (constructor 0x101a4e4b0)

Each parameter is made by `TransactionObjectContainer::insert(label, default, min, max)`.
The labels are wide strings at 0x104d30624… **[exact]**:

| obj off | mxprj field | label | section | default | range | RT idx | display converter |
|---|---|---|---|---|---|---|---|
| 0xa8 | `size` | **Size** | "Room" | 0.5 | 0..1 | 0 (`Sz`) | `ScaleNormalizedToPercentOneDecimal` |
| 0xb0 | `low` | **Low** | "Eq" | 0.5 | 0..1 | 1 (`Lo`) | `ScaleNormalizedToPercentOneDecimal` |
| 0xb8 | `high` | **High** | (Eq) | 0.5 | 0..1 | 2 (`Hi`) | `ScaleNormalizedToPercentOneDecimal` |
| 0xc0 | `pan` | **Pan** | "Position" | 0 | −1..1, bipolar | 3 (`Pos`) | `PanningConverter` |
| 0xc8 | `mix` | **Mix** | "Output" | 0.5 | 0..1 | 4 (`Mix`) | `ScaleNormalizedToPercentOneDecimal` |

All five are `MaschineNumericParameter<float>`, with no enums and no switches. Pan also gets
`setBipolar(true)`, the wheel function `processWheelDeltaSnapToZero`, and the value 2.0 at
parameter +0x1d0 (its wheel scale, as on other bipolar knobs) **[exact]**. Pan uses the same
`PanningConverter` as the channel Pan (docs/channel-software.md).

### 1.3 Knob layout (`getParameterByPosition`, 0x101a4ee08)

| pos | page | parameter | section heading |
|---|---|---|---|
| 0 | Main | Size | Room |
| 1 | Main | Low | Eq |
| 2 | Main | High | – |
| 3 | Main | Pan | Position |
| 4 | Main | Mix | Output |
| 5–7 | Main | – (empty) | |

### 1.4 Serialization and engine binding

`load_object_data` loads the Module base, then 0xa8…0xc8 in order. The mxprj fields
(formerly `unknown_a8`…`unknown_c8`) are `size`, `low`, `high`, `pan`, `mix`.
`connectParameters` binds each parameter (`setAndCallFuncAutomate`) to
`processFloatParameter(idx, …, v)` (0x101a4eadc). That function writes `v` unchanged into the
kernel input +0x00/+0x04/+0x08/+0x0c/+0x10 and calls that input's event handler **[exact]**.

---

## 2. Processor (`MetaverbProcessor`)

- **Constructor** (0x101a4efb8): it allocates a zeroed 0x80300-byte kernel and runs
  `beginInit(44100)` + `endInit()` with all inputs at 0 and no event flags. It is stereo in,
  stereo out.
- **`onSamplerateChanged(sr)`** stores `sr`, which takes effect at the next `resume`.
- **`resume()`** (0x101a4f14c) **[exact]**: it saves the five inputs, calls `beginInit(sr)`,
  restores the inputs, sets all five event flags (+0x14…+0x18) and calls `endInit()`. Any
  earlier state is cleared, and the result depends only on the stored values and `sr`.
- **`process()`**: per sample, `in L → +0x1c`, `in R → +0x20`, `processAudio()`, then
  `+0x24 → out L`, `+0x28 → out R`.

The LFO starts at phase 0 on `resume` and free-runs: it is not tied to the transport or the
tempo.

---

## 3. DSP (`FXCORE::MetaverbCore`)

### 3.1 `beginInit(sr)` / `endInit()` **[exact]**

`beginInit` zeroes the smoother, the shelf state, the 8 delay lines and the write index. It
sets the init flag (+0x170), the control counter to 129 with limit 128, `inv_sr = 1/sr`,
LFO phase 0 and LFO increment `inv_sr·1.5151515` (cycles per sample: **1.515 Hz**). It also
sets the Mix and Pan factors (+0x30/+0x34/+0x38) to 0.1. The inputs (+0x00…+0x10) stay.

`endInit`:
```
inv_sr = 1/sr
steps  = max(1, (i32)((sr·0.1)/128 + 0.5))     // 34 at 44.1 k, 38 at 48 k
inv_steps = 1/(f32)(steps·128)
for each flag set, in order: Sz, Lo, Hi, Pos, Mix → its event handler (§3.2–3.4)
cur[0..16] = tgt[0..16];  dirty = 1
```
Pos runs before Mix, so Pos sees the 0.1 Mix factor. Mix then recomputes both dry targets.

### 3.2 Smoothed values

`SmoothingToolbox<15>` at +0x80 holds current values `cur[16]` (+0x90), per-sample steps
`inc[16]` (+0xd0) and targets `tgt[16]` (+0x110). Lane `k`:

| k | name | set by | value |
|---|---|---|---|
| 0, 1 | `len_l`, `len_r` | Sz | base delay in samples |
| 2 | `depth` | Sz | modulation depth |
| 3 | `in_gain` | Sz | input gain into the tank |
| 4 | `diff` | Sz | allpass coefficient (negative) |
| 5, 6 | `hi_w`, `hi_g` | Hi | high shelf `tan(πf/sr)`, gain² − 1 |
| 7 | `lo_w` | Lo | low shelf `tan(πf/sr)` |
| 8 | `decay` | Sz | decay rate (1/RT60, per sample) |
| 9 | `wet` | Mix | wet gain |
| 10, 11 | `dry_l`, `dry_r` | Mix, Pos | dry gains |
| 12, 13 | `send_l`, `send_r` | Sz, Pos | input balance into the tank |
| 14 | `lo_g` | Lo | low-shelf gain² − 1 |

**Event Sz** (0x101a1e3c0) **[exact]**:
```
sz     = v·fma(v, 0.69, 0.31)
decay  = E(fma(sz·−96, 5, 1381.8816))·inv_sr        // 8.1758·2^(−8·sz) Hz = 1/RT60
depth  = fma(1.5 − v, v·2, 0.5)
in_gain= fma(sz, sz·−0.5, 0.9)
open   = 1 − sz;  e = (open + −0.8)·5
pitch  = open > 0.8 ? e·(e·12) + 30 : 30
p_l    = fma(sz, −16, pitch)
p_r    = p_l − fma(v·(2 − v), 2, 1)
len_l  = (E(fma(p_l, −5, 1381.8816))·0.014954345)·sr
len_r  = (E(fma(p_r, −5, 1381.8816))·0.014954345)·sr
shape  = open > 0.8 ? fma(e·−0.4, e, 1) : 1
diff   = −(shape·fma(max(sz, 1e-4), 1/24, 0.5))      // "<" test: sz < 1e-4 → 1e-4
send:  s = (v·0.707)·pan;  send_l = s > 0 ? 1 − s : 1;  send_r = s < 0 ? s + 1 : 1
```
**Event Pos** (0x101a1e9ac) **[exact]**:
```
pos_l = pan < 0 ? 1 : 1 − pan;   pos_r = pan > 0 ? 1 : pan + 1
dry_l = pos_l·mix_dry;  dry_r = pos_r·mix_dry;  send as in Sz
```
**Event Mix** (0x101a1ea24) **[exact]**:
```
mix_dry = m < 0.5 ? 1 : (1 − m) + (1 − m)
wet     = m < 0.5 ? (m + m)·0.9 : 0.9
dry_l = pos_l·mix_dry;  dry_r = pos_r·mix_dry
```
Every handler sets `dirty` (+0x150).

**Smoother, per sample** **[exact]**: first `cur[k] += inc[k]` for all 16 lanes. Then the
control counter is incremented, and when it reaches 128 it resets and:
- If `dirty`: clear it and run `startSmooth`. For each lane, `d = (tgt − cur)·inv_steps`.
  If `(f64)(d·d) < 1e-15`, the lane jumps (`cur = tgt`, `inc = 0`); otherwise `inc = d`. If
  any lane is moving, `count = steps`.
- Otherwise `count -= 1`. When it reaches exactly 0, `cur = tgt` and `inc = 0` for all lanes.

So a change takes effect at the next 128-sample tick and ramps linearly over
`steps·128` samples (≈ 100 ms: 4352 samples at 44.1 k, 4864 at 48 k).

### 3.3 Shelves

**Event Lo** (0x101a1e688) **[exact]**:
```
a    = 1 − v;  db = (a·(2 − a))·−18
G    = E(fma(db·0.5, 9.965784, 1200));  Gi = E(fma(db·−0.5, 9.965784, 1200))
lo_g = fma(G, G, −1)
f    = E(fma(fma(a, 24, 40), 5, 1381.8816))          // MIDI 40 … 64: 82 … 330 Hz
lo_w = tanf((inv_sr·0.5)·((Gi·f)·2π))
```
**Event Hi** (0x101a1e84c) **[exact]**:
```
a    = 1 − v;  db = ((a·(2 − a))·−12)·0.5
G    = E(fma(db, 9.965784, 1200));  hi_g = fma(G, G, −1)
f    = E(fma(fma(a, −34, 120), 5, 1381.8816))        // MIDI 120 … 86: 8.4 k … 1.2 kHz
hi_w = tanf((inv_sr·0.5)·((G·f)·2π))
```
`tanf` is the system libm call (`_tanf` stub), and Rust's `f32::tan` calls the same function
on macOS. So **Low** is a low shelf from 0 dB (100 %) down to −18 dB (0 %), and **High** a
high shelf from 0 to −12 dB. Both cut only: the knobs set the damping of the reverb input.

Coefficients (4 lanes: 0,1 = low shelf L/R, 2,3 = high shelf L/R) are recomputed in
`processAudio`, after the smoother step:
- low lanes, when `inc[7] ≠ 0 || inc[14] ≠ 0 || init`: `X = (w,w,1,1)`,
  `S[0,1] = w + −1`, `N[0,1] = 1/(w + 1)`, `Gc[0,1] = lo_g`, with `w = cur[7]`.
- high lanes, when `inc[5] ≠ 0 || inc[6] ≠ 0 || init`: `S[2,3] = w + −1`,
  `N[2,3] = 1/(w + 1)`, `Gc[2,3] = hi_g`, with `w = cur[5]`. Then `init = 0`.

When the ramp's final jump fires (`count` reaches 0), the coefficients are **not**
recomputed. They keep the values from the last ramp sample, as in Maschine.

### 3.4 Per-sample signal flow (`processAudio`, 0x101a1ea7c) **[exact]**

```
// LFO: one phase, eight offsets
p = phase + lfo_inc;  p = p < 0.5 ? p : p + −1;  phase = p
dep = (cur[2]·sr)·0.0005
lfo(o) = { x = p + o; x = x < 0.5 ? x : x + −1;  dep + (x·(8 − |x|·16))·dep }
         // o: lines 0–3 = 0, 1/8, 1/2, 5/8; lines 4–7 = 1/4, 3/8, 3/4, 7/8

// input and shelves (lanes as in §3.3; fb, out_prev, x1, s are state)
x = ( fb0 + in_gain·(inL·send_l + 1e-27),
      fb1 + in_gain·(inR·send_r + 1e-27),
      out_prev[0], out_prev[1] )                  // high shelf runs one sample behind
s   = N·(X·(x + x1) − S·s) + 1e-27
out = x + Gc·s
x1  = x·(1, 1, −1, −1)                            // low pass on lanes 0,1, high pass on 2,3

// delays (samples)
L   = (len_l, len_r, len_l, len_r)
dA  = lfo(oA) + L·(0.5063944, 0.5063944, 1, 1)                 // lines 0–3
dB  = lfo(oB) + L·(0.8038510, 0.8038510, 1.4983071, 1.4983071) // lines 4–7
tA  = read(lines 0–3, dA);   tB = read(lines 4–7, dB)

// two allpass stages (g = cur[4]; gm = (g, g, 0, 0))
v  = (out[2], out[3], y_prev[0], y_prev[1])
u  = tA − v·g
y  = tB − gm·u
line k   ← (v + u·g) + 1e-27           (k = 0–3, at the new write index)
line 4+k ← (u + gm·y) + 1e-27
y_prev = y

// decay and feedback (crossed: right tank → left input and vice versa)
gain(d) = x = (d·decay)·−60;  x ≤ −144 ? 6.3095733e-8 : E(fma(x, 9.965784, 1200))
fb = (y[3]·gain(dB[3]), y[2]·gain(dB[2]), 0, 0)

outL = u[2]·wet + inL·dry_l
outR = u[3]·wet + inR·dry_r
```
**Lines.** Each of the 8 lines holds 16384 samples plus 3 guard samples (stride 0x10010
bytes, starting at +0x1f0). The write index `w` advances first: `w' = (w + 1) & 0x3fff`.
Each line is written at `w'`, and also at `w' + 16384` when `w' ≤ 2`. Reads use the old
`w`:
```
n = (i32)d (truncation);  t = (truncf(d) − d) + 1
i = (w + !n) & 0x3fff                  // = w − n − 1
a0..a3 = line[i .. i+3]
c1 = (a2 − a0)·0.5;  c3 = c1 + (a3 − a1)·0.5;  d21 = a2 − a1
P  = ((d21·3) − c3) − c1;   Q = c3 − (d21 + d21)
read = (a1 + t·c1) + (t·t)·(P + t·Q)   // Hermite between a1 and a2
```
Topology. Lanes 0,1 of each stage are allpasses in series (lines 0,1, then 4,5). They feed
lanes 2,3 of the first stage (lines 2,3) one sample later. Lanes 2,3 of the second stage
(lines 6,7) are plain delays, whose outputs return, decayed, to the opposite channel's input.
The wet output is `u[2..3]`, taken inside the second loop allpass. A wet impulse appears 2
samples after the input, followed by the diffused tail.

### 3.5 Physical values (derived from §3.2)

- **Size** 0 / 50 / 100 % → RT60 `1/(8.1758·2^(−8·sz))` = 0.12 s / 0.75 s / 31 s. Base
  lengths `len` = 0.12226·2^(−p/12) s: 10.8 / 11.5 ms at 0 %, 54.5 / 64.8 ms at 100 %. The
  longest line (×1.498) is 97 ms, longer than 16384 samples at 192 kHz, so it wraps there,
  in Maschine as well.
- Modulation: ±`(0.5 + (1.5 − v)·2v)·0.25` ms about a centre of the same size, at 1.515 Hz.
- **Mix**: wet `0.9·min(2m, 1)`, dry `min(1, 2(1 − m))`. 0 % is dry only (bit-identical),
  and 100 % is wet only.
- **Pan** moves the dry signal (`pan > 0` lowers the left dry, `pan < 0` the right). It also
  moves the tank input by `|pan|·Size·0.707`.

---

## 4. Implementation notes

- `Metaverb::set` runs `resume()` on the first call. On later calls it writes each
  **changed** input and runs its event, as `processFloatParameter` does, so the smoother
  ramps and the tank keeps its state.
- The port uses scalars for the 4-lane vectors, with the same operation order. All LFO
  lanes share one phase (the kernel keeps 4 identical copies). `mul_add` is used exactly
  where the kernel has `fmadd`/`fmla`.
- Not verified against Maschine: parameter changes during a render, because `maschine-ref`
  has no automation. The smoother follows the assembly, and unit tests cover re-sending
  values and the ramp.
- Not used: the side-chain, tempo, the transport, and meters (the processor has none).
- **Shared code:** only `NI::AB::ExpTable` (`fx::exp_e`) and the system `tanf`.
  `SmoothingToolbox<15>::startSmooth` (shared with `<13>`) is implemented inline here. No
  other reverb kernel was found to share `MetaverbCore`.

## 5. Null tests

Source project: `~/Documents/Native Instruments/Maschine 2/Projects/4-6 show.mxprj`, sound
3/1 slot 2 (Metaverb, stored size 0.19 / low 0.5 / high 0.5 / pan 0 / mix 0.2), made with
`maschine-ref make … 3 1 2 [field=…]`. Rendered with `--preroll 0` at −12 dBFS. The
comparison is against `fx-null` on the captured input (scratch script `nt.sh`). **−240 dB
means bit-identical** (peak difference 0).

| case | input | sr | s | size / low / high / pan / mix | residual L / R |
|---|---|---|---|---|---|
| project values | noise | 44.1 k | 3 | 0.19 / 0.5 / 0.5 / 0 / 0.2 | −240 / −240 |
| project values | sweep | 48 k | 3 | project | −240 / −240 |
| defaults | noise | 44.1 k | 3 | 0.5 / 0.5 / 0.5 / 0 / 0.5 | −240 / −240 |
| defaults | noise | 48 k | 3 | 0.5 / 0.5 / 0.5 / 0 / 0.5 | −240 / −240 |
| flat shelves | noise | 44.1 k | 1 | 0.19 / 1 / 1 / 0 / 0.2 | −240 / −240 |
| longest tail | impulse | 44.1 k | 12 | 1 / 0.5 / 0.5 / 0 / 1 | −240 / −240 |
| longest tail | impulse | 48 k | 12 | 1 / 1 / 1 / 0 / 1 | −240 / −240 |
| shortest | impulse | 44.1 k | 3 | 0 / 0.5 / 0.5 / 0 / 1 | −240 / −240 |
| darkest | impulse | 48 k | 8 | 0.5 / 0 / 0 / 0 / 1 | −240 / −240 |
| size max / min | noise | 44.1 k | 4 / 3 | 1 resp. 0, rest project | −240 / −240 |
| low 0 / low 1 | noise | 44.1 k | 3 | project, low = 0 resp. 1 | −240 / −240 |
| high 0 / high 1 | noise | 44.1 k | 3 | project, high = 0 resp. 1 | −240 / −240 |
| pan −1 | noise | 44.1 k | 3 | 0.8 / 0.5 / 0.5 / −1 / 0.2 | −240 / −240 |
| pan +1 | noise | 48 k | 3 | 0.8 / 0.5 / 0.5 / 1 / 0.2 | −240 / −240 |
| pan 0.37 | noise | 44.1 k | 3 | 0.6 / 0.5 / 0.5 / 0.37 / 0.2 | −240 / −240 |
| mix 0 / 0.5 / 1 | noise | 44.1 k | 3 | project, mix varied | −240 / −240 |
| mix 0.75 | noise | 48 k | 3 | project, mix 0.75 | −240 / −240 |
| odd values | sine 440 | 48 k | 4 | 0.333 / 0.777 / 0.123 / −0.42 / 0.66 | −240 / −240 |
| | noise | 96 k | 3 | 0.7 / 0.5 / 0.5 / 0 / 0.8 | −240 / −240 |
| | noise | 22.05 k | 3 | 0.7 / 0.5 / 0.5 / 0 / 0.8 | −240 / −240 |
| | impulse | 96 k | 8 | 1 / 0.5 / 0.5 / 0 / 1 | −240 / −240 |
| `--preroll 1`, DECKARD fed 44100 zeros first | noise | 44.1 k | 3 | defaults | −240 / −240 |

The impulse renders carry real tails (−84 dB RMS over 12 s at Size 100 %), and they null
over their whole length. Without the zero padding, the `--preroll 1` render differs by
−23.5 dB, because the LFO phase is different.
