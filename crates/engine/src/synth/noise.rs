//! DECKARD's own noise sources for the drum synths' noise layers, standing in for the
//! recordings compiled into Maschine (pink noise, the 23 "modal" noises, the Hi-hat
//! Memory engine's six cymbal sources), which DECKARD neither ships nor reads.
//!
//! The recordings are steady (no envelope: the engines loop them), so each is made here
//! as white noise shaped to the recording's measured spectral balance (energy per
//! octave, 31 Hz…16 kHz) and peak, in the frequency domain over the whole buffer, so it
//! loops without a seam. Only those few numbers were measured from the originals; the
//! audio is generated (a fixed seed per source, the same on every run). Being shaped by
//! octave, finer detail (narrow resonances) isn't there: close, not identical.

use std::sync::OnceLock;

/// Samples per source (the recordings' length), at [`RATE`].
pub const LEN: usize = 131_072;
/// The rate the sources are made for (the recordings').
pub const RATE: f64 = 44_100.0;
/// "Modal" noises, one after another in [`modal_noises`].
pub const MODAL: usize = 23;

/// A source's measured balance: energy per octave band from 31.25 Hz (dB, relative)
/// and its peak.
struct Profile {
    bands: [f32; 10],
    peak: f32,
}

const fn pr(bands: [f32; 10], peak: f32) -> Profile {
    Profile { bands, peak }
}

const PINK: Profile = pr([-0.6, -0.1, -0.4, 0.0, -0.3, -0.2, -0.6, -1.0, -1.2, -4.5], 1.0);

const MODAL_PROFILES: [Profile; MODAL] = [
    pr([-31.8, -17.0, -6.1, -8.2, -9.6, -5.8, -2.1, 0.0, -3.6, -17.7], 1.0),
    pr([-26.3, -24.8, -7.4, -9.3, -6.5, -6.5, -5.9, 0.0, -5.6, -20.7], 1.0),
    pr([-38.2, -31.9, -12.5, -16.3, -13.7, -10.7, -6.8, 0.0, -4.2, -21.8], 1.0),
    pr([-44.3, -30.3, -1.0, -6.0, -6.9, -5.1, -2.3, 0.0, -4.1, -18.4], 1.0),
    pr([-9.6, -5.9, -1.7, -1.1, 0.0, -6.1, -8.7, -8.2, -13.7, -31.9], 1.0),
    pr([-23.1, -21.7, -0.1, -0.3, 0.0, -0.6, -2.6, -5.0, -14.2, -31.4], 1.0),
    pr([-27.7, -25.5, 0.0, -12.2, -12.9, -13.5, -11.5, -12.8, -17.9, -29.3], 1.0),
    pr([-41.1, -32.6, -10.1, -4.0, 0.0, -6.1, -5.5, -6.4, -13.3, -30.1], 1.0),
    pr([-21.6, -20.9, -2.8, -4.8, -1.9, -1.9, -1.6, 0.0, -7.0, -17.0], 1.0),
    pr([-36.8, -27.2, 0.0, -13.7, -19.8, -23.4, -27.4, -31.8, -43.8, -72.3], 1.0),
    pr([-27.8, -20.6, 0.0, -13.3, -19.2, -17.2, -21.6, -28.0, -33.5, -60.5], 1.0),
    pr([-51.6, -30.1, 0.0, -14.8, -13.9, -17.3, -17.4, -21.5, -29.6, -54.0], 1.0),
    pr([-35.9, -29.5, 0.0, -12.8, -19.9, -22.2, -25.8, -30.0, -38.8, -64.0], 1.0),
    pr([-35.5, 0.0, -7.2, -16.0, -19.8, -19.0, -20.3, -26.8, -36.7, -63.2], 1.0),
    pr([-32.1, -8.8, 0.0, -10.4, -18.2, -22.7, -24.7, -25.8, -39.4, -59.3], 0.888),
    pr([-29.2, 0.0, -7.5, -15.2, -15.7, -18.6, -22.2, -26.1, -33.3, -60.0], 1.0),
    pr([-3.7, 0.0, -12.0, -24.9, -31.3, -19.3, -17.7, -26.1, -44.2, -46.5], 1.0),
    pr([0.0, -0.7, -6.1, -20.8, -26.1, -23.7, -17.4, -19.4, -31.2, -48.3], 1.0),
    pr([-11.7, -1.3, 0.0, -10.9, -13.3, -16.8, -18.0, -22.8, -36.6, -68.2], 1.0),
    pr([-4.8, 0.0, -10.8, -24.7, -33.0, -25.2, -24.4, -25.5, -35.8, -58.2], 1.0),
    pr([-40.9, -37.1, -36.9, -10.6, -9.6, -8.0, -3.8, 0.0, -4.7, -21.0], 0.707),
    pr([-19.6, -16.8, -24.0, 0.0, -1.4, -3.7, -2.3, -15.2, -41.5, -54.3], 0.891),
    pr([-38.2, -35.0, -24.3, 0.0, -5.3, -11.9, -9.1, -15.2, -38.7, -64.0], 0.891),
];

const CYMBAL_PROFILES: [Profile; 6] = [
    pr([-42.5, -37.3, -31.7, -16.4, -12.2, -7.7, -4.9, 0.0, -3.4, -22.3], 0.235),
    pr([-26.0, -29.0, -32.9, -18.6, -7.0, -9.0, -1.9, 0.0, -14.7, -67.5], 0.263),
    pr([-36.8, -36.0, -39.7, -26.1, -23.0, -24.1, -7.1, 0.0, -0.8, -49.0], 0.225),
    pr([-77.2, -75.4, -71.5, -68.5, -53.9, -36.0, -21.1, 0.0, -4.0, -30.1], 0.281),
    pr([-40.9, -37.1, -36.9, -10.6, -9.6, -8.0, -3.8, 0.0, -4.7, -21.0], 0.234),
    pr([-39.0, -33.2, -34.2, -11.7, -7.5, -9.4, -1.5, 0.0, -2.0, -29.9], 0.225),
];

/// In-place radix-2 FFT of `re`/`im` (length a power of two); `inverse` without the 1/n.
fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = std::f64::consts::TAU / len as f64 * if inverse { 1.0 } else { -1.0 };
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                (cr, ci) = (cr * wr - ci * wi, cr * wi + ci * wr);
            }
        }
        len <<= 1;
    }
}

/// Source `p`: white noise from `seed`, shaped to `p`'s octave balance, at `p`'s peak.
fn make(p: &Profile, seed: u64) -> Box<[f32]> {
    let mut s = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let mut white = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    let mut re: Vec<f64> = (0..LEN).map(|_| white()).collect();
    let mut im = vec![0.0; LEN];
    fft(&mut re, &mut im, false);
    // Per-bin power density of band b: its energy over its width (each octave twice
    // the last), flat across the band so each band's energy is the measured one.
    let density = |b: usize| (p.bands[b] as f64 / 10.0) * std::f64::consts::LOG2_10 - b as f64;
    for k in 1..=LEN / 2 {
        let f = k as f64 * RATE / LEN as f64;
        let d = density(((f / 31.25).log2().floor()).clamp(0.0, 9.0) as usize);
        let g = (d / 2.0).exp2();
        re[k] *= g;
        im[k] *= g;
        if k < LEN / 2 {
            re[LEN - k] = re[k];
            im[LEN - k] = -im[k];
        }
    }
    re[0] = 0.0;
    im[0] = 0.0;
    fft(&mut re, &mut im, true);
    let peak = re.iter().fold(0.0f64, |m, v| m.max(v.abs())).max(1e-12);
    let scale = p.peak as f64 / peak;
    re.iter().map(|v| (v * scale) as f32).collect()
}

/// Pink noise, [`LEN`] samples.
pub fn pink() -> &'static [f32] {
    static P: OnceLock<Box<[f32]>> = OnceLock::new();
    P.get_or_init(|| make(&PINK, 4))
}

/// The 23 "modal" noises of [`LEN`] samples, one after another.
pub fn modal_noises() -> &'static [f32] {
    static M: OnceLock<Box<[f32]>> = OnceLock::new();
    M.get_or_init(|| MODAL_PROFILES.iter().enumerate().flat_map(|(k, p)| make(p, 100 + k as u64).into_vec()).collect())
}

/// The Hi-hat Memory engine's source `k` (A–F), [`LEN`] samples.
pub fn cymbal(k: usize) -> &'static [f32] {
    static C: [OnceLock<Box<[f32]>>; 6] = [const { OnceLock::new() }; 6];
    let k = k.min(5);
    C[k].get_or_init(|| make(&CYMBAL_PROFILES[k], 200 + k as u64))
}

/// Make every source now (the first use would otherwise make them on the audio thread).
pub fn warm() {
    pink();
    modal_noises();
    for k in 0..6 {
        cymbal(k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band_db(x: &[f32]) -> [f64; 10] {
        let mut re: Vec<f64> = x.iter().map(|v| *v as f64).collect();
        let mut im = vec![0.0; x.len()];
        fft(&mut re, &mut im, false);
        let mut e = [0.0; 10];
        for (k, (r, i)) in re.iter().zip(&im).enumerate().take(x.len() / 2).skip(1) {
            let f = k as f64 * RATE / x.len() as f64;
            let b = ((f / 31.25).log2().floor()) as i64;
            if (0..10).contains(&b) {
                e[b as usize] += r * r + i * i;
            }
        }
        let top = e.iter().fold(0.0f64, |m, v| m.max(*v));
        e.map(|v| 10.0 * (v / top).log10())
    }

    /// Each source has its profile's peak and, within a few dB, its octave balance.
    #[test]
    fn sources_match_their_profiles() {
        let check = |x: &[f32], p: &Profile, name: &str| {
            assert_eq!(x.len(), LEN);
            let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!((peak - p.peak).abs() < 1e-3, "{name} peak {peak}");
            let got = band_db(x);
            for (b, (&g, &w)) in got.iter().zip(&p.bands).enumerate().take(9).skip(2) {
                let want = w as f64;
                if want > -30.0 {
                    assert!((g - want).abs() < 4.0, "{name} band {b}: {g:.1} vs {want}");
                }
            }
        };
        check(pink(), &PINK, "pink");
        for (k, p) in MODAL_PROFILES.iter().enumerate() {
            check(&modal_noises()[k * LEN..(k + 1) * LEN], p, &format!("modal {k}"));
        }
        for (k, p) in CYMBAL_PROFILES.iter().enumerate() {
            check(cymbal(k), p, &format!("cymbal {k}"));
        }
        assert_eq!(pink(), make(&PINK, 4).as_ref(), "the same every time");
    }
}
