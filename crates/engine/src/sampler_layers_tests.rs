//! A sound's layers and their morph, as the sampler plays them.

use super::*;
use deckard_model::layers::{Fade, Layer, Morph, Shape, spread};

const SR: f64 = 48_000.0;

/// Ten seconds at one level.
fn level(c: f32) -> Arc<Sample> {
    Arc::new(Sample { rate: 48_000, frames: vec![[c, c]; 480_000] })
}

fn sine(hz: f64) -> Arc<Sample> {
    let frames = (0..480_000).map(|i| [(std::f64::consts::TAU * hz * f64::from(i) / SR).sin() as f32 * 0.25; 2]).collect();
    Arc::new(Sample { rate: 48_000, frames })
}

fn zone(sample: Arc<Sample>, root: i64, layer: Layer, on: bool) -> ZoneRt {
    let n = sample.frames.len();
    ZoneRt {
        sample,
        root,
        keys: (0, 127),
        vels: (0, 127),
        gain: 0.75,
        pan: 0.0,
        tune: 0.0,
        play: (0, n),
        looped: None,
        x_fade: 0,
        fade_in: 0,
        fade_out: 0,
        sync: None,
        lanes: None,
        reverse: false,
        once: false,
        tail_ms: 0.0,
        layer: Some(LayerRt { on, env: layer_times(&layer, SR), window: layer.window }),
    }
}

/// A layered sound (an ADSR amp envelope at full sustain, 64 notes at most).
fn sound(zones: Vec<ZoneRt>, morph: Morph) -> SamplerRt {
    let mut m = deckard_model::Sampler::default();
    m.set("envelope_type", 2.0);
    m.set("volume_envelope/attack", 0.0);
    m.set("volume_envelope/sustain", 1.0);
    m.set("volume_envelope/release", 0.6);
    m.set("polyphony", 7.0);
    let mut s = SamplerRt::new(Settings::new(&m, SR), zones, (1.0, 1.0), SR);
    s.set_morph(Some(morph));
    s
}

/// Layers of these levels, their windows spread evenly for `morph`'s shape.
fn levels(of: &[f32], morph: Morph) -> SamplerRt {
    let w = spread(of.len(), morph.shape);
    sound(of.iter().zip(w).map(|(&c, window)| zone(level(c), 60, Layer { window, ..Default::default() }, true)).collect(), morph)
}

const CTX: Context = Context { tempo: 120.0, beats: 0.0, live: false };

/// Render `seconds`: the left channel.
fn run(s: &mut SamplerRt, seconds: f64) -> Vec<f32> {
    let mut out = Vec::new();
    let mut left = (seconds * SR) as usize;
    while left > 0 {
        let n = left.min(64);
        let mut buf = vec![0f32; n * 2];
        s.render(&mut buf);
        out.extend(buf.chunks_exact(2).map(|x| x[0]));
        left -= n;
    }
    out
}

/// What it settles at over `seconds`.
fn settled(s: &mut SamplerRt, seconds: f64) -> f32 {
    let out = run(s, seconds);
    out[out.len() - 64..].iter().sum::<f32>() / 64.0
}

/// What a lone layer of level `c` plays at.
fn alone(c: f32) -> f32 {
    let mut s = levels(&[c], Morph::default());
    s.note_on(1, 60, 127, &CTX);
    settled(&mut s, 0.2)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 0.02 * b.abs().max(0.01)
}

/// The biggest step between two frames.
fn step(out: &[f32]) -> f32 {
    out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max)
}

#[test]
fn layers_sound_together_in_tune() {
    let l = Layer::default();
    let mut both = sound(vec![zone(sine(220.0), 57, l, true), zone(sine(440.0), 69, l, true)], Morph::default());
    both.note_on(1, 69, 127, &CTX);
    assert_eq!(both.voices(), 2);
    run(&mut both, 0.1);
    let out = run(&mut both, 0.5);
    let ups = out.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    assert!((218..=222).contains(&ups), "440 Hz from both layers: {ups} cycles in half a second");
    let mut one = sound(vec![zone(sine(440.0), 69, l, true)], Morph::default());
    one.note_on(1, 69, 127, &CTX);
    run(&mut one, 0.1);
    let peak = |o: &[f32]| o.iter().fold(0f32, |m, x| m.max(x.abs()));
    let (a, b) = (peak(&out), peak(&run(&mut one, 0.5)));
    assert!((a / b - 2.0).abs() < 0.1, "two layers in phase: {a} against {b}");
}

#[test]
fn a_layers_attack_brings_it_in_late() {
    let (a, b) = (alone(0.1), alone(0.2));
    let slow = Layer { attack: 0.5, ..Default::default() };
    let mut s = sound(vec![zone(level(0.1), 60, Layer::default(), true), zone(level(0.2), 60, slow, true)], Morph::default());
    s.note_on(1, 60, 127, &CTX);
    let early = settled(&mut s, 0.02);
    assert!(early > 0.9 * a && early < a + 0.3 * b, "mostly the first layer at first: {early} ({a}, {b})");
    let late = settled(&mut s, 0.6);
    assert!(near(late, a + b), "both once its attack is over: {late}");
}

#[test]
fn a_layers_release_ends_its_voice() {
    let short = Layer { release: 0.05, ..Default::default() };
    let long = Layer { release: 5.0, ..Default::default() };
    let mut s = sound(vec![zone(level(0.1), 60, short, true), zone(level(0.2), 60, long, true)], Morph::default());
    s.note_on(1, 60, 127, &CTX);
    run(&mut s, 0.1);
    s.note_off(1, 60);
    run(&mut s, 0.1);
    assert_eq!(s.voices(), 1, "the layer with the short release is over");
}

#[test]
fn a_muted_layer_is_silent_but_plays() {
    let (a, b) = (alone(0.1), alone(0.2));
    let l = Layer::default();
    // (The same samples after the edit, as the engine keeps them.)
    let (x, y) = (level(0.1), level(0.2));
    let zones = |on: bool| vec![zone(Arc::clone(&x), 60, l, true), zone(Arc::clone(&y), 60, l, on)];
    let mut s = sound(zones(false), Morph::default());
    s.note_on(1, 60, 127, &CTX);
    assert_eq!(s.voices(), 2, "the muted layer has its voice");
    assert!(near(settled(&mut s, 0.2), a));
    // Unmuted under the held note (an edit): it comes in, without a click.
    let mut next = sound(zones(true), Morph::default());
    next.adopt(s);
    let out = run(&mut next, 0.1);
    assert!(near(out[out.len() - 1], a + b), "{}", out[out.len() - 1]);
    assert!(step(&out) <= b / (0.005 * SR as f32) * 1.5, "no click: {}", step(&out));
}

#[test]
fn stealing_takes_whole_notes() {
    let mut s = levels(&[0.01; 8], Morph::default());
    for n in 0..17 {
        s.note_on(n + 1, 40 + n as i64, 127, &CTX);
        run(&mut s, 0.001);
    }
    assert_eq!(s.voices(), 128);
    let mut ages: Vec<u64> = s.voices.iter().map(|v| v.age).collect();
    ages.sort_unstable();
    ages.dedup();
    assert_eq!(ages.len(), 16, "sixteen whole notes");
    for a in ages {
        assert_eq!(s.voices.iter().filter(|v| v.age == a).count(), 8);
    }
}

#[test]
fn morph_off_is_a_stack() {
    let (a, b) = (alone(0.1), alone(0.2));
    let mut s = levels(&[0.1, 0.2], Morph::default());
    s.note_on(1, 60, 127, &CTX);
    assert!(near(settled(&mut s, 0.2), a + b));
    assert_eq!(s.morph_position(), None);
}

/// A morph that hardly moves by itself: POSITION is where it is.
fn still(shape: Shape) -> Morph {
    Morph { on: true, sync: false, seconds: 500.0, reset: false, shape, depth: 0.0, ..Default::default() }
}

/// What two layers settle at with POSITION at `p`.
fn at(p: f64, morph: Morph) -> f32 {
    let mut s = levels(&[0.1, 0.2], Morph { position: p, ..morph });
    s.note_on(1, 60, 127, &CTX);
    settled(&mut s, 0.2)
}

#[test]
fn the_position_chooses_the_layers() {
    let (a, b) = (alone(0.1), alone(0.2));
    let m = still(Shape::Round);
    assert!(near(at(0.0, m), a), "{}", at(0.0, m));
    assert!(near(at(0.25, m), 0.5 * (a + b)), "{}", at(0.25, m));
    assert!(near(at(0.25, Morph { fade: Fade::Power, ..m }), 0.7071 * (a + b)));
    assert!(near(at(0.5, m), b), "{}", at(0.5, m));
}

#[test]
fn round_comes_back_to_the_first() {
    let (a, b) = (alone(0.1), alone(0.2));
    let m = still(Shape::Round);
    assert!(near(at(0.75, m), 0.5 * (a + b)));
    assert!(near(at(1.0, m), a));
}

#[test]
fn reset_starts_each_note_at_the_start() {
    let (a, b) = (alone(0.1), alone(0.2));
    let m = Morph { on: true, sync: false, seconds: 1.0, ..Default::default() };
    for (reset, starts) in [(true, a), (false, b)] {
        let mut s = levels(&[0.1, 0.2], Morph { reset, ..m });
        run(&mut s, 0.5);
        s.note_on(1, 60, 127, &CTX);
        let first = settled(&mut s, 0.012);
        assert!((first - starts).abs() < 0.08 * starts, "reset {reset}: {first}, not {starts}");
    }
}

#[test]
fn a_synced_morph_follows_the_song() {
    let (a, b) = (alone(0.1), alone(0.2));
    let m = Morph { on: true, sync: true, bars: 1.0, reset: false, ..Default::default() };
    let mut s = levels(&[0.1, 0.2], m);
    s.note_on(1, 60, 127, &CTX);
    let mut at = |beats: f64| {
        s.set_song(beats - 0.01, 120.0);
        run(&mut s, 0.001);
        s.set_song(beats, 120.0);
        settled(&mut s, 0.012)
    };
    let half = at(2.0);
    assert!((half - b).abs() < 0.05 * b, "half a bar in, the second layer: {half}");
    let (one, two) = (at(4.0), at(8.0));
    assert!((one - a).abs() < 0.05 * a, "on the bar, the first: {one}");
    assert!((one - two).abs() < 1e-4, "the same every bar: {one} {two}");
}

#[test]
fn there_and_back_keeps_to_its_depth() {
    let m = Morph { on: true, sync: false, seconds: 0.5, shape: Shape::Back, depth: 0.5, ..Default::default() };
    // Only the third layer has any level: what's heard is its window.
    let mut s = levels(&[0.0, 0.0, 0.3], m);
    s.note_on(1, 60, 127, &CTX);
    let out = run(&mut s, 1.0);
    assert!(out.iter().all(|x| x.abs() < 0.01 * alone(0.3)), "half way never reaches the last layer");
    let mut full = levels(&[0.0, 0.0, 0.3], Morph { depth: 1.0, ..m });
    full.note_on(1, 60, 127, &CTX);
    assert!(run(&mut full, 1.0).iter().any(|x| *x > 0.9 * alone(0.3)), "all the way does");
}

#[test]
fn turning_position_does_not_click() {
    let (a, b) = (alone(0.1), alone(0.2));
    let mut s = levels(&[0.1, 0.2], still(Shape::Round));
    s.note_on(1, 60, 127, &CTX);
    run(&mut s, 0.1);
    s.modulate("morph", 0.5);
    let out = run(&mut s, 0.1);
    assert!(near(out[out.len() - 1], b));
    assert!(step(&out) <= a.max(b) / (0.005 * SR as f32) * 1.5, "{}", step(&out));
    assert!((s.morph_position().unwrap() - 0.5).abs() < 0.01);
}

#[test]
fn layers_change_under_a_held_note() {
    let l = Layer::default();
    let (x, y, z) = (level(0.1), level(0.2), level(0.3));
    let of = |parts: &[&Arc<Sample>]| sound(parts.iter().map(|s| zone(Arc::clone(s), 60, l, true)).collect(), Morph::default());
    let mut s = of(&[&x, &y, &z]);
    s.note_on(1, 60, 127, &CTX);
    run(&mut s, 0.1);
    // One removed, the rest in another order, one given another sample.
    for parts in [vec![&x, &z], vec![&z, &x], vec![&z, &y]] {
        let mut next = of(&parts);
        next.adopt(s);
        s = next;
        let out = run(&mut s, 0.1);
        assert!(out.iter().all(|v| v.is_finite()));
        assert!(step(&out) <= 0.7 / (0.005 * SR as f32) * 1.5, "no click: {}", step(&out));
        assert!(s.voices() <= parts.len(), "{} voices for {} layers", s.voices(), parts.len());
    }
    s.note_off(1, 60);
    run(&mut s, 3.0);
    assert_eq!(s.voices(), 0, "nothing stuck");
}

#[test]
fn morph_position_is_reported() {
    let m = Morph { on: true, sync: false, seconds: 1.0, ..Default::default() };
    let mut s = levels(&[0.1, 0.2], m);
    assert_eq!(s.morph_position(), None, "nothing sounds");
    s.note_on(1, 60, 127, &CTX);
    run(&mut s, 0.25);
    let p = s.morph_position().unwrap();
    assert!((p - 0.25).abs() < 0.01, "{p}");
}

/// A layer's level or pan turned while its note is held is heard on that note, ramped.
#[test]
fn a_layers_level_and_pan_move_under_a_held_note() {
    let x = level(0.5);
    let of = |gain: f32, pan: f32| sound(vec![ZoneRt { gain, pan, ..zone(Arc::clone(&x), 60, Layer::default(), true) }], Morph::default());
    let mut s = of(0.75, 0.0);
    s.note_on(1, 60, 127, &CTX);
    let before = *run(&mut s, 0.1).last().unwrap();
    let mut next = of(0.25, 0.0);
    next.adopt(s);
    s = next;
    let out = run(&mut s, 0.1);
    let after = *out.last().unwrap();
    assert!(after < before * 0.1 && after > before * 0.02, "{before} -> {after}");
    assert!(step(&out) <= 1.0 / (0.005 * SR as f32) * 1.5, "ramped: {}", step(&out));
    // Panned hard right: nothing left on the left.
    let mut next = of(0.25, 1.0);
    next.adopt(s);
    s = next;
    let out = run(&mut s, 0.1);
    assert!(out.last().unwrap().abs() < after * 0.01, "{}", out.last().unwrap());
    assert_eq!(s.voices(), 1);
}

/// A key struck again fast: what still rings of it from before makes way (else the two
/// add or cancel by where each is in its wave, and the level jumps from hit to hit).
#[test]
fn a_key_struck_again_takes_over_its_release() {
    // (One-shot, where only the layers' releases end a note, and ADSR.)
    for env_type in [0, 2] {
        struck_again(env_type);
    }
}

fn struck_again(env_type: i64) {
    let l = Layer { release: 1.0, ..Layer::default() };
    let mut s = sound(vec![zone(sine(220.0), 60, l, true), zone(sine(331.0), 60, l, true)], Morph::default());
    s.settings.env_type = env_type;
    let mut peaks = Vec::new();
    for k in 0..16u64 {
        s.note_on(k + 1, 60, 127, &CTX);
        let mut out = run(&mut s, 0.05 + 0.0007 * k as f64);
        s.note_off(k + 1, 60);
        out.extend(run(&mut s, 0.07));
        // (Past the 10 ms the earlier one takes to go.)
        peaks.push(out[960..].iter().fold(0f32, |m, v| m.max(v.abs())));
        assert!(s.voices() <= 2, "only the last note's layers ring on: {}", s.voices());
    }
    let (lo, hi) = peaks.iter().fold((f32::MAX, 0f32), |(a, b), &p| (a.min(p), b.max(p)));
    assert!(hi - lo < 0.02, "every hit as loud as the first: {peaks:.2?}");
    // Another key's release rings on beside it.
    s.note_on(100, 64, 127, &CTX);
    run(&mut s, 0.05);
    assert_eq!(s.voices(), 4);
}
