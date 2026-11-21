//! Windowed stereo spectra for a logarithmic 20 Hz to 20 kHz display.

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

pub const POINTS: usize = 96;

struct PlotBin {
    lower: usize,
    upper: usize,
    fraction: f32,
    peak_bins: std::ops::RangeInclusive<usize>,
}

/// Reusable FFT workspace for one window size and sample rate.
pub struct Analyzer {
    hann: Vec<f32>,
    /// A real-input FFT: half the work of a complex one, and planned (twiddles, SIMD) once.
    fft: std::sync::Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    power: Vec<f32>,
    bins: Vec<PlotBin>,
}

impl Analyzer {
    pub fn new(n: usize, sample_rate: f32) -> Self {
        assert!(n.is_power_of_two() && sample_rate.is_finite() && sample_rate > 0.0);
        let hann = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()).collect();
        let nyquist = n / 2;
        let bin = |i: f32| 20.0 * 1000.0_f32.powf(i / (POINTS - 1) as f32) * n as f32 / sample_rate;
        let bins = (0..POINTS)
            .map(|i| i as f32)
            .take_while(|&i| bin(i) <= nyquist as f32)
            .map(|i| {
                let center = bin(i);
                let lower = center.floor() as usize;
                PlotBin {
                    lower,
                    upper: (lower + 1).min(nyquist),
                    fraction: center - lower as f32,
                    peak_bins: bin(i - 0.5).ceil() as usize..=(bin(i + 0.5).floor() as usize).min(nyquist),
                }
            })
            .collect();
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(n);
        let (input, spectrum, scratch) = (fft.make_input_vec(), fft.make_output_vec(), fft.make_scratch_vec());
        Self { hann, fft, input, spectrum, scratch, power: vec![0.0; nyquist + 1], bins }
    }

    pub fn curves(&mut self, window: &[[f32; 4]]) -> [[f32; POINTS]; 2] {
        [self.curve(window, 0), self.curve(window, 1)]
    }

    /// One stereo pair's curve: 0 the input (`window`'s first two channels), 1 the output.
    pub fn curve(&mut self, window: &[[f32; 4]], pair: usize) -> [f32; POINTS] {
        let Self { hann, fft, input, spectrum, scratch, power, bins } = self;
        let n = hann.len();
        assert_eq!(window.len(), n);
        assert!(pair < 2);
        let fft_scale = 4.0 / n as f32;
        let mut curve = [0.0; POINTS];
        // Average channel power, so opposite-phase signals stay visible.
        power.fill(0.0);
        for ch in [pair * 2, pair * 2 + 1] {
            for ((dst, sample), w) in input.iter_mut().zip(window).zip(hann.iter()) {
                *dst = if sample[ch].is_finite() { sample[ch] * w } else { 0.0 };
            }
            // (Only fails on wrongly sized buffers, which `new` made.)
            let _ = fft.process_with_scratch(input, spectrum, scratch);
            for (p, c) in power.iter_mut().zip(spectrum.iter()) {
                *p += 0.5 * fft_scale * fft_scale * c.norm_sqr();
            }
        }

        for (y, bin) in curve.iter_mut().zip(bins.iter()) {
            let interp = power[bin.lower] * (1.0 - bin.fraction) + power[bin.upper] * bin.fraction;
            // Keep peaks between plot points visible at high frequencies.
            let peak = bin.peak_bins.clone().fold(interp, |p, b| p.max(power[b]));
            *y = ((10.0 * peak.log10() + 90.0) / 90.0).clamp(0.0, 1.0);
        }
        curve
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opposite_phase_stereo_preserves_the_tone_and_output_gain() {
        let n = 4096;
        let rate = 48_000.0;
        let bin = 96;
        let window: Vec<_> = (0..n)
            .map(|i| {
                let x = (std::f32::consts::TAU * (i * bin) as f32 / n as f32).sin();
                [x, -x, 0.1 * x, -0.1 * x]
            })
            .collect();
        let mut analyzer = Analyzer::new(n, rate);
        let [input, output] = analyzer.curves(&window);
        assert_eq!(analyzer.curves(&vec![[0.0; 4]; n]), [[0.0; POINTS]; 2]);
        assert_eq!(analyzer.curves(&window), [input, output]);
        let peak = input.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        let expected = ((bin as f32 * rate / n as f32 / 20.0).log10() / 3.0 * (POINTS - 1) as f32).round() as usize;
        assert_eq!(peak, expected);
        assert!((input[peak] - 1.0).abs() < 0.01);
        assert!((90.0 * (input[peak] - output[peak]) - 20.0).abs() < 0.01);
    }

    /// The real FFT draws what the complex one (`reference_fft`) did, on noise with a tone.
    #[test]
    fn draws_what_the_complex_fft_drew() {
        let n = 4096;
        let rate = 48_000.0;
        let mut seed = 0x1234_5678u32;
        let mut noise = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32 - 0.5
        };
        let window: Vec<[f32; 4]> = (0..n)
            .map(|i| {
                let tone = (std::f32::consts::TAU * 1000.0 * i as f32 / rate).sin();
                [tone + 0.1 * noise(), 0.3 * noise(), 0.01 * noise(), tone * 0.5]
            })
            .collect();
        let mut analyzer = Analyzer::new(n, rate);
        let got = analyzer.curves(&window);

        // The old way: a complex FFT a channel, imaginary part zero.
        let scale = 4.0 / n as f32;
        let mut want = [[0.0; POINTS]; 2];
        for (pair, curve) in want.iter_mut().enumerate() {
            let mut power = vec![0.0f32; n / 2 + 1];
            for ch in [pair * 2, pair * 2 + 1] {
                let mut re: Vec<f32> = window.iter().zip(&analyzer.hann).map(|(s, w)| s[ch] * w).collect();
                let mut im = vec![0.0; n];
                reference_fft(&mut re, &mut im);
                for (i, p) in power.iter_mut().enumerate() {
                    *p += 0.5 * scale * scale * (re[i] * re[i] + im[i] * im[i]);
                }
            }
            for (y, bin) in curve.iter_mut().zip(analyzer.bins.iter()) {
                let interp = power[bin.lower] * (1.0 - bin.fraction) + power[bin.upper] * bin.fraction;
                let peak = bin.peak_bins.clone().fold(interp, |p, b| p.max(power[b]));
                *y = ((10.0 * peak.log10() + 90.0) / 90.0).clamp(0.0, 1.0);
            }
        }
        for (g, w) in got.iter().flatten().zip(want.iter().flatten()) {
            // 0.001 of the 90 dB scale is 0.09 dB.
            assert!((g - w).abs() < 0.001, "{g} vs {w}");
        }
    }

    /// The radix-2 complex FFT this used before.
    fn reference_fft(re: &mut [f32], im: &mut [f32]) {
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
            let ang = -2.0 * std::f64::consts::PI / len as f64;
            let (wr, wi) = (ang.cos(), ang.sin());
            for start in (0..n).step_by(len) {
                let (mut cr, mut ci) = (1f64, 0f64);
                for k in 0..len / 2 {
                    let (a, b) = (start + k, start + k + len / 2);
                    let (xr, xi) = (re[b] * cr as f32 - im[b] * ci as f32, re[b] * ci as f32 + im[b] * cr as f32);
                    re[b] = re[a] - xr;
                    im[b] = im[a] - xi;
                    re[a] += xr;
                    im[a] += xi;
                    (cr, ci) = (cr * wr - ci * wi, cr * wi + ci * wr);
                }
            }
            len <<= 1;
        }
    }
}
