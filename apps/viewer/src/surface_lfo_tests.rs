//! LFOs on DECKARD's own controller layer ([`deckard_surface::Action::Lfo`]).

use crate::layout::{Channel, silent_master_note, strip_cv_flag};
use deckard_model::AudioDest;
use deckard_model::mod_matrix::{CvRoute, ModSource};
use deckard_surface::Action as A;

fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    app
}

#[test]
fn create_then_shape_then_remove() {
    let mut app = app();
    app.surface_act(A::Lfo("create", 2, 0.0));
    let v = app.surface_view();
    assert_eq!(v.lfos[0].pad, 2);
    assert_eq!(v.lfo_focus, 2);
    app.surface_act(A::Lfo("shape", 2, 5.0));
    assert_eq!(app.surface_view().lfos[0].shape, "Random");
    app.surface_act(A::Lfo("shape", 2, 6.0));
    assert_eq!(app.surface_view().lfos[0].shape, "Wander");
    app.surface_act(A::Lfo("remove", 2, 0.0));
    assert!(app.surface_view().lfos.is_empty());
}

#[test]
fn mute_toggles() {
    let mut app = app();
    app.surface_act(A::Lfo("create", 1, 0.0));
    assert!(!app.surface_view().lfos[0].muted);
    app.surface_act(A::Lfo("mute", 1, 0.0));
    assert!(app.surface_view().lfos[0].muted);
    app.surface_act(A::Lfo("mute", 1, 0.0));
    assert!(!app.surface_view().lfos[0].muted);
    app.surface_act(A::Lfo("mute", 4, 0.0));
    assert_eq!(app.surface_view().lfos.len(), 1, "no LFO on pad 4: nothing made");
}

#[test]
fn knob_steps_clamp() {
    let mut app = app();
    app.surface_act(A::Lfo("create", 1, 0.0));
    app.surface_act(A::Lfo("phase", 1, 10.0));
    app.surface_act(A::Lfo("phase", 1, 1000.0));
    assert!(app.surface_view().lfos[0].phase <= 360.0);
    app.surface_act(A::Lfo("quantize", 1, -1.0));
    assert_eq!(app.surface_view().lfos[0].quantize, 0, "Off stays Off");
    app.surface_act(A::Lfo("quantize", 1, 1.0));
    assert_eq!(app.surface_view().lfos[0].quantize, 2);
    assert_eq!(app.surface_view().lfos[0].rate, "1/4");
    app.surface_act(A::Lfo("rate", 1, 1.0));
    assert_eq!(app.surface_view().lfos[0].rate, "1/4.");
    app.surface_act(A::Lfo("level", 1, -500.0));
    assert_eq!(app.surface_view().lfos[0].level, 0.0);
    app.surface_act(A::Lfo("fade", 1, 1000.0));
    assert_eq!(app.surface_view().lfos[0].fade, 16.0);
}

#[test]
fn trigger_focused_uses_the_focused_sound() {
    let mut app = app();
    app.surface_act(A::Lfo("create", 1, 0.0));
    app.surface_act(A::FocusSound(2));
    app.surface_act(A::Lfo("trigger_focused", 1, 1.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "A1 / S3");
    app.surface_act(A::Lfo("trigger_focused", 1, 0.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "Off");
    app.surface_act(A::Lfo("trigger", 1, 1.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "A1 / S1");
}

#[test]
fn points_follow_phase() {
    let mut app = app();
    app.surface_act(A::Lfo("create", 1, 0.0));
    let before = app.lfo_views(true)[0].points.clone();
    assert_eq!(before.len(), 240);
    app.surface_act(A::Lfo("phase", 1, 18.0));
    assert_ne!(app.lfo_views(true)[0].points[0], before[0]);
}

/// The waves' pictures and the mod matrix's slots are made only while their MK3 pages
/// show.
#[test]
fn pictures_and_slots_only_on_their_pages() {
    use deckard_mk3::Event;
    use std::time::Instant;
    let mut app = app();
    app.surface_act(A::Lfo("create", 1, 0.0));
    let v = app.surface_view();
    assert!(v.lfos[0].points.is_empty() && v.mod_slots.is_empty(), "the Control page");
    for (name, down) in [("BUTTON_SHIFT", true), ("BUTTON_SELECT", true), ("BUTTON_SELECT", false)] {
        app.surface.event(&Event::Button(name, down), &v, Instant::now());
    }
    assert_eq!(app.surface_view().lfos[0].points.len(), 240, "the LFO page");
    app.surface.event(&Event::Button("BUTTON_SOLO", true), &v, Instant::now());
    let v = app.surface_view();
    assert!(v.lfos[0].points.is_empty(), "the mod matrix page");
    assert_eq!(v.mod_slots.len(), 1, "its empty slot");
}

/// "env" toggles Envelope mode; with no trigger it takes the focused sound (and keeps
/// it back in LFO mode); the name reads "ENV n"; "retrigger" toggles.
#[test]
fn env_mode_takes_the_focused_sound() {
    let mut app = app();
    app.surface_act(A::Lfo("create", 3, 0.0));
    app.surface_act(A::FocusSound(1));
    app.surface_act(A::Lfo("env", 3, 0.0));
    let l = &app.surface_view().lfos[0];
    assert!(l.env);
    assert_eq!((l.name.as_str(), l.trigger.as_str()), ("ENV 3", "A1 / S2"));
    assert_eq!((l.attack.as_str(), l.decay.as_str(), l.release.as_str(), l.sustain), ("10 ms", "300 ms", "300 ms", 0.5));
    assert!(l.retrigger);
    app.surface_act(A::Lfo("retrigger", 3, 0.0));
    assert!(!app.surface_view().lfos[0].retrigger);
    app.surface_act(A::Lfo("env", 3, 0.0));
    let l = &app.surface_view().lfos[0];
    assert!(!l.env);
    assert_eq!((l.name.as_str(), l.trigger.as_str()), ("LFO 3", "A1 / S2"), "the trigger stays");
    assert!(!l.retrigger, "the envelope's settings kept");
    assert_eq!(l.note_off, None);
    app.surface_act(A::Lfo("env", 3, 0.0));
    assert!(app.lfo_views(true)[0].note_off.is_some());
}

/// Attack steps move the knob a hundredth of its exponential 1 ms–10 s travel, and stay
/// inside it; Sustain steps 1%.
#[test]
fn env_time_steps_are_exponential_and_clamped() {
    use deckard_model::lfo::{ENV_MAX, ENV_MIN, env_knob};
    let mut app = app();
    app.surface_act(A::Lfo("create", 1, 0.0));
    app.surface_act(A::Lfo("env", 1, 0.0));
    let attack = |app: &crate::App| app.doc.as_ref().unwrap().borrow().project().lfos[0].attack;
    let at = env_knob(attack(&app));
    app.surface_act(A::Lfo("attack", 1, 10.0));
    assert!((env_knob(attack(&app)) - (at + 0.1)).abs() < 1e-4);
    let a = attack(&app);
    app.surface_act(A::Lfo("attack", 1, 10.0));
    let b = attack(&app);
    app.surface_act(A::Lfo("attack", 1, 10.0));
    assert!(((b / a) - (attack(&app) / b)).abs() < 1e-3, "each step the same factor");
    app.surface_act(A::Lfo("attack", 1, 1000.0));
    assert_eq!(attack(&app), ENV_MAX);
    app.surface_act(A::Lfo("attack", 1, -1000.0));
    assert!((attack(&app) - ENV_MIN).abs() < 1e-7);
    assert_eq!(app.surface_view().lfos[0].attack, "1 ms");
    app.surface_act(A::Lfo("release", 1, 100.0));
    assert_eq!(app.surface_view().lfos[0].release, "10.00 s");
    app.surface_act(A::Lfo("sustain", 1, -3.0));
    assert!((app.surface_view().lfos[0].sustain - 0.47).abs() < 1e-6);
    app.surface_act(A::Lfo("sustain", 1, 500.0));
    assert_eq!(app.surface_view().lfos[0].sustain, 1.0);
}

/// In Envelope mode the trigger is never turned off: TRIG with 0 keeps it, and stepping
/// the Trigger knob skips Off.
#[test]
fn an_envelopes_trigger_never_goes_off() {
    let mut app = app();
    app.surface_act(A::Lfo("create", 1, 0.0));
    app.surface_act(A::Lfo("env", 1, 0.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "A1 / S1");
    app.surface_act(A::Lfo("trigger_focused", 1, 0.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "A1 / S1", "kept");
    app.surface_act(A::Lfo("trigger", 1, -5.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "A1 / S1", "no Off");
    app.surface_act(A::Lfo("trigger", 1, 1.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "A1 / S2");
    app.surface_act(A::FocusSound(4));
    app.surface_act(A::Lfo("trigger_focused", 1, 1.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "A1 / S5");
}

/// An envelope whose trigger is gone shows "None".
#[test]
fn an_envelope_without_a_trigger_shows_none() {
    use deckard_model::lfo::Lfo;
    let mut app = app();
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::SetLfo(Lfo { env: true, ..Lfo::new(1) })]);
    assert_eq!(app.surface_view().lfos[0].trigger, "None");
}

/// A new project with LFO 1, on default settings with `marked` output channels CV only.
fn cv_app(marked: &[usize]) -> crate::App {
    let mut app = app();
    app.settings = crate::settings::Settings::default();
    for &i in marked {
        app.settings.output_cv_only[i] = true;
    }
    app.surface_act(A::Lfo("create", 1, 0.0));
    app
}

fn cv_routes(app: &crate::App) -> Vec<CvRoute> {
    app.doc.as_ref().unwrap().borrow().project().cv_routes.clone()
}

fn set_cv_routes(app: &crate::App, routes: Vec<CvRoute>) {
    app.doc.as_ref().unwrap().borrow_mut().set_cv_routes(routes).unwrap();
}

const NONE: &str = "—";

/// Group stepped past MIDI is CV: OUT makes the route, at Low 0%, High 50%.
#[test]
fn a_cv_slot_shows_its_rows() {
    let mut app = cv_app(&[2, 3]);
    app.surface_act(A::ModSlot(0, 0, 4));
    app.surface_act(A::ModSlot(0, 1, 2));
    assert!(app.mod_slot_views()[0].midi, "MIDI first");
    app.surface_act(A::ModSlot(0, 1, 1));
    let v = app.mod_slot_views();
    assert!(v[0].cv && !v[0].midi);
    assert_eq!(v[0].rows, ["LFO 1", "CV", NONE, "", NONE, NONE].map(String::from));
    assert!(cv_routes(&app).is_empty(), "no route without an OUT");
    app.surface_act(A::ModSlot(0, 2, 1));
    assert_eq!(cv_routes(&app), vec![CvRoute { low: 0.0, high: 0.5, ..CvRoute::new(ModSource::Lfo(1), 2) }]);
    let v = app.mod_slot_views();
    assert_eq!(v[0].rows, ["LFO 1", "CV", "Out 2 L", "", "0%", "50%"].map(String::from));
    assert!(v[0].cv && !v[0].midi);
    assert_eq!(v.len(), 2, "then the empty slot");
}

#[test]
fn a_cv_slots_out_row_offers_only_cv_channels() {
    let mut app = cv_app(&[2, 3]);
    set_cv_routes(&app, vec![CvRoute::new(ModSource::Lfo(1), 2)]);
    let out = |app: &crate::App| cv_routes(app)[0].channel;
    app.surface_act(A::ModSlot(0, 2, 1));
    assert_eq!(out(&app), 3);
    app.surface_act(A::ModSlot(0, 2, 1));
    assert_eq!(out(&app), 3, "stops at the last");
    app.surface_act(A::ModSlot(0, 2, -5));
    assert_eq!(out(&app), 2, "stops at the first");
}

/// A CV route's source turned to None removes it, and ERASE does too.
#[test]
fn turning_source_to_none_removes_a_cv_route() {
    let mut app = cv_app(&[2]);
    set_cv_routes(&app, vec![CvRoute::new(ModSource::Lfo(1), 2)]);
    app.surface_act(A::ModSlot(0, 0, -1));
    assert!(cv_routes(&app).is_empty());
    set_cv_routes(&app, vec![CvRoute::new(ModSource::Lfo(1), 2)]);
    app.surface_act(A::ModSlot(0, 3, 0));
    assert!(cv_routes(&app).is_empty(), "ERASE");
}

#[test]
fn low_steps_below_zero_to_minus_100() {
    let mut app = cv_app(&[2]);
    set_cv_routes(&app, vec![CvRoute { low: 0.0, high: 0.5, ..CvRoute::new(ModSource::Lfo(1), 2) }]);
    app.surface_act(A::ModSlot(0, 4, -3));
    assert!((cv_routes(&app)[0].low + 0.15).abs() < 1e-6, "{:?}", cv_routes(&app));
    assert_eq!(app.mod_slot_views()[0].rows[4], "−15%");
    app.surface_act(A::ModSlot(0, 4, -100));
    assert_eq!(cv_routes(&app)[0].low, -1.0);
    assert_eq!(app.mod_slot_views()[0].rows[4], "−100%");
    app.surface_act(A::ModSlot(0, 5, 100));
    assert_eq!(cv_routes(&app)[0].high, 1.0, "High up to 100%");
}

/// An empty mask never offers CV: past MIDI there's nothing.
#[test]
fn with_no_cv_channels_the_group_row_skips_cv() {
    let mut app = cv_app(&[]);
    app.surface_act(A::ModSlot(0, 0, 4));
    app.surface_act(A::ModSlot(0, 1, 5));
    let v = app.mod_slot_views();
    assert!(v[0].midi && !v[0].cv, "{:?}", v[0]);
    app.surface_act(A::ModSlot(0, 1, 1));
    let v = app.mod_slot_views();
    assert!(v[0].midi && !v[0].cv, "{:?}", v[0]);
}

/// A route on an unmarked channel, or on one an earlier route has, writes nothing.
#[test]
fn an_inactive_route_says_so() {
    let app = cv_app(&[4]);
    set_cv_routes(&app, vec![CvRoute::new(ModSource::Lfo(1), 2)]);
    assert_eq!(app.mod_slot_views()[0].rows[2], "Out 2 L !", "unmarked");
    set_cv_routes(&app, vec![CvRoute::new(ModSource::Lfo(1), 4), CvRoute::new(ModSource::Lfo(1), 4)]);
    let v = app.mod_slot_views();
    assert_eq!((v[0].rows[2].as_str(), v[1].rows[2].as_str()), ("Out 3 L", "Out 3 L !"), "the first keeps it");
}

/// Between kinds: a sound route moved to CV keeps its range, in one undo step; a CV route
/// turned back to MIDI and forward is itself; its first CC moves it there; a CC route
/// goes on to CV. Each in its column.
#[test]
fn a_cv_route_moves_between_kinds() {
    use deckard_model::mod_matrix::{CcRoute, ModParam, ModRoute};
    let mut app = cv_app(&[2]);
    app.midi_ports.1 = vec!["Port A".into()];
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
    let route = ModRoute { low: 0.2, high: 0.7, ..ModRoute::new(ModSource::Lfo(1), 0, "Sampler", ModParam::Field("cutoff".into())) };
    doc.borrow_mut().set_mod_routes(0, 0, vec![route.clone()]).unwrap();
    let sound = |app: &crate::App| app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].mod_routes.clone();
    let ccs = |app: &crate::App| app.doc.as_ref().unwrap().borrow().project().cc_routes.clone();
    let cv = CvRoute { low: 0.2, high: 0.7, ..CvRoute::new(ModSource::Lfo(1), 2) };
    // (One group: two steps on from A1 is past MIDI.)
    app.surface_act(A::ModSlot(0, 1, 2));
    assert!(app.mod_slot_views()[0].cv);
    assert_eq!(sound(&app), vec![route.clone()], "kept until an Out");
    app.surface_act(A::ModSlot(0, 2, 1));
    assert_eq!((sound(&app), cv_routes(&app)), (vec![], vec![cv.clone()]));
    assert_eq!(app.mod_slot_views().len(), 2);
    assert!(doc.borrow_mut().undo());
    assert_eq!((sound(&app), cv_routes(&app)), (vec![route], vec![]), "one undo step");
    assert!(doc.borrow_mut().redo());
    // Back to MIDI and forward: the route again.
    app.surface_act(A::ModSlot(0, 1, -1));
    assert!(app.mod_slot_views()[0].midi);
    assert_eq!(cv_routes(&app), vec![cv.clone()], "kept until a CC");
    app.surface_act(A::ModSlot(0, 1, 1));
    assert!(app.mod_draft.is_none());
    assert!(app.mod_slot_views()[0].cv);
    // Back to MIDI and a CC: moved, its range kept.
    app.surface_act(A::ModSlot(0, 1, -1));
    app.surface_act(A::ModSlot(0, 4, 5));
    assert_eq!(ccs(&app), vec![CcRoute { low: 0.2, high: 0.7, ..CcRoute::new(ModSource::Lfo(1), "Port A", 1, 5) }]);
    assert!(cv_routes(&app).is_empty());
    // A CC route on to CV.
    app.surface_act(A::ModSlot(0, 1, 1));
    assert!(app.mod_slot_views()[0].cv);
    app.surface_act(A::ModSlot(0, 2, 1));
    assert!(ccs(&app).is_empty());
    assert_eq!(cv_routes(&app), vec![cv]);
    assert_eq!(app.mod_slot_views().len(), 2, "in its column");
}

/// One owner a channel: Out passes over those other routes have, keeping the route's own.
/// With none free a new route's Out does nothing, and the draft stays.
#[test]
fn a_cv_slots_out_row_skips_a_taken_channel() {
    let mut app = cv_app(&[2, 3, 4]);
    let on = |c| CvRoute::new(ModSource::Lfo(1), c);
    set_cv_routes(&app, vec![on(2), on(3)]);
    app.surface_act(A::ModSlot(0, 2, 1));
    assert_eq!(cv_routes(&app)[0].channel, 4, "past the one on 3");
    app.surface_act(A::ModSlot(0, 2, -1));
    assert_eq!(cv_routes(&app)[0].channel, 2, "its own stays offered, 3 passed over");
    app.surface_act(A::ModSlot(1, 2, -1));
    assert_eq!(cv_routes(&app)[1].channel, 3, "2 is taken: it stays");
    // A new route's first Out: the one left.
    app.surface_act(A::ModSlot(2, 0, 4));
    app.surface_act(A::ModSlot(2, 1, 3));
    app.surface_act(A::ModSlot(2, 2, 1));
    assert_eq!(cv_routes(&app)[2].channel, 4);
    // None left: no CV to step to.
    app.surface_act(A::ModSlot(3, 0, 4));
    app.surface_act(A::ModSlot(3, 1, 3));
    let v = app.mod_slot_views();
    assert!(v[3].midi && !v[3].cv, "{:?}", v[3]);
    assert_eq!(cv_routes(&app).len(), 3);
}

/// Every CV only channel taken, one by a route and one by a Zhora's output: Group past
/// MIDI stays MIDI, with no empty CV choice.
#[test]
fn with_every_cv_channel_taken_the_group_row_skips_cv() {
    use crate::layout::{Act, AddWhat, ChainTab, native_entry};
    use deckard_plugins::zhora;
    let mut app = cv_app(&[2, 3]);
    set_cv_routes(&app, vec![CvRoute::new(ModSource::Lfo(1), 2)]);
    (app.group, app.sound, app.chain_tab) = (0, 0, ChainTab::Sound);
    app.add_slot(AddWhat::Plugin(native_entry(&zhora::CLASS)));
    let pitch_on_3 = Act::PluginParamIn(deckard_doc::Chain::Sound(0, 0), 0, zhora::PITCH_OUT, 4.0 / 32.0);
    crate::layout::apply_acts(&mut app, vec![pitch_on_3]);
    assert_eq!(crate::zhora_ui::channels_held(app.doc.as_ref().unwrap().borrow().project()), vec![3]);
    app.surface_act(A::ModSlot(1, 0, 4));
    app.surface_act(A::ModSlot(1, 1, 3));
    let v = app.mod_slot_views();
    assert!(v[1].midi && !v[1].cv, "{:?}", v[1]);
    // A free channel again: CV.
    set_cv_routes(&app, Vec::new());
    app.surface_act(A::ModSlot(0, 1, 1));
    assert!(app.mod_slot_views()[0].cv);
}

/// A strip's output or aux on a CV only side: its flag names the side and says why.
#[test]
fn audio_on_a_cv_only_output_is_flagged() {
    let app = cv_app(&[2]);
    let mut p = app.doc.as_ref().unwrap().borrow().project().clone();
    p.groups[0].channel.dest = AudioDest::Ext(1);
    let flag = |p: &deckard_model::Project, ch| strip_cv_flag(&app.settings, p, ch);
    let said = flag(&p, Channel::Group(0)).expect("flagged");
    assert!(said.contains("Out 2 L") && said.contains("CV only"), "{said}");
    assert!(!said.contains("Out 2 R"), "only the marked side: {said}");
    assert_eq!(flag(&p, Channel::Sound(0, 0)), None, "on its parent");
    p.groups[0].sounds[0].channel.aux_dest[1] = AudioDest::Ext(1);
    assert!(flag(&p, Channel::Sound(0, 0)).is_some(), "an aux too");
}

/// The master on a marked output: what's said after PLAY names the output.
#[test]
fn a_silent_master_says_why() {
    let app = cv_app(&[2]);
    let mut p = app.doc.as_ref().unwrap().borrow().project().clone();
    p.master_output = AudioDest::Ext(0);
    assert_eq!(silent_master_note(&app.settings, &p), None);
    p.master_output = AudioDest::Ext(1);
    let said = silent_master_note(&app.settings, &p).expect("said");
    assert!(said.contains("master") && said.contains("Out 2 L"), "{said}");
    assert!(strip_cv_flag(&app.settings, &p, Channel::Master).is_some(), "the strip too");
}

/// A strip on a pair with a CV only side has no stem; one on another pair still does.
#[test]
fn a_cv_only_output_has_no_stem() {
    use crate::export::{Settings, stems};
    use deckard_engine::export::Stem;
    use deckard_model::extra_out::{Channels, ExtraOut, Owner};
    let app = cv_app(&[]);
    let mut p = app.doc.as_ref().unwrap().borrow().project().clone();
    let strip = |n: i64| ExtraOut {
        owner: Owner::Sound(0, 0),
        bus: n as usize,
        plugin: "x".into(),
        name: String::new(),
        level: 1.0,
        pan: 0.0,
        muted: false,
        dest: AudioDest::Ext(n),
        channels: Channels::Stereo,
    };
    p.master_output = AudioDest::Ext(0);
    p.extra_outs = vec![strip(1), strip(2)];
    let s = Settings::default();
    assert_eq!(stems(&p, &s, 0), [Stem::Master, Stem::Extra(0), Stem::Extra(1)]);
    assert_eq!(stems(&p, &s, 1 << 3), [Stem::Master, Stem::Extra(1)], "Out 2 R marked: the pair has no stem");
}
