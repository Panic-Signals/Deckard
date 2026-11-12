# Sample analysis (sampler roadmap 1a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** a new crate, `deckard-analysis`, that tells a sample's tempo (with ×2 / ½
candidates and its length in beats) and its root note (with cents and a confidence), plus
note names read from file names.

**Architecture:** pure functions over `&[[f32; 2]]` frames and a sample rate (the engine's
`Sample::frames` shape), no I/O, no threads: callers run them off the audio thread (1b
caches the results). Tempo: a log-energy onset envelope in two bands, autocorrelated over
60–200 BPM with a prior around 120, folded into 85–170, then snapped to a whole number of
bars when the length allows (the loop prior), unless a tempo hint (from the file name) is
given. Root: Zhora's pitch detector over hopping windows, a loudness- and clarity-weighted
vote by MIDI note, cents from the winners.

**Tech Stack:** Rust 2024; `deckard-plugins` (Zhora's `pitch::detect`), `deckard-library`
(`names`).

**Spec:** `docs/superpowers/plans/2026-11-11-sampler-roadmap.md` (Phase 1a).

## Global Constraints

- Clean room: no vendor code or tables.
- Notes in DECKARD's naming: MIDI note 60 is C3.
- Folded tempo range 85–170 BPM; search 60–200 BPM (roadmap 1a).
- Low confidence leaves the root unset (`None`) rather than guessing.
- Cargo through mise: `mise exec -- cargo …`; `mise run lint` and the workspace tests at the end.

## Review Focus

1. Silence, a few samples, or a zero rate: `None`, no panic, no divide by zero (Task 2, Task 3 `silence_and_tiny_inputs`).
2. A one-shot (a single kick): no tempo rather than a nonsense one (Task 2 `a_single_hit_has_no_tempo`).
3. A loop at 174 BPM: folds to 87 but keeps 174 as a candidate, so ×2 finds it (Task 2).
4. A hint far from the audio's tempo (a mislabelled file): the hint wins; confidence says so (Task 2 `the_hint_wins`).
5. 96 kHz and 22.05 kHz samples give the same answers as 44.1 kHz (Task 2, Task 3 rate tests).

---

### Task 1: Note names in file names

**Files:** Modify `crates/library/src/names.rs`.

**Interfaces — Produces:** `pub fn note_in_name(name: &str) -> Option<(u8, Option<i8>)>`:
the pitch class (0 = C … 11 = B) and the octave if written (DECKARD's: C3 = 60). A token is
a note when it stands alone between separators (start, end, space, `_`, `-`, `.`, `[`,
`]`, `(`, `)`): a letter A–G (either case), optional `#`, `b` or `♯`/`♭`, optional `-`?digit
octave (−2..8). Single-letter tokens without an octave count only in upper case ("Bass
C.wav" yes, "a.wav" no). The last match in the name wins (names end in the note: "Piano
F#2.wav"). The extension is ignored.

- [ ] Tests: `notes_from_names`: "Piano F#2.wav" → (6, Some(2)); "Bass_Db1_soft.wav" → (1, Some(1)); "Lead C.wav" → (0, None); "Kick 01.wav" → None; "Bass Amber.wav" → None; "Pad E-1.aif" → (4, Some(-1)); "Cello a3.wav" → (9, Some(3)); "drums.wav" → None; "Synth [120] G3.wav" → (7, Some(3)).
- [ ] Run `mise exec -- cargo test -p deckard-library names` — PASS. Commit.

### Task 2: The crate and tempo

**Files:** Create `crates/analysis/{Cargo.toml, src/lib.rs, src/onset.rs, src/tempo.rs, src/test_signals.rs}`; modify root `Cargo.toml` (member).

**Interfaces — Produces:**
```rust
pub struct Tempo {
    pub bpm: f64,          // folded into 85–170, or the hint
    pub beats: f64,        // the range's length in beats at `bpm`
    pub offset: usize,     // frame of the first beat (the grid's phase), within one beat
    pub confidence: f32,   // 0..1; 1 for a hint
    pub source: TempoSource, // Hint | Loop (snapped to whole bars) | Audio
    pub candidates: [f64; 2], // bpm × 2 and bpm / 2
}
pub fn tempo(frames: &[[f32; 2]], rate: u32, hint: Option<f64>) -> Option<Tempo>
pub(crate) fn onset::envelope(frames: &[[f32; 2]], rate: u32) -> (Vec<f32>, f64 /* frames per hop */)
```
- Onset envelope: hop ≈ 5.8 ms (256 at 44.1 kHz, scaled by rate), two bands per hop: the
  full signal's energy and its first difference's (a crude high band, for hats), each
  log-compressed (`ln(1 + 100 e)`), half-wave rectified difference, summed; mean removed.
- Tempo: autocorrelation of the envelope at the lag for each BPM from 60 to 200 in 0.1
  steps (fractional lag, linear interpolation), weighted by a log-normal prior centred on
  120 BPM (σ = 1 octave); add the ×2 lag's value at half weight (harmonics). Best BPM;
  fold by ×2 / ½ into 85–170.
- Confidence: the winning score over the mean score across the search range, mapped
  `((ratio − 1) / 3).clamp(0, 1)`. `None` when the envelope has fewer than 4 onsets above
  its mean + 1 σ, or the range is under 1 s (one-shots), or confidence < 0.1.
- Loop prior: with length L seconds and the folded bpm, `n = L·bpm/60`; if some whole
  count of bars (4 beats) or 2^k beats lies within 4 % of `n`, use `bpm = 60·n'/L` exactly
  and `source = Loop`.
- Hint: `bpm = hint`, `confidence = 1`, `source = Hint`, envelope still used for `offset`.
- Offset: the phase within one beat (hop resolution) maximising the envelope summed at
  beat positions.
- Test signals (`test_signals.rs`, `#[cfg(test)]`): `drums(bpm, bars, rate, swing) ->
  Vec<[f32; 2]>`: kick (decaying 55 Hz sine, 120 ms) on beats 1 and 3, snare (decaying
  noise, 150 ms, deterministic LCG) on 2 and 4, closed hats (8 ms noise bursts) on 8ths;
  `tone(hz, seconds, rate, harmonics)`.

- [ ] Tests (`tempo.rs`): `finds_the_tempo_of_a_drum_loop` (4 bars at 93, 120, 140 BPM, 44.1 kHz: |bpm − true| < 0.05, source Loop); `the_tempo_of_an_unlooped_range` (4 bars + 300 ms tail at 120: within 1 BPM, source Audio); `folds_fast_tempos_and_keeps_the_other` (174 BPM: bpm 87 ± 0.1, candidates[0] ≈ 174); `swing_doesnt_throw_it` (120 BPM, 8ths swung 60 %: within 0.5); `the_hint_wins` (120 BPM audio, hint 100: bpm 100, Hint, confidence 1); `a_single_hit_has_no_tempo` (one kick: None); `silence_and_tiny_inputs` (empty, 10 frames, 2 s of zeros, rate 0: None); `other_rates_agree` (120 BPM at 22 050 and 96 000: within 0.05); `the_offset_finds_the_first_beat` (the loop rotated by 1/4 beat: offset within one hop of 1/4 beat's frames).
- [ ] Run `mise exec -- cargo test -p deckard-analysis tempo` — PASS. Commit.

### Task 3: Root note

**Files:** Create `crates/analysis/src/root.rs`.

**Interfaces — Consumes:** `deckard_plugins::zhora::pitch::{detect, WINDOW, Pitch}`.
**Produces:**
```rust
pub struct Root { pub note: u8, pub cents: f32, pub confidence: f32 }
pub fn root(frames: &[[f32; 2]], rate: u32) -> Option<Root>
pub fn root_with_name(frames: &[[f32; 2]], rate: u32, name: &str) -> Option<Root>
```
- Mono-sum; above 60 kHz average pairs down (as Zhora's `detector_rate`, re-implemented: it's `pub(crate)`); windows of `WINDOW` hopping by `WINDOW / 2`, skipping windows quieter than −50 dBFS RMS.
- Each detection votes for `round(69 + 12·log2(hz/440))` with weight `rms · clarity`.
  Winner: most weight. Cents: the weighted median of `1200·log2(hz / note_hz)` among the
  winner's votes. Confidence: winner's weight over the weight of all loud windows (periodic
  or not).
- `None` under 3 votes for the winner or confidence < 0.4.
- `root_with_name`: a note in the name (Task 1) sets the pitch class; the octave is the
  name's, else the detected note's nearest octave of that class, else 3 (C3 = 60). Cents
  from detection when its class matches, else 0. Confidence 1.

- [ ] Tests: `finds_a_sine`s root (110 Hz → 45, |cents| < 2); `finds_the_cents` (110·2^(−12/1200) → 45, cents −12 ± 2); `a_rich_tone_keeps_its_fundamental` (sawtooth-like, 10 harmonics, 130.81 Hz → 48); `the_longest_note_wins` (3 s E2 + 1 s G2 → 40); `drums_have_no_root` (Task 2's drum loop → None); `silence_and_tiny_inputs` (None); `other_rates_agree` (220 Hz at 96 000 → 57); `the_name_sets_the_class` (220 Hz sine named "Pad A2.wav" → 57 conf 1; named "Pad C.wav" → 60 (C nearest 57's octave: 60), cents 0).
- [ ] Run `mise exec -- cargo test -p deckard-analysis root` — PASS. Commit.

### Task 4: Finish

- [ ] Roadmap: mark 1a built (tempo, root, names); note the corpus is synthetic for now.
- [ ] `mise run lint` PASS; `mise exec -- cargo test -p deckard-analysis -p deckard-library` all ok; `mise exec -- cargo clippy -p deckard-analysis` clean.
