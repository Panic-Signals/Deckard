//! The Poly Synth's engine state (`ni::p5::P5`, docs/synth-poly-synth.md §3): the
//! global (per-instrument) values, the 32 voice slots, the pitch/rate table, and the
//! constructor sequence Maschine runs (`P5::P5(44100)` → `initAudioMemory` →
//! `setProgram(default)` → `allSoundOff`, then `PolySynthProcessor` sets 32 voices and
//! clears again, then the host sample rate).
//!
//! Field comments give the member's byte offset in `P5` (voices: in the 0x124-byte
//! voice slot at `P5 + 0x310 + i·0x124`).

/// Voice slots `P5` keeps (the allocation lists, MIDI note table and voice array are
/// all 32 long; `Voices` picks how many of them play).
pub(crate) const SLOTS: usize = 32;

/// Entries in the exponential pitch/rate table (`initAudioMemory`, P5 + 0x2790).
pub(crate) const TABLE: usize = 2400;

/// "No note" in the voice-note table (P5 + 0x79a8): `-1`. Released notes are stored as
/// `-300 - note` so a re-struck note finds its old voice.
pub(crate) const NO_NOTE: i32 = -1;

/// The constant program `P5::P5` loads (`setProgram`, all 62 normalized values, in
/// `ParameterId` order). Its voice count is 5.
pub(crate) const DEFAULT_PROGRAM: [f32; 62] = [
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.2,
    1.0,
    1.0,
    1.0,
    0.0, // 0x00
    1.0,
    0.8,
    0.09,
    0.0,
    0.88,
    0.3,
    0.2,
    0.0,
    0.01,
    0.6, // 0x0a
    1.0,
    0.3,
    0.5,
    0.0,
    0.0,
    0.0,
    0.0,
    0.2,
    0.1,
    0.0, // 0x14
    1.0,
    0.0,
    0.5,
    0.0,
    1.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0, // 0x1e
    0.0,
    1.0,
    0.1,
    0.0,
    0.05,
    0.4,
    0.7,
    0.4,
    1.0,
    1.0, // 0x28
    0.0,
    0.0,
    0.0,
    0.0,
    1.0,
    0.5,
    1.0,
    0.0,
    0.5,
    1.0 / 12.0, // 0x32
    0.0,
    0.0, // 0x3c
];

/// How `setParameterValue` quantizes a parameter (`ni::p5::Parameter::Type`, from the
/// definitions map built in `___cxx_global_var_init.1`): 0 = switch (`< 0.5` → 0, else
/// 1), 1 = continuous, 2 = three-way (`< 0.33333` → 0, `< 0.66666` → 0.5, else 1, the
/// comparisons in `f64`).
pub(crate) const PARAM_TYPE: [u8; 62] = [
    1, 1, 0, 0, 0, 1, 0, 0, 1, 0, // 0x00
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 0x0a
    1, 1, 0, 1, 0, 0, 0, 1, 1, 0, // 0x14
    0, 0, 1, 0, 0, 1, 1, 0, 0, 0, // 0x1e
    0, 0, 1, 2, 1, 1, 1, 1, 0, 0, // 0x28
    0, 0, 2, 0, 1, 1, 1, 1, 1, 1, // 0x32
    0, 0, // 0x3c
];

/// A voice's envelope (amplifier at voice + 0x2c, filter at + 0x48). Attack ramps
/// `value` by `inc` to `target`; decay and release multiply `rem` by `coef` each sample
/// and give `value = rem + floor`.
#[derive(Clone, Copy, Default)]
pub(crate) struct Env {
    /// +0x00
    pub value: f32,
    /// +0x04: what decay/release approach.
    pub floor: f32,
    /// +0x08: the attack's peak.
    pub target: f32,
    /// +0x0c: attack increment per sample.
    pub inc: f32,
    /// +0x10: distance still to go (`value − floor`).
    pub rem: f32,
    /// +0x14: per-sample decay/release factor.
    pub coef: f32,
    /// +0x18: 0 attack, 1 decay/sustain, 2 release.
    pub stage: i32,
}

/// One voice slot (0x124 bytes in Maschine).
#[derive(Clone, Copy)]
pub(crate) struct Voice {
    /// +0x00: velocity of the held note (> 0 while the key is down, 0 after release).
    pub velocity: f32,
    /// +0x04: the note it plays (`f32`), or the free-voice markers.
    pub note: f32,
    /// +0x08: pitch in semitones (MIDI note; glides in unison).
    pub pitch: f32,
    /// +0x0c: unison spread / random drift added to the glide pitch.
    pub spread: f32,
    /// +0x10: Osc B random detune.
    pub det_b: f32,
    /// +0x14: Osc B pitch offset (semitones + Low + fine + detune + master tune).
    pub off_b: f32,
    /// +0x18: Osc A random detune.
    pub det_a: f32,
    /// +0x1c: Osc A pitch offset.
    pub off_a: f32,
    /// +0x20: cutoff random detune.
    pub det_cut: f32,
    /// +0x24: smoothed cutoff + `det_cut`.
    pub cut: f32,
    /// +0x28: voice runs (nonzero) or is skipped.
    pub active: i32,
    /// +0x2c
    pub amp: Env,
    /// +0x48
    pub flt: Env,
    /// +0x64: filter cutoff (Hz) from the table.
    pub cutoff: f32,
    /// +0x68: Osc B frequency (Hz).
    pub freq_b: f32,
    /// +0x6c: Osc B phase (−1..1).
    pub phase_b: f32,
    /// +0x70: Osc B pulse level (0/1).
    pub state_b: i32,
    /// +0x74: Osc B phase at which the triangle corner is still due (0 after a wrap,
    /// 2 once handled).
    pub corner_b: f32,
    /// +0x78: last `phase − pulse width` of Osc B.
    pub prev_b: f32,
    /// +0x7c: write position in the 4-sample correction rings.
    pub idx_b: i32,
    /// +0x80: Osc B audio output ring (band-limiting corrections land here).
    pub ring_b: [f32; 4],
    /// +0x90: Osc B poly-mod output ring.
    pub ring_bm: [f32; 4],
    /// +0xa0: Osc B audio, delayed.
    pub out_b: [f32; 2],
    /// +0xa8: filter envelope, delayed (for poly mod).
    pub env_d: [f32; 2],
    /// +0xb0: per-voice drift factor `1 + 0.1·detune·r`.
    pub drift: f32,
    /// +0xb4: Osc A frequency (Hz).
    pub freq_a: f32,
    /// +0xb8: Osc A phase.
    pub phase_a: f32,
    /// +0xbc: Osc A frequency, delayed.
    pub freq_a_d: [f32; 2],
    /// +0xc4: Osc A phase increment, smoothed.
    pub inc_a: f32,
    /// +0xc8: Osc A pulse level.
    pub state_a: i32,
    /// +0xcc: where in the sample Osc B wrapped (hard sync), delayed; −1e20 = none.
    pub sync_d: [f32; 2],
    /// +0xd4: last `phase − pulse width` of Osc A.
    pub prev_a: f32,
    /// +0xd8
    pub idx_a: i32,
    /// +0xdc: Osc A output ring.
    pub ring_a: [f32; 4],
    /// +0xec: cutoff, delayed.
    pub cut_d: [f32; 2],
    /// +0xf4: modulated cutoff, delayed.
    pub cutm_d: [f32; 2],
    /// +0xfc: filter feedback (shaped output of the previous sample).
    pub fb: f32,
    /// +0x100: first 2-pole stage state.
    pub z1: [f32; 2],
    /// +0x108: second 2-pole stage state.
    pub z2: [f32; 2],
    /// +0x110: one-pole lowpass whose difference removes DC.
    pub dc: f32,
    /// +0x114: amplifier envelope, delayed 4 samples.
    pub amp_d: [f32; 4],
}

impl Default for Voice {
    fn default() -> Voice {
        Voice {
            velocity: 0.0,
            note: 0.0,
            pitch: 0.0,
            spread: 0.0,
            det_b: 0.0,
            off_b: 0.0,
            det_a: 0.0,
            off_a: 0.0,
            det_cut: 0.0,
            cut: 0.0,
            active: 0,
            amp: Env::default(),
            flt: Env::default(),
            cutoff: 0.0,
            freq_b: 0.0,
            phase_b: 0.0,
            state_b: 0,
            corner_b: 0.0,
            prev_b: 0.0,
            idx_b: 0,
            ring_b: [0.0; 4],
            ring_bm: [0.0; 4],
            out_b: [0.0; 2],
            env_d: [0.0; 2],
            drift: 0.0,
            freq_a: 0.0,
            phase_a: 0.0,
            freq_a_d: [0.0; 2],
            inc_a: 0.0,
            state_a: 0,
            sync_d: [0.0; 2],
            prev_a: 0.0,
            idx_a: 0,
            ring_a: [0.0; 4],
            cut_d: [0.0; 2],
            cutm_d: [0.0; 2],
            fb: 0.0,
            z1: [0.0; 2],
            z2: [0.0; 2],
            dc: 0.0,
            amp_d: [0.0; 4],
        }
    }
}

/// `ni::p5::P5`: everything the instrument keeps.
pub(crate) struct P5 {
    /// +0x000
    pub sr: f32,
    /// +0x004: smoothing length, `max(⌊0.02·sr + 0.5⌋, 1)` samples.
    pub smooth_len: i32,
    /// +0x020: voices that play (Voices parameter).
    pub voices: u32,
    /// +0x024: the normalized program value of each `ParameterId` (clamped 0..1).
    pub program: [f32; 62],
    /// +0x120: sustain pedal / Hold latch is down.
    pub sustain: bool,
    /// +0x124: last voice of the allocation list, −1 when every voice holds a key.
    pub tail: i32,
    /// +0x128: first (next to steal) voice of the allocation list.
    pub head: i32,
    /// +0x12c: last note played in unison.
    pub last_note: i32,
    /// +0x130: control counter (starts at 100; the smoothing block runs when it ends).
    pub ctl_count: i32,
    /// +0x134: smoothing steps left.
    pub smooth_left: i32,
    /// +0x138: glide rate (semitones/s).
    pub glide_rate: f32,
    /// +0x13c: glide target pitch (unison).
    pub glide_target: f32,
    /// +0x140: glide pitch now.
    pub glide: f32,
    /// +0x144: glide step per sample (0 = arrived).
    pub glide_step: f32,
    /// +0x148: LFO phase (0..1).
    pub lfo_phase: f32,
    /// +0x14c: LFO rate (Hz; or beats·multiplier when synced).
    pub lfo_rate: f32,
    /// +0x150: LFO saw level.
    pub lfo_saw: f32,
    /// +0x154: LFO triangle level.
    pub lfo_tri: f32,
    /// +0x158: LFO pulse level.
    pub lfo_pulse: f32,
    /// +0x15c: LFO is in its second half.
    pub lfo_half: i32,
    /// +0x160: all three LFO shapes on: sample & hold of the noise.
    pub lfo_sh_on: f32,
    /// +0x164: held noise value.
    pub lfo_sh: f32,
    /// +0x168: LFO output when no shape is on.
    pub lfo_none: f32,
    /// +0x16c: noise generator state (shift register).
    pub rng: u32,
    /// +0x170: noise amplitude `√(sr/44100)`.
    pub noise_amp: f32,
    /// +0x174: noise lowpass coefficient (LFO noise source).
    pub noise_coef: f32,
    /// +0x178: lowpassed noise.
    pub noise_lp: f32,
    /// +0x17c: LFO <> Noise mix.
    pub lfo_noise: f32,
    /// +0x180: mod wheel ×5, smoothed.
    pub wheel: f32,
    /// +0x184: its step.
    pub wheel_step: f32,
    /// +0x188: Modulation Amount, smoothed.
    pub mod_amount: f32,
    /// +0x18c
    pub mod_amount_step: f32,
    /// +0x190: amplifier decay factor.
    pub amp_decay: f32,
    /// +0x194: amplifier release factor.
    pub amp_release: f32,
    /// +0x198: amplifier release time (table units, before the Release switch).
    pub amp_release_raw: f32,
    /// +0x19c: amplifier attack increment.
    pub amp_attack: f32,
    /// +0x1a0: amplifier sustain level ×0.95.
    pub amp_sustain: f32,
    /// +0x1a4: filter envelope decay factor.
    pub flt_decay: f32,
    /// +0x1a8: filter envelope release factor.
    pub flt_release: f32,
    /// +0x1ac
    pub flt_release_raw: f32,
    /// +0x1b0
    pub flt_attack: f32,
    /// +0x1b4: filter sustain ×0.98.
    pub flt_sustain: f32,
    /// +0x1b8: Osc B Pulse on (0/1).
    pub pulse_on_b: i32,
    /// +0x1bc: Osc B Saw level (0.5/0).
    pub saw_b: f32,
    /// +0x1c0: Osc B Triangle on.
    pub tri_on_b: i32,
    /// +0x1c4: Modulation → Freq B depth.
    pub wm_freq_b: f32,
    /// +0x1c8: Osc B pulse width (−1..1), smoothed.
    pub pw_b: f32,
    /// +0x1cc
    pub pw_b_step: f32,
    /// +0x1d0: Modulation → PW B depth.
    pub wm_pw_b: f32,
    /// +0x1d4: Osc B Low (−91.3 semitones, or 0).
    pub low_b: f32,
    /// +0x1d8: Osc B KBD (1/0): follows the keyboard.
    pub kbd_b: f32,
    /// +0x1dc: Osc B tune knob ×47.625.
    pub tune_b_raw: f32,
    /// +0x1e0: Osc B semitones.
    pub semis_b: f32,
    /// +0x1e4: Osc B fine.
    pub fine_b: f32,
    /// +0x1e8: Poly Mod Osc B depth.
    pub pm_osc_b: f32,
    /// +0x1ec: Poly Mod Filter Env depth.
    pub pm_env: f32,
    /// +0x1f0: Osc A saw level (= mix A × Saw switch).
    pub saw_a: f32,
    /// +0x1f4: Modulation → Freq A depth.
    pub wm_freq_a: f32,
    /// +0x1f8: wheel modulation of Freq A now, and one sample before.
    pub wm_a: [f32; 2],
    /// +0x200: Poly Mod → Freq A depth.
    pub pm_freq_a: f32,
    /// +0x204: Osc A pulse width, smoothed.
    pub pw_a: f32,
    /// +0x208
    pub pw_a_step: f32,
    /// +0x20c: Modulation → PW A depth.
    pub wm_pw_a: f32,
    /// +0x210: Osc A pulse width + wheel modulation, now and before.
    pub pwm_a: [f32; 2],
    /// +0x218: Poly Mod → PW A depth.
    pub pm_pw_a: f32,
    /// +0x21c: Osc A Pulse on.
    pub pulse_on_a: i32,
    /// +0x220: Osc A Saw on (1/0).
    pub saw_on_a: f32,
    /// +0x224: Osc A semitones.
    pub semis_a: f32,
    /// +0x228: Sync on.
    pub sync: i32,
    /// +0x22c: Mixer Oscillator A, smoothed.
    pub mix_a: f32,
    /// +0x230
    pub mix_a_step: f32,
    /// +0x234: Mixer Oscillator B ×2, smoothed.
    pub mix_b: f32,
    /// +0x238
    pub mix_b_step: f32,
    /// +0x23c: Mixer Noise ×1.75.
    pub noise_mix: f32,
    /// +0x240: Mixer External ×25 (no input reaches it in Maschine).
    pub ext_mix: f32,
    /// +0x244: Modulation → Filter depth.
    pub wm_cut: f32,
    /// +0x248: Poly Mod → Filter depth.
    pub pm_cut: f32,
    /// +0x24c: filter Env Amount, smoothed.
    pub env_amt: f32,
    /// +0x250
    pub env_amt_step: f32,
    /// +0x254: filter mode (0 LP, 1 BP, 2 HP).
    pub mode: i32,
    /// +0x258: first stage damping.
    pub damp1: f32,
    /// +0x25c: filter KBD, smoothed.
    pub kbd: f32,
    /// +0x260
    pub kbd_step: f32,
    /// +0x264: cutoff (semitones).
    pub cutoff_semis: f32,
    /// +0x268: cutoff target (cutoff + resonance and keyboard compensation), smoothed.
    pub cutoff: f32,
    /// +0x26c
    pub cutoff_step: f32,
    /// +0x270: Resonance knob.
    pub reso: f32,
    /// +0x274: second stage damping.
    pub damp2: f32,
    /// +0x278: feedback sign (+1 LP/HP, −1 BP).
    pub fb_sign: f32,
    /// +0x27c: DC-removal lowpass coefficient.
    pub dc_coef: f32,
    /// +0x280: 1/sr.
    pub inv_sr: f32,
    /// +0x284: 6.283/sr.
    pub two_pi_sr: f32,
    /// +0x288: π/sr.
    pub pi_sr: f32,
    /// +0x28c: sum of the voices this sample.
    pub acc: f32,
    /// +0x290: volume smoothing steps left.
    pub vol_left: i32,
    /// +0x294: output gain.
    pub vol: f32,
    /// +0x298
    pub vol_step: f32,
    /// +0x29c: Env Amount target (×192, signed by Invert).
    pub env_amt_target: f32,
    /// +0x2a0: KBD target.
    pub kbd_target: f32,
    /// +0x2a4: Osc A pulse width target.
    pub pw_a_target: f32,
    /// +0x2a8: Osc B pulse width target.
    pub pw_b_target: f32,
    /// +0x2ac: mix A target.
    pub mix_a_target: f32,
    /// +0x2b0: mix B target.
    pub mix_b_target: f32,
    /// +0x2b4: volume target (×0.224).
    pub vol_target: f32,
    /// +0x2b8: Modulation Amount target.
    pub mod_amount_target: f32,
    /// +0x2bc: wheel target.
    pub wheel_target: f32,
    /// +0x2c0: pitch bend target (semitones).
    pub bend_target: f32,
    /// +0x2c4: synced LFO rate multiplier (cycles per beat).
    pub lfo_mult: f32,
    /// +0x2c8: LFO synced.
    pub lfo_synced: i32,
    /// +0x2cc: pitch bend now (semitones).
    pub bend: f32,
    /// +0x2d0
    pub bend_step: f32,
    /// +0x2d4: bend range (semitones).
    pub bend_range: f32,
    /// +0x2d8: bend wheel (0..1, 0.5 = centre).
    pub bend_value: f32,
    /// +0x2dc: master Tune (semitones, ±0.7).
    pub master: f32,
    /// +0x2e0: master tune + bend.
    pub tune: f32,
    /// +0x2e4: Detune amount.
    pub detune: f32,
    /// +0x2e8: velocity sensitivity of the amplifier envelope (0.8/0).
    pub vel_amp: f32,
    /// +0x2ec: of the filter envelope (0.55/0).
    pub vel_flt: f32,
    /// +0x2f0: Invert (−1) or not (+1).
    pub invert: f32,
    /// +0x2f4: Release switch (1/0).
    pub release_on: f32,
    /// +0x2f8: unison (all voices play one note).
    pub unison: i32,
    /// +0x2fc: note unison plays (`f32`).
    pub uni_note: f32,
    /// +0x300: LFO Voice Trig.
    pub voice_trig: i32,
    /// +0x304: Hold.
    pub hold: i32,
    /// +0x308: unison holds a key.
    pub uni_held: i32,
    /// +0x30c: unison retriggers its envelopes (Unison, not Unison with Legato).
    pub uni_retrig: i32,
    /// +0x310
    pub v: [Voice; SLOTS],
    /// +0x2790: `2^((i − 1200)/60)`, i.e. 1/60 octave steps over 40 octaves.
    pub table: [f32; TABLE],
    /// +0x4d14: `table[i+1] − table[i]`.
    pub slope: [f32; TABLE],
    /// +0x4d10: the table's last entry (2²⁰), what lookups past the end return.
    pub table_over: f32,
    /// +0x78a8: allocation list, next voice.
    pub next: [i32; SLOTS],
    /// +0x7928: allocation list, previous voice.
    pub prev: [i32; SLOTS],
    /// +0x79a8: note each voice plays (−1 none, −300 − note released).
    pub notes: [i32; SLOTS],
    /// +0x7a28: voice released while the pedal was down.
    pub sustained: [bool; SLOTS],
    /// +0x7a48: voice held by the sostenuto pedal (Maschine never sends it).
    pub sostenuto: [bool; SLOTS],
}

impl P5 {
    /// `P5::P5(44100)` then what `PolySynthProcessor`'s constructor does (32 voices,
    /// cleared), then `onSamplerateChanged(sr)`.
    pub fn new(sr: f32) -> Box<P5> {
        let mut p = Box::new(P5::blank());
        p.construct(44100.0);
        // PolySynthProcessor::PolySynthProcessor: 32 voice slots, hold latch off.
        if p.voices != 32 {
            p.voices = 32;
            p.sustain = false;
            p.all_sound_off();
        }
        p.set_sample_rate(sr);
        p
    }

    fn blank() -> P5 {
        P5 {
            sr: 0.0,
            smooth_len: 0,
            voices: 0,
            program: [0.0; 62],
            sustain: false,
            tail: 0,
            head: 0,
            last_note: 0,
            ctl_count: 0,
            smooth_left: 0,
            glide_rate: 0.0,
            glide_target: 0.0,
            glide: 0.0,
            glide_step: 0.0,
            lfo_phase: 0.0,
            lfo_rate: 0.0,
            lfo_saw: 0.0,
            lfo_tri: 0.0,
            lfo_pulse: 0.0,
            lfo_half: 0,
            lfo_sh_on: 0.0,
            lfo_sh: 0.0,
            lfo_none: 0.0,
            rng: 0,
            noise_amp: 0.0,
            noise_coef: 0.0,
            noise_lp: 0.0,
            lfo_noise: 0.0,
            wheel: 0.0,
            wheel_step: 0.0,
            mod_amount: 0.0,
            mod_amount_step: 0.0,
            amp_decay: 0.0,
            amp_release: 0.0,
            amp_release_raw: 0.0,
            amp_attack: 0.0,
            amp_sustain: 0.0,
            flt_decay: 0.0,
            flt_release: 0.0,
            flt_release_raw: 0.0,
            flt_attack: 0.0,
            flt_sustain: 0.0,
            pulse_on_b: 0,
            saw_b: 0.0,
            tri_on_b: 0,
            wm_freq_b: 0.0,
            pw_b: 0.0,
            pw_b_step: 0.0,
            wm_pw_b: 0.0,
            low_b: 0.0,
            kbd_b: 0.0,
            tune_b_raw: 0.0,
            semis_b: 0.0,
            fine_b: 0.0,
            pm_osc_b: 0.0,
            pm_env: 0.0,
            saw_a: 0.0,
            wm_freq_a: 0.0,
            wm_a: [0.0; 2],
            pm_freq_a: 0.0,
            pw_a: 0.0,
            pw_a_step: 0.0,
            wm_pw_a: 0.0,
            pwm_a: [0.0; 2],
            pm_pw_a: 0.0,
            pulse_on_a: 0,
            saw_on_a: 0.0,
            semis_a: 0.0,
            sync: 0,
            mix_a: 0.0,
            mix_a_step: 0.0,
            mix_b: 0.0,
            mix_b_step: 0.0,
            noise_mix: 0.0,
            ext_mix: 0.0,
            wm_cut: 0.0,
            pm_cut: 0.0,
            env_amt: 0.0,
            env_amt_step: 0.0,
            mode: 0,
            damp1: 0.0,
            kbd: 0.0,
            kbd_step: 0.0,
            cutoff_semis: 0.0,
            cutoff: 0.0,
            cutoff_step: 0.0,
            reso: 0.0,
            damp2: 0.0,
            fb_sign: 0.0,
            dc_coef: 0.0,
            inv_sr: 0.0,
            two_pi_sr: 0.0,
            pi_sr: 0.0,
            acc: 0.0,
            vol_left: 0,
            vol: 0.0,
            vol_step: 0.0,
            env_amt_target: 0.0,
            kbd_target: 0.0,
            pw_a_target: 0.0,
            pw_b_target: 0.0,
            mix_a_target: 0.0,
            mix_b_target: 0.0,
            vol_target: 0.0,
            mod_amount_target: 0.0,
            wheel_target: 0.0,
            bend_target: 0.0,
            lfo_mult: 0.0,
            lfo_synced: 0,
            bend: 0.0,
            bend_step: 0.0,
            bend_range: 0.0,
            bend_value: 0.0,
            master: 0.0,
            tune: 0.0,
            detune: 0.0,
            vel_amp: 0.0,
            vel_flt: 0.0,
            invert: 0.0,
            release_on: 0.0,
            unison: 0,
            uni_note: 0.0,
            voice_trig: 0,
            hold: 0,
            uni_held: 0,
            uni_retrig: 0,
            v: [Voice::default(); SLOTS],
            table: [0.0; TABLE],
            slope: [0.0; TABLE],
            table_over: 0.0,
            next: [0; SLOTS],
            prev: [0; SLOTS],
            notes: [0; SLOTS],
            sustained: [false; SLOTS],
            sostenuto: [false; SLOTS],
        }
    }

    /// `P5::P5(float)` (0x101aab1f4).
    fn construct(&mut self, sr: f32) {
        self.sr = sr;
        self.smooth_len = 20;
        self.voices = 5;
        self.program = DEFAULT_PROGRAM;
        self.sustain = false;
        self.tail = -1;
        self.head = -1;
        self.init_audio_memory();
        self.set_parameter_value(0x37, 0.5);
        // Volume at its definition's full value, smoothed in.
        self.program[0x38] = 1.0;
        self.vol_target = 0.224;
        self.smoother();
        // Bend range and bend wheel at their definitions' defaults (both type 1).
        let range = 1.0f32 / 12.0;
        self.program[0x3b] = range;
        self.bend_range = range.mul_add(24.0, 0.5) as i32 as f32;
        self.retarget_bend();
        self.reset_bend();
        self.set_sample_rate(self.sr);
        self.set_program(&DEFAULT_PROGRAM, 5);
        self.all_sound_off();
    }

    /// `P5::initAudioMemory` (0x101aab758).
    fn init_audio_memory(&mut self) {
        // Entry 0 is 2⁻²⁰ (stored separately at P5 + 0x2790); entries 1..=2400 are
        // `exp2f((j − 1200)/60)`, written four at a time from P5 + 0x2794, each slope
        // the difference to the previous entry. A lookup past the end reads the last
        // entry (2²⁰).
        let mut prev = f32::from_bits(0x3580_0000);
        self.table[0] = prev;
        for j in 1..=TABLE {
            let t = ((j as i32 as f32 + -1200.0) / 60.0).exp2();
            self.slope[j - 1] = t - prev;
            if j < TABLE {
                self.table[j] = t;
            } else {
                self.table_over = t;
            }
            prev = t;
        }
        self.set_sample_rate(self.sr);
        self.vol_left = 0;
        self.vol = 0.0;
        self.invert = 1.0;
        self.glide = 36.0;
        self.ctl_count = 100;
        self.rng = 1;
        self.notes = [-300; SLOTS];
        self.sustained = [false; SLOTS];
        self.sostenuto = [false; SLOTS];
    }

    /// `PolySynthProcessor::onSamplerateChanged` (0x101ab1cdc): only these values
    /// follow the rate; rates already computed by parameters keep the old one.
    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr;
        self.inv_sr = 1.0 / sr;
        self.two_pi_sr = f32::from_bits(0x40c90e56) / sr;
        self.pi_sr = std::f32::consts::PI / sr;
        self.noise_amp = (sr / 44100.0).sqrt();
        self.smooth_len = (sr.mul_add(0.02, 0.5) as i32).max(1);
        self.noise_coef = self.slope[1631].mul_add(NOISE_FRAC, self.table[1631]) * self.two_pi_sr;
        self.dc_coef = self.slope[1181].mul_add(NOISE_FRAC, self.table[1181]) * self.two_pi_sr;
    }

    /// Pitch (semitones, MIDI note numbers) → frequency (Hz): the table at
    /// `5·x + 1381.8816`, interpolated (`fmadd`).
    #[inline]
    pub fn hz(&self, semis: f32) -> f32 {
        self.lookup(semis.mul_add(5.0, 1381.8816))
    }

    /// The table at fractional index `y` (`fcvtzs` truncation; negative → 0, past the
    /// end → `table_over`).
    #[inline]
    pub fn lookup(&self, y: f32) -> f32 {
        let i = y as i32;
        if i >= TABLE as i32 {
            self.table_over
        } else if i < 0 {
            0.0
        } else {
            let i = i as usize;
            (y - i as f32).mul_add(self.slope[i], self.table[i])
        }
    }

    /// An envelope time knob → table value (`v·a + b`, then `·−9.965784 + 1797.947`).
    #[inline]
    pub fn env_time(&self, v: f32, a: f32, b: f32) -> f32 {
        self.lookup(v.mul_add(a, b).mul_add(-9.965784, 1797.947))
    }
}

/// The interpolation fraction `onSamplerateChanged` uses for the noise and DC
/// coefficients (0x3f61b000).
const NOISE_FRAC: f32 = f32::from_bits(0x3f61_b000);

/// Smoothing steps below ±1e-12 are dropped (compared in `f64`).
#[inline]
pub(crate) fn clip_step(step: f32) -> f32 {
    let d = step as f64;
    if d < 1e-12 && d > -1e-12 { 0.0 } else { step }
}
