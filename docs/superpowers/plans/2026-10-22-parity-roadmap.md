# Feature parity roadmap

October 2026. The user's list (docs/PARITY.md), in the user's order, plus Variation /
Generation (Euclidean) and Ableton Link (GPL accepted). Each phase is its own project:
a short design (`docs/superpowers/specs/`) and a step plan (`docs/superpowers/plans/`)
written when it starts, built test-first, ending with the full check (`mise run lint`,
`mise run test`, `mise run check-platforms`). **Every phase works in the software and
on the MK3** (NI's scripts unchanged; the sweep `lua-host-run --press BUTTON_<page>`
shows no stubs or errors for the pages it touches).

Standing rules: clean-room (NI's scripts, strings and file data read for behaviour;
no NI code ported or bundled); tests never write to library folders or the settings
file; no subagents; commit only when the user asks.

## Phase 1 — Group and master effects
The engine runs group chains (`Group::slots`) on each group's bus and the master's on
the master bus, with their levels, mutes and bypasses; the chain area and Mix view
show and edit them (MASTER / GROUP / SOUND); the MK3 PLUG-IN page's MASTER and GROUP
load and edit them. Rendered-ahead sounds feed the group bus in time.

## Phase 2 — Recording notes, Note Repeat, arpeggiator
- Transport REC: pads, MIDI input and the computer keyboard (⌘K, two letter rows,
  Z/X octave, C/V velocity) record into the focused pattern; overdub; count-in
  (Preferences: bars); input quantize; the pattern grows when recording past its end
  (auto-grow); Record Prepare; one undo step per take.
- Note Repeat (rate, gate, swing; hold) and the arpeggiator (type, sequence, octaves,
  dynamic, gate, retrigger, rate, hold, presets) as an engine stage between input and
  sounds, in time with the song; what they play records like pads.
- Automation recording (holding AUTO, or Touch Auto-write) of knob moves into the
  pattern's modulation lane.
- MK3: REC, NOTE REPEAT (the arp page), AUTO, pads.

## Phase 3 — Variation and Generation (Groove)
Reverse-engineer `SequenceVariationModule` (Humanize, Random) and
`SequenceGenerationModule` (Euclidean generation, live mode) from project files and the
binary's strings: their parameters, how they change what a pattern plays, and their
knob pages under Groove; build them in the engine and the software, and on the MK3's
VARIATION page.

## Phase 4 — Hotkeys and the pattern editor's menu
Maschine's shortcuts: ⌘Z / ⌘Y take undo/redo, ⌘⇧Z / ⌘⇧Y step undo/redo, E/X/Y/Z mouse
modes (Draw, Split, Join, Mute), ⌘X/⌘C/⌘V for events, sounds and groups, Del for
sounds/groups/zones, ⌘R rename, ⌘A everywhere, ⌥←/→ nudge by the Nudge Grid, ⌘.
convert to clip (Phase 9). The event area's menu: the five modes, Quantize, Quantize
50%, Cut/Copy/Paste/Delete, Nudge Grid, Takes.

## Phase 5 — Perform FX
The eight Perform FX (Filter, Flanger, Burst Echo, Reso Echo, Ring, Stutter, Tremolo,
Scratcher) as DECKARD's own DSP, in a group's chain, driven by the Smart Strip
(touch on/off, position), recordable as automation; the MK3 PERFORM page.

## Phase 6 — Lock states
Parameter snapshots: up to 64, store, recall, Update, Extended Lock (per group/sound),
morph over time, MIDI program-change recall; stored in the project as Maschine stores
them; the software's Lock panel and the MK3 LOCK page.

## Phase 7 — Macros
MIDI learn (macros and any parameter), host automation of macros (plug-in mode,
later), the MK3 MACRO page, the macro context menus (Rename ⌘R, Learn, Reset,
Cut/Copy/Paste) and the Pages menu (Delete, Clear All).

## Phase 8 — Context menus
Pattern: Clear, Rename, Double, Delete. Scene and Section: Rename, Clear, Unique,
Color, Scene Tempo; Section: Create, Convert to Clip(s). Sound/Group: Cut/Copy/Paste
for groups, Delete (and their keys, Del / ⌘X / ⌘C / ⌘V in the Group list, from Phase
4), Bounce (Phase 10), Find/Purge Missing Samples. MK3: SCENE's
Unique and Scene Tempo; pad-mode settings for several sounds; key switches.

## Phase 9 — Ableton Link and MIDI clock
Link (Ableton's C API, `abl_link`, GPL-2.0: accepted by the user) for tempo, beat
and start/stop sync; MIDI clock send and receive (Preferences ▸ MIDI ▸ Sync); the
header's Link button; the MK3 shows the tempo as followed.

## Phase 10 — Export, clips, the rest
Export Audio (song, scene, pattern; stems per group/sound), Bounce / Sample to Sound,
Export MIDI; Clips in the Song view (Convert to Clip, ⌘.); the MK3 Settings audio-
device page; remaining Preferences pages; footswitch; Live Slicing.

## Out of this roadmap
Maschine as a VST3/AU plug-in in a DAW; NKS; the cloud sample service (its own
project, later).
