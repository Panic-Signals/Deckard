//! Build the model for every `.mxprj` under `$MXPRJ_CORPUS` (default /Users/Shared).

use deckard_model::Project;
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

#[test]
fn corpus_models() {
    let root = std::env::var_os("MXPRJ_CORPUS").map_or_else(|| PathBuf::from("/Users/Shared"), PathBuf::from);
    let mut files = Vec::new();
    collect(&root, &mut files, 0);
    if files.is_empty() {
        eprintln!("no .mxprj files under {}; skipping", root.display());
        return;
    }
    let types = mxprj::types::Types::new();
    let (mut notes, mut warned, mut empty_arrangement) = (0usize, Vec::new(), 0usize);
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        let doc = mxprj::Project::parse(&bytes).unwrap();
        let root = doc.decode(&types).unwrap_or_else(|(e, at)| panic!("{}: {e} in {at}", f.display()));
        let p = Project::from_decoded(root);
        assert!(!p.groups.is_empty(), "{}: no groups", f.display());
        assert!(p.tempo > 20.0 && p.tempo < 400.0, "{}: tempo {}", f.display(), p.tempo);
        notes += p.groups.iter().flat_map(|g| &g.patterns).map(|x| x.notes.len()).sum::<usize>();
        if p.sections.is_empty() {
            empty_arrangement += 1;
        }
        if !p.warnings.is_empty() {
            warned.push(format!("{}: {:?}", f.display(), p.warnings));
        }
    }
    eprintln!("{} projects, {notes} notes, {empty_arrangement} without sections, {} with warnings", files.len(), warned.len());
    for w in warned.iter().take(10) {
        eprintln!("  {w}");
    }
    assert!(warned.is_empty());
}
