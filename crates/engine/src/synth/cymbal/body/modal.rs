//! `NI::DSP::UTIL::ModModalSynth`: the cymbal body's bank of 512 two-pole resonators
//! ("partials"), four per NEON vector (docs/synth-cymbal.md §5.3).
//!
//! Each partial `p` is a resonator at `f = max(ratio·pitch, 10)` Hz with decay time
//! from `damping`; its two inputs (the exciter and the air noise) are mixed in, and its
//! band-passed output (`y − y[n−2]`) is summed to left and right with per-partial
//! amplitudes that glide (linear ramps of `seg_len` samples) to their targets. The bank
//! runs in blocks of 16 samples (`processAudio_optimized`), the newest sample last in
//! memory order and processed first (indices count down).
//!
//! Layout as in Maschine: [`Part`] is a 0x120-byte partial group (`ModalSynthData`
//! `+0x10` pointer), [`Coef`] a 0x90-byte coefficient/state block at
//! `data + 0x20 + g·0x90`, [`Voice`] the 0x1c-byte voice record. One voice.

/// Four lanes, as Maschine's `v4f`.
pub(in super::super) type V4 = [f32; 4];

/// Partials (`ModModalSynth +0x0c`, `0x200` for the cymbal).
pub(in super::super) const PARTIALS: usize = 512;
/// Partial groups allocated (`(PARTIALS + 4)/4`).
pub(in super::super) const GROUPS: usize = (PARTIALS + 4) / 4;

#[inline]
fn splat(x: f32) -> V4 {
    [x; 4]
}
#[inline]
fn mul(a: V4, b: V4) -> V4 {
    std::array::from_fn(|i| a[i] * b[i])
}
#[inline]
fn add(a: V4, b: V4) -> V4 {
    std::array::from_fn(|i| a[i] + b[i])
}
#[inline]
fn sub(a: V4, b: V4) -> V4 {
    std::array::from_fn(|i| a[i] - b[i])
}
/// `fmla`: `acc + a·b`, fused.
#[inline]
fn fma(acc: V4, a: V4, b: V4) -> V4 {
    std::array::from_fn(|i| a[i].mul_add(b[i], acc[i]))
}
/// `fmls`: `acc − a·b`, fused.
#[inline]
fn fms(acc: V4, a: V4, b: V4) -> V4 {
    std::array::from_fn(|i| (-a[i]).mul_add(b[i], acc[i]))
}
#[inline]
fn vmin(a: V4, b: V4) -> V4 {
    std::array::from_fn(|i| fmin(a[i], b[i]))
}
#[inline]
fn vmax(a: V4, b: V4) -> V4 {
    std::array::from_fn(|i| fmax(a[i], b[i]))
}
/// ARM `fmin` (NaN propagates; −0 < +0).
#[inline]
fn fmin(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == b {
        if a.is_sign_negative() { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}
/// ARM `fmax`.
#[inline]
fn fmax(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == b {
        if a.is_sign_positive() { a } else { b }
    } else if a > b {
        a
    } else {
        b
    }
}

/// ARM `FRECPE` (single precision, round to nearest, no flush-to-zero): the 8-bit
/// reciprocal estimate.
pub(in super::super) fn frecpe(x: f32) -> f32 {
    let bits = x.to_bits();
    let sign = bits & 0x8000_0000;
    if x.is_nan() {
        return f32::from_bits(bits | 0x0040_0000);
    }
    if x.is_infinite() {
        return f32::from_bits(sign);
    }
    if x == 0.0 {
        return f32::from_bits(sign | 0x7f80_0000);
    }
    if x.abs() < f32::from_bits(0x0010_0000) {
        // |x| < 2^-128: overflows (round to nearest → infinity).
        return f32::from_bits(sign | 0x7f80_0000);
    }
    let mut exp = ((bits >> 23) & 0xff) as i32;
    // 52-bit fraction as in the pseudocode (fraction<51:29> = operand<22:0>).
    let mut frac = u64::from(bits & 0x007f_ffff) << 29;
    if exp == 0 {
        if frac & (1 << 51) == 0 {
            exp = -1;
            frac = (frac << 2) & ((1 << 52) - 1);
        } else {
            frac = (frac << 1) & ((1 << 52) - 1);
        }
    }
    let scaled = (1u64 << 8) | ((frac >> 44) & 0xff);
    let mut rexp = 253 - exp;
    // RecipEstimate (not increased precision).
    let a = scaled * 2 + 1;
    let b = (1u64 << 19) / a;
    let est = (b + 1) / 2;
    let mut rfrac = (est & 0xff) << 44;
    if rexp == 0 {
        rfrac = (1 << 51) | (rfrac >> 1);
    } else if rexp == -1 {
        rfrac = (1 << 50) | (rfrac >> 2);
        rexp = 0;
    }
    f32::from_bits(sign | ((rexp as u32 & 0xff) << 23) | ((rfrac >> 29) as u32 & 0x007f_ffff))
}

/// ARM `FRECPS`: `2 − a·b`, fused (`inf·0` gives 2).
#[inline]
pub(in super::super) fn frecps(a: f32, b: f32) -> f32 {
    if (a.is_infinite() && b == 0.0) || (a == 0.0 && b.is_infinite()) {
        return 2.0;
    }
    (-a).mul_add(b, 2.0)
}

/// `1/d` the way Maschine's vector code takes it: an estimate refined twice.
#[inline]
pub(in super::super) fn recip(d: V4) -> V4 {
    std::array::from_fn(|i| {
        let r0 = frecpe(d[i]);
        let r1 = r0 * frecps(d[i], r0);
        frecps(d[i], r1) * r1
    })
}

layout! {
    /// A partial group (four partials) in the data block (0x120 bytes, at +0x4820).
    Part [0x120] {
        /// Frequency ratios.
        0x00 ratio: V4,
        /// Frequency in cycles per sample (`freq/sr`).
        0x10 fnorm: V4,
        /// Left and right amplitudes.
        0x20 amp_l: V4,
        0x30 amp_r: V4,
        /// Amplitudes as set (`setPartialAmpL/R` write these and +0x20/+0x30).
        0x40 set_l: V4,
        0x50 set_r: V4,
        /// High-frequency gain (`1 − scale·(clamp(fnorm) − lo)`, unused lanes 0).
        0x60 hf: V4,
        /// Gain targets (`hf·amp`).
        0x70 tgt_l: V4,
        0x80 tgt_r: V4,
        /// Gain ramp steps.
        0x90 step_l: V4,
        0xa0 step_r: V4,
        /// Input gain (`setPartialInputGain`; the cymbal's amplitude curve).
        0xb0 in_gain: V4,
        /// Frequency in Hz (`max(ratio·pitch, 10)`).
        0xc0 freq: V4,
        /// Decay as set (the cymbal's decay curve; the air input's reference).
        0xd0 damp_ref: V4,
        /// Bandwidth term `min((1 + t²)·ln2/(t·decay·sr), 2)`.
        0xe0 bw: V4,
        /// Decay time.
        0xf0 decay: V4,
        /// `t ≈ tan(π·fnorm)` (rational approximation, capped at π·0.48).
        0x100 tan: V4,
        /// Decay time before damping.
        0x110 time: V4,
    }
}

layout! {
    /// A group's resonator coefficients and state (0x90 bytes, at `+0x20 + g·0x90`).
    Coef [0x90] {
        /// Input scale (the cymbal's amplitude curve).
        0x00 in_gain: V4,
        /// Gain of the second input (the air noise).
        0x10 in2: V4,
        /// `b0 = t/(1 + t² + t·bw)`.
        0x20 b: V4,
        /// `a2 = (1 + t² − t·bw)/(1 + t² + t·bw)`.
        0x30 a2: V4,
        /// `a1 = 2(1 − t²)/(1 + t² + t·bw)`.
        0x40 a1: V4,
        /// `y[n−1]`, `y[n−2]`.
        0x50 y1: V4,
        0x60 y2: V4,
        /// Current output gains.
        0x70 gain_l: V4,
        0x80 gain_r: V4,
    }
}

layout! {
    /// The voice record (0x1c bytes, at +0xd940).
    Voice [0x1c] {
        /// Active groups and partials.
        0x00 groups: u32,
        0x04 partials: u32,
        /// Requested active partials (low half) and the maximum (high half).
        0x08 limits: u32,
        /// The selected partial, `index·0x10001` (`setSelectedPartial`).
        0x0c selected: u32,
        /// Hz per unit ratio.
        0x10 pitch: f32,
        /// Coefficients need recalculating.
        0x14 dirty: bool,
        /// Gains jump to their targets (no ramp).
        0x15 reset: bool,
        /// Damping (`setDamping`).
        0x18 damping: f32,
    }
}

/// Coefficient blocks, partial groups and the voice in the data block.
pub(in super::super) const COEF_AT: usize = 0x20;
pub(in super::super) const PART_AT: usize = 0x4820;
pub(in super::super) const VOICE_AT: usize = 0xd940;
/// Bytes of the data block (`allocatePartialsData(1)` for 512 partials).
pub(in super::super) const DATA_SIZE: usize = 0xd95c;

layout! {
    /// `NI::DSP::UTIL::ModModalSynth` (at `CymbalBodyCore +0x409b0`, 0xd0 bytes) with its
    /// `ModalSynthData` block (heap). +0x00 vtable, +0x38 the smoothing record's address
    /// (+0x40), +0x90/+0x98/+0xa0 the data block, its groups and its voice.
    ModalSynth [0xd0] {
        0x08 u08: u32,
        /// Partials (512).
        0x0c partials: u32,
        0x10 sr: f32,
        0x14 inv_sr: f32,
        0x18 u18: u32,
        /// Gain-update period in samples (`s_aSmoothingQualityPeriod[0]`).
        0x1c period: u32,
        /// Gain glide time in ms.
        0x20 glide_ms: f32,
        /// Blocks since the last gain update.
        0x24 tick: u32,
        /// The period in use.
        0x28 seg_period: u32,
        /// Gain updates done in this glide, and per glide.
        0x2c seg: u32,
        0x30 steps: u32,
        /// The glide length must be recomputed.
        0x34 glide_dirty: bool,
        /// Some voice needs its coefficients recalculated.
        0x35 dirty: bool,
        /// Smoothing record +0x00/+0x10/+0x20: high-frequency rolloff, the clamp range
        /// of `fnorm` and its slope.
        0x40 hf_lo: V4,
        0x50 hf_hi: V4,
        0x60 hf_scale: V4,
        /// Smoothing record +0x30: `1/steps`.
        0x70 inv_steps: V4,
        /// Data +0x00: voices (1).
        "heap" voices: u32,
        /// Data +0x04: groups per voice.
        "heap" groups_per_voice: u32,
        /// Data +0x08: partials allocated.
        "heap" data_partials: u32,
        "heap" part: Vec<Part>,
        "heap" coef: Vec<Coef>,
        "heap" voice: Voice,
    }
}

impl ModalSynth {
    /// The bank as `BodyCore()` leaves it (`allocatePartialsData(1)`,
    /// `initPartialsData`, its 32-partial voice set-up), before `CymbalBodyCore()`.
    pub fn new() -> ModalSynth {
        let partials = PARTIALS as u32;
        let mut part = vec![Part::default(); GROUPS];
        let mut coef = vec![Coef::default(); GROUPS - 1];
        for (g, p) in part.iter_mut().enumerate().take(GROUPS - 1) {
            let b = (g as u32) * 16;
            p.ratio = std::array::from_fn(|i| (b | (i as u32 + 1)) as i32 as f32);
            p.hf = splat(1.0);
            p.in_gain = splat(1.0);
            p.freq = splat(10.0);
            p.damp_ref = splat(60.0);
        }
        for c in &mut coef {
            c.in_gain = splat(1.0);
        }
        ModalSynth {
            u08: 0,
            partials,
            sr: 0.0,
            inv_sr: 0.0,
            u18: 0,
            period: 1,
            glide_ms: 10.0,
            tick: 0x7fff_ffff,
            seg_period: 0,
            seg: 0x7fff_ffff,
            steps: 0,
            glide_dirty: true,
            dirty: true,
            hf_lo: [0.0; 4],
            hf_hi: [0.0; 4],
            hf_scale: [0.0; 4],
            inv_steps: [0.0; 4],
            voices: 1,
            groups_per_voice: partials.div_ceil(4),
            data_partials: partials,
            part,
            coef,
            voice: Voice::default(),
        }
    }

    /// The voice set-up `BodyCore()` and `CymbalBodyCore()` inline: `limit` active
    /// partials, all amplitudes 1 (`setPartialAmpL/R` with each partial selected).
    pub fn init_voice(&mut self, limit: u32) {
        let v = &mut self.voice;
        v.pitch = 10.0;
        v.damping = 0.0;
        v.dirty = true;
        v.reset = true;
        v.groups = self.partials.div_ceil(4);
        v.partials = self.partials;
        self.u18 = 0;
        self.period = 1;
        self.glide_ms = 10.0;
        self.glide_dirty = true;
        self.dirty = true;
        let req = limit.min(0xffff);
        self.voice.limits = req.min(self.data_partials) | (req << 16);
        for i in 0..limit as usize {
            if i as u32 <= self.data_partials {
                self.voice.selected = i as u32 * 0x10001;
            }
            self.set_amp_l(i, 1.0);
            self.set_amp_r(i, 1.0);
        }
    }

    /// Walk the data block (offsets from its start).
    #[cfg(test)]
    pub fn walk_data(&mut self, v: &mut dyn FnMut(usize, super::super::attached::Slot)) {
        use super::super::attached::Slot;
        use super::img::{Walk, walk_vec};
        v(0, Slot::U(&mut self.voices));
        v(4, Slot::U(&mut self.groups_per_voice));
        v(8, Slot::U(&mut self.data_partials));
        walk_vec(&mut self.coef, COEF_AT, v);
        walk_vec(&mut self.part, PART_AT, v);
        self.voice.walk(VOICE_AT, v);
    }

    /// Partial `i`'s left amplitude (`setPartialAmpL`: both +0x40 and +0x20).
    pub fn set_amp_l(&mut self, i: usize, a: f32) {
        let p = &mut self.part[i >> 2];
        p.set_l[i & 3] = a;
        p.amp_l[i & 3] = a;
    }

    /// Partial `i`'s right amplitude (`setPartialAmpR`).
    pub fn set_amp_r(&mut self, i: usize, a: f32) {
        let p = &mut self.part[i >> 2];
        p.set_r[i & 3] = a;
        p.amp_r[i & 3] = a;
    }

    /// `reset(voice, …)` as inlined: coefficients recalculate and gains jump.
    pub fn reset(&mut self) {
        self.voice.reset = true;
        self.dirty = true;
    }

    /// `updateSampleRate()` (0x1cd6e18): frequencies and the `tan` term for `sr`
    /// (`sr`/`inv_sr` set by the caller).
    pub fn update_sample_rate(&mut self) {
        let n = self.voice.groups as usize;
        let pitch = self.voice.pitch;
        let ten = splat(10.0);
        // (setPartialRatioVector for each group, then the same again with the tan term.)
        let pi = splat(std::f32::consts::PI);
        let cap = splat(f32::from_bits(0x3fc1_04d5));
        let (c2, c3, c4) = (f32::from_bits(0xbdb7_a1a8), f32::from_bits(0x3d1f_1c25), f32::from_bits(0x3f80_01a3));
        let (c5, c6) = (f32::from_bits(0xbedc_9b20), f32::from_bits(0x3d25_9bd1));
        for p in &mut self.part[..n] {
            p.freq = vmax(mul(p.ratio, splat(pitch)), ten);
            p.fnorm = mul(p.freq, splat(self.inv_sr));
            let v = vmin(mul(p.fnorm, pi), cap);
            let num = mul(v, add(mul(v, add(mul(v, splat(c2)), splat(c3))), splat(c4)));
            let den = add(mul(v, add(mul(v, splat(c5)), splat(c6))), splat(1.0));
            p.tan = mul(recip(den), num);
        }
    }

    /// `setDamping(voice 0, d)` (0x1cd76e0): every partial's decay from its time and
    /// `2^(−d·0.1660964)`.
    pub fn set_damping(&mut self, d: f32) {
        self.voice.damping = d;
        let x = d * f32::from_bits(0x3e2a_152d);
        let r = x.round_ties_even();
        let i = if r < 2_147_483_648.0 && r > -2_147_483_648.0 { r as i32 } else { i32::MIN };
        if self.voice.groups < 1 {
            return;
        }
        let f = x - i as f32;
        let p = f.mul_add(f32::from_bits(0x3d65_6042), f32::from_bits(0x3e77_ced9));
        let p = p.mul_add(f, f32::from_bits(0x3f31_6873));
        let p = p.mul_add(f, 1.0);
        let inv = 1.0 / (p * f32::from_bits(0x3f80_0000u32.wrapping_add((i as u32) << 23)));
        let k = splat(f32::from_bits(0x3a35_b52e));
        let n = self.voice.groups as usize;
        for p in &mut self.part[..n] {
            p.decay = mul(mul(p.time, splat(inv)), k);
        }
    }

    /// `calcParamsOptimized(0)` (0x1d9e8d0): resonator coefficients from frequency,
    /// decay and damping.
    fn calc_params(&mut self) {
        let v = &mut self.voice;
        let mut active = (v.limits & 0xffff).min(self.data_partials);
        if active != v.partials {
            v.partials = active;
            v.groups = (active + 3) >> 2;
        } else {
            active = v.partials;
        }
        let n = v.groups as usize;
        let damping = v.damping;
        let ln2 = splat(f32::from_bits(0x3f31_7215));
        let one = splat(1.0);
        for g in 0..n {
            let p = &mut self.part[g];
            let t = p.tan;
            let t2 = mul(t, t);
            let q = add(t2, one);
            let num = mul(q, ln2);
            let den = mul(t, mul(p.decay, splat(self.sr)));
            p.bw = vmin(mul(num, recip(den)), splat(2.0));
            let tb = mul(t, p.bw);
            let r = recip(add(q, tb));
            let c = &mut self.coef[g];
            c.a2 = mul(sub(q, tb), r);
            let d = sub(one, t2);
            c.a1 = mul(add(d, d), r);
            c.b = mul(t, r);
            let dd = sub(p.damp_ref, splat(damping));
            let x = add(mul(dd, splat(-6200.0)), splat(420_976.0));
            let x = add(x, mul(mul(dd, dd), splat(31.0)));
            c.in2 = mul(x, recip(mul(p.freq, splat(3000.0))));
            let f = sub(vmin(vmax(self.hf_lo, p.fnorm), self.hf_hi), self.hf_lo);
            p.hf = sub(one, mul(self.hf_scale, f));
        }
        let rem = active & 3;
        if rem != 0 && n > 0 {
            let p = &mut self.part[n - 1];
            for l in rem as usize..4 {
                p.hf[l] = 0.0;
            }
        }
    }

    /// `processAudio_optimized<true>(1, in1, in2, outL, outR, n)` (0x1ce9b20): `n` samples,
    /// processed from index `n − 1` down to 0.
    pub fn process(&mut self, in1: &[f32], in2: &[f32], out_l: &mut [f32], out_r: &mut [f32], n: usize) {
        if self.dirty {
            self.dirty = false;
            if self.voice.dirty {
                self.voice.dirty = false;
                self.calc_params();
                if self.voice.reset {
                    self.voice.reset = false;
                    for g in 0..self.voice.groups as usize {
                        let (p, c) = (&mut self.part[g], &mut self.coef[g]);
                        c.y1 = [0.0; 4];
                        c.y2 = [0.0; 4];
                        p.tgt_l = mul(p.hf, p.amp_l);
                        p.tgt_r = mul(p.hf, p.amp_r);
                        c.gain_l = p.tgt_l;
                        c.gain_r = p.tgt_r;
                        p.step_l = [0.0; 4];
                        p.step_r = [0.0; 4];
                    }
                }
            }
        }
        self.tick += 1;
        if self.tick >= self.seg_period {
            self.tick = 0;
            self.seg += 1;
            if self.seg >= self.steps {
                self.seg = 0;
                if self.glide_dirty {
                    self.glide_dirty = false;
                    let period = self.period;
                    let x = self.glide_ms * f32::from_bits(0x3a83_126f) * self.sr;
                    let r = (f64::from(x) + if x < 0.0 { -0.5 } else { 0.5 }) as i32;
                    self.seg_period = period;
                    let steps = (r + (period >> 1) as i32) / period as i32;
                    self.steps = steps as u32;
                    self.inv_steps = splat(1.0 / steps as u32 as f32);
                }
                for g in 0..self.voice.groups as usize {
                    let (p, c) = (&mut self.part[g], &self.coef[g]);
                    p.tgt_l = mul(p.hf, p.amp_l);
                    p.step_l = mul(self.inv_steps, sub(p.tgt_l, c.gain_l));
                    p.tgt_r = mul(p.hf, p.amp_r);
                    p.step_r = mul(self.inv_steps, sub(p.tgt_r, c.gain_r));
                }
            }
            for g in 0..self.voice.groups as usize {
                let (p, c) = (&self.part[g], &mut self.coef[g]);
                c.gain_l = add(c.gain_l, p.step_l);
                c.gain_r = add(c.gain_r, p.step_r);
            }
        }
        let groups = self.voice.groups as usize;
        let k = splat(f32::from_bits(0x3ea0_d973));
        for i in (0..n).rev() {
            let (x1, x2) = (in1[i], splat(in2[i]));
            let (mut l, mut r) = ([0.0f32; 4], [0.0f32; 4]);
            for c in self.coef[..groups].iter_mut().rev() {
                let x = mul(c.in_gain, fma(mul(x2, c.in2), k, splat(x1)));
                let y = fms(fma(mul(c.y1, c.a1), c.b, x), c.y2, c.a2);
                let o = sub(y, c.y2);
                c.y2 = c.y1;
                c.y1 = y;
                l = fma(l, c.gain_l, o);
                r = fma(r, c.gain_r, o);
            }
            out_l[i] = (l[0] + l[1]) + (l[2] + l[3]);
            out_r[i] = (r[0] + r[1]) + (r[2] + r[3]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frecpe_matches_hardware() {
        #[cfg(target_arch = "aarch64")]
        {
            use std::arch::aarch64::vrecpes_f32;
            let mut x = 1e-30f32;
            while x < 1e30 {
                for s in [x, -x, x * 1.37, x * 1.999] {
                    let hw = unsafe { vrecpes_f32(s) };
                    assert_eq!(frecpe(s).to_bits(), hw.to_bits(), "{s}");
                }
                x *= 1.013;
            }
        }
        assert_eq!(frecpe(1.0), 0.998_046_9);
    }
}

#[cfg(test)]
pub(in super::super) mod oracle {
    use super::*;

    fn f(b: &[u8], o: usize) -> f32 {
        f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    }
    fn u(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    }
    fn v(b: &[u8], o: usize) -> V4 {
        std::array::from_fn(|i| f(b, o + 4 * i))
    }

    /// A bank from Maschine's memory: the `ModModalSynth` object, its data block and
    /// the smoothing record.
    pub fn load(m: &[u8], d: &[u8], s: &[u8]) -> ModalSynth {
        let mut x = ModalSynth::new();
        x.period = u(m, 0x1c);
        x.partials = u(m, 0xc);
        x.sr = f(m, 0x10);
        x.inv_sr = f(m, 0x14);
        x.glide_ms = f(m, 0x20);
        x.tick = u(m, 0x24);
        x.seg_period = u(m, 0x28);
        x.seg = u(m, 0x2c);
        x.steps = u(m, 0x30);
        x.glide_dirty = m[0x34] != 0;
        x.dirty = m[0x35] != 0;
        (x.hf_lo, x.hf_hi, x.hf_scale, x.inv_steps) = (v(s, 0), v(s, 0x10), v(s, 0x20), v(s, 0x30));
        x.groups_per_voice = u(d, 4);
        x.data_partials = u(d, 8);
        let (po, vo) = (0x4820, 0xd940);
        for g in 0..GROUPS {
            let c = g * 0x90;
            x.coef[g] = Coef {
                in_gain: v(d, c + 0x20),
                in2: v(d, c + 0x30),
                b: v(d, c + 0x40),
                a2: v(d, c + 0x50),
                a1: v(d, c + 0x60),
                y1: v(d, c + 0x70),
                y2: v(d, c + 0x80),
                gain_l: v(d, c + 0x90),
                gain_r: v(d, c + 0xa0),
            };
            let p = po + g * 0x120;
            let w = |k| v(d, p + k);
            x.part[g] = Part {
                ratio: w(0),
                fnorm: w(0x10),
                amp_l: w(0x20),
                amp_r: w(0x30),
                set_l: w(0x40),
                set_r: w(0x50),
                hf: w(0x60),
                tgt_l: w(0x70),
                tgt_r: w(0x80),
                step_l: w(0x90),
                step_r: w(0xa0),
                in_gain: w(0xb0),
                freq: w(0xc0),
                damp_ref: w(0xd0),
                bw: w(0xe0),
                decay: w(0xf0),
                tan: w(0x100),
                time: w(0x110),
            };
        }
        x.voice = Voice {
            groups: u(d, vo),
            partials: u(d, vo + 4),
            limits: u(d, vo + 8),
            selected: u(d, vo + 0xc),
            pitch: f(d, vo + 0x10),
            dirty: d[vo + 0x14] != 0,
            reset: d[vo + 0x15] != 0,
            damping: f(d, vo + 0x18),
        };
        x
    }

    pub fn floats(p: &std::path::Path) -> Vec<f32> {
        std::fs::read(p).unwrap().as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
    }

    /// CYMBAL_ORACLE_MODAL=<dir written by the harness's modal probe>.
    #[test]
    #[ignore]
    fn modal_matches_maschine() {
        let dir = std::path::PathBuf::from(std::env::var("CYMBAL_ORACLE_MODAL").unwrap());
        let r = |n: &str| std::fs::read(dir.join(n)).unwrap();
        let mut x = load(&r("modal.bin"), &r("data.bin"), &r("smooth.bin"));
        let (i1, i2, ol, or) =
            (floats(&dir.join("in1.bin")), floats(&dir.join("in2.bin")), floats(&dir.join("outl.bin")), floats(&dir.join("outr.bin")));
        let (mut l, mut rr) = ([0f32; 16], [0f32; 16]);
        for k in 0..40 {
            x.process(&i1[16 * k..], &i2[16 * k..], &mut l, &mut rr, 16);
            for i in 0..16 {
                assert_eq!(
                    (l[i].to_bits(), rr[i].to_bits()),
                    (ol[16 * k + i].to_bits(), or[16 * k + i].to_bits()),
                    "block {k} sample {i}: {} {} vs {} {}",
                    l[i],
                    rr[i],
                    ol[16 * k + i],
                    or[16 * k + i]
                );
            }
        }
    }
}
