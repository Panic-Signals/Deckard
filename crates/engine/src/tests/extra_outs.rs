use super::*;

/// A sound set to an external input plays it through its channel (gain, level).
#[test]
fn sounds_play_their_audio_input() {
    let mut p = project();
    p.groups[0].sounds[0].channel.audio_in = deckard_model::AudioSource::Ext(1); // In 2 L+R
    let mut e = click_engine_for(&p, 1000);
    e.sequencing = false;
    let live: LiveIn = Default::default();
    e.live_in = Some(live.clone());
    assert!(e.takes_input());
    let mut frame = [0f32; 32];
    (frame[2], frame[3]) = (0.5, -0.25);
    live.lock().unwrap().extend(std::iter::repeat_n(frame, 4));
    let mut out = vec![0f32; 8];
    e.process(&mut out);
    assert!((out[0] - 0.5).abs() < 1e-4 && (out[1] + 0.25).abs() < 1e-4, "{out:?}");
    assert!(live.lock().unwrap().is_empty());
}

/// A muted sound doesn't pass its external input (Audio Mute or not).
#[test]
fn a_muted_sound_plays_no_audio_input() {
    for audio_mute in [false, true] {
        let mut p = project();
        let snd = &mut p.groups[0].sounds[0];
        (snd.channel.audio_in, snd.muted, snd.channel.audio_mute) = (deckard_model::AudioSource::Ext(1), true, audio_mute);
        let mut e = click_engine_for(&p, 1000);
        e.sequencing = false;
        let live: LiveIn = Default::default();
        e.live_in = Some(live.clone());
        let mut frame = [0f32; 32];
        (frame[2], frame[3]) = (0.5, -0.25);
        live.lock().unwrap().extend(std::iter::repeat_n(frame, 4));
        let mut out = vec![0f32; 8];
        e.process(&mut out);
        assert!(out.iter().all(|v| *v == 0.0), "audio_mute {audio_mute}: {out:?}");
    }
}

static SPLIT_INSTRUMENT: deckard_plugins::native::Class = deckard_plugins::native::Class {
    cid: [8; 16],
    name: "Split Instrument",
    instrument: true,
    params: &[],
    pages: &[],
    make: |_| Box::new(Split { instrument: true, ..Default::default() }),
    scope: false,
};

/// [`Split`] as an effect in `owner`'s slot of `p`, listening as the engine sets it.
fn split_unit(owner: deckard_model::extra_out::Owner, p: &Project) -> Unit {
    let plugin = split_plugin(&SPLIT);
    let mut u = Unit::new(plugin_unit(plugin.clone(), &[], 1000.0), 0, Vec::new(), true);
    u.listen(owner, &plugin, &SPLIT.plugin_id(), p);
    u
}

/// A project whose only sound takes In 1, stopped, and an engine for it, with
/// [`Split`] on the master (its first slot).
fn split_engine(edit: impl FnOnce(&mut Project)) -> Engine {
    let mut p = project();
    p.groups[0].patterns[0].notes.clear();
    p.groups[0].sounds[0].channel.audio_in = deckard_model::AudioSource::Ext(0);
    edit(&mut p);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 1000, Mode::Arrangement, &mut cache);
    e.sequencing = false;
    e.live_in = Some(Default::default());
    let mut chain = bare_chain(vec![split_unit(deckard_model::extra_out::Owner::Master(0), &p)], 1000.0);
    chain.heard = Some(e.heard_board.clone());
    e.master_fx = Some(chain);
    e
}

#[derive(Default)]
struct HeldTestInstrument(u128);

impl deckard_plugins::native::Engine for HeldTestInstrument {
    fn set_param(&mut self, _: u32, _: f64) {}
    fn note_on(&mut self, pitch: u8, _: u8) {
        self.0 |= 1 << pitch;
    }
    fn note_off(&mut self, pitch: u8) {
        self.0 &= !(1 << pitch);
    }
    fn reset(&mut self) {
        self.0 = 0;
    }
    fn process(&mut self, l: &mut [f32], r: &mut [f32], _: &Transport) {
        let level = self.0.count_ones() as f32 / 8.0;
        l.fill(level);
        r.fill(level);
    }
}

static HELD_TEST_INSTRUMENT: deckard_plugins::native::Class = deckard_plugins::native::Class {
    cid: [9; 16],
    name: "Held Test Instrument",
    instrument: true,
    params: &[],
    pages: &[],
    make: |_| Box::<HeldTestInstrument>::default(),
    scope: false,
};

// Hold the instance as the UI would. The timeout lets a blocking regression
// fail on its output instead of hanging the test suite.
fn with_editor_lock(plugin: &Shared, f: impl FnOnce()) {
    std::thread::scope(|scope| {
        let (ready, acquired) = std::sync::mpsc::channel();
        let (done, release) = std::sync::mpsc::channel();
        scope.spawn(move || {
            let _lock = plugin.lock().unwrap();
            ready.send(()).unwrap();
            let _ = release.recv_timeout(std::time::Duration::from_secs(3));
        });
        acquired.recv().unwrap();
        f();
        let _ = done.send(());
    });
}

#[test]
fn busy_instrument_keeps_input_running_and_releases_only_the_ended_notes() {
    let mut e = split_engine(|_| {});
    let plugin = split_plugin(&HELD_TEST_INSTRUMENT);
    e.sounds[0][0] = None;
    e.instruments[0][0] = Some(InstrumentRt::new(plugin.clone(), (1.0, 1.0), false));
    e.note_hold(0, 0, 60, 127);
    e.note_hold(0, 0, 72, 127);
    assert!(about(split_block(&mut e, 0.25).0.0, 0.5));
    with_editor_lock(&plugin, || {
        e.note_release(0, 0, 60); // Preserve this off.
        e.note_hold(0, 0, 65, 127); // A missed on never plays late.
        assert!(about(split_block(&mut e, 0.25).0.0, 0.25));
    });
    assert!(about(split_block(&mut e, 0.25).0.0, 0.375), "only pitch 72 remains");
    with_editor_lock(&plugin, || {
        e.stop();
        assert!(about(split_block(&mut e, 0.25).0.0, 0.25));
    });
    assert!(about(split_block(&mut e, 0.25).0.0, 0.25), "stop releases the remaining pitch");
}

#[test]
fn busy_master_effect_passes_the_dry_mix() {
    let mut e = split_engine(|_| {});
    let FxUnit::Plugin { plugin, .. } = &e.master_fx.as_ref().unwrap().units[0].fx else { unreachable!() };
    let plugin = plugin.clone();
    let mut telemetry = Telemetry::default();
    with_editor_lock(&plugin, || {
        assert!(about(split_block(&mut e, 0.25).0.0, 0.25), "the dry mix passes the busy effect");
        assert!(heard_now(&mut e, &mut telemetry).is_empty(), "the busy effect did not render");
    });
    assert!(about(split_block(&mut e, 0.25).0.0, 0.25));
    assert_eq!(heard_now(&mut e, &mut telemetry), [master_heard(1), master_heard(2)]);
}

fn heard_now(e: &mut Engine, t: &mut Telemetry) -> Vec<Heard> {
    e.publish(t);
    t.heard.clone()
}

fn master_heard(bus: usize) -> Heard {
    Heard { owner: deckard_model::extra_out::Owner::Master(0), bus }
}

/// A bus feeding another runs first; buses feeding each other in a loop (which a
/// project can be made to do) still get an order, each worked out once.
#[test]
fn bus_order_runs_feeders_first_and_survives_loops() {
    let bus = |to: Option<usize>| Bus {
        chain: bare_chain(Vec::new(), 48_000.0),
        buf: Vec::new(),
        route: (to.map(|g| GROUP_BUS + g), (1.0, 1.0)),
        ramp: GainRamp::default(),
    };
    // 0 → 1 → 2, then a ring 3 → 4 → … → 40 → 3.
    let mut buses: Vec<Option<Bus>> = vec![Some(bus(Some(1))), Some(bus(Some(2))), Some(bus(None))];
    buses.extend((3..=40).map(|g| Some(bus(Some(if g == 40 { 3 } else { g + 1 })))));
    let order = bus_order(&buses);
    assert_eq!(order.len(), 41);
    let at = |g: usize| order.iter().position(|&x| x == g).unwrap();
    assert!(at(0) < at(1) && at(1) < at(2), "{order:?}");
}

/// Heard pairs sort as the mixer shows strips: a group's own slots, then its sounds
/// (instrument, then chain), the master's last.
#[test]
fn heard_pairs_sort_in_mixer_order() {
    use deckard_model::extra_out::Owner;
    let h = |owner, bus| Heard { owner, bus };
    let mut v = [
        h(Owner::Master(0), 1),
        h(Owner::Chain(1, 0, 0), 1),
        h(Owner::Sound(1, 0), 2),
        h(Owner::Group(1, 1), 1),
        h(Owner::Sound(0, 3), 1),
        h(Owner::Group(1, 0), 3),
    ];
    v.sort_by_key(heard_rank);
    let owners: Vec<Owner> = v.iter().map(|h| h.owner).collect();
    assert_eq!(
        owners,
        [Owner::Sound(0, 3), Owner::Group(1, 0), Owner::Group(1, 1), Owner::Sound(1, 0), Owner::Chain(1, 0, 0), Owner::Master(0)]
    );
}

/// Silence on a plug-in's extra pairs (just under -90 dBFS, too) isn't sound.
#[test]
fn a_silent_extra_bus_is_not_heard() {
    let mut e = split_engine(|_| {});
    let mut t = Telemetry::default();
    split_block(&mut e, 0.0);
    assert!(heard_now(&mut e, &mut t).is_empty());
    // A quarter of it on the third pair stays under the floor; half of it on the
    // second doesn't.
    split_block(&mut e, 1.2e-4);
    assert_eq!(heard_now(&mut e, &mut t), vec![master_heard(1)]);
}

/// Sound on two extra pairs: each is heard once, in order, however many blocks it
/// plays before the UI takes them; then not again until it plays again.
#[test]
fn sound_on_an_extra_bus_is_heard_once() {
    let mut e = split_engine(|_| {});
    let mut t = Telemetry::default();
    split_block(&mut e, 0.5);
    split_block(&mut e, 0.5);
    assert_eq!(heard_now(&mut e, &mut t), vec![master_heard(1), master_heard(2)]);
    split_block(&mut e, 0.5);
    assert_eq!(heard_now(&mut e, &mut t), vec![master_heard(1), master_heard(2)], "still once each");
    assert!(t.heard.capacity() >= HEARD_ROOM, "room made once: the audio thread keeps it");
    t.heard.clear();
    split_block(&mut e, 0.0);
    assert!(heard_now(&mut e, &mut t).is_empty(), "taken, and quiet since");
    // The main pair goes on as ever.
    let (master, _) = split_block(&mut e, 0.5);
    assert!(about(master.0, 0.5) && about(master.1, 0.5), "{master:?}");
}

/// A pair that has a strip already isn't listened to; one with a strip for another
/// plug-in (the slot's has changed) is.
#[test]
fn a_bus_with_a_strip_is_not_scanned() {
    use deckard_model::extra_out::{ExtraOut, Owner};
    let strip = |bus: usize, plugin: &str| ExtraOut {
        owner: Owner::Master(0),
        bus,
        plugin: plugin.into(),
        name: String::new(),
        level: 1.0,
        pan: 0.0,
        muted: false,
        dest: deckard_model::AudioDest::Parent,
        channels: Default::default(),
    };
    let mut e = split_engine(|p| p.extra_outs = vec![strip(1, &SPLIT.plugin_id()), strip(2, "another")]);
    assert_eq!(e.master_fx.as_ref().unwrap().units[0].scan, 1 << 2);
    let mut t = Telemetry::default();
    split_block(&mut e, 0.5);
    assert_eq!(heard_now(&mut e, &mut t), vec![master_heard(2)]);
    let mut p = project();
    p.extra_outs = vec![strip(1, &SPLIT.plugin_id()), strip(2, &SPLIT.plugin_id())];
    assert_eq!(split_unit(Owner::Master(0), &p).scan, 0);
    assert_eq!(split_unit(Owner::Master(1), &p).scan, 0b110, "another slot's strips");
}

/// A strip added or removed for a sound's slot (not one moved) makes its parts anew
/// on an edit, so they listen to the pairs left.
#[test]
fn a_new_strip_remakes_what_is_heard() {
    use deckard_model::extra_out::{ExtraOut, Owner};
    let a = project();
    let mut b = a.clone();
    let strip = ExtraOut {
        owner: Owner::Chain(0, 0, 1),
        bus: 1,
        plugin: "x".into(),
        name: String::new(),
        level: 1.0,
        pan: 0.0,
        muted: false,
        dest: deckard_model::AudioDest::Parent,
        channels: Default::default(),
    };
    b.extra_outs = vec![strip.clone()];
    let r = reuse_of(&a, &b, 0, 0, 1000.0, &deckard_plugins::Host::default());
    assert!(!r.chain && r.instrument);
    b.extra_outs[0].owner = Owner::Sound(0, 0);
    let r = reuse_of(&a, &b, 0, 0, 1000.0, &deckard_plugins::Host::default());
    assert!(r.chain && !r.instrument);
    let mut c = b.clone();
    c.extra_outs[0].level = 0.5;
    let r = reuse_of(&b, &c, 0, 0, 1000.0, &deckard_plugins::Host::default());
    assert!(r.chain && r.instrument, "a level is no new strip");
}

/// Slice Motion changed alone (the Sampler itself the same) makes the Sampler anew
/// on an edit: one kept would go on playing the Motion it was made with.
#[test]
fn a_new_motion_remakes_the_sampler() {
    use deckard_model::new_sampler::Motion;
    let (_tmp, a, _) = new_sampler_project(deckard_model::new_sampler::PlayMode::Repitch);
    let host = deckard_plugins::Host::default();
    assert!(reuse_of(&a, &a.clone(), 0, 0, 48_000.0, &host).sampler, "nothing changed: kept");
    let mut b = a.clone();
    b.groups[0].sounds[0].new_sampler.as_mut().unwrap().motion = Motion::Next;
    assert!(!reuse_of(&a, &b, 0, 0, 48_000.0, &host).sampler, "Motion on");
    assert!(!reuse_of(&b, &a, 0, 0, 48_000.0, &host).sampler, "and off again");
    // (New settings alone keep it, as before.)
    let mut c = b.clone();
    c.groups[0].sounds[0].sampler.as_mut().unwrap().tune += 1.0;
    let r = reuse_of(&b, &c, 0, 0, 48_000.0, &host);
    assert!(r.sampler && r.settings.is_some());
}

/// A bypassed plug-in makes no sound, so none is heard.
#[test]
fn a_bypassed_slot_is_not_heard() {
    let mut e = split_engine(|_| {});
    e.master_fx.as_mut().unwrap().units[0].mix = 0.0;
    let mut t = Telemetry::default();
    split_block(&mut e, 0.5);
    split_block(&mut e, 0.5);
    assert!(heard_now(&mut e, &mut t).is_empty());
}

/// An instrument's extra pair is heard as its sound's; its silent one isn't.
#[test]
fn an_instruments_extra_bus_is_heard() {
    use deckard_model::extra_out::Owner;
    let mut e = split_engine(|_| {});
    let p = project();
    let plugin = split_plugin(&SPLIT_INSTRUMENT);
    let mut inst = InstrumentRt::new(plugin.clone(), (1.0, 1.0), false);
    inst.listen(Owner::Sound(0, 0), &plugin, &SPLIT_INSTRUMENT.plugin_id(), &p);
    inst.heard = Some(e.heard_board.clone());
    e.instruments[0][0] = Some(inst);
    e.master_fx = None;
    let mut t = Telemetry::default();
    split_block(&mut e, 0.0);
    assert_eq!(heard_now(&mut e, &mut t), vec![Heard { owner: Owner::Sound(0, 0), bus: 2 }]);
}

/// A plug-in with one output pair has nothing to listen to, and nothing is heard.
#[test]
fn a_one_pair_plugin_reports_nothing() {
    use deckard_model::extra_out::Owner;
    let mut e = split_engine(|_| {});
    let class = deckard_plugins::native::CLASSES[0];
    let plugin = split_plugin(class);
    let p = project();
    let mut u = Unit::new(plugin_unit(plugin.clone(), &[], 1000.0), 0, Vec::new(), true);
    u.listen(Owner::Master(0), &plugin, &class.plugin_id(), &p);
    assert_eq!(u.scan, 0);
    e.master_fx.as_mut().unwrap().units[0] = u;
    let mut inst = InstrumentRt::new(plugin.clone(), (1.0, 1.0), false);
    inst.listen(Owner::Sound(0, 0), &plugin, &class.plugin_id(), &p);
    assert_eq!(inst.scan, 0);
    let mut t = Telemetry::default();
    split_block(&mut e, 0.5);
    assert!(heard_now(&mut e, &mut t).is_empty());
}

fn near_pair(a: (f32, f32), b: (f32, f32)) -> bool {
    about(a.0, b.0) && about(a.1, b.1)
}

/// A strip onto Out 3: its pair plays there (a quarter of the input: half of it at
/// unity), metered; the master has only the main pair.
#[test]
fn a_strip_on_ext_reaches_that_output() {
    let mut x = Strips::new(|_| {});
    let (master, out3) = x.block(0.5);
    assert!(near_pair(out3, (0.25, 0.25)), "Out 3: {out3:?}");
    assert!(near_pair(master, (0.5, 0.5)), "the master: {master:?}");
    let mut t = Telemetry::default();
    x.e.publish(&mut t);
    assert_eq!(t.extra_meters.len(), 1);
    assert!(near_pair(t.extra_meters[0], (0.25, 0.25)), "{:?}", t.extra_meters);
}

/// A strip's level and pan are its own: the owner's fader and the master's don't
/// move it, and changing them doesn't make the chain anew.
#[test]
fn a_strips_level_and_pan_are_its_own() {
    let mut x = Strips::new(|p| (p.extra_outs[0].level, p.extra_outs[0].pan) = (0.5, -1.0));
    let left = 0.25 * 0.5 * std::f32::consts::SQRT_2;
    assert!(near_pair(x.block(0.5).1, (left, 0.0)), "{:?}", x.block(0.5));
    let buf = x.e.fx[0][0].as_ref().unwrap().units[0].extras[0].buf.as_ptr();
    x.edit(|p| (p.groups[0].sounds[0].level, p.groups[0].level, p.master_level) = (0.3, 0.4, 0.2));
    x.block(0.5);
    let (master, out3) = x.block(0.5);
    assert!(near_pair(out3, (left, 0.0)), "the faders above don't move it: {out3:?}");
    assert!(master.0 < 0.1, "they move the main pair: {master:?}");
    x.edit(|p| (p.extra_outs[0].level, p.extra_outs[0].pan) = (1.0, 0.0));
    x.block(0.5);
    assert!(near_pair(x.block(0.5).1, (0.25, 0.25)), "its own do");
    assert_eq!(x.e.fx[0][0].as_ref().unwrap().units[0].extras[0].buf.as_ptr(), buf, "the chain was kept");
}

/// A strip into another group goes through that group's fader (and on to where it
/// goes: the master).
#[test]
fn a_strip_into_a_group_goes_through_that_group() {
    let mut x = Strips::new(|p| {
        p.groups.push(Group { level: 0.5, ..Default::default() });
        p.extra_outs[0].dest = deckard_model::AudioDest::Group(1);
    });
    let (master, out3) = x.block(0.5);
    let g = deckard_model::level_gain(0.5) as f32;
    assert!(near_pair(out3, (0.0, 0.0)));
    assert!(near_pair(master, (0.5 + 0.25 * g, 0.5 + 0.25 * g)), "{master:?}");
}

/// A strip into a sound goes through that sound's fader, then where it goes.
#[test]
fn a_strip_into_a_sound_goes_through_that_sound() {
    let mut x = Strips::new(|p| {
        p.groups[0].sounds.push(Sound { level: 0.5, ..Default::default() });
        p.extra_outs[0].dest = deckard_model::AudioDest::Sound(0, 1);
    });
    let (master, out3) = x.block(0.5);
    let s = deckard_model::level_gain(0.5) as f32;
    assert!(near_pair(out3, (0.0, 0.0)));
    assert!(near_pair(master, (0.5 + 0.25 * s, 0.5 + 0.25 * s)), "{master:?}");
}

/// A strip on an output the interface doesn't have connected plays through its
/// parent (here its group, onto the master), until the output is connected.
#[test]
fn an_unconnected_ext_falls_back_to_the_parent() {
    let mut x = Strips::new(|_| {});
    x.e.set_connected_outputs(0b1);
    let (master, out3) = x.block(0.5);
    assert!(near_pair(out3, (0.0, 0.0)), "{out3:?}");
    assert!(near_pair(master, (0.75, 0.75)), "{master:?}");
    x.e.set_connected_outputs(0b101);
    let (master, out3) = x.block(0.5);
    assert!(near_pair(out3, (0.25, 0.25)) && near_pair(master, (0.5, 0.5)), "{master:?} {out3:?}");
}

/// A muted sound, or a muted group, keeps its plug-in running for an audible strip:
/// only its main pair goes silent. The same for a group's own plug-in.
#[test]
fn a_muted_owner_keeps_its_plugin_running_for_its_strip() {
    use deckard_model::extra_out::Owner;
    for audio_mute in [false, true] {
        let mut x = Strips::new(|p| {
            p.groups[0].sounds[0].muted = true;
            p.groups[0].sounds[0].channel.audio_mute = audio_mute;
        });
        let (master, out3) = x.block(0.5);
        assert!(near_pair(master, (0.0, 0.0)) && near_pair(out3, (0.25, 0.25)), "sound (audio mute {audio_mute}): {master:?} {out3:?}");
    }
    let mut x = Strips::new(|p| p.groups[0].muted = true);
    let (master, out3) = x.block(0.5);
    assert!(near_pair(master, (0.0, 0.0)) && near_pair(out3, (0.25, 0.25)), "its group muted: {master:?} {out3:?}");
    // The group's own plug-in (its sounds go into it as ever).
    let mut x = Strips::new(|p| {
        p.groups[0].sounds[0].slots.clear();
        p.groups[0].slots = vec![plugin_slot(&SPLIT)];
        p.extra_outs = vec![split_strip(Owner::Group(0, 0), 1, deckard_model::AudioDest::Ext(2))];
        p.groups[0].muted = true;
    });
    let (master, out3) = x.block(0.5);
    assert!(near_pair(master, (0.0, 0.0)) && near_pair(out3, (0.25, 0.25)), "the group's plug-in: {master:?} {out3:?}");
    // Unmuted again: as before.
    x.edit(|p| p.groups[0].muted = false);
    x.block(0.5);
    let (master, out3) = x.block(0.5);
    assert!(near_pair(master, (0.5, 0.5)) && near_pair(out3, (0.25, 0.25)), "unmuted: {master:?} {out3:?}");
    // Without a strip that sounds, a muted owner is silent as ever.
    let mut x = Strips::new(|p| {
        p.groups[0].sounds[0].muted = true;
        p.groups[0].sounds[0].channel.audio_mute = true;
        p.extra_outs[0].muted = true;
    });
    assert!(x.e.in_routes.is_empty(), "not played");
    assert!(near_pair(x.block(0.5).0, (0.0, 0.0)));
}

/// [`split_block`] with `(l, r)` on In 1.
fn stereo_block(e: &mut Engine, l: f32, r: f32) -> ((f32, f32), Vec<(f32, f32)>) {
    let mut frame = [0f32; 32];
    (frame[0], frame[1]) = (l, r);
    e.live_in.as_ref().unwrap().lock().unwrap().extend(std::iter::repeat_n(frame, 64));
    let mut out = vec![0f32; 128];
    let mut ext = vec![vec![0f32; 128]; OUTPUTS];
    e.process_outputs(&mut out, &mut ext);
    ((out[126], out[127]), ext.iter().map(|b| (b[126], b[127])).collect())
}

/// A mono strip plays (L+R)/2 on both sides, which its pan places as a channel's
/// pan places a sound.
#[test]
fn a_mono_strip_sums_both_sides() {
    use deckard_model::extra_out::Channels;
    let mut x = Strips::new(|p| p.extra_outs[0].channels = Channels::Mono);
    // In 1 (0.8, 0.2): the pair is half of it, (0.4, 0.1); its sum halved, 0.25.
    let (master, ext) = stereo_block(&mut x.e, 0.8, 0.2);
    assert!(near_pair(ext[2], (0.25, 0.25)), "Out 3: {:?}", ext[2]);
    assert!(near_pair(master, (0.8, 0.2)), "the main pair as it is: {master:?}");
    x.edit(|p| p.extra_outs[0].pan = -1.0);
    stereo_block(&mut x.e, 0.8, 0.2);
    let (_, ext) = stereo_block(&mut x.e, 0.8, 0.2);
    assert!(near_pair(ext[2], (0.25 * std::f32::consts::SQRT_2, 0.0)), "panned left: {:?}", ext[2]);
    // Stereo again: each side its own.
    x.edit(|p| (p.extra_outs[0].pan, p.extra_outs[0].channels) = (0.0, Channels::Stereo));
    stereo_block(&mut x.e, 0.8, 0.2);
    let (_, ext) = stereo_block(&mut x.e, 0.8, 0.2);
    assert!(near_pair(ext[2], (0.4, 0.1)), "stereo: {:?}", ext[2]);
}

/// The halves of a split strip each play one side of the pair on both sides, with
/// their own level and output, and meter apart; split while playing, too.
#[test]
fn split_strip_halves_carry_one_side_each() {
    use deckard_model::extra_out::{self, Channels};
    let mut x = Strips::new(|_| {});
    let (_, ext) = stereo_block(&mut x.e, 0.8, 0.2);
    assert!(near_pair(ext[2], (0.4, 0.1)), "stereo first: {:?}", ext[2]);
    x.edit(|p| {
        extra_out::split(&mut p.extra_outs, 0);
        p.extra_outs[1].level = 0.5;
        p.extra_outs[1].dest = deckard_model::AudioDest::Ext(3);
    });
    assert_eq!(x.p.extra_outs.iter().map(|e| e.channels).collect::<Vec<_>>(), vec![Channels::Left, Channels::Right]);
    stereo_block(&mut x.e, 0.8, 0.2);
    let (master, ext) = stereo_block(&mut x.e, 0.8, 0.2);
    assert!(near_pair(ext[2], (0.4, 0.4)), "L on Out 3: {:?}", ext[2]);
    assert!(near_pair(ext[3], (0.05, 0.05)), "R at half on Out 4: {:?}", ext[3]);
    assert!(near_pair(master, (0.8, 0.2)), "the main pair: {master:?}");
    let mut t = Telemetry::default();
    x.e.publish(&mut t);
    assert_eq!(t.extra_meters.len(), 2);
    assert!(near_pair(t.extra_meters[0], (0.4, 0.4)) && near_pair(t.extra_meters[1], (0.05, 0.05)), "{:?}", t.extra_meters);
    // Joined: stereo again, with the left half's settings.
    x.edit(|p| extra_out::join(&mut p.extra_outs, 1));
    stereo_block(&mut x.e, 0.8, 0.2);
    let (_, ext) = stereo_block(&mut x.e, 0.8, 0.2);
    assert!(near_pair(ext[2], (0.4, 0.1)) && near_pair(ext[3], (0.0, 0.0)), "{:?} {:?}", ext[2], ext[3]);
}

/// A muted strip is silent; the main pair plays on.
#[test]
fn a_muted_strip_is_silent() {
    let mut x = Strips::new(|p| p.extra_outs[0].muted = true);
    let (master, out3) = x.block(0.5);
    assert!(near_pair(out3, (0.0, 0.0)) && near_pair(master, (0.5, 0.5)), "{master:?} {out3:?}");
}

/// Taking the plug-in away (its strip with it, or left behind) while playing: no
/// panic, nothing more on its output, and the meters follow.
#[test]
fn removing_the_plugin_drops_its_strip_without_panic() {
    for keep_strip in [false, true] {
        let mut x = Strips::new(|_| {});
        x.block(0.5);
        x.edit(|p| {
            p.groups[0].sounds[0].slots.clear();
            if !keep_strip {
                p.extra_outs.clear();
            }
        });
        x.block(0.5);
        let (master, out3) = x.block(0.5);
        assert!(near_pair(out3, (0.0, 0.0)) && near_pair(master, (0.5, 0.5)), "{master:?} {out3:?}");
        let mut t = Telemetry::default();
        x.e.publish(&mut t);
        assert_eq!(t.extra_meters.len(), x.p.extra_outs.len());
        assert!(t.extra_meters.iter().all(|m| *m == (0.0, 0.0)), "{:?}", t.extra_meters);
    }
}

/// A plug-in with one output pair plays exactly as it did, with or without a strip
/// claiming a pair it doesn't have.
#[test]
fn a_one_pair_plugin_sounds_unchanged() {
    use deckard_model::extra_out::Owner;
    let class = deckard_plugins::native::CLASSES[0];
    let render = |strip: bool| {
        let mut p = project();
        let snd = &mut p.groups[0].sounds[0];
        snd.sampler = None;
        snd.plugin = plugin_slot(class).plugin;
        snd.slots = vec![plugin_slot(class)];
        if strip {
            let mut x = split_strip(Owner::Sound(0, 0), 1, deckard_model::AudioDest::Ext(2));
            x.plugin = class.plugin_id();
            p.extra_outs = vec![x];
        }
        let mut cache = SampleCache::default();
        cache.test_plugins.push((Slot::Sound { group: 0, sound: 0 }, split_plugin(class)));
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        let (mut all, mut others) = (Vec::new(), 0f32);
        for _ in 0..40 {
            let mut out = vec![0f32; 512];
            let mut ext = vec![vec![0f32; 512]; OUTPUTS];
            e.process_outputs(&mut out, &mut ext);
            all.extend_from_slice(&out);
            others = ext.iter().flatten().fold(others, |m, v| m.max(v.abs()));
        }
        (all, others)
    };
    let (plain, _) = render(false);
    let (with, others) = render(true);
    assert!(plain.iter().any(|v| v.abs() > 1e-3), "it plays");
    assert_eq!(plain, with);
    assert_eq!(others, 0.0);
}

static PRESSED: deckard_plugins::native::Class = deckard_plugins::native::Class {
    cid: [9; 16],
    name: "Pressed",
    instrument: true,
    params: &[],
    pages: &[],
    make: |_| Box::new(Pressed::default()),
    scope: false,
};

/// Channel pressure reaches a plug-in instrument (the woosh's Z on Pigments), as
/// controllers and pitch bend do; it used to stop at the engine.
#[test]
fn channel_pressure_reaches_a_plugin_instrument() {
    let mut p = project();
    p.groups[0].patterns[0].notes.clear();
    let snd = &mut p.groups[0].sounds[0];
    snd.sampler = None;
    snd.plugin = plugin_slot(&PRESSED).plugin;
    snd.slots = vec![plugin_slot(&PRESSED)];
    let mut cache = SampleCache::default();
    cache.test_plugins.push((Slot::Sound { group: 0, sound: 0 }, split_plugin(&PRESSED)));
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    e.sequencing = false;
    let mut level = |e: &mut Engine| {
        let mut out = vec![0f32; 512];
        for _ in 0..8 {
            e.process(&mut out);
        }
        out[510]
    };
    assert_eq!(level(&mut e), 0.0, "nothing pressed");
    e.control_in(0, 0, deckard_model::MidiKind::ChannelPressure(127), false);
    let full = level(&mut e);
    assert!(full > 0.1, "pressed: {full}");
    e.control_in(0, 0, deckard_model::MidiKind::ChannelPressure(0), false);
    assert!(level(&mut e).abs() < 1e-6, "let go");
}
