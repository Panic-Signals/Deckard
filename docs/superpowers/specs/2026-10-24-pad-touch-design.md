# Pad Touch: per-sound pad pressure modes (design)

Status: proposed, 2026-10-24.

## Why

The user plays a Hydrasynth (and other synths) from the MK3's pads and wants pressing a
held pad harder or softer to control the synth, per sound: poly aftertouch on one
sound, channel aftertouch or a CC on another, plain velocity on a drum kit. It also
gives the poly-aftertouch work (MPE part 1, and the live poly-aftertouch out added
2026-10-24) a real way to be played and tested on the hardware, for parity with how
Maschine owners customise their controller.

## What the user decided

- **A pad mode per sound** (MK3 pads only), chosen in the keyboard settings:
  - **Velocity:** the hit sets the note's velocity; pressing a held pad sends nothing.
  - **Poly Aftertouch:** notes play at the project's Fixed Velocity; a held pad's
    pressure goes out as poly aftertouch on its note.
  - **Aftertouch:** notes play at Fixed Velocity; pressure goes out as channel
    aftertouch.
  - **CC:** notes play at Fixed Velocity; pressure goes out as a CC, with its own CC
    number setting.
- In the three pressure modes, notes use the **Fixed Velocity setting that's already
  there** (Maschine's, one per project, on the Keyboard page), whatever the pad
  velocity mode says.
- **Curve, Min and Max** are per sound, not global. They replace the three Preferences ▸
  Hardware sliders added earlier today, which go.
- **External controllers are not affected.** Their aftertouch, CCs and MPE pass through
  as they send them.
- **On the MK3 itself, through a mod.** DECKARD ships its own controller-script mod
  (Lua written fresh by us, spliced into NI's scripts in memory the way any mod is,
  docs/lua-mods.md) that adds the settings to the MK3's Keyboard page. The software
  controller panel runs the same scripts, so it shows them too.
- **Standalone comes next.** DECKARD must later work with no Maschine install; then
  these settings move to DECKARD's own controller layer. Not part of this work.

## How it behaves

- **Settings, per sound:** Mode (Velocity, Poly AT, Aftertouch, CC; default Velocity),
  CC number (0–127, default 1 Mod), Curve (0–100 %, default 50 % straight), Min and Max
  (0–100 %, defaults 0 and 100).
- **Min and Max are the values sent** (my call: the user asked for them "to control the
  modulation depth" in CC mode; the same meaning in every pressure mode keeps one rule).
  A held pad's pressure p (0..1) sends `min + (max − min) · curve(p)` scaled to 0–127,
  where `curve` is today's `pad_aftertouch` shape with Min 0 and Max 1. Max is never
  below Min. A pad pressed lightly sends Min; fully, Max.
- **Only changes are sent:** a held pad sends when its 0–127 value changes (as now).
- **Releasing:** when a pad is let go nothing extra is sent in Poly AT (the note ends).
  In Aftertouch and CC modes, when the last held pad of the sound is let go, the value
  goes back to Min (my call: so the synth doesn't stay bent).
- **Several pads held, Aftertouch or CC mode:** the hardest-pressed held pad of that
  sound wins (my call, how channel aftertouch keyboards behave).
- **Where it goes:** the same places a sound's live notes go. Poly AT follows its note
  out (the 2026-10-24 engine change); Aftertouch and CC go as a controller on the
  sound, sent to its MIDI output and to its instrument, as incoming controllers do.
- **Recording:** Poly AT records onto the notes (as now). Aftertouch records into the
  sound's Aftertouch lane, CC into the lane for its number, overwriting as the playhead
  passes, like an external controller's (the ControlWriter).
- **Note Repeat and the Arp** (my calls, from the user's rule that the pressure modes
  play at Fixed Velocity):
  - Velocity mode: pressure on a held pad sets the repeats' velocity, through the pad
    sensitivity curve (as a hit's velocity is). It is not sent and not recorded: in
    Velocity mode pressure is only velocity.
  - Poly AT, Aftertouch and CC modes: repeats play at Fixed Velocity, and pressure only
    goes out (and records) as that mode's aftertouch or CC.
- **Keyboard mode and pad mode both:** the setting is the sound's, whichever way the
  pads play it (Keyboard mode: the focused sound; pad mode: each pad's own sound).

## On the MK3 (the mod)

- The Keyboard page (`KeyboardModePageMK3`) has two parameter pages today, four knobs
  each on the left screen: Scale and Fixed Velocity. The mod adds two more (my call:
  five settings don't fit four knobs): page 3 **Pad Touch** with Mode, Curve, Min and
  Max; page 4 **Pad Touch CC** with the CC number (it only does something in CC mode).
  They show the focused sound's settings. Turning Min above Max pushes Max up with it,
  and turning Max below Min pushes Min down.
- The mod is an extension mod (manifest + Lua, docs/lua-mods.md) kept in the repo and
  built into DECKARD, applied whenever NI's scripts load. Its Lua asks the focused sound
  for new DECKARD parameters by name (`getPadTouchModeParameter` and so on); the script
  host answers them from the project like its other parameters, with value strings
  ("Poly AT", "CC 1 Mod", "50 %").
- If NI's script changes so the mod can't apply, the app says so in the controller
  status and the pads keep the sounds' saved settings.
- The app learns to load mods (the first item of "Still to do" in docs/lua-mods.md)
  only as far as this built-in one needs; a user Mods folder is not part of this.

## Saving

- Kept in the DECKARD block (Maschine ignores it, so the project still opens there),
  as a new section, payload version 2. Version 1 blocks still read. A sound with all
  defaults isn't written, so projects that don't use this save exactly as before.
- Sounds are identified by group and sound index, like the block's lanes.
- Settings follow a sound when it moves, is copied or is duplicated (as its notes'
  expression does).
- Group and sound files have no block, so these settings aren't in them (my call, as
  with DECKARD's channel pressure).

## Not part of this

- Playing recorded aftertouch back out (part 2 of the MPE work; planned next).
- DECKARD's own controller layer (standalone).
- A user Mods folder.
- Settings for external controllers.

## Left to the builder

- The parameter names and the Lua's exact hook into `KeyboardModePageMK3` (read NI's
  script for where; write our own code).
- The block section's byte layout (document it in docs/deckard-block.md).
- How the MK3 thread learns each sound's settings (today it gets one `PadRoute`).
