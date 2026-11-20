# 32 — The keyboard view draws no strip; `Scale` is the paged OSO

Status: `done` (2026-11-20) · HIL: **yes** (rows 2 and 5 of `hil/strip-repoint-2026-11-20.md`
were rewritten for it) · Owner: machine with the Jam.

## What the rig found

SHIFT+NOTE_REPEAT drew a strip titled `Keyboard` — five slots, its own `1/2` counter — before any
encoder touch. The question "why is that here at all?" had no good answer: the manual's table opens
every OSO on an **encoder touch** or a menu key, and no row shows a view switch drawing a strip.
The page was ours, from when the viewer owned the tab page and needed something to draw.

It was also hiding a second mistake. The reference's keyboard-mode OSO is `Scale 1/2` holding eight
slots (PAD LAYOUT over its last three) and `Scale 2/2` holding `Vertical / Direction` — nine items
split 8 + 1, page 2 existing only while the layout is isomorphic. We had built that split as a
separate page called `Keyboard` (6 + 3) and left `Scale` as one page of nine.

## What changed

- **Nothing is published for a keyboard view.** (*Superseded 2026-11-21 on the rig:* SHIFT + PAD_MODE
  now opens the `Scale` OSO — the keyboard's scales. What survives is the item's point: that page is
  `Scale`, never a page of the viewer's own.) `build_oso` has no branch for it: the strip is empty
  until the encoder is touched or a menu opens.
- **`Scale` is the paged OSO**: eight slots then one, `pages` `2` while the layout is isomorphic and
  `1` otherwise, the page derived from the cursor exactly as the ARP's is (`at` 9 → page 2).
- **The tab's whole cursor mechanism went with the page**: `keyboard_page`, `keyboard_focus`,
  `keyboard_edit`, `keyboard_oso_page`, `keyboard_press`, `settle_focus`, `oso_turn`, and the
  label→turn tables they used. The ENCODER in keyboard view is the script's again, and what it
  opens is the Scale OSO.
- **The confirm-and-advance binding is retired.** It only ever existed to drive the page that no
  longer exists; menus browse with the turn and press to edit, which is the reference's rule.

## Tests

`crates/surface/src/jam_tests.rs`: the keyboard view asserts an empty strip; the Scale OSO asserts
8 slots on `1/2` and `Vertical / Direction` on `2/2`; the fill-change test is now the Scale page-2
case (layout flips to Standard under a cursor that was on page 2, and one turn recovers onto a
filled slot). `apps/viewer/src/jam_tests.rs`: the strip tests render the Scale OSO, the browser test
hides and restores it, and the two tab-press tests were deleted with the behaviour they tested.

## Exit criteria

- A keyboard view publishes nothing, and the rig shows no strip until the encoder is touched.
- `Scale 1/2` and `Scale 2/2` as above, with no page that renders empty.
- No dead references to the tab cursor anywhere in the script.
