# Maschine JAM

DECKARD drives a Maschine JAM over USB with no NI software: every press, strip and light goes
through its own driver (`crates/jam`) and script (`crates/surface/lua/devices/maschine_jam/`). It
follows *MASCHINE JAM Manual*, September 2016. A JAM works beside an MK3, and picking a group on
either moves both.

**Status, 2026-11-05.** Played by hand on one JAM on Windows, and working in general; the checklist
below isn't all ticked, and macOS and Linux haven't been tried. What each bit and LED is comes from a
third-party driver, checked against the device's descriptor: see "Checking it" below.

**NI's services fight DECKARD over the lights.** With them running, the pads turn orange and the lights flash at every press. So when a JAM connects, DECKARD offers to stop them, which asks for administrator approval. It doesn't offer while Maschine runs. Maschine needs them, so start them again in Preferences ▸ MIDI, or restart Windows.

Preferences ▸ MIDI ▸ **Maschine JAM** switches it off. DECKARD opens the JAM whether or not
Maschine is running: with both running, each acts on every press and they overwrite each other's
lights, so switch one off.

Preferences ▸ MIDI ▸ **Focus the group MIDI is recorded into** also decides what a Project View pad does
to the focus. On, the default, picking another group's pattern focuses that group, as the manual has it.
Off, the pattern goes on and the focus stays where it was.

## There's no on-screen overlay

What Maschine's overlay shows goes to the status bar as one line, and the encoder walks those
lines the way the manual walks the overlay: **turn to pick a line, press to change it, turn, press
again.** Touching the encoder opens what the view has (the scale in Keyboard, Piano Roll and
Notes; Note Repeat or the Arp while on; morphing in Lock View), and it closes after six idle
seconds. BROWSE opens the library as the main window's own panel, with the same encoder and D-pad.

## The matrix

| Press | Shows | Pads do |
|---|---|---|
| SONG | Project View: each column a group's patterns; bright is what the focused scene plays, half-lit what it doesn't, unlit empty | Pick a pattern, deselect it, or make one |
| PAD MODE | Sounds 1-16, printed at the bottom right | Play; focus the sound |
| SHIFT + PAD MODE | Keyboard: the focused sound on all 64, in the scale, roots white | Play; D-pad up and down move octaves |
| STEP | The focused sound's steps; a light chases the position; scene buttons pick the part | Add or remove a step |
| STEP + 1, 4 or 8 | One sound, four (two lanes each) or eight (one lane each) | A lane's steps; the focus stays |
| SHIFT + STEP | Piano Roll: across is time, up is pitch in the scale | Add or remove a note; D-pad moves the pitches |
| NOTES | Each column's notes for its strip | Add or remove a note |
| SHIFT + LOCK | Up to 64 snapshots | Recall, or take one in an empty place |
| SHIFT + SOLO (PAT LENGTH) | The pattern's length in bars | A pad is a bar |

While SELECT, SOLO, MUTE or a strip mode's button is held, the matrix shows sounds 1-16 in any view
and the pads don't play.

## Other buttons

| Button | Does | With SHIFT (as printed) |
|---|---|---|
| SCENE 1-8 | While playing, the scene plays next; stopped, it's selected and PLAY starts it. The place after the last makes a scene | The bank of eight |
| GROUP A-H | Focus the group. The place after the last makes one | A Follow, E Variation, G Save |
| Top row of pads | | UNDO, REDO, QUANT, QUANT 50%, SEMI−, SEMI+, OCT−, OCT+ |
| D-pad | Left and right: group banks. Up and down: patterns, octaves or pitches by view | In the library, see below |
| CLEAR + pattern, scene, group or sound | Deletes it (a scene's patterns stay) | CLR AUTO: clears the focused sound's modulation |
| DUPLICATE + source, then targets | Copies, onto each in turn; the place after the last makes a new one | DOUBLE the pattern |
| SOLO, MUTE + group or pad | Solo or mute it | SOLO: PAT LENGTH. MUTE: CHOKE all notes |
| SELECT + pad | Focus a sound silently | ACCENT: new steps at the accent velocity |
| Mode + SONG | Pins SELECT, SOLO, MUTE, DUPLICATE or CLEAR; next time, that button is on until pressed again | |
| NOTE REPEAT | Held pads repeat, in Pad or Keyboard mode | ARP, in Keyboard mode |
| PLAY | Start or stop | RESTART |
| REC | Record | COUNT-IN |
| ◀ ▶ | Pages of plug-in parameters, or sounds 1-8 and 9-16 | METRO, LOOP |
| TEMPO | The encoder sets the tempo | TAP |
| GRID | The encoder sets Step, Nudge, Arrange and Perform grids; held, 1-4 pick 1/4, 1/8, 1/16, 1/32 | REC MODE |
| MST, GRP, CUE, IN1 | What the meters show and the encoder sets (0.5 dB a click) | IN1: IN 2 |
| LOCK | Takes or recalls a snapshot; in Lock View, updates the one in focus | Lock View |
| Footswitch | Tip: play. Ring: record | |

SHIFT with the encoder is finer, or ten results at a time in the library.

IN1 and IN2 are the interface's first two stereo inputs, as Preferences ▸ Audio ▸ Routing connects them.
Their levels are DECKARD's own, on the way in, so a sound taking the input and the recorder both hear
them; they're kept in the settings and also in Preferences ▸ Audio. The input stays open while one is on
the meters, so it reads with no sound taking it.

CUE shows the cue bus: the channels with their headphones on, the metronome and the browser's prehear.

## The strips

| Mode | Each strip | Notes |
|---|---|---|
| LEVEL; SHIFT: pan | A group's or sound's | |
| AUX; SHIFT: AUX 2 | A group's or sound's send | |
| TUNE | A group's or sound's | Sampler sounds only |
| SWING | A group's or sound's; the encoder also has the master's | |
| CONTROL | The eight parameters of the page shown; ◀ ▶ turn pages; CONTROL + up or down moves plug-ins, past the last to the "+" | |
| MACRO | The channel's macros | |
| PERFORM | The focused group's Perform FX: touch, move, let go | SHIFT + PERFORM picks the effect |
| NOTES | Strums its column's notes, each finger its own; NOTES + 1, 2, 3: Guitar, Chords, User | |

Held, a mode's button plus a group makes the strips groups, plus a pad makes them that sound's eight,
plus MST (CONTROL and MACRO) the master. A double tap resets a strip. SHIFT moves a tenth as far.
Hold AUTO and move a strip to write modulation while playing.

## The library

BROWSE opens it. Up and down walk the file type, Types, Sub-types and Modes; turning changes each as
it goes. Turn past the last file type for User or Factory, press to switch. Right is the results:
turn, press to load, SHIFT + press for a favourite, up for the favourites switch.

SHIFT + left resets the filters, SHIFT + right brings them back, SHIFT + up or down loads the
previous or next.

## What differs from the manual

| Manual | Here |
|---|---|
| On-screen overlay | The status bar, and the main window's library panel |
| Pad velocity from pressure | The click-pads report on and off: every hit plays at the song's Fixed Velocity value |
| Loop range by SHIFT + ▶ and two scene buttons | SHIFT + ▶ switches looping; DECKARD has no scene loop range yet |
| A group's Tune | DECKARD has no group tune: it moves the group's Samplers together. Plug-in instruments don't move |
| Level and Swing written with AUTO | Only what's a Control-page parameter writes modulation; the mixer's level isn't one yet |
| Variation's on-screen settings (SHIFT + E) | The focused sound's Groove ▸ Variation page, on the strips |
| Plug-in mode, MIDI mode, Controller Editor, SHIFT + F (TS EDIT), SHIFT + H (INSTANCE) | Not here |
| LOCK again updates the snapshot | LOCK recalls it, as on DECKARD's MK3; in Lock View it updates the one in focus |
| Pattern slots with gaps | A group's patterns have no gaps: an unlit pad adds after the last |

## Checking it

With Maschine closed, from the repository:

```
cargo run -p deckard-jam --example jam-check
```

Press anything: it says the name it thinks that is, and lights it. Pads say their column and row, strips
show the finger, the encoder fills the meters. A wrong name or a light upside down is a line in
`crates/jam/src/map.rs`. `jam-learn` and `jam-led-learn` (docs/jam-usb.md) find bits from scratch if
the table turns out wrong altogether.

## Hardware checklist

Ticked by hand. macOS and Linux are unchecked until someone tries them.

- [ ] Every button's name and light (`jam-check`), the D-pad, and the footswitch's two bits
- [ ] Strips and meters the right way up (`BOTTOM_FIRST`); a strip's top reads 1
- [ ] The encoder's direction; one click is one step
- [ ] Press and hold pads with NI's agent running: input streams
- [ ] Pads feel as immediate as the MK3's, and a recorded take lands on time
- [ ] Lights keep time with the window minimized and the MK3 unplugged
- [ ] A kit loaded from the JAM without touching the mouse
- [ ] With an MK3: a group picked on either moves both
- [ ] On closing, what the JAM shows; and that Maschine redraws it afterwards
- [ ] Report `0x40` while pressing pads softly and hard: confirm the pads have no pressure
- [ ] IN1 and IN2 read a signal, in stereo, and the encoder moves what a sound taking them plays
- [ ] CUE reads a cued channel and the click; turn it and only that output changes
