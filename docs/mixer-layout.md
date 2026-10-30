# Maschine 3 Mixer view: layout and behaviour spec (for the egui clone)

Sources:
- `out/ni-res/maschine_nwl_resources/Maschine/Panels/Mixer.txt`: the NWL style sheet, 1941 lines; quotes below are from it unless noted.
- `Definitions/Colors.txt` and `Panels/Header.txt`.
- The mixer bitmaps in `maschine_pictures_nwl_resources/Maschine/Mixer/`. These were measured only. We draw our own shapes and ship no NI bitmaps.
- `out/gui/bindings.tsv`: which document functions the UI classes call.
- The project schema (`crates/mxprj/src/types/group.rs`, `song.rs`).
- The NI manual's chapter "Controlling your mix".

The notation follows docs/ui-layout.md: colours are `#RRGGBB` or `#RRGGBB/aNN`, "group colour" means palette 1, and "mute colour" means palette 4 (§8 there).

- **[exact]**: read from a style sheet, a bitmap, the schema or the manual.
- **[inferred]**: a reasonable reading.

## 1. Where it lives

- **[exact]** The header's mixer button (`Bar#m_pHead Button#m_pMixerVisible`, a 32×33 icon `HDR_BTN_Mixer`) toggles the Mixer.
- **[exact]** The Mixer replaces the work view (`m_pMixerView`). docs/ui-layout.md §1 says it isn't stacked with it.
- **[exact]** Per the manual ("Mix view"), the Mixer takes the upper part. The plug-in strip (the chain/parameter area) stays below it.
- **[inferred]** For us: the Mixer replaces the song panel, the chain panel keeps its 140px, and the group panel goes away. The Mix view is the Mixer plus the chain area below it.
- **[exact]** `Bar#m_pMixerView` background: Surface00 #181818.
- **[exact]** The channel container is `m_pChannelManager`, background Surface05 #212121.
- **[exact]** The channel body area is `m_pChannelArea`, #212121, `outer-margin-top: 1`.

Horizontally, left to right:

```
[sidebar 32 | 2px][ channel strips (scroll H) ............ ][shadow 8][ master/group strip ]
```

## 2. Left sidebar `m_pVisibilityArea` [exact]

- **Size:** 32 wide, `spacing 1`, `outer-margin-right 2`.
- **Background:** #212121, with a 1px contrast line image down the right (`Shared_BG_SidepanelContrastLine`; draw a 1px #000000 line).
- **Upper part:**
  - A spacer `m_pLengthAdjustment`: 25 high when collapsed, 62 when expanded.
  - Then `m_pShowHideFunctionalBlocks`: **Extend/collapse** (`MIX_BTN_Extend`, a 32×20 icon, 7 states).
  - Then a 1px #000000 divider (`outer-margin-top 2`).
- **Lower part** (spacing 2), toggle buttons:
  - `m_pShowHideInOut`: **IO**, show inputs/outputs (`MIX_BTN_SP_InOut`, 32×16). `outer-margin-top 3`.
  - `m_pShowHideSlotMatrix`: **Plug-ins**, show the plug-in list (`MIX_BTN_SP_Plugin`, 32×20).
  - `m_pShowHideSend`: **AUX**, show aux sends (`MIX_BTN_SP_Aux`, 32×16).
- **Toggle states [inferred]:** buttons use the usual 7-state strip: off, hover, on, on-hover, and so on.
- **Behaviour:**
  - **Collapsed:** each strip shows only its 40px header, with a mini level meter. [manual]
  - **IO** shows the audio-in/MIDI-in and MIDI-out sections. The main audio output stays visible. [manual]
  - **Plug-ins** shows the slot list. **AUX** shows the two aux sends. [manual]
- **Persistence:**
  - [exact] The Song stores `sounds_visible_mixer` (group or sound level) and `functional_blocks_visible_mixer` (extended or collapsed) as bools.
  - [inferred] The next three Song bools, `f308`, `f310` and `f318`, are probably the IO, plug-ins and AUX visibility, in sidebar order. Confirm before writing them.

## 3. Strip anatomy, top to bottom (group or sound strip)

Every strip is **71 wide**. Its body background `Bar.ChannelBackground` is 72 wide (73 when `HasSeparator`), which includes a 1px right stroke drawn from `MIX_BG_ChannelRightStroke`. Draw that stroke as 1px #000000; a group boundary at sound level gets a 2px separator.

| # | section (id) | height | contents |
|---|---|---|---|
| 1 | Header `ChannelHeadRow` (group) / `SoundChannelHead` (sound) | **40** | row 1 (`pIdAndLevelBar`, 20 high, margin-left 4): **ID** label 37×20 (Roboto Medium **14**: group letter+number "A1", sound number "1"), then the **mini meter** 2px wide (outer-margin-left 21; master 60). Row 2: **name** label 71×20, font **12**, margin-left 4, top 1, left-aligned |
| 2 | Splitter `m_pHeadBodySplitter` | **3** (+1 gap) | the group colour bar (palette 1); mute colour when muted; 1px #000000 when there are no groups |
| 3 | Input `m_pInput` (IO only; margin-top 1, spacing 1) | sound: 21+21+margins ≈ **48**; group: MIDI only | **Audio input** (sounds only) `m_pAudioInput` 71×21 (margin-top 4, bottom 1, left 1, right 2): a small knob 17×17 (`MIX_KNOB_Small*`, 129 frames; input gain) and the device button `m_pAudioInDevice` 50×17, right-aligned, font 10 ("AUDIO IN" or the port). **MIDI input** `m_pMidiInput` 71×21 (margin-top 2, left 1): the channel button `m_pMidiInChannel` 18×17 (centred, font 10) and the device button `m_pMidiInDevice` 50×17 (right-aligned, margin-right 4). Groups also have a kit-mode button `m_midiInputKitMode` (65×17 routing-button shape, font 10, left) |
| 4 | Plug-in slots `m_pSlotMatrix` (Plug-ins only) | min **99** (vertical scroll) | one row per slot, 71×20 (`MIX_BG_PluginSlotsListitem`): a **bypass square** `m_pActive` 12×20 (`MIX_BTN_PluginSlotsActive`, 7 states: active/bypass × focus), then the **name** `m_pName` (max-width 59, 20 high, left; font 12). Last row: **"+"** `m_pAddSlot` 71×20. Frame `MIX_BG_PluginSlots` (fixed top/bottom 14). Vertical scrollbar 12 wide (margins 8/8/2/2) |
| 5 | Flex filler `m_filler` | rest | empty; pushes the level module down |
| 6 | Level module `m_pLevel` | **165** | see §4 |
| 7 | Main output `m_pAudioOutput` (inside `m_pLevel`, always shown) | **20** (outer-margin-top 1) | the routing button `m_pAudioOutDevice` 65×17 (`MIX_BTN_ChannelRouting`, outer-margin top 3 / left 3, margin-left 5, font 10, left): "Group", "Master", "Ext 1/2", … |
| 8 | Aux `m_pAuxBar` (AUX only) | **62** (margin-left 1, top 3) | ×2 (Aux 1, Aux 2): a 71×17 bar (`MIX_BG_AuxSend*`) with the **destination** button `m_pAux1`/`m_pAux2` 51×17 (margin-left 6, font 10, left; "AUX 1" when unassigned) and a **knob** `m_pAux1Knob` 17×17 (send level). Under each, a **Pre/Post** button `m_pAux1Button` 71×12, font 10, left, margin-left 6; shown only when assigned (text #181818 = invisible otherwise, white when assigned) |
| 9 | MIDI output `m_pMidiOutput` (IO, sounds only) | **22** (margins top 3, left 2, bottom 2) | device button `m_pMidiOutDevice` 50×17 (font 10, left, margin-left 4) and channel button `m_pMidiOutChannel` 18×17 (centred) |

**Bottom:** a horizontal scrollbar under the strips (`m_pScrollbarTracks` at group level, `m_pScrollbarSounds` at sound level).
- Size and margins: 19 high, margins left/right 5, top 1.
- Track: #000000 with a 2px inset frame (`MIX_BG_BottomScrollbar`).
- Thumb: `MIX_HANDLE_BottomScrollbar` with 4px caps.
- No inc/dec buttons (they are 1px wide).

**"Assigned" vs "unassigned" (IO, aux, MIDI) [exact]:**
- Text is #FFFFFF/a142 when unassigned and #FFFFFF/a245 when assigned. MIDI-in assigned text uses #000000/a164 on a lit button.
- Backgrounds swap to the `*Assigned` bitmaps, which are lighter fills. Draw them as rounded 2px rects: Surface02 #323232 unassigned, Surface03 #3C3C3C assigned. [inferred]

## 4. Level module `m_pLevel`, 165 high [exact]

The background is `MIX_BG_LevelMeter` (72×167): a #000000 1px frame at the top and right, the inside transparent (shows #212121), and a 1px #000000 line 142px down. That line separates the fader area (`m_pLevelSetup`, 142×71) from the monitor/mute row. Top to bottom:

1. **Pan slider** `m_pPanKnob`:
   - Track 61×5 (`MIX_BG_PanDragger`) at outer-margin left 5, top 6, background Surface01 #262626.
   - The fill uses **palette 2** (the group colour, pure). [exact] The fill runs from the centre to the value (bipolar). [inferred]
   - Hover or drag: #FFFFFF/a20. No thumb (width 0).
   - The track's centre pixel is #181818, a centre mark.
2. **Headroom / peak** `m_pHeadRoom`:
   - 71×11, outer-margin-top 2, Roboto Medium **9**, centred.
   - #FFFFFF/a61 normally; **#FFFFFF (white) once clipped** (`HasClipped`).
   - Shows the peak since the last reset (`Sound::getHeadRoom` / `Track::getHeadRoom`; master `Song::getCueHeadRoom` for cue). A click resets it. [manual]
   - [inferred] Format: dB with one decimal, "-inf" when silent.
3. **Scale + meters + fader** `pScaleAndMeter`, 71×99:
   - **Scale** `m_pLevelScale` 18×99 (`MIX_MTR_LevelScale`):
     - Labels "10", "0", "10", "20", "30", "∞" (the upper 10 is +10 dB, the rest negative), in #FFFFFF at low alpha (about a40), small digits.
     - Label centres at y = **3, 25, 47, 63, 79, 95**, i.e. dB +10, 0, −10, −20, −30, −∞ [exact]. That is 2.2 px/dB from +10 to −10, then 1.6 px/dB to −30, then −∞ at 95.
     - Use this piecewise-linear dB→y mapping for meters, fader and scale alike. [inferred: the fader shares the scale]
   - **Meter + fader container** `m_pLevelMeter`:
     - 53×100, margin-left 11, spacing 1. Background `MIX_MTR_ZeroLine`: a 1px **0 dB line** #FFFFFF/a41 at y = **26**.
     - Inside, **two meters** (L, R) `LevelMeter.Default`, each **6×96**, outer-margin-top 1.
     - Meter colour: palette 1 (the group colour); **master meters white**. Muted: `MIX_BG_LevelMeterMute` / `MIX_OVERLAY_LevelMute` draw a #000000/a102 overlay.
     - [inferred] Meter background Surface00, value from the bottom.
   - **Fader** `m_pVolumeSlider`:
     - 28×100, no background.
     - Handle `MIX_MTR_Handle` 20×8: a white (#F2F2F2) arrow pointing left, at the fader value, beside the meters.
     - Drag vertically. [inferred, usual Maschine behaviour] Shift is fine, and double-click resets to 0 dB.
4. **Monitor/Mute row** `pMonitorSoloMuteBar`:
   - 71×20, margin-left 1, spacing 16.
   - **Cue** `m_pMonitor`: a headphones icon 20×20 (`MIX_BTN_Monitor`, 7 states).
   - **Mute** `m_pMute`: a 34-wide text button, #FFFFFF/a142 off, **white when on**. The image is "no", so the label "M" or "MUTE" is drawn as text. [inferred label]
5. **Main output** (row 7 of the table above).

**Meter ballistics:** the style sheets have no meter numbers. Use the viewer's current meters: peak per frame, falloff halving every 150 ms. Draw the mini meter in the header (2px) from the same value. [inferred]

## 5. Header states and colours [exact]

| state | header background | ID text | name text |
|---|---|---|---|
| default | #212121 (group, sounds hidden or visible); sound head **#181818** | group colour (palette 1) | #FFFFFF/a245 |
| selected (`:selected`, sounds hidden) | #FFFFFF/a40 | group colour | #FFFFFF/a245 |
| **focused** (`HasFocus`) | **group colour** (palette 1) fill | #000000/a245 | #000000/a245 |
| focused + muted | **mute colour** (palette 4) fill | #000000/a245 | #000000/a245 |
| muted (not focused) | default | mute colour (palette 4) | #FFFFFF/a142 |

**Strip bodies:**
- **Focused or selected:** `#FFFFFF/a20` over #212121 (`$colorMixerChannelSelected`).
- **Muted:** sound body background #212121, and every section (input, slots, IO, aux, MIDI) gets a **#000000/a102 overlay** (`$colorMixerChannelSelectedMute`).

**Header bitmaps:**
- `MIX_BG_GroupChannelHeader*` (72×40): a 1px right stroke, with variants for sounds visible/hidden and with a separator. Draw a 1px #000000 right line.
- **Add group** `m_pAddTrack`: a 72×40 header tile (`MIX_BTN_AddGroup`, 7 states) with a "+" after the last group header. At group level, a click adds a group.

## 6. Master strip `m_pMasterChannelItem` (right, fixed) [exact unless marked]

- **Frame:**
  - Outer strokes bitmap `MIX_BG_MasterOuterStrokes` (81 wide), so the master column is **81** wide: a 71 strip + 5 + 5 margins.
  - Its body and header have outer-margin left/right **5**.
  - A shadow `MIX_SHADOW_Master`, 8 wide, sits at its left edge (a dark gradient over the strips, #000000 fading out).
  - **Collapsed:** `MIX_SHADOW_MasterCollapsed`, 8×40.
- **Header:** 40 high, outer margins left/right 5.
  - **Row 1:** the tab bar `m_pTab` (18 high, margin-left 3, spacing 7):
    - **"MASTER"** text button, 40 wide, left-aligned. Colours: #FFFFFF/a142, hover a164, selected white.
    - **Cue** headphones button `m_pCueButton`, 20×20, outer-margin-top −1.
    - When the header is focused (background **#A0A0A0**, `$colorGrey`), MASTER turns #000000 when selected and the cue icon uses the `CueFocus` variant.
  - **Row 2:** the name and mini meter (outer-margin-left 60).
- **Body:**
  - `m_pMasterBody`: Surface05, outer-margin-top 1, left/right 5. Focused: `#FFFFFF/a40` (`rgba(255,255,255,40)`).
  - Under the body sits a 19-high **scroll dummy** (#212121), aligned with the strips' scrollbar.
  - A **group input spacer** (`m_pDummy`, 21 high) holds the IO rows in line with the other strips.
  - At group level the MIDI-output area is a filler bitmap (`MIX_BG_MidiOutDummyMaster`).
- **Contents:**
  - Master: no mute, no audio/MIDI input, no aux, no MIDI out. [manual]
  - Its level module has white meters, pan, level, headroom and main output.
  - Clicking the cue icon switches the strip to **Cue**: level `Song.cue_level`, pan, destination; no plug-in list. [manual]
- **At sound level:**
  - The fixed right strip shows the **focused group** (`m_pGroupBody`) instead of the master. [manual: "the rightmost strip is that Group"]
  - Its ID label uses the same colour rules (`Bar#m_pMasterChannelItem Bar#m_pGroupBody Label#m_pGroupId`).
  - `m_pShadowBar[SoundsVisible=true]` uses animation 0.

## 7. Group level vs sound level, focus, mouse [manual unless marked]

- **Group level:** one strip per group, then the "+" add-group header. The master is on the right.
- **Sound level:** one strip per sound slot of the focused group (16). The group is on the right.
- **Switching:** **double-click the background of a group header**. The level is `Song.sounds_visible_mixer`. [exact field]
- **Clicking:**
  - Clicking a header background **focuses** that group or sound. This is our `app.group` / `app.sound`.
  - At sound level, clicking another group's header shows that group's sounds.
  - Double-clicking the ID (letter/number) **toggles mute**. Double-clicking the **name** renames it.
  - Right-click (Ctrl-click) on a header opens its context menu.
- **Mute:** click **M**. **Solo = right-click (Ctrl-click) on M**: it "mutes all other channels".
  - Solo is **not stored** as such. The project only has `mute` / `audio_mute`. [exact: schema; the only Solo symbols are `HW::Solo` / `SoloGroup` hardware modes and the Komplete-Kontrol `solo_track` action]
  - So solo writes the `mute` of every sibling (other groups at group level, other sounds at sound level) and clears the soloed one's. Clicking right-click again on the soloed channel should **unmute all**. [inferred: Maschine toggles back]
  - Mute and solo don't affect channels sent to cue.
- **Cue:** the headphones toggles the channel's `cue_enabled`. Several channels can be cued.
- **Plug-in list:**
  - **"+"** adds at the end. Right-click a slot to replace or remove it (choose "None").
  - Click a name to focus that slot in the chain area. Click the square to bypass. Drag to reorder.
- **Selection vs focus:** a click on any control of a visible strip changes it without moving the focus.

## 8. Data binding

The paths are the parameter fields the existing `Document::set` / `Target` code reaches. Mixer data is stored on the **sound** (`Target::Sound`, track parameters) and the **group's strip** (`Target::GroupStrip`, legacy track parameters). Both share `track_parameters` (group.rs).

| control | sound / group field | master (Song) | type / mapping |
|---|---|---|---|
| level fader | `level` | `level` | float, linear amplitude. dB = 20·log10(level) (`LevelScale::convertLevelTodB`). Fader range +10 dB (3.162) to −∞ (0) per the scale; **[inferred]** 0 dB = 1.0 is the default |
| pan | `pan` | `f138` **[inferred]** | float −1..1. Show "C" at 0, else "NL"/"NR" with N = round(\|pan\|·100), as the chain-area knobs already do |
| mute | `mute` (sounds also have `audio_mute`) | — | bool. **[inferred]** `audio_mute` is the sound's audio-only mute (MIDI still sent); the mixer's M sets `mute` |
| cue (headphones) | `cue_enabled` | — | bool |
| cue level | — | `cue_level` | float, linear |
| main output | `audio_output` (`AudioOutputParameter`) | `f128` (master), `f150` (cue) **[inferred]** | an output-destination enum/value; `AudioOutputParameterValue::toString` gives the label |
| aux 1/2 destination | `aux1_output`, `aux2_output` | — | `AudioOutputParameter` |
| aux 1/2 level | `aux1_level`, `aux2_level` | — | float, linear |
| aux pre/post | `aux1_order`, `aux2_order` (enum; legacy `*_order_legacy` bool) | — | **[inferred]** 0 = Post, 1 = Pre |
| audio-in gain (sound) | the sound's audio-input block gain (docs/mxprj-format.md §7) | — | float |
| plug-in slots | `chain.slots` (`Slot.active`, `module`) | Song chain | as the SOUND tab (`Document::add_slot`, `remove_slot`, `set_slot_active`) |
| group or sound level | — | `sounds_visible_mixer` | bool |
| collapsed or extended | — | `functional_blocks_visible_mixer` | bool |
| headroom / peak | runtime (`Sound::getHeadRoom`, `Track::getHeadRoom`, `Song::getCueHeadRoom`) | | not stored; reset on click |
| level tab (focus) | `Song::getLevelTab` (`tLevelTabValues`: Master/Group/Sound) | | **[exact]** runtime, read by every channel item; maps to our `ChainTab` |

The model already exposes `level`, `pan` and `muted` for groups and sounds. `cue_enabled`, the aux fields, `audio_output`, the master `level` and the Song mixer bools need adding to `deckard_model`, with patches in `Document::patch`.

## 9. Bitmaps (measured only, we draw our own)

All are in `maschine_pictures_nwl_resources/Maschine/Mixer/`. 7-state buttons are stacked vertically.

| file | size | what |
|---|---|---|
| MIX_BG_AudioInput / …Assigned | 68×17 | audio-in row background |
| MIX_BG_AuxSend / …Assigned | 68×17 | aux-send row background |
| MIX_BG_BottomScrollbar | 28×19 | scrollbar track (9-slice) |
| MIX_BG_ChannelInputGroup / …Sound | 10×22 / 10×44 | input-section frames (group / sound) |
| MIX_BG_ChannelRightStroke / …Separator | 72×50 / 73×50 | strip body right stroke (1px / separator) |
| MIX_BG_GroupChannelHeader[SI][Separator] | 72–73×40 | group header backgrounds (sounds visible / invisible, with or without separator) |
| MIX_BG_LevelMeter | 72×167 | level-module frame (#000000 lines) |
| MIX_BG_LevelMeterMute | 71×282 | muted level-module overlay, 2 states |
| MIX_BG_MasterOuterStrokes | 81×120 | master column frame, 2 states (sounds visible / hidden) |
| MIX_BG_MidiOutDummyMaster | 15×15 | filler under the master's MIDI-out row |
| MIX_BG_PanDragger | 61×5 | pan track (centre mark #181818) |
| MIX_BG_PluginSlots | 72×49 | plug-in list frame (9-slice) |
| MIX_BG_PluginSlotsActive | 16×112 | slot-active indicator states |
| MIX_BG_PluginSlotsListitem | 71×20 | plug-in list row |
| MIX_BTN_AddGroup | 72×280 | "+" add-group header tile, 7×40 |
| MIX_BTN_ChannelRouting | 65×17 | routing button (output, kit mode) |
| MIX_BTN_Cue / …CueFocus | 20×140 | master header cue (headphones), 7×20 |
| MIX_BTN_Extend | 32×140 | sidebar extend/collapse arrow, 7×20 |
| MIX_BTN_MidiIn/OutChannel[Assigned] | 18×17 | MIDI channel button |
| MIX_BTN_MidiIn/OutDevice[Assigned] | 50×17 | MIDI port button |
| MIX_BTN_Monitor | 20×140 | strip cue (headphones), 7×20 |
| MIX_BTN_PluginSlotsActive | 12×140 | slot bypass square, 7×20 |
| MIX_BTN_PluginSlotsAdd | 12×140 | "+" add slot (stretched to 71), 7×20 |
| MIX_BTN_SP_Aux / SP_InOut / SP_Plugin | 32×112 / 32×112 / 32×140 | sidebar AUX / IO / plug-ins toggles |
| MIX_HANDLE_BottomScrollbar | 21×18 | scrollbar thumb |
| MIX_KNOB_SmallAssigned / …Unassigned | 17×2193 | small knob, 129 frames of 17×17 (input gain, aux levels) |
| MIX_MTR_Handle | 20×8 | fader handle: white left-pointing arrow |
| MIX_MTR_LevelScale | 18×99 | dB scale labels (+10, 0, −10, −20, −30, −∞ at y 3/25/47/63/79/95) |
| MIX_MTR_ZeroLine | 30×33 | 0 dB line #FFFFFF/a41 at y 26 |
| MIX_OVERLAY_LevelMute | 71×25 | mute overlay for the monitor/mute row |
| MIX_SHADOW_Master / …Collapsed | 8×130 / 8×40 | shadow left of the master column |

## 10. Fonts [exact]

The font is Roboto Medium throughout (our UI font):
- ID: 14
- names: 12
- IO, aux, routing and MIDI buttons: 10
- headroom: 9
- plug-in names: the default 12

## 11. Implementation notes for our viewer [inferred]

- Strip pitch is 72px (71 plus a 1px stroke), so 16 sound strips need 1152px.
- Scroll horizontally with the wheel or trackpad, or with the bottom scrollbar. The master/group column stays fixed on the right.
- Vertical budget with everything shown (large window, Mixer replacing the song panel and group panel), for a sound strip:

  | section | px |
  |---|---|
  | header + splitter | 44 |
  | input | 48 |
  | slots | ≥99 |
  | filler | flex |
  | level module | 165 |
  | output | included in the level module |
  | aux | 62 |
  | MIDI out | 22 |
  | scrollbar | 19 |

  That totals about 460 px, so it fits in the song panel plus group panel area. Collapsed is 40 + 19.
- Reuse the viewer's meters (`App.meters`). There are no master meters yet; the engine would need to report the master peak.
- Solo: implement as a mute write over the siblings, as one undo step ("Solo").
- Use the chain-area knob behaviours for drags: Shift is fine, double-click resets (level to 1.0, pan to 0).
