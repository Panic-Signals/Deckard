//! Perform FX read from Maschine's projects with their parameters' names (docs/superpowers/
//! plans/2026-10-22-phase5-perform-fx.md). Skipped where the projects aren't installed.

use deckard_model::Project;

fn open(path: &str) -> Option<Project> {
    let bytes = std::fs::read(path).ok()?;
    let doc = mxprj::Project::parse(&bytes).unwrap();
    Some(Project::from_decoded(doc.decode(&mxprj::types::Types::new()).unwrap()))
}

#[test]
fn perform_fx_parameters_have_their_names() {
    let Some(p) = open("/Users/Shared/Maschine 2 Library/Projects/Crush It.mxprj") else { return };
    let fx = p.groups.iter().flat_map(|g| g.slots.iter()).find(|s| s.module == "PerformanceFX").expect("a Perform FX");
    let names: Vec<&str> = fx.params.iter().map(|(n, _)| n.as_str()).collect();
    for n in [
        "mode",
        "touchstrip_pos",
        "touchstrip_on_off",
        "filter_type",
        "filter_out_level",
        "flanger_polarity",
        "ring_glide",
        "burst_wet_level",
        "reso_mix_mode",
        "stutter_retrigger",
        "scratcher_wet_level",
        "tremolo_rate_max",
    ] {
        assert!(names.contains(&n), "{n} in {names:?}");
    }
    assert_eq!(names.len(), 55, "every stored parameter");
    assert!(names.iter().all(|n| !n.starts_with("unknown")));
    let mode = fx.param("mode").unwrap();
    assert!((0.0..=7.0).contains(&mode));
}

#[test]
fn perform_fx_is_named_as_maschine_shows_it() {
    assert_eq!(deckard_model::module_label("PerformanceFX"), "Perform FX");
}

/// Stutter's quantized Length: 1/2 at the strip's (or knob's) start to 1/64 at its end,
/// on Maschine's curve (1/2 has the least of it: the steps change at about 0.078, 0.245,
/// 0.427, 0.630 and 0.865).
#[test]
fn stutter_lengths_run_from_a_half_to_a_sixty_fourth() {
    use deckard_model::{STUTTER_LENGTHS, stutter_length_step};
    let names: Vec<&str> = STUTTER_LENGTHS.iter().map(|x| x.0).collect();
    assert_eq!(names, ["1/2", "1/4", "1/8", "1/16", "1/32", "1/64"]);
    assert_eq!(STUTTER_LENGTHS[0].1, 2.0, "quarters");
    assert_eq!(STUTTER_LENGTHS[5].1, 1.0 / 16.0);
    for (x, step) in
        [(0.0, 0), (0.07, 0), (0.08, 1), (0.24, 1), (0.25, 2), (0.42, 2), (0.43, 3), (0.62, 3), (0.64, 4), (0.86, 4), (0.87, 5), (1.0, 5)]
    {
        assert_eq!(stutter_length_step(x), step, "at {x}");
    }
}

/// Stutter's Length as Maschine shows it: the note value with Quantize; without, the
/// loop's time at the tempo (ms, or s from a second).
#[test]
fn stutter_length_reads_as_maschine_shows_it() {
    use deckard_model::stutter_length_text;
    assert_eq!(stutter_length_text(0.0, true, 120.0), "1/2");
    assert_eq!(stutter_length_text(0.5, true, 120.0), "1/16");
    assert_eq!(stutter_length_text(1.0, true, 120.0), "1/64");
    assert_eq!(stutter_length_text(0.0, false, 120.0), "707.1 ms");
    assert_eq!(stutter_length_text(1.0, false, 120.0), "22.1 ms");
    assert_eq!(stutter_length_text(0.0, false, 60.0), "1.4 s");
}
