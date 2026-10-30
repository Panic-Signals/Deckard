//! mxprj-coverage: decode every Maschine document under some directories and
//! report how many decode completely, and where the rest stop.
//!
//! Usage: mxprj-coverage [--ext mxgrp] [--top N] [--show-file] [dir ...]
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut exts: Vec<String> = EXTS.iter().map(|s| s.to_string()).collect();
    let mut top = 15;
    let mut show_file = false;
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
            "--show-file" => show_file = true,
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
    // ext -> (ok, total)
    let mut per_ext: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    // "type: error" -> (count, example file, offset)
    let mut fails: BTreeMap<String, (usize, String, usize)> = BTreeMap::new();
    for f in &files {
        let ext = f.extension().unwrap().to_string_lossy().into_owned();
        let e = per_ext.entry(ext).or_default();
        e.1 += 1;
        let Ok(bytes) = std::fs::read(f) else { continue };
        let result = match Project::parse(&bytes) {
            Ok(p) => p.decode(&types).map(drop),
            Err(err) => Err((err, "<container>".to_string())),
        };
        match result {
            Ok(()) => e.0 += 1,
            Err((err, path)) => {
                let last = path.rsplit(" > ").next().unwrap_or("").to_string();
                let key = format!("{last}: {}", err.what);
                let slot = fails.entry(key).or_insert((0, f.display().to_string(), err.offset));
                slot.0 += 1;
            }
        }
    }
    println!("decoded completely:");
    for (ext, (ok, total)) in &per_ext {
        println!("  {ext:7} {ok:5} / {total}");
    }
    let mut v: Vec<_> = fails.into_iter().collect();
    v.sort_by_key(|b| std::cmp::Reverse(b.1.0));
    println!("top stopping points:");
    for (k, (n, file, off)) in v.iter().take(top) {
        println!("  {n:5}  {k}");
        if show_file {
            println!("         e.g. {file} @ {off:#x}");
        }
    }
}
