//! Chopping by playing (sampler roadmap, phase 3): where a slice caught at the playhead
//! goes, where Split and Grid start, what Random picks, and where a slice's pad is heard.

use deckard_model::new_sampler::BeatMap;
use std::time::Instant;

/// The grid a caught slice lands on, in beats: a 1/8 note.
const CATCH_GRID: f64 = 0.5;

/// Where a slice caught at `playhead` goes: on the sample's nearest 1/8 note (`map`),
/// or just there (`free`, or a sample with no tempo). None outside what's sliced, and
/// at its start, where the first slice is already.
pub fn catch_at(playhead: usize, map: Option<&BeatMap>, free: bool, range: (usize, usize)) -> Option<usize> {
    if playhead < range.0 || playhead >= range.1 {
        return None;
    }
    let at = match map.filter(|_| !free) {
        Some(m) => crate::beat::snap(m, playhead as f64, CATCH_GRID).max(0.0) as usize,
        None => playhead,
    };
    (at > range.0 && at < range.1).then_some(at)
}

/// What Split and Grid slice: `range` from the playhead on while the sample plays, else
/// from the start of a selection inside it, else all of it.
pub fn slicer_range(range: (usize, usize), playhead: Option<usize>, selection: Option<(usize, usize)>) -> (usize, usize) {
    let inside = |f: &usize| *f > range.0 && *f + 1 < range.1;
    let from = playhead.filter(inside).or(selection.map(|s| s.0).filter(inside));
    (from.unwrap_or(range.0), range.1)
}

/// Random's slices: the `kept` ones and, of `cands`, as many more as make `count`, none
/// within `near` frames of a kept one. The same `seed` picks the same.
pub fn random_picks(cands: &[usize], kept: &[usize], count: usize, seed: u64, near: usize) -> Vec<usize> {
    let mut free: Vec<usize> = cands.iter().copied().filter(|c| kept.iter().all(|k| k.abs_diff(*c) > near)).collect();
    free.sort_unstable();
    free.dedup();
    // (splitmix64.)
    let mut state = seed;
    let mut next = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut out = kept.to_vec();
    while out.len() < count && !free.is_empty() {
        out.push(free.swap_remove((next() % free.len() as u64) as usize));
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Where frames `a..b` of a sample at `rate`, heard from `start` on, are at `now`: None
/// once they have played.
pub fn heard(start: Instant, now: Instant, (a, b): (usize, usize), rate: u32) -> Option<usize> {
    if rate == 0 {
        return None;
    }
    let at = a + (now.saturating_duration_since(start).as_secs_f64() * f64::from(rate)) as usize;
    (at < b).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_model::new_sampler::ZoneMeta;
    use std::time::Duration;

    #[test]
    fn a_caught_slice_snaps_to_an_eighth() {
        // 120 BPM at 48 kHz: an 8th is 12 000 frames.
        let m = ZoneMeta { bpm: Some(120.0), ..ZoneMeta::default() };
        let map = BeatMap::of(&m, 48_000).unwrap();
        let all = (0, 96_000);
        assert_eq!(catch_at(13_000, Some(&map), false, all), Some(12_000));
        assert_eq!(catch_at(13_000, Some(&map), true, all), Some(13_000));
        assert_eq!(catch_at(13_000, None, false, all), Some(13_000));
        assert_eq!(catch_at(13_000, None, false, (20_000, 96_000)), None, "outside what's sliced");
        assert_eq!(catch_at(96_000, None, false, all), None);
        assert_eq!(catch_at(1_000, Some(&map), false, all), None, "the first slice is there");
        assert_eq!(catch_at(0, None, true, all), None);
    }

    #[test]
    fn the_slicer_starts_at_the_playhead_or_the_selection() {
        let r = (0, 1000);
        assert_eq!(slicer_range(r, Some(300), None), (300, 1000));
        assert_eq!(slicer_range(r, Some(300), Some((200, 400))), (300, 1000));
        assert_eq!(slicer_range(r, None, Some((200, 400))), (200, 1000));
        assert_eq!(slicer_range(r, None, Some((0, 400))), r, "the range's own start");
        assert_eq!(slicer_range((100, 1000), None, Some((50, 400))), (100, 1000));
        assert_eq!(slicer_range(r, Some(5000), Some((200, 400))), (200, 1000), "a playhead elsewhere");
        assert_eq!(slicer_range(r, None, None), r);
    }

    #[test]
    fn random_picks_keep_and_fill() {
        let cands: Vec<usize> = (0..32).map(|k| k * 1000).collect();
        let a = random_picks(&cands, &[], 8, 1, 40);
        assert_eq!(a.len(), 8);
        assert_eq!(a, random_picks(&cands, &[], 8, 1, 40), "the same seed, the same");
        assert_ne!(a, random_picks(&cands, &[], 8, 2, 40));
        assert!(a.windows(2).all(|w| w[0] < w[1]) && a.iter().all(|f| cands.contains(f)));
        let kept = [5020, 9990];
        for seed in 0..20 {
            let b = random_picks(&cands, &kept, 8, seed, 40);
            assert_eq!(b.len(), 8);
            assert!(kept.iter().all(|k| b.contains(k)));
            assert!(!b.contains(&5000) && !b.contains(&10_000), "none on a kept one: {b:?}");
        }
        assert_eq!(random_picks(&cands[..3], &[], 8, 1, 40), [0, 1000, 2000]);
        assert_eq!(random_picks(&cands, &kept, 2, 1, 40), kept, "all kept: as they are");
        assert_eq!(random_picks(&[], &[], 8, 1, 40), [0usize; 0]);
    }

    #[test]
    fn the_heard_place_moves_with_the_clock() {
        let t = Instant::now();
        assert_eq!(heard(t, t, (100, 48_100), 48_000), Some(100));
        assert_eq!(heard(t, t + Duration::from_millis(500), (100, 48_100), 48_000), Some(24_100));
        assert_eq!(heard(t, t + Duration::from_secs(1), (100, 48_100), 48_000), None);
        assert_eq!(heard(t + Duration::from_secs(1), t, (100, 48_100), 48_000), Some(100));
        assert_eq!(heard(t, t, (100, 48_100), 0), None, "no sample");
    }
}
