# Maschine 3 context menus (Sound / Group / Pattern / Scene / Section / Events)

What Maschine 3 shows when you right-click (macOS: Ctrl-click) a "track" (a Sound slot)
and the other objects around it, so that DECKARD can copy it. The sources are:

- **[bin]**: the `analysis/maschine_arm64` binary. The menu builders are decompiled in
  `out/decomp/menus/` and `out/decomp/menus2/` (see `INDEX.tsv` in each). Labels are inline
  `std::string`s, or `MenuStrings::getGP("<Id>", "<English>")` wide strings whose ids appear in
  the UTF-32 string table (e.g. `PatternItem.Clear`, `SceneItem.AppendToSong`).
- **[man]**: the NI Maschine Software manual (docs.native-instruments.com), the chapters
  *Managing Sounds, Groups, and your Project*, *Working with Patterns and Clips* and
  *Working with the Arranger*, including their menu screenshots. Most screenshots are from
  Maschine 2. The `MAS3-SW_*` screenshots are from Maschine 3.
- **(?)** marks anything inferred or unconfirmed.

How the menus work in general (NWL `PopupMenu`/`MenuBar`) [bin]:
- Items are plain text. No item draws a keyboard-shortcut hint. Separators are real `addSeparator()` calls.
- Disabled items are greyed out but still visible (flag `item+0x50`, "enabled").
- The colour submenu is `ColorPicker::addColorPickerSubmenu`. It is a single custom widget
  holding the **16-colour palette (4×4)**, then an optional **"Default"** item ("D Default" in the screenshot).
  The current colour is highlighted. With a multi-selection whose colours differ, nothing is highlighted.
  A pick applies to **all selected** objects (`ObjectVectorAccess::setObjectsColor<T>`; undo "Set Sound Color" / "Set Group Color" / "Set Pattern Color" / "Set Scene Color").
- Right-clicking first **focuses** the clicked object (`SoundManagerItem::openItemContextMenu` →
  `GroupAccess::setFocusSound` + `ObjectVectorAccess::setFocusObject`). It then builds the menu, which
  acts on the **selection** (multi-select via Cmd/Shift-click). (?) Whether right-clicking an unselected
  slot drops the existing multi-selection: it probably behaves like a left click (select only that slot), but this is not verified.

---

## 1. Sound slot (Sound List left of the Pattern Editor; also the Pad-view pad grid and the Mixer Sound channel header)

`SoundItemHelper::populateContextMenu(Maschine&, PopupMenu&, Group&, Sound&, renameCb)`. It is called from
`SoundManagerItem` (Sound List / pads) and `SoundChannelItem` (Mixer). `n` = number of selected Sounds.

| # | Label | Enabled when | Action [bin] |
|---|---|---|---|
| 1 | `Open...` | n == 1 | `GroupFileCommands::loadFocusSound` opens a file dialog (Load Sound, `.mxsnd`) and replaces the focused slot |
| 2 | `Save` | n == 1 | `saveFocusSound` overwrites the source file. For factory or never-saved Sounds it turns into Save As [man] |
| 3 | `Save As...` | n == 1 | `saveFocusSoundAs` opens a save dialog. The default name is the slot name, in the user library |
| – | `Save Selected...` | – | only with internal feature flag 0xb (hidden in release builds). Omit it |
| 4 | `Save with Samples...` | n == 1 | `DialogAccess::openCollateDialog(Sound)` |
| 5 | `Bounce` / `Bounce Selected...` (when n > 1) | **only shown in Ideas view** (`StateHelper::isIdeaSpaceFocused`) | Shows a confirmation dialog (Allow Once / Always Allow / Don't Allow), then renders each Pattern's notes to audio. The Sound's plug-ins are replaced by an Audio plug-in with one Take per Pattern. Undo "Bounce Selected Sounds" [man+bin] |
| — | separator | | |
| 6 | `Purge Missing Samples` | only shown if `SoundAlgorithms::hasLocationIssues` | `MissingSamplesAccess::purge<Sound>` |
| 7 | `Find Missing Samples` | (same as 6) | `MissingSamplesAccess::resolve<Sound>` |
| — | separator | (only together with 6–7) | |
| 8 | `Import MIDI...` | n == 1 | `importMIDIToSound` loads a MIDI file into the **selected Pattern** for this Sound. It replaces this Sound's notes and modulation there and ignores MIDI channels [man] |
| 9 | `Export MIDI...` | n == 1 | `exportMIDIFromSound` writes the selected Pattern's events for this Sound to `.mid` |
| — | separator | | |
| 10 | `Color` ▸ | always | palette + Default, applied to all selected Sounds |
| — | separator | | |
| 11 | `Cut` | always | `ClipboardAccess::cutSoundWeak(group, selectedSounds)` copies the Sounds to the clipboard and resets their slots to the default state [man]. Undo "Cut Sound" |
| 12 | `Copy` | always | `copySoundWeak(group, selectedSounds)` |
| 13 | `Paste` | `ClipboardAccess::canPasteSound()` (clipboard holds Sounds) | `pasteSoundWeak(group, indexOfClickedSlot, true)`, undo "Paste Sound". It replaces the clicked slot, and with several copied Sounds the following slots in order, ignoring gaps in the source selection. Copies that would go past slot 16 are dropped. The Sound's **events are pasted too**: Patterns with the same index are created in the target Group if missing, and this slot's events in existing Patterns are replaced [man]. The clipboard persists, so you can paste many times |
| — | separator | | |
| 14 | `Rename` | n == 1; **only present if the caller passes a rename callback** (Sound List and Mixer do) | starts inline name editing (Enter confirms, Esc cancels). Undo "Set Sound Name" |
| 15 | `Reset` | always | `ObjectVectorAccess::resetObjects<Sound>(selected)` empties the slot: removes plug-ins and resets channel properties, name ("Sound N") and colour to defaults [man]. Undo "Reset %s" / "Reset selected %s(s)". The keyboard **Del** key does the same [man] |
| – | `Export to iMaschine...` | – | feature flag 0xb only (internal). Omit it |

Real Maschine 3 screenshot [man, MAS3]: `Open..., Save, Save As..., Save with Samples..., Bounce | Import MIDI..., Export MIDI..., …`.
The Maschine 2 screenshots show the rest of the list: `| Color ▸ | Cut, Copy, Paste(grey) | Rename, Reset`.

Not in the menu (so don't invent them): Solo/Mute (done with the slot's own mute button: click to mute,
right-click to solo [man]), Duplicate (Alt-drag a slot instead, undo "Duplicate Sound(s)"), Choke/Link
(Pad-view settings), "Clear events".

**Left-click** [man]: an unselected slot is selected and focused. A selected slot is focused and the selection is kept.
Cmd-click toggles the slot in the selection (the focused slot can't be deselected). Shift-click selects a range. Cmd+A selects all, or
selects only the focused slot if all are selected already. Focusing a Sound also shows its channel/plug-ins in the Control area and,
in Keyboard view, its notes. Drag (or multi-drag) moves slots; dropping a slot on a Group moves it there without its
events. Alt-drag duplicates.
**Double-click** the name: inline rename.
**Del** key: Reset.

## 2. Group slot (Group List in Ideas/Song view; Mixer Group header; Group name at the top-left of the Pattern Editor)

`GroupItemHelper::populateContextMenu(Maschine&, PopupMenu&, Song&, Group* g, renameCb)`. `n` = number of selected Groups.

If `g == nullptr` (right-click on the empty "+" area, (?) exact spot), the menu has a single item, **`Paste`**,
enabled when `canPasteGroup()`, which pastes the Groups as new Groups at the end.

Otherwise:

| # | Label | Enabled | Action |
|---|---|---|---|
| 1 | `Open...` | n == 1 | Load Group dialog (`.mxgrp`) replaces this Group |
| 2 | `Save` | n == 1 | save to its file (or Save As for factory/new Groups) |
| 3 | `Save As...` | n == 1 | Save Group dialog |
| – | `Save Selected...` | feature 0xb only | omit |
| 4 | `Save with Samples...` | n == 1 | collate dialog (Group path + "Delete Unused Files") [man] |
| — | separator | | |
| 5,6 | `Purge Missing Samples`, `Find Missing Samples` + separator | only if `GroupAlgorithms::hasLocationIssues` | |
| 7 | `Import MIDI...` | n == 1 | imports a MIDI file into the selected Pattern of the Group. MIDI channels or notes are mapped onto Sounds [man] |
| 8 | `Export MIDI...` | n == 1 | `onExportMidiItemClick`: the whole Group's selected Pattern to `.mid` |
| 9 | `Group MIDI Batch Setup` ▸ | — | `Sounds to MIDI Channels`, `Sounds to MIDI Notes` (`GroupAccess::applyGroupMidiBatch`; an item shows checked while that batch layout is still in effect, (?)) |
| — | separator | | |
| 10 | `Color` ▸ | | palette + Default, applied to the selected Groups |
| — | separator | | |
| 11 | `Cut` | | `cutGroupWeak(song, selected)` puts the Groups in the clipboard and **removes** them. Later Groups shift up [man]. Undo "Cut Group" |
| 12 | `Copy` | | `copyGroupWeak` |
| 13 | `Paste` | `canPasteGroup()` | `pasteGroupWeak(song, index, …)` **replaces** this Group (and the following ones for multi-paste, creating Groups that don't exist yet), including all Patterns and Sounds. Undo "Paste Group" |
| — | separator | | |
| 14 | `Rename` | n == 1; only if a rename callback exists (Group List and Mixer have one; **the Pattern Editor header menu has none**, confirmed by screenshot) | inline rename |
| 15 | `Delete` | | `ObjectVectorAccess::removeObjectsResetLast<Group>` removes the selected Groups, and later ones shift. Deleting the last remaining Group **resets** it instead. Same as the Del key [man] |

**Left-click**: focuses the Group, and the Pattern Editor and Control area show it. Selection modifiers work as for Sounds.
Clicking the letter+number label (e.g. "A1") mutes the Group; right-clicking the label solos it [man].
**Double-click** the name: rename. Alt-drag duplicates (undo "Duplicate Group(s)"). A plain drag reorders.

## 3. Pattern (Ideas-view pattern cell; Pattern Manager slot / pad cell)

`PatternItemHelper::populateContextMenu(Maschine&, PopupMenu&, Group&, EventPattern&, index, renameCb)`. It only
opens on an **existing** Pattern; an empty cell has no menu (double-click it to create a Pattern) [bin+man].

| # | Label (`PatternItem.*`) | Enabled | Action |
|---|---|---|---|
| 1 | `Clear` | | `onClearPattern` → `EventPatternAccess::resetAllSequences` removes all events, modulation and audio from the Pattern and keeps the Pattern itself |
| — | separator | | |
| 2 | `Rename` | | inline rename (all references follow) |
| 3 | `Color` ▸ | | `EventPatternAccess::setColorWeak` (Default = the Group's colour) |
| — | separator | | |
| 4 | `Double` | `canDoublePattern` (false when the doubled Pattern would exceed the length limit, (?)) | `EventPatternAlgorithms::doublePattern` doubles the length and copies the content into the second half. Undo "Double EventPattern" |
| 5 | `Duplicate` | | `GroupAccess::duplicatePattern` → `duplicatePatternInsertBefore(index+1)` inserts a copy **right after** the original and shifts the following Patterns. Undo "Duplicate Pattern" |
| 6 | `Delete` | | deletes the Pattern. Clips/Scene references to it are removed too. Undo "Remove Pattern" (?) |

The Maschine 2 screenshot matches: `Clear | Rename, Color ▸ | Duplicate, Delete` ("Double" is new).
**Left-click** in Ideas view assigns the Pattern to the focused Scene for that Group, or removes it if it is already highlighted, and loads it into the Pattern Editor.
**Double-click on an empty cell** creates a new Pattern. Alt-drag duplicates.

## 4. Scene (Ideas-view top row; the Scene cells of the Scene Manager)

`SceneItemHelper::populateContextMenu` (ids `SceneItem.*`). Confirmed by the MAS3 screenshot:

`Create`, `Append To Song`, `Clear` | `Rename`, `Unique` (enabled only when the Scene shares Patterns with another Scene: `!isSceneUnique`), `Color` ▸, `Scene Tempo` (a toggle; enabled when `isSceneTempoAvailable`) | `Duplicate`, `Delete`

- Create: makes a new empty Scene. Append To Song: adds a Section with this Scene at the end of the arrangement.
- Clear: removes all Pattern references, leaving the Scene empty (undo "Clear Scene").
- Unique: `makeSceneUnique` gives the Scene its own copies of the Patterns.
- Scene Tempo: gives the Scene its own BPM, which the header BPM control then edits.
- Duplicate: `duplicateScene`. Whether the copy references the same Patterns or copies them is set in Preferences › Default › Scene/Section › Duplicate ("Scene Only" / "Scene and Patterns").
- Delete: `removeScene`, undo "Delete Scene".

**Double-click** the name: rename. **Click**: select the Scene (the playhead jumps to it).

## 5. Section (Song-view top row / Section Manager)

`ArrangerHeader::createSectionHeaderMenu` (ids `ArrangerHeader.*`):
`Create`, `Select` ▸ (`None` + Scene list, with the current one checked; other modes of the same submenu are `Insert` ▸ and `Insert as Clips` ▸), `Clear` |
`Rename`, `Unique` (enabled when the Section is not unique), `Color` ▸, `Scene Tempo` | `Auto Length` (enabled only when the Section has a manual length),
`Duplicate`, `Convert to Clip(s)` (enabled when there is anything to convert), `Delete`.
The Section Manager variant (`populateSectionItemContextMenu`) is the same minus Scene Tempo/Convert, and its last item is `Remove`.

## 6. Notes/events in the Pattern Editor (Event area, Group or Keyboard view)

`EventPatternEditorGrid::openContextMenu` → `NoteEventContextMenuHelper::populateContextMenu`. The same menu
opens on a note or on the background.

1. Mouse modes: `Point`, `Pencil`, `Split`, `Join`, `Mute` (strings in the binary; each item has an icon). In the MAS3
   manual screenshot they are labelled **Select, Draw, Split, Join, Mute**: the arrow / pencil / scissors / + / × icons. (?) Use the manual labels.
   Keys: E = Draw, X = Split, Y = Join, Z = Mute (each toggles with Select). No shortcut text is drawn in the menu.
2. separator
3. `Quantize`, `Quantize 50%`, both enabled when `EventPatternTools::canQuantize`. Undo "Quantize events".
4. separator
5. `Cut`, `Copy`: enabled only when at least one note is selected (`getNumSelectedNoteEventsInPatternRange != 0`). Undo "Cut Events".
6. `Paste`: enabled when the note clipboard is non-empty. It pastes at the step nearest the mouse. In Group view the topmost copied Sound lands on the focused Sound; in Keyboard view the highest pitch lands on the row under the mouse [man]. Undo "Paste note event(s)".
7. separator
8. `Delete`: enabled when events are selected. Undo "Delete events".
9. separator
10. `Nudge Grid` ▸: radio list of the nudge resolutions (Step, Step/2, Step/4, Step/8, ..., with the current one checked).
11. (audio Sounds only) separator + `Takes` ▸: pick the Take played by the clicked audio event.

Mouse in **Select** mode [man]:
- Click a note: select it (and audition it). Shift-click: add or remove it from the selection.
- Drag on the background: rubber-band selection. Click the background: deselect all.
- Double-click the background: create a note. Double-click a note: delete it.
- Drag a note: move it on the step grid (Cmd frees it from the grid). Alt-drag: duplicate.
- Drag a note edge: resize. Vertical drag: change Sound (Group view) or pitch (Keyboard view).

Keys: Del/Backspace delete, Cmd+X/C/V cut/copy/paste, Cmd+A select all, Alt+←/→ nudge, Cmd+Z / Cmd+Y undo/redo.

## 7. Clips / patterns in Song view (for completeness)

`MenuHelper::populateClipEventContextMenu`: `Clear` | `Rename` (enabled when ≤ 1 clip is selected), `Color` ▸ | `Double`,
`Duplicate`, `Delete`. On an empty cell, `Insert as Clip` ▸ lists the Group's Patterns. The timeline menu has
`Enable/Disable Loop` | `Auto-Loop Selection`, `Loop All` | `Convert to Clip(s)`.
