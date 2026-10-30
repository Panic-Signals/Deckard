# Plug-ins on the controller

Status: **working** for the MK3 (October 2026). NI's own Control and Module pages run
unchanged; DECKARD gives them the chain, the plug-in list and its own pictures, and the
app does the loading.

## What it does

- **PLUG-IN** shows the focused sound's chain on the left display, a tile per slot:
  the instrument or effect icon, the name, and a picture of the plug-in. A bypassed slot
  is greyed. After the last slot comes the **+**, focused on a sound with no plug-ins.
  The right display shows the focused plug-in's logo, and the knobs its parameters.
- **MASTER / GROUP / SOUND** (buttons 1–3) pick the chain; the 4-D encoder's left/right
  (or buttons 5/6) move along it, past the last slot to the +.
- **Pressing the 4-D encoder** opens the plug-in browser on the right display:
  "(none)" (removes the slot) and the plug-ins. Knob 1 is TYPE (Instrument or Effect;
  only the first slot of a sound takes an instrument), knob 2 FORMAT (Internal, VST3,
  Audio Units), knob 3 VENDOR (DK for the built-in plug-ins, DECKARD's; Maschine's
  scripts say NI, which DECKARD replaces when they load). The list scrolls to keep the
  selection in view. Turn the encoder (or knob 5) to pick, press it (or LOAD) to
  load: into the focused slot in place of what's there, or on the + after the last slot.
  CANCEL leaves.
- The list is the app's "+" menu: Sampler, Audio, Maschine's drum and bass/poly synths
  and DECKARD's instruments; Maschine's effects and DECKARD's; then the external plug-ins
  this build can host (VST3 for now).

## Pictures

Maschine's tiles are small pictures of each plug-in's panel (40 pixels high for an
effect, 90 for an instrument) and its logos 240 × 196. DECKARD's are pictures of its
own panels, as the app draws them in the Mix view:

- `apps/viewer/assets/plugins/<key>_Thumb.png` and `<key>_Logo.png`, `<key>` being the
  style sheet's (the name in small letters, no spaces).
- Made by `cargo test -p deckard --release -- --ignored make_plugin_pictures`, which also
  writes `apps/viewer/src/plugin_pictures.rs`. Run it after adding a plug-in or changing
  a panel; `every_builtin_plugin_has_pictures` fails until then.
- The host puts them over Maschine's for the modules both have (`image-<key>` in the
  style sheet); external plug-ins get Maschine's generic VST tile, and the + its own.

## How it fits together

- `crates/lua-host/src/api/plugins.rs`: `NI.DATA.ModuleFactory` (built-in modules) and
  `NI.DATA.PluginHelper` (external plug-ins by format and vendor, the
  `NI.HOSTING.PluginFilter` bits) from `State::plugins`, which the app sets
  (`Host::set_plugins`); loading becomes `ControlRequest::Load` for the app.
- The chain's tiles are `PluginItem`s built with their parts (header, type icon, name,
  picture frame); pictures come from `NI.UTILS.PictureManager` and draw in `deckard-nwl`.
- `SKIP_NOT_AUv2_PLUGINS` isn't among the values recovered from Maschine; DECKARD uses
  its own bit for it (only the scripts and the host pass it between them).

## Not yet

- Audio Units and VST2 plug-ins aren't listed: the app can't host them yet.
- SHIFT + 3 INSERT (insert mode) and SHIFT + 5/6 (move a plug-in) aren't checked.
- The Sampler's picture is its empty panel (no sample loaded).
