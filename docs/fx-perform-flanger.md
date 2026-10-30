# Maschine's Perform FX Flanger: parameters and DSP

A clean-room description of Maschine 3's Perform FX **Flanger** (mode 1 of
`NI::MASCHINE::DATA::PerformanceFX`), run by `NI::MASCHINE::RT::PerformanceFXProcessor`
as `NI::DSP::JamFlanger` around a `NI::DSP::CombFilter<double>`. DECKARD's version is
`crates/engine/src/fx/perform/flanger.rs`; it nulls bit-exactly against Maschine (§7).

Written from Ghidra output in `out/decomp/perform-flanger/` and
`out/decomp/perform-flanger-comb/` (`INDEX.tsv` maps files to functions) and arm64
disassembly in `out/decomp/perform-flanger-asm/` (`process.s` the per-sample loop,
`comb.s` the comb, `setparam.s`, `pitch.s`, `decay.s`, `srate.s`, `combsr.s`,
`recalc.s`, `allpass.s`, `init.s`). Where the two disagree, this follows the assembly.
None of these files are committed.

**Confidence.** **[exact]**: read from code or assembly and confirmed by the null tests.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation: `f32(·)`/`f64(·)` are conversions; arithmetic is `f32` unless marked. `fma(a,b,c)`
is a fused `a·b + c`. `exp10`, `exp2`, `tanh` are Apple's libm (double).
`K12 = 0x3fb5555555555555` (1/12), `NOTE0 = 0x400840397fa881bf` (log₂ 8.1758 Hz, MIDI
note 0), `C3 = 0x3fd555555555554f`, `C5 = 0x3fc1111111111111`.

---

## 1. Parameters

`DATA::PerformanceFX::connectParameters` routes each stored field to the core's
`setParameter(index, value)`; enums arrive as floats **[exact]**.

| index | field | core value |
|---|---|---|
| 0 | `touchstrip_pos` (through `setTouchstripPosition`) | §2 pitch |
| 1 | `touchstrip_on_off` (through `setTouchstripOnOff`, 0/1) | engage target `v > 0.5 ? 1 : 0` |
| 2 | `flanger_decay` | §3 |
| 3 | `flanger_polarity` (0 Negative, 1 Positive) | sign target `v > 0.5 ? +1 : −1` (double); high-pass `v > 0.5 ? 90 : 60` Hz, recomputed at once |
| 4 | `flanger_tone` (0 Flanger, 1 Phaser) | all-pass count 0 or 16, applied at the next block (§5) |
| 5 | `flanger_stereo` | stereo target `(|v| + 0)·v` |
| 6 | `flanger_saturation` | saturation target `f32(fma(f64(clamp(v,0,1)), 24, −12))` dB |
| 7 | `flanger_wet_level` | wet target `v + v` |

The core keeps the last value of each index (`getParameter`). The connect order is
strip, engage, Polarity, Decay, Stereo, Tone, Saturation, Wet Level **[exact]**.

**Glides [exact].** Every target is a linear glide. It holds `value`, `target`, `step`,
`steps` (float), `count` (int, `round(steps)`, half away from zero), `left`, `moving`.
- **Set:** `target = t`; if `value ≠ t`, then `step = (t − value)/steps`, `left = count`,
  `moving = true`.
- **Each sample:** if `|target − value| ≤ 0` or (`left`, before `left −= 1`) `< 2`, then
  it lands (if moving: `value = target`, `step = 0`, `left = 0`, not moving). Otherwise
  `value += step`.

Glide times: Stereo, ratio, Wet and Saturation 40 ms (`sr·0.001f·40`); engage 10 ms;
delay target and all-pass frequency 15 ms; Decay `(sr/4)·0.001f·50` (it moves only every
4th sample, §4); fade-in `sr·0.001f·0.1f` (4 samples at 44.1 kHz). The sign glides in
double over `f64(sr)·0.001·20` (20 ms).

**Suspended [exact].** While the core is suspended (from construction, while another mode
plays), every parameter setter lands all float glides at once (not the sign or the
fade). The processor suspends a mode it leaves after the crossfade, and on switching to
one it calls strip, engage, `onReset` and `onResume`.

## 2. The strip (`setPitch`) [exact]

With `x = touchstrip_pos`:
- `pitch = f32(fma(f64(clamp(x,0,1)), 125, 10))`: semitones over MIDI note 0, 10–135.
- `bent = clamp((|x|·0.5 + 0.5)·x, 0, 1)` (float fma);
  `tone = f32(fma(f64(bent), 95, 10))`.
- `hz = f32(exp2(fma(f64(pitch), K12, NOTE0)))`; delay target `= 1/hz` seconds
  (13.7 ms at the strip's bottom, 0.4 ms at its top).
- All-pass frequency target `= f32(exp2(fma(f64(tone), K12, NOTE0)))`.
- Ratio target `= f32(fma(f64(t), f64(0.19999993f), f64(1.1f)))`, where
  `t = clamp((pitch − 10)/115, 0, 1)`. The right side's delay is up to this much longer.
- Decay (§3) is recomputed with the new pitch, and the wet target is set again from
  index 7.

## 3. Decay [exact]

```
lo    = clamp((pitch − 20)/110, 0, 1);  hi = clamp((pitch − 10)/120, 0, 1)
top   = f32(fma(f64(hi), −6, 3));  floor = f32(fma(f64(lo), −33, 53))
db    = f32(fma(f64(clamp(d,0,1)), f64(75 − floor), f64(floor)))
secs  = f32(exp10(f64(top + db)·0.05) · 0.001)
target = clamp(d/0.2f, 0, 1) · secs
```

This is the time (seconds) the feedback takes to fall 60 dB. It is 0 at Decay 0, and below
20 % it scales down to 0.

## 4. Per sample (`processAudio`) [exact]

At a block's start, a pending Tone change sets the comb's all-pass count (`tone·16`;
see §5) and restarts the fade-in (`value 0 → 1`).

Then, for each sample:
1. Glides step in this order: stereo, ratio, engage, saturation, wet, delay target,
   all-pass frequency. On control samples (`tick == 0`) Decay steps too, and
   `decay = its value`; otherwise `decay` is the last one used.
2. **The delay's 20 Hz smoothing (double, trapezoidal one-pole):**
   `g, G = warp(f64(1/sr)·0x404f6a7a2955385e)` (20π).
   `v = x + G·fma(−x, g, −2s)`, `y = 2s + g·v`, `s = fma(v, g, s)`, `delay = f32(y)`.
   After a reset, `s = delay target·0.5`, which is already settled.
3. `tick = (tick + 1) & 3`.
4. If the delay or the stereo changed since the comb was last set, or after a reset:
   - `right = delay · f32(fma(f64(stereo), f64(ratio − 1), 1))`.
   - The comb's all-pass frequency is set and its gains computed (§5).
   - For each side, `at = sr·delay`, `D = trunc(at)`, `frac = f64(at − f32(D))`,
     and the tap coefficient is `(1 − frac)/(frac + 1)`.
5. If `decay` changed, or after a reset, or (the delay changed and `tick == 0`), then
   `fb = f32(exp10(f64(delay)·−3 / f64(decay + 1e−10f)))` on both sides. Both sides use
   the left delay.
6. `drive = f32(exp10(f64(saturation)·0.05))`, with the comb un-driving by `1/f64(drive)`.
7. `gate = fade · engage`. The comb takes `f64(gate·in)` per side (§5).
8. `level = f64(f32(exp10(f64(saturation·0.6f)·0.05))·0.4f)`;
   `out = fma(wet, f32(comb·level), in)` (float fma), using the **raw** input.

Released, the input passes untouched plus whatever the comb still holds. Engaging and
releasing glide over 10 ms; there is no other crossfade.

## 5. The comb (`CombFilter<double>`) [exact]

A delay line of `next_pow2(trunc(4·sr))` doubles per side, with a write index.

Per sample:
1. The sign glide steps to `p`, and `x = fb_state·p + in`.
2. **Saturator:** `x = tanh(drive·x)·(1/drive)`.
3. **High-pass** (one-pole at 60/90 Hz, by Polarity): `(v, lp) = one_pole(x)`, and
   `x = v`.
4. **All-passes** (0 in Flanger tone, 16 in Phaser): each gives `x = lp − v`.
5. The line is written at `w`, then `w = (w + 1) & mask`.
6. **Tap:** `s = f32(line[w − D])`, `t = f32(c·(f64(s) − prev))`.
   - If `D < D_last`, then `out = line[w − D_last]`.
   - Otherwise `out = prev + f64(t)`.
   - Then `prev = f64(s + t)`, `D_last = D`, and `out = f32(out)`.
7. The output goes back to the caller, and `fb_state = fb·f64(out)`.

`one_pole(x)`, trapezoidal with state `s`, gains `g` and `G = 1/(g + 1)`:
`v = x + G·fma(−x, g, −2s)`, `lp = 2s + g·v`, `s = fma(v, g, s)`. Here `v` is the
high-pass and `lp − v` the all-pass.

`warp(w)`, tan by series: `w3 = w·w²`, `g = fma(w3, C3, w)`, `g = fma(w²·w3, C5, g)`,
`G = 1/(g + 1)`. A stage at `f` Hz uses `w = f64(f)·π·f64(1/sr)`.

**The all-passes' frequencies.** These use `spread = 48`.
- `share = n ? 1/n : 1`.
- `spacing = f32(exp2(f64(share·(spread + spread))·K12))`.
- `lowest = f32(exp2(f64(−spread)·K12))`.
- `f = frequency·lowest`, then `f ·= spacing` after each stage. In Phaser that is
  `f/16` upward in half octaves (f32 products).

**Changing the count** clears the all-pass, high-pass and tap states, the feedback
and the lines. It lands the sign glide and recomputes the gains. The same count is a
no-op.

## 6. Reset and start [exact]

`onSetSampleRate` (only while suspended, and only at 44.1 kHz or above) sets the glide
times, sizes the lines and computes the 20 Hz gains. It then calls `onReset`, which:
- lands the glides (the sign too);
- clears the comb;
- sets the delay smoothing state to `delay target·0.5`;
- sets the last delay to 1 and the last decay to 0;
- sets `tick` to 0;
- forces a recompute on the next sample;
- starts the fade-in.

Construction leaves Tone pending, so the first block sets the count and restarts the
fade-in.

Initial values: all-pass count 0, high-pass 10 Hz until Polarity arrives, tap coefficient
0.5 with `D = D_last = 1`, pitch 1, delay 1 ms.

## 7. Null tests

Source: `out/perform-flanger/render.sh` (each case a `maschine-ref make … --perform 1`
project, rendered for 2 s after a 1 s preroll), compared by the
`nulls_against_maschine` test in `flanger.rs`. Spot checks went through the whole
`PerformFx` with `fx-null`. The residual is L/R RMS.

| case | settings | input | sr | residual |
|---|---|---|---|---|
| def | defaults (strip 0.5, Decay 0.65, Stereo 1, Flanger, Sat 0.7, Wet 0.45, Negative) | noise | 44.1k | −240 dB (bit-exact) |
| phaser | Tone Phaser | noise | 44.1k | −240 |
| pos0 / pos1 / pos03 | strip 0 / 1 / 0.3 | noise / noise / sweep | 44.1k | −240 each |
| posit | Positive | noise | 44.1k | −240 |
| dec0 / dec1 / dec01 | Decay 0 / 1 (strip 0.8) / 0.1 | noise | 44.1k | −240 each |
| ster0 | Stereo 0, Wet 1 | noise | 44.1k | −240 |
| sat0 / sat1 | Saturation 0 / 1 (Wet 1) | noise | 44.1k | −240 each |
| off | released | noise | 44.1k | −240 |
| r48 | strip 0.65, Phaser, Positive | noise | 48k | −240 |
| imp | strip 0.2, Decay 0.9 | impulse | 44.1k | −240 |

Not tested against Maschine: changes mid-stream (strip movement, engage/release
glides, a Tone change), because the renders hold one setting. Those follow the code
only.

## 8. Rulings and open points

- **A Tone change** applies at DECKARD's next sample, not at Maschine's next 128-sample
  block **[inferred]**. If that's wrong, the all-pass count and fade-in switch up to
  127 samples earlier.
- **Mode switching** is the processor's (`perform/mod.rs`). Maschine suspends the Flanger
  while another mode plays and resets it on return. DECKARD's Flanger has no such hook:
  it lands parameter glides only before its first sample, and isn't reset on return.
  This is requested in `out/perform-ref-requests.md`.
- **Libm:** on platforms other than macOS, `exp10` is `10^x` by `powf`, so the results
  may differ in the last bits there.
