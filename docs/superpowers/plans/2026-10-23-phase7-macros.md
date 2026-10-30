# Phase 7 — Macros

> Roadmap: `docs/superpowers/plans/2026-10-22-parity-roadmap.md`. Design:
> `docs/superpowers/specs/2026-10-23-phase7-macros-design.md`. Inline, test-first; the
> software and the MK3.

**Goal:** MIDI learn (macros and any parameter), host automation of macros (modelled for
the plug-in build), the MK3 MACRO page and Set Macro mode, the macro slot menu (Rename
⌘R, Learn, Reset, Cut/Copy/Paste) and the Pages menu (Delete, Clear All); stored in the
project as Maschine stores them.

## Tasks
1. **Remote keys and mappings in the document** (`doc/src/remote.rs`): keys from MIDI
   bytes (and their names), the MIDI mapper's map (read, learn, unlearn; one control per
   parameter and one parameter per control), host automation IDs (enable the next free,
   disable). Tests: save and reopen keeps them; a v1 mapper (Maschine 3's) writes.
2. **Macro slot edits** (`doc/src/macros.rs`): a macro's own id, copy/paste (target and
   name), reset. Tests.
3. **The app's remote control** (`apps/viewer/src/remote.rs`): an id's knob, MIDI values
   and host values set it (a macro sets its target), MIDI learn, macro Learn, the macro
   clipboard. Tests.
4. **MIDI in** (`midi_io`): remote keys learned or applied before notes are routed. Tests.
5. **The Assignment area** (the software): PAGES / MIDI / HOST, the fields, the menus,
   the keys. Harness tests.
6. **MK3**: `ParameterPageAccess` (macro page active, Set Macro), `PAGE_MACRO` shows the
   Macro properties, `isAutomated`; the app's Set Macro request. Script tests; the sweep
   `--press BUTTON_MACRO` and SHIFT + MACRO.
7. Docs: PARITY row 3, this plan's results; the full check.

## Results
- Tasks 1–7 done: `doc/src/remote.rs` (keys, the MIDI mapper, host automation IDs),
  `macro_oid` / `reaches` / `paste_macro` in `doc/src/macros.rs`; `apps/viewer/src/
  remote.rs` (an id's knob, MIDI and host values, MIDI learn, macro Learn, the macro
  clipboard, Set Macro, the keys), `midi_io` (remote control before notes),
  `layout/assign.rs` (the Assignment area and the macro slot menu), macro targets now
  include internal effects ("Slots"); the host's `api/macros.rs`
  (`ParameterPageAccess`, `PAGE_MACRO`, the parameter cache's `isAutomated` and
  `getPageParameter`, `Song:isLevelTabChanged`).
- The sweeps `--press BUTTON_MACRO` and `--hold BUTTON_SHIFT --press BUTTON_MACRO
  --press BUTTON_CAP_1` show no errors and none of the Macro pages' API stubbed (what
  remains stubbed is on every page: NI.GUI label modes, accessibility, preferences).

## Rulings
- **Remote keys** are Maschine's (`MidiRemoteKey`, read for behaviour): CC n → n, poly
  pressure on note n → 128 + n, Program Change → 256, channel pressure → 257, pitch
  wheel → 258. No project the user has holds a mapping, so the stored shape (a
  `std::pair<key, shared_ptr<RemoteParameter>>` in `MidiMapper.remote_parameters`, the
  `operations` map untouched) is inferred from the decoder. If wrong: Maschine may not
  read DECKARD's mappings (or they may map other controls).
- **A mapped control's message goes no further** (it doesn't also reach the sounds that
  take MIDI controllers), nor does the one learned. If wrong: in Maschine a mapped CC
  may also play through to a sound's MIDI.
- **Values**: absolute, the knob's travel 0..1 from the 7-bit value (the pitch wheel's
  14 bits; a program change's number). Learning a control assigns it and sets nothing.
- **Host automation IDs** are shown 1-based ("ID 1" is slot 0); Enable takes the lowest
  free one. DECKARD has no plug-in build: `App::host_automate(id, value)` is what one
  will call. If wrong: Maschine may number them from 0.
- **A macro's MIDI or host automation points at the macro** (its `ForwardParameter`),
  not its target, so it follows the macro when it is pointed elsewhere.
- **Macro Learn** (the slot menu): the next parameter changed outside the Macro
  properties, in the software or from the controller, fills the macro, then the next
  slot learns, up to the eighth; Learn again or Esc stops. Parameters the channel can't
  reach are refused (status line). Maschine's own Learn is documented for plug-in
  parameter pages; this is DECKARD's reading of it for macros.
- **Copy / Paste** carry a macro's target and name, not its section heading; pasting
  into a channel that can't reach the target is refused; Cut is Copy then Reset.
- **⌘R / ⌘X / ⌘C / ⌘V** act on the focused macro while the Pages tab shows the Macro
  properties and a macro was the last thing clicked (focus `KeyFocus::Macros`);
  otherwise they act as before (events, sounds, groups).
- **MACRO on the MK3** switches the Control area to the Macro properties when its page
  shows (Maschine does it natively; the scripts don't): Channel shown, the Macro set,
  its page kept when it already was.
- **What can be learned, automated or made a macro**: every knob of the channel
  properties, the Sampler and Maschine's internal effects (what has a project
  parameter). External (VST/AU) plug-ins' parameters aren't parameters in the project
  DECKARD can point at yet; their fields are greyed.
