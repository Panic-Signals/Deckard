# Maschine's Filter: parameters and DSP

A clean-room description of Maschine 3's built-in **Filter** effect
(`NI::MASCHINE::DATA::Filter` → `NI::MASCHINE::RT::FilterProcessor` →
`NI::MASCHINE::FXCORE::FilterCore`, with `NI::MASCOMMON::LEGACY::SyncableLFOHelper` for the
LFO rate and `NI::MASCHINE::FXCORE::SidechainFilterCore` on the side-chain input).
DECKARD's port is `crates/engine/src/fx/filter.rs`; it nulls bit-exactly against Maschine
(§7).

It was written from Ghidra output in `out/decomp/filter/` (`INDEX.tsv` maps files to
functions) and arm64 disassembly in `out/decomp/filter-asm/` (`processaudio.s`,
`endinit.s`, `floatparam.s`, `enumparam.s`, `process.s`, `lfohelper.s`, …). Where the two
disagree (operand order, constants Ghidra rounds, such as `1/9600` shown as `0.0001044`),
this spec follows the assembly. Addresses are arm64 VM addresses in
`analysis/maschine_arm64`.

**Confidence.** **[exact]**: read from code or assembly and confirmed by the null tests.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation: `v` is a parameter's stored value. All arithmetic is `f32` unless marked `f64`.
`D = 1e-18` (`0x219392ef`) is the core's denormal offset. `round(x)` is Maschine's
`(int)((double)x ± 0.5)` (sign by `x < 0`), i.e. half away from zero.

---

## 1. Parameters

### 1.1 Stored fields

The constructor is `Filter::Filter()` at 0x101a22ba4 (`0004.c`). Each parameter is created
by `TransactionObjectContainer::insert(label, default, min, max)`. The serializer
(0x101a22984) reads the fields in offset order, which is the order of
`crates/mxprj/src/types/module.rs` (`NI::MASCHINE::DATA::Filter`) **[exact]**.

| obj off | module.rs field | label | type | default | range | RT index | display | section |
|---|---|---|---|---|---|---|---|---|
| 0xa8 | `mode` | **Mode** | Enum | 0 | `LP`, `BP`, `HP`, `Notch` | enum 0 | – | "Type"; not automatable, forces a display update |
| 0xb0 | `cutoff` | **Cutoff** | float | `convertFrom(1000 Hz)` = 0.5490537 | 0..1 | float 0 | `getFrequencyValueString` (§1.3) | "Freq" |
| 0xb8 | `resonance` | **Resonance** | float | 0.5 | 0..1 | float 1 | `ScaleNormalizedToPercentOneDecimal` | |
| 0xc0 | `amount` | **Amount** | float, bipolar | 0 | −1..1 | float 2 | percent (one decimal); wheel snaps to 0 | "Mod" |
| 0xc8 | `source` | **Source** | Enum | 0 | `LFO`, `LFO Sync`, `Env` | enum 1 | – | not automatable, forces a display update |
| 0xd0 | `speed` | **Speed** | float | 0.5 | 0..1 | float 3 | `SyncableLFOHelper::convertValueToString` (§1.3) | |
| 0xd8 | `lfo_shape` | **LFO Shape** | float | 0.5 | 0..1 | float 4 | percent | |
| 0xe0 | `phase` | **Phase** | float, bipolar | 0 | −1..1 | float 5 | percent; wheel snaps to 0 | |
| 0xe8 | `decay` | **Decay** | float | 0.5 | 0..1 | float 6 | percent | |
| 0xf0 | `smooth` | **Smooth** | float | 0.5 | 0..1 | float 7 | percent | |
| 0xf8 | `env_shape` | **Shape** | float, bipolar | 0 | −1..1 | float 8 | percent; wheel snaps to 0 | |
| 0x100 | `side_chain_filter` | (Side-Chain Input) | `SideChainFilter` | | | | docs/fx-compressor.md §1.3 | |

Bipolar parameters store their plain value (−1..1); the processor receives it as is.
Module info: name "Filter", `s_aModuleInfo` id 14. Page names (`s_aPageNames`):
**"Main"**, **"Side-Chain Input"** **[exact]** (`__GLOBAL__sub_I_Modules_Effects_Filter_ALL.cpp`).

### 1.2 Page layout (`getParameterByPosition`, 0x101a24124) **[exact]**

| pos | page | parameter | shown when |
|---|---|---|---|
| 0 | Main | Mode | always |
| 1 | Main | Cutoff | always |
| 2 | Main | Resonance | Mode ≠ Notch |
| 3 | Main | Amount | always |
| 4 | Main | Source | always |
| 5 | Main | Speed (Source LFO, LFO Sync) / Decay (Env) | |
| 6 | Main | LFO Shape (LFO, LFO Sync) / Smooth (Env) | |
| 7 | Main | Phase (LFO Sync) / Shape (Env); empty for LFO | |
| 8 | Side-Chain Input | Source | |
| 9 | Side-Chain Input | Gain | |
| 10 | Side-Chain Input | Filter | |
| 11 | Side-Chain Input | Center Freq | |
| 12 | Side-Chain Input | Width | |

### 1.3 Display converters

- **Cutoff**: `FloatExponentialConverter(46, 12540)`: `Hz = clamp(46·(12540/46)^v, 46, 12540)`
  (computed as `powf(base, a + b·v)`). `getFrequencyValueString` prints "0.00 Hz" when
  |Hz| < 0.005, `"%.2f Hz"` below 10, `"%.1f Hz"` below 1000, else `"%.1f kHz"` of Hz/1000
  **[exact]**. This is the *nominal* frequency; the filter's actual cutoff differs (§5.2).
- **Speed**: Source LFO Sync prints the division `"%d/%d"` of rate `i = round(11·v)`
  (0..11) in §4's table (e.g. "16/1", "3/8", "1/32"; bars). Source LFO prints the free rate
  (§4) through `StringUnits::hzToString` (same formats as Cutoff) **[exact]**.
- The rest print percent with one decimal (`ScaleNormalizedToPercentOneDecimal`); the
  bipolar ones show −100..100 %.

---

## 2. Parameter routing (`FilterProcessor`)

`processFloatParameter` (0x101a23af0, jump table at 0x104d2a0b8) and
`processEnumParameter` (0x101a238ec) write the core's inputs and recompute the values
derived from them at once **[exact]**:

| param | core effect |
|---|---|
| Mode (enum 0) | `mode_in = m`; output index = m; per-mode tables (§5.2): `RES_SCALE = (0.996, 0.996, 0.996, 0)`, `GAIN_KNEE = max((0.5, 0, 0.5, 0), 0.001)`, `GAIN_DROP = (0.42, 0.68, 0.42, 0)`. Recomputes damping, `k1..k3` (§5.2) from the current `g` and resonance, and `gain = 1 − drop·((last_res − knee)/(1 − knee))` (unconditionally; `last_res` is the last smoothed resonance seen above the old knee) |
| Source (enum 1) | LFO helper mode: LFO → free (1), LFO Sync → synced (2), Env → off (0). `env = (Source == Env)`; modulation span limits `(min, max) = env ? (24, 240) : (−12, 127)` (pitch); amount target updated |
| Cutoff (0) | `Hz = clamp(exp10(fma(v, 2.435539704813123, 1.662757831681574)), 46, 12540)` (`f64`, Apple `__exp10`); `note = fma(logf(Hz), 17.31234, −36.37632)`; `p = (note − 30)·(1/97)` (the core's input). Then `cut = p(2 − p)·139 − 12` (pitch target), `span = (cut − min, max − cut)`, amount target `= span[Amount > 0]·Amount`; flags \|= 5 |
| Resonance (1) | target `r = (2 − v)·v` (`f64`); flags \|= 2 |
| Amount (2) | amount target `= span[v > 0]·v`; flags \|= 4 |
| Speed (3) | `SyncableLFOHelper::setSpeed(v)` |
| LFO Shape (4) | §5.3 coefficients |
| Phase (5) | `SyncableLFOHelper::setPhaseOffs((v + 1)/2)` (cycles) |
| Decay (6) | follower hold and release (§5.3), and the output-curve blend |
| Smooth (7) | `t = (v²·69.4 + 0.6)·(rate/1000)` samples; `smooth = t < 1 ? ln2 : ln2/t` |
| Shape (8) | §5.3: `u = (v + 1)/2`, gate vs follower, threshold, gate release |

At creation the processor builds the core at 44.1 kHz and runs `endInit`; the module then
pushes every parameter in constructor order (Mode, Cutoff, Resonance, Amount, Source,
Speed, LFO Shape, Phase, Decay, Smooth, Shape); `onSamplerateChanged(sr)` stores the rate
(and sets the side-chain filter's); `resume()` **zeroes the whole core (0x380 bytes) except
the inputs** and runs `endInit` again (§6). So a fresh effect's state depends only on the
inputs.

---

## 3. Processor (`FilterProcessor::process`, 0x101a258bc) **[exact]**

Per block of Maschine's engine: it splits each host block into **128-sample blocks**
(the last one shorter) and calls `process` for each, all with the **same** `ProcessInfo`
song position, the host block's start **[exact, by test]**:
1. `SyncableLFOHelper::newFrame(tempo, playing, songPosition)` (§4) gives the LFO rate
   `hz`. The core gets `lfo_hz = hz`, `lfo_slew = hz·48·(1/9600)·(44100/rate)` and
   `inc = hz·(1/rate)`. If the helper re-phased (synced and playing), the core's LFO phase
   is set to the helper's phase.
2. Smoothing stop: for cutoff (bit 1), resonance (bit 2) and amount (bit 4), the bit clears
   when `|(f64)current_f32 − target| ≤ 1e-4`. While **any** bit is set all three smooth
   (§5.2); when none is, they freeze where they are.
3. Side-chain is active when input pin 1 has more than one channel. The core's
   `sc` flag follows it. **The side-chain filter (`SidechainFilterCore::processAudio`, see
   docs/fx-compressor.md §4) runs over the side-chain only when the core's Mode input is
   exactly 1.0 (BP)** — apparently a bug (probably meant the Env source), confirmed by the
   null tests: with Mode LP the side-chain reaches the follower unfiltered.
4. Per sample: the core takes main L/R and side-chain L/R and runs `processAudio`; the
   outputs replace main L/R. There is no dry/wet mix and no latency.

---

## 4. LFO rate (`SyncableLFOHelper`, 0x101af98b4..0x101af9d54) **[exact]**

Built with the 12-entry rate table at 0x10dc2c574 (bars, as `num/den` ints):
`16, 8, 4, 2, 1, 1/2, 3/8, 1/4, 3/16, 1/8, 1/16, 1/32`.

- **Free (Source LFO)**: `pos = 11·v`; `i = clamp(round(pos − 0.5), 0, 10)`; `f = pos − i`;
  `s = fma(1 − f, r[i], f·r[i+1])`; `Hz = 1/(s + s)`. So 1/32 Hz at v = 0, 16 Hz at v = 1,
  interpolated in bar length.
- **Synced (Source LFO Sync)**: `i = clamp(round(11·v), 0, 11)`; `beats = 4·r[i]`;
  `Hz = tempo·(1/60)/beats` (`tempo` is the host tempo as `f32`).
- `newFrame`, synced and playing: `x = songPosition(quarters, f64)/beats + phaseOffs`;
  `phase = (f32)(x − round(x))` (−0.5..0.5) and the "re-phase" flag is set; the core's LFO
  restarts there every block. Free or stopped: no re-phase; the core free-runs.
- **The song position** is the f64 at `ProcessInfo+0x30` (the processor stores it next to
  the `BeatTickTime` it builds and `newFrame` reads that f64 at `+8`). For a host position
  `q` (quarters, VST3 `projectTimeMusic`) it is `round(q·240000)/240000`: Maschine's clock
  runs on 240000 ticks per quarter (the same resolution as the processor's `BeatTickTime`,
  `0x3a980`), rounded to the nearest tick. It is the **host block's start**, held for all
  128-sample blocks of that host block, not advanced per block **[exact, by test]** (found
  by solving, per 128-sample block, for the phase that reproduces Maschine's output: it
  steps once per 1024-frame host block of maschine-ref, and differs from `q/beats + 0.5`
  by exactly the tick rounding). So a re-phase every 128 samples pulls the LFO back to
  the block's start phase: with large host blocks the LFO stutters (e.g. 1024 frames: 8
  repeats of 128 samples' worth of sweep), and the result depends on the host block
  size. DECKARD: `fx::song_position` (the rounding) and `Effect::set_transport` (the
  host block's position, held until the next call).
- Source Env: the helper is off and its rate (the value from before) still reaches the core
  unused.

---

## 5. DSP (`FilterCore`, `processAudio` 0x101a25ca0)

### 5.1 Filter, per sample and channel **[exact]**

A trapezoidal state-variable filter whose two integrator states saturate. With `x` the
input clamped to ±100, states `z1` (band), `zh` (last high-pass), `z2` (low), and the
control-rate coefficients `g`, `k1`, `k2`, `k3` (§5.2):

```
hp  = k3·(x − (z2 + (z1·k1 + zh·k2)))
bp  = z1 + g·(zh + hp)
lp  = z2 + g·(z1 + bp)
zh  = hp + D
z1  = sat(bp) ;  z2 = sat(lp)
sat(v) = c·(1 − |c|·q) + D,  c = clamp(v, −lim, lim)      (lim = 2L, q = 0.25/L)
out = [lp, 2·bp, hp, x − 2·bp][Mode] · gain
```

The right channel is computed first, then the left (independent). Outputs: LP, BP (2·bp),
HP, Notch (x − 2·bp; Notch's resonance scale is 0, so its damping is exactly 2).

### 5.2 Control tick **[exact]**

`factor = round(sr/11025)` (4 at 44.1/48 kHz, 8 at 88.2/96 kHz), `rate = round(sr)/factor`
(integer division: 11025, 12000, 22050, 24000). A tick runs every `factor` samples: a
counter (0 after init) increments while it is below `factor − 1`; otherwise it resets and
the tick runs before the sample. Each tick, in order:

1. **LFO** (Source not Env), from the phase `ph` (−0.5..0.5): `p = ph + 0.5`, `q = 1 − p`,
   shape value `y` (0..1) by LFO Shape `v` (coefficients from the setter, `TINY = 1e-10`):
   - `v ≤ 0.5` (triangle → saw down): `c = (1−2v) > TINY ? (1−2v)/2 : 5e-11`;
     `y = 1 − (|q| ≥ c ? p/(1−c) : q/c)`.
   - `0.5 < v ≤ 0.75` (→ parabolic sine): `u = (0.5−p) + (0.5−p)`;
     `y = q + ((u(2 − |u|) + 1)·0.5 − q)·(v − 0.5)·4`.
   - `v > 0.75` (→ square): `a = |(0.5−p) + (0.5−p)|`, `w = max(1 − (v − 0.75)·4, TINY)`,
     `s = a/w`, `sign = p > 0.5 ? −1 : 1`, `y = ((a < w ? sign·s·(2 − |s|) : sign) + 1)·0.5`.

   Then `lfo += lfo_slew·((y − lfo) + D)`; `m = lfo`; `ph = (ph + inc) − round(ph + inc)`.
   (`v` selects the branch through `n = (int)((v>0.5) + (v>0.75) + 0.5)`.)
2. **Smoothing** (if any flag bit is set; `f64`):
   `cur += coef·((target − cur) + 1e-100)` for resonance, then amount, then cutoff, with
   `coef = 1 − 2^(x)`, `x = (f32)((0.31029499·0.9602356 + 0.311434·0.039764404 − 61) /
   max(rate/1000·100 − 1, 1))`, the power taken from `FilterCore`'s 1022-entry table of
   `2^(i/1021 − 0.5)` (6-digit decimals of the `f32` value) with linear interpolation and
   the exponent added to the float's bits. (≈ 0.0375 per tick at 44.1 kHz: ≈ 2.4 ms.)
   Then `res = (f32)cur_r`, `damp = (1 − RES_SCALE·res)·2`, and if `knee < res`:
   `last_res = res`, `gain = 1 − (res − knee)·(1/(1 − knee))·drop`, else `gain = 1`.
3. **Envelope follower** (Source Env): see §5.3; `m` = its output.
4. **Cutoff**: `pitch = cut + m·amount` (`amount` is the smoothed pitch span·Amount);
   `x = pitch > 132 ? 10.9994545 : pitch·0.0833292`; `n = round(x)`, `f = x − n`;
   `Hz = (f(f(f·0.056 + 0.242) + 0.693) + 1)·2^n·8.17742` (8.17742 Hz = MIDI note 0).
   `w = min((3.14159/sr)·Hz, 2)`; `g = w + (w³·0.333333 + w²·w³·0.133333)` (a tan series).
   `s = damp + g`; `k1 = g + s`; `k2 = g·s`; `k3 = 1/(k2 + 1)`.
   Saturation level: `t = (Hz − 100)·(1/7900)`; `L = t < 0 ? 24 : t·(−23.4) + 24`;
   `L = t < 1 ? L : 0.6000004`; `L = max(L, 2e-12)`; `q = 0.25/L`, `lim = L + L`.

The core's pitch for Cutoff is `p(2−p)·139 − 12` with `p = (note − 30)/97`, so the actual
cutoff is not the displayed one: displayed 1 kHz → actual ≈ 2.4 kHz; the range is pitch
−12..127 (≈ 4 Hz..12.5 kHz) for Cutoff 0..1. Resonance r gives damping
`2(1 − 0.996·r(2−r))` (Q up to ≈ 125). The output gain drops above resonance 0.5 (LP/HP,
by up to 0.42) or from 0 (BP, by up to 0.68).

Modulation is unipolar (LFO and follower both run 0..1): Amount > 0 sweeps up toward the
span maximum (pitch 127 for the LFO, 240 for Env), Amount < 0 down toward the minimum
(−12 / 24).

### 5.3 Envelope follower (Source Env) **[exact]**

Level `lvl = max(|a|, |b|)` of the side-chain (when active) or of the clamped input.
`u = (Shape + 1)/2`. Setter values: Shape interpolates over `u·2` in the tables
`time = (5e-5, 1e-4, 0.25)` and `threshold = (0.93, 0.125, 0.8)`; `gate_decay =
(u > 0.4 ? time : 1e-4)·rate < 1 ? 0.306853 : 1 − ln2/((u > 0.4 ? time : 1e-4)·rate)`.
Decay `d`: `hold = round(((|d|·0.5 + 0.5)·d·0.19 + 0.01)·rate)` ticks and
`release = ln2/that` (`ln2` if < 1). Attack is `ln2` (its time is 0).

Per tick:
```
target = u > 0.4 ? lvl                                   (follower)
       : gate: if lvl > threshold { n = 0; h = hold_cur = hold } else { n = count; h = hold_cur }
               count = n + 1; target = h > n ? 1 : 0
d = target − env
if d > 0 { k = attack; rg = 1 } else { k = rg > 0.5 ? 0 : release; rg *= gate_decay }   (old rg tested)
e = env + d·k ;  env = e + D
x = clamp(e, 0, 1) ;  a = x(2 − x)
out += smooth·((a + Decay·(x − a)) − out) ;  out stored + D ;  m = out
```

---

## 6. Initialization (`endInit`, 0x101a244d8) **[exact]**

`endInit` derives every coefficient from the inputs (as the setters do), starts the three
smoothers at their targets, sets the flags to 7, and then evaluates one full control tick
and one sample on silence (inputs zeroed by `resume`), so the LFO phase advances once (with
the LFO rate from before resume, 0 for a new effect) and the filter states hold `D`. The
sample counter is then 0, so the next tick comes after `factor − 1` samples.

---

## 7. Null tests

Source: `/Users/Shared/Chromatic Fire Library/Projects/Exosphere.mxprj`, sound 0/4, slot 1
(a Filter); `maschine-ref make … 0 4 1 field=value…`, `render --preroll 0 --seconds 2`,
then `fx-null` (with `FX_TEMPO=120`, maschine-ref's default host tempo). Script:
`/private/tmp/claude-501/-Applications-Native-Instruments-Maschine-3/d9bc38fd-75cf-45e9-a132-1940350f781c/scratchpad/fx-filter/null.sh` (scratch). Residual is L/R RMS.

| test | settings | input | sr | residual |
|---|---|---|---|---|
| project | HP, Cutoff 0.4391, Res 0 | noise | 44.1k | −240 dB (bit-exact) |
| lp / bp / hp / notch | Mode 0..3, Cutoff 0.5, Res 0.5 | noise | 44.1k | −240 each |
| lp48 | LP, 0.5, 0.5 | noise | 48k | −240 |
| cut0 / cut1 | LP, Cutoff 0 / 1, Res 1 | noise | 44.1k | −240 |
| res1 | BP, Cutoff 0.7, Res 1 | sweep | 44.1k | −240 |
| lfo1..lfo4 | LFO, Amount 1/−1/0.5/0.7, Speed 0.9/1/0.7/0.8, LFO Shape 0/0.25/0.6/0.8, Modes LP/LP/BP/HP | noise | 44.1k | −240 |
| lfo5 / lfo6 | LFO Shape 1 (square), Speed 0.95 / Shape 0.5, Speed 0.33 | noise | 48k / 44.1k | −240 |
| sync1 / sync2 | LFO Sync, Speed 0.9 / 1, Phase 0 / 0.5, tempo 120 | noise | 44.1k / 48k | −240 |
| sync tempo | sync1 at `--tempo 90`, `137.5` | noise | 44.1k | −240 |
| env1..env6 | Env, Shape 1/0.5/−1/−0.3/0/0.3, Decay, Smooth, Amount ±, all four Modes | sweep, noise, impulse, sine 440 | 44.1k/48k | −240 |
| envsc0 | Env, Shape −0.6 (gate), Decay 0.1, Smooth 0.9 | noise | 44.1k | −240 |
| def96 / r88 | LFO, 96 kHz / 88.2 kHz (tick every 8 samples) | noise / sweep | 96k / 88.2k | −240 |
| loud | LP, Cutoff 0.95, Res 1, LFO; input +20 and +46 dBFS (clamp ±100, saturation) | noise | 44.1k | −240 |
| sc0 | Env + side-chain (`--sc 0/5`, In 2 sweep), Mode LP, SC Filter on, SC Gain 0.5 | noise + sweep | 44.1k | −240 (SC unfiltered) |
| sc1 | same, Mode BP | | 44.1k | −240 (SC filtered) |
| sc2 | Env gate (Shape −1), Amount −1, BP, SC impulses | sweep + impulse | 48k | −240 |
| free LFO, `--play` | lfo1 with the transport playing | noise | 44.1k | −240 |
| synced, `--play` | sync1 with the transport playing (`FX_PLAY=1`) | noise | 44.1k | −240 |
| synced, `--play` tempi | sync1 at `--tempo 90` / `137.5` | noise | 44.1k | −240 / −240 |
| synced, `--play` 48k | BP, Res 0.6, Amount −0.7, Speed 1, Phase 0.5, LFO Shape 0.8 | noise | 48k | −240 |
| synced, `--play` slow | HP, Cutoff 0.3, Amount 1, Speed 0.45, Phase −0.3, LFO Shape 0.2, tempo 137.5 | noise | 44.1k | −240 |
| synced, `--play` 96k | LP, Cutoff 0.5, Amount 0.9, Speed 0.75, tempo 90 | noise | 96k | −240 |
| synced, stopped | sync1 at tempo 137.5, no `--play` | noise | 44.1k | −240 |

Not tested against Maschine: parameter changes mid-stream (maschine-ref renders a fixed
project), so the smoothing and the per-block stop (§3) are from the code only.

---

## 8. Open points

1. *(Resolved.)* Synced LFO with the transport running: the position is the host block's
   start, rounded to 240000 ticks per quarter, held for every 128-sample block (§4); the
   port nulls bit-exactly with `--play` (§7; `fx-null` with `FX_PLAY=1`). Still from the
   tests only: where Maschine's engine does the tick rounding and the 128-sample split
   (outside the processor), and how it behaves with host blocks that are not multiples
   of 128 or with a looping/relocating host transport (maschine-ref plays straight from 0).
2. The side-chain filter runs only in Mode BP (§3). This is how Maschine behaves (nulled),
   not necessarily NI's intent.
3. The displayed Cutoff frequency is nominal; the actual cutoff is higher (§5.2).
4. `SideChainFilter::Processor` and the side-chain Source routing: as in
   docs/fx-compressor.md §6.6–6.7.
5. The QML editor (`Modules/FX/FilterEditor.qml`, `FilterModel` with Mode, Cutoff,
   Resonance, Amount, Source, Speed, Phase, LFO/Env shapes, Decay, Smooth bindings) was not
   decoded.
