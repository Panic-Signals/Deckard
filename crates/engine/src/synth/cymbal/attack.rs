//! `NI::DSP::AttackCore<FluffyOsc on, white noise, MainOscType 1, Saturator s,
//! no anticlick, no phase coupling, no pitch envelope, PhaseOscType p>`: the attack
//! layer of the Cymbal's two engines (docs/synth-cymbal.md §5.2). Crash uses
//! `Saturator 1 / PhaseOscType 1` (triangle partials, a soft saturator on the main
//! wave), Ride `Saturator 0 / PhaseOscType 0` (sine partials, no saturator); the code
//! is otherwise the same, so the variant is a `crash` flag of
//! [`MainOscAttack::process_sample`].
//!
//! Layout (offsets from the `AttackCore` base, which sits at `CrashCymbalCore +0x100`
//! and `RideCymbalCore +0x100`):
//!
//! | offset | part |
//! |---|---|
//! | +0x008 | [`MainOscAttack`] (0x570): a phase oscillator shaped into a pulse-ish wave, four [`PhaseOsc`] partials, three envelopes, a one-pole low-pass |
//! | +0x578 | [`FluffyOsc`] (only its `setSampleRate`; the cores weight its output by 0) |
//! | +0x5c0 | [`ParallelNoise`] (0x218): white noise through a 2-pole high-pass and a band filter, under an envelope |
//! | +0x7d8 | the core's own fields: delays and gains, the activity flag, a 4-lane gain ramp |
//!
//! The cores drive the parts directly: per sample they advance the gain ramp
//! ([`AttackCore::advance_ramp`]), call [`MainOscAttack::process_sample`] and
//! [`ParallelNoise::process_sample`], and weight the two by the gains (ramp lanes 1
//! and 2 while ramping, else [`AttackCore::gain_osc`] / [`AttackCore::gain_noise`]).
//! At a sample-rate change they call the parts' `set_sample_rate` one by one and then
//! [`AttackCore::update_delays`].
//!
//! Every arithmetic step follows the VST3 build's arm64 code (fused multiply-adds as
//! `mul_add`, `f64` where it uses doubles). It nulls bit-exactly against Maschine's own
//! functions (the `oracle` test at the bottom).

#![allow(dead_code)]

#[cfg(test)]
use super::attached::Slot;
use super::memo::{K, MemoEnvelope};
use crate::fx::exp_e;

const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// `(60 − 0)·K + 1200`: the exp-table input of a 0-time stage (`E` ≈ 1000).
const ZERO_TIME: f32 = c(0x44e0_be4e);
/// `0·K + 602.0529`: the exp-table input of a 0-length hold.
const HOLD0: f32 = c(0x4416_8363);
const TWO_PI: f32 = c(0x40c9_0fdb);
const PI: f32 = c(0x4049_0fdb);
/// The LCG's scale to ±1 (`0x3000009d`).
const RND: f32 = c(0x3000_009d);

/// The `x + x³/3 + 2x⁵/15` tangent approximation of the filter setters.
#[inline]
fn tan3(x: f32) -> f32 {
    let x2 = x * x;
    let x3 = x * x2;
    let x5 = x2 * x3;
    x + x5.mul_add(c(0x3e08_8886), x3 * c(0x3eaa_aaaa))
}

/// The modal drums' velocity curve: `v` mapped from `[x0, x1]` to 0..1 and bent twice
/// (`t·(1 + s − s·|t|)`), clamped to 0..1.
#[inline]
fn curve(v: f32, x0: f32, x1: f32, s1: f32, s2: f32) -> f32 {
    let x1 = if x1 > x0 { x1 } else { x0 + c(0x3b03_126f) };
    let v = if x0 > v { x0 } else { v };
    let v = if v > x1 { x1 } else { v };
    let d = x1 - x0;
    let d = if d < c(0x33d6_bf95) { c(0x33d6_bf95) } else { d };
    let mut t = (v - x0) / d;
    t *= (-t.abs()).mul_add(s1, s1 + 1.0);
    t *= (-t.abs()).mul_add(s2, s2 + 1.0);
    let t = if t < 0.0 { 0.0 } else { t };
    if t > 1.0 { 1.0 } else { t }
}

/// The wave-shape weight of an increment: `2·inc`, faded in between 0.01 and 0.03.
#[inline]
fn inc_weight(inc: f32) -> f32 {
    let m = if inc < c(0x3c23_d70a) { c(0x3c23_d70a) } else { inc };
    let r = (m + c(0xbc23_d70a)) / c(0x3ca3_d70a);
    (inc + inc) * if m > c(0x3cf5_c28f) { 1.0 } else { r }
}

#[inline]
fn clamp1(x: f32) -> f32 {
    let x = if x < -1.0 { -1.0 } else { x };
    if x > 1.0 { 1.0 } else { x }
}

/// Writing a field into (and reading it from) a memory image in NI's representation,
/// for diffing whole-core state against dumps of Maschine's objects.
pub(super) trait Img: Sized {
    /// Size in NI's layout (an array's element stride).
    const SIZE: usize;
    fn put(&self, img: &mut [u8], off: usize);
    fn get(img: &[u8], off: usize) -> Self;
    /// Visit every field as `(offset, slot)`.
    #[cfg(test)]
    fn walk_at(&mut self, off: usize, v: &mut dyn FnMut(usize, Slot));
}

macro_rules! scalar_img {
    ($($t:ty => $s:ident),*) => {$(
        impl Img for $t {
            #[cfg(test)]
            fn walk_at(&mut self, off: usize, v: &mut dyn FnMut(usize, Slot)) {
                v(off, Slot::$s(self))
            }
            const SIZE: usize = size_of::<$t>();
            fn put(&self, img: &mut [u8], off: usize) {
                img[off..off + size_of::<$t>()].copy_from_slice(&self.to_le_bytes());
            }
            fn get(img: &[u8], off: usize) -> Self {
                <$t>::from_le_bytes(img[off..off + size_of::<$t>()].try_into().unwrap())
            }
        }
    )*};
}
scalar_img!(f32 => F, f64 => D, u32 => U, i32 => I);

impl Img for bool {
    const SIZE: usize = 1;
    #[cfg(test)]
    fn walk_at(&mut self, off: usize, v: &mut dyn FnMut(usize, Slot)) {
        v(off, Slot::B(self))
    }
    fn put(&self, img: &mut [u8], off: usize) {
        img[off] = u8::from(*self);
    }
    fn get(img: &[u8], off: usize) -> Self {
        img[off] != 0
    }
}

impl<T: Img, const N: usize> Img for [T; N] {
    const SIZE: usize = N * T::SIZE;
    #[cfg(test)]
    fn walk_at(&mut self, off: usize, v: &mut dyn FnMut(usize, Slot)) {
        for (i, x) in self.iter_mut().enumerate() {
            x.walk_at(off + i * T::SIZE, v);
        }
    }
    fn put(&self, img: &mut [u8], off: usize) {
        for (i, x) in self.iter().enumerate() {
            x.put(img, off + i * T::SIZE);
        }
    }
    fn get(img: &[u8], off: usize) -> Self {
        std::array::from_fn(|i| T::get(img, off + i * T::SIZE))
    }
}

impl Img for MemoEnvelope {
    const SIZE: usize = 0x80;
    #[cfg(test)]
    fn walk_at(&mut self, o: usize, v: &mut dyn FnMut(usize, Slot)) {
        self.state.walk_at(o, v);
        self.attack.walk_at(o + 0x04, v);
        self.hold.walk_at(o + 0x08, v);
        self.decay.walk_at(o + 0x0c, v);
        self.release.walk_at(o + 0x10, v);
        self.attack_step.walk_at(o + 0x18, v);
        self.hold_len.walk_at(o + 0x20, v);
        self.decay_step.walk_at(o + 0x28, v);
        self.sustain.walk_at(o + 0x30, v);
        self.release_mul.walk_at(o + 0x38, v);
        self.inc.walk_at(o + 0x40, v);
        self.dec.walk_at(o + 0x48, v);
        self.sus.walk_at(o + 0x50, v);
        self.peak32.walk_at(o + 0x58, v);
        self.peak.walk_at(o + 0x60, v);
        self.cur.walk_at(o + 0x68, v);
        self.count.walk_at(o + 0x70, v);
        self.sr.walk_at(o + 0x74, v);
        self.inv_sr.walk_at(o + 0x78, v);
    }
    fn put(&self, img: &mut [u8], o: usize) {
        self.state.put(img, o);
        self.attack.put(img, o + 0x04);
        self.hold.put(img, o + 0x08);
        self.decay.put(img, o + 0x0c);
        self.release.put(img, o + 0x10);
        self.attack_step.put(img, o + 0x18);
        self.hold_len.put(img, o + 0x20);
        self.decay_step.put(img, o + 0x28);
        self.sustain.put(img, o + 0x30);
        self.release_mul.put(img, o + 0x38);
        self.inc.put(img, o + 0x40);
        self.dec.put(img, o + 0x48);
        self.sus.put(img, o + 0x50);
        self.peak32.put(img, o + 0x58);
        self.peak.put(img, o + 0x60);
        self.cur.put(img, o + 0x68);
        self.count.put(img, o + 0x70);
        self.sr.put(img, o + 0x74);
        self.inv_sr.put(img, o + 0x78);
    }
    fn get(img: &[u8], o: usize) -> Self {
        MemoEnvelope {
            state: Img::get(img, o),
            attack: Img::get(img, o + 0x04),
            hold: Img::get(img, o + 0x08),
            decay: Img::get(img, o + 0x0c),
            release: Img::get(img, o + 0x10),
            attack_step: Img::get(img, o + 0x18),
            hold_len: Img::get(img, o + 0x20),
            decay_step: Img::get(img, o + 0x28),
            sustain: Img::get(img, o + 0x30),
            release_mul: Img::get(img, o + 0x38),
            inc: Img::get(img, o + 0x40),
            dec: Img::get(img, o + 0x48),
            sus: Img::get(img, o + 0x50),
            peak32: Img::get(img, o + 0x58),
            peak: Img::get(img, o + 0x60),
            cur: Img::get(img, o + 0x68),
            count: Img::get(img, o + 0x70),
            sr: Img::get(img, o + 0x74),
            inv_sr: Img::get(img, o + 0x78),
        }
    }
}

/// A struct whose fields sit at fixed NI offsets: the struct (each field documented
/// with its offset), its [`Img`] and `image()`.
macro_rules! layout {
    ($(#[$m:meta])* $name:ident ($size:literal) { $( $(#[$fm:meta])* $off:literal $f:ident : $t:ty ),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug)]
        pub(super) struct $name {
            $( #[doc = concat!("+", stringify!($off), ":")] $(#[$fm])* pub(super) $f: $t, )*
        }
        impl Img for $name {
            const SIZE: usize = $size;
            #[cfg(test)]
            fn walk_at(&mut self, base: usize, v: &mut dyn FnMut(usize, Slot)) {
                $( self.$f.walk_at(base + $off, v); )*
            }
            fn put(&self, img: &mut [u8], base: usize) {
                $( self.$f.put(img, base + $off); )*
            }
            fn get(img: &[u8], base: usize) -> Self {
                $name { $( $f: Img::get(img, base + $off), )* }
            }
        }
        impl $name {
            /// Every field at `base` + its NI offset, in NI's representation.
            #[cfg(test)]
            pub(super) fn image(&self, img: &mut [u8], base: usize) {
                self.put(img, base)
            }

            /// Visit every field as `(offset from this object, slot)`, for diffing
            /// against and loading from NI dumps.
            #[cfg(test)]
            pub(super) fn walk(&mut self, v: &mut dyn FnMut(usize, Slot)) {
                self.walk_at(0, v)
            }
        }
    };
}

layout!(
    /// `UTIL::PhaseOsc<type>` (0xc0 bytes): one partial of the main oscillator, a phase
    /// accumulator read as a triangle (type 1, Crash) or a sine (type 0, Ride), under
    /// its own envelope.
    PhaseOsc (0xc0) {
        /// frequency factor (multiplies the main oscillator's base frequency).
        0x00 freq: f32,
        /// release time parameter of the envelope.
        0x04 release: f32,
        /// amplitude offset (plus `amp_mod`, clamped 0..1).
        0x08 amp: f32,
        /// on; off partials return 0.
        0x0c enabled: bool,
        0x0d b0d: bool,
        /// note-on resets the phase.
        0x0e reset: bool,
        /// attack time parameter of the envelope.
        0x10 attack: f32,
        0x18 env: MemoEnvelope,
        /// phase (−0.5..0.5).
        0x98 phase: f32,
        /// last waveform value.
        0x9c out: f32,
        0xa0 sr: f32,
        0xa4 inv_sr: f32,
        /// last increment × 100 × its fade weight.
        0xa8 inc: f32,
        /// increment factor − 1 (the per-note random detune).
        0xac phase_mod: f32,
        /// amplitude modulation (added to `amp`).
        0xb0 amp_mod: f32,
        0xb4 gain: f32,
        /// phase offset of the waveform.
        0xb8 phase_offset: f32,
        /// frequency ratio (`setModulationFreqPhOscRatio`).
        0xbc ratio: f32,
    }
);

impl PhaseOsc {
    fn new() -> PhaseOsc {
        let env = MemoEnvelope { sr: 1.0, inv_sr: 1.0, ..Default::default() };
        PhaseOsc {
            freq: 0.0,
            release: 0.0,
            amp: 0.0,
            enabled: false,
            b0d: false,
            reset: false,
            attack: 0.0,
            env,
            phase: 0.0,
            out: 0.0,
            sr: 1.0,
            inv_sr: 1.0,
            inc: 1.0,
            phase_mod: 0.0,
            amp_mod: 0.0,
            gain: 1.0,
            phase_offset: 0.0,
            ratio: 1.0,
        }
    }

    /// `init(sr, attack, depth, reset)` (type 1 0x1ce5acc, type 0 0x1ce5544): the
    /// envelope's times (0 hold, 0 decay, sustain 1).
    pub(super) fn init(&mut self, sr: f32, attack: f32, depth: f32, reset: bool) {
        if !(1.0..=500_000.0).contains(&sr) || !(-24.0..=40.0).contains(&attack) || !(0.0..=1.0).contains(&depth) {
            return;
        }
        self.sr = sr;
        self.inv_sr = 1.0 / sr;
        self.inc = 0.0;
        self.env.set_sample_rate(sr);
        self.env.hold_len = f64::from(sr * (exp_e(HOLD0) + -0.001));
        self.env.hold = 0.0;
        self.env.decay = 0.0;
        self.env.decay_step = self.env.inv_sr * f64::from(exp_e(ZERO_TIME));
        self.env.sustain = 1.0;
        self.attack = attack;
        self.env.attack = attack;
        self.env.attack_step = f64::from(exp_e((60.0 - attack).mul_add(K, 1200.0))) * self.env.inv_sr;
        self.reset = reset;
        self.env.release = self.release;
        let r = self.env.inv_sr * f64::from(exp_e((60.0 - self.release).mul_add(K, 1200.0)));
        self.env.release_mul = if r < 1.0 { 1.0 - r } else { 0.0 };
    }

    /// `processSample()` (type 1 0x1ce587c when `triangle`, type 0 0x1ce52dc).
    #[inline]
    pub(super) fn process_sample(&mut self, triangle: bool) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        let i = clamp1((self.phase_mod + 1.0) * self.freq * self.ratio * self.inv_sr);
        let p = self.phase + i;
        let ph = if p < 0.5 { p } else { p + -1.0 };
        let m = if i < c(0x3c23_d70a) { c(0x3c23_d70a) } else { i };
        let d = if m > c(0x3cf5_c28f) { c(0x3ca3_d70a) } else { m + c(0xbc23_d70a) };
        let inc = i * 100.0 * d;
        let x = self.phase_offset + ph;
        let x = x - (x as i32) as f32 + -0.25;
        let r = (f64::from(if x < 0.0 { -0.5f32 } else { 0.5 }) + f64::from(x)) as i32 as f32;
        let out = if triangle {
            let s = if inc + inc > 1.0 { 1.0 } else { inc + inc };
            let t = (x - r).abs().mul_add(-4.0, 1.0);
            t * (-s).mul_add(t.abs(), s + 1.0)
        } else {
            let h = 0.5 - ((x - r) + (x - r)).abs();
            let h2 = h * h;
            h * h2.mul_add(c(0x4010_fda0), c(0xc0a4_3f68)).mul_add(h2, c(0x4049_0fd0))
        };
        self.phase = ph;
        self.inc = inc;
        self.out = out;
        let a = self.amp + self.amp_mod;
        let a = if a < 0.0 { 0.0 } else { a };
        let a = if a > 1.0 { 1.0 } else { a };
        let s = self.gain * a * out;
        s * self.env.process()
    }
}

layout!(
    /// `UTIL::MainOscAttack<…>` (0x570 bytes at `AttackCore +0x8`).
    MainOscAttack (0x570) {
        /// base frequency (Hz; `setF0FromBody`).
        0x000 freq: f32,
        /// wave symmetry (−1..1).
        0x004 sym: f32,
        /// sync: note-on restarts the phases.
        0x008 sync: bool,
        /// depth of the per-note random detune (squared).
        0x00c rnd_depth: f32,
        /// attack time parameter of the partials' envelopes.
        0x010 attack: f32,
        0x014 f14: f32,
        0x018 f18: f32,
        /// saturator-envelope curve of the velocity curve's output: `x0`, `x1`, bend,
        /// blend.
        0x01c sat_x0: f32,
        0x020 sat_x1: f32,
        0x024 sat_bend: f32,
        0x028 sat_blend: f32,
        /// saturator drive.
        0x02c sat_drive: f32,
        0x030 f30: [f32; 10],
        /// increment multiplier.
        0x058 mult: f32,
        /// frequency ratio (`setModulationFreqPhOscRatio`).
        0x05c ratio: f32,
        /// low-pass cutoff pitch from velocity (`E(p·5 + 1381.88)` Hz): low, high,
        /// `x0`, `x1`, two bends.
        0x060 cut_lo: f32,
        0x064 cut_hi: f32,
        0x068 cut_x0: f32,
        0x06c cut_x1: f32,
        0x070 cut_s1: f32,
        0x074 cut_s2: f32,
        /// velocity curve of the amplitude: `x0`, `x1`, two bends.
        0x078 vel_x0: f32,
        0x07c vel_x1: f32,
        0x080 vel_s1: f32,
        0x084 vel_s2: f32,
        /// amplitude envelope.
        0x088 env_amp: MemoEnvelope,
        /// second envelope (started at note-on, never read).
        0x108 env_b: MemoEnvelope,
        /// saturator envelope (read by Crash only).
        0x188 env_sat: MemoEnvelope,
        /// saturator: `0.9 − 0.8·e²·drive`, that floored at 2e-12, twice it, its inverse.
        0x208 sat_a: f32,
        0x20c sat_b: f32,
        0x210 sat_b2: f32,
        0x214 sat_inv: f32,
        /// one-pole low-pass: state, input − state, new state, input − new state.
        0x218 lp_state: f32,
        0x21c lp_diff: f32,
        0x220 lp_y: f32,
        0x224 lp_e: f32,
        /// its coefficient `min(2π·fc/sr, 1)`, cutoff (Hz) and `1/sr`.
        0x228 lp_coef: f32,
        0x22c lp_freq: f32,
        0x230 lp_inv_sr: f32,
        /// the four partials (+0x238, +0x2f8, +0x3b8, +0x478).
        0x238 osc: [PhaseOsc; 4],
        /// note velocity.
        0x538 vel: f32,
        /// `1000/(2·sr)`.
        0x53c half_ms: f32,
        0x540 f540: f32,
        /// the wave at the last synced note-on.
        0x544 f544: f32,
        0x548 f548: f32,
        /// main phase (−0.5..0.5).
        0x54c phase: f32,
        0x550 sr: f32,
        0x554 inv_sr: f32,
        /// shape weight of the wave (`2·inc`, faded in).
        0x558 shape: f32,
        /// phase increment per sample (clamped ±1).
        0x55c inc: f32,
        /// last wave value.
        0x560 wave: f32,
        /// LCG state of the random detune.
        0x564 seed: i32,
        /// this note's random detune.
        0x568 rnd: f32,
        /// last cutoff pitch.
        0x56c lp_pitch: f32,
    }
);

impl MainOscAttack {
    fn new() -> MainOscAttack {
        let env = MemoEnvelope { sr: 1.0, inv_sr: 1.0, ..Default::default() };
        MainOscAttack {
            freq: 0.0,
            sym: 0.0,
            sync: false,
            rnd_depth: 0.0,
            attack: 0.0,
            f14: 0.0,
            f18: 0.0,
            sat_x0: 0.0,
            sat_x1: 1.0,
            sat_bend: 0.0,
            sat_blend: 0.0,
            sat_drive: 0.0,
            f30: [0.0; 10],
            mult: 1.0,
            ratio: 1.0,
            cut_lo: 0.0,
            cut_hi: 0.0,
            cut_x0: 0.0,
            cut_x1: 0.0,
            cut_s1: 0.0,
            cut_s2: 0.0,
            vel_x0: 0.0,
            vel_x1: 0.0,
            vel_s1: 0.0,
            vel_s2: 0.0,
            env_amp: env.clone(),
            env_b: env.clone(),
            env_sat: env,
            sat_a: 1.0,
            sat_b: 1.0,
            sat_b2: 2.0,
            sat_inv: 1.0,
            lp_state: 0.0,
            lp_diff: 0.0,
            lp_y: 0.0,
            lp_e: 0.0,
            lp_coef: 0.0,
            lp_freq: 1.0,
            lp_inv_sr: 1.0,
            osc: std::array::from_fn(|_| PhaseOsc::new()),
            vel: 0.0,
            half_ms: 500.0,
            f540: 0.0,
            f544: 0.0,
            f548: 0.0,
            phase: 1.0,
            sr: 1.0,
            inv_sr: 1.0,
            shape: 0.0,
            inc: 0.0,
            wave: 0.0,
            seed: 1,
            rnd: 0.0,
            lp_pitch: -1.0,
        }
    }

    /// `setSampleRate(sr)` (0x1e0172c).
    pub(super) fn set_sample_rate(&mut self, sr: f32) {
        if !(1.0..=500_000.0).contains(&sr) {
            return;
        }
        self.sr = sr;
        let inv = 1.0 / sr;
        self.inv_sr = inv;
        self.half_ms = 1000.0 / (sr + sr);
        self.f544 = 0.0;
        self.phase = 1.0;
        self.shape = 0.0;
        self.inc = 0.0;
        for o in &mut self.osc {
            o.init(sr, self.attack, self.rnd_depth, self.sync);
        }
        for e in [&mut self.env_amp, &mut self.env_b, &mut self.env_sat] {
            e.set_sample_rate(sr);
            e.decay = 0.0;
            e.decay_step = e.inv_sr * f64::from(exp_e(ZERO_TIME));
            e.sustain = 1.0;
        }
        self.lp_inv_sr = inv;
        let old = self.lp_freq;
        self.lp_freq = -1.0;
        if old != -1.0 {
            self.lp_freq = old;
            let k = inv * TWO_PI * old;
            self.lp_coef = if k > 1.0 { 1.0 } else { k };
        }
        self.lp_state = 0.0;
        self.lp_diff = 0.0;
        self.lp_y = 0.0;
        self.lp_e = 0.0;
        self.inc = clamp1(self.inv_sr * (self.freq * (self.ratio * self.mult)));
        self.shape = inc_weight(self.inc);
        self.sym = clamp1(self.sym);
        let sync = if self.sync { 1.0f32 } else { 0.0 } == 1.0;
        self.sync = sync;
        let e = f64::from(exp_e((60.0 - self.attack).mul_add(K, 1200.0)));
        for o in &mut self.osc {
            o.reset = sync;
            o.attack = self.attack;
            o.env.attack = self.attack;
            o.env.attack_step = e * o.env.inv_sr;
        }
    }

    /// `setNoteOn(v)` (0x1e01ae0).
    pub(super) fn set_note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        self.vel = v;
        let t = curve(v, self.vel_x0, self.vel_x1, self.vel_s1, self.vel_s2);
        self.env_amp.set_note_on(t);
        self.env_b.set_note_on(t);
        // The saturator envelope's level: `t` bent once, then blended toward 1 − t.
        let (x0, w) = (self.sat_x0, self.sat_blend);
        let x1 = if self.sat_x1 > x0 { self.sat_x1 } else { x0 + c(0x3b03_126f) };
        let u = if t < x0 { x0 } else { t };
        let u = if u > x1 { x1 } else { u };
        let d = x1 - x0;
        let d = if d < c(0x33d6_bf95) { c(0x33d6_bf95) } else { d };
        let mut q = (u - x0) / d;
        q *= (-q.abs()).mul_add(self.sat_bend, self.sat_bend + 1.0);
        let a = 1.0 - q;
        let m = if w + w > 1.0 { 1.0 } else { w + w };
        let a = (1.0 - a).mul_add(m, a);
        let g = w.mul_add(2.0, -m);
        let r = (q - a).mul_add(g, a);
        let r = if r < 0.0 { 0.0 } else { r };
        self.env_sat.set_note_on(if r > 1.0 { 1.0 } else { r });
        let depth = self.rnd_depth * self.rnd_depth;
        self.seed = self.seed.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        self.rnd = depth * (self.seed as f32 * RND);
        for o in &mut self.osc {
            o.phase_mod = self.rnd;
            o.env.set_note_on(t);
            if o.reset {
                o.phase = 0.0;
            }
        }
        if self.sync {
            self.phase = 1.0;
            self.f540 = 0.0;
            self.f544 = self.wave;
        }
    }

    /// `processSample()` (Crash 0x1e01d88 when `crash`, Ride 0x1e02f10). Returns the
    /// low-pass output.
    #[inline]
    pub(super) fn process_sample(&mut self, crash: bool) -> f32 {
        let shape = self.shape;
        let p = self.inc + self.phase;
        let ph = if p < 0.5 { p } else { p + -1.0 };
        self.phase = ph;
        let mut sum = 0.0f32;
        for o in &mut self.osc {
            sum += o.process_sample(crash);
        }
        let x = ph + sum;
        let r = (f64::from(if x < 0.0 { -0.5f32 } else { 0.5 }) + f64::from(x)) as i32 as f32;
        let x = x - r;
        let x = x + x;
        let b = (self.sym + 1.0) * 0.5;
        let b = (0.5 - b).mul_add(shape, b);
        let a = x.abs();
        let (num, den) = if a > b { (1.0 - a, 1.0 - b) } else { (a, b) };
        let den = if den < c(0x3586_37bd) { c(0x3586_37bd) } else { den };
        let q = num / den;
        let m = if shape + shape > 1.0 { 1.0 } else { shape + shape };
        let g = (-q.abs()).mul_add(m, m + 1.0);
        let wave = if x > 0.0 { q * g } else { -(q * g) };
        self.wave = wave;
        let (sat_a, sat_inv) = if crash {
            let e = self.env_sat.process();
            let sat_a = (e * c(0xbf4c_cccc)).mul_add(e * self.sat_drive, c(0x3f66_6666));
            self.sat_a = sat_a;
            let b = if sat_a < c(0x2c0c_bccc) { c(0x2c0c_bccc) } else { sat_a };
            self.sat_b = b;
            self.sat_b2 = b + b;
            self.sat_inv = 1.0 / b;
            (sat_a, self.sat_inv)
        } else {
            (0.0, 0.0)
        };
        let env = self.env_amp.process();
        let t = curve(self.vel, self.cut_x0, self.cut_x1, self.cut_s1, self.cut_s2);
        let pitch = (self.cut_hi - self.cut_lo).mul_add(t, self.cut_lo);
        if pitch != self.lp_pitch {
            let f = exp_e(pitch.mul_add(5.0, c(0x44ac_bc36)));
            if self.lp_freq != f {
                self.lp_freq = f;
                let k = f * (self.lp_inv_sr * TWO_PI);
                self.lp_coef = if k > 1.0 { 1.0 } else { k };
            }
            self.lp_pitch = pitch;
        }
        let x = if crash {
            let lim = self.sat_b2;
            let w = if wave < -lim { -lim } else { wave };
            let w = if w > lim { lim } else { w };
            let s = (w.abs() * -0.25).mul_add(sat_inv, 1.0);
            ((1.0 / sat_a) * (w * s)) * env
        } else {
            wave * env
        };
        let y = self.lp_coef.mul_add(self.lp_diff, self.lp_state);
        self.lp_y = y;
        let e = x - y;
        self.lp_e = e;
        self.lp_state = y;
        self.lp_diff = e;
        y
    }
}

layout!(
    /// `UTIL::FluffyOsc` (0x48 bytes at `AttackCore +0x578`). Only its sample-rate
    /// setup runs.
    FluffyOsc (0x48) {
        0x00 f00: f32,
        0x04 f04: f32,
        0x08 f08: f32,
        0x0c f0c: f32,
        /// 1: the period counts double.
        0x10 mode: f32,
        0x14 f14: f32,
        0x18 f18: f32,
        /// frequency (Hz).
        0x1c freq: f32,
        0x20 f20: f32,
        0x24 f24: f32,
        0x28 f28: f32,
        0x2c sr: f32,
        0x30 inv_sr: f32,
        /// `clamp(2·freq/sr, −1, 1)`.
        0x34 inc: f32,
        /// sounding (`AttackCore::setNoteOn` checks it).
        0x38 active: bool,
        0x3c f3c: f32,
        /// period in samples (rounded up).
        0x40 period: i32,
    }
);

impl FluffyOsc {
    fn new() -> FluffyOsc {
        FluffyOsc {
            f00: 0.0,
            f04: 1.0,
            f08: 0.0,
            f0c: 0.0,
            mode: 0.0,
            f14: 1.0,
            f18: 0.0,
            freq: 20.0,
            f20: 0.0,
            f24: 0.0,
            f28: 0.0,
            sr: 1.0,
            inv_sr: 1.0,
            inc: 0.0,
            active: false,
            f3c: 0.0,
            period: 0,
        }
    }

    /// `setSampleRate(sr)` (0x1ce3980).
    pub(super) fn set_sample_rate(&mut self, sr: f32) {
        if !(1.0..=500_000.0).contains(&sr) {
            return;
        }
        let inv = 1.0 / sr;
        self.sr = sr;
        self.inv_sr = inv;
        self.inc = clamp1((inv + inv) * self.freq);
        let m = if self.mode == 1.0 { 2.0 } else { 1.0 };
        let n = (1.0 / (self.freq * m)) * sr;
        let d = f64::from(n);
        self.period = (if n < 0.0 { d } else { d + 1.0 }) as i32;
        self.f24 = 0.0;
    }
}

layout!(
    /// `UTIL::NoiseGen<white>` (0xc4 bytes at `ParallelNoise +0x54`): an LCG.
    NoiseGen (0xc4) {
        0x00 enabled: bool,
        0x04 sr: f32,
        /// seed restored at note-on.
        0x08 seed0: i32,
        0x10 f10: f32,
        0x14 seed_a: i32,
        0x18 seed_b: i32,
        /// pink-noise filter sections (two coefficients, two states; idle for white).
        0x1c pink: [[f32; 4]; 7],
        0x8c ready: bool,
        /// output scale `√(sr/44100)·2⁻³¹`.
        0x90 gain: f32,
        0x94 sr2: f32,
        /// LCG state.
        0x98 seed: i32,
        0x9c f9c: f32,
        0xa0 inv_sqrt: f32,
        /// 15 Hz one-pole (idle for white): states, `1/(g+1)`, `g − 1`, `g`,
        /// frequency, `1/sr`.
        0xa4 lp_a4: f32,
        0xa8 lp_a8: f32,
        0xac lp_ac: f32,
        0xb0 lp_b0: f32,
        0xb4 lp_b4: f32,
        0xb8 lp_g: f32,
        0xbc lp_freq: f32,
        0xc0 inv_sr: f32,
    }
);

impl NoiseGen {
    fn new() -> NoiseGen {
        let k = |a: u32, b: u32| [c(a), c(b), 0.0, 0.0];
        NoiseGen {
            enabled: false,
            sr: 1.0,
            seed0: 2000,
            f10: 0.0,
            seed_a: 0,
            seed_b: 2000,
            pink: [
                k(0x3d63_66bd, 0x3f7f_b54a),
                k(0x3d99_c165, 0x3f7e_4a38),
                k(0x3e1d_8b61, 0x3f78_1062),
                k(0x3e9e_f7f8, 0x3f5d_d2f2),
                k(0x3f08_6f8e, 0x3f0c_cccd),
                k(0xbc8a_6dad, 0xbf42_f838),
                k(0x3f09_4467, 0x3ded_6a9c),
            ],
            ready: false,
            gain: RND,
            sr2: 1.0,
            seed: 0,
            f9c: 0.0,
            inv_sqrt: 1.0,
            lp_a4: 0.0,
            lp_a8: 0.0,
            lp_ac: 0.0,
            lp_b0: 0.0,
            lp_b4: 0.0,
            lp_g: 0.0,
            lp_freq: 1.0,
            inv_sr: 1.0,
        }
    }

    /// `setSampleRate(sr)` (0x1ce4ed0).
    pub(super) fn set_sample_rate(&mut self, sr: f32) {
        if !(1.0..=500_000.0).contains(&sr) {
            return;
        }
        self.sr = sr;
        self.ready = true;
        let r = (sr / 44100.0).sqrt();
        self.gain = r * RND;
        self.sr2 = sr;
        self.inv_sqrt = 1.0 / r;
        let inv = 1.0 / sr;
        self.inv_sr = inv;
        if self.lp_freq == -1.0 {
            self.lp_a8 = 0.0;
            self.lp_ac = 0.0;
            self.lp_a4 = 0.0;
        } else {
            let g = tan3(inv * (self.lp_freq * PI));
            self.lp_b0 = 1.0 / (g + 1.0);
            self.lp_b4 = g + -1.0;
            self.lp_g = g;
            self.lp_a8 = 0.0;
            self.lp_ac = 0.0;
            if self.lp_freq == 15.0 {
                return;
            }
        }
        self.lp_freq = 15.0;
        let g = tan3(inv * c(0x423c_7edd));
        self.lp_g = g;
        self.lp_b0 = 1.0 / (g + 1.0);
        self.lp_b4 = g + -1.0;
        self.lp_ac = 0.0;
    }

    /// `setNoteOn(v)` (0x1ce5210): restart the generator from its seed.
    pub(super) fn set_note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) || !self.enabled {
            return;
        }
        self.seed = self.seed0;
        self.seed_a = self.seed0;
        self.seed_b = self.seed0;
        self.f10 = 0.0;
        for s in &mut self.pink {
            s[2] = 0.0;
            s[3] = 0.0;
        }
    }
}

layout!(
    /// `UTIL::ParallelNoise<white>` (0x218 bytes at `AttackCore +0x5c0`): LCG noise
    /// through a 2-pole state-variable filter (A, its high-pass output) and a 2-pole
    /// band filter (B), under an envelope.
    ParallelNoise (0x218) {
        /// level.
        0x000 amp: f32,
        0x004 f04: f32,
        /// filter A cutoff (Hz).
        0x008 hp_freq: f32,
        /// base pitch of filter B (`setModulationNoiseLP` adds to it).
        0x00c bp_base: f32,
        /// filter B frequency (Hz).
        0x010 bp_freq: f32,
        0x014 f14: f32,
        0x018 b18: bool,
        0x01c f1c: f32,
        0x020 f20: f32,
        0x024 f24: f32,
        0x028 f28: f32,
        /// base release time (`setModulationPNoiseEnvDec2` adds to it).
        0x02c rel_base: f32,
        /// velocity curve: `x0`, `x1`, two bends.
        0x030 vel_x0: f32,
        0x034 vel_x1: f32,
        0x038 vel_s1: f32,
        0x03c vel_s2: f32,
        /// `setModulationNoiseLP` value.
        0x040 bp_mod: f32,
        0x044 f44: f32,
        0x048 f48: f32,
        /// `setModulationPNoiseEnvDec2` value.
        0x04c rel_mod: f32,
        0x050 f50: f32,
        0x054 lcg: NoiseGen,
        0x118 env: MemoEnvelope,
        /// filter A (TPT state-variable): states, outputs (hp, bp, lp), `g`, damping
        /// `k`, `g·(g + k)`, cutoff, `1/(1 + g·(g + k))`, reset-damping flag, rate.
        0x198 a_z1: f32,
        0x19c a_z2: f32,
        0x1a0 a_z3: f32,
        0x1a4 a_hp: f32,
        0x1a8 a_bp: f32,
        0x1ac a_lp: f32,
        0x1b0 a_g: f32,
        0x1b4 a_k: f32,
        0x1b8 a_h: f32,
        0x1bc a_freq: f32,
        0x1c0 a_inv: f32,
        0x1c4 a_reset: f32,
        0x1c8 a_sr: f32,
        0x1cc a_inv_sr: f32,
        /// filter B: last input, two states, outputs, `g = min(2πf/sr, 1.9)`, `k`,
        /// feedback coefficient, frequency, reset-k flag, rate.
        0x1d0 b_prev: f32,
        0x1d4 b_s1: f32,
        0x1d8 b_s2: f32,
        0x1dc b_r: f32,
        0x1e0 b_s1x2: f32,
        0x1e4 b_out: f32,
        0x1e8 b_g: f32,
        0x1ec b_k: f32,
        0x1f0 b_c: f32,
        0x1f4 b_freq: f32,
        0x1f8 b_reset: f32,
        0x1fc b_sr: f32,
        0x200 b_inv_sr: f32,
        0x204 sr: f32,
        0x208 inv_sr: f32,
        0x20c f20c: f32,
        /// control block size.
        0x210 block: i32,
        /// note velocity.
        0x214 vel: f32,
    }
);

impl ParallelNoise {
    fn new() -> ParallelNoise {
        let mut env = MemoEnvelope { sr: 1.0, inv_sr: 1.0, ..Default::default() };
        let z = exp_e(ZERO_TIME);
        env.attack_step = f64::from(z);
        env.hold_len = f64::from(exp_e(HOLD0) + -0.001);
        env.decay_step = f64::from(z);
        env.release_mul = if z < 1.0 { 1.0 - f64::from(z) } else { 0.0 };
        ParallelNoise {
            amp: 0.0,
            f04: 0.0,
            hp_freq: 0.0,
            bp_base: 0.0,
            bp_freq: 0.0,
            f14: 0.0,
            b18: false,
            f1c: 0.0,
            f20: 0.0,
            f24: 0.0,
            f28: 0.0,
            rel_base: 0.0,
            vel_x0: 0.0,
            vel_x1: 0.0,
            vel_s1: 0.0,
            vel_s2: 0.0,
            bp_mod: 0.0,
            f44: 0.0,
            f48: 0.0,
            rel_mod: 0.0,
            f50: 0.0,
            lcg: NoiseGen::new(),
            env,
            a_z1: 0.0,
            a_z2: 0.0,
            a_z3: 0.0,
            a_hp: 0.0,
            a_bp: 0.0,
            a_lp: 0.0,
            a_g: 0.0,
            a_k: 2.0,
            a_h: 0.0,
            a_freq: 1.0,
            a_inv: 1.0,
            a_reset: 0.0,
            a_sr: 1.0,
            a_inv_sr: 1.0,
            b_prev: 0.0,
            b_s1: 0.0,
            b_s2: 0.0,
            b_r: 0.0,
            b_s1x2: 0.0,
            b_out: 0.0,
            b_g: 0.0,
            b_k: 2.0,
            b_c: 2.0,
            b_freq: 1.0,
            b_reset: 0.0,
            b_sr: 1.0,
            b_inv_sr: 1.0,
            sr: 1.0,
            inv_sr: 1.0,
            f20c: 0.0,
            block: 1,
            vel: 0.0,
        }
    }

    /// Filter A's cutoff `f` (stored clamped to 0..0.49·sr; coefficients from `f`).
    fn set_a(&mut self, f: f32, sr: f32, inv: f32) -> f32 {
        let lim = sr * c(0x3efa_e148);
        let s = if f < 0.0 { 0.0 } else { f };
        let s = if s > lim { lim } else { s };
        self.a_freq = s;
        let g = tan3(inv * (f * PI));
        self.a_g = g;
        let h = g * (g + self.a_k);
        self.a_h = h;
        self.a_inv = 1.0 / (h + 1.0);
        s
    }

    /// Filter B's frequency `f`.
    fn set_b(&mut self, f: f32, inv: f32) {
        self.b_freq = f;
        let g = (inv * (f * TWO_PI)).min(c(0x3ff3_3333));
        self.b_g = g;
        let n = (g + 2.0) * (2.0 - g) * self.b_k;
        let d = g.mul_add(self.b_k, 2.0) - g;
        self.b_c = n / (d + d);
    }

    /// `setSampleRate(sr, block)` (0x1ce4b08).
    pub(super) fn set_sample_rate(&mut self, sr: f32, block: i32) {
        if !(1.0..=500_000.0).contains(&sr) {
            return;
        }
        self.sr = sr;
        let inv = 1.0 / sr;
        self.inv_sr = inv;
        self.block = block;
        self.env.set_sample_rate(sr);
        self.a_sr = sr;
        self.a_inv_sr = inv;
        let old = self.a_freq;
        self.a_freq = -1.0;
        let cur = if old != -1.0 { self.set_a(old, sr, inv) } else { -1.0 };
        if cur != self.hp_freq {
            self.set_a(self.hp_freq, sr, inv);
        }
        if self.a_reset != 0.0 {
            self.a_reset = 0.0;
            self.a_k = 2.0;
            let h = self.a_g * (self.a_g + 2.0);
            self.a_h = h;
            self.a_inv = 1.0 / (h + 1.0);
        }
        self.b_sr = sr;
        self.b_inv_sr = inv;
        let old = self.b_freq;
        self.b_freq = -1.0;
        if old != -1.0 {
            self.set_b(old, inv);
        }
        if old != self.bp_freq {
            self.set_b(self.bp_freq, inv);
        }
        if self.b_reset != 0.0 {
            self.b_reset = 0.0;
            self.b_k = 2.0;
            let g = self.b_g;
            let n = (g + 2.0) * (2.0 - g);
            let d = g.mul_add(2.0, 2.0) - g;
            self.b_c = (n + n) / (d + d);
        }
        self.lcg.set_sample_rate(sr);
    }

    /// `setNoteOn(v)` (0x1ce50b8).
    pub(super) fn set_note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        self.vel = v;
        let t = curve(v, self.vel_x0, self.vel_x1, self.vel_s1, self.vel_s2);
        if t <= 0.0 {
            return;
        }
        self.lcg.set_note_on(t);
        self.env.set_note_on(t);
    }

    /// `processSample()` (0x1ce4918).
    #[inline]
    pub(super) fn process_sample(&mut self) -> f32 {
        let g = &mut self.lcg;
        g.seed = g.seed.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        let n = self.amp * (g.gain * g.seed as f32);
        let (z1, z2, z3, ga) = (self.a_z1, self.a_z2, self.a_z3, self.a_g);
        let t = z1.mul_add(self.a_h, z2 * ga.mul_add(2.0, self.a_k));
        let hp = self.a_inv * (n - (z3 + t));
        self.a_hp = hp;
        let bp = (z1 + hp).mul_add(ga, z2);
        self.a_bp = bp;
        let lp = (z2 + bp).mul_add(ga, z3);
        self.a_lp = lp;
        self.a_z1 = hp;
        self.a_z2 = bp;
        self.a_z3 = lp;
        let x = (hp + self.b_prev) * 0.25;
        self.b_prev = hp;
        let (gb, cb, s1, s2) = (self.b_g, self.b_c, self.b_s1, self.b_s2);
        let x = x - cb.mul_add(s1, s2);
        let s1 = gb.mul_add(x, s1);
        let s2n = gb.mul_add(s1, s2);
        let o = s2 + s2n;
        self.b_out = o;
        let s1x2 = s1 + s1;
        self.b_s1x2 = s1x2;
        self.b_r = hp - s1x2.mul_add(cb, o);
        self.b_s1 = s1;
        self.b_s2 = s2n;
        o * self.env.process()
    }
}

layout!(
    /// `DSP::AttackCore<…>` (0x888 bytes at `CrashCymbalCore +0x100` and
    /// `RideCymbalCore +0x100`; +0x0 is its vtable).
    AttackCore (0x888) {
        0x008 main: MainOscAttack,
        0x578 fluffy: FluffyOsc,
        0x5c0 noise: ParallelNoise,
        0x7d8 f7d8: f32,
        /// delay length (`updateDelays` multiplies it by 0).
        0x7e0 delay: f32,
        0x7e4 f7e4: i32,
        0x7e8 f7e8: f32,
        /// gain of the FluffyOsc (weighted by 0 in the cores).
        0x7ec gain_fluffy: f32,
        0x7f0 f7f0: f32,
        /// gain of the main oscillator.
        0x7f4 gain_osc: f32,
        0x7f8 f7f8: f32,
        /// gain of the noise.
        0x7fc gain_noise: f32,
        /// delay reference frequency and its "doubled" switch (1 = ×2).
        0x800 delay_freq: f32,
        0x804 delay_mode: f32,
        /// delay in samples (`updateDelays`).
        0x808 delay_n: i32,
        /// delay (ms) from the two pitches below (`updateDelays`).
        0x80c delay_ms: f32,
        0x810 f810: f32,
        0x814 b814: bool,
        /// `updateDelays` pitches: `E(p·K + 602.05)` + `E((q + r)·K + 602.05)`.
        0x818 dl_p: f32,
        0x81c dl_q: f32,
        0x820 dl_r: f32,
        0x824 f824: f32,
        /// a note was started.
        0x828 active: bool,
        /// 4-lane gain ramp (lane 0 fluffy, 1 oscillator, 2 noise): current, step per
        /// sample, target.
        0x840 ramp_cur: [f32; 4],
        0x850 ramp_step: [f32; 4],
        0x860 ramp_target: [f32; 4],
        /// the targets changed (`processControlRate` starts a ramp).
        0x870 ramp_changed: bool,
        0x871 ramping: bool,
        /// control blocks left in the ramp, its length, `1/length` (per sample).
        0x874 ramp_left: u32,
        0x878 ramp_len: u32,
        0x87c ramp_inv: f32,
        /// 0 idle, 1 armed, 2 started, 3 running, 4 done.
        0x880 ramp_state: u32,
    }
);

impl AttackCore {
    /// The constructor (Crash 0x1e01120, Ride 0x1e022a8: the same values).
    pub(super) fn new() -> AttackCore {
        AttackCore {
            main: MainOscAttack::new(),
            fluffy: FluffyOsc::new(),
            noise: ParallelNoise::new(),
            f7d8: 0.0,
            delay: -1.0,
            f7e4: -1,
            f7e8: 0.0,
            gain_fluffy: 0.0,
            f7f0: 0.0,
            gain_osc: 0.0,
            f7f8: 0.0,
            gain_noise: 0.0,
            delay_freq: 0.0,
            delay_mode: 0.0,
            delay_n: 0,
            delay_ms: 0.0,
            f810: 0.0,
            b814: false,
            dl_p: 0.0,
            dl_q: 0.0,
            dl_r: 0.0,
            f824: 0.0,
            active: false,
            ramp_cur: [0.0; 4],
            ramp_step: [0.0; 4],
            ramp_target: [0.0; 4],
            ramp_changed: true,
            ramping: false,
            ramp_left: 0,
            ramp_len: 0,
            ramp_inv: 0.0,
            ramp_state: 1,
        }
    }

    /// `setNoteOn(v)` (Crash 0x1d8b970, Ride 0x1d91054).
    pub(super) fn set_note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        if !self.fluffy.active && self.main.env_amp.state == 0 && self.noise.env.state == 0 && !self.active {
            self.ramp_cur = self.ramp_target;
            self.ramp_changed = true;
            self.ramp_state = 1;
        }
        self.active = true;
        self.main.set_note_on(v);
        self.noise.set_note_on(v);
        self.b814 = false;
    }

    /// `setF0FromBody(f)` (Crash only, 0x1d8b84c).
    pub(super) fn set_f0_from_body(&mut self, f: f32) {
        if !(0.0..=10000.0).contains(&f) {
            return;
        }
        let m = &mut self.main;
        m.freq = f;
        m.inc = clamp1(m.inv_sr * ((m.ratio * m.mult) * f));
        m.shape = inc_weight(m.inc);
    }

    /// `setModulationFreqPhOscRatio(r)` (0x1d8ab60 / 0x1d90574): the partials' ratio
    /// `max(E(r·5 + 1200), 0)`, `r` in ±64.
    pub(super) fn set_modulation_freq_ph_osc_ratio(&mut self, r: f32) {
        if !(-64.0..=64.0).contains(&r) {
            return;
        }
        let e = exp_e(r.mul_add(5.0, 1200.0)).max(0.0);
        let m = &mut self.main;
        m.ratio = e;
        m.inc = clamp1(m.inv_sr * (m.freq * (e * m.mult)));
        m.shape = inc_weight(m.inc);
        for o in &mut m.osc {
            o.ratio = e;
        }
    }

    /// `setModulationNoiseLP(p)` (0x1d8acfc / 0x1d8fef4): filter B at
    /// `E((base + p)·5 + 1381.88)` Hz, 20..20000, `p` in ±60.
    pub(super) fn set_modulation_noise_lp(&mut self, p: f32) {
        if !(-60.0..=60.0).contains(&p) {
            return;
        }
        let n = &mut self.noise;
        n.bp_mod = p;
        let f = exp_e((n.bp_base + p).mul_add(5.0, c(0x44ac_bc36)));
        let f = if f < 20.0 { 20.0 } else { f };
        let f = if f > 20000.0 { 20000.0 } else { f };
        n.bp_freq = f;
        if n.b_freq != f {
            n.b_freq = f;
            let g = (f * TWO_PI * n.b_inv_sr).min(c(0x3ff3_3333));
            n.b_g = g;
            let num = n.b_k * ((g + 2.0) * (2.0 - g));
            let d = g.mul_add(n.b_k, 2.0) - g;
            n.b_c = num / (d + d);
        }
    }

    /// `setModulationPNoiseEnvDec2(p)` (0x1d8ae90 / 0x1d90b2c): the noise envelope's
    /// release time `base + p`, `p` in ±40.
    pub(super) fn set_modulation_pnoise_env_dec2(&mut self, p: f32) {
        if !(-40.0..=40.0).contains(&p) {
            return;
        }
        let n = &mut self.noise;
        n.rel_mod = p;
        let r = n.rel_base + p;
        n.env.release = r;
        let d = f64::from(exp_e((60.0 - r).mul_add(K, 1200.0))) * n.env.inv_sr;
        n.env.release_mul = if d < 1.0 { 1.0 - d } else { 0.0 };
    }

    /// `updateDelays()` (0x1e015f4).
    pub(super) fn update_delays(&mut self) {
        let m = if self.delay_mode == 1.0 { 2.0 } else { 1.0 };
        let z = (1.0 / (self.delay_freq * m)) * 0.0;
        let n = self.delay * z;
        let d = f64::from(n);
        self.delay_n = (if n < 0.0 { d } else { d + 1.0 }) as i32;
        let a = exp_e(self.dl_p.mul_add(K, HOLD0));
        let b = exp_e((self.dl_q + self.dl_r).mul_add(K, HOLD0));
        self.delay_ms = (z + (a + b)) * 1000.0;
    }

    /// `processControlRate()` (vtable; 0x1d8be84 / 0x1d91568): run the gain ramp.
    pub(super) fn process_control_rate(&mut self) {
        if self.ramp_changed {
            self.ramp_changed = false;
            self.ramp_state = 2;
            let mut sq = [0f32; 4];
            for i in 0..4 {
                let s = (self.ramp_target[i] - self.ramp_cur[i]) * self.ramp_inv;
                self.ramp_step[i] = s;
                sq[i] = s * s;
            }
            for i in 0..4 {
                if f64::from(sq[i]) < f64::from_bits(0x3cd2_03af_9ee7_5616) {
                    self.ramp_step[i] = 0.0;
                    self.ramp_cur[i] = self.ramp_target[i];
                    sq[i] = 0.0;
                }
            }
            if ((sq[0] + sq[1]) + sq[2]) + sq[3] > 0.0 {
                self.ramp_left = self.ramp_len;
                self.ramping = true;
            } else {
                self.ramp_state = 4;
            }
            return;
        }
        self.ramp_left = self.ramp_left.wrapping_sub(1);
        if self.ramp_left == 0 {
            self.ramp_cur = self.ramp_target;
            self.ramp_step = [0.0; 4];
            self.ramping = false;
            self.ramp_state = 4;
        } else if self.ramp_state == 2 {
            self.ramp_state = 3;
        } else if self.ramp_state == 4 {
            self.ramp_state = 0;
        }
    }

    /// Advance the gain ramp one sample (the cores do this inline per sample).
    #[inline]
    pub(super) fn advance_ramp(&mut self) {
        for i in 0..4 {
            self.ramp_cur[i] += self.ramp_step[i];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_and_osc_are_finite() {
        let mut a = AttackCore::new();
        a.noise.amp = 1.0;
        a.noise.lcg.enabled = true;
        a.noise.vel_x1 = 1.0;
        a.noise.hp_freq = 500.0;
        a.noise.bp_freq = 4000.0;
        a.main.vel_x1 = 1.0;
        for o in &mut a.main.osc {
            o.enabled = true;
            o.freq = 1.5;
            o.amp = 1.0;
        }
        a.fluffy.set_sample_rate(48000.0);
        a.main.set_sample_rate(48000.0);
        a.noise.set_sample_rate(48000.0, 128);
        a.set_f0_from_body(400.0);
        a.set_note_on(0.8);
        let mut e = 0.0f64;
        for crash in [true, false] {
            for _ in 0..4800 {
                let x = a.main.process_sample(crash) + a.noise.process_sample();
                assert!(x.is_finite());
                e += f64::from(x * x);
            }
        }
        assert!(e > 0.0);
    }

    /// The whole object round-trips through its image.
    #[test]
    fn image_round_trip() {
        let a = AttackCore::new();
        let mut img = vec![0u8; 0x888];
        a.image(&mut img, 0);
        let b = AttackCore::get(&img, 0);
        let mut img2 = vec![0u8; 0x888];
        b.image(&mut img2, 0);
        assert_eq!(img, img2);
    }

    /// `walk` visits exactly what `image` writes.
    #[test]
    fn walk_matches_image() {
        let mut a = AttackCore::new();
        a.main.set_sample_rate(44100.0);
        let mut img = vec![0u8; 0x888];
        a.image(&mut img, 0);
        let mut img2 = vec![0u8; 0x888];
        a.walk(&mut |o, s| s.put(&mut img2, o));
        assert_eq!(img, img2);
    }

    /// Oracle: `CYMBAL_ORACLE_ATTACK=<dir>` holding `*.case` files from
    /// scratchpad/syn-cymbal/attack/oracle.c (Maschine's own AttackCore, started from a
    /// real CymbalProcessor's state): runs the same calls on the same starting state
    /// and compares every output and the final image bit for bit.
    #[test]
    #[ignore]
    fn oracle() {
        let Ok(dir) = std::env::var("CYMBAL_ORACLE_ATTACK") else { return };
        let dir = std::path::Path::new(&dir);
        let mut errs = Vec::new();
        // The constructor (bytes 0..8 are the vtable).
        for v in ["crash", "ride"] {
            let want = std::fs::read(dir.join(format!("ctor_{v}.bin"))).unwrap();
            let mut img = vec![0u8; 0x888];
            AttackCore::new().image(&mut img, 0);
            let mask: Vec<bool> = (0..0x888).map(|k| k >= 8).collect();
            if let Err(e) = diff(&format!("ctor {v}"), &img, &want, &mask) {
                errs.push(e);
            }
        }
        let (mut n, mut energy) = (0, 0.0f64);
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().and_then(|x| x.to_str()) == Some("case") {
                n += 1;
                match run_case(&p) {
                    Ok(e) => energy += e,
                    Err(e) => errs.push(e),
                }
            }
        }
        assert!(n > 0, "no cases in {dir:?}");
        assert!(errs.is_empty(), "{} of {n} cases (+2 ctors) differ:\n{}", errs.len(), errs.join("\n"));
        assert!(energy > 0.0, "the cases are silent");
    }

    fn rd_f32(b: &[u8], i: &mut usize) -> f32 {
        let v = f32::from_le_bytes(b[*i..*i + 4].try_into().unwrap());
        *i += 4;
        v
    }

    fn rd_u32(b: &[u8], i: &mut usize) -> u32 {
        let v = u32::from_le_bytes(b[*i..*i + 4].try_into().unwrap());
        *i += 4;
        v
    }

    fn diff(name: &str, got: &[u8], want: &[u8], mask: &[bool]) -> Result<(), String> {
        let bad: Vec<usize> = (0..want.len()).filter(|&k| mask[k] && got[k] != want[k]).collect();
        if bad.is_empty() {
            return Ok(());
        }
        let words: std::collections::BTreeSet<usize> = bad.iter().map(|k| k & !3).collect();
        let show: Vec<String> = words
            .iter()
            .take(12)
            .map(|&o| {
                let g = u32::from_le_bytes(got[o..o + 4].try_into().unwrap());
                let w = u32::from_le_bytes(want[o..o + 4].try_into().unwrap());
                format!("{o:#x}: {g:#010x} vs {w:#010x}")
            })
            .collect();
        Err(format!("{name}: {} words differ: {}", words.len(), show.join(", ")))
    }

    /// A case: `u32 crash`, the starting image (0x888 bytes), ops until 0, the final
    /// image. Ops: 1 sr (every part's setSampleRate + updateDelays, as the cores call
    /// them), 2 note-on, 3 freq ratio, 4 noise LP, 5 env dec2, 6 f0, 7 n samples (each
    /// followed by the oscillator's and the noise's output), 8 control rate.
    fn run_case(path: &std::path::Path) -> Result<f64, String> {
        const SZ: usize = 0x888;
        let b = std::fs::read(path).unwrap();
        let mut i = 0;
        let crash = rd_u32(&b, &mut i) != 0;
        let start = &b[i..i + SZ];
        i += SZ;
        let mut a = AttackCore::get(start, 0);
        // The bytes the struct models.
        let (mut p1, mut p2) = (vec![0xa5u8; SZ], vec![0x5au8; SZ]);
        a.put(&mut p1, 0);
        a.put(&mut p2, 0);
        let mask: Vec<bool> = p1.iter().zip(&p2).map(|(x, y)| x == y).collect();
        let mut img = vec![0u8; SZ];
        let mut energy = 0.0f64;
        loop {
            match rd_u32(&b, &mut i) {
                0 => break,
                1 => {
                    let sr = rd_f32(&b, &mut i);
                    a.main.set_sample_rate(sr);
                    a.fluffy.set_sample_rate(sr);
                    a.noise.set_sample_rate(sr, 128);
                    a.update_delays();
                }
                2 => a.set_note_on(rd_f32(&b, &mut i)),
                3 => a.set_modulation_freq_ph_osc_ratio(rd_f32(&b, &mut i)),
                4 => a.set_modulation_noise_lp(rd_f32(&b, &mut i)),
                5 => a.set_modulation_pnoise_env_dec2(rd_f32(&b, &mut i)),
                6 => a.set_f0_from_body(rd_f32(&b, &mut i)),
                7 => {
                    let n = rd_u32(&b, &mut i);
                    for k in 0..n {
                        a.advance_ramp();
                        let o = a.main.process_sample(crash);
                        let x = a.noise.process_sample();
                        let (wo, wx) = (rd_f32(&b, &mut i), rd_f32(&b, &mut i));
                        if o.to_bits() != wo.to_bits() || x.to_bits() != wx.to_bits() {
                            return Err(format!("{path:?} sample {k}: osc {o:e} vs {wo:e}, noise {x:e} vs {wx:e}"));
                        }
                        energy += f64::from(o * o + x * x);
                    }
                }
                8 => a.process_control_rate(),
                op => panic!("bad op {op}"),
            }
        }
        a.put(&mut img, 0);
        diff(&format!("{path:?} final"), &img, &b[i..i + SZ], &mask)?;
        Ok(energy)
    }
}
