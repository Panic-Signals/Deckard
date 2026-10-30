# Phase 3 — Variation (design, findings and plan)

> Roadmap: `docs/superpowers/plans/2026-10-22-parity-roadmap.md`. Inline, test-first.
> The user: "Euclidean in a variation actually accessed under groove. we need to reverse
> engineer all the variation parameters." Then: Random "completely replaces all the
> notes", and "If you have a scale selected and you do randomize that all the notes
> exist in that scale" (a change Maschine doesn't make).

## Findings
- **Maschine 3's Variation is the Groove's** (sound ▸ Groove: Swing, Variation, Generate,
  with Generate's Random and Euclidean engines). All its parameters and algorithms were
  reverse-engineered earlier (`docs/channel-software.md` §5) and work in DECKARD's
  software. Generate replaces the sound's notes in the pattern, which is the "Random"
  that replaces every note.
- **The MK3's VARIATION button** no longer opens the old Humanize/Random page in
  Maschine 3. `MaschineControllerMK3:onVariationButton` calls
  `MaschineHelper.showGrooveVariationSection()`: back to Control mode, the sound focused,
  and `ParameterPageAccess.setFocusPageOnGroovePageGroup(App, 1)` (Groove ▸ Variation).
  From there the channel pages reach Generate.
- **The old page** (`VariationPageBase.lua`: HUMANIZE / RANDOM / APPLY on the Workspace's
  `Randomize…` parameters) is registered only by the base `MaschineController`, and no
  supported controller's button opens it in Maschine 3. It isn't built. For the record:
  - **Parameters** (`Workspace::Workspace`, getters checked by disassembly):
    | parameter | default | range or choices |
    |---|---|---|
    | Mode | Random | Humanize, Random |
    | Note Range | 60–72 | 0–127 |
    | Velocity Range | 100–127 | 1–127 |
    | Note Count | 1 | 1–6 |
    | Probability | 50 | 10–100 % |
    | Time Shift (Step) | 0 | 0–0.5 |
    | Note Length (Steps) | 1 | 1–6 |
    | Notes spread | Gauss | Equal, Gauss, 1/2 Gauss |
    | Note Length spread | 1/2 Gauss | Fixed, Equal, Gauss, 1/2 Gauss |
    | Note Count spread | Fixed | Fixed, Equal, Gauss, 1/2 Gauss |
  - **Humanize:** each note's velocity becomes an even draw in the velocity range; with
    Step > 0, its time moves by up to ±Step of a step.
  - **Random:**
    - each step gets notes with chance Probability, with the time shift;
    - lengths come from a spread over [1, Steps], in half steps;
    - the chord size and pitches come from their spreads, snapped to the scale;
    - a pitch that is taken moves up to the next free one.
  - Decompiles: `out/decomp/randomizer`, `out/decomp/chan2/0025–0027.c`.

## What this phase does
1. **MK3 VARIATION** → `setFocusPageOnGroovePageGroup(n)` → `ControlRequest::GroovePage`
   → the app shows the focused sound's Groove page `n` (channel properties, set 2).
2. **Scale rule (DECKARD's own):**
   - With a scale selected (the pattern editor's SCALE, which is also the controller's
     Keyboard scale; Chromatic means none), Generate's Random
     draws chord pitches only from the scale's notes in the note range.
   - The root note (pads, and Euclidean) is moved up into the scale.
   - A range with none of the scale's notes gets no notes.
   - Maschine snaps pitches up and can leave them outside.
   - Changing the root (in the software or on the controller, while the same group
     stays focused) moves the focused sound's Note Lo and Note Hi by the same number of
     semitones, within 0–127 (DECKARD's own too).
3. Docs: PARITY's Euclidean row (it is Maschine's), the Variation line.
