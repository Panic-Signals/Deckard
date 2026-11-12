//! The New Sampler's tempo and pitch controls, as numbers (roadmap 1d): tap tempo, the
//! linked BPM and beat count, the target note's Tune, and the sample's bars and beats.

use deckard_model::new_sampler::{BpmSource, ZoneMeta};

/// Taps further apart than this start a new count, seconds.
pub const TAP_GAP: f64 = 2.0;
/// Taps counted at most (the latest).
const TAPS: usize = 8;

/// The tempo `taps` (times in seconds, in order) give: the mean of their latest gaps.
pub fn tap_bpm(taps: &[f64]) -> Option<f64> {
    let t = &taps[taps.len().saturating_sub(TAPS)..];
    if t.len() < 2 {
        return None;
    }
    let span = t[t.len() - 1] - t[0];
    (span > 0.0).then(|| 60.0 * (t.len() - 1) as f64 / span)
}

/// `meta` with its tempo set by hand to `bpm` (its beat count following, for a sample of
/// `seconds`).
pub fn with_bpm(meta: &ZoneMeta, bpm: f64, seconds: f64) -> ZoneMeta {
    let bpm = (bpm * 100.0).round().clamp(2_000.0, 99_900.0) / 100.0;
    ZoneMeta { bpm: Some(bpm), beats: Some(seconds * bpm / 60.0), bpm_source: BpmSource::Manual, ..meta.clone() }
}

/// `meta` with its length set to `beats` (its tempo following).
pub fn with_beats(meta: &ZoneMeta, beats: f64, seconds: f64) -> Option<ZoneMeta> {
    (beats > 0.0 && seconds > 0.0).then(|| ZoneMeta { beats: Some(beats), ..with_bpm(meta, 60.0 * beats / seconds, seconds) })
}

/// `meta` with its root note set by hand (locked: analysis leaves it).
pub fn with_root(meta: &ZoneMeta, root: u8) -> ZoneMeta {
    ZoneMeta { root: Some(root.min(127)), cents: if meta.root == Some(root) { meta.cents } else { 0.0 }, root_locked: true, ..meta.clone() }
}

/// The note a sample of root `root` (`cents` off it) sounds at with Tune `tune`.
pub fn target_of(root: u8, cents: f32, tune: f64) -> i64 {
    (f64::from(root) + f64::from(cents) / 100.0 + tune).round() as i64
}

/// The Tune (semitones) making a sample of root `root`, `cents` off, sound exactly at
/// `target` (its cents tuned out).
pub fn tune_for(root: u8, cents: f32, target: i64) -> f64 {
    let t = (target - i64::from(root)) as f64 - f64::from(cents) / 100.0;
    (t * 100.0).round() / 100.0
}

/// The beats of a sample at `bpm` (`rate` frames a second, the first on frame
/// `offset`) between frames `from` and `to`: (frame, bar from 1, beat in it from 1).
pub fn beats_in(bpm: f64, offset: f64, rate: u32, from: f64, to: f64) -> Vec<(f64, i64, i64)> {
    if !(bpm > 0.0) || rate == 0 || to <= from {
        return Vec::new();
    }
    let per = rate as f64 * 60.0 / bpm;
    if (to - from) / per > 4000.0 {
        return Vec::new();
    }
    let first = ((from - offset) / per).ceil() as i64;
    let last = ((to - offset) / per).floor() as i64;
    (first..=last).map(|k| (offset + k as f64 * per, k.div_euclid(4) + 1, k.rem_euclid(4) + 1)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taps() {
        assert_eq!(tap_bpm(&[1.0]), None);
        assert_eq!(tap_bpm(&[1.0, 1.5, 2.0, 2.5]), Some(120.0));
        // Only the latest eight count.
        let mut t: Vec<f64> = (0..4).map(|k| k as f64).collect();
        t.extend((0..8).map(|k| 10.0 + k as f64 * 0.5));
        assert_eq!(tap_bpm(&t), Some(120.0));
    }

    #[test]
    fn bpm_and_beats_are_linked() {
        let m = ZoneMeta { bpm: Some(120.0), beats: Some(16.0), ..ZoneMeta::default() };
        let h = with_bpm(&m, 60.0, 8.0);
        assert_eq!((h.bpm, h.beats, h.bpm_source), (Some(60.0), Some(8.0), BpmSource::Manual));
        let b = with_beats(&m, 8.0, 8.0).unwrap();
        assert_eq!((b.bpm, b.beats), (Some(60.0), Some(8.0)));
        assert!(with_beats(&m, 0.0, 8.0).is_none());
    }

    #[test]
    fn the_target_note_sets_tune() {
        // A1 (45) 12 cents flat, to C2 (48): +3 semitones and the cents back.
        assert_eq!(tune_for(45, -12.0, 48), 3.12);
        assert_eq!(target_of(45, -12.0, 3.12), 48);
        assert_eq!(target_of(45, 0.0, 0.0), 45);
        let r = with_root(&ZoneMeta { root: Some(45), cents: -12.0, ..ZoneMeta::default() }, 47);
        assert_eq!((r.root, r.cents, r.root_locked), (Some(47), 0.0, true));
    }

    #[test]
    fn bars_and_beats() {
        // 120 BPM at 48 kHz: a beat every 24 000 frames, the first at 1 000.
        let b = beats_in(120.0, 1000.0, 48_000, 0.0, 100_000.0);
        assert_eq!(b.len(), 5);
        assert_eq!(b[0], (1000.0, 1, 1));
        assert_eq!(b[4], (97_000.0, 2, 1));
        assert!(beats_in(0.0, 0.0, 48_000, 0.0, 1.0).is_empty());
    }
}
