//! Writing a plug-in's state into the project. Uses `$DECKARD_TEST_PROJECT`
//! (default ~/Desktop/test.mxprj, a project with plug-in instruments); skips if missing.

use deckard_doc::Document;
use std::path::PathBuf;

fn project() -> Option<PathBuf> {
    let p = std::env::var_os("DECKARD_TEST_PROJECT")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Desktop/test.mxprj")))?;
    p.exists().then_some(p).or_else(|| {
        eprintln!("test project not found; skipping");
        None
    })
}

#[test]
fn plugin_state_saves_and_undoes() {
    let Some(path) = project() else { return };
    let mut d = Document::open(&path).unwrap();
    let Some((g, s, old)) = d
        .project()
        .groups
        .iter()
        .enumerate()
        .find_map(|(g, gr)| gr.sounds.iter().enumerate().find_map(|(s, snd)| snd.plugin.as_ref().map(|p| (g, s, p.chunk.clone()))))
    else {
        return;
    };
    // The same state is no change.
    d.set_plugin_chunk(g, s, old.clone()).unwrap();
    assert_eq!(d.undo_label(), None);
    let mut new = old.clone();
    new.extend_from_slice(b"edited");
    d.set_plugin_chunk(g, s, new.clone()).unwrap();
    assert_eq!(d.project().groups[g].sounds[s].plugin.as_ref().unwrap().chunk, new);
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("plugin.mxprj");
    d.save_as(&out).unwrap();
    let again = Document::open(&out).unwrap();
    assert_eq!(again.project().groups[g].sounds[s].plugin.as_ref().unwrap().chunk, new, "saved and read back");
    assert!(d.undo());
    assert_eq!(d.project().groups[g].sounds[s].plugin.as_ref().unwrap().chunk, old);
}

/// Notes can be added, changed and deleted in every pattern of groups whose sounds
/// are plug-ins.
#[test]
fn notes_edit_in_plugin_groups() {
    let Some(path) = project() else { return };
    let mut d = Document::open(&path).unwrap();
    let groups: Vec<usize> =
        (0..d.project().groups.len()).filter(|&g| d.project().groups[g].sounds.iter().any(|s| s.plugin.is_some())).collect();
    assert!(!groups.is_empty());
    for g in groups {
        let sound = d.project().groups[g].sounds.iter().position(|s| s.plugin.is_some()).unwrap();
        for p in 0..d.project().groups[g].patterns.len() {
            let n = deckard_model::Note { expr: None, sound, position: 7, length: 120, pitch: 61, velocity: 90, probability: 100 };
            d.add_note(g, p, n).unwrap_or_else(|e| panic!("group {g} pattern {p}: add: {e}"));
            assert!(d.project().groups[g].patterns[p].notes.contains(&n), "group {g} pattern {p}: added");
            let m = deckard_model::Note { pitch: 62, ..n };
            d.change_note(g, p, n, m).unwrap_or_else(|e| panic!("group {g} pattern {p}: change: {e}"));
            assert!(d.project().groups[g].patterns[p].notes.contains(&m), "group {g} pattern {p}: changed");
            d.delete_notes(g, p, &[m]).unwrap_or_else(|e| panic!("group {g} pattern {p}: delete: {e}"));
            assert!(!d.project().groups[g].patterns[p].notes.contains(&m), "group {g} pattern {p}: deleted");
        }
    }
}

/// Deleting every note of a pattern, then adding one, works (for every pattern of
/// every group, plug-in or not).
#[test]
fn add_after_deleting_everything() {
    let Some(path) = project() else { return };
    let mut d = Document::open(&path).unwrap();
    for g in 0..d.project().groups.len() {
        for p in 0..d.project().groups[g].patterns.len() {
            let all = d.project().groups[g].patterns[p].notes.clone();
            if all.is_empty() {
                continue;
            }
            d.delete_notes(g, p, &all).unwrap_or_else(|e| panic!("group {g} pattern {p}: delete all: {e}"));
            assert!(d.project().groups[g].patterns[p].notes.is_empty(), "group {g} pattern {p}: all gone");
            let n = deckard_model::Note {
                expr: None,
                sound: all[0].sound,
                position: d.project().groups[g].patterns[p].start + 240,
                length: 240,
                pitch: 60,
                velocity: 100,
                probability: 100,
            };
            d.add_note(g, p, n).unwrap_or_else(|e| panic!("group {g} pattern {p}: add after delete: {e}"));
            assert_eq!(d.project().groups[g].patterns[p].notes, vec![n], "group {g} pattern {p}: the new note");
        }
    }
}

/// Every Sampler parameter is read, and can be set (undoable, saved), including
/// ones inside its envelope and LFO objects.
#[test]
fn sampler_parameters_set_save_and_undo() {
    use deckard_doc::Target;
    use mxprj::decode::Value;
    let Some(path) = project() else { return };
    let mut d = Document::open(&path).unwrap();
    let (g, s) = d
        .project()
        .groups
        .iter()
        .enumerate()
        .find_map(|(g, gr)| gr.sounds.iter().position(|s| s.sampler.is_some()).map(|s| (g, s)))
        .unwrap();
    let smp = |d: &Document| d.project().groups[g].sounds[s].sampler.clone().unwrap();
    let before = smp(&d);
    assert!(before.params.len() >= 45, "{:?}", before.params.keys().collect::<Vec<_>>());
    for (field, v) in [("cutoff", 0.25), ("volume_envelope/attack", 0.4), ("lfo/speed", 0.375), ("tune", 2.0)] {
        d.set("Sampler", Target::Sampler(g, s), field, Value::Float(v)).unwrap();
        assert_eq!(smp(&d).get(field), v, "{field}");
    }
    assert_eq!(smp(&d).tune, 2.0, "derived fields follow");
    assert_eq!(smp(&d).attack, 0.4);
    // A zone's own tune is untouched.
    assert_eq!(smp(&d).zones.iter().map(|z| z.tune).collect::<Vec<_>>(), before.zones.iter().map(|z| z.tune).collect::<Vec<_>>());
    // The first zone's play start, loop and crossfade.
    let start = before.zones[0].play_range.0 + 100;
    d.set("Zone", Target::Zone(g, s, 0), "play_range/low", Value::Int(start)).unwrap();
    d.set("Zone", Target::Zone(g, s, 0), "loop_active", Value::Int(1)).unwrap();
    d.set("Zone", Target::Zone(g, s, 0), "x_fade", Value::Int(64)).unwrap();
    assert_eq!((smp(&d).zones[0].play_range.0, smp(&d).zones[0].loop_active, smp(&d).zones[0].x_fade), (start, true, 64));
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("sampler.mxprj");
    d.save_as(&out).unwrap();
    let again = Document::open(&out).unwrap();
    let back = again.project().groups[g].sounds[s].sampler.clone().unwrap();
    assert_eq!(back.get("lfo/speed"), 0.375, "saved and read back");
    assert_eq!((back.zones[0].play_range.0, back.zones[0].loop_active, back.zones[0].x_fade), (start, true, 64));
    for _ in 0..7 {
        assert!(d.undo());
    }
    assert_eq!(smp(&d), before);
}

/// Slots can be added to a sound's chain (a Sampler, a plug-in), bypassed and removed;
/// each is one undo step, and the project saves and reads back.
#[test]
fn chain_slots_add_bypass_remove() {
    use deckard_doc::NewModule;
    let Some(path) = project() else { return };
    let mut d = Document::open(&path).unwrap();
    let slots = |d: &Document, g: usize, s: usize| d.project().groups[g].sounds[s].slots.clone();
    // An empty sound (no modules) gets a Sampler.
    let (g, s) = d
        .project()
        .groups
        .iter()
        .enumerate()
        .find_map(|(g, gr)| gr.sounds.iter().position(|s| s.slots.is_empty()).map(|s| (g, s)))
        .expect("an empty sound");
    d.add_slot(g, s, 0, &NewModule::Sampler).unwrap();
    assert_eq!(slots(&d, g, s).iter().map(|x| x.module.as_str()).collect::<Vec<_>>(), ["Sampler"]);
    let smp = d.project().groups[g].sounds[s].sampler.clone().unwrap();
    assert!(smp.zones.is_empty(), "a new Sampler is empty");
    assert_eq!(smp.get("cutoff"), 0.5);
    // Then a plug-in after it.
    let massive = NewModule::Plugin { id: "Native Instruments:VST:INSTR:1315523937".into(), name: "Massive".into(), chunk: Vec::new() };
    d.add_slot(g, s, 1, &massive).unwrap();
    assert_eq!(slots(&d, g, s).iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Sampler", "Massive"]);
    d.set_slot_active(g, s, 1, false).unwrap();
    assert!(!slots(&d, g, s)[1].active);
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("chain.mxprj");
    d.save_as(&out).unwrap();
    let again = Document::open(&out).unwrap();
    assert_eq!(again.project().groups[g].sounds[s].slots, slots(&d, g, s), "saved and read back");
    d.remove_slot(g, s, 0).unwrap();
    assert_eq!(slots(&d, g, s).iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Massive"]);
    let again = {
        d.save_as(&out).unwrap();
        Document::open(&out).unwrap()
    };
    assert_eq!(again.project().groups[g].sounds[s].slots.len(), 1);
    for _ in 0..4 {
        assert!(d.undo());
    }
    assert!(slots(&d, g, s).is_empty());
}

/// A zone can be pointed at an edited copy of its sample, and a Sampler's zones
/// replaced (here: four slices on keys 36..39), each as one undo step that saves.
#[test]
fn zone_sample_and_zone_list() {
    use deckard_doc::{ZoneFrom, ZoneSpec};
    let Some(path) = project() else { return };
    let mut d = Document::open(&path).unwrap();
    let (g, s) = (0, 0);
    let smp = |d: &Document| d.project().groups[g].sounds[s].sampler.clone().unwrap();
    let before = smp(&d);
    let edited = std::path::PathBuf::from("/tmp/Kick - 2.wav");
    d.set_zone_sample(g, s, 0, &edited, 5000, "Normalize").unwrap();
    let z = smp(&d).zones[0].clone();
    assert_eq!((z.sample.kind, z.sample.path.as_str(), z.play_range), (0, "/tmp/Kick - 2.wav", (0, 4999)));
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("zones.mxprj");
    d.save_as(&out).unwrap();
    let again = Document::open(&out).unwrap();
    assert_eq!(again.project().groups[g].sounds[s].sampler.as_ref().unwrap().zones[0], z, "saved and read back");
    // Slices: four zones on the same file, one key each.
    let specs: Vec<ZoneSpec> = (0..4)
        .map(|k| ZoneSpec {
            from: ZoneFrom::Zone(0),
            file: None,
            set: vec![
                ("play_range/low", (k * 1250) as f64),
                ("play_range/high", (k * 1250 + 1249) as f64),
                ("key_range/low", (36 + k) as f64),
                ("key_range/high", (36 + k) as f64),
                ("root_key", (36 + k) as f64),
            ],
        })
        .collect();
    d.set_zones(g, s, &specs, "Apply Slices").unwrap();
    let zs = smp(&d).zones;
    assert_eq!(zs.len(), 4);
    assert_eq!(
        zs.iter().map(|z| (z.key_range, z.root_key, z.play_range)).collect::<Vec<_>>(),
        (0..4).map(|k| ((36 + k, 36 + k), 36 + k, (k * 1250, k * 1250 + 1249))).collect::<Vec<_>>()
    );
    d.save_as(&out).unwrap();
    let again = Document::open(&out).unwrap();
    assert_eq!(again.project().groups[g].sounds[s].sampler.as_ref().unwrap().zones, zs, "slices saved and read back");
    assert!(d.undo() && d.undo());
    assert_eq!(smp(&d).zones, before.zones);
}

/// Effects reorder; an instrument stays first, and can't be added after an effect.
#[test]
fn chain_slots_reorder() {
    use deckard_doc::NewModule;
    let Ok(mut d) = Document::open(std::path::Path::new("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj")) else {
        return;
    };
    let names = |d: &Document| d.project().groups[0].sounds[0].slots.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
    let before = names(&d);
    assert_eq!(before, ["Sampler", "EQ", "Compressor"]);
    d.move_slot(0, 0, 2, 1).unwrap();
    assert_eq!(names(&d), ["Sampler", "Compressor", "EQ"]);
    assert!(d.move_slot(0, 0, 0, 2).is_err(), "the instrument stays first");
    assert!(d.move_slot(0, 0, 1, 0).is_err(), "nothing goes before the instrument");
    let again = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
    assert_eq!(names(&again), ["Sampler", "Compressor", "EQ"]);
    // An instrument plug-in after the effects: refused; an effect: fine.
    let massive = NewModule::Plugin {
        id: "Native Instruments:VST3:INSTR:5653544E694D6D6173736976650000".into(),
        name: "Massive".into(),
        chunk: Vec::new(),
    };
    assert!(d.add_slot(0, 0, 3, &massive).is_err());
    let fx = NewModule::Plugin {
        id: "BABY Audio:VST3:FX:565354436D36626261627920636F6D65".into(),
        name: "Baby Comeback".into(),
        chunk: Vec::new(),
    };
    d.add_slot(0, 0, 3, &fx).unwrap();
    d.move_slot(0, 0, 3, 1).unwrap();
    assert_eq!(names(&d), ["Sampler", "Baby Comeback", "Compressor", "EQ"]);
    assert!(d.undo() && d.undo() && d.undo());
    assert_eq!(names(&d), before);
}
