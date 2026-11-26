use super::*;

/// A 120 BPM song whose sound 0 is an Audio plug-in playing a 4-beat 94 BPM loop
/// (`engine`), on in its 8-beat pattern. With the folder holding the loop.
fn audio_project(engine: deckard_model::AudioEngine) -> (tempfile::TempDir, Project) {
    let tmp = tempfile::tempdir().unwrap();
    let mut p = project();
    let take = deckard_model::Take {
        sample: deckard_model::SampleRef { kind: 0, path: loop_wav(tmp.path(), 94.0, 4.0).to_string_lossy().into(), ..Default::default() },
        beats: 4.0,
        play_range: (0, 0),
    };
    let snd = &mut p.groups[0].sounds[0];
    snd.instrument = "AudioModule".into();
    snd.sampler = None;
    snd.audio = Some(deckard_model::AudioLoop { takes: vec![take], engine, pitchbend: 0.5, fade: 0.1222, ..Default::default() });
    p.groups[0].patterns[0] =
        Pattern { length: 8 * PPQ, audio: vec![deckard_model::AudioCue { sound: 0, take: 0, offset: 0 }], ..Default::default() };
    p.sections = vec![Section { scene: Some(0), length: 16 * PPQ, auto: false }];
    (tmp, p)
}

/// Tuned while it plays (an edit), the loop carries on where it was, transposed.
#[test]
fn an_edited_loop_plays_on_with_its_new_settings() {
    let (_tmp, p) = audio_project(deckard_model::AudioEngine::Stretch);
    let mut tuned = p.clone();
    tuned.groups[0].sounds[0].audio.as_mut().unwrap().tune = 12.0;
    let hz = |x: &[f32]| {
        let left: Vec<f32> = x.chunks_exact(2).map(|f| f[0]).collect();
        let n = left.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        n as f64 * 48_000.0 / left.len() as f64
    };
    let chains = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
    let out = render_with(&p, None, 256, 2.0, |k, e, cache| {
        // Half a second in (between the marks): the tune changes, as an edit.
        if k == 94 {
            let (fresh, _) = Engine::new_after(&tuned, Some((&p, chains.as_slice())), None, 48_000, e.mode(), cache);
            drop(e.adopt(fresh, &tuned));
        }
    });
    // Away from the first beat's mark: before the edit 440 Hz, after it 880 Hz.
    let before = &out[2 * 6_000..2 * 23_000];
    let after = &out[2 * 40_000..2 * 90_000];
    assert!((hz(before) - 440.0).abs() < 20.0, "before: {}", hz(before));
    assert!((hz(after) - 880.0).abs() < 30.0, "after: {}", hz(after));
}

/// A take switched on while the song plays (an edit) is heard at once, where the song
/// is, not at the pattern's next pass.
#[test]
fn a_loop_switched_on_while_playing_is_heard_at_once() {
    let (_tmp, on) = audio_project(deckard_model::AudioEngine::Stretch);
    let mut off = on.clone();
    off.groups[0].patterns[0].audio.clear();
    let chains = Engine::new(&off, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
    let out = render_with(&off, None, 256, 3.0, |k, e, cache| {
        // Two beats in (48 000 frames): the take goes on, as an edit.
        if k == 48_000 / 256 {
            let (fresh, _) = Engine::new_after(&on, Some((&off, chains.as_slice())), None, 48_000, e.mode(), cache);
            drop(e.adopt(fresh, &on));
        }
    });
    let before: f32 = out[..2 * 47_000].iter().map(|v| v * v).sum();
    let after: f32 = out[2 * 48_500..2 * 72_000].iter().map(|v| v * v).sum();
    assert!(before < 1e-6, "silent before: {before}");
    assert!(after > 1.0, "heard at once: {after}");
}

/// A take switched off while playing (the pad's toggle) fades out without a jump,
/// whether rendered live or ahead, across all engines.
#[test]
fn a_loop_switched_off_while_playing_fades_without_clicking() {
    for engine in [deckard_model::AudioEngine::Stretch, deckard_model::AudioEngine::Repitch, deckard_model::AudioEngine::Formant] {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sine.wav");
        let spec = hound::WavSpec { channels: 2, sample_rate: 48_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let frames = 96_000usize; // 4 beats at 120 BPM
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..frames {
            let v = (i as f32 / 48_000.0 * 220.0 * std::f32::consts::TAU).sin() * 0.8;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        let mut on = project();
        let take = deckard_model::Take {
            sample: deckard_model::SampleRef { kind: 0, path: path.to_string_lossy().into(), ..Default::default() },
            beats: 4.0,
            play_range: (0, 0),
        };
        let snd = &mut on.groups[0].sounds[0];
        snd.instrument = "AudioModule".into();
        snd.sampler = None;
        snd.audio = Some(deckard_model::AudioLoop { takes: vec![take], engine, pitchbend: 0.5, fade: 0.1222, ..Default::default() });
        on.groups[0].patterns[0] =
            Pattern { length: 8 * PPQ, audio: vec![deckard_model::AudioCue { sound: 0, take: 0, offset: PPQ }], ..Default::default() };
        on.sections = vec![Section { scene: Some(0), length: 16 * PPQ, auto: false }];
        let mut off = on.clone();
        off.groups[0].patterns[0].audio.clear();
        let chains = Engine::new(&on, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
        for ahead in [None, Some(700)] {
            let out = render_with(&on, ahead, 256, 2.0, |k, e, cache| {
                if k == 48_000 / 256 {
                    let (fresh, _) = Engine::new_after(&off, Some((&on, chains.as_slice())), None, 48_000, e.mode(), cache);
                    drop(e.adopt(fresh, &off));
                }
            });
            let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
            let max_step = left[47_000..52_000].windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
            assert!(max_step < 0.06, "{engine:?} ahead={ahead:?}: click at loop off (max step {max_step})");
            let tail: f32 = left[52_000..].iter().map(|v| v * v).sum();
            assert!(tail < 1e-6, "{engine:?} ahead={ahead:?}: not silent after fade-out: {tail}");
        }
    }
}

/// Edited while the transport is stopped, a loop stays silent until the pattern plays.
#[test]
fn an_edit_while_stopped_does_not_start_a_loop() {
    let (_tmp, p) = audio_project(deckard_model::AudioEngine::Stretch);
    let out = render_with(&p, None, 256, 2.0, |k, e, cache| {
        if k == 48_000 / 256 {
            e.stop();
        }
        if k == 60_000 / 256 {
            let (fresh, _) = Engine::new_after(&p, Some((&p, &[])), None, 48_000, e.mode(), cache);
            drop(e.adopt(fresh, &p));
        }
    });
    let after: f32 = out[2 * 66_000..].iter().map(|v| v * v).sum();
    assert!(after < 1e-6, "silent after the edit: {after}");
}

/// A sound that becomes an Audio plug-in in the edit that cues it is heard at once.
#[test]
fn a_sound_made_audio_by_the_edit_is_heard_at_once() {
    let (_tmp, on) = audio_project(deckard_model::AudioEngine::Stretch);
    let mut off = on.clone();
    off.groups[0].sounds[0].audio = None;
    off.groups[0].patterns[0].audio.clear();
    let chains = Engine::new(&off, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
    let out = render_with(&off, None, 256, 3.0, |k, e, cache| {
        if k == 48_000 / 256 {
            let (fresh, _) = Engine::new_after(&on, Some((&off, chains.as_slice())), None, 48_000, e.mode(), cache);
            drop(e.adopt(fresh, &on));
        }
    });
    let after: f32 = out[2 * 48_500..2 * 72_000].iter().map(|v| v * v).sum();
    assert!(after > 1.0, "heard at once: {after}");
}

/// Started in the middle of its pattern, the loop plays at once, where the song is
/// (its mark comes at the pattern's next four-beat boundary, not later).
#[test]
fn a_loop_started_mid_pattern_plays_from_there() {
    let (_tmp, p) = audio_project(deckard_model::AudioEngine::Stretch);
    let out = render_with(&p, None, 256, 3.0, |k, e, _| {
        if k == 0 {
            // Beat 2 of 8: the mark (on beat 4) two beats (48 000 frames) later.
            e.seek(2 * PPQ);
        }
    });
    let level: f32 = out[..48_000].iter().map(|v| v * v).sum();
    assert!(level > 1.0, "it plays from the start: {level}");
    let m = marks(&out);
    assert!(m.first().is_some_and(|&f| f.abs_diff(48_000) < 1024), "the next mark on beat 4: {m:?}");
}

#[test]
fn audio_loop_deactivates_on_scene_edits_and_jumps_and_plays_contiguous_gate_steps() {
    // 1. Deactivating the group's pattern in Scene 0 (or jumping to an empty scene)
    // stops an active AudioMode::Loop, and `beats == 0.0` snaps to a power-of-two beat count.
    let (_tmp, mut on) = audio_project(deckard_model::AudioEngine::Repitch);
    on.groups[0].sounds[0].audio.as_mut().unwrap().takes[0].beats = 0.0;
    on.scenes.push(deckard_model::Scene { name: "Scene 2".into(), patterns: Vec::new(), ..Default::default() });
    let mut deactivated = on.clone();
    deactivated.scenes[0].patterns.clear();
    let chains = Engine::new(&on, None, 48_000, Mode::Scene(0), &mut SampleCache::default()).1.chains;
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&on, None, 48_000, Mode::Scene(0), &mut cache);
    e.looping = true;
    let mut buf = vec![0.0f32; 512];
    for _ in 0..40 {
        e.process(&mut buf);
    }
    assert!(buf.iter().any(|v| v.abs() > 0.01), "loop playing before pattern deactivation");
    assert_eq!(e.sounds[0][0].as_ref().unwrap().voices(), 1);
    let (fresh, _) = Engine::new_after(&deactivated, Some((&on, chains.as_slice())), None, 48_000, Mode::Scene(0), &mut cache);
    drop(e.adopt(fresh, &deactivated));
    for _ in 0..10 {
        e.process(&mut buf);
    }
    assert!(buf.iter().all(|v| v.abs() < 1e-6), "loop silent after pattern deactivated");

    // Re-jump back to Scene 0 with the pattern active, then jump to empty Scene 1: stops.
    e.jump(&on, Mode::Scene(0));
    for _ in 0..20 {
        e.process(&mut buf);
    }
    assert!(buf.iter().any(|v| v.abs() > 0.01), "loop playing after jump to Scene 0");
    e.jump(&on, Mode::Scene(1));
    for _ in 0..10 {
        e.process(&mut buf);
    }
    assert!(buf.iter().all(|v| v.abs() < 1e-6), "loop silent after jump to empty Scene 1");

    // Queued scene switch also recues cleanly.
    e.jump(&on, Mode::Scene(0));
    e.queue(&on, Mode::Scene(1));
    for _ in 0..800 {
        e.process(&mut buf);
    }
    assert!(buf.iter().all(|v| v.abs() < 1e-6), "loop silent after queued switch to empty Scene 1");

    // 2. Switching to Gate mode and filling all 16th steps plays across every step
    // (including when rendered ahead on an unfocused group) and releases live keys.
    let mut gated = on.clone();
    gated.groups[0].sounds[0].audio.as_mut().unwrap().mode = deckard_model::AudioMode::Gate;
    gated.groups[0].patterns[0].audio.clear();
    gated.groups[0].patterns[0].length = 4 * PPQ;
    gated.groups[0].patterns[0].notes = (0..16)
        .map(|step| deckard_model::Note {
            ratchet: Default::default(),
            expr: None,
            sound: 0,
            position: step * (PPQ / 4),
            length: PPQ / 4,
            pitch: 60,
            velocity: 100,
            probability: 100,
        })
        .collect();
    let out_ahead = render_with(&gated, Some(1), 256, 2.0, |_, _, _| {});
    for beat in 0..4 {
        let slice = &out_ahead[beat * 24_000 * 2 + 2_000..(beat + 1) * 24_000 * 2 - 2_000];
        let energy: f32 = slice.iter().map(|v| v * v).sum();
        assert!(energy > 1.0, "beat {beat} silent in Gate mode with 16th steps: {energy}");
    }
    let (mut ge, _) = Engine::new(&gated, None, 48_000, Mode::Scene(0), &mut cache);
    ge.sequencing = false;
    ge.note_on(0, 0, 60, 100, HELD);
    assert_eq!(ge.sounds[0][0].as_ref().unwrap().voices(), 1);
    ge.note_release(0, 0, 60);
    for _ in 0..10 {
        ge.process(&mut buf);
    }
    assert_eq!(ge.sounds[0][0].as_ref().unwrap().voices(), 0);
}

/// The loop plays from the song's start, once every four beats at the song's 120
/// BPM (not its own 94), with every engine; rendered ahead it sounds the same.
#[test]
fn audio_plugin_loops_play_in_time_with_the_song() {
    for engine in [deckard_model::AudioEngine::Stretch, deckard_model::AudioEngine::Repitch, deckard_model::AudioEngine::Formant] {
        let (_tmp, p) = audio_project(engine);
        let out = render_with(&p, None, 256, 7.0, |_, _, _| {});
        let m = marks(&out);
        assert!(m.first().is_some_and(|&f| f < 1024), "{engine:?}: starts at the song's start: {m:?}");
        let gaps: Vec<usize> = m.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.len() >= 2 && gaps.iter().all(|&g| g.abs_diff(96_000) < 600), "{engine:?}: every 4 beats at 120: {gaps:?}");
        let ahead = render_with(&p, Some(700), 256, 7.0, |_, _, _| {});
        assert_eq!(marks(&ahead), m, "{engine:?}: rendered ahead, the same");
    }
}

/// Marks of the loop in `audio_project` with the pattern's `start` and the cue's `offset`.
/// Rendered live; the render-ahead path must give the same marks.
fn loop_marks(start: i64, offset: i64, seconds: f64) -> Vec<usize> {
    let (_tmp, mut p) = audio_project(deckard_model::AudioEngine::Stretch);
    p.groups[0].patterns[0].start = start;
    p.groups[0].patterns[0].audio[0].offset = offset;
    let live = marks(&render_with(&p, None, 256, seconds, |_, _, _| {}));
    let ahead = marks(&render_with(&p, Some(700), 256, seconds, |_, _, _| {}));
    assert_eq!(ahead, live, "start {start}, offset {offset}: rendered ahead, the same");
    live
}

/// Maschine's rule: the loop is where the pattern's Start puts it (Start 2 beats: its
/// first frame sounds 2 beats into the pass).
#[test]
fn a_loop_follows_the_patterns_start() {
    let m = loop_marks(2 * PPQ, 0, 4.0);
    assert!(m.first().is_some_and(|&f| f.abs_diff(48_000) < 512), "{m:?}");
}

/// The offset is where the take's first frame sounds: one beat in, then every take.
#[test]
fn a_loop_starts_at_its_offset() {
    let m = loop_marks(0, PPQ, 4.0);
    assert!(m.first().is_some_and(|&f| f.abs_diff(24_000) < 512), "{m:?}");
    assert!(m.get(1).is_some_and(|&f| f.abs_diff(24_000 + 96_000) < 512), "{m:?}");
}

/// An offset past the take's length wraps: 5 beats on a 4-beat take is 1.
#[test]
fn an_offset_past_the_take_wraps() {
    assert_eq!(loop_marks(0, 5 * PPQ, 4.0), loop_marks(0, PPQ, 4.0));
}

/// Sounds moving between live and ahead while they play (the focus switching to
/// their group and away, every few blocks) sound exactly as if they never moved.
#[test]
fn moving_between_live_and_ahead_is_seamless() {
    let (_tmp, p) = eight_project();
    let inline = render_with(&p, None, 256, 3.0, |_, _, _| {});
    let mut moves = 0;
    let ahead = render_with(&p, Some(700), 256, 3.0, |k, e, _| {
        if k % 7 == 3 {
            let to = if e.sounds_ahead() > 0 { Some(0) } else { None };
            e.set_focus(to);
            moves += 1;
        }
    });
    assert!(moves > 20);
    let first = inline.iter().zip(&ahead).position(|(a, b)| a != b);
    assert_eq!(first, None, "bit for bit (first difference at frame {:?}, block {:?})", first.map(|i| i / 2), first.map(|i| i / 2 / 256));
}

/// Edits while sounds are rendered ahead: a level change is applied live (exactly
/// as without rendering ahead); an effect's change reaches the player (a lookahead
/// later) and then sounds the same.
#[test]
fn edits_reach_sounds_rendered_ahead() {
    let (_tmp, p) = eight_project();
    let mut louder = p.clone();
    louder.groups[0].sounds[3].level = 0.3;
    let edit = |to: &Project| {
        let to = to.clone();
        move |k: usize, e: &mut Engine, cache: &mut SampleCache| {
            if k == 100 {
                e.update(&to, None, cache);
            }
        }
    };
    let inline = render_with(&p, None, 256, 2.0, edit(&louder));
    let ahead = render_with(&p, Some(700), 256, 2.0, edit(&louder));
    assert_eq!(inline.iter().zip(&ahead).fold(0f32, |m, (a, b)| m.max((a - b).abs())), 0.0, "level: bit for bit");
    let mut driven = p.clone();
    let sat = driven.groups[0].sounds[5].slots.iter_mut().find(|x| x.module == "Saturator").unwrap();
    for (k, v) in sat.params.iter_mut() {
        if k.contains("drive") {
            *v = 1.0;
        }
    }
    let inline = render_with(&p, None, 256, 2.0, edit(&driven));
    let ahead = render_with(&p, Some(700), 256, 2.0, edit(&driven));
    let plain = render_with(&p, None, 256, 2.0, |_, _, _| {});
    let at = 100 * 256 * 2;
    let diff = |a: &[f32], b: &[f32]| a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs()));
    assert_eq!(diff(&inline[..at], &ahead[..at]), 0.0, "the same before the edit");
    assert!(diff(&inline[at..], &plain[at..]) > 0.01, "the edit changes the sound");
    // A lookahead (700 frames) and the effect's glide later, it is the same.
    let settled = at + 2 * 48_000 / 4;
    assert!(diff(&inline[settled..], &ahead[settled..]) < 1e-4, "settled: {}", diff(&inline[settled..], &ahead[settled..]));
}

/// A group with effects is a bus: its sounds' routes end at it, with only their own
/// faders; the group's fader comes after its effects.
#[test]
fn a_group_with_effects_is_a_bus() {
    let (_tmp, mut p) = eight_project();
    p.groups[0].level = 0.5;
    let plain = sound_route(&p, 0, 0);
    assert_eq!(plain.0, None);
    p.groups[0].slots = vec![driven_saturator()];
    let (to, gain) = sound_route(&p, 0, 0);
    assert_eq!(to, Some(GROUP_BUS));
    let s = &p.groups[0].sounds[0];
    assert_eq!(gain, channel_gain(s.level, s.pan));
    assert_eq!(group_out_route(&p, 0), (None, channel_gain(0.5, p.groups[0].pan)));
    p.groups[0].muted = true;
    assert_eq!(sound_route(&p, 0, 0).1, (0.0, 0.0));
}

/// A group's effect changes what the group plays (and nothing when it isn't there).
#[test]
fn group_effects_play() {
    let (_tmp, p) = eight_project();
    let mut fx = p.clone();
    fx.groups[0].slots = vec![driven_saturator()];
    let plain = render_with(&p, None, 256, 1.0, |_, _, _| {});
    let driven = render_with(&fx, None, 256, 1.0, |_, _, _| {});
    assert!(driven.iter().all(|x| x.is_finite()));
    assert!(max_diff(&plain, &driven) > 0.05, "the group's saturator is heard");
    // Through the bus with its effect bypassed: as without a bus (the group's fader
    // and pan where they were).
    let mut bypassed = fx.clone();
    bypassed.groups[0].slots[0].active = false;
    bypassed.groups[0].level = 0.6;
    bypassed.groups[0].pan = 0.3;
    let mut folded = p.clone();
    folded.groups[0].level = 0.6;
    folded.groups[0].pan = 0.3;
    let (a, b) = (render_with(&bypassed, None, 256, 1.0, |_, _, _| {}), render_with(&folded, None, 256, 1.0, |_, _, _| {}));
    assert!(max_diff(&a, &b) < 1e-4, "bus = folded: {}", max_diff(&a, &b));
    // Its fader comes after its effects.
    let mut down = fx.clone();
    down.groups[0].level = 0.0;
    assert!(render_with(&down, None, 256, 0.5, |_, _, _| {}).iter().all(|x| x.abs() < 1e-6));
    let mut muted = fx.clone();
    muted.groups[0].muted = true;
    assert!(render_with(&muted, None, 256, 0.5, |_, _, _| {}).iter().all(|x| *x == 0.0));
}

/// Sounds rendered ahead go through their group's bus as the others do.
#[test]
fn rendered_ahead_sounds_go_through_the_group_bus() {
    let (_tmp, mut p) = eight_project();
    p.groups[0].slots = vec![driven_saturator()];
    let inline = render_with(&p, None, 256, 1.0, |_, _, _| {});
    let ahead = render_with(&p, Some(700), 256, 1.0, |_, _, _| {});
    assert!(max_diff(&inline, &ahead) < 1e-5, "{}", max_diff(&inline, &ahead));
}

/// An edit elsewhere (a sound's level) leaves a group's and the master's effects as
/// they were: a delay's echoes carry on through it.
#[test]
fn edits_keep_group_and_master_effects() {
    let (_, d) = deckard_model::INTERNAL_EFFECTS.iter().find(|(c, _)| *c == "BeatDelay").unwrap();
    let delay = deckard_model::SlotInfo {
        module: "BeatDelay".into(),
        name: "BeatDelay".into(),
        active: true,
        plugin: None,
        params: d.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
    };
    let (_tmp, mut p) = eight_project();
    p.groups[0].slots = vec![delay.clone()];
    p.master_slots = vec![delay];
    let same = p.clone();
    let plain = render_with(&p, None, 256, 1.5, |_, _, _| {});
    let edited = render_with(&p, None, 256, 1.5, move |k, e, cache| {
        if k == 100 {
            e.update(&same, None, cache);
        }
    });
    assert!(max_diff(&plain, &edited) < 1e-6, "{}", max_diff(&plain, &edited));
}
