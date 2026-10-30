# Maschine's Maximizer effect: DSP specification

A clean-room description of Maschine 3's built-in **Maximizer** insert effect
(`NI::MASCHINE::DATA::Maximizer` → `NI::MASCHINE::RT::MaximizerProcessor` →
`NI::MASCHINE::FXCORE::MaximizerCore`). Written from decompiled code and disassembly of the
arm64 slice (`analysis/maschine_arm64`), plus constants read from the binary, and confirmed
**bit-exact** against Maschine by null tests (§5). Addresses are arm64 VM addresses.
Implementation: `crates/engine/src/fx/maximizer.rs`.

Sources:

| dir | contents |
|---|---|
| `out/decomp/maximizer/` (`INDEX.tsv`) | Ghidra C: `DATA::Maximizer` ctor, `getParameterByPosition`, serialize, static init, `MaximizerProcessor` (ctor, `process`, `resume`, parameter handlers, `onSamplerateChanged`), `MaximizerCore` |
| `out/decomp/maximizer-asm/core.s` | raw disassembly of `MaximizerCore` (0x101a1692c…0x101a170cc); the core is small and NEON-dense, so the spec below is read from this, not from Ghidra |

**Confidence.** **[exact]**: read directly from code or disassembly (and here, all of the DSP
is also confirmed by bit-exact null tests). **[inferred]**: a reasonable reading.
**[unknown]**: not recovered.

Notation: `sr` is the host sample rate (f32). All DSP is f32. `fma(a,b,c)` = `a·b + c`
with one rounding (arm64 `fmadd`).

---

## 1. Parameters

### 1.1 Module

- Module name `"Maximizer"`, module-info id **12** (`s_aModuleInfo`, set in
  `__GLOBAL__sub_I_Modules_Effects_Maximizer_ALL.cpp` 0x101a4e1e4). **[exact]**
- Page names (`s_aPageNames`): `"Main"`, `"Side-Chain Input"`. **[exact]**
- Processor `"Maximizer"`: 2 audio input pins × 2 channels (pin 0 = main, pin 1 =
  side-chain), 1 output pin × 2 channels (ctor 0x101a4bacc). Latency reported: 0 (no
  override), although the output is one sample late (§3.5). **[exact]**

### 1.2 Parameters in constructor order (`Maximizer::Maximizer` 0x101a4b52c)

| # | member | field (mxprj) | label | type | default | range | display | section |
|---|---|---|---|---|---|---|---|---|
| 0 | +0xa8 | **amount** (was `unknown_a8`) | `Amount` (0x104d2fca0) | float | **0.5** | 0 … 1 | `ScaleNormalizedToPercentOneDecimal` (v·100, one decimal, "%") | `Depth` |
| 1 | +0xb0 | **curve** (was `unknown_b0`) | `Curve` (0x104d2fcbc) | float | **0.5** | 0 … 1 | `ScaleNormalizedToPercentOneDecimal` | – |
| 2 | +0xb8 | **turbo** (was `unknown_b8`) | `Turbo` (0x104d2fcd4) | bool | **false** | off/on | on/off switch | – |
| – | +0xc0 | `side_chain_filter` | `DATA::SideChainFilter` (shared with Compressor/Limiter/Gate) | | | | | |

All **[exact]**. Only Amount gets a section name (`"Depth"`, built inline as a wchar
string).

### 1.3 Knob layout (`getParameterByPosition` 0x101a4ba04)

| position | page | parameter |
|---|---|---|
| 0 | Main | Amount |
| 1 | Main | Curve |
| 2 | Main | Turbo |
| 3–7 | Main | – |
| 8 | Side-Chain Input | SideChainFilter `[0]+0x1b8` (source) |
| 9 | Side-Chain Input | SideChainFilter `[1]+0x160` |
| 10 | Side-Chain Input | SideChainFilter `[4]+0x158` |
| 11 | Side-Chain Input | SideChainFilter `[2]+0x160` |
| 12 | Side-Chain Input | SideChainFilter `[3]+0x160` |

Identical side-chain page to the Limiter's (docs/fx-limiter.md §1.3). **[exact]**

### 1.4 Serialization

Class version **0** (`iserializer::version` 0x101a4c218). Order: `Module` base, Amount,
Curve, Turbo, SideChainFilter (`load_object_data` 0x101a4c0a8). **[exact]**

### 1.5 Value → core

Amount and Curve are bound with `setAndCallFuncAutomate` to
`processFloatParameter(id, …, v)` (0x101a4b8a0); Turbo to `processBoolParameter`
(0x101a4b8c8). Each stores the raw value in the core and calls the core's event handler:
**[exact]**

| param | core input | handler | effect |
|---|---|---|---|
| Amount `v` | +0x00 | `processEventInAmt` 0x101a16af8 | ramp target lane 0: `a = max(fma(v, −0.99, 1)², 1e-5)`; dirty |
| Curve `v` | +0x04 | `processEventInCrv` 0x101a16b38 | ramp target lane 1: `v`; dirty |
| Turbo `b` | +0x08 | `processEventInTurbo` 0x101a16b50 | `turbo = b`; if `b`: stage-2 follower ← (env 1.0, hold 1.0) and `prevOut ← prevIn` |
| side-chain present | +0x09 | `processEventInSidechain` 0x101a16b80 | every block: `sc = (pin 1 has ≥ 2 channels)` |

Constants: −0.99 = `0xbf7d70a4`, 1e-5 = `0x3727c5ac` (`fmaxnm`). So `a` (the
"threshold", which also sets the make-up gain, §3.4) runs from 1 (0 dB, Amount 0) to 1e-4
(−80 dB, Amount 1; the 1e-5 floor is never reached). The gain for quiet material is `1/a`
(up to +80 dB) at Curve 1 or `1/√a` (up to +40 dB) at Curve 0.

---

## 2. Processor (`MaximizerProcessor`)

- `process` (0x101a4bcc4): if side-chain pin 1 has ≥ 2 channels, its buffers run through
  `SidechainFilterCore::processAudio` in place first (docs/fx-limiter.md §5,
  `crates/engine/src/fx/sidechain.rs`); otherwise the side-chain pointers are the main
  buffers. Then **per sample**: core inputs ← (inL, inR, scL, scR), `processAudio()`,
  outputs → main buffers. **[exact]**
- `resume` (0x101a4bc70): keeps the core's input values (Amount, Curve, Turbo, sc), calls
  `beginInit(sr)`, sets all four event flags and calls `endInit()` — i.e. a full state
  reset with parameters applied at once (no ramp). It also resets the side-chain filter.
  **[exact]**
- `onSamplerateChanged` (0x101a4bc64) stores `sr` for the next `resume` and re-tunes only
  the side-chain filter; the core picks the rate up on `resume`. **[exact]**

---

## 3. `FXCORE::MaximizerCore`

A stereo-linked, per-sample peak-envelope gain rider with a hold, a variable gain curve and
a tanh clipper. Instance 0x100 bytes, 128-byte aligned.

### 3.1 Setup (`beginInit(sr)` 0x101a16950, `endInit` 0x101a16a04)

```
period = 128, counter = 129            // pair at 0x104d297d0: first sample runs a tick
isr    = 1/sr
coef(k) = (k > 1) ? ln2 : k·ln2        // ln2 = 0x3f317215
kAtt   = coef(1515.1515·isr)            // 0x44bd64d9 = 1/0.66 ms   (+0xcc)
kRel   = coef(7.6923075·isr)            // 0x40f62762 = 1/130 ms    (+0xd0)
kHold  = coef(isr·100)                  //                          (+0xd4)
(2π/sr is stored at +0xc0 but never read)
env1 = (env 0, hold 1); env2 = (env 0, hold 1)   // 0x104d14500 = (0,1,0,1) at +0xa0..+0xac
prevIn = prevOut = 0;  ramps cleared
endInit: N = max(1, (int)(sr·0.02/128 + 0.5)); rampInv = 1/(N·128)
         apply all four events (§1.5), then current = target, dirty = 1
```

All **[exact]**.

### 3.2 Parameter ramps (lanes 0 = `a`, 1 = Curve; lanes 2–3 unused)

The same scheme as the Compressor's cores (docs/fx-compressor.md §3.2): **[exact]**

```
every sample: current += inc; counter += 1
if counter ≥ 128: counter = 0; tick:
   if dirty: dirty = 0; inc = (target − current)·rampInv
             any lane with (f64)(inc²) < 1e-15: inc = 0, current = target
             if any lane still moves: countdown = N
   else:     countdown −= 1 (wrapping); if it reaches 0: current = target, inc = 0
```

`a` and `crv` below are the current lane values after this step.

### 3.3 Peak follower with hold (two instances: stage 1 `env1` +0xa0/+0xa4, stage 2 `env2` +0xa8/+0xac)

```
step(d):                                   // d = stereo-linked peak, max(|l|,|r|) as (|l|<|r| ? |r| : |l|)
  diff = d − env
  if diff > 0:            q = kAtt;  hold = 1
  else if hold > 0.5:     q = 0;     hold = hold·(1 − kHold)     // ≈ 10 ms hold
  else:                   q = kRel
  y   = fma(diff, q, env)
  env = y + 1e-30                          // 0x0da24260
  g   = fminnm(y < a ? a : y, 10000)       // 0x461c4000
```

### 3.4 Per sample (`processAudio` 0x101a16b8c)

```
ramps (3.2)
if turbo:
    g2 = env2.step(peak(sc ? (scL,scR) : (inL,inR)))
    (u, v) = prevOut
else:
    g2 = 1
    (u, v) = prevIn
d1 = (!turbo && sc) ? peak(scL, scR) : peak(u, v)
g1 = env1.step(d1)

// lanes (g2, g2, g1, g1):
r = recip(g)               // frecpe + 2 frecps:  e1 = e0·frecps(g,e0);  r = frecps(g,e1)·e1
s = sqrt(r)                // r0 = frsqrte(r); r1 = r0·frsqrts(r·r0, r0); t = r·r1; s = frsqrts(t, r1)·t;  s = 0 where r == 0
G = s + (r − s)·crv        // fmul then fadd (no fma): √(1/g) at Curve 0 → 1/g at Curve 1
(y0, y1, y2, y3) = (inL·G2, inR·G2, u·G1, v·G1)

prevOut = (y0, y1)         // stored even when Turbo is off
prevIn  = (inL, inR)

// output clipper: tanh(y) = (e^2y − 1)/(e^2y + 1), with the shared exp table E (docs/fx-compressor.md §3.1)
E_L = E(fma(y2 + y2, 86.5617, 1200));  E_R = E(fma(y3 + y3, 86.5617, 1200))     // 86.5617 = 0x42ad1f97 = 60/ln 2
outL = (E_L + −1)·recip(E_L + 1);       outR = (E_R + −1)·recip(E_R + 1)         // same recip as above
```

All **[exact]** (and null-tested bit-exact). Reading of it:

- **Amount** sets the level `a` (in linear amplitude, `(1 − 0.99·Amount)²`) below which
  the gain stops rising; above it the gain is `1/env` (Curve 1, hard levelling to ≈ 1.0)
  or `1/√env` (Curve 0, half the reduction in dB). Quiet material gets `1/a` (or `1/√a`)
  of make-up gain. Amount 0 means `a = 1`: no gain below full scale, only the clipper.
- **Turbo** puts a second identical rider in front (driven by the input or side-chain),
  and the main stage then detects on that stage's output.
- The output never exceeds ±1 (tanh). The clipper sees `y·2·60/ln 2`, so |y| beyond
  ~6.9 saturates to ±(2^20−1)/(2^20+1) (table end), and |y| small is ≈ linear.

### 3.5 Latency

The output is the **previous** input sample (or, with Turbo, the previous stage-2 output,
which is the previous input × G2): one sample late in both modes, not compensated
(latency reported 0). With a side-chain and Turbo off, the detector sees the current SC
sample, i.e. one sample of look-ahead relative to the audio. **[exact]**

---

## 4. Implementation notes

1. `recip` and `sqrt` must reproduce the NEON estimate+Newton sequences for
   bit-exactness (aarch64 intrinsics in `maximizer.rs`; other targets fall back to exact
   `1/x` and `√x`, a ≈1-ulp difference).
2. The exp table `E` is shared (`compressor::e`).
3. Parameter changes: only changed values are re-applied (like Maschine's
   `setAndCallFuncAutomate`), so re-sending Turbo = on does not restart stage 2.
4. The side-chain filter is the shared `SidechainFilter`.
5. **[unknown]**: the side-chain page labels (shared, see docs/fx-limiter.md), the QML
   editor's meters (`MaximizerEditor.qml`, `MaximizerModel` bindings only — the core
   exposes no meter).

---

## 5. Null tests

Source project `/Users/Shared/Maschine 2 Library/Projects/Plate Tectonics.mxprj`, sound
1/9, slot 0 (its stored Maximizer: Amount 0.17, Curve 0.38, Turbo on). Built with
`maschine-ref make SRC made.mxprj 1 9 0 field=value…`, rendered with `--preroll 0
--input-wav`, DECKARD via `fx-null`. (`Black Earth.mxprj` also has Maximizers but its made
projects crash `maschine-ref render`, even pass-through.) Script:
`/private/tmp/claude-501/-Applications-Native-Instruments-Maschine-3/d9bc38fd-75cf-45e9-a132-1940350f781c/scratchpad/fx-maximizer/null.sh` (session scratch).

| case | input | sr | amount | curve | turbo | SC | residual |
|---|---|---|---|---|---|---|---|
| project values | noise −12 dB | 48k | 0.17 | 0.38 | on | – | −240 dB (bit-exact), lag 0 |
| defaults | noise −12 dB | 48k | 0.5 | 0.5 | off | – | −240 dB |
| amount min | noise −12 dB | 48k | 0 | 0.5 | off | – | −240 dB |
| amount max, curve max | noise −30 dB | 48k | 1 | 1 | off | – | −240 dB |
| amount max, curve min | noise −30 dB | 48k | 1 | 0 | off | – | −240 dB |
| amount max, turbo | noise −30 dB | 48k | 1 | 1 | on | – | −240 dB |
| sweep | sweep −12 dB | 48k | 0.25 | 0 | on | – | −240 dB |
| sweep hot | sweep 0 dB | 48k | 0.75 | 0.5 | off | – | −240 dB |
| overdriven | noise +6 dB | 48k | 0.5 | 0.5 | on | – | −240 dB |
| impulse | impulse | 48k | 0.9 | 0.3 | off | – | −240 dB |
| low sine | sine 60 Hz −6 dB | 48k | 0.6 | 0.8 | on | – | −240 dB |
| 44.1 kHz | noise −12 dB | 44.1k | 0.7 | 0.2 | on | – | −240 dB |
| 96 kHz | sweep −12 dB | 96k | 0.4 | 0.9 | off | – | −240 dB |
| side-chain | sine 220 −12 / SC noise −6 | 48k | 0.6 | 0.4 | off | `--sc 1/0` | −240 dB (fx-null gain 0 dB, delay 0) |
| side-chain, turbo | same | 48k | 0.6 | 0.4 | on | `--sc 1/0` | −240 dB |
| SC filter on | sweep −12 / SC noise −6 | 44.1k | 0.8 | 0.7 | off | filter on, center 0.3, gain 0.6 | −240 dB |

Not testable with the current tools: parameter changes mid-stream (ramps, Turbo toggling);
these follow the decompiled code (§1.5, §3.2) and the ramp scheme already null-tested for
the Compressor.
