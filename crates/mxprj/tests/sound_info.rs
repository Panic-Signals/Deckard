//! The tag block every Maschine file carries (`NI::SOUND::SoundInfo2`, the `NISD/108`
//! item): read from NI's files, written back unchanged, and filled in by DECKARD.
//! Tests over installed content skip themselves when it is absent.

use mxprj::Project;
use mxprj::sound_info::SoundInfo;
use std::path::{Path, PathBuf};

const ZIKMODO: &str = "/Users/Shared/Velvet Lounge Library/Sounds/Drumsynth/OpenHH Zikmodo.mxsnd";
const PERCMAN: &str = "/Users/Shared/Expansions Selection Library/Groups/Kits/Percman Kit.mxgrp";

fn info_of(path: &str) -> Option<SoundInfo> {
    let bytes = std::fs::read(path).ok()?;
    Some(Project::parse(&bytes).unwrap().sound_info().expect("a sound info"))
}

#[test]
fn factory_sound_tags_read() {
    let Some(info) = info_of(ZIKMODO) else { return };
    assert_eq!(info.name, "OpenHH Zikmodo");
    assert_eq!(info.author, "Native Instruments");
    assert_eq!(info.bank, ["Velvet Lounge"]);
    assert_eq!(info.types(), [vec!["Drumsynth".to_string(), "Hihat Open".to_string()]]);
    assert!(info.characters().is_empty());
    assert_eq!(info.prop("\\@tempo"), Some("0"));
}

#[test]
fn factory_group_characters_read() {
    let Some(info) = info_of(PERCMAN) else { return };
    assert_eq!(info.bank, ["Expansions Selection", "Resonant Blaze"]);
    assert_eq!(info.characters(), ["in progress"]);
    assert_eq!(info.types()[0][0], "Kits");
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() && depth < 5 {
            collect(&p, out, depth + 1);
        } else if ft.is_file() && p.extension().is_some_and(|x| ["mxprj", "mxgrp", "mxsnd", "mxinst", "mxfx"].iter().any(|e| x == *e)) {
            out.push(p);
        }
    }
}

#[test]
fn sound_info_round_trips_over_the_corpus() {
    let mut files = Vec::new();
    collect(Path::new("/Users/Shared"), &mut files, 0);
    if let Some(home) = std::env::var_os("HOME") {
        collect(&Path::new(&home).join("Documents/Native Instruments"), &mut files, 0);
    }
    files.sort();
    // An even spread over the libraries, at most 300 files.
    let step = (files.len() / 300).max(1);
    let mut checked = 0;
    for f in files.iter().step_by(step) {
        let bytes = std::fs::read(f).unwrap();
        let Ok(p) = Project::parse(&bytes) else { continue };
        let info = p.sound_info().unwrap_or_else(|| panic!("{}: no sound info", f.display()));
        assert!(p.with_sound_info(&info) == bytes, "{}: not byte-identical", f.display());
        checked += 1;
    }
    eprintln!("{checked} files round-tripped");
}

#[test]
fn non_ascii_tags_round_trip() {
    let mut info = SoundInfo::default();
    info.name = "Café Ésprit".into();
    info.set_tags(&[vec!["Ambient".into(), "空".into()]], &["Warm".into()]);
    let back = SoundInfo::parse(&info.to_bytes()).unwrap();
    assert_eq!(back, info);
    assert_eq!(back.name, "Café Ésprit");
    assert_eq!(back.types(), [vec!["Ambient".to_string(), "空".to_string()]]);
    assert_eq!(back.characters(), ["Warm"]);
}

#[test]
fn tags_are_written_as_maschine_lists_them() {
    let mut info = SoundInfo::default();
    info.set_tags(&[vec!["Kits".into(), "Percussion".into()]], &["in progress".into()]);
    assert_eq!(info.attributes(), ["\\.in progress", "\\:Kits", "\\:Kits\\:Percussion"]);
}

#[test]
fn new_files_carry_an_empty_info() {
    let info = SoundInfo::parse(&mxprj::container::new_sound_info()).unwrap();
    assert_eq!(info.name, "");
    assert!(info.bank.is_empty() && info.types().is_empty() && info.characters().is_empty());
    assert_eq!(info.prop("\\@verl"), Some("1.7.14"));
}
