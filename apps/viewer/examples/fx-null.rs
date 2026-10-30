//! Run one of DECKARD's internal effects over a WAV, with the settings of the effect
//! in a project made by `maschine-ref make` (the sound keeps one slot, or with
//! `--perform` the group's chain holds a Perform FX, which is used when the sound has
//! no slot), to null-test it against Maschine's render of the same project and input.
//!
//! `cargo run --release -p deckard --example fx-null -- <made.mxprj> <group> <sound> <in.wav> <out.wav> [<sc.wav> [sc-gain-db [sc-delay]]]`
//!
//! With `sc.wav`, that is the effect's side-chain input (as `maschine-ref make --sc`
//! feeds it from In 2), times `sc-gain-db` and `sc-delay` samples late (default 0, 0)
//! to model the source channel. Tempo-synced effects get the project tempo, or
//! `FX_TEMPO` (BPM) when set; `FX_PREROLL` (seconds) runs that much silence first.
//! `FX_PLAY=1` models `maschine-ref render --play`: the transport runs from 0 (after
//! the preroll, which runs stopped) and each host block of 1024 frames gets the song
//! position at its start. `FX_EV=N` (Grain Stretch) sends the settings N samples in, as
//! Maschine's renders do (docs/fx-grain-stretch.md §2).

use deckard_doc::Document;
use std::path::Path;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let d = Document::open(Path::new(&a[1])).expect("project");
    let (g, s): (usize, usize) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let gr = &d.project().groups[g];
    let slot = gr.sounds[s].slots.first().or_else(|| gr.slots.iter().find(|x| x.module == "PerformanceFX")).expect("a slot").clone();
    let (spec, mut l, mut r) = read(&a[4]);
    let sc = a.get(6).map(|p| {
        let (_, mut sl, mut sr) = read(p);
        let gain = 10f32.powf(a.get(7).map_or(0.0, |v| v.parse().expect("sc gain dB")) / 20.0);
        let delay: usize = a.get(8).map_or(0, |v| v.parse().expect("sc delay"));
        for x in [&mut sl, &mut sr] {
            x.iter_mut().for_each(|v| *v *= gain);
            x.splice(0..0, std::iter::repeat_n(0.0, delay));
            x.resize(l.len(), 0.0);
        }
        (sl, sr)
    });
    println!("{} with {:?}", slot.module, slot.params);
    let defer = std::env::var("FX_EV").ok().and_then(|v| v.parse::<usize>().ok());
    let mut fx: Box<dyn deckard_engine::fx::Effect> = match defer {
        Some(n) if slot.module == "GrainStretch" => {
            use deckard_engine::fx::Effect;
            let mut gs = deckard_engine::fx::grain_stretch::GrainStretch::new(spec.sample_rate as f64);
            gs.defer_first_events(n);
            gs.set(&slot.params);
            Box::new(gs)
        }
        _ => deckard_engine::fx::make(&slot.module, &slot.params, spec.sample_rate as f64).expect("an effect DECKARD renders"),
    };
    // The project's tempo, or FX_TEMPO (BPM) to match `maschine-ref render --tempo`.
    let tempo = std::env::var("FX_TEMPO").ok().and_then(|t| t.parse().ok()).unwrap_or(d.project().tempo);
    fx.set_tempo(tempo);
    let play = std::env::var("FX_PLAY").is_ok_and(|v| v != "0");
    // FX_PREROLL (seconds): silence first, as `maschine-ref render --preroll` runs.
    let pre = std::env::var("FX_PREROLL").ok().and_then(|t| t.parse::<f64>().ok()).unwrap_or(0.0);
    let mut left = (pre * spec.sample_rate as f64).round() as usize;
    let (mut zl, mut zr) = (vec![0f32; 512], vec![0f32; 512]);
    while left > 0 {
        let n = left.min(512);
        zl[..n].fill(0.0);
        zr[..n].fill(0.0);
        fx.process_sc(&mut zl[..n], &mut zr[..n], sc.as_ref().map(|_| (&[0f32; 512][..n], &[0f32; 512][..n])));
        left -= n;
    }
    // maschine-ref's host blocks are 1024 frames (`deckard_plugins::MAX_BLOCK`).
    let blk = if play { 1024 } else { 512 };
    for (i, (a, b)) in l.chunks_mut(blk).zip(r.chunks_mut(blk)).enumerate() {
        let n = a.len();
        if play {
            fx.set_transport(Some((i * blk) as f64 / spec.sample_rate as f64 * tempo / 60.0));
        }
        fx.process_sc(a, b, sc.as_ref().map(|(sl, sr)| (&sl[i * blk..i * blk + n], &sr[i * blk..i * blk + n])));
    }
    let out = hound::WavSpec { channels: 2, sample_rate: spec.sample_rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(&a[5], out).unwrap();
    for (x, y) in l.iter().zip(&r) {
        w.write_sample(*x).unwrap();
        w.write_sample(*y).unwrap();
    }
    w.finalize().unwrap();
}

/// A WAV's spec and its left and right channels.
fn read(path: &str) -> (hound::WavSpec, Vec<f32>, Vec<f32>) {
    let mut reader = hound::WavReader::open(path).expect("input wav");
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
        hound::SampleFormat::Int => {
            reader.samples::<i32>().map(|x| x.unwrap() as f32 / (1u32 << (spec.bits_per_sample - 1)) as f32).collect()
        }
    };
    let ch = spec.channels as usize;
    let l = samples.chunks(ch).map(|f| f[0]).collect();
    let r = samples.chunks(ch).map(|f| f[ch.min(2) - 1]).collect();
    (spec, l, r)
}
