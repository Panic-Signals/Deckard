# HIL: Jam strip repoint — `strip-repoint-2026-11-20` (Task 4)

Rig check that the strip runs on DECKARD's own `maschine_jam` script with **no NI
install**, after Tasks 1–3 moved page composition into the script and rendering into
the viewer. The scriptless proof (`jam_oso_pages_need_no_ni_install`) covers the same
ground headless; this page is the hands-on-hardware pass.

## Rig and commands

- **Rig:** Maschine Jam on USB, Maschine quit **and its background agents gone** — with
  both running each acts on every press and they overwrite each other's lights. Check with
  `pgrep -fl 'NIH|NTKDaemon'` and kill what's left:
  `pkill -f 'NIH'; pkill -f NTKDaemon`. Leftover agents (from a Maschine session that has
  since quit) also make NI plug-ins fault while DECKARD restores their state: a segfault at
  startup whose stack runs through `deckard_plugins::Vst3::set_state` into the plug-in —
  `AutoSaveConfiguration::read` / `PAProductInfo` for Reaktor 6 — is this, not the build.
  Kill the agents and it clears.
  Fresh project (`Document::new_project()` state, or wipe the settings file so the
  four pad-layout settings are back at Isomorph / Sequential / In Key / Vertical).
- **Clear the script override folder** — a `devices/maschine_jam/` tree under the
  settings folder's `controller/` dir shadows the shipped script (the user file wins
  over the binary's): `~/Library/Application Support/deckard/controller/` on macOS
  (`$DECKARD_SETTINGS/controller/` when set). Move it aside, so the rig runs the
  script this branch ships.
- **Prove scriptless on hardware** — boot with the NI path explicitly absent:
  `env -u DECKARD_SCRIPTS cargo run --offline -p deckard`
  The viewer must come up with no `controller script:` error in the status line,
  and Preferences ▸ MIDI ▸ Maschine JAM stays ticked.
- **Headless proof (CI):**
  `cargo test --offline --workspace` — look for
  `jam_tests::jam_oso_pages_need_no_ni_install ... ok` (deckard-surface lib) and
  the `jam_strip` / viewer `jam_tests` rows.
- Where to look: the on-screen strip in the viewer window (one row of equal slots,
  group header band above, value + sub-label per slot, underline = focus, corner
  brackets = editing, `Title n/N` counter for multi-page types).

## The pass

### 1. SCALE shows `filled 9`

- Touch the encoder → the SCALE menu opens, strip title `Scale`, one page.
- Fresh project: 8 filled — the chord-type slot is a hole while Chord Mode is Off.
- Turn to `Chord Mode`, press, turn to `Harmonizer`, press again → the chord-type
  slot fills: **9 filled slots**, headers SCALE / CHORD / PAD LAYOUT.
- Expect: no zero-length labels anywhere (holes keep their columns, no cell drawn).

### 2. No strip on a view switch; `Scale 1/2` and `2/2`

- SHIFT + PAD MODE → keyboard view: **no strip at all**. The reference opens an OSO on an
  encoder touch, not on a view switch; a page appearing here is the defect this row replaced.
- Touch the encoder → `Scale 1/2`: 8 slots, headers SCALE / CHORD / PAD LAYOUT.
- Turn the encoder to the last item → `Scale 2/2`: 1 slot, `Vertical / Direction`
  (PAD LAYOUT). Turning back returns to `1/2`.
- The underline is on a filled slot on both pages; no `2/2` renders empty.
- Settings ▸ pad-layout Mode → Standard → `Scale` is a single page (no counter); back to
  Isomorph → `1/2` + `2/2` again.

### 3. ARP is 8 / 7 / 2

- SHIFT + NOTE_REPEAT (Arp on) → keyboard view; touch the encoder → strip
  `Arpeggiator 1/3`: **8 filled** (MAIN + RHYTHM + OTHER).
- Turn the encoder 8 → `2/3`: **7 filled** (Hold + SCALE + CHORD + PAD LAYOUT head).
- Turn 7 more → `3/3`: **2 filled** (layout Type + Direction, no headers).
- Expect the `n/N` counter on all three; turning past the last slot clamps.

### 4. Focus survives a rebuild

- On `Arpeggiator 2/3`, switch the layout Standard → Isomorph (or toggle chord
  mode in SCALE): the fill changes under the strip (4/5/6/8/9 across the rig
  spread) and the focus underline must stay on a filled slot — never on a hole,
  never past the end. A still-filled focus never moves; a stranded one resolves
  backwards onto the last filled slot.

### 5. ENCODER press enters and leaves edit

- Scale OSO open (encoder touched, keyboard view): press ENCODER → corner brackets on the focused slot
  (editing), and the encoder turn edits that slot (root note C3 → C#3 on slot 0);
  without edit the same turn moves the master level instead. Press again → the brackets
  leave and the cursor **stays** on the slot (the menu's rule: browse with the turn). The
  old confirm-and-advance binding went with the page that used to host it.
- **The bracket is the script's**: the press and the turn reach `maschine_jam` itself,
  which keeps the cursor and the edit flag and publishes both. A stuck bracket is a script
  bug, not a viewer one — check `main.lua`'s `menu_press`/`menu_turn` first.
- Every menu behaves the same: press ENCODER → the script's editing bracket on the cursor's
  slot (`[value]` on the status line too); press again → the bracket leaves and the menu
  stays open, browsing. Same for GRID, ARP, PERFORM FX.
- Desktop browser open: the strip hides entirely, and the script declines the ENCODER press
  (`keyboard_press` returns on `deckard.state().browser_open`), so no invisible edit is set —
  closing the panel shows a clean strip. With the script's own browse mode on, the press
  belongs to the library instead (`browser_press`).

### 6. PERFORM FX

- SHIFT + PERFORM → the host adds an FX if the group has none and the
  `PERFORM FX` menu opens: **one filled slot**, `Effect` + mode name
  (e.g. `Filter`), header PERFORM FX. Turn the encoder → the mode steps through
  the list (Filter … Scratcher) **and the slot's text changes in the same frame**.
  (The host ticks the script once more after applying what it asked for; the stale
  frame this page used to record is gone — a mode that lags by a frame is the bug now.)
- The no-FX close is covered headless (`jam_oso_pages_need_no_ni_install` asserts 0
  filled with no FX loaded) — confirm on the rig that the strip never shows a default
  mode.

### 7. TEMPO publishes its page

- Press TEMPO → the strip shows `Tempo` with **one** slot: `120.00` (or the song's tempo), no group
  header above it and no name below it.
- Turn the encoder without pressing → nothing changes (the page opens browsing, as the reference
  does). Press ENCODER → corner brackets on the slot; turn → the tempo moves and the status line
  agrees.
- Press TEMPO again → the page goes. Leave it up and press a page button (STEP/HOLD/PATTERN) → it
  goes with the page: TEMPO is static and never arms the idle timer, so it must survive two seconds
  of no input and die on the page change instead.

## Sign-off

All six rows as above, `DECKARD_SCRIPTS` unset for the whole run, no
`controller script:` status-line error. Note the confirm-and-advance verdict
(row 5) — that binding is the one rig decision Task 3 left open.
