# Maschine's Perform FX Tremolo: parameters and DSP

A clean-room description of the Tremolo mode (mode 6) of Maschine 3's Perform FX
(`NI::MASCHINE::DATA::PerformanceFX` → `NI::MASCHINE::RT::PerformanceFXProcessor` →
`NI::DSP::JamTremolo`, with an embedded `NI::DSP::FreqShifterProcessor` for the Freq Shift
vibrato). DECKARD's implementation is `crates/engine/src/fx/perform/tremolo.rs`, written
from this description; §8 has the null tests against Maschine.

Written from Ghidra output in `out/decomp/perform-tremolo/` (`INDEX.tsv`) and arm64
disassembly in `out/decomp/perform-tremolo-asm/` (`processaudio.s`, `setparam.s`,
`setsr.s`, `ctor.s`, `switchvib.s`, `fs-setsr.s`). Where they disagree the assembly wins
(Ghidra drops NEON lanes and fused multiply-adds). Addresses are arm64 VM addresses in
`analysis/maschine_arm64`.

**Confidence.** **[exact]**: read from code or assembly. **[inferred]**: reconstructed from
partial evidence. **[unknown]**: not recovered.

Notation: `f32` unless marked `f64`. `fma(a,b,c)` = `a·b + c` with one rounding (the
compiler fuses these; they matter for bit-exactness). `clamp01(x)` = `min(max(x,0),1)`
with `x < 0 → 0` tested first. `sr` = sample rate.

---

## 1. Parameters

The processor hands each stored field to the core as `setParameter(index, value)` with
the stored 0–1 value (enums as `0., 1.`) **[exact]**, `connectParameters`:

| index | field | use |
|---|---|---|
| 0 | `touchstrip_pos` | strip position (`setTouchstripPosition`) |
| 1 | `touchstrip_on_off` | engage (`setTouchstripOnOff`, 0/1) |
| 2 | `tremolo_attack_release` | Attk/Rel |
| 3 | `tremolo_stereo` | Stereo |
| 4 | `tremolo_trem_vibr` | Trem/Vibr |
| 5 | `tremolo_vibr_mode` | Vibr Mode: 0 Varispeed, 1 Freq Shift |
| 6, 7 | `tremolo_depth_min`, `_max` | Depth range |
| 8, 9 | `tremolo_rate_min`, `_max` | Rate range |

Every parameter is kept raw (`p[i]`). Setting index *i* recomputes targets as follows
**[exact]**; a target only changes its smoother (§2) when the value differs.

- **1 (engage):** `E.target = p1 > 0.5 ? 1 : 0`.
- **2 (Attk/Rel), x = p2:**
  - `a = clamp01((1.5 − 0.5·|x|)·x)` (the `1.5 − 0.5|x|` is `fmsub`, one rounding);
    `Attack.target = f32(exp10(f64(f32(fma_f64(a, 35, 40))) · 0.05)) · 0.001` (seconds).
  - `y = fma(|x|, 0.2, 0.8)·x`, `r = clamp01(y·(1.5 − 0.5·|y|))`;
    `Release.target = f32(exp10(...r...)) · 0.0004`.
- **4 (Trem/Vibr), x = p4:** `vibW = clamp01(2x)`, `tremW = clamp01(2·(1−x))` (`1−x`
  first), then the whole of "default" below runs (as for index 7).
- **5 (Vibr Mode):** if `(p5 > 0) ≠ mode`, `Fade.target = 0` (the mode switches when the
  fade reaches 0, §4).
- **0, 3, 6–9 (default):** with `s = clamp01(p0)` (`f64`), `d0 = p6`, `d1 = p7`:
  - **Rate (Hz):** `(r0, r1) = f32(fma_f64(clamp01(p8|p9), 34.882686614990234,
    −24.376316070556641))`; `Rate.target = f32(exp2(fma_f64(f64(f32(fma_f64(s,
    f64(r1−r0), r0))), 1/12, 3.0313596725463867)))`. Roughly 1 Hz (rate 0) to 18 Hz
    (rate 1), exponential.
  - **Vibrato depth:** `g(d) = d·(0.39999998 − |d|·(−0.6))` (fmsub);
    `Vib.target = vibW · f32(fma_f64(s, f64(g(d1)−g(d0)), g(d0)))`.
  - **Tremolo depth:** `h(d) = d·fma(−|d|, −0.7, 0.3)`;
    `Trem.target = tremW · f32(fma_f64(s, f64(h(d1)−h(d0)), h(d0)))`.
  - **Pan:** `Pan.target = (p3·(1 − tremW)) · f32(fma_f64(s, f64(d1−d0), d0))`.
  - **Stereo:** `Stereo.target = p3`.

## 2. Smoothers

Linear ramps **[exact]**: `{cur, target, inc, steps (f32), count, nsteps (i32), active}`.
A new target `t ≠ cur` sets `inc = (t − cur)/steps`, `count = nsteps`, `active`. Each
sample, before anything else: if `|t − cur| ≤ 0` or `--count ≤ 0` (old count ≤ 1), an
active ramp lands (`cur = t`, inactive); otherwise `cur += inc`. Steps
(`fs = sr·0.001`, `steps = fs·N`, `nsteps = (i32)steps`): **Fade** 15 ms, everything else
200 ms (Rate, Trem, Vib, Pan, Attack, Release, Stereo, E).

Until the processor resumes (`suspended`, true from construction), every setParameter
lands every active ramp at once: a project loads with its values in place.

## 3. Per sample

Order **[exact]**: all smoothers step (§2); the fade check (§4); then:

1. **LFO phase** (`f64`): `u = fma_f64(f64(Rate), 1/sr, φ)`; `φ = u − (f64(i32(u + 10))
   − 10)` (truncating conversion). `u` is kept (unwrapped) for step 6.
2. **Engage envelope**: two followers in series. With `cA = 1 − expf(−4.5/(Attack·sr))`
   (1 when `Attack ≤ 0`) and `cR` from Release likewise, recomputed every sample:
   `e1 = fma(|E| − e1, |E| > e1 ? cA : cR, e1)`; `e2 = fma(|e1| − e2, |e1| > e2 ? cA :
   cR, e2)`. `e2` is the envelope.
3. **Sine**: `x = f32(φ − 0.5)·6.2831855`; `S(x) = x·fma(fma(fma(fma(fma(x², −2.39e−8,
   2.7526e−6), x², −0.000198409), x², 0.00833333), x², −0.166667), x², 1)` (the f32
   constants `0xb2cd4ca8 0x3638b94f 0xb9500c03 0x3c088885 0xbe2aaac1`). `sinL = S(x)`
   (≈ `−sin 2πφ`).
4. **Vibrato** (§5) gives `vL, vR`.
5. **Right and pan phases** (f32): `u32 = f32(u)`; `qR = Stereo·0.5 + u32`,
   `qP = u32 + 0.5`; each wrapped `q − f32(i32(f64(q)+10))…` exactly as
   `f32(f64(q) − (f64(f32(i32(f64(q)+10))) − 10))`, then `S((q − 0.5)·6.2831855)`:
   `sinR`, `sinP`.
6. **Output**:
   - `td = Trem·e2`; `L = fma(td, sinL, 1)·vL`, `R = fma(td, sinR, 1)·vR`.
   - `p = sinP·(e2·Pan)`; `gL = 1 − p`, `gR = p + 1`; `L ·= (gL·0.33333334)·(4 − gL)`,
     `R ·= (gR·0.33333334)·(4 − gR)` (a soft auto-pan; unity at p = 0).
   - `outL = fma(inL, 1 − Fade, Fade·L)`, likewise R.

The tremolo is unipolar around unity: the level swings `1 ± td`, phase-shifted by
Stereo/2 of a cycle on the right.

## 4. Vibrato mode switch

When `Fade.cur == 0` at the check, `switchVibratoMode` runs **[exact]**: if `(p5 > 0.5)`
differs from the current mode, the Freq Shift state (allpass stages, oscillator = (1, 0))
and the Varispeed delay lines are cleared; the mode is set; `Fade.target = 1`. Then, if
the mode is Varispeed, `Vib.cur = 0` and it ramps (200 ms) to its target. The fade starts
at 0, so this runs on the very first sample: every Tremolo fades in over 15 ms from the
dry signal and its Varispeed depth ramps in over 200 ms.

## 5. Vibrato

**Varispeed** (mode 0): two delay lines (power of two ≥ `(i64)(sr·0.15)`). Each sample
writes the input, then, if `e2 > 0`:

- `max = fma(sr, 0.15, −3)`; `d = ((Vib·0.2)·e2/Rate)·f32(fma_f64(f64(sinL), 0.5, 0.5))·sr`,
  `d = min(max, max(d, 3))` (samples).
- `k = i32(−d) − (d ≥ 0 ? 1 : 0)`; base = write index (after the write) + k; `t = f64(−d)
  − f64(k)`; taps `p0..p3` at base − 1 … base + 2.
- Cubic: `c3 = fma(p3, 0.5, fma(p2, −1.5, fma(p0, −0.5, p1·1.5)))`,
  `c2 = fma(p3, −0.5, fma(p2, 2, fma(p1, −2.5, p0)))`, `c1 = fma(p0, −0.5, p2·0.5)`;
  `y = f32(fma_f64(c1, t, fma_f64(c3, t³, t²·c2)) + p1)` with `t² = t·t`, `t³ = t·t²` in
  `f64`.

With `e2 ≤ 0` the vibrato is the dry input.

**Freq Shift** (mode 1), always running (also disengaged):

- `hz = sinL·((Vib·e2)·90)`; `w = (f64(hz)·2π)/f64(sr)`; `(s, c) = sincos(w)`.
- A Hilbert pair: 7 first-order allpass stages on 4 lanes `[L·a, L·b, R·a, R·b]`, each
  `y = old + k(x − old)`, `old ← x + k(x − old)` (that is `(k + z⁻ᴰ)/(1 + k z⁻ᴰ)`), `D =
  min(i32((sr+1)/44100), 8)` (1 at 44.1/48 kHz). Coefficients, computed by Maschine as
  reciprocal estimate × 2 Newton steps × numerator (so a few differ by one ulp from a
  true division) **[exact]**:

  | stage | lane a | lane b |
  |---|---|---|
  | 1 | `0x3d047cd2` | `0x3eb76271` |
  | 2 | `0x3f1ddb76` | `0x3f4a7446` |
  | 3 | `0x3f662b90` | `0x3f784ff1` |
  | 4 | `0xbf129682` | `0xbf424d26` |
  | 5 | `0xbf5f0faa` | `0xbf6f3ac2` |
  | 6 | `0xbf783b8e` | `0xbf7dbf3b` |
  | 7 | −0.3 | −0.3 |

  Lane a is taken after stage 6, lane b after stage 7.
- Oscillator (`f64`): `re' = fma(c, re, −(s·im)) + 0`, `im' = fma(s, re, c·im)`;
  `L = f32(im')·a_L + f32(re')·b_L`, `R` likewise.

With no shift the oscillator stays at (1, 0) and the output is lane b: an allpass (phase
shifted) copy of the input, so a disengaged Freq Shift Tremolo is not bit-transparent
(confirmed against Maschine, §8).

## 6. The processor around it

`PerformanceFXProcessor` runs the current mode's core on every block (in 128-frame
pieces), engaged or not; engaging is the core's own (§1 index 1, §3.2). Crossfades
between modes are the processor's (shared; see `docs/fx-perform.md`).

## 7. Open points and rulings

- Parameter changes mid-stream: the order in which several changed fields reach the core
  isn't known; DECKARD applies each changed index in index order, and only changed ones
  (a repeated target would restart its ramp, as in Maschine). **[inferred]**
- The core ignores sample rates outside 44.1–176.4 kHz (its settings stay at the
  previous rate's). DECKARD sets the rate it runs at regardless. If wrong: nothing at
  the usual rates; below 44.1 kHz DECKARD's Tremolo works where Maschine's would misbehave.
- The Tremolo reads the strip position and touch from its own parameters, not the shared
  processor's glided position: Maschine smooths them with the core's own 200 ms ramps.
- Not rendered against Maschine: strip movement and release mid-stream (maschine-ref
  renders a fixed project), and Vibr Mode switching; these are from the code only.

## 8. Null tests

`out/perform-tremolo/` (`maschine-ref make … --perform 6 [field=value…]`, `render --input
noise --seconds 2`, 1 s preroll, engaged from load unless said), against DECKARD's whole
`PerformFx` run the same way (`tremolo.rs` tests). Residual is the worse of L/R, dB RMS.

| case | settings | sr | residual |
|---|---|---|---|
| def | defaults (Trem/Vibr 0.3, Varispeed) | 48k | −240 (bit-exact) |
| vib | Trem/Vibr 1 | 48k | −240 |
| vib2 | Trem/Vibr 0.8, pos 0.9, Depth max 1, Rate max 0.9 | 44.1k / 48k / 96k | −240 each |
| fs | Trem/Vibr 1, Freq Shift | 44.1k / 48k / 96k | −240 each |
| fs-sine | as fs, sine 440 Hz input | 48k | −240 |
| mix | Trem/Vibr 0.6, Stereo 1, pos 0.2, Attk/Rel 0 | 44.1k / 48k / 96k | −240 each |
| fast | Trem/Vibr 0, pos 1, Rate max 1, Depth max 1, Attk/Rel 1, Stereo 0.7 | 48k | −240 |
| off | disengaged | 48k | −240 (= input) |
| fsoff | disengaged, Freq Shift | 48k | −240 (an allpass copy, not the input) |
