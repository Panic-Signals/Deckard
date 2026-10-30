# Controller script mods

Status: **partly done.** The script host can load mods and splice them into NI's scripts,
and implements the note-editing calls step-sequencer mods use (docs/controller-steps.md).
The app doesn't load mods yet.

## Why

People customise Maschine's hardware behaviour by changing NI's controller Lua scripts.
DECKARD runs those same scripts, read from the user's own Maschine install (never bundled),
so it can run such mods too. In Maschine 3 the scripts are compiled into the app, so mods
there have to patch its binary. In DECKARD a mod is just a folder.

## Two kinds of mod folder

`Scripts::add_mod` (`crates/lua-host/src/scripts.rs`) accepts either kind:

- **Replacements:** `.lua` files laid out like NI's tree (`Scripts/Maschine/MaschineMK3/...`)
  replace NI's file of the same path. New files add to the set.
- **Extensions:** a folder with a manifest (any `*.json` holding a `resources` list). Each
  entry names one of NI's scripts by file name and gives the file to splice in and where:
  - `append`: at the end;
  - `beforeLine`: before the last line equal to `anchor`;
  - `beforeText`: before the first occurrence of `anchor`.

  Splicing happens in memory; the extracted scripts aren't changed. This is the format of
  vubeatz's "Maschine Update" (`macos.json`). Style sheets are skipped, because DECKARD draws
  its own screens. `nativePatches` and `library` entries change Maschine's compiled program,
  can't apply to DECKARD, and are only reported.

Mods run in the same sandbox as NI's scripts: no file or process access.

## Built in

DECKARD ships one mod of its own, applied whenever it loads NI's scripts
(`Scripts::add_builtin`, `crates/lua-host/mods/`). It is DECKARD's code, written fresh;
NI's scripts were read only for the names it hooks into.

- **Pad Touch** (`mods/pad-touch`): appended to `KeyboardModePageMK3`, it adds two
  parameter pages to the MK3's Keyboard page. Page 3, *Pad Touch*: Mode (Velocity, Poly
  AT, Aftertouch, CC), Curve, Min, Max; page 4, *Pad Touch CC*: the CC number. They are
  the focused sound's settings (the script host's `PadTouch*` sound parameters,
  docs/superpowers/specs/2026-10-24-pad-touch-design.md).

If a Maschine update changes the Keyboard page so the mod can't splice in, the app's
status line says so ("Pad Touch mod didn't apply: …") and the pads keep the sounds' saved
settings.

## Trying a mod

```
cargo run --release -p deckard-lua-host --bin lua-host-run -- out/ni-res \
    --mod "<mod folder>" --project "<a project>.mxprj" --press BUTTON_STEP --pad 0 --png out.png
```

The tool prints what the mod applied and skipped. It also lists the Maschine calls the
scripts used that DECKARD doesn't implement yet, which do nothing.

## Test case: vubeatz "Maschine Update" 0.2.6

Built for Maschine 3.6.0 and tested against 3.7.1's scripts (October 2026). Its `payload`
folder holds 15 Lua extensions, one style sheet, the manifest, and `StepGridChannel.dylib`.

- **Splicing:** all 15 extensions apply, including to the scripts NI changed in 3.7.1. The
  MK3 scripts boot with no errors.
- **Step Mode screen:** pressing STEP opens the mod's page, with its header, KEYBOARD,
  FOLLOW, PROBABILITY and PROB LOCK.
- **The grid works:** pads add and remove steps, the knobs edit the selected step, and
  the displays show the steps (since the note-editing calls, October 2026).
- **Not portable:** Sampler Channel, Attack/Decay and Arranger Audio come from the native
  patches and the library. DECKARD would need its own versions of those features.

## Still to do

1. **Mods in the app.** A `Mods` folder in DECKARD's settings folder, each subfolder one
   mod, loaded over NI's scripts (`apps/viewer/src/controller.rs`, where `Scripts::from_dir`
   is called). Also `--mods DIR` and `$DECKARD_MODS`, a menu item to open the folder and
   reload scripts, and a per-mod report of what applied and which calls are missing.
2. **Mods written for Maschine 2,** which may expect older script names or calls: check
   them one by one.
