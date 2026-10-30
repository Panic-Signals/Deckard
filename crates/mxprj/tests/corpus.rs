//! Parse every `.mxprj` under `$MXPRJ_CORPUS` (default `/Users/Shared`).
//! Skips (passes) if no project files are found, so CI without NI content still runs.

use mxprj::Project;
use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() && depth < 4 {
            collect(&p, out, depth + 1);
        } else if ft.is_file() && p.extension().is_some_and(|x| x == "mxprj") {
            out.push(p);
        }
    }
}

#[test]
fn corpus_parses() {
    let root = std::env::var_os("MXPRJ_CORPUS").map_or_else(|| PathBuf::from("/Users/Shared"), PathBuf::from);
    let mut files = Vec::new();
    collect(&root, &mut files, 0);
    if files.is_empty() {
        eprintln!("no .mxprj files under {}; skipping", root.display());
        return;
    }
    let mut failures = Vec::new();
    let mut versions = std::collections::BTreeMap::<u64, usize>::new();
    let mut name_matches = 0;
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        match Project::parse(&bytes) {
            Ok(p) => {
                *versions.entry(p.header.library_version).or_default() += 1;
                match p.project_header() {
                    Ok(h) if f.file_stem().is_some_and(|s| s.to_string_lossy() == h.name) => name_matches += 1,
                    Ok(_) => {}
                    Err(e) => failures.push(format!("{}: project header: {e}", f.display())),
                }
            }
            Err(e) => failures.push(format!("{}: {e}", f.display())),
        }
    }
    eprintln!(
        "{} files, {} failures, boost library versions {versions:?}, {name_matches} project names equal the file name",
        files.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every document (projects, groups, sounds, instrument and effect presets) under
/// `$MXPRJ_CORPUS` decodes and is written back byte for byte.
#[test]
fn corpus_round_trips() {
    let root = std::env::var_os("MXPRJ_CORPUS").map_or_else(|| PathBuf::from("/Users/Shared"), PathBuf::from);
    let mut files = Vec::new();
    collect_all(&root, &mut files, 0);
    if files.is_empty() {
        eprintln!("no Maschine documents under {}; skipping", root.display());
        return;
    }
    // Spread over threads; each needs its own loader table (not Send).
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let failures: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = files
            .chunks(files.len().div_ceil(threads))
            .map(|chunk| scope.spawn(move || chunk.iter().filter_map(|f| round_trip(f).err()).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    eprintln!("{} documents, {} do not round-trip", files.len(), failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn round_trip(f: &Path) -> Result<(), String> {
    thread_local!(static TYPES: mxprj::types::Types = mxprj::types::Types::new());
    let bytes = std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
    let p = Project::parse(&bytes).map_err(|e| format!("{}: {e}", f.display()))?;
    TYPES.with(|types| {
        let root = p.decode(types).map_err(|(e, at)| format!("{}: decode: {e} in {at}", f.display()))?;
        match p.encode(types, root) {
            Ok(out) if out == bytes => Ok(()),
            Ok(_) => Err(format!("{}: written bytes differ", f.display())),
            Err((e, at)) => Err(format!("{}: encode: {e} in {at}", f.display())),
        }
    })
}

fn collect_all(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() && depth < 8 {
            collect_all(&p, out, depth + 1);
        } else if ft.is_file() && p.extension().is_some_and(|x| ["mxprj", "mxgrp", "mxsnd", "mxinst", "mxfx"].iter().any(|e| x == *e)) {
            out.push(p);
        }
    }
}
