//! `NI::DSP::ClapCore` (docs/synth-snare.md): the Snare's Pow (core mode 0) and Clap
//! (core mode 1) engines.
//!
//! Pow: LCG noise through a resonant band-pass whose pitch follows a bend envelope, a
//! `7000·t` low-pass, then two paths: a `500·t` band-pass gated by envelope A, and a
//! soft-clipped tone low-pass opened by a short hit envelope; a low shelf at the
//! smoothed tune, a 100 Hz high-pass and an `atan` saturator.
//! Clap: the same band-pass noise (gated by envelope A) plus a second noise layer
//! (envelope B, `6300·t` band-pass); the hit envelope is retriggered three times (the
//! "hands") and the sum passes a bell, the low shelf and a second bell.
//!
//! Everything follows the assembly operation by operation (`processAudioImpl<clap,
//! ramp, first>`). The first block after a note-on and the blocks while parameters
//! ramp recompute the filter/envelope coefficients that depend on smoothed parameters;
//! the steady variant skips those, keeping the cached coefficients (`full` below).

mod dsp;

use super::Core;
use crate::fx::exp_e;
use dsp::{BiQuad, FreeEnv, HitEnv, OnePole, Slew, Svf};

const fn f(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// `E(x·5 + 1381.8816)`: MIDI note → Hz.
#[inline]
fn mtof(st: f32) -> f32 {
    exp_e(st.mul_add(5.0, f(0x44ac_bc36)))
}

/// `E(x·9.965784 + 602.0529)` = `2^(x/6.02 − 9.966)`: the cores' log time scale (s).
#[inline]
fn etime(x: f32) -> f32 {
    exp_e(x.mul_add(f(0x411f_73da), f(0x4416_8363)))
}

#[inline]
fn clamp01(x: f32) -> f32 {
    let x = if x < 0.0 { 0.0 } else { x };
    if x > 1.0 { 1.0 } else { x }
}

/// Smoothed parameter lanes (`AB::SmoothingToolbox<13>` at +0x640: current +0x650,
/// increment +0x690, target +0x6d0).
const P_RATE: usize = 0; // hand spacing (s)
const P_HIT: usize = 1; // hit-envelope half time (s)
const P_LAST: usize = 2; // last hand's gain
const P_DECAY: usize = 3; // envelope decay (s)
const P_BLEVEL: usize = 4; // noise-layer level
const P_CLEVEL: usize = 5; // band-pass level
const P_SHARP: usize = 6; // hit low-pass cutoff (Hz)
const P_CLIP: usize = 7; // soft-clip threshold
const P_LPMUL: usize = 8; // tone low-pass cutoff factor
const P_BEND: usize = 9; // bend depth (semitones)
const P_TUNE: usize = 10; // tune factor t (Hz/1000-ish)
const P_PITCH: usize = 11; // noise band-pass pitch (MIDI note)
const P_GAIN: usize = 12; // level compensation

pub(crate) struct Clap {
    /// +0x80: the parameters as set.
    raw: [f32; 7],
    /// +0x9c tune, +0xa4 decay (idx 2), +0xa8 room, +0xac bend, +0xb0 decay (idx 5),
    /// +0xb4 focus, +0xb8 envelope A's attack.
    tune: f32,
    decay: f32,
    room: f32,
    bend: f32,
    decay_pow: f32,
    focus: f32,
    att_a: f32,
    /// +0xa0 the mode asked for, +0xa1 the one that plays (switched at note-on).
    mode_req: bool,
    clap: bool,
    /// +0xbc last note.
    note: u32,
    /// +0xc0…+0xdc: the three hands' phases, increments, fired flags; the hit gain.
    ph: [f32; 3],
    ph_inc: [f32; 3],
    fired: [bool; 3],
    hit_gain: f32,
    /// LCG noise (`x·1103515245 + 12345`): +0xe0 the clap layer, +0x118 the main one.
    seed2: u32,
    scale2: f32,
    seed1: u32,
    scale1: f32,
    /// +0x150, +0x180, +0x1b0, +0x1e0.
    svf: [Svf; 4],
    /// +0x210 tone low-pass, +0x230 100 Hz high-pass, +0x250 hit low-pass, +0x534 the
    /// 20 Hz tune smoother.
    lp_tone: OnePole,
    hp: OnePole,
    lp_hit: OnePole,
    lp_tune: OnePole,
    /// +0x270 bell, +0x2a8 low shelf, +0x2e0 bell.
    bq: [BiQuad; 3],
    /// +0x318, +0x410.
    env_a: FreeEnv,
    env_b: FreeEnv,
    /// +0x508.
    hit: HitEnv,
    /// +0x554 bend, +0x588 noise layer, +0x5bc band-pass, +0x5f0 hit.
    slew: [Slew; 4],
    /// +0x624: 0 off, 1 on, 2 choke fade; +0x630 fade gain; +0x634 its factor (5 ms).
    state: u32,
    fade: f32,
    fade_coef: f32,
    cur: [f32; 16],
    inc: [f32; 16],
    tgt: [f32; 16],
    /// +0x710 dirty, +0x711 ramping, +0x714 counter, +0x718 length, +0x71c 1/(n·128).
    dirty: bool,
    ramp: bool,
    counter: i32,
    n_ctrl: i32,
    k_ctrl: f32,
    /// +0x730 velocity, +0x734 a note played since the last reset.
    vel: f32,
    noted: bool,
    /// The next block runs a `first` variant.
    first: bool,
    /// A note was played since construction (see `note_on`).
    loaded: bool,
    sr: f32,
}

impl Clap {
    pub(crate) fn new(sr: f32) -> Clap {
        let mut svf = [Svf::new(2.0, 0.0); 4];
        svf[0] = Svf::new(0.5, 0.75);
        let mut c = Clap {
            raw: [0.0; 7],
            tune: 0.0,
            decay: 0.0,
            room: 0.0,
            bend: 0.0,
            decay_pow: 0.0,
            focus: 0.0,
            att_a: 0.0,
            mode_req: false,
            clap: false,
            note: 60,
            ph: [0.0; 3],
            ph_inc: [0.0; 3],
            fired: [false; 3],
            hit_gain: 0.0,
            seed2: 0,
            scale2: 0.0,
            seed1: 1,
            scale1: 0.0,
            svf,
            lp_tone: OnePole::new(1.0),
            hp: OnePole::new(100.0),
            lp_hit: OnePole::new(1.0),
            lp_tune: OnePole::new(20.0),
            bq: [BiQuad::new(false), BiQuad::new(true), BiQuad::new(false)],
            env_a: FreeEnv::new(false, 0.5, 0.4),
            env_b: FreeEnv::new(true, 0.1, 0.0),
            hit: HitEnv::new(),
            slew: [Slew::new(1.0), Slew::new(1.0), Slew::new(0.0), Slew::new(0.0)],
            state: 0,
            fade: 0.0,
            fade_coef: 0.0,
            cur: [0.0; 16],
            inc: [0.0; 16],
            tgt: [0.0; 16],
            dirty: true,
            ramp: false,
            counter: 0,
            n_ctrl: 0,
            k_ctrl: 0.0,
            vel: 0.0,
            noted: false,
            first: true,
            loaded: false,
            sr,
        };
        c.set_sample_rate(sr, 128);
        c.reset();
        c
    }

    /// `onSetSampleRate(sr, 128)`.
    fn set_sample_rate(&mut self, sr: f32, block: i32) {
        self.sr = sr;
        let n = ((sr * f(0x3ca3_d70a)) / block as f32 + 0.5) as i32;
        self.n_ctrl = if n > 1 { n } else { 1 };
        self.k_ctrl = 1.0 / (self.n_ctrl * block) as f32;
        self.scale2 = f(0x3000_009d);
        self.scale1 = f(0x3000_009d);
        let inv = 1.0 / sr;
        for s in &mut self.svf {
            s.sr = sr;
            s.inv_sr = inv;
            let fc = s.fc;
            s.fc = f32::NAN;
            s.follow(fc);
            s.reset();
        }
        for p in [&mut self.lp_tone, &mut self.lp_hit, &mut self.lp_tune] {
            p.inv_sr = inv;
            p.coef(p.fc);
            p.z = 0.0;
            p.out = 0.0;
        }
        self.hp.inv_sr = inv;
        self.hp.coef(100.0);
        self.hp.z = 0.0;
        self.hp.out = 0.0;
        let qs = [(f(0x3dcc_cccd), 4.0), (0.0, 10.0), (f(0x3e4c_cccd), 4.0)];
        for (b, (q, g)) in self.bq.iter_mut().zip(qs) {
            b.sr = sr;
            b.inv_sr = inv;
            b.set_fc(b.fc);
            b.z1 = 0.0;
            b.z2 = 0.0;
            b.q = q;
            b.calc();
            b.gain = g;
            b.calc();
        }
        self.env_a.set_samplerate(sr);
        self.env_b.set_samplerate(sr);
        self.hit.inv_sr = inv;
        self.hit.phase = 0.0;
        self.hit.attack = true;
        self.hit.lvl = 0.0;
        self.hit.calc();
        self.hit.rate = inv * 10000.0;
        let n = (sr * f(0x3c75_c290)) as u32;
        let n = if n > 1 { n } else { 1 };
        for s in &mut self.slew {
            s.inv_sr = inv;
            s.n = n;
            s.inv_n = 1.0 / n as f32;
            s.reset();
        }
        self.state = 0;
        self.fade = 0.0;
        let e = ((inv * f(0xbf31_7215)) / f(0x3ba3_d70a)).exp();
        self.fade_coef = if e > 1.0 { 1.0 } else { e };
    }

    /// The voice-state part of `onReset()` (also what a mode change at note-on clears).
    fn clear_voice(&mut self) {
        for s in &mut self.svf {
            s.reset();
        }
        for p in [&mut self.lp_tone, &mut self.hp, &mut self.lp_hit] {
            p.z = 0.0;
            p.out = 0.0;
        }
        for b in &mut self.bq {
            b.z1 = 0.0;
            b.z2 = 0.0;
        }
        self.env_a.reset();
        self.env_b.reset();
        self.hit.phase = 0.0;
        self.hit.attack = true;
        self.hit.lvl = 0.0;
        for s in &mut self.slew {
            s.reset();
        }
        self.first = true;
    }

    /// `onReset()`, and `processAudio`'s reset once the sound has ended.
    fn reset(&mut self) {
        self.vel = 0.0;
        self.noted = false;
        self.ph = [0.0; 3];
        self.fired = [false; 3];
        self.cur = self.tgt;
        self.dirty = true;
        self.clear_voice();
        self.state = 0;
        self.fade = 0.0;
    }

    /// `calculateTuneParams()`: lanes 10–12 from Tune and the note.
    fn calc_tune(&mut self) {
        let t = clamp01(self.tune);
        let (hi, lo, range): (f32, f32, f32) = if self.clap { (18.0, -18.0, 36.0) } else { (24.0, -36.0, 60.0) };
        let st = (self.note as f32 + -60.0) + range.mul_add(t, lo);
        let st = if st < lo { lo } else { st };
        let st = if st > hi { hi } else { st };
        self.tgt[P_TUNE] = mtof(st) * f(0x3dfa_7222);
        self.tgt[P_PITCH] = st + 82.0;
        self.tgt[P_GAIN] = 126.0 / mtof(st + 82.0).sqrt() + -0.5;
        self.dirty = true;
    }

    /// `calculateClapParams()`: lanes 0–2.
    fn calc_clap(&mut self) {
        let d = if self.clap { self.decay } else { 0.5 };
        let (rate, r) = if self.clap && d > 0.5 {
            let u = (d + -0.5) + (d + -0.5);
            let s = clamp01(u).mul_add(7.0, 23.0);
            (etime(s), etime((s + -23.0) * f(0x3fe6_6666)))
        } else {
            let s = clamp01(d + d).mul_add(11.0, 12.0);
            (etime(s), etime((s + -23.0) * 0.5))
        };
        self.tgt[P_RATE] = rate;
        let m = if self.clap {
            let u = (self.focus + -0.5) + (self.focus + -0.5);
            clamp01(u).mul_add(f(0xbf33_3333), 1.0) * 4.0
        } else {
            12.0
        };
        self.tgt[P_HIT] = r * m;
        self.tgt[P_LAST] = clamp01(d + d).mul_add(f(0x3f33_3333), 0.25);
        self.dirty = true;
    }

    /// `calculateRoomDecay()`: lane 3.
    fn calc_room_decay(&mut self) {
        if !self.clap {
            self.tgt[P_DECAY] = etime(clamp01(self.decay_pow).mul_add(28.0, 42.0));
        } else {
            let d = self.decay;
            let t = if d > 0.5 {
                let u = (d + -0.5) + (d + -0.5);
                etime(clamp01(u).mul_add(3.5, f(0x425e_0000)))
            } else {
                clamp01(d + d).mul_add(f(0x3ecc_ccce), f(0x3e4c_cccd))
            };
            let r = clamp01((self.room + f(0xbe99_999a)) / f(0x3f33_3333));
            self.tgt[P_DECAY] = t * r.mul_add(f(0x3f4c_cccc), 0.5);
        }
        self.dirty = true;
    }

    /// `calculateSharpParams()`: lanes 6–8 and envelope A's attack (+0xb8).
    fn calc_sharp(&mut self) {
        let fo = self.focus;
        let (st, lp) = if !self.clap {
            let c = clamp01(fo);
            self.tgt[P_CLIP] = 0.5;
            self.att_a = c.mul_add(f(0xbdb8_51ec), f(0x3dcc_cccd));
            let lp = mtof(c.mul_add(20.0, 100.0));
            (c.mul_add(-60.0, 40.0), if lp > 6000.0 { 6000.0 } else { lp })
        } else {
            let st =
                if fo > 0.5 { clamp01((fo + -0.5) / f(0x3e99_999a)).mul_add(-13.0, 8.0) } else { clamp01(fo + fo).mul_add(-32.0, 40.0) };
            self.tgt[P_CLIP] = clamp01(fo + fo).mul_add(f(0x3ecc_ccce), f(0x3e4c_cccd));
            self.att_a = f(0x3c23_d70a);
            (st, mtof(clamp01(fo).mul_add(20.0, 100.0)))
        };
        self.tgt[P_LPMUL] = lp;
        self.tgt[P_SHARP] = exp_e(st.mul_add(f(0xc11f_73da), f(0x44e0_be4e)));
        self.dirty = true;
    }

    /// Lane 4, inlined in setParameter 2, 3, 6.
    fn calc_level(&mut self) {
        let a = if self.clap { self.decay + f(0xbe4c_cccd) } else { f(0x3e99_999a) };
        let b = clamp01((self.room + f(0xbf33_3333)) / f(0x3e99_999a));
        let a = clamp01(a / f(0x3ee6_6666));
        let s = clamp01((self.focus + f(0xbdcc_cccd)) / f(0x3e99_999a));
        self.tgt[P_BLEVEL] = (a * b) * s.mul_add(0.5, 0.5);
    }

    /// Lane 5 (setParameter 3 and the mode change).
    fn calc_clevel(&mut self) {
        self.tgt[P_CLEVEL] = if self.clap {
            let x = self.room / f(0x3e99_999a);
            let x = if x < 0.0 { 0.0 } else { x };
            if x > 1.0 { 0.5 } else { x * 0.5 }
        } else {
            f(0x3f19_999a)
        };
    }

    /// Lane 9 (bend, Pow only).
    fn calc_bend(&mut self) {
        self.tgt[P_BEND] = if self.clap {
            0.0
        } else {
            let x = (self.bend + -0.5) + (self.bend + -0.5);
            x * x.abs().mul_add(f(0x3f33_3333), f(0x3e99_999a)) * f(0xc302_0000)
        };
        self.dirty = true;
    }

    /// `hasOutputSignal()` without the state test.
    fn alive(&self) -> bool {
        let b = self.env_b.stage != 0 || self.slew[1].out > dsp::THR || self.slew[1].active;
        if self.env_a.stage == 0 && self.slew[2].out <= dsp::THR {
            return self.slew[2].active || (self.clap && b);
        }
        true
    }

    /// One `processAudioImpl` block.
    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let full = self.first || self.ramp;
        let k = if self.clap { f(0x3f00_635a) } else { f(0x3f91_a8b5) };
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            if self.ramp {
                for i in 0..16 {
                    self.cur[i] += self.inc[i];
                }
            }
            let y = if self.clap { self.tick_clap(full) } else { self.tick_pow(full) };
            let g = match self.state {
                0 => 0.0,
                1 => 1.0,
                _ => {
                    let g = self.fade_coef * self.fade;
                    self.fade = g;
                    if (g as f64) < 1e-4 {
                        self.fade = 0.0;
                        self.state = 0;
                        0.0
                    } else {
                        g
                    }
                }
            };
            let o = y * k * g;
            *a = o;
            *b = o;
        }
        self.first = false;
    }

    /// Shared head: tune smoother, envelope A, the bend slew, the main noise band-pass.
    /// Returns (smoothed tune, envelope A).
    #[inline]
    fn head(&mut self, full: bool) -> (f32, f32) {
        let p = self.cur;
        let tune_lp = self.lp_tune.lp(p[P_TUNE]);
        if full {
            self.env_a.set_decay(p[P_DECAY]);
        }
        let ea = self.env_a.process();
        let b = p[P_BEND];
        let ab = if b > 0.0 { b } else { -b };
        self.slew[0].set_time(p[P_DECAY] * if b > 0.0 { f(0x3d4c_cccd) } else { f(0x3cf5_c28f) });
        let (o, fd) = self.slew[0].tick(ea * ab);
        let m = if b > 0.0 { o * fd } else { -(o * fd) };
        let hz = mtof(p[P_PITCH] + b.mul_add(f(0xbf66_6666), m));
        self.seed1 = self.seed1.wrapping_mul(1_103_515_245).wrapping_add(12345);
        self.svf[0].follow(hz);
        let x = (self.scale1 * self.seed1 as i32 as f32) * 0.5;
        self.svf[0].tick(x);
        (tune_lp, ea)
    }

    /// Clamp to ±2c (c = max(clip, 2e-12)).
    #[inline]
    fn clip(&self, v: f32) -> (f32, f32) {
        let c = self.cur[P_CLIP];
        let c = if c < f(0x2c0c_bccc) { f(0x2c0c_bccc) } else { c };
        let lim = c + c;
        let v = if v < -lim { -lim } else { v };
        (if v > lim { lim } else { v }, c)
    }

    /// The hit path: `v·(1 − |v|/(4c))` times the low-passed hit envelope.
    #[inline]
    fn hit_path(&mut self, full: bool, clipped: f32, c: f32, hin: f32) -> f32 {
        let (o, fd) = self.slew[3].tick(hin);
        if full {
            self.lp_hit.follow(self.cur[P_SHARP]);
        }
        let shaped = clipped * ((clipped.abs() * -0.25) / c + 1.0);
        let lp = self.lp_hit.lp(o * fd);
        shaped * lp
    }

    fn tick_pow(&mut self, full: bool) -> f32 {
        let (tune_lp, ea) = self.head(full);
        let p = self.cur;
        if full {
            self.svf[1].follow(p[P_TUNE] * f(0x45da_c000));
        }
        let bp1 = self.svf[0].bp;
        self.svf[1].tick(bp1);
        let lp2 = self.svf[1].lp;
        self.slew[2].set_time(p[P_DECAY] * f(0x3e99_999a));
        let (co, cf) = self.slew[2].tick(ea * self.vel * p[P_CLEVEL]);
        if full {
            self.svf[2].follow(p[P_TUNE] * 500.0);
        }
        let x2 = p[P_GAIN] * lp2;
        self.svf[2].tick(x2 * (co * cf));
        if full {
            self.lp_tone.follow(p[P_TUNE] * p[P_LPMUL]);
        }
        let tone = self.lp_tone.lp(x2);
        let (clipped, c) = self.clip(tone);
        let h = self.hit.tick();
        let hin = self.vel * f(0x3f19_999a) * h;
        let hp = self.hit_path(full, clipped, c, hin);
        let x = self.svf[2].bp + (hp + 0.0);
        self.bq[1].set_fc(tune_lp * f(0x4522_8000));
        let y = self.bq[1].tick(x);
        atan(self.hp.hp(y))
    }

    /// A hand: retrigger the hit envelope with half time `t`.
    #[inline]
    fn hand(&mut self, full: bool, t: f32) {
        if full {
            self.hit.set_time(t);
        }
        self.hit.trigger();
        if full {
            self.slew[3].set_time(t * 3.0);
        }
    }

    fn tick_clap(&mut self, full: bool) -> f32 {
        let (tune_lp, ea) = self.head(full);
        let p = self.cur;
        let bp1 = self.svf[0].bp;
        self.seed2 = self.seed2.wrapping_mul(1_103_515_245).wrapping_add(12345);
        if full {
            self.env_b.set_decay(p[P_DECAY] * f(0x3f19_999a));
            self.slew[1].set_time(p[P_DECAY] * f(0x3e19_999a));
        }
        let eb = self.env_b.process();
        let (bo, bf) = self.slew[1].tick(eb * self.vel * p[P_BLEVEL]);
        if full {
            self.svf[3].follow(p[P_TUNE] * f(0x45c4_e000));
        }
        let n2 = (self.scale2 * self.seed2 as i32 as f32) * f(0x3bc4_9ba6);
        self.svf[3].tick(n2 * (bo * bf));
        let bp4 = self.svf[3].bp;
        self.slew[2].set_time(p[P_DECAY] * f(0x3e99_999a));
        let (co, cf) = self.slew[2].tick(ea * self.vel * p[P_CLEVEL]);
        if full {
            self.svf[2].follow(p[P_TUNE] * 500.0);
        }
        let x2 = bp1 * p[P_GAIN];
        self.svf[2].tick(x2 * (co * cf));
        let (clipped, c) = self.clip(x2);
        let t = p[P_HIT];
        for i in 0..3 {
            if self.fired[i] {
                continue;
            }
            if self.ph[i] >= 1.0 {
                if i == 2 {
                    self.hand(full, t * f(0x3fec_cccd));
                    self.hit_gain = p[P_LAST];
                } else {
                    self.hand(full, t);
                    self.hit_gain = 1.0;
                }
                self.fired[i] = true;
            } else {
                self.ph[i] += self.ph_inc[i];
            }
        }
        let h = self.hit.tick();
        let hin = self.vel * self.hit_gain * h;
        let hp = self.hit_path(full, clipped, c, hin);
        if full {
            let fc = p[P_TUNE] * f(0x4580_2000);
            self.bq[0].set_fc(if fc > 10000.0 { 10000.0 } else { fc });
        }
        let b0 = self.bq[0].tick(hp);
        let x = self.svf[2].bp + (bp4 + b0);
        self.bq[1].set_fc(tune_lp * f(0x4522_8000));
        let y = self.bq[1].tick(x);
        if full {
            self.bq[2].set_fc(p[P_TUNE] * f(0x44cf_8000));
        }
        let y = self.bq[2].tick(y);
        atan(self.hp.hp(y))
    }
}

/// The cores' `atan` (odd polynomial, reflected outside ±1; no fused ops).
#[inline]
fn atan(v: f32) -> f32 {
    #[inline]
    fn poly(r: f32) -> f32 {
        let r2 = r * r;
        let mut s = r * f(0x3b3b_d75c) + f(0xbc84_6dee);
        for c in [0x3d2f_c1fa, 0xbd9a_316f, 0x3dda_3db4, 0xbe11_7fc7, 0x3e4c_bc06, 0xbeaa_aa5c] {
            s = r2 * s + f(c);
        }
        r * (r2 * s + 1.0)
    }
    if v > 1.0 {
        f(0x3fc9_0ff9) - poly(1.0 / v)
    } else if v < -1.0 {
        f(0xbfc9_0ff9) - poly(1.0 / v)
    } else {
        poly(v)
    }
}

impl Core for Clap {
    fn set_parameter(&mut self, idx: u32, v: f32) {
        if idx > 6 {
            return;
        }
        self.raw[idx as usize] = v;
        match idx {
            0 => {
                self.tune = v;
                self.calc_tune();
            }
            1 => self.mode_req = v > 0.0,
            2 => {
                self.decay = v;
                self.calc_clap();
                self.calc_room_decay();
                self.calc_level();
            }
            3 => {
                self.room = v;
                self.calc_room_decay();
                self.calc_level();
                self.calc_clevel();
            }
            4 => {
                self.bend = v;
                self.calc_bend();
            }
            5 => {
                self.decay_pow = v;
                self.calc_room_decay();
            }
            _ => {
                self.focus = v;
                self.calc_sharp();
                self.calc_clap();
                self.calc_level();
            }
        }
    }

    fn note_on(&mut self, vel: f32, note: u32) {
        if !(0.0..=1.0).contains(&vel) {
            self.vel = 0.0;
            return;
        }
        self.note = note;
        self.tune = self.raw[0];
        self.calc_tune();
        // Snapped at every note-on (6 sharp cutoff and 8 tone cutoff keep ramping).
        for k in [0, 1, 2, 3, 4, 5, 7, 9, 10, 11, 12] {
            self.cur[k] = self.tgt[k];
        }
        // Maschine resets the core after the load-time parameters (`onReset`: current =
        // target); without that hook here, the first note stands in for it.
        if !self.loaded {
            self.loaded = true;
            self.cur = self.tgt;
        }
        if self.clap != self.mode_req {
            self.clap = self.mode_req;
            self.calc_tune();
            self.calc_bend();
            self.calc_sharp();
            self.calc_clap();
            self.calc_room_decay();
            self.calc_level();
            self.calc_clevel();
            self.cur = self.tgt;
            self.svf[0].set_res(if self.clap { f(0x3f19_999a) } else { f(0x3f33_3333) });
            self.clear_voice();
        }
        self.env_a.set_attack(self.att_a);
        self.env_a.note_on(vel);
        self.env_b.set_attack(self.cur[P_DECAY] * f(0x3e4c_cccd));
        self.env_b.note_on(vel);
        if self.clap {
            let r = self.cur[P_RATE];
            let a = r * f(0x3f73_3333);
            let b = r + a;
            let c = r.mul_add(f(0x3f0c_cccd), b);
            self.ph_inc = [1.0 / (a * self.sr), 1.0 / (b * self.sr), 1.0 / (c * self.sr)];
            self.ph = [0.0; 3];
            self.fired = [false; 3];
            self.hit_gain = f(0x3f99_999a);
        }
        self.hit.set_time(self.cur[P_HIT]);
        self.hit.trigger();
        for s in &mut self.slew {
            s.note_on();
        }
        self.slew[3].set_time(self.cur[P_HIT] * 3.0);
        self.state = 1;
        self.fade = 0.0;
        self.vel = vel;
        self.noted = true;
        self.first = true;
    }

    fn note_off(&mut self, _note: u32) {}

    fn choke(&mut self) {
        if self.state == 1 {
            self.state = 2;
            self.fade = 1.0;
        }
    }

    fn leave(&mut self) {
        self.choke();
    }

    fn control_rate(&mut self) {
        if self.dirty {
            self.dirty = false;
            let mut any = false;
            for k in 0..16 {
                let d = (self.tgt[k] - self.cur[k]) * self.k_ctrl;
                if ((d * d) as f64) < 1e-15 {
                    self.inc[k] = 0.0;
                    self.cur[k] = self.tgt[k];
                } else {
                    self.inc[k] = d;
                    any = true;
                }
            }
            if any {
                self.counter = self.n_ctrl;
                self.ramp = true;
            }
            return;
        }
        self.counter = self.counter.wrapping_sub(1);
        if self.counter == 0 {
            self.cur = self.tgt;
            self.inc = [0.0; 16];
            self.ramp = false;
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.alive() && self.state != 0 {
            self.run(l, r);
            return;
        }
        l.fill(0.0);
        r.fill(0.0);
        if self.noted {
            self.reset();
        }
    }

    fn has_output(&self) -> bool {
        self.alive() && self.state != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(clap: bool) -> Clap {
        let mut c = Clap::new(48_000.0);
        let mode = if clap { 1.0 } else { 0.0 };
        for (i, v) in [0.5, mode, 0.5, 0.5, 0.5, 0.5, 0.5].iter().enumerate() {
            c.set_parameter(i as u32, *v);
        }
        c
    }

    fn render(c: &mut Clap, n: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n);
        let (mut l, mut r) = ([0.0; 16], [0.0; 16]);
        for i in (0..n).step_by(16) {
            if i % 128 == 0 {
                c.control_rate();
            }
            c.process(&mut l, &mut r);
            out.extend_from_slice(&l);
        }
        out
    }

    #[test]
    fn renders_and_ends() {
        for clap in [false, true] {
            let mut c = make(clap);
            render(&mut c, 2048);
            c.note_on(0.8, 60);
            let a = render(&mut c, 48_000 * 8);
            assert!(a.iter().all(|x| x.is_finite()));
            assert!(a[..4800].iter().any(|x| x.abs() > 1e-3), "silent (clap={clap})");
            assert!(!c.has_output(), "tail never ends (clap={clap})");
            assert!(render(&mut c, 1024).iter().all(|&x| x == 0.0));
        }
    }

    #[test]
    fn set_mid_note_keeps_voice() {
        let mut c = make(true);
        render(&mut c, 2048);
        c.note_on(1.0, 60);
        render(&mut c, 1024);
        c.set_parameter(3, 0.9);
        c.set_parameter(6, 0.1);
        let a = render(&mut c, 1024);
        assert!(c.has_output() && a.iter().any(|x| x.abs() > 1e-4));
    }

    #[test]
    fn choke_silences() {
        let mut c = make(false);
        render(&mut c, 2048);
        c.note_on(1.0, 60);
        render(&mut c, 256);
        c.choke();
        render(&mut c, 4800);
        assert!(!c.has_output());
    }
}
