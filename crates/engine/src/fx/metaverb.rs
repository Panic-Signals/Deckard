//! Maschine's Metaverb (docs/fx-metaverb.md): `RT::MetaverbProcessor` →
//! `FXCORE::MetaverbCore`, a hand-vectorised 4-lane kernel (two lanes per channel). The
//! input is scaled and run through a damping low shelf, then a high shelf (both cut-only
//! bilinear one-poles), into a tank of 8 modulated delay lines (Hermite reads, eight
//! phases of one parabolic LFO) wired as allpass pairs. The last two lines feed back
//! crosswise through a decay gain set by Size. The 15 control values ramp linearly
//! (`NI::AB::SmoothingToolbox<15>`) over ≈100 ms, started on 128-sample control ticks.
//! All `f32`, with `mul_add` exactly where the kernel uses `fmadd`/`fmla`, so the output
//! can match bit for bit.

use super::{Effect, exp_e, param};

/// One delay line: 16384 samples plus 3 guard samples mirroring the first three, so a
/// 4-sample Hermite read never wraps.
const LINE: usize = 16387;
const MASK: u32 = 0x3fff;
/// The kernel's anti-denormal offset (`0x129e74d2`, 1e-27).
const TINY: f32 = f32::from_bits(0x129e_74d2);
/// `exp_e(p·5 + PITCH_HZ)` = 8.1758·2^(p/12) Hz, the frequency of MIDI pitch `p`.
const PITCH_HZ: f32 = f32::from_bits(0x44ac_bc36); // 1381.8816
/// `exp_e(dB·K_DB + 1200)` = 10^(dB/20).
const K_DB: f32 = f32::from_bits(0x411f_73da); // 9.965784
const TWO_PI: f32 = f32::from_bits(0x40c9_0fdb);
const LFO_HZ: f32 = f32::from_bits(0x3fc1_f07c); // 1.5151515
const DEPTH_K: f32 = f32::from_bits(0x3a03_126f); // 0.0005
const LEN_K: f32 = f32::from_bits(0x3c75_0312); // 0.014954345
const POS_K: f32 = f32::from_bits(0x3f34_fdf4); // 0.707
const SZ_A: f32 = f32::from_bits(0x3f30_a3d7); // 0.69
const SZ_B: f32 = f32::from_bits(0x3e9e_b852); // 0.31
const OPEN: f32 = f32::from_bits(0x3f4c_cccd); // 0.8
const IN_K: f32 = f32::from_bits(0x3f66_6666); // 0.9
const SZ_MIN: f32 = f32::from_bits(0x38d1_b717); // 1e-4
/// Decay floor: a gain of 6.3e-8 (−144 dB) once the exponent is at or below −144 dB.
const DECAY_FLOOR: f32 = f32::from_bits(0x3387_7f3f);
/// The smoother's "already there" threshold on the squared per-sample step (f64 1e-15).
const SNAP: f64 = 1e-15;
/// Control period (samples), fixed by `beginInit`.
const PERIOD: i32 = 128;
/// LFO phase offsets of the 8 lines, in cycles.
const PH_A: [f32; 4] = [0.0, 0.125, 0.5, 0.625];
const PH_B: [f32; 4] = [0.25, 0.375, 0.75, 0.875];
/// Line lengths as multiples of the Size lengths (lanes 0,2 left, 1,3 right).
const LEN_A: [f32; 4] = [f32::from_bits(0x3f01_a310), f32::from_bits(0x3f01_a310), 1.0, 1.0];
const LEN_B: [f32; 4] =
    [f32::from_bits(0x3f4d_c92e), f32::from_bits(0x3f4d_c92e), f32::from_bits(0x3fbf_c887), f32::from_bits(0x3fbf_c887)];

// Smoothed values (`SmoothingToolbox<15>` lanes; the target of lane k is core 0x110 + 4k).
const LEN_L: usize = 0; // base delay, samples (Size)
const LEN_R: usize = 1;
const DEPTH: usize = 2; // modulation depth (Size)
const IN_GAIN: usize = 3; // input gain into the tank (Size)
const DIFF: usize = 4; // allpass coefficient, negative (Size)
const HI_W: usize = 5; // tan(π·f/sr) of the high shelf
const HI_G: usize = 6; // high-shelf gain² − 1
const LO_W: usize = 7; // tan(π·f/sr) of the low shelf
const DECAY: usize = 8; // decay rate (Size)
const WET: usize = 9;
const DRY_L: usize = 10;
const DRY_R: usize = 11;
const SEND_L: usize = 12; // Pan × Size: input balance into the tank
const SEND_R: usize = 13;
const LO_G: usize = 14; // low-shelf gain² − 1

/// `FXCORE::MetaverbCore`'s state (§3).
struct Core {
    // Event inputs (stored values): Size, Low, High, Pan (−1..1), Mix.
    in_size: f32,
    in_lo: f32,
    in_hi: f32,
    in_pos: f32,
    in_mix: f32,
    sr: f32,
    inv_sr: f32,
    /// Mix's dry factor (+0x30) and Pan's per-side factors (+0x34, +0x38).
    mix_dry: f32,
    pos_l: f32,
    pos_r: f32,
    // Shelf coefficients (+0x40 … +0x70); lanes 0,1 low shelf, lanes 2,3 high shelf.
    c_x: [f32; 4],
    c_s: [f32; 4],
    c_n: [f32; 4],
    c_g: [f32; 4],
    // The smoother (+0x80): current values, per-sample steps, targets.
    cur: [f32; 16],
    inc: [f32; 16],
    tgt: [f32; 16],
    dirty: bool,
    count: i32,
    steps: i32,
    inv_steps: f32,
    /// Compute both shelves on the next sample (+0x170).
    init: bool,
    widx: u32,
    phase: f32,
    lfo_inc: f32,
    // Shelf state (+0x1b0 … +0x1e0): previous input (negated for the high pass), output,
    // feedback into the input, filter state.
    f_x1: [f32; 4],
    f_out: [f32; 4],
    f_fb: [f32; 4],
    f_s: [f32; 4],
    /// The second allpass stage's output of the previous sample (+0x80290).
    y_prev: [f32; 4],
    ctr: i32,
    lines: Vec<f32>,
}

impl Core {
    /// `MetaverbProcessor()`: a zeroed core, `beginInit(44100)`, `endInit()`.
    fn new(sr: f32) -> Core {
        let mut c = Core {
            in_size: 0.0,
            in_lo: 0.0,
            in_hi: 0.0,
            in_pos: 0.0,
            in_mix: 0.0,
            sr,
            inv_sr: 0.0,
            mix_dry: 0.0,
            pos_l: 0.0,
            pos_r: 0.0,
            c_x: [0.0; 4],
            c_s: [0.0; 4],
            c_n: [0.0; 4],
            c_g: [0.0; 4],
            cur: [0.0; 16],
            inc: [0.0; 16],
            tgt: [0.0; 16],
            dirty: false,
            count: 0,
            steps: 0,
            inv_steps: 0.0,
            init: false,
            widx: 0,
            phase: 0.0,
            lfo_inc: 0.0,
            f_x1: [0.0; 4],
            f_out: [0.0; 4],
            f_fb: [0.0; 4],
            f_s: [0.0; 4],
            y_prev: [0.0; 4],
            ctr: 0,
            lines: vec![0.0; 8 * LINE],
        };
        c.begin_init();
        c.end_init(0);
        c
    }

    /// `beginInit(sr)` (§3.1): clears the state; the event inputs stay.
    fn begin_init(&mut self) {
        self.init = true;
        self.ctr = PERIOD + 1;
        self.cur = [0.0; 16];
        self.inc = [0.0; 16];
        self.tgt = [0.0; 16];
        self.dirty = false;
        self.count = 0;
        self.steps = 0;
        self.inv_steps = 0.0;
        self.widx = 0;
        self.inv_sr = 1.0 / self.sr;
        self.c_g = [0.0; 4];
        self.mix_dry = 0.1;
        self.pos_l = 0.1;
        self.pos_r = 0.1;
        self.phase = 0.0;
        self.lfo_inc = self.inv_sr * LFO_HZ;
        self.lines.fill(0.0);
        self.f_x1 = [0.0; 4];
        self.f_out = [0.0; 4];
        self.f_fb = [0.0; 4];
        self.f_s = [0.0; 4];
        self.y_prev = [0.0; 4];
    }

    /// `endInit()` (§3.1): the smoothing time, then the events whose flags are set (bit 0
    /// Size … bit 4 Mix); every smoothed value then jumps to its target.
    fn end_init(&mut self, flags: u8) {
        self.inv_sr = 1.0 / self.sr;
        let n = ((self.sr * 0.1) / PERIOD as f32 + 0.5) as i32;
        self.steps = if n > 1 { n } else { 1 };
        self.inv_steps = 1.0 / (self.steps * PERIOD) as f32;
        if flags & 1 != 0 {
            self.ev_size();
        }
        if flags & 2 != 0 {
            self.ev_lo();
        }
        if flags & 4 != 0 {
            self.ev_hi();
        }
        if flags & 8 != 0 {
            self.ev_pos();
        }
        if flags & 16 != 0 {
            self.ev_mix();
        }
        self.cur = self.tgt;
        self.dirty = true;
    }

    /// `MetaverbProcessor::resume`: `beginInit(sr)` keeping the inputs, every event flag
    /// set, `endInit()`.
    fn resume(&mut self) {
        self.begin_init();
        self.end_init(0x1f);
    }

    /// The part of the Size and Pan events shared by both: the input balance into the tank.
    fn send_targets(&mut self) {
        let s = (self.in_size * POS_K) * self.in_pos;
        self.tgt[SEND_L] = if s > 0.0 { 1.0 - s } else { 1.0 };
        self.tgt[SEND_R] = if s < 0.0 { s + 1.0 } else { 1.0 };
    }

    /// `processEventInSz` (§3.2).
    fn ev_size(&mut self) {
        let v = self.in_size;
        let sz = v * v.mul_add(SZ_A, SZ_B);
        self.tgt[DECAY] = exp_e((sz * -96.0).mul_add(5.0, PITCH_HZ)) * self.inv_sr;
        // fmla: (0.5, 0.9) + (1.5 − v, sz)·(2v, −0.5·sz)
        self.tgt[DEPTH] = (1.5 - v).mul_add(v * 2.0, 0.5);
        self.tgt[IN_GAIN] = sz.mul_add(sz * -0.5, IN_K);
        let open = 1.0 - sz;
        let e = (open + -OPEN) * 5.0;
        let pitch = if open > OPEN { e * (e * 12.0) + 30.0 } else { 30.0 };
        let p_l = sz.mul_add(-16.0, pitch);
        let p_r = p_l - (v * (2.0 - v)).mul_add(2.0, 1.0);
        let e_l = exp_e(p_l.mul_add(-5.0, PITCH_HZ));
        let e_r = exp_e(p_r.mul_add(-5.0, PITCH_HZ));
        self.tgt[LEN_L] = (e_l * LEN_K) * self.sr;
        self.tgt[LEN_R] = (e_r * LEN_K) * self.sr;
        let shape = if open > OPEN { (e * -0.4).mul_add(e, 1.0) } else { 1.0 };
        let s = if sz < SZ_MIN { SZ_MIN } else { sz };
        self.tgt[DIFF] = -(shape * s.mul_add(f32::from_bits(0x3d2a_aaab), 0.5));
        self.send_targets();
        self.dirty = true;
    }

    /// `processEventInLo` (§3.3): low shelf, 0 … −18 dB, corner MIDI pitch 64 … 40.
    fn ev_lo(&mut self) {
        let a = 1.0 - self.in_lo;
        let db = (a * (2.0 - a)) * -18.0;
        let g = exp_e((db * 0.5).mul_add(K_DB, 1200.0));
        let gi = exp_e((db * -0.5).mul_add(K_DB, 1200.0));
        self.tgt[LO_G] = g.mul_add(g, -1.0);
        let f = exp_e(a.mul_add(24.0, 40.0).mul_add(5.0, PITCH_HZ));
        self.tgt[LO_W] = ((self.inv_sr * 0.5) * ((gi * f) * TWO_PI)).tan();
        self.dirty = true;
    }

    /// `processEventInHi` (§3.3): high shelf, 0 … −12 dB, corner MIDI pitch 120 … 86.
    fn ev_hi(&mut self) {
        let a = 1.0 - self.in_hi;
        let db = ((a * (2.0 - a)) * -12.0) * 0.5;
        let g = exp_e(db.mul_add(K_DB, 1200.0));
        self.tgt[HI_G] = g.mul_add(g, -1.0);
        let f = exp_e(a.mul_add(-34.0, 120.0).mul_add(5.0, PITCH_HZ));
        self.tgt[HI_W] = ((self.inv_sr * 0.5) * ((g * f) * TWO_PI)).tan();
        self.dirty = true;
    }

    /// `processEventInPos` (§3.4).
    fn ev_pos(&mut self) {
        let p = self.in_pos;
        self.pos_l = if p < 0.0 { 1.0 } else { 1.0 - p };
        self.pos_r = if p > 0.0 { 1.0 } else { p + 1.0 };
        self.tgt[DRY_L] = self.pos_l * self.mix_dry;
        self.tgt[DRY_R] = self.pos_r * self.mix_dry;
        self.send_targets();
        self.dirty = true;
    }

    /// `processEventInMix` (§3.4).
    fn ev_mix(&mut self) {
        let m = self.in_mix;
        let low = m < 0.5;
        self.mix_dry = if low { 1.0 } else { (1.0 - m) + (1.0 - m) };
        self.tgt[WET] = if low { (m + m) * IN_K } else { IN_K };
        self.tgt[DRY_L] = self.pos_l * self.mix_dry;
        self.tgt[DRY_R] = self.pos_r * self.mix_dry;
        self.dirty = true;
    }

    /// `SmoothingToolbox<15>::startSmooth`: a linear ramp to every target over the
    /// smoothing time; values whose step would be negligible jump at once.
    fn start_smooth(&mut self) {
        let mut moving = false;
        for k in 0..16 {
            let d = (self.tgt[k] - self.cur[k]) * self.inv_steps;
            if ((d * d) as f64) < SNAP {
                self.inc[k] = 0.0;
                self.cur[k] = self.tgt[k];
            } else {
                self.inc[k] = d;
                moving = true;
            }
        }
        if moving {
            self.count = self.steps;
        }
    }

    /// The Hermite read of line `k`, `d` samples back (§3.5).
    fn tap(&self, k: usize, d: f32) -> f32 {
        let n = d as i32; // fcvtzs
        let t = (d.trunc() - d) + 1.0;
        let i = k * LINE + (self.widx.wrapping_add(!(n as u32)) & MASK) as usize;
        let (a0, a1, a2, a3) = (self.lines[i], self.lines[i + 1], self.lines[i + 2], self.lines[i + 3]);
        let c1 = (a2 - a0) * 0.5;
        let c3 = c1 + (a3 - a1) * 0.5;
        let d21 = a2 - a1;
        let p = ((d21 * 3.0) - c3) - c1;
        let q = c3 - (d21 + d21);
        (a1 + t * c1) + (t * t) * (p + t * q)
    }

    /// `processAudio()` for one sample (§3.5).
    fn tick(&mut self, xl: f32, xr: f32) -> (f32, f32) {
        for k in 0..16 {
            self.cur[k] += self.inc[k];
        }
        self.ctr += 1;
        if self.ctr >= PERIOD {
            self.ctr = 0;
            if self.dirty {
                self.dirty = false;
                self.start_smooth();
            } else {
                self.count = self.count.wrapping_sub(1);
                if self.count == 0 {
                    self.cur = self.tgt;
                    self.inc = [0.0; 16];
                }
            }
        }
        // Shelf coefficients, while their values move (and after a resume).
        if self.inc[LO_W] != 0.0 || self.inc[LO_G] != 0.0 || self.init {
            let w = self.cur[LO_W];
            let a = 1.0 / (w + 1.0);
            self.c_x = [w, w, 1.0, 1.0];
            self.c_s[..2].fill(w + -1.0);
            self.c_n[..2].fill(a);
            self.c_g[..2].fill(self.cur[LO_G]);
        }
        if self.inc[HI_W] != 0.0 || self.inc[HI_G] != 0.0 || self.init {
            let w = self.cur[HI_W];
            let a = 1.0 / (w + 1.0);
            self.c_s[2..].fill(w + -1.0);
            self.c_n[2..].fill(a);
            self.c_g[2..].fill(self.cur[HI_G]);
            self.init = false;
        }

        // LFO: one phase in [−0.5, 0.5), eight offsets, parabolic sine x·(8 − 16|x|).
        let p = self.phase + self.lfo_inc;
        let p = if p < 0.5 { p } else { p + -1.0 };
        self.phase = p;
        let depth = (self.cur[DEPTH] * self.sr) * DEPTH_K;
        let lfo = |o: f32| {
            let x = p + o;
            let x = if x < 0.5 { x } else { x + -1.0 };
            depth + (x * (8.0 - x.abs() * 16.0)) * depth
        };

        // Input gain and balance plus the feedback, then the shelves: lanes 0,1 low shelf
        // (L, R), lanes 2,3 high shelf on the low shelf's previous output.
        let g = self.cur[IN_GAIN];
        let x = [
            self.f_fb[0] + g * (xl * self.cur[SEND_L] + TINY),
            self.f_fb[1] + g * (xr * self.cur[SEND_R] + TINY),
            self.f_fb[2] + self.f_out[0],
            self.f_fb[3] + self.f_out[1],
        ];
        for (k, &x) in x.iter().enumerate() {
            let s = self.c_n[k] * (self.c_x[k] * (x + self.f_x1[k]) - self.c_s[k] * self.f_s[k]) + TINY;
            self.f_s[k] = s;
            self.f_out[k] = x + self.c_g[k] * s;
            self.f_x1[k] = if k < 2 { x } else { -x }; // x·(1, 1, −1, −1)
        }

        // Delay times (samples) and reads of the 8 lines.
        let lens = [self.cur[LEN_L], self.cur[LEN_R], self.cur[LEN_L], self.cur[LEN_R]];
        let mut db = [0.0f32; 4];
        let mut ta = [0.0f32; 4];
        let mut tb = [0.0f32; 4];
        for k in 0..4 {
            db[k] = lfo(PH_B[k]) + lens[k] * LEN_B[k];
            ta[k] = self.tap(k, lfo(PH_A[k]) + lens[k] * LEN_A[k]);
            tb[k] = self.tap(4 + k, db[k]);
        }

        // Two allpass stages: lines 0–3 hold the first, lines 4,5 the second (lanes 2,3 of
        // the second stage are plain delays).
        let gd = self.cur[DIFF];
        let gm = [gd, gd, 0.0, 0.0];
        let v = [self.f_out[2], self.f_out[3], self.y_prev[0], self.y_prev[1]];
        let mut u = [0.0f32; 4];
        let w = (self.widx + 1) & MASK;
        for k in 0..4 {
            u[k] = ta[k] - v[k] * gd;
            let y = tb[k] - gm[k] * u[k];
            self.y_prev[k] = y;
            let wa = (v[k] + u[k] * gd) + TINY;
            let wb = (u[k] + gm[k] * y) + TINY;
            let (ia, ib) = (k * LINE + w as usize, (4 + k) * LINE + w as usize);
            self.lines[ia] = wa;
            self.lines[ib] = wb;
            if w <= 2 {
                self.lines[ia + 0x4000] = wa;
                self.lines[ib + 0x4000] = wb;
            }
        }
        self.widx = w;

        // Decay 10^(−60·delay·rate/20) on the two plain delays, fed back crosswise.
        let gain = |d: f32| {
            let x = (d * self.cur[DECAY]) * -60.0;
            if x <= -144.0 { DECAY_FLOOR } else { exp_e(x.mul_add(K_DB, 1200.0)) }
        };
        let y = self.y_prev;
        self.f_fb = [y[3] * gain(db[3]), y[2] * gain(db[2]), 0.0, 0.0];

        let wet = self.cur[WET];
        (u[2] * wet + xl * self.cur[DRY_L], u[3] * wet + xr * self.cur[DRY_R])
    }
}

/// The Metaverb effect: the parameters (`size`, `low`, `high`, `pan`, `mix`) go to the
/// core as events.
pub struct Metaverb {
    core: Core,
    started: bool,
}

impl Metaverb {
    pub fn new(sr: f64) -> Self {
        Metaverb { core: Core::new(sr as f32), started: false }
    }
}

impl Effect for Metaverb {
    fn set(&mut self, params: &[(String, f64)]) {
        let c = &mut self.core;
        let v = [
            param(params, "size", 0.5) as f32,
            param(params, "low", 0.5) as f32,
            param(params, "high", 0.5) as f32,
            param(params, "pan", 0.0) as f32,
            param(params, "mix", 0.5) as f32,
        ];
        if !self.started {
            // The processor resumes with the stored values.
            self.started = true;
            [c.in_size, c.in_lo, c.in_hi, c.in_pos, c.in_mix] = v;
            c.resume();
            return;
        }
        // processFloatParameter: each changed input, with its event.
        if v[0] != c.in_size {
            c.in_size = v[0];
            c.ev_size();
        }
        if v[1] != c.in_lo {
            c.in_lo = v[1];
            c.ev_lo();
        }
        if v[2] != c.in_hi {
            c.in_hi = v[2];
            c.ev_hi();
        }
        if v[3] != c.in_pos {
            c.in_pos = v[3];
            c.ev_pos();
        }
        if v[4] != c.in_mix {
            c.in_mix = v[4];
            c.ev_mix();
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            (*a, *b) = self.core.tick(*a, *b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(size: f64, low: f64, high: f64, pan: f64, mix: f64) -> Vec<(String, f64)> {
        [("size", size), ("low", low), ("high", high), ("pan", pan), ("mix", mix)].iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn noise(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i * 7919 + 13) % 1000) as f32 / 1000.0 - 0.5).collect()
    }

    fn run(fx: &mut Metaverb, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut a, mut b) = (l.to_vec(), r.to_vec());
        for (x, y) in a.chunks_mut(512).zip(b.chunks_mut(512)) {
            fx.process(x, y);
        }
        (a, b)
    }

    fn energy(x: &[f32]) -> f64 {
        x.iter().map(|v| (*v as f64) * (*v as f64)).sum()
    }

    #[test]
    fn impulse_rings_and_decays() {
        let mut fx = Metaverb::new(44_100.0);
        fx.set(&p(0.5, 0.5, 0.5, 0.0, 1.0));
        let mut x = vec![0.0; 44_100 * 6];
        x[0] = 1.0;
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        // Mix 100 %: no dry signal, a tail that dies away.
        let early = energy(&l[1000..44_100]);
        let late = energy(&l[5 * 44_100..]);
        assert!(early > 1e-3, "{early}");
        assert!(late < early * 1e-4, "{early} {late}");
        // The two sides differ (different line lengths).
        assert!(l.iter().zip(&r).any(|(a, b)| a != b));
    }

    #[test]
    fn bigger_size_rings_longer() {
        let tail = |size: f64| {
            let mut fx = Metaverb::new(48_000.0);
            fx.set(&p(size, 1.0, 1.0, 0.0, 1.0));
            let mut x = vec![0.0; 48_000 * 3];
            x[0] = 1.0;
            let (l, _) = run(&mut fx, &x, &x);
            energy(&l[2 * 48_000..])
        };
        assert!(tail(0.9) > tail(0.3) * 10.0);
    }

    #[test]
    fn dry_mix_is_the_identity() {
        let mut fx = Metaverb::new(44_100.0);
        fx.set(&p(0.7, 0.3, 0.6, 0.0, 0.0));
        let x = noise(20_000);
        let (l, r) = run(&mut fx, &x, &x);
        assert_eq!(l, x);
        assert_eq!(r, x);
    }

    #[test]
    fn set_keeps_state_and_ramps() {
        let x = noise(40_000);
        let mut a = Metaverb::new(44_100.0);
        a.set(&p(0.4, 0.6, 0.5, 0.2, 0.5));
        let (l1, _) = run(&mut a, &x[..10_000], &x[..10_000]);
        // Re-sending the same values changes nothing.
        a.set(&p(0.4, 0.6, 0.5, 0.2, 0.5));
        let (l2, _) = run(&mut a, &x[10_000..], &x[10_000..]);
        let mut b = Metaverb::new(44_100.0);
        b.set(&p(0.4, 0.6, 0.5, 0.2, 0.5));
        let (lb, _) = run(&mut b, &x, &x);
        assert_eq!([l1, l2].concat(), lb);

        // Mix 0 → 1 ramps over 34 control ticks of 128 samples, then lands on the target.
        let mut c = Metaverb::new(44_100.0);
        c.set(&p(0.4, 0.6, 0.5, 0.0, 0.0));
        run(&mut c, &x[..1000], &x[..1000]);
        c.set(&p(0.4, 0.6, 0.5, 0.0, 1.0));
        run(&mut c, &x[..2000], &x[..2000]);
        let d = c.core.cur[DRY_L];
        assert!(d > 0.0 && d < 1.0, "{d}");
        run(&mut c, &x[..4000], &x[..4000]);
        assert_eq!(c.core.cur[DRY_L], 0.0);
        assert_eq!(c.core.cur[WET], IN_K);
    }

    #[test]
    fn silence_stays_finite_and_loud_input_bounded() {
        let mut fx = Metaverb::new(96_000.0);
        fx.set(&p(0.5, 1.0, 1.0, -1.0, 1.0));
        let x: Vec<f32> = noise(200_000).iter().map(|v| v * 8.0).collect();
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 1000.0));
        let z = vec![0.0; 500_000];
        let (l, r) = run(&mut fx, &z, &z);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        // The tail dies away (Size 50 %: RT60 ≈ 0.75 s; Size 100 % would be ≈ 31 s).
        let (e0, e1) = (energy(&l[..100_000]), energy(&l[400_000..]));
        assert!(e1 < e0 * 1e-4, "{e0} {e1}");
    }
}
