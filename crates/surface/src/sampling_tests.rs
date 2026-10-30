//! The Sampling page (SAMPLE, `pages/sampling.lua`; docs/sampling-controller.md).

use super::*;
use crate::tests_support::view;

fn press(s: &mut Surface, v: &View, name: &'static str) -> Vec<Action> {
    let now = Instant::now();
    let mut out = s.event(&Event::Button(name, true), v, now);
    out.extend(s.event(&Event::Button(name, false), v, now));
    out
}

/// A second-long sample on tab `tab`, sliced in four, with EDIT's and ZONE's pages.
fn sampled(tab: usize) -> View {
    let mut v = view();
    let frames = 44_100;
    let knob = |key: &str, name: &str, text: &str, value: f64, max: f64| SamplingKnob {
        key: key.into(),
        name: name.into(),
        text: text.into(),
        value,
        min: 0.0,
        max,
        step: 441.0,
        turn: 0.01,
    };
    let range = |name: &str, key: &str| KnobPage {
        name: name.into(),
        knobs: vec![
            knob(&format!("{key}.Start"), "START", "0.000 s", 0.0, 44_099.0),
            knob(&format!("{key}.End"), "END", "1.000 s", 44_099.0, 44_099.0),
        ],
    };
    v.sampling = SamplingView {
        tab,
        pages: [
            vec![KnobPage { name: "RECORDING".into(), knobs: vec![knob("Rec.Mode", "MODE", "Detect", 0.0, 3.0)] }],
            vec![range("PLAY RANGE", "Edit"), range("SELECTION RANGE", "Sel")],
            Vec::new(),
            vec![range("PLAY RANGE", "Zone")],
        ],
        // (Choices: a step each.)
        slice: vec![knob("Slice.Mode", "MODE", "Split", 1.0, 3.0), knob("Slice.Split", "SLICES", "4", 0.0, 3.0)]
            .into_iter()
            .map(|k| SamplingKnob { step: 1.0, ..k })
            .collect(),
        sample: Some(SampleView {
            name: "Tone.wav".into(),
            frames,
            rate: 44_100,
            overview: vec![(-0.5, 0.5); 480],
            detail: vec![(-0.5, 0.5); 480],
            view: (0.0, frames as f64),
            play: (0, frames),
            slices: vec![0, 11_025, 22_050, 33_075],
            zones: vec![((0, 127), (0, 127))],
            ..Default::default()
        }),
        ..Default::default()
    };
    v
}

#[test]
fn sample_shows_the_sampling_page_and_again_goes_back() {
    let (mut s, v) = (Surface::new(), sampled(1));
    assert_eq!(press(&mut s, &v, "BUTTON_SAMPLE"), [Action::Sampling(true)]);
    assert_eq!(s.page(), "sampling");
    let text = s.screen_text(&v);
    for want in ["RECORD", "EDIT", "SLICE", "ZONE", "TRUNCATE", "APPLY", "PLAY RANGE", "1/2", "Tone.wav", "LENGTH"] {
        assert!(text.iter().any(|t| t == want), "{want} in {text:?}");
    }
    assert_eq!(press(&mut s, &v, "BUTTON_SAMPLE"), [Action::Sampling(false)]);
    assert_ne!(s.page(), "sampling");
}

#[test]
fn the_display_buttons_pick_the_tab() {
    let (mut s, v) = (Surface::new(), sampled(1));
    press(&mut s, &v, "BUTTON_SAMPLE");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_3"), [Action::SamplingTab(2)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_1"), [Action::SamplingTab(0)]);
}

#[test]
fn the_edit_tab_turns_its_pages_and_applies_its_functions() {
    let (mut s, v) = (Surface::new(), sampled(1));
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_SAMPLE");
    // END: a step back for each hundredth of a turn.
    assert_eq!(s.event(&Event::Knob(1, -0.02), &v, now), [Action::SamplingKnob { page: 0, knob: 1, value: 44_099.0 - 882.0 }]);
    press(&mut s, &v, "BUTTON_RIGHT");
    assert_eq!(s.event(&Event::Knob(0, 0.01), &v, now), [Action::SamplingKnob { page: 1, knob: 0, value: 441.0 }], "SELECTION RANGE");
    // ◀ TRUNCATE ▶: NORMALIZE, applied.
    press(&mut s, &v, "BUTTON_SCREEN_6");
    assert!(s.screen_text(&v).iter().any(|t| t == "NORMALIZE"));
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::SamplingCall(SamplingCall::Edit(2))]);
    // Back round past TRUNCATE to the last: AUTOLOOP.
    press(&mut s, &v, "BUTTON_SCREEN_5");
    press(&mut s, &v, "BUTTON_SCREEN_5");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::SamplingCall(SamplingCall::AutoLoop)]);
}

#[test]
fn knobs_5_and_6_zoom_and_scroll_the_sample() {
    let (mut s, mut v) = (Surface::new(), sampled(1));
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_SAMPLE");
    let [Action::SamplingCall(SamplingCall::View(a, b))] = s.event(&Event::Knob(4, 0.05), &v, now)[..] else { panic!("zoomed") };
    assert!(b - a < 44_100.0 && a > 0.0, "in, about the middle: {a}..{b}");
    v.sampling.sample.as_mut().unwrap().view = (a, b);
    let [Action::SamplingCall(SamplingCall::View(c, d))] = s.event(&Event::Knob(5, 0.05), &v, now)[..] else { panic!("scrolled") };
    assert!(c > a && ((d - c) - (b - a)).abs() < 1.0, "on, as wide: {c}..{d}");
    // Touched, their icons show.
    assert!(s.event(&Event::Button("CAP_5", true), &v, now).is_empty());
}

#[test]
fn record_starts_stops_and_cancels() {
    let (mut s, mut v) = (Surface::new(), sampled(0));
    press(&mut s, &v, "BUTTON_SAMPLE");
    assert!(s.screen_text(&v).iter().any(|t| t == "START"));
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::RecordSample(RecordPress::Toggle)]);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_6").is_empty(), "DELETE: no take");
    v.sampling.record.recording = true;
    assert!(s.screen_text(&v).iter().any(|t| t == "STOP"));
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::RecordSample(RecordPress::Cancel)]);
    v.sampling.record.recording = false;
    (v.sampling.takes, v.sampling.take) = (2, Some(1));
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_7"), [Action::Take { index: 0, delete: false }], "PREVIOUS");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::Take { index: 1, delete: true }], "DELETE");
}

#[test]
fn the_slice_pads_pick_slices_and_the_buttons_edit_them() {
    let (mut s, v) = (Surface::new(), sampled(2));
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_SAMPLE");
    assert_eq!(s.event(&Event::PadOn(2, 0.8), &v, now), [Action::SamplingCall(SamplingCall::Prehear(11_025, 22_050))]);
    assert!(s.event(&Event::PadOn(9, 0.8), &v, now).is_empty(), "no ninth slice");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::SamplingCall(SamplingCall::SplitSlice(1))]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::SamplingCall(SamplingCall::RemoveSlice(1))]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_7"), [Action::SamplingCall(SamplingCall::ClearSlices)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::SamplingCall(SamplingCall::ApplySlices)]);
    // The Slicer page: Split's SLICES on knob 2.
    assert_eq!(s.event(&Event::Knob(1, 0.01), &v, now), [Action::SamplingKnob { page: 0, knob: 1, value: 1.0 }]);
}

#[test]
fn the_zone_buttons_go_through_the_zones() {
    let (mut s, mut v) = (Surface::new(), sampled(3));
    press(&mut s, &v, "BUTTON_SAMPLE");
    assert!(press(&mut s, &v, "BUTTON_SCREEN_8").is_empty(), "NEXT: one zone");
    v.sampling.sample.as_mut().unwrap().zones.push(((60, 127), (0, 127)));
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::SamplingCall(SamplingCall::FocusZone(1))]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::SamplingCall(SamplingCall::RemoveZone(0))]);
}

/// C49: armed and waiting (Detect), WAITING flashes, with no error (the scripts have no
/// `os` library: the time is `deckard.now()`).
#[test]
fn waiting_to_record_flashes_start() {
    let (mut s, mut v) = (Surface::new(), sampled(0));
    press(&mut s, &v, "BUTTON_SAMPLE");
    v.sampling.record.waiting = true;
    let lit: std::collections::HashSet<u8> = (0..40)
        .map(|i| {
            s.tick(&v, Instant::now() + std::time::Duration::from_millis(i * 25));
            s.leds(&v).get("LED_SCREEN_BUTTON_5").unwrap()
        })
        .collect();
    assert_eq!(s.error(), None);
    assert!(lit.len() > 1, "flashing: {lit:?}");
    assert!(s.screen_text(&v).iter().any(|t| t == "WAITING"));
}
