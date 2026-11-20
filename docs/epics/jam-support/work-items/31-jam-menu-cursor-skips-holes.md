# 31 — The menu cursor never rests on an item a page cannot show

Status: `done` (2026-11-20, code in the same commit as this item) · HIL: **no** (the rule is NI's,
and the strip's rendering of it is covered headless) · Owner: any machine.

## What

Turning the ENCODER through a script menu moved `m.at` by one *position* per notch and clamped it
to the list. The list holds items a page cannot show — `IsomorphicType` with the keyboard layout on
Standard is the standard example — so the cursor could come to rest on one. Two things followed:

- the strip had nothing to underline there (by design since the OSO contract: the renderer draws
  the published cursor and settles nothing), so the underline vanished;
- the status line named an item the user could not act on, since its `turn` changes nothing that
  shows.

## The rule

**One notch moves one place; the landing is never an item that does not show.** A notch onto such
an item carries on in the same direction until something does show; a turn whose whole run in that
direction is holes leaves the cursor where it was. A turn of `N` notches still travels `N` places,
clamped into the list — it does not count only the items that show.

This is NI's `incrementFocusParameter` (`ParameterHandlerJam.lua:342-393`, via `step_filled`), which
walks past unavailable parameters instead of landing on them. `step_filled` in
`crates/lua-host/src/api/gui.rs` is the same rule for NI's own OSO; the two paths now agree.

## The divergence it exposed

The menu's idea of "available" and the page's idea of "a hole" were two different functions, and
they disagreed: `scale_oso_slots` gated the pad layout's three detail slots on
`oso_is_isomorphic`, while the SCALE menu's items for the same three read
`oso_layout_value(s, …) or "-"` with no gate — so the menu offered an item the page drew as a hole.
Both now gate on the same predicate (SCALE and ARP menus).

**A cursor that skips holes is a lie unless the two lists agree.** The skipping is what finds the
disagreement, which is why it is worth having rather than settling the cursor after the fact.

## Tests

`crates/surface/src/jam_tests.rs`:

- `a_menu_turn_skips_unavailable_items` — with the layout on Standard, every landing of a long
  forward then backward turn names a filled slot, and the turn ends on the last and first filled
  slots respectively;
- `menu_cursor_moves_the_published_focus` keeps its 8-notch page crossing, which is the "N notches
  are N places" half of the rule.
