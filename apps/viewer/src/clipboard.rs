//! Cut, copy and paste of notes in the Pattern Editor (software manual pp. 243–244):
//! what was copied, and where a paste puts it.

use deckard_model::Note;

/// Notes cut or copied, as they were (their pattern's ticks), with what was in focus.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteClip {
    pub notes: Vec<Note>,
    /// The focused sound when copied.
    pub sound: usize,
}

/// Where a paste goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PasteAt {
    /// The event area's menu: the step under the mouse (`tick`, the pattern's ticks)
    /// and, in Keyboard view, the pitch of the row under it.
    Mouse { tick: i64, pitch: Option<i64> },
    /// ⌘V.
    Keys,
}

/// The notes a paste adds: the first copied note on the step grid (`grid` from the
/// pattern's `start`), the others keeping their offsets to it.
/// - From the menu: at the mouse; Group view (`keyboard` false): the topmost copied sound
///   onto the focused one (`sound`), the others below it as they were; Keyboard view:
///   the focused sound's copied notes only, the highest pitch at the mouse's row.
/// - From ⌘V with the focus unchanged: from the first step after the copied notes; with
///   the focus changed: at the same times, onto the focused sound as above (Keyboard
///   view: at the same pitches).
///
/// Sounds past the group's `sounds` are dropped.
pub fn paste(clip: &NoteClip, at: PasteAt, sound: usize, keyboard: bool, start: i64, grid: i64, sounds: usize) -> Vec<Note> {
    let grid = grid.max(1);
    // Keyboard view pastes the notes of the sound that was focused.
    let notes: Vec<Note> = clip.notes.iter().filter(|n| !keyboard || n.sound == clip.sound).copied().collect();
    let Some(first) = notes.iter().map(|n| n.position).min() else { return Vec::new() };
    let snap = |t: i64| start + ((t - start) as f64 / grid as f64).round() as i64 * grid;
    let moved_focus = sound != clip.sound;
    let to = match at {
        PasteAt::Mouse { tick, .. } => snap(tick),
        PasteAt::Keys if moved_focus => snap(first),
        PasteAt::Keys => {
            let end = notes.iter().map(|n| n.position + n.length).max().unwrap_or(first);
            start + (end - start + grid - 1).div_euclid(grid) * grid
        }
    };
    let top = notes.iter().map(|n| n.sound).min().unwrap_or(0);
    let high = notes.iter().map(|n| n.pitch).max().unwrap_or(60);
    let pitch_shift = match at {
        PasteAt::Mouse { pitch: Some(p), .. } if keyboard => p - high,
        _ => 0,
    };
    notes
        .iter()
        .filter_map(|n| {
            let s = match at {
                // ⌘V with the focus where it was: the same sounds.
                PasteAt::Keys if !moved_focus => n.sound,
                _ if keyboard => sound,
                _ => sound + (n.sound - top),
            };
            (s < sounds).then_some(Note { sound: s, position: to + n.position - first, pitch: (n.pitch + pitch_shift).clamp(0, 127), ..*n })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(sound: usize, position: i64, pitch: i64) -> Note {
        Note { expr: None, sound, position, length: 240, pitch, velocity: 100, probability: 100 }
    }

    #[test]
    fn keys_paste_after_the_copied_notes_when_the_focus_stays() {
        let clip = NoteClip { notes: vec![n(2, 0, 60), n(3, 720, 60)], sound: 2 };
        // The copy ends at 960: the next step on, keeping the offsets.
        assert_eq!(paste(&clip, PasteAt::Keys, 2, false, 0, 240, 16), [n(2, 960, 60), n(3, 1680, 60)]);
        // Copied with sound 3 focused (not the topmost): still onto the same sounds.
        let clip = NoteClip { sound: 3, ..clip };
        assert_eq!(paste(&clip, PasteAt::Keys, 3, false, 0, 240, 16), [n(2, 960, 60), n(3, 1680, 60)]);
    }

    #[test]
    fn keys_paste_onto_a_new_focus_at_the_same_times() {
        let clip = NoteClip { notes: vec![n(2, 120, 60), n(3, 720, 60)], sound: 2 };
        // Sound 5 focused: the topmost copied (2) goes to 5, 3 to 6; the first snaps.
        assert_eq!(paste(&clip, PasteAt::Keys, 5, false, 0, 240, 16), [n(5, 240, 60), n(6, 840, 60)]);
        // Past the group's sounds: dropped.
        assert_eq!(paste(&clip, PasteAt::Keys, 15, false, 0, 240, 16), [n(15, 240, 60)]);
    }

    #[test]
    fn menu_paste_goes_to_the_mouse() {
        let clip = NoteClip { notes: vec![n(2, 0, 60), n(3, 480, 60)], sound: 2 };
        assert_eq!(paste(&clip, PasteAt::Mouse { tick: 1000, pitch: None }, 0, false, 0, 240, 16), [n(0, 960, 60), n(1, 1440, 60)]);
    }

    #[test]
    fn keyboard_view_pastes_the_focused_sounds_notes_by_pitch() {
        let clip = NoteClip { notes: vec![n(2, 0, 60), n(2, 240, 64), n(3, 0, 70)], sound: 2 };
        // Only sound 2's (it was focused); the highest pitch (64) at the row's 67.
        assert_eq!(paste(&clip, PasteAt::Mouse { tick: 480, pitch: Some(67) }, 4, true, 0, 240, 16), [n(4, 480, 63), n(4, 720, 67)]);
        // ⌘V onto another sound: same times and pitches.
        assert_eq!(paste(&clip, PasteAt::Keys, 4, true, 0, 240, 16), [n(4, 0, 60), n(4, 240, 64)]);
    }
}
