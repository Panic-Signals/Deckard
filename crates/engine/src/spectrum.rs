//! Windowed stereo spectra for a logarithmic 20 Hz to 20 kHz display.

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
    re: Vec<f32>,
    im: Vec<f32>,
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
        Self { hann, re: vec![0.0; n], im: vec![0.0; n], power: vec![0.0; nyquist + 1], bins }
    }

    pub fn curves(&mut self, window: &[[f32; 4]]) -> [[f32; POINTS]; 2] {
        let Self { hann, re, im, power, bins } = self;
        let n = hann.len();
        assert_eq!(window.len(), n);
        let fft_scale = 4.0 / n as f32;
        let mut curves = [[0.0; POINTS]; 2];
        for (pair, curve) in curves.iter_mut().enumerate() {
            // Average channel power, so opposite-phase signals stay visible.
            power.fill(0.0);
            for ch in [pair * 2, pair * 2 + 1] {
                for ((dst, sample), w) in re.iter_mut().zip(window).zip(hann.iter()) {
                    *dst = if sample[ch].is_finite() { sample[ch] * w } else { 0.0 };
                }
                im.fill(0.0);
                fft(re, im);
                for (i, p) in power.iter_mut().enumerate() {
                    *p += 0.5 * fft_scale * fft_scale * (re[i] * re[i] + im[i] * im[i]);
                }
            }

            for (y, bin) in curve.iter_mut().zip(bins.iter()) {
                let interp = power[bin.lower] * (1.0 - bin.fraction) + power[bin.upper] * bin.fraction;
                // Keep peaks between plot points visible at high frequencies.
                let peak = bin.peak_bins.clone().fold(interp, |p, b| p.max(power[b]));
                *y = ((10.0 * peak.log10() + 90.0) / 90.0).clamp(0.0, 1.0);
            }
        }
        curves
    }
}

fn fft(re: &mut [f32], im: &mut [f32]) {
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
}
