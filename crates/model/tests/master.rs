//! The master's plug-in chain (Maschine's MASTER chain), read from one of its own
//! projects (Velvet Lounge's "Cloud 9 Vibes"; skipped if it isn't installed).

use deckard_model::Project;

#[test]
fn the_masters_effects_are_read() {
    let path = "/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj";
    let Ok(bytes) = std::fs::read(path) else { return };
    let doc = mxprj::Project::parse(&bytes).unwrap();
    let p = Project::from_decoded(doc.decode(&mxprj::types::Types::new()).unwrap());
    let modules: Vec<&str> = p.master_slots.iter().map(|s| s.module.as_str()).collect();
    assert_eq!(modules, ["Limiter", "EQ"]);
    assert!(p.master_slots.iter().all(|s| s.active && !s.params.is_empty()));
}
