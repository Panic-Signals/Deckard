# Phase 7 — Macros: design

> Roadmap: `docs/superpowers/plans/2026-10-22-parity-roadmap.md`. Plan:
> `docs/superpowers/plans/2026-10-23-phase7-macros.md`.

## What Maschine does (sources)
- **Manual** (`out/manual/mk3.txt`, "Audio routing, remote control, and Macro Controls"):
  each channel has pages of eight Macro Controls, each an alias of one parameter of the
  channel or one under it. The **Assignment area** under the Control area has a
  **Pages** pane (Macro properties: the target selector, Reset, section and parameter
  labels, the macro slot menu Rename ⌘R / Learn / Reset / Cut ⌘X / Copy ⌘C / Paste ⌘V,
  the page menu Delete / Clear All) and an **Automation** pane with a **MIDI** tab (Learn
  → the next CC / PC / PW moved is assigned, shown as "CC n", "PC", "PW"; right-click
  Unlearn; one control per parameter, one parameter per control; all channels of all
  enabled ports) and a **Host** tab (Enable → the next free automation ID; click removes).
  Macros are automatable when their targets are.
- **MK3 scripts** (`MacroPageMK3.lua`, `MacroAssignPage.lua`, `MacroHelper.lua`,
  `MaschineControllerMK3:onMacroButton`): MACRO shows `PAGE_MACRO` (a Control page whose
  knobs the app turns natively); SHIFT + MACRO pins `PAGE_MACRO_ASSIGN`, where touching
  a knob calls `ParameterPageAccess.setOrRemoveMacroForParameter(App, Param, Level)` and
  each knob shows `anyMacrosForParameter`. `isMacroPageActive` (the Control area shows
  the Macro properties) lights MACRO on the Control page. `leaveMacroPageGroup` steps the
  page group back and shows the Control page.
- **Project files** (decoded with `mxprj`): a macro is a `ForwardParameter`
  (`target`, `custom_name`, `section_name`) in a `MacroPage` of the channel's
  `macro_pages`. MIDI learn lives in the Project's `MidiMapper.remote_parameters`, an
  `ObjectMap<key, RemoteParameter>` (`remote_map` before project version 3); a
  `RemoteParameter` points at the parameter. Host automation is the Project's
  `remote_parameters`, 128 `RemoteParameter`s, the slot index being the automation ID
  (seen in the user's own reference projects: Perform FX parameters on IDs 0–3).
- **Remote keys** (`MidiRemoteKey::encodeMidiEventToKey`, read for behaviour): CC n → n,
  poly pressure on note n → 128 + n, Program Change → 256, channel pressure → 257, pitch
  wheel → 258; shown as "CC n", "PC", "PW" ("CP", "PP n" for the others).

## DECKARD's design
- **Document** (`crates/doc/src/remote.rs`, `macros.rs`): `midi_mappings`,
  `learn_midi(param, key)`, `unlearn_midi(param)`, `midi_key_for(param)`;
  `host_slots`, `enable_host_automation(param)`, `disable_host_automation(param)`;
  `macro_oid` (a macro's own parameter, to learn or automate it), `macro_at`,
  `paste_macro`, `reset_macro`. `remote::midi_key(bytes)` gives (key, 0..1 value).
- **App** (`apps/viewer/src/remote.rs`): what an object id means (the knob's `Edit` of
  every reachable parameter: channel properties, Sampler, internal effects; a macro's
  is its target's), cached by document revision; incoming MIDI and host values set it
  as the knob does. `App.midi_learn` (a parameter waiting for a control),
  `App.macro_learn` (a macro waiting for a parameter), `App.macro_clip`,
  `App.assign` (the Assignment area: closed, Pages, MIDI, Host), `App.macro_focus`.
  `host_automate(id, value)` is the entry point a plug-in build will call.
- **MIDI** (`midi_io`): before notes are routed, a message that has a remote key goes
  to MIDI learn when learning, or to its mapping when mapped (and no further).
- **The software**: the Assignment area under the knobs (a toggle at the bottom of the
  icon strip): tabs PAGES / MIDI / HOST and a field under each knob; the macro slot menu
  (Rename, Section, Learn, Reset, Cut, Copy, Paste) on the field and the name; ⌘R / ⌘X /
  ⌘C / ⌘V on the focused macro; the page tabs' menu (Rename, Delete, Clear All).
- **MK3** (`crates/lua-host/src/api/macros.rs`): `ParameterPageAccess.isMacroPageActive`,
  `setMacroPageActive`, `anyMacrosForParameter`, `setOrRemoveMacroForParameter`;
  `StateCache:getParameterCache():isAutomated`; showing `PAGE_MACRO` switches the
  Control area to the Macro properties (Maschine does it natively). The Control
  snapshot carries each knob's parameter id and whether it's modulated.
