//! The project's Note Repeat / Arp settings (the song's `arpeggiator`), read from
//! Maschine's own projects (skipped when they aren't installed).

use deckard_model::{ArpType, Project};

fn open(path: &str) -> Option<Project> {
    let bytes = std::fs::read(path).ok()?;
    let doc = mxprj::Project::parse(&bytes).unwrap();
    Some(Project::from_decoded(doc.decode(&mxprj::types::Types::new()).unwrap()))
}

#[test]
fn the_arp_settings_are_read() {
    if let Some(p) = open("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj") {
        let a = &p.arp;
        assert_eq!((a.kind, a.rate, a.octaves, a.sequence), (ArpType::Up, 240, 1, 0));
        assert_eq!((a.gate, a.dynamic, a.hold), (1.0, 1.0, false));
        assert_eq!((a.offset, a.min_key, a.max_key), (0, 0, 127));
    }
    if let Some(p) = open("/Users/Shared/Chromatic Fire Library/Projects/Concrete Dubs.mxprj") {
        let a = &p.arp;
        assert_eq!((a.kind, a.octaves), (ArpType::OrderPlayed, 2));
        assert!((a.gate - 0.7).abs() < 1e-3 && (a.dynamic - 1.03).abs() < 1e-3);
    }
}
