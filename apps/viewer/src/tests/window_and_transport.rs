use super::*;

/// Soloing a plug-in's extra-output strip mutes the other groups and their strips,
/// and its owner (its main pair; the plug-in keeps playing for the strip, the engine's
/// part), as one step; again, unsolo. Soloing the owner leaves its strips heard.
#[test]
fn soloing_a_strip_mutes_the_rest_but_not_its_plugin() {
    use deckard_model::AudioDest;
    use deckard_model::extra_out::{ExtraOut, Owner};
    let mut app = App { settings: settings::Settings::default(), ..App::default() };
    app.new_project();
    let doc = app.doc.clone().unwrap();
    while doc.borrow().project().groups.len() < 3 {
        doc.borrow_mut().add_group().unwrap();
    }
    // Real plug-ins in the strips' slots (undo drops strips whose plug-in isn't there).
    const FX: &str = "T:VST3:FX:p";
    const INSTR: &str = "T:VST3:INSTR:p";
    let plugin = |id: &str| deckard_doc::NewModule::Plugin { id: id.into(), name: "P".into(), chunk: Vec::new() };
    for c in [deckard_doc::Chain::Group(0), deckard_doc::Chain::Group(1), deckard_doc::Chain::Master] {
        doc.borrow_mut().add_slot_in(c, 0, &plugin(FX)).unwrap();
    }
    doc.borrow_mut().add_slot(2, 0, 0, &plugin(INSTR)).unwrap();
    let x = |owner: Owner, bus: usize| ExtraOut {
        owner,
        bus,
        plugin: (if matches!(owner, Owner::Sound(..)) { INSTR } else { FX }).into(),
        name: "Bus".into(),
        level: 1.0,
        pan: 0.0,
        muted: false,
        dest: AudioDest::Ext(1),
        channels: Default::default(),
    };
    doc.borrow_mut()
        .set_extra_outs(vec![x(Owner::Group(0, 0), 1), x(Owner::Group(0, 0), 2), x(Owner::Group(1, 0), 1), x(Owner::Sound(2, 0), 1)])
        .unwrap();
    let state = |app: &App| {
        let d = app.doc.as_ref().unwrap().borrow();
        let p = d.project();
        (p.groups.iter().map(|g| g.muted).collect::<Vec<_>>(), p.extra_outs.iter().map(|x| x.muted).collect::<Vec<_>>())
    };
    let sound_20 = |app: &App| app.doc.as_ref().unwrap().borrow().project().groups[2].sounds[0].muted;
    app.solo(layout::Channel::Extra(0));
    assert_eq!(
        state(&app),
        (vec![true, true, true], vec![false, true, true, true]),
        "the owner's main pair and every other strip, a sound's in another group too"
    );
    doc.borrow_mut().undo();
    assert_eq!(state(&app), (vec![false; 3], vec![false; 4]), "one step");
    app.solo(layout::Channel::Extra(0));
    app.solo(layout::Channel::Extra(0));
    assert_eq!(state(&app), (vec![false; 3], vec![false; 4]), "again: unsolo");
    // A group soloed: the other groups' strips go quiet, its sounds' strips in other
    // groups too (they play whatever their group's mute); its own stay heard.
    app.solo(layout::Channel::Group(0));
    assert_eq!(state(&app), (vec![false, true, true], vec![false, false, true, true]), "the owner's strips stay heard");
    app.solo(layout::Channel::Group(0));
    app.solo(layout::Channel::Group(2));
    assert_eq!(state(&app), (vec![true, true, false], vec![true, true, true, false]), "its sounds' strips stay heard");
    app.solo(layout::Channel::Group(2));
    // A sound's strip soloed: the other groups and that group's sounds (its own too)
    // muted, the group it plays into heard.
    app.solo(layout::Channel::Extra(3));
    assert_eq!(state(&app), (vec![true, true, false], vec![true, true, true, false]));
    assert!(sound_20(&app), "its sound's main pair");
    app.solo(layout::Channel::Extra(3));
    assert_eq!(state(&app), (vec![false; 3], vec![false; 4]));
    assert!(!sound_20(&app));
    // A master's plug-in's strip: its plug-in hears the mix, so no group is muted;
    // only the other strips are.
    let mut outs = doc.borrow().project().extra_outs.clone();
    outs.push(x(Owner::Master(0), 1));
    doc.borrow_mut().set_extra_outs(outs).unwrap();
    app.solo(layout::Channel::Extra(4));
    assert_eq!(state(&app), (vec![false; 3], vec![true, true, true, true, false]), "the groups feed the master");
    app.solo(layout::Channel::Extra(4));
    assert_eq!(state(&app), (vec![false; 3], vec![false; 5]), "unsolo");
}

/// The extra outputs' meters, peaks and headroom are held by place in the list: a
/// strip added at the end keeps the others', one taken away starts them all again.
#[test]
fn extra_meters_start_again_when_the_strips_change() {
    use deckard_model::AudioDest;
    use deckard_model::extra_out::{ExtraOut, Owner};
    let mut app = App { settings: settings::Settings::default(), ..App::default() };
    app.new_project();
    let doc = app.doc.clone().unwrap();
    let x = |bus| ExtraOut {
        owner: Owner::Group(0, 0),
        bus,
        plugin: "p".into(),
        name: String::new(),
        level: 1.0,
        pan: 0.0,
        muted: false,
        dest: AudioDest::Ext(1),
        channels: Default::default(),
    };
    doc.borrow_mut().set_extra_outs(vec![x(1), x(2)]).unwrap();
    app.sync_extra_meters();
    let now = std::time::Instant::now();
    app.extra_meters = vec![(0.5, 0.5), (0.25, 0.25)];
    for i in 0..2 {
        app.peaks.insert(layout::Channel::Extra(i), [(0.5, now); 2]);
        app.headroom.insert(layout::Channel::Extra(i), 0.5);
    }
    app.headroom.insert(layout::Channel::Master, 0.7);
    doc.borrow_mut().set_extra_outs(vec![x(1), x(2), x(3)]).unwrap();
    app.sync_extra_meters();
    assert_eq!(app.extra_meters, vec![(0.5, 0.5), (0.25, 0.25), (0.0, 0.0)], "added at the end");
    assert_eq!(app.headroom.len(), 3);
    doc.borrow_mut().set_extra_outs(vec![x(2), x(3)]).unwrap();
    app.sync_extra_meters();
    assert_eq!(app.extra_meters, vec![(0.0, 0.0); 2]);
    assert!(!app.peaks.keys().any(|c| matches!(c, layout::Channel::Extra(_))));
    assert_eq!(app.headroom.keys().collect::<Vec<_>>(), vec![&layout::Channel::Master], "the channels' are kept");
}

/// A value sent to a plug-in shows until its controller reports it (JUCE's take it
/// in at the next block), or for SENT_HOLD; then what the plug-in says again.
#[test]
fn a_sent_plugin_value_shows_until_the_plugin_reports_it() {
    let t0 = std::time::Instant::now();
    let text = |id: u32, v: f64| format!("{id}:{v:.1}");
    let mut sent = vec![(7, 0.9, t0)];
    let mut read = vec![(0.5, "old".to_string()), (0.2, "other".to_string())];
    hold_sent(&mut read, &[7, 8], &mut sent, t0, text);
    assert_eq!(read[0], (0.9, "7:0.9".to_string()), "the stale read gives way");
    assert_eq!(read[1].1, "other");
    assert_eq!(sent.len(), 1);
    // The plug-in has it: the hold ends.
    let mut read = vec![(0.9, "new".to_string()), (0.2, "other".to_string())];
    hold_sent(&mut read, &[7, 8], &mut sent, t0, text);
    assert_eq!(read[0].1, "new");
    assert!(sent.is_empty());
    // One it never takes shows what it says after the hold.
    let mut sent = vec![(7, 0.9, t0)];
    let mut read = vec![(0.5, "old".to_string())];
    hold_sent(&mut read, &[7], &mut sent, t0 + SENT_HOLD * 2, text);
    assert_eq!(read[0].1, "old");
    assert!(sent.is_empty());
}

/// The Jam's VARIATION page's Apply reaches the app: the sound's pattern gains notes
/// where it had none, which is what the rig found missing (work-item 35).
#[test]
fn the_jams_apply_variation_reaches_the_app() {
    let mut h = blank_harness(false);
    let doc = h.state().doc.clone().expect("a project");
    {
        let mut d = doc.borrow_mut();
        d.add_pattern(0, 0, "P").expect("a pattern");
        let notes: Vec<deckard_model::Note> = (0..4)
            .map(|i| deckard_model::Note {
                ratchet: Default::default(),
                expr: None,
                sound: 0,
                position: i * 960,
                length: 240,
                pitch: 60,
                velocity: 100,
                probability: 100,
            })
            .collect();
        d.edit_notes("Seed", 0, 0, &[], &notes).expect("seeded");
        for (field, value) in [("mode", 1.0), ("probability", 100.0)] {
            d.set_groove_setting(0, 0, &format!("sequence_variation/{field}"), value).expect("the page's setting");
        }
    }
    let count = |d: &deckard_doc::Document| d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).count();
    assert_eq!(count(&doc.borrow()), 4, "four to the bar");
    h.state_mut().surface_act(deckard_surface::Action::ApplyVariation);
    h.run_steps(2);
    let after = count(&doc.borrow());
    assert!(after > 4, "the empty steps fill: {after} notes, status {:?}", h.state().status);
}

#[test]
fn clicking_a_scene_tab_focuses_it() {
    let Some(mut h) = harness(false, false) else { return };
    click(&mut h, scene_tab(3));
    assert_eq!(h.state().scene, 3);
    click(&mut h, scene_tab(1));
    assert_eq!(h.state().scene, 1);
}

#[test]
fn clicking_a_scene_tab_with_the_controller_focuses_it() {
    let Some(mut h) = scripted_harness(false) else { return };
    assert!(matches!(h.state().controller, Some(Ok(_))), "controller scripts should load");
    click(&mut h, scene_tab(3));
    assert_eq!(h.state().scene, 3);
    h.run_steps(10);
    assert_eq!(h.state().scene, 3, "the selection sticks");
}

#[test]
fn a_scene_clicked_while_playing_is_queued_then_plays() {
    let Some(mut h) = harness(true, false) else { return };
    h.state_mut().transport_play();
    if !matches!(h.state().output, Some(Ok(_))) {
        eprintln!("no audio output; skipping");
        return;
    }
    assert_eq!(h.state().playing, Some(Mode::Scene(0)));
    click(&mut h, scene_tab(3));
    assert_eq!(h.state().scene, 3);
    // The playhead is what the audio thread published after its next block.
    let wait = std::time::Instant::now();
    let mut head = h.state().playhead();
    while head.as_ref().is_none_or(|p| p.queued.is_none()) && wait.elapsed() < AUDIO_PATIENCE {
        std::thread::sleep(std::time::Duration::from_millis(5));
        head = h.state().playhead();
    }
    let head = head.expect("playing");
    assert_eq!((head.mode, head.queued), (Mode::Scene(0), Some(Mode::Scene(3))));
    h.run_steps(10);
    assert_eq!(h.state().scene, 3, "the selection sticks while the old scene finishes");
    h.state_mut().stop();
}

/// A plug-in chosen with nothing playing reaches the engine played next, its parameter
/// moved on the way, and a new project takes it out.
#[test]
fn a_midi_plug_in_chosen_while_stopped_plays_and_goes_with_the_project() {
    let mut app = App::default();
    app.new_project();
    app.set_route(layout::RouteSet::GroupMidiInput(0, Some(deckard_engine::note_echo::NAME)));
    let Some(Ok(out)) = &app.output else {
        eprintln!("no audio output; skipping");
        return;
    };
    let info = out.midi_input(0).expect("on the group");
    assert_eq!(info.name, deckard_engine::note_echo::NAME);
    assert_eq!(out.set_midi_input_param(0, 2, 99.0), Some(8.0), "fitted to its range");
    assert_eq!(out.midi_input(0).unwrap().params[1].value, 8.0);
    assert_eq!(out.set_midi_input_param(0, 99, 1.0), None);

    app.transport_play();
    let Some(Ok(out)) = &app.output else { unreachable!() };
    assert_eq!(out.call(|e| e.has_midi_input(0)), Some(true));

    app.new_project();
    let Some(Ok(out)) = &app.output else { unreachable!() };
    assert!(out.midi_input(0).is_none());
}

/// #14: with the window covered (eframe runs only `logic`), an edit from the MK3
/// (TEMPO and the jog wheel) still reaches the song playing.
#[test]
fn edits_reach_the_song_while_the_window_is_covered() {
    let mut app = App::default();
    app.new_project();
    app.transport_play();
    if !matches!(app.output, Some(Ok(_))) {
        eprintln!("no audio output; skipping");
        return;
    }
    app.surface_act(deckard_surface::Action::Tempo(171.0));
    let ctx = egui::Context::default();
    let wait = std::time::Instant::now();
    let mut tempo = 0.0;
    while tempo != 171.0 && wait.elapsed() < AUDIO_PATIENCE {
        app.background(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
        tempo = app.output.as_ref().unwrap().as_ref().unwrap().telemetry(|t| t.tempo);
    }
    app.stop();
    assert_eq!(tempo, 171.0);
}

/// #33: STOP leaves the song where it was (shown there), PLAY goes on from there,
/// RESTART goes back to the start.
#[test]
fn stop_keeps_the_position_and_play_continues() {
    let mut app = App::default();
    app.new_project();
    app.transport_play();
    if !matches!(app.output, Some(Ok(_))) {
        eprintln!("no audio output; skipping");
        return;
    }
    let ctx = egui::Context::default();
    let at = |app: &mut App, within: std::ops::Range<f64>| {
        published(app);
        let wait = std::time::Instant::now();
        let mut pos = 0.0;
        while !within.contains(&pos) && wait.elapsed() < AUDIO_PATIENCE {
            app.background(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
            pos = app.playhead().map_or(0.0, |h| h.position);
        }
        pos
    };
    let quarter = deckard_model::PPQ as f64;
    // Early in the bar: going on from near its end could come round the loop to the
    // start before the position is looked at.
    assert!(at(&mut app, quarter + 1.0..2.0 * quarter) > quarter, "playing");
    app.stop();
    let parked = app.shown_playhead().expect("shown where it stopped").position;
    assert!(parked > quarter, "{parked}");
    // The MK3 (the touch strip's play position) shows it there too.
    assert_eq!(app.surface_view().position, Some(parked as i64));
    app.transport_play();
    let first = at(&mut app, f64::MIN_POSITIVE..f64::MAX);
    assert!(first >= parked - 1.0, "on from {parked}: {first}");
    app.stop();
    app.restart();
    let first = at(&mut app, f64::MIN_POSITIVE..f64::MAX);
    app.stop();
    assert!(first < parked, "RESTART: from the start ({first})");
}

/// SHIFT + REC counts in from the start (of the scene, or the focused section), not
/// from where STOP left the song.
#[test]
fn a_count_in_starts_from_the_beginning() {
    let mut app = App::default();
    app.new_project();
    app.transport_play();
    if !matches!(app.output, Some(Ok(_))) {
        eprintln!("no audio output; skipping");
        return;
    }
    let ctx = egui::Context::default();
    let quarter = deckard_model::PPQ as f64;
    let wait = std::time::Instant::now();
    while app.playhead().map_or(0.0, |h| h.position) <= quarter && wait.elapsed() < AUDIO_PATIENCE {
        app.background(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    app.stop();
    let parked = app.shown_playhead().expect("shown where it stopped").position;
    assert!(parked > quarter, "{parked}");
    app.set_recording(true, true);
    app.background(&ctx);
    published(&app);
    let first = app.playhead().map_or(0.0, |h| h.position);
    app.set_recording(false, false);
    app.stop();
    assert!(first < quarter, "counted in from the start, not {parked}: {first}");
}

#[test]
fn finishing_a_grown_take_starts_at_the_first_occupied_bar() {
    let mut app = App::default();
    app.new_project();
    app.settings.pattern_grow = true;
    app.set_recording(true, false);
    let doc = app.doc.clone().unwrap();
    let bar = doc.borrow().project().bar_length();
    let first = 2 * bar + bar / 4;
    {
        let mut d = doc.borrow_mut();
        d.set_pattern_length(0, 0, 6 * bar).unwrap();
        d.edit_notes(
            "Record",
            0,
            0,
            &[],
            &[
                deckard_model::Note {
                    ratchet: Default::default(),
                    position: first,
                    length: bar / 2,
                    sound: 0,
                    pitch: 60,
                    velocity: 100,
                    probability: 100,
                    expr: None,
                },
                deckard_model::Note {
                    ratchet: Default::default(),
                    position: 4 * bar,
                    length: bar,
                    sound: 1,
                    pitch: 62,
                    velocity: 100,
                    probability: 100,
                    expr: None,
                },
            ],
        )
        .unwrap();
    }
    app.set_recording(false, false);
    let d = doc.borrow();
    let pat = &d.project().groups[0].patterns[0];
    assert_eq!((pat.start, pat.length), (2 * bar, 3 * bar));
    assert_eq!(pat.notes[0].position, first, "events keep their timing; playback span moves");
}

#[test]
fn first_fixed_length_recording_skips_empty_bars_and_overdubs_keep_them() {
    for overdub in [false, true] {
        let mut app = App::default();
        app.new_project();
        app.settings.pattern_grow = false;
        let doc = app.doc.clone().unwrap();
        {
            let mut d = doc.borrow_mut();
            d.add_pattern(0, 0, "Pattern 1").unwrap();
            d.assign_pattern(0, 0, Some(0)).unwrap();
        }
        let bar = doc.borrow().project().bar_length();
        let note = deckard_model::Note {
            ratchet: Default::default(),
            position: bar + bar / 4,
            length: bar / 4,
            sound: 0,
            pitch: 60,
            velocity: 100,
            probability: 100,
            expr: None,
        };
        {
            let mut d = doc.borrow_mut();
            d.set_pattern_length(0, 0, 4 * bar).unwrap();
            d.set_length_auto(0, 0, false);
            if overdub {
                d.edit_notes("seed", 0, 0, &[], &[note]).unwrap();
            }
        }
        app.set_recording(true, false);
        if !overdub {
            app.take_in_recorded(vec![deckard_engine::Recorded {
                group: 0,
                sound: 0,
                pitch: 60,
                velocity: 100,
                length: bar / 4,
                tick: (bar + bar / 4) as f64,
                played: 0.,
                mode: deckard_engine::Mode::Scene(0),
                midi: false,
                seq: 0,
            }]);
        }
        app.set_recording(false, false);
        let d = doc.borrow();
        let pat = &d.project().groups[0].patterns[0];
        assert_eq!(pat.start, if overdub { 0 } else { bar });
        assert_eq!(pat.length, if overdub { 4 * bar } else { 3 * bar });
    }
}

/// Played (with audio): a new Auto-length pattern after a 4-bar one, recorded into
/// bar after bar, grows for as long as the take goes on: it isn't caught by the song
/// coming round before the new bar reaches the engine.
#[test]
fn a_new_pattern_grows_past_a_long_one_while_playing() {
    let mut app = App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    let bar = doc.borrow().project().bar_length();
    {
        let mut d = doc.borrow_mut();
        d.set_tempo(240.0).unwrap();
        d.add_pattern(0, 0, "Pattern 1").unwrap();
        d.assign_pattern(0, 0, Some(0)).unwrap();
        d.set_pattern_length(0, 0, 4 * bar).unwrap();
        d.set_length_auto(0, 0, false);
        let n = deckard_model::Note {
            ratchet: Default::default(),
            expr: None,
            sound: 1,
            position: 0,
            length: 100,
            pitch: 60,
            velocity: 100,
            probability: 100,
        };
        d.edit_notes("t", 0, 0, &[], &[n]).unwrap();
    }
    (app.group, app.pattern, app.scene, app.sound) = (0, 0, 0, 0);
    layout::apply_acts(&mut app, vec![layout::Act::NewPattern(0, 0)]);
    app.set_recording(true, false);
    app.transport_play();
    if !matches!(app.output, Some(Ok(_))) {
        eprintln!("no audio output; skipping");
        return;
    }
    let ctx = egui::Context::default();
    let len = |app: &App| app.doc.as_ref().unwrap().borrow().project().groups[0].patterns[1].length;
    // A note every beat (240 BPM: a bar is a second) for 7 bars' time. The pattern
    // grows a bar ahead when a poll sees the song in its last bar, so the polls (one a
    // beat here; one a frame in the app) have to come round within that bar; the
    // longest gap between them is measured, for a CI runner that oversleeps.
    let beat = std::time::Duration::from_millis(250);
    let wait = std::time::Instant::now();
    let (mut polled, mut gap) = (std::time::Instant::now(), std::time::Duration::ZERO);
    while wait.elapsed() < beat * 4 * 7 {
        if let Some(Ok(out)) = &app.output {
            out.live(deckard_engine::Live::NoteIn { group: 0, sound: 0, pitch: 60, velocity: 100, thru: false });
        }
        std::thread::sleep(beat / 2);
        if let Some(Ok(out)) = &app.output {
            out.live(deckard_engine::Live::NoteRelease { group: 0, sound: 0, pitch: 60 });
        }
        std::thread::sleep(beat / 2);
        app.background(&ctx);
        app.poll_recorded();
        gap = gap.max(polled.elapsed());
        polled = std::time::Instant::now();
    }
    app.set_recording(false, false);
    app.stop();
    if gap > beat * 2 {
        eprintln!("this machine oversleeps ({gap:?} between polls, for a bar of {:?}); skipping", beat * 4);
        return;
    }
    // About 7 bars played (less what starting takes): kept, past the other's 4.
    assert!(len(&app) >= 6 * bar, "{} bars ({}; longest gap between polls {gap:?})", len(&app) as f64 / bar as f64, app.status);
}

/// BUGS #6: a project reopens where it was saved: the focused sound and slot, the
/// Channel page or the plug-ins, and the controller's page and pad mode.
#[test]
fn a_project_reopens_where_it_was_saved() {
    let mut app = App::default();
    app.new_project();
    layout::apply_acts(&mut app, vec![layout::Act::AddScene]);
    app.doc.as_ref().unwrap().borrow_mut().add_slot(0, 3, 0, &deckard_doc::NewModule::Sampler).unwrap();
    (app.sound, app.slot, app.channel) = (3, Some(0), true);
    app.focus_scene(1);
    let view = app.surface_view();
    for a in app.surface.restore("mixer", "keyboard", &view, std::time::Instant::now()) {
        app.surface_act(a);
    }
    app.store_screen();
    let bytes = app.doc.as_ref().unwrap().borrow().to_bytes().unwrap();
    let mut again = App::default();
    again.show_document(Document::from_bytes(bytes, None));
    assert_eq!((again.sound, again.slot, again.channel, again.scene), (3, Some(0), true, 1));
    assert_eq!(again.surface.screen(), Some(("mixer".to_string(), "keyboard".to_string())));
}

/// Two effects on sound 1, the second focused on its second page, and the Groove
/// set on its last page; sound 2 left on the Groove set's second page.
fn chains_left_somewhere() -> App {
    let mut app = App::default();
    app.new_project();
    {
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        d.add_slot(0, 0, 0, &deckard_doc::NewModule::Effect("Compressor")).unwrap();
        d.add_slot(0, 0, 1, &deckard_doc::NewModule::Effect("Compressor")).unwrap();
    }
    app.update_slot_view();
    layout::apply_acts(&mut app, vec![layout::Act::SelectSlot(Some(1)), layout::Act::PluginPage(1)]);
    app.update_slot_view();
    layout::apply_acts(&mut app, vec![layout::Act::PropSet(1), layout::Act::PropPage(2)]);
    app.update_slot_view();
    app.sound = 1;
    app.update_slot_view();
    layout::apply_acts(&mut app, vec![layout::Act::PropPage(1)]);
    app.update_slot_view();
    app
}

/// Each chain shows where it was left: its slot and that slot's page, and each
/// property set's page, whatever was shown in between.
#[test]
fn each_chain_comes_back_where_it_was_left() {
    let mut app = chains_left_somewhere();
    assert_eq!((app.slot, app.prop_page), (None, 1));
    (app.group, app.sound) = (1, 0);
    app.update_slot_view();
    assert_eq!(app.prop_page, 0);
    (app.group, app.sound) = (0, 0);
    app.update_slot_view();
    assert_eq!((app.slot, app.plugin_page, app.prop_set, app.prop_page), (Some(1), 1, 1, 2));
    // Another set and back: still on its page.
    layout::apply_acts(&mut app, vec![layout::Act::PropSet(0)]);
    app.update_slot_view();
    assert_eq!(app.prop_page, 0);
    layout::apply_acts(&mut app, vec![layout::Act::PropSet(1)]);
    assert_eq!(app.prop_page, 2);
    // The other slot and back: still on its page.
    layout::apply_acts(&mut app, vec![layout::Act::SelectSlot(Some(0))]);
    app.update_slot_view();
    assert_eq!(app.plugin_page, 0);
    layout::apply_acts(&mut app, vec![layout::Act::SelectSlot(Some(1))]);
    app.update_slot_view();
    assert_eq!(app.plugin_page, 1);
    app.sound = 1;
    app.update_slot_view();
    assert_eq!(app.prop_page, 1);
}

/// A saved project reopens on the app's view (Ideas or Song, the Mixer, the chain
/// tab) and with each chain where it was left.
#[test]
fn a_project_reopens_on_its_view_and_where_each_chain_was_left() {
    let mut app = chains_left_somewhere();
    (app.song_view, app.mixer, app.chain_tab) = (true, true, layout::ChainTab::Sound);
    app.store_screen();
    let bytes = app.doc.as_ref().unwrap().borrow().to_bytes().unwrap();
    let mut again = App::default();
    again.show_document(Document::from_bytes(bytes, None));
    again.update_slot_view();
    assert_eq!((again.song_view, again.mixer, again.sound, again.prop_set, again.prop_page), (true, true, 1, 1, 1));
    again.sound = 0;
    again.update_slot_view();
    assert_eq!((again.slot, again.plugin_page, again.prop_page), (Some(1), 1, 2));
}

/// C46: on DECKARD's layer the MK3 keeps a clock of its own, as the scripts' timer
/// does, so a covered window still comes back to it: faster while the song plays.
#[test]
fn deckards_layer_keeps_the_mk3_ticking() {
    let mut app = App::default();
    app.new_project();
    assert_eq!(app.mk3_wake(), Some(controller::tick_every(false)));
    app.playing = Some(Mode::Scene(0));
    assert_eq!(app.mk3_wake(), Some(controller::tick_every(true)));
    app.surface.set_theme("palenight");
    assert_eq!(app.mk3_wake(), Some(controller::tick_every(true)));
    app.playing = None;
    assert_eq!(app.mk3_wake(), Some(std::time::Duration::from_millis(30)));
}

/// C46: with the window covered, the focus still follows the scene playing (the MK3's
/// screens show it).
#[test]
fn scenes_are_followed_while_the_window_is_covered() {
    let mut app = App::default();
    app.new_project();
    layout::apply_acts(&mut app, vec![layout::Act::AddScene]);
    app.focus_scene(0);
    app.transport_play();
    let Some(Ok(out)) = &app.output else {
        eprintln!("no audio output; skipping");
        return;
    };
    out.queue(app.doc.as_ref().unwrap().borrow().project(), Mode::Scene(1));
    let ctx = egui::Context::default();
    let wait = std::time::Instant::now();
    while app.scene != 1 && wait.elapsed() < AUDIO_PATIENCE {
        app.background(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    app.stop();
    assert_eq!(app.scene, 1);
}

/// C46: while covered and playing, the background asks to come back at its pace, never
/// at once (with nothing drawn to wait on, that would spin a core).
#[test]
fn the_background_comes_back_at_its_pace() {
    let mut app = App::default();
    app.new_project();
    app.metronome = true;
    app.transport_play();
    if !matches!(app.output, Some(Ok(_))) {
        eprintln!("no audio output; skipping");
        return;
    }
    let ctx = egui::Context::default();
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = asked.clone();
    ctx.set_request_repaint_callback(move |info| log.lock().unwrap().push(info.delay));
    let wait = std::time::Instant::now();
    while app.master_meter == (0.0, 0.0) && wait.elapsed() < AUDIO_PATIENCE {
        app.background(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    app.stop();
    let asked = asked.lock().unwrap();
    assert!(!asked.is_empty(), "it asks to come back");
    assert!(asked.iter().all(|d| *d >= BACKGROUND_PACE), "{asked:?}");
}

/// With `AutoNoVsync`, a visible playing frame must never schedule an immediate (`0 ms`)
/// repaint on its own (such as the playing scene tab's progress strip), or the UI thread
/// spins unthrottled at 400+ FPS.
#[test]
fn visible_frame_paces_repaints_while_playing() {
    let mut app = App::default();
    app.new_project();
    app.preload_plugins = false;
    if let Some(doc) = &app.doc {
        let mut d = doc.borrow_mut();
        let p = d.add_pattern(0, 0, "Pattern 1").unwrap();
        let _ = play_in_scene(&mut d, 0, 0, p);
    }
    app.metronome = true;
    app.transport_play();
    if !matches!(app.output, Some(Ok(_))) {
        eprintln!("no audio output; skipping");
        return;
    }
    let ctx = egui::Context::default();
    // Settle initial two layout passes (egui requests one immediate pass on new widgets).
    for _ in 0..3 {
        let mut raw = egui::RawInput::default();
        raw.predicted_dt = 0.002;
        ctx.run_ui(raw, |ui| app.frame(ui)).textures_delta.clear();
    }
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = asked.clone();
    ctx.set_request_repaint_callback(move |info| log.lock().unwrap().push(info.delay));
    for _ in 0..4 {
        let mut raw = egui::RawInput::default();
        raw.predicted_dt = 0.002;
        ctx.run_ui(raw, |ui| app.frame(ui)).textures_delta.clear();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    app.stop();
    let asked = asked.lock().unwrap();
    assert!(!asked.is_empty(), "playing frame schedules next tick");
    assert!(asked.iter().all(|d| *d >= std::time::Duration::from_millis(10)), "unexpected unthrottled repaint: {asked:?}");
}

/// #15: with the window covered, the meters (the MK3's Mixer shows them) still move.
#[test]
fn meters_move_while_the_window_is_covered() {
    let mut app = App::default();
    app.new_project();
    app.metronome = true;
    app.transport_play();
    if !matches!(app.output, Some(Ok(_))) {
        eprintln!("no audio output; skipping");
        return;
    }
    let ctx = egui::Context::default();
    let wait = std::time::Instant::now();
    while app.master_meter == (0.0, 0.0) && wait.elapsed() < AUDIO_PATIENCE {
        app.background(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    app.stop();
    assert_ne!(app.master_meter, (0.0, 0.0), "the click metered");
}
