//! Per sound: its saved lanes (kind, number), the shown one, and its MIDI event kinds.
use deckard_doc::Document;
use std::collections::BTreeMap;

fn main() {
    let mut combos: BTreeMap<String, usize> = BTreeMap::new();
    for f in std::env::args().skip(1) {
        let Ok(d) = Document::open(std::path::Path::new(&f)) else { continue };
        let p = d.project();
        for (g, gr) in p.groups.iter().enumerate() {
            for (s, snd) in gr.sounds.iter().enumerate() {
                let mut kinds: Vec<String> = gr
                    .patterns
                    .iter()
                    .flat_map(|pt| pt.midi.iter().filter(|m| m.sound == s))
                    .map(|m| match m.kind {
                        deckard_model::MidiKind::Controller { number, .. } => format!("cc{number}"),
                        deckard_model::MidiKind::PitchBend(_) => "bend".into(),
                        deckard_model::MidiKind::ChannelPressure(_) => "pressure".into(),
                    })
                    .collect();
                kinds.sort();
                kinds.dedup();
                let lanes: Vec<String> = snd.lanes.iter().map(|l| format!("{}/{}", l.kind, l.number)).collect();
                let key = format!("lanes {:?} shown {:?} events {:?}", lanes, snd.lane, kinds);
                if !kinds.is_empty() || snd.lanes.len() > 2 {
                    println!("{f} g{g} s{s}: {key}");
                }
                *combos.entry(format!("lanes {:?} shown {:?}", lanes, snd.lane)).or_default() += 1;
            }
        }
    }
    for (k, n) in combos {
        println!("{n:6} {k}");
    }
}
