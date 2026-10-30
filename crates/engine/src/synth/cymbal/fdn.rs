//! `NI::DSP::UTIL::FdbckDelayNetwork`: the feedback delay network both cymbal engines run
//! the body through (docs/synth-cymbal.md §5). It sits at CrashCymbalCore +0x42800 and
//! RideCymbalCore +0x42880; offsets below are relative to it.
//!
//! Fifteen delay lines (a sixteenth lane exists but stays silent) whose lengths are
//! prime numbers of 44.1 kHz samples, scaled to the sample rate; per-line gains
//! `decay^prime` give every line the same T60. The feedback matrix is a 15×15
//! circulant of two values, applied with Eigen's row-major GEMV (four-lane FMA
//! accumulators, reduced `(l0+l2)+(l1+l3)`). Only the left input feeds the network;
//! the left output reads lines 0..7 with alternating signs, the right lines 8..14.

#![allow(dead_code)]

use crate::fx::exp_e;

/// Samples per delay line (indices wrap at `& 0xfff`).
const LINE: usize = 4096;
/// Delay lines in use.
const LINES: usize = 15;
/// The circulant's two values (`0x3e43afb8`, `0xbe92ca58`).
const A: f32 = f32::from_bits(0x3e43_afb8);
const B: f32 = f32::from_bits(0xbe92_ca58);

/// The first 450 primes: `s_aPrimeNumbers` (computed; the table holds exactly these).
fn primes() -> &'static [u32; 450] {
    static P: std::sync::OnceLock<[u32; 450]> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let mut p = [0u32; 450];
        let (mut n, mut k) = (2u32, 0);
        while k < 450 {
            if (2..n).take_while(|d| d * d <= n).all(|d| n % d != 0) {
                p[k] = n;
                k += 1;
            }
            n += 1;
        }
        p
    })
}

pub(super) struct Fdn {
    /// +0x00: the circulant's first column (15 values).
    pub(super) circ: [f32; 15],
    /// +0x40: ±1 signs (`1, −1, …`, lane 15 = 0) the output rows are built from.
    pub(super) signs: [f32; 16],
    /// +0x80: per-line gains `decay^prime` (lane 15 stays 0).
    pub(super) gains: [f32; 16],
    /// +0xc0: feedback matrix, row `r` = the GEMV row of output `r`
    /// (`m[r][c] = circ[(r − c) mod 15]`, row/column 15 zero).
    pub(super) matrix: [[f32; 16]; 16],
    /// +0x4c0: feedback vector (the lines' next inputs, before the input is added).
    pub(super) fb: [f32; 16],
    /// +0x500: left output row (lines 0..7, alternating signs).
    pub(super) out_l: [f32; 16],
    /// +0x540: right output row (lines 8..14, alternating signs).
    pub(super) out_r: [f32; 16],
    /// +0x580: sample rate (as the line-length scaler reads it).
    pub(super) sr_lines: f32,
    /// +0x590: the delay lines, line `i` at `i·4096` (inline in NI's object).
    pub(super) buf: Box<[f32]>,
    /// +0x40590: write index.
    pub(super) wpos: u32,
    /// +0x405a0: read index per line.
    pub(super) rpos: [u32; 16],
    /// +0x405e0: the lines' outputs this sample.
    pub(super) taps: [f32; 16],
    /// +0x40620: each line's length in 44.1 kHz samples (a prime).
    pub(super) prime: [u32; 15],
    /// +0x4065c: T60 parameter (0.1..2).
    pub(super) t60: f32,
    /// +0x40660: delay-length parameter (1..30; the prime-index step).
    pub(super) length: f32,
    /// +0x40664: unused (0).
    pub(super) unused: f32,
    /// +0x40668: wet mix (`out = in + mix·(out_gain·wet − in)`).
    pub(super) mix: f32,
    /// +0x4066c: T60 offset, subtracted from `t60²` (the cores' velocity modulation).
    pub(super) t60_offset: f32,
    /// +0x40670: sample rate.
    pub(super) sr: f32,
    /// +0x40674: `1/sr`.
    pub(super) inv_sr: f32,
    /// +0x40678: `sr/44100`.
    pub(super) ratio: f32,
    /// +0x4067c: input gain (`1/√15`).
    pub(super) in_gain: f32,
    /// +0x40680: per-sample decay factor (`E(−86.5617·ratio/(sr·T) + 1200)`).
    pub(super) decay: f32,
    /// +0x40684: output gain.
    pub(super) out_gain: f32,
}

impl Fdn {
    /// `FdbckDelayNetwork()` (0x1ced604).
    pub(super) fn new() -> Fdn {
        let circ = [A, A, B, B, A, B, A, B, B, B, B, A, A, A, B];
        let mut signs = [0f32; 16];
        for (i, s) in signs.iter_mut().take(LINES).enumerate() {
            *s = if i % 2 == 0 { 1.0 } else { -1.0 };
        }
        let mut matrix = [[0f32; 16]; 16];
        for (r, row) in matrix.iter_mut().take(LINES).enumerate() {
            for (c, m) in row.iter_mut().take(LINES).enumerate() {
                *m = circ[(r + LINES - c) % LINES];
            }
        }
        let mut out_l = [0f32; 16];
        out_l[..8].copy_from_slice(&signs[..8]);
        let mut out_r = [0f32; 16];
        out_r[8..].copy_from_slice(&signs[8..]);
        Fdn {
            circ,
            signs,
            gains: [0.0; 16],
            matrix,
            fb: [0.0; 16],
            out_l,
            out_r,
            sr_lines: 1.0,
            buf: vec![0.0; 16 * LINE].into_boxed_slice(),
            wpos: 0,
            rpos: [0; 16],
            taps: [0.0; 16],
            prime: [0; 15],
            t60: 0.1,
            length: 1.0,
            unused: 0.0,
            mix: 0.0,
            t60_offset: 0.0,
            sr: 44100.0,
            inv_sr: f32::from_bits(0x37be_37c6),
            ratio: 1.0,
            in_gain: f32::from_bits(0x3e84_32a5),
            decay: 0.1,
            out_gain: 0.0,
        }
    }

    /// Silence the lines, their outputs and the feedback, as `setSampleRate` and the
    /// cores' `onReset` do (indices, gains and parameters stay).
    pub(super) fn clear(&mut self) {
        self.buf.fill(0.0);
        self.taps = [0.0; 16];
        self.fb = [0.0; 16];
    }

    /// `setSampleRate(sr)` (0x1cedba8), 1 ≤ sr ≤ 500000.
    pub(super) fn set_sample_rate(&mut self, sr: f32) {
        if !(1.0..=500_000.0).contains(&sr) {
            return;
        }
        self.sr = sr;
        self.inv_sr = 1.0 / sr;
        self.ratio = sr / 44100.0;
        self.sr_lines = sr;
        self.clear();
        self.set_t60(self.t60);
        self.clear();
    }

    /// `setT60(t)` (0x1cedce4), 0.1 ≤ t ≤ 2: the decay factor, then
    /// [`Fdn::set_delay_length`] with the current length.
    pub(super) fn set_t60(&mut self, t: f32) {
        if !(0.1..=2.0).contains(&t) {
            return;
        }
        self.t60 = t;
        let f = t.mul_add(t, -self.t60_offset);
        let f = if f < 0.01 { 0.01 } else { f };
        let f = if f > 4.0 { 4.0 } else { f };
        let x = -1.0 / ((self.sr * f) / self.ratio);
        self.decay = exp_e(x.mul_add(f32::from_bits(0x42ad_1f97), 1200.0));
        self.set_delay_length(self.length);
    }

    /// `setDelayLength(l)` (0x1cede54), 1 ≤ l ≤ 30: line `i` takes the
    /// `round(l·(i+1) − 1)`-th prime as its length in 44.1 kHz samples and the gain
    /// `decay^prime` (`exp2f(log2f(decay)·prime)`).
    pub(super) fn set_delay_length(&mut self, l: f32) {
        if !(1.0..=30.0).contains(&l) {
            return;
        }
        self.length = l;
        let lg = self.decay.log2();
        let half = |x: f32| if x < 0.0 { -0.5 } else { 0.5 };
        for i in 0..LINES {
            let s = self.length.mul_add((i + 1) as f32, -1.0);
            let idx = (half(s) + f64::from(s)) as i32;
            let p = primes()[idx as usize];
            self.prime[i] = p;
            let pf = p as f32;
            let d = ((self.inv_sr * pf) * self.ratio) * self.sr_lines;
            let d = (half(d) + f64::from(d)) as i32 as u32;
            if d < LINE as u32 {
                self.rpos[i] = self.wpos.wrapping_sub(d) & 0xfff;
            }
            self.gains[i] = (lg * pf).exp2();
        }
    }

    /// `processSample(io)` (0x1cee294): `io[0]` feeds the lines; both channels are mixed
    /// with the network's output in place.
    pub(super) fn process_sample(&mut self, io: &mut [f32; 2]) {
        let (inl, inr) = (io[0], io[1]);
        let inj = inl * self.in_gain;
        let w = self.wpos as usize;
        for i in 0..16 {
            self.fb[i] += inj;
            self.buf[i * LINE + w] = self.fb[i];
        }
        for i in 0..LINES {
            self.taps[i] = self.buf[i * LINE + self.rpos[i] as usize];
        }
        self.wpos = (self.wpos + 1) & 0xfff;
        for r in self.rpos.iter_mut() {
            *r = r.wrapping_add(1) & 0xfff;
        }
        let x: [f32; 16] = std::array::from_fn(|i| self.taps[i] * self.gains[i]);
        let sum = |row: &[f32; 16]| {
            let l: [f32; 4] =
                std::array::from_fn(|k| (x[k] * row[k] + x[4 + k] * row[4 + k]) + (x[8 + k] * row[8 + k] + x[12 + k] * row[12 + k]));
            (l[0] + l[2]) + (l[1] + l[3])
        };
        let (sl, sr) = (sum(&self.out_l), sum(&self.out_r));
        io[0] = (self.out_gain * sl - inl).mul_add(self.mix, inl);
        io[1] = (self.out_gain * sr - inr).mul_add(self.mix, inr);
        // fb = M·x: Eigen's row-major GEMV, four FMA lanes per row, `res += 1·acc`.
        for (r, row) in self.matrix.iter().enumerate() {
            let mut l = [0f32; 4];
            for j in (0..16).step_by(4) {
                for (k, lk) in l.iter_mut().enumerate() {
                    *lk = x[j + k].mul_add(row[j + k], *lk);
                }
            }
            let acc = (l[0] + l[2]) + (l[1] + l[3]);
            self.fb[r] = 1.0f32.mul_add(acc, 0.0);
        }
    }

    /// The network's state as NI lays it out, at `img[base..]` (the lines included).
    #[cfg(test)]
    pub(super) fn image(&self, img: &mut [u8], base: usize) {
        let mut f = |o: usize, b: [u8; 4]| img[base + o..base + o + 4].copy_from_slice(&b);
        for i in 0..15 {
            f(4 * i, self.circ[i].to_le_bytes());
            f(0x40620 + 4 * i, self.prime[i].to_le_bytes());
        }
        for i in 0..16 {
            f(0x40 + 4 * i, self.signs[i].to_le_bytes());
            f(0x80 + 4 * i, self.gains[i].to_le_bytes());
            f(0x4c0 + 4 * i, self.fb[i].to_le_bytes());
            f(0x500 + 4 * i, self.out_l[i].to_le_bytes());
            f(0x540 + 4 * i, self.out_r[i].to_le_bytes());
            f(0x405a0 + 4 * i, self.rpos[i].to_le_bytes());
            f(0x405e0 + 4 * i, self.taps[i].to_le_bytes());
            for c in 0..16 {
                f(0xc0 + 64 * i + 4 * c, self.matrix[i][c].to_le_bytes());
            }
        }
        f(0x580, self.sr_lines.to_le_bytes());
        for (i, v) in self.buf.iter().enumerate() {
            f(0x590 + 4 * i, v.to_le_bytes());
        }
        f(0x40590, self.wpos.to_le_bytes());
        for (o, v) in [
            (0x4065c, self.t60),
            (0x40660, self.length),
            (0x40664, self.unused),
            (0x40668, self.mix),
            (0x4066c, self.t60_offset),
            (0x40670, self.sr),
            (0x40674, self.inv_sr),
            (0x40678, self.ratio),
            (0x4067c, self.in_gain),
            (0x40680, self.decay),
            (0x40684, self.out_gain),
        ] {
            f(o, v.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primes_and_lengths() {
        assert_eq!(&primes()[..6], &[2, 3, 5, 7, 11, 13]);
        assert_eq!(primes()[449], 3181);
        let mut f = Fdn::new();
        f.set_sample_rate(48000.0);
        f.set_t60(1.0);
        f.set_delay_length(18.0);
        assert!(f.gains[..15].iter().all(|g| *g > 0.0 && *g < 1.0));
        assert!(f.prime.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn decays_to_silence() {
        let mut f = Fdn::new();
        f.set_sample_rate(44100.0);
        f.set_t60(0.5);
        f.set_delay_length(10.0);
        f.mix = 1.0;
        f.out_gain = 1.0;
        let mut io = [1.0, 0.0];
        f.process_sample(&mut io);
        let mut peak = 0f32;
        for n in 0..200_000 {
            let mut io = [0.0, 0.0];
            f.process_sample(&mut io);
            assert!(io[0].is_finite() && io[1].is_finite());
            if n > 150_000 {
                peak = peak.max(io[0].abs()).max(io[1].abs());
            }
        }
        assert!(peak < 1e-6, "{peak}");
    }

    /// Bit-exact against NI. `CYMBAL_ORACLE_FDN` names a directory with `cases.txt`
    /// (`name cmd…` per line) and `<name>.bin` written by scratchpad/syn-cymbal/fdn/fdn
    /// running the same commands.
    #[test]
    #[ignore]
    fn oracle() {
        let Ok(dir) = std::env::var("CYMBAL_ORACLE_FDN") else { return };
        let cases = std::fs::read_to_string(format!("{dir}/cases.txt")).unwrap();
        for line in cases.lines().filter(|l| !l.trim().is_empty()) {
            let mut w = line.split_whitespace();
            let name = w.next().unwrap();
            let bin = std::fs::read(format!("{dir}/{name}.bin")).unwrap();
            let mut pos = 0;
            let mut f = Fdn::new();
            let mut seed = 1u32;
            for cmd in w {
                let (k, v) = cmd.split_once(':').unwrap_or((cmd, ""));
                match k {
                    "sr" => f.set_sample_rate(v.parse().unwrap()),
                    "t60" => f.set_t60(v.parse().unwrap()),
                    "len" => f.set_delay_length(v.parse().unwrap()),
                    "poke" => {
                        let (o, x) = v.split_once(':').unwrap();
                        let x: f32 = x.parse().unwrap();
                        match u32::from_str_radix(o, 16).unwrap() {
                            0x40668 => f.mix = x,
                            0x40684 => f.out_gain = x,
                            0x4066c => f.t60_offset = x,
                            0x4067c => f.in_gain = x,
                            o => panic!("poke {o:x}"),
                        }
                    }
                    "run" => {
                        let n: usize = v.parse().unwrap();
                        for k in 0..n {
                            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                            let a = (seed as i32) as f32 * (1.0 / 2_147_483_648.0);
                            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                            let b = (seed as i32) as f32 * (1.0 / 2_147_483_648.0);
                            let mut io = if k > 2000 { [0.0, 0.0] } else { [a, b] };
                            f.process_sample(&mut io);
                            for (c, v) in io.iter().enumerate() {
                                let want = f32::from_le_bytes(bin[pos..pos + 4].try_into().unwrap());
                                assert_eq!(v.to_bits(), want.to_bits(), "{name}: sample {k} ch {c}: {v} vs {want}");
                                pos += 4;
                            }
                        }
                    }
                    "dump" => {
                        let want = &bin[pos..pos + 0x40688];
                        let mut img = want.to_vec();
                        f.image(&mut img, 0);
                        // NI leaves padding uninitialised: 0x3c, 0x584..0x590, 0x40594..0x405a0.
                        for (o, (a, b)) in img.iter().zip(want).enumerate() {
                            let pad = (0x3c..0x40).contains(&o) || (0x584..0x590).contains(&o) || (0x40594..0x405a0).contains(&o);
                            assert!(pad || a == b, "{name}: state differs at +{o:#x}");
                        }
                        pos += 0x40700;
                    }
                    _ => panic!("{cmd}"),
                }
            }
        }
    }
}
