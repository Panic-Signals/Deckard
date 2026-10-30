# Maschine's Distortion: parameters and DSP

A clean-room description of Maschine 3's built-in **Distortion** effect. The data class is
`NI::MASCHINE::DATA::MullhollandDrive` (module name "Distortion", mxprj module
`MullhollandDrive`, module-info id 0x1e). Its processor `NI::MASCHINE::RT::MullhollandDriveProcessor`
holds two DSP cores side by side and runs the one chosen by **Mode**:

| Mode | name | core |
|---|---|---|
| 0 | **Mulholland** | `NI::MASCHINE::FXCORE::MullhollandDrive` (hand-vectorised C++, 4 NEON lanes) |
| 1 | **Analog** | `NI::DSP::AnalogDistortion` + two `NI::AB::SampleRateConverter`s (×4 / ×2 oversampling) |

DECKARD's port is `crates/engine/src/fx/distortion.rs`. Both modes null **bit-exactly** against
Maschine at 44.1, 48 and 96 kHz (§6).

Sources (arm64 VM addresses in `analysis/maschine_arm64`):

- `out/decomp/distortion/` — Ghidra C of the data class, processor, both cores and the static
  initializer (`INDEX.tsv` maps files to functions; names in `out/decomp/distortion-names.txt`).
  The shared Ghidra project was locked by another agent; a read-only copy in the scratch
  directory was used.
- `out/decomp/distortion-asm/` — objdump listings: `core-setters.s` and `core-process.s`
  (Mulholland), `analog.s` (AnalogDistortion), `src-up.s` / `src-down.s`
  (`SampleRateConverter::process_intratUP/DOWN`), `data.s`, `init.s`.
- `SampleRateConverter::init/init_intrat` from `out/decomp/saturator-tape/` (docs/fx-saturator.md).

Ghidra drops NEON lanes, `frinti` and the fused operations in these functions; where it and the
assembly disagree this spec follows the assembly.

**Confidence.** **[exact]**: read from code or assembly (and confirmed by the bit-exact null
tests). **[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation: `f32` arithmetic unless noted. `fma(a,b,c)` = `a·b + c` with one rounding; every other
product or sum is rounded on its own. `E(x)` is the shared ExpTable lookup (docs/fx-eq.md §2.1,
`fx::exp_e`), `E(1200 + 9.965784·dB) = 10^(dB/20)`.

---

## 1. Parameters

### 1.1 Stored fields **[exact]**

`MullhollandDrive::MullhollandDrive()` (0x101a514bc) inserts each parameter with
`TransactionObjectContainer::insert(label, default, min, max)`. Labels are wide strings at
0x104d30f30.. . "RT idx" is the `tFloatParameters` / `tBoolParameters` index bound with
`setAndCallFuncAutomate`; values reach the cores **raw** (no normalisation).

| off | mxprj field | label | type | default | range | section | RT | core call | display |
|---|---|---|---|---|---|---|---|---|---|
| 0xa8 | `mode` | Mode | enum | **1** (Analog) | Mulholland, Analog | "Mode" | `setMode` | selects core | option string; not automatable; forces display update |
| 0xb0 | `drive` | Drive | float | 0.25 | 0..1 | "Distortion" | F0 | `setDrive` | percent |
| 0xb8 | `color` | Color | float | 0.25 | 0..1 | | F1 | `setColor` | percent |
| 0xc0 | `feedback` | Feedback | float, **bipolar** | 0.25 | −1..1 | | F2 | `setFB` | percent (−100..100); wheel snaps to 0, +0x1d0 = 2 |
| 0xc8 | `tone` | Tone | float | 0.75 | 0..1 | | F3 | `setToneFB` | percent |
| 0xd0 | `tone_mod` | Tone Mod | float, **bipolar** | 0 | −1..1 | | F4 | `setMove` | percent; wheel snaps to 0, +0x1d0 = 2 |
| 0xd8 | `gate` | Gate | bool | **on** | off/on | "Gate" | B0 | `setGate` | on/off |
| 0xe0 | `release` | Release | float | 0.5 | 0..1 | | F5 | `setUnscaledRelease` | `ReleaseDisplayScaler`: `"%.1f ms"` of `1000·scaleReleaseValue(v)` |
| 0xe8 | `mix` | Mix | float | 0.5 | 0..1 | "Output" | F6 | `setMix` | percent (`FloatFormatPercent` is replaced by `ScaleNormalizedToPercentOneDecimal`) |
| 0xf0 | `analog_saturation` | Saturation | float | 0.25 | 0..1 | "Distortion" | F7 | AD param 0 | percent |
| 0xf8 | `analog_bass` | Bass | float | 1.0 | 0..1 | | F8 | AD param 1 | percent |
| 0x100 | `analog_tone` | Tone | float | 1.0 | 0..1 | | F9 | AD param 3 | percent |
| 0x108 | `analog_focus` | Focus | bool | off | off/on | | B1 | AD param 4 (0.0/1.0) | on/off |
| 0x110 | `analog_definition` | Definition | float | 0.5 | 0..1 | | F10 | AD param 5 | percent |
| 0x118 | `analog_mix` | Mix | float | 1.0 | 0..1 | "Output" | F11 | AD param 7 | percent; wheel snaps to 0 and +0x1d0 = 2 (as the bipolar ones, but not `setBipolar`) |

"percent" = `ScaleNormalizedToPercentOneDecimal` (`"%.1f %"` of 100·v). Mode option strings
(`s_aModeValueList`, from the static initializer 0x101a55570): **"Mulholland"** (0) and
**"Analog"** (1) — note the single *l*, unlike the class name.

`scaleReleaseValue(v)` (0x101a178e0) = `fma(fma(v⁴ − v², 0.8096, v²), 4.99, 0.01)` seconds:
v = 0 → 10.0 ms, 0.5 → ≈ 500 ms, 1 → 5000 ms.

The processor's constructor also sets two Analog parameters that have no field: AD param 2 = 1.0
and AD param 6 = 0.9 **[exact]**.

### 1.2 Serialization and old projects **[exact]**

`serialize<iarchive>` (0x101a5117c): Module base, then, **if class version ≠ 0**: mode, 0xf0,
0xf8, 0x100, 0x108, 0x110, 0x118; then (all versions) 0xb0, 0xb8, 0xc0, 0xc8, 0xd0, 0xd8, 0xe0,
0xe8. A **version-0** archive has only the Mulholland parameters, and the loader sets Mode to
**0 (Mulholland)**. So projects from before the Analog mode existed load as Mulholland, while a
newly created Distortion defaults to Analog. This is the field order in
`crates/mxprj/src/types/module.rs` (fields renamed from `unknown_*`). DECKARD's `Distortion::set`
treats a missing `mode` as 0.

There is no other distortion class in the Maschine 3 binary (no `DATA::Distortion`; the only
other `*Distortion*` symbol is `NI::DSP::AnalogDistortion`, used here). The "Distortion" of
older Maschine versions is this class: its version-0 form is Mulholland only.

### 1.3 Pages **[exact]**

`s_aPageNames`: **"Main"**, **"Gate"**. `getParameterByPosition` (0x101a526c4) depends on Mode:

| pos | page | Mulholland (0) | Analog (1) |
|---|---|---|---|
| 0 | Main | Mode | Mode |
| 1 | Main | Drive | Saturation |
| 2 | Main | Color | Bass |
| 3 | Main | Feedback | Tone |
| 4 | Main | Tone | Focus |
| 5 | Main | Tone Mod | Definition |
| 6 | Main | – | – |
| 7 | Main | Mix | Mix |
| 8 | Gate | Gate | – |
| 9 | Gate | Release | – |
| 10–15 | Gate | – | – |

Section headings come from the parameters (§1.1): pos 0 "Mode", pos 1 "Distortion", pos 7
"Output"; on the Gate page pos 8 "Gate".

---

## 2. Processor (`MullhollandDriveProcessor`) **[exact]**

- **Constructor** (0x101a52924): `FXCORE::MullhollandDrive` (0x600 bytes) +
  `AnalogDistortion` (0x400 bytes). It calls AD `onInit`, AD `onSetSampleRate(44100, 128)`, MD
  `setSampleRate(44100)`, stereo in/out pins, AD `setParameter(2, 1.0)` and `(6, 0.9)`.
- `onSamplerateChanged(sr)`: AD `onSetSampleRate(sr, 128)` always; MD `setSampleRate(sr)` when
  `sr` differs from the processor's stored rate. In practice it always runs once before audio
  (the null tests at 44.1 kHz only match with it), so every Mulholland glide lands on its target
  at the first control tick.
- `resume` → AD `onResume`; `suspend` → AD `onSuspend`. MD has neither.
- `setMode(m)` stores m. `process`:
  - mode 0: MD `processAudio(L, R, n)` in place;
  - mode 1: chunks of at most 128 samples. A counter (+0xb8, starts at 0) calls AD
    `processControlRate` and reloads 128 whenever it is 0; AD `processAudio` gets
    `min(remaining, counter)` samples;
  - any other mode: nothing (pass-through).
- Both cores keep their state while the other mode runs. There is no latency report.

---

## 3. Mulholland (`FXCORE::MullhollandDrive`)

Layout (object offsets): parameters +0x00..+0x1c (drive, color, fb, tone, mix, tone mod,
release, gate), derived values +0x20..+0x30, filter states +0x40..+0x2cf, biquad coefficients
+0x2d0..+0x30f, delay line +0x310..+0x34f, gate +0x350..+0x360, glides +0x380..+0x54f, rates
+0x580..+0x5a0. The constructor (0x101a170cc) starts drive 0, color 0.5, fb 0, tone 0.5, mix
0.5, tone mod 0, release 0.5, gate 1, all glide progress 1.

### 3.1 Helpers **[exact]**

- `pow2(x)`: `n = frinti(x)` (nearest, ties to even); `i = n` if −2³¹ < n < 2³¹ else INT_MIN;
  `f = x − i`; `pow2 = fma(fma(f, 0.248, 0.708), f, 1) · bits(0x3f800000 + (i << 23))`
  (constants 0x3e7df3b6, 0x3f353f7d).
- `decay(n) = expf(m < 1 ? −ln10 : −ln10/m)`, `m = n − 1` (−2.302585 = 0xc0135d8d): falls 20 dB in
  n samples.
- `round_away(x)`: `(int)(x ± 0.5)` in `f64`.

### 3.2 Glides and setters **[exact]**

Twenty values glide (value +0x380, start +0x400, delta +0x480, progress +0x500). Every setter
**retargets**: `start = fma(delta, progress, start)`, `delta = target − start`, `progress = 0`,
`value = start`. At each control tick (§3.5) `progress = min(step + progress, 1)` and
`value = start + delta·progress` (separate roundings). `step = 1/((sr/ctl_n)·0.05)`: 50 ms.

| idx | target |
|---|---|
| 0 | input gain `fma(g³, 99960, 40)`, `g = fma(d, 0.66, 0.34)·d·0.6` (Drive) |
| 1 | output gain `pow2(0.1660964·fma(|fbs|, −6, fma(u(2−u), 9, fma(0.6d, −12, −16))))`, `u = 1.5·col` |
| 2 | Color low-pass `fma(m, −5244, 9000)` Hz, `m = min(2·col, 1)` |
| 3 | crossfade `col·(2 − col)` |
| 4 | Mix |
| 5 | feedback amount `0.3·fbs` |
| 6 | feedback delay in samples (set every tick, §3.5) |
| 8, 9 | `1/(t0+1)`, `1/(t1+1)` |
| 10, 11 | `t0 − 1`, `t1 − 1` |
| 12, 13 | `t0`, `t1` |
| 14, 15 | `e0² − 1`, `e1² − 1` |
| 16..19 | clipper knee `e0, e0, e1, e1` |

Setters:
- `setDrive(d)`: idx 0 and 1.
- `setColor(v)`: `x = 2v − 1`; `col = fma(fma(|x|, 0.3, 0.2), x, 0.5)` (+0x20); `m = min(2col,1)`;
  `db = fma(m, −28, −12)`; `e0 = pow2(−0.0830482·db)` = 10^(−db/40), `e1 = pow2(0.0830482·db)`;
  `f0 = min(0.494·sr, 1200/e0)`, `f1 = min(0.494·sr, 1200/e1)`;
  `t0 = tanf((1/sr)·(f0·π))`, `t1` likewise. Retargets 16..19, 3, 1, 2, 8..15.
- `setFB(v)`: `fbs = s·|s|`, `s = fma(|v|, 0.5, 0.5)·v` (+0x24); idx 5 and 1.
- `setToneFB(v)`: delay `fma(v², 0.02899, 1e-5)` seconds (+0x28), up to 29 ms.
- `setMix(v)`: idx 4.
- `setMove(v)` (Tone Mod): `s = |v|·v`, `t = −0.1·s`, `a = t·|s|` (+0x2c), `b = a > 0 ? 0 : −a`
  (+0x30). The level follower moves the delay by `a·env + b` seconds (±0.1 s).
- `setUnscaledRelease(v)`: `rel = decay(scaleReleaseValue(v)·sr)` (+0x354).
- `setGate(b)`: +0x1c = b.

### 3.3 Per sample (`MullhollandDriveImpl::processAudio`, 0x101a1f254) **[exact]**

Lanes are `[L, R, L, R]`.

1. **Level follower**: `peak = max(|L|, |R|)`; `env = fma(peak − env, (peak > 0 ? kₑ : 0) + 1, env)`,
   `kₑ = 0.693147/(0.177988·sr) − 1` (so +1 gives a ≈ 5.6 s⁻¹ rise; any zero sample drops it to 0).
2. Control tick when the countdown (+0x594) was < 2 (§3.5).
3. **Feedback SVF** (all lanes; lanes 2/3 get no feedback): `in = x + fb·val5`;
   `s1' = s1 + f·(in − (s1·d + s2))`, `s2' = s2 + f·s1'`. `f`, `d` from 11-point tables at
   `370/(sr/2)·10` (§3.4). Taken on: `[s1'L, s1'R, s2'L, s2'R]` (band-pass and low-pass).
4. **Gain and two one-pole high-passes** (lanes 0/1 at 370 Hz, 2/3 at 25 Hz; `c = 1/(t+1)`,
   `τ = t − 1`): `y1 = c·(x·val0 − τ·z1)`, `y2 = c·((y1 − z1) − τ·z2)`, out `y2 − z2(old)`.
5. **Pre-emphasis shelf**: `y = c·(h − τ·z)`, `out = h + (e² − 1)·(t·(z + y))` with lanes 0/1
   using idx 8/10/12/14 (e0) and lanes 2/3 idx 9/11/13/15 (e1).
6. **×4 interpolation + clip**: the last four shelf outputs `a, b, c, x` (oldest first) give
   points at −1.75, −1.5, −1.25, −1 samples:
   `((b·0.867188 − a·0.0703125) + c·0.226563) − x·0.0234375`,
   `((b·0.5625 − a·0.0625) + c·0.5625) − x·0.0625`,
   `((b·0.226563 − a·0.0234375) + c·0.867188) − x·0.0703125`, and `c`.
   Each is clipped `y = r·(k·p)`, `r ≈ 1/(k + |p|)` from `FRECPE` + two `FRECPS` steps, `k` =
   idx 16..19 (≥ 1: higher Color, harder clip).
7. **Decimation** (4-entry rings per phase): `o = (((P1[n−1] + q1) + q2) + q3)·0.25`,
   `q1 = ((P2[n−1]·0.867188 − P2[n]·0.0703125) + P2[n−2]·0.226563) − P2[n−3]·0.0234375`,
   `q2` the same with 0.5625/0.0625 on P3, `q3` with 0.226563/0.0234375/0.867188/0.0703125 on P4.
8. **De-emphasis shelf**: as 5 with the lanes swapped (lanes 0/1 use e1, 2/3 use e0).
9. **Two biquads in series** (DF-I, history toggled by +0x244): lanes 0/1 low-pass at 8150 Hz then
   10863.9 Hz; lanes 2/3 at `f` then 1.333·`f` (f = idx 2). `y = x1·b1 + b0·(x + x2) − a1·y1 − a2·y2`.
10. **Color crossfade**: `wet = B + (A − B)·val3` (A = lanes 0/1, B = lanes 2/3).
11. **Feedback delay**: write `wet` (stereo frame) to the ring (`len` = 2·frames, §3.4), then read
    `di` and `di + 1` frames back and interpolate `a + (b − a)·frac` into the feedback input
    (+0x40). Reads into the not-yet-written part of a fresh ring are masked to 0.
12. **Gate**: if `peak > 0.01` (−40 dB) the hold counter is reloaded with `hold =
    round(0.04·sr)` (40 ms). While the counter (decremented each sample) is ≥ 1:
    `g = 1 − (1 − g)·att` (fused; `att = decay(0.04·sr)`); else `g = fma(g, rel, 1e-18)`.
13. **Mix**: `m = val4·(gate ? g : 1)`; dry gain `(1 − m)(2 − (1 − m))`, wet gain
    `val1·m(2 − m)`; `out = x·dry + wet·wetgain`.

### 3.4 `setSampleRate(sr)` (0x101a17928) **[exact]**

`isr = 1/sr`, `w2π = isr·2π`; max delay `maxd = fma(sr, 0.4535147, 1)` (20001 at 44.1 kHz);
ring `frames = (round_away(maxd + 1) + 0x21) & ~0x1f`. SVF tables (`s_freqTable`,
`s_dampTable`, 0x104d2980c / 0x104d29838) interpolated at `x = 370/(sr/2)·10`
(`fma(T[i], 1 − f, f·T[i+1])`). High-pass `t = tanf((0.494sr < 370 ? 0.494sr·π : 1162.389)/sr)`
and the same with 25 Hz (78.54). Follower `kₑ`; control period `ctl_n = max(1,
round_away(sr/9800))` (5 at 44.1/48 kHz); `step`. Then the Drive glides, `setColor`, the FB
glide, Tone, Mix/FB glides (progress not cleared), Tone Mod, attack/release, hold; then all
signal state is zeroed and every glide's progress set to 1.

### 3.5 Control tick **[exact]**

Every `ctl_n` samples: delay target `max(0, (fma(a, env, b) + tone_s)·sr)` retargets idx 6; all
glides advance; `di + frac` = idx 6 clamped to 0..maxd; and the four low-pass biquads are
redesigned at `[8150, f, 10863.9, 1.333f]` Hz: `ω = min(hz·w2π, 2.9)`,
`sin`, `cos` by the polynomials of docs/fx-saturator.md §6.1 but **unfused** (mul then add),
`α = 0.7·sin`, `r ≈ 1/(1 + α)` (FRECPE + 2 FRECPS), `b1 = r(1 − cos)`, `b0 = r((1 − cos)·0.5)`,
`a1 = r(−2cos)`, `a2 = r(1 − α)`.

---

## 4. Analog (`NI::DSP::AnalogDistortion`)

### 4.1 Rates **[exact]**

`onSetSampleRate(sr, 128)`: `F = 192000` if `ror32((int)sr·0xb7a32847 + 0x576180, 7) < 0x15d87`
(sr a multiple of 48 kHz-ish rates), else 176400: 44.1 → 176.4 kHz (×4), 48 → 192 kHz (×4),
88.2/96 → ×2. `k = (int)(F·128/sr)`; filter glide `ticks = max(1, (int)(0.02F/k + 0.5))`,
`inv = 1/(ticks·k)` (≈ 20 ms); mix glide `ticks = max(1, (int)(0.02sr/128 + 0.5))`, `inv =
1/(ticks·128)`. If F ≠ sr, converters up (sr→F) and down (F→sr) are (re)initialised and reset
(§5). Then `setFilters`, the Focus coefficients, and a filter-glide restart.

### 4.2 Parameters (`setParameter`, 0x1026144ec) **[exact]**

`w = 2π/F`; `C(x) = min(E(x)·w, 1)` (one-pole coefficient at pitch x).

| idx | field | targets |
|---|---|---|
| 0 | Saturation v | sat gain `E(fma(fma(v, 40, 20), 9.965784, 1200))` (+20..+60 dB); and smoothing (as 2) |
| 1 | Bass v | bass `C(fma(fma(1 − v, 110, 4), 5, 1381.8816))` |
| 2 | (fixed 1.0) | smoothing `C(fma(fma(p0, −40, fma(p2, 40, 130)), 5, 1381.8816))` |
| 3 | Tone v | tone `C(fma(fma(v², 45, 67), 5, 1381.8816))` |
| 4 | Focus v | `u = 1 − v`: attack/release `ln2/(fs·t)` with `t = fma(u, 0.029, 0.001)` / `fma(u, 0.029, 0.005)` (or ln 2 when fs·t ≤ 1); amount `u·1.2·0.5/(u + 1.2 − 1)` |
| 5 | Definition v | definition gain `E(fma(fma(v, 40, −20), …))` (−20..+20 dB), compensation `E(fma(−6v, 9.965784, 1200))` |
| 6 | (fixed 0.9) | trim `0.447·v` |
| 7 | Mix v | mix glide target `[v, 1 − v]` |

Focus **on** sets amount 0 (u = 0): the envelope bias is off. Focus off (the default) leaves it on.

`setFilters` also sets the DC blocker `(f32)(2π/F · E(1331.8815918))` in `f64` (≈ 4.6 Hz).

### 4.3 Glides **[exact]**

Twelve filter values (+0x280, targets +0x2e0, increments +0x2b0) and the two mix values
(+0x340/+0x360/+0x350) follow the Tape ramp logic (docs/fx-saturator.md §5.1): a set target
marks the glide dirty; the next control tick computes `inc = (tgt − cur)·inv` per lane (lanes with
`inc² < 1e-15` snap), counts `ticks` ticks, then snaps. `cur += inc` every processed sample (the
filter glide at F, the mix glide at sr). While suspended (from construction until `onResume`),
every set snaps the filter glide; the mix glide is never snapped, so the wet/dry mix fades in from
0 over its first glide.

### 4.4 Per sample at F (`processAudio`, 0x102614c98) **[exact]**

Per channel, left then right (the focus envelope `e` is shared and updated by both):
```
d    = x − bass;  bass = fma(d, kBass, bass)
a    = |x − bass_old| − e;  e = fma(a, a > 0 ? kAtt : kRel, e)
d    = fma(e, focus, d)
t    = fma(d·sat, 8, 128);  n = (int)t,  f = t − n
y    = (f32) fma(T[n1] − T[n0], f, T[n0])      (f64; n0 = clamp(n, 0, 255), n1 = min(max(n, −1), 254) + 1)
sm   = fma(y − sm, kSmooth, sm)
lp   = fma(sm − lp, kTone, lp)
hp   = lp − dc;  dc = fma(hp, kDC, dc)
dl   = fma(x − dl, kTone, dl)                  (the dry signal, tone-filtered)
out  = fma(dl, definition, hp) · comp · trim
```
`T` is `s_fWaveShaperData` (0x10dc44578): 256 `f64`, odd-symmetric around entry 128 (0), ±0.666
at the ends — a soft saturation curve over x ∈ [−16, 16). Then, at sr:
`out = fma(wet, mix, dry·(1 − mix))` with the glided mix values.

`onReset` (never called by the processor) would clear the states; `onResume`/`onSuspend` snap the
filter glide.

---

## 5. Sample-rate converters (`NI::AB::SampleRateConverter`, quality 2) **[exact]**

Integer ratio R (4, or 2 at 88.2/96 kHz). Phase coefficients (`init_intrat`, `f64`, as in
docs/fx-saturator.md §5.2) `h_t[i] = sinc(t + i − 8)·I0(…)/427.564…`, i = 0..15. UP uses
t = 1/R, 2/R, …; DOWN uses t = 1 − 1/R, 1 − 2/R, … . Each phase has its own transposed 16-tap
FIR: `out = acc[0] + x·h[0]`, `acc[i] = acc[i+1] + x·h[i+1]`, `acc[15] = 0` (separate
roundings).

- **UP**: per input sample: the input from 8 samples ago, then the R − 1 phases of the current
  input.
- **DOWN**: inputs cycle through phases; the first input of the stream (and every R-th after) is
  pushed into an 8-deep delay and emits `y = (d + sum)·(1/R)`, `d` = that phase's sample 8 groups
  earlier; the other R − 1 inputs feed the FIR phases, `sum = fir + sum`.

The generic formulation is also exact for R = 2 (`process_intratDOUBLE/HALF`), as the 96 kHz
null tests show.

---

## 6. Null tests

Source: `/Users/Shared/Maschine 2 Library/Projects/Chances.mxprj`, sound 0/10 slot 1 (stored
mode 1, saturation 0.186, bass 0.665, tone 0.938, focus on, definition 0.202, analog mix 1;
Mulholland fields at their defaults). (`Black Earth.mxprj` has more Distortion slots but its
`make`d projects fail to load in Maschine — "unregistered class" — even as pass-through.) Each case:
`maschine-ref make … 0 10 1 field=value…`, render 3 s with `--preroll 0`, `fx-null` on the
captured input (512-sample blocks). Script `t.sh` / `mx.sh` in the scratch directory
`…/scratchpad/fx-distortion/`. "bursts" = the Gate test file (random sine/noise bursts,
−45..0 dB, gaps), which exercises the gate and level follower. Residual = L / R RMS;
**−240 dB = bit-identical**.

| case | input | sr | parameters (others = project) | residual |
|---|---|---|---|---|
| Analog, project | noise | 44.1k | – | −240 / −240 |
| Analog, project | noise | 48k | – | −240 / −240 |
| Analog, project | noise | 96k | – (×2 converters) | −240 / −240 |
| Analog, project | bursts | 44.1k / 48k | – | −240 / −240 |
| Analog | sweep | 48k | – | −240 / −240 |
| Analog | impulse | 44.1k | – | −240 / −240 |
| Saturation 0 / 1 | bursts | 44.1k / 48k | | −240 / −240 |
| Bass 0 / 1 | noise / sweep | 44.1k / 48k | | −240 / −240 |
| Tone 0 / 0.5 | sweep / noise | 44.1k / 48k | | −240 / −240 |
| Focus off | bursts | 44.1k | | −240 / −240 |
| Definition 0 / 1 | bursts | 48k / 44.1k | | −240 / −240 |
| Mix 0 / 0.37 | noise / bursts | 44.1k / 48k | | −240 / −240 |
| Analog odd | bursts | 44.1k | sat .77, bass .21, tone .43, focus off, def .88, mix .6 | −240 / −240 |
| Mulholland defaults | noise | 44.1k / 48k / 96k | mode 0 | −240 / −240 |
| Mulholland | bursts | 44.1k / 48k | mode 0 | −240 / −240 |
| Mulholland | sweep / impulse | 44.1k / 48k | mode 0 | −240 / −240 |
| Drive 0 / 1 | bursts | 44.1k / 48k | | −240 / −240 |
| Color 0 / 1 / 0.5 | bursts / bursts / noise | 44.1k / 48k / 44.1k | | −240 / −240 |
| Feedback −1 / 1 | bursts | 44.1k / 48k | | −240 / −240 |
| Tone 0 (fb .8) / 1 (fb −.7) | bursts | 44.1k / 48k | | −240 / −240 |
| Tone Mod 1 / −1 / .37 | bursts | 44.1k / 48k / 44.1k | fb .6 / .6 / −.4, tone .3 | −240 / −240 |
| Gate off | bursts | 44.1k | | −240 / −240 |
| Release 0 / 1 | bursts | 48k / 44.1k | | −240 / −240 |
| Mix 0 / 1 | noise / bursts | 44.1k / 48k | | −240 / −240 |
| Mulholland odd | bursts | 48k | drive .83, color .71, fb .33, tone .12, tm −.45, rel .27, mix .66 | −240 / −240 |
| Mulholland odd 2 | sweep | 44.1k | drive .05, color .13, fb −.91, tone .97, tm .8, gate off, mix .9 | −240 / −240 |

The cases do change the signal (e.g. Mulholland defaults on bursts differ from the input by
−23 dB RMS, the odd case by −14 dB; Analog by −19..−21 dB).

Not tested: parameter changes mid-stream (the tools cannot automate). They follow §3.2/§4.3 as
decoded: Mulholland setters retarget the 50 ms glides, Analog sets restart the 20 ms glides,
and a Mode change just switches cores (both keep their state). Other rates than 44.1/48/88.2/96
kHz (e.g. 32 kHz) would need the generic (non-integer) converter, which is not ported; DECKARD
then rounds the ratio.
