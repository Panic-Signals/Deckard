# 28 — The strip: DECKARD's own OSO library, `PERFORM_FX` included

Status: `done` (2026-11-20, code at `26c89d1`) — the model exists inside the Jam's own script, and the strip draws it.

Part of the Jam script port (`jam-script-port.md`), slice 2. Supersedes the follow-up promised
in item 18.

## What

Write the OSO model DECKARD does not have. Today it exists only inside NI's scripts:
`OSOController.lua` (239 lines: focus, pages, edit mode, paint callbacks) and
`ParameterHandlerJam.lua` (695 lines: the lists for 14 OSO types). Our side supplies the state
machine's primitives (`setOSOType`, `clearParameters`, the end-of-fill marker, `step_filled`,
`nearest_filled`) but no model at all: `crates/surface` has no OSO code, because the MK3 draws
screens.

Deliver `crates/surface/lua/devices/jam/lib/oso.lua`: a list of `{name, value, type}` slots with
focus, pages, edit and the three painted rows, plus one list builder per OSO type.

`OSO_PERFORM_FX` is the reason this item is user-visible: item 18 recorded Maschine showing one
slot (`Tremolo` over `Mode`, focused) where DECKARD shows nothing, and the FX-less path closing the
page on tick. Both behaviours belong here, so "no FX" and "FX not visible" stay distinguishable.

## Tasks

1. `oso.lua`: slot list, focus, page split, edit mode carried across a rebuild (NI's
   `WasEditMode`), sub-label row, spanning group header.
2. Builders for the types the Jam actually shows, starting with the ones this epic has
   transcriptions for: `OSO_SCALE`, `OSO_ARP_REPEAT`, `OSO_NOTES`, `OSO_TEMPO`, `OSO_SWING`,
   `OSO_TUNE`, `OSO_GRID`, `OSO_RANDOMIZER`, `OSO_SNAPSHOTS`, `OSO_REC_MODE`.
3. `OSO_PERFORM_FX`: build from the FX list and its parameter state (mode, engage, frequency, type,
   resonance), with the FX-less path closing the page.
4. Pages: `n/N` in the title, with `Scale 2/2` present only while the keyboard layout is
   isomorphic, and `Arpeggiator` at 8 / 7 / 2.
5. Read the four pad-layout settings from the host rather than `State::params` defaults; item 29
   supplies persistence but this item must not hardcode them again.
6. Tests: fill, focus stepping over holes, page reachability, and a `PERFORM_FX` case built from a
   synthetic FX list. No NI install in the loop.

## Exit criteria

- The strip renders from our scripts with `DECKARD_SCRIPTS` unset.
- Side-by-side on the rig: `SCALE` reads 9 filled, `+1` from 7 reaches page 2, ARP reads three pages,
  PERFORM_FX shows its slot with corner brackets under `SHIFT+PERFORM`.
- Focus survives a rebuild whose fill changes (4 / 5 / 6 / 8 / 9 was the observed spread).
- No `filled 5` regression on `SCALE`: that was the four-missing-settings bug, not a layout choice.
- Item 18 closes with this item as its evidence.

## Depends on

Item 27 (the Jam must boot from our scripts first). Item 29 is optional but preferred for the
pad-layout values; without it, keep them in `State::params` and say so in the commit.

## Risks

- **The 14-type surface is larger than it looks.** Lists whose content is C++-side (FX, macros,
  plugins) cannot be enumerated from Lua; they have to be built from the host API and the current
  selection. `PERFORM_FX` is exactly this case and is the pattern for the rest.
- **Do not port `OSOController` literally.** It carries controller-specific paint hooks. Keep the
  model in Lua, the painting in Rust (the viewer already paints the strip).

## Landed as (2026-11-20)

The model lives in `crates/surface/lua/devices/maschine_jam/main.lua`, not in
`devices/jam/lib/oso.lua`. One script set per device folder, and the Jam's is upstream's, so a
shared `lib/oso.lua` would have been a second home for pages that already had one.

Against the tasks this item listed:

- **Slot list, `focus`, page split, sub-label row, spanning group header** — landed: three rows a
  slot, one header band per group, holes keep their column.
- **Pages** — `Scale 1/1`, `Arpeggiator 1/3` (8 / 7 / 2), `Keyboard 1/2` (6 + 3, page 2 only while
  the layout is isomorphic).
- **The four pad-layout settings from the host, never hardcoded** — landed, through
  `deckard.state().settings.values`.
- **`PERFORM_FX` from the FX list, with the FX-less path closing the page** — landed, and it no
  longer reads a stale frame (the host republishes after applying what the script asked for).
- **Tests with no NI install in the loop** — landed: `jam_oso_pages_need_no_ni_install` and the
  rest of `crates/surface/src/jam_tests.rs`.

**Divergence worth knowing:** every menu renders through one generic composer
(`generic_oso_page`) instead of a builder per type. `OSO_NOTES`, `OSO_TEMPO`, `OSO_SWING`,
`OSO_TUNE`, `OSO_GRID`, `OSO_RANDOMIZER`, `OSO_SNAPSHOTS` and `OSO_REC_MODE` therefore show their
menu's items one to one, which matches the inventory for single-page types. A dedicated builder is
only worth writing where a type's page differs from its menu — nothing known does yet.

**Ownership rule settled here:** the script publishes the cursor and the edit flag, the renderer
draws them and settles nothing. That is `docs/epics/jam-support/oso-contract.md`.
