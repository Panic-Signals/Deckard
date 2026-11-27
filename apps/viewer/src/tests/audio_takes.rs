use super::*;

/// Finish a Loop/Take recording on sound `s` of group 0: it becomes an Audio plug-in.
fn audio_sound(h: &mut Harness<'static, App>, s: usize, tone: &deckard_engine::sample::Sample, bar: f64) {
    use layout::sampling::RecParam;
    (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound) = (0, 0, s);
    h.state_mut().recorder.view.set(RecParam::Target, 0);
    h.state_mut().finish_recording(recorder::Finished::Take { sample: tone.clone(), channels: 2, mode: 2, ticks: bar, focus: (0, s) });
}

fn tone_of(secs: f64) -> deckard_engine::sample::Sample {
    deckard_engine::sample::Sample { rate: 48_000, frames: vec![[0.25, 0.25]; (secs * 48_000.0) as usize] }
}

/// A free sound of group 0 made an Audio plug-in with a bar-long take, on in the
/// pattern in focus (no notes of its own there), focused: (its sound, the project's
/// folder, kept while the test runs).
fn loop_sound(h: &mut Harness<'static, App>) -> (usize, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    h.state_mut().path = Some(tmp.path().join("x.mxprj"));
    a_pattern(h);
    let p = project_of(h);
    let bar = p.bar_length() as f64;
    let free = |s: &usize| {
        let snd = &p.groups[0].sounds[*s];
        snd.slots.is_empty() && snd.sampler.is_none() && p.groups[0].patterns[0].notes.iter().all(|n| n.sound != *s)
    };
    let s = (1..16).find(free).expect("a free sound");
    audio_sound(h, s, &tone_of(2.0), bar);
    let pi = h.state().pattern;
    h.state().doc.as_ref().unwrap().borrow_mut().set_audio_in(0, s, &[pi], Some(0), "t").unwrap();
    (h.state_mut().group, h.state_mut().sound, h.state_mut().song_view) = (0, s, false);
    h.run_steps(3);
    assert_eq!(project_of(h).groups[0].sounds[s].instrument, "AudioModule", "{}", h.state().status);
    (s, tmp)
}

/// A drag along grid row `row`, from tick `a` to tick `b` (from the pattern's start).
fn drag_ticks(h: &mut Harness<'static, App>, row: usize, a: i64, b: i64, modifiers: egui::Modifiers) {
    let (from, to) = (grid_pos(h, row, a), grid_pos(h, row, b));
    h.event(Event::ModifiersChanged(modifiers));
    h.hover_at(from);
    h.step();
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers });
    h.step();
    for k in 1..=6 {
        let t = k as f32 / 6.0;
        h.event(Event::PointerMoved(from + (to - from) * t));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers });
    h.step();
    h.event(Event::ModifiersChanged(Default::default()));
    h.run_steps(3);
}

/// The one feature beyond Maschine: a pattern's audio dragged along the grid starts
/// that much later, as one undo step.
#[test]
fn dragging_a_loop_moves_its_audio() {
    let Some(mut h) = harness(false, false) else { return };
    let (s, _tmp) = loop_sound(&mut h);
    assert_eq!(offset_of(&h, s), Some(0));
    drag_ticks(&mut h, s, PPQ / 2, PPQ / 2 + PPQ, Default::default());
    assert_eq!(offset_of(&h, s), Some(PPQ));
    let d = h.state().doc.clone().unwrap();
    assert_eq!(d.borrow().undo_labels().first().copied(), Some("Move Audio"));
    assert!(d.borrow_mut().undo());
    assert_eq!(offset_of(&h, s), Some(0));
}

#[test]
fn a_loop_drag_snaps_unless_shift() {
    let Some(mut h) = harness(false, false) else { return };
    let (s, _tmp) = loop_sound(&mut h);
    let step = h.state().grid;
    let by = step * 13 / 10;
    drag_ticks(&mut h, s, PPQ / 2, PPQ / 2 + by, Default::default());
    assert_eq!(offset_of(&h, s), Some(step));
    drag_ticks(&mut h, s, PPQ / 2, PPQ / 2 + by, egui::Modifiers::SHIFT);
    let got = offset_of(&h, s).unwrap();
    assert!((got - (step + by)).abs() <= 1, "{got} vs {}", step + by);
}

#[test]
fn dragging_a_note_on_a_gate_row_moves_the_note() {
    let Some(mut h) = harness(false, false) else { return };
    let (s, _tmp) = loop_sound(&mut h);
    let pi = h.state().pattern;
    let slot = project_of(&h).groups[0].sounds[s].slots.iter().position(|x| x.module == "AudioModule").unwrap();
    layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(deckard_doc::Target::Module(0, s, slot), "mode", 1.0)]);
    let a = project_of(&h).groups[0].sounds[s].audio.clone().unwrap();
    assert_eq!(a.mode, deckard_model::AudioMode::Gate);
    if offset_of(&h, s).is_none() {
        h.state().doc.as_ref().unwrap().borrow_mut().set_audio_in(0, s, &[pi], Some(0), "t").unwrap();
    }
    let start = project_of(&h).groups[0].patterns[pi].start;
    let n = Note {
        ratchet: Default::default(),
        expr: None,
        sound: s,
        position: start,
        length: PPQ,
        pitch: 60,
        velocity: 100,
        probability: 100,
    };
    h.state().doc.as_ref().unwrap().borrow_mut().edit_notes("t", 0, pi, &[], &[n]).unwrap();
    h.run_steps(3);
    // The note: it moves a beat on, the audio stays.
    // (A little past the beat: a note's move snaps down to the grid.)
    drag_ticks(&mut h, s, PPQ / 4, PPQ / 4 + PPQ + 20, Default::default());
    let notes: Vec<i64> = project_of(&h).groups[0].patterns[pi].notes.iter().filter(|x| x.sound == s).map(|x| x.position).collect();
    assert_eq!(notes, vec![start + PPQ]);
    assert_eq!(offset_of(&h, s), Some(0));
    // Empty space on the row: the audio moves.
    drag_ticks(&mut h, s, 3 * PPQ, 4 * PPQ, Default::default());
    assert_eq!(offset_of(&h, s), Some(PPQ));
}

#[test]
fn the_piano_roll_drags_the_focused_loop() {
    let Some(mut h) = harness(false, false) else { return };
    let (s, _tmp) = loop_sound(&mut h);
    h.state_mut().piano_roll = true;
    h.run_steps(3);
    drag_ticks(&mut h, 2, PPQ / 2, PPQ / 2 + PPQ, Default::default());
    assert_eq!(offset_of(&h, s), Some(PPQ));
}

#[test]
fn a_sync_take_on_an_audio_sound_is_a_new_take() {
    let mut h = blank_harness(true);
    let tmp = tempfile::tempdir().unwrap();
    h.state_mut().path = Some(tmp.path().join("x.mxprj"));
    a_pattern(&mut h);
    let p = project_of(&h);
    let bar = p.bar_length() as f64;
    let s = (1..16).find(|&s| p.groups[0].sounds[s].slots.is_empty() && p.groups[0].sounds[s].sampler.is_none()).unwrap();
    audio_sound(&mut h, s, &tone_of(1.0), bar);
    assert_eq!(project_of(&h).groups[0].sounds[s].audio.as_ref().map(|a| a.takes.len()), Some(1));
    // An effect after the Audio plug-in: a new take leaves it.
    let at = project_of(&h).groups[0].sounds[s].slots.len();
    let fx = deckard_doc::NewModule::Effect(deckard_model::PERFORM_FX.0);
    h.state().doc.as_ref().unwrap().borrow_mut().add_slot(0, s, at, &fx).unwrap();
    let chain: Vec<String> = project_of(&h).groups[0].sounds[s].slots.iter().map(|x| x.module.clone()).collect();
    assert_eq!(chain.len(), at + 1);
    h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 1, ticks: bar, focus: (0, s) });
    let q = project_of(&h);
    let snd = &q.groups[0].sounds[s];
    assert_eq!(snd.slots.iter().map(|x| x.module.clone()).collect::<Vec<_>>(), chain, "the chain survives");
    assert_eq!(snd.instrument, "AudioModule", "{}", h.state().status);
    assert!(snd.sampler.is_none());
    assert_eq!(snd.audio.as_ref().map(|a| (a.takes.len(), a.focused)), Some((2, 1)));
    assert!(q.groups[0].patterns[0].audio.contains(&deckard_model::AudioCue { sound: s, take: 1, offset: 0 }));
}

/// A recorded take on an Audio plug-in is edited as a Sampler's sample is: EDIT (no
/// SLICE or ZONE), its play range set on the take, and TRUNCATE writes a copy the take
/// plays.
#[test]
fn an_audio_take_is_edited_in_the_sampling_view() {
    use layout::sampling::{EditOp, SamplingAct};
    let mut h = blank_harness(true);
    let (s, _tmp) = loop_sound(&mut h);
    h.state_mut().sampling_open = true;
    h.state_mut().sampling.tab = 2;
    h.run_steps(3);
    assert_eq!(h.state().sampling.tab, 1, "no SLICE for a take: EDIT");
    let frames = h.state().sampling.sample.as_ref().map(|x| x.frames.len()).expect("the take comes up");
    let (_, _, t, take) = layout::sampling::edited(&project_of(&h), (0, s, 0)).unwrap();
    assert!(take);
    assert_eq!(t, deckard_doc::Target::Take(0, s, 0));
    // The play range: on the take.
    layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(t, "play_range/low", 100.0)]);
    let take = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[s].audio.as_ref().unwrap().takes[0].clone();
    assert_eq!(take(&h).play_range.0, 100, "{}", h.state().status);
    // TRUNCATE to the first half.
    let before = take(&h).sample.path;
    h.state_mut().sampling.selection = Some((0, frames / 2));
    h.state_mut().sampling_act(SamplingAct::Edit(EditOp::Truncate));
    h.run_steps(3);
    let after = take(&h);
    assert_ne!(after.sample.path, before, "{}", h.state().status);
    assert_eq!(h.state().sampling.sample.as_ref().map(|x| x.frames.len()), Some(frames / 2));
    assert_eq!(project_of(&h).groups[0].sounds[s].instrument, "AudioModule", "still an Audio plug-in");
    // NORMALIZE with nothing selected: the take's play range, all of it.
    let before = take(&h).sample.path;
    h.state_mut().sampling.selection = None;
    h.state_mut().sampling_act(SamplingAct::Edit(EditOp::Normalize));
    h.run_steps(3);
    assert_ne!(take(&h).sample.path, before, "{}", h.state().status);
    let peak = h.state().sampling.sample.as_ref().unwrap().frames.iter().fold(0f32, |m, f| m.max(f[0].abs()));
    assert!(peak > 0.9, "normalized: {peak}");
}

/// Loop, Target Sound, started on an empty sound: the take goes there (not the next
/// one), and the sound takes the recording's name.
#[test]
fn a_loop_take_fills_the_empty_focused_sound_and_names_it() {
    use layout::sampling::RecParam;
    let mut h = blank_harness(true);
    let tmp = tempfile::tempdir().unwrap();
    h.state_mut().path = Some(tmp.path().join("x.mxprj"));
    let p = project_of(&h);
    let bar = p.bar_length() as f64;
    let s = (1..16).find(|&s| p.groups[0].sounds[s].slots.is_empty() && p.groups[0].sounds[s].sampler.is_none()).unwrap();
    (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound) = (0, 0, s);
    h.state_mut().recorder.view.set(RecParam::Target, 1);
    h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 2, ticks: bar, focus: (0, s) });
    let q = project_of(&h);
    let snd = &q.groups[0].sounds[s];
    assert_eq!(snd.instrument, "AudioModule", "the focused sound, not the next: {}", h.state().status);
    let file = h.state().recorder.view.takes.last().unwrap().file_stem().unwrap().to_string_lossy().to_string();
    assert_eq!(snd.name, file, "named after the take");
    assert_eq!(file.len(), "20260101T000000".len());
}

/// DELETE on a take just recorded takes the recording back (as Undo): the sound is as
/// it was. After other edits, the take stops playing (off in its patterns).
#[test]
fn deleting_a_recorded_take_stops_it_playing() {
    use layout::sampling::{RecParam, RecordAct, SamplingAct};
    let mut h = blank_harness(true);
    let tmp = tempfile::tempdir().unwrap();
    h.state_mut().path = Some(tmp.path().join("x.mxprj"));
    a_pattern(&mut h);
    let p = project_of(&h);
    let bar = p.bar_length() as f64;
    let s = (1..16).find(|&s| p.groups[0].sounds[s].slots.is_empty() && p.groups[0].sounds[s].sampler.is_none()).unwrap();
    let record = |h: &mut Harness<'static, App>| {
        (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound) = (0, 0, s);
        h.state_mut().recorder.view.set(RecParam::Target, 0);
        h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 2, ticks: bar, focus: (0, s) });
        assert!(project_of(h).groups[0].patterns[0].audio.iter().any(|c| c.sound == s), "{}", h.state().status);
        h.state().recorder.view.takes.len() - 1
    };
    let delete = |h: &mut Harness<'static, App>, i: usize| h.state_mut().sampling_act(SamplingAct::Record(RecordAct::DeleteTake(i)));
    // Straight after: as if it was never recorded.
    let i = record(&mut h);
    delete(&mut h, i);
    let q = project_of(&h);
    assert_eq!(q.groups[0].sounds[s], p.groups[0].sounds[s], "the sound as it was");
    assert!(q.groups[0].patterns[0].audio.iter().all(|c| c.sound != s));
    assert!(h.state().recorder.view.takes.is_empty());
    // With an edit since: off in the pattern, the edit kept.
    let i = record(&mut h);
    let doc = h.state().doc.clone().unwrap();
    let tempo = doc.borrow().project().tempo + 1.0;
    doc.borrow_mut().set_tempo(tempo).unwrap();
    delete(&mut h, i);
    let q = project_of(&h);
    assert!(q.groups[0].patterns[0].audio.iter().all(|c| c.sound != s), "{}", h.state().status);
    assert_eq!(q.tempo, tempo);
}

/// A 4-bar Loop take into a group without patterns: the pattern made is 4 bars, the
/// take whole (Target Sound and Pattern).
#[test]
fn a_long_loop_take_into_a_group_without_patterns_makes_a_pattern_as_long() {
    use layout::sampling::RecParam;
    let mut h = blank_harness(true);
    let tmp = tempfile::tempdir().unwrap();
    h.state_mut().path = Some(tmp.path().join("x.mxprj"));
    let doc = h.state().doc.clone().unwrap();
    let bar = project_of(&h).bar_length();
    for (target, song_view) in [(1, true), (2, true), (1, false), (2, false)] {
        h.state_mut().song_view = song_view;
        let g = doc.borrow_mut().add_group().unwrap();
        assert!(project_of(&h).groups[g].patterns.is_empty());
        (h.state_mut().group, h.state_mut().sound) = (g, 0);
        h.state_mut().recorder.view.set(RecParam::Target, target);
        h.state_mut().finish_recording(recorder::Finished::Take {
            sample: tone_of(8.0),
            channels: 2,
            mode: 2,
            ticks: 4.0 * bar as f64,
            focus: (g, 0),
        });
        let q = project_of(&h);
        let pats = &q.groups[g].patterns;
        assert_eq!(pats.len(), 1, "target {target}: one pattern ({})", h.state().status);
        assert_eq!(pats[0].length, 4 * bar, "target {target}: 4 bars");
        assert!(pats[0].audio.iter().any(|c| c.sound == 0), "target {target}: on");
        let scene = &q.scenes[h.state().heard_scene()];
        assert!(scene.patterns.contains(&(g, 0)), "target {target}, song {song_view}: in the scene heard: {:?}", scene.patterns);
    }
}

/// A recorded take dropped on another sound's row (or clicked) plays there: an empty
/// sound gets an Audio plug-in with it at its length, on in the pattern; clicking it
/// on its own Audio sound selects it (no copy, no Sampler).
#[test]
fn a_take_dragged_onto_a_sound_plays_there() {
    use layout::sampling::{RecParam, RecordAct, SamplingAct};
    let mut h = blank_harness(true);
    let tmp = tempfile::tempdir().unwrap();
    h.state_mut().path = Some(tmp.path().join("x.mxprj"));
    let p = project_of(&h);
    let bar = p.bar_length() as f64;
    let free: Vec<usize> = (1..16).filter(|&s| p.groups[0].sounds[s].slots.is_empty() && p.groups[0].sounds[s].sampler.is_none()).collect();
    let (s1, s2) = (free[0], free[1]);
    (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound) = (0, 0, s1);
    h.state_mut().recorder.view.set(RecParam::Target, 0);
    h.state_mut().finish_recording(recorder::Finished::Take {
        sample: tone_of(2.0),
        channels: 2,
        mode: 2,
        ticks: 2.0 * bar,
        focus: (0, s1),
    });
    let take = h.state().recorder.view.takes[0].clone();
    // Clicked on its own sound: selected, not copied, still an Audio plug-in.
    h.state_mut().sampling_act(SamplingAct::Record(RecordAct::FocusTake(0)));
    let snd = project_of(&h).groups[0].sounds[s1].clone();
    assert_eq!((snd.instrument.as_str(), snd.audio.as_ref().map(|a| a.takes.len())), ("AudioModule", Some(1)), "{}", h.state().status);
    // Dropped on an empty sound's row.
    h.run_steps(3);
    let row = |h: &Harness<'static, App>, s: usize| -> egui::Rect {
        let rows: Vec<(egui::Rect, (usize, usize))> = h.ctx.data(|d| d.get_temp(egui::Id::new(layout::DROP_ROWS))).expect("sound rows");
        rows.iter().find(|(_, gs)| *gs == (0, s)).expect("the sound's row").0
    };
    let at = row(&h, s2).center();
    h.hover_at(at);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    egui::DragAndDrop::set_payload(&h.ctx, layout::sampling::DraggedTake(take.clone()));
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
    let q = project_of(&h);
    let snd = &q.groups[0].sounds[s2];
    assert_eq!(snd.instrument, "AudioModule", "{}", h.state().status);
    let a = snd.audio.as_ref().unwrap();
    assert!((a.takes[0].beats - 2.0 * bar / PPQ as f64).abs() < 1e-6, "the take's length: {}", a.takes[0].beats);
    assert!(q.groups[0].patterns[0].audio.iter().any(|c| c.sound == s2), "on in the pattern");
    assert!(q.groups[0].patterns[0].audio.iter().any(|c| c.sound == s1), "still on where it was recorded");
    // RECORD shows it as a tile with its waveform.
    assert!(h.state().recorder.view.take_waves.get(&take).is_some_and(|w| !w.is_empty()), "the take's wave");
    (h.state_mut().sampling_open, h.state_mut().sampling.tab) = (true, 0);
    h.run_steps(3);
    assert_eq!(h.state().sampling.tab, 0);
}

/// Recording another take on an Audio sound stops its old take at once; the new take
/// plays when it's done (DELETE brings the old one back); a cancel brings it back too.
#[test]
fn a_new_take_silences_the_old_one_while_it_records() {
    use layout::sampling::{RecParam, RecordAct, SamplingAct};
    let mut h = blank_harness(true);
    let (s, _tmp) = loop_sound(&mut h);
    let bar = project_of(&h).bar_length() as f64;
    h.state_mut().recorder.view.set(RecParam::Mode, 2);
    h.state_mut().recorder.view.set(RecParam::Target, 0);
    let cue =
        |h: &Harness<'static, App>| project_of(h).groups[0].patterns[h.state().pattern].audio.iter().find(|c| c.sound == s).map(|c| c.take);
    assert_eq!(cue(&h), Some(0));
    // Cancelled: the old take plays again.
    h.state_mut().recorder.fake_recording((0, s));
    h.state_mut().silence_while_recording();
    assert_eq!(cue(&h), None, "silent while recording");
    h.state_mut().sampling_act(SamplingAct::Record(RecordAct::Cancel));
    h.state_mut().silence_while_recording();
    assert_eq!(cue(&h), Some(0), "back after a cancel");
    // Finished: the new take plays; DELETE: the old one again.
    h.state_mut().recorder.fake_recording((0, s));
    h.state_mut().silence_while_recording();
    assert_eq!(cue(&h), None);
    h.state_mut().recorder.cancel(None);
    h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 2, ticks: bar, focus: (0, s) });
    h.state_mut().silence_while_recording();
    assert_eq!(cue(&h), Some(1), "{}", h.state().status);
    let i = h.state().recorder.view.takes.len() - 1;
    h.state_mut().sampling_act(SamplingAct::Record(RecordAct::DeleteTake(i)));
    assert_eq!(cue(&h), Some(0), "the old take back");
}

/// A Loop take is heard at once: an Audio plug-in in Gate mode (silent without notes)
/// goes to Loop. Sync onto one keeps its mode.
#[test]
fn a_loop_take_switches_a_gate_audio_plugin_to_loop() {
    let mut h = blank_harness(true);
    let tmp = tempfile::tempdir().unwrap();
    h.state_mut().path = Some(tmp.path().join("x.mxprj"));
    let p = project_of(&h);
    let bar = p.bar_length() as f64;
    let s = (1..16).find(|&s| p.groups[0].sounds[s].slots.is_empty() && p.groups[0].sounds[s].sampler.is_none()).unwrap();
    audio_sound(&mut h, s, &tone_of(1.0), bar);
    let mode = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[s].audio.as_ref().map(|a| a.mode);
    let gate = |h: &mut Harness<'static, App>| {
        let i = project_of(h).groups[0].sounds[s].slots.iter().position(|x| x.module == "AudioModule").unwrap();
        let doc = h.state().doc.clone().unwrap();
        doc.borrow_mut().set("t", deckard_doc::Target::Module(0, s, i), "mode", mxprj::decode::Value::Float(1.0)).unwrap();
    };
    gate(&mut h);
    assert_eq!(mode(&h), Some(deckard_model::AudioMode::Gate));
    h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 1, ticks: bar, focus: (0, s) });
    assert_eq!(mode(&h), Some(deckard_model::AudioMode::Gate), "Sync keeps Gate");
    audio_sound(&mut h, s, &tone_of(1.0), bar);
    assert_eq!(mode(&h), Some(deckard_model::AudioMode::Loop), "{}", h.state().status);
}

/// ASIO wanted but the system's driver standing in (the computer just woke, say):
/// ASIO is tried again a few seconds apart, five times at most, never while the
/// song plays; opening the output afresh starts the count again.
#[test]
fn asio_is_tried_again_five_times_while_a_fallback_plays() {
    use deckard_engine::devices::{ASIO_BUILT, AudioDriver};
    let mut app = App { settings: settings::Settings::default(), ..App::default() };
    app.settings.audio_driver = AudioDriver::Asio;
    app.settings.asio_device = Some("DECKARD test: no such ASIO driver".into());
    let fall_back = |app: &mut App| {
        app.output = Some(Err("no output in tests".into()));
        app.output_fallback = Some("the driver isn't ready".into());
    };
    fall_back(&mut app);
    let tries = |app: &App| app.asio_retry.map(|(_, n)| n);
    let due = |app: &mut App| {
        if let Some((at, _)) = &mut app.asio_retry {
            *at -= ASIO_RETRY_GAP * 2;
        }
    };
    app.retry_asio();
    if !ASIO_BUILT {
        assert_eq!(tries(&app), None, "no ASIO to try");
        return;
    }
    assert_eq!(tries(&app), Some(0), "not at once");
    app.retry_asio();
    assert_eq!(tries(&app), Some(0), "not before the gap");
    app.playing = Some(Mode::Scene(0));
    due(&mut app);
    app.retry_asio();
    assert_eq!(tries(&app), Some(0), "not while the song plays");
    app.playing = None;
    for n in 1..=ASIO_RETRIES {
        due(&mut app);
        app.retry_asio();
        assert_eq!(tries(&app), Some(n));
        assert_eq!(deckard_engine::devices::driver(), AudioDriver::System, "the stand-in's driver again");
    }
    due(&mut app);
    app.retry_asio();
    assert_eq!(tries(&app), Some(ASIO_RETRIES), "five tries at most");
    app.close_output();
    fall_back(&mut app);
    app.retry_asio();
    assert_eq!(tries(&app), Some(0), "opened afresh: counted again");
    app.output_fallback = None;
    app.retry_asio();
    assert_eq!(tries(&app), None, "ASIO in use: nothing to try");
}

#[test]
fn a_detect_take_on_an_audio_sound_is_rounded_to_a_sixteenth() {
    let Some(mut h) = harness(false, true) else { return };
    let tmp = tempfile::tempdir().unwrap();
    h.state_mut().path = Some(tmp.path().join("x.mxprj"));
    let p = project_of(&h);
    let bar = p.bar_length() as f64;
    let s = (1..16).find(|&s| p.groups[0].sounds[s].slots.is_empty() && p.groups[0].sounds[s].sampler.is_none()).unwrap();
    audio_sound(&mut h, s, &tone_of(1.0), bar);
    let ticks = 4.0 * PPQ as f64 + 70.0;
    h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(2.0), channels: 2, mode: 0, ticks, focus: (0, s) });
    let q = project_of(&h);
    let a = q.groups[0].sounds[s].audio.as_ref().expect("still Audio");
    assert_eq!(a.takes.len(), 2, "{}", h.state().status);
    assert_eq!(a.takes[1].beats, 4.0);
}

#[test]
fn a_detect_take_on_an_empty_sound_is_a_sampler_zone() {
    let Some(mut h) = harness(false, true) else { return };
    let tmp = tempfile::tempdir().unwrap();
    h.state_mut().path = Some(tmp.path().join("x.mxprj"));
    let p = project_of(&h);
    let s = (1..16).find(|&s| p.groups[0].sounds[s].slots.is_empty() && p.groups[0].sounds[s].sampler.is_none()).unwrap();
    (h.state_mut().group, h.state_mut().sound) = (0, s);
    h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 0, ticks: 960.0, focus: (0, s) });
    let q = project_of(&h);
    let snd = &q.groups[0].sounds[s];
    assert_eq!(snd.sampler.as_ref().map(|x| x.zones.len()), Some(1), "{}", h.state().status);
    assert!(snd.audio.is_none());
}

#[test]
fn the_take_goes_to_the_sound_focused_at_start() {
    let Some(mut h) = harness(false, true) else { return };
    let tmp = tempfile::tempdir().unwrap();
    h.state_mut().path = Some(tmp.path().join("x.mxprj"));
    (h.state_mut().group, h.state_mut().sound) = (0, 5);
    h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 0, ticks: 960.0, focus: (0, 2) });
    let q = project_of(&h);
    assert_eq!(q.groups[0].sounds[2].sampler.as_ref().map(|x| x.zones.len()), Some(1), "{}", h.state().status);
    assert!(q.groups[0].sounds[5].sampler.as_ref().is_none_or(|x| x.zones.is_empty()));
}

#[test]
fn with_no_free_sound_left_a_take_goes_to_the_last_audio_sound() {
    use layout::sampling::RecParam;
    let Some(mut h) = harness(false, true) else { return };
    let tmp = tempfile::tempdir().unwrap();
    h.state_mut().path = Some(tmp.path().join("x.mxprj"));
    let p = project_of(&h);
    let bar = p.bar_length() as f64;
    // Sound 9 an Audio plug-in, sound 15 a Sampler, the rest in between taken.
    audio_sound(&mut h, 9, &tone_of(1.0), bar);
    (h.state_mut().group, h.state_mut().sound) = (0, 15);
    h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 0, ticks: bar, focus: (0, 15) });
    for s in 1..15 {
        if s != 9 {
            (h.state_mut().group, h.state_mut().sound) = (0, s);
            h.state_mut().finish_recording(recorder::Finished::Take {
                sample: tone_of(0.2),
                channels: 2,
                mode: 0,
                ticks: bar,
                focus: (0, s),
            });
        }
    }
    assert!(project_of(&h).groups[0].sounds[15].sampler.is_some());
    (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound) = (0, 0, 0);
    h.state_mut().recorder.view.set(RecParam::Target, 1);
    h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 2, ticks: bar, focus: (0, 0) });
    let q = project_of(&h);
    assert_eq!(q.groups[0].sounds[9].audio.as_ref().map(|a| a.takes.len()), Some(2), "{}", h.state().status);
    assert!(q.groups[0].sounds[15].sampler.is_some());
}

#[test]
fn auto_with_no_free_sound_left_goes_to_the_last_sound_not_the_audio_one() {
    let Some(mut h) = harness(false, true) else { return };
    let tmp = tempfile::tempdir().unwrap();
    h.state_mut().path = Some(tmp.path().join("x.mxprj"));
    let bar = project_of(&h).bar_length() as f64;
    audio_sound(&mut h, 9, &tone_of(1.0), bar);
    for s in 1..16 {
        if s != 9 {
            (h.state_mut().group, h.state_mut().sound) = (0, s);
            h.state_mut().finish_recording(recorder::Finished::Take {
                sample: tone_of(0.2),
                channels: 2,
                mode: 0,
                ticks: bar,
                focus: (0, s),
            });
        }
    }
    // Focused on the Audio sound (no Sampler) at Start, so Auto looks for another; the
    // app's own focus has moved on since.
    (h.state_mut().group, h.state_mut().sound) = (0, 3);
    assert_eq!(h.state().next_free_sound(0, 9, false), 15);
    assert_eq!(h.state().next_free_sound(0, 9, true), 9);
    let path = tmp.path().join("Kick 100.wav");
    let s = tone_of(0.5);
    deckard_engine::edit::write_wav(&path, &s).unwrap();
    let item = recorder::AutoItem { note: 60, velocity: 100, keys: (0, 127), vels: (0, 127), path };
    h.state_mut().finish_recording(recorder::Finished::Auto { done: vec![(item, s.frames.len(), None)], focus: (0, 9) });
    let q = project_of(&h);
    assert_eq!(q.groups[0].sounds[9].instrument, "AudioModule");
    assert!(q.groups[0].sounds[9].sampler.is_none(), "{}", h.state().status);
    assert_eq!(h.state().sound, 15, "{}", h.state().status);
}
