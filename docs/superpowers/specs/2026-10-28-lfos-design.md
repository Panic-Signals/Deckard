# LFOs and the MK3 mod matrix: design

Status: proposed, 2026-10-28. Builds on the modulation matrix
(`crates/model/src/mod_matrix.rs`, `docs/superpowers/specs/2026-10-24-expression-out-design.md`
part 2b) and the DECKARD block (`docs/deckard-block.md`).

## Why

The user wants movement DECKARD can't make today: for example, a stepped, sample-and-hold
style wave pumping a synth's distortion mix. The mod matrix can already drive any
instrument, effect or plug-in parameter, but its only sources are a sound's notes'
Pressure, Slide and Pitch. This adds up to 16 LFOs per project as new sources.

The user's workflow, in their words: go to the LFO page, create a quantized sine LFO,
then go to the mod matrix and assign it to the distortion mix.

## Goals

- Up to 16 LFOs per project, one per pad. A new project has none.
- Each LFO is a project-wide source that shows up in every sound's mod matrix as
  **LFO 1 … LFO 16** (only the ones that exist), so one LFO can drive parameters on
  several sounds.
- Routes can be assigned on the MK3 too, on a new mod matrix page (SHIFT + SOLO).
- Edited on the MK3 (SHIFT + SELECT) and in the software (a new LFO button next to the
  Mixer button), with a picture of the wave that moves as settings change.
- Saved with the project in the DECKARD block, so Maschine still opens the project.

## Non-goals

- Maschine running the LFOs. Maschine opens a project with LFOs and plays it without
  them. **If the project is re-saved in Maschine, the LFOs and their routes are dropped.**
  The user accepted this, as with Pad Touch and MPE today.
- Per-voice LFOs (each note its own LFO). An LFO is one value at a time for the whole
  project.
- LFOs modulating other LFOs' settings.
- More than 16.

## Anti-goals

- A synced LFO that drifts against the song, or that plays back differently from
  run to run. It should line up with the bar every time, live and in an export.
- Breaking a project in Maschine. The project must still open and play there.

## The LFO

Each LFO has these settings (defaults in brackets; *my call* marks choices the user
didn't make but saw and accepted in the mockups or playback):

| Setting | Values | Notes |
|---|---|---|
| Shape [Sine] | Square, Sine, Triangle, Saw, Random | Random: a new random value at the start of each cycle, held until the next (*my call*). |
| Sync [on] | on / off | On: Rate is a note value locked to the tempo. Off: Rate is in Hz. |
| Rate [1/4] | Synced: 4 bars, 2 bars, 1/1, 1/2, 1/4, 1/8, 1/16, 1/32, with dotted and triplet variants (*my call*). Free: 0.01–20 Hz (*my call*). | One cycle per note value. |
| Phase [0°] | 0–360° | Slides the wave along time. The picture moves with it. |
| Level [100%] | 0–100% | How much of the wave comes out. |
| Quantize [Off] | Off, 2–32 steps per cycle (*my call* for the range) | Holds the wave's value for each step, so a sine becomes a staircase. |
| Trigger [Off] | Off, or a sound (group + sound, listed as "A1 / S3 – name", groups lettered as elsewhere) | See below. |
| Fade in [1 beat] | 0–16 beats (*my call*: in beats so it follows the tempo) | Shown only when Trigger is set. |

**Output.** An LFO's output is 0..1: `wave × level × fade`. The mod matrix route maps
that 0..1 onto its own low/high range of the knob's travel, as it does for Pressure.
So 0 output sits at the route's *low*. Level and the route's range both stay: level is
the LFO's strength everywhere it's used, and each route has its own range
(*my call*, played back to the user).

**Free running (Trigger Off).** A synced LFO's phase comes from the song position:
while the transport plays, cycle position = song position in its note value + phase. It
lines up with the bar and is the same on every playback and export. While the transport
is stopped, it keeps running at the tempo from where it was (*my call*). A Hz LFO
restarts at its phase when the transport starts, so it also matches between playback
and export (*my call*). Outside the Arrangement's playback, the ticks and seconds are counted as they
play, each at the tempo then, so a tempo change never jumps an LFO's phase; a count-in,
or waiting for a clock's bar, doesn't count (the song starts at zero after it).

**What "song position" means** (*my call*):
- In the Arrangement, it's the arrangement's position. An export of part of the song
  (a range of sections) passes in that range's offset in the full song, so the LFO
  moves exactly as it does in the full song.
- In Ideas (scenes and patterns looping), it's the time since play started, so a
  4-bar LFO carries across a 1-bar loop instead of resetting every bar. An export of a
  single scene or pattern starts from zero.

**Update rate** (*my call*): LFOs are worked out once per engine span, like every other
route source, with no smoothing, so steps stay sharp. Spans are short enough that this
sounds continuous for smooth shapes.

**Triggered (Trigger set to a sound).** Every note-on that sound plays restarts the LFO:
- from a pattern, or played live on the pads, MK3, Launchpad or a MIDI keyboard. Every
  note the sound plays counts (the user's words: "notes we played live too on that sound
  as well").
- The wave restarts at its phase point, the fade drops to zero, and it ramps back up to
  full level over the fade time.
- Only notes that actually sound count. A muted sound's notes don't trigger (the user
  decided this). Neither do notes kept silent by solo, or notes a probability setting
  skips (*my call*, following the user's answer on mutes). Hook the trigger in after
  mute, solo and probability are decided.
- A new note restarts it again even if the last fade hasn't finished.
- Before the sound's first note, a triggered LFO outputs 0 (*my call*).

## MK3 page

SHIFT + SELECT opens the LFO page. It stays open like MIX (not held), and SHIFT + SELECT
or another page button leaves it (*my call*). SELECT is a hold button today
(`HOLD` in `crates/surface/lua/global.lua`), so SHIFT + SELECT has to be caught before
the hold logic. SELECT without SHIFT stays as it is.

- **Left screen:** the selected LFO's name, **"LFO 1"**, large and centered, filling most
  of the screen (the user asked for this). With no LFO yet: a short hint to press a pad.
- **Right screen:** the selected LFO's wave over two cycles, drawn with phase, level,
  quantize and (when triggered) the fade-in ramp, plus a small header (shape, rate,
  "trig").
- **Display buttons:** 1–5 the shapes (Sine, Square, Triangle, Saw, Random), 6 SYNC
  (lit: synced; off: Hz), 7 ENV (Envelope mode: `2026-10-29-lfo-envelope-design.md`),
  8 TRIG (SHIFT + 8: DELETE, removing the LFO). TRIG on sets the trigger to the focused
  sound; off sets it to Off (*my call*).
- **Knobs:** 1 Shape, 2 Rate, 3 Phase, 4 Level, 5 Quantize, 6 Trigger (steps through the
  project's sounds, then Off), 7 Fade in, 8 unused.
- **Pads:** on this page the pads pick LFOs instead of playing. Lit means the pad has an
  LFO, and the selected one is brighter. An empty pad creates that LFO with the defaults
  and selects it. **ERASE + pad** removes the LFO (*my call*; it matches ERASE + knob,
  and the user saw it in the mockup). **MUTE + pad** mutes or unmutes that LFO (a muted
  pad is lit red; its name shows MUTED).

## Software

- A button next to the Mixer button (`apps/viewer/src/layout.rs`, the "Show or hide the
  Mixer" `icon_button`). Its icon is a circle with a wave in it. It shows or hides the
  LFO panel, as the Mixer button does with the Mixer.
- **Left:** a list of the LFOs that exist (LFO 1, LFO 2, …) with the selected one marked,
  and "+ Add LFO" (it takes the lowest free number). Right-click an LFO → Remove
  (*my call*).
- **Right:** the wave picture, then a control for each setting in the table above. The
  picture redraws as any control moves.
- The mockup the user approved is in
  `.superpowers/brainstorm/98810-1793225271/content/lfo-page.html`.

## Mod matrix

- `ModSource` gets an LFO variant (LFO 1–16). The source picker
  (`apps/viewer/src/layout/mod_routes.rs`) lists the LFOs that exist after Pressure,
  Slide and Pitch.
- Saved in the routes' text as `lfo1` … `lfo16`. Older DECKARD builds skip those lines,
  as `from_text` already skips lines it can't read.
- A route whose LFO is removed stays and does nothing. It comes back if that LFO is made
  again (*my call*). It shows as "LFO 3 (none)".
## MK3 mod matrix page (SHIFT + SOLO)

Today mod routes can only be edited in the software. The user wants to finish the whole
workflow on the hardware, so this project adds a mod matrix page. The layout is the
user's, modeled on the Hydrasynth's. The approved mockup is
`.superpowers/brainstorm/98810-1793225271/content/mod-page-v2.html`.

- SHIFT + SOLO opens it. SOLO is a hold button today, like SELECT, so SHIFT has to be
  caught first. It opens and closes like the LFO page.
- **Eight mod slots per page, one per column** (four on each screen). LEFT / RIGHT go to
  the next or previous eight. After the last route there is always one empty slot to
  start a new route.
- **Each column, top to bottom:** Source, Group, Sound, Module, Destination, Depth.
  - Source: None, Pressure, Slide, Pitch, then each LFO that exists.
  - Group and Sound narrow down the sound being modulated.
  - Module is a slot of that sound's chain (instrument, effect or plug-in).
  - Destination is one of that module's modulatable parameters.
  - Depth is −100% to +100%. The user's layout had no amount row; Depth was added in the
    mockup and the user approved it.
- **The display button above a column** moves its highlight down one row and wraps back
  to the top. **The knob below a column** changes the highlighted row.
- ERASE + a knob removes that slot's route (*my call*, matching ERASE + knob elsewhere).

Mapping onto the stored routes (*my calls*):
- The slots are every route in the project, ordered by group, then sound, then the
  route's order on the sound. The data stays per sound as it is now; this page is a view
  over all of it.
- A slot becomes a real route only once it has a Destination. Until then it exists only
  on screen while the page is open.
- Changing Group, Sound, Module or Destination on an existing slot moves the route to
  the new target. If the target already has a route, the new one replaces it, as in the
  software (one route per parameter).
- Depth is `high − low`. A route made here starts with Low at the destination knob's
  current value. Turning Depth moves High with Low fixed (clamped to 0..1), so a
  negative Depth inverts. Curve stays at its default and is edited in the software.

## Mod matrix: trigger fix-ups

- Removing a sound that is an LFO's trigger sets that trigger to Off. Moving or
  deleting groups or sounds renumbers the trigger so it follows its sound, as other
  cross-references do, in the same undo step (*my call*).

## Saving

- The LFOs are a new section at the end of the DECKARD block payload, version 6
  (version 5 plus the LFO section). Version 6 must read versions 1–5, and
  `docs/deckard-block.md` documents the section.
- The block is written only when it holds something, as today. A project whose only
  DECKARD content is an LFO still gets the block.
- In the document, LFO edits are undoable like route edits. A knob drag is one undo step,
  and creating or removing an LFO is a step of its own.

## Engine

- The LFOs are worked out each span, before `apply_routes`
  (`crates/engine/src/mod_rt.rs`), and feed routes the same way the expression sources
  do. Quantize is applied to the wave's time, and fades follow note-ons.
- Note-ons from every path (pattern playback, live pads, MIDI in) reach the triggers.
- Sounds with an LFO route are modulated sounds, so they render on the audio thread, as
  routed sounds do today.
- Exports render the same LFO movement as playback.

## Left to the builder

- How the LFOs live in the model and document. Following the mod routes' pattern (a
  DECKARD text field, here on the project's root object, carried by undo) is the
  obvious choice.
- The exact binary layout of the block section.
- How Random's values are seeded, as long as playback and export match.
- Visual details of the icon, the panel and the MK3 drawing, beyond the layout above.
- Tests, following the project's existing style (round trips for the text and block,
  engine tests for wave shape, phase, quantize, sync against song position, and
  trigger/fade).
