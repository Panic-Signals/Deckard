# Sampler Preview: three modes

## Why

DECKARD's sampler has grown three jobs that share one set of zones: Maschine's own
sampler, the "Sampler Preview" (Sync and Keys, stems on lanes, reverse, slices), and
layers (takes morphed between). Nothing keeps them apart. A sound is "layered" only while
every zone happens to carry layer data, so an edit that writes a new file (NORMALIZE,
TRUNCATE) drops that layer's data, and with two or more layers the LAYERS tab with it; separating stems on a layer would do
the same. And a take can only be edited once it is the sound's sample, so "pick a take,
trim it, put it on a layer" can't be done.

Josh: make them three distinct samplers in one plug-in; choose the mode, then work. The
plug-in is Sampler Preview; the classic Sampler stays as it is beside it for now (two
versions of Maschine's sampler, on purpose).

## Goals

- Sampler Preview has a MODE on the sound: **Original**, **Advanced**, **Layers**.
- The classic Sampler is untouched.
- What a sound offers depends on its mode alone, never on what its zones hold.
- In Layers: choose a take, edit it, give it a layer, from the app and from the MK3.
- Changing mode never loses work silently.

## Non-goals

- New sampler features in Original or Advanced: they are what exists now, fenced off.
  (Layers gets one new thing, editing a take that is on no layer.)
- The Audio plug-in: unchanged.
- Each mode remembering its own contents (switching back does not restore).

## Anti-goals

- An edit, a stem separation or any other action that takes a sound out of its mode.
- An Original-mode sound that plays differently in DECKARD than the classic Sampler
  would play the same zones.

## Approaches

1. **A mode stored on the sound (picked).** `NewSampler` gains a mode (Original,
   Advanced, Layers). Every "can this sound do X" asks the mode. Old projects need no
   conversion: a mode is read from what they hold.
2. **Three plug-ins in the add menu, no switching.** Least code, but a sample recorded in
   one can't be taken to another, and Josh asked for a mode.
3. **Each mode keeps its own contents.** Nothing is ever lost on a switch, but a sound
   carries three hidden sets of zones. Josh chose a warning instead.

## Design

### The modes

| | Original | Advanced | Layers |
|---|---|---|---|
| Play modes (Sync, Keys), reverse, speed, tail | no (Repitch) | yes | no (Repitch, as layers are today) |
| Stems on the zone's lanes | no | yes | no |
| SLICE tab | yes (Maschine's) | yes | no |
| LAYERS tab, MORPH | no | no | yes |
| RECORD's LAYER and APPLY | no | no | yes |
| Several files dropped at once | as for a classic Sampler | as before layers existed | a layer each |

- Original does what the classic Sampler does with the same zones and no more: Repitch
  only, no stems, no reverse or speed, Maschine's slicing. (It is still a Sampler
  Preview: its mode is DECKARD's, kept in the sound.)
- Advanced is today's Sampler Preview with the layers taken out.
- In Layers every zone is a layer: a sample that comes in (a file, a take, the one kept
  on a mode change) is given layer data then, so the engine needs nothing new. There is
  no "plain sample shown as layer 1" any more, and the sound stays in Layers with none,
  one or eight of them. (This replaces the 2026-11-24 design's "the first take stays the
  sound's plain sample so SLICE works": in Layers there is no SLICE.)
- Entering Layers does what making the first layer does today (the envelope ADSR, Motion
  off, every key).
- What a sound may do is asked of its mode everywhere: the tabs, file drops, take
  placement, stems onto lanes, the MK3's view (which learns the mode even for an empty
  sound).
- Original turns off everything that is Sampler Preview's own, as a classic Sampler has
  it; analysis doesn't put its zones in Sync.
- Slices to pads and Key Shift make sounds in the source's mode (a classic Sampler's:
  Advanced, as today).
- STEMS in Layers: onto a new group only (the lanes are Advanced's).

### Names

- The add menu stays `Sampler`, `Sampler Preview`, `Audio`, and "Convert to Sampler
  Preview" / "Convert to Sampler" stay. [Josh]
- A new Sampler Preview is in Advanced (what it is today) [Claude's call]; one converted
  from a Sampler is in Original (it plays as it did); one converted from an Audio
  plug-in is in Advanced (it needs Sync).

### Projects made before this

Read, not converted: a Sampler Preview with no mode written is Layers if it has a zone
with layer data, else Advanced. The mode is written the next time the sound's settings
are (a sound still at every default writes none). Classic Samplers are not touched.
So a Sampler Preview made before this with one sample or none opens in Advanced, without
the LAYERS tab it showed: MODE gets it back.

### MODE and changing it

- It is called SAMPLER (RECORD already has a MODE: the recorder's) [Claude's call].
- App: a three-way SAMPLER selector in the Sampling view's header (always visible, whatever
  the tab) [Claude's call].
- MK3: a SAMPLER knob on RECORD's knob pages (RECORD is the one tab every sound has)
  [Claude's call]. Turning it only chooses; nothing changes until confirmed.
- A sound without a sample: the mode changes at once.
- A sound with a sample: a confirmation first. [Josh]
  - App: a dialog, "Changing the mode loses all changes to this sound.", `OK` and
    `Cancel`.
  - MK3: the same sentence across the displays, `OK` and `CANCEL` on two display
    buttons; the knob's choice is dropped on CANCEL, on leaving the page and when the
    focused sound changes.
- On OK, as one undo step ("Sampler Mode"): the sound keeps **the sample it was showing**
  [Josh] (the one in the Sampling view: the focused zone's or layer's, or in Layers the
  take being edited) as its only zone, the whole file, on every key, with the new mode's
  default settings. Slices, the other zones or layers, stems, play modes, tempo and root
  set by hand, the morph and the layer settings go. In Layers that zone is layer 1.
- Undo brings all of it back.
- One exception [Claude's call]: Original to Advanced loses nothing, so it keeps every
  zone and asks nothing.
- The recorder's take list is not the sound's: it stays through a mode change.
- While recording or armed, MODE can't be changed.

### Editing in Layers

- EDIT works on what was picked last: a take in the take list, or a layer on LAYERS. The
  waveform and the name show which.
- A take that is on a layer and that layer are the same sample: either way in, the same
  edit.
- An edit on a layer: the layer plays the edited file and keeps everything else (mute,
  solo, envelope, window, level, tune, play mode; tempo and beats follow the new length
  as they do for a zone with stems). One undo step.
- An edit on a take: the take list's entry becomes the edited file (the original stays on
  disk in Recordings). If the take is on a layer, that layer follows as above.
- RECORD's LAYER knows a take and its edited copies (`name - N.wav`) are the same take,
  whichever of them the list or the layer holds: so it is right after an edit, and after
  that edit is undone.
- Editing a take on no layer is new: the Sampling view shows a file that is no zone's.
- ZONE (loop, play range, tune) needs a zone, so it is the picked layer's; a take on no
  layer has EDIT and RECORD only.
- An edit never changes the sound's mode. The same holds in Advanced for a zone with
  stems or slices, as today.

### The MK3

- Tab button 4: `ZONE` over a small `LAYERS` (swapping) in Layers mode; `ZONE` alone in
  the others. LAYERS is reachable in Layers mode even on an empty sound.
- RECORD: the LAYER knob and APPLY (button 8) in Layers mode only.
- Controllers run by NI's own scripts: MODE is changed from the app.

## Claude's calls (not asked)

- A new Sampler Preview is in Advanced; one converted from a Sampler is in Original.
- Original to Advanced keeps everything and doesn't ask.
- The control is named SAMPLER, in the Sampling view's header and as a knob on RECORD's
  pages on the MK3.
- A Sampler Preview from an older project with one sample or none opens in Advanced.
- Original to Advanced leaves loops in Repitch; Sync is then chosen by hand.
- An edit of a take on no layer is not an undo step of the project (the take list isn't
  part of it); the original file stays in Recordings.
- "Convert to Sampler" (leaving Sampler Preview) is as today: it keeps the zones and
  drops what is DECKARD's, whatever the mode.

## Left to the builder

- The mode's name in `NewSampler`'s text and in the controller API (a `mode` field of the
  sampling view; a call to choose and to confirm).
- How the MK3 page draws the confirmation.
- The app's dialog and the MK3's confirmation are new (neither has one to reuse here).
- The status and hover texts that say what a Sampler Preview can do.
- Tests: mode read from old projects; each mode's tabs and knobs; the confirmation (OK,
  Cancel, undo); what is kept; an edited layer stays a layer; an edited take stays on its
  layer; a take on no layer edited then applied.
- `docs/controller-api.md`, `docs/sampling-controller.md`, and the manual (the
  `pull-request` skill asks).
