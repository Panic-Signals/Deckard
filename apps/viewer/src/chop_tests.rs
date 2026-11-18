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
    app.sampling.playhead = Some(11_025);
    app.sampling_act(S::SliceMode(0));
    app.sampling_act(S::SplitCount(0));
    assert_eq!(app.sampling.slices.len(), 4);
    assert_eq!(app.sampling.slices[0], 11_025);
    // Stopped: all of it again; a part selected: from there.
    app.sampling.playhead = None;
    app.sampling_act(S::SplitCount(0));
    assert_eq!(app.sampling.slices[0], 0);
    app.sampling.selection = Some((22_050, 30_000));
    app.sampling_act(S::SplitCount(0));
    assert_eq!(app.sampling.slices[0], 22_050);
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
    // Dragged, the mark goes with it; again, it's let go; removed, the mark goes.
    let i = app.sampling.slices.iter().position(|f| *f == kept).unwrap();
    if i > 0 {
        app.sampling_act(S::MoveSlice(i, kept + 10));
        assert_eq!(app.sampling.kept, [kept + 10]);
        app.sampling_act(S::RemoveSliceAt(kept + 20));
        assert!(app.sampling.kept.is_empty());
    }
    app.sampling_act(S::KeepSlice(1));
    assert_eq!(app.sampling.kept.len(), 1);
    app.sampling_act(S::KeepSlice(1));
    assert!(app.sampling.kept.is_empty(), "kept or not, in turn");
    app.sampling_act(S::KeepSlice(1));
    app.sampling_act(S::SliceMode(0));
    assert!(app.sampling.kept.is_empty(), "Split keeps none");
    app.sampling_act(S::KeepSlice(1));
    assert!(app.sampling.kept.is_empty(), "nor can one be kept there");
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
