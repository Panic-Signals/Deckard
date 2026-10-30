//! The Kick's **Tronic** engine: `NI::DSP::CleanBDCore` (docs/synth-kick.md §4.2). A
//! sine-shaped oscillator (a triangle through a soft "broken sine" shaper) whose pitch
//! falls along a clipped exponential sweep, phase-modulated by a second triangle; a
//! filtered noise/click layer; a drive stage (two state-variable filters, a soft clip with
//! bias, high- and low-passes) mixed against the clean body by Mix. Field comments give
//! the member offset in `CleanBDCore`; operations follow the assembly of
//! `processAudioImpl<…>` (0x1018d5cac, 0x1018d6bfc, 0x1018d7bc8, 0x1018d8adc).

use super::env::{Attack, Decay, FreeEnv, Release};
use super::smooth::Smooth;
use super::sub::{exp2_poly, log_time, pitch_hz, tan_approx};
use crate::fx::exp_e;

const PI: f32 = std::f32::consts::PI;

/// Smoothed values (+0x870 current, +0x8b0 step, +0x8f0 target).
const LPA: usize = 0; // low-pass A cutoff (Hz)
const HP1: usize = 1; // drive high-pass cutoff
const SVF3: usize = 2; // drive band-pass cutoff
const SVF2: usize = 3; // pre-drive low-pass cutoff
const MIX: usize = 4;
const BIAS: usize = 5;
const RES: usize = 6; // drive band-pass resonance
const BP_MIX: usize = 7;
const GAIN: usize = 8; // drive output level
const REL_K: usize = 9; // release factor of envelope D
const PITCH: usize = 10;
const RATE: usize = 11; // pitch-sweep rate
const SUS: usize = 12;
const DEC: usize = 13;
const REL: usize = 14;

/// A one-pole TPT filter (low-pass output `g·(z + y)`, high-pass `y − z`).
#[derive(Clone, Copy, Default, Debug)]
struct OnePole {
    z: f32,
    a: f32,
    b: f32,
    g: f32,
    hz: f32,
}

impl OnePole {
    fn set(&mut self, inv_sr: f32, hz: f32) {
        self.hz = hz;
        let g = tan_approx(inv_sr * (hz * PI));
        self.g = g;
        self.a = 1.0 / (g + 1.0);
        self.b = g + -1.0;
    }
    #[inline]
    fn lp(&mut self, x: f32) -> f32 {
        let y = self.a * (-self.b).mul_add(self.z, x);
        let o = self.g * (self.z + y);
        self.z = y;
        o
    }
    #[inline]
    fn hp(&mut self, x: f32) -> f32 {
        let y = self.a * (-self.b).mul_add(self.z, x);
        let o = y - self.z;
        self.z = y;
        o
    }
}

/// A state-variable filter (TPT): `k` damping, `d` the loop gain.
#[derive(Clone, Copy, Default, Debug)]
struct Svf {
    z1: f32,
    z2: f32,
    g: f32,
    k: f32,
    hz: f32,
    d: f32,
    res: f32,
}

impl Svf {
    fn set(&mut self, sr: f32, inv_sr: f32, hz: f32) {
        let hz = if hz < 0.0 { 0.0 } else { hz };
        let lim = sr * f32::from_bits(0x3efa_e148);
        let hz = if hz > lim { lim } else { hz };
        self.hz = hz;
        let g = tan_approx(inv_sr * (hz * PI));
        self.g = g;
        self.d = 1.0 / g.mul_add(self.k + g, 1.0);
    }
    fn set_res(&mut self, r: f32) {
        self.res = r;
        let r = if r < 0.0 { 0.0 } else { r };
        self.k = if r > 0.99 { f32::from_bits(0x3ca3_d700) } else { (1.0 - r) + (1.0 - r) };
        self.d = 1.0 / self.g.mul_add(self.k + self.g, 1.0);
    }
    /// Returns (band-pass, low-pass).
    #[inline]
    fn process(&mut self, x: f32) -> (f32, f32) {
        let (g, k, z1, z2) = (self.g, self.k, self.z1, self.z2);
        let s4 = z1 + z1;
        let s6 = z2.mul_add(2.0, g * s4);
        let gg = g.mul_add(k, g * g);
        let s4 = s4.mul_add(k, s6);
        let hp = x - self.d * x.mul_add(gg, s4);
        let v1 = g.mul_add(hp, z1);
        let bp = z1 + v1;
        let v2 = g.mul_add(bp, z2);
        self.z1 = v1;
        self.z2 = v2;
        (bp, z2 + v2)
    }
}

/// A soft clipper: linear below `thr`, then `thr + d·(1 − sat·d/4)` up to `2·knee`.
#[derive(Clone, Copy, Debug)]
struct Clip {
    thr: f32,
    knee: f32,
    sat: f32,
    gain: f32,
}

impl Clip {
    #[inline]
    fn side(&self, x: f32) -> f32 {
        if self.thr <= x {
            let d = x - self.thr;
            let k = self.knee + self.knee;
            let d = if k < d { k } else { d };
            let s = (d * -0.25).mul_add(self.sat, 1.0);
            self.gain * d.mul_add(s, self.thr)
        } else {
            self.gain * x
        }
    }
}

/// The anticlick crossfade from the previous note's output.
#[derive(Clone, Copy, Default, Debug)]
struct Xfade {
    freq: f32,
    pos: f32,
    acc: f32,
    now: f32,
    out: f32,
    prev: f32,
    step: f32,
    start: f32,
    k: f32,
    rate: f32,
    inv_sr: f32,
}

impl Xfade {
    fn reset_sr(&mut self, inv: f32) {
        self.inv_sr = inv;
        let f = inv * self.freq;
        self.pos = 1.0;
        self.acc = 0.0;
        self.now = 0.0;
        self.out = 0.0;
        self.prev = 0.0;
        self.step = 0.0;
        self.start = 0.0;
        self.k = 2.0;
        self.rate = f + f;
    }
    fn note_on(&mut self, freq: f32) {
        self.freq = freq;
        self.rate = freq * self.inv_sr * self.k;
        self.pos = 1.0;
        self.acc = 0.0;
        self.step = self.out - self.prev;
        self.start = self.now;
    }
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        self.prev = self.out;
        self.now = x;
        let mut o = x;
        if self.pos > 0.0 {
            let p = self.pos - self.rate;
            self.pos = p;
            let s = p + -0.5;
            let s = (-s).mul_add(s.abs(), s).mul_add(2.0, 0.5);
            let acc = self.acc;
            self.acc = acc + self.step;
            o += (self.start + acc) * s;
        }
        self.out = o;
        o
    }
}

pub struct Tronic {
    /// +0x80…: raw parameters (0 tune, 1 decay, 2 bend, 3 impact, 4 gain, 5 tone,
    /// 6 bias, 7 mix).
    raw: [f32; 8],
    /// +0xa0 tone (80…152), +0xa4 drive, +0xa8 pitch (19…43 st), +0xac sweep time,
    /// +0xb0 sweep amount, +0xb4 decay (42…60).
    tone: f32,
    drive: f32,
    pitch: f32,
    bend_t: f32,
    bend_amt: f32,
    decay: f32,
    /// +0xb8 attack time, +0xbc click level, +0xc0 noise level, +0xc4 PM depth,
    /// +0xc8 PM frequency, +0xcc click decay, +0xd0 frequency, +0xd4 drive amount.
    att: f32,
    click: f32,
    noise_lvl: f32,
    pm_depth: f32,
    pm_freq: f32,
    click_dec: f32,
    freq: f32,
    drive_amt: f32,
    /// +0xd8 note; +0xdc the click counter's step, +0xe0 the counter, +0xe4 fired.
    note: u32,
    cnt_step: f32,
    cnt: f32,
    fired: bool,
    /// +0xe8/+0xf0: envelopes A/D × velocity (for the next note); +0xec/+0xf4 the levels
    /// the next note starts from.
    a_last: f32,
    a_start: f32,
    d_last: f32,
    d_start: f32,
    /// +0xf8 main phase, +0x104 1/sr; +0x110 PM phase, +0x114 PM freq, +0x118 its
    /// increment, +0x11c 1/sr, +0x120 PM phase offset.
    phase: f32,
    inv_sr: f32,
    pm_phase: f32,
    pm_inc: f32,
    /// +0x134 noise generator, +0x138 the noise walk, +0x13c noise scale.
    seed: i32,
    walk: f32,
    noise_scale: f32,
    /// +0x140: the 15 Hz high-pass on the noise.
    hp15: OnePole,
    /// +0x160: the oscillator's shaper; +0x198 the pitch sweep's phase and its shaper.
    shape_pos: Clip,
    shape_neg: Clip,
    shape_k: f32,
    sweep: f32,
    sweep_pos: Clip,
    sweep_neg: Clip,
    /// +0x1d0 noise band-pass, +0x200 pre-drive low-pass, +0x230 drive band-pass.
    svf1: Svf,
    svf2: Svf,
    svf3: Svf,
    sr: f32,
    /// +0x260 drive high-pass, +0x280 DC high-pass (10 Hz), +0x2a0 low-pass A (tone),
    /// +0x2c0 low-pass B (100 Hz).
    hp1: OnePole,
    hp2: OnePole,
    lpa: OnePole,
    lpb: OnePole,
    /// +0x2e0 A (body), +0x3d8 B (PM), +0x4d0 C (noise), +0x5c8 D (drive),
    /// +0x6c0 E (drive output, smoothed) envelopes.
    env_a: FreeEnv,
    env_b: FreeEnv,
    env_c: FreeEnv,
    env_d: FreeEnv,
    env_e: FreeEnv,
    /// +0x7bc: the click envelope, +0x7c0 its factor, +0x7c8 its floor.
    pd: f32,
    pd_coef: f32,
    pd_floor: f32,
    /// +0x7cc / +0x7f8: anticlick crossfades of the body and the drive.
    xf_a: Xfade,
    xf_b: Xfade,
    /// +0x824: the output gate (0 off, 1 on, 2 fading), +0x830 its level, +0x834 factor.
    gate: u32,
    gate_v: f32,
    gate_coef: f32,
    /// +0x844, +0x854: the click and drive clip levels (2), +0x848, +0x858 saturation.
    clip1: f32,
    sat1: f32,
    clip2: f32,
    sat2: f32,
    sm: Smooth<16>,
    /// +0x950 velocity, +0x954 played.
    vel: f32,
    played: bool,
    /// The `processAudioImpl<·, true>` variants (the first chunk after a note).
    first: bool,
}

impl Tronic {
    /// `CleanBDCore()` then `onSetSampleRate(sr, 128)`. `prev_sr` is the rate of an
    /// earlier `onSetSampleRate` call (−1 if none): the click counter's step reads it.
    pub fn new(sr: f32, prev_sr: f32) -> Tronic {
        let mut env_a = FreeEnv::new(Decay::Lin, Release::Log);
        env_a.attack_kind = Attack::Inverted;
        env_a.att_shape = f64::from(f32::from_bits(0x3e4c_cccd));
        env_a.rel_shape = -1.0;
        env_a.vel_sens = 0.0;
        let mut env_d = env_a.clone();
        let mut env_e = env_a.clone();
        env_e.set_smooth_hz(1000.0);
        env_d.sustain = 0.5;
        let mut env_b = FreeEnv::new(Decay::Lin, Release::Lin);
        env_b.sustain = f32::from_bits(0x3d4c_cccd);
        env_b.vel_sens = 0.0;
        let mut env_c = FreeEnv::new(Decay::Shape, Release::Lin);
        env_c.sustain = 0.0;
        env_c.dec_shape = 0.5;
        env_c.vel_sens = 0.0;
        let third = f32::from_bits(0x3f2a_aaab);
        let mut t = Tronic {
            raw: [0.0; 8],
            tone: 0.0,
            drive: 0.0,
            pitch: 0.0,
            bend_t: 0.0,
            bend_amt: 0.0,
            decay: 0.0,
            att: 0.0,
            click: 0.0,
            noise_lvl: 0.0,
            pm_depth: 0.0,
            pm_freq: 0.0,
            click_dec: 0.0,
            freq: 0.0,
            drive_amt: 0.0,
            note: 0,
            cnt_step: 0.0,
            cnt: 0.0,
            fired: false,
            a_last: 0.0,
            a_start: 0.0,
            d_last: 0.0,
            d_start: 0.0,
            phase: 0.0,
            inv_sr: 1.0,
            pm_phase: 0.0,
            pm_inc: 0.0,
            seed: 0,
            walk: 0.0,
            noise_scale: 1.0,
            hp15: OnePole { hz: 1.0, ..OnePole::default() },
            shape_pos: Clip { thr: third, knee: third, sat: 0.0, gain: 1.0 },
            shape_neg: Clip { thr: third, knee: third, sat: 0.0, gain: -1.0 },
            shape_k: f32::from_bits(0x3f49_0fdb),
            sweep: 0.0,
            sweep_pos: Clip {
                thr: f32::from_bits(0x3f0c_cccd),
                knee: f32::from_bits(0x3ee6_6666),
                sat: f32::from_bits(0x400e_38e4),
                gain: 1.0,
            },
            sweep_neg: Clip {
                thr: f32::from_bits(0x3f0c_cccd),
                knee: f32::from_bits(0x3ee6_6666),
                sat: f32::from_bits(0x400e_38e4),
                gain: -1.0,
            },
            svf1: Svf { k: 2.0, hz: 1.0, res: 0.0, ..Svf::default() },
            svf2: Svf { k: f32::from_bits(0x3ef5_c290), hz: 1.0, res: f32::from_bits(0x3f42_8f5c), ..Svf::default() },
            svf3: Svf { k: 0.5, hz: 1.0, res: 0.75, ..Svf::default() },
            sr,
            hp1: OnePole { hz: 1.0, ..OnePole::default() },
            hp2: OnePole { hz: 1.0, ..OnePole::default() },
            lpa: OnePole { hz: 1.0, ..OnePole::default() },
            lpb: OnePole { hz: 1.0, ..OnePole::default() },
            env_a,
            env_b,
            env_c,
            env_d,
            env_e,
            pd: 0.0,
            pd_coef: 0.0,
            pd_floor: 1e-4,
            xf_a: Xfade { freq: 1.0, ..Xfade::default() },
            xf_b: Xfade { freq: 1.0, ..Xfade::default() },
            gate: 0,
            gate_v: 0.0,
            gate_coef: 0.0,
            clip1: 2.0,
            sat1: 1.0,
            clip2: 2.0,
            sat2: 1.0,
            sm: Smooth::new(),
            vel: 0.0,
            played: false,
            first: true,
        };
        t.set_sample_rate(sr, 128, prev_sr);
        t
    }

    /// `onSetSampleRate(sr, block)` (0x1018d28a8).
    fn set_sample_rate(&mut self, sr: f32, block: i32, prev_sr: f32) {
        self.sm.set_rate(sr, block);
        let inv = 1.0 / sr;
        self.inv_sr = inv;
        self.pm_inc = (inv * self.pm_freq).clamp(-1.0, 1.0);
        let q = (sr / 44100.0).sqrt();
        self.noise_scale = 1.0 / q;
        // 15 Hz high-pass on the noise walk.
        let g = tan_approx(inv * f32::from_bits(0x423c_7edd));
        self.hp15 = OnePole { z: 0.0, a: 1.0 / (g + 1.0), b: g + -1.0, g, hz: 15.0 };
        // Noise band-pass at min(0.49·sr, 4 kHz).
        self.sr = sr;
        let f = (sr * f32::from_bits(0x3efa_e148)).min(4000.0);
        self.svf1.set(sr, inv, f);
        self.svf1.z1 = 0.0;
        self.svf1.z2 = 0.0;
        let f = self.svf2.hz;
        self.svf2.set(sr, inv, f);
        self.svf2.z1 = 0.0;
        self.svf2.z2 = 0.0;
        let f = self.hp1.hz;
        self.hp1.set(inv, f);
        self.hp1.z = 0.0;
        let f = self.svf3.hz;
        self.svf3.set(sr, inv, f);
        self.svf3.z1 = 0.0;
        self.svf3.z2 = 0.0;
        let g = tan_approx(inv * f32::from_bits(0x41fb_53d2));
        self.hp2 = OnePole { z: 0.0, a: 1.0 / (g + 1.0), b: g + -1.0, g, hz: 10.0 };
        let f = self.lpa.hz;
        self.lpa.set(inv, f);
        self.lpa.z = 0.0;
        let g = tan_approx(inv * f32::from_bits(0x439d_1463));
        self.lpb = OnePole { z: 0.0, a: 1.0 / (g + 1.0), b: g + -1.0, g, hz: 100.0 };
        self.cnt_step = 1.0 / (prev_sr * 0.001);
        self.env_a.set_samplerate(sr);
        self.env_d.set_samplerate(sr);
        self.env_e.set_samplerate(sr);
        // The click envelope: 0.8 ms.
        self.pd = 0.0;
        let ln_half = inv * f32::from_bits(0xbf31_7215);
        let e = exp_e((ln_half / f32::from_bits(0x3a51_b717)).mul_add(f32::from_bits(0x42ad_1f97), 1200.0));
        self.pd_coef = e.clamp(0.0, 1.0);
        self.env_b.set_samplerate(sr);
        self.env_b.set_decay(f32::from_bits(0x3b03_126f));
        self.env_b.set_release(f32::from_bits(0x3b03_126f));
        self.env_c.set_samplerate(sr);
        self.env_c.set_decay(f32::from_bits(0x3cb8_51ec));
        self.env_c.set_release(0.0);
        self.xf_a.reset_sr(inv);
        self.xf_b.reset_sr(inv);
        self.gate = 0;
        self.gate_v = 0.0;
        self.gate_coef = (ln_half / 0.005).exp().min(1.0);
        self.bend_rate();
        self.sm.touch();
    }

    /// `1/(sweep time · 2^((31 − pitch)/12) · sr + 1e-30)`: the sweep's rate.
    fn bend_rate(&mut self) {
        let e = exp2_poly((self.pitch + -31.0) / -12.0);
        self.sm.target[RATE] = 1.0 / (self.bend_t * e).mul_add(self.sr, f32::from_bits(0x0da2_4260));
    }

    /// The drive amount's derived targets (`calcRatGainParams`, 0x1018d4ad8).
    fn rat_gain(&mut self) {
        let d = self.drive_amt;
        let a = d.mul_add(f32::from_bits(0x3f8a_3d71), f32::from_bits(0xbedc_28f6));
        let a = if a < 0.0 { 0.0 } else { a };
        self.sm.target[RES] = a;
        let e = exp_e((d * 60.0).mul_add(f32::from_bits(0x411f_73da), 1200.0));
        self.sm.target[BP_MIX] = a.mul_add(-2.0, 2.0) * (e + -1.0);
        let c = d.clamp(0.0, 1.0);
        self.sm.target[GAIN] = exp_e(c.mul_add(-4.0, 0.0).mul_add(f32::from_bits(0x411f_73da), 1200.0));
        self.sm.target[BIAS] = self.raw[6].clamp(0.0, 1.0).mul_add(4.0, 0.0);
        self.sm.touch();
    }

    /// `calcRatFreqParams()` (0x1018d4858).
    fn rat_freq(&mut self) {
        let t = self.tone;
        self.sm.target[LPA] = pitch_hz(t.mul_add(0.25, 80.0));
        let c = ((t + -125.0) / 25.0).clamp(0.0, 1.0);
        self.sm.target[HP1] = pitch_hz(c.mul_add(20.0, 10.0));
        self.sm.touch();
        let c = ((t + -140.0) / -60.0).clamp(0.0, 1.0);
        let g = self.drive;
        self.drive_amt = (g * f32::from_bits(0x3f19_999a) - g).mul_add(c, g);
        self.rat_gain();
        let f = self.drive_amt.mul_add(-35.0, self.tone);
        let hi = f + 40.0;
        let f = if f > 100.0 { 100.0 } else { f };
        self.sm.target[SVF3] = pitch_hz(f);
        self.sm.target[SVF2] = pitch_hz(hi);
        self.sm.touch();
    }

    /// The sustain target from the sweep amount and the decay.
    fn sustain(&mut self) {
        let s = (self.bend_amt / f32::from_bits(0x3f33_3333)).clamp(0.0, 1.0);
        let s = s * s.abs().mul_add(0.5, 0.5);
        let s = s.clamp(0.0, 1.0);
        let s = s.mul_add(f32::from_bits(0x3f59_999a), f32::from_bits(0x3e19_999a));
        let d = ((self.decay + -46.5) / 6.5).clamp(0.0, 1.0);
        let d = d.mul_add(0.5, 0.0).clamp(0.0, 1.0);
        let s = (1.0 - s).mul_add(d, s).clamp(0.0, 1.0);
        self.sm.target[SUS] = s.mul_add(f32::from_bits(0x3f4c_cccd), f32::from_bits(0x3e4c_cccd));
    }

    /// `setParameter(i, v)` (0x1018d40a8): 0 tune, 1 decay, 2 bend, 3 impact, 4 gain,
    /// 5 tone, 6 bias, 7 mix.
    pub fn set(&mut self, i: usize, v: f32) {
        if i > 7 {
            return;
        }
        self.raw[i] = v;
        let c = v.clamp(0.0, 1.0);
        match i {
            0 => {
                let p = c.mul_add(24.0, 19.0) + (self.note as f32 + -60.0);
                self.pitch = p.clamp(19.0, 43.0);
                self.sm.target[PITCH] = self.pitch;
                self.bend_rate();
                self.sm.touch();
            }
            1 => {
                let d = c.mul_add(18.0, 42.0);
                self.decay = d;
                self.sm.target[REL] = log_time(d);
                self.sm.target[DEC] = log_time(d + -17.0);
                self.bend_amt = ((self.bend_t + f32::from_bits(0xbda3_d70a)) / f32::from_bits(0x3eb8_51ec)).clamp(0.0, 1.0);
                self.sustain();
                self.sm.touch();
            }
            2 => {
                self.bend_t = c.mul_add(f32::from_bits(0x3f38_51ec), f32::from_bits(0x3da3_d70a));
                self.bend_rate();
                self.bend_amt = ((self.bend_t + f32::from_bits(0xbda3_d70a)) / f32::from_bits(0x3eb8_51ec)).clamp(0.0, 1.0);
                self.sustain();
                self.sm.touch();
            }
            3 => {}
            4 => {
                self.drive = c.mul_add(f32::from_bits(0x3fc6_6666), f32::from_bits(0x3dcc_cccd)).sqrt();
                self.rat_freq();
                self.level();
                self.sm.touch();
            }
            5 => {
                self.tone = c.mul_add(72.0, 80.0);
                self.rat_freq();
            }
            6 => {
                self.rat_gain();
                self.level();
                self.sm.touch();
            }
            _ => {
                self.sm.target[MIX] = v;
                self.sm.touch();
            }
        }
    }

    /// The drive envelope's release factor.
    fn level(&mut self) {
        let g = ((self.raw[4] + f32::from_bits(0xbe4c_cccd)) + (self.raw[4] + f32::from_bits(0xbe4c_cccd))).clamp(0.0, 1.0);
        let k = g.mul_add(-4.5, 6.0);
        let b = self.raw[6].clamp(0.0, 1.0);
        self.sm.target[REL_K] = k.mul_add(b, 0.0) + 1.0;
    }

    /// Whether anything sounds (`processAudio`, 0x1018d5ac8).
    pub fn active(&self) -> bool {
        (self.env_a.stage != 0 || self.env_e.stage != 0) && self.gate != 0
    }

    /// `onChoke()`.
    pub fn choke(&mut self) {
        if self.gate == 1 {
            self.gate = 2;
            self.gate_v = 1.0;
        }
    }

    pub fn control(&mut self) {
        self.sm.control();
    }

    /// `setNoteOn(velocity, note)` (0x1018d4c3c).
    pub fn note_on(&mut self, vel: f32, note: u32) {
        if !(0.0..=1.0).contains(&vel) {
            self.vel = 0.0;
            return;
        }
        self.note = note;
        let c = self.raw[0].clamp(0.0, 1.0);
        let p = (note as f32 + -60.0) + c.mul_add(24.0, 19.0);
        let p = p.clamp(19.0, 43.0);
        self.pitch = p;
        self.sm.target[PITCH] = p;
        let e = exp2_poly((p + -31.0) / -12.0);
        let rate = 1.0 / (self.bend_t * e).mul_add(self.sr, f32::from_bits(0x0da2_4260));
        self.sm.target[RATE] = rate;
        self.sm.touch();
        if (self.env_a.stage != 0 || self.env_e.stage != 0) && self.gate != 0 {
            for i in [MIX, BIAS, BP_MIX, GAIN, REL_K, SUS, DEC, REL] {
                self.sm.cur[i] = self.sm.target[i];
            }
            self.sm.cur[PITCH] = p;
            self.sm.cur[RATE] = rate;
        } else {
            self.sm.cur = self.sm.target;
            self.sm.state = 1;
        }
        self.sweep = 0.0;
        // Impact shapes the attack, click, noise and modulation.
        let im = self.raw[3];
        let a = (im / f32::from_bits(0x3f26_6666)).clamp(0.0, 1.0);
        self.att = log_time(a.mul_add(-30.0, 15.0));
        let h = ((im + -0.5) + (im + -0.5)).clamp(0.0, 1.0);
        let k = h.mul_add(40.0, im * 16.0);
        let v = vel.clamp(0.0, 1.0);
        let n = ((im + f32::from_bits(0xbf19_999a)) / f32::from_bits(0x3ecc_cccc)).clamp(0.0, 1.0);
        let m = (self.raw[7] / f32::from_bits(0x3eb3_3333)).clamp(0.0, 1.0);
        let q = ((im + f32::from_bits(0xbee6_6666)) / f32::from_bits(0x3f0c_cccd)).clamp(0.0, 1.0);
        let w = q.abs().mul_add(f32::from_bits(0x3e4c_cccd), f32::from_bits(0x3f4c_cccd));
        let vv = v.mul_add(f32::from_bits(0x3f19_999a), f32::from_bits(0x3f19_999a));
        self.click = vv * k;
        self.noise_lvl = n * m;
        self.pm_depth = v.mul_add(0.5, 0.5) * ((q * w) * 0.5);
        let s = q.mul_add(7.0, self.bend_amt.mul_add(6.0, p + 49.0));
        self.click_dec = f32::from_bits(0x3d4c_cccd) / exp2_poly(s / 12.0);
        let f = pitch_hz(s);
        self.pm_freq = f;
        self.phase = f32::from_bits(0x3eb3_3333);
        self.pm_phase = 0.25;
        self.pm_inc = (f * self.inv_sr).clamp(-1.0, 1.0);
        self.cnt = 0.0;
        self.fired = false;
        self.env_a.set_attack(self.att);
        self.env_a.note_on(vel);
        self.env_d.set_attack(self.att);
        self.env_d.note_on(vel);
        self.a_start = if self.a_last <= vel { vel } else { self.a_last };
        self.d_start = if self.d_last > vel { self.d_last } else { vel };
        self.env_e.set_attack(self.att);
        self.env_e.note_on(vel);
        self.env_b.set_attack(self.click_dec);
        self.env_b.note_on(vel);
        self.env_c.set_attack(self.att);
        self.env_c.note_on(vel);
        self.xf_a.note_on(self.freq);
        self.xf_b.note_on(self.freq);
        self.gate = 1;
        self.gate_v = 0.0;
        self.vel = vel;
        self.played = true;
        self.first = true;
    }

    /// What `processAudio` resets once the sound has ended.
    fn idle_reset(&mut self) {
        self.vel = 0.0;
        self.played = false;
        self.a_last = 0.0;
        self.a_start = 0.0;
        self.d_last = 0.0;
        self.d_start = 0.0;
        self.sm.cur = self.sm.target;
        self.sm.touch();
        self.phase = 0.0;
        self.pm_phase = 0.0;
        self.hp15.z = 0.0;
        self.svf1.z1 = 0.0;
        self.svf1.z2 = 0.0;
        self.svf2.z1 = 0.0;
        self.svf2.z2 = 0.0;
        self.svf3.z1 = 0.0;
        self.svf3.z2 = 0.0;
        self.hp1.z = 0.0;
        self.hp2.z = 0.0;
        self.lpa.z = 0.0;
        self.lpb.z = 0.0;
        for e in [&mut self.env_a, &mut self.env_b, &mut self.env_c, &mut self.env_d, &mut self.env_e] {
            e.stage = 0;
            e.out = 0.0;
            e.x = 1.0;
            e.t = 0.0;
        }
        self.pd = 0.0;
        self.xf_a.pos = 1.0;
        self.xf_a.acc = 0.0;
        self.xf_a.now = 0.0;
        self.xf_a.out = 0.0;
        self.xf_a.prev = 0.0;
        self.xf_a.step = 0.0;
        self.xf_a.start = 0.0;
        self.xf_b.pos = 1.0;
        self.xf_b.acc = 0.0;
        self.xf_b.now = 0.0;
        self.xf_b.out = 0.0;
        self.xf_b.prev = 0.0;
        self.xf_b.step = 0.0;
        self.xf_b.start = 0.0;
        self.gate = 0;
        self.gate_v = 0.0;
        self.first = true;
    }

    /// One chunk (`processAudio`).
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) -> bool {
        if !self.active() {
            if self.played {
                self.idle_reset();
            }
            return false;
        }
        let first = self.first;
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            let y = self.sample(first);
            *a = y;
            *b = y;
        }
        self.first = false;
        true
    }

    #[inline]
    fn sample(&mut self, first: bool) -> f32 {
        if self.sm.ramping {
            self.sm.tick();
        }
        let cur = self.sm.cur;
        // The pitch sweep: a ramp through an asymmetric soft clip, then 2^(−2.4·x).
        let x = cur[RATE].mul_add(f32::from_bits(0x401a_b50b), self.sweep);
        self.sweep = x;
        let s = if x > 0.0 { self.sweep_pos.side(x) } else { self.sweep_neg.side(-x) };
        let s = s.mul_add(f32::from_bits(0xc01a_b50b), f32::from_bits(0x40a4_05fb));
        let e = exp2_poly(s) + f32::from_bits(0xc0d0_51ec);
        let f = pitch_hz(cur[PITCH] + e);
        self.freq = f;
        // The modulating triangle.
        let p2 = self.pm_phase + self.pm_inc + 10.0;
        let p2 = p2 - ((p2 + 0.5) as i32 as f32);
        self.pm_phase = p2;
        let t = p2 + 10.0;
        let t = t - ((t + 0.5) as i32 as f32);
        let tri2 = (t + t).abs() + -0.5;
        let depth = self.pm_depth * tri2;
        let pm = depth * self.env_b.process();
        // The main oscillator.
        let inc = (f * self.inv_sr).clamp(-1.0, 1.0);
        let p = self.phase + inc + 10.0;
        let p = p - ((p + 0.5) as i32 as f32);
        self.phase = p;
        let t = pm + p + 10.0;
        let t = t - ((t + 0.5) as i32 as f32);
        let tri = (t + t).abs() + -0.5;
        let osc = self.shaper(tri * 4.0);
        // Noise: a bounded random walk, high-passed at 15 Hz.
        self.seed = self.seed.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        let n = self.seed as f32 * f32::from_bits(0x3000_009d);
        let w = self.walk;
        let up = w + n;
        self.walk = if up.abs() > 333.0 { w - n } else { up };
        let hp = self.hp15.hp(w * f32::from_bits(0x3cf8_3dff));
        let nz = self.noise_scale * hp;
        let ec = self.env_c.process();
        let ec = if ec > 0.2 { 0.2 } else { ec };
        let nz = nz * self.noise_lvl.mul_add(0.5, self.click * ec);
        let (bp, _) = self.svf1.process(nz);
        // The click envelope fires 1 ms in.
        if !self.fired {
            if self.cnt >= 1.0 {
                self.fired = true;
                self.pd = 1.0;
            } else {
                self.cnt += self.cnt_step;
            }
        }
        let v = self.pd_coef * self.pd;
        self.pd = v;
        let pdv = if v < self.pd_floor {
            self.pd = 0.0;
            0.0
        } else {
            v - self.pd_floor
        };
        let c = (bp * 10.0).clamp(-self.clip1, self.clip1);
        // Envelope parameters follow the smoothed values.
        if self.sm.check(first, DEC) {
            let d = cur[DEC];
            self.env_a.set_decay(d);
            self.env_d.set_decay(d);
            self.env_e.set_decay(d * f32::from_bits(0x3fe6_6666));
        }
        if self.sm.check(first, SUS) {
            self.env_a.sustain = cur[SUS];
            self.env_d.sustain = cur[SUS];
            self.env_e.sustain = cur[SUS];
        }
        let rel = cur[REL];
        if self.sm.check(first, REL) {
            self.env_a.set_release(rel);
            self.env_e.set_release(rel * f32::from_bits(0x3fe6_6666));
        }
        if first || (self.sm.ramping && self.sm.step[REL_K] != 0.0) {
            self.env_d.set_release(rel * cur[REL_K]);
        }
        let ea = self.env_a.process();
        self.a_last = ea;
        let click = pdv * (c * (c.abs() * -0.25).mul_add(self.sat1, 1.0));
        let ed = self.env_d.process();
        let body = bp * (ed * (ed * ed));
        let k = if ea < 0.0 { 0.0 } else { ea };
        let k = if k > 1.0 { 1.0 } else { k + 0.0 };
        let k = if k > 1.0 { 1.0 } else { k };
        self.d_last = ed;
        let body = click + body * (1.0 - osc).mul_add(k, osc);
        let ya = (ea * self.a_start).mul_add(osc, (ea * self.vel) * body);
        let ya = self.xf_a.process(ya);
        let yb = (ed * self.d_start).mul_add(osc, (ed * self.vel) * body);
        let yb = self.xf_b.process(yb);
        // The drive.
        if self.sm.check(first, SVF2) {
            let f = cur[SVF2];
            if self.svf2.hz != f {
                self.svf2.set(self.sr, self.inv_sr, f);
            }
        }
        let (_, lp) = self.svf2.process(yb);
        if self.sm.check(first, HP1) {
            let f = cur[HP1];
            if self.hp1.hz != f {
                self.hp1.set(self.inv_sr, f);
            }
        }
        let h = self.hp1.hp(yb.mul_add(f32::from_bits(0x3dcc_cccd), lp));
        if self.sm.check(first, SVF3) {
            let f = cur[SVF3];
            if self.svf3.hz != f {
                self.svf3.set(self.sr, self.inv_sr, f);
            }
        }
        if self.sm.check(first, RES) {
            let r = cur[RES];
            if self.svf3.res != r {
                self.svf3.set_res(r);
            }
        }
        let (bp3, _) = self.svf3.process(h);
        let d = cur[BP_MIX].mul_add(bp3, h) + cur[BIAS];
        let d = d.clamp(-self.clip2, self.clip2);
        let d = d * (d.abs() * -0.25).mul_add(self.sat2, 1.0);
        let d = self.hp2.hp(d);
        if self.sm.check(first, LPA) {
            let f = cur[LPA];
            if self.lpa.hz != f {
                self.lpa.set(self.inv_sr, f);
            }
        }
        let la = self.lpa.lp(d);
        let lb = self.lpb.lp(d);
        let ee = self.env_e.process();
        let m = cur[MIX].clamp(0.0, 1.0);
        let gate = match self.gate {
            0 => 0.0,
            1 => 1.0,
            _ => {
                let v = self.gate_coef * self.gate_v;
                self.gate_v = v;
                if (v as f64) < super::env::END {
                    self.gate_v = 0.0;
                    self.gate = 0;
                    0.0
                } else {
                    v
                }
            }
        };
        self.a_last *= self.vel;
        self.d_last *= self.vel;
        let dry = ya * 0.5;
        let wet = cur[GAIN] * la.mul_add(f32::from_bits(0x3ea3_d70a), lb * 0.25);
        let wet = (wet * ee) * (m * (2.0 - m));
        let im = 1.0 - m;
        let out = (im * (2.0 - im)).mul_add(dry, wet);
        out * f32::from_bits(0x3ee3_d1ee) * gate
    }

    /// `BrokenSineShaperStatic::process` (0x101979de0): linear below a threshold, a
    /// quarter sine above.
    fn shaper(&self, x: f32) -> f32 {
        let (c, x) = if x > 0.0 { (&self.shape_pos, x) } else { (&self.shape_neg, -x) };
        if c.thr > x {
            return c.gain * x;
        }
        let d = x - c.thr;
        let k2 = c.knee + c.knee;
        let d = if k2 < d { k2 } else { d };
        let y = (self.shape_k * d) / c.knee;
        let y2 = y * y;
        let q = y2.mul_add(f32::from_bits(0xb2cd_4ca8), f32::from_bits(0x3638_b94f));
        let q = q.mul_add(y2, f32::from_bits(0xb950_0c03));
        let q = q.mul_add(y2, f32::from_bits(0x3c08_8885));
        let q = q.mul_add(y2, f32::from_bits(0xbe2a_aac1));
        let s = y * q.mul_add(y2, 1.0);
        c.gain * (c.thr + (c.knee * s) / self.shape_k)
    }
}
