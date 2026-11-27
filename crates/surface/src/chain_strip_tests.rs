use crate::tests_support::view;
use crate::*;

/// A bypassed slot's kind icon is faded behind a bypass mark in the Control page's
/// chain strip, its name moved right to make room; the slots before it stay as they are.
#[test]
fn a_bypassed_slot_is_marked_in_the_chain_strip() {
    let (s, mut v) = (Surface::new(), view());
    let slot = |name: &str, instrument| SlotView { name: name.into(), instrument, active: true, plugin: true, picture: None };
    v.control.chain = vec![slot("Massive", true), slot("Reverb", false)];
    (v.control.slot_count, v.control.slot_index) = (2, Some(0));
    let before = s.picture(&v);
    // (Where the pictures differ: x, y.)
    let changed = |v: &View| -> Vec<(usize, usize)> {
        let after = s.picture(v);
        before.iter().zip(&after).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| (i % 960, i / 960)).collect()
    };
    for (bypassed, name) in [(0, "the instrument"), (1, "the effect")] {
        let mut v = v.clone();
        v.control.chain[bypassed].active = false;
        let diff = changed(&v);
        assert!(diff.len() > 20, "{name}: a mark ({} pixels)", diff.len());
        let x0 = diff.iter().map(|p| p.0).min().unwrap();
        let (y0, y1) = (diff.iter().map(|p| p.1).min().unwrap(), diff.iter().map(|p| p.1).max().unwrap());
        // (The strip's row on the left screen only.)
        assert!(diff.iter().all(|p| p.0 < 480) && y1 - y0 <= 20, "{name}: the strip's row, y {y0}-{y1}");
        if bypassed == 0 {
            // (The mark starts at the slot's left edge.)
            assert!((12..20).contains(&x0), "{name}: from the first slot's icon, at {x0}");
        } else {
            assert!(x0 > 40, "{name}: the first slot as it was, the second from {x0}");
        }
    }
    assert_eq!(s.error(), None);
}
