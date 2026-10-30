//! doc-roundtrip: open every project under some directories as a Document and save it
//! unchanged (in memory, read-only), checking the bytes are identical, so what DECKARD
//! adds to saving (the DECKARD block, stripping its own fields) leaves a project it has
//! nothing for exactly as it was.
//!
//! Usage: doc-roundtrip [dir ...] (default /Users/Shared)

use deckard_doc::Document;
use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() && depth < 8 {
            collect(&p, out, depth + 1);
        } else if ft.is_file() && p.extension().is_some_and(|x| x == "mxprj") {
            out.push(p);
        }
    }
}

fn main() {
    let mut dirs: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if dirs.is_empty() {
        dirs.push("/Users/Shared".into());
    }
    let mut files = Vec::new();
    for d in &dirs {
        collect(d, &mut files, 0);
    }
    files.sort();
    let (mut same, mut differ, mut failed) = (0, 0, 0);
    for f in &files {
        let Ok(bytes) = std::fs::read(f) else { continue };
        match Document::from_bytes(bytes.clone(), None).and_then(|d| d.to_bytes()) {
            Ok(out) if out == bytes => same += 1,
            Ok(out) => {
                differ += 1;
                println!("DIFFER {} ({} -> {} bytes)", f.display(), bytes.len(), out.len());
            }
            Err(e) => {
                failed += 1;
                println!("FAILED {}: {e}", f.display());
            }
        }
    }
    println!("{} projects: {same} identical, {differ} differ, {failed} failed", files.len());
}
