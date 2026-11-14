//! What can be modulated (`docs/modulation.md`): a module of a channel's chain, or the
//! channel's own gain or aux send — the four the engine plays
//! (`deckard_engine::channel_field`). The rest of a channel's properties (its cue, a
//! send's destination, its MIDI transpose, its swing) are not offered, as Maschine's own
//! Output page shows: its Level, Pan and send Levels, and nothing else it modulates.

use crate::layout::ChainTab;
use deckard_doc::{Chain, Target};
use deckard_model::ModTarget;
use deckard_surface::Action as A;

/// A project with a sound whose Sampler is in slot 0, showing the sound's own (channel)
/// properties, on the page set and page given.
fn app(set: usize, page: usize) -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
    (app.group, app.sound, app.chain_tab, app.channel) = (0, 0, ChainTab::Sound, true);
    (app.prop_set, app.prop_page) = (set, page);
    app.surface_step(0, 240, 100);
    app
}

/// Each knob shown on the channel's page: its name, its section, and whether it can be
/// modulated.
fn knobs(app: &mut crate::App) -> Vec<(String, String, bool)> {
    app.surface_view().control.knobs.iter().flatten().map(|k| (k.name.clone(), k.section.clone(), k.modulatable)).collect()
}

/// A sound's aux send is a channel property the engine plays: its Level knob can be
/// modulated, and a held step's turn writes the send's own lane (no slot).
#[test]
fn a_sounds_aux_send_level_is_modulated() {
    let mut app = app(1, 1); // the sound's Output ▸ Aux page
    let shown = knobs(&mut app);
    // (The surface shows a knob's name and section in capitals.)
    let k = shown
        .iter()
        .position(|(n, s, m)| n.eq_ignore_ascii_case("Level") && s.eq_ignore_ascii_case("Aux 1") && *m)
        .expect("Aux 1's Level knob");
    app.surface_act(A::StepModAt(Some((240, 240))));
    app.surface_act(A::StepMod { from: 0, length: 240, pads: 1 << 1, knob: k, inc: -0.25 });
    assert_eq!(app.surface_view().control.knobs[k].as_ref().unwrap().step_mod.as_deref(), Some("-25%"));
    app.surface_act(A::StepModAt(None));
    let d = app.doc.as_ref().unwrap().borrow();
    let mods: Vec<_> = d.project().groups[0].patterns[0].mods.iter().map(|m| (m.sound, m.slot, m.field.clone(), m.value)).collect();
    assert!(!mods.is_empty(), "written: {mods:?}");
    assert!(mods.iter().all(|m| m.1.is_none() && m.2 == "aux1_level"), "the send's own lane, with no slot: {mods:?}");
    assert!(mods.iter().any(|m| m.3 == -0.25), "the turn's deviation: {mods:?}");
}

/// The channel's other knobs can be modulated neither live nor in a step: the engine
/// doesn't play them, so they aren't offered.
#[test]
fn a_channels_other_knobs_are_not_offered() {
    let mut app = app(1, 0); // Output ▸ Audio
    let audio = knobs(&mut app);
    let named = |shown: &[(String, String, bool)], name: &str| shown.iter().any(|(n, _, m)| n.eq_ignore_ascii_case(name) && *m);
    assert!(named(&audio, "Level"), "Level: {audio:?}");
    for name in ["Cue", "Audio Mute"] {
        assert!(!named(&audio, name), "{name}: {audio:?}");
    }
    (app.prop_set, app.prop_page) = (1, 2); // Output ▸ MIDI
    let midi = knobs(&mut app);
    assert!(!named(&midi, "Transpose"), "Transpose: {midi:?}");
    (app.prop_set, app.prop_page) = (2, 0); // its Groove
    let groove = knobs(&mut app);
    assert!(!named(&groove, "Amount"), "swing isn't modulated yet: {groove:?}");
}

/// A send's knob travels over its level, so its modulation starts from where the send is.
#[test]
fn a_sends_knob_travels_over_its_level() {
    let app = app(1, 1);
    let doc = app.doc.as_ref().unwrap();
    doc.borrow_mut().set("Aux 1 Level", Target::Sound(0, 0), "aux1_level", mxprj::decode::Value::Float(0.5)).unwrap();
    doc.borrow_mut().set("Group Aux 2", Target::GroupStrip(0), "aux2_level", mxprj::decode::Value::Float(0.625)).unwrap();
    let d = doc.borrow();
    let send = |field: &str| ModTarget { slot: None, field: field.into() };
    assert_eq!(crate::modulation::knob_travel(d.project(), Chain::Sound(0, 0), &send("aux1_level")), Some(0.5));
    assert_eq!(crate::modulation::knob_travel(d.project(), Chain::Group(0), &send("aux2_level")), Some(0.625));
    // A channel property the engine doesn't play has no travel (nor a lane).
    assert_eq!(crate::modulation::knob_travel(d.project(), Chain::Sound(0, 0), &send("transpose")), None);
}
