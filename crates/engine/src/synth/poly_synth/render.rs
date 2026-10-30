//! `P5::process` (0x101aa33ec), one sample at a time (docs/synth-poly-synth.md §3.4):
//! the global LFO / noise / modulation, the control-rate smoothing, and per voice the
//! two envelopes, Osc B (saw + triangle + pulse, band-limited with 4-sample polynomial
//! corrections), Osc A (saw + pulse, hard sync to B), the poly modulation, the 4-pole
//! filter (two 2-pole stages, LP/BP/HP) with soft-clipped feedback, DC removal and the
//! amplifier. The arithmetic follows the arm64 code operation by operation (`f32`,
//! `mul_add` where it uses `fmadd`/`fmsub`), so the output can match bit for bit.

use super::engine::{P5, SLOTS, Voice};
use super::params::xorshift;

/// Polynomial step correction (BLEP residual) at distance `x` (0..2) samples:
/// −0.034064297, 0.21099688, −0.359775, −0.20318376, 0.97941124, −0.059825, −1.48783,
/// 0.99986 (Horner order).
const P: [f32; 8] = [
    f32::from_bits(0xbd0b8701),
    f32::from_bits(0x3e580f91),
    f32::from_bits(0xbeb8346e),
    f32::from_bits(0xbe500f67),
    f32::from_bits(0x3f7abab2),
    f32::from_bits(0xbd750b0f),
    f32::from_bits(0xbfbe7137),
    f32::from_bits(0x3f7ff6d3),
];

/// Polynomial corner correction (BLAMP residual), for the triangle's slope changes:
/// −0.004258037, 0.030142412, −0.0599625, −0.04063675, 0.24485281, −0.019941667,
/// −0.743915, 0.99986, −0.41236663.
const Q: [f32; 9] = [
    f32::from_bits(0xbb8b8701),
    f32::from_bits(0x3cf6ed38),
    f32::from_bits(0xbd759b3d),
    f32::from_bits(0xbd2672b9),
    f32::from_bits(0x3e7abab2),
    f32::from_bits(0xbca35cb5),
    f32::from_bits(0xbf3e7137),
    f32::from_bits(0x3f7ff6d3),
    f32::from_bits(0xbed321b8),
];

#[inline]
fn blep(x: f32) -> f32 {
    let mut p = x.mul_add(P[0], P[1]);
    for &c in &P[2..] {
        p = p.mul_add(x, c);
    }
    p
}

#[inline]
fn blamp(x: f32) -> f32 {
    let mut p = x.mul_add(Q[0], Q[1]);
    for &c in &Q[2..] {
        p = p.mul_add(x, c);
    }
    p
}

/// Ring positions around write index `i`: (i+2, i−1, i, i+1) mod 4.
#[inline]
fn taps(i: i32) -> [usize; 4] {
    [((i + 2) & 3) as usize, ((i - 1) & 3) as usize, (i & 3) as usize, ((i + 1) & 3) as usize]
}

/// The four correction values of a step at fraction `t`: `P(2−t)`, `P(1−t)`, `P(t)`,
/// `P(t+1)` (the last two negated by the callers).
#[inline]
fn blep4(t: f32) -> [f32; 4] {
    [blep(2.0 - t), blep(1.0 - t), blep(t), blep(t + 1.0)]
}

#[inline]
fn blamp4(t: f32) -> [f32; 4] {
    [blamp(2.0 - t), blamp(1.0 - t), blamp(t), blamp(t + 1.0)]
}

/// A step correction of height `amp` at `t` into a scratch ring (`fmul`, `fnmul`).
#[inline]
fn step_tmp(i: i32, t: f32, amp: f32) -> [f32; 4] {
    let k = taps(i);
    let p = blep4(t);
    let mut tmp = [0.0; 4];
    tmp[k[0]] = amp * p[0];
    tmp[k[1]] = amp * p[1];
    tmp[k[2]] = -(amp * p[2]);
    tmp[k[3]] = -(amp * p[3]);
    tmp
}

/// A step correction added straight into a ring (`fmadd`/`fmsub`, Osc A).
#[inline]
fn step_into(ring: &mut [f32; 4], i: i32, t: f32, amp: f32) {
    let k = taps(i);
    let p = blep4(t);
    ring[k[0]] = amp.mul_add(p[0], ring[k[0]]);
    ring[k[1]] = amp.mul_add(p[1], ring[k[1]]);
    ring[k[2]] = (-amp).mul_add(p[2], ring[k[2]]);
    ring[k[3]] = (-amp).mul_add(p[3], ring[k[3]]);
}

/// A corner correction of slope change `amp` at `t` into a scratch ring.
#[inline]
fn corner_tmp(i: i32, t: f32, amp: f32) -> [f32; 4] {
    let k = taps(i);
    let q = blamp4(t);
    let mut tmp = [0.0; 4];
    for j in 0..4 {
        tmp[k[j]] = q[j] * amp;
    }
    tmp
}

/// Osc B's rings take a correction: the audio ring adds it, the modulation ring
/// subtracts it.
#[inline]
fn apply_b(v: &mut Voice, tmp: &[f32; 4]) {
    for j in 0..4 {
        v.ring_b[j] = tmp[j] + v.ring_b[j];
        v.ring_bm[j] -= tmp[j];
    }
}

/// `x` limited to 1e-16..0.99 (a phase increment).
#[inline]
fn clamp_inc(x: f32) -> f32 {
    let y = if x > 0.99 { 0.99 } else { x };
    if x < 1e-16 { 1e-16 } else { y }
}

impl P5 {
    /// Render `out.len()` mono samples (Maschine writes the same value to both
    /// channels).
    pub fn render(&mut self, out: &mut [f32]) {
        for o in out.iter_mut() {
            *o = self.sample();
        }
    }

    fn sample(&mut self) -> f32 {
        // LFO: saw, triangle and pulse with linear corrections at their steps.
        let inc = self.lfo_rate * self.inv_sr;
        let mut ph = self.lfo_phase + inc;
        self.lfo_phase = ph;
        let half = inc * 0.5;
        let (saw, tri, pulse);
        if self.lfo_half != 0 {
            let r = 1.0 - ph;
            if r > half {
                pulse = self.lfo_pulse;
                saw = ph * self.lfo_saw;
                tri = ph.mul_add(2.0, -1.5) * self.lfo_tri;
            } else {
                let t = r / inc + 0.5;
                pulse = t * self.lfo_pulse;
                saw = (t - r) * self.lfo_saw;
                tri = (0.5 - inc) * self.lfo_tri;
                ph += -1.0;
                self.lfo_phase = ph;
                self.lfo_half = 0;
                self.lfo_sh = self.lfo_sh_on * self.noise_lp;
                if self.voice_trig != 0 {
                    self.lfo_trigger();
                }
            }
        } else {
            let r = 0.5 - ph;
            if r > half {
                tri = ph.mul_add(-2.0, 0.5) * self.lfo_tri;
                pulse = 0.0;
            } else {
                let t = 0.5 - r / inc;
                pulse = t * self.lfo_pulse;
                tri = (inc + -0.5) * self.lfo_tri;
                self.lfo_half = 1;
                if self.voice_trig != 0 {
                    self.lfo_release();
                }
            }
            saw = ph * self.lfo_saw;
        }

        // White noise (1-bit shift register) and its lowpassed copy.
        self.rng = xorshift(self.rng);
        let noise = self.noise_amp * ((self.rng & 1) as f32 + -0.5);
        self.noise_lp = (noise - self.noise_lp).mul_add(self.noise_coef, self.noise_lp);
        let mut lfo = saw + pulse;
        lfo = tri + lfo;
        lfo += self.lfo_sh;
        lfo += self.lfo_none;

        // Mod wheel (glides to its target) scales the Modulation amount.
        let w = self.wheel_step + self.wheel;
        let w = if self.wheel_step * (w - self.wheel_target) < 0.0 { w } else { self.wheel_target };
        self.wheel = w;
        let depth = (5.0 - w).mul_add(self.mod_amount, w);
        let src = self.lfo_noise.mul_add(self.noise_lp - lfo, lfo);
        let m = src * depth;

        let pwm = self.wm_pw_a.mul_add(m, self.pw_a);
        let pwm_a_prev = self.pwm_a[1];
        self.pwm_a = [pwm, self.pwm_a[0]];
        let pw_b = self.wm_pw_b.mul_add(m, self.pw_b);
        let pw_b = if pw_b > 1.0 { 1.0 } else { pw_b };
        let wm_a_prev = self.wm_a[1];
        self.wm_a = [m * self.wm_freq_a, self.wm_a[0]];
        let wm_cut = m * self.wm_cut;

        self.control(wm_cut);

        // External input: Maschine passes none.
        let ext = 0.0f32;
        self.acc = 0.0;
        let mut out = 0.0f32;
        let n = self.voices as usize;
        if n >= 1 {
            let pw_b = pw_b.max(-1.0);
            let fm_b = m * self.wm_freq_b;
            for i in 0..n.min(SLOTS) {
                if self.v[i].active == 0 {
                    continue;
                }
                out = self.voice(i, pw_b, fm_b, wm_cut, wm_a_prev, pwm_a_prev, noise, ext);
            }
        }
        if self.vol_left > 0 {
            self.vol_left -= 1;
            self.vol = self.vol_step + self.vol;
        }
        out * self.vol
    }

    /// LFO Voice Trig, start of a cycle: envelopes of held voices restart.
    fn lfo_trigger(&mut self) {
        let n = self.voices as usize;
        let (va, vf) = (self.vel_amp, self.vel_flt);
        for i in 0..n.min(SLOTS) {
            let held = self.uni_held != 0;
            let v = &mut self.v[i];
            if !(v.velocity > 0.0 || held) {
                continue;
            }
            v.active = 1;
            let x = v.velocity.mul_add(0.0078125, -0.5);
            let amp = va.mul_add(x, 0.5);
            let a = v.amp.value;
            if a > amp {
                v.amp.coef = self.amp_decay;
                v.amp.floor = (a + 0.02).mul_add(self.amp_sustain, -0.02);
                v.amp.target = a;
                v.amp.rem = a - v.amp.floor;
                v.amp.stage = 1;
            } else {
                v.amp.target = amp;
                v.amp.inc = amp * self.amp_attack;
                v.amp.stage = 0;
            }
            let flt = vf.mul_add(x, 0.5);
            let f = v.flt.value;
            if f > flt {
                v.flt.target = f;
                v.flt.stage = 1;
                v.flt.coef = self.flt_decay;
                v.flt.floor = f * self.flt_sustain;
                v.flt.rem = f - v.flt.floor;
            } else {
                v.flt.target = flt;
                v.flt.inc = flt * self.flt_attack;
                v.flt.stage = 0;
                v.flt.rem = 1.0;
            }
        }
    }

    /// LFO Voice Trig, middle of a cycle: envelopes of held, running voices release.
    fn lfo_release(&mut self) {
        let n = self.voices as usize;
        for i in 0..n.min(SLOTS) {
            if self.v[i].active == 0 {
                continue;
            }
            if self.v[i].velocity > 0.0 || self.uni_held != 0 {
                self.release_voice(i);
            }
        }
    }

    /// The control block: runs every sample once the start-up count of 100 is over.
    fn control(&mut self, wm_cut: f32) {
        self.ctl_count -= 1;
        if self.ctl_count > 0 {
            return;
        }
        self.ctl_count = 1;
        if self.smooth_left != 0 {
            self.smooth_left -= 1;
            self.mix_a = self.mix_a_step + self.mix_a;
            self.mix_b = self.mix_b_step + self.mix_b;
            self.saw_a = self.mix_a * self.saw_on_a;
            self.pw_a += self.pw_a_step;
            self.pw_b += self.pw_b_step;
            self.cutoff = self.cutoff_step + self.cutoff;
            let c = self.cutoff;
            let n = (self.voices as usize).min(SLOTS);
            for v in self.v[..n].iter_mut() {
                v.cut = c + v.det_cut;
            }
            self.env_amt = self.env_amt_step + self.env_amt;
            self.kbd = self.kbd_step + self.kbd;
            self.mod_amount += self.mod_amount_step;
        }
        if self.bend_step != 0.0 {
            let mut b = self.bend_step + self.bend;
            if self.bend_step * (b - self.bend_target) >= 0.0 {
                self.bend_step = 0.0;
                b = self.bend_target;
            }
            self.bend = b;
            let t = b + self.master;
            self.tune = t;
            let b_base = (self.low_b + self.semis_b) + self.fine_b;
            for i in 0..SLOTS {
                let off_a = t + (self.semis_a + self.v[i].det_a);
                self.v[i].off_a = off_a;
                let off_b = t + (b_base + self.v[i].det_b);
                self.v[i].off_b = off_b;
                let p = self.v[i].pitch;
                self.v[i].freq_a = self.hz(off_a + p);
                self.v[i].freq_b = self.hz(self.kbd_b.mul_add(p, off_b));
            }
        }
        let n = (self.voices as usize).min(SLOTS);
        if self.glide_step != 0.0 {
            let mut g = self.glide_step + self.glide;
            self.glide = g;
            if self.glide_step * (g - self.glide_target) > 0.0 {
                self.glide = self.glide_target;
                self.glide_step = 0.0;
                g = self.glide_target;
            }
            for i in 0..n {
                self.v[i].pitch = g + self.v[i].spread;
                self.refresh_freqs(i);
            }
        }
        for i in 0..n {
            if self.v[i].active == 0 {
                continue;
            }
            let y = self.env_amt.mul_add(self.v[i].flt.value, wm_cut) + self.v[i].cut;
            let y = self.kbd.mul_add(self.v[i].pitch, y);
            self.v[i].cutoff = self.hz(y);
            let v = &mut self.v[i];
            let mut fv = v.flt.value;
            if v.amp.value < 0.0 {
                // The release has ended: the voice stops.
                v.amp.rem = 0.0;
                v.amp.floor = 0.0;
                v.flt.value = 0.0;
                v.active = 0;
                v.amp.value = 0.0;
                v.sync_d = [-1e20; 2];
                v.out_b = [0.0; 2];
                v.env_d = [0.0; 2];
                v.z1 = [0.0; 2];
                v.z2 = [0.0; 2];
                v.amp_d = [0.0; 4];
                fv = 0.0;
            } else if v.amp.rem < 1e-4 && v.amp.rem > -1e-4 {
                v.amp.rem = 0.0;
                v.amp.floor = v.amp.value;
            }
            if v.flt.rem < 1e-4 && v.flt.rem > -1e-4 {
                v.flt.rem = 0.0;
                v.flt.floor = fv;
            }
            if ((v.dc * v.dc) as f64) < 1e-7 {
                v.dc = 0.0;
            }
        }
    }

    /// One voice's sample, added (negated) into `acc`; returns `acc` after it.
    #[allow(clippy::too_many_arguments)]
    fn voice(&mut self, i: usize, pw_b: f32, fm_b: f32, wm_cut: f32, wm_a_prev: f32, pwm_a_prev: f32, noise: f32, ext: f32) -> f32 {
        // Amplifier envelope.
        {
            let (dec, sus) = (self.amp_decay, self.amp_sustain);
            let e = &mut self.v[i].amp;
            if e.stage != 0 {
                e.rem *= e.coef;
                e.value = e.rem + e.floor;
            } else {
                let x = e.inc + e.value;
                e.value = x;
                if x > e.target {
                    let t = e.target;
                    e.value = t;
                    e.stage = 1;
                    e.floor = (t + 0.02).mul_add(sus, -0.02);
                    e.rem = t - e.floor;
                    e.coef = dec;
                }
            }
        }
        // Filter envelope; reaching its peak also moves the cutoff.
        {
            let (dec, sus) = (self.flt_decay, self.flt_sustain);
            let e = &mut self.v[i].flt;
            if e.stage != 0 {
                e.rem *= e.coef;
                e.value = e.rem + e.floor;
            } else {
                let x = e.inc + e.value;
                e.value = x;
                if x > e.target {
                    let t = e.target;
                    e.value = t;
                    e.stage = 1;
                    e.floor = t * sus;
                    e.rem = t - e.floor;
                    e.coef = dec;
                    let y = self.env_amt.mul_add(t, wm_cut) + self.v[i].cut;
                    let y = self.kbd.mul_add(self.v[i].pitch, y);
                    self.v[i].cutoff = self.hz(y);
                }
            }
        }

        let (pulse_b, saw_b, tri_b) = (self.pulse_on_b as f32, self.saw_b, self.tri_on_b as f32);
        let sync_on = self.sync != 0;
        let inv_sr = self.inv_sr;
        let v = &mut self.v[i];

        // ---- Osc B: phase −1..1; saw = phase·level, triangle = |phase| − ½, pulse ±½.
        let mut st = v.state_b;
        let inc = clamp_inc(v.freq_b.mul_add(fm_b, v.freq_b) * inv_sr);
        let mut ph = v.phase_b + inc;
        let mut sgn = st as f32 + -0.5;
        let mut d = ph - pw_b;
        let mut sync_at = -1e20f32;
        let corner_due;
        if ph >= 1.0 {
            // Wrap.
            let t = (ph + -1.0) / inc;
            if sync_on {
                sync_at = t;
            }
            let prev = v.prev_b;
            let x = (prev - d).mul_add(t, d);
            let one_t = 1.0 - t;
            if sgn * x < 0.0 {
                let t2 = (x / (x - prev)).mul_add(one_t, t);
                st ^= 1;
                v.state_b = st;
                let amp = -(sgn * pulse_b);
                let tmp = step_tmp(v.idx_b, t2, amp);
                apply_b(v, &tmp);
            }
            if v.corner_b < 1.0 {
                let t3 = ph / inc;
                let tmp = corner_tmp(v.idx_b, t3, -(inc * tri_b));
                apply_b(v, &tmp);
                st = v.state_b;
            }
            // The saw (and the pulse reset) step at the wrap, scaled per ring.
            let k = taps(v.idx_b);
            let p = blep4(t);
            let mut tp = [0.0f32; 4];
            tp[k[0]] = p[0];
            tp[k[1]] = p[1];
            tp[k[2]] = -p[2];
            tp[k[3]] = -p[3];
            let qs = inc * tri_b;
            let q = blamp4(t);
            let mut tq = [0.0f32; 4];
            for j in 0..4 {
                tq[k[j]] = q[j] * qs;
            }
            let s3 = (pulse_b * -0.5).mul_add(st as f32, -saw_b);
            let half_p = pulse_b * 0.5;
            let s2 = half_p.mul_add(st as f32, -saw_b);
            for j in 0..4 {
                v.ring_b[j] = tp[j].mul_add(s3, v.ring_b[j]) + tq[j];
                v.ring_bm[j] = tp[j].mul_add(s2, v.ring_bm[j]) - tq[j];
            }
            v.state_b = 0;
            v.corner_b = 0.0;
            ph += -2.0;
            d = ph - pw_b;
            sgn = -0.5;
            if d * -0.5 < 0.0 {
                let x2 = x + -2.0;
                let lim = if x2 > 0.0 { 0.0 } else { x2 };
                let t4 = t * (d / (d - lim));
                v.state_b = 1;
                let tmp = step_tmp(v.idx_b, t4, half_p);
                apply_b(v, &tmp);
                sgn = 0.5;
            }
            corner_due = ph > v.corner_b;
        } else {
            if sgn * d < 0.0 {
                // Pulse edge.
                let t = d / (d - v.prev_b);
                st ^= 1;
                v.state_b = st;
                let amp = -(sgn * pulse_b);
                let tmp = step_tmp(v.idx_b, t, amp);
                sgn = -sgn;
                apply_b(v, &tmp);
            }
            corner_due = ph > v.corner_b;
        }
        if corner_due {
            // The triangle's lowest point (phase 0) passed.
            v.corner_b = 2.0;
            let t5 = ph / inc;
            let tmp = corner_tmp(v.idx_b, t5, -(inc * tri_b));
            apply_b(v, &tmp);
        }
        {
            let idx = v.idx_b;
            let k = taps(idx);
            let s1 = ph * saw_b;
            let s3 = sgn.mul_add(pulse_b, (ph.abs() + -0.5) * tri_b);
            let s0 = pulse_b.mul_add(0.5, saw_b);
            let w = k[2];
            v.ring_b[w] += s1 + s3;
            v.ring_bm[w] += s0 + (s1 - s3);
            v.phase_b = ph;
            v.prev_b = d;
        }
        let k2 = taps(v.idx_b)[0];
        let out_b = v.out_b[1];
        v.out_b = [v.ring_b[k2], v.out_b[0]];
        v.ring_b[k2] = 0.0;
        let mod_b = v.ring_bm[k2];
        v.ring_bm[k2] = 0.0;
        v.idx_b = (v.idx_b + 1) & 3;

        // ---- Poly mod: Osc B (modulation ring) and the delayed filter envelope.
        let fenv_d = v.env_d[1];
        v.env_d = [v.flt.value, v.env_d[0]];
        let pm = self.pm_osc_b.mul_add(mod_b, (fenv_d * v.drift) * self.pm_env);

        // ---- Osc A: phase −1..1, saw + pulse, hard-synced by B's wrap.
        let fa = v.freq_a_d[1];
        v.freq_a_d = [v.freq_a, v.freq_a_d[0]];
        let fm = self.pm_freq_a.mul_add(pm, wm_a_prev);
        let f = inv_sr * fa.mul_add(fm, fa);
        v.inc_a = (v.inc_a + f) * 0.5;
        let inc = clamp_inc(v.inc_a);
        let mut ph = v.phase_a + inc;
        let st0 = v.state_a;
        let mut sgn = st0 as f32 + -0.5;
        let pamp = (self.mix_a + self.mix_a) * self.pulse_on_a as f32;
        let pw = self.pm_pw_a.mul_add(pm, pwm_a_prev);
        let pw = if pw > 1.0 { 1.0 } else { pw };
        let pw = if pw < -1.0 { -1.0 } else { pw };
        let mut d = ph - pw;
        let sync = v.sync_d[1];
        v.sync_d = [sync_at, v.sync_d[0]];
        let saw_a = self.saw_a;
        let idx = v.idx_a;
        let mut do_sync = false;
        let wrap_t = if ph >= 1.0 { Some((ph + -1.0) / inc) } else { None };
        match wrap_t {
            Some(t) if !(sync > t) => {
                // Wrap (unless the sync comes first).
                let prev = v.prev_a;
                let x = (prev - d).mul_add(t, d);
                let one_t = 1.0 - t;
                let mut st = st0;
                if sgn * x < 0.0 {
                    let t2 = (x / (x - prev)).mul_add(one_t, t);
                    step_into(&mut v.ring_a, idx, t2, pamp * 0.5);
                    st = 1;
                }
                let a = (pamp * -0.5).mul_add(st as f32, -saw_a);
                v.state_a = 0;
                step_into(&mut v.ring_a, idx, t, a);
                ph += -2.0;
                d = ph - pw;
                sgn = -0.5;
                if d * -0.5 < 0.0 {
                    let x2 = x + -2.0;
                    let lim = if x2 > 0.0 { 0.0 } else { x2 };
                    let t3 = t * (d / (d - lim));
                    if sync > t3 {
                        do_sync = true;
                    } else {
                        v.state_a = 1;
                        step_into(&mut v.ring_a, idx, t3, pamp * 0.5);
                        sgn = 0.5;
                    }
                }
                if !do_sync && sync > -1.0 {
                    do_sync = true;
                }
            }
            _ => {
                if sgn * d < 0.0 {
                    let t = d / (d - v.prev_a);
                    if sync > t {
                        do_sync = true;
                    } else {
                        let amp = -(sgn * pamp);
                        let k = taps(idx);
                        let p = blep4(t);
                        v.ring_a[k[0]] = amp.mul_add(p[0], v.ring_a[k[0]]);
                        v.state_a = st0 ^ 1;
                        v.ring_a[k[1]] = amp.mul_add(p[1], v.ring_a[k[1]]);
                        v.ring_a[k[2]] = (-amp).mul_add(p[2], v.ring_a[k[2]]);
                        sgn = -sgn;
                        v.ring_a[k[3]] = (-amp).mul_add(p[3], v.ring_a[k[3]]);
                    }
                }
                if !do_sync && sync > -1.0 {
                    do_sync = true;
                }
            }
        }
        if do_sync {
            // Hard sync: restart where B wrapped, with the step corrected.
            let cur = ph;
            let nph = inc * sync + -1.0;
            let jump = cur - nph;
            let s0 = -((sgn + 0.5) * pamp);
            let s0 = (-saw_a).mul_add(jump, s0);
            v.state_a = 0;
            step_into(&mut v.ring_a, idx, sync, s0 * 0.5);
            d = nph - pw;
            sgn = -0.5;
            if d * -0.5 < 0.0 {
                let x = ((v.prev_a - jump) - d).mul_add(sync, d);
                let lim = if x > 0.0 { 0.0 } else { x };
                let t = sync * (d / (d - lim));
                step_into(&mut v.ring_a, idx, t, pamp * 0.5);
                v.state_a = 1;
                sgn = 0.5;
            }
            ph = nph;
        }
        let k = taps(idx);
        let s = ph.mul_add(saw_a, pamp * sgn);
        v.ring_a[k[2]] = s + v.ring_a[k[2]];
        v.phase_a = ph;
        v.prev_a = d;
        let out_a = v.ring_a[k[0]];
        v.ring_a[k[0]] = 0.0;
        v.idx_a = (idx + 1) & 3;

        // ---- Mixer.
        let fb = v.fb;
        let x = self.mix_b.mul_add(out_b, out_a);
        let x = self.noise_mix.mul_add(noise, x);
        let x = ext + x;

        // ---- Filter: cutoff (delayed, poly-modulated) → tan-like warp.
        let c = v.cut_d[1];
        v.cut_d = [v.cutoff, v.cut_d[0]];
        let cm = (pm * self.pm_cut).mul_add(c, c);
        let c = v.cutm_d[1];
        v.cutm_d = [cm, v.cutm_d[0]];
        let c = if c < 1.0 { 1.0 } else { c };
        let w = c * self.pi_sr;
        let w = if w > 1.5 { 1.5 } else { w };
        // tan(w) ≈ w·(1.00005 + 0.0388452w − 0.0896638w²)/(1 + 0.0404318w − 0.430871w²).
        let num = w * w.mul_add(f32::from_bits(0xbdb7a1a8), f32::from_bits(0x3d1f1c25)).mul_add(w, f32::from_bits(0x3f8001a3));
        let den = w.mul_add(f32::from_bits(0xbedc9b20), f32::from_bits(0x3d259bd1)).mul_add(w, 1.0);
        let g = num / den;
        const TINY: f32 = f32::from_bits(0x33d6bf95);
        // Stage 1 (state-variable, damping `damp1`).
        let x = x - fb;
        let k1 = self.damp1 + g;
        let [za, zb] = v.z1;
        let x = x - za.mul_add(k1, zb);
        let hp1 = x / g.mul_add(k1, 1.0);
        let s = g * hp1;
        let bp1 = za + s;
        let za2 = (s + bp1) + TINY;
        let s = g * bp1;
        let lp1 = zb + s;
        let zb2 = (s + lp1) + TINY;
        v.z1 = [za2, zb2];
        let mode = self.mode.clamp(0, 2) as usize;
        let in2 = self.damp2 * [lp1, bp1, hp1][mode];
        // Stage 2.
        let k2 = g + self.damp2;
        let [za, zb] = v.z2;
        let x = in2 - za.mul_add(k2, zb);
        let hp2 = x / g.mul_add(k2, 1.0);
        let s = g * hp2;
        let bp2 = za + s;
        let za2 = (s + bp2) + TINY;
        let s = g * bp2;
        let lp2 = zb + s;
        let zb2 = (s + lp2) + TINY;
        v.z2 = [za2, zb2];
        let y = [lp2, bp2, hp2][mode];
        // Feedback: a soft clip of the output, y·(0.13335 ∓ 0.01666875·y) up to ±4, then
        // ±0.2667.
        v.fb = if y <= 0.0 {
            if y > -4.0 {
                (y * y.mul_add(f32::from_bits(0x3c888ce7), f32::from_bits(0x3e088ce7))) * self.fb_sign
            } else {
                self.fb_sign * f32::from_bits(0xbe888ce7)
            }
        } else if y < 4.0 {
            (y * y.mul_add(f32::from_bits(0xbc888ce7), f32::from_bits(0x3e088ce7))) * self.fb_sign
        } else {
            self.fb_sign * f32::from_bits(0x3e888ce7)
        };
        // DC removal, then the amplifier (its envelope delayed 4 samples).
        let hp = y - v.dc;
        v.dc = hp.mul_add(self.dc_coef, v.dc);
        let env = v.amp_d[3];
        v.amp_d = [v.amp.value, v.amp_d[0], v.amp_d[1], v.amp_d[2]];
        self.acc = (-hp).mul_add(env, self.acc);
        self.acc
    }
}
