# Phase 9 — Ableton Link and MIDI clock

> Roadmap: `docs/superpowers/plans/2026-10-22-parity-roadmap.md`. Design:
> `docs/superpowers/specs/2026-10-23-phase9-link-midi-clock-design.md`. Inline,
> test-first; the software and the MK3.

**Goal:** Link (tempo, phase, start/stop) and MIDI clock (send and receive, with Start,
Stop, Continue and Song Position) as Maschine has them, in the software's header and
Preferences and on the MK3's Settings ▸ MIDI page.

**Sources:** the software manual (Preferences – MIDI, "Connecting external MIDI
equipment", "Syncing Maschine using Ableton Link"); `SettingsTabMidi.lua`,
`NSAHelper.lua`, `MaschineHelper.toggleLink`, the HW style sheet's `HWLinkStatus` /
`HWLinkPhaseMeter`; Ableton's `abl_link.h`.

## Tasks
1. **`crates/link`**: vendor Link 3.1.5 (`include/`, `extensions/abl_link`, asio's
   headers, licences); `build.rs` with `cc`, skipped when cross-compiling to another
   OS; FFI and `Link` (enable, start/stop sync, peers, tempo, playing, phase). Tests:
   an instance keeps the tempo it's given and reports no peers alone; two instances in
   one process find each other and share a tempo change (skipped when the network
   doesn't let them meet).
2. **Engine follows a clock** (`engine/src/sync.rs`): `Follow`, `Engine::set_tempo`,
   `Engine::follow` (tempo, phase lock modulo the quantum or absolute, a synced start
   waiting for the next bar), `Clock`, `ClockFollower` (MIDI clock in). Tests.
3. **Clock out of the engine**: SPP + Start/Continue, 0xF8 every 1/24 beat, Stop; stamped
   with when due less the offset. Tests.
4. **Output**: `Command::Clock`, the callback asks the clock for the block's heard time
   and calls `follow`; MIDI-out holds stamped messages until due. Tests.
5. **Settings**: `link`, `link_start_stop`, `midi_sync`, `clock_send_offset`,
   `clock_receive_offset`, parsed and written (no file written in tests).
6. **App** (`viewer/src/sync.rs`): Link and the follower, tempo exchange, start/stop
   both ways, receive disables Play, MIDI input feeds the follower, clock out ports;
   the header's LINK button; Preferences ▸ MIDI ▸ Sync and Link. Tests (harness).
7. **MK3**: Workspace `LinkEnabled` / `LinkStartStopSyncEnabled`, `NSA` sync status,
   clock offsets and MIDI devices, `getMidiClockSyncOffset`, the Link status and phase
   widgets; the app sees changes made there. Tests: the MIDI page sweep has no errors
   or stubs of these; turning Link on from the MK3 turns it on in the app.
8. Docs (PARITY row 8, licences, this plan's results); the full check.

## Results
- Tasks 1–8 done. `crates/link` (Link 3.1.5 + `abl_link`, vendored; `Link`,
  `AudioClock`, `Link::local`); `engine/src/sync.rs` (`Follow`, `Clock`,
  `Engine::follow` / `set_tempo` / `sync_start_within`, clock out, `ClockFollower`,
  `SyncMsg`); `output.rs` (`Command::Clock` / `ClockOut`, the clock asked each block
  for when the block is heard); `midi_io.rs` (stamped messages held until due, clock in
  to the follower); `viewer/src/transport_sync.rs` (Link, tempo, transport, the MK3's
  view, Preferences ▸ MIDI ▸ Sync and Link); the header's LINK; the host's
  `api/sync.rs` (NSA's MIDI sync and devices, the receive offset, Workspace's Link
  parameters, `HWLinkStatus` / `HWLinkPhaseMeter`).
- Found on the way: the display renderer ignored `setTopIndex` on stacks (the Settings
  page showed its last tab's side whatever the tab); fixed in `lua-host/src/screen.rs`.

## Rulings
- **Link's code:** Ableton's Link 3.1.5 (the last release; not 4.0's beta) and its C
  wrapper, vendored and built with `cc`, declared by hand — not `rusty_link`, whose
  bindgen (libclang) and CMake build would break the cross-platform check. If wrong:
  swap `crates/link/src/ffi.rs` for `rusty_link`.
- **Cross-checking:** the C++ is skipped when the target OS isn't the host's; the crate
  then runs a session of one. If wrong: Linux and Windows builds made from a Mac have
  no Link (release builds are made on each OS).
- **Quantum:** the song's bar in quarter notes (4 in 4/4, 3 in 6/8). If wrong: Maschine
  may use a fixed 4.
- **Starting with Link:** PLAY waits for the session's next bar, alone or not (the
  timeline is never re-mapped); a start the session made (Start Stop Sync) up to half a
  beat ago starts at once, caught up. If wrong: Maschine alone may start at once.
- **Following isn't editing:** a tempo from Link or MIDI clock becomes the song's without
  an undo step or marking the project changed (rounded to 0.01 BPM; changes under 0.005
  BPM from Link, 0.05 from MIDI clock, ignored). A tempo changed here (header, MK3, undo)
  goes to the session. If wrong: Maschine may record followed tempos.
- **Ports:** clock goes to every MIDI output switched on in Preferences ▸ MIDI and is
  taken from every input switched on (Maschine's Devices lists). If wrong: a per-port
  clock switch would be needed.
- **Sending:** only while the song plays: Song Position then Start (from the top) or
  Continue, 24 clocks a beat (counting on through loops and jumps, no Song Position
  mid-play), Stop at STOP or the song's end. If wrong: devices that want clock while
  stopped see none.
- **Clock Offset:** sending 0–200 ms early (Maschine's 0 … -200); receiving -100 …
  +100 ms, positive: the song plays after the clock.
- **Receiving:** PLAY, RESTART and Space do nothing (PLAY greyed, as Maschine's);
  Start plays what PLAY would (the arrangement in the Song view, the focused scene in
  the Ideas view) from the top, Continue from the song position; no count-in. The
  song's beat is the clock's (absolute), not only its phase.
- **Link over receiving:** turning Link on turns Receive Clock off (software and MK3);
  sending stays.
- **Where:** Link's Enabled and Start Stop Sync sit in Preferences ▸ MIDI beside Sync
  (DECKARD has no General page; Maschine has them under General).
- **MK3 status text:** "LINK OFF", "1 LINK", "n LINKS" (Maschine's own wording wasn't
  found); the status and the phase meter fill the page's 240-pixel column.
- **Phase lock:** corrections under a sixteenth move the sequencer's clock (click-free:
  voices play on), larger ones seek. Plug-ins rendered ahead take a new tempo at their
  next position message (a loop, a jump).
- **Tests never join a real session** (`Link::local`); the one that does is opt-in
  (`DECKARD_LINK_NETWORK_TESTS=1`).
