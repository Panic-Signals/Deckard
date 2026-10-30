//! The Assignment area under the Control area's knobs (software manual: "Audio routing,
//! remote control, and Macro Controls"): the Pages pane (each macro's target, its slot
//! menu) and the Automation pane's MIDI (Learn) and Host (Enable) tabs, a field under
//! each knob.

use super::*;
use crate::remote::{AssignTab, MacroSlot, RemoteAct};

/// How tall the area is.
pub const ASSIGN_H: f32 = 34.0;

/// Where the area goes and what it is under.
pub(super) struct Fields<'a> {
    /// The strip under the knobs, and where the tabs go (under the channel's column).
    pub area: Rect,
    pub tabs: Rect,
    /// Each knob's rectangle (`None` where there's none).
    pub knobs: Vec<Option<Rect>>,
    pub params: &'a [Option<Knob>],
    pub macro_view: Option<&'a MacroView>,
}

pub(super) fn area(ui: &mut Ui, app: &App, doc: &deckard_doc::Document, painter: &Painter, f: Fields, gcol: Color32, acts: &mut Vec<Act>) {
    let Some(tab) = app.remote.assign else { return };
    painter.hline(f.area.x_range(), f.area.top(), Stroke::new(1.0, Color32::BLACK));
    // The tabs: PAGES (the Macro properties only), MIDI, MOD. HOST stays hidden until
    // DECKARD runs as a plug-in: without a host its IDs do nothing.
    let mut x = f.tabs.left() + 6.0;
    let sound_chain = match app.chain() {
        deckard_doc::Chain::Sound(g, s) if f.macro_view.is_none() => Some((g, s)),
        _ => None,
    };
    for (t, name, enabled) in [
        (AssignTab::Pages, "PAGES", f.macro_view.is_some()),
        (AssignTab::Midi, "MIDI", true),
        (AssignTab::Mod, "MOD", sound_chain.is_some()),
    ] {
        let w = painter.layout_no_wrap(name.to_string(), font(11.0), Color32::WHITE).size().x + 8.0;
        let r = Rect::from_min_size(Pos2::new(x, f.tabs.top() + 6.0), Vec2::new(w, 22.0));
        let resp = ui.interact(r, ui.id().with(("assign-tab", name)), if enabled { Sense::click() } else { Sense::hover() });
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
        let on = tab == t && enabled;
        let ink = if !enabled {
            white(60)
        } else if on {
            white(245)
        } else if resp.hovered() {
            white(164)
        } else {
            white(120)
        };
        painter.text(r.center(), Align2::CENTER_CENTER, name, font(11.0), ink);
        if on {
            painter.hline(r.shrink2(Vec2::new(2.0, 0.0)).x_range(), r.bottom() - 1.0, Stroke::new(2.0, white(245)));
        }
        if resp.clicked() {
            acts.push(Act::Remote(RemoteAct::Assign(Some(t))));
        }
        x += w + 6.0;
    }
    if tab == AssignTab::Mod
        && let Some((g, s)) = sound_chain
        && let Some(snd) = doc.project().groups.get(g).and_then(|x| x.sounds.get(s))
    {
        // At the strip's right end, past the knobs' fields.
        let r = Rect::from_min_size(Pos2::new(f.area.right() - 96.0, f.area.top() + 6.0), Vec2::new(90.0, 22.0));
        super::mod_routes::routes_list(ui, r, snd, &doc.project().lfos, g, s, acts);
    }
    let field = |i: usize| {
        f.knobs.get(i).copied().flatten().map(|k| Rect::from_min_size(Pos2::new(k.left(), f.area.top() + 6.0), Vec2::new(k.width(), 22.0)))
    };
    // Learning blinks.
    let blink = (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
    if app.remote.midi_learn.is_some() || app.remote.macro_learn.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    }
    match (tab, f.macro_view) {
        (AssignTab::Pages, Some((level, page, which, targets))) => {
            let ch = channel::macro_channel(*level);
            let macros = doc.macro_pages(ch).get(*page).map(|p| p.macros.clone()).unwrap_or_default();
            for (i, m) in macros.iter().enumerate() {
                let Some(r) = field(i) else { continue };
                let slot: MacroSlot = (ch, *page, i);
                if let Some(k) = f.knobs.get(i).copied().flatten() {
                    if app.remote.macro_learn == Some(slot) {
                        if blink {
                            painter.rect_stroke(k.expand(2.0), 4.0, Stroke::new(1.5, gcol), StrokeKind::Outside);
                        }
                    } else if app.remote.macro_focus == Some(slot) {
                        painter.rect_stroke(k.expand(2.0), 4.0, Stroke::new(1.0, white(200)), StrokeKind::Outside);
                    }
                }
                if let Some((at, text)) = &app.remote.renaming
                    && *at == slot
                {
                    rename_field(ui, r, slot, text, acts);
                    continue;
                }
                let text = match (m.target, which.get(i).copied().flatten()) {
                    (Some(_), Some(k)) => targets[k].path.last().cloned().unwrap_or_default(),
                    // (A plug-in's parameter: its name.)
                    (Some(id), None) => super::plugin_macro_knob(app, doc, id).map_or("(Other)".into(), |k| k.0.to_string()),
                    (None, _) => "Select".into(),
                };
                let resp = ui.put(r, egui::Button::new(egui::RichText::new(text).size(10.0)).fill(S02));
                if resp.clicked() {
                    acts.push(Act::Remote(RemoteAct::Focus(slot)));
                }
                // The target selector.
                egui::Popup::menu(&resp)
                    .id(ui.id().with(("macro-target", *page, i)))
                    .show(|ui| super::macro_target_menu(ui, app, *level, slot, targets, acts));
                resp.context_menu(|ui| macro_slot_menu(ui, app, slot, m.target.is_some(), acts));
            }
        }
        (AssignTab::Mod, _) => {
            let Some((g, s)) = sound_chain else {
                let at = Pos2::new(f.area.left() + 13.0, f.area.center().y);
                painter.text(at, Align2::LEFT_CENTER, "MOD: for a sound's instrument and effects", font(11.0), white(90));
                return;
            };
            let routes = doc.project().groups.get(g).and_then(|x| x.sounds.get(s)).map(|x| x.mod_routes.clone()).unwrap_or_default();
            for i in 0..f.params.len() {
                let Some(r) = field(i) else { continue };
                let target = f.params[i]
                    .as_ref()
                    .and_then(|k| k.3.as_ref())
                    .and_then(|e| super::mod_routes::knob_route_target(doc.project(), app.chain(), e));
                match target {
                    Some(t) => super::mod_routes::knob_field(ui, r, &routes, &doc.project().lfos, t, acts),
                    None => {
                        ui.add_enabled_ui(false, |ui| ui.put(r, egui::Button::new(egui::RichText::new("—").size(10.0))));
                    }
                }
            }
        }
        (AssignTab::Pages, None) => {
            let at = Pos2::new(f.area.left() + 13.0, f.area.center().y);
            painter.text(at, Align2::LEFT_CENTER, "Pages: for the Macro properties", font(11.0), white(90));
        }
        (AssignTab::Midi | AssignTab::Host, mv) => {
            for i in 0..f.params.len() {
                let Some(r) = field(i) else { continue };
                // On the Macro properties, the macro itself; elsewhere the knob's parameter.
                let edit = f.params[i].as_ref().and_then(|k| k.3);
                let param = match mv {
                    Some((level, page, ..)) => doc.macro_oid(channel::macro_channel(*level), *page, i),
                    None => edit.and_then(|e| super::edit_param_oid(app, doc, e)),
                };
                // A plug-in's parameter learns even before it has an object (made as it learns).
                let plugin = match (mv, edit) {
                    (None, Some(super::Edit::Plugin(s, id, _) | super::Edit::PluginChoice(s, id, ..))) => Some((app.chain(), s, id)),
                    _ => None,
                };
                if param.is_none() && (plugin.is_none() || tab != AssignTab::Midi) {
                    ui.add_enabled_ui(false, |ui| ui.put(r, egui::Button::new(egui::RichText::new("—").size(10.0))));
                    continue;
                }
                if tab == AssignTab::Midi {
                    let key = param.and_then(|p| doc.midi_key_for(p));
                    let learning = match plugin {
                        Some(p) => app.remote.plugin_learn == Some(p),
                        None => app.remote.midi_learn == param,
                    };
                    let text = match key {
                        _ if learning => if blink { "Learning" } else { "" }.to_string(),
                        Some(k) => deckard_doc::remote::midi_key_name(k),
                        None => "Learn".into(),
                    };
                    let resp = ui.put(r, egui::Button::new(egui::RichText::new(text).size(10.0)).fill(if learning { gcol } else { S02 }));
                    let resp = resp.on_hover_text("Click, then move a control on your MIDI controller (Esc: stop)");
                    if resp.clicked() {
                        acts.push(Act::Remote(match (plugin, param) {
                            (Some((c, s, id)), _) => RemoteAct::PluginMidiLearn(c, s, id),
                            (None, p) => RemoteAct::MidiLearn(p.unwrap_or_default()),
                        }));
                    }
                    if let (Some(_), Some(param)) = (key, param) {
                        resp.context_menu(|ui| {
                            if ui.button("Unlearn").clicked() {
                                acts.push(Act::Remote(RemoteAct::Unlearn(param)));
                                ui.close();
                            }
                        });
                    }
                } else if let Some(param) = param {
                    let id = doc.host_slot_for(param);
                    let text = id.map_or("Enable".to_string(), |s| format!("ID {}", s + 1));
                    let resp = ui
                        .put(r, egui::Button::new(egui::RichText::new(text).size(10.0)).fill(S02))
                        .on_hover_text("An automation ID for the host (Maschine as a plug-in); click again to remove it");
                    if resp.clicked() {
                        acts.push(Act::Remote(RemoteAct::Host(param)));
                    }
                }
            }
        }
    }
}

/// A macro's name being edited in its field: Enter keeps it, Esc (or clicking away)
/// doesn't.
fn rename_field(ui: &mut Ui, r: Rect, slot: MacroSlot, start: &str, acts: &mut Vec<Act>) {
    let id = ui.id().with(("macro-rename", slot.1, slot.2));
    let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| start.to_string());
    let resp = ui.put(r, egui::TextEdit::singleline(&mut text).font(font(10.0)));
    if !ui.data(|d| d.get_temp::<bool>(id.with("focused")).unwrap_or(false)) {
        resp.request_focus();
        ui.data_mut(|d| d.insert_temp(id.with("focused"), true));
    }
    ui.data_mut(|d| d.insert_temp(id, text.clone()));
    if resp.lost_focus() {
        let enter = ui.input(|x| x.key_pressed(egui::Key::Enter));
        acts.push(Act::Remote(if enter { RemoteAct::Rename(slot, Some(text)) } else { RemoteAct::Cancel }));
        ui.data_mut(|d| {
            d.remove::<String>(id);
            d.remove::<bool>(id.with("focused"));
        });
    }
}

/// The macro slot menu: Rename ⌘R, Section, Learn, Reset, Cut ⌘X, Copy ⌘C, Paste ⌘V.
pub(super) fn macro_slot_menu(ui: &mut Ui, app: &App, slot: MacroSlot, assigned: bool, acts: &mut Vec<Act>) {
    let (ch, page, m) = slot;
    let mut item = |ui: &mut Ui, enabled: bool, label: &str, keys: &str, act: Act| {
        let b = if keys.is_empty() { egui::Button::new(label) } else { egui::Button::new(label).shortcut_text(keys) };
        if ui.add_enabled(enabled, b).clicked() {
            acts.push(Act::Remote(RemoteAct::Focus(slot)));
            acts.push(act);
            ui.close();
        }
    };
    item(ui, true, "Rename", "⌘R", Act::Remote(RemoteAct::Rename(slot, None)));
    let learning = app.remote.macro_learn == Some(slot);
    item(ui, true, if learning { "Stop Learning" } else { "Learn" }, "", Act::Remote(RemoteAct::Learn(slot)));
    item(ui, assigned, "Reset", "", Act::Macro(MacroAct::Reset { ch, page, slot: m }));
    ui.separator();
    item(ui, assigned, "Cut", "⌘X", Act::Remote(RemoteAct::Cut(slot)));
    item(ui, assigned, "Copy", "⌘C", Act::Remote(RemoteAct::Copy(slot)));
    item(ui, app.remote.macro_clip.is_some(), "Paste", "⌘V", Act::Remote(RemoteAct::Paste(slot)));
    ui.separator();
    // The section heading that starts here.
    let id = ui.id().with(("macro-section", page, m));
    let current = app.doc.as_ref().and_then(|d| d.borrow().macro_pages(ch).get(page)?.macros.get(m).map(|x| x.section.clone()));
    let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| current.unwrap_or_default());
    ui.label("Section");
    let te = ui.text_edit_singleline(&mut text);
    ui.data_mut(|d| d.insert_temp(id, text.clone()));
    if te.lost_focus() && ui.input(|x| x.key_pressed(egui::Key::Enter)) {
        acts.push(Act::Macro(MacroAct::Section { ch, page, slot: m, name: text }));
        ui.data_mut(|d| d.remove::<String>(id));
        ui.close();
    }
}
