# OSO test sheet — every page, how to reach it, when it goes

For end-user testing on a machine with a Jam. Every row is a page the strip can show; the commands
are the Jam's own buttons, and the `--jam` column reproduces the same row on a machine with no
hardware (`--jam` holds every named key down before releasing any, so a chord is the chord).

Nothing here is aspirational: each row was run before it was written. Statics stayed up for the
whole 7 s probe, timed pages closed at ~2 s, `BROWSE` showed nothing at all.

## A. The pages

| # | Strip title | Jam buttons | No hardware | Slots | Opens | Goes |
|---|---|---|---|---|---|---|
| 1 | `GRID` | **GRID** | `--jam GRID` | 4 — STEP, NUDGE, ARRANGE, PERFORM | on the press | GRID again, or any view change |
| 2 | `REC MODE` | **SHIFT + GRID** | `--jam SHIFT --jam GRID` | 8 — Metronome Level, Time, Auto, Count-in, Quantize, Default Vel, Accent Vel, Follow | on the chord, even while GRID shows (GRID stays lit) | GRID again, or any view change |
| 3 | `Tempo` | **TEMPO** | `--jam TEMPO` | 1 — `120.00`, no header, no sub-label | on the press | TEMPO again, or any view change |
| 4 | `PERFORM FX` | **SHIFT + PERFORM** | `--jam SHIFT --jam PERFORM` | 1 — Effect (a hole while the group has no Perform FX) | on the chord (the host adds an FX if the group has none) | **the encoder's press**, or any view change — not a timer |
| 5 | `Scale 1/2` / `2/2` | **SHIFT + PAD MODE** | `--jam SHIFT --jam PAD_MODE` | 8 then 1 (nine total); headers SCALE / CHORD / PAD LAYOUT; a hole at chord-type while Chord Mode is Off | on the view switch, and again on an encoder touch | 2 s idle, or any view change |
| 6 | `Scale 1/2` / `2/2` | **NOTES** | `--jam NOTES` | the same page as row 5 | on the press, and again on a touch | 2 s idle, or any view change |
| 7 | `Scale` (no counter) | **SHIFT + STEP** (piano roll) | `--jam SHIFT --jam STEP` | 3 — Root Note, Bank, Type, under one SCALE header | on the view switch, and again on a touch | 2 s idle, or any view change |
| 8 | `Arpeggiator 1/3` … `3/3` | **SHIFT + NOTE_REPEAT** (arp on) | `--jam SHIFT --jam NOTE_REPEAT` | 17 over three pages: 8 / 7 / 2 | on the press, and again on a touch | 2 s idle, or any view change |
| 9 | `Note Repeat` | **NOTE_REPEAT** | `--jam NOTE_REPEAT` | 5 — Rate, Unit, Gate, Lock, Hold | on the press, and again on a touch | NOTE_REPEAT again (off), 2 s idle, view change |
| 10 | `SWING` | **SWING** | `--jam SWING` | 3 — Master, Group, Sound | on the press, and again on a touch | 2 s idle, another channel-mode button, view change |
| 11 | `TUNE` | **TUNE** | `--jam TUNE` | 2 — Group, Sound | on the press, and again on a touch | 2 s idle, another channel-mode button, view change |
| 12 | `LOCK` | **SHIFT + LOCK** | `--jam SHIFT --jam LOCK` | 3 — Morph, Mode, Time / Grid | on the view switch, and again on a touch | 2 s idle, SHIFT + LOCK again, view change |
| 13 | *nothing* | **BROWSE** | `--jam BROWSE` | — | — | the library panel wins while it is open: no strip at all |

"Any view change" is `set_view`: PAD MODE, SONG, SHIFT + STEP and the rest. A page never outlives
the view it was opened in.

## B. What to press inside a page

| Do this | Expect |
|---|---|
| Turn the encoder | the focused slot's value moves; the underline stays on that slot |
| Press the encoder | corner brackets on the focused slot (editing); press again to leave them |
| Turn while editing | the focused value changes (except on PERFORM FX, where the press **closes** the page) |
| Turn past the last item of a multi-page type | the counter moves (`Scale 2/2`, `Arpeggiator 2/3`, `… 3/3`), never to a page that draws nothing |
| Turn through a menu with unavailable items (isomorphic layout on Standard) | the cursor steps over them: it never rests on a hole |
| Leave a timed page alone ~2 s | it goes, and an **encoder touch** brings it back (it is still that view's page) |
| Navigate away (PAD MODE, SONG, …) | whatever was showing goes, static pages included |
| Open BROWSE | the strip disappears; closing the panel brings the page back |

## C. Lights, per row

| Button | While its page shows |
|---|---|
| GRID | lit for rows 1 **and** 2 (REC MODE does not take the GRID light away) |
| TEMPO | lit for row 3 |
| PERFORM | lit for row 4 |
| PAD_MODE | lit in the keyboard view (row 5) |
| STEP | lit in the piano roll (row 7) |
| NOTE_REPEAT | lit while the repeat or the arp is on (rows 8, 9) |
| SWING / TUNE / LOCK | lit on their own pages (rows 10–12) |

## D. Known-absent

| OSO | State |
|---|---|
| `RANDOMIZER` (`Variation`) | **no page is composed, and the model is short of one.** Maschine's page reads thirteen settings over two pages (Humanize: five); DECKARD's Variation module keeps five (`sequence_variation/velocity_amount`, `time_shift`, `length_scale`, `length_variation`, `distribution`) and its *Generate* module keeps the rest (`probability`, `note_range/low|high`, `note_count`, `distribution`, `steps`, `fills`). Maschine's velocity **lo/hi**, its **mode** (Random / Humanize / …), its **Apply** and its **three separate distributions** have no counterpart. A page composed today would draw numbers that do not exist behind it. Static in the reference (it does not time out) — one more thing confirmed against the same list that once got PERFORM FX wrong |
| `INSTANCE` (`SHIFT` + `H`) | **a different window, not this strip.** What it opens is NI's *desktop on-screen overlay* (§2 of the RE notes): its own window, "MASCHINE JAM" in the chrome, an instance page listing products with their pad-layout line. That is the overlay this branch deliberately leaves out — DECKARD's equivalent lives in the in-app library panel |
| desktop `BROWSE` overlay (§2 of the RE notes) | out of scope for this branch — the library stays an in-app panel |
| NI-scripted controllers' OSO | their own path in `lua-host`, untouched, coexists by design |

## E. Sequencing worth a second look on hardware

1. GRID showing → SHIFT + GRID → REC MODE, GRID still lit → GRID → back to GRID, not closed.
2. PERFORM FX up → press the encoder → page goes, the group's FX stays.
3. SWING up → wait 2.5 s (gone) → touch the encoder → SWING back.
4. Scale 1/2 → turn to 2/2 → switch layout Standard in the app's Settings → the second page stops
   existing and the cursor resolves back onto a filled slot.
5. `Arpeggiator 2/3` → change the pad layout → the cursor never lands on a hole.
