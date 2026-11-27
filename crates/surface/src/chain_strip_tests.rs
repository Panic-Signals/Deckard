use crate::tests_support::view;
use crate::*;

/// A bypassed slot's kind icon is crossed out in the Control page's chain strip; the
/// strip is otherwise drawn the same.
#[test]
fn a_bypassed_slot_is_crossed_out_in_the_chain_strip() {
    let (s, mut v) = (Surface::new(), view());
    let slot = |name: &str, instrument| SlotView { name: name.into(), instrument, active: true, plugin: true, picture: None };
    v.control.chain = vec![slot("Massive", true), slot("Reverb", false)];
    (v.control.slot_count, v.control.slot_index) = (2, Some(0));
    let before = s.picture(&v);
    // (Where the pictures differ, on the left screen: x, y.)
    let changed = |v: &View| -> Vec<(usize, usize)> {
        let after = s.picture(v);
        before.iter().zip(&after).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| (i % 960, i / 960)).collect()
    };
    for (bypassed, name) in [(0, "the instrument"), (1, "the effect")] {
        let mut v = v.clone();
        v.control.chain[bypassed].active = false;
        let diff = changed(&v);
        assert!(diff.len() > 20, "{name}: a cross ({} pixels)", diff.len());
        // (Over the icon only: the strip's row, no wider than the icon and a bit.)
        let (x0, x1) = (diff.iter().map(|p| p.0).min().unwrap(), diff.iter().map(|p| p.0).max().unwrap());
        let (y0, y1) = (diff.iter().map(|p| p.1).min().unwrap(), diff.iter().map(|p| p.1).max().unwrap());
        assert!(x1 < 480 && x1 - x0 <= 17 && y1 - y0 <= 17, "{name}: over its icon, {x0}-{x1} x {y0}-{y1}");
        if bypassed == 0 {
            assert!(x0 < 30, "{name}: the first slot's icon, at {x0}");
        } else {
            assert!(x0 > 40, "{name}: the second slot's icon, at {x0}");
        }
    }
    assert_eq!(s.error(), None);
}
