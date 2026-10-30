# Modulation

Status: **working** for Maschine's own effects, October 2026 (plan:
`docs/superpowers/plans/2026-10-22-phase2c-modulation.md`, which has the findings).

## What it does
- **Playback.** A pattern's modulation moves its sound's or group's effect parameters
  while it plays.
  - Points are joined by lines. Before the first point the first value holds, and after
    the last the last.
  - A knob's point is a deviation from the knob's own value, so turning the knob shifts
    the whole movement. A selector's or button's point is the value itself.
  - When the pattern starts again, or the song stops, the parameter is back to its own
    value.
- **Recording in the software.**
  - Drag a knob's ring while the song plays.
  - Or switch on Auto-write (the header's bullseye, red while on; the MK3's AUTO),
    and then any knob drag records.
  - Touch writing, as the controller lanes: while a knob is touched (moving, or within
    250 ms of its last move) its modulation is overwritten as the song passes, at its
    last value when held still, over the pattern's end too; let go or idle, what was
    there plays on.
  - Each touch is one undo step, thinned to few points when let go.
  - ⌃-click (right-click) anywhere on a knob: Remove Modulation removes its modulation
    from every pattern (a touch still open on it ends first, so it isn't written back).
  - A modulated knob shows a mark on its ring where the modulation has it now.
- **Recording on the MK3.**
  - Press **AUTO** to turn Auto-write on (pressed again, off) and turn knobs 1–8 in
    Control mode. The knobs show the deviation in percent. On DECKARD's own layer AUTO
    is a toggle; with Maschine's scripts it writes while held, SHIFT + AUTO pinning it
    on, as Maschine's.
  - **ERASE** + a knob removes that knob's modulation.
  - **SHIFT + pad 10** (Clear Auto) removes the focused sound's modulation in the
    focused pattern.
- **LFOs as a source.** Up to 16 LFOs, one a pad (LFO 1 … LFO 16), each a route
  source after Pressure, Slide and Pitch: Shape (Sine, Square, Triangle, Saw, Random),
  Sync and Rate, Phase, Level, Quantize, Trigger (a sound whose notes restart it) and
  Fade in. A route maps the LFO's 0..1 onto its low..high as it does Pressure. Spec:
  `docs/superpowers/specs/2026-10-28-lfos-design.md`.
- **Envelopes.** Any LFO can be switched to Envelope mode (named ENV n everywhere): an
  ADSR its trigger's notes play, from patterns and live, as a route source like an LFO
  (mod matrix and CC routes alike). Attack rises 0 → 1, Decay falls to Sustain, it
  holds while any note of the trigger sound is held, and Release falls to 0 after the
  last one ends (a pattern note's length, a pad or key let go, a choke, or stop). A, D
  and R (1 ms–10 s) are each the time a full 0↔1 move takes; segments are linear.
  Retrigger on: each note restarts the attack, rising from where it is; off (legato):
  only a note with none held restarts it. Level scales it, Quantize steps its height
  (N steps: N heights, 0 and full among them),
  Mute silences it; one whose trigger sound is gone gives nothing. Each mode keeps its
  own settings across switches; Trigger, Level, Quantize and Mute are shared. MK3:
  display button 7 ENV on the LFO page. Spec:
  `docs/superpowers/specs/2026-10-29-lfo-envelope-design.md`.
- **A routed knob moves.** While a route (a note's expression or an LFO) drives a
  built-in knob (an effect's, the Sampler's, a DECKARD synth's), the knob shows where
  the route has it, position and value, in the Control area, the strip and on the MK3,
  as a plug-in's knob does. Turning or dragging it changes its own value, from where
  that's set (not from where the route shows it).
  With nothing driving it (route gone, LFO muted or missing), it shows its own value;
  Pressure at rest shows Low's. The engine publishes each route's travel each span
  (`meters::RouteTravels`, lock-free; `Output::route_travel`), 32 routes a sound,
  each keyed by its slot and parameter so a knob never reads another route's travel.
- **LFOs on the MK3: SHIFT + SELECT.** The pads pick LFOs (an empty pad makes one,
  ERASE + pad removes one, MUTE + pad mutes or unmutes one: lit red). The left screen shows the focused LFO's name, the right its
  wave. Display buttons 1–5 pick the shape, 6 SYNC, 8 TRIG (the focused sound, or Off).
  Knobs 1–7: Shape, Rate, Phase, Level, Quantize, Trigger, Fade in. SHIFT + SELECT
  again goes back.
- **The mod matrix on the MK3: SHIFT + SOLO.** Every route in the project, eight a page
  (LEFT / RIGHT for more), then an empty slot for a new one. Each column is a slot:
  Source, Group, Sound, Module, Destination, Depth. The display button above a column
  moves its highlight down a row (wrapping), the knob below turns that row, and ERASE +
  the knob removes the route. A slot becomes a route once it has a Destination; one left
  without is dropped when the page goes. Destination passes over a parameter another
  route on the sound already drives.
- **MIDI CC as a destination.** An LFO can be sent out as a MIDI CC: a CC route has a
  Source (an LFO), a Port (a MIDI output port), a Channel (1–16), a CC (0–119: 120–127
  are channel mode messages, such as All Notes Off, not parameters) and a Depth. It sends `round(127 × travel)`, travel mapping the LFO's 0..1 onto low..high as
  any route does (curve 50), and only when that number changes. A CC just chosen (or a route just added) waits
  until it has stayed 0.25 s before anything is sent to it, so turning through CC
  numbers sends nothing to those passed. A muted or removed LFO
  sends nothing more (no reset value); a route at Depth 0, as a new one is, sends
  nothing; a port that isn't open drops it; exports send no MIDI. CC routes are the
  project's, not a sound's (`deckard:cc_routes`, saved in the DECKARD block, version 7).
  - **MK3 mod matrix:** turning Group past the last group selects **MIDI**, and rows 3–5
    read Port, Channel and CC. Source then steps over the LFOs only. The slot becomes a
    CC route on its first CC step (the first output port and channel 1 if not chosen);
    a sound's route turned to MIDI moves there then, keeping its Depth. Group turned
    back from MIDI is the last group again, a draft until it has a Destination. Slots
    list the sounds' routes, then the CC routes, each keeping its column; ERASE + the
    knob removes one.
  - **Software:** the LFO panel's **MIDI CC** section, under the LFO's controls, lists
    the CC routes (Source, Port, Channel, CC, Depth, × to remove). "+ Add CC" adds one:
    the first LFO and port, channel 1, CC 1, Depth 0 (off with no LFOs or no ports).
- **Saving.** Projects keep the format they have. A project with Maschine 2's points
  gets v0 points; a new one gets Maschine 3's v1.

## How
- **Model:** `Sound::modulators` / `Group::modulators` (`ModTarget`: chain slot and
  field) and `Pattern::mods` (`ModPoint`). Each parameter object is mapped back to
  (channel, slot, field).
- **Document** (`crates/doc/src/modulation.rs`): `set_modulation` (adds the lane if
  missing; replaces a span's points), `clear_modulation` (the lane and every pattern's
  points; macros stay) and `clear_pattern_modulation`.
- **Engine** (`crates/engine/src/modscale.rs`):
  - Timelines carry `ModLane`s already at stored values. The app registers each knob's
    `Scale` (from its pages), and points are mapped through it when timelines are made.
  - Each span, effects with a lane playing take its value through their glide, and get
    their own value back when it stops.
  - Modulated sounds render on the audio thread rather than ahead.
- **App** (`apps/viewer/src/modulation.rs`, `App::modulate_at`): deviations are written
  at the play position into the group's playing pattern (`record::pattern_at`) inside a
  document take. They are thinned (`modulation::thin`, 1/200 of the travel) at the end.
- **MK3:** `NI.DATA.WORKSPACE.isAutoWriteEnabledFromHW(Model)`, `setAutoWriteEnabled`,
  `EventPatternTools.removeModulationEvents`, and `ControlRequest::Erase`.

## Not yet
- The Control Lane's modulation pane: listing lanes, and seeing, drawing and editing
  points.
- Step mode modulation: holding steps and turning knobs
  (`ModulationEditingAccess.setModulationStep`).
- Instruments' (drum synths, Bass/Poly Synth), the Sampler's and plug-ins' parameters;
  channel properties.
- Curved segments (v1's other coefficients are read as a straight line).
- Hiding non-modulatable knobs on the MK3 while auto-writing (`Parameter:isAutomatable`).
