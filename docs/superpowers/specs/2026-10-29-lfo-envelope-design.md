# LFO Envelope mode — design

Extends docs/superpowers/specs/2026-10-28-lfos-design.md. Agreed with the user on 2026-10-29;
the approved mockup is `.superpowers/brainstorm/98484-1793311607/content/envelope-mode.html`.

## Intent

The user wants one-shot modulation shaped by notes: "once we've added the envelope to the mod
matrix, I can assign it to, say, a filter parameter. Then when I play a note, the filter cutoff
follows this envelope. A one-shot type thing, not a loop like the LFO." It should behave like a
synth's ADSR: the note's length matters, and the release starts when the note ends.

## Goals

- Any of the 16 LFOs can be switched to **Envelope** mode (and back), per LFO.
- An envelope is a mod-matrix source like an LFO (same routes, same Depth maths, 0..1 output).
- Classic ADSR driven by its trigger sound's notes, from patterns and played live.

## Non-goals

- Tempo-synced envelope times (the user chose plain times).
- Sustain pedal (CC 64) holding the envelope.
- Per-voice (polyphonic) envelopes: one envelope per LFO slot, shared by all notes of the trigger.
- Curve/shape controls for the segments (linear segments; builder may pick a pleasant curve, see below).
- Loop or multi-stage envelopes.

## Anti-goals

- Clicks or jumps in the modulated parameter when notes overlap or retrigger.
- The envelope looping or free-running without a note (it is silent — output 0 — until a note
  starts it, and returns to 0 after the release).

## Behaviour (decided with the user)

- **Mode:** each LFO has a mode, LFO or Envelope. Switching keeps the other mode's own settings
  (Shape/Sync/Rate/Phase/Fade; A/D/S/R/Retrigger), so switching back restores them.
  **Controller call:** Trigger, Level and Quantize are one setting shared by both modes. When
  switching to Envelope with Trigger Off, the trigger becomes the focused sound and stays set if
  the user switches back to LFO (they can turn it Off there).
- **Trigger is required** in Envelope mode: the same trigger picker as LFOs (a sound, "A1 / S3"),
  with no "Off". Any note that sound actually sounds — the same set that restarts an LFO today
  (`trigger_with`: not muted, not soloed out, not a choke/controller event) — is a note-on for it.
- **Note-off** is when that note ends: a pattern note's length runs out, or a live pad/key is
  released. The engine already knows both (pattern note lengths in `trigger_with`; live releases
  in `release_local`).
- **ADSR:** Attack rises 0 → 1; Decay falls to Sustain; holds at Sustain while any note of the
  trigger sound is held; Release falls to 0 after the **last** held note ends. A note-off during
  Attack or Decay releases from wherever the envelope is.
- **Retrigger (on/off, per envelope):**
  - On: every new note restarts the Attack, even while other notes are held.
  - Off (legato): the first note starts the envelope; further notes while any note is held change
    nothing. A note that arrives during the Release (no notes held) starts it again.
  - Either way the Release starts only when the last held note ends.
  - **Controller call:** a restart rises from the envelope's current level, not from 0
    (no clicks). The Attack time is the time a full 0 → 1 rise would take.
- **Losing the trigger** (its sound is removed — the existing fix-up sets it to none):
  **controller call:** the envelope stays in Envelope mode, outputs 0 (its routes leave targets
  alone, as a muted LFO's do), and shows Trigger "None" until one is picked. The pickers offer no
  "Off" in Envelope mode, so the user can only set a sound. MK3 button 8 (TRIG) in Envelope mode
  sets the trigger to the focused sound and never turns it off.
- **Times:** Attack, Decay, Release each 1 ms to **10 s** on an exponential knob (the user's call:
  "milliseconds up to 10 seconds"). Sustain 0–100%.
- **Level** applies as for LFOs (scales the output). **Quantize** steps the envelope's *height*
  (the user's call): with N steps the output snaps to N levels (0 and full among them), so Attack/Decay/Release become
  staircases; Off is smooth.
- **Mute** (Task 8) applies too: a muted envelope outputs nothing.
- **Sustain pedal:** ignored (the user's call). Release follows note ends only.
- Shape, Sync, Rate, Phase and Fade in do not apply to an envelope and are hidden in that mode.

## Defaults (controller's call)

A new envelope (or an LFO switched to Envelope for the first time): Attack 10 ms, Decay 300 ms,
Sustain 50%, Release 300 ms, Retrigger On. Attack, Decay and Release are each the time a full
0↔1 rise or fall takes (so partial moves are proportionally shorter).

## Order

Built after Tasks 8–11 of the LFO branch (mute, SHIFT+ERASE / SHIFT → DELETE on MK3 button 8,
MIDI CC destinations, moving built-in knobs), so it builds on all of them: a muted envelope
outputs nothing; SHIFT shows DELETE on button 8 in both modes.

## Names and pictures

- An LFO in Envelope mode is named **"ENV n"** (n = its pad) everywhere it's named: the software
  list, the MK3 screen title, the mod matrix Source row/picker (`ModSource::Lfo(n)` stays the same
  source; only its displayed name changes). Controller call.
- The picture is the ADSR shape with a fixed-width sustain stretch and a dashed "note off" line
  where the release begins (as in the mockup), on both the software panel and the MK3 right screen.
  Level and Quantize show in the picture as for LFOs.

## Software (LFO panel)

A **Mode** selector (LFO / Envelope) first in the control row. In Envelope mode the row shows:
Mode, Attack, Decay, Sustain, Release, Level, Quantize, Trigger, Retrigger — same knob/selector
widgets as the rest of the panel (Task 4b). Knob value text: "12 ms", "1.20 s".

## MK3 (LFO page)

- Display button 7 (empty today) is **ENV**: toggles the focused LFO's mode; lit in Envelope mode.
- In Envelope mode: buttons 1–5 (shapes) go dark and do nothing; button 6 (SYNC) becomes
  **RETRIG** (toggle, lit when on); button 8 stays TRIG. Task 9's SHIFT → DELETE on button 8 still
  applies.
- Knobs in Envelope mode: Attack, Decay, Sustain, Release, Level, Quantize, Trigger, (8 empty).
- Left screen: "ENV 3" big and centred (plus "MUTED" under it when muted, per Task 8).

## Saving

The mode, A/D/S/R and Retrigger are saved with the LFO in the `deckard:lfos` text (so in the
DECKARD block), appended as optional trailing tokens the way Task 8 added `muted`, so older text
reads as LFO mode with default envelope settings. Undoable like other LFO edits.

## Left to the builder

- The runtime state for the envelope (stage, level, held-note count or set) and where note-offs
  are hooked so both live releases and pattern note ends reach it — real-time safe, no allocation.
  Choked notes and notes cut by a mono/choke group: treat as note-offs for the held count, so the
  envelope never sticks at Sustain.
- Exports: envelopes run in bounces exactly as in playback.
- Stop/transport stop: all held counts clear and the envelope releases (or drops to 0 — builder's
  call, say which).
- Segment curves: linear is acceptable; an exponential-feeling decay/release is fine if it's simple.
- The exact text tokens and block version handling (bump only if needed).
