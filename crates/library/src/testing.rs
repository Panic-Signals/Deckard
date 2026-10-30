//! A small index with NI's schema (the tables and columns DECKARD reads), for tests:
//! three factory loops in "Deep Matter" (Drums › Kick "Clean" at 120 BPM, Drums ›
//! Snare, Bass), a user one-shot, a factory sound and a hidden one.

use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Write the fixture as `komplete.db3` in `dir`; returns its path.
pub fn fixture(dir: &Path) -> PathBuf {
    let path = dir.join("komplete.db3");
    let _ = std::fs::remove_file(&path);
    let db = Connection::open(&path).expect("fixture database");
    let mut sql = String::from(
        "CREATE TABLE k_content_path (id INTEGER PRIMARY KEY, path TEXT NOT NULL, content_type INTEGER NOT NULL DEFAULT 0, alias TEXT);
         CREATE TABLE k_sound_info (id INTEGER PRIMARY KEY, name TEXT NOT NULL, author TEXT, tempo REAL NOT NULL DEFAULT 0,
            content_path_id INTEGER NOT NULL, file_name TEXT NOT NULL, hidden_for INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE k_category (id INTEGER PRIMARY KEY, category TEXT, subcategory TEXT, subsubcategory TEXT);
         CREATE TABLE k_mode (id INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE k_sound_info_category (sound_info_id INTEGER NOT NULL, category_id INTEGER NOT NULL);
         CREATE TABLE k_sound_info_mode (sound_info_id INTEGER NOT NULL, mode_id INTEGER NOT NULL);
         INSERT INTO k_content_path VALUES (1, '/Users/Shared/Deep Matter Library', 2, 'Deep Matter'),
            (2, '/Users/me/Documents/Native Instruments/Maschine 3', 1, 'Standard User Directory');
         INSERT INTO k_category VALUES (1, 'Drums', NULL, NULL), (2, 'Drums', 'Kick', NULL), (3, 'Drums', 'Snare', NULL), (4, 'Bass', '', NULL);
         INSERT INTO k_mode VALUES (1, 'Clean');
         INSERT INTO k_sound_info VALUES
            (10, 'Kick[120] Lagoon', 'Native Instruments', 120, 1, '/Users/Shared/Deep Matter Library/Samples/Loops/Kick[120] Lagoon.wav', 0),
            (11, 'Snare[90] Dust', 'Native Instruments', 0, 1, '/Users/Shared/Deep Matter Library/Samples/Loops/Snare[90] Dust.wav', 0),
            (12, 'Bass[94] Amber', 'Native Instruments', 0, 1, '/Users/Shared/Deep Matter Library/Samples/Loops/Bass[94] Amber.wav', 0),
            (20, 'My Kick', '', 0, 2, '/Users/me/Documents/Native Instruments/Maschine 3/Samples/My Kick.wav', 0),
            (30, 'Shown Sound', 'Native Instruments', 0, 1, '/Users/Shared/Deep Matter Library/Sounds/Shown Sound.mxsnd', 0),
            (31, 'Hidden Sound', 'Native Instruments', 0, 1, '/Users/Shared/Deep Matter Library/Sounds/Hidden Sound.mxsnd', 1);
         INSERT INTO k_sound_info_category VALUES (10, 1), (10, 2), (11, 1), (11, 3), (12, 4);
         INSERT INTO k_sound_info_mode VALUES (10, 1);
        ",
    );
    for kind in crate::Kind::ALL {
        for n in [1, 2] {
            sql += &format!(
                "CREATE TABLE p_sound_info_{}_{n} (id INTEGER PRIMARY KEY, name TEXT NOT NULL, entry1 TEXT, entry2 TEXT, entry3 TEXT, \
                 content_path_id INTEGER NOT NULL);\n",
                kind.table()
            );
        }
    }
    sql += "INSERT INTO p_sound_info_Loop_2 SELECT id, name, 'Deep Matter', 'Loops', NULL, content_path_id FROM k_sound_info WHERE id IN (10, 11, 12);
            INSERT INTO p_sound_info_Oneshot_1 SELECT id, name, NULL, NULL, NULL, content_path_id FROM k_sound_info WHERE id = 20;
            INSERT INTO p_sound_info_Sound_2 SELECT id, name, 'Deep Matter', NULL, NULL, content_path_id FROM k_sound_info WHERE id IN (30, 31);";
    db.execute_batch(&sql).expect("fixture tables");
    path
}

/// A Maschine file at `path` (an empty document) carrying `name` and these tags, as
/// Maschine writes them.
pub fn tagged_file(path: &Path, name: &str, types: &[&[&str]], characters: &[&str]) {
    // The smallest archive Maschine's reader takes: its signature, library version 10, flags.
    let archive = b"\x01\x16serialization::archive\x01\x0a\x00";
    let mut n = 0u8;
    let file = mxprj::container::new_project_file(archive, "3.0.0", &mut || {
        n += 1;
        [n; 16]
    });
    let mut info = mxprj::sound_info::SoundInfo::default();
    info.name = name.to_string();
    let types: Vec<Vec<String>> = types.iter().map(|t| t.iter().map(|s| s.to_string()).collect()).collect();
    let characters: Vec<String> = characters.iter().map(|s| s.to_string()).collect();
    info.set_tags(&types, &characters);
    let file = mxprj::Project::parse(&file).expect("a new file").with_sound_info(&info);
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("its folder");
    std::fs::write(path, file).expect("the file");
}
