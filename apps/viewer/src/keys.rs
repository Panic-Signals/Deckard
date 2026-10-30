//! The computer keyboard as a MIDI keyboard (Keys, ⌘K), as Maschine's (software
//! manual, "Using your computer keyboard as a MIDI keyboard"): the two rows A W S E D F
//! T G Y H U J K O L P ; ' play 18 keys, C to F an octave and a half up, from C3; Z / X
//! move them an octave down / up, C / V the velocity down / up. They play the focused
//! sound at their pitch, and REC records them as it records the pads.

use eframe::egui::Key;

/// The keys of the two rows, in pitch order from the bottom C.
const ROW: [Key; 18] = [
    Key::A,
    Key::W,
    Key::S,
    Key::E,
    Key::D,
    Key::F,
    Key::T,
    Key::G,
    Key::Y,
    Key::H,
    Key::U,
    Key::J,
    Key::K,
    Key::O,
    Key::L,
    Key::P,
    Key::Semicolon,
    Key::Quote,
];

/// Semitones above the range's bottom key that `key` plays.
pub fn offset(key: Key) -> Option<i64> {
    ROW.iter().position(|k| *k == key).map(|i| i as i64)
}

/// Velocity steps (C / V).
const VELOCITY_STEP: i64 = 16;

pub struct Keys {
    pub on: bool,
    /// The bottom key's pitch (C3 = 60).
    pub base: i64,
    pub velocity: i64,
    /// Pitches held, by the key holding each.
    pub held: Vec<(Key, i64)>,
}

impl Default for Keys {
    fn default() -> Keys {
        Keys { on: false, base: 60, velocity: 100, held: Vec::new() }
    }
}

/// What a key does with Keys on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAct {
    NoteOn(i64, i64),
    NoteOff(i64),
    /// The range or the velocity moved (shown briefly).
    Changed,
}

impl Keys {
    /// A key pressed (`down`) or released: the notes to start or end, or a setting.
    pub fn key(&mut self, key: Key, down: bool) -> Option<KeyAct> {
        if let Some(o) = offset(key) {
            let pitch = (self.base + o).clamp(0, 127);
            return if down {
                if self.held.iter().any(|(k, _)| *k == key) {
                    return None;
                }
                self.held.push((key, pitch));
                Some(KeyAct::NoteOn(pitch, self.velocity))
            } else {
                let i = self.held.iter().position(|(k, _)| *k == key)?;
                Some(KeyAct::NoteOff(self.held.remove(i).1))
            };
        }
        if !down {
            return None;
        }
        match key {
            Key::Z => self.base = (self.base - 12).max(0),
            Key::X => self.base = (self.base + 12).min(120),
            Key::C => self.velocity = (self.velocity - VELOCITY_STEP).max(1),
            Key::V => self.velocity = (self.velocity + VELOCITY_STEP).min(127),
            _ => return None,
        }
        Some(KeyAct::Changed)
    }

    /// What the overlay says: the range and the velocity.
    pub fn summary(&self) -> String {
        format!("Keys: {} – {}, velocity {}", note_name(self.base), note_name(self.base + 17), self.velocity)
    }
}

/// "C3" for 60 (Maschine's names).
pub fn note_name(pitch: i64) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[pitch.rem_euclid(12) as usize], pitch.div_euclid(12) - 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_play_eighteen_keys_from_c3() {
        assert_eq!(offset(Key::A), Some(0));
        assert_eq!(offset(Key::W), Some(1));
        assert_eq!(offset(Key::K), Some(12));
        assert_eq!(offset(Key::Quote), Some(17));
        assert_eq!(offset(Key::Q), None);
        let k = Keys::default();
        assert_eq!(k.summary(), "Keys: C3 – F4, velocity 100");
    }

    #[test]
    fn keys_hold_notes_and_move_the_range_and_velocity() {
        let mut k = Keys::default();
        assert_eq!(k.key(Key::S, true), Some(KeyAct::NoteOn(62, 100)));
        assert_eq!(k.key(Key::S, true), None, "a held key's repeat plays nothing");
        assert_eq!(k.key(Key::S, false), Some(KeyAct::NoteOff(62)));
        assert_eq!(k.key(Key::Z, true), Some(KeyAct::Changed));
        assert_eq!(k.key(Key::A, true), Some(KeyAct::NoteOn(48, 100)));
        // A note keeps its pitch to its release even if the range moves meanwhile.
        k.key(Key::X, true);
        k.key(Key::X, true);
        assert_eq!(k.key(Key::A, false), Some(KeyAct::NoteOff(48)));
        k.key(Key::V, true);
        assert_eq!(k.velocity, 116);
        for _ in 0..10 {
            k.key(Key::C, true);
        }
        assert_eq!(k.velocity, 1);
    }
}
