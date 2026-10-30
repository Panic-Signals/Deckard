# The DECKARD block

This is what DECKARD keeps in a project that Maschine has no place for. Today that's
per-note expression (MPE), channel pressure, and each sound's Pad Touch.

It is saved as one extra item at the root of the `.mxprj` hsin tree, after Maschine's
`MAS2/PROJ`:

```text
root                 frames 118 > 106 > 1
  MAS2/PROJ          (Maschine's project, unchanged)
  DKRD/EXPR tag=0    frames DKRD (the payload below) > 1
```

- The domain is `DKRD`, the item id `EXPR` and the frame id `DKRD` (fourcc as a
  little-endian u32 of the big-endian bytes, as Maschine stores `MAS2`). The frame is
  version 1, and the item has a random uuid and no children.
- Real Maschine 3 opens a project carrying the block and ignores it. The user confirmed
  this on 2026-10-24 with a Velvet Lounge copy. Maschine drops the block when it
  re-saves the project.
- DECKARD writes the block only when there is something in it. A project with nothing
  to keep saves exactly as before. Each save replaces the block, so there is never more
  than one. Code: `mxprj::container::set_root_child` and `root_child`, and
  `crates/doc/src/block.rs`.
- A block DECKARD can't read (bad bytes, or a version it doesn't know) is left out with
  a message, and the project opens without it.

## Payload, version 7

Version 7 is version 6 with the CC routes at the end; version 6 is version 5 with the LFOs at the end; version 5 is version 4 with the screen at the end; version 4 is version 3 with a PROB
LOCK section at the end; version 3 is version 2 with a modulation routes section at the
end; version 2 is version 1 with a Pad Touch section at the end. DECKARD writes version 7
and reads all seven.

All values are little-endian. `u32` and `i32` are 4 bytes, `i64` is 8, `f32` is an
IEEE single.

```text
u32 version = 7        (6: without the CC routes; 5: without the LFOs either; 4: without the screen either; 3: without the PROB LOCK section either;
                       2: without routes either; 1: without Pad Touch either)
u32 pattern_count
pattern × pattern_count:
  u32 group            index in the project
  u32 pattern          index in the group; clip patterns are 2^24 + k
  u32 entry_count
  entry × entry_count:
    u32 sound          index in the group
    i64 position       the note's position (ticks from the pattern's start)
    i32 pitch
    u32 ordinal        among the pattern's notes with this sound, position and pitch,
                       in note order (0 for the first)
    u8  release_velocity   1–127, 0 = none
    u8  dims           bit 0 Pitch, bit 1 Slide, bit 2 Pressure
    curve × (bits set in dims, in that order):
      u32 point_count
      point × point_count:
        i64 at         ticks from the note's start, increasing
        f32 value      Pitch in semitones; Slide and Pressure 0–1
        f32 curve      the bend of the segment to the next point; 0 = straight
u32 lane_count         channel pressure (Maschine's "Aftertouch" lane)
lane × lane_count:
  u32 group, u32 pattern, u32 sound
  u32 event_count
  event × event_count:
    i64 position       ticks from the pattern's start
    u8  value          0–127
u32 touch_count        version 2: sounds' Pad Touch, those not at the default
touch × touch_count:
  u32 group, u32 sound   indices in the project when saved
  u8  mode             0 Velocity, 1 Poly AT, 2 Aftertouch, 3 CC
  u8  cc               0–127
  u8  curve            0–100 (%; 50 straight)
  u8  min, u8 max      0–100 (%), max ≥ min
u32 routes_count       version 3: sounds' modulation routes, those with any
routes × routes_count:
  u32 group, u32 sound   indices in the project when saved
  u32 text_length
  u8  text × text_length  the routes as text, UTF-8: one per line,
                       `source slot param low high curve range module`
                       (source pressure|slide|pitch; param f:<field> or p:<plug-in
                       parameter id>; low, high 0–1 of the knob's travel; curve 0–100;
                       range ± semitones; module the slot's class or plug-in id, last)
u32 locked_count       version 4: the sounds with PROB LOCK on
locked × locked_count:
  u32 group, u32 sound   indices in the project when saved
u8  has_screen         version 5: 1 if the screen follows
screen (if has_screen):
  u32 group, u32 sound, u32 scene, u32 pattern   the focus when saved
  u32 slot             the focused plug-in slot + 1 (0: none)
  u8  channel          1: the Channel page shown rather than the plug-ins
  u32 page_len, u8 × page_len   the controller's page (DECKARD's layer; "" if none)
  u32 mode_len, u8 × mode_len   the pads' mode: pad, keyboard, chords or step
u32 lfos_len, u8 × lfos_len   version 6: the project's LFOs as UTF-8 text (`lfo::to_text`)
u32 cc_len, u8 × cc_len       version 7: the project's CC routes as UTF-8 text
                              (`mod_matrix::cc_to_text`)
```

**CC routes** (docs/modulation.md) send an LFO out as a MIDI CC. They live in the
document as a `deckard:cc_routes` text field on the project's root object (the same
text), one route a line: `source channel cc low high curve port`, e.g.
`lfo2 10 74 0 0.5 50 IAC Driver Bus 1` (the port last; it may hold spaces). Lines that
don't read, or whose source isn't an LFO, are skipped; channel is clamped to 1–16, CC to
0–119 (120–127 are channel mode messages), low and high to 0–1, curve to 0–100.

**LFOs** (docs/superpowers/specs/2026-10-28-lfos-design.md) live in the document as a
`deckard:lfos` text field on the project's root object (the same text), so undo carries
them; lines that don't read are skipped and values are clamped. Triggers are (group,
sound) indices, moved when a group is deleted and turned off when a sound is reset.
A line is `pad shape sync rate hz phase level quantize fade trig`, then optional words:
`muted`, `env` (Envelope mode, docs/superpowers/specs/2026-10-29-lfo-envelope-design.md)
and the envelope's `a=` `d=` `s=` `r=` `rt=` (attack, decay, release in seconds 0.001–10,
sustain 0–1, retrigger 0/1; written in both modes). Older text reads as an LFO with the
default envelope; older readers skip the words they don't know (`muted` stays first).

**The screen** (BUGS #6) is where the project was left: it reopens on the same group,
sound, scene, pattern and slot, the Channel page or the plug-ins, and on DECKARD's layer
the controller's page and pad mode (a dialog or a page shown while a button is held
isn't come back to; `global.lua`'s `restore`). It's kept on every save and in the
session, and setting it is no edit. What's no longer there (a removed group, slot) is
left at the default.

**PROB LOCK** (the Events and Step pages' display 8) lives in the document as a
`deckard:prob_lock` field (`1`) on the sound's object. Maschine has its own (the sound's
`ProbabilityLocked`) but doesn't save it in its projects, so it's DECKARD's here.

**Modulation routes** (docs/superpowers/specs/2026-10-24-expression-out-design.md, 2b)
live in the document as a `deckard:mod_routes` text field on the sound's object (the
same text), so copying, pasting and undo carry them; lines that don't read are
skipped.

**Pad Touch** (docs/superpowers/specs/2026-10-24-pad-touch-design.md) lives in the
document as `deckard:touch_*` fields on the sound's object, so copying, pasting and
undo carry it; the block writes it by the sounds' indices at save, and opening puts it
back on those sounds. Values out of range are brought into range when read.

**Which channel pressure.** Only the channel pressure DECKARD made (recorded or drawn
in DECKARD, marked in the document with a `deckard:made` field that's never saved) goes
in the block. Events Maschine made stay in its archive, so a project without DECKARD's
own data saves exactly as it was. Group and sound files have no block, so DECKARD's
channel pressure is left out of them.

**Matching entries to notes.** A note is found by its sound, position, pitch and
ordinal. An entry whose note isn't in the project is dropped. Curves are cut at the
note's end when saved.
