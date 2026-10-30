# Maschine's Reverb ("Reverb (Legacy)"): parameters and DSP

A clean-room description of Maschine 3's built-in **Reverb** effect, listed in the browser
as **"Reverb (Legacy)"** (`s_aModuleInfo` name). The chain is `NI::MASCHINE::DATA::Reverb`
(parameters, persistence) → `NI::MASCHINE::RT::ReverbProcessor` (real-time wrapper) →
`NI::MASCHINE::FXCORE::FDN28Reverb` (the kernel). Unlike Chorus or EQ, the kernel is
**hand-written C++ with NEON intrinsics**, not a compiled Reaktor-Core kernel: a 28-line
feedback delay network processed as 7 groups of 4 lines (one `float32x4` each). It has
four built-in room presets (`FDN28ReverbPreset`), selected by the Room enum.

It was written from Ghidra output in `out/decomp/reverb/` (`INDEX.tsv` maps files to
functions) and from arm64 disassembly saved as `out/decomp/reverb-core.s` (the kernel,
0x101a1981c–0x101a1bb30) and `out/decomp/reverb-data.s` (`DATA::Reverb`,
`ReverbProcessor`, 0x101a6f6ac–0x101a73754). The SIMD code decompiles poorly, so this
spec follows the assembly. Addresses are arm64 VM addresses in `analysis/maschine_arm64`.
The DECKARD port is `crates/engine/src/fx/reverb.rs`. It nulls **bit-exactly** against
Maschine (§6).

As a cross-check during the port, the kernel's own functions (`FDN28Reverb()`,
`setSampleRate`, the setters, `processAudio`, and `ReverbProcessor::staticInit` for the
presets) were run from the binary in a CPU emulator (Unicorn, with `expf`, `bzero`,
`posix_memalign` and `operator new` stubbed). That emulation reproduces Maschine's renders
bit for bit, so it also served as the reference for parameter changes during playback,
which `maschine-ref` cannot automate. The scratch tool is not part of the repository.

**Confidence.** **[exact]**: formula and constants read from code or assembly, and
confirmed by the null tests. **[inferred]**: reconstructed from partial evidence.
**[unknown]**: not recovered.

Notation: all arithmetic is IEEE `f32`. `fma(a,b,c)` = fused `a·b + c` (`fmadd`/`fmla`):
used **only** where marked. Every other product and sum is rounded on its own, including
inside the polynomial approximations. `rnd(x)` = add ±0.5 (by the sign of `x`) in `f64`
and truncate (`fcvtzs`).

---

## 1. Parameters

### 1.1 Module

Module name "Reverb" (`Module::Module(L"Reverb")`), shown as "Reverb (Legacy)". One page:
`s_aPageNames = { "Main" }` (`__GLOBAL__sub_I_Modules_Effects_Reverb_ALL.cpp`,
0x101a73390) **[exact]**.

### 1.2 Parameters in constructor order

`Reverb::Reverb()` (0x101a6f86c) creates the parameters with
`TransactionObjectContainer::insert(label, default, min, max)` **[exact]**. Labels are wide
strings at 0x104d34570 … 0x104d345f8.

| obj off | mxprj field | label | section | default | range | RT idx | display converter |
|---|---|---|---|---|---|---|---|
| 0xa8 | `room` | **Room** | "Room" | 0 | enum: General, Bright, Guitar, Shatter | enum 0 | (enum text) |
| 0xb0 | `size` | **Size** | (Room) | 0.5 | 0..1 | float 0 | `ScaleNormalizedToPercentOneDecimal` |
| 0xb8 | `low` | **Low** | "Eq" | 0.5 | 0..1 | float 1 | `ScaleNormalizedToPercentOneDecimal` |
| 0xc0 | `high` | **High** | (Eq) | 0.5 | 0..1 | float 2 | `ScaleNormalizedToPercentOneDecimal` |
| 0xc8 | `pan` | **Pan** | "Position" | 0 | −1..1, bipolar | float 3 | `PanningConverter` |
| 0xd0 | `stereo` | **Stereo** | (Position) | 0 | −1..1, bipolar | float 4 | `ScaleNormalizedToPercentOneDecimal` |
| 0xd8 | `mix` | **Mix** | (Output) | 0.5 | 0..1 | float 5 | `ScaleNormalizedToPercentOneDecimal` |
| 0xe0 | `freeze` | **Freeze** | "Output" | false | bool | bool 0 | (on/off) |

- Room is an `EnumParameter` over `aRoomValueList = { "General", "Bright", "Guitar",
  "Shatter" }` (built in the module's static initializer). `setAutomatable(false)` is
  called on it. **[exact]**
- `ScaleNormalizedToPercentOneDecimal` (0x10dce2238) prints `v·100` with one decimal and a
  `%` sign, so Stereo shows −100.0 % … 100.0 %. `PanningConverter` (0x10dce22c8) is the
  channel Pan's converter (docs/channel-software.md): "C" at 0, otherwise "L"/"R" with the
  amount. **[exact]** (converter objects); the exact pan text is as documented there.
- Pan and Stereo call `setBipolar(true)` and `setFuncProcessWheel(…)` and set +0x1d0 = 2.0
  (the wheel's range, as for other bipolar knobs) **[exact]** call / **[inferred]** meaning.
- Serialization (`Reverb::serialize<iarchive>`, 0x101a6f6ac): Module base, then 0xa8,
  0xb0, 0xb8, 0xc0, 0xc8, 0xd0, 0xd8, 0xe0 in that order **[exact]**. These were
  `unknown_a8 … unknown_d8, unknown_tail` in `crates/mxprj`.

### 1.3 Knob layout (`getParameterByPosition`, 0x101a70520)

| pos | parameter | section heading |
|---|---|---|
| 0 | Room (enum selector) | "Room" |
| 1 | Size | |
| 2 | Low | "Eq" |
| 3 | High | |
| 4 | Pan | "Position" |
| 5 | Stereo | |
| 6 | Freeze (on/off) | "Output" |
| 7 | Mix | |

The position table (0x104d34486) puts Freeze (+0xe0) at slot 6 and Mix (+0xd8) at slot 7,
and "Output" is the Freeze parameter's section **[exact]**. No parameter is hidden by
another.

### 1.4 Value → kernel (`ReverbProcessor`)

`connectParameters` (0x101a6fdc8) binds each parameter with `setAndCallFuncAutomate`, in
the order Room, Size, Low, High, Pan, Stereo, Mix, Freeze. The processor passes the stored
value through unchanged **[exact]**:

| parameter | processor function | kernel setter |
|---|---|---|
| Room | `processEnumParameter(0)` | `selectPreset(int)` |
| Size | `processFloatParameter(0)` | `setSize(f)` |
| Low | `processFloatParameter(1)` | `setBasstrap(f)` |
| High | `processFloatParameter(2)` | `setDamp(f)` |
| Pan | `processFloatParameter(3)` | `setSource(f)` |
| Stereo | `processFloatParameter(4)` | `setWidth(f)` |
| Mix | `processFloatParameter(5)` | `setDryWet(f)` |
| Freeze | `processBoolParameter(0)` | `setFreeze(bool)` |

---

## 2. Processor (`ReverbProcessor`)

- **Constructor** (0x101a7070c) **[exact]**: stereo in, stereo out. It allocates the kernel
  (0xb00 bytes, 128-aligned), runs `FDN28Reverb()` and then `setSampleRate(44100)`.
- `FDN28Reverb()` (0x101a198d4) sets the kernel's parameter copies: preset 0, Size 0.5,
  Low **0**, High 0.5, Mix 0.5, Pan 0, Stereo 0, Freeze off. It also sets two values that
  no parameter reaches: a "color" of 0.5 (+0xa70, the output tone filter of §3.6, flat at
  0.5) and a wet trim of 0.5 (+0xa38). A mute flag (+0xa3e) is 0 and never set. **[exact]**
- **`onSamplerateChanged(sr)`** calls `setSampleRate(sr)` when the rate differs from the
  stored one. In the renders this happens **after** the parameters are connected, also at
  44.1 kHz: the smoothers start on the loaded values, not gliding from the constructor's.
  **[exact]** (call) / **[inferred]** (the order, from the null tests).
- **`resume()` / `suspend()`** call `activate()`/`deactivate()`, which are empty. Nothing
  else clears the reverb: its state is cleared only by `setSampleRate`. **[exact]**
- **`process()`** (0x101a70ecc): it zeroes the input when the host flags it silent, then
  calls `processAudio(L, R, frames)` in place. There is no block-level state. **[exact]**
- `staticInit` (0x101a708e0) builds the four presets and hands them to
  `FDN28Reverb::init` (a global vector at 0x10dce6250). `init` also sets three stereo
  spread vectors at 0x10dce6270 (§3.1). **[exact]**

So the LFO and the lines start from zero when the sample rate is set and then run freely.
With `maschine-ref render --preroll S`, DECKARD must first process `S·sr` silent samples
(§6).

---

## 3. Kernel setup

### 3.1 Presets (`FDN28ReverbPreset`, 0xa8 bytes) **[exact]**

| field (offset) | General | Bright | Guitar | Shatter | use |
|---|---|---|---|---|---|
| +0x00 | 0.5 | 0.5 | 0 | 0.4 | low shelf: frequency spread |
| +0x04 | 0.5 | 0.5 | 0 | 0.29 | low shelf: gain spread |
| +0x08 | 16736 | 9846.205 | 7744.043 | 6809.627 | high shelf: frequency (Hz) |
| +0x0c | 0.5 | 1 | 0.51 | 0.48 | high shelf: frequency spread (×−0.5) |
| +0x10 | 0 | −0.2 | 0 | 0 | high shelf: gain at High 100 % (dB) |
| +0x14 | −14 | −3.2 | −6 | −7.6 | high shelf: gain added at High 0 (dB) |
| +0x18 | 0.5 | 0.46 | 1 | 0.37 | high shelf: gain spread (×2) |
| +0x1c | 0.285 | 0.285 | 0.478 | 0.684 | feedback at Size 0 |
| +0x20 | 0.70000005 | 0.69799995 | 0.371 | 0.21499997 | feedback range |
| +0x24 | 1 | 1 | 1 | 1 | Householder scale |
| +0x28 | 4 | 4.1 | 5.15 | 5.3 | wet gain |
| +0x2c | 0.36 | 0.36 | 0.36 | 0.36 | LFO rate (Hz) |
| +0x30 | 0.00018 | 0.00018 | 0.00018 | 0.00018 | LFO depth (s) |
| +0x34 (int) | 0 | 2 | 2 | 1 | stereo spread vector |
| +0x38 … +0xa4 | 28 delay times, 13 … 29 ms | | | | by group (4 per group) |

The 28 delay times are in `reverb.rs` (`PRESETS`). The spread vectors (lane positions):
0 = (1, −1, 1, −1), 1 = (1, 1, −1, −1), 2 = (1, 0.5, −0.5, −1).

Line `n` of the "spread" formulas is `group + 7·lane` (0x104d298f0 = (0, 7, 14, 21)), so
a quantity `x` with spread `s` takes the values `x + n·(x·s/27)` (`1/27` = 0x3d17b426)
over the 28 lines. They are accumulated group by group: `v₀ = x + (0,7,14,21)·step`, then
`v₊₁ = step + v` **[exact]**.

### 3.2 Helper approximations **[exact]**

- `exp2v(x)` (vector): `n = round-half-away(x)`, `f = x − n`,
  `p = (f·0.248 + 0.708)·f + 1` (unfused; 0x3e7df3b6, 0x3f353f7d), result `p·2ⁿ` with `2ⁿ`
  built by adding `n<<23` to the bits of 1.0.
- `exp2s(x)` (scalar, tone filter only): `n = rint(x)` (ties to even, clamped to i32),
  `p = fma(fma(f, 0.248, 0.708), f, 1)`, `p·2ⁿ`.
- `tanv(x) = x + (x³·0.33333334 + x⁵·0.1333333)` with `x³ = x·x²`, `x⁵ = x²·x³` (unfused).
  The scalar tone version uses `0x3eaaaaaa` and `fma(x⁵, 0.1333333, x³·0.3333333)`.
- `recip(x)` = NEON `frecpe` (8-bit estimate) followed by two `frecps` steps:
  `e₁ = e₀·fma(−x, e₀, 2)`, `e₂ = fma(−x, e₁, 2)·e₁`. It is not exactly `1/x`.
- The shelf amplitude for `dB` is `A = exp2v(dB·0.0830482)` = `10^(dB/40)`.
- "pi" is 0x40490fd0 (3.14159).

### 3.3 `setSampleRate(sr)` (0x101a19a24) **[exact]**

1. `sr`, `1/sr`. Freeze waits: `rnd(0.005·sr)` (to unfreeze) and `rnd(0.1·sr)` (to freeze).
2. Line lengths, the same for every preset: for each line, `d = max over presets of its
   delay` and `depth = max of the LFO depths`; `max = fma((depth + d)·sr, 1.001, 1) + 3`;
   `len = (rnd(max + 1) + 19) & ~15`. All lines share one zeroed allocation (each line
   followed by 16 floats of padding).
3. Gain smoother coefficient `c = ln2/(0.02·sr)` (`ln2` = 0x3f317215; `ln2` itself when
   `0.02·sr < 1`).
4. Control period `P = max(rnd(sr/9800), 1)` (5 at 44.1/48 k); ramp step
   `1/((sr/P)·0.05)` (ramps take 50 ms).
5. Tone filter (§3.6), low shelves (§3.4), high shelves, size/feedback (§3.5), freeze
   count, input gain target `!(frozen | mute)`, dry/wet, input balance and output matrix
   (with an extra `start = fma(0, Δ, start)`), then `selectPreset(room)`.
6. Reset: freeze count 0, `frozen = freeze`, LFO phase 0, tone state 0, all shelf states
   and line inputs 0, write indices 0, "wrapped" flags clear, control counter 1. The four
   smoothed gains are set to their targets, and the progress of every ramp to 1.

### 3.4 Shelves in the loop (`setBasstrap`, `setDamp`) **[exact]**

Every line has a low shelf and then a high shelf, with per-line coefficients.

**Low** (`setBasstrap(v)`): `u = 1 − v`, `F = expf(fma(u, 1.9061548, 1.1552453))·8.175799`
Hz (8.1758 Hz = MIDI note 0; `expf` is the C library's), `G = −5·u` dB. Per line,
`f = F + n·(F·[+0x00]/27)` and `g = G + n·(G·[+0x04]/27)`, and with `k = log2(10)/40`
(0x3daa152d), `A = exp2v(g·k)`,
`t = tanv(min(f·recip(A), sr)·(1/sr)·pi)`. Stored: `t − 1`, `recip(t + 1)`, `t`, `A² − 1`.
Range: Low 0 → 174.6 Hz, −5 dB; Low 50 % → 67.3 Hz, −2.5 dB; Low 100 % → 26 Hz, 0 dB
(flat). "Low" is a bass cut, with the corner and depth spread up to ×(1 + s) over the lines.

**High** (`setDamp(v)`): `u = 1 − v`, `D = fma(u², +0x14, +0x10)` dB (preset fields),
`f = F_h + n·(F_h·(−0.5·[+0x0c])/27)` with `F_h` = +0x08, `g = D + n·(D·2·[+0x18]/27)`.
`A = exp2v(g·k)`,
`t = tanv(min(f·A, sr)·(1/sr)·pi)`. Stored: `t − 1`, `recip(t + 1)`, `A² − 1`. General
at High 0: −14 dB above 16.7 kHz; 50 %: −3.5 dB; 100 %: flat.

### 3.5 Size, Freeze, Mix, Pan, Stereo **[exact]**

- **Size** (`setSize(s)`): `w = s·fma(|s|, −0.3, 1.3)`, `w = w·(2 − w)`,
  `feedback = fma(w, range, min)`. The Householder term is
  `h = feedback·scale·(−2/28)` (0xbd924924). Feedback at Size 0 / 50 % / 100 %:
  General 0.285 / 0.859 / 0.985, Bright 0.285 / 0.857 / 0.983, Guitar 0.478 / 0.782 /
  0.849, Shatter 0.684 / 0.860 / 0.899.
- **Feedback in use** `fb = Freeze ? 1 : feedback`. **Wet gain**: `v = clamp(fb)·19`
  (19 if `fb > 1`), `i = rnd(v − 0.5)`, `c = fma(T[i], 1 − (v − i), T[i+1]·(v − i))`
  over the table at 0x104d2991c (1 ×10, 0.984127, 0.968254, 0.952381, 0.888889,
  0.825397, 0.730159, 0.650794, 0.507937, 0.365079, 0.238095, 0.238095), then
  `wet_gain = ((0.5·c)·preset.wet)·(2/27)` and wet target `wet_mix·wet_gain`.
- **Freeze** (`setFreeze(b)`): `fb` and the wet gain change at once. The input switches
  after a wait of 0.1 s (freezing) or 5 ms (unfreezing): the input gain target becomes
  `!b`, and the input gain glides there with the gain smoother. The input gain also scales
  the **dry** path, so a frozen reverb mutes the dry signal too. Frozen from the start, the
  output is silence.
- **Mix** (`setDryWet(m)`): `dry = min(2(1 − m), 1)`, `wet_mix = min(2m, 1)`. Both are 1 at
  50 %. Mix 0 passes the input bit-exactly.
- **Pan** (`setSource(p)`): input balance `L = p < 0 ? 1 : 1 − p`, `R = p > 0 ? 1 : 1 + p`,
  ramped; it also shifts the output positions (below).
- **Stereo** (`setWidth(w)`, also run by Pan and the preset): for each lane `j`,
  `x = clamp(spread[j]·w + 0.707·p, −1, 1)` (0x3f34fdf4), left weight
  `(1 − x)(4 − (1 − x))/4`, right weight `(1 + x)(4 − (1 + x))/4`. At Stereo 0 and Pan 0
  every lane goes to both sides with weight 0.75.
- **Ramps**: the output weights and the input balance move linearly over 50 ms at the
  control rate. A setter takes `start = fma(progress, Δ, start)` (where the ramp is now),
  sets `Δ = target − start` and progress 0. Each control tick does
  `progress = min(step + progress, 1)`, `cur = start + Δ·progress` (unfused).
- **Room** (`selectPreset(i)`): LFO depth `depth·sr` and step `rate/sr`, delays `·sr`, the
  spread vector, and then the low, high, size and width code of the new preset.

### 3.6 Output tone filter **[exact]**

Fed by a value with no parameter (`color` = 0.5), so it is **flat in Maschine**:
`c = fma(color, 2, −1)`; `F = (c ≥ 0 ? 2000 : 800)·exp2s(c·(c ≥ 0 ? 8 : 12)·k)`,
`F = min(F, sr)`; `t = tan` (scalar version) of `(1/sr)·F·pi`. Coefficients
`(t − 1, 1/(t + 1))` (true division) and `(c ≥ 0 ? −1 : 1, c ≥ 0 ? 1 : t, g² − 1)`.
With `color` 0.5 the last is 0, so the filter adds `±0`. The port keeps it.

---

## 4. Per sample (`FDN28ReverbImpl::processAudio`, 0x101a1b374) **[exact]**

1. **Freeze countdown**: if the count is ≥ 0: at 0, `frozen = freeze` and the input gain
   target is `!(frozen | mute)`; then decrement.
2. **Control tick** every `P` samples (the first on the first sample): the output-weight
   ramps, then the input-balance ramp.
3. **LFO**: `p += inc`, `p −= round(p)` (half away, `frinta`); `q = (p + 0.25) − round(p)`.
   For lanes `(p, q, p, q)`: `s = v·(|v|·(−16) + 8)` (parabolic sine),
   `m = (s + 1)·depth` samples (0 … 2·depth: 0 … 0.36 ms).
4. **Shelves** per group on the input `x` (last sample's reads):
   `a = b_l·(x − s_l·a_l)`, `y = x + g_l·(t_l·(s_l + a))`, `s_l = a`;
   `b = b_h·(y − s_h·a_h)`, `y += g_h·(b − s_h)`, `s_h = b`. The lane sums
   `S = ((((((y₀ + 0) + y₁) + y₂) + … ) + y₆)`.
5. **Output**: `total = (S₀ + S₁) + (S₂ + S₃)`;
   `yL = (S₀·L₀ + S₁·L₁) + (S₂·L₂ + S₃·L₃)` and the same for `yR` with the right weights;
   through the tone filter. The gains `[in, –, dry, wet]` move by
   `g += c·(target − g)`. `out = wet·g_wet + (x·g_in)·(bal·g_dry)` per side.
6. **Lines**: line input `= feed + y·fb`, with `feed = bal·(x·g_in) + total·h` on lanes
   (L, R, L, R) for group 0, swapped to (R, L, R, L) for group 1, and so on. Delay
   `d = min(max((m + base) + 3, 4), max)`. Write at `w`, advance `w` (wrap at `len`,
   setting the line's "wrapped" flag), read at `r = w − trunc(d)` (+`len` if negative),
   `f = d − trunc(d)`, samples `s₀…s₃ = buf[r … r+3]` (past the end: from the start, or 0
   before the line has wrapped). Interpolation, in this order:
   `c1 = (s1 − s3)·0.5`, `c2 = s2 − s1`, `c0 = (s0 − s2)·0.5`, `a = c2 + c1`,
   `c2 += a`, `c0 += c2`, `b = a + c0`, `y = s2 + f·(c1 + f·(f·c0 − b))`.
   If the line has not wrapped and `w − trunc(d) < 0`, the read is 0. The result is the
   group's shelf input for the next sample.

So the input enters every line, on alternating lanes, and the network output is the sum of
all 28 line outputs after the shelves. The feedback matrix is `fb·(I − (2/28)·11ᵀ)`: a
Householder reflection, lossless at `fb = 1`. The LFO peak (`2·depth`) can exceed the
line's `max`, which then clamps the delay. That happens for the longest lines and in
Maschine too.

---

## 5. Implementation notes

- `Reverb::new(sr)` runs the constructor and `setSampleRate(44100)`. The first `set` calls
  every setter in Maschine's order and then `setSampleRate(sr)`. Later calls run only the
  setters whose values changed, as automation does, so the lines, LFO and ramps keep
  running.
- The kernel's operation order, its fused operations, and the NEON reciprocal estimate are
  kept. The low shelf frequency uses `f32::exp`, which on macOS is the same `expf` as
  Maschine's. Elsewhere it could differ in the last bit.
- Not used: side-chain, tempo, transport, meters.

---

## 6. Null tests

Source project: `/Users/Shared/Maschine 2 Library/Projects/Lost.mxprj`, sound 3/2 slot 1
(Reverb), made with `maschine-ref make … 3 2 1 room=… size=… low=… high=… pan=… stereo=…
mix=… freeze=…` (unlisted fields at their defaults 0 / 0.5 / 0.5 / 0.5 / 0 / 0 / 0.5 / 0).
Rendered for 3 s (impulses 10–12 s), at −12 dBFS, with `--preroll 0`. Compared with
`fx-null` on the captured input. Residuals are L / R. **−240 dB means bit-identical.**

| case | input | sr | parameters | residual |
|---|---|---|---|---|
| defaults | noise | 44.1 k | – | −240 / −240 |
| defaults | noise | 48 k | – | −240 / −240 |
| defaults | impulse | 44.1 k | – | −240 / −240 |
| longest tail, 10 s | impulse | 48 k | size 1 | −240 / −240 |
| Shatter, wet, 10 s | impulse | 44.1 k | room 3, size 1, mix 1 | −240 / −240 |
| Bright | noise | 44.1 k | room 1 | −240 / −240 |
| Guitar | noise | 44.1 k | room 2 | −240 / −240 |
| Shatter | noise | 44.1 k | room 3 | −240 / −240 |
| extremes | noise | 48 k | room 2, size 0.1, low 1, high 0, pan −1, stereo 1, mix 1 | −240 / −240 |
| extremes | noise | 44.1 k | room 3, size 1, low 0, high 1, pan 1, stereo −1, mix 0.3 | −240 / −240 |
| size min / max | noise | 44.1 k | size 0 / size 1 | −240 / −240 |
| low min / max | noise | 44.1 k | low 0 / low 1 | −240 / −240 |
| high min / max | noise | 44.1 k | high 0 / high 1 | −240 / −240 |
| pan | noise | 44.1 k | pan −1 / 1 / −0.3 | −240 / −240 |
| stereo | noise | 44.1 k | stereo −1 / 1 / 0.37 | −240 / −240 |
| mix min / max | noise | 44.1 k | mix 0 / mix 1 | −240 / −240 |
| freeze on (silence) | noise | 44.1 k | freeze 1 | −240 / −240 |
| mixed | sweep | 48 k | room 1, size 0.77, low 0.2, high 0.65, pan 0.25, stereo 0.6, mix 0.6 | −240 / −240 |
| | sine 440 | 44.1 k | room 2, size 0.33 | −240 / −240 |
| control period 10 | noise | 96 k | size 0.8 | −240 / −240 |
| control period 2 | noise | 22.05 k | room 3, size 0.6 | −240 / −240 |
| odd values | noise | 44.1 k | size 0.123, low 0.877, high 0.333, pan 0.111, stereo −0.777, mix 0.444 | −240 / −240 |
| Bright, wet, 12 s | impulse | 48 k | room 1, size 1, low 1, high 1, mix 1 | −240 / −240 |
| `--preroll 1`, DECKARD fed 48000 zeros first | noise | 48 k | room 1, size 0.9 | −240 / −240 |
| `--preroll 1`, input fed as is | noise | 48 k | room 1, size 0.9 | −6.4 (LFO and tail state differ) |

Parameter changes during playback, against the kernel run in the emulator (noise for half
the run, then silence; changes between 512-frame blocks). All four are −240 / −240:

| sr | script (frames, then the values changed) |
|---|---|
| 44.1 k | 10000 · 10000 mix 0.9 · 20000 |
| 44.1 k | 7000 size 0.3 · 9000 size 0.95 low 0.1 · 9000 high 0.9 pan −0.4 · 9000 stereo 0.8 room 2 · 30000 room 3 |
| 48 k | 8000 · 8000 freeze on · 8000 freeze off · 8000 freeze on size 0.2 · 30000 mix 1 |
| 44.1 k | 3000 pan 1 · 3001 pan −1 · 3003 stereo −1 · 5000 pan 0.3 stereo 0.5 · 20000 |

The emulated kernel itself matched Maschine bit-exactly on 9 renders (defaults at 44.1/48 k
and 7 parameter sets, including freeze), with the setters then `setSampleRate` run as in
§2.
