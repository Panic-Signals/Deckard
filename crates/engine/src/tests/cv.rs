//! CV routes written into the outputs, at 1 kHz.

use super::*;
use deckard_model::lfo::{Lfo, Shape};
use deckard_model::mod_matrix::{CvRoute, ModSource};

const FRAMES: usize = 64;

/// A free-running 1 Hz Saw on pad 1.
fn saw() -> Lfo {
    Lfo { shape: Shape::Saw, sync: false, hz: 1.0, ..Lfo::new(1) }
}

/// A route from LFO 1 to `channel`, swinging `low` to `high`.
fn route(channel: u8, low: f32, high: f32) -> CvRoute {
    CvRoute { low, high, ..CvRoute::new(ModSource::Lfo(1), channel) }
}

fn engine(lfo: Lfo, routes: Vec<CvRoute>, mask: u32) -> (tempfile::TempDir, Project, Engine) {
    let (tmp, mut p) = lfo_project(1, vec![lfo]);
    p.cv_routes = routes;
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 1000, Mode::Arrangement, &mut cache);
    e.set_cv_channels(mask);
    (tmp, p, e)
}

/// [`engine`], past the glide its first frames start with.
fn settled(lfo: Lfo, routes: Vec<CvRoute>, mask: u32) -> (tempfile::TempDir, Project, Engine) {
    let (tmp, p, mut e) = engine(lfo, routes, mask);
    e.cv_smooth_left = 0;
    (tmp, p, e)
}

/// `e` takes `q` as an edit: the LFOs first, as adopting does.
fn adopt(e: &mut Engine, q: &Project) {
    let mut cache = SampleCache::default();
    let (mut fresh, _) = Engine::new(q, None, 1000, Mode::Arrangement, &mut cache);
    e.adopt_lfos(&mut fresh);
    e.adopt_cvs(&mut fresh);
}

/// `frames` more, in one call, on 16 outputs full of `fill`.
fn run_over(e: &mut Engine, frames: usize, fill: f32) -> Vec<Vec<f32>> {
    let mut out = vec![0f32; frames * 2];
    let mut ext = vec![vec![fill; frames * 2]; 16];
    e.process_outputs(&mut out, &mut ext);
    ext
}

fn run(e: &mut Engine, frames: usize) -> Vec<Vec<f32>> {
    run_over(e, frames, 0.0)
}

/// Channel 2's frames (Out 2's left).
fn left(ext: &[Vec<f32>]) -> Vec<f32> {
    ext[1].iter().step_by(2).copied().collect()
}

#[test]
fn cv_is_written_per_frame_not_per_span() {
    let (_tmp, _p, mut e) = settled(saw(), vec![route(2, -1.0, 1.0)], 0b100);
    let v = left(&run(&mut e, FRAMES));
    assert_eq!(v.len(), FRAMES);
    for n in 1..FRAMES {
        assert!((v[n] - v[n - 1] - 2.0 / 1000.0).abs() < 1e-5, "frame {n}: {} then {}", v[n - 1], v[n]);
    }
}

#[test]
fn cv_overwrites_audio_on_its_channel() {
    let (_tmp, _p, mut e) = settled(saw(), vec![route(2, -1.0, 1.0)], 0b100);
    let ext = run_over(&mut e, FRAMES, 0.9);
    assert_ne!(ext[1][0], 0.9);
    assert_eq!(ext[1][0], -1.0);
    assert_eq!(ext[1][1], 0.9, "the other side of the pair isn't CV only");
}

#[test]
fn a_cv_only_channel_without_a_route_is_silent() {
    let (_tmp, _p, mut e) = engine(saw(), vec![route(2, -1.0, 1.0)], 0b1100);
    let ext = run_over(&mut e, FRAMES, 0.9);
    assert_eq!(ext[1][1], 0.0);
    assert_eq!(ext[1][FRAMES * 2 - 1], 0.0);
}

#[test]
fn a_muted_source_sits_at_low() {
    let (_tmp, _p, mut e) = settled(Lfo { muted: true, ..saw() }, vec![route(2, -0.5, 0.5)], 0b100);
    assert!(left(&run(&mut e, FRAMES)).iter().all(|&v| v == -0.5));
}

#[test]
fn an_unmarked_channel_gets_no_cv_and_is_flagged() {
    let (_tmp, p, mut e) = engine(saw(), vec![route(2, -1.0, 1.0)], 0);
    let ext = run_over(&mut e, FRAMES, 0.25);
    assert!(ext[1].iter().all(|&v| v == 0.25));
    assert_eq!(Engine::cv_conflicts(&p, 0), vec![0]);
    assert!(Engine::cv_conflicts(&p, 0b100).is_empty());
}

#[test]
fn the_first_route_owns_a_shared_channel() {
    let routes = vec![route(2, -0.5, -0.5), route(2, 0.5, 0.5)];
    let (_tmp, p, mut e) = settled(saw(), routes, 0b100);
    assert_eq!(Engine::cv_conflicts(&p, 0b100), vec![1]);
    assert!(left(&run(&mut e, FRAMES)).iter().all(|&v| v == -0.5));
}

#[test]
fn the_lfo_restart_is_smoothed() {
    let (_tmp, _p, mut e) = engine(saw(), vec![route(2, -1.0, 1.0)], 0b100);
    let before = left(&run(&mut e, 400))[399];
    e.lfo_clock_start();
    let v = left(&run(&mut e, 10));
    let jump = before + 1.0;
    assert!(jump > 0.5, "{jump}");
    let limit = jump / (0.005 * 1000.0) + 1e-5;
    assert!((before - v[0]).abs() <= limit, "{before} then {}", v[0]);
    assert!(v[0] > -0.9, "the first frame hasn't jumped: {}", v[0]);
    for n in 1..10 {
        assert!((v[n] - v[n - 1]).abs() <= limit, "frame {n}: {} then {}", v[n - 1], v[n]);
    }
    for (n, x) in v.iter().enumerate().skip(4) {
        assert!((x - (-1.0 + 2.0 * n as f32 / 1000.0)).abs() < 1e-4, "frame {n}: {x}");
    }
}

#[test]
fn smoothing_ends_with_one_frame_spans() {
    let r = route(2, -1.0, 1.0);
    let (_tmp, _p, mut e) = engine(saw(), vec![r.clone()], 0b100);
    run(&mut e, 400);
    e.lfo_clock_start();
    for n in 0..12 {
        let want = r.value(e.lfo_value(1).unwrap());
        let got = run(&mut e, 1)[1][0];
        if n == 0 {
            assert_ne!(got, want, "still gliding");
        }
        if n >= 4 {
            assert_eq!(got, want, "frame {n}");
        }
    }
    assert_eq!(e.cv_smooth_left, 0);
}

#[test]
fn export_writes_no_cv() {
    let (_tmp, p, _) = engine(saw(), vec![route(2, -1.0, 1.0)], 0);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 1000, Mode::Arrangement, &mut cache);
    let ext = run_over(&mut e, FRAMES, 0.25);
    assert!(ext.iter().flatten().all(|&v| v == 0.25));
}

/// A wait stops the free clock, so nothing may run on inside a span.
#[test]
fn cv_holds_still_during_a_count_in() {
    let (_tmp, _p, mut e) = engine(saw(), vec![route(2, -1.0, 1.0)], 0b100);
    run(&mut e, 100);
    e.sequencing = false;
    e.counting = 10_000;
    let v = left(&run(&mut e, FRAMES));
    assert!(v.iter().all(|&x| x == v[0]), "{v:?}");
}

/// A note with no note-off to split the span on (one sent out over MIDI, say).
#[test]
fn cv_envelope_releases_on_its_notes_end() {
    let env = Lfo { env: true, trigger: Some((0, 1)), attack: 0.001, decay: 0.001, sustain: 1.0, release: 1.0, ..Lfo::new(1) };
    let (_tmp, _p, mut e) = engine(env, vec![route(2, 0.0, 1.0)], 0b100);
    e.sequencing = false;
    e.lfo_trigger(0, 1, 60, Some(100));
    let v = left(&run(&mut e, 256));
    assert!((v[50] - 1.0).abs() < 1e-3, "{}", v[50]);
    assert!(v[200] < 0.95, "released at frame 100: {}", v[200]);
}

#[test]
fn cv_tolerates_missing_and_short_buffers() {
    let (_tmp, _p, mut e) = engine(saw(), vec![route(2, -1.0, 1.0), route(31, -1.0, 1.0)], 0xffff_ffff);
    let mut out = vec![0f32; FRAMES * 2];
    e.process_outputs(&mut out, &mut []);
    let mut ext = vec![vec![0.5f32; 3]; 2];
    e.process_outputs(&mut out, &mut ext);
    assert_ne!(ext[1][0], 0.5, "written as far as it reaches");
    assert_eq!(ext[1][1], 0.0);
    assert_eq!(ext[0], vec![0.0; 3]);
}

#[test]
fn cv_stays_in_range() {
    let (_tmp, _p, mut e) = engine(saw(), vec![route(2, -1.0, 5.0)], 0b100);
    let v = left(&run(&mut e, 900));
    assert!(v.iter().all(|x| (-1.0..=1.0).contains(x)));
    assert_eq!(v[899], 1.0);
    let (_tmp, _p, mut e) = engine(saw(), vec![route(2, f32::NAN, 1.0)], 0b100);
    assert!(left(&run(&mut e, FRAMES)).iter().all(|&x| x == 0.0));
}

#[test]
fn cv_carries_its_level_into_a_new_engine() {
    let (_tmp, p, mut old) = engine(saw(), vec![route(2, -1.0, 1.0)], 0b100);
    old.cv_last[2] = 0.7;
    let mut cache = SampleCache::default();
    let (mut new, _) = Engine::new(&p, None, 1000, Mode::Arrangement, &mut cache);
    new.set_cv_channels(0b100);
    new.take_cv(&old);
    let v = left(&run(&mut new, 1));
    assert!((v[0] - (0.7 + (-1.0 - 0.7) / 5.0)).abs() < 1e-4, "{}", v[0]);
}

#[test]
fn cv_keeps_its_level_per_channel_across_an_edit() {
    let (_tmp, p, mut e) = engine(saw(), vec![route(2, -1.0, 1.0), route(3, -1.0, 1.0)], 0b1100);
    e.cv_last[2] = 0.7;
    e.cv_last[3] = -0.3;
    let mut q = p.clone();
    q.cv_routes = vec![route(3, 0.0, 1.0), route(5, 0.0, 1.0)];
    adopt(&mut e, &q);
    assert_eq!(e.cv_routes[0].low, 0.0);
    assert_eq!(e.cv_last[3], -0.3);
    assert_eq!(e.cv_last[2], 0.7, "it has no route now, and glides from there");
    assert_eq!(e.cv_last[5], 0.0, "a channel new to CV starts from nothing");
}

#[test]
fn editing_high_glides_rather_than_steps() {
    let (_tmp, p, mut e) = engine(saw(), vec![route(2, 0.0, 0.4)], 0b100);
    let before = left(&run(&mut e, 500))[499];
    let mut q = p.clone();
    q.cv_routes = vec![route(2, 0.0, 1.0)];
    adopt(&mut e, &q);
    let v = left(&run(&mut e, 10));
    let jump = 0.5 - before;
    assert!(jump > 0.25, "{jump}");
    assert!((v[0] - (before + jump / 5.0)).abs() < 1e-3, "{before} then {}", v[0]);
    for n in 1..5 {
        assert!(v[n] - v[n - 1] <= jump / 3.0, "frame {n}: {} then {}", v[n - 1], v[n]);
    }
    assert!((v[6] - 0.506).abs() < 1e-4, "{}", v[6]);
}

#[test]
fn muting_a_source_glides_to_low() {
    let (_tmp, p, mut e) = engine(saw(), vec![route(2, -0.5, 0.5)], 0b100);
    let before = left(&run(&mut e, 500))[499];
    let mut q = p.clone();
    q.lfos[0].muted = true;
    adopt(&mut e, &q);
    let v = left(&run(&mut e, 10));
    assert!((v[0] - (before + (-0.5 - before) / 5.0)).abs() < 1e-4, "{before} then {}", v[0]);
    assert!(v[4..].iter().all(|&x| x == -0.5), "{v:?}");
}

#[test]
fn removing_a_route_glides_to_zero() {
    let (_tmp, p, mut e) = engine(saw(), vec![route(2, 0.5, 0.5)], 0b100);
    run(&mut e, 20);
    let mut q = p.clone();
    q.cv_routes.clear();
    adopt(&mut e, &q);
    let v = left(&run(&mut e, 8));
    for (n, want) in [0.4, 0.3, 0.2, 0.1, 0.0, 0.0, 0.0, 0.0].iter().enumerate() {
        assert!((v[n] - want).abs() < 1e-5, "frame {n}: {}", v[n]);
    }
}

#[test]
fn the_first_cv_frames_glide_from_zero() {
    let want = [0.1, 0.2, 0.3, 0.4, 0.5, 0.5];
    let (_tmp, _p, mut e) = engine(saw(), vec![route(2, 0.5, 0.5)], 0b100);
    let v = left(&run(&mut e, 6));
    assert!(v.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-5), "a new engine: {v:?}");
    // Marked while running: the same, from 0 V.
    let (_tmp, _p, mut e) = engine(saw(), vec![route(2, 0.5, 0.5)], 0);
    run(&mut e, 20);
    e.set_cv_channels(0b100);
    let v = left(&run(&mut e, 6));
    assert!(v.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-5), "the first mark: {v:?}");
}

/// The cue's clicks land on Out 2 with its left CV only: the click stays on the
/// right, and the left holds the route's voltage.
#[test]
fn a_metronome_on_a_cv_output_never_reaches_it() {
    let (_tmp, mut p, _) = engine(saw(), vec![route(2, 0.25, 0.25)], 0b100);
    p.cue_output = deckard_model::AudioDest::Ext(1);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    e.set_cv_channels(0b100);
    e.cv_smooth_left = 0;
    e.metronome = true;
    let ext = run(&mut e, 512);
    assert!(ext[1].iter().skip(1).step_by(2).any(|v| v.abs() > 0.01), "it clicks");
    assert!(left(&ext).iter().all(|&v| v == 0.25));
}
