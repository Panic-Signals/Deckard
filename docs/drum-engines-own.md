# DECKARD's own drum engines

Sixteen drum engines are DECKARD's own designs, not ports of Maschine's DSP:

- **Kick:** Rasper, Snappy, Bold, Maple, Push (`synth/kick/bd.rs`)
- **Snare:** Airy, Vintage, Chrome, Iron, Breaker (`synth/snare/modal.rs`)
- **Tom:** Floor, High (`synth/tom/modal.rs`)
- **Perc:** Kettle (`synth/perc/kettle.rs`)
- **Cymbal:** Crash, Ride (`synth/cymbal/metal.rs`)
- **Hi-hat:** Hybrid (`synth/hihat/hybrid.rs`)

## Why

Maschine builds these engines from its modal-drum kit: about 270 KB of state per core, around
40 interacting classes, and hand-vectorised NEON. An earlier attempt translated their machine
code to Rust automatically. That was removed because it isn't a clean-room implementation. A
hand port was estimated at millions of tokens, so these engines are designed instead. The
other engines of each drum remain ports.

## How they work

All sixteen share `synth/modal_lite.rs`, a strike voice with four parts:

- **Body oscillator:** a sine whose pitch falls to its resting note, held and then decaying.
- **Modes:** up to 64 decaying modes (frequency, level, decay, pan), bent together by a pitch
  envelope. Skins start them in phase; metal starts them at scattered phases.
- **Click:** a short burst of filtered noise at the strike.
- **Noise layer:** white, pink or one of the modal noises (`synth/noise.rs`, DECKARD's own
  stand-ins for Maschine's recordings), band-passed, under its own envelope.

Each engine maps Maschine's real parameters (Tune, Decay, Impact, Punch, Skin Tune, Spectra,
Tension, Damp, Density, Bell, …) onto a strike. The note and the velocity scaling work as
measured in Maschine.

## Tuning against Maschine

Each engine was rendered in Maschine (`maschine-ref`) and in DECKARD (`synth-null`) at its
defaults: note 60, velocity 100. They were compared on loudness, time to −60 dB, pitch over
time, brightness and the strongest partials, with extra Maschine renders at parameter extremes
to set the ranges. Some behaviour found this way:

- The modal Kicks treat Tune and the note as one control: Tune 0 plays as note 48 and 1 as
  note 72, rising ×1.69 per 12 steps.
- The modal Snares span an octave each way over Tune. Spectra adds resonances at its own pitch.
- Kettle moves about an octave per 20 semitones of note.

| engine | RMS (Maschine / DECKARD) | −60 dB (ms) | late pitch (Hz) | early brightness (Hz) |
| kick-rasper | -29.1 / -29.3 dB | 445 / 415 | 79 / 79 | 172 / 164 |
| kick-snappy | -29.2 / -29.4 dB | 480 / 470 | 65 / 64 | 139 / 162 |
| kick-bold | -30.6 / -31.4 dB | 385 / 385 | 67 / 67 | 181 / 102 |
| kick-maple | -26.7 / -27.3 dB | 380 / 335 | 45 / 45 | 111 / 112 |
| kick-push | -26.1 / -24.9 dB | 470 / 485 | 53 / 53 | 144 / 125 |
| snare-airy | -29.4 / -28.8 dB | 405 / 480 | 165 / 185 | 246 / 196 |
| snare-vintage | -30.7 / -31.8 dB | 245 / 305 | 316 / 231 | 257 / 281 |
| snare-chrome | -34.2 / -33.0 dB | 360 / 405 | 268 / 270 | 564 / 387 |
| snare-iron | -34.2 / -34.0 dB | 260 / 280 | 230 / 231 | 600 / 287 |
| snare-breaker | -38.6 / -38.4 dB | 350 / 410 | 273 / 272 | 812 / 375 |
| tom-floor | -33.1 / -32.7 dB | 865 / 805 | 67 / 70 | 111 / 103 |
| tom-high | -29.1 / -30.6 dB | 635 / 650 | 124 / 122 | 225 / 198 |
| perc-kettle | -29.6 / -30.2 dB | 1630 / 1440 | 160 / 161 | 137 / 161 |
| cymbal-crash | -33.4 / -34.7 dB | 1200 / 1255 | 6434 / 4625 | 4738 / 6021 |
| cymbal-ride | -41.5 / -41.3 dB | 3420 / 3245 | 1863 / 2405 | 3048 / 2247 |
| hihat-hybrid | -43.4 / -43.4 dB | 905 / 870 | 2542 / 6466 | 9427 / 7716 |

The figures are at the defaults, so other settings may differ more. The Hi-hat's Hybrid and the Cymbal's Ride were retuned
on 2026-10-27 against further renders (each knob's extremes, velocities 30 and 127, notes
48 and 72), scored by the difference of half-octave spectra in four or five time windows:
Hybrid 411 → 127, Ride 372 → 200 (summed over 21 and 20 renders). Both had stopped at the
upper bands (Hybrid 3–14 kHz, Ride 1.5–10 kHz) where Maschine's spread from a few hundred
Hz to 20 kHz; the Hybrid's strike is now a broad bright burst, and the Ride's wash swells
in (`modal_lite::Patch::swell`) instead of starting on a click. The table's figures predate
that. "Late pitch" is the
strongest frequency 0.2–0.5 s in, which picks different partials of the metal engines. They
sound close, not identical: there is no sample-level null with Maschine.
