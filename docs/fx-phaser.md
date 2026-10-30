# Maschine's Phaser: parameters and DSP

A clean-room description of Maschine 3's built-in **Phaser** effect
(`NI::MASCHINE::DATA::Phaser` → `NI::MASCHINE::RT::PhaserProcessor` →
`NI::MASCHINE::FXCORE::PhaserCore`, with `NI::MASCOMMON::LEGACY::SyncableLFOHelper` for the
LFO rate). `PhaserCore` is a compiled Reaktor-Core kernel: a flat struct of inputs, outputs
and state with `endInit()` and `processAudio()` (one stereo sample per call). It has no
`processEventIn*` functions: `PhaserProcessor::process*Parameter` writes the inputs and the
values derived from them directly.

It was written from Ghidra output in `out/decomp/phaser/` (`INDEX.tsv` maps files to
functions) and arm64 disassembly in `out/decomp/phaser-asm/` (`core.s` = `processAudio`,
`endinit.s`, `params.s` = the three `process*Parameter`, `proc.s` = `resume`/`process`,
`ctor.s`, `global.s`). Where the two disagree (Ghidra re-associates some products, e.g.
the loop gain, and drops a `max(·, 0)`), this spec follows the assembly. Addresses are
arm64 VM addresses in `analysis/maschine_arm64`. The DECKARD port is
`crates/engine/src/fx/phaser.rs`; it nulls **bit-exactly** against Maschine (§6).

**Confidence.** **[exact]**: read from code or assembly and confirmed by the null tests.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation: `v` is a parameter's stored value. All arithmetic is `f32` unless marked `f64`;
the kernel has **no fused multiply-adds**. `D = 1e-18` (`0x219392ef`) is the kernel's
denormal offset. `round(x)` is `(int)((double)x ± 0.5)` with the sign by `x < 0` (half away
from zero).

---

## 1. Parameters

### 1.1 Stored fields

The constructor is `Phaser::Phaser()` at 0x101a55a64 (`0011.c`). Each parameter is made by
`TransactionObjectContainer::insert(label, default[, min, max])`; the serializer
(0x101a5588c) loads them in offset order **[exact]**.

| obj off | module.rs field | label | type | default | range | RT index | display | section |
|---|---|---|---|---|---|---|---|---|
| 0xa8 | `frequency` | **Frequency** | float | 0.5 | 0..1 | float 0 | percent, one decimal | "Main" |
| 0xb0 | `amount` | **Amount** | float, bipolar | 0.5 | −1..1 | float 1 | percent (−100..100 %); wheel snaps to 0 | "Mod" |
| 0xb8 | `source` | **Source** | Enum | 1 | `LFO`, `LFO Sync`, `Envelope` | enum 0 | – | not automatable, forces a display update |
| 0xc0 | `speed` | **Speed** | float | 0.5 | 0..1 | float 2 | `SyncableLFOHelper::convertValueToString` (§1.3) | |
| 0xc8 | `shape` | **Shape** | float | 0.5 | 0..1 | float 3 | percent | |
| 0xd0 | `stereo` | **Stereo** | float | 0.5 | 0..1 | float 4 | percent | |
| 0xd8 | `feedback` | **Feedback** | float | 0.5 | 0..1 | float 5 | percent | |
| 0xe0 | `eight_pole` | **8Pole** | bool | off | | bool 0 | on/off | |
| 0xe8 | `mix` | **Mix** | float | 0.5 | 0..1 | float 6 | percent | "Output" |

Percent is `ScaleNormalizedToPercentOneDecimal` (`v·100`, one decimal, "%"). Module info:
name "Phaser" (`s_aModuleInfo` at 0x10dce66b0). Page names: **"Main"** only (one entry,
`__GLOBAL__sub_I_Modules_Effects_Phaser_ALL.cpp`). The Source strings are built there too
(`aSourceValueList` at 0x10e3061a0) **[exact]**. The label "8Pole" is stored without a
space.

### 1.2 Page layout (`getParameterByPosition`, 0x101a56a0c) **[exact]**

| pos | parameter | section heading |
|---|---|---|
| 0 | Frequency | "Main" |
| 1 | Feedback | |
| 2 | 8Pole | |
| 3 | Amount | "Mod" |
| 4 | Source | |
| 5 | Speed (Source LFO, LFO Sync) / **Shape** (Source Envelope) | |
| 6 | Stereo | |
| 7 | Mix | "Output" |

The section names belong to the parameters (Frequency "Main", Amount "Mod", Mix
"Output"), so they appear at slots 0, 3 and 7.

### 1.3 Display converters

- **Speed** (`SyncableLFOHelper::convertValueToString`, as in docs/fx-filter.md §1.3, over
  this effect's 11 rates, §2): Source LFO Sync prints the division `"%d/%d"` of
  `i = clamp(round(10·v), 0, 10)` in `16/1, 8/1, 4/1, 2/1, 1/1, 1/2, 3/8, 1/4, 3/16, 1/8,
  1/16` (bars). Source LFO prints the free rate (§2) through `StringUnits::hzToString`
  (`"%.2f Hz"` below 10 Hz, …).
- All other floats are percent with one decimal (Amount shows −100..100 %).
- The values the DSP derives (§4) are never shown: Frequency 0..100 % is the allpass
  centre pitch 37..124 (≈ 69 Hz .. 10.55 kHz).

---

## 2. LFO rate (`SyncableLFOHelper`)

The processor builds the helper with `s_arrRateDivisions` (0x10dc35ba4), 11 entries in
bars: `16, 8, 4, 2, 1, 1/2, 3/8, 1/4, 3/16, 1/8, 1/16`, and `setPhaseOffs(0.5)`. The
helper code is the one documented in docs/fx-filter.md §4 with `n = 11` **[exact]**:

- Free (Source LFO): `pos = 10·v`, `i = clamp(round(pos − 0.5), 0, 9)`, `f = pos − i`,
  `s = fma(1 − f, r[i], f·r[i+1])`, `Hz = 1/(s + s)`: 1/32 Hz at 0, 8 Hz at 1.
- Synced (Source LFO Sync): `i = clamp(round(10·v), 0, 10)`, `beats = 4·r[i]`,
  `Hz = tempo·(1/60)/beats`.
- Synced with the transport running, `newFrame` re-phases the LFO every block from the
  song position plus 0.5 cycles. The position is the host block's start rounded to
  240000 ticks per quarter, the same for every 128-sample block of the host block
  (docs/fx-filter.md §4) **[exact, by test]**.
- Source Envelope switches the helper off; its last rate still reaches the core, unused.

---

## 3. Processor (`PhaserProcessor`)

**Parameter routing** (`processFloatParameter` 0x101a56404, `processEnumParameter`
0x101a5681c, `processBoolParameter` 0x101a568bc) **[exact]**:

| param | core effect |
|---|---|
| Frequency (0) | `pitch = v·87 + 37`; target `freq_t = pitch` (f64); the span (below) is recomputed with the current `amt87`; amount target `= span[idx]`; flags \|= 3 |
| Amount (1) | `sq = v ≥ 0 ? v·v : −(v·v)`; `amt87 = sq·87`; `f = amt87 + pitch`; `span = (max(f, 37) − pitch, min(f, 124) − pitch)` (as `f < 37 ? 37 : f`, `f > 124 ? 124 : f`); `idx = sq > 0`; amount target `= span[idx]`; flags \|= 2 |
| Speed (2) | `SyncableLFOHelper::setSpeed(v)` |
| Shape (3) | `x = (v + v) − 1`, `y = 1 − |x|`; `x > 0.01`: hold = 11.538501 ms, release = `(y·16000 + 1000)·0.115385` ms; else hold = `(y·2000 + 1000)·0.115385` ms, release = 57.6925 ms. `hold_ticks = round(hold·ctl_ms)`, `r = release·ctl_ms`, `decay = r > 1 ? 1/r : 1` |
| Stereo (4) | `st_gain = v·(−0.05) + 1`; `st_off = (v·v)·0.5` (cycles) |
| Feedback (5) | `fbc = (2 − v)·v`; `lpf_t = (fbc·mix)·(−12400) + 22000`; `fbv = fbc·0.916 + 0.05`; level `L` from §4.3; `wet_t = mixc·L + 0`; `dry_k = (fbv − 1)·L − 1`; `dry_t = mixc·dry_k + 1`; `fb_t = fbv`; flags \|= 0x3c |
| Mix (6) | `lpf_t = (fbc·v)·(−12400) + 22000`; `mixc = (2 − v)·v`; `wet_t`, `dry_t` as above; flags \|= 0x38 |
| Source (enum 0) | value 0 → helper free, 1 → synced (after `setSpeed`), 2 → off; core `env = (value == 2)` |
| 8Pole (bool 0) | `eight = round(round(b))` |

Targets are stored as `f64` of the `f32` expression. `ctl_ms` = control rate/1000 (§4).

**`process()`** (0x101a58550), per block of Maschine's engine (128 samples, every one of
a host block's with the host block's song position; docs/fx-filter.md §3–4) **[exact]**:
1. `newFrame(tempo, playing, position)` gives the LFO rate `hz`. The core gets
   `slew = ((hz·32)·1.044e-4)·(44100/ctl)` (`0x38daf153`, both LFO smoothers) and
   `inc = hz·(1/ctl)`. If the helper re-phased (synced and playing), the LFO phase is set
   to the helper's.
2. Smoothing stop: for each smoother (frequency bit 0, amount 1, feedback 2, dry 3, wet 4,
   low-pass 5) the bit clears when `|(f64)(f32)current − target| ≤ 1e-4`. While **any**
   bit is set all six smooth (§4.1); when none is, they hold.
3. Per sample: inputs L/R → `processAudio` → outputs replace L/R. No latency.

**`resume()`** (0x101a58484) saves the eight parameter inputs, zeroes the whole core
(0x3b8 bytes; this also clears the LFO rate input and the phase input), restores the
inputs and their event flags and runs `endInit` (§5). The module pushes all parameters
before the first `resume`, so a fresh effect depends only on its parameters.

---

## 4. DSP (`PhaserCore::processAudio`, 0x101a58914)

Constants from `endInit` **[exact]**: `steps = round(sr/11025)` (f32 division), `ctl =
round(sr)/steps` (integer; 11025 at 44.1 kHz, 12000 at 48 kHz, with `steps` = 4), `a = (1 − q)/(q + 1)` with
`q = sr/88200` (1/3 at 44.1 kHz), `aa = 1 − a·a`, `lo = sr/24576`, `hi = sr/2.125`,
`w_k = 6.28319/sr`, `π_sr = 3.14159/sr`, `lpf_max = sr·0.494`.

### 4.1 Control tick

A counter (0 after init) increments while below `steps − 1`; otherwise it resets and the
tick runs before the sample (first tick on the `steps`-th sample after `resume`). In order:

1. **Smoothing** (if any flag bit is set; `f64`): `cur += coef·((target − cur) + 1e-100)`
   for low-pass, wet, dry, feedback, amount, frequency, each then taken as `f32`.
   `coef = 1 − 2^x`, `x = (f32)(((0.310295·0.9602356 + 0.311434·0.039764404) − 61) /
   max(ctl/1000·100 − 1, 1))`, the power from the kernel's 1022-entry table of
   `2^(i/1021 − 0.5)` (the same table and lookup as docs/fx-filter.md §5.2): 2^−60.7 in
   100 ms (1/e in ≈ 2.4 ms).
   Then the output low-pass: `f = max(lpf, 0)`, `f = f > lpf_max ? lpf_max : f`,
   `w = f·π_sr`, `g = sin(w)/cos(w)` by the polynomials of §4.6, `g_inv = 1/(g + 1)`,
   `g_m1 = g − 1`.
2. **Modulation** `m1` (left), `m2` (right), 0..1:
   - LFO (Source not Envelope), phase `ph` in [−0.5, 0.5]:
     `t = 4·|ph·0.5|` (as `|ph·0.5|` doubled twice) — a triangle;
     `lfo1 += slew·((t − lfo1) + D)`; the same for `ph2 = (ph + st_off) − round(ph +
     st_off)` into `lfo2`; then `ph = (ph + inc) − round(ph + inc)`. `m1 = lfo1`,
     `m2 = lfo2`.
   - Envelope: `lvl = max(|L|, |R|)` (the current input samples). If `lvl > peak`:
     `peak = lvl`, `count = 0`; else `count += 1` and, if `hold_ticks ≤` the old count,
     `peak = max(peak − decay, 0)`. `e = env + env_c·(min(peak + 1e-6, 1) − env)`,
     stored `+ D`; `m1 = e`, `m2 = st_gain·e`. `env_c = c < 1 ? ln2 : ln2/c` with
     `c = ctl·0.002` (2 ms). The peak starts at 0.1.
3. **Frequencies**: `x = (freq + m·amt)·0.0833292`; `n = round(x)`, `f = x − n`;
   `Hz = (f·(f·0.248 + 0.708) + 1)·2^n·8.17742`; `Hz = Hz > 14000 ? 14000 : Hz`; then
   `Hz > hi ? hi : (lo ≤ Hz ? Hz : lo)`; `w = Hz·w_k`.
4. **Allpass coefficients** per channel from `(s, c) = sin/cos(w)` (§4.6):
   `inv = 1/(s + 1)`, `b0 = (1 − s)·inv`, `c1 = (c·−2)·inv`, `c2 = (s + 1)·inv`,
   `c3 = −c1`, `c4 = −b0`, `b0z = b0·0`. Loop denominator
   `den = 1 − fb·(a·(sel·(b0·(b0·b0))))`, `sel = 8Pole ? b0 : 1`.

### 4.2 Allpass chain and feedback loop, per sample and channel **[exact]**

Nodes `n0..n4` (the clipped loop signal and the four section outputs, each stored `+ D`)
and their previous values `d0..d4` (`d = n + D`, shifted at the start of every sample;
`od` = the values before this shift):
```
s_k = n_k·c1 + od_k·c2 + n_{k+1}·c3 + od_{k+1}·c4          k = 0..3, left to right
t3  = s2 + b0·(s1 + b0·(b0z + s0))
fbo = 8Pole ? s3 + b0·t3 : t3
u   = (x + fb·(z·aa + a·fbo)) / den                        (zero-delay loop, solved)
t   = hp_inv·(u − hp·hp_m1);  v = u + (−0.32239223)·(hp_g·(hp + t));  hp = t + D
c   = v > 6.666 ? 6.666 : (−6.666 ≤ v ? v : −6.666)
x0  = c·(1 − |c|·0.0750075)
y1 = s0 + b0·x0;  y2 = s1 + b0·y1;  y3 = s2 + b0·y2;  y4 = s3 + b0·y3
n = (x0, y1, y2, y3, y4) + D
z = ((8Pole ? y4 : y3) − z·a) + D                          (first-order allpass)
```
So 8Pole off uses three second-order sections (the fourth is computed and unused), on uses
four. The feedback path is `fb·A(z)·chain`, with `A` the fixed first-order allpass
`(a + z⁻¹)/(1 + a·z⁻¹)`; the high-pass is a TPT one-pole at `min(sr, 92.32608 Hz)`
(`g = w + w³·0.333333 + w²·w³·0.133333`, `w = π_sr·92.32608`, `hp_inv = 1/(g + 1)`,
`hp_m1 = g − 1`) whose high-passed part is added back at −0.322.

### 4.3 Levels

`fbv = fbc·0.916 + 0.05` (smoothed as `fb`). `L` interpolates the table at 0x10dc33b6c
`(16, 6, 3.3, 2.2, 1.66, 1.22, 1, 0.75, 0.71, 0.54, 0.43, 0.43)` at `fbv·10`
(`i = round(fbv·10 − 0.5)`, `i0 = (u32)i < 11 ? i : 11`, `i1 = (u32)(i+1) ≥ 11 ? 11 : i+1`,
`L = T[i0] + f·(T[i1] − T[i0])`). `wet = mixc·L`, `dry = mixc·((fbv − 1)·L − 1) + 1`,
`mixc = (2 − Mix)·Mix`. Mix 0 → dry 1, wet 0 (the output is still low-passed, §4.4).

### 4.4 Output (per channel) **[exact]**

```
v1  = g_inv·((x·dry + u·wet) − lp1·g_m1);   lp1' = v1 + D
v2  = g_inv·(g·(lp1 + v1) − lp2·g_m1);      lp2' = v2 + D
out = g·(lp2 + v2)
```
Two TPT one-pole low-passes at `22000 − 12400·fbc·Mix` Hz (capped at 0.494·sr). The wet
signal is the loop node `u`, so the notches come from the feedback structure.

### 4.5 Channels

Left uses `m1` (no offset), right `m2` (LFO phase + Stereo²/2 cycles, or envelope
× `1 − 0.05·Stereo`). The channels share nothing else; the envelope follower listens to
both inputs.

### 4.6 sin/cos polynomials **[exact]**

`w2 = w·w`; `sin = w·(w2·(w2·(w2·(w2·(w2·−2.39e-08 + 2.7526e-06) − 0.000198409) +
0.00833333) − 0.166667) + 1)`; `cos = w2·(w2·(w2·(w2·(w2·−2.605e-07 + 2.47609e-05) −
0.00138884) + 0.0416666) − 0.499923) + 1` (each step `w2·(prev) + k`).

---

## 5. Initialization (`endInit`, 0x101a56ccc) **[exact]**

`endInit` derives every constant and coefficient from the inputs as the setters do,
starts the six smoothers at their targets, sets the LFO smoothers and phase to 0 (the
rate input is 0 after `resume`), the peak to 0.1 and the counter to `steps`, and then runs
one full control tick and one sample on silence (all states become `D`-sized). The counter
is then 0. The kernel's init pass orders some sums differently from `processAudio`, but on
zero state the results are identical, so the port runs its own tick.

---

## 6. Null tests

Source: `/Users/Shared/Chromatic Fire Library/Projects/Exosphere.mxprj`, sound 0/1, slot 2
(a Phaser: Amount 1, LFO Sync, Speed 0.17, Stereo 0, Feedback 0.92). `maschine-ref make …
0 1 2 field=value…`, `render --preroll 0 --seconds 2 --tempo T`, then `fx-null`
(`FX_TEMPO=T`). Scripts: `null.sh` / `matrix.sh` in the scratch dir
`/private/tmp/claude-501/-Applications-Native-Instruments-Maschine-3/d9bc38fd-75cf-45e9-a132-1940350f781c/scratchpad/fx-phaser/`.
"def" = all defaults (`0.5, 0.5, LFO Sync, 0.5, 0.5, 0.5, 0.5, off, 0.5`). Residual L/R
RMS; −240 dB = bit-identical.

| test | settings (on top of def) | input | sr | residual |
|---|---|---|---|---|
| project | stored values | noise | 44.1k | −240 / −240 |
| def / def48 | | noise | 44.1k / 48k | −240 |
| free, free0, free1 | LFO, Speed 0.8 / 0 / 1 | noise | 44.1k / 44.1k / 48k | −240 |
| sync0, sync1 | LFO Sync, Speed 0 / 1 | noise | 44.1k | −240 |
| t90, t137 | Speed 0.8 at tempo 90; Speed 0.3 at 137.5 | noise | 44.1k / 48k | −240 |
| env | Envelope | noise | 44.1k | −240 |
| env0 / env1 | Envelope, Shape 0, Amount 1 / Shape 1, Amount −1 | sweep / impulse | 44.1k | −240 |
| env03 / env07 | Envelope, Shape 0.3, Stereo 1 / Shape 0.7, Stereo 0, Feedback 1 | sine 440 / noise | 48k | −240 |
| p8 / p8fb | 8Pole on / + Feedback 1, Amount −1 | noise | 44.1k / 48k | −240 |
| freq0 / freq1 | Frequency 0 / 1 (Amount 1) | noise | 44.1k | −240 |
| amtm1 / amt0 | Amount −1 / 0 | noise | 44.1k | −240 |
| fb0 / fb1 | Feedback 0 / 1 | noise | 44.1k | −240 |
| mix0 / mix1 | Mix 0 / 1 | noise | 44.1k | −240 |
| st0 / st1 | Stereo 0 / 1 | noise | 44.1k | −240 |
| odd | 0.123, −0.377, LFO, 0.61, 0.2, 0.83, 0.66, on, 0.71 | sweep | 48k | −240 |
| r96 / r88 / r22 | LFO 0.9 / 8Pole, Feedback 0.9 / def (ticks every 9, 8, 2 samples) | noise / sweep / noise | 96k / 88.2k / 22.05k | −240 |
| loud / loud30 | Feedback 1, Mix 1, 8Pole, `--level 6` / Envelope, `--level 30` (clipper active) | noise | 44.1k / 48k | −240 |
| playfree / playenv | `--play`: LFO Speed 0.8 / Envelope | noise | 44.1k | −240 |
| playsync | `--play`, LFO Sync, Speed 0.8 (`fx-null` with `FX_PLAY=1`) | noise | 44.1k | −240 |
| playsync tempi | the same at `--tempo 90` | noise | 44.1k | −240 |
| playsync fast | LFO Sync, Speed 1, Feedback 0.8, 8Pole, `--tempo 137.5` | noise | 44.1k | −240 |
| playsync 48k | LFO Sync, Speed 0.6, Amount −0.9, Mix 1, tempo 120 | noise | 48k | −240 |
| playsync slow | LFO Sync, Speed 0.3, Frequency 0.7, tempo 137.5 | noise | 44.1k | −240 |
| playsync 96k | LFO Sync, Speed 0.9, tempo 90 | noise | 96k | −240 |
| sync stopped | LFO Sync, Speed 0.8, tempo 137.5, no `--play` | noise | 44.1k | −240 |

Not tested against Maschine: parameter changes mid-stream (maschine-ref renders a fixed
project), so the smoothing and the per-block stop (§3) are from the code only.

## 7. Open points

1. *(Resolved.)* Synced LFO with the transport running: Maschine runs the processor in
   128-sample blocks, all with the host block's start position rounded to 240000 ticks per
   quarter (docs/fx-filter.md §4); the port does the same (`process` splits into 128
   samples, `fx::song_position`) and nulls bit-exactly with `--play` (§6).
2. The QML editor (`Modules/FX/PhaserEditor.qml`, `PhaserModel`) was not decoded; the
   page layout above is `getParameterByPosition`.
