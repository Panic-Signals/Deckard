//! Clips read from Maschine's projects (docs/superpowers/plans/2026-10-23-phase10-
//! export.md): each group's clip patterns and where its clips play. Skipped where the
//! projects aren't installed.

use deckard_model::{Clip, Project};

fn open(path: &str) -> Option<Project> {
    let bytes = std::fs::read(path).ok()?;
    let doc = mxprj::Project::parse(&bytes).unwrap();
    Some(Project::from_decoded(doc.decode(&mxprj::types::Types::new()).unwrap()))
}

#[test]
fn a_projects_clips_are_read() {
    let Some(p) = open("/Users/Shared/Mother Board Library/Projects/Aardvark.mxprj") else { return };
    let clips: usize = p.groups.iter().map(|g| g.clips.len()).sum();
    assert_eq!(clips, 24);
    let g = &p.groups[0];
    assert_eq!(g.clips, [Clip { position: 7680, length: 7680, pattern: 0 }]);
    assert_eq!(g.clip_patterns.len(), 1);
    for g in &p.groups {
        assert!(g.clips.iter().all(|c| c.pattern < g.clip_patterns.len() && c.length > 0));
        assert!(g.clips.windows(2).all(|w| w[0].position <= w[1].position), "in time order");
    }
    assert!(p.groups.iter().flat_map(|g| &g.clip_patterns).any(|c| !c.notes.is_empty()), "clips hold notes");
}
