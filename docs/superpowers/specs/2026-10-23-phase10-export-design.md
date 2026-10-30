# Export, bounce and clips: design (Phase 10)

October 2026. Roadmap: `docs/superpowers/plans/2026-10-22-parity-roadmap.md`, Phase 10.
Step plan and rulings: `docs/superpowers/plans/2026-10-23-phase10-export.md`.

## Why

A Maschine session ends in audio: the song or a scene printed to a file, stems for a
mix engineer, a sound bounced in place to free CPU or edit it as audio, a pattern taken
to a DAW as MIDI. DECKARD plays projects but can't get anything out of them.

## What Maschine does (the manuals)

- **File ▸ Export Audio…** opens *Export Song as Audio* (Song view) or *Export Ideas as
  Audio* (Ideas view). **Source:** Range All / Loop (Song), Scenes All / Selected
  (Ideas; one file per scene); Source Master (one file), Group (one per checked group;
  empty groups not listed), Sound (a tree of groups and their sounds, empty slots not
  listed). **Options:** Normalize (peak to 0 dBFS), Loop Optimize (exactly the range;
  the effect tail rendered into the beginning; tempo and length in the file's
  metadata; off: the file runs on to keep the tail, ending on a bar), Split By Section
  (Song: a file per section in the loop), Type WAV / AIFF / MP3, Sample Rate 44.1, 48,
  88.2, 96, 192 kHz, Bit Depth 16 / 24 / 32 float. **Destination:** Folder and Name
  (the project's name). Files: `[Project] - [BPM]`, `[Project] - [NN] [A1] [Group] -
  [BPM]`, `[Project] - [Sound] - [BPM]`; a name taken gets ` - 1`, ` - 2` … so nothing
  is overwritten.
- **The pattern editor's audio dragger** renders the selected pattern (the focused
  group, or in Keyboard view the focused sound) with the panel's settings, named
  `[Group] - [Pattern] - [BPM]` (Keyboard view: `[Group] - [Pattern] [Sound] - [BPM]`);
  dropped on a group it loads into its first empty sound slot.
- **Bounce** (Sound menu and the MK3's PAD MODE, SHIFT + BOUNCE; Ideas view only):
  after a confirmation (Allow Once, Always Allow, Don't Allow) and with a progress
  dialog (Cancel), each pattern playing the sound renders its notes to audio; the
  sound's plug-ins are replaced by an Audio plug-in whose takes are those renders, each
  pattern triggering its own. Its MIDI events leave the clips. Undoable.
- **Export MIDI…** (Sound and Group menus, the MIDI dragger) writes the selected
  pattern, each sound with its MIDI output Channel and Transpose; a group's sounds
  without notes are empty tracks so files line up across patterns.
- **Clips** (Song view): patterns placed freely on a sound's clip lane, ⌘. and Section ▸
  Convert to Clip(s) turn what plays into clips.

## Design

### Offline rendering (`crates/engine/src/export.rs`)

Pure functions over a `Project`, testable without a window:

- `Range` (Song, Sections a..=b, Scene, Pattern) becomes an `(Project, Mode)` to play:
  a section range is the project with only those sections (patterns restart at each
  section's start, as when playing).
- `Stem` (Master, Group g, Sound g s) becomes the project to render: other groups (or
  sounds) muted; a group stem drops the master's effects and plays the master at 0 dB
  centre (the group's output, as Maschine taps it); a sound stem also drops its group's
  effects and fader.
- `render(project, path, mode, rate, loop_optimize, cache)`: a fresh `Engine` at the
  export rate, not looping, rendered block by block (no workers, no rendering ahead:
  the same input gives the same samples) over the range, then on until the output
  stays below −100 dBFS for half a second with no voices (at most 30 s). Loop Optimize:
  the tail is added onto the start, file length = the range. Otherwise the file runs to
  the next bar line after the tail.
- `encode(audio, format)`: WAV (PCM 16/24, IEEE float 32; an `acid` chunk with tempo,
  beats and meter when Loop Optimize) or AIFF (16/24; 32 float as AIFF-C `fl32`),
  written by hand (no new dependency). Normalize scales each file to a 0 dBFS peak.
- `file_name(...)` and `unique_path(...)`: Maschine's naming and ` - n` suffixes.
- `export(plan, progress, cancel)`: every file of a plan, reporting progress and
  stopping when cancelled (a partly written file is removed).

### The app (`apps/viewer/src/export.rs`)

- **File ▸ Export Audio…** opens the panel (Song or Ideas title by the view), with
  Maschine's fields. The settings stay for the session. Export runs on a worker thread
  with its own sample cache; a progress bar and Cancel; the status line names the files.
- **The pattern editor's Export Pattern Audio** (no drag and drop out of the window in
  egui): the same render, Maschine's pattern names, into the panel's folder; and
  **Sample to Sound**: the render loaded into the group's first empty sound slot.
- **Bounce** in the Sound menu (Ideas view), the confirmation and progress dialogs;
  the edit is one undo step ("Bounce"): the sound becomes a lone Audio plug-in with a
  take per pattern, each pattern cueing its take, its notes removed from those patterns.
  Renders go to `Bounces/` next to the project (else DECKARD's Music folder).
- **MK3:** `GroupAccess.bounceSelectedSounds` asks the app to bounce the focused sound
  (the same confirmation, in the software).
- **Export MIDI:** the group's file becomes type 1 (a track per sound, named, empty
  ones kept), Transpose applied; File ▸ Export MIDI… writes the song or the focused
  scene the same way (a DECKARD addition: a track per group sound).

### Clips

Studied from real projects (read-only, `mxprj-dump`) before building; see the plan.

## Not now

MP3 (needs an encoder; listed as missing), dragging files out of the window, the
controller's export (Maschine has none), REX/MPC import.
