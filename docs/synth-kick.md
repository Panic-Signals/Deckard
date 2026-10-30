# Maschine's Kick drum synth: parameters and knob pages

A clean-room note on Maschine 3's internal **Kick** (`NI::MASCHINE::DATA::Kick`), as far as
DECKARD's pages and editor need it. The DSP port is `crates/engine/src/synth/kick.rs`; the
pages are `apps/viewer/src/drum_ui.rs`. Read from `out/decomp/kick/` (`Kick::Kick()` 0274,
`getParameterByPosition` 0415, `getTuneValueString` 0414) and the wide strings and option
vectors of `__GLOBAL__sub_I_Modules_DrumSynth_Objects_ALL.cpp` (`out/decomp/snare/0115.c`).

**[exact]**: read from code. **[inferred]**: matched by evidence.

## 1. Parameters

Pages: **Main** (positions 0–7), **Advanced** (8–15), **Modulation** (16–23), shared by all
drum synths **[exact]**. Engine options: Sub, Tronic, Dusty, Grit, Rasper, Snappy, Bold,
Maple, Push **[exact]**.

Members in constructor order, matched to the stored fields (`kick::FIELDS`) **[inferred]**:
the enum members fall on the enum fields, and every engine's knob positions below pick out
the fields Maschine's own Modulation tab (`KickModulationTab.qml`) shows for that engine.

| off | field | | off | field |
|---|---|---|---|---|
| 0xa8 | `engine` (enum) | | 0x128 | `dusty_kick_filter` |
| 0xb0 | `tune` | | 0x130 | `dusty_kick_noise` |
| 0xb8 | Tune Stepped (not stored) | | 0x138 | `grit_kick_gate` (bool) |
| 0xc0 | `decay` | | 0x140 | `grit_kick_aero_grind` |
| 0xc8 | `punch` | | 0x148 | `grit_kick_aero_amount` |
| 0xd0 | `bend` | | 0x150 | `acoustic_supporter_tune_skin` |
| 0xd8 | `impact` | | 0x158 | `acoustic_supporter_room_size` |
| 0xe0 | `velocity` | | 0x160 | `acoustic_supporter_room_amount` |
| 0xe8 | `sub_kick_p_env_decay` | | 0x168 | `acoustic_supporter_room_character` (enum) |
| 0xf0 | `sub_kick_thin_attack` (enum) | | 0x170 | `medio_snap` |
| 0xf8 | `sub_kick_color` | | 0x178 | `medio_punch_mode` (enum) |
| 0x100 | `sub_kick_attack_amount` | | 0x180 | `medio_punch_amount` |
| 0x108 | `clean_bd_drive_gain` | | 0x188 | `medio_punch_decay` |
| 0x110 | `clean_bd_drive_tone` | | 0x190 | `jungle_crispness_mode` (enum) |
| 0x118 | `clean_bd_bias` | | 0x198 | `jungle_crispness_decay` |
| 0x120 | `clean_bd_drive_dw` | | 0x1a0 | `jungle_crispness_amount` |

Enum options **[exact]**: Thin Attack "Thin", "Thick"; Room Character, Punch Mode and
Crispness Mode each "A", "B" (three two-entry vectors built right after the engine's).

## 2. Knob pages (`getParameterByPosition`) [exact]

| pos | Sub | Tronic | Dusty | Grit | Rasper | Snappy | Bold, Push | Maple |
|---|---|---|---|---|---|---|---|---|
| 0 | Engine | Engine | Engine | Engine | Engine | Engine | Engine | Engine |
| 1 | Tune | Tune | Tune | Tune | Tune | Tune | Tune | Tune |
| 2 | Decay | Decay | Decay | Decay | Decay | Decay | Decay | Decay |
| 3 | Bend | Bend | – | Bend | Punch | Snap | Punch | Skin Tune |
| 4 | Time | Impact | Impact | Impact | Impact | Impact | Impact | Impact |
| 5 | Attack (enum) | – | Filter | Gate | Crisp. Mode | Punch Mode | – | Room (enum) |
| 6 | Amount (only when Attack ≠ Thin) | – | Noise | Grind | Crisp. Decay | Punch Amount | – | Size |
| 7 | Color | – | – | Aero Amount | Crisp. Amount | Punch Decay | – | Room Amount |
| 8–11 | – | Gain, Tone, Bias, Mix | – | – | – | – | – | – |
| 16 | Velocity | Velocity | Velocity | Velocity | Velocity | Velocity | Velocity | Velocity |

## 3. Tune display

`getTuneValueString`: engines Dusty, Bold and Push (mask 0x144) show a percentage; the
others a note number with `"%.2f"` **[exact]**, whose per-engine mapping the decompiler
lost **[unknown]**. DECKARD shows a percentage for every engine for now.

## 4. Rasper, Snappy, Bold, Maple, Push

These five engines are not ported. DECKARD plays its own designs of them
(`synth/kick/bd.rs`), tuned against Maschine's renders. See docs/drum-engines-own.md.
