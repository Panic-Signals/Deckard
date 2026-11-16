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

/// Maschine's favourites show as favourites, and its file is only ever read: one taken
/// off here stays off here, and stays in Maschine's.
#[test]
fn maschines_favorites_are_read_and_never_written() {
    let tmp = tempfile::tempdir().unwrap();
    let product = tmp.path().join("Maschine 3");
    std::fs::create_dir_all(&product).unwrap();
    let ni = testing::fixture(&product);
    let shared = tmp.path().join("Shared");
    std::fs::create_dir_all(&shared).unwrap();
    let file = shared.join("favorites.db3");
    {
        let db = rusqlite::Connection::open(&file).unwrap();
        db.execute_batch("CREATE TABLE favorites (id TEXT PRIMARY KEY); INSERT INTO favorites VALUES ('id-10'), ('id-unrelated');")
            .unwrap();
    }
    let before = std::fs::read(&file).unwrap();
    let mut lib = Library::new(Some(ni), &tmp.path().join("library.db3")).unwrap();
    let favorites = |lib: &mut Library| -> Vec<String> {
        lib.query(&Query { kind: Kind::Loop, favorites: true, ..Query::default() }).items.into_iter().map(|i| i.name).collect()
    };
    assert_eq!(favorites(&mut lib), ["Kick[120] Lagoon"]);
    let kick = PathBuf::from("/Users/Shared/Deep Matter Library/Samples/Loops/Kick[120] Lagoon.wav");
    lib.set_favorite(&kick, false);
    assert!(favorites(&mut lib).is_empty(), "taken off here");
    lib.set_favorite(&kick, true);
    assert_eq!(favorites(&mut lib), ["Kick[120] Lagoon"], "put back");
    assert_eq!(std::fs::read(&file).unwrap(), before, "Maschine's file is untouched");
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

/// A stand-in for the app's NKS reader: the file's text is the preset's name.
fn read_preset(p: &Path) -> Option<deckard_library::Item> {
    let name = String::from_utf8(std::fs::read(p).ok()?).ok()?;
    Some(deckard_library::Item { kind: Kind::Effect, name, types: vec![vec!["Reverb".into()]], ..Default::default() })
}

/// Files the index can't read itself (NKS presets) are read by the reader it's given,
/// in user folders only: under a library folder NI's own index lists them.
#[test]
fn a_reader_lists_presets_in_user_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let (user, factory) = (d.join("User Content"), d.join("Raum Library"));
    for (dir, name) in [(&user, "Mine"), (&factory, "Theirs")] {
        std::fs::create_dir_all(dir.join("Raum")).unwrap();
        std::fs::write(dir.join(format!("Raum/{name}.nksfx")), name).unwrap();
    }
    let roots = [user_root(&user), Root { path: factory.clone(), factory: true, product: Some("Raum".into()) }];
    let q = Query { kind: Kind::Effect, user: true, ..Query::default() };

    let mut ix = UserIndex::open(&d.join("library.db3")).unwrap();
    ix.scan(&roots).unwrap();
    assert!(ix.query(&q).unwrap().items.is_empty(), "no reader, not listed");

    ix.set_reader(read_preset);
    ix.scan(&roots).unwrap();
    let r = ix.query(&q).unwrap();
    assert_eq!(r.items.iter().map(|i| (i.name.as_str(), i.user)).collect::<Vec<_>>(), [("Mine", true)]);
    assert_eq!(r.items[0].types, [vec!["Reverb".to_string()]]);
    assert!(ix.query(&Query { kind: Kind::Effect, user: false, ..Query::default() }).unwrap().items.is_empty());

    std::fs::remove_file(user.join("Raum/Mine.nksfx")).unwrap();
    ix.scan(&roots).unwrap();
    assert!(ix.query(&q).unwrap().items.is_empty(), "dropped when deleted");
}

/// A preset DECKARD just saved is read the same way.
#[test]
fn a_saved_preset_is_read_by_the_reader() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let file = d.join("User Content/Raum/Mine.nksfx");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "Mine").unwrap();
    let mut lib = Library::new(None, &d.join("library.db3")).unwrap();
    lib.set_reader(read_preset);
    lib.add_saved(&file);
    let r = lib.query(&Query { kind: Kind::Effect, user: true, ..Query::default() });
    assert_eq!(r.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Mine"]);
}

/// Multi-snapshot ensembles (.ens/.rkplr) share one file path across hundreds of presets.
/// Favoriting or unfavoriting one snapshot must not smear onto siblings sharing the same path,
/// and NI's per-row favorite status must survive Library::query merging.
#[test]
fn ensemble_favorites_are_per_snapshot_without_smearing() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let product = d.join("Maschine 3");
    std::fs::create_dir_all(&product).unwrap();
    let ni_path = testing::fixture(&product);
    let db = rusqlite::Connection::open(&ni_path).unwrap();
    db.execute_batch(
        "ALTER TABLE k_content_path ADD COLUMN upid TEXT;
         ALTER TABLE k_sound_info ADD COLUMN uuid TEXT;
         INSERT INTO k_content_path (id, path, content_type, alias, upid)
             VALUES (3, 'C:/Users/Public/Documents/Monark', 2, 'Monark', 'monark-upid');
         INSERT INTO k_sound_info (id, name, author, tempo, content_path_id, file_name, hidden_for, uuid)
             VALUES
             (40, 'Bass Sting', 'Native Instruments', 0, 3, 'C:/Users/Public/Documents/Monark/Monark.ens', 0, '6d63bc67-8fd2-9fb4-786f-75f05c168407'),
             (41, 'Bello', 'Native Instruments', 0, 3, 'C:/Users/Public/Documents/Monark/Monark.ens', 0, 'aaaa1111-2222-3333-4444-555566667777');
         INSERT INTO p_sound_info_Instrument_2 (id, name, entry1, entry2, entry3, content_path_id, favorite_id)
             VALUES
             (40, 'Bass Sting', 'Monark', NULL, NULL, 3, 'fav-bass-sting'),
             (41, 'Bello', 'Monark', NULL, NULL, 3, 'fav-bello');",
    )
    .unwrap();
    drop(db);

    let shared = d.join("Shared");
    std::fs::create_dir_all(&shared).unwrap();
    let fav_db = shared.join("favorites.db3");
    let fdb = rusqlite::Connection::open(&fav_db).unwrap();
    fdb.execute_batch("CREATE TABLE favorites (id TEXT PRIMARY KEY); INSERT INTO favorites VALUES ('fav-bello');").unwrap();
    drop(fdb);

    let mut lib = Library::new(Some(ni_path), &d.join("library.db3")).unwrap();
    let q = Query { kind: Kind::Instrument, ..Query::default() };

    let r = lib.query(&q);
    assert_eq!(r.items.len(), 2);
    assert!(!r.items[0].favorite, "Bass Sting must not inherit Bello's NI favorite");
    assert!(r.items[1].favorite, "Bello should retain NI favorite");

    // Toggle Bass Sting ON via its per-snapshot favorite key: Bello stays true, Bass Sting becomes true.
    let key_sting = r.items[0].favorite_key();
    let key_bello = r.items[1].favorite_key();
    lib.set_favorite_key(&key_sting, true);
    let r = lib.query(&q);
    assert!(r.items[0].favorite);
    assert!(r.items[1].favorite);

    // Toggle Bello OFF: Bass Sting stays true, Bello becomes false via not_favorites key.
    lib.set_favorite_key(&key_bello, false);
    let r = lib.query(&q);
    assert!(r.items[0].favorite);
    assert!(!r.items[1].favorite);
}
