//! What analysis hears in a WAV, or in ranges of it:
//! `cargo run --release -p deckard-analysis --example detect -- IN.wav [START:SECONDS ...]`

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(input) = args.first() else {
        eprintln!("detect IN.wav [START:SECONDS ...]");
        return;
    };
    let mut r = hound::WavReader::open(input).unwrap();
    let spec = r.spec();
    let scale = 2f32.powi(i32::from(spec.bits_per_sample) - 1);
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(Result::unwrap).collect(),
        hound::SampleFormat::Int => r.samples::<i32>().map(|s| s.unwrap() as f32 / scale).collect(),
    };
    let ch = usize::from(spec.channels);
    let frames: Vec<[f32; 2]> = samples.chunks(ch).map(|c| [c[0], c[ch.min(2) - 1]]).collect();
    let rate = spec.sample_rate;
    let at = |seconds: f64| ((seconds * f64::from(rate)) as usize).min(frames.len());
    let ranges: Vec<(f64, f64)> = if args.len() > 1 {
        args[1..]
            .iter()
            .map(|a| {
                let (s, l) = a.split_once(':').expect("START:SECONDS");
                (s.parse().unwrap(), l.parse().unwrap())
            })
            .collect()
    } else {
        vec![(0.0, frames.len() as f64 / f64::from(rate))]
    };
    for (start, len) in ranges {
        let part = &frames[at(start)..at(start + len)];
        let a = deckard_analysis::analyse(part, rate, "");
        print!("{start:>7.2} s +{len:<6.2}");
        match a.tempo {
            Some(t) => print!(
                "  {:>7.2} BPM  {:>6.2} beats  first beat {:>6.3} s  {:?} {:.2}",
                t.bpm,
                t.beats,
                t.offset as f64 / f64::from(rate),
                t.source,
                t.confidence
            ),
            None => print!("  no tempo"),
        }
        match a.root {
            Some(n) => println!("  root {} {:+.0} ct ({:.2})", n.note, n.cents, n.confidence),
            None => println!("  no root"),
        }
    }
}
