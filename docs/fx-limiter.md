# Maschine's Limiter effect: DSP specification

A clean-room description of Maschine 3's built-in **Limiter** insert effect
(`NI::MASCHINE::DATA::Limiter` → `NI::MASCHINE::RT::LimiterProcessor` → one of two
engines: `NI::MASCHINE::FXCORE::LimiterCore` for **Legacy** mode and
`NI::DSP::HilbertLimiter` for **Transparent** mode). Written from decompiled code and
disassembly of the arm64 slice (`analysis/maschine_arm64`), plus constants read from the
binary. Addresses are arm64 VM addresses.

Sources (all kept, each directory has an `INDEX.tsv`):

| dir | contents |
|---|---|
| `out/decomp/limiter/` | Ghidra C: processor, `LimiterCore`, `HilbertLimiter`, `NI::DSP::Limiter` (unused, see §6), `SidechainFilterCore`, static init |
| `out/decomp/limiter2/` | Ghidra C: `DATA::Limiter` ctor, serialize, `getParameterByPosition`, meter getters |
| `out/decomp/limiter3/` | Ghidra C: `HannFilter`, `HilbertTransformer`, `SmoothingToolbox<N>` |
| `out/decomp/limiter-asm/` | raw disassembly (`objdump`) of the SIMD-dense functions, used to correct Ghidra (Ghidra loses the return values of `__sincosf_stret`, `log10f`, `hypotf` and the NEON lane shuffles); `hilbert_coefs.py` recomputes the Hilbert coefficients bit-exactly |

Hopper (via REA) didn't respond in this session ("unreadable result"), so the dense code was
cross-checked against the raw disassembly instead of Hopper pseudo-code.

**Confidence.** **[exact]**: formula and constants read directly from code or disassembly.
**[inferred]**: reconstructed from partial code, or a reasonable reading of it.
**[unknown]**: not recovered.

Notation: `sr` is the host sample rate (float). All DSP is `float` (f32) unless marked
`double`. `round½(x)` is `(int)(x + (x<0 ? -0.5 : 0.5))`, computed in double.

---

## 1. Parameters

### 1.1 Module

- Module name `"Limiter"`, module-info id **11** (`s_aModuleInfo`, set in
  `__GLOBAL__sub_I_Modules_Effects_Limiter_ALL.cpp` 0x101a48478). **[exact]**
- Page names (`s_aPageNames`): `"Main"` (0x104d2ea00), `"Side-Chain Input"` (0x104d2ea14).
  **[exact]**
- Processor `"Limiter"`: 2 audio input pins × 2 channels (pin 0 = main, pin 1 =
  side-chain), 1 output pin × 2 channels (`LimiterProcessor::LimiterProcessor`
  0x101a44f24). **[exact]**
- **Latency reported to the host: 0.** `LimiterProcessor` doesn't override
  `ni::ae::Processor::getLatencyInSamples()` (0x1005e5414, `mov x0,#0`), although both
  engines delay the audio (Legacy N samples, Transparent W−1 samples, see §3 and §4).
  **[exact]**

### 1.2 Parameters in constructor order

`Limiter::Limiter()` (0x101a44190) creates parameters with
`TransactionObjectContainer::insert(label, default, min, max[, format])`:

| # | field (mxprj) | label | type | default | range | display (UI converter) | section | wheel |
|---|---|---|---|---|---|---|---|---|
| 0 | `unknown_a8` → **mode** | `Mode` (0x104d2eac4) | EnumParameter | **1** (Transparent) | `Legacy`=0, `Transparent`=1 (`s_aModeValueList`) | – | `Mode` | – |
| 1 | `unknown_b0` → **threshold** | `Threshold` (0x104d2ead8) | float | **0** | −40 … 0 (stored in dB) | `"%.1f dB"` (raw value) | `Depth` | `processWheelDeltaGeneric(1.0, 0.1)` |
| 2 | `unknown_b8` → **release** | `Release` (0x104d2eb20) | float | **0** | 0 … 1 (normalized) | `FloatExponentialConverter(1, 500, e, 0, 6.2146…, "%.1f ms")`, so 1 ms … 500 ms, `ms = 500^v` [inferred formula] | (none set) | `processWheelOnFineResolutionOnly(0.1)` |
| 3 | `unknown_tail` → **ceiling** | `Ceiling` (0x104d2eb40) | float | **1** | 0 … 1 (normalized) | `FloatLinearConverter(−40, −0.3, "%.1f dB")`, so `dB = −40 + 39.7·v` | `Output` | `processWheelDeltaGeneric(0.5, 0.1)` |
| – | `side_chain_filter` | `SideChainFilter` object (0x40 bytes) | – | – | – | – | – | – |

All of this is **[exact]**, except the release display formula, which is **[inferred]** from
the converter's constructor arguments (the converter itself wasn't decompiled).

- Mode is **not automatable** (`setAutomatable(false)`) and forces a display update when
  it changes (`setParameterForcesDisplayUpdate`), because the visible knobs depend on it.
  **[exact]**
- Section names are wchar literals built inline: `"Mode"`, `"Depth"`, `"Output"`.
  **[exact]**

### 1.3 Knob layout (`getParameterByPosition`, 0x101a44ce4)

There are 8 positions per page: positions 0–7 are on page **Main** and 8–15 on page
**Side-Chain Input**. **[exact]**

| position | Legacy (mode 0) | Transparent (mode 1) |
|---|---|---|
| 0 | Mode | Mode |
| 1 | Threshold | Threshold |
| 2 | – | Release |
| 3 | – | Ceiling |
| 8 | SideChainFilter `[0]+0x1b8` | same |
| 9 | SideChainFilter `[1]+0x160` | same |
| 10 | SideChainFilter `[4]+0x158` | same |
| 11 | SideChainFilter `[2]+0x160` | same |
| 12 | SideChainFilter `[3]+0x160` | same |

The side-chain page parameters belong to `DATA::SideChainFilter`, which is shared with
Compressor and Gate. Their labels weren't decoded here **[unknown]**. The first one is
presumably the side-chain source selector, the others the filter on/off, centre
frequency, width and gain (see §5).

### 1.4 Serialization (`serialize<portable_binary_iarchive>`, 0x101a43f7c)

The class version is **1** (`iserializer::version` 0x101a4596c). The fields are stored in
this order: **[exact]**

1. `Module` base
2. `+0xb0` **Threshold**
3. `+0xc8` **SideChainFilter**
4. if version ≥ 1: `+0xa8` **Mode**, `+0xb8` **Release**, `+0xc0` **Ceiling**
5. if version == 0: Mode is set to **0 (Legacy)** through `ValueParameter<long long>::loadValue(0)`
   (0x101a44008). Release and Ceiling keep their defaults.

So in `crates/mxprj/src/types/module.rs` (~line 591): `unknown_b0` = threshold,
`unknown_a8` = mode, `unknown_b8` = release, `unknown_tail` = ceiling. Old (v0) projects
load as Legacy, and new projects default to Transparent.

### 1.5 Value → engine (`LimiterProcessor::processParameter`, 0x101a44ae0)

Each float parameter is bound with `setAndCallFuncAutomate` (so it's applied once at
construction too) to `processParameter(tParameters id, …, float v)`. Mode is bound to
`setMode(v)`, which only stores the int at `+0xa0`. **[exact]**

| id | param | Legacy engine (`LimiterCore`) | Transparent engine (`HilbertLimiter` param) |
|---|---|---|---|
| 0 | Threshold `t` (dB) | `setThreshold(t)`: `gainTarget = expf(t · −0.11512925)` (= 10^(−t/20)). If the core is not active (`activeCount ≤ 0`), the current gain jumps to the target too. | param 0 ← `(t + 40) / 40` |
| 1 | Release `v` | – (unused) | param 1 ← `expf(v · 6.214608) / 500` (= 500^v / 500, so 0.002 … 1.0) |
| 2 | Ceiling `v` | – (unused) | param 2 ← `v` (raw) |

All three rows are **[exact]**. On every `process()` call the processor also sets
HilbertLimiter param 3 (side-chain on) to 1.0 if pin 1 has ≥ 2 channels, otherwise 0.0, and
calls `LimiterCore::setSidechain(sameBool)`. **[exact]**

`HilbertLimiter::setParameter` (0x102604ba8): **[exact]**

```
p0 (threshold): thrDB  = (1 − v)·40;                gT = exp10f(thrDB / 20)      // input drive ≥ 1
p1 (release):   relT   = max(0.001, v);             cRel = 1 − 1/(relT · sr)     // at +0xb2c
p2 (ceiling):   ceilDB = (1 − v)·(−39.7) + (−0.35); gC = exp10f(ceilDB / 20)
p3 (sidechain): sidechain = (v > 0.5)
```

gT and gC are the two lanes of a `SmoothingToolbox<2>`: lane 0 is gT, lane 1 is gC (§4.6).
If the engine is suspended (flag `+0xc`, set by the constructor and `onSuspend`, cleared
by `onResume`), the new value is applied immediately. Otherwise it's a pending target
that is ramped in (§4.6).

> The ceiling the UI shows (−40 + 39.7·v) is 0.05 dB higher than what the DSP uses
> (−40.05 + 39.7·v). With v = 1 the UI says −0.3 dB but the DSP limits to −0.35 dB.
> **[exact]** (both formulas are in the code). The release that is displayed (500^v ms) is
> half the time constant the DSP uses (500^v / 500 s = 2·500^v ms). **[exact]** for the DSP
> side; the display side is **[inferred]** from the converter.

---

## 2. Signal routing (`LimiterProcessor::process`, 0x101a45250)

**[exact]**

- Main L/R are processed in place. Side-chain pin 1 counts as present when it has
  ≥ 2 channels. If it is present, its buffers first run through
  `SidechainFilterCore::processAudio` in place (§5). If it is not, the "side-chain" pointers
  point at the main buffers.
- `mode == 1` (Transparent): the processor feeds the HilbertLimiter in control blocks of
  **128** samples. A counter at `+0xc0` starts at 0. When it reaches 0 the processor calls
  `processControlRate()` and resets the counter to 128. It then calls `processAudio` on
  `min(remaining, counter)` frames with `in = {mainL, mainR, scL, scR}` and
  `out = {mainL, mainR}`.
- `mode == 0` (Legacy): `LimiterCore::processAudio` is inlined here (the same code as
  0x101a1d310).
- Only the selected engine runs. The other engine's state is frozen; there is no crossfade
  or reset on a mode change.
- `suspend()`: `LimiterCore::deactivate()` (`activeCount−−`, meter = 0) and
  `HilbertLimiter::onSuspend()`.
  `resume()`: `LimiterCore::activate()` (`activeCount++`), `SidechainFilterCore::resume()`
  and `HilbertLimiter::onResume()`.
- `onSamplerateChanged(sr)` (0x101a451e0) calls `HilbertLimiter::onSetSampleRate(sr, 128)`
  every time. If `sr` changed, it also calls `LimiterCore::setSampleRate(sr)` and the
  inlined `SidechainFilterCore::setSampleRate`. The constructor initializes the
  HilbertLimiter at 44100 Hz with block size 128.

---

## 3. Legacy mode: `FXCORE::LimiterCore`

Constructor 0x101a1cf68, `setSampleRate` 0x101a1d064, `processAudio` 0x101a1d310 (the
disassembly is in `limiter-asm/`). This is a stereo-linked, two-stage limiter: a fast
envelope stage, then a lookahead stage made of a sliding maximum followed by a sine-window
FIR. Its ceiling is fixed at 1.0 (0 dBFS) after the threshold gain. **[exact]** unless
noted.

### 3.1 Setup (`setSampleRate(sr)`)

```
N  = round½(sr · 88 / 44100)              // lookahead, 88 @ 44.1k, 96 @ 48k, 192 @ 96k
L  = smallest power of two ≥ N (min 2)    // loop: x=2; do{prev=x; x*=2}while(prev<N); L=prev
θ  = π / N                                 // float: 3.1415927f / (float)N
w  = (cos θ, −sin θ)                       // phasor step                 (+0xa8,+0xac)
z0 = (1, 0)                                // phasor reset value          (+0xa0, from 0x104c99718)
s½ = sin(θ/2), c½ = cos(θ/2)               // (+0x1c) = s½; (+0x78,+0x7c) = (c½, s½)
kThr = (sr·0.01 ≥ 1)    ? 0.693147/(sr·0.01)    : 0.693147   // threshold smoothing, 10 ms
kAtt = (sr·0.00066 ≥ 1) ? 0.693147/(sr·0.00066) : 0.693147   // +0x14
kRel = (sr·0.2 ≥ 1)     ? 0.693147/(sr·0.2)     : 0.693147
(+0x18) = kRel − kAtt
tree[2L]     ← 1.0 (memset_pattern16 with 1.0f, 0x104d16f50); treeW = 0; treeLen = N
D[L]         ← 0  (window delay, mask L−1, delay N)
A[2L]        ← 0  (interleaved stereo audio delay, mask 2L−1, delay 2N floats = N frames)
acc = (0,0,0,0); ctr = 1; flip = (0,0,0,0); env = 0
```

The constants 0.00066 (`0x3a2d03da`) and 0.2 (`0x3e4ccccd`) are set by the constructor at
`+0xc4` and `+0xc8`. The constructor also sets `gainTarget = gain = 1.0`. `ln 2` is
written as the literal `0x3f317215`.

### 3.2 Per sample

`(xL, xR)` = main input, `(sL, sR)` = side-chain input when the side-chain flag is set
**and** the side-chain block has ≥ 2 channels, otherwise the main input. The steps are in
this order:

```
inPeak    = max(|xL|, |xR|);  blockPeak = max(blockPeak, inPeak)              // meter only
g         = g + kThr·(gainTarget − g)                                           // threshold gain, smoothed
(aL, aR)  = (xL·g, xR·g)                     // audio is always driven by g, even with side-chain
(dL, dR)  = sidechainActive ? (sL·g, sR·g) : (aL, aR)
p         = max(|dL|, |dR|, 0)                                                  // stereo-linked peak
d         = p − env
env       = env + d · (d > 0 ? kAtt : kRel)                                     // 0.66 ms att / 200 ms rel, one-pole
g1        = env < 1 ? 1 : 1/env
A[ai], A[ai+1] = aL·g1, aR·g1                                                   // stage-1 gain applied before the delay

// sliding maximum of p over the last N samples (binary max-tree of float4 nodes)
tree[treeW] = p;  treeW = (treeW+1 ≥ N) ? 0 : treeW+1
M = max over all L leaf slots           // slots N..L−1 stay 1.0 forever ⇒ M ≥ 1 whenever N < L
y = M · s½
D[di] = y

// sine-window FIR of length N, done recursively with a rotating phasor
if (ctr−− < 2) {                         // every N samples, re-anchor (no drift)
    ctr  = N
    z    = z0 XOR flip;  flip ^= sign bits of lanes 0,1      // z alternates (1,0), (−1,−0), …
    acc  = (acc[2], acc[3], 0, 0)        // partial sum of the last N samples becomes the window sum
} else {
    z    = z · w                         // complex multiply: (zr·wr − zi·wi, zr·wi + zi·wr)
}
x    = D[(di − N) & (L−1)];  di = (di+1) & (L−1)
acc[0..1] += z · (y + x)                 // sliding window (z(n−N) = −z(n), so +x removes the old sample)
acc[2..3] += z · y                       // fresh partial sum since the last re-anchor
A_ = acc[0] + i·acc[1]
E  = c½ · Im(A_·conj z) + s½ · Re(A_·conj z)
   = Σ_{k=0}^{N−1} y(n−k) · sin(π(k+½)/N)        // window sum = 1/s½, so DC gain is 1
G  = 1 / E

(outL, outR) = A[(ai − 2N) & (2L−1)] · G         // audio delayed N frames
ai = (ai + 2) & (2L−1)
```

Assembly details that matter for bit-exactness:

- The rotation is computed as `v5=(zr,zr,zi,zi)*(wr,wi,wr,wi)`, then
  `re = zr·wr − zi·wi`, `im = zr·wi + zi·wr` (0x101a1d404…0x101a1d434).
- The FIR read-out is `trn2` + `fmul` by `(0,0,c½,s½)`, then a lane-2 + lane-3 add
  (0x101a1d488…0x101a1d4b8).
- At a re-anchor sample the phasor is **not** rotated. The jump goes straight to the
  accumulate step with `z = ±(1,0)`.
- The initial `ctr = 1`, so the first sample after `setSampleRate` re-anchors with
  `z = (1,0)`.

> **Two gain stages multiply.** The audio stored in the delay already carries
> `g1 = 1/max(1, env)`, and the output divides again by `E ≈ windowed max(p)`. Both are
> driven by the same un-reduced peak `p`. A steady signal 6 dB over the threshold therefore
> ends up about 12 dB lower, not 6. This is what the disassembly does
> (`fmul.2s v2,v2,v3[0]` at 0x101a1d5dc before the store, `fdiv` at 0x101a1d4d8 on the
> read). **[exact]** for the code; it would be worth confirming with a null test against
> Maschine.

### 3.3 Legacy meter

After each block: `meter = max(blockPeak, meter · (1 − 4/X)^frames + 1e-36)` at `+0xd0`,
where `X` is the first argument of `processAudio` (the inlined copy reads
`ProcessInfo + 8`). X is **[inferred]** to be the sample rate. In Legacy mode,
`getInputMeterValue` returns this value, and the GR and output meters return 0.
**[exact]** (0x101a44e88, 0x101a44ef4, 0x101a44ec4).

---

## 4. Transparent mode: `NI::DSP::HilbertLimiter`

Built as `HilbertLimiter(false)` (0x10260408c). Instance size 0xc00, 128-byte aligned. It
has 4 inputs and 2 outputs (`getNumInputChannels` = 4). It is a stereo-linked,
brick-wall **sample-peak** limiter with a Hilbert-envelope "program" detector, a
log-domain release, a W-sample peak hold and a Hann-window gain FIR. There is no
oversampling and no true-peak detection. **[exact]** unless noted.

### 4.1 Setup (`onInit` 0x1026042c8, `onSetSampleRate(sr, block=128)` 0x102604370)

```
onInit:  p0 = p2 = 1.0;  relT = 0.01;  warp = 0.89 (0x3f63d70a at +0x1a0)
         holdL = holdR = −100 dB (pattern at 0x10c350ff0); meters −40
m   = max(1, (int)(sr / 44100))            // truncation: 1 @ 44.1/48k, 2 @ 88.2/96k, 4 @ 176.4/192k
W   = 128 · m                              // 128 = ctor arg (false → 0x80; true would be 0x20)
K   = max(1, (int)(sr·0.02/128 + 0.5))     // parameter-ramp length in 128-sample control blocks (~20 ms)
inv = 1 / (K·128)
cAtkH = 1 − 1/(0.014·sr)   cRelH = 1 − 1/(0.003·sr)       // Hilbert-env smoother   (+0xb04/+0xb08, +0xb10/+0xb14)
c06   = 1 − 1/(0.0006·sr)                                 // dB smoother, both dirs (+0xb1c/+0xb20)
cRel  = 1 − 1/(relT·sr)                                   // user release           (+0xb2c)
cMet  = 1 − 1/(0.1·sr)                                    // meter release          (+0xb38/+0xb44)
(+0xb28 = 1 − 1/(0.0003·sr) is computed but never read)
HannFilter.setWindowSize(W); HannFilter.setSwitchRate((int)sr)
HilbertTransformer(−warp) then setSampleRateMultiplier(m)
audio delays aL[2W], aR[2W] (mask 2W−1);  dB rings dbL[W], dbR[W] (mask W−1);  all zero
```

### 4.2 Per sample (`processAudio` 0x102605050, read from disassembly)

```
smoother step (§4.6) → gT = lane 0, gC = lane 1
gA = sidechain ? 1.0 : gT
aL[ia] = xL·gA + 1e-12;  aR[ia] = xR·gA + 1e-12             // 1e-12 = 0x2b8cbccc
dL = (sidechain ? scL : xL)·gT + 1e-12
dR = (sidechain ? scR : xR)·gT + 1e-12
dbL[ib] = 20·log10f(|dL|);  dbR[ib] = 20·log10f(|dR|)

(hL0, hL1, hR0, hR1) = HilbertTransformer.process(dL, dR)       // §4.3
eL = hypotf(hL0, hL1);  envL = eL + (envL < eL ? cAtkH : cRelH)·(envL − eL)
eR = hypotf(hR0, hR1);  envR = eR + (envR < eR ? cAtkH : cRelH)·(envR − eR)
lvl = 20·log10f(max(envL, envR, |dL|, |dR|))                    // fcsel order: max(envL,envR), then |dL|, then |dR|

r   = (r < lvl) ? lvl : lvl + cRel·(r − lvl)                    // instant attack, user release, dB domain (+0xb24)
s   = r + c06·(s − r)                                           // 0.6 ms symmetric smoother (+0xb18), s = previous value
gdB = fminnm(0 − s, 0)                                          // → GR value (+0x194)

holdL = peakHold(dbL)  (see 4.4);  if (gdB + holdL > 0) gdB = 0 − holdL
holdR = peakHold(dbR);             if (gdB + holdR > 0) gdB = 0 − holdR
gLin = exp10f(gdB / 20)                     // also written to grBuf[n] (+0xba8)
h    = (float) HannFilter.process((double) gLin)                // §4.5
k    = gC · h
outL = k · aL[(ia − (W−1)) & (2W−1)];  outR = k · aR[(ia − (W−1)) & (2W−1)]   // audio delay W−1
(+0xb90)[n] = gT · k
meters: in  = max(|xL|,|xR|)   (raw), mIn  = (mIn < in)  ? in  : in  + cMet·(mIn − in)    → +0x198
        out = max(|outL|,|outR|),     mOut = (mOut < out) ? out : out + cMet·(mOut − out) → +0x19c
ia, ib advance by 1 (masked)
```

**Ceiling guarantee.** The peak hold covers the samples `[n−W+1, n]` and the Hann window
averages gains over the same W samples. Every gain tap that touches the delayed sample
`n−(W−1)` is therefore ≤ `10^(−dB(n−W+1)/20)`, so `|out| ≤ gC` up to float error and
the 1e-12 offsets. The threshold acts as input drive (`gT = 10^(−thr/20)`), and the
detector's 0 dB reference is the ceiling. **[exact]** for the code; the guarantee is
derived.

`getParameter` (0x102604d1c, jump table at 0x10c34dbc6): 0 → p0, 1 → relT param,
2 → p2, 3 → sidechain (1.0/0.0), 4 → last grBuf value,
5 → **GR meter** = `clamp(GR/−40, 0, 1)`, 6 → **input meter** `mIn` clamped to [0,1],
7 → **output meter** `mOut` clamped to [0,1]. `DATA::Limiter::get{GR,Input,Output}MeterValue`
read params 5, 6 and 7 in Transparent mode. **[exact]**

### 4.3 HilbertTransformer (ctor 0x10261f70c, `process` 0x102620040)

There are 7 cascaded first-order all-pass stages on a float4 vector
`(dL, dL, dR, dR)`. Each stage has an 8-slot ring of float4 values, a per-lane coefficient
`a` and a delay `m` (≤ 8, from `setSampleRateMultiplier`). The transfer function is
`(a + z^−m)/(1 + a·z^−m)`:

```
s       = ring[(idx − m + 1) & 7]
t       = a·(u − s)
ring[(idx+1) & 7] = u + t;  idx = (idx+1) & 7
y       = s + t                 // next stage's u
```

**The return values are not all from the last stage.** Returned:
`(s0, s1, s2, s3) = (stage6.y[0], stage7.y[1], stage6.y[2], stage7.y[3])`. So the "real"
lanes (0 and 2) go through 6 stages and the quadrature lanes (1 and 3) go through 7. The
asm tail is `mov s2,v0[2]; mov s1,v4[1]; mov s3,v4[3]` with `s0 = v0[0]`.

Coefficients use a frequency warp `p = −0.89` on prototype values `c` (table 0x10c351070):
`a = (p + c) · R(c·p + 1)`, where R is the NEON reciprocal `e0 = frecpe(x)`,
`e1 = e0·frecps(x,e0)`, `R = frecps(x,e1)·e1`. The stage → lane pairs are
`(c[0], c[1], c[0], c[1])`. Bit-exact values from the frecpe emulation
(`limiter-asm/hilbert_coefs.py`):

| stage | c (lanes 0/2) | a (lanes 0/2) | c (lanes 1/3) | a (lanes 1/3) |
|---|---|---|---|---|
| 1 | 0.32915163 | −0.793217301 `0xbf4b104a` | 0.5943129 | −0.627703667 `0xbf20b130` |
| 2 | 0.7735346 | −0.373820633 `0xbebf656b` | 0.8816616 | −0.038725365 `0xbd1e9e7d` |
| 3 | 0.9443752 | +0.340897560 `0x3eae8a20` | 0.9837171 | +0.752797246 `0x3f40b752` |
| 4 | −0.32915163 | −0.942926228 `0xbf71639d` | −0.5943129 | −0.970812619 `0xbf78872d` |
| 5 | −0.7735346 | −0.985246122 `0xbf7c3917` | −0.8816616 | −0.992706060 `0xbf7e21fc` |
| 6 | −0.9443752 | −0.996675432 `0xbf7f261f` | −0.9837171 | −0.999045014 `0xbf7fc16a` |
| 7 | 0 | −0.89 `0xbf63d70a` (unused) | 0 | −0.89 `0xbf63d70a` |

The formula and table are **[exact]**. The final bits come from the emulation and are
**[inferred]**: they're within 1 ulp of `(p+c)/(1+cp)`, and they assume `FPCR.AH = 0`
(standard frecpe, no RPRES). For hardware-independent bit-exactness, hard-code the hex
values. Rings are zeroed when `m` changes.

### 4.4 Peak hold (per channel, on the dB ring)

```
v = db[ib]                               // the value just written
if (v ≥ hold)          { hold = v; cnt = W − 1 }
else if (cnt != 0)     { cnt−− }
else {                                   // expired: rescan the last W entries, newest first
    hold = −100; pos = 0
    for j in 0..W−1: if (db[(ib − j) & (W−1)] > hold) { hold = that; pos = j }   // strict >, earliest-newest wins
    cnt = W − 1 − pos
}
```

### 4.5 HannFilter (ctor 0x10261fd74, `setWindowSize` 0x10261fe70, `process` 0x10261ff84), all double

This is a recursive periodic-Hann FIR of length W with unity DC gain:
`out = (1/W)·Σ_{k=0}^{W−1} x(n−k)·(1 − cos(2πk/W))`.

```
setWindowSize(N): invN = 1.0/N;  c = cos(invN·2π);  c2 = c + c
                  buf = new double[1 << bitlen(max(N,1))] (zeroed; strictly > N, e.g. 256 for 128)
                  P = max(N, P_prev); cnt = 0; warm = P − N; all states 0
setSwitchRate(R): P = max(N, R); cnt = 0; warm = P − N; all states 0
process(x):
  buf[i] = x;  d = x − buf[(i − N) & mask];  i = (i+1) & mask
  S  += d                                  // running sum
  r   = d + y1·c2 − y2;  y1o = y1;  y2 = y1;  y1 = r    // resonator at 2π/N
  if (cnt++ ≥ warm) {                      // shadow filter, built from scratch with no subtraction
      r' = x + y1'·c2 − y2';  y2' = y1';  y1' = r';  S' += x
      if (cnt ≥ P) { (y1,y2,S) ← (y1',y2',S'); shadow ← 0; cnt = 0 }   // drift-free swap every P = sr samples
  }
  return invN · (S − (r − y1o·c))          // computed from the main state before any swap
```

### 4.6 Parameter smoothing (`SmoothingToolbox<2>`, in `processControlRate` 0x102604de8)

Four float lanes, of which lanes 0 (gT) and 1 (gC) are used. Per sample:
`ctr += 1; cur = start + delta·(ctr·inv)` (asm order: `(ctr·inv)` first, then `·delta`).
At each 128-sample control block:

- **New value pending:** `target ← pending; start ← cur; ctr ← 0; delta = target − cur`.
  Any lane with `|delta| < ulp(cur)` snaps (`delta = 0; cur = start = target`). If any
  delta is left, a ramp of `K` blocks starts (≈ 20 ms, linear).
- **Otherwise:** count the ramp down. When it reaches 0, `cur = start = target`, `delta = 0`,
  and then any newer pending value is latched the same way.

`onResume` and `onSuspend` jump straight to the targets. **[exact]** for the flow; the
ulp snap test is an **[inferred]** reading of the bit trick.

---

## 5. Side-chain filter (`FXCORE::SidechainFilterCore`, shared with Compressor and Gate)

This is applied in place to the side-chain buffers before either engine sees them, and only
when side-chain pin 1 is connected with ≥ 2 channels. It is not part of the Limiter
proper and should get its own spec. What is known: **[inferred]**

- **Filter off:** each channel is multiplied by a smoothed gain (127-sample ramp
  scheduling).
- **Filter on** (`processAudioWithFilter`): cascaded RBJ-style LP/HP biquads, Q = 1/√2.
  - Centre frequency = `ExpTable((v·119.58942 + 15.48682)·5 + 1381.8816)`
    (`convertFilterCutoffNorm2Hz` 0x101a2069c).
  - Gain = `(ExpTable(gain·−20·0.7071·86.5617 + 1200) + 1)·b0LP·b0HP·g`
    (`calcFilterCoeffs` 0x101a207cc).

---

## 6. Things that look related but aren't used

- `NI::DSP::Limiter` (0x102603918…): a different 2-channel limiter core. The
  LimiterProcessor never constructs it; it's only referenced by its own vtable. **[exact]**
  (the processor constructor builds only `LimiterCore` and `HilbertLimiter(false)`).
- `HilbertLimiter(true)` would use a 32-sample lookahead base. Maschine's Limiter uses
  `false` (128). **[exact]**

---

## 7. Implementation notes and open questions

1. **Latency is 0.** Both modes delay the signal: Legacy by N = round½(sr·88/44100)
   samples, Transparent by W−1 = 128·m − 1. Maschine doesn't compensate this. A faithful
   clone should also report 0; optionally offer real PDC as a non-default "fix".
2. **Legacy double reduction** (§3.2 note): verify with a null test before "fixing" it.
3. **Release and ceiling display vs DSP** (§1.5): the DSP time constant is twice the
   displayed ms, and the DSP ceiling is 0.05 dB below the displayed value. Reproduce the
   DSP values; use the UI formulas only for labels.
4. **Float paths for bit-exactness:**
   - Transparent threshold: `v = (t+40)/40` (f32), then `(1−v)·40`, then `exp10f(x/20)`.
   - Legacy threshold: `expf(t · −0.11512925)`.
   - Release: `expf(v·6.214608)/500`.
   - `log10f`, `exp10f`, `hypotf` and `expf` come from the Apple libm. Exact bits depend on
     libm and are **[unknown]** for other platforms.
5. **Hilbert coefficients:** use the hex values in §4.3 rather than recomputing with
   `1/x`.
6. **The Hann filter runs in double.** Its input is `(double)gLin` and its output is
   rounded to f32 before the multiply by gC.
7. **Sample rates with an exact power-of-two N in Legacy** (N == L, e.g. sr ≈ 64145 Hz):
   no tree padding stays at 1.0, so `M` can fall below 1 once the initial 1.0 slots are
   overwritten. At 44.1, 48, 88.2, 96 and 192 kHz there is padding (N = 88/96/176/192/384,
   L = 128/128/256/256/512). **[exact]** from the code.
8. **Transparent with side-chain:** the main audio is *not* driven by the threshold
   (`gA = 1`), but Legacy always drives the audio by `g`. **[exact]**
9. **[unknown]:**
   - the labels and ranges of the 5 side-chain-page parameters;
   - the `FloatExponentialConverter` implementation (display mapping);
   - the meaning of `ProcessInfo + 8` used by the Legacy meter decay;
   - GUI ballistics in `LimiterEditor.qml`.
