# 34 — TEMPO publishes its OSO

Status: `done` (2026-11-20) · HIL: **yes** (one row, `hil/strip-repoint-2026-11-20.md` row 7) · Owner: machine with the Jam.

## What was missing

Pressing TEMPO wrote the status line (`Tempo 120.0`) and lit the button, and the encoder changed the
tempo — but **no page was ever published**, so the strip showed nothing. `build_oso` had no tempo
case at all: tempo was a flag (`st.tempo`), not a page.

## The reference's shape

`OSO_TEMPO` is static (`overlay-refs.md` §4) and is **one slot**: `120.00`, with no group header and
no sub-label (`hil/jam-overlay-2026-11-16.md`, OSO_TEMPO). The wheel press toggles edit; the type
stays up between presses; 75 ticks later it is still showing, because static types never arm the
timer.

## Landed

- `tempo_menu()` — a one-item static menu, so the bracket, the page change and the closing press all
  come from the menu machinery rather than a second code path. It overrides the single-item default
  to open **browsing**, as the reference does (press ENCODER to start the edit).
- `tempo_oso_page()` — one slot, value `%.2f`, no `group`, no `sub`.
- The `st.tempo` flag is gone with its three readers: the encoder branch (the menu's `menu_turn`
  covers it), the LED (now "a Tempo menu is open"), and `meter_button`'s reset (it closes menus
  anyway).
- `PERFORM FX` is marked static too — the RE list is `GRID`, `TEMPO`, `REC_MODE`, `RANDOMIZER`,
  `PERFORM_FX`, and item 33's table had it wrong.
  **Superseded 2026-11-21 (rig):** PERFORM FX does go by itself, so it arms the idle timer
  and is not static. The rest of the RE list stands.

## Tests

`crates/surface/src/jam_tests.rs::the_tempo_button_publishes_its_oso` — the page and its shape, the
press-to-edit rule, the turn setting the tempo, and both closers (the page changing, the button).
`the_encoder_sets_what_the_meter_shows_and_says_so` gained the ENCODER press that the edit rule
requires.

## Not carried over from the parallel round

A worker in another worktree (`virtual-mk3`, uncommitted) solved the same gap with a bespoke page
plus a **2 s hold applied on top of `MENU_IDLE = 6`**. That double timer contradicts the measured
value: 70 ticks ≈ 2 s *is* the timeout (`overlay-refs.md` §4, and the rig). Main has `MENU_IDLE = 2`
and a static menu needs no hold at all, so only the page shape was taken from that round.
