# Session recording to a file

**Status:** design, awaiting approval
**Date:** 2026-11-16

## Intent and why

DECKARD has no easy way to record a performance to an audio file. Today you'd route
the outputs into something else and splice it together, which is fiddly enough that
the user avoids it live.

This feature is a one-button capture of DECKARD's master output to a WAV file, meant
for:

- **Live sets:** start it before you play, let it run the whole show, post the file
  later or sync it to video.
- **Happy accidents:** grab a jam you like before it's gone, to chop up and rebuild
  later.

It is deliberately *not* the sequencer's recording (patterns, the arranger) and not
the Sampling recorder (which records into takes inside the project). This records what
comes out of the speakers, straight to a file on disk, and nothing about the project
changes.

## Goals

- Record the **final master output** — exactly what you hear, through the master FX
  and limiter — to a WAV file.
- Run **independent of the transport.** It starts when you start it and stops when you
  stop it, capturing everything in between — playing, stopped, tweaking, silence —
  as one continuous real-time recording.
- Be **trustworthy at a glance** during a live set: an obvious window showing it's
  recording, how long it's been going, that signal is arriving, and that the disk
  won't run out.
- **Never leave a corrupt or empty file,** even on a crash or power loss.
- Be easy to find and impossible to confuse with the other record features.

## Non-goals

- **No video.** Camera/webcam/iPhone capture was discussed and explicitly cut from
  this release. Nothing here should preclude adding it later, but it is not in scope.
- **No recording into the project.** This never creates takes, patterns, samples, or
  edits. It only writes a file.
- **No per-track / stem / multi-output recording.** The master only, as a stereo pair.
- **No editing, trimming, or playback** of the recording inside DECKARD.
- **No compressed formats** (MP3/AAC/FLAC). WAV only.

## Anti-goals (these would make it a failure even if it "worked")

- Recording a two-hour set and finding the file empty, truncated, or corrupt.
- Filling the disk mid-set so DECKARD or the OS crashes.
- A second take silently overwriting the first.
- Being mistaken for, or interfering with, pattern/arranger/Sampling recording.

## Decided behaviour

### Where it lives

- **File ▸ Record Session to File…** — a new menu item directly under "Export Audio…",
  so it sits with the other "audio out to disk" command while staying clear of the
  sequencer's record (which lives on the transport/controller, not in these menus).
- While recording, that same menu line reads **Stop Session Recording**.
- **View ▸ Session Recorder** toggles the window's visibility without stopping the
  recording.
- No keyboard shortcut in this release (can be added later).

### Starting

1. Choosing "Record Session to File…" opens a save dialog.
2. The default filename is `session-<project>-<date>-<time>.wav`
   (e.g. `session-NightSet-2026-11-16-2104.wav`). If the project is unsaved and has no
   name, drop that segment: `session-2026-11-16-2104.wav`. Date and time are always
   included so a second take the same day never clobbers the first, and the timestamp
   helps line the file up with external video later.
3. It remembers the last folder used (persisted in settings) and defaults the dialog
   there.
4. Before committing, it reads free space on the chosen drive and shows an estimate of
   how long it can record in this format (~1 GB/hour at 48 kHz / 24-bit stereo), minus
   the safety reserve: e.g. "~2.5 hours, then it auto-stops to protect the drive." If
   space is already below the reserve, warn and don't start.

### What it captures

- The **master output after its level and pan** — `deckard_engine::TapSource::Master`,
  which is exactly what the Sampling recorder's "Internal: master" source uses.
- **24-bit, stereo, at the engine's current sample rate** (no resampling). WAV, integer
  samples.
- **Continuous real time, independent of the transport.** The file's duration equals
  wall-clock time from Start to Stop. Stretches where nothing is playing are recorded
  as **silence** so the timeline stays true (important for syncing to video). See the
  key risk below for why this needs care.
- **If the audio device or sample rate changes mid-recording** (e.g. the user switches
  interface or buffer in Preferences), the session recording **stops cleanly and
  finalizes** the current file rather than risk a corrupt file or a rate discontinuity
  inside one WAV — the same safe behaviour as the disk-reserve and 4 GB stops. *(My
  call, not the user's — a sensible default a builder could also have made; flagged here
  so it's visible.)* Starting a new recording after the change picks up the new rate.

### The window (chosen mock-up: "version B")

A separate always-recording-is-visible window containing:

- A pulsing red "recording" dot and a "RECORDING SESSION" label.
- A large elapsed **timer** (`HH:MM:SS`), with the start time beneath it.
- A live **scrolling waveform** of the captured signal.
- **Stereo level meters** (L/R) with a **peak dB** readout.
- A line showing format and **space used / remaining** ("48 kHz · 24-bit · stereo",
  "132 MB on disk").
- A **Stop** button.

The meter area turns amber, then red, as free space approaches the reserve.

Rendered as a second egui viewport (the app already uses `show_viewport_immediate`
for the splash screen; the recorder window is the same idea as a persistent viewport).

### Disk safety

- **Reserve:** keep at least **1 GB** of free space on the drive (adjustable in
  Preferences ▸ … , default 1 GB). As the recording approaches the reserve, **stop
  cleanly and finalize** the file. Never record the drive down to nothing.
- **Crash-safe:** the WAV is finalized as it grows (header/size fields kept valid, data
  flushed), so a crash or power loss still leaves a playable file of everything up to
  that point.
- **4 GB WAV ceiling:** a single WAV cannot exceed 4 GB (~4 hours at this format). The
  user accepted this limit (sets won't realistically reach it) and chose **not** to
  split into multiple files. To keep the ceiling from corrupting the file, as the
  recording nears 4 GB it performs the same clean stop-and-finalize as the disk
  reserve. One continuous file; no part numbers; no RF64.

### Stopping

- Stop (from the window, the menu, or an auto-stop) finalizes the file and closes it.
- The window can then be dismissed; starting again opens a new file.

## Constraints and context

- DECKARD is a Rust workspace; the app is the `deckard` binary (`apps/viewer`), egui/
  eframe UI, `deckard_engine` for audio.
- `hound` is already a dependency and is the WAV writer used elsewhere.
- The engine tap already exists: `deckard_engine::{Tap, TapSource}`;
  `Output::set_tap(Some(Tap { source, buffer }))` fills an
  `Arc<Mutex<Vec<f32>>>` with interleaved stereo frames that a consumer drains. The
  Sampling recorder (`apps/viewer/src/recorder.rs`) drives it from its `poll`.
- Testing follows the repo norm: targeted tests per change, full suite once at the end.
  New logic (filename building, space/time estimation, reserve and 4 GB cut-offs,
  silence-gap filling, WAV finalize-as-you-go) should be unit-testable without audio
  hardware.

## The key technical risk (flag for the plan)

**Keeping audio flowing to the file when the transport is stopped.** The engine tap
only produces frames while an engine is actually rendering (`Output::play(engine)`).
When DECKARD is fully idle — no transport, no held notes — there may be no engine
running, so no frames arrive at the tap. A naive consumer would then write *nothing*
for those gaps, making the file shorter than real time and ruining sync.

The recording must instead keep the file's length equal to elapsed wall-clock time:
when no frames have arrived for an interval, write the matching number of **silent**
frames. The builder should decide the mechanism (e.g. a session-recording clock that
pads with silence from the sample rate and elapsed time, and/or keeping an output
stream alive for the session's duration) and must verify it against: start recording
with the transport stopped, wait, play a bit, stop, wait, Stop — the file should be
continuous and the played part should land at the right time.

## Coexistence with the Sampling recorder (decided)

The engine holds a **single** tap slot (`Output::set_tap`), and the Sampling recorder
uses it for "Internal" sources. **Decision (the user's): session recording and internal
Sampling recording are mutually exclusive — no second tap.** This keeps the whole
feature inside the viewer and leaves the engine crate untouched.

- While a session recording is running, arming the Sampling recorder on an **Internal**
  source (master / group / sound) is blocked, with a clear message along the lines of
  "Can't record internally while a session recording is running — stop the session
  recording first." Sampling from a hardware **input** is unaffected (it doesn't use
  the engine tap).
- Symmetrically, starting a session recording while the Sampling recorder is armed on an
  internal source is blocked with the mirror message, so neither can silently steal the
  tap from the other.
- The builder owns the exact wording and where the message surfaces, but the rule is
  fixed: one internal tap consumer at a time, session recording and internal sampling
  never both hold it.

## Left to the builder

- The exact module layout (likely a new `apps/viewer/src/session_record.rs` beside
  `recorder.rs`, plus the viewport window and a `Command::RecordSession` /
  `SessionRecorder` menu entries).
- The WAV finalize-as-you-go strategy (periodic header rewrite vs. streaming writer),
  as long as it meets the crash-safe goal.
- The silence-gap mechanism (per the key risk above).
- Exactly where the reserve setting lives in Preferences and its wording.
- The waveform/meter drawing in the window (reuse existing meter helpers where they
  fit).
- Whether disk writing runs on a background thread (preferred: must not block the UI
  or audio).
- The exact wording and placement of the mutual-exclusion message (the rule itself is
  decided above).

---

## Update (2026-11-17): play/pause and a stereo waveform

Added after the first version shipped, at the user's request.

- **Arm, then play.** Choosing **File ▸ Record Session to File…** no longer starts
  recording immediately. It *arms*: it picks the file, opens the Recorder window with
  live meters/waveform, and holds the master tap — but writes nothing. The window's
  **Play ▸ / Pause ⏸** control (and the **Space** bar while the window is focused)
  starts, pauses and resumes. The header shows **READY / RECORDING SESSION / PAUSED**.
- **Pause cuts the time out.** While paused the timer holds and nothing is written;
  resuming appends to the *same* file, so the file's length always equals the recorded
  (played) time — paused spans are simply absent. Recorded time is tracked from
  per-span `Instant`s (`played_before` + the live span), not wall-clock-since-start.
- **No file until the first Play.** The WAV is created lazily on the first write, at the
  live rate, so arming and then stopping leaves no file behind. (A device/rate change is
  therefore detected only after the file exists.)
- **Mutual exclusion starts at arm.** The tap is held from arm through stop, so the
  internal Sampler is blocked for the whole armed/recording/paused span, not just while
  writing (`SessionRecorder::active()`).
- **Stereo waveform.** The window draws the captured per-channel peaks as two lanes —
  left on top, right on the bottom, each mirrored around its lane centre — like the
  Sampler, replacing the earlier single combined trace (which mis-plotted the two peaks).
