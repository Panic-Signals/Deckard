# DECKARD's audio engine: how it runs, live

docs/maschine-audio-engine.md describes Maschine's own engine, as found in its binary.
This file describes DECKARD's engine: what it takes from Maschine, and where it goes
further. The numbers come from `apps/viewer/examples/audio-stress.rs` and
`timing-test.rs`, run through the real output on an 8-performance-core Mac.

## Threads

```
UI / MIDI input / recorder ──Command──▶ [lock-free queue] ──▶ audio thread (owns the Engine)
          ▲                                                      │  render pass ─▶ worker pool (7 threads)
          └──── freed off the audio thread ◀── [queue] ◀─ old parts (engines, timelines)
          └──── Telemetry (meters, playhead, …) ◀─ try_lock, published after each block
```

- **The audio thread owns the engine.** Nothing else touches it, and it never waits
  for another thread. Commands go through a bounded MPMC queue (`queue.rs`, Vyukov's).
  The queue is allocated when the output opens and needs no locks or allocation after
  that (`output.rs`).
- **Edits arrive ready to use.** The UI rebuilds the engine (`Engine::new`) and makes
  the timelines it may need (`Timelines`): the one for the mode playing and the one
  for a queued mode. The audio thread swaps them in (`Engine::adopt_with`); nothing is
  built or freed there. The replaced parts go back through a second queue and are
  freed on the UI thread.
- If playback changed mode in the moment between building and applying an edit, the
  engine reports `Telemetry::stale` and the UI sends the edit again.
- **Reads never touch the engine.** Meters, the playhead, sample positions, the pending
  jump and the tap's frame count are published after each block. The UI reads that.
  `Output::is_current` tells whether what's published already includes the commands
  sent.
- **Queries that need the engine's state at that moment** (a section jump decided
  against the exact position; the recorder's note start) run on the audio thread as
  `Command::Run`, a boxed closure that is sent back to be freed.

## Live timing: sample-accurate, no jitter (better than Maschine)

Maschine applies UI and controller events at the start of the next block: every call
site passes time 0. A hit's timing therefore moves by up to one buffer, depending on
where in the block it came.

DECKARD stamps each live event when it happens (`Command::Live(Instant, Live)`). The
audio thread plays it exactly one block later, at its sample (`Engine::schedule`; the
block loop splits at it, as it does for sequenced notes). The latency is constant and
the jitter is gone.

| 256-sample buffer (5.3 ms) | Timing spread over 30 notes |
|---|---|
| next block (Maschine's way) | 4.95 ms (sd 1.55 ms) |
| sample-accurate (DECKARD) | **0.17 ms** (sd 0.03 ms) |

The switch is Preferences ▸ Audio ▸ Live timing. Turning it off gives Maschine's
behaviour: half a buffer less latency on average, but the jitter is back.

## Multi-core rendering

Each sound's work (its Sampler voices or plug-in, then its effects) is a job, done by a
worker pool and the audio thread (`pool.rs`).
- Claiming a job is a single compare-and-swap.
- The workers run at time-constraint priority and join the device's audio workgroup.
- Mixing, side-chain feeds, taps and meters stay on the audio thread in a fixed order,
  so the output is bit-identical with any number of threads. This is checked by a test
  and by renders of real projects.
- The average load fell 2–3× on plug-in-heavy projects (4-6 show: 8.6 % → 2.8 %).

## Click-free changes while playing (better than Maschine)

- **Level, pan and mute** glide over 10 ms (`GainRamp`). The Sampler renders at unity
  and its level is applied in the mixer, like every other sound's. Once a gain arrives
  it is the exact value, so steady playback is bit-identical to before.
- **Effects** live in the chain with a mix (`Unit`):
  - Switching one off crossfades over 20 ms. It stays in the chain, frozen, so it
    comes back with its state (no reset, no overshoot).
  - A new effect fades in. A removed one (Maschine's own) stays where it was to fade
    out.
  - Edits pair effects by order: the longest common run of the same classes or
    plug-in instances, computed on the stack. Removing slot 1 no longer hands slot 2's
    effect someone else's state.
  - Bypassed plug-ins are kept only if they are already loaded.
- Tests `level_changes_glide`, `bypass_crossfades_and_keeps_state` and
  `removing_an_effect_fades_it_out` fail with the glides off: there is a step at the
  edit.

## Measured (256 samples, three projects, every phase: playback, UI polling, a plug-in page, knob turns, edits)

- **0 blocks missed.** The audio thread can't miss one now; it never waits.
- **Late blocks:** 1 in 30 phases. It was Reaktor 6 taking 5.8 ms in one call just
  after start, which is the plug-in itself.
- **Typical peak load:** 9–17 %, including while edits arrive (it was 53–76 % with the
  first lock-based design).

## Render ahead (further than Maschine)

Optional, and off by default (then every sound plays live, as in Maschine). Turned
on, sounds that can't get live input right now are rendered on a thread of their own,
10–80 ms ahead of the audio thread (`ahead.rs`). "Can't get live input"
means:
- not in the focused group (pads and keys play the focused group);
- no MIDI source of their own, and not in a group set to a MIDI key mode;
- no audio input;
- not part of a side-chain;
- no MIDI routed between them and another sound.

How it works:
- **The engine keeps the one sequencer.** Each block, `Engine::foresee` predicts the
  notes of the coming frames, and the engine sends them to the sounds' players, stamped
  with their output frame.
  - Probability is decided by a hash of the event and its pass through the timeline, so
    it comes out the same whenever it's asked.
  - Notes already sent aren't sent again. After a jump or an edit, new notes are sent
    (late, at once), and none plays twice.
- **Each player owns its sound** (its Sampler, plug-in and effects) and writes its
  output, after effects and before level, into a ring buffer. The audio thread mixes the
  ring with the sound's level, pan and mute (gliding), its meters and its taps. Mutes
  and levels therefore stay instant.
- **Switching a sound between live and ahead happens at an exact frame:**
  - Live to ahead: at the end of a span. The sound's next notes go with its parts.
  - Ahead to live: at the frame where its player stops, which the audio thread chooses
    (the furthest the player may have rendered), so the block can be split there.
- **Edits** go to the player in the sound's own state, and take effect a lookahead
  later.
- **Stop** reaches players a lookahead later.
- **A live note for a sound rendered ahead** (MIDI into a group without the focus) is
  played exactly a lookahead and a block later: delayed, but with no jitter.

Checked by:
- Unit tests:
  - `rendering_ahead_sounds_the_same`: bit-identical to rendering on the audio thread.
  - `moving_between_live_and_ahead_is_seamless`: bit-identical with the focus switching
    every 7 blocks.
  - `edits_reach_sounds_rendered_ahead`: a level edit is bit-identical; an effect edit
    converges after a lookahead.
  - `live_notes_reach_sounds_rendered_ahead`: exactly lookahead + block later.
  - `stop_reaches_sounds_rendered_ahead`.
  - `foresight_matches_the_sequencer`: the prediction equals what the sequencer fires
    (loops, scene switch, section jump, probability).
- Real projects rendered offline with `--ahead` against without:
  - 4-6 show: identical.
  - Exosphere: −197 dB.
  - Plate Tectonics: −137 dB.
  - Orange Ten: −88 dB. The Sampler updates its modulation every few samples counted
    from each render call, and players cut their render calls at other frames.

Measured live, 4 s per run, focus none (every sound that can go ahead does):

| Project, buffer | Audio-thread average load: off → 20 ms ahead | Typical peak |
|---|---|---|
| Exosphere (31 sounds ahead), 128 | 8.6 % → 1.5 % | 13–15 % → 7 % |
| Cloud 9 Vibes (13 ahead), 128 | 6.0 % → 0.8 % | 14–15 % → 3–4 % |
| 4-6 show (13 ahead), 256 | 2.5 % → 2.6 % | 6 % → 4–5 % |

Players were never late in any run.

The live timing test, with the focused group playing (render ahead on), kept a timing
spread of 0.16 ms.

The setting is Preferences ▸ Audio ▸ Render ahead: Off (the default) / 10 / 20 / 40 /
80 ms. It is off by default because every rule above has to hold or a sound plays
differently depending on the focused group: a side-chained Limiter stopped once
another group was focused after an edit (fixed; the "listens" rule now comes from the
project, not from the chains an edit leaves unbuilt).

## Incremental edits

Every edit used to build a whole new engine on the UI thread: every effect, every
Sampler (searching the disk for each sample), and the timeline. That cost 2–8 ms on
these projects, once per frame while a knob was dragged.

`Engine::new_after(project, prev)` builds only what an edit changed. `prev` is the
project the playing engine was built from, and which sounds have effect chains. For each
sound (`reuse_of`):
- **Sampler:** kept if it is unchanged. If only its settings changed (same zones), it
  is kept and the new settings are swapped in. Otherwise it is rebuilt.
- **Plug-in instrument:** kept if it is the same plug-in and state. Its gain and mute
  are updated in place.
- **Effect chain:** kept if it has the same slots (effects, on/off, plug-ins) and
  side-chains. Changed effect settings are set in place, as a knob turn is.
- **Gains** (level, pan, mute) always come new.

Whole-engine shortcuts:
- **The timeline** is kept if the patterns, scenes, sections, grooves and bar length
  are the same. No timelines are then made or sent either.
- **Fallback:** if the project's shape (groups, sounds) or its side-chains changed,
  everything is built.

The playing engine moves the parts it keeps into the edit before adopting it
(`keep_parts`), so the rest of adopting leaves them alone. Ahead players get the same
keep instructions with their edit.

Edits to sounds rendered ahead now land at a fixed frame, a lookahead and a block after
they're made. Before, they landed wherever the player happened to be, which varied from
run to run.

Found sample paths are cached (`SampleCache::found`), so even full builds stop searching
the disk.

Checked by `incremental_edits_sound_like_full_rebuilds`. A level, a Sampler knob, an
effect knob, a bypass, a note added, and an audio mute and unmute, built incrementally,
sound bit-identical to the same edits built in full, rendered here and ahead.

Measured (audio-stress, alternating level and effect edits):

| Project | Full build (avg / max) | Incremental (avg / max) | Edit to audio thread |
|---|---|---|---|
| 4-6 show | 2.9 / 5.7 ms | 0.16 / 0.40 ms | 11 µs (was 41) |
| Exosphere | 3.3 / 8.0 ms | 0.28 / 0.57 ms | 10 µs (was 506) |
| Cloud 9 Vibes | 2.1 / 3.9 ms | 0.08–0.4 ms (build alone) | 6 µs (was 293) |

## Next: further than Maschine

1. **Anticipative rendering:** done (above). Still open:
   - sounds in a side-chain stay live;
   - after a jump or an edit, notes newly due within the lookahead play late by up to
     that much.
2. **Incremental edits:** done (above).
3. **Plug-in protection for shows:**
   - per-plug-in CPU meters (every call is already timed: `take_slowest`);
   - an optional guard that bypasses (with a fade) a plug-in that keeps overrunning;
   - plug-ins hosted in a separate process, so a crashing one can't take the set down.
4. **Sample-accurate automation.** Knob and controller moves would be stamped like
   notes, so automation recorded live plays back exactly.
5. **Latency-compensated recording.** The same timestamps would place recorded notes
   and audio exactly where they were played.

## Group and master effects

- **Group buses.** A group whose chain has effects is a bus (`Bus` in
  `crates/engine/src/lib.rs`). A sound's route stops at the first group with effects
  (`group_route` returns `GROUP_BUS + g`), so its gain is only what comes before the
  group's input. Its sounds mix into the bus buffer, rendered-ahead ones too.
  After each sub-span's sounds (`run_buses`), a bus runs its chain, then its fader and
  pan onto where the group goes (`group_out_route`): the master's output, an external
  output, the cue bus. Buses run in order, one feeding another first.
  Groups without effects stay folded into their sounds' gains, so those projects render
  as before; a bus whose effects are bypassed sounds the same as no bus.
- **Master.** The master's level and pan are applied once to its output. Its chain
  (`Project::master_slots`, read from the song's chain) runs on that buffer just before
  them. Channels routed straight to the master's output past the master also pass
  through its effects, a DECKARD simplification.
- **Edits.** Group and master chains carry their state over on edits
  (`FxChain::adopt`), as sounds' chains do. Plug-ins on them are keyed `Slot::Group` /
  `Slot::Master` in the plug-in host.
- **Document and UI.** Chain edits take a `deckard_doc::Chain` (`Sound`, `Group`,
  `Master`): `add_slot_in`, `remove_slot_in`, `move_slot_in`, `set_slot_active_in`,
  `set_chain_plugin_chunk`. A group's or the master's effect parameters are
  `Target::ChainModule(chain, slot)`. The chain area's MASTER / GROUP / SOUND tabs
  and the MK3's PLUG-IN page edit whichever chain is shown (`App::chain`). The Mix
  view's group and master strips list, bypass and show their effects.
