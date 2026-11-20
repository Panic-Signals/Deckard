//! The New Sampler's tempo and pitch controls, as numbers (roadmap 1d): tap tempo, the
//! linked BPM and beat count, the target note's Tune, and the sample's bars and beats.

use deckard_model::new_sampler::{BeatMap, BpmSource, ZoneMeta};

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
    ZoneMeta {
        root: Some(root.min(127)),
        cents: if meta.root == Some(root) { meta.cents } else { 0.0 },
        root_locked: true,
        root_guess: false,
        key_minor: None,
        ..meta.clone()
    }
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

/// Match key: the Tune putting a sample of root `root` (`cents` off) on the nearest note
/// named as `key` (any octave; a tritone away, the one below), its cents tuned out.
pub fn match_tune(root: u8, cents: f32, key: i64) -> f64 {
    let up = (key - i64::from(root)).rem_euclid(12);
    tune_for(root, cents, i64::from(root) + if up < 6 { up } else { up - 12 })
}

/// The beats of a sample (through its beat map) between frames `from` and `to`:
/// (frame, bar from 1, beat in it from 1).
pub fn beats_in(map: &BeatMap, from: f64, to: f64) -> Vec<(f64, i64, i64)> {
    if to <= from {
        return Vec::new();
    }
    let (b0, b1) = (map.beat_at(from), map.beat_at(to));
    if !(b1 - b0 < 4000.0) {
        return Vec::new();
    }
    (b0.ceil() as i64..=b1.floor() as i64).map(|k| (map.frame_at(k as f64), k.div_euclid(4) + 1, k.rem_euclid(4) + 1)).collect()
}

/// The finest of a bar, a beat, 1/2 … 1/16 of a beat that is at least `min` frames wide
/// at `frame` (so the grid snapped to is one that can be seen): in beats.
pub fn snap_division(map: &BeatMap, frame: f64, min: f64) -> f64 {
    let beat = (map.frame_at(map.beat_at(frame) + 1.0) - frame).abs();
    [0.0625, 0.125, 0.25, 0.5, 1.0].into_iter().find(|d| beat * d >= min).unwrap_or(4.0)
}

/// `frame` moved to the nearest multiple of `division` beats of the sample's grid.
pub fn snap(map: &BeatMap, frame: f64, division: f64) -> f64 {
    if !(division > 0.0) {
        return frame;
    }
    map.frame_at((map.beat_at(frame) / division).round() * division).round()
}

/// Grid slices of `[a, b)` on the sample's own beats (through its warp markers): `a`,
/// then every `division`th note's line after it (4: quarters).
pub fn grid_slices(map: &BeatMap, (a, b): (usize, usize), division: u32) -> Vec<usize> {
    let step = 4.0 / f64::from(division.max(1));
    let mut out = vec![a];
    let mut k = (map.beat_at(a as f64) / step).floor() + 1.0;
    // (A line a few frames after the start isn't a slice of its own.)
    let min = (map.frame_at(step) - map.frame_at(0.0)).abs() / 8.0;
    while out.len() < 4096 {
        let f = map.frame_at(k * step).round();
        if !(f < b as f64) {
            break;
        }
        if f - a as f64 > min {
            out.push(f as usize);
        }
        k += 1.0;
    }
    out
}

/// The stem lane (0 Drums, 1 Vocals, 2 Bass, 3 Leads) a stem file called `name` goes on,
/// by what its name says; the next free one of `taken` if that one is; none when all
/// four are.
pub fn lane_for(name: &str, taken: &[bool; 4]) -> Option<usize> {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    let want = if has(&["drum", "perc", "beat", "kick"]) {
        0
    } else if has(&["voc", "vox", "acap", "sing"]) {
        1
    } else if has(&["bass", "sub"]) {
        2
    } else {
        3
    };
    (0..4).map(|k| (want + k) % 4).find(|&k| !taken[k])
}

/// Quantize grids, beats: 1/4, 1/8, 1/16, 1/32 notes.
pub const WARP_GRIDS: [(f64, &str); 4] = [(1.0, "1/4"), (0.5, "1/8"), (0.25, "1/16"), (0.125, "1/32")];

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
    fn match_key_takes_the_nearest_note() {
        // A1 (45) to a song in C: up 3 to C2. To E: down 5. To D# (a tritone): down 6.
        assert_eq!(match_tune(45, 0.0, 60), 3.0);
        assert_eq!(match_tune(45, 0.0, 64), -5.0);
        assert_eq!(match_tune(45, 0.0, 63), -6.0);
        assert_eq!(match_tune(45, 20.0, 45), -0.2, "in key already: only its cents");
    }

    #[test]
    fn snapping_and_grid_slices_follow_the_sample() {
        // 120 BPM at 48 kHz, the first beat at frame 1 000.
        let m = ZoneMeta { bpm: Some(120.0), grid_offset: 1000, ..ZoneMeta::default() };
        let map = BeatMap::of(&m, 48_000).unwrap();
        assert_eq!(snap(&map, 24_000.0, 1.0), 25_000.0);
        assert_eq!(snap(&map, 14_000.0, 0.5), 13_000.0);
        assert_eq!(snap(&map, 14_000.0, 0.0), 14_000.0);
        // A beat is 24 000 frames: lines 2 000 apart or more are 1/8 beats and up.
        assert_eq!(snap_division(&map, 0.0, 2_000.0), 0.125);
        assert_eq!(snap_division(&map, 0.0, 30_000.0), 4.0);
        // Quarters from the start of the range: the start, then the beats after it.
        assert_eq!(grid_slices(&map, (0, 60_000), 4), [0, 25_000, 49_000]);
        assert_eq!(grid_slices(&map, (1000, 40_000), 8), [1000, 13_000, 25_000, 37_000]);
        // (A line just after the start isn't a slice.)
        assert_eq!(grid_slices(&map, (990, 30_000), 4), [990, 25_000]);
    }

    #[test]
    fn stem_files_find_their_lanes() {
        let mut taken = [false; 4];
        for (name, lane) in [("Song (Vocals).wav", 1), ("song_drums.wav", 0), ("SubBass.wav", 2), ("Keys.wav", 3), ("Guitar.wav", 0)] {
            let got = lane_for(name, &taken);
            if name == "Guitar.wav" {
                assert_eq!(got, None, "all four taken");
            } else {
                assert_eq!(got, Some(lane), "{name}");
                taken[lane] = true;
            }
        }
        assert_eq!(lane_for("Pad.wav", &[false, false, false, true]), Some(0), "Leads taken: the next free");
    }

    #[test]
    fn bars_and_beats() {
        // 120 BPM at 48 kHz: a beat every 24 000 frames, the first at 1 000.
        let m = ZoneMeta { bpm: Some(120.0), grid_offset: 1000, ..ZoneMeta::default() };
        let map = BeatMap::of(&m, 48_000).unwrap();
        let b = beats_in(&map, 0.0, 100_000.0);
        assert_eq!(b.len(), 5);
        assert_eq!(b[0], (1000.0, 1, 1));
        assert_eq!(b[4], (97_000.0, 2, 1));
        assert!(beats_in(&map, 1.0, 0.0).is_empty());
    }
}
