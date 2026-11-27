# MIDI effects (MIDI input plug-ins) — design options

**Status:** Exploratory. Options and recommendations, not an approved build. (2026-11-20)

**Scope:** Ableton-style MIDI effects — transpose a whole sequence, double notes, MIDI
delay, filter, humanize, and similar quick tweaks — applied to a group's notes before they
reach the instrument. This is the "new built-ins in the existing slot" thread.

**Out of scope (separate, bigger feature):** a MIDI-generating plug-in (e.g. VCV Rack)
driving another instrument plug-in (e.g. Replicant) and its parameters. That needs the host
to capture plug-in MIDI *output* — today it passes `outputEvents: null`
(`crates/plugins/src/lib.rs:784`, `:1112`) — and a note source that is another plug-in
rather than the sequencer. See "Relationship to the bigger MIDI feature" at the end.

## Intent and why

The user wants the kind of note-stream tools Ableton exposes as MIDI effects: transpose an
entire sequence in one move, double notes (octaves/intervals), a note-level delay, and small
tweaks to pitch/velocity/timing — things that act on the *notes*, not the audio, so envelopes
retrigger and the change is musical rather than a wet copy of the sound.

## What already exists (the important part)

DECKARD already has the layer these effects belong in: the **MIDI input plug-in** framework.

- `crates/engine/src/midi_input.rs` — the `MidiInput` trait, `NoteIn`/`NoteOut`, `Outbox`,
  `ParamDesc`, the built-in registry.
- `crates/engine/src/note_echo.rs` — the one built-in today, **Note Echo** (a MIDI delay
  with per-echo pitch shift and velocity decay).
- `docs/midi-input-plugins.md` — the framework's own design doc (the native half of
  Feature 15).

How it behaves (from that doc and the code):

- **One plug-in per group**, chosen on the **Group → Input → Plug-in** page. Its parameters
  land on the knobs, in the software and on the MK3.
- **Every note heading for that group's sounds goes in** — pads, keys, pattern notes, Note
  Repeat, the Arp — and **only what the plug-in plays is heard.** It can silence, add, or
  reshape notes on any of the group's sounds.
- **Recording is before it**, so a take holds what was played and re-runs through the plug-in
  on playback.
- Runs **on the audio thread**: nothing may allocate, wait or panic. `Outbox` holds 256
  notes per call (64 reserved for note-offs so a full box can't strand a note).

The trait's hooks:

| Hook | When / for what |
|---|---|
| `note(NoteIn, Clock, Outbox)` | A note came in. Per-note transforms live here. A note with a `length` gets no `Off`. |
| `settle(Clock, Outbox)` | Every note due at this instant has arrived (across all groups). Where chord-aware work goes — a pattern chord arrives one note at a time. |
| `process(Clock, Outbox)` | Once per block, for notes the plug-in plays *by itself* — i.e. a generator/sequencer: play what's due before `now + span`. |
| `stopped` / `finish` | Transport stopped / plug-in removed — end held notes. |
| `params` / `set_param` | Parameters (off-thread description; on-thread value changes). |

**Consequence:** "MIDI effects" is almost entirely a matter of writing new built-ins. The
slot, the timing, the parameter plumbing, the software UI and the MK3 rendering all exist.
A transpose is ~10 lines (the framework doc literally uses `OctaveUp`/`Transpose` as its
example). Register a new one in `BUILT_IN` and `built_in()` in `midi_input.rs`.

## The catalog (options), lowest-effort first

Each is a new `MidiInput` built-in unless noted. "Mechanics" is the hook(s) it needs.

| Effect | What it does | Mechanics | Params | Effort |
|---|---|---|---|---|
| **Transpose** | Shift every note by N semitones | `note()` (mirror `Off`) | Transpose (−24…24 st) | Trivial |
| **Velocity** | Scale / offset / compress / randomize velocity | `note()` | Amount, mode, (seed) | Trivial |
| **Note filter / range** | Pass only notes in a pitch range (or on allowed pitch classes); silence the rest | `note()` silences | Low, High (or a key mask) | Trivial |
| **Octave / double** | Play each note plus a copy at ±12 (or any interval) | `note()` | Interval, −1/+1/+2 octave toggles | Small |
| **Chord** | Play a chord from each single note (fixed intervals, or scale-aware) | `note()` or `settle()` | Chord type, voicing; optional scale | Small–Med |
| **Humanize** | Nudge timing and/or velocity by a small random amount | `note()` + schedule via `clock.seconds`/`process()` | Timing, velocity, seed | Small–Med |
| **Delay (generalized Note Echo)** | What Note Echo does, plus feedback / wider range / swing | `note()` + holds state | Time, repeats, decay, pitch, feedback | Small (have a base) |
| **Chance / randomizer** | Drop notes or alter pitch/velocity with a probability | `note()` | Chance, what varies | Small |
| **Euclid** | Generate a Euclidean pulse on held notes | `process()` + state | Steps, pulses, rotation | Med |
| **Arpeggiator** | Hold a chord, play it out as a pattern | `settle()` to gather the chord + `process()` to schedule | Rate, mode, octaves, gate | Med (**see note**) |

**Scale-awareness** (for Transpose/Chord/Filter): DECKARD has scales
(`crates/model/src/scales.rs`, `scale_tables.rs`). A "diatonic transpose" or scale-snapped
chord is a parameter choice, not new infrastructure — but it is a real design decision (see
Open questions).

**Arpeggiator / Note Repeat overlap:** the framework doc notes that **Note Repeat and the
Arp already run *before* this layer.** A new arpeggiator built-in would duplicate existing
functionality; worth deciding whether to extend the existing Arp instead of adding one here.

## Mechanics a builder needs

- **Stateless pitch/velocity ops** (transpose, velocity, filter): implement `note()` only,
  and mirror the transform in the `NoteIn::Off` arm. Respect `length`: a note carrying a
  `length` gets no separate `Off`, so don't emit one.
- **Chord-aware ops** (double-within-a-chord, strum, chord-from-note across a played chord):
  use `settle()`, which fires once the whole instant's chord has arrived.
- **Generators** (delay, arp, euclid): keep state, emit from `process()` by looking ahead to
  `clock.now + clock.span`, and own your note-offs (play with `length: None`, end them
  yourself). `Repeater::due_into` is the reference pattern.
- **Parameters**: describe them in `params()` (off-thread, may allocate); apply in
  `set_param()` (on-thread — no alloc/wait/panic). Values arrive already fitted to range and
  steps.
- **Register**: add the name to `BUILT_IN` and construct it in `built_in()` in
  `midi_input.rs`.

## Constraints and current limits (from the existing design)

These are real and shape what "MIDI effects" can be today:

1. **One slot per group.** There is exactly one MIDI input plug-in per group, so you cannot
   stack *Transpose → Delay* at once. This is the single biggest limitation for an
   Ableton-style "rack of MIDI effects." Options:
   - (a) ship combined built-ins (e.g. a plug-in that does transpose + velocity);
   - (b) extend the slot into a small ordered chain of `MidiInput` plug-ins (bigger change
     to `Output`'s per-group state and the UI).
2. **Session-only, not saved.** The framework doc's "What's left for later" lists saving a
   plug-in (and its params) in the project. Today, opening/creating a project removes them.
   For musical use this likely needs to be addressed.
3. **Slots are by group index.** Moving or deleting a group requires whoever placed the
   plug-in to move it (`App` already tracks this for group deletion).
4. **Per-note expression doesn't pass through**, and these plug-ins **aren't in exported
   audio** (export builds its engine separately).
5. **Audio-thread rules** apply to all of it: no allocation, waiting, or panicking.

## Open design questions (the real forks)

1. **One slot vs. a chain.** Do we want to stack MIDI effects (Ableton-style), or is one
   per group enough? This decides whether this is "add built-ins" or "add built-ins + rework
   the slot."
2. **Scale-aware or chromatic.** Should transpose/chord snap to the project scale, or stay
   chromatic, or offer both?
3. **Persistence.** Save these in the project (and modulate them), or keep them
   session-only as today?
4. **Scope of "a sequence."** The slot is per *group*. "Transpose this sequence" might want
   per-sound, per-pattern, or per-clip scope instead — a different granularity than the slot
   offers.
5. **Maschine compatibility.** Maschine has no user MIDI-effect chain, so anything here is
   DECKARD-only and belongs in the manual's "differences from Maschine" chapter (subject to
   the public-docs rules: no reverse-engineering references).

## Recommended first slice

If/when this is built, the cheapest proof of the pattern:

- **Transpose, Velocity, Note Filter**, and a **generalized Delay** (building on Note Echo).
- All use `note()`/`settle()` only — **no new UI**, since the Plug-in page already renders
  parameters on the knobs and MK3.
- Defer stacking, persistence, and the arpeggiator (overlaps the existing Arp).

This exercises the whole catalog pattern (register → params → note transform) at minimal
cost, and surfaces the "one slot vs. chain" and "persistence" questions with something real
to try against.

## Relationship to the bigger MIDI feature

The separate, larger ask — a plug-in that *generates* MIDI (VCV Rack) driving another
instrument (Replicant) and controlling its parameters — shares one piece of groundwork with
this: it needs the host to stop discarding plug-in MIDI output (`outputEvents: null` at
`crates/plugins/src/lib.rs:784`, `:1112`). Beyond that it is a different feature: an
instrument's note source is currently always the sequencer/pads (`NoteIn`), never another
plug-in, so routing one plug-in's MIDI (and CC → parameters, via the existing
`crates/doc/src/cc_routes.rs` + `crates/model/src/mod_matrix.rs`) into another is new routing
model work, not a MIDI-effect built-in. The lightest bridge is to wrap a MIDI-only generator
*as* a `MidiInput` plug-in using its `process()` hook. That belongs in its own spec.
