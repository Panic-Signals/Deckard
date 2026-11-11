use super::*;

#[test]
fn adopting_at_and_beyond_the_fixed_chain_pairing_limit_does_not_overrun_the_lcs_table() {
    for n in [31, 32, 33, 96] {
        let p = Project {
            master_slots: (0..n)
                .map(|_| deckard_model::SlotInfo { module: "Filter".into(), name: "Filter".into(), active: true, ..Default::default() })
                .collect(),
            ..Default::default()
        };
        let mut cache = SampleCache::default();
        let (mut playing, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        for u in &mut playing.master_fx.as_mut().unwrap().units {
            u.mix = 0.6;
        }
        let (fresh, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        let _old = playing.adopt(fresh, &p);
        assert!(
            playing.master_fx.as_ref().unwrap().units.iter().all(|u| u.mix == 0.6),
            "all unchanged units retain their state, including beyond slot 32"
        );
        // A changed-length chain must also safely handle the full sentinel boundary.
        let mut changed = p.clone();
        changed.master_slots.push(changed.master_slots[0].clone());
        let (fresh, _) = Engine::new(&changed, None, 48_000, Mode::Arrangement, &mut cache);
        let _old = playing.adopt(fresh, &changed);
        playing.process(&mut [0.; 512]);
        assert!(playing.master_fx.as_ref().unwrap().units.len() >= n + 1);
    }
}
