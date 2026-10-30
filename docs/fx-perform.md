# Maschine's Perform FX: the processor

A clean-room description of what `NI::MASCHINE::RT::PerformanceFXProcessor` does around
the eight effect cores (`NI::DSP::JamFilter`, `JamFlanger`, `JamBurstEcho`,
`JamResoEcho`, `JamRing`, `JamStutter`, `JamTremolo`, `JamScratcher`). The cores have
their own documents (`docs/fx-perform-<name>.md`). DECKARD's processor is `PerformFx` in
`crates/engine/src/fx/perform/mod.rs`; the tools to render Maschine's are in
`out/perform-ref-ready.md` (not committed) and §5.

Sources: `out/decomp/perform/` and `out/decomp/perform-data/` (`INDEX.tsv` maps files
to functions), plus the disassembly of `processEffectsWithCrossfade` (0x101a8a704).
Confidence marks as in fx-filter.md.

## 1. Settings reach the cores **[exact]**

`DATA::PerformanceFX::connectParameters` binds every stored field to
`setFloatParameter` / `setEnumParameter` / `setBoolParameter(mode, index, …)`, which
calls core `mode`'s `setParameter(index, value)`. Enum and bool values arrive as floats
(0., 1., 2.…). Off the audio thread the call is direct; on it, it's queued as a
sample-accurate event.

| mode | core index: field (setter) |
|---|---|
| 0 Filter | 2: `filter_min_freq`; 3: `filter_max_freq`; 4: `filter_resonance`; 5: `filter_type` (enum); 6: `filter_saturation`; 7: `filter_out_level` |
| 1 Flanger | 2: `flanger_decay`; 3: `flanger_polarity` (enum); 4: `flanger_tone` (enum); 5: `flanger_stereo`; 6: `flanger_saturation`; 7: `flanger_wet_level` |
| 2 Burst Echo | 2: `burst_time`; 3: `burst_feedback`; 4: `burst_time_mode` (enum); 5: `burst_filter`; 6: `burst_wet_level`; 7: `burst_ts_assign` (enum); 8: `burst_feed`; 9: `burst_mix_mode` (enum) |
| 3 Reso Echo | 2: `reso_resonance`; 3: `reso_feedback`; 4: `reso_time`; 5: `reso_saturation`; 6: `reso_pump`; 7: `reso_mix_mode` (enum); 8: `reso_wet_level` |
| 4 Ring | 2: `ring_spread`; 3: `ring_mix_mode` (enum); 4: `ring_dry_wet`; 5: `ring_rev_mix`; 6: `ring_rev_time`; 7: `ring_glide` |
| 5 Stutter | 2: `stutter_ts_assign` (enum); 3: `stutter_length`; 4: `stutter_pitch`; 5: `stutter_direction` (enum); 6: `stutter_quantize` (bool); 7: `stutter_retrigger` (bool); 8: `stutter_gate` |
| 6 Tremolo | 2: `tremolo_attack_release`; 3: `tremolo_stereo`; 4: `tremolo_trem_vibr`; 5: `tremolo_vibr_mode` (enum); 6: `tremolo_depth_min`; 7: `tremolo_depth_max`; 8: `tremolo_rate_min`; 9: `tremolo_rate_max` |
| 7 Scratcher | 2: `scratcher_wet_level`; 3: `scratcher_delay_time`; 4: `scratcher_feedback`; 5: `scratcher_freq_shift` |

The table is generated from the decompile's bindings (stored offset → core index). The
names are DECKARD's fields for those offsets (`deckard_model::PERFORM_FX`, in offset
order), and the enum and bool setters fall where the names expect them.

**The strip is different:** `touchstrip_pos` and `touchstrip_on_off` go to the processor
(`setTouchstripPos`, `setTouchstripOnOff`), which keeps them (`+0xa0`, `+0xa4`) and passes
them only to the **current** core (its `setTouchstripPosition`, `setTouchstripOnOff`,
i.e. `setParameter(0, pos)` / `setParameter(1, 0 or 1)`). A core that isn't current
doesn't hear the strip move. It gets the values as they are when it becomes current
(§2).

## 2. Changing mode **[exact]**

- `setModeSelector` only records the mode asked for (`+0xac`) and a flag.
- At the start of a host block (`process`), with no crossfade running, the request
  becomes due. At the next control tick (§3), still with no crossfade, `switchEffectMode`
  runs:
  1. the old core becomes the "leaving" one;
  2. the new core gets the strip's position and engage, the tempo and the song position;
  3. the new core is reset and resumed (vtable +0x38, +0x28);
  4. two ramps (the cores' linear glide, 20 ms: `len = (sr·0.001f)·20`) start from 0 to 1:
     `in` and `out`.
- During the crossfade, per sample: the new core hears `in·x` and the old core `x`, and
  `y = fma(1 − out, old, out·new)`.
- When both ramps have landed (≥ 0.99, not moving), the old core is suspended (+0x30)
  and the ramps reset to 0. Its settings then land at once when set, until it is current
  again.
- **At load** the processor's current core is Filter (mode 0) and the project's mode is
  a pending request, so the first 20 ms crossfade from Filter into it.
  `maschine-ref`'s 1 s preroll hides this. DECKARD starts on the project's mode at once,
  with every other core suspended (`Mode::deactivate`) and the current one activated
  (`Mode::activate`).

## 3. Control ticks **[exact]**

The processor runs the audio in stretches of at most 128 samples, on a counter from its
first sample. At each 128-sample tick it calls the current core's `processControlRate`,
and the leaving core's during a crossfade. Pending mode switches happen only on a tick.
Stretches are also cut at queued events (parameter changes on the audio thread).

## 4. Tempo and song position **[exact]**

Per host block: `+0xf8 = (f64)tempo` (the host's tempo as an f32), `+0x100` = the song
position (quarters), `+0x108` = playing. They reach the current core (and the leaving
one) through `onBPMChanged` / `onPositionChanged`, if it wants them. In DECKARD:
`Mode::set_bpm(f32)` per `set_tempo`; the song position in `Ctx::song`.

## 5. DECKARD's interface for the modes

- `Mode::set(&Params)`: every setting, each time any changes. The strip values in it are
  the ones this mode last saw (§1). A core compares with what it had to send only what
  changed, as Maschine does.
- `Mode::frame(l, r, &Ctx)`: one sample. `Ctx::pos` / `engaged` are the strip glided /
  as set, kept for modes that use them; Maschine's cores take the raw values from `set`
  and glide them themselves.
- `Mode::activate()` / `deactivate()` / `set_bpm(f32)`: §2, §4 (default: nothing).
- Shared helpers in `kit.rs`: `Glide` (the cores' linear glide), `Tpt` (the double
  one-pole de-zipper), `tan5` / `tan5d`, `exp10` (`__exp10`).

## 6. Null tests

`fx::perform::tests::a_change_of_mode_crossfades_over_twenty_ms` checks the switch's
timing and shape. Not yet nulled against Maschine: a change of mode itself (it needs
two exact cores, and a way to automate `mode`; host automation, `--host N=mode`, is
untried).
