use super::*;
fn setup() -> (App, deckard_plugins::Shared, u32) {
    let class = &deckard_plugins::native::CLASSES[0];
    let mut doc = Document::new_project().unwrap();
    doc.add_slot(0, 0, 0, &deckard_doc::NewModule::Plugin { id: class.plugin_id(), name: class.name.into(), chunk: Vec::new() }).unwrap();
    let mut app = App { doc: Some(std::rc::Rc::new(std::cell::RefCell::new(doc))), ..Default::default() };
    let plugin = app
        .samples
        .plugins
        .instance(deckard_plugins::Slot::Sound { group: 0, sound: 0 }, &class.plugin_id(), class.name, &[], 44100.0)
        .unwrap();
    let id = plugin.lock().unwrap().parameters().iter().find(|p| p.steps == 0).unwrap().id;
    (app, plugin, id)
}
#[test]
fn review_undo_snapshot_delete_preserves_hosted_values() {
    let (mut app, plugin, id) = setup();
    plugin.lock().unwrap().set_param(id, 0.2);
    app.lock_slot(0);
    app.lock_delete_slot(0);
    app.undo();
    assert!(app.doc.as_ref().unwrap().borrow().project().lock.slots.contains(&0));
    plugin.lock().unwrap().set_param(id, 0.8);
    app.lock_recall_at(0, 0.0, None);
    assert!((plugin.lock().unwrap().knob_value(id) - 0.2).abs() < 1e-6, "restored snapshot must still recall its hosted value");
}
#[test]
fn review_undo_snapshot_update_preserves_old_hosted_values() {
    let (mut app, plugin, id) = setup();
    plugin.lock().unwrap().set_param(id, 0.2);
    app.lock_slot(0);
    plugin.lock().unwrap().set_param(id, 0.8);
    app.lock_update();
    app.undo();
    plugin.lock().unwrap().set_param(id, 0.6);
    app.lock_recall_at(0, 0.0, None);
    assert!((plugin.lock().unwrap().knob_value(id) - 0.2).abs() < 1e-6, "undoing UPDATE must recover the old hosted snapshot");
}

#[test]
fn review_hosted_only_recall_creates_an_undo_step() {
    let (mut app, plugin, id) = setup();
    plugin.lock().unwrap().set_param(id, 0.2);
    app.lock_slot(0);
    plugin.lock().unwrap().set_param(id, 0.8);
    let before = app.doc.as_ref().unwrap().borrow().undo_labels().len();
    app.lock_recall_at(0, 0.0, None);
    assert!((plugin.lock().unwrap().knob_value(id) - 0.2).abs() < 1e-6, "recall itself works");
    assert_eq!(app.doc.as_ref().unwrap().borrow().undo_labels().len(), before + 1, "a hosted-only recall must be undoable");
}

fn value(p: &deckard_plugins::Shared, id: u32) -> f64 {
    p.lock().unwrap().knob_value(id)
}
fn level(app: &App) -> f64 {
    app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].level
}
fn set_level(app: &App, v: f64) {
    app.doc.as_ref().unwrap().borrow_mut().set("Level", deckard_doc::Target::Sound(0, 0), "level", mxprj::decode::Value::Float(v)).unwrap();
}
#[test]
fn hosted_lock_undo_redo_keeps_each_recalls_own_values_after_manual_edits() {
    let (mut app, p, id) = setup();
    p.lock().unwrap().set_param(id, 0.2);
    app.lock_slot(0);
    p.lock().unwrap().set_param(id, 0.8);
    app.lock_recall_at(0, 0.0, None);
    app.undo();
    assert!((value(&p, id) - 0.8).abs() < 1e-6);
    app.redo();
    assert!((value(&p, id) - 0.2).abs() < 1e-6);
    p.lock().unwrap().set_param(id, 0.6);
    app.lock_recall_at(0, 0.0, None);
    app.undo();
    assert!((value(&p, id) - 0.6).abs() < 1e-6);
    app.undo();
    assert!((value(&p, id) - 0.8).abs() < 1e-6);
    app.redo();
    assert!((value(&p, id) - 0.2).abs() < 1e-6, "redo of first recall retains its own target");
    app.redo();
    assert!((value(&p, id) - 0.2).abs() < 1e-6);
}
#[test]
fn hosted_lock_mixed_recall_undoes_and_redoes_as_one_step() {
    let (mut app, p, id) = setup();
    p.lock().unwrap().set_param(id, 0.2);
    set_level(&app, 0.7);
    app.lock_slot(0);
    p.lock().unwrap().set_param(id, 0.8);
    set_level(&app, 0.3);
    let before = app.doc.as_ref().unwrap().borrow().undo_labels().len();
    app.lock_recall_at(0, 0.0, None);
    assert_eq!(app.doc.as_ref().unwrap().borrow().undo_labels().len(), before + 1);
    assert!((level(&app) - 0.7).abs() < 1e-6);
    app.undo();
    assert!((value(&p, id) - 0.8).abs() < 1e-6);
    assert!((level(&app) - 0.3).abs() < 1e-6);
    app.redo();
    assert!((value(&p, id) - 0.2).abs() < 1e-6);
    assert!((level(&app) - 0.7).abs() < 1e-6);
}
#[test]
fn hosted_lock_snapshot_update_redo_recovers_updated_values() {
    let (mut app, p, id) = setup();
    p.lock().unwrap().set_param(id, 0.2);
    app.lock_slot(0);
    p.lock().unwrap().set_param(id, 0.8);
    app.lock_update();
    app.undo();
    app.redo();
    p.lock().unwrap().set_param(id, 0.6);
    app.lock_recall_at(0, 0.0, None);
    assert!((value(&p, id) - 0.8).abs() < 1e-6);
}
#[test]
fn hosted_lock_completed_morph_is_one_undo_step() {
    let (mut app, p, id) = setup();
    p.lock().unwrap().set_param(id, 0.2);
    set_level(&app, 0.7);
    app.lock_slot(0);
    p.lock().unwrap().set_param(id, 0.8);
    set_level(&app, 0.3);
    app.lock_setting("morph", 1.0);
    let before = app.doc.as_ref().unwrap().borrow().undo_labels().len();
    app.lock_recall_at(0, 10.0, None);
    app.lock_tick(10.1, None);
    assert!(value(&p, id) < 0.8 && value(&p, id) > 0.2);
    app.lock_tick(100.0, None);
    assert_eq!(app.doc.as_ref().unwrap().borrow().undo_labels().len(), before + 1);
    app.undo();
    assert!((value(&p, id) - 0.8).abs() < 1e-6);
    assert!((level(&app) - 0.3).abs() < 1e-6);
    app.redo();
    assert!((value(&p, id) - 0.2).abs() < 1e-6);
    assert!((level(&app) - 0.7).abs() < 1e-6);
}
#[test]
fn hosted_lock_interrupted_morph_redo_restores_the_last_audible_values() {
    let (mut app, p, id) = setup();
    p.lock().unwrap().set_param(id, 0.2);
    set_level(&app, 0.7);
    app.lock_slot(0);
    p.lock().unwrap().set_param(id, 0.8);
    set_level(&app, 0.3);
    app.lock_setting("morph", 1.0);
    app.lock_recall_at(0, 10.0, None);
    app.lock_tick(10.1, None);
    let mid = (value(&p, id), level(&app));
    app.surface_edit("undo");
    assert!(app.lock_morph.is_none());
    assert!((value(&p, id) - 0.8).abs() < 1e-6);
    assert!((level(&app) - 0.3).abs() < 1e-6);
    app.surface_edit("redo");
    assert!((value(&p, id) - mid.0).abs() < 1e-6);
    assert!((level(&app) - mid.1).abs() < 1e-6);
}
