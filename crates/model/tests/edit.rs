//! Edit a decoded project, write it with the encoder, read it back: the model sees
//! exactly the edits. Uses `$DECKARD_TEST_PROJECT` (default: Cloud 9 Vibes from the
//! Velvet Lounge library); skips if it is missing.

use deckard_model::Project;
use mxprj::decode::Value;

fn test_project() -> Option<Vec<u8>> {
    let path =
        std::env::var("DECKARD_TEST_PROJECT").unwrap_or_else(|_| "/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj".into());
    std::fs::read(&path).map_err(|_| eprintln!("{path} not found; skipping")).ok()
}

/// `param.value` of the first object of type `owner` (searching base classes).
fn set_param(root: &mut Value, owner: &str, field: &str, value: Value) -> bool {
    let mut done = false;
    root.walk_mut(&mut |o| {
        if done || o.type_name != owner {
            return;
        }
        if let Some(Value::Obj(p)) = o.find_mut(field)
            && let Some(v) = p.find_mut("value")
        {
            *v = value.clone();
            done = true;
        }
    });
    done
}

#[test]
fn edits_survive_writing() {
    let Some(bytes) = test_project() else { return };
    let types = mxprj::types::Types::new();
    let doc = mxprj::Project::parse(&bytes).unwrap();
    let root = doc.decode(&types).unwrap();
    let before = Project::from_decoded(root.clone());

    let mut tree = Value::Obj(root);
    assert!(set_param(&mut tree, "NI::MASCHINE::DATA::Song", "tempo", Value::Float(97.5)));
    // A longer name changes the archive length, so the container sizes must follow.
    let name = format!("{} (edited by deckard)", before.name);
    assert!(set_param(&mut tree, "NI::MASCHINE::DATA::Project", "name", Value::Str(name.clone())));
    let mut changed = false;
    tree.walk_mut(&mut |o| {
        if !changed
            && o.type_name.ends_with("NoteEvent")
            && let Some(v) = o.find_mut("velocity")
        {
            *v = Value::Int(1);
            changed = true;
        }
    });
    assert!(changed);
    let Value::Obj(edited) = tree else { unreachable!() };

    let out = doc.encode(&types, edited).unwrap();
    assert_ne!(out.len(), bytes.len());
    let again = mxprj::Project::parse(&out).unwrap();
    let after = Project::from_decoded(again.decode(&types).unwrap());

    assert_eq!(after.tempo, 97.5);
    assert_eq!(after.name, name);
    assert_eq!(after.groups.len(), before.groups.len());
    let notes = |p: &Project| p.groups.iter().flat_map(|g| g.patterns.iter().flat_map(|pt| pt.notes.iter().copied())).collect::<Vec<_>>();
    let (a, b) = (notes(&before), notes(&after));
    assert_eq!(a.len(), b.len());
    let mut differing = 0;
    for (x, y) in a.iter().zip(&b) {
        assert_eq!((x.sound, x.position, x.length, x.pitch), (y.sound, y.position, y.length, y.pitch));
        if x.velocity != y.velocity {
            assert_eq!(y.velocity, 1);
            differing += 1;
        }
    }
    assert!(differing >= 1, "the edited note should show up");
    eprintln!("{differing} note(s) changed velocity (notes can share one event object)");
}
