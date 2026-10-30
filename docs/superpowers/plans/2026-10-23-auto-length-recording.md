# Auto length recording

> Inline, test-first; the software and the MK3. Follows the user's request (2026-10-23)
> and Maschine's Record Prepare / Auto-Growing Pattern (MK3 manual pp. 330–334).

**Goal:** REC arms; PLAY starts the take; SHIFT + REC counts in first. A pattern whose
length is Auto (the default) records from one bar looping, grows a bar at a time while
each new bar is played into, and settles on the bars played as soon as a bar passes empty.

## Behaviour
- **REC** (header ●, the MK3's REC) with the song stopped **arms**: REC lights, nothing
  plays. **PLAY** then starts the song and the take at once (no count-in).
  - REC while playing records at once, as before.
- **SHIFT + REC** (SHIFT + click on ●; the MK3's SHIFT + REC) counts in, then plays and
  records: Preferences ▸ Recording ▸ Count-in bars (default 1 bar: 4 clicks in 4/4;
  Off counts 1).
- **Auto length** (a pattern no length was chosen for), recorded into from empty:
  - Nothing played yet: the pattern is **1 bar**, looping.
  - A note played in the pattern's last bar adds the **next bar** (the song plays on into
    it).
  - When the song comes round to the start and the last bar has no notes, the pattern is
    **trimmed to the bars played** (bar 1 played, bar 2 not: after bar 2 it loops bar 1),
    and stays that length; recording goes on over it.
  - REC off or STOP: trimmed to the bars played, as before; nothing played: its length
    from before.
- **Choosing a length** turns Auto off for that pattern: the pattern editor's LENGTH menu
  (which gains **Auto**), the MK3's Record Prepare page (hold REC: knob 4, buttons 5–8).
  The editor's and Record Prepare's length show **Auto** while it is.

## Tasks
1. `record::Grow`: the Auto length rule as a small state machine (`hit`, `at`). Unit tests.
2. `Document`: which patterns have a chosen length (`length_is_auto`, `set_length_auto`;
   the session's, not saved). Tests.
3. App: REC arms when stopped; SHIFT + REC counts in; Auto growth while recording (notes
   in, the playhead each frame). Tests (`auto_length_tests.rs`).
4. Software: SHIFT + click on ●; LENGTH menu's Auto and its display. Tests.
5. MK3: `hasFocusedPatternAutomaticLength`; Record Prepare's length choices turn Auto
   off. Script test.
6. Docs: recording-notes.md, PARITY.

## Rulings
- "4 steps" read as one bar of 4 beats (the 4-click count-in, the bars in the example).
  If wrong: the empty loop and each growth should be a beat, not a bar.
- Auto is per pattern for the session (Maschine stores no flag in the project). A pattern
  with notes records at its length; only an empty one grows.
- Once a bar passes empty the length is fixed for the rest of the take (no growing again).
