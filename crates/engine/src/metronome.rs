//! The metronome: a short click on every beat while the sequencer runs, higher on the
//! first beat of each bar. The click is synthesized here (a decaying sine), so nothing
//! is loaded.

use std::f64::consts::TAU;

/// How long a click rings, and its level.
const CLICK_SECONDS: f64 = 0.03;
const LEVEL: f32 = 0.3;
const PITCH: f64 = 1000.0;
const ACCENT_PITCH: f64 = 1500.0;

pub(crate) struct Click {
    /// The click sounding: samples into it, and its pitch.
    sounding: Option<(u32, f64)>,
    /// Its volume, 0..1 (the Settings page's metronome VOLUME).
    pub(crate) volume: f32,
}

impl Default for Click {
    fn default() -> Click {
        Click { sounding: None, volume: 1.0 }
    }
}

impl Click {
    /// Add the clicks for `out` (interleaved stereo), which starts at timeline frame
    /// `frame` (`None`: the sequencer is stopped, so only a click still ringing sounds).
    /// A beat is `beat_frames` long; `beats_per_bar` beats make a bar.
    pub(crate) fn mix(&mut self, out: &mut [f32], frame: Option<u64>, beat_frames: f64, beats_per_bar: i64, sr: f64) {
        let len = (CLICK_SECONDS * sr) as u32;
        for (k, pair) in out.chunks_exact_mut(2).enumerate() {
            if let Some(f) = frame.filter(|_| beat_frames >= 1.0) {
                let f = f + k as u64;
                let beat = ((f as f64 + 0.5) / beat_frames).floor() as i64;
                if f == (beat as f64 * beat_frames).round() as u64 {
                    let accent = beat.rem_euclid(beats_per_bar.max(1)) == 0;
                    self.sounding = Some((0, if accent { ACCENT_PITCH } else { PITCH }));
                }
            }
            let Some((n, pitch)) = self.sounding.as_mut() else { continue };
            let t = *n as f64 / sr;
            let env = (1.0 - *n as f32 / len as f32).max(0.0).powi(3);
            let v = (TAU * *pitch * t).sin() as f32 * env * LEVEL * self.volume;
            pair[0] += v;
            pair[1] += v;
            *n += 1;
            if *n >= len {
                self.sounding = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clicks start on each beat, the bar's first higher, and ring out after a stop.
    #[test]
    fn clicks_fall_on_the_beats() {
        let sr = 48_000.0;
        let beat = sr / 2.0; // 120 BPM
        let mut c = Click::default();
        let mut out = vec![0.0f32; 48_000 * 2 * 2];
        c.mix(&mut out, Some(0), beat, 4, sr);
        let onsets: Vec<usize> = (0..out.len() / 2)
            .filter(|&k| out[k * 2] == 0.0 && out.get(k * 2 + 2).is_some_and(|v| *v != 0.0) && (k == 0 || out[k * 2 - 2] == 0.0))
            .collect();
        // Each click starts at a beat (its first sample is sin(0) = 0).
        assert_eq!(onsets, [0, 24_000, 48_000, 72_000]);
        let mut silence = vec![0.0f32; 4096];
        c.mix(&mut silence, None, beat, 4, sr);
        assert!(silence.iter().all(|v| *v == 0.0), "nothing new while stopped");
    }

    /// The click's level follows the metronome's volume.
    #[test]
    fn the_click_follows_the_volume() {
        let peak = |volume: f32| {
            let mut c = Click { volume, ..Click::default() };
            let mut out = vec![0.0f32; 4096];
            c.mix(&mut out, Some(0), 24_000.0, 4, 48_000.0);
            out.iter().fold(0.0f32, |m, v| m.max(v.abs()))
        };
        assert!(peak(1.0) > 0.1);
        assert!((peak(0.5) - peak(1.0) / 2.0).abs() < 1e-6, "half");
        assert_eq!(peak(0.0), 0.0, "silent");
    }
}
