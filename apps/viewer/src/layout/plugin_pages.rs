//! UI for editing hosted plug-in parameter pages: page tab context menus, the `+` add
//! page tab, empty parameter slots, parameter slot context menus (Learn, Assign, Rename,
//! Reset, Section), searchable parameter picker, and the Assignment area's Pages tab.

use super::*;
use crate::plugin_pages::{PluginKnobSlot, PluginPageAct};
use deckard_doc::Chain;

/// Summary of the currently shown editable hosted plug-in page, passed to the knob row
/// and the Assignment area.
pub(super) struct PluginPageView {
    pub chain: Chain,
    pub slot: usize,
    pub page: usize,
    pub plugin_id: String,
    pub has_saved: bool,
    pub page_count: usize,
    /// Current page's 8 slots: `(param_id, display_label, section)` when assigned.
    pub knobs: [Option<(u32, String, String)>; 8],
}

/// Draw the `+` button for adding a parameter page after the plug-in's tabs, drag-and-drop
/// reordering of parameter page tabs, plus right-click menus on each parameter page tab
/// (Rename, Move Left/Right, Add, Delete, Clear All, Reset).
#[allow(clippy::too_many_arguments)]
pub(super) fn tab_controls(
    ui: &mut Ui,
    painter: &Painter,
    tab_area: Rect,
    tab_resps: &[Response],
    names: &[String],
    pv: &PluginPageView,
    acts: &mut Vec<Act>,
) {
    let shown_pages = pv.page_count.min(tab_resps.len());
    // Drag a parameter page tab horizontally to reorder pages (never Side-Chain Input).
    if shown_pages > 1 {
        for (i, resp) in tab_resps.iter().enumerate().take(shown_pages) {
            if (resp.dragged() || resp.drag_stopped())
                && let Some(pos) = ui.ctx().pointer_latest_pos()
            {
                let to = tab_resps[..shown_pages]
                    .iter()
                    .position(|r| pos.x < r.rect.center().x)
                    .map_or(shown_pages - 1, |idx| if idx > i { idx - 1 } else { idx });
                if resp.drag_stopped() {
                    if to != i {
                        acts.push(Act::PluginPageEdit(PluginPageAct::MovePage(pv.chain, pv.slot, i, to)));
                    }
                } else {
                    let drop_x = if to > i { tab_resps[to].rect.right() + 1.0 } else { tab_resps[to].rect.left() - 1.0 };
                    ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.id().with("plugin-page-drop"))).vline(
                        drop_x,
                        tab_area.y_range().min + 4.0..=tab_area.y_range().max - 4.0,
                        Stroke::new(2.0, ink()),
                    );
                }
            }
        }
    }
    // Right-click menu on each parameter page tab (never on Side-Chain Input).
    for (i, resp) in tab_resps.iter().enumerate().take(shown_pages) {
        let current_name = names.get(i).cloned().unwrap_or_default();
        resp.menu_on_right_click(|ui| {
            let id = ui.id().with(("plugin-page-name", pv.slot, i));
            let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| current_name.clone());
            ui.label("Name");
            let te = ui.text_edit_singleline(&mut text);
            ui.data_mut(|d| d.insert_temp(id, text.clone()));
            if te.lost_focus() && ui.input(|x| x.key_pressed(egui::Key::Enter)) {
                acts.push(Act::PluginPageEdit(PluginPageAct::RenamePage(pv.chain, pv.slot, i, text)));
                ui.data_mut(|d| d.remove::<String>(id));
                ui.close();
            }
            ui.separator();
            if ui.add_enabled(i > 0, egui::Button::new("Move Left")).on_hover_text("Move this page one position left").clicked() {
                acts.push(Act::PluginPageEdit(PluginPageAct::MovePage(pv.chain, pv.slot, i, i - 1)));
                ui.close();
            }
            if ui
                .add_enabled(i + 1 < pv.page_count, egui::Button::new("Move Right"))
                .on_hover_text("Move this page one position right")
                .clicked()
            {
                acts.push(Act::PluginPageEdit(PluginPageAct::MovePage(pv.chain, pv.slot, i, i + 1)));
                ui.close();
            }
            ui.separator();
            if ui.button("Add Page").on_hover_text("Add a new empty parameter page").clicked() {
                acts.push(Act::PluginPageEdit(PluginPageAct::AddPage(pv.chain, pv.slot)));
                ui.close();
            }
            let del_hint = if pv.page_count > 1 { "Delete this parameter page" } else { "Clear all parameters on this page" };
            if ui.button("Delete").on_hover_text(del_hint).clicked() {
                acts.push(Act::PluginPageEdit(PluginPageAct::DeletePage(pv.chain, pv.slot, i)));
                ui.close();
            }
            if ui.button("Clear All").on_hover_text("Replace all pages with one empty page").clicked() {
                acts.push(Act::PluginPageEdit(PluginPageAct::ClearPages(pv.chain, pv.slot)));
                ui.close();
            }
            if ui
                .add_enabled(pv.has_saved, egui::Button::new("Reset to Default"))
                .on_hover_text("Restore the plug-in's default parameter pages")
                .clicked()
            {
                acts.push(Act::PluginPageEdit(PluginPageAct::ResetPages(pv.chain, pv.slot)));
                ui.close();
            }
        });
    }
    // "+" button to add a new empty parameter page, placed after the last drawn tab
    // (pinned at the right edge of `tab_area` if many pages filled the bar).
    let plus_w = 22.0;
    let x = tab_resps.last().map_or(tab_area.left(), |r| r.rect.right() + 2.0).min(tab_area.right() - plus_w);
    if x >= tab_area.left() && x + plus_w <= tab_area.right() + 1.0 {
        let r = Rect::from_min_max(Pos2::new(x, tab_area.top() + 4.0), Pos2::new(x + plus_w, tab_area.bottom() - 4.0));
        let resp = ui.interact(r, ui.id().with(("plugin-page-add", pv.slot)), Sense::click()).on_hover_text("Add parameter page");
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Add parameter page"));
        plus_glyph(painter, r.center(), 11.0, if resp.hovered() { ink() } else { white(142) });
        if resp.clicked() {
            acts.push(Act::PluginPageEdit(PluginPageAct::AddPage(pv.chain, pv.slot)));
        }
    }
}

/// Draw an empty slot on an editable hosted plug-in page (circle + `+` glyph, or a
/// blinking Learn indicator while waiting for a VST/AU knob).
#[allow(clippy::too_many_arguments)]
pub(super) fn empty_slot(
    ui: &mut Ui,
    app: &App,
    painter: &Painter,
    r: Rect,
    pv: &PluginPageView,
    i: usize,
    gcol: Color32,
    acts: &mut Vec<Act>,
) {
    let slot: PluginKnobSlot = (pv.chain, pv.slot, pv.page, i);
    let learning = app.remote.plugin_pages.learn.as_ref().is_some_and(|l| l.slot == slot);
    let blink = (ui.input(|x| x.time) * 2.0) as i64 % 2 == 0;
    if learning {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        if blink {
            painter.rect_stroke(r.expand(2.0), 4.0, Stroke::new(1.5, gcol), StrokeKind::Outside);
        }
    }
    let resp = ui.interact(r, ui.id().with(("plugin-empty-slot", pv.slot, pv.page, i)), Sense::click()).on_hover_text(if learning {
        "Move a parameter in the plug-in window (click or Esc to stop)"
    } else {
        "Click to learn or pick a plug-in parameter"
    });
    if resp.hovered() {
        painter.rect_filled(r, 4.0, white(10));
    }
    let title = if learning { if blink { "LEARNING" } else { "" } } else { "" };
    if !title.is_empty() {
        painter.text(Pos2::new(r.center().x, r.top() + 8.0), Align2::CENTER_CENTER, title, font(10.0), gcol);
    }
    let stroke = if learning { gcol } else { white(30) };
    painter.circle_stroke(Pos2::new(r.center().x, r.top() + 42.0), 18.0, Stroke::new(1.0, stroke));
    plus_glyph(
        painter,
        Pos2::new(r.center().x, r.top() + 42.0),
        14.0,
        if learning { gcol } else { white(if resp.hovered() { 200 } else { 70 }) },
    );
    if learning && resp.clicked() {
        acts.push(Act::PluginPageEdit(PluginPageAct::Learn(slot)));
        return;
    }
    egui::Popup::menu(&resp)
        .style(crate::layout::menu_style)
        .id(ui.id().with(("plugin-empty-pick", pv.slot, pv.page, i)))
        .show(|ui| param_picker_menu(ui, app, slot, true, acts));
    resp.menu_on_right_click(|ui| slot_menu(ui, app, None, pv, i, acts));
}

/// Inline rename text box for a parameter slot (Enter commits, Esc or click away cancels).
pub(super) fn rename_field(ui: &mut Ui, r: Rect, slot: PluginKnobSlot, start: &str, id_tag: &str, acts: &mut Vec<Act>) {
    let id = ui.id().with((id_tag, slot.1, slot.2, slot.3));
    let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| start.to_string());
    let resp = ui.put(r, egui::TextEdit::singleline(&mut text).font(font(10.0)));
    if !ui.data(|d| d.get_temp::<bool>(id.with("focused")).unwrap_or(false)) {
        resp.request_focus();
        ui.data_mut(|d| d.insert_temp(id.with("focused"), true));
    }
    ui.data_mut(|d| d.insert_temp(id, text.clone()));
    if resp.lost_focus() {
        let enter = ui.input(|x| x.key_pressed(egui::Key::Enter));
        acts.push(Act::PluginPageEdit(if enter { PluginPageAct::RenameParam(slot, Some(text)) } else { PluginPageAct::Cancel }));
        ui.data_mut(|d| {
            d.remove::<String>(id);
            d.remove::<bool>(id.with("focused"));
        });
    }
}

/// Context menu for a hosted plug-in parameter slot: Learn, Assign Parameter, Rename,
/// Reset, Section, plus the existing Create/Remove Macro items when `doc` is passed.
pub(super) fn slot_menu(ui: &mut Ui, app: &App, doc: Option<&deckard_doc::Document>, pv: &PluginPageView, i: usize, acts: &mut Vec<Act>) {
    let slot: PluginKnobSlot = (pv.chain, pv.slot, pv.page, i);
    let assigned = pv.knobs[i].as_ref();
    let learning = app.remote.plugin_pages.learn.as_ref().is_some_and(|l| l.slot == slot);
    if ui
        .button(if learning { "Stop Learning" } else { "Learn" })
        .on_hover_text("Move a parameter in the plug-in window to assign it here")
        .clicked()
    {
        acts.push(Act::PluginPageEdit(PluginPageAct::Learn(slot)));
        ui.close();
    }
    ui.menu_button("Assign Parameter", |ui| {
        param_picker_menu(ui, app, slot, false, acts);
    });
    if ui
        .add_enabled(assigned.is_some(), egui::Button::new("Rename").shortcut_text("⌘R"))
        .on_hover_text("Rename this parameter (empty restores the plug-in's name)")
        .clicked()
    {
        acts.push(Act::PluginPageEdit(PluginPageAct::RenameParam(slot, None)));
        ui.close();
    }
    if ui.add_enabled(assigned.is_some(), egui::Button::new("Reset")).on_hover_text("Remove this parameter from the slot").clicked() {
        acts.push(Act::PluginPageEdit(PluginPageAct::Assign(slot, None)));
        ui.close();
    }
    ui.separator();
    let can_paste = app.remote.plugin_pages.clip.as_ref().is_some_and(|c| c.plugin_id == pv.plugin_id);
    if ui.add_enabled(assigned.is_some(), egui::Button::new("Cut").shortcut_text("⌘X")).clicked() {
        acts.push(Act::PluginPageEdit(PluginPageAct::Focus(slot)));
        acts.push(Act::PluginPageEdit(PluginPageAct::Cut(slot)));
        ui.close();
    }
    if ui.add_enabled(assigned.is_some(), egui::Button::new("Copy").shortcut_text("⌘C")).clicked() {
        acts.push(Act::PluginPageEdit(PluginPageAct::Focus(slot)));
        acts.push(Act::PluginPageEdit(PluginPageAct::Copy(slot)));
        ui.close();
    }
    if ui.add_enabled(can_paste, egui::Button::new("Paste").shortcut_text("⌘V")).clicked() {
        acts.push(Act::PluginPageEdit(PluginPageAct::Focus(slot)));
        acts.push(Act::PluginPageEdit(PluginPageAct::Paste(slot)));
        ui.close();
    }
    if let Some((_, _, cur_section)) = assigned {
        ui.separator();
        let id = ui.id().with(("plugin-section", pv.slot, pv.page, i));
        let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| cur_section.clone());
        ui.label("Section");
        let te = ui.text_edit_singleline(&mut text);
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
        if te.lost_focus() && ui.input(|x| x.key_pressed(egui::Key::Enter)) {
            acts.push(Act::PluginPageEdit(PluginPageAct::Section(slot, text)));
            ui.data_mut(|d| d.remove::<String>(id));
            ui.close();
        }
    }
    if let (Some(doc), Some((param_id, ..))) = (doc, assigned) {
        ui.separator();
        super::plugin_macro_menu(ui, doc, pv.chain, pv.slot, *param_id, acts);
    }
}

/// Searchable list of the plug-in's parameters (and an optional `Learn from Plug-in`
/// button at the top).
pub(super) fn param_picker_menu(ui: &mut Ui, app: &App, slot: PluginKnobSlot, with_learn: bool, acts: &mut Vec<Act>) {
    let (chain, plug, ..) = slot;
    if with_learn {
        let learning = app.remote.plugin_pages.learn.as_ref().is_some_and(|l| l.slot == slot);
        if ui
            .button(if learning { "Stop Learning" } else { "Learn from Plug-in" })
            .on_hover_text("Move a parameter in the plug-in window to assign it here")
            .clicked()
        {
            acts.push(Act::PluginPageEdit(PluginPageAct::Learn(slot)));
            ui.close();
            return;
        }
        ui.separator();
    }
    let params = app.all_plugin_params(chain, plug);
    if params.is_empty() {
        ui.label("No parameters reported by this plug-in");
        return;
    }
    let filter_id = ui.id().with(("plugin-param-filter", plug, slot.2, slot.3));
    let mut query = ui.data(|d| d.get_temp::<String>(filter_id)).unwrap_or_default();
    ui.add(egui::TextEdit::singleline(&mut query).hint_text("Filter parameters...").desired_width(190.0));
    ui.data_mut(|d| d.insert_temp(filter_id, query.clone()));
    let needle = query.trim().to_ascii_lowercase();
    let matched: Vec<&deckard_plugins::ParamInfo> = if needle.is_empty() {
        params.iter().collect()
    } else {
        params.iter().filter(|p| p.title.to_ascii_lowercase().contains(&needle)).collect()
    };
    if matched.is_empty() {
        ui.label("No matching parameters");
        return;
    }
    let row_h = ui.spacing().interact_size.y;
    egui::ScrollArea::vertical().id_salt(("plugin-param-list", plug, slot.2, slot.3)).max_height(240.0).show_rows(
        ui,
        row_h,
        matched.len(),
        |ui, range| {
            for info in &matched[range] {
                if ui.button(&info.title).clicked() {
                    acts.push(Act::PluginPageEdit(PluginPageAct::Assign(slot, Some(info.id))));
                    ui.data_mut(|d| d.remove::<String>(filter_id));
                    ui.close();
                }
            }
        },
    );
}

/// The Assignment area's `PAGES` tab for a hosted plug-in: a field under each of the
/// eight knob slots (Learn / Pick / Rename / Reset / Section) plus a `LEARN` toggle button
/// at the right end of the strip.
pub(super) fn assign_pages(
    ui: &mut Ui,
    app: &App,
    painter: &Painter,
    area: Rect,
    knobs: &[Option<Rect>],
    pv: &PluginPageView,
    gcol: Color32,
    acts: &mut Vec<Act>,
) {
    let blink = (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
    let learning_here = app.remote.plugin_pages.learn.as_ref().filter(|l| (l.slot.0, l.slot.1, l.slot.2) == (pv.chain, pv.slot, pv.page));
    if learning_here.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    }
    // LEARN toggle at the right end of the Assignment strip (as ROUTES sits in MOD).
    let learn_r = Rect::from_min_size(Pos2::new(area.right() - 80.0, area.top() + 6.0), Vec2::new(74.0, 22.0));
    let learn_on = learning_here.is_some();
    let learn_fill = if learn_on && blink {
        gcol
    } else if learn_on {
        s03()
    } else {
        s02()
    };
    let learn_btn = ui
        .put(learn_r, egui::Button::new(egui::RichText::new(if learn_on { "LEARNING" } else { "LEARN" }).size(10.0)).fill(learn_fill))
        .on_hover_text("Learn parameters from the plug-in window into this page (Esc to stop)");
    if learn_btn.clicked() {
        let target_slot = if let Some(l) = learning_here {
            l.slot
        } else {
            let k = app
                .remote
                .plugin_pages
                .focus
                .filter(|&(c, s, p, _)| (c, s, p) == (pv.chain, pv.slot, pv.page))
                .map(|(_, _, _, k)| k)
                .or_else(|| pv.knobs.iter().position(Option::is_none))
                .unwrap_or(0);
            (pv.chain, pv.slot, pv.page, k)
        };
        acts.push(Act::PluginPageEdit(PluginPageAct::Learn(target_slot)));
    }
    for i in 0..8 {
        let Some(k_rect) = knobs.get(i).copied().flatten() else { continue };
        let r = Rect::from_min_size(Pos2::new(k_rect.left(), area.top() + 6.0), Vec2::new(k_rect.width(), 22.0));
        let slot: PluginKnobSlot = (pv.chain, pv.slot, pv.page, i);
        let slot_learning = learning_here.is_some_and(|l| l.slot.3 == i);
        if slot_learning {
            if blink {
                painter.rect_stroke(k_rect.expand(2.0), 4.0, Stroke::new(1.5, gcol), StrokeKind::Outside);
            }
        } else if app.remote.plugin_pages.focus == Some(slot) {
            painter.rect_stroke(k_rect.expand(2.0), 4.0, Stroke::new(1.0, white(200)), StrokeKind::Outside);
        }
        if let Some((at, start)) = &app.remote.plugin_pages.renaming
            && *at == slot
        {
            rename_field(ui, r, slot, start, "plugin-assign-rename", acts);
            continue;
        }
        let text = if slot_learning {
            if blink { "Learning" } else { "" }.to_string()
        } else if let Some((_, label, _)) = &pv.knobs[i] {
            label.clone()
        } else {
            "Select".to_string()
        };
        let fill = if slot_learning { gcol } else { s02() };
        let resp = ui.put(r, egui::Button::new(egui::RichText::new(text).size(10.0)).fill(fill));
        if resp.clicked() {
            acts.push(Act::PluginPageEdit(PluginPageAct::Focus(slot)));
            if slot_learning {
                acts.push(Act::PluginPageEdit(PluginPageAct::Learn(slot)));
            }
        }
        if !slot_learning {
            egui::Popup::menu(&resp)
                .style(crate::layout::menu_style)
                .id(ui.id().with(("plugin-assign-pick", pv.slot, pv.page, i)))
                .show(|ui| param_picker_menu(ui, app, slot, true, acts));
        }
        resp.menu_on_right_click(|ui| slot_menu(ui, app, None, pv, i, acts));
    }
}
