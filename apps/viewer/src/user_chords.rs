//! The user chords Chords mode plays (Chord MODE User Chord): 16 sets of 12 chords, each
//! the semitones above the root it plays. DECKARD keeps them in its own settings folder,
//! a file a set (`User Chords/Set 1.json`: `{ "chords": [[0, 4, 7], [], …] }`), never
//! in Maschine's.

use std::path::{Path, PathBuf};

pub const SETS: usize = 16;
pub const CHORDS: usize = 12;

#[derive(Debug, Clone, PartialEq)]
pub struct UserChords {
    /// `sets[set][slot]`: the chord's notes above the root, lowest first.
    pub sets: Vec<Vec<Vec<i64>>>,
    /// Where they're kept (none: not kept).
    dir: Option<PathBuf>,
}

impl Default for UserChords {
    fn default() -> UserChords {
        UserChords { sets: vec![vec![Vec::new(); CHORDS]; SETS], dir: None }
    }
}

impl UserChords {
    /// The user chords kept in the settings folder.
    pub fn load() -> UserChords {
        crate::menu::settings_dir().map_or_else(UserChords::default, |d| UserChords::load_from(&d.join("User Chords")))
    }

    /// The user chords kept in `dir` (empty sets for missing or unreadable files).
    pub fn load_from(dir: &Path) -> UserChords {
        let mut u = UserChords { dir: Some(dir.to_path_buf()), ..UserChords::default() };
        for (i, set) in u.sets.iter_mut().enumerate() {
            let Ok(text) = std::fs::read_to_string(dir.join(file_name(i))) else { continue };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            let Some(chords) = json.get("chords").and_then(|c| c.as_array()) else { continue };
            for (slot, c) in set.iter_mut().zip(chords) {
                let mut notes: Vec<i64> = c.as_array().into_iter().flatten().filter_map(serde_json::Value::as_i64).collect();
                notes.sort_unstable();
                notes.dedup();
                *slot = notes;
            }
        }
        u
    }

    fn chord(&mut self, set: usize, slot: usize) -> Option<&mut Vec<i64>> {
        self.sets.get_mut(set)?.get_mut(slot)
    }

    /// Add `note` to a chord, or take it out if it's there.
    pub fn toggle(&mut self, set: usize, slot: usize, note: i64) {
        let Some(c) = self.chord(set, slot) else { return };
        match c.binary_search(&note) {
            Ok(i) => {
                c.remove(i);
            }
            Err(i) => c.insert(i, note),
        }
        self.save(set);
    }

    pub fn delete(&mut self, set: usize, slot: usize) {
        let Some(c) = self.chord(set, slot) else { return };
        c.clear();
        self.save(set);
    }

    /// Swap a chord with the one before or after (`next`) it.
    pub fn move_chord(&mut self, set: usize, slot: usize, next: bool) {
        let Some(to) = (if next { slot.checked_add(1) } else { slot.checked_sub(1) }).filter(|&t| t < CHORDS) else { return };
        let Some(s) = self.sets.get_mut(set) else { return };
        if slot < CHORDS {
            s.swap(slot, to);
            self.save(set);
        }
    }

    fn save(&self, set: usize) {
        let (Some(dir), Some(chords)) = (&self.dir, self.sets.get(set)) else { return };
        let json = serde_json::json!({ "chords": chords });
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(dir.join(file_name(set)), format!("{json}\n"));
    }
}

fn file_name(set: usize) -> String {
    format!("Set {}.json", set + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_chords_are_kept_a_file_a_set() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("chords");
        let mut u = UserChords::load_from(&dir);
        u.toggle(2, 0, 7);
        u.toggle(2, 0, 0);
        u.toggle(2, 0, 4);
        u.toggle(2, 1, 3);
        u.toggle(2, 1, 3);
        assert_eq!(u.sets[2][0], [0, 4, 7], "lowest first");
        assert!(u.sets[2][1].is_empty(), "a second press takes a note out");
        u.move_chord(2, 0, true);
        assert_eq!((u.sets[2][0].len(), u.sets[2][1].len()), (0, 3));
        u.move_chord(2, 11, true);
        assert_eq!(UserChords::load_from(&dir).sets, u.sets);
        assert!(dir.join("Set 3.json").exists());
        u.delete(2, 1);
        assert!(UserChords::load_from(&dir).sets[2][1].is_empty());
    }
}
