# Maschine's Grain Stretch: parameters and DSP

A clean-room description of Maschine 3's built-in **Grain Stretch** effect
(`NI::MASCHINE::DATA::GrainStretch` → `NI::MASCHINE::RT::GrainStretchProcessor` →
`NI::MASCHINE::FXCORE::GrainStretchCore`). `GrainStretchCore` is a compiled Reaktor-Core
kernel: a flat struct of about 4 MB (inputs, state and two 2^19-sample buffers), with
`endInit()`, `processAudio()` (one sample per call) and one event handler,
`processEventInTrans()`. The processor computes the other parameters' events inline.

It was written from Ghidra output in `out/decomp/grain_stretch/` (`INDEX.tsv` maps files to
functions) and from the arm64 disassembly of the whole class, saved as
`out/decomp/grain_stretch-all.s` (0x101a3c048–0x101a3e98c). Where they disagree, this spec
follows the assembly. Addresses are arm64 VM addresses in `analysis/maschine_arm64`. The
DECKARD port is `crates/engine/src/fx/grain_stretch.rs`. It nulls **bit-exactly** against
Maschine (§5).

**Confidence.** **[exact]**: formula and constants read from code or assembly.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation: `v` is a parameter's stored value. All arithmetic is `f32`, with no fused
multiply-adds. `rnd(x)` adds `+0.5` (when `x ≥ 0`, NaN included) or `−0.5` to `x` **in
`f64`**, then truncates (`fcvtzs`: saturating, NaN → 0).

There is **no random number generator** in this effect. The grains are periodic.

---

## 1. Parameters

### 1.1 Module

Module name "Grain Stretch" (`s_aModuleInfo`, built in
`__GLOBAL__sub_I_Modules_Effects_GrainStretch_ALL.cpp`, 0x101a41058). One page:
`s_aPageNames = { "Main" }` **[exact]**. The QML editor (`Modules/FX/GrainStretchEditor.qml`)
is titled "GRAIN STRETCH". It has Stretch, On, Pitch on the top row and Loop, Mix, Size, Link
on the bottom row. A bracket ties Pitch and Size to Link.

### 1.2 Parameters in constructor order

The constructor is `GrainStretch::GrainStretch()` at 0x101a3c170 (`0019.c`). Labels are wide
strings at 0x104d2d6b8 … 0x104d2d738 **[exact]**.

| obj off | mxprj field | label | type | section | default | range | RT | display |
|---|---|---|---|---|---|---|---|---|
| 0xa8 | `on` | **On** | bool | "Master" | false | 0/1 | bool 0 | on/off |
| 0xb0 | `stretch` | **Stretch** | float | "Time" | 0 | 0..1 | float 0 | `v·100`, `"%.1f %%"` (0x10dce2238) |
| 0xb8 | `loop_length` | **Loop** | EnumParameter | – | **8** | 0..8 | enum 0 | options `1 2 3 4 6 8 12 16 32` |
| 0xc0 | `pitch` | **Pitch** | float | "Pitch" | 0.5 | 0..1 | float 1 | `FloatLinearConverter(0, 200, "%.1f %%")`: `200·v` % (100 % at the centre) |
| 0xc8 | `link` | **Link** | bool | – | false | 0/1 | bool 1 | on/off |
| 0xd0 | `size` | **Size** | float | – | 0.25 | 0..1 | float 2 | `v·100`, `"%.1f %%"` |
| 0xd8 | `mix` | **Mix** | float | "Output" | 1.0 | 0..1 | float 3 | `v·100`, `"%.1f %%"` |

The Loop options are the wide strings `"1","2","3","4","6","8","12","16","32"` (vector at
0x10e306098, 9 entries). The project stores the **index**, and the kernel uses the index,
not the label (§3.1): index `n` gives a loop of `n/4` beats. So the default `"32"` (index 8)
is 2 beats, and index 0 (`"1"`) never wraps. The labels do not match what the DSP does
**[exact]**. (They would fit sixteenths with a different mapping. No such mapping exists in
the code.)

### 1.3 Knob layout (`getParameterByPosition`, 0x101a3d04c; jump table 0x104d2d654)

| pos | page | parameter | section heading |
|---|---|---|---|
| 0 | Main | On (switch) | "Master" |
| 1 | Main | Stretch | "Time" |
| 2 | Main | Loop (enum selector) | – |
| 3 | Main | Pitch | "Pitch" |
| 4 | Main | Link (switch) | – |
| 5 | Main | Size | – |
| 6 | Main | Mix | "Output" |
| 7 | Main | – (empty) | |

No parameter is hidden in any mode.

### 1.4 Serialization

`GrainStretch::serialize` (0x101a3c048) loads the Module base, then 0xa8, 0xb0, 0xb8, 0xc0,
0xc8, 0xd0, 0xd8 in that order **[exact]**. In `crates/mxprj` these were `unknown_a8 …
unknown_tail` and are now `on, stretch, loop_length, pitch, link, size, mix`.

### 1.5 Value → engine (`connectParameters`, 0x101a3c77c)

| callback | idx | kernel input | handler |
|---|---|---|---|
| `processBoolParameter` | 0 On | +0x1c | inline (§3.4) |
| `processFloatParameter` | 0 Stretch | +0x54 | inline: `speed = min(1 − v, 1)` |
| `processEnumParameter` | 0 Loop | +0x3c | inline (§3.1) |
| `processFloatParameter` | 1 Pitch | +0x2c | `processEventInTrans` (0x101a3e624) |
| `processBoolParameter` | 1 Link | +0x24 | inline (§3.2) |
| `processFloatParameter` | 2 Size | +0x34 | `setSize` (0x101a3e84c) |
| `processFloatParameter` | 3 Mix | +0x10 | inline: `mix = v; mix_p = play·v; target = mix_p·on` |

---

## 2. Processor (`GrainStretchProcessor`)

- **Constructor** (0x101a3d224): stereo in, stereo out. It allocates the kernel (0x400184
  bytes, zeroed) and presets Stretch 0, Pitch 0.5, Link 0, Mix 0.5 **[exact]**.
- **`onSamplerateChanged(sr)`** stores `sr`. It takes effect at the next `resume`.
- **`resume()`** (0x101a3d3a8): it saves the seven inputs, **zeroes the whole kernel** (state
  and both buffers, so tempo and "playing" are 0 too), writes `sr`, the inputs and their
  event flags, and calls `endInit()` **[exact]**.
- **`process()`** (0x101a3ded4): per block, kernel `+0x44 ← ProcessInfo` tempo (`+0xd8`,
  BPM) and `+0x18 ← 1.0` if the transport plays (`ProcessInfo+0xd1`), else 0. Then per
  sample, `in L → +0x0c`, `in R → +0x4c`, `processAudio()`, `+0x48 → out L`,
  `+0x50 → out R` **[exact]**.

**Parameter callbacks after resume [inferred from null tests].** Maschine delivers every
parameter's callback once more **after** the first `resume`. In `maschine-ref` renders this
happens **128 samples** into the audio: 128 samples after the start with `--preroll 0`, and
128 samples after the preroll ends with `--preroll 1`. Only On has a visible effect, because
the other callbacks repeat what `endInit` computed: On sets the loop start to the current
write index and resets the loop position (§3.4). This moves where the loop reads. With the
callbacks at sample 0, some settings null and others differ by about −20 to −40 dB. With
them at 128, every case is exact (§5). The port sends them at once by default.
`GrainStretch::defer_first_events(n)` holds them back for `n` samples, and the null harness
uses `n = 128`. What sets the 128 in Maschine (the main-thread message dispatch, or a
128-sample engine sub-block) is **[unknown]**.

---

## 3. DSP (`FXCORE::GrainStretchCore`)

### 3.1 Buffers, clocks and the loop

Two buffers of 2^19 samples (L at +0x158, R at +0x200158). Indices are masked with
`0x7ffff` **[exact]**. Per sample:
```
w     = counter & 0x7ffff;  counter = rnd((f32)w + 1)        // write index
if tempo changed: bps = tempo·(1/60 = 0x3c88889a)
                  hold_at = rnd_i(rnd(8·sr / bps) + rnd(sr/1000·100)) − 1   // 8 beats + 100 ms
                  loop_n  = (f32)rnd(loop_len / bps)
loop_len = (f32)rnd(rnd(idx)) · 0.25 · sr                    // Loop event / endInit
```
(`rnd_i` takes its sign from the integer add's flags, so it is the true sum's sign even
when the add wraps.) The loop position `pos` advances by `speed = min(1 − Stretch, 1)` per
sample. Before use it wraps once if `pos ≥ loop_n` (`pos −= loop_n`). The read base is:
```
n    = rnd(pos_wrapped);  pos = pos_wrapped + speed
base = on > 0 ? start + n : (f32)w
from = (f32)(rnd(base) − lead_n)                              // lead_n = rnd(len·max(ratio − 1, 0))
```
So with On off, the grains read at the write position. A pitch-up ratio starts them `lead_n`
samples behind, which keeps them from overtaking the write position. With On, they read from
`start` (the write index when On was switched on) plus the loop position. That position
loops over `idx/4` beats at `1 − Stretch` of normal speed. Stretch 100 % holds it still, and
index 0 makes `loop_n = 0`, so it never wraps. **Hold:** while On, a counter (armed by the On
event) runs from 0. When it reaches `hold_at` (8 beats + 100 ms), the buffers stop recording
(`+0x108 = 1`) and the loop plays frozen material. Switching On off clears the hold
**[exact]**.

### 3.2 Grain length (Size, Link, Pitch)

**[exact]**:
```
size_f  = 2^(6·(1 − (s + (s² − s)·0.6666667)) − 4)          // s = Size; 4 … 1/16
           (2^x ≈ (f·(f·0.247993 + 0.708) + 1)·2^n, n = rnd(x), f = x − n)
ratio   = pitch ratio (§3.3)
base_ms = Link·(80/ratio − 80) + 80                         // 80, or 80/ratio with Link
max_ms  = Link·(−190) + 330                                 // 330, or 140 with Link
len     = (sr·0.001) · min(size_f·base_ms, max_ms)          // samples
```
Without Link, grains are 320 ms (Size 0), 190 ms (default 0.25), 80 ms (0.5) and 5 ms
(Size 1). With Link they also shrink as the pitch rises, up to 140 ms. When an event shortens
`len` (`len < previous`), each running grain's phase rate is rescaled by `old_len/len`
(`+0x13c`, `+0x150`), so it still ends on time.

### 3.3 Pitch (`processEventInTrans`)

**[exact]**:
```
b  = 2p − 1;  x = |b|·24;  i = rnd(x − 0.5);  j = rnd(i + 1)
i0 = (u32)i < 24 ? i : 24;  i1 = (u32)j < 24 ? j : 24
T  = b > 0 ? UP : DOWN
st = T[i0] + (x − i)·(T[i1] − T[i0])                        // semitones
q  = rnd(st·4)·0.25                                         // quarter tones
ratio ≈ 2^(q·0.083333336)  (cubic: f·(f·(f·0.0560 + 0.2420) + 0.6930) + 1, × 2^n)
UP   (0x10dc3300c) = 0, 0, 1, 1.5, 2, …, 11.5, 12           (+½ per step from entry 2)
DOWN (0x10dc33070) = 0, 0, −1, −2, −3, −4, −5, −7, −8, …, −12, −16, −20, …, −60
```
Pitch 0.5 gives ratio 1. Above the centre it goes up to +12 semitones, and below it down to
−60 (DOWN has no −6). Also stored: `ph_inc = max(ratio − 1, 1)` (always 1 in practice) and
`lead = max(ratio − 1, 0)`.

### 3.4 On, mix and transport

The On event sets `on = rnd(v)` and `target = mix·play·on`. Switching on also sets
`start = w` (the last write index), `pos = 0`, the hold counter to 0, and arms the hold.
Switching off clears the hold. Mix is smoothed per sample:
```
m      = m + (target − m)·c;   state = m + 1e-18 (0x219392ef)
c      = sr·0.005 < 1 ? ln2 : ln2/(sr·0.005)                 // halves the distance in 5 ms
wet    = m·(2 − m);   dry = 1 − m²
target = Mix · playing · On
```
So the effect is **dry unless On is set and the transport plays**. Stopping either fades the
wet signal out in about 5 ms.

### 3.5 Grains and output (`processAudio`, 0x101a3dff4)

A phase runs from −0.5 to 0.5 at `1/len` per sample. Grain A starts when it crosses above 0,
and grain B when it wraps. So a grain starts every `len/2` samples, and two always overlap.
A starting grain takes `pos = from`, `inc = ratio`, phase 0 and `1/len`. Per sample
**[exact]**:
```
(the clocks of §3.1)
advance both grains: phase += ph_inc, read pos += ratio   (the old values are used below)
if hold ≤ 0: bufR[w] = xR; bufL[w] = xL                   // write before reading
win(x) = 0.5 + K1·u + (K3·u⁵ − K2·u³),  u = 0.25 − |t|, t = x > 1 ? 0.5 : x − 0.5
         K1 = 0x404827fa, K2 = 0x41a49eb8, K3 = 0x422279c1      (≈ 0.5 + 0.5·sin 2πu)
read(buf, p): i = rnd(p − 0.5), f = p − i, Catmull-Rom on buf[i−1 … i+2]
outR = xR·dry + wet·(winA·read(R, a) + winB·read(R, b))
outL = xL·dry + wet·(winA·read(L, a) + winB·read(L, b))
```
The window is about −3.5e-6 at its ends, not exactly 0. The channels are independent. The
output is computed in this order, and each term is rounded on its own.

### 3.6 `endInit` (0x101a3d45c)

On the zeroed kernel, `endInit` computes everything above from the inputs, with tempo 0 (so
`loop_n` and `hold_at` stay garbage until the first sample sees the real tempo). It also
sets the write counter to 1, the phase to −0.5, `pos = speed`, `start = 0` (with On), the
mix smoother one step from 0, and the hold off and **disarmed**. It then runs one silent
sample, which only writes zeros **[exact]**. So without the second On callback (§2) the
buffer would never freeze, and the loop would start at buffer index 0.

---

## 4. Implementation notes

- `GrainStretch::set` resumes on the first call and then sends all callbacks (§2). Later
  calls send only the changed values, in `connectParameters` order. Re-sending On would
  restart the loop, so unchanged values are skipped.
- `set_tempo` gives the BPM, and `set_transport(Some(_))` means playing. Without
  `set_transport` (as in `fx-null`), Grain Stretch passes audio through dry.
- Not verified against Maschine: parameter changes during a render, because `maschine-ref`
  has no automation. The handlers follow the assembly.
- Memory: 2 × 2 MB buffers per instance (boxed).

## 5. Null tests

Source: `/Users/Shared/Maschine 2 Library/Projects/Plate Tectonics.mxprj`, sound 5/4 slot 0
(Grain Stretch at defaults), made with `maschine-ref make … 5 4 0 [field=value…]`. Rendered
with `--preroll 0 --play --tempo T`. The comparison is against DECKARD in a scratch copy of
`fx-null` that calls `set_transport` per block (`FX_PLAY`) and `defer_first_events(128)`
(`FX_EV`): `scratchpad/fx-grain_stretch/gsnull`. **−240 dB means bit-identical.**

| case | sr | tempo | input | settings | residual L/R |
|---|---|---|---|---|---|
| defaults (On off) | 48k | 120 | noise 3 s | – | −240 / −240 |
| On | 44.1k | 120 | noise 3 s | on | −240 / −240 |
| On, past the hold (4.1 s) | 48k | 120 | noise 6 s | on | −240 / −240 |
| Stretch 50 %, past the hold | 44.1k | 100 | noise 6 s | on, stretch 0.5 | −240 / −240 |
| Stretch 100 % (frozen) | 48k | 137.5 | noise 5 s | on, stretch 1 | −240 / −240 |
| project-like | 48k | 120 | noise 3 s | on, stretch .77, pitch .43, size .56 | −240 / −240 |
| Pitch min / max | 44.1k | 120 | noise 3 s | on, pitch 0 / 1 | −240 / −240 |
| Link | 48k | 120 | noise 3 s | on, pitch .73, link | −240 / −240 |
| Link, Size max | 48k | 120 | noise 3 s | on, pitch .2, link, size 1 | −240 / −240 |
| Size min / max | 48k | 120 | noise 3 s | on, size 0 / 1 | −240 / −240 |
| Mix 0 / 0.5 | 48k | 120 | noise 3 s | on, mix 0 / 0.5 | −240 / −240 |
| transport stopped | 48k | 120 | noise 3 s | on, stretch .5, pitch .8 | −240 / −240 |
| sweep | 44.1k | 120 | sweep 3 s | on, stretch .3, pitch .62 | −240 / −240 |
| Link without On (dry) | 48k | 120 | noise 3 s | pitch .9, link | −240 / −240 |
| odd values | 48k | 120 | sine 440 3 s | on, pitch .123, size .877, stretch .333, mix .6 | −240 / −240 |
| 96 kHz | 96k | 120 | noise 3 s | on, stretch .4, pitch .6 | −240 / −240 |
| | 44.1k | 90 | noise 6 s | on, stretch .25, link, pitch .35, size .7 | −240 / −240 |
| Loop index 0 … 7 | 48k | 120 | noise 5 s | on, loop_length n, stretch .2 | −240 / −240 (each) |
| Size 1, 44.1k | 44.1k | 120 | noise 3 s | on, size 1, link, pitch .9 | −240 / −240 |
| `--preroll 1`, DECKARD fed 1 s of silence, callbacks 128 samples after it | 48k | 120 | noise 3 s | project-like | −240 / −240 |
| callbacks at once (DECKARD's default) | 48k | 120 | noise 3 s | project-like | −20.9 (loop start shifted) |
| callbacks at once | 48k | 120 | noise 6 s | on | −240 |
