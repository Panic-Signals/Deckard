//! `NI::DSP::DiscoSDCore`: the Snare's Sharp engine (docs/synth-snare.md).
//!
//! A modal drum: an attack layer (`AttackCore`: a "fluffy" one-shot, a phase-coupled
//! sine oscillator and filtered noise, [`attack`]), a modal body (`BodyTableCore`: a
//! bank of resonators excited by a click and pink "air" noise, [`body`]) and attached
//! bodies (`AttachedBodiesCore`: the snare wires, resonators fed by sampled modal noise,
//! [`attached`]), mixed by three gains. `DiscoSDRealTimeMod` maps the user parameters
//! and the velocity onto the sub-cores' modulation inputs.
//!
//! Member offsets in comments are `DiscoSDCore` offsets (the core sits at
//! `SnareProcessor` +0x18a600).

mod attached;
mod attack;
mod body;
mod kit;

use super::Core;
use crate::fx::exp_e;

/// The core's level lookup: `ExpTable` at `x·9.965784 + 1200` (fused): 2^(x/6.02) ≈
/// `x` dB as an amplitude.
#[inline]
pub(super) fn db_gain(x: f32) -> f32 {
    exp_e(x.mul_add(9.965_784, 1200.0))
}

/// NI's velocity/parameter response curve (`lo`, `hi`, `k`, `m`), as inlined in
/// `FluffyOsc::setNoteOn`, `DiscoSDCore::setNoteOn`, `computePerformance`, …: `v` is
/// clamped to `[lo, hi]` and normalised, bent by `x·(k + 1 − k·|x|)`, then crossfaded
/// from `1 − x` (m = 0) over 1 (m = 0.5) to `x` (m = 1); clamped to `[0, 1]`.
#[inline]
pub(super) fn vcurve(v: f32, lo: f32, hi: f32, k: f32, m: f32) -> f32 {
    let hi = if hi > lo { hi } else { lo + 0.002 };
    let x = if lo > v { lo } else { v };
    let x = if x > hi { hi } else { x };
    let d = hi - lo;
    let d = if d < 1e-7 { 1e-7 } else { d };
    let y = (x - lo) / d;
    let y = y * (-y.abs()).mul_add(k, k + 1.0);
    let a = 1.0 - y;
    let m2 = if m + m > 1.0 { 1.0 } else { m + m };
    let a = (1.0 - a).mul_add(m2, a);
    let s = m.mul_add(2.0, -m2);
    clamp01((y - a).mul_add(s, a))
}

/// `fcmp x, 0; fcsel 0 if mi; fcmp 1; fcsel 1 if gt`.
#[inline]
pub(super) fn clamp01(x: f32) -> f32 {
    let x = if x < 0.0 { 0.0 } else { x };
    if x > 1.0 { 1.0 } else { x }
}

/// The re-rounding NI's shaping helpers do after `x·(…)`: `a = 1 − x; a += 1 − a;
/// a + (x − a)`, exactly as compiled.
#[inline]
pub(super) fn kahan(x: f32) -> f32 {
    let a = 1.0 - x;
    let a = a + (1.0 - a);
    a + (x - a)
}

/// `DiscoSDRealTimeMod` modulation tables (`sModulationValues`: lo, hi per target);
/// output `t` = Σ over the parameters of `lo + x·(hi − lo)`.
const SKIN_T4: (f32, f32) = (-0.5, 0.5);
const TUNE_T3: (f32, f32) = (-6.0, 6.0);
/// Decay, noise and "tune 2" rows for targets 1, 2, 6, 8.
const DECAY_T: [(f32, f32); 4] = [(-4.0, 2.0), (-7.0, 3.0), (-3.0, 2.0), (-5.0, 2.0)];
const NOISE_T: [(f32, f32); 4] = [(-5.0, 2.0), (3.5, -4.5), (-8.0, 0.0), (-4.0, 4.0)];
const TUNE2_T: [(f32, f32); 4] = [(0.0, -1.0), (0.0, -2.0), (0.0, -1.0), (0.0, -2.0)];
/// `sVelocityValues` for targets 0 and 7: (curve, lo, hi).
const VEL_T0: (usize, f32, f32) = (1, -48.0, 0.0);
const VEL_T7: (usize, f32, f32) = (0, -8.0, 0.0);

/// `fmadd(hi − lo, x, lo)`.
#[inline]
fn lerp(t: (f32, f32), x: f32) -> f32 {
    (t.1 - t.0).mul_add(x, t.0)
}

/// `NI::DSP::UTIL::DiscoSDRealTimeMod` (+0x42668).
struct RealTimeMod {
    /// +0x42700: vel, skin, tune, decay, impact, body, noise (as last applied).
    p: [f32; 7],
    /// +0x42718: the nine modulation outputs.
    out: [f32; 9],
    /// +0x42670 skin, +0x42674 tune, +0x42678 decay (shaped), +0x42680 noise,
    /// +0x42688 tune 2, +0x42690 performance, +0x4269c phase-osc ratio.
    skin: f32,
    tune: f32,
    decay: f32,
    noise: f32,
    tune2: f32,
    perf: f32,
    ratio: f32,
    /// +0x426ac..: the five velocity curves (lo, hi, k, m).
    curve_lo: [f32; 5],
    curve_hi: [f32; 5],
    curve_k: [f32; 5],
    curve_m: [f32; 5],
}

impl RealTimeMod {
    fn new() -> RealTimeMod {
        RealTimeMod {
            p: [0.0; 7],
            out: [0.0, -9.0, -7.0, 0.0, 0.0, 0.0, -11.0, 0.0, -9.0],
            skin: 0.0,
            tune: 0.0,
            decay: 0.0,
            noise: 0.0,
            tune2: 0.0,
            perf: 0.0,
            ratio: 0.0,
            curve_lo: [0.0, 0.0, 0.31, 0.19, 0.0],
            curve_hi: [0.95, 1.0, 1.0, 1.0, 1.0],
            curve_k: [-0.4, -1.0, -1.0, 1.0, -1.0],
            curve_m: [1.0, 1.0, 1.0, 1.0, 1.0],
        }
    }

    /// Targets 1, 2, 6, 8 from decay, noise and tune 2.
    fn mix(&mut self, with_t2: bool) {
        let (d, n, t) = (self.decay, self.noise, self.tune2);
        self.out[1] = (lerp(DECAY_T[0], d) + lerp(NOISE_T[0], n)) + lerp(TUNE2_T[0], t);
        if with_t2 {
            self.out[2] = lerp(DECAY_T[1], d) + lerp(TUNE2_T[1], t);
        }
        self.out[6] = (lerp(DECAY_T[2], d) + lerp(NOISE_T[2], n)) + lerp(TUNE2_T[2], t);
        self.out[8] = (lerp(DECAY_T[3], d) + lerp(NOISE_T[3], n)) + lerp(TUNE2_T[3], t);
    }

    /// `setDecay`.
    fn set_decay(&mut self, v: f32) {
        let v = clamp01(v);
        self.p[3] = v;
        let x = if v < 0.03 { 0.03 } else { v };
        let x = if x > 0.85 { 1.0 } else { (x - 0.03) / f32::from_bits(0x3f51_eb86) };
        let x = x * x.abs().mul_add(0.2, 0.8);
        self.decay = clamp01(kahan(x));
        self.mix(true);
    }

    /// `setParameter(5)` (noise).
    fn set_noise(&mut self, v: f32) {
        let v = clamp01(v);
        self.p[6] = v;
        self.noise = v;
        self.mix(false);
    }

    /// `setTune`.
    fn set_tune(&mut self, v: f32) {
        let v = clamp01(v);
        self.p[2] = v;
        self.tune = v;
        let b = if v < 0.62 { 0.0 } else { (v - 0.62) / 0.38 };
        let a = v * v.abs().mul_add(-0.763_78, 1.763_78);
        let b = b * b.abs().mul_add(1.0, 0.0);
        let (a, b) = (kahan(a), kahan(b));
        // `bic` with the `< 0` mask, then `> 1 → 1`.
        let a = if a < 0.0 { 0.0 } else { a };
        let b = if b < 0.0 { 0.0 } else { b };
        let a = if a > 1.0 { 1.0 } else { a };
        let b = if b > 1.0 { 1.0 } else { b };
        self.tune2 = b;
        self.ratio = a.mul_add(f32::from_bits(0x3fd9_da84), -1.0);
        self.mix(true);
        self.out[3] = lerp(TUNE_T3, self.tune);
    }

    /// `computePerformance`: targets 0 and 7 and the "performance" amount.
    fn compute_performance(&mut self) {
        let v = self.p[0];
        let x = if v < 0.535_433 { 0.535_433 } else { v };
        let y = if x > 1.0 { 1.0 } else { (x - 0.535_433) / 0.464_567 };
        let y = y * y.abs().mul_add(-0.385_827, 1.385_827_1);
        let r = kahan(y);
        let r = if r < 0.0 { 0.0 } else { r };
        self.perf = if r > 1.0 { 0.0 } else { 1.0 - r };
        for (t, (c, lo, hi)) in [(0, VEL_T0), (7, VEL_T7)] {
            let x = vcurve(v, self.curve_lo[c], self.curve_hi[c], self.curve_k[c], self.curve_m[c]);
            self.out[t] = (hi - lo).mul_add(x, lo);
        }
    }
}

/// `DiscoSDCore`.
pub(crate) struct Disco {
    /// +0x80: the seven parameters as set.
    param: [f32; 7],
    att: attack::Attack,
    body: Box<body::Body>,
    bodies: attached::Attached,
    rt: RealTimeMod,
    /// +0x42810: 0 idle, 1 playing, 2 fading out (choke / engine change).
    fade_state: u32,
    /// +0x42818 fade time (s), +0x4281c fade gain, +0x42820 its per-sample factor.
    fade_time: f32,
    fade_gain: f32,
    fade_mult: f32,
    /// +0x42828: the end-of-note declick runs; +0x4282c its length, +0x42830
    /// 1/length, +0x42834 its counter.
    declick: bool,
    declick_len: u32,
    declick_inv: f32,
    declick_n: u32,
    /// +0x42840: a note played since the last reset (reset once it has died away).
    played: bool,
    /// +0x42844 impact gain, +0x42848 its offset, +0x4284c impact (−9..0).
    impact_gain: f32,
    impact_off: f32,
    impact_db: f32,
    /// +0x42850 body gain, +0x42854 its offset.
    body_gain: f32,
    body_off: f32,
    /// +0x42860 attached-bodies gain, +0x42864 output gain.
    noise_gain: f32,
    out_gain: f32,
    /// +0x4286c/+0x42870: the MIDI notes that map onto tune 0 and 1.
    key_lo: u32,
    key_hi: u32,
    /// +0x42874: the shaped velocity of the last note.
    vel: f32,
    /// +0x42890..: the velocity curve (lo, hi, k, m; m from the Velocity parameter).
    vel_curve: [f32; 4],
    /// The parameter smoother (+0x428f0 values, +0x42910 increments, +0x42930
    /// targets; lanes: skin, tune, impact, body, output gain, -, -, -).
    sm_val: [f32; 8],
    sm_inc: [f32; 8],
    sm_tgt: [f32; 8],
    /// +0x42950 pending, +0x42951 ramping, +0x42954 counter, +0x42958 steps,
    /// +0x4295c 1/(steps·block), +0x42960 state.
    sm_pending: bool,
    sm_active: bool,
    sm_count: i32,
    sm_steps: i32,
    sm_inv: f32,
    sm_state: u32,
}

impl Disco {
    pub(crate) fn new(sr: f32) -> Disco {
        let out_gain = exp_e(1200.0);
        let mut d = Disco {
            param: [0.0; 7],
            att: attack::Attack::new(),
            body: body::Body::new(),
            bodies: attached::Attached::new(),
            rt: RealTimeMod::new(),
            fade_state: 0,
            fade_time: 0.005,
            fade_gain: 0.0,
            fade_mult: 0.0,
            declick: false,
            declick_len: 1,
            declick_inv: 1.0,
            declick_n: 0,
            played: false,
            impact_gain: db_gain(-2.5),
            impact_off: -2.5,
            impact_db: 0.0,
            body_gain: 0.0,
            body_off: -2.0,
            noise_gain: db_gain(-10.0),
            out_gain,
            key_lo: 0,
            key_hi: 0,
            vel: 0.0,
            vel_curve: [0.0, 0.95, -0.4, 1.0],
            sm_val: [0.0; 8],
            sm_inc: [0.0; 8],
            sm_tgt: [0.0, 0.0, 0.0, 0.0, out_gain, 0.0, 0.0, 0.0],
            sm_pending: true,
            sm_active: false,
            sm_count: 0,
            sm_steps: 1,
            sm_inv: 0.0,
            sm_state: 1,
        };
        // The processor: `onSetSampleRate(sr, 128)`, then `onReset`.
        d.set_sample_rate(sr, 128);
        d.reset();
        d
    }

    /// `onSetSampleRate(sr, block)`.
    fn set_sample_rate(&mut self, sr: f32, block: i32) {
        let n = ((sr * 0.02) / block as f32 + 0.5) as i32;
        let n = if n > 1 { n } else { 1 };
        self.sm_steps = n;
        self.sm_inv = 1.0 / (n.wrapping_mul(block)) as f32;
        self.att.set_sample_rate(sr, block, n, self.sm_inv);
        self.body.set_sample_rate(sr, block);
        self.bodies.set_sample_rate(sr, block);
        let inv = 1.0 / sr;
        self.fade_state = 0;
        self.fade_gain = 0.0;
        let m = ((inv * -f32::from_bits(0x3f31_7215)) / (self.fade_time + f32::from_bits(0x0da2_4260))).exp();
        self.fade_mult = if m > 1.0 { 1.0 } else { m };
        for i in 0..7 {
            self.set_param(i, self.param[i as usize]);
        }
        let n = (sr * 0.015) as u32;
        self.declick_len = if n > 1 { n } else { 1 };
        self.declick_inv = 1.0 / self.declick_len as f32;
    }

    /// The body level as `onReset`/`processAudio` apply it.
    fn apply_body(&mut self, v: f32) {
        let s = clamp01(v);
        self.rt.p[5] = s;
        let s = s * (s.abs() + 0.0);
        let s = clamp01(kahan(s));
        self.body_gain = s * db_gain(self.body_off);
    }

    /// The skin tune as `setNoteOn`/`processAudio` apply it.
    fn apply_skin(&mut self, v: f32) {
        let s = clamp01(v);
        self.rt.p[1] = s;
        self.rt.skin = s;
        let t = lerp(SKIN_T4, s);
        self.rt.out[4] = t;
        if (-1.0..=1.0).contains(&t) {
            self.body.set_tuning_amount(t);
        }
    }

    /// The impact as `setNoteOn`/`processAudio` apply it.
    fn apply_impact(&mut self, v: f32) {
        let s = clamp01(v);
        self.rt.p[4] = s;
        self.impact_db = s.mul_add(9.0, -9.0);
        self.impact_gain = db_gain(self.impact_db + self.impact_off);
    }

    /// `onReset`.
    fn reset(&mut self) {
        self.played = false;
        self.declick = false;
        self.declick_n = 0;
        self.sm_val = self.sm_tgt;
        self.sm_pending = true;
        self.sm_state = 1;
        self.apply_body(self.sm_tgt[3]);
        self.att.reset();
        self.body.reset();
        self.bodies.reset();
        self.fade_state = 0;
        self.fade_gain = 0.0;
    }

    /// `setParameter(idx, v)`.
    fn set_param(&mut self, idx: u32, v: f32) {
        if idx >= 7 {
            return;
        }
        self.param[idx as usize] = v;
        match idx {
            0 => self.sm_target(0, v),
            1 => {
                self.sm_target(1, v);
                let round = |x: f32| {
                    let h = if x >= 0.0 { 0.5 } else { -0.5 };
                    (x as f64 + h) as i64 as u32
                };
                self.key_lo = round(v.mul_add(-12.0, 60.0));
                self.key_hi = round((1.0 - v).mul_add(12.0, 60.0));
            }
            2 => self.rt.set_decay(v),
            3 => self.sm_target(2, v),
            4 => self.sm_target(3, v),
            5 => self.rt.set_noise(v),
            _ => {
                let m = v.mul_add(0.5, 0.5).mul_add(0.9, 0.1);
                self.vel_curve[3] = m;
                self.rt.curve_m[0] = m;
            }
        }
    }

    fn sm_target(&mut self, lane: usize, v: f32) {
        self.sm_tgt[lane] = v;
        self.sm_pending = true;
        self.sm_state = 1;
    }

    /// `computeTuneModulations(tune, at_note_on)`.
    fn tune_mods(&mut self, t: f32, note_on: bool) {
        self.rt.set_tune(t);
        self.bodies.set_crispel_decay2(self.rt.out[1]);
        let semi = self.rt.out[3];
        if (-48.0..=48.0).contains(&semi) {
            self.body.set_tune(semi);
        }
        self.bodies.set_noise_decay2(self.rt.out[6]);
        self.att.set_pnoise_env_dec2(self.rt.out[8]);
        self.att.set_freq_ph_osc_ratio(self.rt.ratio);
        if !note_on {
            self.body.set_dec_master(self.rt.out[2]);
        }
        self.body.mark_dirty();
    }

    /// `computeVelocityModulations`.
    fn vel_mods(&mut self) {
        self.rt.compute_performance();
        self.att.set_noise_lp(self.rt.out[0]);
        let o7 = self.rt.out[7];
        if (-100.0..=100.0).contains(&o7) {
            self.bodies.set_out7(o7);
        }
        self.body.set_partial_mask_decay(self.rt.perf, self.rt.out[2]);
        self.body.mark_dirty();
    }

    /// `setNoteOn(velocity, note)`.
    fn start(&mut self, vel: f32, note: u32) {
        if !(0.0..=1.0).contains(&vel) {
            return;
        }
        let t = if note >= self.key_lo && self.key_hi >= note {
            (note - self.key_lo) as f32 / 12.0
        } else if self.key_hi < note {
            1.0
        } else {
            0.0
        };
        self.sm_val[1] = self.sm_tgt[1];
        self.tune_mods(t, true);
        self.sm_val[0] = self.sm_tgt[0];
        self.apply_skin(self.sm_val[0]);
        self.sm_inc[2] = 0.0;
        self.sm_val[2] = self.sm_tgt[2];
        self.apply_impact(self.sm_val[2]);
        let [lo, hi, k, m] = self.vel_curve;
        self.vel = vcurve(vel, lo, hi, k, m);
        self.rt.p[0] = self.vel;
        self.vel_mods();
        let delay = self.att.delay_ms();
        if delay >= 0.0 {
            self.body.set_delay_from_attack(delay);
        }
        self.bodies.set_delay_from_attack(delay);
        self.body.set_attack_pitch();
        let dec = self.att.env_decay_ms();
        if dec >= 0.0 {
            self.body.set_attack_env_decay(dec);
        }
        self.att.note_on(self.vel);
        self.body.note_on(self.vel);
        self.bodies.note_on(self.vel);
        self.fade_state = 1;
        self.fade_gain = 0.0;
        self.played = true;
        self.declick = true;
        self.declick_n = 0;
    }

    fn sources_active(&self) -> bool {
        self.att.active() || self.body.active() || self.bodies.active()
    }

    /// The smoother's control-rate step (first part of `processControlRate`).
    fn smoother_step(&mut self) {
        if self.sm_pending {
            self.sm_pending = false;
            self.sm_state = 2;
            let mut any = false;
            for i in 0..8 {
                let inc = (self.sm_tgt[i] - self.sm_val[i]) * self.sm_inv;
                if inc * inc < 1e-15 {
                    self.sm_inc[i] = 0.0;
                    self.sm_val[i] = self.sm_tgt[i];
                } else {
                    self.sm_inc[i] = inc;
                    any = true;
                }
            }
            if any {
                self.sm_count = self.sm_steps;
                self.sm_active = true;
            } else {
                self.sm_state = 4;
            }
        } else {
            self.sm_count = self.sm_count.wrapping_sub(1);
            if self.sm_count == 0 {
                self.sm_val = self.sm_tgt;
                self.sm_inc = [0.0; 8];
                self.sm_active = false;
                self.sm_state = 4;
            } else if self.sm_state == 2 {
                self.sm_state = 3;
            } else if self.sm_state == 4 {
                self.sm_state = 0;
            }
        }
    }
}

impl Core for Disco {
    fn set_parameter(&mut self, idx: u32, v: f32) {
        self.set_param(idx, v);
    }

    fn note_on(&mut self, velocity: f32, note: u32) {
        self.start(velocity, note);
    }

    fn note_off(&mut self, _note: u32) {}

    fn choke(&mut self) {
        self.leave();
    }

    fn leave(&mut self) {
        if self.fade_state == 1 {
            self.fade_state = 2;
            self.fade_gain = 1.0;
        }
    }

    fn control_rate(&mut self) {
        self.smoother_step();
        self.att.control_rate();
        self.body.control_rate();
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        l.fill(0.0);
        r.fill(0.0);
        if !((self.sources_active() || self.declick) && self.fade_state != 0) {
            if self.sm_inc[3] != 0.0 {
                self.sm_inc[3] = 0.0;
                self.sm_val[3] = self.sm_tgt[3];
                self.apply_body(self.sm_val[3]);
            }
            if self.played {
                self.reset();
            }
            return;
        }
        let ramp = self.sm_active;
        // `processAudioImpl`: the declick gain lives in a register that starts at 1
        // in every call.
        let mut dc = 1.0f32;
        let mix_gain = f32::from_bits(0x3fa0_37d6); // 1.2517039
        for (ol, or) in l.iter_mut().zip(r.iter_mut()) {
            let mut gain = self.out_gain;
            if ramp {
                for i in 0..8 {
                    self.sm_val[i] += self.sm_inc[i];
                }
                if self.sm_inc[0] != 0.0 {
                    self.apply_skin(self.sm_val[0]);
                }
                if self.sm_inc[1] != 0.0 {
                    self.tune_mods(self.sm_val[1], false);
                }
                if self.sm_inc[2] != 0.0 {
                    self.apply_impact(self.sm_val[2]);
                }
                if self.sm_inc[3] != 0.0 {
                    self.apply_body(self.sm_val[3]);
                }
                if self.sm_inc[4] != 0.0 {
                    gain = self.sm_val[4];
                }
            }
            let a = self.att.process();
            let (bl, br) = self.body.process();
            let n = self.bodies.process();
            let (fade, mut done) = match self.fade_state {
                0 => (0.0, true),
                1 => (1.0, false),
                _ => {
                    let g = self.fade_mult * self.fade_gain;
                    self.fade_gain = g;
                    if (g as f64) < 1e-4 {
                        self.fade_gain = 0.0;
                        self.fade_state = 0;
                        (0.0, true)
                    } else {
                        (g, false)
                    }
                }
            };
            if !self.sources_active() {
                done = true;
            }
            if done {
                let k = if self.declick_n < self.declick_len {
                    self.declick_n + 1
                } else {
                    self.declick = false;
                    self.declick_len
                };
                self.declick_n = k;
                let x = (-(k as f32)).mul_add(self.declick_inv, 1.0);
                dc = x * (x * x);
            }
            let sl = n.mul_add(self.noise_gain, self.impact_gain.mul_add(a, self.body_gain * bl));
            let sr = n.mul_add(self.noise_gain, self.impact_gain.mul_add(a, self.body_gain * br));
            *ol = fade * (gain * (sl * mix_gain)) * dc;
            *or = fade * (gain * (sr * mix_gain)) * dc;
        }
    }

    fn has_output(&self) -> bool {
        (self.sources_active() || self.declick) && self.fade_state != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs an oracle script (see the port notes) on DECKARD's core:
    /// `DISCO_SCRIPT=file DISCO_OUT=raw DISCO_SR=48000 cargo test … oracle_script -- --ignored`.
    #[test]
    #[ignore]
    fn oracle_script() {
        let script = std::fs::read_to_string(std::env::var("DISCO_SCRIPT").unwrap()).unwrap();
        let sr: f32 = std::env::var("DISCO_SR").map(|s| s.parse().unwrap()).unwrap_or(48_000.0);
        let mut d = Disco::new(sr);
        let mut out = Vec::new();
        let (mut ctrl, mut piece) = (0usize, 0usize);
        let mut solo = ' ';
        let apply_solo = |d: &mut Disco, solo: char| {
            if solo != ' ' {
                if solo != 'a' {
                    d.impact_gain = 0.0;
                }
                if solo != 'b' {
                    d.body_gain = 0.0;
                }
                if solo != 'n' {
                    d.noise_gain = 0.0;
                }
            }
        };
        for line in script.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            match w.first().copied() {
                Some("p") => d.set_param(w[1].parse::<f32>().unwrap() as u32, w[2].parse().unwrap()),
                Some("on") => {
                    d.start(w[1].parse().unwrap(), w[2].parse::<f32>().unwrap() as u32);
                    apply_solo(&mut d, solo);
                }
                Some("solo") => solo = w[1].chars().next().unwrap(),
                Some("choke") | Some("leave") => d.leave(),
                Some("run") => {
                    apply_solo(&mut d, solo);
                    let mut n: usize = w[1].parse().unwrap();
                    while n > 0 {
                        if ctrl == 0 {
                            d.control_rate();
                            ctrl = 128;
                        }
                        if piece == 0 {
                            piece = 16;
                        }
                        let k = n.min(ctrl).min(piece);
                        let (mut l, mut r) = ([0f32; 16], [0f32; 16]);
                        d.process(&mut l[..k], &mut r[..k]);
                        for j in 0..k {
                            out.extend_from_slice(&l[j].to_le_bytes());
                            out.extend_from_slice(&r[j].to_le_bytes());
                        }
                        ctrl -= k;
                        piece -= k;
                        n -= k;
                    }
                }
                Some("has") => eprintln!("has {}", u8::from(d.has_output())),
                Some("dbg") => eprintln!("{}", d.att.dbg()),
                _ => {}
            }
        }
        std::fs::write(std::env::var("DISCO_OUT").unwrap(), out).unwrap();
    }
}
