//! mxprj-roundtrip: decode every Maschine document under some directories, write it
//! back with the encoder, and check the bytes are identical.
//!
//! Usage: mxprj-roundtrip [--ext mxgrp] [--top N] [dir ...]
//! Default dirs: /Users/Shared and "/Library/Application Support/Native Instruments".

use mxprj::Project;
use mxprj::types::Types;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const EXTS: &[&str] = &["mxprj", "mxgrp", "mxsnd", "mxinst", "mxfx"];

fn collect(dir: &Path, exts: &[String], out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() && depth < 8 {
            collect(&p, exts, out, depth + 1);
        } else if ft.is_file() && p.extension().is_some_and(|x| exts.iter().any(|e| x == e.as_str())) {
            out.push(p);
        }
    }
}

/// Why a file does not round-trip, or None if it does. Files that don't decode are
/// skipped (`Some(None)` is not used): they report `Err`.
fn check(types: &Types, bytes: &[u8]) -> Result<Option<String>, String> {
    let p = Project::parse(bytes).map_err(|e| format!("parse: {e}"))?;
    if p.with_archive(p.archive) != bytes {
        return Ok(Some("container: rewriting with the same archive changes the file".into()));
    }
    let root = p.decode(types).map_err(|(e, at)| format!("decode: {e} in {at}"))?;
    let out = match p.encode(types, root) {
        Ok(o) => o,
        Err((e, at)) => return Ok(Some(format!("encode: {e} in {at}"))),
    };
    if out == bytes {
        return Ok(None);
    }
    let q = Project::parse(&out).map_err(|e| format!("output does not parse: {e}"))?;
    let (a, b) = (p.archive, q.archive);
    let at = a.iter().zip(b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
    let ctx = |s: &[u8]| s[at.saturating_sub(8)..(at + 8).min(s.len())].iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ");
    Ok(Some(format!("archive differs at {at:#x} of {:#x} (output {:#x}): file [{}] vs ours [{}]", a.len(), b.len(), ctx(a), ctx(b))))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut exts: Vec<String> = EXTS.iter().map(|s| s.to_string()).collect();
    let mut top = 10;
    let mut dirs = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ext" => {
                exts = args[i + 1].split(',').map(String::from).collect();
                i += 1;
            }
            "--top" => {
                top = args[i + 1].parse().expect("--top N");
                i += 1;
            }
            d => dirs.push(PathBuf::from(d)),
        }
        i += 1;
    }
    if dirs.is_empty() {
        dirs = vec!["/Users/Shared".into(), "/Library/Application Support/Native Instruments".into()];
    }
    let mut files = Vec::new();
    for d in &dirs {
        collect(d, &exts, &mut files, 0);
    }
    files.sort();

    let types = Types::new();
    // ext -> (identical, differ, not decoded)
    let mut per_ext: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    let mut reasons: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for f in &files {
        let ext = f.extension().unwrap().to_string_lossy().into_owned();
        let e = per_ext.entry(ext).or_default();
        let Ok(bytes) = std::fs::read(f) else { continue };
        match check(&types, &bytes) {
            Ok(None) => e.0 += 1,
            Ok(Some(why)) => {
                e.1 += 1;
                // group by the reason without offsets
                let key = why.split(" at ").next().unwrap_or(&why).to_string();
                let key = if key.starts_with("archive differs") { "archive differs".to_string() } else { key };
                reasons.entry(key).or_default().push(f.clone());
                if reasons.values().map(Vec::len).sum::<usize>() <= top {
                    eprintln!("{}: {why}", f.display());
                }
            }
            Err(_) => e.2 += 1,
        }
    }
    for (ext, (same, diff, skipped)) in &per_ext {
        println!("{ext:>7}: {same} identical, {diff} differ, {skipped} not decoded");
    }
    for (why, fs) in &reasons {
        println!("{:>5}  {why}", fs.len());
    }
    if !reasons.is_empty() {
        std::process::exit(1);
    }
}
