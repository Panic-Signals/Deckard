use super::*;

/// The audio thread hands meters and the playhead to the UI: peaks merge until
/// taken, and publishing doesn't reallocate the engine's own meters.
#[test]
fn telemetry_merges_peaks_without_reallocating() {
    let mut e = click_engine(48_000);
    let mut t = Telemetry::default();
    let mut buf = vec![0f32; 256];
    e.process(&mut buf);
    e.publish(&mut t);
    assert!(t.meters[0][0].0 > 0.5, "the click is metered: {:?}", t.meters);
    assert!(t.sequencing && t.mode == Some(Mode::Arrangement));
    let ptr = e.meters[0].as_ptr();
    e.process(&mut buf);
    e.publish(&mut t);
    assert_eq!(e.meters[0].as_ptr(), ptr, "the engine's meters were reset in place");
    assert!(t.meters[0][0].0 > 0.5, "the peak stays until the UI takes it");
    assert!(t.position > 0.0);
}

/// A clip plays its own pattern (repeated to fill it) in place of the group's
/// pattern in the section; past the sections it lengthens the song.
#[test]
fn clips_play_in_place_of_patterns() {
    let mut p = project();
    let clip_note =
        Note { ratchet: Default::default(), expr: None, sound: 0, position: 0, length: 240, pitch: 72, velocity: 100, probability: 100 };
    p.groups[0].clip_patterns = vec![Pattern { length: 2 * PPQ, notes: vec![clip_note], ..Default::default() }];
    p.groups[0].clips = vec![deckard_model::Clip { position: 4 * PPQ, length: 4 * PPQ, pattern: 0 }];
    let t = timeline(&p, Mode::Arrangement);
    let got: Vec<(i64, i64)> = t.events.iter().map(|e| (e.tick, e.pitch)).collect();
    let mut want: Vec<(i64, i64)> = (0..4).map(|i| (i * PPQ, 60)).collect();
    want.extend([(4 * PPQ, 72), (6 * PPQ, 72)]);
    assert_eq!(got, want);
    assert_eq!(t.length, 8 * PPQ);
    p.groups[0].clips[0].position = 10 * PPQ;
    assert_eq!(timeline(&p, Mode::Arrangement).length, 14 * PPQ, "a clip after the last section");
    assert!(timeline(&p, Mode::Scene(0)).events.iter().all(|e| e.pitch == 60), "scenes don't play clips");
}

#[test]
fn pattern_loops_to_fill_section() {
    let t = timeline(&project(), Mode::Arrangement);
    assert_eq!(t.length, 8 * PPQ);
    assert_eq!(t.events.iter().map(|e| e.tick).collect::<Vec<_>>(), (0..8).map(|i| i * PPQ).collect::<Vec<_>>());
}

/// Note Repeat in the engine: a held pad plays on the rate's grid until released,
/// and what it plays is recorded.
#[test]
fn note_repeat_plays_and_records_a_held_pad() {
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
    e.looping = true;
    e.set_recording(true);
    e.set_repeat(Some((deckard_model::Arp { rate: 240, gate: 0.5, ..Default::default() }, repeat::RepeatMode::NoteRepeat)));
    let mut out = vec![0f32; 512];
    e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
    // 960 ticks (a quarter at 120 BPM: 24 000 frames).
    let blocks = (960.0 * e.samples_per_tick() / 256.0).ceil() as usize;
    for _ in 0..blocks {
        e.process(&mut out);
    }
    e.schedule(0, Live::NoteRelease { group: 0, sound: 0, pitch: 60 });
    for _ in 0..blocks {
        e.process(&mut out);
    }
    let rec = e.take_recorded();
    assert!((4..=5).contains(&rec.len()), "{rec:?}");
    assert!(rec.iter().all(|r| r.length == 120 && r.velocity == 100));
    let gaps: Vec<f64> = rec.windows(2).map(|w| w[1].tick - w[0].tick).collect();
    assert!(gaps.iter().all(|g| (g - 240.0).abs() < 1.0), "{gaps:?}");
}

/// Pressure for Note Repeat only (a pad in the Velocity pad mode): its repeats play
/// at it, nothing is recorded as expression; per-note pressure (the aftertouch modes)
/// leaves the repeats' velocity alone.
#[test]
fn repeat_pressure_sets_repeat_velocity_only() {
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
    e.looping = true;
    e.set_recording(true);
    e.set_repeat(Some((deckard_model::Arp { rate: 240, gate: 0.5, ..Default::default() }, repeat::RepeatMode::NoteRepeat)));
    let mut out = vec![0f32; 512];
    let blocks = (960.0 * e.samples_per_tick() / 256.0).ceil() as usize;
    e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
    e.schedule(0, Live::RepeatPressure { group: 0, sound: 0, pitch: 60, velocity: 30 });
    for _ in 0..blocks {
        e.process(&mut out);
    }
    e.schedule(0, Live::Expression { group: 0, sound: 0, pitch: 60, kind: ExprKind::Dim(Dim::Pressure), value: 1.0 });
    for _ in 0..blocks {
        e.process(&mut out);
    }
    e.schedule(0, Live::NoteRelease { group: 0, sound: 0, pitch: 60 });
    e.process(&mut out);
    let rec = e.take_recorded();
    assert!(rec.len() >= 6, "{rec:?}");
    assert!(rec.iter().skip(1).all(|r| r.velocity == 30), "{:?}", rec.iter().map(|r| r.velocity).collect::<Vec<_>>());
    let exprs = e.take_recorded_expr();
    assert_eq!(exprs.len(), 1, "only the per-note pressure: {exprs:?}");
}

/// A count-in isn't the end of playback: past half a second of it (nothing sounding
/// but the clicks) the engine hasn't finished.
#[test]
fn a_count_in_is_not_finished() {
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
    let beat = (e.samples_per_tick() * PPQ as f64).round() as u64;
    e.count_in(beat * 8);
    let mut out = vec![0f32; 512];
    for _ in 0..(48_000 / 256) {
        e.process(&mut out);
    }
    assert!(!e.finished(), "a second into the count-in");
}

/// A count-in: the metronome clicks (even when it's off) and the song waits; then it
/// plays from where it was.
#[test]
fn count_in_clicks_then_the_song_starts() {
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
    let beat = (e.samples_per_tick() * PPQ as f64).round() as u64;
    e.count_in(beat * 4);
    let mut out = vec![0f32; 512];
    let mut clicked = false;
    let mut done = 0;
    while done + 256 <= beat * 4 {
        e.process(&mut out);
        clicked |= out.iter().any(|x| x.abs() > 0.01);
        assert_eq!(e.position(), 0.0, "the song waits");
        done += 256;
    }
    assert!(clicked, "the count-in clicks");
    for _ in 0..8 {
        e.process(&mut out);
    }
    assert!(e.position() > 0.0, "then it plays");
}

/// With the metronome off, AUTOENABLE has it click while recording, and only then.
#[test]
fn the_metronome_clicks_while_recording_if_set_to() {
    let clicks = |auto: bool, recording: bool| {
        let mut cache = SampleCache::default();
        // (A song that plays, its notes silent.)
        let mut p = project();
        p.groups[0].sounds[0].sampler = None;
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        e.metronome_auto = auto;
        e.set_recording(recording);
        let mut out = vec![0f32; 512];
        (0..8).any(|_| {
            e.process(&mut out);
            out.iter().any(|x| x.abs() > 0.01)
        })
    };
    assert!(clicks(true, true));
    assert!(!clicks(true, false), "not recording");
    assert!(!clicks(false, true), "not set");
}

/// The metronome's TIMESIGNATURE: eighths click twice a quarter note.
#[test]
fn the_metronome_ticks_on_its_note_value() {
    let onsets = |ticks: u32| {
        let mut cache = SampleCache::default();
        let mut p = project();
        p.groups[0].sounds[0].sampler = None;
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        (e.metronome, e.metronome_ticks) = (true, ticks);
        // (A second: two quarter notes at 120 BPM.)
        let mut out = vec![0f32; 96_000];
        for block in out.chunks_mut(512) {
            e.process(block);
        }
        (0..out.len() / 2)
            .filter(|&k| (k == 0 || out[k * 2 - 2] == 0.0) && out[k * 2] == 0.0 && out.get(k * 2 + 2).is_some_and(|v| *v != 0.0))
            .count()
    };
    assert_eq!((onsets(4), onsets(8)), (2, 4));
}

/// While recording and the song plays, live notes (pads, MIDI) come back with the
/// song position they played at, and their releases; not otherwise.
#[test]
fn live_notes_are_recorded_with_their_position() {
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
    e.looping = true;
    let mut out = vec![0f32; 512];
    e.process(&mut out);
    e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
    e.process(&mut out);
    assert!(e.take_recorded().is_empty(), "not recording");
    e.set_recording(true);
    let at = e.position();
    e.schedule(0, Live::NoteOn { group: 0, sound: 1, pitch: 62, velocity: 90, length: i64::MAX });
    e.process(&mut out);
    for _ in 0..10 {
        e.process(&mut out);
    }
    e.schedule(0, Live::NoteRelease { group: 0, sound: 1, pitch: 62 });
    e.process(&mut out);
    let rec = e.take_recorded();
    assert_eq!(rec.len(), 2, "{rec:?}");
    assert_eq!((rec[0].group, rec[0].sound, rec[0].pitch, rec[0].velocity), (0, 1, 62, 90));
    assert!((rec[0].tick - at).abs() < 1.0, "{} vs {at}", rec[0].tick);
    assert_eq!(rec[1].velocity, 0);
    let ticks_per_block = 256.0 / e.samples_per_tick();
    assert!((rec[1].tick - rec[0].tick - 11.0 * ticks_per_block).abs() < 1.0);
    assert!(e.take_recorded().is_empty(), "taken once");
}

/// A note held over the loop end: its release comes earlier in the song than its
/// start, but later on the clock that never wraps.
#[test]
fn played_keeps_counting_over_the_loop() {
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
    e.looping = true;
    e.set_recording(true);
    let mut out = vec![0f32; 512];
    e.process(&mut out);
    e.schedule(0, Live::NoteOn { group: 0, sound: 1, pitch: 62, velocity: 90, length: i64::MAX });
    e.process(&mut out);
    let mut blocks = 1;
    let mut last = e.position();
    while e.position() >= last {
        last = e.position();
        e.process(&mut out);
        blocks += 1;
    }
    e.schedule(0, Live::NoteRelease { group: 0, sound: 1, pitch: 62 });
    e.process(&mut out);
    let rec = e.take_recorded();
    assert_eq!(rec.len(), 2, "{rec:?}");
    assert!(rec[1].tick <= rec[0].tick, "the song wrapped: {rec:?}");
    let ticks_per_block = 256.0 / e.samples_per_tick();
    assert!((rec[1].played - rec[0].played - blocks as f64 * ticks_per_block).abs() < 1.0, "{rec:?}");
}

/// A count-in: a note in its last 1/16 is recorded where the song starts; one
/// earlier is heard, not recorded.
#[test]
fn the_count_ins_last_sixteenth_is_recorded_at_the_start() {
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
    let spt = e.samples_per_tick();
    let beat = (spt * PPQ as f64).round() as u64;
    e.count_in(beat * 4);
    e.set_recording(true);
    let mut out = vec![0f32; 512];
    let mut done = 0u64;
    let mut played_early = None;
    while done + 256 < beat * 4 {
        let left = (beat * 4 - done) as f64 / spt;
        if left > 600.0 && left < 700.0 && played_early.is_none() {
            e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
            played_early = Some(done);
        }
        if left < 200.0 {
            e.schedule(0, Live::NoteOn { group: 0, sound: 1, pitch: 62, velocity: 90, length: i64::MAX });
            e.process(&mut out);
            break;
        }
        e.process(&mut out);
        done += 256;
    }
    assert!(played_early.is_some());
    let rec = e.take_recorded();
    assert_eq!(rec.len(), 1, "only the late one: {rec:?}");
    assert_eq!((rec[0].sound, rec[0].tick), (1, 0.0), "at the start");
}

/// Per-note expression played live is recorded (with its song position) and not
/// played: one note's bend doesn't bend the sound (part 2 sends it on).
#[test]
fn expression_is_recorded_not_played() {
    use deckard_model::{Dim, ExprKind};
    let mut cache = SampleCache::default();
    let mut render = |bend: bool| {
        let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
        e.looping = true;
        e.set_recording(true);
        let mut out = vec![0f32; 512];
        e.process(&mut out);
        e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
        if bend {
            let kind = ExprKind::Dim(Dim::Pitch);
            e.schedule(0, Live::Expression { group: 0, sound: 0, pitch: 60, kind, value: 12.0 });
        }
        let mut all = Vec::new();
        for _ in 0..8 {
            e.process(&mut out);
            all.extend_from_slice(&out);
        }
        (all, e.take_recorded_expr())
    };
    let (plain, none) = render(false);
    let (bent, rec) = render(true);
    assert!(none.is_empty());
    assert_eq!(rec.len(), 1, "{rec:?}");
    assert_eq!((rec[0].sound, rec[0].pitch, rec[0].value), (0, 60, 12.0));
    assert_eq!(plain, bent, "not played");
}

/// A MIDI controller is recorded only when the input asks for it (the sound that
/// records it), and only while recording.
#[test]
fn control_in_records_only_when_asked() {
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
    e.looping = true;
    let mut out = vec![0f32; 512];
    e.process(&mut out);
    let cc = MidiKind::Controller { number: 1, value: 64 };
    e.schedule(0, Live::ControlIn { group: 0, sound: 0, kind: cc, thru: false, record: true });
    e.process(&mut out);
    assert!(e.take_recorded_controls().is_empty(), "not recording");
    e.set_recording(true);
    e.schedule(0, Live::ControlIn { group: 0, sound: 0, kind: cc, thru: false, record: true });
    e.schedule(0, Live::ControlIn { group: 0, sound: 1, kind: cc, thru: false, record: false });
    e.process(&mut out);
    let rec = e.take_recorded_controls();
    assert_eq!(rec.iter().map(|r| (r.sound, r.kind)).collect::<Vec<_>>(), vec![(0, cc)]);
}

/// The browser's prehear sounds while nothing plays, once, and stops.
#[test]
fn prehear_sounds_with_the_song_stopped() {
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
    assert!(e.prehear(Some(Arc::new(Sample { rate: 48_000, frames: vec![[0.5, 0.5]; 64] })), 1.0).is_none());
    let mut out = vec![0f32; 256];
    e.process(&mut out);
    assert!(out[..120].iter().all(|x| (*x - 0.5).abs() < 1e-3), "{:?}", &out[..8]);
    assert!(out[130..].iter().all(|x| *x == 0.0));
    assert!(e.prehear(None, 1.0).is_some());
}

/// Pads played live (and MIDI notes): a Send links and chokes as in the timeline
/// (`pads`): the linked sound plays too; a choked one fades out.
#[test]
fn live_pads_link_and_choke() {
    let engine = |choke: bool| {
        let mut p = project();
        let other = p.groups[0].sounds[0].clone();
        p.groups[0].sounds.push(other);
        for (i, c) in p.groups[0].sounds.iter_mut().map(|s| &mut s.channel).enumerate() {
            if choke {
                (c.choke_group, c.choke_mode) = (1, i as i64);
            } else {
                (c.link_group, c.link_mode) = (1, i as i64);
            }
        }
        let mut e = click_engine_for(&p, 1000);
        e.sequencing = false;
        set_sample(&mut e, vec![[0.5, 0.5]; 100], &[("envelope_type", 2.0)], true);
        e
    };
    let voices = |e: &Engine, s: usize| e.sounds[0][s].as_ref().map_or(0, |x| x.voices());
    let on = |sound| Live::NoteOn { group: 0, sound, pitch: 60, velocity: 100, length: i64::MAX };
    let mut out = vec![0f32; 2 * 50];
    let mut e = engine(false);
    e.play_live(on(0));
    assert_eq!((voices(&e, 0), voices(&e, 1)), (1, 1), "linked: both play");
    e.play_live(Live::NoteRelease { group: 0, sound: 0, pitch: 60 });
    e.process(&mut out);
    let mut e = engine(true);
    e.play_live(on(1));
    e.process(&mut out);
    assert_eq!(voices(&e, 1), 1);
    e.play_live(on(0));
    e.process(&mut out);
    assert_eq!((voices(&e, 0), voices(&e, 1)), (1, 0), "the Send chokes the Receive");
}

/// Choke All Notes ends what every sound plays, whatever its choke group; the next note
/// plays as usual.
#[test]
fn choke_all_ends_every_voice() {
    let mut p = project();
    let other = p.groups[0].sounds[0].clone();
    p.groups[0].sounds.push(other);
    let mut e = click_engine_for(&p, 1000);
    e.sequencing = false;
    set_sample(&mut e, vec![[0.5, 0.5]; 100], &[("envelope_type", 2.0)], true);
    let voices = |e: &Engine, s: usize| e.sounds[0][s].as_ref().map_or(0, |x| x.voices());
    let on = |sound| Live::NoteOn { group: 0, sound, pitch: 60, velocity: 100, length: i64::MAX };
    let mut out = vec![0f32; 2 * 50];
    e.play_live(on(0));
    e.play_live(on(1));
    e.process(&mut out);
    assert_eq!((voices(&e, 0), voices(&e, 1)), (1, 1));
    e.play_live(Live::ChokeAll);
    e.process(&mut out);
    assert_eq!((voices(&e, 0), voices(&e, 1)), (0, 0), "both end");
    e.play_live(on(0));
    e.process(&mut out);
    assert_eq!(voices(&e, 0), 1, "the next note plays");
}

/// #21: a sound that plays only by a link is silent while the sound with the notes is
/// muted; muted alone, it's silent and the Send still plays.
#[test]
fn muting_the_send_silences_its_links() {
    let heard = |mute: usize| {
        let mut p = project();
        let mut other = p.groups[0].sounds[0].clone();
        other.channel.link_mode = 1;
        p.groups[0].sounds.push(other);
        p.groups[0].sounds[0].channel.link_group = 1;
        p.groups[0].sounds[1].channel.link_group = 1;
        p.groups[0].sounds[mute].muted = true;
        let mut e = click_engine_for(&p, 1000);
        render(&mut e, 2.0).iter().any(|v| v.abs() > 0.01)
    };
    assert!(!heard(0), "the Send muted: its link is silent too");
    assert!(heard(1), "the Receive muted: the Send still plays");
}

#[test]
fn shaker_performer_and_held_notes_release_on_mute() {
    let mut playing = project();
    playing.groups[0].sounds[0].sampler = None;
    playing.groups[0].sounds[0].instrument = "Perc".into();
    playing.groups[0].sounds[0].slots = vec![deckard_model::SlotInfo {
        module: "Perc".into(),
        name: "Perc".into(),
        active: true,
        plugin: None,
        params: vec![("mode".into(), 2.0), ("shaker_env_mode".into(), 2.0), ("shaker_release".into(), 0.1)],
    }];
    playing.groups[0].patterns[0] = Pattern {
        length: 4 * PPQ,
        notes: vec![Note { sound: 0, position: 0, length: 4 * PPQ, pitch: 60, velocity: 127, probability: 100, ..Default::default() }],
        ..Default::default()
    };
    let mut muted = playing.clone();
    muted.groups[0].sounds[0].muted = true;

    let peak_of = |buf: &[f32]| buf.iter().fold(0f32, |m, v| m.max(v.abs()));

    for (with_echo, ahead) in [(false, None), (true, None), (false, Some(512u64))] {
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&playing, None, 48_000, Mode::Scene(0), &mut cache);
        if with_echo {
            e.set_midi_input(0, midi_input::built_in(note_echo::NAME));
        }
        if let Some(la) = ahead {
            e.set_focus(Some(1));
            e.render_ahead(la, 1, 256, true);
        }
        e.jump(&playing, Mode::Scene(0));
        e.looping = true;

        // 1) Play ~250 ms into the 1-bar Performer note, stop transport mid-note, then
        // restart: Performer must not latch its held-note count across the stop.
        let mut buf = vec![0f32; 2 * 256];
        let mut pre_peak = 0f32;
        for _ in 0..48 {
            e.process(&mut buf);
            pre_peak = pre_peak.max(peak_of(&buf));
        }
        assert!(pre_peak > 0.01, "Performer shaking before stop ({with_echo}, {ahead:?}): {pre_peak}");

        e.stop();
        for _ in 0..20 {
            e.process(&mut buf);
        }
        e.jump(&playing, Mode::Scene(0));
        if with_echo {
            e.note_hold(0, 0, 64, 100);
        }
        let mut resumed = 0f32;
        for _ in 0..48 {
            e.process(&mut buf);
            resumed = resumed.max(peak_of(&buf));
        }
        assert!(resumed > 0.01, "Performer shaking after restart ({with_echo}, {ahead:?}): {resumed}");

        // 2) Mute mid-bar: Performer releases immediately and queued Note Echo repeats
        // are cancelled instead of shaking through the rest of the bar.
        let (fresh, _) = Engine::new_after(&muted, Some((&playing, &[])), None, 48_000, Mode::Scene(0), &mut cache);
        drop(e.adopt(fresh, &muted));
        for _ in 0..40 {
            e.process(&mut buf);
        }
        let mut muted_peak = 0f32;
        for _ in 0..120 {
            e.process(&mut buf);
            muted_peak = muted_peak.max(peak_of(&buf));
        }
        assert!(muted_peak < 1e-4, "Performer silent while muted across bar ({with_echo}, {ahead:?}): {muted_peak}");
    }

    // 3) Gated Sampler, AudioModule Gate mode, and MIDI-out sounds also release held
    // notes on mute while one-shot Sampler voices ring out naturally.
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("tone.wav");
    edit::write_wav(&wav, &sample::Sample { rate: 1000, frames: vec![[0.6, 0.6]; 2000] }).unwrap();

    let mut smp_proj = project();
    smp_proj.groups[0].patterns[0].notes.clear();
    let other = smp_proj.groups[0].sounds[0].clone();
    smp_proj.groups[0].sounds.push(other.clone());
    smp_proj.groups[0].sounds.push(other);
    // Sound 0: ADSR gated looped Sampler; Sound 1: One-shot Sampler; Sound 2: AudioModule Gate + MIDI out.
    {
        let smp = smp_proj.groups[0].sounds[0].sampler.as_mut().unwrap();
        smp.zones[0].sample = deckard_model::SampleRef { kind: 0, path: wav.to_string_lossy().into(), ..Default::default() };
        smp.zones[0].play_range = (0, 1999);
        smp.zones[0].loop_active = true;
        smp.zones[0].loop_range = (0, 1999);
        smp.set("envelope_type", 2.0);
        smp.set("volume_envelope/sustain", 1.0);
        smp.set("volume_envelope/release", 0.0);
    }
    {
        let smp = smp_proj.groups[0].sounds[1].sampler.as_mut().unwrap();
        smp.zones[0].sample = deckard_model::SampleRef { kind: 0, path: wav.to_string_lossy().into(), ..Default::default() };
        smp.zones[0].play_range = (0, 1999);
        smp.set("envelope_type", 0.0);
    }
    {
        let s2 = &mut smp_proj.groups[0].sounds[2];
        s2.sampler = None;
        s2.instrument = "AudioModule".into();
        s2.audio = Some(deckard_model::AudioLoop {
            mode: deckard_model::AudioMode::Gate,
            takes: vec![deckard_model::Take {
                sample: deckard_model::SampleRef { kind: 0, path: wav.to_string_lossy().into(), ..Default::default() },
                beats: 4.0,
                play_range: (0, 0),
            }],
            pitchbend: 0.5,
            fade: 0.0,
            ..Default::default()
        });
        s2.channel.midi_out = deckard_model::MidiDest::Port("Synth".into());
        s2.channel.midi_out_channel = 0;
    }
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&smp_proj, None, 1000, Mode::Scene(0), &mut cache);
    let (tx, rx) = std::sync::mpsc::sync_channel(16);
    e.midi_tx = Some(tx);
    e.note_hold(0, 0, 60, 127);
    e.note_on(0, 1, 60, 127, 200);
    e.note_hold(0, 2, 60, 100);
    let mut b = vec![0f32; 2 * 50];
    e.process(&mut b);
    assert_eq!(rx.try_iter().map(|m| m.bytes().to_vec()).collect::<Vec<_>>(), vec![vec![0x90, 60, 100]]);

    let mut smp_muted = smp_proj.clone();
    smp_muted.groups[0].sounds[0].muted = true;
    smp_muted.groups[0].sounds[1].muted = true;
    smp_muted.groups[0].sounds[2].muted = true;
    let (fresh, _) = Engine::new_after(&smp_muted, Some((&smp_proj, &[])), None, 1000, Mode::Scene(0), &mut cache);
    drop(e.adopt(fresh, &smp_muted));
    assert_eq!(rx.try_iter().map(|m| m.bytes().to_vec()).collect::<Vec<_>>(), vec![vec![0x80, 60, 0]], "MIDI out note ended on mute");

    let mut after = vec![0f32; 2 * 200];
    e.process(&mut after);
    assert_eq!(e.sounds[0][0].as_ref().unwrap().voices(), 0, "gated ADSR sampler released");
    assert_eq!(e.sounds[0][2].as_ref().unwrap().voices(), 0, "AudioModule gate released");
    assert_eq!(e.sounds[0][1].as_ref().unwrap().voices(), 1, "one-shot sampler keeps ringing out");
}

#[test]
fn onsets_are_sample_accurate() {
    // 120 BPM: a quarter note is 0.5 s = 24000 frames at 48 kHz.
    let mut e = click_engine(48_000);
    let out = render(&mut e, 10.0);
    let onsets: Vec<usize> = (0..out.len() / 2).filter(|&i| out[i * 2] > 0.5).collect();
    assert_eq!(onsets, (0..8).map(|i| i * 24_000).collect::<Vec<_>>());
}

#[test]
fn octave_up_plays_twice_as_fast() {
    let sr = 1000;
    let mut e = click_engine(sr);
    // a ramp sample, triggered an octave up: every other frame
    // A ramp, scaled to stay within the engine's output range (±4).
    let ramp: Vec<[f32; 2]> = (0..100).map(|i| [i as f32 / 100.0, i as f32 / 100.0]).collect();
    set_sample(&mut e, ramp, &[], false);
    e.timeline = Timeline {
        ratchets: 0,
        events: vec![NoteEvent {
            expr_at: 0,
            roll: None,
            tick: 0,
            group: 0,
            sound: 0,
            pitch: 72,
            velocity: 127,
            length: 1,
            probability: 100,
            midi: None,
            cue: None,
            expr: None,
            from: None,
        }],
        length: 1,
        mods: Vec::new(),
        faders: Vec::new(),
        exprs: Vec::new(),
        prob_locked: Vec::new(),
        tempos: Vec::new(),
        switch_every: 0,
    };
    let out = render(&mut e, 1.0);
    let left: Vec<f32> = out.iter().step_by(2).map(|x| x * 100.0).collect();
    for (i, want) in [0.0, 2.0, 4.0, 6.0, 8.0].iter().enumerate() {
        assert!((left[i] - want).abs() < 1e-3, "frame {i}: {}", left[i]);
    }
    assert!((left[49] - 98.0).abs() < 1e-2 && left[50].abs() < 1e-2, "voice ends at the sample end: {:?}", &left[48..52]);
}

#[test]
fn fifth_up_interpolates() {
    let sr = 1000;
    let mut e = click_engine(sr);
    let ramp: Vec<[f32; 2]> = (0..100).map(|i| [i as f32 / 100.0, i as f32 / 100.0]).collect();
    set_sample(&mut e, ramp, &[], false);
    e.timeline = Timeline {
        ratchets: 0,
        events: vec![NoteEvent {
            expr_at: 0,
            roll: None,
            tick: 0,
            group: 0,
            sound: 0,
            pitch: 67,
            velocity: 127,
            length: 1,
            probability: 100,
            midi: None,
            cue: None,
            expr: None,
            from: None,
        }],
        length: 1,
        mods: Vec::new(),
        faders: Vec::new(),
        exprs: Vec::new(),
        prob_locked: Vec::new(),
        tempos: Vec::new(),
        switch_every: 0,
    };
    let out = render(&mut e, 1.0);
    // The 8-point interpolator follows a ramp once its history is filled.
    let ratio = 2f32.powf(7.0 / 12.0);
    for (i, &l) in out.iter().step_by(2).enumerate().skip(4).take(20) {
        assert!((l * 100.0 - i as f32 * ratio).abs() < 0.02, "frame {i}: {l}");
    }
}

#[test]
fn a_note_played_live_is_reported_and_a_sequenced_one_isnt() {
    let mut e = click_engine(1000);
    let mut out = vec![0f32; 512];
    e.note_on(0, 1, 60, 100, PPQ);
    assert_eq!(e.live_hit.0, 0, "the sequence's way in");
    e.schedule(0, Live::NoteOn { group: 0, sound: 1, pitch: 72, velocity: 90, length: PPQ });
    e.process(&mut out);
    assert_eq!(e.live_hit, (1, 0, 1, 72, 90));
    e.schedule(0, Live::NoteIn { group: 2, sound: 3, pitch: 40, velocity: 30, thru: false });
    e.process(&mut out);
    assert_eq!(e.live_hit, (2, 2, 3, 40, 30));
}

/// What's recorded says whether it came in over MIDI.
#[test]
fn a_recorded_note_says_if_it_was_midi() {
    let mut e = click_engine(1000);
    e.set_recording(true);
    let mut out = vec![0f32; 512];
    e.schedule(0, Live::NoteOn { group: 0, sound: 1, pitch: 72, velocity: 90, length: PPQ });
    e.schedule(0, Live::NoteIn { group: 0, sound: 0, pitch: 40, velocity: 30, thru: false });
    e.process(&mut out);
    let by_pitch = |p: i64| e.recorded.iter().find(|r| r.pitch == p).expect("recorded");
    assert!(!by_pitch(72).midi, "a pad");
    assert!(by_pitch(40).midi);
}

#[test]
fn note_on_plays_without_the_sequencer() {
    let mut e = click_engine(1000);
    e.sequencing = false;
    let mut out = vec![0f32; 200];
    e.process(&mut out[..100]);
    assert!(out.iter().all(|s| *s == 0.0));
    e.note_on(0, 0, 60, 127, PPQ);
    e.process(&mut out[100..]);
    assert_eq!(out[100], 1.0, "the click starts at the next block");
    assert_eq!(e.position(), 0.0, "the song position doesn't move");
}

#[test]
fn tap_takes_a_sound_its_group_or_the_master() {
    for (source, hears) in [
        (TapSource::Sound(0, 0), true),
        (TapSource::Group(0), true),
        (TapSource::Master, true),
        (TapSource::Group(1), false),
        (TapSource::Sound(0, 1), false),
    ] {
        let mut e = click_engine(1000);
        e.sequencing = false;
        let buffer = Arc::new(Mutex::new(Vec::new()));
        e.set_tap(Some(Tap { source, buffer: buffer.clone() }));
        e.note_on(0, 0, 60, 127, PPQ);
        let mut out = vec![0f32; 200];
        e.process(&mut out);
        let got = buffer.lock().unwrap().clone();
        assert_eq!(got.len(), out.len(), "{source:?}: every frame is handed over");
        if hears {
            assert_eq!(got, out, "{source:?}: the tap hears what plays");
        } else {
            assert!(got.iter().all(|s| *s == 0.0), "{source:?}: silence");
        }
    }
}

#[test]
fn a_scene_plays_at_its_own_tempo_from_when_it_starts() {
    let mut p = project();
    // Scene 1, the same quarters, at twice the song's tempo.
    p.scenes.push(Scene { patterns: vec![(0, 0)], tempo: Some(240.0), ..Default::default() });
    // 120 BPM at 1 kHz: a quarter is 500 frames, a bar 2000; at 240, 250 and 1000.
    let mut e = click_engine_for(&p, 1000);
    e.jump(&p, Mode::Scene(0));
    e.looping = true;
    let mut out = vec![0f32; 2 * 3500];
    e.process(&mut out[..2 * 700]);
    assert_eq!(e.tempo(), 120.0);
    e.queue(&p, Mode::Scene(1));
    e.process(&mut out[2 * 700..2 * 2100]);
    assert_eq!(e.tempo(), 240.0, "the scene's, once it starts");
    // Back: the song's again, on the bar.
    e.queue(&p, Mode::Scene(0));
    e.process(&mut out[2 * 2100..]);
    let onsets: Vec<usize> = (0..3500).filter(|&i| out[i * 2] > 0.5).collect();
    assert_eq!(onsets, [0, 500, 1000, 1500, 2000, 2250, 2500, 2750, 3000]);
    assert_eq!(e.tempo(), 120.0);
}

#[test]
fn a_section_plays_at_its_scenes_tempo_and_the_timeline_knows_how_long_that_is() {
    let mut p = project();
    p.scenes.push(Scene { patterns: vec![(0, 0)], tempo: Some(240.0), ..Default::default() });
    let bar = 4 * PPQ;
    p.sections = vec![
        Section { scene: Some(0), length: bar, auto: false },
        Section { scene: Some(1), length: bar, auto: false },
        Section { scene: Some(0), length: bar, auto: false },
    ];
    let mut e = click_engine_for(&p, 1000);
    e.jump(&p, Mode::Arrangement);
    assert_eq!(e.timeline_frames().round() as usize, 2000 + 1000 + 2000);
    let mut out = vec![0f32; 2 * 3400];
    e.process(&mut out);
    let onsets: Vec<usize> = (0..3400).filter(|&i| out[i * 2] > 0.5).collect();
    assert_eq!(onsets, [0, 500, 1000, 1500, 2000, 2250, 2500, 2750, 3000]);
    assert_eq!(e.tempo(), 120.0);
}

/// Under a clock (Link, incoming MIDI clock) the clock's tempo is the tempo.
#[test]
fn a_followed_clock_overrides_a_scenes_tempo() {
    let mut p = project();
    p.scenes[0].tempo = Some(240.0);
    let mut e = click_engine_for(&p, 1000);
    e.jump(&p, Mode::Scene(0));
    e.set_followed(true);
    let mut out = vec![0f32; 2 * 100];
    e.process(&mut out);
    assert_eq!(e.tempo(), 120.0);
    e.set_followed(false);
    e.process(&mut out);
    assert_eq!(e.tempo(), 240.0);
}

/// A tempo edit moves what plays at the song's tempo, not a scene with its own.
#[test]
fn a_song_tempo_edit_leaves_a_scene_with_its_own_alone() {
    let mut p = project();
    p.scenes[0].tempo = Some(90.0);
    let mut e = click_engine_for(&p, 1000);
    e.jump(&p, Mode::Scene(0));
    let mut out = vec![0f32; 2 * 100];
    e.process(&mut out);
    assert_eq!(e.tempo(), 90.0);
    p.tempo = 200.0;
    let mut cache = SampleCache::default();
    e.update(&p, None, &mut cache);
    e.process(&mut out);
    assert_eq!(e.tempo(), 90.0);
}

#[test]
fn swing_delays_off_beat_sixteenths() {
    let mut p = project();
    // Sixteenths on beat 1: ticks 0, 240, 480, 720.
    p.groups[0].patterns[0].notes = (0..4)
        .map(|i| Note {
            ratchet: Default::default(),
            expr: None,
            sound: 0,
            position: i * 240,
            length: 60,
            pitch: 60,
            velocity: 127,
            probability: 100,
        })
        .collect();
    p.groups[0].swing = deckard_model::Swing { amount: 100.0, ..Default::default() };
    let ticks: Vec<i64> = timeline(&p, Mode::Pattern(0, 0)).events.iter().take(4).map(|e| e.tick).collect();
    // The 1/8 cycle is 480 ticks; its midpoint (240) moves to 95 %.
    assert_eq!(ticks, [0, 456, 480, 936]);
}

#[test]
fn meters_report_what_each_sound_played() {
    let mut e = click_engine(1000);
    let mut out = vec![0f32; 2 * 100];
    e.process(&mut out);
    let m = e.take_meters();
    assert_eq!(m[0][0], (1.0, 1.0), "the click's peak");
    e.process(&mut out[..2]);
    assert_eq!(e.take_meters()[0][0], (0.0, 0.0), "silence since");
}

#[test]
fn pan_law_is_unity_at_center() {
    let (l, r) = pan_gains(0.0);
    assert!((l - 1.0).abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
    let (l, r) = pan_gains(-1.0);
    assert!(r.abs() < 1e-6 && l > 1.0);
}

/// A master level change while playing glides (the mix doesn't jump, which zips when
/// a knob sends steps); steady playback is exact once it gets there.
#[test]
fn the_masters_level_glides_to_a_new_setting() {
    let mut x = Strips::new(|_| {});
    let (before, _) = x.block(0.5);
    let target = deckard_model::level_from_db(-12.0);
    let from = x.p.master_level;
    x.edit(|p| p.master_level = target);
    let mut frame = [0f32; 32];
    (frame[0], frame[1]) = (0.5, 0.5);
    x.e.live_in.as_ref().unwrap().lock().unwrap().extend(std::iter::repeat_n(frame, 64));
    let mut out = vec![0f32; 128];
    let mut ext = vec![vec![0f32; 128]; OUTPUTS];
    x.e.process_outputs(&mut out, &mut ext);
    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let after = before.0 * deckard_model::level_gain(target) as f32 / deckard_model::level_gain(from) as f32;
    assert!(left[0] > before.0 * 0.9, "no jump on the first frame: {} vs {}", left[0], before.0);
    assert!(left.windows(2).all(|w| w[1] <= w[0] + 1e-6), "it falls steadily: {left:?}");
    assert!(about(left[63], after), "it gets there within the block: {} vs {after}", left[63]);
    assert!(about(x.block(0.5).0.0, after));
}

/// A plug-in's note-off never comes before the attack it queued for the same note
/// (a key let go before its quantized or arp attack): it moves to that attack.
#[test]
fn a_plug_in_off_waits_for_its_queued_attack() {
    use midi_input::NoteOut::On;
    let on = |pitch| Live::MidiInputOut { group: 2, note: On { at: 0.0, sound: 0, pitch, velocity: 100, length: None } };
    let live = vec![(180, on(60)), (150, on(64)), (90, on(60))];
    let same = |e: &Live| matches!(*e, Live::MidiInputOut { group: 2, note: On { sound: 0, pitch: 60, .. } });
    assert_eq!(after_queued_on(&live, 100, 0, same), 80, "to its latest queued attack");
    assert_eq!(after_queued_on(&live, 100, 90, same), 90, "an off already after it stays");
    assert_eq!(after_queued_on(&[], 100, 0, same), 0);
}

/// A new attack for a pitch removes that pitch's off still queued from the note before
/// (a plug-in plays a block's offs first), or the old off would end the new note.
#[test]
fn a_plug_ins_new_attack_drops_the_stale_off_queued_after_it() {
    use midi_input::NoteOut::{Off, On};
    let mut e = click_engine_for(&project(), 1000);
    e.sequencing = false;
    let off = |at, pitch| Off { at, sound: 0, pitch };
    e.play_midi_input_out(0, 0.0, &[off(8.0, 60), off(8.0, 64)]);
    assert_eq!(e.live.len(), 2);
    e.play_midi_input_out(0, 0.0, &[On { at: 4.0, sound: 0, pitch: 60, velocity: 100, length: None }]);
    let pitches: Vec<i64> = e
        .live
        .iter()
        .filter_map(|(_, l)| match l {
            Live::MidiInputOut { note: Off { pitch, .. }, .. } => Some(*pitch),
            _ => None,
        })
        .collect();
    assert_eq!(pitches, [64], "the later off for 60 is gone; another pitch's stays");
}

/// Choke only one Group's sounds: other Groups' voices keep playing.
#[test]
fn choke_group_ends_only_that_groups_voices() {
    let mut p = project();
    let other = p.groups[0].sounds[0].clone();
    p.groups[0].sounds.push(other);
    let mut e = click_engine_for(&p, 1000);
    e.sequencing = false;
    set_sample(&mut e, vec![[0.5, 0.5]; 100], &[("envelope_type", 2.0)], true);
    let voices = |e: &Engine, s: usize| e.sounds[0][s].as_ref().map_or(0, |x| x.voices());
    let on = |sound| Live::NoteOn { group: 0, sound, pitch: 60, velocity: 100, length: i64::MAX };
    let mut out = vec![0f32; 2 * 50];
    e.play_live(on(0));
    e.play_live(on(1));
    e.process(&mut out);
    e.play_live(Live::ChokeGroup { group: 5 });
    e.process(&mut out);
    assert_eq!((voices(&e, 0), voices(&e, 1)), (1, 1), "another (absent) Group's choke leaves these playing");
    e.play_live(Live::ChokeGroup { group: 0 });
    e.process(&mut out);
    assert_eq!((voices(&e, 0), voices(&e, 1)), (0, 0), "its own Group's choke ends both");
    // One sound's choke ends its own voices only.
    e.play_live(on(0));
    e.play_live(on(1));
    e.process(&mut out);
    e.play_live(Live::ChokeSound { group: 0, sound: 1 });
    e.play_live(Live::ChokeSound { group: 0, sound: 99 });
    e.process(&mut out);
    assert_eq!((voices(&e, 0), voices(&e, 1)), (1, 0), "the sound's choke ends its voices, not its neighbour's");
}

#[test]
fn telemetry_tracks_active_sampler_loops_until_released_or_choked() {
    let mut p = project();
    let s0 = p.groups[0].sounds[0].clone();
    p.groups[0].sounds.push(s0);
    p.groups[0].sounds[0].channel.choke_group = 1;
    p.groups[0].sounds[0].channel.choke_mode = 0;
    p.groups[0].sounds[1].channel.choke_group = 1;
    p.groups[0].sounds[1].channel.choke_mode = 0;
    let mut e = click_engine_for(&p, 48_000);
    e.sequencing = false;
    set_sample(&mut e, vec![[0.4, 0.4]; 24_000], &[("envelope_type", 2.0)], true);
    // Give sound 1 a non-looped one-shot in Choke Group 1.
    let s1_smp = deckard_model::Sampler { envelope_type: 0, ..Default::default() };
    let z1 = sampler::ZoneRt {
        sample: Arc::new(sample::Sample { rate: 48_000, frames: vec![[0.4, 0.4]; 12_000] }),
        root: 60,
        keys: (0, 127),
        vels: (0, 127),
        gain: 1.0,
        pan: 0.0,
        tune: 0.0,
        play: (0, 12_000),
        looped: None,
        x_fade: 0,
        fade_in: 0,
        fade_out: 0,
        sync: None,
        lanes: None,
        reverse: false,
        once: false,
        tail_ms: 0.0,
        layer: None,
    };
    e.sounds[0][1] = Some(SamplerRt::new(sampler::Settings::new(&s1_smp, 48_000.0), vec![z1], (1.0, 1.0), 48_000.0));

    let mut out = vec![0.0f32; 2 * 512];
    let mut t = Telemetry::with_room();

    e.play_live(Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
    e.process(&mut out);
    e.publish(&mut t);
    assert_eq!(t.loop_playing[0] & 1, 1, "looped sustaining voice publishes loop_playing");

    // Choking with sound 1 (Choke Group 1) stops sound 0's loop highlight and leaves non-looped sound 1 unflagged.
    e.play_live(Live::NoteOn { group: 0, sound: 1, pitch: 60, velocity: 100, length: i64::MAX });
    e.process(&mut out);
    e.publish(&mut t);
    assert_eq!(t.loop_playing[0] & 1, 0, "choked loop clears loop_playing immediately");
    assert_eq!(t.loop_playing[0] & 2, 0, "non-looped one-shot does not set loop_playing");
}
