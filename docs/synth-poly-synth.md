# Maschine's Poly Synth: parameters and DSP

A clean-room description of Maschine 3's built-in **Poly Synth**
(`NI::MASCHINE::DATA::PolySynth` → `NI::MASCHINE::RT::PolySynthProcessor` →
`ni::p5::P5`, a Prophet-5-style engine; a `ni::p5::pcb::kProphet5Mantra` symbol sits
next to it). All of it is code: the oscillators are analytic with polynomial
band-limiting, and the only table (pitch/rate exponentials) is computed at start-up
with `exp2f`. No NI data files are read.

Written from Ghidra output in `out/decomp/poly/` (`INDEX.tsv` maps files to
functions; the `connectParameters` lambdas are `0050.c`…`0110.c`) and from the arm64
code, saved as `out/decomp/poly-p5.s` (all of `P5`), `out/decomp/poly-data.s`
(`DATA::PolySynth`) and `out/decomp/poly-paramdefs.s` (the parameter-definition map).
Where Ghidra and the assembly disagree (it drops `fmadd`s, reassociates sums, prints
ints as floats and shows the table one entry off), this spec follows the assembly.
The DECKARD port is `crates/engine/src/synth/poly_synth.rs` with
`poly_synth/{engine,params,voices,render}.rs`. It nulls **bit-exactly** against
Maschine (§5).

**Confidence.** **[exact]**: read from code. **[inferred]**: reconstructed or
measured. **[unknown]**: not recovered.

Notation: `v` = a parameter's normalized engine value; `sr` = sample rate; all math
`f32`; `fma(a,b,c)` = `a·b + c` with one rounding (an arm64 `fmadd`).

---

## 1. Parameters

### 1.1 Module

Module name "Poly Synth" (`s_moduleInfo`, 0x2e). Nine pages,
`s_pageNames = {Filter, Envelopes, Mix / Voices, Oscillator A, Oscillator B, LFO,
Modulation, Poly Mod, Global}` (`__GLOBAL__sub_I_Modules_PolySynth_ALL.cpp`) **[exact]**.
QML editor: `Modules/PolySynth/PolySynthEditor.qml` (tabs Osc A / Osc B / LFO / Mod:
`PolySynthOscAPage.qml`, `PolySynthOscBPage.qml`, `PolySynthLFOPage.qml`,
`PolySynthModPage.qml`).

### 1.2 Parameters (constructor order, `PolySynth::PolySynth` 0x101aad09c)

Stored order (= `serialize`, 0x101aaca14) is the table order except `filter_mode`
(0x1d0) comes after `hold`, and `voices`, `voice_mode` come last. `editor_tab` (0xf0),
`filter_version` (0x2b8) and `osc_version` (0x2c0) are **not serialized**; the two
version flags reach the engine (`ParameterId` 0x3c/0x3d) but have no effect.

| off | mxprj field | label | default | range | P5 id | engine value |
|---|---|---|---|---|---|---|
| 0xf8 | pm_filter_env | Filter Env | 0 | 0..1 | 0x00 | `x=fma(v,4.7,0.4)`, depth `fma(x,x,−0.16)` |
| 0x100 | pm_osc_b | Oscillator B | 0 | 0..1 | 0x01 | `v·3.09` |
| 0x108 | pm_freq_a | Freq A | off | bool | 0x02 | 0.4 / 0 |
| 0x110 | pm_pw_a | PW A | off | bool | 0x03 | −1 / 0 |
| 0x118 | pm_filter | Filter | off | bool | 0x04 | 3.8 / 0 |
| 0x120 | osc_a_tune | Tune | 0 | −24..24 int | 0x05 | `⌊fma(v,47.625,0.4)⌋ − 12` semitones (in `f64`) |
| 0x128 | osc_a_saw | Saw | on | bool | 0x06 | saw level = mix A × on |
| 0x130 | osc_a_pulse | Pulse | off | bool | 0x07 | |
| 0x138 | osc_a_pulse_width | Pulse Width | 0.5 | 0..1 | 0x08 | `fma(v,2,−1)`, smoothed |
| 0x140 | osc_a_sync | Sync | off | bool | 0x09 | hard sync A to B |
| 0x148 | osc_b_tune | Tune | 0 | −24..24 int | 0x1b | `fma(kbd,−35,23) + ⌊(v·47.625+0.4)(2−kbd)⌋` |
| 0x150 | osc_b_fine | Fine | 0 | 0..1 | 0x1c | `fma(v,0.935,0.015)` semitones |
| 0x158 | osc_b_saw | Saw | on | bool | 0x1d | 0.5 / 0 |
| 0x160 | osc_b_triangle | Triangle | off | bool | 0x1e | |
| 0x168 | osc_b_pulse | Pulse | off | bool | 0x1f | |
| 0x170 | osc_b_pulse_width | Pulse Width | 0.5 | 0..1 | 0x20 | `fma(v,2,−1)`, smoothed |
| 0x178 | osc_b_low | Low | off | bool | 0x21 | −91.3 semitones (LFO range) |
| 0x180 | osc_b_kbd | KBD | on | bool | 0x22 | B follows the keyboard |
| 0x188 | mixer_osc_a | Oscillator A | 1 | 0..1 | 0x0a | smoothed |
| 0x190 | mixer_osc_b | Oscillator B | 0 | 0..1 | 0x0b | `2v`, smoothed |
| 0x198 | mixer_noise | Noise | 0 | 0..1 | 0x0c | `1.75v` |
| 0x1a0 | mixer_external | External | 0 | 0..1 | 0x0d | `25v` (no input reaches it) |
| 0x1a8 | filter_cutoff | Cutoff | 1 | 0..1 | 0x0e | `⌊120v⌋ − 39` semitones, smoothed |
| 0x1b0 | filter_resonance | Resonance | 0 | 0..1 | 0x0f | §3.5 |
| 0x1b8 | filter_env_amount | Env Amount | 0 | 0..1 | 0x10 | `v·(±192)`, smoothed |
| 0x1c0 | filter_kbd | KBD | 0 | 0..1 | 0x11 | smoothed |
| 0x1c8 | filter_invert | Invert | off | bool | 0x35 | env amount sign |
| 0x1d0 | filter_mode | Mode | LP | enum LP/BP/HP | 0x34 | `idx/2` |
| 0x1d8 | filter_attack | Attack | 0 | 0..1 | 0x12 | §3.3 |
| 0x1e0 | filter_decay | Decay | 0 | 0..1 | 0x13 | |
| 0x1e8 | filter_sustain | Sustain | 1 | 0..1 | 0x14 | `0.98v` |
| 0x1f0 | filter_release | Release | 0 | 0..1 | 0x15 | |
| 0x1f8 | lfo_mode | Mode | Free | enum Free/Synced (short Free/Sync) | 0x16 | |
| 0x200 | lfo_rate | Rate | 0 | 0..1 | 0x17 | §3.2 |
| 0x208 | lfo_saw | Saw | off | bool | 0x18 | |
| 0x210 | lfo_triangle | Triangle | on | bool | 0x19 | |
| 0x218 | lfo_pulse | Pulse | off | bool | 0x1a | all three = sample & hold |
| 0x220 | mod_lfo_noise | LFO<>Noise | 0 | 0..1 | 0x24 | crossfade LFO → lowpassed noise |
| 0x228 | mod_amount | Amount | 0 | 0..1 | 0x23 | smoothed |
| 0x230 | mod_freq_a | Freq A | off | bool | 0x25 | 0.4 |
| 0x238 | mod_freq_b | Freq B | off | bool | 0x26 | 0.4 |
| 0x240 | mod_pw_a | PW A | off | bool | 0x27 | 1 |
| 0x248 | mod_pw_b | PW B | off | bool | 0x28 | 1 |
| 0x250 | mod_filter | Filter | off | bool | 0x29 | 60 semitones |
| 0x258 | glide | Glide | 0 | 0..1 | 0x2a | rate `T[fma(fma(v,−72,80),9.965784,1200)]` st/s |
| 0x260 | amp_attack | Attack | 0 | 0..1 | 0x2c | §3.3 |
| 0x268 | amp_decay | Decay | 0 | 0..1 | 0x2d | |
| 0x270 | amp_sustain | Sustain | 1 | 0..1 | 0x2e | `0.95v` |
| 0x278 | amp_release | Release | 0 | 0..1 | 0x2f | |
| 0x280 | release | Release | on | bool | 0x30 | off: release time 0 |
| 0x288 | velocity | Velocity | off | bool | 0x31 | sens. 0.8 amp / 0.55 filter |
| 0x290 | lfo_voice_trig | Voice Trig | off | bool | 0x32 | §3.2 |
| 0x298 | hold | Hold | off | bool | 0x33 | latch |
| 0x2a0 | detune | Detune | 0 | 0..1 | 0x36 | §3.6 |
| 0x2a8 | tune | Tune | 0 | −1..1 | 0x37 | `fma(v,1.4,−0.7)` semitones |
| 0x2b0 | volume | Volume | 1 | 0..1 | 0x38 | gain `0.224v`, ramped |
| 0x2c8 | voice_mode | Mode | Poly | enum Poly/Unison/Unison with Legato (short Poly/Unison/Uni + Leg, extra-short Poly/Uni/U+L) | 0x2b | `idx·0.5` |
| 0x2d0 | voices | Voices | 5 ("6") | enum "1".."16" | — | `idx+1` voices; a change resets all voices |
| 0x2d8 | pitchbend_range | Pitchbend | 2 | 0..24 int | 0x3b | semitones |

Normalization (`connectParameters` lambdas): `v = (f32)((1/(max−min))_f64 · (x − min))`;
bools 0/1; Filter Mode `idx·0.5`; LFO Mode 0/1. `P5::setParameterValue` then quantizes by
the definition's type (switch: `< 0.5`→0 else 1; three-way, in `f64`: `< 0.33333`→0,
`< 0.66666`→0.5 else 1) and stores the clamped value **[exact]**.

### 1.3 Control-area pages (`getParameter(page, slot)`, 0x101aae7e4)

| page | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
|---|---|---|---|---|---|---|---|---|
| Filter | Cutoff | Resonance | Env Amount | Invert | KBD | Mode | – | – |
| Envelopes | F Attack | F Decay | F Sustain | F Release | A Attack | A Decay | A Sustain | A Release |
| Mix / Voices | Osc A | Osc B | Noise | – | Glide | Mode (voice) | Voices | Detune |
| Oscillator A | Tune | Sync | – | – | Saw | Pulse | Pulse Width | – |
| Oscillator B | Tune | Fine | Low | KBD | Saw | Triangle | Pulse | Pulse Width |
| LFO | Rate | Mode | Voice Trig | – | Saw | Triangle | Pulse | – |
| Modulation | Amount | LFO<>Noise | Freq A | Freq B | PW A | PW B | Filter | – |
| Poly Mod | Filter Env | Oscillator B | Freq A | PW A | Filter | – | – | – |
| Global | Tune | Pitchbend | Velocity | Release | Hold | – | – | – |

External is on no page. Section headings (`setSectionName` + span): Filter (slot 0,
span 6); Filter Envellope [sic] (0, 4), Amp Envelope (4, 4); Mix (0, 3), Voices (4, 4);
Oscillator A (0, 2), Shapes (4, 3); Oscillator B (0, 4), Shapes (4, 4); LFO (0, 3),
Shapes (4, 3); Modulation (0, 2), Destinations (2, 5); Polymod Source (0, 2),
Destinations (2, 3); Global (0, 5). Value text: `ScaleNormalizedToPercentOneDecimal`
(`v·100` "%.1f %") on Env Amount, KBD, mixer, Glide, Detune, Pulse Widths, Fine,
Amount, Poly Mod sources; Osc tunes are bipolar ints (48 steps, semitones); master
Tune bipolar (fine steps 1200/100); LFO Rate uses `getLFORateValueString`: free →
`P5::getParameterDisplayValue(0x17)` (Hz), synced → a `n/d` division from
`SyncableLFOHelper`; switches show on/off; enums their option strings. Remaining
time knobs use the default converter **[inferred]**.

---

## 2. Signal flow

LFO + noise → wheel/amount → modulation `m`. Per voice: amp ADSR, filter ADSR; Osc B
(saw/tri/pulse, two rings: audio and poly-mod) → poly mod `pm = 3.09·B_mod + depth·fenv`
→ Osc A (saw/pulse, sync from B's wrap, FM by `pm`), mixer (A + 2·mixB·B + noise),
4-pole filter (two 2-pole state-variable stages, LP/BP/HP taps) with soft-clipped
feedback, DC removal, × amp envelope (4-sample delay), summed; × volume gain; the same
value to both channels.

## 3. DSP (`P5::process`, 0x101aa33ec; per sample) — all **[exact]**

### 3.1 Table and construction
`T[0] = 2⁻²⁰`, `T[j] = exp2f((j−1200)/60)` for `j = 1..2400` (entries written from
`P5+0x2794`, so Ghidra's reading is one off); lookup at `y`: `i = (int)y`;
`i ≥ 2400` → `T[2400] = 2²⁰`; `i < 0` → 0; else `fma(y−i, T[i+1]−T[i], T[i])`.
Pitch → Hz: `T[fma(semis,5,1381.8816)]` (MIDI note 0 = 8.18 Hz).
Construction: `P5(44100)` (defaults, `setProgram`, `allSoundOff`), processor sets 32
slots and clears; Maschine then connects the stored parameters **while the engine
still runs at 44.1 kHz** and only then calls `onSamplerateChanged(sr)`, which updates
1/sr, π/sr, 6.283/sr, √(sr/44100), the smoothing length `max(⌊fma(sr,0.02,0.5)⌋,1)`,
the noise and DC coefficients — but not the envelope/glide/LFO rates or the volume
ramp already computed (verified: doing it the other way is off by 48000/44100 at
48 kHz). The first 100 samples skip the control block (counter starts at 100).

### 3.2 LFO, noise, modulation
Phase += rate/sr; saw `ph·0.4`, triangle and pulse with linear step corrections
(see `render.rs`); none on → constant 0.4; all three → sample & hold of the
lowpassed noise at each cycle start. Rate free: `T[fma(fma(v,116.7,−92.7),5,1381.8816)]`
Hz; synced: `tempo/60 · mult`, `mult = 2^(⌊fma(v,9.725,−5.5)⌋+6)/64` (×1.5 if the
fraction > 0.58) cycles per beat; the phase follows the song position only while the
transport runs. Voice Trig: a key (re)starts the LFO; envelopes restart at each LFO
cycle start and release at mid-cycle. Noise: 1-bit shift register
`x' = (x>>1)|(((x>>13)^(x>>5)^x)<<31)`, seed 1, ±0.5·√(sr/44100), one-pole lowpass.
`m = lerp(LFO, noiseLP, LFO<>Noise) · fma(5−w, amount, w)` with `w` the mod wheel ×5.

### 3.3 Envelopes
Attack: `+inc` per sample to the velocity level `fma(sens, vel/128−0.5, 0.5)`;
`inc = level·T[fma(fma(v,90.7,−5),−9.965784,1797.947)]/sr`. Decay/release:
`rem *= coef`, `value = rem + floor`, `coef = 1 − T[fma(fma(v,85,−3),…)]/sr` (decay),
`T[fma(fma(v,92,−10)·rel,…)]` (amp release; filter 92.7). Amp sustain floor
`fma(peak+0.02, 0.95v, −0.02)`, release floor −0.02: the voice stops when the value
goes negative. Filter floor `peak·0.98v`, release floor 0.

### 3.4 Oscillators
Phase −1..1 (wrap subtracts 2). Corrections go into 4-sample rings with the 8th-order
step residual `P` and 9th-order corner residual `Q` (coefficients in `render.rs`),
output 2 samples late. B: saw `0.5·ph`, triangle `|ph|−0.5`, pulse `±0.5`, frequency
`T[fma(kbd,note,offB)]·(1+0.4·m·FreqB)`. A: saw `mixA·ph`, pulse `2·mixA·(±0.5)`, freq
`f·(1 + 0.4·pm·PolyFreqA + 0.4·m·FreqA)` (both delayed), increment smoothed
`(inc+new)/2`, hard sync at B's wrap fraction (2 samples late).

### 3.5 Filter
Cutoff (Hz) `T[5·(kbd·note + envAmt·fenv + 60·m·Filter + cutoff + det) + 1381.8816]`,
× `(1 + 3.8·pm)` if Poly Mod Filter, two samples late; `w = min(max(c,1)·π/sr, 1.5)`,
`g = w(1.00005+0.0388452w−0.0896638w²)/(1+0.0404318w−0.430871w²)`. Two TPT 2-pole
stages, damping `2−2r`, `2−1.6r` with `r` = resonance curve (`2.32v`; `0.52v+0.45`;
`0.92v+0.25`; `0.036v+0.913` at 0.25/0.5/0.75); stage 2 input `damp2·stage1[mode]`;
states +1e-7 each sample. Feedback `fb = sign·y(0.13335 ∓ 0.01666875y)` (|y|<4) else
`±0.2667·sign` (sign −1 for BP). Cutoff target smoothed: `fma(res,4.5,fma(1−kbd,35,cut))`.

### 3.6 Voices
32 slots, Voices plays `idx+1`. Allocation list (doubly linked): a note reuses the
voice that played it; else the longest-released voice; when all hold keys the oldest
is stolen (no fade: its envelopes restart from their current value). Unison: all voices
on one note, newest key priority, on release falls back to the nearest held note;
Unison retriggers envelopes, Unison with Legato only glides. Detune: fixed-seed
shift register (1234567890) per voice: A/B ±0.01·d, cutoff ±0.5·d, drift `1+0.1·d·r`,
unison spread ±0.035·d (d = `0.19205·T[fma(5v,86.5617,1200)]`); at 0 unison voices
spread evenly over ±0.035 semitones. Glide only in unison. Hold latches like a sustain
pedal (with no key down it plays note 60).

### 3.7 MIDI
`processEventBuffers` forwards notes, pitch bend, CC 1 (wheel → id 0x39), CC 64
(sustain) and CC 120 (all sound off). Events take effect at the start of the block.
Bend glides over 0.02·sr samples to `2·range·(v−0.5)`.

## 4. What a new Poly Synth holds

The defaults column of §1.2 (also `PARAMS` in `poly_synth.rs`). The engine's own
`P5` program (only used before Maschine connects) differs and is irrelevant.

## 5. Null tests (maschine-ref vs synth-null, 48 kHz unless noted)

Base patch: Mashup This Out 3/5 with plain values (saw A, filter open, LFO free,
mod 0, poly 8 voices). All rows: residual **−240 dB, peak 0 (bit-exact)**:
defaults-like basic; velocity sens. at vel 1 and 127; notes 36 and 96; 44.1 and
96 kHz; Osc A pulse PW 0.2; Osc B saw+tri+pulse, tune +7, fine; Osc B Low, KBD off;
sync; BP and HP with resonance 0.6/1.0; filter ADSR + env amount + KBD; Invert; amp
ADSR; Release switch off; noise; LFO triangle → Freq A; saw → filter; pulse → PW A/B,
Freq B; sample & hold; no shape; LFO<>Noise; full poly mod; Voice Trig; 7 overlapping
notes on 4 voices (stealing); 16 voices + detune; same note retriggered ×3; Unison
(+detune, glide); Unison with Legato; Unison detune 0; Hold; glide in poly; master
tune / Osc A tune / bend range. Library patches: My Only Thing 4/2 (exact); Mashup
This Out 3/5 with its synced LFO (exact with `--tempo 120`, the host tempo
maschine-ref gives the plug-in).

Not exact: **Steady 1/2** — residual −38.5 dB, diverging from sample 103 (first
control block). The same 61 field values put into another project null exactly, and
Maschine's own render of Steady differs from that one, so the difference is outside
the module's stored fields **[unknown]**. CC 1, CC 64 and pitch bend sent by
maschine-ref do not change Maschine's output at all (renders identical with and
without them), so the wheel, sustain and bend paths are ported from the code but not
null-tested.
