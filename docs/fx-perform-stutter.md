# Maschine's Perform FX Stutter: parameters and DSP

A clean-room description of the Stutter mode (mode 5) of Maschine 3's Perform FX
(`NI::MASCHINE::DATA::PerformanceFX` → `NI::MASCHINE::RT::PerformanceFXProcessor` →
`NI::DSP::JamStutter`). DECKARD's Stutter (`crates/engine/src/fx/perform/stutter.rs`) is
written from this description and nulls bit-exactly against Maschine (§9). It replaced
DECKARD's own earlier design; §11 records how that one differed.

Written from Ghidra output in `out/decomp/perform-jam/` (`INDEX.tsv`: files 0022–0040 and
0232 are `JamStutter`; 0259 is the processor's constructor, where `JamStutter` is built
inline) and `out/decomp/perform-data/` (0003 `connectParameters`, 0005/0007 the Assign and
Quantize hooks, 0023 the Length display). Checked against renders through Maschine
(`maschine-ref`, §8). Offsets are `JamStutter` object offsets.

**Confidence.** **[exact]**: read from the code. **[inferred]**: reconstructed from partial
evidence, or confirmed only by renders. **[unknown]**: not recovered.

Notation: `x` is the strip position or the Length knob (0–1), `bps` = tempo / 60,
`sr` = sample rate.

---

## 1. Parameters

`connectParameters` hands the stored fields to `JamStutter::setParameter(index, value)`
**[exact]**:

| index | field | value the core gets | used when |
|---|---|---|---|
| 0 | `touchstrip_pos` | 0–1 | always (Length or Pitch, by TS Assign) |
| 1 | `touchstrip_on_off` | 0 / 1 | always |
| 2 | `stutter_ts_assign` | 0 Length, 1 Pitch | |
| 3 | `stutter_length` | 0–1 | TS Assign = Pitch only (the knob then sets Length) |
| 4 | `stutter_pitch` | 0–1 | TS Assign = Length only |
| 5 | `stutter_direction` | index × 1/3 (0, 0.333, 0.667) → Forward / Reverse / Both by thresholds 0.333 / 0.666 | TS Assign = Length only; Pitch forces Forward |
| 6 | `stutter_quantize` | 0 / 1 → quantize mode 0 or 3 (thresholds 0.25/0.5/0.75; modes 1 and 2 exist in the core but the bool never reaches them) | always |
| 7 | `stutter_retrigger` | 0 / 1 | TS Assign = Pitch only; Length forces it on |
| 8 | `stutter_gate` | 0–1 | always |

Constructor constants **[exact]**: maximum loop 4.0 s (`+0x178`), Length base 0.5 per beat
(`+0x184`), declick 0.0005 s (`+0x180`), Both hysteresis 0.1 (`+0x188`), pitch reference
floor 0.15 and ceiling ratio 2.0 (`+0xf8`, `+0x100`), DC filter 35 Hz with damping 2
(`+0x298`, `+0x2a8`; both channels).

## 2. Length

The loop rate is `r = f · 0.5` loops per beat, so the loop lasts `1 / (bps · r)` seconds
**[exact]** (`setLength`, `calculateLoopLength`):

- **Quantize on:** `f = 1 << floor((1.3 − 0.3|x|) · 5x + 0.5)`, giving 1/2, 1/4, 1/8,
  1/16, 1/32 and 1/64 notes. 1/2 is at the start of the strip and 1/64 at the end. The
  curve is not linear. The steps change at x ≈ 0.078, 0.245, 0.427, 0.630 and 0.865, so
  1/2 has the smallest share of the strip **[exact]**. The display
  (`convertNormalized2StutterLengthString`) prints `1/(2 << k)` from the same formula.
- **Quantize off:** `f = 2^((0.7 + 0.3|x|) · 5x + 0.5)`, continuous and **relative to the
  tempo**: about 1/2.8 of a whole note at x = 0 down to about 1/90 at x = 1. At 120 BPM that
  is 707 ms down to 22 ms. The display shows milliseconds from the same formula
  **[exact]**.
- **Slow tempi:** if a half note would be longer than 4 s − 10/sr, every length is halved
  until it fits **[exact]**.

## 3. The buffer and the touch

- **Buffer:** two ring buffers (L, R) of `sr · 8 + 3` samples. While disengaged, the input
  is written to them continuously. Once engaged, writing continues until the count since
  the loop's origin reaches half the buffer, so **up to 4 s after the loop's origin is
  captured, whatever Length plays** **[exact]**.
- **Touch, Quantize off:** the loop's origin is the touch. The first pass plays the input
  as it is being captured, then repeats **[exact]**.
- **Touch, Quantize on:** the grid is the loop length, but **no finer than a 16th**: grid
  = `1 / min(4, r)` beats, so 1/32 and 1/64 loops sit on the 16th grid **[exact]**. The
  position is the host's song position plus the samples counted since it last arrived.
  - **In the first quarter of a grid cell, and Direction not Reverse:** the loop's origin
    is that cell's grid line, which is already in the buffer. Playback continues from the
    touch, so the rest of the step plays on, then the step repeats from its grid line
    **[exact]**, render-confirmed (§8 "early").
  - **Later in the cell, or Direction Reverse:** nothing happens until the **next** grid
    line. The input plays on unchanged, and the stutter starts there with the loop's origin
    on that line **[exact]**, render-confirmed (§8 "late", "rev").
- **Engage fade:** engaging and releasing glide the wet mix linearly over 10 ms
  **[exact]**.

## 4. Playing the loop

- **Read:** 4-point cubic (Catmull-Rom) interpolation, then the 35 Hz DC filter: a 2-pole
  TPT state-variable high-pass, `x − 2·bp − lp`. It applies to the wet signal only
  **[exact]**.
- **Declick:** each pass fades in and out over 0.5 ms at the loop's boundaries, except at
  the very start **[exact]**.
- **Output:** `dry + (env · wet − dry) · fade`, where `dry` is the input delayed **2**
  samples by a 3-slot ring inside the core **[exact, by test]**. The core runs while
  disengaged too, so a released Stutter's output is the input 2 samples late (renders: best
  lag 2).
- **Can't run ahead:** the read position is clamped to what has been captured since the
  touch **[exact]**.
- **Tempo changes:** the read rate is scaled by `bps_now / bps_at_touch` **[exact]**.

## 5. Length changes while engaged

- **The origin stays where it was.** The new length is measured from that same origin
  **[exact]**.
- **Longer:** the new length takes over at the current loop's next wrap. Since up to 4 s
  is captured, it can grow to anything up to what has been recorded so far **[exact]**.
- **Re-aligned to the grid:** with Retrigger (always on in Length mode), the pass restarts
  at each wrap. The longer loop therefore plays the new material once and then lands on its
  own grid line from the origin: §8 "grow", where a 1/2 loop restarts at 2.0 s, the
  half-note line **[inferred from the renders]**.
- **Shorter:** the new length takes over at the boundary of the new length counted from
  the origin, if one comes within the current pass **[inferred]** (the `+0x146` flag path).

## 6. Direction

- **Forward / Reverse:** fixed **[exact]**.
- **Reverse reads backwards from the origin.** Until a whole loop has been captured, it
  reads into the audio before the origin, so the step before the grid line plays
  backwards first. Then it plays the captured step backwards **[exact]**, render-confirmed
  (§8 "rev").
- **Both:** the strip's movement sets the direction, with **0.1 of hysteresis**. Moving
  up more than 0.1 past the lowest point since the last change gives Forward; moving down
  more than 0.1 below the highest gives Reverse **[exact]**. The new direction applies at
  the next loop wrap (`+0x144 ← +0x145` there) **[exact]**.
- **On touch:** Forward and Both start forward, Reverse starts in reverse.

## 7. Pitch and Gate

- **Pitch is relative to its value at the touch** **[exact]**, render-confirmed (§8 "p75").
  The pitch control is the Pitch knob in Length mode, or the strip in Pitch mode, glided
  over 100 ms. With `p0` its value at the touch and `p` its current value:
  - if `p0 ≥ 0.15`: the rate is `p / p0` when `p0 ≥ 0.5` or `p < p0`. Otherwise it is
    `1 + clamp01((p − p0) / (1 − p0))`, rising to 2 (an octave up) at `p = 1`. The level
    fades out below `p = 0.1` (`clamp01(p / 0.1)`): moving down slows to a stop, like a
    tape stop.
  - if `p0 < 0.15`: the rate is `2 − 1/(1 − p0)`, rising linearly to 2 at `p = 1`.
  - So **a Pitch setting made before touching has no effect.** Only movement after the
    touch changes the pitch. With Retrigger on, the loop's timing stays on the grid and
    only its content is pitched; with it off, the loop period follows the pitch.
- **Gate, Forward:** the loop is open up to `0.95 · g · L`, then fades linearly to silence
  by `1.05 · g · L`, where `L` is the loop length. Here `g = b + (1.06 − b) · gate` and
  `b = 0.5 − 0.4 · clamp01((L_seconds − 0.01) / 0.09)` **[exact]**:
  - **Gate 1:** `g = 1.06`, fully open.
  - **Gate 0:** the first 10 % of a loop of 0.1 s or longer still plays, up to 50 % for
    very short loops, so **Gate 0 is a short blip, not silence**.
  - **Smoothing:** both values glide over 20 ms.
- **Gate, Reverse:** the gate is a raised-cosine **swell** across each pass instead. Its
  start is `gate·(1.5 − 0.5|gate|)·3.95 − 3`, ramping to 1 by the loop's end **[exact]**.

## 8. Renders

Input: a stepped tone, one sine per 16th at 200 + 40k Hz (k = the step), 120 BPM, 48 kHz,
`--play`. Engaged and released with host automation at block starts. DECKARD was run with
a scratch harness (`out/perform-stutter/harness`, the same preroll, blocks and changes).
`steps read` names the step each window plays.

| case | settings | Maschine | DECKARD |
|---|---|---|---|
| early | 1/16, touch at 1.024 s (19 % into step 8) | repeats step 8 | repeats step 8 |
| late | 1/16, touch at 1.088 s (70 % into step 8) | step 8 plays to its end, then repeats **step 9** | repeats **step 8** |
| grow, Forward | touch 1.024 s at 1/16, strip to 0 (1/2) at 1.5 s | 8 9 10 11, then 8–15 from 2.0 s, on the half-note grid | 8–15 from 1.5 s, repeating every 1 s from there (half a bar off the grid) |
| grow, Both | as above, Direction Both (strip moved down → Reverse) | 7 6 5 4 (audio before the origin, reversed), then 15…8 from 2.0 s | 12 11 10 9 8, then 15…8, about 1/32 later than Maschine |
| g0 | Gate 0 | a 12 ms blip each 1/16 | silence |
| g5 | Gate 0.5 | open about 69 ms of 125 | open about 62 ms of 125 |
| p75 | Pitch 0.75 set before the touch, to 1.0 at 2.0 s | original pitch, then ×1.33 | ×1.41 at once, then ×2 |
| rev | Reverse, touch at 1.024 s | waits to 1.125 s; step 8 backwards, then step 9 backwards | step 8 to 1.125 s, then step 8 backwards |

Residual of the whole render, which includes the fades, the declick, the DC filter and the
3-sample dry delay: early −23.8 dB, late −15.2 dB L/R (`maschine-ref compare`). Not a
null: DECKARD's Stutter is a different design.

---

## 9. DECKARD's Stutter, and the null tests

`stutter.rs` follows §1–7 and the details below, taken from the code (`processAudio`
0x102624b10, `setTouchstripOnOff` 0x102623fc0, `setLength` 0x1026248c8,
`calculateLoopLength` 0x1026241d4, `onSetSampleRate` 0x102623b18) **[exact]**:

- **Order of a sample:** a counter since the last song position; the four glides (fade
  10 ms, Gate shape and raw Gate 20 ms, Pitch 100 ms; linear, `sr·0.001·ms` samples, rounded
  to the nearest for the count); the high-pass cleared when the fade rises from 0; the dry
  ring; the take (written while unheard, or while less than half the buffer is taken); a
  late touch's countdown (only while the fade is below 0.999); then, if heard: the pitch's
  rate and level, the wrap test (`clock mod (len·sr) < last step`), the shorter-loop test,
  the pass's fades (in from its start once the loop has played 0.5 ms; out to its end and,
  if a shorter loop cuts it, to that one's), at a wrap the next length and direction take
  over, the read (`back = taken + 3 − min(played, offset)`, offset negated and wrapped in
  Reverse), the 4-point cubic in `f32` coefficients summed in `f64`, the level, the 35 Hz
  high-pass (`tan` by its series to x⁵, `k` = 2), the gate, then the step: `clock += r`,
  `offset += r·pitch rate` with `r` = tempo ratio × (Retrigger ? 1 : pitch rate).
- **The loop clock** keeps growing with Quantize and wraps at the loop's length without.
  The read offset is not cleared at a touch, only at a wrap.
- **The direction** a wrap takes is the one set at the start of the core's current stretch
  (the processor runs it in stretches of up to 128 samples, split at events): a late touch's
  start sets it mid-stretch, and the wrap there still takes the stretch's.
- **Changes land at the next sample**, after the tempo (`onBPMChanged`, the host's as
  `f32`, ÷ 60) and the song position (`onPositionChanged`: the block's start, and whether it
  plays; the sample counter restarts when it plays or did): Maschine applies its block's
  events after those. A change of TS Assign, Direction, Quantize or the strip goes through
  the same setters as Maschine's (`setLength` also turns Both and resets the Gate's shape).
- **The song position** is the host's unrounded (not the Filter's 240 000-tick value),
  given through `Mode::set_position`; stopped, DECKARD keeps the last one.
- **The strip's position is stored as touched**: Maschine passes it to the core unchanged
  (`setTouchstripPos`), and the Length's step is only in what plays and what shows. DECKARD
  no longer snaps strip touches to a step (`perform_strip_snap` is gone), so the MK3's strip
  LEDs show where the strip was touched.
- **The display** (`convertNormalized2StutterLengthString`, data 0023): with Quantize
  `1/%i` of `2 << k`; without, `|240 / (2f · bpm) · 1000|` ms with the tempo as `f32` (120
  without a processor): below 0.005 "0.00 ms", below 1 `%.2f ms`, below 1000 `%.1f ms`,
  else `%.1f s` **[the value printed in seconds is inferred: ms/1000]**. DECKARD:
  `deckard_model::stutter_length_text`, with the song's tempo.

Null tests (`stutter.rs` `nulls_against_maschine`; renders and their script in
`out/perform-ref/stutter/`, `render.sh`): noise in, `--play` at 120 BPM, engage, release
and moves by host automation at 1024-frame block starts. Residual L/R:

| case | settings | changes | residual |
|---|---|---|---|
| early | 1/16, Forward | touch 1.024 s (19 % into a 16th), release 2.5 s | −240 dB |
| late | 1/16, Forward | touch 1.088 s (70 %), release | −240 dB |
| rev | 1/16, Reverse | touch 1.024 s, release | −240 dB |
| both | 1/16, Both | touch; strip to 0 (1/2) at 1.5 s, to 1 (1/64) at 2.7 s; release | −240 dB |
| grow | 1/16, Forward | touch; strip to 0 (1/2) at 1.5 s, back to 0.5 (1/16) at 3 s | −240 dB |
| g0 | Gate 0 | touch, release | −240 dB |
| g5 | Gate 0.5, strip 0.3, Reverse | touch, release | −240 dB |
| pitch | Pitch 0.75 before the touch | to 1 at 1.6 s, to 0.05 at 2.2 s | −240 dB |
| pitchlo | Pitch 0.1 (below the floor) | to 0.6 at 1.6 s | −240 dB |
| assign | TS Assign Pitch, Length 0.3, Retrigger off | strip 0.4 → 0.9 → 0.2 | −240 dB |
| free | Quantize off, strip 0.4 | touch; strip 0.8 at 1.8 s; release | −240 dB |
| early96 | as early, 96 kHz | | −240 dB |
| early44 | as early, 44.1 kHz | | −97 dB |

**Maschine's song position [unknown]:** a touch's grid position, and from it the loop
clock's fraction, depends on the last bits of the position Maschine's Stutter is given,
which Maschine keeps itself (it is not maschine-ref's `at/sr·tempo/60`, nor any per-block
sum or tick rounding of it tried). The test harness uses what a search against the renders
found: one unit in the last place below the host's at 48 kHz, the host's at 96 kHz, and the
host's rounded to 240 000 ticks at 44.1 kHz, which is only near Maschine's (about 0.03
samples off, so the half-millisecond fades at each pass's end differ: −97 dB). With the
host's own position at 48 kHz, a touch can take its step one sample earlier than Maschine
and wrap one sample late after 2^16 samples. In DECKARD the song position is its own
transport's, so this concerns only the null tests.

## 10. Open points

- **Stopped transport with Quantize:** the core uses the last song position plus a
  sample counter that keeps running while stopped, so it depends on what the host reports
  then; not rendered **[unknown]**. DECKARD keeps the last position it was given.
- **Shrinking while engaged** is now confirmed by the renders ("grow" back to 1/16, "both"
  to 1/64: null).
- **A change of mode to or from Stutter** while engaged follows the code (reset, resume:
  a held strip is touched afresh); not rendered.
- **The two unused quantize modes:** grids of 1 and 2 per beat exist in the core but
  aren't reachable from the stored bool **[exact]**.

## 11. How DECKARD's earlier Stutter differed (replaced)

DECKARD's own first design, before it followed this one: the touched step always repeated
(no waiting for the next grid line after a late touch); Pitch was absolute (±1 octave
around 0.5) and applied at once; Gate 0 was silence; Reverse played the rest of the touched
step forward first; a grown Length started where the strip was slid (off the grid); the
unquantized Length was a fixed 1000 ms to 10 ms; the Length's steps changed at 0.1, 0.3,
0.5, 0.7 and 0.9; 1/32 and 1/64 started on their own grid; Both turned on any movement;
one half note was taken; there were no declick fades, no 35 Hz high-pass, and a 2 ms engage
fade; strip touches were snapped to the Length's steps.
