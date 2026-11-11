mod additive_routes {
    use super::*;
    use deckard_model::{
        lfo::{Lfo, Shape},
        mod_matrix::{ModParam, ModRoute, ModSource, RouteMode},
    };

    fn add(source: ModSource, depth: f32) -> ModRoute {
        ModRoute { mode: RouteMode::Add, high: depth, ..route(source, 0, "Kick", field("decay")) }
    }

    fn square() -> Lfo {
        Lfo { shape: Shape::Square, ..Lfo::new(1) }
    }

    fn engine(p: &Project, cache: &mut SampleCache) -> Engine {
        let mut e = Engine::new(p, None, 48_000, Mode::Arrangement, cache).0;
        e.sequencing = false; // use the stopped LFO clock in direct span checks
        e
    }

    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "{a} vs {b}");
    }

    #[test]
    fn lfo_swings_around_the_knob_and_turning_the_knob_moves_the_sweep() {
        let (_tmp, mut p) = lfo_project(1, vec![square()]);
        p.groups[0].sounds[1].mod_routes = vec![add(ModSource::Lfo(1), 0.4)];
        let mut cache = SampleCache::default();
        let mut e = engine(&p, &mut cache);
        e.apply_routes();
        near(decay_now(&e), 0.7);
        e.lfo_clock.ticks = 480.0;
        e.apply_routes();
        near(decay_now(&e), 0.3);
        let mut turned = p.clone();
        turned.groups[0].sounds[1].slots[0].params[0].1 = 0.6;
        let fresh = engine(&turned, &mut cache);
        drop(e.adopt(fresh, &turned));
        e.apply_routes();
        near(decay_now(&e), 0.4);
        e.lfo_clock.ticks = 0.0;
        e.apply_routes();
        near(decay_now(&e), 0.8);
        // Removal keeps the original knob, rather than the last modulated value.
        turned.groups[0].sounds[1].mod_routes.clear();
        let fresh = engine(&turned, &mut cache);
        drop(e.adopt(fresh, &turned));
        blocks(&mut e, 1);
        near(decay_now(&e), 0.6);
    }

    #[test]
    fn two_add_routes_sum_once_clamp_and_publish_only_the_first_cell() {
        let (_tmp, mut p) = lfo_project(1, vec![square()]);
        p.groups[0].sounds[1].mod_routes = vec![add(ModSource::Lfo(1), 0.4), add(ModSource::Pressure, 0.5)];
        let mut cache = SampleCache::default();
        let mut e = engine(&p, &mut cache);
        let board = Arc::new(meters::RouteTravels::default());
        e.set_route_travels(&board);
        e.touch_start(0, 1, 60, None, 0.0, None);
        e.touch_live((0, 1, 60), ExprKind::Dim(Dim::Pressure), 0.2);
        e.apply_routes();
        assert_eq!(e.mod_routes.len(), 1, "one prepared target");
        near(decay_now(&e), 0.8);
        let key = meters::route_key(&p.groups[0].sounds[1].mod_routes[0]);
        near(f64::from(board.get(0, 1, 0, key).unwrap()), 0.8);
        assert_eq!(board.get(0, 1, 1, key), None);
        e.touch_live((0, 1, 60), ExprKind::Dim(Dim::Pressure), 1.0);
        e.apply_routes();
        near(decay_now(&e), 1.0);
        e.mod_routes[0].routes[0].route.high = 2.0;
        e.lfo_clock.ticks = 480.0;
        e.touch_release(0, 1, 60);
        e.apply_routes();
        near(decay_now(&e), 0.0);
        // Removing the first source moves publication to the surviving source.
        p.groups[0].sounds[1].mod_routes.remove(0);
        drop(e.adopt(engine(&p, &mut cache), &p));
        e.apply_routes();
        near(f64::from(board.get(0, 1, 0, key).unwrap()), 0.5);
        assert_eq!(board.get(0, 1, 1, key), None);
    }

    #[test]
    fn periodic_lfo_rest_uses_level_and_offset_but_envelopes_rest_at_zero() {
        let (_tmp, mut p) = lfo_project(1, vec![Lfo { level: 0.4, offset: 0.2, ..square() }]);
        p.groups[0].sounds[1].mod_routes = vec![add(ModSource::Lfo(1), 1.0)];
        let mut cache = SampleCache::default();
        let mut e = engine(&p, &mut cache);
        e.apply_routes();
        near(decay_now(&e), 0.7);
        e.lfo_clock.ticks = 480.0;
        e.apply_routes();
        near(decay_now(&e), 0.3);
        p.lfos[0].env = true;
        p.lfos[0].trigger = Some((0, 1));
        let mut e = engine(&p, &mut cache);
        e.apply_routes();
        near(decay_now(&e), 0.5);
    }

    #[test]
    fn mixed_modes_keep_range_as_the_anchor_and_drop_non_finite_contributions() {
        let (_tmp, mut p) = lfo_project(1, vec![square()]);
        let r = ModRoute { low: 0.2, high: 0.6, ..route(ModSource::Slide, 0, "Kick", field("decay")) };
        p.groups[0].sounds[1].mod_routes = vec![r, add(ModSource::Lfo(1), 0.4), add(ModSource::Pressure, 0.3)];
        let mut cache = SampleCache::default();
        let mut e = engine(&p, &mut cache);
        e.touch_start(0, 1, 60, None, 0.0, None);
        e.touch_live((0, 1, 60), ExprKind::Dim(Dim::Pressure), f32::INFINITY);
        e.apply_routes();
        near(decay_now(&e), 0.4);
        e.mod_routes[0].routes[1].route.high = f32::NAN;
        e.apply_routes();
        near(decay_now(&e), 0.2);
        assert!(e.mod_routes[0].now.is_finite());
    }

    #[test]
    fn sampler_and_effect_add_to_their_own_stored_bases() {
        let (_tmp, mut p, base) = modulated_project();
        p.groups[0].patterns[0].mods.clear();
        let snd = &mut p.groups[0].sounds[1];
        snd.sampler.as_mut().unwrap().set("cutoff", 0.3);
        snd.mod_routes = vec![
            ModRoute { mode: RouteMode::Add, high: 0.2, ..route(ModSource::Pressure, 0, "Sampler", field("cutoff")) },
            ModRoute { mode: RouteMode::Add, high: 0.2, ..route(ModSource::Pressure, 1, "BeatDelay", field("feedback")) },
        ];
        let mut cache = SampleCache::default();
        let mut e = engine(&p, &mut cache);
        press(&mut e, 0.5, 20);
        near(f64::from(e.sounds[0][1].as_ref().unwrap().settings_cutoff()), 0.4);
        near(feedback_now(&e), (base + 0.1).clamp(0.0, 1.0));
        e.note_release(0, 1, 60);
        blocks(&mut e, 20);
        near(f64::from(e.sounds[0][1].as_ref().unwrap().settings_cutoff()), 0.3);
        near(feedback_now(&e), base);
    }

    fn plugin_project() -> (tempfile::TempDir, Project, u32) {
        let class = deckard_plugins::native::CLASSES[0];
        let id = class.params.iter().find(|x| x.steps == 0).unwrap().id;
        let pr = deckard_model::PluginRef { id: class.plugin_id(), name: class.name.into(), ..Default::default() };
        let (tmp, mut p) = eight_project();
        let snd = &mut p.groups[0].sounds[1];
        snd.sampler = None;
        snd.plugin = Some(pr.clone());
        snd.slots = vec![deckard_model::SlotInfo {
            module: "PluginHost".into(),
            name: class.name.into(),
            active: true,
            plugin: Some(pr.clone()),
            params: vec![],
        }];
        snd.mod_routes = vec![ModRoute { mode: RouteMode::Add, high: 0.4, ..route(ModSource::Lfo(1), 0, &pr.id, ModParam::Plugin(id)) }];
        p.lfos = vec![square()];
        (tmp, p, id)
    }

    #[test]
    fn plugin_base_turns_do_not_accumulate_and_busy_removal_retries() {
        let (_tmp, mut p, id) = plugin_project();
        let mut cache = SampleCache::default();
        let mut e = engine(&p, &mut cache);
        let plugin = e.instruments[0][1].as_ref().unwrap().plugin.clone();
        plugin.lock().unwrap().set_param(id, 0.3);
        for _ in 0..20 {
            e.apply_routes();
        }
        near(plugin.lock().unwrap().modulated(id).unwrap(), 0.5);
        plugin.lock().unwrap().set_param(id, 0.6);
        e.apply_routes();
        near(plugin.lock().unwrap().modulated(id).unwrap(), 0.8);
        {
            let _busy = plugin.lock().unwrap();
            e.lfo_clock.ticks = 480.0;
            e.apply_routes(); // returns without waiting
        }
        e.apply_routes();
        near(plugin.lock().unwrap().modulated(id).unwrap(), 0.4);
        let old = p.clone();
        p.groups[0].sounds[1].mod_routes.clear();
        let fresh = Engine::new_after(&p, Some((&old, &[])), None, 48_000, Mode::Arrangement, &mut cache).0;
        {
            let _busy = plugin.lock().unwrap();
            drop(e.adopt(fresh, &p));
        }
        e.retry_unroutes();
        assert_eq!(plugin.lock().unwrap().modulated(id), None);
        near(plugin.lock().unwrap().knob_value(id), 0.6);
    }

    mod allocation {
        use std::{
            alloc::{GlobalAlloc, Layout, System},
            cell::Cell,
        };
        thread_local! {
            static ACTIVE: Cell<bool> = const { Cell::new(false) };
            static COUNT: Cell<usize> = const { Cell::new(0) };
        }
        struct Tracked;
        fn count() {
            if ACTIVE.try_with(Cell::get).unwrap_or(false) {
                COUNT.with(|n| n.set(n.get() + 1));
            }
        }
        unsafe impl GlobalAlloc for Tracked {
            unsafe fn alloc(&self, l: Layout) -> *mut u8 {
                count();
                unsafe { System.alloc(l) }
            }
            unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
                count();
                unsafe { System.alloc_zeroed(l) }
            }
            unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
                unsafe { System.dealloc(p, l) }
            }
            unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
                count();
                unsafe { System.realloc(p, l, n) }
            }
        }
        #[global_allocator]
        static ALLOCATOR: Tracked = Tracked;

        pub fn measured(f: impl FnOnce()) -> usize {
            struct Guard;
            impl Drop for Guard {
                fn drop(&mut self) {
                    ACTIVE.with(|a| a.set(false));
                }
            }
            COUNT.with(|n| n.set(0));
            ACTIVE.with(|a| a.set(true));
            let guard = Guard;
            f();
            drop(guard);
            COUNT.with(Cell::get)
        }
    }

    #[test]
    fn applying_summed_routes_and_adopting_prepared_routes_does_not_allocate() {
        let (_tmp, mut p, _) = modulated_project();
        p.groups[0].patterns[0].mods.clear();
        let snd = &mut p.groups[0].sounds[1];
        snd.mod_routes =
            vec![ModRoute { mode: RouteMode::Add, high: 0.2, ..route(ModSource::Pressure, 1, "BeatDelay", field("feedback")) }];
        let mut cache = SampleCache::default();
        let mut e = engine(&p, &mut cache);
        let mut fresh = engine(&p, &mut cache);
        let board = Arc::new(meters::RouteTravels::default());
        e.set_route_travels(&board);
        let (_tmp2, q, _) = plugin_project();
        let mut pe = engine(&q, &mut cache);
        let (_tmp3, mut sp) = lfo_project(1, vec![square()]);
        sp.groups[0].sounds[1].mod_routes = vec![add(ModSource::Lfo(1), 0.4), add(ModSource::Pressure, 0.2)];
        let mut se = engine(&sp, &mut cache);
        assert_eq!(
            allocation::measured(|| {
                e.adopt_routes(&mut fresh);
                for _ in 0..1000 {
                    e.apply_routes();
                    pe.apply_routes();
                    se.apply_routes();
                    for sound in se.sounds.iter_mut().flatten().flatten() {
                        sound.finish_modulation();
                    }
                    for chain in e.fx.iter_mut().flatten().flatten() {
                        for unit in &mut chain.units {
                            if let FxUnit::Internal { hit, .. } = &mut unit.fx {
                                *hit = false;
                            }
                        }
                    }
                    pe.lfo_clock.ticks += 10.0;
                    se.lfo_clock.ticks += 10.0;
                }
            }),
            0
        );
    }
}
