//! Note edits: they reproduce Maschine's own edit (the notes added in the reference
//! file test-1-assign, see tests/arrange.rs), undo, save and read back.

use deckard_doc::Document;
use deckard_model::{Note, Project};
use std::path::PathBuf;

fn refs() -> Option<PathBuf> {
    let dir = std::env::var_os("DECKARD_EDIT_REFS")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Desktop")))?;
    dir.join("test-1-assign.mxprj").exists().then_some(dir).or_else(|| {
        eprintln!("reference files not found; skipping");
        None
    })
}

fn open(dir: &std::path::Path, name: &str) -> Document {
    Document::open(&dir.join(format!("test-{name}.mxprj"))).unwrap()
}

fn notes(p: &Project, g: usize, pi: usize) -> Vec<Note> {
    p.groups[g].patterns[pi].notes.clone()
}

/// Notes in `a` but not in `b` (as multisets).
fn minus(a: &[Note], b: &[Note]) -> Vec<Note> {
    let mut rest = b.to_vec();
    a.iter()
        .filter(|n| match rest.iter().position(|m| m == *n) {
            Some(i) => {
                rest.remove(i);
                false
            }
            None => true,
        })
        .copied()
        .collect()
}

fn reopen(d: &mut Document) -> Document {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("n.mxprj");
    d.save_as(&path).unwrap();
    Document::open(&path).unwrap()
}

/// Group 0, pattern 4 ("Kicks"): Maschine's test-1 has 3 more notes than test-7.
#[test]
fn adding_notes_matches_maschine() {
    let Some(dir) = refs() else { return };
    let mut d = open(&dir, "7-section-length");
    let want = notes(open(&dir, "1-assign").project(), 0, 4);
    let added = minus(&want, &notes(d.project(), 0, 4));
    assert_eq!(added.len(), 3, "{added:?}");
    d.edit_notes("Add Notes", 0, 4, &[], &added).unwrap();
    assert_eq!(notes(d.project(), 0, 4), want);
    assert_eq!(notes(reopen(&mut d).project(), 0, 4), want);
    assert!(d.undo());
    assert_eq!(notes(d.project(), 0, 4), notes(open(&dir, "7-section-length").project(), 0, 4));
}

#[test]
fn deleting_notes_matches_maschine() {
    let Some(dir) = refs() else { return };
    let mut d = open(&dir, "1-assign");
    let want = notes(open(&dir, "7-section-length").project(), 0, 4);
    let extra = minus(&notes(d.project(), 0, 4), &want);
    d.delete_notes(0, 4, &extra).unwrap();
    assert_eq!(notes(d.project(), 0, 4), want);
    assert_eq!(notes(reopen(&mut d).project(), 0, 4), want);
}

#[test]
fn changing_a_note() {
    let Some(dir) = refs() else { return };
    let mut d = open(&dir, "7-section-length");
    let old = notes(d.project(), 0, 4)[1];
    let new = Note { position: old.position + 240, length: 480, velocity: 64, ..old };
    d.change_note(0, 4, old, new).unwrap();
    let after = notes(d.project(), 0, 4);
    assert!(after.contains(&new) && !after.contains(&old));
    assert_eq!(after.len(), notes(open(&dir, "7-section-length").project(), 0, 4).len());
    let again = reopen(&mut d);
    assert_eq!(notes(again.project(), 0, 4), after);
}

/// A sound with no notes in the pattern gets a new sequence.
#[test]
fn first_note_of_a_sound_creates_its_sequence() {
    let Some(dir) = refs() else { return };
    let mut d = open(&dir, "7-section-length");
    let before = notes(d.project(), 0, 4);
    let sound = (0..d.project().groups[0].sounds.len()).find(|s| before.iter().all(|n| n.sound != *s)).unwrap();
    let n = Note { expr: None, sound, position: 960, length: 240, pitch: 60, velocity: 100, probability: 100 };
    d.add_note(0, 4, n).unwrap();
    assert!(notes(d.project(), 0, 4).contains(&n));
    let m = Note { position: 1920, ..n };
    d.add_note(0, 4, m).unwrap();
    let again = reopen(&mut d);
    let saved = notes(again.project(), 0, 4);
    assert!(saved.contains(&n) && saved.contains(&m));
    assert_eq!(saved.len(), before.len() + 2);
    // Other patterns are untouched.
    for pi in 0..d.project().groups[0].patterns.len() {
        if pi != 4 {
            assert_eq!(notes(again.project(), 0, pi), notes(open(&dir, "7-section-length").project(), 0, pi));
        }
    }
    assert!(d.undo() && d.undo());
    assert_eq!(notes(d.project(), 0, 4), before);
}

/// Every note of every pattern can be deleted and re-added (exercises shared node
/// markers being moved to their next reference).
#[test]
fn delete_and_restore_every_note_of_a_group() {
    let Some(dir) = refs() else { return };
    let mut d = open(&dir, "7-section-length");
    let n = d.project().groups[0].patterns.len();
    for pi in 0..n {
        let all = notes(d.project(), 0, pi);
        if all.is_empty() {
            continue;
        }
        d.delete_notes(0, pi, &all).unwrap();
        assert!(notes(d.project(), 0, pi).is_empty(), "pattern {pi}");
    }
    let again = reopen(&mut d);
    for pi in 0..n {
        assert!(notes(again.project(), 0, pi).is_empty());
    }
    while d.undo() {}
    for pi in 0..n {
        assert_eq!(notes(d.project(), 0, pi), notes(open(&dir, "7-section-length").project(), 0, pi));
    }
}

/// Probability in an older project: the notes are upgraded to Maschine 3's format
/// (every note, as one archive holds one version per class), then edited.
#[test]
fn probability_upgrades_old_notes() {
    let Some(dir) = refs() else { return };
    let mut d = Document::open(&dir.join("test.mxprj")).unwrap();
    let old = notes(d.project(), 0, 4)[0];
    let before: Vec<Vec<Note>> = (0..d.project().groups.len())
        .flat_map(|g| (0..d.project().groups[g].patterns.len()).map(move |p| (g, p)))
        .map(|(g, p)| notes(d.project(), g, p))
        .collect();
    let had = d.notes_have_probability();
    d.change_note(0, 4, old, Note { probability: 40, ..old }).unwrap();
    assert!(d.notes_have_probability());
    assert!(notes(d.project(), 0, 4).contains(&Note { probability: 40, ..old }));
    let again = reopen(&mut d);
    assert!(notes(again.project(), 0, 4).contains(&Note { probability: 40, ..old }));
    // Undo still works after the upgrade (the history was upgraded too), and saves.
    assert!(d.undo());
    let after: Vec<Vec<Note>> = (0..d.project().groups.len())
        .flat_map(|g| (0..d.project().groups[g].patterns.len()).map(move |p| (g, p)))
        .map(|(g, p)| notes(d.project(), g, p))
        .collect();
    assert_eq!(after, before);
    reopen(&mut d);
    eprintln!("project had probabilities before: {had}");
}

/// MIDI controller and pitch-bend events: added, changed and deleted, undone, saved
/// and read back; also in a project that had none (the event class is then written
/// from scratch).
#[test]
fn midi_events_edit_save_and_undo() {
    use deckard_model::{MidiEvent, MidiKind};
    let Some(dir) = refs() else { return };
    let mut d = open(&dir, "1-assign");
    let midi = |d: &Document, g: usize, p: usize| d.project().groups[g].patterns[p].midi.clone();
    // Every recorded event (sustain pedal, group "Keys 2") goes: the project has none.
    let mut all = 0;
    for g in 0..d.project().groups.len() {
        for p in 0..d.project().groups[g].patterns.len() {
            let m = midi(&d, g, p);
            all += m.len();
            if !m.is_empty() {
                d.edit_midi("Clear", g, p, &m, &[]).unwrap();
                assert!(midi(&d, g, p).is_empty());
            }
        }
    }
    assert!(all > 0, "the reference project has pedal events");
    let mut d = reopen(&mut d);
    // A sound with notes, and one without a sequence in the pattern.
    let (g, p) = (8, 0);
    let with_notes = d.project().groups[g].patterns[p].notes[0].sound;
    let empty = (0..16).find(|s| d.project().groups[g].patterns[p].notes.iter().all(|n| n.sound != *s)).unwrap();
    let add = [
        MidiEvent { sound: with_notes, position: 0, kind: MidiKind::Controller { number: 1, value: 90 } },
        MidiEvent { sound: with_notes, position: 480, kind: MidiKind::PitchBend(12288) },
        MidiEvent { sound: empty, position: 240, kind: MidiKind::Controller { number: 11, value: 5 } },
    ];
    d.edit_midi("Draw", g, p, &[], &add).unwrap();
    let mut got = midi(&d, g, p);
    got.sort_by_key(|m| (m.sound, m.position));
    let mut want = add.to_vec();
    want.sort_by_key(|m| (m.sound, m.position));
    assert_eq!(got, want);
    let again = reopen(&mut d);
    let mut back = midi(&again, g, p);
    back.sort_by_key(|m| (m.sound, m.position));
    assert_eq!(back, want, "saved and read back");
    // Change one and delete one.
    let changed = MidiEvent { kind: MidiKind::Controller { number: 1, value: 20 }, ..add[0] };
    d.edit_midi("Edit", g, p, &[add[0], add[1]], &[changed]).unwrap();
    let got = midi(&d, g, p);
    assert!(got.contains(&changed) && !got.contains(&add[0]) && !got.contains(&add[1]) && got.contains(&add[2]));
    assert!(d.undo());
    assert_eq!(midi(&d, g, p).len(), 3);
    assert!(d.undo());
    assert!(midi(&d, g, p).is_empty());
    let notes_before = d.project().groups[g].patterns[p].notes.len();
    assert_eq!(reopen(&mut d).project().groups[g].patterns[p].notes.len(), notes_before);
}

/// The lanes listed under a sound's notes are saved with it (`midi_automators`):
/// adding one shows it; removing one deletes its events too; both undo.
#[test]
fn lanes_save_add_and_remove() {
    use deckard_model::{LaneRef, MidiEvent, MidiKind};
    let Some(dir) = refs() else { return };
    let mut d = open(&dir, "1-assign");
    let (g, s) = (8, 0);
    let lanes = |d: &Document| (d.project().groups[g].sounds[s].lanes.clone(), d.project().groups[g].sounds[s].lane);
    let vel = LaneRef { kind: 2, number: 0 };
    let prob = LaneRef { kind: 8, number: 0 };
    let pedal = LaneRef { kind: 4, number: 64 };
    assert_eq!(lanes(&d), (vec![vel, prob, pedal], Some(2)), "as Maschine saved them");
    let wheel = LaneRef { kind: 4, number: 1 };
    d.add_lane(g, s, wheel).unwrap();
    d.add_lane(g, s, wheel).unwrap();
    assert_eq!(lanes(&d), (vec![vel, prob, pedal, wheel], Some(3)), "added once, and shown");
    d.add_lane(g, s, LaneRef { kind: 3, number: -1 }).unwrap();
    let again = reopen(&mut d);
    assert_eq!(lanes(&again).0.len(), 5, "saved and read back");
    assert_eq!(lanes(&again).1, Some(4));
    // Remove the pedal lane with its events in one pattern.
    let p = (0..d.project().groups[g].patterns.len()).find(|&p| !d.project().groups[g].patterns[p].midi.is_empty()).unwrap();
    let events: Vec<MidiEvent> = d.project().groups[g].patterns[p]
        .midi
        .iter()
        .filter(|m| m.sound == s && matches!(m.kind, MidiKind::Controller { number: 64, .. }))
        .copied()
        .collect();
    assert!(!events.is_empty());
    d.remove_lane(g, p, s, pedal, &events).unwrap();
    assert!(!lanes(&d).0.contains(&pedal));
    assert!(d.project().groups[g].patterns[p].midi.is_empty());
    let again = reopen(&mut d);
    assert!(!lanes(&again).0.contains(&pedal));
    assert!(d.undo());
    assert!(lanes(&d).0.contains(&pedal));
    assert_eq!(d.project().groups[g].patterns[p].midi.len(), events.len());
    // Removing the shown lane shows the first one.
    let shown = lanes(&d).0[lanes(&d).1.unwrap()];
    d.remove_lane(g, p, s, shown, &[]).unwrap();
    assert_eq!(lanes(&d).1, Some(0));
}
