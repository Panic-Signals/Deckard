# Browser and saving: design

October 2026. Approved scope: the browser (software and MK3) and saving sounds, groups
and plug-in presets with tags. The cloud sample service (with affiliate codes) is a
later, separate project.

## Why

Finding and loading sounds is how a Maschine session starts, and DECKARD has no browser:
samples arrive only by drag and drop, and nothing can be saved except whole projects.
The user wants Maschine's browser as it is — the LIBRARY button's side panel and the
MK3's BROWSE — filtering everything by type and character, with their own saved things
kept apart from the sound packs, and a plain file browser for their own folders.

## Goals

- A browser panel on the left of the software, opened by a LIBRARY button, with two tabs:
  - **LIBRARY**: sections Projects, Groups, Sounds, Instruments, Effects, Loops,
    One-shots. Each filters by **Type** (three levels: type › sub-type › sub-sub-type)
    and **Character**, a **Factory / User** switch, a **Library** (product) picker,
    text search, **Favorites**, and a result list with a prehear player for samples.
  - **FILES**: a folder browser for the user's own files (samples, `.mxsnd`, `.mxgrp`,
    `.mxinst`, `.mxfx`, `.mxprj`), with pinned favourite folders.
- Loading: drag a result onto a pad, sound, group or the pattern area; or double-click
  (or Enter) to load into the focus — a project opens, a group replaces the focused
  group, a sound the focused sound, an instrument/effect goes into the focused slot, a
  loop or one-shot behaves as dropping that file does today (Sampler zone / Audio
  plug-in, `App::drop_samples`).
- **NI's library, when installed, appears the same way**: every Maschine expansion,
  NI instrument/effect preset and loop/one-shot Maschine knows, with its tags.
- The MK3's **BROWSE** works: NI's scripts run unchanged against DECKARD's browser.
- **Saving** Sound, Group, Instrument and Effect (and Save As / Save with Samples) in
  Maschine's own formats, with a dialog for name, Types, Characters and Library; saved
  items appear under User at once, and Maschine 3 opens them.

## Non-goals (now)

- The cloud sample service, purchases and affiliate links (next project).
- Editing tags of factory content, the tag/attribute editor beyond the save dialog,
  REX/MPC import, NKS light guide, Quick Browse (MK3 restores the last browse; store it
  only if cheap).
- Rescanning NI's libraries ourselves: when NI's index is present DECKARD reads it.

## Anti-goals

- Never write to NI's database, NI's library folders, or NI's favourites. DECKARD
  reads them only (a copy-free read-only open; tolerate Maschine having it open in WAL
  mode).
- Never copy, bundle or redistribute NI content; the browser points at files where they
  are.
- No NI code ported (clean-room rule): file layouts and database schemas are data.
- No slow start: the panel must open instantly with 50 000 items; queries in the
  background.

## What Maschine does (findings)

### The library index

Maschine 3 keeps its library in SQLite: macOS
`~/Library/Application Support/Native Instruments/Maschine 3/komplete.db3` (Windows:
`%LOCALAPPDATA%\Native Instruments\Maschine 3\komplete.db3`, to be confirmed on a
Windows install). Tables (schema read on this machine, Maschine 3, October 2026):

| table | holds |
|---|---|
| `k_sound_info` | one row per item: `id, name, author, vendor, comment, color, tempo, device_type_flags, bank_chain_id, content_path_id, file_name` (full path), `file_ext, uuid, file_size, mod_date, hidden_for, favorite_id, sub_path` |
| `k_content_path` | content roots: `path, content_type` (2 = factory product, 1 = user directory), `alias` (product name shown: "Deep Matter"), `visible`, `upid` |
| `k_bank_chain` | the Library › Bank › Sub-bank chain: `bcvendor, entry1, entry2, entry3` |
| `k_category` | Types: `category, subcategory, subsubcategory` (486 here) |
| `k_mode` | Characters: `name` (59 here) |
| `k_sound_info_category`, `k_sound_info_mode` | item ↔ type, item ↔ character |
| `p_sound_info_<Kind>_<1|2>` | per section, `1` = User, `2` = Factory. Kinds: Project, Group, Sound, Instrument, Effect, Loop, Oneshot (counts here: 8/117, 7/1406, 0/1872, 0/6678, 0/1010, 2/3179, 21/26618) |
| `f_sound_info*` | an FTS index over name, author, brand, product, bank, type, character, comment |

Favourites are in `~/Library/Application Support/Native Instruments/Shared/favorites.db3`
(read-only for us; DECKARD keeps its own favourites).

### In the files

Every Maschine file carries its tags itself: the `NISD/108` item holds an
`NI::SOUND::SoundInfo2` (`crates/mxprj/src/container.rs::new_sound_info` writes an empty
one: name, author, vendor, comment…, date, bank chain, types, modes, properties such as
colour, tempo, device type). Reading and writing its bank chain, types and modes is the
piece to finish. Samples (WAV) have no tags of their own in the file; NI's index tags
them by product (and Maschine's loop names carry a tempo, "Drums[110] OffScript 1").

User content goes to `~/Documents/Native Instruments/Maschine 3` (content path 28,
"Standard User Directory"; Maschine 2's and `User Content` are also user roots).

### The MK3

BROWSE in NI's scripts calls (all stubbed today, 51 errors): `App.getDatabaseFrontend()`
`.getBrowserController()` (`changeResultListFocus`, `hasJob`,
`loadSelectedResultListItem`, `toggleFavoritesFilter`), `.getBrowserModel()`
(`getFileTypeSelection`, `getSelectedFileType`, `has{Next,Previous}VisibleFileType`,
`getUserMode`, `getProductGroupSelection`, `getProductModel` (grouped products,
groups, sizes), `getBankChainModel` (slot items, selection), `getAttributesModel`
(active attributes = Types/Characters levels, selections), `getResultListModel`
(focus item, count)), `App.getBrowserControllerDeferredSearch().setUserMode`,
`NI.DATA.QuickBrowseAccess.restoreQuickBrowse`, `SamplePrehearAccess.
{isPrehearTypeSelected, shouldShowPrehearWidget}`, `Widget.InfoBar`, `Widget.showPicture`.
These name the model: file type → user mode → product (grouped) → bank chain →
attributes (types, characters) → result list.

## The design

### A `deckard-library` crate (new)

The index and queries, no UI:

- `Item { kind, user: bool, name, path, product, bank: [Option<String>;3], types:
  Vec<[String;3]>, characters: Vec<String>, tempo, color, author, favorite }`.
- `Kind { Project, Group, Sound, Instrument, Effect, Loop, OneShot }`.
- `Query { kind, user, product, bank, types, characters, text, favorites }` →
  results (sorted by name) plus the **facets**: the types and characters present in
  the current results, with counts (the browser greys or hides empty ones as Maschine
  does).
- Two sources merged into one view:
  1. **NI's index** (when the file exists): opened read-only (`rusqlite`, bundled
     SQLite, `SQLITE_OPEN_READ_ONLY`, `immutable` off so the WAL is honoured), queried
     directly with the schema above. Hidden items (`hidden_for <> 0`) are skipped.
  2. **DECKARD's own index**, `<settings>/library.db3` (SQLite, our schema): Maschine's
     user directories (`~/Documents/Native Instruments/Maschine 3`, and Maschine 2's
     when present) and what DECKARD saves — scanned in the background (rescan on panel
     open; a save adds its file at once). Tags from the file's SoundInfo2 for Maschine
     files. Samples there are listed untagged: as Loop when the name carries a tempo
     ("[94]", "94bpm", `apps/viewer/src/loops.rs`), else One-shot. **The user's own
     folders are not indexed** (the user's decision): they stay in FILES, a plain
     folder browser.
  When both list the same file (a user file Maschine indexed too), DECKARD's entry wins.
- DECKARD's favourites in its own index (by path), shown in both sources.

### Saving

- `deckard-doc` gains: `sound_file(g, s)`, `group_file(g)`, `slot_file(g, s, slot)` →
  bytes of `.mxsnd` / `.mxgrp` / `.mxinst` / `.mxfx`, laid out as Maschine writes them
  (the corpus round-trips today: compare against Maschine's own files byte for byte
  where possible), with the sound info filled in: name, author, Library (bank chain
  entry1 = "User" unless chosen), Types, Characters, colour, tempo.
- `mxprj` gains SoundInfo2 read/write of bank chain, types and modes (and the full
  property list), with a round-trip test over the local corpus.
- **Save with Samples**: the samples copied into a `<name> Samples` folder beside the
  file and referenced relatively, as Maschine does (seen: `Projects/Rest In Power2
  Samples/`).
- Where: Maschine 3's user directory, `~/Documents/Native Instruments/Maschine 3/`
  `Sounds`, `Groups` and `Plug-ins` (instrument and effect presets) — where Maschine 3
  saves (its index lists the user's groups there), so Maschine sees them too — or a
  folder the user picks in Save As. On Windows the same folders under the user's
  Documents.
- After saving, the file is added to DECKARD's index at once (no rescan).
- Menus: the Sound and Group context menus' Save, Save As…, Save with Samples… (greyed
  today) and Open…; the plug-in slot menu's Save Preset… The MK3's FILE page Save
  Sound/Group where the scripts offer it (saves with the current name and tags, no
  dialog).

### The software panel

- A **LIBRARY** button in the header toggles the panel (and F4, Maschine's shortcut).
  The panel docks left, resizable, and pushes the arranger right.
- Layout as in the approved mockup (`.superpowers/brainstorm/.../browser-layout.html`):
  tabs; section icons; Factory/User and Favorites; Library picker; Types as chips with
  the chosen level's children below; Characters chips (multi-select = AND, as
  Maschine); search box; result list (name, tempo for loops, product); prehear row
  (play on select toggle, volume).
- Prehear: samples play through the engine's prehear path (`SamplePrehearAccess`
  already exists for the controller); Sounds/Groups have no prehear unless the file
  has a preview sample (Maschine's `.previews` folder beside it: show it if present).
- FILES: the OS's folders from a root, up-folder, favourite folders (+ to pin),
  showing supported files only; the same loading.

### The MK3

- Implement the stubbed `DatabaseFrontend` / `BrowserModel` / `BrowserController`
  objects over `deckard-library` in `crates/lua-host` (`api/browser.rs`, new), with the
  query state in `State` (app pushes results, controller requests flow back as today:
  a `BrowseCall` like `SamplingCall`), so NI's BROWSE pages draw and LOAD loads
  through the same app loading path as the software.
- Sweep check: `lua-host-run --press BUTTON_BROWSE …` reports no new stubs and no errors.

## Builder decisions (the designer's calls, open to change)

- SQLite via `rusqlite` with the `bundled` feature (no system SQLite dependency on
  Windows/Linux).
- Samples outside NI's index are untagged (no folder-name or audio-analysis tagging).
- The panel shows NI's index as-is; it isn't refreshed while open (Maschine rewrites
  it when it rescans; we re-read on panel open).
- If NI's index is absent (Linux, or no Maschine), the browser works from DECKARD's own
  index alone; a "Library folders" list in Preferences adds NI pack folders the user
  owns (e.g. `/Users/Shared/… Library`), scanned as Factory with their files' own tags
  and the pack's name as Library.

## Tests

- Library queries against a small fixture DB with NI's schema, built in a temp dir
  (never the real one).
- Index scanning over a temp dir of files: tags read from Maschine files; samples
  split into Loop / One-shot by a tempo in the name.
- Save: a sound/group/preset saved to a temp dir reopens in DECKARD with the same
  model and the same tags; byte comparison of the SoundInfo2 against Maschine's for the
  same fields.
- App tests: double-click loads into the focus; drag of a result onto a pad.
- MK3: the BROWSE sweep, and a scripted browse → LOAD test.
- Tests must not write to library folders or the settings file.
