# Maschine 3's Sampling view (Sample Editor): layout, data and edits

An implementation spec for the Sampling view: tabs **Record**, **Edit**, **Slice** and
**Zone**, shown in the lower area in place of the pattern editor.

**Sources**
- Style sheets: `out/ni-res/maschine_nwl_resources/Maschine/Panels/PatternEditorShared.txt`
  (its first ~1000 lines are the "SAMPLING PAGES" section), `Custom/WaveDisplays.txt`,
  `Panels/ZoneEditor.txt` and `Definitions/Colors.txt`.
- Bitmaps in `maschine_pictures_nwl_resources/Maschine/SamplingPages/`. They are measured
  only: we draw our own shapes and never ship NI bitmaps.
- Decompiled code in `out/decomp/samp` and `out/decomp/samp2`. The `*.s.c` copies carry
  the wide-string labels inline; they were made with the scratchpad script
  `samp/strings.py`.
- The NI manual, "Sampling and Sample Mapping".

Colours and fonts use `docs/ui-layout.md`'s conventions:
- **Surfaces:** Surface00 #181818, Surface05 #212121, Surface01 #262626, Surface02 #323232,
  Surface03 #3C3C3C.
- **Text** (#FFFFFF): high a245, medium a164, low a142, sub a61.
- **Palette 1** = the group/sound colour.
- **Font:** Roboto Medium.

**Confidence**
- **[exact]** = read from a style sheet, code, schema or the manual.
- **[inferred]** = a reasonable reading.

---

## 1. Where it lives and the frame [exact unless marked]

**Opening it.** The manual: "the waveform-icon button left of the Pattern Editor", from
the Sound list.
- [inferred] Our version: a third strip button under the drum-grid/piano-roll pair in the
  group panel. It swaps the pattern editor area (`m_pLowerArea`) for the Sampling panel.
- The sound list stays on the left.

**Panel `Bar#m_pSamplingPanel`.** Background #000000, children 1 px apart, so black 1 px
lines separate the rows. Top to bottom:

| row | id | height | contents |
|---|---|---|---|
| 1 | header `m_headerBar` | **37**, Surface00 | the tab bar `m_tabsBar` (margin-left 4, top 2, spacing 10, bottom divider line) with tabs **RECORD EDIT SLICE ZONE**; at the right, for the Zone tab, the `m_pZoneTitleBar` buttons (spacing 10): **Zone List** 18×16 (`sampling_pg_button_zone_list`, 7 states) and **Sample View** 18×16 (`sampling_pg_button_zone_wave`) |
| 2 | info bar `m_pInfoBar` | **16** (margins left 4, right 10) | **play** button 16×16 (`sampling_pg_button_play_sample`, palette-1 fill, outer-margin-right 6): hold it to audition the sample on Cue. Then the **sample name** (font 12, palette-1 text, left). Then the **duration** (font 12, text low, outer-margin-right 8). Far right: the **stereo/mono display toggle** 16×12 (`sampling_pg_button_toggle_view`, 7 states, outer-margin-top 3) |
| 3 | panel stack `m_pPanelStack` | rest | one of `RecordingPanel`, `SampleEditorPanel`, `SlicingPanel`, `ZoneEditorPanel` (each #000000, spacing 1) |

**Tabs `Button.SamplingTab`**
- A text tab (font 12) with `sampling_pg_button_tab` (30×15 per state, 7 states).
  Margins left/right 8, min-width 10.
- Text: low (a142) normally, medium (a164) on hover, high (a245) when selected or
  pressed.
- The selected tab has the bitmap's underline. [inferred] Draw a 2 px white line under it,
  as our other tabs.
- Only **RECORD** shows until the sound has a sample (manual). With a sample, all four
  show.

**Shared rows inside each tab panel** [exact]
- **Timeline `WaveDisplayTimeline#m_pTimeline`**
  - Surface00, min-height 10, font 12, text low, left-aligned, outer-margin-bottom 1.
  - Ticks: text markers in grid medium (#FFFFFF a92); other markers in content sub (a61).
  - Tick heights: small 4, medium 8, large 16. Text offset left 6, top 1.
  - Units: seconds, or bars in Record ▸ Sync (manual).
- **Wave display:** see §2.
- **Zoom/scroll bar `pWaveDisplayBar m_pFoot`** (`SHARED_BG_Scrollbar`, fixed caps 10,
  margin-bottom 2). Bitmaps `sampling_pg_bg_zoom_slider` 211×19 and zone variant 211×20.
  Manual behaviour:
  - drag the body to scroll;
  - drag a handle to zoom with the other edge fixed;
  - double-click to show the whole sample;
  - the mouse wheel over the waveform zooms.
- **Parameter footer `SamplingParameterEditor`**
  - Background `SampTab_BG_FooterToolbar` (50×58, caps 10), spacing 5. With a horizontal
    scrollbar: `…Scroll` (50×75), scrollbar 16 high.
  - Parameters come in **groups** (`Bar.SamplingParameterPageOuterBar`, bitmap
    `SampTab_BG_FooterParameterGroup` 50×58, min-width 68, margin-right 1).
  - Each group has a **section name** label (22 high, margin-left 5, text low) and its
    controls.
  - Each control has a **name sub-headline** (`Button.TransparentLeft`, font 10,
    right-aligned, 12 high, text low) above its value.
  - [inferred] Draw each group as a #262626 box with its title, then label/value pairs.
    Values are drag-numbers or selectors, like the chain area's.

The footer's groups for each tab are listed under that tab below. They are the
`SamplingParameters::update{Record,Edit,Slice,Zone}Parameters` pages.

---

## 2. Waveform display [exact unless marked]

### Edit and Zone ▸ Sample view: `MarkerWaveDisplay`

**Colours**
- Background Surface00 #181818, both inside and outside the play range.
- **Selection** background Surface02 #323232, with an overlay frame
  (`sampling_pg_selection_overlay`, 50×20, 1 px caps).
- **Waveform** colour depends on where it is:
  - outside the play range: Surface03 #3C3C3C;
  - inside the play range: **palette 1** (the sound colour);
  - in the selection: palette 1.
- **dB lines:** rgb(30,30,30). No channel separator.
- **Play cursor:** rgba(255,255,255,198), a 1 px vertical line.
- **Looped section:** palette 22 (the colour mixed 75% into #181818), from
  `Colors.txt` ("used for looped waveform visualisation").

**Play-range markers S and E**
- 3 px wide, white #FFFFFF, with a Surface00 contrast edge.
- Each has a 16×16 flag at the top: `intmod_wave_start_marker` ("S", pointing right) and
  `intmod_wave_end_marker` ("E", pointing left).
- Drag the line or the flag.

**Loop**
- The loop is drawn as a white frame over the loop range: `sampler_loop_frame`, 255×16,
  caps 24/5/24/24. It looks like a title bar (a tab) along the top, plus edges down to the
  bottom.
- Drag a border to resize the loop; drag the title bar to move it. The loop stays inside
  the play range (manual).
- [inferred] Put "LOOP" text in the title bar.

**Stereo**
- The info bar's toggle switches between **two-channel** (L above R, each half height,
  with a 1 px gap) and **single** (L+R mix) display (manual).
- Each channel is drawn symmetric about its own centre line, as min/max per pixel column.

**Recording tab (`WaveDisplayWidget#m_pWaveDisplay`):** Surface00 background, play cursor
rgba(255,255,255,198), waveform palette 1.

### Slice tab: `SlicingWaveDisplay`

- **Outside any slice:** background Surface00, waveform Surface03.
- **Active slices:** waveform palette 1.
- **Hovered or focused slice:** background Surface02.
- **Play cursor:** white.
- **Slice markers:** 1 px lines in content medium (#FFFFFF a164), grab area ±4 px.
- **Focused slice:** start and end markers 3 px white, with Surface00 contrast edges
  (hover: content low). They use the same S/E flag bitmaps.
- **Number labels** [inferred]: draw slice numbers 1… along the top in text low.

### Playhead [exact]

`Sampler::getPlayPositionFocusZone()` returns the processor's field `+0x270`, the
playback frame of the voice playing the **focused zone**. 0 means nothing is playing.

[inferred] For our engine:
- Report each voice's position in sample frames, per (group, sound, zone).
- Take the newest voice of the focused zone.
- Draw the cursor at that frame. Refresh it every UI frame while playing.

---

## 3. Record tab (`RecordingPanel`)

### Layout [exact]

**History-and-command bar `m_pWaveHistoryAndCommandBars`**
- Frame bitmap `sampling_pg_bg_recording_history` (90×70, caps 30/30/15/15). Margins
  left/right 15, spacing 15. Left to right:
- **Threshold bar** `m_pThresholdBar`
  - Scale image `SampTab_MTR_BG_Scale` 111×39, outer-margin-top 12.
  - It holds a **stereo input meter** `MasterLevelMeter` 103×22 (outer-margin-left 5,
    bottom 11): two horizontal channel bars of 100×6 (`SampTab_MTR_BG`), left one with
    outer-margin-top 9.
  - LED fill `SampTab_MTR_Led` 93×6 per state, 2 states.
  - Clip LEDs `SampTab_MTR_Clip` 5×6, 2 states (lit when clipping).
  - A horizontal **threshold slider** over it (width 103), with an 11×22 grip
    `SampTab_MTR_Handle` (disabled variant when Mode isn't Detect).
- **Recording controls** `m_pRecordingControlsBar` (outer-margin-top 6)
  - Buttons 64 wide (`SampTab_BTN_Record` 14×30 per state, 7 states, caps 4),
    margin-bottom 4.
  - Text high; low when disabled or selected.
  - Labels: **START** (becomes **STOP** while recording) and **CANCEL** (manual).
- **Take list** `RecordingHistoryVector`
  - Height 63, margin-top 5, spacing 4.
  - Each **RecordingItem** is a mini waveform 79×58 (`SampTab_BG_WavePreview`, 3
    states: normal, missing sample, missing sample + focused).
  - Mini waveform background rgb(32,32,32). Unfocused: Surface03. Focused: palette 2
    (the pure colour).
  - **Delete** ✕ button 18×18 at x 56, y 2.
  - **Listen** (prehear) button 18×18 at x 56, 24 from the bottom.

Below that is the waveform display of the take being recorded or focused. Its timeline is
in bars in Sync mode.

### Parameters [exact: Recorder::Recorder, labels in order]

| label | type | default / range | notes (manual) |
|---|---|---|---|
| Recording Status | bool | | runtime |
| **Source** | enum | | "Ext. Ster.", "Ext. Mono", "Internal" |
| **Input** | enum or AudioInputParameter | | Ext. Ster.: In 1–4 stereo pairs. Ext. Mono: In 1–4 L or R. Internal: any group, or the Master |
| **Mode** | enum | default 1 | "Detect", "Sync", "Loop", "Auto" |
| **Threshold** | float, dB | **−12**, range **−60…0** | Detect: starts when the input exceeds it; "OFF" = manual start (the slider's leftmost); double-click resets. `Recorder::convertThreshold_dBToString` |
| **Length** | enum | | Sync: 1, 2, 4, 8, 16 bars, "Free" |
| **Target** | enum | | Loop: "Take", "Sound", "Pattern" |
| **Monitor** | bool | | external sources: sends the input to Cue |
| MIDI To, Channel (int, max 16), Note On (frames?, max 30000), Sample (max 30000), Stride, Key Lo/Hi (0–127, Hi 60), Extend, Stride, Vel Lo/Hi (0–127, Hi 100), Extend, Find Loop, Normalize, Trim Sil | | | **Auto** (auto-sampler) only. Footer groups: Recording (SAMPLE, NOTE ON), Input (SOURCE, INPUT, MIDI TO, CHANNEL), Note Map (STRIDE, KEY LO, KEY HI, EXTEND), Velocity Map (STRIDE, VEL LO, VEL HI, EXTEND), Batch Process (FIND LOOP, TRIM SILENCE, NORMALIZE), Monitor |

**Footer groups for the usual modes** [inferred from the manual and the order above]:
- **SOURCE:** Source, Input.
- **MODE:** Mode, then Threshold (Detect), Length (Sync) or Target (Loop).
- **MONITOR:** Monitor.

### Behaviour [exact: manual; code paths named]

- **START:**
  - Detect waits for the threshold.
  - Sync starts at the next bar.
  - Loop starts at the pattern start and records into an Audio module.
- **STOP:** ends the recording; in Sync and Loop it waits for the bar. **CANCEL**
  discards it.
- **Where takes go** (`FinishedRecordingHandler::finishSingleRecording` →
  `RecorderAlgorithms::loadRecordingToSound`):
  - Each take is added to the **Audio Pool** (the Sampler's `takes`).
  - It is loaded into the focused sound's Sampler as one zone covering all keys and
    velocities. Other plug-ins in that sound are removed, but a Sampler stays.
  - In Loop mode it goes into an Audio module.
- **Files:**
  - Saved in the **Recordings** folder of the user directory, or the project folder's
    Recordings when "Prefer Project Folder" is on.
  - Named `[YYMMDD]T[HHMMSS].wav`.
  - [inferred] Written as 24-bit WAV at the engine rate. Our writer should use 24-bit PCM
    (or 32-bit float) WAV.
- **Take list:**
  - Click a take to focus it. Hold its listen button to audition it. ✕ deletes it.
  - Drag a take onto another sound to load it there.
  - Context menu: "Delete", "Remove unused recordings", "Map recordings to zones".
- **Waveform context menu:** "Open containing folder", "Save Sample As…".

### Stored [exact: schema]

The Sampler's `takes`: `ObjectVector<Take>`. Each `Take` (v2) holds:
- `file` (FileLocation);
- `unknown_88` (QuantizedFloatParameter: [inferred] its length or tempo);
- `unknown_a8` and `unknown_b0` (RangeParameters: [inferred] the play and selection
  ranges);
- `offset`, `unknown_98` (ulong), `zoom_factor` (double) and `quick_browse`.

`offset` and `zoom_factor` are the view's scroll and zoom.

---

## 4. Edit tab (`SampleEditorPanel`)

### Layout [exact]

- Rows: timeline, waveform display, zoom bar, then the **edit toolbar**.
- **Toolbar `m_pButtonStack`:** bitmap `SampTab_BG_ButtonToolbar` 50×60 (caps 10),
  Surface00. Its bar has margin-left 12, spacing 8, groups 8 apart.
- **Buttons:** icon-and-label buttons **48×60**, 7 states (`SampTab_BTN_Edit_*`, 48×420).
  In order:
  - **TRUNCATE**
  - **NORM.**
  - **REVERSE**
  - **FADE IN**
  - **FADE OUT**
  - **CORRECT** (DC; `m_pDC`)
  - **SILENCE**
  - **CUT**
  - **COPY**
  - **PASTE**
  - **DUPL.**
  - **STRETCH**
  - **STEMS**, with a separate cancel button `m_cancelStemSeparation`
- [inferred] Draw each as an icon with its label underneath. Labels in white a164, or
  a245 on hover; greyed when not applicable (nothing selected, or no clipboard for Paste).
- **Stretch mode:** the toolbar turns into the stretch bar `pTimestretchBar` (spacing 10,
  margin-left 10, top 5). Its buttons are 62 wide (`SampTab_BTN_Record` style):
  **APPLY**, **CANCEL**.
- **Footer groups** [manual]:
  - **PLAY RANGE:** Start, End.
  - **SELECTION RANGE:** Start, End.
  - In stretch mode, a **STRETCH** group: Tune, Formant C, Mode, Auto Dtct, Src BPM, New
    BPM, Length, Speed.

### Selection and ranges [manual]

- **Play range:** drag S and E, or the PLAY RANGE values. These are the zone's
  `play_range`.
- **Loop:** drag its borders or title bar. It is the zone's `loop_range` and
  `loop_active`.
- **Selection:** drag on the waveform (it is the zone's `selection_range`), or set
  SELECTION RANGE. Double-clicking the waveform selects the play range.
- **Context menu:** "Deselect", "Select All", "Select Play Range", "Select Loop", "Find in
  Explorer/Finder", "Save Sample As…".

### Edit functions [exact: SampleEditUtils / SampleEditAlgorithms]

Every edit works on the **selection** (`SampleEditAlgorithms::verifySelectionRangeOrFail`;
Truncate also allows the play range [inferred]). Edits are **non-destructive to the
original file**:
- `SampleEditAlgorithms::editSample` runs the operation from the zone's current file to a
  **new file**.
- The new file's path comes from `SampleEditUtils::createTargetFileName` plus
  `SampleEditsManager::getDestinationPathForFile`: the same folder or the standard
  destination, the same name, made unique by `FileNameUtils::makeFilePathUnique`, format
  `"%s - %d%s"` (e.g. `Kick - 2.wav`). An existing `" - N"` suffix is stripped first
  (`removeUniquenessSuffix`).
- **.wav, .aif, .aiff and .aifc** keep their extension. Other types are written as `.wav`
  (`createTargetFileName` checks this list).
- The zone is repointed to the new file in one transaction, so **Undo** (⌘Z) restores the
  old file reference.
- Superseded files are queued for deletion (`SampleEditsManager::addFileToDelete`) and
  removed later if no longer referenced.
- Intermediate files are `"%s - temp%s"` in the system temp folder (`createTempFileName`).

With selection `[a, b)` in frames:

| function | algorithm |
|---|---|
| **Truncate** | keep only `[a, b)` (`trim` / `trimStream`). The play range then covers the whole new file |
| **Normalize** | peak = max \|x\| over `[a,b)` across channels (`getPeakStream`). If the peak > 0, multiply `[a,b)` by **1/peak**, i.e. 0 dBFS (`amplifyStream`). Nothing happens if the peak is 0 |
| **Reverse** | reverse `[a,b)` in place (`reverse`) |
| **Fade In / Fade Out** | a linear gain ramp over `[a,b)` (`fadeStream` → `AudioBlockAmplifier::process(block, g0, g1)`): fade in 0 → 1, fade out 1 → 0. [inferred: the endpoints and linear shape come from the amplifier's two gains; the callers pass constants] |
| **Correct (DC)** | per channel, subtract the mean of `[a,b)` from `[a,b)` (`removeDC`: an average, then `addScalarVec(−mean)`) |
| **Silence** | zero `[a,b)` (`silenceStream`) |
| **Cut** | copy `[a,b)` to the clipboard file (`SampleEditsManager::setClipboardFile`), then remove it, joining the rest (`cut`, `cutStream`) |
| **Copy** | write `[a,b)` to the clipboard file |
| **Paste** | insert the clipboard file at the selection start (replacing `[a,b)` when a > b? [inferred: insert at a]). Channel count is matched with `transcodeStreamMismatchingChannels` (mono ↔ stereo) |
| **Duplicate** | insert a copy of `[a,b)` right after b (`duplicate`) |
| **Stretch** | see below (`ZoneSampleEditAlgorithms::timestretch` → `SampleEditUtils::timestretch(Stream)`, élastique Pro V3) |
| **Stems** | separates into drums/percussion, bass, other and vocals in the background, into a new group's first 4 sounds. Out of scope for us |

**Click prevention.** `preventClicksAtStartAndEnd(block, n)` applies to some outputs
(Cut, Paste and Duplicate joins [inferred]). It ramps the first and last `n` frames
linearly (0 → 1, gain step 1/n) and, for n = 1, looks for zero crossings.

### Stretch [exact: TimeStretchParameters labels; manual]

| label | meaning |
|---|---|
| **Tune** | semitones and cents (pitch shift), default 0.00 |
| **Formant C** | formant correction on or off |
| **Mode** | "Beat" or "Free" |
| **Auto Dtct** | Beat: detect the source tempo |
| **Src BPM** | Beat: with Auto, a length of 1/2, 1 or 2 bars; otherwise a BPM value |
| **New BPM** | Beat: the target tempo |
| **Length** | Beat with Auto: 1/16 to 8 bars, including triplets (`getStretchLengthString`) |
| **Speed** | Free: percent, minimum 10% |

`getAlgorithmParams(stretch, pitch, formant)` turns these into an élastique time factor,
pitch factor and formant flag.
- **Beat:** time factor = Src BPM / New BPM.
- **Free:** time factor = 100 / Speed. [inferred]
- **Pitch:** factor = 2^(Tune/12).
- **APPLY** is disabled when New BPM < Src BPM / 10.

élastique is proprietary. [inferred] We need our own algorithm, e.g. a phase vocoder with
transient handling or WSOLA, behind the same parameters. Stretch is the one edit that
won't match Maschine bit for bit.

---

## 5. Slice tab (`SlicingPanel`, `SlicingWaveEditor`)

### Layout [exact]

- Rows: timeline, slicing wave display (§2), zoom bar, then the **slice toolbar**
  `m_pButtonBar` (`SampTab_BG_ButtonToolbar`, margins 10/10, top 1).
- **Tools** (48×60 icon buttons):
  - **SLICE** (`m_pSlice`): click inside a slice to split it.
  - **REMOVE** (`m_pErase`, outer-margin-left 15): click a start border to merge it with
    the previous slice; click inside a slice to drop it from export.
  - **DELETE ALL** (`m_pClear`, outer-margin-left 15): clears all slices.
  - SLICE and REMOVE are mutually exclusive.
- **Right side:**
  - **APPLY** (`m_pApply`): 57 wide, 20×53 per state (`SampTab_BTN_ApplySlice`),
    outer-margin-top 4.
  - The **slice dragger** (`m_pDrag`): 37×53 per state (`SampTab_BTN_SliceDrag`),
    outer-margin-left 3. Drag it onto a sound or a group.
- **Footer groups** [manual]:
  - **SLICE:** Mode, then Slices (Split/Grid), Sensitivity + Zero-X (Detect) or Auto-Snap
    (Manual).
  - **APPLY:** Mono, BPM (Auto/Manual), Adjust.
  - The pattern selector: "Create Pattern", "Replace Pattern", "No Pattern".

### Parameters [exact: SampleSliceContainer ctor labels]

`Mode`, `Slices` (Split count), `Slices` (Grid note length), `Sensitivity`, `BPM` (mode:
Auto/Manual), `Adjust`, `Mouse Mode` (Slice/Remove tool), `Zero-X`, `Show Focus Slice`,
`Num Slices` (read-only), `Analyzing` (progress).

- **Mode:** "Split", "Grid", "Detect", "Manual".
- **Split Slices:** 4, 8, 16, 32.
- **Grid Slices:** 4th, 8th, 16th, 32nd notes.

### Algorithms [exact unless marked]

**Split**
- `getSplitPositions`: N equal parts of the play range.
- Position `k` = start + round(k·len/N), for k = 0…N−1. [inferred rounding]

**Grid** (`calculateGridPositions(sampleRate, frames, bpm, division)`)
- `samplesPerGrid = sampleRate·60·4 / bpm / division`, where division = 4, 8, 16 or 32.
- `count = round(frames / samplesPerGrid)`. Positions are `k·samplesPerGrid`.
- The tempo comes from BPM Auto (the detected tempo, × or ÷ 2 by Adjust) or Manual
  (Adjust sets it).

**Detect** (`SliceDetectorDSP`)
- Mixes down to mono at a 44.1 kHz analysis rate (`mixdownToMono`).
- Spectral onset analysis (`doFrequencyAnalysis`) finds candidate transients **sorted by
  strength**.
- `getTransients(sensitivity, zeroX)` takes the first `count·sensitivity` candidates
  (sensitivity 0…1). It reads from the zero-crossing-snapped list when Zero-X is on. It
  rescales positions to the sample rate (`· sr / 44100`).
- Positions are sorted and form the slice starts.
- [inferred] For our implementation: an energy/spectral-flux onset detector ranked by
  strength gives the same control behaviour.

**Manual**
- Click with SLICE to add a marker.
- Auto-Snap moves new markers to the nearest transient within a range
  (`findNearestTransientWithinRange`).

**Slices** are contiguous `SampleSlice{start, length}` items.
- Dragging a border moves the joined boundary (`moveSliceBorder`).
- Dragging an S or E flag moves one edge on its own (gaps and overlaps allowed:
  `moveSliceStart` / `moveSliceEnd`).
- Clicking inside a slice auditions it.

### Apply and export [manual; Sampler::initializeAsSlices exact]

- **APPLY** exports to the **same sound** (`Sampler::initializeAsSlices`):
  - one zone per slice, all on the same sample file, play range = the slice;
  - slice *i* mapped to one key, starting at **C1 (MIDI 36)**, from the manual's C-2 in
    Maschine's naming [inferred mapping];
  - root key = that key, full velocity range;
  - the editor switches to Keyboard view.
- The **dragger** onto a **group** puts the first 16 slices into sound slots 1–16, one
  Sampler each.
- **Mono** sets Voice 1 and the choke group when slicing to a group.
- **Pattern:** "Create Pattern" or "Replace Pattern" writes a pattern with one note per
  slice at its position (at the BPM). "No Pattern" doesn't.

### Stored [exact: schema]

`SampleSlice` is three `MaschineNumericParameter<unsigned long>`:
- `unknown_48` = start frame [inferred];
- `unknown_50` = length [inferred];
- `unknown_tail` = [unknown].

They live in the slice container, which isn't saved inside the Sampler module;
[inferred] it belongs to the take or zone being edited.

---

## 6. Zone tab (`ZoneEditorPanel`)

### Layout [exact]

- **Zone list `pZoneManager`**
  - Left, Surface00, width **140–250** (resizable with a 3 px Surface00 splitter). Body
    margin-left 15.
  - Rows 24 high, margin-right 10, outer margins 5.
  - Names in text low; medium on hover; white when selected; palette 1 when focused.
  - The last row is the **"+"** (`sampling_pg_btn_zone_list_add`, 24×24, 7 states).
  - Toggled by the Zone List header button.
- **View stack:** **Map view** (default) or **Sample view** (the Sample View header
  button).
- **Map view (`ZoneMapEditor`):** a **grid** over a **keyboard** (`m_pGridAndKeyboardBar`,
  spacing 1) plus zoom scrollbars (horizontal: `SHARED_BG_Scrollbar`; corner spacer
  18×18).
  - **Grid `ZoneEditorGridWidget`**
    - Columns: one per key, **C-2…G8** (MIDI 0–127). Rows: velocity **0–127**, bottom to
      top.
    - Background by key colour: white-key columns rgb(53,53,53), black-key columns
      rgb(47,47,47).
    - Grid lines rgb(38,38,38); octave lines #000000.
    - Rubber-band selection: fill rgba(112,112,112,64), border rgba(200,200,200,255).
  - **Zone rectangles `ZoneWidget`**
    - Fill palette 4 (mute colour) at alpha 32; border white a102.
    - Selected: palette 1 at alpha 60, white border.
    - Focused: palette 1 at alpha 120, white border.
    - The sample name in the header, white, padding 5.
  - **Keyboard `ZoneEditorKeyboard`** under the grid
    - White keys rgb(134,134,134), black keys rgb(28,28,28), lines rgb(28,28,28).
    - The focused zone's root key is drawn in palette 1. Drag it to change the root key.
- **Sample view:** the focused zone's waveform as on the Edit tab (S/E markers, loop
  frame, playhead, timeline in seconds, zoom bar).

### Gestures [manual]

**List**
- Click to focus. ⌘-click toggles; Shift-click selects a range; ⌘A selects all.
- Drag to reorder.
- Drop samples on the list (or "+") to add zones. Drop on an entry, or use "Load
  Sample…", to replace that zone's sample.
- Del/Backspace deletes. The context menu has "Delete", "Delete Selected" and "Load
  Sample…".

**Map**
- Click to focus. Rubber-band selection selects every zone inside or overlapping it.
- Drag left/right borders for the key range, top/bottom borders for the velocity range,
  corners for both.
- Drag inside a zone to move it; its root key moves with it.
- Double-click a zone to extend it to its neighbours' limits.
- Del removes the selection.
- **Context menu:**
  - "Delete";
  - "**Map as Drum Kit**": one key per selected zone, from **C3 (MIDI 60)** upward in list
    order, full velocity range.
- **Dropping samples on the map**
  - Horizontal position sets the root key.
  - Vertical position sets the key range: lower half = only the root key; upper half = up
    to an octave above it; top = the full keyboard.
  - Several samples make adjacent zones.

### Zone parameters (footer) [exact: manual group names]

Each maps to our `Zone` (`crates/model/src/lib.rs`) and the Zone schema:

| group | label | field |
|---|---|---|
| PLAY RANGE | START, END | `play_range/low`, `play_range/high` (frames, inclusive) |
| LOOP | ACTIVE, START, END, XFADE | `loop_active`; `loop_range/low`, `loop_range/high`; `x_fade` (frames). ACTIVE needs the Sampler's Type set to AHD or ADSR |
| TUNE / MIX | TUNE, GAIN, PAN, ROOT KEY | `tune` (±36 st), `gain` (0..1, 0.75 = 0 dB, see sampler-dsp.md §6.6), `pan` (−1..1), `root_key` |
| ENVELOPE | ATTACK, DECAY | `attack` (zone fade-in, frames), `decay` (zone fade-out, frames) |
| MAP | KEY LO, KEY HI, VEL LO, VEL HI | `key_range/low`, `key_range/high`, `velocity_range/low`, `velocity_range/high` |

The zone's other stored fields are `offset` and `zoom_factor` (the Sample view's scroll
and zoom), `selection_range` (the Edit selection) and `file`.

**New zones** cover the full key and velocity ranges (`Sampler::createZone`) with play
range = the whole file. [inferred: default loop off, gain 0.75, root key 60]

---

## 7. What our model and document need [inferred]

- **Zone editing:** `Target::Zone` already covers the PLAY RANGE, LOOP and TUNE / MIX
  fields. Add `key_range/*`, `velocity_range/*`, `root_key`, `attack`/`decay` (fades)
  and `selection_range/*` patches.
- **Zone list changes:** add, remove and reorder zones, and replace a zone's file. These
  are ObjectVector edits like the chain's slots; copy a zone as a template from the same
  Sampler or any project.
- **Takes:** append a `Take` for each recording (template copy), and remove takes.
- **Sample edits:** write a new file (WAV), then set the zone's `file` FileLocation as one
  undo step. For files outside a library, set `kind` 0 with an absolute `path` and no
  alias. [inferred: kind 0 = absolute path, as `FileLocation` has no alias when kind is 0]
- **Playhead:** the engine reports the focused zone's newest voice frame.
- **Recording:** a cpal input stream into a WAV writer, with threshold detection, sync to
  the bar, and an input meter (peak per channel, clip latch).

## DECKARD: SAMPLE on the controller, and the idle meter

- **SAMPLE** (MK3): opening the Sampling page shows the app's Sampling view for the
  focused sound, on RECORD if nothing is sampled yet; its chain is left as it is (a
  Sampler's sample can be edited there). Leaving the page hides the view again.
- **A recorded take** (Detect, Sync, Loop) on a sound without a Sampler makes it a lone
  Sampler first: its other modules and plug-ins are removed, as one undo step
  (`Document::sampler_only`), as the manual describes. A sound with a Sampler keeps its
  chain; the take replaces its zones.
- **SAMPLE LED:** dim while the page is hidden. The script asks for `LS_DIM_FLASH`, and
  DECKARD shows that dim, as `LS_DIM`.
- **Input meter:** RECORD's meter shows the chosen source's level while nothing is recorded.
  - It follows the interface input, or the internal group or master tap while something
    plays.
  - The input is open only while the RECORD tab shows. With Monitor on, you hear it.
- **The controller's RECORD screen:** the left display's input meter
  (`NI.GUI.insertMasterLevelMeter`) shows the same levels.
  - The app's recorder fills the focused recorder every frame (`api/recorder.rs`): levels,
    mode, threshold, waiting/recording.
  - NI's script passes these to the meter, and DECKARD draws it (`record_meter.rs`):
    - a 463 × 10 bar per channel, in the sound's colour, from −60 to 0 dB;
    - a red mark where a channel clips;
    - the Detect threshold as a white line.
  - The input is open while that screen shows, too.
  - `lua-host-run --levels L,R` renders it with given levels.
  - START (button 5) and CANCEL (button 6) work the app's recorder, as its own START /
    STOP and CANCEL buttons do: NI's script calls `NI.DATA.RecorderAccess.toggleRecording`
    / `requestCancelRecording`, which leave a `RecordPress` for the app
    (`Request::Record`). The screen then reads WAITING or STOP from the recorder.
- **RECORD screen knobs:** the app's recorder settings, as the RECORD tab's footer has
  them, in pages of up to four (knobs 5 and 6 zoom and scroll); LEFT / RIGHT page
  through them (`apps/viewer/src/record_knobs.rs`, served as
  `App:getSamplingParameters():getPagesForTab(0)`):
  - RECORDING: MODE, then THRESHOLD (Detect), LENGTH (Sync), LENGTH and TARGET (Loop),
    or SAMPLE and NOTE ON (Auto);
  - INPUT: SOURCE, INPUT (the connected inputs, the master or a group, or Auto's
    sound), MONITOR (external sources);
  - Auto: MIDI (MIDI TO, CHANNEL; external), NOTE MAP, VELOCITY MAP, BATCH PROCESS.
  - A choice moves an option, a number a step, every 32nd of a turn; the threshold half a
    dB every 120th. Turns leave `State::record_edits`, which the app applies (settings
    stay as they are while it records, and the script disables the knobs then).
- **EDIT, SLICE and ZONE on the controller** show the focused zone's sample as the app
  has it (`apps/viewer/src/sampling_hw.rs` → `State::sample`; `api/sampling.rs`,
  `sample_view.rs`), and the controller's tab and the app's follow each other:
  - The wave displays (`insertSampleOwnerWaveEditor`, `insertSliceWaveEditor`): the
    left one the whole sample with a frame around the part shown on the right; the
    sound's colour inside the play range, grey outside it; the selection (EDIT), the
    loop, the slices with the focused one lit (SLICE), the play range's ends and the
    playhead. Knobs 5 and 6 zoom and scroll (the app's view). The zone map
    (`insertZoneMapEditor`, ZONE's left display) shows the zones over keys and
    velocities, the focused one lit.
  - Knobs, as the MK3 manual's pages: EDIT PLAY RANGE and SELECTION RANGE (START,
    END); ZONE PLAY RANGE, LOOP (ACTIVE, START, END, XFADE), TUNE / MIX (TUNE, GAIN, PAN,
    ROOT KEY), ENVELOPE (ATTACK, DECAY), MAP (KEY LO/HI, VEL LO/HI); SLICE the ones NI's
    script asks its slice container for (MODE, SLICES / SENSITIVITY and ZERO-X; the
    focused slice and its START / END on page 2; the PATTERN APPLY makes on page 3).
    Frame knobs cross the part shown in a turn.
  - Buttons: EDIT's ◀ ▶ pick a function and APPLY runs it (Truncate … Duplicate, the
    app's edits; STEMS separates the sample into a new group of four stems —
    `docs/stems.md`; AUTO LOOP finds a loop in the play range and switches it on — DECKARD's
    own `edit::find_loop`, as the software's AUTO LOOP button; STRETCH isn't on the
    controller yet); SLICE's SPLIT
    halves the focused slice and REMOVE joins it to the one before (the slicing becomes
    Manual), DELETE ALL, APPLY… (to the focused sound); ZONE's REMOVE, PREVIOUS, NEXT.
  - Not yet: playing a slice from its pad (there's no sample preview), ADD (the sample
    browser), the record take display.
- **Dragging in the software's waveform:** the S and E flags at the top of the play
  range's lines drag them, as the lines do; the pointer shows what can be dragged there
  (↔ over the markers, loop edges and slices, a hand over the loop's tab and the zoom
  bar, ↕ over the footer's values and the zone map's top and bottom edges).
- **Samples dropped on a sound:** WAV or AIFF files dragged from the desktop onto a sound
  row (the pattern editor's or the Sampling view's list; lit while they're over it) make
  that sound a lone Sampler playing the file, named after it if it still had a default
  name, as one undo step; several files fill that sound and the ones after it. Other
  files dropped on the window open as before. winit 0.30 doesn't say where a drag is on
  macOS or Windows, so `drop_pos.rs` asks the system (AppKit, Win32); elsewhere the last
  pointer position, and a drop nowhere on a row goes to the focused sound.
