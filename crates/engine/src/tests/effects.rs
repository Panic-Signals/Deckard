use super::*;

/// A group's Perform FX: engaged (Reso Echo, the strip low) it's heard; not engaged,
/// the group sounds as without it. (Reso Echo: released with nothing to ring out,
/// Maschine's passes the input through exactly. Its Filter doesn't: it always runs
/// through its oversampling, 16 samples late; docs/fx-perform-filter.md.)
#[test]
fn a_groups_perform_fx_plays_when_engaged() {
    let (_tmp, p) = eight_project();
    let slot = |engaged: f64| deckard_model::SlotInfo {
        module: deckard_model::PERFORM_FX.0.into(),
        name: "Perform FX".into(),
        active: true,
        plugin: None,
        params: deckard_model::PERFORM_FX
            .1
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    match *k {
                        "touchstrip_on_off" => engaged,
                        "touchstrip_pos" => 0.2,
                        "mode" => 3.0,
                        _ => *v,
                    },
                )
            })
            .collect(),
    };
    let with = |engaged: f64| {
        let mut q = p.clone();
        q.groups[0].slots.insert(0, slot(engaged));
        render_with(&q, None, 256, 1.0, |_, _, _| {})
    };
    let plain = render_with(&p, None, 256, 1.0, |_, _, _| {});
    assert!(max_diff(&plain, &with(0.0)) < 1e-6, "disengaged: as without it");
    assert!(max_diff(&plain, &with(1.0)) > 0.05, "engaged: heard");
}

/// The master's effect runs on the whole mix, before the master's level.
#[test]
fn master_effects_play() {
    let (_tmp, p) = eight_project();
    let mut fx = p.clone();
    fx.master_slots = vec![driven_saturator()];
    let plain = render_with(&p, None, 256, 1.0, |_, _, _| {});
    let driven = render_with(&fx, None, 256, 1.0, |_, _, _| {});
    assert!(max_diff(&plain, &driven) > 0.05, "the master's saturator is heard");
    let mut silent = fx.clone();
    silent.master_level = 0.0;
    assert!(render_with(&silent, None, 256, 0.5, |_, _, _| {}).iter().all(|x| x.abs() < 1e-6));
}

/// An edit that takes sounds or groups away while sounds render ahead (an undo of
/// loading a group, say), and one that brings them back: the players of sounds no
/// longer there are let go, and it plays on.
#[test]
fn sounds_taken_away_while_rendering_ahead() {
    let (_tmp, p) = eight_project();
    let mut fewer = p.clone();
    fewer.groups[0].sounds.truncate(4);
    for pat in &mut fewer.groups[0].patterns {
        pat.notes.retain(|n| n.sound < 4);
    }
    let mut more = p.clone();
    more.groups.push(p.groups[0].clone());
    let (fewer, more, back) = (fewer, more, p.clone());
    let out = render_with(&p, Some(700), 256, 2.0, move |k, e, cache| match k {
        40 => e.update(&more, None, cache),
        80 => e.update(&fewer, None, cache),
        120 => e.update(&back, None, cache),
        _ => {}
    });
    assert!(out.iter().all(|x| x.is_finite()));
    assert!(out[130 * 512..].iter().any(|x| x.abs() > 1e-4), "it plays on");
}

/// Edits built incrementally (only what changed, the rest kept) sound exactly as
/// edits built in full: a level, a Sampler knob, an effect knob, a bypass, a note
/// added, a mute and unmute; with sounds rendered here and ahead.
#[test]
fn incremental_edits_sound_like_full_rebuilds() {
    let (_tmp, p0) = eight_project();
    let mut steps: Vec<(usize, Project)> = Vec::new();
    let mut p = p0.clone();
    p.groups[0].sounds[3].level = 0.3;
    steps.push((40, p.clone()));
    p.groups[0].sounds[2].sampler.as_mut().unwrap().set("tune", 7.0);
    steps.push((60, p.clone()));
    for (k, v) in p.groups[0].sounds[5].slots[1].params.iter_mut() {
        if k.contains("drive") {
            *v = 0.8;
        }
    }
    steps.push((80, p.clone()));
    p.groups[0].sounds[1].slots[1].active = false;
    steps.push((100, p.clone()));
    p.groups[0].patterns[0].notes.push(Note {
        ratchet: Default::default(),
        expr: None,
        sound: 6,
        position: 1000,
        length: 400,
        pitch: 55,
        velocity: 120,
        probability: 100,
    });
    steps.push((120, p.clone()));
    p.groups[0].sounds[4].muted = true;
    p.groups[0].sounds[4].channel.audio_mute = true;
    steps.push((140, p.clone()));
    p.groups[0].sounds[4].muted = false;
    p.groups[0].sounds[6].pan = -0.5;
    steps.push((160, p.clone()));
    let chains0 = Engine::new(&p0, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
    let render = |incremental: bool, ahead: Option<u64>| {
        let (mut prev, mut chains) = (p0.clone(), chains0.clone());
        render_with(&p0, ahead, 256, 4.0, |k, e, cache| {
            let Some((_, to)) = steps.iter().find(|(at, _)| *at == k) else { return };
            let (fresh, report) = Engine::new_after(to, incremental.then_some((&prev, chains.as_slice())), None, 48_000, e.mode(), cache);
            if incremental && k == 40 {
                // A level: nothing built (the timeline aside).
                assert!(
                    fresh.sounds[0].iter().all(Option::is_none) && fresh.fx[0].iter().all(Option::is_none),
                    "only what changed is built"
                );
            }
            drop(e.adopt(fresh, to));
            (prev, chains) = (to.clone(), report.chains);
        })
    };
    for ahead in [None, Some(700)] {
        let (full, incr) = (render(false, ahead), render(true, ahead));
        assert!(full.iter().any(|x| x.abs() > 0.05), "it plays");
        let first = full.iter().zip(&incr).position(|(a, b)| a != b);
        assert_eq!(
            first,
            None,
            "ahead {ahead:?}: first difference at frame {:?} (block {:?})",
            first.map(|i| i / 2),
            first.map(|i| i / 512)
        );
    }
}

/// Stop reaches sounds rendered ahead: they fade within a lookahead (and the
/// short fade), as they do rendered here.
#[test]
fn stop_reaches_sounds_rendered_ahead() {
    let (_tmp, p) = eight_project();
    let tail = |ahead: Option<u64>| {
        let out = render_with(&p, ahead, 256, 1.0, |k, e, _| {
            if k == 80 {
                e.stop();
            }
        });
        // Last loud frame after the stop.
        out.chunks(2).rposition(|f| f[0].abs() > 1e-3).unwrap_or(0) as i64 - 80 * 256
    };
    // (Effects ring on after a stop: the delay's echoes last a while.)
    let (inline, ahead) = (tail(None), tail(Some(700)));
    assert!(inline > 0, "it played");
    assert!(ahead <= inline + 700 + 256, "ahead: within a lookahead more ({ahead} vs {inline} frames)");
}

/// A live note for a sound rendered ahead plays within a lookahead.
#[test]
fn live_notes_reach_sounds_rendered_ahead() {
    let (_tmp, mut p) = eight_project();
    p.groups[0].patterns[0].notes.clear();
    let onset = |ahead: Option<u64>| {
        let out = render_with(&p, ahead, 256, 0.5, |k, e, _| {
            e.sequencing = false;
            if k == 10 {
                e.note_on(0, 2, 60, 127, PPQ);
            }
        });
        out.chunks(2).position(|f| f[0].abs() > 1e-3)
    };
    let (now, later) = (onset(None).unwrap(), onset(Some(700)).unwrap());
    assert_eq!(now, 10 * 256, "inline: at once");
    assert_eq!(later, now + 700 + 256, "ahead: exactly a lookahead and a block later");
}

/// Rendering ahead plays what the audio thread would have: the same notes at the
/// same frames, through the same effects.
#[test]
fn rendering_ahead_sounds_the_same() {
    let inline = render_eight(None, 256, 3.0);
    assert!(inline.iter().any(|x| x.abs() > 0.05), "it plays");
    for (lookahead, block) in [(512u64, 256usize), (2048, 128), (300, 512)] {
        let ahead = render_eight(Some(lookahead), block, 3.0);
        let inline = render_eight(None, block, 3.0);
        let err = inline.iter().zip(&ahead).fold(0f32, |m, (a, b)| m.max((a - b).abs()));
        let peak = inline.iter().fold(0f32, |m, a| m.max(a.abs()));
        eprintln!("lookahead {lookahead}, block {block}: max difference {err:e} (peak {peak})");
        assert!(err <= peak * 1e-4, "lookahead {lookahead}, block {block}: differs by {err} (peak {peak})");
    }
}

/// Sounds rendered on worker threads give exactly what one thread gives: eight
/// sounds, half through effects (with state: delay, chorus, limiter), notes at
/// different times.
#[test]
fn worker_threads_render_the_same() {
    let mut p = project();
    let base = p.groups[0].sounds[0].clone();
    let fx = |module: &str, params: Vec<(String, f64)>| deckard_model::SlotInfo {
        module: module.into(),
        name: module.into(),
        active: true,
        plugin: None,
        params,
    };
    let defaults = |class: &str| -> Vec<(String, f64)> {
        let (_, d) = deckard_model::INTERNAL_EFFECTS.iter().find(|(c, _)| *c == class).unwrap();
        d.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    };
    let chains = [
        vec![fx("BeatDelay", defaults("BeatDelay"))],
        vec![fx("Chorus", defaults("Chorus"))],
        vec![fx("Limiter", vec![("mode".into(), 1.0), ("threshold".into(), -20.0), ("release".into(), 0.0), ("ceiling".into(), 0.0)])],
    ];
    p.groups[0].sounds = (0..8)
        .map(|k| {
            let mut s = base.clone();
            s.slots = vec![fx("Sampler", Vec::new())];
            if k % 2 == 1 {
                s.slots.extend(chains[k / 2 % chains.len()].clone());
            }
            s
        })
        .collect();
    p.groups[0].patterns[0].notes = (0..32)
        .map(|i| Note {
            ratchet: Default::default(),
            expr: None,
            sound: i % 8,
            position: i as i64 * 37,
            length: 3600,
            pitch: 48 + (i as i64 * 5) % 24,
            velocity: 100,
            probability: 100,
        })
        .collect();
    let render_with = |pool: Option<Arc<pool::RenderPool>>| {
        let mut e = click_engine_for(&p, 48_000);
        let noise: Vec<[f32; 2]> = (0..96_000u32)
            .map(|i| {
                let x = (i.wrapping_mul(2_654_435_761) >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                [x, -x * 0.7]
            })
            .collect();
        set_sample(&mut e, noise, &[], false);
        e.pool = pool;
        let mut out = Vec::new();
        // Large blocks: jobs long enough for sleeping workers to wake and join in.
        let mut block = vec![0f32; 2 * 2048];
        for _ in 0..(48_000 * 2 / 2048) {
            e.process(&mut block);
            out.extend_from_slice(&block);
        }
        out
    };
    let one = render_with(None);
    assert!(one.iter().any(|x| x.abs() > 0.01), "it plays");
    let pool = Arc::new(pool::RenderPool::new(3, 256, 48_000));
    // (On a busy machine the workers may not wake in time for a while: try more.)
    for k in 0..10 {
        let many = render_with(Some(pool.clone()));
        assert!(one.iter().zip(&many).all(|(a, b)| a.to_bits() == b.to_bits()), "bit for bit");
        if k >= 2 && pool.helped() > 0 {
            break;
        }
    }
    assert!(pool.helped() > 0, "the workers rendered sounds");
}

/// Turning a knob while playing: the running effect keeps its state and takes the
/// new setting (a limiter's ceiling pulled down holds the next click down).
#[test]
fn effect_settings_change_while_playing() {
    let mut p = project();
    let lim = |ceiling: f64| deckard_model::SlotInfo {
        module: "Limiter".into(),
        name: "Limiter".into(),
        active: true,
        plugin: None,
        params: vec![("mode".into(), 1.0), ("threshold".into(), -30.0), ("release".into(), 0.0), ("ceiling".into(), ceiling)],
    };
    let sampler =
        deckard_model::SlotInfo { module: "Sampler".into(), name: "Sampler".into(), active: true, plugin: None, params: Vec::new() };
    p.groups[0].sounds[0].slots = vec![sampler, lim(1.0)];
    let mut e = click_engine_for(&p, 48_000);
    e.sequencing = false;
    let peak = |e: &mut Engine| {
        e.note_on(0, 0, 60, 127, 960);
        let mut out = vec![0f32; 2 * 4800];
        e.process(&mut out);
        out.iter().fold(0f32, |m, v| m.max(v.abs()))
    };
    let open = peak(&mut e);
    p.groups[0].sounds[0].slots[1] = lim(0.0);
    let samples = e.samples.clone();
    let sound = e.sounds[0][0].take();
    e.update(&p, None, &mut Default::default());
    e.samples = samples; // the test sample (Engine::new found no files)
    e.sounds[0][0] = sound;
    // The new ceiling glides in (as in Maschine) before the next click.
    e.process(&mut vec![0f32; 2 * 4800]);
    let held = peak(&mut e);
    assert!(open > 0.5, "the click passes at a high ceiling: {open}");
    assert!(held <= 10f32.powf(-40.05 / 20.0) * 1.01, "the new ceiling holds it down: {held}");
}

#[test]
fn internal_eq_spectrum_captures_the_selected_slot_before_and_after_processing() {
    let board = Arc::new(eq_spectrum::EqSpectrum::default());
    let transport = Transport { tempo: 120.0, playing: false, quarters: 0.0, numerator: 4, denominator: 4 };
    let rate = 48_000.0;
    let params = vec![("output_gain".into(), -0.5)];
    let frames = deckard_plugins::spectrum::FRAMES;
    for (route, owner) in [
        (SpectrumChain::Sound(1, 2), Owner::Chain(1, 2, 3)),
        (SpectrumChain::Group(1), Owner::Group(1, 3)),
        (SpectrumChain::Master, Owner::Master(3)),
    ] {
        let fx = FxUnit::Internal {
            class: "EQ".into(),
            params: params.clone(),
            fx: fx::make("EQ", &params, rate).unwrap(),
            glide: glide::Glide::new(&params, rate),
            modded: params.clone(),
            hit: false,
            modulated: false,
        };
        let mut chain = bare_chain(vec![Unit::new(fx, 3, Vec::new(), true)], rate);
        chain.spectrum = Some((board.clone(), route));
        let mut run = || {
            for offset in (0..frames).step_by(128) {
                for i in 0..128 {
                    let x = (std::f32::consts::TAU * (offset + i) as f32 / 64.0).sin() * 0.5;
                    chain.l[i] = x;
                    chain.r[i] = -x;
                }
                chain.run(128, &transport, &[], 0);
            }
        };
        let mut other = board.watch(Owner::Master(7));
        run();
        assert!(other.receive().is_none());
        let mut watched = board.watch(owner);
        run();
        let window = watched.receive().expect("the selected slot's window");
        let peak = |channel| window.iter().map(|f| f[channel]).fold(0.0f32, f32::max);
        assert!((peak(0) - 0.5).abs() < 0.001);
        assert!((peak(1) - 0.5).abs() < 0.001);
        assert!(peak(2) > 0.0 && peak(2) < 0.45, "the EQ attenuates the captured signal");
        assert!((peak(2) - peak(3)).abs() < 0.001);
    }
}

/// An effect that keeps the last value of its first setting (as its output).
struct LastSet(Arc<std::sync::Mutex<Vec<f64>>>);
impl fx::Effect for LastSet {
    fn set(&mut self, params: &[(String, f64)]) {
        self.0.lock().unwrap().push(params[0].1);
    }
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let v = *self.0.lock().unwrap().last().unwrap() as f32;
        l.fill(v);
        r.fill(v);
    }
}

/// A knob turned on an effect glides: the effect is taken there evenly over
/// glide::GLIDE_MS, a step every glide::STEP frames; a choice changes at once.
#[test]
fn effect_settings_glide() {
    let sr = 48_000.0;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let params = |v: f64, mode: f64| vec![("cutoff".to_string(), v), ("mode".to_string(), mode)];
    let mut fx: Box<dyn fx::Effect> = Box::new(LastSet(seen.clone()));
    fx.set(&params(0.2, 0.0));
    let unit = FxUnit::Internal {
        class: "Test".into(),
        params: params(0.2, 0.0),
        fx,
        glide: glide::Glide::new(&params(0.2, 0.0), sr),
        modded: params(0.2, 0.0),
        hit: false,
        modulated: false,
    };
    let mut chain = FxChain {
        units: vec![Unit::new(unit, 1, Vec::new(), true)],
        gain: (1.0, 1.0),
        l: vec![0.0; MAX_BLOCK],
        r: vec![0.0; MAX_BLOCK],
        dl: vec![0.0; MAX_BLOCK],
        dr: vec![0.0; MAX_BLOCK],
        fade: 0,
        meters: None,
        spectrum: None,
        tap: Vec::new(),
        sl: vec![0.0; MAX_BLOCK],
        sr: vec![0.0; MAX_BLOCK],
        heard: None,
    };
    let transport = Transport { tempo: 120.0, playing: false, quarters: 0.0, numerator: 4, denominator: 4 };
    chain.set_params(&mut [(1, params(0.8, 2.0))]);
    let mut out = Vec::new();
    let n = 256.min(MAX_BLOCK);
    for _ in 0..(sr as usize / 20 / n) {
        chain.run(n, &transport, &[], 0);
        out.extend_from_slice(&chain.l[..n]);
    }
    let at = |ms: usize| out[ms * 48] as f64;
    assert!((at(0) - 0.2).abs() < 0.05, "starts where it was: {}", at(0));
    assert!(at(10) > 0.3 && at(10) < 0.6, "on its way at 10 ms: {}", at(10));
    assert!((at(35) - 0.8).abs() < 1e-6, "there by 35 ms: {}", at(35));
    let steps = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0f32, f32::max);
    assert!(steps < 0.6 / 20.0, "in small steps: {steps}");
    assert!(seen.lock().unwrap().len() > 20, "set as it goes");
}

/// A Filter's cutoff turned down while a bright tone plays takes it out: the edit
/// reaches the running effect (and glides there).
#[test]
fn filter_cutoff_turned_down_while_playing() {
    let filter = |cutoff: f64| deckard_model::SlotInfo {
        module: "Filter".into(),
        name: "Filter".into(),
        active: true,
        plugin: None,
        params: vec![("mode".into(), 0.0), ("cutoff".into(), cutoff), ("resonance".into(), 0.0), ("amount".into(), 0.0)],
    };
    let sampler =
        deckard_model::SlotInfo { module: "Sampler".into(), name: "Sampler".into(), active: true, plugin: None, params: Vec::new() };
    let mut p = project();
    p.groups[0].sounds[0].slots = vec![sampler, filter(1.0)];
    let mut e = click_engine_for(&p, 48_000);
    e.sequencing = false;
    // A bright tone: 6 kHz.
    let tone: Vec<[f32; 2]> = (0..48_000).map(|i| [0.5 * (i as f32 * std::f32::consts::TAU / 8.0).sin(); 2]).collect();
    set_sample(&mut e, tone, &[], true);
    e.note_hold(0, 0, 60, 127);
    let rms = |e: &mut Engine| {
        let mut out = vec![0f32; 2 * 4800];
        e.process(&mut out);
        (out.iter().skip(4800).map(|x| x * x).sum::<f32>() / 4800.0).sqrt()
    };
    rms(&mut e);
    let open = rms(&mut e);
    p.groups[0].sounds[0].slots[1] = filter(0.0);
    let samples = e.samples.clone();
    let sound = e.sounds[0][0].take();
    e.update(&p, None, &mut Default::default());
    e.samples = samples; // the test sample (Engine::new found no files)
    e.sounds[0][0] = sound;
    rms(&mut e);
    let closed = rms(&mut e);
    assert!(open > 0.05, "the tone plays: {open}");
    assert!(closed < open * 0.1, "the cutoff turned down takes it out: {open} -> {closed}");
}

/// Play a steady tone through sound 0 of `before`, then take up `after` while it
/// sounds; the left channel from just before the edit to 50 ms after it.
fn edit_while_playing(before: &Project, after: &Project) -> Vec<f32> {
    let mut e = click_engine_for(before, 48_000);
    e.sequencing = false;
    set_sample(&mut e, vec![[0.5, 0.5]; 48_000], &[], true);
    e.note_hold(0, 0, 60, 127);
    // Let effects settle.
    let mut block = vec![0f32; 2 * 4800];
    for _ in 0..4 {
        e.process(&mut block);
    }
    let mut out = block[block.len() - 2 * 256..].iter().step_by(2).copied().collect::<Vec<f32>>();
    let samples = e.samples.clone();
    let sound = e.sounds[0][0].take();
    e.update(after, None, &mut Default::default());
    e.samples = samples; // the test sample (Engine::new found no files)
    e.sounds[0][0] = sound;
    let mut after_edit = vec![0f32; 2 * 2400];
    e.process(&mut after_edit);
    out.extend(after_edit.iter().step_by(2));
    out
}

/// The largest step between neighbouring samples (a jump is a click).
fn largest_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

/// Level, pan and mute glide: turning a sound down while it holds a steady tone
/// takes ~10 ms, with no step.
#[test]
fn level_changes_glide() {
    let before = project();
    let mut after = before.clone();
    after.groups[0].sounds[0].level = 0.25;
    let x = edit_while_playing(&before, &after);
    let (from, to) = (x[255], *x.last().unwrap());
    assert!(from > 0.1 && to < from * 0.5, "it was turned down: {from} -> {to}");
    assert!((x[256] - from).abs() < (from - to) * 0.01, "no jump at the edit: {} after {from}", x[256]);
    assert!((x[256 + 480] - to).abs() < 1e-6, "there after 10 ms");
    assert!(largest_step(&x) < (from - to) / 300.0, "no step: {}", largest_step(&x));
}

/// Bypassing an effect while it plays crossfades (20 ms) instead of switching, and
/// switching it back on resumes it with its state (no fresh start, no overshoot).
#[test]
fn bypass_crossfades_and_keeps_state() {
    let slot = |module: &str, active: bool, params: Vec<(String, f64)>| deckard_model::SlotInfo {
        module: module.into(),
        name: module.into(),
        active,
        plugin: None,
        params,
    };
    let lim = vec![("mode".into(), 1.0), ("threshold".into(), -20.0), ("release".into(), 0.5), ("ceiling".into(), 0.0)];
    let mut on = project();
    on.groups[0].sounds[0].slots = vec![slot("Sampler", true, Vec::new()), slot("Limiter", true, lim.clone())];
    let mut off = on.clone();
    off.groups[0].sounds[0].slots[1].active = false;
    let x = edit_while_playing(&on, &off);
    let (wet, dry) = (x[255], *x.last().unwrap());
    assert!(dry > wet * 2.0, "the limiter was holding it down: {wet} vs {dry}");
    assert!((x[256] - wet).abs() < (dry - wet) * 0.01, "no jump at the edit");
    assert!(largest_step(&x) < (dry - wet) / 300.0, "a crossfade, not a switch: {}", largest_step(&x));
    // Back on: the limiter picks up where it was (already holding the level).
    let y = edit_while_playing(&off, &on);
    let back = *y.last().unwrap();
    assert!(largest_step(&y) < (dry - wet) / 300.0, "back on without a step: {}", largest_step(&y));
    assert!(y.iter().all(|v| *v <= dry * 1.001), "no overshoot");
    assert!(back < dry * 0.9, "the limiter is back: {back}");
}

/// Removing an effect fades it out where it was, and the effects after it keep
/// their state (they are paired by order, not by slot number).
#[test]
fn removing_an_effect_fades_it_out() {
    let slot = |module: &str, params: Vec<(String, f64)>| deckard_model::SlotInfo {
        module: module.into(),
        name: module.into(),
        active: true,
        plugin: None,
        params,
    };
    let lim = vec![("mode".into(), 1.0), ("threshold".into(), -20.0), ("release".into(), 0.5), ("ceiling".into(), 0.0)];
    let sat = deckard_model::INTERNAL_EFFECTS
        .iter()
        .find(|(c, _)| *c == "Saturator")
        .unwrap()
        .1
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
    let mut both = project();
    both.groups[0].sounds[0].slots = vec![slot("Sampler", Vec::new()), slot("Saturator", sat), slot("Limiter", lim)];
    let mut one = both.clone();
    one.groups[0].sounds[0].slots.remove(1);
    let x = edit_while_playing(&both, &one);
    let swing = x.iter().fold(0f32, |m, v| m.max(*v)) - x.iter().fold(f32::MAX, |m, v| m.min(*v));
    assert!(largest_step(&x) <= swing / 100.0 + 1e-6, "no click: step {} of {swing}", largest_step(&x));
}

/// Side-chaining: sound 1 (routed to None, so unheard) drives a limiter on sound 0.
/// When it plays, sound 0 comes out held down; when it doesn't, sound 0 passes.
#[test]
fn sidechain_ducks_a_sound() {
    let mut p = project();
    let click = p.groups[0].sounds[0].clone();
    p.groups[0].sounds.push(click);
    let slot = |module: &str, params: Vec<(String, f64)>| deckard_model::SlotInfo {
        module: module.into(),
        name: module.into(),
        active: true,
        plugin: None,
        params,
    };
    // Transparent limiter, no drive on the audio (side-chained), 30 dB on the
    // detector, the ceiling at its top.
    let lim = vec![("mode".into(), 1.0), ("threshold".into(), -30.0), ("release".into(), 0.5), ("ceiling".into(), 1.0)];
    p.groups[0].sounds[0].slots = vec![slot("Sampler", Vec::new()), slot("Limiter", lim)];
    p.groups[0].sounds[0].channel.sidechain = vec![(1, deckard_model::ScSource::Sound(0, 1))];
    p.groups[0].sounds[1].channel.dest = deckard_model::AudioDest::None;
    let peak = |with_source: bool| {
        let mut e = click_engine_for(&p, 48_000);
        e.sequencing = false;
        e.note_on(0, 0, 60, 127, 960);
        if with_source {
            e.note_on(0, 1, 60, 127, 960);
        }
        let mut out = vec![0f32; 2 * 4800];
        e.process(&mut out);
        out.iter().fold(0f32, |m, v| m.max(v.abs()))
    };
    let (alone, ducked) = (peak(false), peak(true));
    assert!(alone > 0.5, "sound 0 alone passes: {alone}");
    assert!(ducked < alone * 0.1, "held down by sound 1's side-chain: {ducked} vs {alone}");
}

/// Group routing: a Limiter on group 0 side-chained from sound 1 (routed to None)
/// ducks group 0's output when sound 1 plays.
#[test]
fn sidechain_ducks_a_group() {
    let mut p = project();
    let click = p.groups[0].sounds[0].clone();
    p.groups[0].sounds.push(click);
    let lim = vec![("mode".into(), 1.0), ("threshold".into(), -30.0), ("release".into(), 0.5), ("ceiling".into(), 1.0)];
    p.groups[0].slots =
        vec![deckard_model::SlotInfo { module: "Limiter".into(), name: "Limiter".into(), active: true, plugin: None, params: lim }];
    p.groups[0].channel.sidechain = vec![(0, deckard_model::ScSource::Sound(0, 1))];
    p.groups[0].sounds[1].channel.dest = deckard_model::AudioDest::None;
    let peak = |with_source: bool| {
        let mut e = click_engine_for(&p, 48_000);
        e.sequencing = false;
        e.note_on(0, 0, 60, 127, 960);
        if with_source {
            e.note_on(0, 1, 60, 127, 960);
        }
        let mut out = vec![0f32; 2 * 4800];
        e.process(&mut out);
        out.iter().fold(0f32, |m, v| m.max(v.abs()))
    };
    let (alone, ducked) = (peak(false), peak(true));
    assert!(alone > 0.5, "group 0 alone passes: {alone}");
    assert!(ducked < alone * 0.1, "group 0 held down by sound 1's side-chain: {ducked} vs {alone}");
}

/// Plug-in side-chain routing: a plug-in effect with a side-chain input receives the
/// routed side-chain audio (filtered through its `SidechainFilter`) on both Sound and
/// Group chains.
#[test]
fn sidechain_ducks_a_plugin_effect() {
    struct Duck;
    impl deckard_plugins::native::Engine for Duck {
        fn set_param(&mut self, _: u32, _: f64) {}
        fn note_on(&mut self, _: u8, _: u8) {}
        fn note_off(&mut self, _: u8) {}
        fn reset(&mut self) {}
        fn has_sidechain(&self) -> bool {
            true
        }
        fn process(&mut self, _: &mut [f32], _: &mut [f32], _: &deckard_plugins::Transport) {}
        fn process_sc(&mut self, left: &mut [f32], right: &mut [f32], sidechain: Option<(&[f32], &[f32])>, _: &deckard_plugins::Transport) {
            if let Some((sl, sr)) = sidechain {
                for i in 0..left.len() {
                    let sc = sl[i].abs().max(sr[i].abs());
                    let g = (1.0 - sc * 4.0).max(0.0);
                    left[i] *= g;
                    right[i] *= g;
                }
            }
        }
    }
    static DUCK_CLASS: deckard_plugins::native::Class = deckard_plugins::native::Class {
        cid: *b"DKRDTESTDUCKFX01",
        name: "Duck",
        instrument: false,
        params: &[],
        pages: &[],
        make: |_| Box::new(Duck),
        scope: false,
    };

    let run = |on_group: bool, with_source: bool, sc_gain: f64| {
        let mut p = project();
        let click = p.groups[0].sounds[0].clone();
        p.groups[0].sounds.push(click);
        p.groups[0].sounds[1].channel.dest = deckard_model::AudioDest::None;
        let plugin_id = DUCK_CLASS.plugin_id();
        let plug_slot = deckard_model::SlotInfo {
            module: "PluginHost".into(),
            name: "Duck".into(),
            active: true,
            plugin: Some(deckard_model::PluginRef {
                id: plugin_id.clone(),
                name: "Duck".into(),
                chunk: Default::default(),
                pages: Vec::new(),
            }),
            params: vec![
                ("side_chain_filter/gain".into(), sc_gain),
                ("side_chain_filter/center_freq".into(), 0.5),
                ("side_chain_filter/width".into(), 0.5),
            ],
        };
        let host_slot = if on_group {
            p.groups[0].slots = vec![plug_slot];
            p.groups[0].channel.sidechain = vec![(0, deckard_model::ScSource::Sound(0, 1))];
            deckard_plugins::Slot::Group { group: 0, slot: 0 }
        } else {
            p.groups[0].sounds[0].slots = vec![
                deckard_model::SlotInfo {
                    module: "Sampler".into(),
                    name: "Sampler".into(),
                    active: true,
                    plugin: None,
                    params: Vec::new(),
                },
                plug_slot,
            ];
            p.groups[0].sounds[0].channel.sidechain = vec![(1, deckard_model::ScSource::Sound(0, 1))];
            deckard_plugins::Slot::Chain { group: 0, sound: 0, slot: 1 }
        };
        let mut cache = SampleCache::default();
        let id = deckard_plugins::state::PluginId::parse(&plugin_id, "Duck").unwrap();
        cache.plugins.insert(host_slot, deckard_plugins::Plugin::native(&DUCK_CLASS, id, "Duck", 48_000.0));
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        let mut frames = vec![[0f32; 2]; 100];
        frames[0] = [1.0, 1.0];
        set_sample(&mut e, frames, &[], false);
        e.sequencing = false;
        e.note_on(0, 0, 60, 127, 960);
        if with_source {
            e.note_on(0, 1, 60, 127, 960);
        }
        let mut out = vec![0f32; 2 * 480];
        e.process(&mut out);
        out.iter().fold(0f32, |m, v| m.max(v.abs()))
    };

    for on_group in [false, true] {
        let alone = run(on_group, false, 0.5);
        let ducked = run(on_group, true, 0.5);
        let attenuated_sc = run(on_group, true, 0.0);
        assert!(alone > 0.5, "on_group={on_group}: passes without side-chain: {alone}");
        assert!(ducked < 0.01, "on_group={on_group}: ducked by side-chain: {ducked}");
        assert!(
            (attenuated_sc - alone * 0.6).abs() < 1e-3,
            "on_group={on_group}: side_chain_filter/gain=0 (-20 dB) scales side-chain by 0.1: {attenuated_sc} vs {}",
            alone * 0.6
        );
    }
}

/// Playing patterns, rendering ahead, updated as the app updates (incrementally): a
/// sound whose Compressor is side-chained from another goes silent when muted.
#[test]
fn a_side_chained_sound_mutes_while_playing() {
    let (_tmp, mut p) = eight_project();
    let comp: Vec<(String, f64)> = deckard_model::INTERNAL_EFFECTS
        .iter()
        .find(|(c, _)| *c == "Compressor")
        .unwrap()
        .1
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
    let slot = deckard_model::SlotInfo { module: "Compressor".into(), name: "Compressor".into(), active: true, plugin: None, params: comp };
    p.groups[0].sounds[1].slots.push(slot);
    let at = p.groups[0].sounds[1].slots.len() - 1;
    p.groups[0].sounds[1].channel.sidechain = vec![(at, deckard_model::ScSource::Sound(0, 2))];
    let chains0 = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
    // (Audio mute: muting only the notes lets what plays ring out, its delay's echoes
    // too, so it isn't the same as muted all along.)
    for (audio_mute, ahead) in [(true, None), (true, Some(4096))] {
        let mut muted = p.clone();
        muted.groups[0].sounds[1].muted = true;
        muted.groups[0].sounds[1].channel.audio_mute = audio_mute;
        let out = render_with(&p, ahead, 256, 3.0, |k, e, cache| {
            if k != 40 {
                return;
            }
            let (fresh, _) = Engine::new_after(&muted, Some((&p, chains0.as_slice())), None, 48_000, e.mode(), cache);
            drop(e.adopt(fresh, &muted));
        });
        let want = render_with(&muted, ahead, 256, 3.0, |_, _, _| {});
        let tail = |x: &[f32]| x[2 * 96_000..].to_vec();
        assert!(
            max_diff(&tail(&out), &tail(&want)) < 1e-3,
            "audio mute {audio_mute}, ahead {ahead:?}: {}",
            max_diff(&tail(&out), &tail(&want))
        );
    }
}

/// Two groups (BUGS #3): group 1's sound 1 has a Compressor side-chained from group
/// 0's sound 2. Muting group 1 silences it, built muted and muted while it plays,
/// exactly as with no side-chain.
#[test]
fn a_group_with_a_side_chain_from_another_mutes() {
    let (_tmp, mut p) = eight_project();
    let comp = deckard_model::INTERNAL_EFFECTS.iter().find(|(c, _)| *c == "Compressor").unwrap().1;
    let slot = deckard_model::SlotInfo {
        module: "Compressor".into(),
        name: "Compressor".into(),
        active: true,
        plugin: None,
        params: comp.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
    };
    let b = p.groups[0].clone();
    p.groups.push(b);
    p.scenes[0].patterns.push((1, 0));
    p.groups[1].sounds[1].slots.push(slot);
    let at = p.groups[1].sounds[1].slots.len() - 1;
    p.groups[1].sounds[1].channel.sidechain = vec![(at, deckard_model::ScSource::Sound(0, 2))];
    let mut muted = p.clone();
    muted.groups[1].muted = true;
    let mut alone = muted.clone();
    alone.groups[1].sounds[1].channel.sidechain.clear();
    let want = render_with(&alone, None, 256, 2.0, |_, _, _| {});
    let built = render_with(&muted, None, 256, 2.0, |_, _, _| {});
    assert!(max_diff(&built, &want) < 1e-4, "built muted: {}", max_diff(&built, &want));
    let chains0 = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
    for ahead in [None, Some(4096)] {
        let out = render_with(&p, ahead, 256, 3.0, |k, e, cache| {
            if k != 40 {
                return;
            }
            let (fresh, _) = Engine::new_after(&muted, Some((&p, chains0.as_slice())), None, 48_000, e.mode(), cache);
            drop(e.adopt(fresh, &muted));
        });
        let want = render_with(&alone, ahead, 256, 3.0, |_, _, _| {});
        let tail = |x: &[f32]| x[2 * 96_000..].to_vec();
        assert!(
            max_diff(&tail(&out), &tail(&want)) < 1e-3,
            "muted while playing, ahead {ahead:?}: {}",
            max_diff(&tail(&out), &tail(&want))
        );
    }
}

/// A sound whose effect listens to a side-chain plays live after an edit too (an
/// edit keeps its chain unbuilt): with another group focused it isn't rendered
/// ahead, where its effect would hear nothing ("Effects Bug": the Limiter on a
/// hi-hat, side-chained from the kick, stopped once group A was selected).
#[test]
fn a_listening_sound_stays_live_after_an_edit() {
    let (_tmp, mut p) = eight_project();
    let comp = deckard_model::INTERNAL_EFFECTS.iter().find(|(c, _)| *c == "Compressor").unwrap().1;
    let b = p.groups[0].clone();
    p.groups.push(b);
    p.scenes[0].patterns.push((1, 0));
    p.groups[1].sounds[1].slots.push(deckard_model::SlotInfo {
        module: "Compressor".into(),
        name: "Compressor".into(),
        active: true,
        plugin: None,
        params: comp.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
    });
    let at = p.groups[1].sounds[1].slots.len() - 1;
    p.groups[1].sounds[1].channel.sidechain = vec![(at, deckard_model::ScSource::Sound(1, 2))];
    let chains = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
    let run = |ahead: Option<u64>| {
        render_with(&p, ahead, 256, 3.0, |k, e, cache| {
            if k == 0 {
                e.set_focus(Some(0));
            }
            if k == 40 {
                let (fresh, _) = Engine::new_after(&p, Some((&p, chains.as_slice())), None, 48_000, e.mode(), cache);
                drop(e.adopt(fresh, &p));
                assert!(e.live_hint[1][1], "the listening sound still plays live");
            }
            if k == 60 && ahead.is_some() {
                assert!(e.link_ahead(1, 1).is_none(), "not rendered ahead");
                assert!(e.link_ahead(1, 3).is_some(), "(the group's others are)");
            }
        })
    };
    let inline = run(None);
    let ahead = run(Some(700));
    assert!(max_diff(&inline, &ahead) < 1e-4, "as without rendering ahead: {}", max_diff(&inline, &ahead));
}

/// Lemondrop1's shape (BUGS #3, #4): group 0's sound 1 has a Compressor side-chained
/// from all of group 1, a group with effects of its own whose sound 3 has a Limiter
/// side-chained from its sound 2.
fn group_side_chain_project() -> (tempfile::TempDir, Project) {
    let (tmp, mut p) = eight_project();
    let fx = |class: &str| {
        let d = deckard_model::INTERNAL_EFFECTS.iter().find(|(c, _)| *c == class).unwrap().1;
        deckard_model::SlotInfo {
            module: class.into(),
            name: class.into(),
            active: true,
            plugin: None,
            params: d.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    };
    let b = p.groups[0].clone();
    p.groups.push(b);
    p.scenes[0].patterns.push((1, 0));
    p.groups[1].slots = vec![fx("Saturator"), fx("Limiter")];
    p.groups[0].sounds[1].slots.push(fx("Compressor"));
    let at = p.groups[0].sounds[1].slots.len() - 1;
    p.groups[0].sounds[1].channel.sidechain = vec![(at, deckard_model::ScSource::Group(1))];
    p.groups[1].sounds[3].slots.push(fx("Limiter"));
    let at = p.groups[1].sounds[3].slots.len() - 1;
    p.groups[1].sounds[3].channel.sidechain = vec![(at, deckard_model::ScSource::Sound(1, 2))];
    (tmp, p)
}

/// BUGS #3: muting the listening group silences it, built muted and while playing.
#[test]
fn a_group_side_chained_from_a_group_mutes() {
    let (_tmp, p) = group_side_chain_project();
    let mut muted = p.clone();
    muted.groups[0].muted = true;
    let mut alone = muted.clone();
    alone.groups[0].sounds[1].channel.sidechain.clear();
    for ahead in [None, Some(4096)] {
        let want = render_with(&alone, ahead, 256, 2.0, |_, _, _| {});
        let built = render_with(&muted, ahead, 256, 2.0, |_, _, _| {});
        assert!(max_diff(&built, &want) < 1e-4, "built muted, ahead {ahead:?}: {}", max_diff(&built, &want));
    }
    let chains0 = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
    for ahead in [None, Some(4096)] {
        let out = render_with(&p, ahead, 256, 3.0, |k, e, cache| {
            if k != 40 {
                return;
            }
            let (fresh, _) = Engine::new_after(&muted, Some((&p, chains0.as_slice())), None, 48_000, e.mode(), cache);
            drop(e.adopt(fresh, &muted));
        });
        let want = render_with(&alone, ahead, 256, 3.0, |_, _, _| {});
        let tail = |x: &[f32]| x[2 * 96_000..].to_vec();
        assert!(
            max_diff(&tail(&out), &tail(&want)) < 1e-3,
            "muted while playing, ahead {ahead:?}: {}",
            max_diff(&tail(&out), &tail(&want))
        );
    }
}

/// BUGS #4: which group has the focus doesn't change what's heard.
#[test]
fn the_focus_doesnt_change_a_group_side_chain() {
    let (_tmp, p) = group_side_chain_project();
    let at = |focus: Option<usize>| {
        render_with(&p, Some(4096), 256, 2.0, |k, e, _| {
            if k == 0 {
                e.set_focus(focus);
            }
        })
    };
    let none = at(None);
    for f in [Some(0), Some(1)] {
        assert!(max_diff(&at(f), &none) < 1e-4, "focus {f:?}: {}", max_diff(&at(f), &none));
    }
}

/// A side-chain left on a sound's instrument slot (as moving or adding slots used to
/// leave one) doesn't stop the sound muting.
#[test]
fn a_side_chain_on_the_instrument_slot_still_mutes() {
    let (_tmp, mut p) = eight_project();
    p.groups[0].sounds[1].channel.sidechain = vec![(0, deckard_model::ScSource::Sound(0, 2))];
    for audio_mute in [true, false] {
        let mut muted = p.clone();
        muted.groups[0].sounds[1].muted = true;
        muted.groups[0].sounds[1].channel.audio_mute = audio_mute;
        let mut alone = muted.clone();
        alone.groups[0].sounds[1].channel.sidechain.clear();
        let a = render_with(&muted, None, 256, 2.0, |_, _, _| {});
        let b = render_with(&alone, None, 256, 2.0, |_, _, _| {});
        assert!(max_diff(&a, &b) < 1e-4, "audio mute {audio_mute}: {}", max_diff(&a, &b));
    }
}

/// A sound whose effect is side-chained from another still mutes: built muted, and
/// muted while it plays.
#[test]
fn a_side_chained_sound_still_mutes() {
    let mut p = project();
    let click = p.groups[0].sounds[0].clone();
    p.groups[0].sounds.push(click);
    let slot = |module: &str, params: Vec<(String, f64)>| deckard_model::SlotInfo {
        module: module.into(),
        name: module.into(),
        active: true,
        plugin: None,
        params,
    };
    let comp: Vec<(String, f64)> = deckard_model::INTERNAL_EFFECTS
        .iter()
        .find(|(c, _)| *c == "Compressor")
        .unwrap()
        .1
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
    p.groups[0].sounds[0].slots = vec![slot("Sampler", Vec::new()), slot("Compressor", comp)];
    p.groups[0].sounds[0].channel.sidechain = vec![(1, deckard_model::ScSource::Sound(0, 1))];
    let mut muted = p.clone();
    muted.groups[0].sounds[0].muted = true;
    muted.groups[0].sounds[0].channel.audio_mute = true;
    let peak = |e: &mut Engine| {
        e.note_on(0, 0, 60, 127, 960);
        let mut out = vec![0f32; 2 * 4800];
        e.process(&mut out);
        out.iter().fold(0f32, |m, v| m.max(v.abs()))
    };
    let mut e = click_engine_for(&p, 48_000);
    e.sequencing = false;
    assert!(peak(&mut e) > 0.1, "it plays");
    let mut e = click_engine_for(&muted, 48_000);
    e.sequencing = false;
    assert!(peak(&mut e) < 1e-6, "built muted: silent");
    // Muted while playing (the app's edit).
    let mut e = click_engine_for(&p, 48_000);
    e.sequencing = false;
    peak(&mut e);
    e.update(&muted, None, &mut SampleCache::default());
    let mut out = vec![0f32; 2 * 48_000];
    e.note_on(0, 0, 60, 127, 960);
    e.process(&mut out);
    let tail = out[2 * 24_000..].iter().fold(0f32, |m, v| m.max(v.abs()));
    assert!(tail < 1e-6, "muted while playing: silent ({tail})");
}
