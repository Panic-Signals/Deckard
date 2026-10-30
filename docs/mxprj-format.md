# `.mxprj` format notes

Status: every local document decodes completely, with no bytes left over: 119 projects,
1,406 groups, 1,872 sounds and 286 instrument/effect presets (Maschine 2 and 3, boost
library versions 10–20). Implementation: `crates/mxprj` (loaders per type in
`src/types/`, one file per family). Many fields are still named by struct offset
(`f2e8`, `unknown_*`) until their meaning is established.

## 1. Container: NI `hsin` items

All integers little-endian.

```text
item:
  u64   size                 whole item, including this field
  u32   version              1
  [4]   "hsin"
  u32, u32                   1, 0 in every file seen
  [16]  uuid
  frame                      outermost frame (see below)
  u32   child-list version
  u32   n
  n ×   { u32 tag, u32 domain (fourcc, stored LE), u32 item_id, item }

frame:
  u64   size                 whole frame
  [4]   "DSIN"
  u32   id                   type id; 1 = terminator
  u32   version
  frame                      base-type frame (absent when id == 1)
  ...   own payload          rest of the frame
```

So each item carries a frame chain from its most-derived type down to id 1, and a list
of child items.

Typical project tree:

```text
root            frames 118 > 106 > 1
  MAS2/PROJ     frames 101 > 106 > 1        payload holds an app version string ("2.3.0.3441")
    NISD/108    library/author metadata (UTF-16 strings)
    NISD/121
    NISD/116    frames 116 > 115 > 1
      NISD/109  frames 109 > 1              ← the document
```

Frame 109's payload: `u32 version, u32, u32, u64 blob_size, u32 len, archive[len]`, with
`blob_size == len + 4`. The `u32 len` + bytes pair is how the app's stream `readString`
stores a string.

## 2. Archive: boost `portable_binary_iarchive`

This is the portable archive from boost's serialization examples, not the stock binary
archive.

| Value | Encoding |
|---|---|
| integers, class ids, versions, tracking flags, object ids | 1 signed byte `n`; `0` → 0; else `\|n\|` LE magnitude bytes, negated if `n < 0` |
| `bool`, `char` | 1 raw byte |
| `float`, `double` | raw LE IEEE |
| `std::string` | integer length + bytes. `std::wstring` fields are stored this way too (narrow, converted on load) |

Header: string `serialization::archive`, integer library version, 1 raw flags byte.

Object framing follows boost `basic_iarchive`:

- **First use of a class:** `tracking` then `version` (integers). The optional class id is
  not written.
- **Tracked object:** an object id follows the preamble, for plain members and
  `base_object<>` as well as pointers. Ids count up from 0 across all tracked objects. An
  id lower than the next expected one is a back-reference, and no data follows it.
- **Pointer:** a class id comes first (`-1` = null). A class id not seen before is followed
  by the export name (polymorphic classes only), then the preamble, then the object id.
- **Class ids:** assigned in first-use order. Explicit `register_type<T>()` calls consume
  ids too, as does the first load of a class without a preamble.
- **Pointer loads pre-register the declared class** (boost `load_pointer_type`) unless it
  is abstract (`Module`, `MaschineParameter`). A class id that is new after that is a
  derived class and is followed by its export name. Export names are mostly the C++ name,
  with legacy aliases (`NI::MASCHINE::DATA::Sampler` for `NI::MASCOMMON::DATA::Sampler`,
  `FloatModulationEvent` for `ModulationEvent<MaschineNumericParameter<float>>`, …).
- **Back-references are common:** NI saves from temporaries, so even plain members often
  carry an earlier object id instead of data.
- **Implementation level:** `std::shared_ptr`, `scoped_ptr`, `std::pair` and NI classes have
  preambles; `std::vector` of a primitive is `object_serializable` (no preamble).

Root: a single non-pointer `NI::MASCHINE::DATA::Project`.

## 3. Document kinds

The `MAS2` child's item id names the root object:

| Item id | Extension | Root type |
|---|---|---|
| `PROJ` | `.mxprj` | `NI::MASCHINE::DATA::Project` |
| `GRP ` | `.mxgrp` | `NI::MASCHINE::DATA::Group` |
| `SND ` | `.mxsnd` | `NI::MASCHINE::DATA::Sound` |
| `SLOT` | `.mxinst`, `.mxfx` | `NI::MASCOMMON::DATA::Slot` |

## 4. Notes and patterns

A `Sequence` holds a boost `multi_index` container of `TimedEvent`s: a position in ticks
(960 per quarter note) and a polymorphic `Event*`. A `NoteEvent` is `Event` (flag) →
`LengthEvent` (length) → from version 1 a probability policy → note → velocity.
`EventPattern` has a name, length, color, its own `Sequence`, and a map
`Sound* → SequenceReference` pointing at each sound's note `Sequence`.
`mxprj-dump <file> --notes` lists them.

## 5. Field order

`serialize-schema` (in `tools/re-tools`) writes `out/schema/serialize.txt`. It lists the
reads of each type's `serialize`/`load` for this archive in code order: integer widths,
string loads, nested object loads and class-version tests. Caveats:

- Branches on *loaded values* aren't shown. Example: `ContentInfo` reads its strings only
  when its leading flag is set.
- Integer reads inlined into the caller aren't shown.

Each class's maximum supported version comes from its
`checkSerializationVersionAndThrowIfNewer<..., T>` guard (`cmp w0, #max+1`).

### Example: the start of `Project`

```text
Project (v ≤ 6)
  scoped_ptr<ContentInfo>          object, then pointer to ContentInfo
    ContentInfo::load              int8 has_content; if set: string library,
                                   string product, string snpid, int32 checksum
  WStringParameter  name           → ValueParameter<wstring> → Parameter (string id if v ≥ 1),
                                     then string value; then MaschineParameter (empty)
  int8   unknown flag
  int64  unknown
  v ≥ 1: int32, int32              unknown pair
  v ≥ 2: ObjectVector<Song>, RemoteParameter maps, WorkspaceConfig, MidiMapper, …
```

Corpus check: in 107 of 117 projects the decoded name equals the file name. The rest were
renamed after saving or have an empty name.

## 6. Values the decoder relies on that are not in the file

- `MidiMacro` holds exactly 130 `MaschineNumericParameter<int>` (128 + 2, from its
  constructor).
- `Song` and `Group` read one `QuickBrowseHandle` per group / sound, and `Chain` one per
  slot, using the count of the vector decoded just before.
- `MaschineRoutingParameter` version < 2 skips its value when the containing `Track` /
  `Group` sets a flag that depends on whether a whole project is being loaded.

## 7. Samples and mixer values

Field names come from the wide-string labels each constructor passes when it creates its
parameters (`Zone::Zone`, `Sampler::Sampler`, `AHDSREnvelopeObject::AHDSREnvelopeObject`,
`Sound::Sound`, `Track::Track`); serialization order matches constructor order.

- **Sample location** (`FileLocation`): `kind`, `path`, and for `kind != 0` an `AliasPath`
  (root on the saving machine, library product name). Every zone in the local corpus
  (10,182) has `kind` 2: `path` is relative to the library, sometimes with leading `//`.
  The installed location of a library is `ContentDir` (an HFS path such as
  `Macintosh HD:Users:Shared:Maschine 2 Library:`) in
  `/Library/Preferences/com.native-instruments.<library>.plist`.
- **Zone**: play range and loop range in sample frames, `loop_active`, x-fade, attack,
  decay, `root_key` (default 60), key range, velocity range, `gain` (default 0.75),
  `pan` (−1..1), `tune` (semitones, ±36).
- **Levels** are linear amplitude: `LevelScale::convertLevelTodB` is `20·log10(level)`.
- **Sampler**: editor tab, polyphony, glide, pitchbend, mode, `envelope_type` (corpus:
  0 ×2032, 1 ×621, 2 ×809; read as one-shot / AHD / ADSR), reverse, tune, start, comp,
  drive, sample rate, bits, filter type, cutoff, resonance, EQ frequency/width/gain,
  then modulation amounts (pitch envelope start/decay, velocity→cutoff,
  velocity→volume, start, cutoff, LFO depth, pan), the volume and modulation
  `AHDSREnvelopeObject`s (attack, hold, decay, sustain, release, then pitch, cutoff,
  drive, pan amounts; times stored normalized 0..1) and an LFO.
- **Sound / legacy group channel strip** (`track_parameters`): audio output, cue enabled,
  `level`, `pan`, `mute`, then (sounds only) audio mute, followed by the two aux sends.
  The sound's audio-input block carries an input gain; `base_key` defaults to 60.

## 8. Writing

`decode::encode` runs the same loaders in write mode. Each `int`/`string`/`object`/
`pointer`/`collection` call takes the tree's next field (its name must match) and writes
it. Session state is rebuilt the way boost's `basic_oarchive` builds it:

- **Class ids** are assigned in first-use order. Explicit registrations and declared
  pointer classes count too, unless the class is abstract.
- **Preambles:** `tracking` and `version` are written once per class. They come from the
  first object of that class: an object id means tracked, and the version is
  `Obj::version`.
- **Object ids** count up from 0 in write order. Ids from the tree ("old" ids) are mapped
  to the new ones, so `Value::Ref` back-references stay correct when objects are added or
  removed. New objects need old ids that no other object uses.
- **Pointer export names:** a class first met through a pointer gets its export name
  written. That is `Obj::export` as read, which can be a legacy alias, or else a name that
  maps to the class.
- **Collections** write their length from the list. An item version other than 0 is kept
  in a `<name>#item_version` field just before the list.
- **Integers** use the fewest magnitude bytes. NI's files do the same: no file in the corpus
  needed anything else.

The file around the archive is rewritten from the parsed item tree (`Item::to_bytes`), with
item and frame sizes recomputed. Frame 109's payload is updated with the new archive
length; its prefix and any trailing bytes are kept.

Status: every local document round-trips byte for byte (`mxprj-roundtrip`,
`tests/corpus.rs`). An edited project (tempo and name changed, so the archive length
differs) opens in Maschine 3, which loads all of its plug-ins.

Explicit counts inside the archive, such as the event count of a `Sequence`'s
`multi_index` container, are fields of the tree. Edits that add or remove items must
update them.

## 9. Scenes and the arrangement, as Maschine 3 edits them

This was worked out by comparing projects that Maschine 3 saved before and after a single
edit each. The tool is `mxprj-diff`. It matches object ids up as it walks the two trees and
aligns list items by content, so an insert shows as one change. Files saved by Maschine 3
should be compared with each other: re-saving an older project upgrades the whole format
(new class versions and fields), which buries a single edit under thousands of changes.

- **`Scene`:**
  - `name` (`WStringParameter`).
  - `color`: `ColorParameter` with `color` and `use_default_color`. A new scene uses the
    default colour.
  - `patterns`: an `ObjectMap<Group*, PatternRef>` with an entry for every group. A
    `PatternRef`'s `pattern` points to an `EventPattern`, or is null for an empty slot.
    The map is written in pointer order, so entry order changes between saves and means
    nothing. The map's `current` points at its last entry.
  - `scene_tempo_active` and `tempo`.
- **The scene list** (`ObjectVector<Scene>`) has a `current` pointer (the focused scene)
  and a `selection` set.
- **Sections** are an `ObjectMap<unsigned long, Section>`. The key sets the order:
  - Inserting a section renumbers all keys 0..n.
  - Deleting one leaves a gap (0, 2, 3, …).
  - A `Section` holds `scene` (a pointer), `length` in ticks and `auto_length`. While
    `auto_length` is on, the section lasts as long as its scene and `length` is stale.
  - Setting a length turns `auto_length` off.
- **`MaschineNumericParameter`** keeps `t0` equal to the value and `t1` as the value before
  the last change. Edits update both.
- **Duplicating a scene** inserts the copy right after the original, named
  `<name> (<n>)` (format string `%s (%d)` in the binary).
- **Duplicating a section** also duplicates its scene: the copy is appended to the end of
  the scene list, and the new section goes right after the original.
- **Deleting a scene** also deletes the sections that play it.
- **Saved UI state** that changes along the way is not part of the edit: the song's loop
  range, zoom levels, `focus_entity`, and the root's playhead (`unknown_i64`).

`deckard-doc` implements these edits in `src/arrange.rs`:
- **Matches Maschine:** `tests/arrange.rs` replays each edit and compares the result with
  Maschine's own files: scenes, pattern choices, section scenes and lengths, and section
  keys.
- **Copies:** copied objects get fresh object ids, and references inside the copy follow
  it.
- **Safety:** an edit that would leave a reference to a removed object is refused.
- **Loads in Maschine:** edited files open in Maschine 3, which loads all their plug-ins.

## 10. Notes, as edited

- **Where notes live:** a pattern's `sound_sequences` map has an entry for every sound of
  its group. The entry's `SequenceReference.sequence` is null when the sound has no notes
  in that pattern. Sequences themselves are defined in each sound's own `sequences` list,
  and patterns refer back to them.
- **A sequence's `events`** is a boost `multi_index_container` with one ordered,
  non-unique index (by position):
  - `count`, then the value version;
  - per event, a `TimedEvent` (`position` in ticks, plus a pointer to a `NoteEvent`)
    followed by a node marker;
  - a header marker, then the index's reorder list.
- **The reorder list:** events are saved in index order, so the list is always just null.
  That held for all 9,380 sequences in 40 projects.
- **Node markers** are empty tracked objects. Maschine's files often make them
  back-references to unrelated earlier objects (boost tracks by address, and addresses get
  reused). New events get fresh markers. When an edit removes an object that is still
  referenced later, its definition moves to that next reference
  (`Document::apply_checked`).
- **`NoteEvent`:** a `LengthEvent` base (an `Event` with a flag, plus `length`), then
  `note` and `velocity`. From version 1 there is also a `ProbabilityEventPolicy` base:
  version 0 held a mute flag, and version 1 holds `probability` as a byte, 0–100.
- **One class version per archive:** writing an older project's notes with a probability
  means upgrading every `NoteEvent` (and every policy) to version 1, as Maschine 3 does
  when it re-saves. `Document::upgrade_note_format` does this, including the undo
  history. The writer refuses an archive that mixes versions of one class.

`deckard-doc`'s `edit_notes` removes and adds notes as one undo step:
- **Matches Maschine:** it reproduces Maschine's own added notes (reference file
  `test-1`).
- **New sequences:** the first note of a sound in a pattern creates the sequence.
- **Maschine accepts the output:** files with added, changed and deleted notes, a new
  sequence, and upgraded probabilities all open in Maschine 3.

A sequence can hold other events besides notes: `NI::MASCHINE::DATA::MidiEvent`
(`i14` the kind: 4 = control change, with `i18` the controller number and `i20` the
value 0..127; 3 = pitch bend, `i18` = −1 and `i20` 0..16383 with 8192 the centre). It
has an `Event` base (`flag_11`) and no length. A `TimedEvent` holds it like a note and
a node marker follows it. A project with none can be given one built from these fields:
the class is then registered at its first pointer like any other. Across the local
projects: controllers 1 (mod wheel), 64 (sustain pedal), 98/99 (NRPN) and 123, and pitch
bend. They are recorded along with the notes and played back with them. Deleting every note leaves them, so a
new note must copy its format from a `NoteEvent`, never from the first event left.

### Lanes under the notes

Each sound's (`MixingLayer`'s) `midi_automators` is the list of lanes the pattern
editor offers under its notes, in order, and its `current` is the lane shown. A
`MidiAutomator` has `mode`, an `EnumParameter` holding a MIDI event type, and `controller`
(an unsigned long). Both parameters' `t0` mirror their value. Across the local projects:

| mode | lane | controller |
|---|---|---|
| 2 | Velocity (every sound has it, first) | 0 |
| 8 | Probability (added when Maschine 3 re-saves) | 0 |
| 4 | Control change | the CC number |
| 3 | Pitch bend | −1 or 0 |
| 0 | unknown (5 sounds) | 0 |

A sound with recorded controller events has a lane for them
(`MidiAccess::focusOrCreateMidiAutomator`). Removing a lane leaves `current` pointing
at the first remaining one.

## 11. Time signature, pattern length and swing

- **Time signature:** the song's `numerator` and `denominator`, both
  `MaschineNumericParameter<unsigned long>`. The denominator is stored as a power of two
  (2 = quarter notes). A bar is `960 · 4 · numerator / denominator` ticks.
- **Pattern length:** `EventPattern.length`, in ticks.
- **Groove:** every `MixingLayer` (each sound, each group, and the song for the master)
  starts with these parameters, named from `MixingLayer::MixingLayer`'s labels in
  creation order (= serialization order):
  - `macro_page`, `macro_focus`, `name`;
  - then the groove: `swing_amount` (float, 0–100 %), `swing_cycle` (an enum index),
    `swing_invert` and `swing_bounce` (bools).

  `label-refs` (in `tools/re-tools`) recovers such label lists. It also finds the code that
  reads a given address.
- **How Maschine applies swing** (`NI::MASCHINE::Swing::calcToSwingPos`, decompiled):
  - The position in quarter notes is multiplied by 24 and split into cycles of
    `s_aSwingLengths[cycle]` = 12, 18, 24, 36, 48, 56, 96, 192 units. That is 1/8, 3/16,
    1/4, 3/8, 1/2, ~9/16, 1 and 2 whole notes; index 0, a 1/8 cycle that swings 16ths, is
    the default.
  - With `a` = amount/100, negated when inverted, each cycle's midpoint moves to
    `0.5 + 0.45 · s_aSwingComp[cycle] · (|a| + a²)/2`. `s_aSwingComp` is
    1, .5, .5, .25, .25, .16, .125, .0625.
  - The first half of the cycle stretches linearly to reach the new midpoint, and the
    second half compresses to fit.
  - The manual says group swing adds to sound swing, and master swing to both. The engine
    applies sound, then group, then master.
- **The cycle labels** are generated at runtime (`1/%i`), so the names we show come from
  the table lengths.

## 12. Hosted plug-ins (`PluginHost`)

A sound's chain slot can hold `NI::MASCHINE::DATA::PluginHost` (version 1). Fields used:

| field | meaning |
|---|---|
| `plugin_id` | `"<vendor>:<format>:<kind>:<id>"`: format `VST3` with the class id as 32 hex digits, or `VST` with the VST2 unique id in decimal; kind `INSTR` or `FX` |
| `plugin_name` | the plug-in's name, e.g. `"Massive"` (VST2 → VST3 class ids include it) |
| `chunk` | `PluginChunk`: `size` and `size_data` (the bytes) |
| `parameters` | `PluginParameters`: 16 pages of 8 `PluginParameter`s (Maschine's knob pages) |

`size_data` is `[u32 1][u32 1][u32 n][n bytes]` (little-endian):
- **VST3:** a `.vstpreset`: `"VST3"`, `i32 1`, the class id as 32 ASCII hex digits,
  `i64` offset of the chunk list; the component state at offset 48, then the controller
  state; then `"List"`, `i32 2`, and per entry an id (`Comp`, `Cont`), `i64` offset and
  `i64` size.
- **VST2:** the plug-in's own state. NI's begins with a `u64` size and an `hsin`
  container, exactly like their VST3 component state, so the VST3 version loads it.

The VST3 version of a VST2 plug-in has class id `"VST"` + the 4-byte id (big-endian) +
the name in lower case, truncated to 9 bytes and padded with zeros: Massive's
`1315523937` (`NiMa`) is `5653544E694D616D6173736976650000` (`VSTNiMamassive`), as
in Maschine's own VST3 ids.

Plug-ins re-save their state with small differences (Massive writes a fresh instance
GUID into its `hsin` header; Reaktor re-packs), so states are only written back after
an edit in the plug-in's window.

## 13. Building from scratch

DECKARD makes new projects and objects without copying any file (`crates/doc/src/build.rs`):

- **Blank objects** (`mxprj::decode::blank`): a class's loader runs with nothing to read.
  Numbers are 0, texts empty, collections empty (a `std::vector<T>` still gets T's item
  version), pointers null, except smart pointers (`shared_ptr`/`scoped_ptr` `px`, not a
  `weak_ptr`'s, not an abstract class), whose object is built blank too. A hook gives
  integers that decide what follows (a sound's routing parameters' `skip_value` = 1).
- **Class versions and tracking** come from the project being edited where it has the
  class, else from `mxprj::types::classes::MASCHINE3` (what Maschine 3.7.1 writes; boost
  library version 20).
- **Defaults** that are not 0 are a table per class (`build.rs` `DEFAULTS`), plus the
  module tables in `crates/model` (Sampler, internal effects).
- **The container** (`mxprj::container::new_project_file`): root 118 > 106, `MAS2/PROJ`
  101 (app version) > 106, `NISD/108` (an empty NISound info), `NISD/121`, `NISD/116`
  (frame 115 embeds an empty item) › `NISD/109` (the archive, then `u32 0, u32
  0x8565620d`). Given Maschine's archive and item ids it reproduces the plug-in's
  default state byte for byte.

`Document::new_project()` equals Maschine 3's default state (one group, 16 empty
sounds, one scene) in every field except `Song.fac8`, which Maschine fills with
uninitialised memory. Maschine loads DECKARD-made projects with every module kind and
plays a Sampler built from scratch on a WAV DECKARD wrote.
