//! `NI::DSP::UTIL::Air` (at `CymbalBodyCore +0x40a80`): the body's breath noise, a loop
//! of Maschine's pink-noise recording through a state-variable filter under an envelope,
//! fed to the resonators' second input. Also the small filters it shares with the
//! exciter and the body ([`Svf`], [`OnePole`]) and the velocity curve.

use super::super::memo::MemoEnvelope;
use super::super::res::{self, NOISE_LEN};
use super::img::layout;
use super::src::Src;
use crate::fx::exp_e;

pub(in super::super) const PI: f32 = std::f32::consts::PI;

/// `tan(x)` as Maschine's filters take it: `x + x³/3 + 0.1333·x⁵`.
#[inline]
pub(in super::super) fn tan3(x: f32) -> f32 {
    let x2 = x * x;
    let x3 = x * x2;
    let x5 = x2 * x3;
    x + x5.mul_add(f32::from_bits(0x3e08_8886), x3 * f32::from_bits(0x3eaa_aaaa))
}

/// Maschine's velocity curve (`lo, hi, c1, c2`): `v` mapped to 0..1 between `lo` and
/// `hi`, then bent twice by `x·(1 + c − c·|x|)`.
pub(in super::super) fn curve(v: f32, [lo, hi, c1, c2]: [f32; 4]) -> f32 {
    let hi = if hi > lo { hi } else { lo + 0.002 };
    let v = if lo > v { lo } else { v };
    let v = if v > hi { hi } else { v };
    let d = hi - lo;
    let d = if d < 1e-7 { f32::from_bits(0x33d6_bf95) } else { d };
    let mut x = (v - lo) / d;
    for c in [c1, c2] {
        x *= (-x.abs()).mul_add(c, c + 1.0);
    }
    let x = if x < 0.0 { 0.0 } else { x };
    if x > 1.0 { 1.0 } else { x }
}

/// `E(x)` clamped to 20..20000 Hz, from a pitch: `E(p·5 + 1381.88)`.
#[inline]
pub(in super::super) fn pitch_hz(p: f32) -> f32 {
    let f = exp_e(p.mul_add(5.0, f32::from_bits(0x44ac_bc36)));
    let f = if f < 20.0 { 20.0 } else { f };
    if f > 20000.0 { 20000.0 } else { f }
}

layout! {
    /// A TPT state-variable filter (0x38 bytes).
    Svf [0x38] {
        /// High-pass state.
        0x00 hp: f32,
        /// Band-pass state.
        0x04 band: f32,
        /// Low-pass state.
        0x08 low: f32,
        /// Outputs of the last sample.
        0x0c out_hp: f32,
        0x10 out_band: f32,
        0x14 out_low: f32,
        /// `g = tan(π·f/sr)`.
        0x18 g: f32,
        /// Damping `k`.
        0x1c k: f32,
        /// `g·(g + k)`.
        0x20 gk: f32,
        /// Cutoff (clamped to 0..0.49·sr; −1 = unset).
        0x24 freq: f32,
        /// `1/(1 + g·(g + k))`.
        0x28 a: f32,
        /// Resonance parameter.
        0x2c q: f32,
        0x30 sr: f32,
        0x34 inv_sr: f32,
    }
}

impl Svf {
    pub fn new() -> Svf {
        Svf {
            hp: 0.0,
            band: 0.0,
            low: 0.0,
            out_hp: 0.0,
            out_band: 0.0,
            out_low: 0.0,
            g: 0.0,
            k: 2.0,
            gk: 0.0,
            freq: 1.0,
            a: 1.0,
            q: 0.0,
            sr: 1.0,
            inv_sr: 1.0,
        }
    }

    /// The cutoff: `g` from `f` itself, the stored cutoff clamped.
    pub fn set_freq(&mut self, f: f32) {
        let lim = self.sr * f32::from_bits(0x3efa_e148);
        let c = if f < 0.0 { 0.0 } else { f };
        self.freq = if c > lim { lim } else { c };
        self.g = tan3(self.inv_sr * (f * PI));
        self.gk = self.g * (self.g + self.k);
        self.a = 1.0 / (self.gk + 1.0);
    }

    /// The resonance: `k = 2(1 − max(q, −2))`, 0.02 above 0.99.
    pub fn set_q(&mut self, q: f32) {
        let c = if q < -2.0 { -2.0 } else { q };
        self.k = if c > f32::from_bits(0x3f7d_70a4) { f32::from_bits(0x3ca3_d700) } else { (1.0 - c) + (1.0 - c) };
        self.gk = self.g * (self.k + self.g);
        self.a = 1.0 / (self.gk + 1.0);
        self.q = q;
    }

    /// One sample; returns `(hp, band, low)`.
    #[inline]
    pub fn tick(&mut self, x: f32) -> (f32, f32, f32) {
        let s = self.band * self.g.mul_add(2.0, self.k);
        let s = self.low + self.hp.mul_add(self.gk, s);
        let hp = self.a * (x - s);
        let band = (self.hp + hp).mul_add(self.g, self.band);
        let low = (self.band + band).mul_add(self.g, self.low);
        self.out_hp = hp;
        self.out_band = band;
        self.out_low = low;
        self.hp = hp;
        self.band = band;
        self.low = low;
        (hp, band, low)
    }
}

layout! {
    /// A TPT one-pole filter (0x20 bytes).
    OnePole [0x20] {
        /// State.
        0x00 s: f32,
        0x04 pad: f32,
        /// Output of the last sample.
        0x08 out: f32,
        /// `1/(1 + g)`.
        0x0c a: f32,
        /// `g − 1`.
        0x10 gm1: f32,
        /// `g = tan(π·f/sr)`.
        0x14 g: f32,
        /// Cutoff (−1 = unset).
        0x18 freq: f32,
        0x1c inv_sr: f32,
    }
}

impl OnePole {
    pub fn new() -> OnePole {
        OnePole { s: 0.0, pad: 0.0, out: 0.0, a: 1.0, gm1: 0.0, g: 0.0, freq: 1.0, inv_sr: 1.0 }
    }

    pub fn set_freq(&mut self, f: f32) {
        self.g = tan3(self.inv_sr * (f * PI));
        self.freq = f;
        self.a = 1.0 / (self.g + 1.0);
        self.gm1 = self.g + -1.0;
    }

    /// `setSampleRate` as inlined: the cutoff again for `inv_sr`, state cleared.
    pub fn set_inv_sr(&mut self, inv: f32) {
        self.inv_sr = inv;
        let f = self.freq;
        self.freq = -1.0;
        if f != -1.0 {
            self.set_freq(f);
        }
        self.s = 0.0;
        self.pad = 0.0;
        self.out = 0.0;
    }

    /// High-pass sample.
    #[inline]
    pub fn hp(&mut self, x: f32) -> f32 {
        let y = self.a * (-self.gm1).mul_add(self.s, x);
        let o = y - self.s;
        self.out = o;
        self.s = y;
        o
    }
}

/// The pink-noise loop's samples: Maschine's recording, or its resampled copy.
#[derive(Clone, Debug, Default)]
pub(in super::super) enum Pink {
    #[default]
    Recording,
    Resampled(Vec<f32>),
}

layout! {
    /// `Air` (0x1b4 bytes).
    Air [0x1b4] {
        0x00 u00: f32,
        /// Output gain.
        0x04 gain: f32,
        0x08 u08: [f32; 6],
        /// Filter pitch (`setParamAirFiltFreq`, 15..135).
        0x20 pitch: f32,
        /// Filter cutoff from pitch and modulation (Hz).
        0x24 cutoff: f32,
        /// Glide time of the cutoff modulation (ms).
        0x28 glide_ms: i32,
        /// Filter resonance.
        0x2c q: f32,
        0x30 u30: [f32; 4],
        /// Output mix: low, band (doubled), high.
        0x40 mix: [f32; 3],
        /// Velocity curve.
        0x4c curve: [f32; 4],
        0x5c u5c: f32,
        /// Cutoff modulation (pitch).
        0x60 freq_mod: f32,
        /// Filter pitch in use.
        0x64 cur_pitch: f32,
        0x68 u68: f32,
        0x6c u6c: f32,
        0x70 sr: f32,
        /// Control block length.
        0x74 block: i32,
        /// `PinkSampler` +0x00: restart the loop on notes.
        0x78 restart: bool,
        /// `PinkSampler` +0x04.
        0x7c pink_sr: f32,
        /// `PinkSampler` +0x08..+0x18: data pointers (current, recording, resampled).
        "heap" pink: Pink,
        /// `PinkSampler` +0x20: read position.
        0x98 pos: u32,
        /// `PinkSampler` +0x24: `sr/44100`.
        0x9c ratio: f32,
        /// `PinkSampler` +0x28: start position as a fraction of the loop.
        0xa0 start: f32,
        /// `PinkSampler` +0x2c: start position.
        0xa4 start_pos: i32,
        0xa8 env: MemoEnvelope,
        0x128 svf: Svf,
        0x160 u160: [f32; 4],
        /// Cutoff-modulation glide: current values, increments, targets (lane 0 used).
        0x170 glide_cur: [f32; 4],
        0x180 glide_inc: [f32; 4],
        0x190 glide_target: [f32; 4],
        /// A new target is pending.
        0x1a0 glide_pending: bool,
        0x1a1 u1a1: bool,
        /// Blocks left in the glide.
        0x1a4 glide_left: i32,
        /// Glide length in blocks.
        0x1a8 glide_len: i32,
        /// `1/(glide_len·block)`.
        0x1ac glide_inv: f32,
        /// 1 idle, 2 starting, 3 gliding, 4 done.
        0x1b0 glide_state: u32,
    }
}

impl Air {
    /// `Air()` (0x1cd3ab0).
    pub fn new() -> Air {
        let mut env = MemoEnvelope::default();
        env.sr = 1.0;
        env.inv_sr = 1.0;
        let mut svf = Svf::new();
        svf.freq = 1.0;
        Air {
            u00: 0.0,
            gain: 0.0,
            u08: [0.0; 6],
            pitch: 0.0,
            cutoff: 0.0,
            glide_ms: 1,
            q: 0.0,
            u30: [0.0; 4],
            mix: [0.0; 3],
            curve: [0.0; 4],
            u5c: 0.0,
            freq_mod: 0.0,
            cur_pitch: 0.0,
            u68: 0.0,
            u6c: 1.0,
            sr: 0.0,
            block: 0,
            restart: false,
            pink_sr: 0.0,
            pink: Pink::Recording,
            pos: 0,
            ratio: 1.0,
            start: 0.0,
            start_pos: 0,
            env,
            svf,
            u160: [0.0; 4],
            glide_cur: [0.0; 4],
            glide_inc: [0.0; 4],
            glide_target: [0.0; 4],
            glide_pending: true,
            u1a1: false,
            glide_left: 0,
            glide_len: 0,
            glide_inv: 0.0,
            glide_state: 1,
        }
    }

    fn samples(&self) -> &[f32] {
        static ZEROS: [f32; NOISE_LEN] = [0.0; NOISE_LEN];
        match &self.pink {
            Pink::Recording => res::pink().unwrap_or(&ZEROS),
            Pink::Resampled(v) => v,
        }
    }

    /// `PinkSampler::setSampleRate(sr)` (0x1cd3f24): the recording resampled to `sr` once.
    fn set_pink_rate(&mut self, sr: f32) {
        if !(1.0..=500_000.0).contains(&sr) {
            return;
        }
        self.pink_sr = sr;
        let ratio = if sr == res::WAV_RATE {
            self.pink = Pink::Recording;
            1.0
        } else {
            let rec: Vec<f32> = match res::pink() {
                Some(p) => p.to_vec(),
                None => vec![0.0; NOISE_LEN],
            };
            self.pink = Pink::Resampled(Src::resample(res::WAV_RATE, sr, &rec));
            sr / res::WAV_RATE
        };
        self.ratio = ratio;
        self.start_pos = (self.start * 131_072.0 * ratio).round() as i32;
    }

    /// `setSampleRate(sr, block)` (0x1cd3c90).
    pub fn set_sample_rate(&mut self, sr: f32, block: i32) {
        if !(1.0..=500_000.0).contains(&sr) {
            return;
        }
        self.sr = sr;
        self.block = block;
        self.set_pink_rate(sr);
        self.env.set_sample_rate(sr);
        self.svf.sr = sr;
        self.svf.inv_sr = 1.0 / sr;
        let old = self.svf.freq;
        self.svf.freq = -1.0;
        if old != -1.0 {
            self.svf.set_freq(old);
        }
        if self.svf.freq != self.cutoff {
            self.svf.set_freq(self.cutoff);
        }
        let n = ((self.sr * (self.glide_ms as f32 / 1000.0)) / block as f32 + 0.5) as i32;
        let n = if n > 1 { n } else { 1 };
        self.glide_len = n;
        self.glide_inv = 1.0 / (n.wrapping_mul(block)) as f32;
    }

    /// The cutoff for pitch `p`, if it changed.
    fn retune(&mut self, p: f32) {
        let f = pitch_hz(p);
        if self.svf.freq != f {
            self.svf.set_freq(f);
        }
    }

    /// `setFiltFreqModulation(m)` (0x1cd4374): at once if `m ≥ 0`, else as a glide.
    pub fn set_filt_freq_modulation(&mut self, m: f32) {
        self.freq_mod = m;
        if m < 0.0 {
            self.glide_target[0] = m;
            self.glide_pending = true;
            self.glide_state = 1;
            return;
        }
        self.glide_inc[0] = 0.0;
        self.glide_target[0] = m;
        self.glide_cur[0] = m;
        let f = exp_e((self.pitch + m).mul_add(5.0, f32::from_bits(0x44ac_bc36)));
        self.cutoff = f;
        let f = if f < 20.0 { 20.0 } else { f };
        let f = if f > 20000.0 { 20000.0 } else { f };
        if self.svf.freq != f {
            self.svf.set_freq(f);
        }
    }

    /// `setNoteOn(v)` (0x1cd44d4).
    pub fn set_note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        let x = curve(v, self.curve);
        if x <= 0.0 {
            return;
        }
        if self.restart {
            self.pos = self.start_pos as u32;
        }
        self.env.set_note_on(x);
    }

    /// `processSample()` (0x1cd4848).
    #[inline]
    pub fn process(&mut self) -> f32 {
        let x = self.samples()[self.pos as usize];
        self.pos += 1;
        if self.ratio * 131_072.0 <= self.pos as f32 {
            self.pos = 0;
        }
        for i in 0..4 {
            self.glide_cur[i] += self.glide_inc[i];
        }
        if self.glide_inc[0] != 0.0 {
            let p = self.glide_cur[0] + self.pitch;
            self.cur_pitch = p;
            self.retune(p);
        }
        let (hp, band, low) = self.svf.tick(x);
        let e = self.env.process();
        (((self.mix[0] * low) + ((band + band) * self.mix[1])) + (hp * self.mix[2])) * e * self.gain
    }
}
