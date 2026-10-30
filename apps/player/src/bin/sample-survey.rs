//! sample-survey: how many Sampler zones in a set of projects resolve to local files,
//! and whether those files decode.
//!
//! Usage: sample-survey [dir]   (default /Users/Shared; searches 4 levels for .mxprj)

use deckard_engine::{library::Resolver, sample::Sample};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        match e.file_type() {
            Ok(t) if t.is_dir() && depth < 4 => collect(&p, out, depth + 1),
            Ok(t) if t.is_file() && p.extension().is_some_and(|x| x == "mxprj") => out.push(p),
            _ => {}
        }
    }
}

fn main() {
    let root = std::env::args().nth(1).map_or_else(|| PathBuf::from("/Users/Shared"), PathBuf::from);
    let mut files = Vec::new();
    collect(&root, &mut files, 0);
    let types = mxprj::types::Types::new();
    let (mut zones, mut resolved) = (0usize, 0usize);
    let mut by_ext: BTreeMap<String, (usize, usize)> = BTreeMap::new(); // ext -> (referenced, found)
    let mut missing_by_lib: BTreeMap<String, usize> = BTreeMap::new();
    let mut instruments: BTreeMap<String, usize> = BTreeMap::new();
    let mut envelope_types: BTreeMap<i64, usize> = BTreeMap::new();
    let mut decoded: HashMap<PathBuf, Result<(), String>> = HashMap::new();
    for f in &files {
        let Ok(bytes) = std::fs::read(f) else { continue };
        let Ok(doc) = mxprj::Project::parse(&bytes) else { continue };
        let Ok(tree) = doc.decode(&types) else { continue };
        let p = deckard_model::Project::from_decoded(tree);
        let mut resolver = Resolver::new(Some(f));
        for s in p.groups.iter().flat_map(|g| &g.sounds) {
            if !s.instrument.is_empty() {
                *instruments.entry(s.instrument.clone()).or_default() += 1;
            }
            let Some(smp) = &s.sampler else { continue };
            *envelope_types.entry(smp.envelope_type).or_default() += 1;
            for z in &smp.zones {
                zones += 1;
                let ext = Path::new(&z.sample.path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                let e = by_ext.entry(ext).or_default();
                e.0 += 1;
                match resolver.resolve(&z.sample) {
                    Some(path) => {
                        resolved += 1;
                        e.1 += 1;
                        decoded.entry(path.clone()).or_insert_with(|| Sample::load(&path).map(drop));
                    }
                    None => *missing_by_lib.entry(z.sample.library.clone()).or_default() += 1,
                }
            }
        }
    }
    let bad: Vec<_> = decoded.iter().filter_map(|(p, r)| r.as_ref().err().map(|e| format!("{}: {e}", p.display()))).collect();
    println!(
        "{} projects, {zones} sampler zones, {resolved} resolved to local files ({:.1}%)",
        files.len(),
        100.0 * resolved as f64 / zones.max(1) as f64
    );
    println!("by extension (referenced / found locally): {by_ext:?}");
    println!("missing by library: {missing_by_lib:?}");
    println!("{} distinct local files, {} fail to decode", decoded.len(), bad.len());
    for b in bad.iter().take(10) {
        println!("  {b}");
    }
    println!("sampler envelope types: {envelope_types:?}");
    println!("first module of each sound: {instruments:?}");
}
