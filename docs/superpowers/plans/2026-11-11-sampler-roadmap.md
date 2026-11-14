# Sampler roadmap

What users ask for, what the best samplers do, and the plan.

## Sources

- **NI's "Sampler improvements" forum thread** (community.native-instruments.com/discussion/19900):
  167 posts from 2023-12 to 2024-02, all read.
- **Other samplers' manuals:** Serato Sample, Elektron Octatrack MKII, Akai MPC 3.10,
  E-mu (EOS 4, Emulator III, SP-1200), Ensoniq (EPS-16+, ASR-10). The findings and the
  manual references are in `docs/sampler-research.md`.
- **DECKARD's sampler today**, checked in the code on 2026-11-11.

Each phase gets its own spec and implementation plan. Phases are listed in priority order.

The Clean-room rule from the other plans applies: work from manuals and observed behaviour,
never other products' code or data tables. New fields are DECKARD-only and live in the
DECKARD block, which Maschine ignores.

## What users asked for, ranked

Rough counts: the posts in the NI thread that asked for each thing or +1'd it.

| Rank | Request | Posts | DECKARD today |
|---|---|---|---|
| 1 | Change pitch and tempo independently, live: one-shots across the keys, loops synced to the song | ~30 | Missing. Signalsmith stretch is only in the Audio plug-in; the Sampler's Stretch edit is destructive WSOLA. |
| 2 | Merge the Sampler and the Audio plug-in ("Simpler with Warp on/off") | ~12 | Two separate plug-ins. Answered by a third, the **New Sampler** (below), with Sync on or off. |
| 3 | Key / BPM / pitch detection, a tuner, an automatic root note | ~16 | Missing. |
| 4 | Stems | ~15 | Separation done (`docs/stems.md`), into a new group. Stem lanes inside the Sampler: **Phase 2b**. |
| 5 | Round robin / random / cycle | ~13 | Missing. |
| 6 | More modulation: LFO 2, envelopes, more targets, envelope curves | ~10 | Base mod matrix done; LFOs and envelopes aren't sources yet. |
| 7 | Stereo → mono (sum / L / R) | ~9 | Missing. |
| 8 | MP3 / FLAC import | ~8 | WAV, AIFF and OGG only. |
| 9 | Warp markers / transient quantize | ~7 | Missing. |
| 10 | More than 16 slices, per-slice settings, export chops | ~8 | Slices become zones on one sound; not saved. |
| 11 | Play modes (reverse, ping-pong, random), loop until release | ~7 | Forward loop plus a reverse switch only. |
| 12 | Bars/beats editing, snapping, zoom to the selection, hear the END point | ~7 | The view is in seconds. |
| 13 | Sample pool, clean-up, rename, Save Sample As | ~6 | Takes are kept in memory only. |
| 14 | Bounce / freeze to a pad, merge or append pads | ~6 | Missing per pad. |
| 15 | Granular mode, several start points | ~6 | Missing. |
| 16 | Quick sample, skipback, looped takes, input source per group | ~6 | Missing. |
| 17 | Zones: auto-map, mute, per-zone envelopes | ~5 | Basic map only. |
| 18 | More filters, a Gain knob, polarity flip | ~6 | LP2, BP2, HP2 and EQ only. |

## What the best samplers do that DECKARD should learn from

| Idea | Seen in | Phase |
|---|---|---|
| A beatgrid with tempo markers; ×2 / ½ / tap / adjust | Serato, MPC | 1 |
| Linked tempo / bars / loop-bars fields; BPM from the filename; Calculate BPM from Selection | Octatrack | 1 |
| Match / To Project (fit the sample to the song, or the song to the sample) | MPC | 1 |
| Stretching switched off automatically for short files and one-shots | Serato | 1 |
| Flatten (render the warp and FX into a new sample) | MPC | 1 |
| A sample-only tempo nudge | Octatrack | 1 |
| Warp markers. **No MPC has them.** | Ableton (asked for in the thread) | 1e |
| Catch cues by tapping pads while the sample plays | Serato, MPC | 3 |
| Autoset: Find Samples, Slicer, Random, Key Shift Pad; Favorite keeps a cue | Serato | 3 |
| Non-destructive slice references; Slice Motion (next or random slice on each hit) | MPC | 3 |
| Overlapping slices with their own loop points; linear / random slice locks; sidecar slice files | Octatrack | 3 |
| Convert slices to a track with the notes written in | MPC | 3 |
| Beat Munger: swing ±, an 8th-note mute grid, Grunge / Comb splices | E-mu | 3 |
| 8 layers; layer play Cycle / Velocity / Random (seed) / Crossfade | MPC, Ensoniq | 4 |
| Crossfade by any controller; Crossfade Random for round robin | E-mu, Ensoniq | 4 |
| **Patch Select:** 4 layer combinations per sound, switched live | Ensoniq | 4 |
| Key-up layers, Legato Layer, Layer Delay | Ensoniq | 4 |
| Voice-steal bins (choke), Latch, 8 solo / legato modes | E-mu | 4 |
| **Wave modulation:** loop position / start / loop start / loop end / Transwave | Ensoniq | 5 |
| Loop and release, bidirectional, repeat count, Tail Length, Play Offset | Ensoniq, MPC | 5 |
| Loop tools: auto-correlate, Ensemble / Bowtie crossfades, Loop Compression, Synthesized Loop | E-mu, Ensoniq | 5 |
| Edits auditioned before they're kept (KEEP OLD / NEW) | Ensoniq | 5 |
| Velocity-morphed dual-level envelopes; 6-stage envelopes; Repeat envelopes | Ensoniq, E-mu | 6 |
| Mod processors (lag, quantize, switch, flip-flop), pattern LFOs, a step LFO designer | E-mu, Octatrack | 6 |
| Clock-division retrigger; Sample Retrigger as a modulation target | E-mu | 6 |
| Z-plane-style morphing filters (two frames, a Morph knob) | E-mu | 7 |
| The SP-1200 recipe; a Crunchy / Smooth / Creamy interpolation choice | E-mu | 7 |
| Recorders kept separate from pads; recording on sequencer steps; quantized start | Octatrack | 8 |
| Pad Tap / Pad Hold recording | MPC | 8 |
| Resample through FX or the main output | Ensoniq, Octatrack, MPC | 8 |
| Skipback (retrospective **audio**; the MPC's Retro Rec is MIDI only) | SP-404 (asked for in the thread) | 8 |
| A looper that sets the song's tempo; overdub / replace; ×N follower loops | Octatrack | 8 |
| **Scenes and a crossfader** that morph any parameters between A and B | Octatrack | 9 |

---

## Architecture: a new sampler beside the old two (decided 2026-11-12)

DECKARD gets a third instrument instead of rebuilding the Sampler. Working name **New
Sampler**; the real name comes later.

| Instrument | What it is | Changes |
|---|---|---|
| **Audio** | Loops in time, as today (`docs/audio-plugin.md`) | None |
| **Sampler** | Maschine's Sampler, saved in Maschine's format | Bug fixes only, so existing projects sound the same and still open in Maschine |
| **New Sampler** | DECKARD's own, and the home of every phase below | New |

- **Built from the Sampler's parts:** it reuses the engine's interpolation, envelopes,
  filters and edit code (`crates/engine/src/sampler.rs`, `edit.rs`), as a new module with
  its own settings in the DECKARD block. The Sampler's code isn't changed to carry the new
  features.
- **Convert:** Sampler → New Sampler and Audio → New Sampler in one step, keeping zones,
  start/end, loop, tune and the Audio plug-in's tempo. **New Sampler → Sampler** keeps what
  Maschine can play (zones, start/end, loop, tune, envelopes, filter) and lists what it
  drops (Sync, layers, stem lanes, modulation).
- **Stored as Maschine's Sampler** with a `deckard:new_sampler` field (built 2026-11-12):
  in Maschine it opens as a plain Sampler that still plays (Repitch), so nothing needs
  converting on save. Its zones and files are the Sampler's; what's new lives in
  `deckard:` fields and the DECKARD block (`docs/deckard-block.md`, v13).
- **Which one new sounds get** (a dropped sample, STEMS ▸ Into lanes, a recording, the
  browser) is a preference; the New Sampler once Phase 1 ships.
- **MK3:** it needs its own pages; `sampling.lua` is built around Maschine's Sampler. The
  Sampler's pages stay for the Sampler.
- **Phase 7's vintage modes** (the unused MPC60 / SP-1200 tables) can go in both.

---

## The UI: on par with Serato Sample, in the software and on the MK3 (decided 2026-11-12)

Every phase ships with its UI at this level; a feature isn't done until it is. The bar is
Serato Sample's clarity and speed, in DECKARD's own look (clean room: match the quality,
not the trade dress).

**Software**
- **Two waveforms:** an overview of the whole file (cues, playhead; click to jump) above a
  zoomed main waveform (drag to scrub; +/−, the wheel and keys to zoom; **Freeze** stops it
  following the playhead while editing; **Grid** mode shows the beat grid and its tools).
- **Waveforms coloured by sound:** colour from frequency content (lows warm, highs cool),
  so kicks, bass and hats read at a glance; it recolours live as stem lanes mute.
- **Cues as colours:** each cue or slice has a colour, its pad shows it, and its region is
  shaded in it on both waveforms; empty pads are dark grey. The parameter panel takes the
  selected cue's colour, white when several are selected (edits apply to all).
- **Direct manipulation:** press an empty pad while it plays to catch a cue; drag a cue's
  handles; drag a cue onto another pad to move or swap it, Alt-drag to copy; drop a file
  anywhere to load it.
- **Readouts up top, big:** BPM (with ×2, ½, TAP), key, root note and its lock, target
  note, SYNC, the playback mode.
- **Fast:** waveform peaks precomputed at several zoom levels when the sample loads, so
  zoom and scroll stay at the display's frame rate; analysis shows progress inline and
  never blocks the UI.
- **Checked side by side:** each phase's UI is reviewed with screenshots next to Serato
  Sample's for the same task (load, sync, chop, edit a cue).

**MK3** (two colour screens, RGB pads, the 4-D encoder)
- **Left screen:** the overview strip above the zoomed waveform, cues in their colours, the
  playhead moving smoothly. **Right screen:** the selected cue's parameters on Knobs 1–8,
  headed in its colour.
- **Pads light in their cue colours**, dim when empty, flash on play; pressing an empty pad
  while the sample plays catches a cue there.
- **The 4-D encoder** scrubs (turn), zooms (SHIFT + turn) and moves between cues (push
  left/right); in Grid mode it slips the grid.
- **Stem lanes** stacked on a screen, with their mutes on the buttons (Phase 2b).
- **Same bar for speed:** waveform images are drawn from the same precomputed peaks;
  check the frame rate on the hardware (`crates/nwl`).

**Phase 1d builds this base** (both waveforms, colouring, readouts, cue colours, the MK3
pages); later phases add to it.

---

## Phase 1: Beat matching (top priority)

**Goal:** drop any loop or break onto a pad and it plays at the song's tempo, on its grid,
in key, in the New Sampler.

### 1a. Tempo analysis (`crates/analysis`, new, runs off the audio thread)

**Built 2026-11-12** (`crates/analysis`: `tempo`, `root`, `root_with_name`;
`deckard_library::names::note_in_name`; plan `2026-11-12-sample-analysis.md`). Tested on
synthetic drum loops and tones, and on a corpus of six grooves played by DECKARD's own
drum synths at nine tempos (72–174), as loops and with silence after them (the app's
`the_tempo_of_grooves_on_our_own_drums`); loops recorded elsewhere are still to come.

- **Onset novelty:** spectral flux. Reuse and improve `edit.rs::detect_positions`.
- **Tempo estimate:** a tempogram or autocorrelation, searched over 60–200 BPM, then folded
  into 85–170 (as the Octatrack does).
- **Corrections, in priority order:**
  1. A BPM in the filename, which also fixes ×2 / ½ errors (Octatrack).
  2. The loop prior: the play range should hold a whole number of beats or bars.
  3. Keep the ×2 / ½ candidates.
- **Downbeat offset:** a constant BPM, plus later tempo markers (1e).
- **Root note (moved up from Phase 2):** pitch detection (pYIN on the sample's loudest
  stable stretch; on a stemmed zone, Bass plus Leads) gives a root note and a cents offset,
  with a confidence. Low confidence (drums, noise) leaves the root unset rather than
  guessing. A note name in the filename (C3, F#2, "Am") wins over detection.
- **Testing:** a corpus of loops with known tempos, rendered by DECKARD's own drum engines,
  with error bounds asserted in tests; for the root, DECKARD's synths playing known notes
  and detuned by known cents.

### 1b. Sample metadata (in the DECKARD block per zone, plus a cache keyed by file hash)

**Built 2026-11-12** (plan `2026-11-12-new-sampler-metadata.md`): the New Sampler in the
"+" menu and the controller's browser; its zones analysed in the background (cached in the
settings folder's `analysis`, keyed by the audio's SHA-256), the Auto rule choosing Sync or
Repitch, and a readout on the Sampling page. Not yet: the linked BPM/bars/loop-bars fields,
ACID and `smpl` chunks, `key`. The engine still plays every zone as Repitch (1c).

- **Fields:** `bpm`, `bpm_source`, `bars`, `loop_bars`, `grid_offset`, `tempo_markers`,
  `key`, `root`, `stretch` (Auto / Off / Normal / Beat).
- **BPM, bars and loop bars are linked:** edit one and the others follow, with a priority
  flag (Octatrack).
- **Read before analysing:** ACID and `smpl` chunks.

### 1c. Sampler playback mode: **Repitch | Sync | Keys** (per zone)

**Built 2026-11-12, Rendered engine** (plan `2026-11-12-sync-playback.md`): REPITCH / SYNC /
KEYS on the Sampling page (one undo step); a Sync or Keys zone plays a Signalsmith render
of its play range at the song's tempo with its Tune in it (`crates/engine/src/render.rs`,
made on the app's render thread, kept up to ~400 MB, least recently used dropped). Until
the render comes, and while the tempo moves away from it, the zone reads faster or slower
so it stays in time (repitched); a new render takes over sounding notes where the music
is. Keys renders ±12 semitones around the root, nearest first, and transposes by speed
until a note's render is made. A Sync note played live while the song runs starts on its
zone's START grid (off, 1/16 by default, 1/8, 1/4, a bar): up to half a division early
it waits for the line; later it starts at once, that far into the sample (a 3 ms fade
in); the song's own notes start where they are. An export makes its renders before it
plays. Not yet: the Live engine (Live plays as Rendered), and modulated Tune and the
pitch envelope in the stretcher (they still repitch).

This covers what the Audio plug-in does, and is why the New Sampler can replace it for new work.
- **Repitch:** today's behaviour, and the default for one-shots.
- **Sync:** follows the song's tempo, and is **on by default for loops** dropped onto a pad
  (decided 2026-11-12; Auto mode below decides what counts as a loop). Tune stays independent of tempo. Start is quantized
  to the grid.
- **Keys:** notes change the pitch and keep the length.
- **Auto mode:** stretching stays off for files under 3 s and short "808" files (Serato's
  rule).

**Stretch engine, a per-sound setting: Rendered (default) | Live** (decided 2026-11-12).
- **Rendered:** stretch offline in the background (Signalsmith, already vendored) whenever
  the tempo or tune changes, then swap the result in atomically. Playback then costs the
  same as today. Keys mode renders each note on demand into an LRU cache, with a capped
  number of live stretch voices covering the first hit.
- **Live:** a Signalsmith stretcher per voice, for tempo ramps and modulated stretch that
  can't be pre-rendered. Capped voice count (oldest live voice falls back to Rendered);
  the MPC's live per-voice warp is a known CPU and quality problem, so it's opt-in.
- A global preference sets the default for new sounds. Measure both on a Pi-5-class CPU.

#### Root and tuning, independent of tempo

- **Root:** filled in from 1a's detection, shown as e.g. "Root A2 −12 ct". Click or turn to
  set it by hand; once set by hand it's **locked** and re-analysis won't touch it.
- **Target note:** pick the note the sample should sound at (or **Match key**: the
  project's scale root, nearest up or down). DECKARD sets Tune = target − root, in
  semitones, plus the cents to cancel the detected offset (**Fine-tune to 0 ct**, on by
  default when the root was detected).
- **The length never changes in Sync or Keys:** Tune feeds the stretcher's pitch, not the
  playback rate, so a loop locked to the song's tempo keeps exactly its bars at any note.
  Re-tuning re-renders in the background (Rendered) or applies at once (Live); the old
  render keeps playing until the new one swaps in.
- **Repitch** keeps today's behaviour: tune changes the speed too (the classic sampler
  sound). The UI says so next to Tune when the zone is in Repitch.
- **Keys mode** plays the root on its own key, so the pads or keyboard are in tune with the
  project without any setup.
- **Stem lanes:** one root and one tune for the zone; every lane shifts together.

### 1d. Analysis UI

**First cut built 2026-11-12:** over a New Sampler zone's EDIT and SLICE pages, TEMPO (BPM
and BEATS, linked; ×2, ½, TAP, SET PROJECT BPM, also a right-click on BPM), PITCH (ROOT, set by hand and then locked;
PLAYS AT, the note its Tune makes, its cents tuned out) and REPITCH / SYNC / KEYS; the
waveform's ruler in bars and beats with beat lines. MK3: EDIT's TEMPO / PITCH knob page
(MODE, BPM, ROOT, PLAYS AT), and under SHIFT X2, 1/2, TO PROJECT on the display buttons.
MATCH KEY (the Tune putting the root on the nearest note of the song's scale root; on
the MK3 under SHIFT) and START (the Sync grid; the MK3's SYNC knob page) came after.
Then: the waveform's markers (start, end, loop, slices, the selection) snap to the
sample's beats, SHIFT free; + and − nudge the BPM a hundredth; SLICE ▸ Grid slices on
the sample's own beats (through its warp markers).
Not yet: the
sample-only nudge, Flatten, the key, and Serato's waveform colouring.

- **Header on the EDIT and SLICE pages:**
  - the BPM, with ×2, ½, TAP and ± nudge
  - the key, the root note (with its lock) and the target note
  - **Match** (re-tune the sample to N beats) and **To Project** (set the song's tempo from
    the sample)
  - a SYNC switch
- **Bars.beats timeline:** snap to beats; SHIFT moves freely.
- **SLICE ▸ Grid** uses the sample's own BPM.
- **Sample-only tempo nudge:** a held control that moves the sample, not the song
  (Octatrack).
- **Flatten:** render the warp and the sound's FX into a new sample, to save CPU (MPC).
- **MK3:** SYNC on a button, BPM on a knob, ×2 / ½ under SHIFT. This builds the missing
  STRETCH page on the controller (`sampling.lua:21`).

### 1e. Tempo markers and warp markers

**Warp markers built 2026-11-12:** a zone's markers pin frames to beats (`deckard:zone`'s
`warp` lines; `BeatMap` in `crates/model/src/new_sampler.rs`, straight between markers,
at the end segments' speed beyond). Sync and Keys render through the map, each stretch
between markers at its own speed (`render::stretched_by`). On the Sampling page:
MARKERS (on every hit), QUANTIZE (to 1/4–1/32, with STRENGTH), CLEAR; drag a marker's
handle to move its beat, double-click it to remove it, double-click the ruler for a new
one; the bars-and-beats ruler follows the map. Not yet: Serato's tempo markers and
ADJUST (warp markers cover a piecewise tempo; a tempo-marker UI on top is to come), the
waveform drawn warped (it stays in sample time, the grid moves instead). On the MK3:
EDIT's functions go on with WARP MARKERS, QUANTIZE AUDIO and CLEAR WARP, and a WARP knob
page has GRID and STRENGTH; markers aren't moved one at a time there.

- **Tempo markers:** Serato-style, up to 128, giving a piecewise tempo map. The ADJUST
  control stretches the segment under the playhead.
- **Warp markers:** start at the detected transients. Sync stretches piecewise between
  them. **Quantize audio** snaps them to the grid with a strength setting.
- Warp markers are a gap even on the MPC.

### 1f. Converting into the New Sampler

**Built 2026-11-12:** the instrument slot's right-click menu has **Convert to New Sampler**
(a Sampler keeps its zones; an Audio plug-in's takes become zones, one key each from C3
or all keys for one take, in Sync at the take's tempo, the plug-in's Tune on each) and
**Convert to Sampler** (Sync, Keys and the analysis go; the status line says so). One
undo step each. A pattern that played a take (Loop mode) gets its notes: one each time
the take came round, cut at the pattern's end, the envelope made ADSR so they end with
it; a Gate-mode plug-in's notes play its lone take in Keys. On the MK3, EDIT's last
function converts a Sampler (TO NEW SAMPLER) and back (TO SAMPLER); an Audio plug-in
converts from the software only. Not yet: a take
starting before its pattern loses that first cut-off part.

Audio → New Sampler and Sampler → New Sampler (see Architecture above). The Audio plug-in
and the Sampler both stay.

---

## Phase 2: Key and pitch

- **Key detection:** chroma features plus key profiles, with a confidence.
- **Root note and Match key:** moved to Phase 1 (1a, 1c "Root and tuning").
- **Tuner:** a live display of the detected pitch and cents while the sample plays, and per
  slice (a chop can have a different root from its parent).
- **Auto-map zones:** by detected pitch, or by note names in the filenames (C3, F#2, 060).
- **Key Shift Pad** (Serato): spread one chop chromatically across the 16 pads.

---

## Phase 2b: Stem lanes in the Sampler

**Built 2026-11-12** (first cut): a zone's meta keeps up to four stems (file, mute, solo,
gain); a voice reads them with the sample (all on: the sample itself; else their sum,
crossfaded and ramped over 5 ms on sounding notes); Sync and Keys stretch each lane (the
root's with the sample, a Keys note's once a lane is off unity). STEMS on a New Sampler
zone puts the stems on its lanes (Shift, or its right-click menu: a new group; "Load
stem files…": your own, by name, short ones padded, long ones and other rates left
out). The Sampling page stacks the lanes (name, M, S, level; MIX folds them; NO STEMS
takes them off). MK3: EDIT's STEMS knob page has the levels, display buttons 5–8 the
mutes (SHIFT: solo), the right display the lanes stacked. Slices keep the lanes, each
with mutes of its own. An edit of the sample (truncate, reverse, normalize, stretch…)
edits its lanes alike, in one undo step, the tempo and grid kept (2026-11-13; Paste
alone leaves the zone without lanes: what's pasted has none). A lane's × deletes it:
the others are then all that plays (`stems_only`), at unity too. Not yet: mute and
gain as automation and mod-matrix targets; analysis from the right lane; dropping
files onto lanes (a file picker for now); deleting a lane from the MK3; the scripts
route's native display shows one waveform.

Also 2026-11-13, from a session with a real recording: MP3 loads everywhere; a song is
analysed in a second or two (tempo from two minutes of its middle); SLICE ▸ APPLY's
EACH PLAYS TO ▸ Sample end makes slices start points (a New Sampler's default), and
chops of one sample share one stretched render; the Sampling page has PLAY / STOP
(Shift+Space) and keeps its zoom across zones.

Mute the drums, vocals, bass or leads of a sample right in the New Sampler, without leaving the
sound. Today STEMS makes a new group of four Audio plug-ins (`docs/stems.md`); this keeps the
four stems together in one zone, so they share every edit.

- **At most 4 lanes**, fixed order and colours: **Drums, Vocals, Bass, Leads**. Leads is
  Demucs's "other" stem: everything that isn't drums, bass or vocals (keys, guitars, synths,
  pads).
- **Where lanes come from:**
  - **STEMS ▸ Into lanes** (the new default): runs the existing Demucs separation and puts
    the four results on the focused zone. **STEMS ▸ To new group** keeps Maschine's
    behaviour.
  - **Your own stems:** drop up to four files of the same length onto the lanes (shorter
    ones are padded with silence; longer ones are refused).
- **Model:** a zone gains `stems: [Option<StemLane>; 4]`, each `{ file, kind, mute, solo,
  gain }`, in the DECKARD block. Maschine sees only the original sample, so a project
  opened there still plays (unmuted).
- **Playback:**
  - One voice reads all lanes at the same position. Interpolation, loop, envelopes and the
    filter run once on the sum, so the cost is four reads, not four voices.
  - **All lanes on plays the original file**, bit for bit. Muting a lane switches to the
    sum of the unmuted lanes, with a short crossfade (Demucs's stems don't null perfectly).
  - Mute, solo and gain change on sounding notes, with a ~5 ms ramp, so there are no
    clicks.
- **Shared timeline:** start/end, loop, slices, tempo and warp markers apply to every lane.
  Sync and Keys stretch each lane separately (in the background, cached per lane), so
  muting never waits on a render.
- **Mutes per slice:** each zone keeps its own mute set, so slices of one stemmed sample can
  play different lanes: pad 1 the vocal of bar 3, pad 2 its drums. Slices start with the
  parent's mutes.
- **Automation and modulation:** each lane's mute and gain are parameters, so they can be
  recorded in patterns, locked per step (FEATURES #8) and routed in the mod matrix.
- **Analysis uses the right lane:** BPM and Find Samples' transients on Drums, key and pitch
  on Bass plus Leads, vocal presence on Vocals (Phases 1a, 2, 3).
- **Software UI:** the Sampler's waveform splits into up to four stacked lanes on one time
  axis, sharing zoom, the playhead, slice and warp markers. Each lane has a header with its
  name, colour, **M** (mute) and **S** (solo); a muted lane is drawn dimmed. A **Mix**
  toggle folds the lanes back into one waveform. A zone without stems looks as it does
  today.
- **MK3:** the sample page shows the four lanes stacked; a STEMS view puts the lane mutes
  on Buttons 5–8 and the lane gains on Knobs 5–8 (to fit around `sampling.lua`'s existing
  tabs when built).
- **Builds on:** the existing separation (`crates/stems`, `apps/viewer/src/stems.rs`), and
  Phase 1c's per-zone render cache. Logged in `docs/FEATURES.md` (#16).

---

## Phase 3: Chopping

- **Catch cues:** in SLICE, press empty pads while the sample plays to drop slices at the
  playhead (Serato, MPC). Snap with Quantize to 1/8 beat.
- **Autoset:**
  - **Find Samples:** novelty segmentation, plus beat alignment, loudness, and stems-based
    vocal presence. Suggests up to 32 chops.
  - **Slicer:** 32 equal slices from the playhead.
  - **Random.**
  - **Key Shift Pad.**
  - **Favorite** keeps a chop when you re-roll.
- **Slices as references, not files:**
  - A zone's slice field is All / Range / Slice N, pointing into one shared sample.
  - **Slice Motion** (MPC): next or random slice on each hit, over N slices.
  - Slices may overlap and each has its own loop point (Octatrack). Saved in the DECKARD
    block, plus a sidecar file next to the sample.
- **More than 16 slices:** any count (2–64), spread across key banks or several groups;
  "Apply to group" gives one Sampler per pad.
- **Per-slice settings:** tune, reverse, gain, an attack/release fade, Sync/Keys mode.
- **Convert:** make a new group or pattern with **the notes written in** (slices in order,
  or **linear/random locks**, as on the Octatrack).
- **Export:** each slice as a trimmed WAV; drag slices out as audio or MIDI.
- **Beat Munger edit** (E-mu): swing ± (including removing swing), an 8th-note mute grid
  (which can change the meter), and Grunge / Comb splice controls. Auditioned live, then
  rendered.
- **Tail Length** (MPC): a looped tail for chops that end abruptly. Reverse-tail is the
  MPC trick.

---

## Phase 4: Layers and voice architecture

Ensoniq, E-mu and MPC all go well beyond Maschine's zones here.

- **Layers:**
  - up to 8 per sound
  - one zone per key per layer
  - a velocity window per layer, a key range per zone
- **Layer play:** All, **Cycle** (round robin), **Random with a seed**, Velocity, and
  **Crossfade by any mod source**.
  - The crossfade uses the Ensoniq A–B / C–D trapezoid with a curve, so the mod wheel,
    pressure, key or an LFO can fade between layers (vector-style).
  - E-mu's **Crossfade Random** (one random value per key) gives round robin across
    layers.
  - **Per-layer random:** pitch, level, pan and start (MPC).
- **Patch Select:** four layer combinations per sound.
  - Switched live from the MK3 (two buttons, momentary or latched) and from MIDI CC 70.
  - Recordable as automation.
- **Key-up layers:** a release "thunk". **Legato Layer:** overlapping notes play another
  layer. **Layer Delay:** 0–5 s, with a ± velocity amount.
- **Voice handling:**
  - Voice-steal bins: Poly-N / Mono-A…I, for chokes inside one sound.
  - **Latch** (press to start, press again to stop).
  - **Solo modes:** last / low / high note, Synth-style legato, Fingered Glide.
- **Simultaneous Play** (MPC): a pad also triggers up to 4 others.
- **Auto-sampler:** record N variations per note and velocity, mapped as round-robin
  layers.

---

## Phase 5: Loops and wave modulation (Transwaves)

- **Play modes:**
  - forward, reverse, forward loop, **bidirectional loop**
  - **loop and release** (play on past the loop on release)
  - **Repeats** (loop N times)
  - random start
- **Wave modulation** (Ensoniq): one type per zone, plus SRC, AMT and RANGE.
  - Types: LOOP POS, START, START+LP, LOOPSTRT, LOOP END, LPSTRT-X (pitch-corrected), and
    **TRANSWAV**.
  - TRANSWAV jumps the loop in whole frames. DECKARD adds an **optional crossfade between
    frames**, which fixes the hardware's "flutter".
  - Sources are all mod matrix sources: LFOs, envelopes, velocity, mod wheel, pressure,
    macros.
  - It runs in the read loop of `Stream` (`sampler.rs`), with no allocation.
- **Transwave tools:**
  - **Make Transwave** from a sweep: find single cycles at zero crossings, resample to equal
    frame lengths, concatenate.
  - Import wavetables (Serum-style WAV with a frame size).
  - Build from pulse widths or other basic waves.
  - DECKARD's oscillators (MPC 3.9 style) could reuse the same frames.
- **Loop tools:**
  - auto-correlate around the loop points (refine on each press), with **Fix Size**
  - Auto Loop at a fraction (whole, ½, ⅓, ¼, ⅛)
  - crossfade loop (equal power / linear), **Ensemble** and **Bowtie** crossfades
  - **Loop Compression** (even out the level inside the loop)
  - **Synthesized Loop** (loop anything)
  - loop-end precision of 1/16 sample
- **Audition before keeping** (Ensoniq KEEP OLD / NEW) for every destructive edit, in the
  software and on the MK3.
- **Play Offset** (MPC): slide the start, end and loop window by a set amount; it can be
  modulated (step through a loop's 16ths).

---

## Phase 6: Modulation and envelopes

- **Sources for the matrix** (FEATURES #1): the Sampler's LFOs and envelopes, LFO 2,
  Env 2/3, kRandom (per-note random), noise, and **clock divisions** (retrigger envelopes,
  reset LFOs, retrigger samples).
- **Processors:** lag, sum, switch, quantizer (to steps or a scale), flip-flop, abs, DC
  offset. Cords can modulate other cords' amounts.
- **Envelopes:**
  - attack, decay and release curves
  - an optional 6-stage rate/level mode
  - **HARD / SOFT velocity level sets** (Ensoniq: velocity morphs the envelope's shape)
  - **Repeat mode** (loop the middle stages)
- **LFOs:**
  - **pattern shapes:** octaves, fifths, arpeggios
  - a **16-step LFO designer**, stepped or smoothed (Octatrack)
  - tempo sync
- **Targets:** loop position and length, slice, play offset, **Sample Retrigger** (a zero
  crossing restarts the sample: stutters).
- **Trigless steps:** change parameters or restart LFOs without retriggering the sample
  (Octatrack). This ties in with FEATURES #8 (parameter locks).

---

## Phase 7: Filters and sound character

- **Morphing filter** (Z-plane-style): two filter frames, A and B, with Morph, Frequency and
  a second parameter (Q, Gain, Body or Peak).
  - Presets: vowel (Ah-Ay-Ee, Oo-Ah), Dual EQ Morph, Peak/Shelf Morph, phaser.
  - 4-pole LP/HP, a ladder filter, and key-tracking.
  - Design the filters ourselves; E-mu's coefficient tables are proprietary.
- **Engine modes:**
  - **SP-1200:** 12-bit, a fixed 26.04 kHz, drop-sample pitch with no interpolation or
    reconstruction filter (the moving aliasing), an SSM2044-style LP, tune *or* decay,
    multipitch.
  - **MPC60:** finish the unused tables in `sampler_tables.rs`.
  - Add an **Ensoniq** fixed-rate mode as well.
- **Interpolation choice:** Crunchy / Smooth / Creamy, as a user-facing quality setting.
- **Basics:**
  - **Mono** (sum / L / R / side; destructive, and a playback switch)
  - Gain before FX
  - polarity flip
  - MP3, FLAC and M4A import via `symphonia`, decoded to WAV in the project
- **Offline tools:**
  - Transform Multiply (convolve two samples)
  - Doppler paths
  - Merge / Mix / Splice two samples with pitch matching
  - "Pre-verb": reverse, add reverb, resample, reverse back

---

## Phase 8: Recording and resampling

- **Takes:**
  - saved in the project
  - Remove Unused, Map to Zones, Save Sample As, rename and duplicate on the MK3
  - clean-up of edit files
- **Sample pool** for the whole project.
- **Pad Tap / Pad Hold recording** (MPC): one continuous recording, with each pad pressed
  getting its own range.
- **Recording on sequencer steps** (Octatrack):
  - each step has its own source: input, master, group or a single sound
  - one-shot or repeating
  - **QREC:** quantized start at the next beat or the pattern's end
  - **same-step rule:** play what is being recorded right now
- **Resample:** through the sound's FX or the master. Output to a pad, a zone or a new
  sound. Bounce or freeze a pad.
- **Skipback:**
  - an always-on ring buffer of the master and the chosen input (60 s); "Grab" drops it
    onto the focused pad
  - beats the MPC (MIDI-only retro record)
- **Looper:**
  - the first loop sets the song's tempo (with a ×2 / ½ guard against doubling); later
    loops follow at ×N length, locked in phase
  - overdub, replace, fade, ping-pong, multiply
  - export to a pad
  - foot-switch MIDI map
- **Quick sample:** SHIFT + SAMPLING + pad.
- **Input:** remembered per group; inputs can be renamed.

---

## Phase 9: Scenes and a crossfader (beyond the sampler)

The Octatrack's best-loved feature.
- **Scenes:** each group or song section stores scenes that lock any automatable
  parameters.
- **A/B and Morph:** pick scene A and scene B; a Morph control (a knob, the touch strip or
  MIDI) interpolates every lock between them.
  - While the control moves, scene locks override step locks.
  - The scene on the inactive side can be swapped live.
- **Volume locks:** equal-power, so there's no dip in the middle.
- **Scene mute**, curve options, and scene A/B choices per song section.
- **Builds on:** macros (Phase 7, done) and the mod matrix. Logged in `docs/FEATURES.md` (#17).

---

## Order and dependencies

1. **Phase 1, beat matching:** 1a → 1b → 1c (Sync) → 1d. This is the MVP. Mono and MP3
   (Phase 7) run in parallel; they're small and much asked for.
2. **1c Keys mode, then Phase 2:** both reuse the stretch cache and the analysis crate.
3. **Phase 2b, stem lanes:** can start any time for Repitch (the separation exists); its
   Sync and Keys need 1c's render cache. Do it before Phase 3, which uses the lanes.
4. **Phase 3:** needs Phase 1's grid and Phase 2's key.
5. **Phase 4 → Phase 5:** layers first, because wave modulation and round robin build on
   the zone model.
6. **Phase 6:** extends the mod matrix; Phase 5's targets need it.
7. **1e warp markers and 1f converting**, once Sync has been used for real.
8. **Phases 7, 8 and 9** in that order. Skipback and the sample pool can go first if
   wanted.

## Decisions (answered 2026-11-12)

1. **Stretch rendering:** optional per sound; **Rendered in the background by default**,
   Live per voice as an opt-in (1c).
2. **Sync on by default** for dropped loops: **yes**. Repitch stays for one-shots.
3. **DECKARD-only features** (round robin, layers beyond Maschine's zones, Patch Select,
   wave modulation, stem lanes, scenes), lost if a project is opened in Maschine: **yes,
   acceptable**, as with expression.
4. **MPC firmware teardown:** **skipped for now.** The manuals answered the design
   questions; revisit only to observe behaviour they leave unclear, under the clean-room
   rule.
