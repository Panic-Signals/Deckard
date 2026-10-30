//! The Kick's **Sub** engine: `NI::DSP::SubKickCore` (docs/synth-kick.md §4.1). A sine
//! (a triangle shaped by a polynomial) swept down by a pitch envelope, an amplitude
//! envelope with a fast-release follower, a soft-clipped band-passed noise click for the
//! Thick and Noise attack modes, and a one-pole low-pass. Twelve control values are
//! smoothed over ~20 ms when parameters change. Field comments give the member offset in
//! `SubKickCore`; every operation follows the assembly of `processAudioImpl<…>`
//! (0x10192e0c0, 0x10192ed90, 0x10192fa0c, 0x1019306a4).

use super::env::{Decay, FreeEnv, Release};
use crate::fx::exp_e;

/// `E(5·p + 1381.8816)`: the frequency (Hz) of MIDI pitch `p`.
#[inline]
pub(super) fn pitch_hz(p: f32) -> f32 {
    exp_e(p.mul_add(5.0, f32::from_bits(0x44ac_bc36)))
}

/// `E(9.965784·x + 602.0529)`: a time (s) from a log-scaled value.
#[inline]
pub(super) fn log_time(x: f32) -> f32 {
    exp_e(x.mul_add(f32::from_bits(0x411f_73da), f32::from_bits(0x4416_8363)))
}

/// `tan(w)` by its Taylor series to the 5th power, as the filters compute it.
#[inline]
pub(super) fn tan_approx(w: f32) -> f32 {
    let w2 = w * w;
    let w3 = w * w2;
    let w5 = w2 * w3;
    w + w5.mul_add(f32::from_bits(0x3e08_8886), w3 * f32::from_bits(0x3eaa_aaaa))
}

/// `2^x` by a cubic, with the integer part rounded to nearest (`frinti`).
#[inline]
pub(super) fn exp2_poly(x: f32) -> f32 {
    let r = x.round_ties_even();
    let n = if r < 2147483648.0 && r > -2147483648.0 { r as i32 } else { i32::MIN };
    let f = x - n as f32;
    let p = f.mul_add(f32::from_bits(0x3d65_6042), f32::from_bits(0x3e77_ced9));
    let p = p.mul_add(f, f32::from_bits(0x3f31_6873));
    let p = p.mul_add(f, 1.0);
    p * f32::from_bits((0x3f80_0000u32).wrapping_add((n as u32) << 23))
}

/// A hold-then-decay click envelope (+0x538, +0x550, +0x648 in the core).
#[derive(Clone, Copy, Default, Debug)]
struct Hold {
    /// 0 off, 1 holding, 2 decaying.
    state: u32,
    /// The hold length (in `inc` units).
    hold: f32,
    /// Level while decaying / time while holding.
    v: f32,
    coef: f32,
    inc: f32,
}

impl Hold {
    #[inline]
    fn process(&mut self) -> f32 {
        match self.state {
            0 => 0.0,
            1 => {
                let v = self.inc + self.v;
                self.v = v;
                if v > self.hold {
                    self.v = 1.0;
                    self.state = 2;
                }
                1.0
            }
            _ => {
                let v = self.coef * self.v;
                self.v = v;
                if (v as f64) < super::env::END {
                    self.v = 0.0;
                    self.state = 0;
                    0.0
                } else {
                    v
                }
            }
        }
    }
}

/// The twelve smoothed control values (+0x670 current, +0x6a0 step, +0x6d0 target).
const N: usize = 12;
/// Indices into them.
const LPF: usize = 0;
const BASE: usize = 1;
const BEND: usize = 2;
const TOP: usize = 3;
const HOLD: usize = 4;
const TIME: usize = 5;
const DECAY: usize = 6;
const AMOUNT: usize = 7;
const ATTACK: usize = 8;
const COLOR: usize = 9;
const NOISE: usize = 10;

pub struct SubKick {
    /// +0x80…: the raw parameter values (0 tune … 6 color).
    raw: [f32; 7],
    /// +0x9c: tune in semitones (19…43); +0xa0 bend (0…40 st); +0xa4 the attack mode
    /// set; +0xc0 the mode of the playing note.
    tune: f32,
    bend: f32,
    mode_set: u32,
    mode: i32,
    /// +0xa8: tune + note − 60.
    base: f32,
    /// +0xac: Amount; +0xb0 color (75…110); +0xbc shaped amount.
    amount: f32,
    color: f32,
    amount_shaped: f32,
    /// +0xb4: the oscillator frequency; +0xb8 the top pitch (both as last played).
    freq: f32,
    top: f32,
    /// +0xcc: the oscillator's last sample; +0xd0, +0xd4 the anticlick start levels.
    sine: f32,
    osc_last: f32,
    osc_start: f32,
    /// +0xd8: the oscillator phase; +0xe4 1/sr.
    phase: f32,
    inv_sr: f32,
    /// +0x10c: the noise scale; +0x114 the noise generator's state.
    noise_scale: f32,
    seed: i32,
    /// +0x144: the phase-distortion envelope, +0x148 its factor, +0x150 its floor.
    pd: f32,
    pd_coef: f32,
    pd_floor: f32,
    /// +0x158: amplitude, +0x250 pitch, +0x348 noise envelopes.
    amp_env: FreeEnv,
    pitch_env: FreeEnv,
    noise_env: FreeEnv,
    /// +0x538: the pitch-click hold; +0x550 the high-pass click hold; +0x648 the output
    /// gate.
    click: Hold,
    hp_click: Hold,
    gate: Hold,
    /// +0x568…+0x590: the anticlick crossfade.
    xf_freq: f32,
    xf_pos: f32,
    xf_acc: f32,
    xf_now: f32,
    xf_prev: f32,
    xf_out: f32,
    xf_step: f32,
    xf_start: f32,
    xf_k: f32,
    xf_rate: f32,
    xf_inv_sr: f32,
    /// +0x594…+0x5c4: the amplitude follower: decay set, use flag, level, factor,
    /// output, held output, mix, threshold, inv sr, active flag, fade length, 1/length,
    /// fade counter.
    fol_decay: f32,
    fol_on: bool,
    fol_v: f32,
    fol_coef: f32,
    fol_out: f32,
    fol_held: f32,
    fol_mix: f32,
    fol_thr: f32,
    fol_inv_sr: f32,
    fol_active: bool,
    fade_len: u32,
    fade_inv: f32,
    fade_n: u32,
    /// +0x5c8…: the click high-pass (one-pole TPT) state and coefficients; +0x5e0 Hz.
    hp_z: f32,
    hp_out: f32,
    hp_a: f32,
    hp_b: f32,
    hp_g: f32,
    /// +0x5e8…: the output low-pass; +0x600 its cutoff (Hz), +0x604 1/sr.
    lp_z: f32,
    lp_a: f32,
    lp_b: f32,
    lp_g: f32,
    lp_hz: f32,
    lp_inv_sr: f32,
    /// +0x608…: the noise state-variable filter; +0x620 damping, +0x624 cutoff, +0x630 sr.
    svf_z1: f32,
    svf_z2: f32,
    svf_g: f32,
    svf_k: f32,
    svf_hz: f32,
    svf_d: f32,
    svf_sr: f32,
    svf_inv_sr: f32,
    /// +0x640: the noise clip level; +0x644 its saturation.
    clip: f32,
    sat: f32,
    /// +0x670…: smoothed values.
    cur: [f32; N],
    step: [f32; N],
    target: [f32; N],
    /// +0x700: targets changed; +0x701 smoothing; +0x704 control blocks left; +0x708
    /// ramp length (control blocks); +0x70c 1/(ramp samples); +0x710 the smoother state.
    dirty: bool,
    smoothing: bool,
    left: i32,
    ramp: i32,
    ramp_inv: f32,
    smooth_state: u32,
    /// +0x720: the velocity; +0x724 the note; +0x728 a note has played.
    vel: f32,
    note: u32,
    played: bool,
    /// The processAudioImpl variant: the first sample after a note recomputes the
    /// coefficients (`<·, true>`).
    first: bool,
}

impl SubKick {
    /// `SubKickCore()` then `onSetSampleRate(sr, 128)`.
    pub fn new(sr: f32) -> SubKick {
        let mut amp_env = FreeEnv::new(Decay::Log, Release::Lin);
        amp_env.sustain = 0.0;
        amp_env.gain = 0.6;
        amp_env.vel_sens = 0.0;
        let mut pitch_env = FreeEnv::new(Decay::Log, Release::Log);
        pitch_env.sustain = 1.0;
        pitch_env.gain = 1.0;
        pitch_env.vel_sens = 0.0;
        let mut noise_env = FreeEnv::new(Decay::Log, Release::Lin);
        noise_env.sustain = 0.0;
        noise_env.gain = 1.0;
        noise_env.vel_sens = 0.0;
        let mut s = SubKick {
            raw: [0.0; 7],
            tune: 0.0,
            bend: 0.0,
            mode_set: 3,
            mode: 3,
            base: 0.0,
            amount: 0.0,
            color: 0.0,
            amount_shaped: 0.0,
            freq: 0.0,
            top: 0.0,
            sine: 0.0,
            osc_last: 0.0,
            osc_start: 0.0,
            phase: 0.0,
            inv_sr: 1.0,
            noise_scale: f32::from_bits(0x3000_009d),
            seed: 0,
            pd: 0.0,
            pd_coef: 0.0,
            pd_floor: 1e-4,
            amp_env,
            pitch_env,
            noise_env,
            click: Hold { hold: 1.0, ..Hold::default() },
            hp_click: Hold { hold: 1.0, ..Hold::default() },
            gate: Hold::default(),
            xf_freq: 1.0,
            xf_pos: 1.0,
            xf_acc: 0.0,
            xf_now: 0.0,
            xf_prev: 0.0,
            xf_out: 0.0,
            xf_step: 0.0,
            xf_start: 0.0,
            xf_k: 0.5,
            xf_rate: 0.5,
            xf_inv_sr: 1.0,
            fol_decay: 1.0,
            fol_on: true,
            fol_v: 0.0,
            fol_coef: 0.0,
            fol_out: 0.0,
            fol_held: 0.0,
            fol_mix: 0.2,
            fol_thr: 1e-4,
            fol_inv_sr: 1.0,
            fol_active: false,
            fade_len: 1,
            fade_inv: 1.0,
            fade_n: 0,
            hp_z: 0.0,
            hp_out: 0.0,
            hp_a: 0.0,
            hp_b: 0.0,
            hp_g: 0.0,
            lp_z: 0.0,
            lp_a: 0.0,
            lp_b: 0.0,
            lp_g: 0.0,
            lp_hz: 1.0,
            lp_inv_sr: 1.0,
            svf_z1: 0.0,
            svf_z2: 0.0,
            svf_g: 0.0,
            svf_k: 2.0,
            svf_hz: 1.0,
            svf_d: 1.0,
            svf_sr: 1.0,
            svf_inv_sr: 1.0,
            clip: 0.2,
            sat: 10.0,
            cur: [0.0; N],
            step: [0.0; N],
            target: [0.0; N],
            dirty: true,
            smoothing: false,
            left: 0,
            ramp: 1,
            ramp_inv: 1.0,
            smooth_state: 1,
            vel: 0.0,
            note: 0,
            played: false,
            first: true,
        };
        s.set_sample_rate(sr, 128);
        s
    }

    /// `onSetSampleRate(sr, block)` (0x10192c69c).
    fn set_sample_rate(&mut self, sr: f32, block: i32) {
        let inv = 1.0 / sr;
        // +0xe0/+0xe4 and +0xf8/+0xfc: the oscillators' increments (unused second one).
        self.inv_sr = inv;
        // +0x10c, +0x11c: noise scale √(sr/44100)·2^-31.
        let q = (sr / 44100.0).sqrt();
        self.noise_scale = q * f32::from_bits(0x3000_009d);
        // +0x144…: the phase-distortion envelope: 100 ms to −60 dB… (`E(1200 + ln½/sr/0.1·86.56)`).
        let ln_half = inv * f32::from_bits(0xbf31_7215);
        let e = exp_e((ln_half / 0.1f32).mul_add(f32::from_bits(0x42ad_1f97), 1200.0));
        self.pd = 0.0;
        self.pd_coef = e.clamp(0.0, 1.0);
        // Envelopes.
        self.amp_env.set_samplerate(sr);
        self.amp_env.set_attack(exp_e(352.0 + f32::from_bits(0x3f68_8800)));
        self.amp_env.set_release(exp_e(602.0 + f32::from_bits(0x3d58_c000)));
        self.pitch_env.set_samplerate(sr);
        self.pitch_env.set_attack(exp_e(402.0 + f32::from_bits(0x3f3c_bc00)));
        self.click = Hold { state: 0, hold: self.click.hold, v: 0.0, coef: 0.0, inc: inv };
        self.click.coef = (ln_half / 1e-6).exp().min(1.0);
        self.hp_click = Hold { state: 0, hold: self.hp_click.hold, v: 0.0, coef: 0.0, inc: inv };
        self.hp_click.coef = (ln_half / 2e-5).exp().min(1.0);
        self.noise_env.set_samplerate(sr);
        self.noise_env.set_attack(0.0);
        self.noise_env.set_release(0.0);
        // +0x568…: anticlick.
        self.xf_inv_sr = inv;
        let f = inv * self.xf_freq;
        self.xf_pos = 1.0;
        self.xf_acc = 0.0;
        self.xf_step = 0.0;
        self.xf_start = 0.0;
        self.xf_now = 0.0;
        self.xf_prev = 0.0;
        self.xf_out = 0.0;
        self.xf_k = 2.0;
        self.xf_rate = f + f;
        // +0x594…: follower and fade.
        let n = (sr * f32::from_bits(0x3c75_c290)) as u32;
        self.fol_inv_sr = inv;
        self.fade_len = if n > 1 { n } else { 1 };
        self.fade_inv = 1.0 / self.fade_len as f32;
        self.fol_out = 0.0;
        self.fol_held = 0.0;
        self.fol_v = 0.0;
        self.fol_active = false;
        self.fade_n = 0;
        // +0x5c8: the click high-pass at 100 Hz.
        let g = tan_approx(inv * f32::from_bits(0x439d_1463));
        self.hp_g = g;
        self.hp_a = 1.0 / (g + 1.0);
        self.hp_b = g + -1.0;
        self.hp_z = 0.0;
        self.hp_out = 0.0;
        // +0x5e8: the output low-pass, at its last cutoff.
        self.lp_inv_sr = inv;
        self.set_lp(self.lp_hz);
        self.lp_z = 0.0;
        // +0x608: the noise filter.
        self.svf_sr = sr;
        self.svf_inv_sr = inv;
        let hz = self.svf_hz;
        self.set_svf(hz);
        self.svf_z1 = 0.0;
        self.svf_z2 = 0.0;
        // +0x648: the output gate (5 ms release).
        self.gate = Hold { state: 0, hold: 1.0, v: 0.0, coef: (ln_half / 0.005).exp().min(1.0), inc: inv };
        // +0x704…: the smoother ramp (20 ms in control blocks).
        let r = ((sr * 0.02) / block as f32 + 0.5) as i32;
        self.ramp = if r > 1 { r } else { 1 };
        self.ramp_inv = 1.0 / (self.ramp * block) as f32;
    }

    fn set_lp(&mut self, hz: f32) {
        self.lp_hz = hz;
        let g = tan_approx(self.lp_inv_sr * (hz * std::f32::consts::PI));
        self.lp_g = g;
        self.lp_a = 1.0 / (g + 1.0);
        self.lp_b = g + -1.0;
    }

    fn set_svf(&mut self, hz: f32) {
        let hz = if hz < 0.0 { 0.0 } else { hz };
        let lim = self.svf_sr * f32::from_bits(0x3efa_e148);
        let hz = if hz > lim { lim } else { hz };
        self.svf_hz = hz;
        let g = tan_approx(self.svf_inv_sr * (hz * std::f32::consts::PI));
        self.svf_g = g;
        self.svf_d = 1.0 / g.mul_add(self.svf_k + g, 1.0);
    }

    fn touch(&mut self) {
        self.dirty = true;
        self.smooth_state = 1;
    }

    /// `calcLPFFreq()` (0x10192df54): the output low-pass target.
    fn calc_lpf(&mut self) {
        let hz = if self.mode < 0 {
            let e = pitch_hz(self.base + 75.0);
            let a = (self.amount + self.amount).clamp(0.0, 1.0);
            (e + -300.0).mul_add(a, 300.0)
        } else {
            let a = if self.mode == 2 { self.amount_shaped } else { self.amount };
            let a = if a < 0.0 { 0.0 } else { a };
            let a = if a > 1.0 { 1.0 } else { a };
            pitch_hz(a.mul_add(55.0, 65.0))
        };
        self.target[LPF] = if hz > 10000.0 { 10000.0 } else { hz };
        self.touch();
    }

    fn update_top(&mut self) {
        let top = self.bend + self.base + 16.0;
        self.target[TOP] = top;
        self.target[HOLD] = f32::from_bits(0x3f51_eb85) / pitch_hz(top);
    }

    fn update_color(&mut self) {
        self.target[COLOR] = (self.note as f32 + -60.0).mul_add(0.5, self.color);
    }

    /// `setParameter(i, v)` (0x10192d980): 0 tune, 1 decay, 2 bend, 3 time, 4 mode,
    /// 5 amount, 6 color.
    pub fn set(&mut self, i: usize, v: f32) {
        if i > 6 {
            return;
        }
        self.raw[i] = v;
        let c = v.clamp(0.0, 1.0);
        match i {
            0 => {
                self.tune = c.mul_add(24.0, 19.0);
                self.base = self.tune + (self.note as f32 + -60.0);
                self.target[BASE] = self.base;
                self.update_top();
                self.touch();
                self.calc_lpf();
                self.update_color();
                self.touch();
            }
            1 => {
                self.target[DECAY] = log_time(c.mul_add(32.0, 33.0));
                self.touch();
            }
            2 => {
                self.bend = c.mul_add(40.0, 0.0);
                self.target[BEND] = self.bend;
                self.base = self.tune + (self.note as f32 + -60.0);
                self.target[BASE] = self.base;
                self.update_top();
                self.touch();
                self.calc_lpf();
                self.update_color();
                self.touch();
            }
            3 => {
                self.target[TIME] = log_time(c.mul_add(28.0, 32.0));
                self.touch();
            }
            4 => self.mode_set = v as u32,
            5 => {
                self.amount = v;
                self.amount_shaped = (2.0 - v.abs()) * v;
                self.target[AMOUNT] = self.amount_shaped;
                self.touch();
                self.calc_lpf();
                self.target[ATTACK] = match self.mode {
                    2 => self.amount,
                    1 => self.amount * f32::from_bits(0x3f4c_cccd),
                    _ => 0.0,
                };
                self.touch();
            }
            _ => {
                self.color = c.mul_add(35.0, 75.0);
                self.base = self.tune + (self.note as f32 + -60.0);
                self.target[BASE] = self.base;
                self.update_top();
                self.touch();
                self.calc_lpf();
                self.update_color();
                self.target[NOISE] = if self.mode == 0 {
                    0.0
                } else {
                    let r = self.raw[6].clamp(0.0, 1.0);
                    r.mul_add(48.0, -24.0)
                };
                self.touch();
            }
        }
    }

    /// Whether anything still sounds (`processAudio`'s test, 0x101931748).
    pub fn active(&self) -> bool {
        if self.amp_env.stage == 0 && self.fol_out <= self.fol_thr {
            let idle = if self.fol_active {
                false
            } else if self.mode != 0 {
                self.noise_env.stage == 0
            } else {
                true
            };
            if idle {
                return false;
            }
        }
        self.gate.state != 0
    }

    /// `setNoteOn(velocity, note)` (0x101930d0c).
    pub fn note_on(&mut self, vel: f32, note: u32) {
        if !(0.0..=1.0).contains(&vel) {
            self.vel = 0.0;
            return;
        }
        let silent = !(self.amp_env.stage != 0 || self.fol_thr < self.fol_out || self.fol_active || {
            let m = if self.mode != 0 { self.noise_env.stage } else { 0 };
            m != 0
        });
        if silent || self.gate.state == 0 {
            self.cur = self.target;
        }
        self.vel = vel;
        self.note = note;
        self.played = true;
        self.mode = self.mode_set as i32;
        self.base = (note as f32 + -60.0) + self.tune;
        self.target[BASE] = self.base;
        self.update_top();
        self.touch();
        self.calc_lpf();
        let col = (self.note as f32 + -60.0).mul_add(0.5, self.color);
        self.target[COLOR] = col;
        let att = match self.mode {
            2 => self.amount,
            1 => self.amount * f32::from_bits(0x3f4c_cccd),
            _ => 0.0,
        };
        self.target[ATTACK] = att;
        let noise = if self.mode == 0 {
            0.0
        } else {
            let r = self.raw[6].clamp(0.0, 1.0);
            r.mul_add(48.0, -24.0)
        };
        self.target[NOISE] = noise;
        self.touch();
        self.cur[LPF] = self.target[LPF];
        self.cur[BASE] = self.target[BASE];
        self.cur[TOP] = self.target[TOP];
        self.cur[HOLD] = self.target[HOLD];
        self.cur[COLOR] = col;
        self.cur[ATTACK] = att;
        self.cur[NOISE] = noise;
        self.phase = if self.mode >= 0 { f32::from_bits(0x3f0c_cccd) } else { f32::from_bits(0x3ee6_6666) };
        self.pd = 1.0;
        self.amp_env.set_decay(self.cur[DECAY]);
        self.amp_env.note_on(vel);
        let hold = self.cur[HOLD];
        self.pitch_env.set_release(self.cur[TIME]);
        self.pitch_env.set_decay(hold);
        self.pitch_env.note_on(vel);
        self.click.hold = hold;
        self.click.state = 1;
        self.click.v = 0.0;
        self.noise_env.note_on(vel);
        // the anticlick crossfade from the last output
        self.xf_freq = self.freq;
        self.xf_rate = self.freq * self.xf_inv_sr * self.xf_k;
        self.xf_pos = 1.0;
        self.xf_acc = 0.0;
        self.xf_step = self.xf_out - self.xf_prev;
        self.xf_start = self.xf_now;
        self.fol_v = 1.0;
        self.fol_held = self.fol_out;
        if !self.fol_on {
            self.fol_coef = self.follower_coef(self.fol_decay);
        }
        self.fol_active = true;
        self.fade_n = 0;
        self.osc_start = vel.mul_add(f32::from_bits(0x3f33_3333), self.osc_last);
        self.hp_click.hold = f32::from_bits(0x3e2e_147b) / pitch_hz(self.top);
        self.hp_click.state = 1;
        self.hp_click.v = 0.0;
        self.gate.state = 1;
        self.gate.v = 0.0;
        self.first = true;
    }

    /// The follower's per-sample factor for a decay time: `min(2^(−3/(sr·t)), 0.99999)`.
    fn follower_coef(&self, t: f32) -> f32 {
        let lim = self.fol_inv_sr * f32::from_bits(0x4a00_2c80);
        let t = if t < 1e-4 { 1e-4 } else { t };
        let t = if lim < t { lim } else { t };
        let c = exp2_poly((self.fol_inv_sr * -3.0) / t);
        let m = f32::from_bits(0x3f7f_ff58);
        if c > m { m } else { c }
    }

    /// `onChoke()`: fade out in 5 ms.
    pub fn choke(&mut self) {
        if self.gate.state == 1 {
            self.gate.state = 2;
            self.gate.v = 1.0;
        }
    }

    /// `processControlRate()` (0x1019316bc).
    pub fn control(&mut self) {
        if !self.dirty {
            self.left -= 1;
            if self.left == 0 {
                self.cur = self.target;
                self.step = [0.0; N];
                self.smoothing = false;
                self.smooth_state = 4;
            } else if self.smooth_state == 2 {
                self.smooth_state = 3;
            } else if self.smooth_state == 4 {
                self.smooth_state = 0;
            }
            return;
        }
        self.dirty = false;
        self.smooth_state = 2;
        let mut sum = [0f32; N];
        for (i, s) in sum.iter_mut().enumerate() {
            let d = (self.target[i] - self.cur[i]) * self.ramp_inv;
            self.step[i] = d;
            let q = d * d;
            *s = if q < 1e-15 {
                self.step[i] = 0.0;
                self.cur[i] = self.target[i];
                0.0
            } else {
                q
            };
        }
        let a = sum[0] + sum[1] + sum[2] + sum[3];
        let b = sum[4] + sum[5] + sum[6] + sum[7];
        let t = a + b + sum[8] + sum[9] + sum[10] + sum[11];
        if t > 0.0 {
            self.left = self.ramp;
            self.smoothing = true;
        } else {
            self.smooth_state = 4;
        }
    }

    /// Render `l.len()` samples (mono into both). Returns false when it rendered nothing
    /// (idle).
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) -> bool {
        if !self.active() {
            if self.played {
                self.idle_reset();
            }
            return false;
        }
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            let y = self.sample();
            *a = y;
            *b = y;
        }
        true
    }

    /// What `processAudio` resets once the sound has ended.
    fn idle_reset(&mut self) {
        self.vel = 0.0;
        self.played = false;
        self.sine = 0.0;
        self.osc_last = 0.0;
        self.osc_start = 0.0;
        self.phase = 0.0;
        self.pd = 0.0;
        for e in [&mut self.amp_env, &mut self.pitch_env, &mut self.noise_env] {
            e.stage = 0;
            e.out = 0.0;
            e.x = 1.0;
            e.t = 0.0;
        }
        self.click.state = 0;
        self.click.v = 0.0;
        self.hp_click.state = 0;
        self.hp_click.v = 0.0;
        self.xf_pos = 1.0;
        self.xf_acc = 0.0;
        self.xf_now = 0.0;
        self.xf_prev = 0.0;
        self.xf_out = 0.0;
        self.xf_step = 0.0;
        self.xf_start = 0.0;
        self.fol_out = 0.0;
        self.fol_held = 0.0;
        self.fol_v = 0.0;
        self.fol_active = false;
        self.hp_z = 0.0;
        self.hp_out = 0.0;
        self.svf_z1 = 0.0;
        self.svf_z2 = 0.0;
        self.lp_z = 0.0;
        self.gate.state = 0;
        self.gate.v = 0.0;
        self.fade_n = 0;
        self.cur = self.target;
        self.dirty = true;
        self.smooth_state = 1;
        self.first = true;
    }

    /// One sample (`processAudioImpl<smoothing, first>`).
    #[inline]
    fn sample(&mut self) -> f32 {
        let first = self.first;
        self.first = false;
        if self.smoothing {
            for i in 0..N {
                self.cur[i] += self.step[i];
            }
        }
        // Pitch-envelope release (Time) follows the smoothed value.
        if first || (self.smoothing && self.step[TIME] != 0.0) {
            self.pitch_env.set_release(self.cur[TIME]);
        }
        let bend = self.cur[BEND];
        let base = self.cur[BASE];
        let pe = self.pitch_env.process();
        let ck = self.click.process();
        let p = bend * pe + ck.mul_add(16.0, base);
        let f = exp_e(p.mul_add(5.0, f32::from_bits(0x44ac_bc36)));
        let inc = (f * self.inv_sr).clamp(-1.0, 1.0);
        self.freq = f;
        self.top = self.cur[TOP];
        // The click's high-pass.
        let h = self.hp_click.process();
        let y = self.hp_a * (-self.hp_b).mul_add(self.hp_z, h);
        let hp = y - self.hp_z;
        self.hp_out = hp;
        self.hp_z = y;
        // Phase distortion.
        let v = self.pd_coef * self.pd;
        self.pd = v;
        let d = if v < self.pd_floor {
            self.pd = 0.0;
            0.0
        } else {
            v - self.pd_floor
        };
        let pd = ((self.sine + f32::from_bits(0xbe4c_cccd)) * f32::from_bits(0xbca3_d70a)).mul_add(d, 0.0);
        // The oscillator: a triangle from the phase, then a sine polynomial.
        let ph = inc + self.phase + 10.0;
        let ph = ph - ((ph + 0.5) as i32 as f32);
        self.phase = ph;
        let t = pd + ph + 10.0;
        let t = t - ((t + 0.5) as i32 as f32);
        let tri = (t + t).abs() + -0.5;
        let pi = std::f32::consts::PI;
        let x = tri * pi * f32::from_bits(0x3e22_f981);
        let h = if x < 0.0 { -0.5f64 } else { 0.5 };
        let n = (h + x as f64) as i32;
        let x = (x - n as f32 + (x - n as f32)) * pi;
        let x2 = x * x;
        let q = x2.mul_add(f32::from_bits(0xb2cd_4ca8), f32::from_bits(0x3638_b94f));
        let q = q.mul_add(x2, f32::from_bits(0xb950_0c03));
        let q = q.mul_add(x2, f32::from_bits(0x3c08_8885));
        let q = q.mul_add(x2, f32::from_bits(0xbe2a_aac1));
        let sine = x * q.mul_add(x2, 1.0);
        self.sine = sine;
        // The amplitude follower (decay smoothed).
        if first || (self.smoothing && self.step[DECAY] != 0.0) {
            let dv = self.cur[DECAY];
            if self.fol_decay != dv {
                self.fol_decay = dv;
                if self.fol_on {
                    self.fol_coef = self.follower_coef(dv);
                }
            }
            self.amp_env.set_decay(dv);
        }
        let a = self.vel * self.amp_env.process();
        let held = self.fol_v * self.fol_held;
        self.fol_v *= self.fol_coef;
        let m = if held < a { a } else { held };
        let mix = self.fol_mix.clamp(0.0, 1.0);
        let lvl = ((a + held) - m).mul_add(mix, m);
        let lvl = if lvl < 0.0 { 0.0 } else { lvl };
        self.fol_out = lvl;
        let fade = if lvl < self.fol_thr {
            let n = if self.fade_n < self.fade_len {
                self.fade_n + 1
            } else {
                self.fol_active = false;
                self.fade_len
            };
            self.fade_n = n;
            let g = (n as f32).mul_add(-self.fade_inv, 1.0);
            g * (g * g)
        } else {
            1.0
        };
        let mut o = sine * (lvl * fade);
        self.xf_prev = self.xf_out;
        self.xf_now = o;
        if self.xf_pos > 0.0 {
            let p = self.xf_pos - self.xf_rate;
            self.xf_pos = p;
            let s = p + -0.5;
            let s = (-s).mul_add(s.abs(), s);
            let s = s.mul_add(2.0, 0.5);
            let acc = self.xf_acc;
            self.xf_acc = acc + self.xf_step;
            o += (self.xf_start + acc) * s;
        }
        self.xf_out = o;
        self.osc_last = o;
        // Crossfade from the note's start level into the oscillator by the click.
        let k = hp.clamp(0.0, 1.0);
        let o = (self.osc_start - o).mul_add(k, o) * f32::from_bits(0x3ea3_d70a);
        // Output low-pass (one-pole TPT).
        if first || (self.smoothing && self.step[LPF] != 0.0) {
            let c = self.cur[LPF];
            if self.lp_hz != c {
                self.set_lp(c);
            }
        }
        let y = self.lp_a * (-self.lp_b).mul_add(self.lp_z, o);
        let lp = self.lp_g * (self.lp_z + y);
        self.lp_z = y;
        // The noise click.
        let att = self.cur[ATTACK];
        if first || (self.smoothing && self.step[ATTACK] != 0.0) {
            let a2 = (att + -0.5) + (att + -0.5);
            let a2 = if a2 < 0.0 { 0.0 } else { a2 };
            let a2 = if a2 > 1.0 { 1.0 } else { a2 };
            self.noise_env.set_decay(log_time(a2.mul_add(10.0, 15.0)));
        }
        self.seed = self.seed.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        let nz = att * (self.noise_scale * self.seed as f32);
        let nz = nz * self.noise_env.process();
        if first || (self.smoothing && self.step[NOISE] != 0.0) {
            let hz = pitch_hz(((self.cur[BASE] + -50.0).mul_add(f32::from_bits(0x3f19_999a), 105.0)) + self.cur[NOISE]);
            if self.svf_hz != hz {
                self.set_svf(hz);
            }
        }
        let g = self.svf_g;
        let k = self.svf_k;
        let z1 = self.svf_z1;
        let z2 = self.svf_z2;
        let s4 = z1 + z1;
        let s6 = z2.mul_add(2.0, g * s4);
        let gg = g.mul_add(k, g * g);
        let s4 = s4.mul_add(k, s6);
        let hp_n = nz - self.svf_d * nz.mul_add(gg, s4);
        let v1 = g.mul_add(hp_n, z1);
        let bp = z1 + v1;
        let v2 = g.mul_add(bp, z2);
        self.svf_z1 = v1;
        self.svf_z2 = v2;
        let c = bp.clamp(-self.clip, self.clip);
        let gate = match self.gate.state {
            0 => 0.0,
            1 => 1.0,
            _ => self.gate.process(),
        };
        let s = (c.abs() * -0.25).mul_add(self.sat, 1.0);
        let out = lp + self.vel * (c * s);
        out * f32::from_bits(0x4006_767d) * gate
    }
}
