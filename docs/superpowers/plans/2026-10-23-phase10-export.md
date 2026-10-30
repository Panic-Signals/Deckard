# Phase 10 — Export, bounce, clips

> Roadmap: `docs/superpowers/plans/2026-10-22-parity-roadmap.md`. Design:
> `docs/superpowers/specs/2026-10-23-phase10-export-design.md`. Inline, test-first; the
> software and the MK3.

**Goal:** get audio and MIDI out of DECKARD as Maschine does: File ▸ Export Audio (song,
sections, scenes, patterns; master or stems), Bounce (Sample to Sound), Export MIDI;
then clips in the Song view.

**Sources:** the MK3 manual's "Exporting audio", "Bouncing Sounds in place", "Exporting
audio / MIDI from Patterns"; the binary's strings (`AudioExportOptions`: AIFF, 16 bit,
24 bit, 32 bit float, the five rates, the MP3 bit rates, Master / Group / Sound, All /
Loop / Selected, Loop Optimize, Normalize, Split By Section); `PadModePageMK3.lua`
(SHIFT + button 6 BOUNCE, Ideas view only, `NI.DATA.GroupAccess.bounceSelectedSounds`);
`DialogMessage.txt` (`BounceInPlace`).

## Tasks
1. **Offline export** (`crates/engine/src/export.rs`): ranges, stems, render with the
   tail (Loop Optimize wraps it), WAV/AIFF at 16/24/32f, the `acid` loop chunk,
   Normalize, Maschine's file names and ` - n` suffixes, a cancellable multi-file
   export. Tests: tail and bar end, wrap, determinism, stems add up to the master,
   ranges, rates, decode round trip, names, no overwrite, cancel.
2. **Export Audio panel** (`apps/viewer/src/export.rs`, File ▸ Export Audio…): Song /
   Ideas variants, Source (Range or Scenes; Master / Group / Sound with the checked
   list), Options, Destination; a worker thread, progress and Cancel. Tests: the plan
   of files for each choice; an export from the app into a temp folder.
3. **Pattern audio** (the pattern editor's menu: Export Pattern Audio, Sample to Sound).
4. **Bounce** (Sound menu in the Ideas view; the confirmation and progress dialogs; the
   MK3's SHIFT + BOUNCE). Tests: the sound becomes an Audio plug-in with a take per
   pattern, each pattern cues its own, notes gone, one undo step restores it.
5. **Export MIDI**: the group's file a type 1 file with a track per sound (empty ones
   kept), Transpose; File ▸ Export MIDI… (song / scene). Tests.
6. **Clips** (study real projects first): the model reads them, the engine plays them,
   the document makes and edits them, the Song view shows and edits them.
7. **The MK3's Settings ▸ Audio tab** (`App:getNSA()`).
8. Docs (PARITY, this plan's results); the full check.

## Findings (clips)
Read with `mxprj-dump --decode` from copies of Maschine 3 projects (the user's "Rest in
Power", Mother Board's "Aardvark": 24 clips). A group (project level, class version >
10) holds `patterns_by_id` (its patterns) and then `patterns`, an
`ObjectVector<EventPattern>` of **clip patterns** (named "Clip 1", "Clip 1 (1)" …, each
with its own `length`, `start` and per-sound sequences), and `sequence`, a `Sequence`
whose `TimedEvent`s hold `ClipEvent`s: a `LengthEvent` (the clip's length on the
timeline) and `pattern`, a pointer into the clip patterns; the event's position is the
clip's start in the song. One clip pattern per clip (clips are unique).

## Results
- Tasks 1–8 done. `crates/engine/src/export.rs` (ranges, stems, the tail, WAV/AIFF,
  names, the multi-file export, bounce's render); `apps/viewer/src/export.rs` (the
  panel, its plan of files, the background job, File ▸ Export MIDI);
  `apps/viewer/src/bounce.rs` (Bounce, its dialogs, pattern audio, Sample to Sound);
  `crates/doc/src/audio.rs` (`bounce`); `apps/viewer/src/midi_file.rs` (type 1 files);
  `deckard_model::Clip`, `Group::clips` / `clip_patterns`; the engine's `add_clips`;
  `crates/doc/src/clips.rs`; `apps/viewer/src/clips_ui.rs` and the Song view's clip
  lanes; the host's `api/nsa.rs` and `apps/viewer/src/audio_settings.rs`;
  `GroupAccess.bounceSelectedSounds`.
- Missing: MP3; dragging audio/MIDI out of the window; editing a clip's notes and the
  clip pages on the MK3; the MK3 Settings MIDI tab's NSA calls (Phase 9's sync); the
  remaining Preferences pages; the footswitch; Live Slicing.

## Rulings
- **Loop range.** DECKARD has no loop braces in the Song view: Range ▸ Loop exports the
  section picked in the Song view (all sections when none is). If wrong: a range of
  several sections can't be chosen until loop braces exist.
- **Group and sound stems** are the channel's output: other channels muted, the master's
  effects bypassed and its fader at 0 dB; a sound's stem also bypasses its group's
  effects and fader. So group stems add up to the master when the master has no
  effects. If wrong: Maschine may tap stems after the master's fader.
- **The tail** ends when the output stays below −100 dBFS for half a second with no
  voices playing, at most 30 s after the range. With no tail, the file is exactly the
  range even when that isn't whole bars.
- **No dither** when reducing to 16 or 24 bit (rounded), so exports are bit-for-bit
  repeatable.
- **Normalize** is per file (each stem to its own peak).
- **File names:** the tempo is written `120 BPM` (`97.5 BPM`); the manual's `[BPM]` is
  not spelled out. Ideas view exports add ` - [Scene name]` after the Name; Split By
  Section adds ` - [NN] [Scene name]` (section number); these keep files apart (the
  manual doesn't say how Maschine names them).
- **Loop metadata:** an ACID `acid` chunk in WAV (tempo, beats, meter); AIFF files get
  none.
- **32 bit float AIFF** is written as AIFF-C (`fl32`).
- **MP3** is not offered (no encoder in DECKARD); listed as missing.
- **The panel's settings** last for the session; they aren't saved to DECKARD's
  settings file.
- **Rendering offline** uses its own sample cache and plug-in instances on a worker
  thread, so the playing engine isn't disturbed; plug-ins load again for the export.
- **Bounce** renders each pattern with notes for the sound, the sound alone before its
  fader (its effects in; its level and pan stay on the channel), Loop Optimize on (the
  take is exactly the pattern long, its tail wrapped round), at the audio output's rate
  (44.1 kHz when none is open), 32 bit float WAV into `Bounces/` next to the project
  (DECKARD's Music folder when unsaved), named `[Group] - [Pattern] [Sound] - [BPM]`.
  The sound's notes and MIDI events leave the bounced patterns. "Always Allow" lasts
  for the session. If wrong: Maschine may keep the notes (they don't play in Loop mode).
- **The MK3's SHIFT + BOUNCE** (`GroupAccess.bounceSelectedSounds`) asks in the
  software's dialog, not on the controller's displays (Maschine's `BounceInPlace`
  dialog is drawn natively). It bounces the focused sound (DECKARD has no multiple
  selection of sounds).
- **Pattern audio** (Maschine's audio dragger): the pattern menu's Export Audio writes
  into the Export Audio folder; Sample to Sound renders next to the project and loads
  the file into the group's first empty sound (as a dropped loop). The group's output
  (Group view) or the focused sound's (Keyboard view = the Piano Roll).
- **Export MIDI:** a group's file is type 1, a track per sound (named, empty tracks
  kept), each on its MIDI output channel and Transpose (Sounds to MIDI Notes: channel
  1, a note per sound, no transpose); a sound's file is type 0 on its channel,
  transposed. File ▸ Export MIDI… is DECKARD's: what the song (or, in the Ideas view,
  the focused scene) plays, swung, patterns repeated, a track per loaded sound named
  `A1 Group - Sound`.
- **Clips loop** their pattern to fill their length. Inside a clip, the group's section
  pattern is silent (its notes, controller events, loop cues and modulation passes that
  touch the clip go; a loop stop at the clip's start stays). A clip past the last
  section lengthens the song. Scenes (the Ideas view) don't play clips.
- **⌘.** converts the focused group's pattern in the picked section (Song view only);
  the section's scene keeps the pattern (the clip covers it there). A new clip is named
  `Clip n` (n: the group's clip count), copied with the pattern's notes, MIDI events,
  length, start and colour. **Double** doubles the clip's pattern (notes repeated) and
  the clip. **Duplicate** puts a copy right after it.
- **Patterns beside clips:** in a group with clips, its patterns in the Song
  view are drawn dimmer and its clips fully coloured (the manual's "Patterns … a
  slightly dimmed color state").
- **Settings ▸ Audio on the MK3** shows one driver (the platform's: CoreAudio, WASAPI,
  ALSA), the output devices, the device's own sample rate (DECKARD doesn't set the
  rate: the list has that one), DECKARD's buffer sizes, the latency as a buffer each
  way, and routing channels named `1`…`n` after `Off`. Changes go into the settings
  (saved) and reopen the output if it was open.
