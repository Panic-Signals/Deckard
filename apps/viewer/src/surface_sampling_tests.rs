//! The Sampling page of DECKARD's own controller layer (SAMPLE): what it shows of the
//! focused sound's sample and recorder, and what its knobs and buttons ask of them.

use deckard_surface::{Action as A, SamplingCall};

/// A new project whose focused sound plays a second of a tone (written to a temporary
/// folder), the Sampling page open.
pub(crate) fn app() -> (crate::App, tempfile::TempDir) {
    let rate = 44_100;
    app_with(deckard_engine::sample::Sample {
        rate,
        frames: (0..rate as usize).map(|i| [(i as f32 * 220.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5; 2]).collect(),
    })
}

/// [`app`], the focused sound playing `tone` (saved as Tone.wav in the folder).
pub(crate) fn app_with(tone: deckard_engine::sample::Sample) -> (crate::App, tempfile::TempDir) {
    let mut app = crate::App::default();
    app.new_project();
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("Tone.wav");
    deckard_engine::edit::write_wav(&path, &tone).unwrap();
    let (g, s) = (app.group, app.sound);
    app.load_take(g, s, &path, tone.frames.len());
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

#[test]
fn play_range_start_and_end_support_shift_on_edit_and_zone() {
    use deckard_mk3::Event;
    let (mut app, _tmp) = app();
    let now = std::time::Instant::now();
    let turn = |app: &mut crate::App, ev: Event| {
        let v = app.surface_view();
        let acts = app.surface.event(&ev, &v, now);
        for a in acts {
            app.surface_act(a);
        }
        app.update_sampling();
    };
    turn(&mut app, Event::Button("BUTTON_SAMPLE", true));
    turn(&mut app, Event::Button("BUTTON_SAMPLE", false));

    // EDIT ▸ PLAY RANGE: 0.01 turn without SHIFT moves 441 frames; with SHIFT, 44 frames.
    app.surface_act(A::SamplingTab(1));
    app.update_sampling();
    turn(&mut app, Event::Knob(0, 0.01));
    turn(&mut app, Event::Knob(1, -0.01));
    assert_eq!(zone(&app).play_range, (441, 44_099 - 441));

    turn(&mut app, Event::Button("BUTTON_SHIFT", true));
    turn(&mut app, Event::Knob(0, 0.01));
    turn(&mut app, Event::Knob(1, -0.01));
    assert_eq!(zone(&app).play_range, (441 + 44, 44_099 - 441 - 44));
    turn(&mut app, Event::Button("BUTTON_SHIFT", false));

    // ZONE ▸ PLAY RANGE: SHIFT moves START and END finely too.
    app.surface_act(A::SamplingTab(3));
    app.update_sampling();
    turn(&mut app, Event::Button("BUTTON_SHIFT", true));
    turn(&mut app, Event::Knob(0, 0.01));
    turn(&mut app, Event::Knob(1, -0.01));
    assert_eq!(zone(&app).play_range, (441 + 88, 44_099 - 441 - 88));
    turn(&mut app, Event::Button("BUTTON_SHIFT", false));
}
