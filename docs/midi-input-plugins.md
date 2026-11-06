# MIDI input plug-ins

A slot in the engine for code that takes notes and plays other notes: an arpeggiator, a chord
maker, a Euclidean generator, a note filter. This is the native half of Feature 15 in
`FEATURES.md`. It has the slot and the timing, in Rust, on the audio thread. There's no UI, nothing
in the project file, and no plug-in ships yet.

## What it does

`Output::set_midi_input(group, Some(plugin))` puts a plug-in on a group, one per group. Then:

- **Every note heading for that group's sounds goes in,** at the point both paths meet
  (`Engine::trigger_with`): pads, keys, Note Repeat and the Arp's output, and pattern notes. Choke
  and controllers don't, and neither do muted sounds' notes.
- **Only what the plug-in plays is heard.** It can play nothing, or many notes, on any of the
  group's sounds.
- **Recording is before it.** A take holds what was played, and on playback those notes go through
  the plug-in again. Note Repeat and the Arp come before it too, so a take holds what they play.
- **Sounds rendered ahead:** a plug-in's group sends those sounds no pattern notes in advance. What
  the plug-in plays reaches them as live notes do, so they're heard a player's lookahead later. Notes
  already sent when a plug-in is put in still play, so it takes effect up to a lookahead late.

## Writing one

```rust
impl MidiInput for OctaveUp {
    fn note(&mut self, note: NoteIn, clock: Clock, out: &mut Outbox) {
        match note {
            NoteIn::On { sound, pitch, velocity, length } => {
                out.on(clock.now, sound, pitch + 12, velocity, length);
            }
            NoteIn::Off { sound, pitch } => {
                out.off(clock.now, sound, pitch + 12);
            }
        }
    }
}
```

| Method | When |
|---|---|
| `note` | A note came in. A note with a `length` gets no `Off` |
| `process` | Once per block: play what's due before `clock.now + clock.span`, as `Repeater::due_into` does |
| `stopped` | The transport stopped: the engine has ended every note, and dropped what was still to play |
| `finish` | It's being taken out: end what it holds |

- **Time:** `Clock::now` is in ticks on a clock that runs on when the song stops, and notes are
  played at times on it. `Clock::song` is where the song is, `None` when stopped, for lining up with
  its grid.
- **The audio thread:** nothing here may allocate, wait or panic. `Outbox` holds 256 notes per call,
  and keeps 64 of those for note-offs so a full box can't leave a note stuck.
- **Note ownership:** a note played with no length is the plug-in's to end. An `Off` may come for a
  key it never saw go down, if it was put in while the key was held.

## What's left for later

- Saving a plug-in on a group in the project, and choosing one in the UI and on the MK3.
- Feature 15's Lua scripts. They run off the audio thread, so they'd be one plug-in here that plays
  what a script scheduled ahead.
- Per-note expression doesn't go through, and plug-ins aren't in exported audio: an export's engine
  is built separately.
- Slots are by group index, like the engine's other per-group state, so moving or deleting a group
  needs whoever put the plug-in in to move it.
