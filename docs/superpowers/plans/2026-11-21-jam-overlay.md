# Jam overlay Implementation Plan

**Goal:** Touching a Maschine JAM's encoder opens the page it has — the settings, their
headers, and which one the encoder is on — in a small window of its own over the app,
appearing and disappearing the way Maschine's overlay does.

**Architecture:** The page model lives in the script (`deckard.oso` publishes
`{title, page, pages, slots, focus, editing}`), the host stores it (`OsoPage`/`OsoSlot`),
draws it (`jam_strip`), and hands every ENCODER press and turn back to the script. The
window (`jam_window`) is a second eframe viewport that is only painted while a page is
published, so the page's lifetime and the window's are the same fact.

**Tech Stack:** Rust workspace (`deckard-surface` with Lua pages on `mlua`, `deckard-doc`
for the Variation engine, the `deckard` viewer on egui/eframe 0.36). Lua tests run through
`cargo test -p deckard-surface`.

**Spec:** `docs/superpowers/specs/2026-11-21-jam-overlay-design.md`

## Global Constraints

- The script owns the page, the cursor and the edit flag; the host never computes them.
- Every ENCODER press and turn reaches the script, whatever the page type.
- A hole (`false`) keeps its column; `pages` never counts a page that cannot exist.
- Static pages (`GRID`, `TEMPO`, `REC_MODE`, `RANDOMIZER`, `PERFORM_FX`) never arm the
  idle timer; every other page closes after 70 ticks (about two seconds).
- The window is chromeless, always on top and never activated on show; nothing inside it
  takes a click except the bar, which drags it.
- No mark in the bar: the reference's is NI's, and this is not Maschine.
- Clean room: pages and behaviour come from the JAM manual, the panel and the running app.

## Review Focus

1. A page whose fill changes under a settling cursor stays on a slot, and a cursor on a
   hole underlines nothing. (Task 3.)
2. A turn that changes the state a page was drawn from shows the new state, not the old,
   and applying a request twice cannot happen. (Task 5.)
3. A static page survives the idle timer and dies with its page change; a timed page
   closes on the timer and comes back on an encoder touch. (Task 4.)
4. The window's saved place returns it beside the app on another display, and cannot come
   back off-screen. (Task 6.)
5. The whole overlay runs with no NI install: no script from NI, no library project.
   (Task 7.)

---

## Tasks

- [x] **Task 1 — the wire.** `OsoPage`/`OsoSlot` in `deckard-surface`, the `deckard.oso`
  publisher, `Device::oso`, and `View.page`/`View.browser_open` for the facts a script
  cannot see. Three fields were dropped after their first use: `label`, `menu` and `kind`,
  each required by the host and read by nobody once the press belonged to the script.

- [x] **Task 2 — the composer.** One builder per page in `maschine_jam/main.lua`, keyed by
  the page the Jam's own state selects, with the eight-plus-one `Scale` split, the
  Arpeggiator's three pages, and the two lifetimes.

- [x] **Task 3 — the renderer.** `jam_strip`: eight columns at `width / 8`, spanning
  headers left-aligned over their run's first column, values and sub-labels centred, the
  cursor a rule under each sub-label, editing as inset corner brackets. Pure functions of
  the published page; no state, and no settling of the cursor.

- [x] **Task 4 — the lifetimes.** Static pages recorded with the page they opened on, and
  dropped when `View.page` changes; timed pages on the script's own idle counter. The rig
  corrected two entries: Perform FX is static and closes on the encoder's press, and the
  keyboard view's switch opens the `Scale` page rather than nothing.

- [x] **Task 5 — the routing.** Every ENCODER press and turn goes to the script; the host
  applies what the script asks for, then ticks the script once more so the page it draws is
  the page the change produced.

- [x] **Task 6 — the window.** `jam_window`: a second viewport, the bar with the page's
  name and counter, the bar as the drag handle, the offset-from-the-app saved in
  `Settings`, and `clamp_axis` keeping the window on the app's display.

- [x] **Task 7 — tests and the no-NI proof.** The obligations of the spec each carry a test
  in their own name, and `jam_oso_pages_need_no_ni_install` runs the whole overlay with the
  scripts path unset.

- [x] **Task 8 — the Variation page.** `OSO_RANDOMIZER` had no page behind it. Building
  Maschine's Variation page needed an engine for its Mode: the document's Humanize pass,
  the generator's Random, and — DECKARD's own — Euclid. The page's settings are the
  sound's, saved with the project.

## What the rig changed

Every task above was written before the overlay was in front of hands, and the rig
corrected six things the tests could not:

1. "Unable to change" was a lie: the encoder had changed and saved the value, and the
   settings view the scripts read was never rebuilt after a step.
2. A header clipped to its own band cannot fit `PAD LAYOUT`; headers get the band's width.
3. The page's title was drawn before the row was allocated, so it cropped (`Keyboard 1/2`).
4. The `Mode` slot was a hole on a cold boot because the settings map was empty, not stale.
5. The strip was a row of filled cells: the reference is a flat bar with thin rules, and its
   width follows the app rather than the slot count.
6. The window could not be moved until the mouse-passthrough rule was reversed: the
   reference's strip is movable, so its bar drags it, and its body stays the encoder's.

## HIL

The two-window captures are on the rig machine, recorded per compositor rather than
asserted from a test: the window alone; over Ideas, Song and the Mixer; a timed page
closing and returning; a click through the strip onto the view beneath; and a restart that
brings the window back where it was, including on a second display.
