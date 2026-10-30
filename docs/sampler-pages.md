# Maschine's Sampler: parameter pages

The Sampler's parameters as Maschine shows them on its parameter pages, eight knob slots
per page. This is what `Sampler::getParameter(…, page, slot)` (0x101871df0,
`out/decomp/sampler/0007.c`) returns. Member offsets are mapped to parameters through
`Sampler::Sampler()` (0x10186b214, `0001.c`), where each `insert(label, default, min,
max)` result is stored in `this[k]` (byte offset `8·k`). Field names are the mxprj paths
of `SAMPLER_PARAMS` (`crates/model/src/lib.rs`).

**Page names.** `Sampler::s_aPageNames` is filled at start-up by
`__GLOBAL__sub_I_Modules_Sampler_Objects_ALL.cpp` (`out/decomp/sp-pages/0001.c`). In
order: **Voice Settings / Engine**, **Pitch / Envelope**, **FX / Filter**,
**Modulation**, **LFO**, **Velocity / Modwheel**. **[exact]**

**Sections.** Section names are set in the constructor with
`MaschineParameter::setSectionName` on the first parameter of each group. A parameter
without its own section continues the section of the slot before it; that is how
Maschine groups knobs under one heading (**[inferred]** from the display convention).
**Labels** are the wide-string labels passed to `insert` (`Sampler::Sampler`,
`AHDSREnvelopeObject::AHDSREnvelopeObject` 0x10186aaec, `LFOObject::LFOObject`
0x1018833f8). **[exact]**

"Empty" means `getParameter` returns null for that slot, so the knob is blank.

## 1. Voice Settings / Engine

Condition: Polyphony (`polyphony`, index 0 = Legato).

| slot | field | label | section | condition |
|---|---|---|---|---|
| 1 | `polyphony` | Polyphony | Voice Settings | |
| 2 | — | (empty) | | always empty |
| 3 | `glide` | Glide | (Voice Settings) | only when Polyphony = Legato; otherwise empty |
| 4 | `pitchbend` | Pitchbend | (Voice Settings) | |
| 5 | `mode` | Mode | Engine | |
| 6 | — | (empty) | | |
| 7 | — | (empty) | | |
| 8 | — | (empty) | | |

Glide displays `v²` seconds as `%.2f s` (`FloatSquareDisplayConverter(1.0, "%.2f s")`).

## 2. Pitch / Envelope

Condition: Type (`envelope_type`: 0 One-shot, 1 AHD, 2 ADSR). The envelope slots read
the volume envelope (`this[0x37]`, an `AHDSREnvelopeObject`: members attack, hold,
decay, sustain, release, pitch, cutoff, drive, pan, at +0x00…+0x40).

| slot | field | label | section | condition |
|---|---|---|---|---|
| 1 | `tune` | Tune | Pitch / Gate | |
| 2 | `start` | Start | (Pitch / Gate) | |
| 3 | `reverse` | Reverse | (Pitch / Gate) | |
| 4 | `envelope_type` | Type | (Pitch / Gate) | |
| 5 | `volume_envelope/attack` | Attack | Amplitude Envelope | empty for One-shot |
| 6 | `volume_envelope/hold` (AHD), `volume_envelope/decay` (ADSR) | Hold / Decay | (Amplitude Envelope) | empty for One-shot |
| 7 | `volume_envelope/decay` (AHD), `volume_envelope/sustain` (ADSR) | Decay / Sustain | (Amplitude Envelope) | empty for One-shot |
| 8 | `volume_envelope/release` | Release | (Amplitude Envelope) | ADSR only; otherwise empty |

Tune displays `v·36` as `%.2f` (`FloatScaleDisplayConverter(36, "%.2f")`, bipolar, 72
steps). The attack, hold, decay and release parameters get millisecond converters
(`StringUnits::msToString`).

## 3. FX / Filter

Condition: Filter (`filter`: 0 Off, 1 LP2, 2 BP2, 3 HP2, 4 EQ).

| slot | field | label | section | condition |
|---|---|---|---|---|
| 1 | `comp` | Comp | FX | |
| 2 | `drive` | Drive | (FX) | |
| 3 | `sample_rate` | SR | (FX) | |
| 4 | `bits` | Bits | (FX) | |
| 5 | `filter` | Filter | Filter | |
| 6 | `cutoff` (LP2/BP2/HP2), `eq_frequency` (EQ) | Cutoff / Frequency | (Filter) | empty when Off |
| 7 | `resonance` (LP2/HP2), `eq_width` (EQ) | Resonance / Width | (Filter) | empty when Off and for BP2 |
| 8 | `eq_gain` | Gain | (Filter) | EQ only; otherwise empty |

Resonance displays as a percent with one decimal (`ScaleNormalizedToPercentOneDecimal`).
Cutoff, SR, Frequency, Width and Gain have their own value converters; cutoff and SR
are `getFilterCutoffValueString` and `getLofiSampleRateValueString` (docs/sampler-dsp.md
§8).

## 4. Modulation

The modulation envelope (`this[0x38]`), using the same Type condition as page 2.

| slot | field | label | section | condition |
|---|---|---|---|---|
| 1 | `mod_envelope/attack` | Attack | Modulation Envelope | |
| 2 | `mod_envelope/hold` (Type ≠ ADSR), `mod_envelope/decay` (ADSR) | Hold / Decay | (Modulation Envelope) | |
| 3 | `mod_envelope/decay` (Type ≠ ADSR), `mod_envelope/sustain` (ADSR) | Decay / Sustain | (Modulation Envelope) | |
| 4 | `mod_envelope/release` | Release | (Modulation Envelope) | ADSR only; otherwise empty |
| 5 | `mod_envelope/pitch` | Pitch | Destination | |
| 6 | `mod_envelope/cutoff` | Cutoff | (Destination) | |
| 7 | `mod_envelope/drive` | Drive | (Destination) | |
| 8 | `mod_envelope/pan` | Pan | (Destination) | |

Unlike the amp envelope, these slots are not blanked for One-shot: the modulation
envelope always runs, as AHD unless Type is ADSR.

## 5. LFO

The LFO object (`this[0x39]`; members type, speed, phase, sync, pitch, cutoff, drive,
pan).

| slot | field | label | section |
|---|---|---|---|
| 1 | `lfo/type` | Type | LFO |
| 2 | `lfo/speed` | Speed | (LFO) |
| 3 | `lfo/phase` | Phase | (LFO) |
| 4 | `lfo/sync_mode` | Sync | (LFO) |
| 5 | `lfo/pitch_amount` | Pitch | Destination |
| 6 | `lfo/cutoff_amount` | Cutoff | (Destination) |
| 7 | `lfo/drive_amount` | Drive | (Destination) |
| 8 | `lfo/pan_amount` | Pan | (Destination) |

There are no conditions. Speed displays as Hz or a note division, depending on Sync
(`getLFORateValueString`, docs/sampler-dsp.md §4).

How this page was read: Ghidra names the eight getter calls on page 4 with unrelated
identical-code-folded symbols. Slots 1 and 4 return the enum-parameter offset (+0x170),
which matches Type and Sync, the LFO's two `EnumParameter`s. The constructor puts
section "LFO" on the getter used for slot 1 and "Destination" on the getter used for
slot 5. The order of slots 2, 3, 6, 7 and 8 is **[inferred]** from the member order,
which is also the serialization order.

## 6. Velocity / Modwheel

| slot | field | label | section |
|---|---|---|---|
| 1 | `velocity_start` | Start | Velocity Destination |
| 2 | `velocity_decay` | Decay | (Velocity Destination) |
| 3 | `velocity_cutoff` | Cutoff | (Velocity Destination) |
| 4 | `velocity_volume` | Volume | (Velocity Destination) |
| 5 | `modwheel_start` | Start | Modwheel Destination |
| 6 | `modwheel_cutoff` | Cutoff | (Modwheel Destination) |
| 7 | `modwheel_lfo` | LFO Depth | (Modwheel Destination) |
| 8 | `modwheel_pan` | Pan | (Modwheel Destination) |

All are bipolar except LFO Depth (0..1). The constructor gives the velocity amounts and
modwheel Start a "snap to zero" wheel behaviour.

## Not on any page

`editor_tab`, Scroll X/Y, Zoom X and Vertical Zone Editor Zoom are UI state. Zone
parameters (play/loop range, root key, zone tune, gain, pan, key and velocity ranges)
belong to the zone and sample editors, not to these pages.
