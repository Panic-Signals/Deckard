//! project-summary: print a project's groups, patterns, scenes and arrangement.
//!
//! Usage: project-summary <file.mxprj>

use deckard_model::Project;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: project-summary <file.mxprj>");
        std::process::exit(2);
    });
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(1);
    });
    let root = mxprj::Project::parse(&bytes)
        .map_err(|e| e.to_string())
        .and_then(|p| p.decode(&mxprj::types::Types::new()).map_err(|(e, at)| format!("{e} in {at}")))
        .unwrap_or_else(|e| {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        });
    let p = Project::from_decoded(root);
    let bar = p.bar_length();
    println!("{:?}  {} BPM  {}/{}", p.name, p.tempo, p.numerator, p.denominator);
    for g in &p.groups {
        let notes: usize = g.patterns.iter().map(|x| x.notes.len()).sum();
        println!("  group {:?} {:?}: {} sounds, {} patterns, {} notes", g.name, g.color, g.sounds.len(), g.patterns.len(), notes);
        for pat in &g.patterns {
            println!("    pattern {:?}: {} bars, {} notes", pat.name, pat.length as f64 / bar as f64, pat.notes.len());
        }
    }
    for s in &p.scenes {
        println!("  scene {:?}: {:?}", s.name, s.patterns);
    }
    let bars: Vec<String> =
        p.sections.iter().map(|s| format!("{}x{}", s.scene.map_or("-".into(), |i| i.to_string()), s.length / bar)).collect();
    println!("  arrangement (scene x bars): {}", bars.join(" "));
    for w in &p.warnings {
        println!("  warning: {w}");
    }
}
