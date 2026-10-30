# Maschine 3 Channel properties: how the software implements them

An implementation spec for DECKARD's Channel properties (Input / Output / Groove / Macro) for Sound,
Group and Master, taken from Maschine 3's own code. It complements:
- `docs/channel-manual.md`: what NI's manual says (signal flow, user-facing behaviour). This file
  does not repeat it;
- `docs/mixer-layout.md`: the mixer strip that shows the same parameters;
- `crates/mxprj/src/types/{group,song,sequence,remote}.rs`: the decoders. §2 gives the real
  meaning of their `unknown_*` / `fXXX` fields.

**Sources**
- Decompiled functions (Ghidra headless, `tools/ghidra/decompile.sh`). Each directory has an
  `INDEX.tsv` (file, address, status, name); the names lists are `out/decomp/chan*-names.txt`.
  Files ending in `.c.w` are copies with UTF-32 character pairs annotated (`/*'Ab'*/`).
  - `out/decomp/chan/`: constructors (`Sound::Sound` 0098, `Group::Group` 0096, `Song::Song` 0095,
    `Track::Track` 0101, `MixingLayer::MixingLayer` 0026, `SequenceVariationModule` 0067/0068,
    `SequenceGenerationModule` 0080/0081, `MacroPage` 0102, `MidiMacro` 0108), the page tables
    (`Sound/Group/Track/Song::getParameter` 0133–0140, `MixingLayer::getNumPages/getPageName`
    0113/0114), routing values (`*ParameterValue::toString` 0124/0130/0132,
    `EventInputParameter::*` 0033–0038, `EventOutputParameter::updatePort` 0051), the Generate and
    Variation algorithms (0063–0066, 0075–0079), the macro helpers (0039–0062), `SwingState` (0011–0025).
  - `out/decomp/chan2/`: RT `SoundEventOutputProcessor` (0001–0013), `HumanizeNoteEventsAction`
    (0033/0034/0081/0082), `calcRandomChord/Velocity/Time/LengthDeviation` (0053–0057),
    `MidiMacroProcessor` (0065), `MidiDeviceManager::addStaticInputDevices` (0054), `Swing` (0042–0045, 0086/0087).
  - `out/decomp/chan3/`: the Variation/Generate context lambdas of
    `SongAlgorithms::injectSequenceContextRetrieverIntoSound` (0002, 0003).
  - `out/decomp/chan-init/`: the static initializers `_GLOBAL__sub_I_DATA_Objects{,_2,_3}_ALL.cpp`
    (0001–0003) that build every enum option list and page-name table.
- Option lists were rebuilt from the static initializers by a small stack-replaying script over the
  Ghidra C (every list in §3 was produced this way; spot-checked by hand).
- Serialization: `out/decomp/schema/0353.c` (Group), `0396.c` (Sound), `0395.c` (Song), `0367.c`
  (MixingLayer). Members are matched by object offset (`param_1 + 0xNN` in `serialize` =
  `param_1[0xNN/8]` in the constructor).
- Style sheets: `out/ni-res/maschine_nwl_resources/Maschine/Panels/ParameterArea.txt` and
  `Panels/LeftPanel.txt`; pictures in `out/ni-res/maschine_pictures_nwl_resources/Maschine/`.
- NI controller Lua: `Shared/Components/SlotStackStudio.lua`, `Shared/Helpers/MaschineHelper.lua`,
  `ControlHelper.lua`, `MacroHelper.lua`.

**Confidence tags**: **[code]** read from decompiled code or string tables; **[inferred]** a
reasonable reading that was not fully traced; **[UNCONFIRMED]** open.

Parameter constructor convention [code]: `insert<MaschineNumericParameter<T>>(label, default, min, max)`;
`insert<EnumParameter>(label, default_index, options)`; `insert<AudioOutputParameter>(label, value, levelTab)`
where levelTab is 0 = Song, 1 = Group, 2 = Sound (`SongTypes::tLevelTabValues`).

---

## 1. The page model

### 1.1 Page groups [code]

`Workspace::getPageGroupParameter()` is an enum with the options `Input | Output | Groove | Macro`
(static list at `0x10e2ff400`). Lua iterates it as 0..3 and the controller shows
`string.upper(getAsString(i))`; `SlotStackStudio.lua` builds the four screen channels "INPUT",
"OUTPUT", "GROOVE", "MACRO" in that order.

Every channel object implements `getParameter(pageGroup, page, slot)` and `getPagesNames()`.
`MixingLayer::getNumPages(g)` is `s_aPageNames[g].size()` for g < 3; for g = 3 (Macro) it is the
number of `MacroPage`s. `getPageName(3, i)` is the page's `Name` parameter. Out-of-range names
return a static "None"-like empty string (`getPageName::aNone`).

Page names (`s_aPageNames`, an `array<vector<wstring>,3>` per class) [code]:

| | Input (0) | Output (1) | Groove (2) |
|---|---|---|---|
| Sound | `Audio`, `MIDI` | `Audio`, `Aux`, `MIDI` | `Swing`, `Variation`, `Generate` |
| Group | `MIDI` | `Audio`, `Aux` | `Swing` |
| Song (Master) | (none) | `Audio` | `Swing` |

A page has 8 slots (`NI.DATA.CONST_PARAMETERS_PER_PAGE`, `boost::array<…,8>` in `MacroPage`);
an unused slot returns null and is drawn empty. Section headings come from each parameter's
`setSectionName`: a new heading starts at the first parameter carrying a section name and spans the
following unnamed ones.

### 1.2 Slot tables [code]

`—` = empty slot. Labels are the parameter labels; *Section* is the heading shown above.

**Sound**

| Page | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
|---|---|---|---|---|---|---|---|---|
| Input › Audio | *Audio*: Source | Gain | — | | | | | |
| Input › MIDI | *MIDI*: Source ¹ | Channel | Thru | — | | | | |
| Output › Audio | *Audio*: Dest. | Cue | Level | Pan | Audio Mute ² | — | | |
| Output › Aux | *Aux 1*: Dest. | Level | Order | — | *Aux 2*: Dest. | Level | Order | — |
| Output › MIDI | *MIDI*: Dest. | Channel | Transpose | Aftertouch ³ | — | | | |
| Groove › Swing | *Swing*: Amount | Cycle | Invert | — | | | | |
| Groove › Variation | *Velocity*: Amount | *Time Shift*: Step | *Note Length*: Steps | Variation | — | — | *Spread*: Distribution | Live |
| Groove › Generate | see §5.2 (mode-dependent) | | | | | | | |
| Macro › page n | the 8 `ForwardParameter`s of `MacroPage` n | | | | | | | |

¹ The standalone app shows the first EventInputParameter (`Sound+0x118`); a plug-in instance
(`HeadlessInstance::isPlugin()`) shows the second one (`Sound+0x120`). They are separate saved values.
² Slot 4 is **Audio Mute** (`+0x170`), not Mute (`+0x168`); Mute is the mixer/pad mute.
³ Only when `NI::GP::Features::isOn(0)` (a feature flag) is set; otherwise the slot is empty.

**Group**

| Page | 0 | 1 | 2 | 3 | 4 | 5–7 |
|---|---|---|---|---|---|---|
| Input › MIDI | *Routings*: Key Mode ⁴ | Source ⁵ | Channel ⁵ | Start Note ⁵ | Thru ⁵ | — |
| Output › Audio | Dest. | Cue | Level | Pan | — | — |
| Output › Aux | Dest. | Level | Order | — | Dest. | Level, Order, — |
| Groove › Swing | Amount | Cycle | Invert | — | | |

⁴ Two Key Mode parameters exist: `Group+0x1b0` is used in the standalone app, `+0x1b8` in a plug-in.
⁵ Slots 1–4 exist only while Key Mode = `Manual` (value 2); otherwise the page shows only Key Mode.

**Song (Master)**

| Page | 0 | 1 | 2 | 3 | 4 | 5 | 6–7 |
|---|---|---|---|---|---|---|---|
| Output › Audio | Output (main dest.) | Level | Pan | Output (cue dest.) | Level (cue) | Pan (cue) | — |
| Groove › Swing | Amount | Cycle | Invert | — | | | |

The Master has no Input page and no Aux page. [code]

### 1.3 Groove/Variation focus helpers [code]

`ParameterPageAccess::setFocusPageOnGroovePageGroup(App, page)` sets page group 2 and the page
(Lua calls it with 1 = Variation). `setFocusPageOnOutputPageGroup` does the same for Output.

---

## 2. Parameters, defaults, ranges and the mxprj field mapping

Order = constructor order. "mxprj field" uses our names in `group.rs` / `song.rs`;
**bold** marks a field that was `unknown_*` / `fXXX`.

### 2.1 Sound (`Sound::Sound`, serialized by `Sound::serialize` v ≤ 10)

| member | label | type | default, range | section | mxprj field |
|---|---|---|---|---|---|
| 0x118 | Source | EventInputParameter | `"Default"` | MIDI | `event_input` (standalone MIDI-in port) |
| 0x120 | Source | EventInputParameter | `"Default"` | MIDI | `event_input_b` (plug-in MIDI-in port, v ≥ 6) |
| 0x128 | Channel | Enum | 0 = `All`; `All, 1…16` | | **`unknown_enum_a` = MIDI input channel** |
| 0x130 | Thru | bool | **true** | | **`unknown_bool_d` = MIDI Thru** |
| 0x138 | Source | AudioInputParameter | None | Audio | `audio_input` |
| 0x140 | Gain | float | 0.75, 0…1 (LevelConverter, dB display) | | `input_gain` |
| 0x148 | Dest. | AudioOutputParameter | parent (type 1), levelTab 2 | Audio | `audio_output` |
| 0x150 | Cue | bool | false | | `cue_enabled` |
| 0x158 | Level | float | 0.75, 0…1 (LevelConverter) | | `level` |
| 0x160 | Pan | float | 0, −1…1, bipolar, PanningConverter | | `pan` |
| 0x168 | Mute | bool | false | | `mute` |
| 0x170 | Audio Mute | bool | false | | `audio_mute` |
| 0x178 | Dest. | AudioOutputParameter | None (`getNoneTarget`) | Aux 1 | `aux1_output` |
| 0x180 | Level | float | 0, 0…1 | | `aux1_level` |
| 0x188 | Order | Enum | 0 = `Post`; `Post, Pre` | | `aux1_order` |
| 0x190–0x1a0 | Dest./Level/Order | as above | | Aux 2 | `aux2_*` |
| 0x1a8 | Dest. | EventOutputParameter | None | MIDI | `event_output` |
| 0x1b0 | Channel | Enum | 0 = `1`; `1…16` | | `event_output_channel` |
| 0x1b8 | Transpose | int | 0, −48…48, bipolar | | `transpose` |
| 0x1c0 | Aftertouch | Enum | 0; `Off, Channel, Poly` | | *not serialized* |
| 0x1c8 | — | SequenceVariationModule | §5.1 | | `sequence_variation` |
| 0x1d0 | — | SequenceGenerationModule | §5.2 | | `sequence_generation` |
| 0x1d8 | Base Key | ulong | 60, 0…127, MidiNoteNameConverter | | `base_key` |
| 0x1e0 | Group | Enum | 0; `None, 1…8` | Choke | **`unknown_enum_c` = Choke group** |
| 0x1e8 | Mode | Enum | 0; `Send, Receive` | | **`unknown_enum_d` = Choke mode** |
| 0x1f0 | Group | Enum | 0; `None, 1…8` | Link | **`unknown_enum_e` = Link group** |
| 0x1f8 | Mode | Enum | 0; `Send, Receive` | | **`unknown_enum_f` = Link mode** |
| 0x200 | Scroll Y | ulong | 265, 20 steps, skip-undo | | **`unknown_ulong_b` = piano-roll scroll Y (SW)** |
| 0x208 | PianorollEditorOffsetYHW | ulong | 265 | | **`unknown_ulong_c` = piano-roll scroll Y (HW)** |
| 0x210 | Vertical Zoom | double | 1.0, 0.1…128 | | **`unknown_double` = piano-roll vertical zoom** |
| 0x218 | Is Custom Name | bool | false | | **`unknown_bool_e` = name was user-edited** |
| 0x220 | Velocity | ulong | 0, 0…127 | | **`unknown_ulong_d` (v ≥ 7) = per-sound velocity (`getVelocityJamParameter`)** |
| 0x228 | SoundColor | ColorParameter | 1 | | `color` |
| 0x230 | — | ObjectVector<Sequence> | | | `sequences` |
| 0x238 | — | MidiMacro | §6.3 | | `midi_macro` |

Not automatable: both MIDI Sources, Channel, Thru, audio Source, Dest., Cue, Order, MIDI Channel,
Aftertouch, Base Key, Choke/Link. `Sound::onEventOutputChanged` is connected to the MIDI Dest.
The mixer processor is named `"SoundMixer"` with mute configuration 3; the three amplifiers are
named `"InGain"`, `"Aux1"`, `"Aux2"`. [code]

Legacy (v ≤ 5): the slot after `event_input` was a bool (`active_legacy`), read into a temporary. [code]

### 2.2 Group (`Group::Group`, `Track::Track`, `Group::serialize` v ≤ 0x17)

| member | label | type | default, range / options | mxprj field |
|---|---|---|---|---|
| 0x1b0 | Key Mode | Enum | 0; `Off, Drumkit, Manual` (section *Routings*) | **`unknown_enum_e` = Key Mode (standalone)** |
| 0x1b8 | Key Mode | Enum | same | **`unknown_enum_f` = Key Mode (plug-in)**; `unknown_bool_legacy` (v < 0x13) is converted into it [inferred] |
| 0x1c0 | Source | EventInputParameter | | `event_input` |
| 0x1c8 | Channel | Enum | 0 = `All`; `All, 1…16` | **`unknown_enum_a` = MIDI input channel** |
| 0x1d0 | Start Note | ulong | 36 (C1), 0…127, note names | **`unknown_ulong_a` = Start Note** |
| 0x1d8 | Thru | bool | false (`onGroupThruChanged`) | **`unknown_bool_a` = MIDI Thru** |
| 0x1e0 | Root Note | ulong | 60, 0…127, note names | **`unknown_ulong_d` = keyboard root note** |
| 0x1e8 | Root View | ulong | 60, 0…127 | *not serialized* |
| 0x1f0 | Scale Bank | ulong | 0 | **`unknown_ulong_e`** |
| 0x1f8 | Scale | ulong | 0 | **`unknown_ulong_f`** |
| 0x200 | Chord Mode | ulong | 0 | **`unknown_ulong_g`** |
| 0x208 | Chord Type Chromatic | ulong | 3 | **`unknown_ulong_h`** |
| 0x210 | Chord Type Non Chromatic | ulong | 3 | **`unknown_ulong_i`** |
| 0x218 | Chord Type ChordSet | ulong | 0 | **`unknown_ulong_j`** |
| 0x220 | Chord Position | ulong | 9 | **`unknown_ulong_k`** |
| 0x228 | Keyboard Scale Engine Active | bool | false | **`unknown_bool_e`** |
| 0x230 | Keyboard Scale Engine Key Mode | Enum | 1; `Guide, Mapped, Easy` (section *KEY MODE*) | **`unknown_enum_c`** |
| 0x238 | Last Active Chord Mode | ulong | 0 | **`unknown_ulong_l`** |
| 0x240 | EditorVisible | bool | false | **`unknown_bool_b`** |
| 0x248 | SamplingVisible | bool | false | **`unknown_bool_c`** |
| 0x250 | PianorollVisible | bool | false | **`unknown_bool_d`** (also the Generate "polyphonic" switch, §5.2) |
| 0x258 | StepGridMode | bool | false | **`unknown_bool_f`** (v > 0x11); `unknown_bool_v10` (v 10/11) is probably the same [inferred] |
| 0x260 | SoundVectorOffset | ulong | 0 | **`unknown_ulong_b`** (which bank of 16 sounds is shown) |
| 0x268 | MidiBatchExportType | Enum | 0; `Sounds To MIDI Channels, Sounds To MIDI Notes` | **`unknown_enum_b`** |
| 0x270 | ClipEventBank | ulong | 0 | *not serialized* |
| 0x278 | Tune | float | 0, ±1 shown as ±36.00 (`FloatScaleDisplayConverter(36,"%.2f")`), 72 steps | *not serialized* |
| 0x280 | Zoom Y | Enum | 0; `1x, 2x` | **`unknown_enum_g`** |
| 0x288 | Zoom Y | Enum | 0; `1x, 2x` | **`unknown_enum_h`** (HW copy [inferred]) |
| 0x2f8 | Mode | Enum | 0; `Guitar, Chords, User` | **`unknown_enum_d` = strumming mode** (precedes `strumming_map`) |
| Track 0x148 | FocusPatternBankIndex | ulong | 0 | **`unknown_ulong_c`** |

Track members (base of Group, `Track::serialize`; also the legacy `track_parameters`): 0xe8 Dest.
(levelTab 1) = **`fe8`**, 0xf0 Cue, 0xf8 Level 0.75, 0x100 Pan, 0x108 Mute, 0x110 Dest. (aux 1) =
**`f110`**, 0x118 Level, 0x120 Order (`Post, Pre`) = **`f120`**, 0x128/0x130/0x138 aux 2 =
**`f128`/`f138`**, 0x140 Color (default 9), 0x148 FocusPatternBankIndex. Groups have no Audio Mute. [code]

### 2.3 Song / Master (`Song::Song`)

| member | label | default, range | song.rs field |
|---|---|---|---|
| 0x100 | BPM | 120, 1…999 | `tempo` |
| 0x128 | Output | AudioOutputParameterValue(), levelTab 0 | **`f128` = master output destination** |
| 0x130 / 0x138 | Level / Pan | 0.75 / 0 (−1…1) | `level` / `pan` |
| 0x140 | CueEnabled | false | **`f140`** |
| 0x148 | CueMute | false | **`f148`** |
| 0x150 | Output | levelTab 0, extra flag true | **`f150` = cue output destination** |
| 0x158 / 0x160 | Level / Pan (cue) | 0.75 / 0 | `cue_level` / **`f160` = cue pan** |
| 0x168 | LoopActive | false | **`f168`** |
| 0x170 | Scene Loop | true | **`f170`** |
| 0x180 | LevelTab | 2; `ChainPanel.Song, ChainPanel.Group, ChainPanel.Sound` | **`f180` = Master/Group/Sound tab of the Control area** |
| 0x1c8 | Arp Preset Slot | 2, 1…4 | **`f1c8`** |
| 0x1f8 / 0x200 | PatternEditorOffsetHW / PatternEditorZoomHW | | **`f1f8` / `f200`** |
| 0x268 | Arranger Height | 273 (0x111), min 10 | **`f268`** |
| 0x2d0 | Fix Vel | 100, 1…127 | `fixed_velocity` (also Generate's velocity, §5.2) |

### 2.4 MixingLayer (shared by Sound, Group/Track, Song)

| member | label | type | default, range | mxprj |
|---|---|---|---|---|
| 0x58 | MacroPage | ulong | 0 | `macro_page` (focused macro page) |
| 0x60 | MacroFocus | ulong | 0 | `macro_focus` (focused macro slot) |
| 0x68 | Name | WString | | `name` |
| 0x70 | Amount | float | 0, 0…100, PercentOneDecimal, 100 steps, fine 0.01 / coarse 0.1 (section *Swing*) | `swing_amount` |
| 0x78 | Cycle | Enum | 0; `1/8, 3/16, 1/4, 6/16, 1/2, 9/16, 1/1, 2/1` | `swing_cycle` |
| 0x80 | Invert | bool | false | `swing_invert` |
| 0x88 | Bounce | bool | **true**, not automatable, on no page | `swing_bounce` [UNCONFIRMED purpose] |
| 0x90 | ProbabilityLock | bool | false | *not serialized here* |

The Cycle labels in `crates/model` (`3/8`, `1`, `2`) differ from NI's strings: NI shows `6/16`,
`1/1`, `2/1`. The swing math itself (`Swing::calcToSwingPos`) is already in `crates/model`;
`Swing` has only `setAmount`, `setCycleIndex`, `setInvert`; nothing reads Bounce in the swing code. [code]

---

## 3. Routing values

### 3.1 AudioOutputParameterValue `{type, index, group, sound, slot}` [code]

`toString(levelTab, short)`:

| type | meaning | text |
|---|---|---|
| 0 | none | `None` |
| 1 | parent | `Group` when the owner is a Sound (levelTab 2), `Master` when it is a Group (1), empty otherwise |
| 2 | master | `Master` |
| 3 | external output pair `index` | `Ext. %d` (index+1) |
| 4 | a Group (`group`) | group label (`Group::getLabel(displayIndex)`, e.g. `A`) then, unless `short`, ` - ` + name |
| 5 | a Sound (`group`,`sound`) | group label + `:S%d` (sound index+1), then ` - ` + name unless `short` |
| other | | empty |

Sound default = type 1 (to its Group), Group default = type 1 (to Master), aux default = `getNoneTarget()` (type 0).
The menu entries come from `ObjectCache::getValuesVector<AudioOutputParameter>` (out/decomp/chan/0117.c);
feedback loops are filtered by `ObjectCache::testRoutingForFeedback`. [code]

### 3.2 AudioInputParameterValue [code]

| type | text |
|---|---|
| 0 | `None` |
| 1 | external input `index`: index < N (N = host input count / 2) → `In %d L+R`; next 2N → mono `In %d L` / `In %d R` (odd → L) |
| 2 | a Sound (`SoundAlgorithms::labelForSound`, formatted `{}-{}`) |
| 3 | `Master` |
| 4 | a Group: label + ` - ` + name |

### 3.3 EventInputParameter (MIDI in) [code]

The value is a **port-name string** (`ValueParameter<std::wstring>`). Constructor flag true →
default `"Default"`, false → `"None"` (`EventInputParameter::getDefaultValue(bool)`). Sounds use
true. `updatePort()` turns the string into a port code:

| string | code | meaning |
|---|---|---|
| `Default` | `s_nPortDefault` | the port chosen in Preferences (`setDefaultPort`) |
| `Focus` | −5 | follow focus [inferred: the default port, delivered only to the focused channel] |
| `None` | −2 | no input |
| `All` (standalone) / `Host` (plug-in) | −1 | all ports / the host's MIDI |
| anything else | `MidiDeviceManager::getInputDevicePort(name)` | a named device |

Static menu entries (`MidiDeviceManager::addStaticInputDevices`), followed by the devices:
standalone `Default, Focus, None, All`; plug-in `None, Default, Host, Focus`.

### 3.4 EventOutputParameterValue (MIDI out) [code]

Serialized as `{type, port string, sound, slot}`. `updatePort()`: `None` → −2,
`All` (standalone) / `Host` (plug-in) → −1, else `getOutputDevicePort(name)`. Type 2 targets a Sound
(text = group label + sound index + ` - ` + name). The RT side is `SoundEventOutputProcessor` (§6.1).

---

## 4. MIDI behaviour

### 4.1 Sound MIDI out (`RT::SoundEventOutputProcessor`) [code]

- Port target −2 (`None`) drops all events; otherwise each MIDI event of the Sound is re-sent.
- Every channel message gets its channel nibble replaced by the **Channel** parameter (0–15 = `1…16`).
- Note-on/off note numbers get **Transpose** added; if the result is outside 0…127 the original
  note is kept.
- Sounding notes are remembered (`PlayingNote` pool) so the note-off uses the transposition and
  channel of its note-on even if the parameters changed in between.
- Thru (`processThruParameter`) and the Group's Thru (`processGroupThruParameter`) let incoming MIDI
  pass to the output as well; `DuplicateExternalEventDetectorProcessor::setMidiThru` suppresses
  echoed duplicates.

### 4.2 Group MIDI input [code + inferred]

Key Mode (`Off, Drumkit, Manual`) decides who listens: in `Manual` the Group's own Source/Channel/
Start Note/Thru apply (Start Note default 36 = the note of pad 1 [inferred]); in `Drumkit` the pads
map to the Sounds' Base Keys (`Song::onKitModeChanged`, `Song::connectSoundBaseKeySignals`);
`Off` leaves input to each Sound's own MIDI Source. Sound input filtering = port (§3.3) + Channel
(`All` = omni).

### 4.3 MidiMacro (Sound) [code]

130 int parameters, labels `CC 0` … `CC 127` (format `CC {}`), each 0…127 default 0, fine step 0.01,
plus slot 128 (channel pressure) and slot 129 (14-bit pitch bend). They are automatable.
`MidiMacroProcessor::process` sends, for each slot whose value changed in this block, a MIDI event on
channel 1 (before the Sound's channel rewrite): `0xB0 cc value`, `0xD0 value`, or `0xE0 lsb msb`.
This is how automation lanes produce MIDI CC on a Sound's MIDI output. [code; the visible labels of
slots 128/129 are UNCONFIRMED]

### 4.4 MidiAutomator (MixingLayer `midi_automators`) [code]

A MIDI automation lane: `mode` enum `Not assigned, Note, Velocity, Pitchwheel, Controller,
Program change, Aftertouch, Poly-Aftertouch, Probability` and `controller` (ulong). Created by
`MidiAccess::createMidiAutomator / focusOrCreateMidiAutomator`, recorded via
`EventRecorderAlgorithms::focusOrCreateMidiAutomator`.

---

## 5. Groove: Variation and Generate

Both modules hang off each Sound and work on the **focused pattern's active tick interval** of the
Sound's sequence (`SongAlgorithms::getFocusPatternAndActiveTickInterval`). They are destructive edits
pushed as undoable actions (the Apply / Generate button, see channel-manual §5.3). Ticks: 960 per
quarter note. Randomness comes from `std::mt19937` seeded from `std::random_device("/dev/urandom")`
every time, so results are **not** reproducible. [code]

### 5.1 Variation (`SequenceVariationModule`, typeId `SequenceVariationModule`)

| slot | member | label | type | default, range | section | mxprj (`sequence.rs`) |
|---|---|---|---|---|---|---|
| 0 | +0x48 | Amount | float | 0, 0…100 %, 100 steps | Velocity | `pa` |
| 1 | +0x50 | Step | float | 0, 0…100 % | Time Shift | `pb` |
| 2 | +0x58 | Steps | float | 0.5, 0…1, bipolar, own converter (`0x10e2ff588`) | Note Length | `pc` |
| 3 | +0x60 | Variation | float | 0, 0…100 % | (Note Length) | `pd` |
| 6 | +0x68 | Distribution | Enum | 1 = `Natural`; `Even, Natural, Skewed` | Spread | `enum` |
| 7 | +0x70 | Live | bool | false | | `flag` |

`applyVariation` [code] builds `HumanizeNoteEventsAction(sequence, {0: Amount/100, 1: Step/100,
2: Variation/100}, Distribution, interval, snap, lengthScale = Steps.getValueConverted())`:
1. For every note in the interval it draws, at full strength, a velocity deviation, a time deviation
   (max = ±1 pattern-editor snap step, `Song::getPatternEditorSnapInTicks`) and a length deviation
   (max ±1 snap step).
2. `execute` scales them: velocity = v + Amount·dv clamped 0…127; position = t + Step·dt (≥ 0,
   `relocateEvent`); length = (len × lengthScale) + Variation·dl, at least 10 ticks.
3. Each deviation is drawn in a window `[x − s, x + s]` with s = amount·range/2 (velocity: range 127,
   clamped to 0…127; time/length: range 2·step, clamped to x ± step) using the Distribution:
   `Even` = uniform integer; `Natural` = normal(mean = window centre, σ = width/4);
   `Skewed` = normal(mean = window low end, σ = width/4). Out-of-window draws are redrawn.
   (`calcRandomVelocityDeviation`, `calcRandomTimeDeviation`, `calcRandomLengthDeviation`.)
`Live` re-applies automatically when parameters change (`onLiveModeChanged`) [inferred].
`applyLengthScaling` applies only the length scale [inferred from name and call sites].

### 5.2 Generate (`SequenceGenerationModule`, typeId `SequenceGenerationModule`)

| member | label | type | default, range / options | section | mxprj |
|---|---|---|---|---|---|
| +0x48 | Mode | Enum | 0; `Random, Euclidean` | Engine | `p48` |
| +0x50 | Amount | float | 55, 10…100 % | Probability | `p50` |
| +0x58 | Amount | float | 10, 0…100 % | Velocity | `p58` |
| +0x60 | Note Range | Range (`Note Lo`, `Note Hi`) | 60…72, 0…127, note names | Note Range | `p60` |
| +0x68 | Note Count | ulong | 1, 1…6 | Chords | `p68` |
| +0x70 | Distribution | Enum | 1; `Even, Natural, Skewed` | Spread | `p70` |
| +0x78 | Amount | ulong | 8, 2…32 (max follows Division) | Steps | `p78` |
| +0x80 | Amount | ulong | 5, 1…8 | Fills | `p80` |
| +0x88 | Steps | ulong | 0, 0…7 | Rotate | `p88` |
| +0x90 | Division | Enum | 0; `Grid, 1/4, 1/8, 1/16, 1/32, 1/8 T, 1/16 T, 1/4 Q, 1/8 Q, 1/16 Q, Fit Bar` | Division | `p90` |
| +0x98 | Live | bool | false | | `live_mode` |

Page slots (`Sound::getParameter(2, 2, i)`):
- `Random`: Mode, Probability, Velocity, Note Lo ⁶, Note Hi ⁶, Note Count ⁶, Distribution, —.
- `Euclidean`: Mode, Steps, Fills, Rotate, Division, —, —, Live.
⁶ only in polyphonic mode = the Group's `PianorollVisible` is on; otherwise empty.

Context (`injectSequenceContextRetrieverIntoSound::$_1`): sound, pattern + interval, snap ticks, ticks
per bar, polyphonic flag, **note = Group Root Note** (60 without a group), **velocity = Song "Fix Vel"**
(100), scale-engine bank/type. [code]

**Random** (`generateRandom`) [code]: clears the interval, then for every snap step from the
interval start: `calcRandomChord(rng, P = Probability/100, v = Fix Vel, a = Velocity/100,
range, count, distribution)`:
- with probability P the step gets a chord, else nothing;
- chord size k ∈ [max(1, n − ⌊(n−1)/2⌋), n] drawn with the Distribution (n = Note Count; 1 when
  not polyphonic);
- each note: pitch uniform in [Note Lo, Note Hi] without duplicates (not polyphonic: the root note);
  velocity v ± a·63.5 drawn with the Distribution;
- pitches are snapped to the scale (`ScaleHelper::getScaleOffsetForNote`), duplicates dropped;
  notes last one snap step.

**Euclidean** (`generateEuclidean`) [code]: N = Steps, K = Fills, R = Rotate.
- Pattern bits: `acc = N − K; for i in 0..N { acc += K; bit[i] = acc ≥ N; if bit[i] { acc −= N } }`,
  then rotated by R (`std::rotate` on the bit vector).
- Step length by Division: Grid = snap ticks; 1/4 = 960; 1/8 = 480; 1/16 = 240; 1/32 = 120;
  1/8 T = 320; 1/16 T = 160; 1/4 Q = 768; 1/8 Q = 384; 1/16 Q = 192; Fit Bar = bar length / N.
- The cycle (N steps) repeats from the interval start (aligned down to a cycle) to the interval end;
  each set bit inside the interval gets a note (root note, Fix Vel) one step long.
- `recalcMaxSteps` sets Steps' maximum by Division: Grid → 3840 / snap (≥ 2), 1/4 → 4, 1/8 → 8,
  1/16 → 16, 1/8 T → 12, 1/16 T → 24, 1/4 Q → 5, 1/8 Q → 10, 1/16 Q → 20, others 32.

---

## 6. Macro

### 6.1 Data model [code]

- `MixingLayer::macro_pages` = `ObjectVector<MacroPage>`; each `MacroPage` (a `ParameterPage`) has a
  `Name` WString and `boost::array<shared_ptr<ForwardParameter>, 8>` created with empty labels.
- `ForwardParameter` = a proxy: `target` (MaschineParameter*), custom name (our `unknown_1e8`, i.e.
  **user-entered name**), `section_name`. `getName()`: target set → custom name if non-empty, else
  the target's name; no target → its own name. Values set on the proxy are forwarded to the target
  (`setValueAudioThread`, `takeoverValueFromAudioThread`).
- `MacroPage` / `MacroFocus` (MixingLayer ulongs) remember the focused page and slot.
- Macro pages appear as page group 3; `getParameter(3, page, slot)` = forward parameter.

### 6.2 Assignment [code + inferred]

- `ParameterPageAccess::setMacroForParameter(App, param, pages)` / `setOrRemoveMacroForParameter`
  toggles a macro for a parameter at the current level (Sound/Group/Master tab);
  `getNextUnassignedForwardParameter` takes the next free slot after the last assigned one on the
  last page, and `addMacroPage` appends a page (undo text `Create %s`) when none is free [inferred].
- `setMacroForwardParameter(fp, target, name)` binds a slot; `isMacroEqual` compares target + name.
- `removeMacroPage`, `setCustomName`, `setSectionName` edit names; `MacroAssignmentEditor` is the
  breadcrumb picker in the Assignment area (Sound/Group/Master → module → parameter).
- Controller: `MacroPageStudio.lua` (button 5 leaves the macro page group via
  `MacroHelper.leaveMacroPageGroup`), page/slot navigation via `ControlHelper.hasPrevNextSlotOrPageGroup`.

---

## 7. Style-sheet layout of the Control area

Tokens [code]: `$colorSurface00` rgb(24,24,24), `$colorSurface02` rgb(50,50,50), `$colorSurface05`
rgb(33,33,33), `$colorContentHighLight` rgba(255,255,255,245), `$colorContentMediumLight`
rgba(255,255,255,164), `$colorContentLowLight` rgba(255,255,255,142), `$colorContentSubLight`
rgba(255,255,255,61), `$metricSectionGrid` 64. Pictures have 7 stacked animation frames
(`animations: 7`): normal, over, down, selected, … [inferred frame order].

### 7.1 Overall [code]

```
Bar#m_pChainPanel { background-color: $colorBlack; spacing: 1; }
Bar#m_pChainPanel[Height=Normal]    { height: 140; }
Bar#m_pChainPanel[Height=Maximized] { height: 192; }
```
Left to right: `Bar#m_pLeft` (icon strip) │ `Bar#m_pColumnsManager` (tabs, title, property list)
│ `Bar#pParameterEditor` (parameter area) — with `Stack#m_pAssignmentAreaEditor` below when the
lower area is open.

### 7.2 Far-left icon strip (`Panels/LeftPanel.txt`) [code]

```
Bar#m_pChainPanel Bar#m_pLeft { background-color: $colorSurface05; width: 32; }
Bar#m_pChainPanel Bar#m_pLeft Button#m_pChannelMode { image: url("Maschine/LeftPanel/LP_BTN_ChannelMode.png"); animations: 7; outer-margin-top: 5; }
Bar#m_pChainPanel Bar#m_pLeft Button#m_pPluginMode  { image: url("Maschine/LeftPanel/LP_BTN_PluginMode.png");  animations: 7; outer-margin-top: 3; }
Bar#m_pChainPanel Bar#m_pLeft Label#pDummy { height: 60; }
Bar#m_pChainPanel Bar#m_pLeft Button#m_pShowHideLowerArea { image: url("Maschine/LeftPanel/LP_BTN_ExtendDown.png"); animations: 7; }
```
Both mode pictures are 32×168 → 32×24 per frame. Channel mode shows the property list (Input/
Output/Groove/Macro); Plug-in mode shows the slot list (`MaschineHelper:isShowingPlugins()`).

### 7.3 Column manager: level tabs, title, property list [code]

```
Bar#pTabArea { background-color: $colorSurface05; image: url("Maschine/ParameterArea/PA_BG_MGSTab.png"); fixed-left: 2; fixed-right: 2; margin-left: 8; spacing: 8; }
Bar#pTabArea Button { font-name: $fontRobotoMedium; font-size: 12; foreground-color: $colorContentLowLight; outer-margin-top: 4; }   // :over MediumLight, :selected HighLight
Button#m_pMasterTabButton { width: 48; }  Button#m_pGroupTabButton { width: 42; }  Button#m_pSoundTabButton { width: 43; }
Bar#pTitleArea { background-color: $colorSurface05; height: 32; outer-margin-bottom: 1; }
Bar#pTitleArea Label#m_pInfo { font-name: $fontRobotoRegular; font-size: 15; outer-margin-left: 8; margin-top: 4; }
Stack#m_pModes { background-color: $colorSurface05; height: 88; outer-margin-bottom: 1; }
Label#pSpacer  { background-color: $colorSurface05; height: 8; }
```
The tab buttons set Song `LevelTab` (`ChainPanel::onLevelTabButton`).

Property list ("BIG DIVIDED BY FOUR BUTTON AREA"), a 2×2 grid of picture tiles:
```
Stack#m_pModes Bar#pPageGroups     { margin: 3; spacing: 1; }
Stack#m_pModes Bar#pPageGroups Bar { spacing: 7; }
Button#Input  { image: url("Maschine/ParameterArea/pa_button_tile_in.png");     animations: 7; }
Button#Output { image: url("Maschine/ParameterArea/pa_button_tile_out.png");    animations: 7; }
Button#Groove { image: url("Maschine/ParameterArea/pa_button_tile_groove.png"); animations: 7; }
Button#Macro  { image: url("Maschine/ParameterArea/pa_button_tile_macro.png");  animations: 7; }
```
Tiles are 75×245 → 75×35 per frame; the label is part of the picture (no foreground text). Clicking
sets the Workspace page group (`ChainPanel::setPageGroup`). In Plug-in mode the same stack shows
`Bar#m_pSlotManager` (slot rows 24 high, margins 8/8/4/4, plug-in icons `PA_BTN_PluginIcon_*`).

### 7.4 Parameter area [code]

```
Bar#pParameterEditor { background-color: $colorSurface05; image: url("Maschine/ParameterArea/pa_bg_main.png"); fixed-left: 150; height: 140; }
Bar#m_pHeader { outer-margin-top: 2; height: 32; margin-left: 20; margin-top: 2; }        // page tabs row
ScrollableTab Bar#pTab { height: 24; outer-margin-right: 16; outer-margin-bottom: 2; }
Button.Tab { image: url("Maschine/ParameterArea/PA_BTN_PageTab.png"); animations: 7; margin-left/right: 1; fixed-left/right: 4; foreground-color: $colorContentLowLight; min-width: 10; }  // :selected/:over HighLight
Bar#m_pSections { height: 24; margin-bottom: 2; margin-left: 20; spacing: 13; }          // section headings
Button.AssignmentAreaItem[Edit=false] { image: url(".../pa_BTN_Section_Underline.png"); fixed-left/right: 5; foreground-color: $colorContentMediumLight; min-width: $metricSectionGrid; font-name: $fontRobotoMedium; font-size: 11; margin-bottom: 1; }
Bar#m_pParameters { height: 84; margin-left: 20; spacing: 13; }                            // 8 controls
Bar#m_pParameterWidget { width: 70; height: 84; }
Stack#m_pParameterStack { height: 56; }
ParameterDragger#m_pNumericDragger { image: url(".../pa_knob_bg.png"); diameter-inner: 40; diameter-outer: 50; center-x: 30; center-y: 26; outer-margin-top: 2; outer-margin-left: 5; }
Button#m_pEnumValue { image: url(".../PA_BTN_EnumDisplay.png"); font-name: $fontRobotoMedium; font-size: 14; }  // with m_pEnumDec / m_pEnumInc arrows
Bar#m_pBoolBar { width: 32; height: 24; outer-margin-left: 20; outer-margin-top: 20; }  Button#m_pBoolButton { image: url(".../pa_button_on_off.png"); }
DraggerableValueEdit { image: url(".../PA_BTN_EnumDisplay.png"); outer-margin-top: 17; }    // e.g. Base Key, routing values
Stack#m_pNameStack { height: 16; }  Stack#m_pNameStack Button { foreground-color: $colorContentHighLight; vertical-alignment: bottom; }
Label#m_applyButtonSeparator { background-color: $colorSurface02; width: 1; outer-margin-left: 6; outer-margin-right: 8; }
Button#m_applyButton { image: url(".../PA_BTN_Generic.png"); fixed-left/right: 4; foreground-color: $colorContentMediumLight; width: 54; height: 24; outer-margin-top: 22; }
```
Widget kind per parameter type: float/int → knob (`pa_knob_ring_uni` / `_bi` for bipolar such as
Pan and Transpose); enum and routing → value box with ◂ ▸; bool → on/off button. Section
headings sit on a 64-px grid (one heading per started section). Width per control 70 + spacing 13
→ 8 controls ≈ 20 + 8·70 + 7·13 = 671 px. [code + inferred arithmetic]

Assignment area (below; MIDI / Host automation / Macro pages): `Bar#m_pAssignmentAreaControl`
height 36, tab buttons 151 wide (`PA_BTN_AssignmentType.png`); `Stack#m_pAssignmentAreaEditor`
background `$colorSurface00`, height 46. [code]

---

## 8. Unresolved

- Exact meaning of `Focus` as a MIDI input (−5 port code) and of `swing_bounce` / `ProbabilityLock`.
- Display labels of MidiMacro slots 128/129 (lambdas `MidiMacro::MidiMacro()::$_0…$_2` not read).
- The Variation "Steps" length-scale converter (`0x10e2ff588`) display format, and what `Live`
  re-triggers exactly.
- Group Key Mode `Drumkit` vs `Off` routing details (only parameter wiring was read; the RT graph
  connection code was not decompiled).
- `SoundConfig` and `ParameterSnapshotsManager` unknowns were out of scope.
- Picture frame order of the 7-frame buttons is assumed, not verified.

---

## 9. Where routing is stored in a project

Sources: `out/decomp/routecfg/` (names in `out/decomp/routecfg-names.txt`): `SoundConfig::SoundConfig(Sound const&)`,
`SoundConfig::load`, `SoundConfig::serialize<oarchive>`, `SoundConfig::dropCrossGroupRouting(s)`,
`Group::serializeSoundStates<i/oarchive>`, `Track::serialize<oarchive>`, `MaschineRoutingParameter<…>::save`,
`AudioOutputParameterValue::serialize<oarchive>`; plus `schema/0397.c` (SoundConfig load), `schema/0337.c`
(AOPV load), `chan/0098.c` (`Sound::Sound`), `chan/0090–0092.c` (EOPV constructors). Checked against
`Cloud 9 Vibes.mxprj` (Song v29, Group v22, Sound v9, SoundConfig v7, Track v1, value types v3).

### 9.1 The skip flag

`MaschineRoutingParameter<V>::save` writes the member `m_skipValue` (+0x388) as a byte, then the value only
if it is 0. Who sets it:

| owner | parameters | skip |
|---|---|---|
| Sound (`Sound::Sound`) | audio Source 0x138, Dest. 0x148, Aux 1/2 Dest. 0x178/0x190 (set to 1 in the constructor); both MIDI Sources and MIDI Dest. are also 1 in every saved file | **always 1** |
| Track (`Track::serialize`) | Dest. 0xe8 (`fe8`), Aux 1 0x110 (`f110`), Aux 2 0x128 (`f128`) | `!isProjectLevel` → 0 in a project, 1 in a group/sound file |
| Group (`Group::serialize`) | MIDI Source 0x1c0 (`event_input`) | `!isProjectLevel` (v ≥ 15) → 0 in a project |
| Song | Output 0x128 (`f128`), Cue Output 0x150 (`f150`) | never set → always 0 |

Cloud 9 Vibes: 240 Sounds × 7 routing params all skip 1; 15 Groups' `fe8`/`f110`/`f128`/`event_input`
and the Song's `f128`/`f150` all skip 0. `Song v29 ≥ 21`, so there is **no** `groups_routing`
(`GroupRoutingConfig` is legacy only, Song v < 21). [code + file]

### 9.2 SoundConfig = a Sound's routing + mixer state, stored by its Group

`Group::serializeSoundStates` (save) builds one `SoundConfig(sound)` per Sound, in Sound order; at
group level (`isGroupLevel`) it then calls `dropCrossGroupRoutings` (see 9.4). On load it reads the vector
and calls `SoundConfig::load(guard=true, sounds[i])` for **every** i of the Group's sounds: it indexes the
config vector by the sound index without a bounds check, so the vector must have as many entries as the
Group has sounds (16). `load` runs after `Sound::serialize` and after the Track base, and writes every field
below back into the Sound (`EventInputParameter::loadValueAndSetPort`, `EventOutputParameter::loadValueAndSetPort`,
`ValueParameter::loadValue`). [code]

| SoundConfig (offset, our name) | type | Sound member ← → | meaning |
|---|---|---|---|
| 0x000 `name` | wstring | 0x118 | MIDI Source, standalone (port string, §3.3; e.g. `Default`, `Focus`) |
| 0x018 `unknown_18` (v ≥ 7) | wstring | 0x120 | MIDI Source, plug-in |
| 0x030 `unknown_30` | u64 | 0x128 | MIDI input Channel (0 = All, 1…16) |
| 0x038 `unknown_38` (v ≥ 5; older = true) | bool | 0x130 | MIDI Thru |
| 0x040 `audio_input` | AudioInputParameterValue | 0x138 | audio Source |
| 0x098 `unknown_f_a` | f32 | 0x140 | input Gain |
| 0x0a0 `audio_output` | AudioOutputParameterValue | 0x148 | Dest. |
| 0x0f8 `unknown_f8` | bool | 0x168 | Mute |
| 0x0f9 `unknown_f9` | bool | 0x170 | Audio Mute |
| 0x100 `aux1_output` / 0x158 `aux1_level` / 0x160 `aux1_order` | AOPV / f32 / u64 | 0x178 / 0x180 / 0x188 | Aux 1 |
| 0x168 `aux2_output` / 0x1c0 `aux2_level` / 0x1c8 `aux2_order` | | 0x190 / 0x198 / 0x1a0 | Aux 2 |
| 0x1d0 `event_output` | EventOutputParameterValue | 0x1a8 | MIDI Dest. |
| 0x220 `unknown_220` | u64 | 0x1b0 | MIDI output Channel (0 = `1`) |
| 0x228 `unknown_228` | i32 | 0x1b8 | Transpose |
| 0x230 | u64 | 0x1c0 | Aftertouch — copied and applied but **not serialized** |
| 0x238 `unknown_238` | u64 | 0x1e0 | Choke group |
| 0x240 `unknown_240` | u64 | 0x1e8 | Choke mode |
| 0x248 `unknown_248` | u64 | 0x1f0 | Link group |
| 0x250 `unknown_250` | u64 | 0x1f8 | Link mode |
| 0x258 `unknown_258` | bool | 0x88 (MixingLayer) | swing Bounce (default true) |
| 0x259 `unknown_259` | bool | 0x150 | Cue |
| 0x260 `sidechain_inputs` (v ≥ 4) | map<slot index, SideChainInputParameterValue> | Chain | side-chain input of each plug-in slot (applied only if the slot exists) |

Cloud 9 Vibes: `unknown_220` = 0…15 (sound index; the default "Sounds to MIDI channels"), matching each
Sound's own `event_output_channel`; `name` is `Focus` for 6 Sounds, `Default` otherwise. Legacy: v < 3 the
plug-in port is `Default`; v 3–4 a bool picks `Default`/`All`-or-`Host`; v 0 has two extra floats after
`audio_output` and an extra u64 after transpose. [code + file]

### 9.3 Which value wins on load, and what a writer must write

- **Sound**: the SoundConfig always wins. Its routing params are stored skipped, and even if a writer
  stores a value (skip 0), `SoundConfig::load` overwrites it — as it also overwrites the scalars that
  `Sound::serialize` stores directly (input channel, thru, gain, mute, audio mute, cue, aux levels/orders,
  MIDI out channel, transpose, choke/link, bounce). To change any of: MIDI Source (both builds), input
  Channel, Thru, audio Source, Gain, Dest., Aux dests/levels/orders, MIDI Dest./Channel/Transpose, Mute,
  Cue, Choke/Link → **write the Group's `sound_configs[i]`**; also write the same scalars in the Sound so
  the two copies stay identical (NI does), and keep the Sound's routing params at `skip_value 1`.
- **Group** (project file): Key Mode (`unknown_enum_e` standalone / `unknown_enum_f` plug-in), MIDI Channel
  `unknown_enum_a`, Start Note `unknown_ulong_a`, Thru `unknown_bool_a` are plain parameters; MIDI Source
  is `event_input` with `skip_value 0`. Output = Track `fe8`, Aux 1 = `f110` + `aux1_level` + `f120`
  (order), Aux 2 = `f128` + `aux2_level` + `f138`, all with `skip_value 0`. Nothing else overrides them
  (no `groups_routing` in Song ≥ 21). In a group file (.mxgrp) these routing values are skipped.
- **Master**: Song `f128` (Output) and `f150` (Cue output), always stored (`skip_value 0`). Cloud 9 Vibes:
  both `type 3, index 0` = `Ext. 1`.

### 9.4 Value encodings and object pointers

All three value types are class version 3:

- `AudioOutputParameterValue` = `{type i32, index i64, Group*, Sound*, Slot*}`; `AudioInputParameterValue`
  the same layout. Types as §3.1/§3.2 (`index` = external pair for output type 3 / input type 1, else −1).
- `EventOutputParameterValue` = `{type i32, port wstring, Sound*, Slot*}`; type 0 = none (`"None"`),
  1 = MIDI port named `port` (`All`/`Host` = −1), 2 = a Sound (`Sound*`).
- The pointers are boost polymorphic **raw pointers with object tracking** (v3; v2 used `weak_ptr`s, v < 2
  raw pointers plus a slot fix-up). A null is class id `0xffff`. A non-null pointer to an object already in
  the stream is a back-reference to its tracked object id (our dump prints `-> #N`); a pointer to an object
  not yet written makes boost write the **whole object inline** at that point, and the later regular
  occurrence (the `ObjectVector<Group>` shared_ptr) becomes the back-reference.
- Routing a Sound to Group B (type 4): write `type 4, index -1, group = pointer to B, sound null, slot null`.
  If B precedes the current Group in `song.groups` (or is the current Group, whose id is already assigned),
  this is a back-reference to B's object id; if B comes later, NI's own saver inlines B there (writers
  should prefer earlier/own-group targets or implement inline-first emission). Type 5 (a Sound) sets both
  `group` and `sound`. Group outputs (`fe8` etc.) follow the same rule.
- **Index alone does not suffice** for types 4/5: the target is identified only by the pointer
  (`toString`, `dropCrossGroupRouting` compare `group`/`sound` pointers; `index` is used only for external
  ports), and no post-load fix-up from `index` was found. [code; "no fix-up" = not found, not proven]
- Group-level save (`isGroupLevel`) runs `dropCrossGroupRoutings`: a Sound output of type 4, or type 5 to a
  Sound outside this Group, becomes `parent` (Dest.) / `None` (aux); an audio input of type 2 from another
  Group becomes `None`; side-chain inputs likewise. Project files keep cross-group routing. [code]
