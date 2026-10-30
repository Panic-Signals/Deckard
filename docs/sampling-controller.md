# MK3 SAMPLING page: behaviour spec

What the Maschine MK3 does on its SAMPLING page (the SAMPLE button), written so DECKARD's
own controller layer can do the same thing. This is a clean-room description. NI's scripts
were read only to learn the behaviour, and none of their code is reproduced here.

**References** (NI scripts, under `out/ni-res/maschine_nwl_resources/Scripts/Maschine/`):
`MaschineMK3/Pages/SamplingPageMK3.lua` (only the SAMPLE LED differs; everything else is
inherited), `MaschineStudio/Pages/SamplingPageStudio.lua` (screen layout),
`Components/Pages/SamplingPageBase.lua` (behaviour), `Helper/SamplingHelper.lua`,
`MaschineStudio/Pages/SamplingPageSliceApplyStudio.lua` and
`Components/Pages/SamplingPageSliceApplyBase.lua` (the Apply dialog),
`MaschineStudio/Pages/BrowsePageStudio.lua` and the shared `BrowsePageBase.lua` (ZONE ▸
ADD), and `Components/CapacitiveOverlayNavIcons.lua` (the knob-touch icons). Style sheets:
`Maschine/HW/MaschineStudio/SamplingPage.txt` and
`shared_nwl_resources/Shared/HW/MaschineStudio/CapacitiveOverlays.txt`. The MK2-era
`Maschine/Pages/SamplingPage.lua` is not used by the MK3.

**Geometry** comes from the layout dumps `sm-{rec,edit,edit2,slice,zone}.layout.txt` and
pixels measured on the captures `sm-*-ni.png` (Maschine 3 with a one-shot "Kick Creeks
1.wav", 0.219 s, red sound colour). Coordinates are pixels on the 960 × 272 pair of displays:
the left display is x 0–479 and the right is x 480–959. Rectangles are `[x,y w×h]`. Every
font is Pragmatica (`prg65_j.ttf`), size 14 unless stated otherwise. Colours are `r,g,b` or
`r,g,b,a`. Anything not settled is marked **unknown**.

Related docs: `docs/sampling-layout.md` (the software Sampling view: data, edits, slicing
modes), `docs/sampler-pages.md`, `docs/controller-browse-page.md`.

---

## 1. Model

- **Tab.** The page has four tabs, RECORD, EDIT, SLICE and ZONE. The tab is the
  workspace's *sampling tab*, which the software Sampling view shares. Pressing display
  button 1–4 sets that value (RECORD = 0 … ZONE = 3). The page follows the value whenever it
  changes, including changes made with the mouse.
- **Focus.** Everything acts on the focused sound: its Sampler (zones, focused zone and that
  zone's sample, called "the sample" below) or its Audio plug-in, and its recorder and takes.
- **Audio plug-in instead of a Sampler.** Only RECORD and EDIT are shown. EDIT is drawn as
  the last tab (right-rounded style), and the EDIT functions skip STRETCH and AUTOLOOP.
- **No focused group.** Both display stacks are hidden and buttons 5–8 are blank.
- **Sub-pages.** Two sub-pages can cover the page. While one is open it gets the display
  buttons, knobs, pads and screens:
  - SLICE ▸ APPLY… opens the Slice Apply dialog (§8).
  - ZONE ▸ ADD opens the sample browser (§7.4).
- **Pinned.** The page is a normal (pinned) page, not a hold-to-show page. It runs a timer
  every 2 controller ticks. The timer refreshes the recorder state, the slice-prehear state
  and slice analysis.

## 2. Frame shared by all tabs

### 2.1 Left display

| Element | Rect | Look |
|---|---|---|
| Background | `[0,0 480×272]` | 0,0,0 |
| Tab bar (buttons 1–4) | RECORD `[4,4 117×24]`, EDIT `[121,4 119×24]`, SLICE `[240,4 119×24]`, ZONE `[359,4 117×24]` | Together they form one strip, x 4–475, y 4–27, with a 1 px 71,71,71 outline. The selected tab is filled 178,178,178 with black text. The others are black with white text (alpha 229), centred. |
| Info bar (group) | `[0,35 480×24]` | Fill 26,26,26. Contains: the group number in a 24×24 square of the group colour (number size 13, black); the group name (white, x 28); the tempo "135.00" (white), "BPM" (size 11, alpha 128); and the scene name in the group colour. This is the standard Studio/MK3 info bar, the same on other pages. |
| Display area | `[0,60 480×144]` (y 60–203) | Per tab (§3–§6). The background is black. A 1 px black gap separates it from the info bar (y 59). |
| Parameter header | `[0,206 480×27]` | Page name `[0,206 360×25]`, white, left inset 12. Page indicator "n/m" `[360,206 120×25]`, right-aligned, right inset 12, white alpha 128. |
| Parameter slots 1–4 | `[0,233 120×40]`, `[120,233 …]`, `[240,233 …]`, `[360,233 …]` | Name: size 11, white alpha 128, at y 233, 14 px tall, inset 10 (text starts at x+11). Value: size 20, white, at y 247, 28 px tall, inset 10. Unit: size 15, white alpha 128, 5 px after the value. A slot that holds a parameter has a 1 px divider (119,111,111) on its left edge, at x 120/240/360, y 233–271. Empty slots have none. |

### 2.2 Right display

| Element | Rect | Look |
|---|---|---|
| Buttons 5–8 | `[484,4 115×24]`, `[603,4 115×24]`, `[722,4 115×24]`, `[841,4 115×24]` | Enabled: black fill, 1 px 71,71,71 outline, white text (alpha 229), centred. Disabled: 1 px 41,41,41 outline, text 41,41,41 (barely readable). Hidden: nothing drawn. Selected: filled light grey like a selected tab (**unknown** exact colour, assumed 178,178,178 with black text). |
| Arrow pair (5+6 or 7+8 used as one "◀ LABEL ▶" control) | `[484,4 234×24]` or `[722,4 234×24]` | One 234×24 box with a 1 px 71,71,71 outline. The label is centred (white alpha 229) between small ◀ and ▶ triangles at the box's inner ends. The ◀ hit area is the left 22 px; the ▶ hit area is the right 16 px. |
| Info bar (sample) | `[480,35 480×24]` | Fill 26,26,26. Sample name `[490,38 347×18]`: white, left-aligned, cropped to fit, flexible width. Length value e.g. "0.219 s" (white, auto width, ends about 55 px from the right). "LENGTH" (size 11, alpha 128, right inset 10). The texts show only when there is something to describe: on RECORD the focused take (its name and length); on the other tabs the focused sample (its display name and length). Otherwise the bar is drawn empty. |
| Display area | `[480,60 480×212]` (y 60–271) | Per tab. Black background. |

### 2.3 Common waveform look (EDIT, SLICE overview and detail, ZONE detail)

| Item | Look |
|---|---|
| Background | black (also the play-range background) |
| Waveform inside the play range | the sound colour (here 255,0,30), mirrored about the vertical centre, filled |
| Waveform outside the play range | 34,34,34 |
| Selection | background 16,16,16; waveform still in the sound colour |
| Play-range start marker | a white line 2 px wide over the full height of the area, at the start frame (seen at x 0–1 on the left and x 480–481 on the right). The end marker is the same shape at the end frame; in the captures it sits past the last pixel and is not visible (**unknown** whether it is drawn inside at full zoom-out). |
| Loop | a white frame around the loop region (loop-frame image: 24 px fixed left, top and bottom, 5 px right). Shown only when the zone loops. |
| Play cursor | white, alpha 198, 1 px |
| dB lines / channel separator | none visible in the captures |
| Overview (left) zoom window | When the right view is zoomed in, the left overview draws a highlight frame (4 px corners) around the visible span and masks the parts outside it. **Unknown** colours (image-based). |

The left area is an **overview**: always the whole sample, 480×144. The right area is the
**detail** view of the same sample: 480×212, zoomed and scrolled with knobs 5 and 6.

## 3. RECORD tab

### 3.1 Display buttons 5–8

| State | 5 | 6 | 7 | 8 |
|---|---|---|---|---|
| Idle | **START**: toggles recording (arms, or starts at once, depending on the mode). In Auto mode it is enabled only if a valid MIDI output is selected; otherwise always enabled. | **DELETE**: removes the focused take. Enabled only when a take is focused. | **PREVIOUS**: focuses (and loads) the previous take. Enabled if there is an earlier take; with takes but none focused, both 7 and 8 are enabled (7 jumps to the last take, 8 to the first). | **NEXT**: the next take. Enabled if there is a later one. |
| Waiting for the threshold / sync | **WAITING**: pressing it sends the same record toggle again (presumably it disarms; **unknown**, the app decides) | **CANCEL**: cancels the recording | hidden | hidden |
| Recording | **STOP** | **CANCEL** | hidden | hidden |
| SHIFT held, idle | **TAKES**: toggles "takes on pads" (the workspace setting). Shown selected while it is on. | hidden (on the desktop app; it would be RENAME on headless units) | With takes on pads: an arrow pair "◀ BANK n ▶" (n = 1-based pad bank of takes). 7 goes to the previous bank and 8 to the next; the new bank's first take is focused. Each side is enabled when that bank exists. With takes on pads off: 7 and 8 are hidden. | |

Labels 5 and 6 are plain buttons, not an arrow pair. While waiting, button 5's LED flashes
between bright and dim (4 timer ticks bright, 4 dim). The page checks the recorder state on
every timer tick and redraws the buttons when it changes.

### 3.2 Knobs

- Knobs 1–4 show the app's RECORD parameter pages: 4 slots per page, page name in the
  header, "n/m" when there is more than one page. The capture shows "RECORDING 1/2" with
  MODE "Detect" and THRESHOLD "0.0 dB". LEFT/RIGHT step through the pages (§9.2).
- While waiting or recording, if the current page is flagged *disabled while recording*,
  **every** knob (1–8, zoom and scroll included) is ignored. The knob-touch list overlay is
  also cleared.
- Knob 5: zoom, knob 6: scroll (§10). They act on the focused **take**'s view, not on the
  sample.
- Knobs 7–8: these go to parameter slots 7–8 of the app page, which are normally empty, so
  in practice they do nothing.
- Touching knobs 5–8 shows the zoom/scroll icons (§10) only when not recording and at least
  one take exists.

### 3.3 Pads

- **Takes on pads off** (default): pads play the sounds as usual (sound pad mode), and the
  pad LEDs are the normal sound LEDs.
- **Takes on pads on**: pad n focuses take `n + 16·bank`, loads it into the sound and
  prehears it. Pad LEDs, all in the **sound colour**: the focused take's pad is bright, other
  existing takes are dim, and pads beyond the last take are off. The bank follows the
  focused take (`take index ÷ 16`) whenever the focused sample changes.

### 3.4 Screens

- **Left, the record meter** `[0,75 480×58]`: two horizontal channel bars, left then right.
  Each is 10 px tall, at x 7–468, y 90–99 and y 108–117. An empty bar is 38,38,38; the level
  fills it from the left in the **sound colour**. The meter has a peak hold that falls away
  over 0.5 s.
  - Clip indicators: 6×6 squares that turn red when a channel clips (**unknown** exact
    position; to the right of each bar).
  - In **Detect** mode the threshold is drawn as a white vertical line 2 px wide and 40 px
    tall (y 84–123). It spans both bars, and its x is the threshold's position on the meter
    scale (at 0.0 dB it sits at the right end, x 469–470). In other modes there is no line.
- **Left, the take list** `[0,133 480×71]`: a row of take thumbnails. Row: 63 px tall, top
  inset 13, left inset 8, 8 px apart (**unknown** thumbnail width).
  - Each thumbnail has a 2 px frame. The focused take's frame is white. A take with missing
    samples has its own frame variant, filled with the sound colour when it is focused.
  - The thumbnail waveform is grey 89,89,89, or the sound colour for the focused take.
  - With no takes the row is empty (black).
- **Right**: the record waveform view `[480,60 480×212]`. It shows the focused take's
  waveform, and while recording the incoming audio as it is written (**unknown** detail:
  colours and scrolling while recording). With no takes it is black, and the info bar texts
  are hidden.
- **Status text** while waiting or recording: none from the page beyond the button labels
  (WAITING/STOP, CANCEL) and the red SAMPLE LED (§9.1).

## 4. EDIT tab

### 4.1 Display buttons 5–8

| Button | Label | Action | Enabled / visible |
|---|---|---|---|
| 5+6 | arrow pair "◀ *FUNCTION* ▶" | 5 moves to the previous edit function, 6 to the next; the list wraps around. Moving away from STRETCH closes the stretch settings. | Enabled only when the sound has a sample. |
| 7 | **SETTINGS** | Opens or closes the time-stretch settings (§4.3). | Visible and enabled only when the function is STRETCH and there is a sample. Shown selected (LED bright) while the settings are open. |
| 8 | **APPLY** | Applies the current function to the sample (to the selection range where the function uses one). | Enabled when there is a sample; for STRETCH also only while the stretch values are in range. For STEMS see §4.4. |

**Edit functions, in order, exactly as shown:**

1. TRUNCATE
2. NORMALIZE
3. REVERSE
4. FADE IN
5. FADE OUT
6. FIX DC
7. SILENCE
8. CUT
9. COPY
10. PASTE
11. DUPLICATE
12. STRETCH
13. STEMS (desktop builds only; NI's standalone/headless units have no STEMS)
14. AUTOLOOP (finds loop points for the focused zone and sets them)

The page remembers one function index for Sampler sounds and another for Audio-plug-in
sounds. With an Audio plug-in, STRETCH and AUTOLOOP are skipped in both directions. DECKARD
note: if stem separation is not available, leave STEMS out, as NI does on its standalone
units.

### 4.2 Knobs, pads, screens

- Knobs 1–4: the app's EDIT parameter pages, 4 slots each. The captures show 2 pages:
  "PLAY RANGE 1/2" (START, END in s) and "SELECTION RANGE 2/2" (START, END). A *range*
  parameter takes two knobs, min then max.
- Knob 5 zoom, knob 6 scroll, acting on the sample's view (§10). Knobs 7–8: no effect.
- Pads: sound mode (they play and focus sounds as usual). Pad LEDs are the normal sound LEDs.
- Left: the overview of the sample (§2.3). Right: the detail view. With **no sample** the
  overview is hidden (black), the info bar texts are hidden, buttons 5, 6 and 8 are disabled,
  7 is hidden, and the knob-touch icons are off.

### 4.3 STRETCH settings open

- The right display keeps its waveform area. A second parameter row is added at the bottom
  of the right display: header 26 px with no fill, then 4 slots in the same style as §2.1.
- **All eight knobs** now edit the stretch parameters, 1–4 on the left and 5–8 on the right.
  Zoom and scroll are off, and so are the knob-touch icons.
- APPLY performs the stretch. It is greyed while the values are out of range.

### 4.4 STEMS

- Button 8 reads **APPLY** and starts stem separation for the sample. It is enabled when
  there is a sample and no separation is running anywhere.
- While this sample is being separated, button 8 reads **CANCEL** (always enabled) and
  cancels it. The screens are replaced:
  - Left: the text "Stem Separation in Progress", size 21, white, centred both ways, with
    20 px side insets.
  - Right: "Exporting Sound...", size 18, white, centred, 24 px tall, 76 px from the top of
    the display area, 100 px side insets. Below it, 6 px lower, a progress bar 20 px tall with
    100 px side insets: 1 px border white alpha 30 (76), 4 px padding, fill white alpha 70
    (179), 12 px tall. The bar fills with progress 0–1.
  - Pads are forced to sound mode, and the knob-touch icons are off.

## 5. SLICE tab

### 5.1 Display buttons 5–8

| Button | Label | Action |
|---|---|---|
| 5 | **SPLIT**, or **SLICE** while a slice prehear is playing | Splits the focused slice in two (while playing, at the play position: **unknown**, the app decides). The markers are redrawn. |
| 6 | **REMOVE** | Removes the focused slice's marker, merging that slice with its neighbour. |
| 7 | **DELETE ALL** | Removes all slices. |
| 8 | **APPLY…** | Opens the Slice Apply dialog (§8). |
| SHIFT held | 5 and 6 hidden; 7+8 become an arrow pair "◀ BANK n ▶" | They page the 16-pad slice bank. 7 is enabled when n > 1, 8 when a later bank exists. While SHIFT is held, pressing 5 or 6 also counts as "previous bank". |

All four are enabled only when there is a sample. Without one, presses do nothing.

### 5.2 Knobs (page-local, 3 pages; LEFT/RIGHT move between them)

The page indicator always shows "n/3". With no sample there is one empty page.

| Page | Header | Knob 1 | Knob 2 | Knob 3 | Knob 4 |
|---|---|---|---|---|---|
| 1 Slicer, mode **Detect** | Slicer | MODE | while analysis runs: an "in progress" readout; once done: sensitivity (density) | after analysis: zero-crossing snap | — |
| 1 Slicer, mode **Split** | Slicer | MODE | SLICES (number of slices; the capture shows "Split" / "8") | — | — |
| 1 Slicer, mode **Grid** | Slicer | MODE | note length | BPM mode (auto/manual) | BPM: the auto value or the manual value, as the BPM mode says |
| 1 Slicer, mode **Manual** | Slicer | MODE | after analysis: auto-snap; before: the in-progress readout | — | — |
| 2 Edit | Edit | **SLICE**, value "f/N" (focused slice / count, or "-/-"). Turning it steps the focused slice by one per detent, with a smoothing threshold so a slow turn does not skip. It does not prehear. | (blank) | **START**, value the start frame. Turning it moves the focused slice's start (it drags the previous slice's end with it, and snaps to it within about 10 px' worth of frames). | **END**, value the end frame. It moves the end (detaching it from the next slice's start). |
| 3 Apply | Apply | mono (sum slices to mono) | apply mode (pattern creation) | if the apply mode makes a pattern and the slice mode is not Grid: BPM mode | …and BPM |

Notes on the knobs:

- START and END step by a fraction of the visible span per detent (it gets finer as you zoom
  in). With SHIFT they move one frame per detent.
- The analysis (Detect/Manual) runs in the background. The timer refreshes its progress; when
  it finishes the slices are applied without an undo step.
- Knob 5 zoom, knob 6 scroll (§10).

### 5.3 Pads

Pads are in a custom mode: they do not play sounds. With a bank of 16, pad *p* stands for
slice `p + 16·bank`.

- Pressing the pad of an existing slice that is not focused focuses it and prehears it.
  Pressing the pad of the focused slice prehears it again.
- **Live slicing.** While the **last** slice is focused and playing, the pad just after the
  last slice is the "slicer pad". Pressing it splits the playing slice (it inserts a marker
  in real time) and redraws the markers.
- The bank follows the focused slice (or the last slice if none is focused). It moves one
  further while the slicer pad is active and would be on the next bank.

Pad LEDs, all in the **sound colour**:

| Pad | LED |
|---|---|
| focused slice | bright |
| other existing slice | dim |
| slicer pad (last slice focused and playing) | fast blink: 4 ticks dim, 4 bright |
| only one slice, nothing playing | pad 1 slow blink: 32 ticks dim, 32 bright ("waiting to be sliced") |
| beyond the slices | off |

### 5.4 Screens

- **Left**: slice overview `[0,60 480×144]`. **Right**: slice detail `[480,60 480×212]`. The
  focused slice is always shown.
- Waveform: in a slice, the sound colour; outside every slice, 89,89,89.
- Slice markers are 1 px vertical lines over the full height:
  - a normal marker is white alpha 20 %;
  - a marker set by the slicer ("changed") is white alpha 60 %, which reads as 150,150,150
    on black and 255,150,162 where it crosses a red waveform. In the capture (Split, 8
    slices) every marker is drawn this way, every 60 px on the left and right.
- Focused slice:
  - background 41,41,41;
  - waveform in the sound colour;
  - white start and end markers, the same 2 px markers as the play range.
  - In the capture a white 2 px line is visible only at x 0–1 and x 480–481 (**unknown**
    whether that is slice 1's focus marker or the play-range start).
- Play cursor: white.
- With **no sample** the overview is hidden, buttons 5–8 are disabled, and the knob-touch
  icons are off.

## 6. ZONE tab

### 6.1 Display buttons 5–8

| Button | Label | Action | Enabled |
|---|---|---|---|
| 5 | **REMOVE** | Removes the focused zone | when the Sampler has at least one zone |
| 6 | **ADD** | Opens the sample browser to load a sample into a new zone (§7.4) | always |
| 7 | **PREVIOUS** | Focuses the previous zone | when the focused zone is not the first |
| 8 | **NEXT** | Focuses the next zone | when the focused zone is not the last |

The capture (one zone) shows REMOVE and ADD enabled, and PREVIOUS and NEXT disabled.

### 6.2 Knobs, pads, screens

- Knobs 1–4: the app's ZONE parameter pages, 4 slots each. The capture shows "PLAY RANGE
  1/5" (START 0.000 s, END 0.219 s). Knob 5 zoom, knob 6 scroll.
- Pads: sound mode, as on EDIT. Pad LEDs are the normal sound LEDs.
- **Left: the zone map** `[0,60 480×144]`, with no keyboard strip on the hardware.
  - Horizontal axis: the 128 keys (3.75 px per key). The octave lines in the capture fall
    every 45 px.
  - Vertical axis: velocity (**unknown** direction, presumably high at the top).
  - Grid background: white-key columns 53,53,53, black-key columns 47,47,47, key lines
    38,38,38, octave lines black.
  - Each zone is a rectangle over its key and velocity range:
    - unfocused: fill in the sound colour at alpha 50, 1 px border white alpha 90;
    - selected: fill alpha 90, white border;
    - focused: fill alpha 150, white border;
    - zone text (if any) has 5 px padding.
  - The capture's single full-range zone covers the whole map: fill 223,3,29, 1 px white
    border, with the octave lines faintly through it (221,1,27).
  - The bar behind the map is 160,160,160, normally covered by the map.
- **Right**: the focused zone's sample in the detail view (§2.3), the same view as EDIT.
- No sample: the knob-touch icons are off, and the waveform area and info bar are empty.

## 7. Other sub-pages and states

### 7.1 Tab buttons 1–4

They always select the tab, unless a sub-page is open. With an Audio plug-in, tabs 3 and 4
are hidden. Their LEDs: the selected tab is bright, the visible others dim.

### 7.2 Sound quick-edit

On RECORD and SLICE the controller's sound quick-edit is not allowed. On EDIT and ZONE it
is.

### 7.3 Footswitch

The tip and the ring both drive the looper (press to toggle, as on the other Studio/MK3
pages). "Detect" does nothing.

### 7.4 ZONE ▸ ADD: the sample browser

This is the BROWSE page (`docs/controller-browse-page.md`) in a sampling variant:

- it shows one-shot samples;
- there is no quick-browse;
- button 5 (PREVIOUS) is hidden;
- button 6 reads **CANCEL** and closes the browser, back to ZONE;
- loading the focused result loads it into a **new zone** of the focused Sampler and returns
  to ZONE;
- changing the focused sound closes the browser;
- the wheel and PREV/NEXT go to the browser while it is open.

## 8. Slice Apply dialog (SLICE ▸ APPLY…)

**Opening.** It opens with the focused group and the focused sound preselected as the
target, so OK is enabled at once. Changing the focused sound from anywhere closes the dialog
and returns to SLICE.

**Screens.** The standard "grid" screen:

- Left display: the 8 group buttons of the current group bank, as on other group-pick
  screens.
  - An existing group shows its name and is enabled.
  - The slot right after the last group shows "+", meaning a new group.
  - The rest are blank and disabled.
  - The target group is drawn focused.
- Right display: a 4×4 grid of the target group's 16 sound slots. Each shows the sound's name,
  or "Sound n" for an empty slot. The target sound is drawn focused. Pad level meters are on.
- Info bar: the dialog's own (group info).

**Buttons.**

| Button | Label | Action |
|---|---|---|
| 1, 2 | blank | — |
| 3+4 | arrow pair ◀ ▶ (no label text) | Previous/next group bank, within the banks that exist |
| 5 | **SINGLE** | Toggle. When on, only the focused slice is applied. It does nothing when the sample has exactly one slice. Shown selected (LED bright) when on. It starts off and keeps its state between openings. |
| 6 | blank | — |
| 7 | **CANCEL** | Closes the dialog and returns to SLICE |
| 8 | **OK** | Applies, then closes. Enabled when a target group or sound is set. |

**Group buttons A–H** pick the target group (an existing one, or the "+" slot). This clears
the target sound, so the target becomes "the whole group".

**Pads** pick the target sound (pad n = sound n of the target group). With no target group a
pad does nothing.

**What OK does:**

- Target group, no sound:
  - SINGLE off: the slices are spread over the group's sounds, one slice per sound.
  - SINGLE on: the focused slice goes to the group's first sound.
- Target sound:
  - SINGLE off: the slicing is applied to that sound.
  - SINGLE on: only the focused slice goes to that sound.
- The SLICE knob page 3 (Apply) settings (mono, pattern, BPM) apply as set.

**LEDs.**

- Pads: the target sound's pad blinks (4 ticks on, 4 off). The other pads are dim while a
  target group is set. Colours are each sound's colour in the target group, or white for
  empty slots.
- Groups: existing groups and the "+" slot are dim, in their colours. The target group blinks
  when no sound is targeted.

**Knobs and LEFT/RIGHT** do nothing: no parameters, and the touch list is cleared.

## 9. LEDs

### 9.1 SAMPLE button

| State | LED |
|---|---|
| SAMPLING is the top page | bright |
| SAMPLING is open under something else (a hold-to-show page on top) | dim-flash |
| colour while the recorder is waiting or recording | red |
| colour otherwise | white |

### 9.2 Display buttons and LEFT/RIGHT

- **Display-button LEDs**, for every button:
  - visible and selected: bright;
  - visible and enabled: dim (bright while held);
  - disabled or hidden: off.
  - Exceptions: RECORD ▸ WAITING flashes (§3.1); a disabled button held down stays off
    (unless it was enabled when the press began).
- The captures match:
  - RECORD with no takes: 1 bright, 2–4 dim, 5 dim, 6–8 off.
  - EDIT: 5, 6 and 8 dim, 7 off.
  - SLICE: 5–8 dim.
  - ZONE with one zone: 5 and 6 dim, 7 and 8 off.
- **LEFT/RIGHT**, on RECORD, EDIT and ZONE:
  - they move to the previous/next of the app's parameter pages for that tab (a "page offset"
    on the shared sampling parameters, so the software view follows);
  - LEFT is dim when the current page is past the first, RIGHT when there is a later page,
    and each is bright while held;
  - if a page vanishes (a second page empties) the page steps back by itself.
- **LEFT/RIGHT on SLICE** step through the three local pages (§5.2), clamped at the ends, with
  the same LED rule.
- While the Slice Apply dialog is open, LEFT/RIGHT are ignored. While the browser is open they
  go to the browser.
- **Group LEDs** keep their normal behaviour, except in the Slice Apply dialog.

## 10. Knobs 5–8: zoom, scroll and touch icons

- **Knob 5, zoom.**
  - Each detent multiplies the zoom by 1.06 raised to the power (100 × the encoder's step
    value), so a positive turn zooms in and a negative turn zooms out.
  - The zoom is limited to 1× (the whole sample) up to the view's maximum.
  - The view stays centred on its focus frame: the new offset is that frame minus half the
    new visible length, clamped to the sample.
  - SHIFT makes it 10× finer.
- **Knob 6, scroll.**
  - Each detent moves the view by 2 × the visible length × the step value, clamped so the
    view stays inside the sample. The focus frame then becomes the middle of the view.
  - SHIFT makes it 10× finer.
- **Which view they move.** Zoom and scroll belong to the view of the focused sample (on
  RECORD, the focused take's view), which the software Sampling view also uses. They are not
  recorded as undo steps. With an empty or zero-length sample they do nothing.
- **Touching knobs 5–8** shows two icons in the overlay layer, while the tab's condition
  holds (§3–§6):
  - the zoom icon (magnifier) at right-display (46,250);
  - the horizontal-scroll icon at (166,250).
  - Each is 24×18, so they sit at the bottom edge above knobs 5 and 6.
  - There are no vertical zoom or scroll icons on this page.
  - Releasing every knob hides them.
- **Touching knobs 1–4** gives the usual touch overlay for the touched parameter (the value
  list for list-type parameters). It is cleared while recording on a page that is disabled
  while recording, and in the Slice Apply dialog.

## 11. Other buttons

| Control | On this page |
|---|---|
| SHIFT | Redraws the buttons with their SHIFT labels (RECORD: TAKES and BANK; SLICE: BANK), fine knob steps (zoom, scroll, slice START/END, parameters) |
| WHEEL (turn, press), PREV/NEXT | Only used while the ZONE ▸ ADD browser is open; otherwise they keep their global meaning (**unknown**: no page-specific use) |
| BACK, ERASE, ENTER | Not handled by the page (global behaviour only) |
| Group buttons | Normal, except in the Slice Apply dialog (§8) |
| Level/threshold encoder and input-source buttons | Studio only (the MK3 has none). On the Studio, in Detect mode the level encoder nudges the threshold by 1 per detent, and the source buttons pick the external stereo or mono input unless recording. |
