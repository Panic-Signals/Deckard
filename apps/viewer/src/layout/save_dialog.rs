//! The Save dialog (Save As…, Save with Samples…, Save Preset…): the name, the tags
//! the browser lists it by (Types, Characters, Library), and the folder; it asks
//! before replacing a file.

use super::white;
use crate::App;
use eframe::egui::{self, RichText};

pub fn show(app: &mut App, ctx: &egui::Context) {
    let mut save = false;
    let mut cancel = false;
    let mut choose = false;
    let suggestions = (app.browser.results.types.clone(), app.browser.results.characters.clone());
    let Some(d) = &mut app.save_dialog else { return };
    let title = match d.what {
        deckard_doc::FileItem::Sound(..) => "Save Sound",
        deckard_doc::FileItem::Group(_) => "Save Group",
        deckard_doc::FileItem::Slot(..) => "Save Preset",
    };
    egui::Modal::new(egui::Id::new("save-dialog")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.heading(title);
        egui::Grid::new("save-fields").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Name");
            let name = ui.text_edit_singleline(&mut d.name);
            if name.changed() {
                d.confirm_overwrite = false;
            }
            ui.end_row();
            ui.label("Types");
            ui.add(egui::TextEdit::singleline(&mut d.types).hint_text("Drums › Kick; Percussion"));
            ui.end_row();
            ui.label("Characters");
            ui.add(egui::TextEdit::singleline(&mut d.characters).hint_text("Ambient, Dark"));
            ui.end_row();
            ui.label("Library");
            ui.text_edit_singleline(&mut d.library);
            ui.end_row();
            ui.label("Folder");
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(d.folder.display().to_string()).size(10.0).color(white(180))).truncate());
                if ui.button("Choose…").clicked() {
                    choose = true;
                }
            });
            ui.end_row();
        });
        // Tags the library already has, to pick from.
        let (types, characters) = &suggestions;
        if !types.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Types:").size(10.0).color(white(142)));
                for (t, _) in types.iter().take(16) {
                    if ui.small_button(t).clicked() {
                        if !d.types.trim().is_empty() {
                            d.types.push_str("; ");
                        }
                        d.types.push_str(t);
                    }
                }
            });
        }
        if !characters.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Characters:").size(10.0).color(white(142)));
                for (c, _) in characters.iter().take(16) {
                    if ui.small_button(c).clicked() {
                        if !d.characters.trim().is_empty() {
                            d.characters.push_str(", ");
                        }
                        d.characters.push_str(c);
                    }
                }
            });
        }
        ui.checkbox(&mut d.samples, "With samples (copied beside it)");
        if d.confirm_overwrite {
            ui.colored_label(egui::Color32::from_rgb(0xE3, 0x42, 0x3F), format!("\"{}\" is there already: replace it?", d.name.trim()));
        }
        ui.horizontal(|ui| {
            if ui.button(if d.confirm_overwrite { "Replace" } else { "Save" }).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                save = true;
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });
    });
    if choose && let Some(dir) = rfd::FileDialog::new().set_directory(&d.folder).pick_folder() {
        d.folder = dir;
        d.confirm_overwrite = false;
    }
    if cancel {
        app.save_dialog = None;
    } else if save {
        app.confirm_save();
    }
}
