//! The window driven with simulated mouse input (egui_kittest).
use super::*;
use deckard_model::Dim;
use deckard_model::Note;
use egui::{Event, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

mod audio_takes;
mod controller_pages;
mod dialogs_and_menus;
mod ideas_and_panels;
mod layers_ui;
mod menus_and_project;
mod pattern_editor;
mod rows_and_keys;
mod sampling_editor;
mod window_and_transport;

const PROJECT: &str = "/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj";

/// How long a test on the real audio output waits for what it's waiting for. It goes
/// on as soon as that happens, so this only has to outlast a busy machine, where the
/// song can take seconds to get going.
const AUDIO_PATIENCE: std::time::Duration = std::time::Duration::from_secs(60);

/// Wait for what the audio thread publishes to catch up with what was sent to it:
/// until then, the playhead is still the last engine's.
fn published(app: &App) {
    let wait = std::time::Instant::now();
    while !matches!(&app.output, Some(Ok(out)) if out.is_current()) && wait.elapsed() < AUDIO_PATIENCE {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

pub(super) fn harness(scripts: bool, song_view: bool) -> Option<Harness<'static, App>> {
    let path = std::env::var("DECKARD_TEST_PROJECT").unwrap_or_else(|_| PROJECT.into());
    if !Path::new(&path).exists() {
        eprintln!("{path} not found; skipping");
        return None;
    }
    let scripts = scripts.then(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/ni-res")).filter(|p| p.exists());
    let mut app =
        App { scripts, model: "M3".into(), show_controller: true, follow: true, grid: PPQ / 4, prop_set: 1, ..Default::default() };
    // (Maschine's scripts when asked for: a new install runs DECKARD's own layer.)
    if app.scripts.is_some() {
        app.settings.controller = settings::ControllerChoice::Automatic;
    }
    app.open(Path::new(&path));
    Some(harness_of(app, song_view))
}

/// [`harness`] on a new project (one group of 16 empty sounds, one scene), so the
/// test runs anywhere, CI included: no library project needed.
pub(super) fn blank_harness(song_view: bool) -> Harness<'static, App> {
    let mut app = App {
        settings: settings::Settings::default(),
        model: "M3".into(),
        show_controller: true,
        follow: true,
        grid: PPQ / 4,
        prop_set: 1,
        ..Default::default()
    };
    app.new_project();
    harness_of(app, song_view)
}

fn harness_of(mut app: App, song_view: bool) -> Harness<'static, App> {
    app.song_view = song_view;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0 + layout::CONTROLLER_WIDTH, 930.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
    h.run_steps(3);
    h
}

/// [`harness`] for a test of Maschine's own controller scripts: `None` (skipped) without
/// the extract from the user's install in `out/ni-res`, which is never committed.
pub(super) fn scripted_harness(song_view: bool) -> Option<Harness<'static, App>> {
    let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/ni-res");
    if !scripts.exists() {
        eprintln!("{} not found; skipping", scripts.display());
        return None;
    }
    harness(true, song_view)
}

fn click(h: &mut Harness<'static, App>, pos: Pos2) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.run_steps(3);
}

/// Centre of scene tab `s` in the Ideas view (header 33, panel margin 3, strip 32 + 1).
fn scene_tab(s: usize) -> Pos2 {
    Pos2::new(3.0 + 33.0 + s as f32 * 99.0 + 50.0, 33.0 + 3.0 + 16.0)
}

const WIDTH: f32 = 1500.0 + layout::CONTROLLER_WIDTH;

/// Centre of section `i`'s header in the Song view (same layout maths as the view).
fn section_header(h: &Harness<'static, App>, i: usize) -> Pos2 {
    let d = h.state().doc.as_ref().unwrap().borrow();
    let p = d.project();
    let left = 3.0 + 33.0 + 163.0 + 1.0;
    let right = if h.state().controller.is_some() { WIDTH - layout::CONTROLLER_WIDTH } else { WIDTH };
    let width = right - 3.0 - left;
    let bar = p.bar_length() as f32;
    let total = p.arrangement_length() as f32 / bar;
    let ppb = ((width - 40.0) / total.max(16.0)).max(12.0);
    let x0 = left + p.section_start(i) as f32 / bar * ppb;
    let x1 = left + (p.section_start(i) + p.sections[i].length) as f32 / bar * ppb;
    Pos2::new((x0 + x1) / 2.0, 33.0 + 3.0 + 16.0)
}

fn play(h: &mut Harness<'static, App>) -> bool {
    h.state_mut().transport_play();
    h.state_mut().sync_transport();
    h.run_steps(2);
    matches!(h.state().output, Some(Ok(_)))
}

/// Click the widget labelled `label`: a button if there is one (the last, when a
/// screen button shares the hardware button's name), else any widget.
fn press(h: &mut Harness<'static, App>, label: &str) {
    let buttons: Vec<_> = h.query_all_by_role_and_label(egui::accesskit::Role::Button, label).collect();
    match buttons.last() {
        Some(b) => b.click(),
        None => h.get_by_label(label).click(),
    }
    h.run_steps(3);
}

/// SAMPLE on the controller makes the focused sound a lone Sampler and shows the
/// Sampling view; leaving the page hides it.
/// The controller's EDIT, SLICE and ZONE screens show the focused zone's sample and
/// work it: EDIT's knob 1 moves the play range's start, the tabs follow each other,
/// SLICE shows the app's slices.
/// AUTO LOOP loops a steady tone inside the zone's play range and switches the loop
/// on, as one undo step.
/// A file dropped from the desktop.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

fn right_click(h: &mut Harness<'static, App>, pos: Pos2) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.run_steps(3);
}

fn project_of(h: &Harness<'static, App>) -> deckard_model::Project {
    h.state().doc.as_ref().unwrap().borrow().project().clone()
}

/// Centre of pattern cell (group, pattern) in the Ideas view.
fn cell(g: usize, pi: usize) -> Pos2 {
    Pos2::new(3.0 + 33.0 + g as f32 * 99.0 + 49.0, 33.0 + 3.0 + 32.0 + 4.0 + 4.0 + pi as f32 * 24.0 + 12.0)
}

fn drag(h: &mut Harness<'static, App>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.step();
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    for k in 1..=6 {
        let t = k as f32 / 6.0;
        h.event(Event::PointerMoved(from + (to - from) * t));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.step();
    h.run_steps(3);
}

/// Where the pattern editor puts tick `t` (from the pattern start) of row `row`,
/// with the panel sizes of a 930-high window and no controller panel.
fn grid_pos(h: &Harness<'static, App>, row: usize, t: i64) -> Pos2 {
    let p = project_of(h);
    let (g, pi) = (h.state().group, h.state().pattern);
    let start = p.groups[g].patterns.get(pi).map_or(0, |pt| pt.start);
    let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
    let (from, scale): (f64, f32) = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid-time"))).expect("time axis drawn");
    Pos2::new(grid.left() + ((start + t) as f64 - from) as f32 * scale, grid.top() + row as f32 * 24.0 + 12.0)
}

fn expression_of(h: &Harness<'static, App>, sound: usize) -> Option<deckard_model::Expression> {
    let n = notes_of(h).into_iter().find(|n| n.sound == sound)?;
    h.state().doc.as_ref().unwrap().borrow().expression(n.expr?).cloned()
}

/// The lane under the note area: (inner bottom, inner height).
fn lane(h: &Harness<'static, App>) -> (f32, f32) {
    let lane: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-lane"))).expect("lane drawn");
    (lane.bottom() - 7.0, lane.height() - 14.0)
}

/// The rows of the lane list (the last is "+").
fn lane_list(h: &Harness<'static, App>) -> Vec<(layout::Lane, egui::Rect)> {
    h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-lane-list"))).expect("lane list drawn")
}

/// Show `lane` by clicking it in the list.
fn pick_lane(h: &mut Harness<'static, App>, lane: layout::Lane) {
    let r = lane_list(h).into_iter().find(|(l, _)| *l == lane).expect("lane listed").1;
    click(h, r.center());
    assert_eq!(h.state().lane, lane);
}

fn notes_of(h: &Harness<'static, App>) -> Vec<Note> {
    let (g, pi) = (h.state().group, h.state().pattern);
    project_of(h).groups[g].patterns[pi].notes.clone()
}

/// A pattern in group 0, on in the scene, if it has none (a new project has none).
fn a_pattern(h: &mut Harness<'static, App>) {
    let doc = h.state().doc.clone().unwrap();
    let mut d = doc.borrow_mut();
    if d.project().groups[0].patterns.is_empty() {
        let pi = d.add_pattern(0, usize::MAX, "Pattern 1").unwrap();
        super::play_in_scene(&mut d, 0, 0, pi).unwrap();
    }
}

/// Sound `s`'s audio offset in the pattern in focus.
fn offset_of(h: &Harness<'static, App>, s: usize) -> Option<i64> {
    project_of(h).groups[0].patterns[h.state().pattern].audio.iter().find(|c| c.sound == s).map(|c| c.offset)
}

fn pedal_before(p: &deckard_model::Project, g: usize, pi: usize) -> usize {
    p.groups[g].patterns[pi].midi.iter().filter(|m| layout::Lane::of(&m.kind) == layout::Lane::Controller(64)).count()
}

fn double_click(h: &mut Harness<'static, App>, pos: Pos2) {
    h.run_steps(40);
    h.hover_at(pos);
    h.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() });
            h.step();
        }
    }
    h.run_steps(3);
}

fn type_text(h: &mut Harness<'static, App>, text: &str) {
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(Event::Text(text.into()));
    h.step();
    h.run_steps(2);
}

/// What `make_plugin_pictures` writes after the pictures' list.
const PICTURE_HELPERS: &str = r#"
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Whether DECKARD has a picture of the plug-in with this key.
pub fn has(key: &str) -> bool {
    PICTURES.iter().any(|(k, ..)| *k == key)
}

/// Give DECKARD's controller layer the pictures (thumbnails and logos).
pub fn give(s: &mut deckard_surface::Surface) {
    for (key, thumb, logo) in PICTURES {
        for (bytes, is_thumb) in [(*thumb, true), (*logo, false)] {
            if let Some((w, h, px)) = decode(bytes) {
                s.add_picture(key, is_thumb, w, h, px);
            }
        }
    }
}

/// The picture key for a chain slot (`s.name`): DECKARD's own built-in picture if it
/// has one, else the plug-in's thumbnail and logo from `NI Resources\image` (loaded on
/// demand into `s`).
pub fn slot_key(s: &deckard_surface::Surface, name: &str) -> Option<String> {
    let key = deckard_lua_host::plugin_picture_key(name);
    if key.is_empty() {
        return None;
    }
    if has(&key) || s.has_picture(&key, true) || s.has_picture(&key, false) {
        return Some(key);
    }
    with_ni_pictures(|ni| ni.load_slot(s, name, &key)).then_some(key)
}

/// Ensure the controller's surface has the browser tile (`thumb = true`) and right-screen
/// logo (`thumb = false`) for `product` under `"artwork:<lower>"`.
pub fn ensure_browser_product(s: &deckard_surface::Surface, product: &str) {
    let lower = product.trim().to_lowercase();
    if lower.is_empty() {
        return;
    }
    let key = format!("artwork:{lower}");
    if s.has_picture(&key, true) && s.has_picture(&key, false) {
        return;
    }
    with_ni_pictures(|ni| {
        ni.load_browser_product(s, &lower, &key);
    });
}

#[derive(Clone, Default)]
struct Entry {
    artwork: Option<PathBuf>,
    plugin: Option<PathBuf>,
    logo: Option<PathBuf>,
}

impl Entry {
    fn merge(&mut self, other: &Self) {
        if self.artwork.is_none() {
            self.artwork.clone_from(&other.artwork);
        }
        if self.plugin.is_none() {
            self.plugin.clone_from(&other.plugin);
        }
        if self.logo.is_none() {
            self.logo.clone_from(&other.logo);
        }
    }
}

type DecodedImage = Option<(usize, usize, Vec<[u8; 4]>)>;

#[derive(Default)]
struct NiPictures {
    root: Option<PathBuf>,
    by_name: HashMap<String, Entry>,
    by_key: HashMap<String, Entry>,
    decoded: HashMap<PathBuf, DecodedImage>,
}

fn ni_image_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("DECKARD_NI_IMAGE_DIR") {
        let p = PathBuf::from(dir);
        return p.is_dir().then_some(p);
    }
    let mut candidates = vec![
        PathBuf::from(r"C:\Users\Public\Documents\NI Resources\image"),
        PathBuf::from("/Users/Shared/NI Resources/image"),
    ];
    if let Some(pub_dir) = std::env::var_os("PUBLIC") {
        candidates.insert(0, PathBuf::from(pub_dir).join(r"Documents\NI Resources\image"));
    }
    candidates.into_iter().find(|p| p.is_dir())
}

fn strip_hash(name: &str) -> &str {
    if name.len() > 33 && name.is_char_boundary(name.len() - 33) {
        let (stem, suffix) = name.split_at(name.len() - 33);
        if suffix.starts_with('_') && suffix[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
            return stem;
        }
    }
    name
}

fn candidate_keys(name: &str) -> Vec<String> {
    let base = deckard_lua_host::plugin_picture_key(name);
    if base.is_empty() {
        return Vec::new();
    }
    let mut out = vec![base.clone()];
    let no_fx = base.strip_suffix("fx").filter(|s| !s.is_empty()).unwrap_or(&base);
    if no_fx != base {
        out.push(no_fx.to_string());
    }
    for s in [base.as_str(), no_fx] {
        let no_digits = s.trim_end_matches(|c: char| c.is_ascii_digit());
        if !no_digits.is_empty() && !out.iter().any(|x| x == no_digits) {
            out.push(no_digits.to_string());
        }
    }
    out
}

impl NiPictures {
    fn scan(root: Option<PathBuf>) -> Self {
        let mut ni = Self { root: root.clone(), ..Self::default() };
        if let Some(dir) = root {
            Self::walk(&dir, 3, &mut ni);
        }
        ni
    }

    fn walk(dir: &Path, depth: usize, ni: &mut Self) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for path in dirs {
            let artwork = ["MST_artwork.png", "VB_artwork.png"].into_iter().map(|f| path.join(f)).find(|p| p.is_file());
            let plugin = ["MST_plugin.png"].into_iter().map(|f| path.join(f)).find(|p| p.is_file());
            let logo = ["MST_logo.png", "VB_logo.png"].into_iter().map(|f| path.join(f)).find(|p| p.is_file());
            if artwork.is_some() || plugin.is_some() || logo.is_some() {
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    let entry = Entry { artwork, plugin, logo };
                    let lower = name.trim().to_lowercase();
                    let clean = strip_hash(&lower).trim().to_string();
                    ni.by_name.entry(lower).or_default().merge(&entry);
                    ni.by_name.entry(clean.clone()).or_default().merge(&entry);
                    for k in candidate_keys(&clean) {
                        ni.by_key.entry(k).or_default().merge(&entry);
                    }
                }
            }
            if depth > 1 {
                Self::walk(&path, depth - 1, ni);
            }
        }
    }

    fn lookup(&self, name: &str) -> Option<Entry> {
        let lower = strip_hash(&name.trim().to_lowercase()).trim().to_string();
        let mut found = self.by_name.get(&lower).cloned();
        for k in candidate_keys(&lower) {
            if let Some(e) = self.by_key.get(&k) {
                match &mut found {
                    Some(cur) => cur.merge(e),
                    None => found = Some(e.clone()),
                }
            }
        }
        found
    }

    fn decode_cached(&mut self, path: &Path) -> DecodedImage {
        if let Some(cached) = self.decoded.get(path) {
            return cached.clone();
        }
        let img = std::fs::read(path).ok().and_then(|b| decode(&b));
        self.decoded.insert(path.to_path_buf(), img.clone());
        img
    }

    fn load_slot(&mut self, s: &deckard_surface::Surface, name: &str, key: &str) -> bool {
        let Some(entry) = self.lookup(name) else { return false };
        let thumb_path = entry.plugin.as_ref().or(entry.artwork.as_ref()).or(entry.logo.as_ref()).cloned();
        let logo_path = entry.logo.as_ref().or(entry.plugin.as_ref()).or(entry.artwork.as_ref()).cloned();
        let mut any = false;
        if let Some(p) = thumb_path {
            if let Some((w, h, px)) = self.decode_cached(&p) {
                s.add_picture(key, true, w, h, px);
                any = true;
            }
        }
        if let Some(p) = logo_path {
            if let Some((w, h, px)) = self.decode_cached(&p) {
                s.add_picture(key, false, w, h, px);
                any = true;
            }
        }
        any
    }

    fn load_browser_product(&mut self, s: &deckard_surface::Surface, product_lower: &str, key: &str) -> bool {
        let Some(entry) = self.lookup(product_lower) else { return false };
        let thumb_path = entry.artwork.as_ref().or(entry.plugin.as_ref()).or(entry.logo.as_ref()).cloned();
        let logo_path = entry.logo.as_ref().or(entry.artwork.as_ref()).or(entry.plugin.as_ref()).cloned();
        let mut any = false;
        if let Some(p) = thumb_path {
            if let Some((w, h, px)) = self.decode_cached(&p) {
                s.add_picture(key, true, w, h, px);
                any = true;
            }
        }
        if let Some(p) = logo_path {
            if let Some((w, h, px)) = self.decode_cached(&p) {
                s.add_picture(key, false, w, h, px);
                any = true;
            }
        }
        any
    }
}

fn with_ni_pictures<R>(f: impl FnOnce(&mut NiPictures) -> R) -> R {
    static NI: OnceLock<Mutex<NiPictures>> = OnceLock::new();
    let root = ni_image_root();
    let mut guard = NI.get_or_init(|| Mutex::new(NiPictures::scan(root.clone()))).lock().expect("ni pictures lock");
    if guard.root != root {
        *guard = NiPictures::scan(root);
    }
    f(&mut guard)
}

/// A PNG as RGBA pixels (w, h, pixels).
fn decode(bytes: &[u8]) -> Option<(usize, usize, Vec<[u8; 4]>)> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let buf = &buf[..info.buffer_size()];
    let px = match info.color_type {
        png::ColorType::Rgba => buf.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect(),
        png::ColorType::Rgb => buf.chunks(3).map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks(2).map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().map(|&g| [g, g, g, 255]).collect(),
        _ => return None,
    };
    Some((info.width as usize, info.height as usize, px))
}
"#;

fn render_dir() -> Option<PathBuf> {
    std::env::var_os("DECKARD_RENDER_DIR").map(PathBuf::from)
}
