# Maschine's Transient Master: parameters and DSP

A clean-room description of Maschine 3's built-in **Transient Master** effect
(`NI::MASCHINE::DATA::TransientDesigner` → `NI::MASCHINE::RT::KontaktDSPCoreWrapper<NI::DSP::TranDes>`).
The DSP is Kontakt's transient designer, `NI::DSP::TranDes`: a plain C++ class (not a
Reaktor-Core kernel) whose `processAudio` dispatches to one of 64 template instances
(`processAudio<Modes N>`) by six option flags.

DECKARD's port is `crates/engine/src/fx/transient_master.rs`. It nulls **bit-exactly**
against Maschine (§6).

Sources: arm64 disassembly of `analysis/maschine_arm64`, saved under
`out/decomp/transient_master-asm/`: `data.s` (DATA class, converter, serialization,
0x101a7d178..0x101a7e93c), `wrapper.s` (the RT wrapper), `init.s` (static initializers) and
`trandes.s` (all of `NI::DSP::TranDes`, 0x10264eb70..0x10265fb60). Ghidra was not needed.
The VST3 plug-in (`Maschine 3.vst3`, which `maschine-ref` runs) has the identical `TranDes`
code (mode 0 compared instruction by instruction).

**Confidence.** **[exact]**: read from the assembly and confirmed by the null tests.
**[inferred]**: reconstructed from partial evidence. **[unknown]**: not recovered.

Notation: `v` is a parameter's stored value, `n` its normalized value (0..1), `sr` the sample
rate (`f32`). `k(τ) = ln2 / (sr·τ)` with `ln2 = 0x3f317215`. All arithmetic is `f32`;
`fma(a, b, c) = a·b + c` with one rounding (`fmadd`), used where the text says so.

---

## 1. Parameters

### 1.1 Stored fields

`TransientDesigner::TransientDesigner()` (0x101a7d218) inserts the parameters with
`TransactionObjectContainer::insert(label, default, min, max)` **[exact]**. Labels are wide
strings at 0x104d36b9c.. .

| obj off | mxprj field | label | section | default | range | RT idx | display converter |
|---|---|---|---|---|---|---|---|
| 0xa8 | `attack` | **Attack** | – | 0 | −1..1, bipolar | 0 | `ScaleNormalizedToPercentOneDecimal` (`v·100`, `"%.1f %"`: −100.0..100.0 %) |
| 0xb0 | `sustain` | **Sustain** | – | 0 | −1..1, bipolar | 1 | percent, one decimal (as Attack) |
| 0xb8 | `input_gain` | **Input Gain** | "Depth" (see note) | 0 | −1..1 | 2 | `TransientDesignerGainConverter`: `v·(v < 0 ? 24 : 6)` dB, `"%.1f dB"` (−24.0..+6.0 dB) |
| 0xc0 | `limit` | **Limit** | – | **true** | bool | 3 | on/off |

Notes **[exact]** unless marked:
- `setBipolar(true)` is called for Attack and Sustain only. Both get a `setFuncProcessWheel`
  (function 0x1002d6fa4, not decoded) and `+0x1d0 = 2.0` (Input Gain also gets `+0x1d0 = 2.0`);
  the meaning of `+0x1d0` is **[unknown]** (probably the encoder resolution).
- The only `setSectionName` call names the section of **Input Gain** "Depth" (wide string
  at 0x104d36c1c, length 5). It is the same word the Gate uses; it may be a copy-paste in
  NI's code **[inferred]**.
- `TransientDesignerGainConverter::convertFrom(dB) = dB / (dB < 0 ? 24 : 6)`.

### 1.2 mxprj serialization

`iserializer<…, TransientDesigner>::load_object_data` (0x101a7dbe0) reads the Module base,
then **+0xb8, +0xa8, +0xb0, +0xc0** (Input Gain, Attack, Sustain, Limit) with no version
branches **[exact]**. `crates/mxprj/src/types/module.rs` names them `input_gain, attack,
sustain, limit` (were `unknown_b8, unknown_a8, unknown_b0, unknown_c0`).

### 1.3 Pages

`s_aPageNames` (`__GLOBAL__sub_I_Modules_Effects_TransientDesigner_Objects_ALL.cpp`,
0x101a7ff9c) has one page, **"Main"** (0x104d36b44). The module info name is
"Transient Master". `getParameterByPosition` (0x101a7d8d0; jump table 0x104d36b30) **[exact]**:

| pos | page | parameter |
|---|---|---|
| 0 | Main | Input Gain |
| 1 | Main | Attack |
| 2 | Main | Sustain |
| 3 | Main | Limit |
| 4–7 | Main | – (empty) |

There are no meter functions.

---

## 2. Wrapper (`KontaktDSPCoreWrapper<TranDes>`)

- Constructor (0x101a7e73c): stereo in / stereo out; allocates `TranDes` (+0xa0), sets its
  channel counts (+0xc, +0x10) to 2, calls `onInit` and `onSetSampleRate(44100, 128)`.
- `onSamplerateChanged(sr)` → `onSetSampleRate(sr, 128)`. `resume` → `onReset`, `onResume`.
  `suspend` → `onSuspend`.
- `processParameter(idx, _, x)` → `TranDes::setParameter(idx, x)`. The DATA constructor
  binds each parameter's automation to it through `NumericParameter::getNormalized`:
  `n = (f32)((f64)(v − min) · (1/(max − min)))` = `(v + 1)·0.5` for the floats, and 0/1 for
  Limit **[exact]**. It also calls `setParameter(4..7, 0)` once (options off, §3.3).
- `process` (0x101a7e93c): a counter (+0xa8, starting at 0) of samples left in the control
  block. When it is 0, `processControlRate()` runs and the counter is set to 128. Each call of
  `processAudio` gets `min(remaining, counter)` samples. Blocks therefore cross `process` calls.

There is no latency.

---

## 3. `TranDes` control

### 3.1 `onSetSampleRate(sr, 128)` (0x10264ecc0) **[exact]**

```
ramp_n   = max(trunc(sr·0.02/128 + 0.5), 1)        (+0x248; 7 at 44.1k, 8 at 48k)
ramp_inv = 1/(f32)(ramp_n·128)                       (+0x24c)
coefficients k(τ):
  +0x17c 11.8 µs (0x3745f89d)   +0x180 31.3 µs        +0x184 = +0x188 = +0x194 = +0x198  66 ms
  +0x18c 6.77 ms (0x3bddd6e0)   +0x190 8.124 ms       +0x19c 8.031 ms   +0x1a0 7.795 ms
  +0x1a4 = +0x1ac 50 ms         +0x1a8 0.5 ms         +0x1b0 0.12 ms    +0x1b4 1.7 s
s9 = 1/sr
HP10: g = tanf(s9·31.415928 (0x41fb53d2 = 10π));  b0 = 1/(g+1), b1 = −b0, a1 = (g−1)·b0   (+0x48..+0x50)
HP4:  g = tanf(s9·12.566371 (0x41490fdb = 4π));   same form                               (+0x154..+0x15c)
LP300: g = tanf(s9·942.4778 (300π)); b0 = g/(1+g), b1 = g (sic), a1 = (g−1)/(g+1)          (+0x168..+0x170)
```

`tanf` is the system libm's (Rust's `f32::tan` calls the same function on macOS). The
slots +0x180, +0x188, +0x190, +0x198, +0x1a0 and the 300 Hz low-pass are used only by
option bit 1 ("variant B", §4.3), which Maschine never sets.

### 3.2 Parameter smoothing **[exact]**

Eight slots: target (+0x220), current (+0x1e0), per-sample step (+0x200). Slot 0 Attack,
1 Sustain, 2 Input Gain, 7 an output fade (set to 1 by `onResume`, 0 by `onSuspend`;
read only by option bit 4); slots 3–6 are unused. A flag `changed` (+0x240) and `ramping`
(+0x241), a counter (+0x244).

`processControlRate` (0x10264f064):
```
if changed:
    changed = false
    inc[j] = (target[j] − cur[j])·ramp_inv              (all 8)
    for each j: if (f64)(inc[j]²) < 1e-14 (0x3cd203af9ee75616): inc[j] = 0, cur[j] = target[j]
    sum = ((sq0+sq1)+sq2)+sq3 + (((sq4+sq5)+sq6)+sq7)    (sq = inc², 0 for snapped slots)
    if sum > 0: counter = ramp_n, ramping = true
else:
    counter −= 1 (i32, wrapping)
    if counter == 0: cur = target, inc = 0, ramping = false
```
While `ramping`, every sample first adds `inc` to `cur` (all 8 slots). A change therefore
glides linearly over `ramp_n·128` samples (≈ 20 ms) and then snaps. A state word (+0x250:
1, 2, 3, 4, 0) is also kept; nothing in the audio path reads it.

`onResume` copies all targets to `cur`, then sets `target[7] = 1`, the output gain
(+0x44) to 1, and `changed`. The first control block therefore starts a ramp on slot 7
(nothing reads it in the modes Maschine uses), so the first ≈ 20 ms run in the "ramping"
template, which differs only by the (zero) increments.

### 3.3 `setParameter(idx, n)` (0x10264ef0c) **[exact]**

The raw value is kept at +0x1c + 4·idx. Then:

| idx | effect |
|---|---|
| 0 Attack | `target[0] = ((n − 0.5)·120)·2` (−120..120, dB per unit of transient envelope) |
| 1 Sustain | `target[1] = ((n − 0.5)·115)·2` (−115..115) |
| 2 Input Gain | `s = n − 0.5; target[2] = (s + s)·(n < 0.5 ? 24 : 6)` (dB) |
| 3 Limit | flag +0x3c = `n > 0.5` |
| 4, 5, 6, 7 | flags +0x3d, +0x3e, +0x3f, +0x40 = `n > 0.5` (Maschine sets them to 0) |

Indices 0–2 set `changed`. While suspended (+0x08, set by the constructor and `onSuspend`,
cleared by `onResume`), every call also copies all targets to `cur` and sets `changed`.

### 3.4 Mode dispatch (`processAudio`, 0x10264f304) **[exact]**

`mode = ramping | (+0x3f)<<1 | (+0x3d)<<2 | (+0x3e)<<3 | (+0x40)<<4 | (+0x3c)<<5`. What each
bit does in the templates:

| bit | flag | effect |
|---|---|---|
| 0 | ramping | add `inc` to `cur` before each sample |
| 1 | param 6 | variant B: other envelope constants, no rectifier, 300 Hz low-pass on the transient envelope (§4.3) |
| 2 | param 4 | no attack term |
| 3 | param 5 | no sustain term (no e5/u/w) |
| 4 | param 7 | output scaled by `cur[7]` instead of +0x44 |
| 5 | param 3 = **Limit** | clip the output to ±0.9999 |

Maschine reaches only modes 0, 1 (Limit off) and 32, 33 (Limit on).

---

## 4. `TranDes::processAudio<0/1/32/33>` (per sample) **[exact]**

### 4.1 Detector

```
for c in L, R:  y_c = fma(−a1, y1_c, fma(b0, x_c, b1·x1_c))     (HP10, state +0x54/+0x94)
pk = max over c of |y_c|           (pk = |y| < pk ? pk : |y|, starting at 0)
p  = (0.0005 ≤ pk ≤ 3) ? T[i] + fr·(T[i+1] − T[i]) (fma)  :  powf(pk, 0.1)
      with s = pk·10000, i = trunc(s), fr = s − (f32)i,
      T = s_cfmapVectorPower0p1 (0x10c3dff80): T[i] = (f32)((i/10000)^0.1 in f64), i = 0..30001
e  = p·0.7 (0x3f333333)
```

### 4.2 Envelopes and gain

Six followers `env[0..5]` (+0x1b8..+0x1cc; reset by `onReset` to 0.262, 0.425, 0, 0.403,
0.403, 0.63). `follow(env, x, up, down)`: `d = x − env; env = fma(d > 0 ? up : down, d, env)`.

```
att = cur[0]
follow(e1, e, k(11.8 µs), k(66 ms))                      // +0x17c / +0x184
follow(e2, e, k(6.77 ms), k(66 ms))                      // +0x18c / +0x194
t = e1 − e2;  if t > 0.0744 (0x3d985f07): t = t·1.1417 (0x3f92233a)
t = t + (−0.0504) (0xbd4e703b);  clamp t to 0..0.12 (0x3df5c28f)
y = fma(−a1, y1, fma(b0, t, b1·x1))                      // HP4, state +0x160/+0x164
a = |y|;  d = a − e3;  e3 = d > 0 ? a : fma(k(8.031 ms), d, e3)
attack = att·e3
d = e − e4;  e4 = d > 0 ? e : fma(k(50 ms), d, e4)       // peak hold
follow(e5, e, k(0.5 ms), k(50 ms))
u = clamp((e4 − e5) + (−0.04), 0, 10000)
follow(e6, e, k(0.12 ms), k(1.7 s))
w = clamp((e6 − e5) − u, 0, 10000)
sus = cur[1]
s  = fma(sus·(−0.33) (0xbea8f5c3), e3, sus·w)
dB = (attack + cur[2]) + s
g  = ExpTable(fma(dB, 9.965784 (0x411f73da), 1200))      // = 10^(dB/20), docs/fx-eq.md §2.1
```

Clamps are `fcsel` pairs (`x < 0 → 0`, then `x > max → max`), so NaN passes through.
The exp-table lookup is `NI::AB::ExpTable` (`fx::exp_e`), the same as the EQ's.

So Attack adds up to ±120 dB per unit of the transient envelope `e3` (which is at most
≈ 0.12, so ≈ ±14 dB), and Sustain adds ±115 dB per unit of `w − 0.33·e3`, where `w` grows
while the signal decays below its recent peak.

### 4.3 Output

```
for c in L, R:
    z = fma(−a1, y1, fma(b0, g·x_c, b1·x1))             // HP10 again, state +0xd4/+0x114
    if Limit: z = z < −0.9999 ? −0.9999 : z;  z = z > 0.9999 ? 0.9999 : z   (0x3f7ff972)
    out_c = out_gain(+0x44)·z
```

The 10 Hz high-pass is always in the path, even with every control at 0 (the effect is then
a 10 Hz high-pass, plus the clip when Limit is on).

Variant B (bit 1, not reachable from Maschine) uses +0x180/+0x188 and +0x190/+0x198 for e1/e2,
`t > 0.0732 → t·1.984`, offset −0.111, clamps `y` to 0..10000 instead of rectifying, releases
e3 with +0x1a0, and feeds e3 through the 300 Hz low-pass before both uses. It is not ported.

---

## 5. Implementation notes

1. The port follows the assembly operation by operation (`mul_add` exactly where the kernel
   uses `fmadd`/`fmsub`). The 0.1-power table is generated (it equals all 30002 stored
   entries). `tanf` and `powf` are the platform's, as in Maschine.
2. Only modes 0/1/32/33 are implemented: the "ramping" bit is the `ramping` flag, Limit is the
   clip. Options 4–7 are fixed at 0 by Maschine.
3. The first `set` applies every parameter while suspended, then `onReset` and `onResume`
   (as Maschine: the project's values reach the core before it resumes). Later `set`s send
   only the changed parameters through `setParameter`, so they glide over ≈ 20 ms as in
   Maschine, and filter and envelope state are kept.
4. The 128-sample control block runs across `process` calls, like the wrapper's counter.
5. Not `metered()`: the Transient Master has no meters.

---

## 6. Null tests

Source project: `/Users/Shared/Deep Matter Library/Projects/S42 Ring.mxprj`, sound 0/0 slot 3
(Transient Master; stored input_gain 0, attack 0.3, sustain 0, limit on). Each case was made
with `maschine-ref make … 0 0 3 [field=value …]`, rendered for 4 s with `--preroll 0`, run
through `fx-null` on the captured input, and compared. "hits" is a generated test file of
drum-like hits (kicks, snares, hats; −36..+3 dBFS, R = L·0.3..1) followed by a quiet tone on
a DC offset; "burst" is 1 s of 200 Hz at −12 dBFS and 0.3 s at −26 dBFS after silence.
Scripts: `t.sh`, `bin/gen.py` in the session scratch dir `…/scratchpad/fx-transient_master/`.
Residuals are L / R RMS; **−240 dB means bit-identical**.

| case | input | sr | parameters (others = project) | residual |
|---|---|---|---|---|
| project values | hits | 44.1k | – | −240 / −240 |
| project values | hits | 48k | – | −240 / −240 |
| defaults | hits | 44.1k | all 0, limit on | −240 / −240 |
| defaults | hits | 48k | all 0, limit on | −240 / −240 |
| neutral | hits | 44.1k | attack 0, limit off | −240 / −240 |
| attack max | hits | 44.1k | attack 1, limit off | −240 / −240 |
| attack min | hits | 44.1k | attack −1, limit off | −240 / −240 |
| attack mid | hits | 48k | attack 0.5 | −240 / −240 |
| sustain max | hits | 44.1k | attack 0, sustain 1, limit off | −240 / −240 |
| sustain min | hits | 48k | attack 0, sustain −1, limit off | −240 / −240 |
| sustain mid | hits | 44.1k | attack 0, sustain −0.4 | −240 / −240 |
| gain max | hits | 44.1k | attack 0, gain 1, limit off | −240 / −240 |
| gain max + limit | hits | 48k | attack 0, gain 1 | −240 / −240 |
| gain min | hits | 44.1k | attack 0, gain −1 | −240 / −240 |
| gain −0.5 | hits | 44.1k | attack 0, gain −0.5, limit off | −240 / −240 |
| odd values | hits | 48k | attack 0.37, sustain −0.61, gain 0.23 | −240 / −240 |
| odd values 2 | hits | 44.1k | attack −0.8, sustain 0.9, gain −0.33, limit off | −240 / −240 |
| everything max | hits | 44.1k | attack 1, sustain 1, gain 1, limit on | −240 / −240 |
| noise | noise −12 | 44.1k | attack 0.6, sustain 0.3 | −240 / −240 |
| sweep | sweep −12 | 48k | attack −0.5, sustain 0.5, limit off | −240 / −240 |
| impulse | impulse | 44.1k | attack 1, sustain 1, limit off | −240 / −240 (peak 1e-38) |
| sine | sine 440 | 48k | attack 0.4, sustain 0.7 | −240 / −240 |
| burst | burst | 44.1k | attack 0.5, sustain 0.5, limit off | −240 / −240 (peak 1e-38) |

All cases change the signal (Maschine's output differs from its input by −9 to −64 dB RMS).
Two cases show isolated differences of ~1e-38 (subnormal level) in the decaying 10 Hz
high-pass tail, which suggests that Maschine flushes subnormals to zero; they are below
any audible or measurable level. Parameter changes in mid-stream cannot be tested with the
current tools; they follow §3.2 as decoded.
