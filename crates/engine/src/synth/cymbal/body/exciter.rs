//! `NI::DSP::UTIL::BodyExciter<false>` (at `CymbalBodyCore +0x778`): the strike that
//! drives the resonators, an envelope through a 20 Hz high-pass and a state-variable
//! filter (its low-pass output). The `false` variant never reads its 256 KB delay
//! buffer; it only clears it.

use super::super::memo::{K, MemoEnvelope};
use super::air::{OnePole, Svf, curve};
use super::img::layout;
use crate::fx::exp_e;

/// Bytes of the cleared delay buffer at +0x10c.
pub(in super::super) const BUF: usize = 0x40000;

layout! {
    /// `BodyExciter<false>` (0x40138 bytes; +0x10c..+0x4010c is the unused buffer).
    Exciter [0x40138] {
        /// Attack, decay and filter pitch as set (the modulations add to them).
        0x00 attack: f32,
        0x04 decay: f32,
        0x08 freq: f32,
        /// Filter cutoff and resonance applied at the sample rate.
        0x0c filt_freq: f32,
        0x10 filt_q: f32,
        0x14 u14: [f32; 2],
        /// Attack, decay and pitch modulations.
        0x1c attack_mod: f32,
        0x20 decay_mod: f32,
        0x24 freq_mod: f32,
        0x28 u28: [f32; 2],
        0x30 env: MemoEnvelope,
        0xb0 svf: Svf,
        /// The 20 Hz high-pass.
        0xe8 hp: OnePole,
        0x108 sr2: f32,
        0x4010c u4010c: [f32; 2],
        0x40114 sr: f32,
        0x40118 inv_sr: f32,
        /// Velocity curve.
        0x4011c curve: [f32; 4],
        0x4012c u4012c: f32,
        0x40130 u40130: u32,
        0x40134 u40134: f32,
    }
}

impl Exciter {
    pub fn new() -> Exciter {
        let mut env = MemoEnvelope::default();
        env.sr = 1.0;
        env.inv_sr = 1.0;
        env.hold_len = f64::from(super::ec(602, f32::from_bits(0x3d58_c000)) + f32::from_bits(0xba83_126f));
        env.decay_step = f64::from(super::ec(1797, f32::from_bits(0x3f72_7000)));
        env.sustain = 1.0;
        let mut svf = Svf::new();
        svf.freq = 0.0;
        let mut hp = OnePole::new();
        hp.a = f32::from_bits(0x3203_7e85);
        hp.gm1 = f32::from_bits(0x4cf9_3280);
        hp.g = f32::from_bits(0x4cf9_3280);
        hp.freq = 20.0;
        Exciter {
            attack: 0.0,
            decay: 0.0,
            freq: 0.0,
            filt_freq: 0.0,
            filt_q: 0.0,
            u14: [0.0; 2],
            attack_mod: 0.0,
            decay_mod: 0.0,
            freq_mod: 0.0,
            u28: [0.0; 2],
            env,
            svf,
            hp,
            sr2: 1.0,
            u4010c: [0.0; 2],
            sr: 1.0,
            inv_sr: 1.0,
            curve: [0.0; 4],
            u4012c: 0.0,
            u40130: 1,
            u40134: 0.0,
        }
    }

    /// `setSampleRate(sr)` (0x1cd55a0).
    pub fn set_sample_rate(&mut self, sr: f32) {
        if !(1.0..=500_000.0).contains(&sr) {
            return;
        }
        let inv = 1.0 / sr;
        self.sr = sr;
        self.inv_sr = inv;
        self.env.set_sample_rate(sr);
        self.hp.set_inv_sr(inv);
        let hp_freq = self.hp.freq;
        self.svf.sr = sr;
        self.svf.inv_sr = inv;
        let old = self.svf.freq;
        self.svf.freq = -1.0;
        if old != -1.0 {
            self.svf.set_freq(old);
        }
        self.sr2 = sr;
        if self.svf.freq != self.filt_freq {
            self.svf.set_freq(self.filt_freq);
        }
        if self.svf.q != self.filt_q {
            self.svf.set_q(self.filt_q);
        }
        if hp_freq != 20.0 {
            self.hp.set_freq(20.0);
        }
    }

    /// `setNoteOn(v)` (0x1cd5458).
    pub fn set_note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        let x = curve(v, self.curve);
        if x > 0.0 {
            self.env.set_note_on(x);
        }
    }

    /// `BodyCore::setModulationExcAttack` as it reaches the exciter.
    pub fn set_attack_mod(&mut self, m: f32) {
        self.attack_mod = m;
        let a = self.attack + m;
        self.env.attack = a;
        self.env.attack_step = f64::from(exp_e((60.0 - a).mul_add(K, 1200.0))) * self.env.inv_sr;
    }

    /// `BodyCore::setModulationExcDecay` as it reaches the exciter (the envelope's
    /// release).
    pub fn set_decay_mod(&mut self, m: f32) {
        self.decay_mod = m;
        let d = self.decay + m;
        self.env.release = d;
        let r = f64::from(exp_e((60.0 - d).mul_add(K, 1200.0))) * self.env.inv_sr;
        self.env.release_mul = if r >= 1.0 { 0.0 } else { 1.0 - r };
    }

    /// `BodyCore::setModulationExcFreq` as it reaches the exciter.
    pub fn set_freq_mod(&mut self, m: f32) {
        self.freq_mod = m;
        let f = exp_e((self.freq + m).mul_add(5.0, f32::from_bits(0x44ac_bc36)));
        if self.svf.freq != f {
            self.svf.set_freq(f);
        }
    }

    /// `processSample()` (0x1cd52f4): the filtered envelope (low-pass output).
    #[inline]
    pub fn process(&mut self) -> f32 {
        let e = self.env.process();
        let x = self.hp.hp(e);
        self.svf.tick(x).2
    }
}
