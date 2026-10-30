//! A software Maschine controller driven by the real hardware scripts.
//!
//! The scripts (from the user's own install, see `deckard-lua-host`) run as they do
//! for a connected controller: buttons and pads clicked here become `onSwitchEvent`/
//! `onPadEvent`, the app timer ticks them, and the panel shows the LEDs they set and
//! the displays drawn as NI draws them (`deckard-nwl`, from the install's style
//! sheets; screen text when those aren't there). The layout is a simplified MK3.

use deckard_lua_host::{Host, Scripts, State, state::Prop};
use eframe::egui::{self, Color32, RichText, Vec2};
use std::path::Path;
use std::time::{Duration, Instant};

/// What the panel asks the app to do after a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// PAD MODE's SHIFT + BOUNCE: bounce sound (group, sound) in place.
    Bounce(usize, usize),
    /// The scripts started or stopped the transport.
    Transport(bool),
    /// A pad was hit: play (group, sound).
    Note(usize, usize),
    /// The scripts focused another group.
    FocusGroup(usize),
    /// The scripts focused another sound of the focused group (a pad in Pad mode,
    /// SELECT + pad).
    FocusSound(usize),
    /// A scene was picked on the controller (SCENE + pad, Ideas view).
    LaunchScene(usize),
    /// A section was picked on the controller (SCENE + pad, Song view).
    LaunchSection(usize),
    /// The scripts switched between the Ideas view (true) and the Song view.
    IdeasView(bool),
    /// The scripts focused another scene (without launching it).
    FocusScene(usize),
    /// The scripts focused pattern `.1` of group `.0`.
    FocusPattern(usize, usize),
    /// The scripts focused another section (Song view, without launching it).
    FocusSection(usize),
    /// The pads switched to Keyboard mode (true) or back to Pad mode.
    KeyboardMode(bool),
    /// RESTART: play from the beginning.
    Restart,
    /// SAMPLE opened the Sampling page (true), or it closed.
    Sampling(bool),
    /// START / STOP (true) or CANCEL on the Sampling page's RECORD screen.
    Record(bool),
    /// Loop was switched on or off.
    Loop(bool),
    /// The metronome was switched on or off (SHIFT + TAP).
    Metronome(bool),
    /// REC: recording started (with a count-in when `.1`) or stopped.
    Recording(bool, bool),
    /// Settings ▸ MIDI changed Link or MIDI sync (see [`Controller::sync_view`]).
    Sync,
}

pub struct Controller {
    pub host: Host,
    pub model: String,
    /// DECKARD's built-in mods that didn't apply to these scripts (why), if any.
    pub mod_note: Option<String>,
    last_tick: Instant,
    playing: bool,
    recording: bool,
    looping: bool,
    metronome: bool,
    group: usize,
    sound: usize,
    ideas: bool,
    keyboard: bool,
    /// The Sampling page shows, and its page id.
    sampling: bool,
    sampling_page: Option<i64>,
    /// The focused scene and (group, pattern) as last reported to the app.
    scene: Option<usize>,
    section: Option<usize>,
    pattern: Option<(usize, usize)>,
    /// Modifier buttons held down by a click, released by the next click.
    held: Vec<&'static str>,
    /// NI's look for the displays, if the install's style sheets are there.
    look: Option<std::rc::Rc<deckard_nwl::draw::Look>>,
    /// The displays as last drawn: the widget trees (page, overlay), the picture (both,
    /// 960 × 272) and how many times they've changed.
    screens: Option<(Display, deckard_nwl::draw::Canvas)>,
    pub screens_drawn: u64,
    texture: Option<(u64, egui::TextureHandle)>,
    /// The note selection (group, pattern, notes) as last shared with the app.
    selection: Option<(usize, usize, Vec<deckard_model::Note>)>,
    /// Link and MIDI sync as last shared with the app.
    sync: deckard_lua_host::state::SyncView,
}

const TICK: Duration = Duration::from_millis(50);

/// How often the scripts' timer runs: faster while the song plays, so the strip's play
/// position and the pads' flashes keep time (a tick costs about a millisecond).
pub fn tick_every(playing: bool) -> Duration {
    if playing { Duration::from_millis(16) } else { TICK }
}

type Display = (deckard_nwl::draw::Element, Option<deckard_nwl::draw::Element>);

impl Controller {
    pub fn new(scripts_dir: &Path, model: &str, project: Option<deckard_doc::Shared>) -> Result<Controller, String> {
        let mut scripts = Scripts::from_dir(scripts_dir).map_err(|e| format!("{}: {e}", scripts_dir.display()))?;
        if scripts.is_empty() {
            return Err(format!("no scripts under {}", scripts_dir.display()));
        }
        // DECKARD's own mods (Pad Touch on the Keyboard page).
        let skipped = scripts.add_builtin().skipped;
        let mod_note = (!skipped.is_empty()).then(|| format!("Pad Touch mod didn't apply: {}", skipped.join("; ")));
        if let Some(n) = &mod_note {
            eprintln!("deckard: {n}");
        }
        let mut host = Host::new(scripts, State::new(model, project)).map_err(|e| e.to_string())?;
        let look = deckard_nwl::draw::Look::mk3(scripts_dir).ok().map(std::rc::Rc::new);
        if let Some(l) = &look {
            // DECKARD's own plug-in pictures, over Maschine's for the modules both have.
            for (key, thumb, logo) in crate::plugin_pictures::PICTURES {
                l.add_picture(&deckard_lua_host::plugin_picture_path(key, true), thumb);
                l.add_picture(&deckard_lua_host::plugin_picture_path(key, false), logo);
            }
            host.set_look(l.clone());
        }
        host.boot().map_err(|e| e.to_string())?;
        host.tick();
        let sampling_page = host.constant("NI.HW.PAGE_SAMPLING").ok();
        Ok(Controller {
            host,
            model: model.to_string(),
            mod_note,
            last_tick: Instant::now(),
            playing: false,
            recording: false,
            looping: false,
            metronome: false,
            group: 0,
            sound: 0,
            ideas: true,
            keyboard: false,
            sampling: false,
            sampling_page,
            scene: None,
            section: None,
            pattern: None,
            held: Vec::new(),
            look,
            screens: None,
            screens_drawn: 0,
            texture: None,
            selection: None,
            sync: Default::default(),
        })
    }

    /// The displays drawn as the scripts have them now (both, 960 × 272), redrawn
    /// only when their widgets changed.
    pub fn screens(&mut self) -> Option<&deckard_nwl::draw::Canvas> {
        let look = self.look.as_ref()?;
        let tree = self.host.display()?;
        if self.screens.as_ref().is_none_or(|(old, _)| *old != tree) {
            let mut canvas = look.render(&tree.0, 960, 272);
            if let Some(o) = &tree.1 {
                look.render_over(&mut canvas, o);
            }
            self.screens = Some((tree, canvas));
            self.screens_drawn += 1;
        }
        self.screens.as_ref().map(|(_, c)| c)
    }

    /// The style sheets' palettes: (palette, index, RGBA), for measuring colours.
    #[cfg(test)]
    pub fn palettes(&self) -> Vec<(u32, u32, [u8; 4])> {
        let Some(look) = self.look.as_ref() else { return Vec::new() };
        (0..12).flat_map(|p| (0..40).filter_map(move |i| look.sheet.palette(p, i).map(|c| (p, i, c)))).collect()
    }

    /// The displays' laid-out boxes (see [`deckard_nwl::draw::Look::layout`]), the
    /// overlay's after a `--- overlay` line.
    #[cfg(test)]
    pub fn screen_layout(&self) -> Vec<String> {
        let (Some(look), Some(tree)) = (self.look.as_ref(), self.host.display()) else { return Vec::new() };
        let mut out = look.layout(&tree.0, 960, 272);
        if let Some(o) = &tree.1 {
            out.push("--- overlay".into());
            out.extend(look.layout(o, 960, 272));
        }
        out
    }

    /// How soon the app timer is due again (for the app to wake then).
    pub fn interval(&self) -> Duration {
        tick_every(self.host.state.borrow().transport.what.is_some())
    }

    /// Run the app timer if it's due; report transport changes.
    pub fn tick(&mut self, requests: &mut Vec<Request>) {
        if self.last_tick.elapsed() >= self.interval() {
            self.last_tick = Instant::now();
            self.host.tick();
        }
        let sampling = self.sampling_page.is_some() && self.host.state.borrow().hw.page_stack.last().copied() == self.sampling_page;
        if sampling != self.sampling {
            self.sampling = sampling;
            requests.push(Request::Sampling(sampling));
        }
        let press = self.host.state.borrow_mut().record_press.take();
        if let Some(p) = press {
            requests.push(Request::Record(p == deckard_lua_host::state::RecordPress::Toggle));
        }
        let keyboard = self.host.keyboard_mode();
        if keyboard != self.keyboard {
            self.keyboard = keyboard;
            requests.push(Request::KeyboardMode(keyboard));
        }
        let (restarted, looping, metronome, recording, count_in) = {
            let mut st = self.host.state.borrow_mut();
            let t = &mut st.transport;
            (std::mem::take(&mut t.restarted), t.looping, t.metronome, t.recording, std::mem::take(&mut t.count_in))
        };
        // SHIFT + REC while armed asks again, with the count-in.
        if recording != self.recording || (recording && count_in) {
            self.recording = recording;
            requests.push(Request::Recording(recording, count_in));
        }
        if metronome != self.metronome {
            self.metronome = metronome;
            requests.push(Request::Metronome(metronome));
        }
        if looping != self.looping {
            self.looping = looping;
            requests.push(Request::Loop(looping));
        }
        let sync = self.host.state.borrow().sync.clone();
        if sync != self.sync {
            self.sync = sync;
            requests.push(Request::Sync);
        }
        let (playing, (group, sound), ideas, launched, section, scene, pattern, focus_section) = {
            let mut st = self.host.state.borrow_mut();
            let pattern = st.focus.pattern.map(|p| (st.focus.group, p));
            (
                st.transport.playing,
                (st.focus.group, st.focus.sound),
                st.ideas_view,
                st.launched_scene.take(),
                st.launched_section.take(),
                st.focus.scene,
                pattern,
                st.focus.section,
            )
        };
        if focus_section != self.section {
            self.section = focus_section;
            if let Some(s) = focus_section.filter(|_| section.is_none()) {
                requests.push(Request::FocusSection(s));
            }
        }
        if let Some(scene) = launched {
            requests.push(Request::LaunchScene(scene));
        }
        if let Some((g, s)) = self.host.state.borrow_mut().bounce_request.take() {
            requests.push(Request::Bounce(g, s));
        }
        if let Some(section) = section {
            requests.push(Request::LaunchSection(section));
        }
        if ideas != self.ideas {
            self.ideas = ideas;
            requests.push(Request::IdeasView(ideas));
        }
        if restarted {
            self.playing = playing;
            requests.push(Request::Restart);
        } else if playing != self.playing {
            self.playing = playing;
            requests.push(Request::Transport(playing));
        }
        if group != self.group {
            self.group = group;
            requests.push(Request::FocusGroup(group));
        }
        if sound != self.sound {
            self.sound = sound;
            requests.push(Request::FocusSound(sound));
        }
        if scene != self.scene {
            self.scene = scene;
            if let Some(s) = scene.filter(|_| launched.is_none()) {
                requests.push(Request::FocusScene(s));
            }
        }
        if pattern != self.pattern {
            self.pattern = pattern;
            if let Some((g, p)) = pattern {
                requests.push(Request::FocusPattern(g, p));
            }
        }
    }

    /// Press a button by name (`BUTTON_TRANSPORT_PLAY`), as from the hardware.
    pub fn press(&mut self, button: &str, requests: &mut Vec<Request>) {
        let _ = self.host.press(button);
        self.host.tick();
        self.tick(requests);
    }

    /// Hold or release a button.
    pub fn hold(&mut self, button: &str, pressed: bool, requests: &mut Vec<Request>) {
        let _ = self.host.button(button, pressed);
        self.host.tick();
        self.tick(requests);
    }

    /// Hit pad `pad` (1-based). It plays its sound unless the pads are picking scenes
    /// or sections.
    pub fn pad(&mut self, pad: usize, requests: &mut Vec<Request>) {
        let plays = self.host.pads_play_sounds();
        self.host.pad(pad as i64, 1.0);
        self.host.tick();
        if plays {
            let group = self.host.state.borrow().focus.group;
            requests.push(Request::Note(group, pad - 1));
        }
        self.tick(requests);
    }

    /// A pad pressed (with its velocity, 0..1) or released on the hardware. The pad's
    /// sound already played (from the controller's thread).
    pub fn pad_event(&mut self, pad: usize, trigger: bool, value: f64, requests: &mut Vec<Request>) {
        self.host.pad_event(pad as i64, trigger, value);
        self.host.tick();
        self.tick(requests);
    }

    /// A knob (`ENCODER_SCREEN_1`…) turned by `inc` (a fraction of a turn).
    pub fn encoder(&mut self, encoder: &str, inc: f64, requests: &mut Vec<Request>) {
        let _ = self.host.encoder(encoder, inc);
        self.host.tick();
        self.tick(requests);
    }

    /// The 4-D encoder turned by `steps` clicks.
    pub fn wheel(&mut self, steps: i32, requests: &mut Vec<Request>) {
        self.host.wheel(steps as i64);
        self.host.tick();
        self.tick(requests);
    }

    /// The touch strip (finger, 0 touched / 1 moved / 2 released, position 0..1).
    pub fn touch(&mut self, finger: usize, kind: u8, pos: f64, requests: &mut Vec<Request>) {
        self.host.touch(finger as i64, kind as i64, pos);
        self.host.tick();
        self.tick(requests);
    }

    /// Latch a modifier button down, as clicking it does.
    pub fn latch(&mut self, button: &str, requests: &mut Vec<Request>) {
        if let Some(b) = MODIFIERS.iter().copied().find(|b| *b == button)
            && !self.held.contains(&b)
        {
            self.held.push(b);
            self.hold(b, true, requests);
        }
    }

    /// Whether the Sampling page shows.
    pub fn sampling(&self) -> bool {
        self.sampling
    }

    /// The transport was started or stopped in the app's own UI.
    pub fn set_playing(&mut self, playing: bool) {
        self.host.state.borrow_mut().transport.playing = playing;
        self.playing = playing;
    }

    /// Recording was started or stopped in the app's own UI.
    pub fn set_recording(&mut self, on: bool) {
        self.host.state.borrow_mut().transport.recording = on;
        self.recording = on;
    }

    /// Loop was switched in the app's own UI.
    pub fn set_looping(&mut self, on: bool) {
        self.host.state.borrow_mut().transport.looping = on;
        self.looping = on;
    }

    /// Link and MIDI sync as the app has them now (the Settings ▸ MIDI page shows them).
    pub fn set_sync(&mut self, view: deckard_lua_host::state::SyncView) {
        if view != self.sync {
            self.host.set_sync(view.clone());
            self.sync = view;
        }
    }

    /// Link and MIDI sync as the scripts left them (after [`Request::Sync`]).
    pub fn sync_view(&self) -> deckard_lua_host::state::SyncView {
        self.sync.clone()
    }

    /// The metronome was switched in the app's own UI.
    pub fn set_metronome(&mut self, on: bool) {
        self.host.state.borrow_mut().transport.metronome = on;
        self.metronome = on;
    }

    /// Tell the scripts about a change made in the app's own UI.
    pub fn sync(&mut self, (group, sound): (usize, usize), scene: Option<usize>, section: Option<usize>, ideas: bool) {
        let mut st = self.host.state.borrow_mut();
        if section.is_some() {
            st.focus.section = section;
            self.section = section;
        }
        if st.focus.group != group {
            st.focus.group = group;
            self.group = group;
        }
        if st.focus.sound != sound {
            st.focus.sound = sound;
            self.sound = sound;
        }
        if scene.is_some() {
            st.focus.scene = scene;
            self.scene = scene;
        }
        st.ideas_view = ideas;
        self.ideas = ideas;
    }

    /// Share the pattern editor's step and nudge grids and note selection with the
    /// scripts (the Step and Events pages edit and select the same notes, the Grid page
    /// sets the same grids): whichever side changed them since the last call wins.
    pub fn sync_selection(
        &mut self,
        grid: &mut i64,
        nudge: &mut usize,
        group: usize,
        pattern: usize,
        selected: &mut Vec<deckard_model::Note>,
    ) {
        let mut st = self.host.state.borrow_mut();
        st.sync_grid(grid, nudge);
        if st.selection != self.selection {
            self.selection = st.selection.clone();
            *selected = match &st.selection {
                Some((g, p, notes)) if (*g, *p) == (group, pattern) => notes.clone(),
                _ => Vec::new(),
            };
        } else {
            let mine = (!selected.is_empty()).then(|| (group, pattern, selected.clone()));
            if mine != self.selection {
                st.selection = mine.clone();
                self.selection = mine;
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, requests: &mut Vec<Request>) {
        ui.horizontal(|ui| {
            ui.strong(format!("Controller ({})", self.model));
            let errors = self.host.errors.len();
            if errors > 0 {
                ui.colored_label(Color32::LIGHT_RED, format!("{errors} script errors"))
                    .on_hover_text(self.host.errors.last().cloned().unwrap_or_default());
            }
        });
        // The eight buttons above the displays, labelled as the scripts label them.
        let mut screen_press = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, b) in self.host.screen_buttons().into_iter().enumerate() {
                let text = if b.visible { b.label.clone() } else { String::new() };
                let ink = if b.selected { Color32::BLACK } else { Color32::from_gray(220) };
                let mut button =
                    egui::Button::new(RichText::new(text).size(9.0).color(ink)).min_size(Vec2::new(44.0, 18.0)).selected(b.selected);
                if b.selected {
                    button = button.fill(Color32::from_gray(200));
                }
                if ui.add_enabled(b.enabled, button).on_hover_text(format!("Screen button {}", i + 1)).clicked() {
                    screen_press = Some(i + 1);
                }
            }
        });
        if let Some(i) = screen_press {
            self.press(&format!("BUTTON_SCREEN_{i}"), requests);
        }
        self.screen(ui);
        ui.add_space(6.0);

        let mut press = None;
        ui.horizontal(|ui| {
            for (label, button, led) in [
                ("|◀", "BUTTON_TRANSPORT_LOOP", "LED_TRANSPORT_LOOP"),
                ("▶", "BUTTON_TRANSPORT_PLAY", "LED_TRANSPORT_PLAY"),
                ("●", "BUTTON_TRANSPORT_RECORD", "LED_TRANSPORT_RECORD"),
                ("■", "BUTTON_TRANSPORT_STOP", "LED_TRANSPORT_STOP"),
                ("TAP", "BUTTON_TAP", "LED_TAP"),
            ] {
                if self.button(ui, label, led, Vec2::new(36.0, 24.0)).clicked() {
                    press = Some(button);
                }
            }
            ui.separator();
            // On the hardware these pages show while their button is held. A mouse
            // can't hold a button and hit a pad, so a click latches the button down and
            // the next click releases it (e.g. SCENE, then pads pick scenes).
            for (label, button, led) in [
                ("SCENE", "BUTTON_SCENE", "LED_SCENE"),
                ("PATTERN", "BUTTON_PATTERN", "LED_PATTERN"),
                ("SHIFT", "BUTTON_SHIFT", "LED_SHIFT"),
            ] {
                let latched = self.held.contains(&button);
                let resp = self.button(ui, label, led, Vec2::new(58.0, 24.0));
                let resp = if latched { resp.highlight() } else { resp };
                let tip = if latched { "Held: click to release" } else { "Click to hold (as holding the button on the hardware)" };
                if resp.on_hover_text(tip).clicked() {
                    if latched {
                        self.held.retain(|b| *b != button);
                    } else {
                        self.held.push(button);
                    }
                    self.hold(button, !latched, requests);
                }
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            // Group buttons A–H, two rows of four as on the hardware.
            ui.vertical(|ui| {
                for row in [b"ABCD", b"EFGH"] {
                    ui.horizontal(|ui| {
                        for &c in row {
                            let c = c as char;
                            if self.button(ui, &c.to_string(), &format!("LED_GROUP_{c}"), Vec2::new(34.0, 30.0)).clicked() {
                                press = Some(GROUP_BUTTONS[(c as u8 - b'A') as usize]);
                            }
                        }
                    });
                }
            });
            ui.add_space(12.0);
            // Pads: 13–16 on top, 1–4 at the bottom.
            ui.vertical(|ui| {
                for row in (0..4).rev() {
                    ui.horizontal(|ui| {
                        for col in 0..4 {
                            let pad = row * 4 + col + 1;
                            // Pads fire on press, as the hardware does.
                            let resp = self.button(ui, &pad.to_string(), &format!("LED_PAD_{pad}"), Vec2::splat(44.0));
                            if crate::layout::pressed(ui, &resp) {
                                self.pad(pad, requests);
                            }
                        }
                    });
                }
            });
        });
        if let Some(b) = press {
            self.press(b, requests);
        }
    }

    /// A button lit with its LED's colour.
    fn button(&self, ui: &mut egui::Ui, label: &str, led: &str, size: Vec2) -> egui::Response {
        let fill = match self.host.led_rgb(led) {
            Some([r, g, b]) => Color32::from_rgb(r, g, b),
            None => Color32::from_gray(45),
        };
        let text = if fill.r() as u32 + fill.g() as u32 + fill.b() as u32 > 300 { Color32::BLACK } else { Color32::WHITE };
        ui.add(egui::Button::new(RichText::new(label).color(text)).fill(fill).min_size(size))
    }

    /// The active page's screens: as drawn on the hardware, or as text.
    fn screen(&mut self, ui: &mut egui::Ui) {
        if self.screens().is_some() {
            let drawn = self.screens_drawn;
            if self.texture.as_ref().is_none_or(|(n, _)| *n != drawn) {
                let c = &self.screens.as_ref().unwrap().1;
                let image = egui::ColorImage::from_rgba_unmultiplied([c.width, c.height], &c.px.concat());
                let options = egui::TextureOptions::NEAREST;
                match &mut self.texture {
                    Some((n, t)) => {
                        t.set(image, options);
                        *n = drawn;
                    }
                    None => self.texture = Some((drawn, ui.ctx().load_texture("controller-screens", image, options))),
                }
            }
            if let Some((_, t)) = &self.texture {
                let w = ui.available_width().min(960.0);
                ui.image((t.id(), Vec2::new(w, w * 272.0 / 960.0)));
            }
            return;
        }
        self.screen_text(ui);
    }

    /// The active page's screens as text: the widgets with text, left and right.
    fn screen_text(&self, ui: &mut egui::Ui) {
        let st = self.host.state.borrow();
        let Some(root) = self.host.active_screen() else {
            ui.weak("(no screen)");
            return;
        };
        let page = &st.widgets.all[root];
        let halves: Vec<usize> = page.children.iter().copied().filter(|&c| st.widgets.all[c].name.contains("Screen")).collect();
        let halves = if halves.is_empty() { vec![root] } else { halves };
        ui.label(RichText::new(&page.name).small().weak());
        ui.horizontal_top(|ui| {
            for h in halves {
                let mut lines = Vec::new();
                texts(&st.widgets, h, &mut lines);
                egui::Frame::new().fill(Color32::from_rgb(12, 12, 16)).inner_margin(6.0).corner_radius(3.0).show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(170.0);
                        ui.set_min_height(110.0);
                        for l in lines.iter().take(12) {
                            let text = RichText::new(l).monospace().size(11.0).color(Color32::from_rgb(220, 225, 235));
                            ui.add(egui::Label::new(text).truncate());
                        }
                    });
                });
            }
        });
    }
}

const MODIFIERS: [&str; 3] = ["BUTTON_SCENE", "BUTTON_PATTERN", "BUTTON_SHIFT"];

const GROUP_BUTTONS: [&str; 8] = [
    "BUTTON_GROUP_A",
    "BUTTON_GROUP_B",
    "BUTTON_GROUP_C",
    "BUTTON_GROUP_D",
    "BUTTON_GROUP_E",
    "BUTTON_GROUP_F",
    "BUTTON_GROUP_G",
    "BUTTON_GROUP_H",
];

/// Visible text of the widgets under `id`, in tree order; labels on one bar are
/// joined on one line.
fn texts(w: &deckard_lua_host::state::Widgets, id: usize, out: &mut Vec<String>) {
    let widget = &w.all[id];
    if matches!(widget.props.get("Visible"), Some(Prop::Bool(false))) {
        return;
    }
    let own = |x: &deckard_lua_host::state::Widget| match (x.props.get("Text"), x.props.get("style")) {
        (Some(Prop::Str(t)), _) if !t.is_empty() => Some(t.clone()),
        (_, Some(Prop::Str(t))) if !t.is_empty() && x.kind.contains("Label") => Some(t.clone()),
        _ => None,
    };
    let leaves: Vec<String> = widget
        .children
        .iter()
        .map(|&c| &w.all[c])
        .filter(|c| c.children.is_empty() && !matches!(c.props.get("Visible"), Some(Prop::Bool(false))))
        .filter_map(own)
        .collect();
    if let Some(t) = own(widget) {
        out.push(t);
    }
    if !leaves.is_empty() {
        out.push(leaves.join("  "));
    }
    for &c in &widget.children {
        if !w.all[c].children.is_empty() {
            texts(w, c, out);
        }
    }
}

#[cfg(test)]
mod tick_tests {
    /// Playing, the scripts look often enough to catch the strip's last LED: NI lights
    /// it for the last 1/48 of the loop (`value2Led` rounds over 24 steps), a 1-bar loop
    /// at 180 BPM's last 28 ms.
    #[test]
    fn playing_ticks_catch_the_strips_last_led() {
        let window = std::time::Duration::from_secs_f64(4.0 * 60.0 / 180.0 / 48.0);
        assert!(super::tick_every(true) < window, "{:?} vs {window:?}", super::tick_every(true));
        assert_eq!(super::tick_every(false), super::TICK, "stopped: as before");
    }
}
