# 33 — A static OSO dies with the page it was opened on

Status: `done` (2026-11-20) · HIL: **no** (the rule is NI's, and it is covered headless) · Owner: any machine.

## What

`REC MODE` stayed up when the controller's page changed; in Maschine it goes. NI's
`onControllerPageChanged` hides every **static** OSO unless SWING/TUNE is held
(`ParameterHandlerJam.lua:180-196`), and the HIL capture records the same thing from the other
side: `REC_MODE` showing, then a STEP page push, and the type goes `NONE` on that tick
(`hil/jam-overlay-2026-11-16.md:158`).

Two lifetimes, from `JamHelper.isJamStaticOSOVisible` / `isJamNonStaticOSOVisible`:

| | static | non-static |
|---|---|---|
| types | `GRID`, `TEMPO`, `REC_MODE`, `RANDOMIZER`, `PERFORM_FX` | the rest — `SCALE`, `ARP_REPEAT`, `SWING`, `TUNE`, `LOCK`, `NOTES` |
| closed by | its key again, the page changing, the device going | the idle timer (70 ticks ≈ 2 s), an explicit close, a page change |

(Corrected 2026-11-20 against `docs/superpowers/specs/2026-11-16-jam-overlay-refs.md` §4 — this item
first listed PERFORM FX as non-static, and `NOTES` never arms the timer on press either. Item 34
carries the TEMPO OSO that the static list implies.)

## How it reached the script

The rule needs the controller's page, which a device could not see: its own `deckard.page()` is
its own runtime's page, not the controller's. So the fact is published — `View.page`, read by
scripts as `deckard.state().surface_page` — and each menu records the page it opened on
(`st.menu.page`). A static menu whose page no longer matches is closed on the tick.

## Tests

`crates/surface/src/jam_tests.rs::a_page_change_drops_a_static_oso`: `SHIFT+GRID` opens `REC MODE`;
changing `View.page` closes it; the same change leaves the non-static `SCALE` OSO up for its timer.

## Open

- Whether a *non-static* OSO should also die on a page change when it is mid-edit — NI arms the
  timer on show and the leave-function covers the rest, which our idle timer stands in for. Worth
  a rig look, not a code guess.
