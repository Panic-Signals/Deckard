# Launchpad Pro [MK3]

DECKARD drives a Launchpad Pro [MK3] connected over USB, with no setup. When it's
plugged in, the status bar says "Launchpad Pro [MK3] connected" and the Launchpad goes
into DAW mode. When DECKARD quits, it goes back to standalone. The behaviour is the
script `devices/launchpad_pro_mk3/main.lua`, which you can copy into your controller
folder and change (`controller-api.md` "MIDI controllers").

## Session

The grid is the Ideas view as Maschine shows it. Each column is a group (the eight
the focused group is in) with its patterns down the top seven rows, the first at the
top. The grid's bottom row is the playhead.

- A pattern is lit in its colour: full where the focused scene plays it, dim where
  not. Focus another scene and the grid shows that scene's patterns.
- **A pad** turns its pattern on or off in the focused scene. Turning one on also
  focuses it. A group plays one pattern in a scene, so the one it played goes off.
- **The faint white pad** after a group's last pattern makes a new pattern there, in
  the scene.
- **The playhead** (the grid's bottom row): while playing, the longest pattern in the
  focused scene spread over the eight pads, however long it is: an eighth each. The
  part played is dim, the pad playing full white, those still to come faint, so you can
  see how long until it loops round. Stopped, its first pad strobes with the tempo.
  Its pads do nothing.
- **The row of buttons under the grid** shows the scenes in their colours: dim, the
  focused one full, the playing one pulsing, a queued one flashing. A press launches that
  scene, and when stopped it starts playing. The faint white button after the last
  scene makes a new scene, focused.
- **Left and right** scroll the scenes eight at a time, always as far as the place for
  a new one. **Up and down** scroll the patterns seven at a time. **SHIFT + left or
  right** focuses the previous or next eight groups. The arrows are lit when there's
  somewhere to go.
- **Mute** is a mode (yellow while on; press it again to leave). A pad mutes or
  unmutes its column's group. A scene's button mutes the groups that play in it, or
  unmutes them when they're all muted already. A muted group's pattern in the focused
  scene shows dimmer than full.
- **Play** starts and stops playback (green while playing). **Record** starts and
  stops recording, as RECORD on the MK3 (red while recording, dim otherwise).

## Note, Chord and Sequencer

These are the Launchpad's own modes. Their notes come on its MIDI port and play the
focused sound as any keyboard does. For the sequencer to follow DECKARD's tempo, send
MIDI clock to `LPProMK3 MIDI` (Preferences ▸ MIDI ▸ Sync).

**Drum tracks and the Drum layout** send notes from C1 (36) up, laid out as Maschine's
pads are: four by four from the bottom left. To have them play the focused group's
pads, as pad mode does:

1. On the Launchpad, give the drum track a channel of its own (SHIFT + Sequencer,
   then the track's channel). The sequencer's tracks send on channels 1–4 by default
   and Note mode on 1, so pick another, 10 say.
2. In DECKARD, Preferences ▸ MIDI ▸ **Drum channel**: set Launchpad Pro [MK3] to the
   same channel.
3. For Print to Clip, set **Prints** beside it to the drum track's number (1–4). A
   print says which track it came from but not what kind, so DECKARD needs telling.

Notes 36–51 on that channel then play pads 1–16 of whichever group is focused, each
sound at its root. Notes outside them play nothing. The Launchpad's other channels
play the focused sound as before. (A group's own MIDI input does the same for one
group: its Channel properties, Input ▸ MIDI, Key Mode Manual, Source `LPProMK3 MIDI`,
the drum channel, Start Note C1.)

**Print to Clip** (the Launchpad's button, in Sequencer mode) writes the sequence into
the focused group's pattern in the focused scene. A pattern is made if there is none,
and the pattern takes the sequence's length, longer or shorter. One undo takes it all back. Notes
running past the end wrap round to the start. If a packet goes missing on the way,
nothing is printed and the status bar says so.

- When the group takes MIDI as pads (Key Mode Manual or Drumkit), the print plays its
  pads as the notes would live: it replaces all of the group's notes in the pattern,
  and notes outside the 16 pads are left out.
  So does a print from the track set as the drum track (Preferences ▸ MIDI ▸ Drum
  channel ▸ Prints), into the focused group: notes from C1 play pads 1–16.
- Otherwise every note goes to the focused sound at the pitch the Launchpad sent,
  replacing that sound's notes.

## Checklist with the hardware

Checked on 2026-10-28 with a Launchpad attached but not watched (no one to look or
press): on macOS the ports are "Launchpad Pro MK3 LPProMK3 MIDI / DIN / DAW". It takes
DAW mode, the Session layout select (and reports it back), the Print to Clip enable and
the lighting messages without complaint. It reports the Note layout after DAW mode is
off.

Still to check by hand:

- [x] The ports are found on macOS. Windows is still to check: `device.lua` guesses its
      DAW port names (`MIDIIN3 (LPProMK3`, `MIDIOUT3 (LPProMK3`). If they're wrong, the
      wrong port is kept out of note input.
- [ ] The Session lights match the Ideas view and the colours are readable (dim, full).
- [ ] The playhead row moves with the longest pattern and strobes with the tempo when
      stopped.
- [ ] Mute mode mutes groups from the pads and from the scene buttons.
- [ ] Pulsing and flashing (the playing and queued scenes) keep time and don't lag.
      (They're drawn by DECKARD from the tempo, not the device's own clock-synced
      modes, so they aren't in phase with the bar.)
- [ ] Play starts and stops, Record records; both light as described.
- [ ] A pad turns its pattern on and off in the focused scene; the faint pads make a
      pattern and a scene; the arrows scroll with nine or more scenes or patterns, and
      SHIFT + left / right with nine or more groups.
- [ ] A drum track on the drum channel plays the focused group's pads, and with
      Prints set to its track, its print lands on them too. (2026-10-28: the print's
      track byte read on the hardware: `04 00` for track 1, `04 01` for track 2.)
- [ ] The Note layout plays the focused sound. The DAW port's presses play nothing.
- [ ] The sequencer follows DECKARD's clock.
- [ ] The Print to Clip button lights (the `18 01` message) and printing writes the
      pattern.
- [ ] Unplugging and plugging back in works. Quitting leaves the Launchpad in
      standalone.
