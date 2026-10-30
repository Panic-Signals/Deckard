//! The Pattern Editor's note tools, as Maschine's (software manual pp. 238–242;
//! `EventPatternTools`): split, join, mute and nudge. Each gives the notes to remove and
//! the notes to add, for [`crate::Document::edit_notes`].

use deckard_model::Note;

/// What a tool changes: notes taken out, notes put in.
pub type Change = (Vec<Note>, Vec<Note>);

/// Split `targets` at `at`: each note playing there (begun before it, ending after it)
/// becomes two, the second from `at`.
pub fn split(targets: &[Note], at: i64) -> Change {
    let cut: Vec<Note> = targets.iter().copied().filter(|n| n.position < at && at < n.position + n.length).collect();
    let made = cut
        .iter()
        .flat_map(|n| [Note { length: at - n.position, ..*n }, Note { position: at, length: n.position + n.length - at, ..*n }])
        .collect();
    (cut, made)
}

/// Join `clicked` with the next note on its key (same sound and pitch), filling any gap.
/// A clicked note in `selected` joins each selected note with the next selected one on
/// its key instead; notes in between are left alone.
pub fn join(all: &[Note], clicked: Note, selected: &[Note]) -> Change {
    let key = |n: &Note| (n.sound, n.pitch);
    let joined = |a: Note, b: Note| Note { length: (b.position + b.length).max(a.position + a.length) - a.position, ..a };
    let next = |of: Note, among: &[Note]| {
        among.iter().copied().filter(|n| key(n) == key(&of) && n.position > of.position).min_by_key(|n| n.position)
    };
    if !selected.contains(&clicked) {
        return match next(clicked, all) {
            Some(b) => (vec![clicked, b], vec![joined(clicked, b)]),
            None => (Vec::new(), Vec::new()),
        };
    }
    // Each selected note with the next selected one on its key, in pairs from the left.
    let mut sorted: Vec<Note> = selected.to_vec();
    sorted.sort_by_key(|n| (n.sound, n.pitch, n.position));
    let (mut gone, mut made) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i + 1 < sorted.len() {
        let (a, b) = (sorted[i], sorted[i + 1]);
        if key(&a) == key(&b) {
            gone.extend([a, b]);
            made.push(joined(a, b));
            i += 2;
        } else {
            i += 1;
        }
    }
    (gone, made)
}

/// Mute or unmute: the clicked note alone, or every selected note when it is one of
/// them (all muted: unmuted, else all muted). A muted note is probability 0 (Maschine 3
/// keeps no flag); unmuted, 100. The undo label goes with it.
pub fn toggle_mute(clicked: Note, selected: &[Note]) -> (Change, &'static str) {
    let targets: Vec<Note> = if selected.contains(&clicked) { selected.to_vec() } else { vec![clicked] };
    let unmute = targets.iter().all(muted);
    let probability = if unmute { 100 } else { 0 };
    let gone: Vec<Note> = targets.into_iter().filter(|n| n.probability != probability).collect();
    let made = gone.iter().map(|n| Note { probability, ..*n }).collect();
    ((gone, made), if unmute { "Unmute Note" } else { "Mute Note" })
}

/// Whether a note is muted.
pub fn muted(n: &Note) -> bool {
    n.probability == 0
}

/// Move `notes` by `delta` ticks within the pattern's `start..end`: a note pushed past an
/// end comes round from the other.
pub fn nudge(notes: &[Note], delta: i64, start: i64, end: i64) -> Change {
    let len = (end - start).max(1);
    let made = notes.iter().map(|n| Note { position: start + (n.position + delta - start).rem_euclid(len), ..*n }).collect();
    (notes.to_vec(), made)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(sound: usize, position: i64, length: i64, pitch: i64) -> Note {
        Note { expr: None, sound, position, length, pitch, velocity: 100, probability: 100 }
    }

    #[test]
    fn split_cuts_the_notes_playing_there() {
        let (gone, new) = split(&[n(0, 0, 960, 60), n(1, 480, 240, 60), n(2, 960, 240, 60)], 480);
        assert_eq!(gone, [n(0, 0, 960, 60)], "only notes playing across the cut");
        assert_eq!(new, [n(0, 0, 480, 60), n(0, 480, 480, 60)]);
        assert_eq!(split(&[n(0, 0, 240, 60)], 480), (vec![], vec![]), "nothing playing: nothing");
    }

    #[test]
    fn join_fills_the_gap_to_the_next_note_on_the_key() {
        let a = n(0, 0, 240, 60);
        let other_key = n(0, 240, 240, 62);
        let b = n(0, 960, 240, 60);
        let all = [a, other_key, b, n(0, 1920, 240, 60)];
        let (gone, new) = join(&all, a, &[]);
        assert_eq!(gone, [a, b]);
        assert_eq!(new, [n(0, 0, 1200, 60)]);
        // The last note on its key: nothing.
        assert_eq!(join(&all, all[3], &[]), (vec![], vec![]));
    }

    #[test]
    fn join_on_a_selection_pairs_selected_notes_per_key() {
        let a = n(0, 0, 240, 60);
        let mid = n(0, 480, 240, 60);
        let b = n(0, 960, 240, 60);
        let c = n(1, 0, 240, 60);
        let d = n(1, 480, 480, 60);
        let all = [a, mid, b, c, d];
        // a and b selected (mid isn't, and stays); c and d (another sound) too.
        let (gone, new) = join(&all, a, &[a, b, c, d]);
        assert!(gone.contains(&a) && gone.contains(&b) && gone.contains(&c) && gone.contains(&d));
        assert!(!gone.contains(&mid));
        assert_eq!(new.len(), 2);
        assert!(new.contains(&n(0, 0, 1200, 60)));
        assert!(new.contains(&n(1, 0, 960, 60)));
        // An unselected note clicked: joins with the next note, selected or not.
        let (gone, new) = join(&all, mid, &[a, b]);
        assert_eq!((gone, new), (vec![mid, b], vec![n(0, 480, 720, 60)]));
    }

    #[test]
    fn mute_toggles_the_clicked_note_or_the_selection() {
        let a = n(0, 0, 240, 60);
        let muted_b = Note { probability: 0, ..n(0, 480, 240, 60) };
        let ((gone, new), label) = toggle_mute(a, &[]);
        assert_eq!((gone, new, label), (vec![a], vec![Note { probability: 0, ..a }], "Mute Note"));
        let ((_, new), label) = toggle_mute(muted_b, &[]);
        assert_eq!((new, label), (vec![Note { probability: 100, ..muted_b }], "Unmute Note"));
        // A selection not all muted: all muted (the muted one left as it is).
        let ((gone, new), _) = toggle_mute(muted_b, &[a, muted_b]);
        assert_eq!((gone, new), (vec![a], vec![Note { probability: 0, ..a }]));
        // All muted: all unmuted.
        let muted_a = Note { probability: 0, ..a };
        let ((_, new), label) = toggle_mute(muted_a, &[muted_a, muted_b]);
        assert!(new.iter().all(|n| n.probability == 100) && new.len() == 2);
        assert_eq!(label, "Unmute Note");
        // Unselected clicked: only it.
        let ((gone, _), _) = toggle_mute(a, &[muted_b]);
        assert_eq!(gone, [a]);
    }

    #[test]
    fn nudge_wraps_round_the_pattern() {
        let (gone, new) = nudge(&[n(0, 3600, 240, 60), n(0, 0, 240, 60)], 240, 0, 3840);
        assert_eq!(gone.len(), 2);
        assert_eq!(new, [n(0, 0, 240, 60), n(0, 240, 240, 60)], "past the end: from the start");
        let (_, new) = nudge(&[n(0, 1000, 240, 60)], -240, 960, 4800);
        assert_eq!(new, [n(0, 4600, 240, 60)], "200 before the start: 200 before the end");
    }
}
