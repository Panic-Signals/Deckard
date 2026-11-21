# Layers and Morph (sampler roadmap, phase 4, first part)

Brainstormed 2026-11-20 and 21. What the user said is marked *theirs*; every other
choice is *my call*.

## Why

*Theirs:* an intense sound-design tool in the mould of the Ensoniq EPS-16+. You bring in
external samples, one per layer, give each its own envelope, and the layers fade in and
out of each other so the whole sound becomes one slow, evolving loop. The finished sound
plays across the keyboard like a normal instrument. The model is the "morphing
soundscape" in the user's own EPS-16+ tool: up to 8 layers, each with a crossfade window,
the windows spread evenly, one LFO sweeping through them.

## What they decided

- Up to 8 layers a sound, each an external sample dropped on its lane.
- Each layer has its own envelope, edited on a LAYERS page on the MK3 too.
- **One sweep, eight windows** (not eight independent looping envelopes): a single MORPH
  position moves through the layers.
- The sound is **live**: it plays across the keys with the morph running, and can still
  be edited. Baking it into one sample (**RENDER**) comes later.
- Stems to layers isn't needed.
- The mock-up (https://claude.ai/artifact/F3Tr6F78VkbgSpKFKH5Uva, its Main and MK3
  boards) "looked great". It was drawn before the morph was talked about, so: its look,
  its lanes with each layer's envelope drawn over the waveform, and its MK3 controls are
  kept; where this text and the mock-up differ on what a setting is or whether it
  exists, this text wins. Its two stems boards are dropped.

## Goals

- Drop a sample on a layer and hear it at once, in tune with the others, on every key.
- A morph that runs by itself from the moment it's switched on, slow enough to last
  minutes, in time with the song if wanted.
- Everything reachable from the MK3 except dropping files.

## Not in this part

Velocity layers, round-robin and random layer play, Patch Select, key-up and legato
layers, layer delay, voice bins, Simultaneous Play, the auto-sampler (all still phase 4,
later); the mock-up's per-layer FILTER, VELOCITY, DELAY and envelope TYPE, its
"A NOTE PLAYS" chooser and dragging a sound from the list; a layer of several zones (a multisample per layer); Sync or Keys on a layer;
stems on a layer; RENDER; Transwaves (phase 5).

## Anti-goals

- A layered sound that needs wiring in the mod matrix before it moves.
- Clicks when a layer's window opens or closes, or when MORPH is turned by hand.
- Layers that drift out of tune with each other because their samples' roots differ.

## Approaches

1. **A layer is a zone (chosen).** The sampler already starts a voice for every zone a
   note falls in. A layer is a New Sampler zone over the sound's keys with a level, an
   envelope and a morph window. It keeps all a zone has (start, end, loop, tune, root)
   and the EDIT tab edits it as it is. Maschine opens it as overlapping zones.
2. **Layers inside one voice, as stem lanes are.** Cheaper, but lanes are one length and
   read at one place; layers that loop at their own lengths are the point.
3. **Eight sounds and a group morph.** No engine work, but eight pads and no one
   instrument.

Chosen by me, the how being left to me; told to the user before this was written.

## What it adds

All of it is the New Sampler's. A plain Sampler shows none of it.

### A layered sound

A New Sampler sound is *layered* when any of its zones is a layer. Its layers are its
layer zones in zone order, at most 8. A layered sound has no slices: SLICE is hidden for
it, MOTION is off, and a sound with slices offers no LAYERS tab until its slices are
gone (one zone left). A sound with one plain zone becomes layered when LAYERS is first
used: that zone becomes layer 1.

Every layer zone covers the sound's **KEYS** (low to high, default the whole keyboard)
and all velocities. Setting KEYS sets every layer's range. This is all that's left of
"apply": the sound is playable as it's built, so there is no APPLY step.

A layer plays as a Repitch zone does: the key's pitch from the zone's own root key and
tune, so layers with different roots sound the same note. LEVEL, PAN, TUNE and ROOT are
the zone's own fields (gain, pan, tune, root key), so Maschine hears them too. A dropped
sample is analysed as any New Sampler sample is; a root found (a guess too) goes in as
the zone's root key, and its cents into the zone's tune. With no root found it is C3.

The first use of LAYERS on a sound whose amp envelope is One-shot switches it to ADSR
(as applying slices does), so notes end and a layer's RELEASE has something to follow.

### A layer's settings

| Setting | Range | Default |
|---------|-------|---------|
| **LEVEL** | −inf to +12 dB | 0 dB |
| **PAN** | L to R | centre |
| **TUNE** | ±36 semitones, as a zone's | 0 |
| **M**, **S** | mute, solo (as stem lanes') | off |
| **ATTACK** | 0 to 30 s | 5 ms |
| **DECAY** | 0 to 30 s | 0 |
| **SUSTAIN** | 0 to 100 % | 100 % |
| **RELEASE** | 0 to 30 s | 50 ms |
| **Window** | four points on MORPH, 0 to 1 | spread evenly |

The layer's envelope is on top of the sound's own amp envelope (which still shapes
everything), so a layer can come in late or slowly within a note. Start, end and loop
are the zone's, on EDIT with the layer focused.

### Morph

The sound has (saved with the New Sampler's own settings):

| Setting | Values | Default |
|---------|--------|---------|
| **MORPH** | Off, On | Off |
| **POSITION** | 0 to 100 % | 0 |
| **RATE** | free: 500 s to 0.05 s a cycle; **SYNC**: 1/16 to 64 bars a cycle | 8 bars, SYNC on |
| **SHAPE** | Round, There and back | Round |
| **DEPTH** | 0 to 100 % | 100 % |
| **RESET** | Off, On | On |
| **FADE** | Linear, Equal power | Linear |

- **Off:** every layer plays at its level: a plain stack. Windows do nothing.
- **On:** each layer's loudness is also its window at the morph position *p*.
- *p* = POSITION + DEPTH × sweep + modulation. The sweep goes 0 to 1 once a cycle.
  Round: *p* wraps (past the last layer comes the first). There and back: the sweep goes
  up then down in one cycle and *p* is held within 0 to 1. DEPTH is There and back's
  only: Round always goes the whole way round (anything less would jump once a cycle),
  and shows DEPTH as "-".
- **RESET** on: a note starts its own sweep from 0 (each note morphs from its start, as
  the EPS does with LFO reset). Off: one sweep for the sound, all notes together; with
  SYNC and the song running it is the song's position, so it's the same every pass.
- A window is four points A ≤ B ≤ C ≤ D: silent before A, rising to full at B, full to C,
  falling to silent at D. FADE chooses straight lines or equal power (square root of
  them).
- **Spread evenly** (the default, and a **SPREAD** button to go back to it), for *n*
  layers, layer *i* from 0:
  - Round: centre *i/n*, A = centre − 1/n, B = C = centre, D = centre + 1/n, wrapping
    round 0 and 1. Two neighbours always add to 1.
  - There and back: centre *i/(n−1)*, the same with 1/(n−1), nothing wrapping; the first
    is full from 0 and the last to 1. One layer: full everywhere.
  - Adding, removing or reordering a layer spreads again while the windows are as SPREAD
    left them. Once one has been dragged, they stay as they are and a new layer gets the
    last layer's window.
- **MORPH position is a modulation target** of the Sampler ("morph", in the mod matrix's
  list for a New Sampler sound with layers): an LFO (in envelope mode too), pressure or
  slide moves POSITION, in a route's Range mode as in Add. Modulation is the sound's, as
  all Sampler modulation is: with RESET on, a route still moves every note together.
  The morph lives outside the Sampler's fields, so this is a new target path beside
  `MODULATED`.
- Changes to *p*, a level, a mute are ramped (about 5 ms, as lane levels are) so nothing
  clicks.

### In the software: the LAYERS tab

A fifth tab, **LAYERS**, after ZONE, for a New Sampler sound with a sample or none (a new
New Sampler opens on it empty beside RECORD). As the mock-up's Main board:

```
RECORD  EDIT  SLICE  ZONE  [LAYERS]                      KEYS C-2 – G8
┌ MORPH ───────────────────────────────────────────────────────────┐
│ /\    /\    /\    /\         windows, one colour a layer         │
│/  \  /  \  /  \  /  \        | position line moves as it plays   │
└──────────────────────────────────────────────────────────────────┘
1 Pad Warm     M S  [waveform ~~~~~~~~~~~~~]   0.0 dB
2 Choir Ah     M S  [waveform ~~~~~~~~~~~~~]  -3.0 dB
3 Glass        M S  [waveform ~~~~~~~~~~~~~]   0.0 dB
4 (drop a sample here)
┌ LAYER 2 ──────────────┐ ┌ ENVELOPE ─────────┐ ┌ MORPH ───────────────────────┐
│ LEVEL PAN TUNE ROOT   │ │ A  D  S  R  shape │ │ ON POSITION RATE SYNC SHAPE  │
└───────────────────────┘ └───────────────────┘ │ DEPTH RESET FADE SPREAD      │
                                                └──────────────────────────────┘
```

- Drop a WAV or MP3 on a lane: an empty lane gets a new layer, a full one has its sample
  replaced (its settings kept). Dropping several files fills the lanes from there on.
  A drop past the eighth says so in the status line.
- Click a lane: that layer is focused (the EDIT tab then shows its sample). Drag a lane
  up or down to reorder. Right-click: Remove layer, Duplicate layer.
- In the morph strip, drag a window's points sideways. The position line shows the
  newest sounding note's *p* (the sound's, with RESET off).
- Each lane's waveform shows its playhead while it sounds, dimmed by how closed its
  window is, and has the layer's envelope drawn over it (as the mock-up). A **+ LAYER**
  button opens a file chooser for the next empty lane.
- Every edit is one undo step.

### On the MK3

SAMPLING gets **LAYERS** (New Sampler sounds only): display button 4 pressed again goes
between ZONE and LAYERS, there being four tab buttons. Screens and controls as the
mock-up's MK3 board, with the morph strip added above the layer list:

- The pads go on playing the sound.
- Knob 7 or the 4-D encoder picks the focused layer; knob 8 is always its LEVEL.
- Display buttons 5 to 8 mute the four layers shown (SHIFT: solo). SHIFT + display
  button 1 is **REMOVE**, 2 **SPREAD**.
- Knob pages (◀ ▶), knobs 1 to 6:

| Page | Knobs |
|------|-------|
| **LAYER** | PAN, TUNE, ROOT |
| **ENVELOPE** | ATTACK, DECAY, SUSTAIN, RELEASE |
| **MORPH** | MORPH, POSITION, RATE, SHAPE, DEPTH, RESET |
| **WINDOW** | A, B, C, D of the focused layer, FADE |

- RATE is one knob: it turns through the synced values then the free ones (no SYNC knob
  on the hardware).
- The MK3's browser loads into the focused sound, not a zone, so in this part a layer's
  file is dropped (or chosen with + LAYER) in the software only.

### Files and Maschine

Layer settings are in the DECKARD block, per zone, and the morph with the New Sampler's
settings; both round-trip in text as the rest does. Maschine opens the sound as a Sampler
with overlapping zones that all play at once, at the zones' own gain and tune, and loses
the rest if it saves (as the manual already says of the New Sampler).

## Constraints

- No allocation or locking on the audio thread; the morph and windows are worked out per
  block (or finer) from values already there.
- A note of *n* layers is *n* voices. With `MAX_VOICES` 128 that's 16 notes of 8. A voice
  stolen takes its note's other layers with it (no half notes; the polyphony limit
  already steals by note, the `MAX_VOICES` path doesn't). Every layer with a sample
  starts a voice on every note, muted or shut ones too, so unmuting a layer or turning
  POSITION on a held drone brings it in. A silent layer still moves on through its
  sample, so it's in the right place when it opens; it may skip its effects meanwhile.
- The Sampler's per-voice effects (drive, filter and so on) run per layer voice. That
  the drive of a sum isn't the sum of drives is accepted.
- Clean room, as ever: the EPS behaviour is from the user's own tool and Ensoniq's
  published manuals.

## Left to the builder

- Names and shapes in the model (suggested: `ZoneMeta.layer: Option<Layer>` with mute,
  solo, envelope, window; `NewSampler.morph: Morph`), and the text lines.
- How the per-note sweep and the shared one are kept in `SamplerRt`.
- Layer colours (the mock-up's eight), exact sizes, the envelope drawing.
- Whether a shut layer skips its effects, if measuring shows it's worth it.
- The manual: a **Layers** section in `new-sampler.md` (site PR #14), with the MK3 pages.

## How it's checked

- Model: layer and morph settings round-trip through text; spreading for 1, 2 and 8
  layers, both shapes; a dragged window survives adding a layer.
- Engine: two layers of different roots sound the same pitch on a key; MORPH off sums
  them; on, at POSITION 0 only layer 1 sounds, halfway between two both at half (linear)
  or 0.707 (equal power); Round wraps from the last to the first; a synced sweep with
  RESET off is at the same place on every pass of the song; RESET on starts each note at
  0; a layer's attack delays its level; turning POSITION makes no step bigger than the
  ramp allows; stealing takes whole notes.
- Viewer and MK3: dropping files makes layers and analyses roots; reorder, remove,
  duplicate, undo; the MK3 pages turn the same values; the mod matrix lists and drives
  "morph".
- By ear, by the user: three or four long samples, MORPH on at 16 bars.
