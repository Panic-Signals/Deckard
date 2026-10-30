# Phase 2b — Note Repeat and the arpeggiator (design and plan)

> Roadmap: `docs/superpowers/plans/2026-10-22-parity-roadmap.md`. Inline, test-first.
> Sources: MK3 manual "Note Repeat and Arp" (pp. 188–195), NI's RepeatPage scripts, the
> song's `arpeggiator` in Maschine's projects (40 sampled).

## What Maschine stores (one per project, `Song::arpeggiator`)
| field | meaning | evidence |
|---|---|---|
| `type` | Up, Down, Up & Down, Order Played, Chord | manual's order; 3 seen with a non-default arp |
| `pc0` | rate, in ticks (960 a quarter): 240 = 1/16, 480 = 1/8, 320 = 1/8T, 160 = 1/16T, 60 = 1/64 | values across projects |
| `sequence` | the rhythm (0 Off, 1–8) | |
| `octaves` | 0 = one octave … 7 = eight | manual: up to eight |
| `dynamic` | velocity factor 0.01–2 (1 = 100%) | |
| `gate` | length as a factor of the rate 0–2 (1 = 100%) | |
| `hold`, `retrigger`, `repeat`, `offset` (8 = 0, ±8), `inversion`, `min_key`, `max_key` | page 2 | |
| `pa8`, `pb0`, `pb8` (always 12), `pd8` | not yet known; kept as read | |
The four presets (`arp_presets`) are kept as read; DECKARD's presets are the MK3's
buttons 5–8 rates (1/4, 1/8, 1/16, 1/32 by default).

## Behaviour
- **Note Repeat** (pads in group mode): a held pad repeats at the rate, each note
  `gate × rate` long at the pad's velocity (× dynamic).
- **Arp** (keyboard / chord mode): the held pitches (within min–max key) play one at a
  time in `type` order, over `octaves`, with retrigger / repeat / offset / inversion as
  the manual describes; Chord plays them together. `sequence` 1–8 puts rests in the
  cycle (DECKARD's own eight rhythms: Maschine's aren't documented).
- In time with the song (the next rate boundary of the song position); stopped, on its
  own clock from the first pad. **Hold** latches; **LOCK** keeps it on outside the page.
- Its notes play like pads and **record** (REC) as they play.
- Only the controller's pads (and the computer keyboard) go through it, not MIDI input
  (as Maschine).

## How
- **Model / document:** `Project::arp` (`Arp`); edits through `Target::Song` paths
  `arpeggiator/<field>`.
- **Engine:** `Repeater` between live input and sounds: `Command::Repeat(Option<Arp>,
  Mode)`; held live notes (`NoteOn` with `i64::MAX`) feed it instead of sounding, their
  releases leave it; it fires notes with lengths at rate boundaries (song frames when
  playing, its own clock otherwise), recorded as pad hits.
- **Pads held:** the controller reports pad releases, so pads hold notes
  (`Request::PadOn/PadOff`) instead of fixed-length hits.
- **MK3:** `NI.DATA.getArpeggiator(App)` (its parameters), `getArpeggiatorRateParameter`,
  `…RateUnitParameter`, `…PresetParameter`, `NI.HW.getArpeggiatorActiveParameter`,
  `SongAccess.isArpRepeatLocked`; the NOTE REPEAT page runs on them; holding NOTE
  REPEAT (or LOCK) switches the repeater on.
- **Software:** a Note Repeat / Arp panel (View ▸ Note Repeat / Arp): on, lock, hold,
  rate buttons, the page's knobs; ⌘K's keys go through it like pads.

## Steps (test-first)
1. Model: read `arpeggiator` (Cloud 9: defaults; another project: type 3, gate 0.7).
2. Engine repeater: note repeat timing and gate; stopped clock; release ends it.
3. Arp orders: Up/Down/Up & Down/Order Played/Chord, octaves, min/max key.
4. Arp advanced: retrigger, repeat, offset, inversion; sequences' rests; dynamic.
5. Recording: repeated notes record.
6. Pads held: controller pad releases; app holds.
7. MK3 NOTE REPEAT page: parameters, presets, hold, lock, gate reset; sweep clean.
8. Software panel. Docs; full check.
