use super::*;

/// Play `notes` (start frame, pitch, velocity, length frames) of a 1 kHz sine at
/// 44.1 kHz through a Sampler with `params`; returns the left channel.
fn play_sine(params: &[(&str, f64)], notes: &[(usize, i64, i64, usize)], frames: usize) -> Vec<f32> {
    play_tone(1000.0, params, notes, frames)
}

fn play_tone(hz: f32, params: &[(&str, f64)], notes: &[(usize, i64, i64, usize)], frames: usize) -> Vec<f32> {
    let sr = 44_100.0;
    let mut e = click_engine(sr as u32);
    e.sequencing = false;
    let sine: Vec<[f32; 2]> = (0..44_100).map(|i| (i as f32 * hz * std::f32::consts::TAU / sr).sin()).map(|x| [x * 0.5, x * 0.5]).collect();
    set_sample(&mut e, sine, params, false);
    let mut out = vec![0f32; frames * 2];
    let mut at = 0;
    let mut events: Vec<(usize, bool, i64, i64)> = notes.iter().flat_map(|&(t, p, v, l)| [(t, true, p, v), (t + l, false, p, v)]).collect();
    events.sort_by_key(|e| (e.0, e.1));
    for (t, on, pitch, vel) in events {
        if t > at {
            e.process(&mut out[at * 2..t.min(frames) * 2]);
            at = t.min(frames);
        }
        if on {
            e.note_hold(0, 0, pitch, vel);
        } else {
            e.note_release(0, 0, pitch);
        }
    }
    if at < frames {
        e.process(&mut out[at * 2..]);
    }
    out.iter().step_by(2).copied().collect()
}

#[test]
fn sampler_velocity_scales_as_its_cube() {
    let full = play_sine(&[], &[(0, 60, 127, 4410)], 4410);
    let half = play_sine(&[], &[(0, 60, 64, 4410)], 4410);
    let ratio = rms(&half[100..]) / rms(&full[100..]);
    let want = (64.0f32 / 127.0).powi(3);
    assert!((ratio - want).abs() < 0.01, "velocity 64: {ratio} (want {want})");
    // Velocity → Volume at 0: velocity doesn't matter.
    let flat = play_sine(&[("velocity_volume", 0.0)], &[(0, 60, 64, 4410)], 4410);
    assert!((rms(&flat[100..]) / rms(&full[100..]) - 1.0).abs() < 0.01);
}

#[test]
fn sampler_low_pass_cuts_highs() {
    let open = play_sine(&[], &[(0, 60, 127, 8820)], 8820);
    let lp = play_sine(&[("filter", 1.0), ("cutoff", 0.2)], &[(0, 60, 127, 8820)], 8820);
    let hp = play_sine(&[("filter", 3.0), ("cutoff", 0.2)], &[(0, 60, 127, 8820)], 8820);
    let (o, l, h) = (rms(&open[4410..]), rms(&lp[4410..]), rms(&hp[4410..]));
    assert!(crate::sampler::cutoff_hz(0.2, 1) < 400.0, "{}", crate::sampler::cutoff_hz(0.2, 1));
    assert!(l < 0.2 * o, "1 kHz through a low pass at {:.0} Hz: {l} vs {o}", crate::sampler::cutoff_hz(0.2, 1));
    assert!(h > 0.7 * o, "and through a high pass: {h} vs {o}");
}

/// Level (dB, relative to no filter) of tones at several frequencies through each
/// filter mode, and how far each engine model's output is from Standard.
#[test]
fn sampler_filters_and_engine_models() {
    let freqs = [100.0, 300.0, 1000.0, 3000.0, 8000.0];
    let level = |hz: f32, params: &[(&str, f64)]| rms(&play_tone(hz, params, &[(0, 60, 127, 8820)], 8820)[4410..]);
    let db = |a: f32, b: f32| 20.0 * (a / b).log10();
    let mut table = Vec::new();
    for (name, params) in [
        ("LP2 cutoff .55 res 0", vec![("filter", 1.0), ("cutoff", 0.55)]),
        ("LP2 cutoff .55 res .8", vec![("filter", 1.0), ("cutoff", 0.55), ("resonance", 0.8)]),
        ("BP2 cutoff .55", vec![("filter", 2.0), ("cutoff", 0.55)]),
        ("HP2 cutoff .55", vec![("filter", 3.0), ("cutoff", 0.55)]),
        ("EQ 1 kHz gain +.5", vec![("filter", 4.0), ("eq_frequency", 0.5663), ("eq_gain", 0.5)]),
        ("EQ 1 kHz gain -1", vec![("filter", 4.0), ("eq_frequency", 0.5663), ("eq_gain", -1.0)]),
    ] {
        let row: Vec<f32> = freqs.iter().map(|&f| db(level(f, &params), level(f, &[]))).collect();
        eprintln!("{name:24} {}", row.iter().map(|d| format!("{d:7.1}")).collect::<String>());
        table.push(row);
    }
    eprintln!(
        "cutoff .55: LP {:.0} Hz, BP {:.0} Hz, HP {:.0} Hz",
        crate::sampler::cutoff_hz(0.55, 1),
        crate::sampler::cutoff_hz(0.55, 2),
        crate::sampler::cutoff_hz(0.55, 3)
    );
    // Low pass: passes lows, cuts highs; resonance boosts near the cutoff.
    assert!(table[0][0] > -1.0 && table[0][4] < -12.0, "LP {:?}", table[0]);
    // Band pass: peaks in the middle; high pass: the opposite of the low pass.
    assert!(table[2][2] > table[2][0] && table[2][2] > table[2][4], "BP {:?}", table[2]);
    assert!(table[3][0] < -12.0 && table[3][4] > -1.0, "HP {:?}", table[3]);
    // EQ: Maschine's peak band reaches A² at its centre (A = 2^(dB/6); see
    // docs/sampler-dsp.md §8.5): +18 dB at gain 0.5, -36 dB at -1; less away from it.
    assert!((table[4][2] - 18.0).abs() < 1.5 && table[4][0] < table[4][2] - 10.0, "EQ+ {:?}", table[4]);
    assert!((table[5][2] + 36.0).abs() < 1.5, "EQ- {:?}", table[5]);
    // Engine models: difference from Standard (12-bit quantizing, MP60's gain).
    let std_ = play_tone(1000.0, &[], &[(0, 60, 127, 8820)], 8820);
    for m in 1..=6 {
        let x = play_tone(1000.0, &[("mode", m as f64)], &[(0, 60, 127, 8820)], 8820);
        let diff: Vec<f32> = x.iter().zip(&std_).map(|(a, b)| a - b).collect();
        eprintln!("mode {m}: level {:+.2} dB, difference {:.1} dB below the signal", db(rms(&x), rms(&std_)), -db(rms(&diff), rms(&std_)));
    }
}

/// Random Sampler settings (every parameter over its whole range, as knobs can set
/// them) never panic, never produce NaN or infinity, and render in good time.
#[test]
fn sampler_survives_any_settings() {
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let ranges: Vec<(&str, f64, f64)> = deckard_model::SAMPLER_PARAMS
        .iter()
        .map(|(k, _)| {
            let (lo, hi) = match *k {
                "polyphony" => (0.0, 7.0),
                "mode" => (0.0, 6.0),
                "envelope_type" => (0.0, 2.0),
                "filter" => (0.0, 4.0),
                "lfo/type" => (0.0, 4.0),
                "lfo/sync_mode" => (0.0, 2.0),
                "reverse" => (0.0, 1.0),
                "lfo/phase" => (-0.5, 0.5),
                k if k.starts_with("velocity_") || k.starts_with("modwheel_") || k.contains("amount") || k == "tune" || k == "eq_gain" => {
                    (-1.0, 1.0)
                }
                k if k.starts_with("mod_envelope/") && ["pitch", "cutoff", "drive", "pan"].iter().any(|d| k.ends_with(d)) => (-1.0, 1.0),
                _ => (0.0, 1.0),
            };
            (*k, lo, hi)
        })
        .collect();
    let t = std::time::Instant::now();
    for round in 0..300 {
        let mut params: Vec<(&str, f64)> = Vec::new();
        for (k, lo, hi) in &ranges {
            let v = match round % 3 {
                0 => lo + rnd() * (hi - lo),
                1 => {
                    if rnd() < 0.5 {
                        *lo
                    } else {
                        *hi
                    }
                }
                _ => (lo + hi) / 2.0 + (rnd() - 0.5) * 0.1 * (hi - lo),
            };
            let v = if ["polyphony", "mode", "envelope_type", "filter", "lfo/type", "lfo/sync_mode", "reverse"].contains(k) {
                v.round()
            } else {
                v
            };
            params.push((k, v));
        }
        let notes = [(0, 36, 127, 300), (100, 60, 1, 2000), (150, 96, 64, 50), (400, 127, 127, 100), (500, 0, 100, 900)];
        let out = play_tone(440.0, &params, &notes, 2000);
        assert!(out.iter().all(|x| x.is_finite()), "round {round}: not finite with {params:?}");
        assert!(out.iter().all(|x| x.abs() < 100.0), "round {round}: blew up with {params:?}");
    }
    assert!(t.elapsed().as_secs_f64() < 60.0, "too slow: {:?}", t.elapsed());
}

/// Knob drags while notes sound: every block, the Sampler is rebuilt with other
/// settings and takes over the sounding voices (as `Engine::adopt` does).
#[test]
fn sampler_survives_settings_changing_while_playing() {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let sr = 44_100.0;
    let mut e = click_engine(sr as u32);
    e.sequencing = false;
    let sine: Vec<[f32; 2]> = (0..20_000).map(|i| (i as f32 * 0.05).sin()).map(|x| [x * 0.5, x * 0.5]).collect();
    set_sample(&mut e, sine, &[("envelope_type", 2.0)], true);
    let zone_sample = e.samples[0].clone();
    let mut out = vec![0f32; 2 * 64];
    let keys = [
        "lfo/cutoff_amount",
        "lfo/speed",
        "lfo/type",
        "lfo/sync_mode",
        "lfo/phase",
        "cutoff",
        "resonance",
        "filter",
        "mod_envelope/cutoff",
        "drive",
        "comp",
        "bits",
        "sample_rate",
        "polyphony",
        "mode",
        "envelope_type",
        "glide",
        "lfo/pitch_amount",
        "lfo/pan_amount",
        "tune",
        "start",
        "reverse",
        "eq_frequency",
        "eq_gain",
        "eq_width",
    ];
    let mut model = deckard_model::Sampler::default();
    for block in 0..20_000 {
        if block % 37 == 0 {
            e.note_hold(0, 0, 30 + (rnd() * 70.0) as i64, 1 + (rnd() * 126.0) as i64);
        }
        if block % 53 == 0 {
            e.note_release(0, 0, 30 + (rnd() * 70.0) as i64);
        }
        // A knob moves.
        let k = keys[(rnd() * keys.len() as f64) as usize];
        let v = match k {
            "filter" => (rnd() * 4.99).floor(),
            "lfo/type" => (rnd() * 4.99).floor(),
            "lfo/sync_mode" | "envelope_type" => (rnd() * 2.99).floor(),
            "polyphony" => (rnd() * 7.99).floor(),
            "mode" => (rnd() * 6.99).floor(),
            "reverse" => (rnd() * 1.99).floor(),
            "lfo/phase" => rnd() - 0.5,
            k if k.contains("amount") || k == "tune" || k == "eq_gain" || k == "mod_envelope/cutoff" => rnd() * 2.0 - 1.0,
            _ => rnd(),
        };
        model.set(k, v);
        let old = e.sounds[0][0].take().unwrap();
        let zone = sampler::ZoneRt {
            sample: zone_sample.clone(),
            root: 60,
            keys: (0, 127),
            vels: (0, 127),
            gain: 0.75,
            pan: 0.0,
            tune: 0.0,
            play: (0, 20_000),
            looped: Some((5_000, 15_000)),
            x_fade: (rnd() * 3000.0) as usize,
            fade_in: 0,
            fade_out: 0,
            sync: None,
            lanes: None,
            reverse: false,
            once: false,
            tail_ms: 0.0,
            layer: None,
        };
        let mut fresh = SamplerRt::new(sampler::Settings::new(&model, sr), vec![zone], (1.0, 1.0), sr);
        fresh.adopt(old);
        e.sounds[0][0] = Some(fresh);
        e.process(&mut out);
        assert!(out.iter().all(|x| x.is_finite()), "block {block}: {k} = {v}: {:?} with {:?}", &out[..4], model.params);
        // Before the output's safety clamp: the voices themselves stay in bounds.
        let peak = e.sounds[0][0].as_ref().unwrap().last_peak();
        assert!(peak < 32.0, "block {block}: {k} = {v}: voice peak {peak} with {:?}", model.params);
    }
}

#[test]
fn sampler_mono_chokes_the_previous_note() {
    // Polyphony "1": the second note fades the first out over 10 ms.
    let two = play_sine(&[("polyphony", 1.0)], &[(0, 60, 127, 8820), (2205, 72, 127, 8820)], 8820);
    let poly = play_sine(&[], &[(0, 60, 127, 8820), (2205, 72, 127, 8820)], 8820);
    let (mono, both) = (rms(&two[4000..]), rms(&poly[4000..]));
    assert!(mono < both * 0.8, "one voice left with polyphony 1: {mono} vs {both}");
}

#[test]
fn sampler_envelopes() {
    // AHD (type 1): plays its hold and decay even after a short note.
    let ahd =
        play_sine(&[("envelope_type", 1.0), ("volume_envelope/hold", 0.4), ("volume_envelope/decay", 0.4)], &[(0, 60, 127, 100)], 8820);
    assert!(rms(&ahd[2000..2500]) > 0.1, "AHD ignores the note-off");
    // ADSR: sustains at sustain³ while held, then releases.
    let adsr = play_sine(
        &[("envelope_type", 2.0), ("volume_envelope/decay", 0.0), ("volume_envelope/sustain", 0.5), ("volume_envelope/release", 0.0)],
        &[(0, 60, 127, 4410)],
        8820,
    );
    let held = rms(&adsr[2000..4000]) / rms(&play_sine(&[], &[(0, 60, 127, 8820)], 8820)[2000..4000]);
    assert!((held - 0.125).abs() < 0.01, "sustain 0.5 → level 0.125: {held}");
    assert!(rms(&adsr[5000..]) < 1e-4, "released");
}

#[test]
fn sampler_lofi_reduces_bits() {
    let crushed = play_sine(&[("bits", 0.9)], &[(0, 60, 127, 4410)], 4410);
    let mut levels: Vec<i64> = crushed[200..].iter().map(|v| (v * 1e4).round() as i64).collect();
    levels.sort_unstable();
    levels.dedup();
    assert!(levels.len() < 40, "a few levels only: {}", levels.len());
}

#[test]
fn jump_goes_to_the_section_at_the_given_tick() {
    // One section of 8 quarters; jump from beat 2 back to beat 0 once.
    let mut e = click_engine(1000);
    e.jump_at(Some((2 * PPQ, 0)));
    let mut out = vec![0f32; 2 * 2000];
    e.process(&mut out);
    let onsets: Vec<usize> = (0..2000).filter(|&i| out[i * 2] > 0.5).collect();
    assert_eq!(onsets, [0, 500, 1000, 1500], "beats 0, 1, then 0, 1 again");
    assert_eq!(e.pending_jump(), None);
}

#[test]
fn probability_drops_about_that_share_of_notes() {
    let mut e = click_engine(1000);
    let events: Vec<NoteEvent> = (0..400)
        .map(|i| NoteEvent {
            expr_at: 0,
            roll: None,
            tick: i * 240,
            group: 0,
            sound: 0,
            pitch: 60,
            velocity: 127,
            length: 1,
            probability: 25,
            midi: None,
            cue: None,
            expr: None,
            from: None,
        })
        .collect();
    e.timeline = Timeline { events, length: 400 * 240, ..Default::default() };
    let out = render(&mut e, 120.0);
    let played = (0..out.len() / 2).filter(|&i| out[i * 2] > 0.5).count();
    assert!((70..=130).contains(&played), "about 100 of 400 notes at 25 %: {played}");
}

/// PROB LOCK: a locked sound's chances come out the same on every pass (the first
/// pass's), an unlocked one's are rolled again each pass.
#[test]
fn a_locked_sounds_chances_repeat_every_pass() {
    let passes = |locked: bool| {
        let mut e = click_engine(1000);
        e.looping = true;
        let events: Vec<NoteEvent> = (0..32)
            .map(|i| NoteEvent {
                expr_at: 0,
                roll: None,
                tick: i * 240,
                group: 0,
                sound: 0,
                pitch: 60,
                velocity: 127,
                length: 1,
                probability: 50,
                midi: None,
                cue: None,
                expr: None,
                from: None,
            })
            .collect();
        let prob_locked = if locked { vec![(0, 0)] } else { Vec::new() };
        e.timeline = Timeline { events, length: 32 * 240, prob_locked, ..Default::default() };
        let mut out = vec![0f32; 2 * 512];
        while e.fired.len() < 32 * 4 {
            e.process(&mut out);
        }
        let plays: Vec<bool> = e.fired.iter().map(|f| f.4).collect();
        plays.chunks(32).take(4).map(|c| c.to_vec()).collect::<Vec<_>>()
    };
    let locked = passes(true);
    assert!(locked.windows(2).all(|w| w[0] == w[1]), "the same each pass");
    assert!(locked[0].contains(&true) && locked[0].contains(&false));
    let free = passes(false);
    assert!(free.windows(2).any(|w| w[0] != w[1]), "rolled again");
}

/// A sound sent to Out 2 plays there, not on the master's output; with the master
/// on Out 3, what goes through it plays on Out 3 (`process`'s buffer).
#[test]
fn sounds_play_on_their_external_outputs() {
    use deckard_model::AudioDest as D;
    let mut p = project();
    p.groups[0].sounds[0].channel.dest = D::Ext(1);
    let mut e = click_engine_for(&p, 48_000);
    e.sequencing = false;
    let play = |e: &mut Engine| {
        e.note_on(0, 0, 60, 127, 960);
        let mut out = vec![0f32; 2 * 4800];
        let mut ext = vec![vec![0f32; 2 * 4800]; OUTPUTS];
        e.process_outputs(&mut out, &mut ext);
        let peak = |x: &[f32]| x.iter().fold(0f32, |m, v| m.max(v.abs()));
        (peak(&out), ext.iter().map(|b| peak(b)).collect::<Vec<f32>>())
    };
    let (master, ext) = play(&mut e);
    assert!(master < 1e-6 && ext[1] > 0.5, "on Out 2 only: {master} {ext:?}");
    assert_eq!(e.master_bus(), Some(0));
    let mut q = project();
    q.master_output = D::Ext(2);
    let e2 = click_engine_for(&q, 48_000);
    assert_eq!(e2.master_bus(), Some(2), "the master on Out 3");
}

/// Routing: a sound's Dest. decides which channels' levels it passes through.
#[test]
fn routing_gains() {
    use deckard_model::AudioDest as D;
    let mut p = project();
    p.groups[0].level = 0.5; // quieter than unity
    p.groups.push(Group { level: 0.75, pan: 1.0, sounds: vec![Sound { level: 0.75, ..Default::default() }], ..Default::default() });
    let unity = 1.0f32;
    let close = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4;
    let g05 = deckard_model::level_gain(0.5) as f32;
    assert!(close(sound_gain(&p, 0, 0), (g05, g05)), "through its group");
    p.groups[0].sounds[0].channel.dest = D::Master;
    assert!(close(sound_gain(&p, 0, 0), (unity, unity)), "past its group");
    p.groups[0].sounds[0].channel.dest = D::None;
    assert_eq!(sound_gain(&p, 0, 0), (0.0, 0.0));
    p.groups[0].sounds[0].channel.dest = D::Group(1);
    let g = sound_gain(&p, 0, 0);
    assert!(g.0 < 1e-4 && (g.1 - 2f32.sqrt()).abs() < 1e-3, "through group 2, panned right: {g:?}");
    p.groups[0].sounds[0].channel.dest = D::Ext(1);
    assert!(close(sound_gain(&p, 0, 0), (unity, unity)), "straight onto Out 2");
    assert_eq!(sound_bus(&p, 0, 0), Some(1));
    p.groups[0].sounds[0].channel.dest = D::Ext(OUTPUTS as i64);
    assert_eq!(sound_gain(&p, 0, 0), (0.0, 0.0), "an output there isn't");
    p.groups[0].sounds[0].channel.dest = D::Parent;
    p.groups[0].sounds[0].cue = true;
    assert!(close(sound_gain(&p, 0, 0), (unity, unity)), "the cue bus, on the master's output");
    assert_eq!(sound_bus(&p, 0, 0), Some(0), "its own way there, past the master");
    p.cue_output = D::Ext(1);
    assert!(close(sound_gain(&p, 0, 0), (unity, unity)), "the cue bus on Out 2");
    assert_eq!(sound_bus(&p, 0, 0), Some(1));
}

/// A sound's chain runs Maschine's own effects: a limiter with 30 dB of drive and a
/// low ceiling holds the sound down, and bypassed it doesn't.
#[test]
fn internal_effects_run_in_the_chain() {
    let mut p = project();
    let slot = |module: &str, active: bool, params: Vec<(String, f64)>| deckard_model::SlotInfo {
        module: module.into(),
        name: module.into(),
        active,
        plugin: None,
        params,
    };
    let lim = vec![("mode".into(), 1.0), ("threshold".into(), -30.0), ("release".into(), 0.0), ("ceiling".into(), 0.0)];
    let peak = |p: &Project| {
        let mut e = click_engine_for(p, 48_000);
        e.sequencing = false;
        e.note_on(0, 0, 60, 127, 960);
        let mut out = vec![0f32; 2 * 4800];
        e.process(&mut out);
        out.iter().fold(0f32, |m, v| m.max(v.abs()))
    };
    p.groups[0].sounds[0].slots = vec![slot("Sampler", true, Vec::new()), slot("Limiter", false, lim.clone())];
    let dry = peak(&p);
    p.groups[0].sounds[0].slots[1].active = true;
    let wet = peak(&p);
    let ceiling = 10f32.powf(-40.05 / 20.0);
    assert!(dry > 0.5, "the click plays: {dry}");
    assert!(wet <= ceiling * 1.01, "held at the limiter's ceiling: {wet} (dry {dry})");
}

/// A live note placed `offset` frames into a block starts exactly there, whatever
/// the block size (live input keeps its timing to the sample).
#[test]
fn live_events_land_on_their_sample() {
    for (block, offset) in [(256usize, 0u64), (256, 100), (256, 255), (64, 37), (1024, 900)] {
        let mut e = click_engine(48_000);
        e.sequencing = false;
        let mut out = vec![0f32; block * 2];
        e.process(&mut out);
        e.schedule(offset, Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 127, length: PPQ });
        e.process(&mut out);
        let first = out.chunks(2).position(|f| f[0].abs() > 0.1);
        assert_eq!(first, Some(offset as usize), "block {block}, offset {offset}");
    }
}

/// What `foresee` predicts from any moment is exactly what the sequencer then
/// fires (when, which events, whether their probability lets them play): looping,
/// a queued switch to another scene, a queued jump, uneven blocks.
#[test]
fn foresight_matches_the_sequencer() {
    let mut p = project();
    // Two scenes of different lengths and notes with probabilities.
    let notes = |k: i64| -> Vec<Note> {
        (0..12)
            .map(|i| Note {
                ratchet: Default::default(),
                expr: None,
                sound: 0,
                position: i * 160 + k,
                length: 100,
                pitch: 40 + i + k,
                velocity: 100,
                probability: 30 + (i * 7) % 70,
            })
            .collect()
    };
    p.groups[0].patterns = vec![
        Pattern { length: 2 * PPQ, notes: notes(0), ..Default::default() },
        Pattern { length: 3 * PPQ, notes: notes(5), ..Default::default() },
    ];
    p.scenes = vec![Scene { patterns: vec![(0, 0)], ..Default::default() }, Scene { patterns: vec![(0, 1)], ..Default::default() }];
    p.sections = vec![Section { scene: Some(0), length: 4 * PPQ, auto: false }, Section { scene: Some(1), length: 6 * PPQ, auto: false }];
    for (mode, setup) in [(Mode::Scene(0), 0), (Mode::Scene(0), 1), (Mode::Arrangement, 2)] {
        let mut e = click_engine_for(&p, 48_000);
        e.jump(&p, mode);
        e.looping = true;
        match setup {
            1 => e.queue(&p, Mode::Scene(1)),
            2 => e.jump_at(Some((3 * PPQ, PPQ))),
            _ => {}
        }
        let sizes = [256usize, 100, 512, 37, 1024, 300];
        let mut k = 0;
        let mut outcomes = (false, false);
        // From several moments, foresee what's next, then play it.
        for _ in 0..12 {
            let (start, skip, span) = (e.time, 700u64, 9_000u64);
            let mut want = Vec::new();
            e.foresee(skip, span, |seen| {
                if let Seen::Event { t, e: ev, plays, .. } = seen {
                    want.push((start + t, ev.tick, ev.sound, ev.pitch, plays));
                }
            });
            e.fired.clear();
            let mut done = 0u64;
            while done < skip + span {
                let n = (sizes[k % sizes.len()] as u64).min(skip + span - done) as usize;
                k += 1;
                let mut out = vec![0f32; n * 2];
                e.process(&mut out);
                done += n as u64;
            }
            let got: Vec<_> = e.fired.iter().copied().filter(|f| f.0 >= start + skip).collect();
            assert_eq!(got, want, "mode {mode:?}, setup {setup}, from frame {start}");
            outcomes = (outcomes.0 || got.iter().any(|f| f.4), outcomes.1 || got.iter().any(|f| !f.4));
        }
        assert_eq!(outcomes, (true, true), "notes both kept and dropped by their probability");
    }
}

/// A New Sampler's layer zones get their envelopes and windows, and only the soloed
/// one is heard; the sound gets its morph.
#[test]
fn a_layered_sound_builds_layer_zones() {
    use deckard_model::layers::{Layer, Morph};
    let (_tmp, mut p, _) = new_sampler_project(deckard_model::new_sampler::PlayMode::Repitch);
    let snd = &mut p.groups[0].sounds[0];
    snd.new_sampler.as_mut().unwrap().morph = Morph { on: true, position: 0.25, ..Default::default() };
    let smp = snd.sampler.as_mut().unwrap();
    let mut second = smp.zones[0].clone();
    smp.zones[0].meta.as_mut().unwrap().layer = Some(Layer { attack: 0.5, ..Default::default() });
    second.meta.as_mut().unwrap().layer = Some(Layer { solo: true, window: [0.0, 0.5, 0.5, 1.0], ..Default::default() });
    smp.zones.push(second);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    let rt = e.sounds[0][0].as_ref().unwrap();
    let (a, b) = (rt.zones[0].layer.unwrap(), rt.zones[1].layer.unwrap());
    assert!(!a.on && b.on, "the soloed layer only");
    assert_eq!(a.env.attack, 24_000);
    assert_eq!(b.window, [0.0, 0.5, 0.5, 1.0]);
    assert!((rt.morph_pos() - 0.25).abs() < 1e-6);
    assert_eq!(e.morph_position(0, 0), None, "nothing sounds");
    e.sounds[0][0].as_mut().unwrap().note_on(1, 60, 100, &sampler::Context { tempo: 120.0, beats: 0.0, live: false });
    assert!(e.morph_position(0, 0).is_some());
    // A sound without layers has no morph.
    let (_tmp, plain, _) = new_sampler_project(deckard_model::new_sampler::PlayMode::Repitch);
    let (e, _) = Engine::new(&plain, None, 48_000, Mode::Arrangement, &mut cache);
    assert!(e.sounds[0][0].as_ref().unwrap().zones[0].layer.is_none());
}

/// A layer whose file is missing is left out; the others play, their windows and
/// mutes as they were.
#[test]
fn a_missing_layer_file_leaves_the_rest() {
    use deckard_model::layers::Layer;
    let (_tmp, mut p, _) = new_sampler_project(deckard_model::new_sampler::PlayMode::Repitch);
    let smp = p.groups[0].sounds[0].sampler.as_mut().unwrap();
    let w = deckard_model::layers::spread(3, Default::default());
    let first = smp.zones[0].clone();
    smp.zones = (0..3)
        .map(|k| {
            let mut z = first.clone();
            z.meta.as_mut().unwrap().layer = Some(Layer { window: w[k], mute: k == 2, ..Default::default() });
            z
        })
        .collect();
    smp.zones[1].sample.path = "/nowhere/gone.wav".into();
    let mut cache = SampleCache::default();
    let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    let rt = e.sounds[0][0].as_ref().unwrap();
    assert_eq!(rt.zones.len(), 2);
    let (a, c) = (rt.zones[0].layer.unwrap(), rt.zones[1].layer.unwrap());
    assert_eq!((a.window, c.window), (w[0], w[2]));
    assert!(a.on && !c.on, "the third is still the muted one");
}

fn zone0(e: &Engine) -> &sampler::ZoneRt {
    &e.sounds[0][0].as_ref().unwrap().zones[0]
}

/// A Sync zone is stretched as it plays: it stays its file, asks for no render, and a
/// change of tempo is only the engine's (a second at 141 BPM goes through half as much
/// again of a 94 BPM loop, at its pitch).
#[test]
fn a_sync_zone_is_stretched_as_it_plays() {
    let (_tmp, mut p, frames) = new_sampler_project(deckard_model::new_sampler::PlayMode::Sync);
    p.tempo = 141.0;
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    assert!(cache.wanted_renders().is_empty(), "nothing made ahead");
    let z = zone0(&e);
    assert_eq!(z.sample.frames.len(), frames, "the file itself");
    assert_eq!(z.sync.as_ref().and_then(|s| s.live), Some(0.0));
    e.sequencing = false;
    e.note_hold(0, 0, 60, 100);
    let mut out = vec![0f32; 2 * 4800];
    let mut loud = 0f32;
    for _ in 0..10 {
        e.process(&mut out);
        loud = out.iter().fold(loud, |m, v| m.max(v.abs()));
    }
    assert!(loud > 0.01, "it sounds");
    let at = e.sounds[0][0].as_ref().unwrap().play_position(0).unwrap() as f64;
    assert!((at - 72_000.0).abs() < 3000.0, "{at}");
    // The zone's own speed: at half, half as far.
    p.groups[0].sounds[0].sampler.as_mut().unwrap().zones[0].meta.as_mut().unwrap().speed = 0.5;
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    e.sequencing = false;
    e.note_hold(0, 0, 60, 100);
    for _ in 0..10 {
        e.process(&mut out);
    }
    let at = e.sounds[0][0].as_ref().unwrap().play_position(0).unwrap() as f64;
    assert!((at - 36_000.0).abs() < 3000.0, "{at}");
}

/// A pattern's note sounding when the tempo changes ends where it's written, not at
/// the frame its end fell on at the tempo it began at (a 2-beat note of a 4-beat
/// loop: its end is the note's, not the sample's).
#[test]
fn a_note_sounding_through_a_tempo_change_keeps_its_length() {
    let (_tmp, mut p, _) = new_sampler_project(deckard_model::new_sampler::PlayMode::Sync);
    p.tempo = 240.0;
    let smp = p.groups[0].sounds[0].sampler.as_mut().unwrap();
    for (k, v) in [("envelope_type", 2.0), ("volume_envelope/sustain", 1.0), ("volume_envelope/release", 0.0)] {
        smp.params.insert(k.into(), v);
    }
    let pat = &mut p.groups[0].patterns[0];
    pat.notes.truncate(1);
    pat.notes[0].length = 2 * PPQ;
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Pattern(0, 0), &mut cache);
    let voices = |e: &Engine| e.sounds[0][0].as_ref().unwrap().voices();
    // A beat at 240 (12,000 frames): the note has a beat to go.
    play_frames(&mut e, 12_000);
    assert_eq!(voices(&e), 1);
    p.tempo = 120.0;
    e.update(&p, None, &mut cache);
    // Three quarters of a beat at 120: past where it ended at 240 (12,000 on).
    play_frames(&mut e, 18_000);
    assert_eq!(voices(&e), 1, "still held in its second beat");
    play_frames(&mut e, 12_000);
    assert_eq!(voices(&e), 0, "over at its end");
    // The tempo raised: it ends sooner, with its beats.
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Pattern(0, 0), &mut cache);
    play_frames(&mut e, 12_000);
    p.tempo = 240.0;
    e.update(&p, None, &mut cache);
    // A beat and a half to go: 18,000 frames at 240 (36,000 at 120).
    play_frames(&mut e, 12_000);
    assert_eq!(voices(&e), 1, "in its second beat");
    play_frames(&mut e, 12_000);
    assert_eq!(voices(&e), 0, "over at its end, not a beat and a half on");
}

/// A Sync note sounding when the tempo changes stays with the song: its beats fall
/// on the song's after the change as before (the stretcher, fed ahead for the tempo
/// it began at, isn't left ahead of the song or behind it).
#[test]
fn a_stretched_note_stays_on_the_beat_through_a_tempo_change() {
    // Where the hits of an 8-beat 94 BPM loop (a burst on each beat) are heard.
    let onsets = |from: f64, to: f64, first: usize| -> Vec<i64> {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("hits.wav");
        let spec = hound::WavSpec { channels: 2, sample_rate: 48_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        let beat = 48_000.0 * 60.0 / 94.0;
        let frames = (beat * 8.0) as usize;
        for i in 0..frames {
            let into = i as f64 % beat;
            let v = if into < 480.0 { (i as f32 / 48_000.0 * 2000.0 * std::f32::consts::TAU).sin() * 0.8 } else { 0.0 };
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        let path: String = path.to_string_lossy().into();
        let mut p = project();
        p.tempo = from;
        let snd = &mut p.groups[0].sounds[0];
        snd.new_sampler = Some(Default::default());
        let z = &mut snd.sampler.as_mut().unwrap().zones[0];
        z.sample = deckard_model::SampleRef { kind: 0, path: path.clone(), ..Default::default() };
        z.play_range = (0, frames as i64 - 1);
        z.meta = Some(deckard_model::new_sampler::ZoneMeta {
            file: path,
            mode: deckard_model::new_sampler::PlayMode::Sync,
            bpm: Some(94.0),
            beats: Some(8.0),
            ..Default::default()
        });
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        e.sequencing = false;
        e.note_hold(0, 0, 60, 100);
        let mut heard = Vec::new();
        let mut out = vec![0f32; 2 * 64];
        let mut run = |e: &mut Engine, n: usize, heard: &mut Vec<f32>| {
            for _ in 0..n / 64 {
                e.process(&mut out);
                heard.extend(out.chunks(2).map(|f| f[0].abs()));
            }
        };
        run(&mut e, first, &mut heard);
        p.tempo = to;
        e.update(&p, None, &mut cache);
        run(&mut e, 200_000, &mut heard);
        // A hit: loud after 2,000 quiet frames.
        let mut at = Vec::new();
        let mut quiet = 2000;
        for (i, v) in heard.iter().enumerate() {
            if *v > 0.2 {
                if quiet >= 2000 {
                    at.push(i as i64);
                }
                quiet = 0;
            } else {
                quiet += 1;
            }
        }
        at
    };
    // Two beats at 188 (15,319 frames each, 30,656 in whole blocks), then 94: the
    // third beat on comes every 30,638 frames from there.
    let slow = onsets(188.0, 94.0, 30_656);
    for (k, want) in [(2, 30_656 - 18), (3, 30_656 - 18 + 30_638), (4, 30_656 - 18 + 2 * 30_638)] {
        assert!((slow[k] - want).abs() < 600, "188 to 94, beat {}: {} for {want} ({slow:?})", k + 1, slow[k]);
    }
    // A beat at 94 (30,656 frames: 18 into its second), then 188.
    let fast = onsets(94.0, 188.0, 30_656);
    for (k, want) in [(2, 30_656 + 15_310), (3, 30_656 + 15_310 + 15_319), (4, 30_656 + 15_310 + 2 * 15_319)] {
        assert!((fast[k] - want).abs() < 600, "94 to 188, beat {}: {} for {want} ({fast:?})", k + 1, fast[k]);
    }
}

/// A start point (a zone that plays to its file's end) set to reverse plays from
/// there back to the file's beginning; a slice with an end of its own, its own frames.
#[test]
fn a_reversed_start_point_plays_back_to_the_beginning() {
    let (_tmp, mut p, frames) = new_sampler_project(deckard_model::new_sampler::PlayMode::Repitch);
    let half = frames as i64 / 2;
    let z = &mut p.groups[0].sounds[0].sampler.as_mut().unwrap().zones[0];
    z.play_range = (half, frames as i64 - 1);
    z.meta.as_mut().unwrap().reverse = true;
    let mut cache = SampleCache::default();
    let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    assert!(zone0(&e).reverse);
    assert_eq!(zone0(&e).play, (0, half as usize));
    p.groups[0].sounds[0].sampler.as_mut().unwrap().zones[0].play_range = (half, frames as i64 - 100);
    let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    assert_eq!(zone0(&e).play, (half as usize, frames - 99));
}

/// A slice's one-shot reaches the engine, and a reversed start point that loops keeps
/// its place (its loop is what plays backwards).
#[test]
fn a_slice_is_a_one_shot_or_a_loop() {
    let (_tmp, mut p, frames) = new_sampler_project(deckard_model::new_sampler::PlayMode::Repitch);
    let half = frames as i64 / 2;
    let z = &mut p.groups[0].sounds[0].sampler.as_mut().unwrap().zones[0];
    z.play_range = (half, frames as i64 - 1);
    z.meta.as_mut().unwrap().once = true;
    let mut cache = SampleCache::default();
    let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    assert!(zone0(&e).once);
    assert_eq!((zone0(&e).tail_ms, e.sounds[0][0].as_ref().unwrap().motion), (0.0, deckard_model::new_sampler::Motion::Off));
    // Its tail and the sound's Slice Motion reach the engine too.
    p.groups[0].sounds[0].sampler.as_mut().unwrap().zones[0].meta.as_mut().unwrap().tail = 250.0;
    p.groups[0].sounds[0].new_sampler.as_mut().unwrap().motion = deckard_model::new_sampler::Motion::Random;
    let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    assert_eq!((zone0(&e).tail_ms, e.sounds[0][0].as_ref().unwrap().motion), (250.0, deckard_model::new_sampler::Motion::Random));
    let z = &mut p.groups[0].sounds[0].sampler.as_mut().unwrap().zones[0];
    z.meta.as_mut().unwrap().reverse = true;
    z.loop_active = true;
    z.loop_range = (half, half + 999);
    let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    assert_eq!(zone0(&e).play, (half as usize, frames));
    assert_eq!(zone0(&e).looped, Some((half as usize, half as usize + 1000)));
}

/// A Sync zone plays its own sample in time (repitched) and asks for its render;
/// given it, it plays the render: as long as 4 beats at 120, in time at 120.
#[test]
fn a_sync_zone_asks_then_plays_its_render() {
    let (_tmp, p, frames) = new_sampler_project(deckard_model::new_sampler::PlayMode::Sync);
    let mut cache = SampleCache::default();
    cache.stretch_ahead(true);
    let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    let z = zone0(&e);
    assert_eq!((z.sample.frames.len(), z.sync.as_ref().unwrap().tempo), (frames, 94.0), "its own sample, in time at 94");
    let jobs = cache.wanted_renders();
    assert_eq!(jobs.len(), 1);
    let (e2, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    drop(e2);
    assert!(cache.wanted_renders().is_empty(), "asked once until made");
    for j in &jobs {
        let r = j.run();
        cache.add_render(j, r);
    }
    let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    let z = zone0(&e);
    let want = (frames as f64 * 94.0 / 120.0).round() as usize;
    assert_eq!((z.sample.frames.len(), z.play), (want, (0, want)));
    assert_eq!(z.sync.as_ref().unwrap().tempo, 120.0);
    // Another tempo asks for another render.
    let mut faster = p.clone();
    faster.tempo = 130.0;
    let (_e, _) = Engine::new(&faster, None, 48_000, Mode::Arrangement, &mut cache);
    assert_eq!(cache.wanted_renders().len(), 1);
}

/// An export makes the renders it needs before it plays: none are left wanted, and
/// what it plays is the stretched loop.
#[test]
fn an_export_waits_for_its_renders() {
    for mode in [deckard_model::new_sampler::PlayMode::Sync, deckard_model::new_sampler::PlayMode::Keys] {
        let (_tmp, p, _) = new_sampler_project(mode);
        let mut cache = SampleCache::default();
        cache.stretch_ahead(true);
        let fmt = export::Format { rate: 48_000, ..Default::default() };
        let cancel = std::sync::atomic::AtomicBool::new(false);
        export::render(&p, None, Mode::Arrangement, 0.0, &fmt, &mut cache, &cancel, &mut |_| {}).unwrap();
        assert!(cache.wanted_renders().is_empty(), "nothing still wanted");
        assert!(!cache.renders.is_empty(), "the render was made");
        let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        assert_eq!(zone0(&e).sync.as_ref().unwrap().tempo, 120.0, "the engine it played had the render");
        assert!(cache.wanted_renders().is_empty());
    }
}

/// A zone's stem lanes are loaded with it; in Sync each is stretched as the sample
/// is (asked for with it; with a lane muted the zone waits for them all).
#[test]
fn stem_lanes_load_and_stretch_with_the_zone() {
    use deckard_model::new_sampler::{PlayMode, StemLane};
    for mode in [PlayMode::Repitch, PlayMode::Sync] {
        let (tmp, mut p, frames) = new_sampler_project(mode);
        let stem = |name: &str| -> String {
            let path = tmp.path().join(name);
            edit::write_wav(&path, &Sample { rate: 48_000, frames: vec![[0.1; 2]; frames] }).unwrap();
            path.to_string_lossy().into()
        };
        let meta = p.groups[0].sounds[0].sampler.as_mut().unwrap().zones[0].meta.as_mut().unwrap();
        meta.stems[0] = Some(StemLane { mute: true, ..StemLane::new(stem("d.wav")) });
        meta.stems[3] = Some(StemLane::new(stem("l.wav")));
        meta.stems[1] = Some(StemLane::new("/nowhere/v.wav".into()));
        let mut cache = SampleCache::default();
        cache.stretch_ahead(true);
        let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        let l = zone0(&e).lanes.clone().expect("lanes");
        assert_eq!(l.samples.iter().map(Option::is_some).collect::<Vec<_>>(), [true, false, false, true], "a missing file is no lane");
        assert_eq!(l.levels, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(l.samples[0].as_ref().unwrap().frames.len(), frames);
        if mode == PlayMode::Sync {
            let jobs = cache.wanted_renders();
            assert_eq!(jobs.len(), 3, "the sample's render and a render a lane");
            // The sample's comes first: a lane is muted, so the zone waits for its
            // lanes' and plays as it is till then (never whole).
            cache.add_render(&jobs[0], jobs[0].run());
            let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
            let z = zone0(&e);
            assert_eq!(z.sample.frames.len(), frames, "not the render yet");
            assert_eq!(z.lanes.as_ref().expect("its lanes").levels, [0.0, 0.0, 0.0, 1.0]);
            assert!(cache.wanted_renders().is_empty(), "asked for once");
            for j in &jobs[1..] {
                cache.add_render(j, j.run());
            }
            let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
            let z = zone0(&e);
            let l = z.lanes.as_ref().expect("stretched lanes");
            assert_eq!(l.samples[3].as_ref().unwrap().frames.len(), z.sample.frames.len(), "as long as the zone's render");
            assert_eq!(l.levels, [0.0, 0.0, 0.0, 1.0]);
        }
    }
}

/// A zone stretched as it plays: its playhead goes on to the end of the sample, as
/// the sound does (the stretcher gives its last frames after it's fed the last).
#[test]
fn a_stretched_zones_playhead_reaches_the_end() {
    let (_tmp, p, frames) = new_sampler_project(deckard_model::new_sampler::PlayMode::Sync);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    e.sequencing = false;
    e.note_hold(0, 0, 60, 100);
    let mut out = vec![0f32; 2 * 240];
    let (mut last, mut blocks) = (0usize, 0);
    loop {
        e.process(&mut out);
        match e.zone_position(0, 0, 0) {
            Some(at) => {
                assert!(at + 600 >= last, "it goes on: {at} after {last}");
                last = last.max(at);
            }
            None if last > 0 => break,
            None => {}
        }
        blocks += 1;
        assert!(blocks < 4000, "it ends");
    }
    assert!(last + 1000 >= frames, "the playhead stopped at {last} of {frames}");
}

/// A Sync zone playing the second half of its file: the playhead is in the sample's
/// frames (from the start marker on), not the render's (from 0), before the render
/// comes and after.
#[test]
fn a_sync_zones_playhead_is_in_the_samples_frames() {
    let (_tmp, mut p, frames) = new_sampler_project(deckard_model::new_sampler::PlayMode::Sync);
    let half = frames as i64 / 2;
    p.groups[0].sounds[0].sampler.as_mut().unwrap().zones[0].play_range = (half, frames as i64 - 1);
    let mut cache = SampleCache::default();
    cache.stretch_ahead(true);
    let at = |cache: &mut SampleCache, blocks: usize| {
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, cache);
        e.sequencing = false;
        e.note_hold(0, 0, 60, 100);
        let mut out = vec![0f32; 2 * 4800];
        for _ in 0..blocks {
            e.process(&mut out);
        }
        e.zone_position(0, 0, 0).expect("it plays") as i64
    };
    let early = at(&mut cache, 1);
    assert!((half..half + 9000).contains(&early), "repitched: {early} from {half}");
    for j in cache.wanted_renders() {
        cache.add_render(&j, j.run());
    }
    // 0.1 s of the render (4800 frames at 120) is 120/94 as much of the sample.
    let one = at(&mut cache, 1);
    assert!((one - half - 6128).abs() < 300, "rendered: {one} from {half}");
    let three = at(&mut cache, 3);
    assert!((three - half - 3 * 6128).abs() < 400, "rendered: {three} from {half}");
}

/// Chops of one sample, each from its own start to the sample's end: one render
/// between them, each zone playing it from its own place.
#[test]
fn chops_of_a_sample_share_a_render() {
    let (_tmp, mut p, frames) = new_sampler_project(deckard_model::new_sampler::PlayMode::Sync);
    let zones = &mut p.groups[0].sounds[0].sampler.as_mut().unwrap().zones;
    let mut chop = zones[0].clone();
    chop.play_range = (frames as i64 / 2, frames as i64 - 1);
    (chop.key_range, chop.root_key) = ((61, 61), 61);
    zones[0].key_range = (60, 60);
    zones.push(chop);
    let mut cache = SampleCache::default();
    cache.stretch_ahead(true);
    let _ = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    let jobs = cache.wanted_renders();
    assert_eq!(jobs.len(), 1, "one render for both");
    assert_eq!(jobs[0].key.range, (0, frames));
    cache.add_render(&jobs[0], jobs[0].run());
    let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    assert!(cache.wanted_renders().is_empty());
    let z = &e.sounds[0][0].as_ref().unwrap().zones;
    assert!(Arc::ptr_eq(&z[0].sample, &z[1].sample));
    let len = z[0].sample.frames.len();
    assert_eq!(z[0].play, (0, len));
    assert_eq!(z[1].play.1, len);
    assert!(z[1].play.0.abs_diff(len / 2) <= 2, "{:?} of {len}", z[1].play);
}

/// Keys asks for the notes' renders around the root, nearest first.
#[test]
fn keys_asks_for_its_notes() {
    let (_tmp, p, _) = new_sampler_project(deckard_model::new_sampler::PlayMode::Keys);
    let mut cache = SampleCache::default();
    cache.stretch_ahead(true);
    let _ = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    let base = cache.wanted_renders();
    assert_eq!(base.len(), 1, "the root's render first");
    cache.add_render(&base[0], base[0].run());
    let _ = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    let notes = cache.wanted_renders();
    assert_eq!(notes.len(), 24);
    let shifts: Vec<f64> = notes.iter().take(3).map(|j| j.semitones).collect();
    assert_eq!(shifts, [1.0, -1.0, 2.0]);
}

/// Repitch zones, and a plain Sampler's zones whatever their meta, play as before.
#[test]
fn repitch_zones_are_untouched() {
    let (_tmp, p, frames) = new_sampler_project(deckard_model::new_sampler::PlayMode::Repitch);
    let mut cache = SampleCache::default();
    let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    assert!(zone0(&e).sync.is_none() && zone0(&e).sample.frames.len() == frames);
    let (_tmp, mut p, _) = new_sampler_project(deckard_model::new_sampler::PlayMode::Sync);
    p.groups[0].sounds[0].new_sampler = None;
    let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    assert!(zone0(&e).sync.is_none());
    assert!(cache.wanted_renders().is_empty());
}
