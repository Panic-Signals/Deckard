//! How Maschine spells notes in a scale (`ScaleHelper::getNoteName`), from its tables
//! ([`crate::scale_tables`]; docs/scales.md). A scale is its place in the tables:
//! bank * 15 + its place in the bank. Notes are named with MIDI note 60 as C3.

use crate::scale_tables as t;

/// A note's name in scale `scale` played from `root`: sharps in the chromatic scale,
/// the root from the mixed list, the other notes by their degree of the root's major
/// scale. Notes outside the scale have no name.
pub fn note_name(scale: usize, root: i64, note: i64) -> String {
    if !(0..=127).contains(&root) || !(0..=127).contains(&note) || scale >= t::SNAP.len() {
        return String::new();
    }
    let octave = note / 12 - 2;
    if scale == 0 {
        return format!("{}{octave}", t::NOTES_SHARP[(note % 12) as usize]);
    }
    let r = (root % 12) as usize;
    if root == note {
        return format!("{}{octave}", t::NOTES_MIXED[r]);
    }
    let k = (note % 12 - r as i64).rem_euclid(12) as usize;
    if t::SNAP[scale][k] != 0 {
        return String::new();
    }
    let nth = (0..k).filter(|&i| t::SNAP[scale][i] == 0).count();
    let Some(&(degree, accidental)) = t::DEGREES[scale].get(nth) else { return String::new() };
    let letter = (t::MAJOR_ROOT_DEGREE_OFFSETS[r] as usize + degree as usize) % 7;
    let shift = accidental as i64 + t::MAJOR_SCALE_INCREMENTS[r][degree as usize] as i64;
    match shift {
        // Double sharps and flats are named as the sharp note they sound.
        2 => format!("{}{octave}", t::NOTES_SHARP[(t::MAJOR_SEMITONES[letter] as usize + 2) % 12]),
        -2 => format!("{}{octave}", t::NOTES_SHARP[(t::MAJOR_SEMITONES[letter] as usize + 10) % 12]),
        // B# belongs to the octave below the C it sounds as.
        _ => format!("{}{}{}", t::LETTERS[letter], t::SUFFIXES[(shift + 1) as usize], octave - (shift == 1 && letter == 6) as i64),
    }
}

/// The root note's name: sharps for the chromatic scale, else the mixed spelling.
pub fn root_name(scale: usize, note: i64) -> String {
    let names = if scale == 0 { &t::NOTES_SHARP } else { &t::NOTES_MIXED };
    format!("{}{}", names[note.rem_euclid(12) as usize], note.div_euclid(12) - 2)
}
