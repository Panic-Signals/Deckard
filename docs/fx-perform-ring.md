# Maschine's Perform FX Ring: parameters and DSP

A clean-room description of the **Ring** mode (mode 4) of Maschine 3's Perform FX
(`NI::MASCHINE::DATA::PerformanceFX` → `NI::MASCHINE::RT::PerformanceFXProcessor` →
`NI::DSP::JamRing`, which owns an `NI::DSP::PlateReverb`). DECKARD's version is
`crates/engine/src/fx/perform/ring.rs`; the reverb is DECKARD's existing port of
`NI::DSP::PlateReverb` (docs/fx-plate-reverb.md §4, `fx/plate_reverb/plate.rs`).

Written from Ghidra output in `out/decomp/perform-ring/` (`INDEX.tsv` maps files to
functions) and arm64 disassembly in `out/decomp/perform-ring-asm/` (`process.s`,
`pos.s`, `setparam.s`, `reset.s`, `setsr.s`, `init.s`). Where they disagree (fused
multiply-adds, NEON lanes, `fcvtas` rounding), this follows the assembly. Addresses are
arm64 VM addresses in `analysis/maschine_arm64`.

**Confidence.** **[exact]**: read from code or assembly (and, where §6 says so, confirmed
by null tests). **[inferred]**: reconstructed from partial evidence. **[unknown]**: not
recovered.

Notation: `v` is a stored value (0..1); `f32` unless marked `f64`; `fma(a,b,c) = a·b + c`
with one rounding; `sr` the sample rate; `clamp(x)` to 0..1.

---

## 1. Parameters

`DATA::PerformanceFX::connectParameters` binds each stored field to
`PerformanceFXProcessor::setFloatParameter/setEnumParameter(mode 4, index, …)`, which
calls `JamRing::setParameter(index, v)` **[exact]**. Index 0 (`touchstrip_pos`) arrives
through `setTouchstripPosition` and index 1 (`touchstrip_on_off`) through
`setTouchstripOnOff` (as 0/1).

| index | field | what `setParameter` does |
|---|---|---|
| 0 | `touchstrip_pos` | the pitches (§2.1) |
| 1 | `touchstrip_on_off` | glide 14 → `v > 0.5 ? 1 : 0` |
| 2 | `ring_spread` | stores `v` raw (`+0xa0`); glide 20 → `clamp(v / 0.15f)`; re-runs §2.1 |
| 3 | `ring_mix_mode` | glide 17 → `v > 0.5 ? 0 : 1` (1 = Mix); re-runs index 4 with its stored value |
| 4 | `ring_dry_wet` | `x = (v − 0.5); x = x + x`; dry = `x > 1 ? 0 : 1 − x`, then `x < 0 ? 1 : dry`; times `1 − (mix_mode > 0.5)`; glide 18 → dry, glide 19 → `clamp(v + v)` |
| 5 | `ring_rev_mix` | glide 15 → `v` |
| 6 | `ring_rev_time` | reverb decay `((fma(|v|, 0.75, 0.25)·v)·0.9f)`, exactly as `PlateReverb::setParameter(1, …)` |
| 7 | `ring_glide` | `v ≤ 0.01f`: rate `1/0.004f`; else `ms = f32(exp10(f64(f32(fma(f64 clamp(v), 40, 30)))·0.05))`, rate `1/(ms·0.001f)` (Hz); glide 16 → rate |

All **[exact]**. Every call ends, while the core is "immediate" (before its first resume,
and while suspended), by landing every glide on its target.

## 2. Smoothed values

21 linear glides (`+0xb8 + 0x20·i`: value, target, step, epsilon 0, length, countdown,
reload, flag) **[exact]**. Setting: target = `v`; if value ≠ target, step =
`(v − value)/length`, countdown = reload. Each sample: if `|target − value| ≤ 0` or the
countdown (before decrementing) is below 2, land (value = target, step 0); else value +=
step. Lengths (`onSetSampleRate`): `sr·0.001f·5` for glides 0–5, `·20` for 6–20; the
reload is that rounded to nearest (`fcvtas`).

| glide | holds |
|---|---|
| 0 | `note_hz(pos)` from the *raw* position (an 8.18–8.66 Hz value; used only by §4.5) |
| 1 | `note_hz(base)` |
| 2–5 | the four oscillators' `note_hz` |
| 6–13 | unused |
| 14 | engage (0/1) |
| 15 | Rev Mix |
| 16 | Glide rate (Hz) |
| 17 | Mix-mode flag |
| 18 | dry level |
| 19 | wet level |
| 20 | Spread / 0.15, clamped |

### 2.1 Pitches (`setTouchstripPosInternal`) **[exact]**

`x = f64(clamp(pos))`, `s = f64(clamp(spread_raw))`, notes as `f32` MIDI numbers:

- base `b = f32(fma(x, 70, 30))`
- `n2 = f32(fma(x, 40.8000001907, 17.2999992371))`
- `n3 = f32(fma(x, 51.3999984741, 28.7999992371))`
- `n4 = f32((log2(f64(f32(fma(x, −10.5999984741, 100)))) − 3.0313596…)·12)`
- `n5 = f32(fma(x, 52.3999977112, 60.7000007629))`
- each oscillator note `f32(fma(s, f64(n_k − b), f64(b)))`.

`note_hz(n) = f32(exp2(fma(f64(n), 1/12, 3.0313596…)))` (the constant is log2 of 8.1758 Hz,
bits `0x400840397fa881bf`). Glide 0 uses the raw (unclamped) `pos` in the same formula.

## 3. Lifetime

### 3.1 Construction and `onInit` **[exact]**

The core's own `PlateReverb` is constructed and initialised as the Reverb module's
(docs/fx-plate-reverb.md §4), then set: pre-delay 0, mix 1 (fully wet), wet gain 1.5,
low shelf 0.5, decay 0.5, high damp 0.2. The core starts "immediate". `+0x8e0` (the
envelope's log constant) = bits `0xc0026bb1bbb5317f` (≈ ln 0.1).

### 3.2 `onSetSampleRate(sr, 128)` **[exact]**

Ignored below 44.1 kHz. Glide lengths (§2); band-pass `T = 1/sr` (f64), `k = 1.2f` (f64
`0x3ff3333330000000`); the pitch filters' `T = 1/sr` (f32; their states cleared when it
changes); engage envelope `attack = 1 − exp(c/(sr·0.005f))`, `release = 1 − exp(c/(sr·0.1f))`
(f64); `setParameter(0, pos)`; the reverb's own `onSetSampleRate(sr, 128)`.

The processor calls every core's `onSuspend`, then `onSetSampleRate(sr, 128)`, then the
current core's `onReset` and `onResume`.

### 3.3 `onReset` **[exact]**

Every glide lands; band-pass states 0; oscillators to phase 0 (`x = 1, y = 0`); the
reverb's `onReset`; then the six pitch filters (§4.1) are run in: their coefficient from
`tan_series(f64(T)·62831.853…)` (bits `0x40eeadfb4c5d390c`, i.e. 10 kHz), states from 0,
20 iterations of the filter on the glides' *targets*; finally the Glide coefficient is
marked stale so the first sample recomputes it.

### 3.4 Control rate **[exact]**

`processControlRate` is the reverb's (ramp bookkeeping, docs/fx-plate-reverb.md), called
by the processor at the start of every 128-sample block.

## 4. Per sample (`processAudio`) **[exact]**

1. Advance all 21 glides.
2. **Glide filters.** If glide 16 changed since last time: `g = tan_series(f64(rate)·π·f64(T))`,
   `a = 1/(g + 1)` (f64). Six one-pole filters, inputs glides 2, 3, 4, 5, 1, 0 (as f64):
   `t = fma(x, −g, s·−2)`, `u = a·t + x`, `y = fma(2, s, g·u)`, `s = fma(u, g, s)`.
   `tan_series(x) = fma(x⁵, 0.1333…, fma(x³, 0.3333…, x))` with `x³ = x·x²`, `x⁵ = x²·x³`
   (constants `0x3fc1111111111111`, `0x3fd555555555554f`).
3. **Band-pass tuning.** `base = f32(y_base)`; per oscillator k: `fc = f32(fma(f64 spread_raw,
   f64(f32(y_k) − base), f64 base))` (spread *unclamped*); `g = tan_series((fc·π)·T)`,
   `a1 = 1/fma(k + g, g, 1)`, `a2 = g·a1`, `a3 = g·a2`.
4. **Oscillators.** `w = (f64(f32 y_k)·2π)/f64(sr)`; `c, s = cos w, sin w` (times an
   amplitude of 1); `x' = fma(c, x, −(s·y)) + 0`, `y' = fma(s, x, c·y)`; output `f32(y')`.
5. **Ring and bands.** Per band and channel, `in = f64(input)·f64(osc_k)`;
   trapezoidal SVF: `v3 = in − ic2`, `v1 = fma(ic1, a1, a2·v3)`, `v2 = ic2 + fma(a2, ic1, a3·v3)`,
   `ic1 = 2v1 − ic1`, `ic2 = 2v2 − ic2` (fused); the band pass `v1` is the output.
6. **Panning.** `r = f32(y_raw)`; triangles `tri(a) = (|a − (trunc(a + 10.5) − 10)| − 0.25)·4`:
   `p2 = tri(fma(r, 1.3, 0.1))·0.6`, `p3 = tri(fma(r, 1.3, −0.3))·0.7`,
   `p4 = tri(fma(r, 2.7, −0.2))·0.8` (all f32). `sp = f64(glide 20)`, `s2 = f64(f32(fma(sp, sp, 0)))`.
   `pan(u) = (u·⅓)·(4 − u)` (f64). Left: band 1 · `pan(1 − s2)`, band k+1 · `pan(1 − p_k)`·sp;
   right with `1 + …`. Sums `L = b4 + (b3 + (b2 + b1))`, times `0.5899999737739563`, to f32.
7. **Engage envelope.** `e = f64|glide 14|`; `env = fma(e − env, e > env ? attack : release, env)`.
8. **Wet.** `level = f32(fma(sp, −0.70000005, 1.70000005))`;
   `wet = level · f32(fma(f64(f32 env), f64 glide 19, 0))`; ring = `wet · L` (and R).
9. **Reverb.** The ring pair through the Plate reverb, in place, one sample
   (`processAudioImpl<false>`).
10. **Mix.** `dry = f32(fma(f64(f32 env), f64(glide18 − glide17), f64 glide17))`;
    `rev = clamp(m + m)`, `direct = clamp((1 − m) + (1 − m))` with `m` = glide 15;
    `out = fma(dry, input, reverb·rev) + ring·direct`.

So released (envelope 0) the input passes at the Mix-mode flag's level: unity in Mix,
silence in Wet only, while the reverb's tail plays on.

## 5. DECKARD's version

`ring.rs` follows §1–§4. The reverb is `fx/plate_reverb/plate.rs` compiled as a private
module of `ring.rs` (`#[path]`), because `fx::plate_reverb` keeps `Plate` private; the
Ring's settings go in through `Plate::set_parameter` (indices 0, 1, 7, 8, 9, 10), which do
what `JamRing::onInit` and `setParameter(6)` do to the reverb's fields directly.
Parameters are applied on change. The first `set` lands them all, then runs
`onSetSampleRate`, `onReset` and `onResume`, in that order **[inferred]**: the
reverb's decay coefficients are worked out from its *current* decay (docs/fx-plate-reverb.md
§4.1), so they are right only if the sample rate is set up after the parameters arrive;
with the rate set up first, the reverb decays as for Rev Time's default and the renders
don't null (−32 to −44 dB). With this order every render below nulls exactly.

## 6. Null tests

Renders by `maschine-ref` (out/perform-ring/, not committed): sound 0/0 → group 0 with a
Perform FX in Ring mode, engaged from load, 1 s preroll, 2 s of input at −12 dBFS. The
test `fx::perform::ring::tests::nulls_against_maschine` runs `Ring` over the same input
after 1 s of silence and reports the worst sample of the difference.

| case | settings (others at Maschine's defaults; engaged) | input | sr | worst difference |
|---|---|---|---|---|
| dry | pos 0.2, Spread 0, Rev Mix 0, Dry/Wet 1 | noise | 44.1k | exact (0) |
| spread | pos 0.8, Spread 1, Rev Mix 0, Dry/Wet 0.7 | noise | 44.1k | exact |
| def | defaults (pos 0.5, Spread 0.3, Rev Mix 0.5, Rev Time 0.1, Dry/Wet 0.35) | noise | 44.1k | exact |
| rev | Rev Mix 1, Rev Time 0.8, Dry/Wet 0.6 | noise | 44.1k | exact |
| wet | Wet only, Dry/Wet 0.5, Spread 0.6 | sine 440 | 44.1k | exact |
| s48 | pos 0.35, Glide 0.5, Rev Mix 0.3 | noise | 48k | exact |
| pos0 | pos 0, Spread 0.05, Dry/Wet 0.9, Rev Mix 0.2 | sweep | 44.1k | exact |
| pos1 | pos 1, Spread 0.15, Dry/Wet 0.25, Rev Time 1 | noise | 44.1k | exact |
| imp | pos 0.6, Rev Mix 0.8, Rev Time 0.5, Dry/Wet 1 | impulse | 44.1k | exact |
| dw0 | Dry/Wet 0, Rev Mix 0.7, Rev Time 0 | noise | 48k | exact |
| r96 | pos 0.45, Spread 0.9, Glide 1, Wet only | noise | 96k | exact |
| r88 | pos 0.7, Spread 0.4, Glide 0.005, Rev Mix 1 | sine 220 | 88.2k | exact |
| off | released, Rev Mix 0.6 | noise | 44.1k | exact (= input) |
| offwet | released, Wet only | noise | 44.1k | exact (silence) |

The same renders through the whole `PerformFx` (`fx-null`, def, rev, wet, imp) also null
at −240 dB.

## 7. Open points

- Parameter changes mid-stream, the strip moving, and release aren't rendered by
  maschine-ref (it renders a fixed project), so glides and the envelope's release are
  from the code only.
- Oscillator amplitudes (`+0x388…`) are 1 and never changed by any code read.
