# Sampler research: what other samplers do

Gathered on 2026-11-11 for the sampler roadmap
(`docs/superpowers/plans/2026-11-11-sampler-roadmap.md`). It is a clean-room study:
manuals and observed behaviour only, never code or data tables from these products.

Sources:
- **Serato Sample:** the user manual, all 12 articles (support.serato.com, section
  115000138014).
- **Elektron Octatrack MKII:** the OS 1.40C manual
  (elektron.se/wp-content/uploads/2026/08/Octatrack-MKII-User-Manual_ENG_OS1.40C_260827.pdf).
- **Akai MPC:** the Standalone OS User Guide v3.10
  (cdn.inmusicbrands.com/Software/M310PCFIN26OCT/).
- **E-mu:**
  - the EOS 4.0 manual (medias.audiofanzine.com/files/e-mu-eos-4-manual-476441.pdf)
  - the Emulator III reference
  - the ESI-2000 manual
  - the SP-1200 owner's manual (archive.org/details/synthmanual-emu-sp-1200-owners-manual)
  - the Proteus X manual
  - the Rossum Morpheus manual, for the Z-plane filters
- **Ensoniq:** see the Ensoniq section below.

---

## Serato Sample (the beat-matching benchmark)

- **Model:** one source file per instance with **32 cues pointing into it**, not 32
  samples. A cue is set by catching the playhead: press an empty pad while the source
  plays. A cue with no length plays to the end of the file.
- **Analysis on load:** beatgrid, BPM, key. The manual is silent on the algorithms.
- **Beatgrid editing:**
  - Up to 128 tempo markers (a piecewise-constant tempo map); the grid is fixed at 4/4.
  - Tools: ×2/½, TAP, SET GRID START, ADJUST ◀▶ (stretches the segment under the
    playhead), and Ctrl-drag to slip the grid.
  - The grid is **saved into the audio file**, shared with Serato DJ.
- **Pitch 'n Time:**
  - Tempo and key always go through time-stretch: Sync is unavailable when it's off.
  - It is automatically **off for files under 3 s, and for files under 10 s with "808"
    in the name**, so one-shots stay natural.
- **Sync** locks the source BPM to the host.
- **Per-cue settings:**
  - Key Shift −24…+24 st, as an offset on top of the global key.
  - Time Stretch −75%…+300% on top of the source or synced tempo.
  - Level ±12 dB, one Filter knob (80 Hz–17 kHz), Attack and Release 0–10 s, Reverse,
    Favorite.
- **Quantize** snaps new cues to 1/8 beat.
- **Play modes (global):**
  - **Hold:** plays while held; release on note-off; at a cue end it waits silent while
    held.
  - **Trigger:** one-shot; the release starts once the attack is done, so A/R works as a
    fixed envelope.
  - **No loop modes.**
- **Mono/Poly** with up to 32 voices. Keyboard mode plays one cue chromatically, with Glide
  (Trigger or Sustain legato), Mono only.
- **Autoset (re-rolls everything except Favorited cues; can be undone):**
  - **Find Samples:** random cues, placed on the beat, inside regions judged good for
    sampling.
  - **Set Slicer:** 32 consecutive equal cues from the playhead, 1/16–16 beats each.
  - **Set Random.**
  - **Key Shift Pad:** one cue spread chromatically across the pads.
  - **Serato DJ:** imports the file's hot cues.
- **Stems:** real-time separation into 4 stems, with mute/solo per stem.
- **Routing:** each cue can go to one of about 16 extra outputs.
- **Editing:** multi-select cues and edit them together; drag a cue to another pad to move
  or swap it, Alt-drag to copy it. 100 undo steps.

## Elektron Octatrack MKII (live sample manipulation)

- **Recorders kept apart from playback:**
  - 8 track recorders, each with a buffer. A buffer is an ordinary Flex slot any track can
    play.
  - Sources per recorder: inputs AB/CD, plus SRC3 = MAIN / CUE / a single track
    (resampling).
  - **Recorder trigs** sit on sequencer steps, each with its own source mask, in
    one-shot (armable) or repeating kinds.
  - RLEN is set in steps.
  - TRIG modes: ONE (records RLEN), ONE2 (pressing again stops it), HOLD.
  - FIN/FOUT fades are set in steps.
  - **QREC** starts a recording at the next N steps or at the end of the pattern. **QPL**
    quantizes manual playback starts the same way.
  - **Same-step rule:** a play trig on the step where a recording starts plays the audio
    being captured right now.
  - Buffers are lost at power-off, and users hate it. Make saving takes obvious.
- **Tempo attributes:**
  - ORIGINAL TEMPO, TRIM LEN (bars) and LOOP LEN (bars) are linked: edit one and the
    others follow. A priority flag marks which value you know.
  - The first BPM guess is folded into 85–170, then **numbers in the filename fix ×2/½
    errors**.
  - **CAL BPM FROM SELECTION** assumes the selection is a musical length.
  - Per-sample TIMESTRETCH: OFF / NORMAL / BEAT, plus a transient sensitivity (TSNS).
  - The track's TSTR is AUTO (follow the sample) or a forced setting.
  - **Sample tempo nudge** moves only the samples, not the sequencer.
  - Users find its stretch grainy, so keep stretching optional.
- **Slices:**
  - Up to 64, which may **overlap**, each with its own start, loop and end.
  - Grid presets: 2, 3, 4, 6, 8, 12, 16, 24, 32, 48 or 64, with zero-crossing alignment.
  - Slices belong to the sample *slot*; they can be saved to a sidecar file next to the
    sample.
  - With SLIC on, STRT selects a slice and LEN plays N consecutive slices.
  - **Create linear / random locks** rewrites the trigs to play the slices in order or at
    random.
  - Pad presses in SLICES mode are recorded as slice locks.
  - **No transient auto-slicing.**
- **Loop modes:** OFF, ON, PINGPONG. Negative RATE plays the sample in reverse.
- **Sequencer features that affect sample playback:**
  - Trigless trigs: p-locks plus an LFO/envelope retrigger without retriggering the sample.
  - Lock trigs.
  - Slide trigs.
  - Chromatic mode that changes pitch without restarting the sample.
  - RTRG/RTIM retrigs per track plus TRIG COUNT per trig.
  - PLAYS FREE tracks: launched from pads, quantized, ONE / ONE2 / HOLD (a light clip
    launcher).
- **Scenes and the crossfader (its killer feature):**
  - 16 scenes per part. Each scene locks any parameters; the fader interpolates every lock
    between scene A and scene B. A parameter locked on only one side moves between that
    lock and its base value.
  - While the fader moves, scene locks override step p-locks.
  - The scene on the inactive side can be swapped live.
  - **Equal-power XVOL/XLV** volume locks, so there's no dip at the centre.
  - Scene mute.
  - Arranger rows set scenes A and B.
  - Users want curve settings on the fader.
- **Pickup looper:**
  - The first loop recorded becomes the master and **sets the project BPM**. Later loops
    are slaves at ×N length, locked in phase.
  - Overdub vs. GAIN-only (fade-out), plus replace.
  - Directions FWD / REV / ping-pong; loop multiply.
  - The sequencer can sync to the loop (start at its wrap). There is a foot-controller MIDI
    map.
  - Complaints: clicks, BPM doubling at low tempos, and loops can't be sequenced.
- **LFOs and modulation:**
  - 3 LFOs per track; the **LFO Designer** draws a 16-step wave, stepped or interpolated.
  - LFO → STRT in slice mode gives the famous stutter.
  - Start can't be modulated on streamed (Static) samples.
- **AMP envelope:** ATK, HOLD (in steps), REL. Modes: from the current level vs. from zero.
- **Neighbor machines:** a track takes the previous track's output, for FX chains.

## Akai MPC (MPC 3.10)

- **Warp on each layer:**
  - Warp, plus BPM Sync (follow the project tempo live) or a fixed Stretch, plus an
    Algorithm (Default, Pro Ten, Repitch, or the FFT "Super" in the paid Pro Pack;
    zPlane élastique offline).
  - Warp off = tape behaviour (pitch changes the length).
  - The detected key is shown next to Semi.
  - The manual warns that warp is CPU-heavy. **Flatten Pad** renders the layers, FX and
    warp into one sample. Users mostly warp to find the ratio, then render offline.
- **Sample Edit:**
  - BPM with ×2 and /2, Detect / Tap.
  - Key detection (root plus major/minor).
  - **Match** re-tunes the sample to the project tempo for N beats; **To Project** sets the
    project tempo from the sample.
  - **Patched Phrase** makes a new sample that plays at the project tempo.
  - Browser audition warps to the project tempo, and **Sync** starts it at the next bar.
- **No warp markers anywhere**, and **no retrospective audio** (Retro Rec is MIDI only).
  Both are gaps Deckard could fill.
- **Chop:**
  - Methods: Manual (tap pads while it plays), Threshold plus Min Time, Regions (N equal),
    BPM (bars / beats / division).
  - Link Slices off lets slices overlap or leave gaps. 0-snap snaps to zero crossings.
  - **Non-destructive:** a pad layer's Slice field is All / Pad / Slice N, a bookmark into
    one shared sample.
  - **Convert** makes a new track with **Create Events** (the slices played in order over N
    bars).
  - Process menu: Stereo→Mono (L / R / Sum), Bit Reduce, fades with 6 curves, Stems.
- **Layers:**
  - 8 per pad; each can be a sample or an oscillator.
  - Layer Play: **Cycle** (round robin), Velocity, **Random with a seed**, **Crossfade**
    (driven by the mod matrix; vector and wave-sequencing).
  - Per-layer random pitch, level, pan and offset.
  - Velocity → start offset (a negative value delays the start).
- **Slice Motion:** each hit steps to the next slice, or a random one, over N slices.
  Round robin from one file.
- **Playback modes:**
  - One Shot / Note On / Note Off.
  - Pad Loop: Forward / Reverse / Alternating, with a **Repeats** count.
  - **Play Offset** slides the start/end/loop window by a fixed amount.
  - **Tail Length** adds a tail to a chop that ends abruptly.
- **Recording:**
  - Inputs or a lossless resample, up to 60 min.
  - Methods: Sample, Slice (tap pads to place slices), **Pad Tap** (each pad press moves
    the recording to that pad), **Pad Hold**.
  - Keep dialog: track, pad and root, plus Add Event at the moment recording began.
- **Keygroups:**
  - 128 keygroups × 8 layers.
  - The Advanced engine: 2 filters (serial or parallel), 4 DAHDSR envelopes (loopable,
    tempo-synced), 2 + 2 LFOs, a 32-slot mod matrix, Unison or Harmonizer stacking.
- **Auto Sampler:** note stride, up to 8 velocity layers, loop with a crossfade.
- **Pad tools:**
  - 16 Levels by layer or slice.
  - Simultaneous Play (one pad triggers up to 4 others), Mute Targets (choke), 32 mute
    groups.
- **Vintage mode:** MPC3000 / MPC60 applied to the whole output. **No SP-1200 mode.**
- **Firmware:**
  - Official images: `MPC-3.10.0-Gen1-update.img` (167 MB) and `…Gen2-update.img`
    (377 MB), from cdn.inmusicbrands.com/Software/M310PCFIN26OCT/.
  - Gen 1 is the custom `AZ01` container, holding an xz-compressed **ext4** root (about
    1.8 GB; 32-bit ARM, RK3288). Tooling: github.com/TheKikGen/MPC-LiveXplore.
  - Gen 2 is a signed Rockchip FIT image; its root filesystem type is not identified.
  - MPC Beats is now a free tier of MPC 3, behind an account.
  - **Not downloaded or analysed.**

## E-mu (EOS 4, Emulator III, SP-1200, Emulator X)

- **Structure:** Sample → Voice (sample zones) → Preset, with up to 256 voices per preset.
  Each voice has **three crossfade/switch windows**:
  - key (positional crossfade)
  - velocity (switch or crossfade)
  - **realtime:** a controller range per voice, whose controller is chosen by routing a
    PatchCord to `AmpXfd`
- **Crossfade Random:** one random value per key, routed to AmpXfd with split ranges, gives
  random round robin.
- **Links:** stack presets with their own key and velocity windows, transpose and volume.
  *Subsume* merges a linked preset's voices into the current one.
- **Per-voice settings:**
  - Chorus (amount, width, **ITD** inter-aural delay).
  - Delay 0–10 s from note-on.
  - Start offset (can be modulated).
  - Glide with a curve.
  - **8 solo modes:** Melody last/low/high, Synth last/low/high, Fingered Glide, and
    Multiple Trigger.
  - **Latch mode.**
  - **Assign groups:** voice bins Poly 16/8/4/2 or Mono A–I; voices steal only within their
    bin (choke).
- **Sample Retrigger:** a modulation destination; a negative-going zero crossing restarts
  the sample. Driven by an LFO for stutter, or by noise for random retriggers.
- **Z-plane filters:**
  - Two complex filter frames, A and B, interpolated by Morph. Frequency is the starting
    Morph position; the second parameter is Q, Gain, Body, Peak or Expression.
  - EOS has 21 types: LP 12/24/36, HP, BP, contrary BP, swept EQs, phasers, flanger,
    vocal formants, **Dual EQ Morph**, **Peak/Shelf Morph**.
  - The Morpheus "cube" has 8 corners on the axes Frequency × Morph × Transform. Emulator
    X2 has 55 responses plus a Morph Designer (1–6 sections, morph between two settings).
  - Morphing filters "allow high resonance without losing bass".
- **PatchCords:**
  - 24 per voice (36 in Proteus X), each with a ± amount. Cords can modulate other cords'
    amounts.
  - Source variants: `+` unipolar, `~` centred, `<` inverted.
  - Sources: kRandom (per-note random), noise, **clock divisions** (they trigger envelopes,
    reset LFOs, retrigger samples).
  - **Processors:** summing amp, switch, two lags, abs, diode, flip-flop, quantizer, 4×
    gain, DC offset.
- **Envelopes:** three 6-stage rate/level envelopes per voice (Atk1/Atk2/Dcy1/Dcy2/Rls1/
  Rls2). The Filter and Aux envelopes can go negative.
- **LFOs:** 2 per voice. Shapes include pulses, Sine 1,2 / 1,3,5, and **pattern LFOs**
  (Octaves, Fifth+Octave, Sus4 trip, Neener: musical step patterns on pitch).
- **Function generators** (Morpheus, Emulator X2/X3):
  - 1–64 steps; rate free or BPM-synced.
  - Directions: forward / reverse / ping-pong / one-shot / random / Brownian.
  - Per-step smoothing, and quantizing to semitone / octave / triad / scale.
- **Sample tools:**
  - Auto Loop at a fraction of the sample (whole, ½, ⅓, ¼, ⅛) by auto-correlation with a
    crossfade.
  - Auto Correlate: refine the loop on repeated presses. Fix Size keeps the loop length.
  - **Loop Compression** evens out level inside a loop.
  - Crossfade loop (equal power or linear).
  - Sample Calculator: samples per cycle, for single-cycle loops.
  - Paste modes Insert / Mix / Xfade Mix.
  - Taper with 4 curves.
  - Rate convert and digital tune with a **Crunchy / Smooth / Creamy** anti-alias choice.
  - **Transform Multiply:** convolve two samples.
  - **Doppler paths:** fly-by, near↔far, circles, random.
  - Time Compression and Pitch Change, 50–200%, with 12 algorithm types (deep, bass,
    tight for drums, …).
  - Bit converter.
- **Beat Munger (E4 Ultra):**
  - Detects beats, then real-time audition before rendering.
  - Tempo Adjust.
  - Swing ± (anti-swing removes swing) in Fast / Norm / Slow modes.
  - **Grunge** (splice crossfade) and **Comb** (splice spacing: echo or comb effects).
  - An **8th-note mute grid** that can change the time signature (4/4 → 7/8).
- **Emulator III:**
  - **Scrub wheel** (scrub through a sample with the pitch wheel while editing).
  - Audio inputs as drum triggers.
  - Offline digital delay and "Ping Pong" pan tools.
- **Emulator X2:** **Twistaloop** (real-time tempo-locked slicing, ½–2× speed, sections
  switched by MIDI); Synth Swipe (auto-sampler).
- **SP-1200:**
  - **12-bit, 26.04 kHz at a single playback rate.**
  - **Drop-sample pitch:** samples are skipped or repeated, with no interpolation and no
    reconstruction filter, so moving aliasing is the sound.
  - Each pad's slider is **tune OR decay**.
  - **Multipitch / Multilevel:** one sound spread over 8 pads.
  - Output filters: channels 1–2 a dynamic SSM2044 4-pole LP; channels 3–6 fixed LP;
    channels 7–8 unfiltered.
  - Users: the grit comes from pitching down plus aliasing, not from 12-bit alone.

## Ensoniq (EPS-16 Plus, ASR-10/88, ASR-X)

Sources:
- **ASR-10 Musician's Manual:** read in full (archive.org/details/ensoniq-asr-10-archive-001).
- **EPS-16 Plus Musician's Manual:** manualslib.com/manual/1168266, Edit/Wave pp. 6-4–6-7.
  It uses the same parameter wording as the ASR-10.
- **EPS Advanced Applications Guide:** archive.org.
- **Transoniq Hacker #115 and #155:** buchty.net.

**Transwaves**
- **On the samplers (EPS-16+, ASR-10) it is a wave-modulation type, `TRANSWAV`, not a loop
  mode.**
  - The loop jumps in whole loop lengths: loop 0–100, then 100–200, then 200–300, and so on.
  - MOD AMT (±99) is the number of jumps across the source's range.
  - Any of the 15 mod sources can drive it.
  - Jumps are **stepped, with no crossfade**, so it "flutters" unless the frames are prepared
    carefully.
- **The original EPS lacks it.** The samplers have no ROM Transwaves; users build their own:
  - **By hand:** sample a slow resonant sweep, set a one-cycle loop at a zero crossing, then
    TRANSWAV on the mod wheel or an envelope.
  - **By synthesis:** copy a 128-sample square into 8 layers, invert a different range in each
    to make pulse widths, then concatenate the frames.
  - With an envelope as the source it works as a crude real-time time-stretch.
- **On the VFX, SD-1 and TS-10 synths:** ROM wavetables of about 100 frames, scanned from a
  start wave plus modulation (PPG-style).

**Wave-mod types (EPS-16+, ASR-10)**

| Type | What it moves |
|---|---|
| OFF | nothing |
| LOOP POS | the loop window, smoothly, at a fixed length |
| START | the sample start |
| START+LP | start and loop together |
| LOOPSTRT | the loop start |
| LOOP END | the loop end |
| LPSTRT-X | the loop start, with pitch correction as the loop length changes |
| TRANSWAV | the loop, in whole-frame jumps |

Each type takes SRC, MOD AMT and RANGE (RANGE is a multiplier, AMT fine-tunes it).
The original EPS had only LOOP, START and BOTH.

**Structure**
- An Instrument holds **8 Layers** and up to 127 WaveSamples. **One WaveSample per key per
  layer**, so a key can sound up to 8 voices.
- The key range is set per WaveSample; the velocity window per Layer.
- **Patch Select:** 2 buttons give 4 states, each an 8-bit mask of the layers that are on.
  - The buttons are momentary; holding them and pressing Instrument locks a state in.
  - MIDI CC 70.
  - Per-track mode: LIVE, one of the 4 fixed states, or HELD (latches the first key's patch).
- **Layer masks for key-down and key-up:** a layer can fire on release (a harpsichord thunk).
- **Legato Layer:** overlapping notes play a different layer (attack vs. no-attack sustain).
- **Layer Delay:** 0–5000 ms, with a ± velocity amount.
- Glide modes: Off, Legato, Pedal, Trigger, Minimode.
- User pitch tables (8 per instrument, ± cents per key).
- Layer restrike decay.

**Voice**
- **Playback modes:** forward, backward, loop forward, loop bidirectional, and **loop and
  release** (loops while held, plays through to the end on release).
- The loop end has 1/16-sample precision.
- **Filters:** two in series, 4 poles in total (3LP+1LP, 2LP+2LP, 3LP+1HP, 2LP+2HP), with no
  resonance (a common complaint).
- **Three envelopes** (pitch, filter, amp), each with 5 levels and 5 times plus a second
  release.
  - **Two level sets, HARD VEL and SOFT VEL;** velocity morphs between them along a curve.
  - Attack time scaled by velocity; times scaled by key.
  - Modes: Normal, Finish, and **Repeat** (loops stages 2–5).
- **Volume-mod trapezoid:** any source (velocity, key, wheel, pressure) with breakpoints A–B
  fade-in and C–D fade-out, curve CROSSFADE or LINEAR. This is how crossfades are built from
  any source.
- **15 mod sources:** LFO, noise, ENV1, ENV2, pressure+velocity, three velocity curves,
  keyboard, pitch, wheel, pedal, external controller, pressure, wheel+pressure.

**Editing (ASR-10)**
- Loop crossfades: Cross Fade Loop (3–6 dB scale, fade zone %), Reverse, **Ensemble**
  (a 100% zone), **Bowtie**, Bidirectional, **Synthesized Loop** ("loop anything", with a
  smoothness setting), and Make Loop Longer.
- Auto-loop finding, which reviewers rated among the best.
- Time compress/expand 50–250% with a quality setting; rate conversion.
- Mix, **Merge** (crossfade one sample into another) and Splice two wavesamples, with pitch
  matching.
- Data tools: copy, replicate, reverse, invert, add, scale.
- **Every destructive command auditions before you keep it: `KEEP=OLD/NEW`.**

**Effects and resampling**
- **EPS-16+:** 13 ROM effects; each voice is sent to bus 1–3; the effect is stored per
  instrument.
  - **Resample With Effect** renders a wavesample through the effect. The manual's tip:
    reverse, add reverb, resample, reverse back for a "pre-verb".
- **ASR-10:** 50 algorithms. Record sources: **INPUT+FX** (record the input through the
  effect) and **MAIN-OUT** (resample the whole output, sequencer included).
- **ASR-X:** resampling is the default way of sampling; 16 layers; a resonant 3-pole filter.
- **Users love:** Transwaves (a "Transwave synthesizer where you make the waves yourself"),
  Patch Select's instant 4 variants, resampling through effects, and clean transposition
  far down (fixed-rate output).
- **Complaints:** no resonance on the voice filters, no waveform display, fiddly manual
  looping.
