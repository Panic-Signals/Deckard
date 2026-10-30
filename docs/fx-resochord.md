# Maschine's Resochord: parameters and DSP

A clean-room description of Maschine 3's built-in **Resochord** effect
(`NI::MASCHINE::DATA::Resochord` → `NI::MASCHINE::RT::ResochordProcessor` →
`NI::MASCHINE::FXCORE::ResochordCore`). `ResochordCore` is a compiled Reaktor-Core kernel:
a flat struct of inputs, outputs and state with `endInit()`, `processAudio()` (one sample
per call) and one `processEventIn*()` per parameter (three more are inlined in the
processor).

It was written from Ghidra output in `out/decomp/resochord/` (`INDEX.tsv` maps files to
functions) and from arm64 disassembly: `out/decomp/resochord-all.s` (everything),
`resochord-pa.s` (`processAudio`), `resochord-ei.s` (`endInit`), `resochord-ev.s` (event
handlers). **Ghidra re-associates sums and products in its output** (for example it
prints `a + b + c` where the code computes `a + (b + c)`), so every chain of three or
more operands was taken from the assembly. Addresses are arm64 VM addresses in
`analysis/maschine_arm64`. The DECKARD port is `crates/engine/src/fx/resochord.rs`. It
nulls **bit-exactly** against Maschine (§6).

**Confidence.** **[exact]**: read from code or assembly. **[inferred]**: reconstructed
from partial evidence. **[unknown]**: not recovered.

Notation: `v` is a parameter's stored value. Cells of the kernel are written `k[i]`, `i`
being the byte offset / 4 (the port uses the same indices). All arithmetic is `f32`; the
kernel has **no fused multiply-adds**. `rnd(x)` adds ±0.5 (by the sign of `x`) **in
`f64`** and truncates. `DC` = 1e-18 (`0x219392ef`), the kernel's denormal guard.

---

## 1. Parameters

### 1.1 Module

Module name "Resochord" (`Module::Module`, string at 0x104d338bc). One page:
`s_aPageNames = { "Main" }` (`__GLOBAL__sub_I_Modules_Effects_Resochord_ALL.cpp`,
0x101a6f12c) **[exact]**.

### 1.2 Parameters in constructor order

Constructor `Resochord::Resochord()` at 0x101a63dbc. Each parameter is made by
`TransactionObjectContainer::insert(label, default, min, max)` or, for enums,
`insert(label, default, options)` **[exact]**. Labels are wide strings at 0x104d33900–
0x104d33a5c.

| obj off | mxprj field | label | section | default | range | RT idx | display |
|---|---|---|---|---|---|---|---|
| 0xa8 | `spread` | **Spread** | – | 0.5 | 0..1 | float 0 | `ScaleNormalizedToPercentOneDecimal` |
| 0xb0 | `brightness` | **Brightness** | "Color" | 0.75 | 0..1 | float 1 | `ScaleNormalizedToPercentOneDecimal` |
| 0xb8 | `feedback` | **Feedback** | – | 0.75 | 0..1 | float 2 | `ScaleNormalizedToPercentOneDecimal` |
| 0xc0 | `decay` | **Decay** | – | 0.5 | 0..1 | float 3 | `ScaleNormalizedToPercentOneDecimal` |
| 0xc8 | `mix` | **Mix** | "Output" | 0.75 | 0..1 | float 4 | `ScaleNormalizedToPercentOneDecimal` |
| 0xd0 | `tune` | **Tune** | – | 0 | −24..24 (int) | int 0 | none (plain integer); `setBipolar(true)`; +0x1bc = 0.02 (fine step) |
| 0xd8 | `style` | **Style** | – | 0 | enum, 5 | enum 1 | "maj", "min", "alt", "meta", "frank" |
| 0xe0 | `mode` | **Mode** | "Pitch" (span 4) | 1 | enum, 2 | enum 0 | "String", "Chord"; not automatable; +0x2d8 = 1 |
| 0x100 | `chord` | **Chord** | – | 1 | enum, 5 | enum 2 | "-", "5", "7", "7/9", "7/11" |

The option vectors are built by the static initializer: Mode at 0x10e306308, Style at
0x10e306320, Chord at 0x10e306338 **[exact]**. `ScaleNormalizedToPercentOneDecimal`
(0x10dce2238) prints `v·100` with one decimal and `%`. The physical values of §3 are
never shown. The QML editor model (`GUI::qml_module::ResochordModel`) names the Tune
binding `getTransposeBinding`; the parameter's label is "Tune".

### 1.3 Knob layout (`getParameterByPosition`, 0x101a64bb8)

| pos | String mode (0) | Chord mode (1) |
|---|---|---|
| 0 | Mode | Mode |
| 1 | Spread | Style |
| 2 | – (empty) | Chord |
| 3 | Tune | Tune |
| 4 | Brightness | Brightness |
| 5 | Feedback | Feedback |
| 6 | Decay | Decay |
| 7 | Mix | Mix |

Positions 1 and 2 test the Mode value (non-zero = Chord) **[exact]**. Section headings:
slot 0 "Pitch" (spanning 4), slot 4 "Color", slot 7 "Output" **[exact]**.

### 1.4 Serialization

`Resochord::serialize<iarchive>` (0x101a63be4) loads the Module base, then the objects at
0xd8, 0xe0, 0x100, 0xa8, 0xd0, 0xb0, 0xb8, 0xc0, 0xc8 in that order **[exact]**. In
`crates/mxprj` these are `style`, `mode`, `chord`, `spread`, `tune`, `brightness`,
`feedback`, `decay`, `mix` (formerly `unknown_d8`, `unknown_e0`, `unknown_100`,
`unknown_a8`, `unknown_d0`, `unknown_b0`, `unknown_b8`, `unknown_c0`, `unknown_tail`).

### 1.5 Value → engine

`processFloatParameter` (0x101a647b4), `processIntParameter` (0x101a648f8) and
`processEnumParameter` (0x101a64a3c) store the value into a kernel input and run its
handler **[exact]**:

| parameter | kernel input | handler |
|---|---|---|
| Spread | k[0xd] | `processEventInSpread` |
| Brightness | k[0xb] | inline (§3.1) |
| Feedback | k[0x6] | `processEventInFeedback` |
| Decay | k[0x9] | `processEventInDecay` |
| Mix | k[0x4] | inline (§3.1) |
| Tune | k[0x17] = (f32)t | inline (§3.1) |
| Mode | k[0xf] = (f32)index | `processEventInMode` |
| Style | k[0x13] = (f32)index | `processEventInStyle` |
| Chord | k[0x15] = (f32)index | `processEventInChord` |

### 1.6 MIDI

The effect does **not** follow MIDI notes. `ResochordProcessor::process` (0x101a687c0)
only copies audio: per sample `in L → k[3]`, `in R → k[8]`, `processAudio()`,
`k[0x11] → out L`, `k[0x12] → out R`. It reads no events, and the kernel has no note or
gate input. The pitch comes only from Tune, Mode, Style and Chord. (A note-following
version would need an `Effect` method for note events that sets Tune; nothing in
Maschine does this.)

---

## 2. Processor and initialization

- **Constructor** (0x101a64d38): stereo in, stereo out. It allocates the kernel (0x306c8
  bytes, zeroed) and presets the inputs (Mode 1, Spread 0.5, Chord 1, Tune −12,
  Brightness 0.9, Feedback 0.75, Decay 0.8, Mix 0.75). The stored values replace these
  before audio runs.
- **`onSamplerateChanged(sr)`** stores `sr` (+0x48); it is used at the next `resume`.
- **`resume()`** (0x101a64ee0): saves the nine inputs, **zeroes the kernel**, writes
  `sr` to k[0], restores the inputs with their event flags (1.0), and calls `endInit()`.
- **`endInit()`** (0x101a64fc0) **[exact]** sets the rate-dependent constants (§2.1),
  evaluates every handler, and runs one `processAudio` step on a silent input in which
  **every glide and smoother completes at once** (position or coefficient 1). The port
  does exactly that: constants, the handlers in dependency order (Spread, Mode, Tune,
  Style, Chord, Spread, Decay, Feedback, Brightness, Mix), then `tick(init = true)`. The
  output of the first rendered sample proves the equivalence (§6).

So the control counter and the vibrato free-run from `resume`. With `maschine-ref render
--preroll S`, DECKARD must first process `S·sr` silent samples (§6).

### 2.1 Constants (`endInit`) **[exact]**

```
steps   = rnd(sr / 8820)            control period, samples (5 at 44.1/48 k, 11 at 96 k)
ctl     = rnd(sr) / steps           integer division; control rate (8820 Hz at 44.1 k)
k[0x1a] = steps − 1; k[0x19] = steps (so the first step runs the control block)
ramp    = 1 / (ctl · 0.02)          glide step: 20 ms linear glides (k[0x8c], k[0x1c], k[0x23])
k[0x93] = 1 / (ctl div 50)          vibrato phase step; k[0x92] = 50
L       = 0.677542·0.399963 + 0.678425·0.600037 − 4 = log2(0.1)     (log table, k[0x2b])
k[0x68] = 1 − 2^L                   level follower attack (one sample to 90 %)
k[0x69] = 1 − 2^(L / max(sr − 1, 1))           its release (1 s to 90 %)
k[0x2a] = 1 − 2^(L / max(15·ctl/1000 − 1, 1))  envelope attack (15 ms to 90 %)
k[0x64] = rnd(sr)                   level-follower hold (1 s)
k[0x47] = min(2π·30/sr, 1)          DC blocker
k[0x180] = 1 − ln2/(0.01·sr), k[0x62] = ln2/(0.05·sr), k[0x60] = ln2/(0.001·sr)   (AGC)
tan(πf/sr) one-poles: 30 Hz (input high-pass), 4991.889 Hz, 184.5965 Hz (wet filters),
                     181.8083 Hz (every voice's shelf)
voice limits: 0.9·sr (pitch-filter clamp), 0.494·sr (Brightness clamp), π/sr
k[0x4b] = 0.412639, k[0x50] = 0.420360, shelf mix = −0.0197935
k[0x6b..0x77]: 0.5, 2 (Brightness), 0.5, 2 (Feedback), 0.5, 2 (Decay), 0.75, 4 (Spread)
```

The exponentials use the kernel's octave table `s_04D103F0` (0x10dc364c8), the same
table as Chorus (docs/fx-chorus.md §3.1), with `2^n` put into the exponent bits.

---

## 3. Control values

### 3.1 Handlers **[exact]**

- **Mix** `m`: dry target `1 − m` (glide k[0x1d]), wet curve `k[0x63] = (2 − m)·m`.
- **Brightness** `b`: `k[0x2f] = (|b|·−0.6 + 1.6)·b` (the envelope's Brightness
  depth); above 0.5 the wet level is trimmed by `1 − (b − 0.5)·2·0.33`.
- **Feedback** `f`: 41-step tables at `40f` (linear interpolation, index clamped to
  40): input gain `IN_GAIN·⅓` (0.28 … 0.09), loop gain `FB_GAIN` (0.62 … 1.2), clip level
  `CLIP·½` (25 … 0.045), each gliding over 20 ms. Above 0.5 the wet level is trimmed by
  `1 − (f − 0.5)·2·0.3`, and the AGC weight is `q = ((2f−1)(2 − (2f−1))·0.99)²`.
- **Decay** `d`: with `h = 2d`, `a = h−1, b = 1` above `h = 1`, else `a = 0, b = h`;
  `s = 0.9a³ + 0.1b³`, `dd = s²`. Hold `rnd(ctl/1000·(488·dd + 12))` control steps
  (12 … 500 ms), threshold `0.2 − 0.19·dd`, release coefficient
  `1 − 2^(L / max(ctl/1000·(7992·dd + 8) − 1, 1))` (8 ms … 8 s to 90 %). Below `s = 0.5`
  the wet level gets `+((0.5 − s)·2)²·0.3`.
- **Spread** `s`: `s²` (String mode only) × the voice multipliers −0.222, +0.222,
  −0.444, +0.444, −0.666, +0.666 semitones. Below `s = 0.5` (String mode) the wet level is
  trimmed by `0.25·(((1 − s²) − 0.75)·4)²`.
- **Mode**: String → base note 36, chord gain 0, vibrato depth 0, wet offset −0.2;
  Chord → base note 60, chord gain 1, vibrato depth 0.07 semitones, offset 0.
- **Style / Chord**: `index = 30·style + 6·chord`; the six voices take
  `CHORDS[index + j]` (`s_06C064A8`, 0x10dc36270, 150 values, clamped to entry 149).
- **Tune** `t`: base note += `t` semitones.

Each voice's pitch target is `base + tune + spread·mult + chord·gain + vibrato` and it
glides there linearly over 20 ms. The wet level target is
`(spread trim + decay boost + mode offset) · brightness trim · feedback trim`.

### 3.2 Control block (every `steps` samples) **[exact]**

1. **Vibrato** (every 50th control step, so 1 Hz): phase `p += k[0x93]`, wrapped to
   ±0.5; triangle `1 − 4|p|`, and a second one a quarter period later. Voices A/C get
   `+depth·tri0`, D/F `−depth·tri0`, B `+depth·tri1`, E `−depth·tri1`. Every voice
   re-aims its glide.
2. **Envelope**: `peak = max(|L|, |R|)`. Above the Decay threshold the hold counter
   restarts; the gate is 1 while the counter is below the hold. A one-pole follows the
   gate (`attack` k[0x2a] up, Decay release k[0x2d] down, `+DC`), `e·(2 − e)`.
3. The envelope × Brightness depth is smoothed (coefficient `min(k[0x23], 1)`) into
   `k[0x87]`, and every voice recomputes its Brightness filter.
4. Signal present (`peak > 0.0001`) re-arms the level follower's 1 s hold.
5. The glides: pitch of each voice (period `sr / (8.17742·2^(pitch/12.0006))`, with a
   cubic `2^x`), dry gain, clip level (`±2·level`, the soft-clip slope `0.25/(2·level)`),
   loop gain, input gain, wet level.

Per voice, when its pitch or `k[0x87]` changed **[exact]**:

```
f_b  = 8.17742·2^((pitch + (136 − pitch)·k[0x87]) / 12.0006)       (quadratic 2^x)
brt  = π / tan(π·clamp(f_b, 1, 0.9·sr)/sr)
g_b  = tan(π·clamp(f_b, 0, 0.494·sr)/sr)                            (Brightness low-pass)
ptan = π / tan(π/period)
corr = atan(brt/ptan)·(−1/π)·period·0.5                            (its phase delay)
delay = period − 1 + corr, clamped to 0 … 8190
```

`tan` is `x + x³·0.333333 + (x²·x³)·0.133333`; `atan` an odd degree-17 polynomial on
|x| ≤ 1 and `±π/2 − atan(1/x)` outside. The delay is split into `n = rnd(d − 0.5)` and
`f = d − n`.

---

## 4. Audio (every sample) **[exact]**

1. **Input high-pass** (30 Hz) per side, with the input gain:
   `y = (x·gain − s·(g−1)) / (g+1)`, `base = y − s`, `s = y + DC`.
2. **Voices.** A, B, C take the left `base`, D, E, F the right. Each:
   `x = base + out·loop_gain` (`out`: its own clipped output of the previous sample);
   182 Hz shelf `u = (x − s1(g−1))/(g+1)`, `w = ((s1 + u)·g)·−0.0197935`; Brightness
   low-pass `y = ((x + w) − s2(g_b−1))/(g_b+1)`, written to its 8192-sample line as
   `g_b·(s2 + y)`.
3. **Wet** per side: `x_3 + (x_1 + x_2)` (the voices' inputs, not their lines), DC
   blocker `m += (sum − m)·k[0x47]`, then two sections:
   `x += 0.412639·(y1 − s1)` (4992 Hz one-pole) and
   `x += 0.420360·(g2·(s2 + y2))` (184.6 Hz one-pole).
4. **Level follower** (k[0x67]): gate = hold counter not expired; attack k[0x68],
   release k[0x69]. It scales the wet level (smoothed per control step).
5. **AGC**: `pk = max(|wetL|, |wetR|)·q`; `d = |pk + 1 − 0.75q| − level`; up at k[0x60]
   (and the hold `h = 1`), down at k[0x62] once `h` (× k[0x180] per sample) is ≤ 0.5.
   `gain = 1/|level|`, or 100 below 0.01. At Feedback ≤ 0.5, `q = 0` and the gain is 1.
6. **Output**: `out = in·dry + (wet·gain)·wet_level`.
7. **Lines**: each voice reads its line `d` samples behind the sample just written
   (cubic over taps `n − 1 … n + 2`, the formula of Chorus §3.5); the output feeds the
   next sample's input, so the loop is `period + corr` long. Then it clips: `c = clamp(y, −2·level, 2·level)`,
   `out = c·(1 − |c|·0.25/(2·level)) + DC`.

With Mix 0 the output is the input bit for bit. The two sides share only the envelope
and the AGC (both follow the louder side).

---

## 5. Implementation notes

- The port keeps the kernel as an array of cells indexed like the kernel, so each line
  of `tick` can be checked against the assembly; voices are described by the base
  indices of their cell blocks (`Voice`).
- `Resochord::set` resumes on the first call and afterwards sends only the changed
  parameters' events, as Maschine does; the glides, delay lines and followers keep
  running.
- Not verified against Maschine: parameter changes during a render (`maschine-ref` has
  no automation). The handlers follow the assembly, and a unit test checks that
  re-sending the same values changes nothing.
- Not used: side-chain, tempo, transport, meters (the processor has none).

## 6. Null tests

Source project: `/Users/Shared/Maschine 2 Library/Projects/Breeze.mxprj`, sound 4/0
slot 1 (Resochord; stored style 0, mode 1, chord 0, spread 0.5, tune 12, brightness
0.42, feedback 0.65, decay 0.82, mix 0), made with `maschine-ref make … 4 0 1
[field=value …]`. Rendered at −12 dBFS with `--preroll 0` unless noted; compared with
`fx-null` on the captured input (scratch script `null.sh`). Residuals L / R; **−240 dB
means bit-identical**. Fields not listed keep the project's values.

| case | input | sr | parameters | residual |
|---|---|---|---|---|
| project (mix 0) | noise | 44.1 k | – | −240 / −240 |
| defaults | noise | 44.1 k | mode 1, style 0, chord 1, spread 0.5, tune 0, br 0.75, fb 0.75, decay 0.5, mix 0.75 | −240 / −240 |
| | noise | 44.1 k | mix 0.5 | −240 / −240 |
| String | noise | 44.1 k / 48 k | mode 0, mix 1 | −240 / −240 |
| Chord | noise | 48 k | mode 1, mix 1 | −240 / −240 |
| all 25 Style × Chord | noise | 44.1 k | mode 1, style 0–4, chord 0–4, mix 1 | −240 / −240 (each) |
| spread min / max | noise | 44.1 k | mode 0, spread 0 / 1, mix 1 | −240 / −240 |
| spread max in Chord | noise | 44.1 k | mode 1, spread 1, mix 1 | −240 / −240 |
| tune −24 / +24 | noise | 44.1 k | mode 0 tune −24 / mode 1 tune 24, mix 1 | −240 / −240 |
| tune 7 | noise | 48 k | mode 0, tune 7, spread 0.3, mix 1 | −240 / −240 |
| brightness 0 / 1 | noise | 44.1 k | br 0 / 1 (mode 1 and 0), mix 1 | −240 / −240 |
| feedback 0 / 0.5 / 0.513 / 1 | noise | 44.1 k | mix 1 | −240 / −240 |
| feedback 1, bright 1 | noise | 48 k | mode 0, fb 1, br 1, mix 1 | −240 / −240 |
| decay 0 / 0.3 / 1 | noise | 44.1 k | mix 1 | −240 / −240 |
| mix 0 / 1 | noise | 44.1 k | – | −240 / −240 |
| odd values | noise | 48 k | mode 0, spread 0.777, tune −5, br 0.123, fb 0.91, decay 0.66, mix 0.42 | −240 / −240 |
| | sweep | 44.1 k | mix 0.8 | −240 / −240 |
| | sweep | 48 k | mode 1, style 3, chord 2, tune −24, mix 1 | −240 / −240 |
| | sine 220 | 48 k | mode 0, mix 1 | −240 / −240 |
| ringing | impulse | 44.1 k | mode 0, fb 0.95, decay 1, mix 1 | −240 / −240 |
| self-oscillation | impulse | 44.1 k | mode 1, fb 1, decay 1, br 1, mix 1 | −240 / −240 |
| loud (0 dBFS) | noise | 44.1 k | mode 1, fb 1, decay 1, mix 1 | −240 / −240 |
| +6 dBFS | noise | 48 k | mode 0, fb 1, br 1, mix 1 | −240 / −240 |
| quiet (−60 dBFS) | noise | 44.1 k | mode 1, mix 1 | −240 / −240 |
| control period 11 | noise | 96 k | mode 1 / mode 0 tune −24, mix 1 | −240 / −240 |
| control period 3 | noise | 22.05 k | mode 1, mix 1 | −240 / −240 |
| control period 22 | noise | 192 k | mode 0, mix 1 | −240 / −240 |
| `--preroll 1`, input as is | noise | 44.1 k | mode 1, mix 1 | −64.8 / −65.7 (vibrato phase) |
| `--preroll 1`, DECKARD fed 44100 zeros first | noise | 44.1 k | mode 1, mix 1 | −240 / −240 |
