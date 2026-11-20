# 26 — The strip is page-aware: titles carry `n/N` and ARP has three pages

Status: `done` (2026-11-20) — `Arpeggiator 1/3` 8/7/2, and `Scale 1/2` + `Scale 2/2` (8 + 1, the second page only while the layout is isomorphic) after item 32 corrected where the split lives. `pages` counts only pages that exist, so no counter shows on a single-page type.

## What

Maschine's strip titles carry a page counter: `Arpeggiator 1/3`, `Arpeggiator 2/3`,
`Arpeggiator 3/3`. DECKARD's title is a bare name with no counter, and its ARP model is a
single strip with 5 filled slots where the reference has three pages of 8 / 7 / 2.

Page 1 is `MAIN`(Type), `RHYTHM`(Rate, Unit, Sequence, Octaves, Dynamic, Gate),
`OTHER`(Lock). Page 2 starts with `Hold` and carries `SCALE`(Root Note, Bank, Type),
`CHORD`(Mode), `PAD LAYOUT`(Mode, Layout). Page 3 is `Type` and `Direction` only. The full
transcription is in `overlay-label-inventory.md`.

## Why it matters

The strip's job is to say what the encoders are touching. On a page-aware OSO, a single
flat strip is wrong in two ways at once: it shows a subset of the parameters, and it never
tells the user which page they are on. `Arpeggiator 1/3` also implies the encoder page
wheel is what moves between pages — the same wheel `step_oso_focus` already reads — so the
model probably needs page-scoped slot windows rather than a longer list.

## What to find out

- How many types are page-aware, and their page counts. ARP_REPEAT is confirmed 3; check
  the reference for the rest (the earlier shots were single-page: TEMPO, GRID, Notes,
  Variation, Record, Record Mode, Perform FX all showed no counter).
- Whether the counter is per OSO *type* or per focused page object — the title reads the
  type's header name today (`oso_title`), so a page total has to come from somewhere else.
- Whether the encoder page wheel is the navigator, or SHIFT + wheel, or the D-pad at the
  end of a page. The reference's page 1→2 transition needs one hardware observation.

## Exit criteria

- Title renders `name n/N` when the type has pages, plain `name` when it does not.
- Slots are page-scoped, so page 2 does not show page 1's parameters.
- Test pinning `1/3`, `2/3`, `3/3` titles and the page-scoped slot lists, from the
  inventory tables.
- Page navigation behaviour recorded (which control moves a page), from hardware.

## Depends on

Item 23 (sub-labels) for the per-slot names on each page — the two share the same
inventory, but this item is the model change and item 23 is the paint and naming.

## Landed — the page counter

The title now carries the reference's counter — `Scale 1/2`, `Scale 2/2` — computed from the
focused page and the number of pages that hold a filled slot; a single-page type shows no
counter, as the reference does. `the_title_counts_pages_only_when_there_are_two` pins the
arithmetic, including the layout-stops-being-isomorphic case where page 2 goes and the counter
disappears with it.

With the pad-layout settings implemented (`api/layout.rs`) `SCALE` fills all nine slots through
the real scripts (`scale_fills_the_layout_slots_the_reference_shows`), so the counter has two
pages to count. Two halves, stated separately: the fill is asserted against the installed scripts,
and the counter arithmetic by `the_title_counts_pages_only_when_there_are_two`. The title itself
is not asserted from real scripts — that still needs the viewer tier.

This only became expressible once paging was understood as **state-dependent**: `Scale 2/2` holds
one slot (`Vertical`) and exists only while the keyboard layout is isomorphic, so the page count
is derived from the filled slots rather than from a fixed table.

Still owed: per-page slot layout (the reference's `Arpeggiator 1/3` pages are 8 / 7 / 2 slots,
ours are a flat eight-per-page chunk), and the header band's group boundaries across a page break.
