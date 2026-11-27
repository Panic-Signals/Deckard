use super::*;

#[test]
fn remaining_aux_sound_pre_and_post_fader_reach_the_selected_output() {
    for pre in [false, true] {
        let mut p = fader_project();
        let s = &mut p.groups[0].sounds[0];
        s.channel.dest = deckard_model::AudioDest::None;
        s.channel.aux_dest[0] = deckard_model::AudioDest::Master;
        s.channel.aux_pre[0] = pre;
        s.aux[0] = 0.75;
        s.level = 0.5;
        let mut e = fader_engine(&p);
        let got = steady(&mut e, 100).0;
        let want = 0.5 * if pre { 1.0 } else { deckard_model::level_gain(0.5) as f32 };
        assert!(near(got, want), "pre={pre}: {got}, want {want}");
    }
}

#[test]
fn remaining_aux_group_send_works_without_a_group_effect() {
    let mut p = fader_project();
    let g = &mut p.groups[0];
    g.channel.dest = deckard_model::AudioDest::None;
    g.channel.aux_dest[1] = deckard_model::AudioDest::Master;
    g.aux[1] = 0.75;
    let mut e = fader_engine(&p);
    assert!(e.buses[0].is_some());
    assert!(near(steady(&mut e, 100).0, 0.5));
}

#[test]
fn remaining_aux_automation_plays_and_stopped_returns_to_base() {
    let mut p = fader_project();
    let s = &mut p.groups[0].sounds[0];
    s.channel.dest = deckard_model::AudioDest::None;
    s.channel.aux_dest[0] = deckard_model::AudioDest::Master;
    s.aux[0] = 0.75;
    s.modulators = vec![fader_lane("aux1_level")];
    p.groups[0].patterns[0].mods = vec![fader_point(Some(0), "aux1_level", 0, -0.25)];
    let mut e = fader_engine(&p);
    assert!(near(steady(&mut e, 100).0, 0.5 * deckard_model::level_gain(0.5) as f32));
    e.sequencing = false;
    assert!(near(steady(&mut e, 100).0, 0.5));
}

#[test]
fn remaining_aux_cued_sound_does_not_leak_to_the_room() {
    let mut p = fader_project();
    let s = &mut p.groups[0].sounds[0];
    s.cue = true;
    s.channel.aux_dest[0] = deckard_model::AudioDest::Master;
    s.aux[0] = 0.75;
    p.cue_output = deckard_model::AudioDest::Ext(1);
    let mut e = fader_engine(&p);
    assert!(near(steady(&mut e, 100).0, 0.0));
}

#[test]
fn remaining_aux_return_buses_run_after_their_senders() {
    let mut p = fader_project();
    let mut ret = p.groups[0].clone();
    ret.sounds.clear();
    ret.channel.dest = deckard_model::AudioDest::None;
    ret.channel.aux_dest[0] = deckard_model::AudioDest::Master;
    ret.aux[0] = 0.75;
    p.groups.push(ret);
    p.groups[0].channel.dest = deckard_model::AudioDest::None;
    p.groups[0].channel.aux_dest[0] = deckard_model::AudioDest::Group(1);
    p.groups[0].aux[0] = 0.75;
    let mut e = fader_engine(&p);
    assert_eq!(e.bus_order, vec![0, 1]);
    assert!(near(steady(&mut e, 100).0, 0.5));
    p.groups[1].channel.aux_dest[0] = deckard_model::AudioDest::Group(0);
    let mut e = fader_engine(&p);
    assert!(e.group_aux.iter().all(|s| s.is_empty()));
    assert!(near(steady(&mut e, 100).0, 0.0));
}

#[test]
fn remaining_aux_group_cycles_are_silent_and_do_not_feed_back() {
    let mut p = fader_project();
    p.groups[0].channel.aux_dest[0] = deckard_model::AudioDest::Group(0);
    p.groups[0].aux[0] = 1.0;
    let e = fader_engine(&p);
    assert!(e.group_aux[0].is_empty());
}

fn passive_fx_slot() -> deckard_model::SlotInfo {
    deckard_model::SlotInfo { module: "Compressor".into(), name: "Compressor".into(), active: false, plugin: None, params: Vec::new() }
}

fn bus_sound_with_fx() -> deckard_model::Sound {
    deckard_model::Sound {
        level: 0.75,
        slots: vec![passive_fx_slot()],
        channel: deckard_model::SoundChannel { dest: deckard_model::AudioDest::Master, ..Default::default() },
        ..Default::default()
    }
}

/// A sound plays into a bus Sound, which goes to the master; a group's or sound's
/// Limiter is side-chained from that bus Sound. The bus Sound runs with the buses,
/// so it must run before its listeners both initially and after incremental edits.
#[test]
fn sidechain_from_a_bus_sound_ducks_groups_and_sounds_across_edits() {
    let mut p = project();
    let click = p.groups[0].sounds[0].clone();
    let mut bus = bus_sound_with_fx();
    bus.slots[0].active = true;
    bus.level = 0.0;
    let mut kick = click.clone();
    kick.channel.dest = deckard_model::AudioDest::Sound(0, 1);
    p.groups[0].sounds = vec![click, bus, kick];
    let lim = vec![("mode".into(), 1.0), ("threshold".into(), -30.0), ("release".into(), 0.5), ("ceiling".into(), 1.0)];
    p.groups[0].slots =
        vec![deckard_model::SlotInfo { module: "Limiter".into(), name: "Limiter".into(), active: true, plugin: None, params: lim.clone() }];
    p.groups[0].channel.sidechain = vec![(0, deckard_model::ScSource::Sound(0, 1))];

    for bare_bus in [false, true] {
        for incremental in [false, true] {
            let mut base = p.clone();
            if bare_bus {
                base.groups[0].sounds[1].slots.clear();
            }
            let run = |with_source: bool| {
                let mut cache = SampleCache::default();
                let (mut e, rep) = Engine::new(&base, None, 48_000, Mode::Arrangement, &mut cache);
                let mut frames = vec![[0f32; 2]; 100];
                frames[0] = [1.0, 1.0];
                set_sample(&mut e, frames, &[], false);
                if incremental {
                    let mut edited = base.clone();
                    edited.master_level = 0.78;
                    let (fresh, _) =
                        Engine::new_after(&edited, Some((&base, rep.chains.as_slice())), None, 48_000, Mode::Arrangement, &mut cache);
                    drop(e.adopt(fresh, &edited));
                }
                let at = |n: usize| e.bus_order.iter().position(|&x| x == n);
                assert!(
                    at(sound_bus_id(0, 1)).is_some() && at(sound_bus_id(0, 1)) < at(0),
                    "bare_bus={bare_bus} incremental={incremental}: {:?}",
                    e.bus_order
                );
                e.sequencing = false;
                e.note_on(0, 0, 60, 127, 960);
                if with_source {
                    e.note_on(0, 2, 60, 127, 960);
                }
                let mut out = vec![0f32; 2 * 4800];
                e.process(&mut out);
                out.iter().fold(0f32, |m, v| m.max(v.abs()))
            };
            let (alone, ducked) = (run(false), run(true));
            assert!(alone > 0.4, "bare_bus={bare_bus} incremental={incremental}: alone={alone}");
            assert!(ducked < alone * 0.1, "bare_bus={bare_bus} incremental={incremental}: ducked={ducked} vs {alone}");
        }
    }

    let mut sp = project();
    let mut listener = sp.groups[0].sounds[0].clone();
    listener.slots.push(deckard_model::SlotInfo {
        module: "Limiter".into(),
        name: "Limiter".into(),
        active: true,
        plugin: None,
        params: lim,
    });
    let listen_slot = listener.slots.len() - 1;
    listener.channel.sidechain = vec![(listen_slot, deckard_model::ScSource::Sound(0, 1))];
    let mut bus = bus_sound_with_fx();
    bus.slots[0].active = true;
    bus.level = 0.0;
    let mut kick = sp.groups[0].sounds[0].clone();
    kick.channel.dest = deckard_model::AudioDest::Sound(0, 1);
    sp.groups[0].sounds = vec![listener, bus, kick];
    sp.groups[0].slots =
        vec![deckard_model::SlotInfo { module: "EQ".into(), name: "EQ".into(), active: true, plugin: None, params: Vec::new() }];
    for incremental in [false, true] {
        let run = |with_source: bool| {
            let mut cache = SampleCache::default();
            let (mut e, rep) = Engine::new(&sp, None, 48_000, Mode::Arrangement, &mut cache);
            let mut frames = vec![[0f32; 2]; 100];
            frames[0] = [1.0, 1.0];
            set_sample(&mut e, frames, &[], false);
            if incremental {
                let mut edited = sp.clone();
                edited.master_level = 0.78;
                let (fresh, _) =
                    Engine::new_after(&edited, Some((&sp, rep.chains.as_slice())), None, 48_000, Mode::Arrangement, &mut cache);
                drop(e.adopt(fresh, &edited));
            }
            let at = |n: usize| e.bus_order.iter().position(|&x| x == n);
            assert!(at(sound_bus_id(0, 1)) < at(sound_bus_id(0, 0)) && at(sound_bus_id(0, 0)) < at(0));
            e.sequencing = false;
            e.note_on(0, 0, 60, 127, 960);
            if with_source {
                e.note_on(0, 2, 60, 127, 960);
            }
            let mut out = vec![0f32; 2 * 4800];
            e.process(&mut out);
            out.iter().fold(0f32, |m, v| m.max(v.abs()))
        };
        let (alone, ducked) = (run(false), run(true));
        assert!(alone > 0.4, "incremental={incremental}: sound alone={alone}");
        assert!(ducked < alone * 0.1, "incremental={incremental}: sound ducked={ducked} vs {alone}");
    }
}

#[test]
fn aux_review_dry_plus_both_aux_sends_to_group_sound_bus_and_ext_outputs() {
    let mut p = fader_project();
    p.groups[0].sounds.push(bus_sound_with_fx());

    let mut ret_grp = deckard_model::Group { level: 0.75, slots: vec![passive_fx_slot()], ..Default::default() };
    ret_grp.channel.dest = deckard_model::AudioDest::Ext(1);
    p.groups.push(ret_grp);

    let src = &mut p.groups[0].sounds[0];
    src.level = 0.5;
    src.channel.dest = deckard_model::AudioDest::Master;
    src.channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 1);
    src.channel.aux_pre[0] = false;
    src.aux[0] = 0.75;
    src.channel.aux_dest[1] = deckard_model::AudioDest::Group(1);
    src.channel.aux_pre[1] = true;
    src.aux[1] = 0.75;

    let mut e = fader_engine(&p);
    push_half_gain(e.fx[0][1].as_mut().unwrap());
    push_half_gain(&mut e.buses[1].as_mut().unwrap().chain);
    let (master, ext2) = steady_outs(&mut e, 120);
    let fader = deckard_model::level_gain(0.5) as f32;
    // Dry (0.5 * fader) + Aux 1 post-fader through Sound(0, 1) with HalfGainEffect (0.5 * fader * 0.5).
    let want_master = 0.5 * fader + 0.5 * fader * 0.5;
    // Aux 2 pre-fader through Group(1) with HalfGainEffect onto Ext(1) (0.5 * 0.5).
    let want_ext2 = 0.5 * 0.5;
    assert!(near(master, want_master), "master={master}, want={want_master}");
    assert!(near(ext2, want_ext2), "ext2={ext2}, want={want_ext2}");
}

#[test]
fn aux_review_pre_and_post_source_level_and_pan_on_sound_and_group_sends() {
    let lvl = deckard_model::level_from_db(-6.0);
    let g_lvl = deckard_model::level_gain(lvl) as f32;
    let pan = -0.5;
    let (pl, pr) = pan_gains(pan);
    for pre in [false, true] {
        // Sound send with source level + pan
        let mut p = fader_project();
        let s = &mut p.groups[0].sounds[0];
        s.level = lvl;
        s.pan = pan;
        s.channel.dest = deckard_model::AudioDest::None;
        s.channel.aux_dest[1] = deckard_model::AudioDest::Master;
        s.channel.aux_pre[1] = pre;
        s.aux[1] = 0.75;
        let mut e = fader_engine(&p);
        let (l, r) = steady(&mut e, 100);
        let (wl, wr) = if pre { (0.5, 0.5) } else { (0.5 * g_lvl * pl, 0.5 * g_lvl * pr) };
        assert!(near(l, wl) && near(r, wr), "sound pre={pre}: got ({l}, {r}), want ({wl}, {wr})");

        // Group send with group level + pan
        let mut p = fader_project();
        let g = &mut p.groups[0];
        g.level = lvl;
        g.pan = pan;
        g.channel.dest = deckard_model::AudioDest::None;
        g.channel.aux_dest[0] = deckard_model::AudioDest::Master;
        g.channel.aux_pre[0] = pre;
        g.aux[0] = 0.75;
        let mut e = fader_engine(&p);
        let (l, r) = steady(&mut e, 100);
        assert!(near(l, wl) && near(r, wr), "group pre={pre}: got ({l}, {r}), want ({wl}, {wr})");
    }
}

#[test]
fn aux_review_bus_sound_processes_incoming_main_and_aux_audio_and_onward_sends() {
    // Put the Bus Sound at (0, 0) and the sender at (0, 1) so index order is reversed:
    // topological bus_order must run sender (0, 1) in render_pass and Bus Sound (0, 0) in run_buses.
    let mut p = fader_project();
    let mut sender = p.groups[0].sounds[0].clone();
    sender.channel.dest = deckard_model::AudioDest::None;
    sender.channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 0);
    sender.aux[0] = 0.75;

    let mut bus_snd = bus_sound_with_fx();
    bus_snd.channel.aux_dest[1] = deckard_model::AudioDest::Ext(1);
    bus_snd.aux[1] = 0.75;
    p.groups[0].sounds = vec![bus_snd, sender];

    let mut e = fader_engine(&p);
    assert_eq!(e.bus_order, vec![sound_bus_id(0, 0)]);
    push_half_gain(e.fx[0][0].as_mut().unwrap());
    let (master, ext2) = steady_outs(&mut e, 120);
    assert!(near(master, 0.25), "master={master}");
    assert!(near(ext2, 0.25), "onward send ext2={ext2}");
    let meters = e.take_meters();
    assert!(near(meters[0][0].0, 0.25), "bus sound meter={:?}", meters[0][0]);

    // Group main output + Group Aux send feeding a Bus Sound in Group 1
    let mut p = fader_project();
    p.groups[0].slots = vec![passive_fx_slot()];
    p.groups[0].channel.dest = deckard_model::AudioDest::Sound(1, 0);
    p.groups[0].channel.aux_dest[0] = deckard_model::AudioDest::Sound(1, 0);
    p.groups[0].aux[0] = 0.75;

    let g1 = deckard_model::Group { level: 0.75, sounds: vec![bus_sound_with_fx()], ..Default::default() };
    p.groups.push(g1);

    let mut e = fader_engine(&p);
    assert_eq!(e.bus_order, vec![0, sound_bus_id(1, 0)]);
    push_half_gain(e.fx[1][0].as_mut().unwrap());
    let (master, _) = steady(&mut e, 120);
    // Group 0 sends 0.5 via main + 0.5 via Aux 1 = 1.0 into Sound(1, 0), halved by HalfGainEffect -> 0.5.
    assert!(near(master, 0.5), "group->sound bus master={master}");

    // Active native Maschine Compressor on Bus Sound attenuates incoming Aux audio
    let mut p_native = fader_project();
    let mut comp_bus = bus_sound_with_fx();
    comp_bus.slots = vec![deckard_model::SlotInfo {
        module: "Compressor".into(),
        name: "Compressor".into(),
        active: true,
        plugin: None,
        params: [("mode", 0.0), ("threshold", 0.0), ("amount", 0.0), ("knee", 0.5), ("attack", 0.2), ("release", 0.3), ("gain", 0.5)]
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect(),
    }];
    p_native.groups[0].sounds.push(comp_bus);
    p_native.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::None;
    p_native.groups[0].sounds[0].channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 1);
    p_native.groups[0].sounds[0].aux[0] = 0.75;
    let mut e_native = fader_engine(&p_native);
    let (native_out, _) = steady(&mut e_native, 120);
    // Compressor with amount=0.0, gain=0.5 (-18 dB = 10^(-18/20) ≈ 0.12589) scales 0.5 input to ~0.06295.
    assert!((native_out - 0.5 * 0.12589254).abs() < 1e-3, "native compressor on bus sound: got {native_out}");
}

#[test]
fn aux_review_mute_solo_and_cue_semantics_across_bus_sounds_and_aux_sends() {
    let mut p = fader_project();
    p.groups[0].sounds.push(bus_sound_with_fx());
    p.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::None;
    p.groups[0].sounds[0].channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 1);
    p.groups[0].sounds[0].aux[0] = 0.75;

    // 1. Trigger mute (audio_mute = false) on Bus Sound still passes incoming audio.
    let mut trig_muted = p.clone();
    trig_muted.groups[0].sounds[1].muted = true;
    trig_muted.groups[0].sounds[1].channel.audio_mute = false;
    let mut e = fader_engine(&trig_muted);
    push_half_gain(e.fx[0][1].as_mut().unwrap());
    assert!(near(steady(&mut e, 100).0, 0.25));

    // 2. Audio Mute (audio_mute = true) on Bus Sound silences it.
    let mut audio_muted = trig_muted.clone();
    audio_muted.groups[0].sounds[1].channel.audio_mute = true;
    let mut e = fader_engine(&audio_muted);
    assert!(near(steady(&mut e, 100).0, 0.0));

    // 3. Soloing sender (0, 0) via retain_sounds keeps Bus Sound (0, 1)'s FX return alive.
    let mut e = fader_engine(&p);
    push_half_gain(e.fx[0][1].as_mut().unwrap());
    e.retain_sounds(|g, s| (g, s) == (0, 0));
    assert!(near(steady(&mut e, 100).0, 0.25));

    // 4. Cue on Bus Sound routes post-FX signal to cue output (Ext 2) without leaking to Master.
    let mut cued = p.clone();
    cued.cue_output = deckard_model::AudioDest::Ext(1);
    cued.groups[0].sounds[1].cue = true;
    let mut e = fader_engine(&cued);
    e.sequencing = false;
    push_half_gain(e.fx[0][1].as_mut().unwrap());
    let (master, cue_out) = steady_outs(&mut e, 120);
    assert!(near(master, 0.0), "cued bus sound leaked to master: {master}");
    assert!(near(cue_out, 0.25), "cued bus sound on cue out: {cue_out}");
    assert!(near(e.cue.peak.0, 0.25), "cue peak: {:?}", e.cue.peak);
}

#[test]
fn aux_review_feedback_cycles_across_sounds_and_groups_are_broken_without_runaway() {
    // Sound(0, 0) -> Aux 1 -> Sound(0, 1) -> Aux 1 -> Sound(0, 0)
    let mut p = fader_project();
    let mut s1 = bus_sound_with_fx();
    s1.channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 0);
    s1.aux[0] = 1.0;
    p.groups[0].sounds.push(s1);
    p.groups[0].sounds[0].slots = vec![passive_fx_slot()];
    p.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::None;
    p.groups[0].sounds[0].channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 1);
    p.groups[0].sounds[0].aux[0] = 0.75;

    let mut e = fader_engine(&p);
    let (l, r) = steady(&mut e, 200);
    assert!(l.is_finite() && r.is_finite() && l <= 0.5 + 1e-4, "cycle output=({l}, {r})");

    // Self-routing on Sound main dest and Aux dest is silenced
    let mut self_p = fader_project();
    self_p.groups[0].sounds[0].slots = vec![passive_fx_slot()];
    self_p.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::Sound(0, 0);
    self_p.groups[0].sounds[0].channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 0);
    self_p.groups[0].sounds[0].aux[0] = 1.0;
    let mut e = fader_engine(&self_p);
    assert!(e.sound_aux[0][0].is_empty());
    assert!(near(steady(&mut e, 100).0, 0.0));
}

#[test]
fn aux_review_live_adopt_glides_send_and_bus_edits_without_clicks_or_lost_fx_state() {
    let mut p0 = fader_project();
    p0.groups[0].sounds.push(bus_sound_with_fx());
    p0.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::None;

    let mut cache = SampleCache::default();
    let (mut e, rep0) = Engine::new(&p0, None, 1000, Mode::Arrangement, &mut cache);
    e.live_in = Some(Default::default());
    e.looping = true;
    push_half_gain(e.fx[0][1].as_mut().unwrap());
    assert!(near(steady(&mut e, 80).0, 0.0));

    // 1. Live-assign Aux 1 -> Sound(0, 1): must ramp smoothly from 0 to 0.25 (with HalfGainEffect preserved).
    let mut p1 = p0.clone();
    p1.groups[0].sounds[0].channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 1);
    p1.groups[0].sounds[0].aux[0] = 0.75;
    let (fresh, rep1) = Engine::new_after(&p1, Some((&p0, rep0.chains.as_slice())), None, 1000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh, &p1));

    let mut frame = [0f32; 32];
    (frame[0], frame[1]) = (0.5, 0.5);
    e.live_in.as_ref().unwrap().lock().unwrap().extend(std::iter::repeat_n(frame, 4));
    let mut first_block = [0f32; 8];
    e.process(&mut first_block);
    // At sr=1000, 10ms glide is 10 frames: sample 0 must be small (< 0.05), not a step jump to 0.25.
    assert!(first_block[0] > 0.0 && first_block[0] < 0.05, "step jump on send add: {:?}", first_block);
    assert!(near(steady(&mut e, 80).0, 0.25), "fx state lost or wrong steady level after adopt");

    // 2. Live-unassign Aux 1 -> None: must ramp smoothly down from 0.25 to 0.0 without a step drop to 0.
    let (fresh2, _) = Engine::new_after(&p0, Some((&p1, rep1.chains.as_slice())), None, 1000, Mode::Arrangement, &mut cache);
    drop(e.adopt(fresh2, &p0));
    e.live_in.as_ref().unwrap().lock().unwrap().extend(std::iter::repeat_n(frame, 4));
    let mut fade_block = [0f32; 8];
    e.process(&mut fade_block);
    assert!(fade_block[0] > 0.15 && fade_block[0] < 0.25, "abrupt drop on send remove: {:?}", fade_block);
    assert!(near(steady(&mut e, 80).0, 0.0), "did not settle to 0 after send remove");
}

#[test]
fn aux_review_bus_sound_automation_and_extra_output_strips() {
    // 1. Automation on a Bus Sound's onward Aux 2 send
    let mut p = fader_project();
    let mut bus_snd = bus_sound_with_fx();
    bus_snd.channel.dest = deckard_model::AudioDest::None;
    bus_snd.channel.aux_dest[1] = deckard_model::AudioDest::Master;
    bus_snd.aux[1] = 0.75;
    bus_snd.modulators = vec![fader_lane("aux2_level")];
    p.groups[0].sounds.push(bus_snd);
    p.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::Sound(0, 1);
    p.groups[0].patterns[0].mods = vec![fader_point(Some(1), "aux2_level", 0, -0.25)];

    let mut e = fader_engine(&p);
    let modded = 0.5 * deckard_model::level_gain(0.5) as f32;
    assert!(near(steady(&mut e, 100).0, modded));
    e.sequencing = false;
    assert!(near(steady(&mut e, 100).0, 0.5));

    // 2. Extra-output strip on Sound(0, 0) routed into Bus Sound(0, 1)
    let mut s = Strips::new(|p| {
        p.groups[0].sounds.push(bus_sound_with_fx());
        p.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::None;
        p.extra_outs[0].dest = deckard_model::AudioDest::Sound(0, 1);
    });
    push_half_gain(s.e.fx[0][1].as_mut().unwrap());
    // Input 0.5 -> Split bus 1 (x0.5 = 0.25) -> strip (level 1.0) -> Sound(0, 1) (HalfGainEffect x0.5 = 0.125).
    let (master, _) = s.block(0.5);
    assert!(near(master.0, 0.125) && near(master.1, 0.125), "strip->sound bus got={master:?}, want 0.125");
}
