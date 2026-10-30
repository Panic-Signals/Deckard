//! The browser panel, left of the arranger (the LIBRARY button, F4): the LIBRARY tab
//! (sections, Factory/User, favourites, product, Types, Characters, search, results,
//! prehear) and the FILES tab (a plain folder browser). As the approved mockup
//! (docs/superpowers/specs/2026-10-22-browser-design.md).

use super::{Act, S00, S01, S02, S03, apply, font, white};
use crate::App;
use crate::browser::{BrowseAct, Tab};
use deckard_library::Kind;
use eframe::egui::{self, Align2, Color32, RichText, Sense, Ui, Vec2};

pub const WIDTH: f32 = 300.0;
const ACCENT: Color32 = Color32::from_rgb(0xF3, 0x9C, 0x12);
const RESULT_ROW: f32 = 20.0;

/// What dragging a result carries: its index in the results.
#[derive(Clone, Copy)]
pub struct Dragged(pub usize);

/// What dragging a file in FILES carries.
#[derive(Clone)]
pub struct DraggedFile(pub std::path::PathBuf);

const KINDS: [(Kind, &str); 7] = [
    (Kind::Project, "Projects"),
    (Kind::Group, "Groups"),
    (Kind::Sound, "Sounds"),
    (Kind::Instrument, "Instruments"),
    (Kind::Effect, "Effects"),
    (Kind::Loop, "Loops"),
    (Kind::OneShot, "One-shots"),
];

pub fn panel(app: &mut App, ui: &mut Ui) {
    let mut acts = Vec::new();
    egui::Panel::left("browser").exact_size(WIDTH).resizable(false).frame(egui::Frame::new().fill(S00).inner_margin(6.0)).show(ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        ui.horizontal(|ui| {
            for (tab, label) in [(Tab::Library, "LIBRARY"), (Tab::Files, "FILES")] {
                if chip(ui, label, app.browser.tab == tab, 12.0).clicked() {
                    acts.push(Act::Browse(BrowseAct::Tab(tab)));
                }
            }
        });
        ui.add_space(2.0);
        match app.browser.tab {
            Tab::Library => library(app, ui, &mut acts),
            Tab::Files => files(app, ui, &mut acts),
        }
    });
    apply(app, acts);
}

fn library(app: &App, ui: &mut Ui, acts: &mut Vec<Act>) {
    let b = &app.browser;
    let q = &b.query;
    ui.horizontal_wrapped(|ui| {
        for (k, label) in KINDS {
            if chip(ui, label, q.kind == k, 11.0).clicked() {
                acts.push(Act::Browse(BrowseAct::Kind(k)));
            }
        }
    });
    ui.horizontal(|ui| {
        for (user, label) in [(false, "Factory"), (true, "User")] {
            if chip(ui, label, q.user == user, 11.0).clicked() {
                acts.push(Act::Browse(BrowseAct::User(user)));
            }
        }
        ui.add_space(8.0);
        if chip(ui, if q.favorites { "♥ Favorites" } else { "♡ Favorites" }, q.favorites, 11.0).clicked() {
            acts.push(Act::Browse(BrowseAct::Favorites(!q.favorites)));
        }
    });
    if !q.user && !b.results.products.is_empty() {
        let shown = q.product.clone().unwrap_or_else(|| "All libraries".into());
        egui::ComboBox::from_id_salt("browser-product").width(WIDTH - 20.0).selected_text(shown).show_ui(ui, |ui| {
            if ui.selectable_label(q.product.is_none(), "All libraries").clicked() {
                acts.push(Act::Browse(BrowseAct::Product(None)));
            }
            for p in &b.results.products {
                if ui.selectable_label(q.product.as_ref() == Some(p), p).clicked() {
                    acts.push(Act::Browse(BrowseAct::Product(Some(p.clone()))));
                }
            }
        });
    }
    // Types: the chosen path (click a level to go back to it), then the next level.
    label(ui, "TYPES");
    ui.horizontal_wrapped(|ui| {
        if !q.types.is_empty() && chip(ui, "All", false, 11.0).clicked() {
            acts.push(Act::Browse(BrowseAct::Types(Vec::new())));
        }
        for (i, t) in q.types.iter().enumerate() {
            if chip(ui, &format!("{t} ›"), true, 11.0).clicked() {
                acts.push(Act::Browse(BrowseAct::Types(q.types[..i].to_vec())));
            }
        }
        if q.types.len() < 3 {
            for (t, n) in &b.results.types {
                if chip(ui, &format!("{t} {n}"), false, 11.0).clicked() {
                    let mut path = q.types.clone();
                    path.push(t.clone());
                    acts.push(Act::Browse(BrowseAct::Types(path)));
                }
            }
        }
    });
    label(ui, "CHARACTERS");
    ui.horizontal_wrapped(|ui| {
        let mut shown: Vec<(String, Option<usize>)> = b.results.characters.iter().map(|(c, n)| (c.clone(), Some(*n))).collect();
        // A chosen character stays shown even when nothing left has it.
        for c in &q.characters {
            if !shown.iter().any(|(s, _)| s == c) {
                shown.push((c.clone(), None));
            }
        }
        for (c, n) in shown {
            let on = q.characters.contains(&c);
            let text = n.map_or(c.clone(), |n| format!("{c} {n}"));
            if chip(ui, &text, on, 11.0).clicked() {
                acts.push(Act::Browse(BrowseAct::Character(c, !on)));
            }
        }
    });
    let mut text = q.text.clone();
    let edit = ui.add(egui::TextEdit::singleline(&mut text).hint_text("🔍 search").desired_width(WIDTH - 20.0));
    if edit.changed() {
        acts.push(Act::Browse(BrowseAct::Text(text)));
    }
    let n = b.results.items.len();
    let status = if b.waiting() {
        "…".to_string()
    } else if let Some(e) = &b.error {
        format!("NI's library: {e}")
    } else {
        format!("{n} results")
    };
    ui.label(RichText::new(status).size(10.0).color(white(142)));
    results(app, ui, acts);
}

fn results(app: &App, ui: &mut Ui, acts: &mut Vec<Act>) {
    let b = &app.browser;
    let height = (ui.available_height() - 34.0).max(60.0);
    egui::ScrollArea::vertical().max_height(height).auto_shrink([false, false]).show_rows(
        ui,
        RESULT_ROW,
        b.results.items.len(),
        |ui, rows| {
            for i in rows {
                let item = &b.results.items[i];
                let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), RESULT_ROW), Sense::click_and_drag());
                let selected = b.selected == Some(i);
                let fill = if selected {
                    Color32::from_rgb(0x33, 0x40, 0x5A)
                } else if resp.hovered() {
                    S02
                } else if i % 2 == 0 {
                    S01
                } else {
                    S00
                };
                ui.painter().rect_filled(r, 0.0, fill);
                let fav = if item.favorite { "♥ " } else { "" };
                ui.painter().text(
                    r.left_center() + Vec2::new(6.0, 0.0),
                    Align2::LEFT_CENTER,
                    format!("{fav}{}", item.name),
                    font(11.0),
                    white(230),
                );
                let detail = match (item.tempo, &item.product) {
                    (Some(t), _) if matches!(item.kind, Kind::Loop) => format!("{t:.0} BPM"),
                    (_, Some(p)) => p.clone(),
                    _ => String::new(),
                };
                ui.painter().text(r.right_center() - Vec2::new(6.0, 0.0), Align2::RIGHT_CENTER, detail, font(10.0), white(120));
                if resp.double_clicked() {
                    acts.push(Act::Browse(BrowseAct::Load(i, None)));
                } else if resp.clicked() {
                    acts.push(Act::Browse(BrowseAct::Select(i)));
                }
                if resp.drag_started() {
                    resp.dnd_set_drag_payload(Dragged(i));
                }
                resp.context_menu(|ui| {
                    let label = if item.favorite { "Remove from Favorites" } else { "Add to Favorites" };
                    if ui.button(label).clicked() {
                        acts.push(Act::Browse(BrowseAct::Favorite(i, !item.favorite)));
                        ui.close();
                    }
                });
            }
        },
    );
    prehear_row(app, ui, acts);
}

/// The FILES tab: pinned folders, the folder shown (up, pin), what's in it.
fn files(app: &App, ui: &mut Ui, acts: &mut Vec<Act>) {
    let b = &app.browser;
    let pins = &app.settings.favorite_folders;
    if !pins.is_empty() {
        label(ui, "FAVORITE FOLDERS");
        for d in pins {
            let name = d.file_name().map_or_else(|| d.display().to_string(), |n| n.to_string_lossy().into_owned());
            let resp = ui
                .add(egui::Button::new(RichText::new(format!("★ {name}")).size(11.0)).frame(false))
                .on_hover_text(d.display().to_string());
            if resp.clicked() {
                acts.push(Act::Browse(BrowseAct::Folder(d.clone())));
            }
            resp.context_menu(|ui| {
                if ui.button("Remove from Favorites").clicked() {
                    acts.push(Act::Browse(BrowseAct::PinFolder(d.clone(), false)));
                    ui.close();
                }
            });
        }
        ui.separator();
    }
    ui.horizontal(|ui| {
        if ui.button("↑").on_hover_text("The folder above").clicked() {
            acts.push(Act::Browse(BrowseAct::Up));
        }
        let pinned = pins.contains(&b.files_dir);
        if ui.button(if pinned { "★" } else { "☆" }).on_hover_text("Pin this folder").clicked() {
            acts.push(Act::Browse(BrowseAct::PinFolder(b.files_dir.clone(), !pinned)));
        }
        ui.add(egui::Label::new(RichText::new(format!("📁 {}", b.files_dir.display())).size(10.0).color(white(180))).truncate());
    });
    let height = (ui.available_height() - 34.0).max(60.0);
    egui::ScrollArea::vertical().max_height(height).auto_shrink([false, false]).show_rows(ui, RESULT_ROW, b.files.len(), |ui, rows| {
        for i in rows {
            let e = &b.files[i];
            let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), RESULT_ROW), Sense::click_and_drag());
            let selected = b.file_selected.as_ref() == Some(&e.path);
            let fill = if selected {
                Color32::from_rgb(0x33, 0x40, 0x5A)
            } else if resp.hovered() {
                S02
            } else if i % 2 == 0 {
                S01
            } else {
                S00
            };
            ui.painter().rect_filled(r, 0.0, fill);
            let name = e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let text = if e.dir { format!("📁 {name}") } else { name };
            ui.painter().text(r.left_center() + Vec2::new(6.0, 0.0), Align2::LEFT_CENTER, text, font(11.0), white(230));
            if e.dir {
                if resp.clicked() {
                    acts.push(Act::Browse(BrowseAct::Folder(e.path.clone())));
                }
            } else if resp.double_clicked() {
                acts.push(Act::Browse(BrowseAct::LoadFile(e.path.clone(), None)));
            } else if resp.clicked() {
                acts.push(Act::Browse(BrowseAct::SelectFile(e.path.clone())));
            }
            if !e.dir && resp.drag_started() {
                resp.dnd_set_drag_payload(DraggedFile(e.path.clone()));
            }
        }
    });
}

/// ▶, auto-prehear and its level.
pub(super) fn prehear_row(app: &App, ui: &mut Ui, acts: &mut Vec<Act>) {
    let b = &app.browser;
    ui.horizontal(|ui| {
        let can = b.selected_item().is_some_and(|i| crate::browser::can_prehear(i));
        if ui.add_enabled(can, egui::Button::new("▶ Prehear")).clicked()
            && let Some(i) = b.selected
        {
            acts.push(Act::Browse(BrowseAct::Prehear(i)));
        }
        let mut auto = b.prehear_auto;
        if ui.checkbox(&mut auto, "auto").changed() {
            acts.push(Act::Browse(BrowseAct::PrehearAuto(auto)));
        }
        let mut gain = b.prehear_gain;
        if ui.add(egui::Slider::new(&mut gain, 0.0..=1.0).show_value(false)).changed() {
            acts.push(Act::Browse(BrowseAct::Gain(gain)));
        }
    });
}

/// A small rounded toggle with `text`, lit when `on`.
pub(super) fn chip(ui: &mut Ui, text: &str, on: bool, size: f32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), font(size), if on { Color32::BLACK } else { white(200) });
    let (r, resp) = ui.allocate_exact_size(galley.size() + Vec2::new(12.0, 6.0), Sense::click());
    let fill = if on {
        ACCENT
    } else if resp.hovered() {
        S03
    } else {
        S02
    };
    ui.painter().rect_filled(r, 9.0, fill);
    ui.painter().galley(r.center() - galley.size() / 2.0, galley, Color32::WHITE);
    resp
}

fn label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(9.0).color(white(120)));
}
