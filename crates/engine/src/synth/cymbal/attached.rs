//! `NI::DSP::AttachedBodiesCore<…, AttachedBodiesNoise::white, Crispel, PitchEnv>`: the
//! Cymbal's "attached bodies" noise layer (docs/synth-cymbal.md §5.4). It sits at
//! `CrashCymbalCore +0x42000` and `RideCymbalCore +0x42000`.
//!
//! Per sample it mixes white noise (an LCG, envelope A) with one of the 23 recorded
//! noises of resource WAVE 3 read at a variable rate (envelope B), filters the sum
//! through two identical "crispel" state-variable filters in series (cutoff swept by
//! envelope C, resonance by envelope D) and two one-pole high-passes, and returns the
//! last high-pass output. Notes start after a delay (a 20-deep queue of pending notes).
//!
//! Two instantiations differ only in `processSample`: the Crash's (`AttachedBodiesAM::off`,
//! 0x1da4018) and the Ride's (`AttachedBodiesAM::on`, 0x1dd43b0), which multiplies the mix
//! by a random-rate triangle amplitude modulation. Their constructors, `onSetSampleRate`,
//! `setNoteOn` and setters are byte-identical.
//!
//! All arithmetic follows the VST3 arm64 code; `mul_add` marks its fused multiply-adds.

#![allow(dead_code)]

use super::memo::{K, MemoEnvelope};
use super::res;
use crate::fx::exp_e;

/// `f32` 2π as Maschine writes it (`0x40c90fdb`).
const TAU: f32 = f32::from_bits(0x40c9_0fdb);
/// `1/2^31` as an `f32` (`0x3000009d`): scales an LCG word to ±1.
const INV_2_31: f32 = f32::from_bits(0x3000_009d);
/// The exp-table pitch of the crispel cutoff, `x·5 + 1381.8816` (`0x44acbc36`).
const CRISPEL_PITCH: f32 = f32::from_bits(0x44ac_bc36);
/// The hold time constant `E(602.0529)` (`0x44168363`) of envelopes with hold 0.
const HOLD_PITCH: f32 = f32::from_bits(0x4416_8363);
/// Depth of the pending note-on queue.
const QUEUE: usize = 20;

/// The 32-bit LCG every NI noise source steps (`x·1103515245 + 12345`, wrapping).
#[inline]
fn lcg(x: u32) -> u32 {
    x.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039)
}

/// `NI::DSP::UTIL::NoiseGen<white>` at +0x000. Only `gain` and `seed` feed the white-noise
/// path; the rest is state of its (here unused) coloured modes, kept for parity.
#[derive(Clone, Debug, Default)]
pub(super) struct NoiseGen {
    /// +0x00: reseed (and clear the filters) on note-on.
    pub reset_on_note: bool,
    /// +0x04: sample rate.
    pub sr: f32,
    /// +0x08: the seed restored on note-on (50).
    pub seed0: u32,
    /// +0x10..+0x8c: state/coefficients of the coloured-noise modes (raw words).
    pub aux: [u32; 31],
    /// +0x8c: a sample rate has been set.
    pub sr_set: bool,
    /// +0x90: output gain `√(sr/44100)/2^31`.
    pub gain: f32,
    /// +0x94: sample rate (again).
    pub sr2: f32,
    /// +0x98: the LCG state.
    pub seed: u32,
    /// +0x9c: unused word.
    pub pad: u32,
    /// +0xa0: `1/√(sr/44100)`.
    pub inv_q: f32,
    /// +0xa4..+0xb0: one-pole low-pass state (unused by white noise).
    pub lp_state: [f32; 3],
    /// +0xb0: its `1/(t+1)`.
    pub lp_g: f32,
    /// +0xb4: its `t − 1`.
    pub lp_m: f32,
    /// +0xb8: its `tan(π·fc/sr)` (polynomial).
    pub lp_t: f32,
    /// +0xbc: its cutoff (15 Hz once a rate is set).
    pub lp_fc: f32,
    /// +0xc0: `1/sr`.
    pub lp_inv_sr: f32,
}

impl NoiseGen {
    fn new() -> Self {
        let mut aux = [0u32; 31];
        for (off, v) in [
            (0x18, 0x32),
            (0x1c, 0x3d63_66bd),
            (0x20, 0x3f7f_b54a),
            (0x2c, 0x3d99_c165),
            (0x30, 0x3f7e_4a38),
            (0x3c, 0x3e1d_8b61),
            (0x40, 0x3f78_1062),
            (0x4c, 0x3e9e_f7f8),
            (0x50, 0x3f5d_d2f2),
            (0x5c, 0x3f08_6f8e),
            (0x60, 0x3f0c_cccd),
            (0x6c, 0xbc8a_6dad),
            (0x70, 0xbf42_f838),
            (0x7c, 0x3f09_4467),
            (0x80, 0x3ded_6a9c),
        ] {
            aux[(off - 0x10) / 4] = v;
        }
        NoiseGen { sr: 1.0, seed0: 0x32, aux, gain: INV_2_31, sr2: 1.0, inv_q: 1.0, lp_fc: 1.0, lp_inv_sr: 1.0, ..Default::default() }
    }

    /// The `tan` polynomial `x + (x⁵·0.13333 + x³·0.33333)`.
    fn tan(x: f32) -> f32 {
        let x2 = x * x;
        let x3 = x * x2;
        let x5 = x2 * x3;
        x + x5.mul_add(f32::from_bits(0x3e08_8886), x3 * f32::from_bits(0x3eaa_aaaa))
    }

    /// `setSampleRate(sr)` (0x1ce4ed0); rates outside 1..500000 are ignored.
    pub fn set_sample_rate(&mut self, sr: f32) {
        if !(1.0..=500_000.0).contains(&sr) {
            return;
        }
        self.sr = sr;
        self.sr_set = true;
        let q = (sr / 44100.0).sqrt();
        self.gain = q * INV_2_31;
        self.sr2 = sr;
        self.inv_q = 1.0 / q;
        let inv = 1.0 / sr;
        self.lp_inv_sr = inv;
        let fc = self.lp_fc;
        if fc != -1.0 {
            let t = Self::tan(inv * (fc * std::f32::consts::PI));
            self.lp_g = 1.0 / (t + 1.0);
            self.lp_m = t + -1.0;
            self.lp_t = t;
            self.lp_state[1] = 0.0;
            self.lp_state[2] = 0.0;
            if fc == 15.0 {
                self.lp_state = [0.0; 3];
                return;
            }
        }
        self.lp_fc = 15.0;
        let t = Self::tan(inv * f32::from_bits(0x423c_7edd));
        self.lp_t = t;
        self.lp_g = 1.0 / (t + 1.0);
        self.lp_m = t + -1.0;
        self.lp_state = [0.0; 3];
    }

    /// `setNoteOn(v)` (0x1ce5210): reseed and clear the filters when `reset_on_note`.
    pub fn set_note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) || !self.reset_on_note {
            return;
        }
        let s = self.seed0;
        self.seed = s;
        self.aux[0] = 0; // +0x10
        self.aux[1] = s; // +0x14
        self.aux[2] = s; // +0x18
        for off in (0x24..0x8c).step_by(0x10) {
            self.aux[(off - 0x10) / 4] = 0;
            self.aux[(off - 0x10) / 4 + 1] = 0;
        }
    }
}

/// `UTIL::NoiseSampler` at +0x0c8: one recorded noise read with linear interpolation.
#[derive(Clone, Debug, Default)]
pub(super) struct NoiseSampler {
    /// +0xc8: base pitch of the read rate (15 in the presets).
    pub base_pitch: f32,
    /// +0xcc: rotate-speed modulation added to `base_pitch`.
    pub rotate: f32,
    /// +0xd0: which of the 23 noises (the cores set it with `data`/`block`).
    pub index: u32,
    /// +0xd4: restart at sample 0 on note-on.
    pub reset_on_note: bool,
    /// +0xd8: rate ratio `E((base + rotate)·5 + 1200)`.
    pub ratio: f32,
    /// +0xdc: `ratio/sr` (the position advances by `inc·44100` per sample).
    pub inc: f32,
    /// +0xe0: sample rate.
    pub sr: f32,
    /// +0xe4: `1/sr`.
    pub inv_sr: f32,
    /// +0xe8: length of the noise in samples (131072).
    pub len: u32,
    /// +0xf0: start of the read noise, as a sample index into resource WAVE 3 (NI: a
    /// pointer).
    pub data: usize,
    /// +0xf8: start of the selected noise, as a sample index (NI: a pointer;
    /// `onSetSampleRate` copies it to `data`).
    pub block: usize,
    /// +0x100: read position.
    pub pos: f32,
}

/// The two-integrator state-variable filter ("crispel") at +0x388 and +0x3bc. Its input
/// is the average of two samples (×0.25 of their sum); it outputs low-pass.
#[derive(Clone, Debug, Default)]
pub(super) struct Svf {
    /// +0x00: previous input.
    pub prev: f32,
    /// +0x04: first integrator.
    pub s1: f32,
    /// +0x08: second integrator.
    pub s2: f32,
    /// +0x0c: high-pass output.
    pub hp: f32,
    /// +0x10: band-pass output.
    pub bp: f32,
    /// +0x14: low-pass output.
    pub lp: f32,
    /// +0x18: `min(2π·f/sr, 1.9)`.
    pub g: f32,
    /// +0x1c: damping `max(2·(1 − res), 0.02)`.
    pub k: f32,
    /// +0x20: the feedback coefficient from `g` and `k`.
    pub c: f32,
    /// +0x24: cutoff (Hz; −1 = unset).
    pub freq: f32,
    /// +0x28: resonance.
    pub res: f32,
    /// +0x2c: sample rate.
    pub sr: f32,
    /// +0x30: `1/sr`.
    pub inv_sr: f32,
}

impl Svf {
    fn new() -> Self {
        Svf { k: 2.0, c: 2.0, freq: 1.0, sr: 1.0, inv_sr: 1.0, ..Default::default() }
    }

    fn coef(&mut self) {
        let (g, k) = (self.g, self.k);
        let num = k * ((g + 2.0) * (2.0 - g));
        let d = g.mul_add(k, 2.0) - g;
        self.c = num / (d + d);
    }

    /// Set the cutoff (no-op when unchanged).
    pub fn set_freq(&mut self, f: f32) {
        if self.freq != f {
            self.freq = f;
            self.g = (f * TAU * self.inv_sr).min(1.9);
            self.coef();
        }
    }

    /// Set the resonance (no-op when unchanged).
    pub fn set_res(&mut self, r: f32) {
        if self.res != r {
            self.res = r;
            self.k = ((1.0 - r) + (1.0 - r)).max(f32::from_bits(0x3ca3_d70a));
            self.coef();
        }
    }

    /// New rate: re-apply a set cutoff.
    fn set_sample_rate(&mut self, sr: f32, inv: f32) {
        self.sr = sr;
        self.inv_sr = inv;
        let f = self.freq;
        self.freq = -1.0;
        if f != -1.0 {
            self.set_freq(f);
        }
    }

    /// One sample; returns the low-pass output.
    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let inp = (x + self.prev) * 0.25;
        self.prev = x;
        let v = inp - self.c.mul_add(self.s1, self.s2);
        let s1 = self.g.mul_add(v, self.s1);
        let s2 = self.g.mul_add(s1, self.s2);
        let lp = self.s2 + s2;
        let bp = s1 + s1;
        self.lp = lp;
        self.bp = bp;
        self.hp = x - bp.mul_add(self.c, lp);
        self.s1 = s1;
        self.s2 = s2;
        lp
    }
}

/// The lagged one-pole high-pass at +0x3f0 and +0x40c (`y += c·h_prev; h = x − y`).
#[derive(Clone, Debug, Default)]
pub(super) struct OnePole {
    /// +0x00: low-pass state.
    pub y: f32,
    /// +0x04: high-pass state.
    pub h: f32,
    /// +0x08: low-pass output.
    pub out_y: f32,
    /// +0x0c: high-pass output.
    pub out_h: f32,
    /// +0x10: `min(2π·f/sr, 1)`.
    pub c: f32,
    /// +0x14: cutoff (Hz; −1 = unset).
    pub freq: f32,
    /// +0x18: `1/sr`.
    pub inv_sr: f32,
}

impl OnePole {
    fn new() -> Self {
        OnePole { freq: 1.0, inv_sr: 1.0, ..Default::default() }
    }

    /// Set the cutoff (no-op when unchanged).
    pub fn set_freq(&mut self, f: f32) {
        if self.freq != f {
            self.freq = f;
            let c = f * (self.inv_sr * TAU);
            self.c = if c > 1.0 { 1.0 } else { c };
        }
    }

    fn set_sample_rate(&mut self, inv: f32) {
        self.inv_sr = inv;
        let f = self.freq;
        self.freq = -1.0;
        if f != -1.0 {
            self.set_freq(f);
        }
        self.y = 0.0;
        self.h = 0.0;
        self.out_y = 0.0;
        self.out_h = 0.0;
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.c.mul_add(self.h, self.y);
        let h = x - y;
        self.out_y = y;
        self.out_h = h;
        self.y = y;
        self.h = h;
        h
    }
}

/// `AttachedBodiesCore` (0x5c8 bytes). Field docs give offsets from its base.
#[derive(Clone, Debug)]
pub(super) struct AttachedBodies {
    /// Not NI state: the Ride's instantiation (`AttachedBodiesAM::on`), whose
    /// `processSample` runs the amplitude modulation.
    pub am: bool,
    /// +0x000: white noise.
    pub noise: NoiseGen,
    /// +0x0c8: recorded noise.
    pub sampler: NoiseSampler,
    /// +0x108: envelope A (white-noise level).
    pub env_noise: MemoEnvelope,
    /// +0x188: envelope B (recorded-noise level).
    pub env_sample: MemoEnvelope,
    /// +0x208: envelope C (crispel cutoff sweep).
    pub env_freq: MemoEnvelope,
    /// +0x288: envelope D (crispel resonance sweep).
    pub env_res: MemoEnvelope,
    /// +0x308: envelope E (started with the others, never read).
    pub env_e: MemoEnvelope,
    /// +0x388, +0x3bc: the crispel filters.
    pub svf: [Svf; 2],
    /// +0x3f0, +0x40c: the high-passes.
    pub hp: [OnePole; 2],
    /// +0x428: sample rate.
    pub sr: f32,
    /// +0x42c: `1/sr`.
    pub inv_sr: f32,
    /// +0x430: the block size given to `onSetSampleRate`.
    pub block: i32,
    /// +0x434: note delay offset (ms, added to `delay_ms`).
    pub delay_offset: f32,
    /// +0x438: preset value (not read here).
    pub u438: f32,
    /// +0x43c: noise index as the presets store it (an integer word).
    pub noise_index: u32,
    /// +0x440: base pitch as the presets store it.
    pub base_pitch: f32,
    /// +0x444: AM rate (Hz).
    pub am_rate: f32,
    /// +0x448: AM rate jitter (× the random value).
    pub am_jitter: f32,
    /// +0x44c: preset value (not read here).
    pub u44c: f32,
    /// +0x450: AM depth of the random value.
    pub am_random_depth: f32,
    /// +0x454: AM shape (triangle → ramp).
    pub am_shape: f32,
    /// +0x458: AM amount (0..1; ≤ 0 = off).
    pub am_amount: f32,
    /// +0x45c: white-noise gain.
    pub noise_gain: f32,
    /// +0x460: white-noise amount (the gain is `(amount + modulation)²`).
    pub noise_amount: f32,
    /// +0x464: recorded-noise gain.
    pub sample_gain: f32,
    /// +0x468: preset value (not read here).
    pub u468: f32,
    /// +0x46c: envelope A attack base (`setModulationAttack` adds to it).
    pub noise_attack: f32,
    /// +0x470: preset value (not read here).
    pub u470: f32,
    /// +0x474: envelope A release base (`setModulationNoiseDecay2` adds to it).
    pub noise_decay2: f32,
    /// +0x478: envelope B attack base.
    pub sample_attack: f32,
    /// +0x47c: preset value (not read here).
    pub u47c: f32,
    /// +0x480: envelope B release base (`setModulationCrispelDecay2` adds to it).
    pub crispel_decay2: f32,
    /// +0x484: preset word (not read here).
    pub u484: u32,
    /// +0x488: crispel cutoff pitch at the top of envelope C.
    pub freq_top: f32,
    /// +0x48c: crispel cutoff pitch at rest.
    pub freq_base: f32,
    /// +0x490: crispel cutoff (Hz) applied by `onSetSampleRate`.
    pub freq: f32,
    /// +0x494: resonance at the top of envelope D.
    pub res_top: f32,
    /// +0x498: resonance at rest.
    pub res_base: f32,
    /// +0x49c: preset value (not read here).
    pub u49c: f32,
    /// +0x4a0: high-pass cutoff (Hz).
    pub hp_freq: f32,
    /// +0x4a4: note delay (ms).
    pub delay_ms: f32,
    /// +0x4a8: the AM's LCG state.
    pub am_seed: u32,
    /// +0x4ac: the AM's current random value (±1).
    pub am_random: f32,
    /// +0x4b0: the AM's phase step per sample.
    pub am_step: f32,
    /// +0x4b4: the AM's phase (−0.5..0.5).
    pub am_phase: f32,
    /// +0x4b8: note delay in samples.
    pub delay: u32,
    /// +0x4bc: pending note-ons.
    pub pending: u32,
    /// +0x4c0: unused word.
    pub u4c0: u32,
    /// +0x4c4: velocity curve: lower bound.
    pub vel_lo: f32,
    /// +0x4c8: velocity curve: upper bound.
    pub vel_hi: f32,
    /// +0x4cc: velocity curve: first bend.
    pub vel_bend1: f32,
    /// +0x4d0: velocity curve: second bend.
    pub vel_bend2: f32,
    /// +0x4d4..+0x4e4: preset values (not read here).
    pub u4d4: [f32; 4],
    /// +0x4e4: samples after a note before the AM starts.
    pub am_start: u32,
    /// +0x4e8: samples since the note (counts up to `am_start`).
    pub am_count: u32,
    /// +0x4ec: velocities of the pending notes.
    pub queue_vel: [f32; QUEUE],
    /// +0x53c: samples waited by each pending note.
    pub queue_wait: [u32; QUEUE],
    /// +0x58c: pending notes whose delay has run out.
    pub queue_due: [bool; QUEUE],
    /// +0x5a0: rotate-speed modulation.
    pub mod_rotate: f32,
    /// +0x5a4: modulation slot (written by the cores).
    pub mod_5a4: f32,
    /// +0x5a8: noise-amount modulation.
    pub mod_noise_amount: f32,
    /// +0x5ac: attack modulation.
    pub mod_attack: f32,
    /// +0x5b0: modulation slot (written by the cores).
    pub mod_5b0: f32,
    /// +0x5b4: crispel decay-2 modulation.
    pub mod_crispel_decay2: f32,
    /// +0x5b8: noise decay-2 modulation.
    pub mod_noise_decay2: f32,
    /// +0x5bc: crispel cutoff modulation (pitch; ×1.5 at rest).
    pub mod_freq: f32,
    /// +0x5c0: modulation slot (written by the cores).
    pub mod_5c0: f32,
}

fn memo_new() -> MemoEnvelope {
    MemoEnvelope { sr: 1.0, inv_sr: 1.0, ..Default::default() }
}

impl AttachedBodies {
    /// The constructor (Crash 0x1da3184; the Ride's 0x1dd3b78 is identical). `am`: the
    /// Ride's instantiation.
    pub fn new(am: bool) -> Self {
        let hold = exp_e(HOLD_PITCH) + -0.001;
        let mut env_noise = memo_new();
        env_noise.hold_len = f64::from(env_noise.sr * hold);
        let mut env_sample = memo_new();
        env_sample.hold_len = f64::from(hold);
        let mut env_freq = memo_new();
        env_freq.attack = -40.0;
        env_freq.attack_step = f64::from(exp_e(2_196.578_369_140_625));
        env_freq.hold_len = f64::from(hold);
        let env_res = env_freq.clone();
        let mut a = AttachedBodies {
            am,
            noise: NoiseGen::new(),
            sampler: NoiseSampler { sr: 1.0, ..Default::default() },
            env_noise,
            env_sample,
            env_freq,
            env_res,
            env_e: memo_new(),
            svf: [Svf::new(), Svf::new()],
            hp: [OnePole::new(), OnePole::new()],
            sr: -1.0,
            inv_sr: -1.0,
            block: -1,
            delay_offset: 0.0,
            u438: 0.0,
            noise_index: 0,
            base_pitch: 0.0,
            am_rate: 0.0,
            am_jitter: 0.0,
            u44c: 0.0,
            am_random_depth: 0.0,
            am_shape: 0.0,
            am_amount: 0.0,
            noise_gain: 0.0,
            noise_amount: 0.0,
            sample_gain: 0.0,
            u468: 0.0,
            noise_attack: 0.0,
            u470: 0.0,
            noise_decay2: 0.0,
            sample_attack: 0.0,
            u47c: 0.0,
            crispel_decay2: 0.0,
            u484: 0,
            freq_top: 0.0,
            freq_base: 0.0,
            freq: 0.0,
            res_top: 0.0,
            res_base: 0.0,
            u49c: 0.0,
            hp_freq: 0.0,
            delay_ms: 0.0,
            am_seed: 1,
            am_random: 0.0,
            am_step: 0.0,
            am_phase: 0.0,
            delay: 0,
            pending: 0,
            u4c0: 0,
            vel_lo: 0.0,
            vel_hi: 0.0,
            vel_bend1: 0.0,
            vel_bend2: 0.0,
            u4d4: [0.0; 4],
            am_start: 0,
            am_count: 0,
            queue_vel: [0.0; QUEUE],
            queue_wait: [0; QUEUE],
            queue_due: [false; QUEUE],
            mod_rotate: 0.0,
            mod_5a4: 0.0,
            mod_noise_amount: 0.0,
            mod_attack: 0.0,
            mod_5b0: 0.0,
            mod_crispel_decay2: 0.0,
            mod_noise_decay2: 0.0,
            mod_freq: 0.0,
            mod_5c0: 0.0,
        };
        let f = exp_e(CRISPEL_PITCH);
        a.freq = f;
        a.hp_freq = f;
        for s in &mut a.svf {
            s.set_freq(f);
            s.set_res(0.0);
        }
        for h in &mut a.hp {
            h.set_freq(f);
        }
        a
    }

    /// `onSetSampleRate(sr, block)` (0x1cfb9f4): rates for the envelopes, noise and
    /// filters; the sampler restarts on its selected noise; the note delay in samples.
    pub fn set_sample_rate(&mut self, sr: f32, block: i32) {
        self.sr = sr;
        let inv = 1.0 / sr;
        self.inv_sr = inv;
        self.block = block;
        self.env_noise.set_sample_rate(sr);
        self.env_sample.set_sample_rate(sr);
        self.env_freq.set_sample_rate(sr);
        self.env_res.set_sample_rate(sr);
        self.env_e.set_sample_rate(sr);
        self.noise.set_sample_rate(sr);
        let s = &mut self.sampler;
        s.sr = sr;
        s.inv_sr = inv;
        s.inc = inv * s.ratio;
        s.data = s.block;
        s.len = 0x20000;
        s.pos = 0.0;
        self.svf[0].set_sample_rate(sr, inv);
        self.svf[1].set_sample_rate(sr, inv);
        self.hp[0].set_sample_rate(inv);
        self.hp[1].set_sample_rate(inv);
        let (f, r) = (self.freq, self.res_base);
        self.svf[0].set_freq(f);
        self.svf[0].set_res(r);
        self.svf[1].set_freq(f);
        self.svf[1].set_res(r);
        let h = self.hp_freq;
        self.hp[0].set_freq(h);
        self.hp[1].set_freq(h);
        self.update_delay();
    }

    /// The note delay in samples from `delay_ms + delay_offset` (the end of
    /// `onSetSampleRate`; the cores compute the same).
    pub fn update_delay(&mut self) {
        let ms = self.delay_ms + self.delay_offset;
        let s = if ms < 0.0 { 0.0 } else { ms * 0.001 };
        self.delay = (self.sr * s) as u32;
    }

    /// The velocity curve: `v` → 0..1 between `vel_lo` and `vel_hi`, bent twice.
    fn curve(&self, v: f32) -> f32 {
        let lo = self.vel_lo;
        let hi = if self.vel_hi > lo { self.vel_hi } else { lo + 0.002 };
        let x = if lo > v { lo } else { v };
        let x = if x > hi { hi } else { x };
        let min = f32::from_bits(0x33d6_bf95);
        let range = hi - lo;
        let range = if range < min { min } else { range };
        let mut x = (x - lo) / range;
        let (b1, b2) = (self.vel_bend1, self.vel_bend2);
        x *= (-x.abs()).mul_add(b1, b1 + 1.0);
        x *= (-x.abs()).mul_add(b2, b2 + 1.0);
        let x = if x < 0.0 { 0.0 } else { x };
        if x > 1.0 { 1.0 } else { x }
    }

    /// `setNoteOn(v)` (0x1cfd56c): queue the note; when the head of the queue is due
    /// (`processSample` marks it), start the envelopes at the curved velocity.
    pub fn set_note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        let g = self.curve(v);
        if g <= 0.0 {
            return;
        }
        if !self.queue_due[0] {
            let n = (self.pending as usize).min(QUEUE - 1);
            self.queue_vel[n] = v;
            self.queue_wait[n] = 0;
            self.queue_due[n] = false;
            self.pending = n as u32 + 1;
            return;
        }
        self.noise.set_note_on(v);
        if self.sampler.reset_on_note {
            self.sampler.pos = 0.0;
        }
        self.env_freq.set_note_on(g);
        self.env_res.set_note_on(g);
        self.env_noise.set_note_on(g);
        self.env_sample.set_note_on(g);
        self.env_e.set_note_on(g);
        self.queue_vel[0] = 0.0;
        self.queue_wait[0] = 0;
        self.queue_due[0] = false;
        let n = (self.pending as usize).min(QUEUE);
        if n >= 1 {
            self.queue_vel.copy_within(1..n, 0);
            self.queue_vel[n - 1] = 0.0;
            self.queue_wait.copy_within(1..n, 0);
            self.queue_wait[n - 1] = 0;
            self.queue_due.copy_within(1..n, 0);
            self.queue_due[n - 1] = false;
            self.pending -= 1;
        }
        self.am_count = 0;
    }

    /// A release multiplier `1 − E((60 − p)·K + 1200)·inv` (0 when that is ≥ 1).
    fn release_mul(p: f32, inv: f64) -> f64 {
        let d = f64::from(exp_e((60.0 - p).mul_add(K, 1200.0))) * inv;
        if d < 1.0 { 1.0 - d } else { 0.0 }
    }

    /// `setModulationCrispelDecay2(v)` (0x1cfc540): envelope B's release, −40..40.
    pub fn set_modulation_crispel_decay2(&mut self, v: f32) {
        if !(-40.0..=40.0).contains(&v) {
            return;
        }
        self.mod_crispel_decay2 = v;
        let p = self.crispel_decay2 + v;
        self.env_sample.release = p;
        self.env_sample.release_mul = Self::release_mul(p, self.env_sample.inv_sr);
    }

    /// `setModulationNoiseDecay2(v)` (0x1cfc680): envelope A's release, −40..40.
    pub fn set_modulation_noise_decay2(&mut self, v: f32) {
        if !(-40.0..=40.0).contains(&v) {
            return;
        }
        self.mod_noise_decay2 = v;
        let p = self.noise_decay2 + v;
        self.env_noise.release = p;
        self.env_noise.release_mul = Self::release_mul(p, self.env_noise.inv_sr);
    }

    /// `setModulationRotateSpeed(v)` (0x1cfca20): the recorded noise's read rate, −60..60.
    pub fn set_modulation_rotate_speed(&mut self, v: f32) {
        if !(-60.0..=60.0).contains(&v) {
            return;
        }
        self.mod_rotate = v;
        let s = &mut self.sampler;
        s.rotate = v;
        let r = exp_e((s.base_pitch + v).mul_add(5.0, 1200.0));
        s.ratio = r;
        s.inc = r * s.inv_sr;
    }

    /// `setModulationAmountNoise(v)` (0x1d4c798): white-noise gain `(amount + v)²`, −2..2
    /// (only when `noise_amount` is in 0..2).
    pub fn set_modulation_amount_noise(&mut self, v: f32) {
        if !(-2.0..=2.0).contains(&v) {
            return;
        }
        self.mod_noise_amount = v;
        let b = self.noise_amount;
        if !(0.0..=2.0).contains(&b) {
            return;
        }
        let a = b + v;
        let a = if a < 0.0 { 0.0 } else { a };
        self.noise_gain = a * a;
    }

    /// `setModulationAttack(v)` (0x1d90088): envelopes A and B attack, −40..40.
    pub fn set_modulation_attack(&mut self, v: f32) {
        if !(-40.0..=40.0).contains(&v) {
            return;
        }
        self.mod_attack = v;
        let p = self.noise_attack + v;
        self.env_noise.attack = p;
        self.env_noise.attack_step = f64::from(exp_e((60.0 - p).mul_add(K, 1200.0))) * self.env_noise.inv_sr;
        let p = self.sample_attack + v;
        self.env_sample.attack = p;
        self.env_sample.attack_step = f64::from(exp_e((60.0 - p).mul_add(K, 1200.0))) * self.env_sample.inv_sr;
    }

    /// The Ride's amplitude modulation (head of 0x1dd43b0): 1 until `am_start` samples
    /// after the note, then `1 + amount·(shape² − 1)` with a triangle/ramp `shape` whose
    /// rate jumps to a new random value every period.
    fn am_gain(&mut self) -> f32 {
        let amt = self.am_amount;
        if amt <= 0.0 || self.am_count <= self.am_start {
            self.am_count = self.am_count.wrapping_add(1);
            return 1.0;
        }
        let rate = self.am_step;
        let rate = if rate < -1.0 { -1.0 } else { rate };
        let rate = if rate > 1.0 { 1.0 } else { rate };
        let mut ph = self.am_phase + rate;
        let r = if ph >= 0.5 {
            ph += -1.0;
            self.am_seed = lcg(self.am_seed);
            let r = (self.am_seed as i32) as f32 * INV_2_31;
            self.am_random = r;
            r
        } else {
            self.am_random
        };
        self.am_phase = ph;
        self.am_step = (self.am_jitter.mul_add(r, 1.0) * self.am_rate) * self.inv_sr;
        let t = ph + 0.25;
        let half = if t < 0.0 { -0.5 } else { 0.5 };
        let n = ((half + f64::from(t)) as i32) as f32;
        let tri = ((t - n) * (t - n).abs().mul_add(-16.0, 8.0) + 1.0) * 0.5;
        let shape = ((0.5 - ph) - tri).mul_add(self.am_shape, tri);
        let depth = (r.abs() + -1.0).mul_add(self.am_random_depth, 1.0);
        let s = depth * shape;
        let s = s * s;
        let a = if amt < 0.0 { 0.0 } else { amt };
        let a = if a > 1.0 { 1.0 } else { a };
        (s + -1.0).mul_add(a, 1.0)
    }

    /// `processSample()` (Crash 0x1da4018, Ride 0x1dd43b0): one output sample.
    pub fn process_sample(&mut self) -> f32 {
        let mut i = 0;
        while i < self.pending as usize {
            if self.queue_wait[i] >= self.delay {
                self.queue_due[i] = true;
                let v = self.queue_vel[i];
                self.set_note_on(v);
            } else {
                self.queue_wait[i] += 1;
            }
            i += 1;
        }
        let am = if self.am { self.am_gain() } else { 1.0 };
        self.noise.seed = lcg(self.noise.seed);
        let rnd = self.noise.seed as i32 as f32;
        let env_a = self.env_noise.process();
        // The recorded noise (silent without the resource).
        let s = &mut self.sampler;
        let pos = s.pos;
        let k = pos as u32;
        let (a, b) = match res::noises() {
            Some(n) if s.len > 0 => {
                let j = if s.len - 1 == k { 0 } else { k + 1 };
                let at = |i: u32| n.get(s.data + i as usize).copied().unwrap_or(0.0);
                (at(k), at(j))
            }
            _ => (0.0, 0.0),
        };
        let np = s.inc.mul_add(res::WAV_RATE, pos);
        let len = s.len as f32;
        s.pos = np - if np < len { 0.0 } else { len };
        let env_b = self.env_sample.process();
        // Envelope C sweeps the crispel cutoff (not when it is or falls idle).
        let c = self.env_freq.process();
        if self.env_freq.state != 0 {
            let lo = self.mod_freq.mul_add(1.5, self.freq_base);
            let p = ((self.mod_freq + self.freq_top) - lo).mul_add(c, lo);
            let f = exp_e(p.mul_add(5.0, CRISPEL_PITCH));
            self.svf[0].set_freq(f);
            self.svf[1].set_freq(f);
        }
        // Envelope D sweeps the resonance (only run when it has a range).
        let res = if self.res_base == 0.0 && self.res_top == 0.0 {
            (self.env_res.state != 0).then_some(0.0)
        } else {
            let d = self.env_res.process();
            let d = if d < 0.0 { 0.0 } else { d };
            let d = if d > 1.0 { 1.0 } else { d };
            let r = (self.res_top - self.res_base).mul_add(d, self.res_base);
            let r = (r * f32::from_bits(0x3f3d_70a4)) * (2.0 - r);
            (self.env_res.state != 0).then_some(r)
        };
        if let Some(r) = res {
            self.svf[0].set_res(r);
            self.svf[1].set_res(r);
        }
        let lerp = (b - a).mul_add(pos - k as f32, a);
        let w = ((self.noise.gain * rnd) * env_a) * self.noise_gain;
        let mut x = (lerp * env_b).mul_add(self.sample_gain, w);
        if self.am {
            x *= am;
        }
        let y = self.svf[0].tick(x);
        let y = self.svf[1].tick(y);
        let y = self.hp[0].tick(y);
        self.hp[1].tick(y)
    }

    /// Every NI field with its offset (the `data`/`block` pointers excluded), for
    /// [`AttachedBodies::image`] and the oracle tests.
    #[cfg(test)]
    pub(super) fn walk(&mut self, v: &mut dyn FnMut(usize, Slot)) {
        let n = &mut self.noise;
        v(0, Slot::B(&mut n.reset_on_note));
        v(4, Slot::F(&mut n.sr));
        v(8, Slot::U(&mut n.seed0));
        for (i, w) in n.aux.iter_mut().enumerate() {
            v(0x10 + 4 * i, Slot::U(w));
        }
        v(0x8c, Slot::B(&mut n.sr_set));
        v(0x90, Slot::F(&mut n.gain));
        v(0x94, Slot::F(&mut n.sr2));
        v(0x98, Slot::U(&mut n.seed));
        v(0x9c, Slot::U(&mut n.pad));
        v(0xa0, Slot::F(&mut n.inv_q));
        for (i, x) in n.lp_state.iter_mut().enumerate() {
            v(0xa4 + 4 * i, Slot::F(x));
        }
        v(0xb0, Slot::F(&mut n.lp_g));
        v(0xb4, Slot::F(&mut n.lp_m));
        v(0xb8, Slot::F(&mut n.lp_t));
        v(0xbc, Slot::F(&mut n.lp_fc));
        v(0xc0, Slot::F(&mut n.lp_inv_sr));
        let s = &mut self.sampler;
        v(0xc8, Slot::F(&mut s.base_pitch));
        v(0xcc, Slot::F(&mut s.rotate));
        v(0xd0, Slot::U(&mut s.index));
        v(0xd4, Slot::B(&mut s.reset_on_note));
        v(0xd8, Slot::F(&mut s.ratio));
        v(0xdc, Slot::F(&mut s.inc));
        v(0xe0, Slot::F(&mut s.sr));
        v(0xe4, Slot::F(&mut s.inv_sr));
        v(0xe8, Slot::U(&mut s.len));
        v(0x100, Slot::F(&mut s.pos));
        for (o, m) in [
            (0x108, &mut self.env_noise),
            (0x188, &mut self.env_sample),
            (0x208, &mut self.env_freq),
            (0x288, &mut self.env_res),
            (0x308, &mut self.env_e),
        ] {
            v(o, Slot::U(&mut m.state));
            v(o + 4, Slot::F(&mut m.attack));
            v(o + 8, Slot::F(&mut m.hold));
            v(o + 0xc, Slot::F(&mut m.decay));
            v(o + 0x10, Slot::F(&mut m.release));
            v(o + 0x18, Slot::D(&mut m.attack_step));
            v(o + 0x20, Slot::D(&mut m.hold_len));
            v(o + 0x28, Slot::D(&mut m.decay_step));
            v(o + 0x30, Slot::D(&mut m.sustain));
            v(o + 0x38, Slot::D(&mut m.release_mul));
            v(o + 0x40, Slot::D(&mut m.inc));
            v(o + 0x48, Slot::D(&mut m.dec));
            v(o + 0x50, Slot::D(&mut m.sus));
            v(o + 0x58, Slot::F(&mut m.peak32));
            v(o + 0x60, Slot::D(&mut m.peak));
            v(o + 0x68, Slot::D(&mut m.cur));
            v(o + 0x70, Slot::U(&mut m.count));
            v(o + 0x74, Slot::F(&mut m.sr));
            v(o + 0x78, Slot::D(&mut m.inv_sr));
        }
        let [s0, s1] = &mut self.svf;
        for (o, s) in [(0x388, s0), (0x3bc, s1)] {
            let f = [
                &mut s.prev,
                &mut s.s1,
                &mut s.s2,
                &mut s.hp,
                &mut s.bp,
                &mut s.lp,
                &mut s.g,
                &mut s.k,
                &mut s.c,
                &mut s.freq,
                &mut s.res,
                &mut s.sr,
                &mut s.inv_sr,
            ];
            for (i, x) in f.into_iter().enumerate() {
                v(o + 4 * i, Slot::F(x));
            }
        }
        let [h0, h1] = &mut self.hp;
        for (o, h) in [(0x3f0, h0), (0x40c, h1)] {
            let f = [&mut h.y, &mut h.h, &mut h.out_y, &mut h.out_h, &mut h.c, &mut h.freq, &mut h.inv_sr];
            for (i, x) in f.into_iter().enumerate() {
                v(o + 4 * i, Slot::F(x));
            }
        }
        v(0x428, Slot::F(&mut self.sr));
        v(0x42c, Slot::F(&mut self.inv_sr));
        v(0x430, Slot::I(&mut self.block));
        for (o, x) in [
            (0x434, &mut self.delay_offset),
            (0x438, &mut self.u438),
            (0x440, &mut self.base_pitch),
            (0x444, &mut self.am_rate),
            (0x448, &mut self.am_jitter),
            (0x44c, &mut self.u44c),
            (0x450, &mut self.am_random_depth),
            (0x454, &mut self.am_shape),
            (0x458, &mut self.am_amount),
            (0x45c, &mut self.noise_gain),
            (0x460, &mut self.noise_amount),
            (0x464, &mut self.sample_gain),
            (0x468, &mut self.u468),
            (0x46c, &mut self.noise_attack),
            (0x470, &mut self.u470),
            (0x474, &mut self.noise_decay2),
            (0x478, &mut self.sample_attack),
            (0x47c, &mut self.u47c),
            (0x480, &mut self.crispel_decay2),
            (0x488, &mut self.freq_top),
            (0x48c, &mut self.freq_base),
            (0x490, &mut self.freq),
            (0x494, &mut self.res_top),
            (0x498, &mut self.res_base),
            (0x49c, &mut self.u49c),
            (0x4a0, &mut self.hp_freq),
            (0x4a4, &mut self.delay_ms),
            (0x4ac, &mut self.am_random),
            (0x4b0, &mut self.am_step),
            (0x4b4, &mut self.am_phase),
            (0x4c4, &mut self.vel_lo),
            (0x4c8, &mut self.vel_hi),
            (0x4cc, &mut self.vel_bend1),
            (0x4d0, &mut self.vel_bend2),
            (0x5a0, &mut self.mod_rotate),
            (0x5a4, &mut self.mod_5a4),
            (0x5a8, &mut self.mod_noise_amount),
            (0x5ac, &mut self.mod_attack),
            (0x5b0, &mut self.mod_5b0),
            (0x5b4, &mut self.mod_crispel_decay2),
            (0x5b8, &mut self.mod_noise_decay2),
            (0x5bc, &mut self.mod_freq),
            (0x5c0, &mut self.mod_5c0),
        ] {
            v(o, Slot::F(x));
        }
        for (i, x) in self.u4d4.iter_mut().enumerate() {
            v(0x4d4 + 4 * i, Slot::F(x));
        }
        v(0x43c, Slot::U(&mut self.noise_index));
        v(0x484, Slot::U(&mut self.u484));
        v(0x4a8, Slot::U(&mut self.am_seed));
        v(0x4b8, Slot::U(&mut self.delay));
        v(0x4bc, Slot::U(&mut self.pending));
        v(0x4c0, Slot::U(&mut self.u4c0));
        v(0x4e4, Slot::U(&mut self.am_start));
        v(0x4e8, Slot::U(&mut self.am_count));
        for i in 0..QUEUE {
            v(0x4ec + 4 * i, Slot::F(&mut self.queue_vel[i]));
            v(0x53c + 4 * i, Slot::U(&mut self.queue_wait[i]));
            v(0x58c + i, Slot::B(&mut self.queue_due[i]));
        }
    }

    /// Writes the state as NI lays it out at `img[base..]` (the `data`/`block` pointers
    /// are skipped).
    #[cfg(test)]
    pub(super) fn image(&self, img: &mut [u8], base: usize) {
        self.clone().walk(&mut |o, s| s.put(img, base + o));
    }
}

/// A field for [`AttachedBodies::walk`].
#[cfg(test)]
pub(super) enum Slot<'a> {
    F(&'a mut f32),
    U(&'a mut u32),
    I(&'a mut i32),
    D(&'a mut f64),
    B(&'a mut bool),
}

#[cfg(test)]
impl Slot<'_> {
    /// Write the field at `img[o..]`.
    pub(super) fn put(self, img: &mut [u8], o: usize) {
        match self {
            Slot::F(x) => img[o..o + 4].copy_from_slice(&x.to_le_bytes()),
            Slot::U(x) => img[o..o + 4].copy_from_slice(&x.to_le_bytes()),
            Slot::I(x) => img[o..o + 4].copy_from_slice(&x.to_le_bytes()),
            Slot::D(x) => img[o..o + 8].copy_from_slice(&x.to_le_bytes()),
            Slot::B(x) => img[o] = *x as u8,
        }
    }

    /// Read the field from `img[o..]`.
    pub(super) fn get(self, img: &[u8], o: usize) {
        let w = |n: usize| -> [u8; 8] {
            let mut a = [0; 8];
            a[..n].copy_from_slice(&img[o..o + n]);
            a
        };
        match self {
            Slot::F(x) => *x = f32::from_le_bytes(w(4)[..4].try_into().unwrap()),
            Slot::U(x) => *x = u32::from_le_bytes(w(4)[..4].try_into().unwrap()),
            Slot::I(x) => *x = i32::from_le_bytes(w(4)[..4].try_into().unwrap()),
            Slot::D(x) => *x = f64::from_le_bytes(w(8)),
            Slot::B(x) => *x = img[o] != 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The NI byte offsets `walk` covers (pointers excluded).
    fn mask() -> Vec<bool> {
        let mut m = vec![false; 0x5c8];
        AttachedBodies::new(false).walk(&mut |o, s| {
            let n = match s {
                Slot::D(_) => 8,
                Slot::B(_) => 1,
                _ => 4,
            };
            m[o..o + n].iter_mut().for_each(|x| *x = true);
        });
        m
    }

    fn load(dir: &str, name: &str, am: bool) -> AttachedBodies {
        let img = std::fs::read(format!("{dir}/{name}.bin")).unwrap();
        let mut a = AttachedBodies::new(am);
        a.walk(&mut |o, s| s.get(&img, o));
        a.sampler.data = u64::from_le_bytes(img[0xf0..0xf8].try_into().unwrap()) as usize;
        a.sampler.block = u64::from_le_bytes(img[0xf8..0x100].try_into().unwrap()) as usize;
        a
    }

    fn same(dir: &str, name: &str, a: &AttachedBodies, ptrs: bool) {
        let want = std::fs::read(format!("{dir}/{name}.bin")).unwrap();
        let mut got = want.clone();
        a.image(&mut got, 0);
        let m = mask();
        let bad: Vec<_> = (0..0x5c8).filter(|&i| m[i] && got[i] != want[i]).map(|i| format!("{i:#x}")).collect();
        assert!(bad.is_empty(), "{name}: bytes differ at {bad:?}");
        if ptrs {
            assert_eq!(a.sampler.data as u64, u64::from_le_bytes(want[0xf0..0xf8].try_into().unwrap()), "{name} data");
        }
    }

    /// Bit-exact against NI dumps made by scratchpad attached/att.c
    /// (`CYMBAL_ORACLE_ATTACHED=<out dir>`).
    #[test]
    #[ignore]
    fn oracle() {
        let Ok(dir) = std::env::var("CYMBAL_ORACLE_ATTACHED") else { return };
        let dir = dir.as_str();
        for v in 0..2 {
            same(dir, &format!("ctor_{v}"), &AttachedBodies::new(v == 1), false);
        }
        for sr in [44100, 48000, 96000] {
            let srf = sr as f32;
            for v in 0..2usize {
                let am = v == 1;
                let init = format!("init_{v}_{sr}");
                let mut a = load(dir, &init, am);
                a.set_sample_rate(srf * 0.5 + 1000.0, 64);
                same(dir, &format!("ssr_{v}_{sr}"), &a, true);
                let mut a = load(dir, &init, am);
                let mut steps: Vec<Box<dyn Fn(&mut AttachedBodies)>> = vec![
                    Box::new(|a| a.set_modulation_crispel_decay2(10.0)),
                    Box::new(|a| a.set_modulation_crispel_decay2(-45.0)),
                    Box::new(|a| a.set_modulation_noise_decay2(-12.5)),
                    Box::new(|a| a.set_modulation_rotate_speed(23.3)),
                    Box::new(|a| a.set_modulation_rotate_speed(70.0)),
                ];
                if am {
                    steps.push(Box::new(|a| a.set_modulation_amount_noise(0.7)));
                    steps.push(Box::new(|a| a.set_modulation_amount_noise(-1.9)));
                    steps.push(Box::new(|a| a.set_modulation_attack(-7.0)));
                    steps.push(Box::new(|a| a.set_modulation_attack(41.0)));
                }
                for (i, s) in steps.iter().enumerate() {
                    s(&mut a);
                    same(dir, &format!("set_{v}_{sr}_{i}"), &a, true);
                }
                for poked in [false, true] {
                    let mut a = load(dir, &init, am);
                    if poked {
                        a.res_base = 0.2;
                        a.res_top = 0.6;
                        a.mod_freq = 3.0;
                        a.delay = 200;
                        a.sampler.reset_on_note = true;
                        a.noise.reset_on_note = true;
                        if am {
                            a.am_amount = 0.8;
                            a.am_rate = 7.0;
                            a.am_jitter = 0.3;
                            a.am_shape = 0.4;
                            a.am_random_depth = 0.5;
                            a.am_start = 100;
                            a.am_step = 7.0 / srf;
                        }
                        a.set_modulation_rotate_speed(12.0);
                        a.set_modulation_crispel_decay2(-8.0);
                        a.set_modulation_noise_decay2(6.0);
                    }
                    let p = poked as u8;
                    let raw = std::fs::read(format!("{dir}/out_{v}_{sr}_{p}.f32")).unwrap();
                    a.set_note_on(if poked { 0.9 } else { 0.8 });
                    for (i, w) in raw.chunks(4).enumerate() {
                        if !poked {
                            match i {
                                1000 => a.set_note_on(0.5),
                                1001 => a.set_note_on(1.0),
                                15000 => a.set_note_on(0.05),
                                _ => {}
                            }
                        } else if i == 7000 {
                            a.set_note_on(0.6);
                            a.set_note_on(0.7);
                        } else if i == 20000 {
                            a.set_note_on(0.95);
                        }
                        let y = a.process_sample();
                        let want = f32::from_le_bytes(w.try_into().unwrap());
                        assert_eq!(y.to_bits(), want.to_bits(), "v{v} sr{sr} poked{p} sample {i}: {y} vs {want}");
                    }
                    same(dir, &format!("end_{v}_{sr}_{p}"), &a, true);
                }
            }
        }
    }
}
