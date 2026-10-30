# Ableton Link and MIDI clock: design

October 2026. Phase 9 of the parity roadmap
(`docs/superpowers/plans/2026-10-22-parity-roadmap.md`). The user accepted GPL-2.0 for
Ableton Link.

## Why

Maschine plays in time with other music software and hardware three ways: Ableton Link
(tempo, beat phase and, optionally, start/stop shared over the network), sending MIDI
clock (Maschine leads a drum machine or another sequencer) and receiving it (Maschine
follows one). DECKARD has none of them, so it can't be used in a setup with anything
else.

## What Maschine does (findings)

From the software manual (`out/manual/sw.txt`: Preferences – MIDI page, "Connecting
external MIDI equipment", "Syncing Maschine using Ableton Link") and NI's scripts
(`SettingsTabMidi.lua`, `NSAHelper.lua`, `MaschineHelper.toggleLink`):

- **Link**: the LINK button in the header joins or leaves the session; while on it shows
  a moving bar, the session's phase over a bar, even when stopped. Pressing Play with
  Link on, playback starts on the next downbeat ("once the moving bar is filled"), in
  the session's tempo and phase. Anyone in the session can change the tempo; the last
  change wins. Preferences ▸ General ▸ Link: Enabled (the button) and Start Stop Sync
  (start and stop together with the peers that also have it on).
- **MIDI Sync** (Preferences ▸ MIDI ▸ Sync): Mode Off / Send Clock / Receive Clock
  (`NSA:getMidiSyncStatus()` 0, 1, 2). Receive isn't offered while Link is on ("Link
  takes priority"); sending is. **Clock Offset**: sending, 0 … 200 shown as 0 … -200 ms
  (`NSA:getCurrentClockOffset`, clock sent that much early); receiving, 0 … 200 shown as
  -100 … +100 ms (`HWPrefs:getMidiClockSyncOffset`). Clock goes to the output ports
  switched on in Devices and comes from the inputs switched on there. While receiving,
  the header's Play is disabled: the external clock starts and stops Maschine.
- **The MK3**: Settings ▸ MIDI page 1 shows MIDI Sync (Mode, Clock Offset) and the
  input and output devices with their status; page 2 has the Link section (Enabled,
  START SYNC). The page's side shows `NI.GUI.insertLinkStatus` (the number of peers) and
  `insertLinkPhaseMeter` (the phase bar). The tempo the MK3 shows is the song's tempo
  parameter, which follows Link or the clock.

## Design

### Link: `crates/link` (`deckard-link`)
Ableton's Link 3.1.5 and its C wrapper `abl_link`, vendored (`vendor/`, GPL-2.0-or-later;
asio-standalone headers, Boost licence), built with the `cc` crate (C++17) — no CMake,
no bindgen (hand-written declarations of the few `abl_link_*` functions used), so it
builds wherever the C++ compiler does. When the target OS isn't the host's (the
cross-platform type-check), the build script skips the C++ and the crate offers a
stand-in that never finds peers (`deckard_link::AVAILABLE` is false).

`Link` wraps an instance: enable, start/stop sync, peers, the session's tempo and play
state on the app thread (capture, change, commit), and `Link::clock()`: an
`Arc<dyn deckard_engine::sync::Clock>` the audio thread asks each block.

### The engine follows a clock: `crates/engine/src/sync.rs`
- `Follow { tempo, beat, quantum, absolute }`: where an outside clock is when the
  block about to be rendered is heard. `Engine::follow` (once per block, from the
  output's callback): takes the tempo (keeping the song position: `Engine::set_tempo`,
  click-free since only the sequencer's clock moves); locks the song's beat to the
  clock's — Link: modulo the quantum (a bar), MIDI clock: absolutely (its song
  position); a start waits silently for the clock's next bar (Link) before the count-in
  or the song.
- `Clock` (a trait the output's callback calls without blocking or allocating): Link's
  audio-thread session state, or `ClockFollower`, which follows incoming MIDI clock
  (24 per beat, Start, Continue, Stop, Song Position Pointer) fed by the MIDI input
  thread, tempo smoothed over the last ticks.
- **Sending clock**: the engine sends Song Position Pointer and Start (from the top) or
  Continue (elsewhere) when the song starts, a clock every 1/24 beat while it plays and
  Stop when it stops, each stamped with when it is due (when that sample is heard,
  minus the clock offset). The MIDI-out thread holds stamped messages until they are
  due, so clock jitter is the thread's wake-up, not the audio buffer.

### The app: `apps/viewer/src/sync.rs`
Owns the Link instance and the clock follower; each frame it exchanges tempo (the
project's tempo follows the session, without an undo step; a tempo changed here is
committed to the session), starts and stops with the session (Start Stop Sync) or the
external clock, and hands the output its clock. Settings (persisted like the others):
`link`, `link_start_stop`, `midi_sync` (off / send / receive), `clock_send_offset`,
`clock_receive_offset`.

UI: the header's LINK button (its phase bar, peers in its tooltip and beside it);
Preferences ▸ MIDI ▸ Sync (Mode, Clock Offset) and Link (Enabled, Start Stop Sync).
MK3: `Workspace` Link parameters, `NSA` MIDI sync and devices, the HW preferences'
receive offset, the Link status and phase widgets.

## Non-goals
- Link Audio (Link 4), MTC, MIDI clock while DECKARD is a plug-in (it isn't one yet).
- Sending clock while stopped.
