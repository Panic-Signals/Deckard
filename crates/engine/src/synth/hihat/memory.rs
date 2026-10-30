//! `NI::DSP::IFFTCymbalCore` (docs/synth-hihat.md §4): the Memory engine.
//!
//! It loops one of six recorded cymbal spectra (`Source` A–F, 131072 samples each, NI's
//! data read from the user's Maschine install by [`super::res`]) at a pitch-dependent
//! rate, with Hermite interpolation and a phase modulation by band-passed white noise
//! under a 1 s envelope (the "Strike" shimmer). The loop goes through a sample-and-hold
//! decimator, a 250 Hz high-pass, (with Distress) a second decimator, a 5 kHz shelf and a
//! 600 Hz high-pass, two peaking EQs (Color), a 6 kHz band-pass whose envelope follower
//! normalises the level, three decay envelopes, a drive, a soft clipper and a 200 Hz
//! high-pass. Field comments give the member offsets; arithmetic follows the arm64
//! code (`f32`, fused multiply-adds where it uses `fmadd`).

use super::c606::{PI, Svf, clamp01, pow2_cubic, tan3};
use super::env::FreeEnv;
use crate::fx::exp_e;

/// `log2(10)·3`: dB → exp-table steps (`0x411f73da`).
const K: f32 = f32::from_bits(0x411f_73da);
/// The exp table's pitch offset (`0x44acbc36`).
const P0: f32 = f32::from_bits(0x44ac_bc36);
/// Semitone offsets of the six sources (`s_afPitchConsts`).
const PITCH: [f32; 6] = [-8.0, -7.0, 4.0, 11.0, -7.0, 3.0];
/// Phase-modulation depth of the six sources (`s_afPMAmtConsts`).
const PM_AMT: [u32; 6] = [0x3f80_0000, 0x3fa6_6666, 0x3f19_999a, 0x3f33_3333, 0x3fc0_0000, 0x3fa6_6666];
const MASK: u32 = 0x1ffff;

/// macOS `rand()` (Park–Miller, seed 1): `onReset` draws the loop start from it.
pub(super) mod rand {
    use std::sync::atomic::{AtomicU32, Ordering};
    static STATE: AtomicU32 = AtomicU32::new(1);

    /// The next value, as libc's `rand()`.
    pub fn next() -> u32 {
        let mut x = STATE.load(Ordering::Relaxed);
        loop {
            let n = ((x as u64 * 16807) % 0x7fff_ffff) as u32;
            match STATE.compare_exchange(x, n, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return n,
                Err(y) => x = y,
            }
        }
    }

    /// Restart the sequence after `n` draws (for tests and null runs).
    #[allow(dead_code)]
    pub fn reset(n: u32) {
        STATE.store(1, Ordering::Relaxed);
        for _ in 0..n {
            next();
        }
    }
}

/// A TPT one-pole, high-pass output (`y − z`).
#[derive(Clone, Copy, Default)]
struct OnePole {
    z: f32,
    a: f32,
    b: f32,
}

impl OnePole {
    fn new(inv: f32, fc: f32) -> OnePole {
        let g = tan3(inv * (fc * PI));
        OnePole { z: 0.0, a: 1.0 / (g + 1.0), b: g + -1.0 }
    }

    fn hp(&mut self, x: f32) -> f32 {
        let y = self.a * (-self.b).mul_add(self.z, x);
        let o = y - self.z;
        self.z = y;
        o
    }
}

/// A sample-and-hold decimator: a phase accumulator in [−½, ½] that takes a new sample
/// each wrap; the held sample is mixed in by `mix`.
#[derive(Clone, Copy, Default)]
struct Decim {
    acc: f32,
    rate: f32,
    held: f32,
    mix: f32,
}

impl Decim {
    fn tick(&mut self, y: f32) -> f32 {
        let mut s = self.rate + self.acc;
        let held = if s > 0.5 {
            self.held = y;
            s += -1.0;
            y
        } else {
            self.held
        };
        let s = if s < -0.5 { -0.5 } else { s };
        self.acc = if s > 0.5 { 0.5 } else { s };
        (held - y).mul_add(clamp01(self.mix), y)
    }
}

/// The peaking EQs: a band-pass `y` (bilinear, `g·(x − x[n−2])`) added back with `gain`.
#[derive(Clone, Copy, Default)]
struct Peak {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
    g: f32,
    norm: f32,
    c1: f32,
    c2: f32,
    gain: f32,
}

impl Peak {
    fn set(&mut self, g: f32, gk: f32, gain: f32) {
        let g2 = g * g;
        self.g = g;
        let p = g2 + 1.0;
        self.norm = 1.0 / (p + gk);
        let m = g2 + -1.0;
        self.c1 = m + m;
        self.c2 = p - gk;
        self.gain = gain;
    }

    /// The band-pass part.
    fn bp(&mut self, x: f32) -> f32 {
        let s = self.g * (x - self.x2);
        self.x2 = self.x1;
        self.x1 = x;
        let s = (-self.c1).mul_add(self.y1, s);
        let s = (-self.c2).mul_add(self.y2, s);
        let y = self.norm * s;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// `UTIL::BiQuad` type 2 (a shelf), as the Snare port has it (`synth/snare/clap/dsp.rs`).
#[derive(Clone, Copy, Default)]
struct Shelf {
    z1: f32,
    z2: f32,
    g2: f32,
    a1: f32,
    a2: f32,
    b0: f32,
    b1: f32,
    b2: f32,
    q: f32,
    gain: f32,
}

/// BiQuad's `2^x` (quadratic).
fn pow2_bq(x: f32) -> f32 {
    let i = x.round_ties_even();
    let n = if i < 2_147_483_648.0 && i > -2_147_483_648.0 { i as i32 } else { i32::MIN };
    let e = f32::from_bits(0x3f80_0000u32.wrapping_add((n as u32).wrapping_shl(23)));
    let fr = x - n as f32;
    let p = fr.mul_add(f32::from_bits(0x3e7d_f3b6), f32::from_bits(0x3f35_3f7d));
    p.mul_add(fr, 1.0) * e
}

impl Shelf {
    fn calc(&mut self) {
        let f = f32::from_bits;
        let g = self.g2;
        let g2 = g * g;
        let sp = g2.mul_add(f(0xb2cd_4ca8), f(0x3638_b94f));
        let sp = sp.mul_add(g2, f(0xb950_0c03));
        let sp = sp.mul_add(g2, f(0x3c08_8885));
        let sp = sp.mul_add(g2, f(0xbe2a_aac1));
        let sin = g * sp.mul_add(g2, 1.0);
        let cp = g2.mul_add(f(0xb48b_dad9), f(0x37cf_b5a1));
        let cp = cp.mul_add(g2, f(0xbab6_09bd));
        let cp = cp.mul_add(g2, f(0x3d2a_aa99));
        let cp = cp.mul_add(g2, f(0xbeff_f5e8));
        let cos = g2.mul_add(cp, 1.0);
        let alpha = (1.0 - self.q) * sin;
        let big_a = pow2_bq(self.gain / 40.0 * f(0x4054_9a80));
        let ap = big_a + 1.0;
        let am = big_a + -1.0;
        let sq = pow2_bq(big_a.ln() * f(0x3fb8_aa3b) * 0.5);
        let s0 = (alpha + alpha) * sq;
        let s1 = (-am).mul_add(cos, ap);
        let s2 = 1.0 / (s1 + s0);
        let s3 = am.mul_add(cos, ap);
        self.b0 = s2 * (big_a * (s3 + s0));
        self.b1 = big_a * (ap.mul_add(cos, am) * -2.0) * s2;
        self.b2 = s2 * (big_a * (s3 - s0));
        let t = (-ap).mul_add(cos, am);
        self.a1 = -(s2 * (t + t));
        self.a2 = -(s2 * (s1 - s0));
    }

    fn tick(&mut self, x: f32) -> f32 {
        let w = self.a1.mul_add(self.z1, x);
        let w = self.a2.mul_add(self.z2, w);
        let y = self.b0.mul_add(w, self.z1 * self.b1);
        let y = self.b2.mul_add(self.z2, y);
        self.z2 = self.z1;
        self.z1 = w;
        y
    }
}

/// A simple release envelope (gate, choke): 0 off, 1 on, 2 releasing.
#[derive(Clone, Copy, Default)]
struct Release {
    st: u32,
    lv: f32,
    k: f32,
}

impl Release {
    fn tick(&mut self) -> f32 {
        match self.st {
            0 => 0.0,
            1 => 1.0,
            _ => {
                let l = self.k * self.lv;
                self.lv = l;
                if (l as f64) < f64::from_bits(0x3f1a_36e2_eb1c_432d) {
                    self.lv = 0.0;
                    self.st = 0;
                    0.0
                } else {
                    l
                }
            }
        }
    }

    fn release(&mut self) {
        if self.st == 1 {
            self.st = 2;
            self.lv = 1.0;
        }
    }
}

pub(super) struct Memory {
    pub p: [f32; 7],   // +0x80: tune, decay, strike, source, color, distress, gate
    tune_st: f32,      // +0x9c
    decay_db: f32,     // +0xa0
    source: usize,     // +0xa8
    pub gate: bool,    // +0xac
    attack: f32,       // +0xb0
    pm_decay: f32,     // +0xb4
    freq: f32,         // +0xb8
    note: u32,         // +0xd4
    pos: f32,          // +0xd0
    noise_gain: f32,   // +0xdc
    lcg: i32,          // +0xe4
    dec1: Decim,       // +0x12c
    dec2: Decim,       // +0x110
    hp250: OnePole,    // +0x148
    hp600: OnePole,    // +0x168
    hp8k: OnePole,     // +0x188
    hp200: OnePole,    // +0x1a8
    eq1: Peak,         // +0x1c8
    eq2: Peak,         // +0x208
    shelf: Shelf,      // +0x248
    svf_b: Svf,        // +0x280: 6 kHz band-pass (the follower's input)
    svf_a: Svf,        // +0x2b0: the noise band-pass at the pitch
    env1: FreeEnv,     // +0x2e0
    env2: FreeEnv,     // +0x3d8
    env3: FreeEnv,     // +0x4d0
    env4: FreeEnv,     // +0x5c8: phase-modulation envelope
    gate_env: Release, // +0x6c0
    fol: f32,          // +0x6e0
    fol_att: f32,      // +0x6e8
    fol_rel: f32,      // +0x6ec
    xf_t: f32,         // +0x6f4
    xf_flag: bool,     // +0x6f8
    xf_k: f32,         // +0x6fc
    xf_coef: f32,      // +0x700
    level: f32,        // +0x704
    xf_prev: f32,      // +0x708
    xf_mix: f32,       // +0x70c
    inv: f32,          // +0x710
    thr: f32,          // +0x714
    tail: bool,        // +0x718
    fade_len: u32,     // +0x71c
    fade_inv: f32,     // +0x720
    fade_n: u32,       // +0x724
    clip: f32,         // +0x730
    knee: f32,         // +0x734
    choke: Release,    // +0x738
    // SmoothingToolbox<12> (+0x750): tune st, PM depth, strike gain, decay s, decay mix,
    // EQ gain dB, EQ frequency, decimator mix, Distress mix, Distress pitch, shelf gain,
    // loop rate.
    cur: [f32; 12],
    incs: [f32; 12],
    tgt: [f32; 12],
    start: bool,
    pub smoothing: bool,
    ramp_left: i32,
    ramp_len: i32,
    ramp_inv: f32,
    vel: f32,            // +0x810
    pending_reset: bool, // +0x814
    sr: f32,             // +0x818
    /// Template flag D: recompute every coefficient in the next block.
    pub fresh: bool,
}

impl Memory {
    /// Construction, `onSetSampleRate(44100, 128)`, `onSetSampleRate(sr, 128)`.
    pub fn new(sr: f32) -> Memory {
        let inv = 1.0 / sr;
        let n = ((sr * f32::from_bits(0x3ca3_d70a)) / 128.0 + 0.5) as i32;
        let n = if n > 1 { n } else { 1 };
        let ln2 = f32::from_bits(0x3f31_7215);
        let fol = |t: f32| if t * sr <= 1.0 { ln2 } else { ln2 / (sr * t) };
        let rel = |t: f32| {
            let k = ((inv * f32::from_bits(0xbf31_7215)) / t).exp();
            if k > 1.0 { 1.0 } else { k }
        };
        let svf = |fc: f32, k: f32| {
            let mut s = Svf { k, sr, inv, ..Svf::default() };
            s.set(fc);
            s
        };
        let fl = (sr * f32::from_bits(0x3c75_c290)) as u32;
        let fl = if fl > 1 { fl } else { 1 };
        let mut shelf = Shelf { q: f32::from_bits(0x3e4c_cccd), ..Shelf::default() };
        let lo = sr / 24576.0;
        let hi = sr / 2.125;
        let c = if lo > 5000.0 { lo } else { 5000.0 };
        let c = if c > hi { hi } else { c };
        let g = tan3(inv * (c * PI));
        shelf.g2 = g + g;
        shelf.calc();
        let mut m = Memory {
            p: [0.0; 7],
            tune_st: 0.0,
            decay_db: 0.0,
            source: 0,
            gate: false,
            attack: 0.0,
            pm_decay: 0.0,
            freq: 1000.0,
            note: 60,
            pos: 0.0,
            noise_gain: (sr / 44100.0).sqrt() * f32::from_bits(0x3000_009d),
            lcg: 0,
            dec1: Decim { mix: 0.25, ..Decim::default() },
            dec2: Decim::default(),
            hp250: OnePole::new(inv, 250.0),
            hp600: OnePole::new(inv, 600.0),
            hp8k: OnePole::new(inv, 8000.0),
            hp200: OnePole::new(inv, 200.0),
            eq1: Peak::default(),
            eq2: Peak::default(),
            shelf,
            svf_b: svf(f32::min(sr * f32::from_bits(0x3efa_e148), 6000.0), f32::from_bits(0x3fcc_cccd)),
            svf_a: svf(f32::min(sr * f32::from_bits(0x3efa_e148), 1000.0), f32::from_bits(0x3ecc_cccc)),
            env1: FreeEnv::new(sr, 0.001, 0.5),
            env2: FreeEnv::new(sr, 0.001, 1.0),
            env3: FreeEnv::new(sr, 0.001, f32::from_bits(0x3ecc_cccd)),
            // Its attack is set to 0 (clamped to one sample).
            env4: FreeEnv::new(sr, (1.0 / sr as f64) as f32, f32::from_bits(0x3f33_3333)),
            gate_env: Release { k: rel(f32::from_bits(0x3a83_126f)), ..Release::default() },
            fol: 0.0,
            fol_att: fol(f32::from_bits(0x3a83_126f)),
            fol_rel: fol(f32::from_bits(0x3c23_d70a)),
            xf_t: 1.0,
            xf_flag: true,
            xf_k: 0.0,
            xf_coef: 0.0,
            level: 0.0,
            xf_prev: 0.0,
            xf_mix: 0.0,
            inv,
            thr: f32::from_bits(0x38d1_b717),
            tail: false,
            fade_len: fl,
            fade_inv: 1.0 / fl as f32,
            fade_n: 0,
            clip: 12.0,
            knee: f32::from_bits(0x3e2a_aaab),
            choke: Release { k: rel(f32::from_bits(0x3ba3_d70a)), ..Release::default() },
            cur: [0.0; 12],
            incs: [0.0; 12],
            tgt: [0.0; 12],
            start: true,
            smoothing: false,
            ramp_left: 0,
            ramp_len: n,
            ramp_inv: 1.0 / (n * 128) as f32,
            vel: 0.0,
            pending_reset: false,
            sr,
            fresh: true,
        };
        m.set_tune(0.0);
        m
    }

    /// The recording of the current source (silence without an install).
    fn buf(&self) -> Option<&'static [f32]> {
        super::res::cymbal(self.source)
    }

    /// `setParameter(i, v)`.
    pub fn set_param(&mut self, i: usize, v: f32) {
        if i > 6 {
            return;
        }
        self.p[i] = v;
        match i {
            0 => self.set_tune(v),
            1 => self.set_decay(v),
            2 => self.set_strike(v),
            3 => {
                if (0.0..=5.0).contains(&v) {
                    self.source = v as u32 as usize;
                }
            }
            4 => {
                let a = clamp01(v / f32::from_bits(0x3f37_0a3d));
                let s1 = a.mul_add(24.0, -24.0);
                let b = (v + f32::from_bits(0xbf37_0a3d)) / f32::from_bits(0x3e91_eb86);
                let b = if b < 0.0 { 0.0 } else { b };
                let s2 = if b > 1.0 { 12.0 } else { b * 12.0 };
                let st = if v <= f32::from_bits(0x3f37_0a3d) { s1 } else { s2 };
                self.tgt[6] = (st / 12.0).exp2() * f32::from_bits(0x4592_e000);
                let x = v + v;
                let x = if x < 0.0 { 0.0 } else { x };
                self.tgt[5] = if x > 1.0 { -24.0 } else { x * -24.0 };
                self.start = true;
            }
            5 => {
                self.set_distress_targets();
                self.start = true;
            }
            _ => self.gate = v > 0.0,
        }
    }

    /// The Distress-dependent targets (`setParameter(5)` and `setParamTune`).
    fn set_distress_targets(&mut self) {
        let st = self.tune_st;
        let half = if st > 0.0 { st } else { st * 0.5 };
        let x = clamp01(self.p[5]);
        let s1 = 1.0 - x;
        let s1 = s1 * (2.0 - s1);
        let s3 = x * (2.0 - x);
        let s4 = s1.mul_add(0.0, s3 * -2.0);
        self.tgt[9] = half + (s4 + 125.0);
        self.tgt[8] = s1.mul_add(0.0, s3 * f32::from_bits(0x3f66_6666));
        self.tgt[10] = s1.mul_add(0.0, s3 * 3.0);
    }

    /// `setParamTune(v)`.
    fn set_tune(&mut self, v: f32) {
        let t = clamp01(v).mul_add(24.0, -12.0) + (self.note as f32 + -60.0);
        let t = if t < -12.0 { -12.0 } else { t };
        let st = if t > 12.0 { 12.0 } else { t };
        let c = st / -5.0;
        let c = if c < 0.0 { 0.0 } else { c };
        let s11 = if c > 1.0 { f32::from_bits(0x3f59_999a) } else { c * f32::from_bits(0x3f59_999a) };
        let e = (st / 12.0).exp2();
        self.tune_st = st;
        self.tgt[0] = st;
        self.tgt[7] = s11;
        self.tgt[11] = (e * 44100.0) / self.sr;
        self.set_distress_targets();
        self.update_pitch();
        self.start = true;
    }

    /// The pitch frequency, phase-modulation depth and its envelope's decay (shared by
    /// Tune, Decay and Strike).
    fn update_pitch(&mut self) {
        let f = exp_e(((PITCH[self.source] + 70.0) + self.tune_st).mul_add(5.0, P0));
        self.freq = f;
        let a = f32::from_bits(0x383b_3f2a) / (f.sqrt() * f32::from_bits(0x3bdf_266c));
        let c = clamp01((self.decay_db + f32::from_bits(0xc224_0000)) / -5.0);
        let pm = f32::from_bits(PM_AMT[self.source]) * self.p[2];
        let k = c.mul_add(f32::from_bits(0xbecc_ccd0), f32::from_bits(0x4019_999a));
        self.tgt[1] = a * (pm * k);
        self.pm_decay = c.mul_add(f32::from_bits(0x3df5_c290), f32::from_bits(0x3da3_d70a));
    }

    fn set_decay(&mut self, v: f32) {
        let d = clamp01(v.abs().mul_add(f32::from_bits(0xbe4c_cccd), f32::from_bits(0x3f99_999a)) * v);
        let db = d.mul_add(34.0, 24.0);
        self.decay_db = db;
        self.tgt[3] = exp_e(db.mul_add(K, f32::from_bits(0x4416_8363)));
        self.tgt[4] = clamp01((db + -42.0) / -7.0);
        self.update_pitch();
        self.start = true;
    }

    fn set_strike(&mut self, v: f32) {
        self.p[2] = v;
        self.tgt[2] = exp_e(clamp01(v).mul_add(10.0, 0.0).mul_add(K, 1200.0));
        let s = v * 10.0;
        let s = if s > 1.0 { 1.0 } else { s };
        let s = if s < 0.0 { 0.0 } else { s };
        self.attack = s.mul_add(f32::from_bits(0xbba3_d70a), f32::from_bits(0x3bc4_9ba6));
        self.update_pitch();
        self.start = true;
    }

    /// Whether `processAudio` runs (`hasOutputSignal`).
    pub fn active(&self) -> bool {
        let sounding = self.env1.active() || self.env2.active() || self.env3.active();
        sounding && (!self.gate || self.gate_env.st != 0) && self.choke.st != 0
    }

    /// `setNoteOn(v, note)`.
    pub fn note_on(&mut self, v: f32, note: u32) {
        if !(0.0..=1.0).contains(&v) {
            self.vel = 0.0;
            return;
        }
        self.note = note;
        self.set_tune(self.p[0]);
        if self.active() {
            self.cur[..4].copy_from_slice(&self.tgt[..4]);
            self.cur[4] = self.tgt[4];
            self.cur[7] = self.tgt[7];
            self.cur[8] = self.tgt[8];
            self.cur[9] = self.tgt[9];
            self.cur[11] = self.tgt[11];
        } else {
            self.cur = self.tgt;
            self.start = true;
        }
        for e in [&mut self.env1, &mut self.env3, &mut self.env2, &mut self.env4] {
            e.note_on(v);
        }
        self.env1.set_attack(self.attack);
        self.env3.set_attack(self.attack);
        self.env2.set_attack(self.attack);
        self.env4.set_decay(self.pm_decay);
        if self.svf_a.f != self.freq {
            self.svf_a.set(self.freq);
        }
        if self.gate {
            self.gate_env.st = 1;
            self.gate_env.lv = 0.0;
        }
        self.xf_k = 1.0;
        self.xf_prev = self.level;
        if !self.xf_flag {
            self.xf_coef = self.xf_factor();
        }
        self.tail = true;
        self.fade_n = 0;
        self.choke.st = 1;
        self.choke.lv = 0.0;
        self.vel = v;
        self.pending_reset = true;
        self.fresh = true;
    }

    fn xf_factor(&self) -> f32 {
        let s1 = self.inv * -3.0;
        let lim = self.inv * f32::from_bits(0x4a00_2c80);
        let t = self.xf_t;
        let t = if t < f32::from_bits(0x38d1_b717) { f32::from_bits(0x38d1_b717) } else { t };
        let t = if lim < t { lim } else { t };
        pow2_cubic(s1 / t)
    }

    pub fn note_off(&mut self) {
        self.gate_env.release();
    }

    pub fn choke(&mut self) {
        self.choke.release();
    }

    /// `onReset()`: the loop restarts at a random position (`rand()`).
    pub fn reset(&mut self) {
        self.vel = 0.0;
        self.pending_reset = false;
        self.cur = self.tgt;
        self.start = true;
        for f in [&mut self.hp250, &mut self.hp600, &mut self.hp8k, &mut self.hp200] {
            f.z = 0.0;
        }
        self.shelf.z1 = 0.0;
        self.shelf.z2 = 0.0;
        for q in [&mut self.eq1, &mut self.eq2] {
            q.x1 = 0.0;
            q.x2 = 0.0;
            q.y1 = 0.0;
            q.y2 = 0.0;
        }
        for s in [&mut self.svf_a, &mut self.svf_b] {
            s.ic1 = 0.0;
            s.ic2 = 0.0;
        }
        self.dec1.acc = 0.0;
        self.dec2.acc = 0.0;
        for e in [&mut self.env1, &mut self.env3, &mut self.env2, &mut self.env4] {
            e.reset();
        }
        self.gate_env.st = 0;
        self.gate_env.lv = 0.0;
        self.choke.st = 0;
        self.choke.lv = 0.0;
        let r = rand::next() as i32 as f32;
        self.pos = (r * 131072.0) * f32::from_bits(0x3000_0000);
        self.fresh = true;
    }

    /// `processControlRate()` (SmoothingToolbox<12>).
    pub fn control(&mut self) {
        if self.start {
            self.start = false;
            let mut any = false;
            for i in 0..12 {
                let d = (self.tgt[i] - self.cur[i]) * self.ramp_inv;
                self.incs[i] = d;
                if ((d * d) as f64) < f64::from_bits(0x3cd2_03af_9ee7_5616) {
                    self.incs[i] = 0.0;
                    self.cur[i] = self.tgt[i];
                } else {
                    any = true;
                }
            }
            if any {
                self.ramp_left = self.ramp_len;
                self.smoothing = true;
            }
            return;
        }
        self.ramp_left = self.ramp_left.wrapping_sub(1);
        if self.ramp_left == 0 {
            self.cur = self.tgt;
            self.incs = [0.0; 12];
            self.smoothing = false;
        }
    }

    /// `processAudio`: one block of at most 16 samples.
    pub fn process(&mut self, out: &mut [f32]) {
        if !self.active() {
            if self.pending_reset {
                self.reset();
            }
            return;
        }
        let Some(buf) = self.buf() else { return };
        let (fresh, sm) = (self.fresh, self.smoothing);
        let rc = |i: usize, incs: &[f32; 12]| fresh || (sm && incs[i] != 0.0);
        let distress = self.p[5] > 0.0;
        for o in out.iter_mut() {
            if sm {
                for i in 0..12 {
                    self.cur[i] += self.incs[i];
                }
            }
            let incs = self.incs;
            *o = self.sample(buf, distress, |i| rc(i, &incs));
        }
        self.fresh = false;
    }

    fn sample(&mut self, buf: &[f32], distress: bool, rc: impl Fn(usize) -> bool) -> f32 {
        if rc(3) {
            let d = self.cur[3];
            self.env1.set_decay(if d > f32::from_bits(0x3d8f_5c29) { f32::from_bits(0x3d8f_5c29) } else { d });
            self.env3.set_decay(d);
            self.env2.set_decay(if d > f32::from_bits(0x3df5_c28f) { f32::from_bits(0x3df5_c28f) } else { d });
            let t = d * 0.5;
            let t = if t > 1.0 { 1.0 } else { t };
            if self.xf_t != t {
                self.xf_t = t;
                if self.xf_flag {
                    self.xf_coef = self.xf_factor();
                }
            }
        }
        self.pos += self.cur[11];
        self.lcg = self.lcg.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        let n = self.noise_gain * self.lcg as f32;
        let (_, bpa) = self.svf_a.tick(n);
        let e4 = self.env4.tick();
        let pm = self.cur[1] * (bpa * e4);
        let pos = self.pos;
        let idx = pm.mul_add(44100.0, pos);
        let pu = pos as u32;
        self.pos = (pos - pu as f32) + (pu & MASK) as f32;
        let idx = idx + 1_310_720.0;
        let i = idx as u32;
        let f = idx - i as f32;
        let at = |k: u32| buf[(k & MASK) as usize];
        let (xm, x0, x1, x2) = (at(i.wrapping_sub(1)), at(i), at(i.wrapping_add(1)), at(i.wrapping_add(2)));
        let d1 = (x1 - xm) * 0.5;
        let d2 = (x2 - x0) * 0.5;
        let f2 = f * f;
        let f3 = f * f2;
        let h = (f3 + f3) + f2 * -3.0;
        let h0 = h + 1.0;
        let h1 = f + f2.mul_add(-2.0, f3);
        let h3 = f3 - f2;
        let y = h0.mul_add(x0, -(h * x1));
        let y = h1.mul_add(d1, y);
        let y = h3.mul_add(d2, y);
        if rc(0) {
            let r = exp_e(self.cur[0].mul_add(f32::from_bits(0x3f66_6666), f32::from_bits(0x4307_0f5c)).mul_add(5.0, P0));
            self.dec1.rate = r * self.inv;
        }
        if rc(7) {
            self.dec1.mix = self.cur[7];
        }
        let y = self.dec1.tick(y);
        let mut y = self.hp250.hp(y);
        if distress {
            if rc(9) {
                let r = exp_e(self.cur[9].mul_add(5.0, P0));
                self.dec2.rate = r * self.inv;
            }
            if rc(8) {
                self.dec2.mix = self.cur[8];
            }
            let z = self.dec2.tick(y);
            if rc(10) {
                self.shelf.gain = self.cur[10];
                self.shelf.calc();
            }
            let z = self.shelf.tick(z);
            y = self.hp600.hp(z);
        }
        if rc(5) {
            let gain = self.cur[5];
            let g = tan3(self.inv * f32::from_bits(0x4666_b5f2)); // 4700π
            let a = pow2_cubic(gain * f32::from_bits(0x3daa_aa7e));
            let k = f32::from_bits(0x4125_105d) / (a * f32::from_bits(0x4057_4bc6));
            let k = if k < f32::from_bits(0x3c23_d70a) { f32::from_bits(0x3c23_d70a) } else { k };
            let k = if k > 100.0 { 100.0 } else { k };
            self.eq1.set(g, g * k, a.mul_add(a, -1.0) * k);
        }
        if rc(6) {
            let g = tan3((self.cur[6] * PI) * self.inv);
            let k = f32::from_bits(0x3f44_45a9);
            self.eq2.set(g, g * k, f32::from_bits(0x4138_00c0));
        }
        let b1 = self.eq1.bp(y);
        let o1 = self.eq1.gain.mul_add(b1, y);
        let b2 = self.eq2.bp(o1);
        let eqg = exp_e((self.cur[5] * -0.5).mul_add(K, 1200.0));
        let x = self.eq2.gain.mul_add(b2, o1) * eqg;
        let (_, bpb) = self.svf_b.tick(x);
        let d = bpb.abs() - self.fol;
        let c = if d > 0.0 { self.fol_att } else { self.fol_rel };
        self.fol += d * c;
        let norm = if self.fol > f32::from_bits(0x3c23_d70a) { 3.0 / self.fol } else { 300.0 };
        let e1 = self.env1.tick();
        let drive = exp_e((e1 * 8.0).mul_add(K, 1200.0));
        let s14 = x * norm;
        let hp = self.hp8k.hp(s14);
        if rc(2) {
            self.env2.set_gain(self.cur[2].mul_add(f32::from_bits(0x3e99_999a), f32::from_bits(0x3f19_999a)));
        }
        let e3 = self.env3.tick() + f32::from_bits(0xbcf5_c28f);
        let e3 = if e3 < 0.0 { 0.0 } else { e3 };
        let s8 = self.cur[2].mul_add(e1, e3);
        let e2 = self.env2.tick();
        let new = self.vel * (e2 - s8).mul_add(clamp01(self.cur[4]), s8);
        let old = self.xf_k * self.xf_prev;
        self.xf_k *= self.xf_coef;
        let mx = if old < new { new } else { old };
        let lv = ((old + new) - mx).mul_add(clamp01(self.xf_mix), mx);
        let lv = if lv < 0.0 { 0.0 } else { lv };
        self.level = lv;
        let mut fade = 1.0;
        if lv < self.thr {
            let n = if self.fade_n < self.fade_len {
                self.fade_n + 1
            } else {
                self.tail = false;
                self.fade_len
            };
            self.fade_n = n;
            let f = (n as f32).mul_add(-self.fade_inv, 1.0);
            fade = f * (f * f);
        }
        let s2 = ((drive + -1.0) + f32::from_bits(0x3f33_3333)).mul_add(hp, s14);
        let s = s2 * (lv * fade);
        let s = if s < -self.clip { -self.clip } else { s };
        let s = if s > self.clip { self.clip } else { s };
        let s = s * (s.abs() * -0.25).mul_add(self.knee, 1.0);
        let s = self.hp200.hp(s);
        let s = if self.gate { s * self.gate_env.tick() } else { s };
        let ch = self.choke.tick();
        s * f32::from_bits(0x3cfc_0b13) * ch
    }
}
