//! Play one of DECKARD's internal instruments (drum synths, Bass Synth, Poly Synth)
//! with the settings of the instrument in a project made by `maschine-ref make` (the
//! sound keeps slot 0), to null-test it against Maschine's render of the same notes.
//!
//! `cargo run --release -p deckard --example synth-null -- <made.mxprj> <group> <sound> <out.wav> [options]`
//!
//! Options as `maschine-ref render`'s: `--note N[:VEL]` (off 0.5 s before the end),
//! `--notes T:N:V:D,…`, `--cc T:NUM:VAL,…`, `--bend T:VAL,…`, `--seconds S` (default
//! 4), `--sr HZ` (default 48000), `--tempo BPM` (default: the project's), `--preroll S`
//! (default 0: silence rendered first; Maschine's renders run 1 s unless given
//! `--preroll 0`). Events land at the start of the 1024-frame block holding them, as
//! there.

use deckard_doc::Document;
use std::path::Path;

/// maschine-ref's host block (`deckard_plugins::MAX_BLOCK`).
const BLOCK: usize = 1024;

#[derive(Clone, Copy, Debug)]
enum Ev {
    On(i64, i64),
    Off(i64),
    Cc(u8, u8),
    Bend(u16),
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let [_, project, g, s, out, opts @ ..] = &a[..] else {
        eprintln!("synth-null <made.mxprj> <group> <sound> <out.wav> [options]");
        std::process::exit(2)
    };
    let opt = |name: &str| opts.iter().position(|x| x == name).and_then(|i| opts.get(i + 1)).map(String::as_str);
    let d = Document::open(Path::new(project)).expect("project");
    let (g, s): (usize, usize) = (g.parse().unwrap(), s.parse().unwrap());
    let slot = d.project().groups[g].sounds[s].slots.first().expect("a slot").clone();
    let sr: f64 = opt("--sr").map_or(48_000.0, |v| v.parse().expect("--sr"));
    let seconds: f64 = opt("--seconds").map_or(4.0, |v| v.parse().expect("--seconds"));
    let preroll: f64 = opt("--preroll").map_or(0.0, |v| v.parse().expect("--preroll"));
    let tempo: f64 = opt("--tempo").map_or(d.project().tempo, |v| v.parse().expect("--tempo"));
    println!("{} with {:?}", slot.module, slot.params);
    let mut synth = deckard_engine::synth::make(&slot.module, &slot.params, sr).expect("an instrument DECKARD plays");
    synth.set_tempo(tempo);
    let (mut l, mut r) = (vec![0f32; BLOCK], vec![0f32; BLOCK]);
    let mut left = (preroll * sr) as usize;
    while left > 0 {
        let k = left.min(BLOCK);
        synth.process(&mut l[..k], &mut r[..k]);
        left -= k;
    }
    let n = (seconds * sr) as usize;
    let mut events = events(opt("--notes"), opt("--cc"), opt("--bend"), sr);
    if let Some(v) = opt("--note") {
        let mut p = v.split(':');
        let key: i64 = p.next().unwrap().parse().expect("--note");
        let vel: i64 = p.next().map_or(100, |x| x.parse().expect("--note vel"));
        events.insert(0, (0, Ev::On(key, vel)));
        // maschine-ref: off in the block holding 0.5 s before the end.
        let off = n.saturating_sub((0.5 * sr) as usize) / BLOCK * BLOCK;
        events.push((off, Ev::Off(key)));
        events.sort_by_key(|(f, _)| *f);
    }
    events.reverse();
    let (mut ol, mut or) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut at = 0;
    while at < n {
        let k = BLOCK.min(n - at);
        while let Some(&(f, e)) = events.last()
            && f < at + k
        {
            events.pop();
            match e {
                Ev::On(key, vel) => synth.note_on(key, vel),
                Ev::Off(key) => synth.note_off(key),
                Ev::Cc(number, value) => synth.control(deckard_model::MidiKind::Controller { number, value }),
                Ev::Bend(v) => synth.control(deckard_model::MidiKind::PitchBend(v)),
            }
        }
        synth.process(&mut l[..k], &mut r[..k]);
        ol.extend_from_slice(&l[..k]);
        or.extend_from_slice(&r[..k]);
        at += k;
    }
    let spec = hound::WavSpec { channels: 2, sample_rate: sr as u32, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(out, spec).unwrap();
    for (x, y) in ol.iter().zip(&or) {
        w.write_sample(*x).unwrap();
        w.write_sample(*y).unwrap();
    }
    w.finalize().unwrap();
}

/// As maschine-ref's: events at frames rounded down to their block, note-offs first.
fn events(notes: Option<&str>, cc: Option<&str>, bend: Option<&str>, sr: f64) -> Vec<(usize, Ev)> {
    let at = |t: f64| ((t * sr) as usize) / BLOCK * BLOCK;
    let f64_ = |x: &str| x.parse::<f64>().unwrap_or_else(|_| panic!("bad number {x}"));
    let num = |x: &str| x.parse::<i64>().unwrap_or_else(|_| panic!("bad number {x}"));
    let mut out = Vec::new();
    for item in notes.into_iter().flat_map(|s| s.split(',')) {
        let f: Vec<&str> = item.split(':').collect();
        let [t, n, v, d] = f[..] else { panic!("--notes T:N:V:D,...") };
        out.push((at(f64_(t)), Ev::On(num(n), num(v))));
        out.push((at(f64_(t) + f64_(d)), Ev::Off(num(n))));
    }
    for item in cc.into_iter().flat_map(|s| s.split(',')) {
        let f: Vec<&str> = item.split(':').collect();
        let [t, c, v] = f[..] else { panic!("--cc T:NUM:VAL,...") };
        out.push((at(f64_(t)), Ev::Cc(num(c) as u8, num(v) as u8)));
    }
    for item in bend.into_iter().flat_map(|s| s.split(',')) {
        let f: Vec<&str> = item.split(':').collect();
        let [t, v] = f[..] else { panic!("--bend T:VAL,...") };
        out.push((at(f64_(t)), Ev::Bend(num(v) as u16)));
    }
    out.sort_by_key(|(f, e)| (*f, !matches!(e, Ev::Off(_))));
    out
}
