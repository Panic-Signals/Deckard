//! The application menus, as in Maschine: File, Edit, View, Controller and Help.
//! One description of them is drawn twice: under the MASCHINE wordmark in the header
//! (Maschine's in-window menu) and, on macOS, in the menu bar.

use crate::App;
use eframe::egui::{self, Key, Ui};
use std::path::PathBuf;

/// What a menu entry does.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    New,
    Open,
    OpenRecent(PathBuf),
    ClearRecent,
    Save,
    SaveAs,
    SaveCopyAs,
    Revert,
    Quit,
    /// Undo or redo this many steps.
    Undo(usize),
    Redo(usize),
    /// Undo or redo a single edit, inside a take too.
    UndoStep,
    RedoStep,
    SelectAll,
    Deselect,
    DeleteSelected,
    SongView(bool),
    PianoRoll(bool),
    DrawMode,
    Follow,
    ShowController,
    FullScreen,
    Model(&'static str),
    ReloadScripts,
    /// The folder of the user's own controller scripts and themes.
    ControllerFolder,
    Manual,
    About,
    Preferences,
    Mixer,
    /// Show or hide the waveform on the Sampler pages.
    Waveform,
    /// The Note Repeat / Arp panel.
    RepeatPanel,
    /// The Lock panel.
    LockPanel,
    /// Edit ▸ MIDI Change….
    MidiChange,
    /// View ▸ MIDI Monitor.
    MidiMonitor,
    /// File ▸ Export Audio….
    ExportAudio,
    /// File ▸ Export MIDI… (the song, or the focused scene).
    ExportMidi,
}

/// ⌘ + `key` (+ Shift).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shortcut {
    pub key: Key,
    pub shift: bool,
}

impl Shortcut {
    fn text(self) -> String {
        format!("{}⌘{}", if self.shift { "Shift+" } else { "" }, self.key_char().to_uppercase())
    }

    /// The key as macOS menus want it (lower case).
    fn key_char(self) -> String {
        match self.key {
            Key::Comma => ",".into(),
            k => k.name().to_lowercase(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Command { label: String, cmd: Command, shortcut: Option<Shortcut>, enabled: bool, checked: bool },
    Separator,
    Submenu { label: String, items: Vec<Item> },
}

fn cmd(label: impl Into<String>, cmd: Command) -> Item {
    Item::Command { label: label.into(), cmd, shortcut: None, enabled: true, checked: false }
}

impl Item {
    fn key(mut self, key: Key, shift: bool) -> Item {
        if let Item::Command { shortcut, .. } = &mut self {
            *shortcut = Some(Shortcut { key, shift });
        }
        self
    }

    fn enabled(mut self, on: bool) -> Item {
        if let Item::Command { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }

    fn checked(mut self, on: bool) -> Item {
        if let Item::Command { checked, .. } = &mut self {
            *checked = on;
        }
        self
    }
}

/// Controller models the scripts know (`Scripts/Maschine/Controllers.lua`).
pub const MODELS: [(&str, &str); 9] = [
    ("M3", "MASCHINE MK3"),
    ("MH1071", "MASCHINE+"),
    ("MS1", "MASCHINE STUDIO"),
    ("MJ1", "MASCHINE JAM"),
    ("MM3", "MASCHINE MIKRO MK3"),
    ("M2", "MASCHINE MK2"),
    ("M1", "MASCHINE MK1"),
    ("MM2", "MASCHINE MIKRO MK2"),
    ("MM1", "MASCHINE MIKRO MK1"),
];

/// How many steps the undo and redo history menus list.
const HISTORY: usize = 30;

/// The menus for the app's current state.
pub fn menus(app: &App) -> Vec<(&'static str, Vec<Item>)> {
    let doc = app.doc.as_ref().map(|d| d.borrow());
    let has = doc.is_some();
    let dirty = doc.as_ref().is_some_and(|d| d.is_dirty());
    let undo: Vec<String> = doc.as_ref().map_or(Vec::new(), |d| d.undo_labels().into_iter().map(String::from).collect());
    let redo: Vec<String> = doc.as_ref().map_or(Vec::new(), |d| d.redo_labels().into_iter().map(String::from).collect());
    let notes = doc
        .as_ref()
        .and_then(|d| d.project().groups.get(app.group)?.patterns.get(app.pattern).map(|p| !p.notes.is_empty()))
        .unwrap_or(false);
    drop(doc);

    let mut recent: Vec<Item> = app.recent.iter().map(|p| cmd(recent_label(p), Command::OpenRecent(p.clone()))).collect();
    if recent.is_empty() {
        recent.push(cmd("No Recent Projects", Command::ClearRecent).enabled(false));
    } else {
        recent.push(Item::Separator);
        recent.push(cmd("Clear Menu", Command::ClearRecent));
    }
    let file = vec![
        cmd("New", Command::New).key(Key::N, false),
        cmd("Open…", Command::Open).key(Key::O, false),
        Item::Submenu { label: "Open Recent".into(), items: recent },
        Item::Separator,
        cmd("Save", Command::Save).key(Key::S, false).enabled(dirty),
        cmd("Save As…", Command::SaveAs).key(Key::S, true).enabled(has),
        cmd("Save Copy As…", Command::SaveCopyAs).enabled(has),
        cmd("Revert to Saved", Command::Revert).enabled(dirty),
        Item::Separator,
        cmd("Export Audio…", Command::ExportAudio).enabled(has),
        cmd("Export MIDI…", Command::ExportMidi).enabled(has),
        Item::Separator,
        cmd("Preferences…", Command::Preferences).key(Key::Comma, false),
        Item::Separator,
        cmd("Quit", Command::Quit).key(Key::Q, false),
    ];

    let history = |labels: &[String], make: fn(usize) -> Command| -> Vec<Item> {
        let mut items: Vec<Item> = labels.iter().take(HISTORY).enumerate().map(|(i, l)| cmd(l.clone(), make(i + 1))).collect();
        if labels.len() > HISTORY {
            items.push(cmd(format!("… {} more", labels.len() - HISTORY), make(labels.len())).enabled(false));
        }
        if items.is_empty() {
            items.push(cmd("Empty", make(0)).enabled(false));
        }
        items
    };
    let edit = vec![
        match undo.first() {
            Some(l) => cmd(format!("Undo {l}"), Command::Undo(1)),
            None => cmd("Undo", Command::Undo(1)).enabled(false),
        }
        .key(Key::Z, false),
        match redo.first() {
            Some(l) => cmd(format!("Redo {l}"), Command::Redo(1)),
            None => cmd("Redo", Command::Redo(1)).enabled(false),
        }
        .key(Key::Y, false),
        cmd("Undo Step", Command::UndoStep).key(Key::Z, true).enabled(!undo.is_empty()),
        cmd("Redo Step", Command::RedoStep).key(Key::Y, true).enabled(!redo.is_empty()),
        Item::Submenu { label: "Undo History".into(), items: history(&undo, Command::Undo) },
        Item::Submenu { label: "Redo History".into(), items: history(&redo, Command::Redo) },
        Item::Separator,
        cmd("Select All Notes", Command::SelectAll).enabled(notes),
        cmd("Deselect", Command::Deselect).enabled(!app.selected.is_empty()),
        cmd("Delete Selected Notes", Command::DeleteSelected).enabled(!app.selected.is_empty()),
        Item::Separator,
        cmd("MIDI Change…", Command::MidiChange).enabled(notes || app.doc.is_some()),
    ];

    let view = vec![
        cmd("Ideas View", Command::SongView(false)).checked(!app.song_view),
        cmd("Song View", Command::SongView(true)).checked(app.song_view),
        Item::Separator,
        cmd("Drum Grid", Command::PianoRoll(false)).checked(!app.piano_roll),
        cmd("Piano Roll", Command::PianoRoll(true)).checked(app.piano_roll),
        cmd("Draw Mode", Command::DrawMode).checked(app.draw_mode()),
        Item::Separator,
        cmd("Mixer", Command::Mixer).checked(app.mixer),
        cmd("Sample Waveform", Command::Waveform).checked(app.settings.show_waveform),
        cmd("Note Repeat / Arp", Command::RepeatPanel).checked(app.repeat_panel),
        cmd("Lock", Command::LockPanel).checked(app.lock_panel),
        cmd("MIDI Monitor", Command::MidiMonitor).checked(app.midi_monitor),
        Item::Separator,
        cmd("Follow Playback", Command::Follow).checked(app.follow),
        cmd("Full Screen", Command::FullScreen).key(Key::F, false),
    ];

    let scripts = app.scripts.is_some();
    let mut controller = vec![
        cmd("Show Controller", Command::ShowController).checked(app.show_controller && app.controller.is_some()).enabled(scripts),
        Item::Submenu {
            label: "Controller Model".into(),
            items: MODELS.iter().map(|(m, name)| cmd(*name, Command::Model(m)).checked(app.model == *m).enabled(scripts)).collect(),
        },
        cmd("Reload Scripts", Command::ReloadScripts).enabled(scripts),
    ];
    if !scripts {
        controller.insert(0, cmd("No Maschine scripts: DECKARD's own controller layer runs", Command::ReloadScripts).enabled(false));
        controller.insert(1, Item::Separator);
    }
    controller.push(Item::Separator);
    controller.push(cmd("Open Controller Scripts Folder", Command::ControllerFolder));

    let help = vec![cmd("MASCHINE Software Manual", Command::Manual), cmd("About DECKARD", Command::About)];
    vec![("File", file), ("Edit", edit), ("View", view), ("Controller", controller), ("Help", help)]
}

fn recent_label(p: &std::path::Path) -> String {
    let name = p.file_stem().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
    match p.parent().and_then(|d| d.file_name()) {
        Some(dir) => format!("{name}  ({})", dir.to_string_lossy()),
        None => name,
    }
}

/// Draw `items` into a menu.
pub fn show(ui: &mut Ui, items: &[Item], out: &mut Vec<Command>) {
    for it in items {
        match it {
            Item::Separator => {
                ui.separator();
            }
            Item::Submenu { label, items } => {
                ui.menu_button(label, |ui| show(ui, items, out));
            }
            Item::Command { label, cmd, shortcut, enabled, checked } => {
                // Checked entries are highlighted, like the app's other menus.
                let mut b = egui::Button::new(label).selected(*checked);
                if let Some(s) = shortcut {
                    b = b.shortcut_text(s.text());
                }
                if ui.add_enabled(*enabled, b).clicked() {
                    out.push(cmd.clone());
                    ui.close();
                }
            }
        }
    }
}

/// Commands whose shortcuts were pressed.
pub fn shortcuts(ctx: &egui::Context, menus: &[(&'static str, Vec<Item>)]) -> Vec<Command> {
    fn walk(ctx: &egui::Context, items: &[Item], out: &mut Vec<Command>) {
        for it in items {
            match it {
                Item::Submenu { items, .. } => walk(ctx, items, out),
                Item::Command { cmd, shortcut: Some(s), enabled, .. } => {
                    let mods = if s.shift { egui::Modifiers::COMMAND | egui::Modifiers::SHIFT } else { egui::Modifiers::COMMAND };
                    if ctx.input_mut(|i| i.consume_key(mods, s.key)) && *enabled {
                        out.push(cmd.clone());
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    // Shift variants first, so ⇧⌘Z isn't taken as ⌘Z.
    let mut all: Vec<Item> = menus.iter().flat_map(|(_, items)| items.clone()).collect();
    all.sort_by_key(|it| !matches!(it, Item::Command { shortcut: Some(Shortcut { shift: true, .. }), .. }));
    walk(ctx, &all, &mut out);
    out
}

/// Projects opened recently, most recent first (kept in the user's settings folder).
pub fn load_recent() -> Vec<PathBuf> {
    let Some(f) = recent_file() else { return Vec::new() };
    std::fs::read_to_string(f).unwrap_or_default().lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect()
}

pub fn save_recent(list: &[PathBuf]) {
    let Some(f) = recent_file() else { return };
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text: String = list.iter().map(|p| format!("{}\n", p.display())).collect();
    let _ = std::fs::write(f, text);
}

/// `$DECKARD_SETTINGS/recent.txt`, else in Application Support (macOS) or
/// `~/.config`.
fn recent_file() -> Option<PathBuf> {
    Some(settings_dir()?.join("recent.txt"))
}

/// The folder for the app's settings: `$DECKARD_SETTINGS`, else Application Support
/// (macOS), `%APPDATA%` (Windows) or `$XDG_CONFIG_HOME` / `~/.config` (Linux and the
/// rest). Tests get a folder of their own, never the user's.
pub fn settings_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("DECKARD_SETTINGS") {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = test_settings_dir() {
        return Some(dir);
    }
    let base = if cfg!(target_os = "macos") {
        std::env::home_dir()?.join("Library/Application Support")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| Some(std::env::home_dir()?.join(".config")))?
    };
    let dir = base.join("deckard");
    // Settings kept under the project's earlier name move over once.
    let old = base.join("maschine-rs");
    if !dir.exists() && old.is_dir() {
        let _ = std::fs::rename(&old, &dir);
    }
    Some(dir)
}

/// The tests' settings folder: one per test run, shared by its tests (and the threads
/// they start), removed as the process exits.
#[cfg(test)]
fn test_settings_dir() -> Option<PathBuf> {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    unsafe extern "C" {
        fn atexit(f: extern "C" fn()) -> std::ffi::c_int;
    }
    extern "C" fn remove() {
        if let Some(dir) = DIR.get() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    let dir = DIR.get_or_init(|| {
        let dir = tempfile::Builder::new().prefix("deckard-test-settings-").tempdir().unwrap().keep();
        // SAFETY: a plain function, run once as the process exits.
        unsafe { atexit(remove) };
        dir
    });
    Some(dir.clone())
}

#[cfg(not(test))]
fn test_settings_dir() -> Option<PathBuf> {
    None
}

/// The menus in the macOS menu bar. Clicked entries are queued for [`take`].
#[cfg(target_os = "macos")]
pub mod native {
    use super::{Command, Item};
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::{NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSMenu, NSMenuItem};
    use objc2_foundation::NSString;
    use std::cell::RefCell;

    #[derive(Default)]
    struct State {
        built: String,
        commands: Vec<Command>,
        clicked: Vec<usize>,
        target: Option<Retained<Target>>,
        ctx: Option<eframe::egui::Context>,
        /// Whether macOS may quit the app outright (nothing unsaved, or quitting already).
        may_terminate: bool,
        /// macOS asked to quit (the Dock, the app switcher) while it may not: Quit, which
        /// asks about unsaved changes first.
        quit_asked: bool,
        terminate_hooked: bool,
    }

    /// `applicationShouldTerminate:` for winit's application delegate, which has none: a
    /// quit from outside the app (the Dock's Quit, logging out) waits for the question
    /// about unsaved changes, as closing the window does.
    extern "C-unwind" fn should_terminate(
        _this: *mut objc2::runtime::AnyObject,
        _cmd: objc2::runtime::Sel,
        _app: *mut objc2::runtime::AnyObject,
    ) -> usize {
        const NS_TERMINATE_CANCEL: usize = 0;
        const NS_TERMINATE_NOW: usize = 1;
        let ctx = STATE.with(|s| {
            let mut s = s.borrow_mut();
            if s.may_terminate {
                return None;
            }
            s.quit_asked = true;
            s.ctx.clone()
        });
        match ctx {
            Some(ctx) => {
                ctx.request_repaint();
                NS_TERMINATE_CANCEL
            }
            None => NS_TERMINATE_NOW,
        }
    }

    /// Say whether macOS may quit the app without asking (see [`should_terminate`]).
    pub fn set_may_terminate(may: bool) {
        STATE.with(|s| s.borrow_mut().may_terminate = may);
    }

    fn hook_terminate(mtm: MainThreadMarker) {
        if STATE.with(|s| s.borrow().terminate_hooked) {
            return;
        }
        let Some(delegate) = NSApplication::sharedApplication(mtm).delegate() else { return };
        let object: &objc2::runtime::AnyObject = (*delegate).as_ref();
        let class = object.class();
        let imp: objc2::runtime::Imp = unsafe {
            std::mem::transmute::<
                extern "C-unwind" fn(*mut objc2::runtime::AnyObject, objc2::runtime::Sel, *mut objc2::runtime::AnyObject) -> usize,
                objc2::runtime::Imp,
            >(should_terminate)
        };
        // Returns NSApplicationTerminateReply (an NSUInteger) and takes the app.
        let types = c"Q@:@";
        unsafe {
            objc2::ffi::class_addMethod(class as *const _ as *mut _, sel!(applicationShouldTerminate:), imp, types.as_ptr());
        }
        STATE.with(|s| s.borrow_mut().terminate_hooked = true);
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State::default());
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DeckardMenuTarget"]
        struct Target;

        unsafe impl NSObjectProtocol for Target {}

        impl Target {
            #[unsafe(method(menuItemClicked:))]
            fn clicked(&self, item: &NSMenuItem) {
                let ctx = STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    s.clicked.push(item.tag() as usize);
                    s.ctx.clone()
                });
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            }
        }
    );

    /// Put `menus` in the menu bar (rebuilt only when they change).
    pub fn update(ctx: &eframe::egui::Context, menus: &[(&'static str, Vec<Item>)]) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        hook_terminate(mtm);
        let sig = format!("{menus:?}");
        if STATE.with(|s| s.borrow().built == sig) {
            return;
        }
        let target = STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.ctx = Some(ctx.clone());
            s.commands.clear();
            s.target.get_or_insert_with(|| unsafe { msg_send![Target::alloc(mtm), init] }).clone()
        });
        let bar = NSMenu::new(mtm);
        // The application menu (macOS titles it with the app's name).
        let app = NSMenu::new(mtm);
        add(mtm, &app, &target, &super::cmd("About DECKARD", Command::About));
        app.addItem(&NSMenuItem::separatorItem(mtm));
        add(mtm, &app, &target, &super::cmd("Preferences…", Command::Preferences).key(eframe::egui::Key::Comma, false));
        app.addItem(&NSMenuItem::separatorItem(mtm));
        let hide = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &NSString::from_str("Hide"),
                Some(sel!(hide:)),
                &NSString::from_str("h"),
            )
        };
        app.addItem(&hide);
        app.addItem(&NSMenuItem::separatorItem(mtm));
        add(mtm, &app, &target, &super::cmd("Quit", Command::Quit).key(eframe::egui::Key::Q, false));
        submenu(mtm, &bar, "DECKARD", app);
        for (title, items) in menus {
            let m = NSMenu::new(mtm);
            m.setAutoenablesItems(false);
            for it in items {
                // Quit and Preferences live in the application menu on macOS.
                if matches!(it, Item::Command { cmd: Command::Quit | Command::Preferences, .. }) {
                    continue;
                }
                add(mtm, &m, &target, it);
            }
            submenu(mtm, &bar, title, m);
        }
        NSApplication::sharedApplication(mtm).setMainMenu(Some(&bar));
        STATE.with(|s| s.borrow_mut().built = sig);
        if std::env::var_os("DECKARD_MENU_DUMP").is_some() {
            dump(&NSApplication::sharedApplication(mtm).mainMenu().unwrap(), 0);
        }
    }

    /// Print the menu bar as installed (`DECKARD_MENU_DUMP`), to check it.
    fn dump(menu: &NSMenu, depth: usize) {
        for item in menu.itemArray().iter() {
            let key = item.keyEquivalent().to_string();
            let state = if item.state() == NSControlStateValueOn { " [on]" } else { "" };
            let off = if item.isEnabled() { "" } else { " (disabled)" };
            eprintln!(
                "{}{}{}{}{}",
                "  ".repeat(depth),
                item.title(),
                if key.is_empty() { String::new() } else { format!("  ⌘{key}") },
                state,
                off
            );
            if let Some(sub) = item.submenu() {
                dump(&sub, depth + 1);
            }
        }
    }

    fn submenu(mtm: MainThreadMarker, bar: &NSMenu, title: &str, menu: Retained<NSMenu>) {
        let item = NSMenuItem::new(mtm);
        item.setTitle(&NSString::from_str(title));
        menu.setTitle(&NSString::from_str(title));
        item.setSubmenu(Some(&menu));
        bar.addItem(&item);
    }

    fn add(mtm: MainThreadMarker, menu: &NSMenu, target: &Target, it: &Item) {
        match it {
            Item::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
            Item::Submenu { label, items } => {
                let m = NSMenu::new(mtm);
                m.setAutoenablesItems(false);
                for it in items {
                    add(mtm, &m, target, it);
                }
                submenu(mtm, menu, label, m);
            }
            Item::Command { label, cmd, shortcut, enabled, checked } => {
                let key = shortcut.map_or(String::new(), |s| s.key_char());
                let item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        mtm.alloc(),
                        &NSString::from_str(label),
                        Some(sel!(menuItemClicked:)),
                        &NSString::from_str(&key),
                    )
                };
                if let Some(s) = shortcut {
                    let mut mask = NSEventModifierFlags::Command;
                    if s.shift {
                        mask |= NSEventModifierFlags::Shift;
                    }
                    item.setKeyEquivalentModifierMask(mask);
                }
                unsafe { item.setTarget(Some(target)) };
                item.setEnabled(*enabled);
                item.setState(if *checked { NSControlStateValueOn } else { NSControlStateValueOff });
                let tag = STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    s.commands.push(cmd.clone());
                    s.commands.len() - 1
                });
                item.setTag(tag as isize);
                menu.addItem(&item);
            }
        }
    }

    /// Commands chosen in the menu bar since the last call.
    pub fn take() -> Vec<Command> {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            let clicked = std::mem::take(&mut s.clicked);
            let mut cmds: Vec<Command> = clicked.into_iter().filter_map(|i| s.commands.get(i).cloned()).collect();
            if std::mem::take(&mut s.quit_asked) {
                cmds.push(Command::Quit);
            }
            cmds
        })
    }
}
