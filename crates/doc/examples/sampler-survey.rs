//! How often each Sampler parameter differs from its default, across projects; and
//! the first sound using each (project, group, sound).
use deckard_doc::Document;
use deckard_model::SAMPLER_PARAMS;
use std::collections::BTreeMap;

fn main() {
    let mut count: BTreeMap<&str, (usize, String)> = BTreeMap::new();
    let mut samplers = 0;
    for f in std::env::args().skip(1) {
        let Ok(d) = Document::open(std::path::Path::new(&f)) else { continue };
        for (g, gr) in d.project().groups.iter().enumerate() {
            for (s, snd) in gr.sounds.iter().enumerate() {
                let Some(smp) = &snd.sampler else { continue };
                samplers += 1;
                for (k, def) in SAMPLER_PARAMS {
                    if k.contains("editor") {
                        continue;
                    }
                    if (smp.get(k) - def).abs() > 1e-6 {
                        let e = count.entry(k).or_insert((0, format!("{f} g{g} s{s} = {}", smp.get(k))));
                        e.0 += 1;
                    }
                }
            }
        }
    }
    println!("{samplers} samplers");
    for (k, (n, first)) in count {
        println!("{n:6} {k:28} {first}");
    }
}
