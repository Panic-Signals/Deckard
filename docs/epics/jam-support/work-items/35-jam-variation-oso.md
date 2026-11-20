# 35 — RANDOMIZER publishes its Variation page

Status: `done` (2026-11-21) · HIL: **yes** (four rows in `hil/oso-test-sheet.md`, plus one probe) · Owner: machine with the Jam.

## What was missing

`OSO_RANDOMIZER` is on the static list (`oso-contract.md` §4) and shows **nothing**: `maschine_jam`
never composes a page for it, and DECKARD's model could not have fed one. The rig found it
(`hil/oso-test-sheet.md`, "Known-absent"), and the captures of the reference (2026-11-21) pin its
shape: **thirteen settings over two pages, five in Humanize, static** (no timer — the encoder's press
closes it, as with PERFORM FX but without the timer).

## The reference's shape

`Variation` (Humanize, one page, no counter):

| # | Value | Sub-label | Section |
|---|---|---|---|
| 1 | `Humanize` | Mode | |
| 2 | `Apply` (a button) | — | |
| 3 | `100` | Velocity Lo | VELOCITY RANGE |
| 4 | `127` | Velocity Hi | VELOCITY RANGE |
| 5 | `± 0 %` | Step | TIME SHIFT |

`Variation 1/2` (Random, keyboard mode):

| # | Value | Sub-label | Section |
|---|---|---|---|
| 1 | `Random` | Mode | |
| 2 | `Apply` | — | |
| 3 | `50 %` | Probability | PROBABILITY |
| 4 | `C3` | Note Lo | NOTE RANGE |
| 5 | `C4` | Note Hi | NOTE RANGE |
| 6 | `100` | Velocity Lo | VELOCITY RANGE |
| 7 | `127` | Velocity Hi | VELOCITY RANGE |
| 8 | `1` | Note Count | CHORDS |

`Variation 2/2`:

| # | Value | Sub-label | Section |
|---|---|---|---|
| 1 | `1.0` | Steps | NOTE LENGTH |
| 2 | `± 0 %` | Step | TIME SHIFT |
| 3 | `Fixed` | Note Count | DISTRIBUTIONS |
| 4 | `Gauss` | Notes | DISTRIBUTIONS |
| 5 | `1/2 Gauss` | Note Length | DISTRIBUTIONS |

Maschine 3 moved the split against the MK3 manual's 2.x layout (which had Note Length and Time Shift
on page 1): the capture wins, 8 + 5 = 13.

**Random in pad mode** drops Note Range and Chords (the manual's pad layout: `Probability | Velocity
Range | Time Shift | Note Length | Note Length dist`), so the page there is 6 slots. The composer
follows the pads' mode rather than always showing the keyboard page.

## The parameter contract

The MK3 manual's Variation chapter (`docs/channel-manual.md` §5.2) gives labels, ranges and steps;
NI documents no algorithm. Settings live under `sequence_variation/` in the sound, which
`Sound::groove` already keeps when the project's format has no place for them
(`SequenceVariationModule` is 4 floats + 1 enum + 1 bool and cannot hold the rest).

| Field | Values | Default | Source |
|---|---|---|---|
| `mode` | 0 Humanize, 1 Random | 0 (Humanize: the reference titles that page plain `Variation`, Random `1/2`) | capture (2 modes) |
| `velocity_range/low` | 0–127 | 100 | capture |
| `velocity_range/high` | 0–127 | 127 | capture |
| `time_shift` | 0–50 %, 5 % steps (SHIFT 1 %) | 0 | manual §5.2 |
| `probability` | 10–100 %, 10 % steps (SHIFT 1 %) | 50 | manual §5.2 |
| `note_range/low` | 0–127 | 60 (C3) | manual §5.2, capture |
| `note_range/high` | 0–127 | 72 (C4) | manual §5.2, capture |
| `note_count` | 1–6 | 1 | manual §5.2 |
| `steps` | 1–6 (one decimal) | 1.0 | manual §5.2, capture |
| `dist/note_count` | 0 Equal, 1 Gauss, 2 ½ Gauss, 3 Fixed | 3 | manual §5.2 |
| `dist/notes` | 0 Equal, 1 Gauss, 2 ½ Gauss | 0 | manual §5.2 |
| `dist/note_length` | 0 Equal, 1 Gauss, 2 ½ Gauss, 3 Fixed | 3 | manual §5.2 |

Distribution numbering is **DECKARD's**: `Rng::spread` already reads `0` Equal, `1` Gauss, `2` ½ Gauss
(`crates/doc/src/groove.rs`), which is the manual's vocabulary in the manual's order; `Fixed` is
new and means "do not vary this target". NI's enum indices are not recoverable from the manual and
are not invented here.

## What Apply does

One pass, two modes, both against the focused Sound's notes in the focused pattern, one undo step,
and **re-applied from the notes as they were** when Live is on (the existing
`apply_variation_from`), so a drag re-rolls instead of compounding.

**Humanize** (mode 0) — perturb only, nothing created or removed:
- velocity drawn in `[velocity_range/low, velocity_range/high]`
- position ± `time_shift` % of a grid step, clamped inside the pattern
- length untouched

**Random** (mode 1) — vary the beats, and make notes:
- each note survives with `probability` % (else it goes)
- pitch drawn in `[note_range/low, note_range/high]`, kept in the selected scale when one is on
  (manual: "Random mode conforms to the selected scale"), shaped by `dist/notes`
- velocity drawn in the velocity band
- length `steps` grid steps, shaped by `dist/note_length` (`Fixed`: exactly `steps`)
- position ± `time_shift` %
- `note_count` notes stack the chord per step, shaped by `dist/note_count` (`Fixed`: exactly the
  CHORDS value)

Two readings are DECKARD's, not NI's, and are marked for the rig:

1. **Humanize's velocity rule** — the manual does not say whether the band replaces the velocity or
   bounds a deviation ("users complain it is a fixed range, not a deviation around the original",
   §5.2). This build draws in the band. Probe: Velocity 40/60, Apply, then look at whether the notes
   sit inside 40–60.
2. **Random varies rather than regenerates** — the manual offers "randomly generated patterns *or*
   variations of your beats". This build varies what is there (probability keeps or drops, ranges
   re-place) so that it stays distinct from the Generate page, which replaces wholesale.

## Landed

- **The model** (`crates/doc/src/groove.rs`): `Variation` — the thirteen settings, each
  clamped to the manual's range, read over DECKARD's defaults — and `Document::variation`.
  `apply_variation` / `apply_variation_from` take the selected scale and dispatch on the
  mode: Humanize clamps the velocity into the band (the in-app knob's `velocity_amount` is
  still the deviation) and moves only the position; Random keeps or drops by the
  probability and re-makes what stays, in the note range and the scale, as chords of the
  Note Count, at the page's length and velocity. Both re-apply from the notes as they were,
  so a live drag re-rolls instead of compounding.
- **The view** (`crates/surface/src/lib.rs`): `VariationView` on `View.variation`, and
  `Action::Variation { field, value }` / `Action::ApplyVariation`.
- **The script API** (`crates/surface/src/api.rs`): `state().variation`,
  `deckard.variation(field, value)` (the twelve names validated, so a typo is an error and
  not a silent nothing), `deckard.apply_variation()`.
- **The app** (`apps/viewer/src/main.rs`): the view is the focused sound's; a setting
  change goes through `set_groove_setting` and the live re-apply; Apply is the in-app
  Groove page's button, so the two can't drift.
- **The page** (`maschine_jam/main.lua`): a static menu of the thirteen, five shown in
  Humanize, the note range and the chords hidden in pad mode; the composer slices it into
  the reference's pages (8 + 5, or 5) with the section headers. SHIFT + E opens it and
  still shows the strips their Groove Variation page.

## Tests

- `crates/doc/src/groove.rs`: `humanize_bounds_velocity_and_leaves_the_rest` (nothing made
  or dropped, the band bounds, positions and lengths untouched) and
  `random_keeps_the_ranges_the_chords_and_the_length` (pitches in range, velocities in the
  band, a step long, the chord is the Note Count).
- `crates/surface/src/jam_tests.rs`: `the_variation_page_publishes_its_settings` — SHIFT + E
  publishes the Humize page of five (Mode, Apply without a sub-label, the velocity range,
  the time shift), the MODE turn asks the app to switch, Random then shows two pages of
  eight with Probability, the note range as C3/C4 and CHORDS, and the encoder's press on
  Apply asks the app to vary.
