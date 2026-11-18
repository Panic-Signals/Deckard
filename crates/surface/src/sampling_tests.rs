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
    assert_eq!(s.leds(&v).get("LED_SAMPLE"), Some(deckard_mk3::leds::SAMPLE_DIM), "dim outside Sampling, as the other buttons");
    assert_eq!(press(&mut s, &v, "BUTTON_SAMPLE"), [Action::Sampling(true)]);
    assert_eq!(s.page(), "sampling");
    assert_eq!(s.leds(&v).get("LED_SAMPLE"), Some(deckard_mk3::leds::palette(16, 4)), "lit in Sampling");
    let text = s.screen_text(&v);
    for want in ["RECORD", "EDIT", "SLICE", "ZONE", "TRUNCATE", "APPLY", "PLAY RANGE", "1/2", "Tone.wav", "LENGTH"] {
        assert!(text.iter().any(|t| t == want), "{want} in {text:?}");
    }
    assert_eq!(press(&mut s, &v, "BUTTON_SAMPLE"), [Action::Sampling(false)]);
    assert_ne!(s.page(), "sampling");
    assert_eq!(s.leds(&v).get("LED_SAMPLE"), Some(deckard_mk3::leds::SAMPLE_DIM), "dim again when closed");
}

/// SAMPLING lights red while it records, on its page or not.
#[test]
fn sample_lights_red_while_recording() {
    let (mut s, mut v) = (Surface::new(), sampled(0));
    let red = crate::leds::bright([255, 0, 0]);
    assert_ne!(s.leds(&v).get("LED_SAMPLE"), Some(red));
    v.sampling.record.recording = true;
    assert_eq!(s.leds(&v).get("LED_SAMPLE"), Some(red), "recording, elsewhere");
    press(&mut s, &v, "BUTTON_SAMPLE");
    assert_eq!(s.page(), "sampling");
    assert_eq!(s.leds(&v).get("LED_SAMPLE"), Some(red), "recording, on the page");
    v.sampling.record.recording = false;
    assert_eq!(s.leds(&v).get("LED_SAMPLE"), Some(deckard_mk3::leds::palette(16, 4)), "white again");
}

/// RECORD: TAKES (button 7, next to DELETE) puts the takes on the pads (Maschine's), a
/// pad plays its take and they light, the focused brightest; the takes show under the
/// meters as mini waveforms.
#[test]
fn takes_go_on_the_pads_and_show_under_the_meters() {
    let (mut s, mut v) = (Surface::new(), sampled(0));
    let now = Instant::now();
    (v.sampling.takes, v.sampling.take) = (3, Some(2));
    press(&mut s, &v, "BUTTON_SAMPLE");
    let before = s.picture(&v);
    v.sampling.take_waves = vec![std::sync::Arc::new(vec![(-0.8, 0.8); 64]); 3];
    let after = s.picture(&v);
    let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
    assert!(changed > 200, "the takes' waves drawn ({changed} pixels)");
    assert!(s.event(&Event::PadOn(1, 0.8), &v, now).is_empty(), "RECORD's pads: nothing yet");
    // TAKES, next to DELETE, no SHIFT needed; lit while on.
    let text = s.screen_text(&v);
    let at = |t: &str| text.iter().position(|x| x == t).unwrap_or_else(|| panic!("{t}: {text:?}"));
    assert_eq!(at("TAKES"), at("DELETE") + 1, "{text:?}");
    let dim = s.leds(&v).get("LED_SCREEN_BUTTON_6");
    assert_eq!(s.leds(&v).get("LED_SCREEN_BUTTON_7"), dim, "TAKES off: dim, as DELETE");
    assert!(press(&mut s, &v, "BUTTON_SCREEN_7").is_empty());
    let leds = s.leds(&v);
    assert_ne!(leds.get("LED_SCREEN_BUTTON_7"), dim, "TAKES on: lit");
    assert_eq!(leds.get("LED_PAD_3"), Some(deckard_mk3::leds::palette(16, 4)), "the focused take bright");
    assert_eq!(leds.get("LED_PAD_1"), Some(deckard_mk3::leds::palette(16, 2)), "a take, dim");
    assert_eq!(leds.get("LED_PAD_4"), Some(0), "no take");
    assert_eq!(s.event(&Event::PadOn(1, 0.8), &v, now), [Action::Take { index: 0, delete: false }]);
    assert!(s.event(&Event::PadOn(4, 0.8), &v, now).is_empty(), "no fourth take");
}

/// An Audio plug-in's take: RECORD and EDIT; SLICE and ZONE aren't there.
#[test]
fn a_take_has_record_and_edit_only() {
    let (mut s, mut v) = (Surface::new(), sampled(1));
    v.sampling.sample.as_mut().unwrap().take = true;
    press(&mut s, &v, "BUTTON_SAMPLE");
    assert!(press(&mut s, &v, "BUTTON_SCREEN_3").is_empty(), "no SLICE");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_1"), [Action::SamplingTab(0)]);
    let text = s.screen_text(&v);
    assert!(!text.iter().any(|t| t == "SLICE" || t == "ZONE"), "{text:?}");
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
    // Back round past TRUNCATE to the last: TO NEW SAMPLER, and AUTOLOOP before it.
    press(&mut s, &v, "BUTTON_SCREEN_5");
    press(&mut s, &v, "BUTTON_SCREEN_5");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::SamplingCall(SamplingCall::Convert)]);
    press(&mut s, &v, "BUTTON_SCREEN_5");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::SamplingCall(SamplingCall::AutoLoop)]);
}

#[test]
fn play_range_start_and_end_turn_finely_with_shift() {
    for tab in [1, 3] {
        let (mut s, v) = (Surface::new(), sampled(tab));
        let now = Instant::now();
        press(&mut s, &v, "BUTTON_SAMPLE");
        s.event(&Event::Button("BUTTON_SHIFT", true), &v, now);
        // Without SHIFT, 0.02 of a turn moves 882 frames (2 * 441); with SHIFT, a tenth (88 frames)
        // and even a single encoder tick (0.001) moves 4 frames immediately.
        assert_eq!(
            s.event(&Event::Knob(0, 0.001), &v, now),
            [Action::SamplingKnob { page: 0, knob: 0, value: 4.0 }],
            "tab {tab} START tick"
        );
        assert_eq!(s.event(&Event::Knob(0, 0.02), &v, now), [Action::SamplingKnob { page: 0, knob: 0, value: 92.0 }], "tab {tab} START");
        assert_eq!(
            s.event(&Event::Knob(1, -0.02), &v, now),
            [Action::SamplingKnob { page: 0, knob: 1, value: 44_099.0 - 88.0 }],
            "tab {tab} END",
        );
        s.event(&Event::Button("BUTTON_SHIFT", false), &v, now);
    }
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

/// A New Sampler zone's EDIT tab: its TEMPO / PITCH page, and under SHIFT the
/// display buttons double or halve its tempo or give it to the song.
#[test]
fn a_new_sampler_zone_has_its_tempo_on_edit() {
    let (mut s, mut v) = (Surface::new(), sampled(1));
    let k = |key: &str, name: &str, text: &str, value: f64| SamplingKnob {
        key: key.into(),
        name: name.into(),
        text: text.into(),
        value,
        min: 0.0,
        max: 999.0,
        step: 1.0,
        turn: 0.01,
    };
    v.sampling.pages[1].push(KnobPage {
        name: "TEMPO / PITCH".into(),
        knobs: vec![k("Beat.Mode", "MODE", "Sync", 1.0), k("Beat.Bpm", "BPM", "120.00", 120.0), k("Beat.Root", "ROOT", "A1", 45.0)],
    });
    press(&mut s, &v, "BUTTON_SAMPLE");
    press(&mut s, &v, "BUTTON_RIGHT");
    press(&mut s, &v, "BUTTON_RIGHT");
    assert!(s.screen_text(&v).iter().any(|t| t == "TEMPO / PITCH"));
    assert_eq!(s.event(&Event::Knob(1, 0.01), &v, Instant::now()), [Action::SamplingKnob { page: 2, knob: 1, value: 121.0 }]);
    s.event(&Event::Button("BUTTON_SHIFT", true), &v, Instant::now());
    assert!(s.screen_text(&v).iter().any(|t| t == "TO PROJECT"));
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::SamplingCall(SamplingCall::ScaleBpm(true))]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::SamplingCall(SamplingCall::ScaleBpm(false))]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_7"), [Action::SamplingCall(SamplingCall::ToProject)]);
    s.event(&Event::Button("BUTTON_SHIFT", false), &v, Instant::now());
    // Without SHIFT, the edit functions as ever.
    press(&mut s, &v, "BUTTON_SCREEN_6");
    assert!(s.screen_text(&v).iter().any(|t| t == "NORMALIZE"));
    // Back past the first: the New Sampler's own, last. TO SAMPLER, then the warp ones.
    press(&mut s, &v, "BUTTON_SCREEN_5");
    press(&mut s, &v, "BUTTON_SCREEN_5");
    assert!(s.screen_text(&v).iter().any(|t| t == "TO SAMPLER"));
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::SamplingCall(SamplingCall::Convert)]);
    press(&mut s, &v, "BUTTON_SCREEN_5");
    assert!(s.screen_text(&v).iter().any(|t| t == "CLEAR WARP"));
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::SamplingCall(SamplingCall::Warp(2))]);
    // A plain Sampler's zone: no warp functions, and it converts the other way.
    v.sampling.pages[1].pop();
    assert!(s.screen_text(&v).iter().any(|t| t == "TO NEW SAMPLER"));
}

/// A zone with stem lanes: on EDIT's STEMS knob page the knobs are the lanes' levels,
/// display buttons 5-8 mute them and, with SHIFT, solo them.
#[test]
fn stem_lanes_have_their_mutes_on_the_display_buttons() {
    use deckard_model::sampling_view::LaneView;
    let (mut s, mut v) = (Surface::new(), sampled(1));
    let lane =
        |lane: usize, name: &str| LaneView { lane, name: name.into(), heard: true, detail: vec![(-0.2, 0.2); 480], ..Default::default() };
    v.sampling.sample.as_mut().unwrap().lanes = vec![lane(0, "DRUMS"), lane(1, "VOCALS"), lane(3, "LEADS")];
    v.sampling.pages[1].push(KnobPage {
        name: "STEMS".into(),
        knobs: vec![SamplingKnob {
            key: "Stem.Gain0".into(),
            name: "DRUMS".into(),
            text: "+0.0 dB".into(),
            value: 0.0,
            min: -60.0,
            max: 6.0,
            step: 0.5,
            turn: 0.01,
        }],
    });
    press(&mut s, &v, "BUTTON_SAMPLE");
    // Off the STEMS page the buttons are the edit functions'.
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::SamplingCall(SamplingCall::Edit(1))], "APPLY (TRUNCATE)");
    press(&mut s, &v, "BUTTON_RIGHT");
    press(&mut s, &v, "BUTTON_RIGHT");
    let text = s.screen_text(&v);
    assert!(text.iter().any(|t| t == "STEMS") && text.iter().any(|t| t == "LEADS"), "{text:?}");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::SamplingCall(SamplingCall::LaneMute(0))]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_7"), [Action::SamplingCall(SamplingCall::LaneMute(3))], "the third shown is Leads");
    assert!(press(&mut s, &v, "BUTTON_SCREEN_8").is_empty(), "no fourth lane");
    s.event(&Event::Button("BUTTON_SHIFT", true), &v, Instant::now());
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::SamplingCall(SamplingCall::LaneSolo(1))]);
    s.event(&Event::Button("BUTTON_SHIFT", false), &v, Instant::now());
    assert_eq!(s.event(&Event::Knob(0, -0.01), &v, Instant::now()), [Action::SamplingKnob { page: 2, knob: 0, value: -0.5 }]);
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
    // SHIFT + 7: PREVIOUS (7 itself is TAKES).
    s.event(&Event::Button("BUTTON_SHIFT", true), &v, Instant::now());
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_7"), [Action::Take { index: 0, delete: false }], "PREVIOUS");
    s.event(&Event::Button("BUTTON_SHIFT", false), &v, Instant::now());
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
fn the_slice_pads_catch_keep_and_key_shift() {
    let (mut s, mut v) = (Surface::new(), sampled(2));
    let now = Instant::now();
    let call = |c| vec![Action::SamplingCall(c)];
    let mode = |v: &mut View, m: &str| v.sampling.slice[0].text = m.into();
    press(&mut s, &v, "BUTTON_SAMPLE");
    // Manual, the sample playing: an empty pad drops a slice there; one with a slice plays it.
    mode(&mut v, "Manual");
    assert!(s.event(&Event::PadOn(9, 0.8), &v, now).is_empty(), "nothing plays: nothing caught");
    v.sampling.sample.as_mut().unwrap().playhead = Some(5000);
    assert_eq!(s.event(&Event::PadOn(9, 0.8), &v, now), call(SamplingCall::CatchSlice(false)));
    assert_eq!(s.event(&Event::PadOn(2, 0.8), &v, now), call(SamplingCall::Prehear(11_025, 22_050)));
    s.event(&Event::Button("BUTTON_SHIFT", true), &v, now);
    assert_eq!(s.event(&Event::PadOn(9, 0.8), &v, now), call(SamplingCall::CatchSlice(true)), "SHIFT: exactly there");
    assert_eq!(s.event(&Event::PadOn(2, 0.8), &v, now), call(SamplingCall::Prehear(11_025, 22_050)), "Manual keeps none");
    // SHIFT + display 5: the focused slice key shifted. No RE-ROLL outside Random.
    assert!(s.screen_text(&v).iter().any(|t| t == "KEY SHIFT"));
    assert!(!s.screen_text(&v).iter().any(|t| t == "RE-ROLL"));
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), call(SamplingCall::KeyShift(1)));
    assert!(press(&mut s, &v, "BUTTON_SCREEN_6").is_empty());
    // Find and Random: SHIFT + a slice's pad keeps it; an empty pad does nothing.
    for m in ["Find", "Random"] {
        mode(&mut v, m);
        assert_eq!(s.event(&Event::PadOn(3, 0.8), &v, now), call(SamplingCall::KeepSlice(2)), "{m}");
        assert!(s.event(&Event::PadOn(9, 0.8), &v, now).is_empty(), "{m}");
    }
    assert!(s.screen_text(&v).iter().any(|t| t == "RE-ROLL"));
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), call(SamplingCall::Reroll));
    s.event(&Event::Button("BUTTON_SHIFT", false), &v, now);
    assert!(s.event(&Event::PadOn(9, 0.8), &v, now).is_empty(), "Random: no catching");
    // A kept slice's pad is lit as the focused one is.
    let lit = |s: &mut Surface, v: &View, pad: &str| s.leds(v).get(pad);
    let dim = lit(&mut s, &v, "LED_PAD_4");
    v.sampling.sample.as_mut().unwrap().kept = vec![3];
    assert_ne!(lit(&mut s, &v, "LED_PAD_4"), dim, "kept: bright");
    // Split: SHIFT + a pad neither keeps nor undoes.
    mode(&mut v, "Split");
    s.event(&Event::Button("BUTTON_SHIFT", true), &v, now);
    assert_eq!(s.event(&Event::PadOn(2, 0.8), &v, now), call(SamplingCall::Prehear(11_025, 22_050)));
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

/// With no sample to work on, EDIT, SLICE and ZONE say why (C64): what the sound
/// plays instead. RECORD has nothing to say: it makes one.
#[test]
fn a_sound_with_no_sample_says_so() {
    let mut s = Surface::new();
    let without = |tab: usize| {
        let mut v = sampled(tab);
        v.sampling.sample = None;
        v.sampling.no_sample = Some(NoSample { title: "NOT A SAMPLE".into(), detail: "Massive X is an instrument".into() });
        v
    };
    press(&mut s, &without(1), "BUTTON_SAMPLE");
    for tab in 1..=3 {
        let text = s.screen_text(&without(tab));
        for want in ["NOT A SAMPLE", "Massive X is an instrument"] {
            assert!(text.iter().any(|t| t == want), "tab {tab}: {want} in {text:?}");
        }
    }
    let text = s.screen_text(&without(0));
    assert!(!text.iter().any(|t| t == "NOT A SAMPLE"), "{text:?}");
}
