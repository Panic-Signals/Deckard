//! What analysis hears in an audio file (WAV, AIFF, MP3), or in ranges of it:
//! `cargo run --release -p deckard-analysis --example detect -- FILE [START:SECONDS ...]`

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(input) = args.first() else {
        eprintln!("detect FILE [START:SECONDS ...]");
        return;
    };
    let t = std::time::Instant::now();
    let sample = deckard_engine::sample::Sample::load(std::path::Path::new(input)).unwrap();
    let (frames, rate) = (sample.frames, sample.rate);
    eprintln!("{} frames at {rate} Hz, read in {:.2} s", frames.len(), t.elapsed().as_secs_f64());
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
        let t = std::time::Instant::now();
        let a = deckard_analysis::analyse(part, rate, "");
        eprintln!("analysed in {:.2} s", t.elapsed().as_secs_f64());
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
        // Find: sixteen starts, on the bars when there's a tempo.
        let t = std::time::Instant::now();
        let lines: Vec<usize> = a.tempo.map_or(Vec::new(), |tp| {
            let bar = 4.0 * 60.0 / tp.bpm * f64::from(rate);
            (0..).map(|k| tp.offset as f64 + k as f64 * bar).take_while(|f| *f < part.len() as f64).map(|f| f as usize).collect()
        });
        let found = deckard_analysis::find_samples(part, rate, (0, part.len()), &lines, 16);
        let times: Vec<String> =
            found.iter().map(|f| format!("{}:{:04.1}", f / rate as usize / 60, (*f as f64 / f64::from(rate)) % 60.0)).collect();
        eprintln!("found in {:.2} s: {}", t.elapsed().as_secs_f64(), times.join("  "));
        let t = std::time::Instant::now();
        let k = deckard_analysis::key(part, rate);
        eprintln!("key in {:.2} s: {k:?}", t.elapsed().as_secs_f64());
    }
}
