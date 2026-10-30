//! Separate a WAV into its stems and time it:
//! `cargo run --release -p deckard-stems --example separate -- MODEL_DIR IN.wav OUT_DIR [gpu|cpu]`

use std::sync::atomic::AtomicBool;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, input, out] = [&args[0], &args[1], &args[2]];
    let device = if args.get(3).map(String::as_str) == Some("cpu") { deckard_stems::Device::Cpu } else { deckard_stems::Device::Gpu };
    let model = deckard_stems::load(std::path::Path::new(dir)).unwrap();
    let mut r = hound::WavReader::open(input).unwrap();
    let spec = r.spec();
    let scale = 2f32.powi(spec.bits_per_sample as i32 - 1);
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(Result::unwrap).collect(),
        hound::SampleFormat::Int => r.samples::<i32>().map(|s| s.unwrap() as f32 / scale).collect(),
    };
    let ch = spec.channels as usize;
    let frames: Vec<[f32; 2]> = samples.chunks(ch).map(|c| [c[0], c[ch.min(2) - 1]]).collect();
    eprintln!("{} frames at {} Hz ({:.1} s) on {device:?}", frames.len(), spec.sample_rate, frames.len() as f64 / spec.sample_rate as f64);
    let t = std::time::Instant::now();
    let stems =
        deckard_stems::separate(&model, &frames, spec.sample_rate, device, |p| eprint!("\r{:.0} %", p * 100.0), &AtomicBool::new(false))
            .unwrap();
    eprintln!("\nseparated in {:.1} s", t.elapsed().as_secs_f64());
    // The stems should add up to the mix: how far off they are, and each one's level.
    let rms = |v: &mut dyn Iterator<Item = f32>| {
        let (s, n) = v.fold((0.0f64, 0usize), |(s, n), x| (s + f64::from(x * x), n + 1));
        (s / n.max(1) as f64).sqrt()
    };
    let mix = rms(&mut frames.iter().flat_map(|f| [f[0], f[1]]));
    let rest = rms(&mut (0..frames.len()).flat_map(|i| {
        let sum = stems.iter().fold([0.0f32; 2], |a, s| [a[0] + s.frames[i][0], a[1] + s.frames[i][1]]);
        [frames[i][0] - sum[0], frames[i][1] - sum[1]]
    }));
    eprintln!("stems' sum off by {:.1} dB below the mix", 20.0 * (mix / rest.max(1e-12)).log10());
    for s in &stems {
        eprintln!("{:>6}: {:.1} dB", s.kind.name(), 20.0 * (rms(&mut s.frames.iter().flat_map(|f| [f[0], f[1]])) / mix).log10());
    }
    std::fs::create_dir_all(out).unwrap();
    for s in stems {
        let spec =
            hound::WavSpec { channels: 2, sample_rate: spec.sample_rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut w = hound::WavWriter::create(format!("{out}/{}.wav", s.kind.name()), spec).unwrap();
        for f in s.frames {
            w.write_sample(f[0]).unwrap();
            w.write_sample(f[1]).unwrap();
        }
        w.finalize().unwrap();
    }
}
