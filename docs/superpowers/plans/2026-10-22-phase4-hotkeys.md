# Phase 4 — Hotkeys and the event area's menu

> Roadmap: `docs/superpowers/plans/2026-10-22-parity-roadmap.md`. Inline, test-first; every
> task works from the software and, where the MK3 has the function, from the MK3.

**Goal:** Maschine's computer-keyboard shortcuts and the Pattern Editor's mouse modes and
context menu, with the MK3's undo and nudge matching.

**Sources:** the software manual (pp. 22, 235–246: undo, Nudge Grid, mouse modes, editing,
nudging, cut/copy/paste, quantizing); `ActionHelper.lua` (SHIFT + pads 1–4); decompiles in
`out/decomp/events` (`EventPatternTools::toggleMuteOnNoteEvent`, `joinNoteEventWithNext`,
`splitNoteEvents`, `nudgeEventsInPatternRange`).

## Findings
- **Undo:** ⌘Z / ⌘Y are take undo / redo (a recording, or a modulation take, is one step);
  ⌘⇧Z / ⌘⇧Y undo / redo a single step, inside a take too. MK3 SHIFT + pads 1 / 2 are
  `undoTransactionMarker` / `redoTransactionMarker` (take); pads 3 / 4 the plain
  `undoTransaction` / `redoTransaction` (step). Outside a take, both kinds are the same.
- **Mouse modes:** Select (default), Draw (E), Split (X), Join (Y), Mute (Z): each key
  toggles between its mode and Select. Not while the computer keyboard plays notes (Keys).
  - **Split:** a click splits the note under the mouse at the step under the mouse (⌘:
    off the grid); a selected note splits every selected note playing there.
  - **Join:** a click joins the note with the next one on the same key (sound and pitch),
    filling any gap; a selected note joins each selected note with the next selected one
    on its key. Label "Join note events".
  - **Mute:** a click mutes or unmutes the note (a selected note: all selected). A muted
    note is probability 0 (Maschine 3 stores no flag): mute sets 0, unmute 100. Labels
    "Mute Note" / "Unmute Note". Muted notes are drawn grey.
- **Nudge:** ⌥← / ⌥→ move the selection (none selected: every note shown) by the Nudge
  Grid: Step, Step/2 (default), Step/4, Step/8, Step/16 of the Step Grid (grid off: the
  sequencer's finest). Notes nudged past an end come round from the other. The MK3's
  nudge uses the same Nudge Grid.
- **Cut / copy / paste:** ⌘X / ⌘C / ⌘V and the menu. None selected: every note shown.
  The first pasted note snaps to the Step Grid and the others keep their offsets to it.
  - From the menu: at the step under the mouse; the topmost copied sound (Group view) goes
    to the focused sound, the highest copied pitch (Keyboard view) to the row under the
    mouse.
  - From the keyboard: with the focus and playhead unchanged since the copy, one step after
    the originals; playhead moved (stopped): from the playhead; focus changed: the same
    times, onto the focused sound (Group view) or the same pitches (Keyboard view).
  - A paste past the pattern's end lengthens the pattern.
- **Event area menu** (right-click): the five modes; Quantize, Quantize 50%; Cut, Copy,
  Paste, Delete; Nudge Grid ▸ Step … Step/16. Right-click on a note no longer deletes it.
- **Elsewhere:** ⌘A selects every note shown; Del / ⌫ deletes the selected notes, or the
  focused sound or group when the Sound or Group list was clicked last; ⌘X / ⌘C / ⌘V and
  ⌘R (rename) act on that list's sound or group the same way.

## Tasks
1. **Step undo** (`crates/doc`): a take keeps its parts; `undo_step` / `redo_step`.
   Tests: inside a take, step undo takes back the last part only and take undo the rest;
   step redo puts it back; outside a take, step undo = undo. Lua host: the Marker calls
   take, the plain calls step (`tests/scripts.rs`: SHIFT + pad 3 undoes one part of a
   take).
2. **Edit menu and shortcuts** (`menu.rs`): Undo ⌘Z, Redo ⌘Y, Undo Step ⌘⇧Z, Redo Step
   ⌘⇧Y. Test: the menu's shortcuts, and that ⌘⇧Z steps.
3. **Note tools** (`crates/doc/src/notes.rs`): `split_notes`, `join_notes`, `toggle_mute`,
   `nudge_notes` (with wrap), `paste_notes`. Unit tests from the findings.
4. **Mouse modes** (`App.mouse_mode` replaces `draw_mode`): E / X / Y / Z, the pencil, the
   cursor, Split / Join / Mute clicks, muted notes grey. Tests through the harness.
5. **Nudge Grid and ⌥ arrows**, shared with the MK3's nudge. Tests.
6. **Event clipboard and the context menu.** Tests.
7. **Lists:** Del, ⌘X / ⌘C / ⌘V, ⌘R on the last-clicked list. Tests.
8. Docs: PARITY rows, this plan's results; the full check.

## Results
- Done: tasks 1–6; task 7 for sounds (⌘X / ⌘C / ⌘V, Del = reset, ⌘R) and ⌘R for groups.
- The MK3's Grid page (SHIFT + FOLLOW ▸ STEP): its pads set the Step Grid (Maschine's
  15 values, pad 16 OFF), knob 4 the Nudge Grid; both shared with the app both ways
  (`State::sync_grid`). The app's GRID menu lists the same 15 values.
- The MK3's POSITION knob and nudges now move by the Nudge Grid (the manual's rule);
  two older script tests expected the step grid and were corrected.

## Rulings
- ⌘V with nothing changed pastes from the first step after the copied notes end ("one
  step after the original events" read as after them all, so a copied bar follows
  itself). If wrong: pastes land later than Maschine's.
- The stopped-playhead paste rule isn't done: DECKARD has no playhead to move while
  stopped. If wrong: a paste after moving the playhead goes by the focus rule instead.
- Selected notes in Join pair up per key from the left (a+b, c+d), Maschine's exact
  chaining unread. If wrong: three selected notes on a key join two, not three.
- Group Del / ⌘X / ⌘C / ⌘V moved to Phase 8 with the Group menu's Cut / Copy / Paste /
  Delete (the same missing document operations).
- ⌘A in the lists isn't done: DECKARD has no multiple selection of sounds or groups.
