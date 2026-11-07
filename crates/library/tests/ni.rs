//! NI's library index, read through a fixture with its schema (never the real one).

use deckard_library::ni::NiIndex;
use deckard_library::{Kind, Query, testing};
use std::time::{Duration, Instant};

fn loops(types: &[&str]) -> Query {
    Query { kind: Kind::Loop, types: types.iter().map(|s| s.to_string()).collect(), ..Query::default() }
}

#[test]
fn factory_loops_filter_by_type_and_character() {
    let tmp = tempfile::tempdir().unwrap();
    let ni = NiIndex::open(&testing::fixture(tmp.path())).unwrap();
    let r = ni.query(&loops(&["Drums"])).unwrap();
    assert_eq!(r.items.len(), 2);
    assert_eq!(r.types, [("Kick".to_string(), 1), ("Snare".to_string(), 1)]);
    let clean = Query { characters: vec!["Clean".into()], ..loops(&["Drums"]) };
    let r = ni.query(&clean).unwrap();
    assert_eq!(r.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Kick[120] Lagoon"]);
    assert_eq!(r.items[0].types, [vec!["Drums".to_string(), "Kick".to_string()]]);
    assert_eq!(r.items[0].tempo, Some(120.0));
    assert_eq!(r.items[0].product.as_deref(), Some("Deep Matter"));
}

#[test]
fn top_level_types_and_characters_are_facets() {
    let tmp = tempfile::tempdir().unwrap();
    let ni = NiIndex::open(&testing::fixture(tmp.path())).unwrap();
    let r = ni.query(&loops(&[])).unwrap();
    assert_eq!(r.items.len(), 3);
    assert_eq!(r.types, [("Bass".to_string(), 1), ("Drums".to_string(), 2)]);
    assert_eq!(r.characters, [("Clean".to_string(), 1)]);
}

#[test]
fn user_switch_separates_user_items() {
    let tmp = tempfile::tempdir().unwrap();
    let ni = NiIndex::open(&testing::fixture(tmp.path())).unwrap();
    let r = ni.query(&Query { kind: Kind::OneShot, user: true, ..Query::default() }).unwrap();
    assert_eq!(r.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["My Kick"]);
    assert!(r.items[0].user);
    let r = ni.query(&Query { kind: Kind::OneShot, ..Query::default() }).unwrap();
    assert!(r.items.is_empty());
}

#[test]
fn text_search_matches_name() {
    let tmp = tempfile::tempdir().unwrap();
    let ni = NiIndex::open(&testing::fixture(tmp.path())).unwrap();
    let r = ni.query(&Query { text: "snare".into(), ..loops(&[]) }).unwrap();
    assert_eq!(r.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Snare[90] Dust"]);
}

#[test]
fn a_product_maschine_marks_as_gone_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let path = testing::fixture(tmp.path());
    assert_eq!(NiIndex::open(&path).unwrap().query(&loops(&[])).unwrap().products, ["Deep Matter"]);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("UPDATE k_content_path SET state = 4 WHERE alias = 'Deep Matter'", []).unwrap();
    drop(db);
    let r = NiIndex::open(&path).unwrap().query(&loops(&[])).unwrap();
    assert!(r.items.is_empty() && r.products.is_empty(), "{:?}", r.items);
    // The user's own content isn't part of it.
    let own = NiIndex::open(&path).unwrap().query(&Query { kind: Kind::OneShot, user: true, ..Query::default() }).unwrap();
    assert_eq!(own.items.len(), 1);
}

#[test]
fn hidden_items_are_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let ni = NiIndex::open(&testing::fixture(tmp.path())).unwrap();
    let r = ni.query(&Query { kind: Kind::Sound, ..Query::default() }).unwrap();
    assert!(r.items.iter().all(|i| i.name != "Hidden Sound"));
}

#[test]
fn products_list_the_aliases() {
    let tmp = tempfile::tempdir().unwrap();
    let ni = NiIndex::open(&testing::fixture(tmp.path())).unwrap();
    let r = ni.query(&loops(&[])).unwrap();
    assert_eq!(r.products, ["Deep Matter"]);
    let r = ni.query(&Query { product: Some("Other Pack".into()), ..loops(&[]) }).unwrap();
    assert!(r.items.is_empty());
}

#[test]
fn instrument_products_use_bank_chain_entry1() {
    let tmp = tempfile::tempdir().unwrap();
    let path = testing::fixture(tmp.path());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "INSERT INTO k_sound_info VALUES
            (40, 'Acid Lead', 'Native Instruments', 0, 1, '/Users/Shared/Deep Matter Library/Presets/Acid Lead.nksf', 0),
            (41, 'User Pad', '', 0, 2, '/Users/me/Documents/Native Instruments/User Content/Omnisphere/User Pad.nksf', 0);
         INSERT INTO p_sound_info_Instrument_2 VALUES (40, 'Acid Lead', 'Massive X', 'Deep Matter', NULL, 1, NULL);
         INSERT INTO p_sound_info_Instrument_1 VALUES (41, 'User Pad', 'Omnisphere', NULL, NULL, 2, NULL);",
    )
    .unwrap();
    drop(db);
    let ni = NiIndex::open(&path).unwrap();
    let factory = ni.query(&Query { kind: Kind::Instrument, ..Query::default() }).unwrap();
    assert_eq!(factory.products, ["Massive X"]);
    assert_eq!(factory.items[0].product.as_deref(), Some("Massive X"));
    let user = ni.query(&Query { kind: Kind::Instrument, user: true, ..Query::default() }).unwrap();
    assert_eq!(user.products, ["Omnisphere"]);
    assert_eq!(user.items[0].product.as_deref(), Some("Omnisphere"));
}

/// Maschine holding its index (rescanning) makes a query fail quickly, not hang;
/// the next one works.
#[test]
fn busy_index_keeps_last_results() {
    let tmp = tempfile::tempdir().unwrap();
    let path = testing::fixture(tmp.path());
    let lock = rusqlite::Connection::open(&path).unwrap();
    lock.pragma_update(None, "journal_mode", "WAL").unwrap();
    lock.pragma_update(None, "locking_mode", "EXCLUSIVE").unwrap();
    lock.execute_batch("BEGIN EXCLUSIVE; UPDATE k_sound_info SET name = name;").unwrap();
    let ni = NiIndex::open(&path);
    let t = Instant::now();
    assert!(ni.as_ref().map_or(true, |ni| ni.query(&loops(&[])).is_err()));
    // The busy timeout is 500ms; loaded CI runners oversleep it, so leave headroom.
    assert!(t.elapsed() < Duration::from_secs(3));
    lock.execute_batch("ROLLBACK").unwrap();
    drop(lock);
    assert_eq!(NiIndex::open(&path).unwrap().query(&loops(&[])).unwrap().items.len(), 3);
}

/// Maschine starting while DECKARD has its index open: the next query reads through
/// Maschine's WAL and sees what it wrote.
#[test]
fn a_later_maschine_write_is_seen() {
    let tmp = tempfile::tempdir().unwrap();
    let path = testing::fixture(tmp.path());
    let ni = NiIndex::open(&path).unwrap();
    assert_eq!(ni.query(&loops(&[])).unwrap().items.len(), 3);
    let maschine = rusqlite::Connection::open(&path).unwrap();
    maschine.pragma_update(None, "journal_mode", "WAL").unwrap();
    maschine.execute_batch("DELETE FROM p_sound_info_Loop_2 WHERE id = 12").unwrap();
    assert_eq!(ni.query(&loops(&[])).unwrap().items.len(), 2);
}

#[test]
fn index_is_opened_read_only() {
    let tmp = tempfile::tempdir().unwrap();
    let path = testing::fixture(tmp.path());
    let before = std::fs::read(&path).unwrap();
    let ni = NiIndex::open(&path).unwrap();
    for _ in 0..10 {
        ni.query(&loops(&["Drums"])).unwrap();
    }
    drop(ni);
    assert!(std::fs::read(&path).unwrap() == before);
}

/// Maschine keeps its index in WAL mode; with Maschine closed (no `-wal` beside it),
/// reading it leaves nothing behind in NI's folder.
#[test]
fn opening_creates_no_files_beside_the_index() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let path = testing::fixture(&d);
    rusqlite::Connection::open(&path).unwrap().pragma_update(None, "journal_mode", "WAL").unwrap();
    let before: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(before.len(), 1);
    let ni = NiIndex::open(&path).unwrap();
    assert_eq!(ni.query(&loops(&[])).unwrap().items.len(), 3);
    drop(ni);
    let after: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(after, before);
}
