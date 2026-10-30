//! Maschine's Beat Delay (docs/fx-beat-delay.md): `RT::BeatDelayProcessor` →
//! `FXCORE::StudioDelayCore`. A tempo-synced stereo delay: two 2^19-sample lines (A, B)
//! whose taps cross-feed each other (Crossover), a "Color" tone filter on the taps, a
//! "Split" state-variable filter that sends the bands of the input to opposite lines,
//! and a Stereo width / dry-wet output. Delay-time changes crossfade between two read
//! pointers; the 36 smoothed parameters ramp over ~20 ms at a 128-sample control rate.
//! All `f32` (the Split coefficients partly in `f64`, as the original); `mul_add` only
//! where Maschine fuses (`fmadd`/`fnmsub`/`fmla`), so the output can match bit for bit.

use super::exp_e;

use super::{Effect, param};

/// Ring-buffer length (each line) and index mask.
const LEN: usize = 1 << 19;
const MASK: u32 = (LEN - 1) as u32;
/// Control-rate period (`+0x280`).
const BLOCK: i32 = 128;

/// `StudioDelayCore::UnitArray`: 1/16, 1/16T, 1/32 relative to a 1/16 note.
const UNIT: [f32; 3] = [1.0, f32::from_bits(0x3f2a_aaab), 0.5];
/// 3/log10(2): dB·K + 1200 indexes the exp table as 10^(dB/20).
const K_DB: f32 = f32::from_bits(0x411f_73da);
/// dB·K_DB + this indexes the exp table as 10^(dB/20)/1000.
const DB_MS: f32 = f32::from_bits(0x4416_8363); // 602.0529
/// MIDI pitch·5 + this indexes the exp table as the pitch's frequency in Hz.
const PITCH_HZ: f32 = f32::from_bits(0x44ac_bc36); // 1381.8816
const PI: f32 = f32::from_bits(0x4049_0fdb);
/// 10^(1/20): the feedback gain per dB.
const DB_BASE: f32 = f32::from_bits(0x3f8f_9db2);

// Smoother lanes: nine groups of four (targets `+0x1d0`, values `+0xb0`), §2.3.
/// Feedback matrix: B ← R, B ← L, A ← L, A ← R.
const FB: usize = 0;
/// Read-pointer crossfade, dry gain, wet (direct), wet (swapped).
const XF: usize = 4;
const DRY: usize = 5;
const WET: usize = 6;
const WET_X: usize = 7;
/// Color mix: dry tap, (always 0), high-pass lane, low-pass lane.
const COL_MIX: usize = 8;
/// Color filter: 1/(1+g1), 1/(1+g2), g1 − 1, g2 − 1.
const COL_E: usize = 12;
/// Color filter output scale: 1, g2, 1, g2.
const COL_F: usize = 16;
/// Split send scale into line B (1 − y) and line A (1); lanes 2 and 3 unused (1).
const SEND: usize = 20;
/// Split band gains: m, 1 − m, 2m, 1 − m.
const SPLIT_A: usize = 24;
/// Split SVF frequency (lanes 0 and 2; 1 and 3 stay 0).
const SPLIT_W: usize = 28;
/// Split SVF damping (lanes 0 and 2; 1 and 3 stay 0).
const SPLIT_D: usize = 32;
const LANES: usize = 36;

/// Field names, in the order Maschine connects them.
const FIELDS: [&str; 9] = ["time", "offset", "feedback", "crossover", "color", "split", "stereo", "mix", "unit"];

pub struct BeatDelay {
    sr: f32,
    /// The stored values, `FIELDS` order (the enums as their index).
    vals: [f32; 9],
    /// Host tempo (`set_tempo`), applied at the start of every block.
    host_bpm: f32,
    /// No audio yet: new settings apply as Maschine's `resume` does.
    fresh: bool,

    // Core inputs as last applied (`+0x0c`, `+0x5c`..`+0x90`).
    bpm: f32,
    unit: i32,
    /// "Tsh" (`+0x70`): a pre-delay Maschine never sets (always 0).
    tsh: f32,
    time: f32,
    offset: f32,
    feedback: f32,
    crossover: f32,
    /// (Stereo + 1)/2.
    st: f32,
    /// Wet gain correction for high feedback.
    corr: f32,
    mix: f32,
    /// 15000/bpm: a 1/16 note in ms.
    beat: f32,
    /// Delay of line A and B in samples.
    delay: [f32; 2],
    /// Feedback gain per pass of line A and B (before Crossover).
    gain: [f32; 2],

    // Smoother (`+0xb0`..`+0x270`).
    cur: [f32; LANES],
    inc: [f32; LANES],
    tgt: [f32; LANES],
    dirty: bool,
    steps: i32,
    remaining: i32,
    inv_len: f32,
    counter: i32,

    // Audio state.
    a: Vec<f32>,
    b: Vec<f32>,
    wpos: u32,
    /// Read pointers: line A new, line A old, line B new, line B old.
    rd: [u32; 4],
    /// Color outputs of the previous frame (A, B): the feedback.
    fb: [f32; 2],
    /// Split SVF band-pass and low-pass states, lanes L, L, R, R.
    s1: [f32; 4],
    s2: [f32; 4],
    /// Color filter states (`+0x400320` …): previous taps, previous lane outputs, two
    /// integrators.
    cx: [f32; 4],
    cw: [f32; 4],
    cs: [f32; 4],
    cv: [f32; 4],
}

impl BeatDelay {
    pub fn new(sr: f64) -> BeatDelay {
        let mut d = BeatDelay {
            sr: sr as f32,
            vals: [3.0, 0.0, 0.5, 0.25, 0.5, 0.25, 0.0, 0.5, 0.0],
            host_bpm: 120.0,
            fresh: true,
            bpm: 120.0,
            unit: 0,
            tsh: 0.0,
            time: 0.0,
            offset: 0.0,
            feedback: 0.0,
            crossover: 0.0,
            st: 0.0,
            corr: 0.0,
            mix: 0.0,
            beat: 0.0,
            delay: [0.0; 2],
            gain: [0.0; 2],
            cur: [0.0; LANES],
            inc: [0.0; LANES],
            tgt: [0.0; LANES],
            dirty: false,
            steps: 1,
            remaining: 0,
            inv_len: 1.0,
            counter: 0,
            a: vec![0.0; LEN],
            b: vec![0.0; LEN],
            wpos: 0,
            rd: [0; 4],
            fb: [0.0; 2],
            s1: [0.0; 4],
            s2: [0.0; 4],
            cx: [0.0; 4],
            cw: [0.0; 4],
            cs: [0.0; 4],
            cv: [0.0; 4],
        };
        d.resume();
        d
    }

    /// `BeatDelayProcessor::resume` = `beginInit` + `endInit` (§2.6): clear all state,
    /// apply every stored value in the core's order, set the values to the targets
    /// (no ramp). The tempo is the core's last one (120 before any audio).
    fn resume(&mut self) {
        self.cur = [0.0; LANES];
        self.inc = [0.0; LANES];
        self.tgt = [0.0; LANES];
        for i in [COL_F, COL_F + 2, SEND + 1, SEND + 2, SEND + 3] {
            self.tgt[i] = 1.0;
        }
        self.a.fill(0.0);
        self.b.fill(0.0);
        self.wpos = 0;
        self.rd = [0; 4];
        self.fb = [0.0; 2];
        self.s1 = [0.0; 4];
        self.s2 = [0.0; 4];
        self.cx = [0.0; 4];
        self.cw = [0.0; 4];
        self.cs = [0.0; 4];
        self.cv = [0.0; 4];
        (self.unit, self.time, self.offset, self.feedback, self.crossover) = (0, 0.0, 0.0, 0.0, 0.0);
        (self.st, self.corr, self.mix) = (0.0, 0.0, 0.0);
        self.gain = [0.0; 2];
        self.remaining = 0;
        // `beginInit` sets the counter past the period: the first sample ticks.
        self.counter = BLOCK + 1;

        let steps = ((self.sr * f32::from_bits(0x3ca3_d70a)) / BLOCK as f32 + 0.5) as i32;
        self.steps = if steps > 1 { steps } else { 1 };
        self.inv_len = 1.0 / (self.steps * BLOCK) as f32;

        let v = self.vals;
        self.ev_mix(v[7]);
        self.ev_bpm();
        self.ev_time(v[0]);
        self.ev_unit(v[8]);
        self.ev_tsh();
        self.ev_offset(v[1]);
        self.ev_feedback(v[2]);
        self.ev_crossover(v[3]);
        self.ev_split(v[5]);
        self.ev_color(v[4]);
        self.ev_stereo(v[6]);
        self.cur = self.tgt;
        self.dirty = true;
    }

    // --- Parameter events (`processEventIn*`, §2.2) ---

    /// Recompute both delays (`processEventInT/Unit/Tp/BPM`); unless a crossfade is
    /// running, point the new read pointers at them, start a crossfade and update the
    /// feedback gains.
    fn delays(&mut self) {
        let base = UNIT[self.unit as usize] * ((self.beat * self.sr) * f32::from_bits(0x3a83_126f));
        let (k, d) = if self.time < 1.0 { (1.0, base * 0.5) } else { (self.time, base) };
        let fl = d.floor();
        let tsh = (self.sr * ((self.tsh * self.tsh) * f32::from_bits(0x3b03_126f))).floor();
        self.delay = [fl.mul_add(k, tsh), (k + self.offset).mul_add(fl, -tsh)];
        if self.inc[XF] != 0.0 {
            return;
        }
        self.rd[0] = self.wpos.wrapping_sub(self.delay[0] as i32 as u32) & MASK;
        self.rd[2] = self.wpos.wrapping_sub(self.delay[1] as i32 as u32) & MASK;
        self.tgt[XF] = 1.0;
        self.dirty = true;
        self.set_fb_params();
    }

    fn ev_bpm(&mut self) {
        self.beat = 15000.0 / self.bpm;
        self.delays();
    }

    fn ev_time(&mut self, v: f32) {
        self.time = v;
        self.delays();
    }

    fn ev_unit(&mut self, v: f32) {
        self.unit = (v as i32).clamp(0, 2);
        self.delays();
    }

    fn ev_tsh(&mut self) {
        self.delays();
    }

    fn ev_offset(&mut self, v: f32) {
        self.offset = v;
        self.delays();
    }

    /// `setFBParams`: per-pass gains for a −60 dB decay over 56 ms … 25 s (Feedback),
    /// then the Crossover matrix.
    fn set_fb_params(&mut self) {
        let db = self.feedback.mul_add(53.0, 35.0);
        let db = if db == 35.0 { -100.0 } else { db };
        let ms = exp_e(db.mul_add(K_DB, DB_MS)).mul_add(1000.0, f32::from_bits(0x38d1_b717));
        let per = (1.0 / ms) * (-60000.0 / self.sr);
        let pass = |d: f32| {
            let e = d * per;
            DB_BASE.powf(if e < -120.0 { -120.0 } else { e })
        };
        self.gain = [pass(self.delay[0]), pass(self.delay[1])];
        self.crossover_matrix();
    }

    /// The Crossover part of `setFBParams` / `processEventInXFB`.
    fn crossover_matrix(&mut self) {
        let x = self.crossover;
        let y = x.mul_add(2.0, -1.0).max(0.0);
        let u = [x.mul_add(-2.0, 1.0).clamp(0.0, 1.0), (x + x).clamp(0.0, 1.0)]; // fmaxnm, fminnm
        let [ga, gb] = self.gain;
        let (a0, a1) = (u[0] * ga, u[1] * ga);
        self.tgt[FB] = gb * u[0];
        self.tgt[FB + 1] = (1.0 - y).mul_add(a1, y);
        self.tgt[FB + 2] = a0;
        self.tgt[FB + 3] = a1;
        self.tgt[SEND] = 1.0 - y;
        self.dirty = true;
    }

    fn ev_feedback(&mut self, v: f32) {
        self.feedback = v;
        self.set_fb_params();
        let h = self.feedback + -0.5;
        let h = h + h;
        self.corr = if self.feedback < 0.5 { 1.0 } else { (h * h).mul_add(f32::from_bits(0xbe4c_cccd), 1.0) };
        self.wet();
    }

    fn ev_crossover(&mut self, v: f32) {
        self.crossover = v;
        self.crossover_matrix();
    }

    /// Dry and wet gains (`processEventInMix/St`, the end of `…InFB`).
    fn wet(&mut self) {
        let m = self.mix;
        let dry = ((1.0 - m) + (1.0 - m)).min(1.0);
        let mm = m * (2.0 - m);
        let st = self.st;
        self.tgt[DRY] = dry;
        self.tgt[WET] = ((st * (2.0 - st)) * mm) * self.corr;
        self.tgt[WET_X] = (((st + 1.0) * (1.0 - st)) * mm) * self.corr;
        self.dirty = true;
    }

    fn ev_mix(&mut self, v: f32) {
        self.mix = v;
        self.wet();
    }

    fn ev_stereo(&mut self, v: f32) {
        self.st = (v + 1.0) * 0.5;
        self.wet();
    }

    /// `processEventInSplit` (partly in `f64`).
    fn ev_split(&mut self, v: f32) {
        let f = exp_e(v.mul_add(53.0, 50.0).mul_add(5.0, PITCH_HZ));
        let w = ((f as f64 * f64::from_bits(0x4019_21fc_8f32_378b)) / self.sr as f64) as f32;
        let s = v as f64;
        let q = (s.mul_add(f64::from_bits(0xbff0_624d_d2f1_a9fc), f64::from_bits(0x3ff4_7ae1_47ae_147b)) * s)
            * f64::from_bits(0x3fe9_9999_9999_999a);
        let q = 1.0 - ((q as f32) as f64 + f64::from_bits(0x3fd9_9999_9999_999a));
        let damp = (q + q) as f32;
        let m = v * 10.0;
        let m = if m > 1.0 { 1.0 } else { m };
        self.tgt[SPLIT_A..SPLIT_A + 4].copy_from_slice(&[m, 1.0 - m, m + m, 1.0 - m]);
        self.tgt[SPLIT_W] = w;
        self.tgt[SPLIT_W + 2] = w;
        self.tgt[SPLIT_D] = damp;
        self.tgt[SPLIT_D + 2] = damp;
        self.dirty = true;
    }

    /// `setColorParams`: below 0.47 the low-pass lane, above 0.53 the high-pass lane, a crossfade
    /// through the dry tap in between; the corners move with |2c − 1|.
    fn ev_color(&mut self, v: f32) {
        let k1 = [f32::from_bits(0x3f07_ae14), 0.5]; // 0.53, 0.5
        let k2 = [0.5, f32::from_bits(0x3ef0_a3d7)]; // 0.5, 0.47
        let k3 = [f32::from_bits(0x3cf5_c280), f32::from_bits(0x3cf5_c290)]; // ≈ 0.03
        let c = [if v > k1[0] { k1[0] } else { v }, if v > k1[1] { k1[1] } else { v }];
        let hp = if k2[0] > c[0] { 0.0 } else { (c[0] + -0.5) / k3[0] };
        let lp = if k2[1] > c[1] { 1.0 } else { (0.5 - c[1]) / k3[1] };
        self.tgt[COL_MIX] = 1.0 - (hp + lp);
        self.tgt[COL_MIX + 2] = hp;
        self.tgt[COL_MIX + 3] = lp;
        let t = v.mul_add(2.0, -1.0).abs();
        let f1 = exp_e(t.mul_add(54.0, 36.0).mul_add(5.0, PITCH_HZ));
        let f2 = exp_e(t.mul_add(-65.0, 125.0).mul_add(5.0, PITCH_HZ));
        let g1 = ((f1 * PI) / self.sr).tan();
        let g2 = ((f2 * PI) / self.sr).tan();
        self.tgt[COL_E..COL_E + 4].copy_from_slice(&[1.0 / (g1 + 1.0), 1.0 / (g2 + 1.0), g1 + -1.0, g2 + -1.0]);
        self.tgt[COL_F + 1] = g2;
        self.tgt[COL_F + 3] = g2;
        self.dirty = true;
    }

    /// Apply a changed stored value (`processFloat/EnumParameter`).
    fn event(&mut self, i: usize, v: f32) {
        match i {
            0 => self.ev_time(v),
            1 => self.ev_offset(v),
            2 => self.ev_feedback(v),
            3 => self.ev_crossover(v),
            4 => self.ev_color(v),
            5 => self.ev_split(v),
            6 => self.ev_stereo(v),
            7 => self.ev_mix(v),
            _ => self.ev_unit(v),
        }
    }

    // --- Audio (`processAudio`, §2.4) ---

    /// The control tick: start a ramp toward new targets, or count one down.
    fn tick(&mut self) {
        if self.dirty {
            self.dirty = false;
            let mut sum = 0f32;
            for i in 0..LANES {
                let inc = (self.tgt[i] - self.cur[i]) * self.inv_len;
                let sq = inc * inc;
                if (sq as f64) < f64::from_bits(0x3cd2_03af_9ee7_5616) {
                    self.inc[i] = 0.0;
                    self.cur[i] = self.tgt[i];
                } else {
                    self.inc[i] = inc;
                    sum += sq;
                }
            }
            if sum > 0.0 {
                self.remaining = self.steps;
            }
        } else {
            self.remaining = self.remaining.wrapping_sub(1);
            if self.remaining == 0 {
                self.cur = self.tgt;
                self.inc = [0.0; LANES];
            }
        }
        // A finished (or never started) crossfade: the new pointers become the old.
        if self.inc[XF] == 0.0 {
            self.inc[XF] = 0.0;
            self.tgt[XF] = 0.0;
            self.cur[XF] = 0.0;
            self.rd[1] = self.rd[0];
            self.rd[3] = self.rd[2];
        }
    }

    #[inline]
    fn frame(&mut self, il: f32, ir: f32) -> (f32, f32) {
        self.counter += 1;
        if self.counter >= BLOCK {
            // Maschine runs `processEventInBPM` at the start of each of its internal
            // blocks (≤ 128 samples). It only has an effect when a control tick follows
            // before the next one, so it runs here, right before every tick (§2.5).
            self.bpm = self.host_bpm;
            self.ev_bpm();
            self.counter = 0;
            self.tick();
        }
        for i in 0..LANES {
            self.cur[i] += self.inc[i];
        }
        let c = &self.cur;
        let lane = |g: usize| [c[g], c[g + 1], c[g + 2], c[g + 3]];
        let (fbm, d, e, f, send) = (lane(FB), lane(COL_MIX), lane(COL_E), lane(COL_F), lane(SEND));
        let (sa, sw, sd) = (lane(SPLIT_A), lane(SPLIT_W), lane(SPLIT_D));

        // Split: a state-variable filter per lane (L, L, R, R; lanes 1 and 3 pass
        // through), its bands crossed over to the two lines.
        let x = [il, il, ir, ir];
        let mut hp = [0f32; 4];
        let mut bp = [0f32; 4];
        let mut lp = [0f32; 4];
        for i in 0..4 {
            hp[i] = x[i] - (self.s2[i] + self.s1[i] * sd[i]);
            bp[i] = self.s1[i] + sw[i] * hp[i];
            lp[i] = self.s2[i] + sw[i] * bp[i];
        }
        let t = [sa[0] * (hp[2] + lp[0]), sa[1] * hp[3], sa[2] * bp[0], sa[3] * hp[1]];
        let to_b = send[0] * (t[0] + t[1]);
        let to_a = send[1] * (t[2] + t[3]);
        self.s1 = bp;
        self.s2 = lp;

        // Feedback (the previous color outputs) plus the split sends into the lines.
        let [fl, fr] = self.fb;
        let p = [fbm[0] * fr, fbm[1] * fl, fbm[2] * fl, fbm[3] * fr];
        let w = self.wpos as usize;
        self.b[w] = (p[0] + p[1]) + to_b;
        self.a[w] = (p[2] + p[3]) + to_a;

        // Taps, crossfaded from the old to the new read pointers.
        let xf = c[XF];
        let nx = 1.0 - xf;
        let tap_a = self.a[self.rd[0] as usize].mul_add(xf, nx * self.a[self.rd[1] as usize]);
        let tap_b = self.b[self.rd[2] as usize].mul_add(xf, nx * self.b[self.rd[3] as usize]);

        // Color: per line a one-pole high-pass (lanes 0/2) and a two-pole low-pass (1/3) on
        // the previous tap, mixed with the current tap. Written lane-wise with the
        // original's 0/1 masks, which keep the unused lanes' arithmetic.
        let ev = [e[0], e[1], e[0], e[1]];
        let e2 = [e[2], e[3], e[2], e[3]];
        const O: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
        const I: [f32; 4] = [1.0, 0.0, 1.0, 0.0];
        let mut u = [0f32; 4];
        let mut y = [0f32; 4];
        let mut v2 = [0f32; 4];
        let mut out = [0f32; 4];
        for i in 0..4 {
            u[i] = ev[i] * (self.cx[i] - self.cs[i] * e2[i]);
            let (dn, up) = (u[i] - self.cs[i], u[i] + self.cs[i]);
            y[i] = f[i] * if i % 2 == 0 { dn } else { up };
            let g = ev[i] * O[i] + I[i];
            v2[i] = g * (self.cw[i] - (e2[i] * O[i]) * self.cv[i]);
            let z = f[i] * (self.cv[i] * O[i] + v2[i]);
            out[i] = z * O[i] + y[i] * I[i];
        }
        self.cs = u;
        self.cv = v2;
        self.cx = [tap_a, tap_a, tap_b, tap_b];
        self.cw = y;
        let oa = (d[0] * tap_a + d[1] * tap_a) + (d[2] * out[0] + d[3] * out[1]);
        let ob = (d[0] * tap_b + d[1] * out[1]) + (d[2] * out[2] + d[3] * out[3]);
        self.fb = [oa, ob];

        let (dry, wet, wet_x) = (c[DRY], c[WET], c[WET_X]);
        let l = ob.mul_add(wet_x, il.mul_add(dry, oa * wet));
        let r = oa.mul_add(wet_x, ir.mul_add(dry, ob * wet));

        self.wpos = (self.wpos + 1) & MASK;
        for p in &mut self.rd {
            *p = (*p + 1) & MASK;
        }
        (l, r)
    }
}

impl Effect for BeatDelay {
    fn set(&mut self, params: &[(String, f64)]) {
        for (i, f) in FIELDS.iter().enumerate() {
            let v = param(params, f, self.vals[i] as f64) as f32;
            if v != self.vals[i] {
                self.vals[i] = v;
                if !self.fresh {
                    self.event(i, v);
                }
            }
        }
        if self.fresh {
            // Maschine loads the values, then resumes the processor before any audio.
            self.resume();
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.fresh = false;
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            (*a, *b) = self.frame(*a, *b);
        }
    }

    fn set_tempo(&mut self, bpm: f64) {
        self.host_bpm = bpm as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(kv: &[(&str, f64)]) -> Vec<(String, f64)> {
        kv.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn run(d: &mut BeatDelay, l: &mut [f32], r: &mut [f32]) {
        for (a, b) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
            d.process(a, b);
        }
    }

    fn peak(x: &[f32]) -> usize {
        x.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0
    }

    #[test]
    fn exp_table_matches_exp2() {
        for x in [0.0f32, 600.0, 1200.0, 1800.0, 2399.5] {
            let want = 2f32.powf((x - 1200.0) / 60.0);
            assert!((exp_e(x) / want - 1.0).abs() < 1e-4, "{x}");
        }
    }

    #[test]
    fn echo_lands_on_the_beat() {
        // 3/16 at 120 BPM and 48 kHz: 3 · 125 ms = 18000 samples; dry off.
        let mut d = BeatDelay::new(48000.0);
        d.set(&params(&[("mix", 1.0), ("stereo", 1.0), ("feedback", 0.0), ("split", 0.0)]));
        d.set_tempo(120.0);
        let mut l = vec![0f32; 40000];
        let mut r = vec![0f32; 40000];
        l[0] = 1.0;
        r[0] = 1.0;
        run(&mut d, &mut l, &mut r);
        assert_eq!(peak(&l[..30000]), 18000);
        assert_eq!(peak(&r[..30000]), 18000);
        assert!(l[..17990].iter().all(|v| v.abs() < 1e-6));
    }

    #[test]
    fn tempo_moves_the_echo() {
        let mut d = BeatDelay::new(48000.0);
        d.set(&params(&[("time", 1.0), ("mix", 1.0), ("stereo", 1.0), ("feedback", 0.0), ("split", 0.0)]));
        d.set_tempo(100.0);
        let mut l = vec![0f32; 20000];
        let mut r = vec![0f32; 20000];
        l[0] = 1.0;
        run(&mut d, &mut l, &mut r);
        // 1/16 at 100 BPM: 150 ms.
        assert_eq!(peak(&l), 7200);
    }

    #[test]
    fn offset_delays_the_right_line() {
        let mut d = BeatDelay::new(48000.0);
        d.set(&params(&[("time", 1.0), ("offset", 2.0), ("mix", 1.0), ("stereo", 1.0), ("feedback", 0.0), ("split", 0.0)]));
        let mut l = vec![0f32; 30000];
        let mut r = vec![0f32; 30000];
        l[0] = 1.0;
        r[0] = 1.0;
        run(&mut d, &mut l, &mut r);
        // 1/16 = 6000 samples; the right side is 1 + 2 sixteenths late.
        assert!(l[6000].abs() > 0.1, "{}", l[6000]);
        assert!(r[18000].abs() > 0.1, "{}", r[18000]);
        assert!(r[6000].abs() < 1e-6);
    }

    #[test]
    fn dry_only_when_mix_is_zero() {
        let mut d = BeatDelay::new(44100.0);
        d.set(&params(&[("mix", 0.0)]));
        let mut l: Vec<f32> = (0..20000).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut r = l.clone();
        let x = l.clone();
        run(&mut d, &mut l, &mut r);
        assert_eq!(l, x);
        assert_eq!(r, x);
    }

    #[test]
    fn stays_finite_at_full_feedback() {
        let mut d = BeatDelay::new(44100.0);
        d.set(&params(&[("time", 0.0), ("feedback", 1.0), ("crossover", 1.0), ("color", 0.0), ("split", 1.0)]));
        d.set_tempo(200.0);
        let mut l: Vec<f32> = (0..200_000).map(|i| ((i * 7919) % 1000) as f32 / 500.0 - 1.0).collect();
        let mut r = l.clone();
        run(&mut d, &mut l, &mut r);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
    }

    #[test]
    fn decays_to_silence() {
        let mut d = BeatDelay::new(48000.0);
        d.set(&params(&[("time", 1.0), ("feedback", 0.3)]));
        let mut l = vec![0f32; 480_000];
        let mut r = vec![0f32; 480_000];
        l[0] = 1.0;
        run(&mut d, &mut l, &mut r);
        let tail = l[470_000..].iter().chain(&r[470_000..]).fold(0f32, |m, v| m.max(v.abs()));
        assert!(tail < 1e-9, "{tail}");
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
    }

    #[test]
    fn changes_keep_the_line() {
        let mut d = BeatDelay::new(48000.0);
        let p = [("time", 1.0), ("mix", 1.0), ("stereo", 1.0), ("feedback", 0.0), ("split", 0.0)];
        d.set(&params(&p));
        let mut l = vec![0f32; 4000];
        let mut r = vec![0f32; 4000];
        l[0] = 1.0;
        run(&mut d, &mut l, &mut r);
        // A change mid-way doesn't clear the line: the echo still comes (high-passed,
        // and the color lanes see the tap one sample late).
        d.set(&params(&[p[0], p[1], p[2], p[3], p[4], ("color", 0.55)]));
        let mut l = vec![0f32; 4000];
        let mut r = vec![0f32; 4000];
        run(&mut d, &mut l, &mut r);
        assert_eq!(peak(&l), 2001);
        assert!(l[..2001].iter().all(|v| v.abs() < 1e-6));
    }
}
