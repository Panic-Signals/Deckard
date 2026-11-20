# 23 — Slot sub-labels + group-span headers (display follow-up)

Status: `done` (2026-11-20, code at `26c89d1`) — sub-labels, spanning headers and the corner brackets all draw. The brackets are now driven by the script's published `editing` (one owner for the cursor), so a bracket that appears on the wrong slot is a script bug, not a renderer one.

## What

Hands-on comparison (REC_MODE): Maschine shows three text rows per slot —
group header (`METRONOME`), value (`-6.0 dB`), sub-label (`Level`); and one
group header spans its sub-slots (`METRONOME` over Level / `1/4` /
`Auto-On`). DECKARD shows two rows (group header repeated per slot, value)
and no sub-labels.

The sub-labels are not in the Lua `getOSOParameters` tables (sub-slots store
`""`) and `setParameter` passes only `(param, label)`. Per the Task 1 refs,
value text is the bound parameter's own display string — the sub-label is
presumably the parameter's C++ display name (internal names are `Volume`,
`TimeSignature`, `AutoEnable`, `CountInLength`; Maschine shows `Level`,
`Signature`, `Auto-On`, `Length`). That display-name source was never
recovered (refs §5 open point).

## Procedure

1. Confirm against Maschine which slots carry sub-labels and their exact
   text (item 17 sweep covers this).
2. If they are C++ parameter display names: recover the source (decompile
   `ParameterWidgetJam` label path or the DATA display-name table), add a
   `overlay_display`-style name helper, render a third slot line, and span
   one group header over adjacent same-header slots.
3. If any sub-label equals the Lua slot label instead, just render it (no RE).

## Exit criteria

- Strip shows header + value + sub-label per Maschine, group headers spanned;
  matrix row updated with the verdict.
- Commits: viewer/host code + tests as needed.

## Depends on

Item 17 (which slots need sub-labels). Produces viewer/host code.

## Source search — 2026-11-17 (searched, not found)

Where the display names are **not**:

- NI's own Lua: `RecModePage.lua:70-82` passes only parameter objects to the handler
  (`getVolumeParameter`, `getTimeSignatureParameter`, `getAutoEnableParameter`,
  `getCountInLengthParameter`, `getQuantizeModeParameter`). No strings. The scripts call
  `param:getDisplayName()` 56 times, so the display name is an API call on the parameter,
  not data any script holds.
- `Resources/Strings/` — no hit for `Auto-On`, `AutoEnable`, `Signature`, `Level`.
- `Resources/Maschine 3.rsrc` — no hit for the same.
- The app binary (`Maschine 3.app/Contents/MacOS/Maschine 3`, 593 MB) **has** `Level`,
  `Signature` and `Follow` as strings, but not `Auto-On` or `Count-In`, so at least some
  names are composed at runtime rather than stored whole.

Our host implements `getDisplayName` for `Group`, `Sound`, `EventPattern` and `Scene`
(`api/data.rs:950`) but **not for `Param`**, whose `getName` returns the internal name
(`AutoEnable`). So the sub-label cannot come from anything we already hold.

## Where the verdict went

There is no recoverable table, so the faithful source is the reference screenshots: one
capture per OSO, read off slot by slot, recorded with the shot name and date, and rendered
only for OSOs actually photographed. Anything unphotographed stays a gap — humanising
`AutoEnable` into "Auto Enable" would be inventing a name, which is the one thing the strip
must not do.

## Inventory recovered — 2026-11-17

Eight reference shots (ARP ×3 pages, Variation, Record Mode, Notes, Record, Perform FX)
give the names for those OSOs. Full transcription in
`../overlay-label-inventory.md`; nothing there is inferred.

What the inventory settles for this item:

- **Sub-labels are per-knob display names**, e.g. `Root Note`, `Bank`, `Type` under a single
  `SCALE` header — so the row under the value is the parameter's name, not a second header.
- **Headers span, and the first slot often has none** (`Humanize`, `Guitar`, `Hold` all sit
  under an empty header). Rendering an empty header is correct, not a missing lookup.
- **Slot counts differ from ours per OSO** (`Record` 6 vs our 4, `Variation` 5 vs our 3,
  `Notes` 3 = our 3). Those gaps are model gaps, not holes — track with item 26 and the
  matrix; this item only supplies names once a slot exists.
- **Page counters** (`Arpeggiator 1/3`) are item 26, not a sub-label concern.

## Landed — the two rows and the spanning header

Each slot draws its value with the parameter's display name beneath it, and a group header now
sits in its own band above the slots and **spans the group** (`SCALE` over three slots, `CHORD`
over its two, `PAD LAYOUT` over the last three on `Scale 1/2`).

The names come from `crates/lua-host/src/api/labels.rs`, keyed by our parameter name, transcribed
from the reference shots — NI's Lua passes parameter objects and never names, so there is nothing
to read at runtime. A parameter with no entry draws a blank row rather than a guessed label.

One rule worth keeping: **a header run is a change of header text, and nothing else.**
`JamOso::header` already inherits a page's header backwards onto every following slot, so a
filled slot carries its group's text and the run is just consecutive equal text. An earlier
version of this code guessed that a `None` header meant "continues the group" — dead for real
rows, and where it could fire it stretched the previous group across a slot on a guess. Lengths
come from slot numbers, so a hole inside a group still sits under its band rather than splitting
it.

**The reference page's eight sub-labels all render now (nine slots fill in total, the
ninth being page 2's `Vertical`), and the reason the last four did not is worth recording.** The reference's last
three slots (`Isomorph`, `Sequential`, `In Key`) and page 2's `Vertical` had **no parameter behind
them at all**: the host implemented no workspace keyboard-layout or isomorphic getters, so
`getScaleOSOParameters` filled nine entries with the last four holes — the `filled 5` of every
hardware log, and the reason `Scale 2/2` never appeared.

`crates/lua-host/src/api/layout.rs` adds those four settings, named for the values the reference
shows (`Isomorph`, `Sequential`, `In Key`, `Vertical`, plus `Chromatic` from
`PadModeHelper.IsomorphicTypeChromatic`). A value with no transcribed name renders empty rather
than as an index. `scale_fills_the_layout_slots_the_reference_shows` drives the real scripts:
nine slots filled, and those four names read off `overlay_display`.

Every option is named and indexed from two independent sources that agree where they can be
checked against each other:

- **The manual** (MASCHINE JAM, Keyboard mode, `PAD LAYOUT` table, p. 98 of 2.6.11) gives the
  **option sets**: `Mode` `Standard` / `Isomorph`; `Layout` `3rds` / `4ths` / `Sequential`;
  `Type` `In Key` / `Chromatic`; `Direction` `Horizontal` / `Vertical`.
- **The app's own saved state** gives the **indices**: `~/Library/Preferences/
  com.native-instruments.Maschine 3.plist` holds `JamKeyboardLayout`, `JamIsoLayout`,
  `JamIsoType`, `JamIsoDir`.

The values are **global preferences, not project data** — two projects saved with different pad
layouts carry `WorkspaceConfig` fields that are identical one by one (21 values, compared through
the decoder) and hold no layout field at all. That corrects
an assumption made here earlier (and worth keeping): a setting belonging to the "workspace" object
in the scripts does not follow the project file.

The two sources disagree on one point, and the stored value wins: the manual lists `Horizontal`
before `Vertical`, but `JamIsoDir = 0` on a machine last set to `Vertical` puts **`Vertical` at 0**.
That single entry is the one resting on the saved value alone, with no second witness — and the
`JamIsoLayout = 2` / `JamIsoType = 1` pair is what shows the plist reflects the *last* saved state
rather than the first.

| key | value saved | index |
|---|---|---|
| `JamKeyboardLayout` | 1 = `Isomorph` | agrees with `PadModeHelper.IsomorphicKeyboardLayout` |
| `JamIsoLayout` | 2 = `Sequential` | manual's order 3rds / 4ths / Sequential |
| `JamIsoType` | 1 = `Chromatic` | agrees with `IsomorphicTypeChromatic` |
| `JamIsoDir` | 0 = `Vertical` | contradicts the manual's listing order |

**A short table is a reachability bug, not a labelling one.** `Layout` was capped at two values by
its table length, so `Sequential`, the manual's third, could not be selected at all.
`every_option_the_manual_lists_has_a_name` walks each table, asserts every option is named and the
clamp is the last index.

**Our host still keeps these in memory** (`State::params`), so they reset each launch. The fix is
not the project format — it is a settings file keyed like the plist above.

Two traps met on the way, both worth remembering: a knob's `setValue(1)` stores a **number**, so a
value table that matched only `Prop::Int` silently fell through to the raw index; and the label
and display wiring lives in `overlay_display`'s chain, where a patch that assumed the wrong
indent lands nowhere and fails silently.
