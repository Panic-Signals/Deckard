//! Slices onto pads and into files: counts of 64, shuffled notes, APPLY to pads, EXPORT.

use crate::layout::sampling::{SPLIT_COUNTS, SamplingAct as S};
use crate::surface_sampling_tests::app;
use deckard_model::new_sampler::{PlayMode, ZoneMeta};

fn project(app: &crate::App) -> deckard_model::Project {
    app.doc.as_ref().unwrap().borrow().project().clone()
}

/// The app's tone (a second at 44.1 kHz) sliced at `n` equal places by hand.
fn sliced(n: usize) -> (crate::App, tempfile::TempDir) {
    let (mut app, tmp) = app();
    app.sampling_act(S::Tab(2));
    app.sampling_act(S::SliceMode(3));
    app.sampling.slices = (0..n).map(|k| k * 44_100 / n).collect();
    (app, tmp)
}

#[test]
fn split_into_64_goes_on_64_keys() {
    let (mut app, _tmp) = app();
    app.sampling_act(S::Tab(2));
    app.sampling_act(S::SliceMode(0));
    app.sampling_act(S::SplitCount(SPLIT_COUNTS.iter().position(|c| *c == 64).unwrap()));
    assert_eq!(app.sampling.slices.len(), 64);
    app.sampling_act(S::ApplySlices);
    let p = project(&app);
    let zones = &p.groups[app.group].sounds[app.sound].sampler.as_ref().unwrap().zones;
    assert_eq!(zones.len(), 64);
    assert_eq!((zones[0].key_range, zones[63].key_range), ((60, 60), (123, 123)));
}

#[test]
fn shuffled_notes_keep_their_times() {
    let notes = |shuffle: bool| {
        let (mut app, _tmp) = sliced(8);
        let (g, s) = (app.group, app.sound);
        app.doc.as_ref().unwrap().borrow_mut().add_pattern(g, 0, "Pattern 1").unwrap();
        app.sampling_act(S::PatternMode(0));
        app.sampling_act(S::NoteOrder(shuffle));
        app.sampling_act(S::ApplySlices);
        let mut n: Vec<(i64, i64)> =
            project(&app).groups[g].patterns[app.pattern].notes.iter().filter(|n| n.sound == s).map(|n| (n.position, n.pitch)).collect();
        n.sort_unstable();
        n
    };
    let (plain, shuffled) = (notes(false), notes(true));
    assert_eq!(plain.iter().map(|n| n.1).collect::<Vec<_>>(), (60..68).collect::<Vec<_>>());
    assert_eq!(plain.iter().map(|n| n.0).collect::<Vec<_>>(), shuffled.iter().map(|n| n.0).collect::<Vec<_>>(), "the same times");
    let mut pitches: Vec<i64> = shuffled.iter().map(|n| n.1).collect();
    assert_ne!(pitches, (60..68).collect::<Vec<_>>(), "other slices");
    pitches.sort_unstable();
    assert_eq!(pitches, (60..68).collect::<Vec<_>>(), "each once");
}

#[test]
fn seventeen_slices_make_two_groups_of_pads() {
    let (mut app, _tmp) = sliced(17);
    let before = project(&app);
    let (g, s) = (app.group, app.sound);
    let focus = (g, s, app.zone, app.sampling.tab);
    let name = before.groups[g].sounds[s].name.clone();
    let slices = app.sampling.slices.clone();
    app.sampling_act(S::SliceToEnd(false));
    app.sampling_act(S::ApplyTo(1));
    app.sampling_act(S::ApplySlices);
    let p = project(&app);
    let n = before.groups.len();
    assert_eq!(p.groups.len(), n + 2, "{}", app.status);
    assert_eq!((p.groups[n].name.clone(), p.groups[n + 1].name.clone()), (format!("{name} 1"), format!("{name} 2")));
    let used = |gi: usize| p.groups[gi].sounds.iter().filter(|x| !x.slots.is_empty()).count();
    assert_eq!((used(n), used(n + 1)), (16, 1));
    for (i, &a) in slices.iter().enumerate() {
        let snd = &p.groups[n + i / 16].sounds[i % 16];
        let zones = &snd.sampler.as_ref().expect("a sampler on the pad").zones;
        assert_eq!(zones.len(), 1);
        let b = slices.get(i + 1).copied().unwrap_or(44_100);
        assert_eq!((zones[0].play_range, zones[0].key_range, zones[0].root_key), ((a as i64, b as i64 - 1), (0, 127), 60), "slice {i}");
        assert_eq!(snd.name, format!("{name} {}", i + 1));
        // A Sampler's zone has no tempo: said to play as a sampler does.
        assert_eq!(zones[0].meta.as_ref().map(|m| m.mode), Some(PlayMode::Repitch));
    }
    assert!(p.groups[n].patterns.is_empty(), "No Pattern: none");
    // What was sliced is as it was.
    assert_eq!(format!("{:?}", p.groups[g].sounds), format!("{:?}", before.groups[g].sounds));
    assert_eq!((app.group, app.sound, app.zone, app.sampling.tab), focus);
    assert_eq!(app.sampling.slices, slices);
    assert!(app.status.contains("17 slices"), "{}", app.status);
    // One step back: both groups gone.
    let doc = app.doc.clone().unwrap();
    assert_eq!(doc.borrow().undo_label(), Some("Apply Slices to Pads"));
    assert!(doc.borrow_mut().undo());
    assert_eq!(project(&app).groups.len(), n);
}

#[test]
fn sixty_four_slices_make_four_groups_with_their_patterns() {
    let (mut app, _tmp) = sliced(64);
    let n = project(&app).groups.len();
    app.sampling_act(S::ApplyTo(1));
    app.sampling_act(S::PatternMode(0));
    app.sampling_act(S::ApplySlices);
    let p = project(&app);
    assert_eq!(p.groups.len(), n + 4, "{}", app.status);
    let ticks_per_frame = p.tempo / 60.0 * deckard_model::PPQ as f64 / 44_100.0;
    let mut seen = 0;
    for k in 0..4 {
        let group = &p.groups[n + k];
        assert_eq!(group.sounds.iter().filter(|x| !x.slots.is_empty()).count(), 16);
        assert_eq!(group.patterns.len(), 1);
        let pat = &group.patterns[0];
        assert_eq!(pat.length % p.bar_length(), 0);
        assert!(pat.length as f64 >= 44_100.0 * ticks_per_frame);
        assert_eq!(pat.notes.len(), 16);
        assert!(p.scenes[app.scene].patterns.contains(&(n + k, 0)), "in the focused scene: {:?}", p.scenes[app.scene].patterns);
        for note in &pat.notes {
            let i = k * 16 + note.sound;
            let at = pat.start + ((i * 44_100 / 64) as f64 * ticks_per_frame).round() as i64;
            assert_eq!((note.position, note.pitch), (at, 60), "slice {i}");
            seen += 1;
        }
    }
    assert_eq!(seen, 64);
}

#[test]
fn shuffled_pads_play_in_another_order() {
    let (mut app, _tmp) = sliced(20);
    let n = project(&app).groups.len();
    app.sampling_act(S::ApplyTo(1));
    app.sampling_act(S::PatternMode(0));
    app.sampling_act(S::NoteOrder(true));
    app.sampling_act(S::ApplySlices);
    let p = project(&app);
    let ticks_per_frame = p.tempo / 60.0 * deckard_model::PPQ as f64 / 44_100.0;
    // (time slot, slice played) of every note of both groups.
    let mut played: Vec<(usize, usize)> = Vec::new();
    for k in 0..2 {
        let pat = &p.groups[n + k].patterns[0];
        for note in &pat.notes {
            let slot = (0..20)
                .find(|i| pat.start + ((i * 44_100 / 20) as f64 * ticks_per_frame).round() as i64 == note.position)
                .expect("at a slice's time");
            played.push((slot, k * 16 + note.sound));
        }
    }
    played.sort_unstable();
    assert_eq!(played.iter().map(|x| x.0).collect::<Vec<_>>(), (0..20).collect::<Vec<_>>(), "a note a slot");
    let mut slices: Vec<usize> = played.iter().map(|x| x.1).collect();
    assert_ne!(slices, (0..20).collect::<Vec<_>>());
    slices.sort_unstable();
    assert_eq!(slices, (0..20).collect::<Vec<_>>(), "every slice once");
}

#[test]
fn a_sync_zone_stays_in_sync_on_its_pad() {
    let (mut app, _tmp) = app();
    app.convert_instrument(true);
    let (g, s) = (app.group, app.sound);
    let doc = app.doc.clone().unwrap();
    let file = doc.borrow().project().groups[g].sounds[s].sampler.as_ref().unwrap().zones[0].sample.path.clone();
    let meta = ZoneMeta { file, mode: PlayMode::Sync, bpm: Some(120.0), beats: Some(2.0), ..ZoneMeta::default() };
    doc.borrow_mut().set_zone_meta(g, s, 0, &meta, "Zone Tempo").unwrap();
    app.update_sampling();
    app.sampling_act(S::Tab(2));
    app.sampling_act(S::SliceMode(3));
    app.sampling.slices = vec![0, 22_050];
    let n = project(&app).groups.len();
    app.sampling_act(S::ApplyTo(1));
    app.sampling_act(S::ApplySlices);
    let p = project(&app);
    assert_eq!(p.groups.len(), n + 1, "{}", app.status);
    assert_eq!(p.groups[n].name, p.groups[g].sounds[s].name, "one group: the sound's name");
    for pad in 0..2 {
        let snd = &p.groups[n].sounds[pad];
        assert!(snd.new_sampler.is_some(), "a New Sampler as the sliced one");
        let m = snd.sampler.as_ref().unwrap().zones[0].meta.clone().expect("its tempo comes along");
        assert_eq!((m.mode, m.bpm), (PlayMode::Sync, Some(120.0)));
    }
    // A New Sampler's slices play to the sample's end unless told otherwise.
    assert_eq!(p.groups[n].sounds[0].sampler.as_ref().unwrap().zones[0].play_range, (0, 44_099));
}

#[test]
fn export_writes_a_file_per_slice() {
    let (mut app, tmp) = sliced(4);
    let dir = tmp.path().join("out");
    assert_eq!(app.export_slices_to(&dir, false), Ok(4));
    let files = app.slice_files(&dir);
    assert_eq!(
        files.iter().map(|f| f.file_name().unwrap().to_str().unwrap()).collect::<Vec<_>>(),
        ["Tone 01.wav", "Tone 02.wav", "Tone 03.wav", "Tone 04.wav"]
    );
    for f in &files {
        let s = deckard_engine::sample::Sample::load(f).unwrap();
        assert_eq!((s.rate, s.frames.len()), (44_100, 11_025));
    }
    // There already: nothing replaced unless asked.
    app.sampling.slices = vec![0, 22_050];
    let err = app.export_slices_to(&dir, false).unwrap_err();
    assert!(err.contains("Tone 01.wav") && err.contains("there already"), "{err}");
    assert_eq!(deckard_engine::sample::Sample::load(&files[0]).unwrap().frames.len(), 11_025);
    assert_eq!(app.export_slices_to(&dir, true), Ok(2));
    assert_eq!(deckard_engine::sample::Sample::load(&files[0]).unwrap().frames.len(), 22_050);
    // No slices: the whole range, one file.
    app.sampling.slices.clear();
    assert_eq!(app.export_slices_to(&tmp.path().join("whole"), false), Ok(1));
}

#[test]
fn pads_that_play_to_the_end_are_ended_by_their_notes() {
    let (mut app, _tmp) = sliced(3);
    let n = project(&app).groups.len();
    let (g, s) = (app.group, app.sound);
    assert_eq!(project(&app).groups[g].sounds[s].sampler.as_ref().unwrap().envelope_type, 0, "a one-shot");
    app.sampling_act(S::SliceToEnd(true));
    app.sampling_act(S::ApplyTo(1));
    app.sampling_act(S::ApplySlices);
    let p = project(&app);
    assert_eq!(p.groups.len(), n + 1, "{}", app.status);
    for pad in 0..3 {
        let smp = p.groups[n].sounds[pad].sampler.as_ref().unwrap();
        assert_eq!(smp.zones[0].play_range.1, 44_099, "to the sample's end");
        assert_eq!(smp.envelope_type, 2, "held: the note ends it");
    }
    assert_eq!(p.groups[g].sounds[s].sampler.as_ref().unwrap().envelope_type, 0, "the sliced sound is as it was");
}

#[test]
fn a_sync_zones_notes_go_by_its_own_tempo_and_each_slices_length() {
    let (mut app, _tmp) = app();
    app.convert_instrument(true);
    let (g, s) = (app.group, app.sound);
    let doc = app.doc.clone().unwrap();
    let file = doc.borrow().project().groups[g].sounds[s].sampler.as_ref().unwrap().zones[0].sample.path.clone();
    // A second at 60 BPM is a beat, whatever the song's tempo.
    let meta = ZoneMeta { file, mode: PlayMode::Sync, bpm: Some(60.0), beats: Some(1.0), ..ZoneMeta::default() };
    doc.borrow_mut().set_zone_meta(g, s, 0, &meta, "Zone Tempo").unwrap();
    app.update_sampling();
    app.sampling_act(S::Tab(2));
    app.sampling_act(S::SliceMode(3));
    app.sampling.slices = vec![0, 11_025, 33_075];
    let n = project(&app).groups.len();
    app.sampling_act(S::ApplyTo(1));
    app.sampling_act(S::PatternMode(0));
    app.sampling_act(S::ApplySlices);
    let p = project(&app);
    assert_ne!(p.tempo, 60.0);
    let ppq = deckard_model::PPQ;
    let pat = &p.groups[n].patterns[0];
    let mut notes: Vec<(usize, i64, i64)> = pat.notes.iter().map(|x| (x.sound, x.position - pat.start, x.length)).collect();
    notes.sort_unstable();
    assert_eq!(notes, [(0, 0, ppq / 4), (1, ppq / 4, ppq / 2), (2, ppq * 3 / 4, ppq / 4)]);
}
