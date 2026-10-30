# Maschine 3 Sampling ▸ Record: how the software implements it

An implementation spec for DECKARD's Record tab, taken from Maschine 3's own code. It complements:
- `docs/recording-manual.md`: what NI's manual says;
- `docs/sampling-layout.md` §3: the panel's pixel layout and style sheet.

Where those docs mark something **[UNCONFIRMED]**, this file settles it from the code where it can.

**Sources**
- Decompiled functions:
  - `out/decomp/samp/0004.s.c` and `0009.s.c` (`SamplingParameters::updateRecordParameters`, `Recorder::Recorder`);
  - `out/decomp/samp2/*` (the `FinishedRecordingHandler` functions, `RecorderAlgorithms::loadRecordingToSound`, `Recorder::prepareNextAutoSamplerFile`);
  - `out/decomp/rec/*` (new this session: the RT `RecorderProcessor`, `AutoSamplerProcessor`, `RecorderAlgorithms`, `RecorderAccess`, `RecordingPanel`, `RecorderPostProcessJob`). Names are in `out/decomp/rec/INDEX.tsv`.
- Lambda bodies `Recorder::Recorder()::$_0…$_7`, disassembled with `objdump`. Their addresses come from the `__func` vtables at `0x10d31d420…0x10d31d740`.
- UTF-32 strings in `analysis/maschine_arm64`: the option lists are at file offset `0x4d3f028…0x4d3fd30`.
- The serialization schema: `out/schema/serialize.txt` (`Recorder::serialize`).
- NI's controller Lua (`SamplingPageBase.lua`, `SamplingHelper.lua`): enum names and record-state handling.

**Confidence tags**
- **[code]**: read directly from decompiled or disassembled code or from string tables.
- **[inferred]**: a reasonable reading of code whose details were not fully traced.
- **[UNCONFIRMED]**: not settled.

There is **no QML** for this page. The Record tab is a native NWL widget (`NI::MASCHINE::GUI::RecordingPanel`) styled by
`maschine_nwl_resources/Maschine/Panels/PatternEditorShared.txt`. Its parameter footer is the generic
`SamplingParameterEditor`, fed by `SamplingParameters::updateRecordParameters`.

---

## 1. Data model: one `Recorder` per Song

- `Song+0x320` holds a single `NI::MASCHINE::DATA::Recorder` (a `Module`). `StateHelper::getFocusRecorder` returns it, so record settings are **per project, not per sound**. [code]
- It is saved in the project (`Song::serialize` → `load Recorder`, versions 0–3) [code]. The saved fields:
  - Recording Status (bool, v≥1);
  - Source enum;
  - Ext-stereo Input enum;
  - Ext-mono Input enum;
  - the Internal Input `AudioInputParameter` (v≥2; older versions used an enum);
  - Mode enum;
  - Threshold float;
  - Length enum;
  - Monitor bool;
  - Target enum (v≥3).
- The Auto-Sampler parameters (MIDI To … Trim Sil) are **not saved**. They return to their defaults when a project loads. [code]

### Parameters (in `Recorder::Recorder()` order; the field offset is the Recorder member)

| # | label (UI) | field | type | default | range / values |
|---|---|---|---|---|---|
| – | Recording Status | 0x150 | bool | false | runtime "is recording" flag |
| 0 | **Source** | 0x158 | enum | **0 = Ext. Ster.** | `Ext. Ster.`, `Ext. Mono`, `Internal` (Lua `SOURCE_EXTERNAL_STEREO/MONO`, `SOURCE_INTERNAL`) |
| 1 | **Input** (ext stereo) | 0x160 | enum | 0 | `In 1 L+R` … `In 16 L+R` (16 entries) |
| 2 | **Input** (ext mono) | 0x168 | enum | 0 | `In 1 L`, `In 1 R`, `In 2 L` … `In 16 R` (32 entries, L/R alternating; pair = v/2, R if v odd). Changes call `Recorder::setMonoRecordingChannel` |
| 3 | **Input** (internal) | 0x170 | `AudioInputParameter` | `AudioInputParameterValue::getMasterSource()` = **Master** | Master or any Group (see §2.2) |
| 4 | **Input** (Auto + Internal) | 0x178 | `AudioInputParameter` | empty (invalid) | a Group + Sound whose plug-in is played and recorded (see §4) |
| 5 | **Mode** | 0x180 | enum | **1 = Sync** | `Detect`, `Sync`, `Loop`, `Auto` (Lua `MODE_DETECT=0 … MODE_AUTO=3`) |
| 6 | **Threshold** | 0x188 | float dB | **−12** | **−60 … 0**. Shown as `"%4.1f dB"`, and as `OFF` when ≤ −59.99. Encoder steps: coarse 0.5, fine 0.1 [inferred: `processWheelDeltaGeneric(…, 0.5, 0.1)`] |
| 7 | **Length** | 0x190 | enum | **0 = 1 Bar** | `1 Bar`, `2 Bars`, `4 Bars`, `8 Bars`, `16 Bars`, `FREE`. The RT side uses `bars = 1 << index`, and FREE (index 5) → 0 = no limit |
| 8 | **Target** | 0x198 | enum | **0 = Take** | `Take`, `Sound`, `Pattern` |
| 9 | **Monitor** | 0x1a0 | bool | false | → `RecorderProcessor::setDirectMonitoring` (input to Cue) |
| 10 | **MIDI To** | 0x1a8 | `EventOutputParameter` | `None` | MIDI output devices. Not automatable |
| 11 | **Channel** | 0x1b0 | int | **1** | **1 … 16** |
| 12 | **Note On** | 0x1b8 | ulong ms | **2000** | **100 … 30000**, 299 steps = **100 ms** steps. Not automatable |
| 13 | **Sample** | 0x1c0 | ulong ms | **5000** | **100 … 30000**, 100 ms steps. Not automatable |
| 14 | **Stride** (keys) | 0x1c8 | ulong | 1 | 1 … max. The max is set dynamically to `max(1, KeyHi − KeyLo)` |
| 15 | **Key Lo** | 0x1d0 | ulong (note) | **60 (C3)** | 0 … 127. Shown as a note name (`SizeTMidiNoteConverter`, C3 = 60, C-2 = 0, G8 = 127) |
| 16 | **Key Hi** | 0x1d8 | ulong (note) | **60 (C3)** | 0 … 127 |
| 17 | **Extend** (keys) | 0x1e0 | bool | **true** | |
| 18 | **Stride** (velocity) | 0x1e8 | ulong | 1 | 1 … `max(1, VelHi − VelLo)` |
| 19 | **Vel Lo** | 0x1f0 | ulong | **100** | 0 … 127 |
| 20 | **Vel Hi** | 0x1f8 | ulong | **100** | 0 … 127 |
| 21 | **Extend** (velocity) | 0x200 | bool | **true** | |
| 22 | **Find Loop** | 0x208 | bool | false | |
| 23 | **Normalize** | 0x210 | bool | false | |
| 24 | **Trim Sil** | 0x218 | bool | false | display name `Trim Silence` |

**Coupling rules** (the `Recorder()` change lambdas) [code]:
- **Note On ↔ Sample:**
  - setting Note On above Sample raises Sample to it;
  - setting Sample below Note On lowers Note On to it.
  - So Note On ≤ Sample always holds. Sample − Note On is the release tail.
- **Key Lo / Key Hi:**
  - setting Lo above Hi raises Hi to Lo;
  - setting Hi below Lo lowers Lo to Hi.
  - Either change then sets Key Stride's max to `max(1, Hi − Lo)`.
- **Vel Lo / Vel Hi:** the same rules, applied to Vel Stride.
- **MIDI To:** changing it copies the selected port index to the auto-sampler processor.

Note: the defaults Key 60..60 and Vel 100..100 with Extend on mean **Auto samples a single note (C3 at velocity 100), mapped
over the whole keyboard and velocity range**, until you widen the ranges.

---

## 2. Footer layout per Source/Mode (`SamplingParameters::updateRecordParameters`)

The footer has six `SamplingParameterPage` groups. They are created in the `SamplingParameters` constructor and filled
each update in this order (the order pushed into the page list) [code]:

| order | group title | contents |
|---|---|---|
| 1 | **Recording** | `Mode`, then: Detect → `Threshold`. Sync → `Length`. Loop → `Length`, `Target`. Auto → `Sample`, `Note On` |
| 2 | **Input** | `Source`, then `Input`, which is one of four parameters (below). In Auto with an **external** source, `MIDI To` and `Channel` are appended |
| 3 | **Note Map** | Auto only: `Stride`, `Key Lo`, `Key Hi`, `Extend` |
| 4 | **Velocity Map** | Auto only: `Stride`, `Vel Lo`, `Vel Hi`, `Extend` |
| 5 | **Batch Process** | Auto only: `Find Loop`, `Trim Silence`, `Normalize` |
| 6 | **Monitor** | only when Source is external (Ext. Ster. or Ext. Mono), in every mode including Auto: `Monitor`. This group is built with its title flag off [inferred: shown without a section title] |

Which `Input` parameter appears:
- Ext. Ster. → the stereo enum (0x160).
- Ext. Mono → the mono enum (0x168).
- Internal and Mode ≠ Auto → `AudioInputParameter` 0x170: Master or Groups.
- Internal and Mode = Auto → `AudioInputParameter` 0x178: Group/Sound.

Each Source keeps its own Input selection.

The controller Lua shows the same parameters across hardware pages (see `recording-manual.md` §6). The `Recording` page is
disabled while waiting or recording (`isDisabledWhenRecording`).

### 2.1 External input lists [code]
- The enum vectors are static, with **16 stereo pairs** (`In 1 L+R`…`In 16 L+R`) and **32 mono channels** (`In 1 L`…`In 16 R`).
  This matches Maschine's 16 stereo inputs in Preferences ▸ Audio ▸ Routing (§7).
- The manual's "In 1–4" is out of date.
- [UNCONFIRMED] whether the selector greys out or hides inputs that aren't mapped to a device channel. No trimming code was
  found in `Recorder` or `updateRecordParameters`.

### 2.2 Internal input list [code + inferred]
- `AudioInputParameterValue::toString` formats each value. The value types are:
  - **0/1 external:** `In %d L+R`, or `In %d L`/`R`;
  - **2 Sound:** `{}-{}` built from `SoundAlgorithms::labelForSound(group, sound)`, i.e. a group label and sound label such as `A-1` [inferred: plus the name];
  - **3/4 Master/Group:** `Group::getLabel(displayIndex)`, optionally followed by `" - "` and the mixing layer's display name, e.g. `A - Drums`.
- Non-Auto Internal (0x170): **Master** (the default) plus every **Group** (`isValidInternalInputGroup` scans the value
  list for a group). Single Sounds are not offered. Solo the sound and pick its group instead.
- Auto + Internal (0x178): a **Group + Sound** (the value carries group at +0x10 and sound at +0x18). It starts empty, and
  START stays disabled until a sound is picked (`hasValidMidiOutputSelected` returns `value != empty` for Internal).
  MIDI To and Channel are hidden, because notes go **internally** to that sound (§4.2).

---

## 3. Panel behaviour (`RecordingPanel`)

### Buttons (`updateRecordButtons`) [code]

The start button's text and state follow `Recorder::getState()`:

| state | text | enabled | selected |
|---|---|---|---|
| 0 idle | `Start` | false only in Auto when `hasValidMidiOutputSelected` is false | no |
| 3 | `Waiting` | yes | no |
| 1, 2, 6, 7, 8 | `Stop` | yes | yes |

- The **Cancel** button is enabled whenever the state ≠ idle.
- The **threshold slider** on the meter is enabled only while idle. The controller Lua shows it only in Detect.
- The slider maps 0…1 linearly to −60…0 dB (`setThreshold`: `dB = x*60 − 60`). Double-click calls `resetRecThreshold`, which resets
  Threshold to its default (−12 dB), only in Detect. The undo label is "Set default".

### Info-bar status strings (`RecorderAccess::getRecordingInfoString`) [code]
- `Waiting for input...`: Detect, armed.
- `Waiting for next bar...`: Sync and Loop, armed.
- `Waiting for end of Pattern...`: Loop, when the grid is the pattern length.
- `Waiting...`
- `Recording...`
- `Recording {note} at {velocity} ({i}/{n})`: Auto, e.g. `Recording C3 at 100 (1/1)`.
- `Processing...`: state 8, post-processing.
- There is also an empty-page hint, `Press START to record your input source`.

### Take list context menu [code]
- `Delete`
- `Delete unused Takes` (`SoundAccess.removeUnusedTakes`)
- `Map Takes to Zones` (`SoundAccess.mapTakesToZones`)

The manual's wording differs ("Remove unused recordings" / "Map recordings to zones"); use the code strings above.

### After a recording finishes [code]
`RecorderAccess::onRecorderStopped` runs `FinishedRecordingHandler::finishRecording` in one undo context. It then switches the
Sampling tab:
- **Zone** (tab 3) after an Auto run;
- otherwise **Record** (tab 0).

---

## 4. Auto Sampler

### 4.1 Building the item list (`RecorderAlgorithms::createAutoSamplerItems`) [code]
1. `keys = getRangesWithRoot(KeyStride, KeyLo, KeyHi, KeyExtend)`.
2. `vels = getRangesWithRoot(VelStride, VelLo, VelHi, VelExtend)`.
3. Build items **note-major**: `for k in keys { for v in vels { item(root=k.root, vel=v.root, keyRange=k, velRange=v) } }`.
   The recording order is the same: every velocity layer of the lowest note, then the next note, and so on.
4. The file for each item is `<dir>/<stem> <NoteName> <velocity>.wav`, made unique (`FileNameUtils::makeFilePathUnique`).
   - `<dir>` and `<stem>` come from the recorder's file path (§6).
   - `NoteName` uses `MidiStrings::getMidiNoteName`: `C-2`…`G8`, `C#3` style, with C3 = 60.
   - Example: `20261014T153000 C3 100.wav`.
5. The `MappingInfo(root, keyRange, velRange)` is stored with the item.

`getRangesWithRoot(stride, lo, hi, extend)` returns a list of `(root, lo, hi)` [code, `detail::getRangesWithRootFromProperties`]:

```
if stride == 0 or hi < lo or lo > 127 or hi > 127: return []
if lo == hi: return [(hi, extend ? 0 : hi, extend ? 127 : hi)]
s     = min(stride, hi - lo)
start = max(0, lo - (s-1)/2)            # integer division
out = []; root = lo
loop:
    root = min(root, 127)
    up   = min(root + s/2, 127)
    out.push((root, min(start, up), max(start, up)))
    start = up + 1;  root += s
    if up >= hi: break
if extend: out.first.lo = 0; out.last.hi = 127
```

- Each zone is **roughly centred on its root**: `[root − ⌊(s−1)/2⌋, root + ⌊s/2⌋]`, and neighbouring zones touch.
- The last root can lie above `hi`. For example, lo=60, hi=65, stride 3 gives roots 60, 63, 66.
- Example: Key 36…48, stride 12 gives roots 36 (31–42) and 48 (43–54). With Extend they become 0–42 and 43–127.
- Velocity uses the same function. The layer's "root" is the velocity that is actually played.

### 4.2 Running it [code]
- **Start** (`Recorder::startRecording`, Mode = Auto):
  1. Build the items, then set item 0's note and velocity on the `AutoSamplerProcessor` (+0xa8 note, +0xb0 velocity).
  2. Point the recorder's file path at item 0's file.
  3. Open the WAV stream and set the auto-sampler state to 1.
- **`AutoSamplerProcessor::process`**, on the audio thread:
  - **State 1:** sends an `AutoSamplerStartRecorderEvent` to the recorder in the same block (sample-accurate) and goes to state 2.
  - **State 2:** on its first block, emits one MIDI note `emitMidiNoteEvent(note, velocity, …)`:
    - **Internal source:** port 0 and channel `0xFFFFFFFE`, i.e. sent internally to the chosen Sound;
    - **external source:** the selected MIDI To port and Channel.
  - It counts `NoteOn_ms × SR / 1000` frames, then goes idle (state 0). The note-off falls at that point [inferred: the emitter
    schedules the off after the note-on length].
  - **State 3:** cancel, which calls `MidiEventEmitter::stop()` (all notes off).
- **`RecorderProcessor::processWaitingForStartAuto`:**
  - on that event it sets `targetFrames = Sample_ms × SR / 1000` and starts recording **in the same block as the note-on**;
  - `processStartedAuto` stops the take once `targetFrames` have been written.
  - So each file is exactly *Sample* ms long, including the release. There is **no gap or pre-roll setting**.
- **Next item** (`Recorder::onTimer`):
  1. When a take ends, run the post-process job on a worker thread (`Processing...`).
  2. Increment the item index.
  3. If not cancelled, `prepareNextAutoSamplerFile()` loads the next note, velocity and file name, and `openStreamAndStartRecorder()`
     arms the next take.
  - The steps run strictly one after another: one note at a time, one file per note×velocity.
- **Post-processing per file** (`RecorderPostProcessJob::process(path, normalize, trimSilence, findLoop)`), in this order:
  1. `Normalize`: `SampleEditUtils::normalize` over the whole file.
  2. `Trim Silence`: `SampleEditUtils::trimSilence`.
  3. `Find Loop`: `ni::kreator::mir::autoloop::findLoop(path, 0, 1.0, 0)`. If it succeeds, the loop start/end are stored with the item.

### 4.3 Loading the result (`FinishedRecordingHandler::finishAutoSamplerRecording`) [code]
1. **Target sound:** the group/sound focused when recording started (`getSharedTargetGroupAndSound`).
   - If that Sound **has no Sampler** (for example it holds the synth plug-in being sampled), the result goes to the
     **next available Sound** in the group (`GroupAlgorithms::nextMatchingSound(…, isSoundAvailableForRecording)`), or to
     the group's **last** Sound if none is available.
   - If it already has a Sampler, zones are added to that Sampler.
2. Call `Sound::getOrCreateSampler`. On a freshly created Sampler, two parameters are initialised (one set to 2, one to 0)
   [UNCONFIRMED which: plausibly voice/playback mode].
3. For each item:
   - `Sampler::loadAndSelectZones(file)`;
   - `MappingInfo::apply` sets the zone's root key, key range and velocity range;
   - if Find Loop found a loop, turn on the zone's loop and set the loop range to `(min(start,end), max(start,end))`.
   - Existing zones are **not cleared** [inferred from the absence of `zones.clear()`, unlike the single-take path].
4. Focus that Sound if its group is still focused. The UI then switches to the **Zone** tab.

The Sound is not renamed in this path. Zone names come from the file names (`<stem> C3 100`).

---

## 5. Detect, Sync and Loop timing (`RecorderProcessor`) [code]

- **Detect** (`processWaitingForStartDetect`):
  - Compares each channel's input peak with the threshold, converted to linear: `exp(0.115129255·dB)` = 10^(dB/20).
  - Starts on the first block where either channel exceeds it.
  - If the threshold is ≤ the level of −59.99 dB (`OFF`), it starts **immediately** on Start, i.e. a manual start.
  - While waiting, it keeps a **10 ms pre-roll** ring buffer (`SR·10/1000` frames), so the attack before the trigger is kept.
  - Stop ends the take at once.
- **Sync** (`processWaitingForStartSync` / `processStartedSync` / `processWaitingForStopSync`):
  - Arms only while the transport is **playing**. Recording starts at the next grid line.
  - The grid is **one bar** (`TimeSignatureNumerator × 960 ticks`).
  - Recording lasts `Length` bars (`1<<i`), or forever for FREE.
  - If the transport stops, recording stops.
  - Pressing Stop moves to "waiting for stop", which ends the take on the **next bar line** (frame-accurate).
  - The recorded bar count is tracked in `RecorderProcessor+0x198`.
- **Loop:** the same engine as Sync. The grid is `RecorderAlgorithms::computeRecordingGrid`:
  - in the Ideas view with Target ≠ Pattern: the **focused pattern's length**;
  - in the Ideas view with Target = Pattern: the input group's focused pattern length, used only when Source = Internal and the
    input is a group; otherwise 1 bar;
  - in Song or Arranger view: **1 bar**.
  - So recording starts at the next pattern boundary ("Waiting for end of Pattern...") and lasts `Length`.
  - One arm produces **one take**. Nothing re-arms automatically per pass; the code re-arms only for Auto.
    A layered "looper" workflow means pressing Start again, which the footswitch makes easy.
- The tempo is stored in the WAV for Sync and Loop: `loadRecordingToSound` writes an NI `SoundInfo2` chunk
  (`taghelper::updateNiSoundInfoInFile`).
  - The tempo comes from `getRecordingTempo` (the song tempo over the recorded range).
  - The device flag is 8 for Loop and 4 otherwise.

---

## 6. Files [code]

- **Folder** (`createOutputFilePath`):
  - If Preferences ▸ General **Prefer Project Folder** is on, the project has been saved, and the project is not under the
    factory library: `<project dir>/<Project Display Name> <suffix>`. The suffix is a global wide string, probably
    `Recordings` [inferred; not read].
  - Otherwise: the user-paths **Recordings** directory (Standard User Directory/`Recordings`).
  - The folder is created if missing.
- **Name:** `{}.wav`, made unique.
  - Normally `{}` is `boost::posix_time::to_iso_string(now)`, i.e. **`YYYYMMDDTHHMMSS`** with a 4-digit year
    (e.g. `20261014T153000.wav`). The manual's `[YYMMDD]` is wrong.
  - Auto + Internal: `{}` is the **name of the sampled Sound** (made filename-safe). Auto files are then `<SoundName> C3 100.wav`, etc.
  - [UNCONFIRMED] There is a second branch: when the focused Sound's first module slot is empty, its name is used instead of
    the timestamp. The decompile shows this condition, but which slot list it checks was not identified.
- **Format:** `AudioEncoderFileStream(path, channels, engineSampleRate, encoding 0)`.
  - Channels: **1 for Ext. Mono**, otherwise 2.
  - [UNCONFIRMED] what `encoding 0` is (probably the default PCM width). Use 24-bit PCM or 32-bit float.

---

## 7. Where takes go, and Target (`FinishedRecordingHandler`, `RecorderAlgorithms::loadRecordingToSound`) [code]

`finishSingleRecording` (every mode except Auto):

**1. Pick the destination Sound.**
- Start from the group/sound focused at Start.
- In **Loop with Target Sound or Pattern**: the **next available empty Sound** after it in the group, or the group's
  **last** Sound when none is available.

**2. Load the file into it** with `loadRecordingToSound(sound, file, patternGetter, tempo)`:

- **Loop mode with a tempo, and the Sound has no Sampler** (or the Sound already has an Audio module):
  1. `getOrCreateAudioModule`, then `TakeAlgorithms::addSampleToTakes(file, tempo)`.
  2. If a pattern is returned (Target Pattern: `getOrCreatePatternForRecording` makes or reuses the pattern in the slot
     chosen by `setTarget`, assigns it to the focused scene (creating a scene if needed) and focuses it; otherwise the
     current pattern), call `setAudioLoopEnabled(pattern, sound, true)`.
  3. If the pattern is shorter than the recording (`recordedBars × ticksPerBar`): **double** it ⌊log2(rec/len)⌋ times when
     possible, then `setAutoLength` it to the recording's length.
- **Otherwise (Detect, Sync, or a Sound that has a Sampler):**
  1. `getOrCreateSampler`, then append a new **Take** (the sampler's `takes`, i.e. the Audio Pool) with the file.
  2. If the Sampler is new, initialise its parameter at +0xe8 to 2.
  3. **Clear all zones**, then create **one zone** loaded with the file (full key and velocity range).
- In both cases, `SoundAlgorithms::updateNameIfNotCustomized` renames the Sound after the sample.

**3. Focus the destination Sound.**

**Play state during Loop recording:**
- **Target Take** (`storeTakePlayState` / `muteTake`): the Sound's audio loop in the focused pattern is switched off while the
  new take records.
- **Target Pattern** (`storePatternPlayState` / `mutePattern`): the scene's pattern reference for the group is cleared,
  muting the old pattern.
- **Cancel:** `restoreTakePlayState` / `restorePatternPlayState` put things back.
- [inferred] So the old loop does not play over the new recording.

**Summary**

| Target | destination Sound | plug-in | pattern |
|---|---|---|---|
| Take | the focused Sound | Audio (or its existing Sampler) | the current pattern plays the new take (audio loop on) |
| Sound | next empty Sound in the group (or the last Sound) | Audio | the current pattern; the new Sound's loop is enabled, so it layers on the earlier ones |
| Pattern | next empty Sound in the group (or the last Sound) | Audio | a new or next pattern slot is assigned to the scene, with only this recording enabled |

The cancel and undo paths are `FinishedRecordingHandler::cancelRecording` and `RecorderAccess::undoPreviousRecordingTake`.

---

## 8. Monitor and metering [code]

- `Monitor` → `RecorderProcessor::setDirectMonitoring(bool)`: the selected external input is sent to the **Cue** bus.
  It is offered only for external sources.
- The meter shows `Recorder::getLevelGUIReadout()` (stereo peak). There are clip LEDs per channel.
  The Lua controller's meter uses a 0.5 s peak hold.

---

## 9. Preferences ▸ Audio ▸ Routing (where external inputs come from)

- `NI::MASCOMMON::GUI::DialogPreferencesAudio` has a `MultiTab#m_pRoutingTabs` with an **Input** and an **Output** tab
  (`Bar#pRoutingInputBar` / `Bar#pRoutingOutputBar`; style in
  `maschine_nwl_resources/Maschine/Dialogs/Preferences.txt`). Each tab is a list of rows (`onRowCreate`: style
  `OptionsListItem`, rows alternate "odd"/"even"). [code]
- **Row count:** `NSAAccess::getInternalChannelsCount`, i.e. Maschine's internal channels: 16 stereo inputs = 32 mono rows,
  matching the recorder's `In 1 L` … `In 16 R`. [code + inferred]
- **Row label:** `NSAAccess::getInternalChannelString(row, isInput)`. The text comes from a virtual on the audio interface
  (`InterfaceNSA` vtable +0x140/+0x148) and was not decoded. [UNCONFIRMED] It is presumably the Maschine channel name, e.g.
  `In 1 L`.
- **Row value:** a `ButtonMenu` listing the audio device's physical channels.
  - Each entry is `NSAAccess::getPhysicalChannelString`, formatted `"<n>: <device channel name>"`, where the name comes from
    CoreAudio via `LockedAudioDevice::getInputChannelName(n-1)`.
  - Entry 0 is `- not connected -`.
  - Choosing an entry calls `NSAAccess::setMappedChannel(row, n)`, and `getMappedChannel` shows the current choice. [code]
- So for DECKARD:
  - Prefs ▸ Audio ▸ Routing ▸ Inputs maps each of 32 mono Maschine input channels (16 L/R pairs) to a device input channel
    or "not connected".
  - The Record page's `In k L+R` reads Maschine pair k, i.e. channels 2k−2 and 2k−1. `In k L` and `In k R` read one channel.
  - [UNCONFIRMED] Defaults: the first device channels mapped 1:1 in order.

### DECKARD's routing (Preferences ▸ Audio)

- An **Output device** and an **Input device**, then **Routing** with Input and Output tabs: one row per mono channel
  (`In 1 L` … `In 16 R`, `Out 1 L` … `Out 16 R`) with three columns: DECKARD's name, an **Alias**, and the interface
  channel (or **Not connected**).
- Defaults: In 1 and Out 1 on the interface's channels 1–2; everything else not connected.
- Not connected means off: the channel isn't offered in the sound and group pickers (Audio input Source, Dest., Aux,
  the master's and cue's Output), on the Sampling page's Ext. Ster. / Ext. Mono list, or on the controller (whose
  channel pages come from the same lists). One already picked stays offered, so a setting isn't lost silently.
- An alias names the channel wherever it's offered. A stereo pair (`In 1 L+R`, `Ext. 1`) shows its alias when both
  sides share it, else both channels' names.
- Each device keeps its own routing and aliases (by name, for inputs and outputs apart): picking another device brings
  its own back; DECKARD opens the output device last picked (the system's default while it isn't connected).
- The engine renders each external output (`Engine::process_outputs`): what's routed to the master's output plays as
  before, and Ext. n elsewhere fills Out n; the audio callback puts each output channel on its interface channel.
