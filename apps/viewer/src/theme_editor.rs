//! Preferences ▸ Hardware: edit the controller's colours, save them as a theme, and
//! pass that file to someone else. A theme is a Lua file in the controller folder's
//! `themes/`. Import reads one, export writes one. There is no account and no gallery.

use deckard_surface::theme_file::{self, Look};
use eframe::egui;

/// Which side the colour rows are editing. The two do not share colours.
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Hardware,
    Desktop,
}

/// Kept in egui's temp data so it survives the next frame.
#[derive(Clone)]
struct Editor {
    side: Side,
    hardware: Draft,
    /// `custom`, or `theme:lcd-mode`, so picking another desktop theme resamples.
    desktop_source: String,
    desktop_background: [u8; 3],
    desktop_text: [u8; 3],
}

/// What the hardware colour rows are editing.
#[derive(Clone)]
struct Draft {
    /// The theme these colours were sampled from.
    theme: String,
    title: String,
    look: Look,
    /// A colour changed and has not been saved.
    dirty: bool,
}

pub(crate) fn show(ui: &mut egui::Ui, app: &mut super::App) {
    let id = egui::Id::new("deckard-theme-draft");
    let mut editor = ui.data(|d| d.get_temp::<Editor>(id)).unwrap_or_else(|| Editor::sample(app));
    if editor.hardware.theme != app.settings.controller_theme {
        editor.hardware = sample(app);
    }
    let source = desktop_source(app);
    if editor.desktop_source != source {
        let (background, text) = sample_desktop(app);
        editor.desktop_background = background;
        editor.desktop_text = text;
        editor.desktop_source = source;
    }

    ui.label("Edit");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut editor.side, Side::Desktop, "Desktop");
        ui.selectable_value(&mut editor.side, Side::Hardware, "Hardware");
    });
    ui.end_row();

    match editor.side {
        Side::Desktop => desktop_rows(ui, app, &mut editor),
        Side::Hardware => hardware_rows(ui, app, &mut editor.hardware),
    }

    ui.data_mut(|d| d.insert_temp(id, editor));
}

fn desktop_rows(ui: &mut egui::Ui, app: &mut super::App, editor: &mut Editor) {
    let mut changed = false;
    changed |= swatch(ui, "Background", "The window behind everything. It does not change the Maschine.", &mut editor.desktop_background);
    changed |= swatch(ui, "Text", "The window's writing. It does not change the Maschine.", &mut editor.desktop_text);
    if changed {
        app.settings.desktop_background = Some(editor.desktop_background);
        app.settings.desktop_text = Some(editor.desktop_text);
        editor.desktop_source = "custom".into();
        super::layout::sync_desktop(ui.ctx(), app);
    }
    ui.label("");
    if ui.button("Reset").on_hover_text("Back to the desktop theme chosen above.").clicked() {
        app.settings.desktop_background = None;
        app.settings.desktop_text = None;
        let (background, text) = sample_desktop(app);
        editor.desktop_background = background;
        editor.desktop_text = text;
        editor.desktop_source = desktop_source(app);
        super::layout::sync_desktop(ui.ctx(), app);
    }
    ui.end_row();
}

fn hardware_rows(ui: &mut egui::Ui, app: &mut super::App, draft: &mut Draft) {
    ui.label("Theme name").on_hover_text(
        "Save writes a theme from these colours. Export saves that file so you can send it on; Import loads one you were sent. Removing your copy brings the built-in theme back.",
    );
    ui.text_edit_singleline(&mut draft.title);
    ui.end_row();

    let mut changed = false;
    changed |= swatch(ui, "Background", "The screens behind everything.", &mut draft.look.background);
    changed |= swatch(ui, "Text", "Names, values and the mixer. Plug-in pictures keep their own colour.", &mut draft.look.text);
    changed |= swatch(ui, "Muted", "Labels, ticks and anything secondary.", &mut draft.look.weak);
    changed |= swatch(ui, "Accent", "Knobs, markers, the browse highlight, and the orange on a plug-in picture.", &mut draft.look.accent);
    changed |= swatch(
        ui,
        "Handles",
        "The dots on the EQ, filter and effect plots, and the dot beside a knob while that parameter is modulated.",
        &mut draft.look.record,
    );
    changed |= swatch(ui, "Text on colour", "Text drawn on a group or pad colour.", &mut draft.look.on_colour);
    changed |= swatch(ui, "Button", "Buttons, tabs and the lines on a plot. Not the plot's background.", &mut draft.look.button);
    changed |= swatch(ui, "Button text", "", &mut draft.look.button_text);
    changed |= swatch(ui, "Selected", "The chosen button or tab.", &mut draft.look.selected);
    changed |= swatch(ui, "Selected text", "", &mut draft.look.selected_text);
    if changed {
        draft.dirty = true;
        if !app.surface.apply_look(draft.look) {
            app.status = "Couldn't recolour the screens.".into();
        }
    }

    ui.label("");
    ui.horizontal(|ui| {
        if ui.button("Save theme").clicked() && save(app, draft) {
            draft.dirty = false;
            draft.theme = app.settings.controller_theme.clone();
        }
        if ui.button("Import…").clicked() {
            import(app);
        }
        if ui.button("Export…").clicked() {
            export(app, draft);
        }
        if ui.add_enabled(user_file(&draft.theme).is_some(), egui::Button::new("Remove my copy")).clicked() && remove(app, &draft.theme) {
            *draft = sample(app);
        }
    });
    ui.end_row();
}

impl Editor {
    fn sample(app: &super::App) -> Editor {
        let (desktop_background, desktop_text) = sample_desktop(app);
        Editor { side: Side::Hardware, hardware: sample(app), desktop_source: desktop_source(app), desktop_background, desktop_text }
    }
}

fn desktop_source(app: &super::App) -> String {
    if app.settings.desktop_background.is_some() && app.settings.desktop_text.is_some() {
        return "custom".into();
    }
    format!("{}:{}", app.settings.desktop_theme, app.settings.lcd_mode)
}

fn sample_desktop(app: &super::App) -> ([u8; 3], [u8; 3]) {
    match (app.settings.desktop_background, app.settings.desktop_text) {
        (Some(background), Some(text)) => (background, text),
        _ => super::layout::window_colours(&app.settings.desktop_theme, &app.settings.lcd_mode),
    }
}

fn swatch(ui: &mut egui::Ui, label: &str, tip: &str, rgb: &mut [u8; 3]) -> bool {
    let row = ui.label(label);
    if !tip.is_empty() {
        row.on_hover_text(tip);
    }
    let changed = ui.color_edit_button_srgb(rgb).changed();
    ui.end_row();
    changed
}

fn sample(app: &super::App) -> Draft {
    let theme = app.settings.controller_theme.clone();
    let rgb = |name: &str| app.surface.rgb(name);
    Draft {
        theme: theme.clone(),
        title: super::theme_name(&theme),
        look: Look {
            background: rgb("background").unwrap_or([0, 0, 0]),
            text: rgb("text").unwrap_or([255, 255, 255]),
            weak: rgb("weak").or_else(|| rgb("muted")).unwrap_or([128, 128, 128]),
            accent: rgb("accent").unwrap_or([255, 165, 42]),
            record: rgb("record").unwrap_or([224, 60, 60]),
            on_colour: rgb("on_colour").unwrap_or([0, 0, 0]),
            button: rgb("button").unwrap_or([71, 71, 71]),
            button_text: rgb("button_text").unwrap_or([255, 255, 255]),
            selected: rgb("button_selected").unwrap_or([178, 178, 178]),
            selected_text: rgb("button_selected_text").unwrap_or([0, 0, 0]),
        },
        dirty: false,
    }
}

fn save(app: &mut super::App, draft: &Draft) -> bool {
    let title = one_line(&draft.title);
    let Some(id) = theme_file::slug(&title) else {
        app.status = "Give the theme a name.".into();
        return false;
    };
    if !install(app, &id, &theme_file::lua(&title, &draft.look)) {
        return false;
    }
    app.status = format!("Saved theme {title}. Export it to send it to someone.");
    true
}

fn import(app: &mut super::App) {
    let Some(path) = rfd::FileDialog::new().add_filter("DECKARD theme", &["lua"]).pick_file() else { return };
    let Ok(text) = std::fs::read_to_string(&path) else {
        app.status = "Couldn't read that file.".into();
        return;
    };
    if !theme_text_ok(&text) {
        app.status = "That file is not a theme.".into();
        return;
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let Some(id) = theme_file::slug(stem) else {
        app.status = "Rename the file to letters and numbers, then import it again.".into();
        return;
    };
    if install(app, &id, &text) {
        app.status = format!("Imported {}.", super::theme_name(&id));
    }
}

fn export(app: &mut super::App, draft: &Draft) {
    let (id, text) = export_text(draft);
    let Some(path) = rfd::FileDialog::new().add_filter("DECKARD theme", &["lua"]).set_file_name(format!("{id}.lua")).save_file() else {
        return;
    };
    match std::fs::write(&path, text) {
        Ok(()) => app.status = format!("Exported {id}.lua."),
        Err(e) => app.status = format!("Couldn't export: {e}"),
    }
}

fn remove(app: &mut super::App, id: &str) -> bool {
    let Some(path) = user_file(id) else { return false };
    if let Err(e) = std::fs::remove_file(&path) {
        app.status = format!("Couldn't remove it: {e}");
        return false;
    }
    if app.surface.themes().iter().any(|t| t == id) {
        if !app.surface.reload_theme() {
            app.status = app.surface.error().unwrap_or_else(|| "Couldn't restore that theme.".into());
            return false;
        }
    } else {
        app.surface.set_lcd(&app.settings.lcd_mode);
        app.surface.set_theme("maschine");
        app.settings.controller_theme = app.surface.theme().to_string();
        app.settings.save();
    }
    app.status = "Removed your copy. The built-in theme is back.".into();
    true
}

/// Write `text` as the user's `themes/<id>.lua` and show it. If it does not load, the
/// previous file (or none) is put back, so a bad import cannot replace a working theme.
fn install(app: &mut super::App, id: &str, text: &str) -> bool {
    let Some(dir) = themes_dir() else {
        app.status = "Couldn't find the controller folder.".into();
        return false;
    };
    // `maschine` is the base every saved theme builds on, and `lcd` is the special
    // paper-and-ink theme. A file under either name would replace that, or loop.
    if id == "maschine" || id == "lcd" {
        app.status = "Maschine and LCD keep their own names. Use a different one.".into();
        return false;
    }
    let path = dir.join(format!("{id}.lua"));
    let previous = std::fs::read(&path).ok();
    if let Err(e) = std::fs::write(&path, text) {
        app.status = format!("Couldn't write the theme: {e}");
        return false;
    }
    let showing = app.surface.theme() == id;
    let loaded = if showing {
        app.surface.reload_theme()
    } else {
        app.surface.set_lcd(&app.settings.lcd_mode);
        app.surface.set_theme(id);
        app.surface.theme() == id
    };
    if !loaded {
        let message = app.surface.error().unwrap_or_else(|| "That file is not a theme.".into());
        match &previous {
            Some(bytes) => {
                let _ = std::fs::write(&path, bytes);
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
        if showing {
            let _ = app.surface.reload_theme();
        }
        app.status = message;
        return false;
    }
    app.settings.controller_theme = id.to_string();
    app.settings.save();
    true
}

fn themes_dir() -> Option<std::path::PathBuf> {
    let dir = super::menu::settings_dir()?.join("controller").join("themes");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn user_file(id: &str) -> Option<std::path::PathBuf> {
    let path = super::menu::settings_dir()?.join("controller").join("themes").join(format!("{id}.lua"));
    path.is_file().then_some(path)
}

/// The theme as it is stored: the user's file, else a stock palette, else a
/// hand-written theme shipped with DECKARD.
fn theme_text(id: &str) -> Option<String> {
    if let Some(path) = user_file(id)
        && let Ok(text) = std::fs::read_to_string(path)
    {
        return Some(text);
    }
    if let Some(text) = theme_file::source(id) {
        return Some(text);
    }
    let path = format!("themes/{id}.lua");
    deckard_surface::SHIPPED.iter().find(|(p, _)| *p == path).map(|(_, text)| (*text).to_string())
}

fn one_line(title: &str) -> String {
    let title = title.replace(['\n', '\r'], " ");
    let title = title.trim();
    if title.is_empty() { "Theme".into() } else { title.to_string() }
}

/// An import has to be a theme file, not an arbitrary script: small, text, and a
/// theme table.
fn theme_text_ok(text: &str) -> bool {
    text.len() <= 64 * 1024 && !text.contains('\0') && text.contains("return")
}

/// The file name and text Export would write. A colour that has not been saved yet
/// is written from the rows; otherwise the stored theme file is written as it is.
fn export_text(draft: &Draft) -> (String, String) {
    let title = one_line(&draft.title);
    let id = theme_file::slug(&title).unwrap_or_else(|| draft.theme.clone());
    let text = if draft.dirty {
        theme_file::lua(&title, &draft.look)
    } else {
        theme_text(&draft.theme).unwrap_or_else(|| theme_file::lua(&title, &draft.look))
    };
    (id, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nord() -> Look {
        theme_file::stock().iter().find(|t| t.id == "nord").unwrap().look
    }

    fn app() -> crate::App {
        let mut app = crate::App::default();
        let surface = app.new_surface();
        app.surface = surface;
        app
    }

    #[test]
    fn a_name_is_one_line() {
        assert_eq!(one_line("  Nord \n"), "Nord");
        assert_eq!(one_line("\r\n"), "Theme");
        assert!(!theme_text_ok("not a theme"));
        assert!(!theme_text_ok(&format!("return {{}}\0")));
        assert!(theme_text_ok("return {}"));
    }

    #[test]
    fn saving_a_theme_shows_it_and_a_bad_file_does_not_replace_it() {
        let mut app = app();
        let draft = Draft { theme: "maschine".into(), title: "Cove".into(), look: nord(), dirty: true };
        assert!(save(&mut app, &draft), "{}", app.status);
        assert_eq!(app.settings.controller_theme, "cove");
        let stored = theme_text("cove").unwrap();
        assert!(stored.contains("base = \"maschine\""));
        assert!(user_file("cove").is_some());

        assert!(!install(&mut app, "maschine", &stored));
        assert!(app.status.contains("own names"));
        assert!(!install(&mut app, "lcd", &stored));

        assert!(!install(&mut app, "cove", "return {"));
        assert!(theme_text("cove").unwrap().contains("base = \"maschine\""));

        assert!(!install(&mut app, "not_a_theme", "hello"));
        assert!(user_file("not_a_theme").is_none());

        let (id, text) = export_text(&Draft { theme: "nord".into(), title: "Nord".into(), look: nord(), dirty: false });
        assert_eq!(id, "nord");
        assert!(text.contains("local background"));
        let (id, text) = export_text(&draft);
        assert_eq!(id, "cove");
        assert!(text.contains("local background"));

        let sampled = sample(&app);
        assert_eq!(sampled.theme, "cove");
        app.settings.desktop_background = Some([1, 2, 3]);
        app.settings.desktop_text = Some([4, 5, 6]);
        assert_eq!(desktop_source(&app), "custom");
        assert_eq!(sample_desktop(&app), ([1, 2, 3], [4, 5, 6]));
        let editor = Editor::sample(&app);
        assert_eq!(editor.desktop_background, [1, 2, 3]);

        assert!(remove(&mut app, "cove"), "{}", app.status);
        assert!(user_file("cove").is_none());
        assert_ne!(app.settings.controller_theme, "cove");
    }
}
