# The Song view on the controller

Status: **working** for the MK3 (October 2026). NI's own Arranger page runs unchanged;
DECKARD implements the section calls it makes (`crates/lua-host/src/api/arranger.rs`)
and draws the arranger widget it shows (`crates/lua-host/src/arranger_view.rs`).
Behaviour follows the MK3 manual's "Arranging Sections in the Song View".

## What works

- **Switching views.** ARRANGER shows the Ideas view or the Song view, whichever the
  app shows; SHIFT + IDEAS / SONG (buttons 1 and 2) switch, and the app's song panel
  switches with them (and the other way round). The Song view always has a section in
  focus while there are any.
- **The displays.** The right display shows the sections' names along the top (the
  focused one lit and underlined in its scene's colour), a bar timeline, and a lane per
  group of the focused bank of eight with the patterns each section's scene plays (a
  line where a pattern repeats; muted groups dimmed). The left display shows the whole
  song: a line of scene colours, a thin lane per group, and a frame around what the
  right display shows. The playhead shows while the song plays.
- **Knobs.** 1 POSITION moves the focused section earlier or later; 2 SCENE changes the
  scene it plays; 4 LENGTH makes it a bar longer or shorter (a beat with SHIFT; it then
  stops following its scene's length); 5 zooms and 6 scrolls the arranger.
- **LEFT / RIGHT** focus the previous or next section; the arranger scrolls to show it,
  and the app's Song view picks it too.
- **Pads** pick sections (as SCENE + pad); a pad after the last section adds one playing
  the focused scene. CREATE, DUPLICATE and DELETE work as on the SCENE page.

## Restart and Loop

The header's transport is Restart, Loop, Play, Stop, Record, Metronome. **Restart** plays from the
beginning, playing or not: the song from its first bar in the Song view (whatever section
is picked), the focused scene from its start in the Ideas view. **Loop** makes the song
start over at its end instead of stopping (scenes always loop). On the MK3, RESTART is
Restart and SHIFT + RESTART switches Loop, as in NI's transport script; the app and the
controller share both.

## Tap tempo and the metronome

On the MK3, TAP sets the song's tempo from the last few taps (up to five; a pause of
over two seconds starts over), to a tenth of a BPM, as NI's script asks
(`NHLController:onTapTempo`, `crates/lua-host/src/api/hw.rs`). SHIFT + TAP switches the
metronome, as does the metronome switch after Record in the app's header; both share it.
The metronome is a short synthesized click on every beat while the sequencer runs,
higher on each bar's first beat (`crates/engine/src/metronome.rs`); it isn't saved.

## Pads and the pattern editor

Switching the pads to Keyboard mode (KEYBOARD, or the Step page's button 4) shows the
app's pattern editor as a piano roll; PAD MODE switches it back to the drum grid.

## New groups

As the manual's "Creating Groups": on the controller the dim white group button after
the last group creates a new empty group there and focuses it (SHIFT + group buttons
reach the last bank); in the app, the "+" after the last group (the Ideas view's group
tiles, the Song view's group list) does the same. New groups have 16 empty sounds, the
default colour and the name of their place (Group A2, B2, …); `Document::add_group`
(`crates/doc/src/groups.rs`).

## Not done

- Clips (the CLIP tab) and converting sections to clips; loop ranges over sections.
- The arranger doesn't page along with the playhead (FOLLOW).
