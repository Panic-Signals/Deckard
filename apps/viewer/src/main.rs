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

mod about;
mod arp_ui;
mod audio_settings;
mod audio_ui;
mod blink;
mod boot;
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
#[cfg(test)]
mod controller_quality_tests;
mod controller_timing;
mod drop_pos;
mod drum_ui;
mod export;
mod extra_outs;
mod github;
mod goodbye;
mod group_move;
mod icon;

/// The version shown: the release's (`DECKARD_VERSION`, set by the build scripts), else
/// the crate's.
pub const VERSION: &str = match option_env!("DECKARD_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
/// Built by the release scripts (with a version minted on GitHub), not from source.
pub const RELEASE_BUILD: bool = option_env!("DECKARD_VERSION").is_some();
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
mod input_levels;
mod jam_io;
#[cfg(test)]
mod jam_tests;
mod keep_running;
mod keys;
mod layout;
mod lock;
#[cfg(test)]
mod lock_hosted_tests;
#[cfg(test)]
mod lock_tests;
mod loops;
#[cfg(test)]
mod macro_surface_tests;
#[cfg(test)]
mod manual_shots;
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
mod ni_agent;
mod ni_services;
#[cfg(test)]
mod pad_blink_tests;
#[cfg(test)]
mod pad_light_tests;
#[cfg(test)]
mod parity_tests;
mod plugin_macros;
#[cfg(test)]
mod plugin_panel_tests;
mod plugin_pictures;
mod preset_load;
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
#[cfg(test)]
mod scene_tempo_tests;
mod settings;
mod splice;
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
mod surface_mixer_tests;
#[cfg(test)]
mod surface_repeat_tests;
mod surface_sampling;
#[cfg(test)]
mod surface_sampling_tests;
#[cfg(test)]
mod surface_select_tests;
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
mod update;
mod user_chords;
mod wine;
mod zhora_ui;

use deckard_doc::{Document, Shared};
use deckard_engine::{Engine, Mode, SampleCache, output::Output};
use deckard_model::{Note, PPQ};
use eframe::egui;
use std::path::{Path, PathBuf};

/// A group's new pattern playing in `scene` (Ideas view), as drawing a note's first
/// pattern does; a project with no scene gets one.
fn play_in_scene(d: &mut deckard_doc::Document, scene: usize, g: usize, pi: usize) -> Result<(), String> {
    let scene = if scene < d.project().scenes.len() { scene } else { d.add_scene()? };
    d.assign_pattern(scene, g, Some(pi))
}

fn main() {
    deckard_plugins::mark_main_thread();
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
    // --write-icon OUT.png [SIZE] [--icon-style television|letter]: draw the app's icon
    // (1024 px, the television), and stop.
    if let Some(out) = flag("--write-icon") {
        let size = args.iter().position(|a| a == "--write-icon").and_then(|i| args.get(i + 2)).and_then(|v| v.parse().ok()).unwrap_or(1024);
        let style = match flag("--icon-style") {
            None => icon::Style::default(),
            Some(k) => icon::Style::from_key(&k).unwrap_or_else(|| {
                eprintln!("--icon-style: television or letter, not {k}");
                std::process::exit(2);
            }),
        };
        if let Err(e) = icon::write_png(Path::new(&out), size, style) {
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
        // The icon picked in Preferences ▸ General, from the start.
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([width, 930.0])
            .with_drag_and_drop(true)
            .with_icon(std::sync::Arc::new(settings::Settings::load().app_icon.icon_data())),
        ..Default::default()
    };
    let splash = boot::splash_wanted(&args);
    let result = eframe::run_native(
        "DECKARD",
        options,
        Box::new(move |cc| {
            layout::style(&cc.egui_ctx);
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
                _running: Some(keep_running::begin()),
                boot: Some(boot::Boot::new(boot::all_steps(path))),
                ..Default::default()
            };
            if splash {
                // The frame loop runs startup, a step a frame; there are no flags to apply.
                return Ok(Box::new(app));
            }
            app.boot_to_end();
            app.boot = None;
            if app.settings.app_icon != icon::Style::default() {
                // The Dock and the taskbar's big icon, which the window's icon doesn't set.
                icon::apply(&cc.egui_ctx, app.settings.app_icon);
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

/// Where a chain was left: its focused slot, the page each of its slots showed, and the
/// page each property set (Input, Output, Groove, Macro) showed. (A sound's Groove
/// page is the app's, `App::groove_page`, not the chain's.)
#[derive(Debug, Default, Clone)]
pub struct ChainPages {
    slot: Option<usize>,
    slot_pages: std::collections::HashMap<usize, usize>,
    prop_pages: [usize; 4],
}

/// A plug-in's parameter pages as the SOUND tab (or its strip panel) shows them, and the
/// values of the page shown (value, text).
pub struct PluginView {
    /// The chain and slot it shows.
    pub key: (deckard_doc::Chain, usize),
    pub pages: Vec<ViewPage>,
    pub sidechain: bool,
    /// The page whose values `values` holds (`usize::MAX`: none yet).
    pub page: usize,
    /// The page the slot shows (its own; the focused slot's follows `App::plugin_page`).
    pub shown: usize,
    pub values: Vec<(f64, String)>,
    refreshed: std::time::Instant,
    /// When it was last drawn (or focused): views not seen for [`VIEW_KEEP`] go.
    pub(crate) seen: std::time::Instant,
    /// The slot's plug-in (its id in the project) when made: another there drops it.
    plugin_id: String,
    /// The instance it reads (another in the slot rebuilds the view).
    instance: std::sync::Weak<std::sync::Mutex<deckard_plugins::Plugin>>,
    pub error: Option<String>,
    /// Values DECKARD sent (parameter, value, when) the plug-in may not report yet.
    pub sent: Vec<(u32, f64, std::time::Instant)>,
}

/// How often a view's values are read again, and the longest one waits when many are due.
pub(crate) const VIEW_REFRESH: std::time::Duration = std::time::Duration::from_millis(100);
const VIEW_OVERDUE: std::time::Duration = std::time::Duration::from_millis(200);
/// How many due views are read in one update, past those overdue.
const VIEW_READS: usize = 4;
/// How long a view of a slot no longer drawn (nor focused) is kept.
pub(crate) const VIEW_KEEP: std::time::Duration = std::time::Duration::from_secs(1);

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

/// A saved page's name: its explicit `name`, or its knobs' section headings.
fn saved_page_name(pg: &deckard_model::PluginPage) -> String {
    if !pg.name.is_empty() {
        return pg.name.clone();
    }
    let mut sections: Vec<&str> = Vec::new();
    for k in &pg.params {
        let s = k.section.trim();
        if !s.is_empty() && !sections.contains(&s) {
            sections.push(s);
        }
    }
    sections.join(" / ")
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
    let mut pages: Vec<ViewPage> = if saved.iter().any(|pg| !pg.params.is_empty()) {
        let all = p.parameter_list(false);
        saved
            .iter()
            .map(|pg| {
                let knobs = pg
                    .params
                    .iter()
                    .filter_map(|k| {
                        let info = all
                            .iter()
                            .find(|i| i.id == k.id)
                            .or_else(|| (k.id != u32::MAX).then(|| all.get(k.id as usize)).flatten())?
                            .clone();
                        let label = if k.name.is_empty() { info.title.clone() } else { k.name.clone() };
                        Some((info, label, k.section.clone()))
                    })
                    .collect();
                ViewPage::new(p, saved_page_name(pg), knobs)
            })
            .filter(|pg| !pg.knobs.is_empty())
            .collect()
    } else {
        p.parameters()
            .into_iter()
            .take(128)
            .collect::<Vec<_>>()
            .chunks(8)
            .map(|c| ViewPage::new(p, String::new(), c.iter().map(|i| (i.clone(), i.title.clone(), String::new())).collect()))
            .collect()
    };
    name_unnamed_pages(&mut pages);
    pages
}

/// Give pages that have no explicit section headings (`autoname` NKS presets, or
/// automatic parameter pages) section headings and page names derived from their knob
/// labels.
fn name_unnamed_pages(pages: &mut [ViewPage]) {
    let mut prev: Option<(String, usize)> = None;
    for pg in pages.iter_mut() {
        if !pg.name.is_empty() {
            prev = Some((pg.name.clone(), 1));
            continue;
        }
        let derived = infer_knob_sections(&mut pg.knobs);
        if !derived.is_empty() {
            prev = Some((derived.clone(), 1));
            pg.name = derived;
        } else if let Some((base, n)) = &mut prev {
            *n += 1;
            pg.name = format!("{base} {n}");
        } else if let Some((_, first, sec)) = pg.knobs.first_mut() {
            let fallback = first.split_whitespace().next().unwrap_or(first.as_str()).trim().to_string();
            let name = if fallback.is_empty() { "Main".to_string() } else { fallback };
            if sec.is_empty() {
                *sec = name.clone();
            }
            prev = Some((name.clone(), 1));
            pg.name = name;
        }
    }
}

/// Infer section headings from knob labels when a page has none, and return the page
/// name (up to two sections joined by `" / "`).
fn infer_knob_sections(knobs: &mut [(deckard_plugins::ParamInfo, String, String)]) -> String {
    if knobs.is_empty() || knobs.iter().any(|(_, _, s)| !s.trim().is_empty()) {
        return String::new();
    }
    let labels: Vec<String> = knobs.iter().map(|(_, l, _)| l.trim().to_string()).collect();
    let two_word = |s: &str| -> Option<String> {
        let mut w = s.split_whitespace();
        let (a, b, _) = (w.next()?, w.next()?, w.next()?);
        let numbered = b.chars().all(|c| c.is_ascii_digit()) || (b.len() <= 3 && b.chars().next().is_some_and(|c| c.is_ascii_uppercase()));
        numbered.then(|| format!("{a} {b}"))
    };
    let mut cands: Vec<Option<String>> = Vec::with_capacity(labels.len());
    for (i, l) in labels.iter().enumerate() {
        if let Some((sec, _)) = l.split_once(" - ").or_else(|| l.split_once(": ")).or_else(|| l.split_once(" / ")) {
            let sec = sec.trim();
            if !sec.is_empty() {
                cands.push(Some(sec.to_string()));
                continue;
            }
        }
        if let Some((sec, _)) = l.split_once('-')
            && sec.len() >= 3
            && sec.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && sec.chars().last().is_some_and(|c| c.is_ascii_digit())
        {
            cands.push(Some(sec.to_string()));
            continue;
        }
        let words: Vec<&str> = l.split_whitespace().collect();
        if words.first().is_some_and(|w| w.eq_ignore_ascii_case("macro") || w.eq_ignore_ascii_case("macros")) {
            cands.push(Some("Macros".into()));
            continue;
        }
        if let Some(tw) = two_word(l)
            && labels.iter().enumerate().any(|(j, other)| j != i && two_word(other).as_deref() == Some(tw.as_str()))
        {
            cands.push(Some(tw));
            continue;
        }
        if words.len() >= 2 && words[0].len() >= 3 {
            let w0 = words[0];
            let neighbor_matches = [i.wrapping_sub(1), i + 1]
                .into_iter()
                .filter_map(|j| labels.get(j))
                .any(|other| other.split_whitespace().next().is_some_and(|ow| ow.eq_ignore_ascii_case(w0)));
            if neighbor_matches {
                cands.push(Some(w0.to_string()));
                continue;
            }
        }
        cands.push(None);
    }
    let mut sections: Vec<String> = Vec::new();
    let mut last = String::new();
    for (k, sec) in cands.into_iter().enumerate() {
        if let Some(sec) = sec {
            if !sec.eq_ignore_ascii_case(&last) {
                knobs[k].2 = sec.clone();
                last = sec.clone();
                if !sections.iter().any(|s| s.eq_ignore_ascii_case(&sec)) {
                    sections.push(sec);
                }
            }
        }
    }
    sections.into_iter().take(2).collect::<Vec<_>>().join(" / ")
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

/// A slot's hosted plug-in whose preset saves as NKS ([`App::nks_plugin`]).
struct NksPlugin {
    ids: deckard_plugins::nks::PluginIds,
    instrument: bool,
    plugin: deckard_model::PluginRef,
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
    use deckard_engine::devices::{self, AudioDriver};
    *fallback = None;
    let (out, no_asio) = devices::open_preferring(settings.audio_driver, |driver| {
        devices::set_driver(driver);
        if driver == AudioDriver::Asio {
            return Output::with_device(settings.asio_device.as_deref(), settings.buffer_frames, settings.multicore);
        }
        Output::with_device(settings.output_device.as_deref(), settings.buffer_frames, settings.multicore).or_else(|e| {
            if settings.output_device.is_some() {
                *fallback = Some(e);
                Output::with_options(settings.buffer_frames, settings.multicore)
            } else {
                Err(e)
            }
        })
    });
    // ASIO not being in use says more than which system device stood in.
    *fallback = no_asio.or(fallback.take());
    let out = out?;
    out.set_routing(&settings.output_routing, settings.cv_mask());
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
    inputs: std::sync::Arc<input_levels::InputBus>,
    rate: u32,
) -> Result<cpal::Stream, String> {
    use cpal::traits::StreamTrait;
    let (dev, config) = deckard_engine::devices::open_input(device.as_deref())?;
    if !deckard_engine::devices::format_ok(config.sample_format()) {
        return Err(format!("input format {:?} is not supported", config.sample_format()));
    }
    if config.sample_rate() != rate {
        return Err(format!("the input runs at {} Hz and the output at {rate} Hz", config.sample_rate()));
    }
    let channels = config.channels() as usize;
    let map: Vec<Option<usize>> = (0..32).map(|i| routing.get(i).copied().unwrap_or(Some(i)).filter(|&c| c < channels)).collect();
    // Room up front: on ASIO this callback runs on the output's thread.
    live.lock().unwrap().reserve(2 * 8192);
    let stream = deckard_engine::devices::build_input(
        &dev,
        config,
        move |data: &[f32]| {
            if let Ok(mut q) = live.try_lock() {
                for f in data.chunks(channels) {
                    q.push_back(inputs.frame(f, &map));
                }
                // A few blocks at most (nothing may be reading).
                while q.len() > 8192 {
                    q.pop_front();
                }
            }
        },
        |e| eprintln!("audio input error: {e}"),
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

/// A button of Preferences ▸ Plug-ins ▸ Windows plug-ins.
#[derive(Debug, Clone)]
enum WineAction {
    InstallWine(wine::Family),
    InstallYabridge,
    /// The packages a yabridge build needs, as root.
    InstallBuildTools(String),
    /// yabridge from its source (Wine 9.22 and later).
    BuildYabridge,
    Sync,
}

#[derive(Default)]
pub struct App {
    group_load: Option<preset_load::GroupLoad>,
    /// A non-group library load held back a couple of frames so the MK3 "loading" overlay is drawn first.
    item_load: Option<preset_load::ItemLoad>,
    /// The open project, shared with the controller scripts.
    doc: Option<Shared>,
    /// The project an Open replaced, with its undo history, and its file: Undo with
    /// nothing of the new one's to undo brings it back ([`App::back_to_replaced`]).
    replaced: Option<(Shared, Option<PathBuf>)>,
    /// The document revision playback last took up.
    seen_revision: u64,
    group_moves: group_move::State,
    /// Groups last seen, to notice one deleted (see [`App::clear_midi_inputs`]).
    midi_input_groups: usize,
    path: Option<PathBuf>,
    error: Option<String>,
    /// Focus: group, its pattern and sound, and the scene.
    group: usize,
    pattern: usize,
    sound: usize,
    /// SELECT's other sounds in the focused group (the focused one is always selected).
    sound_selection: Vec<usize>,
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
    lock_plugins: lock::HostedLocks,
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
    /// The scene the Ideas view's tabs are to scroll to, when next drawn.
    pub(crate) show_scenes: Option<usize>,
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
    /// Startup in progress, as a queue of steps (`None` once it is over).
    boot: Option<boot::Boot>,
    /// Test hook: what the AUDIO OUTPUT step returns (the rate, or why not), instead of opening one.
    #[cfg(test)]
    boot_output: Option<Result<f64, String>>,
    /// Test hook: the plug-ins the PLUG-INS step finds, instead of the project's.
    #[cfg(test)]
    boot_wanted: Option<Vec<(deckard_plugins::Slot, deckard_model::PluginRef)>>,
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
    /// The knobs being written as modulation, one per `(Chain, ModTarget)`.
    mod_takes: Vec<modulation::ModTake>,
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
    /// Preferences ▸ General ▸ Icon: each icon, drawn small, once it's been shown.
    icon_previews: Vec<(icon::Style, egui::TextureHandle)>,
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
    /// The Groove page (Swing, Variation, Generate) last shown on any sound: every
    /// sound's Groove shows it, and VARIATION goes back to it. `None` before one shows.
    groove_page: Option<usize>,
    /// Where each chain was left, shown again on coming back to it. `pages_of` is the
    /// chain the fields above were last kept for, `page_key` the slot `plugin_page` is for.
    chain_pages: std::collections::HashMap<deckard_doc::Chain, ChainPages>,
    pages_of: Option<deckard_doc::Chain>,
    page_key: Option<(deckard_doc::Chain, Option<usize>)>,
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
    /// The Maschine JAM over USB (started by `main`, not in tests), and its script while
    /// it's connected.
    jam: Option<jam_io::JamIo>,
    jam_device: Option<deckard_surface::device::Device>,
    /// The JAM's script wants the Settings page's values (its REC MODE readout).
    jam_settings_wanted: bool,
    /// The last JAM script error shown.
    jam_error: Option<String>,
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
    /// The inputs' levels and meters, shared with the input streams; and the meters as
    /// shown.
    input_bus: std::sync::Arc<input_levels::InputBus>,
    input_meters: [(f32, f32); input_levels::PAIRS],
    /// A controller shows the inputs' meters: the input stays open for them.
    input_watched: bool,
    /// The input was opened for the meters alone, with no engine started for it.
    input_for_meters: bool,
    settings_input_failed: Option<InputKey>,
    /// The page of the focused slot's parameters shown.
    plugin_page: usize,
    /// The live views of the hosted plug-ins shown (the focused slot's, and each strip
    /// panel drawn lately), by chain and slot.
    plugin_views: std::collections::HashMap<(deckard_doc::Chain, usize), PluginView>,
    /// The focused slot whose view `plugin_page` drives (as last updated).
    plugin_focus: Option<(deckard_doc::Chain, usize)>,
    /// The hosted plug-in panels drawn since the last update (drawing only reads `App`).
    plugin_drawn: std::cell::RefCell<Vec<(deckard_doc::Chain, usize)>>,
    /// Each plug-in's parameters (by chain and slot, with the plug-in's id), for the
    /// macros that control them.
    plugin_params: std::cell::RefCell<std::collections::HashMap<(deckard_doc::Chain, usize), (String, Vec<deckard_plugins::ParamInfo>)>>,
    /// Open the focused sound's plug-in window one second after start-up
    /// (`--plugin-window`).
    pending_editor: bool,
    /// Audio output, opened on first Play.
    output: Option<Result<Output, String>>,
    /// Why the output device picked couldn't be opened (the default plays instead), or
    /// why ASIO couldn't be had (the system's driver plays instead).
    output_fallback: Option<String>,
    /// The ASIO fallback the status line last told of; whether the driver asked for its
    /// streams to be opened afresh, and when they last were.
    fallback_told: Option<String>,
    asio_reopen_due: bool,
    asio_reopened: Option<std::time::Instant>,
    /// The stem separation model offered at start-up: the question, then the install.
    stems_offer: Option<StemsOffer>,
    /// Asking to quit NI's agent, which holds the MK3's screens.
    ni_agent: ni_agent::Ask,
    /// Asking to stop NI's services, which fight DECKARD over a JAM's lights.
    ni_services_ask: ni_agent::Ask,
    ni_services: ni_services::Watch,
    /// Windows plug-ins through Wine (Preferences ▸ Plug-ins, Linux): what's installed,
    /// the task running (what it is, where its result comes), the last task's message.
    wine_status: Option<wine::Status>,
    wine_task: Option<(&'static str, std::sync::mpsc::Receiver<Result<String, String>>)>,
    wine_message: String,
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
    /// The sound `zone` was last opened for: focusing another opens on its saved zone.
    zone_of: Option<(usize, usize)>,
    /// How many live notes [`App::follow_played_zone`] has looked at.
    zone_hit_seen: u64,
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
    /// Which of the clipboards was filled last: the one ⌘V pastes.
    last_copied: clipboard::Copied,
    /// Text for the system clipboard, put there at the end of the frame (see
    /// [`App::flush_clipboard`]).
    clipboard_text: Option<String>,
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
    take_info_key: Option<(usize, usize, usize, deckard_model::SampleRef)>,
    /// The focused group's Audio plug-in takes as the pattern editor draws them, by
    /// (group, sound, take): the file and frames they're of, and peaks (min, max) across
    /// the played frames.
    loop_peaks: std::collections::HashMap<(usize, usize, usize), (String, (i64, i64), std::sync::Arc<Vec<(f32, f32)>>)>,
    /// Bumped when `loop_peaks` changes; what the controller has seen.
    loop_peaks_gen: (u64, u64),
    /// Each recorded take's file, and the undo step its recording made (how many steps
    /// there were after it, and its label): DELETE undoes it while it's still the last.
    take_steps: std::collections::HashMap<PathBuf, (usize, String, usize)>,
    /// While a take records onto an Audio sound: how many undo steps there were before
    /// its old take was switched off (so a cancel, or DELETE, puts it back).
    rec_base: Option<usize>,
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
    /// Each plug-in extra output's strip's meter (as `project.extra_outs` lists them),
    /// falling back like the others.
    extra_meters: Vec<(f32, f32)>,
    /// Which strip each of `extra_meters` is (owner, bus, plug-in): when the list changes,
    /// the meters, peaks and headroom held by index start again.
    extra_meter_keys: Vec<(deckard_model::extra_out::Owner, usize, String, deckard_model::extra_out::Channels)>,
    cue_meter: (f32, f32),
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
    /// Just updated to this release: asks whether to open its release notes.
    whats_new: Option<String>,
    /// The About box is open.
    about: Option<about::State>,
    /// The update check and what it found.
    update: update::Notice,
    /// Quitting was confirmed: let the window close.
    quitting: bool,
    /// Where the open project is kept between runs, saved or not (see [`App::keep_session`]);
    /// `None` (tests): not kept.
    session_dir: Option<PathBuf>,
    /// The project and edit last kept there ([`App::doc_generation`], revision), and
    /// when the next edit is due to be kept.
    session_kept: (u64, u64),
    session_due: Option<std::time::Instant>,
    session_due_for: (u64, u64),
    /// The backup being written in the background, if any (see [`App::session_written`]).
    session_writing: Option<std::thread::JoinHandle<()>>,
    /// Goes up with each project opened or made.
    doc_generation: u64,
    /// Cached `(doc_generation, revision, macro_targets, group_noted)` for `surface_view`.
    surface_doc_cache: std::cell::RefCell<(u64, u64, std::collections::HashSet<u64>, Vec<std::collections::HashSet<usize>>)>,
    /// The focused Zhora (group, sound, slot) as last shown, while the audio thread has it.
    zhora_shown: std::cell::RefCell<Option<((usize, usize, usize), deckard_surface::ZhoraView)>>,
    /// The focused Tuner effect (chain, slot) as last shown, while the audio thread has it.
    tuner_shown: std::cell::RefCell<Option<((deckard_doc::Chain, usize), deckard_surface::TunerView)>>,
    /// When the Zhoras were last looked at for a finished Tune.
    tunes_watched: Option<std::time::Instant>,
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
    /// New screens, if they changed: their count and frame to rasterize on the display thread.
    pub screens: Option<(u64, mk3_io::ScreenFrame)>,
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

/// Coalesce a frame's 500 Hz MK3 reports so consecutive turns of the same knob add up
/// and intermediate touch-strip moves don't lag behind the finger.
fn coalesce_mk3_events(events: Vec<deckard_mk3::Event>) -> Vec<deckard_mk3::Event> {
    use deckard_mk3::Event;
    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    for (i, e) in events.iter().enumerate() {
        match *e {
            Event::Knob(k, inc) => {
                if let Some(prev_inc) = out.iter_mut().rev().take_while(|x| matches!(x, Event::Knob(..))).find_map(|x| match x {
                    Event::Knob(pk, pi) if *pk == k => Some(pi),
                    _ => None,
                }) {
                    *prev_inc += inc;
                    continue;
                }
                out.push(Event::Knob(k, inc));
            }
            Event::Touch(f, 1, _) => {
                let superseded = events[i + 1..].iter().find_map(|next| match *next {
                    Event::Touch(nf, 1, _) if nf == f => Some(true),
                    Event::Touch(nf, 0 | 2, _) if nf == f => Some(false),
                    _ => None,
                });
                if superseded == Some(true) {
                    continue;
                }
                out.push(*e);
            }
            _ => out.push(*e),
        }
    }
    out
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
    /// for the next save: it reopens there (BUGS #6), with the app's view and where each
    /// chain was left.
    pub(crate) fn store_screen(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        self.keep_pages();
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
            view: self.view_text(),
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
        self.apply_view_text(&s.view);
        // The slot and pages come back from where its chain was left.
        (self.slot_of, self.pages_of, self.page_key) = (None, None, None);
        let chain = self.chain();
        self.chain_pages.entry(chain).or_default().slot = self.slot;
        if !s.page.is_empty() {
            let view = self.surface_view();
            for a in self.surface.restore(&s.page, &s.mode, &view, std::time::Instant::now()) {
                self.surface_act(a);
            }
        }
    }

    /// Make `d` the open document (or show why it couldn't be opened).
    fn show_document(&mut self, d: Result<Document, String>) {
        self.replaced = None;
        self.leave_document();
        match d {
            Ok(mut d) => {
                // Pads play at the fixed velocity (100) unless FIXED VEL is turned off.
                let _ = d.set_initial(deckard_doc::Target::Song, "pad_velocity_mode", mxprj::decode::Value::Int(1));
                self.take_up(d.shared());
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Let go of what belongs to the open document (playback, caches, plug-in views).
    fn leave_document(&mut self) {
        self.zone_of = None;
        self.lock_morph = None;
        self.lock_active = false;
        self.lock_plugins = Default::default();
        modulation::register();
        self.stop();
        self.backed_up = false;
        self.lane_of = None;
        self.mod_takes.clear();
        // (The mod matrix page's slot being made was the old project's.)
        self.mod_draft = None;
        *self.mod_order.borrow_mut() = Default::default();
        // Where this project's sounds and groups were saved (Save asks again).
        self.saved.clear();
        self.editors.clear();
        // (The plug-in views were the old project's instances.)
        self.plugin_views.clear();
        self.plugin_focus = None;
        // (Where its chains were left was the old project's.)
        self.chain_pages.clear();
        (self.pages_of, self.page_key) = (None, None);
        self.samples = SampleCache::default();
        self.preload_plugins = deckard_plugins::on_main_thread();
        self.frames_drawn = 0;
    }

    /// Make `doc` the open document, where it was last left.
    fn take_up(&mut self, doc: Shared) {
        {
            self.doc_generation += 1;
            // What's copied keeps its expression in the new project (ids are the old one's).
            if let Some(old) = self.doc.clone().filter(|old| !std::rc::Rc::ptr_eq(old, &doc)) {
                let old = old.borrow();
                let mut d = doc.borrow_mut();
                if let Some(clip) = &mut self.note_clip {
                    for n in &mut clip.notes {
                        *n = d.carry_note(&old, *n);
                    }
                }
                self.sound_clipboard = std::mem::take(&mut self.sound_clipboard).into_iter().map(|c| c.carried(&old, &mut d)).collect();
            }
            if let Some(dir) = self.controller_scripts() {
                self.controller = Some(controller::Controller::new(&dir, &self.model, Some(doc.clone())));
            }
            // Projects with an arrangement open in the Song view, others in Ideas.
            self.song_view = !doc.borrow().project().sections.is_empty();
            self.mixer_sounds = doc.borrow().project().mixer_sounds;
            self.headroom.clear();
            self.seen_revision = doc.borrow().revision();
            self.clear_midi_inputs();
            self.midi_input_groups = doc.borrow().project().groups.len();
            self.doc = Some(doc);
            self.error = None;
            (self.group, self.pattern, self.sound, self.scene) = (0, 0, 0, 0);
            self.section = None;
            self.focus_scene(0);
            self.restore_screen();
            self.sync_controller();
        }
    }

    /// Undo past the start of the open project's history: the project an Open replaced
    /// comes back, as it was left (false: none, or the open one has unsaved changes).
    fn back_to_replaced(&mut self) -> bool {
        if self.dirty() {
            return false;
        }
        let Some((doc, path)) = self.replaced.take() else { return false };
        self.leave_document();
        self.take_up(doc);
        self.path = path;
        let name = self.path.as_ref().and_then(|p| p.file_stem()).map_or("the last project".into(), |n| n.to_string_lossy().to_string());
        self.status = format!("back to {name}");
        true
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
                self.clipboard_text = Some(c.name.clone());
                self.sound_clipboard = vec![c];
                self.last_copied = clipboard::Copied::Sound;
            }),
            Act::CutSound(g, s) => {
                let clip = doc.borrow().copy_sound(g, s);
                clip.and_then(|c| {
                    self.clipboard_text = Some(c.name.clone());
                    self.sound_clipboard = vec![c];
                    self.last_copied = clipboard::Copied::Sound;
                    let mut d = doc.borrow_mut();
                    d.reset_sound(g, s)?;
                    d.relabel_last("Cut Sound");
                    drop(d);
                    self.samples.plugins.retain(|slot| {
                        !matches!(
                            slot,
                            deckard_plugins::Slot::Sound { group, sound }
                                | deckard_plugins::Slot::Chain { group, sound, .. }
                                if *group == g && *sound == s
                        )
                    });
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
            Act::MoveGroup(g, to) => self.move_group(g, to),
            Act::ResetSound(g, s) => {
                let r = doc.borrow_mut().reset_sound(g, s);
                if r.is_ok() {
                    self.samples.plugins.retain(|slot| {
                        !matches!(
                            slot,
                            deckard_plugins::Slot::Sound { group, sound }
                                | deckard_plugins::Slot::Chain { group, sound, .. }
                                if *group == g && *sound == s
                        )
                    });
                }
                r
            }
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
        self.clipboard_text = doc.borrow().project().groups.get(g).map(|x| x.name.clone());
        self.last_copied = clipboard::Copied::Group;
        self.status = "copied the group".into();
        Ok(())
    }

    /// Delete group `g` (the last: reset), the focus kept on the group it was on, or
    /// the one before.
    fn delete_group(&mut self, g: usize) -> Result<(), String> {
        let Some(doc) = self.doc.clone() else { return Ok(()) };
        doc.borrow_mut().delete_group(g)?;
        self.samples.plugins.retain(|slot| {
            !matches!(
                slot,
                deckard_plugins::Slot::Sound { group, .. }
                    | deckard_plugins::Slot::Chain { group, .. }
                    | deckard_plugins::Slot::Group { group, .. }
                    if *group == g
            )
        });
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

    /// A note played live on the focused sound shows the zone it lands in, on the
    /// Sampler's pages and the sampling page alike. (Not the sequence's: the page being
    /// edited shouldn't change under you as the song plays.)
    fn follow_played_zone(&mut self) {
        let Some(Ok(out)) = &self.output else { return };
        let (count, group, sound, pitch, velocity) = out.telemetry(|t| t.live_hit);
        if count == self.zone_hit_seen {
            return;
        }
        self.zone_hit_seen = count;
        self.open_saved_zone();
        if (group, sound) != (self.group, self.sound) {
            return;
        }
        let Some(doc) = &self.doc else { return };
        let doc = doc.borrow();
        let smp = doc.project().groups.get(group).and_then(|g| g.sounds.get(sound)).and_then(|s| s.sampler.as_ref());
        if let Some(zone) = smp.and_then(|smp| layout::sampling::zone_for_note(&smp.zones, self.zone, pitch, velocity)) {
            self.zone = zone;
        }
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
        self.remember_libraries();
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
        drop(d);
        // One at a time, in order: an older backup never lands over a newer one.
        self.session_written();
        let write = move || {
            let _ = std::fs::create_dir_all(&dir);
            for (name, data) in [("session.mxprj", bytes), ("session.txt", info.into_bytes())] {
                let tmp = dir.join(format!("{name}.saving"));
                if std::fs::write(&tmp, data).is_ok() {
                    let _ = std::fs::rename(&tmp, dir.join(name));
                }
            }
        };
        if clean {
            write();
        } else {
            self.session_writing = Some(std::thread::spawn(write));
        }
    }

    /// Wait for the backup being written in the background, if there is one.
    fn session_written(&mut self) {
        if let Some(w) = self.session_writing.take() {
            let _ = w.join();
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
        let at = match self.session_due {
            Some(at) if self.session_due_for == now || self.session_due_for == (0, 0) => {
                self.session_due_for = now;
                at
            }
            _ => {
                let at = std::time::Instant::now() + SESSION_DELAY;
                self.session_due = Some(at);
                self.session_due_for = now;
                at
            }
        };
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
            self.open_replacing(&path);
        }
    }

    /// Open `path` in place of the open project, which Undo can bring back (see
    /// [`App::back_to_replaced`]), unsaved changes and all.
    fn open_replacing(&mut self, path: &Path) {
        // (It comes back where it was left.)
        self.store_screen();
        let old = self.doc.clone().map(|d| (d, self.path.clone()));
        self.open(path);
        if self.error.is_none() {
            self.replaced = old;
            self.remember(path);
        }
    }

    /// Unsaved changes on quitting are asked about (Save / Discard), as Maschine does.
    fn asks_on_quit(&self) -> bool {
        self.dirty()
    }

    fn quit(&mut self, ctx: &egui::Context) {
        // A Tune just finished counts as a change.
        self.store_finished_tunes(|_| true, true);
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
            // Changes were saved or discarded: open without asking again.
            Pending::Open(p) => self.open_replacing(&p),
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
                self.lock_end_morph();
                if let Some(doc) = self.doc.clone() {
                    let ok = doc.borrow_mut().undo_step();
                    if !ok && !self.back_to_replaced() {
                        self.status = "nothing to undo".into();
                    }
                }
                self.lock_hosted_sync();
            }
            C::RedoStep => {
                self.lock_end_morph();
                if let Some(doc) = &self.doc
                    && !doc.borrow_mut().redo_step()
                {
                    self.status = "nothing to redo".into();
                }
                self.lock_hosted_sync();
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
                // The ni-tech-manuals copy is a private bucket, so the browser shows an XML
                // access error. This is the manual Native Instruments still publishes.
                let url = "https://docs.native-instruments.com/online-guides/maschine-software-manual/en/index-en";
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
            C::DeckardManual => ctx.open_url(egui::OpenUrl::new_tab("https://panicsignals.tv/software/deckard/manual/")),
            C::Splice => splice::open(ctx, None),
            C::About => self.about = Some(about::State::open()),
            C::Preferences => self.prefs = Some(self.prefs.unwrap_or(0)),
            C::Mixer => self.set_mixer(!self.mixer),
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
        self.audio_driver_preferences(ui);
        let rate = match &self.output {
            Some(Ok(o)) => o.rate as f64,
            _ => 48_000.0,
        };
        let ms = |n: u32| n as f64 / rate * 1000.0;
        let mut size = self.settings.buffer_frames;
        let mut restart = false;
        egui::Grid::new("audio-engine").num_columns(2).show(ui, |ui| {
            ui.label("Buffer size");
            crate::layout::combo("buffer-size").selected_text(format!("{size} samples ({:.1} ms)", ms(size))).show_ui(ui, |ui| {
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
            crate::layout::combo("render-ahead").selected_text(shown(ahead)).show_ui(ui, |ui| {
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

    /// Preferences > Audio on Windows: the audio system to play through, and what it's
    /// doing. A build without ASIO just says which it is.
    fn audio_driver_preferences(&mut self, ui: &mut egui::Ui) {
        use deckard_engine::devices::{ASIO_BUILT, AudioDriver};
        if !cfg!(windows) {
            return;
        }
        let wanted = self.settings.audio_driver;
        let mut pick = wanted;
        egui::Grid::new("audio-driver").num_columns(2).show(ui, |ui| {
            ui.label("Driver");
            if ASIO_BUILT {
                crate::layout::combo("audio-driver-pick").selected_text(pick.name()).show_ui(ui, |ui| {
                    for d in [AudioDriver::System, AudioDriver::Asio] {
                        ui.selectable_value(&mut pick, d, d.name());
                    }
                });
            } else {
                ui.label(AudioDriver::System.name());
            }
            ui.end_row();
            if let Some(Ok(out)) = &self.output
                && out.driver == AudioDriver::Asio
            {
                let latency = out.latency().as_secs_f64() * 1000.0;
                ui.label("");
                ui.label(format!("{}: {} Hz, {:?}, {latency:.1} ms from a block's start to the outputs, as the driver says", out.device, out.rate, out.format));
                ui.end_row();
                ui.label("");
                ui.label(
                    egui::RichText::new(
                        "The driver has the last word on the buffer size and the rate. If it won't take the size below, set it in its own control panel: DECKARD follows.",
                    )
                    .weak(),
                );
                ui.end_row();
            }
        });
        if pick != wanted {
            self.settings.set_audio_driver(pick);
            (self.inputs, self.outputs) = (None, None);
            self.channel_names.clear();
            self.close_output();
            if pick == AudioDriver::Asio {
                deckard_engine::devices::rescan();
            }
            self.open_output_now();
        }
        ui.separator();
    }

    /// Close the audio output and whatever else holds the driver: playback stops.
    fn close_output(&mut self) {
        self.stop();
        self.output = None;
        self.midi_wired = false;
        self.live_input = None;
        if deckard_engine::devices::driver() == deckard_engine::devices::AudioDriver::Asio {
            self.recorder.release_input();
        }
    }

    /// Open the audio output (with the current settings), saying so.
    fn open_output_now(&mut self) {
        let _ = self.output.get_or_insert_with(|| open_output(&self.settings, &mut self.output_fallback));
        self.status = match &self.output {
            Some(Ok(out)) if out.driver == deckard_engine::devices::AudioDriver::Asio => {
                format!("audio: ASIO, {}, {}", out.device, if self.settings.multicore { "multi-core" } else { "one core" })
            }
            _ => format!(
                "audio: {} samples, {}",
                self.settings.buffer_frames,
                if self.settings.multicore { "multi-core" } else { "one core" }
            ),
        };
    }

    /// Close the audio output and open it again (with the current settings): playback
    /// stops; MIDI and the live input are wired to the new one.
    fn restart_output(&mut self) {
        self.close_output();
        self.open_output_now();
    }

    /// What ASIO asks of the app, off its callbacks, each pass: to have its absence told,
    /// and to be opened afresh with what the driver is set to now (its control panel is
    /// the usual reason). A driver that keeps asking is only obeyed now and then, so it
    /// gets to play in between.
    fn watch_audio_driver(&mut self) {
        use deckard_engine::devices::{self, AudioDriver};
        let asio_wanted = self.settings.audio_driver == AudioDriver::Asio;
        if asio_wanted && self.output_fallback != self.fallback_told {
            self.fallback_told = self.output_fallback.clone();
            if let Some(why) = &self.fallback_told {
                self.status = format!("audio: ASIO isn't in use ({why}); playing through {}", AudioDriver::System.name());
            }
        }
        if let Some(Ok(out)) = &self.output
            && out.take_invalidated()
        {
            self.asio_reopen_due = true;
        }
        let now = std::time::Instant::now();
        if !self.asio_reopen_due || self.asio_reopened.is_some_and(|t| now.duration_since(t) < ASIO_REOPEN_GAP) {
            return;
        }
        self.asio_reopen_due = false;
        self.asio_reopened = Some(now);
        self.close_output();
        devices::rescan();
        self.open_output_now();
    }

    fn audio_preferences(&mut self, ui: &mut egui::Ui) {
        use deckard_engine::devices::{self, ASIO_BUILT, AudioDriver};
        self.audio_engine_preferences(ui);
        ui.separator();
        let wanted = self.settings.audio_driver.usable();
        let asio = wanted == AudioDriver::Asio;
        let inputs = self.inputs.get_or_insert_with(|| devices::input_devices_of(wanted)).clone();
        let outputs = self.outputs.get_or_insert_with(|| devices::output_devices_of(wanted)).clone();
        // (ASIO has no default, and listing what plays when it's away isn't what's asked.)
        let (default_in, default_out) =
            if asio { (None, None) } else { (recorder::default_input(), deckard_engine::output::default_output()) };
        let default_label = |d: &Option<String>| {
            if asio { "None picked".to_string() } else { format!("System default ({})", d.as_deref().unwrap_or("none")) }
        };
        let (mut input, mut output) = if asio {
            (self.settings.asio_device.clone(), self.settings.asio_device.clone())
        } else {
            (self.settings.input_device.clone(), self.settings.output_device.clone())
        };
        egui::Grid::new("audio-interface").num_columns(3).show(ui, |ui| {
            // The same ASIO driver serves input and output.
            let rows = if asio {
                vec![("ASIO driver", &mut output, &outputs, &default_out, "out")]
            } else {
                vec![
                    ("Output device", &mut output, &outputs, &default_out, "out"),
                    ("Input device", &mut input, &inputs, &default_in, "in"),
                ]
            };
            for (label, pick, devices, default, kind) in rows {
                ui.label(label);
                let shown = pick.clone().unwrap_or_else(|| default_label(default));
                crate::layout::combo(label).selected_text(shown).width(320.0).show_ui(ui, |ui| {
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
                    let text = if self.settings.audio_driver == AudioDriver::Asio {
                        format!("ASIO isn't in use: {why}. Playing through {}.", AudioDriver::System.name())
                    } else {
                        format!("can't be opened: {why}. Playing through the system default.")
                    };
                    ui.label(egui::RichText::new(text).color(trouble));
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
            ui.label("");
            if ui.button("Rescan").clicked() {
                (self.inputs, self.outputs) = (None, None);
                self.channel_names.clear();
                if asio {
                    devices::rescan();
                    // Asked for, and not what plays (the driver may be free now): try again.
                    if matches!(&self.output, Some(Ok(o)) if o.driver != AudioDriver::Asio) {
                        self.restart_output();
                    }
                }
            }
            ui.end_row();
        });
        // Another device: its own routing comes back.
        if asio {
            if output != self.settings.asio_device {
                self.settings.set_asio_device(output);
                self.restart_output();
            }
        } else {
            if input != self.settings.input_device {
                self.settings.set_input_device(input);
            }
            if output != self.settings.output_device {
                self.settings.set_output_device(output);
                self.restart_output();
            }
        }
        if ASIO_BUILT && self.settings.audio_driver == AudioDriver::Asio && self.settings.asio_device.is_none() {
            ui.label(egui::RichText::new("Pick the ASIO driver of your interface.").weak());
        }
        let channels_of =
            |devices: &[(String, usize)], d: &Option<String>| devices.iter().find(|(n, _)| Some(n) == d.as_ref()).map_or(0, |(_, c)| *c);
        let (in_device, out_device) = if asio {
            (self.settings.asio_device.clone(), self.settings.asio_device.clone())
        } else {
            (self.settings.input_device.clone().or(default_in), self.settings.output_device.clone().or(default_out))
        };
        let (in_channels, out_channels) = (channels_of(&inputs, &in_device), channels_of(&outputs, &out_device));
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Input level").on_hover_text(
                "In 1's and In 2's levels on the way in: what sounds taking them and the recorder hear (a Maschine JAM's IN1 and IN2)",
            );
            for input in 0..input_levels::PAIRS {
                ui.label(format!("In {}", input + 1));
                let mut db = deckard_model::level_db(self.settings.input_levels[input]).clamp(-60.0, 10.0);
                let drag = egui::DragValue::new(&mut db).range(-60.0..=10.0).speed(0.1).fixed_decimals(1).suffix(" dB");
                if ui.add(drag).changed() {
                    self.settings.input_levels[input] = deckard_model::level_from_db(db);
                    self.input_bus.set_levels(&self.settings.input_levels);
                    self.settings.save();
                }
            }
        });
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
        let before = (self.settings.output_routing.clone(), self.settings.output_cv_only.clone());
        let project = self.doc.as_ref().map(|d| d.borrow().project().clone()).unwrap_or_default();
        let (own, port, channels) = if outs { ("Out", "Output", out_channels) } else { ("In", "Input", in_channels) };
        let device = if outs { out_device } else { in_device };
        let names = self.channel_names_of(&device.unwrap_or_default(), !outs, channels);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("routing").striped(true).num_columns(if outs { 4 } else { 3 }).show(ui, |ui| {
                ui.strong("DECKARD");
                ui.strong("Alias");
                ui.strong("Interface");
                if outs {
                    ui.strong("CV only");
                }
                ui.end_row();
                let may: Vec<bool> = (0..settings::CHANNELS).map(|i| self.settings.may_mark_cv_only(&project, i)).collect();
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
                    crate::layout::combo(("route", outs, i)).selected_text(text(cur)).width(200.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut pick, None, text(None));
                        for c in 0..channels.max(cur.map_or(0, |c| c + 1)) {
                            ui.selectable_value(&mut pick, Some(c), text(Some(c)));
                        }
                    });
                    routes[i] = pick;
                    if outs {
                        let marked = s.output_cv_only[i];
                        ui.add_enabled_ui(may[i] || marked, |ui| {
                            ui.checkbox(&mut s.output_cv_only[i], "")
                                .on_hover_text("This channel carries control voltage, not audio, so audio isn't offered it.");
                        })
                        .response
                        .on_disabled_hover_text("The project's master or cue output plays on this channel.");
                    }
                    ui.end_row();
                }
            });
        });
        if (&self.settings.output_routing, &self.settings.output_cv_only) != (&before.0, &before.1)
            && let Some(Ok(out)) = &self.output
        {
            out.set_routing(&self.settings.output_routing, self.settings.cv_mask());
        }
    }

    /// Preferences > MIDI: the input and output ports DECKARD uses (each can be
    /// switched off), and what sounds set to the Default MIDI source take.
    fn midi_preferences(&mut self, ui: &mut egui::Ui) {
        self.sync_preferences(ui);
        ui.checkbox(&mut self.settings.jam_enabled, "Maschine JAM").on_hover_text(
            "DECKARD drives a JAM plugged in over USB. Switch it off when Maschine should have the JAM to itself: with both running, each acts on every press and they overwrite each other's lights.",
        );
        if ni_services::SUPPORTED {
            let seen = self.ni_services.seen;
            ui.horizontal(|ui| {
                ui.label(if seen.services { "NI's services: running" } else { "NI's services: stopped" }).on_hover_text(
                    "They light a JAM too, so its pads turn orange and its lights flash while DECKARD drives it. Maschine needs them running.",
                );
                if seen.services {
                    if ui.add_enabled(!seen.maschine, egui::Button::new("Stop")).on_hover_text("Asks for administrator approval. Not while Maschine runs.").clicked() {
                        ni_services::start_or_stop(false);
                    }
                } else if ui.button("Start").on_hover_text("For Maschine. Asks for administrator approval.").clicked() {
                    ni_services::start_or_stop(true);
                }
            });
        }
        ui.checkbox(&mut self.settings.focus_follows, "Focus the group MIDI is recorded into").on_hover_text(
            "While recording, MIDI going into another group makes it the focused group, and so does picking a pattern on a JAM's Project View. Off: the focus stays where you put it, and the MIDI still records into its group and the pattern still plays.",
        );
        ui.separator();
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
                    crate::layout::combo(("drum-channel", &id)).selected_text(text(ch)).show_ui(ui, |ui| {
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
                    crate::layout::combo(("drum-track", &id))
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
        egui::Grid::new("general-prefs").num_columns(2).spacing([20.0, 12.0]).show(ui, |ui| {
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
            ui.label("Icon");
            self.icon_choice(ui);
            ui.end_row();
            ui.label("Updates");
            ui.checkbox(&mut self.settings.check_updates, "Check for updates at launch")
                .on_hover_text("Asks GitHub for DECKARD's latest release once per launch. Help ▸ About shows what it found.");
            ui.end_row();
            ui.label("");
            ui.label(egui::RichText::new("File ▸ New starts from it: its groups, sounds, plug-ins, tempo and routing, untitled.").weak());
            ui.end_row();
        });
    }

    /// The icons to pick from, each drawn as it looks: the window, the taskbar and the Dock
    /// show the one picked, at once.
    fn icon_choice(&mut self, ui: &mut egui::Ui) {
        if self.icon_previews.is_empty() {
            self.icon_previews = icon::Style::ALL
                .into_iter()
                .map(|s| {
                    let n = 128;
                    let image = egui::ColorImage::from_rgba_unmultiplied([n, n], s.pixels(n).as_flattened());
                    (s, ui.ctx().load_texture(format!("app-icon-{}", s.key()), image, egui::TextureOptions::LINEAR))
                })
                .collect();
        }
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                for (style, texture) in &self.icon_previews {
                    let on = self.settings.app_icon == *style;
                    let picture = egui::Image::new((texture.id(), egui::vec2(56.0, 56.0)));
                    let r = ui
                        .vertical_centered(|ui| {
                            ui.set_width(72.0);
                            let r = ui.add(egui::Button::image(picture).selected(on).frame(true));
                            ui.label(style.label());
                            r
                        })
                        .inner;
                    if r.clicked() && !on {
                        self.settings.app_icon = *style;
                        icon::apply(ui.ctx(), *style);
                    }
                }
            });
            ui.label(
                egui::RichText::new(if cfg!(windows) {
                    "The window and the taskbar show it. Shortcuts keep the icon they were installed with."
                } else if cfg!(target_os = "macos") {
                    "The Dock shows it while DECKARD runs; Finder keeps the app's own."
                } else {
                    "The window and the taskbar show it."
                })
                .weak(),
            );
        });
    }

    fn preferences(&mut self, ctx: &egui::Context) {
        let Some(mut tab) = self.prefs else { return };
        let mut open = true;
        let before = self.settings.clone();
        let mut rescan = false;
        let mut library_change = None;
        let mut controller_change = None;
        let mut wine_action = None;
        egui::Window::new("Preferences").collapsible(false).resizable(true).default_size([760.0, 700.0]).open(&mut open).show(ctx, |ui| {
            crate::layout::dialog_style(ui);
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
                egui::Grid::new("recording-prefs").num_columns(2).spacing([20.0, 12.0]).show(ui, |ui| {
                    ui.label("Count-in");
                    crate::layout::combo("count-in")
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
                egui::Grid::new("hardware-prefs").num_columns(2).spacing([20.0, 12.0]).show(ui, |ui| {
                    ui.label("Pad sensitivity");
                    let mut n = self.settings.pad_sensitivity;
                    ui.add(egui::Slider::new(&mut n, 0..=100).suffix(" %"));
                    self.settings.pad_sensitivity = n;
                    ui.end_row();
                    ui.label("");
                    ui.label(egui::RichText::new("How hard the MK3's pads must be pressed: higher, gentle presses play louder.").weak());
                    ui.end_row();
                    if cfg!(windows)
                        && let Some(mk3) = &self.mk3
                    {
                        ui.label("Controller screens");
                        // Both drivers stay in the list. Hiding the one that is not open
                        // left no way back, and a restart did not put it back either.
                        let switching = matches!(self.ni_agent, ni_agent::Ask::Quitting(_));
                        let in_use = ni_agent::screen_driver_in_use(mk3.displays_released(), mk3.displays_connected(), switching);
                        let selected = in_use.map(ni_agent::ScreenDriver::label).unwrap_or("Not open");
                        let mut picked = None;
                        crate::layout::combo("screen-driver").selected_text(selected).show_ui(ui, |ui| {
                            for driver in [ni_agent::ScreenDriver::Windows, ni_agent::ScreenDriver::Maschine] {
                                let hover = match driver {
                                    ni_agent::ScreenDriver::Windows => "DECKARD draws the screens, using Windows' WinUSB driver. Asks for administrator approval.",
                                    ni_agent::ScreenDriver::Maschine => "Clears the screens and gives them to Maschine. Asks for administrator approval.",
                                };
                                if ui.selectable_label(in_use == Some(driver), driver.label()).on_hover_text(hover).clicked() {
                                    picked = Some(driver);
                                }
                            }
                        });
                        if let Some(driver) = picked {
                            match driver {
                                ni_agent::ScreenDriver::Windows if in_use != Some(driver) => {
                                    mk3.release_displays(false);
                                    ni_agent::use_windows(&mut self.ni_agent, &mut self.status);
                                }
                                ni_agent::ScreenDriver::Maschine if in_use != Some(driver) => {
                                    mk3.release_displays(true);
                                    ni_agent::use_maschine(&mut self.ni_agent, &mut self.status);
                                }
                                _ => {}
                            }
                        }
                        ui.end_row();
                        ui.label("");
                        ui.label(egui::RichText::new("Windows screen driver lets DECKARD draw the screens. Maschine screen driver clears them and gives them back to Maschine.").weak());
                        ui.end_row();
                    }
                    ui.label("Controller");
                    use settings::ControllerChoice as C;
                    let mut choice = self.settings.controller;
                    crate::layout::combo("controller-layer")
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
                    crate::layout::combo("controller-theme").selected_text(theme_name(&theme)).show_ui(ui, |ui| {
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
                    ui.label("Maschine JAM");
                    ui.checkbox(&mut self.settings.relative_strips, "Relative level touchstrips").on_hover_text(
                        "On the mixer strips (LEVEL, AUX, SWING, TUNE, MACRO), a touch no longer sets the value where the finger lands, and a double tap no longer resets it: moving the finger changes the value by the distance moved, from where it was. CONTROL mode keeps absolute-fader behaviour. Off: a touch sets the value.",
                    );
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
                ui.label(egui::RichText::new("VST2 plug-ins are listed but can't be hosted yet.").weak());
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.catalog.scanning(), egui::Button::new("Rescan")).clicked() {
                        rescan = true;
                    }
                    ui.label(egui::RichText::new(&self.catalog.status).weak());
                });
                if cfg!(target_os = "linux") {
                    self.wine_section(ui, &mut wine_action);
                }
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
                for d in settings::Settings::standard_dirs() {
                    ui.horizontal(|ui| {
                        ui.add_enabled(false, egui::Button::new("Standard").small());
                        let exists = d.exists();
                        ui.label(egui::RichText::new(d.display().to_string()).color(if exists {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::GRAY
                        }))
                        .on_hover_text(if exists { "A standard folder; always searched" } else { "A standard folder; it doesn't exist here" });
                    });
                }
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
                    if ui.add_enabled(!self.settings.dirs.is_empty(), egui::Button::new("Remove Added")).clicked() {
                        self.settings.dirs.clear();
                    }
                    if ui.add_enabled(!self.catalog.scanning(), egui::Button::new("Rescan")).clicked() {
                        rescan = true;
                    }
                });
            }
        });
        if self.settings != before {
            self.settings.save();
            deckard_plugins::module::set_search_paths(self.settings.scan_dirs());
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
        if !open {
            // Looked at again when the window next opens.
            self.wine_status = None;
        }
        if let Some(a) = wine_action {
            self.start_wine(a);
        }
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
                    crate::layout::dialog_style(ui);
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
                        crate::layout::dialog_style(ui);
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

    /// At launch: a release newer than the last one run was just updated to, so offer
    /// its release notes; and remember this one.
    fn after_update(&mut self, version: &str) {
        if !RELEASE_BUILD && !cfg!(test) {
            return;
        }
        if update::just_updated(&self.settings.last_version, version) {
            self.whats_new = Some(version.to_string());
        }
        if self.settings.last_version != version {
            self.settings.last_version = version.to_string();
            #[cfg(not(test))]
            self.settings.save();
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(version) = self.whats_new.clone() {
            let mut choice = None;
            egui::Modal::new(egui::Id::new("whats-new")).show(ctx, |ui| {
                crate::layout::dialog_style(ui);
                ui.set_width(360.0);
                ui.label(egui::RichText::new(format!("DECKARD was updated to {version}")).strong());
                ui.add_space(4.0);
                ui.label("Would you like to see the release notes?");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("See Release Notes").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Not Now").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        choice = Some(false);
                    }
                });
            });
            if let Some(open) = choice {
                if open {
                    ctx.open_url(egui::OpenUrl::new_tab(update::release_notes_url(&version)));
                }
                self.whats_new = None;
            }
        }
        if let Some(then) = self.confirm.clone() {
            let name = self.path.as_ref().and_then(|p| p.file_stem()).map_or("the project".into(), |n| n.to_string_lossy().into_owned());
            let question = match then {
                Pending::Revert => format!("Discard all changes to \"{name}\" since it was last saved?"),
                _ => format!("Save changes to \"{name}\"?"),
            };
            let mut choice = None;
            egui::Modal::new(egui::Id::new("unsaved-changes")).show(ctx, |ui| {
                crate::layout::dialog_style(ui);
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
        let holding = self.mk3.as_ref().is_some_and(mk3_io::Mk3Io::agent_holding);
        if let Some(failure) = ni_agent::take_failure() {
            self.status = failure;
            self.ni_agent = ni_agent::Ask::Declined;
        }
        ni_agent::dialog(ctx, &mut self.ni_agent, holding, &mut self.status, ni_agent::quit);
        // A JAM connected with NI's services running: they fight over its lights.
        if let Some(failure) = ni_services::take_failure() {
            self.status = failure;
            self.ni_services_ask = ni_agent::Ask::Declined;
        }
        let connected = self.jam_device.is_some();
        if connected || self.prefs.is_some() {
            let seen = self.ni_services.look(std::time::Instant::now());
            let offer = seen.offer(connected);
            ni_agent::ask_about(
                "ni-services",
                &ni_services::WORDS,
                ctx,
                &mut self.ni_services_ask,
                offer,
                &mut self.status,
                ni_services::stop,
            );
        }
        self.preferences(ctx);
        if self.about.is_some() && !about::show(self, ctx) {
            self.about = None;
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
        self.remember_libraries();
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
        if !report.missing.is_empty() {
            status += &format!("; {} samples not found", report.missing.len());
        }
        if let Some(note) = layout::silent_master_note(&self.settings, project) {
            status += &format!("; {note}");
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
    /// What Edit ▸ MIDI Change picked: as clicking it. Past the last, nothing.
    pub(crate) fn midi_recall(&mut self, r: midi_io::Recall) {
        let Some(doc) = self.doc.clone() else { return };
        let (scenes, sections) = {
            let d = doc.borrow();
            (d.project().scenes.len(), d.project().sections.len())
        };
        match r {
            midi_io::Recall::Scene(s) if s < scenes => self.launch_scene(s),
            midi_io::Recall::Section(s) if s < sections => self.launch_section(s),
            _ => {}
        }
    }

    /// Whether scenes can play at their own tempos: not when a clock sets the tempo.
    pub(crate) fn scene_tempo_available(&self) -> bool {
        !self.settings.link && self.settings.midi_sync != settings::MidiSync::Receive
    }

    /// The scene playing now, if it has its own tempo, and that tempo.
    pub(crate) fn playing_scene_tempo(&self, p: &deckard_model::Project) -> Option<(usize, f64)> {
        if !self.scene_tempo_available() {
            return None;
        }
        let head = self.playhead()?;
        let s = match head.mode {
            Mode::Scene(s) => s,
            Mode::Arrangement => p.section_at(head.position as i64).and_then(|i| p.sections[i].scene)?,
            _ => return None,
        };
        Some((s, p.scenes.get(s)?.tempo?))
    }

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
            self.focus_sound(0);
            self.pattern = self.scene_pattern(group).unwrap_or(0);
        }
    }

    /// Focus a sound. A plain focus replaces SELECT's set with that one sound.
    fn focus_sound(&mut self, sound: usize) {
        self.sound = sound;
        self.sound_selection.clear();
    }

    fn group_len(&self) -> usize {
        self.doc.as_ref().and_then(|d| d.borrow().project().groups.get(self.group).map(|g| g.sounds.len())).unwrap_or(0)
    }

    /// SELECT + a pad. MULTI off replaces the set, or only moves the focus when the pad
    /// is already selected. MULTI on adds a pad, or removes one that isn't the focus.
    fn select_pad(&mut self, index: usize, multi: bool) {
        let n = self.group_len();
        if index >= n {
            return;
        }
        let selected = index == self.sound || self.sound_selection.contains(&index);
        if multi {
            if selected {
                if index != self.sound {
                    self.sound_selection.retain(|s| *s != index);
                }
            } else {
                self.sound_selection.push(index);
            }
        } else if selected {
            if index != self.sound && !self.sound_selection.contains(&self.sound) {
                self.sound_selection.push(self.sound);
            }
            self.sound = index;
        } else {
            self.sound = index;
            self.sound_selection.clear();
        }
    }

    /// SELECT + ALL: every sound of the focused group, focus unchanged.
    fn select_all_sounds(&mut self) {
        let n = self.group_len();
        self.sound_selection = (0..n).filter(|&s| s != self.sound).collect();
    }

    /// SELECT + NONE: only the focused sound stays selected.
    fn select_no_sounds(&mut self) {
        self.sound_selection.clear();
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

    /// Where playback is in its bar: what the touch strip tracks with no pattern to
    /// follow (an empty scene runs a bar at a time, and Sync starts on bars).
    fn bar_at(&self, head: Option<Playhead>) -> Option<(i64, i64)> {
        let head = head?;
        let bar = self.doc.as_ref()?.borrow().project().bar_length().max(1);
        Some(((head.position as i64).rem_euclid(bar), bar))
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
                Act::NewPattern(s, g) | Act::NewPatternAt(s, g, _) => {
                    let slot = if let Act::NewPatternAt(.., slot) = act { Some(slot) } else { None };
                    let n = d.project().groups.get(g).map_or(0, |x| x.patterns.len());
                    // Like the focused pattern (its colour), or the group's first; the
                    // Settings page's LENGTH long.
                    let from = if g == self.group { self.pattern } else { 0 };
                    let mut made = 0;
                    let length = self.default_pattern_length(d.project());
                    let r = d.transaction("New Pattern", |d| {
                        made = match slot {
                            Some(slot) => d.add_pattern_in_slot(g, from, slot, &format!("Pattern {}", slot + 1))?,
                            None => d.add_pattern(g, from, &format!("Pattern {}", n + 1))?,
                        };
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
                Act::MoveAudio { g, p, s, offset } => d.set_audio_offset(g, p, s, offset, "Move Audio").map(|_| None),
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
                    if add.is_empty() && d.project().groups.get(g).is_none_or(|gr| p >= gr.patterns.len()) {
                        return;
                    }
                    let upgrade = add.iter().any(|n| n.probability != 100) && !d.notes_have_probability();
                    let length = self.default_pattern_length(d.project());
                    let scene = self.scene;
                    let mut made = p;
                    let r = d.outside_take(|d| {
                        d.transaction(label, |d| {
                            let n = d.project().groups.get(g).ok_or("no such group")?.patterns.len();
                            if made >= n {
                                made = d.add_pattern(g, 0, &format!("Pattern {}", n + 1))?;
                                d.set_pattern_length(g, made, length)?;
                                if scene < d.project().scenes.len() {
                                    d.assign_pattern(scene, g, Some(made))?;
                                }
                            }
                            d.edit_notes(label, g, made, &remove, &add)
                        })
                    });
                    if r.is_ok() {
                        if g == self.group {
                            self.pattern = made;
                        }
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

    /// Move the playhead to tick `tick` of the focused pattern's loop when stopped.
    fn seek_pattern(&mut self, tick: i64) {
        if self.playing.is_some() {
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        let d = doc.borrow();
        let p = d.project();
        let Some(grp) = p.groups.get(self.group) else { return };
        let plen = if self.pattern < grp.patterns.len() { layout::pattern_len(p, self.group, self.pattern) } else { p.bar_length() }.max(1);
        let t = tick.clamp(0, plen - 1);
        let mode = if self.song_view { Mode::Arrangement } else { Mode::Scene(self.scene) };
        let cur = self.shown_playhead().filter(|h| h.mode == mode).map_or(0, |h| h.position.max(0.0) as i64);
        let to = match mode {
            Mode::Arrangement => {
                let mut sec_start = 0i64;
                for sec in &p.sections {
                    if cur < sec_start + sec.length {
                        break;
                    }
                    sec_start += sec.length;
                }
                let in_sec = (cur - sec_start).max(0);
                sec_start + (in_sec / plen) * plen + t
            }
            _ => (cur / plen) * plen + t,
        };
        drop(d);
        self.parked = Some(Playhead { mode, queued: None, position: to as f64, played: to as f64 });
        self.sync_transport();
    }

    /// Move the playhead to tick `tick` of the arrangement when stopped.
    fn seek_arrangement(&mut self, tick: i64) {
        if self.playing.is_some() {
            return;
        }
        let to = tick.max(0);
        let mode = Mode::Arrangement;
        self.parked = Some(Playhead { mode, queued: None, position: to as f64, played: to as f64 });
        self.sync_transport();
    }

    /// A knob moved while writing modulation (docs/superpowers/plans/
    /// 2026-10-22-phase2c-modulation.md): with the song at `head` (mode, tick), its
    /// deviation from the knob's own value goes into the playing pattern of its
    /// channel's group there, one undo step until `done` (the knob let go), when the
    /// movement is thinned to points. Stopped, or a knob that can't be modulated, it's
    /// an ordinary edit.
    fn modulate_at(&mut self, edit: layout::Edit, amount: f32, done: bool, head: Option<(Mode, f64)>) {
        let Some(doc) = self.doc.clone() else { return };
        let Some((chain, target)) = modulation::mod_target(edit) else {
            layout::apply_acts(self, vec![edit.to_act(amount)]);
            return;
        };
        let Some((mode, tick)) = head else {
            self.end_mod_take_for(chain, &target);
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
            let Some((pi, pos)) = record::pattern_at(p, mode, tick, g) else {
                drop(d);
                layout::apply_acts(self, vec![edit.to_act(amount)]);
                return;
            };
            let Some(at) = self.knob_travel(p, chain, &target) else { return };
            (pi, pos, (amount as f64 - at).clamp(-1.0, 1.0))
        };
        // If this knob already had an open touch on another pattern, end that touch first.
        if self.mod_takes.iter().any(|t| t.chain == chain && t.target == target && t.pattern != pi) {
            self.end_mod_take_for(chain, &target);
        }
        if !self.mod_takes.iter().any(|t| t.chain == chain && t.target == target) {
            if self.mod_takes.is_empty() {
                doc.borrow_mut().begin_take("Record Modulation");
            }
            self.ensure_mod_param(chain, &target);
            self.mod_takes.push(modulation::ModTake {
                chain,
                target: target.clone(),
                pattern: pi,
                samples: Vec::new(),
                moved: std::time::Instant::now(),
            });
        }
        if let Some(take) = self.mod_takes.iter_mut().find(|t| t.chain == chain && t.target == target) {
            take.moved = std::time::Instant::now();
        }
        self.touch_write_for(chain, &target, pos, dev);
        if done {
            self.end_mod_take_for(chain, &target);
        }
    }

    /// The touch on `(chain, target)` writes `dev` at `pos`, over what was there since the
    /// last position it wrote (the points passed over go; over the pattern's end, its end
    /// and its start).
    fn touch_write_for(&mut self, chain: deckard_doc::Chain, target: &deckard_model::ModTarget, pos: i64, dev: f64) {
        let Some(doc) = self.doc.clone() else { return };
        let Some(take) = self.mod_takes.iter_mut().find(|t| t.chain == chain && t.target == *target) else { return };
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
            Some(_) => d.set_modulation(c, &t, pi, (pos, pos), &[(pos, dev)]),
            None => d.set_modulation(c, &t, pi, (pos, pos), &[(pos, dev)]),
        };
        if take.samples.last().is_some_and(|s| s.0 == pos) {
            *take.samples.last_mut().unwrap() = (pos, dev);
        } else {
            take.samples.push((pos, dev));
        }
        if let Err(e) = result {
            self.status = format!("can't write modulation: {e}");
        }
    }

    /// The song at `head` while knobs are touched: each touch still moving (within
    /// [`record::IDLE`]) goes on overwriting at its last value as the song passes, as the
    /// controller lanes do; idle, stopped or in another pattern, that touch ends.
    pub(crate) fn tick_mod_take(&mut self, head: Option<(Mode, f64)>, now: std::time::Instant) {
        if self.mod_takes.is_empty() {
            return;
        }
        let (Some((mode, tick)), Some(doc)) = (head, self.doc.clone()) else { return self.end_mod_take() };
        let mut writes = Vec::new();
        let mut ended = Vec::new();
        {
            let d = doc.borrow();
            let p = d.project();
            for take in &self.mod_takes {
                if now.duration_since(take.moved) >= record::IDLE {
                    ended.push((take.chain, take.target.clone()));
                    continue;
                }
                let g = match take.chain {
                    deckard_doc::Chain::Sound(g, _) | deckard_doc::Chain::Group(g) => g,
                    deckard_doc::Chain::Master => {
                        ended.push((take.chain, take.target.clone()));
                        continue;
                    }
                };
                let at = record::pattern_at(p, mode, tick, g);
                match (at, take.samples.last()) {
                    (Some((pi, pos)), Some(&(last, dev))) if pi == take.pattern && pos != last => {
                        writes.push((take.chain, take.target.clone(), pos, dev));
                    }
                    (Some((pi, _)), _) if pi == take.pattern => {}
                    _ => ended.push((take.chain, take.target.clone())),
                }
            }
        }
        for (chain, target, pos, dev) in writes {
            self.touch_write_for(chain, &target, pos, dev);
        }
        for (chain, target) in ended {
            self.end_mod_take_for(chain, &target);
        }
    }

    /// Where the knob of `target` in `c` is (0..1), unmodulated: a module's parameter,
    /// its channel's fader, or a plug-in's parameter.
    pub(crate) fn knob_travel(&self, p: &deckard_model::Project, c: deckard_doc::Chain, target: &deckard_model::ModTarget) -> Option<f64> {
        if let Some(v) = modulation::knob_travel(p, c, target) {
            return Some(v);
        }
        let (slot, id) = (target.slot?, target.field.parse::<u32>().ok()?);
        if let Some(inst) = self.samples.plugins.existing(self.plugin_slot_key(c, slot))
            && let Ok(g) = inst.try_lock()
        {
            return Some(g.knob_value(id));
        }
        let v = self.plugin_views.get(&(c, slot))?;
        let k = v.pages.get(v.page)?.knobs.iter().position(|(info, ..)| info.id == id)?;
        v.values.get(k).map(|(val, _)| *val)
    }

    /// Where a knob's modulation has it (0..1): as last written in the touch going on,
    /// else at the play position; `None` when it isn't modulated.
    fn mod_position(&self, edit: layout::Edit) -> Option<f64> {
        let (chain, target) = modulation::mod_target(edit)?;
        if let Some(take) = self.mod_takes.iter().find(|t| t.chain == chain && t.target == target)
            && let Some(&(_, dev)) = take.samples.last()
        {
            let doc = self.doc.as_ref()?.borrow();
            return Some((self.knob_travel(doc.project(), chain, &target)? + dev).clamp(0.0, 1.0));
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
        if !self.mod_takes.is_empty() && (self.playing.is_none() || (was && !self.auto_write)) {
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
        let at = self.knob_travel(p, chain, &target)?;
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

    /// Finish the open modulation touch for `(chain, target)`, thinning its run to points
    /// and closing the undo take once the last active touch ends.
    fn end_mod_take_for(&mut self, chain: deckard_doc::Chain, target: &deckard_model::ModTarget) {
        let Some(idx) = self.mod_takes.iter().position(|t| t.chain == chain && t.target == *target) else { return };
        let take = self.mod_takes.remove(idx);
        let Some(doc) = self.doc.clone() else { return };
        let mut d = doc.borrow_mut();
        if let Err(e) = modulation::write_run(&mut d, take.chain, &take.target, take.pattern, &take.samples) {
            self.status = format!("can't write modulation: {e}");
        }
        if self.mod_takes.is_empty() {
            d.end_take();
        }
    }

    /// Finish all open modulation touches: each movement as few points (when it stayed
    /// within one pass of the pattern), and close the shared undo step.
    fn end_mod_take(&mut self) {
        if self.mod_takes.is_empty() {
            return;
        }
        let takes = std::mem::take(&mut self.mod_takes);
        let Some(doc) = self.doc.clone() else { return };
        let mut d = doc.borrow_mut();
        for take in takes {
            if let Err(e) = modulation::write_run(&mut d, take.chain, &take.target, take.pattern, &take.samples) {
                self.status = format!("can't write modulation: {e}");
            }
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
            self.count_in_and_play();
            return;
        }
        if on == self.recording {
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        // No pattern of the focused group in the focused scene: a new one to record into.
        let scene_exists = self.scene < doc.borrow().project().scenes.len();
        if on && scene_exists && self.scene_pattern(self.group).is_none() {
            layout::apply_acts(self, vec![layout::Act::NewPattern(self.scene, self.group)]);
        }
        self.recording = on;
        if let Some(Ok(c)) = &mut self.controller {
            c.set_recording(on);
        }
        if on {
            doc.borrow_mut().begin_take("Record");
            self.take = record::Take::begin(doc.borrow().project());
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
                self.count_in_and_play();
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
            // A grown pattern: the bars played, or as it was.
            if let Some(grow) = self.grow.take() {
                let (g, pi) = (grow.group, grow.pattern);
                let d = doc.borrow();
                let p = d.project();
                let bar = p.bar_length();
                let pat = &p.groups[g].patterns[pi];
                let first = pat.notes.iter().map(|n| n.position).min();
                let start = first.map_or(pat.start, |n| pat.start + (n - pat.start).max(0) / bar * bar);
                let end = pat.notes.iter().map(|n| n.position - start + n.length).max();
                let to = end.map_or(grow.before, |e| ((e + bar - 1) / bar).max(1) * bar);
                drop(d);
                let _ = doc.borrow_mut().set_pattern_span(g, pi, start, to);
            }
            // First takes in fixed-length empty patterns follow the same start
            // rule. Overdubs retain the existing pattern's deliberate space.
            for (g, pi) in self.take.new_recordings() {
                let d = doc.borrow();
                let bar = d.project().bar_length().max(1);
                let Some(pat) = d.project().groups.get(g).and_then(|g| g.patterns.get(pi)) else { continue };
                let Some(first) = pat.notes.iter().map(|n| n.position).min() else { continue };
                let leading = (first - pat.start).max(0) / bar * bar;
                let (start, length) = (pat.start + leading, (pat.length - leading).max(bar));
                drop(d);
                if leading > 0 {
                    let _ = doc.borrow_mut().set_pattern_span(g, pi, start, length);
                }
            }
            doc.borrow_mut().end_take();
            self.status = "Recorded".into();
        }
    }

    /// SHIFT + REC: count in (Preferences' bars, at least one), then play and record
    /// from the beginning, as PLAY from a fresh start does (not where STOP left it).
    fn count_in_and_play(&mut self) {
        self.count_in_next = self.settings.count_in_bars.max(1);
        self.parked = None;
        self.transport_play();
        self.sync_transport();
        self.status = "Recording".into();
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
            // MIDI recorded into another group: that group and sound have the focus.
            if r.midi && r.velocity > 0 && r.group != self.group && self.settings.focus_follows {
                self.focus_group(r.group);
                self.sound = r.sound;
            }
            // A note into a growing pattern's last bar adds the next.
            if r.velocity > 0
                && let Some(grow) = &mut self.grow
                && r.group == grow.group
            {
                let at = record::pattern_at(doc.borrow().project(), r.mode, r.tick, r.group);
                let start = doc.borrow().project().groups[grow.group].patterns.get(grow.pattern).map_or(0, |p| p.start);
                if let Some((pi, pos)) = at.filter(|x| x.0 == grow.pattern)
                    && let Some(len) = grow.reach(pos - start)
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

    /// A growing pattern, with the song at `tick` in `mode`: into its last bar, the next
    /// is added (a bar ahead, so the song plays on into it).
    fn auto_grow_at(&mut self, mode: Mode, tick: f64) {
        let Some(doc) = self.doc.clone() else { return };
        let Some(grow) = &mut self.grow else { return };
        let (g, pi) = (grow.group, grow.pattern);
        let at = record::pattern_at(doc.borrow().project(), mode, tick, g).filter(|x| x.0 == pi);
        let start = doc.borrow().project().groups[g].patterns.get(pi).map_or(0, |p| p.start);
        if let Some((_, pos)) = at
            && let Some(len) = grow.reach(pos - start)
            && let Err(e) = doc.borrow_mut().set_pattern_length(g, pi, len)
        {
            self.status = format!("can't grow the pattern: {e}");
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
                controller::Request::FocusSound(s) => self.focus_sound(s),
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
        Some(if self.surface.holding() || self.surface.animates() || self.filter_modulates() || self.picture_moves() {
            std::time::Duration::from_millis(30)
        } else {
            controller::tick_every(self.playing.is_some() || self.recorder.busy() || self.recorder.is_metering())
        })
    }

    /// The focused Filter's Amount is driving its LFO, so the right screen has to keep
    /// redrawing.
    fn filter_modulates(&self) -> bool {
        if self.channel {
            return false;
        }
        let Some(doc) = &self.doc else { return false };
        let Ok(d) = doc.try_borrow() else { return false };
        d.chain_slots(self.chain())
            .and_then(|slots| self.slot.and_then(|i| slots.get(i)))
            .is_some_and(|s| s.module == "Filter" && deckard_engine::fx::filter::modulates(&s.params))
    }

    /// The focused effect's picture is moving (a chorus, a tremolo, a modulated tail).
    fn picture_moves(&self) -> bool {
        if self.channel {
            return false;
        }
        let Some(doc) = &self.doc else { return false };
        let Ok(d) = doc.try_borrow() else { return false };
        d.chain_slots(self.chain())
            .and_then(|slots| self.slot.and_then(|i| slots.get(i)))
            .is_some_and(|s| deckard_engine::fx::picture::moves(&s.module, &s.params))
    }

    /// Where a moving picture is in its cycle. The rate is added on, so turning Speed
    /// does not jump the drawing.
    fn picture_phase(module: &str, params: &[(String, f64)], tempo: f64) -> f32 {
        use std::sync::Mutex;
        static CLOCK: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));
        if !deckard_engine::fx::picture::moves(module, params) {
            return 0.0;
        }
        let now = Self::filter_seconds();
        let hz = deckard_engine::fx::picture::rate_hz(module, params, tempo);
        let mut clock = CLOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (at, phase) = *clock;
        let phase = deckard_engine::fx::filter::advance_phase(phase, if at == 0.0 { 0.0 } else { now - at }, hz);
        *clock = (now, phase);
        phase as f32
    }

    /// The focused built-in effect, other than the EQ and the Filter, as the right
    /// screen draws it. The same on every page.
    fn effect_picture(slot: &deckard_model::SlotInfo, phase: f32, side_live: bool, page: usize) -> Option<deckard_surface::EqCurveView> {
        let shot = deckard_engine::fx::picture::shot(&slot.module, &slot.params, phase, side_live, page)?;
        Some(deckard_surface::EqCurveView {
            kind: shot.kind.to_string(),
            points: shot.points,
            bands: shot.bands,
            extent: shot.extent,
            lfo: shot.lfo,
            side: shot.side,
            side_live: shot.side_live,
            dim: shot.dim,
            preview: shot.preview,
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
            if let Some((n, frame)) = out.screens {
                m.set_screens(n, frame);
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

    /// Maschine's scripts are showing their Mixer page, when that page id exists.
    fn hardware_shows_mixer(host: &deckard_lua_host::Host) -> Option<bool> {
        let page = host.constant("NI.HW.PAGE_MIXER").ok()?;
        Some(host.state.borrow().hw.page_stack.last().copied() == Some(page))
    }

    /// The MK3's events since the last frame, to the layer that runs it (Maschine's
    /// scripts, or DECKARD's own): what the pads play, the lights and new screens.
    /// Hardware-free, so tests drive it with made-up events.
    pub(crate) fn mk3_events(&mut self, events: Vec<deckard_mk3::Event>, requests: &mut Vec<controller::Request>) -> Mk3Out {
        use deckard_mk3::Event;
        let events = coalesce_mk3_events(events);
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
        let mut mixer_on = None;
        let out = if let Some(Ok(c)) = &mut self.controller {
            for e in events {
                match e {
                    Event::Button(name, down) => {
                        c.hold(name, down, requests);
                        // Maschine's own scripts: MIX shows their mixer page. Follow it
                        // in the window. DECKARD's layer does this with MixerView.
                        if down && name == "BUTTON_MIX" {
                            mixer_on = Self::hardware_shows_mixer(&c.host);
                        }
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
                screens = c.screens().map(|s| (self.mk3_screens_sent, mk3_io::ScreenFrame::Pixels(std::sync::Arc::new(s.px.clone()))));
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
            self.browser.poll();
            self.refresh_surface_settings();
            let mut view = self.surface_view();
            let mut actions = Vec::new();
            for e in &events {
                actions.extend(self.surface.event(e, &view, now));
            }
            actions.extend(self.surface.tick(&view, now));
            if !actions.is_empty() {
                for a in actions {
                    self.surface_act(a);
                }
                // (A slot or sound the buttons just focused shows its plug-in's knobs.)
                self.update_slot_view();
                self.refresh_surface_settings();
                view = self.surface_view();
            }
            // On the Browse page, the browser has its results to show.
            if self.surface.page() == "browse" && !self.browser.started() {
                self.browse(browser::BrowseAct::Toggle);
                self.browse(browser::BrowseAct::Toggle);
                self.browser.poll();
                self.refresh_surface_settings();
                view = self.surface_view();
            }
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
            Mk3Out {
                route,
                leds: self.surface.leds(&view),
                screens: self.surface.screen_picture(&view).map(|(n, p)| (n, mk3_io::ScreenFrame::Picture(p))),
            }
        };
        if let Some(on) = mixer_on {
            self.set_mixer(on);
        }
        out
    }

    /// The focused built-in EQ, as the right screen draws it.
    fn eq_curve(slot: &deckard_model::SlotInfo) -> Option<deckard_surface::EqCurveView> {
        if slot.module != "EQ" {
            return None;
        }
        let curve = deckard_engine::fx::eq::response(&slot.params, 48_000.0, 120);
        Some(deckard_surface::EqCurveView { points: curve.points, bands: curve.bands.into_iter().collect(), ..Default::default() })
    }

    /// Seconds since the first filter picture, so its LFO keeps moving.
    fn filter_seconds() -> f64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
    }

    /// Where the LFO is in its cycle. The rate is added on, so turning Speed does not
    /// jump the picture to another point.
    fn filter_depth(params: &[(String, f64)], tempo: f64) -> f32 {
        use std::sync::Mutex;
        static CLOCK: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));
        let now = Self::filter_seconds();
        let hz = deckard_engine::fx::filter::lfo_rate(params, tempo);
        let mut clock = CLOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (at, phase) = *clock;
        let phase = deckard_engine::fx::filter::advance_phase(phase, if at == 0.0 { 0.0 } else { now - at }, hz);
        *clock = (now, phase);
        if !deckard_engine::fx::filter::modulates(params) {
            return 0.0;
        }
        deckard_engine::fx::filter::lfo_sample(params, phase)
    }

    /// Whether a side-chain input is routed into this effect.
    fn sidechain_routed(p: &deckard_model::Project, chain: deckard_doc::Chain, slot: usize) -> bool {
        let deckard_doc::Chain::Sound(g, s) = chain else { return false };
        p.groups.get(g).and_then(|g| g.sounds.get(s)).is_some_and(|snd| snd.channel.sidechain.iter().any(|(k, _)| *k == slot))
    }

    /// The focused built-in Filter, as the right screen draws it.
    fn filter_curve(slot: &deckard_model::SlotInfo, tempo: f64, side_live: bool) -> Option<deckard_surface::EqCurveView> {
        if slot.module != "Filter" {
            return None;
        }
        let depth = Self::filter_depth(&slot.params, tempo);
        let curve = deckard_engine::fx::filter::response(&slot.params, 120, depth);
        let amount = slot.params.iter().find(|(k, _)| k == "amount").map_or(0.0, |(_, v)| *v);
        let extent = if amount.abs() > 1e-4 { deckard_engine::fx::filter::response(&slot.params, 120, 1.0).points } else { Vec::new() };
        Some(deckard_surface::EqCurveView {
            points: curve.points,
            bands: vec![curve.cutoff],
            extent,
            lfo: deckard_engine::fx::filter::lfo_picture(&slot.params, 48),
            side: deckard_engine::fx::sidechain::response(&slot.params, 120),
            side_live,
            ..Default::default()
        })
    }

    /// What DECKARD's own MK3 layer shows of the app now.
    fn surface_view(&self) -> deckard_surface::View {
        use deckard_surface::{ControlView, GroupView, KnobView, SoundView, View};
        let c = self.control_snapshot([false; 8]);
        let head = self.shown_playhead();
        let edits = self.knob_edits();
        // (Not on the Macro page: its knobs are all macros.)
        let on_macros = self.channel && self.prop_set == 3;
        if let Some(doc) = &self.doc {
            let d = doc.borrow();
            let rev = d.revision();
            let mut cache = self.surface_doc_cache.borrow_mut();
            if (cache.0, cache.1) != (self.doc_generation, rev) {
                let noted = d
                    .project()
                    .groups
                    .iter()
                    .map(|grp| grp.patterns.iter().flat_map(|pt| pt.notes.iter().map(|n| n.sound)).collect())
                    .collect();
                *cache = (self.doc_generation, rev, d.macro_targets(), noted);
            }
        }
        let doc_cache = self.surface_doc_cache.borrow();
        // A Zhora's outputs by the channels' names (their aliases).
        let zhora_focused = self.focused_zhora().is_some();
        let knob_text = |i: usize, text: &str| match edits.get(i).copied().flatten() {
            Some(layout::Edit::PluginChoice(_, id, _, cur, _)) if zhora_focused => {
                zhora_ui::choice_name(&self.settings, id, cur).unwrap_or_else(|| text.to_string())
            }
            _ => text.to_string(),
        };
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
                    picture: plugin_pictures::slot_key(&self.surface, &s.name),
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
                        text: knob_text(i, &k.text),
                        value: k.value,
                        modulatable: edits.get(i).copied().flatten().and_then(modulation::mod_target).is_some(),
                        step_mod: self.step_mod_text(edits.get(i).copied().flatten()),
                        auto_mod: self.auto_mod_text(edits.get(i).copied().flatten()),
                        in_macro: !on_macros && k.param.is_some_and(|p| doc_cache.2.contains(&p)),
                    })
                })
                .collect(),
            // (Only while the plug-in browser shows: the list is long.)
            modules: if self.surface.page() == "module" { self.module_views() } else { Vec::new() },
            eq: None,
            filter: None,
            picture: None,
            zhora: None,
            tuner: None,
            euclid: self.euclid_view(c.level, c.page, head),
            random: self.random_view(c.level, c.page, head),
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
            keep_focus: !self.settings.focus_follows,
            relative_strips: self.settings.relative_strips,
            follow: self.follow,
            position: head.map(|h| h.position as i64),
            progress: self.pattern_at(self.group, head).or_else(|| self.bar_at(head)).map(|(at, length)| at as f64 / length as f64),
            group: self.group,
            sound: self.sound,
            control,
            sampling: self.sampling_surface_view(),
            ..Default::default()
        };
        // (Only while BROWSE shows, or the JAM has the panel open: the results can be long.)
        if self.surface.page() == "browse" || (self.jam_device.is_some() && self.browser.open) {
            let b = self.browser.view();
            for name in &b.products {
                plugin_pictures::ensure_browser_product(&self.surface, name);
            }
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
                routing: b.routing,
                patterns: b.patterns,
            };
        }
        let Some(doc) = &self.doc else { return v };
        let d = doc.borrow();
        let p = d.project();
        v.tempo = self.playing_scene_tempo(p).map_or(p.tempo, |t| t.1);
        // The built-in EQ's or Filter's curve, for the right screen. Both pages use it.
        if !self.channel {
            let slot = d.chain_slots(self.chain()).and_then(|slots| self.slot.and_then(|i| slots.get(i)));
            v.control.eq = slot.and_then(Self::eq_curve);
            let tempo = v.tempo;
            let live = self.slot.is_some_and(|i| Self::sidechain_routed(p, self.chain(), i));
            v.control.filter = slot.and_then(|s| Self::filter_curve(s, tempo, live));
            let page = c.page;
            v.control.picture = slot.and_then(|s| Self::effect_picture(s, Self::picture_phase(&s.module, &s.params, tempo), live, page));
            // The tuner reads the instance: only for the page that shows it.
            if self.surface.page() == "control" {
                v.control.zhora = self.zhora_view(p);
                v.control.tuner = self.tuner_view();
            }
        }
        v.project = p.name.clone();
        v.swing = p.swing.amount;
        v.master_volume = p.master_level;
        v.master_db = deckard_model::level_db(p.master_level);
        v.master_meter = self.master_meter;
        v.cue_meter = self.cue_meter;
        v.input_meters = self.input_meters;
        v.input_levels = self.settings.input_levels;
        v.cue_level = p.cue_level;
        v.scale_root = self.scale.0;
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
            // While modulation is written, where it has the fader: a turn goes on from there.
            let t = match channel {
                layout::Channel::Sound(g, s) => Some(deckard_doc::Target::Sound(g, s)),
                layout::Channel::Group(g) => Some(deckard_doc::Target::GroupStrip(g)),
                _ => None,
            };
            let written = |edit: fn(deckard_doc::Target) -> layout::Edit| {
                t.map(edit).filter(|e| layout::knob_drag_writes(self, *e)).and_then(|e| self.mod_position(e))
            };
            let level = written(layout::Edit::Level).unwrap_or(level);
            let pan = written(layout::Edit::Pan).map_or(pan, |a| a * 2.0 - 1.0);
            deckard_surface::MixView {
                level: deckard_model::level_db(level),
                pan,
                meter,
                headroom: self.headroom.get(&channel).copied().unwrap_or(0.0),
                plugins: slots.iter().map(|x| (x.is_instrument(), x.active)).collect(),
                ..Default::default()
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
        } else {
            // None: as long as a new one would be (a step makes it), so the Step page
            // takes its pads.
            v.pattern_length = self.default_pattern_length(p);
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
                let noted = doc_cache.3.get(g);
                GroupView {
                    name: grp.name.clone(),
                    rgb: layout::rgb(p, g, None),
                    color: layout::colour_index(p, g, None),
                    muted: grp.muted,
                    default_color: grp.color == deckard_model::Color::Default,
                    playing: self.lit.iter().any(|&(lg, _)| lg == g),
                    mix: deckard_surface::MixView {
                        aux: grp.aux,
                        swing: grp.swing.amount,
                        tune: grp.sounds.iter().find_map(|s| s.sampler.as_ref().map(|x| x.tune)),
                        ..mix(grp.level, grp.pan, loudest, layout::Channel::Group(g), &grp.slots)
                    },
                    perform: self.perform_slot(g).map(|i| {
                        let t = || deckard_doc::Target::ChainModule(deckard_doc::Chain::Group(g), i);
                        (d.num(t(), "touchstrip_on_off").unwrap_or(0.0) > 0.5, d.num(t(), "touchstrip_pos").unwrap_or(0.5))
                    }),
                    perform_mode: self.perform_slot(g).map(|i| {
                        let t = deckard_doc::Target::ChainModule(deckard_doc::Chain::Group(g), i);
                        let mode = (d.num(t, "mode").unwrap_or(0.0).round().max(0.0) as usize).min(7);
                        (mode, module_ui::PERFORM_MODES[mode].to_string())
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
                            slot: pt.slot,
                        })
                        .collect(),
                    scene_pattern: p.scenes.get(self.scene).and_then(|sc| sc.patterns.iter().find(|(pg, _)| *pg == g).map(|&(_, pi)| pi)),
                    progress: head.and_then(|_| layout::group_progress(p, head, self.scene, g)).map(|(bars, played)| played / bars as f64),
                    playing_progress: head
                        .and_then(|h| {
                            let scene = if let Mode::Scene(s) = h.mode { s } else { self.scene };
                            layout::group_progress(p, head, scene, g)
                        })
                        .map(|(bars, played)| played / bars as f64),
                    sounds: (0..grp.sounds.len())
                        .map(|s| {
                            let snd = &grp.sounds[s];
                            SoundView {
                                name: snd.name.clone(),
                                rgb: layout::rgb(p, g, Some(s)),
                                color: layout::colour_index(p, g, Some(s)),
                                muted: snd.muted,
                                // (A sound may only play MIDI out, with no plug-ins. A Sampler
                                // whose module didn't become a slot still has samples.)
                                used: !snd.slots.is_empty()
                                    || snd.sampler.as_ref().is_some_and(|sm| sm.zones.iter().any(|z| !z.sample.path.is_empty()))
                                    || !snd.name.is_empty()
                                    || snd.channel.midi_out != deckard_model::MidiDest::None
                                    || noted.is_some_and(|n| n.contains(&s)),
                                playing: self.lit.contains(&(g, s)),
                                mix: deckard_surface::MixView {
                                    aux: snd.aux,
                                    swing: snd.swing.amount,
                                    tune: snd.sampler.as_ref().map(|x| x.tune),
                                    ..mix(snd.level, snd.pan, meter(g, s), layout::Channel::Sound(g, s), &snd.slots)
                                },
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
                                selected: g == self.group && (s == self.sound || self.sound_selection.contains(&s)),
                                audio: if g == self.group { self.loop_view(p, g, s) } else { None },
                            }
                        })
                        .collect(),
                }
            })
            .collect();
        if let Some(load) = &self.group_load {
            if let Some(group) = v.groups.get_mut(load.group) {
                group.name = format!("Loading {}…", load.item.name);
            }
            v.browser.busy = true;
        }
        v.loading = self.group_load.as_ref().map(|l| l.item.name.clone()).or_else(|| self.item_load.as_ref().map(|l| l.item.name.clone()));
        v
    }

    /// Carry out what DECKARD's own MK3 layer asked for.
    /// The section the controller's Song view focuses: the one picked (the last if it's
    /// gone), else the first.
    fn focused_section(&self, p: &deckard_model::Project) -> Option<usize> {
        let last = p.sections.len().checked_sub(1)?;
        Some(self.section.map_or(0, |i| i.min(last)))
    }

    /// The software Mixer. Opening it closes the LFO panel, which shares its place.
    pub(crate) fn set_mixer(&mut self, on: bool) {
        self.mixer = on;
        self.lfos &= !self.mixer;
    }

    pub(crate) fn surface_act(&mut self, a: deckard_surface::Action) {
        use deckard_doc::Target;
        use deckard_lua_host::state::ControlRequest as R;
        use deckard_surface::Action as A;
        use layout::Act;
        let g = self.group;
        let project = |app: &App| app.doc.as_ref().map(|d| d.borrow().project().clone());
        match a {
            A::Play => self.transport_play(),
            A::Stop => self.stop(),
            A::Record { on, count_in } => self.set_recording(on, count_in),
            A::Restart => self.restart(),
            A::Loop(on) => self.set_looping(on),
            A::Metronome(on) => self.set_metronome(on),
            A::AutoWrite(on) => {
                self.auto_write = on;
                if !on {
                    self.end_mod_take();
                }
            }
            A::Tempo(bpm) => layout::apply_acts(self, vec![Act::SetTempo(bpm)]),
            A::Swing(amount) => {
                let Some(p) = project(self) else { return };
                layout::apply_acts(self, vec![Act::SetSwing(Target::Song, deckard_model::Swing { amount, ..p.swing })]);
            }
            A::Volume(level) => layout::apply_acts(self, vec![Act::SetLevel(Target::Song, level)]),
            A::VolumeDb(db) => layout::apply_acts(self, vec![Act::SetLevel(Target::Song, deckard_model::level_from_db(db))]),
            A::FocusGroup(group) => self.focus_group(group),
            A::FocusSound(s) => self.focus_sound(s),
            A::SelectPad { index, multi } => self.select_pad(index, multi),
            A::SelectAllSounds => self.select_all_sounds(),
            A::SelectNoSounds => self.select_no_sounds(),
            A::FocusScene(scene) => {
                if project(self).is_some_and(|p| scene < p.scenes.len()) {
                    self.launch_scene(scene);
                }
            }
            A::ShowScenes(first) => self.show_scenes = Some(first),
            A::NewPatternIn(group) => {
                if project(self).is_some_and(|p| group < p.groups.len()) {
                    let was = (self.group, self.pattern);
                    layout::apply_acts(self, vec![Act::NewPattern(self.scene, group)]);
                    if group != was.0 {
                        (self.group, self.pattern) = was;
                    }
                }
            }
            A::NewPatternAt { group, slot } => {
                if project(self).is_some_and(|p| group < p.groups.len()) {
                    let was = (self.group, self.pattern);
                    layout::apply_acts(self, vec![Act::NewPatternAt(self.scene, group, slot)]);
                    if group != was.0 {
                        (self.group, self.pattern) = was;
                    }
                }
            }
            // (A MIDI controller's scene button: stopped, it starts the scene too.)
            A::LaunchScene(scene) => {
                if project(self).is_some_and(|p| scene < p.scenes.len()) {
                    self.launch_scene(scene);
                    if self.playing.is_none() && !self.receiving_clock() {
                        self.parked = None;
                        self.play_from(Mode::Scene(scene), 0);
                        self.sync_transport();
                    }
                }
            }
            A::PickPattern(pi) => {
                if project(self).is_some_and(|p| p.groups.get(g).is_some_and(|grp| pi < grp.patterns.len())) {
                    layout::apply_acts(self, vec![Act::AssignPattern(self.scene, g, Some(pi)), Act::FocusPattern(g, pi)]);
                }
            }
            A::TouchWrite(knob) => self.surface_touch_write(knob),
            A::Sampling(_) | A::SamplingTab(_) | A::SamplingKnob { .. } | A::SamplingCall(_) | A::RecordSample(_) | A::Take { .. } => {
                self.surface_sampling(a)
            }
            A::ScenePattern { group, pattern } => {
                if project(self).is_some_and(|p| {
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
                let Some(p) = project(self) else { return };
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
            A::MixerView(on) => self.set_mixer(on),
            A::FocusSection(i) => {
                if project(self).is_some_and(|p| i < p.sections.len()) {
                    self.launch_section(i);
                }
            }
            A::SectionEdit(what) => {
                let Some(p) = project(self) else { return };
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
                let Some(p) = project(self) else { return };
                let Some(i) = self.focused_section(&p) else { return };
                let to = if next { i + 1 } else { i.wrapping_sub(1) };
                if to < p.sections.len() {
                    layout::apply_acts(self, vec![Act::MoveSection(i, to)]);
                }
            }
            A::SectionScene(next) => {
                let Some(p) = project(self) else { return };
                let Some(i) = self.focused_section(&p) else { return };
                let Some(at) = p.sections[i].scene else { return };
                let to = if next { at + 1 } else { at.wrapping_sub(1) };
                if to < p.scenes.len() {
                    layout::apply_acts(self, vec![Act::SectionScene(i, to)]);
                }
            }
            A::SectionLength(ticks) => {
                let Some(i) = project(self).as_ref().and_then(|p| self.focused_section(p)) else { return };
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
                    B::Routing(on) => C::Routing(on),
                    B::Patterns(on) => C::Patterns(on),
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
                let level = deckard_model::level_from_db(db);
                if layout::knob_drag_writes(self, layout::Edit::Level(t)) {
                    self.modulate(layout::Edit::Level(t), level as f32, false);
                } else {
                    layout::apply_acts(self, vec![Act::SetLevel(t, level)]);
                }
            }
            A::ChannelPan { group, sound, pan } => {
                let t = match sound {
                    Some(s) => Target::Sound(group, s),
                    None => Target::GroupStrip(group),
                };
                if layout::knob_drag_writes(self, layout::Edit::Pan(t)) {
                    self.modulate(layout::Edit::Pan(t), ((pan + 1.0) / 2.0) as f32, false);
                } else {
                    layout::apply_acts(self, vec![Act::SetPan(t, pan)]);
                }
            }
            A::MuteGroup(group) => layout::apply_acts(self, vec![Act::ToggleMute(Target::Group(group))]),
            A::SoloGroup(group) => self.solo(layout::Channel::Group(group)),
            A::MuteSound(s) => layout::apply_acts(self, vec![Act::ToggleMute(Target::Sound(g, s))]),
            A::SoloSound(s) => self.solo(layout::Channel::Sound(g, s)),
            A::EraseSound(s) => {
                let Some(pat) = project(self).as_ref().and_then(|p| p.groups.get(g)?.patterns.get(self.pattern).cloned()) else { return };
                let remove: Vec<_> = pat.notes.iter().filter(|n| n.sound == s).copied().collect();
                if !remove.is_empty() {
                    layout::apply_acts(self, vec![Act::EditNotes { g, p: self.pattern, label: "Erase", remove, add: Vec::new() }]);
                }
            }
            A::Turn { knob, inc, fine } => self.control_request(R::Turn { pos: knob, inc, fine }),
            A::Page(next) => self.control_request(R::Page(next)),
            A::PluginPage(n) => {
                let Some(doc) = self.doc.clone() else { return };
                let show = {
                    let d = doc.borrow();
                    self.slot.and_then(|i| layout::slot_page(self, d.project(), i)).filter(|pg| n < pg.names.len()).map(|pg| (pg.show)(n))
                };
                if let Some(act) = show {
                    layout::apply_acts(self, vec![act]);
                }
            }
            A::Zhora(what) => self.zhora_press(what),
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
            A::StepNotes { at, length, velocity, pitches } => self.surface_step_notes(at, length, velocity, &pitches[..]),
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
            A::Repeat("on", v) => self.repeat_soft = v > 0.5,
            A::Repeat(what, v) => self.arp_edit(&format!("arp/{what}"), v),
            A::ChokeAll => {
                if let Some(Ok(out)) = &self.output
                    && out.has_engine()
                {
                    out.live(deckard_engine::Live::ChokeAll);
                }
            }
            A::DeletePattern { group, pattern } => {
                if project(self).is_some_and(|p| p.groups.get(group).is_some_and(|grp| pattern < grp.patterns.len())) {
                    layout::apply_acts(self, vec![Act::DeletePattern(group, pattern)]);
                }
            }
            A::CopyPattern { group, from, to } => {
                if let Some(doc) = &self.doc
                    && let Err(e) = doc.borrow_mut().copy_pattern(group, from, to)
                {
                    self.status = format!("can't duplicate the pattern: {e}");
                }
            }
            A::DeleteScene(scene) => {
                if project(self).is_some_and(|p| p.scenes.len() > 1 && scene < p.scenes.len()) {
                    layout::apply_acts(self, vec![Act::DeleteScene(scene)]);
                }
            }
            A::CopyScene { from, to } => {
                if let Some(doc) = &self.doc
                    && let Err(e) = doc.borrow_mut().copy_scene(from, to)
                {
                    self.status = format!("can't duplicate the scene: {e}");
                }
            }
            A::CopyGroup { from, to } => {
                let Some(p) = project(self) else { return };
                if from == to || from >= p.groups.len() || to > p.groups.len() {
                    return;
                }
                let mut acts = vec![Act::CopyGroup(from)];
                if to == p.groups.len() {
                    acts.push(Act::AddGroup);
                }
                acts.push(Act::PasteGroup(to));
                layout::apply_acts(self, acts);
            }
            A::CopySound { from, to } => {
                let Some(p) = project(self) else { return };
                let sounds = p.groups.get(g).map_or(0, |grp| grp.sounds.len());
                if from != to && from < sounds && to < sounds {
                    layout::apply_acts(self, vec![Act::CopySound(g, from), Act::PasteSound(g, to)]);
                }
            }
            A::SetSend { group, sound, aux, level } => {
                let t = match sound {
                    Some(s) => Target::Sound(group, s),
                    None => Target::GroupStrip(group),
                };
                layout::apply_acts(self, vec![Act::SetParam(t, if aux == 0 { "aux1_level" } else { "aux2_level" }, level)]);
            }
            A::SetChannelSwing { group, sound, amount } => {
                let Some(p) = project(self) else { return };
                let (t, swing) = match (group, sound) {
                    (Some(gr), Some(s)) => {
                        let Some(x) = p.groups.get(gr).and_then(|grp| grp.sounds.get(s)) else { return };
                        (Target::Sound(gr, s), x.swing)
                    }
                    (Some(gr), None) => {
                        let Some(x) = p.groups.get(gr) else { return };
                        (Target::Group(gr), x.swing)
                    }
                    _ => (Target::Song, p.swing),
                };
                layout::apply_acts(self, vec![Act::SetSwing(t, deckard_model::Swing { amount, ..swing })]);
            }
            A::SetTune { group, sound, semitones } => self.jam_tune(group, sound, semitones),
            A::CueDb(db) => layout::apply_acts(self, vec![Act::SetParam(Target::Song, "cue_level", deckard_model::level_from_db(db))]),
            A::InputDb(input, db) => {
                if let Some(level) = self.settings.input_levels.get_mut(input) {
                    *level = deckard_model::level_from_db(db);
                    self.input_bus.set_levels(&self.settings.input_levels);
                    self.settings.save();
                }
            }
            A::WatchInput(on) => self.input_watched = on,
            A::Library(on) => {
                if self.browser.open != on {
                    self.browse(browser::BrowseAct::Toggle);
                }
            }
            A::ResetKnob(knob) => self.jam_reset_knob(knob),
            A::PerformMode { group, mode } => self.jam_perform_mode(group, mode),
            A::SettingsWanted(on) => self.jam_settings_wanted = on,
            A::Lfo(what, pad, v) => self.surface_lfo(what, pad, v),
            A::ModSlot(slot, row, steps) => mod_slots::step(self, slot, row, steps),
            A::ModSlotsClosed => self.mod_draft = None,
            A::Lock(what, n) => match what {
                "button" => self.lock_button(),
                "pad" => self.lock_pad(n),
                "slot" => self.lock_slot(n),
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
            A::MoveGroup { from, to } => layout::apply_acts(self, vec![Act::MoveGroup(from, to)]),
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
                let Some(p) = project(self) else { return };
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
                .filter_map(|(ch, v)| {
                    let key = match *ch {
                        layout::Channel::Sound(g, s) => (Some(g), Some(s)),
                        layout::Channel::Group(g) => (Some(g), None),
                        layout::Channel::Master => (None, None),
                        layout::Channel::Extra(_) => return None,
                    };
                    Some((key, *v))
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
                        Some(e) => {
                            let base = modulation::mod_target(e).and_then(|(ch, t)| self.knob_travel(p, ch, &t)).unwrap_or(value as f64);
                            format!("{:+.0}%", (self.mod_position(e).unwrap_or(base) - base) * 100.0)
                        }
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
                    let n = (self.knob_acc[pos].abs() / ROUTE_STEP).floor() as usize;
                    if n == 0 || sets.is_empty() {
                        return;
                    }
                    let up = self.knob_acc[pos] > 0.0;
                    self.knob_acc[pos] -= (n as f64) * ROUTE_STEP * self.knob_acc[pos].signum();
                    let to = match cur {
                        usize::MAX => 0,
                        c if up => (c + n).min(sets.len() - 1),
                        c => c.saturating_sub(n),
                    };
                    if to == cur {
                        return;
                    }
                    layout::apply_acts(self, vec![Act::SetRoute(sets[to].clone())]);
                    return;
                }
                // A Zhora's output or Input: on along the steps it offers (None, then the
                // CV only channels free for it, or the connected inputs), a step each
                // quarter of the way a continuous one would go.
                let zhora_out = match edit {
                    Edit::PluginChoice(_, id, _, cur, _) => {
                        let d = doc.borrow();
                        self.zhora_offered(d.project(), id, cur)
                    }
                    _ => None,
                };
                if let Edit::PluginChoice(slot, id, steps, cur, _) = edit
                    && let Some(channels) = zhora_out
                {
                    let unit = 0.25 / steps.max(1) as f64;
                    let n = (self.knob_acc[pos].abs() / unit).floor();
                    if n < 1.0 {
                        return;
                    }
                    let sign = self.knob_acc[pos].signum();
                    self.knob_acc[pos] -= n * unit * sign;
                    let to = zhora_ui::step_choice(cur, n as i64 * sign as i64, &channels);
                    if to == cur {
                        return;
                    }
                    layout::apply_acts(self, vec![Act::PluginParam(slot, id, to as f64 / steps.max(1) as f64)]);
                    return;
                }
                // A stepped value moves a step once the knob has gone a quarter of the
                // way a continuous one would. A Variation or Generate setting over a
                // range of whole numbers (a percentage, a note) is kept whole: it moves
                // a whole one once the knob has gone as far as a continuous one would
                // (else each small turn would be rounded back to where it was).
                let step = match edit {
                    Edit::Choice(_, _, options, ..) => Some((1.0 / (options.len().max(2) - 1) as f64, 0.25)),
                    Edit::Int(_, _, lo, hi, _) => Some((1.0 / (hi - lo).max(1.0), 0.25)),
                    Edit::Param(_, f, lo, hi, _) if (f.starts_with("sequence_") || f == "base_key") && hi - lo >= 2.0 => {
                        Some((1.0 / (hi - lo), 1.0))
                    }
                    Edit::Toggle(..) => Some((1.0, 0.25)),
                    Edit::PluginChoice(_, _, steps, ..) => Some((1.0 / steps.max(1) as f64, 0.25)),
                    Edit::MidiInputParam(_, _, _, _, steps, _) if steps >= 2 => Some((1.0 / (steps - 1) as f64, 0.25)),
                    _ => None,
                };
                let to = match step {
                    Some((q, speed)) => {
                        let unit = (q * speed).max(1e-9);
                        let n = (self.knob_acc[pos].abs() / unit).floor();
                        if n < 1.0 {
                            return;
                        }
                        let sign = self.knob_acc[pos].signum();
                        self.knob_acc[pos] -= n * unit * sign;
                        value as f64 + n * q * sign
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
                // (The groove's whole numbers stay whole: the knob's position is an f32.)
                if let (Edit::Param(t, f, lo, hi, _), Some((_, 1.0))) = (edit, step) {
                    Act::SetParam(t, f, (lo + to.clamp(0.0, 1.0) * (hi - lo)).round())
                } else {
                    edit.to_act(to.clamp(0.0, 1.0) as f32)
                }
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
            // On to the next insert of the chain shown (Master, Group or Sound), or past
            // the last one to the "+". An empty chain, or a step past the "+", stays on
            // this tab.
            R::Slot(d) => {
                let n = self.chain_slots().len();
                let cur = self.slot.filter(|&i| i < n).unwrap_or(n) as i64;
                let to = (cur + d).clamp(0, n as i64) as usize;
                if to == n {
                    // The "+": no slot focused, plug-ins shown, this tab kept.
                    self.slot = None;
                    self.slot_of = Some(self.chain());
                    self.channel = false;
                    return;
                }
                match self.chain() {
                    deckard_doc::Chain::Sound(g, s) => Act::FocusSlot(g, s, to),
                    c => Act::FocusChainSlot(c, to),
                }
            }
            R::Load { entry, replace } => {
                let Some((_, what)) = self.plugin_choices().into_iter().nth(entry) else { return };
                let n = self.chain_slots().len();
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
                self.prop_page = self.kept_prop_page(self.prop_set);
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
            // VARIATION: the Groove page last shown (Swing, Variation or Generate, on
            // any sound), else page `n`.
            R::GroovePage(n) => {
                layout::apply_acts(self, vec![Act::ChainTab(layout::ChainTab::Sound)]);
                self.channel = true;
                self.prop_set = 2;
                let page = *self.groove_page.get_or_insert(n.min(2));
                self.prop_page = page;
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
            if let Some(j) = &self.jam {
                j.set_output(out.sender());
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
        // Edit ▸ MIDI Change's scenes and sections, as Lock's.
        if let Some(doc) = &self.doc {
            let d = doc.borrow();
            let cfg = |r: &deckard_model::MidiRecall| {
                (r.trigger != 0).then(|| midi_io::LockMidi {
                    source: r.source.clone(),
                    channel: r.channel.clamp(1, 16) as u8,
                    trigger: r.trigger,
                })
            };
            self.midi.set_recalls(cfg(&d.project().scene_recall), cfg(&d.project().section_recall));
        }
        for r in self.midi.take_recalls() {
            self.midi_recall(r);
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

    /// Open the interface input while a sound takes audio from it or a Zhora listens
    /// (and an engine to play it through) or a controller shows its meters; close it
    /// when none does.
    pub(crate) fn input_sync(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let plays = deckard_engine::project_takes_input(doc.borrow().project());
        let wants = plays || self.input_watched;
        self.input_bus.set_levels(&self.settings.input_levels);
        let key = (self.settings.input_device_in_use(), self.settings.input_routing.clone());
        if !wants {
            self.live_input = None;
            self.settings_input_failed = None;
            return;
        }
        // Open already, or failed with these settings (no retrying every frame).
        if self.live_input.as_ref().is_some_and(|(_, k)| *k == key) || self.settings_input_failed.as_ref() == Some(&key) {
            // Open for a meter, and now a sound takes it: it needs an engine to play through.
            if plays
                && std::mem::take(&mut self.input_for_meters)
                && let Some(Ok(out)) = &self.output
                && !out.has_engine()
            {
                let d = doc.borrow();
                let (mut engine, _) = Engine::new(d.project(), self.path.as_deref(), out.rate, Mode::Arrangement, &mut self.samples);
                engine.sequencing = false;
                out.play(engine);
            }
            return;
        }
        let Ok(out) = self.output.get_or_insert_with(|| open_output(&self.settings, &mut self.output_fallback)) else { return };
        let live: deckard_engine::LiveIn = Default::default();
        match open_live_input(&key.0, &key.1, live.clone(), self.input_bus.clone(), out.rate) {
            Ok(stream) => {
                out.set_live_in(live);
                if plays && !out.has_engine() {
                    let d = doc.borrow();
                    let (mut engine, _) = Engine::new(d.project(), self.path.as_deref(), out.rate, Mode::Arrangement, &mut self.samples);
                    engine.sequencing = false;
                    out.play(engine);
                }
                self.live_input = Some((stream, key));
                self.input_for_meters = !plays;
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
        self.remember_libraries();
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
        self.lock_hosted_sync();
        self.follow_group_moves();
        let Some(doc) = self.doc.clone() else { return };
        // A group gone moves the numbers of those after it, and plug-ins go by number.
        let groups = doc.borrow().project().groups.len();
        if groups < self.midi_input_groups {
            self.clear_midi_inputs();
            self.status = "a group was deleted: MIDI plug-ins were taken out".into();
        }
        self.midi_input_groups = groups;
        let d = doc.borrow();
        let Some(Ok(out)) = &self.output else { return };
        // An edit that reached the engine while it switched modes is sent again.
        let (mode, stale) = out.telemetry(|t| (t.mode, t.stale && t.actions >= self.adopt_sent));
        if d.revision() == self.seen_revision && !stale {
            return;
        }
        self.remember_libraries();
        let Some(Ok(out)) = &self.output else { return };
        // The mode it plays, once it has published one.
        let Some(mode) = mode.filter(|_| out.has_engine()) else { return };
        self.seen_revision = d.revision();
        // Built against what the playing engine has (when it is the one we last
        // edited): only what changed is made.
        let plays = out.plays();
        // (An edit sent again because playback switched mode meanwhile is built in
        // full: its timelines are what didn't arrive.)
        let prev = self.built.as_ref().filter(|b| b.0 == plays && !stale).map(|b| (&b.1, b.2.as_slice()));
        let (mut fresh, report) = Engine::new_after(d.project(), prev, self.path.as_deref(), out.rate, mode, &mut self.samples);
        for (a, b) in self.group_moves.pending.drain(..) {
            fresh.prepare_group_swap(a, b);
        }
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
        self.add_heard_strips();
        let fall = 0.5f32.powf(dt / 0.15);
        let (fresh, master) = match &self.output {
            Some(Ok(out)) => out.take_meters(),
            _ => (Vec::new(), (0.0, 0.0)),
        };
        let cue = match &self.output {
            Some(Ok(out)) => out.take_cue_peak(),
            _ => (0.0, 0.0),
        };
        self.cue_meter = ((self.cue_meter.0 * fall).max(cue.0), (self.cue_meter.1 * fall).max(cue.1));
        for (pair, shown) in self.input_meters.iter_mut().enumerate() {
            let peak = self.input_bus.take_peaks(pair);
            *shown = ((shown.0 * fall).max(peak.0), (shown.1 * fall).max(peak.1));
        }
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
        // The extra outputs' strips, one meter per strip.
        let extra = match &self.output {
            Some(Ok(out)) => out.take_extra_meters(),
            _ => Vec::new(),
        };
        self.sync_extra_meters();
        for (i, m) in self.extra_meters.iter_mut().enumerate() {
            let v = extra.get(i).copied().unwrap_or((0.0, 0.0));
            *m = ((m.0 * fall).max(v.0), (m.1 * fall).max(v.1));
            let p = v.0.max(v.1);
            if p > 0.0 {
                let h = self.headroom.entry(layout::Channel::Extra(i)).or_insert(0.0);
                *h = h.max(p);
            }
        }
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
        levels.extend(self.extra_meters.iter().enumerate().map(|(i, m)| (layout::Channel::Extra(i), *m)));
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
        active |=
            self.recorder.busy() || self.recorder.is_metering() || self.recorder.view.meter.0 > 0.001 || self.recorder.view.meter.1 > 0.001;
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
        self.lock_end_morph();
        let Some(doc) = self.doc.clone() else { return };
        let ok = doc.borrow_mut().undo();
        if !ok && !self.back_to_replaced() {
            self.status = "nothing to undo".into();
        }
        self.lock_hosted_sync();
    }

    fn redo(&mut self) {
        self.lock_end_morph();
        if let Some(doc) = &self.doc {
            let ok = doc.borrow_mut().redo();
            if !ok {
                self.status = "nothing to redo".into();
            }
        }
        self.lock_hosted_sync();
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
        self.open_saved_zone();
        self.poll_stems();
        self.update_loop_peaks();
        // The focused Audio plug-in take's frames and rate (its tempo shows on its page).
        let take_key = self.doc.as_ref().and_then(|doc| {
            let d = doc.borrow();
            let a = d.project().groups.get(self.group)?.sounds.get(self.sound)?.audio.as_ref()?;
            let t = a.takes.get(a.focused)?;
            Some(((self.group, self.sound, a.focused, t.sample.clone()), d.path().map(Path::to_path_buf)))
        });
        match take_key {
            Some((key, path)) => {
                if self.take_info_key.as_ref() != Some(&key) {
                    let (g, s, k, ref smp_ref) = key;
                    self.take_info = self.samples.sample(smp_ref, path.as_deref()).map(|smp| ((g, s, k), smp.frames.len(), smp.rate));
                    self.take_info_key = Some(key);
                }
            }
            None => {
                self.take_info = None;
                self.take_info_key = None;
            }
        }
        let clock = self.record_clock();
        let output = match &self.output {
            Some(Ok(o)) => Some(o),
            _ => None,
        };
        // The input meter moves while a RECORD tab shows (the app's, or the controller's
        // Sampling page), before recording too.
        let hw_record = matches!(&self.controller, Some(Ok(c)) if c.sampling() && c.host.state.borrow().sampling_tab == 0)
            || (self.mk3.is_some() && self.surface.page() == "sampling" && self.sampling.tab == 0);
        let listen = self.meter_inputs && ((self.sampling_open && self.sampling.tab == 0) || hw_record);
        self.recorder.meter(
            listen,
            recorder::StartWith {
                output,
                device: self.settings.input_device_in_use(),
                routing: &self.settings.input_routing,
                inputs: self.input_bus.clone(),
                clock,
                dir: PathBuf::new(),
                stem: String::new(),
                focus: (self.group, self.sound),
            },
        );
        if let Some(rec) = self.recorder.poll(output, clock) {
            self.finish_recording(rec);
        }
        self.silence_while_recording();
        self.sync_sampling_page();
        self.sync_browse_page();
        self.sync_arp();
        self.sync_auto_write();
        self.tick_mod_take(self.playhead().map(|h| (h.mode, h.position)), std::time::Instant::now());
        if !self.sampling_open {
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        let focused = self.focused_sample();
        // An Audio plug-in's take: RECORD and EDIT only.
        if focused.as_ref().is_some_and(|f| f.2.is_some()) && self.sampling.tab > 1 {
            self.sampling.tab = 1;
        }
        let zone = focused.map(|(k, sref, _)| (k, sref));
        let path = doc.borrow().path().map(Path::to_path_buf);
        match zone {
            Some((zi, sref)) => {
                let key = (self.group, self.sound, zi, sref.path.clone());
                if self.sampling.sample_key.as_ref() != Some(&key) {
                    match self.samples.sample(&sref, path.as_deref()) {
                        Some(s) => {
                            let frames = s.frames.len();
                            self.sampling.set_sample(key, s);
                            // A sample that comes up has its play range (or what was saved) selected.
                            if self.sampling.selection.is_none() {
                                let d = doc.borrow();
                                let z = d.project().groups.get(self.group).and_then(|g| g.sounds.get(self.sound));
                                let z = z.and_then(|x| x.sampler.as_ref()).and_then(|x| x.zones.get(zi));
                                self.sampling.selection = z.and_then(|z| layout::sampling::first_selection(z, frames));
                            }
                        }
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
    /// Sound `s`'s Audio plug-in loop in the focused pattern for the controller's pattern
    /// editors: the take on there (else the focused one, off), at a column for every
    /// four of the app's peaks.
    fn loop_view(&self, p: &deckard_model::Project, g: usize, s: usize) -> Option<deckard_surface::LoopView> {
        let snd = p.groups.get(g)?.sounds.get(s)?;
        let a = snd.audio.as_ref().filter(|_| snd.instrument == "AudioModule")?;
        let cue = p.groups[g].patterns.get(self.pattern).and_then(|pt| pt.audio.iter().find(|c| c.sound == s));
        let k = cue.map_or(a.focused, |c| c.take);
        let take = a.takes.get(k)?;
        let ticks = (take.beats * PPQ as f64).round() as i64;
        let (_, _, peaks) = self.loop_peaks.get(&(g, s, k))?;
        if ticks <= 0 || peaks.is_empty() {
            return None;
        }
        let fold = |c: &[(f32, f32)]| c.iter().fold((0f32, 0f32), |(lo, hi), &(a, b)| (lo.min(a), hi.max(b)));
        Some(deckard_surface::LoopView {
            peaks: std::sync::Arc::new(peaks.chunks(4).map(fold).collect()),
            ticks,
            offset: cue.map_or(0, |c| c.offset),
            on: cue.is_some(),
            gate: a.mode == deckard_model::AudioMode::Gate,
        })
    }

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

    /// A sound newly focused opens on the zone it was saved focused on, as Maschine does.
    fn open_saved_zone(&mut self) {
        let at = (self.group, self.sound);
        if self.zone_of == Some(at) {
            return;
        }
        self.zone_of = Some(at);
        let Some(doc) = &self.doc else { return };
        let doc = doc.borrow();
        let saved = doc.project().groups.get(at.0).and_then(|g| g.sounds.get(at.1)).and_then(|x| x.sampler.as_ref());
        self.zone = saved.map_or(0, |x| x.focused_zone);
    }

    /// The focused zone's range to edit: the selection, else the play range [a, b).
    fn edit_range(&self) -> Option<(usize, usize)> {
        if let Some(sel) = self.sampling.selection {
            return Some(sel);
        }
        let doc = self.doc.as_ref()?.borrow();
        // The focused zone's play range, or an Audio plug-in's take's.
        let (_, z, ..) = layout::sampling::edited(doc.project(), (self.group, self.sound, self.zone))?;
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
                        let (g, s) = (self.group, self.sound);
                        self.place_take(g, s, &p);
                    }
                }
                RecordAct::DeleteTake(i) => {
                    if i < self.recorder.view.takes.len() {
                        let path = self.recorder.view.takes.remove(i);
                        self.recorder.view.take_waves.remove(&path);
                        self.unload_take(&path);
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
        let tempo = self.playing_scene_tempo(p).map_or(p.tempo, |t| t.1);
        recorder::Clock { playing, position, length, tempo, grid, bar }
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
            crate::layout::dialog_style(ui);
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
            egui::Grid::new("arp-settings").num_columns(2).spacing([20.0, 10.0]).show(ui, |ui| {
                for k in view.knobs.iter().filter(|k| k.key != "arp/preset") {
                    ui.label(&k.name);
                    match view.lists.iter().find(|(key, _)| *key == k.key) {
                        Some((_, list)) => {
                            crate::layout::combo(&k.key).selected_text(&k.text).show_ui(ui, |ui| {
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
                // The focused zone, or an Audio plug-in's take.
                let t = self
                    .doc
                    .as_ref()
                    .and_then(|d| layout::sampling::edited(d.borrow().project(), (self.group, self.sound, self.zone)))
                    .map_or(deckard_doc::Target::Zone(self.group, self.sound, self.zone), |e| e.2);
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
            // A slice's pad: hear the slice (C63).
            C::Prehear(a, b) => {
                if let Some(part) = self.sampling_part(a, b) {
                    self.ensure_engine();
                    if let Some(Ok(out)) = &self.output {
                        out.prehear(Some(std::sync::Arc::new(part)), 1.0);
                    }
                }
                return;
            }
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
            device: self.settings.input_device_in_use(),
            routing: &self.settings.input_routing,
            inputs: self.input_bus.clone(),
            clock,
            dir,
            stem,
            focus: (self.group, self.sound),
        });
    }

    /// A finished recording: Detect and Sync load the take into the focused sound; Loop
    /// into its target; Auto maps its notes into a Sampler.
    fn finish_recording(&mut self, f: recorder::Finished) {
        match f {
            recorder::Finished::Take { sample, channels, mode, ticks, focus } => {
                let path = self.sample_dir("Recordings").join(recorder::take_name());
                let steps = |app: &Self| app.doc.as_ref().map_or(0, |d| d.borrow().undo_labels().len());
                // (From before the old take was switched off, if it was.)
                let base = self.rec_base.take();
                let before = steps(self);
                if let Err(e) = deckard_engine::edit::write_wav_channels(&path, &sample, channels) {
                    self.recorder.view.error = Some(format!("can't save the recording: {e}"));
                    return;
                }
                self.recorder.view.take_waves.insert(path.clone(), recorder::mini_wave(&sample.frames));
                self.recorder.view.takes.push(path.clone());
                self.recorder.view.focused = Some(self.recorder.view.takes.len() - 1);
                if mode == 2 {
                    self.finish_loop(&path, sample.frames.len(), sample.rate, ticks, focus);
                } else {
                    let (g, s) = focus;
                    if self.is_audio_sound(g, s) {
                        // An Audio plug-in keeps its place: the take is one more of its takes. Detect
                        // rounds its length to a sixteenth; Sync keeps its bars.
                        let sixteenth = (PPQ / 4) as f64;
                        let ticks = if mode == 0 { (ticks / sixteenth).round().max(1.0) * sixteenth } else { ticks };
                        self.put_audio_take(g, s, &path, sample.frames.len(), sample.rate, ticks, 0, true, "Record");
                    } else {
                        self.load_take(g, s, &path, sample.frames.len());
                    }
                }
                if let Some(doc) = &self.doc
                    && steps(self) > before
                {
                    let d = doc.borrow();
                    let labels = d.undo_labels();
                    self.take_steps.insert(path.clone(), (labels.len(), labels[0].to_string(), base.unwrap_or(before)));
                }
                self.sampling.tab = 0;
            }
            recorder::Finished::Auto { done, focus } => self.finish_auto(done, focus),
        }
    }

    /// A take recording onto an Audio sound (Detect, Sync, or Loop's Target Take) stops
    /// that sound's old take the moment recording starts; a cancel puts it back.
    fn silence_while_recording(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let steps = || doc.borrow().undo_labels().len();
        let v = &self.recorder.view;
        if !self.recorder.busy() {
            // Cancelled (a finished take took `rec_base` already): back as it was.
            if let Some(base) = self.rec_base.take() {
                while steps() > base && doc.borrow().undo_labels().first() == Some(&"Record") {
                    self.undo();
                }
            }
            return;
        }
        if self.rec_base.is_some() || v.status != layout::sampling::RecStatus::Recording || (v.mode == 2 && v.target != 0) || v.mode == 3 {
            return;
        }
        let Some((g, s)) = self.recorder.focus() else { return };
        self.rec_base = Some(steps());
        let pi = self.pattern_in(g);
        let on = {
            let d = doc.borrow();
            let p = d.project();
            p.groups.get(g).is_some_and(|gr| {
                gr.sounds.get(s).is_some_and(|x| x.audio.is_some() && x.instrument == "AudioModule")
                    && gr.patterns.get(pi).is_some_and(|pt| pt.audio.iter().any(|c| c.sound == s))
            })
        };
        if on && let Err(e) = doc.borrow_mut().set_audio_in(g, s, &[pi], None, "Record") {
            self.status = format!("can't stop the old take: {e}");
        }
    }

    /// A deleted recording stops playing: while its recording is still the last edit,
    /// that's undone (the sound, pattern and take as they were); after other edits, an
    /// Audio plug-in take is switched off in every pattern that plays it.
    fn unload_take(&mut self, path: &Path) {
        let Some(doc) = self.doc.clone() else { return };
        let last = {
            let d = doc.borrow();
            let labels = d.undo_labels();
            self.take_steps
                .get(path)
                .filter(|(n, label, _)| labels.len() == *n && labels.first() == Some(&label.as_str()))
                .map(|&(n, _, base)| n.saturating_sub(base).max(1))
        };
        self.take_steps.remove(path);
        if let Some(steps) = last {
            for _ in 0..steps {
                self.undo();
            }
            return;
        }
        let Some(file) = path.file_name().map(|f| f.to_string_lossy().to_string()) else { return };
        let named = |p: &str| p.rsplit(['/', '\\']).next() == Some(file.as_str());
        let mut off = Vec::new();
        {
            let d = doc.borrow();
            for (g, gr) in d.project().groups.iter().enumerate() {
                for (s, snd) in gr.sounds.iter().enumerate() {
                    let Some(a) = &snd.audio else { continue };
                    for (k, t) in a.takes.iter().enumerate() {
                        if !named(&t.sample.path) {
                            continue;
                        }
                        let pats: Vec<usize> = gr
                            .patterns
                            .iter()
                            .enumerate()
                            .filter(|(_, p)| p.audio.iter().any(|c| c.sound == s && c.take == k))
                            .map(|(i, _)| i)
                            .collect();
                        if !pats.is_empty() {
                            off.push((g, s, pats));
                        }
                    }
                }
            }
        }
        if off.is_empty() {
            self.status = format!("{file} isn't playing; Undo takes a recording back");
            return;
        }
        let done = doc.borrow_mut().transaction("Delete Take", |d| {
            for (g, s, pats) in &off {
                d.set_audio_in(*g, *s, pats, None, "Delete Take")?;
            }
            Ok(())
        });
        if let Err(e) = done {
            self.status = format!("can't switch the take off: {e}");
        }
    }

    /// Whether sound `s` of group `g` is an Audio plug-in sound.
    fn is_audio_sound(&self, g: usize, s: usize) -> bool {
        self.doc.as_ref().is_some_and(|doc| {
            doc.borrow()
                .project()
                .groups
                .get(g)
                .and_then(|x| x.sounds.get(s))
                .is_some_and(|x| x.audio.is_some() && x.instrument == "AudioModule")
        })
    }

    /// The first sound from `from` on (the focused one, if it's empty) in group `g` with
    /// nothing loaded, else (a Loop take,
    /// `audio_last`) the group's last Audio plug-in, else its last sound (Maschine's
    /// `isSoundAvailableForRecording` and the manual's "last Sound of the Group that
    /// contains an Audio plug-in").
    fn next_free_sound(&self, g: usize, from: usize, audio_last: bool) -> usize {
        let Some(doc) = &self.doc else { return from };
        let d = doc.borrow();
        let Some(gr) = d.project().groups.get(g) else { return from };
        (from..gr.sounds.len())
            .find(|&s| gr.sounds[s].slots.is_empty() && gr.sounds[s].sampler.is_none())
            .or_else(|| audio_last.then(|| gr.sounds.iter().rposition(|x| x.instrument == "AudioModule")).flatten())
            .unwrap_or(gr.sounds.len().saturating_sub(1))
    }

    /// The pattern focused in group `g`: the app's, or what the group plays in the scene.
    fn pattern_in(&self, g: usize) -> usize {
        if g == self.group { self.pattern } else { self.scene_pattern(g).unwrap_or(0) }
    }

    /// Loop mode: the take goes to the focused sound (Take) or the next free one (Sound,
    /// Pattern), and plays from the start of the pattern (a new pattern for Pattern),
    /// which grows to fit it. As the manual has it, into an Audio plug-in (Take: one
    /// more take of the sound's), in time with the song; a sound with a Sampler keeps it
    /// (the take its zone, a note in the pattern playing it).
    fn finish_loop(&mut self, path: &Path, frames: usize, rate: u32, ticks: f64, focus: (usize, usize)) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, from) = focus;
        let target = self.recorder.view.target;
        let s = if target == 0 { from } else { self.next_free_sound(g, from, true) };
        let sampler = doc.borrow().project().groups.get(g).and_then(|x| x.sounds.get(s)).is_some_and(|x| x.sampler.is_some());
        if !sampler {
            self.put_audio_take(g, s, path, frames, rate, ticks, target, true, "Record Loop");
            return;
        }
        self.load_take(g, s, path, frames);
        let focused = self.pattern_in(g);
        let mut pi = focused;
        if target == 2 {
            let n = doc.borrow().project().groups.get(g).map_or(0, |x| x.patterns.len());
            let made = doc.borrow_mut().add_pattern(g, focused, &format!("Pattern {}", n + 1));
            match made {
                Ok(i) => {
                    pi = i;
                    if !self.song_view
                        && let Err(e) = doc.borrow_mut().assign_pattern(self.scene, g, Some(i))
                    {
                        self.status = format!("the new pattern isn't in the scene: {e}");
                    }
                    if g == self.group {
                        self.pattern = i;
                    }
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
        // As `load_take`: the focus moves only in the group on show.
        if g == self.group {
            self.sound = s;
        }
    }

    /// The scene a new pattern has to be in to be heard: the focused one; in the Song
    /// view, the scene of the section playing, else of the focused section.
    fn heard_scene(&self) -> usize {
        if !self.song_view {
            return self.scene;
        }
        let Some(doc) = &self.doc else { return self.scene };
        let d = doc.borrow();
        let p = d.project();
        let playing = self.playhead().filter(|h| h.mode == Mode::Arrangement).and_then(|h| p.section_at(h.position as i64));
        playing.or_else(|| self.focused_section(p)).and_then(|i| p.sections[i].scene).unwrap_or(self.scene)
    }

    /// Put a take in sound `s` of group `g` as an Audio plug-in's: one more take of the
    /// sound's own on an Audio sound (the focused one, or the last one when no sound is
    /// free), else its only one; playing from the
    /// start of the pattern (a new pattern for target 2), which grows to hold `ticks`.
    /// `rename`: the sound takes the file's name unless it was named by hand.
    #[allow(clippy::too_many_arguments)]
    fn put_audio_take(
        &mut self,
        g: usize,
        s: usize,
        path: &Path,
        frames: usize,
        rate: u32,
        ticks: f64,
        target: usize,
        rename: bool,
        label: &str,
    ) {
        let loop_rec = label == "Record Loop";
        let Some(doc) = self.doc.clone() else { return };
        let audio = self.is_audio_sound(g, s);
        let spec = deckard_doc::TakeSpec { file: path.to_path_buf(), frames, rate, beats: ticks / PPQ as f64 };
        let (scene, song_view, focused) = (self.scene, self.song_view, self.pattern_in(g));
        let heard = self.heard_scene();
        let mut made = None;
        let done = doc.borrow_mut().transaction(label, |d| {
            // An Audio sound keeps its chain (effects); any other becomes a lone Audio plug-in.
            if !audio {
                d.audio_only(g, s)?;
            }
            let k = if audio {
                d.add_take(g, s, &spec, label)?
            } else {
                d.set_takes(g, s, std::slice::from_ref(&spec), label)?;
                0
            };
            let mut pi = focused.min(d.project().groups[g].patterns.len().saturating_sub(1));
            if d.project().groups[g].patterns.is_empty() {
                // The group's first: "Pattern 1", in the scene being heard, as drawing a
                // note makes it, so the take plays when the recording ends.
                pi = d.add_pattern(g, usize::MAX, "Pattern 1")?;
                play_in_scene(d, heard, g, pi)?;
                made = Some(pi);
            } else if target == 2 {
                let n = d.project().groups[g].patterns.len();
                pi = d.add_pattern(g, focused, &format!("Pattern {}", n + 1))?;
                if !song_view {
                    d.assign_pattern(scene, g, Some(pi))?;
                }
                made = Some(pi);
            }
            d.set_audio_in(g, s, &[pi], Some(k), label)?;
            // Loop recording is heard at once ("so that you can immediately hear the
            // results"): an Audio plug-in in Gate mode goes to Loop.
            if loop_rec
                && let Some(snd) = d.project().groups[g].sounds.get(s)
                && snd.audio.as_ref().is_some_and(|a| a.mode == deckard_model::AudioMode::Gate)
                && let Some(i) = snd.slots.iter().position(|x| x.module == "AudioModule")
            {
                d.set(label, deckard_doc::Target::Module(g, s, i), "mode", mxprj::decode::Value::Float(0.0))?;
            }
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
            let name = d.project().groups[g].sounds[s].name.clone();
            if rename
                && (name.is_empty() || name.starts_with("Sound ") || name.parse::<u32>().is_ok())
                && let Some(stem) = path.file_stem()
            {
                d.rename(deckard_doc::Target::Sound(g, s), &stem.to_string_lossy())?;
            }
            Ok(())
        });
        match done {
            Ok(()) => {
                // As `load_take`: the focus moves only in the group on show.
                if g == self.group {
                    if let Some(pi) = made {
                        self.pattern = pi;
                    }
                    self.sound = s;
                    self.zone = 0;
                }
            }
            Err(e) => self.status = format!("can't load the take: {e}"),
        }
    }

    /// Auto: the recorded notes as zones of a Sampler, in the focused sound if it has
    /// one, else in the next free sound; then the Zone tab.
    /// `focus`: the group and sound focused at Start.
    fn finish_auto(&mut self, done: Vec<recorder::AutoTake>, focus: (usize, usize)) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, from) = focus;
        let has_sampler = doc.borrow().project().groups.get(g).and_then(|x| x.sounds.get(from)).is_some_and(|x| x.sampler.is_some());
        let s = if has_sampler { from } else { self.next_free_sound(g, from, false) };
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
                // As `load_take`: the focus moves only in the group on show.
                if g == self.group {
                    self.sound = s;
                    self.zone = existing;
                    self.sampling.tab = 3;
                }
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

    /// The sample search uses the same folders the library browser scans.
    fn remember_libraries(&mut self) {
        self.samples.library_dirs = self.library_roots().into_iter().map(|r| r.path).collect();
    }

    /// The folders DECKARD's library indexes: Maschine's user folders (Maschine 3's,
    /// and Maschine 2's beside it) and NI's User Content (NKS presets), listed under User, and the library folders from
    /// Preferences, under Factory, each its folder's name (less " Library") as product.
    fn library_roots(&self) -> Vec<deckard_library::user::Root> {
        use deckard_library::user::Root;
        let user = self.user_dir();
        let m2 = user.parent().map(|p| p.join("Maschine 2"));
        let content = self.user_content_dir();
        let mut roots: Vec<Root> = [Some(user), m2, Some(content)]
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
        // A hosted plug-in's preset is an NKS preset in NI's User Content, in a folder
        // named after the plug-in, filed under it (Maschine's `NKSRoundtrip::saveAs`).
        let nks = self.nks_plugin(what).map(|n| n.plugin.name);
        let before = self.saved.get(&what).map(|s| &s.info);
        self.save_dialog = Some(SaveDialog {
            what,
            name,
            types: before.map(|i| i.types().iter().map(|t| t.join(" › ")).collect::<Vec<_>>().join("; ")).unwrap_or_default(),
            characters: before.map(|i| i.characters().join(", ")).unwrap_or_default(),
            library: before.and_then(|i| i.bank.first().cloned()).or_else(|| nks.clone()).unwrap_or_else(|| "User".into()),
            folder: match &nks {
                Some(plugin) => self.user_content_dir().join(plugin.replace(['/', '\\', ':'], "-")),
                None => self.user_dir().join(folder),
            },
            samples: mode == SaveMode::WithSamples,
            confirm_overwrite: false,
        });
    }

    /// The hosted plug-in in slot `what` when its preset saves as NKS: a VST3 plug-in
    /// (or a VST2 one, hosted through its VST3 version), as Maschine's `canSaveAsNKS`.
    /// An AU's preset, and an internal module's, stay Maschine's own files.
    fn nks_plugin(&self, what: deckard_doc::FileItem) -> Option<NksPlugin> {
        use deckard_plugins::state::{Format, PluginId};
        let deckard_doc::FileItem::Slot(g, s, k) = what else { return None };
        let doc = self.doc.as_ref()?.borrow();
        let slot = doc.project().groups.get(g)?.sounds.get(s)?.slots.get(k)?;
        let plugin = slot.plugin.clone()?;
        let id = PluginId::parse(&plugin.id, &plugin.name)?;
        // DECKARD's own modules are stored as VST3 plug-ins, but they're internal
        // modules: their presets are Maschine's own files, which NI's apps load.
        if deckard_plugins::native::find(&id.cid).is_some() {
            return None;
        }
        let ids = deckard_plugins::nks::PluginIds {
            vst3: (id.format == Format::Vst3).then_some(id.cid),
            vst2: if id.format == Format::Vst2 { Some(plugin.id.rsplit(':').next()?.parse().ok()?) } else { None },
            name: plugin.name.clone(),
            vendor: id.vendor,
        };
        (id.format != Format::AudioUnit).then(|| NksPlugin { ids, instrument: slot.is_instrument(), plugin })
    }

    /// NI's User Content folder, beside Maschine 3's, where NKS presets are saved.
    fn user_content_dir(&self) -> PathBuf {
        let user = self.user_dir();
        user.parent().map_or_else(|| user.join("User Content"), |ni| ni.join("User Content"))
    }

    /// The knob pages an NKS preset of slot `k` of `chain` gets: those shown (a
    /// plug-in's automatic ones too), else those saved with it; a page's name is its
    /// first knob's section when that has none (NKS pages have no names of their own).
    fn nks_pages(&self, chain: deckard_doc::Chain, k: usize, plugin: &deckard_model::PluginRef) -> Vec<Vec<deckard_plugins::nks::Knob>> {
        use deckard_plugins::nks::Knob;
        let knob = |i: usize, id: u32, name: &str, section: &str, page: &str| Knob {
            id: Some(id),
            name: name.to_string(),
            section: if i == 0 && section.is_empty() { page.to_string() } else { section.to_string() },
        };
        match self.plugin_views.get(&(chain, k)) {
            Some(v) => {
                v.pages.iter().map(|pg| pg.knobs.iter().enumerate().map(|(i, (p, l, s))| knob(i, p.id, l, s, &pg.name)).collect()).collect()
            }
            None => plugin
                .pages
                .iter()
                .map(|pg| pg.params.iter().enumerate().map(|(i, p)| knob(i, p.id, &p.name, &p.section, &pg.name)).collect())
                .collect(),
        }
    }

    /// Write slot `what`'s hosted plug-in as the NKS preset `s` says (its state as it is
    /// now). Saving the same file again keeps its `uuid`; a new file gets a new one.
    fn write_nks(&mut self, what: deckard_doc::FileItem, s: Saved) -> bool {
        use deckard_plugins::nks;
        let deckard_doc::FileItem::Slot(g, sd, k) = what else { return false };
        let key = self.instance_key(g, sd, k);
        self.store_plugin_states_of(|x| *x == key);
        let Some(NksPlugin { ids, instrument, plugin }) = self.nks_plugin(what) else { return false };
        let again = self.saved.get(&what).is_some_and(|o| o.path == s.path);
        let uuid = again
            .then(|| std::fs::read(&s.path).ok().and_then(|b| nks::parse(&b).ok()))
            .flatten()
            .map(|p| p.info.uuid)
            .filter(|u| !u.is_empty())
            .unwrap_or_else(nks::new_uuid);
        let mut bankchain = s.info.bank.clone();
        bankchain.resize(bankchain.len().max(3), String::new());
        let info = nks::SoundInfo {
            author: s.info.author.clone(),
            bankchain,
            characters: s.info.characters(),
            comment: String::new(),
            device_type: if instrument { "INST" } else { "FX" }.into(),
            modes: Vec::new(),
            name: s.info.name.clone(),
            types: s.info.types(),
            uuid,
            vendor: ids.vendor.clone(),
        };
        let pages = self.nks_pages(deckard_doc::Chain::Sound(g, sd), k, &plugin);
        let written = deckard_plugins::state::chunk_parts(&plugin.chunk).and_then(|(_, state)| {
            let bytes = nks::write(&info, &pages, &ids, state);
            let dir = s.path.parent().ok_or("no folder to save in")?;
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let tmp = s.path.with_extension("deckard-tmp");
            std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, &s.path)).map_err(|e| e.to_string())
        });
        match written {
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

    /// The Save dialog's Save: asks first when the file is there (and isn't this
    /// one's own), then writes it.
    fn confirm_save(&mut self) {
        let Some(what) = self.save_dialog.as_ref().map(|d| d.what) else { return };
        let nks = self.nks_plugin(what).map(|n| if n.instrument { "nksf" } else { "nksfx" });
        let Some(dlg) = &mut self.save_dialog else { return };
        let Some(doc) = self.doc.clone() else { return };
        let ext = match dlg.what {
            deckard_doc::FileItem::Sound(..) => "mxsnd",
            deckard_doc::FileItem::Group(_) => "mxgrp",
            deckard_doc::FileItem::Slot(g, s, k) => nks.unwrap_or_else(|| doc.borrow().slot_extension(g, s, k)),
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
        if s.path.extension().is_some_and(|e| e == "nksf" || e == "nksfx") {
            return self.write_nks(what, s);
        }
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
            FileItem::Slot(g, s, k) => std::fs::read(&file).map_err(|e| e.to_string()).and_then(|b| {
                let doc = self.doc.clone().ok_or("no project".to_string())?;
                let mut d = doc.borrow_mut();
                d.transaction("Load Preset", |d| {
                    d.load_slot_file(g, s, k, &b)?;
                    if k == 0 && d.project().groups[g].sounds[s].slots.first().is_some_and(|x| x.is_instrument()) {
                        d.rename(deckard_doc::Target::Sound(g, s), preset_sound_name(&item.name))?;
                    }
                    Ok(())
                })
            }),
            FileItem::Group(g) => self.queue_group_load(&item, g),
            _ => self.queue_item_load(&item, target),
        };
        if let Err(e) = r {
            self.status = e;
        }
    }

    /// What the browser asks for.
    fn browse(&mut self, a: browser::BrowseAct) {
        use browser::BrowseAct as B;
        self.browser.touch();
        if matches!(a, B::Rescan) {
            let roots = self.library_roots();
            self.browser.rescan(roots);
            self.status = "Library refreshed".into();
            return;
        }
        if let B::Folder(dir) = &a
            && !dir.is_dir()
        {
            self.status = format!("{} isn't a folder", dir.display());
            return;
        }
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
                let result = if item.kind == deckard_library::Kind::Group {
                    self.queue_group_load(&item, target.map_or(self.group, |t| t.0))
                } else {
                    self.queue_item_load(&item, target)
                };
                if let Err(e) = result {
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
            B::Routing(on) => {
                b.routing = on;
                self.settings.load_group_routing = on;
                self.settings.save();
                return;
            }
            B::Patterns(on) => {
                b.patterns = on;
                self.settings.load_group_patterns = on;
                self.settings.save();
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
            B::Rescan => return,
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
                let result = if item.kind == deckard_library::Kind::Group {
                    self.queue_group_load(&item, target.map_or(self.group, |t| t.0))
                } else {
                    self.queue_item_load(&item, target)
                };
                if let Err(e) = result {
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

    pub(crate) fn prehear_item(&mut self, item: &deckard_library::Item) {
        let Some(file) = browser::prehear_file(item) else {
            if let Some(Ok(out)) = &self.output {
                out.prehear(None, 0.0);
            }
            return;
        };
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
        // Most of the library's presets are NKS files, or a product's own format.
        if matches!(item.kind, Kind::Instrument | Kind::Effect) {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if matches!(ext.as_str(), "nksf" | "nksfx") {
                let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
                return self.load_nks_preset(&item.name, (g, s), &bytes);
            }
            if !matches!(ext.as_str(), "mxinst" | "mxfx") {
                let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
                if let Some(res) = deckard_plugins::nks::native_preset(&ext, &bytes, &self.catalog.entries) {
                    let plugin = res.map_err(|e| format!("{}: {e}", item.name))?;
                    return self.load_resolved_preset(&item.name, (g, s), plugin);
                }
                return Err(format!("{}: .{ext} presets can't be loaded yet", item.name));
            }
        }
        match item.kind {
            Kind::Group | Kind::Sound | Kind::Instrument | Kind::Effect => {
                let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
                let mut d = doc.borrow_mut();
                let slot = match item.kind {
                    Kind::Group => {
                        d.load_group_file_with(g, &bytes, self.browser.routing, self.browser.patterns)?;
                        None
                    }
                    Kind::Sound => {
                        d.load_sound_file(g, s, &bytes)?;
                        (!d.project().groups[g].sounds[s].slots.is_empty()).then_some(0)
                    }
                    Kind::Instrument => {
                        let slots = &d.project().groups[g].sounds[s].slots;
                        if slots.first().is_some_and(|x| !x.is_instrument()) {
                            return Err("the sound's chain starts with an effect: an instrument can't go before it".into());
                        }
                        d.transaction("Load Preset", |d| {
                            d.load_slot_file(g, s, 0, &bytes)?;
                            d.rename(deckard_doc::Target::Sound(g, s), preset_sound_name(&item.name))?;
                            Ok(())
                        })?;
                        Some(0)
                    }
                    _ => {
                        let n = d.project().groups[g].sounds[s].slots.len();
                        d.load_slot_file(g, s, n, &bytes)?;
                        Some(n)
                    }
                };
                drop(d);
                if matches!(item.kind, Kind::Group) {
                    self.editors
                        .retain(|&(c, ..)| !matches!(c, deckard_doc::Chain::Group(cg) | deckard_doc::Chain::Sound(cg, _) if cg == g));
                    self.slot_of = None;
                } else {
                    let chain = deckard_doc::Chain::Sound(g, s);
                    self.editors.retain(|&(c, i, _)| !(c == chain && (matches!(item.kind, Kind::Sound) || Some(i) == slot)));
                    self.sound = s;
                    self.chain_tab = layout::ChainTab::Sound;
                    self.slot_of = Some(chain);
                    self.slot = slot;
                    if slot.is_some() {
                        // The loaded module, not the sound's channel settings.
                        self.channel = false;
                    }
                }
                self.plugin_page = 0;
                self.plugin_views.clear();
                if self.output.is_some() {
                    self.load_plugins();
                }
                self.status = format!("{} loaded", item.name);
            }
            Kind::Loop => self.drop_samples((g, s), std::slice::from_ref(path)),
            Kind::OneShot => {
                let smp = deckard_engine::sample::Sample::load(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
                doc.borrow_mut().transaction("Load Sample", |d| {
                    let snd = &d.project().groups[g].sounds[s];
                    if snd.sampler.is_none() {
                        if snd.slots.first().is_some_and(|x| x.is_instrument()) {
                            d.remove_slot(g, s, 0)?;
                        }
                        d.add_slot(g, s, 0, &deckard_doc::NewModule::Sampler)?;
                    }
                    let has = d.project().groups[g].sounds[s].sampler.as_ref().is_some_and(|x| !x.zones.is_empty());
                    let spec = new_zone(
                        path,
                        smp.frames.len(),
                        if has { deckard_doc::ZoneFrom::Zone(0) } else { deckard_doc::ZoneFrom::Template },
                    );
                    d.set_zones(g, s, &[spec], "Load Sample")?;
                    if let Some(stem) = path.file_stem() {
                        d.rename(deckard_doc::Target::Sound(g, s), &stem.to_string_lossy())?;
                    }
                    Ok(())
                })?;
                self.editors.retain(|&(c, i, _)| !(c == deckard_doc::Chain::Sound(g, s) && i == 0));
                self.sound = s;
                self.chain_tab = layout::ChainTab::Sound;
                self.slot_of = Some(deckard_doc::Chain::Sound(g, s));
                self.slot = Some(0);
                self.plugin_page = 0;
                self.plugin_views.clear();
                self.zone = 0;
                self.status = format!("{} into the Sampler on sound {}", item.name, s + 1);
            }
            Kind::Project => {}
        }
        Ok(())
    }

    /// Load an NKS preset as its plug-in, with its state: an instrument in place of the
    /// sound's, an effect last in its chain.
    fn load_nks_preset(&mut self, name: &str, (g, s): (usize, usize), bytes: &[u8]) -> Result<(), String> {
        let plugin =
            deckard_plugins::nks::parse(bytes).and_then(|p| p.resolve(&self.catalog.entries)).map_err(|e| format!("{name}: {e}"))?;
        self.load_resolved_preset(name, (g, s), plugin)
    }

    fn load_resolved_preset(&mut self, name: &str, (g, s): (usize, usize), plugin: deckard_plugins::nks::Resolved) -> Result<(), String> {
        let Some(doc) = self.doc.clone() else { return Err("no project".into()) };
        let pages: Vec<deckard_doc::PluginPageSpec> = plugin
            .pages
            .iter()
            .map(|pg| deckard_doc::PluginPageSpec {
                name: String::new(),
                knobs: pg.iter().map(|k| (k.id.unwrap_or(u32::MAX), k.section.clone())).collect(),
                custom_names: pg.iter().map(|k| k.name.clone()).collect(),
            })
            .collect();
        let module = deckard_doc::NewModule::Plugin { id: plugin.id, name: plugin.name, chunk: plugin.chunk };
        let mut d = doc.borrow_mut();
        let (count, first_is_instrument) = {
            let slots = &d.project().groups[g].sounds[s].slots;
            (slots.len(), slots.first().map(|x| x.is_instrument()))
        };
        if module.is_instrument() && first_is_instrument == Some(false) {
            return Err("the sound's chain starts with an effect: an instrument can't go before it".into());
        }
        let slot = if module.is_instrument() { 0 } else { count };
        d.transaction("Load Preset", |d| {
            if module.is_instrument() && first_is_instrument == Some(true) {
                d.remove_slot(g, s, 0)?;
            }
            d.add_slot(g, s, slot, &module)?;
            if !pages.is_empty() {
                d.save_plugin_pages(deckard_doc::Chain::Sound(g, s), slot, &pages)?;
            }
            if module.is_instrument() {
                d.rename(deckard_doc::Target::Sound(g, s), preset_sound_name(name))?;
            }
            Ok(())
        })?;
        drop(d);
        self.editors.retain(|&(c, i, _)| !(c == deckard_doc::Chain::Sound(g, s) && i == slot));
        self.channel = false;
        self.sound = s;
        self.chain_tab = layout::ChainTab::Sound;
        self.slot_of = Some(deckard_doc::Chain::Sound(g, s));
        self.slot = Some(slot);
        self.plugin_page = 0;
        self.plugin_views.clear();
        self.status = format!("{name} loaded");
        Ok(())
    }

    /// Look up a default NKS preset for `plugin_name` in NI's library index, preferring
    /// one whose `NICA` / `DSINy` pages carry section headings.
    fn default_nks_preset(&self, plugin_name: &str) -> Option<deckard_plugins::nks::Resolved> {
        let mut best: Option<deckard_plugins::nks::Resolved> = None;
        for path in deckard_library::ni::nks_presets_for(plugin_name) {
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let resolved = if matches!(ext.to_ascii_lowercase().as_str(), "nksf" | "nksfx") {
                deckard_plugins::nks::parse(&bytes).and_then(|p| p.resolve(&self.catalog.entries)).ok()
            } else {
                deckard_plugins::nks::native_preset(ext, &bytes, &self.catalog.entries).and_then(Result::ok)
            };
            let Some(resolved) = resolved else { continue };
            let has_sections = resolved.pages.iter().any(|pg| pg.iter().any(|k| !k.section.is_empty()));
            if has_sections {
                return Some(resolved);
            }
            if best.is_none() && !resolved.pages.is_empty() {
                best = Some(resolved);
            }
        }
        best
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
        let heard = self.heard_scene();
        doc.borrow_mut().transaction("Load Loop", |d| {
            d.audio_only(g, s)?;
            d.set_takes(g, s, &[spec], "Load Loop")?;
            let pi = match focused.filter(|&p| p < d.project().groups[g].patterns.len()) {
                Some(p) => p,
                None if d.project().groups[g].patterns.is_empty() => {
                    let pi = d.add_pattern(g, usize::MAX, "Pattern 1")?;
                    play_in_scene(d, heard, g, pi)?;
                    pi
                }
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

    /// A recorded take put on sound (`g`, `s`) (a take clicked, or dragged onto a sound's
    /// row): on an Audio sound, that take on in the pattern (added to its takes if it
    /// isn't one); an empty sound gets an Audio plug-in playing it; a Sampler gets it as
    /// its zone. A sound playing an instrument is left alone.
    fn place_take(&mut self, g: usize, s: usize, path: &Path) {
        let Some(doc) = self.doc.clone() else { return };
        let Ok(smp) = deckard_engine::sample::Sample::load(path) else {
            self.status = format!("can't read {}", path.display());
            return;
        };
        let file = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let named = |p: &str| p.rsplit(['/', '\\']).next() == Some(file.as_str());
        // (What the sound is, the take's index there if it has it, and its length in beats
        // wherever the project has it.)
        let (kind, here, beats, tempo) = {
            let d = doc.borrow();
            let p = d.project();
            let Some(snd) = p.groups.get(g).and_then(|x| x.sounds.get(s)) else { return };
            let audio = snd.audio.as_ref().filter(|_| snd.instrument == "AudioModule");
            let kind = if audio.is_some() {
                0
            } else if snd.slots.is_empty() && snd.sampler.is_none() {
                1
            } else if snd.sampler.is_some() {
                2
            } else {
                3
            };
            let here = audio.and_then(|a| a.takes.iter().position(|t| named(&t.sample.path)));
            let beats = p
                .groups
                .iter()
                .flat_map(|gr| gr.sounds.iter())
                .filter_map(|x| x.audio.as_ref())
                .flat_map(|a| a.takes.iter())
                .find(|t| named(&t.sample.path) && t.beats > 0.0)
                .map(|t| t.beats);
            (kind, here, beats, p.tempo)
        };
        let ticks = beats.map_or_else(
            || {
                let sixteenth = (PPQ / 4) as f64;
                let t = smp.frames.len() as f64 / f64::from(smp.rate.max(1)) * tempo / 60.0 * PPQ as f64;
                (t / sixteenth).round().max(1.0) * sixteenth
            },
            |b| b * PPQ as f64,
        );
        match (kind, here) {
            (0, Some(k)) => {
                let pi = self.pattern_in(g);
                if let Err(e) = doc.borrow_mut().set_audio_in(g, s, &[pi], Some(k), "Select Take") {
                    self.status = format!("can't select the take: {e}");
                }
            }
            (0 | 1, _) => self.put_audio_take(g, s, path, smp.frames.len(), smp.rate, ticks, 0, true, "Place Take"),
            (2, _) => self.load_take(g, s, path, smp.frames.len()),
            _ => self.status = "a take goes on an empty sound, an Audio plug-in or a Sampler".into(),
        }
    }

    /// Size the extra outputs' meters to the project's strips; when the strips changed
    /// (one gone, added in between, moved, split, joined or made mono), start every
    /// strip's meter, held peak and headroom again, as they're held by place in the list.
    fn sync_extra_meters(&mut self) {
        let keys: Vec<_> = self
            .doc
            .as_ref()
            .map(|d| d.borrow().project().extra_outs.iter().map(|x| (x.owner, x.bus, x.plugin.clone(), x.channels)).collect())
            .unwrap_or_default();
        if keys == self.extra_meter_keys {
            return;
        }
        // Strips added at the end keep the others' readings.
        let appended = keys.len() >= self.extra_meter_keys.len() && keys[..self.extra_meter_keys.len()] == self.extra_meter_keys[..];
        if !appended {
            self.extra_meters.clear();
            self.peaks.retain(|ch, _| !matches!(ch, layout::Channel::Extra(_)));
            self.headroom.retain(|ch, _| !matches!(ch, layout::Channel::Extra(_)));
        }
        self.extra_meters.resize(keys.len(), (0.0, 0.0));
        self.extra_meter_keys = keys;
    }

    /// Solo channel `ch`: mute the other channels of its level and unmute it, as one
    /// step; soloing it again (the others all muted) unmutes them all. Plug-ins' extra
    /// outputs play on whatever their channel's mute (the engine keeps the plug-in
    /// running for them), so their strips are muted too: soloing a group mutes every
    /// strip but its own and its sounds'; soloing a sound, its group's other sounds'
    /// strips. A strip soloed mutes every other strip and every channel but the group
    /// its plug-in plays in: its own channel's main pair too (the plug-in plays on for
    /// the strip).
    fn solo(&mut self, ch: layout::Channel) {
        use deckard_doc::Target;
        use deckard_model::extra_out::Owner;
        let Some(doc) = self.doc.clone() else { return };
        let project = doc.borrow().project().clone();
        let p = &project;
        // The group a strip's plug-in plays in (none: the master's).
        let group_of = |o: Owner| match o {
            Owner::Sound(g, _) | Owner::Chain(g, _, _) | Owner::Group(g, _) => Some(g),
            Owner::Master(_) => None,
        };
        let groups = || (0..p.groups.len()).map(|g| (Target::Group(g), p.groups[g].muted));
        let sounds =
            |g: usize| (0..p.groups.get(g).map_or(0, |x| x.sounds.len())).map(move |s| (Target::Sound(g, s), p.groups[g].sounds[s].muted));
        // The channels and strips solo mutes (and unmutes again), and the one of each
        // it leaves heard.
        let every: Vec<usize> = (0..p.extra_outs.len()).collect();
        type Solo = (Vec<(Target, bool)>, Option<Target>, Vec<usize>, Option<usize>);
        let (all, me, strips, mine): Solo = match ch {
            layout::Channel::Group(g) => {
                let strips = every.into_iter().filter(|&i| group_of(p.extra_outs[i].owner) != Some(g)).collect();
                (groups().collect(), Some(Target::Group(g)), strips, None)
            }
            layout::Channel::Sound(g, s) => {
                let strips = every
                    .into_iter()
                    .filter(|&i| matches!(p.extra_outs[i].owner, Owner::Sound(a, b) | Owner::Chain(a, b, _) if a == g && b != s))
                    .collect();
                (sounds(g).collect(), Some(Target::Sound(g, s)), strips, None)
            }
            layout::Channel::Master => return,
            layout::Channel::Extra(i) => match p.extra_outs.get(i).map(|x| x.owner) {
                // A sound's plug-in: its group stays heard (the strip may play into it);
                // the other groups and that group's sounds are muted.
                Some(Owner::Sound(g, _) | Owner::Chain(g, _, _)) => {
                    (groups().filter(|(t, _)| *t != Target::Group(g)).chain(sounds(g)).collect(), None, every, Some(i))
                }
                Some(Owner::Group(..)) => (groups().collect(), None, every, Some(i)),
                // The master's plug-in hears the whole mix: the groups stay heard (so
                // does the master's main pair), only the other strips are muted.
                Some(Owner::Master(_)) => (Vec::new(), None, every, Some(i)),
                None => return,
            },
        };
        let soloed = all.iter().all(|(t, m)| if Some(*t) == me { !m } else { *m })
            && strips.iter().all(|&i| if Some(i) == mine { !p.extra_outs[i].muted } else { p.extra_outs[i].muted });
        let mutes: Vec<(Target, bool)> = all.iter().map(|(t, _)| (*t, !soloed && Some(*t) != me)).collect();
        let mut outs = p.extra_outs.clone();
        for &i in &strips {
            outs[i].muted = !soloed && Some(i) != mine;
        }
        let label = if soloed { "Unsolo" } else { "Solo" };
        if let Err(e) = doc.borrow_mut().transaction(label, |d| {
            d.set_mutes(label, &mutes)?;
            d.set_extra_outs(outs)
        }) {
            self.status = format!("can't solo: {e}");
        }
    }

    /// Apply one action as a click would (for tests).
    #[cfg(test)]
    fn arrange_act(&mut self, a: layout::Act) {
        layout::apply_acts(self, vec![a]);
    }

    /// Keep the shown slot on the chain shown, and the plug-in views up to date: the
    /// focused slot's and each strip panel's drawn lately (the others go), their shown
    /// page's values read every 100 ms without waiting on the audio thread.
    fn update_slot_view(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let chain = self.chain();
        let all = self.chain_slots();
        let slots = all.len();
        // Back on a chain, it shows where it was left: its slot (unless one was just
        // picked on it) and its property pages.
        if self.pages_of != Some(chain) {
            let kept = self.chain_pages.get(&chain);
            self.prop_page = kept.map_or(0, |k| k.prop_pages[self.prop_set.min(3)]);
            if let (2, deckard_doc::Chain::Sound(..), Some(p)) = (self.prop_set, chain, self.groove_page) {
                self.prop_page = p;
            }
            if self.slot_of != Some(chain) {
                self.slot_of = Some(chain);
                self.slot = kept.and_then(|k| k.slot).filter(|&i| i < slots).or((slots > 0).then_some(0));
            }
            self.pages_of = Some(chain);
        }
        if self.slot.is_some_and(|i| i >= slots) {
            self.slot = slots.checked_sub(1);
        }
        // A slot focused anew shows the page it was on (the first if none).
        if self.page_key != Some((chain, self.slot)) {
            self.page_key = Some((chain, self.slot));
            self.plugin_page = self.slot.and_then(|i| self.chain_pages.get(&chain)?.slot_pages.get(&i).copied()).unwrap_or(0);
        }
        let now = std::time::Instant::now();
        let focused = self.slot.filter(|&i| all[i].plugin.is_some()).map(|i| (chain, i));
        // Focused anew, a plug-in shows the page its view is on (turned on its strip
        // panel, say).
        if focused != self.plugin_focus {
            self.plugin_focus = focused;
            if let Some(v) = focused.and_then(|k| self.plugin_views.get(&k)) {
                self.plugin_page = v.shown;
            }
        }
        self.keep_pages();
        // The views wanted: the focused slot's and the panels drawn since (still
        // plug-ins); the others go once not seen for a while.
        let drawn = std::mem::take(&mut *self.plugin_drawn.borrow_mut());
        let mut wanted = Vec::new();
        {
            let d = doc.borrow();
            let id = |k: &(deckard_doc::Chain, usize)| Some(d.chain_slots(k.0)?.get(k.1)?.plugin.as_ref()?.id.as_str());
            let holds = |k: &(deckard_doc::Chain, usize)| id(k).is_some();
            for k in focused.into_iter().chain(drawn) {
                if !wanted.contains(&k) && holds(&k) {
                    wanted.push(k);
                }
            }
            for k in &wanted {
                if let Some(v) = self.plugin_views.get_mut(k) {
                    v.seen = now;
                }
            }
            // (Another plug-in in the slot, an undo's say: its own view, made again.)
            self.plugin_views.retain(|k, v| id(k) == Some(v.plugin_id.as_str()) && now.duration_since(v.seen) < VIEW_KEEP);
        }
        // New ones: the focused slot's at once (as the controller wants it), the others
        // a few at a time.
        let mut built = 0;
        for k in wanted {
            if self.plugin_views.contains_key(&k) || (Some(k) != focused && built >= VIEW_READS) {
                continue;
            }
            if let Some(v) = self.build_view(k, Some(k) == focused, now) {
                self.plugin_views.insert(k, v);
                built += 1;
            }
        }
        for v in self.plugin_views.values_mut() {
            if Some(v.key) == focused {
                v.shown = self.plugin_page;
            }
            v.shown = v.shown.min(v.pages.len().saturating_sub(1));
        }
        // Values of the pages shown, every 100 ms (the plug-in's window may move them):
        // a page just turned first, then the oldest; a few each time, past the focused
        // one's and any overdue.
        let mut due: Vec<(bool, std::time::Instant, (deckard_doc::Chain, usize))> = self
            .plugin_views
            .values()
            .filter(|v| v.error.is_none() && (v.page != v.shown || now.duration_since(v.refreshed) >= VIEW_REFRESH))
            .map(|v| (v.page == v.shown, v.refreshed, v.key))
            .collect();
        due.sort_by_key(|&(loaded, at, _)| (loaded, at));
        let mut read = 0;
        for (loaded, at, k) in due {
            let urgent = !loaded || Some(k) == focused || now.duration_since(at) >= VIEW_OVERDUE;
            if read >= VIEW_READS && !urgent {
                continue;
            }
            if self.refresh_view(k, now) {
                read += 1;
            }
        }
    }

    /// Keep where the chain shown is (its slot and pages) for coming back to it.
    fn keep_pages(&mut self) {
        let Some(chain) = self.pages_of else { return };
        let kept = self.chain_pages.entry(chain).or_default();
        kept.slot = self.slot;
        kept.prop_pages[self.prop_set.min(3)] = self.prop_page;
        if self.prop_set == 2 && self.channel && matches!(chain, deckard_doc::Chain::Sound(..)) {
            self.groove_page = Some(self.prop_page);
        }
        if let Some(i) = self.slot {
            kept.slot_pages.insert(i, self.plugin_page);
        }
    }

    /// The app's view as saved with the project (`SavedScreen::view`): one setting a
    /// line, then a `chain` line for each chain's slot and pages.
    fn view_text(&self) -> String {
        let tab = match self.chain_tab {
            layout::ChainTab::Master => "master",
            layout::ChainTab::Group => "group",
            layout::ChainTab::Sound => "sound",
        };
        let mut t = format!(
            "song {}\ntab {tab}\nmixer {}\nmixer_sounds {}\npiano_roll {}\nprops {}\n",
            self.song_view as u8, self.mixer as u8, self.mixer_sounds as u8, self.piano_roll as u8, self.prop_set
        );
        if let Some(p) = self.groove_page {
            t += &format!("groove_page {p}\n");
        }
        let mut chains: Vec<_> = self.chain_pages.iter().collect();
        chains.sort_by_key(|(c, _)| format!("{c:?}"));
        for (c, k) in chains {
            let at = match c {
                deckard_doc::Chain::Sound(g, s) => format!("s {g} {s}"),
                deckard_doc::Chain::Group(g) => format!("g {g}"),
                deckard_doc::Chain::Master => "m".into(),
            };
            let slot = k.slot.map_or("-".into(), |i| i.to_string());
            let [a, b, c, d] = k.prop_pages;
            let mut pages: Vec<_> = k.slot_pages.iter().filter(|&(_, &p)| p > 0).collect();
            pages.sort();
            let pages: Vec<String> = pages.iter().map(|(i, p)| format!("{i}:{p}")).collect();
            t += &format!("chain {at} slot {slot} props {a} {b} {c} {d} pages {}\n", pages.join(","));
        }
        t
    }

    /// Take up a saved [`App::view_text`]; lines that don't read are skipped.
    fn apply_view_text(&mut self, text: &str) {
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let flag = |i: usize| w.get(i).map(|v| *v == "1");
            match w.first().copied() {
                Some("song") => self.song_view = flag(1).unwrap_or(self.song_view),
                Some("mixer") => self.mixer = flag(1).unwrap_or(self.mixer),
                Some("mixer_sounds") => self.mixer_sounds = flag(1).unwrap_or(self.mixer_sounds),
                Some("piano_roll") => self.piano_roll = flag(1).unwrap_or(self.piano_roll),
                Some("groove_page") => {
                    if let Some(n) = w.get(1).and_then(|v| v.parse::<usize>().ok()) {
                        self.groove_page = Some(n.min(2));
                    }
                }
                Some("props") => {
                    if let Some(n) = w.get(1).and_then(|v| v.parse::<usize>().ok()) {
                        self.prop_set = n.min(3);
                    }
                }
                Some("tab") => match w.get(1).copied() {
                    Some("master") => self.chain_tab = layout::ChainTab::Master,
                    Some("group") => self.chain_tab = layout::ChainTab::Group,
                    Some("sound") => self.chain_tab = layout::ChainTab::Sound,
                    _ => {}
                },
                Some("chain") => {
                    let num = |i: usize| w.get(i).and_then(|v| v.parse::<usize>().ok());
                    let (chain, rest) = match w.get(1).copied() {
                        Some("s") => match (num(2), num(3)) {
                            (Some(g), Some(s)) => (deckard_doc::Chain::Sound(g, s), 4),
                            _ => continue,
                        },
                        Some("g") => match num(2) {
                            Some(g) => (deckard_doc::Chain::Group(g), 3),
                            None => continue,
                        },
                        Some("m") => (deckard_doc::Chain::Master, 2),
                        _ => continue,
                    };
                    let mut kept = ChainPages::default();
                    let mut i = rest;
                    while i < w.len() {
                        match w[i] {
                            "slot" => {
                                kept.slot = num(i + 1);
                                i += 2;
                            }
                            "props" => {
                                for (j, p) in kept.prop_pages.iter_mut().enumerate() {
                                    *p = num(i + 1 + j).unwrap_or(0);
                                }
                                i += 5;
                            }
                            "pages" => {
                                for pair in w.get(i + 1).map_or("", |v| *v).split(',') {
                                    if let Some((a, b)) = pair.split_once(':')
                                        && let (Ok(a), Ok(b)) = (a.parse(), b.parse())
                                    {
                                        kept.slot_pages.insert(a, b);
                                    }
                                }
                                i += 2;
                            }
                            _ => i += 1,
                        }
                    }
                    self.chain_pages.insert(chain, kept);
                }
                _ => {}
            }
        }
    }

    /// The page property set `set` of the chain shown was left on (0 if never shown).
    pub(crate) fn kept_prop_page(&self, set: usize) -> usize {
        let chain = self.chain();
        if let (2, deckard_doc::Chain::Sound(..), Some(p)) = (set, chain, self.groove_page) {
            return p;
        }
        self.chain_pages.get(&chain).map_or(0, |k| k.prop_pages[set.min(3)])
    }

    /// The focused slot's plug-in view (what the SOUND tab and the controller show).
    #[cfg(test)]
    pub(crate) fn plugin_view(&self) -> Option<&PluginView> {
        self.plugin_focus.and_then(|k| self.plugin_views.get(&k))
    }

    /// The page slot `key` shows: the focused slot's [`App::plugin_page`], else its own.
    pub(crate) fn view_page(&self, key: (deckard_doc::Chain, usize)) -> usize {
        if self.plugin_focus == Some(key) { self.plugin_page } else { self.plugin_views.get(&key).map_or(0, |v| v.shown) }
    }

    /// A view of the plug-in in slot `key` (its pages, no values yet). `focused`: loaded
    /// if need be and waited for (its error if it can't load), as the controller wants
    /// it; else only an instance already running, and none while the audio thread holds
    /// it (again next time): drawing a panel never loads a plug-in.
    fn build_view(&mut self, key: (deckard_doc::Chain, usize), focused: bool, now: std::time::Instant) -> Option<PluginView> {
        let pr = self.doc.as_ref()?.borrow().chain_slots(key.0)?.get(key.1)?.plugin.clone()?;
        // No pages saved: a hosted plug-in's default NKS preset's.
        let default_nks = if pr.pages.is_empty() && !pr.id.starts_with(deckard_plugins::native::VENDOR) {
            self.default_nks_preset(&pr.name)
        } else {
            None
        };
        let saved = match &default_nks {
            Some(nks) => nks
                .pages
                .iter()
                .map(|pg| deckard_model::PluginPage {
                    name: String::new(),
                    params: pg
                        .iter()
                        .map(|k| deckard_model::PluginKnob {
                            id: k.id.unwrap_or(u32::MAX),
                            name: k.name.clone(),
                            section: k.section.clone(),
                        })
                        .collect(),
                })
                .collect(),
            None => pr.pages.clone(),
        };
        // (The master's plug-ins take no side-chain.)
        let sidechain = |g: &deckard_plugins::Plugin| !matches!(key.0, deckard_doc::Chain::Master) && g.has_sidechain();
        let view = |pages, sidechain, instance, error| PluginView {
            key,
            pages,
            sidechain,
            page: usize::MAX,
            shown: 0,
            values: Vec::new(),
            refreshed: now,
            seen: now,
            plugin_id: pr.id.clone(),
            instance,
            error,
            sent: Vec::new(),
        };
        if !focused {
            let p = self.samples.plugins.existing(self.plugin_slot_key(key.0, key.1))?;
            let cid = deckard_plugins::state::PluginId::parse(&pr.id, &pr.name).map(|i| i.cid);
            let (pages, sc) = match p.try_lock() {
                // (Still the slot's old plug-in: once it's replaced.)
                Ok(g) if Some(g.id.cid) == cid => (view_pages(&g, &saved), sidechain(&g)),
                Ok(_) | Err(std::sync::TryLockError::WouldBlock) => return None,
                Err(_) => (Vec::new(), false),
            };
            return Some(view(pages, sc, std::sync::Arc::downgrade(&p), None));
        }
        match self.slot_plugin_in(key.0, key.1) {
            Ok(p) => {
                let (pages, sc) = p
                    .lock()
                    .map(|mut g| {
                        // A fresh instance with nothing to show takes the preset's sound.
                        if pr.chunk.is_empty()
                            && !g.has_automatable_params()
                            && let Some(nks) = &default_nks
                        {
                            let _ = g.load_chunk(&nks.chunk);
                            g.mark_saved(Vec::new());
                        }
                        (view_pages(&g, &saved), sidechain(&g))
                    })
                    .unwrap_or_default();
                Some(view(pages, sc, std::sync::Arc::downgrade(&p), None))
            }
            Err(e) => Some(view(Vec::new(), false, std::sync::Weak::new(), Some(e))),
        }
    }

    /// Load the project's state for slot `key` into its instance `p` if it differs from
    /// what the instance was loaded from or last saved (as [`deckard_plugins::Host::instance`]
    /// does, so it's no edit); not while the audio thread holds it (again next time).
    fn follow_plugin_chunk(&mut self, key: (deckard_doc::Chain, usize), p: &deckard_plugins::Shared) {
        let Some(doc) = self.doc.clone() else { return };
        let d = doc.borrow();
        let Some(pr) = d.chain_slots(key.0).and_then(|s| s.get(key.1)).and_then(|s| s.plugin.as_ref()) else { return };
        let Ok(mut g) = p.try_lock() else { return };
        if g.chunk() != pr.chunk.as_slice()
            && let Err(e) = g.load_chunk(&pr.chunk)
        {
            self.status = format!("{}: {e}", pr.name);
        }
    }

    /// Read view `key`'s shown page's values again (false: the plug-in was busy, or
    /// another is in the slot now, its view rebuilt).
    fn refresh_view(&mut self, key: (deckard_doc::Chain, usize), now: std::time::Instant) -> bool {
        // (The instance running, never loaded nor waited for here.)
        let Some(p) = self.samples.plugins.existing(self.plugin_slot_key(key.0, key.1)) else { return false };
        let Some(v) = self.plugin_views.get(&key) else { return false };
        // Another instance in the slot (an undo, a reload): its pages again, on the page
        // shown.
        if !std::ptr::eq(v.instance.as_ptr(), std::sync::Arc::as_ptr(&p)) {
            let shown = v.shown;
            self.plugin_views.remove(&key);
            if let Some(mut v) = self.build_view(key, self.plugin_focus == Some(key), now) {
                v.shown = shown.min(v.pages.len().saturating_sub(1));
                self.plugin_views.insert(key, v);
            }
            return false;
        }
        // The focused plug-in takes the project's state when it changed there (an undo):
        // the engine does that once it runs, and nothing else did before.
        if self.plugin_focus == Some(key) {
            self.follow_plugin_chunk(key, &p);
        }
        let v = self.plugin_views.get_mut(&key).unwrap();
        let page = v.shown;
        let ids: Vec<u32> = v.pages.get(page).map_or(Vec::new(), |pg| pg.knobs.iter().map(|(i, ..)| i.id).collect());
        // A VST3's values come from its controller without holding the instance
        // (the audio thread would wait); DECKARD's own read instantly under it.
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
        let Some((values, labels)) = values else { return false };
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
        true
    }

    /// Add a module or plug-in at the end of the focused sound's chain, and show it.
    /// The MIDI input plug-in on `group`, as the audio output has it.
    pub(crate) fn midi_input(&self, group: usize) -> Option<deckard_engine::midi_input::Info> {
        match &self.output {
            Some(Ok(out)) => out.midi_input(group),
            _ => None,
        }
    }

    /// Put the built-in MIDI input plug-in called `name` on `group`, or none.
    fn set_midi_input(&mut self, group: usize, name: Option<&str>) {
        let Ok(out) = self.output.get_or_insert_with(|| open_output(&self.settings, &mut self.output_fallback)) else {
            return;
        };
        out.set_midi_input(group, name.and_then(deckard_engine::midi_input::built_in));
    }

    /// Take the MIDI input plug-ins out: they go by group number, and aren't saved.
    fn clear_midi_inputs(&mut self) {
        let Some(Ok(out)) = &self.output else { return };
        for group in 0..deckard_engine::midi_input::SLOTS {
            if out.midi_input(group).is_some() {
                out.set_midi_input(group, None);
            }
        }
    }

    /// Make a routing choice from the channel properties.
    fn set_route(&mut self, r: layout::RouteSet) {
        let Some(doc) = self.doc.clone() else { return };
        let mut d = doc.borrow_mut();
        let done = match r {
            layout::RouteSet::Sound(g, s, route) => d.set_sound_route(g, s, route),
            layout::RouteSet::Group(g, aux, dest) => d.set_group_route(g, aux, dest),
            layout::RouteSet::GroupMidiInput(g, name) => {
                drop(d);
                self.set_midi_input(g, name);
                return;
            }
            layout::RouteSet::GroupMidiSource(g, port) => {
                d.set("MIDI Source", deckard_doc::Target::Group(g), "event_input", mxprj::decode::Value::Str(port))
            }
            layout::RouteSet::Master(cue, dest) => d.set_master_output(cue, dest),
            layout::RouteSet::SideChain(c, slot, src) => d.set_chain_sidechain(c, slot, src),
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
        // (The base key is the root note Generate writes.)
        let generation = field.starts_with("sequence_generation/") || field == "base_key";
        if !(variation || generation) || self.groove_setting(g, s, live).unwrap_or(0.0) == 0.0 {
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

    /// Put what a copy left in `clipboard_text` on the system clipboard. Every copy in
    /// DECKARD (sounds, groups, notes, macros) does this, and not for the text's sake:
    /// the window layer (egui-winit) turns ⌘V into a Paste event only when the system
    /// clipboard holds text, and drops the keystroke otherwise; so without this, ⌘V
    /// after ⌘C did nothing unless some unrelated text happened to be there.
    pub fn flush_clipboard(&mut self, ctx: &egui::Context) {
        if let Some(text) = self.clipboard_text.take() {
            ctx.copy_text(text);
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
            if ctx.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
                acts.push(Act::ResetSound(g, s));
            }
        }
        // ⌘V in either list pastes what was copied last: notes copied in the Pattern
        // Editor onto the sound clicked (the group's first, for a group clicked), at
        // their times; a sound onto the focused sound; a group over the group clicked.
        if matches!(self.key_focus, KeyFocus::Sounds | KeyFocus::Groups)
            && (cmd(egui::Key::V) || ev(|e| matches!(e, egui::Event::Paste(_))))
        {
            match self.last_copied {
                clipboard::Copied::Notes if self.note_clip.is_some() => acts.push(Act::PasteNotes(clipboard::PasteAt::Keys)),
                clipboard::Copied::Group if self.key_focus == KeyFocus::Groups && self.group_clipboard.is_some() => {
                    acts.push(Act::PasteGroup(g));
                }
                _ if !self.sound_clipboard.is_empty() => acts.push(Act::PasteSound(g, s)),
                _ => {}
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
        self.clipboard_text = Some(if notes.len() == 1 { "1 note".into() } else { format!("{} notes", notes.len()) });
        self.note_clip = Some(clipboard::NoteClip { notes: notes.clone(), sound: self.sound });
        self.last_copied = clipboard::Copied::Notes;
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
        let (g, focused, scene, song_view) = (self.group, self.pattern, self.scene, self.song_view);
        let (sound, keyboard, grid) = (self.sound, self.piano_roll, self.grid);
        let mut pasted = None;
        let r = doc.borrow_mut().transaction("Paste Notes", |d| {
            // The focused pattern; a group without one (clicked in the Group list)
            // gets "Pattern 1", playing in the focused scene, as drawing a note does.
            let n = d.project().groups.get(g).ok_or("no such group")?.patterns.len();
            let pi = if focused < n {
                focused
            } else if n == 0 {
                let pi = d.add_pattern(g, usize::MAX, "Pattern 1")?;
                if !song_view {
                    d.assign_pattern(scene, g, Some(pi))?;
                }
                pi
            } else {
                0
            };
            let p = d.project();
            let group = &p.groups[g];
            let pat = &group.patterns[pi];
            let add = clipboard::paste(&clip, at, sound, keyboard, pat.start, grid, group.sounds.len());
            if add.is_empty() {
                return Ok(());
            }
            let (start, len, bar) = (pat.start, layout::pattern_len(p, g, pi), p.bar_length());
            let end = add.iter().map(|n| n.position + n.length).max().unwrap_or(start) - start;
            d.edit_notes("Paste Notes", g, pi, &[], &add)?;
            if end > len {
                d.set_pattern_length(g, pi, (end + bar - 1) / bar * bar)?;
            }
            pasted = Some((pi, add));
            Ok(())
        });
        match (r, pasted) {
            (Ok(()), Some((pi, add))) => {
                self.pattern = pi;
                self.selected = add;
            }
            (Ok(()), None) => {}
            (Err(e), _) => self.status = format!("can't paste: {e}"),
        }
    }

    /// Whether the event area is in Draw mode.
    pub fn draw_mode(&self) -> bool {
        self.mouse == layout::MouseMode::Draw
    }

    /// A Variation or Generate setting of sound `s` in group `g`.
    /// The focused sound's notes in the focused pattern, while its Generate page
    /// (CHANNEL, Groove, page 3) shows in Random mode: by grid step, over the note range
    /// (widened to take in every note), with where playback is.
    fn random_view(&self, level: usize, page: usize, head: Option<Playhead>) -> Option<deckard_surface::RandomView> {
        if !self.channel || self.prop_set != 2 || level != 2 || page != 2 {
            return None;
        }
        let (g, s) = (self.group, self.sound);
        let f = |k: &str, d: f64| self.groove_setting(g, s, &format!("sequence_generation/{k}")).unwrap_or(d);
        if f("mode", 0.0) as i64 != 0 {
            return None;
        }
        let d = self.doc.as_ref()?.borrow();
        let p = d.project();
        let pat = p.groups.get(g)?.patterns.get(self.pattern)?;
        let grid = self.grid.max(1);
        let length = if pat.length > 0 { pat.length } else { p.bar_length() };
        let steps = ((length + grid - 1) / grid).max(1) as usize;
        let notes: Vec<(usize, i64, i64)> = pat
            .notes
            .iter()
            .filter(|n| n.sound == s && n.position >= pat.start)
            .map(|n| (((n.position - pat.start) / grid) as usize, n.pitch, n.velocity))
            .filter(|n| n.0 < steps)
            .collect();
        let (a, b) = (f("note_range/low", 60.0) as i64, f("note_range/high", 72.0) as i64);
        let (mut lo, mut hi) = (a.min(b).clamp(0, 127), a.max(b).clamp(0, 127));
        for n in &notes {
            (lo, hi) = (lo.min(n.1), hi.max(n.1));
        }
        let beat = (p.bar_length() / p.numerator.max(1) / grid).max(1) as usize;
        let position = self.pattern_at(g, head).map(|(at, _)| at as f64 / grid as f64);
        Some(deckard_surface::RandomView {
            steps,
            beat,
            notes,
            position,
            probability: f("probability", 55.0),
            velocity_amount: f("velocity_amount", 10.0),
            distribution: f("distribution", 1.0) as usize,
            range: (lo, hi),
        })
    }

    /// The focused sound's Euclidean rhythm, while its Generate page (CHANNEL, Groove,
    /// page 3) shows in Euclidean mode: the steps that get a note and where playback is
    /// in them, as Generate lays them from the pattern's start.
    fn euclid_view(&self, level: usize, page: usize, head: Option<Playhead>) -> Option<deckard_surface::EuclidView> {
        if !self.channel || self.prop_set != 2 || level != 2 || page != 2 {
            return None;
        }
        let (g, s) = (self.group, self.sound);
        let f = |k: &str, d: f64| self.groove_setting(g, s, &format!("sequence_generation/{k}")).unwrap_or(d);
        if f("mode", 0.0) as i64 != 1 {
            return None;
        }
        let steps = f("steps", 8.0).max(1.0) as usize;
        let (fills, rotate) = (f("fills", 5.0) as usize, f("rotate", 0.0) as usize);
        let bar = self.doc.as_ref()?.borrow().project().bar_length();
        let step = deckard_doc::division_ticks(f("division", 0.0) as i64, self.grid, bar, steps as i64).max(1);
        let position = self.pattern_at(g, head).map(|(at, _)| (at as f64 / step as f64).rem_euclid(steps as f64));
        Some(deckard_surface::EuclidView { hits: deckard_doc::euclid(steps, fills, rotate), fills: fills.min(steps), rotate, position })
    }

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
                self.plugin_views.clear();
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
        let c = self.chain();
        let removed_key = self.plugin_slot_key(c, i);
        // The removal and the addition undo as one step.
        doc.borrow_mut().begin_take("Change Module");
        if let Err(e) = doc.borrow_mut().remove_slot_in(c, i) {
            doc.borrow_mut().end_take();
            self.status = format!("can't replace it: {e}");
            return;
        }
        self.samples.plugins.retain(|s| *s != removed_key);
        self.add_slot_at(what, i);
        doc.borrow_mut().end_take();
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
        let label = match &module {
            deckard_doc::NewModule::Plugin { name, .. } if !name.is_empty() => "Load Plug-in",
            deckard_doc::NewModule::Effect(_) => "Load Effect",
            _ => "Load Module",
        };
        let added = doc.borrow_mut().transaction(label, |d| {
            d.add_slot_in(c, at, &module)?;
            if let deckard_doc::Chain::Sound(g, s) = c
                && at == 0
                && instrument
                && matches!(what, layout::AddWhat::Instrument(_) | layout::AddWhat::Plugin(_))
            {
                let name = d.project().groups[g].sounds[s].slots[0].name.clone();
                d.rename(deckard_doc::Target::Sound(g, s), &name)?;
            }
            Ok(())
        });
        match added {
            Ok(()) => {
                self.slot = Some(at);
                self.slot_of = Some(c);
                self.plugin_page = 0;
                self.plugin_views.clear();
            }
            Err(e) => self.status = format!("can't add it: {e}"),
        }
    }

    /// The plug-ins the project carries: each instrument, then the effects after it, then
    /// the groups' and the master's effects.
    pub(crate) fn wanted_plugins(&self) -> Vec<(deckard_plugins::Slot, deckard_model::PluginRef)> {
        #[cfg(test)]
        if let Some(w) = &self.boot_wanted {
            return w.clone();
        }
        let Some(doc) = self.doc.clone() else { return Vec::new() };
        let d = doc.borrow();
        let mut wanted: Vec<_> = d
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
        wanted
    }

    /// Load one plug-in at the output's `rate`.
    pub(crate) fn load_plugin(&mut self, slot: deckard_plugins::Slot, p: &deckard_model::PluginRef, rate: f64) -> Result<(), String> {
        self.samples.plugins.instance(slot, &p.id, &p.name, &p.chunk, rate).map(|_| ())
    }

    /// What the status line says once the plug-ins are loaded.
    pub(crate) fn plugins_loaded_status(n: usize, secs: f64, failed: &[String]) -> String {
        if failed.is_empty() { format!("{n} plug-ins loaded ({secs:.1} s)") } else { format!("plug-ins failed: {}", failed.join(", ")) }
    }

    /// Load every plug-in instrument of the project (at the output's rate), so playing
    /// and editing never wait for one.
    fn load_plugins(&mut self) {
        let wanted = self.wanted_plugins();
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
            if let Err(e) = self.load_plugin(*slot, p, rate) {
                failed.push(e);
            }
        }
        self.status = Self::plugins_loaded_status(wanted.len(), t.elapsed().as_secs_f64(), &failed);
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
            if let Err(e) = store_chunk(&mut doc.borrow_mut(), slot, chunk) {
                self.status = format!("plug-in state not stored: {e}");
            }
        }
    }

    /// Preferences ▸ Plug-ins ▸ Windows plug-ins (Linux): what's installed, and the
    /// buttons that install the rest. What a button starts is put in `action`.
    fn wine_section(&mut self, ui: &mut egui::Ui, action: &mut Option<WineAction>) {
        ui.separator();
        ui.strong("Windows plug-ins (through Wine and yabridge)");
        ui.label(
            egui::RichText::new(
                "NI's plug-ins have no Linux versions. Installed into Wine, yabridge makes them plug-ins DECKARD hosts like any other: \
                 install Wine, then yabridge, then your plug-ins into Wine (NI's through Native Access: wine <its installer>.exe), then Sync.",
            )
            .weak(),
        );
        let st = self.wine_status.get_or_insert_with(wine::Status::probe).clone();
        let busy = self.wine_task.is_some();
        egui::Grid::new("wine").num_columns(3).spacing([20.0, 8.0]).show(ui, |ui| {
            ui.label("Wine");
            match &st.wine {
                Some(v) => {
                    ui.label(v);
                    if st.yabridge_needs_build() {
                        ui.label(egui::RichText::new("newer than yabridge's 5.1.1 release: with it, plug-in windows take no clicks or typing. yabridge has to be built from its source (below).").weak())
                            .on_hover_text("yabridge issues #382 and #409. The fix is in yabridge's master branch, which DECKARD can build; docs/windows-plugins-troubleshooting.md tells the whole story.");
                    } else {
                        ui.label("");
                    }
                }
                None => {
                    ui.label("not installed");
                    match (st.family.wine_command(), st.pkexec) {
                        (Some(_), true) => {
                            if ui
                                .add_enabled(!busy, egui::Button::new("Install Wine…"))
                                .on_hover_text("Asks for an administrator's password")
                                .clicked()
                            {
                                *action = Some(WineAction::InstallWine(st.family));
                            }
                        }
                        (Some(cmd), false) => {
                            ui.label(format!("as root: {cmd}"));
                        }
                        (None, _) => {
                            ui.label("install it with your distribution's package manager");
                        }
                    }
                }
            }
            ui.end_row();
            ui.label("yabridge");
            // The button that gets a built yabridge: the build itself, or first the
            // tools it needs (as root), or what to install by hand.
            let build_button = |ui: &mut egui::Ui, action: &mut Option<WineAction>| {
                if st.build.ready() {
                    if ui
                        .add_enabled(!busy, egui::Button::new("Build yabridge"))
                        .on_hover_text("Builds yabridge's master branch from its source into ~/.local/share/yabridge; a few minutes. The release copy is kept as yabridge.release.")
                        .clicked()
                    {
                        *action = Some(WineAction::BuildYabridge);
                    }
                } else {
                    let missing = st.build.missing.join(" ");
                    match (&st.build.root_command, st.pkexec) {
                        (Some(cmd), true) => {
                            if ui
                                .add_enabled(!busy, egui::Button::new("Install build tools…"))
                                .on_hover_text(format!("Asks for an administrator's password and installs: {missing}"))
                                .clicked()
                            {
                                *action = Some(WineAction::InstallBuildTools(cmd.clone()));
                            }
                        }
                        (Some(cmd), false) => {
                            ui.label(format!("as root: {cmd}"));
                        }
                        (None, _) => {
                            ui.label(format!("to build it, install: {missing}"));
                        }
                    }
                }
            };
            match (&st.yabridgectl, st.yabridge_needs_build()) {
                (Some(p), false) => {
                    ui.label(match &st.yabridge_build {
                        Some(b) => format!("{} ({b})", p.display()),
                        None => p.display().to_string(),
                    });
                    ui.label("");
                }
                (Some(p), true) => {
                    ui.label(format!("{} (the 5.1.1 release)", p.display()));
                    build_button(ui, action);
                }
                (None, true) => {
                    ui.label("not installed");
                    build_button(ui, action);
                }
                (None, false) => {
                    ui.label("not installed");
                    if ui
                        .add_enabled(!busy, egui::Button::new("Install yabridge"))
                        .on_hover_text("Downloads yabridge 5.1.1 into ~/.local/share/yabridge")
                        .clicked()
                    {
                        *action = Some(WineAction::InstallYabridge);
                    }
                }
            }
            ui.end_row();
            ui.label("Plug-ins in Wine");
            ui.label(format!("{} in {}", st.windows_plugins, st.windows_vst3.display()));
            ui.label("");
            ui.end_row();
            ui.label("Bridged");
            ui.label(format!("{} in {}", st.bridged, wine::bridged_dir().display()));
            if ui.add_enabled(!busy && st.yabridgectl.is_some(), egui::Button::new("Sync now")).clicked() {
                *action = Some(WineAction::Sync);
            }
            ui.end_row();
        });
        ui.checkbox(&mut self.settings.bridge_windows, "Sync before each rescan");
        ui.horizontal(|ui| {
            if let Some((what, _)) = &self.wine_task {
                ui.spinner();
                ui.label(format!("{what}…"));
            } else if !self.wine_message.is_empty() {
                ui.label(egui::RichText::new(&self.wine_message).weak());
            }
        });
    }

    /// Run a Windows-plug-ins task in the background (one at a time).
    fn start_wine(&mut self, action: WineAction) {
        if self.wine_task.is_some() {
            return;
        }
        let (what, job): (&'static str, Box<dyn FnOnce() -> Result<String, String> + Send>) = match action {
            WineAction::InstallWine(f) => ("Installing Wine", Box::new(move || wine::install_wine(f))),
            WineAction::InstallYabridge => ("Installing yabridge", Box::new(wine::install_yabridge)),
            WineAction::InstallBuildTools(cmd) => ("Installing the build tools", Box::new(move || wine::install_build_tools(&cmd))),
            WineAction::BuildYabridge => ("Building yabridge", Box::new(wine::build_yabridge)),
            WineAction::Sync => ("Syncing Windows plug-ins", Box::new(wine::sync)),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("wine".into())
            .spawn(move || {
                let _ = tx.send(job());
            })
            .expect("a thread for the task");
        self.wine_task = Some((what, rx));
    }

    /// Take up a finished Windows-plug-ins task (call every frame). A sync that worked
    /// rescans, so the bridged plug-ins show up.
    fn poll_wine(&mut self) {
        let result = match &self.wine_task {
            Some((_, rx)) => match rx.try_recv() {
                Ok(r) => r,
                Err(_) => return,
            },
            None => return,
        };
        let what = self.wine_task.take().map_or("", |(w, _)| w);
        self.wine_message = match &result {
            Ok(m) => format!("{what}: {m}"),
            Err(e) => format!("{what}: {e}"),
        };
        self.wine_status = None;
        if what == "Syncing Windows plug-ins" && result.is_ok() {
            self.catalog.rescan(&self.settings);
        }
        // A new yabridge: its bundles have to be made again.
        if what == "Building yabridge" && result.is_ok() {
            self.start_wine(WineAction::Sync);
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
        self.watch_tunes(ctx);
        self.share_tempo();
        self.follow_engine(ctx, undrawn(ctx, BACKGROUND_PACE));
        self.sync_notes_played();
        if self.browser.poll() {
            ctx.request_repaint();
        }
        if self.browser.waiting() {
            ctx.request_repaint_after(undrawn(ctx, std::time::Duration::from_millis(30)));
        }
        let mut requests = Vec::new();
        if let Some(Ok(c)) = &mut self.controller {
            c.tick(&mut requests);
            ctx.request_repaint_after(undrawn(ctx, c.interval()));
        }
        self.mk3_sync(ctx, &mut requests, true);
        self.jam_sync(ctx, true);
        self.handle(requests);
        self.sync_selection();
        self.sync_audio_settings();
        self.update_sampling();
        self.sync_sampling_page();
        self.sync_browse_page();
        self.sync_arp();
        self.sync_auto_write();
        self.tick_mod_take(self.playhead().map(|h| (h.mode, h.position)), std::time::Instant::now());
        if self.browser.poll() || self.browser.waiting() {
            ctx.request_repaint_after(undrawn(ctx, std::time::Duration::from_millis(30)));
        }
        self.update_slot_view();
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

/// Store the state `chunk` of the plug-in in `slot` into `doc`.
pub(crate) fn store_chunk(doc: &mut deckard_doc::Document, slot: deckard_plugins::Slot, chunk: Vec<u8>) -> Result<(), String> {
    match slot {
        deckard_plugins::Slot::Sound { group, sound } => doc.set_plugin_chunk(group, sound, chunk),
        deckard_plugins::Slot::Chain { group, sound, slot } => doc.set_slot_plugin_chunk(group, sound, slot, chunk),
        deckard_plugins::Slot::Group { group, slot } => doc.set_chain_plugin_chunk(deckard_doc::Chain::Group(group), slot, chunk),
        deckard_plugins::Slot::Master { slot } => doc.set_chain_plugin_chunk(deckard_doc::Chain::Master, slot, chunk),
    }
}

/// The least time between opening an ASIO driver afresh at its request and the next.
const ASIO_REOPEN_GAP: std::time::Duration = std::time::Duration::from_secs(5);

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
        if let Some(j) = &self.jam {
            j.shut_down();
        }
        // The app leaves without running destructors: an ASIO driver is let go of here.
        if deckard_engine::devices::driver() == deckard_engine::devices::AudioDriver::Asio {
            self.close_output();
        }
    }

    /// While the window is covered or minimized eframe runs no `ui`, only this: keep the
    /// controller (scripts, LEDs, screens) and MIDI going, as `frame` does when shown.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.finish_quit(ctx) || self.boot.is_some() {
            return;
        }
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
    /// Once confirmed, closing must bypass periodic device/engine work. A dead
    /// audio callback can otherwise make each redundant parameter send wait.
    fn finish_quit(&mut self, ctx: &egui::Context) -> bool {
        if ctx.input(|i| i.viewport().close_requested()) && !self.asks_on_quit() {
            self.quitting = true;
        }
        if !self.quitting {
            return false;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        true
    }
    /// One frame of the whole window.
    fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = &ui.ctx().clone();
        if self.finish_quit(ctx) {
            return;
        }
        if self.boot.is_some() {
            self.boot_frame(ui);
            return;
        }
        self.poll_group_load();
        self.poll_item_load();
        if self.group_load.is_some() || self.item_load.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        // Before this frame's edits, so a finished Tune is the step before them.
        self.watch_tunes(ctx);
        self.share_tempo();
        // A selection belongs to the pattern it was made in.
        if self.selected_in != (self.group, self.pattern) {
            self.selected.clear();
            self.selected_in = (self.group, self.pattern);
        }
        self.follow_played_zone();
        self.update_waveform();
        self.update_sampling();
        self.poll_recorded();
        // An update was installed: quit (saving, if asked), and the helper restarts us.
        if self.update.poll() && self.update.restarting() {
            self.quit(ctx);
        }
        if self.recording {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        // A stem separation's progress, and its end.
        if self.stems.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        // The recorder takes up its audio every frame while armed, recording, or metering.
        if self.recorder.busy() || self.recorder.is_metering() {
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
        if self.finish_quit(ctx) {
            return;
        }
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
        // Plug-ins' timers, which this app's repainting would otherwise starve.
        deckard_plugins::editor::serve_timers();
        self.poll_wine();
        if self.wine_task.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if !self.editors.is_empty() {
            self.poll_plugin_editors();
            // On Linux the editors' timers run from these ticks: keep them coming.
            ctx.request_repaint_after(std::time::Duration::from_millis(if cfg!(target_os = "linux") { 16 } else { 250 }));
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
        self.jam_sync(ui.ctx(), false);
        self.handle(requests);
        self.sync_selection();
        self.sync_audio_settings();
        self.sync_browse_page();
        self.update_slot_view();
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
        self.update_slot_view();
        self.follow_edits();
        if !self.dropped_samples.is_empty() {
            let files = std::mem::take(&mut self.dropped_samples);
            let rows = ctx.data(|d| d.get_temp::<Vec<(egui::Rect, (usize, usize))>>(egui::Id::new(layout::DROP_ROWS))).unwrap_or_default();
            let at = self.file_drag.and_then(|p| rows.iter().find(|(r, _)| r.contains(p)).map(|&(_, gs)| gs));
            self.drop_samples(at.unwrap_or((self.group, self.sound)), &files);
            self.file_drag = None;
        }
        // A browser result dropped on a sound's row loads there.
        // (`take_payload` empties the payload whatever its type: only the right one.)
        if ctx.input(|i| i.pointer.any_released())
            && egui::DragAndDrop::has_payload_of_type::<layout::browser::Dragged>(ctx)
            && let Some(dragged) = egui::DragAndDrop::take_payload::<layout::browser::Dragged>(ctx)
            && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
        {
            let rows = ctx.data(|d| d.get_temp::<Vec<(egui::Rect, (usize, usize))>>(egui::Id::new(layout::DROP_ROWS))).unwrap_or_default();
            if let Some(&(_, gs)) = rows.iter().find(|(r, _)| r.contains(p)) {
                self.browse(browser::BrowseAct::Load(dragged.0, Some(gs)));
            }
        }
        // A recorded take dragged from RECORD's list onto a sound's row.
        // (`take_payload` empties the payload whatever its type: only the right one.)
        if ctx.input(|i| i.pointer.any_released())
            && egui::DragAndDrop::has_payload_of_type::<layout::sampling::DraggedTake>(ctx)
            && let Some(dragged) = egui::DragAndDrop::take_payload::<layout::sampling::DraggedTake>(ctx)
            && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
        {
            let rows = ctx.data(|d| d.get_temp::<Vec<(egui::Rect, (usize, usize))>>(egui::Id::new(layout::DROP_ROWS))).unwrap_or_default();
            if let Some(&(_, (g, s))) = rows.iter().find(|(r, _)| r.contains(p)) {
                self.place_take(g, s, &dragged.0);
            }
        }
        // (`take_payload` empties the payload whatever its type: only the right one.)
        if ctx.input(|i| i.pointer.any_released())
            && egui::DragAndDrop::has_payload_of_type::<layout::browser::DraggedFile>(ctx)
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
        self.flush_clipboard(ctx);
    }
}

fn preset_sound_name(name: &str) -> &str {
    for ext in [".nksf", ".nksfx", ".nfm8", ".nmsv", ".nrkt", ".rkplr", ".ens", ".mxinst"] {
        if let Some(stem) = name.strip_suffix(ext)
            && !stem.is_empty()
        {
            return stem;
        }
    }
    name
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

    /// Soloing a plug-in's extra-output strip mutes the other groups and their strips,
    /// and its owner (its main pair; the plug-in keeps playing for the strip, the engine's
    /// part), as one step; again, unsolo. Soloing the owner leaves its strips heard.
    #[test]
    fn soloing_a_strip_mutes_the_rest_but_not_its_plugin() {
        use deckard_model::AudioDest;
        use deckard_model::extra_out::{ExtraOut, Owner};
        let mut app = App { settings: settings::Settings::default(), ..App::default() };
        app.new_project();
        let doc = app.doc.clone().unwrap();
        while doc.borrow().project().groups.len() < 3 {
            doc.borrow_mut().add_group().unwrap();
        }
        // Real plug-ins in the strips' slots (undo drops strips whose plug-in isn't there).
        const FX: &str = "T:VST3:FX:p";
        const INSTR: &str = "T:VST3:INSTR:p";
        let plugin = |id: &str| deckard_doc::NewModule::Plugin { id: id.into(), name: "P".into(), chunk: Vec::new() };
        for c in [deckard_doc::Chain::Group(0), deckard_doc::Chain::Group(1), deckard_doc::Chain::Master] {
            doc.borrow_mut().add_slot_in(c, 0, &plugin(FX)).unwrap();
        }
        doc.borrow_mut().add_slot(2, 0, 0, &plugin(INSTR)).unwrap();
        let x = |owner: Owner, bus: usize| ExtraOut {
            owner,
            bus,
            plugin: (if matches!(owner, Owner::Sound(..)) { INSTR } else { FX }).into(),
            name: "Bus".into(),
            level: 1.0,
            pan: 0.0,
            muted: false,
            dest: AudioDest::Ext(1),
            channels: Default::default(),
        };
        doc.borrow_mut()
            .set_extra_outs(vec![x(Owner::Group(0, 0), 1), x(Owner::Group(0, 0), 2), x(Owner::Group(1, 0), 1), x(Owner::Sound(2, 0), 1)])
            .unwrap();
        let state = |app: &App| {
            let d = app.doc.as_ref().unwrap().borrow();
            let p = d.project();
            (p.groups.iter().map(|g| g.muted).collect::<Vec<_>>(), p.extra_outs.iter().map(|x| x.muted).collect::<Vec<_>>())
        };
        let sound_20 = |app: &App| app.doc.as_ref().unwrap().borrow().project().groups[2].sounds[0].muted;
        app.solo(layout::Channel::Extra(0));
        assert_eq!(
            state(&app),
            (vec![true, true, true], vec![false, true, true, true]),
            "the owner's main pair and every other strip, a sound's in another group too"
        );
        doc.borrow_mut().undo();
        assert_eq!(state(&app), (vec![false; 3], vec![false; 4]), "one step");
        app.solo(layout::Channel::Extra(0));
        app.solo(layout::Channel::Extra(0));
        assert_eq!(state(&app), (vec![false; 3], vec![false; 4]), "again: unsolo");
        // A group soloed: the other groups' strips go quiet, its sounds' strips in other
        // groups too (they play whatever their group's mute); its own stay heard.
        app.solo(layout::Channel::Group(0));
        assert_eq!(state(&app), (vec![false, true, true], vec![false, false, true, true]), "the owner's strips stay heard");
        app.solo(layout::Channel::Group(0));
        app.solo(layout::Channel::Group(2));
        assert_eq!(state(&app), (vec![true, true, false], vec![true, true, true, false]), "its sounds' strips stay heard");
        app.solo(layout::Channel::Group(2));
        // A sound's strip soloed: the other groups and that group's sounds (its own too)
        // muted, the group it plays into heard.
        app.solo(layout::Channel::Extra(3));
        assert_eq!(state(&app), (vec![true, true, false], vec![true, true, true, false]));
        assert!(sound_20(&app), "its sound's main pair");
        app.solo(layout::Channel::Extra(3));
        assert_eq!(state(&app), (vec![false; 3], vec![false; 4]));
        assert!(!sound_20(&app));
        // A master's plug-in's strip: its plug-in hears the mix, so no group is muted;
        // only the other strips are.
        let mut outs = doc.borrow().project().extra_outs.clone();
        outs.push(x(Owner::Master(0), 1));
        doc.borrow_mut().set_extra_outs(outs).unwrap();
        app.solo(layout::Channel::Extra(4));
        assert_eq!(state(&app), (vec![false; 3], vec![true, true, true, true, false]), "the groups feed the master");
        app.solo(layout::Channel::Extra(4));
        assert_eq!(state(&app), (vec![false; 3], vec![false; 5]), "unsolo");
    }

    /// The extra outputs' meters, peaks and headroom are held by place in the list: a
    /// strip added at the end keeps the others', one taken away starts them all again.
    #[test]
    fn extra_meters_start_again_when_the_strips_change() {
        use deckard_model::AudioDest;
        use deckard_model::extra_out::{ExtraOut, Owner};
        let mut app = App { settings: settings::Settings::default(), ..App::default() };
        app.new_project();
        let doc = app.doc.clone().unwrap();
        let x = |bus| ExtraOut {
            owner: Owner::Group(0, 0),
            bus,
            plugin: "p".into(),
            name: String::new(),
            level: 1.0,
            pan: 0.0,
            muted: false,
            dest: AudioDest::Ext(1),
            channels: Default::default(),
        };
        doc.borrow_mut().set_extra_outs(vec![x(1), x(2)]).unwrap();
        app.sync_extra_meters();
        let now = std::time::Instant::now();
        app.extra_meters = vec![(0.5, 0.5), (0.25, 0.25)];
        for i in 0..2 {
            app.peaks.insert(layout::Channel::Extra(i), [(0.5, now); 2]);
            app.headroom.insert(layout::Channel::Extra(i), 0.5);
        }
        app.headroom.insert(layout::Channel::Master, 0.7);
        doc.borrow_mut().set_extra_outs(vec![x(1), x(2), x(3)]).unwrap();
        app.sync_extra_meters();
        assert_eq!(app.extra_meters, vec![(0.5, 0.5), (0.25, 0.25), (0.0, 0.0)], "added at the end");
        assert_eq!(app.headroom.len(), 3);
        doc.borrow_mut().set_extra_outs(vec![x(2), x(3)]).unwrap();
        app.sync_extra_meters();
        assert_eq!(app.extra_meters, vec![(0.0, 0.0); 2]);
        assert!(!app.peaks.keys().any(|c| matches!(c, layout::Channel::Extra(_))));
        assert_eq!(app.headroom.keys().collect::<Vec<_>>(), vec![&layout::Channel::Master], "the channels' are kept");
    }

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
        let Some(mut h) = scripted_harness(false) else { return };
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
        while head.as_ref().is_none_or(|p| p.queued.is_none()) && wait.elapsed() < AUDIO_PATIENCE {
            std::thread::sleep(std::time::Duration::from_millis(5));
            head = h.state().playhead();
        }
        let head = head.expect("playing");
        assert_eq!((head.mode, head.queued), (Mode::Scene(0), Some(Mode::Scene(3))));
        h.run_steps(10);
        assert_eq!(h.state().scene, 3, "the selection sticks while the old scene finishes");
        h.state_mut().stop();
    }

    /// A plug-in chosen with nothing playing reaches the engine played next, its parameter
    /// moved on the way, and a new project takes it out.
    #[test]
    fn a_midi_plug_in_chosen_while_stopped_plays_and_goes_with_the_project() {
        let mut app = App::default();
        app.new_project();
        app.set_route(layout::RouteSet::GroupMidiInput(0, Some(deckard_engine::note_echo::NAME)));
        let Some(Ok(out)) = &app.output else {
            eprintln!("no audio output; skipping");
            return;
        };
        let info = out.midi_input(0).expect("on the group");
        assert_eq!(info.name, deckard_engine::note_echo::NAME);
        assert_eq!(out.set_midi_input_param(0, 2, 99.0), Some(8.0), "fitted to its range");
        assert_eq!(out.midi_input(0).unwrap().params[1].value, 8.0);
        assert_eq!(out.set_midi_input_param(0, 99, 1.0), None);

        app.transport_play();
        let Some(Ok(out)) = &app.output else { unreachable!() };
        assert_eq!(out.call(|e| e.has_midi_input(0)), Some(true));

        app.new_project();
        let Some(Ok(out)) = &app.output else { unreachable!() };
        assert!(out.midi_input(0).is_none());
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
        while tempo != 171.0 && wait.elapsed() < AUDIO_PATIENCE {
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
        let at = |app: &mut App, within: std::ops::Range<f64>| {
            published(app);
            let wait = std::time::Instant::now();
            let mut pos = 0.0;
            while !within.contains(&pos) && wait.elapsed() < AUDIO_PATIENCE {
                app.background(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(10));
                pos = app.playhead().map_or(0.0, |h| h.position);
            }
            pos
        };
        let quarter = deckard_model::PPQ as f64;
        // Early in the bar: going on from near its end could come round the loop to the
        // start before the position is looked at.
        assert!(at(&mut app, quarter + 1.0..2.0 * quarter) > quarter, "playing");
        app.stop();
        let parked = app.shown_playhead().expect("shown where it stopped").position;
        assert!(parked > quarter, "{parked}");
        // The MK3 (the touch strip's play position) shows it there too.
        assert_eq!(app.surface_view().position, Some(parked as i64));
        app.transport_play();
        let first = at(&mut app, f64::MIN_POSITIVE..f64::MAX);
        assert!(first >= parked - 1.0, "on from {parked}: {first}");
        app.stop();
        app.restart();
        let first = at(&mut app, f64::MIN_POSITIVE..f64::MAX);
        app.stop();
        assert!(first < parked, "RESTART: from the start ({first})");
    }

    /// SHIFT + REC counts in from the start (of the scene, or the focused section), not
    /// from where STOP left the song.
    #[test]
    fn a_count_in_starts_from_the_beginning() {
        let mut app = App::default();
        app.new_project();
        app.transport_play();
        if !matches!(app.output, Some(Ok(_))) {
            eprintln!("no audio output; skipping");
            return;
        }
        let ctx = egui::Context::default();
        let quarter = deckard_model::PPQ as f64;
        let wait = std::time::Instant::now();
        while app.playhead().map_or(0.0, |h| h.position) <= quarter && wait.elapsed() < AUDIO_PATIENCE {
            app.background(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.stop();
        let parked = app.shown_playhead().expect("shown where it stopped").position;
        assert!(parked > quarter, "{parked}");
        app.set_recording(true, true);
        app.background(&ctx);
        published(&app);
        let first = app.playhead().map_or(0.0, |h| h.position);
        app.set_recording(false, false);
        app.stop();
        assert!(first < quarter, "counted in from the start, not {parked}: {first}");
    }

    #[test]
    fn finishing_a_grown_take_starts_at_the_first_occupied_bar() {
        let mut app = App::default();
        app.new_project();
        app.settings.pattern_grow = true;
        app.set_recording(true, false);
        let doc = app.doc.clone().unwrap();
        let bar = doc.borrow().project().bar_length();
        let first = 2 * bar + bar / 4;
        {
            let mut d = doc.borrow_mut();
            d.set_pattern_length(0, 0, 6 * bar).unwrap();
            d.edit_notes(
                "Record",
                0,
                0,
                &[],
                &[
                    deckard_model::Note {
                        position: first,
                        length: bar / 2,
                        sound: 0,
                        pitch: 60,
                        velocity: 100,
                        probability: 100,
                        expr: None,
                    },
                    deckard_model::Note {
                        position: 4 * bar,
                        length: bar,
                        sound: 1,
                        pitch: 62,
                        velocity: 100,
                        probability: 100,
                        expr: None,
                    },
                ],
            )
            .unwrap();
        }
        app.set_recording(false, false);
        let d = doc.borrow();
        let pat = &d.project().groups[0].patterns[0];
        assert_eq!((pat.start, pat.length), (2 * bar, 3 * bar));
        assert_eq!(pat.notes[0].position, first, "events keep their timing; playback span moves");
    }

    #[test]
    fn first_fixed_length_recording_skips_empty_bars_and_overdubs_keep_them() {
        for overdub in [false, true] {
            let mut app = App::default();
            app.new_project();
            app.settings.pattern_grow = false;
            let doc = app.doc.clone().unwrap();
            {
                let mut d = doc.borrow_mut();
                d.add_pattern(0, 0, "Pattern 1").unwrap();
                d.assign_pattern(0, 0, Some(0)).unwrap();
            }
            let bar = doc.borrow().project().bar_length();
            let note = deckard_model::Note {
                position: bar + bar / 4,
                length: bar / 4,
                sound: 0,
                pitch: 60,
                velocity: 100,
                probability: 100,
                expr: None,
            };
            {
                let mut d = doc.borrow_mut();
                d.set_pattern_length(0, 0, 4 * bar).unwrap();
                d.set_length_auto(0, 0, false);
                if overdub {
                    d.edit_notes("seed", 0, 0, &[], &[note]).unwrap();
                }
            }
            app.set_recording(true, false);
            if !overdub {
                app.take_in_recorded(vec![deckard_engine::Recorded {
                    group: 0,
                    sound: 0,
                    pitch: 60,
                    velocity: 100,
                    length: bar / 4,
                    tick: (bar + bar / 4) as f64,
                    played: 0.,
                    mode: deckard_engine::Mode::Scene(0),
                    midi: false,
                    seq: 0,
                }]);
            }
            app.set_recording(false, false);
            let d = doc.borrow();
            let pat = &d.project().groups[0].patterns[0];
            assert_eq!(pat.start, if overdub { 0 } else { bar });
            assert_eq!(pat.length, if overdub { 4 * bar } else { 3 * bar });
        }
    }

    /// Played (with audio): a new Auto-length pattern after a 4-bar one, recorded into
    /// bar after bar, grows for as long as the take goes on: it isn't caught by the song
    /// coming round before the new bar reaches the engine.
    #[test]
    fn a_new_pattern_grows_past_a_long_one_while_playing() {
        let mut app = App::default();
        app.new_project();
        let doc = app.doc.clone().unwrap();
        let bar = doc.borrow().project().bar_length();
        {
            let mut d = doc.borrow_mut();
            d.set_tempo(240.0).unwrap();
            d.add_pattern(0, 0, "Pattern 1").unwrap();
            d.assign_pattern(0, 0, Some(0)).unwrap();
            d.set_pattern_length(0, 0, 4 * bar).unwrap();
            d.set_length_auto(0, 0, false);
            let n = deckard_model::Note { expr: None, sound: 1, position: 0, length: 100, pitch: 60, velocity: 100, probability: 100 };
            d.edit_notes("t", 0, 0, &[], &[n]).unwrap();
        }
        (app.group, app.pattern, app.scene, app.sound) = (0, 0, 0, 0);
        layout::apply_acts(&mut app, vec![layout::Act::NewPattern(0, 0)]);
        app.set_recording(true, false);
        app.transport_play();
        if !matches!(app.output, Some(Ok(_))) {
            eprintln!("no audio output; skipping");
            return;
        }
        let ctx = egui::Context::default();
        let len = |app: &App| app.doc.as_ref().unwrap().borrow().project().groups[0].patterns[1].length;
        // A note every beat (240 BPM: a bar is a second) for 7 bars' time. The pattern
        // grows a bar ahead when a poll sees the song in its last bar, so the polls (one a
        // beat here; one a frame in the app) have to come round within that bar; the
        // longest gap between them is measured, for a CI runner that oversleeps.
        let beat = std::time::Duration::from_millis(250);
        let wait = std::time::Instant::now();
        let (mut polled, mut gap) = (std::time::Instant::now(), std::time::Duration::ZERO);
        while wait.elapsed() < beat * 4 * 7 {
            if let Some(Ok(out)) = &app.output {
                out.live(deckard_engine::Live::NoteIn { group: 0, sound: 0, pitch: 60, velocity: 100, thru: false });
            }
            std::thread::sleep(beat / 2);
            if let Some(Ok(out)) = &app.output {
                out.live(deckard_engine::Live::NoteRelease { group: 0, sound: 0, pitch: 60 });
            }
            std::thread::sleep(beat / 2);
            app.background(&ctx);
            app.poll_recorded();
            gap = gap.max(polled.elapsed());
            polled = std::time::Instant::now();
        }
        app.set_recording(false, false);
        app.stop();
        if gap > beat * 2 {
            eprintln!("this machine oversleeps ({gap:?} between polls, for a bar of {:?}); skipping", beat * 4);
            return;
        }
        // About 7 bars played (less what starting takes): kept, past the other's 4.
        assert!(len(&app) >= 6 * bar, "{} bars ({}; longest gap between polls {gap:?})", len(&app) as f64 / bar as f64, app.status);
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

    /// Two effects on sound 1, the second focused on its second page, and the Groove
    /// set on its last page; sound 2 left on the Groove set's second page.
    fn chains_left_somewhere() -> App {
        let mut app = App::default();
        app.new_project();
        {
            let doc = app.doc.clone().unwrap();
            let mut d = doc.borrow_mut();
            d.add_slot(0, 0, 0, &deckard_doc::NewModule::Effect("Compressor")).unwrap();
            d.add_slot(0, 0, 1, &deckard_doc::NewModule::Effect("Compressor")).unwrap();
        }
        app.update_slot_view();
        layout::apply_acts(&mut app, vec![layout::Act::SelectSlot(Some(1)), layout::Act::PluginPage(1)]);
        app.update_slot_view();
        layout::apply_acts(&mut app, vec![layout::Act::PropSet(1), layout::Act::PropPage(2)]);
        app.update_slot_view();
        app.sound = 1;
        app.update_slot_view();
        layout::apply_acts(&mut app, vec![layout::Act::PropPage(1)]);
        app.update_slot_view();
        app
    }

    /// Each chain shows where it was left: its slot and that slot's page, and each
    /// property set's page, whatever was shown in between.
    #[test]
    fn each_chain_comes_back_where_it_was_left() {
        let mut app = chains_left_somewhere();
        assert_eq!((app.slot, app.prop_page), (None, 1));
        (app.group, app.sound) = (1, 0);
        app.update_slot_view();
        assert_eq!(app.prop_page, 0);
        (app.group, app.sound) = (0, 0);
        app.update_slot_view();
        assert_eq!((app.slot, app.plugin_page, app.prop_set, app.prop_page), (Some(1), 1, 1, 2));
        // Another set and back: still on its page.
        layout::apply_acts(&mut app, vec![layout::Act::PropSet(0)]);
        app.update_slot_view();
        assert_eq!(app.prop_page, 0);
        layout::apply_acts(&mut app, vec![layout::Act::PropSet(1)]);
        assert_eq!(app.prop_page, 2);
        // The other slot and back: still on its page.
        layout::apply_acts(&mut app, vec![layout::Act::SelectSlot(Some(0))]);
        app.update_slot_view();
        assert_eq!(app.plugin_page, 0);
        layout::apply_acts(&mut app, vec![layout::Act::SelectSlot(Some(1))]);
        app.update_slot_view();
        assert_eq!(app.plugin_page, 1);
        app.sound = 1;
        app.update_slot_view();
        assert_eq!(app.prop_page, 1);
    }

    /// A saved project reopens on the app's view (Ideas or Song, the Mixer, the chain
    /// tab) and with each chain where it was left.
    #[test]
    fn a_project_reopens_on_its_view_and_where_each_chain_was_left() {
        let mut app = chains_left_somewhere();
        (app.song_view, app.mixer, app.chain_tab) = (true, true, layout::ChainTab::Sound);
        app.store_screen();
        let bytes = app.doc.as_ref().unwrap().borrow().to_bytes().unwrap();
        let mut again = App::default();
        again.show_document(Document::from_bytes(bytes, None));
        again.update_slot_view();
        assert_eq!((again.song_view, again.mixer, again.sound, again.prop_set, again.prop_page), (true, true, 1, 1, 1));
        again.sound = 0;
        again.update_slot_view();
        assert_eq!((again.slot, again.plugin_page, again.prop_page), (Some(1), 1, 2));
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
        while app.scene != 1 && wait.elapsed() < AUDIO_PATIENCE {
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
        while app.master_meter == (0.0, 0.0) && wait.elapsed() < AUDIO_PATIENCE {
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
        while app.master_meter == (0.0, 0.0) && wait.elapsed() < AUDIO_PATIENCE {
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
        let Some(mut h) = scripted_harness(true) else { return };
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
        let Some(mut h) = scripted_harness(false) else { return };
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
        let Some(mut h) = scripted_harness(false) else { return };
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
        let Some(mut h) = scripted_harness(false) else { return };
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
        let Some(mut h) = scripted_harness(false) else { return };
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
        let Some(mut h) = scripted_harness(true) else { return };
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
        let Some(mut h) = scripted_harness(false) else { return };
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
        let Some(mut h) = scripted_harness(true) else { return };
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
        let Some(mut h) = scripted_harness(true) else { return };
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
        let Some(mut h) = scripted_harness(false) else { return };
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
        let Some(mut h) = scripted_harness(true) else { return };
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
        let Some(mut h) = scripted_harness(false) else { return };
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
    /// A double-click on a knob types its value, as it's shown; ⌘-click sets its default.
    #[test]
    fn poly_synth_knob_takes_a_typed_value() {
        let Some(mut h) = harness(false, false) else { return };
        layout::style(&h.ctx);
        let Some(sound) = (0..16).find(|&s| project_of(&h).groups[0].sounds.get(s).is_some_and(|x| x.slots.is_empty())) else { return };
        h.state_mut().sound = sound;
        h.run_steps(2);
        h.state_mut().add_slot(layout::AddWhat::Instrument("PolySynth"));
        h.state_mut().mixer = true;
        h.run_steps(5);
        let panels: Vec<(usize, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-panels"))).unwrap();
        let r = panels.iter().find(|p| p.0 == 0).unwrap().1;
        let body = egui::Rect::from_min_max(Pos2::new(r.left() + 4.0, r.top() + 26.0), Pos2::new(r.right() - 4.0, r.bottom()));
        let k = body.height() / 216.0;
        let tabs = h.ctx.data(|d| d.get_temp::<Vec<egui::Rect>>(egui::Id::new("deckard-poly-tabs"))).unwrap();
        click(&mut h, tabs[1].center());
        h.run_steps(3);
        let c = body.min + egui::vec2(506.0 + 20.5, 76.0 + 20.5) * k;
        let cutoff = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[sound].slots[0].param("filter_cutoff").unwrap();
        let double = |h: &mut Harness<'static, App>, modifiers: egui::Modifiers, times: usize| {
            // (Long enough after the last click not to count with it.)
            h.run_steps(40);
            h.hover_at(c);
            h.step();
            for _ in 0..times {
                for pressed in [true, false] {
                    h.event(Event::PointerButton { pos: c, button: PointerButton::Primary, pressed, modifiers });
                    h.step();
                }
            }
            h.run_steps(2);
        };
        double(&mut h, Default::default(), 2);
        assert_eq!(cutoff(&h), 1.0, "opening the box changes nothing");
        // Esc leaves it.
        h.event(Event::Text("10".into()));
        h.step();
        h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        h.run_steps(3);
        assert_eq!(cutoff(&h), 1.0, "Esc cancels");
        // Enter keeps it, read as the knob shows it (a percentage).
        double(&mut h, Default::default(), 2);
        h.event(Event::Text("25".into()));
        h.step();
        h.event(Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        h.run_steps(3);
        assert!((cutoff(&h) - 0.25).abs() < 0.002, "typed 25 %: {}", cutoff(&h));
        // ⌘-click: its default.
        h.event(Event::ModifiersChanged(egui::Modifiers::COMMAND));
        double(&mut h, egui::Modifiers::COMMAND, 1);
        h.event(Event::ModifiersChanged(Default::default()));
        assert_eq!(cutoff(&h), 1.0, "its default");
    }

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
        let start = p.groups[g].patterns.get(pi).map_or(0, |pt| pt.start);
        let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
        let (from, scale): (f64, f32) = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid-time"))).expect("time axis drawn");
        Pos2::new(grid.left() + ((start + t) as f64 - from) as f32 * scale, grid.top() + row as f32 * 24.0 + 12.0)
    }

    /// SCALE (next to QUANTIZE) sets the controller's scale, and the piano roll marks
    /// the keys outside it.
    #[test]
    fn scale_menu_sets_the_scale_and_marks_the_piano_roll() {
        let Some(mut h) = scripted_harness(false) else { return };
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
        let view = h.state().plugin_view().expect("pages shown");
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
        let view = h.state().plugin_view().unwrap();
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

    /// The Ideas view shows a group's patterns in their places, with the gaps between, and
    /// a double-click on a gap or below the last makes a pattern there.
    #[test]
    fn the_ideas_view_shows_and_fills_gaps() {
        let mut app = App { show_controller: false, follow: true, grid: PPQ / 4, prop_set: 1, ..Default::default() };
        app.new_project();
        {
            let mut d = app.doc.as_ref().unwrap().borrow_mut();
            d.add_pattern(0, 0, "One").unwrap();
            d.add_pattern_in_slot(0, 0, 3, "Four").unwrap();
        }
        (app.group, app.pattern, app.scene, app.sound, app.song_view) = (0, 0, 0, 0, false);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1500.0, 930.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
        h.run_steps(4);
        let place = |h: &Harness<'static, App>, slot: usize| -> egui::Rect {
            h.ctx.data(|d| d.get_temp(layout::header_rect(&format!("pattern place 0 {slot}")))).expect("drawn")
        };
        let slots = |h: &Harness<'static, App>| project_of(h).groups[0].patterns.iter().map(|p| p.slot).collect::<Vec<_>>();
        let (first, fourth) = (place(&h, 0), place(&h, 3));
        assert!((fourth.top() - first.top() - 3.0 * first.height()).abs() < 0.5, "its own row: {first:?} {fourth:?}");
        // Clicking the second of the list, in the fourth place, picks it.
        click(&mut h, fourth.center());
        assert_eq!(h.state().pattern, 1);
        // A gap.
        let gap = place(&h, 1).center();
        double_click(&mut h, gap);
        assert_eq!(slots(&h), [0, 1, 3], "{}", h.state().status);
        assert_eq!(project_of(&h).groups[0].patterns[2].name, "Four", "the rest stay");
        // Below the last, leaving a gap.
        let below = place(&h, 6).center();
        double_click(&mut h, below);
        assert_eq!(slots(&h), [0, 1, 3, 6]);
    }

    /// Auto-write on while the song plays: dragging a mixer strip's fader or pan writes
    /// modulation and leaves them where they are. Otherwise it moves them.
    #[test]
    fn dragging_a_mixer_fader_writes_with_auto_write_on() {
        let mut app = App { show_controller: false, follow: true, grid: PPQ / 4, prop_set: 1, ..Default::default() };
        app.new_project();
        {
            let mut d = app.doc.as_ref().unwrap().borrow_mut();
            d.add_pattern(0, 0, "Pattern 1").unwrap();
            d.assign_pattern(0, 0, Some(0)).unwrap();
        }
        (app.group, app.pattern, app.scene, app.sound) = (0, 0, 0, 0);
        (app.mixer, app.mixer_sounds) = (true, false);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1500.0, 930.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
        h.run_steps(4);
        let strips: Vec<(layout::Channel, egui::Rect)> =
            h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-mixer-strips"))).expect("mixer drawn");
        let r = strips.iter().find(|(c, _)| *c == layout::Channel::Group(0)).unwrap().1;
        let x = r.left() + 48.0;
        let zero_db = r.bottom() - 186.0 + 26.0 + 25.0;
        let pan = egui::pos2(r.left() + 5.0 + 30.5, r.bottom() - 186.0 + 8.0);
        let level = |h: &Harness<'static, App>| project_of(h).groups[0].level;
        let pan_now = |h: &Harness<'static, App>| project_of(h).groups[0].pan;
        // Stopped: it moves.
        let before = level(&h);
        drag(&mut h, egui::pos2(x, zero_db), egui::pos2(x, zero_db + 22.0));
        assert!(level(&h) < before * 0.8, "about -10 dB: {}", level(&h));
        // Playing with Auto-write on: written.
        let before = level(&h);
        h.state_mut().auto_write = true;
        h.state_mut().transport_play();
        if !matches!(h.state().output, Some(Ok(_))) {
            eprintln!("no audio output; skipping");
            return;
        }
        let wait = std::time::Instant::now();
        while h.state().playhead().is_none() && wait.elapsed() < std::time::Duration::from_secs(60) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let at = zero_db + 22.0;
        drag(&mut h, egui::pos2(x, at), egui::pos2(x, at + 15.0));
        drag(&mut h, pan, pan + egui::vec2(10.0, 0.0));
        h.state_mut().stop();
        h.run_steps(2);
        let p = project_of(&h);
        assert_eq!(level(&h), before, "the fader stays");
        assert_eq!(pan_now(&h), 0.0, "and the pan");
        let points =
            |field: &str| p.groups[0].patterns[0].mods.iter().filter(|m| m.sound.is_none() && m.slot.is_none() && m.field == field).count();
        assert!(points("level") > 0, "the level written");
        assert!(points("pan") > 0, "the pan written");
        assert!(p.groups[0].patterns[0].mods.iter().filter(|m| m.field == "level").all(|m| m.value < 0.0), "down from the fader's place");
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
        h.state_mut().finish_recording(recorder::Finished::Take {
            sample: tone(1.0),
            channels: 2,
            mode: 2,
            ticks: 2.0 * bar,
            focus: (0, 0),
        });
        let q = project_of(&h);
        assert_eq!(q.groups[0].patterns.len(), n + 1, "{}", h.state().status);
        let pat = &q.groups[0].patterns[n];
        // An Audio plug-in playing the take (its length the recording's, in beats), on in
        // the new pattern without notes.
        assert!(pat.notes.iter().all(|x| x.sound != s1));
        assert_eq!(pat.audio, vec![deckard_model::AudioCue { sound: s1, take: 0, offset: 0 }]);
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
        h.state_mut().finish_recording(recorder::Finished::Take {
            sample: tone(0.5),
            channels: 1,
            mode: 2,
            ticks: bar / 2.0,
            focus: (0, s1),
        });
        let q = project_of(&h);
        assert!(q.groups[0].patterns[n].audio.iter().any(|c| c.sound == s2));
        assert!(q.groups[0].sounds[s2].audio.is_some());
        // Loop, Target Take, on that sound: a second take, playing in the pattern.
        (h.state_mut().sound, h.state_mut().pattern) = (s2, n);
        h.state_mut().recorder.view.set(RecParam::Target, 0);
        h.state_mut().finish_recording(recorder::Finished::Take {
            sample: tone(0.5),
            channels: 1,
            mode: 2,
            ticks: bar / 2.0,
            focus: (0, s2),
        });
        let q = project_of(&h);
        assert_eq!(q.groups[0].sounds[s2].audio.as_ref().map(|a| (a.takes.len(), a.focused)), Some((2, 1)));
        assert!(q.groups[0].patterns[n].audio.contains(&deckard_model::AudioCue { sound: s2, take: 1, offset: 0 }));

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
        h.state_mut().finish_recording(recorder::Finished::Auto { done, focus: (g, 0) });
        let q = project_of(&h);
        let zones = &q.groups[g].sounds[s3].sampler.as_ref().expect("a Sampler in the free sound").zones;
        let got: Vec<(i64, i64, i64)> = zones.iter().map(|z| (z.root_key, z.key_range.0, z.key_range.1)).collect();
        assert_eq!(got, vec![(48, 0, 53), (60, 54, 127)]);
        assert!(zones.iter().all(|z| z.loop_active && z.loop_range == (4800, 20_000)));
        assert_eq!((h.state().sound, h.state().sampling.tab), (s3, 3));
    }

    /// Finish a Loop/Take recording on sound `s` of group 0: it becomes an Audio plug-in.
    fn audio_sound(h: &mut Harness<'static, App>, s: usize, tone: &deckard_engine::sample::Sample, bar: f64) {
        use layout::sampling::RecParam;
        (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound) = (0, 0, s);
        h.state_mut().recorder.view.set(RecParam::Target, 0);
        h.state_mut().finish_recording(recorder::Finished::Take { sample: tone.clone(), channels: 2, mode: 2, ticks: bar, focus: (0, s) });
    }

    fn tone_of(secs: f64) -> deckard_engine::sample::Sample {
        deckard_engine::sample::Sample { rate: 48_000, frames: vec![[0.25, 0.25]; (secs * 48_000.0) as usize] }
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

    /// A free sound of group 0 made an Audio plug-in with a bar-long take, on in the
    /// pattern in focus (no notes of its own there), focused: (its sound, the project's
    /// folder, kept while the test runs).
    fn loop_sound(h: &mut Harness<'static, App>) -> (usize, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        h.state_mut().path = Some(tmp.path().join("x.mxprj"));
        a_pattern(h);
        let p = project_of(h);
        let bar = p.bar_length() as f64;
        let free = |s: &usize| {
            let snd = &p.groups[0].sounds[*s];
            snd.slots.is_empty() && snd.sampler.is_none() && p.groups[0].patterns[0].notes.iter().all(|n| n.sound != *s)
        };
        let s = (1..16).find(free).expect("a free sound");
        audio_sound(h, s, &tone_of(2.0), bar);
        let pi = h.state().pattern;
        h.state().doc.as_ref().unwrap().borrow_mut().set_audio_in(0, s, &[pi], Some(0), "t").unwrap();
        (h.state_mut().group, h.state_mut().sound, h.state_mut().song_view) = (0, s, false);
        h.run_steps(3);
        assert_eq!(project_of(h).groups[0].sounds[s].instrument, "AudioModule", "{}", h.state().status);
        (s, tmp)
    }

    /// Sound `s`'s audio offset in the pattern in focus.
    fn offset_of(h: &Harness<'static, App>, s: usize) -> Option<i64> {
        project_of(h).groups[0].patterns[h.state().pattern].audio.iter().find(|c| c.sound == s).map(|c| c.offset)
    }

    /// A drag along grid row `row`, from tick `a` to tick `b` (from the pattern's start).
    fn drag_ticks(h: &mut Harness<'static, App>, row: usize, a: i64, b: i64, modifiers: egui::Modifiers) {
        let (from, to) = (grid_pos(h, row, a), grid_pos(h, row, b));
        h.event(Event::ModifiersChanged(modifiers));
        h.hover_at(from);
        h.step();
        h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers });
        h.step();
        for k in 1..=6 {
            let t = k as f32 / 6.0;
            h.event(Event::PointerMoved(from + (to - from) * t));
            h.step();
        }
        h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers });
        h.step();
        h.event(Event::ModifiersChanged(Default::default()));
        h.run_steps(3);
    }

    /// The one feature beyond Maschine: a pattern's audio dragged along the grid starts
    /// that much later, as one undo step.
    #[test]
    fn dragging_a_loop_moves_its_audio() {
        let Some(mut h) = harness(false, false) else { return };
        let (s, _tmp) = loop_sound(&mut h);
        assert_eq!(offset_of(&h, s), Some(0));
        drag_ticks(&mut h, s, PPQ / 2, PPQ / 2 + PPQ, Default::default());
        assert_eq!(offset_of(&h, s), Some(PPQ));
        let d = h.state().doc.clone().unwrap();
        assert_eq!(d.borrow().undo_labels().first().copied(), Some("Move Audio"));
        assert!(d.borrow_mut().undo());
        assert_eq!(offset_of(&h, s), Some(0));
    }

    #[test]
    fn a_loop_drag_snaps_unless_shift() {
        let Some(mut h) = harness(false, false) else { return };
        let (s, _tmp) = loop_sound(&mut h);
        let step = h.state().grid;
        let by = step * 13 / 10;
        drag_ticks(&mut h, s, PPQ / 2, PPQ / 2 + by, Default::default());
        assert_eq!(offset_of(&h, s), Some(step));
        drag_ticks(&mut h, s, PPQ / 2, PPQ / 2 + by, egui::Modifiers::SHIFT);
        let got = offset_of(&h, s).unwrap();
        assert!((got - (step + by)).abs() <= 1, "{got} vs {}", step + by);
    }

    #[test]
    fn dragging_a_note_on_a_gate_row_moves_the_note() {
        let Some(mut h) = harness(false, false) else { return };
        let (s, _tmp) = loop_sound(&mut h);
        let pi = h.state().pattern;
        let slot = project_of(&h).groups[0].sounds[s].slots.iter().position(|x| x.module == "AudioModule").unwrap();
        layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(deckard_doc::Target::Module(0, s, slot), "mode", 1.0)]);
        let a = project_of(&h).groups[0].sounds[s].audio.clone().unwrap();
        assert_eq!(a.mode, deckard_model::AudioMode::Gate);
        if offset_of(&h, s).is_none() {
            h.state().doc.as_ref().unwrap().borrow_mut().set_audio_in(0, s, &[pi], Some(0), "t").unwrap();
        }
        let start = project_of(&h).groups[0].patterns[pi].start;
        let n = Note { expr: None, sound: s, position: start, length: PPQ, pitch: 60, velocity: 100, probability: 100 };
        h.state().doc.as_ref().unwrap().borrow_mut().edit_notes("t", 0, pi, &[], &[n]).unwrap();
        h.run_steps(3);
        // The note: it moves a beat on, the audio stays.
        // (A little past the beat: a note's move snaps down to the grid.)
        drag_ticks(&mut h, s, PPQ / 4, PPQ / 4 + PPQ + 20, Default::default());
        let notes: Vec<i64> = project_of(&h).groups[0].patterns[pi].notes.iter().filter(|x| x.sound == s).map(|x| x.position).collect();
        assert_eq!(notes, vec![start + PPQ]);
        assert_eq!(offset_of(&h, s), Some(0));
        // Empty space on the row: the audio moves.
        drag_ticks(&mut h, s, 3 * PPQ, 4 * PPQ, Default::default());
        assert_eq!(offset_of(&h, s), Some(PPQ));
    }

    #[test]
    fn the_piano_roll_drags_the_focused_loop() {
        let Some(mut h) = harness(false, false) else { return };
        let (s, _tmp) = loop_sound(&mut h);
        h.state_mut().piano_roll = true;
        h.run_steps(3);
        drag_ticks(&mut h, 2, PPQ / 2, PPQ / 2 + PPQ, Default::default());
        assert_eq!(offset_of(&h, s), Some(PPQ));
    }

    #[test]
    fn a_sync_take_on_an_audio_sound_is_a_new_take() {
        let mut h = blank_harness(true);
        let tmp = tempfile::tempdir().unwrap();
        h.state_mut().path = Some(tmp.path().join("x.mxprj"));
        a_pattern(&mut h);
        let p = project_of(&h);
        let bar = p.bar_length() as f64;
        let s = (1..16).find(|&s| p.groups[0].sounds[s].slots.is_empty() && p.groups[0].sounds[s].sampler.is_none()).unwrap();
        audio_sound(&mut h, s, &tone_of(1.0), bar);
        assert_eq!(project_of(&h).groups[0].sounds[s].audio.as_ref().map(|a| a.takes.len()), Some(1));
        // An effect after the Audio plug-in: a new take leaves it.
        let at = project_of(&h).groups[0].sounds[s].slots.len();
        let fx = deckard_doc::NewModule::Effect(deckard_model::PERFORM_FX.0);
        h.state().doc.as_ref().unwrap().borrow_mut().add_slot(0, s, at, &fx).unwrap();
        let chain: Vec<String> = project_of(&h).groups[0].sounds[s].slots.iter().map(|x| x.module.clone()).collect();
        assert_eq!(chain.len(), at + 1);
        h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 1, ticks: bar, focus: (0, s) });
        let q = project_of(&h);
        let snd = &q.groups[0].sounds[s];
        assert_eq!(snd.slots.iter().map(|x| x.module.clone()).collect::<Vec<_>>(), chain, "the chain survives");
        assert_eq!(snd.instrument, "AudioModule", "{}", h.state().status);
        assert!(snd.sampler.is_none());
        assert_eq!(snd.audio.as_ref().map(|a| (a.takes.len(), a.focused)), Some((2, 1)));
        assert!(q.groups[0].patterns[0].audio.contains(&deckard_model::AudioCue { sound: s, take: 1, offset: 0 }));
    }

    /// A recorded take on an Audio plug-in is edited as a Sampler's sample is: EDIT (no
    /// SLICE or ZONE), its play range set on the take, and TRUNCATE writes a copy the take
    /// plays.
    #[test]
    fn an_audio_take_is_edited_in_the_sampling_view() {
        use layout::sampling::{EditOp, SamplingAct};
        let mut h = blank_harness(true);
        let (s, _tmp) = loop_sound(&mut h);
        h.state_mut().sampling_open = true;
        h.state_mut().sampling.tab = 2;
        h.run_steps(3);
        assert_eq!(h.state().sampling.tab, 1, "no SLICE for a take: EDIT");
        let frames = h.state().sampling.sample.as_ref().map(|x| x.frames.len()).expect("the take comes up");
        let (_, _, t, take) = layout::sampling::edited(&project_of(&h), (0, s, 0)).unwrap();
        assert!(take);
        assert_eq!(t, deckard_doc::Target::Take(0, s, 0));
        // The play range: on the take.
        layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(t, "play_range/low", 100.0)]);
        let take = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[s].audio.as_ref().unwrap().takes[0].clone();
        assert_eq!(take(&h).play_range.0, 100, "{}", h.state().status);
        // TRUNCATE to the first half.
        let before = take(&h).sample.path;
        h.state_mut().sampling.selection = Some((0, frames / 2));
        h.state_mut().sampling_act(SamplingAct::Edit(EditOp::Truncate));
        h.run_steps(3);
        let after = take(&h);
        assert_ne!(after.sample.path, before, "{}", h.state().status);
        assert_eq!(h.state().sampling.sample.as_ref().map(|x| x.frames.len()), Some(frames / 2));
        assert_eq!(project_of(&h).groups[0].sounds[s].instrument, "AudioModule", "still an Audio plug-in");
        // NORMALIZE with nothing selected: the take's play range, all of it.
        let before = take(&h).sample.path;
        h.state_mut().sampling.selection = None;
        h.state_mut().sampling_act(SamplingAct::Edit(EditOp::Normalize));
        h.run_steps(3);
        assert_ne!(take(&h).sample.path, before, "{}", h.state().status);
        let peak = h.state().sampling.sample.as_ref().unwrap().frames.iter().fold(0f32, |m, f| m.max(f[0].abs()));
        assert!(peak > 0.9, "normalized: {peak}");
    }

    /// Loop, Target Sound, started on an empty sound: the take goes there (not the next
    /// one), and the sound takes the recording's name.
    #[test]
    fn a_loop_take_fills_the_empty_focused_sound_and_names_it() {
        use layout::sampling::RecParam;
        let mut h = blank_harness(true);
        let tmp = tempfile::tempdir().unwrap();
        h.state_mut().path = Some(tmp.path().join("x.mxprj"));
        let p = project_of(&h);
        let bar = p.bar_length() as f64;
        let s = (1..16).find(|&s| p.groups[0].sounds[s].slots.is_empty() && p.groups[0].sounds[s].sampler.is_none()).unwrap();
        (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound) = (0, 0, s);
        h.state_mut().recorder.view.set(RecParam::Target, 1);
        h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 2, ticks: bar, focus: (0, s) });
        let q = project_of(&h);
        let snd = &q.groups[0].sounds[s];
        assert_eq!(snd.instrument, "AudioModule", "the focused sound, not the next: {}", h.state().status);
        let file = h.state().recorder.view.takes.last().unwrap().file_stem().unwrap().to_string_lossy().to_string();
        assert_eq!(snd.name, file, "named after the take");
        assert_eq!(file.len(), "20260101T000000".len());
    }

    /// DELETE on a take just recorded takes the recording back (as Undo): the sound is as
    /// it was. After other edits, the take stops playing (off in its patterns).
    #[test]
    fn deleting_a_recorded_take_stops_it_playing() {
        use layout::sampling::{RecParam, RecordAct, SamplingAct};
        let mut h = blank_harness(true);
        let tmp = tempfile::tempdir().unwrap();
        h.state_mut().path = Some(tmp.path().join("x.mxprj"));
        a_pattern(&mut h);
        let p = project_of(&h);
        let bar = p.bar_length() as f64;
        let s = (1..16).find(|&s| p.groups[0].sounds[s].slots.is_empty() && p.groups[0].sounds[s].sampler.is_none()).unwrap();
        let record = |h: &mut Harness<'static, App>| {
            (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound) = (0, 0, s);
            h.state_mut().recorder.view.set(RecParam::Target, 0);
            h.state_mut().finish_recording(recorder::Finished::Take {
                sample: tone_of(1.0),
                channels: 2,
                mode: 2,
                ticks: bar,
                focus: (0, s),
            });
            assert!(project_of(h).groups[0].patterns[0].audio.iter().any(|c| c.sound == s), "{}", h.state().status);
            h.state().recorder.view.takes.len() - 1
        };
        let delete = |h: &mut Harness<'static, App>, i: usize| h.state_mut().sampling_act(SamplingAct::Record(RecordAct::DeleteTake(i)));
        // Straight after: as if it was never recorded.
        let i = record(&mut h);
        delete(&mut h, i);
        let q = project_of(&h);
        assert_eq!(q.groups[0].sounds[s], p.groups[0].sounds[s], "the sound as it was");
        assert!(q.groups[0].patterns[0].audio.iter().all(|c| c.sound != s));
        assert!(h.state().recorder.view.takes.is_empty());
        // With an edit since: off in the pattern, the edit kept.
        let i = record(&mut h);
        let doc = h.state().doc.clone().unwrap();
        let tempo = doc.borrow().project().tempo + 1.0;
        doc.borrow_mut().set_tempo(tempo).unwrap();
        delete(&mut h, i);
        let q = project_of(&h);
        assert!(q.groups[0].patterns[0].audio.iter().all(|c| c.sound != s), "{}", h.state().status);
        assert_eq!(q.tempo, tempo);
    }

    /// A 4-bar Loop take into a group without patterns: the pattern made is 4 bars, the
    /// take whole (Target Sound and Pattern).
    #[test]
    fn a_long_loop_take_into_a_group_without_patterns_makes_a_pattern_as_long() {
        use layout::sampling::RecParam;
        let mut h = blank_harness(true);
        let tmp = tempfile::tempdir().unwrap();
        h.state_mut().path = Some(tmp.path().join("x.mxprj"));
        let doc = h.state().doc.clone().unwrap();
        let bar = project_of(&h).bar_length();
        for (target, song_view) in [(1, true), (2, true), (1, false), (2, false)] {
            h.state_mut().song_view = song_view;
            let g = doc.borrow_mut().add_group().unwrap();
            assert!(project_of(&h).groups[g].patterns.is_empty());
            (h.state_mut().group, h.state_mut().sound) = (g, 0);
            h.state_mut().recorder.view.set(RecParam::Target, target);
            h.state_mut().finish_recording(recorder::Finished::Take {
                sample: tone_of(8.0),
                channels: 2,
                mode: 2,
                ticks: 4.0 * bar as f64,
                focus: (g, 0),
            });
            let q = project_of(&h);
            let pats = &q.groups[g].patterns;
            assert_eq!(pats.len(), 1, "target {target}: one pattern ({})", h.state().status);
            assert_eq!(pats[0].length, 4 * bar, "target {target}: 4 bars");
            assert!(pats[0].audio.iter().any(|c| c.sound == 0), "target {target}: on");
            let scene = &q.scenes[h.state().heard_scene()];
            assert!(scene.patterns.contains(&(g, 0)), "target {target}, song {song_view}: in the scene heard: {:?}", scene.patterns);
        }
    }

    /// A recorded take dropped on another sound's row (or clicked) plays there: an empty
    /// sound gets an Audio plug-in with it at its length, on in the pattern; clicking it
    /// on its own Audio sound selects it (no copy, no Sampler).
    #[test]
    fn a_take_dragged_onto_a_sound_plays_there() {
        use layout::sampling::{RecParam, RecordAct, SamplingAct};
        let mut h = blank_harness(true);
        let tmp = tempfile::tempdir().unwrap();
        h.state_mut().path = Some(tmp.path().join("x.mxprj"));
        let p = project_of(&h);
        let bar = p.bar_length() as f64;
        let free: Vec<usize> =
            (1..16).filter(|&s| p.groups[0].sounds[s].slots.is_empty() && p.groups[0].sounds[s].sampler.is_none()).collect();
        let (s1, s2) = (free[0], free[1]);
        (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound) = (0, 0, s1);
        h.state_mut().recorder.view.set(RecParam::Target, 0);
        h.state_mut().finish_recording(recorder::Finished::Take {
            sample: tone_of(2.0),
            channels: 2,
            mode: 2,
            ticks: 2.0 * bar,
            focus: (0, s1),
        });
        let take = h.state().recorder.view.takes[0].clone();
        // Clicked on its own sound: selected, not copied, still an Audio plug-in.
        h.state_mut().sampling_act(SamplingAct::Record(RecordAct::FocusTake(0)));
        let snd = project_of(&h).groups[0].sounds[s1].clone();
        assert_eq!((snd.instrument.as_str(), snd.audio.as_ref().map(|a| a.takes.len())), ("AudioModule", Some(1)), "{}", h.state().status);
        // Dropped on an empty sound's row.
        h.run_steps(3);
        let row = |h: &Harness<'static, App>, s: usize| -> egui::Rect {
            let rows: Vec<(egui::Rect, (usize, usize))> = h.ctx.data(|d| d.get_temp(egui::Id::new(layout::DROP_ROWS))).expect("sound rows");
            rows.iter().find(|(_, gs)| *gs == (0, s)).expect("the sound's row").0
        };
        let at = row(&h, s2).center();
        h.hover_at(at);
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        egui::DragAndDrop::set_payload(&h.ctx, layout::sampling::DraggedTake(take.clone()));
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(3);
        let q = project_of(&h);
        let snd = &q.groups[0].sounds[s2];
        assert_eq!(snd.instrument, "AudioModule", "{}", h.state().status);
        let a = snd.audio.as_ref().unwrap();
        assert!((a.takes[0].beats - 2.0 * bar / PPQ as f64).abs() < 1e-6, "the take's length: {}", a.takes[0].beats);
        assert!(q.groups[0].patterns[0].audio.iter().any(|c| c.sound == s2), "on in the pattern");
        assert!(q.groups[0].patterns[0].audio.iter().any(|c| c.sound == s1), "still on where it was recorded");
        // RECORD shows it as a tile with its waveform.
        assert!(h.state().recorder.view.take_waves.get(&take).is_some_and(|w| !w.is_empty()), "the take's wave");
        (h.state_mut().sampling_open, h.state_mut().sampling.tab) = (true, 0);
        h.run_steps(3);
        assert_eq!(h.state().sampling.tab, 0);
    }

    /// Recording another take on an Audio sound stops its old take at once; the new take
    /// plays when it's done (DELETE brings the old one back); a cancel brings it back too.
    #[test]
    fn a_new_take_silences_the_old_one_while_it_records() {
        use layout::sampling::{RecParam, RecordAct, SamplingAct};
        let mut h = blank_harness(true);
        let (s, _tmp) = loop_sound(&mut h);
        let bar = project_of(&h).bar_length() as f64;
        h.state_mut().recorder.view.set(RecParam::Mode, 2);
        h.state_mut().recorder.view.set(RecParam::Target, 0);
        let cue = |h: &Harness<'static, App>| {
            project_of(h).groups[0].patterns[h.state().pattern].audio.iter().find(|c| c.sound == s).map(|c| c.take)
        };
        assert_eq!(cue(&h), Some(0));
        // Cancelled: the old take plays again.
        h.state_mut().recorder.fake_recording((0, s));
        h.state_mut().silence_while_recording();
        assert_eq!(cue(&h), None, "silent while recording");
        h.state_mut().sampling_act(SamplingAct::Record(RecordAct::Cancel));
        h.state_mut().silence_while_recording();
        assert_eq!(cue(&h), Some(0), "back after a cancel");
        // Finished: the new take plays; DELETE: the old one again.
        h.state_mut().recorder.fake_recording((0, s));
        h.state_mut().silence_while_recording();
        assert_eq!(cue(&h), None);
        h.state_mut().recorder.cancel(None);
        h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 2, ticks: bar, focus: (0, s) });
        h.state_mut().silence_while_recording();
        assert_eq!(cue(&h), Some(1), "{}", h.state().status);
        let i = h.state().recorder.view.takes.len() - 1;
        h.state_mut().sampling_act(SamplingAct::Record(RecordAct::DeleteTake(i)));
        assert_eq!(cue(&h), Some(0), "the old take back");
    }

    /// A Loop take is heard at once: an Audio plug-in in Gate mode (silent without notes)
    /// goes to Loop. Sync onto one keeps its mode.
    #[test]
    fn a_loop_take_switches_a_gate_audio_plugin_to_loop() {
        let mut h = blank_harness(true);
        let tmp = tempfile::tempdir().unwrap();
        h.state_mut().path = Some(tmp.path().join("x.mxprj"));
        let p = project_of(&h);
        let bar = p.bar_length() as f64;
        let s = (1..16).find(|&s| p.groups[0].sounds[s].slots.is_empty() && p.groups[0].sounds[s].sampler.is_none()).unwrap();
        audio_sound(&mut h, s, &tone_of(1.0), bar);
        let mode = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[s].audio.as_ref().map(|a| a.mode);
        let gate = |h: &mut Harness<'static, App>| {
            let i = project_of(h).groups[0].sounds[s].slots.iter().position(|x| x.module == "AudioModule").unwrap();
            let doc = h.state().doc.clone().unwrap();
            doc.borrow_mut().set("t", deckard_doc::Target::Module(0, s, i), "mode", mxprj::decode::Value::Float(1.0)).unwrap();
        };
        gate(&mut h);
        assert_eq!(mode(&h), Some(deckard_model::AudioMode::Gate));
        h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 1, ticks: bar, focus: (0, s) });
        assert_eq!(mode(&h), Some(deckard_model::AudioMode::Gate), "Sync keeps Gate");
        audio_sound(&mut h, s, &tone_of(1.0), bar);
        assert_eq!(mode(&h), Some(deckard_model::AudioMode::Loop), "{}", h.state().status);
    }

    #[test]
    fn a_detect_take_on_an_audio_sound_is_rounded_to_a_sixteenth() {
        let Some(mut h) = harness(false, true) else { return };
        let tmp = tempfile::tempdir().unwrap();
        h.state_mut().path = Some(tmp.path().join("x.mxprj"));
        let p = project_of(&h);
        let bar = p.bar_length() as f64;
        let s = (1..16).find(|&s| p.groups[0].sounds[s].slots.is_empty() && p.groups[0].sounds[s].sampler.is_none()).unwrap();
        audio_sound(&mut h, s, &tone_of(1.0), bar);
        let ticks = 4.0 * PPQ as f64 + 70.0;
        h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(2.0), channels: 2, mode: 0, ticks, focus: (0, s) });
        let q = project_of(&h);
        let a = q.groups[0].sounds[s].audio.as_ref().expect("still Audio");
        assert_eq!(a.takes.len(), 2, "{}", h.state().status);
        assert_eq!(a.takes[1].beats, 4.0);
    }

    #[test]
    fn a_detect_take_on_an_empty_sound_is_a_sampler_zone() {
        let Some(mut h) = harness(false, true) else { return };
        let tmp = tempfile::tempdir().unwrap();
        h.state_mut().path = Some(tmp.path().join("x.mxprj"));
        let p = project_of(&h);
        let s = (1..16).find(|&s| p.groups[0].sounds[s].slots.is_empty() && p.groups[0].sounds[s].sampler.is_none()).unwrap();
        (h.state_mut().group, h.state_mut().sound) = (0, s);
        h.state_mut().finish_recording(recorder::Finished::Take {
            sample: tone_of(1.0),
            channels: 2,
            mode: 0,
            ticks: 960.0,
            focus: (0, s),
        });
        let q = project_of(&h);
        let snd = &q.groups[0].sounds[s];
        assert_eq!(snd.sampler.as_ref().map(|x| x.zones.len()), Some(1), "{}", h.state().status);
        assert!(snd.audio.is_none());
    }

    #[test]
    fn the_take_goes_to_the_sound_focused_at_start() {
        let Some(mut h) = harness(false, true) else { return };
        let tmp = tempfile::tempdir().unwrap();
        h.state_mut().path = Some(tmp.path().join("x.mxprj"));
        (h.state_mut().group, h.state_mut().sound) = (0, 5);
        h.state_mut().finish_recording(recorder::Finished::Take {
            sample: tone_of(1.0),
            channels: 2,
            mode: 0,
            ticks: 960.0,
            focus: (0, 2),
        });
        let q = project_of(&h);
        assert_eq!(q.groups[0].sounds[2].sampler.as_ref().map(|x| x.zones.len()), Some(1), "{}", h.state().status);
        assert!(q.groups[0].sounds[5].sampler.as_ref().is_none_or(|x| x.zones.is_empty()));
    }

    #[test]
    fn with_no_free_sound_left_a_take_goes_to_the_last_audio_sound() {
        use layout::sampling::RecParam;
        let Some(mut h) = harness(false, true) else { return };
        let tmp = tempfile::tempdir().unwrap();
        h.state_mut().path = Some(tmp.path().join("x.mxprj"));
        let p = project_of(&h);
        let bar = p.bar_length() as f64;
        // Sound 9 an Audio plug-in, sound 15 a Sampler, the rest in between taken.
        audio_sound(&mut h, 9, &tone_of(1.0), bar);
        (h.state_mut().group, h.state_mut().sound) = (0, 15);
        h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 0, ticks: bar, focus: (0, 15) });
        for s in 1..15 {
            if s != 9 {
                (h.state_mut().group, h.state_mut().sound) = (0, s);
                h.state_mut().finish_recording(recorder::Finished::Take {
                    sample: tone_of(0.2),
                    channels: 2,
                    mode: 0,
                    ticks: bar,
                    focus: (0, s),
                });
            }
        }
        assert!(project_of(&h).groups[0].sounds[15].sampler.is_some());
        (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound) = (0, 0, 0);
        h.state_mut().recorder.view.set(RecParam::Target, 1);
        h.state_mut().finish_recording(recorder::Finished::Take { sample: tone_of(1.0), channels: 2, mode: 2, ticks: bar, focus: (0, 0) });
        let q = project_of(&h);
        assert_eq!(q.groups[0].sounds[9].audio.as_ref().map(|a| a.takes.len()), Some(2), "{}", h.state().status);
        assert!(q.groups[0].sounds[15].sampler.is_some());
    }

    #[test]
    fn auto_with_no_free_sound_left_goes_to_the_last_sound_not_the_audio_one() {
        let Some(mut h) = harness(false, true) else { return };
        let tmp = tempfile::tempdir().unwrap();
        h.state_mut().path = Some(tmp.path().join("x.mxprj"));
        let bar = project_of(&h).bar_length() as f64;
        audio_sound(&mut h, 9, &tone_of(1.0), bar);
        for s in 1..16 {
            if s != 9 {
                (h.state_mut().group, h.state_mut().sound) = (0, s);
                h.state_mut().finish_recording(recorder::Finished::Take {
                    sample: tone_of(0.2),
                    channels: 2,
                    mode: 0,
                    ticks: bar,
                    focus: (0, s),
                });
            }
        }
        // Focused on the Audio sound (no Sampler) at Start, so Auto looks for another; the
        // app's own focus has moved on since.
        (h.state_mut().group, h.state_mut().sound) = (0, 3);
        assert_eq!(h.state().next_free_sound(0, 9, false), 15);
        assert_eq!(h.state().next_free_sound(0, 9, true), 9);
        let path = tmp.path().join("Kick 100.wav");
        let s = tone_of(0.5);
        deckard_engine::edit::write_wav(&path, &s).unwrap();
        let item = recorder::AutoItem { note: 60, velocity: 100, keys: (0, 127), vels: (0, 127), path };
        h.state_mut().finish_recording(recorder::Finished::Auto { done: vec![(item, s.frames.len(), None)], focus: (0, 9) });
        let q = project_of(&h);
        assert_eq!(q.groups[0].sounds[9].instrument, "AudioModule");
        assert!(q.groups[0].sounds[9].sampler.is_none(), "{}", h.state().status);
        assert_eq!(h.state().sound, 15, "{}", h.state().status);
    }

    /// The divider above the chain area: dragged, the patterns and scenes take less or more
    /// of the window and everything under them the rest; double-clicked, back as it was.
    #[test]
    fn the_song_divider_resizes_what_is_above_and_below_it() {
        let Some(mut h) = harness(false, false) else { return };
        h.run_steps(3);
        let top = |h: &Harness<'static, App>| h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new("deckard-grid"))).expect("grid drawn");
        // (Where the divider is now, across the middle of the window.)
        let at = |h: &Harness<'static, App>| {
            h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new("deckard-song-divider"))).expect("divider drawn").center()
        };
        let grid0 = top(&h);
        assert_eq!(h.state().settings.song_panel_h, None, "the window's size decides");
        // Up: less above, and the editor grows by as much.
        let from = at(&h);
        drag(&mut h, from, from - egui::vec2(0.0, 100.0));
        assert_eq!(h.state().settings.song_panel_h, Some(243.0));
        assert!((at(&h).y - (from.y - 100.0)).abs() < 0.5, "it followed the pointer");
        assert!((top(&h).top() - (grid0.top() - 100.0)).abs() < 0.5, "{:?}", top(&h));
        assert!((top(&h).height() - (grid0.height() + 100.0)).abs() < 0.5, "{:?}", top(&h));
        assert_eq!(top(&h).bottom(), grid0.bottom(), "its foot stays");
        // As far up as it goes.
        let from = at(&h);
        drag(&mut h, from, Pos2::new(from.x, 0.0));
        assert_eq!(h.state().settings.song_panel_h, Some(147.0));
        if let Ok(dir) = std::env::var("DECKARD_SHOTS") {
            h.render().unwrap().save(Path::new(&dir).join("divider-up.png")).unwrap();
        }
        // As far down: the editor keeps its room.
        let from = at(&h);
        drag(&mut h, from, Pos2::new(from.x, 2000.0));
        let down = h.state().settings.song_panel_h.unwrap();
        assert!(down > 343.0, "{down}");
        assert!(top(&h).height() > 30.0, "{:?}", top(&h));
        if let Ok(dir) = std::env::var("DECKARD_SHOTS") {
            h.render().unwrap().save(Path::new(&dir).join("divider-down.png")).unwrap();
        }
        // Clicked without moving: stays. Double-clicked: as it was.
        let from = at(&h);
        click(&mut h, from);
        assert_eq!(h.state().settings.song_panel_h, Some(down));
        click(&mut h, from);
        assert_eq!(h.state().settings.song_panel_h, None);
        h.run_steps(2);
        assert_eq!(top(&h), grid0);
    }

    /// The Edit tab always shows the play range, the selection and the loop, in samples; a
    /// sample comes up selected; a value can be typed to the sample.
    #[test]
    fn the_edit_tab_shows_and_types_its_ranges_in_samples() {
        let Some(mut h) = harness(false, true) else { return };
        (h.state_mut().group, h.state_mut().sound) = (0, 0);
        h.run_steps(3);
        let panel_top = 33.0 + 3.0 + 343.0 + 2.0 + 140.0 + 2.0;
        click(&mut h, Pos2::new(3.0 + 16.0, panel_top + 7.0 + 4.0 * 28.0 + 12.0));
        h.run_steps(3);
        let zone = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].clone();
        let (lo, hi) = zone(&h).play_range;
        assert_eq!(h.state().sampling.selection, Some((lo as usize, hi as usize + 1)), "the play range");
        let at = |h: &Harness<'static, App>, group: &str, label: &str| -> egui::Rect {
            let id = egui::Id::new(("deckard-sample-field", group.to_string(), label.to_string()));
            h.ctx.data(|d| d.get_temp(id)).unwrap_or_else(|| panic!("{group} {label} shown"))
        };
        for (group, label) in [
            ("PLAY RANGE", "START"),
            ("PLAY RANGE", "END"),
            ("SELECTION RANGE", "START"),
            ("SELECTION RANGE", "END"),
            ("LOOP", "ACTIVE"),
            ("LOOP", "START"),
            ("LOOP", "END"),
            ("LOOP", "XFADE"),
        ] {
            at(&h, group, label);
        }
        let typed = |h: &mut Harness<'static, App>, r: egui::Rect, text: &str| {
            h.run_steps(40);
            let c = r.center();
            h.hover_at(c);
            h.step();
            for _ in 0..2 {
                for pressed in [true, false] {
                    h.event(Event::PointerButton { pos: c, button: PointerButton::Primary, pressed, modifiers: Default::default() });
                    h.step();
                }
            }
            h.run_steps(2);
            h.event(Event::Text(text.into()));
            h.step();
            h.event(Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
            h.run_steps(3);
        };
        // To the sample, however long the sample is.
        let r = at(&h, "PLAY RANGE", "START");
        typed(&mut h, r, "1237");
        assert_eq!(zone(&h).play_range.0, 1237);
        let r = at(&h, "LOOP", "END");
        typed(&mut h, r, "9011");
        assert_eq!(zone(&h).loop_range.1, 9011);
        let r = at(&h, "LOOP", "START");
        typed(&mut h, r, "1298");
        assert_eq!(zone(&h).loop_range.0, 1298);
        assert_eq!(zone(&h).play_range.0, 1237, "the start stays 61 samples before the loop");
        let r = at(&h, "LOOP", "XFADE");
        typed(&mut h, r, "500");
        assert_eq!(zone(&h).x_fade, 500);
        let r = at(&h, "SELECTION RANGE", "END");
        typed(&mut h, r, "5000");
        assert_eq!(h.state().sampling.selection.map(|s| s.1), Some(5001), "both ends in, as it reads");
        if let Ok(dir) = std::env::var("DECKARD_SHOTS") {
            h.render().unwrap().save(Path::new(&dir).join("sample-edit.png")).unwrap();
        }
        // Nothing selected: still there, and an end selects from the play range.
        let wave: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-sample-wave"))).expect("waveform drawn");
        click(&mut h, Pos2::new(wave.center().x, wave.center().y));
        assert_eq!(h.state().sampling.selection, None, "deselected");
        at(&h, "SELECTION RANGE", "START");
        let r = at(&h, "SELECTION RANGE", "END");
        typed(&mut h, r, "6000");
        assert_eq!(h.state().sampling.selection, Some((1237, 6001)));
    }

    /// A sampled sound opens on the zone it was saved focused on, not always the first.
    #[test]
    fn a_sound_opens_on_its_saved_zone() {
        let Some(mut h) = harness(false, false) else { return };
        let p = project_of(&h);
        let found = p.groups.iter().enumerate().find_map(|(g, x)| {
            x.sounds.iter().enumerate().find_map(|(s, y)| y.sampler.as_ref().filter(|z| z.focused_zone > 0).map(|z| (g, s, z.focused_zone)))
        });
        let Some((g, s, saved)) = found else {
            eprintln!("no sound saved on a later zone; skipping");
            return;
        };
        (h.state_mut().group, h.state_mut().sound) = (g, s);
        h.run_steps(3);
        assert_eq!(h.state().zone, saved);
        // Focused by hand, it stays while the sound is focused.
        h.state_mut().zone = 0;
        h.run_steps(3);
        assert_eq!(h.state().zone, 0);
    }

    /// The sample's zooming scroll bar, as the Pattern Editor's: up zooms in round where
    /// it was grabbed, along scrolls, an end moves with the other held, double-click shows
    /// it all.
    #[test]
    fn zoom_bar_zooms_and_scrolls_the_sample() {
        let Some(mut h) = harness(false, true) else { return };
        (h.state_mut().group, h.state_mut().sound) = (0, 0);
        h.run_steps(3);
        let panel_top = 33.0 + 3.0 + 343.0 + 2.0 + 140.0 + 2.0;
        click(&mut h, Pos2::new(3.0 + 16.0, panel_top + 7.0 + 4.0 * 28.0 + 12.0));
        h.run_steps(3);
        assert!(h.state().sampling.sample.is_some(), "the kick's sample is shown");
        let wave: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-sample-wave"))).expect("waveform drawn");
        let bar_y = wave.bottom() + 1.0 + 8.0;
        let len = h.state().sampling.len() as f64;
        let window = |h: &Harness<'static, App>| {
            let (a, b) = h.state().sampling.view();
            (a / len, (b - a) / len)
        };
        assert_eq!(h.state().sampling.view, None, "all of it");
        // Up from the middle: zoomed in, still round the middle.
        let mid = Pos2::new(wave.center().x, bar_y);
        drag(&mut h, mid, mid - egui::vec2(0.0, 60.0));
        let (from, part) = window(&h);
        assert!(part < 0.6, "zoomed in: {part}");
        assert!((from + part / 2.0 - 0.5).abs() < 0.05, "round the middle: {from} {part}");
        // Along: scrolled right, as far in.
        let thumb = Pos2::new(wave.left() + ((from + part / 2.0) as f32) * wave.width(), bar_y);
        drag(&mut h, thumb, thumb + egui::vec2(40.0, 0.0));
        assert!(window(&h).0 > from, "scrolled");
        assert!((window(&h).1 - part).abs() < 1e-4, "as far in");
        // Its right end, in: the left stays.
        let (from, part) = window(&h);
        let end = Pos2::new(wave.left() + ((from + part) as f32) * wave.width() - 2.0, bar_y);
        drag(&mut h, end, end - egui::vec2(30.0, 0.0));
        assert!((window(&h).0 - from).abs() < 1e-4, "the left end held");
        assert!(window(&h).1 < part, "shorter");
        // Double-click: all of it again.
        let (from, part) = window(&h);
        let thumb = Pos2::new(wave.left() + ((from + part / 2.0) as f32) * wave.width(), bar_y);
        for _ in 0..2 {
            click(&mut h, thumb);
        }
        h.run_steps(2);
        assert_eq!(h.state().sampling.view, None, "reset");
        // Up and up: no further in than the waveform goes.
        drag(&mut h, mid, mid - egui::vec2(0.0, 900.0));
        let (a, b) = h.state().sampling.view();
        assert!(b - a >= 63.0 && b - a < len, "{a} {b}");
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
        // Duplicate moved the next pattern on a place; Delete leaves the others where they are.
        let mut moved = before.groups[g].patterns[1].clone();
        moved.slot += 1;
        assert_eq!(p.groups[g].patterns[1], moved);
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
            // (Coming to a chain shows where it was left: switched first, then paged.)
            h.state_mut().chain_tab = tab;
            h.run_steps(1);
            for set in 0..3 {
                for page in 0..3 {
                    (h.state_mut().prop_set, h.state_mut().prop_page) = (set, page);
                    h.run_steps(3);
                    shot(&mut h, &format!("{level}-{set}-{page}"));
                }
            }
        }
        h.state_mut().chain_tab = layout::ChainTab::Sound;
        h.run_steps(1);
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
        h.run_steps(1);
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
        h.run_steps(1);
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
        app.session_written();
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

    #[test]
    fn ni_resources_pictures_resolve_slots_and_browser_products() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, thumb_png, logo_png) = plugin_pictures::PICTURES[0];
        let omni_art = tmp.path().join("omnisphere");
        let omni_vendor = tmp.path().join("spectrasonics").join("omnisphere");
        let kontakt = tmp.path().join("kontakt");
        for d in [&omni_art, &omni_vendor, &kontakt] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(omni_art.join("MST_artwork.png"), thumb_png).unwrap();
        std::fs::write(omni_vendor.join("MST_plugin.png"), thumb_png).unwrap();
        std::fs::write(omni_vendor.join("MST_logo.png"), logo_png).unwrap();
        std::fs::write(kontakt.join("MST_artwork.png"), thumb_png).unwrap();
        std::fs::write(kontakt.join("MST_logo.png"), logo_png).unwrap();

        let prev = std::env::var_os("DECKARD_NI_IMAGE_DIR");
        // SAFETY: scoped to this unit test; `with_ni_pictures` rescans when the path changes.
        unsafe { std::env::set_var("DECKARD_NI_IMAGE_DIR", tmp.path()) };
        let s = deckard_surface::Surface::new();
        assert_eq!(plugin_pictures::slot_key(&s, "Omnisphere").as_deref(), Some("omnisphere"));
        assert!(s.has_picture("omnisphere", true) && s.has_picture("omnisphere", false));
        assert_eq!(plugin_pictures::slot_key(&s, "Kontakt 7").as_deref(), Some("kontakt7"));
        assert!(s.has_picture("kontakt7", true) && s.has_picture("kontakt7", false));
        plugin_pictures::ensure_browser_product(&s, "Omnisphere");
        assert!(s.has_picture("artwork:omnisphere", true) && s.has_picture("artwork:omnisphere", false));
        match prev {
            Some(v) => unsafe { std::env::set_var("DECKARD_NI_IMAGE_DIR", v) },
            None => unsafe { std::env::remove_var("DECKARD_NI_IMAGE_DIR") },
        }
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
            // Padded to 240 × 196 with nothing: the screen's own background shows around
            // the panel, not black bars (C58).
            let mut logo = image::RgbaImage::from_pixel(240, 196, image::Rgba([0, 0, 0, 0]));
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

    #[test]
    fn mk3_knob_burst_keeps_full_travel() {
        use deckard_mk3::Event as E;
        let mut app = App::default();
        app.new_project();
        app.controller = None;
        let mut req = Vec::new();
        // On the Mixer page (`knob` reads `deckard.state()` directly), a burst of 10
        // 500 Hz reports in one frame (+0.10 turn = +7 dB) must move by the full sum.
        app.mk3_events(vec![E::Button("BUTTON_MIX", true), E::Button("BUTTON_MIX", false)], &mut req);
        let before = app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].level;
        app.mk3_events(vec![E::Knob(0, 0.01); 10], &mut req);
        let after = app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].level;
        let db = deckard_model::level_db(after) - deckard_model::level_db(before);
        assert!((db - 7.0).abs() < 0.05, "mixer knob moved by full +7 dB burst: {before} -> {after} ({db:+.2} dB)");

        // On the Control page with a native plug-in (Replicant), a burst of 10 reports
        // in one frame on knob 4 (Detune, continuous) must move by the full +0.20.
        app.mk3_events(vec![E::Button("BUTTON_PLUGIN", true), E::Button("BUTTON_PLUGIN", false)], &mut req);
        let cls = deckard_plugins::native::CLASSES[0];
        app.doc
            .as_ref()
            .unwrap()
            .borrow_mut()
            .add_slot(0, 0, 0, &deckard_doc::NewModule::Plugin { id: cls.plugin_id(), name: cls.name.into(), chunk: Vec::new() })
            .unwrap();
        app.slot = Some(0);
        app.mk3_events(Vec::new(), &mut req);
        let p0 = app.plugin_view().unwrap().values[3].0;
        app.mk3_events(vec![E::Knob(3, 0.02); 10], &mut req);
        let p1 = app.plugin_view().unwrap().values[3].0;
        let want = (p0 + 0.20).clamp(0.0, 1.0);
        assert!((p1 - want).abs() < 1e-4, "plug-in knob moved by full burst: {p0} -> {p1} (want {want})");
    }

    #[test]
    fn mk3_touchstrip_move_burst_lights_pitch_and_mod_without_redrawing_screens() {
        use deckard_mk3::Event as E;
        let mut app = App::default();
        app.new_project();
        app.controller = None;
        let mut req = Vec::new();
        let (_, frame) = app.mk3_events(Vec::new(), &mut req).screens.expect("initial screen draw");
        assert!(matches!(&frame, mk3_io::ScreenFrame::Picture(p) if p.px.is_empty()), "UI thread records DrawOps without rasterizing");
        assert_eq!(frame.into_pixels().len(), 960 * 272, "display thread rasterizes the 960x272 frame");

        // Pitch mode: select PITCH, flush the screen update, then slide to 0.9 in a burst.
        app.mk3_events(vec![E::Button("BUTTON_PITCH", true), E::Button("BUTTON_PITCH", false)], &mut req);
        app.mk3_events(Vec::new(), &mut req);
        let out = app.mk3_events(vec![E::Touch(0, 0, 0.5), E::Touch(0, 1, 0.65), E::Touch(0, 1, 0.9)], &mut req);
        assert!(out.screens.is_none(), "touchstrip pitch move does not re-rasterize the 960x272 screens");
        // Upper half of the strip (dots 13..=23) is lit, lower half (dots 1..=12) is dark.
        assert_eq!(out.leds.get("LED_TS_12"), Some(0), "dot 12 stays dark on upward bend");
        assert!(out.leds.get("LED_TS_13").unwrap() > 0 && out.leds.get("LED_TS_23").unwrap() > 0, "dots 13..23 light up on upward bend");

        // Mod mode: select MOD, flush the screen update, then slide to 0.8 in a burst.
        app.mk3_events(vec![E::Button("BUTTON_MOD", true), E::Button("BUTTON_MOD", false)], &mut req);
        app.mk3_events(Vec::new(), &mut req);
        let out = app.mk3_events(vec![E::Touch(0, 0, 0.2), E::Touch(0, 1, 0.5), E::Touch(0, 1, 0.8)], &mut req);
        assert!(out.screens.is_none(), "touchstrip mod move does not re-rasterize the 960x272 screens");
        assert!(out.leds.get("LED_TS_01").unwrap() > 0 && out.leds.get("LED_TS_20").unwrap() > 0, "mod bar lights up to dot 20");
        assert_eq!(out.leds.get("LED_TS_25"), Some(0), "top dot stays dark at 0.8");
    }

    /// A group with no pattern: the touch strip is sent the place in the bar.
    #[test]
    fn the_strip_tracks_the_bar_in_a_group_without_patterns() {
        let mut app = App::default();
        app.new_project();
        app.controller = None;
        let doc = app.doc.clone().unwrap();
        let bar = doc.borrow().project().bar_length();
        app.group = doc.borrow_mut().add_group().unwrap();
        assert_eq!(app.scene_pattern(app.group), None);
        assert_eq!(app.surface_view().progress, None, "no playhead, nothing to track");
        app.seek_pattern(bar / 4);
        assert_eq!(app.surface_view().progress, Some(0.25), "a quarter through its bar");
    }

    #[test]
    fn seek_pattern_moves_playhead_only_when_stopped() {
        let mut app = App::default();
        app.new_project();
        app.controller = None;
        app.seek_pattern(2 * PPQ);
        let head = app.shown_playhead().expect("parked playhead set while stopped");
        assert_eq!(head.position as i64, 2 * PPQ);
        // While playing, clicking the timeline must not move the playhead.
        app.playing = Some(Mode::Scene(0));
        app.seek_pattern(3 * PPQ);
        assert_eq!(app.parked.expect("parked unchanged").position as i64, 2 * PPQ);
    }

    /// Drawing or adding notes in the Pattern Editor when the focused group has no active
    /// pattern creates `"Pattern 1"` with the default pattern length, assigns it to the
    /// active scene, and adds the drawn notes in a single undo step.
    #[test]
    fn drawing_notes_without_a_pattern_creates_and_assigns_one() {
        let mut app = App { follow: true, grid: PPQ / 4, prop_set: 1, ..Default::default() };
        app.new_project();
        app.controller = None;
        let mut h = Harness::builder()
            .with_size(egui::vec2(1500.0, 930.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
        h.run_steps(3);

        assert!(project_of(&h).groups[0].patterns.is_empty());
        assert_eq!(h.state().scene_pattern(0), None);
        let default_len = h.state().default_pattern_length(&project_of(&h));

        // 1. Clicking in Draw mode on the Group (pad-step) note grid creates Pattern 1,
        // assigns it to the active scene, focuses the sound, and places the note in it.
        h.state_mut().mouse = layout::MouseMode::Draw;
        h.run_steps(2);
        let step = PPQ / 4;
        let at = grid_pos(&h, 2, step + 10);
        click(&mut h, at);
        let p = project_of(&h);
        assert_eq!(p.groups[0].patterns.len(), 1, "a pattern was created");
        assert_eq!(p.groups[0].patterns[0].name, "Pattern 1");
        assert_eq!(p.groups[0].patterns[0].length, default_len);
        assert_eq!(p.scenes[0].patterns, vec![(0, 0)], "assigned to the active scene");
        assert_eq!(h.state().pattern, 0);
        assert_eq!(h.state().sound, 2, "the clicked sound row is focused");
        assert_eq!(p.groups[0].patterns[0].notes.len(), 1);
        let note = p.groups[0].patterns[0].notes[0];
        assert_eq!((note.sound, note.position, note.length), (2, step, step));

        // Undo removes both the note and the auto-created pattern in one step.
        let doc = h.state().doc.clone().unwrap();
        assert_eq!(doc.borrow().undo_labels(), vec!["Add Note"]);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_steps(2);
        assert!(project_of(&h).groups[0].patterns.is_empty(), "single undo removes pattern and note");
        assert!(project_of(&h).scenes[0].patterns.is_empty());

        // 2. Click-and-drag painting across steps in Draw mode when there is no pattern
        // creates Pattern 1 and places all painted steps in one undo step, and dragging
        // the right edge of a note adjusts its length.
        let (from, to) = (grid_pos(&h, 1, 10), grid_pos(&h, 1, 2 * step + 10));
        drag(&mut h, from, to);
        let p = project_of(&h);
        assert_eq!(p.groups[0].patterns.len(), 1);
        assert_eq!(p.scenes[0].patterns, vec![(0, 0)]);
        let pos: Vec<i64> = p.groups[0].patterns[0].notes.iter().filter(|n| n.sound == 1).map(|n| n.position).collect();
        assert_eq!(pos, vec![0, step, 2 * step], "three steps painted");
        assert_eq!(doc.borrow().undo_labels(), vec!["Draw Notes"]);

        // Drag the right edge of the last painted note to resize its length.
        let edge_from = grid_pos(&h, 1, 3 * step) - egui::vec2(2.0, 0.0);
        let edge_to = grid_pos(&h, 1, 5 * step) - egui::vec2(2.0, 0.0);
        drag(&mut h, edge_from, edge_to);
        let p = project_of(&h);
        let last = p.groups[0].patterns[0].notes.iter().find(|n| n.sound == 1 && n.position == 2 * step).expect("last note");
        assert!(last.length > step, "right-edge drag extended the note length: {}", last.length);
        assert_eq!(doc.borrow().undo_labels(), vec!["Resize Notes", "Draw Notes"]);

        // Undo both steps back to no pattern.
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_steps(2);
        assert!(project_of(&h).groups[0].patterns.is_empty());

        // 3. Keyboard (piano-roll) note grid in Draw mode when there is no pattern creates
        // Pattern 1, assigns it to the scene, and places the note at the clicked pitch.
        h.state_mut().piano_roll = true;
        h.state_mut().sound = 3;
        h.run_steps(2);
        let at = grid_pos(&h, 4, step + 10);
        click(&mut h, at);
        let p = project_of(&h);
        assert_eq!(p.groups[0].patterns.len(), 1);
        assert_eq!(p.scenes[0].patterns, vec![(0, 0)]);
        assert_eq!(p.groups[0].patterns[0].notes.len(), 1);
        let pr_note = p.groups[0].patterns[0].notes[0];
        assert_eq!((pr_note.sound, pr_note.position), (3, step));

        // 4. Double-clicking in Select mode on a newly added Group with no pattern creates
        // Pattern 1 for that Group, assigns it to the active scene, and adds the note.
        h.state_mut().piano_roll = false;
        h.state_mut().mouse = layout::MouseMode::Select;
        h.state_mut().arrange(layout::Act::AddGroup);
        h.run_steps(2);
        assert_eq!(h.state().group, 1);
        assert!(project_of(&h).groups[1].patterns.is_empty());
        assert_eq!(h.state().scene_pattern(1), None);
        let at = grid_pos(&h, 0, 2 * step + 10);
        double_click(&mut h, at);
        let p = project_of(&h);
        assert_eq!(p.groups[1].patterns.len(), 1);
        assert_eq!(p.groups[1].patterns[0].name, "Pattern 1");
        assert!(p.scenes[0].patterns.contains(&(1, 0)), "Group B's new pattern is in the scene");
        assert_eq!(p.groups[1].patterns[0].notes.len(), 1);
        assert_eq!(p.groups[1].patterns[0].notes[0].position, 2 * step);
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

#[test]
fn confirmed_quit_bypasses_periodic_work_and_keeps_close_requested() {
    let mut app = App::default();
    let ctx = eframe::egui::Context::default();
    ctx.begin_pass(Default::default());
    assert!(!app.finish_quit(&ctx));
    app.proceed(&ctx, Pending::Quit);
    assert!(app.finish_quit(&ctx));
    assert!(app.quitting);
    let mut output = ctx.end_pass();
    output.textures_delta.clear();
    assert!(
        output.viewport_output[&eframe::egui::ViewportId::ROOT].commands.iter().any(|c| matches!(c, eframe::egui::ViewportCommand::Close))
    );
}
