# The browser and saving

Status: **working**, October 2026 (design: `docs/superpowers/specs/2026-10-22-browser-design.md`).

## What it does

- **Software:** the LIBRARY button in the header (or F4) opens the browser on the left.
  - **LIBRARY tab:** sections (Projects, Groups, Sounds, Instruments, Effects, Loops,
    One-shots), Factory / User, ♡ Favorites, the library (product), Types (click down
    the levels, the path above to go back), Characters (all chosen must match), search,
    the results and a prehear row (▶, auto, level).
    - Click a result to choose it (and prehear it, with auto on).
    - Double-click to load it into the focused sound or group, or drag it onto a sound's row.
    - Right-click to add it to or remove it from the Favorites.
  - **FILES tab:** a plain folder browser (folders, then the files DECKARD loads),
    with pinned favourite folders (☆), loading the same way.
- **Loading:**
  - a project opens;
  - a group replaces the focused group (its slot in every scene is emptied);
  - a sound replaces the sound and keeps the pad's notes;
  - an instrument goes first in the chain, an effect last;
  - a loop goes as a dropped file does: an Audio plug-in on an empty sound, a zone on a Sampler;
  - a one-shot goes into a Sampler (one is added to an empty sound).
- **Saving:** the Sound and Group menus have Open…, Save, Save As… and Save with
  Samples…; a plug-in slot's menu has Save Preset… and Load Preset….
  - The Save dialog asks for a name, Types ("Drums › Kick; Bass"), Characters and a
    Library, and asks before replacing a file.
  - Files go to Maschine 3's user folder (`Sounds`, `Groups`, `Plug-ins`) in
    Maschine's own formats. They are listed under User at once.
  - Save writes the same file again.
- **Preferences ▸ Library:** folders of sound packs, listed under Factory (each its
  folder's name, less " Library", as the product), for when NI's library isn't installed.
- **MK3:** BROWSE runs NI's own browse page on the same library:
  - the section (◀ ▶), USER, the products, Type / Sub-type / Character on the knobs;
  - the results: the 4-D encoder turns through them, and pressing it (or LOAD) loads;
  - ★ for the favourites, and prehear.

## Where it comes from

- **NI's index:** `komplete.db3`.
  - Where it is: macOS `~/Library/Application Support/Native Instruments/Maschine 3/`, Windows
    `%LOCALAPPDATA%\Native Instruments\Maschine 3\`.
  - How it's opened: read-only. It's opened immutable when Maschine isn't using it (no
    `-wal` beside it), so SQLite leaves no files in NI's folder. If Maschine starts, it's
    reopened through Maschine's WAL.
  - What it holds:
    - each section's items are in `p_sound_info_<Kind>_<1 user | 2 factory>`;
    - paths and tempos are in `k_sound_info`;
    - products are `k_content_path.alias` (content type 2);
    - Types come from `k_category` (three levels);
    - Characters come from `k_mode`.
- **DECKARD's index:** `library.db3` in DECKARD's settings folder. It holds Maschine's
  user folders, the library folders and what DECKARD saves.
  - Maschine files are tagged from their own tag block.
  - Samples are untagged: a Loop when their name gives a tempo, else a One-shot.
- **The tag block** of every Maschine file (`NISD/108`, `NI::SOUND::SoundInfo2`;
  `crates/mxprj/src/sound_info.rs`) holds:
  - name, author, vendor and comment;
  - the bank chain (Library, Bank, Sub-bank);
  - one attribute list: Types as `\:Drums`, `\:Drums\:Kick` (each level listed) and
    Characters as `\.Punchy`;
  - properties (`\@color`, `\@tempo`, …).

  Read and written back byte for byte.
- **Previews:** Maschine keeps a sound's or group's preview beside it,
  `.previews/<file name>.ogg`.
- **Files:** sound, group, instrument and effect files are laid out as projects, with the
  kind `SND `, `GRP ` or `SLOT` (`crates/doc/src/files.rs`).
  - A sound file has no pattern events.
  - A group file has no `patterns` or `sequence` (its patterns are in `patterns_by_id`), and
    its routing is skipped.
  - Saved with samples: copied into `<name> Samples` beside it, referenced relative to the
    user folder (`FileLocation` kind 1, "Standard User Directory").
- **Loading files from other Maschine versions:** objects whose class is at another
  version than the project's are rebuilt at the project's, their fields copied by name.
  - Old groups' legacy mixer settings become the `Track` base.
  - Every load is checked: if the project wouldn't save and reopen, it's undone and
    refused. Of 100 factory files sampled, one Maschine 1 sound is refused.

## The MK3's model

`crates/lua-host/src/api/browser.rs` gives NI's scripts:
- `App:getDatabaseFrontend()`: the browser model (file type, user mode, products as
  one group, the bank chain's slot 0, three attributes, the result list) and controller;
- `App:getBrowserControllerDeferredSearch()`, whose changes become `BrowseCall`s for the app;
- `NI.DB3` constants (DECKARD's own values; Maschine's aren't recovered).

The app hands over a `BrowserView` each frame; on a change the scripts get
`onDB3ModelChanged`. NI's custom widgets are given the style sheet's structure
(`ResultListItemWidget` with `Label#m_pTextLabel`; `ProductIconRow` of three `Bar.ProductBar`).

## Not yet

- NI's per-product artwork on the MK3's product tiles (the style sheet's default
  picture shows).
- Banks under products; product groups (by category / vendor).
- Tag editing beyond the Save dialog; Quick Browse.
- The cloud sample service (a separate project).
