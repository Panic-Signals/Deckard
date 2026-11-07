//! The levels and meters of the interface's first two stereo inputs (a JAM's IN1 and IN2;
//! Preferences ▸ Audio). The level is DECKARD's own, on the way in: the sounds taking that
//! input and the recorder both hear it after it. Read and written on the audio threads,
//! so all atomics.

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// Stereo inputs with a level and a meter.
pub const PAIRS: usize = 2;

const CHANNELS: usize = PAIRS * 2;

/// The inputs' gains and peaks, by Maschine's input channel (In 1 L, In 1 R, In 2 L, …).
pub struct InputBus {
    gains: [AtomicU32; CHANNELS],
    peaks: [AtomicU32; CHANNELS],
}

impl Default for InputBus {
    fn default() -> Self {
        InputBus { gains: std::array::from_fn(|_| AtomicU32::new(1f32.to_bits())), peaks: Default::default() }
    }
}

impl InputBus {
    /// The levels, as stored (0..1, 0.75 = unity, as a fader's).
    pub fn set_levels(&self, levels: &[f64; PAIRS]) {
        for (channel, gain) in self.gains.iter().enumerate() {
            let level = deckard_model::level_gain(levels[channel / 2]) as f32;
            gain.store(level.to_bits(), Relaxed);
        }
    }

    /// A channel's gain: unity for one with no level of its own.
    pub fn gain(&self, channel: usize) -> f32 {
        self.gains.get(channel).map_or(1.0, |g| f32::from_bits(g.load(Relaxed)))
    }

    /// `sample` of `channel`, as it comes in: after its level, and metered.
    pub fn take_in(&self, channel: usize, sample: f32) -> f32 {
        let out = sample * self.gain(channel);
        if let Some(peak) = self.peaks.get(channel)
            && out.abs() > f32::from_bits(peak.load(Relaxed))
        {
            peak.store(out.abs().to_bits(), Relaxed);
        }
        out
    }

    /// A frame of the interface as Maschine's 32 input channels (`map`: which of the
    /// interface's channels each reads).
    pub fn frame(&self, interface: &[f32], map: &[Option<usize>]) -> [f32; 32] {
        let mut frame = [0f32; 32];
        for (i, c) in map.iter().enumerate().take(32) {
            if let Some(&v) = c.and_then(|c| interface.get(c)) {
                frame[i] = self.take_in(i, v);
            }
        }
        frame
    }

    /// Stereo input `pair`'s peaks (left, right) since the last call.
    pub fn take_peaks(&self, pair: usize) -> (f32, f32) {
        let take = |c: usize| self.peaks.get(c).map_or(0.0, |p| f32::from_bits(p.swap(0, Relaxed)));
        (take(pair * 2), take(pair * 2 + 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_start_at_unity() {
        let bus = InputBus::default();
        assert_eq!(bus.gain(0), 1.0);
        assert_eq!(bus.take_in(3, 0.5), 0.5);
    }

    #[test]
    fn a_frame_comes_in_through_its_level_and_is_metered() {
        let bus = InputBus::default();
        bus.set_levels(&[deckard_model::level_from_db(-6.0), 0.75]);
        let map = [Some(1), Some(0), Some(2), None, Some(9)];
        let frame = bus.frame(&[0.5, -1.0, 0.25], &map);
        let g = deckard_model::level_gain(deckard_model::level_from_db(-6.0)) as f32;
        assert!((frame[0] + g).abs() < 1e-6, "In 1 L reads the interface's second: {frame:?}");
        assert!((frame[1] - 0.5 * g).abs() < 1e-6);
        assert_eq!(frame[2], 0.25, "In 2 at unity");
        assert_eq!(frame[3], 0.0, "not connected");
        assert_eq!(frame[4], 0.0, "no such channel on the interface");
        let (l, r) = bus.take_peaks(0);
        assert!((l - g).abs() < 1e-6 && (r - 0.5 * g).abs() < 1e-6, "after the level: {l} {r}");
        assert_eq!(bus.take_peaks(1), (0.25, 0.0));
        assert_eq!(bus.take_peaks(0), (0.0, 0.0), "taken");
    }

    #[test]
    fn inputs_past_the_second_pass_as_they_are() {
        let bus = InputBus::default();
        bus.set_levels(&[0.0, 0.0]);
        assert_eq!(bus.gain(4), 1.0);
        assert_eq!(bus.take_in(4, 0.5), 0.5);
        assert_eq!(bus.take_peaks(2), (0.0, 0.0));
        assert_eq!(bus.take_in(0, 0.5), 0.0, "a closed fader");
    }
}
