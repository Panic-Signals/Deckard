//! DECKARD's own index (Maschine's user folders and what DECKARD saves) and the merged
//! library. Temp dirs only.

use deckard_library::user::{Root, UserIndex};
use deckard_library::{Kind, Library, Query, testing};
use std::path::{Path, PathBuf};

fn user_root(p: &Path) -> Root {
    Root { path: p.to_path_buf(), factory: false, product: None }
}

fn touch(p: &Path) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, b"RIFF").unwrap();
}

#[test]
fn scan_reads_tags_from_maschine_files() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let root = d.join("Maschine 3");
    testing::tagged_file(&root.join("Sounds/Big Kick.mxsnd"), "Big Kick", &[&["Drums", "Kick"]], &["Punchy"]);
    let mut ix = UserIndex::open(&d.join("library.db3")).unwrap();
    assert_eq!(ix.scan(&[user_root(&root)]).unwrap(), 1);
    let r = ix.query(&Query { kind: Kind::Sound, user: true, ..Query::default() }).unwrap();
    assert_eq!(r.items.len(), 1);
    let i = &r.items[0];
    assert_eq!((i.name.as_str(), i.user), ("Big Kick", true));
    assert_eq!(i.types, [vec!["Drums".to_string(), "Kick".to_string()]]);
    assert_eq!(i.characters, ["Punchy"]);
    assert_eq!(r.types, [("Drums".to_string(), 1)]);
}

#[test]
fn samples_split_by_tempo_in_name() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let root = d.join("Maschine 3");
    touch(&root.join("Samples/Bass[94] Amber.wav"));
    touch(&root.join("Samples/kick.wav"));
    touch(&root.join("Samples/notes.txt"));
    let mut ix = UserIndex::open(&d.join("library.db3")).unwrap();
    ix.scan(&[user_root(&root)]).unwrap();
    let loops = ix.query(&Query { kind: Kind::Loop, user: true, ..Query::default() }).unwrap();
    assert_eq!(loops.items.iter().map(|i| (i.name.as_str(), i.tempo)).collect::<Vec<_>>(), [("Bass[94] Amber", Some(94.0))]);
    let shots = ix.query(&Query { kind: Kind::OneShot, user: true, ..Query::default() }).unwrap();
    assert_eq!(shots.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["kick"]);
}

#[test]
fn rescan_drops_deleted_files() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let root = d.join("Maschine 3");
    let kick = root.join("Samples/kick.wav");
    touch(&kick);
    let mut ix = UserIndex::open(&d.join("library.db3")).unwrap();
    ix.scan(&[user_root(&root)]).unwrap();
    std::fs::remove_file(&kick).unwrap();
    assert_eq!(ix.scan(&[user_root(&root)]).unwrap(), 0);
    assert!(ix.query(&Query { kind: Kind::OneShot, user: true, ..Query::default() }).unwrap().items.is_empty());
}

#[test]
fn saved_file_appears_without_rescan() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let mut lib = Library::new(None, &d.join("library.db3")).unwrap();
    let f = d.join("Maschine 3/Groups/My Kit.mxgrp");
    testing::tagged_file(&f, "My Kit", &[&["Kits"]], &[]);
    lib.add_saved(&f);
    let r = lib.query(&Query { kind: Kind::Group, user: true, ..Query::default() });
    assert_eq!(r.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["My Kit"]);
}

#[test]
fn merged_results_prefer_own_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let ni = testing::fixture(&d);
    let mut lib = Library::new(Some(ni), &d.join("library.db3")).unwrap();
    // The same file NI lists, also in DECKARD's index (as a factory folder).
    let path = PathBuf::from("/Users/Shared/Deep Matter Library/Samples/Loops/Kick[120] Lagoon.wav");
    lib.add_entry(deckard_library::Item {
        kind: Kind::Loop,
        name: "Kick[120] Lagoon".into(),
        path: path.clone(),
        tempo: Some(120.0),
        ..Default::default()
    });
    let r = lib.query(&Query { kind: Kind::Loop, ..Query::default() });
    assert_eq!(r.items.iter().filter(|i| i.path == path).count(), 1);
    assert!(r.items.iter().find(|i| i.path == path).unwrap().types.is_empty(), "DECKARD's entry wins");
    assert_eq!(r.items.len(), 3);
}

#[test]
fn favorites_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let ni = testing::fixture(&d);
    let mut lib = Library::new(Some(ni), &d.join("library.db3")).unwrap();
    let snare = PathBuf::from("/Users/Shared/Deep Matter Library/Samples/Loops/Snare[90] Dust.wav");
    lib.set_favorite(&snare, true);
    let r = lib.query(&Query { kind: Kind::Loop, favorites: true, ..Query::default() });
    assert_eq!(r.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Snare[90] Dust"]);
    assert!(r.items[0].favorite);
    lib.set_favorite(&snare, false);
    assert!(lib.query(&Query { kind: Kind::Loop, favorites: true, ..Query::default() }).items.is_empty());
}

#[test]
fn a_factory_folder_lists_under_factory_with_its_product() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let root = d.join("Retro Pack Library");
    testing::tagged_file(&root.join("Sounds/Tape Snare.mxsnd"), "Tape Snare", &[&["Drums", "Snare"]], &[]);
    let mut ix = UserIndex::open(&d.join("library.db3")).unwrap();
    ix.scan(&[Root { path: root, factory: true, product: Some("Retro Pack".into()) }]).unwrap();
    let r = ix.query(&Query { kind: Kind::Sound, ..Query::default() }).unwrap();
    assert_eq!(r.items[0].product.as_deref(), Some("Retro Pack"));
    assert_eq!(r.products, ["Retro Pack"]);
    assert!(ix.query(&Query { kind: Kind::Sound, user: true, ..Query::default() }).unwrap().items.is_empty());
}
