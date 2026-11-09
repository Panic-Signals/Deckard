mod additive_routes {
    use super::*;

    #[test]
    fn old_text_is_range_and_add_text_round_trips_with_spaced_modules() {
        let old = "pitch 2 p:4096 0.25 0.75 70 12 Native Instruments:VST3:FX:1 2\n";
        let mut r = from_text(old).remove(0);
        assert_eq!(r.mode, RouteMode::Range);
        assert_eq!(to_text(&[r.clone()]), old);
        r.mode = RouteMode::Add;
        r.source = ModSource::Slide;
        assert_eq!(from_text(&to_text(&[r.clone()])), vec![r]);
    }

    #[test]
    fn range_matches_the_original_formula_for_every_curve_and_source() {
        for source in [ModSource::Pressure, ModSource::Slide, ModSource::Pitch, ModSource::Lfo(1)] {
            for curve in [0, 25, 50, 75, 100] {
                for x in [None, Some(-1.0), Some(0.0), Some(0.2), Some(1.0), Some(3.0)] {
                    let r =
                        ModRoute { low: 0.8, high: 0.1, curve, ..ModRoute::new(source, 0, "Sampler", ModParam::Field("cutoff".into())) };
                    let c = f64::from(curve) / 100.0;
                    let x = f64::from(x.unwrap_or(0.0));
                    let at = if source == ModSource::Pitch {
                        let t = (x / 2.0).clamp(-1.0, 1.0);
                        ((t.signum() * pad_touch::shape(t.abs(), c) + 1.0) / 2.0) as f32
                    } else {
                        pad_touch::shape(x, c) as f32
                    };
                    assert_eq!(r.travel(Some(x as f32)), 0.8 + (0.1 - 0.8) * at);
                }
            }
        }
    }

    #[test]
    fn additive_source_is_shaped_before_rest_is_subtracted() {
        let r = ModRoute {
            mode: RouteMode::Add,
            curve: 75,
            low: 0.1,
            high: 0.7,
            ..ModRoute::new(ModSource::Lfo(1), 0, "Filter", ModParam::Field("cutoff".into()))
        };
        let expected = 0.6 * (pad_touch::shape(0.2, 0.75) as f32 - 0.3);
        assert!(close(r.contribution(Some(0.2), 0.3), expected));
        let inverted = ModRoute { low: r.high, high: r.low, ..r.clone() };
        assert!(close(inverted.contribution(Some(0.2), 0.3), -expected));
        let pitch = ModRoute { source: ModSource::Pitch, ..r };
        assert!(close(pitch.contribution(None, 0.5), 0.0));
        assert!(pitch.contribution(Some(-2.0), 0.5) < 0.0);
        assert!(pitch.contribution(Some(2.0), 0.5) > 0.0);
    }

    #[test]
    fn non_finite_routes_and_sources_are_rejected() {
        let r = ModRoute::new(ModSource::Pressure, 0, "Filter", ModParam::Field("cutoff".into()));
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(!r.travel(Some(bad)).is_finite());
            assert!(!r.contribution(Some(bad), 0.0).is_finite());
            assert!(!r.contribution(Some(0.5), bad).is_finite());
            assert!(!ModRoute { high: bad, ..r.clone() }.travel(Some(0.5)).is_finite());
            assert!(!ModRoute { low: bad, ..r.clone() }.contribution(Some(0.5), 0.0).is_finite());
        }
        assert!(from_text("pressure 0 f:cutoff NaN 1 50 2 Filter mode:add\npressure 0 f:cutoff 0 inf 50 2 Filter\n").is_empty());
    }
}
