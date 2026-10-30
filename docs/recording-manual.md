# Maschine 3 Sampling > Record page: spec from the NI manual

Sources, fetched 2026-10-14:
- Maschine Software manual, "Sampling and sample mapping":
  https://docs.native-instruments.com/ni-tech-manuals/maschine-software-manual/en/sampling-and-sample-mapping
- Maschine Software manual, "Basic concepts > Preferences – Audio page":
  https://docs.native-instruments.com/ni-tech-manuals/maschine-software-manual/en/basic-concepts
- Maschine Software manual, "Audio routing, remote control, and Macro Controls":
  https://docs.native-instruments.com/ni-tech-manuals/maschine-software-manual/en/audio-routing,-remote-control,-and-macro-controls
- Hardware manuals, same chapter: Maschine MK3 (`online-guides/maschine-mk3-manual/en/sampling-and-sample-mapping`)
  and Maschine+ (`online-guides/maschine-plus-manual/en/sampling-and-sample-mapping`)
- NI's controller Lua, extracted to `out/ni-res/maschine_nwl_resources/Scripts/Maschine/`
  (`Components/Pages/SamplingPageBase.lua`, `Jam/ParameterHandlerJam.lua`, `Helper/SamplingHelper.lua`)

Quoted text is copied from the manual. Items marked **[UNCONFIRMED]** are not in the manual.

---

## 0. Page structure

- Sample Editor tabs: **Record**, **Edit**, **Slice**, **Zone**. (The controller uses the same names, `RECORD/EDIT/SLICE/ZONE`, on Buttons 1–4.)
- "By default, the Sample Editor is on the blank Record page and no other tabs are visible. However, the Record tab, plus additional tabs, will become visible once audio has been recorded."
- "At the bottom of the Record page, the **INPUT** section and the **RECORDING** section allow you to select which source should be recorded and how the recording should be made."
- When SOURCE is external, a **MONITOR** section appears on the right.
- In Auto mode there are also **NOTE MAP**, **VELOCITY MAP** and **BATCH PROCESS** sections (see §4).
- Horizontal **input level meters** sit above the RECORDING section.
- Above the bottom sections: an information bar, a waveform display, a zoom scroll bar, a timeline, and the **Audio Pool** (takes shown as mini waveforms).

## 1. Record source (INPUT section)

### SOURCE (exact UI labels)
| Value | Meaning |
|---|---|
| `Ext. Ster.` | "To record external audio signals connected to your audio interface … (for stereo signals)" |
| `Ext. Mono` | "… (for mono signals)" |
| `Internal` | "To record audio signals coming from Maschine itself" |

The manual lists no "Bus" source. The SOURCE label is written "Ext. Ster." in the UI. The Lua enums are `NI.DATA.SOURCE_EXTERNAL_STEREO`, `SOURCE_EXTERNAL_MONO` and `SOURCE_INTERNAL`.

### INPUT (depends on SOURCE)
- **Ext. Ster.**: "Selects either of Maschine's four external stereo inputs **In 1–4**."
- **Ext. Mono**: "Selects either of Maschine's eight external mono inputs: the left ("L") or right ("R") channel of each input pair In 1–4." This gives 8 entries: In 1 L, In 1 R, … In 4 R. **[UNCONFIRMED]** the exact label format (e.g. "In 1 L"). The Lua code reads `ExtMonoInputs` as an index where `floor(v/2)` is the stereo pair and `v % 2 == 0` is the left channel. Elsewhere the routing chapter writes labels as "In 1 L+R".
- **Internal**: "Selects the output of any available **Group** or the **Master**." Auto Sampler adds: "Selects the output of any available Group or the Master channel."
  - Individual Sounds are **not** in the list. The manual's tip: "You can easily record the output of a particular Sound by setting SOURCE to Internal, soloing this Sound, and selecting its parent Group in the INPUT selector."
  - The Auto Sampler section contradicts this: "select Input Source to Internal and the Input to **Group and Sound** of the plug-in you want the Auto Sampler to capture." **[UNCONFIRMED]** whether Auto mode's Internal INPUT also lists Sounds. The general INPUT table says Groups or Master only.
  - Tip: record live pad playing by setting Internal to a Group with a drum kit loaded.
- Lua parameter objects: `getRecordingSourceParameter`, `getExtStereoInputsParameter`, `getExtMonoInputsParameter`, `getInternalInputsParameter`. Each SOURCE has its own INPUT parameter, so the selection for one source is kept when you switch to another.

### Where external inputs are configured
- **Preferences > Audio > Routings > Input**: "Here you can select which inputs on your audio interface should be used for each of the 16 stereo inputs of Maschine … The choices made here will determine which inputs can be used when sampling external sources."
- Output routing in Preferences lists "the 16 stereo outputs of Maschine". Master and Cue destinations are labelled "Ext. 1–16".
- **The manual is inconsistent about input counts.** Preferences says 16 stereo inputs. Sound input routing says "In 1 L+R – In 16 L+R … 32 … mono" and also "In 1–8". The Record page says 4 stereo inputs (In 1–4) and 8 mono. The Record page figure (In 1–4) is what the recorder exposes. **[UNCONFIRMED]** whether Maschine 3 desktop now exposes more than 4 on the Record page.

## 2. Recording mode (RECORDING section)

### MODE
"Click the MODE selector in the RECORDING section to select from the four available recording modes": **Detect**, **Sync**, **Loop**, **Auto**. The Maschine+ manual says "three", which is an outdated count. The Lua enums are `NI.DATA.MODE_DETECT`, `MODE_SYNC`, `MODE_LOOP` and `MODE_AUTO`.

The extra controls each mode shows, confirmed in `ParameterHandlerJam.lua`:
| Mode | Params after MODE/SOURCE/INPUT |
|---|---|
| Detect | `THRESHOLD` (`getDetectThresholdParameter`) |
| Sync | `LENGTH` (`getSyncLengthParameter`) |
| Loop | `LENGTH` + `TARGET` (`getLoopTargetParameter`) |
| (external source) | `MONITOR` (`getDirectMonitoringParameter`) |
| Auto | SAMPLE, NOTE ON, MIDI TO, CHANNEL, Note Map, Velocity Map, Batch Process (§4) |

### Detect
- "record audio after the set threshold has been exceeded … without the silence before the incoming sound is played."
- **THRESHOLD**: "After clicking Start, any input signal level exceeding this threshold will start the recording. You can then manually stop the recording by clicking Stop."
  - A slider on the horizontal input level meters is "a strict equivalent to the THRESHOLD control". It is shown only in Detect mode.
  - "Double-click the slider to reset the threshold to its default value (**-12 dB**)."
  - The threshold can be turned down to **OFF**. Manual-record tip: "set the MODE to Detect, dial the THRESHOLD down to OFF, and start the recording by clicking Start. To stop recording, click Stop."
  - **[UNCONFIRMED]** the threshold's dB range. In Lua it is a dB parameter whose maximum is about 0. The meter maps it as `1 - value/(max-min) * -1`.
- Plug-in: "If the focused Sound Slot contains neither an Audio nor Sampler plug-in, the recorded audio will automatically be loaded into a Sampler plug-in for the first Take."
- Timeline units: **seconds** in Detect mode.

### Sync
- "start recording audio in time with the Pattern Grid … The recording will begin at the start of the **next bar**."
- **LENGTH**: "either **1, 2, 4, 8, or 16 bars**, or choose **Free** if you do not want to set a duration limit to the recording."
- "You can click Stop to end recording at any time, and the recording will stop **at the next bar**."
- Plug-in behaviour is the same as Detect: a Sampler is loaded if the slot has no Audio or Sampler plug-in.
- Timeline units: **bars** in Sync mode.

### Loop
- "record audio that you intend to play back in sync with your Project using the **Audio plug-in**. It records audio in the same way as Sync." It shows the same LENGTH control (1/2/4/8/16/Free).
- Start: "The recording will start at the **beginning of the Pattern**." While waiting, the status reads "Waiting for end of Pattern…".
- Stop: "Stop beforehand to stop the recording at the next bar".
- "the recording is loaded into an Audio plug-in so that you can immediately hear the results."
- Exception: "If there was already a Sample[r] Plug-in in the Slot when engaging Loop recording, it will not change the Sampler to an Audio plug-in … all recorded takes will be preserved."
- "If the Audio Plug-in was used in Loop mode, the last recorded Take will automatically play back with the Pattern."
- The manual does not describe continuous multi-pass loop recording. Each Start…Stop or Start…LENGTH cycle produces one take. **[UNCONFIRMED]** whether recording re-arms automatically after LENGTH completes. The manual says nothing about it, so treat it as one take per arm.
- **TARGET** (Loop mode only; see §3).

### Auto
"automatically sample an external or internal synthesizer … options … for setting the notes' stride (interval), key range, and velocity … batch process the samples to automatically find the best loop points, trim silence …, and normalize." See §4.

## 3. TARGET (Loop mode only)

Exact values: **Take**, **Sound**, **Pattern**.

- **Take**: "record each new Take into the focused Sound slot. After a Take is recorded, it will become visible in the Audio Pool and is automatically selected for playback. You can review all the Takes … and … select any of these Takes to make it the one that actively plays back in the Audio plug-in."
- **Sound**: "a layered workflow similar to … guitar loop pedals … each recording … will be loaded into an **empty Sound Slot in the current Group and Pattern** and will begin playing back … layered on top of all the previous recordings … You can continue this process until all empty Sound slots in the Group are used up. After that, each additional recording will be recorded as a new Take into the **last Sound of the Group that contains an Audio plug-in** (and the previous Takes will still be preserved …)."
- **Pattern**: "similar to the Sound Target … except that each new recording will also be assigned to a **new Pattern** … the first recording will be placed into an Audio plug-in in the first available Sound Slot in the Group, and then a new Pattern will be created where only this new recording is playing back. Making another recording will result in the recording being loaded into another unused Sound Slot, and another new Pattern will be made where only this most recent recording is playing. If you switch back to the previous Pattern, you'll hear only the previous recording (the newest will automatically be deactivated in the previous Pattern)."
- Related: "In the Audio Plug-in, changing the selected take only affects the current Pattern. The other Patterns remain unchanged." This means the selected take is stored per Pattern.

## 4. Auto Sampler (MODE = Auto)

### Purpose and requirements
- "create sampler instruments … from a MIDI-capable hardware synthesizer, a software instrument, or a combination of synthesizers, hardware effects, and effect plug-ins." The result "can be saved as a Sound and all its samples."
- Caveat: "effects are applied to each sampled note, not on multiple notes or chords; and … effects parameters cannot be edited or automated".
- External instrument: "connect the instrument to your Maschine (audio and MIDI), then select Input Source to External … Ensure that Maschine can receive audio input from the external device on the correct audio input."
- Internal instrument (plug-in): "select Input Source to Internal and the Input to Group and Sound of the plug-in you want the Auto Sampler to capture."
- From Lua: **Start is disabled unless a valid MIDI output is selected** (`NI.DATA.RecorderAlgorithms.hasValidMidiOutputSelected(Recorder)`). A MIDI TO target is therefore required, including for internal plug-ins. **[UNCONFIRMED]** what MIDI TO lists for internal plug-ins (presumably the Group/Sound).

### Sections and parameters (manual names)
**RECORDING**
| Param | Description |
|---|---|
| MODE | "Select Auto to use Auto-Sampler." |
| SAMPLE | "length of time Auto-Sampler records samples … short times for percussive sounds or bass stabs, and longer times for strings and pads … record for longer to give Maschine enough material to find the best loop points. … use … in conjunction with the NOTE ON parameter." |
| NOTE ON | "length of time Auto-Sampler triggers the instrument it is recording" (note-on duration before note-off). The remainder, SAMPLE − NOTE ON, is effectively the release tail. |

**INPUT**
| Param | Description |
|---|---|
| SOURCE | Ext. Ster. / Ext. Mono / Internal (as in §1) |
| INPUT | as in §1 |
| MIDI TO | "the MIDI port you want to use when connecting to external MIDI equipment. All available devices are shown here." |
| CHANNEL | "the MIDI channel to match the MIDI channel of the external equipment … Maschine will use this to trigger the sound". **[UNCONFIRMED]** the range; presumably 1–16. |

**NOTE MAP**
| Param | Description |
|---|---|
| STRIDE | "the number of notes that each sample will cover. A Stride value of 1 records every note within the Key Lo and Key Hi range … A value of 12 … would cover 12 notes, meaning one sample would be stretched across a range of notes." |
| KEY LO | "lowest note of the sample note range" |
| KEY HI | "highest note of the sample note range" |
| EXTEND | "Extends the highest and lowest samples to include the whole range of the keyboard." |

**VELOCITY MAP**
| Param | Description |
|---|---|
| STRIDE | "the velocity range each sample will cover … Stride value of 1 to record every velocity … within the Vel Lo and Vel Hi range … higher Stride value, resulting in fewer samples" |
| VEL LO | "lowest velocity value you want to sample" |
| VEL HI | "highest velocity you want to sample" |
| EXTEND | "Extends the upper and lower velocity values to include the full velocity range of the instrument." |

**BATCH PROCESS** (toggles)
| Param | Description |
|---|---|
| FIND LOOP | "Automatically finds the best loop point in your samples" (for pads and strings) |
| TRIM SILENCE | "removes unwanted silence from the beginning and end of each sample" |
| NORMALIZE | "Adjusts the level of each sample to the maximum possible value without clipping" |

**MONITOR**
| Param | Description |
|---|---|
| MONITOR | "enables you to hear the Auto-Sampler as it works through the process of capturing samples." |

### Not documented [UNCONFIRMED]
- Default values and units for SAMPLE and NOTE ON (seconds or ms?).
- Whether the gap or silence between notes is configurable. No such parameter is listed.
- Zone layout. One zone per sampled note and velocity layer is implied. Probably each zone's root is the sampled note and it spans `[note, note+stride-1]`. **The manual does not say** whether the range goes upward or is centred, how velocity zones split, or how samples or the Sound are named.
- Whether the result replaces existing zones in the focused Sound's Sampler.
- The order of playback (note-major or velocity-major).

## 5. Other controls and behaviour

### Level meters and threshold
"The level meters above the RECORDING section show you at any time the level of the selected audio source." In Detect mode they include the threshold fader. The Lua code feeds the meter from `Recorder:getLevelGUIReadout()` (stereo levels).

### MONITOR (non-Auto modes)
Shown only for external sources (Ext. Ster. or Ext. Mono): "activate the MONITOR button to send the input signal to the **Cue bus** of Maschine, allowing you to hear on a separate channel (for example, your headphones) the audio source". The Cue output destination is set in Master Output properties (Ext. 1–16). On Maschine+ it is on page 3, Knob 1.

### Start / Waiting / Stop / Cancel
- "Click **Start** to arm the recording."
- Detect: before the threshold is crossed, "the Start button turns to a **Waiting** button and the **Waiting for input…** message appears in the information bar above the waveform display." Clicking Waiting starts recording manually. **Cancel** cancels. While recording, **Stop** "stops immediately", and Cancel discards ("the recorded audio will not be saved").
- Sync: a flashing **Waiting** label with "**Waiting for the next bar…**". It records for LENGTH. Stop stops at the next bar. Cancel discards.
- Loop: a flashing **Waiting** label with "**Waiting for end of Pattern…**". It starts at the Pattern start. It records for LENGTH. Stop stops at the next bar. Cancel discards.
- Controller button labels (Lua, `updateRecordPageScreenButtons`): idle `START` / `DELETE` (with Shift: `TAKES` / `RENAME`). Waiting `WAITING` / `CANCEL`. Recording `STOP` / `CANCEL`. PREV/NEXT are hidden while recording. Parameter page 1 is disabled while waiting or recording (`isDisabledWhenRecording`).
- Recorder states in Lua: STOPPED=0, WAITING=1, RECORDING=2.
- "the recorded audio will be stored in the Sound that was under focus as you started the recording."
- **Count-in / Prelisten**: the Record page documentation does **not** mention either. There is no count-in parameter. The closest equivalents are the Waiting phase and MONITOR.

### Footswitch (hardware; Cancel / Abort / Undo semantics)
- A press arms. Once the sync point is reached, recording starts. It stops after LENGTH, or with a second press (stops at the next bar, especially with Free).
- **Cancel**: press again while waiting. "No new recordings will be made, nor new Patterns."
- **Abort**: double-press while recording. "Any audio recorded will be discarded, no loops will be loaded, and no new Pattern created."
- **Undo**: "If a recording finished before you could cancel it, then simply press and hold the footswitch to trigger Undo." This implies a completed recording is a single undoable action, including any Sound or Pattern created for the Target.

### When recording completes
1. The take is saved to disk as `[YYMMDD]T[HHMMSS].wav`, in the `Recordings` subfolder of the Standard User Directory, or of the Project folder if "Prefer Project Folder" is on.
2. Its waveform is shown, and the name appears in the information bar.
3. The take is "appended to the Audio Pool of the Sound and selected".
4. "A Sampler Plug-in is automatically loaded in the first Plug-in slot of the Sound … **All Plug-ins previously loaded in that Sound are removed.**" In Loop mode an Audio plug-in is loaded instead, unless a Sampler already exists (see above).
5. "The Sound slot takes the name of your recording."
6. Detect and Sync with a Sampler: "mapped to a new Zone covering the entire key and velocity ranges … **Any existing Zones will be replaced.**"
7. Loop with Audio: the last take plays back with the Pattern.
8. "any MIDI Events for that Sound in the current Pattern will remain", so the sample may play at existing note pitches.

### Waveform display and Audio Pool (takes)
- (1) The waveform shows the take selected in the Audio Pool. The mouse wheel zooms. During playback a white vertical playhead is shown. The right-click menu has: **Open containing folder**, **Save Sample As…**.
- (2) Zoom scroll bar: drag horizontally to scroll and vertically to zoom. Drag the handles to zoom with one edge fixed. Double-click resets.
- (3) Timeline: bars in Sync, seconds in Detect.
- (4) Information bar: file name and length. Click and hold the play icon to audition on the Cue bus. The circle icon at the far right toggles single-channel and two-channel display.
- (5) **Audio Pool**: "All the takes you have made since you opened the current Project … displayed as mini waveforms under the waveform display."
  - Click a take to select it. This also maps it "to a new Zone covering the entire key and velocity ranges", replacing existing Zones (Sampler). With the Audio plug-in it selects the playing take for the current Pattern only.
  - Click and hold the play icon in the bottom-right corner to audition on Cue without loading.
  - The × in the top-right corner deletes the take.
  - Drag a mini waveform to another Sound slot to load it there.
  - Context menu: **Delete**, **Remove unused recordings** ("all takes that are not currently mapped to any Zone"), **Map recordings to zones** ("Zones are put on adjacent keys and cover the entire velocity range. Any existing Zones will be replaced.").
  - "All recordings (takes) in the Audio Pool are saved with the Project."
- Controller: Knob 5 zooms and Knob 6 scrolls. Button 7/8 = PREV/NEXT take. Button 6 = DELETE. Takes can be shown on pads in banks of 16 ("BANK n"). Pressing the focused pad auditions on Cue.
- On recording, the waveform display shows the take being recorded. The Lua invalidates `WaveEditorRecord` and updates the header (`updateWaveEditorRecordHeader`). **[UNCONFIRMED]** the exact live-drawing behaviour.

## 6. Hardware parameter layout (for naming and order)

- MK3: SAMPLING > Button 1 = RECORD page. Button 5 = START/STOP/WAITING. Button 6 = DELETE/CANCEL. Buttons 7/8 = PREV/NEXT.
- Maschine+: page 1 Knob 1 SOURCE, Knob 2 INPUT. Page 2 Knob 1 MODE, then mode params (THRESHOLD / LENGTH / TARGET). Page 3 Knob 1 MONITOR (external only).
- Jam parameter order (Lua): MODE, SOURCE, INPUT (per source), then THRESHOLD (Detect) | LENGTH (Sync) | LENGTH, TARGET (Loop), then MONITOR.
