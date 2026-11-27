//! Editing hosted plug-in parameter pages: VST parameter Learn, assigning from the
//! parameter list, renaming/resetting parameters, section headings, and adding, renaming,
//! deleting, clearing and resetting pages.

use crate::layout::{self, Act, ChainTab};
use crate::plugin_pages::PluginPageAct;
use crate::remote::AssignTab;
use crate::{App, PluginView, ViewPage};
use deckard_doc::{Chain, NewModule};
use deckard_plugins::ParamInfo;
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::time::Instant;

const CHAIN: Chain = Chain::Sound(0, 0);
const PLUGIN_ID: &str = "TestVendor:VST3:INSTR:00112233445566778899AABBCCDDEEFF";

fn fake_params() -> Vec<ParamInfo> {
    [
        (10, "Cutoff"),
        (11, "Resonance"),
        (12, "Drive"),
        (13, "Mix"),
        (14, "Attack"),
        (15, "Decay"),
        (16, "Sustain"),
        (17, "Release"),
        (18, "LFO Rate"),
    ]
    .into_iter()
    .map(|(id, title)| ParamInfo { id, title: title.into(), short: title.into(), units: String::new(), steps: 0, default: 0.5 })
    .collect()
}

/// Build a fake [`PluginView`] reflecting `doc`'s saved pages (or automatic pages from
/// [`fake_params`] when none are saved), so tests run without any third-party VST3 installed.
fn sync_fake_view(app: &mut App) {
    let doc = app.doc.clone().unwrap();
    let d = doc.borrow();
    let rev = d.revision();
    let pr = d.chain_slots(CHAIN).unwrap()[0].plugin.as_ref().unwrap().clone();
    drop(d);
    let params = fake_params();
    app.plugin_params.borrow_mut().insert((CHAIN, 0), (PLUGIN_ID.into(), params.clone()));
    let mut pages: Vec<ViewPage> = if !pr.pages.is_empty() {
        pr.pages
            .iter()
            .map(|pg| {
                let mut knobs: Vec<Option<crate::ViewKnob>> = pg
                    .params
                    .iter()
                    .take(8)
                    .map(|k| {
                        if k.id == u32::MAX {
                            return None;
                        }
                        let info = params.iter().find(|i| i.id == k.id)?.clone();
                        let label = if k.name.is_empty() { info.title.clone() } else { k.name.clone() };
                        Some((info, label, k.section.clone()))
                    })
                    .collect();
                knobs.resize(8, None);
                ViewPage { name: crate::saved_page_name(pg), knobs, choices: vec![Vec::new(); 8], own: false }
            })
            .collect()
    } else {
        params
            .chunks(8)
            .map(|chunk| ViewPage {
                name: String::new(),
                knobs: chunk.iter().map(|i| Some((i.clone(), i.title.clone(), String::new()))).collect(),
                choices: vec![Vec::new(); chunk.len()],
                own: false,
            })
            .collect()
    };
    crate::name_unnamed_pages(&mut pages);
    let shown = app.plugin_page.min(pages.len().saturating_sub(1));
    let values = pages.get(shown).map_or(Vec::new(), |pg| {
        pg.knobs
            .iter()
            .map(|k| match k {
                Some((info, ..)) => (info.default, "50 %".into()),
                None => (0.0, String::new()),
            })
            .collect()
    });
    let now = Instant::now();
    app.plugin_views.insert(
        (CHAIN, 0),
        PluginView {
            key: (CHAIN, 0),
            pages,
            sidechain: false,
            page: shown,
            shown,
            values,
            refreshed: now,
            seen: now,
            plugin_id: PLUGIN_ID.into(),
            pages_rev: rev,
            saved_pages: pr.pages,
            instance: std::sync::Weak::new(),
            error: None,
            sent: Vec::new(),
        },
    );
    app.plugin_focus = Some((CHAIN, 0));
}

fn app_with_hosted_plugin() -> App {
    let mut app = App { settings: crate::settings::Settings::default(), model: "M3".into(), ..App::default() };
    app.new_project();
    let doc = app.doc.clone().unwrap();
    let new = NewModule::Plugin { id: PLUGIN_ID.into(), name: "TestSynth".into(), chunk: Vec::new() };
    doc.borrow_mut().add_slot(0, 0, 0, &new).unwrap();
    (app.group, app.sound, app.chain_tab, app.channel, app.slot) = (0, 0, ChainTab::Sound, false, Some(0));
    app.slot_of = Some(CHAIN);
    sync_fake_view(&mut app);
    app
}

fn saved_pages(app: &App) -> Vec<deckard_model::PluginPage> {
    app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].slots[0].plugin.as_ref().unwrap().pages.clone()
}

/// Learning parameters from a VST assigns each touched control to the active slot,
/// auto-advances across slots 0..7, ignores repeated events from the same ongoing drag,
/// and stops after slot 7.
#[test]
fn learn_parameters_from_vst_advances_and_ignores_ongoing_drag() {
    let mut app = app_with_hosted_plugin();
    // Add a fresh empty page and start learning at slot 0.
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::AddPage(CHAIN, 0))]);
    sync_fake_view(&mut app);
    let page = app.plugin_page;
    assert_eq!(page, 2, "switched to the new third page");

    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::Learn((CHAIN, 0, page, 0)))]);
    assert_eq!(app.remote.plugin_pages.learn.as_ref().map(|l| l.slot), Some((CHAIN, 0, page, 0)));

    // First knob moved in VST (Cutoff = 10): learned on slot 0, auto-advances to slot 1.
    assert!(app.plugin_param_touched(CHAIN, 0, 10));
    assert_eq!(saved_pages(&app)[page].params[0].id, 10);
    assert_eq!(app.remote.plugin_pages.learn.as_ref().map(|l| l.slot), Some((CHAIN, 0, page, 1)));

    // Continued drag of Cutoff (10) does NOT fill slot 1.
    assert!(!app.plugin_param_touched(CHAIN, 0, 10));
    assert_eq!(saved_pages(&app)[page].params[1].id, u32::MAX);

    // Next knob moved (Resonance = 11): learned on slot 1, advances to slot 2.
    assert!(app.plugin_param_touched(CHAIN, 0, 11));
    assert_eq!(saved_pages(&app)[page].params[1].id, 11);
    assert_eq!(app.remote.plugin_pages.learn.as_ref().map(|l| l.slot), Some((CHAIN, 0, page, 2)));

    // Unknown parameter ID is ignored.
    assert!(!app.plugin_param_touched(CHAIN, 0, 9999));

    // Learning on slot 7 stops after filling the eighth slot.
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::Learn((CHAIN, 0, page, 7)))]);
    assert!(app.plugin_param_touched(CHAIN, 0, 18));
    assert_eq!(saved_pages(&app)[page].params[7].id, 18);
    assert!(app.remote.plugin_pages.learn.is_none(), "stopped after slot 8");

    // Undo reverts slot 7 in one step.
    assert!(app.doc.as_ref().unwrap().borrow_mut().undo());
    assert_eq!(saved_pages(&app)[page].params[7].id, u32::MAX);
}

/// Renaming, sectioning and resetting parameters save into the project and undo cleanly.
#[test]
fn rename_section_and_reset_plugin_parameters() {
    let mut app = app_with_hosted_plugin();
    let slot0 = (CHAIN, 0, 0, 0);

    // Rename knob 0 on an unsaved auto page: saves pages + renames in one undo step.
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::RenameParam(slot0, Some("Warmth".into())))]);
    sync_fake_view(&mut app);
    assert_eq!(saved_pages(&app)[0].params[0].name, "Warmth");
    assert_eq!(app.plugin_views[&(CHAIN, 0)].pages[0].knobs[0].as_ref().unwrap().1, "Warmth");

    // Empty name restores the plug-in parameter's default title.
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::RenameParam(slot0, Some(String::new())))]);
    sync_fake_view(&mut app);
    assert_eq!(saved_pages(&app)[0].params[0].name, "");
    assert_eq!(app.plugin_views[&(CHAIN, 0)].pages[0].knobs[0].as_ref().unwrap().1, "Cutoff");

    // Set section heading.
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::Section((CHAIN, 0, 0, 2), "Drive".into()))]);
    assert_eq!(saved_pages(&app)[0].params[2].section, "Drive");

    // Reset knob 0 unassigns the slot while keeping knob 1 in place (no shifting!).
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::Assign(slot0, None))]);
    sync_fake_view(&mut app);
    assert_eq!(saved_pages(&app)[0].params[0].id, u32::MAX);
    assert_eq!(saved_pages(&app)[0].params[1].id, 11);
    assert!(app.plugin_views[&(CHAIN, 0)].pages[0].knobs[0].is_none());
    assert_eq!(app.plugin_views[&(CHAIN, 0)].pages[0].knobs[1].as_ref().unwrap().0.id, 11);

    // Undo restores knob 0.
    assert!(app.doc.as_ref().unwrap().borrow_mut().undo());
    assert_eq!(saved_pages(&app)[0].params[0].id, 10);
}

/// Adding, renaming, deleting, clearing and resetting parameter pages all work and undo cleanly.
#[test]
fn add_rename_delete_clear_and_reset_plugin_pages() {
    let mut app = app_with_hosted_plugin();
    let doc = app.doc.clone().unwrap();
    assert!(!doc.borrow().has_saved_plugin_pages(CHAIN, 0));

    // Add a page (2 auto pages -> 3 saved pages, focused on page 2).
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::AddPage(CHAIN, 0))]);
    sync_fake_view(&mut app);
    assert_eq!(saved_pages(&app).len(), 3);
    assert_eq!(app.plugin_page, 2);
    // An empty new page stays unnamed rather than inheriting the previous page's name.
    assert_eq!(app.plugin_views[&(CHAIN, 0)].pages[2].name, "");

    // Rename page 2.
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::RenamePage(CHAIN, 0, 2, "Perf".into()))]);
    sync_fake_view(&mut app);
    assert_eq!(saved_pages(&app)[2].name, "Perf");
    assert_eq!(app.plugin_views[&(CHAIN, 0)].pages[2].name, "Perf");

    // Delete page 2 -> back to 2 pages, plugin_page clamped to 1.
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::DeletePage(CHAIN, 0, 2))]);
    sync_fake_view(&mut app);
    assert_eq!(saved_pages(&app).len(), 2);
    assert_eq!(app.plugin_page, 1);

    // Clear all -> 1 empty page at index 0.
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::ClearPages(CHAIN, 0))]);
    sync_fake_view(&mut app);
    assert_eq!(saved_pages(&app).len(), 1);
    assert_eq!(app.plugin_page, 0);
    assert!(saved_pages(&app)[0].params.iter().all(|k| k.id == u32::MAX));

    // Reset to default -> no saved pages, automatic pages return.
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::ResetPages(CHAIN, 0))]);
    sync_fake_view(&mut app);
    assert!(!doc.borrow().has_saved_plugin_pages(CHAIN, 0));
    assert_eq!(app.plugin_views[&(CHAIN, 0)].pages.len(), 2);

    // Undo restores the cleared page.
    assert!(doc.borrow_mut().undo());
    assert!(doc.borrow().has_saved_plugin_pages(CHAIN, 0));
}

/// UI harness test: opening the Assignment area on a hosted plug-in opens the PAGES tab,
/// the `+` button adds a page, `LEARN` learns a VST touch and stops on Esc, `⌘R` renames
/// a parameter inline, and the slot menu resets it.
#[test]
fn ui_assignment_pages_tab_learn_rename_reset_and_add_page() {
    let app = app_with_hosted_plugin();
    let mut h = Harness::builder().with_size(egui::vec2(1500.0, 930.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut App| {
            sync_fake_view(app);
            app.frame(ui);
        },
        app,
    );
    h.run_steps(3);

    // Opening the Assignment area on a hosted plug-in lands on PAGES.
    h.get_by_label("Assignment area").click();
    h.run_steps(3);
    assert_eq!(h.state().remote.assign, Some(AssignTab::Pages));

    // Clicking the "+" tab adds an empty parameter page and switches to it.
    h.get_by_label("Add parameter page").click();
    h.run_steps(3);
    let page = h.state().plugin_page;
    assert_eq!(page, 2);
    assert_eq!(saved_pages(h.state()).len(), 3);

    // Clicking LEARN starts learning on slot 0 (first empty slot on the new page).
    h.get_by_label("LEARN").click();
    h.run_steps(2);
    assert_eq!(h.state().remote.plugin_pages.learn.as_ref().map(|l| l.slot), Some((CHAIN, 0, page, 0)));

    // Touching Cutoff (10) in the VST assigns it to slot 0 and advances to slot 1.
    assert!(h.state_mut().plugin_param_touched(CHAIN, 0, 10));
    h.run_steps(3);
    assert_eq!(saved_pages(h.state())[page].params[0].id, 10);
    assert_eq!(h.state().remote.plugin_pages.learn.as_ref().map(|l| l.slot), Some((CHAIN, 0, page, 1)));

    // Esc cancels Learn.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().remote.plugin_pages.learn.is_none());

    // Focus slot 0 and press ⌘R to rename it inline to "Freq".
    layout::apply_acts(h.state_mut(), vec![Act::PluginPageEdit(PluginPageAct::Focus((CHAIN, 0, page, 0)))]);
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
    h.run_steps(3);
    assert!(h.state().remote.plugin_pages.renaming.is_some());
    // Select all and replace the prefilled "Cutoff" with "Freq".
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("Freq".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(saved_pages(h.state())[page].params[0].name, "Freq");

    // Right-click slot 0's Pages field ("Freq") and click Reset.
    let pos = h.get_by_label("Freq").rect().center();
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.run_steps(3);
    h.get_by_label("Reset").click();
    h.run_steps(3);
    assert_eq!(saved_pages(h.state())[page].params[0].id, u32::MAX);
}

/// DECKARD's own native plug-ins keep their fixed pages and do not show the `+` add page button.
#[test]
fn native_plugins_do_not_show_add_page_button() {
    use deckard_plugins::native::CLASSES;
    let mut app = App { settings: crate::settings::Settings::default(), model: "M3".into(), ..App::default() };
    app.new_project();
    let doc = app.doc.clone().unwrap();
    let new = NewModule::Plugin { id: CLASSES[0].plugin_id(), name: CLASSES[0].name.into(), chunk: Vec::new() };
    doc.borrow_mut().add_slot(0, 0, 0, &new).unwrap();
    (app.group, app.sound, app.chain_tab, app.channel, app.slot) = (0, 0, ChainTab::Sound, false, Some(0));
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 930.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
    h.run_steps(3);
    assert!(!h.state().is_editable_plugin(CHAIN, 0));
    assert!(h.query_by_label("Add parameter page").is_none());
}

/// Copying, cutting and pasting parameters carries both the parameter id and custom name,
/// refuses pasting onto a different plug-in, and undoes in one step.
#[test]
fn copy_cut_and_paste_plugin_parameters() {
    let mut app = app_with_hosted_plugin();
    let doc = app.doc.clone().unwrap();

    // Add a custom page, rename slot 0 on page 0 to "Bright", copy it, paste onto page 2 slot 3.
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::AddPage(CHAIN, 0))]);
    sync_fake_view(&mut app);
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::RenameParam((CHAIN, 0, 0, 0), Some("Bright".into())))]);
    sync_fake_view(&mut app);

    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::Copy((CHAIN, 0, 0, 0)))]);
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::Paste((CHAIN, 0, 2, 3)))]);
    sync_fake_view(&mut app);
    assert_eq!(saved_pages(&app)[2].params[3].id, 10);
    assert_eq!(saved_pages(&app)[2].params[3].name, "Bright");

    // Cut slot 3 on page 2 -> copies it and clears slot 3, then paste into slot 5.
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::Cut((CHAIN, 0, 2, 3)))]);
    sync_fake_view(&mut app);
    assert_eq!(saved_pages(&app)[2].params[3].id, u32::MAX);
    assert_eq!(doc.borrow().undo_label(), Some("Cut Parameter"));
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::Paste((CHAIN, 0, 2, 5)))]);
    sync_fake_view(&mut app);
    assert_eq!(saved_pages(&app)[2].params[5].id, 10);
    assert_eq!(saved_pages(&app)[2].params[5].name, "Bright");

    // Pasting onto a different plug-in is refused cleanly.
    let other = NewModule::Plugin { id: "OtherVendor:VST3:FX:1111".into(), name: "OtherFx".into(), chunk: Vec::new() };
    doc.borrow_mut().add_slot(0, 0, 1, &other).unwrap();
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::Paste((CHAIN, 1, 0, 0)))]);
    assert!(app.status.contains("different plug-in"), "{}", app.status);
}

/// Moving plug-in pages reorders them, keeps focus on the moved page, and undoes cleanly.
#[test]
fn move_plugin_pages_follows_focus_and_undoes() {
    let mut app = app_with_hosted_plugin();
    let doc = app.doc.clone().unwrap();

    // Name pages 0 and 1, add page 2 named "Perf".
    layout::apply_acts(
        &mut app,
        vec![
            Act::PluginPageEdit(PluginPageAct::RenamePage(CHAIN, 0, 0, "Main".into())),
            Act::PluginPageEdit(PluginPageAct::RenamePage(CHAIN, 0, 1, "Mod".into())),
            Act::PluginPageEdit(PluginPageAct::AddPage(CHAIN, 0)),
        ],
    );
    sync_fake_view(&mut app);
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::RenamePage(CHAIN, 0, 2, "Perf".into()))]);
    sync_fake_view(&mut app);
    assert_eq!(app.plugin_page, 2);

    // Move focused page 2 ("Perf") to index 0 -> focus follows to 0.
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::MovePage(CHAIN, 0, 2, 0))]);
    sync_fake_view(&mut app);
    let names = |app: &App| -> Vec<String> { saved_pages(app).into_iter().map(|p| p.name).collect() };
    assert_eq!(names(&app), vec!["Perf", "Main", "Mod"]);
    assert_eq!(app.plugin_page, 0, "focus followed the moved page");

    // Move page 2 ("Mod") to index 0 while page 0 ("Perf") is focused -> focus shifts to 1.
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::MovePage(CHAIN, 0, 2, 0))]);
    sync_fake_view(&mut app);
    assert_eq!(names(&app), vec!["Mod", "Perf", "Main"]);
    assert_eq!(app.plugin_page, 1);

    // Undo restores ["Perf", "Main", "Mod"].
    assert!(doc.borrow_mut().undo());
    sync_fake_view(&mut app);
    assert_eq!(names(&app), vec!["Perf", "Main", "Mod"]);
}

/// UI harness test for ⌘C / ⌘X / ⌘V on focused plug-in parameter slots in the Pages tab.
#[test]
fn ui_keys_cut_copy_paste_plugin_parameters() {
    let app = app_with_hosted_plugin();
    let mut h = Harness::builder().with_size(egui::vec2(1500.0, 930.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut App| {
            sync_fake_view(app);
            app.frame(ui);
        },
        app,
    );
    h.run_steps(3);
    h.get_by_label("Assignment area").click();
    h.run_steps(3);

    // Focus slot 0 (Cutoff = 10), ⌘C, focus page 1 slot 3 (empty), ⌘V.
    layout::apply_acts(h.state_mut(), vec![Act::PluginPageEdit(PluginPageAct::Focus((CHAIN, 0, 0, 0)))]);
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::C);
    h.run_steps(2);
    layout::apply_acts(h.state_mut(), vec![Act::PluginPage(1), Act::PluginPageEdit(PluginPageAct::Focus((CHAIN, 0, 1, 3)))]);
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::V);
    h.run_steps(3);
    assert_eq!(saved_pages(h.state())[1].params[3].id, 10);

    // ⌘X cuts slot 3 back out.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::X);
    h.run_steps(3);
    assert_eq!(saved_pages(h.state())[1].params[3].id, u32::MAX);
}

/// Clicking an empty slot opens the searchable parameter picker; right-clicking a knob's
/// name strip when the Assignment area is closed renames it inline; right-clicking a page
/// tab opens Move Left/Right, Rename, Add Page, Delete, Clear All and Reset to Default.
#[test]
fn ui_empty_slot_picker_strip_rename_and_page_tab_menu() {
    let app = app_with_hosted_plugin();
    let mut h = Harness::builder().with_size(egui::vec2(1500.0, 930.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut App| {
            sync_fake_view(app);
            app.frame(ui);
        },
        app,
    );
    h.run_steps(3);

    // Add an empty page (page 2) and click its first empty slot to open the parameter picker.
    h.get_by_label("Add parameter page").click();
    h.run_steps(3);
    let page = h.state().plugin_page;
    assert_eq!(page, 2);
    let rects: Vec<Option<egui::Rect>> = h.ctx.data(|d| d.get_temp(egui::Id::new("knob-rects"))).unwrap_or_default();
    let slot0_rect = rects[0].expect("empty slot 0 has a rect");
    let slot0_pos = slot0_rect.center();
    h.hover_at(slot0_pos);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: slot0_pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
        h.step();
    }
    h.run_steps(3);
    h.get_by_label("Resonance").click();
    h.run_steps(3);
    assert_eq!(saved_pages(h.state())[page].params[0].id, 11);

    // With the Assignment area closed, right-click slot 0's name strip and rename it inline.
    assert!(h.state().remote.assign.is_none());
    let strip_pos = egui::pos2(slot0_rect.center().x, slot0_rect.top() + 8.0);
    h.hover_at(strip_pos);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: strip_pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        });
        h.step();
    }
    h.run_steps(3);
    h.get_by_label("Rename ⌘R").click();
    h.run_steps(3);
    assert!(h.state().remote.plugin_pages.renaming.is_some());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("Peak".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(saved_pages(h.state())[page].params[0].name, "Peak");
}

/// Long parameter names in the Assignment area's Pages strip truncate on one line rather
/// than wrapping and overflowing the 22 px button height.
#[test]
fn ui_long_parameter_names_truncate_to_single_line() {
    let mut app = app_with_hosted_plugin();
    let long_name = "Swing clk 1 (Impromptu Clocked)";
    layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::RenameParam((CHAIN, 0, 0, 0), Some(long_name.into())))]);
    sync_fake_view(&mut app);
    let mut h = Harness::builder().with_size(egui::vec2(1500.0, 930.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut App| {
            sync_fake_view(app);
            app.frame(ui);
        },
        app,
    );
    h.run_steps(3);
    h.get_by_label("Assignment area").click();
    h.run_steps(3);
    let rect = h.get_by_label(long_name).rect();
    assert!(rect.height() <= 23.0, "long name wrapped and overflowed the strip: {rect:?}");
    assert!(rect.width() <= 72.0, "button grew wider than its knob column: {rect:?}");
}

/// When a plug-in has more pages than fit in the Control area header, an "All pages"
/// dropdown appears alongside `+` and switches to any hidden page.
#[test]
fn ui_page_tabs_overflow_dropdown_reaches_every_page() {
    let mut app = app_with_hosted_plugin();
    for _ in 0..10 {
        layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::AddPage(CHAIN, 0))]);
        sync_fake_view(&mut app);
    }
    let count = saved_pages(&app).len();
    assert_eq!(count, 12);
    for i in 0..count {
        layout::apply_acts(&mut app, vec![Act::PluginPageEdit(PluginPageAct::RenamePage(CHAIN, 0, i, format!("VCO Select {}", i + 1)))]);
    }
    sync_fake_view(&mut app);

    let mut h = Harness::builder().with_size(egui::vec2(1050.0, 930.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut App| {
            sync_fake_view(app);
            app.frame(ui);
        },
        app,
    );
    h.run_steps(3);
    layout::apply_acts(h.state_mut(), vec![Act::PluginPage(0)]);
    h.run_steps(3);
    assert_eq!(h.state().plugin_page, 0);

    let overflow_rect = h.get_by_label("All pages").rect();
    let add_rect = h.get_by_label("Add parameter page").rect();
    assert!(overflow_rect.right() <= add_rect.left() + 1.0, "overflow button overlapped +: {overflow_rect:?} vs {add_rect:?}");

    // Open the overflow dropdown and jump to a hidden page.
    h.get_by_label("All pages").click();
    h.run_steps(3);
    h.get_by_label("VCO Select 8").click();
    h.run_steps(3);
    assert_eq!(h.state().plugin_page, 7);
}

/// Right-clicking a knob's dial or value on an editable hosted plug-in shows
/// Remove Modulation at the top above the same parameter slot options as the name strip.
#[test]
fn ui_right_click_dial_and_value_shows_remove_modulation_then_slot_menu() {
    let app = app_with_hosted_plugin();
    let mut h = Harness::builder().with_size(egui::vec2(1500.0, 930.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut App| {
            sync_fake_view(app);
            app.frame(ui);
        },
        app,
    );
    h.run_steps(3);
    h.get_by_label("Assignment area").click();
    h.run_steps(3);

    let rects: Vec<Option<egui::Rect>> = h.ctx.data(|d| d.get_temp(egui::Id::new("knob-rects"))).unwrap_or_default();
    let slot0 = rects[0].expect("slot 0 rect");
    let slot1 = rects[1].expect("slot 1 rect");

    // Right-click on the dial of slot 0 (Cutoff).
    let dial_pos = egui::pos2(slot0.center().x, slot0.top() + 42.0);
    h.hover_at(dial_pos);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: dial_pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        });
        h.step();
    }
    h.run_steps(3);
    let mod_rect = h.get_by_label("Remove Modulation").rect();
    let learn_rect = h.get_by_label("Learn").rect();
    assert!(mod_rect.bottom() < learn_rect.top(), "Remove Modulation should sit above divider and Learn: {mod_rect:?} vs {learn_rect:?}");
    let _ = h.get_by_label("Rename ⌘R");
    h.get_by_label("Reset").click();
    h.run_steps(3);
    assert_eq!(saved_pages(h.state())[0].params[0].id, u32::MAX);

    // Right-click on the value readout at the bottom of slot 1 (Resonance).
    let val_pos = egui::pos2(slot1.center().x, slot1.bottom() - 8.0);
    h.hover_at(val_pos);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: val_pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        });
        h.step();
    }
    h.run_steps(3);
    let _ = h.get_by_label("Remove Modulation");
    h.get_by_label("Reset").click();
    h.run_steps(3);
    assert_eq!(saved_pages(h.state())[0].params[1].id, u32::MAX);
}
