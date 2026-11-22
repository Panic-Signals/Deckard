# Replicant scope: a patch preview

**Date:** 2026-11-18
**Status:** proposed

## Why

The current scope taps Replicant's real output, so it is a flat line until a note
sounds, and it mostly shows what the held note is doing rather than what the knobs
do. The point of the display is sound design on the MK3: turn a knob, see the signal
change. It should therefore be a **preview of the patch**, always available, not a
monitor of live audio.

## What it shows

A **short simulated note**, rendered through Replicant's own DSP with the current knob
values, drawn in two parts on the right screen beside Replicant's picture:

- **Top (~90 px):** the waveform of a ~21 ms slice taken 30 % into the held note,
  triggered on a rising zero crossing, one low/high pair per screen column. This is
  where oscillator, mix, sub, noise, pulse width, supersaw, filter type/cutoff/
  resonance and drive are visible.
- **Bottom (~32 px):** the note's amplitude envelope over its whole length, one level
  per column. This is where Attack, Decay, Sustain, Release, Velocity and Volume are
  visible.

The note is A4 (MIDI 69) at velocity 100, one second long: held for 0.6 s, then
released for 0.4 s. Envelope times longer than that are truncated, as on any short
preview. Filter-envelope and LFO effects appear in the waveform slice at that moment
in the note, not as a separate graph.

## How it is made

- `Instance` (not `Replicant`) owns the preview, so it is built from the slot's
  **knob values** in `Instance::values`; modulation writes straight to the engine and
  must not make the picture jitter.
- A preview render makes a fresh engine from the class, applies every knob value,
  plays the A4 note, and keeps the mono signal for one second in `Instance.preview`.
- `Instance.preview` is dropped when a knob value changes (in `Instance::set_param`)
  or a state loads. It is rebuilt lazily on the next `scope()` call, on the UI thread,
  so a knob turn costs one short render (about a millisecond in release).
- Downsampling to columns happens per request from the cached second of audio, so the
  column count is free to change.
- The scope is only offered for Replicant (`Instance.class` is `replicant::CLASS`);
  no other plug-in's screen changes.
- Level scaling stays at 6× (`SCOPE_GAIN`), clamped to ±1, so a single voice at the
  default Volume nearly fills the waveform area.

## Display and pacing

The panel keeps its place: x 728–951, y 68–203 (the right half of the right MK3
screen). A separator divides the waveform from the envelope.

```
┌──────────────────────────┐
│      ╱╲     ╱╲           │  waveform, centre line
│  ╱╲╱  ╲╱╲╱   ╲╱╲         │
│ ───────────────────────  │
│ ▁▂▃▅▆██▓▓▒▒░░            │  envelope: held, then release
└──────────────────────────┘
```

The picture is static between edits, so the 30 ms redraw that was added for the live
scope is removed; knob events already redraw the screen. `focused_replicant` and the
`mk3_wake` condition go away.

## Surface API

`ControlView.scope` becomes:

- `wave: Vec<(f32, f32)>` — low/high per column, −1..1
- `envelope: Vec<f32>` — level per column, 0..1

Lua sees `control.scope.wave` (flat low/high pairs, as now) and `control.scope.env`.

`Plugin::scope`/`Instance::scope` take `&mut self` (they fill the cache) and return
the new `native::Scope` struct.

## Tests

- Preview appears with no note played, and is not flat.
- Two instances with the same settings render identical pictures; a knob change
  changes the picture.
- Envelope reflects a long vs short Attack.
- The waveform still starts on a rising crossing and stays within ±1.
- Surface: both the waveform area and the envelope area receive accent pixels, and
  nothing spills onto the left screen or over the picture.
- Existing plug-in, surface and app suites; one app test (`channel_properties`) is
  already failing on `main` and is out of scope.

## Non-goals

- Live output monitoring, or a switch between live and preview.
- Previews for Plaits, Zhora, the Tuner or effects.
- Showing modulation or the LFO/envelope as separate graphs.
- Changing how the note is rendered in real time.
