# Maschine 3 main window: UI layout spec (for the egui clone)

These are facts taken from the NWL skin style sheets in the user's own install
(`out/ni-res/maschine_nwl_resources/Maschine/` and `shared_nwl_resources/Shared/`).
Every `$variable` has been resolved. We ship no NI assets. Where NI draws a widget
from a bitmap, this spec gives the size, and the colour sampled from that bitmap, so
we can draw the widget as a vector shape.

Colour notation: `#RRGGBB`, or `#RRGGBB/aNN` where NN is alpha out of 255.
"Group colour" means the 16-colour palette in §8, indexed by the group's or sound's
colour slot.

```
+--------------------------------------------------------------------------------+ margin 3, bg #000000
| HEADER 33px  [logo][menu][browser][mixer] | [rew][play][loop][rec] (metro tempo |
|              link swing 4/4 | position | follow perf kbd)  ... [ctrl][meter][cpu][NI]
+-------+--+---------------------------------------------------------------------+ 2px gap
|BROWSER|LP| COLS 163 | SONG PANEL: Ideas view OR Song view (arranger)         |
|324px  |32| title 50 | 167/255/343 px tall (S/M/L), min 147, max 600         |
|(opt.) |  | groups   |                                                          |
|       +--+----------+-------------------------------------------------------- + 2px
|       |LP| CHAIN / PARAMETER AREA  140px (192 maximised)                       |
|       +--+----------+-------------------------------------------------------- + 2px
|       |LP| COLS 163 | GROUP PANEL (min 340): header 38 + timeline 19          |
|       |  | title 56 | pattern editor grid, 24px rows                          |
|       |  | sounds   |                                                          |
|       |  |----------|---- 7px splitter -------------------------------------- |
|       |  | mod list | velocity/modulation lane (min 85)              |16px|  |
+-------+--+----------+---------------------------------------------------------+
LP = 32px left icon strip. Column manager | editor are separated by a 1px #000000 splitter.
```

## 1. Window and macro layout (TopLevel.txt, Maschine.txt, Metrics.txt, Splitters.txt)

- **Window presets (w×h):** small 1129×631 (or 1194 wide with the browser open), medium 1210×758, large 1500×930. Minimum height is 631.
- **Root:** `pMain` has bg #000000, a 3px margin and 0 spacing. Below it, the body is also #000000.
- **Vertical order (confirmed):** Header, then `pBody`. Inside the body, the browser sits on the left and the `m_pStandardView` work area fills the rest. The work area has 3 rows:
  1. `m_pSongPanel`
  2. `m_pLowerArea`. This is the chain or parameter area (`m_pChainPanel`). In sampling mode, `m_pSamplingPanel` occupies this area instead.
  3. `m_pGroupPanel`

  Rows are 2px apart (`m_pLowerArea` spacing 2, `$spacingPanel` 2). The header has 2px of `outer-margin-bottom`. The work-area min width is 256. The mixer (`m_pMixerVisible`) swaps the work view; it is not stacked. A dev-only footer exists but should be ignored.
- **Song panel height:** small 167, medium 255, large 343. Min 147, max 600. Its default define is 280 (min 206).
- **Chain panel height:** 140 normal, 192 maximised. Min width is 675 (8 controls).
- **Group panel:** min 340.
  - The sound list (`m_pSoundManager`) has min height 168. The column manager overrides this to 173.
  - The pattern editor upper part, including its timeline, has min 188.
  - The left upper part has min 222.
  - The modulation or velocity lane has min 85.
- **Column manager** (`m_pColumnsManager`): fixed width **163**. It is the left column of both the song panel and the group panel.
- **Left icon strip** (`pLeft`): 32 wide.
  - In the song panel it is #212121. It holds the Ideas/Song toggle (`m_linearArrangerVisible`), a 32×24 icon at top margin 6.
  - In the group panel it is #181818 with top margin 7. Top to bottom it holds 32×24 icon buttons, with 4px gaps and 32×5 separators between them:
    - pad/list menu
    - piano-roll toggle
    - sampling toggle
    - velocity button (lower section)
    - modulation button (lower section)
- **Browser:** width 324, bg #181818, sits left of the work area.
- **Splitters:** generic splitters are 2px #000000. Panel splitters (`Splitter.Panel`) are 3px #404040. Inside the song and group panels, the column/editor splitter is 1px #000000. The splitter between sound list and modulation, and between grid and modulation, is a 7px grab zone drawn as a 1px line (offset −3).
- **Metrics grid:** 6 / 24 / 64 / 96. Group spacing 3. Standard track height 24 (48 when zoomed). Track content height 20 (41 when zoomed).

## 2. Header bar (Header.txt; sizes from the `HDR_*` bitmaps)

- **Height:** 33 (the bitmap height). The background is a flat **#181818** bitmap with 300px fixed ends, then 2px below. Vertical separators between sections are 3×33.
- **Frames:** buttons use 7-frame bitmaps. The likely frame order is off, on, off-hover, on-hover, off-pressed, on-pressed, disabled. Icons are white glyphs whose alpha changes with state: idle ≈a143, hover ≈a163–245, disabled a61.

Left to right, in style-sheet order:

| Item | Size (w×h) | Notes |
|---|---|---|
| Maschine logo | 157×22 | margins: top 5, left 6, right 23. A picto version is used when the window is narrower than 1299. |
| Main menu (`m_pOpenMenu`) | 32×33 | dropdown menu, offset (−9, +3) |
| Browser toggle | 32×33 | |
| Mixer toggle | 32×33 | |
| Transport bar | margin-top 4 | |
| — Rewind/restart | 32×24 | |
| — Play | 32×24 | **on = #BDF164** (green); off = white a143 |
| — Loop | 32×24 | white glyph |
| — Record | 32×24 | **on = #E3423F** (red). A "record perform" variant exists. |
| Transport display box | 27 high | rounded dark box with 20px fixed ends. Margins 2, outer-margin-top 2. Internal separators are 3×24. |
| — Small caps labels (BPM, SWING…) | — | font 10 Medium, #FFFFFF/a142, 8px right gap |
| — Metronome | 20×20 | margins: left 4, right 5, top 4 |
| — Tempo (`m_pTempo`) / scene tempo | min 46 | value font 12 Medium #FFFFFF/a245, height 24. Scene tempo is 39×26 in the group colour (Pragmatica 12), with a 39×2 colour bar. |
| — Link status | 48×24 | off: bg #181818, text a142. On: bg #323232, text a245, font 10. Phase meter 46×23 on #323232. |
| — Swing | 40 | |
| — Time signature | 18 + "/" + 18 | "/" is font 14. Disabled digits are a142. |
| — Position display (`pTransportDisplay`) | min 80 | Value 56×32, font 12, left-aligned. Type label 32. **Scene-name button** 88×24, font 12, group colour. |
| — Follow playhead | 20×12 | margins: left 8, right 6 |
| — Perform menu button | 44 | font 12, a245 |
| — Computer MIDI keyboard | 24×24 | |
| Right bar: controller icons | 33 (bitmap 40×33) | one each for MAS / KK / Jam |
| Master meter box | 96 wide | Meter 74×19, 2 channels with a fader grip, 2px spacing. |
| CPU meter | 11×11 | bg #FFFFFF/a61, fill #FFFFFF/a245 |
| Engine (bypass) button | 32×33 | |
| NI logo | — | margins: left 7, right 8, top 4 |

Small widths: the logo collapses below 1299px of width, and the engine button is hidden below 1157.

## 3. Song panel shared parts (SongPanel.txt, SongGroupPanelShared.txt)

- Panel bg #000000. The editor column bg (`pColumns`) is **#212121**.
- **Column title** (`pTitle`) is 50 high:
  - `titleTop` 32: info label, Roboto Regular 15, left margin 8.
  - `titleBottom` 20: scene-layer toggle.
  - A popup button sits at the right (margins: top 12, right 8).
- **Column footer:** #212121. It holds:
  - grid-snap toggle, 24×24
  - grid value text, 40 wide, font 12 #FFFFFF
- **Group list (Song view)**, `m_pGroupVector` on #212121:
  - **Row height 24**. Row bg #212121, **#292929 when focused**. There is a 4px darker bottom edge in the bitmap.
  - Layout, left to right:
    - `m_pBite`: 4px colour strip in the group colour. Uses the highlight colour from palette 13 while MIDI is active, and the mute colour from palette 4 when muted.
    - **Mute/ID button**: 28 wide, min height 23. Shows the group ID ("A1") in Roboto Medium 12 #FFFFFF/a164, left margin 8.
    - **Name label**: #FFFFFF/a142, left margin 10, vertically centred.
    - Level meter: 5×21, two 2×16 bars in the group colour.
  - Focused row: the ID button gets a **group-colour bg with #181818 text**, and the name turns #FFFFFF.
  - Muted rows: ID text #666666 and name #FFFFFF/a61. A focused muted row uses the palette-4 bg.
  - Unfocused but selected: ID button bg #262626.
  - "+" add row: 160 wide. When focused or drag-hovered its bg is #4F4F4F.

## 4. Ideas view (IdeaSpace.txt)

Groups are **columns, 99px wide**, a tile of the group header sits at the bottom, and
scenes run along the top. Every horizontal list uses item width 99, with a 27px "+"
item at the end.

- **Root:** bg #000000, spacing 0. The scene area bg is #181818.
- **Scene row:**
  - `m_sceneList` is **32 high**, bg #212121.
  - Scene tabs are 99–100 wide and 32 high, overlapping −1. Inner margins: 6 left/right, 4 top/bottom.
  - Label: Roboto Medium 12, text in the scene colour.
  - Tab bitmap colours: tab #262626; hover/selected #3C3C3C with a 1px #181818 edge.
  - A selected tab is filled with the scene colour.
  - The rename edit box is 92×20 on #1E1E1E.
  - Left and right nav arrows appear when the list scrolls.
- **Scene focus line:** 3px #3C3C3C under the scene row.
- **Pattern area:**
  - bg #212121, 1px above it.
  - Each group column holds a vertical list of pattern cells, **98×24**, text indent 7.
  - The focused column gets a #FFFFFF/a24 wash.
- **Pattern cells:** the shape is a rounded rect, radius about 3px, inset about 5px horizontally and 3–4px vertically inside the 24px cell. States:
  - Empty: nothing, or a #FFFFFF/a41 outline on hover/selection.
  - Exists but not in this scene: group colour dimmed by a #171717/a133 overlay (about 52%). Text #000000/a245.
  - **Assigned to the focused scene:** full group colour, text #000000/a245.
  - **Focused/selected:** full group colour plus an approximately 2px #FFFFFF/a235 outline.
  - Muted group: #4A4A4A (unassigned) or #808080 (assigned), text #000000/a164.
  - Rename edit box: 86×16 at (6, 3), text #FFFFFF on #000000/a164.
- **Group header tiles** (`m_groupList`): **99×56**, area bg #212121. Contents:
  - Top row 16 high, 4px from the top. It holds the **group letter in Roboto Black 16**, centred, min width 16, drawn in the group colour.
  - Name label 86×24, #FFFFFF/a245, or a142 when muted.
  - **Progress strip** 86×4. It shows up to 8 bar segments with 2px gaps. Colours:
    - filled: group colour
    - unfilled: group colour at 25% opacity
    - no pattern: #262626
    - gaps: #181818
  - Focused tile: **filled with the group colour**. Letter and name turn #000000/a245. The progress strip turns black: filled a245, unfilled a61.
  - Selected but not focused: bg #FFFFFF/a61.
  - Muted: grey. Filled #7D7D7D, unfilled #323232, letter in the palette-4 colour.
- **Scrollbars:** the horizontal one is 21 high. The footer spacer is 36 high, or 48 when a scrollbar is shown.
- **Playing scene:** the style sheets have no dedicated "playing" colour. The playing scene shows as the selected tab filled with the scene colour, and the header's scene-name button in the scene colour.
- **Ideas/Song toggle:** the 32px left-strip button (`m_linearArrangerVisible`). The scene-layer toggle is in the column title.

## 5. Song view / arranger (Arranger.txt)

- Editor bg #212121, min 16 bars, playhead a 1px #FFFFFF line.
- **Timeline** (`SongTimeline`):
  - 18 high, bg #212121, 1px #000000 rules top and bottom.
  - Font Medium 12, text offset (6, −1).
  - Bar and beat lines #FFFFFF/a40. Numbers #FFFFFF/a61.
  - **Loop range** fill is #FFFFFF/a20 when loop is off and #FFFFFF/a40 when it is on. Inside the loop, bar lines are a92 and numbers a164 when on, or a142 when off.
  - A scene-end drag handle sits on the timeline.
- **Section/scene bar** (`ArrangerHeader`):
  - **32 high**, bg #212121. Out-of-range bars are #FFFFFF/a40.
  - Scene clips have bg #323232 and border #323232. A focused clip is #464646 with border #4F4F4F.
  - Divider between scenes: 1px #000000. A 3px focus line marks the focused scene in the scene colour.
  - Text is Medium 12, with margins 7 left and 8 right. Truncation indicator 12px. There is a "+" add-scene button.
  - Column title 50 = this bar (32) + the timeline (18).
- **Tracks** (`ArrangerTrack`):
  - **26 high with −2 spacing, so the row pitch is 24.** This lines up with the 24px group list rows.
  - bg #212121, row separators #FFFFFF/a10. A focused group's row gets a #FFFFFF/a12 highlight.
  - The area after the scene end is #181818.
  - Grid lane: y 1, h 24, bar and beat lines #FFFFFF/a20.
- **Clips / patterns:**
  - Rounded rect with **radius 3**; the mask alpha ramps over 3px. Filled with the group colour.
  - Text #000000/a245, Medium 12, margins 7–8.
  - Bitmap frames distinguish first, middle and last iterations of a looped pattern, plus focused and muted states. Muted clips are drawn grey.
  - Selected-clip overlay: 20 high, #2B2B2B/a179.
- **Scene-mode shade:** #000000/a50. The height-compensation spacer is #181818.

## 6. Group panel and pattern editor (GroupPanel.txt, PatternEditor.txt)

- Panel bg #000000. The column bg (`pColumns`) is **#181818**.
- **Sound list title** (`pTitleManagers`): 56 high.
  - Top 32: group name, Roboto Regular 15, in the group colour, left margin 8.
  - Bottom 24: pre-hear and list/pad mode buttons.
- **Sound rows:** same widget as the group list (§3), on #181818.
  - **Height 24** (48 with y-zoom). The ID/mute button is centred and shows the pad number.
  - With y-zoom, the meter and name move down 10–11px.
- **Pad view (alternative to the list):** 4×4 grid with 2px spacing and 1px margin.
  - Pad colour: group colour at 50%. A selected, focused pad uses the full colour. Muted pads use the palette-4 colour.
  - Pad settings text is font 11, #FFFFFF/a142 for labels and a245 for values.
- **Column footer:** grid toggle 24×24, snap value 40 wide (font 12 #FFFFFF), pencil 24×24.
- **Pattern editor header** (`m_pPatternEditorHeader`):
  - **38 high**, bg #181818, font 10.
  - Pattern name in the group colour, left margin 8.
  - "Length"/"Start" labels in #FFFFFF/a61. Values are font 12 #FFFFFF in a 46-wide field.
  - MIDI and audio drag buttons on the right.
  - A clip-lane variant (`ClipEditorHeader`) is 39 high, with clips and patterns in the group colour (radius 3) and #000000 text.
- **Timeline row:** `m_pHeader` is 19 high on #000000. It holds `PatternEditorTimeline`, which is **18 high** at top offset 1, bg #181818, font Medium 12, text offset (6, −1). Line and text colours:

  | Element | Inside pattern | Outside pattern |
  |---|---|---|
  | Bar lines | #FFFFFF/a92 | #FFFFFF/a40 |
  | Beat lines | #FFFFFF/a40 | #FFFFFF/a20 |
  | Numbers | #FFFFFF/a142 | #FFFFFF/a61 |

  The active pattern range is filled #262626, or #181818 while growing.
- **Header alignment:** header 38 plus timeline row 19 is about the 56px sound-list title, so grid rows line up with sound rows.
- **Drum grid** (`PatternEditorGrid`):
  - bg **#181818**, **row height 24** (48 zoomed), no spacing between rows.
  - Vertical line colours:

    | Line | Inside pattern | Outside pattern |
    |---|---|---|
    | Step | **#212121** | #1C1C1C |
    | Beat | **#484848** | #2A2A2A |
    | Bar | **#8A8A8A** | #4B4B4B |

  - Past the pattern end, a #000000/a80 overlay darkens the grid.
  - Horizontal lines: outer border #FFFFFF/a12, inner light line #181818, and a **separator every 4 sounds** in #FFFFFF/a24.
  - The focused sound's row is highlighted #FFFFFF/a12. The selection rectangle is #000000. The play marker is #FFFFFF.
- **Notes:**
  - Group or sound colour. Fill alpha scales with velocity from **80 (velocity 0) to 255 (velocity 127)**. No border.
  - Drum-grid notes are inset 2 at the top and 3 at the bottom, so 19 high in a 24 row. Zoomed MIDI rows are inset 12/14.
  - Selected: 1px #FFFFFF outer border, #181818 inner border, and a #FFFFFF/a40 overlay.
  - Muted notes #4A4A4A. Disabled notes use the palette-4 colour; disabled and muted is #323232.
- **Piano roll:** min height 100.
  - Lane colours: white-key rows #212121, black-key rows #181818.
  - Keyboard: white keys #9B9B9B, black keys #181818, lines #181818. Keys in range are #FFFFFF/a200. Octave labels are #000000.
- **Velocity / modulation lane** (`ModulationEditor`):
  - bg **#121212**. Vertical lines use the same colours as the grid. Horizontal guide lines are #FFFFFF/a6.
  - Events (velocity bars or automation points) are in the group colour. Selected events are #FFFFFF; muted events #404040.
  - The pattern-end line is #FFFFFF.
  - Tooltip: #000000/a164 text on #FFFFFF/a245.
  - Lane margins: 7 top and bottom. Font 10.
  - The clip-mode variant has bg #262626 and guide lines #383838.
  - The body sits 6px below the splitter, with a 5px inner top margin. A 16px #181818 placeholder sits on the right.
  - Editor foot: 24 high.
- **Modulation list (left, under the sound list):**
  - bg #181818, margin-left 3.
  - Rows 20 high with 1px spacing, margins 6 top and 4 left.
  - Names #FFFFFF/a164. A selected name is #FFFFFF on #FFFFFF/a40.
  - Scale bar: 52 wide, font 11, #3C3C3C, right-aligned.

## 7. Parameter / chain area (ParameterArea.txt, skim)

- Height 140 (192 maximised). Panel bg #000000 with 1px spacing. Column and editor bg #212121.
- **Left column (163 wide):**
  - Tab row with Master (48), Group (42) and Sound (43) tabs, font 12. Text is #FFFFFF/a142, a164 on hover, a245 when selected.
  - Title 32: info label, font 15.
  - Modes stack 88 high: the plugin slot list, with 24px rows and 2px spacing. Slot names are a164, a61 when inactive, a245 when focused.
- **Parameter editor:**
  - Header 32. Page tabs are 24 high (a142, or a245 when selected). Preset name is font 12, a245.
  - Section row 24, spacing 13.
  - Parameter row 84 high. Each parameter widget is 70×84 with 13 spacing; the control stack inside is 56. Enum values are font 14. The "Apply" button is 54×24.

## 8. Colours and fonts (Colors.txt, Shared/SW Colors.txt, Fonts.txt)

- **Surfaces:** black #000000; Surface00 #181818 (panels, header, grids, text-edit bg); Surface05 #212121 (song panel and list bg); Surface01 #262626; Surface02 #323232; Surface03 #3C3C3C; Surface04 #4A4A4A. Legacy: group #404040, panel #303030.
- **Text on dark:** high #FFFFFF/a245, medium a164, low a142, sub a61. Text on colour uses the same alphas in #000000.
- **Strokes:** sub #FFFFFF/a20, low a40, medium a92. **States:** hover #FFFFFF/a20, active a40, pressed a82. Grid overlays are #FFFFFF at a6, a10 or a12.
- **Group colour palette** (palette 1, as drawn in the UI; slot 17 is white):
  1 #FF423E, 2 #FF7442, 3 #FC9824, 4 #FFDD1A, 5 #FFFF19, 6 #A3FF18, 7 #53FF53, 8 #19EAA3,
  9 #19EAEB, 10 #18CEFF, 11 #4D94FF, 12 #8472FF, 13 #B66EFF, 14 #DF5FFF, 15 #FF53F0, 16 #FF4880.
- **Derived palettes:** 4 = mute, about 52% of palette 1 (slot 1 = #842C2A); 13 = MIDI-activity highlight, lighter (slot 1 = #FF605C); 9 = pads at 50% opacity; 16 = 25% opacity; 6 = 20% opacity; 24 = 35% colour mixed into #181818.
- **Fonts:** the UI font is **Roboto (CJK SC) Medium**. Titles use Regular, Ideas group letters use Black, scene tempo uses Pragmatica.
- **Font sizes:** **12** default (labels, values, clips, timelines); **15** panel titles (Regular); **10** header captions, pattern-editor header, modulation lane; **11** pad view, modulation scale, assignment items; **14** time-sig "/", enum values, notifications; **16** Ideas group letter (Black). Other tokens: caption 9, overline 10, body 12, title 15, browser tabs 14.
