# Launchpad Pro [MK3] integration — design

FEATURES #5. A Launchpad Pro [MK3] next to (or instead of) the MK3: a Session grid of
the Ideas view, notes and its own sequencer playing DECKARD, and Print to Clip writing
a sequence into the focused sound's pattern.

## Intent

- **Session mode**, as Live does it but in Maschine's shape: the grid is the Ideas view.
  Rows are groups (A at the top), columns are scenes, each cell lit in its group's
  colour when that scene has a pattern for the group. The bottom row launches the
  scene above it. The arrows scroll scenes (left/right) and groups (up/down) eight at a
  time when there are more than fit.
- **Note / Chord / Sequencer** on the Launchpad play DECKARD like any MIDI keyboard
  (its notes come on its MIDI port, which DECKARD already listens to).
- **Print to Clip**: the Launchpad's sequencer sends a pattern; DECKARD writes it into
  the focused sound's pattern.
- **Written in Lua, as a controller script.** The user's call (2026-10-28): "if
  possible it would be great to do it all in lua so others can easily add other
  controllers they use but if we lose too much integration then lets not do that".
  It loses nothing: the controller layer's `View` already carries the Ideas grid
  (groups, scenes, their patterns, colours, what plays) and its actions already focus
  and launch scenes. What the layer lacks is a MIDI device: a folder of scripts that
  names its ports, a handler for the bytes that come in, and a way to send bytes out.
  The Launchpad is the first such device; anyone can add theirs the same way.

Non-goals: the Launchpad's DAW Fader layouts (mixer on the grid); Launchpad models
other than the Pro [MK3] (they'd be their own device folders later); Lua in the note
path (notes stay in Rust, for timing).

## Device protocol (Launchpad Pro [MK3])

From Novation's Launchpad Pro [MK3] Programmer's Reference (2020-04-15), and, for Print
to Clip (which Novation doesn't document), from how Ableton Live 11's own Launchpad
script reads it (behaviour only, read from its bytecode constants). Values in hex.

**Ports.** Three: `LPProMK3 MIDI` (Note/Chord/Custom modes' notes; the sequencer's
output), `LPProMK3 DIN` (the MIDI DIN sockets), `LPProMK3 DAW` (Session mode: lights in,
presses out; SysEx both ways). On Windows the names differ; match by "LPProMK3" plus
"DAW" / "MIDI". The device sends note-on velocity 0 for note-off.

**SysEx header** (both ways): `F0 00 20 29 02 0E`, then a command byte.

| Command | Direction | Bytes after the command | Meaning |
|---|---|---|---|
| `10` | to device | `01` / `00` | DAW mode on (Session layout enabled) / off (standalone). Off when the software quits. |
| `00` | to device | `<layout> <page> 00` | Select a layout: `00` Session, `04` Note/Drum, `02` Chord, `07` Sequencer Steps, `11` Programmer, … |
| `00` | from device | `<layout> <page> …` | Sent whenever the layout changes on the device. |
| `03` | to device | `<spec>…` | Light LEDs: each spec is type, LED index, data. Type `00` palette (1 byte), `01` flashing (2: colour B, colour A), `02` pulsing (1), `03` RGB (3 bytes, 0–127). Up to 106 specs per message. |
| `03` | from device | Print to Clip packet | See below. |
| `18` | to device | `01` / `00` | Print to Clip button enabled / disabled (as Live sends it; to confirm on hardware). |

**Session layout numbering** (DAW port, channel 1). Grid pads are notes `11`–`88` in
decimal: row r (1 bottom … 8 top), column c (1 left … 8 right) is `10·r + c`. Buttons
are CCs: top row 91–98 (91/92 are left/right arrows, 93 Session), left column 80, 70,
…, 10 from the top (80/70 up/down arrows; 60 Clear, 50 Duplicate, 40 Quantize), right
column 89, 79, …, 19, bottom rows 101–108 (upper) and 1–8 (lower), Shift 90, logo 99.
Presses: note on/off for pads (with velocity; aftertouch on the same channel), CC 127/0
for buttons.

**Lighting in Session** (DAW port): note-on (pads) or CC (buttons) with the palette
index as velocity/value; channel 1 static, 2 flashing (one beat on, one off, in time with
received MIDI clock, else 120 BPM), 3 pulsing (two beats). The 128-entry palette is in
the reference (0 off, 3 white, 5 red, 21 green, 45 blue, …). Exact colours use the
`03` SysEx with type `03` (RGB).

**Print to Clip** (from the device on the DAW port): `F0 00 20 29 02 0E 03 <d…> F7`.
With `d` the bytes after the command:

- `d[0]`: 1 begin, 2 data, 3 end.
- `d[1..9]`: packet id, 8 bytes of 4 bits each, most significant first. Ids count up
  from 0; one that isn't the last + 1 (other than 0) is a transmission error: drop the
  print and say so.
- `d[9..11]`: not used by Live.
- Data packets (`d` at least 13 long), payload `d[11..]`:
  - exactly 3 bytes: the clip length;
  - else 3 bytes of group offset, then notes of 6 bytes each: start (2 bytes), length
    (2 bytes), pitch, velocity. A payload not a whole number of notes is ignored.
- Multi-byte values are 7 bits per byte, most significant first. Times are in
  1/500 beat: `beats = value / 500`. A note's start is the group offset + its start.
- End: the clip is written. Live sorts the notes by start and wraps notes that end
  past the clip's length onto its start.

## How DECKARD does it

### MIDI devices in the controller layer

A device is a folder `devices/<id>/` (shipped in `crates/surface/lua/`, replaceable from
the user's controller folder as every script is). Its `device.lua` returns:

```lua
return {
  name = "Launchpad Pro [MK3]",
  ports = { daw = { "LPProMK3 DAW", "MK3 Pro DAW" } },  -- role → name fragments
  exclusive = { "daw" },   -- roles DECKARD must not also take notes from
}
```

and its `main.lua` returns handlers, like a page: `open()`, `close()`, `midi(role,
bytes)`, `tick(view)` (each frame, with the same `View` the MK3 gets). Scripts send with
`midi.send(role, bytes)` and call the same actions as the MK3's (`focus_scene`,
`focus_group`, …) plus two new ones:

- `launch_scene(i)`: what clicking a scene does in the Ideas view (focus it; while a
  scene plays, it starts when the playing one ends).
- `print_notes{ length = beats, notes = { {pitch, start, length, velocity}, … } }`.

A device's lookup is `devices/<id>/<path>` (user, then shipped), then `lib/<path>`
(shared helpers) — never the MK3's pages. One Lua state per device, the MK3's own state
untouched, so a broken device script can't stop the MK3 (and the reverse).

The app opens a device when a port matching its manifest appears (checked when the MIDI
port list changes), calls `open()`, routes the port's input to `midi(...)` on the UI
thread, and sends what the script sends. `close()` runs when the app quits or the port
goes away. Ports a device marks `exclusive` are left out of note input.

### The Launchpad script

- `open()`: DAW mode on, Session layout, Print to Clip enabled, the grid drawn.
- `tick(view)`: draws the grid and buttons from the view, sending only LEDs that changed
  (one `03` RGB SysEx per frame for the changes). Only while the device shows Session
  (it reports layout changes; Note/Sequencer layouts draw themselves).
  - A cell with a pattern: the group's colour, dim. The focused scene's cells: full.
    The playing scene's cells: pulsing; a queued scene's: flashing.
  - Bottom row 101–108: a scene there (dim white), the playing one green, a queued one
    flashing green. Arrows lit when there is somewhere to scroll.
- Pad press: focus that group and that scene. Bottom-row press: launch that scene.
- `midi` with a Print to Clip packet: reassembled across packets, then `print_notes`.
- `close()`: DAW mode off (the Launchpad back to standalone).

### Print to Clip into DECKARD

`print_notes` writes into the focused group's pattern in the focused scene, as notes of
the focused sound, as one undoable edit:

- no pattern there: a new pattern is made and put in the scene;
- the sound's notes in the pattern are replaced by the printed ones (the other sounds'
  notes stay);
- the pattern's length becomes the printed length, if longer;
- times from beats to ticks (`PPQ` 960); notes wrapping past the length as Live does.

## Choices made here (the builder's or mine)

- Lua device host rather than Rust per device: the user's preference, at no loss.
- Rows are groups and columns are scenes (Maschine's Ideas view), not Live's tracks as
  columns: the feature asks for scene launch on the bottom row, which only fits this way.
- A pad press focuses rather than launching (the user's choice, 2026-10-28), since a Maschine scene launches all its
  groups together (there is no one-clip launch); the bottom row launches.
- Print replaces the focused sound's notes in the focused pattern (one undo) rather than
  making a new pattern each time (the user's choice, 2026-10-28).
- Clock: the Launchpad's sequencer syncs to DECKARD when MIDI clock is sent to its MIDI
  port (Preferences ▸ MIDI ▸ Sync); nothing new.

## Unverified until a Launchpad is connected

Port names on each OS; the `18` enable message; whether Print to Clip needs the
Session layout or DAW mode only; the feel of colours (dim/full levels); the lighting
rate the device takes without lag. The tests use the byte formats above; a hardware
checklist closes the feature.
