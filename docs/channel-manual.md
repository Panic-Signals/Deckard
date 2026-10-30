# Maschine 3 Channel properties (Input / Output / Groove / Macro): spec from the NI manual

Sources, fetched 2026-10-14:
- Maschine Software manual (the HTML manual for Maschine 3):
  - "Audio routing, remote control, and Macro Controls":
    https://docs.native-instruments.com/ni-tech-manuals/maschine-software-manual/en/audio-routing,-remote-control,-and-macro-controls
  - "Basic concepts" (Navigating Channel properties…, Preferences – Audio/MIDI/Default pages, Adjusting the swing):
    https://docs.native-instruments.com/ni-tech-manuals/maschine-software-manual/en/basic-concepts
  - "Playing on the controller" (Mute and Solo, **Groove**):
    https://docs.native-instruments.com/ni-tech-manuals/maschine-software-manual/en/playing-on-the-controller
  - "Using effects" (side-chain, external audio, send effects, multi-effects):
    https://docs.native-instruments.com/ni-tech-manuals/maschine-software-manual/en/using-effects
  - "Controlling your mix" (Mixer channel strips, Cue bus),
    "Managing Sounds, Groups, and your Project" (channels, multi-selection),
    "Working with plug-ins" (Parameter pages, multiple-output / multitimbral plug-ins),
    "Working with patterns and clips" (MIDI import/export).
- Maschine MK3 manual PDF (Maschine 2.x era, same chapter text plus hardware detail and the
  **Variation** engine): https://www.native-instruments.com/fileadmin/ni_media/downloads/manuals/maschine/MASCHINE_MK3_Manual_English.pdf
  (§ "Adding variation to Patterns", p. 360–362; § "Swing on the Note Repeat / Arp output", p. 194).
- Screenshots in the online manual (their visible labels are transcribed below).
- Where the manual is silent, the **Maschine 3 binary / NI Lua** (`out/ni-res/…`, strings of
  `Maschine 3.app`) and `docs/mxprj-format.md` were checked; those findings are marked
  **[NOT IN MANUAL]**.

Text in "quotes" is copied from the manual. **Bold** names are UI labels as the manual spells
them. (?) marks anything unclear.

---

## 0. Overview and navigation

- "Channel properties are global settings that apply to a particular Sound, Group, or to the
  Master independently of which Plug-ins are loaded in it."
- "Channel properties are organized into four sets. Similar sets of properties are available for
  all channels (Sounds, Groups, and Master): **Input properties** (unavailable for the Master),
  **Output properties**, **Groove properties**, and **Macro properties**." There is no
  "General" set.
- Control area, top left: three tabs **MASTER**, **GROUP**, **SOUND** pick which channel is
  shown (focused Group / focused Sound).
- Far left of the Control area, a vertical icon strip:
  - **Channel icon** ("showing a little knob"): shows the Channel properties. "The button lights
    up. The **Channel Property selector** appears in the left part of the Control area, showing a
    square of four buttons": **Input** | **Output** (top row), **Groove** | **Macro** (bottom
    row), each with an icon. The selected one is highlighted.
  - **Plug-in icon** (under it; a plug symbol): shows the Plug-in List instead.
  - A down-pointing arrow at the bottom left opens the **Assignment area** (tabs
    **Automation** and **Pages**), see §6 and §7.
- Above the selector: channel name (e.g. "Sound 1", "Leads") with a magnifier (quick-browse) icon.
- In the screenshots for the **Master**, the **Input** button is drawn dimmed (disabled).
- The **Parameter area** (right part) shows the selected set. Page names (tabs) sit at the top;
  "The name of the page currently displayed is highlighted." If the page names don't fit, "two
  small arrows are displayed on the left to click through the pages."
- Each parameter = control + label beneath. Control types: **Knob** (drag vertically, [Shift]
  for finer), **Button** (toggle; "When the button is activated, it shows a small colored LED"),
  **Selector** (click value → drop-down list; drawn with small up/down arrows above/below the
  value).
- Each page is divided into **sections**, each with an upper-case heading and a thin rule over
  the controls it covers (e.g. "AUDIO", "AUX 1", "CUE", "SWING").
- Multi-selection: "Parameter adjustments apply to all selected Sounds/Groups for any parameter
  in any set of Channel properties." Knobs: all selected move by the same delta, clipped at
  range limits. Buttons/selectors: all selected are set to the focused one's value. "The
  adjustments made in the Assignment area are never mirrored in any other selected Sound/Group."

### Page/tab matrix

| Set | Sound | Group | Master |
|---|---|---|---|
| **Input** | **Audio** \| **MIDI** | **MIDI** only | — (unavailable) |
| **Output** | **Audio** \| **Aux** \| **MIDI** | **Audio** \| **Aux** | **Audio** only |
| **Groove** | **Swing** (+ **Variation** \| **Generate** in Maschine 3 [NOT IN MANUAL], §5.3) | **Swing** | **Swing** |
| **Macro** | user pages ("Page 1", …) | user pages | user pages |

Manual notices backing this: "The **Audio** page of the Input properties is available for Sounds
only." "There is no **MIDI** page in the Input properties of the Master." "There is no **Aux**
page in the Output properties of the Master." "The **MIDI** page of the Output properties is
available for Sounds only." "The Groove properties have a single Parameter page: **Swing**."
Screenshot caption: "The Parameter pages of the Sound's Output properties: Audio (currently
displayed), Aux, and MIDI."

---

## 1. Signal flow (default routing)

- "each Sound has its own audio channel. The output of the Sound's channel is sent to its parent
  Group."
- "each Group has its own audio channel, which sums all channels of the included Sounds. The
  output of the Group's channel is sent to the Master."
- "the Master has its own channel, which sums the channels of all Groups and ends up at the main
  output of Maschine" (audio interface in stand-alone, host in plug-in mode).
- Order in a channel strip (Mixer, top → bottom): input routing → Plug-ins → pan and level →
  output routing.
- **Bussing points**: a Sound or Group "acting as bussing point (i.e. containing an Effect
  Plug-in in its first Plug-in slot)". "When you load an effect into the first Plug-in slot of a
  Sound, Maschine automatically configures the Sound's input to receive any signal(s) coming from
  other Sounds and Groups in your Project and sends them through its own Plug-in slots". Such
  Sounds/Groups then appear in other channels' **Dest.** menus.
- Send-effect rules: "You cannot send the Master output to send effects." "You cannot send a send
  effect's output to itself, nor the output of a Group to one of its own Sounds." You can chain
  send effects, and a Group's effects process its own Sounds plus anything sent to it. "take care
  to avoid feedback loops!" (the manual doesn't say whether loops are blocked (?)).
- Each Sound/Group has three outputs: **Main**, **Aux 1**, **Aux 2**.

---

## 2. SOUND channel

### 2.1 Sound › Input › **Audio**

Screenshot: tabs `Audio` `MIDI`; section **AUDIO**; selector showing **None**, label
**Source**; knob **Gain**.

| Section | Label | Values / behaviour |
|---|---|---|
| AUDIO | **Source** | "Selects an external audio input to be routed to the Sound. The available entries are **None** (no external input), the 16 stereo inputs **In 1 L+R** – **In 16 L+R** of Maschine, the 32 corresponding left or right mono inputs, as well as any additional outputs of multiple-output Plug-ins loaded in other Sounds of the same Group." Default **None**. Mono entry names are not spelled out; presumably "In 1 L", "In 1 R", … (?) |
| AUDIO | **Gain** | "Adjusts the gain of the incoming signal." Range, unit and default not given (?) |

- "Each Sound can use one external mono or stereo input. The same external signal can be fed into
  any number of Sounds."
- Stand-alone: In 1…16 map to physical inputs chosen in Preferences › Audio › Routings › **Input**
  ("which inputs on your audio interface should be used for each of the 16 stereo inputs of
  Maschine"). Plug-in: they're the plug-in's virtual inputs from the DAW ("any of its sixteen mono
  virtual inputs" in one place, "In 1–8" stereo in another, so the count is inconsistent (?)).
- Multiple-output plug-ins: "The Plug-in's first output pair is inserted in the usual Plug-in
  signal chain… The Plug-in's additional outputs are made available as audio sources for other
  Sounds of the same Group (they appear in the **Source** menu…)".
- Mixer equivalent (IO section, Sound strips only): **AUDIO IN** label (= Source) plus a small
  knob on its left (= Gain).
- Typical use ("Applying effects to external audio"): set Source, adjust Gain, then add an effect
  in the Sound's Plug-in List; "the effect is loaded in a new Plug-in slot and directly starts to
  process your external audio!"

### 2.2 Sound › Input › **MIDI**

The manual has one table for the Sound and Group MIDI pages. Section heading in the table:
"**MIDI ROUTING** Section". The Group screenshot shows the heading **ROUTINGS** (?).

| Label (as in manual) | Applies to | Values / behaviour |
|---|---|---|
| **Key Mode** | "(Group mode only)": Group only, not shown for Sounds (?) | see §3.1 |
| **Source** | "(stand-alone and in Manual mode only)" | "Select the MIDI port on which the Sound or Group will receive MIDI notes. Available entries are **None** (MIDI note input deactivated, default setting), **All** (MIDI notes are received on all activated input ports), and each activated MIDI input port." "Note that when **None** is selected for both a Sound and its parent Group, if the Sound is focused it will receive MIDI notes from all MIDI ports and on all MIDI channels." |
| **Channel** | "(Manual mode only)" | "Select the MIDI channel on which the Sound or Group will receive MIDI notes. If you select **All** (default setting), the Sound or Group will receive MIDI notes from all channels." Values: All, 1–16. A Sound's Channel is also used when importing a multi-channel MIDI file to its Group. |
| **Start Note** | "(Manual mode only)" | Described for Groups only (§3.1). Whether a Sound page has it is unclear (?) |
| **Thru** | "(Manual mode only)" | "Switch this parameter on to send incoming events through to the MIDI outputs… Switch this parameter off to stop incoming events from being routed to the outputs." Default not stated (screenshot: off). |

Older text, kept in the Mix-view steps: "If Maschine is running as a plug-in you see a **MIDI IN**
button instead. Click it to enable the MIDI input from the host. This is equivalent to enabling
the **Active** button described above." So in **plug-in mode** the page has an **Active**
(on/off, "Host") button in place of **Source** (?), though the current table no longer lists
**Active**.

What the Mixer screenshot shows (Maschine 3 Sound strip, IO section): a channel field (**All**) and
a source field (**FOCUS**). Its menu offers **Default**, **Focus** (checked), **None**, **All**,
then port names (e.g. "Maschine MK2 In"). So Maschine 3's Sound MIDI Source list is probably:
`Default | Focus | None | All | <each activated input port>` (?).
Related preference: Preferences › Default › Sound › **Default MIDI Input Mode**: "**Focus**: MIDI
input from any connected controller can be used to trigger the focused (selected) Sound slot." /
"**None**: The selected Sound will not receive MIDI data." Presumably **Default** on a Sound
follows this preference (?).

Semantics:
- "By default and without any configuration, incoming MIDI notes on any MIDI port and any MIDI
  channel will trigger different pitches of the Sound slot currently under focus."
- "**MIDI input settings for a Sound** affect only that Sound: Incoming MIDI notes will trigger
  different notes of this Sound… the incoming MIDI notes will be routed to the first Plug-in slot
  of the Sound."
- "If the Sound has no Plug-ins, incoming MIDI notes will be forwarded to the Sound's MIDI output,
  if activated".
- **Merging with the Group**: "MIDI input settings of a Sound and its parent Group are merged: For
  example, if you configure a Group to react to incoming notes on MIDI channel 1, but one Sound is
  setup to react to incoming notes on MIDI channel 2, then this Sound will react to one incoming
  note on MIDI channel 1 and all incoming notes on channel 2."
- These settings also drive: plug-in preset selection by Program Change (a VST/AU in the Sound
  "will listen to Program Change messages according to the **Source** and **Channel** settings"
  of this page); MIDI-file import into Groups; the Pattern Editor's MIDI dragger.
- MIDI CC automation (§6) is **not** affected by these settings.
- In plug-in mode, Scene/Section triggering via MIDI notes "has priority over any Sound trigger
  set to the same MIDI channel."
- Program Change → preset switching works only for Native Instruments / External plug-ins
  "loaded at the Sound level".
- Not automatable: none listed for the Sound MIDI input page.

### 2.3 Sound › Output › **Audio**

Screenshot (Sound "Sound 1"): tabs `Audio` `Aux` `MIDI`; section **AUDIO**; selector **Group**
(label **Dest.**); toggle **Cue**; knob **Level**; knob **Pan**; toggle **Audio Mute**.

| Section | Label | Values / behaviour |
|---|---|---|
| AUDIO | **Dest.** | "Selects where you want to send the main audio output of your channel." For Sounds: "**None** (no output), **Master**, **Group** (parent Group, default setting), any other Group or any other Sound acting as bussing point (i.e. containing an Effect Plug-in in its first Plug-in slot), as well as the 16 external stereo outputs **Ext. 1**–**16** of Maschine." |
| AUDIO | **Cue** | Toggle, default off. "When **Cue** is activated, the main output of the channel is sent to the Cue bus instead of its normal target (defined in **Dest.**…)." "Note that when **Cue** is activated, the channel's Aux 1 and Aux 2 outputs are muted as well — but they are not sent to the Cue bus". Not automatable. |
| AUDIO | **Level** | "Adjusts the overall volume level of the channel." dB; range and default not given (?). The hardware VOLUME shortcut drives this; "To reset the output level of the Master to 0 dB, double-click the Master Volume slider" suggests 0 dB is the default for levels (?). |
| AUDIO | **Pan** | "Defines the pan position of the channel in the stereo field." Centre default (?) |
| AUDIO | **Audio Mute** | "(Sounds only)". Toggle, default off. "If you enable **Audio Mute**, muting this Sound will not only bypass its events but also mute its audio output, thereby muting any audio tails from notes already played." |

**Dest. labels** (same rules for the Aux Dest. menus):
- In the drop-down: Groups as "**[Group name]** (e.g., **Drums**)"; Sounds as
  "**[Group name]: [Sound name]-[input number]** (e.g., **Drums: Kick-1**)".
- In the selector display: Groups as "**[Group letter+number]** (e.g., **A2**)"; Sounds as
  "**[Group letter+number]:S[Sound number]-[input number]** (e.g., **A2:S4-1** for the first input
  of the Sound 4 of Group A2)". The fixed entries display as **None**, **Master**, **Group**,
  **Ext. 1**… The "-[input number]" suffix implies a bussing Sound can have more than one input
  (?).
- External outputs: stand-alone → physical outputs chosen in Preferences › Audio › Routings ›
  **Output** ("a list of the 16 stereo outputs of Maschine"); plug-in → the plug-in's virtual
  outputs.

**Mute semantics** (Playing on the controller › Mute and Solo): "At the Group level, the Mute
function is an **audio mute**… At the Sound level, the Mute function is by default a **trigger
mute**: the Pattern content (the events) for the muted Sound will not be triggered — but any audio
remaining from past events for this Sound will still be audible". Sound solo only applies within
its Group.
**Cue and mute**: "The mute and solo of channels don't apply when these channels are sent to the
Cue bus".

### 2.4 Sound › Output › **Aux**

Screenshot (shown for a Group; Sounds are the same): tabs `Audio` `Aux` (+`MIDI` for Sounds); two
sections, **AUX 1** and **AUX 2**, each with selector **None** (label **Dest.**), knob **Level**,
selector **Post** (label **Order**).

| Section | Label | Values / behaviour |
|---|---|---|
| AUX 1 / AUX 2 | **Dest.** | Same options as the Audio page's **Dest.** "except that for Aux 1 and Aux 2 the default destination is **None**". Whether "Group"/parent is offered is not stated (?) |
| AUX 1 / AUX 2 | **Level** | "Adjusts the level of the signal sent to the Aux 1 or Aux 2 output." Range/default not given (?) |
| AUX 1 / AUX 2 | **Order** | **Pre** / **Post** (default **Post**). "If **Order** is set to **Pre**, the channel will be fed into Aux 1 or Aux 2 *before* the **Level** and **Pan** settings in the **Audio** section of the **Audio** page are applied. If **Order** is set to **Post** (default setting), the **Level** and **Pan** settings of the main output also affect the signals sent to the auxiliary outputs." |

- Mixer: **AUX 1** / **AUX 2** labels (default labels; they show the destination once set) with
  a small level knob on the right and a **Post**/**Pre** label below.
- Using the main output rather than an aux to chain effects is recommended, because only the main
  output has **Cue** and hardware shortcuts.

### 2.5 Sound › Output › **MIDI**

Section **MIDI**. Screenshot not retrievable. Mixer: **MIDI OUT** label plus a small channel
field (shows **1** for Sound 1, **2** for Sound 2…).

| Section | Label | Values / behaviour |
|---|---|---|
| MIDI | **Dest.** | "Selects the MIDI port on which the Sound will send MIDI data." Stand-alone: "**None** (MIDI output deactivated, default setting), every activated MIDI output port, as well as any Sound in the same Group containing a Plug-in that can receive MIDI (multitimbral Plug-in)." Plug-in: "**None**… and **Host**. If **Host** is selected, Maschine will forward the MIDI data from the Sound to the host application." Mixer menu screenshot shows Sound entries as "**S3 - Kick Smooth 3**", "**S4 - OpenHH 909X**", i.e. `S<n> - <Sound name>`. |
| MIDI | **Channel** | "Selects the MIDI channel on which the Sound will send MIDI data. By default Sound slot 1 will send MIDI data on channel 1, Sound slot 2 on channel 2, etc." Values 1–16. |
| MIDI | **Transpose** | "Applies an offset to the MIDI note numbers before the notes are sent. Values range from -48… to +48… The default value is 0." Semitones. |

With the MIDI output active, a Sound sends: "MIDI notes corresponding to the notes played by that
Sound in the current Pattern", "MIDI notes corresponding to your hits on the pads", and "All events
of the MIDI tracks you might have created in the MIDI Automation pane of the Control Lane". "A
Sound without any Plug-in loaded can still send the MIDI data listed above! Furthermore, if the
Sound has no Plug-in it will forward any incoming MIDI notes to its MIDI output." MIDI-file
export uses each Sound's **Channel** and **Transpose**.

### 2.6 Sound › Groove: see §5. Sound › Macro: see §7.

---

## 3. GROUP channel

### 3.1 Group › Input › **MIDI** (only page)

Screenshot (Group "Centaury Kit"): single tab `MIDI`; section **ROUTINGS**; selectors
**Manual** (label **Key Mode**), **None** (**Source**), **All** (**Channel**); knob
**Start Note**; toggle **Thru**.

| Label | Values / behaviour |
|---|---|
| **Key Mode** | "Enables the MIDI note input for the selected Group (deactivated by default)." Options: **Off** ("disable MIDI note input for the selected Group"); **Drumkit** ("conveniently configure the MIDI note input for the selected Group and place each Sound on a specific key chromatically across the keyboard starting from C2 upwards." "If nothing is loaded on a Sound nothing can be triggered". It also changes how the Pattern Editor's MIDI dragger exports MIDI); **Manual** ("configure the MIDI note input based on your own preferences, including the **Source**, **Channel**, **Start Note** and **Thru** settings"). Default **Off**. |
| **Source** | "(stand-alone and in Manual mode only)". **None** (default) / **All** / each activated input port. |
| **Channel** | "(Manual mode only)". **All** (default) / 1–16. |
| **Start Note** | "(Manual mode only)". "Define the MIDI root note for that Group (C1 by default). The MIDI root note and the upper 15 MIDI notes (from [Root Note] until [Root Note + 15]) will trigger the root note of Sound slot 1–16, respectively. Incoming MIDI notes outside this range will be ignored by the Group." Elsewhere it's called **Root Note**; "C1 (which corresponds to MIDI note number 36 in the Maschine convention)". Each Sound plays its own root note, which is C3 (MIDI import: "MIDI notes will be imported at the default root note C3"). **Not automatable** ("Group's Input properties: the **Root Note** knob in the **MIDI** page"). |
| **Thru** | "(Manual mode only)". On = pass incoming events through to the MIDI outputs. |

- Disabled-state rule (inferred from "(… only)"): when Key Mode ≠ Manual, Source/Channel/Start
  Note/Thru are hidden or disabled; Source also disappears in plug-in mode (?).
- "**MIDI input settings for a Group** affect all Sounds in the Group: Each Sound will be
  triggered by a different MIDI note. Typically, this allows you to quickly set up a MIDI remote
  control for an entire drum kit."
- MIDI import of a single-channel file into a Group: note `RootNote + k` → Sound slot `k+1`
  (k = 0…15).
- Mixer: Group strips also have the **MIDI IN** source + channel fields.
- **[NOT IN MANUAL]** The Maschine 3 binary has a Group context-menu entry **Group MIDI Batch
  Setup** with sub-items **Sounds to MIDI Channels** and **Sounds to MIDI Notes** (presumably
  bulk-sets each Sound's MIDI input channel, or the Group's note mapping) (?).

### 3.2 Group › Output › **Audio**

Same as §2.3 without **Audio Mute**: **Dest.**, **Cue**, **Level**, **Pan** (section **AUDIO**).
Group **Dest.** options: "**None** (no output), **Master** (default setting), any other Group
acting as bussing point (i.e. containing an Effect Plug-in in its first Plug-in slot), any Sound
acting as bussing point in any other Group, as well as the 16 external stereo outputs **Ext.
1**–**16** of Maschine." A Group's own Sounds are excluded. Group mute is always an audio mute.

### 3.3 Group › Output › **Aux**

Same as §2.4: AUX 1 / AUX 2 × (**Dest.**, **Level**, **Order**).

### 3.4 Group › Groove, Macro: see §5 and §7.

---

## 4. MASTER channel

### 4.1 Master › Input
Unavailable (button disabled).

### 4.2 Master › Output › **Audio** (only page)

Screenshot (Project "Raminagrobis"): single tab `Audio`; section **AUDIO**: selector **Ext. 1**
(label **Output**), knob **Level**, knob **Pan**; section **CUE**: selector **Ext. 2**
(**Output**), knob **Level**, knob **Pan**.

| Section | Label | Values / behaviour |
|---|---|---|
| AUDIO | **Output** | "Selects where you want to send the main stereo output of your Maschine Project. The available options are the 16 external stereo outputs **Ext. 1**–**16**." Default **Ext. 1** ("the Master is output to the first stereo pair of outputs"). |
| AUDIO | **Level** | "Adjusts the overall volume level of the master output. This control is the same as the Master Volume slider in the Header". Double-clicking the Header slider resets to 0 dB. |
| AUDIO | **Pan** | "Defines the panoramic position of the master output in the stereo field." |
| CUE | **Output** | Ext. 1–16. "By selecting a different output from the one used for the main stereo output, you can effectively pre-listen on this additional output (e.g., on your headphones) any channels sent to this Cue bus." Default shown in the screenshot: **Ext. 2** (?) |
| CUE | **Level** | "Adjusts the level of the Cue output." |
| CUE | **Pan** | "Defines the panoramic position of the Cue output in the stereo field." |

- Cue bus also carries: Browser sample pre-listening, the **metronome**, and Sample Editor
  pre-listen.
- Mixer: Master and Cue share the strip at the far right. Click **Master** or the headphone icon
  in its header to switch. The Master strip has no Mute, no audio/MIDI input, no aux, no MIDI out;
  the Cue strip has no Plug-in List.
- No **Aux** page and no **MIDI** page on the Master. The Master can't feed send effects.
- Side-chaining "is not possible at the Master level".

---

## 5. GROOVE properties (all channels)

### 5.1 Swing page (Sound, Group, Master)

Screenshot (Sound "Bouncer"): single tab `Swing`; section **SWING**; knob **Amount**; selector
**1/8** (label **Cycle**); toggle **Invert**.

| Section | Label | Values / behaviour |
|---|---|---|
| SWING | **Amount** | "Adjusts the amount of swing, i.e. the amount by which some events are shifted. At 0 % events are not shifted. Raise the **Amount** value to increase the strength of the swing." Unit %, 0.0–100.0 % (the example uses "0.0 %" and "100.0 %"). Default 0 % (?) |
| SWING | **Cycle** | "Determines on what musical resolution the groove is applied. This directly affects which events will be shifted. Values are measured in fractions of a whole note." The manual gives no full list; the screenshot default is **1/8**, and the example uses **1/2**. |
| SWING | **Invert** | "Allows you to invert the groove so that instead of being delayed in the Pattern, events will be triggered ahead of time." Default off. |

Hierarchy: "The Groove properties of a Sound only affect that particular Sound." "The Groove
properties of a Group affect all Sounds in that Group: The Group's swing is added to the swing of
each individual Sound". "At the Master level… The Master's swing is added to the swing of each
individual Group and Sound".
- The Header's **SWING** control (drag vertically) is the Master's swing Amount (the "overall swing
  of your song").
- Hardware SWING shortcut: encoder = Master; hold Group button = Group; hold pad = Sound. It
  "control[s] the **AMOUNT** parameter in the **SWING** page".
- Not automatable: "Master's Groove properties: the **Amount** knob and **Invert** toggle in the
  **Swing** page." Sound/Group swing parameters are automatable and modulatable.
- Swing shifts only the **triggers**: "adjusting the Groove properties will **not** actually move
  any events in the Pattern Editor."
- "The Swing engines (at the Master, Group, and Sound levels) process notes coming from the Pattern
  Editor and from the Arp engine… or Note Repeat engine". "When the playback is not running, the
  swing is still applied to the output of Arp… or Note Repeat… If you press PLAY on your
  controller, the playback starts immediately and the swing cycle is reset."

**Worked example from the manual** (screenshot of 3 rows, one bar of 8 eighth notes, Cycle =
**1/2** = two beats):
- Top, Amount 0.0 %: evenly spaced.
- Middle, Amount 100.0 %, Invert off: in each half-note cycle the 1st hit stays, the 2nd moves later
  a little, the 3rd (the cycle midpoint) moves later the most, the 4th moves later a little. The
  next cycle starts unshifted on beat 3 (1.3).
- Bottom, Amount 100.0 %, Invert on: same hits shifted **earlier** by mirrored amounts.

**Algorithm [NOT IN MANUAL]** (from `docs/mxprj-format.md` §11, decompiled
`NI::MASCHINE::Swing::calcToSwingPos`; the manual example agrees with it):
- Position in quarter notes × 24, split into cycles of `[12, 18, 24, 36, 48, 56, 96, 192]` units
  for Cycle index 0…7 (1/8, 3/16, 1/4, 3/8, 1/2, ~7/12 (?), 1, 2 whole notes). Index 0 (1/8,
  swings 16ths) is the default.
- `a = Amount/100`, negated if Invert. The cycle midpoint moves to
  `0.5 + 0.45 · comp[cycle] · (|a| + a²)/2`, with comp = `[1, .5, .5, .25, .25, .16, .125, .0625]`.
- The first half of the cycle is stretched linearly to the new midpoint; the second half is
  compressed to fit (piecewise-linear time warp; cycle boundaries fixed).
- Applied Sound → Group → Master in turn (the manual says "added").
- Stored per MixingLayer: `swing_amount`, `swing_cycle`, `swing_invert`, plus `swing_bounce`
  (bool; no UI label in the manual (?)).
- The cycle labels are generated at runtime as `1/%i` (so the exact Maschine 3 display strings for
  non-unit fractions are unclear (?)).

### 5.2 Variation (manual: hardware "Variation" engine, Maschine 2.x MK3 manual)

The software manual (the current HTML manual) **does not mention Variation or Generate at all**.
The MK3 hardware manual (§ "Adding variation to Patterns") documents the Variation engine,
reached from the controller (**VARIATION** button; SHIFT+VARIATION = Navigate):

- "The variation engine contains two modes: **Humanize**: This mode adds natural rhythmic
  fluctuations to programmed sequences. **Random**: This mode provides randomly generated
  patterns or variations of your beats and melodies based on customizable values."
- "Variation is available in both Group mode and Keyboard mode and can be applied to a Sound. In
  Keyboard mode, the Variation in Random mode contains extra parameters that allow you to add
  variety to melodic content." "Random mode conforms to the selected scale."
- Buttons: **HUMANIZE**, **RANDOM**, **APPLY** ("Applies the current settings to the selected
  Sound."). It edits the focused Sound's Pattern events (destructive, undoable (?)).

**Humanize mode parameters**

| Section | Label | Values |
|---|---|---|
| VELOCITY RANGE | **Velocity Lo** | "Adjust notes with a velocity no lower than the set value." (0–127 (?)) |
| VELOCITY RANGE | **Velocity Hi** | "Adjust notes with a velocity no higher than the set value." |
| TIME SHIFT | **Step** | "Set the amount of time shift per step. The value is applied to all notes and will move them randomly in both plus and minus values according to the overall percentage set here." "+/- 0-50% in 5% increments" (SHIFT: 1 % steps). Percent of one step (Step Grid). |

How Humanize velocity works is not spelled out. Presumably each note gets a new random velocity in
[Lo, Hi]: users complain it is a fixed range, not a deviation around the original (?).

**Random mode parameters** ("(Keyboard mode)" = only shown when the pads are in Keyboard mode)

| Section | Label | Values |
|---|---|---|
| PROBABILITY (Keyboard mode) | **Probability** | "Set the probability of how often a note is created. At 50% a note may or may not be created at each step. At 100% a note will be created at every step of pattern." "10-100% in 10% increments" (SHIFT: 1 %). The controller Lua also shows Probability in pad (Group) mode. |
| NOTE RANGE (Keyboard mode) | **Note Lo** / **Note Hi** | "Create notes no lower / no higher than the set value." |
| VELOCITY RANGE | **Velocity Lo** / **Velocity Hi** | "Create notes with a velocity no lower / no higher than the set value." |
| CHORDS (Keyboard mode) | **Note Count** | "Set the amount of notes that are available to make up a chord per step… The Note Count range is from 1 to 6." |
| NOTE LENGTH | **Steps** | "Set the length of a note in steps. The value ranges from 1 to 6 steps per note." |
| TIME SHIFT | **Step** | as in Humanize: ±0–50 % in 5 % increments. |
| DISTRIBUTIONS | **Note Count** (Keyboard mode) | **Fixed** ("The value set in the Note Count of the CHORDS section will take precedent"), **Equal** ("evenly distributed across the NOTE RANGE"), **Gauss** ("likely to be distributed to the middle section of the NOTE RANGE"), **½ Gauss** ("middle and lower section of the NOTE RANGE"). |
| DISTRIBUTIONS | **Notes** (Keyboard mode) | **Equal** ("evenly distributed across the Pattern in accordance with the Step Grid"), **Gauss** ("more is likely to be distributed to the middle part of the note range"), **½ Gauss** ("middle and lower part of the Pattern in accordance with the Step Grid"). |
| DISTRIBUTIONS | **Note Length** | **Fixed** ("same length across the Pattern"), **Equal** ("evenly distributed"), **Gauss** ("Note lengths in the middle part of the pattern are more likely to be shorter"), **½ Gauss** ("middle to lower part of the pattern are more likely to be shorter"). |

Controller page layout from NI's Lua (`VariationPageBase.lua`), section names per knob 1–8:
- Random, Keyboard, page 1: Probability | Note Range (Lo, Hi) | Velocity Range (Lo, Hi) | Chords |
  Note Length | Time Shift. Page 2: Distribution (Note Count dist., Note dist., Note Length dist.).
- Random, pad mode: Probability | Velocity Range (Lo, Hi) | Time Shift | Note Length | Distribution
  (Note Length dist.).
- Humanize: Velocity Range (Lo, Hi) | Time Shift.
- These legacy parameters are **global Workspace parameters** (`getRandomize*Parameter`), not
  per-Sound. The Apply button is disabled when the Sound holds an Audio plug-in in Loop mode.

### 5.3 Maschine 3: Variation and Generate as Groove pages [NOT IN MANUAL]

Evidence from the Maschine 3 binary (embedded Lua and stylesheet):
- Stylesheet comment: `// APPLY VARIATION BUTTON (Groove > Variation page, Sound level, channel
  parameters)`. Widgets `Button#m_applyButton` / `m_applyVariationButton` sit in the Parameter
  area after a 1-px separator. So the **Apply** button is a button at the right end of the
  parameter row.
- Lua: "The wheel button icon (apply variation / generate) is only shown on the 'Variation' and
  'Generate' pages of the Groove section".
  `MaschineHelper.showGrooveVariationSection()` = focus SOUND level, then
  `setFocusPageOnGroovePageGroup(App, 1)`. `getApplyButtonState`: `IsVariation = FocusedPage == 1`;
  on the other active page the button calls `SoundAccess.generateSequence(App, Sound)`, otherwise
  `SoundAccess.applyVariation(App, Sound)`. Variation's button is enabled only if
  `SoundAlgorithms.hasVariationToApply(Sound)`.
- Controller comment: "[SHIFT] + [NOTE] = shortcut to the GROOVE Variation/Generate section of the
  Control Page".
- File format (`crates/mxprj` `Sound::serialize`, v > 9): each **Sound** has a
  `SequenceVariationModule` (4 floats + 1 enum + 1 bool) and a `SequenceGenerationModule` (enum,
  2 floats, a RangeParameter, 3 ulongs, 2 enums, a ulong, `live_mode` bool). The runtime has a
  `DSeqModule` / `DSeqProcessor` (sequence generator).

Conclusion: in Maschine 3, **Groove properties of a Sound** have three pages, **Swing** (index 0),
**Variation** (index 1) and **Generate** (index 2 (?)). Group and Master keep only **Swing** (?).
Variation and Generate are per-Sound and end in an **Apply** / **Generate** action button. Their
parameter labels, ranges and algorithms are **not documented** by NI. Likely the Variation page is
the per-Sound successor of §5.2 (Humanize/Random, velocity range, time shift…; 4 floats + mode enum
fit "Velocity Lo/Hi, Time Shift, Probability + Humanize|Random"), and Generate drives a pattern
generator with a "live" mode (?). Labels need recovering from the GUI (screenshot of the running
app) or the parameter label tables (`tools/re-tools label-refs`).

---

## 6. MIDI / host automation of Channel properties (Assignment area › Automation)

- "You can assign many parameters of the Channel properties and Plug-ins of your Sounds, your
  Groups, and your Master to MIDI messages… In addition, when Maschine is running as a plug-in, you
  can assign these parameters to automation IDs". Software only.
- Automation is **permanent** and **absolute**. Modulation is temporary (until the end of the
  Clip), relative, and Sound/Group only. Both can be combined.
- UI: Assignment area (down-arrow at the bottom left of the Control area) → **Automation** tab →
  sub-tabs **MIDI** and **Host**. Under each automatable parameter of the current page sits an
  **Assignment field**: "**Learn**" (MIDI tab) or "**Enable**" (Host tab) when empty; greyed and
  inactive if not automatable. The area follows page changes.
- MIDI: click **Learn**, which then blinks "**Learning**"; moving a control assigns it. The field
  shows the CC number, "**PC**" (Program Change) or "**PW**" (Pitchbend). Messages: "Pitch wheel,
  Program Change, and MIDI CC 0–127". "the assignments you create will receive MIDI messages on all
  channels of all activated MIDI ports." A new learn replaces the old one; clicking again or [Esc]
  cancels. One MIDI control → one parameter (assigning it elsewhere removes the first). Right-click
  → **Unlearn**.
- Host: click **Enable** to assign "the next free automation ID". The field shows the ID; click
  again to remove it.
- Automatable = "controlled by a knob or a button". Most selectors are not. Exceptions in Channel
  properties (not automatable): Sound/Group Output › Audio › **Cue**; Group Input › MIDI › **Root
  Note** (Start Note); Master Groove › Swing › **Amount** and **Invert**.
- Macro Controls are automatable / modulatable "if, and only if" their targets are.
- Scene/Section/Lock triggering via MIDI: **Edit › MIDI Change** dialog ("MIDI Change": "To
  change Scenes, Sections, or Lock Snapshots via MIDI Notes or MIDI Program Changes, enable one or
  more of the following options:" with **Scene** / **Section** / **Lock**, each with an
  "**Enabled**" checkbox, and a **Close** button). Per option: source (controller list or
  **None**), **MIDI Note** or **Program Change**, channel (default 1). Scene change beats Lock
  change on the same source/channel.

---

## 7. MACRO properties (all channels)

### 7.1 What they are
- "Macro Controls enable you to control in one same location a selection of parameters coming
  from different sources. Available in every channel (Sounds, Groups, and Master)".
- "Each Macro Control can be assigned to one destination with the full range of the selected
  parameter. On the other hand, the same parameter can be the destination of more than one Macro
  Control." They are aliases: MIDI/host automation is configured on parameters independently
  (Maschine 1.x used Macros for MIDI, 2.x+ does not).
- **Target scope rule**: "A Macro Control of a channel can control any parameter located in the
  properties or Plug-ins of that channel or of any underlying channel."
  - Sound: its own properties and Plug-ins, **plus** "MIDI Control Change messages, Pitchbend and
    Program Changes" to control external MIDI devices (sent via the Sound's MIDI output (?)).
  - Group: its own properties and Plug-ins plus any Sound in the Group.
  - Master: any channel in the Project.
  - "The Macro properties themselves (even from a different channel) are not available as targets
    for any Macro Control."
- Layout: Parameter pages of **8** knobs (hardware Knob 1–8; NI/External plug-in pages hold "8
  knobs"). Any number of pages. Macros are always knobs (?).
- Display: the page tab label mirrors its section names joined by " / " (screenshot tab
  "**Metaverb / Grain Delay / Groove**", sections **METAVERB** (Reverb Size, Reverb Mix), **GRAIN
  DELAY** (Grain Pitch, Grain Size, Density), **GROOVE** (Amount)). Parameters keep their target
  label (e.g. "Reverb Size") unless renamed.
- Empty state: unassigned slots show default labels "**Macro 3**", "**Macro 4**"… (i.e. "Macro
  N"), and a new set has one page "**Page 1**" (screenshot of a Sound "Driver": tab `Page 1 ×`, `+`).
  An unassigned slot's knob is presumably not drawn or is inactive (?). The Pattern Editor modulation
  lane shows "(none)" (?).
- Saved with the Project ("Macro Controls are saved with your Maschine Project").
- Hardware: **MACRO** button jumps to the focused channel's Macro properties; SHIFT+MACRO =
  "Set Macro mode" (touch Knob 1–8 to add/remove a parameter as a Macro; a small icon appears next
  to the assigned parameter name).

### 7.2 Assigning from a parameter (quick way)
1. Show the Channel or Plug-in parameters (Channel / Plug-in icon) at the right level (MASTER /
   GROUP / SOUND).
2. Right-click the parameter **name** (text under the knob, switch or menu) → **Create Master
   Macro**, **Create Group Macro**, or **Create Sound Macro** "depending on the options available"
   (only levels at or above the parameter's channel are offered, per the scope rule).
3. To undo: right-click → **Remove Macro**.
The new Macro goes to the next free slot (which page and slot is not stated (?)). A parameter
assigned as a Macro is marked with a small icon next to its name.

### 7.3 Assigning with the Pages pane (Assignment area › **Pages**)
The **Pages** tab is enabled only for Macro properties and NI/External plug-ins (greyed for other
Channel properties and internal plug-ins). With it open, the Control area shows (numbered callouts
from the manual screenshot):
1. **Delete Page button ("x")** after each page name: deletes that page.
2. **Add Page button ("+")** after the last page label: appends a page. "By default, pages are
   labeled "Page 1," "Page 2," etc." (screenshot: `Metaverb / Grain Delay / Groove ×`, `Page 2 ×`,
   `Page 3 ×`, `+`).
3. **Section Label fields** (a text field above each of the 8 knobs): double-click, type, [Enter].
   "The new section will include all following parameters until the next section starts (this can
   be seen only after you have closed the Assignment area). Moreover, the Parameter page will mirror
   the label(s) of its section(s) — if there are more than one section, the page label will mirror
   all of them, separated by slashes."
4. **Reset** button (right end of the Assignment area): "remove the Macro Control assignment for
   the selected knob."
5. **Parameter Label fields** (the label under each knob): double-click to rename; "These labels
   will be mirrored everywhere in Maschine for the corresponding parameters."
6. **Focus frame** (orange corner brackets around one knob): the Macro being edited; click any
   Macro to focus it.
7. **Target selector** (in the Assignment area): a breadcrumb of drop-down menus, e.g.
   `Grain Delay ▾ › Main ▾ › Size ▾`.

**Target selector procedure**:
- An unassigned Macro shows a single **Select** menu. Its sub-categories: **Settings** ("listing the
  properties of the current channel", i.e. Input/Output/Groove), **Slots** ("listing the Plug-ins
  loaded in the current channel"), and the next level down: **Groups** (when editing the Master),
  **Sounds** (when editing a Group), or **MIDI** (when editing a Sound), which "lists the full
  range of MIDI Control Change messages from 0-127". (The overview also mentions Pitchbend and
  Program Change as Sound targets (?).)
- Each choice renames that menu to the choice and opens the next menu to its right (e.g. the
  Parameter pages of the chosen plug-in, then the parameter). The screenshot of the page menu shows
  a heading "**Pages**" with items "**Main**", "**Output**". Choosing a parameter assigns it.
- "To modify a particular item in the selection… click it and select another item for that level.
  Items on the left of the selected item… stay untouched, while items on its right… are reset."

**Macro slot context menu** (right-click an assigned or unassigned Macro, Pages tab open):
**Rename** (Ctrl/Cmd+R), **Reset**, *(separator)*, **Cut** (Ctrl/Cmd+X), **Copy** (Ctrl/Cmd+C),
**Paste** (Ctrl/Cmd+V; greyed when the clipboard is empty).
**Page-name context menu**: **Delete** ("Deletes current page with all assignments."),
**Clear All** ("Clears all the assignments, and deletes all of the pages.").

### 7.4 MIDI / host on Macros
A Macro is a normal knob on a Parameter page, so it can be MIDI-learned / host-enabled in the
Automation pane (§6), provided its target is automatable. Same for modulation.

**[NOT IN MANUAL]** per MixingLayer the file stores `macro_page` and `macro_focus` (current page
and focused Macro).

---

## 8. Side-chain input (Plug-in page, not a Channel property, but routing-related)

- Supported: internal **Compressor, Maximizer, Limiter, Gate, Filter**; AU plug-ins with side-chain
  input; VST plug-ins with multiple inputs. Loading one adds a **Side-Chain Input** Parameter page
  "at the end of the page list". Not at the Master level. Not shown in the Mix-view Plug-in Strip.
- Parameters:
  - INPUT section: **Source**, "**None** (side-chain deactivated, default setting), the outputs of
    all (other) Sounds, and the outputs of all (other) Groups". Menu labels: Groups "[Group name]"
    (e.g. Drums), Sounds "[Group name]: [Sound name]" (e.g. Drums: Kick). Display labels: Groups
    "[Group name]", Sounds "[Group letter+number]:S[Sound number]" (e.g. A1:S4). **Gain**: "input
    level of the side-chain signal".
  - FILTER section: **Filter** (on/off), **Center Freq**, **Width** (bandwidth).
- "the settings of the Side-Chain Input page stay in place when you switch to another Plug-in
  supporting side-chain."

---

## 9. Mixer (Mix view) equivalents, for cross-checking

Left of the Mixer: buttons **IO** (show/hide audio and MIDI in/out), Plug-in icon, **AUX**.
Sound/Group strip, top → bottom:
1. Header (name, index; click index = mute, right-click = solo).
2. Audio input (Sound only): **AUDIO IN** + gain knob (= Input › Audio Source/Gain).
3. MIDI input: channel field + **MIDI IN** port field (= Input › MIDI Source/Channel). Shown as a
   **MIDI IN** on/off button in plug-in mode.
4. Plug-in List.
5. Fader section: pan slider (top), fader + meter + peak value (click resets), **Mute**,
   headphones (**Cue**).
6. Main audio output menu (= Output › Audio Dest.).
7. Aux: **AUX 1**/**AUX 2** menus + level knob + **Post**/**Pre** (= Output › Aux).
8. MIDI output (Sound only): **MIDI OUT** + channel field (= Output › MIDI Dest./Channel).

---

## 10. Gaps and open questions

- No numeric ranges or defaults in the manual for **Gain**, **Level**, **Pan**, Aux **Level**, or
  Swing **Amount** (only "%").
- Full **Cycle** value list not printed (taken from the decompiled table; display strings (?)).
- **Sound Input › MIDI** exact control set in Maschine 3 (Source list with Default/Focus? Start
  Note? Thru? plug-in-mode **Active**/Host?) is unclear. The table is written for Groups and the
  Mixer screenshot shows Default/Focus/None/All/ports.
- **Variation** and **Generate** Groove pages (Maschine 3) are absent from all NI manuals found.
  Only their existence, Sound-level scope and Apply/Generate buttons are confirmed (binary).
  Parameter labels, ranges and algorithms need RE or a live screenshot.
- `swing_bounce` has no documented UI.
- Whether Maschine blocks feedback-loop routings or only warns.
- Mono input entry names in Input › Audio › Source.
