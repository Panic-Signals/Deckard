//! `NI::DSP::GaloisReverb`, the Room and Hall modes of Maschine's Reverb
//! (docs/fx-plate-reverb.md §3).
//!
//! A 31-line feedback delay network whose feedback matrix is a circulant built from a
//! length-31 two-valued sequence (hence "Galois"), with prime delay lengths, an LFO on the
//! line lengths, per-line one-pole damping set from a two-band T60, then per output channel
//! a chain of nested allpass diffusers (4 for Room, 2 for Hall), a low-shelf EQ, a one-pole
//! high cut and a dry/wet mix with a stereo width control. Room and Hall share the code;
//! the `hall` flag changes the line lengths, the diffusers, the input matrix and the output
//! matrix (blended by Diffusion).
//!
//! Matrix-vector products are sequential fused dot products, as Eigen's NEON kernel does
//! them; everything else follows the binary's operation order, `mul_add` where it fuses.

#![allow(clippy::needless_range_loop)] // lane-indexed loops mirror the binary's SIMD lanes
use super::{c, exp10f};

/// Lines of the network (the matrices are 32 wide; the 32nd lane stays silent).
const N: usize = 31;
const W: usize = 32;
/// The two-valued sequence of the circulant feedback matrix: `A` = −0.20333, `B` = 0.15022.
const SEQ: &[u8; 31] = b"AABAAABABABBBBABBABAABBAAAAABBB";
const SEQ_A: f32 = c(0xbe50_3656);
const SEQ_B: f32 = c(0x3e19_d390);
/// Swaps applied to the identity to give each line its step in the prime table.
const SWAPS_ROOM: [usize; 6] = [0, 1, 7, 8, 20, 21];
const SWAPS_HALL: [usize; 4] = [0, 1, 6, 7];
/// Per line, an offset into the prime table.
const OFFSET: [i32; N] = [4, 3, 5, 5, 5, 5, 2, 1, 1, 1, 4, 5, 1, 5, 1, 4, 3, 4, 2, 4, 1, 2, 2, 5, 4, 2, 3, 1, 3, 0, 3];
/// Input routing (32 × 2): which lines the left and right pre-delayed input feed.
const IN_L: [f32; W] = [
    1.0, 0.0, 1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0,
    -1.0, 0.0, -1.0, 0.0, 0.0, 0.0,
];
const IN_R: [f32; W] = [
    0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 1.0, 0.0, 0.0,
];
/// Output taps as (line, left, right).
const OUT_ROOM: [(usize, f32, f32); 16] = [
    (0, 1.0, 0.0),
    (1, 0.0, 1.0),
    (2, 0.0, -1.0),
    (3, -1.0, 0.0),
    (4, 1.0, 0.0),
    (5, 0.0, 1.0),
    (8, -1.0, 0.0),
    (9, 0.0, -1.0),
    (12, -1.0, 0.0),
    (13, 0.0, -1.0),
    (16, 1.0, 0.0),
    (17, 0.0, 1.0),
    (20, -1.0, 0.0),
    (21, 0.0, -1.0),
    (24, 1.0, 0.0),
    (25, 0.0, 1.0),
];
/// Hall: two output matrices, blended by Diffusion. The second scales the first four
/// taps by `0x3ee4f92e` (0.447) and the rest by `0x3f900f2b` (1.126).
const OUT_HALL: [(usize, f32, f32); 16] = [
    (0, 1.0, 0.0),
    (1, 0.0, 1.0),
    (2, 0.0, -1.0),
    (3, -1.0, 0.0),
    (6, -1.0, 0.0),
    (7, 0.0, 1.0),
    (8, 0.0, -1.0),
    (9, 1.0, 0.0),
    (12, -1.0, 0.0),
    (13, 0.0, 1.0),
    (16, 0.0, -1.0),
    (17, 1.0, 0.0),
    (20, -1.0, 0.0),
    (21, 0.0, 1.0),
    (24, 0.0, -1.0),
    (25, 1.0, 0.0),
];
/// Diffuser delay lengths per stage (three lines each).
const DIFF_ROOM: [[i32; 3]; 4] = [[31, 79, 41], [43, 73, 53], [59, 61, 67], [71, 47, 37]];
const DIFF_HALL: [[i32; 3]; 2] = [[73, 181, 97], [103, 179, 127]];
/// Pre-delay buffer length (two lines).
const PD_LEN: i32 = 16384;

/// `__sincos_stret` from the system math library.
#[cfg(target_os = "macos")]
fn sincos(x: f64) -> (f64, f64) {
    #[repr(C)]
    struct D2 {
        s: f64,
        c: f64,
    }
    unsafe extern "C" {
        fn __sincos_stret(x: f64) -> D2;
    }
    // SAFETY: a pure libm function.
    let r = unsafe { __sincos_stret(x) };
    (r.s, r.c)
}
#[cfg(not(target_os = "macos"))]
fn sincos(x: f64) -> (f64, f64) {
    x.sin_cos()
}

/// `__sincosf_stret` from the system math library.
#[cfg(target_os = "macos")]
fn sincosf(x: f32) -> (f32, f32) {
    #[repr(C)]
    struct F2 {
        s: f32,
        c: f32,
    }
    unsafe extern "C" {
        fn __sincosf_stret(x: f32) -> F2;
    }
    // SAFETY: a pure libm function.
    let r = unsafe { __sincosf_stret(x) };
    (r.s, r.c)
}
#[cfg(not(target_os = "macos"))]
fn sincosf(x: f32) -> (f32, f32) {
    x.sin_cos()
}

/// `GaloisReverb::PrimeNums::prime_nums`: the primes from 2 on.
fn primes() -> &'static [i32] {
    static P: std::sync::OnceLock<Vec<i32>> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let n = 30000;
        let mut sieve = vec![true; n];
        let mut out = Vec::new();
        for i in 2..n {
            if sieve[i] {
                out.push(i as i32);
                let mut j = i * i;
                while j < n {
                    sieve[j] = false;
                    j += i;
                }
            }
        }
        out
    })
}

/// A power-of-two circular buffer with a write index (`GaloisReverb::DelayLine`).
#[derive(Clone)]
struct Line {
    buf: Vec<f32>,
    w: u32,
    mask: u32,
}

impl Line {
    fn new(len: i32) -> Line {
        let size = if len > 1 { (len as u32).next_power_of_two() } else { 1 };
        Line { buf: vec![0.0; size as usize], w: 0, mask: size - 1 }
    }

    /// Linear interpolation `d` samples behind the write position.
    fn read_frac(&self, d: f32) -> f32 {
        let n = d as i32;
        let frac = d - n as f32;
        let a = self.buf[(self.w.wrapping_sub(n as u32) & self.mask) as usize];
        let b = self.buf[(self.w.wrapping_add(!(n as u32)) & self.mask) as usize];
        a.mul_add(1.0 - frac, frac * b)
    }
}

/// `AllPassNested<3>`: three delay lines in a 3×3 feedback matrix.
#[derive(Clone)]
struct Nested {
    /// `a[col][row]`: the coefficient of state `z[col]` in row `u[row]`.
    a: [[f32; 3]; 3],
    /// Input gains per row.
    b: [f32; 3],
    /// Output: coefficients of `z[0]` and of the input.
    c_z: f32,
    c_x: f32,
    z: [f32; 3],
    lines: [Line; 3],
    len: [i32; 3],
}

impl Nested {
    fn new(len: [i32; 3]) -> Nested {
        Nested {
            a: [[0.0; 3]; 3],
            b: [0.0; 3],
            c_z: 0.0,
            c_x: 0.0,
            z: [0.0; 3],
            lines: [Line::new(len[0]), Line::new(len[1]), Line::new(len[2])],
            len,
        }
    }

    fn tick(&mut self, x: f32) -> f32 {
        let [z0, z1, z2] = self.z;
        let a = &self.a;
        let u = [
            a[2][0].mul_add(z2, a[1][0].mul_add(z1, a[0][0] * z0)) + self.b[0] * x,
            a[2][1].mul_add(z2, a[1][1].mul_add(z1, a[0][1] * z0)) + self.b[1] * x,
            a[0][2] * z0 + (a[1][2] * z1 + a[2][2] * z2) + x * self.b[2],
        ];
        for j in 0..3 {
            let l = &mut self.lines[j];
            l.buf[(l.w & l.mask) as usize] = u[j];
            let w1 = l.w.wrapping_add(1);
            self.z[j] = l.buf[(w1.wrapping_sub(self.len[j] as u32) & l.mask) as usize];
            l.w = w1 & l.mask;
        }
        self.c_z.mul_add(z0, x * self.c_x)
    }

    fn clear(&mut self) {
        self.z = [0.0; 3];
        for l in &mut self.lines {
            l.buf.fill(0.0);
        }
    }
}

/// One output channel's effects (`GaloisReverb::ChannelEffects`).
#[derive(Clone)]
struct Channel {
    stages: Vec<Nested>,
    // `NI::DSP::EQ`: a state-variable low shelf at 200 Hz (mixes, gains, states).
    m: [f32; 3],
    a1: f32,
    a2: f32,
    a3: f32,
    ic1: f32,
    ic2: f32,
    /// One-pole high cut: coefficient and state.
    hc: f32,
    hc_s: f32,
}

impl Channel {
    fn tick(&mut self, x: f32) -> f32 {
        let mut s = x;
        for st in &mut self.stages {
            s = st.tick(s);
        }
        let v3 = s - self.ic2;
        let v1 = self.a1.mul_add(self.ic1, v3 * self.a2);
        let v2 = self.ic2 + self.a2.mul_add(self.ic1, v3 * self.a3);
        self.ic1 = v1.mul_add(2.0, -self.ic1);
        self.ic2 = v2.mul_add(2.0, -self.ic2);
        let y = self.m[2].mul_add(v2, self.m[0].mul_add(s, v1 * self.m[1]));
        let y = self.hc.mul_add(self.hc_s - y, y);
        self.hc_s = y;
        y
    }

    /// `EQ::setParams(200, 1, gain_db, 44100, LowShelf)`.
    fn set_shelf(&mut self, gain_db: f32) {
        let a = exp10f(gain_db / 40.0);
        let r = 1.0 / a.sqrt();
        let w = ((200.0 * std::f64::consts::PI) / 44100.0) as f32;
        let w2 = w * w;
        let w3 = w2 * w;
        let w5 = w2 * w3;
        let g = w5.mul_add(c(0x3e08_8889), w3.mul_add(c(0x3eaa_aaab), w)) * r;
        let k = 1.0f32;
        self.a1 = 1.0 / g.mul_add(k + g, 1.0);
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
        self.m = [1.0, (a + -1.0) / 1.0, a.mul_add(a, -1.0)];
    }
}

/// `NI::DSP::SmoothingToolbox<L>`: linear parameter ramps over `steps` control ticks.
struct Ramp<const L: usize> {
    tgt: [f32; L],
    cur: [f32; L],
    start: [f32; L],
    end: [f32; L],
    delta: [f32; L],
    /// Samples into the ramp (a float per lane).
    count: [f32; L],
    /// `1/(steps·128)`.
    inv: f32,
    steps: i32,
    countdown: i32,
    /// 0 idle, 2 started, 3 running, 4 finished.
    state: i32,
    dirty: bool,
    ramping: bool,
}

impl<const L: usize> Ramp<L> {
    fn new() -> Self {
        Ramp {
            tgt: [0.0; L],
            cur: [0.0; L],
            start: [0.0; L],
            end: [0.0; L],
            delta: [0.0; L],
            count: [0.0; L],
            inv: 0.0,
            steps: 0,
            countdown: 0,
            state: 0,
            dirty: false,
            ramping: false,
        }
    }

    /// `startSmooth()`: ramp from the current values to the targets. Lanes closer than
    /// one ulp of the current value jump there.
    fn start_smooth(&mut self) {
        self.end = self.tgt;
        self.start = self.cur;
        self.count = [0.0; L];
        let mut any = false;
        for l in 0..L {
            let d = self.end[l] - self.start[l];
            self.delta[l] = d;
            let a = self.cur[l].abs();
            let ulp = if a > 0.0 && a.is_finite() { f32::from_bits(a.to_bits() + 1) - a } else { 0.0 };
            if d.abs() < ulp {
                self.delta[l] = 0.0;
                self.cur[l] = self.end[l];
                self.start[l] = self.end[l];
            }
            any |= d != 0.0;
        }
        if any {
            self.countdown = self.steps;
            self.ramping = true;
            self.state = 2;
        } else {
            self.state = 4;
        }
    }

    /// Jump to the end of the ramp (`endSmooth`), then start again toward the targets.
    fn finish(&mut self) {
        self.cur = self.end;
        self.start = self.end;
        self.delta = [0.0; L];
        self.count = [0.0; L];
        self.state = 4;
        self.ramping = false;
        self.start_smooth();
    }

    /// `setVarsToFinalSmoothingVals` for this toolbox.
    fn set_final(&mut self) {
        self.end = self.tgt;
        self.finish();
    }

    /// The control-rate part of `processControlRate()`.
    fn control(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.start_smooth();
        } else if self.state != 0 {
            self.countdown -= 1;
            if self.countdown == 0 {
                self.finish();
            } else if self.state == 2 {
                self.state = 3;
            } else if self.state == 4 {
                self.state = 0;
            }
        }
    }

    /// One sample of the ramp.
    fn step(&mut self) {
        for l in 0..L {
            self.count[l] += 1.0;
            self.cur[l] = self.start[l] + self.delta[l] * (self.count[l] * self.inv);
        }
    }
}

pub(super) struct Galois {
    hall: bool,
    /// `+0x8`: parameters jump to their values (before the first resume).
    immediate: bool,
    /// Raw parameters 0..9 (`+0x6cec`).
    raw: [f32; 10],
    /// `SmoothingToolbox<11>` (`+0x6d80`): gain, T60, damping, mod depth, diffusion,
    /// pre-delay, left extra delay, high cut, low shelf, mix, width.
    a: Ramp<12>,
    /// `SmoothingToolbox<31>` (`+0x6e80`): the line lengths as floats.
    b: Ramp<W>,
    /// Line lengths from the prime table.
    len_i: [i32; N],
    /// `+0x6ce4`: the low band's per-sample gain and the high/low ratio.
    g_lo: f32,
    ratio: f32,
    /// `+0x6aec`: the lengths read per sample (index 0 is never modulated).
    dlen: [f32; W],
    /// Above 48 kHz the network runs at `sr/k` (§3.7): the decimation factor, the input
    /// being summed and the last two outputs being interpolated.
    k: usize,
    acc: (f32, f32, usize),
    held: [(f32, f32); 2],
    perm: [i32; N],
    // LFO (`+0x6c88`): a rotating phasor in f64.
    re: f64,
    im: f64,
    rot_c: f64,
    rot_s: f64,
    /// Feedback matrix, column-major 32×32.
    m: Vec<f32>,
    out: [[f32; W]; 2],
    out_a: [[f32; W]; 2],
    out_b: [[f32; W]; 2],
    in_l: [f32; W],
    in_r: [f32; W],
    y: [f32; W],
    x: [f32; W],
    s: [f32; W],
    da: [f32; W],
    db: [f32; W],
    lines: Vec<Line>,
    pd: [Line; 2],
    ch: [Channel; 2],
}

impl Galois {
    /// The constructor, `onInit()` and `onSetSampleRate(44100, 128)`.
    pub(super) fn new(hall: bool) -> Box<Galois> {
        let mut m = vec![0.0f32; W * W];
        for r in 0..N {
            for j in 0..N {
                let k = (j + N - r) % N;
                m[r + W * j] = if SEQ[k] == b'A' { SEQ_A } else { SEQ_B };
            }
        }
        let mut perm: [i32; N] = std::array::from_fn(|i| i as i32);
        let swaps: &[usize] = if hall { &SWAPS_HALL } else { &SWAPS_ROOM };
        for (k, &s) in swaps.iter().enumerate() {
            perm.swap(k, s);
        }
        let mut in_l = IN_L;
        let mut in_r = IN_R;
        if hall {
            in_l[4] = 0.0;
            in_l[6] = -0.0;
            in_r[5] = 0.0;
            in_r[7] = -0.0;
        }
        let mut out = [[0.0f32; W]; 2];
        let mut out_a = [[0.0f32; W]; 2];
        let mut out_b = [[0.0f32; W]; 2];
        if hall {
            for (i, &(j, l, r)) in OUT_HALL.iter().enumerate() {
                out_a[0][j] = l;
                out_a[1][j] = r;
                let k = if i < 4 { c(0x3ee4_f92e) } else { c(0x3f90_0f2b) };
                out_b[0][j] = l * k;
                out_b[1][j] = r * k;
            }
        } else {
            for &(j, l, r) in &OUT_ROOM {
                out[0][j] = l;
                out[1][j] = r;
            }
        }
        let stages: Vec<Nested> =
            if hall { DIFF_HALL.iter().map(|l| Nested::new(*l)).collect() } else { DIFF_ROOM.iter().map(|l| Nested::new(*l)).collect() };
        let ch = Channel { stages, m: [0.0; 3], a1: 0.0, a2: 0.0, a3: 0.0, ic1: 0.0, ic2: 0.0, hc: c(0x3d6d_0b76), hc_s: 0.0 };
        let mut g = Box::new(Galois {
            hall,
            immediate: true,
            raw: [0.2, 0.0, 0.01, 0.5, 0.3, 0.0, 0.0, 0.0, 0.5, 1.0],
            a: Ramp::new(),
            b: Ramp::new(),
            len_i: [0; N],
            g_lo: 0.0,
            ratio: 0.0,
            dlen: [0.0; W],
            k: 1,
            acc: (0.0, 0.0, 0),
            held: [(0.0, 0.0); 2],
            perm,
            re: 1.0,
            im: 0.0,
            rot_c: 1.0,
            rot_s: 0.0,
            m,
            out,
            out_a,
            out_b,
            in_l,
            in_r,
            y: [0.0; W],
            x: [0.0; W],
            s: [0.0; W],
            da: [0.0; W],
            db: [0.0; W],
            lines: Vec::new(),
            pd: [Line::new(PD_LEN), Line::new(PD_LEN)],
            ch: [ch.clone(), ch],
        });
        // The lines are sized for the largest Size; line 0's length never changes.
        g.set_lengths(4.0);
        g.lines = g.len_i.iter().map(|&l| Line::new(l)).collect();
        for ch in &mut g.ch {
            ch.set_shelf(0.0);
        }
        g.set_lengths(1.0);
        for k in 0..N {
            g.dlen[k] = g.len_i[k] as f32;
        }
        g.set_sample_rate(44100.0);
        g
    }

    /// The prime-table line lengths for size factor `f` (`setParameter(1)`).
    fn set_lengths(&mut self, f: f32) {
        let p = primes();
        if self.hall {
            let s8 = (f * 23.0) * 32.0;
            for k in 0..N {
                let e = (self.perm[k] as f32 * 0.03125).powf(c(0x3fce_147b));
                let idx = (s8.mul_add(e, 90.0) + OFFSET[k] as f32) as i32;
                self.len_i[k] = p[idx as usize];
                self.b.tgt[k] = self.len_i[k] as f32;
            }
        } else {
            let s0 = f * 10.0;
            for k in 0..N {
                let idx = (s0.mul_add(self.perm[k] as f32, 90.0) + OFFSET[k] as f32) as i32;
                self.len_i[k] = p[idx as usize];
                self.b.tgt[k] = self.len_i[k] as f32;
            }
        }
        self.a.tgt[6] = (self.len_i[1] - self.len_i[0]) as f32;
    }

    /// `setParameter(idx, v)`.
    pub(super) fn set_parameter(&mut self, idx: u32, v: f32) {
        if idx >= 10 {
            return;
        }
        self.raw[idx as usize] = v;
        match idx {
            0 => {
                let t = (v * c(0x406c_cccd)).exp() * 0.5;
                self.a.tgt[1] = t;
                self.a.tgt[0] = (1.0 - exp10f(-4.0 / t)).sqrt() * 0.5;
            }
            1 => self.set_lengths(v.mul_add(3.0, 1.0)),
            2 => self.a.tgt[2] = v.powf(0.25),
            3 => self.a.tgt[3] = v * 10.0,
            4 => self.a.tgt[4] = v * 0.5,
            5 => self.a.tgt[5] = (v * 0.25) * 44100.0,
            6 => self.a.tgt[7] = (1.0 - v).mul_add(19000.0, 2000.0),
            7 => self.a.tgt[8] = v * -12.0,
            8 => self.a.tgt[9] = v,
            _ => self.a.tgt[10] = v,
        }
        if idx == 1 {
            self.b.dirty = true;
            self.b.state = 1;
        }
        self.a.dirty = true;
        self.a.state = 1;
        if self.immediate {
            self.final_values();
        }
    }

    /// `setVarsToFinalSmoothingVals()`: jump to the targets and derive the coefficients.
    fn final_values(&mut self) {
        self.a.set_final();
        self.b.set_final();
        self.set_high_cut();
        self.set_shelf();
        self.set_diffusion();
        self.set_t60();
    }

    /// The one-pole high cut's coefficient (`exp(−2π·f/44100)`, in f64).
    fn set_high_cut(&mut self) {
        let hc = (((self.a.cur[7] * -2.0) as f64 * std::f64::consts::PI) / 44100.0).exp() as f32;
        for ch in &mut self.ch {
            ch.hc = hc;
        }
    }

    fn set_shelf(&mut self) {
        let shelf = self.a.cur[8];
        for ch in &mut self.ch {
            ch.set_shelf(shelf);
        }
    }

    /// A two-band T60 (`+0x6ce0`): the low band's gain per sample and the ratio of the
    /// high band's, then the lines' damping.
    fn set_t60(&mut self) {
        let t = self.a.cur[1];
        let damp = self.a.cur[2];
        let l = (t + t).ln() * c(0x3e8a_60dd);
        let s1 = damp.mul_add(c(0xbf73_3333), 1.0);
        let s3 = damp.mul_add(-0.5, 1.0);
        let t_hi = t * l.mul_add(s1, s3 * (1.0 - l));
        self.g_lo = exp10f(-3.0 / (t * 44100.0));
        self.ratio = exp10f(-3.0 / (t_hi * 44100.0)) / self.g_lo;
        self.set_damping();
    }

    /// Per line: a gain for the low band and a one-pole that gives the high band's decay.
    fn set_damping(&mut self) {
        for i in 0..N {
            self.da[i] = self.g_lo.powf(self.b.cur[i]);
            let r = self.ratio.powf(self.b.cur[i]);
            let r = 1.0 / (r * r);
            self.db[i] = if r == 1.0 { 0.0 } else { ((c(0xbef6_7751) - r) + r.mul_add(c(0x403d_9dd4), c(0xbf44_ad9e)).sqrt()) / (1.0 - r) };
        }
        for i in 0..W {
            self.da[i] *= 1.0 - self.db[i];
        }
    }

    /// Diffusion: `setDiffuserCoeffs`, and for Hall the output matrix blend.
    fn set_diffusion(&mut self) {
        let d = self.a.cur[4];
        self.set_diffuser_coeffs(d);
        if self.hall {
            let k1 = d + d;
            let k0 = 1.0 - k1;
            for r in 0..2 {
                for j in 0..W {
                    self.out[r][j] = k1 * self.out_b[r][j] + k0 * self.out_a[r][j];
                }
            }
        }
    }

    /// `setDiffuserCoeffs(d)`: the nested allpasses' matrices (alternating signs).
    fn set_diffuser_coeffs(&mut self, d: f32) {
        let f2 = d * c(0xbf66_6666);
        let f6 = f2.mul_add(d, 1.0);
        let f7 = f2 * d;
        let f8 = c(0x3f66_6666) * d;
        let f5 = (-d).mul_add(f8, 1.0);
        let f9 = -d * f8;
        for ch in &mut self.ch {
            for (i, st) in ch.stages.iter_mut().enumerate() {
                if i % 2 == 0 {
                    st.a = [[f9, f9, f2], [f5, f9, f2], [0.0, f5, f2]];
                    st.b = [d, d, 1.0];
                    st.c_z = f5;
                    st.c_x = d;
                } else {
                    st.a = [[f7, f7, f8], [f6, f7, f8], [0.0, f6, f8]];
                    st.b = [-d, -d, 1.0];
                    st.c_z = f6;
                    st.c_x = -d;
                }
            }
        }
    }

    /// `onSetSampleRate(sr, 128)` (rates up to 48 kHz: no resampling).
    pub(super) fn set_sample_rate(&mut self, sr: f32) {
        // Above 48 kHz Maschine resamples to `sr/k`, k = ⌊sr/44100⌋ (§3.7).
        let k = ((sr / 44100.0) as i32).max(1);
        self.k = if sr > 48000.0 { k as usize } else { 1 };
        let block = 128 / self.k as i32;
        let sr = if sr > 48000.0 { sr / k as f32 } else { sr };
        let (s, cc) = sincos(12.566370614359172 / sr as f64);
        self.rot_c = cc;
        self.rot_s = s;
        self.re = cc.mul_add(0.0, s * -0.0) + 1.0;
        self.im = s.mul_add(0.0, cc * 0.0);
        // Ramps of 50 ms (most values) and 200 ms (line lengths) in 128-sample ticks.
        for (k, ramp_steps, inv) in
            [(c(0x3d4c_cccd), &mut self.a.steps, &mut self.a.inv), (c(0x3e4c_cccd), &mut self.b.steps, &mut self.b.inv)]
        {
            let n = ((sr * k) / block as f32 + 0.5) as i32;
            let n = if n > 1 { n } else { 1 };
            *ramp_steps = n;
            *inv = 1.0 / (n * block) as f32;
        }
        let raw = self.raw;
        for (i, v) in raw.iter().enumerate() {
            self.set_parameter(i as u32, *v);
        }
        self.final_values();
    }

    /// `onReset()`: clear the signal state (not the LFO, the EQ or the high cut).
    pub(super) fn reset(&mut self) {
        self.y = [0.0; W];
        self.x = [0.0; W];
        self.s = [0.0; W];
        self.acc = (0.0, 0.0, 0);
        self.held = [(0.0, 0.0); 2];
        for l in self.lines.iter_mut().chain(self.pd.iter_mut()) {
            l.buf.fill(0.0);
        }
        for ch in &mut self.ch {
            for st in &mut ch.stages {
                st.clear();
            }
        }
    }

    /// `PlateReverbProcessor::setMode` clears the chosen engine like `onReset`.
    pub(super) fn clear_for_mode(&mut self) {
        self.reset();
    }

    /// `onResume()`.
    pub(super) fn resume(&mut self) {
        self.final_values();
        self.immediate = false;
    }

    /// `processControlRate()`: start, run or finish the parameter ramps.
    pub(super) fn control(&mut self) {
        self.a.control();
        self.b.control();
    }

    pub(super) fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        // `processAudio<ModeSelector>`: the ramping variant while a ramp runs.
        let ramping = self.a.ramping || self.b.ramping;
        if self.k == 1 {
            for (i, (a, b)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
                if ramping {
                    self.ramp(i);
                }
                (*a, *b) = self.tick(*a, *b);
            }
            return;
        }
        // Approximate resampling: average k input frames, run one network sample, and
        // interpolate linearly between the last two outputs (one frame group late).
        let k = self.k;
        let mut j = 0;
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            self.acc.0 += *a;
            self.acc.1 += *b;
            self.acc.2 += 1;
            let t = self.acc.2 as f32 / k as f32;
            let [p, q] = self.held;
            (*a, *b) = (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
            if self.acc.2 == k {
                if ramping {
                    self.ramp(j);
                }
                j += 1;
                let y = self.tick(self.acc.0 / k as f32, self.acc.1 / k as f32);
                self.held = [q, y];
                self.acc = (0.0, 0.0, 0);
            }
        }
    }

    /// One sample of the ramps, and the coefficients that follow them: high cut and shelf
    /// every sample, diffusion every 8th, the T60 (or, with only the lengths moving, the
    /// damping) every 32nd sample of the block.
    fn ramp(&mut self, i: usize) {
        self.a.step();
        self.b.step();
        if self.a.delta[7] != 0.0 {
            self.set_high_cut();
        }
        if self.a.delta[8] != 0.0 {
            self.set_shelf();
        }
        if i & 7 != 0 {
            return;
        }
        if self.a.delta[4] != 0.0 {
            self.set_diffusion();
        }
        if i & 31 != 0 {
            return;
        }
        if self.a.delta[1] != 0.0 || self.a.delta[2] != 0.0 {
            self.set_t60();
        } else if self.b.ramping {
            self.set_damping();
        }
    }

    fn tick(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        // LFO: rotate the phasor.
        let re = self.rot_c.mul_add(self.re, -(self.rot_s * self.im)) + 0.0;
        let im = self.rot_s.mul_add(self.re, self.rot_c * self.im);
        self.re = re;
        self.im = im;
        let depth = self.a.cur[3];
        let mod_im = depth * im as f32;
        let mod_re = depth * re as f32;
        // Pre-delays: the left one is longer by the first two lines' difference.
        let pd = self.a.cur[5];
        let extra = self.a.cur[6];
        let [pl, pr] = &mut self.pd;
        pl.buf[(pl.w & pl.mask) as usize] = in_l;
        let pdl = pl.read_frac(pd + extra);
        pl.w = pl.w.wrapping_add(1) & pl.mask;
        pr.buf[(pr.w & pr.mask) as usize] = in_r;
        let pdr = pr.read_frac(pd);
        pr.w = pr.w.wrapping_add(1) & pr.mask;
        // Feedback matrix.
        for r in 0..W {
            let mut acc = 0.0f32;
            for j in 0..W {
                acc = self.m[r + W * j].mul_add(self.x[j], acc);
            }
            self.y[r] = acc + 0.0;
        }
        let mut inj = [0.0f32; W];
        for i in 0..W {
            inj[i] = self.in_r[i].mul_add(pdr, self.in_l[i] * pdl);
        }
        for k in 1..N {
            self.dlen[k] = if k % 2 == 1 { mod_re } else { mod_im } + self.b.cur[k];
        }
        let l0 = self.len_i[0] as u32;
        for k in 0..N {
            let line = &mut self.lines[k];
            let w = line.w;
            line.buf[(w & line.mask) as usize] = self.y[k];
            let at = (w.wrapping_sub(l0).wrapping_add(50) & line.mask) as usize;
            line.buf[at] += inj[k];
            self.x[k] = line.read_frac(self.dlen[k] + -1.0);
            line.w = w.wrapping_add(1) & line.mask;
        }
        // Damping.
        for i in 0..W {
            let v = self.da[i] * self.x[i] + self.db[i] * self.s[i];
            self.x[i] = v;
            self.s[i] = v;
        }
        let mut o = [0.0f32; 2];
        for (ch, out) in o.iter_mut().enumerate() {
            let mut acc = 0.0f32;
            for j in 0..W {
                acc = self.out[ch][j].mul_add(self.x[j], acc);
            }
            *out = self.ch[ch].tick(acc + 0.0);
        }
        let (s, cc) = sincosf((self.a.cur[9] * c(0x4049_0fdb)) * 0.5);
        let g = self.a.cur[0] * s;
        let width = self.a.cur[10];
        let mid = (o[0] + o[1]) * 0.5;
        let yl = cc.mul_add(in_l, g * (o[0] - mid).mul_add(width, mid));
        let yr = cc.mul_add(in_r, g * (o[1] - mid).mul_add(width, mid));
        (yl, yr)
    }
}
