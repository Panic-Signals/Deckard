# Maschine's Flanger: parameters and DSP

A clean-room description of Maschine 3's built-in **Flanger** effect
(`NI::MASCHINE::DATA::Flanger` → `NI::MASCHINE::RT::FlangerProcessor` →
`NI::MASCHINE::FXCORE::FlangerCore`). `FlangerCore` is a compiled Reaktor-Core kernel: a
flat struct of inputs, outputs and state, with `endInit()`, `processAudio()` (one sample
per call) and one event handler (`processEventInCtr`). The processor computes the other
parameters' derived values itself and writes them into the kernel. The LFO's rate comes
from `NI::MASCOMMON::LEGACY::SyncableLFOHelper`, the helper that the Filter also uses
(docs/fx-filter.md §4).

It was written from Ghidra output in `out/decomp/flanger/` (`INDEX.tsv` maps files to
functions) and from arm64 disassembly in `out/decomp/flanger-asm/` (`core.s` =
`processAudio`, `endinit.s`, `ctr.s`, `params.s` = the three `process*Parameter`,
`proc.s` = constructor/resume, `process.s`). Where the two disagree (Ghidra shows some
`f64` roundings as `f32` and drops a `fmaxnm`), this spec follows the assembly. Addresses
are arm64 VM addresses in `analysis/maschine_arm64`. The DECKARD port is
`crates/engine/src/fx/flanger.rs`. It nulls **bit-exactly** against Maschine (§5), except
also for a synced LFO while the transport plays.

**Confidence.** **[exact]**: formula and constants read from code or assembly.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation: `v` is a parameter's stored value. All arithmetic is `f32` unless the text says
`f64`. The kernel and processor have **no fused multiply-adds** (the LFO helper has one,
docs/fx-filter.md §4). `rnd(x)` adds ±0.5 (by the sign of `x`) **in `f64`** and truncates,
so halves round away from zero.

---

## 1. Parameters

### 1.1 Module

Module name "Flanger" (`Module::Module`, wide string at 0x10dce6318; `s_aModuleInfo` =
0x11). One page, `s_aPageNames = { "Main" }` (`__GLOBAL__sub_I_Modules_Effects_Flanger_ALL.cpp`,
0x101a2e494) **[exact]**.

### 1.2 Parameters in constructor order

Constructor `Flanger::Flanger()` at 0x101a28f14 (`0015.c`). Labels are wide strings at
0x104d2ae30…0x104d2af1c **[exact]**.

| obj off | mxprj field | label | section | default | range | RT idx | display converter |
|---|---|---|---|---|---|---|---|
| 0xa8 | `frequency` | **Frequency** | "Main" | 0.5 | 0..1 | float 0 | `ScaleNormalizedToPercentOneDecimal` |
| 0xb0 | `amount` | **Amount** | "Mod" | 0.75 | −1..1, bipolar | float 1 | `ScaleNormalizedToPercentOneDecimal` (−100.0 %…100.0 %); wheel snaps to 0 |
| 0xb8 | `source` | **Source** | | 0 | enum `LFO`, `LFO Sync`, `Envelope` | enum 0 | (enum names); not automatable, forces a display update |
| 0xc0 | `speed` | **Speed** | | 0.5 | 0..1 | float 2 | `SyncableLFOHelper::convertValueToString` (§1.4) |
| 0xc8 | `shape` | **Shape** | | 0.5 | 0..1 | float 3 | `ScaleNormalizedToPercentOneDecimal` |
| 0xd0 | `stereo` | **Stereo** | | 0.5 | 0..1 | float 4 | `ScaleNormalizedToPercentOneDecimal` |
| 0xd8 | `feedback` | **Feedback** | | 0.5 | 0..1 | float 5 | `ScaleNormalizedToPercentOneDecimal` |
| 0xe0 | `invert` | **Invert** | | false | bool | bool 0 | on/off |
| 0xe8 | `mix` | **Mix** | "Output" | 0.5 | 0..1 | float 6 | `ScaleNormalizedToPercentOneDecimal` |

The source list `aSourceValueList` is `{ "LFO", "LFO Sync", "Envelope" }` **[exact]**.

### 1.3 Knob layout (`getParameterByPosition`, 0x101a29ff4) **[exact]**

| pos | parameter | section heading |
|---|---|---|
| 0 | Frequency | "Main" |
| 1 | Feedback | |
| 2 | Invert (switch) | |
| 3 | Amount | "Mod" |
| 4 | Source (enum) | |
| 5 | **Speed** when Source ≠ Envelope; **Shape** when Source = Envelope | |
| 6 | Stereo | |
| 7 | Mix | "Output" |

### 1.4 Speed display

`SyncableLFOHelper` built with the 11-entry rate table at 0x10dc2e78c (bars, `num/den`):
`16, 8, 4, 2, 1, 1/2, 3/8, 1/4, 3/16, 1/8, 1/16` **[exact]**. With `n = 11`:
- Source LFO Sync: `i = clamp(rnd(10·v), 0, 10)`, printed `"%d/%d"` of the table entry
  (e.g. "16/1", "3/8", "1/16").
- Source LFO (and Envelope): the free rate `Hz` of §2.4 through the `hzToString` formats:
  "0.00 Hz" if |Hz| < 0.005, `"%.2f Hz"` below 10, `"%.1f Hz"` below 1000, else
  `"%.1f kHz"`. Range 0.03 Hz (v = 0) … 1.00 Hz (50 %) … 8.00 Hz (v = 1).

### 1.5 Serialization

`Flanger::serialize<iarchive>` (0x101a28d14) loads the Module base, then 0xa8, **0xb8 (only
for class version ≠ 0)**, 0xb0, 0xc0, 0xc8, 0xd0, 0xd8, 0xe0, 0xe8 **[exact]**. These are the
`crates/mxprj` fields `frequency`, `source`, `amount`, `speed`, `shape`, `stereo`,
`feedback`, `invert`, `mix` (were `unknown_a8/b8/b0/c0/c8/d0/d8/e0/tail`). Version-0
projects (e.g. "Big Stream") have no Source, which means LFO.

---

## 2. Processor (`FlangerProcessor`)

### 2.1 Structure **[exact]**

- **Constructor** (0x101a2a16c): stereo in/out. It allocates the kernel (0x82a0 bytes,
  zeroed), builds the LFO helper (11 rates, free mode, speed 0.5), presets the kernel inputs
  (all 0 except Mix input 1.0 and LFO-rate input 1.0), sets the helper's phase offset to
  0.5, and calls `onSamplerateChanged(44100)`. The module then pushes every parameter in
  constructor order (Frequency, Amount, Source, Speed, Shape, Stereo, Feedback, Invert,
  Mix).
- **`onSamplerateChanged(sr)`** only stores `sr`.
- **`resume()`** (0x101a2a338): saves the kernel's ten inputs, **zeroes the whole kernel**
  (both delay lines, all state), writes `sr` and the inputs back with their "changed" flags
  (1.0), and calls `endInit()` (§3.1). So a fresh effect's state depends only on the
  inputs. The LFO-rate input is the last block's rate (1.0 Hz before any block ran).

### 2.2 Parameter routing (`processFloatParameter` 0x101a298c4, jump table 0x104d2ad90; `processEnumParameter` 0x101a29c8c; `processBoolParameter` 0x101a29dec) **[exact]**

Each handler stores the input and recomputes the derived kernel values at once:

| param | effect |
|---|---|
| Frequency (0) | `processEventInCtr` (0x101a2be54): centre delay `d` (ms, §3.2); `d_hi = 68.62889 − d`, `d_lo = d − 0.10045407`; centre target `= (f64)d`; span (below); flags \|= 3 |
| Amount (1) | `a = v`, `a2 = v<0 ? −v² : v²`; span; flags \|= 2 |
| Speed (2) | stored; `SyncableLFOHelper::setSpeed(v)` |
| Shape (3) | follower hold/fall (§3.5) |
| Stereo (4) | `st_env = v·(−0.02) + 1`, `st_offset = v²·0.5` |
| Feedback (5) | `f = (2 − v)·v`; feedback block (below) |
| Mix (6) | `mix_sq = v²`, `dry_t = (f64)(mix_sq·(g − 1) + 1)`, `mix_c = (2 − v)·v`, `wet_t = (f64)(mix_c·g)`; flags \|= 0x18 |
| Source (enum) | LFO → helper free (1), LFO Sync → synced (2) (both `setSpeed(stored speed)` first), Envelope → off (0) (`switchToMode`). Kernel: `env = Source == Envelope`, `lfo_on = !env`; `red = span_red·lfo_on`; depth target; flags \|= 2 |
| Invert (bool) | `i = Invert`; `fb_sign = (1, −1)[i]`, `knee_k = (0.33, 0.3)[i]`, `d_scale = (1.0, 0.5)[i]`; feedback block |

**Span** (Frequency, Amount, Source):
```
span = a > 0 ? −(a·d_lo) : −(a2·d_hi)              // ms; < 0 shortens the delay
s = |span|·0.014592483
span_red = s > 1 ? 0.999999 : s < 0.3 ? 0 : (s − 0.3)·1.42857
red = span_red·lfo_on
depth_t = (f64)(span · R(red·Hz))                   // R: §3.2
```
**Feedback block** (Feedback, Invert):
```
fb = f·fb_sign;  fb_t = (f64)fb;  A = |fb|
k = A > 0.64 ? ((A − 0.64)·2.77778)²·knee_k : 0
g = 0.92 − k                                        // wet compensation
wet_t = (f64)(g·mix_c);  dry_t = (f64)((g − 1)·mix_sq + 1);  flags |= 0x1c
```

### 2.3 Gains

`dry = 1 − (1 − g)·Mix²`, `wet = g·Mix·(2 − Mix)`. With no feedback compensation
(`g = 0.92`), Mix 0 gives the input unchanged (bit-exact), Mix 50 % gives 0.98 dry / 0.69
wet, and **Mix 100 % gives 0.92 dry + 0.92 wet**: the effect never goes fully wet. Feedback
above 40 % (`(2 − v)v > 0.64`) lowers `g`, down to 0.59 at 100 % (0.62 inverted).

### 2.4 `process()` (0x101a2b548) **[exact]**

Per host block (Maschine's own blocks are **128 samples**: the per-block smoothing stop
nulls only with 128, see §5):
1. `SyncableLFOHelper::newFrame(tempo, playing, songPosition)` gives the rate `Hz`
   (docs/fx-filter.md §4, with 11 rates: free `pos = 10·v`, `i = clamp(rnd(pos − 0.5), 0,
   9)`, `s = fma(1 − f, r[i], f·r[i+1])`, `Hz = 1/(2s)`; synced `beats = 4·r[clamp(rnd(10v),
   0, 10)]`, `Hz = tempo·(1/60)/beats`). Off (Envelope), the previous rate stays.
2. Kernel: LFO-rate input `= Hz`, depth target recomputed with this `Hz` (§2.2), flags |= 2,
   `inc = Hz·(1/sr)`. If the helper re-phased (synced, transport playing), the LFO phase
   is set to the helper's phase (`frac(position/beats + 0.5)`). The position is the host
   block's start rounded to 240000 ticks per quarter, the same for every 128-sample block
   of the host block (docs/fx-filter.md §4) **[exact, by test]**.
3. **Smoothing stop**: for centre (bit 1), depth (2), feedback (4), dry (8), wet (0x10), the
   bit clears when `|(f64)current_f32 − target| ≤ 1e-4`. While any bit is set, all five
   smoothers step (§3.3); when none is, they freeze.
4. Per sample: `in L → +0xc`, `in R → +0x50`, `processAudio()`, `+0x5c → out L`, `+0x68 →
   out R`. In place, no latency.

---

## 3. DSP (`FXCORE::FlangerCore`)

### 3.1 `endInit()` (0x101a2a428) **[exact]**

```
steps   = rnd((f32)(sr / 8820))          // 5 at 44.1/48 k, 11 at 96 k
period  = steps − 1                      // control counter limit
ctl     = rnd(sr) / steps                // integer division
L       = (0.310295·0.9602356 + 0.311434·0.039764404) + (−61)
coef    = 1 − (f64)exp2((f32)((f64)L / max((f64)ctl/1000·100 − 1, 1)))   // all five smoothers
ms      = sr/1000;  1/sr;  follower level = 0.1;  LFO phase = phase input (0 when fresh)
HP:  w = (3.14159/sr)·min(sr·0.494, 45);  t = tan≈(w);  b = 1/(t + 1);  a = t − 1
```
`exp2` is the Chorus's table-driven `2^x` (docs/fx-chorus.md §3.1; same 1023-entry table,
0x10dc2c784). The smoothers reach −365 dB after 10 ms (1/e ≈ 0.24 ms). `tan≈` is a
rational approximation: `w·P(w²)/Q(w²)` with `P = 1 − 0.166667x + 0.00833333x² −
0.000198409x³ + 2.7526e-6x⁴ − 2.39e-8x⁵` and `Q = 1 − 0.499923x + 0.0416666x² −
0.00138884x³ + 2.47609e-5x⁴ − 2.605e-7x⁵` (Horner, `f32`). Every derived value of §2.2 is
computed from the inputs, the smoothers start at their targets, flags = 0x1f, and one
`processAudio` sample runs on silence with a control step (it advances the LFO once and the
write index to 1; the high-pass states become 1e-18).

### 3.2 Centre delay and depth **[exact]**

```
x = (v·(|v|·0.33 + 0.67)·113 + 10)·0.0833291       // pitch/12, 10..123 semitones
n = rnd(x);  f = x − n
p = (((f·0.056 + 0.242)·f + 0.693)·f + 1)·2^n
d = p < 0.001 ? 122288 : 122.288/p                  // ms
```
Frequency 0 → 68.63 ms, 50 % → 4.50 ms, 100 % → 0.100 ms (≈ 14.6 Hz … 9.95 kHz as a first
comb notch `1/d`). `R(x)`, x clamped to 0..10, interpolates linearly in
`{1, 1, 1, 0.85, 0.75, 0.55, 0.5, 0.45, 0.4, 0.35}` (0x10dc2c75c; index `rnd(x − 0.5)`,
both indices clamped to 9 unsigned). It shrinks the depth for fast LFOs over spans wider than
30 % of the range (`red·Hz` > 2).

### 3.3 Control tick

The counter increments while below `period`; otherwise it resets to 0 and, if any flag bit
is set, each smoother does `cur += coef·((target − cur) + 1e-100)` (`f64`) and the kernel
uses `(f32)cur` **[exact]**.

### 3.4 Per sample (`processAudio`, 0x101a2b920) **[exact]**

Two 4096-sample lines (L at +0x248, R at +0x4248), write index `w` (masked 0xfff).
`zL, zR`: high-pass states; `oL, oR`: last high-pass outputs.
```
w' = (w + 1) & 0xfff;  control tick (§3.3)
clip(x, v) = x + c·(1 − |c|·0.14880952),  c = clamp(v, −3.36, 3.36)
bufR[w] = clip(xR, oR·fb)
modulation:
  LFO (Source ≠ Envelope):  ph in [−0.5, 0.5)
     mL = 4·|ph·0.5|                          (= 2|ph|, a triangle 0..1)
     q = ph + st_offset;  q −= rnd(q);  mR = 4·|q·0.5|
     ph += inc;  ph −= rnd(ph)
  Envelope:  lvl = max(|xL|, |xR|)
     if lvl > level: level = lvl, held = 0
     else: held += 1 (post); if hold ≤ held_before: level = max(level − fall, 0)
     mL = min(level + 1e-6, 1);  mR = st_env·mL
bufL[w] = clip(xL, oL·fb)
dS = clamp((centre + mS·depth)·d_scale·ms, 0, 4094)  // samples, per side
yS = cubic read at dS behind w (below)
h  = b·(yS − zS·a);  zS' = h + 1e-18;  oS = h − zS     // DC blocker, ≈ 45 Hz
outS = xS·dry + wet·oS
```
Cubic read (Catmull-Rom, the Chorus's formula with other indices): `n = rnd(d − 0.5)`,
`t = d − n`, `A = buf[w−n]`, `B = buf[w−n−1]`, `C = buf[w−n+1]`, `E = buf[w−n−2]`;
`c1 = (B − C)·0.5`, `c2 = (E − A)·0.5`, `ab = A − B`, `s = ab + c1`, `q = ab + (c2 + s)`,
`y = A + t·(c1 + t·(t·q − (s + q)))`. So the delay is exactly `d` samples (`d = 0` reads the
sample just written). Right is computed before left. The channels are independent with the
LFO; the follower takes both inputs.

Behaviour: the sweep runs from the centre delay toward 0.1 ms (Amount > 0, linear in
Amount) or toward 68.6 ms (Amount < 0, Amount²). The LFO is a triangle, the right side
`Stereo²/2` cycles ahead (up to 180°); with the envelope, the right side's sweep is scaled by
`1 − 0.02·Stereo`. Invert flips the feedback sign and **halves all delays**. Feedback
(`(2 − v)v`, so 75 % at 50 %) is soft-clipped at ±1.68 and taken after the DC blocker.

### 3.5 Envelope follower times (Shape) **[exact]**

`u = 2·Shape − 1`, `s = 1 − |u|`. If `u ≤ 0.01`: hold `(s·2000 + 1000)·0.115385` ms
(115 ms at Shape 0 → 346 ms at 50 %), fall time 57.69 ms. Else: hold 11.54 ms, fall time
`(s·16000 + 1000)·0.115385` ms (1961 ms just above 50 % → 115 ms at 100 %).
`hold = rnd(hold_ms·sr/1000)` samples; `fall = 1/(fall_ms·sr/1000)` per sample (1 if that is
≤ 1). The level rises instantly to the input peak, holds, then falls linearly (1 → 0 in the
fall time).

---

## 4. Implementation notes

- `Flanger::set` pushes all parameters (constructor order) and resumes on the first call;
  later calls send only changed parameters as events, so the lines, LFO and smoothers keep
  running.
- `process` splits host blocks into 128-sample blocks and does §2.4 steps 1–3 per block.
  `set_tempo` gives the synced rate; `set_transport` gives the song position for the synced
  re-phase: the host block's start, held (not advanced) for all its 128-sample blocks and
  rounded by `fx::song_position`.
- The LFO helper is the same as the Filter's (copied, with 11 rates and phase offset 0.5).
  The `exp2` table is the same as the Chorus's (copied).
- No `mul_add` except in the helper's free rate (an `fmla` in Maschine).

## 5. Null tests

Sources (read-only): "Big Stream.mxprj" sound 0/3 slot 2 (version 0, no Source; stored
frequency 0.78, amount 0.94, speed 0, shape 0.78, stereo 0.45, feedback 0.04, invert on,
mix 0.13) and "Anti Blocker.mxprj" sound 2/0 slot 3 (frequency 0, source LFO, amount −0.66,
speed 0.27, shape 0.5, stereo 0.5, feedback 0.55, invert off, mix 0), both in
`/Users/Shared/Maschine 2 Library/Projects/`. Made with `maschine-ref make … [field=value]`
(unlisted fields keep the project's values), rendered 3 s at −12 dBFS with `--preroll 0`,
compared against `fx-null` on the captured input. **−240 dB = bit-identical.** Synced cases
render with Maschine's default tempo 120 (`FX_TEMPO=120`) unless `--tempo` is given.

| source | settings | input | sr | residual L / R (dB) |
|---|---|---|---|---|
| Big Stream | project values (invert on) | noise | 44.1 k | −240 / −240 |
| Big Stream | all defaults (freq 0.5, amount 0.75, speed 0.5, shape/stereo/fb/mix 0.5, invert off) | noise | 44.1 k | −240 / −240 |
| Big Stream | all defaults | noise | 48 k | −240 / −240 |
| Big Stream | frequency 0 / invert off | noise | 44.1 k | −240 / −240 |
| Big Stream | frequency 1 / invert off | noise | 44.1 k | −240 / −240 |
| Big Stream | amount −1 / invert off | noise | 44.1 k | −240 / −240 |
| Big Stream | amount −0.4, frequency 0.3 | noise | 44.1 k | −240 / −240 |
| Big Stream | amount 0 | noise | 44.1 k | −240 / −240 |
| Big Stream | feedback 1, invert off | noise | 44.1 k | −240 / −240 |
| Big Stream | feedback 1, invert on | noise | 44.1 k | −240 / −240 |
| Big Stream | speed 1 (8 Hz), amount 1, frequency 0.1 (depth reduction; 128-sample blocks) | noise | 44.1 k | −240 / −240 |
| Big Stream | speed 0.93, amount 0.9, frequency 0.05, invert, fb 0.8 | noise | 48 k | −240 / −240 |
| Big Stream | speed 0.81, amount −0.95, frequency 0.6, fb 0.3, mix 0.7 | noise | 44.1 k | −240 / −240 |
| Big Stream | speed 0, stereo 1 | noise | 48 k | −240 / −240 |
| Big Stream | mix 0 | noise | 44.1 k | −240 / −240 |
| Big Stream | mix 1, feedback 0.9 | sweep | 44.1 k | −240 / −240 |
| Big Stream | frequency 0, amount −1, fb 0.7, mix 1 (reads clamp at 4094) | noise | 96 k | −240 / −240 |
| Big Stream | speed 0.8, amount 0.6, mix 0.6 (control rate 3) | noise | 22.05 k | −240 / −240 |
| Anti Blocker | project values, mix 0.5 | noise | 44.1 k | −240 / −240 |
| Anti Blocker | LFO Sync, speed 0.5, amount 0.8, mix 0.6 (tempo 120) | noise | 44.1 k | −240 / −240 |
| Anti Blocker | LFO Sync, speed 0 (16 bars), amount −0.8 | noise | 48 k | −240 / −240 |
| Anti Blocker | LFO Sync, speed 1 (1/16), amount 0.9, freq 0.2, mix 1 | noise | 44.1 k | −240 / −240 |
| Anti Blocker | LFO Sync, speed 0.7, `--tempo 96` | noise | 48 k | −240 / −240 |
| Anti Blocker | Envelope, amount 0.8, mix 0.6 | sweep | 44.1 k | −240 / −240 |
| Anti Blocker | Envelope, amount 1, shape 0, mix 1 | impulse | 44.1 k | −240 / −240 |
| Anti Blocker | Envelope, amount −1, shape 1, stereo 1, mix 1 | impulse | 48 k | −240 / −240 |
| Anti Blocker | Envelope, amount 0.5, shape 0.3, fb 0.9, invert | noise | 44.1 k | −240 / −240 |
| Anti Blocker | Envelope, amount −0.5, shape 0.7, stereo 0.2 | sine 220 | 48 k | −240 / −240 |
| Anti Blocker | stereo 1, mix 1 | noise | 44.1 k | −240 / −240 |
| Anti Blocker | stereo 0, mix 1, speed 0.75 | noise | 44.1 k | −240 / −240 |
| Anti Blocker | odd values: 0.123 / 0.377 / speed 0.611 / stereo 0.29 / fb 0.71 / invert / mix 0.83 | noise | 48 k | −240 / −240 |
| Anti Blocker | mix 1, feedback 1 | impulse | 44.1 k | −240 / −240 |
| Anti Blocker | free LFO, `--play --tempo 120` | noise | 44.1 k | −240 / −240 |
| Anti Blocker | LFO Sync, speed 0.7, fb 0.55, freq 0.2, amount 0.9, mix 1, `--play --tempo 120` (`fx-null` with `FX_PLAY=1`) | noise | 44.1 k | −240 / −240 |
| Anti Blocker | the same at `--play --tempo 90` | noise | 44.1 k | −240 / −240 |
| Anti Blocker | the same with speed 1, `--play --tempo 137.5` | noise | 44.1 k | −240 / −240 |
| Anti Blocker | LFO Sync, speed 0.4, amount −0.8, freq 0.4, mix 0.6, `--play --tempo 120` | noise | 48 k | −240 / −240 |
| Anti Blocker | LFO Sync, speed 0 (16 bars), amount 0.8, freq 0.1, mix 0.7, `--play --tempo 137.5` | noise | 44.1 k | −240 / −240 |
| Anti Blocker | LFO Sync, speed 0.7, tempo 90, stopped | noise | 44.1 k | −240 / −240 |

Block size: with the per-block step of §2.4 run every 16/32/64 or 512/1024 samples, the
`speed 1` case nulls only to ≈ −85 dB (the smoothing stop freezes the depth at a
slightly different value); 128 nulls every case (256 fails the `speed 0.81` case), so
Maschine's internal block is 128 samples **[exact, by test]**.

## 6. Open points

1. *(Resolved.)* Synced LFO with the transport playing: the re-phase uses the host block's
   start position (rounded to 240000 ticks per quarter) for every 128-sample block, not a
   position advancing per block (docs/fx-filter.md §4); bit-exact with `--play` (§5).
2. Parameter changes during a render are not verified (maschine-ref has no automation); the
   event and smoothing code follows the assembly, and a unit test checks that re-sending the
   same values changes nothing.
3. The QML editor (`FlangerEditor.qml`, `FlangerModel`) was not decoded.
