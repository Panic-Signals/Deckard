//! Chopping by playing: slices caught at the playhead, Random and its kept slices,
//! Split from the playhead, Key Shift.

use crate::layout::sampling::SamplingAct as S;
use crate::surface_sampling_tests::{app, app_with};
use deckard_engine::sample::Sample;
use deckard_model::new_sampler::{PlayMode, ZoneMeta};

const RATE: u32 = 44_100;

/// Eight seconds with a burst of noise every half second from a quarter in: sixteen hits.
fn hits() -> Sample {
    let mut frames = vec![[0f32; 2]; 8 * RATE as usize];
    let mut seed = 1u32;
    for k in 0..16 {
        for i in 0..4000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let v = ((seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0) * 0.6 * (-(i as f32) / 800.0).exp();
            frames[RATE as usize / 4 + k * RATE as usize / 2 + i] = [v; 2];
        }
    }
    Sample { rate: RATE, frames }
}

#[test]
fn catching_drops_a_slice_at_the_playhead_in_manual_mode() {
    let (mut app, _tmp) = app();
    app.sampling_act(S::SliceMode(3));
    assert_eq!(app.sampling.slices, [0]);
    app.sampling_act(S::Catch(false));
    assert_eq!(app.sampling.slices, [0], "nothing plays: nothing caught");
    app.sampling.playhead = Some(20_000);
    app.sampling_act(S::Catch(false));
    assert_eq!(app.sampling.slices, [0, 20_000], "a sample with no tempo: where it plays");
    app.sampling_act(S::Catch(true));
    assert_eq!(app.sampling.slices, [0, 20_000], "there already");
    app.sampling.playhead = Some(9_000);
    app.sampling_act(S::Catch(true));
    assert_eq!(app.sampling.slices, [0, 9_000, 20_000], "in order");
    // Any other mode: a pad drops nothing.
    app.sampling_act(S::SliceMode(0));
    let split = app.sampling.slices.clone();
    app.sampling.playhead = None;
    app.sampling_act(S::SliceMode(0));
    app.sampling.playhead = Some(30_000);
    app.sampling_act(S::Catch(false));
    assert!(!app.sampling.slices.contains(&30_000));
    assert_eq!(split.len(), app.sampling.slices.len());
}

#[test]
fn a_caught_slice_lands_on_the_samples_eighths() {
    let (mut app, _tmp) = app();
    app.convert_instrument(true);
    let (g, s) = (app.group, app.sound);
    let doc = app.doc.clone().unwrap();
    let file = doc.borrow().project().groups[g].sounds[s].sampler.as_ref().unwrap().zones[0].sample.path.clone();
    // 120 BPM at 44.1 kHz: an 8th is 11 025 frames.
    let meta = ZoneMeta { file, mode: PlayMode::Sync, bpm: Some(120.0), beats: Some(2.0), ..ZoneMeta::default() };
    doc.borrow_mut().set_zone_meta(g, s, 0, &meta, "Zone Tempo").unwrap();
    app.sampling_act(S::SliceMode(3));
    app.sampling.playhead = Some(12_000);
    app.sampling_act(S::Catch(false));
    assert_eq!(app.sampling.slices, [0, 11_025]);
    app.sampling_act(S::Catch(true));
    assert_eq!(app.sampling.slices, [0, 11_025, 12_000], "SHIFT: exactly there");
}

#[test]
fn split_starts_at_the_playhead() {
    let (mut app, _tmp) = app();
    // A slice's pad sounding (or the song playing the sound): from the start as ever.
    app.sampling.playhead = Some(11_025);
    app.sampling_act(S::SliceMode(0));
    app.sampling_act(S::SplitCount(0));
    assert_eq!((app.sampling.slices.len(), app.sampling.slices[0]), (4, 0));
    // PLAY plays it: from where it is.
    app.audition = Some((app.group, app.sound, 60));
    app.sampling_act(S::SplitCount(0));
    assert_eq!((app.sampling.slices.len(), app.sampling.slices[0]), (4, 11_025));
    app.sampling_act(S::SliceMode(1));
    assert_eq!(app.sampling.slices[0], 11_025, "Grid too");
    app.sampling_act(S::SliceMode(0));
    // Stopped: all of it again.
    (app.audition, app.sampling.playhead) = (None, None);
    app.sampling_act(S::SplitCount(0));
    assert_eq!(app.sampling.slices[0], 0);
    // The selection a sample comes up with isn't a place to start; one made on EDIT is.
    app.sampling.selection = Some((22_050, 30_000));
    app.sampling_act(S::SplitCount(0));
    assert_eq!(app.sampling.slices[0], 0);
    app.sampling_act(S::Tab(1));
    app.sampling_act(S::Select(Some((22_050, 30_000))));
    app.sampling_act(S::Tab(2));
    app.sampling_act(S::SplitCount(0));
    assert_eq!(app.sampling.slices[0], 22_050);
    app.sampling_act(S::Select(None));
    app.sampling_act(S::SplitCount(0));
    assert_eq!(app.sampling.slices[0], 0);
}

#[test]
fn random_rerolls_and_keeps() {
    let (mut app, _tmp) = app_with(hits());
    app.sampling_act(S::SliceMode(5));
    app.sampling_act(S::RandomCount(1));
    let first = app.sampling.slices.clone();
    assert_eq!(first.len(), 8, "eight of the sixteen hits: {first:?}");
    assert!(first.iter().all(|f| (f + 600 + RATE as usize / 4) % (RATE as usize / 2) < 1500), "each on a hit: {first:?}");
    // A slice kept stays through re-rolls and another count; the rest change.
    app.sampling_act(S::KeepSlice(3));
    let kept = first[3];
    assert_eq!(app.sampling.kept, [kept]);
    // Dragged, the mark goes with it.
    app.sampling_act(S::MoveSlice(3, kept + 10));
    assert_eq!(app.sampling.kept, [kept + 10]);
    app.sampling_act(S::MoveSlice(3, kept));
    assert_eq!(app.sampling.kept, [kept]);
    // One kept where slicing no longer reaches is forgotten.
    app.sampling.kept.push(100_000_000);
    app.sampling_act(S::Reroll);
    assert_eq!(app.sampling.kept, [kept]);
    let mut seen = vec![first.clone()];
    for _ in 0..4 {
        app.sampling_act(S::Reroll);
        assert_eq!(app.sampling.slices.len(), 8);
        assert!(app.sampling.slices.contains(&kept));
        seen.push(app.sampling.slices.clone());
    }
    seen.dedup();
    assert!(seen.len() > 2, "re-rolls pick others: {seen:?}");
    app.sampling_act(S::RandomCount(0));
    assert_eq!(app.sampling.slices.len(), 4);
    assert!(app.sampling.slices.contains(&kept));
    // More asked for than there are hits: as many as there are.
    app.sampling_act(S::RandomCount(3));
    assert!(app.sampling.slices.len() <= 17 && app.sampling.slices.len() >= 12, "{:?}", app.sampling.slices);
    // Find keeps it too.
    app.sampling_act(S::SliceMode(4));
    app.sampling_act(S::FindCount(0));
    assert!(app.sampling.slices.contains(&kept) && app.sampling.slices.len() == 4, "{:?}", app.sampling.slices);
    // Removed, its mark goes.
    app.sampling.kept.clear();
    let second = app.sampling.slices[1];
    app.sampling_act(S::KeepSlice(1));
    assert_eq!(app.sampling.kept, [second]);
    app.sampling_act(S::RemoveSliceAt(second + 5));
    assert!(app.sampling.kept.is_empty() && !app.sampling.slices.contains(&second));
    app.sampling_act(S::KeepSlice(1));
    assert_eq!(app.sampling.kept.len(), 1);
    app.sampling_act(S::KeepSlice(1));
    assert!(app.sampling.kept.is_empty(), "kept or not, in turn");
    app.sampling_act(S::KeepSlice(1));
    app.sampling_act(S::SliceMode(0));
    assert!(app.sampling.kept.is_empty(), "Split keeps none");
    app.sampling_act(S::KeepSlice(1));
    assert!(app.sampling.kept.is_empty(), "nor can one be kept there");
    // APPLY: the kept marks are done with.
    app.sampling_act(S::SliceMode(5));
    app.sampling_act(S::KeepSlice(1));
    assert_eq!(app.sampling.kept.len(), 1);
    app.sampling_act(S::ApplySlices);
    assert!(app.sampling.kept.is_empty());
}

#[test]
fn a_slice_pad_gives_a_playhead() {
    use deckard_lua_host::state::SamplingCall as C;
    let (mut app, _tmp) = app();
    app.sampling_call(C::Prehear(11_025, 44_100));
    if app.prehear_from.is_none() {
        // (No audio output on this machine: nothing plays.)
        return;
    }
    app.update_sampling();
    let at = app.sampling.playhead.expect("the slice plays");
    assert!((11_025..22_050).contains(&at), "{at}");
    // Past its end: no playhead.
    app.prehear_from = app.prehear_from.map(|(t, a, b)| (t - std::time::Duration::from_secs(2), a, b));
    app.update_sampling();
    assert_eq!(app.sampling.playhead, None);
    assert!(app.prehear_from.is_none());
}

fn sounds(app: &crate::App) -> Vec<deckard_model::Sound> {
    app.doc.as_ref().unwrap().borrow().project().groups[app.group].sounds.clone()
}

#[test]
fn key_shift_puts_a_slice_on_the_next_empty_sound() {
    let (mut app, _tmp) = app();
    let before = sounds(&app);
    let target = before.iter().position(|x| x.slots.is_empty()).unwrap();
    let focus = (app.group, app.sound, app.zone);
    app.sampling_act(S::SliceMode(3));
    app.sampling.slices = vec![0, 11_025, 22_050];
    app.sampling_act(S::KeyShift(1));
    let after = sounds(&app);
    let new = &after[target];
    let zones = &new.sampler.as_ref().expect("a Sampler there").zones;
    assert_eq!(zones.len(), 1);
    assert_eq!((zones[0].play_range, zones[0].key_range, zones[0].root_key), ((11_025, 22_049), (0, 127), 60));
    assert_eq!(zones[0].sample.path, before[app.sound].sampler.as_ref().unwrap().zones[0].sample.path);
    assert!(new.new_sampler.is_some(), "a New Sampler");
    // A Sampler's zone has no tempo: said to play as a sampler does (left unsaid, the
    // analysis of a New Sampler zone would put a loop in Sync).
    let m = zones[0].meta.clone().expect("how it plays is set");
    assert_eq!((m.mode, m.file.as_str()), (PlayMode::Repitch, zones[0].sample.path.as_str()));
    assert!(new.name.ends_with(" keys"), "{}", new.name);
    assert_eq!(focus, (app.group, app.sound, app.zone), "the focus stays");
    assert_eq!(app.sampling.slices, [0, 11_025, 22_050], "and the slices");
    assert!(app.status.contains(&format!("pad {}", target + 1)), "{}", app.status);
    // The sliced sound is as it was; one undo takes the new one away.
    assert_eq!(after[app.sound].sampler.as_ref().unwrap().zones.len(), 1);
    let doc = app.doc.clone().unwrap();
    assert_eq!(doc.borrow().undo_label(), Some("Key Shift"));
    assert!(doc.borrow_mut().undo());
    assert!(sounds(&app)[target].slots.is_empty());
    // No such slice: nothing.
    app.sampling_act(S::KeyShift(7));
    assert!(sounds(&app)[target].slots.is_empty());
}

#[test]
fn key_shift_of_a_sync_zone_plays_in_keys_and_leaves_its_notes() {
    let (mut app, _tmp) = app();
    app.convert_instrument(true);
    let (g, s) = (app.group, app.sound);
    let doc = app.doc.clone().unwrap();
    let file = doc.borrow().project().groups[g].sounds[s].sampler.as_ref().unwrap().zones[0].sample.path.clone();
    let meta = ZoneMeta { file, mode: PlayMode::Sync, bpm: Some(120.0), beats: Some(2.0), ..ZoneMeta::default() };
    doc.borrow_mut().set_zone_meta(g, s, 0, &meta, "Zone Tempo").unwrap();
    // A note of the sliced sound in the pattern: not the new sound's.
    let note = deckard_model::Note {
        ratchet: Default::default(),
        expr: None,
        sound: s,
        position: 0,
        length: 240,
        pitch: 60,
        velocity: 100,
        probability: 100,
    };
    let pi = doc.borrow_mut().add_pattern(g, 0, "Pattern 1").unwrap();
    doc.borrow_mut().edit_notes("Note", g, pi, &[], &[note]).unwrap();
    let target = sounds(&app).iter().position(|x| x.slots.is_empty()).unwrap();
    app.sampling_act(S::SliceMode(3));
    app.sampling.slices = vec![0, 22_050];
    app.sampling_act(S::KeyShift(0));
    let new = sounds(&app)[target].clone();
    let m = new.sampler.as_ref().unwrap().zones[0].meta.clone().expect("its tempo comes along");
    assert_eq!((m.mode, m.bpm), (PlayMode::Keys, Some(120.0)));
    assert_eq!(new.sampler.as_ref().unwrap().zones[0].play_range, (0, 22_049));
    let d = doc.borrow();
    let notes = &d.project().groups[g].patterns[pi].notes;
    assert!(notes.iter().all(|n| n.sound == s) && notes.len() == 1, "{notes:?}");
}

#[test]
fn key_shift_without_an_empty_sound_says_so() {
    let (mut app, tmp) = app();
    let g = app.group;
    let path = tmp.path().join("Tone.wav");
    while let Some(s) = sounds(&app).iter().position(|x| x.slots.is_empty()) {
        app.load_take(g, s, &path, 44_100);
    }
    let doc = app.doc.clone().unwrap();
    let label = doc.borrow().undo_label().map(str::to_string);
    app.sampling_act(S::SliceMode(3));
    app.sampling.slices = vec![0, 11_025];
    app.sampling_act(S::KeyShift(1));
    assert!(app.status.contains("empty sound"), "{}", app.status);
    assert_eq!(doc.borrow().undo_label().map(str::to_string), label);
}

/// The app's tone as a half-bar loop at 120 BPM (four eighths of 11 025 frames), its
/// edits written beside a project in the test's folder.
fn loop_at_120(warped: bool) -> (crate::App, tempfile::TempDir) {
    let (mut app, tmp) = app();
    app.path = Some(tmp.path().join("Song.mxprj"));
    app.convert_instrument(true);
    let (g, s) = (app.group, app.sound);
    let doc = app.doc.clone().unwrap();
    let file = doc.borrow().project().groups[g].sounds[s].sampler.as_ref().unwrap().zones[0].sample.path.clone();
    let warps = if warped {
        deckard_model::new_sampler::warps_at(&ZoneMeta { bpm: Some(120.0), ..ZoneMeta::default() }, RATE, &[0, 22_050])
    } else {
        Vec::new()
    };
    let meta = ZoneMeta { file, mode: PlayMode::Sync, bpm: Some(120.0), beats: Some(2.0), warps, ..ZoneMeta::default() };
    doc.borrow_mut().set_zone_meta(g, s, 0, &meta, "Zone Tempo").unwrap();
    app.update_sampling();
    (app, tmp)
}

fn zone0(app: &crate::App) -> deckard_model::Zone {
    let d = app.doc.as_ref().unwrap().borrow();
    d.project().groups[app.group].sounds[app.sound].sampler.as_ref().unwrap().zones[0].clone()
}

#[test]
fn munge_takes_an_eighth_out_of_the_bar() {
    use crate::layout::sampling::EditOp;
    let (mut app, _tmp) = loop_at_120(true);
    let before = zone0(&app);
    app.sampling_act(S::MungeOpen(true));
    let m = app.sampling.munge.expect("the bar is open");
    assert!(!m.changes());
    // Nothing set: APPLY keeps the sample as it is.
    app.sampling_act(S::MungeApply);
    assert_eq!(zone0(&app), before);
    assert!(app.status.contains("nothing to change"), "{}", app.status);
    let mut mutes = m.mutes;
    mutes[3] = true;
    app.sampling_act(S::SetMunge(deckard_engine::munge::Munge { mutes, ..m }));
    app.sampling_act(S::MungeHear);
    assert_eq!(zone0(&app), before, "HEAR keeps nothing");
    app.sampling_act(S::MungeApply);
    assert!(app.sampling.munge.is_none(), "the bar closes");
    let z = zone0(&app);
    assert_ne!(z.sample.path, before.sample.path, "a new file: {}", app.status);
    let made = Sample::load(std::path::Path::new(&z.sample.path)).unwrap();
    assert_eq!(made.frames.len(), 3 * 11_025, "three eighths left");
    let meta = z.meta.clone().expect("it keeps what it knows");
    assert_eq!((meta.bpm, meta.beats, meta.warps.len(), meta.mode), (Some(120.0), Some(1.5), 0, PlayMode::Sync));
    assert_eq!(meta.file, z.sample.path);
    app.undo();
    assert_eq!(zone0(&app), before, "one step back");
    // The other edits don't answer to it, and STRETCH closes it.
    app.sampling_act(S::MungeOpen(true));
    app.sampling_act(S::Edit(EditOp::Munge));
    assert_eq!(zone0(&app), before);
    app.sampling_act(S::StretchOpen(true));
    assert!(app.sampling.munge.is_none() && app.sampling.stretch.is_some());
    app.sampling_act(S::MungeOpen(true));
    assert!(app.sampling.stretch.is_none());
}

#[test]
fn munge_swings_a_selection_and_keeps_the_rest() {
    let (mut app, _tmp) = loop_at_120(false);
    let before = app.sampling.sample.clone().unwrap();
    app.sampling.selection = Some((11_025, 33_075));
    app.sampling_act(S::MungeOpen(true));
    let m = app.sampling.munge.unwrap();
    app.sampling_act(S::SetMunge(deckard_engine::munge::Munge { swing: 0.6, sixteenths: false, ..m }));
    app.sampling_act(S::MungeApply);
    let z = zone0(&app);
    let made = Sample::load(std::path::Path::new(&z.sample.path)).unwrap();
    assert_eq!(made.frames.len(), before.frames.len(), "swing keeps the length");
    let near = |a: &[[f32; 2]], b: &[[f32; 2]]| a.iter().zip(b).all(|(x, y)| (x[0] - y[0]).abs() < 1e-4);
    assert!(near(&made.frames[..11_025], &before.frames[..11_025]) && near(&made.frames[33_075..], &before.frames[33_075..]));
    assert!(!near(&made.frames[11_025..33_075], &before.frames[11_025..33_075]), "the selection is swung");
    assert_eq!(z.meta.unwrap().beats, Some(2.0));
}

#[test]
fn munge_needs_a_tempo() {
    let (mut app, tmp) = app();
    app.path = Some(tmp.path().join("Song.mxprj"));
    let before = zone0(&app);
    app.sampling_act(S::MungeOpen(true));
    let m = app.sampling.munge.unwrap();
    app.sampling_act(S::SetMunge(deckard_engine::munge::Munge { swing: 0.5, ..m }));
    app.sampling_act(S::MungeApply);
    assert_eq!(zone0(&app), before);
    assert!(app.status.contains("tempo"), "{}", app.status);
    app.sampling_act(S::MungeHear);
    assert!(app.status.contains("tempo"), "{}", app.status);
    // Every eighth off: nothing would be left.
    let (mut app, _tmp) = loop_at_120(false);
    let before = zone0(&app);
    app.sampling_act(S::MungeOpen(true));
    let m = app.sampling.munge.unwrap();
    app.sampling_act(S::SetMunge(deckard_engine::munge::Munge { mutes: [true; 8], ..m }));
    app.sampling_act(S::MungeApply);
    assert_eq!(zone0(&app), before);
    assert!(app.status.contains("nothing would be left"), "{}", app.status);
}

/// MOTION (the sound's) and TAIL (the zone's), from the window and from the MK3's knobs:
/// each one undo step, and both reach the engine.
#[test]
fn motion_and_tail_are_set_and_undone() {
    use deckard_model::new_sampler::Motion;
    let (mut app, _tmp) = loop_at_120(false);
    let (g, s) = (app.group, app.sound);
    let sound = |app: &crate::App| app.doc.as_ref().unwrap().borrow().project().groups[g].sounds[s].clone();
    let state = |app: &crate::App| (sound(app).new_sampler.unwrap().motion, zone0(app).meta.unwrap().tail);
    assert_eq!(state(&app), (Motion::Off, 0.0));
    app.sampling_act(S::Motion(Motion::Next));
    assert_eq!(state(&app), (Motion::Next, 0.0));
    app.sampling_act(S::Motion(Motion::Next));
    app.undo();
    assert_eq!(state(&app), (Motion::Off, 0.0), "one step, and setting it again was none");
    app.sampling_act(S::Meta(ZoneMeta { tail: 250.0, ..zone0(&app).meta.unwrap() }));
    assert_eq!(state(&app), (Motion::Off, 250.0));
    // The MK3 has a page for both.
    let knob = |app: &mut crate::App, key: &str| {
        let pages = app.surface_view().sampling.pages;
        pages[1].iter().flat_map(|p| p.knobs.clone()).find(|k| k.key == key)
    };
    let tail = knob(&mut app, "Play.Tail").expect("a TAIL knob");
    assert_eq!((tail.text.as_str(), tail.value), ("250 ms", 250.0));
    assert_eq!(knob(&mut app, "Play.Motion").expect("a MOTION knob").text, "Off");
    app.record_apply.insert("Play.Tail".into(), crate::sampling_hw::Apply::Tail);
    app.record_apply.insert("Play.Motion".into(), crate::sampling_hw::Apply::Motion);
    app.sampling_knob("Play.Tail", 404.0);
    app.sampling_knob("Play.Motion", 2.0);
    assert_eq!(state(&app), (Motion::Random, 400.0), "the knob steps by 10 ms");
    assert_eq!(knob(&mut app, "Play.Tail").unwrap().text, "400 ms");
    assert_eq!(knob(&mut app, "Play.Motion").unwrap().text, "Random");
    app.undo();
    assert_eq!(state(&app), (Motion::Off, 400.0));
    // A plain Sampler has neither.
    app.convert_instrument(false);
    app.sampling_act(S::Motion(Motion::Next));
    assert!(sound(&app).new_sampler.is_none());
    assert!(knob(&mut app, "Play.Tail").is_none() && knob(&mut app, "Play.Motion").is_none());
}
