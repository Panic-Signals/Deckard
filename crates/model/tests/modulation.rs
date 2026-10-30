//! Modulation (lanes and points) read from Maschine's projects: a Maschine 2 project
//! (v0 points, "Atma"), Maschine 3's templates (v1 points; the Hip Hop template's are
//! the group's). Skipped where they aren't installed.

use deckard_model::Project;

fn open(path: &str) -> Option<Project> {
    let bytes = std::fs::read(path).ok()?;
    let doc = mxprj::Project::parse(&bytes).unwrap();
    Some(Project::from_decoded(doc.decode(&mxprj::types::Types::new()).unwrap()))
}

#[test]
fn v0_points_are_read_with_their_lane() {
    let Some(p) = open("/Users/Shared/Solar Breeze Library/Projects/Atma.mxprj") else { return };
    // A sound with a lane on its `analog_mix`, whose points the patterns hold.
    let (g, s) = p
        .groups
        .iter()
        .enumerate()
        .flat_map(|(g, gr)| gr.sounds.iter().enumerate().map(move |(s, snd)| (g, s, snd)))
        .find(|(_, _, snd)| snd.modulators.iter().any(|t| t.field == "analog_mix"))
        .map(|(g, s, _)| (g, s))
        .expect("a lane on analog_mix");
    let target = p.groups[g].sounds[s].modulators.iter().find(|t| t.field == "analog_mix").unwrap();
    assert!(target.slot.is_some(), "a module's parameter");
    let points: Vec<_> =
        p.groups[g].patterns.iter().flat_map(|pt| pt.mods.iter()).filter(|m| m.sound == Some(s) && m.field == "analog_mix").collect();
    assert!(!points.is_empty());
    assert!(points.iter().any(|m| m.position == 0 && (m.value - 0.8048780560493469).abs() < 1e-6));
    assert!(points.iter().all(|m| m.slot == target.slot));
}

#[test]
fn v1_points_are_read() {
    let Some(p) = open("/Library/Application Support/Native Instruments/Maschine 3/Template Projects/02/Dance Template.mxprj") else {
        return;
    };
    let all: Vec<_> = p.groups.iter().flat_map(|g| g.patterns.iter().flat_map(|pt| pt.mods.iter())).collect();
    assert_eq!(all.len(), 136);
    assert!(all.iter().any(|m| m.position == 0 && (m.value + 0.04).abs() < 1e-6), "the first point: −0.04");
    assert!(all.iter().any(|m| m.position == 1204));
}

#[test]
fn a_groups_points_are_the_patterns_own() {
    let Some(p) = open("/Library/Application Support/Native Instruments/Maschine 3/Template Projects/01/Hip Hop Template.mxprj") else {
        return;
    };
    let group: Vec<_> =
        p.groups.iter().flat_map(|g| g.patterns.iter().flat_map(|pt| pt.mods.iter())).filter(|m| m.sound.is_none()).collect();
    assert_eq!(group.len(), 19);
    assert!(group.iter().any(|m| m.field == "touchstrip_on_off"));
    assert!(p.groups.iter().any(|g| g.modulators.iter().any(|t| t.field == "touchstrip_on_off")));
}
