# Feature parity with Maschine 3

What DECKARD still lacks against Maschine 3 (software and MK3), October 2026. Sources:
the Maschine 3 software manual and the MK3 manual (2026), `docs/context-menus.md`, a
sweep of every MK3 mode button through `lua-host-run` (the host functions NI's scripts
call that DECKARD only stubs), and the code.

Legend: **missing** = nothing there; **shell** = the controller page shows (NI's
scripts run) but nothing behind it works; **partial** = some of it works.

## The list asked for

| # | Feature | State | What's there / what's missing |
|---|---|---|---|
| 1a | **Audio recording** | partial | Sampling ▸ RECORD works (Detect, Sync, Loop, Auto; software and MK3), and Loop takes land in an Audio plug-in. Missing: punch/re-record of takes onto a track while the song plays, an Audio plug-in take list (Audio Pool), `Recorded takes` on the controller (`api/data.rs:133`). |
| 1b | **MIDI / pad recording into patterns** | done | Pads, MIDI and the computer keyboard (⌘K) record into the playing patterns; REC arms (PLAY starts it), SHIFT + REC counts in, Auto length (a bar, growing as played into, settling on the bars played), input quantize, one undo step per take; MK3 REC (`docs/recording-notes.md`). Modulation (automation in patterns) plays and records: ring drag / Auto-write in the software, AUTO on the MK3 (`docs/modulation.md`). Capture follows Maschine (`docs/superpowers/specs/2026-10-25-recording-capture-design.md`): notes go in at note-on and stretch while held (held over the loop end too), the count-in's last 1/16 records onto the start, doubles on a quantized step are replaced, a pad pressed again ends its earlier note. Missing: Replace mode (ERASE + REC). |
| 2 | **Perform FX** | done | The eight (Filter, Flanger, Burst Echo, Reso Echo, Ring, Stutter, Tremolo, Scratcher) as DECKARD's own DSP, first in a group's chain (+ ▸ Internal ▸ Perform FX; SHIFT + PERFORM on the MK3), Main / Setup pages per mode; the Smart Strip (ENGAGE and the position knob in the software, the MK3's strip in PERFORM mode) plays it and Auto-write records it as modulation (`docs/superpowers/plans/2026-10-22-phase5-perform-fx.md`). The sound is DECKARD's, not Maschine's. |
| 3 | **Macros** | done | Macro pages in every channel (Master, Group, Sound), targets from each channel's properties, Sampler and internal effects (and those of the channels under it); the Assignment area (the arrow under the Control area's icons): Pages (each macro's target selector, the slot menu Rename ⌘R / Learn / Reset / Cut ⌘X / Copy ⌘C / Paste ⌘V, Section), MIDI (Learn: CC, PC, PW, channel and poly pressure on any port and channel; Unlearn) and Host (automation IDs) for any parameter and for macros; the page tabs' menu (Rename, Delete, Clear All); stored as Maschine stores them (`MidiMapper`, the 128 `remote_parameters`). MK3: MACRO (the Macro properties, its knobs turning the macros), SHIFT + MACRO Set Macro (touch a knob: macro of MASTER / GROUP / SOUND; LEFT / RIGHT turn the pages meanwhile and MACRO leaves it, DECKARD's; the macro icon on knobs a macro controls) (`docs/superpowers/plans/2026-10-23-phase7-macros.md`). Host automation waits for a plug-in build (`App::host_automate` is its entry point); external plug-ins' parameters can be made macros from the MK3 (SHIFT + MACRO on the plug-in's page; their pages are saved as Maschine saves them), and from the window (right-click a plug-in knob's name); a macro of any channel turns them, and the Assignment area's MIDI tab learns a control for them (the pages saved as it learns, one undo step); a macro's target menu lists them too, under the plug-in, its page and the knob (the plug-ins of the channels the macro reaches, running ones). |
| 4 | **Lock states** (parameter snapshots) | done | Up to 64 snapshots of every modulatable parameter (mixer strips, swing, slots on/off, every module's and plug-in's parameters; level/mute pairs as Maschine), stored in the project as Maschine stores them (its Maschine 2 snapshots read too); LOCK, Extended Lock (pads, banks, UPDATE, DELETE), Morph (Travel over Time, Target to the next Grid), MIDI Lock Change (Edit ▸ MIDI Change, notes or program changes); the software's Lock panel (View ▸ Lock) and the MK3 (`docs/superpowers/plans/2026-10-22-phase6-lock.md`). MIDI Change's Scene and Section parts are still missing. |
| 5a | **Note Repeat** | done | Engine repeater, MK3 NOTE REPEAT page, software panel, recorded (`docs/note-repeat-arp.md`). |
| 5b | **Arpeggiator** | done | Types, octaves, retrigger/repeat/offset/inversion, sequences (DECKARD's rhythms), hold, lock, presets; MK3 and software (`docs/note-repeat-arp.md`). |
| 5c | **Euclidean** | done | Maschine 3's Groove ▸ Generate (Mode: Euclidean; Steps, Fills, Rotate, Division, Live), with Random beside it and Variation (`docs/channel-software.md` §5). Software pages and Apply/Generate; on the MK3, VARIATION shows Groove ▸ Variation and the channel pages reach Generate. DECKARD's own rule: with a scale selected, Generate's Random keeps every note in it. |
| 6 | **Group (and master) effects** | done | Group buses and the master chain play; the GROUP/MASTER tabs, the Mix view and the MK3 PLUG-IN page edit them (`docs/engine-design.md`). |
| 8 | **Ableton Link** | done | Ableton's Link 3.1.5 (`crates/link`, vendored, GPL-2.0-or-later, built with `cc`): the header's LINK (its phase bar, peers in its tooltip) joins the session; tempo both ways (a session's tempo becomes the song's without an undo step), PLAY waits for the session's next bar, Start Stop Sync both ways. MIDI clock (Preferences ▸ MIDI ▸ Sync: Off / Send Clock / Receive Clock, Clock Offset): sent to the outputs switched on (Song Position, Start / Continue, 24 per beat, Stop; timed to the audio), followed from the inputs switched on (tempo, Start / Continue / Stop, Song Position; PLAY greyed). The MK3's Settings ▸ MIDI page (mode, offset, devices, Link and START SYNC, the Link status and phase meter) is the app's (`docs/superpowers/plans/2026-10-23-phase9-link-midi-clock.md`). Not built in the cross-platform type-check (a stand-in without the network). |
| 10 | **MPE and per-note expression** (beyond parity: Maschine has none) | partial | Part 1 of 4, capture and keep: MPE inputs (Preferences ▸ MIDI, per port, with a member bend range), poly pressure (the Hydrasynth, a Launchpad) and the MK3's pad pressure (each sound's Pad Touch: Velocity, Poly AT, Aftertouch or a CC, with a curve, Min and Max, on the MK3's Keyboard page by DECKARD's built-in mod; `docs/superpowers/specs/2026-10-24-pad-touch-design.md`) record each note's pressure, slide, pitch and release velocity as curves on it (thinned to breakpoints); split and join slice and join them; channel pressure, CC and pitch bend record into lanes, overwriting as they're written (touch, 250 ms); a dot marks a note with expression. Kept in the DECKARD block Maschine ignores (`docs/deckard-block.md`). Pad Touch's Aftertouch reaches a sound's MIDI output but not its plug-in or DECKARD instrument (no channel pressure to instruments yet), and Poly AT isn't sent to Note Repeat's or the Arp's repeated notes. Part 2a, sending it out to MIDI (`docs/superpowers/specs/2026-10-24-expression-out-design.md`): recorded and live per-note expression plays out of MIDI ports — a normal port gets each note's pressure as poly aftertouch and its release velocity; a port set to MPE (Preferences ▸ MIDI, with a bend range) gives each note a member channel (2–16) with its starting bend, CC74 and pressure, then its curves, channel-wide lanes on channel 1, the oldest note stolen past 15, channels reset on stop or a format change; DECKARD sends the synth its MPE set-up (Send MPE setup to synths, on by default). Part 2b, the modulation matrix's base: a sound's routes (the Assignment area's MOD tab under each knob, and its ROUTES list) let its notes' Pressure (hardest-pressed note), Slide or Pitch (latest note), live or recorded, drive any continuous parameter of its chain — the Sampler, a drum synth, an internal effect, a hosted plug-in's parameter — between a Low and a High (inverted when Low is above), along a curve, Pitch over ± a range; kept in the DECKARD block. Part 3, the Expression editor (`docs/superpowers/specs/2026-10-25-expression-editor-design.md`): the pattern editor's lane list gains Pitch, Slide, Pressure and Release (listed once the sound's notes have expression, offered in the "+" dialog to draw from scratch); curves are drawn over each note's span, Ableton-style: click adds a breakpoint, drag moves a point or a segment, ⌥-drag bends a segment (played back bent), double-click removes a point, draw mode draws freehand (thinned); Release is a bar per note; right-click clears a dimension or all expression of the selected (or all) notes; the Pitch lane's range is ±2/12/24/48 st, and the piano roll draws each note's pitch curve over it. Missing: assigning routes from the MK3, other sources (macros, knobs, LFOs: `docs/FEATURES.md` #1, #9), group and master targets, per-note expression to MPE plug-ins, editing expression on the MK3, DECKARD's sounds and macros responding per voice (part 4) (`docs/superpowers/specs/2026-10-24-mpe-capture-design.md`). |
| 9 | **Maschine's hotkeys** | done (but a few list keys) | See below; Phase 4 (`docs/superpowers/plans/2026-10-22-phase4-hotkeys.md`). |

## Hotkeys (software)

What Maschine has (manual) vs DECKARD:

| Shortcut (macOS / Windows) | Maschine | DECKARD |
|---|---|---|
| ⌘Z / Ctrl+Z | Take Undo | ✓ (a recording or modulation take is one step) |
| ⌘Y / Ctrl+Y | Take Redo | ✓ |
| ⌘⇧Z / Ctrl+Shift+Z | **Step** Undo | ✓ (one edit, inside a take too); MK3 SHIFT + pad 3 |
| ⌘⇧Y / Ctrl+Shift+Y | Step Redo | ✓; MK3 SHIFT + pad 4 |
| ⌘F | Full screen | ✓ |
| F4 | Show/hide Browser | ✓ |
| ⌘K | Computer keyboard as MIDI (Keys): two letter rows play C3–F4; Z/X octave, C/V velocity; overlay | ✓ |
| ⌘. | Convert the selected pattern to a clip | ✓ in the Song view: the focused group's pattern in the picked section becomes a clip over it (Phase 10) |
| ⌘A | Select all (event area, Sound/Group lists, Zone list) | event area only (DECKARD has no multiple selection of sounds or groups) |
| ⌘X / ⌘C / ⌘V | Cut/Copy/Paste events, Sounds, Groups, macros | events ✓ (Maschine's paste rules, but the stopped playhead's: DECKARD has none), Sounds ✓, macros ✓ (the focused macro in the Assignment area's Pages tab; ⌘R renames it); Groups missing (Phase 8, with the Group menu's) |
| Del / Backspace | Delete events, Sound reset, delete Group, delete Zones | events ✓, Sounds ✓ (reset); Groups, Zones missing (Phase 8) |
| ⌥← / ⌥→ | Nudge selected (or all) events by the Nudge Grid | ✓ round the pattern; the MK3's nudges use the same Nudge Grid |
| E / X / Y / Z | Mouse modes: Draw / Split / Join / Mute (toggle with Select) | ✓ (not while Keys plays notes); muted notes are probability 0, drawn grey |
| ⌘R | Rename (parameter, objects) | Sounds and Groups ✓ (the list clicked last); parameters missing (Phase 7) |
| Enter / Esc | Confirm / cancel naming fields | ✓ |
| Space | Play/stop | ✓ |
| ⌘N/⌘O/⌘S/⌘⇧S/⌘,/⌘Q | File and app | ✓ |

## Context menus (software)

From `docs/context-menus.md` against the code:

- **Sound:** missing Find/Purge Missing Samples. (Open, Save, Save As, Save with Samples, Bounce (Ideas view; Allow Once / Always Allow / Don't Allow, progress, Cancel; one undo step), Export MIDI: done.)
- **Group:** greyed Cut, Copy, Paste, Delete. (Open, Save, Save As, Save with Samples: done.)
- **Pattern:** missing Clear, Rename, Double; Delete greyed.
- **Scene:** missing Clear, Rename, Unique, Color, Scene Tempo (the MK3 also stubs `isSceneUnique`, `isSceneTempoAvailable`).
- **Section** (Song view): missing Create, Clear, Rename, Unique, Color, Scene Tempo. Convert to Clip(s): done.
- **Event area** (pattern editor): done: Select/Draw/Split/Join/Mute, Quantize, Quantize 50%, Cut/Copy/Paste/Delete, Nudge Grid. Takes missing (no takes of notes).
- **Clips:** Duplicate, Double, Delete (the Song view's clip menu); dragging moves and resizes. Missing: Clear, Rename, Color, editing a clip's notes in the pattern editor.
- **Plug-in slot menu:** Presets submenu for VST/AU presets, Save Plug-in preset.
- **Macro parameter / page names:** done (the macro slot menu, the page tabs' menu; see Macros).

## The MK3: patterns and events

The sweep presses each mode button and its 16 buttons and 8 knobs; what NI's scripts
then ask for and DECKARD only stubs:

- **Pattern / Events / Step / Variation / Select / Duplicate / Swing / Fix Velocity /
  Follow / Pitch / Notes:** run with no new stubs beyond accessibility calls; the step
  editor, events editing and audio-loop lanes work (`docs/controller-steps.md`).
  VARIATION shows Groove ▸ Variation (`setFocusPageOnGroovePageGroup`). Missing behind
  them: the 128th-step guess, DOUBLE as one undo step.
- **REC + pads (record notes), Note Repeat, Arp, Lock, Perform, Macro:** see above.
- **Pad mode:** base key, choke and link groups' multi-selection (`StateHelper.getIs*MultiSelection` stubs: wrong with several sounds selected); Keyboard: key switches (`Sound.isKeySwitch`), `resetKeyboardPadViewRoot`.
- **Mod (Smart Strip modulation):** `getLastTouchstripValueForSound` stub — Smart Strip
  modes (Pitch Bend, Mod, Perform, Notes) need checking end to end.
- **Browse:** done (`docs/browser.md`): NI's page on DECKARD's library, 0 errors.
- **File:** recent files, new project, project-changed state.
- **Settings:** the Audio tab works (`App:getNSA()`: driver, device, the device's rate,
  buffer size, input and output routing, from and into Preferences ▸ Audio). The MIDI
  tab's NSA calls (devices, sync) are still stubs.
- **Sampling:** works; its browser corner (load a sample from the controller's browser)
  hits the Browse stubs.
- **Arrange:** `Widget.InfoBar` missing (the info line).

## Other manual chapters not done

- **Clips** (Phase 10): read from and saved in projects as Maschine stores them (a group's
  `patterns` and `sequence` of `ClipEvent`s), played in the arrangement in place of the
  section's pattern, ⌘. and Convert to Clip(s), moved, resized, duplicated, doubled,
  deleted. Missing: editing a clip's notes, Clear/Rename/Color, creating an empty clip,
  the MK3's Clip pages (SHIFT + PATTERN), APPEND of scenes as clips, loop-brace
  conversion (DECKARD has no loop braces).
- **Browser:** done (`docs/browser.md`); left: the tag editor, Quick Browse, banks under
  products, NI's product artwork on the MK3.
- **Export Audio** (Phase 10): done: File ▸ Export Audio (Song: All / Loop (the picked
  section), Split By Section; Ideas: All / Selected scenes; Master, Group or Sound stems;
  Normalize, Loop Optimize, WAV / AIFF, 44.1–192 kHz, 16 / 24 / 32 float; Maschine's
  names), rendered offline in the background; the pattern menu's Export Audio and Sample
  to Sound; Bounce; File ▸ Export MIDI (the song or scene) and type 1 group MIDI files.
  Missing: MP3, dragging audio or MIDI out of the window, REX and MPC import. (Save
  with samples: done for sounds and groups.)
- **Modulation** (`docs/modulation.md`): left are the Control Lane's modulation pane
  (seeing and drawing points), Step mode modulation, instruments' and the Sampler's
  parameters, channel properties, curves; and host automation.
- **MIDI:** MIDI mode on the MK3 (as a MIDI controller), MIDI Control of parameters
  (learn), Program Change, Groups/Sounds receiving MIDI notes (partial: `midi_io.rs`).
- **Plug-in mode** (Maschine as VST3/AU in a DAW) and host sync.
- **Preferences:** Default, Audio (partial), MIDI, Library, Plug-ins, Hardware, Colors
  pages.
- **NKS** presets and the light guide; **footswitch** (the MK3's PEDAL socket isn't
  decoded: its HID report needs reading with a pedal attached); **Live Slicing**.

## Order suggested

1. Group and master effects in the engine (the model already has them; Perform FX need them).
2. Pad/MIDI recording into patterns, with count-in and input quantize, then Note Repeat
   and the arpeggiator (they record through the same path).
3. Hotkeys (cheap; fix the ⌘⇧Z conflict) and the event-area context menu and mouse modes.
4. Perform FX on the Smart Strip.
5. Lock snapshots.
6. Macros on the controller, MIDI learn.
7. Missing context-menu items (Rename, Clear, Double, Unique, Color, Scene Tempo).
8. Ableton Link and MIDI clock (done: GPL-2.0 accepted).
9. Export audio/MIDI, clips, the browser on the controller, Preferences pages.
