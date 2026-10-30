//! `NI::AB::SampleRateConverter` at quality 2, one channel (docs/synth-tom.md §5.2), as
//! `UTIL::PinkSampler::setSampleRate` uses it to bring Air's 44.1 kHz pink noise to the
//! host rate. Ported from the arm64 code operation by operation (`f32`, `f64` where it
//! uses doubles, `mul_add` for its fused multiply-adds): the output is bit-exact.
//!
//! A rate ratio that is not an integer (48, 96, 32, 192 kHz, …) takes the "generic"
//! path: an 8-tap (more when downsampling) windowed interpolator whose taps are degree-5
//! polynomials of the fractional position, evaluated per output frame. Integer ratios
//! (×2, ×N, ÷2, ÷N) take polyphase paths ("intrat") with a 16-tap Kaiser-windowed sinc.

#![allow(dead_code)]

use super::samples;

/// Taps of the interpolation kernel (`InterpolCoefficients`, quality 2: 8) and the
/// degree of its pieces (5).
const N: u32 = 8;
const M: u32 = 5;

/// The kernel's four pieces (quality 2), coefficients of x⁰…x⁵ (`InterpolCoefficients`
/// constructor) **[exact]**.
const PIECES: [[u32; 6]; 4] = [
    [0x3f80_0000, 0x0000_0000, 0xbfef_bf40, 0xbdab_07b6, 0x3fc9_948e, 0xbf1e_49a5],
    [0x3ee7_235d, 0x4046_ad79, 0xc10d_895e, 0x40f7_7d97, 0xc034_3706, 0x3ebe_7b5c],
    [0x4110_2672, 0xc1ad_d1f9, 0x419d_38bb, 0xc107_b3d9, 0x3fe2_31fe, 0xbe12_d342],
    [0xc0bd_1d3c, 0x4125_984e, 0xc0dc_3032, 0x400d_18b9, 0xbeaf_eafa, 0x3cab_bd9a],
];

fn piece(row: usize, j: u32) -> f32 {
    f32::from_bits(PIECES[row][j as usize])
}

/// `binom(j, m)` as the converter computes it: `j·(j−1)…(m+1) / (j−m)!` in `u32`.
fn binom(j: u32, m: u32) -> u32 {
    if j == m {
        return 1;
    }
    let (mut num, mut den) = (1u32, 1u32);
    for k in 0..j - m {
        num = num.wrapping_mul(j - k);
        den = den.wrapping_mul(k + 1);
    }
    num.checked_div(den).unwrap_or(0)
}

/// The generic converter's state for one channel.
struct Generic {
    /// Output frames per input frame bookkeeping (`init`): whole input frames per output
    /// frame (+0x18), the remainder (+0x1c) and the denominator (+0x20).
    cnt: u32,
    rem: f32,
    den: f32,
    /// 1/dst (+0xc4).
    inv: f32,
    /// Taps (+0xbc) and groups of four (+0xb0).
    taps: usize,
    groups: usize,
    /// Per group: thresholds, then (A[m], B[m]) for m = 5…0, four lanes each (+0xc8).
    table: Vec<[f32; 4]>,
    /// The input history (`taps` frames, padded to whole groups).
    hist: Vec<f32>,
    /// Input frames still needed before the next output (+0xb4), and the phase (+0xb8).
    need: u32,
    frac: f32,
}

impl Generic {
    /// `init(src, dst, 1, quality 2)` taking the generic path, then `resetAllChannels`.
    fn new(src: f32, dst: f32) -> Generic {
        let ratio = dst / src;
        let (mut cnt, mut rem) = (0u32, src);
        if dst <= src {
            let mut f = src;
            loop {
                cnt += 1;
                f -= dst;
                if dst > f {
                    break;
                }
            }
            rem = f;
        }
        let inv = 1.0 / dst;
        // Taps: 8, or ⌈8·src/dst⌉ when downsampling (`fcvtpu`).
        let taps = if ratio > 1.0 { N } else { ((N as f64 * src as f64) / dst as f64).ceil() as u32 };
        let groups = taps.div_ceil(4) as usize;
        let len = groups * 4;
        // Step through the kernel (d18) and its width (d20); gain g.
        let (step, width, g) = if ratio > 1.0 { (2.0f64, 2.0f64, 1.0f32) } else { (2.0 * dst as f64, 2.0 * src as f64, dst / src) };
        let off = (width * N as f64) * -0.5;
        let mut thr = vec![0f32; len];
        let mut a = vec![vec![0f32; len]; M as usize + 1];
        let mut b = vec![vec![0f32; len]; M as usize + 1];
        let half = N >> 1;
        for i in 0..taps {
            let t = (i as f64).mul_add(step, off);
            let at = if t >= 0.0 { t } else { -t };
            let q = at / width;
            let mut k = q as u32;
            let mut r = (-(k as f64)).mul_add(width, at);
            // A whole-number position on the negative side counts one piece lower.
            let w0 = r == 0.0 && t < 0.0;
            let r16 = if r != 0.0 { r } else { width };
            r = if t >= 0.0 { width - r } else { r16 };
            k = k.wrapping_sub(w0 as u32);
            let nxt_off: i32 = if t < 0.0 { if k != 0 { -1 } else { 0 } } else { 1 };
            let k2 = k.wrapping_add(nxt_off as u32);
            let idx = (taps - 1 - i) as usize;
            let s28 = if t >= 0.0 { g } else { -g };
            let s5 = if step + t > 0.0 { if t >= 0.0 { 1.0f32 } else { -1.0 } } else { 1.0 };
            let s29 = q as f32;
            thr[idx] = (r / step) as f32;
            let s30 = s5 * s29;
            let s31 = s28 * s5;
            for m in 0..=M {
                let mut s9 = 0f32;
                if half > k {
                    for j in (m..=M).rev() {
                        s9 = piece(k as usize, j).mul_add(binom(j, m) as f32, s9 * s29);
                    }
                    for _ in 0..m {
                        s9 *= s28;
                    }
                }
                a[m as usize][idx] = g * s9;
                let mut s8 = 0f32;
                if half > k2 {
                    for j in (m..=M).rev() {
                        s8 = piece(k2 as usize, j).mul_add(binom(j, m) as f32, s30 * s8);
                    }
                    for _ in 0..m {
                        s8 *= s31;
                    }
                }
                b[m as usize][idx] = g * s8;
            }
        }
        let mut table = Vec::with_capacity(groups * (2 * M as usize + 3));
        let lanes = |v: &[f32], gi: usize| [v[4 * gi], v[4 * gi + 1], v[4 * gi + 2], v[4 * gi + 3]];
        for gi in 0..groups {
            table.push(lanes(&thr, gi));
            for m in (0..=M as usize).rev() {
                table.push(lanes(&a[m], gi));
                table.push(lanes(&b[m], gi));
            }
        }
        Generic { cnt, rem, den: dst, inv, taps: taps as usize, groups, table, hist: vec![0.0; len], need: 1, frac: 0.0 }
    }

    /// Shift `k` input frames into the history.
    fn feed(&mut self, input: &[f32], pos: &mut usize, k: usize) {
        let l = self.taps;
        let k = k.min(l).min(input.len() - *pos);
        self.hist.copy_within(k..l, 0);
        self.hist[l - k..l].copy_from_slice(&input[*pos..*pos + k]);
        *pos += k;
    }

    /// `convert` (`process_generic`): `out.len()` frames from `input`.
    fn convert(&mut self, input: &[f32], out: &mut [f32]) {
        let n_in = input.len() as u32;
        let mut pos = 0;
        let k = self.need.min(n_in);
        self.feed(input, &mut pos, k as usize);
        self.need -= k;
        let mut used = k;
        let mut w = vec![[0f32; 4]; self.groups];
        let per = 2 * M as usize + 3;
        for o in out.iter_mut() {
            let x = self.frac * self.inv;
            for (gi, wg) in w.iter_mut().enumerate() {
                let t = &self.table[gi * per..(gi + 1) * per];
                let mask: [bool; 4] = std::array::from_fn(|l| x >= t[0][l]);
                let mut acc: [f32; 4] = std::array::from_fn(|l| if mask[l] { t[2][l] } else { t[1][l] });
                for m in 0..M as usize {
                    let (lo, hi) = (t[3 + 2 * m], t[4 + 2 * m]);
                    for l in 0..4 {
                        acc[l] = acc[l] * x + if mask[l] { hi[l] } else { lo[l] };
                    }
                }
                *wg = acc;
            }
            let mut v: [f32; 4] = std::array::from_fn(|l| self.hist[l] * w[0][l]);
            for (gi, wg) in w.iter().enumerate().skip(1) {
                for l in 0..4 {
                    v[l] += self.hist[4 * gi + l] * wg[l];
                }
            }
            *o = (v[0] + v[2]) + (v[1] + v[3]);
            let mut adv = self.cnt;
            self.frac += self.rem;
            if self.frac > self.den {
                self.frac -= self.den;
                adv += 1;
            }
            if adv != 0 {
                let total = adv.wrapping_add(used);
                if total > n_in {
                    let excess = total - n_in;
                    self.need = excess;
                    let k = adv.wrapping_sub(excess);
                    self.feed(input, &mut pos, k as usize);
                    used = k.wrapping_add(used).wrapping_add(self.need);
                } else {
                    self.feed(input, &mut pos, adv as usize);
                    used = total;
                }
            }
        }
    }
}

/// The pink noise at rate `sr` as `PinkSampler::setSampleRate` keeps it: the frames
/// (resampled unless `sr` is 44.1 kHz) and the playback ratio it stores (+0x24,
/// `sr / 44100`, 1 at 44.1 kHz).
pub(super) fn pink_at(sr: f32) -> (Box<[f32]>, f32) {
    let src = samples::pink();
    if sr == samples::RATE {
        return (src.into(), 1.0);
    }
    let (out, _) = convert(src, samples::RATE, sr);
    (out, sr / samples::RATE)
}

/// `SampleRateConverter::init(src, dst, 1, 2)` + `convert(input, n)` with the output
/// length `PinkSampler` asks for (`⌈((len+1−need)·den − phase) / (cnt·den + rem)⌉`).
/// Returns the frames and the length.
fn convert(input: &[f32], src: f32, dst: f32) -> (Box<[f32]>, usize) {
    let ratio = dst / src;
    // `init`'s path choice: an integer ratio (×2 "DOUBLE", ×N "UP"), an integer inverse
    // (÷2 "HALF", ÷N "DOWN"), else "generic".
    let inv = src / dst;
    if ratio == 1.0 {
        return (input.into(), input.len());
    }
    if (ratio as i32) as f32 == ratio {
        // `init_intrat` sets the bookkeeping to cnt 0, rem 1, den N: N·len frames.
        let r = ratio as u32;
        let n = ((input.len() as f64 * r as f64) / 1.0).ceil() as usize;
        let t = intrat_taps(r, 1.0, 0.0);
        let out = if r == 2 { double(input, &t, n) } else { up(input, &t, r, n) };
        return (out.into(), n);
    }
    if (inv as i32) as f32 == inv && inv == 2.0 {
        let n = (input.len() as f64 * dst as f64 / (2.0 * dst as f64)).ceil() as usize;
        let t = intrat_taps(2, -1.0, 1.0);
        return (half(input, &t, n, ratio).into(), n);
    }
    // ÷N (N > 2, "DOWN") is not ported: such rates (11025 Hz, …) do not occur; the
    // generic converter stands in **[not exact]**.
    let mut c = Generic::new(src, dst);
    let num = ((input.len() as u32 + 1 - c.need) as i32 as f64) * c.den as f64 - c.frac as f64;
    let n = (num / (c.cnt as f32).mul_add(c.den, c.rem) as f64).ceil() as u32 as usize;
    let mut out = vec![0f32; n];
    c.convert(input, &mut out);
    (out.into(), n)
}

/// `init_intrat`'s polyphase taps: for phase p = 1…r−1 (position `start + p·step`,
/// `step = sign/r`, summed in `f32`) the 16 taps of a Kaiser-windowed sinc at
/// `x = pos + j`, j = −8…7 **[exact]**.
fn intrat_taps(r: u32, sign: f32, start: f32) -> Vec<f32> {
    let pi = f64::from_bits(0x4009_21fb_5444_2d11);
    let step = sign / r as f32;
    let mut pos = start;
    let mut t = Vec::with_capacity(16 * (r as usize - 1));
    for _ in 1..r {
        pos += step;
        for j in -8..8 {
            let x = pos + j as f32;
            if x == 0.0 {
                t.push(1.0);
                continue;
            }
            let a = x as f64 * pi;
            let s = a.sin();
            let u = ((x + x) * 0.0625) as f64;
            let b = (-u).mul_add(u, 1.0).sqrt() * 8.0;
            // I0(b), Abramowitz & Stegun 9.8.1/9.8.2 (`fmadd` Horner).
            let i0 = if b >= 3.75 {
                let y = 3.75 / b;
                let e = b.exp() / b.sqrt();
                let mut p = y.mul_add(f64::from_bits(0x3f70_125e_fd3b_7f88), f64::from_bits(0xbf90_df2b_ca08_4c55));
                for c in [
                    0x3f9a_fce6_f0e5_5ae3u64,
                    0xbf95_1227_1efd_c716,
                    0x3f82_c3f3_8a50_d154,
                    0xbf59_d0c1_4e13_894a,
                    0x3f62_7548_2b92_d5db,
                    0x3f8b_35a5_ff2d_9d1c,
                    0x3fd9_8845_3365_de00,
                ] {
                    p = y.mul_add(p, f64::from_bits(c));
                }
                e * p
            } else {
                let y = b / 3.75;
                let y = y * y;
                let mut p = y.mul_add(f64::from_bits(0x3f72_c3d7_5ac5_4874), f64::from_bits(0x3fa2_78a8_8849_e5fd));
                for c in [0x3fd1_05b4_74e7_32aau64, 0x3ff3_4ed8_3fc7_962c, 0x4008_b833_b894_30a4, 0x400c_1ffe_e624_8ca7] {
                    p = y.mul_add(p, f64::from_bits(c));
                }
                y.mul_add(p, 1.0)
            };
            t.push(((s / a) * i0 / f64::from_bits(0x407a_b906_bec5_d25f)) as f32);
        }
    }
    t
}

/// The ×2 kernel on the 16 state values `s` (`process_intratDOUBLE`, also `HALF`'s
/// odd-sample step): taps are symmetric, so only `t[0..8]` are used, the second half in
/// mirrored lane order. Returns the new accumulator lane 0.
fn half_band(s: &mut [f32], t: &[f32], v: f32) -> f32 {
    let p0: [f32; 4] = std::array::from_fn(|l| v * t[l]);
    let p1: [f32; 4] = std::array::from_fn(|l| v * t[4 + l]);
    let a0: [f32; 4] = std::array::from_fn(|l| s[l] + p0[l]);
    let b0: [f32; 4] = std::array::from_fn(|l| p0[l] + s[4 + l]);
    let a1: [f32; 4] = std::array::from_fn(|l| s[8 + l] + p1[l]);
    let b1: [f32; 4] = std::array::from_fn(|l| p1[l] + s[12 + l]);
    // Lanes rotated left (a) and right (b), then the hand-overs between the groups.
    let n = [a0[1], a0[2], a0[3], a1[0], 0.0, b0[0], b0[1], b0[2], a1[1], a1[2], a1[3], b1[3], b0[3], b1[0], b1[1], b1[2]];
    s.copy_from_slice(&n);
    a0[0]
}

/// ×r (`process_intratUP`), from a reset state, one call: phase 0 is the input delayed
/// by 8 frames, phases 1…r−1 transposed 16-tap FIRs fed with the newest input frame.
fn up(input: &[f32], t: &[f32], r: u32, n: usize) -> Vec<f32> {
    let mut s = vec![0f32; 16 * (r as usize - 1)];
    let mut out = Vec::with_capacity(n);
    let mut v = input[0];
    let (mut next, mut direct, mut delay) = (1usize, 0usize, 8u32);
    let mut phase = 0u32;
    while out.len() < n {
        if phase == 0 {
            if delay > 0 {
                delay -= 1;
                out.push(0.0); // the reset delay line
            } else {
                out.push(input[direct]);
                direct += 1;
            }
            phase = 1;
        } else {
            let p = (phase - 1) as usize;
            let (sp, tp) = (&mut s[16 * p..16 * p + 16], &t[16 * p..16 * p + 16]);
            let acc: [f32; 16] = std::array::from_fn(|i| v * tp[i] + sp[i]);
            out.push(acc[0]);
            sp[..15].copy_from_slice(&acc[1..]);
            sp[15] = 0.0;
            phase += 1;
        }
        if phase == r {
            phase = 0;
            if next < input.len() {
                v = input[next];
                next += 1;
            }
        }
    }
    out
}

/// ×2 (`process_intratDOUBLE`), from a reset state, one call.
fn double(input: &[f32], t: &[f32], n: usize) -> Vec<f32> {
    let mut s = [0f32; 16];
    let mut out = Vec::with_capacity(n);
    let mut v = input[0];
    let (mut next, mut direct, mut delay) = (1usize, 0usize, 8u32);
    let mut phase = 0;
    while out.len() < n {
        if phase == 0 {
            if delay > 0 {
                delay -= 1;
                out.push(0.0);
            } else {
                out.push(input[direct]);
                direct += 1;
            }
            phase = 1;
        } else {
            out.push(half_band(&mut s, t, v));
            phase = 0;
            if next < input.len() {
                v = input[next];
                next += 1;
            }
        }
    }
    out
}

/// ÷2 (`process_intratHALF`), from a reset state, one call: odd input frames go through
/// the half-band kernel into an accumulator, even ones (delayed by 16 frames) are added
/// and the sum, times the ratio, is the output.
fn half(input: &[f32], t: &[f32], n: usize, ratio: f32) -> Vec<f32> {
    let mut s = [0f32; 16];
    let mut out = Vec::with_capacity(n);
    let mut acc = 0f32;
    let (mut direct, mut delay) = (0usize, 8u32);
    let mut odd = false;
    for &x in input {
        if odd {
            let a = half_band(&mut s, t, x);
            acc += a;
        } else {
            let d = if delay > 0 {
                delay -= 1;
                0.0
            } else {
                let d = input[direct];
                direct += 2;
                d
            };
            acc += d;
            if out.len() < n {
                out.push(acc * ratio);
                acc = 0.0;
            }
        }
        odd = !odd;
    }
    out.resize(n, 0.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binomials() {
        assert_eq!(binom(5, 2), 10);
        assert_eq!(binom(4, 4), 1);
        assert_eq!(binom(5, 0), 1);
    }

    #[test]
    fn upsampled_sine_stays_a_sine() {
        let input: Vec<f32> = (0..4410).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let (out, n) = convert(&input, 44100.0, 48000.0);
        assert_eq!(n, ((4410.0f64 * 48000.0 / 44100.0).ceil()) as usize);
        // Away from the edges the output follows sin(0.05·t·44100/48000).
        let k = 44100.0 / 48000.0;
        for i in 200..4000 {
            let want = ((i as f32 - 4.0) * k * 0.05).sin() * 0.5;
            assert!((out[i] - want).abs() < 0.02, "{i}: {} vs {want}", out[i]);
        }
    }

    /// `RS_DUMP=dir cargo test … resample::tests::dump -- --ignored` writes
    /// `rs_pink_<sr>_rs.bin` (f32 LE) for comparison with the emulated Maschine.
    #[test]
    #[ignore]
    fn dump() {
        let dir = std::env::var("RS_DUMP").expect("RS_DUMP");
        for sr in [48000.0f32, 96000.0, 32000.0, 192000.0, 88200.0, 176400.0, 22050.0] {
            let (d, _) = pink_at(sr);
            let b: Vec<u8> = d.iter().flat_map(|x| x.to_le_bytes()).collect();
            std::fs::write(format!("{dir}/rs_pink_{}_rs.bin", sr as u32), b).unwrap();
        }
    }
}
