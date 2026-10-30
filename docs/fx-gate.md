# Maschine's Gate: parameters and DSP

A clean-room description of Maschine 3's built-in **Gate** effect
(`NI::MASCHINE::DATA::Gate` → `NI::MASCHINE::RT::GateProcessor` →
`NI::MASCHINE::FXCORE::GateCore`, with `NI::MASCHINE::FXCORE::SidechainFilterCore` on the
side-chain input). `GateCore` is a compiled Reaktor-Core kernel (like the Chorus's): it has
only `endInit` and one event handler (`processEventInAtt`); its per-sample code is inlined
in `GateProcessor::process`.

DECKARD's port is `crates/engine/src/fx/gate.rs`. It nulls **bit-exactly** against Maschine
(§5).

Sources: arm64 disassembly of the whole Gate unit (0x101a316a8..0x101a34e24), saved as
`out/decomp/gate-asm/gate.s` and `out/decomp/gate-asm/init.s` (static initializers). The
Ghidra run (`out/decomp/gate-names.txt`) could not lock the shared project while other
agents used it, so this spec is written from the assembly alone. The functions are short
and the assembly is unambiguous. Addresses are arm64 VM addresses in `analysis/maschine_arm64`.

**Confidence.** **[exact]**: formula and constants read from the assembly.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation: `v` is a parameter's stored value. `k = sr/1000` (samples per ms). All arithmetic
is `f32` with no fused multiply-adds unless the text says otherwise. `TINY = 1e-18`
(`0x219392ef`).

---

## 1. Parameters

### 1.1 Stored fields

The constructor is `Gate::Gate()` at 0x101a316a8. Each parameter is created by
`TransactionObjectContainer::insert(label, default, min, max)` **[exact]**. Labels are
wide-string literals at 0x104d2c2d0 (Threshold) … 0x104d2c348 (Mix).

| obj off | mxprj field | label | section | default | range | RT idx | display converter |
|---|---|---|---|---|---|---|---|
| 0xa8 | `threshold` | **Threshold** | "Depth" | 0.5623 (`0x3f0ff2e5`) | **0.1..1** | 0 | `GateThresholdDisplayConverter`: `20·log10(v²)` dB, `"%.1f dB"` (−40..0 dB; default −10.0 dB) |
| 0xb0 | `attack` | **Attack** | "Time" | 0 | 0..1 | 1 | `ScaleNormalizedToPercentOneDecimal` (`v·100`, `"%.1f %"`) |
| 0xb8 | `hold` | **Hold** | (Time) | 0.5 | 0..1 | 2 | percent, one decimal |
| 0xc0 | `release` | **Release** | (Time) | 0.25 | 0..1 | 3 | percent, one decimal |
| 0xc8 | `mix` | **Mix** | "Output" | 1.0 | 0..1 | 4 | percent, one decimal |
| 0xd0 | `side_chain_filter` | (SideChainFilter) | | | | | see docs/fx-compressor.md §1.3 |

Threshold is the only parameter whose stored value is not 0..1: the project stores the
linear value 0.1..1 itself (for example `threshold = 0.5623` in `Crush It.mxprj`).
`GateThresholdDisplayConverter` (0x101a32dc0..0x101a32e14) **[exact]**:
- `convertTo(v) = (f32)(17.371779276130074 · (f64)logf(v))`. The constant `0x40315f2ced384f29`
  is 40/ln 10, so this is `20·log10(v²)`.
- `convertFrom(dB) = (f32)exp(dB / 17.371779276130074)`.
- `getValueAsString` prints `"%.1f dB"` (wide string at 0x104d2c3a0).

The Threshold fader in `GateEditor.qml` spans −40..0 dB and carries the input meter (see
`apps/viewer/src/layout/strip/editors.rs`). Maschine shows Attack, Hold and Release as
percentages, so the physical times in §4 are never displayed.

No `setFuncProcessWheel` is set, so the encoders use the default step.

### 1.2 mxprj serialization

`iserializer<…, Gate>::load_object_data` (0x101a33068) reads the Module base, then +0xa8,
+0xb0, +0xb8, +0xc0, +0xc8 and the SideChainFilter (+0xd0) **[exact]**, with no
class-version branches. This is the order in `crates/mxprj/src/types/module.rs` (Gate
block, fields renamed from `unknown_a8..c8` to `threshold, attack, hold, release, mix`).

### 1.3 Pages

`s_aPageNames` (built by `__GLOBAL__sub_I_Modules_Effects_Gate_ALL.cpp`, 0x101a34e24):
**"Main"** (0x104d2c278) and **"Side-Chain Input"** (0x104d2c28c) **[exact]**.

`getParameterByPosition` (0x101a31fa0; jump table at 0x104d2c265) **[exact]**:

| pos | page | parameter |
|---|---|---|
| 0 | Main | Threshold |
| 1 | Main | Attack |
| 2 | Main | Hold |
| 3 | Main | Release |
| 4 | Main | Mix |
| 5–7 | Main | – (empty) |
| 8 | Side-Chain Input | Source (`SideChainFilter[0]+0x1b8`) |
| 9 | Side-Chain Input | Gain (`[1]`) |
| 10 | Side-Chain Input | Filter (`[4]+0x158`) |
| 11 | Side-Chain Input | Center Freq (`[2]`) |
| 12 | Side-Chain Input | Width (`[3]`) |

This is the same Side-Chain Input page as the Compressor and Limiter.

### 1.4 Meters

`Gate::getInputMeterValue` (0x101a32090) returns processor+0xb0. There is no gain-reduction
or output meter function **[exact]**. The meter is updated once per block in
`GateProcessor::process` **[exact]**:
`m = max(m·(1 − 4·info[+0x10])^n + 1e-36 (fmadd), peak)`, where `peak` is the block maximum
of the **signed** `max(L, R)` of the input (not absolute values). `resume` zeroes it.

---

## 2. Processor (`GateProcessor`)

### 2.1 Structure

`GateProcessor::GateProcessor(SidechainFilterCore&)` (0x101a3209c) **[exact]**:
- It has 2 audio input pins × 2 channels (pin 1 is the side-chain) and 1 output pin × 2
  channels.
- It allocates a zeroed `GateCore` (0xc0 bytes, at +0xa8) and keeps a reference to the
  module's `SidechainFilterCore` (+0xa0).
- `onSamplerateChanged(sr)` stores sr (+0x48) and calls `SidechainFilterCore::setSampleRate`.
- `suspend()` zeroes the meter.
- `resume()` (0x101a32238) reads the core's five parameter inputs, zeroes the whole core
  (+0x04..+0xbf), and stores sr at +0x00. It then writes the inputs back with their "event"
  flags set to 1.0 and calls `GateCore::endInit()` (§3.3). Finally it calls
  `SidechainFilterCore::resume()` and zeroes the meter.

### 2.2 Parameters (`processFloatParameter`, 0x101a31c6c)

These are applied immediately (Mix is smoothed inside the kernel; the others are not)
**[exact]**:

| idx | param | effect on the core |
|---|---|---|
| 0 | Threshold | `thr = v²` (+0x28); `thr_eff = v² − 0.01` (+0x68, minus +0x64 = `0x3c23d892`) |
| 1 | Attack | `+0x30 = v`, then `processEventInAtt` (§3.4) |
| 2 | Hold | `+0x38 = v`; `hold_ms = v²·497.5 + 2.5` (+0x78); `hold_eff = att_eff > hold_ms ? att_eff : hold_ms` (+0x7c); `hold_n` (§3.5) |
| 3 | Release | `+0x40 = v`; `rel_ms = v²·492.5 + 7.5` (+0x80); `hold_n` (§3.5); `rel_c = coef(rel_ms·k − 1)` (+0xac) |
| 4 | Mix | `+0x1c = v`; `mix_t = (2 − v)·v` (+0x58) |

### 2.3 `process()` (0x101a32894)

1. Main L/R are processed in place. **Side-chain is active** when input pin 1 exists and
   has more than 1 channel. In that case the SC pointers are pin 1's buffers; otherwise they
   alias the main buffers. The flag is written to the core (+0x48 and +0x14) every block.
2. When SC is active, `SidechainFilterCore::processAudio(scL, scR, n)` scales and filters
   the SC buffers in place (docs/fx-compressor.md §4).
3. The per-sample kernel (§3.2) runs, then the meter is updated (§1.4).

There is no latency.

---

## 3. `GateCore`

### 3.1 Shared kernel primitives **[exact]**

- `round(x)`: add ±0.5 (−0.5 when x < 0) in `f64`, then truncate to `i32`.
- `exp2(x)` is the Reaktor-Core approximation also used by the Chorus
  (docs/fx-chorus.md §3.1):
  - `n = round(x)`
  - `y = ((x − n) + 0.5)·1021`
  - `j = round(y − 0.5)` and `f = y − j`
  - `exp2(x) = (T[j&0x3ff]·(1 − f) + T[(j&0x3ff)+1]·f) · 2ⁿ`, with `2ⁿ` built in the
    exponent bits

  The table is at 0x10dc2eaec (1023 floats), equal entry for entry to the Chorus's
  (`2^(i/1021 − 0.5)` to 6 significant digits, 7 near-ties rounded the other way, and
  entry 1022 = √2). This was checked against all 1023 stored values.
- `coef(n) = 1 − exp2(L01 / max(n, 1))` (`fmaxnm`). This is the one-pole coefficient that
  shrinks the remaining distance to −20 dB after `n` steps.
- `L001 ≈ log2(0.01)` and `L01 ≈ log2(0.1)` are two entries of a log table at 0x10dc2ff64,
  interpolated at a constant position:
  - `L001 = (T[0]·0x3df5d000 + T[1]·0x3f614600) + (−7)`, with `T[0] = 0x3eb5d91b` and
    `T[1] = 0x3eb669cf`
  - `L01 = (T[327]·0x3eccc800 + T[328]·0x3f199c00) + (−4)`, with `T[327] = 0x3f2d7364` and
    `T[328] = 0x3f2dad43`

### 3.2 Per sample (inlined in `GateProcessor::process`) **[exact]**

```
d       = sc ? max(|scL|, |scR|) : max(|L|, |R|)       // (a > b) ? a : b
mix_s   = mix_s + mix_c·((mix_t − mix_s) + TINY)
if d > thr_eff:  count = 0;  latch = hold_n            // re-trigger (strict >)
target  = (latch > count) ? 1 : 0
diff    = target − env
env     = env + (diff > 0 ? att_c : rel_c)·(diff + TINY)
g       = mix_s·(env − 1) + 1
L, R    = L·g, R·g
count   = count + 1                                    // i32, wraps
```

The gate's envelope is linear gain (0..1). Mix crossfades between dry (Mix 0) and fully
gated (Mix 1) with the curve `v(2 − v)`. Mix 0.5 leaves 25 % of the signal when the gate is
closed.

### 3.3 `endInit()` (0x101a322f4) **[exact]**

It is called by `resume`. Every input whose event flag is set is squared into a cache
(Attack +0xb4, Hold +0xb8, Release +0xbc). Threshold (already v²) goes to +0xb0. Then:

```
L001, L01 as in §3.1                                    (+0x6c; +0x70 = +0x98 = +0xa4)
k        = sr/1000                                      (+0x84 = +0x9c = +0xa8)
thr_eff  = thr + (−0.01);  +0x64 = 0.01
rel_ms   = rel²·492.5 + 7.5;   hold_ms = hold²·497.5 + 2.5
a        = att²·249.75 + 0.25
att_eff  = a · (1 − exp2(L001/m)) / (1 − exp2(L01/m)),  m = max(a − 1, 1)   (§3.4)
hold_eff = att_eff > hold_ms ? att_eff : hold_ms
mix_t    = mix·(2 − mix)            (only if Mix's event flag is set; it always is here)
mix_c    = coef(k·100 − 1)          (+0x60: Mix glides 90 % of the way in ≈ 100 ms)
rel_c    = coef(k·rel_ms − 1)
mix_s    = mix_t;  env = 0
hold_n   (§3.5)
att_c    = coef(k·a − 1)
```

It then runs one step of §3.2 on the stored (zeroed) inputs, with the main input as the
detector. That leaves `count = 1`, and `env = att_c` or `rel_c·1e-18` depending on whether
0 > thr_eff.

### 3.4 Attack (`processEventInAtt`, 0x101a32bac) **[exact]**

```
a        = v²·249.75 + 0.25                  // attack, ms (0.25..250)
m        = max(a − 1, 1)
att_eff  = a · (1 − exp2(L001/m)) / (1 − exp2(L01/m))      (+0x74; the two lanes run as one NEON vector)
hold_eff = att_eff > hold_ms ? att_eff : hold_ms
hold_n   (§3.5)
att_c    = coef(a·k − 1)                     (+0xa0)
```

`att_eff` (≈ 2·a for long attacks) puts the attack's 40 dB length in the hold. Note that it
divides by the attack in **ms** (`a − 1`), whereas `att_c` uses samples (`a·k − 1`). This
is probably a unit slip in the kernel, but it is what the code does.

### 3.5 Hold length **[exact]**

```
extra  = max(35 − (rel_ms + hold_eff), 0)    // fmaxnm with +0
hold_n = round(k · (hold_eff + extra))       // = k·max(att_eff, hold_ms, 35 − rel_ms)
```

The gate stays open (target 1) for `hold_n` samples after the last sample above the
threshold. It is re-armed by every sample above it.

### 3.6 Physical ranges

| param | stored 0 | 0.5 | 1 |
|---|---|---|---|
| Threshold | (0.1 is the min) −40 dB | v = 0.5 → −12.0 dB | 0 dB |
| Attack (time to 90 %) | 0.25 ms | 62.7 ms | 250 ms |
| Hold | 2.5 ms | 126.9 ms | 500 ms |
| Release (time to −20 dB) | 7.5 ms | 130.6 ms | 500 ms |
| Mix | dry | 75 % gated | fully gated |

At Threshold 0.1, `thr_eff = 0.1² − 0.0100003 < 0`, so every sample re-triggers and the gate
never closes.

---

## 4. Implementation notes

1. The port follows the assembly operation by operation (no FMA; `round` and `exp2` as in
   §3.1). It nulls bit-exactly.
2. `exp2_approx` and its table are the same as in `chorus.rs`. `gate.rs` has its own
   `pub(crate)` copy (`fx::gate::exp2_approx`), because `chorus.rs` keeps its copy
   private. Either could be shared.
3. `Effect::gain_reduction` returns the current gain `g`. `metered()` stays false: the Gate
   panel has only the input meter on the Threshold fader (§1.4), not the Compressor's
   GR/output meters.
4. The first `set` applies every parameter, then `resume` (§3.3). Later `set`s apply only
   the changed parameters, through §2.2, so the envelope, the hold counter and the Mix glide
   keep their state.

---

## 5. Null tests

Source project: `/Users/Shared/Maschine 2 Library/Projects/Crush It.mxprj`, sound 1/1 slot 2
(Gate; stored threshold 0.5623, attack 0, hold 0.5, release 0.65, mix 1). Each case was made
with `maschine-ref make … 1 1 2 [field=value …]`, rendered for 4 s with `--preroll 0`, and
compared with `fx-null` on the captured input. "bursts" is a test WAV of random sine and
noise bursts (−45..0 dB, 3–300 ms, with gaps; R = L·random 0.3..1), so the gate opens,
holds and closes many times. Side-chain cases use `--sc 1/0` (sound 1/0 fed by In 2) and
`fx-null … sc.wav 0 0`. Scripts: `t.sh` and `tsc.sh` in the session scratch directory
`…/scratchpad/fx-gate/`. Residuals are L / R RMS; **−240 dB means bit-identical** (peak
difference 0).

| case | input | sr | parameters (others = project) | residual |
|---|---|---|---|---|
| project values | bursts | 44.1k | – | −240 / −240 |
| project values | bursts | 48k | – | −240 / −240 |
| defaults | bursts | 44.1k | thr 0.5623, att 0, hold 0.5, rel 0.25, mix 1 | −240 / −240 |
| noise (gate stays shut) | noise −12 | 44.1k | – | −240 / −240 |
| sweep | sweep −12 | 48k | – | −240 / −240 |
| impulse | impulse | 44.1k | thr 0.2 | −240 / −240 |
| threshold min (always open) | bursts | 44.1k | thr 0.1 | −240 / −240 |
| threshold max | bursts | 44.1k | thr 1 | −240 / −240 |
| threshold mid | bursts | 48k | thr 0.3 | −240 / −240 |
| attack max | bursts | 44.1k | att 1 | −240 / −240 |
| attack mid | bursts | 48k | att 0.5, thr 0.2 | −240 / −240 |
| attack small | bursts | 44.1k | att 0.05, thr 0.25 | −240 / −240 |
| hold & release min | bursts | 44.1k | hold 0, rel 0 | −240 / −240 |
| hold max | bursts | 48k | hold 1 | −240 / −240 |
| release min | bursts | 44.1k | rel 0, thr 0.3 | −240 / −240 |
| release max | bursts | 48k | rel 1 | −240 / −240 |
| mix min | bursts | 44.1k | mix 0 | −240 / −240 |
| mix mid | bursts | 44.1k | mix 0.5 | −240 / −240 |
| mix odd | bursts | 48k | mix 0.123 | −240 / −240 |
| odd values | bursts | 48k | thr 0.377, att 0.31, hold 0.07, rel 0.83, mix 0.77 | −240 / −240 |
| odd values 2 | bursts | 44.1k | thr 0.15, att 0.9, hold 0.2, rel 0.1, mix 0.9 | −240 / −240 |
| side-chain | noise / SC bursts | 44.1k | – | −240 / −240 |
| side-chain | bursts / SC bursts | 48k | thr 0.3, att 0.2, rel 0.4 | −240 / −240 |
| SC filter on | noise / SC bursts | 44.1k | SC filter 1, center 0.3, gain 0.6 | −240 / −240 |
| SC gain low | sweep / SC bursts | 48k | SC gain 0.2, hold 0.8, mix 0.6 | −240 / −240 |

These cases do change the signal. For example, threshold mid differs from its input by
−38 dB RMS, and with the side-chain the "noise" case opens where it was shut without it.
Parameter changes in mid-stream cannot be tested with the current tools. They follow
§2.2 exactly as decoded.
