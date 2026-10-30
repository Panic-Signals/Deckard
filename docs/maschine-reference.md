# Maschine 3 as a reference renderer

Status: **works.** DECKARD's VST3 host (`crates/plugins`) runs `Maschine 3.vst3` headless,
loads any `.mxprj` into it, feeds a test signal or a MIDI note, and gets Maschine's own
output back, sample-exact and deterministic. Prototype:
`apps/viewer/examples/maschine-ref.rs`.

## 1. The plug-in

`/Library/Audio/Plug-Ins/VST3/Maschine 3.vst3` (classes: `Maschine 3`, Audio Module Class,
cid `5653544E694D336D61736368696E6520`; plus a `Plugin Compatibility Class`).

| buses | |
|---|---|
| audio in | 16 stereo: `Maschine In 1` … `Maschine In 16` (all main-type) |
| audio out | 16 stereo: `Maschine Out 1` … `Maschine Out 16`; Out 1 = master |
| event | 1 in, 1 out (16 channels) |

- `Plugin::new` works from the main thread; no editor needed. ~0.6 s to initialise (it
  logs to stdout, scans its content database). Controller is the component itself (no
  separate controller state); 129 host parameters (automation/macro slots).
- It processes at whatever `setupProcessing` sets (tested 44.1 and 48 kHz). Block size ≤
  1024 (our `MAX_BLOCK`).
- **It writes into the user's home**: `~/Library/Logs/Native Instruments/Maschine 3.log`,
  `~/Library/Application Support/Native Instruments/Maschine 3/komplete.db3`, preferences,
  and creates `~/Documents/Native Instruments/...` folders. Setting `HOME` and
  `CFFIXED_USER_HOME` to a scratch directory before loading redirects all of it, and the
  output is unchanged (authorisation is system-wide). `maschine-ref` does this by default
  (`$TMPDIR/maschine-ref-home`).

## 2. State = a `.mxprj` file

`IComponent::getState` returns **a complete `.mxprj` file**, byte for byte the same
container (`u64 size` + `hsin` item, `MAS2/PROJ` › `NISD/116` › `NISD/109` boost archive).
`mxprj-dump`, `crates/mxprj` and `deckard_doc::Document` read it unchanged; the default
state (76,357 bytes) is an empty project: one group "", 16 empty sounds, M3 class versions.
Conversely `setState(<any .mxprj file's bytes>)` loads that project, synchronously enough
that the very first `process` already runs the new graph (effects need no preroll;
sampled sounds and drum synths rendered identically with 0.5 s or 2 s preroll).

So a reference scene is just a project file, built with our own `crates/doc` editor.

### Routing a test signal through one effect

What `maschine-ref make` writes (all through `deckard_doc` APIs):

- every other sound emptied (`reset_sound`), so nothing else sounds or loads;
- the chosen sound keeps one slot of its chain (the effect) and is set active;
- Sound Input › Audio › Source = `AudioSource::Ext(0)` (= `In 1 L+R`, plug-in bus 0), input
  gain 0.75 (0 dB), level 0.75 (0 dB), pan 0, unmuted;
- Output = `AudioDest::Ext(1)` (= `Ext. 2`, plug-in output bus 1), which **bypasses the
  group and master** chains/faders;
- optional `field=value` on the kept module (`Target::Module(g, s, 0)`; field names are
  `crates/mxprj` names, e.g. `unknown_b0` of `Compressor`).

**Calibration:** with no slot (`-`), out = in **bit-exactly at lag 0** (residual −240 dB).
So the channel strip at unity is the identity, and an effect render is the effect alone.
Renders are deterministic (two runs null to −240 dB).

### MIDI

Notes from the host reach a sound only if its **plug-in** MIDI source is `Host`. A plug-in
instance reads a separate saved value (`SoundConfig.midi_source_plugin`, Sound
`event_input_b`; see channel-software.md §2.1 ¹) — `midi_source` (the standalone app's)
is ignored, and `Default` does not mean the host. `make` sets both to `Host`, channel All,
and the group's Key Mode to Off. For this we added `SoundRoute::MidiSourcePlugin` to
`crates/doc`. With that, `--note` renders the sound's instrument (tested: Kick drum synth,
Sampler with project samples). Unmodified projects whose sounds say `Default` stay silent
under host MIDI; projects also did not play their patterns from host transport (`--play`)
in our tests — not investigated further.

A Sampler + known WAV built from scratch works (`Document::new_project`, `add_slot`
Sampler, `set_zones` with `ZoneFrom::Template`; rendered with `make` + `--note 60`).
Untested/open: host transport sync; side-chain inputs.

## 3. `maschine-ref`

```sh
cargo build --release -p deckard --example maschine-ref
M=target/release/examples/maschine-ref
SRC="$HOME/Documents/Native Instruments/Maschine 2/Projects/grainular.mxprj"

$M info                                   # classes and buses
$M state $TMPDIR/default.mxprj               # default state (an empty project)
$M list "$SRC"                            # sounds, chains, module params (field = stored value)
$M make "$SRC" $TMPDIR/thru.mxprj 2 2 -      # pass-through (calibration)
$M make "$SRC" $TMPDIR/comp.mxprj 2 2 1 unknown_b0=0.3   # sound 2/2 keeps slot 1 (Compressor)
$M render $TMPDIR/comp.mxprj $TMPDIR/comp.wav --input noise --seconds 2 --input-wav $TMPDIR/in.wav
$M render $TMPDIR/thru.mxprj $TMPDIR/thru.wav --input noise --seconds 2
$M compare $TMPDIR/in.wav $TMPDIR/thru.wav   # null test: best lag, residual dB (here −240)
$M make "$SRC" $TMPDIR/kick.mxprj 2 0 0      # Kick drum synth
$M render $TMPDIR/kick.mxprj $TMPDIR/kick.wav --note 60 --seconds 2
```

`render` options: `--input sweep|noise|sine:HZ|impulse|wav:PATH|none` (default sweep,
`--level -12` dBFS; `wav:` is played as is), `--note N[:VEL]` (off 0.5 s before the end),
`--seconds`, `--sr`, `--bus N` (default 1 when a sound goes to Ext. 2, else 0 = master),
`--preroll S` (default 1, processed silence while pumping the main run loop), `--play
[--tempo]`, `--realtime` (pump the run loop during the render). It prints the RMS of every
output bus that carries signal. Output: 32-bit float stereo WAV.

Null-testing a port: render the same input through DECKARD's effect with the same stored
parameter values (`maschine-ref list` shows them) and `compare` the two WAVs.

## 4. Changes made for this

- `crates/plugins`: `Plugin::component()` (bus queries) and `Plugin::output_bus(bus,
  frames)` (outputs other than the main one).
- `crates/doc`: `SoundRoute::MidiSourcePlugin(String)` (writes `midi_source_plugin`).
- `apps/viewer`: dev-dependencies `hound`, `vst3`; the example.
