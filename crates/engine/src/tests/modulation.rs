use super::*;

/// A pattern's points become the parameter's stored values along the timeline:
/// the knob's value moved by each deviation, clamped to the range.
#[test]
fn modulation_points_are_the_parameters_values_on_the_timeline() {
    let (_tmp, p, base) = modulated_project();
    let t = timeline(&p, Mode::Arrangement);
    let lane = t.mods.iter().find(|l| l.group == 0 && l.sound == Some(1)).expect("a lane");
    assert_eq!((lane.slot, lane.field.as_str()), (1, "feedback"));
    assert_eq!(lane.from, 0);
    assert!((lane.points[0].1 - (base - 0.5).clamp(0.0, 1.0)).abs() < 1e-9);
    assert!((lane.points[1].1 - (base + 0.5).clamp(0.0, 1.0)).abs() < 1e-9);
    assert!((lane.value_at(lane.points[0].0 as f64) - lane.points[0].1).abs() < 1e-9);
    let mid = (lane.points[0].0 + lane.points[1].0) as f64 / 2.0;
    assert!((lane.value_at(mid) - (lane.points[0].1 + lane.points[1].1) / 2.0).abs() < 1e-9, "joined by a line");
    assert!((lane.value_at(lane.to as f64 - 1.0) - lane.points[1].1).abs() < 1e-9, "the last holds");
}

/// While the pattern plays the effect follows the points; stopped, it has the
/// knob's own value again. A modulated sound plays on the audio thread.
#[test]
fn modulation_moves_an_effects_parameter_while_playing() {
    let (_tmp, p, base) = modulated_project();
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    assert!(e.live_hint[0][1], "rendered live");
    e.looping = true;
    let mut buf = vec![0f32; 256 * 2];
    for _ in 0..20 {
        e.process(&mut buf);
    }
    // On the line between the points (within the 30 ms glide behind it).
    let lane = e.timeline.mods.iter().find(|l| l.sound == Some(1)).unwrap().clone();
    let want = lane.value_at(e.frame as f64 / e.samples_per_tick());
    assert!((feedback_now(&e) - want).abs() < 0.03, "{} vs {want}", feedback_now(&e));
    assert!(feedback_now(&e) < base - 0.3, "well under the knob's {base}");
    e.sequencing = false;
    for _ in 0..20 {
        e.process(&mut buf);
    }
    assert!((feedback_now(&e) - base).abs() < 1e-6, "back to {base}: {}", feedback_now(&e));
}

/// Pressure moves an effect's parameter from Low to High; let go, it rests at Low;
/// the route removed, it has the knob's value again.
#[test]
fn pressure_moves_an_effects_parameter() {
    use deckard_model::mod_matrix::ModSource;
    let (_tmp, mut p, base) = modulated_project();
    p.groups[0].patterns[0].mods.clear();
    let mut r = route(ModSource::Pressure, 1, &p.groups[0].sounds[1].slots[1].module.clone(), field("feedback"));
    r.low = 0.1;
    p.groups[0].sounds[1].mod_routes = vec![r];
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    press(&mut e, 1.0, 20);
    assert!((feedback_now(&e) - 1.0).abs() < 1e-3, "{}", feedback_now(&e));
    e.note_release(0, 1, 60);
    blocks(&mut e, 20);
    assert!((feedback_now(&e) - 0.1).abs() < 1e-3, "{}", feedback_now(&e));
    let mut plain = p.clone();
    plain.groups[0].sounds[1].mod_routes.clear();
    let (fresh, _) = Engine::new_after(&plain, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &plain));
    blocks(&mut e, 20);
    assert!((feedback_now(&e) - base).abs() < 1e-6, "back to {base}: {}", feedback_now(&e));
}

/// Pressure moves the Sampler's cutoff, while notes play; the route gone, the
/// cutoff is the knob's again.
#[test]
fn pressure_moves_the_samplers_cutoff() {
    use deckard_model::mod_matrix::ModSource;
    let (_tmp, mut p) = eight_project();
    assert_eq!(p.groups[0].sounds[1].slots[0].module, "Sampler");
    let knob = p.groups[0].sounds[1].sampler.as_ref().unwrap().get("cutoff");
    let mut r = route(ModSource::Pressure, 0, "Sampler", field("cutoff"));
    (r.low, r.high) = (0.2, 0.8);
    p.groups[0].sounds[1].mod_routes = vec![r];
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    press(&mut e, 0.5, 2);
    let cutoff = |e: &Engine| e.sounds[0][1].as_ref().unwrap().settings_cutoff();
    assert!((cutoff(&e) - 0.5).abs() < 1e-4, "{}", cutoff(&e));
    let mut plain = p.clone();
    plain.groups[0].sounds[1].mod_routes.clear();
    let (fresh, _) = Engine::new_after(&plain, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &plain));
    blocks(&mut e, 1);
    assert!((f64::from(cutoff(&e)) - knob).abs() < 1e-4, "{} vs {knob}", cutoff(&e));
}

/// Pressure moves a drum synth's parameter.
#[test]
fn pressure_moves_a_drum_synths_parameter() {
    use deckard_model::mod_matrix::ModSource;
    let (_tmp, mut p) = eight_project();
    let snd = &mut p.groups[0].sounds[1];
    snd.sampler = None;
    snd.slots = vec![deckard_model::SlotInfo {
        module: "Kick".into(),
        name: "Kick".into(),
        active: true,
        plugin: None,
        params: vec![("decay".into(), 0.5)],
    }];
    snd.mod_routes = vec![route(ModSource::Pressure, 0, "Kick", field("decay"))];
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    press(&mut e, 0.25, 2);
    let v = e.sounds[0][1].as_ref().unwrap().synth_param("decay").unwrap();
    assert!((v - 0.25).abs() < 1e-6, "{v}");
}

/// A Square LFO at 1/4: the decay is at High for the first half beat, at Low for
/// the second.
#[test]
fn an_lfo_moves_a_routed_parameter() {
    use deckard_model::lfo::{Lfo, Shape};
    let mut l = Lfo::new(1);
    l.shape = Shape::Square;
    let (_tmp, p) = lfo_project(1, vec![l]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    blocks(&mut e, 1);
    assert!((decay_now(&e) - 1.0).abs() < 1e-6, "{}", decay_now(&e));
    let n = (e.samples_per_tick() * 480.0 / 256.0).ceil() as usize;
    blocks(&mut e, n);
    assert!((480.0..960.0).contains(&e.position()), "{}", e.position());
    assert!(decay_now(&e).abs() < 1e-6, "{}", decay_now(&e));
}

/// A Wander LFO moves a routed parameter along its four-sine sum, doesn't repeat on
/// the next beat, and restarts from cycle 0 when triggered by a note.
#[test]
fn a_wander_lfo_moves_a_routed_parameter_and_retriggers() {
    use deckard_model::lfo::{Lfo, Shape};
    let l = Lfo { shape: Shape::Wander, ..Lfo::new(1) };
    let (_tmp, p) = lfo_project(1, vec![l.clone()]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    blocks(&mut e, 1);
    let at_start = decay_now(&e);
    assert!((at_start - f64::from(l.wave_at(0.0))).abs() < 1e-5, "{at_start}");
    let one_beat = (e.samples_per_tick() * 960.0 / 256.0).round() as usize;
    blocks(&mut e, one_beat - 1);
    let c1 = e.lfo_cycles(&e.lfos[0]);
    blocks(&mut e, 1);
    assert!((decay_now(&e) - f64::from(l.wave_at(c1))).abs() < 1e-5);
    assert!((decay_now(&e) - at_start).abs() > 1e-2, "a cycle later is a new point on the curve");

    let trig = Lfo { shape: Shape::Wander, trigger: Some((0, 1)), fade: 2.0, ..Lfo::new(1) };
    let (_tmp, p) = lfo_project(1, vec![trig.clone()]);
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    assert_eq!(e.lfo_value(1), Some(0.0), "0 before the trigger's first note");
    e.note_on(0, 1, 60, 100, 240);
    assert_eq!(e.lfo_cycles(&e.lfos[0]), 0.0, "restarted at the note-on");
    blocks(&mut e, one_beat);
    let c = e.lfo_cycles(&e.lfos[0]);
    let v = e.lfo_value(1).unwrap();
    assert!((v - trig.output(c, Some(c))).abs() < 1e-4, "{v} vs {}", trig.output(c, Some(c)));
    e.note_on(0, 1, 60, 100, 240);
    assert_eq!(e.lfo_cycles(&e.lfos[0]), 0.0, "retriggered back to cycle 0");
}

/// Each route's travel is published for its knob (by group, sound and its place in
/// the sound's routes, read only by the same route): it follows the LFO; a route
/// gone, or the engine let go, publishes none.
#[test]
fn route_travels_are_published() {
    use deckard_model::lfo::{Lfo, Shape};
    let mut l = Lfo::new(1);
    l.shape = Shape::Square;
    let (_tmp, p) = lfo_project(1, vec![l]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    let board = std::sync::Arc::new(meters::RouteTravels::default());
    e.set_route_travels(&board);
    let key = meters::route_key(&p.groups[0].sounds[1].mod_routes[0]);
    assert_eq!(board.get(0, 1, 0, key), None, "nothing yet");
    blocks(&mut e, 1);
    assert_eq!(board.get(0, 1, 0, key), Some(1.0), "High");
    // Another route read at its place (the document's list changed first): none.
    let other = route(deckard_model::mod_matrix::ModSource::Lfo(1), 0, "Kick", field("tone"));
    assert_eq!(board.get(0, 1, 0, meters::route_key(&other)), None, "not the engine's route");
    let n = (e.samples_per_tick() * 480.0 / 256.0).ceil() as usize;
    blocks(&mut e, n);
    assert_eq!(board.get(0, 1, 0, key), Some(0.0), "Low");
    // The route removed: none.
    let mut q = p.clone();
    q.groups[0].sounds[1].mod_routes.clear();
    let (fresh, _) = Engine::new_after(&q, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &q));
    assert_eq!(board.get(0, 1, 0, key), None, "route gone");
    blocks(&mut e, 1);
    assert_eq!(board.get(0, 1, 0, key), None);
    // A muted LFO: none; the engine let go: none.
    let mut m = p.clone();
    m.lfos[0].muted = true;
    let (fresh, _) = Engine::new_after(&p, Some((&q, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &p));
    blocks(&mut e, 1);
    assert!(board.get(0, 1, 0, key).is_some(), "back");
    let (fresh, _) = Engine::new_after(&m, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &m));
    blocks(&mut e, 1);
    assert_eq!(board.get(0, 1, 0, key), None, "muted");
    drop(e.adopt(Engine::new_after(&p, Some((&m, &[])), None, 48_000, Mode::Arrangement, &mut cache).0, &p));
    blocks(&mut e, 1);
    e.release_routes();
    assert_eq!(board.get(0, 1, 0, key), None, "released");
}

/// A route from an LFO that isn't there leaves the knob at its own value.
#[test]
fn a_missing_lfo_leaves_the_knob_alone() {
    use deckard_model::lfo::Lfo;
    let (_tmp, p) = lfo_project(9, vec![Lfo::new(1)]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    assert_eq!(e.lfo_value(9), None);
    blocks(&mut e, 20);
    assert!((decay_now(&e) - 0.5).abs() < 1e-6, "{}", decay_now(&e));
}

/// A muted LFO leaves the knob at its own value; unmuted, it moves it again on its
/// clock. Its trigger's notes still count while it's muted.
#[test]
fn a_muted_lfo_leaves_the_knob_alone() {
    use deckard_model::lfo::{Lfo, Shape};
    let l = Lfo { shape: Shape::Square, muted: true, ..Lfo::new(1) };
    let (_tmp, p) = lfo_project(1, vec![l]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    assert_eq!(e.lfo_value(1), None);
    blocks(&mut e, 1);
    assert!((decay_now(&e) - 0.5).abs() < 1e-6, "{}", decay_now(&e));
    let mut q = p.clone();
    q.lfos[0].muted = false;
    let (fresh, _) = Engine::new_after(&q, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &q));
    blocks(&mut e, 1);
    assert!((decay_now(&e) - 1.0).abs() < 1e-6, "{}", decay_now(&e));
    let t = Lfo { trigger: Some((0, 1)), muted: true, ..Lfo::new(1) };
    let (_tmp, p) = lfo_project(1, vec![t]);
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    e.note_on(0, 1, 60, 100, 240);
    assert_eq!(e.lfos[0].trig_at.map(|t| t.1), Some(e.time), "a muted LFO's trigger still counts");
}

/// A CC route from Square LFO 1 (high 0.5) to CC 74 on channel 3 of "Synth", and what
/// the engine sends.
fn cc_engine(lfo: deckard_model::lfo::Lfo, port: &str) -> (tempfile::TempDir, Engine, std::sync::mpsc::Receiver<midi::MidiOut>) {
    use deckard_model::mod_matrix::{CcRoute, ModSource};
    let (tmp, mut p) = lfo_project(1, vec![lfo]);
    p.cc_routes = vec![CcRoute { high: 0.5, ..CcRoute::new(ModSource::Lfo(1), port, 3, 74) }];
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    let (tx, rx) = std::sync::mpsc::sync_channel(64);
    e.midi_tx = Some(tx);
    (tmp, e, rx)
}

fn ccs(rx: &std::sync::mpsc::Receiver<midi::MidiOut>) -> Vec<(String, Vec<u8>)> {
    rx.try_iter().map(|m| (m.port.to_string(), m.bytes().to_vec())).collect()
}

/// An LFO sends its CC at 127 × its travel, only when the value changes: a Square at
/// 1/4 sends 64 (half of the way at High 0.5), then 0 half a beat later.
#[test]
fn an_lfo_sends_a_cc() {
    use deckard_model::lfo::{Lfo, Shape};
    let (_tmp, mut e, rx) = cc_engine(Lfo { shape: Shape::Square, ..Lfo::new(1) }, "Synth");
    blocks(&mut e, 1);
    assert_eq!(ccs(&rx), vec![("Synth".to_string(), vec![0xb2, 74, 64])]);
    blocks(&mut e, 4);
    assert!(ccs(&rx).is_empty(), "the same value isn't sent again");
    let n = (e.samples_per_tick() * 480.0 / 256.0).ceil() as usize;
    blocks(&mut e, n);
    assert_eq!(ccs(&rx), vec![("Synth".to_string(), vec![0xb2, 74, 0])]);
}

/// An edit (a new engine adopted) doesn't send the same value again.
#[test]
fn an_edit_keeps_what_a_cc_sent() {
    use deckard_model::lfo::{Lfo, Shape};
    let (_tmp, mut e, rx) = cc_engine(Lfo { shape: Shape::Square, ..Lfo::new(1) }, "Synth");
    blocks(&mut e, 1);
    assert_eq!(ccs(&rx).len(), 1);
    let (_t, p) = lfo_project(1, vec![Lfo { shape: Shape::Square, ..Lfo::new(1) }]);
    let mut q = p.clone();
    q.cc_routes = vec![deckard_model::mod_matrix::CcRoute {
        high: 0.5,
        ..deckard_model::mod_matrix::CcRoute::new(deckard_model::mod_matrix::ModSource::Lfo(1), "Synth", 3, 74)
    }];
    let mut cache = SampleCache::default();
    let (fresh, _) = Engine::new_after(&q, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &q));
    blocks(&mut e, 1);
    assert!(ccs(&rx).is_empty());
}

/// Depth turned back to 0 settles the CC once on its constant (it doesn't freeze at
/// the last value sent).
#[test]
fn depth_back_to_0_settles_the_cc() {
    use deckard_model::lfo::{Lfo, Shape};
    let (_tmp, mut e, rx) = cc_engine(Lfo { shape: Shape::Square, ..Lfo::new(1) }, "Synth");
    blocks(&mut e, 1);
    assert_eq!(ccs(&rx), vec![("Synth".to_string(), vec![0xb2, 74, 64])]);
    e.cc_routes[0].route.high = 0.0;
    blocks(&mut e, 1);
    assert_eq!(ccs(&rx), vec![("Synth".to_string(), vec![0xb2, 74, 0])]);
    blocks(&mut e, 200);
    assert!(ccs(&rx).is_empty(), "once");
}

/// Frames in a CC's settle (0.25 s at 48 kHz) as blocks of 256, and a few more.
const SETTLE_BLOCKS: usize = 48_000 / 4 / 256 + 2;

/// The engine `e` adopting `q` (from `p`): an edit.
fn edit_to(e: &mut Engine, p: &Project, q: &Project) {
    let mut cache = SampleCache::default();
    let (fresh, _) = Engine::new_after(q, Some((p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, q));
}

/// A live route given another port, channel or CC (an edit) sends its value there,
/// once it has stayed there 0.25 s.
#[test]
fn a_moved_cc_sends_to_its_new_place() {
    use deckard_model::lfo::{Lfo, Shape};
    use deckard_model::mod_matrix::{CcRoute, ModSource};
    let l = Lfo { shape: Shape::Square, ..Lfo::new(1) };
    let (_tmp, mut e, rx) = cc_engine(l.clone(), "Synth");
    blocks(&mut e, 1);
    assert_eq!(ccs(&rx).len(), 1);
    let (_t, p) = lfo_project(1, vec![l]);
    let route = |port: &str, ch, cc| CcRoute { high: 0.5, ..CcRoute::new(ModSource::Lfo(1), port, ch, cc) };
    for (port, ch, cc, want) in [("Other", 3, 74, vec![0xb2, 74]), ("Other", 5, 74, vec![0xb4, 74]), ("Other", 5, 1, vec![0xb4, 1])] {
        let mut q = p.clone();
        q.cc_routes = vec![route(port, ch, cc)];
        edit_to(&mut e, &p, &q);
        blocks(&mut e, SETTLE_BLOCKS);
        let got = ccs(&rx);
        assert_eq!(got.len(), 1, "{port} {ch} {cc}: {got:?}");
        assert_eq!((got[0].0.as_str(), &got[0].1[..2]), (port, &want[..]), "{port} {ch} {cc}");
    }
}

/// A CC turned 1, 2, 3 in quick edits sends nothing to 1 or 2: only to 3, 0.25 s
/// after the last change. A route left as it was keeps sending meanwhile.
#[test]
fn a_cc_settles_before_it_sends() {
    use deckard_model::lfo::{Lfo, Shape};
    use deckard_model::mod_matrix::{CcRoute, ModSource};
    let l = Lfo { shape: Shape::Square, ..Lfo::new(1) };
    let (_tmp, mut e, rx) = cc_engine(l.clone(), "Synth");
    let (_t, p) = lfo_project(1, vec![l]);
    let kept = CcRoute { high: 0.5, ..CcRoute::new(ModSource::Lfo(1), "Synth", 3, 74) };
    let turned = |cc| CcRoute { high: 0.5, ..CcRoute::new(ModSource::Lfo(1), "Synth", 1, cc) };
    let mut q = p.clone();
    q.cc_routes = vec![kept.clone()];
    edit_to(&mut e, &p, &q);
    blocks(&mut e, 1);
    assert_eq!(ccs(&rx), vec![("Synth".to_string(), vec![0xb2, 74, 64])], "an unchanged route sends at once");
    let mut was = q;
    for cc in [1, 2, 3] {
        let mut q = p.clone();
        q.cc_routes = vec![kept.clone(), turned(cc)];
        edit_to(&mut e, &was, &q);
        was = q;
        blocks(&mut e, 10);
        assert!(ccs(&rx).iter().all(|(_, b)| b[0] == 0xb2), "nothing to CC {cc} yet");
    }
    // Half a beat on (120 BPM): the square falls, the kept route sends 0.
    let n = (e.samples_per_tick() * 480.0 / 256.0).ceil() as usize;
    blocks(&mut e, n);
    let got = ccs(&rx);
    assert!(got.contains(&("Synth".to_string(), vec![0xb2, 74, 0])), "the kept route keeps sending: {got:?}");
    let to_new: Vec<_> = got.iter().filter(|(_, b)| b[0] == 0xb0).collect();
    assert_eq!(to_new.len(), 1, "{got:?}");
    assert_eq!(to_new[0].1[1], 3, "only to the last CC");
}

/// A muted LFO sends nothing more (no reset value); a CC to a port that isn't open is
/// sent like any (the port's thread drops it) and nothing breaks.
#[test]
fn a_muted_lfo_sends_no_cc() {
    use deckard_model::lfo::{Lfo, Shape};
    let (_tmp, mut e, rx) = cc_engine(Lfo { shape: Shape::Square, muted: true, ..Lfo::new(1) }, "Synth");
    blocks(&mut e, 200);
    assert!(ccs(&rx).is_empty());
    // Depth 0 (a new route): nothing either.
    let (_tmp, mut e, rx) = cc_engine(Lfo { shape: Shape::Square, ..Lfo::new(1) }, "Synth");
    e.cc_routes[0].route.high = 0.0;
    blocks(&mut e, 200);
    assert!(ccs(&rx).is_empty(), "Depth 0");
    let (_tmp, mut e, _rx) = cc_engine(Lfo::new(1), "Nowhere");
    e.midi_tx = None;
    blocks(&mut e, 20);
}

/// A triggered LFO is silent until its sound's note, then fades in over its fade.
#[test]
fn a_note_restarts_a_triggered_lfo() {
    use deckard_model::lfo::{Lfo, Shape};
    let mut l = Lfo::new(1);
    // (A whole note's cycle: the wave is 1 for its first two beats.)
    (l.shape, l.rate, l.trigger, l.fade) = (Shape::Square, 6, Some((0, 1)), 1.0);
    let (_tmp, p) = lfo_project(1, vec![l]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    blocks(&mut e, 4);
    assert_eq!(e.lfo_value(1), Some(0.0));
    e.note_on(0, 1, 60, 100, 240);
    blocks(&mut e, 47);
    let since = (e.lfo_clock.ever - e.lfos[0].trig_at.unwrap().0) / PPQ as f64;
    assert!((since - 0.5).abs() < 0.01, "{since}");
    let v = e.lfo_value(1).unwrap();
    assert!((f64::from(v) - since).abs() < 1e-4, "{v} vs {since}");
}

/// A triggered LFO's wave starts at its phase point at each note, wherever in the
/// bar the note falls.
#[test]
fn a_triggered_lfo_starts_at_its_phase_point() {
    use deckard_model::lfo::{Lfo, Shape};
    let mut l = Lfo::new(1);
    (l.shape, l.trigger, l.fade) = (Shape::Square, Some((0, 1)), 0.0);
    let (_tmp, p) = lfo_project(1, vec![l]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    // 0.62 of a beat in: on the song's clock the wave would be 0.
    blocks(&mut e, 58);
    e.note_on(0, 1, 60, 100, 240);
    assert_eq!(e.lfo_value(1), Some(1.0));
    // A quarter beat on: still the cycle's first half.
    blocks(&mut e, 23);
    assert_eq!(e.lfo_value(1), Some(1.0));
    assert!((decay_now(&e) - 1.0).abs() < 1e-6, "{}", decay_now(&e));
}

/// An LFO driving a plug-in's parameter is removed (the route stays): the plug-in
/// plays its knob again.
#[test]
fn a_removed_lfo_lets_go_of_a_plugins_parameter() {
    use deckard_model::lfo::Lfo;
    use deckard_model::mod_matrix::{ModParam, ModSource};
    let class = deckard_plugins::native::CLASSES[0];
    let id = class.params.iter().find(|x| x.steps == 0).unwrap().id;
    let pr = deckard_model::PluginRef { id: class.plugin_id(), name: class.name.into(), ..Default::default() };
    let (_tmp, mut p) = eight_project();
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
    snd.mod_routes = vec![route(ModSource::Lfo(1), 0, &pr.id, ModParam::Plugin(id))];
    p.lfos = vec![Lfo::new(1)];
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    blocks(&mut e, 2);
    let plugin = e.instruments[0][1].as_ref().unwrap().plugin.clone();
    assert!(plugin.lock().unwrap().modulated(id).is_some(), "the LFO drives it");
    let mut gone = p.clone();
    gone.lfos.clear();
    let (fresh, _) = Engine::new_after(&gone, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &gone));
    blocks(&mut e, 1);
    assert_eq!(plugin.lock().unwrap().modulated(id), None, "back to its knob");
    // Busy when the span comes: let go at a later one.
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    blocks(&mut e, 2);
    let (fresh, _) = Engine::new_after(&gone, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &gone));
    {
        let _busy = plugin.lock().unwrap();
        e.apply_routes();
    }
    assert!(plugin.lock().unwrap().modulated(id).is_some(), "still held");
    blocks(&mut e, 1);
    assert_eq!(plugin.lock().unwrap().modulated(id), None, "let go once free");
    // Muted rather than removed: let go the same way.
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    blocks(&mut e, 2);
    assert!(plugin.lock().unwrap().modulated(id).is_some());
    let mut muted = p.clone();
    muted.lfos[0].muted = true;
    let (fresh, _) = Engine::new_after(&muted, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &muted));
    blocks(&mut e, 1);
    assert_eq!(plugin.lock().unwrap().modulated(id), None, "muted: back to its knob");
}

/// Muted notes, notes of sounds a solo left out, and chokes don't trigger.
#[test]
fn muted_soloed_and_choked_notes_dont_trigger() {
    use deckard_model::lfo::Lfo;
    let mut l = Lfo::new(1);
    l.trigger = Some((0, 1));
    let (_tmp, p) = lfo_project(1, vec![l]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    e.sound_muted[0][1] = true;
    e.note_on(0, 1, 60, 100, 240);
    assert_eq!(e.lfos[0].trig_at, None, "muted");
    e.sound_muted[0][1] = false;
    e.retain_sounds(|g, s| (g, s) != (0, 1));
    e.note_on(0, 1, 60, 100, 240);
    assert_eq!(e.lfos[0].trig_at, None, "soloed out");
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    let tps = 1.0 / e.samples_per_tick();
    let choke = NoteEvent {
        expr_at: 0,
        roll: None,
        tick: 0,
        group: 0,
        sound: 1,
        pitch: 0,
        velocity: 0,
        length: 0,
        probability: 100,
        midi: Some(pads::CHOKE),
        cue: None,
        expr: None,
        from: None,
    };
    e.trigger(choke, tps);
    assert_eq!(e.lfos[0].trig_at, None, "choked");
    blocks(&mut e, 1);
    e.note_on(0, 1, 60, 100, 240);
    assert_eq!(e.lfos[0].trig_at.map(|t| t.1), Some(e.time), "a note that sounds");
}

/// An envelope on LFO 1 that sound 1's notes play: `a`, `d`, `r` seconds, sustain 0.5.
fn env_engine(a: f32, d: f32, r: f32, retrigger: bool) -> (tempfile::TempDir, Project, Engine, SampleCache) {
    use deckard_model::lfo::Lfo;
    let l = Lfo { env: true, trigger: Some((0, 1)), attack: a, decay: d, sustain: 0.5, release: r, retrigger, ..Lfo::new(1) };
    let (tmp, p) = lfo_project(1, vec![l]);
    let mut cache = SampleCache::default();
    let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    (tmp, p, e, cache)
}

fn env_now(e: &Engine) -> f32 {
    e.lfo_value(1).unwrap()
}

/// Silent before a note; a held pad rises, holds at the sustain, and falls to 0 when
/// let go; its route follows.
#[test]
fn an_envelope_follows_a_held_pad() {
    let (_tmp, _p, mut e, _c) = env_engine(0.01, 0.01, 0.05, true);
    blocks(&mut e, 2);
    assert_eq!(e.lfo_value(1), Some(0.0), "silent before a note");
    e.note_hold(0, 1, 60, 100);
    blocks(&mut e, 1);
    let v = env_now(&e);
    assert!((v - 256.0 / 480.0).abs() < 1e-3, "rising: {v}");
    blocks(&mut e, 10);
    assert!((env_now(&e) - 0.5).abs() < 1e-6, "sustain: {}", env_now(&e));
    assert!((decay_now(&e) - 0.5).abs() < 1e-6, "the route follows: {}", decay_now(&e));
    e.note_release(0, 1, 60);
    blocks(&mut e, 1);
    let v = env_now(&e);
    assert!(v < 0.5 && v > 0.0, "releasing: {v}");
    blocks(&mut e, 15);
    assert_eq!(env_now(&e), 0.0);
}

/// Two notes held, one let go: still at the sustain; the last let go: released.
#[test]
fn overlapping_notes_release_after_the_last() {
    let (_tmp, _p, mut e, _c) = env_engine(0.01, 0.01, 0.01, true);
    e.note_hold(0, 1, 60, 100);
    blocks(&mut e, 2);
    e.note_hold(0, 1, 62, 100);
    blocks(&mut e, 10);
    e.note_release(0, 1, 60);
    blocks(&mut e, 3);
    assert!((env_now(&e) - 0.5).abs() < 1e-6, "one still held: {}", env_now(&e));
    e.note_release(0, 1, 62);
    blocks(&mut e, 1);
    assert!(env_now(&e) < 0.5);
    blocks(&mut e, 5);
    assert_eq!(env_now(&e), 0.0);
}

/// Retrigger off: a second note while one is held changes nothing, a note in the
/// release starts it again (from where it was); on: every note restarts the attack.
#[test]
fn legato_off_ignores_a_second_note_and_retrigger_restarts_it() {
    for retrigger in [false, true] {
        let (_tmp, _p, mut e, _c) = env_engine(0.1, 0.1, 0.1, retrigger);
        e.note_hold(0, 1, 60, 100);
        blocks(&mut e, 40);
        assert!((env_now(&e) - 0.5).abs() < 1e-6, "{}", env_now(&e));
        e.note_hold(0, 1, 62, 100);
        blocks(&mut e, 1);
        let v = env_now(&e);
        if retrigger {
            assert!((v - (0.5 + 256.0 / 4800.0)).abs() < 1e-3, "restarted from 0.5: {v}");
        } else {
            assert!((v - 0.5).abs() < 1e-6, "legato: {v}");
        }
    }
    let (_tmp, _p, mut e, _c) = env_engine(0.1, 0.1, 0.1, false);
    e.note_hold(0, 1, 60, 100);
    blocks(&mut e, 40);
    e.note_release(0, 1, 60);
    blocks(&mut e, 2);
    let falling = env_now(&e);
    e.note_hold(0, 1, 64, 100);
    blocks(&mut e, 1);
    let v = env_now(&e);
    assert!((v - (falling + 256.0 / 4800.0)).abs() < 1e-3, "rising from {falling}: {v}");
}

/// A pattern note's end (its length) releases the envelope.
#[test]
fn a_pattern_notes_end_releases_the_envelope() {
    use deckard_model::lfo::Lfo;
    let l = Lfo { env: true, trigger: Some((0, 1)), attack: 0.001, decay: 0.001, release: 0.001, ..Lfo::new(1) };
    let (_tmp, mut p) = lfo_project(1, vec![l]);
    let mut n = p.groups[0].patterns[0].notes[0].clone();
    (n.sound, n.position, n.length, n.probability) = (1, 0, 240, 100);
    p.groups[0].patterns[0].notes.push(n);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    let end = (240.0 * e.samples_per_tick() / 256.0) as usize;
    blocks(&mut e, end - 2);
    assert!((env_now(&e) - 0.5).abs() < 1e-6, "held for its length: {}", env_now(&e));
    blocks(&mut e, 4);
    assert_eq!(env_now(&e), 0.0, "released at its end");
    assert_eq!(e.lfos[0].held, 0);
}

/// Stop lets go of every held note (the envelope releases), and so does a choke of
/// the trigger sound.
#[test]
fn stop_releases_a_held_envelope() {
    let (_tmp, _p, mut e, _c) = env_engine(0.01, 0.01, 0.05, true);
    e.note_hold(0, 1, 60, 100);
    e.note_hold(0, 1, 62, 100);
    blocks(&mut e, 10);
    e.stop();
    assert_eq!(e.lfos[0].held, 0);
    blocks(&mut e, 1);
    assert!(env_now(&e) < 0.5 && env_now(&e) > 0.0, "releases: {}", env_now(&e));
    blocks(&mut e, 15);
    assert_eq!(env_now(&e), 0.0);
    e.note_hold(0, 1, 60, 100);
    blocks(&mut e, 10);
    let choke = NoteEvent {
        expr_at: 0,
        roll: None,
        tick: 0,
        group: 0,
        sound: 1,
        pitch: 0,
        velocity: 0,
        length: 0,
        probability: 100,
        midi: Some(pads::CHOKE),
        cue: None,
        expr: None,
        from: None,
    };
    e.trigger(choke, 1.0 / e.samples_per_tick());
    assert_eq!(e.lfos[0].held, 0, "a choke ends its notes");
    blocks(&mut e, 15);
    assert_eq!(env_now(&e), 0.0);
}

/// An edit mid-note (the decay turned): the envelope carries on, held, at its level.
#[test]
fn an_edit_mid_note_keeps_the_envelope() {
    let (_tmp, p, mut e, mut cache) = env_engine(0.01, 0.01, 0.01, true);
    e.note_hold(0, 1, 60, 100);
    blocks(&mut e, 10);
    let mut q = p.clone();
    q.lfos[0].decay = 0.02;
    let (fresh, _) = Engine::new_after(&q, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &q));
    assert_eq!(e.lfos[0].held, 1);
    assert!((env_now(&e) - 0.5).abs() < 1e-6, "no restart, no drop: {}", env_now(&e));
    blocks(&mut e, 1);
    assert!((env_now(&e) - 0.5).abs() < 1e-6, "{}", env_now(&e));
    e.note_release(0, 1, 60);
    blocks(&mut e, 6);
    assert_eq!(env_now(&e), 0.0);
}

/// Adopt `q` as an edit of `p`.
fn adopt_edit(e: &mut Engine, p: &Project, q: &Project, cache: &mut SampleCache) {
    let (fresh, _) = Engine::new_after(q, Some((p, &[])), None, 48_000, Mode::Arrangement, cache);
    drop(e.adopt(fresh, q));
}

/// The trigger changed while its note is held: the envelope lets go of that note and
/// releases from where it is (letting go of the old pad later changes nothing).
#[test]
fn a_trigger_changed_mid_note_releases() {
    let (_tmp, p, mut e, mut cache) = env_engine(0.01, 0.01, 0.05, true);
    e.note_hold(0, 1, 60, 100);
    blocks(&mut e, 10);
    let mut q = p.clone();
    q.lfos[0].trigger = Some((0, 2));
    // (Sound 2's own notes in the song would play the envelope again.)
    q.groups[0].patterns[0].notes.retain(|x| x.sound != 2);
    adopt_edit(&mut e, &p, &q, &mut cache);
    assert_eq!(e.lfos[0].held, 0);
    assert!((env_now(&e) - 0.5).abs() < 1e-6, "no jump: {}", env_now(&e));
    blocks(&mut e, 1);
    assert!(env_now(&e) < 0.5 && env_now(&e) > 0.0, "releasing: {}", env_now(&e));
    e.note_release(0, 1, 60);
    blocks(&mut e, 15);
    assert_eq!(env_now(&e), 0.0);
}

/// Envelope → LFO → Envelope with a note let go between: nothing stays held.
#[test]
fn a_mode_toggle_mid_note_never_sticks() {
    let (_tmp, p, mut e, mut cache) = env_engine(0.01, 0.01, 0.05, true);
    e.note_hold(0, 1, 60, 100);
    blocks(&mut e, 10);
    let mut q = p.clone();
    q.lfos[0].env = false;
    adopt_edit(&mut e, &p, &q, &mut cache);
    e.note_release(0, 1, 60);
    blocks(&mut e, 1);
    adopt_edit(&mut e, &q, &p, &mut cache);
    assert_eq!(e.lfos[0].held, 0);
    blocks(&mut e, 15);
    assert_eq!(env_now(&e), 0.0);
}

/// Turning Attack, Decay or Release mid-segment carries on from the level reached
/// (moving no more than a block's worth), then follows the new time.
#[test]
fn a_time_edit_mid_segment_keeps_the_value() {
    let step = 256.0 / 48_000.0 / 0.5;
    // Attack 5 s, a second in: turned to 0.5 s.
    let (_tmp, p, mut e, mut cache) = env_engine(5.0, 0.3, 5.0, true);
    e.note_hold(0, 1, 60, 100);
    blocks(&mut e, 188);
    let v = env_now(&e);
    assert!((v - 0.2).abs() < 0.01, "{v}");
    let mut q = p.clone();
    q.lfos[0].attack = 0.5;
    adopt_edit(&mut e, &p, &q, &mut cache);
    assert!((env_now(&e) - v).abs() < 1e-4, "attack: {} vs {v}", env_now(&e));
    blocks(&mut e, 1);
    assert!(env_now(&e) - v > 0.0 && env_now(&e) - v <= step as f32 + 1e-4, "{}", env_now(&e));
    // Released, a second into a 5 s release: turned to 0.5 s.
    blocks(&mut e, 100);
    e.note_release(0, 1, 60);
    blocks(&mut e, 188);
    let v = env_now(&e);
    let mut r = q.clone();
    r.lfos[0].release = 0.5;
    adopt_edit(&mut e, &q, &r, &mut cache);
    assert!((env_now(&e) - v).abs() < 1e-4, "release: {} vs {v}", env_now(&e));
    blocks(&mut e, 1);
    assert!(v - env_now(&e) > 0.0 && v - env_now(&e) <= step as f32 + 1e-4, "{}", env_now(&e));
    // Decay 5 s to 0, a second in: turned to 0.5 s.
    let (_tmp, mut p, _e, mut cache) = env_engine(0.001, 5.0, 5.0, true);
    p.lfos[0].sustain = 0.0;
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    e.note_hold(0, 1, 60, 100);
    blocks(&mut e, 188);
    let v = env_now(&e);
    assert!((v - 0.8).abs() < 0.01, "{v}");
    let mut q = p.clone();
    q.lfos[0].decay = 0.5;
    adopt_edit(&mut e, &p, &q, &mut cache);
    assert!((env_now(&e) - v).abs() < 1e-4, "decay: {} vs {v}", env_now(&e));
    blocks(&mut e, 1);
    assert!(v - env_now(&e) > 0.0 && v - env_now(&e) <= step as f32 + 1e-4, "{}", env_now(&e));
}

/// A muted envelope, or one whose trigger is gone, gives nothing (its routes leave
/// their targets alone).
#[test]
fn a_muted_or_untriggered_envelope_gives_nothing() {
    use deckard_model::lfo::Lfo;
    for l in [Lfo { env: true, trigger: Some((0, 1)), muted: true, ..Lfo::new(1) }, Lfo { env: true, trigger: None, ..Lfo::new(1) }] {
        let (_tmp, p) = lfo_project(1, vec![l]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        e.note_hold(0, 1, 60, 100);
        blocks(&mut e, 4);
        assert_eq!(e.lfo_value(1), None);
        assert!((decay_now(&e) - 0.5).abs() < 1e-6, "the knob's own value: {}", decay_now(&e));
    }
}

/// An edit mid-fade: the fade carries on.
#[test]
fn an_edit_keeps_a_running_fade() {
    use deckard_model::lfo::Lfo;
    let mut l = Lfo::new(1);
    (l.trigger, l.fade) = (Some((0, 1)), 2.0);
    let (_tmp, p) = lfo_project(1, vec![l]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    e.note_on(0, 1, 60, 100, 240);
    blocks(&mut e, 24);
    let at = e.lfos[0].trig_at;
    assert!(at.is_some());
    let mut q = p.clone();
    q.lfos[0].level = 0.5;
    let (fresh, _) = Engine::new_after(&q, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &q));
    assert_eq!(e.lfos[0].trig_at, at);
    assert_eq!(e.lfos[0].lfo.level, 0.5, "the edit's LFO");
}

/// An edit giving an LFO another trigger: it waits for that sound's first note.
#[test]
fn another_trigger_waits_for_its_note() {
    use deckard_model::lfo::Lfo;
    let l = Lfo { trigger: Some((0, 1)), ..Lfo::new(1) };
    let (_tmp, p) = lfo_project(1, vec![l]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    e.note_on(0, 1, 60, 100, 240);
    blocks(&mut e, 24);
    assert!(e.lfos[0].trig_at.is_some());
    let mut q = p.clone();
    q.lfos[0].trigger = Some((0, 2));
    let (fresh, _) = Engine::new_after(&q, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &q));
    assert_eq!(e.lfos[0].trig_at, None);
    assert_eq!(e.lfo_value(1), Some(0.0), "silent until the new sound's note");
}

/// A synced LFO in Ideas counts the ticks played, each at the tempo then: a tempo
/// change (an edit) mid-play doesn't move it, and it carries on at the new tempo.
#[test]
fn a_synced_lfo_follows_the_position_not_the_tempo_history() {
    use deckard_model::lfo::Lfo;
    let (_tmp, p) = lfo_project(1, vec![Lfo::new(1)]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Pattern(0, 0), &mut cache);
    play_frames(&mut e, 24_000);
    let c = e.lfo_cycles(&e.lfos[0]);
    assert!((c - 1.0).abs() < 1e-9, "a beat at 120: {c}");
    let mut slow = p.clone();
    slow.tempo = 90.0;
    let (fresh, _) = Engine::new_after(&slow, Some((&p, &[])), None, 48_000, Mode::Pattern(0, 0), &mut cache);
    drop(e.adopt(fresh, &slow));
    let c = e.lfo_cycles(&e.lfos[0]);
    assert!((c - 1.0).abs() < 1e-9, "the tempo changed, the LFO didn't move: {c}");
    play_frames(&mut e, 32_000);
    let c = e.lfo_cycles(&e.lfos[0]);
    assert!((c - 2.0).abs() < 1e-9, "a beat at 90: {c}");
}

/// After a bar's count-in, a bar played: a 1-bar synced LFO is exactly on the bar
/// (the count-in doesn't count).
#[test]
fn a_synced_lfo_is_on_the_bar_after_a_count_in() {
    use deckard_model::lfo::Lfo;
    let l = Lfo { rate: 6, ..Lfo::new(1) };
    let (_tmp, p) = lfo_project(1, vec![l]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Pattern(0, 0), &mut cache);
    let bar = 96_000;
    e.count_in(bar as u64);
    play_frames(&mut e, bar);
    let c = e.lfo_cycles(&e.lfos[0]);
    assert!(c.abs() < 1e-9, "still at the start after the count-in: {c}");
    play_frames(&mut e, bar);
    let c = e.lfo_cycles(&e.lfos[0]);
    assert!((c - 1.0).abs() < 1e-9, "{c}");
}

/// A Hz LFO after a count-in is where it is in an export (from the song's start).
#[test]
fn a_hz_lfo_after_a_count_in_matches_an_export() {
    use deckard_model::lfo::Lfo;
    let l = Lfo { sync: false, hz: 0.7, ..Lfo::new(1) };
    let (_tmp, p) = lfo_project(1, vec![l]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Pattern(0, 0), &mut cache);
    e.count_in(96_000);
    play_frames(&mut e, 96_000 + 30_000);
    let (mut x, _) = Engine::new(&p, None, 48_000, Mode::Pattern(0, 0), &mut cache);
    play_frames(&mut x, 30_000);
    let (a, b) = (e.lfo_cycles(&e.lfos[0]), x.lfo_cycles(&x.lfos[0]));
    assert!((a - b).abs() < 1e-9, "{a} vs {b}");
    assert_eq!(e.lfo_value(1), x.lfo_value(1));
}

/// Stopped, an LFO keeps running at the tempo from where it was (in the
/// arrangement too, from the position it stopped at).
#[test]
fn a_stopped_lfo_keeps_running_from_where_it_was() {
    use deckard_model::lfo::Lfo;
    let (_tmp, p) = lfo_project(1, vec![Lfo::new(1)]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    play_frames(&mut e, 24_000);
    e.stop();
    play_frames(&mut e, 24_000);
    let c = e.lfo_cycles(&e.lfos[0]);
    assert!((c - 2.0).abs() < 1e-9, "{c}");
}

/// A tempo change mid-fade: a triggered LFO carries on from where it was.
#[test]
fn a_tempo_change_keeps_a_triggered_lfo_in_place() {
    use deckard_model::lfo::Lfo;
    let l = Lfo { trigger: Some((0, 1)), fade: 4.0, ..Lfo::new(1) };
    let (_tmp, p) = lfo_project(1, vec![l]);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Pattern(0, 0), &mut cache);
    e.note_on(0, 1, 60, 100, 240);
    play_frames(&mut e, 24_000);
    let (c, v) = (e.lfo_cycles(&e.lfos[0]), e.lfo_value(1));
    assert!((c - 1.0).abs() < 1e-9, "{c}");
    let mut slow = p.clone();
    slow.tempo = 90.0;
    let (fresh, _) = Engine::new_after(&slow, Some((&p, &[])), None, 48_000, Mode::Pattern(0, 0), &mut cache);
    drop(e.adopt(fresh, &slow));
    assert!((e.lfo_cycles(&e.lfos[0]) - c).abs() < 1e-9);
    assert_eq!(e.lfo_value(1), v);
}

/// Pressure moves a plug-in instrument's parameter; the route gone, the
/// modulation ends (the plug-in plays its knob again).
#[test]
fn pressure_moves_a_native_plugins_parameter() {
    use deckard_model::mod_matrix::{ModParam, ModSource};
    let class = deckard_plugins::native::CLASSES[0];
    let id = class.params.iter().find(|x| x.steps == 0).unwrap().id;
    let pr = deckard_model::PluginRef { id: class.plugin_id(), name: class.name.into(), ..Default::default() };
    let (_tmp, mut p) = eight_project();
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
    snd.mod_routes = vec![route(ModSource::Pressure, 0, &pr.id, ModParam::Plugin(id))];
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    press(&mut e, 0.75, 2);
    let plugin = e.instruments[0][1].as_ref().unwrap().plugin.clone();
    assert_eq!(plugin.lock().unwrap().modulated(id), Some(0.75));
    let mut plain = p.clone();
    plain.groups[0].sounds[1].mod_routes.clear();
    let (fresh, _) = Engine::new_after(&plain, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &plain));
    assert_eq!(plugin.lock().unwrap().modulated(id), None, "removing a route restores the plug-in");
    // Busy (the UI holding it) when the route goes: it's let go once free.
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    press(&mut e, 0.5, 2);
    let (fresh, _) = Engine::new_after(&plain, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    {
        let _busy = plugin.lock().unwrap();
        drop(e.adopt(fresh, &plain));
    }
    assert_eq!(plugin.lock().unwrap().modulated(id), Some(0.5), "still held");
    blocks(&mut e, 1);
    assert_eq!(plugin.lock().unwrap().modulated(id), None, "let go once free");
}

/// A Sampler route ended by its sound's instrument changing (a Kick loaded in its
/// place) leaves the new instrument as it is.
#[test]
fn ending_a_route_leaves_the_next_instrument_alone() {
    use deckard_model::mod_matrix::ModSource;
    let (_tmp, mut p) = eight_project();
    p.groups[0].sounds[1].sampler.as_mut().unwrap().set("tune", 0.9);
    p.groups[0].sounds[1].mod_routes = vec![route(ModSource::Pressure, 0, "Sampler", field("tune"))];
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    press(&mut e, 0.5, 2);
    let mut kick = p.clone();
    let snd = &mut kick.groups[0].sounds[1];
    snd.sampler = None;
    snd.slots = vec![deckard_model::SlotInfo {
        module: "Kick".into(),
        name: "Kick".into(),
        active: true,
        plugin: None,
        params: vec![("tune".into(), 0.2)],
    }];
    let (fresh, _) = Engine::new_after(&kick, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &kick));
    let v = e.sounds[0][1].as_ref().unwrap().synth_param("tune").unwrap();
    assert!((v - 0.2).abs() < 1e-9, "the Kick's own tune: {v}");
}

/// A route whose slot holds another module does nothing.
#[test]
fn a_route_to_another_module_does_nothing() {
    use deckard_model::mod_matrix::ModSource;
    let (_tmp, mut p, base) = modulated_project();
    p.groups[0].patterns[0].mods.clear();
    p.groups[0].sounds[1].mod_routes = vec![route(ModSource::Pressure, 1, "Compressor", field("feedback"))];
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    assert!(e.mod_routes.is_empty());
    press(&mut e, 1.0, 20);
    assert!((feedback_now(&e) - base).abs() < 1e-6);
}

/// A group's lane moves its bus's effect.
#[test]
fn modulation_moves_a_groups_effect() {
    let (_tmp, mut p, _) = modulated_project();
    let delay = p.groups[0].sounds[1].slots[1].clone();
    p.groups[0].slots = vec![delay];
    let len = p.groups[0].patterns[0].length.max(p.bar_length());
    let point = |position, value| deckard_model::ModPoint { sound: None, slot: Some(0), field: "feedback".into(), position, value };
    p.groups[0].patterns[0].mods = vec![point(0, 0.4), point(len, 0.4)];
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    let mut buf = vec![0f32; 256 * 2];
    for _ in 0..10 {
        e.process(&mut buf);
    }
    let bus = e.buses[0].as_ref().unwrap();
    let FxUnit::Internal { glide, .. } = &bus.chain.units[0].fx else { unreachable!() };
    let v = glide.now().iter().find(|(k, _)| k == "feedback").unwrap().1;
    assert!((v - 0.9).abs() < 1e-6, "0.5 up 0.4: {v}");
}

/// Pattern modulation lanes move the Sampler's parameters and a plug-in's parameters
/// while playing, and let go when stopped.
#[test]
fn modulation_moves_the_sampler_and_a_plugin_while_playing() {
    let (_tmp, mut p) = eight_project();
    let len = p.groups[0].patterns[0].length.max(p.bar_length());
    let base_cutoff = p.groups[0].sounds[1].sampler.as_ref().unwrap().get("cutoff");
    let class = deckard_plugins::native::CLASSES[0];
    let param = class.params.iter().find(|x| x.steps == 0).unwrap();
    let id = param.id;
    let pr = deckard_model::PluginRef { id: class.plugin_id(), name: class.name.into(), ..Default::default() };
    let snd2 = &mut p.groups[0].sounds[2];
    snd2.sampler = None;
    snd2.plugin = Some(pr.clone());
    snd2.slots = vec![deckard_model::SlotInfo {
        module: "PluginHost".into(),
        name: class.name.into(),
        active: true,
        plugin: Some(pr),
        params: vec![(id.to_string(), 0.25)],
    }];
    let pt = |sound, field: &str, position, value| deckard_model::ModPoint {
        sound: Some(sound),
        slot: Some(0),
        field: field.into(),
        position,
        value,
    };
    let id_field = id.to_string();
    p.groups[0].patterns[0].mods =
        vec![pt(1, "cutoff", 0, -0.5), pt(1, "cutoff", len, -0.5), pt(2, &id_field, 0, 0.5), pt(2, &id_field, len, 0.5)];
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    blocks(&mut e, 2);
    let cutoff = e.sounds[0][1].as_ref().unwrap().settings_cutoff();
    assert!((f64::from(cutoff) - (base_cutoff - 0.5).clamp(0.0, 1.0)).abs() < 1e-4, "{cutoff} vs {base_cutoff}");
    let plugin = e.instruments[0][2].as_ref().unwrap().plugin.clone();
    let want = (param.default + 0.5).clamp(0.0, 1.0);
    assert!((plugin.lock().unwrap().modulated(id).unwrap() - want).abs() < 1e-9);
    e.sequencing = false;
    blocks(&mut e, 2);
    let cutoff = e.sounds[0][1].as_ref().unwrap().settings_cutoff();
    assert!((f64::from(cutoff) - base_cutoff).abs() < 1e-4, "stopped: back to {base_cutoff} ({cutoff})");
    assert_eq!(plugin.lock().unwrap().modulated(id), None, "stopped: plug-in back to its knob");
}
