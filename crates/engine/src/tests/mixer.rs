use super::*;

/// The cue bus's meter reads what's on it, at its level; a channel not on it, nothing.
#[test]
fn the_cue_meter_reads_what_is_on_the_bus() {
    let mut p = input_project();
    p.cue_output = deckard_model::AudioDest::Ext(1);
    p.cue_level = deckard_model::level_from_db(-6.0);
    let g = deckard_model::level_gain(p.cue_level) as f32;
    let mut e = input_project_engine(&p);
    e.sequencing = false;
    steady_outs(&mut e, 80);
    assert_eq!(e.cue.peak, (0.0, 0.0), "nothing on it");
    p.groups[0].sounds[0].cue = true;
    let mut e = input_project_engine(&p);
    e.sequencing = false;
    let (master, cue) = steady_outs(&mut e, 80);
    assert!(near(master, 0.0) && near(cue, 0.5 * g), "on Out 2: {master} {cue}");
    assert!(near(e.cue.peak.0, 0.5 * g) && near(e.cue.peak.1, 0.5 * g), "{:?}", e.cue.peak);
}

/// On the master's output, the bus is past the master: its level (and the meter shows
/// the bus), and its plug-ins, which a cued channel doesn't go through.
#[test]
fn the_cue_on_the_masters_output_is_past_the_master() {
    let mut p = input_project();
    (p.master_output, p.cue_output) = (deckard_model::AudioDest::Ext(0), deckard_model::AudioDest::Ext(0));
    p.master_level = deckard_model::level_from_db(-12.0);
    let m = deckard_model::level_gain(p.master_level) as f32;
    let mut e = input_project_engine(&p);
    e.sequencing = false;
    let mut chain = bare_chain(Vec::new(), e.sr);
    push_half_gain(&mut chain);
    e.master_fx = Some(chain);
    let (master, _) = steady_outs(&mut e, 120);
    assert!(near(master, 0.5 * 0.5 * m), "through the master: {master}");
    p.groups[0].sounds[0].cue = true;
    let mut e = input_project_engine(&p);
    e.sequencing = false;
    let mut chain = bare_chain(Vec::new(), e.sr);
    push_half_gain(&mut chain);
    e.master_fx = Some(chain);
    let (master, _) = steady_outs(&mut e, 120);
    assert!(near(master, 0.5), "cued: past the master's plug-ins and level: {master}");
    assert!(near(e.cue.peak.0, 0.5), "{:?}", e.cue.peak);
    assert!(e.master_peak.0 < 0.01, "not on the master's meter: {:?}", e.master_peak);
}

/// A group is on the bus through its sounds, or with effects, its own bus; a sound
/// through a group with effects on the bus isn't itself.
#[test]
fn what_is_on_the_cue_bus_follows_the_routes() {
    use deckard_model::AudioDest as D;
    let mut p = input_project();
    p.groups[0].cue = true;
    assert!(CueBus::of(&p).sounds[0][0], "through its group");
    assert!(CueBus::of(&p).groups[0]);
    p.groups[0].slots = vec![driven_saturator()];
    assert!(!CueBus::of(&p).sounds[0][0], "its group's bus is");
    assert!(CueBus::of(&p).groups[0]);
    p.groups[0].slots.clear();
    p.groups[0].cue = false;
    p.groups[0].sounds[0].channel.dest = D::Master;
    assert!(!CueBus::of(&p).sounds[0][0]);
    let mut e = input_project_engine(&p);
    e.sequencing = false;
    steady(&mut e, 80);
    assert_eq!(e.cue.peak, (0.0, 0.0));
    // With effects and on the bus: metered after them.
    p.groups[0].sounds[0].channel.dest = D::Parent;
    p.groups[0].slots = vec![driven_saturator()];
    p.groups[0].cue = true;
    let mut e = input_project_engine(&p);
    e.sequencing = false;
    steady_outs(&mut e, 400);
    assert!(e.cue.peak.0 > 0.1, "{:?}", e.cue.peak);
}

/// The metronome and the prehear are on the cue bus: its output, its level, its meter.
#[test]
fn the_prehear_is_on_the_cue_bus() {
    let mut p = input_project();
    p.cue_output = deckard_model::AudioDest::Ext(1);
    p.cue_level = deckard_model::level_from_db(-6.0);
    let g = deckard_model::level_gain(p.cue_level) as f32;
    let mut e = input_project_engine(&p);
    e.sequencing = false;
    let (master, cue) = hear(&mut e, 20);
    assert_eq!(master, 0.0);
    assert!(near(cue, 0.5 * g), "{cue}");
    assert!(near(e.cue.peak.0, 0.5 * g), "{:?}", e.cue.peak);
}

/// The metronome goes the same way.
#[test]
fn the_click_is_on_the_cue_bus() {
    let mut p = input_project();
    p.cue_output = deckard_model::AudioDest::Ext(1);
    // (At a rate a click can be heard at.)
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    e.metronome = true;
    let mut master = 0f32;
    let mut cue = 0f32;
    for _ in 0..40 {
        let mut out = vec![0f32; 16];
        let mut ext = vec![vec![0f32; 16]; 2];
        e.process_outputs(&mut out, &mut ext);
        master = out.iter().fold(master, |m, v| m.max(v.abs()));
        cue = ext[1].iter().fold(cue, |m, v| m.max(v.abs()));
    }
    assert_eq!(master, 0.0);
    assert!(cue > 0.01, "{cue}");
    assert!(e.cue.peak.0 > 0.01, "{:?}", e.cue.peak);
}

/// With the cue on the master's output (the default), they sound there, the master's
/// fader not in the way.
#[test]
fn the_prehear_on_the_masters_output_is_past_its_fader() {
    let mut p = input_project();
    p.master_level = deckard_model::level_from_db(-12.0);
    let mut e = input_project_engine(&p);
    e.sequencing = false;
    let (master, _) = hear(&mut e, 20);
    assert!(near(master, 0.5), "{master}");
}

/// A cue output the interface doesn't have: they go on being heard, on the master's.
#[test]
fn the_prehear_falls_back_to_the_masters_output() {
    let mut p = input_project();
    p.cue_output = deckard_model::AudioDest::Ext(7);
    let mut e = input_project_engine(&p);
    e.sequencing = false;
    let (master, _) = hear(&mut e, 20);
    assert!(master > 0.1, "{master}");
    p.cue_output = deckard_model::AudioDest::None;
    let mut e = input_project_engine(&p);
    e.sequencing = false;
    let (master, _) = hear(&mut e, 20);
    assert!(near(master, 0.5), "no cue output at all: {master}");
}

/// A sound's level follows its lane while the pattern plays, from its fader's place;
/// stopped, it's the fader's own again.
#[test]
fn a_lane_moves_a_sounds_level() {
    let mut p = fader_project();
    p.groups[0].sounds[0].modulators = vec![fader_lane("level")];
    p.groups[0].patterns[0].mods = vec![fader_point(Some(0), "level", 0, -0.25)];
    assert_eq!(sound_gain(&p, 0, 0), (1.0, 1.0), "its fader isn't in the route");
    let mut e = fader_engine(&p);
    let want = 0.5 * deckard_model::level_gain(0.5) as f32;
    assert!(near(steady(&mut e, 100).0, want), "the lane: {want}");
    e.sequencing = false;
    assert!(near(steady(&mut e, 100).0, 0.5), "stopped: the fader's own");
}

/// Points are joined by lines, and the lane holds only where its pattern plays.
#[test]
fn a_lane_is_followed_along_the_pattern() {
    let mut p = fader_project();
    p.groups[0].sounds[0].modulators = vec![fader_lane("level")];
    p.groups[0].patterns[0].mods = vec![fader_point(Some(0), "level", 0, 0.0), fader_point(Some(0), "level", 2 * PPQ, -0.5)];
    let t = timeline(&p, Mode::Arrangement);
    assert_eq!(t.faders.len(), 2, "a pass per bar of the section: {:?}", t.faders);
    assert_eq!((t.faders[1].from, t.faders[1].to), (4 * PPQ, 8 * PPQ));
    let mut e = fader_engine(&p);
    // A quarter is 500 frames at 120 BPM and 1000 Hz: there, half way (a little behind
    // it, gliding); past the last point, it holds.
    let halfway = steady(&mut e, 500).0;
    assert!((halfway - 0.5 * deckard_model::level_gain(0.5) as f32).abs() < 0.02, "{halfway}");
    let held = steady(&mut e, 600).0;
    assert!(near(held, 0.5 * deckard_model::level_gain(0.25) as f32), "{held}");
}

/// A fader at the bottom still rises with its lane (no gain to scale from).
#[test]
fn a_lane_lifts_a_silent_fader() {
    let mut p = fader_project();
    p.groups[0].sounds[0].level = 0.0;
    p.groups[0].sounds[0].modulators = vec![fader_lane("level")];
    p.groups[0].patterns[0].mods = vec![fader_point(Some(0), "level", 0, 0.75)];
    let mut e = fader_engine(&p);
    assert!(near(steady(&mut e, 100).0, 0.5));
    e.sequencing = false;
    assert!(near(steady(&mut e, 100).0, 0.0));
}

/// Pan's lane: a deviation of its knob's travel, which covers −1..1.
#[test]
fn a_lane_moves_a_sounds_pan() {
    let mut p = fader_project();
    p.groups[0].sounds[0].modulators = vec![fader_lane("pan")];
    p.groups[0].patterns[0].mods = vec![fader_point(Some(0), "pan", 0, 0.5)];
    let mut e = fader_engine(&p);
    let (l, r) = steady(&mut e, 100);
    assert!(l.abs() < 1e-4 && r > 0.5, "right: {l} {r}");
    e.sequencing = false;
    let (l, r) = steady(&mut e, 100);
    assert!(near(l, r), "the middle again: {l} {r}");
}

/// A group's lane moves the sounds that go through it, and with effects, its bus.
#[test]
fn a_lane_moves_a_groups_level() {
    let mut p = fader_project();
    p.groups[0].modulators = vec![fader_lane("level")];
    p.groups[0].patterns[0].mods = vec![fader_point(None, "level", 0, -0.25)];
    let mut e = fader_engine(&p);
    assert_eq!(e.sound_faders[0][0], [0], "no bus: on the sound's way out");
    let want = 0.5 * deckard_model::level_gain(0.5) as f32;
    assert!(near(steady(&mut e, 100).0, want));
    e.sequencing = false;
    assert!(near(steady(&mut e, 100).0, 0.5));
    // With effects, it's after them.
    p.groups[0].slots = vec![driven_saturator()];
    let mut e = fader_engine(&p);
    assert!(e.sound_faders[0][0].is_empty());
    assert_eq!(e.bus_faders[0], [0]);
    let moved = steady(&mut e, 500).0;
    e.sequencing = false;
    let own = steady(&mut e, 500).0;
    assert!((moved / own - deckard_model::level_gain(0.5) as f32).abs() < 1e-3, "{moved} / {own}");
}

/// Only the faders a sound's way out passes through move it.
#[test]
fn a_fader_off_the_route_leaves_a_sound_alone() {
    use deckard_model::AudioDest as D;
    let mut p = fader_project();
    p.groups[0].modulators = vec![fader_lane("level")];
    p.groups[0].sounds.push(Sound { modulators: vec![fader_lane("level")], ..Default::default() });
    // Past its group, to a bus sound with a lane.
    p.groups[0].sounds[0].channel.dest = D::Sound(0, 1);
    p.groups[0].sounds[1].channel.dest = D::Master;
    let (faders, sounds, _) = channel_faders(&p);
    assert_eq!(faders.iter().map(|f| (f.group, f.sound)).collect::<Vec<_>>(), [(0, None), (0, Some(1))]);
    assert_eq!(sounds[0][0], [1], "the bus sound's, not the group's");
    // On the cue bus: none of them.
    p.groups[0].sounds[0].cue = true;
    assert!(channel_faders(&p).1[0][0].is_empty());
}

/// With no lane, nothing changes: the fader's in the route as ever.
#[test]
fn a_fader_with_no_lane_stays_in_the_route() {
    let mut p = fader_project();
    p.groups[0].sounds[0].level = 0.5;
    let want = deckard_model::level_gain(0.5) as f32;
    assert!(near(sound_gain(&p, 0, 0).0, want));
    let e = fader_engine(&p);
    assert!(e.faders.is_empty());
    // Another of the channel's properties modulated isn't its fader.
    p.groups[0].sounds[0].modulators = vec![fader_lane("aux1_level")];
    assert!(near(sound_gain(&p, 0, 0).0, want));
}

/// A fader moved during play: the lane plays on from its new place, with the same
/// timeline.
#[test]
fn a_moved_fader_keeps_its_lane() {
    let mut p = fader_project();
    p.groups[0].sounds[0].modulators = vec![fader_lane("level")];
    p.groups[0].patterns[0].mods = vec![fader_point(Some(0), "level", 0, -0.25)];
    let mut cache = SampleCache::default();
    let (mut e, built) = Engine::new(&p, None, 1000, Mode::Arrangement, &mut cache);
    e.live_in = Some(Default::default());
    e.looping = true;
    steady(&mut e, 100);
    let mut moved = p.clone();
    moved.groups[0].sounds[0].level = 0.5;
    let (fresh, _) = Engine::new_after(&moved, Some((&p, built.chains.as_slice())), None, 1000, Mode::Arrangement, &mut cache);
    assert!(fresh.keeps_timelines(), "nothing that makes the timeline changed");
    drop(e.adopt(fresh, &moved));
    assert!(near(steady(&mut e, 100).0, 0.5 * deckard_model::level_gain(0.25) as f32));
}

/// A sound taking audio in with effects (and nothing else in its chain): the input
/// goes through them.
#[test]
fn an_input_sound_plays_through_its_effects() {
    let mut dry = vec![0f32; 128];
    input_engine(Vec::new(), deckard_model::AudioDest::Parent).process(&mut dry);
    assert!(dry[100].abs() > 0.1, "dry: {dry:?}");
    let mut wet = vec![0f32; 128];
    input_engine(vec![driven_saturator()], deckard_model::AudioDest::Parent).process(&mut wet);
    assert!(max_diff(&dry, &wet) > 0.05, "the Saturator changes it: {dry:?} / {wet:?}");
}

/// A sound whose chain starts with an instrument takes no audio in: it would play
/// through the instrument's chain (and click).
#[test]
fn an_instrument_sound_takes_no_input() {
    let sampler =
        deckard_model::SlotInfo { module: "Sampler".into(), name: "Sampler".into(), active: true, plugin: None, params: Vec::new() };
    let mut e = input_engine(vec![sampler, driven_saturator()], deckard_model::AudioDest::Parent);
    assert!(!e.takes_input());
    let mut out = vec![0f32; 128];
    e.process(&mut out);
    assert!(out.iter().all(|x| x.abs() < 1e-6), "{out:?}");
}

/// A sound taking audio in plays where it's routed, not straight to the master.
#[test]
fn an_input_sound_plays_on_its_output() {
    let mut e = input_engine(Vec::new(), deckard_model::AudioDest::Ext(1));
    let (mut out, mut ext) = (vec![0f32; 128], vec![vec![0f32; 128]; 4]);
    e.process_outputs(&mut out, &mut ext);
    assert!(out.iter().all(|x| x.abs() < 1e-6), "not on the master: {out:?}");
    assert!(ext[1][100].abs() > 0.1, "on Out 2: {:?}", ext[1]);
}

#[test]
fn midi_note_tail_releases_at_its_deadline_past_the_last_bar() {
    let mut p = project();
    let bar = 4 * PPQ;
    p.sections[0].length = bar;
    p.groups[0].sounds[0].sampler = None;
    p.groups[0].sounds[0].channel.midi_out = deckard_model::MidiDest::Port("Synth".into());
    let pattern = &mut p.groups[0].patterns[0];
    let mut note = pattern.notes[0].clone();
    note.position = bar - 60;
    note.length = 100;
    pattern.notes = vec![note, Note { position: bar + 42, pitch: 61, ..note }];

    // At 1920 Hz and 120 BPM one tick is one frame. The gate ends 40 frames
    // past the bar, partway through the second process call.
    let mut cache = SampleCache::default();
    for (mode, looping) in [(Mode::Arrangement, false), (Mode::Scene(0), true)] {
        let (mut e, _) = Engine::new(&p, None, 1920, mode, &mut cache);
        e.looping = looping;
        let (tx, rx) = std::sync::mpsc::sync_channel(16);
        e.midi_tx = Some(tx);
        e.process(&mut vec![0.0; 2 * (bar as usize + 39)]);
        let on = rx.try_recv().expect("late note in the last bar plays");
        assert_eq!(on.bytes(), &[0x90, 60, 127]);
        assert!(rx.try_recv().is_err(), "the note is still held past the bar");

        e.process(&mut [0.0; 16]);
        let off = rx.try_recv().expect("note-off is sent inside the block containing its deadline");
        assert_eq!(off.bytes(), &[0x80, 60, 0]);
        assert!(rx.try_recv().is_err(), "the note that starts beyond the pattern length stays silent");
    }
}

/// A sound sends its notes to its MIDI output: its channel, transposed, with
/// note-offs at the note's end; muted sounds send nothing.
#[test]
fn sounds_send_midi() {
    let mut p = project();
    p.groups[0].sounds[0].channel.midi_out = deckard_model::MidiDest::Port("Synth".into());
    p.groups[0].sounds[0].channel.midi_out_channel = 2;
    p.groups[0].sounds[0].channel.transpose = -12;
    let mut e = click_engine_for(&p, 1000);
    let (tx, rx) = std::sync::mpsc::sync_channel(64);
    e.midi_tx = Some(tx);
    let mut out = vec![0f32; 2 * 1000];
    e.process(&mut out);
    let msgs: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
    // 120 BPM: two beats a second; notes last a sixteenth (125 ms).
    assert_eq!(msgs, vec![vec![0x92, 48, 127], vec![0x82, 48, 0], vec![0x92, 48, 127], vec![0x82, 48, 0]]);
    // Live input with thru: out too; release ends it.
    e.note_in(0, 0, 64, 90, true);
    e.note_release(0, 0, 64);
    let msgs: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
    assert_eq!(msgs, vec![vec![0x92, 52, 90], vec![0x82, 52, 0]]);
    // Muted: nothing.
    p.groups[0].sounds[0].muted = true;
    let mut e = click_engine_for(&p, 1000);
    let (tx, rx) = std::sync::mpsc::sync_channel(64);
    e.midi_tx = Some(tx);
    e.process(&mut out);
    assert_eq!(rx.try_iter().count(), 0);
}
