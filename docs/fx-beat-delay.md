# Maschine's Beat Delay effect: DSP specification

A clean-room description of Maschine 3's built-in **Beat Delay** insert effect
(`NI::MASCHINE::DATA::BeatDelay` → `NI::MASCHINE::RT::BeatDelayProcessor` →
`NI::MASCHINE::FXCORE::StudioDelayCore`). Written from decompiled code and disassembly of
the arm64 slice (`analysis/maschine_arm64`) and constants read from the binary. Addresses
are arm64 VM addresses. Port: `crates/engine/src/fx/beat_delay.rs` (bit-exact, §5).

Sources:

| dir | contents |
|---|---|
| `out/decomp/beat-delay/` | Ghidra C (`INDEX.tsv`): `DATA::BeatDelay` ctor, serialize, `connectParameters`, `getParameterByPosition`, static init; `BeatDelayProcessor` ctor/process/resume/parameter dispatch; all of `StudioDelayCore` |
| `out/decomp/beat-delay-asm/` | `objdump` listings (`INDEX.tsv`) used for the exact operation order: `processAudio.s`, `processEventIn.s`, `setFB_setColor.s`, `endInit.s`, `processor.s`, `connectParameters.s` |

The Ghidra project was locked by concurrent runs, so these were decompiled from a private
copy of `analysis/ghidra` (same database).

**Confidence.** **[exact]**: read directly from code or disassembly and confirmed by the
bit-exact null tests. **[inferred]**: a reasonable reading. **[unknown]**: not recovered.

Notation: `sr` is the sample rate (f32). All DSP is f32 unless marked f64. `fma(a,b,c)` =
a·b + c with one rounding (`fmadd`/`fmla`); `fnms(a,b,c)` = a·b − c with one rounding
(`fnmsub`). `E(x)` is the `NI::AB::ExpTable` lookup ≈ 2^((x−1200)/60), exactly as in
docs/fx-eq.md §2.1.

---

## 1. Parameters

### 1.1 Module

- Module name `"Beat Delay"`, module-info id **26** (0x1a) (`s_moduleInfo`, set in
  `__GLOBAL__sub_I_Modules_Effects_BeatDelay_ALL.cpp` 0x1019ff7a8). **[exact]**
- Page names (`s_pageNames`): `"Main"`, `"Unit"`. **[exact]**
- Processor `"BeatDelay"`: stereo in / stereo out (`setStereoInStereoOut`), processed in
  place; latency reported 0. **[exact]**

### 1.2 Parameters in constructor order

`BeatDelay::BeatDelay()` (0x1019fc380) **[exact]**:

| obj off | field (mxprj) | label | type | default | range | RT id | display | section |
|---|---|---|---|---|---|---|---|---|
| 0xa8 | `time` | **Time** | Enum | **3** ("3/16") | 17 values, list A | enum 0 | the list string | `Delay` |
| 0xb0 | `offset` | **Offset** | Enum | **0** | 9 values "0" … "8" | enum 1 | the list string | – |
| 0xb8 | `feedback` | **Feedback** | float | **0.5** | 0 … 1 | float 0 | `ScaleNormalizedToPercentOneDecimal` (v·100, "%.1f %") | – |
| 0xc0 | `crossover` | **Crossover** | float | **0.25** | 0 … 1 | float 1 | percent, one decimal | – |
| 0xc8 | `color` | **Color** | float | **0.5** | 0 … 1 | float 2 | percent, one decimal | – |
| 0xd0 | `split` | **Split** | float | **0.25** | 0 … 1 | float 3 | percent, one decimal | – |
| 0xd8 | `stereo` | **Stereo** | float, **bipolar** | **0** | −1 … 1 | float 4 | `FloatLinearConverter(0, 100, "%.1f %%")`: −100.0 % … 100.0 % | `Output` |
| 0xe0 | `mix` | **Mix** | float | **0.5** | 0 … 1 | float 5 | percent, one decimal | – |
| 0xe8 | `unit` | **Unit** | Enum, **not automatable** | **0** ("1/16") | list C | enum 2 | the list string | `Unit` |

- List A (Time): `"1/32"`, `"1/16"`, `"2/16"`, `"3/16"`, … `"16/16"` (index 0 = 1/32,
  index n ≥ 1 = n/16). List B (Offset): `"0"` … `"8"`. List C (Unit): `"1/16"`,
  `"1/16T"`, `"1/32"`. **[exact]**
- Stereo's encoder uses `processWheelDeltaSnapToZero` (with 2.0 at `+0x1d0`). **[exact]**
- Serialization (`serialize<iarchive>` 0x1019fc1a8): Module base, then the nine fields in
  the table's order (`unknown_a8` … `unknown_tail` in mxprj, now named as above). **[exact]**

### 1.3 Knob layout (`getParameterByPosition`, 0x1019fd1e4)

| page | pos | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
|---|---|---|---|---|---|---|---|---|---|
| Main | 0–7 | TIME | OFFSET | FEEDBACK | CROSSOVER | COLOR | SPLIT | STEREO | MIX |
| Unit | 8 | UNIT | – | – | – | – | – | – | – |

No mode-dependent knobs. **[exact]**

### 1.4 Routing (`connectParameters` 0x1019fc98c, `BeatDelayProcessor` 0x1019fcdf0/0x1019fce38)

Each parameter is bound with `setAndCallFuncAutomate` (applied at once, too). Enum values
arrive as `(float)index`. Each writes a core input and calls the core's event **[exact]**:

| param | core input | event |
|---|---|---|
| Time | `+0x10` | `processEventInT` |
| Offset | `+0x1c` | `processEventInTp` |
| Unit | `+0x14` | `processEventInUnit` |
| Feedback | `+0x20` | `processEventInFB` |
| Crossover | `+0x24` | `processEventInXFB` |
| Color | `+0x2c` | `processEventInColr` |
| Split | `+0x28` | `processEventInSplit` |
| Stereo | `+0x30` | `processEventInSt` |
| Mix | `+0x00` | `processEventInMix` |

`BeatDelayProcessor::process` (0x1019fd628), per block, when input pin 0 has the expected
channel count: core `+0x0c` ← **ProcessInfo tempo** (`+0xd8`), `processEventInBPM()`;
`+0x04` ← playing, `+0x08` ← PPQ·24 (both unused by the DSP); then `processAudio()` per
sample. **[exact]** The tempo is the host's (`ProcessContext::tempo`) whether or not the
transport plays (null-tested at 100 and 137.5 BPM, stopped and playing). A core input
`+0x18` ("Tsh", a pre-delay) exists but nothing sets it (0). **[exact]**

---

## 2. DSP

### 2.1 Structure

Two delay lines `A` (`+0x290`) and `B` (`+0x200290`) of 2^19 floats each (mask 0x7ffff),
one write pointer `wp` and four read pointers: A-new, A-old, B-new, B-old. **[exact]**

### 2.2 Events

**Delays** (`processEventInT/Unit/Tp/BPM`, identical code) **[exact]**:

```
beat  = 15000 / bpm                              // a 1/16 note in ms (BPM event)
base  = UnitArray[unit] · ((beat·sr)·0.001)      // UnitArray = {1, 2/3 (0x3f2aaaab), 0.5}
k, d  = time < 1 ? (1, base·0.5) : (time, base)  // "1/32" = half a unit
fl    = floor(d)
tsh   = floor(sr·((tsh·tsh)·0.002))              // 0
dA    = fma(fl, k, tsh)
dB    = fnms(k + offset, fl, tsh)                // Offset adds whole units to line B
if inc[XF] != 0: return                          // a crossfade is running: nothing else
A-new = (wp − (int)dA) & mask;  B-new = (wp − (int)dB) & mask
tgt[XF] = 1; dirty; setFBParams()
```

So a Time/Offset/Unit/tempo change during a running crossfade only stores the new lengths;
they take effect at the next BPM event after it ends.

**setFBParams** (0x101a22130) **[exact]**:

```
a   = fma(feedback, 53, 35); if a == 35: a = −100           // dB
ms  = fma(E(fma(a, 3/log10 2 (0x411f73da), 602.0529)), 1000, 0.0001)   // 10^(a/20) ms: T60
per = (1/ms) · (−60000/sr)                                   // dB per sample
gA  = powf(10^(1/20) (0x3f8f9db2), max(dA·per, −120))        // gain per pass, libm powf
gB  = same with dB
then the Crossover matrix (below)
```

**Crossover** (`processEventInXFB`, end of setFBParams) with `x` = Crossover **[exact]**:

```
y  = max(fma(x, 2, −1), 0)
u0 = clamp(fma(x, −2, 1), 0, 1);  u1 = clamp(x + x, 0, 1)    // fmaxnm/fminnm
tgt FB = [gB·u0, fma(1−y, u1·gA, y), u0·gA, u1·gA]
tgt SEND[0] = 1 − y
```

**Feedback** (`processEventInFB`): Feedback → setFBParams; `corr = feedback < 0.5 ? 1 :
fma(h·h, −0.2, 1)` with `h = (feedback − 0.5) + same`; then the wet gains. **[exact]**

**Mix / Stereo** (`processEventInMix`, `…InSt`) with `st = (stereo + 1)·0.5` **[exact]**:

```
tgt DRY   = min((1−mix) + (1−mix), 1)
mm        = mix·(2 − mix)
tgt WET   = ((st·(2 − st))·mm)·corr          // own side
tgt WET_X = (((st + 1)·(1 − st))·mm)·corr    // other side
```

**Split** (`processEventInSplit`, partly f64) **[exact]**:

```
w = (f32)((f64)E(fma(fma(s, 53, 50), 5, 1381.8816)) · 6.28319 (0x401921fc8f32378b) / (f64)sr)
q = (f32)(fma((f64)s, −1.024, 1.28) · (f64)s · 0.8)
d = (f32)(2·(1 − ((f64)q + 0.4)))
m = min(s·10, 1)
tgt SPLIT_A = [m, 1−m, m+m, 1−m];  tgt SPLIT_W = [w, 0, w, 0];  tgt SPLIT_D = [d, 0, d, 0]
```

The SVF frequency is the MIDI pitch 50 + 53·s (≈ 147 Hz … 2.9 kHz).

**Color** (`setColorParams` 0x101a222f8) with `c` = Color **[exact]**:

```
hp = c < 0.5  ? 0 : (min(c, 0.53) + −0.5) / 0.029999971   // 0 → 1 over 0.50 … 0.53
lp = c < 0.47 ? 1 : (0.5 − min(c, 0.5)) / 0.030000001     // 1 → 0 over 0.47 … 0.50
tgt COL_MIX = [1 − (hp + lp), 0, hp, lp]
t  = |fma(c, 2, −1)|
f1 = E(fma(fma(t, 54, 36), 5, 1381.8816));  f2 = E(fma(fma(t, −65, 125), 5, 1381.8816))
g1 = tanf((f1·π)/sr);  g2 = tanf((f2·π)/sr)              // libm tanf
tgt COL_E = [1/(g1+1), 1/(g2+1), g1 + −1, g2 + −1];  tgt COL_F = [1, g2, 1, g2]
```

(`<` here is the original's `K > v` comparison, the same for non-NaN.) Lanes 0/2 (weight
`hp`) are a one-pole high-pass with corner `f1`, lanes 1/3 (weight `lp`) a two-pole
low-pass with corner `f2` (§2.4; DC gain 0 and 1). So Color 0.53 → 1 thins the echoes
(high-pass corner rising from pitch ≈ 39 to 90, ≈ 78 Hz → 1.5 kHz), Color 0.47 → 0 darkens
them (low-pass corner falling from pitch ≈ 121 to 60, ≈ 8.9 kHz → 262 Hz), 0.47 … 0.53
crossfades to the plain tap (exactly 0.5 = plain). **[exact]** for the formulas; the
corner/response reading is **[inferred]** from them.

### 2.3 Smoother (36 lanes)

Targets at `+0x1d0`, values at `+0xb0`, increments at `+0x140`, nine groups of four:
`FB`, `[XF, DRY, WET, WET_X]`, `COL_MIX`, `COL_E`, `COL_F`, `SEND` (`[1−y, 1, 1, 1]`),
`SPLIT_A`, `SPLIT_W`, `SPLIT_D`. Every event sets `dirty`. **[exact]**

- Ramp length: `steps = (int)((sr·0.02)/128 + 0.5)`, at least 1 (7 at 44.1 kHz, 8 at
  48 kHz); `inv_len = 1/(steps·128)`. **[exact]**
- Control tick (`processAudio`, first thing, when the counter reaches 128; `beginInit`
  sets the counter to 129 so the first sample ticks) **[exact]**:
  - if `dirty`: clear it; per lane `inc = (tgt − cur)·inv_len`; if `(f64)(inc²) < 1e-15`
    then `inc = 0, cur = tgt`. If any lane kept an increment, `remaining = steps`.
  - else `remaining −= 1`; at 0: `cur = tgt`, all `inc = 0`.
  - then, in every case, if `inc[XF] == 0`: `inc[XF] = tgt[XF] = cur[XF] = 0`, A-old ←
    A-new, B-old ← B-new (the crossfade is over, or never started).
- Then (every sample) `cur += inc` for all 36 lanes. **[exact]**

### 2.4 Per sample (`processAudio` 0x101a22518; asm `processAudio.s`) **[exact]**

```
x  = [inL, inL, inR, inR]
for lane i:                                   // SVF; lanes 1 and 3 have w = d = 0: hp = x
  hp[i] = x[i] − (s2[i] + s1[i]·SPLIT_D[i])
  bp[i] = s1[i] + SPLIT_W[i]·hp[i]
  lp[i] = s2[i] + SPLIT_W[i]·bp[i]
t     = SPLIT_A · [hp[2] + lp[0], hp[3], bp[0], hp[1]]
toB   = SEND[0]·(t0 + t1)                      // R high + L low (+ R dry): into line B
toA   = SEND[1]·(t2 + t3)                      // L band (+ L dry): into line A
s1 = bp; s2 = lp
p     = FB · [fbB, fbA, fbA, fbB]              // fbA, fbB = last frame's color outputs
B[wp] = (p0 + p1) + toB
A[wp] = (p2 + p3) + toA
tapA  = fma(A[A-new], XF, (1 − XF)·A[A-old])
tapB  = fma(B[B-new], XF, (1 − XF)·B[B-old])
```

Color, four lanes (A high-pass, A low-pass, B high-pass, B low-pass) on the **previous** taps `cx`, with
`ev = [E0,E1,E0,E1]`, `e2 = [E2,E3,E2,E3]` (`COL_E`), `F` = `COL_F`, `O = [0,1,0,1]`,
`I = [1,0,1,0]`; the masks are applied literally (the multiplications by 0 and 1 are in
the code):

```
u    = ev·(cx − cs·e2)
y    = F·[u0 − cs0, u1 + cs1, u2 − cs2, u3 + cs3]
v2   = (ev·O + I)·(cw − (e2·O)·cv)
z    = F·(cv·O + v2)
out  = z·O + y·I                                   // [hpA, lpA, hpB, lpB]
cs = u; cv = v2; cx = [tapA, tapA, tapB, tapB]; cw = y
oA = (M0·tapA + M1·tapA) + (M2·out0 + M3·out1)     // M = COL_MIX (M1 = 0)
oB = (M0·tapB + M1·out1) + (M2·out2 + M3·out3)
fbA, fbB = oA, oB
L = fma(oB, WET_X, fma(inL, DRY, oA·WET))
R = fma(oA, WET_X, fma(inR, DRY, oB·WET))
wp, all read pointers += 1 (& mask)
```

So the filtered parts of an echo come one sample after its dry part.

### 2.5 Tempo and block size

`processEventInBPM` runs at the start of each block Maschine processes, and it re-arms a
crossfade (between identical taps when nothing changed) whenever none is running. The
crossfade changes the output by rounding only, but to null bit for bit the timing has to
match: Maschine's own blocks are ≤ 128 samples (renders null with BPM events every 16,
32, 64 or 128 samples, not with 256/512/1024 at 44.1 kHz) **[exact, by test]**. With
blocks that divide 128, the only event that matters is the one right before each control
tick, so the port runs `processEventInBPM` (with the latest `set_tempo`) right before
every tick, independent of the host block size.

### 2.6 Initialization (`BeatDelayProcessor` ctor 0x1019fd3e4, `resume` 0x1019fd5a8)

The ctor allocates the core (0x400380 bytes), `beginInit(44100)`, `endInit()`, and sets
defaults (mix 0.5, bpm 120, time 3, …); connecting the parameters then applies the stored
values. `resume()` keeps the core inputs `+0x00 … +0x33` (stored values and the current
tempo), calls `beginInit(sr)` (zero everything incl. the lines, counters 128/129, unit
lanes of `COL_F`/`SEND` = 1) and then `endInit`, which runs the events in the order Mix,
BPM, T, Unit, Tsh, Tp (Offset), FB, XFB, Split, Colr, St, and finally copies all targets
to the values (no ramp) with `dirty` set. **[exact]** Before any audio the core's tempo
is still 120, so at another tempo the first BPM event ramps the feedback gains (§2.3)
from the 120-BPM values over the first ~20 ms (inaudible: the first echo comes later).
The port does the same.

---

## 3. Port notes

- `set()` before the first `process()` re-runs `resume` (as Maschine resumes before
  audio); afterwards each changed value runs its event, so state is kept.
- Subnormals: Maschine's tails below ~1e-37 differ slightly in the subnormal range (an
  impulse with 0.95 feedback: residual peak 2.2e-37, −240 dB RMS). Maschine's output
  contains subnormals, so this is not a plain flush-to-zero; probably its FPCR flushes
  subnormal *inputs* only (FEAT_AFP `FIZ`) **[inferred]**. Not emulated.
- Parameter changes during playback were not null-tested (`maschine-ref` has no
  automation); the event code is the same that the init path runs.

## 4. Display (for the UI)

- TIME / OFFSET / UNIT show their list strings (§1.2).
- FEEDBACK, CROSSOVER, COLOR, SPLIT, MIX: `v·100` as `"%.1f %"`.
- STEREO: `100·v` (−100 … 100) as `"%.1f %"`, bipolar.

## 5. Null tests

`maschine-ref make "Orange Ten.mxprj" … 0 6 1` with the defaults plus the listed changes,
`render --input <in> --preroll 0 --sr <sr> --tempo <bpm>`, then `FX_TEMPO=<bpm> fx-null`
(512-sample blocks) and `compare`. "exact" = residual −240 dB, peak 0 (bit-identical).

| sr | BPM | input | settings (else defaults) | result |
|---|---|---|---|---|
| 48k | 120 | noise 3 s | defaults (time 3/16, fb 0.5, xo 0.25, color 0.5, split 0.25, stereo 0, mix 0.5) | exact |
| 48k | 100, 137.5 | noise 3 s | defaults, transport stopped and `--play` | exact (DECKARD at 120 instead: −18 dB) |
| 48k | 120 | noise 3 s | time 0, 1, 8, 16; offset 4, 8; unit 1, 2; time 0 + unit 2 + offset 3 | exact |
| 48k | 120 | noise 3 s | feedback 0, 0.25, 0.75, 1 | exact |
| 48k | 120 | noise 3 s | crossover 0, 0.5, 0.75, 1 | exact |
| 48k | 120 | noise 3 s | color 0, 0.25, 0.47, 0.49, 0.51, 0.53, 0.75, 1 | exact |
| 48k | 120 | noise 3 s | split 0, 0.05, 0.5, 1 | exact |
| 48k | 120 | noise 3 s | stereo −1, −0.5, 0.5, 1; mix 0, 0.25, 1 | exact |
| 44.1k | 120 | noise 3 s | defaults; time 0, 16; offset 8; unit 1, 2; feedback 0, 1; crossover 0, 1; color 0, 0.49, 0.51, 1; split 0, 1; stereo −1, 1; mix 0, 1 | exact |
| 44.1k | 120 | noise 3 s | fb 0.9, xo 0.6, color 0.2, split 0.7, stereo 0.3, mix 0.8 | exact |
| 44.1k | 120 | noise 3 s | Orange Ten's own (time 0, fb 0.779, color 0.999, split 0.77, stereo 1, mix 0) | exact |
| 44.1k | 93 | sweep 4 s | time 5, offset 2, fb 0.8, color 0.8 | exact |
| 48k | 31 | noise 14 s | time 16, offset 8, fb 0.6 (12.1 s > 2^19 samples: the line wraps) | exact |
| 48k | 140 | noise 12 s | time 1, unit 2, fb 1, xo 1, color 0, split 1, mix 1 | exact |
| 48k | 174 | impulse 3 s | time 2, fb 0.95, xo 0.9, color 0.1 | −240 dB RMS, peak 2.2e-37 (subnormal tail, §3) |

Before the BPM-event timing of §2.5 was modelled (event once per 512-sample block), 44.1 kHz
renders left −150 … −170 dB RMS (peak −126 dB); 48 kHz was exact either way (8·128 =
2·512).
