# Plug-ins in sounds: AU ids, chunks, parameter pages, preferences, module menu

Reverse engineered from Maschine 3 (arm64 slice of `Maschine 3.app`), decompiled with
`tools/ghidra/decompile.sh` into `out/decomp/ph1` … `ph6`, plus strings, the NWL
resources (`out/ni-res/maschine_nwl_resources/Maschine/Dialogs/Preferences.txt`), the
user's own `Plugin_arm64.data` and preferences plist, and saved projects. Addresses are
arm64 VM addresses.

**[exact]** means read from code or data. **[inferred]** means a reasonable reading of
partial evidence.

---

## 1. How a plug-in is identified in a project (`PluginHost.plugin_id`)

### 1.1 The id string

`NI::HOSTING::UniquePluginID::UniquePluginID(PluginUID, vendor, type, technology)`
(0x102587df0) formats it with `"%s:%s:%s:%s"` (0x10d0524cc):

```
<vendor>:<technology>:<type>:<uid>
```

**[exact]**

`technologyString()` and `pluginUidString()` split the id on `:` and require exactly
four parts, taking part 1 and part 3. **[exact]**

**Technology strings.** These are static `std::string`s at 0x10dc43f30, 0x10dc43f48 and
0x10dc43f60. **[exact]**

| technology | string |
|---|---|
| Audio Unit | `AU` |
| VST 2 | `VST` |
| VST 3 | `VST3` |

**Type strings.** Static strings at 0x10dc43ee8, 0x10dc43f18 and 0x10dc43f00.
**[exact]**

| type | string |
|---|---|
| instrument | `INSTR` |
| effect | `FX` |
| MIDI effect | `MF` |

How the type is chosen (`HOSTING::init()` lambda, 0x10255efc8), from the descriptor's
type bits at `+0x60`. **[exact]**
- If bit 0 (`isPluginInstrument`) is set: `INSTR`.
- Otherwise, if bit 1 (`isPluginEffect`) is set:
  - `MF` if bit 4 (`isPluginMidiProcessing`) is also set and the plug-in is an AU;
  - `FX` otherwise.

**The `<uid>` part.** `HOSTING::pluginID(PluginUID)` dispatches on the UID's `tag()`
(AU 0, VST2 1, VST3 2) to a formatter registered in `HOSTING::init()`. **[exact]**
- **AU** (`AUUID`, 4 bytes): `sprintf("%i", value)`, a signed decimal. The value is
  `AUPluginDescriptor+0x70`.
- **VST 2** (`VST2UID`): `"%i"` of the VST2 unique id, as in the projects seen so far,
  e.g. `Native Instruments:VST:INSTR:1315523937`.
- **VST 3** (`VST3UID`, 16 bytes): `VST3::UID::toString(false)`, which is 32 upper-case
  hex characters (`…:VST3:INSTR:5653544E694D616D6173736976650000`).

**What the AU value is.** The `AUPluginDescriptor(CAComponentDescription)` constructor
(0x10257d300) copies the 20-byte description to `+0x6c`:

| offset | field |
|---|---|
| `+0x6c` | `componentType` |
| `+0x70` | `componentSubType` |
| `+0x74` | `componentManufacturer` |
| `+0x78` | `flags` |
| `+0x7c` | `mask` |

**[exact]**

So the AU uid is the decimal of the component's **subtype** OSType, read as a 32-bit
integer: the four-character code big-endian, so `'Ni$D'` → `0x4E692444`, printed with
`%i`. **[exact]** The manufacturer and type are not part of the uid. They are identified
by `<vendor>` (the manufacturer's name) and `<type>`; on loading, the uid is matched
against the scanned descriptors. **[inferred]**

**Vendor.** `PluginDescriptor` holds a vendor name and a plug-in name. For AUs both come
from the component name `"Vendor: Plugin"`
(`AUPlugin::getPluginAndVendorNameFromResource`). **[inferred]**

**`plugin_name`** is the descriptor's plug-in name (`Massive`, `Reaktor 6`). The
descriptor suffix is used only in menus: `" (AU)"` (`AUPluginDescriptor::suffix`) or
`" (VST3)"`. **[exact]**

**Is there a descriptor object in the project?** No. A `PluginHost` stores only
`plugin_id`, `plugin_name` and `chunk`. The `NI::HOSTING::*PluginDescriptor` serializers
(`AUPluginDescriptor`, `VST2PluginDescriptor`, `VST3PluginDescriptor`) are used for the
**plug-in cache file** (§3.3), not for projects. **[exact]**

Base `PluginDescriptor` serialization (0x10002c0d8), in order:
- file path; `long` at `+0x20`;
- two strings: name and vendor;
- four `int`s: `+0x68`, the type bits at `+0x60`, `+0x5c`, `+0x58`;
- two `bool`s: `+0x66` and `+0x64`.

**[exact]** for the order. The meaning of each field is **[inferred]**.

### 1.2 The chunk (`PluginHost.chunk.size_data`)

`HOSTING::PluginChunk::write` (0x10258b0a8) and `read` (0x10258b0fc) give the layout.
All three header fields are little-endian 32-bit values. **[exact]**

```
s32  1            // version; written as 1, ignored on read
s32  chunkType    // enum eChunkType, 0..7
s32  size
u8   data[size]
```

Chunk types per technology:

| technology | chunkType | data | source |
|---|---|---|---|
| VST 3 | 1 | Our `.vstpreset` (Comp/Cont), as already handled | `VST3Serializer::getPluginChunk`, 0x102578430 **[exact]** |
| VST 2 | 1 | Raw `effGetChunk` data | **[exact]** from projects |
| AU, not NI | 3 | The AU's `kAudioUnitProperty_ClassInfo` dictionary (`CAAudioUnit::GetAUPreset`) | `AUSerializer::getPluginChunk`, 0x102582e70 **[exact]** |
| AU, NI (`AUPlugin` flag `+0x38`) | 1 | Only the raw bytes of the ClassInfo key `"vstdata"`: NI's own state blob, the same format as their VST data | `AUSerializer::getPluginChunk` **[exact]** |

- **Type 3** is serialized with `CFPropertyListCreateData(…, format 100, …)`, where 100
  is `kCFPropertyListXMLFormat_v1_0`. So the data is an **XML plist**. On load,
  `CFPropertyListCreateWithData` is followed by `SetAUPreset`. **[exact]**
- **Loading a type 1 or 4 chunk into an AU** (`AUSerializer::setPluginChunk`,
  0x102582fc4) builds a ClassInfo dictionary and calls `SetAUPreset`:
  - `version` = 0;
  - `type`, `subtype`, `manufacturer` = the component description, as `SInt32`
    CFNumbers;
  - `vstdata` = the data.

  The keys are CFStrings at 0x10d59a500–580. After the preset is set, Maschine calls
  `AUParameterListenerNotify(…, kAUParameterListener_AnyParameter)`. **[exact]**
- **Types 6 and 7** are special: an NI interprocess or "round-trip" state passed through
  a private property (AU property ids 8 and 9 through `get/setNIProperty`; VST3
  likewise). Ignore them, and write 1 or 3. **[exact]** that they are special;
  **[inferred]** that it is safe to ignore them.

### 1.3 Hosting AUs

- **Instantiation is AU v2** (`CAAudioUnit::Open` → `AudioComponentInstanceNew`). There
  is no `AudioComponentInstantiate`. **[exact]**
- **The calls used** are `AudioUnitInitialize`, `AudioUnitRender`,
  `MusicDeviceMIDIEvent`, `AudioUnitGet/SetProperty`, `AudioUnitSetParameter` and
  `AudioUnitScheduleParameters`. **[exact]**
- **Bus layout:** `"Cannot set valid bus layout for audio unit {}"` shows Maschine
  negotiates the stream format and channels. **[exact]** for the string.
- **The editor** comes from `AUPlugin::getEditor`: the Cocoa view through
  `kAudioUnitProperty_CocoaUI`. **[inferred]**
- **Discovery:**
  - `HOSTING::discoverAUPlugins` uses `AudioComponentFindNext`.
  - Each candidate is then validated in a separate scanner process
    (`ScanPluginsParallel::spawnAuScans`, `spawnProcessMacOS`; log
    `"Scan AU Plugin ({}/{}): {}"`).

  **[exact]**
- **The local cache** (`Plugin_arm64.data`) lists AUs with their raw description, e.g.
  `aufx` / `+012` / `SHDK` (SoundHack) stored little-endian. That matches 4-byte OSTypes.
  **[exact]**

---

## 2. Plug-in parameter pages

**Objects in the project** (`NI::MASCHINE::DATA::PluginParameters` →
`ObjectVector<PluginPage>` → `boost::array<shared_ptr<PluginParameter>, 8>`).
**[exact]**
- **The pages are saved in the project** with the `PluginHost`.
- **Each page** has a `name` (a `WStringParameter`) and up to **8** parameters.
- **Each `PluginParameter`** has:
  - its normalized value: the cached last value, a `MaschineNumericParameter<float>`;
  - `index`;
  - for version 1, `unknown_2b8`, a string and **[inferred]** the user's custom name
    (`setCustomName`), and `unknown_flag`, **[inferred]** "use autonaming"
    (`setUseAutonaming`);
  - `section_name`.

**`index`** is the plug-in's **parameter id**. **[exact]** for how it is used
(`PluginParameter::setParameterId(Transaction&, unsigned)` and `parameterId()` at
`+0x2b4`); **[inferred]** for the id kind. It is what the host's
`PluginParameters::getParameterID(index)` returns (vtable `+0x58`). Per format:
- **VST 3:** the `ParamID`;
- **AU:** the `AudioUnitParameterID`
  (`AUParameters::getParameterIDbyIndex` / `getParameterIndexByID`);
- **VST 2:** the parameter index.

In saved projects with Massive (VST2 and VST3) the ids run 0, 1, 2, … in order. That's
consistent with this, since Massive's ids equal its indices.

**Automapping** (`PluginParameterManagerBase::applyAutomapping`, 0x1017d0a58).
**[exact]**
1. Walk the plug-in's parameters `i = 0..count-1` in order, get `id = getParameterID(i)`,
   and keep those that pass `isUsedAndAutomatable(id)`. Stop after **128** parameters
   (0x80).
2. Make `ceil(n / 8)` pages, at most 16, adding pages as needed; slots fill page by page
   in order.
3. For each kept parameter:
   - `setParameterId(id)`, then `setupFromPluginValues()`, which copies the current value
     and name;
   - if this is not a forced re-map and the slot's previous id differs, clear its custom
     name and set "KKS always show value" to false.
4. With no parameters at all, the pages are removed.
5. In a forced re-map (`onForceAutomap`, or when automapping a fresh plug-in), every page
   is first cleared (`setupPages(0)`).

**`isUsedAndAutomatable(id)`** (0x1017dc3e4) gets the parameter's info and returns true
when:
- the info call fails (a parameter whose info can't be read is kept); or
- it is automatable, is not hidden or read-only, and its kind is not `0x1000`.

**[exact]** for the control flow. **[inferred]** for the meaning of the flags: in VST3
terms, keep `kCanAutomate` and drop `kIsReadOnly`, `kIsHidden` and a special kind
(likely program change or bypass).

**Page names.** For automapped plug-ins every page's `name` is empty in the saved
projects. **[exact]** The UI then shows a generic name, **[unknown]** exactly which;
show "Page N". Plug-ins with a side-chain input get one extra page after the plug-in's
pages, named **`Side-Chain Input`** (`getPageName`; wide string 0x104cf9b74;
`getNumPages` adds `hasSideChainPage()`). **[exact]**

**NI and NKS plug-ins.**
- **Where their pages come from.** `PluginHostBase` has `sendControllerAssignmentsNKS`,
  `onNksAssignmentsChanged`, `applyNISoundMapping` / `storeNISoundMapping`,
  `parseNksMetadata`, `nksNavigationData` and AU `getNIProperty`. **[exact]** for the
  names. NI and NKS plug-ins send their controller assignments (pages, names, sections)
  to the host through these. Maschine then fills the pages from those assignments
  instead of automapping. **[inferred]**
- **Saving:** they are saved as ordinary `PluginPage`s, so they appear on reload without
  NKS.
- **What we can do:** for plug-ins we can't query, automapping alone gives a working
  fallback.

---

## 3. Preferences › Plug-ins

### 3.1 UI

The page is `NI::MASCOMMON::GUI::DialogPreferencesPlugins`; its constructor is at
0x101b4eff0, `out/decomp/ph6/0008.c`. It has a `MultiTab` with two tabs. **[exact]**

**Manager** (`pManagerBar`): a table of scanned plug-ins.
- **Columns:** a checkbox (enable/disable each plug-in, `onPluginToggle`), the plug-in
  name (header `Preferences.Plugin` → "Plugin"), and `Preferences.DefaultConfig` →
  "Default Config" (a default preset per plug-in; reset with
  `onPluginResetDefaultConfig`; "Select a default preset for").
- **Footer:**
  - **Use VST2 Plugins** (`m_useVST2`)
  - **Use VST3 Plugins** (`m_useVST3`)
  - **Use Audio Units** (`m_useAudioUnits`)
  - **Always Use Latest Version of NI Plugins** (`m_pUseMaxVersionEngine`)
  - a scan-on-startup checkbox (`m_scanOnStartup`)
  - **Rescan** (`rescanPlugins`)

**Location** (`pPathBar`): a list of plug-in folders, with **Add** ("Add Plugin
Directory"), **Remove**, an editable path (`onPathEdit`, "Locate Directory") and
**Rescan**.

**Strings** (wide, about 0x4d4d680–0x4d4dc28): "Plugin", "Default Config", "Always Use
Latest Version of NI Plugins", "Use VST3 Plugins", "Use VST2 Plugins", "Use Audio Units",
"Rescan", "Add", "Remove", "Add Plugin Directory", "Locate Directory". **[exact]**

### 3.2 Where the settings live

**Preferences plist:** `~/Library/Preferences/com.native-instruments.Maschine 3.plist`
(`NI::MASCOMMON::Registry`, key class 1). **[exact]**

| key | type | default | source |
|---|---|---|---|
| `UseAudioUnits` | bool | **true** | `PreferencesPlugins::useAudioUnits` 0x1006502f4 **[exact]** |
| `UseVST2Plugins` | bool | **true** | `PreferencesPlugins::useVST2Plugins` 0x10065019c **[exact]** |
| `UseVST3Plugins` | bool | true | present = 1 in the user's plist **[exact]** |
| `UseMaxVersionEngine` | bool | | "Always Use Latest Version of NI Plugins" **[exact]** for the key string |

**Plug-in cache and folders:**
`~/Library/Application Support/Native Instruments/Maschine 3/Plugin_arm64.data`.
- **Building the path.** It is `ni::filesystem::user_data_ni_dir()` / `Maschine 3` /
  `"{}{}.data"`, with `Plugin` and the architecture suffix `_arm64`
  (`PluginManager::getSettingsFileName`, 0x10000fdf0). **[exact]**
- **Format.** It is a boost archive (`serialization::archive`) of `PluginManager`, which
  holds:
  - a `PluginPathCollection` of folders;
  - the `multi_index_container<shared_ptr<PluginDescriptor>>` of scanned plug-ins,
    indexed by file name and by plug-in name (case-insensitive);
  - each plug-in's enabled flag and default config.

  **[exact]** for the structure; per-field details **[inferred]**.
- **When the file is missing** (`PluginManager::PluginManager`, 0x10000ee8c), the
  folders are seeded with `addVST3StandardDirectories`. On this machine the file holds
  `/Library/Audio/Plug-Ins/VST3` and `~/Library/Audio/Plug-Ins/VST3`. **[exact]**
- **No VST2 folders.** `PluginPathCollection::getVST2PluginCandidates()` (0x100011544)
  returns an empty list in this build. Maschine 3 on arm64 doesn't scan VST2 bundles at
  all: the user-visible folders are VST3 folders, and VST2 ids in old projects are
  resolved through VST2→VST3 migration ("…may not yet support Apple Silicon or VST2 to
  VST3 migration"). **[exact]** for the empty function; **[inferred]** for the rest.
- **AUs need no folder.** They are listed by the Component Manager. Other standard
  folders appear in strings (`/Library/Audio/Plug-Ins/VST/`, `…/Components/`), but
  belong to product administration, not to the scanner.

---

## 4. Adding a module to a sound's slot (the slot menu)

`SlotListHelper::addInternalPluginsMenu` (0x101dc7544) and
`SlotListHelper::addPluginsSubMenu`. **[exact]**

- **"Internal" submenu.** Groups come from `ModuleFactory::getInfoGroups()`
  (`ModuleFactory()` at 0x1002f4624, `out/decomp/ph5/0004.c`), in this order; separators go between
  groups, and group 1 is a "Drum Synth" submenu:

  | # | group | DATA classes (`NI::MASCHINE::DATA::` unless noted) | names shown |
  |---|---|---|---|
  | 0 | | `MASCOMMON::DATA::AudioModule`, `MASCOMMON::DATA::Sampler` | Audio, Sampler |
  | 1 | **Drum Synth ▸** | `Kick`, `Snare`, `Hihat`, `Tom`, `Perc`, `Cymbal` | Kick, Snare, Hi-hat, Tom, Percussion, Cymbal |
  | 2 | | `BassSynth`, `PolySynth` | Bass Synth, Poly Synth |
  | 3 | | `Compressor`, `Gate`, `TransientDesigner`, `Limiter`, `Maximizer` | Compressor, Gate, Transient Master, Limiter, Maximizer |
  | 4 | | `EQ`, `Filter`, `CabinetEmulation` | EQ, Filter, Cabinet |
  | 5 | | `Chorus`, `Flanger`, `FM`, `FreqShifter`, `Phaser` | |
  | 6 | | `Iceverb`, `Metaverb`, `Reflex`, `Reverb`, `PlateReverb` | |
  | 7 | | `BeatDelay`, `GrainDelay`, `GrainStretch`, `Resochord` | |
  | 8 | | `MullhollandDrive`, `LoFi`, `Saturator` | Drive, Lo-Fi, Saturator |
  | 9 | | `PerformanceFX` | Perform FX (only where `StateHelper::canAddPerformanceFX`) |
  | 10 | "Developer ▸" | | developer builds only |

  **[exact]** for the groups and classes. The display names are matched from the
  binary's wide strings, so they are **[inferred]** per entry.
- **Which modules a slot offers.**
  - The menu is built for a slot of a group or sound chain. Each `ModuleInfo` has an id
    (`+0`), a wide name (`+8`) and a flag at `+0x20`.
  - When the slot is an effect slot (`param_5` set), only infos with the `+0x20` flag
    are shown; so instruments are offered in the first (instrument) slot and effects in
    any slot. **[inferred]** for the flag's meaning.
  - Module id `0x2b` (43) is hidden unless the chain belongs to a Group. **[exact]**
- **Plug-ins submenus** (`addPluginsSubMenu`). Each format is a submenu, in this order:
  **"Audio Units"**, **"VST3"**, **"VST"**. Each is shown when there are plug-ins of that
  format; the "Use …" preferences filter them. **[exact]** for the labels and order.
  - Each format menu (`addPluginFormatMenu`) is filled from
    `PluginHelper::getValidPluginDescriptorsOrderedByVendorAndName(technology, typeMask)`,
    with one submenu per **vendor** and the plug-in names inside it.
  - The type mask makes the instrument slot list instruments and other slots list
    effects (and MIDI effects for AU). **[exact]** for vendor grouping and ordering;
    **[inferred]** for the mask per slot.
- **Other menu entries:** "None" (`addNoneMenu`, which clears the slot), Load/Save
  preset, Cut/Copy/Paste and Controller Pages.

### Parameter pages of internal modules

Each internal module implements `getPagesNames()` and `getParameter(page, slot)`. For the
Sampler (`Sampler::s_aPageNames`) the pages are:
1. "Voice Settings / Engine"
2. "Pitch / Envelope"
3. "FX / Filter"
4. "Modulation"
5. "LFO"
6. "Velocity / Modwheel"

The parameter in each slot is given by `Sampler::getParameter`
(`out/decomp/sampler/0007.c`).
