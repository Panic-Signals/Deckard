mod additive_routes {
    use super::*;

    #[test]
    fn additive_draft_can_choose_a_taken_destination_without_replacing_it() {
        let mut app = app();
        let d = destinations(&app, 0, 0, 0);
        let taken = ModRoute { module: d[0].module.clone(), param: d[0].param.clone(), ..cutoff(ModSource::Pressure) };
        set_routes(&app, vec![taken.clone()]);
        app.surface_act(A::ModSlot(1, 0, 1)); // Pressure draft
        app.surface_act(A::ModSlot(1, 6, 1)); // Add before choosing Destination
        for row in 1..=3 {
            app.surface_act(A::ModSlot(1, row, 1));
        }
        app.surface_act(A::ModSlot(1, 4, 1));
        let list = routes(&app);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], taken);
        assert_eq!(list[1].param, list[0].param);
        assert_eq!(list[1].mode, RouteMode::Add);
        assert!(rows(&app, 1)[5].starts_with("Add"));
        app.surface_act(A::ModSlot(1, 5, 2));
        assert_eq!(routes(&app)[0], taken, "editing one contribution keeps the other");
        app.surface_act(A::ModSlot(1, 6, 1));
        assert_eq!(routes(&app)[1].mode, RouteMode::Add, "Range already owned");
        app.surface_act(A::ModSlot(1, 0, 0));
        assert_eq!(routes(&app), vec![taken]);
    }

    #[test]
    fn destination_filter_and_placement_only_exclude_other_range_routes() {
        let app = app();
        let dests = destinations(&app, 0, 0, 0);
        let range = ModRoute { module: dests[0].module.clone(), param: dests[0].param.clone(), ..cutoff(ModSource::Pressure) };
        let add = ModRoute { mode: RouteMode::Add, source: ModSource::Slide, ..range.clone() };
        let mut list = vec![range.clone()];
        assert_eq!(free(dests.clone(), &list, 0, None, RouteMode::Add), dests);
        assert_eq!(free(dests.clone(), &list, 0, None, RouteMode::Range).len(), dests.len() - 1);
        place(&mut list, add.clone(), None);
        assert_eq!(list, vec![range.clone(), add.clone()]);
        place(&mut list, add.clone(), Some(1));
        assert_eq!(list, vec![range.clone(), add.clone()]);
        place(&mut list, range.clone(), Some(0));
        assert_eq!(list, vec![range, add]);
    }

    #[test]
    fn additive_depth_covers_the_full_signed_range_from_any_knob_base() {
        let mut app = app();
        let r = ModRoute { mode: RouteMode::Add, low: 1.0, high: 1.0, ..cutoff(ModSource::Pressure) };
        set_routes(&app, vec![r]);
        app.surface_act(A::ModSlot(0, 5, 2));
        assert_eq!((routes(&app)[0].low, routes(&app)[0].high), (0.0, 0.1));
        app.surface_act(A::ModSlot(0, 5, -4));
        assert_eq!((routes(&app)[0].low, routes(&app)[0].high), (0.1, 0.0));
        app.surface_act(A::ModSlot(0, 5, 100));
        assert_eq!((routes(&app)[0].low, routes(&app)[0].high), (0.0, 1.0));
        app.surface_act(A::ModSlot(0, 5, -100));
        assert_eq!((routes(&app)[0].low, routes(&app)[0].high), (1.0, 0.0));
    }

    #[test]
    fn control_area_uses_final_sum_once_and_edits_the_original_base() {
        let app = app();
        let doc = app.doc.as_ref().unwrap();
        let mut p = doc.borrow().project().clone();
        let snd = &mut p.groups[0].sounds[0];
        snd.sampler.as_mut().unwrap().set("cutoff", 0.3);
        snd.mod_routes = vec![
            ModRoute { mode: RouteMode::Add, ..cutoff(ModSource::Pressure) },
            ModRoute { mode: RouteMode::Add, ..cutoff(ModSource::Slide) },
        ];
        let vals = mod_routes::routed_values(snd, |k| Some(if k == 0 { 0.7 } else { 0.1 }));
        assert_eq!(vals.len(), 1);
        let smp = mod_routes::live_sampler(snd.sampler.as_ref().unwrap(), 0, &vals);
        assert!((smp.get("cutoff") - 0.7).abs() < 1e-6);
        let bases = mod_routes::sampler_bases(snd.sampler.as_ref().unwrap(), deckard_doc::Target::Sampler(0, 0), 0, &vals);
        assert!((mod_routes::base_of(&bases, deckard_doc::Target::Sampler(0, 0), "cutoff", 0.7) - 0.3).abs() < 1e-6);
    }
}
