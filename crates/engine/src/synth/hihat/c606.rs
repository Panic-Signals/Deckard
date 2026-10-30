//! `NI::DSP::Hihat606Core` (docs/synth-hihat.md §3): the Silver and Circuit engines.
//!
//! Silver: six pulse oscillators at 1, 1.25, 1.49, 1.7, 1.79 and 2.55 × the tuned
//! frequency (a 606-style metal bank) through an 8 kHz one-pole low-pass, cross-faded
//! with white noise by Noise. Circuit: six "chaotic" counters clocked at 30 × that
//! frequency, each one stepping at half the rate of the one before (a binary divider),
//! scaled by Dissonance and offset by Seed's random increments. Both then go through a
//! band-pass state-variable filter at Color, an envelope follower that normalises the
//! level, two decay envelopes (a 30 ms click and the main decay), a high-pass SVF at
//! 0.85 × Color, a saturator and a choke/gate envelope.
//!
//! Field names carry the member offset. All arithmetic follows the arm64 code (`f32`,
//! fused multiply-adds where it uses `fmadd`).

use super::env::FreeEnv;
use crate::fx::exp_e;

/// `processAudioImpl` keeps the oscillator increments, filter and decay coefficients from
/// the last "fresh" block (template flag D) unless it is smoothing (flag C); these three
/// control the recomputation.
pub(super) struct Core606 {
    pub p: [f32; 9],       // +0x80: the core parameters as last set
    tune_st: f32,          // +0xa4: semitones of the note + Tune
    pub gate: bool,        // +0xa8
    pub silver_next: bool, // +0xa9: Silver (vs Circuit) for the next note
    silver: bool,          // +0xaa: latched at note-on
    note: u32,             // +0xac
    // Six pulse oscillators (+0xb0 + 0x18·i): phase, (phase+width), frequency, increment,
    // width 1.525, 1/sr.
    osc_ph: [f32; 6],
    osc_ph2: [f32; 6],
    osc_inc: [f32; 6],
    // One-pole low-pass at 8 kHz (+0x140): state, output, 1/(1+g), g−1, g.
    lp_s: f32,
    lp_out: f32,
    lp_a: f32,
    lp_b: f32,
    lp_g: f32,
    noise_gain: f32, // +0x164: sqrt(sr/44100)·2^-31
    lcg: i32,        // +0x16c
    // Circuit (+0x198): six random increments (Seed), clock increment/accumulator, six
    // counters, five divider flags.
    rnd: [f32; 6],
    clk: f32,
    cnt: [f32; 6],
    flags: [bool; 5],
    // Envelope follower (+0x1d8): level, attack/release coefficients.
    fol: f32,
    fol_att: f32,
    fol_rel: f32,
    // Band-pass SVF (+0x1e4) and high-pass SVF (+0x214): ic1, ic2, f, g, k, h.
    bp: Svf,
    hp: Svf,
    // Retrigger crossfade (+0x244).
    xf_t: f32,     // +0x244: time constant (0.4 × decay)
    xf_flag: bool, // +0x248
    xf_k: f32,     // +0x24c: decaying weight of the previous level
    xf_coef: f32,  // +0x250
    level: f32,    // +0x254
    xf_prev: f32,  // +0x258
    xf_mix: f32,   // +0x25c
    inv: f32,      // +0x260
    thr: f32,      // +0x264
    tail: bool,    // +0x268
    fade_len: u32, // +0x26c
    fade_inv: f32, // +0x270
    fade_n: u32,   // +0x274
    env1: FreeEnv, // +0x278: 30 ms click
    env2: FreeEnv, // +0x370: main decay
    // Gate (+0x468) and choke (+0x480) envelopes: state 0 off / 1 on / 2 release, level,
    // release coefficient.
    gate_st: u32,
    gate_lv: f32,
    gate_k: f32,
    choke_st: u32,
    choke_lv: f32,
    choke_k: f32,
    // Smoothed values (+0x4b0) and their targets (+0x4f0): frequency, Color frequency,
    // 0.85 × Color frequency, decay (dB), saturation gain, output gain, noise,
    // dissonance; increments (+0x4d0).
    pub cur: [f32; 8],
    pub tgt: [f32; 8],
    incs: [f32; 8],
    start: bool,         // +0x510: start smoothing at the next control tick
    pub smoothing: bool, // +0x511
    ramp_left: i32,      // +0x514
    ramp_len: i32,       // +0x518
    ramp_inv: f32,       // +0x51c
    vel: f32,            // +0x530
    pending_reset: bool, // +0x534
    sr: f32,             // +0x538
    sr_inv: f32,         // +0x53c
    /// Template flag D: recompute coefficients every sample in the next block.
    pub fresh: bool,
}

#[derive(Default, Clone, Copy)]
pub(super) struct Svf {
    pub ic1: f32,
    pub ic2: f32,
    pub f: f32,
    pub g: f32,
    pub k: f32,
    pub h: f32,
    pub sr: f32,
    pub inv: f32,
}

/// `x` clamped to [0, 1] as the code does it (`x < 0 → 0`, then `x > 1 → 1`).
pub(super) fn clamp01(x: f32) -> f32 {
    let x = if x < 0.0 { 0.0 } else { x };
    if x > 1.0 { 1.0 } else { x }
}

/// The tangent approximation behind every prewarped coefficient: `w + w³/3 + 2w⁵/15`.
pub(super) fn tan3(w: f32) -> f32 {
    let w2 = w * w;
    let w3 = w * w2;
    let w5 = w2 * w3;
    w + w5.mul_add(f32::from_bits(0x3e08_8886), w3 * f32::from_bits(0x3eaa_aaaa))
}

pub(super) const PI: f32 = f32::from_bits(0x4049_0fdb);

impl Svf {
    /// A new cutoff (clamped to [0, 0.49·sr]).
    pub fn set(&mut self, f: f32) {
        let c = self.sr * f32::from_bits(0x3efa_e148);
        let f = if f < 0.0 { 0.0 } else { f };
        let f = if f > c { c } else { f };
        self.f = f;
        let g = tan3(self.inv * (f * PI));
        self.g = g;
        self.h = 1.0 / g.mul_add(self.k + g, 1.0);
    }

    /// One sample: returns (high-pass, band-pass).
    pub fn tick(&mut self, x: f32) -> (f32, f32) {
        let (g, k) = (self.g, self.k);
        let g2 = g * g;
        let a = self.ic1 + self.ic1;
        let s6 = self.ic2.mul_add(2.0, g * a);
        let s2 = g.mul_add(k, g2);
        let s4 = a.mul_add(k, s6);
        let s2 = x.mul_add(s2, s4);
        let hp = x - self.h * s2;
        let v1 = g.mul_add(hp, self.ic1);
        let bp = self.ic1 + v1;
        let v2 = g.mul_add(bp, self.ic2);
        self.ic1 = v1;
        self.ic2 = v2;
        (hp, bp)
    }
}

/// The oscillator frequency ratios (Silver).
const RATIOS: [f32; 6] = [
    1.0,
    1.25,
    f32::from_bits(0x3fbe_b852), // 1.49
    f32::from_bits(0x3fd9_999a), // 1.7
    f32::from_bits(0x3fe5_1eb8), // 1.79
    f32::from_bits(0x4023_3333), // 2.55
];

/// `2^x` for the retrigger crossfade's per-sample factor: round to nearest, a cubic for
/// the fraction, the integer part into the exponent; at most 0.99999.
pub(super) fn pow2_cubic(x: f32) -> f32 {
    let r = x.round_ties_even();
    let i = if r < 2147483648.0 && r > -2147483648.0 { r as i32 } else { i32::MIN };
    let f = x - i as f32;
    let p = f.mul_add(f32::from_bits(0x3d65_6042), f32::from_bits(0x3e77_ced9));
    let p = p.mul_add(f, f32::from_bits(0x3f31_6873));
    let p = p.mul_add(f, 1.0);
    let e = f32::from_bits((0x3f80_0000i32.wrapping_add(i.wrapping_shl(23))) as u32);
    let v = p * e;
    let m = f32::from_bits(0x3f7f_ff58);
    if v > m { m } else { v }
}

impl Core606 {
    /// Construction, then `onSetSampleRate(44100, 128)` (the processor's constructor) and
    /// `onSetSampleRate(sr, 128)`.
    pub fn new(sr: f32) -> Core606 {
        let mut c = Core606 {
            p: [0.0; 9],
            tune_st: 0.0,
            gate: false,
            silver_next: false,
            silver: false,
            note: 60,
            osc_ph: [0.0; 6],
            osc_ph2: [0.0; 6],
            osc_inc: [0.0; 6],
            lp_s: 0.0,
            lp_out: 0.0,
            lp_a: 0.0,
            lp_b: 0.0,
            lp_g: 0.0,
            noise_gain: 0.0,
            lcg: 0,
            rnd: [0.0; 6],
            clk: 0.0,
            cnt: [0.0; 6],
            flags: [true; 5],
            fol: 0.0,
            fol_att: 0.0,
            fol_rel: 0.0,
            bp: Svf { k: f32::from_bits(0x3e4c_ccd0), ..Svf::default() },
            hp: Svf { k: f32::from_bits(0x3fcc_cccd), ..Svf::default() },
            xf_t: 1.0,
            xf_flag: true,
            xf_k: 0.0,
            xf_coef: 0.0,
            level: 0.0,
            xf_prev: 0.0,
            xf_mix: 0.0,
            inv: 0.0,
            thr: f32::from_bits(0x38d1_b717),
            tail: false,
            fade_len: 1,
            fade_inv: 1.0,
            fade_n: 0,
            env1: FreeEnv::new(sr, 0.0013, f32::from_bits(0x3f33_3333)),
            env2: FreeEnv::new(sr, 0.0013, 0.5),
            gate_st: 0,
            gate_lv: 0.0,
            gate_k: 0.0,
            choke_st: 0,
            choke_lv: 0.0,
            choke_k: 0.0,
            cur: [0.0; 8],
            tgt: [0.0; 8],
            incs: [0.0; 8],
            start: true,
            smoothing: false,
            ramp_left: 0,
            ramp_len: 1,
            ramp_inv: 0.0,
            vel: 0.0,
            pending_reset: false,
            sr,
            sr_inv: 0.0,
            fresh: true,
        };
        c.set_sample_rate(sr);
        c
    }

    /// `onSetSampleRate(sr, 128)` (the parts that matter after construction).
    fn set_sample_rate(&mut self, sr: f32) {
        let n = ((sr * f32::from_bits(0x3ca3_d70a)) / 128.0 + 0.5) as i32;
        let n = if n > 1 { n } else { 1 };
        self.ramp_len = n;
        self.ramp_inv = 1.0 / (n * 128) as f32;
        let inv = 1.0 / sr;
        let s8 = inv * f32::from_bits(0xbf31_7215); // −ln 2/sr
        let k = (s8 / f32::from_bits(0x3ba3_d70a)).exp(); // 5 ms
        self.choke_k = if k > 1.0 { 1.0 } else { k };
        self.choke_st = 0;
        self.sr = sr;
        self.sr_inv = inv;
        // Oscillator increments, from the frequencies they hold.
        let f0 = self.cur_freq();
        for (inc, r) in self.osc_inc.iter_mut().zip(RATIOS) {
            let x = inv * (f0 * r);
            let x = if x < -1.0 { -1.0 } else { x };
            *inc = if x > 1.0 { 1.0 } else { x };
        }
        // The 8 kHz low-pass.
        let g = tan3(inv * f32::from_bits(0x46c4_597c)); // 8000π
        self.lp_g = g;
        self.lp_a = 1.0 / (g + 1.0);
        self.lp_b = g + -1.0;
        self.lp_s = 0.0;
        self.lp_out = 0.0;
        for s in [&mut self.bp, &mut self.hp] {
            s.sr = sr;
            s.inv = inv;
            s.f = -1.0;
            s.ic1 = 0.0;
            s.ic2 = 0.0;
        }
        // Envelope follower: 1 ms attack, 10 ms release (ln 2 per time constant).
        let ln2 = f32::from_bits(0x3f31_7215);
        let a = f32::from_bits(0x3a83_126f) * sr; // 0.001·sr
        let r = f32::from_bits(0x3c23_d70a) * sr; // 0.01·sr
        self.fol_att = if 1.0 > a { ln2 } else { ln2 / a };
        self.fol_rel = if 1.0 > r { ln2 } else { ln2 / r };
        self.noise_gain = (sr / 44100.0).sqrt() * f32::from_bits(0x3000_009d);
        self.inv = inv;
        let fl = (sr * f32::from_bits(0x3c75_c290)) as u32; // 15 ms
        self.fade_len = if fl > 1 { fl } else { 1 };
        self.fade_inv = 1.0 / self.fade_len as f32;
        self.xf_k = 0.0;
        self.tail = false;
        self.fade_n = 0;
        self.gate_st = 0;
        let k = (s8 / f32::from_bits(0x3a83_126f)).exp(); // 1 ms
        self.gate_k = if k > 1.0 { 1.0 } else { k };
        self.fresh = true;
    }

    fn cur_freq(&self) -> f32 {
        self.cur[0]
    }

    /// `setParameter(i, v)`.
    pub fn set_param(&mut self, i: usize, v: f32) {
        if i > 8 {
            return;
        }
        self.p[i] = v;
        match i {
            0 => {
                let t = clamp01(v).mul_add(48.0, -24.0) + (self.note as f32 + -60.0);
                let t = if t < -24.0 { -24.0 } else { t };
                let t = if t > 24.0 { 24.0 } else { t };
                self.tune_st = t;
                self.tgt[0] = exp_e((t + 70.0).mul_add(5.0, f32::from_bits(0x44ac_bc36)));
                self.start_smooth();
            }
            1 => {
                self.tgt[3] = clamp01(v).mul_add(40.0, 25.0);
                self.start_smooth();
            }
            2 => {
                self.set_color(v);
                self.start_smooth();
            }
            3 => {
                let db = clamp01(v).mul_add(24.0, -6.0);
                let a = clamp01((db + -2.0) / 13.0);
                let o = a.mul_add(10.0, 0.0) - db;
                const K: f32 = f32::from_bits(0x411f_73da); // 9.965784
                self.tgt[4] = exp_e(db.mul_add(K, 1200.0));
                self.tgt[5] = exp_e(o.mul_add(K, 1200.0));
                self.start_smooth();
            }
            4 => self.silver_next = v > 0.0,
            5 => {
                self.tgt[6] = v;
                self.start_smooth();
            }
            6 => {
                let mut r = ((v * 30.0) as i32).wrapping_mul(0x07d3_adeb).wrapping_add(0x12aa_0131);
                for x in &mut self.rnd {
                    let s = (r as f32).mul_add(f32::from_bits(0x2f80_000d), 0.5);
                    *x = s.mul_add(f32::from_bits(0x3e99_999a), f32::from_bits(0x3dcc_cccd));
                    r = r.wrapping_mul(0x07d3_adeb).wrapping_add(0x12aa_0131);
                }
            }
            7 => {
                self.tgt[7] = v;
                self.start_smooth();
            }
            _ => {
                self.gate = v > 0.0;
                self.fresh = true;
            }
        }
    }

    fn set_color(&mut self, v: f32) {
        let c = clamp01(v).mul_add(50.0, 82.0) + (self.note as i32 - 60) as f32;
        let c = if c < 82.0 { 82.0 } else { c };
        let c = if c > 132.0 { 132.0 } else { c };
        let f = exp_e(c.mul_add(5.0, f32::from_bits(0x44ac_bc36)));
        self.tgt[1] = f;
        self.tgt[2] = f * f32::from_bits(0x3f59_999a);
    }

    fn start_smooth(&mut self) {
        self.start = true;
    }

    /// `setNoteOn(v, note)`.
    pub fn note_on(&mut self, v: f32, note: u32) {
        if !(0.0..=1.0).contains(&v) {
            self.vel = 0.0;
            return;
        }
        self.note = note;
        let t = clamp01(self.p[0]).mul_add(48.0, -24.0);
        let t = (note as f32 + -60.0) + t;
        let t = if t < -24.0 { -24.0 } else { t };
        let t = if t > 24.0 { 24.0 } else { t };
        self.tune_st = t;
        self.tgt[0] = exp_e((t + 70.0).mul_add(5.0, f32::from_bits(0x44ac_bc36)));
        self.set_color(self.p[2]);
        self.silver = self.silver_next;
        self.cur = self.tgt;
        self.start = true;
        self.vel = if v > 1.0 { 1.0 } else { v };
        self.choke_st = 1;
        self.choke_lv = 0.0;
        self.env1.note_on(self.vel);
        self.env2.note_on(self.vel);
        self.xf_k = 1.0;
        self.xf_prev = self.level;
        if !self.xf_flag {
            self.xf_coef = self.xf_factor();
        }
        self.tail = true;
        self.fade_n = 0;
        if self.gate {
            self.gate_st = 1;
            self.gate_lv = 0.0;
        }
        self.pending_reset = true;
        self.fresh = true;
    }

    fn xf_factor(&self) -> f32 {
        let s1 = self.inv * -3.0;
        let lim = self.inv * f32::from_bits(0x4a00_2c80); // 2.1e6
        let t = self.xf_t;
        let t = if t < f32::from_bits(0x38d1_b717) { f32::from_bits(0x38d1_b717) } else { t };
        let t = if lim < t { lim } else { t };
        pow2_cubic(s1 / t)
    }

    /// `setNoteOff()`: release the gate.
    pub fn note_off(&mut self) {
        if self.gate_st == 1 {
            self.gate_st = 2;
            self.gate_lv = 1.0;
        }
    }

    /// `onChoke()`: release the choke envelope (5 ms).
    pub fn choke(&mut self) {
        if self.choke_st == 1 {
            self.choke_st = 2;
            self.choke_lv = 1.0;
        }
    }

    /// `onReset()`.
    pub fn reset(&mut self) {
        self.vel = 0.0;
        self.reset_voice();
    }

    fn reset_voice(&mut self) {
        self.vel = 0.0;
        self.fol = 0.0;
        self.bp.ic1 = 0.0;
        self.bp.ic2 = 0.0;
        self.hp.ic1 = 0.0;
        self.hp.ic2 = 0.0;
        self.lp_s = 0.0;
        self.lp_out = 0.0;
        self.env1.reset();
        self.env2.reset();
        self.gate_st = 0;
        self.gate_lv = 0.0;
        self.choke_st = 0;
        self.choke_lv = 0.0;
        self.level = 0.0;
        self.xf_prev = 0.0;
        self.xf_k = 0.0;
        self.tail = false;
        self.fade_n = 0;
        self.pending_reset = false;
        self.cur = self.tgt;
        self.start = true;
    }

    /// `processControlRate()`: the 20 ms parameter ramps.
    pub fn control(&mut self) {
        if self.start {
            self.start = false;
            let mut any = false;
            for i in 0..8 {
                let d = (self.tgt[i] - self.cur[i]) * self.ramp_inv;
                self.incs[i] = d;
                if d * d < 1e-15 {
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
            self.incs = [0.0; 8];
            self.smoothing = false;
        }
    }

    /// Whether `processAudio` runs a voice (`hasOutputSignal`'s test).
    pub fn active(&self) -> bool {
        let sounding = self.env1.active() || self.env2.active() || self.thr < self.level || self.tail;
        sounding && (if self.gate { self.gate_st != 0 && self.choke_st != 0 } else { self.choke_st != 0 })
    }

    /// `processAudio`: one block (≤ 16 samples from the wrapper).
    pub fn process(&mut self, out: &mut [f32]) -> bool {
        if !self.active() {
            if self.pending_reset {
                self.reset_voice();
            }
            return false;
        }
        // Fresh blocks recompute every coefficient; smoothing blocks only those whose
        // value ramps (`processAudioImpl<…, C = true, D = false>`).
        let (fresh, sm, incs) = (self.fresh, self.smoothing, self.incs);
        let rc = [0, 1, 2, 3].map(|i| fresh || (sm && incs[i] != 0.0));
        for o in out.iter_mut() {
            if self.smoothing {
                for i in 0..8 {
                    self.cur[i] += self.incs[i];
                }
            }
            *o = self.sample(rc);
        }
        self.fresh = false;
        true
    }

    fn sample(&mut self, rc: [bool; 4]) -> f32 {
        let src = if self.silver { self.metal(rc[0]) } else { self.circuit() };
        if rc[1] && self.bp.f != self.cur[1] {
            self.bp.set(self.cur[1]);
        }
        let (_, bp) = self.bp.tick(src);
        // Envelope follower and normalisation.
        let d = bp.abs() - self.fol;
        let c = if d > 0.0 { self.fol_att } else { self.fol_rel };
        self.fol = d.mul_add(c, self.fol);
        let e = self.fol;
        let e = if e < f32::from_bits(0x3c23_d70a) { f32::from_bits(0x3c23_d70a) } else { e };
        let q = bp / e;
        let q = if q < -1.0 { -1.0 } else { q };
        let q = if q > 1.0 { 1.0 } else { q };
        let q = if q < f32::from_bits(0xbdcc_cccd) { f32::from_bits(0xbdcc_cccd) } else { q };
        if rc[3] {
            self.update_decay();
        }
        let e1 = self.env1.tick();
        let e2 = self.env2.tick();
        let e2 = e2 + f32::from_bits(0xbe19_999a); // −0.15
        let e2 = if e2 < 0.0 { 0.0 } else { e2 };
        let new = self.vel * (e1 + e2);
        let old = self.xf_k * self.xf_prev;
        self.xf_k *= self.xf_coef;
        let mx = if old < new { new } else { old };
        let sum = old + new;
        let m = clamp01(self.xf_mix);
        let lv = (sum - mx).mul_add(m, mx);
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
        let x = q * (lv * fade);
        if rc[2] && self.hp.f != self.cur[2] {
            self.hp.set(self.cur[2]);
        }
        let (hp, _) = self.hp.tick(x);
        let y = hp * self.cur[4];
        let s = f32::from_bits(0xbefa_e147) / (y.abs() + f32::from_bits(0x3f33_3333)) + f32::from_bits(0x3f33_3333);
        let s = if y > 0.0 { s } else { -s };
        let g = if self.gate {
            match self.gate_st {
                0 => 0.0,
                1 => 1.0,
                _ => {
                    let l = self.gate_k * self.gate_lv;
                    self.gate_lv = l;
                    if (l as f64) < f64::from_bits(0x3f1a_36e2_eb1c_432d) {
                        self.gate_lv = 0.0;
                        self.gate_st = 0;
                        0.0
                    } else {
                        l
                    }
                }
            }
        } else {
            1.0
        };
        let ch = match self.choke_st {
            0 => 0.0,
            1 => 1.0,
            _ => {
                let l = self.choke_k * self.choke_lv;
                self.choke_lv = l;
                if (l as f64) < f64::from_bits(0x3f1a_36e2_eb1c_432d) {
                    self.choke_lv = 0.0;
                    self.choke_st = 0;
                    0.0
                } else {
                    l
                }
            }
        };
        let k = if self.silver { f32::from_bits(0x3fa5_676f) } else { f32::from_bits(0x3f9c_26a2) };
        let o = self.cur[5] * s;
        let o = if self.gate { o * g } else { o };
        o * k * ch
    }

    /// The Silver source: six pulse oscillators, the 8 kHz low-pass, and the noise blend.
    fn metal(&mut self, recompute: bool) -> f32 {
        if recompute {
            let f = self.cur[0];
            for (i, (inc, r)) in self.osc_inc.iter_mut().zip(RATIOS).enumerate() {
                let fi = if i == 0 { f } else { f * r };
                let x = fi * self.sr_inv;
                let x = if x < -1.0 { -1.0 } else { x };
                *inc = if x > 1.0 { 1.0 } else { x };
            }
        }
        let noise = self.cur[6];
        let mut osc = 0.0;
        if noise < f32::from_bits(0x3f7f_be77) {
            // 0.999
            let mut d = [0.0f32; 6];
            for (i, di) in d.iter_mut().enumerate() {
                let s = self.osc_ph[i] + self.osc_inc[i];
                let a = s + 10.0;
                let a = a - ((a + 0.5) as i32) as f32;
                let b = (s + 1.525) + 10.0;
                let b = b - ((b + 0.5) as i32) as f32;
                self.osc_ph[i] = a;
                self.osc_ph2[i] = b;
                *di = a - b;
            }
            let sum = ((((d[0] + d[1]) + d[2]) + d[3]) + d[4]) + d[5];
            let y = self.lp_a * (-self.lp_b).mul_add(self.lp_s, sum);
            let out = self.lp_g * (self.lp_s + y);
            self.lp_out = out;
            self.lp_s = y;
            osc = out;
        }
        let mut n = 0.0;
        if noise > 0.0 {
            self.lcg = self.lcg.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
            n = self.noise_gain * self.lcg as f32;
        }
        let x = clamp01(noise);
        let a = 1.0 - x;
        let a = a * (2.0 - a);
        let b = (x * (2.0 - x)) * n;
        a.mul_add(osc, b)
    }

    /// The Circuit source: the clocked counter chain.
    fn circuit(&mut self) -> f32 {
        let inc = (self.cur[0] * 30.0) * self.sr_inv;
        let old = self.cnt;
        let mut acc = inc + self.clk;
        self.clk = acc;
        if acc > 1.0 {
            let a = clamp01(self.cur[7]).mul_add(f32::from_bits(0x3f21_47af), f32::from_bits(0x3f11_eb85));
            let mut c = self.cnt;
            let mut f0 = self.flags[0];
            let wrap = |x: f32| -> f32 {
                let h = if x < 0.0 { -0.5 } else { 0.5 };
                x - ((h + x as f64) as i32) as f32
            };
            loop {
                c[0] = a.mul_add(c[0], self.rnd[0]);
                if f0 {
                    let mut lvl = 1;
                    loop {
                        c[lvl] = wrap(a.mul_add(c[lvl], self.rnd[lvl]));
                        self.cnt[lvl] = c[lvl];
                        if lvl == 5 || !self.flags[lvl] {
                            break;
                        }
                        lvl += 1;
                    }
                    // Toggle the flags from the innermost reached level outwards.
                    for j in (1..=lvl.min(4)).rev() {
                        self.flags[j] = !self.flags[j];
                    }
                }
                c[0] = wrap(c[0]);
                acc += -1.0;
                f0 = !f0;
                if acc <= 1.0 {
                    break;
                }
            }
            self.clk = acc;
            self.cnt[0] = c[0];
            self.flags[0] = f0;
        }
        let s = ((((old[0] + old[1]) + old[2]) + old[3]) + old[4]) + old[5];
        (s * f32::from_bits(0x3ecc_cccd)) * 3.0
    }

    /// The per-sample decay update while coefficients are fresh: the decay time
    /// `1 ms·10^(dB/20)` with velocity adding ±1.5 dB.
    fn update_decay(&mut self) {
        let v = (self.vel + -0.5) + (self.vel + -0.5);
        let db = v.mul_add(1.5, self.cur[3]);
        let t = exp_e(db.mul_add(f32::from_bits(0x411f_73da), f32::from_bits(0x4416_8363)));
        let t1 = t.min(f32::from_bits(0x3cf5_c28f)); // fminnm with 0.03
        self.env1.set_decay(t1);
        self.env2.set_decay(t);
        let tx = t * f32::from_bits(0x3ecc_cccd);
        if self.xf_t != tx {
            self.xf_t = tx;
            if self.xf_flag {
                self.xf_coef = self.xf_factor();
            }
        }
    }
}
