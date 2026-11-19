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

/// The next number after `state` (splitmix64).
fn mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Which slice each of `n` notes plays, the notes in time order: its own, or (`shuffled`)
/// another's, by `seed`. Shuffled, two or more are never left as they were.
pub fn note_order(n: usize, shuffled: bool, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    if !shuffled || n < 2 {
        return order;
    }
    let mut state = seed;
    for i in (1..n).rev() {
        order.swap(i, (mix(&mut state) % (i as u64 + 1)) as usize);
    }
    if order.iter().enumerate().all(|(i, k)| i == *k) {
        order.rotate_left(1);
    }
    order
}

/// The files `n` exported slices of a sample named `stem` go to: "<stem> 01.wav" on, with
/// what a file name can't hold replaced.
pub fn slice_file_names(stem: &str, n: usize) -> Vec<String> {
    let clean: String = stem.trim().chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let stem = if clean.is_empty() { "Slice" } else { clean.as_str() };
    let digits = n.to_string().len().max(2);
    (1..=n).map(|k| format!("{stem} {k:0digits$}.wav")).collect()
}

/// Random's slices: the `kept` ones and, of `cands`, as many more as make `count`, none
/// within `near` frames of a kept one. The same `seed` picks the same.
pub fn random_picks(cands: &[usize], kept: &[usize], count: usize, seed: u64, near: usize) -> Vec<usize> {
    let mut free: Vec<usize> = cands.iter().copied().filter(|c| kept.iter().all(|k| k.abs_diff(*c) > near)).collect();
    free.sort_unstable();
    free.dedup();
    let mut state = seed;
    let mut next = || mix(&mut state);
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
    fn notes_play_their_own_slice_or_another() {
        assert_eq!(note_order(4, false, 9), [0, 1, 2, 3]);
        for seed in 0..50 {
            let o = note_order(8, true, seed);
            let mut sorted = o.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, (0..8).collect::<Vec<_>>(), "every slice once");
            assert_ne!(o, sorted, "not as they were");
            assert_eq!(o, note_order(8, true, seed));
            assert_eq!(note_order(2, true, seed), [1, 0]);
        }
        assert_ne!(note_order(8, true, 1), note_order(8, true, 2));
        assert_eq!(note_order(1, true, 3), [0]);
        assert_eq!(note_order(0, true, 3), [0usize; 0]);
    }

    #[test]
    fn exported_slices_are_numbered() {
        assert_eq!(slice_file_names("Break", 3), ["Break 01.wav", "Break 02.wav", "Break 03.wav"]);
        assert_eq!(slice_file_names("Break", 120)[4], "Break 005.wav");
        assert_eq!(slice_file_names("a/b:c", 1), ["a_b_c 01.wav"]);
        assert_eq!(slice_file_names("  ", 1), ["Slice 01.wav"]);
        assert!(slice_file_names("x", 0).is_empty());
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
