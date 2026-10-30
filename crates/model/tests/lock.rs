//! Lock snapshots read from Maschine's projects (docs/superpowers/plans/2026-10-22-
//! phase6-lock.md). Skipped where the projects aren't installed.

use deckard_model::Project;

fn open(path: &str) -> Option<Project> {
    let bytes = std::fs::read(path).ok()?;
    let doc = mxprj::Project::parse(&bytes).unwrap();
    Some(Project::from_decoded(doc.decode(&mxprj::types::Types::new()).unwrap()))
}

#[test]
fn the_songs_lock_snapshots_and_settings_are_read() {
    let Some(p) = open("/Users/Shared/Chromatic Fire Library/Projects/Lightspeed.mxprj") else { return };
    let l = &p.lock;
    assert_eq!(l.slots.len(), 16, "{:?}", l.slots);
    assert!(l.morph);
    assert_eq!((l.mode, l.time, l.grid), (0, 1, 3));
    assert_eq!(l.banks, 2);
    assert!(l.slots.iter().all(|s| *s < 64));
    // A project without any: the defaults.
    let Some(p) = open("/Users/Shared/Chromatic Fire Library/Projects/Mirage.mxprj") else { return };
    assert_eq!(p.lock.slots.len(), 1);
    assert_eq!((p.lock.morph, p.lock.banks), (false, 1));
}
