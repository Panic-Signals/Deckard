//! The Sampling page of DECKARD's own controller layer (SAMPLE): what it shows of the
//! focused sound's sample and recorder, and what its knobs and buttons ask of them.

use deckard_surface::{Action as A, SamplingCall};

/// A new project whose focused sound plays a second of a tone (written to a temporary
/// folder), the Sampling page open.
fn app() -> (crate::App, tempfile::TempDir) {
    let mut app = crate::App::default();
    app.new_project();
    let tmp = tempfile::tempdir().unwrap();
    let rate = 44_100;
    let tone = deckard_engine::sample::Sample {
        rate,
        frames: (0..rate as usize).map(|i| [(i as f32 * 220.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5; 2]).collect(),
    };
    let path = tmp.path().join("Tone.wav");
    deckard_engine::edit::write_wav(&path, &tone).unwrap();
    app.load_into_sound(&path);
    app.surface_act(A::Sampling(true));
    app.update_sampling();
    (app, tmp)
}

fn zone(app: &crate::App) -> deckard_model::Zone {
    let d = app.doc.as_ref().unwrap().borrow();
    d.project().groups[app.group].sounds[app.sound].sampler.as_ref().unwrap().zones[0].clone()
}

#[test]
fn the_sampling_page_shows_the_focused_sample() {
    let (mut app, _tmp) = app();
    let v = app.surface_view().sampling;
    let s = v.sample.expect("the tone is shown");
    assert_eq!((s.name.as_str(), s.frames, s.overview.len()), ("Tone.wav", 44_100, 480));
    assert_eq!(v.pages[1].iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["PLAY RANGE", "SELECTION RANGE"]);
    assert_eq!(v.pages[3].len(), 5, "ZONE's five pages");
    assert!(!v.pages[0].is_empty(), "RECORD's knobs");
    app.surface_act(A::Sampling(false));
    assert!(!app.sampling_open);
    assert!(app.surface_view().sampling.sample.is_none(), "closed: nothing to show");
}

#[test]
fn the_sampling_knobs_set_the_zone() {
    let (mut app, _tmp) = app();
    app.surface_act(A::SamplingTab(1));
    // EDIT ▸ PLAY RANGE ▸ END.
    app.surface_act(A::SamplingKnob { page: 0, knob: 1, value: 22_049.0 });
    assert_eq!(zone(&app).play_range.1, 22_049);
    // ZONE ▸ TUNE / MIX ▸ TUNE.
    app.surface_act(A::SamplingTab(3));
    app.surface_act(A::SamplingKnob { page: 2, knob: 0, value: 12.0 });
    assert_eq!(zone(&app).tune, 12.0);
}

#[test]
fn the_slice_tab_slices_and_applies() {
    let (mut app, _tmp) = app();
    app.surface_act(A::SamplingTab(2));
    let v = app.surface_view().sampling;
    assert_eq!(v.tab, 2);
    let split = v.slice.iter().position(|k| k.key == "Slice.Split").unwrap();
    // 16 slices (one of Split's counts).
    let at = v.slice[split].min as usize..=v.slice[split].max as usize;
    let sixteen = at.clone().find(|&i| crate::layout::sampling::SPLIT_COUNTS[i] == 16).unwrap();
    app.surface_act(A::SamplingKnob { page: 0, knob: split, value: sixteen as f64 });
    assert_eq!(app.surface_view().sampling.sample.unwrap().slices.len(), 16);
    app.surface_act(A::SamplingCall(SamplingCall::ApplySlices));
    let d = app.doc.as_ref().unwrap().borrow();
    assert_eq!(d.project().groups[app.group].sounds[app.sound].sampler.as_ref().unwrap().zones.len(), 16);
    drop(d);
}
