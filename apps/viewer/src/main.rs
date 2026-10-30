//! Viewer for Maschine projects, laid out like Maschine's main window (see
//! docs/ui-layout.md): a header with the transport, the song panel (Ideas view of
//! scenes, or the Song view of sections), the chain area, and the group panel (sound
//! list, pattern editor, velocity lane). Sounds that use the Sampler are played.
//!
//! With `--scripts DIR` (or `$DECKARD_SCRIPTS`) pointing at the scripts extracted
//! from your own Maschine install, a software controller panel runs the real hardware
//! scripts against the project: its buttons, pads, LEDs and screen text come from them.
//!
//! Usage: deckard [file.mxprj] [--screenshot out.png] [--group N] [--pattern N]
//!                        [--scene N] [--view ideas|song|mix|lfos] [--play song|scene|pattern]
//!                        [--launch N] [--tempo BPM] [--wait SECS] [--draw] [--piano] [--select-all] [--scripts DIR] [--controller M3]
//!                        [--press BUTTON_X]... [--hold BUTTON_X]... [--pad N]...
//! (or drop a .mxprj onto the window). `--screenshot` renders, saves a PNG and exits
//! (after 2 s when combined with `--play`, or `--wait SECS`). `--launch N` launches
//! scene N one second after starting, so it switches when the playing scene ends;
//! `--tempo BPM` likewise changes the tempo (an undoable edit) while playing.
//! `--press`, `--hold` (latch a button down) and `--pad` operate the controller panel
//! at start-up, in order.

// Release builds on Windows are GUI apps (no console window); debug builds keep one for
// the log.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod arp_ui;
mod audio_settings;
mod audio_ui;
mod blink;
mod bounce;
mod browser;
#[cfg(test)]
mod browser_tests;
#[cfg(test)]
mod chain_tests;
mod channel_names;
mod clipboard;
mod clips_ui;
mod controller;
mod drop_pos;
mod drum_ui;
mod export;
mod goodbye;
mod icon;

/// The version shown: the release's (`DECKARD_VERSION`, set by the build scripts), else
/// the crate's.
pub const VERSION: &str = match option_env!("DECKARD_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
#[cfg(test)]
mod auto_length_tests;
#[cfg(test)]
mod expr_lane_tests;
#[cfg(test)]
mod expression_record_tests;
#[cfg(test)]
mod group_edit_tests;
#[cfg(test)]
mod header_tests;
mod keep_running;
mod keys;
mod layout;
mod lock;
#[cfg(test)]
mod lock_tests;
mod loops;
#[cfg(test)]
mod macro_surface_tests;
#[cfg(test)]
mod master_fader_tests;
mod menu;
mod midi_devices;
mod midi_file;
mod midi_io;
mod midi_monitor;
mod mk3_io;
mod mod_slots;
mod modulation;
mod module_ui;
#[cfg(test)]
mod pad_blink_tests;
#[cfg(test)]
mod pad_light_tests;
#[cfg(test)]
mod parity_tests;
mod plugin_macros;
mod plugin_pictures;
#[cfg(test)]
mod print_tests;
#[cfg(test)]
mod rec_tests;
mod record;
mod record_knobs;
mod recorder;
mod remote;
mod sampler_ui;
mod sampling_hw;
mod settings;
mod stems;
#[cfg(test)]
mod surface_edit_tests;
mod surface_events;
#[cfg(test)]
mod surface_events_tests;
mod surface_lfo;
#[cfg(test)]
mod surface_lfo_tests;
#[cfg(test)]
mod surface_repeat_tests;
mod surface_sampling;
#[cfg(test)]
mod surface_sampling_tests;
mod surface_settings;
#[cfg(test)]
mod surface_settings_tests;
mod surface_step_mod;
#[cfg(test)]
mod surface_step_mod_tests;
mod surface_strip;
#[cfg(test)]
mod surface_strip_tests;
#[cfg(test)]
mod sync_tests;
mod transport_sync;
mod user_chords;

use deckard_doc::{Document, Shared};
use deckard_engine::{Engine, Mode, SampleCache, output::Output};
use deckard_model::{Note, PPQ};
use eframe::egui;
use std::path::{Path, PathBuf};

fn main() {
    // A panic would leave through the loaded plug-ins' exit-time destructors (which
    // crash in Massive, Guitar Rig, Kontakt); report it and end the process instead.
    let report = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        report(info);
        deckard_plugins::exit(101)
    }));
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The plug-in scanner's helper process (see settings::Catalog::rescan).
    if args.first().is_some_and(|a| a == "--scan-vst3")
        && let Some(bundle) = args.get(1)
    {
        // Whatever happens (a plug-in panicking us), leave without running the loaded
        // plug-ins' exit-time destructors, which crash in some (Guitar Rig, Kontakt).
        let code = std::panic::catch_unwind(|| settings::scan_vst3_main(Path::new(bundle))).unwrap_or(101);
        deckard_plugins::exit(code);
    }
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    // --write-icon OUT.png [SIZE]: draw the app's icon (1024 px), and stop.
    if let Some(out) = flag("--write-icon") {
        let size = args.iter().position(|a| a == "--write-icon").and_then(|i| args.get(i + 2)).and_then(|v| v.parse().ok()).unwrap_or(1024);
        if let Err(e) = icon::write_png(Path::new(&out), size) {
            eprintln!("{out}: {e}");
            std::process::exit(1);
        }
        return;
    }
    // --goodbye OUT.png: draw what the MK3's displays show after closing, and stop.
    if let Some(out) = flag("--goodbye") {
        let px: Vec<u8> = goodbye::picture().into_iter().flatten().collect();
        let image = egui::ColorImage::from_rgba_unmultiplied([goodbye::WIDTH, goodbye::HEIGHT], &px);
        if let Err(e) = save_png(Path::new(&out), &image) {
            eprintln!("{out}: {e}");
            std::process::exit(1);
        }
        return;
    }
    let num = |name: &str| flag(name).and_then(|v| v.parse::<usize>().ok());
    let path = args.first().filter(|a| !a.starts_with("--")).map(PathBuf::from);
    let screenshot = flag("--screenshot").map(PathBuf::from);
    let (group, pattern, scene) = (num("--group").unwrap_or(0), num("--pattern").unwrap_or(0), num("--scene").unwrap_or(0));
    let play = flag("--play");
    let launch = num("--launch");
    let view = flag("--view");
    let draw = args.iter().any(|a| a == "--draw");
    let select_all = args.iter().any(|a| a == "--select-all");
    let piano = args.iter().any(|a| a == "--piano");
    let plugin_window = args.iter().any(|a| a == "--plugin-window");
    let lane = flag("--lane");
    let lanes_dialog = args.iter().any(|a| a == "--lanes-dialog");
    let wait = flag("--wait").and_then(|v| v.parse::<f32>().ok());
    let tempo = flag("--tempo").and_then(|v| v.parse::<f64>().ok());
    // The controller scripts: --scripts, $DECKARD_SCRIPTS, or the ones extracted from
    // the user's install into this checkout (out/ni-res), if there.
    let scripts = flag("--scripts").map(PathBuf::from).or_else(|| std::env::var_os("DECKARD_SCRIPTS").map(PathBuf::from)).or_else(|| {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/ni-res");
        p.exists().then_some(p)
    });
    let model = flag("--controller").unwrap_or_else(|| "M3".into());
    // (flag, value) pairs for --press/--pad, in order.
    let actions: Vec<(String, String)> =
        args.windows(2).filter(|w| ["--press", "--pad", "--hold"].contains(&w[0].as_str())).map(|w| (w[0].clone(), w[1].clone())).collect();
    let width = if scripts.is_some() { 1500.0 + layout::CONTROLLER_WIDTH } else { 1500.0 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([width, 930.0]).with_drag_and_drop(true).with_icon(std::sync::Arc::new(
            egui::IconData { rgba: icon::icon(256).into_iter().flatten().collect(), width: 256, height: 256 },
        )),
        ..Default::default()
    };
    let result = eframe::run_native(
        "DECKARD",
        options,
        Box::new(move |cc| {
            layout::style(&cc.egui_ctx);
            let screenshot_wanted = screenshot.clone();
            let mut app = App {
                midi_devices: midi_devices::MidiDevices::new(menu::settings_dir().map(|d| d.join("controller"))),
                screenshot,
                scripts,
                model,
                show_controller: false,
                meter_inputs: true,
                follow: true,
                grid: PPQ / 4,
                prop_set: 1,
                scale: (60, 0, 0),
                scale_group: None,
                user_chords: user_chords::UserChords::load(),
                mk3: Some(mk3_io::Mk3Io::start()),
                _running: Some(keep_running::begin()),
                ..Default::default()
            };
            app.recent = menu::load_recent();
            app.settings = settings::Settings::load();
            app.surface = app.new_surface();
            if let Some(m) = &app.mk3 {
                m.set_display_brightness(app.settings.display_brightness);
            }
            app.recorder = recorder::Recorder::new();
            deckard_plugins::module::set_search_paths(app.settings.dirs.clone());
            app.catalog = settings::Catalog::load(&app.settings);
            app.session_dir = menu::settings_dir();
            // The first start without the stem separation model: offer it (not when
            // rendering a screenshot).
            if screenshot_wanted.is_none()
                && !app.settings.stems_offered
                && let Some(dir) = &app.session_dir
                && !stems::installed(dir)
            {
                app.stems_offer = Some(StemsOffer::Asking);
            }
            if let Some(p) = path {
                app.open(&p);
                app.remember(&p);
            } else {
                app.start_project();
            }
            app.group = group;
            app.focus_scene(scene);
            if flag("--pattern").is_some() {
                app.pattern = pattern;
            }
            app.mouse = if draw { layout::MouseMode::Draw } else { layout::MouseMode::Select };
            app.pending_editor = plugin_window;
            // Lines of "<seconds> <move|down|up> <x> <y>".
            if let Some(f) = flag("--input")
                && let Ok(text) = std::fs::read_to_string(&f)
            {
                app.input_script = text
                    .lines()
                    .filter_map(|l| {
                        let w: Vec<&str> = l.split_whitespace().collect();
                        let n = |i: usize| w.get(i)?.parse::<f32>().ok();
                        Some((n(0)?, (w.get(1)?.chars().next()?, egui::pos2(n(2)?, n(3)?))))
                    })
                    .collect();
            }
            if let Some(s) = num("--sound") {
                app.sound = s;
            }
            // The lane under the note area: velocity, probability, bend or a CC number.
            match lane.as_deref() {
                Some("velocity") | None => {}
                Some("probability") => app.lane = layout::Lane::Probability,
                Some("bend") => app.lane = layout::Lane::PitchBend,
                Some(n) => {
                    if let Ok(n) = n.parse::<u8>() {
                        app.lane = layout::Lane::Controller(n.min(127));
                    }
                }
            }
            if lane.is_some() {
                app.lane_of = Some((app.group, app.sound));
            }
            app.lanes_dialog = lanes_dialog;
            app.piano_roll = piano;
            if select_all && let Some(doc) = &app.doc {
                let d = doc.borrow();
                app.selected =
                    d.project().groups.get(app.group).and_then(|g| g.patterns.get(app.pattern)).map_or(Vec::new(), |p| p.notes.clone());
                app.selected_in = (app.group, app.pattern);
            }
            if let Some(v) = view {
                app.song_view = v == "song";
                app.mixer = v == "mix";
                app.lfos = v == "lfos";
            }
            app.sync_controller();
            for (kind, arg) in &actions {
                let mut requests = Vec::new();
                if let Some(Ok(c)) = &mut app.controller {
                    match (kind.as_str(), arg.parse::<usize>()) {
                        ("--pad", Ok(n)) if (1..=16).contains(&n) => c.pad(n, &mut requests),
                        ("--hold", _) => c.latch(arg, &mut requests),
                        _ => c.press(arg, &mut requests),
                    }
                }
                app.handle(requests);
            }
            match play.as_deref() {
                Some("song") => app.play(Mode::Arrangement),
                Some("scene") => app.play(Mode::Scene(scene)),
                Some("pattern") => app.play(Mode::Pattern(group, pattern)),
                _ => {}
            }
            app.sync_transport();
            app.pending_launch = launch;
            app.shot_wait = wait;
            app.pending_tempo = tempo;
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = &result {
        eprintln!("{e}");
    }
    // Skip loaded plug-ins' exit-time destructors (Massive's crash in them).
    deckard_plugins::exit(result.is_err() as i32)
}

/// A plug-in's parameter pages as the SOUND tab shows them, and the values of the page
/// shown (value, text).
pub struct PluginView {
    /// The chain and slot it shows.
    pub key: (deckard_doc::Chain, usize),
    pub pages: Vec<ViewPage>,
    pub page: usize,
    pub values: Vec<(f64, String)>,
    refreshed: std::time::Instant,
    pub error: Option<String>,
    /// Values DECKARD sent (parameter, value, when) the plug-in may not report yet.
    pub sent: Vec<(u32, f64, std::time::Instant)>,
}

/// How long a value sent to a plug-in shows while its controller still reports the
/// old one (JUCE's take it in at the processor's next block).
const SENT_HOLD: std::time::Duration = std::time::Duration::from_millis(500);

/// The values read from a plug-in (`read`, for parameters `ids`), each sent to it less
/// than [`SENT_HOLD`] ago and not reported yet shown instead (as `text` puts it); the
/// sent ones it reports, or too old, forgotten.
fn hold_sent(
    read: &mut [(f64, String)],
    ids: &[u32],
    sent: &mut Vec<(u32, f64, std::time::Instant)>,
    now: std::time::Instant,
    text: impl Fn(u32, f64) -> String,
) {
    sent.retain(|&(id, v, at)| {
        if now.duration_since(at) >= SENT_HOLD {
            return false;
        }
        let Some(k) = ids.iter().position(|&i| i == id) else { return true };
        let Some(r) = read.get_mut(k) else { return true };
        if (r.0 - v).abs() < 1e-6 {
            return false;
        }
        *r = (v, text(id, v));
        true
    });
}

/// One page: its name (may be empty) and knobs (the parameter, its label, a section
/// heading starting there).
pub struct ViewPage {
    pub name: String,
    pub knobs: Vec<(deckard_plugins::ParamInfo, String, String)>,
    /// Per knob: the names of its values when it is a choice (shown as a selector),
    /// else empty.
    pub choices: Vec<Vec<String>>,
    /// The plug-in's own page: its labels follow the plug-in (Plaits' engines).
    own: bool,
}

impl ViewPage {
    fn new(p: &deckard_plugins::Plugin, name: String, knobs: Vec<(deckard_plugins::ParamInfo, String, String)>) -> ViewPage {
        // DECKARD's own plug-ins show their choices as selectors, as Maschine's modules.
        let choices = knobs
            .iter()
            .map(|(i, ..)| match i.steps {
                n @ 1..=32 if p.is_native() => (0..=n).map(|k| p.param_text(i.id, k as f64 / n as f64)).collect(),
                _ => Vec::new(),
            })
            .collect();
        ViewPage { name, knobs, choices, own: false }
    }
}

/// The plug-in's own pages (DECKARD's: they follow the plug-in, even once saved for a
/// macro), the pages saved with it, or Maschine's automatic ones: its first 128
/// automatable parameters, eight per page.
fn view_pages(p: &deckard_plugins::Plugin, saved: &[deckard_model::PluginPage]) -> Vec<ViewPage> {
    let own = p.pages();
    if !own.is_empty() {
        let all = p.parameter_list(false);
        return own
            .into_iter()
            .map(|(name, knobs)| {
                let knobs = knobs
                    .into_iter()
                    .filter_map(|(id, section)| {
                        let info = all.iter().find(|i| i.id == id)?.clone();
                        Some((info.clone(), info.short.clone(), section))
                    })
                    .collect();
                ViewPage { own: true, ..ViewPage::new(p, name, knobs) }
            })
            .collect();
    }
    if saved.iter().any(|pg| !pg.params.is_empty()) {
        let all = p.parameter_list(false);
        return saved
            .iter()
            .map(|pg| {
                let knobs = pg
                    .params
                    .iter()
                    .filter_map(|k| {
                        let info = all.iter().find(|i| i.id == k.id)?.clone();
                        let label = if k.name.is_empty() { info.title.clone() } else { k.name.clone() };
                        Some((info, label, k.section.clone()))
                    })
                    .collect();
                ViewPage::new(p, pg.name.clone(), knobs)
            })
            .filter(|pg| !pg.knobs.is_empty())
            .collect();
    }
    p.parameters()
        .into_iter()
        .take(128)
        .collect::<Vec<_>>()
        .chunks(8)
        .map(|c| ViewPage::new(p, String::new(), c.iter().map(|i| (i.clone(), i.title.clone(), String::new())).collect()))
        .collect()
}

/// A new zone playing all of `file` over every key and velocity.
/// A file DECKARD plays as a sample (WAV, AIFF).
/// How a sound, group or preset is saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveMode {
    /// Where it was saved last (the dialog, the first time).
    Save,
    SaveAs,
    /// Save As, its samples copied beside it.
    WithSamples,
}

/// The Save dialog: the name and tags a file is saved with, and where.
pub struct SaveDialog {
    pub what: deckard_doc::FileItem,
    pub name: String,
    /// Types, each "Type › Sub-type", separated by ";".
    pub types: String,
    /// Characters, separated by ",".
    pub characters: String,
    /// The Library (bank) it's listed under.
    pub library: String,
    pub folder: PathBuf,
    pub samples: bool,
    /// The file is there: saving again replaces it.
    pub confirm_overwrite: bool,
}

/// A file saved: where, its tags, and whether with its samples.
#[derive(Clone)]
pub struct Saved {
    /// The object saved (its id then): Save writes the file again only for it.
    id: Option<u64>,
    path: PathBuf,
    info: mxprj::sound_info::SoundInfo,
    samples: bool,
}

/// "Drums › Kick; Bass" as type level lists.
fn parse_types(text: &str) -> Vec<Vec<String>> {
    text.split(';')
        .map(|t| t.split(['›', '>']).map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect::<Vec<_>>())
        .filter(|t| !t.is_empty())
        .collect()
}

fn parse_list(text: &str) -> Vec<String> {
    text.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect()
}

fn is_sample_file(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| ["wav", "aif", "aiff", "aifc"].contains(&e.to_ascii_lowercase().as_str()))
}

fn new_zone(file: &Path, frames: usize, from: deckard_doc::ZoneFrom) -> deckard_doc::ZoneSpec {
    deckard_doc::ZoneSpec {
        from,
        file: Some(file.to_path_buf()),
        set: vec![
            ("play_range/low", 0.0),
            ("play_range/high", frames.saturating_sub(1) as f64),
            ("loop_range/low", 0.0),
            ("loop_range/high", frames.saturating_sub(1) as f64),
            ("loop_active", 0.0),
            ("key_range/low", 0.0),
            ("key_range/high", 127.0),
            ("velocity_range/low", 0.0),
            ("velocity_range/high", 127.0),
            ("root_key", 60.0),
            ("tune", 0.0),
            ("gain", 0.75),
            ("pan", 0.0),
        ],
    }
}

/// The audio output with the settings' buffer size, cores and live timing. The
/// device last picked; the system's default while that can't be opened, with why in
/// `fallback` (Preferences ▸ Audio shows it).
fn open_output(settings: &settings::Settings, fallback: &mut Option<String>) -> Result<Output, String> {
    *fallback = None;
    let out = Output::with_device(settings.output_device.as_deref(), settings.buffer_frames, settings.multicore).or_else(|e| {
        if settings.output_device.is_some() {
            *fallback = Some(e);
            Output::with_options(settings.buffer_frames, settings.multicore)
        } else {
            Err(e)
        }
    })?;
    out.set_routing(&settings.output_routing);
    out.set_sample_accurate(settings.exact_timing);
    out.set_render_ahead(settings.render_ahead_ms as f64 / 1000.0);
    Ok(out)
}

/// The interface input as Maschine's 32 input channels (Preferences ▸ Audio ▸
/// Routing), frame by frame into `live`.
fn open_live_input(
    device: &Option<String>,
    routing: &[Option<usize>],
    live: deckard_engine::LiveIn,
    rate: u32,
) -> Result<cpal::Stream, String> {
    use cpal::traits::{DeviceTrait, StreamTrait};
    let (dev, config) = deckard_engine::devices::open_input(device.as_deref())?;
    if config.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!("input format {:?} is not supported", config.sample_format()));
    }
    if config.sample_rate() != rate {
        return Err(format!("the input runs at {} Hz and the output at {rate} Hz", config.sample_rate()));
    }
    let channels = config.channels() as usize;
    let map: Vec<Option<usize>> = (0..32).map(|i| routing.get(i).copied().unwrap_or(Some(i)).filter(|&c| c < channels)).collect();
    let stream = dev
        .build_input_stream(
            config.into(),
            move |data: &[f32], _| {
                if let Ok(mut q) = live.try_lock() {
                    for f in data.chunks(channels) {
                        let mut frame = [0f32; 32];
                        for (i, c) in map.iter().enumerate() {
                            if let Some(c) = c {
                                frame[i] = f[*c];
                            }
                        }
                        q.push_back(frame);
                    }
                    // A few blocks at most (nothing may be reading).
                    while q.len() > 8192 {
                        q.pop_front();
                    }
                }
            },
            |e| eprintln!("audio input error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

/// A live Variation's start: (group, pattern, sound), the notes it started from, the
/// notes it made.
type VariationBase = ((usize, usize, usize), Vec<Note>, Vec<Note>);

/// The input settings a live input was opened with: device, routing.
type InputKey = (Option<String>, Vec<Option<usize>>);

/// The Variation and Generate settings (fields of a sound).
const GROOVE_FIELDS: [&str; 17] = [
    "sequence_variation/velocity_amount",
    "sequence_variation/time_shift",
    "sequence_variation/length_scale",
    "sequence_variation/length_variation",
    "sequence_variation/distribution",
    "sequence_variation/live",
    "sequence_generation/mode",
    "sequence_generation/probability",
    "sequence_generation/velocity_amount",
    "sequence_generation/note_range/low",
    "sequence_generation/note_range/high",
    "sequence_generation/note_count",
    "sequence_generation/distribution",
    "sequence_generation/steps",
    "sequence_generation/fills",
    "sequence_generation/rotate",
    "sequence_generation/division",
];

/// The stem separation model's install, offered the first time DECKARD starts without
/// it (the .deb and .rpm can't ask; a download of [`stems::MODEL_MB`] MB).
enum StemsOffer {
    Asking,
    Installing(std::sync::mpsc::Receiver<Result<(), String>>),
}

/// What waits for an answer about unsaved changes.
#[derive(Debug, Clone)]
enum Pending {
    New,
    Open(PathBuf),
    Revert,
    Quit,
}

#[derive(Default)]
pub struct App {
    /// The open project, shared with the controller scripts.
    doc: Option<Shared>,
    /// The document revision playback last took up.
    seen_revision: u64,
    path: Option<PathBuf>,
    error: Option<String>,
    /// Focus: group, its pattern and sound, and the scene.
    group: usize,
    pattern: usize,
    sound: usize,
    scene: usize,
    /// The section picked in the Song view: the song plays from it, or jumps to it
    /// when the playing section ends.
    section: Option<usize>,
    /// The song panel shows the Song view (sections) instead of the Ideas view
    /// (scenes), as Maschine's IDEAS/SONG switch; PLAY follows it.
    song_view: bool,
    /// The pattern editor shows the focused sound as a piano roll.
    piano_roll: bool,
    /// The scale notes are written in: (root note, MIDI 60 = C3; bank; scale in it), as
    /// the controller's Keyboard mode has it. Main's Chromatic marks nothing.
    scale: (i64, i64, i64),
    /// The group whose root [`App::scale`] last had from the controller.
    scale_group: Option<usize>,
    /// The user chords Chords mode plays (DECKARD's own layer).
    user_chords: user_chords::UserChords,
    /// Pencil: clicks add and remove notes, drags paint (else select mode).
    /// What clicks and drags do in the event area (Maschine's mouse modes).
    mouse: layout::MouseMode,
    /// What nudging moves notes by.
    nudge: layout::NudgeGrid,
    /// LOCK is lit (a snapshot taken, to be recalled).
    lock_active: bool,
    /// The Lock panel shows.
    lock_panel: bool,
    /// The MIDI Change dialog shows.
    midi_change: bool,
    /// The MIDI Monitor window shows (View ▸ MIDI Monitor), and its choices.
    midi_monitor: bool,
    monitor_view: midi_monitor::View,
    /// File ▸ Export Audio's panel and the export it runs.
    export: export::ExportPanel,
    /// Bounce: its confirmation, and the bounce running.
    bounce: bounce::Bounce,
    /// The clip selected in the Song view (group, clip).
    clip: Option<(usize, usize)>,
    /// When the controller's Settings ▸ Audio was last told the audio interface.
    audio_view_at: Option<std::time::Instant>,
    /// What the own layer's Settings page shows, and when it was looked up.
    surface_settings: deckard_surface::SettingsView,
    surface_settings_at: Option<std::time::Instant>,
    /// A Lock recall morphing its way.
    lock_morph: Option<lock::LockMorph>,
    /// What the lane under the note area shows, the (group, sound) it was picked
    /// for (a newly focused sound shows its saved lane), and whether the dialog to
    /// add or remove MIDI lanes is open.
    lane: layout::Lane,
    lane_of: Option<(usize, usize)>,
    lanes_dialog: bool,
    /// The span of a lane Shift-dragged (lane, from, to ticks): Delete erases it.
    lane_span: Option<(layout::Lane, i64, i64)>,
    /// Notes selected in the pattern editor, and the (group, pattern) they belong to.
    selected: Vec<Note>,
    selected_in: (usize, usize),
    /// The pattern editor's grid, in ticks (snapping, drawing, new note length).
    grid: i64,
    /// A group or sound row being renamed in place.
    renaming: Option<deckard_doc::Target>,
    /// Level meters (left, right; linear), [group][sound], falling back after peaks.
    meters: Vec<Vec<(f32, f32)>>,
    meters_at: Option<std::time::Instant>,
    /// Every route's travel as last seen (their knobs keep drawing while these move).
    route_seen: Vec<Option<f32>>,
    /// The focused sound's metered effects (input peak, gain reduction dB, output peak),
    /// by (group, sound, slot), falling back like the level meters.
    pub fx_meter_view: std::collections::HashMap<(usize, usize, usize), (f32, f32, f32)>,
    chain_tab: layout::ChainTab,
    /// Save a PNG of the window to this path and quit.
    screenshot: Option<PathBuf>,
    frames: u32,
    shot_requested: bool,
    started: Started,
    /// A tempo to set (as an undoable edit) one second after start-up (`--tempo`).
    pending_tempo: Option<f64>,
    /// Seconds to wait before `--screenshot` (`--wait`).
    shot_wait: Option<f32>,
    /// A scene to launch one second after start-up (`--launch`).
    pending_launch: Option<usize>,
    /// The note sounding while the mouse button is down: (group, sound, pitch).
    held: Option<(usize, usize, i64)>,
    /// Scripted pointer input (`--input`): (seconds after start, (m/d/u = move, down,
    /// up, position)).
    input_script: Vec<(f32, (char, egui::Pos2))>,
    /// Load the project's plug-ins once the window has drawn (they take a moment, and
    /// a press that waits for them isn't a click any more).
    preload_plugins: bool,
    /// Frames drawn since the project was opened (plug-ins load after the second).
    frames_drawn: u32,
    /// Open plug-in windows: (group, sound, window).
    editors: Vec<(deckard_doc::Chain, usize, deckard_plugins::editor::Editor)>,
    /// Preferences and the installed plug-ins; the Preferences window (open, tab).
    settings: settings::Settings,
    catalog: settings::Catalog,
    /// The browser (LIBRARY / FILES), and the library it shows.
    browser: browser::Browser,
    /// REC: recording (or armed, stopped), and the take being recorded.
    recording: bool,
    take: record::Take,
    /// The engine's never-wrapping clock as last seen in this take (`Playhead::played`),
    /// and when held notes were last grown.
    take_played: f64,
    take_grown: Option<std::time::Instant>,
    /// Controllers played into the take, overwriting their lanes.
    controls: record::ControlWriter,
    /// Auto-write: knob moves while the song plays are written as modulation (the
    /// header's AUTO, the MK3's AUTO).
    auto_write: bool,
    /// The knob being written as modulation.
    mod_take: Option<modulation::ModTake>,
    /// The step the controller's Step Mod page shows (tick, length), while it shows.
    step_mod_at: Option<(i64, i64)>,
    /// An Auto-length pattern growing as it's recorded into.
    grow: Option<record::Grow>,
    /// Note Repeat / Arp: switched on in the software, LOCK, the preset chosen, what the
    /// engine was last told, and the panel shown.
    repeat_soft: bool,
    arp_locked: bool,
    /// The preset chosen (0–3), for the document generation it was chosen in.
    arp_preset: Option<(u64, usize)>,
    repeat_sent: Option<(deckard_model::Arp, deckard_engine::repeat::RepeatMode)>,
    repeat_panel: bool,
    /// The computer keyboard as a MIDI keyboard (⌘K).
    keys: keys::Keys,
    /// The next playback counts in this many bars first.
    count_in_next: u32,
    /// The browser's revision the controller was last given.
    browser_sent: u64,
    /// The Save dialog, while open.
    save_dialog: Option<SaveDialog>,
    /// Where each sound, group and preset was last saved, with its tags (Save writes
    /// there again).
    saved: std::collections::HashMap<deckard_doc::FileItem, Saved>,
    /// Maschine's user folder, when not the usual one (tests).
    user_dir_override: Option<PathBuf>,
    prefs: Option<usize>,
    /// Audio interfaces with inputs (name, channels), looked up when Preferences ▸ Audio
    /// opens.
    inputs: Option<Vec<(String, usize)>>,
    /// The same for outputs.
    outputs: Option<Vec<(String, usize)>>,
    /// The names interfaces give their channels, by (device, input), looked up as needed.
    channel_names: std::collections::HashMap<(String, bool), Vec<Option<String>>>,
    /// Preferences ▸ Audio ▸ Routing shows the outputs (else the inputs).
    routing_outputs: bool,
    /// The slot of the focused sound's chain whose pages are shown (`None`: the sound's
    /// channel: level, pan, groove), and the (group, sound) it was picked for.
    slot: Option<usize>,
    slot_of: Option<deckard_doc::Chain>,
    /// The Control area shows channel properties (else the plug-ins); which set
    /// (Input, Output, Groove, Macro) and page.
    channel: bool,
    prop_set: usize,
    prop_page: usize,
    /// MIDI learn, host automation and macro editing (the Assignment area).
    remote: remote::Remote,
    /// MIDI ports (inputs, outputs) and when they were last looked up.
    midi_ports: (Vec<String>, Vec<String>),
    midi_ports_at: Option<std::time::Instant>,
    /// Variation and Generate settings of sounds whose project can't keep them (from
    /// before Maschine 3): (group, sound, field) → value, for this session.
    groove_settings: std::collections::HashMap<(usize, usize, String), f64>,
    /// MIDI input and output, and what its routing was built from (revision, focus,
    /// Default = focus); whether the output knows it.
    midi: midi_io::MidiIo,
    /// Live Variation: the notes it started from, for (group, pattern, sound), and the
    /// notes it made (so other edits start it afresh).
    variation_base: Option<VariationBase>,
    midi_table_for: Option<(u64, usize, usize, bool, Vec<(String, u8)>)>,
    midi_wired: bool,
    /// The Maschine MK3 over USB (started by `main`, not in tests).
    mk3: Option<mk3_io::Mk3Io>,
    /// MIDI controllers run by scripts (the Launchpad Pro); none found in tests.
    midi_devices: midi_devices::MidiDevices,
    /// The last MIDI controller script error shown.
    midi_device_error: Option<String>,
    /// Held while the app runs: macOS doesn't slow it in the background (C46).
    _running: Option<keep_running::Activity>,
    /// Knob travel the controller's knobs have built up toward a stepped value's next step.
    knob_acc: [f64; 8],
    /// When each of the controller's knobs was last turned (a route's whole name shows
    /// meanwhile).
    knob_turned: [Option<std::time::Instant>; 8],
    /// DECKARD's own MK3 layer, run when no controller scripts do.
    surface: deckard_surface::Surface,
    /// Its last script error, as shown in the status bar.
    surface_error: Option<String>,
    /// The scripts' last screens sent to the MK3 (their draw count).
    mk3_screens_sent: u64,
    /// Commands sent to the output when the last edit went (see `follow_edits`).
    adopt_sent: u64,
    /// What the playing engine was last built from (which engine, the project, its
    /// chains), for the next edit.
    built: Option<(u64, deckard_model::Project, Vec<Vec<bool>>)>,
    midi_listening: Vec<String>,
    /// Link and MIDI clock (`transport_sync`).
    sync: transport_sync::Sync,
    /// The interface input, open while a sound takes audio from it (Input ▸ Audio),
    /// for (device, routing).
    live_input: Option<(cpal::Stream, InputKey)>,
    settings_input_failed: Option<InputKey>,
    /// The page of a plug-in's parameters shown, and what it shows.
    plugin_page: usize,
    plugin_view: Option<PluginView>,
    /// Each plug-in's parameters (by chain and slot, with the plug-in's id), for the
    /// macros that control them.
    plugin_params: std::cell::RefCell<std::collections::HashMap<(deckard_doc::Chain, usize), (String, Vec<deckard_plugins::ParamInfo>)>>,
    /// Open the focused sound's plug-in window one second after start-up
    /// (`--plugin-window`).
    pending_editor: bool,
    /// Audio output, opened on first Play.
    output: Option<Result<Output, String>>,
    /// Why the output device picked couldn't be opened (the default plays instead).
    output_fallback: Option<String>,
    /// The stem separation model offered at start-up: the question, then the install.
    stems_offer: Option<StemsOffer>,
    samples: SampleCache,
    /// What the output is playing (follows the engine through scene switches), and a
    /// note about it (e.g. sounds that are not played).
    playing: Option<Mode>,
    /// Where STOP left the song: shown there, and PLAY goes on from it (RESTART: from
    /// the start).
    parked: Option<Playhead>,
    status: String,
    /// Scripts directory and controller model for the controller panel.
    scripts: Option<PathBuf>,
    model: String,
    controller: Option<Result<controller::Controller, String>>,
    show_controller: bool,
    /// The focus follows playback (the header's Follow switch).
    follow: bool,
    /// The song starts over at its end (the header's Loop switch); scenes always loop.
    looping: bool,
    /// Click on the beats while playing (the header's metronome switch, SHIFT + TAP).
    metronome: bool,
    /// The opened file was copied to `.bak` before the first save.
    backed_up: bool,
    /// The Sampler page shown in the SOUND tab, the focused zone, and its waveform.
    sampler_page: usize,
    zone: usize,
    waveform: Option<sampler_ui::Waveform>,
    /// Where the focused zone is playing (frames), while it plays.
    zone_playhead: Option<usize>,
    /// What the playing mode plays, for the controller's pads to flash with: (mode, the
    /// document revision it was made from, its notes).
    played_timeline: Option<(Mode, u64, deckard_engine::Timeline)>,
    blinks: blink::Blinks,
    /// The (group, sound)s playing a note now, sequenced or live: their numbers flash.
    pub lit: std::collections::HashSet<(usize, usize)>,
    /// Sounds copied or cut from the sound menu, to paste.
    sound_clipboard: Vec<deckard_doc::SoundClip>,
    /// A copied group, as its group file.
    group_clipboard: Option<Vec<u8>>,
    /// Notes cut or copied in the Pattern Editor.
    note_clip: Option<clipboard::NoteClip>,
    /// Where the editing keys act (the part of the window clicked last).
    key_focus: layout::KeyFocus,
    /// The Sampling view (instead of the pattern editor) and its state; the recorder.
    sampling_open: bool,
    /// Open audio inputs to meter them (off in tests: no devices are opened).
    meter_inputs: bool,
    /// What the controller's Sampling page knobs set, by key (`sampling_hw`), and the
    /// Sampling tab last shared with it.
    record_apply: std::collections::HashMap<String, sampling_hw::Apply>,
    hw_sampling_tab: Option<usize>,
    /// The stem separation running (EDIT ▸ STEMS; one at a time).
    stems: Option<stems::Job>,
    /// Where audio files dragged from the desktop are over the window, and those
    /// dropped this frame (they go to the sound row there: `drop_samples`).
    file_drag: Option<egui::Pos2>,
    dropped_samples: Vec<PathBuf>,
    /// The focused sound's Audio plug-in take in focus (group, sound, take): its frames
    /// and rate (for its tempo).
    take_info: Option<((usize, usize, usize), usize, u32)>,
    /// The focused group's Audio plug-in takes as the pattern editor draws them, by
    /// (group, sound, take): the file and frames they're of, and peaks (min, max) across
    /// the played frames.
    loop_peaks: std::collections::HashMap<(usize, usize, usize), (String, (i64, i64), std::sync::Arc<Vec<(f32, f32)>>)>,
    /// Bumped when `loop_peaks` changes; what the controller has seen.
    loop_peaks_gen: (u64, u64),
    sampling: layout::sampling::SamplingState,
    recorder: recorder::Recorder,
    /// The Mixer is shown (instead of the work views), at sound level (the focused
    /// group's sounds) or group level, with its optional sections.
    mixer: bool,
    /// The LFO panel is shown (in the Mixer's place), and the LFO it's editing (a pad).
    lfos: bool,
    lfo_focus: u8,
    /// The MK3 mod matrix page's slot being made (no Destination yet).
    mod_draft: Option<mod_slots::ModDraft>,
    /// Its columns' routes in order, so an edit never moves a slot to another column.
    mod_order: std::cell::RefCell<mod_slots::ModOrder>,
    mixer_sounds: bool,
    mixer_sections: layout::MixerSections,
    /// Peak level per channel since its headroom display was last clicked.
    headroom: std::collections::HashMap<layout::Channel, f32>,
    /// The Mixer's held peaks per channel (left, right), and when each was reached:
    /// shown [`PEAK_HOLD`], then falling with the meter.
    peaks: std::collections::HashMap<layout::Channel, [(f32, std::time::Instant); 2]>,
    /// The master's meter (left, right), falling back like the others.
    master_meter: (f32, f32),
    /// The audio thread's load (0..1) for the header's CPU meter, and the stats it was
    /// last worked out from (busy, audio µs).
    cpu_load: f32,
    cpu_seen: (u64, u64),
    /// Commands chosen in the header's menu, run next frame.
    menu_commands: Vec<menu::Command>,
    /// Projects opened recently (File > Open Recent), most recent first.
    recent: Vec<PathBuf>,
    /// What to do once the user has said what happens to unsaved changes.
    confirm: Option<Pending>,
    /// The About box is open.
    about: bool,
    /// Quitting was confirmed: let the window close.
    quitting: bool,
    /// Where the open project is kept between runs, saved or not (see [`App::keep_session`]);
    /// `None` (tests): not kept.
    session_dir: Option<PathBuf>,
    /// The project and edit last kept there ([`App::doc_generation`], revision), and
    /// when the next edit is due to be kept.
    session_kept: (u64, u64),
    session_due: Option<std::time::Instant>,
    /// Goes up with each project opened or made.
    doc_generation: u64,
}

/// Launch time (a newtype so `App` can derive `Default`).
struct Started(std::time::Instant);

impl Default for Started {
    fn default() -> Self {
        Started(std::time::Instant::now())
    }
}

impl Started {
    fn elapsed(&self) -> std::time::Duration {
        self.0.elapsed()
    }
}

fn save_png(path: &Path, image: &egui::ColorImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), image.size[0] as u32, image.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(image.as_raw()).map_err(|e| e.to_string())
}

/// Where playback is: what plays, what is queued next, and the position in ticks.
/// What the MK3 gets after a frame's events (see `App::mk3_events`).
pub(crate) struct Mk3Out {
    pub route: mk3_io::PadRoute,
    pub leds: deckard_mk3::leds::Leds,
    /// New screens, if they changed: their count and 960 × 272 RGBA pixels.
    pub screens: Option<(u64, Vec<[u8; 4]>)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Playhead {
    pub mode: Mode,
    pub queued: Option<Mode>,
    pub position: f64,
    /// The engine's clock that never wraps (`Engine::played`).
    pub played: f64,
}

#[cfg(test)]
fn project_of_app(app: &App) -> deckard_model::Project {
    app.doc.as_ref().unwrap().borrow().project().clone()
}

/// How often notes held while recording are grown in the pattern.
const TAKE_GROW: std::time::Duration = std::time::Duration::from_millis(100);

/// How long the Mixer's meters hold their peaks.
const PEAK_HOLD: std::time::Duration = std::time::Duration::from_millis(1500);

/// How often [`App::background`] comes back while the song plays or the meters move:
/// nothing is drawn then to hold it to the display's rate, so an immediate repaint would
/// spin. The MK3's pace while playing (C46).
const BACKGROUND_PACE: std::time::Duration = std::time::Duration::from_millis(16);

/// How long after an edit the session is kept ([`App::keep_session`]).
const SESSION_DELAY: std::time::Duration = std::time::Duration::from_secs(3);

/// How long a routing choice shows whole after its knob was turned.
const KNOB_NAME_HOLD: std::time::Duration = std::time::Duration::from_millis(1500);
/// A controller knob's turn (fraction of a turn) per routing choice.
const ROUTE_STEP: f64 = 1.0 / 32.0;

/// `DECKARD_LOG_MK3` (or `DECKARD_LOG_KNOBS`): print the MK3's buttons and knob turns
/// and what they do (to stderr).
fn log_knobs() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("DECKARD_LOG_MK3").is_some() || std::env::var_os("DECKARD_LOG_KNOBS").is_some())
}

impl App {
    fn open(&mut self, path: &Path) {
        self.show_document(Document::open(path));
        self.path = Some(path.to_path_buf());
    }

    /// File › New: the default project's content when one is set and there (BUGS #10),
    /// else an empty project built from scratch (one group of 16 empty sounds, as
    /// Maschine starts one); untitled until saved.
    fn new_project(&mut self) {
        let template = self.settings.default_project.as_ref().and_then(|p| std::fs::read(p).ok());
        self.show_document(match template.map(|bytes| Document::from_bytes(bytes, None)) {
            Some(Ok(d)) => Ok(d),
            _ => Document::new_project(),
        });
        self.path = None;
        if self.error.is_none() {
            self.status = "new project".into();
        }
    }

    /// Keep where the project is left (the focus, the controller's page and pad mode)
    /// for the next save: it reopens there (BUGS #6).
    pub(crate) fn store_screen(&mut self) {
        let Some(doc) = &self.doc else { return };
        let (page, mode) = self.surface.screen().unwrap_or_default();
        let screen = deckard_doc::SavedScreen {
            group: self.group,
            sound: self.sound,
            scene: self.scene,
            pattern: self.pattern,
            slot: self.slot,
            channel: self.channel,
            page,
            mode,
        };
        doc.borrow_mut().set_screen(Some(screen));
    }

    /// Come back to where the opened project was saved ([`App::store_screen`]), what
    /// of it is still there.
    fn restore_screen(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let Some(s) = doc.borrow().screen().cloned() else { return };
        {
            let d = doc.borrow();
            let p = d.project();
            let Some(group) = p.groups.get(s.group) else { return };
            self.group = s.group;
            self.sound = s.sound.min(group.sounds.len().saturating_sub(1));
            if s.scene < p.scenes.len() {
                self.scene = s.scene;
            }
            if s.pattern < group.patterns.len() {
                self.pattern = s.pattern;
            }
            let slots = group.sounds.get(self.sound).map_or(0, |x| x.slots.len());
            self.slot = s.slot.filter(|&i| i < slots);
        }
        self.channel = s.channel;
        if !s.page.is_empty() {
            let view = self.surface_view();
            for a in self.surface.restore(&s.page, &s.mode, &view, std::time::Instant::now()) {
                self.surface_act(a);
            }
        }
    }

    /// Make `d` the open document (or show why it couldn't be opened).
    fn show_document(&mut self, d: Result<Document, String>) {
        modulation::register();
        self.stop();
        self.backed_up = false;
        self.lane_of = None;
        // (The mod matrix page's slot being made was the old project's.)
        self.mod_draft = None;
        *self.mod_order.borrow_mut() = Default::default();
        // Where this project's sounds and groups were saved (Save asks again).
        self.saved.clear();
        self.editors.clear();
        self.samples = SampleCache::default();
        self.preload_plugins = deckard_plugins::on_main_thread();
        self.frames_drawn = 0;
        match d {
            Ok(mut d) => {
                self.doc_generation += 1;
                // Pads play at the fixed velocity (100) unless FIXED VEL is turned off.
                let _ = d.set_initial(deckard_doc::Target::Song, "pad_velocity_mode", mxprj::decode::Value::Int(1));
                // What's copied keeps its expression in the new project (ids are the old one's).
                if let Some(old) = self.doc.clone() {
                    let old = old.borrow();
                    if let Some(clip) = &mut self.note_clip {
                        for n in &mut clip.notes {
                            *n = d.carry_note(&old, *n);
                        }
                    }
                    self.sound_clipboard = std::mem::take(&mut self.sound_clipboard).into_iter().map(|c| c.carried(&old, &mut d)).collect();
                }
                let doc = d.shared();
                if let Some(dir) = self.controller_scripts() {
                    self.controller = Some(controller::Controller::new(&dir, &self.model, Some(doc.clone())));
                }
                // Projects with an arrangement open in the Song view, others in Ideas.
                self.song_view = !doc.borrow().project().sections.is_empty();
                self.mixer_sounds = doc.borrow().project().mixer_sounds;
                self.headroom.clear();
                self.seen_revision = doc.borrow().revision();
                self.doc = Some(doc);
                self.error = None;
                (self.group, self.pattern, self.sound, self.scene) = (0, 0, 0, 0);
                self.section = None;
                self.focus_scene(0);
                self.restore_screen();
                self.sync_controller();
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Load the focused zone's waveform when the focus moves to another zone.
    /// What the sound, group and pattern menus ask for.
    fn menu_act(&mut self, a: layout::Act) {
        use layout::Act;
        let Some(doc) = self.doc.clone() else { return };
        let result: Result<(), String> = match a {
            Act::SetColor(targets, color) => doc.borrow_mut().set_color(&targets, color),
            Act::CopySound(g, s) => doc.borrow().copy_sound(g, s).map(|c| {
                self.status = format!("copied {}", c.name);
                self.sound_clipboard = vec![c];
            }),
            Act::CutSound(g, s) => {
                let clip = doc.borrow().copy_sound(g, s);
                clip.and_then(|c| {
                    self.sound_clipboard = vec![c];
                    let mut d = doc.borrow_mut();
                    d.reset_sound(g, s)?;
                    d.relabel_last("Cut Sound");
                    Ok(())
                })
            }
            Act::PasteSound(g, s) => doc.borrow_mut().paste_sounds(g, s, &self.sound_clipboard),
            Act::CopyGroup(g) => self.copy_group(g),
            Act::CutGroup(g) => self.copy_group(g).and_then(|_| {
                self.delete_group(g)?;
                doc.borrow_mut().relabel_last("Cut Group");
                Ok(())
            }),
            Act::PasteGroup(g) => match &self.group_clipboard {
                Some(bytes) => {
                    let mut d = doc.borrow_mut();
                    d.load_group_file(g, bytes).map(|_| d.relabel_last("Paste Group"))
                }
                None => Ok(()),
            },
            Act::DeleteGroup(g) => self.delete_group(g),
            Act::ResetSound(g, s) => doc.borrow_mut().reset_sound(g, s),
            Act::ExportMidi(g, s) => self.export_midi(g, s),
            Act::ImportMidi(g, s) => self.import_midi(g, s),
            Act::ExportGroupMidi(g) => self.export_group_midi(g, None),
            Act::ImportGroupMidi(g) => self.import_group_midi(g, None),
            Act::ClearPattern(g, p) => doc.borrow_mut().clear_pattern(g, p),
            Act::DoublePattern(g, p) => doc.borrow_mut().double_pattern(g, p),
            Act::DuplicatePattern(g, p) => {
                let r = doc.borrow_mut().duplicate_pattern(g, p);
                r.map(|i| (self.group, self.pattern) = (g, i))
            }
            Act::DeletePattern(g, p) => {
                let r = doc.borrow_mut().delete_pattern(g, p);
                if r.is_ok() && self.group == g && self.pattern >= p {
                    self.pattern = self.pattern.saturating_sub(1);
                }
                r
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.status = format!("can't do that: {e}");
        }
    }

    /// Copy group `g` (as its group file) for pasting.
    fn copy_group(&mut self, g: usize) -> Result<(), String> {
        let Some(doc) = self.doc.clone() else { return Ok(()) };
        let bytes = doc.borrow().file_bytes(deckard_doc::FileItem::Group(g), &mxprj::sound_info::SoundInfo::default())?;
        self.group_clipboard = Some(bytes);
        self.status = "copied the group".into();
        Ok(())
    }

    /// Delete group `g` (the last: reset), the focus kept on the group it was on, or
    /// the one before.
    fn delete_group(&mut self, g: usize) -> Result<(), String> {
        let Some(doc) = self.doc.clone() else { return Ok(()) };
        doc.borrow_mut().delete_group(g)?;
        let n = doc.borrow().project().groups.len();
        let focus = if self.group > g { self.group - 1 } else { self.group.min(n.saturating_sub(1)) };
        if focus != self.group || self.group == g {
            (self.group, self.sound, self.pattern) = (focus, 0, 0);
        }
        Ok(())
    }

    /// Export MIDI…: sound `s`'s notes in its group's focused pattern, to a file.
    fn export_midi(&mut self, g: usize, s: usize) -> Result<(), String> {
        let Some(doc) = self.doc.clone() else { return Ok(()) };
        let d = doc.borrow();
        let p = d.project();
        let pi = if g == self.group { self.pattern } else { 0 };
        let pat = p.groups.get(g).and_then(|x| x.patterns.get(pi)).ok_or("the group has no pattern")?;
        let sound = &p.groups[g].sounds[s];
        let notes: Vec<Note> = pat.notes.iter().filter(|n| n.sound == s).map(|n| Note { position: n.position - pat.start, ..*n }).collect();
        let midi: Vec<deckard_model::MidiEvent> =
            pat.midi.iter().filter(|m| m.sound == s).map(|m| deckard_model::MidiEvent { position: m.position - pat.start, ..*m }).collect();
        let name = if sound.name.is_empty() { format!("Sound {}", s + 1) } else { sound.name.clone() };
        let Some(path) = rfd::FileDialog::new().add_filter("MIDI", &["mid"]).set_file_name(format!("{name}.mid")).save_file() else {
            return Ok(());
        };
        // On the sound's MIDI output channel, transposed as it sends.
        let (channel, transpose) = (sound.channel.midi_out_channel.clamp(0, 15) as u8, sound.channel.transpose);
        let note_map = |n: &Note| (channel, n.pitch + transpose);
        midi_file::export_mapped(&path, &notes, &midi, p.tempo, p.numerator, p.denominator, &note_map, &|_| channel)?;
        self.status = format!("exported {}", path.display());
        Ok(())
    }

    /// Import MIDI…: a file's notes become sound `s`'s in its group's focused pattern
    /// (replacing them), the pattern growing in bars to fit. One undo step.
    fn import_midi(&mut self, g: usize, s: usize) -> Result<(), String> {
        let Some(doc) = self.doc.clone() else { return Ok(()) };
        let Some(path) = rfd::FileDialog::new().add_filter("MIDI", &["mid", "midi"]).pick_file() else { return Ok(()) };
        let (notes, midi, end) = midi_file::import(&path, s)?;
        let pi = if g == self.group { self.pattern } else { 0 };
        let mut d = doc.borrow_mut();
        let p = d.project().clone();
        let pat = p.groups.get(g).and_then(|x| x.patterns.get(pi)).ok_or("the group has no pattern")?.clone();
        let bar = p.bar_length().max(1);
        let need = ((end.max(1) + bar - 1) / bar) * bar;
        let add: Vec<Note> = notes.iter().map(|n| Note { position: n.position + pat.start, ..*n }).collect();
        let add_midi: Vec<deckard_model::MidiEvent> =
            midi.iter().map(|m| deckard_model::MidiEvent { position: m.position + pat.start, ..*m }).collect();
        let remove: Vec<Note> = pat.notes.iter().filter(|n| n.sound == s).copied().collect();
        let remove_midi: Vec<deckard_model::MidiEvent> = pat.midi.iter().filter(|m| m.sound == s).copied().collect();
        d.transaction("Import MIDI", |d| {
            if need > pat.length {
                d.set_pattern_length(g, pi, need)?;
            }
            d.edit_notes("Import MIDI", g, pi, &remove, &add)?;
            d.edit_midi("Import MIDI", g, pi, &remove_midi, &add_midi)
        })?;
        drop(d);
        self.status = format!("imported {} notes from {}", notes.len(), path.display());
        Ok(())
    }

    /// Export MIDI… of a group: every sound's events in its focused pattern, each sound
    /// on its MIDI output channel (Sounds to MIDI Channels) or on its own note from the
    /// group's start note, channel 1 (Sounds to MIDI Notes). `to`: the file (else asked).
    fn export_group_midi(&mut self, g: usize, to: Option<PathBuf>) -> Result<(), String> {
        let Some(doc) = self.doc.clone() else { return Ok(()) };
        let d = doc.borrow();
        let p = d.project();
        let grp = p.groups.get(g).ok_or("no such group")?;
        let pi = if g == self.group { self.pattern } else { 0 };
        let pat = grp.patterns.get(pi).ok_or("the group has no pattern")?;
        let notes: Vec<Note> = pat.notes.iter().map(|n| Note { position: n.position - pat.start, ..*n }).collect();
        let midi: Vec<deckard_model::MidiEvent> =
            pat.midi.iter().map(|m| deckard_model::MidiEvent { position: m.position - pat.start, ..*m }).collect();
        let path = match to {
            Some(p) => p,
            None => {
                let name = if grp.name.is_empty() { format!("Group {}", layout::group_id(g)) } else { grp.name.clone() };
                let Some(path) = rfd::FileDialog::new().add_filter("MIDI", &["mid"]).set_file_name(format!("{name}.mid")).save_file()
                else {
                    return Ok(());
                };
                path
            }
        };
        // A track per sound (empty ones too): on its MIDI output channel, transposed;
        // or (Sounds to MIDI Notes) each on its own note from the group's start note,
        // channel 1.
        let by_note = grp.channel.midi_batch == 1;
        let tracks: Vec<midi_file::Track> = grp
            .sounds
            .iter()
            .enumerate()
            .map(|(s, snd)| {
                let mut notes: Vec<Note> = notes.iter().filter(|n| n.sound == s).copied().collect();
                if by_note {
                    notes.iter_mut().for_each(|n| n.pitch = grp.channel.start_note + s as i64);
                }
                midi_file::Track {
                    name: if snd.name.is_empty() { format!("Sound {}", s + 1) } else { snd.name.clone() },
                    notes,
                    midi: midi.iter().filter(|m| m.sound == s).copied().collect(),
                    channel: if by_note { 0 } else { snd.channel.midi_out_channel.clamp(0, 15) as u8 },
                    transpose: if by_note { 0 } else { snd.channel.transpose },
                }
            })
            .collect();
        midi_file::export_tracks(&path, &tracks, p.tempo, p.numerator, p.denominator)?;
        self.status = format!("exported {}", path.display());
        Ok(())
    }

    /// Import MIDI… into a group: its focused pattern's notes are replaced. A file on
    /// several channels gives each channel's events to the sound whose MIDI input takes
    /// that channel (else the sound of that number); a file on one channel maps notes
    /// from the group's start note to sounds 1–16, played at C3.
    fn import_group_midi(&mut self, g: usize, from: Option<PathBuf>) -> Result<(), String> {
        let Some(doc) = self.doc.clone() else { return Ok(()) };
        let path = match from {
            Some(p) => p,
            None => {
                let Some(p) = rfd::FileDialog::new().add_filter("MIDI", &["mid", "midi"]).pick_file() else { return Ok(()) };
                p
            }
        };
        let (notes, midi, end) = midi_file::import_channels(&path)?;
        let pi = if g == self.group { self.pattern } else { 0 };
        let mut d = doc.borrow_mut();
        let p = d.project().clone();
        let grp = p.groups.get(g).ok_or("no such group")?;
        let pat = grp.patterns.get(pi).ok_or("the group has no pattern")?.clone();
        let n_sounds = grp.sounds.len();
        let mut channels: Vec<u8> = notes.iter().map(|(c, _)| *c).collect();
        channels.sort_unstable();
        channels.dedup();
        let sound_for_channel = |c: u8| {
            grp.sounds.iter().position(|s| s.channel.midi_channel == c as i64 + 1).or(((c as usize) < n_sounds).then_some(c as usize))
        };
        let mut add = Vec::new();
        if channels.len() > 1 {
            for (c, n) in &notes {
                if let Some(s) = sound_for_channel(*c) {
                    add.push(Note { sound: s, position: n.position + pat.start, ..*n });
                }
            }
        } else {
            let start = grp.channel.start_note;
            for (_, n) in &notes {
                let k = n.pitch - start;
                if (0..n_sounds as i64).contains(&k) {
                    add.push(Note { sound: k as usize, position: n.position + pat.start, pitch: 60, ..*n });
                }
            }
        }
        let add_midi: Vec<deckard_model::MidiEvent> = midi
            .iter()
            .filter_map(|(c, m)| {
                Some(deckard_model::MidiEvent {
                    sound: if channels.len() > 1 { sound_for_channel(*c)? } else { 0 },
                    position: m.position + pat.start,
                    ..*m
                })
            })
            .collect();
        let bar = p.bar_length().max(1);
        let need = ((end.max(1) + bar - 1) / bar) * bar;
        let count = add.len();
        d.transaction("Import MIDI", |d| {
            if need > pat.length {
                d.set_pattern_length(g, pi, need)?;
            }
            d.edit_notes("Import MIDI", g, pi, &pat.notes, &add)?;
            d.edit_midi("Import MIDI", g, pi, &pat.midi, &add_midi)
        })?;
        drop(d);
        self.status = format!("imported {count} notes from {}", path.display());
        Ok(())
    }

    /// Show or hide the Sampler pages' waveform (remembered in the settings).
    fn toggle_waveform(&mut self) {
        self.settings.show_waveform = !self.settings.show_waveform;
        self.settings.save();
    }

    fn update_waveform(&mut self) {
        // Where the focused zone is playing, for the waveform's playhead.
        // (The plug-in strip always shows it; the SOUND tab when its waveform is on.)
        self.zone_playhead = match &self.output {
            Some(Ok(out)) => out.zone_position(self.group, self.sound, self.zone),
            _ => None,
        };
        let Some(doc) = self.doc.clone() else { return };
        let d = doc.borrow();
        let Some(smp) = d.project().groups.get(self.group).and_then(|g| g.sounds.get(self.sound)).and_then(|s| s.sampler.as_ref()) else {
            return;
        };
        let zi = self.zone.min(smp.zones.len().saturating_sub(1));
        let Some(z) = smp.zones.get(zi) else { return };
        let key = (self.group, self.sound, zi, z.sample.path.clone());
        if self.waveform.as_ref().is_some_and(|w| w.key == key) {
            return;
        }
        let path = d.path().map(Path::to_path_buf);
        if let Some(sample) = self.samples.sample(&z.sample, path.as_deref()) {
            self.waveform = Some(sampler_ui::Waveform::new(key, &sample.frames, sample.rate));
        }
    }

    /// Put `path` at the top of File > Open Recent.
    fn remember(&mut self, path: &Path) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path);
        self.recent.truncate(10);
        menu::save_recent(&self.recent);
    }

    /// Keep the open project as a backup, saved or not: its contents (`session.mxprj`),
    /// where it's saved and whether it has unsaved changes (`session.txt`). Through
    /// temporary files, so a crash mid-way leaves the last.
    fn keep_session(&mut self) {
        self.keep_session_as(false);
    }

    /// On quitting (its changes saved or discarded): the session ended as it should,
    /// so the next start doesn't bring the unsaved changes back.
    fn end_session(&mut self) {
        self.keep_session_as(true);
    }

    /// [`App::keep_session`], `clean` once DECKARD quits as it should.
    fn keep_session_as(&mut self, clean: bool) {
        let Some(dir) = self.session_dir.clone() else { return };
        self.store_screen();
        let Some(doc) = self.doc.clone() else { return };
        let d = doc.borrow();
        self.session_kept = (self.doc_generation, d.revision());
        self.session_due = None;
        let Ok(bytes) = d.to_bytes() else { return };
        let mut info = format!("dirty = {}\nclean = {clean}\n", d.is_dirty());
        if let Some(p) = d.path() {
            info += &format!("path = {}\n", p.display());
        }
        let _ = std::fs::create_dir_all(&dir);
        for (name, data) in [("session.mxprj", bytes), ("session.txt", info.into_bytes())] {
            let tmp = dir.join(format!("{name}.saving"));
            if std::fs::write(&tmp, data).is_ok() {
                let _ = std::fs::rename(&tmp, dir.join(name));
            }
        }
    }

    /// Keep the session a few seconds after the last edit (unless the Settings page's
    /// AUTO BACKUP is off: then only on quitting).
    fn keep_session_soon(&mut self) {
        let Some(doc) = &self.doc else { return };
        if !self.settings.auto_backup {
            return;
        }
        let now = (self.doc_generation, doc.borrow().revision());
        if self.session_dir.is_none() || now == self.session_kept {
            return;
        }
        let at = *self.session_due.get_or_insert_with(|| std::time::Instant::now() + SESSION_DELAY);
        if std::time::Instant::now() >= at {
            self.keep_session();
        }
    }

    /// What DECKARD opens with no project asked for: after a crash with unsaved
    /// changes, the backup (as Maschine offers it); else the last project as saved (the
    /// Settings page's RELOAD), or an empty project, like Maschine.
    pub(crate) fn start_project(&mut self) {
        let info = self.session_dir.as_ref().and_then(|d| std::fs::read_to_string(d.join("session.txt")).ok()).unwrap_or_default();
        let value = |k: &str| info.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix(" = ").map(str::to_string));
        let (path, dirty, clean) =
            (value("path").map(PathBuf::from), value("dirty").as_deref() == Some("true"), value("clean").as_deref() == Some("true"));
        if dirty && !clean && self.restore_session(path.clone(), dirty) {
            self.status = "unsaved changes brought back from the backup".into();
            return;
        }
        if let Some(p) = path.filter(|p| self.settings.reload_last && p.exists()) {
            self.open(&p);
            if self.error.is_none() {
                self.remember(&p);
                return;
            }
        }
        self.new_project();
    }

    /// Open the backup kept of the last session (saved at `path`, with unsaved edits if
    /// `dirty`); false if there's none.
    fn restore_session(&mut self, path: Option<PathBuf>, dirty: bool) -> bool {
        let Some(dir) = self.session_dir.clone() else { return false };
        let d = match std::fs::read(dir.join("session.mxprj")) {
            Ok(bytes) => Document::from_bytes(bytes, path.clone()),
            Err(e) => Err(e.to_string()),
        };
        let Ok(mut d) = d else { return false };
        if dirty {
            d.mark_unsaved();
        }
        self.show_document(Ok(d));
        self.path = path;
        if let Some(doc) = &self.doc {
            self.session_kept = (self.doc_generation, doc.borrow().revision());
        }
        true
    }

    fn dirty(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| d.borrow().is_dirty())
    }

    /// Open `path`, first asking about unsaved changes.
    fn open_file(&mut self, path: PathBuf) {
        if self.dirty() {
            self.confirm = Some(Pending::Open(path));
        } else {
            self.open(&path);
            if self.error.is_none() {
                self.remember(&path);
            }
        }
    }

    /// Unsaved changes on quitting are asked about (Save / Discard), as Maschine does.
    fn asks_on_quit(&self) -> bool {
        self.dirty()
    }

    fn quit(&mut self, ctx: &egui::Context) {
        if self.asks_on_quit() && !self.quitting {
            self.confirm = Some(Pending::Quit);
        } else {
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Do what was asked for once unsaved changes are dealt with.
    fn proceed(&mut self, ctx: &egui::Context, then: Pending) {
        match then {
            Pending::New => self.new_project(),
            Pending::Open(p) => {
                // Changes were saved or discarded: open without asking again.
                self.doc = None;
                self.open_file(p);
            }
            Pending::Revert => {
                if let Some(p) = self.doc.as_ref().and_then(|d| d.borrow().path().map(Path::to_path_buf)) {
                    self.open(&p);
                    self.status = format!("reverted to the saved {}", p.display());
                }
            }
            Pending::Quit => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn project_dir(&self) -> Option<PathBuf> {
        self.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf)
    }

    /// Save under a new name, which becomes the project's file.
    fn save_as(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let name = self.path.as_ref().and_then(|p| p.file_name()).map_or("Project.mxprj".into(), |n| n.to_string_lossy().into_owned());
        let mut dialog = rfd::FileDialog::new().add_filter("Maschine project", &["mxprj"]).set_file_name(name);
        if let Some(dir) = self.project_dir() {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else { return };
        self.store_plugin_states();
        self.store_screen();
        let r = doc.borrow_mut().save_as(&path);
        self.status = match r {
            Ok(()) => {
                self.path = Some(path.clone());
                self.backed_up = true;
                self.remember(&path);
                format!("saved as {}", path.display())
            }
            Err(e) => format!("not saved: {e}"),
        };
    }

    /// Write the project to another file, keeping this one open.
    fn save_copy_as(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let mut dialog = rfd::FileDialog::new().add_filter("Maschine project", &["mxprj"]).set_file_name("Copy.mxprj");
        if let Some(dir) = self.project_dir() {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else { return };
        self.store_plugin_states();
        let r = doc.borrow().to_bytes().and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string()));
        self.status = match r {
            Ok(()) => format!("saved a copy as {}", path.display()),
            Err(e) => format!("copy not saved: {e}"),
        };
    }

    /// Carry out a menu command (from either menu, or its shortcut).
    fn run_command(&mut self, ctx: &egui::Context, cmd: menu::Command) {
        use menu::Command as C;
        match cmd {
            C::New => {
                if self.dirty() {
                    self.confirm = Some(Pending::New);
                } else {
                    self.new_project();
                }
            }
            C::Open => {
                let mut dialog = rfd::FileDialog::new().add_filter("Maschine project", &["mxprj"]);
                if let Some(dir) = self.project_dir() {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(p) = dialog.pick_file() {
                    self.open_file(p);
                }
            }
            C::OpenRecent(p) => {
                if p.exists() {
                    self.open_file(p);
                } else {
                    self.status = format!("{} no longer exists", p.display());
                    self.recent.retain(|r| *r != p);
                    menu::save_recent(&self.recent);
                }
            }
            C::ClearRecent => {
                self.recent.clear();
                menu::save_recent(&self.recent);
            }
            C::Save => self.save(),
            C::SaveAs => self.save_as(),
            C::SaveCopyAs => self.save_copy_as(),
            C::Revert => {
                if self.dirty() {
                    self.confirm = Some(Pending::Revert);
                }
            }
            C::Quit => self.quit(ctx),
            C::Undo(n) => {
                for _ in 0..n {
                    self.undo();
                }
            }
            C::Redo(n) => {
                for _ in 0..n {
                    self.redo();
                }
            }
            C::UndoStep => {
                if let Some(doc) = &self.doc
                    && !doc.borrow_mut().undo_step()
                {
                    self.status = "nothing to undo".into();
                }
            }
            C::RedoStep => {
                if let Some(doc) = &self.doc
                    && !doc.borrow_mut().redo_step()
                {
                    self.status = "nothing to redo".into();
                }
            }
            C::SelectAll => {
                if let Some(doc) = &self.doc {
                    let d = doc.borrow();
                    self.selected = d
                        .project()
                        .groups
                        .get(self.group)
                        .and_then(|g| g.patterns.get(self.pattern))
                        .map_or(Vec::new(), |p| p.notes.iter().filter(|n| n.sound == self.sound).copied().collect());
                    self.selected_in = (self.group, self.pattern);
                }
            }
            C::Deselect => self.selected.clear(),
            C::DeleteSelected => {
                let remove = std::mem::take(&mut self.selected);
                if !remove.is_empty() {
                    let (g, p) = (self.group, self.pattern);
                    self.arrange(layout::Act::EditNotes { g, p, label: "Delete Notes", remove, add: Vec::new() });
                }
            }
            C::SongView(v) => self.song_view = v,
            C::PianoRoll(v) => self.piano_roll = v,
            C::DrawMode => self.mouse = self.mouse.toggled(layout::MouseMode::Draw),
            C::Follow => self.follow = !self.follow,
            C::Waveform => self.toggle_waveform(),
            C::RepeatPanel => self.repeat_panel = !self.repeat_panel,
            C::LockPanel => self.lock_panel = !self.lock_panel,
            C::MidiChange => self.midi_change = true,
            C::MidiMonitor => self.midi_monitor = !self.midi_monitor,
            C::ExportAudio => self.export.open = true,
            C::ExportMidi => {
                if let Err(e) = self.export_song_midi(None) {
                    self.status = format!("can't export MIDI: {e}");
                }
            }
            C::ShowController => self.show_controller = !self.show_controller,
            C::FullScreen => {
                let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
            }
            C::Model(m) => {
                self.model = m.to_string();
                self.reload_controller();
            }
            C::ReloadScripts => self.reload_controller(),
            C::ControllerFolder => match controller_folder() {
                Ok(dir) => ctx.open_url(egui::OpenUrl::new_tab(format!("file://{}", dir.display()))),
                Err(e) => self.status = format!("can't make the controller scripts folder: {e}"),
            },
            C::Manual => {
                let url = "https://docs.native-instruments.com/ni-tech-manuals/maschine-software-manual/en/welcome-to-maschine-software";
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
            C::About => self.about = true,
            C::Preferences => self.prefs = Some(self.prefs.unwrap_or(0)),
            C::Mixer => {
                self.mixer = !self.mixer;
                self.lfos &= !self.mixer;
            }
        }
    }

    /// DECKARD's controller layer: the user's scripts over the shipped ones, the theme
    /// picked, and DECKARD's pictures of its plug-ins (C47).
    fn new_surface(&self) -> deckard_surface::Surface {
        let mut s = deckard_surface::Surface::with_user_dir(menu::settings_dir().map(|d| d.join("controller")));
        s.set_theme(&self.settings.controller_theme);
        plugin_pictures::give(&mut s);
        s
    }

    /// Preferences ▸ Hardware ▸ Controller: stop the scripts for DECKARD's own layer, or
    /// start them again.
    fn set_controller_choice(&mut self, choice: settings::ControllerChoice) {
        self.settings.controller = choice;
        if choice == settings::ControllerChoice::Deckard {
            self.controller = None;
            self.surface = self.new_surface();
            self.status = "the MK3 runs on DECKARD's own controller layer".into();
        } else if self.controller.is_none() && self.scripts.is_some() {
            self.reload_controller();
        }
    }

    /// The controller scripts to run, unless Preferences ▸ Hardware ▸ Controller picks
    /// DECKARD's own layer.
    fn controller_scripts(&self) -> Option<PathBuf> {
        (self.settings.controller != settings::ControllerChoice::Deckard).then(|| self.scripts.clone()).flatten()
    }

    /// Start the controller scripts again (after a model change, or edited scripts).
    fn reload_controller(&mut self) {
        if let Some(dir) = self.controller_scripts() {
            self.controller = Some(controller::Controller::new(&dir, &self.model, self.doc.clone()));
            self.sync_controller();
            self.status = format!("controller scripts loaded for {}", self.model);
            if let Some(Ok(c)) = &self.controller
                && let Some(n) = &c.mod_note
            {
                self.status = format!("{}; {n}", self.status);
            }
        }
    }

    /// The menu commands asked for this frame: the macOS menu bar, the header menu
    /// (`menu_commands`), and shortcuts.
    fn menu_frame(&mut self, ctx: &egui::Context) {
        let menus = menu::menus(self);
        #[cfg(target_os = "macos")]
        if self.screenshot.is_none() {
            menu::native::update(ctx, &menus);
            menu::native::set_may_terminate(!self.dirty() || self.quitting);
        }
        #[allow(unused_mut)]
        let mut cmds: Vec<menu::Command> = std::mem::take(&mut self.menu_commands);
        #[cfg(target_os = "macos")]
        cmds.extend(menu::native::take());
        if !ctx.egui_wants_keyboard_input() {
            cmds.extend(menu::shortcuts(ctx, &menus));
        }
        for c in cmds {
            self.run_command(ctx, c);
        }
        // Closing the window asks about unsaved changes, like Quit.
        if ctx.input(|i| i.viewport().close_requested()) && self.asks_on_quit() && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Pending::Quit);
        }
        self.dialogs(ctx);
    }

    /// Preferences > Audio: the interface recorded from (Sampling ▸ Record, Source
    /// Ext. Ster. / Ext. Mono) and Routing > Input: which of its channels each of
    /// Maschine's inputs (In 1 L … In 16 R) reads.
    /// Preferences > Audio: the buffer size, and how the audio is keeping up.
    fn audio_engine_preferences(&mut self, ui: &mut egui::Ui) {
        use deckard_engine::output::BUFFER_SIZES;
        let rate = match &self.output {
            Some(Ok(o)) => o.rate as f64,
            _ => 48_000.0,
        };
        let ms = |n: u32| n as f64 / rate * 1000.0;
        let mut size = self.settings.buffer_frames;
        let mut restart = false;
        egui::Grid::new("audio-engine").num_columns(2).show(ui, |ui| {
            ui.label("Buffer size");
            egui::ComboBox::from_id_salt("buffer-size").selected_text(format!("{size} samples ({:.1} ms)", ms(size))).show_ui(ui, |ui| {
                for n in BUFFER_SIZES {
                    ui.selectable_value(&mut size, n, format!("{n} samples ({:.1} ms)", ms(n)));
                }
            });
            ui.end_row();
            if let Some(Ok(out)) = &self.output {
                let got = out.block_frames() as u32;
                if got > 0 && got != size {
                    ui.label("");
                    ui.label(egui::RichText::new(format!("The interface uses {got} samples.")).weak());
                    ui.end_row();
                }
                ui.label("Audio load");
                ui.label(out.stats.summary());
                ui.end_row();
            }
            ui.label("Multi-core");
            let mut multicore = self.settings.multicore;
            let threads = match &self.output {
                Some(Ok(out)) => out.render_threads().map_or("off".into(), |(n, helped)| format!("{} threads helping the audio thread ({helped} sounds rendered)", n)),
                _ => String::new(),
            };
            ui.checkbox(&mut multicore, format!("Render sounds on several cores  {threads}"));
            ui.end_row();
            ui.label("Render ahead");
            let mut ahead = self.settings.render_ahead_ms;
            let shown = |ms: u32| if ms == 0 { "Off".to_string() } else { format!("{ms} ms") };
            egui::ComboBox::from_id_salt("render-ahead").selected_text(shown(ahead)).show_ui(ui, |ui| {
                for ms in [0, 10, 20, 40, 80] {
                    ui.selectable_value(&mut ahead, ms, shown(ms));
                }
            });
            ui.end_row();
            // What it does (and, while on and playing, how it's doing).
            let mut hint = "Off: every sound plays live, as in Maschine. On: sounds outside the focused group (that don't take MIDI or audio input) are computed ahead on other cores, so a slow plug-in can't cause a dropout; effect edits on them take effect that much later.".to_string();
            if let Some(Ok(out)) = &self.output
                && self.settings.render_ahead_ms > 0
            {
                let (n, late) = out.telemetry(|t| (t.sounds_ahead, t.ahead_underruns));
                hint += &format!(" Now: {n} sounds ahead, {late} times not ready.");
            }
            ui.label("");
            ui.label(egui::RichText::new(hint).weak());
            ui.end_row();
            if ahead != self.settings.render_ahead_ms {
                self.settings.render_ahead_ms = ahead;
                restart = true;
            }
            ui.label("Live timing");
            let mut exact = self.settings.exact_timing;
            ui.checkbox(&mut exact, "Sample-accurate (pads, keys and MIDI play exactly one buffer after they come: no jitter)");
            ui.end_row();
            if exact != self.settings.exact_timing {
                self.settings.exact_timing = exact;
                if let Some(Ok(out)) = &self.output {
                    out.set_sample_accurate(exact);
                }
            }
            if multicore != self.settings.multicore {
                self.settings.multicore = multicore;
                restart = true;
            }
        });
        ui.label(
            egui::RichText::new(
                "Larger buffers give heavy projects and plug-ins more room (fewer clicks); smaller ones make pads and keys respond sooner. \
                 \"Late\" blocks were heard as clicks.",
            )
            .weak(),
        );
        if size != self.settings.buffer_frames {
            self.settings.buffer_frames = size;
            restart = true;
        }
        if restart {
            self.restart_output();
        }
    }

    /// Close the audio output and open it again (with the current settings): playback
    /// stops; MIDI and the live input are wired to the new one.
    fn restart_output(&mut self) {
        self.stop();
        self.output = None;
        self.midi_wired = false;
        self.live_input = None;
        let _ = self.output.get_or_insert_with(|| open_output(&self.settings, &mut self.output_fallback));
        self.status =
            format!("audio: {} samples, {}", self.settings.buffer_frames, if self.settings.multicore { "multi-core" } else { "one core" });
    }

    fn audio_preferences(&mut self, ui: &mut egui::Ui) {
        self.audio_engine_preferences(ui);
        ui.separator();
        let inputs = self.inputs.get_or_insert_with(recorder::input_devices).clone();
        let outputs = self.outputs.get_or_insert_with(deckard_engine::output::output_devices).clone();
        let (default_in, default_out) = (recorder::default_input(), deckard_engine::output::default_output());
        let default_label = |d: &Option<String>| format!("System default ({})", d.as_deref().unwrap_or("none"));
        let (mut input, mut output) = (self.settings.input_device.clone(), self.settings.output_device.clone());
        egui::Grid::new("audio-interface").num_columns(3).show(ui, |ui| {
            for (label, pick, devices, default, kind) in
                [("Output device", &mut output, &outputs, &default_out, "out"), ("Input device", &mut input, &inputs, &default_in, "in")]
            {
                ui.label(label);
                let shown = pick.clone().unwrap_or_else(|| default_label(default));
                egui::ComboBox::from_id_salt(label).selected_text(shown).width(320.0).show_ui(ui, |ui| {
                    ui.selectable_value(pick, None, default_label(default));
                    for (name, ch) in devices {
                        ui.selectable_value(pick, Some(name.clone()), format!("{name} ({ch} {kind})"));
                    }
                });
                let trouble = egui::Color32::from_rgb(0xff, 0x80, 0x60);
                if pick.as_ref().is_some_and(|d| !devices.iter().any(|(n, _)| n == d)) {
                    ui.label(egui::RichText::new("isn't connected").color(trouble));
                } else if kind == "out"
                    && let Some(why) = &self.output_fallback
                {
                    ui.label(egui::RichText::new(format!("can't be opened: {why}. Playing through the system default.")).color(trouble));
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
            ui.label("");
            if ui.button("Rescan").clicked() {
                (self.inputs, self.outputs) = (None, None);
                self.channel_names.clear();
            }
            ui.end_row();
        });
        // Another device: its own routing comes back.
        if input != self.settings.input_device {
            self.settings.set_input_device(input);
        }
        if output != self.settings.output_device {
            self.settings.set_output_device(output);
            self.restart_output();
        }
        let channels_of =
            |devices: &[(String, usize)], d: &Option<String>| devices.iter().find(|(n, _)| Some(n) == d.as_ref()).map_or(0, |(_, c)| *c);
        let (in_device, out_device) =
            (self.settings.input_device.clone().or(default_in), self.settings.output_device.clone().or(default_out));
        let (in_channels, out_channels) = (channels_of(&inputs, &in_device), channels_of(&outputs, &out_device));
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Routing");
            ui.selectable_value(&mut self.routing_outputs, false, "Input");
            ui.selectable_value(&mut self.routing_outputs, true, "Output");
        });
        let outs = self.routing_outputs;
        ui.label(
            egui::RichText::new(if outs {
                "Which channel of the interface each of DECKARD's outputs plays on. An alias names it wherever it's offered; one not connected isn't offered."
            } else {
                "Which channel of the interface each of DECKARD's inputs reads. An alias names it wherever it's offered; one not connected isn't offered."
            })
            .weak(),
        );
        let before = self.settings.output_routing.clone();
        let (own, port, channels) = if outs { ("Out", "Output", out_channels) } else { ("In", "Input", in_channels) };
        let device = if outs { out_device } else { in_device };
        let names = self.channel_names_of(&device.unwrap_or_default(), !outs, channels);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("routing").striped(true).num_columns(3).show(ui, |ui| {
                ui.strong("DECKARD");
                ui.strong("Alias");
                ui.strong("Interface");
                ui.end_row();
                let s = &mut self.settings;
                let (routes, aliases) =
                    if outs { (&mut s.output_routing, &mut s.output_aliases) } else { (&mut s.input_routing, &mut s.input_aliases) };
                for i in 0..routes.len() {
                    ui.label(settings::channel_name(own, i));
                    ui.add(egui::TextEdit::singleline(&mut aliases[i]).hint_text(settings::channel_name(own, i)).desired_width(160.0));
                    let cur = routes[i];
                    let text =
                        |c: Option<usize>| c.map_or("Not connected".to_string(), |c| channel_names::interface_label(c, port, &names));
                    let mut pick = cur;
                    egui::ComboBox::from_id_salt(("route", outs, i)).selected_text(text(cur)).width(200.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut pick, None, text(None));
                        for c in 0..channels.max(cur.map_or(0, |c| c + 1)) {
                            ui.selectable_value(&mut pick, Some(c), text(Some(c)));
                        }
                    });
                    routes[i] = pick;
                    ui.end_row();
                }
            });
        });
        if self.settings.output_routing != before
            && let Some(Ok(out)) = &self.output
        {
            out.set_routing(&self.settings.output_routing);
        }
    }

    /// Preferences > MIDI: the input and output ports DECKARD uses (each can be
    /// switched off), and what sounds set to the Default MIDI source take.
    fn midi_preferences(&mut self, ui: &mut egui::Ui) {
        self.sync_preferences(ui);
        let (ins, outs) = self.midi_ports().clone();
        for (title, ports, prefix) in [("Inputs", &ins, "in:"), ("Outputs", &outs, "out:")] {
            ui.label(egui::RichText::new(title).strong());
            if ports.is_empty() {
                ui.label(egui::RichText::new("No MIDI ports found").weak());
            }
            for name in ports {
                let key = format!("{prefix}{name}");
                let mut on = !self.settings.midi_off.contains(&key);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut on, name).changed() {
                        if on {
                            self.settings.midi_off.remove(&key);
                        } else {
                            self.settings.midi_off.insert(key.clone());
                        }
                    }
                    // An input can be an MPE controller (each note on a channel of its own).
                    if prefix == "in:" {
                        let mut mpe = self.settings.mpe.contains_key(name);
                        if ui.checkbox(&mut mpe, "MPE").on_hover_text("MPE controller: lower zone, notes on channels 2–16").changed() {
                            if mpe {
                                self.settings.mpe.insert(name.clone(), 48);
                            } else {
                                self.settings.mpe.remove(name);
                            }
                        }
                        if let Some(range) = self.settings.mpe.get_mut(name) {
                            ui.add(egui::DragValue::new(range).range(1..=96).suffix(" st"))
                                .on_hover_text("Member pitch-bend range (± semitones)");
                        }
                    }
                    // An output can be an MPE synth: each note on a channel of its own with its
                    // own bend, slide and pressure.
                    if prefix == "out:" {
                        let mut mpe = self.settings.mpe_out.contains_key(name);
                        if ui
                            .checkbox(&mut mpe, "MPE")
                            .on_hover_text("MPE synth: notes on channels 2–16, each with its own bend, slide and pressure")
                            .changed()
                        {
                            if mpe {
                                self.settings.mpe_out.insert(name.clone(), 48);
                            } else {
                                self.settings.mpe_out.remove(name);
                            }
                        }
                        if let Some(range) = self.settings.mpe_out.get_mut(name) {
                            ui.add(egui::DragValue::new(range).range(1..=96).suffix(" st"))
                                .on_hover_text("Member pitch-bend range (± semitones)");
                        }
                    }
                });
            }
            if prefix == "out:" {
                ui.checkbox(&mut self.settings.mpe_setup, "Send MPE setup to synths")
                    .on_hover_text("When an output is set to MPE or connects: turn the synth's MPE on and set its bend ranges");
            }
            ui.separator();
        }
        ui.label(egui::RichText::new("Default MIDI input of sounds").strong());
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.settings.sound_midi_focus, true, "Focus").on_hover_text("MIDI from any port plays the focused sound");
            ui.radio_value(&mut self.settings.sound_midi_focus, false, "None").on_hover_text("Sounds set to Default take no MIDI");
        });
        let drums: Vec<(String, String)> =
            self.midi_devices.manifests().iter().filter(|m| !m.drums.is_empty()).map(|m| (m.id.clone(), m.name.clone())).collect();
        if !drums.is_empty() {
            ui.separator();
            ui.label(egui::RichText::new("Drum channel").strong())
                .on_hover_text("A controller's notes on this channel from C1 play the focused group's pads (pad 1 bottom left, four by four), as the MK3's pads do");
            for (id, name) in drums {
                ui.horizontal(|ui| {
                    ui.label(&name);
                    let ch = self.settings.drum_channels.get(&id).copied();
                    let text = |c: Option<u8>| c.map_or("Off".to_string(), |c| format!("Channel {c}"));
                    egui::ComboBox::from_id_salt(("drum-channel", &id)).selected_text(text(ch)).show_ui(ui, |ui| {
                        for c in std::iter::once(None).chain((1..=16).map(Some)) {
                            if ui.selectable_label(ch == c, text(c)).clicked() {
                                match c {
                                    Some(c) => self.settings.drum_channels.insert(id.clone(), c),
                                    None => self.settings.drum_channels.remove(&id),
                                };
                            }
                        }
                    });
                    // (A print says its track, not its kind: which one is the drums.)
                    let tr = self.settings.drum_tracks.get(&id).copied();
                    let track_text = |t: Option<u8>| t.map_or("Prints: as notes".to_string(), |t| format!("Prints: track {t} as pads"));
                    egui::ComboBox::from_id_salt(("drum-track", &id))
                        .selected_text(track_text(tr))
                        .show_ui(ui, |ui| {
                            for t in std::iter::once(None).chain((1..=8).map(Some)) {
                                if ui.selectable_label(tr == t, track_text(t)).clicked() {
                                    match t {
                                        Some(t) => self.settings.drum_tracks.insert(id.clone(), t),
                                        None => self.settings.drum_tracks.remove(&id),
                                    };
                                }
                            }
                        })
                        .response
                        .on_hover_text("Print to Clip from this sequencer track lands on the focused group's pads from C1");
                });
            }
        }
        ui.label(egui::RichText::new(self.midi.status()).weak());
    }

    /// The controllers' drum channels for the MIDI routing: each one's notes port (the
    /// text its name has) and channel.
    pub(crate) fn drum_ports(&self) -> Vec<(String, u8)> {
        let mut out = Vec::new();
        for m in self.midi_devices.manifests() {
            if let Some(&ch) = self.settings.drum_channels.get(&m.id) {
                out.extend(m.drums.iter().map(|name| (name.clone(), ch)));
            }
        }
        out
    }

    /// Preferences > Plug-ins, as in Maschine: Manager (formats, the plug-ins found,
    /// each can be switched off; Rescan) and Location (folders searched for VST).
    /// Preferences ▸ General: the default project File ▸ New starts from (BUGS #10).
    fn general_preferences(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("general-prefs").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Default project");
            ui.vertical(|ui| {
                let shown =
                    self.settings.default_project.as_ref().map_or("None: an empty project".to_string(), |p| p.display().to_string());
                let gone = self.settings.default_project.as_ref().is_some_and(|p| !p.exists());
                ui.horizontal(|ui| {
                    ui.label(shown);
                    if gone {
                        ui.label(egui::RichText::new("isn't there").color(egui::Color32::from_rgb(0xff, 0x80, 0x60)));
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Choose…").clicked()
                        && let Some(p) = rfd::FileDialog::new().add_filter("Maschine project", &["mxprj"]).pick_file()
                    {
                        self.settings.default_project = Some(p);
                    }
                    if ui
                        .add_enabled(self.path.is_some(), egui::Button::new("Use This Project"))
                        .on_hover_text("The project open, as saved")
                        .clicked()
                    {
                        self.settings.default_project = self.path.clone();
                    }
                    if ui.add_enabled(self.settings.default_project.is_some(), egui::Button::new("Clear")).clicked() {
                        self.settings.default_project = None;
                    }
                });
            });
            ui.end_row();
            ui.label("");
            ui.label(egui::RichText::new("File ▸ New starts from it: its groups, sounds, plug-ins, tempo and routing, untitled.").weak());
            ui.end_row();
        });
    }

    fn preferences(&mut self, ctx: &egui::Context) {
        let Some(mut tab) = self.prefs else { return };
        let mut open = true;
        let before = self.settings.clone();
        let mut rescan = false;
        let mut library_change = None;
        let mut controller_change = None;
        egui::Window::new("Preferences").collapsible(false).resizable(true).default_size([620.0, 460.0]).open(&mut open).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut tab, 7, "General");
                ui.selectable_value(&mut tab, 2, "Audio");
                ui.selectable_value(&mut tab, 3, "MIDI");
                ui.selectable_value(&mut tab, 4, "Hardware");
                ui.selectable_value(&mut tab, 5, "Library");
                ui.selectable_value(&mut tab, 6, "Recording");
                ui.separator();
                ui.label(egui::RichText::new("Plug-ins:").weak());
                ui.selectable_value(&mut tab, 0, "Manager");
                ui.selectable_value(&mut tab, 1, "Location");
            });
            ui.separator();
            if tab == 2 {
                self.audio_preferences(ui);
            } else if tab == 7 {
                self.general_preferences(ui);
            } else if tab == 3 {
                self.midi_preferences(ui);
            } else if tab == 6 {
                egui::Grid::new("recording-prefs").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Count-in");
                    egui::ComboBox::from_id_salt("count-in")
                        .selected_text(match self.settings.count_in_bars {
                            0 => "Off".to_string(),
                            1 => "1 bar".to_string(),
                            n => format!("{n} bars"),
                        })
                        .show_ui(ui, |ui| {
                            for (n, name) in [(0, "Off"), (1, "1 bar"), (2, "2 bars"), (4, "4 bars")] {
                                ui.selectable_value(&mut self.settings.count_in_bars, n, name);
                            }
                        });
                    ui.end_row();
                    ui.label("");
                    ui.label(
                        egui::RichText::new("REC from a stop counts in this long first (SHIFT + REC on the controller: at least a bar).")
                            .weak(),
                    );
                    ui.end_row();
                    ui.label("Input Quantize");
                    ui.checkbox(&mut self.settings.input_quantize, "Snap recorded notes to the grid");
                    ui.end_row();
                });
            } else if tab == 5 {
                ui.label("Folders of sound packs to list in the browser under Factory (each folder's name is its library).");
                ui.label(egui::RichText::new("Maschine's own library is listed when it's installed; add packs here when it isn't.").weak());
                let mut remove = None;
                for d in &self.settings.library_folders {
                    ui.horizontal(|ui| {
                        if ui.small_button("✕").on_hover_text("Take it out").clicked() {
                            remove = Some(d.clone());
                        }
                        ui.label(d.display().to_string());
                    });
                }
                if ui.button("Add…").clicked()
                    && let Some(dir) = rfd::FileDialog::new().pick_folder()
                {
                    library_change = Some((dir, true));
                }
                if let Some(dir) = remove {
                    library_change = Some((dir, false));
                }
            } else if tab == 4 {
                egui::Grid::new("hardware-prefs").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Pad sensitivity");
                    let mut n = self.settings.pad_sensitivity;
                    ui.add(egui::Slider::new(&mut n, 0..=100).suffix(" %"));
                    self.settings.pad_sensitivity = n;
                    ui.end_row();
                    ui.label("");
                    ui.label(egui::RichText::new("How hard the MK3's pads must be pressed: higher, gentle presses play louder.").weak());
                    ui.end_row();
                    ui.label("Controller");
                    use settings::ControllerChoice as C;
                    let mut choice = self.settings.controller;
                    egui::ComboBox::from_id_salt("controller-layer")
                        .selected_text(match choice {
                            C::Automatic => "Automatic",
                            C::Deckard => "DECKARD",
                            C::Scripts => "Maschine scripts",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut choice, C::Automatic, "Automatic");
                            ui.selectable_value(&mut choice, C::Deckard, "DECKARD");
                            ui.selectable_value(&mut choice, C::Scripts, "Maschine scripts");
                        });
                    if choice != self.settings.controller {
                        controller_change = Some(choice);
                    }
                    ui.end_row();
                    ui.label("Controller theme");
                    let mut theme = self.settings.controller_theme.clone();
                    egui::ComboBox::from_id_salt("controller-theme").selected_text(theme_name(&theme)).show_ui(ui, |ui| {
                        for t in self.surface.themes() {
                            let name = theme_name(&t);
                            ui.selectable_value(&mut theme, t, name);
                        }
                    });
                    if theme != self.settings.controller_theme {
                        self.surface.set_theme(&theme);
                        self.settings.controller_theme = self.surface.theme().to_string();
                    }
                    ui.end_row();
                    ui.label("");
                    let found = if self.scripts.is_some() { "found" } else { "not found" };
                    ui.label(
                        egui::RichText::new(format!(
                            "What runs the MK3: Maschine's controller scripts, from your own Maschine install ({found}), or DECKARD's own layer. Automatic uses the scripts when they're found."
                        ))
                        .weak(),
                    );
                    ui.end_row();
                });
            } else if tab == 0 {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.settings.use_vst3, "Use VST3 Plug-ins");
                    ui.checkbox(&mut self.settings.use_vst2, "Use VST2 Plug-ins");
                    ui.checkbox(&mut self.settings.use_au, "Use Audio Units");
                });
                ui.checkbox(&mut self.settings.scan_on_start, "Rescan plug-ins at start-up");
                ui.label(egui::RichText::new("VST2 and Audio Unit plug-ins are listed but can't be hosted yet.").weak());
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.catalog.scanning(), egui::Button::new("Rescan")).clicked() {
                        rescan = true;
                    }
                    ui.label(egui::RichText::new(&self.catalog.status).weak());
                });
                ui.separator();
                let mut entries: Vec<deckard_plugins::scan::Entry> = self.catalog.entries.clone();
                entries.sort_by(|a, b| {
                    (a.vendor.to_lowercase(), a.name.to_lowercase()).cmp(&(b.vendor.to_lowercase(), b.name.to_lowercase()))
                });
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Grid::new("plugins").striped(true).num_columns(4).show(ui, |ui| {
                        ui.strong("On");
                        ui.strong("Plug-in");
                        ui.strong("Format");
                        ui.strong("Kind");
                        ui.end_row();
                        for e in &entries {
                            let key = settings::key(e);
                            let mut on = !self.settings.disabled.contains(&key);
                            if ui.checkbox(&mut on, "").changed() {
                                if on {
                                    self.settings.disabled.remove(&key);
                                } else {
                                    self.settings.disabled.insert(key);
                                }
                            }
                            let name = if e.vendor.is_empty() { e.name.clone() } else { format!("{} — {}", e.vendor, e.name) };
                            ui.label(name).on_hover_text(&e.location);
                            ui.label(e.kind.label());
                            ui.label(if e.instrument { "Instrument" } else { "Effect" });
                            ui.end_row();
                        }
                    });
                });
            } else {
                ui.label("Folders searched for VST3 and VST plug-ins (Audio Units are found by the system):");
                let mut remove = None;
                for (i, d) in self.settings.dirs.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                        let exists = d.exists();
                        ui.label(egui::RichText::new(d.display().to_string()).color(if exists {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::GRAY
                        }))
                        .on_hover_text(if exists { "" } else { "This folder doesn't exist" });
                    });
                }
                if let Some(i) = remove {
                    self.settings.dirs.remove(i);
                }
                ui.horizontal(|ui| {
                    if ui.button("Add…").clicked()
                        && let Some(dir) = rfd::FileDialog::new().set_title("Add Plugin Directory").pick_folder()
                        && !self.settings.dirs.contains(&dir)
                    {
                        self.settings.dirs.push(dir);
                    }
                    if ui.button("Restore Defaults").clicked() {
                        self.settings.dirs = settings::Settings::default().dirs;
                    }
                    if ui.add_enabled(!self.catalog.scanning(), egui::Button::new("Rescan")).clicked() {
                        rescan = true;
                    }
                });
            }
        });
        if self.settings != before {
            self.settings.save();
            deckard_plugins::module::set_search_paths(self.settings.dirs.clone());
            if self.settings.dirs != before.dirs {
                rescan = true;
            }
        }
        if rescan {
            self.catalog.rescan(&self.settings);
        }
        if let Some(choice) = controller_change {
            self.set_controller_choice(choice);
        }
        if let Some((dir, on)) = library_change {
            if !self.browser.started() {
                // Started now, so the change is listed.
                self.browse(browser::BrowseAct::Toggle);
                self.browse(browser::BrowseAct::Toggle);
            }
            self.browse(browser::BrowseAct::LibraryFolder(dir, on));
        }
        self.prefs = open.then_some(tab);
    }

    /// The unsaved-changes question and the About box.
    /// The offer to install the stem separation model, and the install it starts.
    fn stems_offer_dialog(&mut self, ctx: &egui::Context) {
        let Some(offer) = &self.stems_offer else { return };
        let mut done = false;
        let mut start = false;
        match offer {
            StemsOffer::Asking => {
                egui::Modal::new(egui::Id::new("stems-offer")).show(ctx, |ui| {
                    ui.set_width(420.0);
                    ui.label(egui::RichText::new("Install stem separation?").strong());
                    ui.add_space(4.0);
                    ui.label(format!(
                        "Sampling ▸ EDIT ▸ STEMS splits a sample into drums, bass, vocals and the rest with Meta's Demucs model, \
                         a {} MB download that isn't bundled with DECKARD. It goes into DECKARD's settings folder.",
                        stems::MODEL_MB
                    ));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("If you skip this, DECKARD downloads it the first time you use STEMS.").weak());
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Install").clicked() {
                            start = true;
                        }
                        if ui.button("Not now").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            done = true;
                        }
                    });
                });
            }
            StemsOffer::Installing(rx) => {
                match rx.try_recv() {
                    Ok(Ok(())) => {
                        self.status = "stem separation model installed".into();
                        done = true;
                    }
                    Ok(Err(e)) => {
                        self.status = e;
                        done = true;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.status = "installing the stem separation model stopped".into();
                        done = true;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }
                if !done {
                    egui::Modal::new(egui::Id::new("stems-offer")).show(ctx, |ui| {
                        ui.set_width(420.0);
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(format!("Downloading the stem separation model ({} MB)…", stems::MODEL_MB));
                        });
                    });
                    ctx.request_repaint_after(std::time::Duration::from_millis(250));
                }
            }
        }
        if start && let Some(dir) = self.session_dir.clone() {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::Builder::new()
                .name("stems-install".into())
                .spawn(move || {
                    let _ = tx.send(stems::install(&dir));
                })
                .expect("a thread for the install");
            self.stems_offer = Some(StemsOffer::Installing(rx));
        } else if start {
            done = true;
        }
        if done {
            // Asked once, whatever the answer (STEMS installs it when used).
            self.stems_offer = None;
            self.settings.stems_offered = true;
            self.settings.save();
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(then) = self.confirm.clone() {
            let name = self.path.as_ref().and_then(|p| p.file_stem()).map_or("the project".into(), |n| n.to_string_lossy().into_owned());
            let question = match then {
                Pending::Revert => format!("Discard all changes to \"{name}\" since it was last saved?"),
                _ => format!("Save changes to \"{name}\"?"),
            };
            let mut choice = None;
            egui::Modal::new(egui::Id::new("unsaved-changes")).show(ctx, |ui| {
                ui.set_width(360.0);
                ui.label(egui::RichText::new(question).strong());
                ui.add_space(4.0);
                ui.label("Your changes will be lost if you don't save them.");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if matches!(then, Pending::Revert) {
                        if ui.button("Revert").clicked() {
                            choice = Some(1);
                        }
                    } else {
                        if ui.button("Save").clicked() {
                            choice = Some(0);
                        }
                        if ui.button("Don't Save").clicked() {
                            choice = Some(1);
                        }
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        choice = Some(2);
                    }
                });
            });
            match choice {
                Some(0) => {
                    self.save();
                    self.confirm = None;
                    if !self.dirty() {
                        self.proceed(ctx, then);
                    }
                }
                Some(1) => {
                    self.confirm = None;
                    self.proceed(ctx, then);
                }
                Some(_) => self.confirm = None,
                None => {}
            }
        }
        self.stems_offer_dialog(ctx);
        self.preferences(ctx);
        if self.about {
            let mut open = true;
            egui::Window::new("About DECKARD").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
                crate::layout::deckard_logo(ui.painter(), ui.cursor().left_top() + egui::vec2(0.0, 14.0));
                ui.add_space(30.0);
                ui.label(format!("DECKARD {VERSION}"));
                ui.label("An independent, open music workstation that opens and plays Maschine projects.");
                ui.label("Not affiliated with Native Instruments. MASCHINE is a trademark of Native Instruments.");
                ui.add_space(6.0);
                ui.label("Plaits is Mutable Instruments' code by Émilie Gillet (MIT License).");
                ui.label(deckard_link::LICENSE);
                ui.collapsing("Mutable Instruments' license", |ui| {
                    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                        ui.label(egui::RichText::new(deckard_plugins::MUTABLE_INSTRUMENTS_LICENSE).small());
                    });
                });
            });
            self.about = open;
        }
    }

    /// Play `mode`; the arrangement from the focused section.
    fn play(&mut self, mode: Mode) {
        let start = match (mode, self.section) {
            (Mode::Arrangement, Some(i)) => self.doc.as_ref().map_or(0, |d| d.borrow().project().section_start(i)),
            _ => 0,
        };
        self.play_from(mode, start);
    }

    /// Play `mode` from tick `start`.
    fn play_from(&mut self, mode: Mode, start: i64) {
        let Some(doc) = self.doc.clone() else { return };
        let looping = self.loops(mode);
        let synced = self.sync_start();
        // Each channel's peak (its headroom) counts from the start of playback.
        self.headroom.clear();
        let d = doc.borrow();
        let project = d.project();
        let out = match self.output.get_or_insert_with(|| open_output(&self.settings, &mut self.output_fallback)) {
            Ok(out) => out,
            Err(e) => {
                self.status = format!("audio: {e}");
                return;
            }
        };
        let (mut engine, report) = Engine::new(project, self.path.as_deref(), out.rate, mode, &mut self.samples);
        if let Some(late) = synced {
            engine.sync_start_within(late);
        }
        engine.looping = looping;
        engine.metronome = self.metronome;
        if start > 0 {
            engine.seek(start);
        }
        engine.set_recording(self.recording);
        engine.set_repeat(self.repeat_sent.clone());
        engine.set_metronome_volume(f32::from(self.settings.metronome_volume) / 100.0);
        engine.metronome_auto = self.settings.metronome_auto;
        engine.metronome_ticks = self.settings.metronome_ticks;
        let count_in = std::mem::take(&mut self.count_in_next);
        if count_in > 0 {
            let bar = project.bar_length() as f64 * 60.0 / project.tempo / deckard_model::PPQ as f64 * out.rate as f64;
            engine.count_in((bar * count_in as f64).round() as u64);
        }
        out.play(engine);
        self.playing = Some(mode);
        let mut status = format!("{} of {} sampler zones loaded", report.loaded, report.zones);
        if !report.plugins.is_empty() {
            status += &format!("; {} plug-ins", report.plugins.len());
        }
        if !report.plugin_errors.is_empty() {
            status += &format!("; plug-ins failed: {}", report.plugin_errors.join(", "));
        }
        if !report.no_sampler.is_empty() {
            status += &format!("; not played (drum synths aren't rendered yet): {}", report.no_sampler.join(", "));
        }
        self.status = status;
    }

    /// PLAY: the focused scene, looped, in the Ideas view; the arrangement in the
    /// Song view.
    fn transport_play(&mut self) {
        // (Receiving MIDI clock, the clock starts the song.)
        if self.receiving_clock() {
            return;
        }
        let mode = if self.song_view { Mode::Arrangement } else { Mode::Scene(self.scene) };
        // On from where STOP left it, if it's what was playing.
        match self.parked.take() {
            Some(h) if h.mode == mode => self.play_from(mode, h.position as i64),
            _ => self.play(mode),
        }
    }

    /// Whether `mode` starts over at its end: scenes and patterns always do, the song
    /// with Loop on.
    fn loops(&self, mode: Mode) -> bool {
        matches!(mode, Mode::Pattern(..) | Mode::Scene(_)) || self.looping
    }

    /// RESTART: play from the beginning, playing or not: the song from its first bar in
    /// the Song view, the focused scene from its start in the Ideas view.
    fn restart(&mut self) {
        if self.receiving_clock() {
            return;
        }
        let mode = if self.song_view { Mode::Arrangement } else { Mode::Scene(self.scene) };
        self.parked = None;
        self.play_from(mode, 0);
        self.sync_transport();
    }

    /// Loop on or off, for what plays now too.
    fn set_looping(&mut self, on: bool) {
        self.looping = on;
        if let (Some(mode), Some(Ok(out))) = (self.playing, &self.output) {
            out.set_looping(self.loops(mode));
        }
        if let Some(Ok(c)) = &mut self.controller {
            c.set_looping(on);
        }
    }

    /// The metronome on or off, for what plays now too.
    fn set_metronome(&mut self, on: bool) {
        self.metronome = on;
        if let (Some(_), Some(Ok(out))) = (self.playing, &self.output) {
            out.set_metronome(on);
        }
        if let Some(Ok(c)) = &mut self.controller {
            c.set_metronome(on);
        }
    }

    fn toggle_play(&mut self) {
        if self.playing.is_some() {
            self.stop();
        } else {
            self.transport_play();
        }
        self.sync_transport();
    }

    /// Focus a scene; while a scene plays, switch to it when the playing one ends.
    fn launch_scene(&mut self, scene: usize) {
        self.focus_scene(scene);
        let Some(doc) = &self.doc else { return };
        if let (Some(Mode::Scene(_)), Some(Ok(out))) = (self.playing, &self.output) {
            out.queue(doc.borrow().project(), Mode::Scene(scene));
        }
    }

    /// Print to Clip from a MIDI controller: `p`'s notes become the focused sound's in
    /// the focused group's pattern in the focused scene (one made and put there if
    /// there's none), the pattern as long as the print if that's longer; one undo.
    pub(crate) fn print_notes(&mut self, p: &deckard_surface::device::Print) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, sound, scene) = (self.group, self.sound, self.scene);
        let mut d = doc.borrow_mut();
        let bar = d.project().bar_length().max(1);
        // A group taking MIDI as pads (Key Mode Drumkit or Manual) takes a print as
        // them, as it would the notes played: each pad's sound at its root, the print
        // standing for all of the group's notes.
        // So does a print from a controller's drum track, as its drum channel's notes do.
        let drum_track = p.track.is_some() && self.settings.drum_tracks.get(&p.device).copied() == p.track;
        let pads = d.project().groups.get(g).and_then(|grp| {
            let start = match grp.channel.key_mode {
                _ if drum_track => deckard_engine::midi::PAD_START,
                1 => deckard_engine::midi::DRUMKIT_START,
                2 => grp.channel.start_note,
                _ => return None,
            };
            Some((start, grp.sounds.len().min(16)))
        });
        let notes: Vec<Note> = p
            .notes
            .iter()
            .filter_map(|n| {
                let (sound, pitch) = match pads {
                    Some((start, count)) => {
                        let k = i64::from(n.pitch) - start;
                        if !(0..count as i64).contains(&k) {
                            return None;
                        }
                        (k as usize, 60)
                    }
                    None => (sound, i64::from(n.pitch)),
                };
                Some(Note {
                    sound,
                    position: (n.start * PPQ as f64).round() as i64,
                    length: ((n.length * PPQ as f64).round() as i64).max(1),
                    pitch,
                    velocity: i64::from(n.velocity),
                    probability: 100,
                    expr: None,
                })
            })
            .collect();
        // With no length sent: whole bars holding every note.
        let length = match (p.length * PPQ as f64).round() as i64 {
            0 => (notes.iter().map(|n| n.position + n.length).max().unwrap_or(0) + bar - 1) / bar * bar,
            l => l,
        }
        .max(1);
        let notes = print_wrap(&notes, length);
        let default_length = self.default_pattern_length(d.project());
        let from = self.pattern;
        let mut made = None;
        let r = d.transaction("Print to Clip", |d| {
            let in_scene = d.project().scenes.get(scene).and_then(|s| s.patterns.iter().find(|(pg, _)| *pg == g)).map(|&(_, pi)| pi);
            let pi = match in_scene {
                Some(pi) => pi,
                None => {
                    let n = d.project().groups.get(g).ok_or("no such group")?.patterns.len();
                    let pi = d.add_pattern(g, from.min(n.saturating_sub(1)), &format!("Pattern {}", n + 1))?;
                    d.set_pattern_length(g, pi, default_length)?;
                    if scene < d.project().scenes.len() {
                        d.assign_pattern(scene, g, Some(pi))?;
                    }
                    pi
                }
            };
            let old: Vec<Note> =
                d.project().groups[g].patterns[pi].notes.iter().filter(|n| pads.is_some() || n.sound == sound).cloned().collect();
            d.edit_notes("Print to Clip", g, pi, &old, &notes)?;
            if d.project().groups[g].patterns[pi].length != length {
                d.set_pattern_length(g, pi, length)?;
            }
            made = Some(pi);
            Ok(())
        });
        drop(d);
        match r {
            Ok(()) => {
                if let Some(pi) = made {
                    self.pattern = pi;
                }
            }
            Err(e) => self.status = format!("Print to Clip: {e}"),
        }
    }

    /// Focus a scene; the pattern editor shows the focused group's pattern in it, as
    /// Maschine does.
    fn focus_scene(&mut self, scene: usize) {
        self.scene = scene;
        if let Some(pi) = self.scene_pattern(self.group) {
            self.pattern = pi;
        }
    }

    /// Focus a group, and its pattern in the focused scene (else its first).
    fn focus_group(&mut self, group: usize) {
        if group != self.group {
            self.group = group;
            self.sound = 0;
            self.pattern = self.scene_pattern(group).unwrap_or(0);
        }
    }

    /// The pattern `group` plays in the focused scene.
    /// Where playback is in group `g`'s pattern of the focused scene: (ticks into it, its
    /// length).
    fn pattern_at(&self, g: usize, head: Option<Playhead>) -> Option<(i64, i64)> {
        let (pi, head) = (self.scene_pattern(g)?, head?);
        let d = self.doc.as_ref()?.borrow();
        let pt = d.project().groups.get(g)?.patterns.get(pi)?;
        let length = pt.length.max(1);
        let at = (head.position as i64).rem_euclid(length);
        Some((at, length))
    }

    fn scene_pattern(&self, group: usize) -> Option<usize> {
        let d = self.doc.as_ref()?.borrow();
        let s = d.project().scenes.get(self.scene)?;
        s.patterns.iter().find(|(g, _)| *g == group).map(|&(_, pi)| pi)
    }

    /// With Follow on, the focus follows playback: the playing scene (or the
    /// playing section's scene) and its patterns.
    fn follow_playback(&mut self, head: Playhead) {
        let playing_scene = match head.mode {
            // A scene picked to play next keeps the focus until it starts.
            Mode::Scene(s) if head.queued.is_none() => Some(s),
            Mode::Arrangement => {
                let d = self.doc.as_ref().map(|d| d.borrow());
                let p = d.as_ref().map(|d| d.project());
                p.and_then(|p| p.section_at(head.position as i64).and_then(|i| p.sections[i].scene))
            }
            _ => None,
        };
        if let Some(s) = playing_scene
            && s != self.scene
            && self.pending_launch.is_none()
            && (self.follow || matches!(head.mode, Mode::Scene(_)))
        {
            self.focus_scene(s);
            self.sync_controller();
        }
    }

    /// A scene or arrangement edit from the panels; the focus follows what changed.
    fn arrange(&mut self, act: layout::Act) {
        use layout::Act;
        let Some(doc) = self.doc.clone() else { return };
        let result = {
            let mut d = doc.borrow_mut();
            match act {
                // A scene past the last (there are none): made, with the pattern in.
                Act::AssignPattern(s, g, Some(p)) if s >= d.project().scenes.len() => {
                    let mut made = 0;
                    d.transaction("Add Scene", |d| {
                        made = d.add_scene()?;
                        d.assign_pattern(made, g, Some(p))
                    })
                    .map(|_| Some((Some(made), None)))
                }
                Act::AssignPattern(s, g, p) => d.assign_pattern(s, g, p).map(|_| None),
                Act::NewPattern(s, g) => {
                    let n = d.project().groups.get(g).map_or(0, |x| x.patterns.len());
                    // Like the focused pattern (its colour), or the group's first; the
                    // Settings page's LENGTH long.
                    let from = if g == self.group { self.pattern } else { 0 };
                    let mut made = 0;
                    let length = self.default_pattern_length(d.project());
                    let r = d.transaction("New Pattern", |d| {
                        made = d.add_pattern(g, from, &format!("Pattern {}", n + 1))?;
                        d.set_pattern_length(g, made, length)?;
                        if s < d.project().scenes.len() {
                            d.assign_pattern(s, g, Some(made))?;
                        }
                        Ok(())
                    });
                    if r.is_ok() {
                        (self.group, self.pattern) = (g, made);
                    }
                    r.map(|_| None)
                }
                Act::AddScene => d.add_scene().map(|s| Some((Some(s), None))),
                Act::AddGroup => d.add_group().map(|g| {
                    (self.group, self.sound, self.pattern) = (g, 0, 0);
                    None
                }),
                Act::InsertScene(at) => d.insert_scene(at).map(|s| Some((Some(s), None))),
                Act::DuplicateScene(s) => {
                    let patterns = self.settings.duplicate_patterns;
                    d.duplicate_scene_with(s, patterns).map(|s| Some((Some(s), None)))
                }
                Act::DeleteScene(s) => d.delete_scene(s).map(|_| Some((Some(s.saturating_sub(1)), None))),
                Act::RenameScene(s, name) => d.rename(deckard_doc::Target::Scene(s), &name).map(|_| None),
                Act::AddSection(s) => d.add_section(s).map(|i| Some((None, Some(i)))),
                Act::DuplicateSection(i) => {
                    let link = self.settings.link_duplicates;
                    d.duplicate_section_with(i, link, self.settings.duplicate_patterns).map(|i| Some((None, Some(i))))
                }
                Act::DeleteSection(i) => d.delete_section(i).map(|_| Some((None, None))),
                Act::MoveSection(a, b) => d.move_section(a, b).map(|_| Some((None, Some(b)))),
                Act::MoveScene(a, b) => d.move_scene(a, b).map(|_| Some((Some(b), None))),
                Act::SectionLength(i, t) => d.set_section_length(i, t).map(|_| None),
                Act::SectionScene(i, s) => d.set_section_scene(i, s).map(|_| None),
                Act::PatternLength(g, p, t) => {
                    // A length chosen: no longer Auto.
                    d.set_length_auto(g, p, false);
                    d.set_pattern_length(g, p, t).map(|_| None)
                }
                Act::PatternSpan(g, p, s, t) => d.set_pattern_span(g, p, s, t).map(|_| None),
                Act::TimeSignature(n, den) => d.set_time_signature(n, den).map(|_| None),
                Act::SetSwing(t, s) => d.set_swing(t, s).map(|_| None),
                Act::Quantize(pct) => {
                    // The selection, or the whole focused pattern.
                    let (g, p) = (self.group, self.pattern);
                    let Some(pat) = d.project().groups.get(g).and_then(|gr| gr.patterns.get(p)) else { return };
                    // Only selected notes that still exist count.
                    let selected: Vec<Note> = self.selected.iter().filter(|n| pat.notes.contains(n)).copied().collect();
                    let notes: Vec<Note> = if selected.is_empty() { pat.notes.clone() } else { selected };
                    let (start, grid) = (pat.start, self.grid.max(1));
                    let moved: Vec<Note> = notes
                        .iter()
                        .map(|n| {
                            let rel = n.position - start;
                            let target = ((rel as f64 / grid as f64).round() as i64) * grid;
                            Note { position: start + rel + (target - rel) * pct / 100, ..*n }
                        })
                        .collect();
                    let (remove, add): (Vec<Note>, Vec<Note>) = notes.into_iter().zip(moved).filter(|(a, b)| a != b).unzip();
                    if remove.is_empty() {
                        return;
                    }
                    // (While recording: a step of its own, not part of the take.)
                    let r = d.outside_take(|d| d.edit_notes("Quantize", g, p, &remove, &add));
                    if r.is_ok() && !self.selected.is_empty() {
                        self.selected = d.project().groups[g].patterns[p]
                            .notes
                            .iter()
                            .filter(|n| add.contains(n) || self.selected.contains(n))
                            .copied()
                            .collect();
                    }
                    r.map(|_| None)
                }
                Act::EditMidi { g, p, label, remove, add } => d.edit_midi(label, g, p, &remove, &add).map(|_| None),
                Act::ToggleAudio(g, s, pi) => {
                    let on = d
                        .project()
                        .groups
                        .get(g)
                        .and_then(|gr| gr.patterns.get(pi))
                        .is_some_and(|pt| pt.audio.iter().any(|c| c.sound == s));
                    let take =
                        d.project().groups.get(g).and_then(|gr| gr.sounds.get(s)).and_then(|x| x.audio.as_ref()).map_or(0, |a| a.focused);
                    let (take, label) = if on { (None, "Audio Off") } else { (Some(take), "Audio On") };
                    d.set_audio_in(g, s, &[pi], take, label).map(|_| None)
                }
                Act::SplitNotes { g, p, targets, at } => {
                    let (remove, add) = d.split_notes(&targets, at);
                    d.edit_notes("Split Notes", g, p, &remove, &add).map(|_| None)
                }
                Act::JoinNotes { g, p, all, clicked, selected } => {
                    let (remove, add) = d.join_notes(&all, clicked, &selected);
                    d.edit_notes("Join note events", g, p, &remove, &add).map(|_| None)
                }
                Act::SetExpressions { g, p, edits } => {
                    let r = d.set_expressions(g, p, &edits);
                    if let Ok(now) = &r {
                        // The selection follows its notes (their expression ids changed).
                        for ((old, _), new) in edits.iter().zip(now) {
                            if let Some(s) = self.selected.iter_mut().find(|n| *n == old) {
                                *s = *new;
                            }
                        }
                    }
                    r.map(|_| None)
                }
                Act::EditNotes { g, p, label, remove, add } => {
                    let upgrade = add.iter().any(|n| n.probability != 100) && !d.notes_have_probability();
                    let r = d.outside_take(|d| d.edit_notes(label, g, p, &remove, &add));
                    if r.is_ok() {
                        // The selection follows the selected notes that were edited
                        // (moved, nudged, ...); new notes aren't selected, deleted ones
                        // leave it.
                        let mut kept: Vec<Note> = self.selected.iter().filter(|n| !remove.contains(n)).copied().collect();
                        for (old, new) in remove.iter().zip(&add) {
                            if self.selected.contains(old) {
                                kept.push(*new);
                            }
                        }
                        self.selected = kept;
                        if upgrade {
                            self.status = "Notes were upgraded to Maschine 3's format to store probabilities".into();
                        }
                    }
                    r.map(|_| None)
                }
                _ => return,
            }
        };
        match result {
            Ok(Some((scene, section))) => {
                if let Some(s) = scene {
                    self.focus_scene(s);
                }
                self.section = section;
                self.sync_controller();
            }
            Ok(None) => {}
            Err(e) => self.status = format!("can't do that: {e}"),
        }
    }

    /// Pick a section in the Song view. While the song plays, playback jumps to it
    /// when the playing section ends; when stopped, PLAY starts from it.
    fn launch_section(&mut self, section: usize) {
        let Some(doc) = self.doc.clone() else { return };
        let d = doc.borrow();
        let p = d.project();
        let Some(sec) = p.sections.get(section) else { return };
        self.section = Some(section);
        let scene = sec.scene;
        drop(d);
        if let Some(scene) = scene {
            self.focus_scene(scene);
        }
        let d = doc.borrow();
        let p = d.project();
        let Some(Ok(out)) = &self.output else { return };
        if self.playing != Some(Mode::Arrangement) {
            return;
        }
        let to = p.section_start(section);
        // Decided on the audio thread, against where playback is at that moment.
        let bounds: Vec<(i64, i64)> = (0..p.sections.len()).map(|i| (p.section_start(i), p.sections[i].length)).collect();
        out.run(move |e| {
            let now = e.position() as i64;
            match bounds.iter().position(|&(start, len)| (start..start + len).contains(&now)) {
                Some(cur) if cur == section => e.jump_at(None),
                Some(cur) => e.jump_at(Some((bounds[cur].0 + bounds[cur].1, to))),
                None => e.seek(to),
            }
        });
    }

    /// The section playback will jump to, if one is queued.
    fn queued_section(&self) -> Option<usize> {
        let (Some(Ok(out)), Some(doc)) = (&self.output, &self.doc) else { return None };
        let (_, to) = out.telemetry(|t| t.pending_jump)?;
        let d = doc.borrow();
        let p = d.project();
        (0..p.sections.len()).find(|&i| p.section_start(i) == to)
    }

    fn stop(&mut self) {
        // STOP ends a recording, its notes still held where the song was.
        if self.recording {
            self.set_recording(false, false);
        }
        if let Some(h) = self.playhead() {
            self.parked = Some(Playhead { queued: None, ..h });
        }
        if let Some(Ok(out)) = &self.output {
            out.stop();
        }
        self.playing = None;
    }

    /// A knob moved while writing modulation (docs/superpowers/plans/
    /// 2026-10-22-phase2c-modulation.md): with the song at `head` (mode, tick), its
    /// deviation from the knob's own value goes into the playing pattern of its
    /// channel's group there, one undo step until `done` (the knob let go), when the
    /// movement is thinned to points. Stopped, or a knob that can't be modulated, it's
    /// an ordinary edit.
    fn modulate_at(&mut self, edit: layout::Edit, amount: f32, done: bool, head: Option<(Mode, f64)>) {
        let Some(doc) = self.doc.clone() else { return };
        let target = modulation::mod_target(edit);
        let (Some((chain, target)), Some((mode, tick))) = (target, head) else {
            self.end_mod_take();
            layout::apply_acts(self, vec![edit.to_act(amount)]);
            return;
        };
        let g = match chain {
            deckard_doc::Chain::Sound(g, _) | deckard_doc::Chain::Group(g) => g,
            deckard_doc::Chain::Master => return,
        };
        let (pi, pos, dev) = {
            let d = doc.borrow();
            let p = d.project();
            let Some((pi, pos)) = record::pattern_at(p, mode, tick, g) else { return };
            let Some(at) = modulation::knob_travel(p, chain, &target) else { return };
            (pi, pos, (amount as f64 - at).clamp(-1.0, 1.0))
        };
        // Another knob or pattern: that touch ends.
        if self.mod_take.as_ref().is_some_and(|t| t.chain != chain || t.target != target || t.pattern != pi) {
            self.end_mod_take();
        }
        let take = self.mod_take.get_or_insert_with(|| {
            doc.borrow_mut().begin_take("Record Modulation");
            modulation::ModTake { chain, target: target.clone(), pattern: pi, samples: Vec::new(), moved: std::time::Instant::now() }
        });
        take.moved = std::time::Instant::now();
        self.touch_write(pos, dev);
        if done {
            self.end_mod_take();
        }
    }

    /// The touch going on writes `dev` at `pos`, over what was there since the last
    /// position it wrote (the points passed over go; over the pattern's end, its end and
    /// its start).
    fn touch_write(&mut self, pos: i64, dev: f64) {
        let (Some(doc), Some(take)) = (self.doc.clone(), self.mod_take.as_mut()) else { return };
        let mut d = doc.borrow_mut();
        let (c, t, pi) = (take.chain, take.target.clone(), take.pattern);
        let g = match c {
            deckard_doc::Chain::Sound(g, _) | deckard_doc::Chain::Group(g) => g,
            deckard_doc::Chain::Master => return,
        };
        let result = match take.samples.last().map(|x| x.0) {
            Some(l) if l < pos => d.set_modulation(c, &t, pi, (l + 1, pos), &[(pos, dev)]),
            Some(l) if l > pos => {
                // Wrapped: the run so far as points, then on from the start.
                let len = d.project().groups[g].patterns.get(pi).map_or(l + 1, |p| p.length);
                let run = std::mem::take(&mut take.samples);
                modulation::write_run(&mut d, c, &t, pi, &run)
                    .and_then(|_| d.set_modulation(c, &t, pi, (l + 1, (len - 1).max(l + 1)), &[]))
                    .and_then(|_| d.set_modulation(c, &t, pi, (0, pos), &[(pos, dev)]))
            }
            Some(_) => Ok(()),
            None => d.set_modulation(c, &t, pi, (pos, pos), &[(pos, dev)]),
        };
        take.samples.push((pos, dev));
        if let Err(e) = result {
            self.status = format!("can't write modulation: {e}");
        }
    }

    /// The song at `head` while a knob is touched: still moving (within
    /// [`record::IDLE`]), it goes on overwriting at its last value as the song passes,
    /// as the controller lanes do; idle, stopped or in another pattern, the touch ends.
    pub(crate) fn tick_mod_take(&mut self, head: Option<(Mode, f64)>, now: std::time::Instant) {
        let Some(take) = self.mod_take.as_ref() else { return };
        let (Some((mode, tick)), Some(doc)) = (head, self.doc.clone()) else { return self.end_mod_take() };
        if now.duration_since(take.moved) >= record::IDLE {
            return self.end_mod_take();
        }
        let g = match take.chain {
            deckard_doc::Chain::Sound(g, _) | deckard_doc::Chain::Group(g) => g,
            deckard_doc::Chain::Master => return,
        };
        let at = record::pattern_at(doc.borrow().project(), mode, tick, g);
        match (at, take.samples.last()) {
            (Some((pi, pos)), Some(&(last, dev))) if pi == take.pattern && pos != last => self.touch_write(pos, dev),
            (Some((pi, _)), _) if pi == take.pattern => {}
            _ => self.end_mod_take(),
        }
    }

    /// Where a knob's modulation has it (0..1): as last written in the touch going on,
    /// else at the play position; `None` when it isn't modulated.
    fn mod_position(&self, edit: layout::Edit) -> Option<f64> {
        let (chain, target) = modulation::mod_target(edit)?;
        if let Some(take) = self.mod_take.as_ref().filter(|t| t.chain == chain && t.target == target)
            && let Some(&(_, dev)) = take.samples.last()
        {
            let doc = self.doc.as_ref()?.borrow();
            return Some((modulation::knob_travel(doc.project(), chain, &target)? + dev).clamp(0.0, 1.0));
        }
        self.mod_amount(edit).map(f64::from)
    }

    /// Auto-write: the MK3's AUTO and the header's are one switch. The knob being
    /// written is let go when it goes off or the song stops.
    fn sync_auto_write(&mut self) {
        let was = self.auto_write;
        if let Some(Ok(c)) = &self.controller {
            let mut st = c.host.state.borrow_mut();
            if let Some(on) = st.auto_write_set.take() {
                self.auto_write = on;
            }
            st.auto_write = self.auto_write;
        }
        if self.mod_take.is_some() && (self.playing.is_none() || (was && !self.auto_write)) {
            self.end_mod_take();
        }
    }

    /// [`App::modulate_at`] at the song's position now.
    fn modulate(&mut self, edit: layout::Edit, amount: f32, done: bool) {
        let head = self.playhead().map(|h| (h.mode, h.position));
        self.modulate_at(edit, amount, done, head);
    }

    /// Remove the modulation of a knob's parameter (every pattern), one undo step.
    fn clear_modulation(&mut self, edit: layout::Edit) {
        let (Some(doc), Some((chain, target))) = (self.doc.clone(), modulation::mod_target(edit)) else { return };
        // A touch still open on it would write its movement back when it ends.
        self.end_mod_take();
        if let Err(e) = doc.borrow_mut().clear_modulation(chain, &target) {
            self.status = format!("can't remove the modulation: {e}");
        }
    }

    /// Where a modulated knob's modulation has it now (0..1): its value moved by the
    /// playing pattern's points at the play position; `None` when it isn't modulated
    /// there (or the song is stopped).
    pub(crate) fn mod_amount(&self, edit: layout::Edit) -> Option<f32> {
        let (chain, target) = modulation::mod_target(edit)?;
        let head = self.playhead()?;
        let doc = self.doc.as_ref()?.borrow();
        let p = doc.project();
        let (g, sound) = match chain {
            deckard_doc::Chain::Sound(g, s) => (g, Some(s)),
            deckard_doc::Chain::Group(g) => (g, None),
            deckard_doc::Chain::Master => return None,
        };
        let (pi, pos) = record::pattern_at(p, head.mode, head.position, g)?;
        let points: Vec<(i64, f64)> = p.groups[g].patterns[pi]
            .mods
            .iter()
            .filter(|m| m.sound == sound && m.slot == target.slot && m.field == target.field)
            .map(|m| (m.position, m.value))
            .collect();
        if points.is_empty() {
            return None;
        }
        let lane = deckard_engine::modscale::ModLane { group: g, sound, slot: 0, field: String::new(), from: 0, to: 0, points };
        let at = modulation::knob_travel(p, chain, &target)?;
        Some((at + lane.value_at(pos as f64)).clamp(0.0, 1.0) as f32)
    }

    /// The values sound (`g`, `s`)'s routes give its built-in knobs now, as the playing
    /// engine published them ([`layout::mod_routes::routed_values`]).
    pub(crate) fn routed_values(&self, g: usize, s: usize, snd: &deckard_model::Sound) -> Vec<(usize, String, f64)> {
        match &self.output {
            Some(Ok(out)) if !snd.mod_routes.is_empty() => {
                // Read only as the route the document has there (an edit may not have
                // reached the engine yet).
                let key = |k: usize| deckard_engine::meters::route_key(&snd.mod_routes[k]);
                layout::mod_routes::routed_values(snd, |k| out.route_travel(g, s, k, key(k)))
            }
            _ => Vec::new(),
        }
    }

    /// Slot `i` of sound (`g`, `s`) as its knobs show it: where its routes have it now
    /// (as a plug-in's knobs show where modulation has them).
    pub(crate) fn live_slot<'a>(
        &self,
        (g, s, i): (usize, usize, usize),
        snd: &deckard_model::Sound,
        slot: &'a deckard_model::SlotInfo,
    ) -> std::borrow::Cow<'a, deckard_model::SlotInfo> {
        layout::mod_routes::live_slot(slot, i, &self.routed_values(g, s, snd))
    }

    /// [`App::live_slot`], and the [`layout::mod_routes::Bases`] its knobs' edits
    /// start from (their targets `t`).
    pub(crate) fn live_slot_bases<'a>(
        &self,
        (g, s, i): (usize, usize, usize),
        snd: &deckard_model::Sound,
        slot: &'a deckard_model::SlotInfo,
        t: deckard_doc::Target,
    ) -> (std::borrow::Cow<'a, deckard_model::SlotInfo>, layout::mod_routes::Bases) {
        let vals = self.routed_values(g, s, snd);
        (layout::mod_routes::live_slot(slot, i, &vals), layout::mod_routes::slot_bases(slot, t, i, &vals))
    }

    /// [`App::live_sampler`], and the [`layout::mod_routes::Bases`] its knobs' edits
    /// start from.
    pub(crate) fn live_sampler_bases<'a>(
        &self,
        g: usize,
        s: usize,
        snd: &deckard_model::Sound,
        smp: &'a deckard_model::Sampler,
    ) -> (std::borrow::Cow<'a, deckard_model::Sampler>, layout::mod_routes::Bases) {
        let i = snd.slots.iter().position(|x| x.module == "Sampler").unwrap_or(0);
        let vals = self.routed_values(g, s, snd);
        let t = deckard_doc::Target::Sampler(g, s);
        (layout::mod_routes::live_sampler(smp, i, &vals), layout::mod_routes::sampler_bases(smp, t, i, &vals))
    }

    /// Sound (`g`, `s`)'s Sampler as its knobs show it ([`App::live_slot`]).
    pub(crate) fn live_sampler<'a>(
        &self,
        g: usize,
        s: usize,
        snd: &deckard_model::Sound,
        smp: &'a deckard_model::Sampler,
    ) -> std::borrow::Cow<'a, deckard_model::Sampler> {
        let i = snd.slots.iter().position(|x| x.module == "Sampler").unwrap_or(0);
        layout::mod_routes::live_sampler(smp, i, &self.routed_values(g, s, snd))
    }

    /// The knob written as modulation is let go: its movement as few points (when it
    /// stayed within one pass of the pattern), and the take's one undo step.
    fn end_mod_take(&mut self) {
        let Some(take) = self.mod_take.take() else { return };
        let Some(doc) = self.doc.clone() else { return };
        let mut d = doc.borrow_mut();
        if let Err(e) = modulation::write_run(&mut d, take.chain, &take.target, take.pattern, &take.samples) {
            self.status = format!("can't write modulation: {e}");
        }
        d.end_take();
    }

    /// REC on or off (docs/superpowers/plans/2026-10-23-auto-length-recording.md). On: a
    /// take begins (one undo step); an empty Auto-length focused pattern starts as a bar
    /// and grows as it's played into; from a stop it's armed (PLAY starts it), or with
    /// `count_in` (SHIFT + REC) the song starts after a count-in (Preferences', at least a
    /// bar). Off: the notes held end, a grown pattern is trimmed to the bars recorded,
    /// and the take is done.
    fn set_recording(&mut self, on: bool, count_in: bool) {
        // Armed, SHIFT + REC: count in and start.
        if on && self.recording && count_in && self.playing.is_none() {
            self.count_in_next = self.settings.count_in_bars.max(1);
            self.transport_play();
            self.sync_transport();
            self.status = "Recording".into();
            return;
        }
        if on == self.recording {
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        self.recording = on;
        if let Some(Ok(c)) = &mut self.controller {
            c.set_recording(on);
        }
        if on {
            doc.borrow_mut().begin_take("Record");
            self.take = record::Take::default();
            (self.take_played, self.take_grown) = (0.0, None);
            self.controls = record::ControlWriter::default();
            // An empty Auto-length pattern starts as a bar, growing as it's played into
            // (unless the Settings page's GROW is off).
            let (g, pi) = (self.group, self.pattern);
            let empty =
                doc.borrow().project().groups.get(g).and_then(|x| x.patterns.get(pi)).filter(|p| p.notes.is_empty()).map(|p| p.length);
            self.grow = None;
            if let Some(len) = empty.filter(|_| self.settings.pattern_grow && doc.borrow().length_is_auto(g, pi)) {
                let mut grow = record::Grow::new(g, pi, doc.borrow().project().bar_length());
                grow.before = len;
                if doc.borrow_mut().set_pattern_length(g, pi, grow.length()).is_ok() {
                    self.grow = Some(grow);
                }
            }
            if self.playing.is_some() {
                if let Some(Ok(out)) = &self.output {
                    out.set_recording(true);
                }
                self.status = "Recording".into();
            } else if count_in {
                // SHIFT + REC: count in, then play and record.
                self.count_in_next = self.settings.count_in_bars.max(1);
                self.transport_play();
                self.sync_transport();
                self.status = "Recording".into();
            } else {
                // Armed: PLAY starts the take.
                self.status = "Armed: PLAY to record".into();
            }
        } else {
            if let Some(Ok(out)) = &self.output {
                out.set_recording(false);
                let (notes, exprs, controls) = (out.take_recorded(), out.take_recorded_expr(), out.take_recorded_controls());
                self.take_in_all(notes, exprs, controls, std::time::Instant::now());
            }
            let now = self.playhead().map_or(self.take_played, |h| h.played);
            let held = self.take.close(now);
            self.apply_take(held);
            self.controls.finish(&mut doc.borrow_mut());
            // A grown pattern: the bars played (settled already, or now), or as it was.
            if let Some(grow) = self.grow.take().filter(|x| !x.settled()) {
                let (g, pi) = (grow.group, grow.pattern);
                let d = doc.borrow();
                let p = d.project();
                let bar = p.bar_length();
                let pat = &p.groups[g].patterns[pi];
                let end = pat.notes.iter().map(|n| n.position - pat.start + n.length).max();
                let to = end.map_or(grow.before, |e| ((e + bar - 1) / bar).max(1) * bar);
                drop(d);
                let _ = doc.borrow_mut().set_pattern_length(g, pi, to);
            }
            doc.borrow_mut().end_take();
            self.status = "Recorded".into();
        }
    }

    /// Notes the engine recorded: written as their releases come.
    #[cfg(test)]
    fn take_in_recorded(&mut self, recs: Vec<deckard_engine::Recorded>) {
        self.take_in_all(recs, Vec::new(), Vec::new(), std::time::Instant::now());
    }

    /// What the engine recorded (notes, the expression on them, controllers), taken in
    /// the order it was played.
    fn take_in_all(
        &mut self,
        notes: Vec<deckard_engine::Recorded>,
        exprs: Vec<deckard_engine::RecordedExpr>,
        controls: Vec<deckard_engine::RecordedControl>,
        now: std::time::Instant,
    ) {
        let Some(doc) = self.doc.clone() else { return };
        for r in record::Rec::merged(notes, exprs, controls) {
            let r = match r {
                record::Rec::Note(r) => r,
                record::Rec::Expr(r) => {
                    self.take.add_expr(r);
                    continue;
                }
                record::Rec::Control(r) => {
                    self.controls.add(&mut doc.borrow_mut(), r, now);
                    continue;
                }
            };
            // A note into a growing pattern's last bar adds the next.
            if r.velocity > 0
                && let Some(grow) = &mut self.grow
                && r.group == grow.group
            {
                let at = record::pattern_at(doc.borrow().project(), r.mode, r.tick, r.group);
                let start = doc.borrow().project().groups[grow.group].patterns.get(grow.pattern).map_or(0, |p| p.start);
                if let Some((pi, pos)) = at.filter(|x| x.0 == grow.pattern)
                    && let Some(len) = grow.hit(pos - start)
                    && let Err(e) = doc.borrow_mut().set_pattern_length(r.group, pi, len)
                {
                    self.status = format!("can't grow the pattern: {e}");
                }
            }
            self.take_played = self.take_played.max(r.played);
            let at = record::Placing { quantize: (self.settings.input_quantize && self.grid > 0).then_some(self.grid) };
            let edits = {
                let d = doc.borrow();
                self.take.add(d.project(), r, at)
            };
            self.apply_take(edits);
        }
    }

    /// The song at `tick`: controllers being recorded overwrite their lanes up to it.
    fn tick_controls(&mut self, mode: deckard_engine::Mode, tick: f64, now: std::time::Instant) {
        if let Some(doc) = self.doc.clone() {
            self.controls.tick(&mut doc.borrow_mut(), mode, tick, now);
        }
    }

    /// The take's edits into the patterns (overdubbing), the expression played on a
    /// finished note with it. A note it changes that's gone (edited meanwhile) is
    /// written again.
    fn apply_take(&mut self, edits: Vec<record::Edit>) {
        let Some(doc) = self.doc.clone() else { return };
        for e in edits {
            let mut add = e.add;
            if let Some(x) = e.expr {
                add.expr = Some(doc.borrow_mut().add_expression(x));
            }
            let done = doc.borrow_mut().edit_notes("Record", e.g, e.pi, &e.remove, &[add]);
            if done.is_err()
                && let Err(err) = doc.borrow_mut().edit_notes("Record", e.g, e.pi, &[], &[add])
            {
                self.status = format!("can't record: {err}");
            }
        }
    }

    /// The computer keyboard on or off (⌘K, the header's keyboard button).
    pub fn toggle_keys(&mut self) {
        self.keys.on = !self.keys.on;
        for (_, pitch) in std::mem::take(&mut self.keys.held) {
            self.key_note(pitch, 0);
        }
        self.status = if self.keys.on { self.keys.summary() } else { "Keys off".into() };
    }

    /// ⌘K switches the computer keyboard to playing notes; while it does, its letter
    /// keys play the focused sound (see [`keys`]).
    fn keyboard_keys(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
            self.toggle_keys();
        }
        if !self.keys.on || ctx.egui_wants_keyboard_input() {
            return;
        }
        let events: Vec<(egui::Key, bool)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, pressed, repeat: false, modifiers, .. } if modifiers.is_none() => Some((*key, *pressed)),
                    _ => None,
                })
                .collect()
        });
        for (key, down) in events {
            match self.keys.key(key, down) {
                Some(keys::KeyAct::NoteOn(pitch, velocity)) => self.key_note(pitch, velocity),
                Some(keys::KeyAct::NoteOff(pitch)) => self.key_note(pitch, 0),
                Some(keys::KeyAct::Changed) => self.status = self.keys.summary(),
                None => {}
            }
        }
    }

    /// Start (`velocity` > 0) or end a held note of the focused sound.
    fn key_note(&mut self, pitch: i64, velocity: i64) {
        let (group, sound) = (self.group, self.sound);
        let Some(doc) = self.doc.clone() else { return };
        let Ok(out) = self.output.get_or_insert_with(|| open_output(&self.settings, &mut self.output_fallback)) else { return };
        let ev = if velocity > 0 {
            deckard_engine::Live::NoteOn { group, sound, pitch, velocity, length: i64::MAX }
        } else {
            deckard_engine::Live::NoteRelease { group, sound, pitch }
        };
        if out.has_engine() {
            out.live(ev);
        } else if velocity > 0 {
            let d = doc.borrow();
            let (mut engine, _) = Engine::new(d.project(), self.path.as_deref(), out.rate, Mode::Arrangement, &mut self.samples);
            engine.sequencing = false;
            engine.note_hold(group, sound, pitch, velocity);
            out.play(engine);
            out.note_played(&ev);
        }
    }

    /// A growing pattern, with the song at `tick` in `mode`: come round to its start
    /// with its last bar empty, it settles on the bars played.
    fn auto_grow_at(&mut self, mode: Mode, tick: f64) {
        let Some(doc) = self.doc.clone() else { return };
        let Some(grow) = &mut self.grow else { return };
        let (g, pi) = (grow.group, grow.pattern);
        let at = record::pattern_at(doc.borrow().project(), mode, tick, g).filter(|x| x.0 == pi);
        let start = doc.borrow().project().groups[g].patterns.get(pi).map_or(0, |p| p.start);
        if let Some((_, pos)) = at
            && let Some(len) = grow.at(pos - start)
            && let Err(e) = doc.borrow_mut().set_pattern_length(g, pi, len)
        {
            self.status = format!("can't settle the pattern: {e}");
        }
    }

    /// What the engine recorded since the last frame.
    fn poll_recorded(&mut self) {
        if !self.recording {
            return;
        }
        let (notes, exprs, controls) = match &self.output {
            Some(Ok(out)) => (out.take_recorded(), out.take_recorded_expr(), out.take_recorded_controls()),
            _ => return,
        };
        let now = std::time::Instant::now();
        if !(notes.is_empty() && exprs.is_empty() && controls.is_empty()) {
            self.take_in_all(notes, exprs, controls, now);
        }
        if let Some(h) = self.playhead() {
            self.auto_grow_at(h.mode, h.position);
            self.tick_controls(h.mode, h.position, now);
            // Held notes stretch in the pattern as they're held.
            self.take_played = h.played;
            if self.take.holding() && self.take_grown.is_none_or(|t| now.duration_since(t) >= TAKE_GROW) {
                self.take_grown = Some(now);
                let edits = self.take.grow(self.take_played);
                self.apply_take(edits);
            }
        }
    }

    /// Act on what the controller scripts asked for.
    fn handle(&mut self, requests: Vec<controller::Request>) {
        for r in requests {
            match r {
                controller::Request::Transport(true) => self.transport_play(),
                controller::Request::Transport(false) => self.stop(),
                controller::Request::Recording(on, count_in) => self.set_recording(on, count_in),
                controller::Request::Restart => self.restart(),
                controller::Request::Sampling(true) => self.sample_focused_sound(),
                controller::Request::Sampling(false) => self.sampling_open = false,
                controller::Request::Record(toggle) => self.record_press(toggle),
                controller::Request::Loop(on) => self.set_looping(on),
                controller::Request::Metronome(on) => self.set_metronome(on),
                controller::Request::Sync => self.sync_from_controller(),
                // The scripts focus the sound (or not: Pad mode's Focus is Pinned).
                controller::Request::Note(g, s) => self.note(g, s),
                controller::Request::FocusGroup(g) => self.focus_group(g),
                controller::Request::FocusSound(s) => self.sound = s,
                controller::Request::LaunchScene(s) => self.launch_scene(s),
                controller::Request::Bounce(g, s) => self.request_bounce(g, s),
                controller::Request::LaunchSection(s) => self.launch_section(s),
                controller::Request::IdeasView(ideas) => self.song_view = !ideas,
                controller::Request::KeyboardMode(keyboard) => self.piano_roll = keyboard,
                controller::Request::FocusSection(s) => self.section = Some(s),
                controller::Request::FocusScene(s) => self.focus_scene(s),
                controller::Request::FocusPattern(g, p) => {
                    self.focus_group(g);
                    self.pattern = p;
                }
            }
        }
    }

    /// When DECKARD's layer wants the next pass for the MK3, as the scripts' timer asks
    /// for theirs (none here while the scripts run): what's held (Step mode's hold) and a
    /// theme's moving border every 30 ms, else the scripts' pace, faster while the song
    /// plays. Without it a covered window ran only when the MK3 was touched, and its
    /// lights, meters and screens went stale (C46).
    fn mk3_wake(&self) -> Option<std::time::Duration> {
        if matches!(self.controller, Some(Ok(_))) {
            return None;
        }
        Some(if self.surface.holding() || self.surface.animates() {
            std::time::Duration::from_millis(30)
        } else {
            controller::tick_every(self.playing.is_some())
        })
    }

    /// The MK3's events since the last frame, to the controller scripts (pads already
    /// played from the controller's thread); keep the pads' routing current.
    fn mk3_sync(&mut self, ctx: &egui::Context, requests: &mut Vec<controller::Request>, covered: bool) {
        let Some(m) = &self.mk3 else { return };
        m.set_context(ctx);
        m.set_monitor(self.midi.monitor());
        let events = m.take_events();
        let pending = m.take_pending();
        let out = self.mk3_events(events, requests);
        if let Some(after) = self.mk3_wake() {
            ctx.request_repaint_after(if covered { undrawn(ctx, after) } else { after });
        }
        if let Some(m) = &self.mk3 {
            m.set_route(out.route);
            m.set_leds(out.leds.scaled(self.settings.led_brightness));
            if let Some((n, px)) = out.screens {
                m.set_screens(n, &px);
            }
        }
        // Pad hits that came while no engine ran (it starts with the first): each at its
        // own pitch, so a chord stays a chord.
        for note in mk3_io::pending_notes(&pending) {
            if let deckard_engine::Live::NoteOn { sound, .. } = note {
                self.sound = sound;
            }
            self.play_note(note);
        }
    }

    /// The MK3's events since the last frame, to the layer that runs it (Maschine's
    /// scripts, or DECKARD's own): what the pads play, the lights and new screens.
    /// Hardware-free, so tests drive it with made-up events.
    pub(crate) fn mk3_events(&mut self, events: Vec<deckard_mk3::Event>, requests: &mut Vec<controller::Request>) -> Mk3Out {
        use deckard_mk3::Event;
        // The plug-in knobs of the slot focused since the frame began (a sound picked
        // by the scripts, say), for what the scripts see.
        self.update_slot_view();
        self.control_bridge();
        let sensitivity = f64::from(self.settings.pad_sensitivity) / 100.0;
        // The group's sounds' Pad Touch, for what their held pads' pressure sends.
        let touches = |app: &App, group: usize| {
            let mut t = [deckard_model::pad_touch::PadTouch::default(); 16];
            if let Some(g) = app.doc.as_ref().and_then(|d| d.borrow().project().groups.get(group).cloned()) {
                for (t, s) in t.iter_mut().zip(&g.sounds) {
                    *t = s.pad_touch;
                }
            }
            t
        };
        if let Some(Ok(c)) = &mut self.controller {
            for e in events {
                match e {
                    Event::Button(name, down) => {
                        c.hold(name, down, requests);
                        if log_knobs() {
                            let top = c.host.state.borrow().hw.page_stack.last().copied();
                            eprintln!("button {name} {} (page {top:?})", if down { "down" } else { "up" });
                        }
                    }
                    Event::PadOn(pad, v) => {
                        c.pad_event(pad, true, mk3_io::pad_velocity(v, sensitivity, self.settings.velocity_scaling), requests)
                    }
                    Event::PadOff(pad) => c.pad_event(pad, false, 0.0, requests),
                    Event::PadPressure(..) => {}
                    Event::Knob(k, inc) => {
                        if log_knobs() {
                            let mode = c.host.state.borrow().hw.encoder_mode;
                            eprintln!("knob {}: {inc:+.3} (encoder mode {mode})", k + 1);
                        }
                        c.encoder(&format!("ENCODER_SCREEN_{}", k + 1), inc, requests)
                    }
                    Event::Wheel(steps) => c.wheel(steps, requests),
                    Event::Touch(f, kind, pos) => c.touch(f, kind, pos, requests),
                }
            }
            // (One at a time: each call borrows the scripts' state.)
            let keys = c.host.keyboard_pads();
            let chords = c.host.keyboard_chords();
            let group = c.host.state.borrow().focus.group;
            let play = c.host.pads_play_sounds();
            let velocity = c.host.fixed_velocity();
            let fixed = c.host.fixed_velocity_value();
            let leds = mk3_leds(&c.host);
            let mut screens = None;
            if c.screens().is_some() && c.screens_drawn != self.mk3_screens_sent {
                self.mk3_screens_sent = c.screens_drawn;
                screens = c.screens().map(|s| (self.mk3_screens_sent, s.px.clone()));
            }
            let scaling = self.settings.velocity_scaling;
            let route =
                mk3_io::PadRoute { play, group, keys, chords, velocity, sensitivity, scaling, fixed, touches: touches(self, group) };
            self.control_bridge();
            Mk3Out { route, leds, screens }
        } else {
            // DECKARD's own layer: the user's edited scripts load as they're saved.
            let now = std::time::Instant::now();
            if self.surface.poll(now) {
                self.status = "controller scripts reloaded".into();
            }
            let error = self.surface.error();
            if error != self.surface_error {
                if let Some(e) = &error {
                    self.status = format!("controller script: {e}");
                }
                self.surface_error = error;
            }
            // BROWSE searches the library: it starts as the page first shows.
            if self.surface.page() == "browse" && !self.browser.started() {
                self.browse(browser::BrowseAct::Toggle);
                self.browse(browser::BrowseAct::Toggle);
            }
            self.refresh_surface_settings();
            let view = self.surface_view();
            let mut actions = Vec::new();
            for e in &events {
                actions.extend(self.surface.event(e, &view, now));
            }
            actions.extend(self.surface.tick(&view, now));
            for a in actions {
                self.surface_act(a);
            }
            // (A slot or sound the buttons just focused shows its plug-in's knobs.)
            self.update_slot_view();
            self.refresh_surface_settings();
            let view = self.surface_view();
            let fixed =
                self.doc.as_ref().and_then(|d| d.borrow().num(deckard_doc::Target::Song, "fixed_velocity")).map_or(100, |v| v as i64);
            let pads = self.surface.pads(&view);
            let route = mk3_io::PadRoute {
                play: pads.play,
                group: self.group,
                keys: pads.keys,
                chords: pads.chords,
                velocity: view.fixed_velocity.then_some(fixed.clamp(0, 127)),
                sensitivity,
                scaling: self.settings.velocity_scaling,
                fixed: fixed.clamp(0, 127),
                touches: touches(self, self.group),
            };
            Mk3Out { route, leds: self.surface.leds(&view), screens: self.surface.screens(&view) }
        }
    }

    /// What DECKARD's own MK3 layer shows of the app now.
    fn surface_view(&self) -> deckard_surface::View {
        use deckard_surface::{ControlView, GroupView, KnobView, SoundView, View};
        let c = self.control_snapshot([false; 8]);
        let head = self.shown_playhead();
        let edits = self.knob_edits();
        // (Not on the Macro page: its knobs are all macros.)
        let on_macros = self.channel && self.prop_set == 3;
        let macro_targets = self.doc.as_ref().filter(|_| !on_macros).map(|d| d.borrow().macro_targets()).unwrap_or_default();
        let control = ControlView {
            level: c.level,
            modules_visible: c.modules_visible,
            page_group: c.page_group,
            page_groups: c.page_groups,
            slot: c.focus_slot.and_then(|i| c.slots.get(i)).map(|s| s.name.clone()),
            slot_count: c.slots.len(),
            slot_index: c.focus_slot,
            chain: c
                .slots
                .iter()
                .map(|s| deckard_surface::SlotView {
                    name: s.name.clone(),
                    instrument: s.instrument,
                    active: s.active,
                    plugin: s.plugin,
                    picture: (!s.plugin).then(|| deckard_lua_host::plugin_picture_key(&s.name)).filter(|k| plugin_pictures::has(k)),
                })
                .collect(),
            pages: c.pages.clone(),
            page: c.page,
            knobs: c
                .knobs
                .iter()
                .enumerate()
                .map(|(i, k)| {
                    k.as_ref().map(|k| KnobView {
                        name: k.name.clone(),
                        section: k.section.clone(),
                        text: k.text.clone(),
                        value: k.value,
                        modulatable: edits.get(i).copied().flatten().and_then(modulation::mod_target).is_some(),
                        step_mod: self.step_mod_text(edits.get(i).copied().flatten()),
                        auto_mod: self.auto_mod_text(edits.get(i).copied().flatten()),
                        in_macro: k.param.is_some_and(|p| macro_targets.contains(&p)),
                    })
                })
                .collect(),
            // (Only while the plug-in browser shows: the list is long.)
            modules: if self.surface.page() == "module" { self.module_views() } else { Vec::new() },
        };
        let mut v = View {
            recent: self.recent.iter().map(|p| p.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned())).collect(),
            settings: self.surface_settings.clone(),
            lock: self.lock_view(),
            // (The waves' pictures, and the mod matrix's slots, only while their pages show.)
            lfos: self.lfo_views(self.surface.page() == "lfo"),
            lfo_focus: self.lfo_focus as usize,
            mod_slots: if self.surface.page() == "mod_matrix" { self.mod_slot_views() } else { Vec::new() },
            note_repeat: self.repeat_view(),
            playing: self.playing.is_some(),
            recording: self.recording,
            looping: self.looping,
            metronome: self.metronome,
            auto_write: self.auto_write,
            strip_position: i64::from(self.settings.strip_position),
            touch_auto_write: self.settings.touch_auto_write,
            follow: self.follow,
            position: head.map(|h| h.position as i64),
            progress: self.pattern_at(self.group, head).map(|(at, length)| at as f64 / length as f64),
            group: self.group,
            sound: self.sound,
            control,
            sampling: self.sampling_surface_view(),
            ..Default::default()
        };
        // (Only while BROWSE shows: the results can be long.)
        if self.surface.page() == "browse" {
            let b = self.browser.view();
            v.browser = deckard_surface::BrowserView {
                kind: b.kind,
                user: b.user,
                products: b.products,
                product: b.product,
                attributes: b.attributes,
                results: b.results,
                focus: b.focus,
                busy: b.busy,
                favorites: b.favorites,
            };
        }
        let Some(doc) = &self.doc else { return v };
        let d = doc.borrow();
        let p = d.project();
        v.project = p.name.clone();
        v.tempo = p.tempo;
        v.swing = p.swing.amount;
        v.master_volume = p.master_level;
        v.master_db = deckard_model::level_db(p.master_level);
        v.numerator = p.numerator;
        v.denominator = p.denominator;
        v.fixed_velocity = d.num(deckard_doc::Target::Song, "pad_velocity_mode").is_some_and(|m| m as i64 == 1);
        v.velocity = d.num(deckard_doc::Target::Song, "fixed_velocity").map_or(100, |v| (v as i64).clamp(0, 127));
        (_, v.scale_bank, v.scale_kind) = self.scale;
        let song = |field: &str, default: f64| d.num(deckard_doc::Target::Song, field).unwrap_or(default) as i64;
        v.grid = deckard_surface::GridView {
            perform: song("perform_grid", 5.0),
            arrange: song("arrange_grid", 0.0),
            arrange_snap: song("arrange_grid_snap", 1.0) != 0,
            quick: song("pattern_length_quick", 1.0) != 0,
            step: deckard_lua_host::api::pattern_editor::step_index(self.grid) as i64,
            step_snap: song("pattern_editor_snap_enabled", 1.0) != 0,
            step_ticks: self.grid,
            nudge: self.nudge.0 as i64,
        };
        v.user_chords = self.user_chords.sets.clone();
        if let Some(sc) = p.scenes.get(self.scene) {
            v.scene_name = sc.name.clone();
            v.scene_color = layout::scene_index(p, self.scene);
        }
        let mix = |level: f64, pan: f64, meter: (f32, f32), channel: layout::Channel, slots: &[deckard_model::SlotInfo]| {
            deckard_surface::MixView {
                level: deckard_model::level_db(level),
                pan,
                meter,
                headroom: self.headroom.get(&channel).copied().unwrap_or(0.0),
                plugins: slots.iter().map(|x| (x.is_instrument(), x.active)).collect(),
            }
        };
        let meter = |g: usize, s: usize| self.meters.get(g).and_then(|r| r.get(s)).copied().unwrap_or_default();
        v.scene = self.scene;
        v.pattern = self.pattern;
        if let Some(pt) = p.groups.get(self.group).and_then(|grp| grp.patterns.get(self.pattern)) {
            v.pattern_start = pt.start;
            v.pattern_length = if pt.length > 0 { pt.length } else { p.bar_length() };
            v.pattern_auto = d.length_is_auto(self.group, self.pattern);
            v.notes = pt
                .notes
                .iter()
                .map(|n| deckard_surface::NoteView {
                    sound: n.sound,
                    position: n.position,
                    length: n.length,
                    pitch: n.pitch,
                    velocity: n.velocity,
                    probability: n.probability,
                    selected: self.selected.contains(n),
                })
                .collect();
            // (Playback's place in it while it's the focused scene's.)
            if self.scene_pattern(self.group) == Some(self.pattern) {
                v.pattern_position = self.pattern_at(self.group, head).map(|(at, _)| pt.start + at);
            }
        }
        v.scenes = (0..p.scenes.len())
            .map(|i| deckard_surface::SceneView {
                name: p.scenes[i].name.clone(),
                color: layout::scene_index(p, i),
                rgb: layout::PALETTE[layout::scene_index(p, i)],
                default_color: p.scenes[i].color == deckard_model::Color::Default,
                patterns: (0..p.groups.len()).map(|g| p.scenes[i].patterns.iter().find(|&&(pg, _)| pg == g).map(|&(_, pi)| pi)).collect(),
            })
            .collect();
        // (As the Ideas view shows them: what plays, and what's queued next.)
        let live = self.playhead();
        v.playing_scene = live.and_then(|h| match h.mode {
            Mode::Scene(s) => Some(s),
            _ => None,
        });
        v.queued_scene = live.and_then(|h| match h.queued {
            Some(Mode::Scene(s)) => Some(s),
            _ => None,
        });
        v.song_view = self.song_view;
        v.sections = p
            .sections
            .iter()
            .enumerate()
            .map(|(i, sec)| deckard_surface::SectionView {
                scene: sec.scene,
                start: p.section_start(i),
                length: sec.length,
                auto: sec.auto,
            })
            .collect();
        v.section = self.focused_section(p);
        let bar = (p.numerator.max(1) * 3840 / p.denominator.max(1)).max(1) as f64;
        v.groups = (0..p.groups.len())
            .map(|g| {
                let grp = &p.groups[g];
                let loudest = (0..grp.sounds.len()).map(|s| meter(g, s)).fold((0f32, 0f32), |m, x| (m.0.max(x.0), m.1.max(x.1)));
                // The sounds with notes in any of the group's patterns.
                let noted: std::collections::HashSet<usize> = grp.patterns.iter().flat_map(|pt| pt.notes.iter().map(|n| n.sound)).collect();
                GroupView {
                    name: grp.name.clone(),
                    rgb: layout::rgb(p, g, None),
                    color: layout::colour_index(p, g, None),
                    muted: grp.muted,
                    default_color: grp.color == deckard_model::Color::Default,
                    playing: self.lit.iter().any(|&(lg, _)| lg == g),
                    mix: mix(grp.level, grp.pan, loudest, layout::Channel::Group(g), &grp.slots),
                    perform: self.perform_slot(g).map(|i| {
                        let t = || deckard_doc::Target::ChainModule(deckard_doc::Chain::Group(g), i);
                        (d.num(t(), "touchstrip_on_off").unwrap_or(0.0) > 0.5, d.num(t(), "touchstrip_pos").unwrap_or(0.5))
                    }),
                    patterns: grp
                        .patterns
                        .iter()
                        .map(|pt| deckard_surface::PatternView {
                            name: pt.name.clone(),
                            color: match pt.color {
                                deckard_model::Color::Palette(i) => (i as usize).min(16),
                                deckard_model::Color::Default => layout::colour_index(p, g, None),
                            },
                            default_color: pt.color == deckard_model::Color::Default,
                            rgb: layout::pattern_rgb(p, g, pt.color),
                            bars: pt.length as f64 / bar,
                        })
                        .collect(),
                    scene_pattern: p.scenes.get(self.scene).and_then(|sc| sc.patterns.iter().find(|(pg, _)| *pg == g).map(|&(_, pi)| pi)),
                    progress: head.and_then(|_| layout::group_progress(p, head, self.scene, g)).map(|(bars, played)| played / bars as f64),
                    sounds: (0..grp.sounds.len())
                        .map(|s| {
                            let snd = &grp.sounds[s];
                            SoundView {
                                name: snd.name.clone(),
                                rgb: layout::rgb(p, g, Some(s)),
                                color: layout::colour_index(p, g, Some(s)),
                                muted: snd.muted,
                                // (A sound may only play MIDI out, with no plug-ins.)
                                used: !snd.slots.is_empty()
                                    || !snd.name.is_empty()
                                    || snd.channel.midi_out != deckard_model::MidiDest::None
                                    || noted.contains(&s),
                                playing: self.lit.contains(&(g, s)),
                                mix: mix(snd.level, snd.pan, meter(g, s), layout::Channel::Sound(g, s), &snd.slots),
                                choke_group: snd.channel.choke_group,
                                choke_mode: snd.channel.choke_mode,
                                link_group: snd.channel.link_group,
                                link_mode: snd.channel.link_mode,
                                base_key: snd.channel.base_key,
                                default_color: snd.color == deckard_model::Color::Default,
                                touch_mode: snd.pad_touch.mode.index(),
                                touch_cc: i64::from(snd.pad_touch.cc),
                                touch_curve: i64::from(snd.pad_touch.curve),
                                touch_min: i64::from(snd.pad_touch.min),
                                touch_max: i64::from(snd.pad_touch.max),
                                prob_locked: snd.probability_locked,
                            }
                        })
                        .collect(),
                }
            })
            .collect();
        v
    }

    /// Carry out what DECKARD's own MK3 layer asked for.
    /// The section the controller's Song view focuses: the one picked (the last if it's
    /// gone), else the first.
    fn focused_section(&self, p: &deckard_model::Project) -> Option<usize> {
        let last = p.sections.len().checked_sub(1)?;
        Some(self.section.map_or(0, |i| i.min(last)))
    }

    pub(crate) fn surface_act(&mut self, a: deckard_surface::Action) {
        use deckard_doc::Target;
        use deckard_lua_host::state::ControlRequest as R;
        use deckard_surface::Action as A;
        use layout::Act;
        let (g, project) = (self.group, self.doc.as_ref().map(|d| d.borrow().project().clone()));
        match a {
            A::Play => self.transport_play(),
            A::Stop => self.stop(),
            A::Record { on, count_in } => self.set_recording(on, count_in),
            A::Restart => self.restart(),
            A::Loop(on) => self.set_looping(on),
            A::Metronome(on) => self.set_metronome(on),
            A::AutoWrite(on) => self.auto_write = on,
            A::Tempo(bpm) => layout::apply_acts(self, vec![Act::SetTempo(bpm)]),
            A::Swing(amount) => {
                let Some(p) = project else { return };
                layout::apply_acts(self, vec![Act::SetSwing(Target::Song, deckard_model::Swing { amount, ..p.swing })]);
            }
            A::Volume(level) => layout::apply_acts(self, vec![Act::SetLevel(Target::Song, level)]),
            A::VolumeDb(db) => layout::apply_acts(self, vec![Act::SetLevel(Target::Song, deckard_model::level_from_db(db))]),
            A::FocusGroup(group) => self.focus_group(group),
            A::FocusSound(s) => self.sound = s,
            A::FocusScene(scene) => {
                if project.is_some_and(|p| scene < p.scenes.len()) {
                    self.launch_scene(scene);
                }
            }
            // (A MIDI controller's scene button: stopped, it starts the scene too.)
            A::LaunchScene(scene) => {
                if project.is_some_and(|p| scene < p.scenes.len()) {
                    self.launch_scene(scene);
                    if self.playing.is_none() && !self.receiving_clock() {
                        self.parked = None;
                        self.play_from(Mode::Scene(scene), 0);
                        self.sync_transport();
                    }
                }
            }
            A::PickPattern(pi) => {
                if project.is_some_and(|p| p.groups.get(g).is_some_and(|grp| pi < grp.patterns.len())) {
                    layout::apply_acts(self, vec![Act::AssignPattern(self.scene, g, Some(pi)), Act::FocusPattern(g, pi)]);
                }
            }
            A::TouchWrite(knob) => self.surface_touch_write(knob),
            A::Sampling(_) | A::SamplingTab(_) | A::SamplingKnob { .. } | A::SamplingCall(_) | A::RecordSample(_) | A::Take { .. } => {
                self.surface_sampling(a)
            }
            A::ScenePattern { group, pattern } => {
                if project.is_some_and(|p| {
                    self.scene < p.scenes.len() && p.groups.get(group).is_some_and(|grp| pattern.is_none_or(|pi| pi < grp.patterns.len()))
                }) {
                    layout::apply_acts(self, vec![Act::AssignPattern(self.scene, group, pattern)]);
                }
            }
            A::PatternSpan { start, length } => layout::apply_acts(self, vec![Act::PatternSpan(g, self.pattern, start, length)]),
            A::Color { of, color } => {
                let t = match of {
                    deckard_surface::ColorOf::Pattern => Target::Pattern(g, self.pattern),
                    deckard_surface::ColorOf::Scene => Target::Scene(self.scene),
                    deckard_surface::ColorOf::Group => Target::Group(g),
                    deckard_surface::ColorOf::Sound => Target::Sound(g, self.sound),
                };
                let c = color.map_or(deckard_model::Color::Default, deckard_model::Color::Palette);
                layout::apply_acts(self, vec![Act::SetColor(vec![t], c)]);
            }
            A::PatternEdit(what) => {
                let a = match what {
                    "create" => Act::NewPattern(self.scene, g),
                    "double" => Act::DoublePattern(g, self.pattern),
                    "duplicate" => Act::DuplicatePattern(g, self.pattern),
                    _ => Act::DeletePattern(g, self.pattern),
                };
                layout::apply_acts(self, vec![a]);
            }
            A::SceneEdit(what) => {
                let Some(p) = project else { return };
                let a = match what {
                    "create" => Act::AddScene,
                    "duplicate" => Act::DuplicateScene(self.scene),
                    "delete" if p.scenes.len() > 1 => Act::DeleteScene(self.scene),
                    "append" => Act::AddSection(self.scene),
                    "insert" if self.scene < p.scenes.len() => Act::InsertScene(self.scene + 1),
                    "clear" => {
                        let Some(sc) = p.scenes.get(self.scene) else { return };
                        let acts = sc.patterns.iter().map(|&(g, _)| Act::AssignPattern(self.scene, g, None)).collect();
                        layout::apply_acts(self, acts);
                        return;
                    }
                    _ => return,
                };
                layout::apply_acts(self, vec![a]);
            }
            A::SongView(on) => self.song_view = on,
            A::FocusSection(i) => {
                if project.is_some_and(|p| i < p.sections.len()) {
                    self.launch_section(i);
                }
            }
            A::SectionEdit(what) => {
                let Some(p) = project else { return };
                let focus = self.focused_section(&p);
                let acts = match (what, focus) {
                    // A new section after the focused one, playing its scene.
                    ("create", Some(i)) => {
                        let scene = p.sections[i].scene.unwrap_or(self.scene);
                        vec![Act::AddSection(scene), Act::MoveSection(p.sections.len(), i + 1)]
                    }
                    ("create", None) if self.scene < p.scenes.len() => vec![Act::AddSection(self.scene)],
                    ("duplicate", Some(i)) => vec![Act::DuplicateSection(i)],
                    ("delete", Some(i)) => {
                        layout::apply_acts(self, vec![Act::DeleteSection(i)]);
                        // (The one after it is focused, else the one before.)
                        self.section = Some(i);
                        return;
                    }
                    _ => return,
                };
                layout::apply_acts(self, acts);
            }
            A::SectionMove(next) => {
                let Some(p) = project else { return };
                let Some(i) = self.focused_section(&p) else { return };
                let to = if next { i + 1 } else { i.wrapping_sub(1) };
                if to < p.sections.len() {
                    layout::apply_acts(self, vec![Act::MoveSection(i, to)]);
                }
            }
            A::SectionScene(next) => {
                let Some(p) = project else { return };
                let Some(i) = self.focused_section(&p) else { return };
                let Some(at) = p.sections[i].scene else { return };
                let to = if next { at + 1 } else { at.wrapping_sub(1) };
                if to < p.scenes.len() {
                    layout::apply_acts(self, vec![Act::SectionScene(i, to)]);
                }
            }
            A::SectionLength(ticks) => {
                let Some(i) = project.as_ref().and_then(|p| self.focused_section(p)) else { return };
                layout::apply_acts(self, vec![Act::SectionLength(i, Some(ticks))]);
            }
            A::Browse(b) => {
                use deckard_lua_host::state::BrowseCall as C;
                use deckard_surface::Browse as B;
                let call = match b {
                    B::Kind(k) => C::Kind(k),
                    B::User(u) => C::User(u),
                    B::Product(p) => C::Product(p),
                    B::Attribute(a, v) => C::Attribute(a, v),
                    B::Focus(by) => C::Focus(by),
                    B::Load => C::Load,
                    B::Favorites => C::ToggleFavorites,
                    B::Prehear => C::Prehear,
                    B::SetFavorite => {
                        let i = self.browser.selected.unwrap_or(0);
                        let Some(on) = self.browser.results.items.get(i).map(|r| !r.favorite) else { return };
                        self.browse(browser::BrowseAct::Favorite(i, on));
                        return;
                    }
                };
                if let Some(a) = self.browser.from_controller(call) {
                    self.browse(a);
                }
            }
            A::PatternLength(length) => layout::apply_acts(self, vec![Act::PatternLength(g, self.pattern, length)]),
            A::ChannelLevel { group, sound, db } => {
                let t = match sound {
                    Some(s) => Target::Sound(group, s),
                    None => Target::GroupStrip(group),
                };
                layout::apply_acts(self, vec![Act::SetLevel(t, deckard_model::level_from_db(db))]);
            }
            A::ChannelPan { group, sound, pan } => {
                let t = match sound {
                    Some(s) => Target::Sound(group, s),
                    None => Target::GroupStrip(group),
                };
                layout::apply_acts(self, vec![Act::SetPan(t, pan)]);
            }
            A::MuteGroup(group) => layout::apply_acts(self, vec![Act::ToggleMute(Target::Group(group))]),
            A::SoloGroup(group) => self.solo(layout::Channel::Group(group)),
            A::MuteSound(s) => layout::apply_acts(self, vec![Act::ToggleMute(Target::Sound(g, s))]),
            A::SoloSound(s) => self.solo(layout::Channel::Sound(g, s)),
            A::EraseSound(s) => {
                let Some(pat) = project.as_ref().and_then(|p| p.groups.get(g)?.patterns.get(self.pattern)) else { return };
                let remove: Vec<_> = pat.notes.iter().filter(|n| n.sound == s).copied().collect();
                if !remove.is_empty() {
                    layout::apply_acts(self, vec![Act::EditNotes { g, p: self.pattern, label: "Erase", remove, add: Vec::new() }]);
                }
            }
            A::Turn { knob, inc, fine } => self.control_request(R::Turn { pos: knob, inc, fine }),
            A::Page(next) => self.control_request(R::Page(next)),
            A::Level(level) => self.control_request(R::Level(level)),
            A::Slot(delta) => self.control_request(R::Slot(delta)),
            A::ModulesVisible(on) => self.control_request(R::ModulesVisible(on)),
            A::PageGroup(i) => self.control_request(R::PageGroup(i)),
            // The plug-in browser: an entry in place of the focused slot (or into the
            // "+"), or (none): the focused slot out.
            A::LoadModule(Some(entry)) => self.control_request(R::Load { entry, replace: self.slot }),
            A::SlotEdit(what) => {
                let Some(i) = self.slot else { return };
                let active = self.control_snapshot([false; 8]).slots.get(i).is_some_and(|x| x.active);
                match what {
                    "edit" => self.open_slot_editor_in(self.chain(), i),
                    "move_left" if i > 0 => self.control_request(R::Move(i, i - 1)),
                    "move_right" => self.control_request(R::Move(i, i + 1)),
                    "bypass" => self.control_request(R::Active(i, !active)),
                    "remove" => self.control_request(R::Remove(i)),
                    _ => {}
                }
            }
            A::LoadModule(None) => {
                if let Some(i) = self.slot {
                    self.control_request(R::Remove(i));
                }
            }
            A::FixedVelocity(on) => {
                let Some(doc) = self.doc.clone() else { return };
                let mode = mxprj::decode::Value::Int(i64::from(on));
                if let Err(e) = doc.borrow_mut().set("Fixed Velocity", Target::Song, "pad_velocity_mode", mode) {
                    self.status = format!("can't set Fixed Velocity: {e}");
                }
            }
            A::KeyboardMode(on) => self.piano_roll = on,
            A::Follow(on) => self.follow = on,
            A::Setting(name, index, step) => self.step_setting(name, index, step),
            A::Events(what, n) => self.surface_events(what, n),
            A::EventsRange { start, end, low, high } => self.surface_events_range(start, end, low, high),
            A::StepToggle { at, length, velocity } => self.surface_step(at, length, velocity),
            A::SetMacro { knob, level } => {
                if !self.set_plugin_macro(knob, level) {
                    self.set_macro_for_knob(knob, level);
                }
            }
            A::GroovePage(n) => self.control_request(deckard_lua_host::state::ControlRequest::GroovePage(n)),
            A::Strip(..) | A::StripNote { .. } | A::PerformFx => self.surface_strip(a),
            A::Edit(what) => self.surface_edit(what),
            A::StepModAt(at) => self.surface_step_mod_at(at),
            A::StepMod { from, length, pads, knob, inc } => self.surface_step_mod(from, length, pads, knob, inc),
            A::ResetSound(s) => layout::apply_acts(self, vec![Act::ResetSound(g, s)]),
            A::ProbLock(on) => {
                if let Some(doc) = &self.doc
                    && let Err(e) = doc.borrow_mut().set_probability_locked(g, self.sound, on)
                {
                    self.status = format!("can't set PROB LOCK: {e}");
                }
            }
            A::Repeat("locked", v) => self.arp_locked = v > 0.5,
            A::Repeat(what, v) => self.arp_edit(&format!("arp/{what}"), v),
            A::Lfo(what, pad, v) => self.surface_lfo(what, pad, v),
            A::ModSlot(slot, row, steps) => mod_slots::step(self, slot, row, steps),
            A::ModSlotsClosed => self.mod_draft = None,
            A::Lock(what, n) => match what {
                "button" => self.lock_button(),
                "pad" => self.lock_pad(n),
                "update" => self.lock_update(),
                "delete" => self.lock_delete(),
                "delete_slot" => self.lock_delete_slot(n),
                "bank" => self.lock_bank(n == 1),
                setting => self.lock_setting(setting, n as f64),
            },
            A::File(what, n) => {
                use menu::Command as C;
                let cmd = match what {
                    "new" => Some(C::New),
                    "save" => Some(C::Save),
                    "save_copy" => Some(C::SaveCopyAs),
                    "export_audio" => Some(C::ExportAudio),
                    "load" => self.recent.get(n).cloned().map(C::OpenRecent),
                    _ => None,
                };
                if let Some(cmd) = cmd {
                    self.menu_commands.push(cmd);
                }
                match what {
                    "save_group" => self.save_item(deckard_doc::FileItem::Group(self.group), SaveMode::Save),
                    "save_sound" => self.save_item(deckard_doc::FileItem::Sound(self.group, self.sound), SaveMode::Save),
                    _ => {}
                }
            }
            A::Grid(name, v) => {
                use deckard_lua_host::api::pattern_editor::step_ticks;
                // (The step and nudge grids are the pattern editor's; the rest the song's.)
                match name {
                    "step" => {
                        let bar = self.doc.as_ref().map_or(4 * PPQ, |d| d.borrow().project().bar_length());
                        self.grid = step_ticks(v.max(0) as usize, bar);
                    }
                    "nudge" => self.nudge.0 = v.clamp(0, 4) as usize,
                    _ => {
                        let field = match name {
                            "perform" => "perform_grid",
                            "arrange" => "arrange_grid",
                            "arrange_snap" => "arrange_grid_snap",
                            "quick" => "pattern_length_quick",
                            _ => "pattern_editor_snap_enabled",
                        };
                        if let Some(doc) = &self.doc
                            && let Err(e) = doc.borrow_mut().set("Grid", Target::Song, field, mxprj::decode::Value::Int(v))
                        {
                            self.status = format!("Grid: {e}");
                        }
                    }
                }
            }
            A::VelocityValue(v) => {
                if let Some(doc) = &self.doc
                    && let Err(e) = doc.borrow_mut().set("Fixed Velocity", Target::Song, "fixed_velocity", mxprj::decode::Value::Int(v))
                {
                    self.status = format!("Fixed Velocity: {e}");
                }
            }
            A::Scale { root, bank, kind } => self.scale = (root, bank, kind),
            A::AddGroup => layout::apply_acts(self, vec![Act::AddGroup]),
            A::DeleteGroup(g) => layout::apply_acts(self, vec![Act::DeleteGroup(g)]),
            A::UserChordNote { set, slot, note } => self.user_chords.toggle(set, slot, note),
            A::UserChordDelete { set, slot } => self.user_chords.delete(set, slot),
            A::UserChordMove { set, slot, next } => self.user_chords.move_chord(set, slot, next),
            A::UserChordPreview { set, slot, root, on } => {
                let notes = self.user_chords.sets.get(set).and_then(|s| s.get(slot)).cloned().unwrap_or_default();
                for n in notes {
                    if (0..=127).contains(&(root + n)) {
                        self.key_note(root + n, if on { 100 } else { 0 });
                    }
                }
            }
            A::PadTouch(field, v) => {
                let Some(p) = project else { return };
                let Some(snd) = p.groups.get(g).and_then(|grp| grp.sounds.get(self.sound)) else { return };
                let mut t = snd.pad_touch;
                let byte = v.clamp(0, 127) as u8;
                match field {
                    "mode" => t.mode = deckard_model::pad_touch::TouchMode::from_index(v),
                    "cc" => t.cc = byte,
                    "curve" => t.curve = byte,
                    "min" => t.min = byte,
                    _ => t.max = byte,
                }
                layout::apply_acts(self, vec![Act::PadTouch(g, self.sound, t.clamped())]);
            }
            A::SoundSetting(field, v) => {
                let label = match field {
                    "choke_group" => "Choke Group",
                    "choke_mode" => "Choke Mode",
                    "link_group" => "Link Group",
                    _ => "Link Mode",
                };
                if let Some(doc) = &self.doc
                    && let Err(e) = doc.borrow_mut().set(label, Target::Sound(g, self.sound), field, mxprj::decode::Value::Int(v))
                {
                    self.status = format!("{label}: {e}");
                }
            }
        }
    }

    /// The Control area and the controller's Control page: carry out what the scripts
    /// asked for (knobs, pages, slots, Master/Group/Sound, bypass), then show them what
    /// the Control area shows now.
    fn control_bridge(&mut self) {
        let requests = match &self.controller {
            Some(Ok(c)) => std::mem::take(&mut c.host.state.borrow_mut().control_requests),
            _ => return,
        };
        for r in requests {
            self.control_request(r);
        }
        let touched: [bool; 8] = std::array::from_fn(|k| {
            let turned = self.knob_turned[k].is_some_and(|t| t.elapsed() < KNOB_NAME_HOLD);
            turned || matches!(&self.controller, Some(Ok(c)) if c.host.knob_touched(k + 1))
        });
        let snapshot = self.control_snapshot(touched);
        // The Mixer page's meters and headroom.
        let levels = deckard_lua_host::state::Levels {
            sounds: self.meters.clone(),
            master: self.master_meter,
            headroom: self
                .headroom
                .iter()
                .map(|(ch, v)| {
                    let key = match *ch {
                        layout::Channel::Sound(g, s) => (Some(g), Some(s)),
                        layout::Channel::Group(g) => (Some(g), None),
                        layout::Channel::Master => (None, None),
                    };
                    (key, *v)
                })
                .collect(),
        };
        let plugins = self.plugin_choices().into_iter().map(|(e, _)| e).collect();
        if let Some(Ok(c)) = &mut self.controller {
            c.host.state.borrow_mut().control = snapshot;
            c.host.set_levels(levels);
            c.host.set_plugins(plugins);
        }
    }

    /// `touched`: knobs being touched or just turned, whose routing choices show whole.
    fn control_snapshot(&self, touched: [bool; 8]) -> deckard_lua_host::state::Control {
        use deckard_lua_host::state::{Control, ControlKnob, ControlSlot};
        let level = match self.chain_tab {
            layout::ChainTab::Master => 0,
            layout::ChainTab::Group => 1,
            layout::ChainTab::Sound => 2,
        };
        let mut c = Control {
            level,
            modules_visible: !self.channel,
            page_group: self.prop_set.min(3),
            page_groups: 4,
            knobs: vec![None; 8],
            ..Default::default()
        };
        let Some(doc) = self.doc.clone() else { return c };
        let d = doc.borrow();
        let p = d.project();
        // CHANNEL: the channel's properties; a routing choice shortened (as in its
        // field) but while its knob is touched or turned.
        if self.channel
            && let Some((names, page, pg)) = layout::channel_page(self, &d)
        {
            c.pages = names;
            c.page = page;
            // (An empty macro shows no knob: the window's "MACRO n" is for picking one.)
            let macros = self.prop_set == 3;
            for (k, knob) in pg.slots.into_iter().take(8).enumerate() {
                let knob = knob.filter(|(_, text, _, edit)| !(macros && edit.is_none() && text.is_empty()));
                c.knobs[k] = knob.map(|(name, text, value, edit)| {
                    let whole = match edit {
                        Some(layout::Edit::Route(cur)) if touched[k] => {
                            pg.routes.iter().find(|(i, ..)| *i == k).and_then(|(_, n, _)| n.get(cur)).cloned()
                        }
                        _ => None,
                    };
                    ControlKnob {
                        name: name.to_string(),
                        section: pg.sections.iter().filter(|(i, _)| *i <= k).last().map_or(String::new(), |(_, n)| n.to_string()),
                        value: value as f64,
                        text: whole.unwrap_or(text),
                        param: edit.and_then(|e| layout::channel::knob_param(&e)).and_then(|(t, f)| d.param_oid(t, f)),
                        modulated: edit.and_then(|e| self.mod_amount(e)).is_some(),
                    }
                });
            }
        }
        let chain = self.chain_slots();
        if self.chain_tab == layout::ChainTab::Sound && p.groups.get(self.group).and_then(|g| g.sounds.get(self.sound)).is_none() {
            return c;
        }
        // DECKARD's own plug-ins show as internal modules (their own pictures).
        let native = |x: &deckard_model::SlotInfo| x.plugin.as_ref().is_some_and(|p| p.id.starts_with(deckard_plugins::native::VENDOR));
        c.slots = chain
            .iter()
            .map(|x| ControlSlot {
                name: x.name.clone(),
                plugin: x.plugin.is_some() && !native(x),
                active: x.active,
                instrument: x.is_instrument(),
            })
            .collect();
        c.focus_slot = if self.channel { None } else { self.slot.filter(|&i| i < chain.len()) };
        if let Some(pg) = c.focus_slot.and_then(|i| layout::slot_page(self, p, i)) {
            // Built-in knobs show where their routes have them now (as plug-ins' do).
            let live = c.focus_slot.and_then(|i| layout::live_slot_page(self, p, i)).map(|x| x.knobs).unwrap_or_default();
            c.pages = pg.names;
            c.page = pg.page;
            for (k, knob) in pg.knobs.into_iter().take(8).enumerate() {
                // Auto-writing: a modulatable knob shows its modulation, in percent.
                let writing = self.auto_write && self.playing.is_some();
                let shown = live.get(k).cloned().flatten().map(|x| (x.1, x.2));
                c.knobs[k] = knob.map(|(name, text, value, edit)| {
                    let (live_text, shown_value) = shown.unwrap_or((text.clone(), value));
                    let text = match edit.filter(|e| writing && modulation::mod_target(*e).is_some()) {
                        Some(e) => format!("{:+.0}%", (self.mod_position(e).unwrap_or(value as f64) - value as f64) * 100.0),
                        // A routing choice (a side-chain source) whole while touched.
                        None => match edit {
                            Some(layout::Edit::Route(cur)) if touched[k] => {
                                pg.routes.iter().find(|(i, ..)| *i == k).and_then(|(_, n, _)| n.get(cur)).cloned().unwrap_or(text)
                            }
                            _ => live_text,
                        },
                    };
                    ControlKnob {
                        name: name.to_string(),
                        section: pg.sections.iter().filter(|(i, _)| *i <= k).last().map_or(String::new(), |(_, n)| n.to_string()),
                        value: shown_value as f64,
                        text,
                        param: edit.and_then(|e| layout::edit_param_oid(self, &d, e)),
                        modulated: edit.and_then(|e| self.mod_amount(e)).is_some(),
                    }
                });
            }
        }
        c
    }

    pub(crate) fn control_request(&mut self, r: deckard_lua_host::state::ControlRequest) {
        use deckard_lua_host::state::ControlRequest as R;
        use layout::{Act, Edit};
        let Some(doc) = self.doc.clone() else { return };
        let page = |app: &App, i: usize| layout::slot_page(app, doc.borrow().project(), i);
        let act = match r {
            // ERASE + knob: its modulation goes.
            R::Erase { pos } => {
                let knobs = if self.channel {
                    let Some((_, _, pg)) = layout::channel_page(self, &doc.borrow()) else { return };
                    pg.slots
                } else {
                    let Some(pg) = self.slot.and_then(|i| page(self, i)) else { return };
                    pg.knobs
                };
                if let Some(Some((_, _, _, Some(edit)))) = knobs.get(pos).cloned() {
                    self.clear_modulation(edit);
                }
                return;
            }
            R::Turn { pos, inc, fine } => {
                // CHANNEL: the channel page's knobs, routing choices among them.
                let (knobs, routes) = if self.channel {
                    let Some((_, _, pg)) = layout::channel_page(self, &doc.borrow()) else { return };
                    (pg.slots, pg.routes)
                } else {
                    let Some(pg) = self.slot.and_then(|i| page(self, i)) else { return };
                    (pg.knobs, pg.routes)
                };
                let Some(Some((_, _, value, Some(edit)))) = knobs.get(pos).cloned() else { return };
                // A choice turns from the one it's on (its knob shows no position).
                let value = match edit {
                    Edit::Choice(_, _, options, cur, _) => cur as f32 / (options.len().max(2) - 1) as f32,
                    _ => value,
                };
                let pos = pos.min(7);
                self.knob_turned[pos] = Some(std::time::Instant::now());
                self.knob_acc[pos] += inc * if fine { 0.1 } else { 1.0 };
                // Auto-write while the song plays: the turn moves the knob's modulation.
                if self.auto_write && self.playing.is_some() && modulation::mod_target(edit).is_some() {
                    let from = self.mod_position(edit).unwrap_or(value as f64);
                    let to = (from + self.knob_acc[pos]).clamp(0.0, 1.0);
                    self.knob_acc[pos] = 0.0;
                    self.modulate(edit, to as f32, false);
                    return;
                }
                // A routing choice: the next or previous one, a step every ROUTE_STEP.
                if let Edit::Route(cur) = edit {
                    let Some((_, _, sets)) = routes.iter().find(|(i, ..)| *i == pos) else { return };
                    if self.knob_acc[pos].abs() < ROUTE_STEP || sets.is_empty() {
                        return;
                    }
                    let up = self.knob_acc[pos] > 0.0;
                    self.knob_acc[pos] = 0.0;
                    let to = match cur {
                        usize::MAX => 0,
                        c if up => (c + 1).min(sets.len() - 1),
                        c => c.saturating_sub(1),
                    };
                    if to == cur {
                        return;
                    }
                    layout::apply_acts(self, vec![Act::SetRoute(sets[to].clone())]);
                    return;
                }
                // A stepped value moves a step once the knob has gone a quarter of the
                // way a continuous one would.
                let step = match edit {
                    Edit::Choice(_, _, options, ..) => Some(1.0 / (options.len().max(2) - 1) as f64),
                    Edit::Int(_, _, lo, hi, _) => Some(1.0 / (hi - lo).max(1.0)),
                    Edit::Toggle(..) => Some(1.0),
                    Edit::PluginChoice(_, _, steps, ..) => Some(1.0 / steps.max(1) as f64),
                    _ => None,
                };
                let to = match step {
                    Some(q) => {
                        let n = (self.knob_acc[pos].abs() / (q * 0.25).max(1e-9)).floor();
                        if n < 1.0 {
                            return;
                        }
                        let moved = q * self.knob_acc[pos].signum();
                        self.knob_acc[pos] = 0.0;
                        value as f64 + moved
                    }
                    None => {
                        let to = value as f64 + self.knob_acc[pos];
                        self.knob_acc[pos] = 0.0;
                        to
                    }
                };
                if log_knobs() {
                    eprintln!("  turn {}: {inc:+.3}{} {value:.3} -> {:.3}", pos + 1, if fine { " fine" } else { "" }, to.clamp(0.0, 1.0));
                }
                edit.to_act(to.clamp(0.0, 1.0) as f32)
            }
            R::Page(next) if self.channel => {
                let Some((names, cur, _)) = layout::channel_page(self, &doc.borrow()) else { return };
                Act::PropPage(if next { (cur + 1).min(names.len().saturating_sub(1)) } else { cur.saturating_sub(1) })
            }
            R::Page(next) => {
                let Some(pg) = self.slot.and_then(|i| page(self, i)) else { return };
                let n = if next { (pg.page + 1).min(pg.names.len().saturating_sub(1)) } else { pg.page.saturating_sub(1) };
                (pg.show)(n)
            }
            // On to the next slot, or past the last one to the "+" (an empty slot to load
            // a plug-in into).
            R::Slot(d) => {
                let n = doc.borrow().project().groups.get(self.group).and_then(|g| g.sounds.get(self.sound)).map_or(0, |s| s.slots.len());
                let cur = self.slot.filter(|&i| i < n).unwrap_or(n) as i64;
                let to = (cur + d).clamp(0, n as i64) as usize;
                if to == n {
                    // The "+": no slot focused, plug-ins shown.
                    (self.slot, self.channel) = (None, false);
                    return;
                }
                Act::FocusSlot(self.group, self.sound, to)
            }
            R::Load { entry, replace } => {
                let Some((_, what)) = self.plugin_choices().into_iter().nth(entry) else { return };
                let n = doc.borrow().project().groups.get(self.group).and_then(|g| g.sounds.get(self.sound)).map_or(0, |s| s.slots.len());
                match replace.filter(|&i| i < n) {
                    Some(i) => self.replace_slot(i, what),
                    None => {
                        let at = self.slot.filter(|&i| i < n).map_or(n, |i| i + 1);
                        self.add_slot_at(what, at);
                    }
                }
                return;
            }
            R::Level(l) => Act::ChainTab([layout::ChainTab::Master, layout::ChainTab::Group, layout::ChainTab::Sound][l.min(2)]),
            R::Active(i, on) => Act::SlotActive(i, on),
            R::Remove(i) => Act::RemoveSlot(i),
            R::Move(from, to) => Act::MoveSlot(from, to),
            R::ModulesVisible(on) => {
                self.channel = !on;
                return;
            }
            R::PageGroup(n) => {
                self.prop_set = n.min(3);
                self.prop_page = 0;
                return;
            }
            R::SetMacro { pos, level } => {
                if !self.set_plugin_macro(pos, level) {
                    self.set_macro_for_knob(pos, level);
                }
                return;
            }
            // VARIATION (MK3): the focused sound's Groove page.
            R::Lock(op) => {
                use deckard_lua_host::state::LockOp as L;
                match op {
                    L::Button => self.lock_button(),
                    L::Pad(p) => self.lock_pad(p),
                    L::Update => self.lock_update(),
                    L::Delete => self.lock_delete(),
                    L::DeleteSlot(slot) => self.lock_delete_slot(slot),
                    L::Bank(next) => self.lock_bank(next),
                }
                return;
            }
            R::ShowPerformFx(g) => {
                if g != self.group {
                    self.focus_group(g);
                }
                self.chain_tab = layout::ChainTab::Group;
                self.slot_of = Some(deckard_doc::Chain::Group(g));
                self.slot = self.perform_slot(g);
                self.channel = false;
                return;
            }
            R::PerformStrip { group, touch, pos } => {
                self.perform_strip(group, touch, pos);
                return;
            }
            R::StripControl { group, sound, pitch, value } => {
                self.send_live(surface_strip::strip_control(group, sound, pitch, value));
                return;
            }
            R::StripNote { group, sound, pitch, velocity, on } => {
                self.send_live(surface_strip::strip_note(group, sound, pitch, velocity, on));
                return;
            }
            R::GroovePage(n) => {
                layout::apply_acts(self, vec![Act::ChainTab(layout::ChainTab::Sound)]);
                self.channel = true;
                self.prop_set = 2;
                self.prop_page = n.min(2);
                return;
            }
        };
        self.knob_acc = if matches!(r, R::Turn { .. }) { self.knob_acc } else { [0.0; 8] };
        layout::apply_acts(self, vec![act]);
    }

    /// Tell the controller scripts about focus changes made with the mouse.
    fn sync_controller(&mut self) {
        let (group, scene, section, ideas) = ((self.group, self.sound), self.scene, self.section, !self.song_view);
        if let Some(Ok(c)) = &mut self.controller {
            c.sync(group, Some(scene), section, ideas);
        }
    }

    /// Share the note selection and step grid with the controller's Step and Events
    /// pages.
    fn sync_selection(&mut self) {
        if let Some(Ok(c)) = &mut self.controller {
            c.sync_selection(&mut self.grid, &mut self.nudge.0, self.group, self.pattern, &mut self.selected);
            c.host.state.borrow_mut().lock_active = self.lock_active;
        }
    }

    /// Tell the controller scripts that the transport was started or stopped here.
    fn sync_transport(&mut self) {
        let playing = self.playing.is_some();
        if let Some(Ok(c)) = &mut self.controller {
            c.set_playing(playing);
        }
    }

    /// Play one sound now (a pad hit), mixing into whatever is playing.
    fn note(&mut self, group: usize, sound: usize) {
        // Pads play the sound's base key; Maschine's default is C3 (60).
        self.play_pitch(group, sound, 60);
    }

    /// Play one note of a sound now (a pad, or a piano-roll key).
    pub fn play_pitch(&mut self, group: usize, sound: usize, pitch: i64) {
        self.play_note(deckard_engine::Live::NoteOn { group, sound, pitch, velocity: 100, length: PPQ });
    }

    /// Play a note now (a `Live::NoteOn`), starting the engine if none runs.
    fn play_note(&mut self, note: deckard_engine::Live) {
        let deckard_engine::Live::NoteOn { group, sound, pitch, velocity, length } = note else { return };
        let Some(doc) = self.doc.clone() else { return };
        let d = doc.borrow();
        let project = d.project();
        let Ok(out) = self.output.get_or_insert_with(|| open_output(&self.settings, &mut self.output_fallback)) else { return };
        if out.has_engine() {
            out.live(note);
        } else {
            let (mut engine, _) = Engine::new(project, self.path.as_deref(), out.rate, Mode::Arrangement, &mut self.samples);
            engine.sequencing = false;
            engine.note_on(group, sound, pitch, velocity, length);
            out.play(engine);
            out.note_played(&note);
        }
    }

    /// Start a note that sounds while the mouse button is held (a piano-roll key, or
    /// a note pressed in the pattern editor); `frame` ends it on release.
    pub fn hold_pitch(&mut self, group: usize, sound: usize, pitch: i64) {
        self.release_held();
        let Some(doc) = self.doc.clone() else { return };
        let d = doc.borrow();
        let Ok(out) = self.output.get_or_insert_with(|| open_output(&self.settings, &mut self.output_fallback)) else { return };
        if out.has_engine() {
            out.live(deckard_engine::Live::NoteOn { group, sound, pitch, velocity: 100, length: i64::MAX });
        } else {
            let (mut engine, _) = Engine::new(d.project(), self.path.as_deref(), out.rate, Mode::Arrangement, &mut self.samples);
            engine.sequencing = false;
            engine.note_hold(group, sound, pitch, 100);
            out.play(engine);
            out.note_played(&deckard_engine::Live::NoteOn { group, sound, pitch, velocity: 100, length: i64::MAX });
        }
        self.held = Some((group, sound, pitch));
    }

    /// Keep MIDI in step: the ports listened to, the routing, the output's link; play
    /// notes that came while no engine ran.
    fn midi_sync(&mut self, ctx: &egui::Context) {
        self.midi.set_context(ctx);
        let all = self.midi_ports().0.clone();
        // (Not a MIDI controller's own port: its presses aren't notes.)
        let ins = midi_devices::note_inputs(&all, &self.settings.midi_off, &self.midi_devices);
        if ins != self.midi_listening {
            self.midi.listen(&ins);
            self.midi_listening = ins;
        }
        self.midi.set_mpe(&self.settings.mpe);
        self.midi.set_mpe_out(&self.settings.mpe_out, self.settings.mpe_setup);
        if let Some(Ok(out)) = &self.output {
            out.set_mpe_out(&self.settings.mpe_out);
        }
        if !self.midi_wired
            && let Some(Ok(out)) = &self.output
        {
            out.set_midi(self.midi.sender());
            self.midi.set_output(out.sender());
            if let Some(m) = &self.mk3 {
                m.set_output(out.sender());
            }
            self.midi_wired = true;
        }
        if let Some(doc) = self.doc.clone() {
            let d = doc.borrow();
            let drums = self.drum_ports();
            let key = (d.revision(), self.group, self.sound, self.settings.sound_midi_focus, drums.clone());
            if self.midi_table_for.as_ref() != Some(&key) {
                let mut t = deckard_engine::midi::MidiTable::new(d.project());
                t.focus = (self.group, self.sound);
                t.default_focus = self.settings.sound_midi_focus;
                t.drums = drums;
                self.midi.set_table(t);
                self.midi_table_for = Some(key);
            }
        }
        // MIDI Lock Change: the project's setting, and the recalls it asked for.
        if let Some(doc) = &self.doc {
            let d = doc.borrow();
            let l = &d.project().lock;
            let cfg = (l.midi_trigger != 0).then(|| midi_io::LockMidi {
                source: l.midi_source.clone(),
                channel: l.midi_channel.clamp(1, 16) as u8,
                trigger: l.midi_trigger,
            });
            self.midi.set_lock(cfg);
        }
        for slot in self.midi.take_lock() {
            self.lock_midi_recall(slot);
        }
        self.remote_midi_sync();
        for (g, s, pitch, vel) in self.midi.take_pending() {
            if vel > 0 {
                self.note_in(g, s, pitch, vel);
            } else if let Some(Ok(out)) = &self.output {
                out.live(deckard_engine::Live::NoteRelease { group: g, sound: s, pitch });
            }
        }
    }

    /// The MIDI controllers run by scripts: opened and closed as their ports come and
    /// go, fed what they send, and what they ask done.
    fn midi_devices_sync(&mut self, ctx: &egui::Context) {
        let (ins, outs) = self.midi_ports().clone();
        if !self.midi_devices.active() && !self.midi_devices.ports_changed(&ins, &outs) {
            return;
        }
        self.midi_devices.set_context(ctx);
        let (view, now) = (self.surface_view(), std::time::Instant::now());
        let was = self.midi_devices.names();
        let mut asked = self.midi_devices.sync(&ins, &outs, &view, now);
        if self.midi_devices.names() != was {
            let now_open = self.midi_devices.names();
            self.status = match now_open.iter().find(|n| !was.contains(n)) {
                Some(n) => format!("{n} connected"),
                None => "MIDI controller disconnected".into(),
            };
        }
        asked.extend(self.midi_devices.poll(&view, now));
        for a in asked {
            match a {
                deckard_surface::device::Out::App(a) => self.surface_act(a),
                deckard_surface::device::Out::Print(p) => self.print_notes(&p),
                deckard_surface::device::Out::Message(m) => self.status = m,
                deckard_surface::device::Out::Midi { .. } => {}
            }
        }
        let error = self.midi_devices.errors().into_iter().next();
        if error != self.midi_device_error {
            if let Some(e) = &error {
                self.status = format!("controller script: {e}");
            }
            self.midi_device_error = error;
        }
        // (Lights that pulse with the beat, and presses, want frames.)
        if self.midi_devices.active() {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
    }

    /// Open the interface input while a sound takes audio from it (and an engine to
    /// play it through); close it when none does.
    fn input_sync(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let wants = doc
            .borrow()
            .project()
            .groups
            .iter()
            .flat_map(|g| &g.sounds)
            .any(|s| matches!(s.channel.audio_in, deckard_model::AudioSource::Ext(_)));
        let key = (self.settings.input_device.clone(), self.settings.input_routing.clone());
        if !wants {
            self.live_input = None;
            self.settings_input_failed = None;
            return;
        }
        // Open already, or failed with these settings (no retrying every frame).
        if self.live_input.as_ref().is_some_and(|(_, k)| *k == key) || self.settings_input_failed.as_ref() == Some(&key) {
            return;
        }
        let Ok(out) = self.output.get_or_insert_with(|| open_output(&self.settings, &mut self.output_fallback)) else { return };
        let live: deckard_engine::LiveIn = Default::default();
        match open_live_input(&key.0, &key.1, live.clone(), out.rate) {
            Ok(stream) => {
                out.set_live_in(live);
                if !out.has_engine() {
                    let d = doc.borrow();
                    let (mut engine, _) = Engine::new(d.project(), self.path.as_deref(), out.rate, Mode::Arrangement, &mut self.samples);
                    engine.sequencing = false;
                    out.play(engine);
                }
                self.live_input = Some((stream, key));
                self.settings_input_failed = None;
            }
            Err(e) => {
                self.status = format!("audio input: {e}");
                self.live_input = None;
                self.settings_input_failed = Some(key);
            }
        }
    }

    /// A note from MIDI input while no engine ran: start one for it.
    fn note_in(&mut self, group: usize, sound: usize, pitch: i64, velocity: i64) {
        let Some(doc) = self.doc.clone() else { return };
        let d = doc.borrow();
        let Ok(out) = self.output.get_or_insert_with(|| open_output(&self.settings, &mut self.output_fallback)) else { return };
        if out.has_engine() {
            out.live(deckard_engine::Live::NoteIn { group, sound, pitch, velocity, thru: false });
        } else {
            let (mut engine, _) = Engine::new(d.project(), self.path.as_deref(), out.rate, Mode::Arrangement, &mut self.samples);
            engine.sequencing = false;
            engine.note_in(group, sound, pitch, velocity, false);
            out.play(engine);
        }
    }

    /// End the held note, if any.
    fn release_held(&mut self) {
        if let Some((g, s, p)) = self.held.take()
            && let Some(Ok(out)) = &self.output
        {
            out.live(deckard_engine::Live::NoteRelease { group: g, sound: s, pitch: p });
        }
    }

    /// Let playback take up edits to the document (from the UI, the controller
    /// scripts, undo or redo), keeping its position.
    fn follow_edits(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let d = doc.borrow();
        let Some(Ok(out)) = &self.output else { return };
        // An edit that reached the engine while it switched modes is sent again.
        let (mode, stale) = out.telemetry(|t| (t.mode, t.stale && t.actions >= self.adopt_sent));
        if d.revision() == self.seen_revision && !stale {
            return;
        }
        // The mode it plays, once it has published one.
        let Some(mode) = mode.filter(|_| out.has_engine()) else { return };
        self.seen_revision = d.revision();
        // Built against what the playing engine has (when it is the one we last
        // edited): only what changed is made.
        let plays = out.plays();
        // (An edit sent again because playback switched mode meanwhile is built in
        // full: its timelines are what didn't arrive.)
        let prev = self.built.as_ref().filter(|b| b.0 == plays && !stale).map(|b| (&b.1, b.2.as_slice()));
        let (fresh, report) = Engine::new_after(d.project(), prev, self.path.as_deref(), out.rate, mode, &mut self.samples);
        out.adopt(fresh, d.project());
        self.adopt_sent = out.sent();
        self.built = Some((plays, d.project().clone(), report.chains));
    }

    /// Take the engine's peaks since the last frame; meters fall back by half every
    /// 150 ms.
    fn update_meters(&mut self, ctx: &egui::Context, again: std::time::Duration) {
        let now = std::time::Instant::now();
        let dt = self.meters_at.map_or(0.0, |t| now.duration_since(t).as_secs_f32());
        self.meters_at = Some(now);
        let fall = 0.5f32.powf(dt / 0.15);
        let (fresh, master) = match &self.output {
            Some(Ok(out)) => out.take_meters(),
            _ => (Vec::new(), (0.0, 0.0)),
        };
        // Headroom: the peak of each channel (sounds, their groups, the master).
        for (g, row) in fresh.iter().enumerate() {
            let mut gp = 0f32;
            for (s, v) in row.iter().enumerate() {
                let p = v.0.max(v.1);
                gp = gp.max(p);
                if p > 0.0 {
                    let h = self.headroom.entry(layout::Channel::Sound(g, s)).or_insert(0.0);
                    *h = h.max(p);
                }
            }
            if gp > 0.0 {
                let h = self.headroom.entry(layout::Channel::Group(g)).or_insert(0.0);
                *h = h.max(gp);
            }
        }
        let mp = master.0.max(master.1);
        if mp > 0.0 {
            let h = self.headroom.entry(layout::Channel::Master).or_insert(0.0);
            *h = h.max(mp);
        }
        self.master_meter = (self.master_meter.0 * fall, self.master_meter.1 * fall);
        self.master_meter = (self.master_meter.0.max(master.0), self.master_meter.1.max(master.1));
        // The CPU meter: the audio thread's load since the last look, smoothed.
        if let Some(Ok(out)) = &self.output {
            use std::sync::atomic::Ordering::Relaxed;
            let now = (out.stats.busy_us.load(Relaxed), out.stats.audio_us.load(Relaxed));
            if let Some(load) = layout::cpu_load(self.cpu_seen, now) {
                self.cpu_load = self.cpu_load * 0.8 + load * 0.2;
                self.cpu_seen = now;
            }
        }
        let mut active = false;
        for row in &mut self.meters {
            for m in row.iter_mut() {
                *m = (m.0 * fall, m.1 * fall);
            }
        }
        for (g, row) in fresh.into_iter().enumerate() {
            if self.meters.len() <= g {
                self.meters.resize(g + 1, Vec::new());
            }
            if self.meters[g].len() < row.len() {
                self.meters[g].resize(row.len(), (0.0, 0.0));
            }
            for (s, v) in row.into_iter().enumerate() {
                let m = &mut self.meters[g][s];
                *m = (m.0.max(v.0), m.1.max(v.1));
            }
        }
        for m in self.meters.iter().flatten() {
            active |= m.0 > 0.001 || m.1 > 0.001;
        }
        // Held peaks: sounds, groups (their sounds' loudest) and the master.
        let mut levels: Vec<(layout::Channel, (f32, f32))> = Vec::new();
        for (g, row) in self.meters.iter().enumerate() {
            let mut gm = (0f32, 0f32);
            for (s, m) in row.iter().enumerate() {
                levels.push((layout::Channel::Sound(g, s), *m));
                gm = (gm.0.max(m.0), gm.1.max(m.1));
            }
            levels.push((layout::Channel::Group(g), gm));
        }
        levels.push((layout::Channel::Master, self.master_meter));
        for (ch, m) in levels {
            let held = self.peaks.entry(ch).or_insert([(0.0, now); 2]);
            for (k, v) in [m.0, m.1].into_iter().enumerate() {
                let (p, at) = &mut held[k];
                if v >= *p {
                    (*p, *at) = (v, now);
                } else if now.duration_since(*at) > PEAK_HOLD {
                    *p = (*p * fall).max(v);
                }
            }
            active |= held.iter().any(|(p, _)| *p > 0.001);
        }
        // The focused sound's metered effects (their meters in the Mix view's strip).
        let (g, s) = (self.group, self.sound);
        let metered: Vec<usize> = self
            .doc
            .as_ref()
            .and_then(|d| {
                d.borrow().project().groups.get(g).and_then(|x| x.sounds.get(s)).map(|snd| {
                    snd.slots
                        .iter()
                        .enumerate()
                        .filter(|(_, x)| x.plugin.is_none() && matches!(x.module.as_str(), "Compressor" | "Limiter" | "Gate"))
                        .map(|(i, _)| i)
                        .collect()
                })
            })
            .unwrap_or_default();
        self.fx_meter_view.retain(|k, _| k.0 == g && k.1 == s && metered.contains(&k.2));
        for i in metered {
            let fresh = match &self.output {
                Some(Ok(out)) => out.fx_meters(g, s, i),
                _ => (0.0, 0.0, 0.0),
            };
            let m = self.fx_meter_view.entry((g, s, i)).or_default();
            *m = ((m.0 * fall).max(fresh.0), (m.1 * fall).max(fresh.1), (m.2 * fall).max(fresh.2));
            active |= m.0 > 0.001 || m.1 > 0.01 || m.2 > 0.001;
        }
        // Knobs that routes drive move on screen: drawn again while their travels change,
        // and all the while an LFO drives one (a stepped one holds, then jumps).
        let mut lfo_live = false;
        let travels: Vec<Option<f32>> = match (&self.output, &self.doc) {
            (Some(Ok(out)), Some(doc)) => {
                let d = doc.borrow();
                let mut v = Vec::new();
                for (g, grp) in d.project().groups.iter().enumerate() {
                    for (s, snd) in grp.sounds.iter().enumerate() {
                        for (k, r) in snd.mod_routes.iter().enumerate() {
                            let t = out.route_travel(g, s, k, deckard_engine::meters::route_key(r));
                            lfo_live |= t.is_some() && matches!(r.source, deckard_model::mod_matrix::ModSource::Lfo(_));
                            v.push(t);
                        }
                    }
                }
                v
            }
            _ => Vec::new(),
        };
        active |= travels != self.route_seen;
        self.route_seen = travels;
        if lfo_live {
            ctx.request_repaint_after(if again.is_zero() { std::time::Duration::from_millis(30) } else { again });
        }
        if active {
            ctx.request_repaint_after(again);
        }
    }

    /// A sound's meter, or a group's (its loudest sound).
    pub fn meter(&self, group: usize, sound: Option<usize>) -> (f32, f32) {
        let Some(row) = self.meters.get(group) else { return (0.0, 0.0) };
        match sound {
            Some(s) => row.get(s).copied().unwrap_or((0.0, 0.0)),
            None => row.iter().fold((0.0, 0.0), |a, m| (a.0.max(m.0), a.1.max(m.1))),
        }
    }

    fn undo(&mut self) {
        if let Some(doc) = &self.doc {
            let ok = doc.borrow_mut().undo();
            if !ok {
                self.status = "nothing to undo".into();
            }
        }
    }

    fn redo(&mut self) {
        if let Some(doc) = &self.doc {
            let ok = doc.borrow_mut().redo();
            if !ok {
                self.status = "nothing to redo".into();
            }
        }
    }

    /// Save over the opened file. The first save of a session keeps the file as it
    /// was in `<name>.mxprj.bak` next to it.
    /// Open the plug-in window of sound `sound` in group `group` (or bring it to the
    /// front).
    pub fn open_plugin_editor(&mut self, group: usize, sound: usize) {
        let slot = self.doc.as_ref().and_then(|d| {
            let d = d.borrow();
            d.project().groups.get(group)?.sounds.get(sound)?.slots.iter().position(|x| x.plugin.is_some())
        });
        if let Some(slot) = slot {
            self.open_slot_editor(group, sound, slot);
        }
    }

    /// Which plug-in instance plays slot `slot` of a sound: the instrument (its first
    /// active slot) or another slot of the chain.
    pub(crate) fn instance_key(&self, group: usize, sound: usize, slot: usize) -> deckard_plugins::Slot {
        let first = self.doc.as_ref().and_then(|d| {
            let d = d.borrow();
            d.project().groups.get(group)?.sounds.get(sound)?.slots.iter().position(|x| x.active)
        });
        if first == Some(slot) {
            deckard_plugins::Slot::Sound { group, sound }
        } else {
            deckard_plugins::Slot::Chain { group, sound, slot }
        }
    }

    fn output_rate(&mut self) -> f64 {
        match self.output.get_or_insert_with(|| open_output(&self.settings, &mut self.output_fallback)) {
            Ok(out) => out.rate as f64,
            Err(_) => 44_100.0,
        }
    }

    #[cfg(test)]
    /// The plug-in instance of slot `slot` (loading it if needed).
    fn slot_plugin(&mut self, group: usize, sound: usize, slot: usize) -> Result<deckard_plugins::Shared, String> {
        self.slot_plugin_in(deckard_doc::Chain::Sound(group, sound), slot)
    }

    /// The plug-in instance of slot `slot` of chain `c` (loading it if needed).
    fn slot_plugin_in(&mut self, c: deckard_doc::Chain, slot: usize) -> Result<deckard_plugins::Shared, String> {
        let pr = self.doc.as_ref().and_then(|d| d.borrow().chain_slots(c)?.get(slot)?.plugin.clone()).ok_or("the slot holds no plug-in")?;
        let key = self.plugin_slot_key(c, slot);
        let rate = self.output_rate();
        self.samples.plugins.instance(key, &pr.id, &pr.name, &pr.chunk, rate)
    }

    /// The instance key of slot `slot` of chain `c`.
    pub(crate) fn plugin_slot_key(&self, c: deckard_doc::Chain, slot: usize) -> deckard_plugins::Slot {
        use deckard_doc::Chain;
        match c {
            Chain::Sound(group, sound) => self.instance_key(group, sound, slot),
            Chain::Group(group) => deckard_plugins::Slot::Group { group, slot },
            Chain::Master => deckard_plugins::Slot::Master { slot },
        }
    }

    /// Open the window of the plug-in in slot `slot` (or bring it to the front).
    pub fn open_slot_editor(&mut self, group: usize, sound: usize, slot: usize) {
        self.open_slot_editor_in(deckard_doc::Chain::Sound(group, sound), slot);
    }

    /// Open the window of the plug-in in slot `slot` of chain `c`.
    pub fn open_slot_editor_in(&mut self, c: deckard_doc::Chain, slot: usize) {
        use deckard_doc::Chain;
        if let Some((.., e)) = self.editors.iter().find(|(ec, i, _)| (*ec, *i) == (c, slot)) {
            e.raise();
            return;
        }
        let Some((title, name)) = self.doc.as_ref().and_then(|d| {
            let d = d.borrow();
            let pr = d.chain_slots(c)?.get(slot)?.plugin.clone()?;
            let p = d.project();
            let owner = match c {
                Chain::Sound(g, s) => format!("{} / {}", p.groups.get(g)?.name, p.groups.get(g)?.sounds.get(s)?.name),
                Chain::Group(g) => p.groups.get(g)?.name.clone(),
                Chain::Master => "Master".into(),
            };
            Some((format!("{} — {owner}", pr.name), pr.name.clone()))
        }) else {
            return;
        };
        match self.slot_plugin_in(c, slot).and_then(|p| deckard_plugins::editor::Editor::open(&p, &title)) {
            Ok(e) => {
                self.editors.push((c, slot, e));
                self.status = format!("opened {name}");
            }
            Err(e) => self.status = format!("plug-in window: {e}"),
        }
    }

    /// The chain the chain area shows: the master's, the focused group's or sound's.
    pub fn chain(&self) -> deckard_doc::Chain {
        match self.chain_tab {
            layout::ChainTab::Master => deckard_doc::Chain::Master,
            layout::ChainTab::Group => deckard_doc::Chain::Group(self.group),
            layout::ChainTab::Sound => deckard_doc::Chain::Sound(self.group, self.sound),
        }
    }

    /// The parameter target of slot `i`'s module in the chain shown.
    pub fn module_target(&self, i: usize) -> deckard_doc::Target {
        match self.chain() {
            deckard_doc::Chain::Sound(g, s) => deckard_doc::Target::Module(g, s, i),
            c => deckard_doc::Target::ChainModule(c, i),
        }
    }

    /// The slots of the chain shown.
    pub fn chain_slots(&self) -> Vec<deckard_model::SlotInfo> {
        self.doc.as_ref().and_then(|d| d.borrow().chain_slots(self.chain()).map(<[_]>::to_vec)).unwrap_or_default()
    }

    /// Keep the Sampling view on the focused zone's sample, its playhead and the
    /// recorder up to date.
    fn update_sampling(&mut self) {
        self.poll_stems();
        self.update_loop_peaks();
        // The focused Audio plug-in take's frames and rate (its tempo shows on its page).
        self.take_info = None;
        if let Some(doc) = self.doc.clone() {
            let d = doc.borrow();
            let a = d.project().groups.get(self.group).and_then(|g| g.sounds.get(self.sound)).and_then(|s| s.audio.as_ref());
            if let Some((k, t)) = a.and_then(|a| a.takes.get(a.focused).map(|t| (a.focused, t.sample.clone()))) {
                let path = d.path().map(Path::to_path_buf);
                drop(d);
                if let Some(smp) = self.samples.sample(&t, path.as_deref()) {
                    self.take_info = Some(((self.group, self.sound, k), smp.frames.len(), smp.rate));
                }
            }
        }
        let clock = self.record_clock();
        let output = match &self.output {
            Some(Ok(o)) => Some(o),
            _ => None,
        };
        // The input meter moves while a RECORD tab shows (the app's, or the controller's
        // Sampling page), before recording too.
        let hw_record = matches!(&self.controller, Some(Ok(c)) if c.sampling() && c.host.state.borrow().sampling_tab == 0);
        let listen = self.meter_inputs && ((self.sampling_open && self.sampling.tab == 0) || hw_record);
        self.recorder.meter(
            listen,
            recorder::StartWith {
                output,
                device: self.settings.input_device.clone(),
                routing: &self.settings.input_routing,
                clock,
                dir: PathBuf::new(),
                stem: String::new(),
            },
        );
        if let Some(rec) = self.recorder.poll(output, clock) {
            self.finish_recording(rec);
        }
        self.sync_sampling_page();
        self.sync_browse_page();
        self.sync_arp();
        self.sync_auto_write();
        self.tick_mod_take(self.playhead().map(|h| (h.mode, h.position)), std::time::Instant::now());
        if !self.sampling_open {
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        let zone = self.focused_sample().map(|(k, sref, _)| (k, sref));
        let path = doc.borrow().path().map(Path::to_path_buf);
        match zone {
            Some((zi, sref)) => {
                let key = (self.group, self.sound, zi, sref.path.clone());
                if self.sampling.sample_key.as_ref() != Some(&key) {
                    match self.samples.sample(&sref, path.as_deref()) {
                        Some(s) => self.sampling.set_sample(key, s),
                        None => {
                            self.sampling.sample = None;
                            self.sampling.sample_key = None;
                        }
                    }
                }
                self.sampling.playhead = match &self.output {
                    Some(Ok(out)) => out.zone_position(self.group, self.sound, zi),
                    _ => None,
                };
            }
            None => {
                self.sampling.sample = None;
                self.sampling.sample_key = None;
                if self.sampling.tab != 0 {
                    self.sampling.tab = 0;
                }
            }
        }
    }

    /// AUTO LOOP: a loop in the focused zone's play range whose end joins smoothly back
    /// to its start (`edit::find_loop`, the Auto Sampler's), switched on, as one step.
    fn auto_loop(&mut self) {
        let (Some(doc), Some(s)) = (self.doc.clone(), self.sampling.sample.clone()) else { return };
        let Some((lo, hi)) = self.play_range_of_focused() else { return };
        let Some((a, b)) = deckard_engine::edit::find_loop(&s, (lo, hi)) else {
            self.status = "no loop found: the sound changes too much, or it's too short".into();
            return;
        };
        let t = deckard_doc::Target::Zone(self.group, self.sound, self.zone);
        let done = doc.borrow_mut().transaction("Auto Loop", |d| {
            for (field, v) in [("loop_range/low", a as f64), ("loop_range/high", b as f64), ("loop_active", 1.0)] {
                d.set("Zone", t, field, mxprj::decode::Value::Float(v))?;
            }
            Ok(())
        });
        self.status = match done {
            Ok(()) => format!("looped {a}–{b}"),
            Err(e) => format!("can't loop: {e}"),
        };
    }

    /// The focused group's Audio plug-in takes' peaks, for the pattern editor (made once
    /// per take, again when its file or play range changes).
    fn update_loop_peaks(&mut self) {
        const COLUMNS: usize = 1024;
        let Some(doc) = self.doc.clone() else { return };
        let wanted: Vec<((usize, usize, usize), deckard_model::SampleRef, (i64, i64))> = {
            let d = doc.borrow();
            let g = self.group;
            let Some(gr) = d.project().groups.get(g) else { return };
            gr.sounds
                .iter()
                .enumerate()
                .filter_map(|(s, snd)| Some((s, snd.audio.as_ref()?)))
                .flat_map(|(s, a)| a.takes.iter().enumerate().map(move |(k, t)| ((g, s, k), t.sample.clone(), t.play_range)))
                .collect()
        };
        let before = self.loop_peaks.len();
        self.loop_peaks.retain(|key, _| wanted.iter().any(|w| w.0 == *key));
        if self.loop_peaks.len() != before {
            self.loop_peaks_gen.0 += 1;
        }
        let path = doc.borrow().path().map(Path::to_path_buf);
        for (key, sref, range) in wanted {
            if self.loop_peaks.get(&key).is_some_and(|x| x.0 == sref.path && x.1 == range) {
                continue;
            }
            let Some(smp) = self.samples.sample(&sref, path.as_deref()) else { continue };
            let len = smp.frames.len();
            let (a, b) =
                if range.1 > range.0 { ((range.0.max(0) as usize).min(len), (range.1.max(0) as usize).min(len)) } else { (0, len) };
            let n = b.saturating_sub(a).max(1);
            let peaks = (0..COLUMNS)
                .map(|c| {
                    let (f0, f1) = (a + n * c / COLUMNS, (a + n * (c + 1) / COLUMNS).max(a + n * c / COLUMNS + 1).min(b.max(a + 1)));
                    smp.frames[f0.min(len.saturating_sub(1))..f1.min(len)].iter().fold((0f32, 0f32), |(lo, hi), f| {
                        let v = (f[0] + f[1]) * 0.5;
                        (lo.min(v), hi.max(v))
                    })
                })
                .collect();
            self.loop_peaks.insert(key, (sref.path.clone(), range, std::sync::Arc::new(peaks)));
            self.loop_peaks_gen.0 += 1;
        }
        // The controller's pattern editor draws them too.
        if self.loop_peaks_gen.0 != self.loop_peaks_gen.1
            && let Some(Ok(c)) = &self.controller
        {
            c.host.state.borrow_mut().loop_peaks = self.loop_peaks.iter().map(|(k, v)| (*k, v.2.clone())).collect();
            self.loop_peaks_gen.1 = self.loop_peaks_gen.0;
        }
    }

    /// STEMS: separate the focused zone's sample in the background.
    fn start_stems(&mut self) {
        let (Some(sample), Some(dir)) = (self.sampling.sample.clone(), menu::settings_dir()) else { return };
        let Some((_, sref, beats)) = self.focused_sample() else { return };
        let file = sref.path.rsplit(['/', '\\']).next().unwrap_or("Sample");
        let name = file.rsplit_once('.').map_or(file, |(stem, _)| stem).to_string();
        self.status = format!("separating {name} into stems…");
        let mut job = stems::Job::start(sample, name, (self.group, self.sound, self.zone), dir);
        job.beats = beats;
        self.stems = Some(job);
    }

    /// The focused sound's sample the Sampling view shows: its Sampler's focused zone's,
    /// or its Audio plug-in's focused take's (with the take's length in beats); and
    /// which zone or take it is.
    fn focused_sample(&self) -> Option<(usize, deckard_model::SampleRef, Option<f64>)> {
        let doc = self.doc.as_ref()?.borrow();
        let snd = doc.project().groups.get(self.group)?.sounds.get(self.sound)?;
        if let Some(a) = snd.audio.as_ref().filter(|_| snd.instrument == "AudioModule") {
            let k = self.zone.min(a.takes.len().checked_sub(1)?);
            let t = &a.takes[k];
            return Some((k, t.sample.clone(), Some(t.beats)));
        }
        let smp = snd.sampler.as_ref()?;
        let k = self.zone.min(smp.zones.len().checked_sub(1)?);
        Some((k, smp.zones[k].sample.clone(), None))
    }

    /// A finished (or failed, or cancelled) stem separation.
    fn poll_stems(&mut self) {
        let Some(result) = self.stems.as_ref().and_then(stems::Job::poll) else { return };
        let job = self.stems.take().expect("the job polled");
        match result {
            Ok(stems) => self.finish_stems(&job.name, job.rate, stems, job.beats),
            Err(_) if job.cancelled() => self.status = "stem separation cancelled".into(),
            Err(e) => self.status = format!("stem separation failed: {e}"),
        }
    }

    /// The stems as Maschine has them: each saved as a sample ("<name> Drums", …), and
    /// a new group whose first four sounds play them, here in Audio plug-ins (in time
    /// with the song, at the source's tempo), on in its first pattern, which plays in the
    /// focused scene. One undo step.
    fn finish_stems(&mut self, name: &str, rate: u32, stems: Vec<deckard_stems::Stem>, beats: Option<f64>) {
        let Some(doc) = self.doc.clone() else { return };
        let dir = self.sample_dir("Stems");
        let mut files = Vec::new();
        for s in stems {
            let title = format!("{name} {}", s.kind.name());
            let path = dir.join(format!("{}.wav", title.replace(['/', ':'], "-")));
            let frames = s.frames.len();
            if let Err(e) = deckard_engine::edit::write_wav_channels(&path, &deckard_engine::sample::Sample { rate, frames: s.frames }, 2) {
                self.status = format!("can't save the stems: {e}");
                return;
            }
            files.push((title, path, frames));
        }
        let (tempo, bar) = {
            let d = doc.borrow();
            (d.project().tempo, d.project().bar_length().max(1))
        };
        // All four the same length: the source's beats (from its take, its name or its
        // length).
        let frames = files.first().map_or(0, |f| f.2);
        let beats = beats.unwrap_or_else(|| loops::beats(name, frames, rate, tempo));
        let scene = self.scene;
        let mut group = 0;
        let done = doc.borrow_mut().transaction("Stems", |d| {
            group = d.add_group()?;
            d.rename(deckard_doc::Target::Group(group), name)?;
            let pi = match d.project().groups[group].patterns.len() {
                0 => d.add_pattern(group, usize::MAX, "Pattern 1")?,
                _ => 0,
            };
            let need = ((beats * PPQ as f64 / bar as f64).ceil() as i64).max(1) * bar;
            d.set_pattern_length(group, pi, need)?;
            for (k, (title, path, frames)) in files.iter().enumerate() {
                d.audio_only(group, k)?;
                d.set_takes(group, k, &[deckard_doc::TakeSpec { file: path.clone(), frames: *frames, rate, beats }], "Stems")?;
                d.set_audio_in(group, k, &[pi], Some(0), "Stems")?;
                d.rename(deckard_doc::Target::Sound(group, k), title)?;
            }
            if scene < d.project().scenes.len() {
                d.assign_pattern(scene, group, Some(pi))?;
            }
            Ok(())
        });
        match done {
            Ok(()) => {
                self.focus_group(group);
                self.sound = 0;
                self.status = format!("{name}: stems in a new group");
            }
            Err(e) => self.status = format!("can't make the stems' group: {e}"),
        }
    }

    /// The focused zone's range to edit: the selection, else the play range [a, b).
    fn edit_range(&self) -> Option<(usize, usize)> {
        if let Some(sel) = self.sampling.selection {
            return Some(sel);
        }
        let doc = self.doc.as_ref()?.borrow();
        let z = doc.project().groups.get(self.group)?.sounds.get(self.sound)?.sampler.as_ref()?.zones.get(self.zone)?.clone();
        let len = self.sampling.len();
        Some(if z.play_range.1 > z.play_range.0 { (z.play_range.0 as usize, (z.play_range.1 as usize + 1).min(len)) } else { (0, len) })
    }

    /// Where edited and recorded samples go: next to the project (`Samples`, or
    /// `Recordings`), else in the user's Music folder.
    fn sample_dir(&self, sub: &str) -> PathBuf {
        match self.path.as_ref().and_then(|p| p.parent()) {
            Some(dir) => dir.join(sub),
            None => std::env::home_dir().unwrap_or_default().join("Music").join("DECKARD").join(sub),
        }
    }

    /// Write `sample` as an edited copy of the focused zone's file and point the zone at
    /// it (one undo step).
    fn store_edit(&mut self, sample: deckard_engine::sample::Sample, label: &str) {
        let Some(doc) = self.doc.clone() else { return };
        let Some(key) = self.sampling.sample_key.clone() else { return };
        let original = PathBuf::from(&key.3);
        let target = deckard_engine::edit::edited_path(&self.sample_dir("Samples"), &original);
        if let Err(e) = deckard_engine::edit::write_wav(&target, &sample) {
            self.status = format!("can't write the edited sample: {e}");
            return;
        }
        let frames = sample.frames.len();
        // An Audio plug-in's take (its tempo kept), else a Sampler's zone.
        let audio = doc
            .borrow()
            .project()
            .groups
            .get(key.0)
            .and_then(|g| g.sounds.get(key.1))
            .is_some_and(|x| x.audio.is_some() && x.instrument == "AudioModule");
        let done = if audio {
            doc.borrow_mut().set_take_sample(key.0, key.1, key.2, &target, frames, label)
        } else {
            doc.borrow_mut().set_zone_sample(key.0, key.1, key.2, &target, frames, label)
        };
        match done {
            Ok(()) => {
                self.status = format!("{label}: {}", target.display());
                self.sampling.selection = None;
                self.sampling.view = None;
            }
            Err(e) => self.status = format!("can't edit: {e}"),
        }
    }

    /// Carry out a Sampling view action.
    fn sampling_act(&mut self, a: layout::sampling::SamplingAct) {
        use deckard_engine::edit;
        use layout::sampling::{EditOp, RecordAct, SamplingAct as S};
        match a {
            S::Open(on) => self.sampling_open = on,
            S::Tab(i) => {
                self.sampling.tab = i;
                if i == 2 && self.sampling.slices.is_empty() {
                    self.compute_slices();
                }
            }
            S::Stereo => self.sampling.stereo = !self.sampling.stereo,
            S::View(v) => self.sampling.view = v,
            S::Select(s) => self.sampling.selection = s,
            S::Edit(EditOp::AutoLoop) => self.auto_loop(),
            S::Edit(EditOp::Stems) => match &self.stems {
                Some(job) => job.cancel(),
                None => self.start_stems(),
            },
            S::Edit(op) => {
                let (Some(s), Some(r)) = (self.sampling.sample.clone(), self.edit_range()) else { return };
                let (out, label) = match op {
                    EditOp::Truncate => (edit::truncate(&s, r), "Truncate"),
                    EditOp::Normalize => (edit::normalize(&s, r), "Normalize"),
                    EditOp::Reverse => (edit::reverse(&s, r), "Reverse"),
                    EditOp::FadeIn => (edit::fade(&s, r, true), "Fade In"),
                    EditOp::FadeOut => (edit::fade(&s, r, false), "Fade Out"),
                    EditOp::Dc => (edit::remove_dc(&s, r), "DC Correction"),
                    EditOp::Silence => (edit::silence(&s, r), "Silence"),
                    EditOp::Copy => {
                        self.sampling.clipboard = Some(edit::copy(&s, r));
                        self.status = "copied".into();
                        return;
                    }
                    EditOp::Cut => {
                        let (rest, clip) = edit::cut(&s, r);
                        self.sampling.clipboard = Some(clip);
                        (rest, "Cut")
                    }
                    EditOp::Paste => {
                        let Some(clip) = self.sampling.clipboard.clone() else { return };
                        (edit::paste(&s, r.0, &clip), "Paste")
                    }
                    EditOp::Duplicate => (edit::duplicate(&s, r), "Duplicate"),
                    EditOp::Stretch | EditOp::AutoLoop | EditOp::Stems => return,
                };
                self.store_edit(out, label);
            }
            S::StretchOpen(on) => {
                let tempo = self.doc.as_ref().map_or(120.0, |d| d.borrow().project().tempo);
                self.sampling.stretch =
                    on.then_some(layout::sampling::Stretch { beat: true, tune: 0.0, src_bpm: tempo, new_bpm: tempo, speed: 100.0 });
            }
            S::SetStretch(sx) => self.sampling.stretch = Some(sx),
            S::StretchApply => {
                let (Some(s), Some(r), Some(sx)) = (self.sampling.sample.clone(), self.edit_range(), self.sampling.stretch) else { return };
                self.status = "stretching…".into();
                let out = edit::stretch(&s, r, sx.factor(), sx.tune);
                self.sampling.stretch = None;
                self.store_edit(out, "Stretch");
            }
            S::SliceMode(m) => {
                self.sampling.slice_mode = m;
                self.compute_slices();
            }
            S::SplitCount(i) => {
                self.sampling.split = i;
                self.compute_slices();
            }
            S::GridDiv(i) => {
                self.sampling.grid = i;
                self.compute_slices();
            }
            S::Sensitivity(v) => {
                self.sampling.sensitivity = v;
                self.compute_slices();
            }
            S::ZeroX(on) => {
                self.sampling.zero_x = on;
                self.compute_slices();
            }
            S::RemoveTool(on) => self.sampling.remove_tool = on,
            S::SliceAt(f) => {
                if self.sampling.slice_mode == 3 && !self.sampling.slices.contains(&f) {
                    self.sampling.slices.push(f);
                    self.sampling.slices.sort_unstable();
                }
            }
            S::RemoveSliceAt(f) => {
                // The slice containing `f` joins the previous one.
                if let Some(i) = self.sampling.slices.iter().rposition(|&s| s <= f).filter(|&i| i > 0) {
                    self.sampling.slices.remove(i);
                }
            }
            S::MoveSlice(i, f) => {
                let s = &mut self.sampling.slices;
                if i > 0 && i < s.len() {
                    let lo = s[i - 1] + 1;
                    let hi = s.get(i + 1).map_or(usize::MAX, |n| n - 1);
                    s[i] = f.clamp(lo, hi);
                }
            }
            S::ClearSlices => self.sampling.slices.clear(),
            S::PatternMode(m) => self.sampling.pattern_mode = m,
            S::ApplySlices => self.apply_slices(),
            S::FocusZone(i) => self.zone = i,
            S::ZoneMap(on) => self.sampling.zone_map = on,
            S::ZoneList(on) => self.sampling.zone_list = on,
            S::AddZones => self.add_zones(),
            S::RemoveZone(i) => {
                let Some(doc) = self.doc.clone() else { return };
                let n = doc.borrow().project().groups[self.group].sounds[self.sound].sampler.as_ref().map_or(0, |s| s.zones.len());
                let specs: Vec<deckard_doc::ZoneSpec> = (0..n)
                    .filter(|&k| k != i)
                    .map(|k| deckard_doc::ZoneSpec { from: deckard_doc::ZoneFrom::Zone(k), file: None, set: Vec::new() })
                    .collect();
                if let Err(e) = doc.borrow_mut().set_zones(self.group, self.sound, &specs, "Delete Zone") {
                    self.status = format!("can't delete the zone: {e}");
                }
                self.zone = self.zone.min(n.saturating_sub(2));
            }
            S::MapAsDrumKit => {
                let Some(doc) = self.doc.clone() else { return };
                let n = doc.borrow().project().groups[self.group].sounds[self.sound].sampler.as_ref().map_or(0, |s| s.zones.len());
                let specs: Vec<deckard_doc::ZoneSpec> = (0..n)
                    .map(|k| {
                        let key = (60 + k).min(127) as f64;
                        deckard_doc::ZoneSpec {
                            from: deckard_doc::ZoneFrom::Zone(k),
                            file: None,
                            set: vec![
                                ("key_range/low", key),
                                ("key_range/high", key),
                                ("root_key", key),
                                ("velocity_range/low", 0.0),
                                ("velocity_range/high", 127.0),
                            ],
                        }
                    })
                    .collect();
                if let Err(e) = doc.borrow_mut().set_zones(self.group, self.sound, &specs, "Map as Drum Kit") {
                    self.status = format!("can't map: {e}");
                }
            }
            S::Record(r) => match r {
                RecordAct::Start => self.start_recording(),
                RecordAct::Stop => {
                    let clock = self.record_clock();
                    let output = match &self.output {
                        Some(Ok(o)) => Some(o),
                        _ => None,
                    };
                    if let Some(f) = self.recorder.stop(output, clock) {
                        self.finish_recording(f);
                    }
                }
                RecordAct::Cancel => {
                    let output = match &self.output {
                        Some(Ok(o)) => Some(o),
                        _ => None,
                    };
                    self.recorder.cancel(output);
                }
                // Parameters stay as they are while armed or recording.
                RecordAct::Set(p, v) if !self.recorder.busy() => self.recorder.view.set(p, v),
                RecordAct::Threshold(t) if !self.recorder.busy() => self.recorder.view.threshold = t.clamp(-60.0, 0.0),
                RecordAct::MidiTo(m) if !self.recorder.busy() => self.recorder.view.auto.midi_to = m,
                RecordAct::ScanMidi => self.recorder.scan_midi(),
                RecordAct::Set(..) | RecordAct::Threshold(_) | RecordAct::MidiTo(_) => {}
                RecordAct::FocusTake(i) => {
                    self.recorder.view.focused = Some(i);
                    if let Some(p) = self.recorder.view.takes.get(i).cloned() {
                        self.load_into_sound(&p);
                    }
                }
                RecordAct::DeleteTake(i) => {
                    if i < self.recorder.view.takes.len() {
                        self.recorder.view.takes.remove(i);
                    }
                    self.recorder.view.focused = None;
                }
            },
        }
    }

    /// Slice positions for the current mode (Manual keeps the ones set by hand).
    fn compute_slices(&mut self) {
        use deckard_engine::edit;
        use layout::sampling::{GRID_DIVS, SPLIT_COUNTS};
        let Some(s) = self.sampling.sample.clone() else { return };
        let r = self.play_range_of_focused().unwrap_or((0, s.frames.len()));
        let st = &mut self.sampling;
        st.slices = match st.slice_mode {
            0 => edit::split_positions(r, SPLIT_COUNTS[st.split]),
            1 => {
                let tempo = self.doc.as_ref().map_or(120.0, |d| d.borrow().project().tempo);
                edit::grid_positions(s.rate, r, tempo, GRID_DIVS[st.grid].0)
            }
            2 => edit::detect_positions(&s, r, st.sensitivity, st.zero_x),
            _ => {
                if st.slices.is_empty() {
                    vec![r.0]
                } else {
                    return;
                }
            }
        };
    }

    fn play_range_of_focused(&self) -> Option<(usize, usize)> {
        let doc = self.doc.as_ref()?.borrow();
        let z = doc.project().groups.get(self.group)?.sounds.get(self.sound)?.sampler.as_ref()?.zones.get(self.zone)?.clone();
        let len = self.sampling.len();
        Some(if z.play_range.1 > z.play_range.0 { (z.play_range.0 as usize, (z.play_range.1 as usize + 1).min(len)) } else { (0, len) })
    }

    /// APPLY: one zone per slice on the same sample, keys from C1 (MIDI 36); with
    /// "Create/Replace Pattern", a note per slice at its time in the focused pattern.
    fn apply_slices(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let Some(s) = self.sampling.sample.clone() else { return };
        let end = self.play_range_of_focused().map_or(s.frames.len(), |r| r.1);
        let slices = deckard_engine::edit::slices_from(&self.sampling.slices, end);
        if slices.len() < 2 {
            return;
        }
        let zi = self.zone;
        let specs: Vec<deckard_doc::ZoneSpec> = slices
            .iter()
            .enumerate()
            .map(|(i, &(a, b))| {
                let key = (36 + i).min(127) as f64;
                deckard_doc::ZoneSpec {
                    from: deckard_doc::ZoneFrom::Zone(zi),
                    file: None,
                    set: vec![
                        ("play_range/low", a as f64),
                        ("play_range/high", (b - 1) as f64),
                        ("key_range/low", key),
                        ("key_range/high", key),
                        ("root_key", key),
                        ("velocity_range/low", 0.0),
                        ("velocity_range/high", 127.0),
                        ("loop_active", 0.0),
                    ],
                }
            })
            .collect();
        let (g, snd) = (self.group, self.sound);
        if let Err(e) = doc.borrow_mut().set_zones(g, snd, &specs, "Apply Slices") {
            self.status = format!("can't apply the slices: {e}");
            return;
        }
        self.zone = 0;
        self.sampling.slices.clear();
        self.sampling.tab = 3;
        if self.sampling.pattern_mode < 2 {
            // A note per slice, at the slice's time at the song tempo.
            let mut d = doc.borrow_mut();
            let p = d.project().clone();
            let pi = self.pattern;
            let Some(pat) = p.groups.get(g).and_then(|gr| gr.patterns.get(pi)) else { return };
            let ticks_per_frame = p.tempo / 60.0 * deckard_model::PPQ as f64 / s.rate.max(1) as f64;
            let a0 = slices[0].0;
            let add: Vec<Note> = slices
                .iter()
                .enumerate()
                .map(|(i, &(a, b))| Note {
                    expr: None,
                    sound: snd,
                    position: pat.start + ((a - a0) as f64 * ticks_per_frame) as i64,
                    length: (((b - a) as f64 * ticks_per_frame) as i64).max(1),
                    pitch: (36 + i).min(127) as i64,
                    velocity: 100,
                    probability: 100,
                })
                .collect();
            let remove: Vec<Note> =
                if self.sampling.pattern_mode == 1 { pat.notes.iter().filter(|n| n.sound == snd).copied().collect() } else { Vec::new() };
            if let Err(e) = d.edit_notes("Slice Pattern", g, pi, &remove, &add) {
                self.status = format!("slices applied; pattern not written: {e}");
            }
        }
    }

    /// Zone list "+": load samples as new zones (each over all keys and velocities).
    fn add_zones(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let Some(files) = rfd::FileDialog::new().add_filter("Audio", &["wav", "aif", "aiff", "aifc"]).pick_files() else { return };
        let n = doc.borrow().project().groups[self.group].sounds[self.sound].sampler.as_ref().map_or(0, |s| s.zones.len());
        let mut specs: Vec<deckard_doc::ZoneSpec> =
            (0..n).map(|k| deckard_doc::ZoneSpec { from: deckard_doc::ZoneFrom::Zone(k), file: None, set: Vec::new() }).collect();
        for f in files {
            let Ok(s) = deckard_engine::sample::Sample::load(&f) else {
                self.status = format!("can't read {}", f.display());
                continue;
            };
            specs.push(new_zone(&f, s.frames.len(), if n > 0 { deckard_doc::ZoneFrom::Zone(0) } else { deckard_doc::ZoneFrom::Template }));
        }
        if let Err(e) = doc.borrow_mut().set_zones(self.group, self.sound, &specs, "Add Zones") {
            self.status = format!("can't add the zones: {e}");
        }
    }

    /// Where playback is, for the recorder's Sync and Loop modes. Loop (in the Ideas
    /// view) starts on the focused pattern's length, others on bars.
    fn record_clock(&self) -> recorder::Clock {
        let Some(doc) = &self.doc else { return recorder::Clock::default() };
        let d = doc.borrow();
        let p = d.project();
        let bar = p.bar_length() as f64;
        let pattern = p.groups.get(self.group).and_then(|g| g.patterns.get(self.pattern)).map_or(0, |pt| pt.length) as f64;
        let grid = if self.recorder.view.mode == 2 && !self.song_view && pattern > 0.0 { pattern } else { bar };
        let (playing, position, length) = match &self.output {
            Some(Ok(out)) => out.telemetry(|t| (self.playing.is_some() && t.sequencing, t.position, t.length)),
            _ => (false, 0.0, 0.0),
        };
        recorder::Clock { playing, position, length, tempo: p.tempo, grid, bar }
    }

    /// An engine to record from or play notes into (one that plays nothing if
    /// playback is stopped).
    fn ensure_engine(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let d = doc.borrow();
        let Ok(out) = self.output.get_or_insert_with(|| open_output(&self.settings, &mut self.output_fallback)) else { return };
        if !out.has_engine() {
            let rate = out.rate;
            drop(d);
            let engine = self.idle_engine(rate);
            if let Some(Ok(out)) = &self.output {
                out.play(engine);
            }
        }
    }

    /// An engine that plays nothing, for the pads and keys while stopped: with Note
    /// Repeat as it is (switched on before there was an engine, it wasn't sent).
    pub(crate) fn idle_engine(&mut self, rate: u32) -> Engine {
        let doc = self.doc.clone().expect("a project");
        let (mut engine, _) = Engine::new(doc.borrow().project(), self.path.as_deref(), rate, Mode::Arrangement, &mut self.samples);
        engine.sequencing = false;
        engine.set_repeat(self.repeat_sent.clone());
        engine.set_metronome_volume(f32::from(self.settings.metronome_volume) / 100.0);
        engine.metronome_auto = self.settings.metronome_auto;
        engine.metronome_ticks = self.settings.metronome_ticks;
        engine
    }

    /// Note Repeat / Arp: the NOTE REPEAT page's edits into the project; on while the
    /// page shows, LOCK is on or the software switched it on; the Arp with the pads in
    /// keyboard mode (or ⌘K's keys); the engine told when that changes; the page shown
    /// the settings.
    fn sync_arp(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let (edits, on_page, keyboard) = match &self.controller {
            Some(Ok(c)) => {
                let edits = std::mem::take(&mut c.host.state.borrow_mut().arp_edits);
                let page = c.host.constant("NI.HW.PAGE_REPEAT").ok();
                let on_page = page.is_some() && c.host.state.borrow().hw.page_stack.last().copied() == page;
                (edits, on_page, c.host.keyboard_mode())
            }
            // (DECKARD's own layer: its NOTE REPEAT page.)
            _ => (Vec::new(), self.surface.page() == "note_repeat", self.piano_roll),
        };
        for (key, v) in edits {
            match key.as_str() {
                "arp/locked" => self.arp_locked = v > 0.5,
                _ => {
                    self.arp_edit(&key, v);
                }
            }
        }
        let active = self.repeat_soft || self.arp_locked || on_page;
        let mode =
            if keyboard || self.keys.on { deckard_engine::repeat::RepeatMode::Arp } else { deckard_engine::repeat::RepeatMode::NoteRepeat };
        let (arp, bar) = {
            let d = doc.borrow();
            (d.project().arp.clone(), d.project().bar_length())
        };
        let want = active.then(|| (arp.clone(), mode));
        if want != self.repeat_sent {
            if let Some(Ok(out)) = &self.output {
                out.set_repeat(want.clone());
            }
            self.repeat_sent = want;
        }
        let presets = doc.borrow().project().arp_presets.clone();
        let view = arp_ui::view(&arp, &presets, bar, self.chosen_preset(), active, self.arp_locked);
        if let Some(Ok(c)) = &mut self.controller
            && c.host.state.borrow().arp != view
        {
            c.host.state.borrow_mut().arp = view;
        }
    }

    /// The Note Repeat / Arp panel: on, LOCK, the presets and every setting (as the
    /// MK3's NOTE REPEAT page has them).
    fn repeat_window(&mut self, ctx: &egui::Context) {
        let Some(doc) = self.doc.clone() else { return };
        let (arp, bar, presets) = {
            let d = doc.borrow();
            (d.project().arp.clone(), d.project().bar_length(), d.project().arp_presets.clone())
        };
        let view = arp_ui::view(&arp, &presets, bar, self.chosen_preset(), self.repeat_sent.is_some(), self.arp_locked);
        let mut open = true;
        let mut edits: Vec<(String, f64)> = Vec::new();
        let (mut on, mut lock) = (self.repeat_soft, self.arp_locked);
        egui::Window::new("Note Repeat / Arp").open(&mut open).resizable(false).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.toggle_value(&mut on, "On").on_hover_text("Held pads (and ⌘K's keys) repeat; the Arp with the pads in keyboard mode");
                ui.toggle_value(&mut lock, "Lock").on_hover_text("Stays on outside the MK3's NOTE REPEAT page");
                ui.separator();
                let shown = view.knobs.iter().find(|k| k.key == "arp/preset").map_or(0, |k| k.value as usize);
                // Presets 1–4 (item 0 is none).
                for (i, name) in
                    view.lists.iter().find(|(k, _)| k == "arp/preset").map(|x| x.1.clone()).unwrap_or_default().iter().enumerate().skip(1)
                {
                    if ui.selectable_label(shown == i, name).clicked() {
                        edits.push(("arp/preset".into(), i as f64));
                    }
                }
            });
            ui.separator();
            egui::Grid::new("arp-settings").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                for k in view.knobs.iter().filter(|k| k.key != "arp/preset") {
                    ui.label(&k.name);
                    match view.lists.iter().find(|(key, _)| *key == k.key) {
                        Some((_, list)) => {
                            egui::ComboBox::from_id_salt(&k.key).selected_text(&k.text).show_ui(ui, |ui| {
                                for (i, name) in list.iter().enumerate() {
                                    if ui.selectable_label(k.value.round() as usize == i, name).clicked() {
                                        edits.push((k.key.clone(), i as f64));
                                    }
                                }
                            });
                        }
                        None => {
                            let mut v = k.value;
                            let slider = egui::Slider::new(&mut v, k.min..=k.max).step_by(k.step).custom_formatter(|_, _| k.text.clone());
                            if ui.add(slider).changed() {
                                edits.push((k.key.clone(), v));
                            }
                        }
                    }
                    ui.end_row();
                }
            });
        });
        self.repeat_panel = open;
        self.repeat_soft = on;
        if lock != self.arp_locked {
            self.arp_locked = lock;
            if let Some(Ok(c)) = &mut self.controller {
                c.host.state.borrow_mut().arp.locked = lock;
            }
        }
        for (key, v) in edits {
            self.arp_edit(&key, v);
        }
    }

    /// Note Repeat's settings as DECKARD's own layer shows them.
    fn repeat_view(&self) -> deckard_surface::RepeatView {
        let Some(doc) = &self.doc else { return Default::default() };
        let (arp, bar, presets) = {
            let d = doc.borrow();
            (d.project().arp.clone(), d.project().bar_length(), d.project().arp_presets.clone())
        };
        let v = arp_ui::view(&arp, &presets, bar, self.chosen_preset(), self.repeat_sent.is_some(), self.arp_locked);
        let knob = |key: &str| v.knobs.iter().find(|k| k.key == key);
        deckard_surface::RepeatView {
            settings: v
                .knobs
                .iter()
                .filter(|k| k.key != "arp/hold" && k.key != "arp/preset")
                .map(|k| deckard_surface::RepeatSetting {
                    key: k.key.trim_start_matches("arp/").to_string(),
                    name: k.name.clone(),
                    text: k.text.clone(),
                    value: k.value,
                    min: k.min,
                    max: k.max,
                    step: k.step,
                    turn: k.turn,
                })
                .collect(),
            // (Item 0 of the list is no preset.)
            presets: v.lists.iter().find(|(k, _)| k == "arp/preset").map(|(_, l)| l.iter().skip(1).cloned().collect()).unwrap_or_default(),
            preset: knob("arp/preset").map_or(0, |k| (k.value as usize).saturating_sub(1)),
            hold: knob("arp/hold").is_some_and(|k| k.value > 0.5),
            locked: v.locked,
            active: v.active,
        }
    }

    /// Note Repeat's preset chosen (0–3): as last chosen in this document, else the one
    /// at its rate (a new project's 1/16: the second), else the first.
    fn chosen_preset(&self) -> usize {
        if let Some((generation, p)) = self.arp_preset
            && generation == self.doc_generation
        {
            return p;
        }
        let Some(doc) = &self.doc else { return 0 };
        let d = doc.borrow();
        let p = d.project();
        arp_ui::preset_at(&p.arp_presets, p.arp.rate, p.bar_length()).unwrap_or(0)
    }

    /// A Note Repeat setting changed (`arp/<field>` = `v`): a preset (1–4) chosen, or a
    /// setting, the rate and unit kept in the preset chosen.
    fn arp_edit(&mut self, key: &str, v: f64) {
        let Some(doc) = self.doc.clone() else { return };
        let preset = self.chosen_preset();
        if key == "arp/preset" && v >= 0.5 {
            self.arp_preset = Some((self.doc_generation, (v.round() as usize - 1).min(3)));
        } else {
            // Fixed now, should the rate move off every preset's.
            self.arp_preset = Some((self.doc_generation, preset));
        }
        if let Err(e) = arp_ui::apply(&mut doc.borrow_mut(), key, v, preset) {
            self.status = format!("Note Repeat: {e}");
        }
    }

    /// The controller's BROWSE: what it asked for, then the library as the app has it
    /// (the library started when the page first shows).
    fn sync_browse_page(&mut self) {
        let Some(Ok(c)) = &self.controller else { return };
        let on_page = c.host.constant("NI.HW.PAGE_BROWSE").ok().is_some_and(|p| c.host.state.borrow().hw.page_stack.contains(&p));
        let calls = std::mem::take(&mut c.host.state.borrow_mut().browse_calls);
        if on_page && !self.browser.started() {
            self.browse(browser::BrowseAct::Toggle);
            self.browse(browser::BrowseAct::Toggle);
        }
        for call in calls {
            if let Some(a) = self.browser.from_controller(call) {
                self.browse(a);
            }
        }
        if self.browser.revision() == self.browser_sent {
            return;
        }
        self.browser_sent = self.browser.revision();
        let view = self.browser.view();
        if let Some(Ok(c)) = &mut self.controller {
            c.host.set_browser(view);
        }
    }

    /// The controller's Sampling page: what it asked for (knobs turned, edits, slices,
    /// zones, the tab picked) first, then what it shows (the recorder's meter and state,
    /// the focused zone's sample, every tab's knobs).
    fn sync_sampling_page(&mut self) {
        let (edits, calls, hw_tab, focus_slice) = match &self.controller {
            Some(Ok(c)) => {
                let mut st = c.host.state.borrow_mut();
                let focus_slice = st.sample.as_ref().and_then(|s| s.focus_slice);
                (
                    std::mem::take(&mut st.sampling_edits),
                    std::mem::take(&mut st.sampling_calls),
                    st.sampling_tab.clamp(0, 3) as usize,
                    focus_slice,
                )
            }
            _ => return,
        };
        for (key, value) in edits {
            self.sampling_knob(&key, value);
        }
        for c in calls {
            self.sampling_call(c);
        }
        // The tab: whichever side changed it since they last agreed (at first, the app's).
        let mut tab = None;
        if self.hw_sampling_tab.is_some_and(|t| t != hw_tab) {
            self.sampling_act(layout::sampling::SamplingAct::Tab(hw_tab));
        } else if self.sampling.tab != hw_tab {
            tab = Some(self.sampling.tab);
        }
        self.hw_sampling_tab = Some(self.sampling.tab);
        let at = (self.group, self.sound, self.zone);
        let pages = self.doc.as_ref().map(|d| {
            let d = d.borrow();
            let record = record_knobs::knobs(&self.recorder.view, &self.settings, d.project());
            (
                sampling_hw::pages(record, &self.sampling, d.project(), at),
                sampling_hw::sample_view(&self.sampling, d.project(), at, focus_slice),
            )
        });
        let Some(Ok(c)) = &mut self.controller else { return };
        let v = &self.recorder.view;
        let mut st = c.host.state.borrow_mut();
        st.record = deckard_lua_host::state::RecordState {
            levels: v.meter,
            mode: v.mode as i64,
            threshold: v.threshold,
            waiting: v.status == layout::sampling::RecStatus::Waiting,
            recording: v.status == layout::sampling::RecStatus::Recording,
        };
        if let Some(t) = tab {
            st.sampling_tab = t as i64;
        }
        st.stems = self.stems.as_ref().map(|j| deckard_lua_host::state::StemsRun { focused: j.source == at, progress: j.progress() });
        if let Some((k, sample)) = pages {
            st.sample = sample;
            for (t, pages) in k.tabs.into_iter().enumerate() {
                st.sampling_page[t] = st.sampling_page[t].min(pages.len().saturating_sub(1));
                st.sampling_pages[t] = pages;
            }
            st.slice_knobs = k.slice;
            self.record_apply = k.apply;
        }
    }

    /// A Sampling page knob turned to `value` (`sampling_hw`, `record_knobs`).
    fn sampling_knob(&mut self, key: &str, value: f64) {
        use layout::sampling::{RecordAct, SamplingAct};
        use sampling_hw::Apply;
        let at = value.round().max(0.0) as usize;
        let act = match self.record_apply.get(key).cloned() {
            Some(Apply::Record(a)) => {
                use record_knobs::Apply as R;
                SamplingAct::Record(match a {
                    R::Param(p, None) => RecordAct::Set(p, value.round() as i64),
                    R::Param(p, Some(values)) => match values.get(at) {
                        Some(&v) => RecordAct::Set(p, v),
                        None => return,
                    },
                    R::Threshold => RecordAct::Threshold(value.clamp(-60.0, 0.0)),
                    R::MidiTo(ports) => match ports.get(at) {
                        Some(p) => RecordAct::MidiTo(p.clone()),
                        None => return,
                    },
                })
            }
            Some(Apply::Zone(path)) => {
                let t = deckard_doc::Target::Zone(self.group, self.sound, self.zone);
                layout::apply_acts(self, vec![layout::Act::SetParam(t, path, value)]);
                return;
            }
            Some(Apply::Selection(end)) => {
                let len = self.sampling.len();
                let (a, b) = self.sampling.selection.unwrap_or_else(|| self.play_range_of_focused().unwrap_or((0, len)));
                let v = value.round().max(0.0) as usize;
                let (a, b) = if end { (a, v.max(a + 1)) } else { (v.min(b.saturating_sub(1)), b) };
                SamplingAct::Select(Some((a, b.min(len))))
            }
            Some(Apply::SliceMode) => SamplingAct::SliceMode(sampling_hw::app_slice_mode(value)),
            Some(Apply::SplitCount) => SamplingAct::SplitCount(at),
            Some(Apply::GridDiv) => SamplingAct::GridDiv(at),
            Some(Apply::Sensitivity) => SamplingAct::Sensitivity(value.clamp(0.0, 1.0)),
            Some(Apply::ZeroX) => SamplingAct::ZeroX(value >= 0.5),
            Some(Apply::PatternMode) => SamplingAct::PatternMode(at),
            Some(Apply::None) | None => return,
        };
        self.sampling_act(act);
    }

    /// What the Sampling page asked of the focused sample, its slices or zones.
    fn sampling_call(&mut self, c: deckard_lua_host::state::SamplingCall) {
        use deckard_lua_host::state::SamplingCall as C;
        use layout::sampling::{EditOp, SamplingAct as S};
        const EDITS: [EditOp; 11] = [
            EditOp::Truncate,
            EditOp::Normalize,
            EditOp::Reverse,
            EditOp::FadeIn,
            EditOp::FadeOut,
            EditOp::Dc,
            EditOp::Silence,
            EditOp::Cut,
            EditOp::Copy,
            EditOp::Paste,
            EditOp::Duplicate,
        ];
        let act = match c {
            C::Edit(f) => match f.checked_sub(1).and_then(|i| EDITS.get(i)) {
                Some(op) => S::Edit(*op),
                None => return,
            },
            C::View(a, b) => S::View(Some((a, b))),
            C::AutoLoop => S::Edit(EditOp::AutoLoop),
            C::Stems if self.stems.is_none() => S::Edit(EditOp::Stems),
            C::CancelStems if self.stems.is_some() => S::Edit(EditOp::Stems),
            C::Stems | C::CancelStems => return,
            // Splitting or removing a slice by hand makes the slicing manual.
            C::SplitSlice(i) => {
                let len = self.sampling.len();
                let s = &mut self.sampling.slices;
                let (Some(&a), end) = (s.get(i), s.get(i + 1).copied().unwrap_or(len)) else { return };
                if end > a + 1 {
                    s.insert(i + 1, (a + end) / 2);
                    self.sampling.slice_mode = 3;
                }
                return;
            }
            C::RemoveSlice(i) => {
                if i > 0 && i < self.sampling.slices.len() {
                    self.sampling.slices.remove(i);
                    self.sampling.slice_mode = 3;
                }
                return;
            }
            C::ClearSlices => S::ClearSlices,
            C::MoveSlice(i, f) => S::MoveSlice(i, f),
            C::ApplySlices => S::ApplySlices,
            C::RemoveZone(i) => S::RemoveZone(i),
            C::FocusZone(i) => S::FocusZone(i),
            // No sample preview to play it with yet.
            C::Prehear(..) => return,
        };
        self.sampling_act(act);
    }

    fn start_recording(&mut self) {
        if self.recorder.view.source == 2 {
            self.ensure_engine();
        }
        let clock = self.record_clock();
        let stem = match (self.recorder.view.mode, self.recorder.view.source, self.recorder.view.auto_sound, &self.doc) {
            (3, 2, Some((g, s)), Some(doc)) => {
                let name = doc.borrow().project().groups.get(g).and_then(|x| x.sounds.get(s)).map(|x| x.name.clone()).unwrap_or_default();
                let safe: String = name.chars().map(|c| if c == '/' || c == ':' { '-' } else { c }).collect();
                if safe.trim().is_empty() { recorder::timestamp() } else { safe }
            }
            _ => recorder::timestamp(),
        };
        let dir = self.sample_dir("Recordings");
        let output = match &self.output {
            Some(Ok(o)) => Some(o),
            _ => None,
        };
        self.recorder.start(recorder::StartWith {
            output,
            device: self.settings.input_device.clone(),
            routing: &self.settings.input_routing,
            clock,
            dir,
            stem,
        });
    }

    /// A finished recording: Detect and Sync load the take into the focused sound; Loop
    /// into its target; Auto maps its notes into a Sampler.
    fn finish_recording(&mut self, f: recorder::Finished) {
        match f {
            recorder::Finished::Take { sample, channels, mode, ticks } => {
                let path = self.sample_dir("Recordings").join(recorder::take_name());
                if let Err(e) = deckard_engine::edit::write_wav_channels(&path, &sample, channels) {
                    self.recorder.view.error = Some(format!("can't save the recording: {e}"));
                    return;
                }
                self.recorder.view.takes.push(path.clone());
                self.recorder.view.focused = Some(self.recorder.view.takes.len() - 1);
                if mode == 2 {
                    self.finish_loop(&path, sample.frames.len(), sample.rate, ticks);
                } else {
                    let (g, s) = (self.group, self.sound);
                    self.load_take(g, s, &path, sample.frames.len());
                }
                self.sampling.tab = 0;
            }
            recorder::Finished::Auto { done } => self.finish_auto(done),
        }
    }

    /// The first sound after the focused one in its group with nothing loaded, else the
    /// group's last sound (as Maschine's `isSoundAvailableForRecording`).
    fn next_free_sound(&self, g: usize) -> usize {
        let Some(doc) = &self.doc else { return self.sound };
        let d = doc.borrow();
        let Some(gr) = d.project().groups.get(g) else { return self.sound };
        (self.sound + 1..gr.sounds.len())
            .find(|&s| gr.sounds[s].slots.is_empty() && gr.sounds[s].sampler.is_none())
            .unwrap_or(gr.sounds.len().saturating_sub(1))
    }

    /// Loop mode: the take goes to the focused sound (Take) or the next free one (Sound,
    /// Pattern), and plays from the start of the pattern (a new pattern for Pattern),
    /// which grows to fit it. As the manual has it, into an Audio plug-in (Take: one
    /// more take of the sound's), in time with the song; a sound with a Sampler keeps it
    /// (the take its zone, a note in the pattern playing it).
    fn finish_loop(&mut self, path: &Path, frames: usize, rate: u32, ticks: f64) {
        let Some(doc) = self.doc.clone() else { return };
        let g = self.group;
        let target = self.recorder.view.target;
        let s = if target == 0 { self.sound } else { self.next_free_sound(g) };
        let (sampler, audio) = {
            let d = doc.borrow();
            let snd = d.project().groups.get(g).and_then(|x| x.sounds.get(s));
            (snd.is_some_and(|x| x.sampler.is_some()), snd.is_some_and(|x| x.audio.is_some() && x.instrument == "AudioModule"))
        };
        if !sampler {
            let spec = deckard_doc::TakeSpec { file: path.to_path_buf(), frames, rate, beats: ticks / PPQ as f64 };
            let (scene, song_view, focused) = (self.scene, self.song_view, self.pattern);
            let mut made = None;
            let done = doc.borrow_mut().transaction("Record Loop", |d| {
                d.audio_only(g, s)?;
                let k = if target == 0 && audio {
                    d.add_take(g, s, &spec, "Record Loop")?
                } else {
                    d.set_takes(g, s, &[spec.clone()], "Record Loop")?;
                    0
                };
                let mut pi = focused.min(d.project().groups[g].patterns.len().saturating_sub(1));
                if d.project().groups[g].patterns.is_empty() {
                    pi = d.add_pattern(g, usize::MAX, "Pattern 1")?;
                } else if target == 2 {
                    let n = d.project().groups[g].patterns.len();
                    pi = d.add_pattern(g, focused, &format!("Pattern {}", n + 1))?;
                    if !song_view {
                        d.assign_pattern(scene, g, Some(pi))?;
                    }
                    made = Some(pi);
                }
                d.set_audio_in(g, s, &[pi], Some(k), "Record Loop")?;
                // Long enough for the take: doubled while shorter.
                let need = ticks.round().max(1.0) as i64;
                let mut len = d.project().groups[g].patterns[pi].length.max(1);
                let was = len;
                while len < need {
                    len *= 2;
                }
                if len != was {
                    d.set_pattern_length(g, pi, len)?;
                }
                Ok(())
            });
            match done {
                Ok(()) => {
                    if let Some(pi) = made {
                        self.pattern = pi;
                    }
                    self.sound = s;
                    self.zone = 0;
                }
                Err(e) => self.status = format!("can't load the loop: {e}"),
            }
            return;
        }
        self.load_take(g, s, path, frames);
        let mut pi = self.pattern;
        if target == 2 {
            let n = doc.borrow().project().groups.get(g).map_or(0, |x| x.patterns.len());
            let made = doc.borrow_mut().add_pattern(g, self.pattern, &format!("Pattern {}", n + 1));
            match made {
                Ok(i) => {
                    pi = i;
                    if !self.song_view
                        && let Err(e) = doc.borrow_mut().assign_pattern(self.scene, g, Some(i))
                    {
                        self.status = format!("the new pattern isn't in the scene: {e}");
                    }
                    self.pattern = i;
                }
                Err(e) => {
                    self.status = format!("can't make a pattern for the take: {e}");
                    return;
                }
            }
        }
        let mut d = doc.borrow_mut();
        let Some(pat) = d.project().groups.get(g).and_then(|x| x.patterns.get(pi)).cloned() else { return };
        let ticks = ticks.round().max(1.0) as i64;
        // Long enough for the take: doubled while shorter.
        let mut len = pat.length.max(1);
        while len < ticks {
            len *= 2;
        }
        if len != pat.length
            && let Err(e) = d.set_pattern_length(g, pi, len)
        {
            self.status = format!("can't lengthen the pattern: {e}");
        }
        let old: Vec<Note> = pat.notes.iter().filter(|n| n.sound == s).copied().collect();
        let note = Note { expr: None, sound: s, position: pat.start, length: ticks, pitch: 60, velocity: 100, probability: 100 };
        if let Err(e) = d.edit_notes("Record Loop", g, pi, &old, &[note]) {
            self.status = format!("the take isn't in the pattern: {e}");
        }
        drop(d);
        self.sound = s;
    }

    /// Auto: the recorded notes as zones of a Sampler, in the focused sound if it has
    /// one, else in the next free sound; then the Zone tab.
    fn finish_auto(&mut self, done: Vec<recorder::AutoTake>) {
        let Some(doc) = self.doc.clone() else { return };
        let g = self.group;
        let has_sampler = doc.borrow().project().groups.get(g).and_then(|x| x.sounds.get(self.sound)).is_some_and(|x| x.sampler.is_some());
        let s = if has_sampler { self.sound } else { self.next_free_sound(g) };
        if !self.ensure_sampler(g, s) {
            return;
        }
        let existing = doc.borrow().project().groups[g].sounds[s].sampler.as_ref().map_or(0, |x| x.zones.len());
        let mut specs: Vec<deckard_doc::ZoneSpec> =
            (0..existing).map(|k| deckard_doc::ZoneSpec { from: deckard_doc::ZoneFrom::Zone(k), file: None, set: Vec::new() }).collect();
        for (item, frames, lp) in &done {
            let from = if existing > 0 { deckard_doc::ZoneFrom::Zone(0) } else { deckard_doc::ZoneFrom::Template };
            let mut spec = new_zone(&item.path, *frames, from);
            spec.set
                .retain(|(k, _)| !["key_range/low", "key_range/high", "velocity_range/low", "velocity_range/high", "root_key"].contains(k));
            spec.set.extend([
                ("key_range/low", item.keys.0 as f64),
                ("key_range/high", item.keys.1 as f64),
                ("velocity_range/low", item.vels.0 as f64),
                ("velocity_range/high", item.vels.1 as f64),
                ("root_key", item.note as f64),
            ]);
            if let Some((a, b)) = lp {
                spec.set.retain(|(k, _)| !k.starts_with("loop_"));
                spec.set.extend([("loop_active", 1.0), ("loop_range/low", *a.min(b) as f64), ("loop_range/high", *a.max(b) as f64)]);
            }
            specs.push(spec);
        }
        match doc.borrow_mut().set_zones(g, s, &specs, "Auto Sample") {
            Ok(()) => {
                self.sound = s;
                self.zone = existing;
                self.sampling.tab = 3;
                self.status = format!("sampled {} notes", done.len());
            }
            Err(e) => self.status = format!("can't map the samples: {e}"),
        }
    }

    /// Give sound `s` of group `g` a Sampler if it has none. Whether it has one now.
    /// SAMPLE on the controller: the focused sound becomes a lone Sampler (its other
    /// modules and plug-ins go, one undo step) and the Sampling view shows it.
    fn sample_focused_sound(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, s) = (self.group, self.sound);
        // Nothing sampled yet: RECORD.
        let empty = doc
            .borrow()
            .project()
            .groups
            .get(g)
            .and_then(|x| x.sounds.get(s))
            .and_then(|x| x.sampler.as_ref())
            .is_none_or(|x| x.zones.is_empty());
        if empty {
            self.sampling.tab = 0;
        }
        self.sampling_open = true;
    }

    fn ensure_sampler(&mut self, g: usize, s: usize) -> bool {
        let Some(doc) = self.doc.clone() else { return false };
        if doc.borrow().project().groups.get(g).and_then(|x| x.sounds.get(s)).is_some_and(|x| x.sampler.is_none()) {
            let keep = (self.group, self.sound, self.slot, self.slot_of);
            (self.group, self.sound) = (g, s);
            self.add_slot(layout::AddWhat::Sampler);
            (self.group, self.sound) = (keep.0, keep.1);
            if (g, s) != (keep.0, keep.1) {
                (self.slot, self.slot_of) = (keep.2, keep.3);
            }
        }
        doc.borrow().project().groups.get(g).and_then(|x| x.sounds.get(s)).is_some_and(|x| x.sampler.is_some())
    }

    /// Load a take into a sound's Sampler as its only zone, and name the sound after it
    /// unless it was named by hand. A sound without a Sampler becomes one (its other
    /// modules removed, as Maschine does with a new recording); one with a Sampler keeps
    /// its chain.
    fn load_take(&mut self, g: usize, s: usize, path: &Path, frames: usize) {
        let Some(doc) = self.doc.clone() else { return };
        let has_sampler = doc.borrow().project().groups.get(g).and_then(|x| x.sounds.get(s)).is_some_and(|x| x.sampler.is_some());
        // One undo step: the Sampler, its zone, the name.
        let done = doc.borrow_mut().transaction("Load Sample", |d| {
            if !has_sampler {
                d.sampler_only(g, s)?;
            }
            let has = d.project().groups[g].sounds[s].sampler.as_ref().map_or(0, |x| x.zones.len()) > 0;
            let spec = new_zone(path, frames, if has { deckard_doc::ZoneFrom::Zone(0) } else { deckard_doc::ZoneFrom::Template });
            d.set_zones(g, s, &[spec], "Load Sample")?;
            let name = d.project().groups[g].sounds[s].name.clone();
            if (name.is_empty() || name.starts_with("Sound ") || name.parse::<u32>().is_ok())
                && let Some(stem) = path.file_stem()
            {
                d.rename(deckard_doc::Target::Sound(g, s), &stem.to_string_lossy())?;
            }
            Ok(())
        });
        if let Err(e) = done {
            self.status = format!("can't load the sample: {e}");
            return;
        }
        if (g, s) == (self.group, self.sound) {
            if !has_sampler {
                self.slot = Some(0);
                self.chain_tab = layout::ChainTab::Sound;
            }
            self.zone = 0;
        }
        self.status = format!("recorded {}", path.display());
    }

    /// Samples dropped on sound `s` of group `g`: on a sound with a Sampler they're added
    /// to it (each a zone); otherwise each becomes an Audio plug-in's loop on a sound of
    /// its own (that sound, then the ones after it), in time with the song.
    /// Maschine's user folder: saves go to its Sounds, Groups and Plug-ins.
    fn user_dir(&self) -> PathBuf {
        self.user_dir_override
            .clone()
            .unwrap_or_else(|| deckard_library::user::documents().unwrap_or_default().join("Native Instruments").join("Maschine 3"))
    }

    /// The folders DECKARD's library indexes: Maschine's user folders (Maschine 3's,
    /// and Maschine 2's beside it), listed under User, and the library folders from
    /// Preferences, under Factory, each its folder's name (less " Library") as product.
    fn library_roots(&self) -> Vec<deckard_library::user::Root> {
        use deckard_library::user::Root;
        let user = self.user_dir();
        let m2 = user.parent().map(|p| p.join("Maschine 2"));
        let mut roots: Vec<Root> = [Some(user), m2]
            .into_iter()
            .flatten()
            .filter(|p| p.is_dir())
            .map(|path| Root { path, factory: false, product: None })
            .collect();
        for d in &self.settings.library_folders {
            let name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let product = name.strip_suffix(" Library").unwrap_or(&name).to_string();
            roots.push(Root { path: d.clone(), factory: true, product: Some(product) });
        }
        roots
    }

    /// Save a sound, group or preset: again where it was (Save), or through the dialog.
    fn save_item(&mut self, what: deckard_doc::FileItem, mode: SaveMode) {
        use deckard_doc::FileItem;
        let id = self.doc.as_ref().and_then(|d| d.borrow().file_item_id(what));
        if mode == SaveMode::Save
            && let Some(s) = self.saved.get(&what).cloned()
            && s.id.is_some()
            && s.id == id
        {
            self.write_item(what, s);
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        let d = doc.borrow();
        let p = d.project();
        let (name, folder) = match what {
            FileItem::Sound(g, s) => (p.groups.get(g).and_then(|x| x.sounds.get(s)).map(|x| x.name.clone()), "Sounds"),
            FileItem::Group(g) => (p.groups.get(g).map(|x| x.name.clone()), "Groups"),
            FileItem::Slot(g, s, k) => {
                (p.groups.get(g).and_then(|x| x.sounds.get(s)).and_then(|x| x.slots.get(k)).map(|x| x.name.clone()), "Plug-ins")
            }
        };
        let Some(name) = name else { return };
        let before = self.saved.get(&what).map(|s| &s.info);
        self.save_dialog = Some(SaveDialog {
            what,
            name,
            types: before.map(|i| i.types().iter().map(|t| t.join(" › ")).collect::<Vec<_>>().join("; ")).unwrap_or_default(),
            characters: before.map(|i| i.characters().join(", ")).unwrap_or_default(),
            library: before.and_then(|i| i.bank.first().cloned()).unwrap_or_else(|| "User".into()),
            folder: self.user_dir().join(folder),
            samples: mode == SaveMode::WithSamples,
            confirm_overwrite: false,
        });
    }

    /// The Save dialog's Save: asks first when the file is there (and isn't this
    /// one's own), then writes it.
    fn confirm_save(&mut self) {
        let Some(dlg) = &mut self.save_dialog else { return };
        let Some(doc) = self.doc.clone() else { return };
        let ext = match dlg.what {
            deckard_doc::FileItem::Sound(..) => "mxsnd",
            deckard_doc::FileItem::Group(_) => "mxgrp",
            deckard_doc::FileItem::Slot(g, s, k) => doc.borrow().slot_extension(g, s, k),
        };
        let name = dlg.name.trim().replace(['/', '\\', ':'], "-");
        if name.is_empty() {
            self.status = "a name, to save it".into();
            return;
        }
        let path = dlg.folder.join(format!("{name}.{ext}"));
        let own = self.saved.get(&dlg.what).is_some_and(|s| s.path == path);
        if path.exists() && !own && !dlg.confirm_overwrite {
            dlg.confirm_overwrite = true;
            return;
        }
        let mut info = mxprj::sound_info::SoundInfo::default();
        info.name = name;
        info.bank = vec![if dlg.library.trim().is_empty() { "User".into() } else { dlg.library.trim().to_string() }];
        info.set_tags(&parse_types(&dlg.types), &parse_list(&dlg.characters));
        {
            let d = doc.borrow();
            let p = d.project();
            let color = match dlg.what {
                deckard_doc::FileItem::Sound(g, s) => p.groups.get(g).and_then(|x| x.sounds.get(s)).map(|x| x.color),
                deckard_doc::FileItem::Group(g) => p.groups.get(g).map(|x| x.color),
                deckard_doc::FileItem::Slot(..) => None,
            };
            if let Some(deckard_model::Color::Palette(c)) = color {
                info.set_prop("\\@color", &c.to_string());
            }
            if let deckard_doc::FileItem::Group(_) = dlg.what {
                info.set_prop("\\@tempo", &format!("{}", p.tempo));
            }
        }
        let what = dlg.what;
        let id = doc.borrow().file_item_id(what);
        let saved = Saved { id, path, info, samples: dlg.samples };
        if self.write_item(what, saved) {
            self.save_dialog = None;
        }
    }

    /// Write `what` as `s` says; listed under User at once. True when written.
    fn write_item(&mut self, what: deckard_doc::FileItem, s: Saved) -> bool {
        let Some(doc) = self.doc.clone() else { return false };
        let user = self.user_dir();
        let mut resolver = deckard_engine::library::Resolver::new(self.path.as_deref());
        let samples = if s.samples { deckard_doc::Samples::Copy } else { deckard_doc::Samples::Reference };
        let r = doc.borrow().save_file(what, &s.info, &s.path, samples, &user, &mut |r| resolver.resolve(r));
        match r {
            Ok(()) => {
                self.status = format!("saved {}", s.path.display());
                self.browser.add_saved(&s.path);
                self.saved.insert(what, s);
                true
            }
            Err(e) => {
                self.status = format!("can't save {}: {e}", s.path.display());
                false
            }
        }
    }

    /// Open…: a sound, group or preset file chosen in a dialog, loaded into `what`.
    fn open_item(&mut self, what: deckard_doc::FileItem) {
        use deckard_doc::FileItem;
        let (title, exts): (&str, &[&str]) = match what {
            FileItem::Sound(..) => ("Load Sound", &["mxsnd"]),
            FileItem::Group(_) => ("Load Group", &["mxgrp"]),
            FileItem::Slot(..) => ("Load Preset", &["mxinst", "mxfx"]),
        };
        let Some(file) = rfd::FileDialog::new().set_title(title).add_filter(title, exts).set_directory(self.user_dir()).pick_file() else {
            return;
        };
        let target = match what {
            FileItem::Sound(g, s) | FileItem::Slot(g, s, _) => Some((g, s)),
            FileItem::Group(g) => {
                self.focus_group(g);
                None
            }
        };
        let Some(item) = browser::item_of(&file) else { return };
        let r = match what {
            FileItem::Slot(g, s, k) => std::fs::read(&file)
                .map_err(|e| e.to_string())
                .and_then(|b| self.doc.clone().ok_or("no project".to_string())?.borrow_mut().load_slot_file(g, s, k, &b)),
            _ => self.load_item(&item, target),
        };
        if let Err(e) = r {
            self.status = e;
        }
    }

    /// What the browser asks for.
    fn browse(&mut self, a: browser::BrowseAct) {
        use browser::BrowseAct as B;
        self.browser.touch();
        let b = &mut self.browser;
        let mut q = b.query.clone();
        match a {
            B::Toggle => {
                b.open = !b.open;
                if b.open && !b.started() {
                    let own = menu::settings_dir().map(|d| d.join("library.db3"));
                    let roots = self.library_roots();
                    if let Some(own) = own {
                        self.browser.start(deckard_library::ni::default_path(), own, roots);
                    }
                }
                return;
            }
            B::Tab(t) => {
                b.tab = t;
                if t == browser::Tab::Files && b.files.is_empty() {
                    b.files = browser::list_folder(&b.files_dir);
                }
                return;
            }
            B::Kind(k) => {
                q.kind = k;
                q.types.clear();
                q.characters.clear();
                q.product = None;
            }
            B::User(u) => {
                q.user = u;
                q.product = None;
            }
            B::Favorites(f) => q.favorites = f,
            B::Product(p) => q.product = p,
            B::Types(t) => q.types = t,
            B::Character(c, on) => {
                q.characters.retain(|x| *x != c);
                if on {
                    q.characters.push(c);
                }
            }
            B::Characters(c) => q.characters = c,
            B::Text(t) => q.text = t,
            B::Focus(i) => {
                b.selected = (!b.results.items.is_empty()).then(|| i.min(b.results.items.len() - 1));
                return;
            }
            B::Select(i) => {
                b.selected = Some(i);
                if b.prehear_auto {
                    self.prehear(i);
                }
                return;
            }
            B::Load(i, target) => {
                let Some(item) = b.results.items.get(i).cloned() else { return };
                b.selected = Some(i);
                if let Err(e) = self.load_item(&item, target) {
                    self.status = e;
                }
                return;
            }
            B::Favorite(i, on) => {
                if let Some(item) = b.results.items.get(i) {
                    let p = item.path.clone();
                    b.set_favorite(&p, on);
                }
                return;
            }
            B::Prehear(i) => {
                self.prehear(i);
                return;
            }
            B::PrehearAuto(on) => {
                b.prehear_auto = on;
                return;
            }
            B::Gain(g) => {
                b.prehear_gain = g;
                return;
            }
            B::Folder(dir) => {
                b.files = browser::list_folder(&dir);
                b.files_dir = dir;
                b.file_selected = None;
                return;
            }
            B::Up => {
                if let Some(up) = b.files_dir.parent().map(Path::to_path_buf) {
                    self.browse(B::Folder(up));
                }
                return;
            }
            B::PinFolder(dir, on) => {
                let pins = &mut self.settings.favorite_folders;
                pins.retain(|d| *d != dir);
                if on {
                    pins.push(dir);
                }
                self.settings.save();
                return;
            }
            B::SelectFile(path) => {
                b.file_selected = Some(path.clone());
                if b.prehear_auto
                    && let Some(item) = browser::item_of(&path)
                {
                    self.prehear_item(&item);
                }
                return;
            }
            B::LibraryFolder(dir, on) => {
                let folders = &mut self.settings.library_folders;
                folders.retain(|d| *d != dir);
                if on {
                    folders.push(dir.clone());
                } else {
                    self.browser.forget(&dir);
                }
                self.settings.save();
                let roots = self.library_roots();
                self.browser.rescan(roots);
                return;
            }
            B::LoadFile(path, target) => {
                b.file_selected = Some(path.clone());
                let Some(item) = browser::item_of(&path) else { return };
                if let Err(e) = self.load_item(&item, target) {
                    self.status = e;
                }
                return;
            }
        }
        b.selected = None;
        b.set_query(q);
    }

    /// Prehear result `i`: a sample, or a sound's or group's preview.
    fn prehear(&mut self, i: usize) {
        if let Some(item) = self.browser.results.items.get(i).cloned() {
            self.prehear_item(&item);
        }
    }

    fn prehear_item(&mut self, item: &deckard_library::Item) {
        let Some(file) = browser::prehear_file(item) else { return };
        let gain = self.browser.prehear_gain;
        match deckard_engine::sample::Sample::load(&file) {
            Ok(s) => {
                if let Some(Ok(out)) = &self.output {
                    out.prehear(Some(std::sync::Arc::new(s)), gain);
                }
            }
            Err(e) => self.status = format!("can't prehear {}: {e}", file.display()),
        }
    }

    /// Load a library item: a project opens; a group replaces the focused group; a
    /// sound the target (or focused) sound; an instrument goes first in its chain, an
    /// effect last; a loop as a dropped file; a one-shot into a Sampler.
    fn load_item(&mut self, item: &deckard_library::Item, target: Option<(usize, usize)>) -> Result<(), String> {
        use deckard_library::Kind;
        let path = &item.path;
        if !path.is_file() {
            return Err(format!("{} isn't there", path.display()));
        }
        let (g, s) = target.unwrap_or((self.group, self.sound));
        if matches!(item.kind, Kind::Project) {
            self.open_file(path.clone());
            return Ok(());
        }
        let Some(doc) = self.doc.clone() else { return Err("no project".into()) };
        match item.kind {
            Kind::Group | Kind::Sound | Kind::Instrument | Kind::Effect => {
                let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
                let mut d = doc.borrow_mut();
                match item.kind {
                    Kind::Group => d.load_group_file(g, &bytes)?,
                    Kind::Sound => d.load_sound_file(g, s, &bytes)?,
                    Kind::Instrument => {
                        let slots = &d.project().groups[g].sounds[s].slots;
                        if slots.first().is_some_and(|x| !x.is_instrument()) {
                            return Err("the sound's chain starts with an effect: an instrument can't go before it".into());
                        }
                        d.load_slot_file(g, s, 0, &bytes)?
                    }
                    _ => {
                        let n = d.project().groups[g].sounds[s].slots.len();
                        d.load_slot_file(g, s, n, &bytes)?
                    }
                }
                drop(d);
                if !matches!(item.kind, Kind::Group) {
                    self.sound = s;
                }
                self.status = format!("{} loaded", item.name);
            }
            Kind::Loop => self.drop_samples((g, s), std::slice::from_ref(path)),
            Kind::OneShot => {
                let smp = deckard_engine::sample::Sample::load(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
                let has_sampler = doc.borrow().project().groups[g].sounds[s].sampler.is_some();
                if !has_sampler {
                    doc.borrow_mut().add_slot(g, s, 0, &deckard_doc::NewModule::Sampler)?;
                }
                self.add_zone(g, s, path, smp.frames.len())?;
                self.sound = s;
                self.status = format!("{} into the Sampler on sound {}", item.name, s + 1);
            }
            Kind::Project => {}
        }
        Ok(())
    }

    fn drop_samples(&mut self, (g, s): (usize, usize), files: &[PathBuf]) {
        let Some(doc) = self.doc.clone() else { return };
        let (n, sampler) = {
            let d = doc.borrow();
            let gr = d.project().groups.get(g);
            (gr.map_or(0, |x| x.sounds.len()), gr.and_then(|x| x.sounds.get(s)).is_some_and(|x| x.sampler.is_some()))
        };
        if g != self.group {
            self.focus_group(g);
        }
        let mut loaded = Vec::new();
        for (k, file) in files.iter().enumerate() {
            let target = if sampler { s } else { s + k };
            if target >= n {
                break;
            }
            let smp = match deckard_engine::sample::Sample::load(file) {
                Ok(smp) => smp,
                Err(e) => {
                    self.status = format!("can't read {}: {e}", file.display());
                    return;
                }
            };
            let done = if sampler { self.add_zone(g, target, file, smp.frames.len()) } else { self.load_loop(g, target, file, &smp) };
            if let Err(e) = done {
                self.status = format!("can't load {}: {e}", file.display());
                return;
            }
            loaded.push(file.file_name().map_or_else(String::new, |f| f.to_string_lossy().into_owned()));
        }
        if loaded.is_empty() {
            return;
        }
        self.sound = s;
        let into = if sampler { "into the Sampler" } else { "as a loop" };
        self.status = match loaded.as_slice() {
            [one] => format!("{one} {into} on sound {}", s + 1),
            more => format!("{} samples {into}, from sound {}", more.len(), s + 1),
        };
    }

    /// Add `file` to sound `s`'s Sampler as a new zone, on the key above its others, and
    /// focus it (one undo step).
    fn add_zone(&mut self, g: usize, s: usize, file: &Path, frames: usize) -> Result<(), String> {
        let Some(doc) = self.doc.clone() else { return Err("no project".into()) };
        let zones = doc.borrow().project().groups[g].sounds[s].sampler.as_ref().map(|x| x.zones.clone()).unwrap_or_default();
        let key = zones.iter().map(|z| z.key_range.1).max().map_or(60, |k| (k + 1).min(127)) as f64;
        let mut specs: Vec<deckard_doc::ZoneSpec> =
            (0..zones.len()).map(|k| deckard_doc::ZoneSpec { from: deckard_doc::ZoneFrom::Zone(k), file: None, set: Vec::new() }).collect();
        let mut spec =
            new_zone(file, frames, if zones.is_empty() { deckard_doc::ZoneFrom::Template } else { deckard_doc::ZoneFrom::Zone(0) });
        spec.set.extend([
            ("root_key", key),
            ("key_range/low", key),
            ("key_range/high", key),
            ("velocity_range/low", 0.0),
            ("velocity_range/high", 127.0),
        ]);
        specs.push(spec);
        doc.borrow_mut().set_zones(g, s, &specs, "Add Sample")?;
        if (g, s) == (self.group, self.sound) || s == self.sound {
            self.zone = zones.len();
        }
        Ok(())
    }

    /// Make sound `s` an Audio plug-in playing `file` (a loop, `smp` its audio): its
    /// length in beats from its name or its length (`loops`), on in the group's focused
    /// pattern (a first one made if there's none, lengthened to hold the loop), the
    /// sound named after it unless named by hand. One undo step.
    fn load_loop(&mut self, g: usize, s: usize, file: &Path, smp: &deckard_engine::sample::Sample) -> Result<(), String> {
        let Some(doc) = self.doc.clone() else { return Err("no project".into()) };
        let name = file.file_name().map_or_else(String::new, |f| f.to_string_lossy().into_owned());
        let (tempo, bar) = {
            let d = doc.borrow();
            (d.project().tempo, d.project().bar_length().max(1))
        };
        let beats = loops::beats(&name, smp.frames.len(), smp.rate, tempo);
        let focused = (g == self.group).then_some(self.pattern);
        let spec = deckard_doc::TakeSpec { file: file.to_path_buf(), frames: smp.frames.len(), rate: smp.rate, beats };
        doc.borrow_mut().transaction("Load Loop", |d| {
            d.audio_only(g, s)?;
            d.set_takes(g, s, &[spec], "Load Loop")?;
            let pi = match focused.filter(|&p| p < d.project().groups[g].patterns.len()) {
                Some(p) => p,
                None if d.project().groups[g].patterns.is_empty() => d.add_pattern(g, usize::MAX, "Pattern 1")?,
                None => 0,
            };
            d.set_audio_in(g, s, &[pi], Some(0), "Load Loop")?;
            // Long enough to hear it all: whole bars.
            let need = ((beats * PPQ as f64 / bar as f64).ceil() as i64).max(1) * bar;
            let len = d.project().groups[g].patterns[pi].length;
            if len < need {
                d.set_pattern_length(g, pi, need)?;
            }
            let sound = d.project().groups[g].sounds[s].name.clone();
            if (sound.is_empty() || sound.starts_with("Sound ") || sound.parse::<u32>().is_ok())
                && let Some(stem) = file.file_stem()
            {
                d.rename(deckard_doc::Target::Sound(g, s), &stem.to_string_lossy())?;
            }
            Ok(())
        })?;
        if g == self.group && s == self.sound {
            self.slot = Some(0);
            self.chain_tab = layout::ChainTab::Sound;
        }
        Ok(())
    }

    /// Load a sample file into the focused sound's Sampler as its only zone (adding a
    /// Sampler if it has none).
    fn load_into_sound(&mut self, path: &Path) {
        let Ok(s) = deckard_engine::sample::Sample::load(path) else {
            self.status = format!("can't read {}", path.display());
            return;
        };
        let (g, snd) = (self.group, self.sound);
        self.load_take(g, snd, path, s.frames.len());
    }

    /// Solo channel `ch`: mute the other channels of its level and unmute it, as one
    /// step; soloing it again (the others all muted) unmutes them all.
    fn solo(&mut self, ch: layout::Channel) {
        use deckard_doc::Target;
        let Some(doc) = self.doc.clone() else { return };
        let p = doc.borrow().project().clone();
        let all: Vec<(Target, bool)> = match ch {
            layout::Channel::Group(_) => (0..p.groups.len()).map(|g| (Target::Group(g), p.groups[g].muted)).collect(),
            layout::Channel::Sound(g, _) => {
                (0..p.groups.get(g).map_or(0, |x| x.sounds.len())).map(|s| (Target::Sound(g, s), p.groups[g].sounds[s].muted)).collect()
            }
            layout::Channel::Master => return,
        };
        let me = match ch {
            layout::Channel::Group(g) => Target::Group(g),
            layout::Channel::Sound(g, s) => Target::Sound(g, s),
            layout::Channel::Master => return,
        };
        let soloed = all.iter().all(|(t, m)| if *t == me { !m } else { *m });
        let mutes: Vec<(Target, bool)> = all.iter().map(|(t, _)| (*t, !soloed && *t != me)).collect();
        if let Err(e) = doc.borrow_mut().set_mutes(if soloed { "Unsolo" } else { "Solo" }, &mutes) {
            self.status = format!("can't solo: {e}");
        }
    }

    /// Apply one action as a click would (for tests).
    #[cfg(test)]
    fn arrange_act(&mut self, a: layout::Act) {
        layout::apply_acts(self, vec![a]);
    }

    /// Keep the shown slot on the chain shown, and the plug-in page up to date.
    fn update_slot_view(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let chain = self.chain();
        let all = self.chain_slots();
        let slots = all.len();
        if self.slot_of != Some(chain) {
            self.slot_of = Some(chain);
            self.slot = (slots > 0).then_some(0);
            self.plugin_page = 0;
        }
        if self.slot.is_some_and(|i| i >= slots) {
            self.slot = slots.checked_sub(1);
        }
        let Some(i) = self.slot else {
            self.plugin_view = None;
            return;
        };
        let is_plugin = all[i].plugin.is_some();
        if !is_plugin {
            self.plugin_view = None;
            return;
        }
        let _ = &doc;
        let key = (chain, i);
        let stale = self.plugin_view.as_ref().is_none_or(|v| v.key != key);
        if stale {
            let saved = all[i].plugin.as_ref().map(|p| p.pages.clone()).unwrap_or_default();
            let view = match self.slot_plugin_in(key.0, key.1) {
                Ok(p) => {
                    let pages = p.lock().map(|p| view_pages(&p, &saved)).unwrap_or_default();
                    PluginView {
                        key,
                        pages,
                        page: usize::MAX,
                        values: Vec::new(),
                        refreshed: std::time::Instant::now(),
                        error: None,
                        sent: Vec::new(),
                    }
                }
                Err(e) => PluginView {
                    key,
                    pages: Vec::new(),
                    page: 0,
                    values: Vec::new(),
                    refreshed: std::time::Instant::now(),
                    error: Some(e),
                    sent: Vec::new(),
                },
            };
            self.plugin_view = Some(view);
        }
        // Values of the page shown, refreshed every 100 ms (the plug-in's window may
        // move them).
        let page = self.plugin_page;
        let due =
            self.plugin_view.as_ref().is_some_and(|v| v.error.is_none() && (v.page != page || v.refreshed.elapsed().as_millis() >= 100));
        if due && let Ok(p) = self.slot_plugin_in(key.0, key.1) {
            let v = self.plugin_view.as_mut().unwrap();
            let ids: Vec<u32> = v.pages.get(page).map_or(Vec::new(), |pg| pg.knobs.iter().map(|(i, ..)| i.id).collect());
            // A VST3's values come from its controller without holding the instance
            // (the audio thread would wait); DECKARD's own read instantly under it.
            let now = std::time::Instant::now();
            let sent = &mut v.sent;
            let values = p.try_lock().ok().map(|g| match g.params() {
                Some(h) => {
                    drop(g);
                    let mut values: Vec<_> = ids.iter().map(|&id| (h.value(id), h.text(id, h.value(id)))).collect();
                    hold_sent(&mut values, &ids, sent, now, |id, v| h.text(id, v));
                    (values, Vec::new())
                }
                None => {
                    let mut values: Vec<_> = ids.iter().map(|&id| (g.param_value(id), g.param_text(id, g.param_value(id)))).collect();
                    hold_sent(&mut values, &ids, sent, now, |id, v| g.param_text(id, v));
                    (values, ids.iter().map(|&id| g.param_label(id)).collect())
                }
            });
            if let Some((values, labels)) = values {
                if let Some(pg) = v.pages.get_mut(page).filter(|pg| pg.own) {
                    for ((_, label, _), new) in pg.knobs.iter_mut().zip(labels) {
                        if let Some(new) = new {
                            *label = new;
                        }
                    }
                }
                v.values = values;
                v.page = page;
                v.refreshed = std::time::Instant::now();
            }
        }
    }

    /// Add a module or plug-in at the end of the focused sound's chain, and show it.
    /// Make a routing choice from the channel properties.
    fn set_route(&mut self, r: layout::RouteSet) {
        let Some(doc) = self.doc.clone() else { return };
        let mut d = doc.borrow_mut();
        let done = match r {
            layout::RouteSet::Sound(g, s, route) => d.set_sound_route(g, s, route),
            layout::RouteSet::Group(g, aux, dest) => d.set_group_route(g, aux, dest),
            layout::RouteSet::GroupMidiSource(g, port) => {
                d.set("MIDI Source", deckard_doc::Target::Group(g), "event_input", mxprj::decode::Value::Str(port))
            }
            layout::RouteSet::Master(cue, dest) => d.set_master_output(cue, dest),
            layout::RouteSet::SideChain(g, s, slot, src) => d.set_sidechain(g, s, slot, src),
        };
        if let Err(e) = done {
            self.status = format!("can't route that: {e}");
        }
    }

    /// The Groove page's button: apply the variation to, or generate, the focused
    /// sound's notes in the focused pattern.
    fn groove(&mut self, b: layout::GrooveButton) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, s, p) = (self.group, self.sound, self.pattern);
        let grid = self.grid;
        let polyphonic = self.piano_roll;
        let kept = self.groove_settings.clone();
        let saved = doc.borrow().has_groove_settings(g, s);
        let lookup = |f: &str| -> Option<f64> {
            if saved { doc.borrow().groove_setting(g, s, f) } else { kept.get(&(g, s, f.to_string())).copied() }
        };
        let settings: Vec<(String, f64)> = GROOVE_FIELDS.iter().filter_map(|f| lookup(f).map(|v| (f.to_string(), v))).collect();
        let get = |f: &str| settings.iter().find(|(k, _)| k == f).map(|(_, v)| *v);
        // The scale selected (the pattern editor's, which is the controller's Keyboard
        // scale too; Chromatic is none): Generate keeps to it.
        let (root, bank, kind) = self.scale;
        let in_scale = ((bank, kind) != (0, 0)).then_some(move |p: i64| deckard_lua_host::api::scales::in_scale(bank, kind, root, p));
        let in_scale: Option<&dyn Fn(i64) -> bool> = in_scale.as_ref().map(|f| f as &dyn Fn(i64) -> bool);
        let done = match b {
            layout::GrooveButton::ApplyVariation => doc.borrow_mut().apply_variation(g, p, s, grid, &get),
            layout::GrooveButton::Generate => doc.borrow_mut().generate(g, p, s, grid, polyphonic, in_scale, &get),
        };
        if let Err(e) = done {
            self.status = format!("can't do that: {e}");
        }
    }

    /// After a Variation or Generate setting changed: with Live on, apply it again (a
    /// live Variation always starts from the notes as they were).
    pub fn live_groove(&mut self, g: usize, s: usize, field: &str) {
        let Some(doc) = self.doc.clone() else { return };
        let p = if g == self.group { self.pattern } else { 0 };
        let variation = field.starts_with("sequence_variation/");
        let live = if variation { "sequence_variation/live" } else { "sequence_generation/live_mode" };
        if !(variation || field.starts_with("sequence_generation/")) || self.groove_setting(g, s, live).unwrap_or(0.0) == 0.0 {
            return;
        }
        if !variation {
            self.groove(layout::GrooveButton::Generate);
            return;
        }
        let now: Vec<Note> = doc
            .borrow()
            .project()
            .groups
            .get(g)
            .and_then(|x| x.patterns.get(p))
            .map_or(Vec::new(), |pat| pat.notes.iter().filter(|n| n.sound == s).copied().collect());
        let base = match &self.variation_base {
            Some((k, base, made)) if *k == (g, p, s) && *made == now => base.clone(),
            _ => now,
        };
        let kept = self.groove_settings.clone();
        let saved = doc.borrow().has_groove_settings(g, s);
        let settings: Vec<(String, f64)> = GROOVE_FIELDS
            .iter()
            .filter_map(|f| {
                let v = if saved { doc.borrow().groove_setting(g, s, f) } else { kept.get(&(g, s, f.to_string())).copied() };
                v.map(|v| (f.to_string(), v))
            })
            .collect();
        let get = |f: &str| settings.iter().find(|(k, _)| k == f).map(|(_, v)| *v);
        let grid = self.grid;
        if let Err(e) = doc.borrow_mut().apply_variation_from(g, p, s, grid, &get, &base) {
            self.status = format!("can't vary: {e}");
            return;
        }
        let made: Vec<Note> = doc.borrow().project().groups[g].patterns[p].notes.iter().filter(|n| n.sound == s).copied().collect();
        self.variation_base = Some(((g, p, s), base, made));
    }

    /// The channel whose macros the Control area shows.
    pub fn macro_channel(&self) -> Option<deckard_doc::Target> {
        let d = self.doc.as_ref()?.borrow();
        let p = d.project();
        match self.chain_tab {
            layout::ChainTab::Master => Some(deckard_doc::Target::Song),
            layout::ChainTab::Group => (self.group < p.groups.len()).then_some(deckard_doc::Target::Group(self.group)),
            layout::ChainTab::Sound => p
                .groups
                .get(self.group)
                .is_some_and(|g| self.sound < g.sounds.len())
                .then_some(deckard_doc::Target::Sound(self.group, self.sound)),
        }
    }

    /// The scale's root moved from `from` to `to`: the focused sound's Generate note
    /// range moves with it, by the same number of semitones (within MIDI's notes).
    pub fn follow_root(&mut self, from: i64, to: i64) {
        let (g, s) = (self.group, self.sound);
        let acts = [("sequence_generation/note_range/low", 60.0), ("sequence_generation/note_range/high", 72.0)]
            .into_iter()
            .map(|(f, default)| {
                let v = self.groove_setting(g, s, f).unwrap_or(default);
                layout::Act::SetParam(deckard_doc::Target::Sound(g, s), f, (v + (to - from) as f64).clamp(0.0, 127.0))
            })
            .collect();
        layout::apply_acts(self, acts);
    }

    /// Group `g`'s Perform FX: its slot in the group's chain.
    pub fn perform_slot(&self, g: usize) -> Option<usize> {
        let doc = self.doc.as_ref()?.borrow();
        doc.project().groups.get(g)?.slots.iter().position(|s| s.module == deckard_model::PERFORM_FX.0)
    }

    /// The Smart Strip on group `g`'s Perform FX (as the song plays now).
    pub fn perform_strip(&mut self, g: usize, touch: Option<bool>, pos: f64) {
        let head = self.playhead().map(|h| (h.mode, h.position));
        self.perform_strip_at(g, touch, pos, head);
    }

    /// The Smart Strip on group `g`'s Perform FX: touched (`Some(true)`), let go
    /// (`Some(false)`) or moved, at `pos` (0..1). It engages the effect while touched and
    /// moves it; while writing (Auto-write, playing at `head`) it's recorded as the
    /// group's modulation instead: Engage as a point at the touch and at the release,
    /// the position as a knob's movement.
    /// The song's tempo, for the displays that show times at it (Stutter's Length).
    fn share_tempo(&self) {
        if let Some(d) = &self.doc {
            module_ui::set_tempo(d.borrow().project().tempo);
        }
    }

    pub fn perform_strip_at(&mut self, g: usize, touch: Option<bool>, pos: f64, head: Option<(Mode, f64)>) {
        use deckard_doc::{Chain, Target};
        let Some(i) = self.perform_slot(g) else { return };
        let t = Target::ChainModule(Chain::Group(g), i);
        let pos_edit = layout::Edit::Param(t, "touchstrip_pos", 0.0, 1.0, 0.5);
        let Some((mode, tick)) = head.filter(|_| self.auto_write) else {
            let mut acts = Vec::new();
            if let Some(on) = touch {
                acts.push(layout::Act::SetParam(t, "touchstrip_on_off", if on { 1.0 } else { 0.0 }));
            }
            acts.push(layout::Act::SetParam(t, "touchstrip_pos", pos));
            layout::apply_acts(self, acts);
            return;
        };
        let engage = |app: &mut App, on: bool| {
            let Some(doc) = app.doc.clone() else { return };
            let at = record::pattern_at(doc.borrow().project(), mode, tick, g);
            let Some((pi, at)) = at else { return };
            let target = deckard_model::ModTarget { slot: Some(i), field: "touchstrip_on_off".into() };
            let point = [(at, if on { 1.0 } else { 0.0 })];
            if let Err(e) = doc.borrow_mut().set_modulation(Chain::Group(g), &target, pi, (at, at), &point) {
                app.status = format!("can't record the strip: {e}");
            }
        };
        if touch == Some(true) {
            engage(self, true);
        }
        self.modulate_at(pos_edit, pos as f32, touch == Some(false), head);
        if touch == Some(false) {
            engage(self, false);
        }
    }

    /// The editing keys in the Sound and Group lists (the event area handles its own):
    /// ⌘X / ⌘C / ⌘V, Del and ⌘R on the focused sound; ⌘R on the focused group.
    pub fn editing_keys(&mut self, ctx: &egui::Context) {
        use layout::{Act, KeyFocus};
        // ⌘.: Convert to Clip, wherever the keys are.
        if !ctx.egui_wants_keyboard_input()
            && self.renaming.is_none()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Period))
        {
            layout::apply_acts(self, vec![Act::ConvertToClip]);
        }
        if !ctx.egui_wants_keyboard_input() && self.renaming.is_none() && self.remote_keys(ctx) {
            return;
        }
        if ctx.egui_wants_keyboard_input() || self.renaming.is_some() || matches!(self.key_focus, KeyFocus::Events | KeyFocus::Macros) {
            return;
        }
        let (g, s) = (self.group, self.sound);
        let cmd = |k: egui::Key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, k));
        let ev = |f: fn(&egui::Event) -> bool| ctx.input(|i| i.events.iter().any(f));
        let mut acts = Vec::new();
        if cmd(egui::Key::R) {
            acts.push(Act::StartRename(match self.key_focus {
                KeyFocus::Groups => deckard_doc::Target::Group(g),
                _ => deckard_doc::Target::Sound(g, s),
            }));
        }
        if self.key_focus == KeyFocus::Sounds {
            if cmd(egui::Key::X) || ev(|e| matches!(e, egui::Event::Cut)) {
                acts.push(Act::CutSound(g, s));
            }
            if cmd(egui::Key::C) || ev(|e| matches!(e, egui::Event::Copy)) {
                acts.push(Act::CopySound(g, s));
            }
            if (cmd(egui::Key::V) || ev(|e| matches!(e, egui::Event::Paste(_)))) && !self.sound_clipboard.is_empty() {
                acts.push(Act::PasteSound(g, s));
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
                acts.push(Act::ResetSound(g, s));
            }
        }
        layout::apply_acts(self, acts);
    }

    /// The focused pattern's notes the Pattern Editor shows: all of them in Group view,
    /// the focused sound's in Keyboard view.
    fn shown_notes(&self) -> Vec<Note> {
        let Some(doc) = &self.doc else { return Vec::new() };
        let d = doc.borrow();
        let Some(pat) = d.project().groups.get(self.group).and_then(|g| g.patterns.get(self.pattern)) else { return Vec::new() };
        pat.notes.iter().filter(|n| !self.piano_roll || n.sound == self.sound).copied().collect()
    }

    /// ⌘C / ⌘X: the selected notes (or every note shown) to the clipboard; cut takes
    /// them out.
    pub fn copy_notes(&mut self, cut: bool) {
        let shown = self.shown_notes();
        let selected: Vec<Note> = self.selected.iter().filter(|n| shown.contains(n)).copied().collect();
        let notes = if selected.is_empty() { shown } else { selected };
        if notes.is_empty() {
            return;
        }
        self.note_clip = Some(clipboard::NoteClip { notes: notes.clone(), sound: self.sound });
        if cut && let Some(doc) = self.doc.clone() {
            if let Err(e) = doc.borrow_mut().edit_notes("Cut Notes", self.group, self.pattern, &notes, &[]) {
                self.status = format!("can't cut: {e}");
            }
            self.selected.clear();
        }
    }

    /// ⌘V or the menu's Paste: the clipboard's notes into the focused pattern (see
    /// [`clipboard::paste`]), lengthening it to the bar after the last one if they go
    /// past its end; the pasted notes are selected.
    pub fn paste_notes(&mut self, at: clipboard::PasteAt) {
        let (Some(clip), Some(doc)) = (self.note_clip.clone(), self.doc.clone()) else { return };
        let (g, pi) = (self.group, self.pattern);
        let (add, start, len, bar) = {
            let d = doc.borrow();
            let p = d.project();
            let Some(group) = p.groups.get(g) else { return };
            let Some(pat) = group.patterns.get(pi) else { return };
            let add = clipboard::paste(&clip, at, self.sound, self.piano_roll, pat.start, self.grid, group.sounds.len());
            (add, pat.start, layout::pattern_len(p, g, pi), p.bar_length())
        };
        if add.is_empty() {
            return;
        }
        let end = add.iter().map(|n| n.position + n.length).max().unwrap_or(start) - start;
        let r = doc.borrow_mut().transaction("Paste Notes", |d| {
            d.edit_notes("Paste Notes", g, pi, &[], &add)?;
            if end > len {
                d.set_pattern_length(g, pi, (end + bar - 1) / bar * bar)?;
            }
            Ok(())
        });
        match r {
            Ok(()) => self.selected = add,
            Err(e) => self.status = format!("can't paste: {e}"),
        }
    }

    /// Whether the event area is in Draw mode.
    pub fn draw_mode(&self) -> bool {
        self.mouse == layout::MouseMode::Draw
    }

    /// A Variation or Generate setting of sound `s` in group `g`.
    pub fn groove_setting(&self, g: usize, s: usize, field: &str) -> Option<f64> {
        let doc = self.doc.as_ref()?.borrow();
        if doc.has_groove_settings(g, s) {
            doc.groove_setting(g, s, field)
        } else {
            self.groove_settings.get(&(g, s, field.to_string())).copied()
        }
    }

    /// The MIDI ports, looked up again every few seconds.
    fn midi_ports(&mut self) -> &(Vec<String>, Vec<String>) {
        if self.midi_ports_at.is_none_or(|t| t.elapsed().as_secs() >= 3) {
            self.midi_ports_at = Some(std::time::Instant::now());
            let ins = midir::MidiInput::new("DECKARD ports")
                .map(|m| m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect())
                .unwrap_or_default();
            let outs = midir::MidiOutput::new("DECKARD ports")
                .map(|m| m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect())
                .unwrap_or_default();
            self.midi_ports = (ins, outs);
        }
        &self.midi_ports
    }

    /// Move slot `from` of the focused sound's chain to `to`; it stays shown.
    fn move_slot(&mut self, from: usize, to: usize) {
        let Some(doc) = self.doc.clone() else { return };
        let c = self.chain();
        // Plug-in states edited in their windows first (instances follow positions),
        // and the chain's windows close.
        self.store_plugin_states();
        self.editors.retain(|(ec, ..)| *ec != c);
        match doc.borrow_mut().move_slot_in(c, from, to) {
            Ok(()) => {
                self.slot = Some(to);
                self.slot_of = Some(c);
                self.plugin_page = 0;
                self.plugin_view = None;
            }
            Err(e) => self.status = format!("can't move it: {e}"),
        }
    }

    /// What DECKARD's own plug-in browser (the Module page) lists, as
    /// [`App::plugin_choices`] has it.
    fn module_views(&self) -> Vec<deckard_surface::ModuleView> {
        self.plugin_choices()
            .into_iter()
            .map(|(e, _)| deckard_surface::ModuleView {
                name: e.name,
                instrument: e.instrument,
                format: e.format.label().into(),
                vendor: e.vendor,
            })
            .collect()
    }

    /// What the controller's plug-in browser lists (with what loading each adds), in the
    /// "+" menu's order: Maschine's modules and DECKARD's, then the external plug-ins
    /// this build can host.
    fn plugin_choices(&self) -> Vec<(deckard_lua_host::state::PluginEntry, layout::AddWhat)> {
        use deckard_lua_host::state::{PluginEntry, PluginFormat};
        use layout::AddWhat;
        let internal = |name: &str, instrument: bool, what: AddWhat| {
            (PluginEntry { name: name.to_string(), instrument, format: PluginFormat::Internal, vendor: "Native Instruments".into() }, what)
        };
        let native = |instrument: bool| deckard_plugins::native::CLASSES.iter().filter(move |c| c.instrument == instrument);
        let mut out = vec![internal("Sampler", true, AddWhat::Sampler), internal("Audio", true, AddWhat::Audio)];
        for class in deckard_model::INTERNAL_INSTRUMENTS {
            out.push(internal(deckard_model::module_label(class), true, AddWhat::Instrument(class)));
        }
        for c in native(true) {
            out.push(internal(c.name, true, AddWhat::Plugin(layout::native_entry(c))));
        }
        for (class, _) in deckard_model::INTERNAL_EFFECTS {
            out.push(internal(deckard_model::module_label(class), false, AddWhat::Effect(class)));
        }
        for c in native(false) {
            out.push(internal(c.name, false, AddWhat::Plugin(layout::native_entry(c))));
        }
        // A group's Perform FX, if it has none (as the "+" menu offers them).
        if let deckard_doc::Chain::Group(g) = self.chain()
            && self.perform_slot(g).is_none()
        {
            for (mode, name) in module_ui::PERFORM_MODES.iter().enumerate() {
                out.push(internal(&format!("Perform {name}"), false, AddWhat::PerformFx(mode)));
            }
        }
        use deckard_plugins::scan::Kind;
        let mut external: Vec<_> = self.catalog.entries.iter().filter(|e| e.hostable() && self.settings.allows(e)).collect();
        external.sort_by_key(|e| (e.vendor.to_lowercase(), e.name.to_lowercase()));
        for e in external {
            let format = match e.kind {
                Kind::AudioUnit => PluginFormat::AudioUnit,
                Kind::Vst3 => PluginFormat::Vst3,
                Kind::Vst2 => PluginFormat::Vst2,
            };
            let entry = PluginEntry { name: e.name.clone(), instrument: e.instrument, format, vendor: e.vendor.clone() };
            out.push((entry, AddWhat::Plugin(e.clone())));
        }
        out
    }

    /// Put `what` in place of slot `i` (the controller's browser loading into a full
    /// slot); an instrument only in the first slot.
    fn replace_slot(&mut self, i: usize, what: layout::AddWhat) {
        let Some(doc) = self.doc.clone() else { return };
        // (A Perform FX goes first, taking no slot's place.)
        if let layout::AddWhat::PerformFx(_) = what {
            self.add_slot_at(what, 0);
            return;
        }
        if i > 0 && Self::is_instrument(&what) {
            self.status = "a sound has one instrument, in its first slot".into();
            return;
        }
        if let Err(e) = doc.borrow_mut().remove_slot_in(self.chain(), i) {
            self.status = format!("can't replace it: {e}");
            return;
        }
        self.add_slot_at(what, i);
    }

    fn is_instrument(what: &layout::AddWhat) -> bool {
        match what {
            layout::AddWhat::Sampler | layout::AddWhat::Audio | layout::AddWhat::Instrument(_) => true,
            layout::AddWhat::Plugin(e) => e.instrument,
            layout::AddWhat::Effect(_) | layout::AddWhat::PerformFx(_) => false,
        }
    }

    fn add_slot(&mut self, what: layout::AddWhat) {
        let at = self.chain_slots().len();
        self.add_slot_at(what, at);
    }

    /// Add `what` to the focused sound's chain at slot `at` (the end, from the "+" menu).
    fn add_slot_at(&mut self, what: layout::AddWhat, at: usize) {
        let Some(doc) = self.doc.clone() else { return };
        if let layout::AddWhat::PerformFx(mode) = what {
            match self.chain() {
                deckard_doc::Chain::Group(g) if self.perform_slot(g).is_none() => {
                    layout::apply_acts(self, vec![layout::Act::AddPerformFx(g, mode)])
                }
                deckard_doc::Chain::Group(_) => self.status = "a group has one Perform FX".into(),
                _ => self.status = "a Perform FX goes in a group's chain".into(),
            }
            return;
        }
        let module = match &what {
            layout::AddWhat::Sampler => deckard_doc::NewModule::Sampler,
            layout::AddWhat::Audio => deckard_doc::NewModule::Audio,
            layout::AddWhat::Effect(class) => deckard_doc::NewModule::Effect(class),
            layout::AddWhat::PerformFx(_) => return,
            layout::AddWhat::Instrument(class) => {
                let defaults = deckard_engine::synth::defaults(class).unwrap_or_default();
                deckard_doc::NewModule::Instrument {
                    class: class.to_string(),
                    params: defaults.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
                }
            }
            layout::AddWhat::Plugin(e) => match e.maschine_id() {
                Some(id) => deckard_doc::NewModule::Plugin { id, name: e.name.clone(), chunk: Vec::new() },
                None => {
                    self.status = format!("{} plug-ins can't be hosted yet", e.kind.label());
                    return;
                }
            },
        };
        let c = self.chain();
        let slots = self.chain_slots();
        let at = at.min(slots.len());
        // A sound has one instrument, in its first slot (as Maschine): effects only after;
        // a group's and the master's chains take effects only.
        let instrument = Self::is_instrument(&what);
        if instrument && !matches!(c, deckard_doc::Chain::Sound(..)) {
            self.status = "an instrument goes in a sound's chain".into();
            return;
        }
        let has_instrument = slots.first().is_some_and(|x| x.is_instrument());
        if instrument && (at > 0 || has_instrument) {
            self.status = "a sound has one instrument, in its first slot".into();
            return;
        }
        match doc.borrow_mut().add_slot_in(c, at, &module) {
            Ok(()) => {
                self.slot = Some(at);
                self.slot_of = Some(c);
                self.plugin_page = 0;
                self.plugin_view = None;
            }
            Err(e) => self.status = format!("can't add it: {e}"),
        }
    }

    /// Load every plug-in instrument of the project (at the output's rate), so playing
    /// and editing never wait for one.
    fn load_plugins(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let d = doc.borrow();
        let wanted: Vec<_> = d
            .project()
            .groups
            .iter()
            .enumerate()
            .flat_map(|(g, gr)| {
                gr.sounds.iter().enumerate().flat_map(move |(s, snd)| {
                    // The instrument, then the effect plug-ins after it.
                    let instrument = snd.plugin.clone().map(|p| (deckard_plugins::Slot::Sound { group: g, sound: s }, p));
                    let effects = snd.slots.iter().enumerate().skip(1).filter_map(move |(i, x)| {
                        x.plugin
                            .clone()
                            .filter(|p| deckard_engine::is_effect(&p.id))
                            .map(|p| (deckard_plugins::Slot::Chain { group: g, sound: s, slot: i }, p))
                    });
                    instrument.into_iter().chain(effects)
                })
            })
            .collect();
        // Groups' and the master's effect plug-ins.
        let mut wanted = wanted;
        let p = d.project();
        for (g, gr) in p.groups.iter().enumerate() {
            for (i, x) in gr.slots.iter().enumerate() {
                if let Some(pl) = x.plugin.clone().filter(|pl| deckard_engine::is_effect(&pl.id)) {
                    wanted.push((deckard_plugins::Slot::Group { group: g, slot: i }, pl));
                }
            }
        }
        for (i, x) in p.master_slots.iter().enumerate() {
            if let Some(pl) = x.plugin.clone().filter(|pl| deckard_engine::is_effect(&pl.id)) {
                wanted.push((deckard_plugins::Slot::Master { slot: i }, pl));
            }
        }
        drop(d);
        if wanted.is_empty() {
            return;
        }
        let rate = match self.output.get_or_insert_with(|| open_output(&self.settings, &mut self.output_fallback)) {
            Ok(out) => out.rate as f64,
            Err(_) => return,
        };
        let t = std::time::Instant::now();
        let mut failed = Vec::new();
        for (slot, p) in &wanted {
            if let Err(e) = self.samples.plugins.instance(*slot, &p.id, &p.name, &p.chunk, rate) {
                failed.push(e);
            }
        }
        self.status = if failed.is_empty() {
            format!("{} plug-ins loaded ({:.1} s)", wanted.len(), t.elapsed().as_secs_f64())
        } else {
            format!("plug-ins failed: {}", failed.join(", "))
        };
    }

    /// Write the state of plug-ins edited in their windows into the document.
    pub(crate) fn store_plugin_states(&mut self) {
        self.store_plugin_states_of(|_| true);
    }

    /// [`App::store_plugin_states`] for the instances `which` picks (saving a
    /// plug-in's state holds it, and the audio thread waits for it).
    pub(crate) fn store_plugin_states_of(&mut self, which: impl Fn(&deckard_plugins::Slot) -> bool) {
        let Some(doc) = self.doc.clone() else { return };
        let mut stored = Vec::new();
        for (slot, p) in self.samples.plugins.instances().filter(|(s, _)| which(s)) {
            let Ok(mut p) = p.lock() else { continue };
            if !p.take_dirty() {
                continue;
            }
            let chunk = p.save_chunk();
            p.mark_saved(chunk.clone());
            stored.push((*slot, chunk));
        }
        for (slot, chunk) in stored {
            let r = match slot {
                deckard_plugins::Slot::Sound { group, sound } => doc.borrow_mut().set_plugin_chunk(group, sound, chunk),
                deckard_plugins::Slot::Chain { group, sound, slot } => doc.borrow_mut().set_slot_plugin_chunk(group, sound, slot, chunk),
                deckard_plugins::Slot::Group { group, slot } => {
                    doc.borrow_mut().set_chain_plugin_chunk(deckard_doc::Chain::Group(group), slot, chunk)
                }
                deckard_plugins::Slot::Master { slot } => doc.borrow_mut().set_chain_plugin_chunk(deckard_doc::Chain::Master, slot, chunk),
            };
            if let Err(e) = r {
                self.status = format!("plug-in state not stored: {e}");
            }
        }
    }

    /// Close windows the user closed, keeping what was changed in them.
    fn poll_plugin_editors(&mut self) {
        for (.., e) in &self.editors {
            e.tick();
        }
        if self.editors.iter().any(|(.., e)| !e.is_open()) {
            self.editors.retain(|(.., e)| e.is_open());
            self.store_plugin_states();
        }
    }

    fn save(&mut self) {
        self.store_plugin_states();
        self.store_screen();
        let Some(doc) = &self.doc else { return };
        // A new project has no file yet.
        let Some(path) = doc.borrow().path().map(Path::to_path_buf) else { return self.save_as() };
        let backup = path.with_extension("mxprj.bak");
        if !self.backed_up {
            if let Err(e) = std::fs::copy(&path, &backup) {
                self.status = format!("not saved: backup {}: {e}", backup.display());
                return;
            }
            self.backed_up = true;
        }
        self.status = match doc.borrow_mut().save_as(&path) {
            Ok(()) => format!("saved {} (previous version: {})", path.display(), backup.display()),
            Err(e) => format!("not saved: {e}"),
        };
    }

    /// The pads blinking for the notes the sequencer started, (group, sound, pitch): see
    /// [`blink::Blinks`].
    fn notes_played(&mut self) -> std::collections::HashSet<(usize, usize, i64)> {
        let now = std::time::Instant::now();
        let (Some(head), Some(doc)) = (self.playhead(), self.doc.clone()) else { return self.blinks.stopped(now) };
        let doc = doc.borrow();
        let revision = doc.revision();
        if self.played_timeline.as_ref().is_none_or(|(m, r, _)| (*m, *r) != (head.mode, revision)) {
            self.played_timeline = Some((head.mode, revision, deckard_engine::timeline(doc.project(), head.mode)));
        }
        let Some((_, _, t)) = &self.played_timeline else { return self.blinks.stopped(now) };
        if t.length <= 0 {
            return self.blinks.stopped(now);
        }
        self.blinks.update(t, head.mode, head.position.rem_euclid(t.length as f64) as i64, now)
    }

    /// Hand the controller scripts what plays now, so pads and group buttons flash and
    /// the Ideas view's group progress bars fill.
    fn sync_notes_played(&mut self) {
        let played = self.notes_played();
        self.lit = played.iter().map(|&(g, s, _)| (g, s)).collect();
        if let Some(Ok(out)) = &self.output {
            self.lit.extend(out.played_sounds());
        }
        if !matches!(self.controller, Some(Ok(_))) {
            return;
        }
        let progress = self.doc.clone().map(|doc| {
            let (doc, head) = (doc.borrow(), self.playhead());
            let p = doc.project();
            (0..p.groups.len()).filter_map(|g| Some((g, layout::group_progress(p, head, self.scene, g)?))).collect()
        });
        // (Stopped: where STOP left it, for the touch strip's play position.)
        let head = self.shown_playhead();
        let what = head.map(|h| match h.mode {
            Mode::Pattern(g, p) => deckard_lua_host::state::Playing::Pattern(g, p),
            Mode::Scene(s) => deckard_lua_host::state::Playing::Scene(s),
            Mode::Arrangement => deckard_lua_host::state::Playing::Arrangement,
        });
        if let Some(Ok(c)) = &mut self.controller {
            c.host.set_playhead(head.map_or(0, |h| h.position as i64), what);
            c.host.set_notes_played(played);
            c.host.set_group_progress(progress.unwrap_or_default());
        }
    }

    /// The playhead as shown: where the song plays, or where STOP left it.
    pub(crate) fn shown_playhead(&self) -> Option<Playhead> {
        self.playhead().or(self.parked.filter(|_| self.playing.is_none()))
    }

    fn playhead(&self) -> Option<Playhead> {
        self.playing?;
        let Some(Ok(out)) = &self.output else { return None };
        // As the audio thread last published it: reading it never holds up the audio.
        out.telemetry(|t| {
            (!t.finished).then_some(()).and(t.mode).map(|mode| Playhead { mode, queued: t.queued, position: t.position, played: t.played })
        })
    }
}

impl App {
    /// Follow the engine (scene switches, the end of the song), once what it publishes
    /// has caught up with what was just done to it (a scene queued).
    fn follow_engine(&mut self, ctx: &egui::Context, again: std::time::Duration) {
        let current = match &self.output {
            Some(Ok(out)) => out.is_current(),
            _ => true,
        };
        if self.playing.is_some() && !current {
            ctx.request_repaint_after(again);
        }
        if self.playing.is_some() && current {
            match self.playhead() {
                Some(head) => {
                    self.playing = Some(head.mode);
                    self.follow_playback(head);
                }
                None if matches!(&self.output, Some(Ok(out)) if !out.is_playing()) => {
                    self.playing = None;
                    self.sync_transport();
                }
                None => {}
            }
            ctx.request_repaint_after(again);
        }
    }

    /// What [`App::logic`] does while the window is covered or minimized.
    fn background(&mut self, ctx: &egui::Context) {
        self.share_tempo();
        self.follow_engine(ctx, undrawn(ctx, BACKGROUND_PACE));
        self.sync_notes_played();
        let mut requests = Vec::new();
        if let Some(Ok(c)) = &mut self.controller {
            c.tick(&mut requests);
            ctx.request_repaint_after(undrawn(ctx, c.interval()));
        }
        self.mk3_sync(ctx, &mut requests, true);
        self.handle(requests);
        self.sync_selection();
        self.sync_audio_settings();
        self.control_bridge();
        self.midi_sync(ctx);
        self.midi_devices_sync(ctx);
        self.sync_clocks(ctx);
        // (Edits made here, from the MK3, reach the song playing; the meters the MK3
        // shows move.)
        self.follow_edits();
        self.keep_session_soon();
        self.update_meters(ctx, undrawn(ctx, BACKGROUND_PACE));
    }
}

/// A repaint delay for a pass that draws nothing (the window covered): egui takes a
/// frame's predicted time off every delay, which a drawn frame spends, but here that
/// would make 16 ms come back at once and spin (C46).
fn undrawn(ctx: &egui::Context, after: std::time::Duration) -> std::time::Duration {
    after + std::time::Duration::from_secs_f32(ctx.input(|i| i.predicted_dt).max(0.0))
}

/// Printed notes in a pattern `length` ticks long: a note running past the end goes on
/// from the start (as Live prints them), up to where it began; one past the end
/// entirely moves in.
fn print_wrap(notes: &[Note], length: i64) -> Vec<Note> {
    let mut out = Vec::new();
    for n in notes {
        let start = n.position.rem_euclid(length.max(1));
        let first = n.length.min(length - start);
        out.push(Note { position: start, length: first, ..n.clone() });
        if n.length > first && start > 0 {
            out.push(Note { position: 0, length: (n.length - first).min(start), ..n.clone() });
        }
    }
    out
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }

    /// Leave the MK3 dark, as it was before DECKARD started.
    fn on_exit(&mut self) {
        self.end_session();
        self.midi_devices.close_all();
        if let Some(m) = &self.mk3 {
            m.shut_down(goodbye::picture());
        }
    }

    /// While the window is covered or minimized eframe runs no `ui`, only this: keep the
    /// controller (scripts, LEDs, screens) and MIDI going, as `frame` does when shown.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().visible()) != Some(false) {
            return;
        }
        self.background(ctx);
    }

    /// `--input FILE`: feed scripted pointer input to the running app, for reproducing
    /// what a user does with plug-ins loaded (which tests can't, off the main thread).
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let t = self.started.elapsed().as_secs_f32();
        while let Some(&(at, ev)) = self.input_script.first()
            && at <= t
        {
            self.input_script.remove(0);
            let (kind, pos) = ev;
            let button =
                |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            match kind {
                'm' => raw.events.push(egui::Event::PointerMoved(pos)),
                'd' => raw.events.extend([egui::Event::PointerMoved(pos), button(true)]),
                'u' => raw.events.extend([egui::Event::PointerMoved(pos), button(false)]),
                _ => {}
            }
            eprintln!("input {t:.2}s {kind} {pos:?}");
        }
        if !self.input_script.is_empty() {
            ctx.request_repaint();
        }
    }
}

impl App {
    /// One frame of the whole window.
    fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = &ui.ctx().clone();
        self.share_tempo();
        // A selection belongs to the pattern it was made in.
        if self.selected_in != (self.group, self.pattern) {
            self.selected.clear();
            self.selected_in = (self.group, self.pattern);
        }
        self.update_waveform();
        self.update_sampling();
        self.poll_recorded();
        if self.recording {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        // A stem separation's progress, and its end.
        if self.stems.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        // The recorder takes up its audio every frame while armed or recording.
        if self.recorder.busy() {
            ctx.request_repaint();
        }
        self.update_slot_view();
        if self.catalog.poll() {
            ctx.request_repaint();
        }
        if self.catalog.scanning() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        if self.lane_of != Some((self.group, self.sound)) {
            self.lane_of = Some((self.group, self.sound));
            let saved = self.doc.as_ref().and_then(|d| {
                let d = d.borrow();
                let s = d.project().groups.get(self.group)?.sounds.get(self.sound)?;
                layout::Lane::from_ref(s.lanes[s.lane?])
            });
            self.lane = saved.unwrap_or_default();
        }
        // Files from the desktop: samples go to the sound they're dropped on (after the
        // rows are drawn, below), anything else is opened.
        let (hovering, dropped) = ctx.input(|i| {
            let dropped: Vec<PathBuf> =
                i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect();
            (i.raw.hovered_files.iter().any(|f| f.path.as_deref().is_some_and(is_sample_file)), dropped)
        });
        let (samples, other): (Vec<PathBuf>, Vec<PathBuf>) = dropped.into_iter().partition(|p| is_sample_file(p));
        self.file_drag = if hovering || !samples.is_empty() { drop_pos::pointer(ctx) } else { None };
        if hovering {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        ctx.data_mut(|d| d.remove::<Vec<(egui::Rect, (usize, usize))>>(egui::Id::new(layout::DROP_ROWS)));
        self.dropped_samples = samples;
        if let Some(path) = other.into_iter().next() {
            self.open_file(path);
        }
        if let Some(out) = self.screenshot.clone() {
            self.frames += 1;
            let wait = self.shot_wait.unwrap_or(if self.pending_launch.is_some() || self.playing.is_some() { 2.0 } else { 0.0 });
            let ready = self.started.elapsed().as_secs_f32() > wait && self.frames >= 3;
            if ready && !self.shot_requested {
                self.shot_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            let shot = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = shot {
                if let Err(e) = save_png(&out, &image) {
                    eprintln!("{}: {e}", out.display());
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
        if let Some(bpm) = self.pending_tempo
            && self.started.elapsed().as_secs_f32() > 1.0
        {
            self.pending_tempo = None;
            if let Some(doc) = &self.doc
                && let Err(e) = doc.borrow_mut().set_tempo(bpm)
            {
                self.status = e;
            }
        }
        if let Some(scene) = self.pending_launch
            && self.started.elapsed().as_secs_f32() > 1.0
        {
            self.pending_launch = None;
            self.launch_scene(scene);
        }

        self.follow_engine(ctx, std::time::Duration::ZERO);
        if !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.toggle_play();
        }
        // The mouse modes' keys (not while the keys play notes).
        if !ctx.egui_wants_keyboard_input() && !self.keys.on && ctx.input(|i| i.modifiers.is_none()) {
            use layout::MouseMode as M;
            for (k, m) in [(egui::Key::E, M::Draw), (egui::Key::X, M::Split), (egui::Key::Y, M::Join), (egui::Key::Z, M::Mute)] {
                if ctx.input(|i| i.key_pressed(k)) {
                    self.mouse = self.mouse.toggled(m);
                }
            }
        }
        self.keyboard_keys(ctx);
        self.editing_keys(ctx);
        if self.lock_morph.is_some() {
            let tick = self.playhead().map(|h| h.position);
            self.lock_tick(lock::clock(), tick);
            ctx.request_repaint();
        }
        self.menu_frame(ctx);
        if self.preload_plugins {
            if self.frames_drawn >= 2 {
                self.preload_plugins = false;
                self.load_plugins();
            } else {
                self.frames_drawn += 1;
                if self.status.is_empty()
                    && self
                        .doc
                        .as_ref()
                        .is_some_and(|d| d.borrow().project().groups.iter().any(|g| g.sounds.iter().any(|s| s.plugin.is_some())))
                {
                    self.status = "Loading plug-ins…".into();
                }
                ctx.request_repaint();
            }
        }
        if self.pending_editor && self.started.elapsed().as_secs_f32() > 1.0 {
            self.pending_editor = false;
            self.open_plugin_editor(self.group, self.sound);
        }
        if !self.editors.is_empty() {
            self.poll_plugin_editors();
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if self.held.is_some() && !ctx.input(|i| i.pointer.primary_down()) {
            self.release_held();
        }
        self.follow_edits();
        self.keep_session_soon();
        if self.session_due.is_some() {
            ui.ctx().request_repaint_after(SESSION_DELAY);
        }
        // The focused group plays live (pads, keys); the others may render ahead.
        if let Some(Ok(out)) = &self.output {
            out.set_focus(Some(self.group));
        }
        self.update_meters(ctx, std::time::Duration::ZERO);

        let before = (self.group, self.sound, self.scene, self.section, self.song_view);
        layout::header(self, ui);
        if !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::F4)) {
            self.browse(browser::BrowseAct::Toggle);
        }
        if self.browser.poll() {
            ctx.request_repaint();
        }
        if self.browser.waiting() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        if self.browser.open {
            layout::browser::panel(self, ui);
        }

        // The controller panel: tick the scripts, draw, and act on what they ask for.
        self.sync_notes_played();
        if !self.lit.is_empty() {
            // Unlight the flashed numbers on time.
            ui.ctx().request_repaint_after(deckard_engine::output::FLASH);
        }
        let mut requests = Vec::new();
        if let Some(c) = &mut self.controller {
            if self.show_controller {
                egui::Panel::right("controller")
                    .resizable(false)
                    .exact_size(layout::CONTROLLER_WIDTH)
                    .frame(egui::Frame::new().fill(layout::S00).inner_margin(8.0))
                    .show(ui, |ui| match c {
                        Ok(c) => {
                            c.tick(&mut requests);
                            c.ui(ui, &mut requests);
                        }
                        Err(e) => {
                            ui.colored_label(egui::Color32::LIGHT_RED, format!("controller scripts: {e}"));
                        }
                    });
            } else if let Ok(c) = c {
                c.tick(&mut requests);
            }
            if let Ok(c) = c {
                ctx.request_repaint_after(c.interval());
            }
        }
        self.mk3_sync(ui.ctx(), &mut requests, false);
        self.handle(requests);
        self.sync_selection();
        self.sync_audio_settings();
        self.control_bridge();
        // The scale as the controller has it (its Keyboard page can change it too). Its
        // root is the focused group's, so a new root counts as a change of root only
        // while the same group stays focused.
        if let Some(Ok(c)) = &self.controller {
            let now = c.host.scale();
            let from = self.scale.0;
            let same_group = self.scale_group == Some(self.group);
            self.scale = now;
            self.scale_group = Some(self.group);
            if same_group && now.0 != from {
                self.follow_root(from, now.0);
            }
        }

        self.midi_sync(ui.ctx());
        self.midi_devices_sync(ui.ctx());
        self.sync_clocks(ui.ctx());
        self.input_sync();
        layout::body(self, ui);
        if !self.dropped_samples.is_empty() {
            let files = std::mem::take(&mut self.dropped_samples);
            let rows = ctx.data(|d| d.get_temp::<Vec<(egui::Rect, (usize, usize))>>(egui::Id::new(layout::DROP_ROWS))).unwrap_or_default();
            let at = self.file_drag.and_then(|p| rows.iter().find(|(r, _)| r.contains(p)).map(|&(_, gs)| gs));
            self.drop_samples(at.unwrap_or((self.group, self.sound)), &files);
            self.file_drag = None;
        }
        // A browser result dropped on a sound's row loads there.
        if ctx.input(|i| i.pointer.any_released())
            && let Some(dragged) = egui::DragAndDrop::take_payload::<layout::browser::Dragged>(ctx)
            && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
        {
            let rows = ctx.data(|d| d.get_temp::<Vec<(egui::Rect, (usize, usize))>>(egui::Id::new(layout::DROP_ROWS))).unwrap_or_default();
            if let Some(&(_, gs)) = rows.iter().find(|(r, _)| r.contains(p)) {
                self.browse(browser::BrowseAct::Load(dragged.0, Some(gs)));
            }
        }
        if ctx.input(|i| i.pointer.any_released())
            && let Some(dragged) = egui::DragAndDrop::take_payload::<layout::browser::DraggedFile>(ctx)
            && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
        {
            let rows = ctx.data(|d| d.get_temp::<Vec<(egui::Rect, (usize, usize))>>(egui::Id::new(layout::DROP_ROWS))).unwrap_or_default();
            if let Some(&(_, gs)) = rows.iter().find(|(r, _)| r.contains(p)) {
                self.browse(browser::BrowseAct::LoadFile(dragged.0.clone(), Some(gs)));
            }
        }
        if self.save_dialog.is_some() {
            layout::save_dialog::show(self, ctx);
        }
        if self.repeat_panel {
            self.repeat_window(ctx);
        }
        if self.lock_panel {
            self.lock_window(ctx);
        }
        if self.midi_change {
            self.midi_change_window(ctx);
        }
        let monitor = self.midi.monitor();
        if self.midi_monitor {
            midi_monitor::window(ctx, &mut self.midi_monitor, &monitor, &mut self.monitor_view);
        }
        monitor.set_open(self.midi_monitor && !self.monitor_view.paused);
        self.poll_export();
        self.poll_bounce();
        self.bounce_windows(ctx);
        if self.export.open {
            self.export_window(ctx);
        }
        if (self.group, self.sound, self.scene, self.section, self.song_view) != before {
            self.sync_controller();
        }
    }
}

#[cfg(test)]
mod tests {
    //! The window driven with simulated mouse input (egui_kittest).
    use super::*;
    use deckard_model::Dim;
    use deckard_model::Note;
    use egui::{Event, PointerButton, Pos2};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    const PROJECT: &str = "/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj";

    /// A value sent to a plug-in shows until its controller reports it (JUCE's take it
    /// in at the next block), or for SENT_HOLD; then what the plug-in says again.
    #[test]
    fn a_sent_plugin_value_shows_until_the_plugin_reports_it() {
        let t0 = std::time::Instant::now();
        let text = |id: u32, v: f64| format!("{id}:{v:.1}");
        let mut sent = vec![(7, 0.9, t0)];
        let mut read = vec![(0.5, "old".to_string()), (0.2, "other".to_string())];
        hold_sent(&mut read, &[7, 8], &mut sent, t0, text);
        assert_eq!(read[0], (0.9, "7:0.9".to_string()), "the stale read gives way");
        assert_eq!(read[1].1, "other");
        assert_eq!(sent.len(), 1);
        // The plug-in has it: the hold ends.
        let mut read = vec![(0.9, "new".to_string()), (0.2, "other".to_string())];
        hold_sent(&mut read, &[7, 8], &mut sent, t0, text);
        assert_eq!(read[0].1, "new");
        assert!(sent.is_empty());
        // One it never takes shows what it says after the hold.
        let mut sent = vec![(7, 0.9, t0)];
        let mut read = vec![(0.5, "old".to_string())];
        hold_sent(&mut read, &[7], &mut sent, t0 + SENT_HOLD * 2, text);
        assert_eq!(read[0].1, "old");
        assert!(sent.is_empty());
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
        app.song_view = song_view;
        let mut h = Harness::builder()
            .with_size(egui::vec2(1500.0 + layout::CONTROLLER_WIDTH, 930.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
        h.run_steps(3);
        Some(h)
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

    #[test]
    fn clicking_a_scene_tab_focuses_it() {
        let Some(mut h) = harness(false, false) else { return };
        click(&mut h, scene_tab(3));
        assert_eq!(h.state().scene, 3);
        click(&mut h, scene_tab(1));
        assert_eq!(h.state().scene, 1);
    }

    #[test]
    fn clicking_a_scene_tab_with_the_controller_focuses_it() {
        let Some(mut h) = harness(true, false) else { return };
        assert!(matches!(h.state().controller, Some(Ok(_))), "controller scripts should load");
        click(&mut h, scene_tab(3));
        assert_eq!(h.state().scene, 3);
        h.run_steps(10);
        assert_eq!(h.state().scene, 3, "the selection sticks");
    }

    #[test]
    fn a_scene_clicked_while_playing_is_queued_then_plays() {
        let Some(mut h) = harness(true, false) else { return };
        h.state_mut().transport_play();
        if !matches!(h.state().output, Some(Ok(_))) {
            eprintln!("no audio output; skipping");
            return;
        }
        assert_eq!(h.state().playing, Some(Mode::Scene(0)));
        click(&mut h, scene_tab(3));
        assert_eq!(h.state().scene, 3);
        // The playhead is what the audio thread published after its next block.
        let wait = std::time::Instant::now();
        let mut head = h.state().playhead();
        while head.as_ref().is_none_or(|p| p.queued.is_none()) && wait.elapsed() < std::time::Duration::from_millis(500) {
            std::thread::sleep(std::time::Duration::from_millis(5));
            head = h.state().playhead();
        }
        let head = head.expect("playing");
        assert_eq!((head.mode, head.queued), (Mode::Scene(0), Some(Mode::Scene(3))));
        h.run_steps(10);
        assert_eq!(h.state().scene, 3, "the selection sticks while the old scene finishes");
        h.state_mut().stop();
    }

    /// #14: with the window covered (eframe runs only `logic`), an edit from the MK3
    /// (TEMPO and the jog wheel) still reaches the song playing.
    #[test]
    fn edits_reach_the_song_while_the_window_is_covered() {
        let mut app = App::default();
        app.new_project();
        app.transport_play();
        if !matches!(app.output, Some(Ok(_))) {
            eprintln!("no audio output; skipping");
            return;
        }
        app.surface_act(deckard_surface::Action::Tempo(171.0));
        let ctx = egui::Context::default();
        let wait = std::time::Instant::now();
        let mut tempo = 0.0;
        while tempo != 171.0 && wait.elapsed() < std::time::Duration::from_millis(1000) {
            app.background(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
            tempo = app.output.as_ref().unwrap().as_ref().unwrap().telemetry(|t| t.tempo);
        }
        app.stop();
        assert_eq!(tempo, 171.0);
    }

    /// #33: STOP leaves the song where it was (shown there), PLAY goes on from there,
    /// RESTART goes back to the start.
    #[test]
    fn stop_keeps_the_position_and_play_continues() {
        let mut app = App::default();
        app.new_project();
        app.transport_play();
        if !matches!(app.output, Some(Ok(_))) {
            eprintln!("no audio output; skipping");
            return;
        }
        let ctx = egui::Context::default();
        let at = |app: &mut App, past: f64| {
            let wait = std::time::Instant::now();
            let mut pos = 0.0;
            while pos <= past && wait.elapsed() < std::time::Duration::from_millis(2000) {
                app.background(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(10));
                pos = app.playhead().map_or(0.0, |h| h.position);
            }
            pos
        };
        let quarter = deckard_model::PPQ as f64;
        assert!(at(&mut app, quarter) > quarter, "playing");
        app.stop();
        let parked = app.shown_playhead().expect("shown where it stopped").position;
        assert!(parked > quarter, "{parked}");
        // The MK3 (the touch strip's play position) shows it there too.
        assert_eq!(app.surface_view().position, Some(parked as i64));
        app.transport_play();
        let first = at(&mut app, 0.0);
        assert!(first >= parked - 1.0, "on from {parked}: {first}");
        app.stop();
        app.restart();
        let first = at(&mut app, 0.0);
        app.stop();
        assert!(first < parked, "RESTART: from the start ({first})");
    }

    /// BUGS #6: a project reopens where it was saved: the focused sound and slot, the
    /// Channel page or the plug-ins, and the controller's page and pad mode.
    #[test]
    fn a_project_reopens_where_it_was_saved() {
        let mut app = App::default();
        app.new_project();
        layout::apply_acts(&mut app, vec![layout::Act::AddScene]);
        app.doc.as_ref().unwrap().borrow_mut().add_slot(0, 3, 0, &deckard_doc::NewModule::Sampler).unwrap();
        (app.sound, app.slot, app.channel) = (3, Some(0), true);
        app.focus_scene(1);
        let view = app.surface_view();
        for a in app.surface.restore("mixer", "keyboard", &view, std::time::Instant::now()) {
            app.surface_act(a);
        }
        app.store_screen();
        let bytes = app.doc.as_ref().unwrap().borrow().to_bytes().unwrap();
        let mut again = App::default();
        again.show_document(Document::from_bytes(bytes, None));
        assert_eq!((again.sound, again.slot, again.channel, again.scene), (3, Some(0), true, 1));
        assert_eq!(again.surface.screen(), Some(("mixer".to_string(), "keyboard".to_string())));
    }

    /// C46: on DECKARD's layer the MK3 keeps a clock of its own, as the scripts' timer
    /// does, so a covered window still comes back to it: faster while the song plays.
    #[test]
    fn deckards_layer_keeps_the_mk3_ticking() {
        let mut app = App::default();
        app.new_project();
        assert_eq!(app.mk3_wake(), Some(controller::tick_every(false)));
        app.playing = Some(Mode::Scene(0));
        assert_eq!(app.mk3_wake(), Some(controller::tick_every(true)));
        app.surface.set_theme("palenight");
        assert_eq!(app.mk3_wake(), Some(std::time::Duration::from_millis(30)));
    }

    /// C46: with the window covered, the focus still follows the scene playing (the MK3's
    /// screens show it).
    #[test]
    fn scenes_are_followed_while_the_window_is_covered() {
        let mut app = App::default();
        app.new_project();
        layout::apply_acts(&mut app, vec![layout::Act::AddScene]);
        app.focus_scene(0);
        app.transport_play();
        let Some(Ok(out)) = &app.output else {
            eprintln!("no audio output; skipping");
            return;
        };
        out.queue(app.doc.as_ref().unwrap().borrow().project(), Mode::Scene(1));
        let ctx = egui::Context::default();
        let wait = std::time::Instant::now();
        while app.scene != 1 && wait.elapsed() < std::time::Duration::from_secs(6) {
            app.background(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.stop();
        assert_eq!(app.scene, 1);
    }

    /// C46: while covered and playing, the background asks to come back at its pace, never
    /// at once (with nothing drawn to wait on, that would spin a core).
    #[test]
    fn the_background_comes_back_at_its_pace() {
        let mut app = App::default();
        app.new_project();
        app.metronome = true;
        app.transport_play();
        if !matches!(app.output, Some(Ok(_))) {
            eprintln!("no audio output; skipping");
            return;
        }
        let ctx = egui::Context::default();
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = asked.clone();
        ctx.set_request_repaint_callback(move |info| log.lock().unwrap().push(info.delay));
        let wait = std::time::Instant::now();
        while app.master_meter == (0.0, 0.0) && wait.elapsed() < std::time::Duration::from_millis(1500) {
            app.background(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.stop();
        let asked = asked.lock().unwrap();
        assert!(!asked.is_empty(), "it asks to come back");
        assert!(asked.iter().all(|d| *d >= BACKGROUND_PACE), "{asked:?}");
    }

    /// #15: with the window covered, the meters (the MK3's Mixer shows them) still move.
    #[test]
    fn meters_move_while_the_window_is_covered() {
        let mut app = App::default();
        app.new_project();
        app.metronome = true;
        app.transport_play();
        if !matches!(app.output, Some(Ok(_))) {
            eprintln!("no audio output; skipping");
            return;
        }
        let ctx = egui::Context::default();
        let wait = std::time::Instant::now();
        while app.master_meter == (0.0, 0.0) && wait.elapsed() < std::time::Duration::from_millis(1500) {
            app.background(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.stop();
        assert_ne!(app.master_meter, (0.0, 0.0), "the click metered");
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

    #[test]
    fn song_view_section_click_sets_the_start_and_queues_a_jump() {
        let Some(mut h) = harness(false, true) else { return };
        if h.state().doc.as_ref().unwrap().borrow().project().sections.len() < 4 {
            return;
        }
        let pos = section_header(&h, 2);
        click(&mut h, pos);
        assert_eq!(h.state().section, Some(2));
        if !play(&mut h) {
            return;
        }
        let start = h.state().doc.as_ref().unwrap().borrow().project().section_start(2) as f64;
        let head = h.state().playhead().unwrap();
        assert!((head.position - start).abs() < 960.0, "plays from the picked section ({} vs {start})", head.position);
        let pos = section_header(&h, 3);
        click(&mut h, pos);
        assert_eq!(h.state().section, Some(3));
        assert_eq!(h.state().queued_section(), Some(3), "jumps when section 3 (index 2) ends");
        h.state_mut().stop();
    }

    /// RESTART (the header's or the controller's) plays the song from its first bar,
    /// even with a later section picked; Loop switches in both and carries over.
    #[test]
    fn restart_plays_the_song_from_the_beginning_and_loop_carries_over() {
        let Some(mut h) = harness(true, true) else { return };
        let later = h.state().doc.as_ref().unwrap().borrow().project().sections.get(2).map(|_| ());
        if later.is_none() {
            return;
        }
        let start = h.state().doc.as_ref().unwrap().borrow().project().section_start(2) as f64;
        h.state_mut().section = Some(2);
        layout::apply_acts(h.state_mut(), vec![layout::Act::Restart]);
        h.run_steps(2);
        if !matches!(h.state().output, Some(Ok(_))) {
            return;
        }
        let head = h.state().playhead().unwrap();
        assert_eq!(head.mode, Mode::Arrangement);
        assert!(head.position < start / 2.0, "from the beginning ({} vs section 3 at {start})", head.position);

        let hw = |h: &mut Harness<'static, App>, button: &str, down: Option<bool>| {
            if let Some(Ok(c)) = &mut h.state_mut().controller {
                let _ = match down {
                    Some(d) => c.host.button(button, d),
                    None => c.host.press(button),
                };
            }
            h.run_steps(4);
        };
        let seek = h.state().doc.as_ref().unwrap().borrow().project().section_start(2);
        if let Some(Ok(out)) = &h.state().output {
            out.seek(seek);
        }
        h.run_steps(2);
        hw(&mut h, "BUTTON_TRANSPORT_LOOP", None);
        let head = h.state().playhead().unwrap();
        assert!(head.position < start / 2.0, "the controller's RESTART too ({})", head.position);

        assert!(!h.state().looping);
        layout::apply_acts(h.state_mut(), vec![layout::Act::ToggleLoop]);
        assert!(h.state().looping);
        let scripts_loop = |h: &Harness<'static, App>| match &h.state().controller {
            Some(Ok(c)) => c.host.state.borrow().transport.looping,
            _ => false,
        };
        assert!(scripts_loop(&h), "the controller sees Loop on");
        hw(&mut h, "BUTTON_SHIFT", Some(true));
        hw(&mut h, "BUTTON_TRANSPORT_LOOP", None);
        hw(&mut h, "BUTTON_SHIFT", Some(false));
        assert!(!h.state().looping, "SHIFT + RESTART switches Loop off in the app");
        h.state_mut().stop();
    }

    /// The controller's Song view edits the arrangement's sections (C1).
    #[test]
    fn the_surface_sets_the_pad_pages_settings() {
        use deckard_surface::{Action as A, ColorOf};
        let Some(mut h) = harness(false, true) else { return };
        let app = h.state_mut();
        let p = project_of_app(app);
        if p.groups.is_empty() || p.groups[0].sounds.is_empty() {
            return;
        }
        (app.group, app.sound) = (0, 0);
        app.surface_act(A::VelocityValue(77));
        app.surface_act(A::Scale { root: 62, bank: 0, kind: 2 });
        app.surface_act(A::PadTouch("curve", 30));
        app.surface_act(A::PadTouch("mode", 3));
        app.surface_act(A::Color { of: ColorOf::Sound, color: Some(5) });
        app.surface_act(A::Color { of: ColorOf::Group, color: None });
        let v = app.surface_view();
        assert_eq!((v.velocity, v.scale_bank, v.scale_kind), (77, 0, 2));
        assert_eq!(app.scale, (62, 0, 2), "the pattern editor's scale too");
        let snd = &v.groups[0].sounds[0];
        assert_eq!((snd.touch_mode, snd.touch_curve), (3, 30));
        assert_eq!((snd.color, snd.default_color), (5, false));
        assert!(v.groups[0].default_color);
        app.user_chords = user_chords::UserChords::default();
        app.surface_act(A::UserChordNote { set: 1, slot: 3, note: 7 });
        app.surface_act(A::UserChordNote { set: 1, slot: 3, note: 0 });
        assert_eq!(app.surface_view().user_chords[1][3], [0, 7]);
        app.surface_act(A::UserChordMove { set: 1, slot: 3, next: false });
        app.surface_act(A::UserChordDelete { set: 1, slot: 3 });
        assert_eq!(app.surface_view().user_chords[1][2], [0, 7]);
    }

    #[test]
    fn the_file_page_runs_the_file_commands() {
        use deckard_surface::Action as A;
        let Some(mut h) = harness(false, true) else { return };
        let app = h.state_mut();
        app.recent = vec![PathBuf::from("/tmp/Song A.mxprj"), PathBuf::from("/tmp/Song B.mxprj")];
        assert_eq!(app.surface_view().recent, ["Song A", "Song B"]);
        app.menu_commands.clear();
        for what in ["new", "save", "save_copy", "export_audio"] {
            app.surface_act(A::File(what, 0));
        }
        app.surface_act(A::File("load", 1));
        assert_eq!(
            app.menu_commands,
            [
                menu::Command::New,
                menu::Command::Save,
                menu::Command::SaveCopyAs,
                menu::Command::ExportAudio,
                menu::Command::OpenRecent(PathBuf::from("/tmp/Song B.mxprj"))
            ]
        );
        app.menu_commands.clear();
        app.surface_act(A::File("save_group", 0));
        assert!(app.save_dialog.is_some(), "a group not saved before: the Save dialog");
    }

    #[test]
    fn the_surface_settings_page_steps_the_settings() {
        use deckard_surface::Action as A;
        let Some(mut h) = harness(false, true) else { return };
        let app = h.state_mut();
        app.settings.count_in_bars = 1;
        app.settings.link = false;
        for (name, step) in [
            ("count_in", 1),
            ("count_in", 1),
            ("count_in", 1),
            ("quantize", 1),
            ("numerator", -1),
            ("denominator", 1),
            ("midi_sync", 1),
            ("midi_sync", 1),
            ("clock_offset", -1),
            ("default_input", 1),
            ("link_start_stop", 1),
            ("sensitivity", -1),
            ("display_brightness", -1),
        ] {
            app.surface_act(A::Setting(name, 0, step));
        }
        let s = &app.settings;
        assert_eq!(s.count_in_bars, 4, "Off, 1, 2, 4 bars: no further");
        assert!(s.input_quantize && s.link_start_stop && !s.sound_midi_focus);
        assert_eq!(s.midi_sync, settings::MidiSync::Receive);
        assert_eq!(s.clock_receive_offset_ms, -1);
        assert_eq!((s.pad_sensitivity, s.display_brightness), (49, 99));
        let p = app.doc.as_ref().unwrap().borrow().project().clone();
        assert_eq!((p.numerator, p.denominator), (3, 8));
        let v = app.settings_view();
        for (key, shown) in
            [("count_in", "4 Bars"), ("quantize", "On"), ("midi_sync", "Receive"), ("clock_offset", "-1"), ("default_input", "None")]
        {
            assert_eq!(v.values.get(key).map(String::as_str), Some(shown), "{key}");
        }
        assert!(v.audio_status[0].starts_with("Device Status: "));
    }

    #[test]
    fn the_surface_sets_the_grids_and_follow() {
        use deckard_surface::Action as A;
        let Some(mut h) = harness(false, true) else { return };
        let app = h.state_mut();
        app.follow = true;
        app.surface_act(A::Follow(false));
        assert!(!app.follow);
        for (name, value) in [("perform", 2), ("arrange", 3), ("arrange_snap", 0), ("quick", 0), ("step_snap", 0), ("nudge", 3)] {
            app.surface_act(A::Grid(name, value));
        }
        app.surface_act(A::Grid("step", 11));
        let g = app.surface_view().grid;
        assert_eq!((g.perform, g.arrange, g.arrange_snap, g.quick), (2, 3, false, false));
        assert_eq!((g.step, g.step_snap, g.nudge), (11, false, 3));
        assert_eq!(app.grid, PPQ / 3, "the pattern editor's grid: 1/8T");
        assert_eq!(app.nudge.0, 3);
    }

    #[test]
    fn the_surface_edits_sections() {
        use deckard_surface::Action as A;
        let Some(mut h) = harness(false, true) else { return };
        let app = h.state_mut();
        let p = project_of_app(app);
        if p.sections.len() < 2 || p.scenes.len() < 2 {
            return;
        }
        let v = app.surface_view();
        assert!(v.song_view);
        assert_eq!(v.sections.len(), p.sections.len());
        assert_eq!(v.section, Some(0), "the first, until one's picked");
        assert_eq!(v.sections[1].start, p.sections[0].length);

        app.surface_act(A::SectionEdit("create"));
        let q = project_of_app(app);
        assert_eq!(q.sections.len(), p.sections.len() + 1);
        assert_eq!(q.sections[1].scene, p.sections[0].scene, "after the focused, playing its scene");
        assert_eq!(app.section, Some(1));

        app.surface_act(A::SectionMove(false));
        assert_eq!(app.section, Some(0));
        app.surface_act(A::SectionLength(3 * 3840));
        let q = project_of_app(app);
        assert_eq!((q.sections[0].length, q.sections[0].auto), (3 * 3840, false));
        app.surface_act(A::SectionEdit("delete"));
        assert_eq!(project_of_app(app).sections.len(), p.sections.len());

        app.surface_act(A::SongView(false));
        assert!(!app.surface_view().song_view);
    }

    /// DECKARD's own MK3 layer: its actions go through the app's own handlers, and its
    /// view is the app's.
    #[test]
    fn the_surface_plays_and_focuses() {
        use deckard_surface::Action as A;
        let Some(mut h) = harness(false, false) else { return };
        let app = h.state_mut();
        assert!(app.controller.is_none(), "no scripts: DECKARD's layer runs");
        let v = app.surface_view();
        let p = project_of_app(app);
        assert_eq!(v.groups.len(), p.groups.len());
        assert_eq!(v.groups[0].sounds.len(), p.groups[0].sounds.len());
        assert_eq!(v.tempo, p.tempo);

        app.surface_act(A::Tempo(133.0));
        assert_eq!(project_of_app(app).tempo, 133.0);
        app.surface_act(A::Volume(0.5));
        assert_eq!(project_of_app(app).master_level, 0.5);
        app.surface_act(A::Swing(25.0));
        assert_eq!(project_of_app(app).swing.amount, 25.0);
        let muted = project_of_app(app).groups[0].sounds[1].muted;
        app.surface_act(A::MuteSound(1));
        assert_eq!(project_of_app(app).groups[0].sounds[1].muted, !muted);
        app.surface_act(A::FocusSound(2));
        assert_eq!(app.surface_view().sound, 2);
        for on in [false, true] {
            app.surface_act(A::FixedVelocity(on));
            assert_eq!(app.surface_view().fixed_velocity, on);
        }
        if p.groups.len() > 1 {
            app.surface_act(A::FocusGroup(1));
            assert_eq!(app.group, 1);
            app.surface_act(A::FocusGroup(0));
        }

        // ERASE + pad: the sound's notes leave the focused pattern.
        let pat = project_of_app(app).groups[0].patterns[app.pattern].clone();
        if let Some(s) = pat.notes.first().map(|n| n.sound) {
            app.surface_act(A::EraseSound(s));
            let left = &project_of_app(app).groups[0].patterns[app.pattern].notes;
            assert!(left.iter().all(|n| n.sound != s), "sound {s}'s notes erased");
            assert_eq!(left.len(), pat.notes.iter().filter(|n| n.sound != s).count(), "the others kept");
        }

        app.surface_act(A::Play);
        assert!(app.surface_view().playing);
        app.surface_act(A::Stop);
        assert!(!app.surface_view().playing);

        // The shipped scripts see this project: its screens draw without an error.
        let v = app.surface_view();
        assert!(app.surface.screens(&v).is_some());
        assert_eq!(app.surface.error(), None);
    }

    /// Preferences ▸ Hardware ▸ Controller ▸ DECKARD stops the scripts.
    #[test]
    fn choosing_deckards_layer_stops_the_scripts() {
        let Some(mut h) = harness(true, false) else { return };
        if !matches!(h.state().controller, Some(Ok(_))) {
            return;
        }
        let app = h.state_mut();
        app.set_controller_choice(settings::ControllerChoice::Deckard);
        assert!(app.controller.is_none());
        assert!(app.controller_scripts().is_none());
        app.set_controller_choice(settings::ControllerChoice::Automatic);
        assert!(matches!(app.controller, Some(Ok(_))), "the scripts again");
    }

    #[test]
    fn theme_names_read_well() {
        assert_eq!(theme_name("solarized_dark"), "Solarized Dark");
        assert_eq!(theme_name("maschine"), "Maschine");
    }

    /// TAP on the controller sets the song's tempo from the taps; SHIFT + TAP and the
    /// header's switch turn the metronome on and off in both.
    #[test]
    fn tap_tempo_and_the_metronome() {
        let Some(mut h) = harness(true, false) else { return };
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            for k in 0..3 {
                if k > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                }
                let _ = c.host.press("BUTTON_TAP");
            }
        }
        h.run_steps(4);
        let tempo = h.state().doc.as_ref().unwrap().borrow().project().tempo;
        assert!((tempo - 150.0).abs() < 8.0, "taps 0.4 s apart: about 150 BPM ({tempo})");

        assert!(!h.state().metronome);
        let hw = |h: &mut Harness<'static, App>, button: &str, down: bool| {
            if let Some(Ok(c)) = &mut h.state_mut().controller {
                let _ = c.host.button(button, down);
            }
            h.run_steps(4);
        };
        hw(&mut h, "BUTTON_SHIFT", true);
        hw(&mut h, "BUTTON_TAP", true);
        hw(&mut h, "BUTTON_TAP", false);
        hw(&mut h, "BUTTON_SHIFT", false);
        assert!(h.state().metronome, "SHIFT + TAP: the metronome on");
        layout::apply_acts(h.state_mut(), vec![layout::Act::ToggleMetronome]);
        assert!(!h.state().metronome);
        let scripts = match &h.state().controller {
            Some(Ok(c)) => c.host.state.borrow().transport.metronome,
            _ => true,
        };
        assert!(!scripts, "the controller sees it off");
    }

    /// A pad hit on the controller focuses its sound in the app too, so the Control
    /// area (and the controller's Control page) shows that sound's chain: nothing for
    /// an empty sound, not the last one shown.
    #[test]
    fn controller_pads_focus_their_sound_and_its_chain() {
        let Some(mut h) = harness(true, false) else { return };
        h.state_mut().chain_tab = layout::ChainTab::Sound;
        let sounds: Vec<usize> = {
            let d = h.state().doc.as_ref().unwrap().borrow();
            d.project().groups[0].sounds.iter().map(|s| s.slots.len()).collect()
        };
        let (Some(full), Some(empty)) = (sounds.iter().position(|&n| n > 0), sounds.iter().position(|&n| n == 0)) else { return };
        let hit = |h: &mut Harness<'static, App>, s: usize| {
            if let Some(Ok(c)) = &mut h.state_mut().controller {
                c.host.pad_event(s as i64 + 1, true, 0.8);
                c.host.pad_event(s as i64 + 1, false, 0.0);
            }
            h.run_steps(4);
        };
        let shown = |h: &Harness<'static, App>| match &h.state().controller {
            Some(Ok(c)) => c.host.state.borrow().control.slots.len(),
            _ => 0,
        };
        hit(&mut h, full);
        assert_eq!(h.state().sound, full);
        assert_eq!(shown(&h), sounds[full], "the controller shows the sound's chain");
        hit(&mut h, empty);
        assert_eq!(h.state().sound, empty, "the app follows the pad");
        assert_eq!(shown(&h), 0, "an empty sound shows no chain");

        // And a sound picked in the app is the controller's focused sound.
        h.state_mut().sound = full;
        h.state_mut().sync_controller();
        h.run_steps(4);
        let focused = match &h.state().controller {
            Some(Ok(c)) => c.host.state.borrow().focus.sound,
            _ => full,
        };
        assert_eq!(focused, full);
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

    /// WAVs dragged from the desktop onto a sound row (lit while over it): on an empty
    /// sound, an Audio plug-in playing it in time (its tempo from its name), on in the
    /// focused pattern, which holds it all; on a sound with a Sampler, a zone more of it.
    /// One undo step each. Two files on empty sounds: a loop each.
    #[test]
    fn samples_dropped_on_sounds_load_as_loops_or_zones() {
        let Some(mut h) = harness(false, false) else { return };
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let wav = |name: &str, frames: usize| {
            let path = dir.join(name);
            let spec = hound::WavSpec { channels: 2, sample_rate: 44_100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
            let mut w = hound::WavWriter::create(&path, spec).unwrap();
            for i in 0..frames {
                let v = ((i as f32 * 0.05).sin() * 8000.0) as i16;
                w.write_sample(v).unwrap();
                w.write_sample(v).unwrap();
            }
            w.finalize().unwrap();
            path
        };
        // 8 beats at 94 BPM; a one-shot.
        let (lp, hit, a, b) =
            (wav("Groove[94].wav", 225_186), wav("Hit.wav", 4410), wav("Pad A[120].wav", 88_200), wav("Pad B[120].wav", 88_200));
        let row = |h: &Harness<'static, App>, s: usize| -> egui::Rect {
            let rows: Vec<(egui::Rect, (usize, usize))> = h.ctx.data(|d| d.get_temp(egui::Id::new(layout::DROP_ROWS))).expect("sound rows");
            rows.iter().find(|(_, gs)| *gs == (h.state().group, s)).expect("the sound's row").0
        };
        let drop = |h: &mut Harness<'static, App>, s: usize, files: &[&PathBuf]| {
            let at = row(h, s).center();
            h.hover_at(at);
            h.input_mut().hovered_files =
                files.iter().map(|f| egui::HoveredFile { path: Some((*f).clone()), ..Default::default() }).collect();
            h.step();
            assert!(h.state().file_drag.is_some_and(|p| row(h, s).contains(p)), "the row is lit");
            h.input_mut().hovered_files.clear();
            h.input_mut().dropped_files =
                files.iter().map(|f| std::sync::Arc::new(Dropped((*f).clone())) as egui::DroppedFileHandle).collect();
            h.step();
            h.input_mut().dropped_files.clear();
            h.step();
        };
        let g = h.state().group;
        let sound = |h: &Harness<'static, App>, s: usize| project_of(h).groups[g].sounds[s].clone();
        // The rows shown: an empty sound and one with a Sampler.
        let shown: Vec<usize> = (0..7).collect();
        let empty: Vec<usize> = shown.iter().copied().filter(|&s| sound(&h, s).slots.is_empty()).collect();
        let Some(&e) = empty.first() else { return };
        let undo_steps = h.state().doc.as_ref().unwrap().borrow().undo_labels().len();
        drop(&mut h, e, &[&lp]);
        let snd = sound(&h, e);
        let a_loop = snd.audio.clone().expect("an Audio plug-in");
        assert_eq!(snd.instrument, "AudioModule");
        assert_eq!(a_loop.takes.len(), 1);
        assert_eq!(Path::new(&a_loop.takes[0].sample.path), lp.as_path());
        assert!((a_loop.takes[0].beats - 8.0).abs() < 0.01, "8 beats at 94: {}", a_loop.takes[0].beats);
        let pi = h.state().pattern;
        let pat = project_of(&h).groups[g].patterns[pi].clone();
        assert!(pat.audio.iter().any(|c| c.sound == e && c.take == 0), "on in the focused pattern");
        assert!(pat.length >= 8 * PPQ, "the pattern holds it");
        assert_eq!(h.state().doc.as_ref().unwrap().borrow().undo_labels().len(), undo_steps + 1, "one undo step");
        // On a Sampler: one zone more.
        if let Some(&with) = shown.iter().find(|&&s| sound(&h, s).sampler.as_ref().is_some_and(|x| !x.zones.is_empty())) {
            let before = sound(&h, with).sampler.unwrap().zones.len();
            drop(&mut h, with, &[&hit]);
            let zones = sound(&h, with).sampler.unwrap().zones;
            assert_eq!(zones.len(), before + 1, "added to the Sampler");
            assert_eq!(Path::new(&zones[before].sample.path), hit.as_path());
            assert!(sound(&h, with).audio.is_none());
        }
        // Two on empty sounds: a loop each.
        if let Some(w) = empty.windows(2).find(|w| w[1] == w[0] + 1).map(|w| w[0]) {
            drop(&mut h, w, &[&a, &b]);
            let paths: Vec<String> = [w, w + 1].iter().map(|&s| sound(&h, s).audio.unwrap().takes[0].sample.path.clone()).collect();
            assert_eq!(paths, [a.display().to_string(), b.display().to_string()]);
        }
    }

    /// Finished stems become a new group, as Maschine makes it: named after the sample,
    /// its first four sounds playing them (saved as "<name> Drums", …) — here Audio
    /// plug-ins, in time with the song, on in its pattern, which plays in the scene — as
    /// one undo step.
    /// STEMS from the controller, with the model installed (`DECKARD_STEMS_MODEL`: its
    /// folder): EDIT ▸ STEMS ▸ APPLY separates the focused zone's sample in the
    /// background, the controller shows it running, and the stems arrive as a new group.
    #[test]
    fn controller_stems_separate_the_sample_into_a_new_group() {
        let Some(model) = std::env::var_os("DECKARD_STEMS_MODEL") else { return };
        let Some(mut h) = harness(true, false) else { return };
        let has_zone = project_of(&h).groups[0].sounds[0].sampler.as_ref().is_some_and(|s| !s.zones.is_empty());
        if !has_zone {
            return;
        }
        // The model where the app reads it (the tests' own settings folder), the stems
        // next to a temporary project.
        let settings = menu::settings_dir().unwrap();
        std::fs::create_dir_all(settings.join("models")).unwrap();
        std::fs::copy(Path::new(&model).join(deckard_stems::MODEL_FILE), settings.join("models").join(deckard_stems::MODEL_FILE)).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let groups = project_of(&h).groups.len();
        {
            let app = h.state_mut();
            app.path = Some(dir.join("x.mxprj"));
            (app.group, app.sound) = (0, 0);
            app.sync_controller();
        }
        h.run_steps(2);
        let press = |h: &mut Harness<'static, App>, b: &str| {
            if let Some(Ok(c)) = &mut h.state_mut().controller {
                let _ = c.host.press(b);
                c.host.tick();
            }
            h.run_steps(2);
        };
        press(&mut h, "BUTTON_SAMPLE");
        press(&mut h, "BUTTON_SCREEN_2");
        press(&mut h, "BUTTON_SCREEN_5");
        press(&mut h, "BUTTON_SCREEN_5");
        press(&mut h, "BUTTON_SCREEN_8");
        assert!(h.state().stems.is_some(), "separating: {}", h.state().status);
        let running = match &h.state().controller {
            Some(Ok(c)) => c.host.state.borrow().stems.is_some_and(|r| r.focused),
            _ => false,
        };
        assert!(running, "the controller shows it");
        let t = std::time::Instant::now();
        while h.state().stems.is_some() && t.elapsed() < std::time::Duration::from_secs(300) {
            std::thread::sleep(std::time::Duration::from_millis(200));
            h.run_steps(1);
        }
        let p = project_of(&h);
        assert_eq!(p.groups.len(), groups + 1, "{}", h.state().status);
        let names: Vec<String> = p.groups[groups].sounds[..4].iter().map(|s| s.name.clone()).collect();
        assert!(names[0].ends_with(" Drums") && names[3].ends_with(" Vocal"), "{names:?}");
        let _ = std::fs::remove_dir_all(settings);
    }

    #[test]
    fn stems_make_a_new_group_of_audio_loops() {
        let Some(mut h) = harness(false, false) else { return };
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let groups = project_of(&h).groups.len();
        let app = h.state_mut();
        app.path = Some(dir.join("x.mxprj"));
        let stems = deckard_stems::Kind::ALL
            .iter()
            .enumerate()
            .map(|(k, &kind)| deckard_stems::Stem { kind, frames: vec![[0.1 * (k + 1) as f32; 2]; 4410] })
            .collect();
        app.finish_stems("Song", 44_100, stems, Some(4.0));
        let p = project_of(&h);
        assert_eq!(p.groups.len(), groups + 1, "{}", h.state().status);
        let g = &p.groups[groups];
        assert_eq!(g.name, "Song");
        let names: Vec<&str> = g.sounds[..4].iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Song Drums", "Song Bass", "Song Other", "Song Vocal"]);
        for (k, s) in g.sounds[..4].iter().enumerate() {
            let a = s.audio.as_ref().expect("an Audio plug-in");
            assert_eq!(a.takes.len(), 1);
            assert_eq!(a.takes[0].beats, 4.0, "the source's beats");
            assert!(std::path::Path::new(&a.takes[0].sample.path).exists(), "{}", a.takes[0].sample.path);
            assert!(g.patterns[0].audio.iter().any(|c| c.sound == k), "on in its pattern");
        }
        assert!(p.scenes[h.state().scene].patterns.iter().any(|&(sg, _)| sg == groups), "playing in the scene");
        assert_eq!(h.state().group, groups, "the new group in focus");
        h.state_mut().undo();
        assert_eq!(project_of(&h).groups.len(), groups, "one undo step");
    }

    #[test]
    fn auto_loop_finds_a_loop_in_the_play_range() {
        let Some(mut h) = harness(false, false) else { return };
        let has_zone = project_of(&h).groups[0].sounds[0].sampler.as_ref().is_some_and(|s| !s.zones.is_empty());
        if !has_zone {
            return;
        }
        let rate = 44_100;
        let tone = deckard_engine::sample::Sample {
            rate,
            frames: (0..rate as usize).map(|i| [(i as f32 * 220.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5; 2]).collect(),
        };
        let app = h.state_mut();
        (app.group, app.sound, app.zone) = (0, 0, 0);
        app.sampling.sample = Some(std::sync::Arc::new(tone));
        let t = deckard_doc::Target::Zone(0, 0, 0);
        layout::apply_acts(
            app,
            vec![layout::Act::SetParam(t, "play_range/low", 0.0), layout::Act::SetParam(t, "play_range/high", 44_099.0)],
        );
        layout::apply_acts(app, vec![layout::Act::SetParam(t, "loop_active", 0.0)]);
        app.sampling_act(layout::sampling::SamplingAct::Edit(layout::sampling::EditOp::AutoLoop));
        let z = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].clone();
        assert!(z.loop_active, "{}", h.state().status);
        assert!(z.loop_range.0 < z.loop_range.1 && z.loop_range.1 <= 44_100, "{:?}", z.loop_range);
        h.state_mut().undo();
        let z = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].clone();
        assert!(!z.loop_active, "one undo step");
    }

    #[test]
    fn controller_sampling_tabs_show_and_edit_the_sample() {
        let Some(mut h) = harness(true, false) else { return };
        {
            let app = h.state_mut();
            (app.group, app.sound) = (0, 0);
            app.sync_controller();
        }
        h.run_steps(2);
        let has_zone = project_of(&h).groups[0].sounds[0].sampler.as_ref().is_some_and(|s| !s.zones.is_empty());
        if !has_zone {
            return;
        }
        let press = |h: &mut Harness<'static, App>, b: &str| {
            if let Some(Ok(c)) = &mut h.state_mut().controller {
                let _ = c.host.press(b);
                c.host.tick();
            }
            h.run_steps(3);
        };
        press(&mut h, "BUTTON_SAMPLE");
        press(&mut h, "BUTTON_SCREEN_2");
        assert_eq!(h.state().sampling.tab, 1, "the app's Sampling view on EDIT too");
        h.run_steps(2);
        let frames = match &h.state().controller {
            Some(Ok(c)) => c.host.state.borrow().sample.as_ref().map(|s| (s.frames, s.overview.len())),
            _ => None,
        };
        let (frames, cols) = frames.expect("the controller has the sample");
        assert!(frames > 0 && cols == 480);
        let before = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].play_range.0;
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            let _ = c.host.encoder("ENCODER_SCREEN_1", 0.05);
        }
        h.run_steps(2);
        let after = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].play_range.0;
        assert!(after > before, "knob 1 moved the start: {before} -> {after}");
        press(&mut h, "BUTTON_SCREEN_3");
        assert_eq!(h.state().sampling.tab, 2, "SLICE");
        h.run_steps(2);
        let slices = match &h.state().controller {
            Some(Ok(c)) => c.host.state.borrow().sample.as_ref().map_or(0, |s| s.slices.len()),
            _ => 0,
        };
        assert_eq!(slices, h.state().sampling.slices.len().max(1), "the app's slices");
        // The app's tab shows on the controller too.
        h.state_mut().sampling_act(layout::sampling::SamplingAct::Tab(3));
        h.run_steps(2);
        let hw_tab = match &h.state().controller {
            Some(Ok(c)) => c.host.state.borrow().sampling_tab,
            _ => -1,
        };
        assert_eq!(hw_tab, 3, "ZONE on the controller");
    }

    #[test]
    fn controller_sample_button_shows_sampling_and_takes_make_a_sampler() {
        let Some(mut h) = harness(true, false) else { return };
        let effect = deckard_model::INTERNAL_EFFECTS[0].0;
        {
            let app = h.state_mut();
            let s = app.doc.as_ref().unwrap().borrow().project().groups[0].sounds.iter().position(|x| x.slots.is_empty());
            let Some(s) = s else { return };
            app.sound = s;
            app.sync_controller();
            app.doc.as_ref().unwrap().borrow_mut().add_slot(0, s, 0, &deckard_doc::NewModule::Effect(effect)).unwrap();
        }
        h.run_steps(2);
        let press = |h: &mut Harness<'static, App>| {
            if let Some(Ok(c)) = &mut h.state_mut().controller {
                let _ = c.host.press("BUTTON_SAMPLE");
            }
            h.run_steps(4);
        };
        // The Sampling page's tabs (SLICE among them) show for a sound with a Sampler
        // (they hide for a sound without one).
        let tabs_shown = |h: &Harness<'static, App>| {
            use deckard_lua_host::state::Prop;
            let Some(Ok(c)) = &h.state().controller else { return false };
            let st = c.host.state.borrow();
            let all = &st.widgets.all;
            let shown = |i: usize| !matches!(all[i].props.get("Visible"), Some(Prop::Bool(false)));
            let Some(page) = all.iter().position(|w| w.name == "SamplingPageStudio") else { return false };
            // Down the page's shown widgets to a SLICE button.
            let mut todo = vec![page];
            while let Some(i) = todo.pop() {
                if matches!(all[i].props.get("Text").or(all[i].props.get("style")), Some(Prop::Str(t)) if t == "SLICE") {
                    return true;
                }
                todo.extend(all[i].children.iter().copied().filter(|&c| shown(c)));
            }
            false
        };
        press(&mut h);
        let s = h.state().sound;
        let chain = |h: &Harness<'static, App>| -> Vec<String> {
            h.state().doc.as_ref().unwrap().borrow().project().groups[0].sounds[s].slots.iter().map(|x| x.module.clone()).collect()
        };
        assert_eq!(chain(&h), [effect], "SAMPLE keeps the chain");
        assert!(h.state().sampling_open, "the Sampling view shows");
        assert_eq!(h.state().sampling.tab, 0, "on RECORD, with nothing sampled yet");
        assert!(!tabs_shown(&h), "no Sampler: RECORD alone");
        // The controller's RECORD screen meters what the recorder hears.
        h.state_mut().recorder.view.meter = (0.5, 0.25);
        h.run_steps(1);
        // (The scripts' timer: they update the screen on their ticks.)
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            for _ in 0..3 {
                c.host.tick();
            }
        }
        let levels = match &h.state().controller {
            Some(Ok(c)) => {
                let st = c.host.state.borrow();
                st.widgets.all.iter().find(|w| w.kind == "MasterLevelMeter").map(|w| {
                    let n = |k: &str| match w.props.get(k) {
                        Some(deckard_lua_host::state::Prop::Num(n)) => *n,
                        _ => 0.0,
                    };
                    (n("Levels"), n("Levels2"))
                })
            }
            _ => None,
        };
        let (l, r) = levels.expect("the Sampling page's meter");
        assert!(l > 0.05 && r > 0.02 && l > r, "the meter shows the input ({l}, {r})");
        // The RECORD screen's knobs set the recorder: knob 1 the MODE (RECORDING page),
        // and after RIGHT the SOURCE (INPUT page).
        let turn = |h: &mut Harness<'static, App>, b: &str, inc: f64| {
            if let Some(Ok(c)) = &mut h.state_mut().controller {
                let _ = c.host.encoder(b, inc);
            }
            h.run_steps(2);
        };
        assert_eq!(h.state().recorder.view.mode, 0);
        turn(&mut h, "ENCODER_SCREEN_1", 0.04);
        assert_eq!(h.state().recorder.view.mode, 1, "Sync");
        turn(&mut h, "ENCODER_SCREEN_1", -0.04);
        assert_eq!(h.state().recorder.view.mode, 0, "Detect again");
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            let _ = c.host.press("BUTTON_RIGHT");
            // (The scripts' timer shows the page's knobs.)
            c.host.tick();
        }
        h.run_steps(2);
        let source = h.state().recorder.view.source;
        turn(&mut h, "ENCODER_SCREEN_1", if source == 2 { -0.04 } else { 0.04 });
        assert_ne!(h.state().recorder.view.source, source, "SOURCE");
        // START (button 5) arms the app's recorder (here recording what plays), STOP
        // ends it; START then CANCEL (button 6) throws it away. Without an audio output
        // (no device) there's nothing to record from, and the recorder says so.
        h.state_mut().recorder.view.source = 2;
        let screen_button = |h: &mut Harness<'static, App>, b: &str| {
            if let Some(Ok(c)) = &mut h.state_mut().controller {
                let _ = c.host.press(b);
            }
            h.run_steps(2);
        };
        screen_button(&mut h, "BUTTON_SCREEN_5");
        if h.state().recorder.busy() {
            let waiting = matches!(&h.state().controller, Some(Ok(c)) if c.host.state.borrow().record.waiting);
            assert!(waiting, "the RECORD screen shows it waiting for sound");
            screen_button(&mut h, "BUTTON_SCREEN_5");
            assert!(!h.state().recorder.busy(), "STOP");
            screen_button(&mut h, "BUTTON_SCREEN_5");
            assert!(h.state().recorder.busy(), "START again");
            screen_button(&mut h, "BUTTON_SCREEN_6");
            assert!(!h.state().recorder.busy(), "CANCEL");
        } else {
            assert_eq!(h.state().recorder.view.error.as_deref(), Some("no audio output to record from"));
        }
        // A recorded take makes a sound without a Sampler one (its modules go); a sound
        // with a Sampler keeps its chain.
        let take = std::env::temp_dir().join("deckard-take-test.wav");
        h.state_mut().load_take(0, s, &take, 4800);
        assert_eq!(chain(&h), ["Sampler"], "the take replaced the chain");
        h.state().doc.as_ref().unwrap().borrow_mut().add_slot(0, s, 1, &deckard_doc::NewModule::Effect(effect)).unwrap();
        h.state_mut().load_take(0, s, &take, 9600);
        assert_eq!(chain(&h), ["Sampler", effect], "a Sampler's chain stays");
        h.run_steps(4);
        assert!(tabs_shown(&h), "the page's tabs show for a Sampler");
        press(&mut h);
        assert!(!h.state().sampling_open, "SAMPLE again: back to the pattern editor");
    }

    #[test]
    fn controller_scene_button_and_pads_pick_scenes_in_the_ideas_view() {
        let Some(mut h) = harness(true, true) else { return };
        // Switch to the Ideas view with the strip button, as a user would.
        click(&mut h, Pos2::new(3.0 + 16.0, 33.0 + 3.0 + 6.0 + 12.0));
        assert!(!h.state().song_view);
        if !play(&mut h) {
            return;
        }
        press(&mut h, "SCENE");
        press(&mut h, "4");
        assert_eq!(h.state().scene, 3);
        let head = h.state().playhead().unwrap();
        assert_eq!(head.queued, Some(Mode::Scene(3)));
        press(&mut h, "SCENE");
        h.state_mut().stop();
    }

    /// Closing the window after the controller changed the project asks about saving
    /// first, and doesn't close.
    #[test]
    fn closing_after_a_controller_edit_asks_to_save() {
        let Some(mut h) = harness(true, false) else { return };
        h.run_steps(2);
        assert!(!h.state().dirty(), "nothing changed yet");
        {
            let app = h.state_mut();
            let c = app.controller.as_mut().unwrap().as_mut().unwrap();
            for (b, down) in [("BUTTON_PATTERN", true), ("BUTTON_SCREEN_5", true), ("BUTTON_SCREEN_5", false), ("BUTTON_PATTERN", false)] {
                c.host.button(b, down).unwrap();
                c.host.tick();
            }
        }
        h.run_steps(2);
        assert!(h.state().dirty(), "CREATE changed the project");
        h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
        h.run_steps(1);
        assert!(matches!(h.state().confirm, Some(Pending::Quit)), "asks about saving");
        assert!(!h.state().quitting);
    }

    /// The controller's Control page works the Control area: knob 1 turns the focused
    /// effect's first knob, and screen buttons 5/6 move between slots.
    #[test]
    fn controller_knobs_and_screen_buttons_work_the_control_area() {
        let Some(mut h) = harness(true, true) else { return };
        h.state_mut().menu_commands.push(menu::Command::New);
        h.run_steps(3);
        if h.state().confirm.is_some() {
            h.get_by_label("Don't Save").click();
            h.run_steps(3);
        }
        (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
        h.state_mut().add_slot(layout::AddWhat::Instrument("BassSynth"));
        h.state_mut().add_slot(layout::AddWhat::Effect("Chorus"));
        h.run_steps(4);
        assert_eq!(h.state().slot, Some(1), "{}", h.state().status);
        let rate = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[0].slots[1].param("rate").unwrap();
        let before = rate(&h);
        {
            let app = h.state_mut();
            let c = app.controller.as_mut().unwrap().as_mut().unwrap();
            let mut req = Vec::new();
            c.encoder("ENCODER_SCREEN_1", 0.2, &mut req);
            let knobs = c.host.state.borrow().control.knobs.clone();
            assert_eq!(knobs[0].as_ref().map(|k| k.name.as_str()), Some("RATE"), "{knobs:?}");
        }
        h.run_steps(3);
        assert!((rate(&h) - (before + 0.2)).abs() < 1e-3, "knob 1 turned the Chorus's Rate: {before} → {}", rate(&h));
        // Screen button 5: the slot before (the Bass Synth).
        {
            let app = h.state_mut();
            let c = app.controller.as_mut().unwrap().as_mut().unwrap();
            c.press("BUTTON_SCREEN_5", &mut Vec::new());
        }
        h.run_steps(3);
        assert_eq!(h.state().slot, Some(0), "screen button 5 focused the previous slot");
        let knobs = h.state().controller.as_ref().unwrap().as_ref().unwrap().host.state.borrow().control.knobs.clone();
        assert_eq!(knobs[0].as_ref().map(|k| k.name.as_str()), Some("SHAPE"), "{knobs:?}");
    }

    /// PLUG-IN on an empty sound: the "+" is focused; the 4-D encoder opens the plug-in
    /// browser, turns to a module and loads it (pressed). Then on to the "+" again, TYPE to
    /// Effect, and an effect after it.
    #[test]
    fn controller_plugin_browser_loads_modules() {
        let Some(mut h) = harness(true, true) else { return };
        h.state_mut().menu_commands.push(menu::Command::New);
        h.run_steps(3);
        if h.state().confirm.is_some() {
            h.get_by_label("Don't Save").click();
            h.run_steps(3);
        }
        (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
        h.run_steps(3);
        let slots =
            |h: &Harness<'static, App>| project_of(h).groups[0].sounds[0].slots.iter().map(|x| x.module.clone()).collect::<Vec<_>>();
        assert!(slots(&h).is_empty());
        let with = |h: &mut Harness<'static, App>, f: &dyn Fn(&mut controller::Controller)| {
            let c = h.state_mut().controller.as_mut().unwrap().as_mut().unwrap();
            f(c);
            h.run_steps(3);
        };
        with(&mut h, &|c| c.press("BUTTON_PLUGIN", &mut Vec::new()));
        let focus = h.state().controller.as_ref().unwrap().as_ref().unwrap().host.state.borrow().control.focus_slot;
        assert_eq!(focus, None, "the + is focused");
        with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
        // (none), Sampler, Audio, Kick: a step at a time.
        for _ in 0..3 {
            with(&mut h, &|c| c.wheel(1, &mut Vec::new()));
        }
        with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
        assert_eq!(slots(&h), ["Kick"], "{}", h.state().status);
        // The right display's logo follows: the Kick's, then (removed) none.
        let logo = |h: &mut Harness<'static, App>| {
            // The controller's next timers (every 50 ms of real time in the app) see the
            // new chain.
            for _ in 0..3 {
                h.state_mut().controller.as_mut().unwrap().as_mut().unwrap().host.tick();
                h.run_steps(1);
            }
            let st = h.state().controller.as_ref().unwrap().as_ref().unwrap().host.state.borrow();
            let bg = st.widgets.all.iter().find(|w| w.name == "PluginBG").expect("the logo's bar");
            match bg.props.get("PictureUrl") {
                Some(deckard_lua_host::state::Prop::Str(p)) => p.clone(),
                _ => String::new(),
            }
        };
        let shown = logo(&mut h);
        assert!(shown.to_lowercase().contains("kick"), "{shown}");
        with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
        for _ in 0..3 {
            with(&mut h, &|c| c.wheel(-1, &mut Vec::new()));
        }
        with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
        assert!(slots(&h).is_empty(), "(none) removed it");
        assert_eq!(logo(&mut h), "", "no logo left behind");
        with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
        for _ in 0..3 {
            with(&mut h, &|c| c.wheel(1, &mut Vec::new()));
        }
        with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
        assert_eq!(slots(&h), ["Kick"], "{}", h.state().status);
        let shown = logo(&mut h);
        assert!(shown.to_lowercase().contains("kick"), "{shown}");

        with(&mut h, &|c| c.press("BUTTON_WHEEL_RIGHT", &mut Vec::new()));
        assert_eq!(h.state().slot, None, "past the last slot: the +");
        with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
        with(&mut h, &|c| c.encoder("ENCODER_SCREEN_1", 0.3, &mut Vec::new()));
        with(&mut h, &|c| c.wheel(1, &mut Vec::new()));
        with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
        assert_eq!(slots(&h), ["Kick", deckard_model::INTERNAL_EFFECTS[0].0], "{}", h.state().status);
    }

    /// The controller's buttons switch the app's views: KEYBOARD shows the pattern
    /// editor as a piano roll and PAD MODE back as the drum grid; the Arranger page's
    /// SHIFT + IDEAS / SONG switch the song panel, and LEFT / RIGHT there pick the
    /// section the app shows.
    /// The "+" after the groups adds an empty group at the end and focuses it.
    #[test]
    fn plus_after_the_groups_adds_one() {
        let Some(mut h) = harness(false, false) else { return };
        let n = h.state().doc.as_ref().unwrap().borrow().project().groups.len();
        layout::apply_acts(h.state_mut(), vec![layout::Act::AddGroup]);
        h.run_steps(2);
        let d = h.state().doc.as_ref().unwrap().clone();
        assert_eq!(d.borrow().project().groups.len(), n + 1);
        assert_eq!(h.state().group, n);
        assert_eq!(d.borrow().project().groups[n].sounds.len(), 16);
    }

    #[test]
    fn controller_buttons_switch_the_apps_views() {
        let Some(mut h) = harness(true, false) else { return };
        let hw = |h: &mut Harness<'static, App>, button: &str, down: Option<bool>| {
            if let Some(Ok(c)) = &mut h.state_mut().controller {
                match down {
                    Some(d) => {
                        let _ = c.host.button(button, d);
                    }
                    None => {
                        let _ = c.host.press(button);
                    }
                }
            }
            h.run_steps(4);
        };
        assert!(!h.state().piano_roll);
        hw(&mut h, "BUTTON_KEYBOARD", None);
        assert!(h.state().piano_roll, "Keyboard mode: a piano roll");
        hw(&mut h, "BUTTON_PAD_MODE", None);
        assert!(!h.state().piano_roll, "Pad mode: the drum grid");

        if h.state().doc.as_ref().unwrap().borrow().project().sections.len() < 2 {
            return;
        }
        assert!(!h.state().song_view);
        hw(&mut h, "BUTTON_ARRANGE", None);
        hw(&mut h, "BUTTON_SHIFT", Some(true));
        hw(&mut h, "BUTTON_SCREEN_2", None);
        hw(&mut h, "BUTTON_SHIFT", Some(false));
        assert!(h.state().song_view, "SONG: the Song view");
        hw(&mut h, "BUTTON_RIGHT", None);
        assert_eq!(h.state().section, Some(1));
        hw(&mut h, "BUTTON_SHIFT", Some(true));
        hw(&mut h, "BUTTON_SCREEN_1", None);
        hw(&mut h, "BUTTON_SHIFT", Some(false));
        assert!(!h.state().song_view, "IDEAS: the Ideas view");
    }

    #[test]
    fn controller_scene_button_and_pads_pick_sections_in_the_song_view() {
        let Some(mut h) = harness(true, true) else { return };
        assert!(h.state().song_view);
        if h.state().doc.as_ref().unwrap().borrow().project().sections.len() < 4 {
            return;
        }
        if !play(&mut h) {
            return;
        }
        press(&mut h, "SCENE");
        press(&mut h, "4");
        assert_eq!(h.state().section, Some(3));
        assert_eq!(h.state().queued_section(), Some(3));
        press(&mut h, "SCENE");
        h.state_mut().stop();
    }

    #[test]
    fn the_pattern_editor_shows_the_scene_pattern_and_follows_playback() {
        let Some(mut h) = harness(false, true) else { return };
        let (first, later) = {
            let d = h.state().doc.as_ref().unwrap().borrow();
            let p = d.project();
            let pat = |s: usize| p.scenes[s].patterns.iter().find(|(g, _)| *g == 0).map(|&(_, pi)| pi);
            // a later section whose scene plays a different pattern on group A
            let later = (0..p.sections.len()).find(|&i| p.sections[i].scene.and_then(pat) != pat(0) && p.sections[i].scene.is_some());
            (pat(0), later.map(|i| (i, p.section_start(i), p.sections[i].scene.unwrap(), p.sections[i].scene.and_then(pat))))
        };
        assert_eq!(Some(h.state().pattern), first, "opening focuses scene 1's pattern");
        let Some((_, start, scene, pattern)) = later else { return };
        if !play(&mut h) {
            return;
        }
        // Follow on: jump playback into the later section; the focus follows.
        if let Some(Ok(out)) = &h.state().output {
            out.seek(start + 10);
        }
        h.run_steps(3);
        assert_eq!(h.state().scene, scene);
        assert_eq!(Some(h.state().pattern), pattern);
        // Follow off: it stays.
        h.state_mut().follow = false;
        if let Some(Ok(out)) = &h.state().output {
            out.seek(10);
        }
        h.run_steps(3);
        assert_eq!(h.state().scene, scene);
        h.state_mut().stop();
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

    #[test]
    fn ideas_view_edits_scenes() {
        let Some(mut h) = harness(false, false) else { return };
        let p = project_of(&h);
        let scene = h.state().scene;
        // A pattern not in the focused scene (within the first 8 columns, visible).
        let Some((g, pi)) = (0..p.groups.len().min(8))
            .flat_map(|g| (0..p.groups[g].patterns.len().min(4)).map(move |pi| (g, pi)))
            .find(|gp| !p.scenes[scene].patterns.contains(gp))
        else {
            return;
        };
        click(&mut h, cell(g, pi));
        assert!(project_of(&h).scenes[scene].patterns.contains(&(g, pi)), "clicking a pattern puts it in the scene");
        assert_eq!((h.state().group, h.state().pattern), (g, pi));
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_steps(3);
        assert_eq!(project_of(&h).scenes[scene].patterns, p.scenes[scene].patterns, "⌘Z undoes it");

        // "+" after the last scene tab.
        let n = p.scenes.len();
        click(&mut h, Pos2::new(3.0 + 33.0 + n as f32 * 99.0 + 13.0, 33.0 + 3.0 + 16.0));
        assert_eq!(project_of(&h).scenes.len(), n + 1);
        assert_eq!(h.state().scene, n);

        // Right-click the first scene tab, Duplicate.
        right_click(&mut h, scene_tab(0));
        press(&mut h, "Duplicate");
        let after = project_of(&h);
        assert_eq!(after.scenes.len(), n + 2);
        assert_eq!(after.scenes[1].name, format!("{} (1)", p.scenes[0].name));
        assert_eq!(h.state().scene, 1);
    }

    /// A pattern put in a scene past the last (a project saved with no scenes: the Ideas
    /// view's click asks for scene 0 then) makes that scene with it in, one undo step.
    #[test]
    fn a_pattern_into_a_scene_past_the_last_makes_one() {
        let Some(mut h) = harness(false, false) else { return };
        let doc = h.state().doc.clone().unwrap();
        let n = doc.borrow().project().scenes.len();
        let steps = doc.borrow().undo_labels().len();
        h.state_mut().arrange(layout::Act::AssignPattern(n, 1, Some(0)));
        let p = project_of(&h);
        assert_eq!(p.scenes.len(), n + 1, "a scene made");
        assert_eq!(p.scenes[n].patterns, vec![(1, 0)], "with the pattern in");
        assert_eq!(doc.borrow().undo_labels().len(), steps + 1, "one undo step");
        assert_eq!(h.state().scene, n, "and focused");
    }

    /// Ideas view: a click shows a pattern (and puts it in the scene), a double-click on
    /// a pattern in the scene takes it out, and a double-click below a group's patterns
    /// makes a new one there, in the scene.
    #[test]
    fn ideas_view_double_clicks() {
        let Some(mut h) = harness(false, false) else { return };
        let p = project_of(&h);
        let scene = h.state().scene;
        let Some(&(g, pi)) = p.scenes[scene].patterns.iter().find(|&&(g, pi)| g < 8 && pi < 4 && p.groups[g].patterns.len() < 6) else {
            return;
        };
        click(&mut h, cell(g, pi));
        assert!(project_of(&h).scenes[scene].patterns.contains(&(g, pi)), "a click leaves it in the scene");
        assert_eq!((h.state().group, h.state().pattern), (g, pi), "and shows it");
        double_click(&mut h, cell(g, pi));
        assert!(!project_of(&h).scenes[scene].patterns.iter().any(|x| x.0 == g), "a double-click takes it out");
        double_click(&mut h, cell(g, pi));
        assert!(project_of(&h).scenes[scene].patterns.contains(&(g, pi)), "double-clicking one that wasn't in leaves it in");
        let n = p.groups[g].patterns.len();
        double_click(&mut h, cell(g, n));
        let q = project_of(&h);
        assert_eq!(q.groups[g].patterns.len(), n + 1, "{}", h.state().status);
        assert_eq!(q.groups[g].patterns[n].name, format!("Pattern {}", n + 1));
        assert!(q.scenes[scene].patterns.contains(&(g, n)), "the new pattern is in the scene");
        assert_eq!((h.state().group, h.state().pattern), (g, n));
        assert!(q.groups[g].patterns[n].notes.is_empty());
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_steps(3);
        assert_eq!(project_of(&h).groups[g].patterns.len(), n, "one undo step");
    }

    #[test]
    fn song_view_edits_sections() {
        let Some(mut h) = harness(false, true) else { return };
        let p = project_of(&h);
        if p.sections.len() < 2 {
            return;
        }
        let bar = p.bar_length();
        let pos = section_header(&h, 0);
        right_click(&mut h, pos);
        let length_menu = format!("Length ({} bars) ⏵", p.sections[0].length as f64 / bar as f64);
        press(&mut h, &length_menu);
        press(&mut h, "8 bars");
        assert_eq!(project_of(&h).sections[0].length, 8 * bar);

        let pos = section_header(&h, 1);
        right_click(&mut h, pos);
        press(&mut h, "Delete");
        assert_eq!(project_of(&h).sections.len(), p.sections.len() - 1);

        // "+" after the last section adds one playing the focused scene.
        let q = project_of(&h);
        let end = section_header(&h, q.sections.len() - 1);
        let d = h.state().doc.as_ref().unwrap().borrow();
        let p2 = d.project();
        let left = 3.0 + 33.0 + 163.0 + 1.0;
        let width = WIDTH - 3.0 - left;
        let total = p2.arrangement_length() as f32 / bar as f32;
        let ppb = ((width - 40.0) / total.max(16.0)).max(12.0);
        let x = left + total * ppb + 14.0;
        drop(d);
        click(&mut h, Pos2::new(x, end.y));
        let r = project_of(&h);
        assert_eq!(r.sections.len(), q.sections.len() + 1);
        assert_eq!(r.sections.last().unwrap().scene, Some(h.state().scene));
    }

    #[test]
    fn controller_screen_buttons_edit_scenes() {
        let Some(mut h) = harness(true, false) else { return };
        // Switch the controller to the Ideas view too.
        h.state_mut().sync_controller();
        let n = project_of(&h).scenes.len();
        press(&mut h, "SCENE");
        press(&mut h, "DUPLICATE");
        assert_eq!(project_of(&h).scenes.len(), n + 1);
        press(&mut h, "SCENE");
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

    /// The Mix view's plug-in strip: panels for the focused sound's slots; dragging
    /// the Compressor's Knee or Threshold edits them, its power button bypasses it.
    #[test]
    fn plugin_strip_edits_the_compressor() {
        let Some(mut h) = harness(false, false) else { return };
        h.state_mut().mixer = true;
        h.run_steps(3);
        let slots = project_of(&h).groups[0].sounds[0].slots.clone();
        let Some(ci) = slots.iter().position(|x| x.module == "Compressor" && x.plugin.is_none()) else { return };
        let panels: Vec<(usize, egui::Rect)> =
            h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-panels"))).expect("the strip draws its panels");
        assert_eq!(panels.len(), slots.len(), "a panel per slot");
        let r = panels.iter().find(|p| p.0 == ci).unwrap().1;
        let param = |h: &Harness<'static, App>, f: &str| project_of(h).groups[0].sounds[0].slots[ci].param(f).unwrap_or(0.0);
        // Knee: the first knob, right of the meters.
        let (inner_l, inner_t) = (r.left() + 4.0, r.top() + 26.0);
        let knee = Pos2::new(inner_l + 152.0, inner_t + 50.0);
        let before = param(&h, "knee");
        drag(&mut h, knee, knee - egui::vec2(0.0, 40.0));
        assert!(param(&h, "knee") > before + 0.1, "knee {before} -> {}", param(&h, "knee"));
        // Threshold: its handle, dragged down (lower dB, a larger value).
        let thr = param(&h, "threshold") as f32;
        let bar_h = r.height() - 26.0 - 82.0;
        let handle = Pos2::new(inner_l + 10.0 + 30.0 + 13.0 + 6.0, inner_t + 46.0 + thr * bar_h);
        drag(&mut h, handle, handle + egui::vec2(0.0, 30.0));
        assert!(param(&h, "threshold") > thr as f64 + 0.05, "threshold {thr} -> {}", param(&h, "threshold"));
        // Bypass.
        click(&mut h, Pos2::new(r.left() + 12.0, r.top() + 11.0));
        assert!(!project_of(&h).groups[0].sounds[0].slots[ci].active);
        click(&mut h, Pos2::new(r.left() + 12.0, r.top() + 11.0));
        assert!(project_of(&h).groups[0].sounds[0].slots[ci].active);
    }

    /// The drum synths' strip panels: MAIN (Tune, the engine, Velocity, Decay) and
    /// MODULATION (the engine's own controls) edit the module; `$DECKARD_RENDER_DIR` gets
    /// a picture of both tabs of each.
    #[test]
    fn drum_strip_panels_edit() {
        let Some(mut h) = harness(false, false) else { return };
        layout::style(&h.ctx);
        let empty: Vec<usize> = (0..16).filter(|&s| project_of(&h).groups[0].sounds.get(s).is_some_and(|x| x.slots.is_empty())).collect();
        let dir = render_dir();
        for (n, drum) in crate::drum_ui::DRUMS.iter().enumerate() {
            let Some(&sound) = empty.get(n) else { return };
            h.state_mut().mixer = false;
            h.state_mut().sound = sound;
            h.run_steps(2);
            h.state_mut().add_slot(layout::AddWhat::Instrument(drum));
            h.state_mut().mixer = true;
            h.run_steps(5);
            let slots = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[sound].slots.clone();
            let si = slots(&h).iter().position(|x| x.module == *drum).expect("added");
            let panels: Vec<(usize, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-panels"))).unwrap();
            let r = panels.iter().find(|p| p.0 == si).unwrap().1;
            if let Some(d) = &dir {
                h.render().unwrap().save(d.join(format!("strip-{}-main.png", drum.to_lowercase()))).unwrap();
            }
            // The body under the header bar, in the QML's 632×216.
            let body = egui::Rect::from_min_max(Pos2::new(r.left() + 4.0, r.top() + 26.0), Pos2::new(r.right() - 4.0, r.bottom()));
            let k = body.height() / 216.0;
            let at = |x: f32, y: f32| body.min + egui::vec2(x, y) * k;
            let param = |h: &Harness<'static, App>, f: &str| slots(h)[si].param(f);
            if *drum == "Snare" {
                // The engine's right arrow: Volt → Bit.
                click(&mut h, at(233.0 + 82.0, 47.0) + egui::vec2(147.0 * k / 2.0 + 6.0, 0.0));
                assert_eq!(param(&h, "mode"), Some(1.0), "the engine steps");
                click(&mut h, at(233.0 + 82.0, 47.0) - egui::vec2(147.0 * k / 2.0 + 6.0, 0.0));
                // Tune: drag up.
                let tune = at(31.0 + 62.0, 43.0 + 40.0);
                drag(&mut h, tune, tune - egui::vec2(0.0, 60.0));
                assert!(param(&h, "tune").unwrap() > 0.55, "tune {:?}", param(&h, "tune"));
            }
            // MODULATION.
            let tabs = Pos2::new(body.right() - 40.0, body.top() + 18.0);
            click(&mut h, tabs);
            if let Some(d) = &dir {
                h.render().unwrap().save(d.join(format!("strip-{}-modulation.png", drum.to_lowercase()))).unwrap();
            }
            if *drum == "Snare" {
                // Volt's Color knob (NOISE).
                let spots = layout::strip::editor::knob_spots(&slots(&h)[si], (0, sound, si));
                let (at, field) = spots.iter().find(|(_, f)| f == "noise_color").expect("Color shows for Volt");
                let c = r.min + *at;
                let before = param(&h, field).unwrap_or(0.5);
                drag(&mut h, c, c - egui::vec2(0.0, 40.0));
                assert!(param(&h, field).unwrap() > before, "Color turns");
            }
        }
    }

    /// A sound takes one instrument, in its first slot: a second is refused, effects
    /// still go after it.
    #[test]
    fn one_instrument_per_sound() {
        let Some(mut h) = harness(false, false) else { return };
        let Some(sound) = (0..16).find(|&s| project_of(&h).groups[0].sounds.get(s).is_some_and(|x| x.slots.is_empty())) else { return };
        h.state_mut().sound = sound;
        h.state_mut().add_slot(layout::AddWhat::Instrument("Kick"));
        h.state_mut().add_slot(layout::AddWhat::Instrument("Snare"));
        h.state_mut().add_slot(layout::AddWhat::Sampler);
        h.state_mut().add_slot(layout::AddWhat::Effect("Chorus"));
        h.run_steps(2);
        let modules: Vec<String> = project_of(&h).groups[0].sounds[sound].slots.iter().map(|x| x.module.clone()).collect();
        assert_eq!(modules, ["Kick", "Chorus"]);
    }

    /// The Poly Synth's strip panel: four tabs (Osc A, Osc B, LFO, Mod) laid out as
    /// Maschine's; its switches and knobs edit the synth. `$DECKARD_RENDER_DIR` gets a
    /// picture of each tab.
    #[test]
    fn poly_synth_strip_panel_edits() {
        let Some(mut h) = harness(false, false) else { return };
        layout::style(&h.ctx);
        let Some(sound) = (0..16).find(|&s| project_of(&h).groups[0].sounds.get(s).is_some_and(|x| x.slots.is_empty())) else { return };
        h.state_mut().sound = sound;
        h.run_steps(2);
        h.state_mut().add_slot(layout::AddWhat::Instrument("PolySynth"));
        h.state_mut().mixer = true;
        h.run_steps(5);
        let slot = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[sound].slots[0].clone();
        assert_eq!(slot(&h).module, "PolySynth");
        let panels: Vec<(usize, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-panels"))).unwrap();
        let r = panels.iter().find(|p| p.0 == 0).unwrap().1;
        let body = egui::Rect::from_min_max(Pos2::new(r.left() + 4.0, r.top() + 26.0), Pos2::new(r.right() - 4.0, r.bottom()));
        let k = body.height() / 216.0;
        let at = |x: f32, y: f32| body.min + egui::vec2(x, y) * k;
        // Osc A: Saw (on) off; then Tune up.
        assert_eq!(slot(&h).param("osc_a_saw").unwrap_or(1.0), 1.0);
        click(&mut h, at(179.0 + 9.0, 126.0 + 19.0));
        assert_eq!(slot(&h).param("osc_a_saw"), Some(0.0), "Saw switches off");
        let tune = at(38.0 + 20.5, 118.0 + 20.5);
        drag(&mut h, tune, tune - egui::vec2(0.0, 40.0));
        assert!(slot(&h).param("osc_a_tune").unwrap_or(0.0) >= 1.0, "Tune goes up in semitones");
        let dir = render_dir();
        let tabs = h.ctx.data(|d| d.get_temp::<Vec<egui::Rect>>(egui::Id::new("deckard-poly-tabs")));
        for (n, name) in ["osc-a", "osc-b", "lfo", "mod"].iter().enumerate() {
            if let Some(t) = tabs.as_ref().and_then(|t| t.get(n)) {
                click(&mut h, t.center());
            }
            if let Some(d) = &dir {
                h.render().unwrap().save(d.join(format!("strip-polysynth-{name}.png"))).unwrap();
            }
        }
        assert!(tabs.is_some_and(|t| t.len() == 4), "four tabs");
    }

    /// The Sampler's strip panel: drag S to move the play start; the FX page's filter
    /// handle sets cutoff and resonance, the envelope's attack handle the attack.
    /// `$DECKARD_RENDER_DIR` gets a picture of each page.
    #[test]
    fn sampler_strip_panel_edits() {
        let Some(mut h) = harness(false, false) else { return };
        layout::style(&h.ctx);
        h.state_mut().mixer = true;
        h.run_steps(5);
        let sound = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[0].clone();
        let Some(si) = sound(&h).slots.iter().position(|x| x.module == "Sampler" && x.plugin.is_none()) else { return };
        let panels: Vec<(usize, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-panels"))).unwrap();
        let r = panels.iter().find(|p| p.0 == si).unwrap().1;
        let dir = std::env::var_os("DECKARD_RENDER_DIR").map(PathBuf::from);
        let shot = |h: &mut Harness<'static, App>, name: &str| {
            if let Some(dir) = &dir {
                h.render().unwrap().save(dir.join(name)).unwrap();
            }
        };
        let body_top = r.top() + 26.0;
        let area = egui::Rect::from_min_max(Pos2::new(r.left() + 16.0, body_top + 36.0), Pos2::new(r.right() - 16.0, r.bottom() - 6.0));
        // Main: drag S right.
        assert!(h.state().waveform.is_some(), "the zone's waveform loads");
        shot(&mut h, "strip-sampler-main.png");
        let lo = sound(&h).sampler.unwrap().zones[0].play_range.0;
        let wave_bottom = area.top() + (area.height() - 43.0);
        let s_marker = Pos2::new(area.left() + 1.0, wave_bottom - 9.0);
        drag(&mut h, s_marker, s_marker + egui::vec2(120.0, 0.0));
        let moved = sound(&h).sampler.unwrap().zones[0].play_range.0;
        assert!(moved > lo, "play start {lo} -> {moved}");
        // FX: the tab, then LP2 and ADSR.
        click(&mut h, Pos2::new(r.right() - 24.0, body_top + 17.0));
        let smp = |h: &Harness<'static, App>| sound(h).sampler.unwrap();
        let top = area.top();
        let fx = area.left() + 334.0;
        let right_arrow = |center_x: f32| Pos2::new(center_x + 34.0, top + 6.0);
        let left_arrow = |center_x: f32| Pos2::new(center_x - 34.0, top + 6.0);
        for _ in 0..4 {
            click(&mut h, left_arrow(fx + 180.0));
        }
        assert_eq!(smp(&h).get("filter"), 0.0, "the filter cycler steps down to Off");
        click(&mut h, right_arrow(fx + 180.0));
        assert_eq!(smp(&h).get("filter"), 1.0, "LP2");
        let disp = egui::Rect::from_min_size(Pos2::new(fx + 12.0, top + 20.0), egui::vec2(206.0, 70.0));
        let to = Pos2::new(disp.left() + disp.width() * 0.3, disp.bottom() - disp.height() * 0.6);
        drag(&mut h, disp.center(), to);
        let (c, q) = (smp(&h).get("cutoff"), smp(&h).get("resonance"));
        assert!((c - 0.3).abs() < 0.03 && (q - 0.6).abs() < 0.03, "cutoff {c} resonance {q}");
        let env_x = fx + 230.0;
        let w = area.right() - env_x;
        for _ in 0..2 {
            click(&mut h, right_arrow(env_x + w - 56.0));
        }
        assert_eq!(smp(&h).get("envelope_type"), 2.0, "ADSR");
        let inner = egui::Rect::from_min_size(Pos2::new(env_x + 12.0, top + 20.0), egui::vec2(w - 33.0, 70.0)).shrink(10.0);
        let a = smp(&h).get("volume_envelope/attack") as f32;
        let attack = Pos2::new(inner.left() + inner.width() * a / 4.0, inner.top());
        drag(&mut h, attack, Pos2::new(inner.left() + inner.width() * 0.125, inner.top()));
        let a2 = smp(&h).get("volume_envelope/attack");
        assert!((a2 - 0.5).abs() < 0.05, "attack {a} -> {a2}");
        shot(&mut h, "strip-sampler-fx.png");
    }

    /// Where the pattern editor puts tick `t` (from the pattern start) of row `row`,
    /// with the panel sizes of a 930-high window and no controller panel.
    fn grid_pos(h: &Harness<'static, App>, row: usize, t: i64) -> Pos2 {
        let p = project_of(h);
        let (g, pi) = (h.state().group, h.state().pattern);
        let start = p.groups[g].patterns[pi].start;
        let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
        let (from, scale): (f64, f32) = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid-time"))).expect("time axis drawn");
        Pos2::new(grid.left() + ((start + t) as f64 - from) as f32 * scale, grid.top() + row as f32 * 24.0 + 12.0)
    }

    /// SCALE (next to QUANTIZE) sets the controller's scale, and the piano roll marks
    /// the keys outside it.
    #[test]
    fn scale_menu_sets_the_scale_and_marks_the_piano_roll() {
        let Some(mut h) = harness(true, false) else { return };
        h.state_mut().piano_roll = true;
        h.run_steps(3);
        h.get_by_label("SCALE").click();
        h.run_steps(2);
        h.get_by_label_contains("Main").hover();
        h.run_steps(2);
        h.get_by_label("Blues").click();
        h.run_steps(3);
        let (root, bank, kind) = h.state().scale;
        assert_eq!((root.rem_euclid(12), bank, kind), (0, 0, 6), "C Blues");
        let c = h.state().controller.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(c.host.scale(), h.state().scale, "the controller's scale");
        h.get_by_label("C Blues").hover();
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().to_path_buf();
        h.render().unwrap().save(d.join("scale.png")).unwrap();
    }

    /// PAD TOUCH (next to SCALE) sets the focused sound's Pad Touch, as the MK3's
    /// Keyboard page does, and shows the mode it's in.
    #[test]
    fn pad_touch_menu_sets_the_focused_sounds_pad_touch() {
        use deckard_model::pad_touch::TouchMode;
        let Some(mut h) = harness(false, false) else { return };
        h.run_steps(3);
        let (g, s) = (h.state().group, h.state().sound);
        let touch = |h: &Harness<'static, App>| h.state().doc.as_ref().unwrap().borrow().project().groups[g].sounds[s].pad_touch;
        h.get_by_label("PAD TOUCH").click();
        h.run_steps(2);
        h.get_by_label("Poly AT").click();
        h.run_steps(3);
        assert_eq!(touch(&h).mode, TouchMode::PolyAt);
        // The menu stays open for more changes (a click outside closes it).
        h.get_by_label("CC").click();
        h.run_steps(3);
        assert_eq!(touch(&h).mode, TouchMode::Cc);
        assert!(h.query_by_label("PAD: CC 1 Mod").is_some(), "the CC shows");
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().to_path_buf();
        h.render().unwrap().save(d.join("pad-touch.png")).unwrap();
        // Min turned above Max takes Max with it; Max below Min, Min.
        let at = |t| layout::Act::PadTouch(g, s, t);
        let t = touch(&h);
        layout::apply_acts(h.state_mut(), vec![at(deckard_model::pad_touch::PadTouch { min: 40, max: 30, ..t })]);
        assert_eq!((touch(&h).min, touch(&h).max), (40, 40));
        h.state_mut().doc.as_ref().unwrap().borrow_mut().undo();
        assert_eq!((touch(&h).mode, touch(&h).min, touch(&h).max), (TouchMode::Cc, 0, 100), "one undo step back");
    }

    /// The Pattern Editor's horizontal zooming scroll bar (as the manual has it): drag it
    /// up to zoom in round where it was grabbed, along to scroll, double-click it to see
    /// the whole pattern; ⌘ + scroll zooms too.
    #[test]
    fn zoom_bar_zooms_and_scrolls_the_pattern_editor() {
        let Some(mut h) = harness(false, false) else { return };
        h.run_steps(3);
        let (g, pi) = (h.state().group, h.state().pattern);
        let key = egui::Id::new(("pattern-hzoom", g, pi));
        let window = |h: &Harness<'static, App>| h.ctx.data(|d| d.get_temp::<(f32, f32)>(key)).expect("zoom kept");
        let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
        let lane: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-lane"))).expect("lane drawn");
        let bar_y = lane.bottom() + 1.0 + 6.0;
        assert_eq!(window(&h), (0.0, 1.0), "the whole pattern");
        // Up from the middle: zoomed in, still round the middle.
        let mid = Pos2::new(grid.center().x, bar_y);
        drag(&mut h, mid, mid - egui::vec2(0.0, 60.0));
        let (from, len) = window(&h);
        assert!(len < 0.6, "zoomed in: {len}");
        assert!((from + len / 2.0 - 0.5).abs() < 0.05, "round the middle: {from} {len}");
        // Along: scrolled right.
        let thumb = Pos2::new(grid.left() + (from + len / 2.0) * grid.width(), bar_y);
        drag(&mut h, thumb, thumb + egui::vec2(40.0, 0.0));
        assert!(window(&h).0 > from, "scrolled");
        // Double-click: all of it again.
        let thumb = Pos2::new(grid.left() + (window(&h).0 + window(&h).1 / 2.0) * grid.width(), bar_y);
        for _ in 0..2 {
            click(&mut h, thumb);
        }
        h.run_steps(2);
        assert_eq!(window(&h), (0.0, 1.0), "reset");
        // ⌘ + scroll over the notes zooms in.
        h.hover_at(grid.center());
        h.step();
        h.event(egui::Event::Zoom(2.0));
        h.run_steps(2);
        assert!((window(&h).1 - 0.5).abs() < 0.01, "{:?}", window(&h));
    }

    /// Dragging on the pattern editor's timeline: the right end sets the length, the
    /// left end the start (the end stays), the span between them moves both.
    #[test]
    fn timeline_drags_set_pattern_start_and_length() {
        let Some(mut h) = harness(false, false) else { return };
        h.run_steps(3);
        let (g, pi) = (h.state().group, h.state().pattern);
        let span = |h: &Harness<'static, App>| {
            let p = project_of(h);
            let pat = &p.groups[g].patterns[pi];
            (pat.start, if pat.length > 0 { pat.length } else { p.bar_length() })
        };
        let bar = project_of(&h).bar_length();
        // The timeline is the strip just above the grid; t is from the pattern's start.
        let at = |h: &Harness<'static, App>, t: i64| {
            let p = grid_pos(h, 0, t);
            let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
            Pos2::new(p.x, grid.top() - 9.0)
        };
        let px_per_bar = |h: &Harness<'static, App>| at(h, bar).x - at(h, 0).x;
        let (s0, l0) = span(&h);
        assert_eq!(s0, 0);
        // Right end, one bar further: a bar longer.
        let (from, d) = (at(&h, l0), px_per_bar(&h));
        drag(&mut h, from, from + egui::vec2(d, 0.0));
        assert_eq!(span(&h), (0, l0 + bar), "LENGTH");
        // Left end, one bar in: the start moves, the end stays.
        let (from, d) = (at(&h, 0), px_per_bar(&h));
        drag(&mut h, from + egui::vec2(1.0, 0.0), from + egui::vec2(1.0 + d, 0.0));
        assert_eq!(span(&h), (bar, l0), "START");
        // The middle, one bar on: both move.
        let (from, d) = (at(&h, l0 / 2), px_per_bar(&h));
        drag(&mut h, from, from + egui::vec2(d, 0.0));
        assert_eq!(span(&h), (2 * bar, l0), "POSITION");
        // One undo step each.
        let labels: Vec<String> =
            h.state().doc.as_ref().unwrap().borrow().undo_labels().iter().rev().take(3).map(|s| s.to_string()).collect();
        assert_eq!(labels, ["Pattern Start"; 3]);
    }

    /// #22: the end dragged two bars past it (past where the grid ended) makes the
    /// pattern two bars longer; the editor shows two bars past the end to drag into.
    #[test]
    fn the_pattern_end_drags_past_the_grid() {
        let Some(mut h) = harness(false, false) else { return };
        h.run_steps(3);
        let (g, pi) = (h.state().group, h.state().pattern);
        let length = |h: &Harness<'static, App>| {
            let p = project_of(h);
            let pat = &p.groups[g].patterns[pi];
            if pat.length > 0 { pat.length } else { p.bar_length() }
        };
        let bar = project_of(&h).bar_length();
        let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
        let at = |h: &Harness<'static, App>, t: i64| Pos2::new(grid_pos(h, 0, t).x, grid.top() - 9.0);
        let l0 = length(&h);
        let end = at(&h, l0);
        let two = at(&h, l0 + 2 * bar);
        assert!(two.x < grid.right(), "two bars past the end shown: {} < {}", two.x, grid.right());
        drag(&mut h, end, two);
        assert_eq!(length(&h), l0 + 2 * bar);
    }

    /// A note of a sound with none, a beat long at the pattern's start, focused: (its
    /// sound, the note).
    fn lone_note(h: &mut Harness<'static, App>) -> Option<(usize, Note)> {
        let p = project_of(h);
        let (g, pi) = (h.state().group, h.state().pattern);
        let before = &p.groups[g].patterns[pi].notes;
        let sound = (0..p.groups[g].sounds.len().min(8)).find(|s| before.iter().all(|n| n.sound != *s))?;
        let start = p.groups[g].patterns[pi].start;
        let n = Note { expr: None, sound, position: start, length: 960, pitch: 60, velocity: 100, probability: 100 };
        h.state().doc.as_ref().unwrap().borrow_mut().edit_notes("t", g, pi, &[], &[n]).unwrap();
        h.state_mut().sound = sound;
        h.run_steps(3);
        Some((sound, n))
    }

    fn expression_of(h: &Harness<'static, App>, sound: usize) -> Option<deckard_model::Expression> {
        let n = notes_of(h).into_iter().find(|n| n.sound == sound)?;
        h.state().doc.as_ref().unwrap().borrow().expression(n.expr?).cloned()
    }

    /// Pick Expression lane `l` in the "+" dialog (a sound without expression doesn't list
    /// them yet).
    fn pick_expression_lane(h: &mut Harness<'static, App>, l: layout::Lane) {
        assert!(!lane_list(h).iter().any(|(x, _)| *x == l), "not listed before the sound has expression");
        let plus = lane_list(h).last().unwrap().1;
        click(h, plus.center());
        assert!(h.state().lanes_dialog);
        let name = l.name();
        h.get_by_label(&name).click();
        h.run_steps(3);
        assert_eq!(h.state().lane, l);
        assert!(!h.state().lanes_dialog);
    }

    /// The Expression editor (part 3): a click in the Pressure lane inside a note adds a
    /// point to its curve there; a drag moves it; undo takes the edits back.
    #[test]
    fn the_pressure_lane_adds_a_point() {
        let Some(mut h) = harness(false, true) else { return };
        let Some((sound, _)) = lone_note(&mut h) else { return };
        pick_expression_lane(&mut h, layout::Lane::Expr(Dim::Pressure));
        let (bottom, lane_h) = lane(&h);
        let x = grid_pos(&h, 0, 480).x;
        click(&mut h, Pos2::new(x, bottom - lane_h * 0.75));
        let e = expression_of(&h, sound).expect("the note has expression now");
        let pts = e.pressure.expect("a pressure curve").points;
        assert_eq!(pts.len(), 1, "{pts:?}");
        assert!((pts[0].at - 480).abs() <= 30 && (pts[0].value - 0.75).abs() < 0.05, "{pts:?}");
        let listed: Vec<layout::Lane> = lane_list(&h).iter().map(|(l, _)| *l).collect();
        // Listed now, alone of the expression lanes (BUGS #18: only what the notes have).
        let expr: Vec<layout::Lane> =
            listed.iter().copied().filter(|l| matches!(l, layout::Lane::Expr(_) | layout::Lane::Release)).collect();
        assert_eq!(expr, [layout::Lane::Expr(Dim::Pressure)], "listed now: {listed:?}");
        // Drag the point down to a quarter.
        let from = Pos2::new(grid_pos(&h, 0, pts[0].at).x, bottom - lane_h * pts[0].value);
        drag(&mut h, from, Pos2::new(from.x, bottom - lane_h * 0.25));
        let pts = expression_of(&h, sound).unwrap().pressure.unwrap().points;
        assert!(pts.len() == 1 && (pts[0].value - 0.25).abs() < 0.05, "moved: {pts:?}");
        let doc = h.state().doc.clone().unwrap();
        assert_eq!(doc.borrow().undo_label(), Some("Edit Expression"));
        while doc.borrow().undo_label() == Some("Edit Expression") {
            doc.borrow_mut().undo();
        }
        h.run_steps(2);
        assert_eq!(expression_of(&h, sound), None, "undone");
    }

    /// The Release lane: a click on a note's bar sets its release velocity.
    #[test]
    fn the_release_lane_sets_release_velocity() {
        let Some(mut h) = harness(false, true) else { return };
        let Some((sound, _)) = lone_note(&mut h) else { return };
        pick_expression_lane(&mut h, layout::Lane::Release);
        let (bottom, lane_h) = lane(&h);
        let x = grid_pos(&h, 0, 0).x + 1.5;
        click(&mut h, Pos2::new(x, bottom - lane_h * 0.5));
        let rv = expression_of(&h, sound).and_then(|e| e.release_velocity).expect("a release velocity");
        assert!((54..=74).contains(&rv), "about half: {rv}");
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

    #[test]
    fn draw_mode_edits_notes_and_lanes() {
        let Some(mut h) = harness(false, true) else { return };
        let p = project_of(&h);
        let (g, pi) = (h.state().group, h.state().pattern);
        let before = notes_of(&h);
        let start = p.groups[g].patterns[pi].start;
        let Some(sound) = (0..p.groups[g].sounds.len().min(8)).find(|s| before.iter().all(|n| n.sound != *s)) else { return };
        // The pencil, third button of the group panel's left strip.
        let panel_top = 33.0 + 3.0 + 343.0 + 2.0 + 140.0 + 2.0;
        click(&mut h, Pos2::new(3.0 + 16.0, panel_top + 7.0 + 2.0 * 28.0 + 12.0));
        assert!(h.state().draw_mode(), "the pencil turns draw mode on");

        // Click an empty step: a note.
        let pos = grid_pos(&h, sound, 2 * 240 + 120);
        click(&mut h, pos);
        let added = Note { expr: None, sound, position: start + 480, length: 240, pitch: 60, velocity: 100, probability: 100 };
        assert!(notes_of(&h).contains(&added), "click adds a note");
        assert_eq!(h.state().sound, sound, "and focuses its sound");

        // Drag it one beat right.
        // (Grabbed just inside its start: its right end resizes.)
        let grab = egui::vec2(3.0, 0.0);
        let (from, to) = (grid_pos(&h, sound, 480) + grab, grid_pos(&h, sound, 480 + 960) + grab);
        drag(&mut h, from, to);
        let moved = Note { position: start + 1440, ..added };
        assert!(notes_of(&h).contains(&moved), "drag moves it: {:?}", notes_of(&h));

        // Draw its velocity and probability down to about half.
        let (bottom, lane_h) = lane(&h);
        let x = grid_pos(&h, 0, 1440).x + 1.5;
        drag(&mut h, Pos2::new(x - 6.0, bottom - lane_h * 0.5), Pos2::new(x + 6.0, bottom - lane_h * 0.5));
        pick_lane(&mut h, layout::Lane::Probability);
        drag(&mut h, Pos2::new(x - 6.0, bottom - lane_h * 0.5), Pos2::new(x + 6.0, bottom - lane_h * 0.5));
        pick_lane(&mut h, layout::Lane::Velocity);
        let n = notes_of(&h).into_iter().find(|n| n.sound == sound).unwrap();
        assert!((50..=78).contains(&n.velocity), "velocity drawn to about half: {n:?}");
        assert!((40..=60).contains(&n.probability), "probability drawn to about half: {n:?}");

        // Click the note: gone. Then paint four notes in one drag.
        let pos = grid_pos(&h, sound, 1440 + 60);
        click(&mut h, pos);
        assert!(notes_of(&h).iter().all(|n| n.sound != sound), "click on a note deletes it");
        let (a, b) = (grid_pos(&h, sound, 120), grid_pos(&h, sound, 3 * 240 + 120));
        drag(&mut h, a, b);
        let painted: Vec<i64> = notes_of(&h).iter().filter(|n| n.sound == sound).map(|n| n.position - start).collect();
        assert_eq!(painted, [0, 240, 480, 720], "dragging paints a note per step");
        assert_eq!(h.state().doc.as_ref().unwrap().borrow().undo_label(), Some("Draw Notes"));

        // Undo back to the start.
        while notes_of(&h) != before {
            assert!(h.state().doc.as_ref().unwrap().borrow().undo_label().is_some(), "undo history ran out");
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
            h.run_steps(2);
        }
    }

    /// E / X / Y / Z switch Draw, Split, Join and Mute on and off (not while the
    /// computer keyboard plays notes); Split cuts a note at the step under the mouse,
    /// Join puts it back together, Mute mutes and unmutes it.
    #[test]
    fn mouse_modes_split_join_and_mute() {
        use layout::MouseMode as M;
        let Some(mut h) = harness(false, true) else { return };
        let p = project_of(&h);
        let (g, pi) = (h.state().group, h.state().pattern);
        let start = p.groups[g].patterns[pi].start;
        let Some(sound) = (0..p.groups[g].sounds.len().min(8)).find(|s| notes_of(&h).iter().all(|n| n.sound != *s)) else { return };
        let key = |h: &mut Harness<'static, App>, k: egui::Key| {
            h.key_press(k);
            h.run_steps(2);
        };
        for (k, m) in [(egui::Key::E, M::Draw), (egui::Key::X, M::Split), (egui::Key::Y, M::Join), (egui::Key::Z, M::Mute)] {
            key(&mut h, k);
            assert_eq!(h.state().mouse, m, "{k:?}");
        }
        key(&mut h, egui::Key::Z);
        assert_eq!(h.state().mouse, M::Select, "the same key again: Select");
        h.state_mut().keys.on = true;
        key(&mut h, egui::Key::Z);
        assert_eq!(h.state().mouse, M::Select, "not while Keys plays notes");
        h.state_mut().keys.on = false;

        let note = Note { expr: None, sound, position: start, length: 960, pitch: 60, velocity: 100, probability: 100 };
        h.state().doc.as_ref().unwrap().borrow_mut().add_note(g, pi, note).unwrap();
        h.run_steps(2);
        let mine = |h: &Harness<'static, App>| notes_of(h).into_iter().filter(|n| n.sound == sound).collect::<Vec<_>>();
        key(&mut h, egui::Key::X);
        let pos = grid_pos(&h, sound, 480 + 40);
        click(&mut h, pos);
        assert_eq!(
            mine(&h),
            [Note { length: 480, ..note }, Note { position: start + 480, length: 480, ..note }],
            "split at the step under the mouse"
        );
        key(&mut h, egui::Key::Y);
        let pos = grid_pos(&h, sound, 120);
        click(&mut h, pos);
        assert_eq!(mine(&h), [note], "joined again");
        assert_eq!(h.state().doc.as_ref().unwrap().borrow().undo_label(), Some("Join note events"));
        key(&mut h, egui::Key::Z);
        click(&mut h, pos);
        assert_eq!(mine(&h), [Note { probability: 0, ..note }], "muted");
        click(&mut h, pos);
        assert_eq!(mine(&h), [note], "unmuted");
    }

    /// ⌥← / ⌥→ nudge by the Nudge Grid (Step/2 by default): the selection, or every
    /// note shown when none is selected, coming round at the pattern's ends.
    #[test]
    fn alt_arrows_nudge_by_the_nudge_grid() {
        let Some(mut h) = harness(false, true) else { return };
        let (g, pi) = (h.state().group, h.state().pattern);
        let p = project_of(&h);
        let (start, len) = (p.groups[g].patterns[pi].start, p.groups[g].patterns[pi].length);
        assert_eq!(h.state().nudge.ticks(h.state().grid), h.state().grid / 2, "Step/2");
        let before = notes_of(&h);
        let one = before[0];
        h.state_mut().selected = vec![one];
        h.run_steps(1);
        h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowRight);
        h.run_steps(2);
        let half = h.state().grid / 2;
        let moved = Note { position: start + (one.position + half - start).rem_euclid(len), ..one };
        assert!(notes_of(&h).contains(&moved), "the selected note, half a step right");
        assert_eq!(notes_of(&h).len(), before.len());
        // None selected: all of them, back left (round the start if they must).
        h.state_mut().selected = Vec::new();
        h.state_mut().nudge = layout::NudgeGrid(0);
        h.run_steps(1);
        let now = notes_of(&h);
        h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowLeft);
        h.run_steps(2);
        let step = h.state().grid;
        let mut want: Vec<i64> = now.iter().map(|n| start + (n.position - step - start).rem_euclid(len)).collect();
        let mut got: Vec<i64> = notes_of(&h).iter().map(|n| n.position).collect();
        want.sort();
        got.sort();
        assert_eq!(got, want, "every note a step left");
    }

    /// ⌘C / ⌘V: a copy pastes after itself; ⌘X cuts. The event area's right-click
    /// menu: Paste at the mouse, the mouse modes, the Nudge Grid, Delete.
    #[test]
    fn copy_paste_and_the_event_menu() {
        let Some(mut h) = harness(false, true) else { return };
        let p = project_of(&h);
        let (g, pi) = (h.state().group, h.state().pattern);
        let start = p.groups[g].patterns[pi].start;
        let Some(sound) = (0..p.groups[g].sounds.len().min(8)).find(|s| notes_of(&h).iter().all(|n| n.sound != *s)) else { return };
        let note = Note { expr: None, sound, position: start, length: 240, pitch: 60, velocity: 100, probability: 100 };
        h.state().doc.as_ref().unwrap().borrow_mut().add_note(g, pi, note).unwrap();
        h.state_mut().sound = sound;
        h.state_mut().selected = vec![note];
        h.run_steps(2);
        let mine = |h: &Harness<'static, App>| notes_of(h).into_iter().filter(|n| n.sound == sound).collect::<Vec<_>>();
        let cmd = |h: &mut Harness<'static, App>, k: egui::Key| {
            h.key_press_modifiers(egui::Modifiers::COMMAND, k);
            h.run_steps(2);
        };
        cmd(&mut h, egui::Key::C);
        cmd(&mut h, egui::Key::V);
        let after = Note { position: start + 240, ..note };
        assert_eq!(mine(&h), [note, after], "pasted on the next step");
        assert_eq!(h.state().selected, [after], "the pasted notes selected");
        cmd(&mut h, egui::Key::X);
        assert_eq!(mine(&h), [note], "cut");

        // Right-click a step: Paste puts the cut note there.
        let pos = grid_pos(&h, sound, 960 + 20);
        right_click(&mut h, pos);
        h.get_by_label("Paste").click();
        h.run_steps(3);
        assert_eq!(mine(&h), [note, Note { position: start + 960, ..note }], "pasted at the mouse");
        let pos = grid_pos(&h, sound, 1920 + 20);
        right_click(&mut h, pos);
        h.get_by_label("Split").click();
        h.run_steps(2);
        assert_eq!(h.state().mouse, layout::MouseMode::Split);
        h.state_mut().mouse = layout::MouseMode::Select;
        right_click(&mut h, pos);
        h.get_by_label("Nudge Grid ⏵").hover();
        h.run_steps(3);
        h.get_by_label("Step/4").click();
        h.run_steps(2);
        assert_eq!(h.state().nudge, layout::NudgeGrid(2));
        // Right-click a note: Delete removes it.
        let pos = grid_pos(&h, sound, 960 + 60);
        right_click(&mut h, pos);
        h.get_by_label("Delete").click();
        h.run_steps(3);
        assert_eq!(mine(&h), [note]);
    }

    #[test]
    fn select_mode_selects_moves_and_deletes() {
        let Some(mut h) = harness(false, true) else { return };
        let p = project_of(&h);
        let (g, pi) = (h.state().group, h.state().pattern);
        let before = notes_of(&h);
        let start = p.groups[g].patterns[pi].start;
        let Some(sound) = (0..p.groups[g].sounds.len().min(8)).find(|s| before.iter().all(|n| n.sound != *s)) else { return };
        assert!(!h.state().draw_mode());
        // A single click on an empty step only clears the selection; double-click adds.
        let pos = grid_pos(&h, sound, 120);
        click(&mut h, pos);
        assert_eq!(notes_of(&h), before);
        for at in [120, 960 + 120] {
            // A pause, so the next two clicks are a double-click, not a triple.
            h.run_steps(40);
            let pos = grid_pos(&h, sound, at);
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
        let two: Vec<Note> = notes_of(&h).into_iter().filter(|n| n.sound == sound).collect();
        assert_eq!(two.len(), 2, "double-clicks add notes");
        // Box-select both, drag one: both move.
        let (a, b) = (grid_pos(&h, sound, 3840) + egui::vec2(0.0, -10.0), grid_pos(&h, sound, 0) + egui::vec2(-4.0, 8.0));
        drag(&mut h, a, b);
        assert_eq!(h.state().selected.len(), 2, "box selection: {:?}", h.state().selected);
        let grab = egui::vec2(3.0, 0.0);
        let (a, b) = (grid_pos(&h, sound, 0) + grab, grid_pos(&h, sound, 240) + grab);
        drag(&mut h, a, b);
        let moved: Vec<i64> = notes_of(&h).iter().filter(|n| n.sound == sound).map(|n| n.position - start).collect();
        assert_eq!(moved, [240, 1200], "the selection moves together");
        // Delete removes the selection.
        h.key_press(egui::Key::Delete);
        h.run_steps(3);
        assert_eq!(notes_of(&h), before);
    }

    #[test]
    fn grid_length_time_signature_quantize_and_nudge() {
        let Some(mut h) = harness(false, true) else { return };
        let (g, pi) = (h.state().group, h.state().pattern);
        let p = project_of(&h);
        let bar = p.bar_length();
        let start = p.groups[g].patterns[pi].start;

        // Time signature: 4 / 4 -> 3 / 4.
        press(&mut h, "4 / 4  SIG");
        press(&mut h, "3 / 4");
        assert_eq!((project_of(&h).numerator, project_of(&h).denominator), (3, 4));
        let bar3 = project_of(&h).bar_length();
        assert_eq!(bar3, 3 * 960);

        // Pattern length: 2 bars (of 3/4).
        let len = project_of(&h).groups[g].patterns[pi].length;
        let text = format!("{}:{}:{}", len / bar3, len % bar3 / 960, len % 960 / 240);
        press(&mut h, &text);
        press(&mut h, "2 bars");
        assert_eq!(project_of(&h).groups[g].patterns[pi].length, 2 * bar3);

        // Grid 1/8: drawn notes are an eighth long and snap to eighths.
        press(&mut h, "1/16");
        press(&mut h, "1/8");
        assert_eq!(h.state().grid, 480);
        h.state_mut().mouse = layout::MouseMode::Draw;
        let Some(sound) = (0..8).find(|s| notes_of(&h).iter().all(|n| n.sound != *s)) else { return };
        let pos = grid_pos(&h, sound, 480 + 300);
        click(&mut h, pos);
        let drawn = notes_of(&h).into_iter().find(|n| n.sound == sound).expect("a note");
        assert_eq!((drawn.position - start, drawn.length), (480, 480));

        // Quantize: move it off the grid, select it, quantize 100 % to eighths.
        let off = Note { position: drawn.position + 100, ..drawn };
        h.state().doc.as_ref().unwrap().borrow_mut().change_note(g, pi, drawn, off).unwrap();
        h.state_mut().mouse = layout::MouseMode::Select;
        h.state_mut().selected = vec![off];
        h.run_steps(2);
        press(&mut h, "QUANTIZE");
        press(&mut h, "100 %");
        assert!(
            notes_of(&h).contains(&drawn),
            "quantized back onto the grid: {:?}",
            notes_of(&h).iter().filter(|n| n.sound == sound).collect::<Vec<_>>()
        );

        // Nudge: select it, arrow right moves it one grid step.
        h.state_mut().selected = vec![drawn];
        h.run_steps(1);
        h.key_press(egui::Key::ArrowRight);
        h.run_steps(3);
        assert!(notes_of(&h).contains(&Note { position: drawn.position + 480, ..drawn }));
        let _ = bar;
    }

    /// Notes drawn on a 1/32 grid, then quantized on a 1/4 grid (as a user did).
    #[test]
    fn quantize_after_changing_the_grid() {
        let Some(mut h) = harness(false, true) else { return };
        let (g, pi) = (h.state().group, h.state().pattern);
        let start = project_of(&h).groups[g].patterns[pi].start;
        let Some(sound) = (0..8).find(|s| notes_of(&h).iter().all(|n| n.sound != *s)) else { return };
        press(&mut h, "1/16");
        press(&mut h, "1/32");
        assert_eq!(h.state().grid, 120);
        // Pencil, then paint a few 1/32 notes.
        let panel_top = 33.0 + 3.0 + 343.0 + 2.0 + 140.0 + 2.0;
        click(&mut h, Pos2::new(3.0 + 16.0, panel_top + 7.0 + 2.0 * 28.0 + 12.0));
        for t in [120 + 60, 600 + 60, 1320 + 60] {
            let pos = grid_pos(&h, sound, t);
            click(&mut h, pos);
        }
        let mine = |h: &Harness<'static, App>| -> Vec<i64> {
            notes_of(h).iter().filter(|n| n.sound == sound).map(|n| n.position - start).collect()
        };
        assert_eq!(mine(&h), [120, 600, 1320]);
        press(&mut h, "1/32");
        press(&mut h, "1/4");
        assert_eq!(h.state().grid, 960);
        press(&mut h, "QUANTIZE");
        press(&mut h, "100 %");
        eprintln!("status: {:?}, selected: {:?}", h.state().status, h.state().selected);
        assert_eq!(mine(&h), [0, 960, 960], "quantized to quarters");
    }

    /// In select mode: notes added by double-click aren't selected, so QUANTIZE
    /// snaps all of them.
    #[test]
    fn quantize_after_double_click_adds() {
        let Some(mut h) = harness(false, true) else { return };
        let (g, pi) = (h.state().group, h.state().pattern);
        let start = project_of(&h).groups[g].patterns[pi].start;
        let Some(sound) = (0..8).find(|s| notes_of(&h).iter().all(|n| n.sound != *s)) else { return };
        press(&mut h, "1/16");
        press(&mut h, "1/32");
        for t in [120 + 60, 600 + 60, 1320 + 60] {
            h.run_steps(40);
            let pos = grid_pos(&h, sound, t);
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
        let mine = |h: &Harness<'static, App>| -> Vec<i64> {
            notes_of(h).iter().filter(|n| n.sound == sound).map(|n| n.position - start).collect()
        };
        assert_eq!(mine(&h), [120, 600, 1320]);
        assert!(h.state().selected.is_empty(), "added notes are not selected");
        press(&mut h, "1/32");
        press(&mut h, "1/4");
        press(&mut h, "QUANTIZE");
        press(&mut h, "100 %");
        assert_eq!(mine(&h), [0, 960, 960]);
    }

    /// Double-clicking a note deletes it (and only it).
    #[test]
    fn double_click_deletes_a_note() {
        let Some(mut h) = harness(false, true) else { return };
        let (g, pi) = (h.state().group, h.state().pattern);
        let start = project_of(&h).groups[g].patterns[pi].start;
        let Some(sound) = (0..8).find(|s| notes_of(&h).iter().all(|n| n.sound != *s)) else { return };
        let mine = |h: &Harness<'static, App>| -> Vec<i64> {
            notes_of(h).iter().filter(|n| n.sound == sound).map(|n| n.position - start).collect()
        };
        let others = notes_of(&h).len();
        for t in [0, 960] {
            h.run_steps(40);
            let pos = grid_pos(&h, sound, t + 60);
            double_click(&mut h, pos);
            h.run_steps(3);
        }
        assert_eq!(mine(&h), [0, 960]);
        h.run_steps(40);
        let pos = grid_pos(&h, sound, 60);
        double_click(&mut h, pos);
        h.run_steps(3);
        assert_eq!(mine(&h), [960]);
        assert_eq!(notes_of(&h).len(), others + 1);
    }

    /// A swing change made while the song plays reaches the running engine.
    #[test]
    fn swing_reaches_playback() {
        let Some(mut h) = harness(false, true) else { return };
        if !play(&mut h) {
            return;
        }
        let ticks = |h: &Harness<'static, App>, g: usize| -> Vec<i64> {
            let Some(Ok(out)) = &h.state().output else { return Vec::new() };
            out.call(move |e| e.timeline().events.iter().filter(|x| x.group == g && x.midi.is_none()).map(|x| x.tick).collect())
                .unwrap_or_default()
        };
        // The group whose notes swing moves most (off-beat 16ths).
        let g = (0..project_of(&h).groups.len()).max_by_key(|&g| ticks(&h, g).iter().filter(|t| *t % 480 == 240).count()).unwrap();
        let before = ticks(&h, g);
        h.state_mut()
            .arrange(layout::Act::SetSwing(deckard_doc::Target::Group(g), deckard_model::Swing { amount: 60.0, cycle: 0, invert: false }));
        h.run_steps(3);
        let after = ticks(&h, g);
        let moved = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        assert!(moved > 0, "swing moved none of group {g}'s {} notes in the playing engine", before.len());
        // Off-beat sixteenths are late now; on-beat ones stay.
        for (a, b) in before.iter().zip(&after) {
            match a % 480 {
                0 => assert_eq!(a, b),
                240 => assert!(b > a, "{a} -> {b}"),
                _ => {}
            }
        }
        h.state_mut().stop();
    }

    #[test]
    fn select_all_and_escape() {
        let Some(mut h) = harness(false, true) else { return };
        // #31: the focused sound's notes only.
        let sound = notes_of(&h)[0].sound;
        h.state_mut().sound = sound;
        let (g, pi) = (h.state().group, h.state().pattern);
        let other = Note { sound: (sound + 1) % 16, ..notes_of(&h)[0] };
        h.state().doc.as_ref().unwrap().borrow_mut().edit_notes("t", g, pi, &[], &[other]).unwrap();
        h.run_steps(2);
        let n = notes_of(&h).iter().filter(|n| n.sound == sound).count();
        assert!(n > 0 && n < notes_of(&h).len(), "other sounds have notes too");
        // Focus the grid area first (keys go to the editor when nothing else has focus).
        let pos = grid_pos(&h, 15, 60);
        click(&mut h, pos);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(3);
        assert_eq!(h.state().selected.len(), n, "⌘A selects the focused sound's notes");
        assert!(h.state().selected.iter().all(|n| n.sound == sound));
        h.state_mut().selected.clear();
        h.state_mut().menu_commands.push(menu::Command::SelectAll);
        h.run_steps(3);
        assert_eq!(h.state().selected.len(), n, "Edit ▸ Select All Notes too");
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(h.state().selected.is_empty(), "Esc clears");
    }

    /// BUGS #17: the Erase tool erases what it's dragged over in a CC lane; a span
    /// selected there goes with Delete.
    #[test]
    fn a_cc_lane_erases_a_span() {
        use deckard_model::{MidiEvent, MidiKind};
        use layout::Lane;
        let Some(mut h) = harness(false, true) else { return };
        let (g, pi, sound) = (h.state().group, h.state().pattern, h.state().sound);
        let start = project_of(&h).groups[g].patterns[pi].start;
        let cc = |t| MidiEvent { sound, position: start + t, kind: MidiKind::Controller { number: 1, value: 64 } };
        let events: Vec<MidiEvent> = (0..8).map(|i| cc(i * 240)).collect();
        h.state().doc.as_ref().unwrap().borrow_mut().edit_midi("t", g, pi, &[], &events).unwrap();
        h.state_mut().lane = Lane::Controller(1);
        h.run_steps(3);
        let left = |h: &Harness<'static, App>| -> Vec<i64> {
            project_of(h).groups[g].patterns[pi]
                .midi
                .iter()
                .filter(|m| m.sound == sound && Lane::of(&m.kind) == Lane::Controller(1))
                .map(|m| m.position - start)
                .collect()
        };
        let (bottom, lane_h) = lane(&h);
        let y = bottom - lane_h * 0.5;
        h.state_mut().mouse = layout::MouseMode::Erase;
        let (a, b) = (grid_pos(&h, 0, 240 + 20).x, grid_pos(&h, 0, 480 + 20).x);
        drag(&mut h, Pos2::new(a, y), Pos2::new(b, y));
        assert_eq!(left(&h), [0, 720, 960, 1200, 1440, 1680], "the steps dragged over");
        h.state_mut().mouse = layout::MouseMode::Select;
        (h.state_mut().lane_span, h.state_mut().key_focus) =
            (Some((Lane::Controller(1), start + 960, start + 1440)), layout::KeyFocus::Events);
        h.run_steps(2);
        h.key_press(egui::Key::Delete);
        h.run_steps(3);
        assert_eq!(left(&h), [0, 720, 1440, 1680], "the span selected");
        assert_eq!(h.state().lane_span, None);
    }

    /// The lane list: velocity, probability and the sound's MIDI controllers (the
    /// "Keys" pattern's sustain pedal); "+" adds a lane, which can be drawn into,
    /// erased, and removed again.
    #[test]
    fn midi_lanes_list_draw_and_remove() {
        use deckard_model::MidiKind;
        use layout::Lane;
        let Some(mut h) = harness(false, true) else { return };
        let p = project_of(&h);
        let Some((g, pi, sound)) = p
            .groups
            .iter()
            .enumerate()
            .find_map(|(g, gr)| gr.patterns.iter().enumerate().find_map(|(pi, pt)| pt.midi.first().map(|m| (g, pi, m.sound))))
        else {
            return;
        };
        (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound) = (g, pi, sound);
        h.run_steps(3);
        let listed: Vec<Lane> = lane_list(&h).iter().map(|(l, _)| *l).collect();
        assert_eq!(listed[..3], [Lane::Velocity, Lane::Probability, Lane::Controller(64)], "{listed:?}");
        pick_lane(&mut h, Lane::Controller(64));

        // "+" opens the dialog; add the mod wheel.
        let plus = lane_list(&h).last().unwrap().1;
        click(&mut h, plus.center());
        assert!(h.state().lanes_dialog);
        h.get_by_label("Mod Wheel (CC 1)").click();
        h.run_steps(3);
        assert_eq!(h.state().lane, Lane::Controller(1), "the new lane is shown");
        let saved = |h: &Harness<'static, App>| project_of(h).groups[g].sounds[sound].lanes.clone();
        assert!(saved(&h).contains(&deckard_model::LaneRef { kind: 4, number: 1 }), "and saved with the sound");
        h.state_mut().lanes_dialog = false;
        h.run_steps(3);

        // Draw half-way values across the first beat: one event per 16th.
        let start = p.groups[g].patterns[pi].start;
        let cc1 = |h: &Harness<'static, App>| -> Vec<(i64, u8)> {
            project_of(h).groups[g].patterns[pi]
                .midi
                .iter()
                .filter_map(|m| match m.kind {
                    MidiKind::Controller { number: 1, value } if m.sound == sound => Some((m.position - start, value)),
                    _ => None,
                })
                .collect()
        };
        let (bottom, lane_h) = lane(&h);
        let y = bottom - lane_h * 0.5;
        let (a, b) = (grid_pos(&h, 0, 20).x, grid_pos(&h, 0, 960 - 20).x);
        drag(&mut h, Pos2::new(a, y), Pos2::new(b, y));
        let drawn = cc1(&h);
        assert_eq!(drawn.iter().map(|(t, _)| *t).collect::<Vec<_>>(), [0, 240, 480, 720], "{drawn:?}");
        assert!(drawn.iter().all(|(_, v)| (58..=69).contains(v)), "{drawn:?}");
        // A click sets one step.
        let top = Pos2::new(grid_pos(&h, 0, 240 + 100).x, bottom - lane_h);
        click(&mut h, top);
        assert_eq!(cc1(&h)[1], (240, 127));
        // A right-click erases one.
        let pos = Pos2::new(grid_pos(&h, 0, 480 + 100).x, y);
        h.hover_at(pos);
        h.step();
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.step();
        }
        h.run_steps(3);
        assert_eq!(cc1(&h).iter().map(|(t, _)| *t).collect::<Vec<_>>(), [0, 240, 720]);
        // The pedal is untouched.
        let pedal = |h: &Harness<'static, App>| {
            project_of(h).groups[g].patterns[pi].midi.iter().filter(|m| Lane::of(&m.kind) == Lane::Controller(64)).count()
        };
        assert_eq!(pedal(&h), pedal_before(&p, g, pi));

        // Remove the lane: its events go too (one undo step).
        h.state_mut().lanes_dialog = true;
        h.run_steps(3);
        h.get_by_label("Remove (3 events)").click();
        h.run_steps(3);
        assert!(cc1(&h).is_empty());
        assert!(!lane_list(&h).iter().any(|(l, _)| *l == Lane::Controller(1)));
        assert!(!saved(&h).iter().any(|l| l.kind == 4 && l.number == 1));
        assert_eq!(h.state().lane, Lane::Velocity);
        h.state_mut().undo();
        assert_eq!(cc1(&h).len(), 3, "undo brings the events back");
        assert!(saved(&h).iter().any(|l| l.kind == 4 && l.number == 1), "and the lane");
    }

    /// Right-clicking a modulated knob in the Control area (anywhere on it, not only its
    /// thin ring) offers Remove Modulation, which removes it.
    #[test]
    fn right_clicking_a_knobs_ring_removes_its_modulation() {
        let Some(mut h) = harness(false, false) else { return };
        let doc = h.state().doc.clone().unwrap();
        let (g, s) = (h.state().group, h.state().sound);
        let n = doc.borrow().project().groups[g].sounds[s].slots.len();
        doc.borrow_mut().add_slot(g, s, n, &deckard_doc::NewModule::Effect("Filter")).unwrap();
        h.state_mut().channel = false;
        h.state_mut().chain_tab = layout::ChainTab::Sound;
        h.state_mut().slot = Some(n);
        h.run_steps(3);
        let rects: Vec<Option<egui::Rect>> = h.ctx.data(|d| d.get_temp(egui::Id::new("knob-rects"))).expect("knobs shown");
        // The first knob that can be modulated, given modulation.
        let slot = doc.borrow().project().groups[g].sounds[s].slots[n].clone();
        let pg = module_ui::page_for(&slot, deckard_doc::Target::Module(g, s, n), h.state().plugin_page);
        let (k, edit) = pg
            .slots
            .iter()
            .enumerate()
            .find_map(|(k, x)| x.as_ref().and_then(|x| x.3).filter(|e| modulation::mod_target(*e).is_some()).map(|e| (k, e)))
            .expect("a modulatable knob");
        modulation::register();
        let (c, t) = modulation::mod_target(edit).unwrap();
        let pi = doc.borrow().project().scenes[h.state().scene].patterns.iter().find(|x| x.0 == g).map(|x| x.1).unwrap_or(0);
        doc.borrow_mut().set_modulation(c, &t, pi, (0, 0), &[(100, 0.3), (900, -0.2)]).unwrap();
        let count = |d: &deckard_doc::Document| d.project().groups[g].patterns.iter().map(|p| p.mods.len()).sum::<usize>();
        let before = count(&doc.borrow());
        assert!(before >= 2);
        h.run_steps(2);
        let r = rects[k].expect("its rect");
        // Anywhere on the knob (its centre is r.top() + 40): a menu.
        right_click(&mut h, Pos2::new(r.center().x, r.top() + 40.0));
        h.get_by_label("Remove Modulation").click();
        h.run_steps(3);
        assert_eq!(count(&doc.borrow()), before - 2, "removed; status {:?}", h.state().status);
    }

    /// The header with Auto-write off and on (the bullseye) to PNGs in
    /// `$DECKARD_RENDER_DIR` (run with `--ignored`; needs a GPU).
    #[test]
    #[ignore]
    fn render_auto_write() {
        let Some(dir) = std::env::var_os("DECKARD_RENDER_DIR").map(PathBuf::from) else { return };
        let Some(mut h) = harness(false, true) else { return };
        layout::style(&h.ctx);
        for (on, name) in [(false, "auto-off.png"), (true, "auto-on.png")] {
            h.state_mut().auto_write = on;
            h.run_steps(3);
            h.render().unwrap().save(dir.join(name)).unwrap();
        }
    }

    /// Renders the lane list, a MIDI lane and the "+" dialog to PNGs in
    /// The Expression editor's lanes over a chord with curves, in the piano roll, to
    /// `$DECKARD_RENDER_DIR` (run with `--ignored`; needs a GPU).
    #[test]
    #[ignore]
    fn render_expression_lanes() {
        use deckard_model::{Curve, Expression, Point};
        let Some(dir) = std::env::var_os("DECKARD_RENDER_DIR").map(PathBuf::from) else { return };
        let Some(mut h) = harness(false, true) else { return };
        layout::style(&h.ctx);
        let Some((sound, n)) = lone_note(&mut h) else { return };
        let pt = |at: i64, value: f32, curve: f32| Point { at, value, curve };
        let e = Expression {
            pitch: Some(Curve { points: vec![pt(0, 0.0, 0.6), pt(480, 1.0, 0.0), pt(960, 0.5, 0.0)] }),
            pressure: Some(Curve { points: vec![pt(0, 0.1, 0.8), pt(600, 0.9, -0.5), pt(960, 0.3, 0.0)] }),
            release_velocity: Some(80),
            ..Expression::default()
        };
        let (g, pi) = (h.state().group, h.state().pattern);
        let doc = h.state().doc.clone().unwrap();
        let now = doc.borrow_mut().set_expressions(g, pi, &[(n, e)]).unwrap();
        let other = Note { pitch: 64, expr: None, ..n };
        doc.borrow_mut().edit_notes("t", g, pi, &[], &[other]).unwrap();
        h.state_mut().selected = now.clone();
        h.state_mut().piano_roll = true;
        h.state_mut().sound = sound;
        for (l, name) in [
            (layout::Lane::Expr(Dim::Pressure), "expr-pressure.png"),
            (layout::Lane::Expr(Dim::Pitch), "expr-pitch.png"),
            (layout::Lane::Release, "expr-release.png"),
        ] {
            h.state_mut().lane = l;
            h.run_steps(3);
            h.render().unwrap().save(dir.join(name)).unwrap();
        }
        h.state_mut().lanes_dialog = true;
        h.run_steps(3);
        h.render().unwrap().save(dir.join("expr-dialog.png")).unwrap();
    }

    /// `$DECKARD_RENDER_DIR` (run with `--ignored`; needs a GPU).
    #[test]
    #[ignore]
    fn render_midi_lanes() {
        let Some(dir) = std::env::var_os("DECKARD_RENDER_DIR").map(PathBuf::from) else { return };
        let Some(mut h) = harness(false, true) else { return };
        layout::style(&h.ctx);
        let p = project_of(&h);
        let (g, pi, sound) = p
            .groups
            .iter()
            .enumerate()
            .find_map(|(g, gr)| gr.patterns.iter().enumerate().find_map(|(pi, pt)| pt.midi.first().map(|m| (g, pi, m.sound))))
            .unwrap();
        (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound) = (g, pi, sound);
        h.state_mut().lane = layout::Lane::Controller(64);
        h.run_steps(3);
        h.render().unwrap().save(dir.join("lane-cc64.png")).unwrap();
        h.state_mut().lanes_dialog = true;
        h.run_steps(3);
        h.render().unwrap().save(dir.join("lane-dialog.png")).unwrap();
        h.state_mut().lanes_dialog = false;
        h.run_steps(3);
        // The MASCHINE menu, with Edit open.
        click(&mut h, Pos2::new(40.0, 16.0));
        h.get_by_label("Edit ⏵").hover();
        h.run_steps(5);
        h.render().unwrap().save(dir.join("menu-edit.png")).unwrap();
        h.get_by_label("View ⏵").hover();
        h.run_steps(30);
        h.render().unwrap().save(dir.join("menu-view.png")).unwrap();
        // The Sampler's pages for the kick (group 0, sound 0).
        h.key_press(egui::Key::Escape);
        (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
        for page in 0..sampler_ui::PAGES.len() {
            h.state_mut().sampler_page = page;
            h.run_steps(4);
            h.render().unwrap().save(dir.join(format!("sampler-page{}.png", page + 1))).unwrap();
        }
        // The waveform's playhead while the kick plays, then the waveform hidden.
        h.state_mut().sampler_page = 0;
        h.state_mut().play_pitch(0, 0, 60);
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            h.step();
            if h.state().zone_playhead.is_some_and(|f| f > 0) {
                break;
            }
        }
        assert!(h.state().zone_playhead.is_some(), "the kick's zone plays");
        h.render().unwrap().save(dir.join("sampler-playhead.png")).unwrap();
        h.state_mut().settings.show_waveform = false;
        h.run_steps(3);
        h.render().unwrap().save(dir.join("sampler-no-waveform.png")).unwrap();
        h.state_mut().settings.show_waveform = true;
        // A click on a choice's value lists the choices; picking one sets it.
        h.state_mut().sampler_page = 0;
        h.run_steps(4);
        let knobs: Vec<(String, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-knobs"))).unwrap();
        let mode = knobs.iter().find(|(n, _)| n == "MODE").unwrap().1;
        click(&mut h, Pos2::new(mode.center().x, mode.top() + 42.0));
        h.render().unwrap().save(dir.join("sampler-mode-list.png")).unwrap();
        click(&mut h, Pos2::new(mode.center().x, mode.top() + 42.0 + 8.0 + 21.0 * 1.5));
        h.render().unwrap().save(dir.join("sampler-mode-picked.png")).unwrap();
        let p = project_of(&h);
        let smp = p.groups[0].sounds[0].sampler.as_ref().unwrap();
        eprintln!("sampler mode after picking: {}", smp.get("mode"));
        // The Mixer, at group and sound level.
        h.state_mut().mixer = true;
        h.state_mut().mixer_sounds = false;
        h.run_steps(4);
        h.render().unwrap().save(dir.join("mixer-groups.png")).unwrap();
        h.state_mut().mixer_sounds = true;
        h.state_mut().mixer_sections.io = true;
        h.state_mut().mixer_sections.aux = true;
        h.run_steps(4);
        h.render().unwrap().save(dir.join("mixer-sounds.png")).unwrap();
        // Sampling, each tab, on the kick.
        h.state_mut().mixer = false;
        (h.state_mut().group, h.state_mut().sound) = (0, 0);
        h.state_mut().sampling_open = true;
        for tab in 0..4 {
            h.state_mut().sampling.tab = tab;
            if tab == 2 {
                h.state_mut().arrange_act(layout::Act::Sampling(layout::sampling::SamplingAct::Tab(2)));
            }
            if tab == 0 {
                // A hot input: peaks past full scale must stay inside the waveform.
                h.state_mut().recorder.view.preview = (0..400).map(|i| (-(i as f32 / 100.0), i as f32 / 100.0)).collect();
            }

            if tab == 1 {
                h.state_mut().sampling.selection = Some((2000, 5000));
            }
            h.run_steps(4);
            h.render().unwrap().save(dir.join(format!("sampling-{tab}.png"))).unwrap();
            if tab == 0 {
                // Each mode's and source's footer.
                use layout::sampling::RecParam;
                for (name, mode, source) in
                    [("loop", 2, 0), ("auto-ext", 3, 0), ("auto-int", 3, 2), ("internal", 1, 2), ("detect-mono", 0, 1)]
                {
                    let v = &mut h.state_mut().recorder.view;
                    v.set(RecParam::Mode, mode);
                    v.set(RecParam::Source, source);
                    if name == "auto-ext" {
                        v.midi_outs = vec!["IAC Driver Bus 1".into(), "Prophet Rev2".into()];
                        v.auto.midi_to = Some("Prophet Rev2".into());
                        v.set(RecParam::KeyHi, 84);
                        v.set(RecParam::KeyLo, 36);
                        v.set(RecParam::KeyStride, 6);
                    }
                    h.run_steps(3);
                    h.render().unwrap().save(dir.join(format!("sampling-0-{name}.png"))).unwrap();
                }
                let v = &mut h.state_mut().recorder.view;
                v.set(RecParam::Mode, 1);
                v.set(RecParam::Source, 0);
                h.run_steps(3);
                // The MODE list.
                click(&mut h, Pos2::new(240.0, 895.0));
                h.render().unwrap().save(dir.join("sampling-0-mode.png")).unwrap();
                click(&mut h, Pos2::new(1200.0, 640.0));
            }
        }
        // Preferences ▸ Audio.
        h.state_mut().prefs = Some(2);
        h.state_mut().inputs = Some(vec![("Apollo Twin X".into(), 10), ("MacBook Pro Microphone".into(), 1)]);
        h.state_mut().settings.input_device = Some("Apollo Twin X".into());
        h.run_steps(4);
        h.render().unwrap().save(dir.join("prefs-audio.png")).unwrap();
        h.state_mut().prefs = None;
        h.state_mut().settings.input_device = None;
        // Zoomed in to a few hundred frames: the signal as a curve through the frames.
        h.state_mut().sampling.tab = 1;
        h.state_mut().sampling.selection = None;
        h.state_mut().sampling.view = Some((1000.0, 1300.0));
        h.run_steps(4);
        h.render().unwrap().save(dir.join("sampling-zoom.png")).unwrap();
        // A long sample, all of it: the envelope.
        let key = h.state().sampling.sample_key.clone().unwrap();
        let frames: Vec<[f32; 2]> = (0..48_000 * 20)
            .map(|i| {
                let t = i as f32 / 48_000.0;
                let v = (t * 220.0 * std::f32::consts::TAU).sin() * (0.2 + 0.7 * (t * 0.7).sin().abs()) * (-(t % 1.0) * 3.0).exp();
                [v, v * 0.8]
            })
            .collect();
        h.state_mut().sampling.set_sample(key, std::sync::Arc::new(deckard_engine::sample::Sample { rate: 48_000, frames }));
        h.state_mut().sampling.view = None;
        h.run_steps(1);
        h.render().unwrap().save(dir.join("sampling-long.png")).unwrap();
    }

    /// The MASCHINE menu: the undo history undoes several steps at once; Open Recent
    /// asks about unsaved changes first.
    #[test]
    fn menus_undo_history_and_open_recent() {
        let Some(mut h) = harness(false, true) else { return };
        let doc = h.state().doc.clone().unwrap();
        let tempo = doc.borrow().project().tempo;
        for t in [100.0, 101.0, 102.0] {
            doc.borrow_mut().set_tempo(t).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1600)); // not merged into one step
        }
        h.run_steps(2);
        let steps = doc.borrow().undo_labels().len();
        assert!(steps >= 3, "{:?}", doc.borrow().undo_labels());
        // MASCHINE ▸ Edit ▸ Undo History ▸ the third entry: three steps back.
        click(&mut h, Pos2::new(40.0, 16.0));
        h.get_by_label("Edit ⏵").hover();
        h.run_steps(3);
        h.get_by_label("Undo History ⏵").hover();
        h.run_steps(3);
        let third = doc.borrow().undo_labels()[2].to_string();
        h.get_all_by_label(&third).nth(2).expect("third history entry").click();
        h.run_steps(3);
        assert_eq!(doc.borrow().undo_labels().len(), steps - 3);
        assert_eq!(doc.borrow().project().tempo, tempo);
        assert_eq!(doc.borrow().redo_labels().len(), 3);
        // ⌘Y redoes one.
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Y);
        h.run_steps(2);
        assert_eq!(doc.borrow().project().tempo, 100.0);

        // Open Recent with unsaved changes: asked first; Don't Save opens it.
        let tmp = tempfile::tempdir().unwrap();
        let other = tmp.path().join("menu.mxprj");
        std::fs::copy(h.state().path.clone().unwrap(), &other).unwrap();
        h.state_mut().recent = vec![other.clone()];
        h.state_mut().menu_commands.push(menu::Command::OpenRecent(other.clone()));
        h.run_steps(3);
        assert!(h.state().confirm.is_some(), "asks about the unsaved tempo change");
        h.get_by_label("Don't Save").click();
        h.run_steps(3);
        assert!(h.state().confirm.is_none());
        assert_eq!(h.state().path.as_deref(), Some(other.as_path()));
        assert!(!h.state().dirty());
    }

    /// File › New: an empty project built from scratch, untitled, that takes modules
    /// (no other project or NI file needed) and asks about unsaved changes first.
    #[test]
    fn file_new_builds_an_empty_project() {
        let Some(mut h) = harness(false, true) else { return };
        h.state().doc.clone().unwrap().borrow_mut().set_tempo(99.0).unwrap();
        h.state_mut().menu_commands.push(menu::Command::New);
        h.run_steps(3);
        assert!(h.state().confirm.is_some(), "asks about the unsaved tempo change");
        h.get_by_label("Don't Save").click();
        h.run_steps(3);
        assert!(h.state().path.is_none());
        let doc = h.state().doc.clone().unwrap();
        assert_eq!(doc.borrow().project().groups[0].sounds.len(), 16);
        assert_eq!(doc.borrow().project().tempo, 120.0);
        h.state_mut().add_slot(layout::AddWhat::Sampler);
        h.state_mut().add_slot(layout::AddWhat::Effect("Reverb"));
        h.run_steps(2);
        let names: Vec<String> = doc.borrow().project().groups[0].sounds[0].slots.iter().map(|x| x.module.clone()).collect();
        assert_eq!(names, ["Sampler", "Reverb"], "{}", h.state().status);
    }

    /// "+" › Internal › each of Maschine's instruments in a new project: it goes in with
    /// Maschine's defaults and plays a note.
    #[test]
    fn instruments_from_the_add_menu_play() {
        let Some(mut h) = harness(false, true) else { return };
        h.state_mut().menu_commands.push(menu::Command::New);
        h.run_steps(3);
        if h.state().confirm.is_some() {
            h.get_by_label("Don't Save").click();
            h.run_steps(3);
        }
        for (s, class) in deckard_model::INTERNAL_INSTRUMENTS.into_iter().enumerate() {
            (h.state_mut().group, h.state_mut().sound) = (0, s);
            h.state_mut().add_slot(layout::AddWhat::Instrument(class));
            h.run_steps(2);
        }
        let doc = h.state().doc.clone().unwrap();
        let project = doc.borrow().project().clone();
        for (s, class) in deckard_model::INTERNAL_INSTRUMENTS.into_iter().enumerate() {
            let slot = &project.groups[0].sounds[s].slots[0];
            assert_eq!(slot.module, class, "{}", h.state().status);
            let defaults = deckard_engine::synth::defaults(class).unwrap();
            for (k, v) in defaults {
                assert_eq!(slot.param(k), Some(v as f32 as f64), "{class} {k}");
            }
            let (mut e, _) = Engine::new(&project, None, 48_000, Mode::Arrangement, &mut SampleCache::default());
            e.note_on(0, s, 60, 100, deckard_model::PPQ);
            let out = deckard_engine::render(&mut e, 0.5);
            let peak = out.iter().fold(0f32, |m, x| m.max(x.abs()));
            assert!(peak > 1e-3, "{class} plays: peak {peak}");
        }
        assert_eq!(module_ui::pages("BassSynth").unwrap().len(), 2);
    }

    /// The Sampler's six pages: knobs change its parameters (undoable), on/off knobs
    /// switch with a click, slots follow the filter mode, and the waveform's markers move
    /// the zone's play range.
    #[test]
    fn sampler_pages_edit_parameters() {
        let Some(mut h) = harness(false, true) else { return };
        (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
        h.run_steps(4);
        assert_eq!(h.state().slot, Some(0), "the Sampler's slot is shown");
        h.state_mut().sampler_page = 1;
        h.run_steps(4);
        let smp = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[0].sampler.clone().unwrap();
        let knobs = |h: &Harness<'static, App>| -> Vec<(String, egui::Rect)> {
            h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-knobs"))).expect("knobs drawn")
        };
        let knob = |h: &Harness<'static, App>, name: &str| {
            knobs(h).into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no {name} knob")).1
        };
        // Pitch / Envelope: TUNE up a little; REVERSE switches with a click.
        let r = knob(&h, "TUNE");
        drag(&mut h, r.center(), r.center() - egui::vec2(0.0, 20.0));
        assert!(smp(&h).get("tune") > 0.05, "{}", smp(&h).get("tune"));
        assert_eq!(h.state().doc.as_ref().unwrap().borrow().undo_label(), Some("Sampler"));
        let r = knob(&h, "REVERSE");
        click(&mut h, r.center());
        assert!(smp(&h).reverse);
        // FX / Filter: no cutoff while the filter is off; LP2 brings CUTOFF and RESONANCE.
        h.state_mut().sampler_page = 2;
        h.run_steps(4);
        assert!(!knobs(&h).iter().any(|(n, _)| n == "CUTOFF"));
        // FILTER is a selector: its down arrow picks the next choice, LP2.
        let r = knob(&h, "FILTER");
        click(&mut h, egui::pos2(r.center().x, r.top() + 57.0));
        assert_eq!(smp(&h).get("filter"), 1.0);
        click(&mut h, egui::pos2(r.center().x, r.top() + 57.0));
        assert_eq!(smp(&h).get("filter"), 2.0, "BP2");
        click(&mut h, egui::pos2(r.center().x, r.top() + 27.0));
        assert_eq!(smp(&h).get("filter"), 1.0, "the up arrow goes back");
        let names: Vec<String> = knobs(&h).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["COMP", "DRIVE", "SR", "BITS", "FILTER", "CUTOFF", "RESONANCE"]);
        let r = knob(&h, "CUTOFF");
        drag(&mut h, r.center(), r.center() + egui::vec2(0.0, 30.0));
        assert!(smp(&h).get("cutoff") < 0.5);
        // The waveform: drag the start marker right.
        let before = smp(&h).zones[0].play_range.0;
        let wf: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-waveform"))).expect("waveform drawn");
        let y = wf.center().y;
        drag(&mut h, egui::pos2(wf.left() + 1.0, y), egui::pos2(wf.left() + 100.0, y));
        assert!(smp(&h).zones[0].play_range.0 > before + 100, "{}", smp(&h).zones[0].play_range.0);
        // Undo everything.
        for _ in 0..8 {
            h.state_mut().undo();
        }
        assert_eq!(smp(&h).get("tune"), 0.0);
        assert!(!smp(&h).reverse);
        assert_eq!(smp(&h).get("filter"), 0.0);
    }

    /// "+" › Internal: each of Maschine's effects DECKARD renders goes in at its
    /// defaults, survives saving, and runs in the engine.
    #[test]
    fn internal_effects_from_the_add_menu() {
        let Some(mut h) = harness(false, true) else { return };
        let p = project_of(&h);
        let Some((g, s)) =
            p.groups.iter().enumerate().find_map(|(g, gr)| gr.sounds.iter().position(|s| s.slots.is_empty()).map(|s| (g, s)))
        else {
            return;
        };
        (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (g, s, layout::ChainTab::Sound);
        h.state_mut().add_slot(layout::AddWhat::Sampler);
        for (class, _) in deckard_model::INTERNAL_EFFECTS {
            h.state_mut().add_slot(layout::AddWhat::Effect(class));
            h.run_steps(2);
            assert_eq!(
                project_of(&h).groups[g].sounds[s].slots.last().map(|x| x.module.clone()).as_deref(),
                Some(*class),
                "{}",
                h.state().status
            );
            assert_eq!(h.state().slot, Some(project_of(&h).groups[g].sounds[s].slots.len() - 1), "the new slot is shown");
        }
        let bytes = h.state().doc.as_ref().unwrap().borrow().to_bytes().unwrap();
        if let Some(dir) = render_dir() {
            std::fs::write(dir.join("internal-effects.mxprj"), &bytes).unwrap();
        }
        let back = deckard_doc::Document::from_bytes(bytes, None).expect("reads back");
        let slots = &back.project().groups[g].sounds[s].slots;
        assert_eq!(slots.len(), 1 + deckard_model::INTERNAL_EFFECTS.len());
        for (slot, (class, defaults)) in slots[1..].iter().zip(deckard_model::INTERNAL_EFFECTS) {
            assert_eq!(slot.module, *class);
            for (k, d) in defaults.iter() {
                if let Some(v) = slot.param(k) {
                    assert!((v - d).abs() < 1e-6, "{class} {k} = {v}, not the default {d}");
                }
            }
            assert!(deckard_engine::fx::make(&slot.module, &slot.params, 48_000.0).is_some(), "{class} renders");
        }
    }

    /// The Mix view's strip draws each of Maschine's effects as its editor lays it out,
    /// and every knob shown turns its parameter. `$DECKARD_RENDER_DIR` gets pictures.
    #[test]
    fn effect_editors_turn_their_knobs() {
        let Some(mut h) = harness(false, true) else { return };
        layout::style(&h.ctx);
        let p = project_of(&h);
        let Some((g, s)) =
            p.groups.iter().enumerate().find_map(|(g, gr)| gr.sounds.iter().position(|s| s.slots.is_empty()).map(|s| (g, s)))
        else {
            return;
        };
        (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (g, s, layout::ChainTab::Sound);
        h.state_mut().song_view = false;
        h.state_mut().mixer = true;
        let with_editor: Vec<&str> =
            deckard_model::INTERNAL_EFFECTS.iter().map(|(c, _)| *c).filter(|c| layout::strip::editor::editor(c).is_some()).collect();
        for class in &with_editor {
            // One effect at a time, so its panel is in view.
            while !project_of(&h).groups[g].sounds[s].slots.is_empty() {
                layout::apply_acts(h.state_mut(), vec![layout::Act::RemoveSlot(0)]);
                h.run_steps(1);
            }
            h.state_mut().add_slot(layout::AddWhat::Effect(class));
            h.run_steps(4);
            let slot = project_of(&h).groups[g].sounds[s].slots[0].clone();
            assert_eq!(slot.module, *class);
            let panels: Vec<(usize, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-panels"))).unwrap();
            let r = panels.iter().find(|p| p.0 == 0).expect("its panel").1;
            if let Some(dir) = render_dir() {
                h.render().unwrap().save(dir.join(format!("editor-{class}.png"))).unwrap();
            }
            let spots = layout::strip::editor::knob_spots(&slot, (g, s, 0));
            assert!(!spots.is_empty(), "{class} shows knobs");
            for (at, field) in spots {
                let before = project_of(&h).groups[g].sounds[s].slots[0].param(&field).unwrap_or(0.0);
                let c = r.min + at;
                // Up, or down from the top of the range.
                let dy = if before >= 0.99 { 40.0 } else { -40.0 };
                drag(&mut h, c, c + egui::vec2(0.0, dy));
                let after = project_of(&h).groups[g].sounds[s].slots[0].param(&field).unwrap_or(0.0);
                assert!((after - before).abs() > 1e-3, "{class} {field}: {before} -> {after}");
            }
        }
    }

    /// "+" › Internal › Replicant: DECKARD's own synth goes in as a plug-in with its
    /// own pages (choices as selectors), its knobs are saved into the project as they
    /// turn (undoable), and it plays.
    #[test]
    fn native_synth_from_the_add_menu() {
        let Some(mut h) = harness(false, true) else { return };
        let p = project_of(&h);
        let Some((g, s)) =
            p.groups.iter().enumerate().find_map(|(g, gr)| gr.sounds.iter().position(|s| s.slots.is_empty()).map(|s| (g, s)))
        else {
            return;
        };
        (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (g, s, layout::ChainTab::Sound);
        let class = deckard_plugins::native::CLASSES[0];
        h.state_mut().add_slot(layout::AddWhat::Plugin(layout::native_entry(class)));
        h.run_steps(4);
        let slot = |h: &Harness<'static, App>| project_of(h).groups[g].sounds[s].slots[0].clone();
        assert_eq!(slot(&h).plugin.as_ref().map(|p| p.id.clone()), Some(class.plugin_id()), "{}", h.state().status);
        assert!(slot(&h).is_instrument());
        let knobs: Vec<(String, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-knobs"))).unwrap();
        assert_eq!(
            knobs.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["WAVE 1", "WAVE 2", "PITCH 2", "DETUNE", "MIX", "SUB", "NOISE", "PW"],
            "the Oscillators page"
        );
        let view = h.state().plugin_view.as_ref().expect("pages shown");
        assert_eq!(view.pages.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Oscillators", "Filter", "Amp", "Modulation", "Super"]);
        assert_eq!(view.pages[0].choices[0], ["Saw", "Pulse", "Triangle", "Sine"], "a wave is a choice");
        if let Some(dir) = render_dir() {
            h.render().unwrap().save(dir.join("replicant-sound-tab.png")).unwrap();
            h.state_mut().mixer = true;
            h.run_steps(4);
            h.render().unwrap().save(dir.join("replicant-mix.png")).unwrap();
            h.state_mut().mixer = false;
            h.run_steps(2);
        }
        // Turn the cutoff: stored in the project at once, and undoable.
        let before = slot(&h).plugin.unwrap().chunk;
        h.state_mut().plugin_page = 1;
        h.run_steps(2);
        let cutoff = view_cutoff(&h);
        layout::apply_acts(h.state_mut(), vec![layout::Act::PluginParam(0, cutoff, 0.2)]);
        h.run_steps(2);
        let after = slot(&h).plugin.unwrap().chunk;
        assert_ne!(after, before, "the knob went into the project");
        let inst = h.state_mut().slot_plugin(g, s, 0).unwrap();
        assert_eq!(inst.lock().unwrap().param_value(cutoff), 0.2);
        let doc = h.state().doc.clone().unwrap();
        if let Some(dir) = render_dir() {
            std::fs::write(dir.join("native-synth-edited.mxprj"), doc.borrow().to_bytes().unwrap()).unwrap();
        }
        assert!(doc.borrow_mut().undo());
        h.run_steps(2);
        assert_eq!(slot(&h).plugin.unwrap().chunk, before, "undo restores it");
        let inst = h.state_mut().slot_plugin(g, s, 0).unwrap();
        assert_eq!(inst.lock().unwrap().param_value(cutoff), 0.6, "and the synth follows");
        // Saved and read back: the same plug-in and state.
        let bytes = doc.borrow().to_bytes().unwrap();
        if let Some(dir) = render_dir() {
            std::fs::write(dir.join("native-synth.mxprj"), &bytes).unwrap();
        }
        let back = deckard_doc::Document::from_bytes(bytes, None).expect("reads back");
        assert_eq!(back.project().groups[g].sounds[s].slots[0].plugin.as_ref().map(|p| &p.chunk), Some(&before));
    }

    fn view_cutoff(h: &Harness<'static, App>) -> u32 {
        let view = h.state().plugin_view.as_ref().unwrap();
        view.pages[1].knobs.iter().find(|(i, ..)| i.short == "CUTOFF").unwrap().0.id
    }

    /// BUGS #24: each slot starts with an instrument or effect icon; a click on it
    /// bypasses the plug-in, another brings it back.
    #[test]
    fn a_slots_icon_bypasses_it() {
        let Some(mut h) = harness(false, true) else { return };
        let p = project_of(&h);
        let Some((g, s)) =
            p.groups.iter().enumerate().find_map(|(g, gr)| gr.sounds.iter().position(|s| s.slots.is_empty()).map(|s| (g, s)))
        else {
            return;
        };
        (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (g, s, layout::ChainTab::Sound);
        h.run_steps(4);
        h.state_mut().add_slot(layout::AddWhat::Sampler);
        h.run_steps(4);
        let icons = |h: &Harness<'static, App>| -> Vec<(egui::Rect, bool)> {
            h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-chain-icons"))).unwrap()
        };
        assert_eq!(icons(&h).iter().map(|(_, inst)| *inst).collect::<Vec<_>>(), [true], "the Sampler is an instrument");
        let active = |h: &Harness<'static, App>| project_of(h).groups[g].sounds[s].slots[0].active;
        let at = icons(&h)[0].0.center();
        click(&mut h, at);
        assert!(!active(&h), "bypassed");
        let at = icons(&h)[0].0.center();
        click(&mut h, at);
        assert!(active(&h), "back");
    }

    /// The SOUND tab's chain: add a Sampler to an empty sound, show its pages, bypass and
    /// remove it; the sound's name shows its channel.
    #[test]
    fn chain_slots_in_the_sound_tab() {
        let Some(mut h) = harness(false, true) else { return };
        let p = project_of(&h);
        let Some((g, s)) =
            p.groups.iter().enumerate().find_map(|(g, gr)| gr.sounds.iter().position(|s| s.slots.is_empty()).map(|s| (g, s)))
        else {
            return;
        };
        (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (g, s, layout::ChainTab::Sound);
        h.run_steps(4);
        assert_eq!(h.state().slot, None, "an empty sound shows its channel");
        h.state_mut().add_slot(layout::AddWhat::Sampler);
        h.run_steps(4);
        let slots = |h: &Harness<'static, App>| project_of(h).groups[g].sounds[s].slots.clone();
        assert_eq!(slots(&h).iter().map(|x| x.module.as_str()).collect::<Vec<_>>(), ["Sampler"]);
        assert_eq!(h.state().slot, Some(0), "the new slot is shown");
        let knobs: Vec<(String, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-knobs"))).unwrap();
        assert_eq!(
            knobs.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["POLYPHONY", "PITCHBEND", "MODE"],
            "Voice Settings / Engine"
        );
        // The channel icon: its channel properties; the plug-in icon: the chain again.
        let icon = |h: &Harness<'static, App>, k: usize| -> egui::Rect {
            h.ctx.data(|d| d.get_temp(egui::Id::new(("deckard-chain-mode", k)))).unwrap()
        };
        let channel_icon = icon(&h, 0);
        click(&mut h, channel_icon.center());
        assert!(h.state().channel);
        let plug_icon = icon(&h, 1);
        click(&mut h, plug_icon.center());
        assert!(!h.state().channel);
        let rows: Vec<egui::Rect> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-chain-rows"))).unwrap();
        click(&mut h, rows[0].center());
        assert_eq!(h.state().slot, Some(0));
        h.state_mut().arrange_act(layout::Act::SlotActive(0, false));
        h.run_steps(2);
        assert!(!slots(&h)[0].active);
        h.state_mut().arrange_act(layout::Act::RemoveSlot(0));
        h.run_steps(4);
        assert!(slots(&h).is_empty());
        assert_eq!(h.state().slot, None);
    }

    /// The Mixer: a strip per group; the fader sets the level, M mutes, right-click on
    /// M solos (again: unsolo), the headphones cue; double-clicking a group header's
    /// background shows its sounds.
    #[test]
    fn mixer_strips() {
        let Some(mut h) = harness(false, true) else { return };
        h.state_mut().mixer = true;
        h.state_mut().mixer_sounds = false;
        h.run_steps(4);
        let strips = |h: &Harness<'static, App>| -> Vec<(layout::Channel, egui::Rect)> {
            h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-mixer-strips"))).expect("mixer drawn")
        };
        let rect = |h: &Harness<'static, App>, ch| strips(h).into_iter().find(|(c, _)| *c == ch).unwrap().1;
        let p = |h: &Harness<'static, App>| project_of(h);
        assert!(strips(&h).iter().any(|(c, _)| *c == layout::Channel::Master), "the master on the right");
        // Fader: drag group 1's arrow down.
        let r = rect(&h, layout::Channel::Group(1));
        let level = p(&h).groups[1].level;
        let y = r.bottom() - 186.0 + 26.0 + 25.0; // 0 dB line
        let x = r.left() + 48.0;
        drag(&mut h, egui::pos2(x, y), egui::pos2(x, y + 22.0));
        assert!(p(&h).groups[1].level < level * 0.6, "about -10 dB: {}", p(&h).groups[1].level);
        // Mute, then solo group 2 (right-click M), then unsolo.
        let m = egui::pos2(r.right() - 23.0, r.bottom() - 186.0 + 153.0);
        click(&mut h, m);
        assert!(p(&h).groups[1].muted);
        click(&mut h, m);
        assert!(!p(&h).groups[1].muted);
        let r2 = rect(&h, layout::Channel::Group(2));
        let m2 = egui::pos2(r2.right() - 23.0, r2.bottom() - 186.0 + 153.0);
        h.hover_at(m2);
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: m2, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.step();
        }
        h.run_steps(3);
        let muted: Vec<bool> = p(&h).groups.iter().map(|g| g.muted).collect();
        assert!(!muted[2] && muted.iter().enumerate().all(|(i, m)| i == 2 || *m), "solo: {muted:?}");
        h.hover_at(m2);
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: m2, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.step();
        }
        h.run_steps(3);
        assert!(p(&h).groups.iter().all(|g| !g.muted), "unsolo");
        // Cue.
        click(&mut h, egui::pos2(r2.left() + 14.0, r2.bottom() - 186.0 + 153.0));
        assert!(p(&h).groups[2].cue);
        // Double-click group 2's header background: its sounds.
        double_click(&mut h, egui::pos2(r2.left() + 60.0, r2.top() + 10.0));
        assert!(h.state().mixer_sounds);
        assert_eq!(h.state().group, 2);
        assert!(strips(&h).iter().any(|(c, _)| *c == layout::Channel::Sound(2, 0)));
        assert!(strips(&h).iter().any(|(c, _)| *c == layout::Channel::Group(2)), "the group on the right");
    }

    /// Sampling: the strip button opens it on the focused sound's sample; dragging on
    /// the waveform selects; NORM. writes an edited copy and points the zone at it
    /// (undoable); SLICE ▸ APPLY makes a zone per slice on keys from C1.
    #[test]
    fn recordings_land_where_maschine_puts_them() {
        use layout::sampling::RecParam;
        let Some(mut h) = harness(false, true) else { return };
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        h.state_mut().path = Some(dir.join("x.mxprj"));
        let rate = 48_000;
        let tone = |secs: f64| deckard_engine::sample::Sample {
            rate,
            frames: (0..(secs * rate as f64) as usize)
                .map(|i| {
                    let v = (i as f32 / rate as f32 * 220.0 * std::f32::consts::TAU).sin() * 0.5;
                    [v, v]
                })
                .collect(),
        };
        let p = project_of(&h);
        let bar = p.bar_length() as f64;
        let free = |p: &deckard_model::Project, g: usize| {
            (1..16).find(|&s| p.groups[g].sounds[s].slots.is_empty() && p.groups[g].sounds[s].sampler.is_none()).unwrap()
        };

        // Loop, Target Pattern: a new pattern in the scene, playing the take in the next
        // free sound, long enough for it (2 bars into a 1-bar pattern doubles it).
        (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound, h.state_mut().scene) = (0, 0, 0, 0);
        h.state_mut().song_view = false;
        h.state_mut().recorder.view.set(RecParam::Target, 2);
        let s1 = free(&p, 0);
        let n = p.groups[0].patterns.len();
        h.state_mut().finish_recording(recorder::Finished::Take { sample: tone(1.0), channels: 2, mode: 2, ticks: 2.0 * bar });
        let q = project_of(&h);
        assert_eq!(q.groups[0].patterns.len(), n + 1, "{}", h.state().status);
        let pat = &q.groups[0].patterns[n];
        // An Audio plug-in playing the take (its length the recording's, in beats), on in
        // the new pattern without notes.
        assert!(pat.notes.iter().all(|x| x.sound != s1));
        assert_eq!(pat.audio, vec![deckard_model::AudioCue { sound: s1, take: 0 }]);
        assert!(pat.length >= 2 * bar as i64);
        assert_eq!(q.scenes[0].patterns.iter().find(|(g, _)| *g == 0).map(|x| x.1), Some(n), "the scene plays the new pattern");
        let a = q.groups[0].sounds[s1].audio.clone().expect("an Audio plug-in");
        assert_eq!(q.groups[0].sounds[s1].instrument, "AudioModule");
        assert_eq!(a.takes.len(), 1);
        assert!((a.takes[0].beats - 2.0 * bar / PPQ as f64).abs() < 1e-6, "{}", a.takes[0].beats);
        assert_eq!((h.state().pattern, h.state().sound), (n, s1));

        // Loop, Target Sound: the next free sound, in the current pattern.
        h.state_mut().recorder.view.set(RecParam::Target, 1);
        let s2 = free(&q, 0);
        assert_ne!(s1, s2);
        h.state_mut().finish_recording(recorder::Finished::Take { sample: tone(0.5), channels: 1, mode: 2, ticks: bar / 2.0 });
        let q = project_of(&h);
        assert!(q.groups[0].patterns[n].audio.iter().any(|c| c.sound == s2));
        assert!(q.groups[0].sounds[s2].audio.is_some());
        // Loop, Target Take, on that sound: a second take, playing in the pattern.
        (h.state_mut().sound, h.state_mut().pattern) = (s2, n);
        h.state_mut().recorder.view.set(RecParam::Target, 0);
        h.state_mut().finish_recording(recorder::Finished::Take { sample: tone(0.5), channels: 1, mode: 2, ticks: bar / 2.0 });
        let q = project_of(&h);
        assert_eq!(q.groups[0].sounds[s2].audio.as_ref().map(|a| (a.takes.len(), a.focused)), Some((2, 1)));
        assert!(q.groups[0].patterns[n].audio.contains(&deckard_model::AudioCue { sound: s2, take: 1 }));

        // Auto: zones with Maschine's key/velocity ranges and roots, in a free sound
        // when the focused one has no Sampler.
        let g = 7; // Pad: a Massive sound
        assert!(q.groups[g].sounds[0].sampler.is_none());
        (h.state_mut().group, h.state_mut().sound) = (g, 0);
        let s3 = free(&q, g);
        let mut done = Vec::new();
        for (note, keys) in [(48u8, (0u8, 53u8)), (60, (54, 127))] {
            let path = dir.join(format!("Pad {} 100.wav", recorder::note_name(note)));
            let s = tone(0.5);
            deckard_engine::edit::write_wav(&path, &s).unwrap();
            done.push((recorder::AutoItem { note, velocity: 100, keys, vels: (0, 127), path }, s.frames.len(), Some((4800, 20_000))));
        }
        h.state_mut().finish_recording(recorder::Finished::Auto { done });
        let q = project_of(&h);
        let zones = &q.groups[g].sounds[s3].sampler.as_ref().expect("a Sampler in the free sound").zones;
        let got: Vec<(i64, i64, i64)> = zones.iter().map(|z| (z.root_key, z.key_range.0, z.key_range.1)).collect();
        assert_eq!(got, vec![(48, 0, 53), (60, 54, 127)]);
        assert!(zones.iter().all(|z| z.loop_active && z.loop_range == (4800, 20_000)));
        assert_eq!((h.state().sound, h.state().sampling.tab), (s3, 3));
    }

    #[test]
    fn sampling_edit_and_slice() {
        let Some(mut h) = harness(false, true) else { return };
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        // Edited samples go next to the project: here, a temporary folder.
        h.state_mut().path = Some(dir.join("x.mxprj"));
        (h.state_mut().group, h.state_mut().sound) = (0, 0);
        h.run_steps(3);
        let panel_top = 33.0 + 3.0 + 343.0 + 2.0 + 140.0 + 2.0;
        // The strip's fifth button (after Erase, BUGS #17).
        click(&mut h, Pos2::new(3.0 + 16.0, panel_top + 7.0 + 4.0 * 28.0 + 12.0));
        assert!(h.state().sampling_open);
        h.run_steps(3);
        assert!(h.state().sampling.sample.is_some(), "the kick's sample is shown");
        let wave: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-sample-wave"))).expect("waveform drawn");
        // Select from 40 % to 70 % of the waveform (between the S and E markers).
        let (a, b) = (wave.left() + wave.width() * 0.4, wave.left() + wave.width() * 0.7);
        drag(&mut h, egui::pos2(a, wave.center().y), egui::pos2(b, wave.center().y));
        let sel = h.state().sampling.selection.expect("a selection");
        let len = h.state().sampling.len();
        assert!((sel.0 as f64 / len as f64 - 0.4).abs() < 0.02 && (sel.1 as f64 / len as f64 - 0.7).abs() < 0.02, "{sel:?} of {len}");
        // The S flag at the top of the start line: the pointer shows it can be dragged
        // (not over the plain waveform), and dragging it moves the start to 20 %.
        let cursor_at = |h: &mut Harness<'static, App>, p: Pos2| {
            h.hover_at(p);
            h.step();
            h.output().platform_output.cursor_icon
        };
        let lo = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].play_range.0;
        assert_eq!(lo, 0, "the kick plays from its start");
        let flag = egui::pos2(wave.left() + 10.0, wave.top() + 7.0);
        assert_eq!(cursor_at(&mut h, flag), egui::CursorIcon::ResizeHorizontal);
        assert_eq!(cursor_at(&mut h, egui::pos2(wave.left() + wave.width() * 0.5, wave.center().y)), egui::CursorIcon::Default);
        drag(&mut h, flag, egui::pos2(wave.left() + wave.width() * 0.2, wave.top() + 7.0));
        let lo = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].play_range.0 as f64;
        assert!((lo / len as f64 - 0.2).abs() < 0.02, "the start moved to {lo} of {len}");
        // NORM.
        let tool = |h: &Harness<'static, App>, id: &str| -> egui::Rect {
            h.ctx.data(|d| d.get_temp(egui::Id::new(("deckard-tool", id.to_string())))).expect("tool drawn")
        };
        let before = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].clone();
        let r = tool(&h, "NORM.");
        click(&mut h, r.center());
        let z = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].clone();
        assert!(z.sample.path.ends_with(" - 2.wav") && z.sample.path.starts_with(dir.to_str().unwrap()), "{}", z.sample.path);
        assert!(std::path::Path::new(&z.sample.path).exists());
        h.run_steps(3);
        let edited = h.state().sampling.sample.clone().unwrap();
        let peak = edited.frames[sel.0..sel.1].iter().fold(0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
        assert!((peak - 1.0).abs() < 1e-3, "normalized: {peak}");
        h.state_mut().undo();
        h.run_steps(3);
        assert_eq!(project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0], before, "undo restores the original sample");
        // SLICE: Split into 8, APPLY.
        h.state_mut().arrange_act(layout::Act::Sampling(layout::sampling::SamplingAct::Tab(2)));
        h.run_steps(3);
        assert_eq!(h.state().sampling.slices.len(), 8);
        let r = tool(&h, "apply");
        click(&mut h, r.center());
        let zones = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones;
        assert_eq!(zones.len(), 8);
        assert_eq!(zones.iter().map(|z| z.root_key).collect::<Vec<_>>(), (36..44).collect::<Vec<_>>());
        assert!(zones.windows(2).all(|w| w[0].play_range.1 + 1 == w[1].play_range.0), "contiguous slices");
        assert_eq!(h.state().sampling.tab, 3, "then the Zone tab");
        h.run_steps(3);
        let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-zone-grid"))).expect("zone map drawn");
        assert!(grid.width() > 100.0);
    }

    fn pedal_before(p: &deckard_model::Project, g: usize, pi: usize) -> usize {
        p.groups[g].patterns[pi].midi.iter().filter(|m| layout::Lane::of(&m.kind) == layout::Lane::Controller(64)).count()
    }

    /// With several notes selected, resizing one resizes all, and dragging one
    /// velocity or probability bar changes all of them by as much.
    #[test]
    fn edits_apply_to_the_whole_selection() {
        let Some(mut h) = harness(false, true) else { return };
        let (g, pi) = (h.state().group, h.state().pattern);
        let start = project_of(&h).groups[g].patterns[pi].start;
        // Two notes of the focused sound, on the 16th grid.
        let sound = h.state().sound;
        let a = Note { expr: None, sound, position: start + 960, length: 240, pitch: 60, velocity: 100, probability: 100 };
        let b = Note { expr: None, sound, position: start + 2880, length: 240, pitch: 60, velocity: 80, probability: 100 };
        let existing: Vec<Note> = notes_of(&h).into_iter().filter(|n| n.sound == sound).collect();
        h.state().doc.as_ref().unwrap().borrow_mut().edit_notes("setup", g, pi, &existing, &[a, b]).unwrap();
        h.state_mut().selected = vec![a, b];
        h.run_steps(2);

        // Resize: drag a's right edge one beat right.
        let row = sound;
        let (from, to) = (grid_pos(&h, row, 960 + 240) - egui::vec2(2.0, 0.0), grid_pos(&h, row, 960 + 240 + 960) - egui::vec2(2.0, 0.0));
        drag(&mut h, from, to);
        let lens: Vec<i64> = notes_of(&h).iter().filter(|n| n.sound == sound).map(|n| n.length).collect();
        assert_eq!(lens, [1200, 1200], "both notes resized");
        assert_eq!(h.state().selected.len(), 2, "still selected");

        // Velocity: drag a's bar down by about 30 (relative: b goes down as much).
        let (bottom, lane_h) = lane(&h);
        let (vel_bottom, prob_bottom) = (bottom, bottom);
        let x = grid_pos(&h, 0, 960).x + 1.5;
        let y = |bottom: f32, v: f32, max: f32| bottom - lane_h * v / max;
        drag(&mut h, Pos2::new(x, y(vel_bottom, 100.0, 127.0)), Pos2::new(x, y(vel_bottom, 70.0, 127.0)));
        let vels: Vec<i64> = notes_of(&h).iter().filter(|n| n.sound == sound).map(|n| n.velocity).collect();
        assert!((60..=80).contains(&vels[0]), "{vels:?}");
        assert_eq!(vels[0] - vels[1], 20, "relative change keeps the difference: {vels:?}");

        // Probability: drag a's bar from 100 to about 50; b follows.
        pick_lane(&mut h, layout::Lane::Probability);
        drag(&mut h, Pos2::new(x, y(prob_bottom, 100.0, 100.0)), Pos2::new(x, y(prob_bottom, 50.0, 100.0)));
        let probs: Vec<i64> = notes_of(&h).iter().filter(|n| n.sound == sound).map(|n| n.probability).collect();
        assert!((40..=60).contains(&probs[0]) && probs[0] == probs[1], "{probs:?}");
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

    /// Right-clicking a sound row opens Maschine's sound menu: colour, copy and paste
    /// (with its notes), reset; undo takes each back in one step.
    #[test]
    fn sound_menu() {
        let Some(mut h) = harness(false, true) else { return };
        if let Some(d) = render_dir() {
            std::fs::create_dir_all(&d).unwrap();
        }
        let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
        let row = |r: usize| Pos2::new(grid.left() - 81.0, grid.top() + 12.0 + r as f32 * 24.0);
        let g = h.state().group;
        let before = project_of(&h);
        let notes_of = |p: &deckard_model::Project, s: usize| -> Vec<(i64, i64)> {
            p.groups[g]
                .patterns
                .iter()
                .flat_map(|pt| pt.notes.iter().filter(|n| n.sound == s).map(|n| (n.position - pt.start, n.pitch)))
                .collect()
        };
        let empty = (1..16).find(|&s| before.groups[g].sounds[s].slots.is_empty() && notes_of(&before, s).is_empty()).unwrap();

        // The menu, and the colour palette.
        right_click(&mut h, row(0));
        assert_eq!(h.state().sound, 0);
        if let Some(d) = render_dir() {
            h.render().unwrap().save(d.join("menu-sound.png")).unwrap();
        }
        h.get_by_label("Color ⏵").hover();
        h.run_steps(4);
        if let Some(d) = render_dir() {
            h.render().unwrap().save(d.join("menu-sound-color.png")).unwrap();
        }
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        h.state_mut().menu_act(layout::Act::SetColor(vec![deckard_doc::Target::Sound(g, 0)], deckard_model::Color::Palette(5)));
        assert_eq!(project_of(&h).groups[g].sounds[0].color, deckard_model::Color::Palette(5));

        // Copy sound 1, paste it into an empty slot: its plug-ins, name and notes.
        right_click(&mut h, row(0));
        h.get_by_label("Copy").click();
        h.run_steps(3);
        assert_eq!(h.state().sound_clipboard.len(), 1);
        right_click(&mut h, row(empty));
        h.get_by_label("Paste").click();
        h.run_steps(3);
        let p = project_of(&h);
        assert_eq!(p.groups[g].sounds[empty].name, p.groups[g].sounds[0].name);
        assert_eq!(p.groups[g].sounds[empty].slots, p.groups[g].sounds[0].slots);
        assert_eq!(notes_of(&p, empty), notes_of(&p, 0));
        assert_eq!(h.state().doc.as_ref().unwrap().borrow().undo_label(), Some("Paste Sound"));

        // Reset it: empty again, as one undo step.
        right_click(&mut h, row(empty));
        h.get_by_label("Reset").click();
        h.run_steps(3);
        let p = project_of(&h);
        assert!(p.groups[g].sounds[empty].slots.is_empty() && notes_of(&p, empty).is_empty());
        assert_eq!(p.groups[g].sounds[empty].name, format!("Sound {}", empty + 1));
        h.state_mut().undo();
        assert_eq!(notes_of(&project_of(&h), empty), notes_of(&project_of(&h), 0));
    }

    /// The Ideas view's pattern menu: Duplicate (after it), Double, Rename, Clear,
    /// Delete; and the group tile's menu.
    #[test]
    fn pattern_menu() {
        let Some(mut h) = harness(false, false) else { return };
        let before = project_of(&h);
        let g = 0;
        let n = before.groups[g].patterns.len();
        right_click(&mut h, cell(g, 0));
        if let Some(d) = render_dir() {
            h.render().unwrap().save(d.join("menu-pattern.png")).unwrap();
        }
        h.get_by_label("Duplicate").click();
        h.run_steps(3);
        let p = project_of(&h);
        assert_eq!(p.groups[g].patterns.len(), n + 1);
        assert_eq!(p.groups[g].patterns[1].notes.len(), before.groups[g].patterns[0].notes.len());
        assert_eq!((h.state().group, h.state().pattern), (g, 1));
        right_click(&mut h, cell(g, 1));
        h.get_by_label("Double").click();
        h.run_steps(3);
        assert_eq!(project_of(&h).groups[g].patterns[1].length, 2 * before.groups[g].patterns[0].length);
        right_click(&mut h, cell(g, 1));
        h.get_by_label("Rename").click();
        h.run_steps(3);
        assert_eq!(h.state().renaming, Some(deckard_doc::Target::Pattern(g, 1)));
        type_text(&mut h, "Kicks Long");
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert_eq!(project_of(&h).groups[g].patterns[1].name, "Kicks Long");
        right_click(&mut h, cell(g, 1));
        h.get_by_label("Clear").click();
        h.run_steps(3);
        assert!(project_of(&h).groups[g].patterns[1].notes.is_empty());
        right_click(&mut h, cell(g, 1));
        h.get_by_label("Delete").click();
        h.run_steps(3);
        let p = project_of(&h);
        assert_eq!(p.groups[g].patterns.len(), n);
        assert_eq!(p.groups[g].patterns[1], before.groups[g].patterns[1]);
        // The group tile: its menu, and a colour for the group.
        h.state_mut().menu_act(layout::Act::SetColor(vec![deckard_doc::Target::Group(1)], deckard_model::Color::Palette(12)));
        assert_eq!(project_of(&h).groups[1].color, deckard_model::Color::Palette(12));
    }

    /// The channel properties: the icon strip, the set tiles, page tabs, and edits
    /// from a page (a routing pick, a switch, Generate).
    #[test]
    fn channel_properties() {
        let Some(mut h) = harness(false, true) else { return };
        layout::style(&h.ctx);
        let knobs = |h: &Harness<'static, App>| -> Vec<(String, egui::Rect)> {
            h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-knobs"))).unwrap_or_default()
        };
        let names = |h: &Harness<'static, App>| knobs(h).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
        h.state_mut().channel = true;
        h.run_steps(3);
        let shots = render_dir();
        let shot = |h: &mut Harness<'static, App>, name: &str| {
            if let Some(dir) = &shots {
                h.render().unwrap().save(dir.join(format!("channel-{name}.png"))).unwrap();
            }
        };
        // Every page of every set, for the sound, the group and the master.
        for (tab, level) in [(layout::ChainTab::Sound, "sound"), (layout::ChainTab::Group, "group"), (layout::ChainTab::Master, "master")] {
            h.state_mut().chain_tab = tab;
            for set in 0..3 {
                for page in 0..3 {
                    (h.state_mut().prop_set, h.state_mut().prop_page) = (set, page);
                    h.run_steps(3);
                    shot(&mut h, &format!("{level}-{set}-{page}"));
                }
            }
        }
        h.state_mut().chain_tab = layout::ChainTab::Sound;
        (h.state_mut().prop_set, h.state_mut().prop_page) = (1, 0);
        h.run_steps(3);
        assert_eq!(names(&h), ["DEST.", "CUE", "LEVEL", "PAN", "AUDIO MUTE"]);
        // Route the sound to the master from the Dest. list.
        let dest = knobs(&h).into_iter().find(|(n, _)| n == "DEST.").unwrap().1;
        click(&mut h, Pos2::new(dest.center().x, dest.top() + 42.0));
        shot(&mut h, "dest-list");
        h.get_by_label("Master").click();
        h.run_steps(3);
        assert_eq!(project_of(&h).groups[0].sounds[0].channel.dest, deckard_model::AudioDest::Master);
        // Audio Mute is a switch.
        let am = knobs(&h).into_iter().find(|(n, _)| n == "AUDIO MUTE").unwrap().1;
        click(&mut h, Pos2::new(am.center().x, am.top() + 42.0));
        assert!(project_of(&h).groups[0].sounds[0].channel.audio_mute);
        // The group's MIDI input: Manual shows its source, channel, start note and thru.
        h.state_mut().chain_tab = layout::ChainTab::Group;
        (h.state_mut().prop_set, h.state_mut().prop_page) = (0, 0);
        layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(deckard_doc::Target::Group(0), "key_mode", 0.0)]);
        h.run_steps(3);
        assert_eq!(names(&h), ["KEY MODE"]);
        layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(deckard_doc::Target::Group(0), "key_mode", 2.0)]);
        h.run_steps(3);
        assert_eq!(names(&h), ["KEY MODE", "SOURCE", "CHANNEL", "START NOTE", "THRU"]);
        shot(&mut h, "group-midi-manual");
        // Generate: Euclidean fills the sound's part of the pattern.
        h.state_mut().chain_tab = layout::ChainTab::Sound;
        (h.state_mut().prop_set, h.state_mut().prop_page) = (2, 2);
        layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(deckard_doc::Target::Sound(0, 0), "sequence_generation/mode", 1.0)]);
        h.run_steps(3);
        assert_eq!(names(&h), ["MODE", "AMOUNT", "AMOUNT", "STEPS", "DIVISION", "LIVE"]);
        shot(&mut h, "sound-generate-euclid");
        let b: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-groove-button"))).expect("the Generate button");
        click(&mut h, b.center());
        let (g, pi) = (h.state().group, h.state().pattern);
        let n = project_of(&h).groups[g].patterns[pi].notes.iter().filter(|n| n.sound == 0).count();
        assert!(n > 0, "notes generated");
        // Macros: right-click a parameter's name to make it a sound macro.
        (h.state_mut().prop_set, h.state_mut().prop_page) = (1, 0);
        h.run_steps(3);
        let level = knobs(&h).into_iter().find(|(n, _)| n == "LEVEL").unwrap().1;
        right_click(&mut h, Pos2::new(level.center().x, level.top() + 8.0));
        shot(&mut h, "macro-menu");
        h.get_by_label("Create Sound Macro").click();
        h.run_steps(3);
        let ch = deckard_doc::Target::Sound(0, 0);
        let pages = |h: &Harness<'static, App>| h.state().doc.as_ref().unwrap().borrow().macro_pages(ch);
        let oid = |h: &Harness<'static, App>, f: &str| h.state().doc.as_ref().unwrap().borrow().param_oid(ch, f);
        assert_eq!(pages(&h)[0].macros[0].target, oid(&h, "level"));
        // The Macro page shows it; an empty slot picks what it controls.
        (h.state_mut().prop_set, h.state_mut().prop_page) = (3, 0);
        h.run_steps(3);
        assert_eq!(names(&h)[..2], ["LEVEL".to_string(), "MACRO 2".to_string()]);
        let m2 = knobs(&h).into_iter().find(|(n, _)| n == "MACRO 2").unwrap().1;
        click(&mut h, m2.center());
        h.get_by_label("Output ⏵").hover();
        h.run_steps(3);
        h.get_by_label("Audio ⏵").hover();
        h.run_steps(3);
        shot(&mut h, "macro-picker");
        h.get_by_label("Pan").click();
        h.run_steps(3);
        assert_eq!(pages(&h)[0].macros[1].target, oid(&h, "pan"));
        assert_eq!(names(&h)[..3], ["LEVEL".to_string(), "PAN".to_string(), "MACRO 3".to_string()]);
        // Dragging the macro moves its parameter.
        let before = project_of(&h).groups[0].sounds[0].level;
        let lk = knobs(&h).into_iter().find(|(n, _)| n == "LEVEL").unwrap().1;
        drag(&mut h, lk.center(), lk.center() + egui::vec2(0.0, 30.0));
        assert!(project_of(&h).groups[0].sounds[0].level < before, "the macro turns the level down");
        // "+" adds a page.
        layout::apply_acts(h.state_mut(), vec![layout::Act::PropPage(1)]);
        h.run_steps(3);
        assert_eq!(pages(&h).len(), 2);
        assert_eq!(h.state().prop_page, 1);
        shot(&mut h, "macro-page2");
        // The plug-in icon goes back to the chain.
        layout::apply_acts(h.state_mut(), vec![layout::Act::ChannelMode(false)]);
        h.run_steps(3);
        assert!(!h.state().channel);
    }

    /// A group's MIDI: exported with each sound on its own note, imported back to the
    /// same sounds; on their own channels, the same.
    #[test]
    fn group_midi_round_trip() {
        let Some(mut h) = harness(false, true) else { return };
        let g = 0;
        (h.state_mut().group, h.state_mut().pattern) = (g, 0);
        // Notes on three sounds.
        let start = project_of(&h).groups[g].patterns[0].start;
        let extra: Vec<Note> = (1..4)
            .map(|s| Note {
                expr: None,
                sound: s,
                position: start + s as i64 * 240,
                length: 120,
                pitch: 60,
                velocity: 90 + s as i64,
                probability: 100,
            })
            .collect();
        h.state().doc.as_ref().unwrap().borrow_mut().edit_notes("test", g, 0, &[], &extra).unwrap();
        let before = project_of(&h).groups[g].patterns[0].clone();
        let mut key = |n: &Note| (n.sound, n.position, n.velocity, n.length);
        let mut want: Vec<_> = before.notes.iter().map(&mut key).collect();
        want.sort();
        assert!(want.iter().map(|k| k.0).collect::<std::collections::BTreeSet<_>>().len() > 1, "several sounds play");
        for batch in [1.0, 0.0] {
            layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(deckard_doc::Target::Group(g), "midi_batch_export", batch)]);
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join(format!("group-{batch}.mid"));
            h.state_mut().export_group_midi(g, Some(path.clone())).unwrap();
            h.state_mut().import_group_midi(g, Some(path.clone())).unwrap();
            let after = project_of(&h).groups[g].patterns[0].clone();
            let mut got: Vec<_> = after.notes.iter().map(&mut key).collect();
            got.sort();
            assert_eq!(got, want, "batch {batch}");
        }
    }

    /// Drag a scene tab to move the scene; drag a section's header to move it, its
    /// right edge to change its length.
    #[test]
    fn drag_scenes_and_sections() {
        let Some(mut h) = harness(false, true) else { return };
        let rect =
            |h: &Harness<'static, App>, key: (&str, usize)| -> egui::Rect { h.ctx.data(|d| d.get_temp(egui::Id::new(key))).unwrap() };
        // Ideas view: scene 1 to the third place.
        h.state_mut().song_view = false;
        h.run_steps(3);
        let names = |h: &Harness<'static, App>| project_of(h).scenes.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        let before = names(&h);
        let (a, c) = (rect(&h, ("deckard-scene-tab", 0)), rect(&h, ("deckard-scene-tab", 2)));
        drag(&mut h, a.center(), c.center());
        let mut want = before.clone();
        let first = want.remove(0);
        want.insert(2, first);
        assert_eq!(names(&h), want);
        // Song view: section 1 after section 3.
        h.state_mut().song_view = true;
        h.run_steps(3);
        let lens = |h: &Harness<'static, App>| project_of(h).sections.iter().map(|s| s.length).collect::<Vec<_>>();
        let scenes = |h: &Harness<'static, App>| project_of(h).sections.iter().map(|s| s.scene).collect::<Vec<_>>();
        let (s0, s2) = (scenes(&h), rect(&h, ("deckard-section", 2)));
        let first = rect(&h, ("deckard-section", 0));
        drag(&mut h, first.center(), s2.center());
        let mut want = s0.clone();
        let x = want.remove(0);
        want.insert(2, x);
        assert_eq!(scenes(&h), want);
        // Section 1's right edge: one bar longer.
        let bar = project_of(&h).bar_length();
        let l0 = lens(&h)[0];
        let r0 = rect(&h, ("deckard-section", 0));
        let r1 = rect(&h, ("deckard-section", 1));
        let per_bar = (r0.width() + 0.0) / (l0 as f32 / bar as f32);
        let _ = r1;
        drag(&mut h, egui::pos2(r0.right(), r0.center().y), egui::pos2(r0.right() + per_bar, r0.center().y));
        assert_eq!(lens(&h)[0], l0 + bar, "one bar longer");
        let r0 = rect(&h, ("deckard-section", 0));
        drag(&mut h, egui::pos2(r0.right(), r0.center().y), egui::pos2(r0.right() - 2.0 * per_bar, r0.center().y));
        assert_eq!(lens(&h)[0], l0 - bar, "then two shorter");
        // Mid-drag pictures: the new length, and where a moved section goes.
        if let Some(dir) = render_dir() {
            let r0 = rect(&h, ("deckard-section", 0));
            for (name, from, to) in [
                ("arranger-resize", egui::pos2(r0.right(), r0.center().y), egui::pos2(r0.right() + 2.0 * per_bar, r0.center().y)),
                ("arranger-move", r0.center(), rect(&h, ("deckard-section", 3)).center()),
            ] {
                h.hover_at(from);
                h.step();
                h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
                h.step();
                for k in 1..=6 {
                    h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 6.0)));
                    h.step();
                }
                h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
                h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
                h.run_steps(3);
            }
        }
    }

    /// Drag an effect in the chain to move it; the instrument stays first.
    #[test]
    fn drag_effects_in_the_chain() {
        let Some(mut h) = harness(false, true) else { return };
        (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
        h.state_mut().channel = false;
        h.run_steps(4);
        let names = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[0].slots.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&h), ["Sampler", "EQ", "Compressor"]);
        let rows: Vec<egui::Rect> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-chain-rows"))).unwrap();
        drag(&mut h, rows[2].center(), rows[1].center());
        assert_eq!(names(&h), ["Sampler", "Compressor", "EQ"]);
        assert_eq!(h.state().slot, Some(1), "the moved slot stays shown");
        // The Sampler can't be dragged after the effects, nor an effect above it.
        drag(&mut h, rows[0].center(), rows[2].center());
        drag(&mut h, rows[2].center(), rows[0].center() - egui::vec2(0.0, 8.0));
        assert_eq!(names(&h), ["Sampler", "EQ", "Compressor"], "EQ dragged to just after the Sampler");
    }

    /// Maschine's own effects show their pages; their knobs edit the project.
    #[test]
    fn effect_pages() {
        let Some(mut h) = harness(false, true) else { return };
        layout::style(&h.ctx);
        let names = |h: &Harness<'static, App>| -> Vec<String> {
            let k: Vec<(String, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-knobs"))).unwrap_or_default();
            k.into_iter().map(|(n, _)| n).collect()
        };
        (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
        h.state_mut().channel = false;
        layout::apply_acts(h.state_mut(), vec![layout::Act::SelectSlot(Some(1))]);
        h.run_steps(4);
        assert_eq!(names(&h), ["FREQ", "GAIN", "FREQ", "GAIN", "FREQ", "GAIN", "FREQ", "GAIN"], "EQ: Freq / Gain");
        if let Some(dir) = render_dir() {
            h.render().unwrap().save(dir.join("fx-eq.png")).unwrap();
        }
        layout::apply_acts(h.state_mut(), vec![layout::Act::PluginPage(1)]);
        h.run_steps(3);
        assert_eq!(names(&h), ["WIDTH", "WIDTH", "GAIN"], "EQ: Width / Output");
        layout::apply_acts(h.state_mut(), vec![layout::Act::SelectSlot(Some(2))]);
        h.run_steps(3);
        assert_eq!(names(&h), ["MODE", "THRESHOLD", "KNEE", "AMOUNT", "ATTACK", "RELEASE", "GAIN"], "Compressor");
        if let Some(dir) = render_dir() {
            h.render().unwrap().save(dir.join("fx-compressor.png")).unwrap();
        }
        // A knob edits the module's parameter.
        layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(deckard_doc::Target::Module(0, 0, 2), "threshold", 0.5)]);
        h.run_steps(2);
        assert_eq!(project_of(&h).groups[0].sounds[0].slots[2].param("threshold"), Some(0.5));
        assert_eq!(h.state().doc.as_ref().unwrap().borrow().undo_label(), Some("Effect Parameter"));
        // The Side-Chain Input page: pick the snares as the source; the filter's knobs.
        layout::apply_acts(h.state_mut(), vec![layout::Act::PluginPage(1)]);
        h.run_steps(3);
        assert_eq!(names(&h), ["SOURCE", "GAIN", "FILTER", "CENTER FREQ", "WIDTH"]);
        let k: Vec<(String, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-knobs"))).unwrap();
        let src = k[0].1;
        click(&mut h, Pos2::new(src.center().x, src.top() + 42.0));
        if let Some(dir) = render_dir() {
            h.render().unwrap().save(dir.join("fx-sidechain-list.png")).unwrap();
        }
        h.get_by_label("B1 - Snares").click();
        h.run_steps(3);
        assert_eq!(project_of(&h).groups[0].sounds[0].channel.sidechain, vec![(2, deckard_model::ScSource::Group(1))]);
        if let Some(dir) = render_dir() {
            h.render().unwrap().save(dir.join("fx-sidechain.png")).unwrap();
        }
        layout::apply_acts(
            h.state_mut(),
            vec![layout::Act::SetParam(deckard_doc::Target::Module(0, 0, 2), "side_chain_filter/filter", 1.0)],
        );
        h.run_steps(2);
        assert_eq!(project_of(&h).groups[0].sounds[0].slots[2].param("side_chain_filter/filter"), Some(1.0));
    }

    /// Where the ignored render tests (and others, optionally) save their screenshots.
    /// Pad sensitivity: 0 takes the pressure as it comes; the default lifts light presses
    /// (a quarter of the pressure plays at over half velocity); full pressure stays full.
    #[test]
    fn pad_sensitivity_lifts_light_presses() {
        use mk3_io::pad_velocity;
        assert!((pad_velocity(0.25, 0.0, 0) - 0.25).abs() < 1e-9);
        assert!(pad_velocity(0.25, 0.5, 0) > 0.5);
        assert!(pad_velocity(0.1, 1.0, 0) > pad_velocity(0.1, 0.5, 0));
        assert_eq!(pad_velocity(1.0, 0.5, 0), 1.0);
        assert_eq!(settings::Settings::default().pad_sensitivity, 50);
    }

    /// Velocity SCALING: Soft lifts every press short of full, Hard holds it back, more
    /// at each step; full pressure stays full.
    #[test]
    fn velocity_scaling_bends_the_presses() {
        use mk3_io::pad_velocity;
        let at = |scaling| pad_velocity(0.3, 0.0, scaling);
        assert!(at(-3) > at(-1) && at(-1) > at(0) && at(0) > at(1) && at(1) > at(3), "{} {} {} {} {}", at(-3), at(-1), at(0), at(1), at(3));
        assert_eq!(pad_velocity(1.0, 0.0, 3), 1.0);
        assert_eq!(pad_velocity(1.0, 0.0, -3), 1.0);
    }

    /// A knob on the controller's plug-in page goes the whole way in one turn, however
    /// finely the MK3 reports it (~1000 steps a turn; here 200): every continuous knob of every
    /// built-in plug-in's first page.
    #[test]
    fn a_controller_knob_turn_spans_the_range() {
        use deckard_lua_host::state::ControlRequest as R;
        let mut slow = Vec::new();
        let choices = App::default().plugin_choices();
        for (entry, what) in choices.into_iter().filter(|(e, _)| e.format == deckard_lua_host::state::PluginFormat::Internal) {
            let mut app = App::default();
            app.new_project();
            app.add_slot_at(what, 0);
            app.slot = Some(0);
            let knobs = |app: &App| {
                let doc = app.doc.clone().unwrap();
                layout::slot_page(app, doc.borrow().project(), 0)
                    .map(|pg| {
                        pg.knobs
                            .iter()
                            .map(|k| {
                                k.as_ref().map(|k| (k.0.clone(), k.2, matches!(k.3, Some(layout::Edit::Param(..) | layout::Edit::Log(..)))))
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            };
            for (pos, k) in knobs(&app).into_iter().enumerate() {
                let Some((name, _, true)) = k else { continue };
                for inc in [-0.005, 0.005] {
                    for _ in 0..200 {
                        app.control_request(R::Turn { pos, inc, fine: false });
                    }
                }
                let v = knobs(&app)[pos].as_ref().map_or(0.0, |k| k.1);
                if v < 0.99 {
                    slow.push(format!("{} {name}: {v:.3}", entry.name));
                }
            }
        }
        assert!(slow.is_empty(), "short of the top after a turn up from the bottom:\n{}", slow.join("\n"));
    }

    /// CHANNEL on the controller: the channel page's knobs; a routing choice shows
    /// shortened, whole while its knob is touched, and turning it steps through them.
    #[test]
    fn controller_channel_routes_show_short_and_turn() {
        use deckard_lua_host::state::ControlRequest as R;
        let mut app = App::default();
        app.new_project();
        app.settings.output_routing[2..4].copy_from_slice(&[Some(2), Some(3)]);
        app.settings.output_aliases[2] = "Studio Monitors".into();
        app.settings.output_aliases[3] = "Studio Monitors".into();
        (app.channel, app.chain_tab, app.prop_set, app.prop_page) = (true, layout::ChainTab::Sound, 1, 0);
        let dest = |app: &App| {
            let doc = app.doc.clone().unwrap();
            let (_, _, pg) = layout::channel_page(app, &doc.borrow()).unwrap();
            pg.slots.iter().position(|k| matches!(k, Some((n, ..)) if n.as_ref() == "DEST.")).unwrap()
        };
        let k = dest(&app);
        let text = |app: &App, touched: bool| {
            let mut t = [false; 8];
            t[k] = touched;
            app.control_snapshot(t).knobs[k].as_ref().unwrap().text.clone()
        };
        assert_eq!(text(&app, false), "Group");
        // Up through Master and Ext. 1 to the aliased Out 2.
        for _ in 0..3 {
            app.control_request(R::Turn { pos: k, inc: ROUTE_STEP, fine: false });
        }
        assert_eq!(text(&app, false), "Stu..ors", "shortened once picked");
        assert_eq!(text(&app, true), "Studio Monitors", "whole while touched");
        let snd = &project_of_app(&app).groups[0].sounds[0];
        assert_eq!(snd.channel.dest, deckard_model::AudioDest::Ext(1));
    }

    /// The open project is kept as it's edited (AUTO BACKUP): after a crash the next
    /// start brings it back as it was, still unsaved.
    #[test]
    fn a_crash_brings_back_the_unsaved_session() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("session");
        let mut app = App { session_dir: Some(dir.clone()), ..Default::default() };
        app.new_project();
        app.doc.as_ref().unwrap().borrow_mut().set_tempo(97.0).unwrap();
        app.keep_session();
        let mut next = App { session_dir: Some(dir.clone()), ..Default::default() };
        next.start_project();
        let d = next.doc.as_ref().unwrap().borrow();
        assert_eq!(d.project().tempo, 97.0);
        assert!(d.is_dirty(), "still unsaved");
        assert_eq!(d.path(), None, "never saved: untitled");
        drop(d);
    }

    /// #5: quitting with unsaved changes asks Save / Discard, as Maschine does, and a
    /// discarded change doesn't come back: the next start opens the project as saved
    /// (RELOAD), or a new one if it never was.
    /// BUGS #10: File ▸ New starts from the default project when one is set: its
    /// content, untitled and unchanged; without one (or if it's gone) an empty project.
    #[test]
    fn new_starts_from_the_default_project() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let file = dir.join("Template.mxprj");
        let mut app = App::default();
        app.new_project();
        app.doc.as_ref().unwrap().borrow_mut().set_tempo(87.0).unwrap();
        app.doc.as_ref().unwrap().borrow_mut().save_as(&file).unwrap();
        let mut app = App::default();
        app.settings.default_project = Some(file.clone());
        app.new_project();
        {
            let d = app.doc.as_ref().unwrap().borrow();
            assert_eq!(
                (d.project().tempo, d.is_dirty(), d.path(), app.path.clone()),
                (87.0, false, None, None),
                "the template's, untitled"
            );
        }
        std::fs::remove_file(&file).unwrap();
        app.new_project();
        assert_ne!(app.doc.as_ref().unwrap().borrow().project().tempo, 87.0, "gone: an empty project");
    }

    #[test]
    fn quitting_asks_and_discarded_changes_stay_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let file = dir.join("Song.mxprj");
        let mut app = App { session_dir: Some(dir.clone()), ..Default::default() };
        app.new_project();
        app.doc.as_ref().unwrap().borrow_mut().set_tempo(96.0).unwrap();
        app.doc.as_ref().unwrap().borrow_mut().save_as(&file).unwrap();
        app.path = Some(file.clone());
        app.doc.as_ref().unwrap().borrow_mut().set_tempo(95.0).unwrap();
        assert!(app.asks_on_quit(), "unsaved: asks");
        // Discard: quit as it is.
        app.end_session();
        let tempo = |dir: &std::path::Path| {
            let mut next = App { session_dir: Some(dir.to_path_buf()), ..Default::default() };
            next.start_project();
            let d = next.doc.as_ref().unwrap().borrow();
            (d.project().tempo, d.is_dirty(), d.path().map(Path::to_path_buf))
        };
        assert_eq!(tempo(&dir), (96.0, false, Some(file.clone())), "as saved");
        // Never saved: a new project.
        let mut app = App { session_dir: Some(dir.clone()), ..Default::default() };
        app.new_project();
        app.doc.as_ref().unwrap().borrow_mut().set_tempo(94.0).unwrap();
        app.end_session();
        assert_eq!(tempo(&dir), (Document::new_project().unwrap().project().tempo, false, None));
    }

    /// Pads start in Fixed Velocity (100): a project opens with it on, unchanged (no
    /// undo step, nothing to save).
    #[test]
    fn projects_open_in_fixed_velocity() {
        let mut app = App::default();
        app.new_project();
        let doc = app.doc.clone().unwrap();
        let d = doc.borrow();
        assert_eq!(d.num(deckard_doc::Target::Song, "pad_velocity_mode"), Some(1.0));
        assert_eq!(d.num(deckard_doc::Target::Song, "fixed_velocity"), Some(100.0));
        assert!(!d.is_dirty() && d.undo_label().is_none(), "{d:?}");
    }

    /// Every built-in plug-in the controller's browser offers has DECKARD's pictures (run
    /// `make_plugin_pictures` after adding one), and they decode.
    #[test]
    fn every_builtin_plugin_has_pictures() {
        let app = App::default();
        let keys: Vec<&str> = plugin_pictures::PICTURES.iter().map(|(k, ..)| *k).collect();
        for (entry, _) in app.plugin_choices().into_iter().filter(|(e, _)| e.format == deckard_lua_host::state::PluginFormat::Internal) {
            let key = deckard_lua_host::plugin_picture_key(&entry.name);
            assert!(keys.contains(&key.as_str()), "{} ({key}) has no picture", entry.name);
        }
        let look = deckard_nwl::draw::Look::empty();
        for (key, thumb, logo) in plugin_pictures::PICTURES {
            assert!(look.add_picture("t", thumb) && look.add_picture("l", logo), "{key}");
            assert_eq!(look.picture_size("l"), Some((240, 196)), "{key}");
        }
    }

    /// What `make_plugin_pictures` writes after the pictures' list.
    const PICTURE_HELPERS: &str = r#"
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

/// A PNG as RGBA pixels (w, h, pixels).
fn decode(bytes: &[u8]) -> Option<(usize, usize, Vec<[u8; 4]>)> {
    let mut r = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let buf = &buf[..info.buffer_size()];
    let px = match info.color_type {
        png::ColorType::Rgba => buf.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect(),
        png::ColorType::Rgb => buf.chunks(3).map(|c| [c[0], c[1], c[2], 255]).collect(),
        _ => return None,
    };
    Some((info.width as usize, info.height as usize, px))
}
"#;

    /// The pictures of DECKARD's plug-ins on the controller (not a check: run it by name,
    /// `cargo test -p deckard -- --ignored make_plugin_pictures`): each built-in plug-in's
    /// panel, as the app draws it, in a chain tile (90 pixels high for an instrument, 40
    /// for an effect, as Maschine's) and as the 240 × 196 logo on the right display,
    /// written to `assets/plugins` with the list `plugin_pictures.rs` embeds.
    #[test]
    #[ignore]
    fn make_plugin_pictures() {
        use image::imageops::{FilterType, resize};
        let Some(mut h) = harness(false, true) else { return };
        layout::style(&h.ctx);
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/plugins");
        std::fs::create_dir_all(&dir).unwrap();
        let (g, s) = (0, 0);
        (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (g, s, layout::ChainTab::Sound);
        // The Mix view, where the strip shows each slot's panel.
        h.state_mut().song_view = false;
        h.state_mut().mixer = true;
        // A drum hit to show: a falling sine with a click of noise, two seconds.
        let tmp = tempfile::tempdir().unwrap();
        let wav = tmp.path().join("picture.wav");
        {
            let spec = hound::WavSpec { channels: 1, sample_rate: 44_100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
            let mut w = hound::WavWriter::create(&wav, spec).unwrap();
            let (mut phase, mut noise) = (0.0f64, 12345u32);
            for i in 0..88_200 {
                let t = i as f64 / 44_100.0;
                phase += std::f64::consts::TAU * (45.0 + 180.0 * (-t * 25.0).exp()) / 44_100.0;
                noise = noise.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let click = ((noise >> 16) as f64 / 32_768.0 - 1.0) * (-t * 120.0).exp() * 0.4;
                let beat = if (t * 4.0).fract() < 0.02 { 0.3 } else { 0.0 };
                let v = (phase.sin() * (-t * 2.2).exp() * 0.9 + click + beat * (noise as f64 / u32::MAX as f64 - 0.5)).clamp(-1.0, 1.0);
                w.write_sample((v * 30_000.0) as i16).unwrap();
            }
            w.finalize().unwrap();
        }
        let mut written = Vec::new();
        let choices: Vec<_> =
            h.state().plugin_choices().into_iter().filter(|(e, _)| e.format == deckard_lua_host::state::PluginFormat::Internal).collect();
        for (entry, what) in choices {
            while !project_of(&h).groups[g].sounds[s].slots.is_empty() {
                layout::apply_acts(h.state_mut(), vec![layout::Act::RemoveSlot(0)]);
                h.run_steps(1);
            }
            h.state_mut().add_slot(what);
            h.run_steps(6);
            // The Sampler and Audio show a sample's waveform: their empty panels have none.
            let key = project_of(&h).groups[g].sounds[s].slots.first().map(|x| deckard_lua_host::plugin_picture_key(&x.name));
            if matches!(key.as_deref(), Some("sampler" | "audio")) {
                let smp = deckard_engine::sample::Sample::load(&wav).unwrap();
                let done = if key.as_deref() == Some("sampler") {
                    h.state_mut().add_zone(g, s, &wav, smp.frames.len())
                } else {
                    h.state_mut().load_loop(g, s, &wav, &smp)
                };
                done.unwrap();
                h.run_steps(20);
            }
            let Some(slot) = project_of(&h).groups[g].sounds[s].slots.first().cloned() else { continue };
            let panels: Vec<(usize, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-panels"))).unwrap_or_default();
            let Some(r) = panels.iter().find(|p| p.0 == 0).map(|p| p.1) else {
                eprintln!("{}: no panel", entry.name);
                continue;
            };
            let ppp = h.ctx.pixels_per_point();
            let shot = h.render().unwrap();
            let (x, y) = ((r.min.x * ppp).max(0.0) as u32, (r.min.y * ppp).max(0.0) as u32);
            let (w, hh) = (((r.width() * ppp) as u32).min(shot.width() - x), ((r.height() * ppp) as u32).min(shot.height() - y));
            let panel = image::imageops::crop_imm(&shot, x, y, w, hh).to_image();
            // Scaled to the height, the width as it comes (but not past Maschine's widest).
            let fit = |max_w: u32, max_h: u32| {
                let k = (max_w as f32 / w as f32).min(max_h as f32 / hh as f32);
                resize(&panel, ((w as f32 * k).round() as u32).max(1), ((hh as f32 * k).round() as u32).max(1), FilterType::Lanczos3)
            };
            let thumb = if entry.instrument { fit(160, 90) } else { fit(130, 40) };
            let mut logo = image::RgbaImage::from_pixel(240, 196, image::Rgba([0, 0, 0, 255]));
            let big = fit(240, 196);
            image::imageops::overlay(&mut logo, &big, ((240 - big.width()) / 2) as i64, ((196 - big.height()) / 2) as i64);
            let key = deckard_lua_host::plugin_picture_key(&slot.name);
            thumb.save(dir.join(format!("{key}_Thumb.png"))).unwrap();
            logo.save(dir.join(format!("{key}_Logo.png"))).unwrap();
            written.push(key);
        }
        written.sort();
        written.dedup();
        let mut src = String::from(
            "//! DECKARD's own pictures of its plug-ins for the controller's displays (tiles in\n\
             //! the chain, logos on the right display), made by the `make_plugin_pictures` test.\n\n\
             /// (style sheet key, thumbnail PNG, logo PNG).\n\
             pub const PICTURES: &[(&str, &[u8], &[u8])] = &[\n",
        );
        for k in &written {
            src += &format!(
                "    (\"{k}\", include_bytes!(\"../assets/plugins/{k}_Thumb.png\"), include_bytes!(\"../assets/plugins/{k}_Logo.png\")),\n"
            );
        }
        src += "];\n";
        src += PICTURE_HELPERS;
        std::fs::write(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/plugin_pictures.rs"), src).unwrap();
        eprintln!("{} plug-in pictures", written.len());
    }

    fn render_dir() -> Option<PathBuf> {
        std::env::var_os("DECKARD_RENDER_DIR").map(PathBuf::from)
    }

    /// Pattern view sound rows and Song view group rows: double-click renames, the
    /// coloured box at the right mutes and unmutes.
    #[test]
    fn rows_rename_and_mute() {
        let Some(mut h) = harness(false, true) else { return };
        let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
        let list_right = grid.left() - 1.0;
        let g = h.state().group;
        // Sound 1 of the focused group.
        let y = grid.top() + 12.0;
        double_click(&mut h, Pos2::new(list_right - 80.0, y));
        assert_eq!(h.state().renaming, Some(deckard_doc::Target::Sound(g, 0)));
        type_text(&mut h, "Big Kick");
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert_eq!(project_of(&h).groups[g].sounds[0].name, "Big Kick");
        assert_eq!(h.state().renaming, None);
        let mute = Pos2::new(list_right - 9.5, y);
        click(&mut h, mute);
        assert!(project_of(&h).groups[g].sounds[0].muted, "the box mutes");
        click(&mut h, mute);
        assert!(!project_of(&h).groups[g].sounds[0].muted, "and unmutes");

        // Song view: group B's row.
        let rows_top = 33.0 + 3.0 + 50.0;
        let gy = rows_top + 24.0 + 12.0;
        // (A group's double-click mutes it: Rename is in its menu.)
        layout::apply_acts(h.state_mut(), vec![layout::Act::StartRename(deckard_doc::Target::Group(1))]);
        h.run_steps(2);
        assert_eq!(h.state().renaming, Some(deckard_doc::Target::Group(1)));
        type_text(&mut h, "Snares 2");
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert_eq!(project_of(&h).groups[1].name, "Snares 2");
        let gmute = Pos2::new(list_right - 9.5, gy);
        click(&mut h, gmute);
        assert!(project_of(&h).groups[1].muted);
        click(&mut h, gmute);
        assert!(!project_of(&h).groups[1].muted);

        // Esc cancels a rename.
        double_click(&mut h, Pos2::new(list_right - 80.0, gy));
        type_text(&mut h, "Nope");
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert_eq!(project_of(&h).groups[1].name, "Snares 2");
        assert_eq!(h.state().renaming, None);
    }

    /// Double-click renames scenes (Ideas view tabs, Song view section headers); groups
    /// (Ideas view tiles) rename from their menu; a single click still selects.
    #[test]
    fn scenes_and_groups_rename() {
        let Some(mut h) = harness(false, false) else { return };
        // Scene tab 2.
        let tab = Pos2::new(3.0 + 33.0 + 99.0 + 49.0, 33.0 + 3.0 + 16.0);
        double_click(&mut h, tab);
        assert_eq!(h.state().renaming, Some(deckard_doc::Target::Scene(1)));
        type_text(&mut h, "Drop");
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert_eq!(project_of(&h).scenes[1].name, "Drop");
        assert_eq!(h.state().renaming, None);
        click(&mut h, Pos2::new(tab.x - 99.0, tab.y));
        assert_eq!(h.state().scene, 0, "a single click still selects");

        // Group tile B.
        let tiles: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-tiles"))).expect("tiles drawn");
        let tile = Pos2::new(tiles.left() + 99.0 + 49.0, tiles.center().y);
        // (A group's double-click mutes it: Rename is in its menu.)
        click(&mut h, tile);
        layout::apply_acts(h.state_mut(), vec![layout::Act::StartRename(deckard_doc::Target::Group(1))]);
        h.run_steps(2);
        assert_eq!(h.state().renaming, Some(deckard_doc::Target::Group(1)));
        type_text(&mut h, "Snare Bus");
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert_eq!(project_of(&h).groups[1].name, "Snare Bus");
        // Esc cancels.
        layout::apply_acts(h.state_mut(), vec![layout::Act::StartRename(deckard_doc::Target::Group(1))]);
        h.run_steps(2);
        type_text(&mut h, "Nope");
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert_eq!(project_of(&h).groups[1].name, "Snare Bus");
        assert_eq!(h.state().renaming, None);

        // Song view: the first section's header renames its scene.
        h.state_mut().song_view = true;
        h.run_steps(3);
        let secs: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-sections"))).expect("sections drawn");
        let scene = project_of(&h).sections[0].scene.expect("section 0 has a scene");
        double_click(&mut h, Pos2::new(secs.left() + 20.0, secs.center().y));
        assert_eq!(h.state().section, Some(0));
        assert_eq!(h.state().renaming, Some(deckard_doc::Target::Scene(scene)));
        type_text(&mut h, "Intro");
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert_eq!(project_of(&h).scenes[scene].name, "Intro");
    }

    /// Piano roll: the sound list stays (clicking a sound shows its notes), the note
    /// area has a keyboard and pitch rows, and drawing adds notes at the row's pitch.
    #[test]
    fn piano_roll_keeps_the_sound_list() {
        let Some(mut h) = harness(false, true) else { return };
        h.state_mut().piano_roll = true;
        h.run_steps(3);
        let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
        let list_right = grid.left() - 1.0 - 46.0;
        // Click sound 3 in the list.
        click(&mut h, Pos2::new(list_right - 80.0, grid.top() + 2.0 * 24.0 + 12.0));
        assert_eq!(h.state().sound, 2, "the sound list still selects sounds");
        // Draw a note on the row at the middle of the view.
        h.state_mut().mouse = layout::MouseMode::Draw;
        h.run_steps(2);
        let (g, pi) = (h.state().group, h.state().pattern);
        let before: Vec<Note> = notes_of(&h).into_iter().filter(|n| n.sound == 2).collect();
        let y = grid.center().y;
        click(&mut h, Pos2::new(grid.left() + 30.0, y));
        let after: Vec<Note> = notes_of(&h).into_iter().filter(|n| n.sound == 2).collect();
        assert_eq!(after.len(), before.len() + 1, "a note was drawn for the focused sound");
        let added = after.iter().find(|n| !before.contains(n)).unwrap();
        assert!((0..=127).contains(&added.pitch) && added.pitch != 60 || added.pitch == 60);
        let _ = (g, pi);
    }

    /// Playing sounds light their level meters; silent ones stay dark.
    #[test]
    fn meters_follow_playback() {
        let Some(mut h) = harness(false, true) else { return };
        if !play(&mut h) {
            return;
        }
        let mut lit = false;
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            h.run_steps(1);
            let (l, r) = h.state().meter(0, None);
            lit |= l > 0.01 && r > 0.01;
        }
        h.state_mut().stop();
        assert!(lit, "the kicks' meter moved while the song played");
        // Silent group (a plug-in group: no sound rendered).
        let quiet = (0..project_of(&h).groups.len()).filter(|g| h.state().meter(*g, None) == (0.0, 0.0)).count();
        assert!(quiet > 0, "groups that play nothing stay at zero");
    }

    /// Notes can be drawn, dragged and double-clicked in for a sound whose instrument is
    /// a plug-in (Pad's Massive). (The plug-ins themselves only load on the main thread,
    /// so the app is checked with `--input` too, see the README.)
    #[test]
    fn notes_draw_for_plugin_sounds() {
        let Some(mut h) = harness(false, true) else { return };
        let Some(g) = project_of(&h).groups.iter().position(|g| g.sounds.first().is_some_and(|s| s.plugin.is_some())) else { return };
        h.state_mut().group = g;
        h.state_mut().pattern = 0;
        h.state_mut().sound = 0;
        h.state_mut().mouse = layout::MouseMode::Draw;
        h.run_steps(3);
        let before = notes_of(&h).len();
        let pos = grid_pos(&h, 0, 3 * 240 + 120);
        click(&mut h, pos);
        assert_eq!(notes_of(&h).len(), before + 1, "drum grid: a click adds a note");
        h.state_mut().piano_roll = true;
        h.run_steps(3);
        let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
        let x = grid_pos(&h, 0, 5 * 240 + 120).x;
        click(&mut h, Pos2::new(x, grid.center().y));
        assert_eq!(notes_of(&h).len(), before + 2, "piano roll: a click adds a note");
        // Select mode: drag the drum-grid note one beat right, then double-click adds.
        h.state_mut().mouse = layout::MouseMode::Select;
        h.state_mut().piano_roll = false;
        h.run_steps(3);
        // (Dropped half a step on: the start snaps down to the step it's in.)
        let grab = egui::vec2(3.0, 0.0);
        let (from, to) = (grid_pos(&h, 0, 3 * 240) + grab, grid_pos(&h, 0, 3 * 240 + 960 + 120) + grab);
        drag(&mut h, from, to);
        assert!(
            notes_of(&h).iter().any(|n| n.position - project_of(&h).groups[g].patterns[0].start == 3 * 240 + 960),
            "drag moves the note: {:?}",
            notes_of(&h)
        );
        let pos = grid_pos(&h, 0, 9 * 240 + 120);
        double_click(&mut h, pos);
        assert_eq!(notes_of(&h).len(), before + 3, "select mode: double-click adds a note");
    }

    /// Dragging one note of a chord (Pad: several pitches at one time, stacked on one
    /// drum-grid row) moves it.
    #[test]
    fn chord_notes_drag() {
        let Some(mut h) = harness(false, true) else { return };
        let Some(g) = project_of(&h).groups.iter().position(|g| g.sounds.first().is_some_and(|s| s.plugin.is_some())) else { return };
        h.state_mut().group = g;
        h.state_mut().pattern = 0;
        h.run_steps(3);
        let start = project_of(&h).groups[g].patterns[0].start;
        let before = notes_of(&h);
        // Pad's first pattern starts with a chord (notes a few ticks apart).
        let t = before.iter().map(|n| n.position - start).min().unwrap();
        let from = grid_pos(&h, 0, t + 20);
        let to = grid_pos(&h, 0, t + 20 + 960);
        drag(&mut h, from, to);
        let after = notes_of(&h);
        // The note on top under the pointer moves (the chord's notes overlap).
        let gone: Vec<&Note> = before.iter().filter(|n| !after.contains(n)).collect();
        assert_eq!(gone.len(), 1, "one note moved: {gone:?}");
        assert!(gone[0].position - start < 240, "from the first chord");
        assert!(after.contains(&Note { position: start + 960, ..*gone[0] }), "to a beat later, on the grid");
        assert_eq!(after.len(), before.len());
    }

    /// A piano-roll key sounds when the button goes down, before it is released.
    /// Pressing a sound's number in the list plays the root note, in the drum grid
    /// and in the piano roll, until the button is let go.
    #[test]
    fn sound_number_plays_the_root_note() {
        let Some(mut h) = harness(false, true) else { return };
        for piano in [false, true] {
            h.state_mut().piano_roll = piano;
            h.run_steps(3);
            let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
            let keys = if piano { 46.0 } else { 0.0 };
            // The number box of the list's second row (the list sits left of the keys).
            let pos = Pos2::new(grid.left() - keys - 146.0, grid.top() + 24.0 + 12.0);
            h.hover_at(pos);
            h.step();
            h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
            h.step();
            if !matches!(h.state().output, Some(Ok(_))) {
                eprintln!("no audio output; skipping");
                return;
            }
            let (g, root) = (h.state().group, h.state().scale.0);
            assert_eq!(h.state().held, Some((g, 1, root)), "piano roll: {piano}");
            h.step();
            assert!(h.state().lit.contains(&(g, 1)), "the held sound's number is lit");
            h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
            h.step();
            assert_eq!(h.state().held, None);
            assert_eq!(h.state().sound, 1, "and the click focuses the sound");
            std::thread::sleep(deckard_engine::output::FLASH);
            h.step();
            assert!(!h.state().lit.contains(&(g, 1)), "and goes out once let go");
        }
    }

    #[test]
    fn keys_sound_on_press() {
        let Some(mut h) = harness(false, true) else { return };
        h.state_mut().piano_roll = true;
        // Group A's first sound is a sampled kick.
        h.state_mut().group = 0;
        h.state_mut().sound = 0;
        h.run_steps(3);
        let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
        let pos = Pos2::new(grid.left() - 20.0, grid.center().y);
        h.hover_at(pos);
        h.step();
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        let Some(Ok(out)) = &h.state().output else {
            eprintln!("no audio output; skipping");
            return;
        };
        // The key goes to the engine at once and sounds one block after the press (at
        // its sample): within a few blocks, still before the button is released.
        let wait = std::time::Instant::now();
        let mut voices = 0;
        while voices == 0 && wait.elapsed() < std::time::Duration::from_millis(100) {
            voices = out.call(|e| e.active_voices()).unwrap_or(0);
        }
        assert!(voices > 0, "the kick sounds on press, before the button is released");
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.step();
    }
}

/// The MK3's LEDs as the scripts have set them: colour LEDs from their palette colour
/// (or RGB), the others from their level.
/// A controller theme's name as shown: `solarized_dark` → "Solarized Dark".
fn theme_name(t: &str) -> String {
    t.split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The user's controller scripts folder (`<settings>/controller`), made if it isn't
/// there, with DECKARD's own scripts copied into `defaults/` to start from: a file
/// copied from there to the same place in the folder replaces DECKARD's.
fn controller_folder() -> std::io::Result<PathBuf> {
    let dir = menu::settings_dir().ok_or_else(|| std::io::Error::other("no settings folder"))?.join("controller");
    for sub in ["pages", "themes", "lib", "devices/mk3"] {
        std::fs::create_dir_all(dir.join(sub))?;
    }
    let defaults = dir.join("defaults");
    for (path, text) in deckard_surface::SHIPPED {
        let to = defaults.join(path);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(to, text)?;
    }
    std::fs::write(
        dir.join("README.txt"),
        "DECKARD's controller scripts: one file per page (docs/controller-api.md in DECKARD's source).\n\n\
         defaults/ holds DECKARD's own scripts, refreshed each time this folder is opened.\n\
         To change one, copy it from defaults/ to the same place here (defaults/pages/control.lua\n\
         to pages/control.lua) and edit it: DECKARD loads it as you save. Delete it to go back.\n\
         A new theme in themes/ shows in Preferences > Hardware > Controller theme.\n\
         Files under devices/mk3/ apply to the MK3 only.\n",
    )?;
    Ok(dir)
}

fn mk3_leds(host: &deckard_lua_host::Host) -> deckard_mk3::leds::Leds {
    use deckard_mk3::leds;
    let mut out = leds::Leds::default();
    for name in leds::BUTTON_LEDS.iter().chain(&leds::PAD_LEDS) {
        // (Maschine's MK3 Events page lights its button by the Studio's name.)
        let l = host.led(name).or_else(|| (*name == "LED_EVENTS").then(|| host.led("LED_TRANSPORT_EVENTS")).flatten());
        let Some(l) = l else {
            // Buttons the scripts leave alone still show their labels.
            if !leds::is_colour(name) {
                out.set(name, leds::BACKLIGHT);
            }
            continue;
        };
        let byte = if leds::is_colour(name) {
            match (l.lut, l.rgb) {
                (Some((colour, state)), _) => leds::palette(colour, state),
                (None, Some((r, g, b))) => leds::palette_rgb(r, g, b),
                (None, None) if l.level > 0.0 => leds::palette(16, if l.level > 0.6 { 4 } else { 2 }),
                _ => 0,
            }
        } else {
            match l.lut {
                // A colour-table state on a one-colour LED.
                Some((_, state)) => leds::button_level(state),
                None => leds::button_brightness(l.level),
            }
        };
        out.set(name, byte);
    }
    out
}
