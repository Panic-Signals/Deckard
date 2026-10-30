# Maschine's Perform FX: Reso Echo

A clean-room description of the Reso Echo mode (mode 3) of Maschine 3's Perform FX
(`NI::MASCHINE::DATA::PerformanceFX` → `NI::MASCHINE::RT::PerformanceFXProcessor` →
`NI::DSP::JamResoEcho`, with `NI::DSP::StateVariableFilter<float, 10>` and
`NI::DSP::Limiter`). The processor around the eight effects (strip routing, mode
crossfade, control ticks) is in [fx-perform.md](fx-perform.md). DECKARD's version is
`crates/engine/src/fx/perform/reso.rs`; it nulls bit-exactly against Maschine (§7).

Written from Ghidra output in `out/decomp/perform-reso/` and `out/decomp/perform-jam/`
(`INDEX.tsv` maps files to functions), `NI::DSP::Limiter` in `out/decomp/limiter/`
(0018–0031), and arm64 disassembly in `out/decomp/perform-reso-asm/` (`process.s`,
`svf.s`, `setsr.s`, `setparam.s`, `setfeedback.s`, `fbank.s`, `limiter.s`). Where they
disagree (fused multiply-adds, constants Ghidra rounds or mis-assembles from
`mov`/`movk`), this follows the assembly. Addresses are arm64 VM addresses in
`analysis/maschine_arm64`.

**Confidence.** **[exact]**: read from the code and confirmed by the null tests.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation: all arithmetic is `f32` unless marked `f64`. `fma(a, b, c)` = a·b + c with one
rounding (arm64 `fmadd`/`fmla`); where the code fuses, the description says so.
`clamp01(x)` = min(max(x, 0), 1) (NaN → 0). Library calls are Apple libm: `__exp10`
(f64), `exp2`, `exp` (f64), `tanhf`, `logf`. `E(x)` is the exp-table lookup of
docs/fx-eq.md §2.1 (`crate::fx::exp_e`).

---

## 1. Parameters **[exact]**

The core takes `setParameter(index, value)` (0x102630344); each value is also stored
raw (`+0x80 + 4·index`).

| index | field | stored → used |
|---|---|---|
| 0 | `touchstrip_pos` | §2.1 (bank frequencies, pan), also the feedback gain (§2.3) |
| 1 | `touchstrip_on_off` | 0/1, §2.6 |
| 2 | `reso_resonance` | `res = (f32)fma((f64)clamp01(v), (f64)0.26f, (f64)0.6f)`; damping `k = fma(res, −2, 2)` |
| 3 | `reso_feedback` | §2.3 |
| 4 | `reso_time` | `x = clamp01(fma(|v|, −0.5, 1.5)·v)`; `t_dB = (f32)fma((f64)x, (f64)30.9794f, 23)`; delay `= sr · (f32)(__exp10((f64)t_dB·0.05)·0.001)` samples (14.1–500 ms) |
| 5 | `reso_saturation` | drive `= (f32)__exp10((f64)(f32)fma(c, 14, −4)·0.05)`, makeup `= (f32)__exp10((f64)(f32)fma(c, −14, 4)·0.05)`, `c = (f64)clamp01(v)` (both in f64) |
| 6 | `reso_pump` | limiter drive `= (f32)__exp10((f64)(f32)fma((f64)clamp01(v), 24, 0)·0.05)` (0–24 dB) |
| 7 | `reso_mix_mode` | ≤ 0.5 Mix, ≤ 1.5 Insert, ≤ 2.00001 Wet only (larger: ignored); §2.7 |
| 8 | `reso_wet_level` | wet `= v·1.5` |

Indices 2, 4 and 0 recompute the feedback gain (§2.3) after their own change.

## 2. Structure

### 2.1 Glides and the strip **[exact]**

Seventeen linear glides (`+0x438` + 0x20·i) carry the settings to the audio: the strip
position, the five bank frequencies, the damping input `res`, the feedback gain, engage
(0/1), the delay time, drive, makeup, pump, insert amount, mix shape, dry level and wet
level. Each moves to a new target in equal steps over `len = (sr·0.001f)·20` samples
(f32; `n = (int)len`): a set computes `step = (target − cur)/len` and `count = n`. Per
sample, if `|target − cur| > 0` and the count (before decrementing) is ≥ 2, then
`cur += step`; otherwise `cur = target`. Settings that arrive while the core is suspended
(at load) land at once. This is `kit::Glide`.

Bank frequencies from the strip `p = clamp01(pos)`, as MIDI notes:
`note_i = (f32)fma((f64)p, A_i, B_i)`, `Hz_i = (f32)exp2(fma((f64)note_i, 1/12,
0x4008_4039_7fa8_81bf))`. The constant is log2(440) − 69/12 = 3.0313597.

| i | A | B | range |
|---|---|---|---|
| 1 (insert) | 110 | 20 | 26 Hz – 15 kHz |
| 2 | 85 | 0 | 8 Hz – 1.1 kHz |
| 3 | 76.7f | 30.3f | 47 Hz – 4.0 kHz |
| 4 | 80 | 41 | 87 Hz – 8.9 kHz |
| 5 | 80.2f | 49.8f | 141 Hz – 15 kHz |

(`f` after a value means the f32 constant promoted to f64.)

### 2.2 Per sample **[exact]**

1. Tick all 17 glides.
2. **Bank coefficients.** Ten band passes: a pair (left, right) at each frequency.
   `g = tan5(Hz·π_f·(1/sr))`, where `tan5(w) = fma(w⁵, 0x3e088889, fma(w³, 0x3eaaaaab,
   w))` with `w² = w·w`, `w³ = w·w²`, `w⁵ = w²·w³`. Damping `k = fma(res, −2, 2)`;
   `a1 = 1/fma(k+g, g, 1)`, `a2 = g·a1`, `a3 = g·a2`.
3. **Engage smoother.** `e` = a double one-pole (§2.5, 3 Hz) of the engage glide.
4. **Bank input.** `feed = e·1.3`; `a = fma(feed, inL, fbL)`, `b = fma(feed, inR, fbR)`
   (`fb` from the previous sample, §2.4). Bands 0, 1 (insert, frequency 1) take
   `inL·0.8`, `inR·0.8`; bands 2–9 (frequencies 2–5) take `a, b, a, b, …`.
5. **SVF** (Cytomic trapezoidal), each band: `v3 = x − ic2`;
   `v1 = fma(a1, ic1, v3·a2)`; `v2 = ic2 + fma(a2, ic1, v3·a3)`; `ic1 = 2·v1 − ic1`;
   `ic2 = 2·v2 − ic2`. Band output `k·v1` (unit-peak band pass).
6. **Pan.** From the strip glide `p`, four triangles:
   `tri(x, d) = ((|(x − (f32)(int)(x + 10.5)) + 10| + (−0.25))·4)·d` with
   `w0 = tri(p·0.7, 0.25)`, `w1 = tri(fma(p, 1.3, 0.1), 0.4)`,
   `w2 = tri(fma(p, 1.3, −0.3), −0.3)`, `w3 = tri(fma(p, 0.1, −0.2), −0.25)`.
   The side gains are `1 − w` (left) and `w + 1` (right), shaped by
   `curve(s) = (s·0x3eaaaa9f)·(4 − s)`.
7. **Sum** (per side; `r_j` = resonator j's band output on that side, j = 0..3 for
   frequencies 2..5): `v = r1·curve1; v = fma(r0, curve0, v); v = fma(r2, curve2, v);
   v = fma(r3, curve3, v)`; `sum = v + v`.
8. **High pass at 60 Hz.** Another trapezoidal SVF per side,
   `g = tan5((1/sr)·0x433c7edd)` (60π), `k = 2`, `a1 = 1/fma(g, g+2, 1)`, `a2`, `a3` as in
   step 2. `v3 = x − ic2`; `v2 = ic2 + fma(ic1, a2, a3·v3)`; `v1 = fma(ic1, a1, v3·a2)`;
   `hp = fma(−v1, 2, x) − v2`; `ic1 = fma(2, v1, −ic1)`; `ic2 = fma(2, v2, −ic2)`.
9. **Pump.** The limiter (§2.4) gets `hp·pump`; `unpump = 1/(pump + 1e-20f)`.
10. **Saturate.** `y = makeup·tanhf(drive·lim)`.
11. **Delay time.** `D` = the double one-pole (§2.5, 3 Hz) of the time glide (samples).
12. **Feedback level** (§2.3): `env = fma(0 − env, c, env)` (f64; `c` the release fall,
    or 1 while `env < 0`); `m = max(engaged ? 1 : 0, (f32)env)`;
    `F = (f32)fma((f64)m, (f64)(fb_glide + (−0.15f)), (f64)0.15f)`.
13. **Delay.** Write `y` (two buffers, length the power of two ≥ `(size)fma(sr, 0.5, 5)`,
    32768 at 48 kHz), advance the write index, then read the left at `−D` and the right
    at `D·(−0.8f)` (cubic, §2.8).
14. **Out.** `own = s·(2 − s)` and `echo = (1 − s)·(2 − (1 − s))` from the shape glide
    `s` (Mix 0.5: 0.75 each; Insert or Wet only 1: `y` alone).
    `wet = fma(tap, echo, y·own)·wet_level`. `fb = (tap·F)·unpump` for the next sample.
    `z = (f64)(insert·e)`; `dry = (f32)fma(z, (f64)(k·v1_band0or1 − in), (f64)in)`;
    `gain = (f32)fma(z, (f64)(wet_level − dry_level), (f64)dry_level)`;
    `out = wet + dry·gain`.

### 2.3 Feedback gain and release (`setFeedback` 0x10262f8c4) **[exact]**

With the raw Feedback `fb`, the mapped `res` and `t_dB`:

- `a = clamp01((res − 0.6f)·4)`; `gain = (f32)__exp10((f64)(f32)fma((f64)a, 3, −1.5)·0.05)`
  (−1.5 to +1.5 dB)
- `q = clamp01(pos_raw/0.2f)`; `posg = (f32)fma((f64)q, (f64)−0.20000005f, (f64)1.2f)`
  (1.2 at the strip's bottom, 1 from 0.2 up)
- `t = clamp01((t_dB − 30)/23)`; `tg = (f32)fma((f64)t, (f64)0.099999964f, (f64)0.6f)`
- `amt = (f32)fma((f64)clamp01(fb), (f64)(tg + (−0.2f)), (f64)0.2f)`
- **feedback glide target** `= gain·posg·amt`
- **release time:** `u = clamp01((t_dB + (−38))/12)`; `d0 = 45 − (f64)(f32)fma((f64)u, 10, 0)`;
  `T = __exp10(fma(clamp01(fb) as f64, 79 − d0, d0)·0.05)·0.001` s;
  `fall = 1 − exp(ln(0.001)/(T·sr))` (f64; `ln 0.001 = 0xc01ba18a998fffa0`).

Engaged, the feedback is the glide's value. **Released** (`setParameter(1, 0)` while
engaged), `env` restarts at 1 and falls by `fall` per sample (−60 dB in `T`), and the
feedback goes from the glide's value to 0.15 of it. The loop's gain then drops below one,
so the tail rings out and dies. It never sustains forever.

### 2.4 The limiter (`NI::DSP::Limiter`, set by `onSetSampleRate`) **[exact]**

Settings: Threshold 0.9, Release 0.01, Ceiling 0.94 (`0x3f70a3d7`), both flags off.

- `pre = E(fma((1 − 0.9f)·40, K, 1200))`, `post = E(fma((1 − 0.94f)·(−40), K, 1200))`,
  `K = 0x411f73da` (9.965784: dB → exp-table steps).
- Coefficients: `up = 1 − 1/(0.005f·sr)`, `down = 1 − 1/(max(0.01f·6, 0.001)·sr)`,
  `rise = 1/(−15·sr) + 1`, `fall = 1/(−3·sr) + 1`.
- Per sample: `a = |fma(L, pre, 1e-12)|`, `b = |fma(R, pre, 1e-12)|`, `peak = a < b ? b : a`;
  `dB = logf(peak)·0x410af968` (8.68589); `env = fma(env < dB ? up : down, env − dB, dB)`;
  `slow = fma(slow < dB ? rise : fall, slow − dB, dB)`; `m = env < slow ? slow : env`;
  `g = E(fma(min(0 − m, 0), K, 1200))`; out `= clamp(post·(g·x), −2.5, 2.5)`.
- Its 128-sample control tick only interpolates changed thresholds, which never change
  here.

### 2.5 The double one-pole (engage and delay time) **[exact]**

`G = tan5d(3·π·(f64)(1/sr))` (the same series in f64 with `0x3fd555555555554f`,
`0x3fc1111111111111`), `H = 1/(G + 1)`. Per sample, with `x = (f64)glide`:
`v = H·fma(−G, x, s·(−2)) + x`; `y = fma(s, 2, G·v)`; `s = fma(G, v, s)`; out `(f32)y`.
Settled at `x`: `s = x/2` (output `x`). This is `kit::Tpt`.

### 2.6 Engage **[exact]**

`setParameter(1, v)`: the engage glide's target = `v > 0.5`. Released while engaged sets
`env = 1` (§2.3). Then the dry level is reset: Mix 0.7 when engaged, else 1; Insert 1;
Wet only 0.

### 2.7 Mix modes **[exact]**

| mode | shape glide | insert glide | dry level |
|---|---|---|---|
| Mix | 0.5 | 0 | 0.7 engaged, 1 released |
| Insert | 1 | 1 | 1 |
| Wet only | 1 | 0 | 0 |

In Insert, the dry signal moves, with the engage smoother `e`, to the insert band pass
(frequency 1), whose level goes to the wet level.

### 2.8 The delay read **[exact]**

For position `x` (−D or −0.8·D): `i = (int)x − (x ≤ 0 ? 1 : 0)` (truncated, then one
lower), `f = (f64)x − (f64)i`, base = the write index (after the write) + `i`, and
`y(−1)`, `y0`, `y1`, `y2` around it. Catmull-Rom in f32 coefficients:
`c3 = fma(0.5, y2, fma(−1.5, y1, fma(−0.5, y(−1), y0·1.5)))`,
`c1 = fma(−0.5, y(−1), y1·0.5)`,
`c2 = fma(−0.5, y2, fma(2, y1, fma(−2.5, y0, y(−1))))`;
then in f64: `fma(f, c1, fma(f³, c3, f²·c2)) + y0` (f² = f·f, f³ = f·f²), rounded to f32.

## 3. Startup and reset **[exact]**

The core is built suspended (`+0xb8`), so settings land at once. `onSetSampleRate`
(sr ≥ 44 100) sets everything sample-rate dependent, recomputes the time and the
feedback, and calls `onReset`. That lands every glide, clears the band, high-pass and
delay states and the feedback, settles both one-poles at their targets (`s = target/2`),
and sets `env = 1`. A mode switch (fx-perform.md §2) gives the core the strip's position
and engage and then resets it, so it starts settled whatever came before.

## 4–6. —

## 7. Null tests

Renders by `maschine-ref` (out/perform-ref-ready.md): `make … --perform 3`, `render`
with its 1 s preroll, 1024-frame blocks, 120 BPM. Changes during a render use host
automation (`--host`, `--param`). The tests are
`fx::perform::tests::reso_echo_nulls_against_maschine` and
`reso_echo_releases_and_moves_as_maschines`; they skip when `out/perform-ref/reso/` is
absent. Residual is L/R RMS.

| case | settings | input | sr | residual |
|---|---|---|---|---|
| default | strip 0.5, all defaults | noise | 48k | −240 / −240 (bit-exact) |
| impulse | strip 0.5 | impulse 0 dBFS, 3 s | 48k | −240 |
| sweep | strip 0.5 | sweep | 44.1k | −240 |
| low | strip 0.05, Feedback 0.95, Saturation 1, Resonance 1 | noise | 48k | −240 |
| insert | strip 0.7, Insert, Time 0.8, Pump 1 | noise | 48k | −240 |
| wet | strip 0.3, Wet only, Wet Level 1, Time 0 | noise | 48k | −240 |
| off | released | noise | 48k | −240 (out = in) |
| release, impulse | released at 0.5 s | impulse, 3 s | 48k | −240 |
| release, noise | released at 1.0 s | noise, 3 s | 48k | −240 |
| strip moves | strip 0.5 → 0.1 (0.3 s) → 0.9 (0.6 s) → 0.33 (1.2 s) | noise | 48k | −240 |
| re-engage | released 0.5 s, engaged 0.9 s, released 1.4 s | noise | 48k | −240 |
| insert, full | strip 0.2, Feedback 1, Time 1, Insert, Pump 0.9; released 1.0 s, strip → 0.75 at 2.5 s | noise, 4 s | 48k | −240 |

Not tested against Maschine: changes of mode to or from Reso Echo (the other effects
weren't exact when this was written), and Mix Mode or Time changed while engaged (they
glide like the rest, from the code only).
