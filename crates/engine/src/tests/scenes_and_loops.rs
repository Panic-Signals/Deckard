use super::*;

fn loop_engine(p: &Project, cache: &mut SampleCache) -> Engine {
    let (mut e, _) = Engine::new(p, None, 1000, Mode::Scene(0), cache);
    e.jump(p, Mode::Scene(0));
    e.looping = true;
    e
}

fn adopt_at_1k(e: &mut Engine, p: &Project, q: &Project, cache: &mut SampleCache) {
    let (fresh, _) = Engine::new_after(q, Some((p, &[])), None, 1000, Mode::Scene(0), cache);
    drop(e.adopt(fresh, q));
}

/// A pattern turned off while the song plays: its long one-shot sample (2 s, longer
/// than a beat) stops at once, though a one-shot plays past its note's end; its short
/// hit (0.2 s) rings out.
#[test]
fn a_pattern_turned_off_stops_its_long_samples_and_lets_hits_ring() {
    let dir = tempfile::tempdir().unwrap();
    let (long, hit) = (dir.path().join("long.wav"), dir.path().join("hit.wav"));
    edit::write_wav(&long, &sample::Sample { rate: 1000, frames: vec![[0.5, 0.5]; 2000] }).unwrap();
    edit::write_wav(&hit, &sample::Sample { rate: 1000, frames: vec![[0.5, 0.5]; 200] }).unwrap();
    let mut p = project();
    let second = p.groups[0].sounds[0].clone();
    p.groups[0].sounds.push(second);
    for (s, (wav, len)) in [(&long, 2000), (&hit, 200)].into_iter().enumerate() {
        let smp = p.groups[0].sounds[s].sampler.as_mut().unwrap();
        smp.zones[0].sample = deckard_model::SampleRef { kind: 0, path: wav.to_string_lossy().into(), ..Default::default() };
        smp.zones[0].play_range = (0, len - 1);
        smp.set("envelope_type", 0.0);
    }
    p.groups[0].patterns[0].notes = (0..2)
        .map(|sound| Note {
            ratchet: Default::default(),
            expr: None,
            sound,
            position: 0,
            length: PPQ / 4,
            pitch: 60,
            velocity: 127,
            probability: 100,
        })
        .collect();
    let mut off = p.clone();
    off.scenes[0].patterns.clear();
    let mut cache = SampleCache::default();
    let mut e = loop_engine(&p, &mut cache);
    let voices = |e: &Engine, s: usize| e.sounds[0][s].as_ref().unwrap().voices();
    // Past the sixteenth's note-off: both one-shots still play.
    let mut out = vec![0f32; 2 * 150];
    e.process(&mut out);
    assert!(voices(&e, 0) > 0 && voices(&e, 1) > 0, "both playing");
    adopt_at_1k(&mut e, &p, &off, &mut cache);
    let mut out = vec![0f32; 2 * 25];
    e.process(&mut out);
    assert_eq!(voices(&e, 0), 0, "the long sample stops within its 10 ms fade");
    assert!(voices(&e, 1) > 0, "the hit rings out");
    // The same sample on its pad, played live, isn't the pattern's: it plays on.
    let mut e = loop_engine(&p, &mut cache);
    let mut out = vec![0f32; 2 * 150];
    e.process(&mut out);
    e.note_on(0, 0, 62, 127, PPQ / 4);
    e.process(&mut out[..2 * 10]);
    assert_eq!(voices(&e, 0), 2, "the pattern's and the pad's");
    adopt_at_1k(&mut e, &p, &off, &mut cache);
    let mut out = vec![0f32; 2 * 25];
    e.process(&mut out);
    assert_eq!(voices(&e, 0), 1, "the pad's plays on");
}

/// A loop recorded into a pattern: a bar-long note on a sustaining Sampler, its sample
/// a slow ramp (the level says how far in it is), at 1 kHz and 120 BPM (a bar is 2000
/// frames). `on`: its pattern is in the scene.
fn loop_project(dir: &Path, on: bool) -> Project {
    let wav = dir.join("loop.wav");
    if !wav.exists() {
        let frames = (0..8000).map(|i| [i as f32 / 10_000.0, i as f32 / 10_000.0]).collect();
        edit::write_wav(&wav, &sample::Sample { rate: 1000, frames }).unwrap();
    }
    let mut p = project();
    let smp = p.groups[0].sounds[0].sampler.as_mut().unwrap();
    smp.zones[0].sample = deckard_model::SampleRef { kind: 0, path: wav.to_string_lossy().into(), ..Default::default() };
    smp.zones[0].play_range = (0, 7999);
    smp.set("envelope_type", 2.0);
    smp.set("volume_envelope/release", 0.0);
    p.groups[0].patterns[0].notes = vec![Note {
        ratchet: Default::default(),
        expr: None,
        sound: 0,
        position: 0,
        length: 4 * PPQ,
        pitch: 60,
        velocity: 127,
        probability: 100,
    }];
    if !on {
        p.scenes[0].patterns.clear();
    }
    p
}

/// The loop's pattern taken out of the scene mid-bar: it stops at once, not at the
/// end of its note (the bar's end).
#[test]
fn a_loop_stops_when_its_pattern_is_taken_out() {
    let dir = tempfile::tempdir().unwrap();
    let (p, q) = (loop_project(dir.path(), true), loop_project(dir.path(), false));
    let mut cache = SampleCache::default();
    let mut e = loop_engine(&p, &mut cache);
    let mut out = vec![0f32; 2 * 1000];
    e.process(&mut out);
    assert!(out[2 * 999] > 0.01, "the loop plays: {}", out[2 * 999]);
    adopt_at_1k(&mut e, &p, &q, &mut cache);
    let mut out = vec![0f32; 2 * 500];
    e.process(&mut out);
    let loudest = out[2 * 20..].iter().fold(0f32, |m, x| m.max(x.abs()));
    assert!(loudest < 1e-4, "silent within 20 ms, not at the bar's end: {loudest}");
}

/// Put back in mid-bar: it plays at once, from where it would be had it played all
/// along, not from the next bar. Short notes (hits) still wait for their next time.
#[test]
fn a_loop_comes_back_where_it_would_be_when_its_pattern_is_put_in() {
    let dir = tempfile::tempdir().unwrap();
    let (p, q) = (loop_project(dir.path(), false), loop_project(dir.path(), true));
    let mut cache = SampleCache::default();
    // As it sounds having played from the start.
    let mut all_along = loop_engine(&q, &mut cache);
    let mut want = vec![0f32; 2 * 1500];
    all_along.process(&mut want);
    let mut e = loop_engine(&p, &mut cache);
    let mut out = vec![0f32; 2 * 1500];
    e.process(&mut out[..2 * 1000]);
    assert!(out[..2 * 1000].iter().all(|x| x.abs() < 1e-6), "nothing before");
    adopt_at_1k(&mut e, &p, &q, &mut cache);
    e.process(&mut out[2 * 1000..]);
    for i in [1020, 1200, 1499] {
        assert!(out[2 * i] > 0.01, "playing at frame {i}: {}", out[2 * i]);
        assert!((out[2 * i] - want[2 * i]).abs() < 2e-3, "at frame {i}: {} where it'd be {}", out[2 * i], want[2 * i]);
    }
    let mut hits = q.clone();
    hits.groups[0].patterns[0].notes[0].length = 240;
    let mut e = loop_engine(&p, &mut cache);
    let mut out = vec![0f32; 2 * 1200];
    e.process(&mut out[..2 * 100]);
    adopt_at_1k(&mut e, &p, &hits, &mut cache);
    e.process(&mut out[2 * 100..]);
    assert!(out.iter().all(|x| x.abs() < 1e-6), "a hit isn't joined late");
}

/// The group switched mid-bar to another pattern with a loop of its own: the new
/// pattern's loop is joined where it would be and plays on (the old pattern's long
/// samples stop; the one just joined isn't among them).
#[test]
fn a_loop_joined_on_a_pattern_switch_plays_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = loop_project(dir.path(), true);
    let mut other = p.groups[0].patterns[0].clone();
    other.slot = 1;
    other.notes[0].pitch = 62;
    p.groups[0].patterns.push(other);
    let mut q = p.clone();
    q.scenes[0].patterns = vec![(0, 1)];
    let mut cache = SampleCache::default();
    let mut e = loop_engine(&p, &mut cache);
    let mut out = vec![0f32; 2 * 1000];
    e.process(&mut out);
    adopt_at_1k(&mut e, &p, &q, &mut cache);
    let mut out = vec![0f32; 2 * 500];
    e.process(&mut out);
    assert_eq!(e.sounds[0][0].as_ref().unwrap().voices(), 1, "the new pattern's loop plays");
    assert!(out[2 * 499].abs() > 0.01, "and is heard: {}", out[2 * 499]);
}
#[test]
fn beat_grace_catches_on_beat_unmute_and_pattern_switch_without_double_firing() {
    let dir = tempfile::tempdir().unwrap();
    let kick_wav = dir.path().join("kick.wav");
    let hat_wav = dir.path().join("hat.wav");
    edit::write_wav(&kick_wav, &sample::Sample { rate: 1000, frames: vec![[0.6, 0.0]; 200] }).unwrap();
    edit::write_wav(&hat_wav, &sample::Sample { rate: 1000, frames: vec![[0.0, 0.4]; 200] }).unwrap();

    let mut base = project();
    let hat_sound = base.groups[0].sounds[0].clone();
    base.groups[0].sounds.push(hat_sound);
    for (s, wav) in [(0, &kick_wav), (1, &hat_wav)] {
        let smp = base.groups[0].sounds[s].sampler.as_mut().unwrap();
        smp.zones[0].sample = deckard_model::SampleRef { kind: 0, path: wav.to_string_lossy().into(), ..Default::default() };
        smp.zones[0].play_range = (0, 199);
        smp.set("envelope_type", 0.0); // one-shot
    }
    base.groups[0].patterns[0].notes = vec![
        Note { ratchet: Default::default(), expr: None, sound: 0, position: 0, length: 240, pitch: 60, velocity: 127, probability: 100 },
        Note { ratchet: Default::default(), expr: None, sound: 1, position: 0, length: 240, pitch: 60, velocity: 127, probability: 100 },
    ];

    let mut cache = SampleCache::default();
    // 1) Unmuting sound 0 25 ms after beat 0 catches beat 0's kick without re-triggering sound 1's hat.
    let mut muted_kick = base.clone();
    muted_kick.groups[0].sounds[0].muted = true;
    let mut e = loop_engine(&muted_kick, &mut cache);
    let mut before = vec![0f32; 2 * 25];
    e.process(&mut before);
    assert!(before.chunks_exact(2).all(|f| f[0].abs() < 1e-6), "muted kick silent before unmute");
    let hat_before = before[2 * 20 + 1];
    assert!(hat_before > 0.1, "hat playing on right channel: {hat_before}");

    adopt_at_1k(&mut e, &muted_kick, &base, &mut cache);
    let mut after = vec![0f32; 2 * 60];
    e.process(&mut after);
    let kick_peak = after.chunks_exact(2).map(|f| f[0]).fold(0f32, f32::max);
    let hat_peak = after.chunks_exact(2).map(|f| f[1]).fold(0f32, f32::max);
    assert!(kick_peak > 0.2, "on-beat unmute catches beat 0 kick: {kick_peak}");
    assert!((hat_peak - hat_before).abs() < 0.05, "already-unmuted hat is not double-fired: {hat_peak} vs {hat_before}");

    // 2) Switching from an empty pattern slot (slot 7) back to pattern 0 (slot 0) 25 ms after beat 0 catches beat 0.
    let mut drop_proj = base.clone();
    drop_proj.groups[0].patterns.push(Pattern { slot: 7, length: 4 * PPQ, notes: vec![], ..Default::default() });
    drop_proj.scenes[0].patterns = vec![(0, 1)];
    let mut e = loop_engine(&drop_proj, &mut cache);
    e.process(&mut vec![0f32; 2 * 25]);
    adopt_at_1k(&mut e, &drop_proj, &base, &mut cache);
    let mut after_switch = vec![0f32; 2 * 60];
    e.process(&mut after_switch);
    assert!(after_switch.chunks_exact(2).any(|f| f[0] > 0.2), "on-beat pattern return catches beat 0 kick");

    // 3) Adding a step/note to an already-playing pattern (same slot, same mute) 25 ms behind playhead does NOT fire it.
    let mut no_kick_note = base.clone();
    no_kick_note.groups[0].patterns[0].notes.retain(|n| n.sound != 0);
    let mut e = loop_engine(&no_kick_note, &mut cache);
    e.process(&mut vec![0f32; 2 * 25]);
    adopt_at_1k(&mut e, &no_kick_note, &base, &mut cache);
    let mut after_edit = vec![0f32; 2 * 60];
    e.process(&mut after_edit);
    assert!(after_edit.chunks_exact(2).all(|f| f[0].abs() < 1e-6), "note edit behind playhead waits for next cycle");
}

#[test]
fn beat_grace_fades_just_fired_kick_on_mute_or_drop_but_keeps_mid_beat_tail() {
    let dir = tempfile::tempdir().unwrap();
    let kick_wav = dir.path().join("kick48.wav");
    let hat_wav = dir.path().join("hat48.wav");
    edit::write_wav(&kick_wav, &sample::Sample { rate: 48_000, frames: vec![[0.7, 0.0]; 12_000] }).unwrap();
    edit::write_wav(&hat_wav, &sample::Sample { rate: 48_000, frames: vec![[0.0, 0.4]; 12_000] }).unwrap();

    let mut playing = project();
    let hat_sound = playing.groups[0].sounds[0].clone();
    playing.groups[0].sounds.push(hat_sound);
    for (s, wav) in [(0, &kick_wav), (1, &hat_wav)] {
        let smp = playing.groups[0].sounds[s].sampler.as_mut().unwrap();
        smp.zones[0].sample = deckard_model::SampleRef { kind: 0, path: wav.to_string_lossy().into(), ..Default::default() };
        smp.zones[0].play_range = (0, 11_999);
        smp.set("envelope_type", 0.0); // one-shot
    }
    playing.groups[0].patterns[0].notes = vec![
        Note { ratchet: Default::default(), expr: None, sound: 0, position: 0, length: 240, pitch: 60, velocity: 127, probability: 100 },
        Note { ratchet: Default::default(), expr: None, sound: 1, position: 0, length: 240, pitch: 60, velocity: 127, probability: 100 },
    ];
    let mut muted = playing.clone();
    muted.groups[0].sounds[0].muted = true;

    let mut cache = SampleCache::default();
    // 1) One-shot Sampler kick muted 20 ms (960 frames @ 48 kHz) into beat 0 fades out within 15 ms; hat keeps playing.
    let (mut e, _) = Engine::new(&playing, None, 48_000, Mode::Scene(0), &mut cache);
    e.jump(&playing, Mode::Scene(0));
    e.looping = true;
    let mut start = vec![0f32; 2 * 960];
    e.process(&mut start);
    assert!(start[2 * 500] > 0.2, "kick started on beat 0");
    let (fresh, _) = Engine::new_after(&muted, Some((&playing, &[])), None, 48_000, Mode::Scene(0), &mut cache);
    drop(e.adopt(fresh, &muted));
    let mut fade = vec![0f32; 2 * 1440]; // 30 ms
    e.process(&mut fade);
    let tail_kick = fade[2 * 720..].chunks_exact(2).map(|f| f[0].abs()).fold(0f32, f32::max);
    let tail_hat = fade[2 * 720..].chunks_exact(2).map(|f| f[1].abs()).fold(0f32, f32::max);
    assert!(tail_kick < 1e-4, "one-shot kick faded within 15 ms of on-beat mute: {tail_kick}");
    assert!(tail_hat > 0.1, "unmuted hat unaffected by kick mute: {tail_hat}");

    // 2) Muting mid-beat (100 ms = 4800 frames after attack) lets the one-shot tail ring out as Event Mute specifies.
    let (mut e, _) = Engine::new(&playing, None, 48_000, Mode::Scene(0), &mut cache);
    e.jump(&playing, Mode::Scene(0));
    e.looping = true;
    e.process(&mut vec![0f32; 2 * 4800]);
    let (fresh, _) = Engine::new_after(&muted, Some((&playing, &[])), None, 48_000, Mode::Scene(0), &mut cache);
    drop(e.adopt(fresh, &muted));
    let mut mid = vec![0f32; 2 * 960];
    e.process(&mut mid);
    let mid_kick = mid[2 * 720..].chunks_exact(2).map(|f| f[0].abs()).fold(0f32, f32::max);
    assert!(mid_kick > 0.2, "mid-beat mute leaves existing one-shot tail ringing: {mid_kick}");

    // 3) Drum Synth Kick muted 20 ms into beat 0 also chokes cleanly instead of booming through the beat.
    let mut ds_play = playing.clone();
    ds_play.groups[0].sounds[0].sampler = None;
    ds_play.groups[0].sounds[0].slots = vec![deckard_model::SlotInfo {
        module: "Kick".into(),
        name: "Kick".into(),
        active: true,
        plugin: None,
        params: vec![("decay".into(), 0.9)],
    }];
    let mut ds_mute = ds_play.clone();
    ds_mute.groups[0].sounds[0].muted = true;
    let (mut e, _) = Engine::new(&ds_play, None, 48_000, Mode::Scene(0), &mut cache);
    e.jump(&ds_play, Mode::Scene(0));
    e.looping = true;
    let mut ds_start = vec![0f32; 2 * 960];
    e.process(&mut ds_start);
    assert!(ds_start.chunks_exact(2).any(|f| f[0].abs() > 0.05), "drum synth kick sounding on beat 0");
    let (mut e_ref, _) = Engine::new(&ds_play, None, 48_000, Mode::Scene(0), &mut cache);
    e_ref.jump(&ds_play, Mode::Scene(0));
    e_ref.looping = true;
    e_ref.process(&mut vec![0f32; 2 * 960]);
    let mut ds_unchoked = vec![0f32; 2 * 1920];
    e_ref.process(&mut ds_unchoked);
    let unchoked_tail = ds_unchoked[2 * 1680..].chunks_exact(2).map(|f| f[0].abs()).fold(0f32, f32::max);

    let (fresh, _) = Engine::new_after(&ds_mute, Some((&ds_play, &[])), None, 48_000, Mode::Scene(0), &mut cache);
    drop(e.adopt(fresh, &ds_mute));
    let mut ds_fade = vec![0f32; 2 * 1920];
    e.process(&mut ds_fade);
    let ds_tail = ds_fade[2 * 1680..].chunks_exact(2).map(|f| f[0].abs()).fold(0f32, f32::max);
    assert!(ds_tail < 0.005 && unchoked_tail > 0.1, "drum synth kick choked vs unchoked tail: choked={ds_tail}, unchoked={unchoked_tail}");
}

#[test]
fn launched_scene_starts_when_the_playing_one_ends() {
    // Scene 0: quarters for a bar; scene 1: half notes for a bar.
    let mut p = project();
    let halves = (0..2)
        .map(|i| Note {
            ratchet: Default::default(),
            expr: None,
            sound: 0,
            position: i * 2 * PPQ,
            length: 240,
            pitch: 60,
            velocity: 127,
            probability: 100,
        })
        .collect();
    p.groups[0].patterns.push(Pattern { length: 4 * PPQ, notes: halves, ..Default::default() });
    p.scenes.push(Scene { patterns: vec![(0, 1)], ..Default::default() });
    // 120 BPM at 1 kHz: a quarter is 500 frames, a bar 2000.
    let mut e = click_engine_for(&p, 1000);
    e.jump(&p, Mode::Scene(0));
    e.looping = true;
    let mut out = vec![0f32; 2 * 6000];
    e.process(&mut out[..2 * 700]);
    e.queue(&p, Mode::Scene(1));
    assert_eq!((e.mode(), e.queued()), (Mode::Scene(0), Some(Mode::Scene(1))));
    e.process(&mut out[2 * 700..]);
    let onsets: Vec<usize> = (0..6000).filter(|&i| out[i * 2] > 0.5).collect();
    assert_eq!(onsets, [0, 500, 1000, 1500, 2000, 3000, 4000, 5000]);
    assert_eq!((e.mode(), e.queued()), (Mode::Scene(1), None));
}

/// `project()` plus group 1 with a silent pattern `len` ticks long, in scene 0.
fn with_odd_pattern(len: i64) -> Project {
    let mut p = project();
    p.groups.push(Group { patterns: vec![Pattern { length: len, ..Default::default() }], ..Default::default() });
    p.scenes[0].patterns.push((1, 0));
    p
}

/// A 1-bar and a 1:0:2 pattern each loop by themselves, as in Maschine's Ideas view:
/// the scene repeats when they line up again (9 bars), not when the longer one ends.
#[test]
fn a_scenes_patterns_loop_by_themselves() {
    let odd = 4 * PPQ + 2 * PPQ / 4;
    let mut p = with_odd_pattern(odd);
    p.groups[1].patterns[0].notes = vec![Note {
        expr: None,
        sound: 0,
        position: 0,
        length: 240,
        pitch: 60,
        velocity: 127,
        probability: 100,
        ratchet: Default::default(),
    }];
    let t = timeline(&p, Mode::Scene(0));
    assert_eq!((t.length, t.switch_every), (9 * 4 * PPQ, odd));
    let at = |g: usize| t.events.iter().filter(|e| e.group == g).map(|e| e.tick).collect::<Vec<_>>();
    assert_eq!(at(0), (0..36).map(|i| i * PPQ).collect::<Vec<_>>(), "the bar's quarters, every bar");
    assert_eq!(at(1), (0..8).map(|i| i * odd).collect::<Vec<_>>());
    // Lengths that would line up only after more than MAX_CYCLE_BARS: the longest sets it.
    let p = with_odd_pattern(4 * PPQ + 1);
    let t = timeline(&p, Mode::Scene(0));
    assert_eq!((t.length, t.switch_every), (4 * PPQ + 1, 4 * PPQ + 1));
    // Song sections still start every pattern from the top.
    let mut p = with_odd_pattern(odd);
    p.sections = vec![Section { scene: Some(0), length: odd, auto: true }];
    assert_eq!(timeline(&p, Mode::Arrangement).length, odd);
}

/// Playing, the 1-bar pattern goes on past the 1:0:2 one's end: no restart there.
#[test]
fn a_shorter_pattern_doesnt_start_over_with_the_longest() {
    let p = with_odd_pattern(4 * PPQ + 2 * PPQ / 4);
    // 120 BPM at 1 kHz: a quarter is 500 frames, the 1:0:2 pattern 2250.
    let mut e = click_engine_for(&p, 1000);
    e.jump(&p, Mode::Scene(0));
    e.looping = true;
    let mut out = vec![0f32; 2 * 5000];
    e.process(&mut out);
    let onsets: Vec<usize> = (0..5000).filter(|&i| out[i * 2] > 0.5).collect();
    assert_eq!(onsets, (0..10).map(|i| i * 500).collect::<Vec<_>>());
}

/// A launched scene still starts where the playing scene's longest pattern ends, not
/// at the end of the longer cycle its patterns make.
#[test]
fn a_launched_scene_starts_when_the_longest_pattern_ends() {
    let mut p = with_odd_pattern(4 * PPQ + 2 * PPQ / 4);
    let halves = (0..2)
        .map(|i| Note {
            expr: None,
            sound: 0,
            position: i * 2 * PPQ,
            length: 240,
            pitch: 60,
            velocity: 127,
            probability: 100,
            ratchet: Default::default(),
        })
        .collect();
    p.groups[0].patterns.push(Pattern { length: 4 * PPQ, notes: halves, ..Default::default() });
    p.scenes.push(Scene { patterns: vec![(0, 1)], ..Default::default() });
    let mut e = click_engine_for(&p, 1000);
    e.jump(&p, Mode::Scene(0));
    e.looping = true;
    let mut out = vec![0f32; 2 * 6000];
    e.process(&mut out[..2 * 700]);
    e.queue(&p, Mode::Scene(1));
    e.process(&mut out[2 * 700..]);
    let onsets: Vec<usize> = (0..6000).filter(|&i| out[i * 2] > 0.5).collect();
    assert_eq!(onsets, [0, 500, 1000, 1500, 2000, 2250, 3250, 4250, 5250], "scene 1 from 2250");
    assert_eq!((e.mode(), e.queued()), (Mode::Scene(1), None));
    // Queued in the second pass of the 1:0:2 pattern: at its end, 4500.
    let mut e = click_engine_for(&p, 1000);
    e.jump(&p, Mode::Scene(0));
    e.looping = true;
    let mut out = vec![0f32; 2 * 6000];
    e.process(&mut out[..2 * 3000]);
    e.queue(&p, Mode::Scene(1));
    e.process(&mut out[2 * 3000..]);
    let onsets: Vec<usize> = (0..6000).filter(|&i| out[i * 2] > 0.5).collect();
    assert_eq!(onsets, [0, 500, 1000, 1500, 2000, 2500, 3000, 3500, 4000, 4500, 5500], "scene 1 from 4500");
}

#[test]
fn update_keeps_position_and_follows_tempo() {
    // 120 BPM at 1 kHz: a quarter is 500 frames.
    let mut p = project();
    let mut e = click_engine_for(&p, 1000);
    let mut out = vec![0f32; 2 * 1250];
    e.process(&mut out);
    assert_eq!(e.position(), 1250.0 * 960.0 / 500.0, "2.5 beats in");
    // Twice the tempo: the next beats come every 250 frames from the same position.
    p.tempo = 240.0;
    let mut cache = SampleCache::default();
    let samples = e.samples.clone();
    let sound = e.sounds[0][0].take();
    e.update(&p, None, &mut cache);
    e.samples = samples; // the test sample (Engine::new found no files)
    e.sounds[0][0] = sound;
    assert_eq!(e.position(), 2.5 * 960.0);
    let mut out = vec![0f32; 2 * 1000];
    e.process(&mut out);
    let onsets: Vec<usize> = (0..1000).filter(|&i| out[i * 2] > 0.5).collect();
    assert_eq!(onsets, [125, 375, 625, 875], "beats 3..6 at 240 BPM");
}

/// #14: a tempo edit while playing, as the app sends it (built after the playing
/// project, then adopted), plays at the new tempo from where it is.
#[test]
fn an_adopted_tempo_edit_follows_the_tempo() {
    let p = project();
    let mut e = click_engine_for(&p, 1000);
    let mut out = vec![0f32; 2 * 1250];
    e.process(&mut out);
    let mut q = p.clone();
    q.tempo = 240.0;
    let mut cache = SampleCache::default();
    let chains: Vec<Vec<bool>> = p.groups.iter().map(|g| vec![false; g.sounds.len()]).collect();
    let (fresh, _) = Engine::new_after(&q, Some((&p, &chains)), None, 1000, e.mode(), &mut cache);
    let mut timelines = if fresh.keeps_timelines() { Timelines::default() } else { Timelines::of(&q, &[e.mode()]) };
    let samples = e.samples.clone();
    let sound = e.sounds[0][0].take();
    let _old = e.adopt_with(fresh, &mut timelines);
    e.samples = samples;
    e.sounds[0][0] = sound;
    assert_eq!(e.tempo(), 240.0);
    assert_eq!(e.position(), 2.5 * 960.0);
    let mut out = vec![0f32; 2 * 1000];
    e.process(&mut out);
    let onsets: Vec<usize> = (0..1000).filter(|&i| out[i * 2] > 0.5).collect();
    assert_eq!(onsets, [125, 375, 625, 875], "beats 3..6 at 240 BPM");
}

#[test]
fn midi_events_join_the_timeline_before_notes() {
    let mut p = project();
    let pedal = |position, value| deckard_model::MidiEvent { sound: 0, position, kind: MidiKind::Controller { number: 64, value } };
    p.groups[0].patterns[0].midi = vec![pedal(0, 127), pedal(2 * PPQ + 10, 0)];
    let t = timeline(&p, Mode::Pattern(0, 0));
    let first: Vec<Option<MidiKind>> = t.events.iter().take(2).map(|e| e.midi).collect();
    assert_eq!(
        first,
        [Some(MidiKind::Controller { number: 64, value: 127 }), None],
        "the pedal goes down before the note at the same tick"
    );
    assert!(t.events.iter().any(|e| e.tick == 2 * PPQ + 10 && e.midi == Some(MidiKind::Controller { number: 64, value: 0 })));
    assert_eq!(t.events.iter().filter(|e| e.midi.is_none()).count(), 4);
}

#[test]
fn held_notes_sound_until_released() {
    let mut e = click_engine(1000);
    e.sequencing = false;
    // A looped, sustaining sound (ADSR) with a short release.
    set_sample(&mut e, vec![[0.5, 0.5]; 100], &[("envelope_type", 2.0), ("volume_envelope/release", 0.0)], true);
    e.note_hold(0, 0, 60, 127);
    let mut out = vec![0f32; 2 * 3000];
    e.process(&mut out);
    assert!(out[2 * 2999] > 0.4, "still sounding after 3 s (six beats)");
    e.note_release(0, 0, 60);
    let mut out = vec![0f32; 2 * 100];
    e.process(&mut out);
    assert!(out[2 * 99].abs() < 1e-6, "silent after the release");
    assert!(e.finished());
}
