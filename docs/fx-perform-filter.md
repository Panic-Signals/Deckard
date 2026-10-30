# Maschine's Perform FX Filter: parameters and DSP

A clean-room description of the **Filter** mode (mode 0) of Maschine 3's Perform FX
(`NI::MASCHINE::RT::PerformanceFXProcessor` → `NI::DSP::JamFilter`, with
`NI::DSP::TSKFilterNLA<float, 2>` as its core and `NI::AB::SampleRateConverter` for 2×
oversampling). DECKARD's version is `crates/engine/src/fx/perform/filter.rs`, written from
this description.

It was written from Ghidra output in `out/decomp/perform-filter/` (`INDEX.tsv` maps files to
functions; names in `out/decomp/perform-filter-names.txt`; a read-only copy of the Ghidra
project was used) and arm64 disassembly in `out/decomp/perform-filter-asm/` (`process.s`,
`tsk.s`, `tskfreq.s`, `params.s`, `init.s`, `display.s`). Where Ghidra and the assembly
disagree (fused operations, `frinti` shown as a truncation, NEON lanes) this follows the
assembly. Addresses are arm64 VM addresses in `analysis/maschine_arm64`. The processor around
the eight effects (engage events, mode switching) is described in `docs/fx-perform.md`.

**Confidence.** **[exact]**: read from code or assembly. **[inferred]**: reconstructed from
partial evidence. **[unknown]**: not recovered.

Notation: `f32` arithmetic unless marked. `fma(a, b, c)` = `a·b + c` with one rounding; `fma₆₄`
the same in `f64` on the values widened; every other product or sum is rounded on its own.

---

## 1. Parameters **[exact]**

`JamFilter::setParameter(index, value)` (0x10262624c). The processor sends the module's
stored values: the strip's on/off as 0/1, the type as its index, the rest as 0–1.

| idx | field | setter | DSP value |
|---|---|---|---|
| 0 | `touchstrip_pos` p | `setTouchstripPosition` | pitch `P = (f32) fma₆₄(clamp(p), Pmax − Pmin, Pmin)` (MIDI note numbers; `Pmax − Pmin` in f32); then `setRes` again |
| 1 | `touchstrip_on_off` | `setTouchstripOnOff` | engage target 1 if > 0.5, else 0 |
| 2 | `filter_min_freq` v | `setFreqMin` | `Pmin = (f32) fma₆₄(clamp(v), 50, 10)` (notes 10–60, 13 Hz–294 Hz) |
| 3 | `filter_max_freq` v | `setFreqMax` | `Pmax = (f32) fma₆₄(clamp(v), 30, 110)` (notes 110–140, 4.7–26.6 kHz) |
| 4 | `filter_resonance` v | `setRes` | §1.2 |
| 5 | `filter_type` | – | 0 LP (< 0.5), 1 BP (< 1.5), 2 HP |
| 6 | `filter_saturation` v | `setSat` | drive `(f32) exp10(f64((f32) fma₆₄(clamp(v), 50, 0))·0.05)` (0–50 dB) |
| 7 | `filter_out_level` v | `setOutputGain` | the same with `fma₆₄(clamp(v), 50, −50)` (−50–0 dB) |

`clamp` is to 0..1. Every value is also cached by index (the strip position and resonance
are read back from that cache).

### 1.1 Cutoff

`f = (f32) exp2(fma₆₄(P, 1/12, 3.03135967…))` (constants `0x3fb5555555555555`, `0x400840397fa881bf`) (`exp2` from the system library;
`8.1758·2^(P/12)`, the MIDI note's frequency). It is the target of the cutoff glide (§3).
`setFreqMin`, `setFreqMax` and `setTouchstripPosition` recompute it, then the resonance.

### 1.2 Resonance

```
r = v·1.2
r = r · (r < 1 ? fma(|r|, −0.9, 1.9) : 1)
if r > 0.6 and f_target > 16000:
    r = (f32) fma₆₄(clamp((f_target − 16000)/8000), 0.6 − r, r)
```

`f_target` is the cutoff glide's target at that moment. `r` is the resonance glide's target.
The DATA side shows Resonance as `1.2·v` (`s_maxResonance`).

### 1.3 Defaults

Constructor: `Pmin` 20, `Pmax` 100, saturation clip constants (§4.2), every glide at 0. The
processor then sets each parameter while the effect is suspended (§3.2), so a new effect
starts at its values without gliding.

## 2. Signal flow **[exact]**

Per host block, in chunks of at most 512 samples:

1. **Oversampling** when the sample rate is below 88 kHz (factor F = 2, flag at +0xc4); at
   88 kHz and above F = 1 and no converters run. Each channel has its own up- and
   down-converter: `SampleRateConverter::init(sr, 2·sr, 1 channel, quality 2)` and back.
   These are the integer-ratio ×2 / ×½ converters of docs/fx-saturator.md §5 (Tape): the
   same 16 taps, even outputs of the up-converter the input delayed by 8 samples, odd ones
   the FIR; the down-converter `½·(even input delayed by 8 + FIR over the odd inputs)`.
2. For each of the `(u64)fma(F, n, 0.1)` oversampled samples:
   1. tick the six glides (§3), in the order cutoff, resonance, saturation, engage, out
      level, type fade;
   2. the type fade (§3.3);
   3. two draws of the noise generator (§2.1);
   4. coefficients, if the cutoff or the resonance changed (§4.1);
   5. `x'ᵢ = fma(nᵢ, 1e-30, xᵢ)` for each channel (the input plus a tiny noise);
   6. the core (§4) on `x'ᵢ · (drive·0.5)` (the drive product first, then times the input);
   7. the mode's output `y` (§4.3), times the type fade, times `(out + out)`;
   8. `outᵢ = fma(engage, yᵢ − x'ᵢ, x'ᵢ)`: the dry signal is the input after the
      converters, so even released the effect delays and slightly filters the signal;
   9. the engage glide's value is kept (+0xb4) for §3.2.
3. Down-conversion.

### 2.1 Noise

A 32-bit LCG, `s = s·0x19fb4787 + 0x16d1b86b` (wrapping), drawn twice per sample (left,
right). Each draw gives `n = f32(0x40000000 | s >> 9) − 3` (−1..1). The seed is
`g_seed·(−0x61cac084)`, from a global `ni::random` seed factory (dynamically initialised;
each new instance advances it). **[exact]** in effect: it differs from run to run, since two
Maschine renders of the same self-oscillating project differ (§5). At 1e-30 the noise only
matters in silence, about 600 dB down, except that it is what a self-oscillating filter
grows from. DECKARD seeds it with 0.

## 3. Glides **[exact]**

Six linear glides, each: value, target, step, epsilon (0), length `L = (sr·F)·0.001·20`
(f32, 20 ms of oversampled samples), counter, reload `(int)L`, pending, done.

- **Set target `t`:** if `t ≠ value`: `step = (t − value)/L`, counter = reload, pending.
- **Tick:** if `|t − value| <= 0`, or the counter, decremented, is `<= 0`: if pending, snap
  (value = target, step 0, counter 0, not pending). Otherwise `value += step`.
- **Snap all:** snaps every pending glide.

### 3.1 What each glide holds

| glide | target |
|---|---|
| cutoff | `f` in Hz (§1.1) |
| resonance | `r` (§1.2) |
| saturation | drive |
| engage | 0 / 1 |
| out level | gain |
| type fade | 0..1 |

### 3.2 Instant changes

The setters of the cutoff, resonance, saturation and out level (and so the strip position),
when the engage glide's last value (+0xb4) is exactly 0, snap: the strip position snaps the
cutoff alone, then (through `setRes`) every pending glide; `setRes`, `setFreqMin/Max`,
`setSat` and `setOutputGain` snap every pending glide. `setTouchstripOnOff` doesn't snap; when
it engages with +0xb4 = 0 it clears the core's states (§4). While suspended (flag +0x5790,
set by `onSuspend` and at construction), every setter snaps all glides.

### 3.3 Type fade

`setParameter(5)` with a different type: the next type is kept and the fade's target set to
0. Each sample, after the tick, if the fade's value is exactly 0: the type becomes the next
one, the target is set to 1 (gliding from 0), and the core's states are cleared. The wet
signal is multiplied by the fade.

`onResume` sets the fade's value and target to 0 (and leaves suspension). So after every
resume, the first sample switches to the (same) type and the wet signal fades in over 20 ms.

### 3.4 Reset

`onReset` (also from `onSetSampleRate`): clears the core's states, snaps the glides, resets
the converters, sets the type to the next one, marks the coefficients stale and sets the
cutoff and resonance targets again.

## 4. The core: `TSKFilterNLA<float, 2>` **[exact]**

A nonlinear two-integrator filter per channel with resonance feedback, every stage through a
soft clip. Constants (onInit): clip A `L₁ = 4`, `s₁ = 0.25`; clip B `L₂ = 3.4`,
`s₂ = 0.29411766` (`0x3e969696`); `m = 0.2`. `T = 1/(sr·F)`.

### 4.1 Coefficients

When the glided cutoff `f` differs from the last one used:

```
w = (f·π)·T                      (π = 3.1415927f)
x = min(w, 1.50845)              (0x3fc114e4)
g = atan≈(x)
```

`atan≈`: for `x > 1`: `ρ = 1/x`, `g = 1.5707954 − ρ·A(ρ)`; for `x < −1` the same with
−1.5707954; else `g = x·A(x)`. `A(ρ) = 1 + q·(c₇ + q·(c₆ + … q·(c₁ + ρ·c₀)))`, `q = ρ²`, each
product and sum rounded on its own, with c₀..c₇ = 0.00286623, −0.0161657, 0.0429096,
−0.0752896, 0.106563, −0.142089, 0.199936, −0.333331 (`0x3b3bd75c`, `0xbc846dee`,
`0x3d2fc1fa`, `0xbd9a316f`, `0x3dda3db4`, `0xbe117fc7`, `0x3e4cbc06`, `0xbeaaaa5c`).

Then, with the resonance feedback `k`:

```
b₁ = (g·(g + 1)) / fma(g, (2 − k) + g, 1)
b₂ = g / (g + 1)
ι  = 1 / g
```

For the high pass (§4.3): `w' = ((f·0.5)·π)·T`, `h = fma(w'²·w'³, 0.13333334,
fma(w'³, 0.33333334, w'))` (w'² = w'·w', w'³ = w'·w'², w'²·w'³ rounded), `c = 1/(h + 1)`.

When the glided resonance `r` differs from the last one used: `k = r < 2 ? r + r : 4`, then
the coefficients again (the same formulas, with the new `k`).

### 4.2 Soft clip `S(v; s, L)`

```
t = s·v
if |t| < 0.001 (compared in f64):  return t·L
if |t| >= 10:                      return copysign(L, v)
y = t·2.88539                      (0x4038aa3b)
i = round-to-nearest-even(y) as integer
e = fma(fma(fma(y − i, 0.056, 0.242), y − i, 0.693), y − i, 1) · 2^i
                                   (0x3d656042, 0x3e77ced9, 0x3f316873; 2^i built from bits)
return ((e − 1)/(e + 1))·L
```

That is `L·tanh(v/L)` with `s = 1/L`, by `tanh = (e^{2t} − 1)/(e^{2t} + 1)`.

### 4.3 One sample, one channel

States: `y₁, y₂` (integrators), `y₃ = S_A(y₁)`, `y₄ = S_A(y₂)`, `xₚ` (last input).
`S_A = S(·; s₁, L₁)`, `S_B = S(·; s₂, L₂)`. `M(a, b) = (f32)(f64(a) + f64(m)·f64(b − a))`.

```
u   = fma(ι, y₂, y₃ − y₄)
v₁  = b₂·u
A₁  = S_A(v₁)
B₁  = S_B(fma(−k, A₁, x))
B₂  = S_B(fma(k, M(y₁ − y₂, y₃ − y₄), xₚ))
v₂  = b₁·fma(ι, y₁, (B₁ + B₂) − y₃)
A₂  = S_A(v₂)
v₃  = b₂·fma(ι, y₂, (y₃ + A₂) − y₄)
A₃  = S_A(v₃)
B₃  = S_B(fma(k, M(v₂ − v₃, A₂ − A₃), x))
y₁' = fma(g, ((B₃ + B₂) − y₃) − A₂, y₁)
y₃' = S_A(y₁')
y₂' = fma(g, ((y₃ + y₃') − y₄) − A₃, y₂)
y₄' = S_A(y₂')
xₚ' = x
```

The left channel runs first, then the right with its own `g, b₁, b₂, ι`. Both share `k`.

**Outputs** (after the core):

- **LP:** `y₂`.
- **BP:** `(y₃ − y₄)·1.5`.
- **HP:** `B = S_B(fma(k, M(y₁ − y₂, y₃ − y₄), xₚ))` on the new states, `u = B − y₃`, then a
  one-pole stage with state `z`:
  `o = u + c·fma(−u, h, −2z)`, `z' = fma(o, h, z)`; the output is `o`.

## 5. Null tests

Maschine 3 rendered headless (`maschine-ref make … --perform 0 field=value…`, `render
--seconds 2`, 1 s preroll; `out/perform-ref-ready.md`), DECKARD through `fx-null` with
`FX_PREROLL=1`. Script: `out/perform-filter/null.sh` (scratch). Engaged unless noted; other
parameters at Maschine's defaults; input −12 dBFS unless noted. Residual is L/R RMS at lag 0;
"−240" is bit-exact but for the noise (§2.1), whose peak was below 3e-29 in every case.

| test | settings | input | sr | residual |
|---|---|---|---|---|
| off | released, Resonance 1 | noise | 48k | −240 (Maschine's output is the input 16 samples late) |
| lp / bp / hp | Type 0 / 1 / 2 | noise | 48k | −240 each |
| lp44 | defaults | noise | 44.1k | −240 |
| pos0 | strip 0 | noise | 48k | −240 |
| sat1 | Saturation 1, Out Level 0.2 | noise | 44.1k | −240 |
| minmax | Min. Freq 1, Max. Freq 0, strip 0.7, BP | sweep | 48k | −240 |
| out1 | Out Level 1, Saturation 0 | sine 440 | 48k | −240 |
| r88 | defaults (no oversampling) | sweep | 88.2k | −240 |
| r96lp | strip 0.3, Resonance 0.5 | noise | 96k | −240 |
| hfres / hfres2 | strip 0.95 / 1, Resonance 0.8 / 0.75 (the > 16 kHz limit) | noise / sweep | 48k / 44.1k | −240 |
| res7 / res7hp | Resonance 0.7, LP / HP | sweep / noise | 48k / 44.1k | −240 |
| sat9 | Saturation 0.9, Resonance 0.6, input 0 dBFS | sweep | 48k | −240 |
| loudhp2 | HP, Saturation 0.8, Resonance 0.5, input +6 dBFS | sweep | 44.1k | −240 |

**Self-oscillating** (Resonance 1, strip 0.4–0.95; also 96 kHz HP at 0.9, and +12 dBFS with
Saturation 1): the filter rings by itself, grown from the noise during the preroll, so its
phase depends on the seed. Maschine against itself (the same project rendered twice) differs
by −6 to −25 dB; DECKARD against Maschine likewise. Level and pitch agree: e.g. Resonance 1,
strip 0.4: −6.8 dB RMS, 269.5 Hz in both; impulse at strip 0.5: −6.3 dB, 564 Hz; HP at strip
0.95: −11.4 dB, 14.4 kHz.

Not tested against Maschine: changes mid-stream (engaging, releasing, moving the strip, a
type change), since the renders hold their parameters; the glides (§3) and the type fade
are from the code, and covered by DECKARD's own tests.

## 6. Open points

1. The order the processor applies a simultaneous strip touch (position and engage) in
   (§3.2): DECKARD applies the parameters in index order. **[inferred]**
2. The noise seed (§2.1): Maschine's varies by run; DECKARD's is fixed.
3. The processor's handling of a released effect: Maschine runs Filter whether engaged or
   not (`PerformanceFXProcessor::processAudio` calls the current effect each 128-sample
   block), so released it delays the signal 16 samples. Two shared DECKARD tests assume a
   released Perform FX is transparent (out/perform-ref-requests.md).
