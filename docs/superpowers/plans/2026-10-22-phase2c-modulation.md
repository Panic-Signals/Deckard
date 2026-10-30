# Phase 2c — Modulation: playback and Auto-write (design and plan)

> Roadmap: `docs/superpowers/plans/2026-10-22-parity-roadmap.md`. Inline, test-first.
> Sources: the software manual's "Recording and editing modulation" and the MK3
> manual's "Recording modulation on the controller" and "… in Step mode"; NI's scripts
> (`HardwareControllerBase:onAutoWriteButton`, `MaschineHelper.isAutoWriting`); the
> modulation in Maschine's projects (Maschine 2 libraries, the user's Maschine 3
> projects, Maschine 3's template projects).

## What Maschine stores
- **Lanes:** each channel (sound, group, song) has `modulators`
  (`ObjectVector<Modulator>`), each `target` a pointer to the modulated parameter (a
  module's `MaschineNumericParameter`, or a channel property).
- **Points:** `ModulationEvent<T>` in the pattern's sequences, `parameter` a pointer to
  the same parameter. Version 0 (Maschine 2, and Maschine 3 before its templates) stores
  `constant`; version 1 stores a `Modulation { a: [f32; 4], b: [f32; 4] }`. Every v1
  point seen (157 in the templates) is `a = [0, 1, 0, 0]`, `b = [value, 0, 0, 0]`: a
  plain point; Maschine 3 reads a v0 constant as that point (`Modulation::constant`).
  Curves (⌥-drag) presumably fill the rest; DECKARD reads `b[0]` and writes plain points.
  An archive holds one version per class: DECKARD writes the version the project already
  has, else v1 (Maschine 3's).
- **Values:** knobs (continuous): a deviation from the parameter's value, as a fraction
  of its range (−1..1; projects show −0.99..1); selectors and buttons: absolute.
- **Timing:** points are joined by lines (the Control Lane; recorded movements are
  "reduced" to few points). Before the first point the first value holds, after the
  last the last; when the pattern starts again (loop, restart) the parameter is back to
  its own value until the pattern plays. Maschine 2's step points (one per step) play
  as joined lines too. Ruling: `cost if wrong` = step data ramps slightly between steps.

## Behaviour
- **Playback:** while a pattern plays, each lane's value (the knob's value + the
  deviation, clamped to the range) drives the parameter; on stop, and in patterns
  without points for it, the parameter is its own value.
- **Recording (software):** dragging a knob's outer ring during playback records the
  movement into the playing pattern of that channel's group; ⌃-click (right-click) on
  the ring removes the knob's modulation. An Auto-write button (the header) makes plain
  knob drags record too (Touch Auto-write).
- **Recording (MK3):** hold AUTO (SHIFT+AUTO pins it) and turn knobs 1–8 in Control
  mode: the knobs record deviations (displayed as a percentage); ERASE + knob removes
  that knob's modulation; SHIFT + pad 10 (Clear Auto) removes all of the focused
  channel's. In Step mode, holding step pads and turning a knob sets those steps.
- Each touch (press to release) is one undo step; the points written replace the
  pattern's points for that parameter over the span touched.
- Only Sound and Group parameters (not the master's).

## How
- **Model:** `Sound::modulators` / `Group::modulators`: `Vec<ModTarget { slot:
  Option<usize>, field: String }>` (`slot: None` = a channel property); `Pattern::mods:
  Vec<ModPoint { sound: Option<usize> (None = the group), target: usize (index in the
  channel's modulators), position, value }>`.
- **Document:** `set_modulation(chain, slot, field, g, pattern, span, points)` (adds the
  lane when missing; one undo step), `clear_modulation(chain, slot, field)` (every
  pattern; reuses `automation.rs`'s lane/event removal), `clear_channel_modulation`.
- **Engine:** `Pattern` carries `mods: Vec<ModLane { sound: Option<usize>, slot, field,
  points: Vec<(tick, stored value)> }>` already mapped to the parameter's stored value
  (the app knows each knob's scale): per render span, the units whose lanes are playing
  get `set` with the field changed, and their own value back when the lane stops.
- **App:** a `ModTake` like `record::Take`: samples (tick in pattern, deviation) while
  writing, thinned to points (a point where the movement departs from a line by more
  than 1/200 of the range) and written on release. The knob ring shows the modulated
  value (the playing pattern's lane at the play position).
- **MK3:** `NI.DATA.WORKSPACE.isAutoWriteEnabledFromHW`, `isAutoWriteEnabledFromHWModel`,
  `setAutoWriteEnabled`, `Workspace:getAutoWritePinnedParameter`, `Parameter:isAutomatable`,
  `isModulated`; control knobs show "%" text while auto-writing.

## Steps (test-first)
1. Model: read lanes and points, v0 and v1 (Atma v0: PlateReverb room size; Dance
   Template v1: Filter cutoff with −0.04 at 0).
2. Document: write points (v0 project → v0, new → v1), add/remove lanes; saved projects
   reload with the same points; Maschine-format check by roundtrip.
3. Engine: a lane moves a unit's parameter while playing, its own value after stop.
4. App: mapping deviation → stored value per knob scale; lanes sent with patterns.
5. App: ring-drag / Auto-write recording; thinning; one undo step; remove with ⌃-click.
6. MK3: AUTO (hold / SHIFT pin), knob turns record, ERASE+knob, SHIFT+pad 10; Step mode.
7. Docs (`docs/modulation.md`, PARITY); full check.
