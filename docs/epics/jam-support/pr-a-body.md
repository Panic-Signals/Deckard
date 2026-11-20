# PR A — the Jam's on-screen overlay, in a window of its own

Draft body for a pull request to `Panic-Signals/Deckard`. Branch `jam-oso`, cut from
`upstream/main` (`f2b5ea3`), five commits. Nothing here is PR B: no `jam_server`, `jam_layout`,
`jam_input`, `jam_lights`, `onscreen_jam`, `mk3_server`, `pad_play`, and no MK1 layer.

## What it does

A Maschine Jam's ENCODER is blind in DECKARD today: the script runs, the lights work, and the
on-screen parameter overlay (the manual's **OSO** — the on-screen overlay) draws nothing. This
adds it: the page the script publishes, rendered in a **chromeless floating window** that shows
while a page is published and disappears with it.

Three parts, in the order the commits take them.

**The contract** (`docs/epics/jam-support/oso-contract.md`). Pages live in the script; rendering
and input routing live in the host; the script owns the slot's behaviour. The script publishes
`deckard.oso{title, page, pages, slots, focus, editing}`; the host stores it (`OsoPage`,
`OsoSlot`), draws it, and hands **every** ENCODER press and turn back to the script. There is one
cursor — the script's — on every page. The viewer-side cursor that drifted from it is gone.

**The script** (`crates/surface/lua/devices/maschine_jam/main.lua`) composes the pages: one
composer per OSO type, a menu cursor that steps over items a page cannot show, the settings read as
a projection rather than a push, TEMPO's page, and the two lifetimes from the reference's own
model — static types die on their key or on the page change (`View.surface_page`), everything else
on a ~2 s idle timer.

**The window** (`apps/viewer/src/jam_window.rs`). A second eframe viewport:
`show_viewport_immediate`, no decorations, always on top, never activated when shown. Its bar
carries the page's name and the `n/N` counter (`Scale  1/2`); the bar is also its only handle —
dragging it moves the window, and the body has no click target at all, because the strip belongs
to the encoder. Its place is remembered as an offset from the app window, so it comes back beside
the app on whatever display the app opens on, clamped so it can never be restored off-screen.

## Review pointers

- `crates/surface/src/{lib,api,device}.rs` — the wire: `OsoPage`/`OsoSlot`, the `deckard.oso`
  publisher, `View.page` and `View.browser_open`.
- `apps/viewer/src/jam_strip.rs` — the renderer. Pure functions of the published page: it draws
  the cursor where the script put it and settles nothing of its own, and it keeps no state.
- `apps/viewer/src/jam_io.rs` — the routing, plus the extra tick that republishes a page after a
  turn changed the state it was drawn from (PERFORM FX shows the mode it just changed to).
- `apps/viewer/src/surface_settings.rs` — the settings as a projection: the View carries them, so
  the scripts read them with nothing asking.
- `docs/epics/jam-support/plan-2026-11-21-overlay-window.md` — the plan work-item 22 demanded:
  the viewport, the chrome, the lifetime rule, the saved offset, the platform facts, and the
  metrics, which are measured from captures because the reference keeps none in its binary.

## The strip's metrics

The reference has no hardcoded strip metrics (its window state and styling are runtime), so the
numbers are measured off captures of Maschine 3.6.0: bar 28 pt over a 144 pt body (header 40,
value 40, sub 40, pad 24); header 11.5 pt, value 21, sub 13; eight columns always, `width / 8`, so a
page of one slot fills the first and leaves seven empty (nothing stretches), with 1 pt separators,
the group header left-aligned over its run's first column, and the value and sub centred;
the cursor a rule under each sub-label with the focused one bright; editing in inset corner
brackets.

One deliberate departure from the reference: **no mark in the bar**. The reference has an NI
wordmark; this is DECKARD and it does not wear another company's logo, so the bar is the page's
name and its counter, and nothing else.

`apps/viewer/src/main.rs` gains one drawing path that is not the window's: `--jam-shot` renders the
same painter into the app's viewport so `--screenshot` can capture it. A second viewport is not in
the main one's picture, and Screen Recording is not always granted — it is how the strip's own
pixels were reviewed and how CI can look at them.

## Testing

`cargo test --offline --workspace`, green on the tip. The obligations the contract names each have
a test with the contract's own name:

- `jam_strip::tests::the_underline_follows_the_published_cursor_wherever_it_moves`,
  `the_edit_mark_is_the_scripts_and_marks_only_the_focused_slot`
- `jam_strip::tests::a_page_whose_fill_varies_keeps_the_cursor_on_a_slot`,
  `a_cursor_on_a_hole_underlines_nothing`
- `jam_tests::menu_cursor_moves_the_published_focus`,
  `a_menu_turn_skips_unavailable_items`
- `jam_tests::a_published_page_keeps_its_cursor_on_a_slot_when_its_fill_changes`
- `jam_tests::a_turn_republishes_the_page_it_changed`
- `surface_settings_tests::the_scripts_read_the_settings_with_nothing_asking_for_them`
- `jam_tests::a_page_change_drops_a_static_oso`, `the_tempo_button_publishes_its_oso`
- `jam_tests::jam_oso_pages_need_no_ni_install` — the whole overlay with no NI install at all
- `jam_window::tests::*` — geometry, the offset band, and that an unmoved window writes nothing
- `jam_tests::the_strip_is_a_window_of_its_own` — a window sized from the page; no page, no window

For a machine with no Jam, `--jam KEY` runs the script and presses those keys (in chord order), and
`DECKARD_JAM_TRACE=1` prints what eframe made of the strip:

```text
DECKARD_JAM_TRACE=1 ./target/debug/deckard --jam SHIFT --jam PAD_MODE --jam ENCODER_TOUCH
jam strip: native window at [695.0 524.0] size [1170.0 172.0]
```

`docs/epics/jam-support/hil/strip-window-2026-11-21.md` is the rig pass, and it says what it is: the
window's existence, its size and its separateness are machine-verified (a native viewport, never
`EmbeddedWindow`, and the app's own screenshot holds no strip), while these still want hands on the
hardware and are **not** claimed here — moving it by the bar per compositor, that a click on the
body does nothing, the two-second idle and the static page's death on a page change, and the
window's place surviving a restart on a second monitor.

## Out of scope

The virtual MK3/Jam panel line (PR B), NI-scripted controllers' OSO path (their
`setOSOType`/`addParameter` API stays in `lua-host`), and the desktop BROWSE overlay, which stays
in-app.
