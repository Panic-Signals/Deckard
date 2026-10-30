# Maschine's Cabinet: parameters and DSP

A clean-room description of Maschine 3's built-in **Cabinet** module, class
`NI::MASCHINE::DATA::CabinetEmulation` (mxprj module name "CabinetEmulation", display name
"Cabinet") → `NI::MASCHINE::RT::DSPCoreWrapper<NI::DSP::Cabinet>` → `NI::DSP::Convolver`
(three of them) + `NI::DSP::Air` + two `NI::AB::SampleRateConverter`s.

The DECKARD port is `crates/engine/src/fx/cabinet.rs`. At 44.1, 48, 88.2, 96, 32, 22.05 and
192 kHz it nulls against Maschine to −135…−150 dB below the signal (§6). It is not bit-exact
only because Maschine convolves with a partitioned FFT and DECKARD directly (§3.2).

Sources: Ghidra output in `out/decomp/cabinet/` (data class, wrapper, `Cabinet`, `Convolver`;
`INDEX.tsv` maps files to functions), `out/decomp/cabinet-air.s` (all of `DSP::Air`,
hand-read), `out/decomp/cabinet-src-*.s` (`SampleRateConverter::init_generic`,
`process_generic`, `convert`) and the saturator's `out/decomp/saturator-tape/` (the same
converter, decompiled). Addresses are arm64 VM addresses in `analysis/maschine_arm64`.

**Confidence.** **[exact]**: read from code or assembly (and, where §6 says so, confirmed by
null tests). **[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation: `v` is a stored value; arithmetic is `f32` unless marked `f64`; `fma(a,b,c) = a·b + c`
with one rounding; `sr` the host sample rate.

---

## 1. Parameters

### 1.1 Module

`Module(L"Cabinet")`, `s_aModuleInfo` id 0xf, one page: `s_aPageNames = { "Main" }`
(`__GLOBAL__sub_I_Modules_Effects_Cabinet_ALL.cpp`, 0x101a7c600) **[exact]**.

### 1.2 Fields in constructor order

`DATA::CabinetEmulation::CabinetEmulation()` at 0x101a78334 (`0043.c`). Every field is
serialized in this order with no version branches; renamed in `module.rs` from
`unknown_a8 … unknown_c0` **[exact]**:

| obj off | mxprj field | label | section | default | range | RT | display |
|---|---|---|---|---|---|---|---|
| 0xa8 | `cabinet` | **Cabinet** (enum) | "Cabinet" | 0 (Am. 2x12) | 4 values (§1.3) | `processCabinetParameter` (data side) | value string |
| 0xb0 | `microphone` | **Microphone** (enum) | "Microphone" | 0 (Dynamic 57) | 6 values (§1.3) | `processMicrophoneParameter` (data side) | value string |
| 0xb8 | `distance` | **Distance** | – | 0 | 0..1 | idx 2 | `ScaleNormalizedToPercentOneDecimal` (`"35.0 %"`) |
| 0xc0 | `mix` | **Mix** | "Output" | 1 | 0..1 | idx 4 | `ScaleNormalizedToPercentOneDecimal` |

Both enums call `setKKSAlwaysShowValue(true)` (secondary vtable +0xe0 of `EnumParameter`).
Distance and Mix get `processWheelDeltaSnapToZero` and a wheel factor of 2.0 (+0x1d0), like
other percent knobs. The constructor then calls `Cabinet::setParameter` with `(0, 0)`,
`(1, 0)`, `(2, 0)`, `(3, 1/3)`, `(4, 0)`, `(5, 0.25)`: indices 3 (Air size) and 5 (wet gain)
are never wired to a field, so they stay **fixed** at 1/3 and 0.25 **[exact]**.

### 1.3 Selector values

From the static initializer (wide strings at 0x104d35e30…; long, short) **[exact]**:

| Cabinet idx | long | short | IR set (`s_IRs` index) |
|---|---|---|---|
| 0 | `Am. 2x12` | `A 2x12` | Fender (1) |
| 1 | `Brit. 2x12A` | `B 2x12` | Vox (3) |
| 2 | `Vin. 4x12A` | `V 4x12` | Marshall (2) |
| 3 | `Mod. 4x12` | `M 4x12` | Engl (0) |

| Microphone idx | long | short |
|---|---|---|
| 0 | `Dynamic 57` | `Dy 57` |
| 1 | `Dynamic 421` | `Dy 421` |
| 2 | `Dynamic 441` | `Dy 441` |
| 3 | `Ribbon 121` | `Rib 121` |
| 4 | `Condenser 47` | `Con 47` |
| 5 | `Condenser 67` | `Con 67` |

(The "A" in "Brit. 2x12A" / "Vin. 4x12A" is in the binary's strings.)

### 1.4 Knob layout

No `getParameterByPosition` override: page "Main" holds the fields in order **[inferred
from the absence of an override; matches the editor]**:

| slot | label | field | section heading |
|---|---|---|---|
| 0 | CABINET | `cabinet` (enum, 4) | "Cabinet" |
| 1 | MICROPHONE | `microphone` (enum, 6) | "Microphone" |
| 2 | DISTANCE | `distance` 0..1 | – |
| 3 | MIX | `mix` 0..1 | "Output" |
| 4–7 | – | – | – |

---

## 2. Choosing the impulse response

### 2.1 The impulse responses

67 mono impulse responses, 2048 `f32` samples each at 44.1 kHz, are compiled into the
binary as `NI::DSP::CabinetIRs::<name>` arrays (0x10c3599a8…, 0x2000 bytes apart). A static
initializer (`__GLOBAL__sub_I_unity_0_cxx.cxx`, 0x102668250) copies pointer lists from
`__DATA_CONST` into five `std::vector<const float*>`s, and `s_IRs` (0x10d3e4e40) points to
them: `{ EnglIRs, FenderIRs, MarshalIRs, VoxIRs, MesaBoogieIRs }`. The vector orders are in
`cabinet.rs` (`SETS`) **[exact]**. The MesaBoogie set (8 IRs) is unreachable.

**These are NI recordings and DECKARD does not ship them.** `fx::cabinet::bank()` reads them
at run time from the user's install: the first readable of `$MASCHINE_BINARY`,
`/Applications/Native Instruments/Maschine 3/Maschine 3.app/Contents/MacOS/Maschine 3`,
`/Library/Audio/Plug-Ins/VST3/Maschine 3.vst3/Contents/MacOS/Maschine 3`. It takes the arm64
(else x86_64) slice of the fat Mach-O, finds the `__ZN2NI3DSP10CabinetIRs<len><name>E`
symbols in `LC_SYMTAB`, maps each address through its `LC_SEGMENT_64` and reads 8192 bytes.
Without a readable binary the wet path is silent (the dry part of Mix still passes).

### 2.2 Selector → IR (`processCabinetParameter` 0x101a78aa8, `processMicrophoneParameter` 0x101a78b1c) **[exact]**

The data object writes straight into the DSP object (`Cabinet+0x8`, `+0xc`, `+0x24`, `+0x28`)
and calls `setImpulseResponse()` itself; the RT `setParameter(0/1)` cases do the same.

- **Cabinet** (`v = idx/3`): `x = v < 1/3 ? 0.4 : v < 2/3 ? 0.8 : v < 1 ? 0.6 : 0.2`;
  `set = (u32)(x·4)` (1, 3, 2, 0). If the set changed, the variant is recomputed from the
  **stored** microphone fraction: `variant = (int)(frac·(float)(count − 1))`.
- **Microphone** (`w = idx/5`): `frac` from the row of the *current* Cabinet value, by
  `w < 0.2, < 0.4, < 0.6, < 0.8, < 1, ≥ 1`; then `variant = (int)((float)(count − 1)·frac)`.

| Cabinet | fractions (mic 0…5) | IR picked (mic 0…5) |
|---|---|---|
| Am. 2x12 (Fender, 7) | 1, 1/6, 1/3, 0.5, 2/3, 5/6 | U89, MD421, MD441, Royer, U47, U67 |
| Brit. 2x12A (Vox, 17) | 0.5, 0.0625, 0.1875, 0.375, 0.6875, 0.9375 | SM57_API, MD421_GML, MD441_API, Royer_API, U47_API, U87_GML |
| Vin. 4x12A (Marshall, 17) | 0.625, 0.125, 0.1875, 0.5, 0.75, 0.875 | SM57full_Neve, MD421_GML, MD441_Neve, Royer_Neve, U47_none, U67_Neve |
| Mod. 4x12 (Engl, 18) | 10/17, 2/17, 4/17, 8/17, 14/17, 16/17 (as `f32`) | SM57full_GML, MD421_GML, MD441_GML, Royer_GML, U47_none, U67_GML |

(Am. 2x12 has no SM57: "Dynamic 57" gives its U89.) Because a Cabinet change keeps the old
fraction, changing only the Cabinet after the Microphone can land on a different mic than
choosing them in the other order; DECKARD keeps that state the same way. On load the
fields are applied Cabinet first, then Microphone (all 24 combinations null, §6).

### 2.3 Switching (`setImpulseResponse` 0x10263af14, `processControlRate` 0x10263cef8) **[exact]**

Three convolvers rotate: *current* (`+0x2d80`), *fading-in* (`+0x2d88`), *spare* (`+0x2d90`).
`setImpulseResponse` (under a mutex) resets the spare, loads the IR into it (both channels
the same IR) and sets "ready" (`+0x22`). At the next control tick, if ready and no fade is
running, spare and fading-in swap and a crossfade starts: gains (old, new) from (1, 0) to
(0, 1) as a linear ramp over `⌊882/P + ½⌋` ticks of `P = (int)(5644800/sr)` samples (≈ 20 ms;
at 44.1 kHz 7 × 128). The new convolver starts from silence. When the ramp's step is 0
again, gains reset to (1, 0) and the new convolver becomes current. While suspended
(`+0x21`, i.e. at load) the switch is immediate. Per sample during a fade:
`y = fma(old, g_old, new·g_new)`.

---

## 3. DSP

### 3.1 Structure (`processAudio` 0x10263d8b0) **[exact]**

`DSPCoreWrapper::process` (0x101a79e08) calls `processControlRate` every 128 samples and
`processAudio` on the stretches between (≤ 128 samples). Per stretch:

1. copy the input (dry);
2. at 44.1 kHz: convolve in place (with the crossfade of §2.3); otherwise resample to 44.1
   kHz, convolve, resample back (§4);
3. `Air::processAudio` in place (§3.3);
4. per sample: Mix ramp step, `out = fma(wet, g_wet, dry·g_dry)`, `g_wet = mix·0.25`,
   `g_dry = 1 − mix` (`setParameter(4/5)`: `+0x3290 = mix·wet_gain`, `+0x3294 = 1 − mix`).

So at Mix 100 % the output is the cabinet alone, 12 dB down from a unit-gain IR.

### 3.2 Convolution

`NI::DSP::Convolver` is a zero-latency non-uniformly partitioned FFT convolver
(`PartitionArray`, split-radix FFTs, ~3700 lines decompiled) with per-partition scheduling
and its own gain ramps. It was **not ported**: its result equals the linear convolution
`y[n] = Σ h[k]·x[n−k]` (k < 2048, no latency), and DECKARD computes that directly with `f64`
sums, so the residual is Maschine's FFT rounding, ≈ −135 dB relative (§6). The impulse test
confirms 0 latency and the gain: the output is `0.25·h` to −148 dB.

### 3.3 `NI::DSP::Air` (Distance; `cabinet-air.s`, 0x1026397a4…0x10263a0f0) **[exact]**

Inlined into `Cabinet::setParameter`: idx 2 stores `amount = v` (`Air+0x8`) and ramp targets
`0.7·v` on lanes 0/2; idx 3 (fixed 1/3) sets the size `s = fma(v, 0.03, 0.005) = 0.015 s` and
tap delays `t₀ = (int)(sr·s)`, `t_{k+1}` from `sr·s·1.17^k` multiplied up in `f32` (8 taps;
re-evaluated per sample only while `s` ramps).

`onSetSampleRate`: line length `2^(⌊ln(sr·0.10504)/ln 2⌋ + 1)` (8192 at 44.1/48 kHz), mask
`len − 1`; one-pole coefficients `c1 = min(1, 2π'/sr · 261.63)`, `c2 = min(1, 2π'/sr ·
1108.73)` with `2π' = 6.282 (0x40c90e56)`; ramp ticks `max(1, ⌊sr·0.02/128 + ½⌋)`.

Per sample (after the ramp step; skipped, output = input, while `amount ≤ 0`, the raw
unsmoothed value):

```
A = [a[t0], a[t2], a[t4], a[t7]]·g,  B = [b[t1], b[t3], b[t5], b[t7]]·g   (index (t + w) & mask)
g = [1, −0.7305, 0.5336, −0.3898]                       (0x3f800000 bf3b02f0 3f089d4b bec798ee)
SA = (A0 + A2) + (A1 + A3);  SB = (B0 + B2) + (B1 + B3)
fb = 0.16·(SA + SB)
for (x, s1, s2) in [(L + fb, …), (R + fb, …)]:
    d = x − s1;  s1 += c1·d;  e = d − s2;  s2 += c2·e      (mul then add, no fma)
w = (w − 1) & mask;  a[w] = s2_L;  b[w] = s2_R
L' = fma(SA, ramp0, L);  R' = fma(SB, ramp2, R)
```

(The SIMD code also runs the two filters on the plain `L`, `R` lanes; those results are never
read.) Tap 6 is never used. Distance only adds the reflections: at 0 the wet path is the
convolution alone.

### 3.4 Ramps (`SmoothingToolbox`) **[exact]**

All ramps are 4-lane linear: at a tick with a new target, `step = (target − cur)·inv`, lanes
with `(f64)(step²) < 1e-15` snap; if any lane moves, a countdown of `ticks` starts; at its end
`cur = target`. `cur += step` per sample. Mix: `max(1, ⌊sr·0.02/128 + ½⌋)` ticks, `inv =
1/(ticks·128)`. Air: same. While suspended (initial load) targets apply immediately. Order in
`processControlRate`: Air tick, IR swap check, crossfade tick, Mix tick, fade-end check.

---

## 4. Other sample rates

`onSetSampleRate` (0x10263a92c): if `sr ≠ 44100` the convolvers run at 44.1 kHz between two
`NI::AB::SampleRateConverter`s, `init(sr → 44100)` and `init(44100 → sr)`, 2 channels,
quality 2. Per stretch of `n` host samples, the inner count is (`fcvtpu`, `f64`):

`m = ⌈((n − pending + 1)·out − phase) / fma(step, out, rem)⌉`

then `convert(dry, n → buf, m)`, convolve `m` samples, `convert(buf, m → out, n)`.

### 4.1 Generic converter (any non-integer ratio: 48, 96, 32, 192 kHz …) **[exact]**

`init`: `r = out/in`; for `out ≤ in`: `step = ⌊in/out⌋` by repeated subtraction, `rem = in −
step·out`; else `step = 0`, `rem = in`. Kernel: `InterpolCoefficients(2)`, 8 points, four
degree-5 segments in `|u|` (constants in `cabinet.rs` `KERNEL`; `K(0) = 1`, `K(1…3) ≈ 0`).

`init_generic` (0x102fd8b3c): taps `L = 8` (up) or `⌈8·in/out⌉` (down, `f64`); `dA, dB, g =
2, 2, 1` (up) or `2·out, 2·in, out/in` (down); `base = dB·8·(−½)`. For tap `i` (stored at
`L−1−i`): `t = fma(i, dA, base)`, `q = |t|/dB`, `seg = ⌊q⌋` (minus 1 if exactly on a boundary
and `t < 0`), boundary `= dist/dA` with `dist` the distance to the next segment edge in the
direction of motion; `u0 = (f32)q`, `σ = ±g` by the sign of `t`, `flip = −1` if the tap crosses
0 within the period. Per power `m = 0…5`: `lo[m] = g·σ^m·Σ_{j≥m} C(j,m)·c[seg][j]·u0^{j−m}`
(Horner `v = fma(c, C, round(v·u0))`, then `m` multiplications by σ), and `hi[m]` likewise
for the next segment with `flip·u0` and `σ·flip`; segments ≥ 4 give 0.

`process_generic` (0x102fda2f4): per output, `x = phase·(1/out)`; per group of 4 taps, lane
mask `x ≥ boundary`, `acc = sel(hi₅, lo₅)`, then `acc = acc·x + sel(hi_m, lo_m)` for m = 4…0
(mul then add). Output = Σ over groups of `hist·acc` (4 lanes, mul then add), lanes summed
`(l0 + l2) + (l1 + l3)`. Then `phase += rem`; if `phase > out`: `phase −= out`, take `step+1`
inputs, else `step`. The history (newest last, `L` long) shifts in; inputs owed when a call
runs out are taken first in the next call (`pending`, initially 1; `phase` initially 0).

### 4.2 ×2 / ×½ (88.2 and 22.05 kHz) **[exact]**

Ratios 2 and ½ use `process_intratDOUBLE` / `process_intratHALF` (types 5/6), already ported
for the Saturator's Tape mode (`saturator/tape.rs`, docs/fx-saturator.md); `cabinet.rs`
carries a copy (`src_coefs`, `fir_step`, `Up2`, `Down2`). 88.2 kHz: half then double
(`m = n/2`); 22.05 kHz: double then half (`m = 2n`). Odd stretch lengths (odd host blocks)
are not modelled.

### 4.3 Not ported

Ratios 4 and ¼ (176.4, 11.025 kHz) use `process_intratUP/DOWN` (types 3/4): DECKARD uses the
generic converter instead, which does not null (different filter and latency) **[unknown]**.

---

## 5. Implementation notes

- `Cabinet::new` loads the IR bank once (`OnceLock`); the first `set()` is treated as the
  suspended load (immediate IR, Mix and Distance), later ones ramp/crossfade as above.
- Control ticks follow the wrapper's 128-sample counter across host blocks.
- Cost: direct convolution of 2 × 2048 taps per (44.1 kHz) sample, ≈ 7 % of one core per
  instance at 48 kHz (4 s of audio in 0.3 s). An FFT convolver would be cheaper but would not
  change the null-test result.

---

## 6. Null tests

Source project: `/Users/Shared/Maschine Central Library/Projects/Good Fall.mxprj`, sound 0/0
slot 1 (Cabinet). Each case: `maschine-ref make … 0 0 1 field=value…`, rendered with
`--preroll 0`, run through `fx-null` on the captured input (script `run.sh` in the session
scratch directory `…/scratchpad/fx-cabinet/`). Noise/sweep at −12 dBFS. "rel" = residual RMS
relative to the output RMS; −240 dB absolute means bit-identical.

| case | input | sr | parameters | output | residual (abs) | rel |
|---|---|---|---|---|---|---|
| defaults-like | impulse | 44.1k | cab 0, mic 0, dist 0, mix 1 | −56.1 | −204.5 | −148 |
| all 24 cab × mic | noise 1 s | 44.1k | dist 0, mix 1 | −18.6…−21.6 | −153.6…−157.6 | −135…−136 |
| all 24 cab × mic | noise 1 s | 48k | dist 0, mix 1 | −19.0…−22.1 | −154.3…−158.2 | −135…−136 |
| distance mid | noise 2 s | 44.1k | cab 3, mic 2, dist 0.5, mix 1 | −19.8 | −155.2 | −135 |
| distance max | noise 2 s | 44.1k | cab 3, mic 2, dist 1, mix 1 | −18.8 | −154.4 | −136 |
| distance + mix | noise 2 s | 44.1k | dist 0.25, mix 0.5 | −19.8 | −160.8 | −141 |
| mix 0 | noise 2 s | 44.1k / 48k | dist 0 or 1, mix 0 | −16.8 | **−240** | – |
| mix 0.3 | noise 2 s | 44.1k | dist 0, mix 0.3 | −19.5 | −164.3 | −145 |
| tiny distance | noise 2 s | 44.1k | dist 0.01, mix 0.77 | −21.7 | −157.7 | −136 |
| distance, impulse | impulse | 44.1k | cab 1, mic 1, dist 1, mix 1 | −62.0 | −208.8 | −147 |
| distance mid | noise 2 s | 48k | cab 1, mic 4, dist 0.5, mix 1 | −20.9 | −156.9 | −136 |
| distance max | noise 2 s | 48k | dist 1, mix 1 | −20.0 | −156.0 | −136 |
| distance + mix | noise 2 s | 48k | dist 0.25, mix 0.5 | −21.4 | −162.4 | −141 |
| mix 0.3 / 0.77 | noise 2 s | 48k | | −19.6 / −22.5 | −165.6 / −159.4 | −146 / −137 |
| impulse | impulse | 48k | cab 0, mic 0, dist 0, mix 1 | −63.9 | −206.2 | −142 |
| sweep | sweep 4 s | 48k | cab 3, mic 5, dist 0.4, mix 0.9 | −11.0 | −144.6 | −134 |
| with 1 s preroll | noise 2 s | 44.1k / 48k | cab 1, mic 4, dist 0.01, mix 0.77 | −22.3 / −22.5 | −158.8 / −159.4 | −137 |
| 96 kHz | noise 1 s | 96k | cab 2, mic 3, dist 0.6, mix 0.8 | −24.9 | −161.4 | −137 |
| 88.2 kHz | noise 1 s | 88.2k | same | −24.5 | −160.6 | −136 |
| 32 kHz | noise 1 s | 32k | same | −20.6 | −157.4 | −137 |
| 22.05 kHz | noise 1 s | 22.05k | same | −19.2 | −156.7 | −137 |
| 192 kHz | noise 1 s | 192k | same | −26.9 | −164.3 | −137 |
| 176.4 kHz | noise 1 s | 176.4k | same | −26.7 | −27.0 | ≈ 0 (§4.3) |

Not null-tested (maschine-ref cannot change parameters mid-render): the IR crossfade and the
Distance/Mix ramps after a change; they follow the code of §2.3/§3.4.

---

## 7. Open points

1. `Convolver` not ported (§3.2): residual ≈ −135 dB instead of bit-exact.
2. `intratUP/DOWN` (×4, ¼) not ported (§4.3).
3. Odd host block lengths at 88.2/22.05 kHz (§4.2).
4. Knob layout inferred from field order (§1.4).
