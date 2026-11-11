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

#[test]
fn review_second_shared_route_keeps_column_when_destination_changes() {
    use super::*;
    use deckard_model::mod_matrix::RouteMode;
    let mut app = App::default();
    let mut doc = deckard_doc::Document::new_project().unwrap();
    doc.add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
    app.doc = Some(std::rc::Rc::new(std::cell::RefCell::new(doc)));
    let a = ModRoute { mode: RouteMode::Add, ..cutoff(ModSource::Pressure) };
    let b = ModRoute { source: ModSource::Slide, ..a.clone() };
    set_routes(&app, vec![a, b]);
    let initial = app.mod_slot_views();
    assert_eq!(initial[1].rows[0], "Slide");
    step(&mut app, 1, 4, 1);
    let after = app.mod_slot_views();
    assert_eq!(after[1].rows[0], "Slide", "the edited column should still contain the same source");
    let doc = app.doc.clone().unwrap();
    assert!(doc.borrow_mut().undo());
    assert_eq!(app.mod_slot_views(), initial);
    assert!(doc.borrow_mut().redo());
    assert_eq!(app.mod_slot_views(), after);
    let pressure = routes(&app)[0].clone();
    step(&mut app, 1, 5, 2);
    assert_eq!(routes(&app)[0], pressure, "next encoder turn still edits Slide");
}

#[test]
fn additive_identical_routes_keep_the_second_draft_in_its_own_column() {
    use super::*;
    let mut app = App::default();
    let mut doc = deckard_doc::Document::new_project().unwrap();
    doc.add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
    doc.add_slot(0, 1, 0, &deckard_doc::NewModule::Sampler).unwrap();
    app.doc = Some(std::rc::Rc::new(std::cell::RefCell::new(doc)));
    let r = ModRoute { mode: RouteMode::Add, ..cutoff(ModSource::Pressure) };
    set_routes(&app, vec![r.clone(), r]);
    let _ = app.mod_slot_views();
    step(&mut app, 1, 2, 1); // move only the second identical contribution to sound 2
    let p = app.doc.as_ref().unwrap().borrow().project().clone();
    assert_eq!(&app.mod_columns(&p)[..2], &[ModSlot::Route(0, 0, 0), ModSlot::Draft]);
    step(&mut app, 1, 4, 1);
    assert_eq!(routes(&app).len(), 1);
    assert!(app.mod_slot_views()[1].rows[2].starts_with("S2"));
}

#[test]
fn additive_shared_routes_keep_order_when_an_earlier_route_is_removed_after_a_move() {
    use super::*;
    let mut app = App::default();
    let mut doc = deckard_doc::Document::new_project().unwrap();
    for s in 0..2 {
        doc.add_slot(0, s, 0, &deckard_doc::NewModule::Sampler).unwrap();
        doc.set("Filter", deckard_doc::Target::Sampler(0, s), "filter", mxprj::decode::Value::Int(1)).unwrap();
    }
    app.doc = Some(std::rc::Rc::new(std::cell::RefCell::new(doc)));
    let a = ModRoute { mode: RouteMode::Add, ..cutoff(ModSource::Pressure) };
    let b = ModRoute { source: ModSource::Slide, ..a.clone() };
    let other = ModRoute { param: ModParam::Field("drive".into()), ..cutoff(ModSource::Pitch) };
    set_routes(&app, vec![a, other]);
    app.doc.as_ref().unwrap().borrow_mut().set_mod_routes(0, 1, vec![b]).unwrap();
    let _ = app.mod_slot_views();
    step(&mut app, 0, 2, 1);
    let choice = destinations(&app, 0, 1, 0).iter().position(|d| d.param == ModParam::Field("cutoff".into())).unwrap();
    step(&mut app, 0, 4, choice as i64 + 1);
    let moved = app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[1].mod_routes[1].clone();
    assert_eq!(moved.source, ModSource::Pressure);
    app.doc.as_ref().unwrap().borrow_mut().set_mod_routes(0, 1, vec![moved]).unwrap();
    let shown = app.mod_slot_views();
    assert_eq!(shown[0].rows[0], "Pressure");
    assert_eq!(shown[1].rows[0], "Pitch", "deleting Slide must not move Pressure past the unrelated column");
}
