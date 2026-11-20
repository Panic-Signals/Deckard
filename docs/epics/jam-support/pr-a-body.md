# PR A — the Jam's on-screen overlay, in a window of its own

Draft body for a pull request to `Panic-Signals/Deckard`. Branch `jam-oso`, cut from
`upstream/main` (`f2b5ea3`), 22 commits. Nothing here is PR B: no `jam_server`, `jam_layout`,
`jam_input`, `jam_lights`, `onscreen_jam`, `mk3_server`, `pad_play`, and no MK1 layer.

## What it does

A Maschine Jam's ENCODER is blind in DECKARD today: the script runs, the lights work, and the
on-screen parameter overlay (the manual's **OSO** — the on-screen overlay) draws nothing. This
adds it: the page the script publishes, rendered in a **chromeless floating window** that shows
while a page is published and disappears with it.

Four parts, in the order the commits take them.

**The contract** (`docs/epics/jam-support/oso-contract.md`). Pages live in the script; rendering
and input routing live in the host; the script owns the slot's behaviour. The script publishes
`deckard.oso{title, page, pages, slots, focus, editing}`; the host stores it (`OsoPage`,
`OsoSlot`), draws it, and hands **every** ENCODER press and turn back to the script. There is one
cursor — the script's — on every page. The viewer-side cursor that drifted from it is gone.

**The window** (`apps/viewer/src/jam_window.rs`). A second eframe viewport:
`show_viewport_immediate`, no decorations, always on top, never activated when shown. Its bar
carries the page's name and the `n/N` counter (`Scale  1/2`); the bar is also its only handle —
dragging it moves the window, and the body has no click target at all, because the strip belongs
to the encoder. Its place is remembered as an offset from the app window, so it comes back beside
the app on whatever display the app opens on, clamped so it can never be restored off-screen.

**The pages** (`crates/surface/lua/devices/maschine_jam/main.lua`). One composer per OSO type, a
menu cursor that steps over items a page cannot show, the settings read as a projection rather
than a push, and the two lifetimes from the reference's own model — static types die on their key
or on the page change (`View.surface_page`), everything else on a ~2 s idle timer.

What the rig settled, once each page was in front of hands on a Jam:

- The strip is **eight columns**, `width / 8`, and nothing stretches: a page of one slot fills the
  first and leaves seven empty, and a column keeps its width when its page does not (`a105011`,
  `6e8a56e`). The bar is a real bar — dragging it moves the window (`e819c76`).
- Every page **opens on its own button and closes on the same one**, and the idle timer reaches
  them all (`7404cca`). GRID keeps its light into REC MODE, and the piano roll gets its own Scale
  page rather than the full one (`1f83c5f`). PERFORM FX does time out (`265d13d`), the lock view
  opens its page on the switch (`e861cb3`), and SHIFT + E closes the page it opened (`97591bc`).

**The Groove page** (`b9ad3c5` … `f689b88`). `OSO_RANDOMIZER` was on the static list with nothing
behind it. It now publishes Maschine's **Variation** page: thirteen settings over two pages, five
of them in Humanize, static, closed by the encoder's press or another page. Its shape comes from
captures of the reference (`hil/oso-test-sheet.md`), its labels, ranges and steps from the MK3
manual's Variation chapter, and its **Mode** selects the engine:

| Mode | Engine |
|---|---|
| `Humanize` | the variation pass: velocity band, time shift, length scale. Nothing created |
| `Random` | the generator's Random — the same call the app's Generate ▸ Random button makes |
| `Euclid` | the generator's Euclidean mode — DECKARD's own addition, recorded as such |

Each setting lives where its engine keeps it: probability, note range and chords in the generator's
module; the velocity band, note length, time shift and the three distributions in the sound's
DECKARD field. They are the **sound's**, saved with the project. Unset, the engine behaves exactly
as before this branch.

## Review pointers

- `crates/surface/src/{lib,api,device}.rs` — the wire: `OsoPage`/`OsoSlot`, the `deckard.oso`
  publisher, `View.page` and `View.browser_open`, and the Variation page's `state().variation` /
  `deckard.variation` / `deckard.apply_variation`.
- `apps/viewer/src/jam_strip.rs` — the renderer. Pure functions of the published page: it draws
  the cursor where the script put it and settles nothing of its own, and it keeps no state.
- `apps/viewer/src/jam_io.rs` — the routing, plus the extra tick that republishes a page after a
  turn changed the state it was drawn from (PERFORM FX shows the mode it just changed to).
- `crates/doc/src/groove.rs` — `Variation`, `apply_groove_page` (the mode dispatch) and the
  generator that reads the page's slots.
- `docs/epics/jam-support/plan-2026-11-21-overlay-window.md` — the plan work-item 22 demanded:
  the viewport, the chrome, the lifetime rule, the saved offset, the platform facts, and the
  metrics, which are measured from captures because the reference keeps none in its binary.
- `docs/epics/jam-support/work-items/35-jam-variation-oso.md` — the Groove page's contract, its
  parameter homes, and the one reading still marked as DECKARD's.
- `docs/maschine-jam.md`'s deviation table — every place this deliberately differs from NI's
  manual, including the third mode and the settings belonging to the sound.

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

`cargo test --offline -p deckard -p deckard-surface -p deckard-doc`, green on the tip: 841 + 309 +
220 passing, the ignored tests pre-existing. The obligations the contract names each have a test
with the contract's own name:

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
- `jam_tests::shift_e_pressed_twice_closes_the_variation_page`,
  `the_variation_page_publishes_its_settings`,
  `the_variation_page_shows_euclid_as_its_third_mode`
- `groove::tests::every_variation_setting_round_trips`,
  `the_apps_path_reads_its_settings_and_varies_the_notes` — a setting is written where its engine
  reads it, and Apply varies the notes
- `groove::tests::the_jams_apply_and_the_generate_button_are_one_engine` — one seed, both paths,
  identical notes: "one engine" proved rather than promised

For a machine with no Jam, `--jam KEY` runs the script and presses those keys (in chord order), and
`DECKARD_JAM_TRACE=1` prints what eframe made of the strip:

```text
DECKARD_JAM_TRACE=1 ./target/debug/deckard --jam SHIFT --jam PAD_MODE --jam ENCODER_TOUCH
jam strip: native window at [695.0 524.0] size [1170.0 172.0]
```

`docs/epics/jam-support/hil/strip-window-2026-11-21.md` is the rig pass for the window, and
`hil/oso-test-sheet.md` is the end-user sheet: every page, its buttons, its slots, when it opens,
when it goes, and the two sequences still worth a second look (Random's invariants, and whether
Humanize's velocity band replaces a velocity or bounds a deviation).

## Out of scope

The virtual MK3/Jam panel line (PR B), NI-scripted controllers' OSO path (their
`setOSOType`/`addParameter` API stays in `lua-host`), and the desktop BROWSE overlay and `INSTANCE`
page, which stay in-app — `INSTANCE` is a different window with its own chrome, recorded as
known-absent in the sheet.
