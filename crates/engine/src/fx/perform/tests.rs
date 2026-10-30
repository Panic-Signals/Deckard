use super::PerformFx;
use crate::fx::Effect;

const SR: f64 = 48_000.0;

/// A Perform FX in `mode` with Maschine's defaults, `extra` on top, engaged or not.
fn fx(mode: f64, engaged: bool, extra: &[(&str, f64)]) -> PerformFx {
    let mut f = PerformFx::new(SR);
    let mut p: Vec<(String, f64)> = deckard_model::PERFORM_FX.1.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    let mut put = |k: &str, v: f64| match p.iter_mut().find(|(n, _)| n == k) {
        Some(x) => x.1 = v,
        None => p.push((k.to_string(), v)),
    };
    put("mode", mode);
    put("touchstrip_on_off", if engaged { 1.0 } else { 0.0 });
    for (k, v) in extra {
        put(k, *v);
    }
    f.set(&p);
    f.set_tempo(120.0);
    f
}

fn engage(f: &mut PerformFx, on: bool) {
    let mut p = f.params.clone();
    if let Some(x) = p.iter_mut().find(|(n, _)| n == "touchstrip_on_off") {
        x.1 = if on { 1.0 } else { 0.0 };
    }
    f.set(&p);
}

/// Run `input` (mono, the same on both sides) through in 128-sample blocks.
fn run(f: &mut PerformFx, input: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(input.len());
    for chunk in input.chunks(128) {
        let mut l = chunk.to_vec();
        let mut r = chunk.to_vec();
        f.process(&mut l, &mut r);
        out.extend(l);
    }
    out
}

fn sine(freq: f32, n: usize) -> Vec<f32> {
    (0..n).map(|i| (std::f32::consts::TAU * freq * i as f32 / SR as f32).sin() * 0.5).collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

#[test]
fn disengaged_every_mode_passes_the_input_through() {
    // As Maschine's, an effect may still run its signal path released: Filter through its
    // oversampling, 16 samples late and very slightly low-passed (docs/fx-perform-filter.md).
    // So: the input at some lag up to 32 samples, closely.
    let input = sine(440.0, 9600);
    for mode in 0..8 {
        let mut f = fx(mode as f64, false, &[]);
        let out = run(&mut f, &input);
        let worst = (0..=32)
            .map(|lag| out[2000..].iter().zip(&input[2000 - lag..]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max))
            .fold(f32::MAX, f32::min);
        assert!(worst < 1e-3, "mode {mode}: {worst}");
    }
}

#[test]
fn the_filter_cuts_highs_low_pass() {
    // Low pass, the strip low: an 8 kHz tone all but gone, 100 Hz through.
    let mut f = fx(0.0, true, &[("touchstrip_pos", 0.25)]);
    let hi = run(&mut f, &sine(8000.0, 24_000));
    let mut f = fx(0.0, true, &[("touchstrip_pos", 0.25)]);
    let lo = run(&mut f, &sine(100.0, 24_000));
    assert!(rms(&hi[12_000..]) < 0.02, "8 kHz: {}", rms(&hi[12_000..]));
    assert!(rms(&lo[12_000..]) > 0.15, "100 Hz: {}", rms(&lo[12_000..]));
}

#[test]
fn flanger_and_reso_echo_ring_after_an_impulse() {
    for mode in [1.0, 3.0] {
        let mut f = fx(mode, true, &[]);
        let mut x = vec![0.0; 48_000];
        x[2400] = 1.0;
        let out = run(&mut f, &x);
        assert!(rms(&out[2401..4800]) > 1e-3, "mode {mode}: a tail");
    }
}

#[test]
fn burst_echo_repeats_the_burst() {
    // A click on engaging, then silence: it comes back after the echo time.
    let mut f = fx(2.0, true, &[("burst_time_mode", 3.0), ("burst_time", 0.5)]);
    let mut x = vec![0.0; 96_000];
    for v in x.iter_mut().take(480) {
        *v = 0.5;
    }
    let out = run(&mut f, &x);
    assert!(rms(&out[4_800..]) > 1e-3, "echoes after the click");
}

#[test]
fn ring_modulates_the_input() {
    // All wet: a 1 kHz tone comes out as other frequencies (little left of the tone).
    let input = sine(1000.0, 48_000);
    let mut f = fx(4.0, true, &[("ring_dry_wet", 1.0), ("ring_rev_mix", 0.0), ("touchstrip_pos", 0.5)]);
    let out = run(&mut f, &input);
    let (a, b) = (&out[24_000..], &input[24_000..]);
    let corr = a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() / (rms(a) * rms(b) * a.len() as f32);
    assert!(rms(a) > 0.02, "sound: {}", rms(a));
    assert!(corr.abs() < 0.3, "not the tone: {corr}");
}

#[test]
fn stutter_repeats_a_loop() {
    // Engage while noise plays (the loop is taken from it), then feed silence: the loop
    // keeps playing.
    let mut f = fx(5.0, false, &[("stutter_quantize", 0.0), ("stutter_gate", 1.0), ("stutter_direction", 0.0)]);
    let mut seed = 1u32;
    let noise: Vec<f32> = (0..48_000)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1 << 24) as f32 - 0.5
        })
        .collect();
    engage(&mut f, true);
    run(&mut f, &noise);
    let out = run(&mut f, &vec![0.0; 48_000]);
    assert!(rms(&out[24_000..]) > 0.05, "still playing: {}", rms(&out[24_000..]));
}

#[test]
fn tremolo_moves_the_level() {
    let input = sine(440.0, 96_000);
    let mut f = fx(
        6.0,
        true,
        &[("tremolo_trem_vibr", 0.0), ("tremolo_depth_max", 1.0), ("tremolo_depth_min", 1.0), ("tremolo_attack_release", 0.0)],
    );
    let out = run(&mut f, &input);
    let levels: Vec<f32> = out[48_000..].chunks(480).map(rms).collect();
    let (lo, hi) = (levels.iter().cloned().fold(1.0, f32::min), levels.iter().cloned().fold(0.0, f32::max));
    assert!(hi > 2.0 * lo, "{lo} .. {hi}");
}

#[test]
fn scratcher_brakes_to_a_stop() {
    // Engaged with no swirl: the record slows down and stops (silence).
    let input = sine(440.0, 5 * 48_000);
    let mut f = fx(7.0, true, &[("scratcher_wet_level", 0.0), ("touchstrip_pos", 0.0)]);
    let out = run(&mut f, &input);
    assert!(rms(&out[..4800]) > 0.1, "playing at first");
    assert!(rms(&out[4 * 48_000..]) < 0.01, "stopped: {}", rms(&out[4 * 48_000..]));
}

#[test]
fn engaging_and_releasing_do_not_click() {
    // A steady tone through the filter, engaged and released: no step bigger than the
    // tone's own.
    let input = sine(100.0, 48_000);
    let mut f = fx(0.0, false, &[("touchstrip_pos", 0.1)]);
    let mut out = run(&mut f, &input[..16_000]);
    engage(&mut f, true);
    out.extend(run(&mut f, &input[16_000..32_000]));
    engage(&mut f, false);
    out.extend(run(&mut f, &input[32_000..]));
    let step = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
    assert!(step < 0.02, "{step}");
}

#[test]
fn the_factory_makes_it() {
    let p: Vec<(String, f64)> = deckard_model::PERFORM_FX.1.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    assert!(crate::fx::make("PerformanceFX", &p, SR).is_some());
}

/// Released, Reso Echo rings out and stops, even at full Feedback and Saturation.
#[test]
fn reso_echo_dies_away_after_release() {
    for extra in [vec![], vec![("reso_feedback", 1.0), ("reso_saturation", 1.0), ("reso_resonance", 1.0)]] {
        let mut f = fx(3.0, true, &extra);
        let noise: Vec<f32> = (0..24_000).map(|i| ((i * 7919 % 1000) as f32 / 1000.0 - 0.5) * 0.8).collect();
        run(&mut f, &noise);
        engage(&mut f, false);
        let out = run(&mut f, &vec![0.0; 48_000 * 8]);
        let end = rms(&out[out.len() - 4_800..]);
        assert!(end < 1e-3, "{extra:?}: still {end} after 8 s");
    }
}

/// Maschine's own renders of a Perform FX (`maschine-ref make … --perform MODE`, `render`
/// with its 1 s preroll), read from `out/perform-ref/` when they're there: DECKARD's
/// output for the same input and settings, and the residual in dB (L, R).
pub(super) fn null_test(mode: f64, dir: &str, input: &str, render: &str, extra: &[(&str, f64)]) -> Option<(f64, f64)> {
    null_test_with(mode, dir, input, render, extra, &[])
}

/// [`null_test`] with settings changed during the render: (seconds, field, value), each
/// at the start of the 1024-frame block it falls in, as `maschine-ref render --param`.
pub(super) fn null_test_with(
    mode: f64,
    dir: &str,
    input: &str,
    render: &str,
    extra: &[(&str, f64)],
    changes: &[(f64, &str, f64)],
) -> Option<(f64, f64)> {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../out/perform-ref/");
    let read = |p: &str| -> Option<(u32, Vec<f32>, Vec<f32>)> {
        let mut r = hound::WavReader::open(format!("{root}{dir}/{p}")).ok()?;
        let sr = r.spec().sample_rate;
        let v: Vec<f32> = r.samples::<f32>().map(Result::unwrap).collect();
        Some((sr, v.iter().step_by(2).copied().collect(), v.iter().skip(1).step_by(2).copied().collect()))
    };
    let (sr, il, ir) = read(input)?;
    let (_, ml, mr) = read(render)?;
    let mut p: Vec<(String, f64)> = deckard_model::PERFORM_FX.1.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    for (k, v) in [("mode", mode), ("touchstrip_on_off", 1.0)].iter().chain(extra) {
        match p.iter_mut().find(|(n, _)| n == k) {
            Some(x) => x.1 = *v,
            None => p.push((k.to_string(), *v)),
        }
    }
    let mut f = PerformFx::new(sr as f64);
    f.set(&p);
    f.set_tempo(120.0);
    let (mut zl, mut zr) = (vec![0f32; sr as usize], vec![0f32; sr as usize]);
    f.process(&mut zl, &mut zr);
    let (mut l, mut r) = (il.clone(), ir.clone());
    for (k, (a, b)) in l.chunks_mut(1024).zip(r.chunks_mut(1024)).enumerate() {
        let at = k * 1024;
        let due: Vec<_> = changes.iter().filter(|(t, ..)| ((*t * sr as f64) as usize) / 1024 * 1024 == at).collect();
        if !due.is_empty() {
            for (_, field, v) in due {
                p.iter_mut().find(|(n, _)| n == field).expect("a Perform FX field").1 = *v;
            }
            f.set(&p);
        }
        f.process(a, b);
    }
    if std::env::var("NULL_DEBUG").is_ok() {
        let w = (sr / 10) as usize;
        let rows: Vec<String> = l
            .chunks(w)
            .zip(ml.chunks(w))
            .map(|(a, b)| {
                let e = a.iter().zip(b).map(|(x, y)| ((x - y) as f64).powi(2)).sum::<f64>();
                let s = b.iter().map(|y| (*y as f64).powi(2)).sum::<f64>();
                format!("{:.0}", 10.0 * (e / s.max(1e-30)).max(1e-30).log10())
            })
            .collect();
        eprintln!("{render}: per 0.1 s (dB rel.): {}", rows.join(" "));
        let first = l.iter().zip(&ml).position(|(a, b)| (a - b).abs() > 1e-7);
        if let Some(i) = first {
            eprintln!("{render}: first difference at {i}: {} vs {} (in {})", l[i], ml[i], il[i]);
            for j in i.saturating_sub(2)..(i + 4).min(l.len()) {
                eprintln!("  {j}: dk {:.9} m {:.9}", l[j], ml[j]);
            }
        }
    }
    let db = |x: &[f32], y: &[f32]| {
        let e = x.iter().zip(y).map(|(a, b)| ((a - b) as f64).powi(2)).sum::<f64>() / x.len() as f64;
        10.0 * e.max(1e-24).log10()
    };
    Some((db(&l, &ml), db(&r, &mr)))
}

/// Reso Echo released, re-engaged and its strip moved during the render (Maschine's
/// through host automation), against Maschine's renders.
#[test]
fn reso_echo_releases_and_moves_as_maschines() {
    let pos = [("touchstrip_pos", 0.5)];
    type Case<'a> = (&'a str, &'a str, &'a [(&'a str, f64)], &'a [(f64, &'a str, f64)]);
    let cases: &[Case] = &[
        ("in-imp3.wav", "rel-imp.wav", &pos, &[(0.5, "touchstrip_on_off", 0.0)]),
        ("in-noise3.wav", "rel-noise.wav", &pos, &[(1.0, "touchstrip_on_off", 0.0)]),
        ("in-noise3.wav", "move.wav", &pos, &[(0.3, "touchstrip_pos", 0.1), (0.6, "touchstrip_pos", 0.9), (1.2, "touchstrip_pos", 0.33)]),
        (
            "in-noise3.wav",
            "reengage.wav",
            &pos,
            &[(0.5, "touchstrip_on_off", 0.0), (0.9, "touchstrip_on_off", 1.0), (1.4, "touchstrip_on_off", 0.0)],
        ),
        (
            "in-noise4.wav",
            "rel2.wav",
            &[("touchstrip_pos", 0.2), ("reso_feedback", 1.0), ("reso_time", 1.0), ("reso_mix_mode", 1.0), ("reso_pump", 0.9)],
            &[(1.0, "touchstrip_on_off", 0.0), (2.5, "touchstrip_pos", 0.75)],
        ),
    ];
    for (input, render, extra, changes) in cases {
        let Some((l, r)) = null_test_with(3.0, "reso", input, render, extra, changes) else { return };
        eprintln!("{render}: residual L {l:.1} dB, R {r:.1} dB");
        assert!(std::env::var("NULL_DEBUG").is_ok() || (l < -120.0 && r < -120.0), "{render}: L {l:.1} dB, R {r:.1} dB");
    }
}

/// Reso Echo against Maschine's renders (docs/fx-perform-reso-echo.md §7).
#[test]
fn reso_echo_nulls_against_maschine() {
    type Case<'a> = (&'a str, &'a str, &'a [(&'a str, f64)]);
    let cases: &[Case] = &[
        ("in-noise.wav", "def.wav", &[("touchstrip_pos", 0.5)]),
        ("in-imp.wav", "def-imp.wav", &[("touchstrip_pos", 0.5)]),
        ("in-sweep.wav", "def-sweep.wav", &[("touchstrip_pos", 0.5)]),
        (
            "in-noise.wav",
            "low.wav",
            &[("touchstrip_pos", 0.05), ("reso_feedback", 0.95), ("reso_saturation", 1.0), ("reso_resonance", 1.0)],
        ),
        ("in-noise.wav", "insert.wav", &[("touchstrip_pos", 0.7), ("reso_mix_mode", 1.0), ("reso_time", 0.8), ("reso_pump", 1.0)]),
        ("in-noise.wav", "wet.wav", &[("touchstrip_pos", 0.3), ("reso_mix_mode", 2.0), ("reso_wet_level", 1.0), ("reso_time", 0.0)]),
        ("in-noise.wav", "off.wav", &[("touchstrip_on_off", 0.0), ("touchstrip_pos", 0.5)]),
    ];
    for (input, render, extra) in cases {
        let Some((l, r)) = null_test(3.0, "reso", input, render, extra) else { return };
        eprintln!("{render}: residual L {l:.1} dB, R {r:.1} dB");
        assert!(std::env::var("NULL_DEBUG").is_ok() || (l < -120.0 && r < -120.0), "{render}: L {l:.1} dB, R {r:.1} dB");
    }
}

/// A change of mode (Maschine's processor): it waits for the block and then the next
/// 128-sample tick, crossfades over 20 ms (960 samples at 48 kHz) with the new effect's
/// input ramping in, without a click; then only the new effect is heard.
#[test]
fn a_change_of_mode_crossfades_over_twenty_ms() {
    // Reso Echo, released with nothing ringing, passes the input through exactly; Reso
    // Echo engaged doesn't. Tremolo released is the other side.
    let input = sine(220.0, 48_000);
    let mut f = fx(6.0, false, &[]);
    let mut p = f.params.clone();
    let before = run(&mut f, &input[..4096]);
    p.iter_mut().find(|(n, _)| n == "mode").unwrap().1 = 3.0;
    f.set(&p);
    let after = run(&mut f, &input[4096..]);
    let out: Vec<f32> = before.into_iter().chain(after).collect();
    let step = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
    let tone = input.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
    assert!(step < tone * 1.5, "no click: {step} vs {tone}");
    // From 4096 + 960 on (the switch lands on the tick at 4096): Reso Echo alone, released,
    // is the input itself.
    let worst = out[4096 + 960..].iter().zip(&input[4096 + 960..]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    assert!(worst < 1e-6, "{worst}");
}
