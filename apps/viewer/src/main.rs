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
/// A zone's meta after an edit took its sample from `old_len` frames to `new_len`:
/// its warp markers go; stretched (`stretched`), its tempo follows; cut, its length in
/// beats does, and its grid moves by the `shift` frames taken from the front.
fn resized(meta: &mut deckard_model::new_sampler::ZoneMeta, old_len: usize, new_len: usize, rate: u32, (shift, stretched): (usize, bool)) {
    if new_len == old_len || old_len == 0 {
        return;
    }
    let ratio = new_len as f64 / old_len as f64;
    meta.warps.clear();
    if stretched {
        meta.bpm = meta.bpm.map(|b| b / ratio);
        meta.grid_offset = (meta.grid_offset as f64 * ratio) as u64;
    } else {
        meta.beats = meta.beats.map(|b| b * ratio);
        if let Some(per) = meta.bpm.map(|b| f64::from(rate) * 60.0 / b).filter(|p| *p >= 1.0) {
            meta.grid_offset = (meta.grid_offset as f64 - shift as f64).rem_euclid(per) as u64;
        }
    }
}

/// The stem lane the bass is on ([`deckard_model::new_sampler::LANE_NAMES`]).
const BASS_LANE: usize = 2;

mod drag_out;
mod drop_pos;
mod drum_ui;
mod export;
mod extra_outs;
mod github;
mod goodbye;
mod group_move;
mod icon;
#[cfg(test)]
mod quit_tests;
mod theme_editor;

/// The version shown: the release's (`DECKARD_VERSION`, set by the build scripts), else
/// the crate's.
pub const VERSION: &str = match option_env!("DECKARD_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
/// Built by the release scripts (with a version minted on GitHub), not from source.
pub const RELEASE_BUILD: bool = option_env!("DECKARD_VERSION").is_some();
pub(crate) type SurfaceViewStamp = (
    Option<(usize, u64)>,
    usize,
    usize,
    usize,
    Option<usize>,
    bool,
    Option<usize>,
    Option<deckard_doc::Chain>,
    Option<((deckard_doc::Chain, usize), usize, std::time::Instant)>,
    bool,
);
mod analysis;
#[cfg(test)]
mod auto_length_tests;
mod beat;
mod chop;
mod chop_app;
mod chop_group;
#[cfg(test)]
mod chop_group_tests;
#[cfg(test)]
mod chop_tests;
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
mod jam_strip;
#[cfg(test)]
mod jam_tests;
mod jam_window;
mod keep_running;
mod keys;
mod layers;
#[cfg(test)]
mod layers_tests;
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
mod midi_ports;
mod mk3_io;
mod mod_slots;
mod modulation;
#[cfg(test)]
mod modulation_tests;
mod module_ui;
mod ni_agent;
mod ni_services;
#[cfg(test)]
mod pad_blink_tests;
#[cfg(test)]
mod pad_light_tests;
mod panic_log;
#[cfg(test)]
mod parity_tests;
mod plugin_macros;
#[cfg(test)]
mod plugin_panel_tests;
mod plugin_pictures;
mod plugin_spectrum;
#[cfg(test)]
mod preferences_tests;
mod preset_load;
#[cfg(test)]
mod print_tests;
#[cfg(test)]
mod rec_tests;
mod record;
mod record_knobs;
mod recorder;
mod remote;
mod render_jobs;
mod sampler_ui;
mod sampling_hw;
#[cfg(test)]
mod scene_tempo_tests;
mod session_backup;
mod session_record;
#[cfg(test)]
mod session_tests;
mod session_window;
mod settings;
mod splash;
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
    deckard_plugins::set_fault_logger(panic_log::record_plugin_fault);
    // A panic would leave through the loaded plug-ins' exit-time destructors (which
    // crash in Massive, Guitar Rig, Kontakt); report it and end the process instead.
    let report = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        panic_log::record(info);
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
    // --goodbye OUT.png: draw the standby picture, and stop. Closing leaves the displays black.
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
    // `--jam KEY` (repeatable): press a Jam control through the script, so the strip and its
    // window can be looked at with no Jam on the wire. An unknown name is named back.
    let jam_shot = args.iter().any(|a| a == "--jam-shot");
    let jam_hil_presses: Vec<&'static str> = args
        .windows(2)
        .filter(|w| w[0] == "--jam")
        .filter_map(|w| {
            let name = w[1].as_str();
            let known = deckard_jam::map::BUTTON_LEDS.iter().copied().chain(["ENCODER_TOUCH", "ENCODER"]).find(|n| *n == name);
            if known.is_none() {
                eprintln!("--jam: no Jam control called {name}");
            }
            known
        })
        .collect();
    // (flag, value) pairs for --press/--pad, in order.
    let actions: Vec<(String, String)> =
        args.windows(2).filter(|w| ["--press", "--pad", "--hold"].contains(&w[0].as_str())).map(|w| (w[0].clone(), w[1].clone())).collect();
    let width = if scripts.is_some() { 1500.0 + layout::CONTROLLER_WIDTH } else { 1500.0 };
    let app_icon = settings::Settings::load().app_icon;
    let vsync = std::env::var_os("DECKARD_VSYNC").is_some();
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    wgpu_options.surface.present_mode = if vsync { eframe::wgpu::PresentMode::AutoVsync } else { eframe::wgpu::PresentMode::AutoNoVsync };
    wgpu_options.surface.desired_maximum_frame_latency = Some(1);
    let options = eframe::NativeOptions {
        // The icon picked in Preferences ▸ General, from the start.
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([width, 930.0])
            .with_drag_and_drop(true)
            .with_icon(std::sync::Arc::new(app_icon.icon_data())),
        wgpu_options,
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
                jam_hil: !jam_hil_presses.is_empty(),
                jam_hil_presses,
                jam_shot,
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
            if app_icon != icon::Style::default() {
                // The Dock and the taskbar's big icon, which the window's icon doesn't set
                // (on every launch, splash or not).
                icon::apply(&cc.egui_ctx, app_icon);
            }
            if splash {
                // The frame loop runs startup, a step a frame; there are no flags to apply.
                return Ok(Box::new(app));
            }
            app.boot_to_end();
            app.boot = None;
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
                Some("ratchet") => app.lane = layout::Lane::Ratchet,
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
/// page is kept with the sound, `Document::groove_page`.)
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
    instance: std::sync::Weak<deckard_plugins::SharedPlugin>,
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
/// macro), the pages saved with it, or else its mapping's pages (`mapped`, from the
/// controller scripts' `mappings/`) followed by Maschine's automatic ones: its first 128
/// automatable parameters, eight per page.
fn view_pages(
    p: &deckard_plugins::Plugin,
    saved: &[deckard_model::PluginPage],
    mapped: Option<&[deckard_surface::widget::mapping::PageParams]>,
) -> Vec<ViewPage> {
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
        let mut params = p.parameters();
        // The mapping's pages first, in its order (each knob found by id, title or name);
        // the parameters they leave follow, eight a page.
        let mut pages = Vec::new();
        for pg in mapped.unwrap_or_default() {
            let knobs: Vec<_> = pg
                .knobs
                .iter()
                .flatten()
                .filter_map(|r| {
                    let found =
                        r.id.and_then(|id| params.iter().position(|i| i.id == id))
                            .or_else(|| r.title.as_ref().and_then(|t| params.iter().position(|i| i.title.eq_ignore_ascii_case(t))))
                            .or_else(|| params.iter().position(|i| r.knob.iter().any(|n| n.eq_ignore_ascii_case(&i.title))))?;
                    let info = params.remove(found);
                    Some((info.clone(), info.title.clone(), pg.name.clone()))
                })
                .collect();
            if !knobs.is_empty() {
                pages.push(ViewPage::new(p, pg.name.clone(), knobs));
            }
        }
        let rest = 128usize.saturating_sub(pages.len() * 8);
        pages.extend(
            params
                .into_iter()
                .take(rest)
                .collect::<Vec<_>>()
                .chunks(8)
                .map(|c| ViewPage::new(p, String::new(), c.iter().map(|i| (i.clone(), i.title.clone(), String::new())).collect())),
        );
        pages
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
    /// Render an audio preview beside a sound for the browser to prehear.
    pub preview: bool,
    /// The file is there: saving again replaces it.
    pub confirm_overwrite: bool,
    /// Why the last Save didn't write it, shown in the dialog.
    pub error: Option<String>,
}

/// A file saved: where, its tags, and whether with its samples.
#[derive(Clone)]
pub struct Saved {
    /// The object saved (its id then): Save writes the file again only for it.
    id: Option<u64>,
    path: PathBuf,
    info: mxprj::sound_info::SoundInfo,
    samples: bool,
    preview: bool,
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
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| deckard_engine::sample::EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
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
    Ok(set_up_output(out?, settings))
}

/// An output just opened, with the settings' routing and timing.
fn set_up_output(out: Output, settings: &settings::Settings) -> Output {
    out.set_routing(&settings.output_routing, settings.cv_mask());
    out.set_sample_accurate(settings.exact_timing);
    out.set_render_ahead(settings.render_ahead_ms as f64 / 1000.0);
    out
}

/// ASIO tried again after it didn't open: this often, this many times.
const ASIO_RETRY_GAP: std::time::Duration = std::time::Duration::from_secs(3);
const ASIO_RETRIES: u32 = 5;

/// The interface input as Maschine's 32 input channels (Preferences ▸ Audio ▸
/// Routing), frame by frame into `live`.
fn open_live_input(
    device: &Option<String>,
    routing: &[Option<usize>],
    live: deckard_engine::LiveIn,
    inputs: std::sync::Arc<input_levels::InputBus>,
    rate: u32,
) -> Result<deckard_engine::devices::InputStream, String> {
    let format = deckard_engine::devices::input_format(device.as_deref())?;
    if format.rate != rate {
        return Err(format!("the input runs at {} Hz and the output at {rate} Hz", format.rate));
    }
    let channels = format.channels;
    let map: Vec<Option<usize>> = (0..32).map(|i| routing.get(i).copied().unwrap_or(Some(i)).filter(|&c| c < channels)).collect();
    // Room up front: on ASIO this callback runs on the output's thread.
    live.lock().unwrap().reserve(2 * 8192);
    deckard_engine::devices::open_input_stream(
        device.as_deref(),
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
}

/// A live Variation's start: (group, pattern, sound), the notes it started from, the
/// notes it made.
type VariationBase = ((usize, usize, usize), Vec<Note>, Vec<Note>);

/// The input settings a live input was opened with: device, routing.
type InputKey = (Option<String>, Vec<Option<usize>>);

/// The Variation and Generate settings (fields of a sound).
const GROOVE_FIELDS: [&str; 28] = [
    "sequence_variation/velocity_amount",
    "sequence_variation/time_shift",
    "sequence_variation/length_scale",
    "sequence_variation/length_variation",
    "sequence_variation/distribution",
    "sequence_variation/live",
    // The Variation page (work-item 35).
    "sequence_variation/mode",
    "sequence_variation/velocity_range/low",
    "sequence_variation/velocity_range/high",
    "sequence_variation/probability",
    "sequence_variation/note_range/low",
    "sequence_variation/note_range/high",
    "sequence_variation/note_count",
    "sequence_variation/steps",
    "sequence_variation/dist/note_count",
    "sequence_variation/dist/notes",
    "sequence_variation/dist/note_length",
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

/// The Variation page's short names (`deckard.variation`) as the sound's fields keep
/// them (work-item 35).
fn variation_field(name: &str) -> Option<&'static str> {
    Some(match name {
        "mode" => "sequence_variation/mode",
        "velocity_lo" => "sequence_variation/velocity_range/low",
        "velocity_hi" => "sequence_variation/velocity_range/high",
        "time_shift" => "sequence_variation/time_shift",
        "probability" => "sequence_generation/probability",
        "note_lo" => "sequence_generation/note_range/low",
        "note_hi" => "sequence_generation/note_range/high",
        "note_count" => "sequence_generation/note_count",
        "steps" => "sequence_variation/steps",
        "dist_note_count" => "sequence_variation/dist/note_count",
        "dist_notes" => "sequence_variation/dist/notes",
        "dist_note_length" => "sequence_variation/dist/note_length",
        // Euclid's own, which the generator keeps under its module.
        "euclid_steps" => "sequence_generation/steps",
        "fills" => "sequence_generation/fills",
        "rotate" => "sequence_generation/rotate",
        "division" => "sequence_generation/division",
        _ => return None,
    })
}

/// The stem separation model's install, offered the first time DECKARD starts without
/// it (the .deb and .rpm can't ask; a download of [`stems::MODEL_MB`] MB).
enum StemsOffer {
    Asking,
    Installing(std::sync::mpsc::Receiver<Result<(), String>>),
}

/// How a start with no project asked for begins (see [`App::start_project`]).
#[derive(Debug, Clone, PartialEq)]
enum Start {
    /// The backup of a session that crashed with unsaved changes.
    Backup,
    /// This project (the last one, as saved).
    Reopen(PathBuf),
    New,
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
    /// Anchor sound in the focused group for Shift+click range selection.
    sound_anchor: Option<usize>,
    scene: usize,
    /// The section picked in the Song view: the song plays from it, or jumps to it
    /// when the playing section ends.
    section: Option<usize>,
    /// The song panel shows the Song view (sections) instead of the Ideas view
    /// (scenes), as Maschine's IDEAS/SONG switch; PLAY follows it.
    song_view: bool,
    /// The Song-mode loop range in ticks; this is transport state, not project data.
    song_loop: Option<(i64, i64)>,
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
    /// The Audio & CPU Monitor window shows.
    audio_monitor: bool,
    /// Cross-platform system/process sampler used by the diagnostics window.
    system_info: sysinfo::System,
    /// Last sampled system diagnostics, held steady between refreshes for readability.
    system_diagnostics: Vec<(&'static str, String)>,
    system_diagnostics_at: Option<std::time::Instant>,
    monitor_view: midi_monitor::View,
    /// File ▸ Export Audio's panel and the export it runs.
    export: export::ExportPanel,
    /// Bounce: its confirmation, and the bounce running.
    bounce: bounce::Bounce,
    /// The clip selected in the Song view (group, clip).
    clip: Option<(usize, usize)>,
    /// When the controller's Settings ▸ Audio was last told the audio interface.
    audio_view_at: Option<std::time::Instant>,
    /// What the own layer's Settings page shows, the settings it was built from (so a
    /// change anywhere rebuilds it), and when its live part was last looked up.
    surface_settings: deckard_surface::SettingsView,
    surface_settings_from: Option<crate::settings::Settings>,
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
    /// The note the Sampling page's PLAY holds (group, sound, pitch), until its STOP.
    audition: Option<(usize, usize, i64)>,
    /// The slice a pad plays (the prehear, which tells no position): when it started
    /// and its frames.
    prehear_from: Option<(std::time::Instant, usize, usize)>,
    /// Scripted pointer input (`--input`): (seconds after start, (m/d/u = move, down,
    /// up, position)).
    input_script: Vec<(f32, (char, egui::Pos2))>,
    /// A drag out of the window took the mouse ([`drag_out`]): the button it never
    /// hears come up is let go for egui.
    drag_release: bool,
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
    /// The browser preview being decoded, if any; the generation drops stale decodes.
    prehear_task: Option<(u64, std::sync::mpsc::Receiver<Result<deckard_engine::sample::Sample, String>>)>,
    prehear_generation: u64,
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
    /// "Create audio preview" was unticked in the Save dialog (for the session).
    preview_declined: bool,
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
    /// Where each chain was left, shown again on coming back to it. `pages_of` is the
    /// chain the fields above were last kept for, `page_key` the slot `plugin_page` is for.
    chain_pages: std::collections::HashMap<deckard_doc::Chain, ChainPages>,
    pages_of: Option<deckard_doc::Chain>,
    page_key: Option<(deckard_doc::Chain, Option<usize>)>,
    /// MIDI learn, host automation and macro editing (the Assignment area).
    remote: remote::Remote,
    /// MIDI ports (inputs, outputs), backed by a PnP/refresh watcher when running.
    midi_ports: (Vec<String>, Vec<String>),
    midi_ports_at: Option<std::time::Instant>,
    midi_port_watch: Option<midi_ports::Watcher>,
    midi_prefs_shown: bool,
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
    /// `--jam`: run the Jam's script, and press `jam_hil_presses` on it, with no Jam plugged
    /// in — how the on-screen overlay and its window are looked at without hardware
    /// (`docs/superpowers/plans/2026-11-21-jam-overlay.md`, "HIL").
    jam_hil: bool,
    jam_hil_presses: Vec<&'static str>,
    /// `--jam-shot`: draw the strip window's content into the app's own viewport, so
    /// `--screenshot` captures what that window shows. A second viewport is not in the main
    /// one's picture, and Screen Recording is not always granted (`hil/strip-window-2026-11-21.md`).
    jam_shot: bool,
    /// The last JAM script error shown.
    jam_error: Option<String>,
    /// Same-frame `View` built by `mk3_events` and handed off to `jam_events` when
    /// nothing changed in between, avoiding building `surface_view()` multiple times/frame.
    frame_surface_view: Option<(SurfaceViewStamp, deckard_surface::View)>,
    /// MIDI controllers run by scripts (the Launchpad Pro); none found in tests.
    midi_devices: midi_devices::MidiDevices,
    /// The last MIDI controller script error shown.
    midi_device_error: Option<String>,
    /// Held while the app runs: macOS doesn't slow it in the background (C46).
    _running: Option<keep_running::Activity>,
    /// Held while the song plays or something records: the computer stays awake.
    awake: keep_running::StayAwake,
    /// Knob travel the controller's knobs have built up toward a stepped value's next step.
    knob_acc: [f64; 8],
    /// When each of the controller's knobs was last turned (a route's whole name shows
    /// meanwhile).
    knob_turned: [Option<std::time::Instant>; 8],
    /// DECKARD's own MK3 layer, run when no controller scripts do.
    surface: deckard_surface::Surface,
    /// Its last script error, as shown in the status bar.
    surface_error: Option<String>,
    /// Whether the MK3 showed its Browse page last frame (leaving it stops prehear).
    surface_was_browsing: bool,
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
    live_input: Option<(deckard_engine::devices::InputStream, InputKey)>,
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
    /// A stepped plug-in parameter's names as last read (by instance, id and steps):
    /// [`App::knob_choices`] reads them with `try_lock`, and a frame that finds the
    /// plug-in busy shows these rather than dropping the list for a dial.
    step_names: std::cell::RefCell<std::collections::HashMap<(usize, u32, i32), Vec<String>>>,
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
    /// ASIO wanted but not playing (often just after the computer wakes): when it was
    /// last tried again, and how many times since the output was opened.
    asio_retry: Option<(std::time::Instant, u32)>,
    /// The stem separation model offered at start-up: the question, then the install.
    stems_offer: Option<StemsOffer>,
    /// TYPE: a mode asked for the sound (group, sound) that waits for its
    /// confirmation (the change loses the sound's changes).
    pub(crate) sampler_ask: Option<((usize, usize), deckard_model::new_sampler::SamplerMode)>,
    /// In Layers, a take was picked on the sound (group, sound): EDIT works on it while
    /// it is on no layer ([`App::shown_take`]).
    pub(crate) take_shown: Option<(usize, usize)>,
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
    /// The playhead was put in the pattern (clicked while stopped) and nothing has
    /// played since: ⌘V pastes there.
    placed_head: bool,
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
    /// The notes that have just started, sequenced or played, (group, sound, pitch): the
    /// controller's pads flash white for them.
    pub hits: std::collections::HashSet<(usize, usize, i64)>,
    /// The (group, sound)s actively playing a loop in the engine now.
    pub loop_lit: std::collections::HashSet<(usize, usize)>,
    /// Live note pitches currently held per `(group, sound)`.
    held_pitches: std::collections::HashMap<(usize, usize), Vec<i64>>,
    /// Sounds copied or cut from the sound menu, to paste.
    sound_clipboard: Vec<deckard_doc::SoundClip>,
    /// A copied group, as its group file.
    plugin_clipboard: Option<deckard_doc::CopiedSlot>,
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
    /// The New Sampler's zones being analysed, and the document revision they were
    /// last looked for at.
    analysis: analysis::Jobs,
    analysis_revision: Option<u64>,
    /// The New Sampler's Sync and Keys renders being made.
    renders: render_jobs::Renders,
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
    /// Records the master output to a WAV file (File ▸ Session Record).
    session: session_record::SessionRecorder,
    /// The live session Recorder window is shown.
    session_window_open: bool,
    /// Whether the Recorder window floats above other windows (on by default).
    session_window_pin: session_window::Pin,
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
    /// Encodes and writes the backup off the UI thread (see [`App::keep_session`]).
    session_keeper: session_backup::Keeper,
    /// The backup there is marked ended, now AUTO BACKUP is off.
    session_retired: bool,
    /// Goes up with each project opened or made.
    doc_generation: u64,
    /// Cached document summary for `surface_view` and `layout::chain_area`.
    surface_doc_cache: std::cell::RefCell<SurfaceDocCache>,
    /// The focused Zhora (group, sound, slot) as last shown, while the audio thread has it.
    zhora_shown: std::cell::RefCell<Option<((usize, usize, usize), deckard_surface::ZhoraView)>>,
    /// The focused Tuner effect (chain, slot) as last shown, while the audio thread has it.
    tuner_shown: std::cell::RefCell<Option<((deckard_doc::Chain, usize), deckard_surface::TunerView)>>,
    /// The focused synth's scope (chain, slot) as last shown, while the audio thread has it.
    scope_shown: std::cell::RefCell<Option<((deckard_doc::Chain, usize), deckard_surface::ScopeView)>>,
    /// A hosted plug-in's live output as its scope shows it: the slot, the instance tapped
    /// (a reload makes a new one) and the tap's reader, and the picture last made.
    live_scope: std::cell::RefCell<Option<zhora_ui::LiveScope>>,
    /// The preview last rendered of an internal synth, with the module and the parameters
    /// it was rendered from: rendered again only when they change.
    internal_scope: std::cell::RefCell<Option<(String, Vec<(String, f64)>, deckard_surface::ScopeView)>>,
    plugin_spectrum: std::cell::RefCell<Option<plugin_spectrum::Focus>>,
    /// When the Zhoras were last looked at for a finished Tune.
    tunes_watched: Option<std::time::Instant>,
}

#[derive(Default)]
pub(crate) struct SurfaceDocCache {
    key: Option<(u64, u64)>,
    pub(crate) macro_targets: std::collections::HashSet<u64>,
    group_noted: Vec<std::collections::HashSet<usize>>,
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

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

fn format_duration(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    let days = seconds / 86_400;
    let hours = seconds / 3_600 % 24;
    let minutes = seconds / 60 % 60;
    let seconds = seconds % 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else {
        format!("{minutes}m {seconds}s")
    }
}

pub(crate) fn save_png(path: &Path, image: &egui::ColorImage) -> Result<(), String> {
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
pub(crate) const BACKGROUND_PACE: std::time::Duration = std::time::Duration::from_millis(16);

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
        self.sound_selection.clear();
        self.sound_anchor = None;
        self.zone_of = None;
        self.lock_morph = None;
        self.lock_active = false;
        self.looping = false;
        self.song_loop = None;
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
                let ctl = controller::Controller::new(&dir, &self.model, Some(doc.clone()));
                if let Ok(c) = &ctl {
                    c.set_pinned(&self.settings.pinned_pages);
                }
                self.controller = Some(ctl);
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
            self.sound_selection.clear();
            self.sound_anchor = None;
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
                    self.saved.remove(&deckard_doc::FileItem::Sound(g, s));
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
            Act::PasteSound(g, s) => doc.borrow_mut().paste_sounds(g, s, &self.sound_clipboard).map(|_| {
                for sound in s..s + self.sound_clipboard.len() {
                    self.saved.remove(&deckard_doc::FileItem::Sound(g, sound));
                }
            }),
            Act::CopyGroup(g) => self.copy_group(g),
            Act::CutGroup(g) => self.copy_group(g).and_then(|_| {
                self.delete_group(g)?;
                doc.borrow_mut().relabel_last("Cut Group");
                Ok(())
            }),
            Act::PasteGroup(g) => match &self.group_clipboard {
                Some(bytes) => {
                    let mut d = doc.borrow_mut();
                    d.load_group_file(g, bytes).map(|_| {
                        d.relabel_last("Paste Group");
                        self.forget_group_presets(g);
                    })
                }
                None => Ok(()),
            },
            Act::DeleteGroup(g) => self.delete_group(g),
            Act::MoveGroup(g, to) => self.move_group(g, to),
            Act::ResetSound(g, s) => {
                let count = doc.borrow().project().groups.get(g).map_or(0, |gr| gr.sounds.len());
                if let Some(sounds) = self.selected_sounds_for(g, s, count) {
                    let r = doc.borrow_mut().transaction("Reset Sounds", |d| {
                        for &si in &sounds {
                            d.reset_sound(g, si)?;
                        }
                        Ok(())
                    });
                    if r.is_ok() {
                        for &si in &sounds {
                            self.saved.remove(&deckard_doc::FileItem::Sound(g, si));
                        }
                        self.samples.plugins.retain(|slot| {
                            !matches!(
                                slot,
                                deckard_plugins::Slot::Sound { group, sound }
                                    | deckard_plugins::Slot::Chain { group, sound, .. }
                                    if *group == g && sounds.contains(sound)
                            )
                        });
                    }
                    r
                } else {
                    let r = doc.borrow_mut().reset_sound(g, s);
                    if r.is_ok() {
                        self.saved.remove(&deckard_doc::FileItem::Sound(g, s));
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
                if r.is_ok() {
                    if self.group == g && self.pattern >= p {
                        self.pattern = self.pattern.saturating_sub(1);
                    }
                    if let Some(grow) = &mut self.grow
                        && grow.group == g
                    {
                        if grow.pattern == p {
                            self.grow = None;
                        } else if grow.pattern > p {
                            grow.pattern -= 1;
                        }
                    }
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
        let was_last = doc.borrow().project().groups.len() == 1;
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
        if let Some(grow) = &mut self.grow {
            if was_last || grow.group == g {
                self.grow = None;
            } else if grow.group > g {
                grow.group -= 1;
            }
        }
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

    /// [`App::keep_session`], `clean` once DECKARD quits as it should. The UI thread only
    /// takes a snapshot; the encoding and the writing are the keeper's. With AUTO BACKUP
    /// off, only where the project is saved is kept (on quitting), not its contents.
    fn keep_session_as(&mut self, clean: bool) {
        let Some(dir) = self.session_dir.clone() else { return };
        self.store_screen();
        let Some(doc) = self.doc.clone() else { return };
        let d = doc.borrow();
        self.session_kept = (self.doc_generation, d.revision());
        self.session_due = None;
        let mut info = format!("dirty = {}\nclean = {clean}\n", d.is_dirty());
        if let Some(p) = d.path() {
            info += &format!("path = {}\n", p.display());
        }
        let job = if self.settings.auto_backup {
            session_backup::Job::Keep { dir, snapshot: d.snapshot(), info }
        } else {
            session_backup::Job::Info { dir, info }
        };
        drop(d);
        self.session_keeper.submit(job);
        if clean {
            self.session_keeper.finish();
        }
    }

    /// Wait for the backup being written in the background, if there is one.
    fn session_written(&mut self) {
        self.session_keeper.finish();
    }

    /// Keep the session a few seconds after the last edit (unless the Settings page's
    /// AUTO BACKUP is off: then only on quitting).
    fn keep_session_soon(&mut self) {
        let Some(doc) = &self.doc else { return };
        if !self.settings.auto_backup {
            // Off: no backup is taken, and the one there (from before) is not to come
            // back after a crash as if it were this session's.
            if !self.session_retired
                && let Some(dir) = self.session_dir.clone()
            {
                self.session_retired = true;
                self.session_due = None;
                self.session_kept = (u64::MAX, 0);
                self.session_keeper.submit(session_backup::Job::Retire { dir });
            }
            return;
        }
        self.session_retired = false;
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
    pub(crate) fn start_project(&mut self) -> Start {
        let (path, backup) = self.last_session();
        if backup && self.restore_session(path.clone(), true) {
            self.status = "unsaved changes brought back from the backup".into();
            return Start::Backup;
        }
        if let Some(p) = self.reopens(path) {
            self.open(&p);
            if self.error.is_none() {
                self.remember(&p);
                return Start::Reopen(p);
            }
        }
        self.new_project();
        Start::New
    }

    /// What [`App::start_project`] will do, as far as can be told without doing it.
    fn start_plan(&self) -> Start {
        let (path, backup) = self.last_session();
        if backup && self.session_dir.as_ref().is_some_and(|d| d.join("session.mxprj").exists()) {
            return Start::Backup;
        }
        self.reopens(path).map_or(Start::New, Start::Reopen)
    }

    /// The last session's project file, and whether it ended in a crash with unsaved
    /// changes (so its backup is to be brought back).
    fn last_session(&self) -> (Option<PathBuf>, bool) {
        let info = self.session_dir.as_ref().and_then(|d| std::fs::read_to_string(d.join("session.txt")).ok()).unwrap_or_default();
        let value = |k: &str| info.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix(" = ").map(str::to_string));
        let (dirty, clean) = (value("dirty").as_deref() == Some("true"), value("clean").as_deref() == Some("true"));
        (value("path").map(PathBuf::from), dirty && !clean)
    }

    /// The last project, if it is to be opened again (the Settings page's RELOAD).
    fn reopens(&self, path: Option<PathBuf>) -> Option<PathBuf> {
        path.filter(|p| self.settings.reload_last && p.exists())
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
            C::AudioMonitor => self.audio_monitor = !self.audio_monitor,
            C::ExportAudio => self.export.open = true,
            C::ExportMidi => {
                if let Err(e) = self.export_song_midi(None) {
                    self.status = format!("can't export MIDI: {e}");
                }
            }
            C::SessionRecord => self.toggle_session_record(),
            C::SessionRecorderWindow => self.session_window_open = !self.session_window_open,
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
        s.set_pinned(self.settings.pinned_pages.clone(), self.settings.pinned_open.as_deref());
        s.set_lcd(&self.settings.lcd_mode);
        s.set_theme(&self.settings.controller_theme);
        plugin_pictures::give(&mut s);
        s
    }

    pub(crate) fn sync_surface_pins(&mut self) {
        if self.surface.pins_match(&self.settings.pinned_pages, self.settings.pinned_open.as_deref()) {
            return;
        }
        self.settings.pinned_pages = self.surface.pinned();
        self.settings.pinned_open = self.surface.pinned_open();
        #[cfg(not(test))]
        self.settings.save();
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
            let ctl = controller::Controller::new(&dir, &self.model, self.doc.clone());
            if let Ok(c) = &ctl {
                c.set_pinned(&self.settings.pinned_pages);
            }
            self.controller = Some(ctl);
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
                    ui.add(egui::Label::new(egui::RichText::new(format!("The interface uses {got} samples.")).weak()).wrap());
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
            ui.add(egui::Label::new(egui::RichText::new(hint).weak()).wrap());
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
        ui.add(egui::Label::new(egui::RichText::new(
                "Larger buffers give heavy projects and plug-ins more room (fewer clicks); smaller ones make pads and keys respond sooner. \
                 \"Late\" blocks were heard as clicks.",
            )
            .weak()).wrap());
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
                ui.add(egui::Label::new(egui::RichText::new(
                        "The driver has the last word on the buffer size and the rate. If it won't take the size below, set it in its own control panel: DECKARD follows.",
                    )
                    .weak()).wrap());
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
        // Tearing down the `Output` drops the session recording's master tap. A device or
        // buffer change at the SAME sample rate fires no RateChange, so clean-stop the
        // session here (before the old `Output` is dropped) rather than let it pad pure
        // silence for the rest of the set. `set_tap(None)` is moot: the `Output` is going.
        if self.session.active() {
            self.session.abort(session_record::StopReason::DeviceChange);
            // The window stays open so it can show the "audio device changed" reason.
        }
        self.output = None;
        self.midi_wired = false;
        self.live_input = None;
        self.asio_retry = None;
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

    /// ASIO wanted but the system's driver standing in (or nothing playing): the driver
    /// often isn't ready yet when the computer wakes, so try it again a few times, a few
    /// seconds apart, keeping what plays until it opens. Not while the song plays or
    /// anything records, which the switch would stop. An interface that stays away is
    /// left alone after that (loading its driver holds the screen up a moment) until the
    /// output is opened again.
    fn retry_asio(&mut self) {
        use deckard_engine::devices::{self, ASIO_BUILT, AudioDriver};
        let missing = ASIO_BUILT
            && self.settings.audio_driver == AudioDriver::Asio
            && self.settings.asio_device.is_some()
            && self.output.is_some()
            && self.output_fallback.is_some();
        if !missing {
            self.asio_retry = None;
            return;
        }
        let now = std::time::Instant::now();
        let (last, tries) = *self.asio_retry.get_or_insert((now, 0));
        if tries >= ASIO_RETRIES
            || now.duration_since(last) < ASIO_RETRY_GAP
            || self.asio_reopen_due
            || self.playing.is_some()
            || self.recorder.busy()
            || self.session.active()
        {
            return;
        }
        self.asio_retry = Some((now, tries + 1));
        devices::set_driver(AudioDriver::Asio);
        match Output::with_device(self.settings.asio_device.as_deref(), self.settings.buffer_frames, self.settings.multicore) {
            Ok(out) => {
                // close_output lets the recorder's input go too, as ASIO is the driver now.
                self.close_output();
                self.output = Some(Ok(set_up_output(out, &self.settings)));
                self.output_fallback = None;
                self.asio_retry = None;
                self.open_output_now();
            }
            Err(_) => devices::set_driver(AudioDriver::System),
        }
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
            ui.add(egui::Label::new(egui::RichText::new("Pick the ASIO driver of your interface.").weak()).wrap());
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
            ui.strong("Session recording").on_hover_text("File ▸ Record Session to File… records the master output to a WAV.");
            ui.label("Keep free");
            let mut mb = self.settings.session_reserve_mb;
            let drag = egui::DragValue::new(&mut mb).range(0..=1_000_000).speed(64).suffix(" MB");
            if ui.add(drag).changed() {
                self.settings.session_reserve_mb = mb;
                self.settings.save();
            }
            ui.label(egui::RichText::new("the recording auto-stops before the drive drops below this (1 GB = 1024 MB)").weak());
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Routing");
            ui.selectable_value(&mut self.routing_outputs, false, "Input");
            ui.selectable_value(&mut self.routing_outputs, true, "Output");
        });
        let outs = self.routing_outputs;
        ui.add(egui::Label::new(egui::RichText::new(if outs {
                "Which channel of the interface each of DECKARD's outputs plays on. An alias names it wherever it's offered; one not connected isn't offered."
            } else {
                "Which channel of the interface each of DECKARD's inputs reads. An alias names it wherever it's offered; one not connected isn't offered."
            })
            .weak()).wrap());
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
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Ports").strong());
            if ui.button("Refresh").on_hover_text("Scan MIDI ports again (for software ports like loopMIDI).").clicked() {
                self.refresh_midi_ports();
            }
        });
        let (ins, outs) = self.midi_ports().clone();
        for (title, ports, prefix) in [("Inputs", &ins, "in:"), ("Outputs", &outs, "out:")] {
            ui.label(egui::RichText::new(title).strong());
            if ports.is_empty() {
                ui.add(egui::Label::new(egui::RichText::new("No MIDI ports found").weak()).wrap());
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
        ui.add(egui::Label::new(egui::RichText::new(self.midi.status()).weak()).wrap());
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
            ui.add(
                egui::Label::new(
                    egui::RichText::new("File ▸ New starts from it: its groups, sounds, plug-ins, tempo and routing, untitled.").weak(),
                )
                .wrap(),
            );
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
            ui.add(
                egui::Label::new(
                    egui::RichText::new(if cfg!(windows) {
                        "The window and the taskbar show it. Shortcuts keep the icon they were installed with."
                    } else if cfg!(target_os = "macos") {
                        "The Dock shows it while DECKARD runs; Finder keeps the app's own."
                    } else {
                        "The window and the taskbar show it."
                    })
                    .weak(),
                )
                .wrap(),
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
        let (fit, wide) = (crate::layout::dialog_max_height(ctx), crate::layout::dialog_max_width(ctx));
        egui::Window::new("Preferences")
            .collapsible(false)
            .resizable(true)
            .default_size([760.0f32.min(wide), 700.0f32.min(fit)])
            .max_height(fit)
            .max_width(wide)
            .open(&mut open)
            .show(ctx, |ui| {
            crate::layout::dialog_style(ui);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut tab, 7, "General");
                ui.selectable_value(&mut tab, 2, "Audio");
                ui.selectable_value(&mut tab, 3, "MIDI");
                ui.selectable_value(&mut tab, 4, "Hardware");
                ui.selectable_value(&mut tab, 5, "Library");
                ui.selectable_value(&mut tab, 6, "Recording");
                ui.separator();
                ui.add(egui::Label::new(egui::RichText::new("Plug-ins:").weak()).wrap());
                ui.selectable_value(&mut tab, 0, "Manager");
                ui.selectable_value(&mut tab, 1, "Location");
            });
            ui.separator();
            // Past the window's height the tab scrolls (a short screen). The Plug-in Manager's
            // list scrolls by itself in the room left under its settings, and at least
            // `LIST_MIN` (then the tab scrolls too: on Linux the Wine settings alone can be
            // taller than a short screen).
            const LIST_MIN: f32 = 160.0;
            let view_h = ui.available_height();
            egui::ScrollArea::vertical().id_salt(("prefs-tab", tab)).auto_shrink([false, false]).show(ui, |ui| {
            let top = ui.cursor().top();
            if tab == 2 {
                self.audio_preferences(ui);
            } else if tab == 7 {
                self.general_preferences(ui);
            } else if tab == 3 {
                if !self.midi_prefs_shown {
                    self.refresh_midi_ports();
                }
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
                    ui.add(egui::Label::new(egui::RichText::new("REC from a stop counts in this long first (SHIFT + REC on the controller: at least a bar).")
                            .weak()).wrap());
                    ui.end_row();
                    ui.label("Input Quantize");
                    ui.checkbox(&mut self.settings.input_quantize, "Snap recorded notes to the grid");
                    ui.end_row();
                });
            } else if tab == 5 {
                ui.label("Folders of sound packs to list in the browser under Factory (each folder's name is its library).");
                ui.add(egui::Label::new(egui::RichText::new("Maschine's own library is listed when it's installed; add packs here when it isn't.").weak()).wrap());
                let mut remove = None;
                for d in &self.settings.library_folders {
                    ui.horizontal(|ui| {
                        // The font has no "✕", so that character drew as an empty box.
                        let response = ui.add_sized([18.0, 18.0], egui::Button::new("")).on_hover_text("Remove this folder");
                        let r = response.rect.shrink(5.0);
                        let color = if response.hovered() {
                            ui.visuals().widgets.hovered.fg_stroke.color
                        } else {
                            ui.visuals().text_color()
                        };
                        let stroke = egui::Stroke::new(1.5, color);
                        ui.painter().line_segment([r.left_top(), r.right_bottom()], stroke);
                        ui.painter().line_segment([r.right_top(), r.left_bottom()], stroke);
                        if response.clicked() {
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
                    ui.add(egui::Label::new(egui::RichText::new("How hard the MK3's pads must be pressed: higher, gentle presses play louder. The MK3 applies it, as with Maschine's Hardware > Pads.")
                            .weak()).wrap());
                    ui.end_row();
                    // Velocity SCALING, as the MK3's own Settings page has it (Soft 3 … Hard 3):
                    // Maschine's measured curves (`mk3_io::pad_velocity`).
                    ui.label("Velocity scaling");
                    let mut scaling = self.settings.velocity_scaling.clamp(-3, 3);
                    let names = surface_settings::VELOCITY_SCALINGS;
                    egui::ComboBox::from_id_salt("velocity-scaling").selected_text(names[(scaling + 3) as usize]).show_ui(ui, |ui| {
                        for (s, name) in (-3i8..=3).zip(names) {
                            ui.selectable_value(&mut scaling, s, name);
                        }
                    });
                    self.settings.velocity_scaling = scaling;
                    ui.end_row();
                    ui.label("");
                    ui.add(egui::Label::new(egui::RichText::new("The curve from pad pressure to velocity: Soft plays gentle hits louder, Hard needs harder ones. Maschine's curves, as in its Hardware > Pads.")
                            .weak()).wrap());
                    ui.end_row();
                    // The two brightnesses are the MK3's own (its LEDs' level report, its
                    // displays' backlight byte), so they follow the model picked here rather
                    // than the cable: another controller has different lights
                    // (`settings::sets_brightness`).
                    if settings::sets_brightness(&self.model) {
                        ui.label("LED brightness");
                        let mut leds = self.settings.led_brightness;
                        ui.add(egui::Slider::new(&mut leds, 0..=100).suffix(" %"));
                        self.settings.led_brightness = leds;
                        ui.end_row();
                        ui.label("");
                        ui.add(egui::Label::new(egui::RichText::new("Every LED on the MK3, one level for the whole panel: Maschine's Hardware > LEDs.").weak()).wrap());
                        ui.end_row();
                        ui.label("Display brightness");
                        let mut displays = self.settings.display_brightness;
                        ui.add(egui::Slider::new(&mut displays, 0..=100).suffix(" %"));
                        self.settings.display_brightness = displays;
                        ui.end_row();
                        ui.label("");
                        ui.add(egui::Label::new(egui::RichText::new("The MK3's two screens: Maschine's Hardware > Display.").weak()).wrap());
                        ui.end_row();
                        if self.mk3.is_none() {
                            ui.label("");
                            ui.add(egui::Label::new(egui::RichText::new("Applies when the MK3 connects.").weak()).wrap());
                            ui.end_row();
                        }
                    }
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
                        ui.add(egui::Label::new(egui::RichText::new("Windows screen driver lets DECKARD draw the screens. Maschine screen driver clears them and gives them back to Maschine.").weak()).wrap());
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
                    ui.label("Desktop theme").on_hover_text("The software window. It does not change the hardware.");
                    let mut desktop = self.settings.desktop_theme.clone();
                    egui::ComboBox::from_id_salt("desktop-theme").selected_text(theme_name(&desktop)).show_ui(ui, |ui| {
                        for t in self.surface.themes() {
                            // Stock palettes are hardware themes. The window only has a look
                            // for the hand-written themes, so the rest would be a no-op here.
                            if deckard_surface::theme_file::source(&t).is_some() && t != desktop {
                                continue;
                            }
                            let name = theme_name(&t);
                            ui.selectable_value(&mut desktop, t, name);
                        }
                    });
                    if desktop != self.settings.desktop_theme {
                        self.settings.desktop_theme = desktop;
                        self.settings.desktop_background = None;
                        self.settings.desktop_text = None;
                    }
                    ui.end_row();
                    ui.label("Hardware theme").on_hover_text("The Maschine's screens, pads and groups.");
                    let mut theme = self.settings.controller_theme.clone();
                    crate::layout::combo("controller-theme").selected_text(theme_name(&theme)).show_ui(ui, |ui| {
                        for t in self.surface.themes() {
                            let name = theme_name(&t);
                            ui.selectable_value(&mut theme, t, name);
                        }
                    });
                    if theme != self.settings.controller_theme {
                        self.surface.set_lcd(&self.settings.lcd_mode);
                        self.surface.set_theme(&theme);
                        self.settings.controller_theme = self.surface.theme().to_string();
                    }
                    ui.end_row();
                    if self.settings.controller_theme == "lcd" || self.settings.desktop_theme == "lcd" {
                        ui.label("LCD colour");
                        let modes = [("yellow", "Yellow / black"), ("red", "Red / black"), ("blue", "Blue / white")];
                        let mut mode = self.settings.lcd_mode.clone();
                        let shown = modes.iter().find(|(id, _)| *id == mode).map(|(_, name)| *name).unwrap_or("Yellow / black");
                        egui::ComboBox::from_id_salt("lcd-mode").selected_text(shown).show_ui(ui, |ui| {
                            for (id, name) in modes {
                                ui.selectable_value(&mut mode, id.to_string(), name);
                            }
                        });
                        if mode != self.settings.lcd_mode {
                            self.settings.lcd_mode = deckard_surface::lcd::mode(&mode).to_string();
                            self.surface.set_lcd(&self.settings.lcd_mode);
                        }
                        ui.end_row();
                    }
                    theme_editor::show(ui, self);
                    ui.label("Maschine JAM");
                    ui.checkbox(&mut self.settings.relative_strips, "Relative level touchstrips").on_hover_text(
                        "On the mixer strips (LEVEL, AUX, SWING, TUNE, MACRO), a touch no longer sets the value where the finger lands, and a double tap no longer resets it: moving the finger changes the value by the distance moved, from where it was. CONTROL mode keeps absolute-fader behaviour. Off: a touch sets the value.",
                    );
                    ui.end_row();
                    ui.label("");
                    if ui
                        .checkbox(&mut self.settings.show_overlay, "Show the on-screen overlay")
                        .on_hover_text(
                            "The Jam's on-screen overlay — the page its script publishes (the OSO) — drawn in its own small window over the app. Off: the pages still run, so the encoders, the lights and every page behave as they ever did, but nothing is drawn. Switching it puts the window back beside the app. (The Jam's touchstrips are unaffected.)",
                        )
                        .changed()
                    {
                        self.settings.jam_strip_pos = None;
                    }
                    ui.end_row();
                    ui.label("");
                    let found = if self.scripts.is_some() { "found" } else { "not found" };
                    ui.add(egui::Label::new(egui::RichText::new(format!(
                            "What runs the MK3: Maschine's controller scripts, from your own Maschine install ({found}), or DECKARD's own layer. Automatic uses the scripts when they're found."
                        ))
                        .weak()).wrap());
                    ui.end_row();
                });
            } else if tab == 0 {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.settings.use_vst3, "Use VST3 Plug-ins");
                    ui.checkbox(&mut self.settings.use_vst2, "Use VST2 Plug-ins");
                    ui.checkbox(&mut self.settings.use_au, "Use Audio Units");
                });
                ui.checkbox(&mut self.settings.scan_on_start, "Rescan plug-ins at start-up");
                ui.add(egui::Label::new(egui::RichText::new("VST2 plug-ins are listed but can't be hosted yet.").weak()).wrap());
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.catalog.scanning(), egui::Button::new("Rescan")).clicked() {
                        rescan = true;
                    }
                    ui.add(egui::Label::new(egui::RichText::new(&self.catalog.status).weak()).wrap());
                });
                if cfg!(target_os = "linux") {
                    self.wine_section(ui, &mut wine_action);
                }
                ui.separator();
                let mut entries: Vec<deckard_plugins::scan::Entry> = self.catalog.entries.clone();
                entries.sort_by(|a, b| {
                    (a.vendor.to_lowercase(), a.name.to_lowercase()).cmp(&(b.vendor.to_lowercase(), b.name.to_lowercase()))
                });
                let list_h = (view_h - (ui.cursor().top() - top)).max(LIST_MIN);
                egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
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
        self.midi_prefs_shown = open && tab == 3;
        if !open {
            // Looked at again when the window next opens.
            self.wine_status = None;
        }
        if let Some(a) = wine_action {
            self.start_wine(a);
        }
    }

    /// TYPE's question: a mode change keeps the sample shown and loses the rest.
    fn sampler_mode_dialog(&mut self, ctx: &egui::Context) {
        if self.sampler_ask.is_none() {
            return;
        }
        let mut answer = None;
        egui::Modal::new(egui::Id::new("sampler-mode")).show(ctx, |ui| {
            crate::layout::fit_dialog(ui, |ui| {
                crate::layout::dialog_style(ui);
                ui.set_width(380.0);
                ui.label(egui::RichText::new("Changing the mode loses all changes to this sound.").strong());
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        answer = Some(false);
                    }
                });
            })
        });
        if let Some(ok) = answer {
            self.answer_sampler_mode(ok);
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
                    crate::layout::fit_dialog(ui, |ui| {
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
                    })
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
                        crate::layout::fit_dialog(ui, |ui| {
                            crate::layout::dialog_style(ui);
                            ui.set_width(420.0);
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(format!("Downloading the stem separation model ({} MB)…", stems::MODEL_MB));
                            });
                        })
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
                crate::layout::fit_dialog(ui, |ui| {
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
                })
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
                crate::layout::fit_dialog(ui, |ui| {
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
                })
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
        self.sampler_mode_dialog(ctx);
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
        self.audio_monitor(ctx);
        if self.about.is_some() && !about::show(self, ctx) {
            self.about = None;
        }
    }

    fn audio_monitor(&mut self, ctx: &egui::Context) {
        if !self.audio_monitor {
            return;
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.audio_monitor = false;
            return;
        }
        let mut open = true;
        let monitor_size = ctx.input(|input| input.viewport().monitor_size).unwrap_or(egui::vec2(900.0, 700.0));
        let window_size = egui::vec2(420.0, (monitor_size.y - 48.0).max(300.0));
        egui::Window::new("Audio & CPU Monitor")
            .title_bar(false)
            .open(&mut open)
            .resizable(true)
            .default_size(window_size)
            .max_size(window_size)
            .show(ctx, |ui| {
                crate::layout::dialog_style(ui);
                let available_height = ui.available_height();
                egui::ScrollArea::vertical().auto_shrink([true, false]).max_height(available_height).show(ui, |ui| {
                    if let Some(Ok(out)) = &self.output {
                        use std::sync::atomic::Ordering::Relaxed;
                        let stats = &out.stats;
                        let value = |a: &std::sync::atomic::AtomicU64| a.load(Relaxed);
                        ui.heading("Audio device");
                        egui::Grid::new("audio-monitor-device").num_columns(2).show(ui, |ui| {
                            ui.label("Driver");
                            ui.label(out.driver.name());
                            ui.end_row();
                            ui.label("Device");
                            ui.label(&out.device);
                            ui.end_row();
                            ui.label("Sample rate");
                            ui.label(format!("{} Hz", out.rate));
                            ui.end_row();
                            ui.label("Buffer");
                            ui.label(format!(
                                "{} samples ({:.1} ms)",
                                out.block_frames(),
                                out.block_frames() as f64 / out.rate as f64 * 1000.0
                            ));
                            ui.end_row();
                            ui.label("Reported latency");
                            ui.label(format!("{:.1} ms", out.latency().as_secs_f64() * 1000.0));
                            ui.end_row();
                        });
                        ui.separator();
                        ui.heading("Audio thread");
                        egui::Grid::new("audio-monitor-stats").num_columns(2).show(ui, |ui| {
                            ui.label("Current load");
                            ui.label(format!("{:.1} %", self.cpu_load * 100.0));
                            ui.end_row();
                            ui.label("Peak load");
                            ui.label(format!("{:.1} %", value(&stats.peak_load) as f64 / 10.0));
                            ui.end_row();
                            ui.label("Callbacks");
                            ui.label(value(&stats.callbacks).to_string());
                            ui.end_row();
                            ui.label("Late callbacks");
                            ui.label(value(&stats.late).to_string());
                            ui.end_row();
                            ui.label("Missed blocks");
                            ui.label(value(&stats.missed).to_string());
                            ui.end_row();
                            ui.label("Device errors / xruns");
                            ui.label(value(&stats.device_errors).to_string());
                            ui.end_row();
                            ui.label("Queue-full events");
                            ui.label(value(&stats.queue_full).to_string());
                            ui.end_row();
                        });
                        ui.separator();
                        ui.heading("Render-ahead");
                        let (ahead, underruns) = out.telemetry(|t| (t.sounds_ahead, t.ahead_underruns));
                        ui.label(format!("{ahead} sounds currently ahead; {underruns} not-ready events"));
                        if let Some((threads, helped)) = out.render_threads() {
                            ui.label(format!("{threads} render threads; {helped} worker-rendered jobs"));
                        } else {
                            ui.label("Render-ahead workers: off");
                        }
                    } else {
                        ui.label("Audio output is not open.");
                        ui.label("Start playback or open a browser preview to initialize it.");
                    }
                    ui.separator();
                    ui.heading("System");
                    for (label, value) in self.system_diagnostics() {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            ui.label(value);
                        });
                    }
                });
            });
        self.audio_monitor = open;
        if self.audio_monitor {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn system_diagnostics(&mut self) -> Vec<(&'static str, String)> {
        const SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
        if self.system_diagnostics_at.is_some_and(|at| at.elapsed() < SAMPLE_INTERVAL) {
            return self.system_diagnostics.clone();
        }
        use sysinfo::{Pid, ProcessesToUpdate, System};
        self.system_info.refresh_cpu_usage();
        self.system_info.refresh_memory();
        let pid = Pid::from_u32(std::process::id());
        self.system_info.refresh_processes(ProcessesToUpdate::Some(&[pid]), false);
        let mut diagnostics = Vec::new();
        diagnostics.push(("System CPU", format!("{:.1} %", self.system_info.global_cpu_usage())));
        diagnostics.push(("Logical CPUs", self.system_info.cpus().len().to_string()));
        diagnostics.push(("Physical cores", System::physical_core_count().map_or_else(|| "unavailable".into(), |count| count.to_string())));
        if let Some(process) = self.system_info.process(pid) {
            diagnostics.push(("DECKARD CPU", format!("{:.1} %", process.cpu_usage())));
            diagnostics.push(("Process memory", format_bytes(process.memory())));
            diagnostics.push(("Virtual address space", format_bytes(process.virtual_memory())));
            diagnostics.push(("Process threads", process.tasks().map_or_else(|| "unavailable".into(), |tasks| tasks.len().to_string())));
        } else {
            diagnostics.push(("DECKARD CPU", "unavailable".into()));
            diagnostics.push(("Process memory", "unavailable".into()));
            diagnostics.push(("Virtual address space", "unavailable".into()));
            diagnostics.push(("Process threads", "unavailable".into()));
        }
        diagnostics.push((
            "System memory",
            format!("{} used / {} total", format_bytes(self.system_info.used_memory()), format_bytes(self.system_info.total_memory())),
        ));
        diagnostics.push(("Available memory", format_bytes(self.system_info.available_memory())));
        diagnostics.push((
            "Swap",
            format!("{} used / {} total", format_bytes(self.system_info.used_swap()), format_bytes(self.system_info.total_swap())),
        ));
        diagnostics.push(("Uptime", format_duration(std::time::Duration::from_secs(System::uptime()))));

        #[cfg(target_os = "linux")]
        {
            let load = std::fs::read_to_string("/proc/loadavg").ok().map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "));
            diagnostics.push(("Load average", load.unwrap_or_else(|| "unavailable".into())));
        }
        self.system_diagnostics = diagnostics;
        self.system_diagnostics_at = Some(std::time::Instant::now());
        self.system_diagnostics.clone()
    }

    /// Play `mode`; the arrangement from the focused section.
    fn play(&mut self, mode: Mode) {
        let start = if mode == Mode::Arrangement && self.looping {
            self.song_loop.map_or(0, |(start, _)| start)
        } else {
            match (mode, self.section) {
                (Mode::Arrangement, Some(i)) => self.doc.as_ref().map_or(0, |d| d.borrow().project().section_start(i)),
                _ => 0,
            }
        };
        self.play_from(mode, start);
    }

    /// Play `mode` from tick `start`.
    fn play_from(&mut self, mode: Mode, start: i64) {
        if self.recording && start > 0 {
            self.auto_grow_at(mode, start as f64);
        }
        let Some(doc) = self.doc.clone() else { return };
        self.placed_head = false;
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
        if mode == Mode::Arrangement
            && let Some((start, end)) = self.song_loop
        {
            engine.set_loop_range(start, end);
        }
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
        if on && self.song_view {
            if let Some(doc) = &self.doc {
                self.song_loop = Some((0, doc.borrow().project().arrangement_length().max(0)));
            }
        }
        self.store_screen();
        if let Some(doc) = &self.doc {
            doc.borrow_mut().mark_unsaved();
        }
        if let (Some(mode), Some(Ok(out))) = (self.playing, &self.output) {
            out.set_looping(self.loops(mode));
            if mode == Mode::Arrangement
                && let Some((start, end)) = self.song_loop
            {
                out.set_loop_range(start, end);
            }
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
                    ratchet: Default::default(),
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
        if self.recording && self.scene_pattern(self.group) == Some(self.pattern) {
            let head = self.playhead().map(|h| (h.mode, h.position));
            self.arm_grow_for(self.group, self.pattern, head, false);
        }
    }

    /// Focus a sound. A plain focus replaces SELECT's set with that one sound.
    fn focus_sound(&mut self, sound: usize) {
        self.sound = sound;
        self.sound_selection.clear();
        self.sound_anchor = Some(sound);
    }

    /// Whether `sound` in `group` is part of the active sound selection (focused or secondary).
    pub(crate) fn is_sound_selected(&self, group: usize, sound: usize) -> bool {
        group == self.group && (sound == self.sound || self.sound_selection.contains(&sound))
    }

    /// When two or more sounds in `group` are selected and `sound` is among them, returns
    /// all selected sound indices in `0..count` in pad order.
    pub(crate) fn selected_sounds_for(&self, group: usize, sound: usize, count: usize) -> Option<Vec<usize>> {
        if group != self.group || self.sound_selection.is_empty() || !self.is_sound_selected(group, sound) {
            return None;
        }
        let out: Vec<usize> = (0..count).filter(|&s| s == self.sound || self.sound_selection.contains(&s)).collect();
        (out.len() > 1).then_some(out)
    }

    /// Click a sound in the Sound list or Mixer strip header:
    /// Shift+click selects the inclusive range from `sound_anchor` (or `self.sound`) to `sound`;
    /// Cmd/Ctrl+click (`toggle`) adds or removes `sound`; plain click focuses `sound`.
    pub(crate) fn click_sound(&mut self, sound: usize, shift: bool, toggle: bool) {
        let n = self.group_len();
        if sound >= n {
            return;
        }
        if shift {
            let anchor = self.sound_anchor.filter(|&a| a < n).unwrap_or(self.sound);
            let (lo, hi) = (anchor.min(sound), anchor.max(sound));
            self.sound = sound;
            self.sound_selection = (lo..=hi).filter(|&s| s != sound).collect();
            self.sound_anchor = Some(anchor);
        } else if toggle {
            if sound == self.sound {
                if let Some(next) = self.sound_selection.pop() {
                    self.sound = next;
                    self.sound_anchor = Some(next);
                }
            } else if self.sound_selection.contains(&sound) {
                self.sound_selection.retain(|&s| s != sound);
            } else {
                if !self.sound_selection.contains(&self.sound) {
                    self.sound_selection.push(self.sound);
                }
                self.sound_selection.retain(|&s| s != sound);
                self.sound = sound;
                self.sound_anchor = Some(sound);
            }
        } else {
            self.focus_sound(sound);
        }
    }

    /// Move focus to `sound` while keeping the current multi-selection when `sound` is
    /// already selected (e.g. opening a context menu on a selected sound).
    pub(crate) fn focus_selected_sound(&mut self, sound: usize) {
        let n = self.group_len();
        if sound >= n {
            return;
        }
        if self.is_sound_selected(self.group, sound) {
            if sound != self.sound {
                if !self.sound_selection.contains(&self.sound) {
                    self.sound_selection.push(self.sound);
                }
                self.sound_selection.retain(|&s| s != sound);
                self.sound = sound;
            }
        } else {
            self.focus_sound(sound);
        }
    }

    /// Apply a sound swing edit across all selected sounds when `s` is part of a multi-selection.
    pub(crate) fn set_multi_sound_swing(&mut self, g: usize, s: usize, sw: deckard_model::Swing, exact: bool) -> bool {
        use deckard_doc::Target;
        use mxprj::decode::Value;
        let Some(doc) = self.doc.clone() else { return false };
        let count = doc.borrow().project().groups.get(g).map_or(0, |gr| gr.sounds.len());
        let Some(sounds) = self.selected_sounds_for(g, s, count) else { return false };
        let edits: Vec<(Target, &str, Value)> = {
            let d = doc.borrow();
            let p = d.project();
            let Some(old) = p.groups.get(g).and_then(|gr| gr.sounds.get(s)).map(|snd| snd.swing) else { return false };
            let mut out = Vec::new();
            if exact || (sw.amount - old.amount).abs() > 1e-9 {
                let delta = sw.amount - old.amount;
                for &si in &sounds {
                    let cur = p.groups[g].sounds[si].swing.amount;
                    let amount = if exact { sw.amount } else { cur + delta }.clamp(0.0, 100.0);
                    out.push((Target::Sound(g, si), "swing_amount", Value::Float(amount)));
                }
            }
            if exact || sw.cycle != old.cycle {
                let cycle = sw.cycle.clamp(0, 7);
                for &si in &sounds {
                    out.push((Target::Sound(g, si), "swing_cycle", Value::Int(cycle)));
                }
            }
            if exact || sw.invert != old.invert {
                let inv = sw.invert as i64;
                for &si in &sounds {
                    out.push((Target::Sound(g, si), "swing_invert", Value::Int(inv)));
                }
            }
            out
        };
        if !edits.is_empty()
            && let Err(e) = doc.borrow_mut().set_many("Swing", &edits)
        {
            self.status = format!("can't set swing: {e}");
        }
        true
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
            if self.sound_anchor.is_none() {
                self.sound_anchor = Some(self.sound);
            }
        } else if selected {
            if index != self.sound && !self.sound_selection.contains(&self.sound) {
                self.sound_selection.push(self.sound);
            }
            self.sound_selection.retain(|&s| s != index);
            self.sound = index;
            self.sound_anchor = Some(index);
        } else {
            self.sound = index;
            self.sound_selection.clear();
            self.sound_anchor = Some(index);
        }
    }

    /// SELECT + ALL: every sound of the focused group, focus unchanged.
    fn select_all_sounds(&mut self) {
        let n = self.group_len();
        self.sound_selection = (0..n).filter(|&s| s != self.sound).collect();
        if self.sound_anchor.is_none() {
            self.sound_anchor = Some(self.sound);
        }
    }

    /// SELECT + NONE: only the focused sound stays selected.
    fn select_no_sounds(&mut self) {
        self.sound_selection.clear();
        self.sound_anchor = Some(self.sound);
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

    /// Progress (0..1) shown on the MK3 touchstrip: the focused group's pattern (or,
    /// while growing an empty Auto-length take, the bars since the first recorded one), or
    /// the current bar with no pattern.
    fn strip_progress(&self, head: Option<Playhead>) -> Option<f64> {
        let (at, length) = self.pattern_at(self.group, head).or_else(|| self.bar_at(head))?;
        if self.recording
            && let Some(grow) = &self.grow
            && self.group == grow.group
            && self.scene_pattern(self.group) == Some(grow.pattern)
        {
            let d = self.doc.as_ref()?.borrow();
            let p = d.project();
            let rel = head.and_then(|h| Self::mode_pattern_tick(p, h.mode, h.position, grow.group, grow.pattern)).unwrap_or(at);
            let pattern = p.groups.get(grow.group).and_then(|g| g.patterns.get(grow.pattern));
            let first_note = pattern.and_then(|pt| pt.notes.iter().map(|n| n.position - pt.start).min());
            return Some(grow.strip_progress(rel, first_note));
        }
        Some(at as f64 / length as f64)
    }

    /// Sound `s` of group `g`'s Audio plug-in loop switched on (its focused take) or off
    /// in the pattern the group plays (the focused one, for the focused group with none),
    /// as Maschine's step pad and the pattern editor's double-click do. One undo step.
    fn toggle_audio_loop(&mut self, g: usize, s: usize) -> Result<(), String> {
        let doc = self.doc.clone().ok_or("no project")?;
        let pi = self.scene_pattern(g).or((g == self.group).then_some(self.pattern)).ok_or("the group plays no pattern")?;
        let mut d = doc.borrow_mut();
        let p = d.project();
        let snd = p.groups.get(g).and_then(|x| x.sounds.get(s)).ok_or("no such sound")?;
        let take = snd.audio.as_ref().ok_or("the sound has no Audio plug-in")?.focused;
        let pattern = p.groups[g].patterns.get(pi).ok_or("no such pattern")?;
        let on = pattern.audio.iter().any(|c| c.sound == s);
        if on { d.set_audio_in(g, s, &[pi], None, "Audio Off") } else { d.set_audio_in(g, s, &[pi], Some(take), "Audio On") }
    }

    pub(crate) fn scene_pattern(&self, group: usize) -> Option<usize> {
        let d = self.doc.as_ref()?.borrow();
        let s = d.project().scenes.get(self.scene)?;
        s.patterns.iter().find(|(g, _)| *g == group).map(|&(_, pi)| pi)
    }

    /// Group `g`'s active pattern for recording or clearing modulation: the playing
    /// pattern at the playhead when running, else its scene pattern, else the focused one.
    pub(crate) fn active_modulation_pattern(&self, g: usize) -> usize {
        if let Some(doc) = &self.doc {
            let d = doc.borrow();
            if let Some(h) = self.playhead()
                && let Some((pi, _)) = record::pattern_at(d.project(), h.mode, h.position, g)
            {
                return pi;
            }
            drop(d);
            if let Some(Mode::Pattern(pg, pi)) = self.playing
                && pg == g
            {
                return pi;
            }
            if let Some(pi) = self.scene_pattern(g) {
                return pi;
            }
        }
        if g == self.group { self.pattern } else { 0 }
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
        if self.recording {
            match &act {
                Act::NewPattern(s, g) | Act::NewPatternAt(s, g, _)
                    if *s == self.scene && self.grow.as_ref().is_some_and(|gr| gr.group == *g) =>
                {
                    self.settle_active_grow();
                }
                Act::AssignPattern(s, g, p)
                    if *s == self.scene && self.grow.as_ref().is_some_and(|gr| gr.group == *g && Some(gr.pattern) != *p) =>
                {
                    self.settle_active_grow();
                }
                _ => {}
            }
        }
        let mut arm_after: Option<(usize, usize, bool)> = None;
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
                Act::AssignPattern(s, g, p) => {
                    let r = d.assign_pattern(s, g, p).map(|_| None);
                    if r.is_ok()
                        && s == self.scene
                        && let Some(pi) = p
                    {
                        arm_after = Some((g, pi, g == self.group));
                    }
                    r
                }
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
                        if let Some(gr) = &mut self.grow
                            && gr.group == g
                            && gr.pattern >= made
                        {
                            gr.pattern += 1;
                        }
                        if self.recording {
                            self.take.insert_empty_pattern(g, made);
                            if s == self.scene {
                                arm_after = Some((g, made, g == self.group));
                            }
                        }
                        (self.group, self.pattern) = (g, made);
                    }
                    r.map(|_| None)
                }
                Act::MovePattern(g, p, slot) => {
                    // The focus stays on the pattern it was on, wherever that went.
                    let slots = |d: &deckard_doc::Document| {
                        d.project().groups.get(g).map(|x| x.patterns.iter().map(|p| p.slot).collect::<Vec<_>>())
                    };
                    let before = slots(&*d).unwrap_or_default();
                    let focused = (self.group == g).then(|| before.get(self.pattern).copied()).flatten();
                    let growing = self.grow.as_ref().filter(|gr| gr.group == g).and_then(|gr| before.get(gr.pattern).copied());
                    let r = d.move_pattern(g, p, slot);
                    if let (Ok(_), Some(&from)) = (&r, before.get(p)) {
                        let mapped = |s0: usize| {
                            if s0 == from {
                                slot
                            } else if s0 == slot {
                                from
                            } else {
                                s0
                            }
                        };
                        let after = slots(&*d);
                        if let Some(f) = focused
                            && let Some(i) = after.as_ref().and_then(|s| s.iter().position(|x| *x == mapped(f)))
                        {
                            self.pattern = i;
                        }
                        if let (Some(gs), Some(gr)) = (growing, self.grow.as_mut())
                            && let Some(i) = after.as_ref().and_then(|s| s.iter().position(|x| *x == mapped(gs)))
                        {
                            gr.pattern = i;
                        }
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
                    if self.grow.as_ref().is_some_and(|gr| (gr.group, gr.pattern) == (g, p)) {
                        self.grow = None;
                    }
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
        if self.recording
            && let Some((g, pi, force)) = arm_after
        {
            let head = self.playhead().map(|h| (h.mode, h.position));
            self.arm_grow_for(g, pi, head, force);
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
        self.placed_head = true;
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
        self.placed_head = true;
        self.sync_transport();
    }

    pub(crate) fn set_song_loop(&mut self, start: i64, end: i64) {
        let Some(doc) = self.doc.clone() else { return };
        let length = doc.borrow().project().arrangement_length().max(0);
        let start = start.clamp(0, length);
        let end = end.clamp(start, length);
        self.song_loop = Some((start, end));
        self.store_screen();
        doc.borrow_mut().mark_unsaved();
        if let (Some(Mode::Arrangement), Some(Ok(out))) = (self.playing, &self.output) {
            out.set_loop_range(start, end);
        }
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
        if self.recording {
            self.mark_grow_recorded(&doc, g, mode, tick);
        }
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

    /// What knob `k` of the page shows with its parameter at `at` (0..1): the plug-in's
    /// own text for a hosted one, the module's for a built-in one. `None` where it can't say.
    /// The values a knob's parameter picks among, if it's a choice of a few (at most
    /// six): DECKARD's choices and switches by name, a plug-in's stepped parameter by its
    /// text at each step. Empty otherwise (the MK3 draws a dial, or nothing, over it).
    fn knob_choices(&self, edit: layout::Edit) -> Vec<String> {
        use layout::Edit;
        const MAX: usize = 6;
        let stepped = |chain: deckard_doc::Chain, slot: usize, id: u32| -> Option<Vec<String>> {
            let v = self.plugin_views.get(&(chain, slot))?;
            let page = v.pages.get(v.page)?;
            let i = page.knobs.iter().position(|(info, ..)| info.id == id)?;
            if let Some(names) = page.choices.get(i).filter(|c| !c.is_empty()) {
                return Some(names.clone());
            }
            let steps = page.knobs[i].0.steps;
            if !(1..MAX as i32).contains(&steps) {
                return None;
            }
            let inst = self.samples.plugins.existing(self.plugin_slot_key(chain, slot))?;
            let key = (std::sync::Arc::as_ptr(&inst) as *const () as usize, id, steps);
            let Ok(p) = inst.try_lock() else { return self.step_names.borrow().get(&key).cloned() };
            let names: Vec<String> = (0..=steps).map(|s| p.param_text(id, f64::from(s) / f64::from(steps)).trim().to_string()).collect();
            self.step_names.borrow_mut().insert(key, names.clone());
            Some(names)
        };
        let names = match edit {
            Edit::Choice(_, _, names, ..) => Some(names.iter().map(|s| s.to_string()).collect()),
            Edit::Toggle(..) => Some(vec!["Off".to_string(), "On".to_string()]),
            Edit::PluginChoice(slot, id, ..) | Edit::Plugin(slot, id, _) => stepped(self.chain(), slot, id),
            Edit::PluginIn(c, i, id, _) => stepped(c, i, id),
            _ => None,
        };
        names.filter(|n: &Vec<String>| (2..=MAX).contains(&n.len())).unwrap_or_default()
    }

    fn mod_text(&self, edit: layout::Edit, k: usize, at: f64) -> Option<String> {
        use layout::Edit;
        let (chain, target) = modulation::mod_target(edit)?;
        let slot_i = target.slot?;
        if let Edit::PluginIn(c, i, id, _) = edit {
            let inst = self.samples.plugins.existing(self.plugin_slot_key(c, i))?;
            let text = inst.try_lock().ok()?.param_text(id, at);
            let units = self
                .plugin_views
                .get(&(c, i))
                .and_then(|v| v.pages.get(v.page)?.knobs.iter().find(|(info, ..)| info.id == id))
                .map(|(info, ..)| info.units.clone())
                .unwrap_or_default();
            return Some(if units.is_empty() || text.ends_with(&units) { text } else { format!("{text} {units}") });
        }
        let layout::Act::SetParam(t, field, value) = edit.to_act(at as f32) else { return None };
        let doc = self.doc.as_ref()?.borrow();
        let p = doc.project();
        let slots = match chain {
            deckard_doc::Chain::Sound(g, s) => &p.groups.get(g)?.sounds.get(s)?.slots,
            deckard_doc::Chain::Group(g) => &p.groups.get(g)?.slots,
            deckard_doc::Chain::Master => return None,
        };
        let pg = if let deckard_doc::Target::Sampler(g, s) = t {
            let mut smp = p.groups.get(g)?.sounds.get(s)?.sampler.clone()?;
            smp.set(field, value);
            sampler_ui::page(&smp, g, s, self.sampler_page.min(sampler_ui::PAGES.len() - 1))
        } else {
            let mut slot = slots.get(slot_i)?.clone();
            slot.params.iter_mut().find(|(f, _)| f == field)?.1 = value;
            let names = crate::module_ui::pages(&slot.module)?;
            module_ui::page_for(&slot, t, self.plugin_page.min(names.len().saturating_sub(1)))
        };
        pg.slots.into_iter().nth(k).flatten().map(|knob| knob.1)
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

    /// Whether `edit`'s parameter has modulation in its group's active pattern (the
    /// playing pattern while playing, the focused one when stopped), or an open touch.
    pub(crate) fn is_modulated_in(&self, p: &deckard_model::Project, edit: layout::Edit) -> bool {
        let Some((chain, target)) = modulation::mod_target(edit) else { return false };
        if self.mod_takes.iter().any(|t| t.chain == chain && t.target == target) {
            return true;
        }
        let (g, sound) = match chain {
            deckard_doc::Chain::Sound(g, s) => (g, Some(s)),
            deckard_doc::Chain::Group(g) => (g, None),
            deckard_doc::Chain::Master => return false,
        };
        let pi = match self.playhead() {
            Some(h) => record::pattern_at(p, h.mode, h.position, g).map(|(pi, _)| pi),
            None if g == self.group => Some(self.pattern),
            None => p.scenes.get(self.scene).and_then(|sc| sc.patterns.iter().find_map(|&(gr, pi)| (gr == g).then_some(pi))),
        };
        let Some(pat) = pi.and_then(|pi| p.groups.get(g)?.patterns.get(pi)) else { return false };
        pat.mods.iter().any(|m| m.sound == sound && m.slot == target.slot && m.field == target.field)
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
            // An empty Auto-length pattern starts as a bar (or a bar ahead of where
            // playback already is), growing as it's played into (unless the Settings
            // page's GROW is off).
            self.grow = None;
            let head = self.playhead().map(|h| (h.mode, h.position));
            self.arm_grow_for(self.group, self.pattern, head, true);
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
            self.settle_active_grow();
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
                doc.borrow_mut().set_length_auto(g, pi, false);
                if leading > 0 {
                    let _ = doc.borrow_mut().set_pattern_span(g, pi, start, length);
                }
            }
            doc.borrow_mut().end_take();
            self.status = "Recorded".into();
        }
    }

    /// Unwrapped tick of `mode` at `tick` when `(g, pi)` is the pattern `mode` plays for `g`.
    fn mode_pattern_tick(p: &deckard_model::Project, mode: Mode, tick: f64, g: usize, pi: usize) -> Option<i64> {
        let t = match mode {
            Mode::Pattern(pg, ppi) => ((pg, ppi) == (g, pi)).then_some(tick)?,
            Mode::Scene(s) => p.scenes.get(s)?.patterns.contains(&(g, pi)).then_some(tick)?,
            Mode::Arrangement => {
                let si = p.section_at(tick as i64)?;
                let scene = p.sections[si].scene?;
                p.scenes.get(scene)?.patterns.contains(&(g, pi)).then_some(tick - p.section_start(si) as f64)?
            }
        };
        Some((t.round() as i64).max(0))
    }

    /// Settle `self.grow` (trimming to the bars played, or restoring `before` when empty).
    fn settle_active_grow(&mut self) {
        let Some(doc) = self.doc.clone() else {
            self.grow = None;
            return;
        };
        let Some(grow) = self.grow.take() else { return };
        let (g, pi) = (grow.group, grow.pattern);
        let settle = {
            let d = doc.borrow();
            let p = d.project();
            let bar = p.bar_length();
            p.groups.get(g).and_then(|gr| gr.patterns.get(pi)).map(|pat| {
                let first = pat.notes.iter().map(|n| n.position).min();
                let start = first.map_or(pat.start, |n| pat.start + (n - pat.start).max(0) / bar * bar);
                let end = pat.notes.iter().map(|n| n.position - start + n.length).max();
                let to = end.map_or(grow.before, |e| ((e + bar - 1) / bar).max(1) * bar);
                (start, to, end.is_some())
            })
        };
        if let Some((start, to, has_notes)) = settle {
            if has_notes {
                doc.borrow_mut().set_length_auto(g, pi, false);
            }
            let _ = doc.borrow_mut().set_pattern_span(g, pi, start, to);
        }
    }

    /// Start auto-growing `(g, pi)` when recording into an empty Auto-length pattern.
    /// If `at` is `Some((mode, tick))` and `(g, pi)` is playing there, `Grow` starts a bar
    /// ahead of `tick` so mid-scene recording never wraps backwards at bar boundaries.
    fn arm_grow_for(&mut self, g: usize, pi: usize, at: Option<(Mode, f64)>, force: bool) {
        if !self.recording || !self.settings.pattern_grow {
            return;
        }
        if self.grow.as_ref().is_some_and(|gr| (gr.group, gr.pattern) == (g, pi)) {
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        let eligible = {
            let d = doc.borrow();
            let p = d.project();
            if !d.length_is_auto(g, pi) {
                None
            } else {
                p.groups.get(g).and_then(|gr| gr.patterns.get(pi)).filter(|pt| pt.notes.is_empty()).map(|pt| {
                    let rel = at.and_then(|(m, t)| Self::mode_pattern_tick(p, m, t, g, pi));
                    (pt.length, p.bar_length(), rel)
                })
            }
        };
        let Some((len, bar, rel)) = eligible else { return };
        if let Some(prev) = &self.grow {
            let prev_occupied = prev.has_recorded()
                || doc
                    .borrow()
                    .project()
                    .groups
                    .get(prev.group)
                    .and_then(|gr| gr.patterns.get(prev.pattern))
                    .is_some_and(|pt| !pt.notes.is_empty());
            if prev_occupied && prev.group != g && !force {
                return;
            }
            self.settle_active_grow();
        }
        let mut grow = record::Grow::new(g, pi, bar);
        grow.before = len;
        if let Some(rel) = rel {
            let _ = grow.reach(rel);
        }
        if doc.borrow_mut().set_pattern_length(g, pi, grow.length()).is_ok() {
            self.grow = Some(grow);
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

    fn mark_grow_recorded(&mut self, doc: &std::rc::Rc<std::cell::RefCell<deckard_doc::Document>>, group: usize, mode: Mode, tick: f64) {
        if self.grow.as_ref().is_none_or(|gr| gr.group != group) {
            let candidate = {
                let d = doc.borrow();
                let p = d.project();
                record::pattern_at(p, mode, tick, group)
                    .map(|(pi, _)| pi)
                    .filter(|&pi| self.settings.pattern_grow && d.length_is_auto(group, pi))
                    .filter(|&pi| p.groups.get(group).and_then(|g| g.patterns.get(pi)).is_some_and(|pt| pt.notes.is_empty()))
            };
            if let Some(pi) = candidate {
                self.arm_grow_for(group, pi, Some((mode, tick)), false);
            }
        }
        let Some(len) = self.grow.as_mut().and_then(|grow| {
            if group != grow.group {
                return None;
            }
            let d = doc.borrow();
            let p = d.project();
            p.groups.get(group)?.patterns.get(grow.pattern)?;
            let at = Self::mode_pattern_tick(p, mode, tick, group, grow.pattern)?;
            grow.record_at(at);
            grow.reach(at)
        }) else {
            return;
        };
        if let Some(grow) = &self.grow
            && let Err(e) = doc.borrow_mut().set_pattern_length(grow.group, grow.pattern, len)
        {
            self.status = format!("can't grow the pattern: {e}");
        }
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
                    self.mark_grow_recorded(&doc, r.group, r.mode, r.tick);
                    self.take.add_expr(r);
                    continue;
                }
                record::Rec::Control(r) => {
                    self.mark_grow_recorded(&doc, r.group, r.mode, r.tick);
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
            if r.velocity > 0 {
                self.mark_grow_recorded(&doc, r.group, r.mode, r.tick);
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
        let rel = {
            let d = doc.borrow();
            let p = d.project();
            if p.groups.get(g).and_then(|gr| gr.patterns.get(pi)).is_none() {
                self.grow = None;
                return;
            }
            Self::mode_pattern_tick(p, mode, tick, g, pi)
        };
        if let Some(len) = rel.and_then(|rel| grow.reach(rel))
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
            if let Some(pi) = self.scene_pattern(self.group)
                && self.grow.as_ref().is_none_or(|gr| gr.group == self.group && gr.pattern != pi)
            {
                let force = self.grow.as_ref().is_some_and(|gr| gr.group == self.group);
                self.arm_grow_for(self.group, pi, Some((h.mode, h.position)), force);
            }
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
        let pace = controller::tick_every(self.playing.is_some() || self.recorder.busy() || self.recorder.is_metering());
        if self.plugin_spectrum.borrow().as_ref().is_some_and(plugin_spectrum::Focus::is_live) {
            return Some(pace.min(plugin_spectrum::FRAME_INTERVAL));
        }
        Some(if self.surface.holding() || self.surface.animates() || self.filter_modulates() || self.picture_moves() {
            pace.min(std::time::Duration::from_millis(30))
        } else {
            pace
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
            ..Default::default()
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
            m.set_leds(out.leds);
            m.set_led_brightness(self.settings.led_brightness);
            m.set_display_brightness(self.settings.display_brightness);
            // Written to the controller only when it changes (and on connecting).
            m.set_pad_sensitivity(self.settings.pad_sensitivity);
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

    pub(crate) fn surface_view_stamp(&self) -> SurfaceViewStamp {
        (
            self.doc.as_ref().map(|d| (std::rc::Rc::as_ptr(d) as usize, d.borrow().revision())),
            self.group,
            self.sound,
            self.scene,
            self.section,
            self.song_view,
            self.slot,
            self.slot_of,
            self.plugin_focus.and_then(|k| self.plugin_views.get(&k)).map(|v| (v.key, v.shown, v.refreshed)),
            !self.surface_settings.values.is_empty(),
        )
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
        // The group's sounds' Pad Touch, for what their held pads' pressure sends.
        let touches = |app: &App, group: usize| {
            let mut t = [deckard_model::pad_touch::PadTouch::default(); 16];
            if let Some(d) = &app.doc
                && let Some(g) = d.borrow().project().groups.get(group)
            {
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
                    Event::PadOn(pad, v) => c.pad_event(pad, true, mk3_io::pad_velocity(v, self.settings.velocity_scaling), requests),
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
            let pin_changed = c.sync_pins(&mut self.settings.pinned_pages);
            let route = mk3_io::PadRoute { play, group, keys, chords, velocity, scaling, fixed, touches: touches(self, group) };
            if pin_changed {
                #[cfg(not(test))]
                self.settings.save();
            }
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
            self.stop_prehear_after_browse();
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
                self.follow_edits();
                // (A slot or sound the buttons just focused shows its plug-in's knobs.)
                self.update_slot_view();
                self.refresh_surface_settings();
                view = self.surface_view();
            }
            self.stop_prehear_after_browse();
            self.sync_surface_pins();
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
            let (pads, leds, screens) = self.surface.outputs(&view);
            let route = mk3_io::PadRoute {
                play: pads.play,
                group: self.group,
                keys: pads.keys,
                chords: pads.chords,
                velocity: view.fixed_velocity.then_some(fixed.clamp(0, 127)),
                scaling: self.settings.velocity_scaling,
                fixed: fixed.clamp(0, 127),
                touches: touches(self, self.group),
            };
            self.frame_surface_view = Some((self.surface_view_stamp(), view));
            Mk3Out { route, leds, screens: screens.map(|(n, p)| (n, mk3_io::ScreenFrame::Picture(std::sync::Arc::new(p)))) }
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

    fn refresh_doc_cache(&self, d: &deckard_doc::Document) {
        let rev = d.revision();
        let mut cache = self.surface_doc_cache.borrow_mut();
        if cache.key != Some((self.doc_generation, rev)) {
            let group_noted = d
                .project()
                .groups
                .iter()
                .map(|grp| grp.patterns.iter().flat_map(|pt| pt.notes.iter().map(|n| n.sound)).collect())
                .collect();
            *cache = SurfaceDocCache { key: Some((self.doc_generation, rev)), macro_targets: d.macro_targets(), group_noted };
        }
    }

    pub(crate) fn doc_cache(&self, d: &deckard_doc::Document) -> std::cell::Ref<'_, SurfaceDocCache> {
        self.refresh_doc_cache(d);
        self.surface_doc_cache.borrow()
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
            self.refresh_doc_cache(&d);
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
        let mod_at: Vec<Option<f64>> = (0..8).map(|i| edits.get(i).copied().flatten().and_then(|e| self.mod_position(e))).collect();
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
                        mod_position: mod_at[i],
                        mod_text: mod_at[i].and_then(|at| edits.get(i).copied().flatten().and_then(|e| self.mod_text(e, i, at))),
                        choices: edits.get(i).copied().flatten().map(|e| self.knob_choices(e)).unwrap_or_default(),
                        param_id: match edits.get(i).copied().flatten() {
                            Some(
                                layout::Edit::Plugin(_, id, _)
                                | layout::Edit::PluginChoice(_, id, ..)
                                | layout::Edit::PluginIn(_, _, id, _),
                            ) => Some(id),
                            _ => None,
                        },
                        name: k.name.clone(),
                        section: k.section.clone(),
                        text: knob_text(i, &k.text),
                        value: k.value,
                        modulatable: edits.get(i).copied().flatten().and_then(modulation::mod_target).is_some(),
                        step_mod: self.step_mod_text(edits.get(i).copied().flatten()),
                        auto_mod: self.auto_mod_text(edits.get(i).copied().flatten()),
                        in_macro: !on_macros && k.param.is_some_and(|p| doc_cache.macro_targets.contains(&p)),
                        modulated: k.modulated,
                    })
                })
                .collect(),
            // (Only while the plug-in browser shows: the list is long.)
            modules: if self.surface.page() == "module" { self.module_views() } else { Vec::new() },
            eq: None,
            filter: None,
            picture: None,
            output_spectrum: Vec::new(),
            zhora: None,
            tuner: None,
            scope: None,
            euclid: self.euclid_view(c.level, c.page, head),
            random: self.random_view(c.level, c.page, head),
        };
        let mut v = View {
            recent: self.recent.iter().map(|p| p.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned())).collect(),
            settings: self.surface_settings.clone(),
            // (The strip hides while this is open, so a script declines an ENCODER press there.)
            browser_open: self.browser.open,
            page: self.surface.page(),
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
            progress: self.strip_progress(head),
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
        let Some(doc) = &self.doc else {
            self.plugin_spectrum.borrow_mut().take();
            return v;
        };
        let d = doc.borrow();
        let p = d.project();
        let chain = self.chain();
        let slot = d.chain_slots(chain).and_then(|slots| self.slot.and_then(|i| slots.get(i)));
        (v.control.eq, v.control.output_spectrum) = self.plugin_spectrum_view(chain, self.slot.zip(slot));
        v.tempo = self.playing_scene_tempo(p).map_or(p.tempo, |t| t.1);
        // The built-in EQ's or Filter's curve, for the right screen. Both pages use it.
        if !self.channel {
            let tempo = v.tempo;
            let live = self.slot.is_some_and(|i| Self::sidechain_routed(p, self.chain(), i));
            v.control.filter = slot.and_then(|s| Self::filter_curve(s, tempo, live));
            let page = c.page;
            v.control.picture = slot.and_then(|s| Self::effect_picture(s, Self::picture_phase(&s.module, &s.params, tempo), live, page));
            // The tuner reads the instance: only for the page that shows it.
            if self.surface.page() == "control" {
                v.control.zhora = self.zhora_view(p);
                v.control.tuner = self.tuner_view();
                v.control.scope = self.scope_view();
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
        // The focused sound's Variation page (work-item 35).
        v.variation = self.doc.as_ref().map(|doc| {
            let d = doc.borrow();
            let x = d.variation(self.group, self.sound);
            deckard_surface::VariationView {
                mode: x.mode,
                velocity_lo: x.velocity_lo,
                velocity_hi: x.velocity_hi,
                time_shift: x.time_shift,
                probability: x.probability,
                note_lo: x.note_lo,
                note_hi: x.note_hi,
                note_count: x.note_count,
                steps: x.steps,
                dist_note_count: x.dist_note_count,
                dist_notes: x.dist_notes,
                dist_note_length: x.dist_note_length,
                euclid_steps: d.groove_setting(self.group, self.sound, "sequence_generation/steps").unwrap_or(8.0) as i64,
                fills: d.groove_setting(self.group, self.sound, "sequence_generation/fills").unwrap_or(5.0) as i64,
                rotate: d.groove_setting(self.group, self.sound, "sequence_generation/rotate").unwrap_or(0.0) as i64,
                division: d.groove_setting(self.group, self.sound, "sequence_generation/division").unwrap_or(0.0) as i64,
                keyboard: self.piano_roll,
            }
        });
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
            v.pattern_auto = d.length_is_auto(self.group, self.pattern) && pt.notes.is_empty();
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
                    ratchet: n.ratchet,
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
        let (has_live_engine, out_is_current) = match &self.output {
            Some(Ok(out)) if out.has_engine() => (true, out.is_current()),
            _ => (false, false),
        };
        let active_mode = self.playing.map(|m| live.map_or(m, |h| h.mode));
        v.groups = (0..p.groups.len())
            .map(|g| {
                let grp = &p.groups[g];
                let loudest = (0..grp.sounds.len()).map(|s| meter(g, s)).fold((0f32, 0f32), |m, x| (m.0.max(x.0), m.1.max(x.1)));
                // The sounds with notes in any of the group's patterns.
                let noted = doc_cache.group_noted.get(g);
                let scene_pattern = p.scenes.get(self.scene).and_then(|sc| sc.patterns.iter().find(|(pg, _)| *pg == g).map(|&(_, pi)| pi));
                let loop_pattern = match active_mode {
                    Some(Mode::Pattern(pg, pi)) => (pg == g).then(|| grp.patterns.get(pi)).flatten(),
                    Some(Mode::Scene(sc)) => {
                        p.scenes.get(sc).and_then(|s| s.patterns.iter().find(|&&(pg, _)| pg == g)).and_then(|&(_, pi)| grp.patterns.get(pi))
                    }
                    Some(Mode::Arrangement) => head
                        .and_then(|h| p.section_at(h.position as i64))
                        .and_then(|i| p.sections[i].scene)
                        .and_then(|sc| p.scenes.get(sc))
                        .and_then(|s| s.patterns.iter().find(|&&(pg, _)| pg == g))
                        .map(|&(_, pi)| pi)
                        .or(scene_pattern)
                        .and_then(|pi| grp.patterns.get(pi)),
                    None => scene_pattern.or((g == self.group).then_some(self.pattern)).and_then(|pi| grp.patterns.get(pi)),
                };
                let group_ahead = self.settings.render_ahead_ms > 0 && g != self.group;
                let pat_at = if v.playing { self.pattern_at(g, head).map(|(at, _)| at) } else { None };
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
                    scene_pattern,
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
                            let loop_mode = snd.instrument == "AudioModule"
                                && snd.audio.as_ref().is_some_and(|a| a.mode == deckard_model::AudioMode::Loop);
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
                                hits: {
                                    let mut hits: Vec<i64> =
                                        self.hits.iter().filter(|&&(hg, hs, _)| (hg, hs) == (g, s)).map(|&(_, _, p)| p).collect();
                                    hits.sort_unstable();
                                    hits
                                },
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
                                loop_mode,
                                loop_playing: {
                                    let audio_loop_active = loop_mode
                                        && !grp.muted
                                        && !snd.muted
                                        && loop_pattern.is_some_and(|pt| pt.audio.iter().any(|c| c.sound == s));
                                    let sampler_loop_active = !grp.muted
                                        && !snd.muted
                                        && snd.sampler.as_ref().is_some_and(|smp| {
                                            if smp.envelope_type == 0 {
                                                return false;
                                            }
                                            let zone_loops = |pitch: i64, vel: Option<i64>| {
                                                smp.zones.iter().any(|z| {
                                                    z.loop_active
                                                        && z.loop_range.1 > z.loop_range.0
                                                        && (z.key_range.0..=z.key_range.1).contains(&pitch)
                                                        && vel.is_none_or(|v| (z.velocity_range.0..=z.velocity_range.1).contains(&v))
                                                })
                                            };
                                            if has_live_engine && !group_ahead && self.loop_lit.contains(&(g, s)) {
                                                return true;
                                            }
                                            if !out_is_current {
                                                if self
                                                    .held_pitches
                                                    .get(&(g, s))
                                                    .is_some_and(|ps| ps.iter().any(|&pt| zone_loops(pt, None)))
                                                {
                                                    return true;
                                                }
                                                if matches!(self.held, Some((hg, hs, hp)) if (hg, hs) == (g, s) && zone_loops(hp, None)) {
                                                    return true;
                                                }
                                            }
                                            if v.playing && (!has_live_engine || !out_is_current || group_ahead) {
                                                if let Some(pt) = loop_pattern {
                                                    return match pat_at {
                                                        Some(at) => {
                                                            let tick = pt.start + at;
                                                            pt.notes.iter().any(|n| {
                                                                n.sound == s
                                                                    && n.position <= tick
                                                                    && tick < n.position + n.length
                                                                    && zone_loops(n.pitch, Some(n.velocity))
                                                            })
                                                        }
                                                        None => {
                                                            pt.notes.iter().any(|n| n.sound == s && zone_loops(n.pitch, Some(n.velocity)))
                                                        }
                                                    };
                                                }
                                            }
                                            false
                                        });
                                    audio_loop_active || sampler_loop_active
                                },
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
            A::Variation { field, value } => {
                let Some(name) = variation_field(field) else { return };
                let Some(doc) = self.doc.clone() else { return };
                let (g, s) = (self.group, self.sound);
                let done = doc.borrow_mut().set_groove_setting(g, s, name, value);
                match done {
                    Err(e) => self.status = format!("can't set that: {e}"),
                    Ok(()) => self.live_groove(g, s, name),
                }
            }
            A::ApplyVariation => self.groove(layout::GrooveButton::ApplyVariation),
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
                let targets = match of {
                    deckard_surface::ColorOf::Pattern => vec![Target::Pattern(g, self.pattern)],
                    deckard_surface::ColorOf::Scene => vec![Target::Scene(self.scene)],
                    deckard_surface::ColorOf::Group => vec![Target::Group(g)],
                    deckard_surface::ColorOf::Sound => {
                        let count = self.group_len();
                        self.selected_sounds_for(g, self.sound, count).map_or_else(
                            || vec![Target::Sound(g, self.sound)],
                            |ss| ss.into_iter().map(|si| Target::Sound(g, si)).collect(),
                        )
                    }
                };
                let c = color.map_or(deckard_model::Color::Default, deckard_model::Color::Palette);
                layout::apply_acts(self, vec![Act::SetColor(targets, c)]);
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
            A::EraseKnob(knob) => self.control_request(R::Erase { pos: knob }),
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
            A::ToggleAudioLoop { group, sound } => {
                if let Err(e) = self.toggle_audio_loop(group, sound) {
                    self.status = format!("can't switch the loop: {e}");
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
            // (A script asking for the Settings page's values is satisfied by
            // construction: the View carries them as they are, whoever is looking.)
            A::SettingsWanted(_) => {}
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
                let count = self.group_len();
                let res = if let Some(sounds) = self.selected_sounds_for(g, self.sound, count) {
                    let edits: Vec<_> = sounds.into_iter().map(|si| (Target::Sound(g, si), field, mxprj::decode::Value::Int(v))).collect();
                    self.doc.as_ref().map(|doc| doc.borrow_mut().set_many(label, &edits))
                } else {
                    self.doc
                        .as_ref()
                        .map(|doc| doc.borrow_mut().set(label, Target::Sound(g, self.sound), field, mxprj::decode::Value::Int(v)))
                };
                if let Some(Err(e)) = res {
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
                        modulated: edit.is_some_and(|e| self.is_modulated_in(p, e)),
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
                        modulated: edit.is_some_and(|e| self.is_modulated_in(p, e)),
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
            // VARIATION: the Groove page the focused sound was left on (Swing,
            // Variation or Generate), else page `n`.
            R::GroovePage(n) => {
                layout::apply_acts(self, vec![Act::ChainTab(layout::ChainTab::Sound)]);
                self.channel = true;
                self.prop_set = 2;
                let (g, s) = (self.group, self.sound);
                let page = self.sound_groove_page(deckard_doc::Chain::Sound(g, s)).unwrap_or(n.min(2));
                if let Some(doc) = &self.doc {
                    doc.borrow_mut().set_groove_page(g, s, page);
                }
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

    /// The key the Sampling page's PLAY plays: the focused zone's root (an Audio plug-in's
    /// take: as its page shows it).
    fn audition_key(&self) -> Option<i64> {
        let doc = self.doc.as_ref()?.borrow();
        layout::sampling::edited(doc.project(), (self.group, self.sound, self.zone)).map(|e| e.1.root_key)
    }

    /// The Sampling page's PLAY / STOP: the focused sound's note on `pitch` held until
    /// the next press, which ends it and whatever of the sound still rings (a one-shot
    /// doesn't end with its note). The song, if it plays, plays on.
    pub(crate) fn audition_toggle(&mut self, pitch: i64) {
        let (g, s) = (self.group, self.sound);
        // Slice Motion would play another slice than the one shown: this one is heard by
        // itself (as its pad on SLICE plays it).
        let in_motion = self.doc.as_ref().is_some_and(|d| {
            let d = d.borrow();
            d.project().groups.get(g).and_then(|x| x.sounds.get(s)).is_some_and(|x| {
                x.new_sampler.is_some_and(|n| n.motion != Default::default()) && x.sampler.as_ref().is_some_and(|z| z.zones.len() > 1)
            })
        });
        if in_motion {
            if self.prehear_from.take().is_some() {
                if let Some(Ok(out)) = &self.output {
                    out.prehear(None, 0.0);
                }
                return;
            }
            let selection = self.sampling.selection.take();
            let range = self.edit_range();
            self.sampling.selection = selection;
            if let Some((a, b)) = range {
                self.sampling_call(deckard_lua_host::state::SamplingCall::Prehear(a, b));
            }
            return;
        }
        let sounding = self.sampling.playhead.is_some();
        if let Some((ag, asnd, ap)) = self.audition.take()
            && let Some(Ok(out)) = &self.output
        {
            out.live(deckard_engine::Live::NoteRelease { group: ag, sound: asnd, pitch: ap });
            out.live(deckard_engine::Live::ChokeSound { group: ag, sound: asnd });
            if sounding {
                return;
            }
        } else if sounding {
            if let Some(Ok(out)) = &self.output {
                out.live(deckard_engine::Live::ChokeSound { group: g, sound: s });
            }
            return;
        }
        let before = self.held.take();
        self.hold_pitch(g, s, pitch);
        // Not the mouse's: it stays down when the button comes up.
        self.audition = self.held.take();
        self.held = before;
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
        if let Some(w) = &self.midi_port_watch {
            w.set_context(ctx);
        }
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
        self.forget_replaced_presets(doc.borrow().undo_label());
        let ok = doc.borrow_mut().undo();
        if !ok && !self.back_to_replaced() {
            self.status = "nothing to undo".into();
        }
        self.lock_hosted_sync();
    }

    fn redo(&mut self) {
        self.lock_end_morph();
        if let Some(doc) = self.doc.clone() {
            self.forget_replaced_presets(doc.borrow().redo_label());
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
        self.follow_analysis();
        self.follow_renders();
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
        // A session recording holds the master tap, so internal Sampling metering must not
        // grab it at the same time (mutual exclusion).
        let listen = self.meter_inputs
            && ((self.sampling_open && self.sampling.tab == 0) || hw_record)
            && !(self.session.active() && self.recorder.view.source == 2);
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
        // The session recorder takes up its tap and keeps the file's length true to
        // wall-clock time; it releases the tap itself on an auto-stop. (Re-borrow the
        // output: `finish_recording` above needed `self` mutably.)
        let output = match &self.output {
            Some(Ok(o)) => Some(o),
            _ => None,
        };
        self.session.poll(output);
        self.silence_while_recording();
        self.sync_sampling_page();
        self.sync_browse_page();
        self.sync_arp();
        self.sync_auto_write();
        self.tick_mod_take(self.playhead().map(|h| (h.mode, h.position)), std::time::Instant::now());
        // What was asked of, or picked on, another sound is dropped with the focus.
        let at = (self.group, self.sound);
        if self.sampler_ask.is_some_and(|(of, _)| of != at) {
            self.sampler_ask = None;
        }
        if self.take_shown.is_some_and(|of| of != at) {
            self.take_shown = None;
        }
        if !self.sampling_open {
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        let focused = self.focused_sample();
        // A take on no layer: RECORD and EDIT only.
        if self.shown_take().is_some() && self.sampling.tab > 1 {
            self.sampling.tab = 1;
        }
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
                    // (A slice's pad was playing the sample before: not this one.)
                    self.prehear_from = None;
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
                            // Zoomed in elsewhere in the file: the zone's own part of it comes into view.
                            let range = layout::sampling::edited(doc.borrow().project(), (self.group, self.sound, zi))
                                .filter(|_| zi != usize::MAX)
                                .map(|e| e.1.play_range);
                            if let Some((lo, hi)) = range.filter(|r| r.1 > r.0) {
                                self.sampling.reveal(lo as f64, hi as f64 + 1.0);
                            }
                        }
                        None => {
                            self.sampling.clear_sample();
                        }
                    }
                }
                self.sampling.playhead = match &self.output {
                    Some(Ok(out)) => out.zone_position(self.group, self.sound, zi),
                    _ => None,
                };
                // A slice's pad plays it: where that is by now.
                if self.sampling.playhead.is_none()
                    && let Some((start, a, b)) = self.prehear_from
                {
                    let rate = self.sampling.sample.as_ref().map_or(0, |s| s.rate);
                    self.sampling.playhead = chop::heard(start, std::time::Instant::now(), (a, b), rate);
                    if self.sampling.playhead.is_none() {
                        self.prehear_from = None;
                    }
                }
                // Its stem lanes' waveforms.
                let files: [Option<String>; deckard_model::new_sampler::LANES] = {
                    let d = doc.borrow();
                    let z = layout::sampling::edited(d.project(), (self.group, self.sound, self.zone)).filter(|e| !e.3 && zi != usize::MAX);
                    let stems = z.and_then(|(_, z, ..)| z.meta).map(|m| m.stems).unwrap_or_default();
                    stems.map(|l| l.map(|l| l.file))
                };
                let samples = &mut self.samples;
                self.sampling.set_lanes(files, |file| {
                    samples.sample(&deckard_model::SampleRef { kind: 0, path: file.to_string(), ..Default::default() }, path.as_deref())
                });
            }
            None => {
                self.sampling.clear_sample();
                // (An empty New Sampler has its LAYERS.)
                let layers = self.sampling.tab == 4 && layers::can_layer(doc.borrow().project(), self.group, self.sound);
                if self.sampling.tab != 0 && !layers {
                    self.sampling.tab = 0;
                }
            }
        }
        // LAYERS: every layer's waveform, where each plays, where the morph is.
        if self.sampling.tab == 4 {
            let files: Vec<(usize, String)> = {
                let d = doc.borrow();
                let p = d.project();
                let zones =
                    p.groups.get(self.group).and_then(|g| g.sounds.get(self.sound)).and_then(|s| s.sampler.as_ref()).map(|x| &x.zones);
                layers::layers_in(p, self.group, self.sound)
                    .iter()
                    .filter_map(|&(zi, _)| Some((zi, zones?.get(zi)?.sample.clone())))
                    .map(|(zi, sref)| (zi, sref.path))
                    .collect()
            };
            let samples = &mut self.samples;
            self.sampling.set_layers(files.iter().map(|f| f.1.clone()).collect(), |file| {
                samples.sample(&deckard_model::SampleRef { kind: 0, path: file.to_string(), ..Default::default() }, path.as_deref())
            });
            let (g, s) = (self.group, self.sound);
            match &self.output {
                Some(Ok(out)) => {
                    self.sampling.layer_heads = files.iter().map(|&(zi, _)| out.zone_position(g, s, zi)).collect();
                    self.sampling.morph_at = out.morph_position(g, s);
                }
                _ => {
                    self.sampling.layer_heads.clear();
                    self.sampling.morph_at = None;
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

    /// Whether STEMS puts the focused zone's stems on its lanes: an analysed zone of a
    /// Sampler Preview in Advanced (the lanes are that mode's).
    pub(crate) fn stems_onto_lanes(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| {
            let d = d.borrow();
            let p = d.project();
            layers::mode_of(p, self.group, self.sound) == Some(deckard_model::new_sampler::SamplerMode::Advanced)
                && p.groups[self.group].sounds[self.sound]
                    .sampler
                    .as_ref()
                    .and_then(|x| x.zones.get(self.zone))
                    .is_some_and(|z| z.meta.is_some())
        })
    }

    /// STEMS: separate the focused zone's sample in the background.
    fn start_stems(&mut self, to_group: bool) {
        let (Some(sample), Some(dir)) = (self.sampling.sample.clone(), menu::settings_dir()) else { return };
        let into_lanes = !to_group && self.stems_onto_lanes();
        let Some((_, sref, beats)) = self.focused_sample() else { return };
        let file = sref.path.rsplit(['/', '\\']).next().unwrap_or("Sample");
        let name = file.rsplit_once('.').map_or(file, |(stem, _)| stem).to_string();
        self.status = format!("separating {name} into stems…");
        let mut job = stems::Job::start(sample, name, (self.group, self.sound, self.zone), dir);
        job.beats = beats;
        job.into_lanes = into_lanes;
        self.stems = Some(job);
    }

    /// The focused sound's sample the Sampling view shows: its Sampler's focused zone's,
    /// or its Audio plug-in's focused take's (with the take's length in beats); and
    /// which zone or take it is.
    fn focused_sample(&self) -> Option<(usize, deckard_model::SampleRef, Option<f64>)> {
        // (In Layers, a take picked that is on no layer: no zone's, so no zone index.)
        if let Some((_, file)) = self.shown_take() {
            let sample = deckard_model::SampleRef { kind: 0, path: file.to_string_lossy().into(), ..Default::default() };
            return Some((usize::MAX, sample, None));
        }
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

    /// Analyse the New Sampler's zones that aren't yet, and keep what's found in each
    /// zone still holding that file (no undo step).
    fn follow_analysis(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let done = self.analysis.poll();
        if !done.is_empty() {
            let mut targets = Vec::new();
            {
                let d = doc.borrow();
                for (g, grp) in d.project().groups.iter().enumerate() {
                    for (s, snd) in grp.sounds.iter().enumerate() {
                        // (Not in Original: it knows of its zones what a Sampler does.)
                        let Some(smp) = snd.sampler.as_ref().filter(|_| layers::own(d.project(), g, s)) else { continue };
                        for (z, zone) in smp.zones.iter().enumerate() {
                            if let Some(job) = done.iter().find(|j| zone.meta.is_none() && j.file == zone.sample.path) {
                                let name = analysis::file_name_of(&zone.sample.path);
                                targets.push((g, s, z, analysis::meta_from(&job.analysis, &job.file, &name)));
                            }
                        }
                    }
                }
            }
            for (g, s, z, meta) in targets {
                let _ = doc.borrow_mut().note_zone_meta(g, s, z, &meta);
            }
        }
        let revision = doc.borrow().revision();
        if self.analysis_revision == Some(revision) {
            return;
        }
        self.analysis_revision = Some(revision);
        let mut wanted = Vec::new();
        {
            let d = doc.borrow();
            let own =
                |s: &&deckard_model::Sound| s.new_sampler.is_some_and(|n| n.mode != deckard_model::new_sampler::SamplerMode::Original);
            for snd in d.project().groups.iter().flat_map(|g| &g.sounds).filter(own) {
                for zone in snd.sampler.iter().flat_map(|x| &x.zones).filter(|z| z.meta.is_none() && !z.sample.path.is_empty()) {
                    wanted.push(zone.sample.clone());
                }
            }
        }
        for sample in wanted {
            if let Some(audio) = self.samples.sample(&sample, self.path.as_deref()) {
                self.analysis.want(&sample.path, &analysis::file_name_of(&sample.path), audio);
            }
        }
    }

    /// The Sync and Keys renders engines asked for go to the render thread; those made
    /// go to the samples, and playback is built again to play them (its voices carry on
    /// where the music is).
    fn follow_renders(&mut self) {
        for job in self.samples.wanted_renders() {
            self.renders.want(job);
        }
        let made = self.renders.poll();
        if made.is_empty() {
            return;
        }
        for (job, r) in made {
            self.samples.add_render(&job, r);
        }
        // Built in full: the sounds' zones change, not the document.
        self.built = None;
        self.seen_revision = u64::MAX;
    }

    /// A finished (or failed, or cancelled) stem separation.
    fn poll_stems(&mut self) {
        let Some(result) = self.stems.as_ref().and_then(stems::Job::poll) else { return };
        let job = self.stems.take().expect("the job polled");
        match result {
            Ok(stems) if job.into_lanes => self.finish_stem_lanes(&job.name, job.rate, stems, job.source),
            Ok(stems) => self.finish_stems(&job.name, job.rate, stems, job.beats),
            Err(_) if job.cancelled() => self.status = "stem separation cancelled".into(),
            Err(e) => self.status = format!("stem separation failed: {e}"),
        }
    }

    /// The stems as the zone's lanes (roadmap 2b): each saved as a sample ("<name>
    /// Drums", …) and put on zone `at` (still playing the sample they came from), all
    /// on. One undo step.
    fn finish_stem_lanes(&mut self, name: &str, rate: u32, stems: Vec<deckard_stems::Stem>, at: (usize, usize, usize)) {
        use deckard_model::new_sampler::StemLane;
        use deckard_stems::Kind;
        let Some(doc) = self.doc.clone() else { return };
        let meta = doc
            .borrow()
            .project()
            .groups
            .get(at.0)
            .and_then(|g| g.sounds.get(at.1))
            .and_then(|x| x.sampler.as_ref())
            .and_then(|x| x.zones.get(at.2))
            .and_then(|z| z.meta.clone());
        let Some(mut meta) = meta else {
            self.status = "the zone changed while its stems were made: they weren't kept".into();
            return;
        };
        // A root analysis wasn't sure of (none, or a guess) is heard again now there's a
        // bass to hear: the stems together are the sample.
        if !meta.root_locked
            && (meta.root.is_none() || meta.root_guess)
            && let Some(bass) = stems.iter().find(|s| s.kind == Kind::Bass)
        {
            let mut mix = vec![[0f32; 2]; bass.frames.len()];
            for s in &stems {
                for (m, f) in mix.iter_mut().zip(&s.frames) {
                    (m[0], m[1]) = (m[0] + f[0], m[1] + f[1]);
                }
            }
            if let Some(r) = deckard_analysis::root_by_bass(&mix, rate, Some(&bass.frames)) {
                (meta.root, meta.cents, meta.root_guess, meta.key_minor) = (Some(r.note), r.cents, r.guess, r.minor);
            }
        }
        let dir = self.sample_dir("Stems");
        for s in stems {
            let lane = match s.kind {
                Kind::Drums => 0,
                Kind::Vocals => 1,
                Kind::Bass => 2,
                Kind::Other => 3,
            };
            let path = dir.join(format!("{name} {}.wav", s.kind.name()).replace(['/', ':'], "-"));
            if let Err(e) = deckard_engine::edit::write_wav_channels(&path, &deckard_engine::sample::Sample { rate, frames: s.frames }, 2) {
                self.status = format!("can't save the stems: {e}");
                return;
            }
            meta.stems[lane] = Some(StemLane::new(path.to_string_lossy().into()));
        }
        meta.stems_only = false;
        match doc.borrow_mut().set_zone_meta(at.0, at.1, at.2, &meta, "Stems") {
            Ok(()) => self.status = format!("{name}: stems on its lanes (M and S on each)"),
            Err(e) => self.status = format!("can't put the stems on the zone: {e}"),
        }
    }

    /// Stems of your own on the focused New Sampler zone's lanes: up to four files at
    /// the sample's rate, each on the lane its name says (drums, vocals, bass; else
    /// Leads, or the next free). One shorter than the sample is padded with silence (a
    /// copy, in Stems); one longer is refused. One undo step.
    fn load_stems(&mut self, files: &[PathBuf]) {
        use deckard_model::new_sampler::StemLane;
        let Some(doc) = self.doc.clone() else { return };
        let Some((zi, zone, _, false)) = layout::sampling::edited(doc.borrow().project(), (self.group, self.sound, self.zone)) else {
            return;
        };
        let (Some(mut meta), Some(sample)) = (zone.meta.filter(|_| self.stems_onto_lanes()), self.sampling.sample.clone()) else {
            self.status = "stems go on a zone of a Sampler Preview in Advanced (once it's analysed)".into();
            return;
        };
        let mut taken: [bool; 4] = std::array::from_fn(|k| meta.stems[k].is_some());
        let mut left_out = Vec::new();
        for f in files {
            let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let stem = match deckard_engine::sample::Sample::load(f) {
                Ok(s) if s.rate != sample.rate => {
                    left_out.push(format!("{name} ({} Hz, the sample is {})", s.rate, sample.rate));
                    continue;
                }
                Ok(s) if s.frames.len() > sample.frames.len() => {
                    left_out.push(format!("{name} (longer than the sample)"));
                    continue;
                }
                Ok(s) => s,
                Err(e) => {
                    left_out.push(format!("{name} ({e})"));
                    continue;
                }
            };
            let Some(lane) = beat::lane_for(&name, &taken) else {
                left_out.push(format!("{name} (four lanes at most)"));
                continue;
            };
            let mut path = f.clone();
            if stem.frames.len() < sample.frames.len() {
                let mut frames = stem.frames;
                frames.resize(sample.frames.len(), [0.0; 2]);
                path = deckard_engine::edit::edited_path(&self.sample_dir("Stems"), f);
                if let Err(e) =
                    deckard_engine::edit::write_wav_channels(&path, &deckard_engine::sample::Sample { rate: stem.rate, frames }, 2)
                {
                    left_out.push(format!("{name} ({e})"));
                    continue;
                }
            }
            taken[lane] = true;
            meta.stems[lane] = Some(StemLane::new(path.to_string_lossy().into()));
        }
        if let Err(e) = doc.borrow_mut().set_zone_meta(self.group, self.sound, zi, &meta, "Load Stems") {
            self.status = format!("can't put the stems on the zone: {e}");
        } else if !left_out.is_empty() {
            self.status = format!("left out: {}", left_out.join(", "));
        }
    }

    /// A stem lane of the focused zone muted, soloed, set to a gain, or all removed.
    fn lane_act(&mut self, lane: usize, act: layout::sampling::LaneAct) {
        use layout::sampling::LaneAct as L;
        let Some(doc) = self.doc.clone() else { return };
        let Some((zi, zone, _, false)) = layout::sampling::edited(doc.borrow().project(), (self.group, self.sound, self.zone)) else {
            return;
        };
        let Some(mut meta) = zone.meta else { return };
        let label = match act {
            L::Clear => {
                meta.stems = Default::default();
                meta.stems_only = false;
                "Remove Stems"
            }
            L::Delete => {
                if meta.stems.get(lane).is_none_or(Option::is_none) {
                    return;
                }
                meta.stems[lane] = None;
                // What's left is all that plays; with none left, the sample again.
                meta.stems_only = meta.has_stems();
                "Delete Stem"
            }
            _ => {
                let Some(l) = meta.stems.get_mut(lane).and_then(Option::as_mut) else { return };
                match act {
                    L::Mute => {
                        l.mute = !l.mute;
                        "Stem Mute"
                    }
                    L::Solo => {
                        l.solo = !l.solo;
                        "Stem Solo"
                    }
                    L::Gain(g) => {
                        l.gain = g.clamp(0.0, 2.0);
                        "Stem Gain"
                    }
                    L::Clear | L::Delete => return,
                }
            }
        };
        if let Err(e) = doc.borrow_mut().set_zone_meta(self.group, self.sound, zi, &meta, label) {
            self.status = format!("can't change the stems: {e}");
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
        // (A take on no layer: all of it.)
        if self.shown_take().is_some() {
            return Some((0, self.sampling.len()));
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

    /// A New Sampler zone's meta after an edit of its sample: its stem lanes edited the
    /// same way by `lane` (each written as a new file), so the zone stays four waveforms
    /// cut alike; its tempo and grid follow (`stretched`: the tempo changes with the
    /// length, else the beats do; `shift`: frames taken off the front). `None`: the zone
    /// has no lanes, or one can't be carried (the edit then leaves the zone without).
    fn carry_lanes(
        &mut self,
        key: &(usize, usize, usize, String),
        new_len: usize,
        lane: &dyn Fn(&deckard_engine::sample::Sample) -> deckard_engine::sample::Sample,
        (shift, stretched): (usize, bool),
    ) -> Option<deckard_model::new_sampler::ZoneMeta> {
        let doc = self.doc.clone()?;
        let mut meta = {
            let d = doc.borrow();
            let snd = d.project().groups.get(key.0)?.sounds.get(key.1)?;
            snd.new_sampler.as_ref()?;
            snd.sampler.as_ref()?.zones.get(key.2)?.meta.clone().filter(|m| m.has_stems())?
        };
        let old_len = self.sampling.len();
        let rate = self.sampling.sample.as_ref()?.rate;
        let dir = self.sample_dir("Stems");
        let project = self.path.clone();
        for k in 0..deckard_model::new_sampler::LANES {
            let Some(l) = meta.stems[k].as_mut() else { continue };
            let sref = deckard_model::SampleRef { kind: 0, path: l.file.clone(), ..Default::default() };
            let src = self.samples.sample(&sref, project.as_deref()).filter(|x| x.frames.len() == old_len)?;
            let out = lane(&src);
            if out.frames.len() != new_len {
                return None;
            }
            let path = deckard_engine::edit::edited_path(&dir, Path::new(&l.file));
            deckard_engine::edit::write_wav(&path, &out).ok()?;
            l.file = path.to_string_lossy().into();
        }
        resized(&mut meta, old_len, new_len, rate, (shift, stretched));
        Some(meta)
    }

    /// A layer's meta for its edited file, `new_len` frames long: what it was, its tempo
    /// and beats following the new length. None: the zone isn't a layer.
    fn layer_meta_after(
        &self,
        key: &(usize, usize, usize, String),
        new_len: usize,
        how: (usize, bool),
    ) -> Option<deckard_model::new_sampler::ZoneMeta> {
        let doc = self.doc.clone()?;
        let d = doc.borrow();
        if !layers::can_layer(d.project(), key.0, key.1) {
            return None;
        }
        let mut meta =
            d.project().groups[key.0].sounds[key.1].sampler.as_ref()?.zones.get(key.2)?.meta.clone().filter(|m| m.layer.is_some())?;
        meta.stems = Default::default();
        meta.stems_only = false;
        let rate = self.sampling.sample.as_ref().map_or(0, |x| x.rate);
        resized(&mut meta, self.sampling.len(), new_len, rate, how);
        Some(meta)
    }

    /// Write `sample` as an edited copy of the focused zone's file and point the zone at
    /// it (one undo step). `lanes`: how the edit goes for a stem lane (see
    /// [`Self::carry_lanes`]); without it the zone's lanes don't follow. `meta`: what the
    /// zone knows of the new file (its tempo, say) and zone parameters to set on it (its
    /// play range, else all of the file), kept in the same step; else the file is looked
    /// at afresh.
    pub(crate) fn store_edit(
        &mut self,
        sample: deckard_engine::sample::Sample,
        label: &str,
        lanes: Option<(&dyn Fn(&deckard_engine::sample::Sample) -> deckard_engine::sample::Sample, (usize, bool))>,
        meta: Option<(deckard_model::new_sampler::ZoneMeta, [(&'static str, i64); 4])>,
    ) {
        let Some(doc) = self.doc.clone() else { return };
        let Some(key) = self.sampling.sample_key.clone() else { return };
        let original = PathBuf::from(&key.3);
        // A take on no layer: the take list has the edited copy (the project isn't
        // changed: the list isn't part of it), the original staying in Recordings.
        if let Some((i, take)) = self.shown_take() {
            let target = deckard_engine::edit::edited_path(&self.sample_dir("Recordings"), &take);
            match deckard_engine::edit::write_wav(&target, &sample) {
                Ok(()) => {
                    let v = &mut self.recorder.view;
                    v.take_waves.insert(target.clone(), recorder::mini_wave(&sample.frames));
                    v.takes[i] = target.clone();
                    self.status = format!("{label}: {}", target.display());
                    self.sampling.selection = None;
                }
                Err(e) => self.status = format!("can't write the edited take: {e}"),
            }
            return;
        }
        let target = deckard_engine::edit::edited_path(&self.sample_dir("Samples"), &original);
        if let Err(e) = deckard_engine::edit::write_wav(&target, &sample) {
            self.status = format!("can't write the edited sample: {e}");
            return;
        }
        let frames = sample.frames.len();
        let carried = lanes.and_then(|(lane, how)| self.carry_lanes(&key, frames, lane, how));
        // (The lanes are those carried: ones that couldn't be are left behind.)
        // A layer stays one: its settings go with it to the new file.
        let carried = carried.or_else(|| self.layer_meta_after(&key, frames, lanes.map_or((0, false), |l| l.1)));
        let (meta, params) = meta.unzip();
        let carried = match (meta, carried) {
            (Some(m), Some(c)) => Some(deckard_model::new_sampler::ZoneMeta { stems: c.stems, stems_only: c.stems_only, ..m }),
            (Some(m), None) => Some(deckard_model::new_sampler::ZoneMeta { stems: Default::default(), stems_only: false, ..m }),
            (None, c) => c,
        };
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
            doc.borrow_mut().transaction(label, |d| {
                d.set_zone_sample(key.0, key.1, key.2, &target, frames, label)?;
                // Its lanes, cut alike, on the new file.
                if let Some(mut meta) = carried {
                    meta.file = d.project().groups[key.0].sounds[key.1]
                        .sampler
                        .as_ref()
                        .map_or_else(String::new, |x| x.zones[key.2].sample.path.clone());
                    d.set_zone_meta(key.0, key.1, key.2, &meta, label)?;
                }
                for (field, v) in params.into_iter().flatten() {
                    d.set(label, deckard_doc::Target::Zone(key.0, key.1, key.2), field, mxprj::decode::Value::Int(v))?;
                }
                Ok(())
            })
        };
        match done {
            Ok(()) => {
                self.status = format!("{label}: {}", target.display());
                self.sampling.selection = None;
                // The take it was recorded as is this file now, in the list too.
                let v = &mut self.recorder.view;
                if let Some(t) = v.takes.iter_mut().find(|t| t.file_name() == original.file_name()) {
                    v.take_waves.insert(target.clone(), recorder::mini_wave(&sample.frames));
                    *t = target.clone();
                }
            }
            Err(e) => self.status = format!("can't edit: {e}"),
        }
    }

    /// Carry out a Sampling view action.
    fn sampling_act(&mut self, a: layout::sampling::SamplingAct) {
        use deckard_engine::edit;
        use layout::sampling::{EditOp, RecordAct, SamplingAct as S};
        // A take on no layer has no zone: only what works on its file, the recorder and
        // the layers.
        if self.shown_take().is_some() {
            let file_edit = match &a {
                S::Edit(op) => !matches!(op, EditOp::Stems | EditOp::AutoLoop | EditOp::Munge),
                S::Open(_) | S::Tab(_) | S::Stereo | S::View(_) | S::Select(_) => true,
                S::StretchOpen(_) | S::SetStretch(_) | S::StretchApply => true,
                S::Record(_) | S::Layer(..) | S::Morph(_) | S::Sampler(_) | S::SamplerAnswer(_) => true,
                _ => false,
            };
            if !file_edit {
                self.status = "give the take a layer first (RECORD: LAYER, then APPLY)".into();
                return;
            }
        }
        // A layer has no play mode, tempo or Motion of its own: those are Advanced's.
        if matches!(a, S::Meta(_) | S::PlayMode(_) | S::Tap(_) | S::ToProject(_) | S::Motion(_))
            && self.doc.as_ref().is_some_and(|d| layers::can_layer(d.borrow().project(), self.group, self.sound))
        {
            return;
        }
        // LAYERS (and any tab but RECORD and EDIT) leaves a take on no layer.
        if matches!(a, S::Tab(t) if t > 1) {
            self.take_shown = None;
        }
        match a {
            S::Sampler(mode) => self.ask_sampler_mode(mode, false),
            S::SamplerAnswer(ok) => self.answer_sampler_mode(ok),
            S::Open(on) => {
                self.sampling_open = on;
                // (The question is the open page's.)
                if !on {
                    self.sampler_ask = None;
                }
            }
            S::Tab(i) => {
                self.sampling.tab = i;
                if i == 2 && self.sampling.slices.is_empty() {
                    // A sample already sliced shows its slices (Manual keeps them).
                    match self.chops() {
                        Some((_, starts, _)) => {
                            self.sampling.slices = starts;
                            self.sampling.slice_mode = 3;
                            self.sampling.kept.clear();
                        }
                        None => self.compute_slices(),
                    }
                }
            }
            S::Stereo => self.sampling.stereo = !self.sampling.stereo,
            S::Audition(pitch) => self.audition_toggle(pitch),
            S::View(v) => self.sampling.view = v,
            S::Select(s) => {
                self.sampling.selected = s.is_some() && self.sampling.tab == 1;
                self.sampling.selection = s;
            }
            S::Edit(EditOp::AutoLoop) => self.auto_loop(),
            S::Edit(EditOp::Stems) => match &self.stems {
                Some(job) => job.cancel(),
                None => self.start_stems(false),
            },
            S::StemsToGroup => {
                if self.stems.is_none() {
                    self.start_stems(true);
                }
            }
            S::LoadStems => {
                if let Some(files) = rfd::FileDialog::new().add_filter("Audio", &deckard_engine::sample::EXTENSIONS).pick_files() {
                    self.load_stems(&files);
                }
            }
            S::Layer(lane, act) => self.layer_act(lane, act),
            S::Morph(act) => self.morph_act(act),
            S::Lane(k, act) => self.lane_act(k, act),
            S::LanesMix(on) => self.sampling.lanes_mix = on,
            S::DragOut(lane, whole) => self.drag_out(lane, whole),
            S::LaneHear(lane, root, tempo) => self.hear_lane(lane, root, tempo),
            S::Edit(op) => {
                let (Some(s), Some(r)) = (self.sampling.sample.clone(), self.edit_range()) else { return };
                type Lane = Box<dyn Fn(&deckard_engine::sample::Sample) -> deckard_engine::sample::Sample>;
                // The edit, and the same for a stem lane (`None`: the lanes can't follow).
                let (out, label, lane): (_, _, Option<Lane>) = match op {
                    EditOp::Truncate => (edit::truncate(&s, r), "Truncate", Some(Box::new(move |l| edit::truncate(l, r)))),
                    EditOp::Normalize => {
                        let out = edit::normalize(&s, r);
                        // The lanes by as much as the sample was (each to its own peak would
                        // change their balance).
                        let peak = |x: &deckard_engine::sample::Sample| {
                            x.frames
                                .get(r.0..r.1.min(x.frames.len()))
                                .map_or(0.0, |f| f.iter().flat_map(|f| [f[0].abs(), f[1].abs()]).fold(0.0, f32::max))
                        };
                        let gain = if peak(&s) > 0.0 { peak(&out) / peak(&s) } else { 1.0 };
                        let lane: Lane = Box::new(move |l| {
                            let mut l = l.clone();
                            let end = r.1.min(l.frames.len());
                            for f in l.frames.get_mut(r.0..end).into_iter().flatten() {
                                *f = [f[0] * gain, f[1] * gain];
                            }
                            l
                        });
                        (out, "Normalize", Some(lane))
                    }
                    EditOp::Reverse => (edit::reverse(&s, r), "Reverse", Some(Box::new(move |l| edit::reverse(l, r)))),
                    EditOp::FadeIn => (edit::fade(&s, r, true), "Fade In", Some(Box::new(move |l| edit::fade(l, r, true)))),
                    EditOp::FadeOut => (edit::fade(&s, r, false), "Fade Out", Some(Box::new(move |l| edit::fade(l, r, false)))),
                    EditOp::Dc => (edit::remove_dc(&s, r), "DC Correction", Some(Box::new(|l| l.clone()))),
                    EditOp::Silence => (edit::silence(&s, r), "Silence", Some(Box::new(move |l| edit::silence(l, r)))),
                    EditOp::Copy => {
                        self.sampling.clipboard = Some(edit::copy(&s, r));
                        self.status = "copied".into();
                        return;
                    }
                    EditOp::Cut => {
                        let (rest, clip) = edit::cut(&s, r);
                        self.sampling.clipboard = Some(clip);
                        (rest, "Cut", Some(Box::new(move |l| edit::cut(l, r).0)))
                    }
                    EditOp::Paste => {
                        let Some(clip) = self.sampling.clipboard.clone() else { return };
                        // What's pasted has no stems of its own.
                        (edit::paste(&s, r.0, &clip), "Paste", None)
                    }
                    EditOp::Duplicate => (edit::duplicate(&s, r), "Duplicate", Some(Box::new(move |l| edit::duplicate(l, r)))),
                    EditOp::Stretch | EditOp::Munge | EditOp::AutoLoop | EditOp::Stems => return,
                };
                let shift = if op == EditOp::Truncate { r.0 } else { 0 };
                self.store_edit(out, label, lane.as_deref().map(|l| (l, (shift, false))), None);
            }
            S::MungeOpen(on) => {
                self.sampling.munge = on.then(Default::default);
                self.sampling.stretch = None;
            }
            S::SetMunge(m) => self.sampling.munge = Some(m),
            S::MungeApply => self.munge_apply(),
            S::MungeHear => self.munge_hear(),
            S::StretchOpen(on) => {
                self.sampling.munge = None;
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
                let (factor, tune) = (sx.factor(), sx.tune);
                let lane = move |l: &deckard_engine::sample::Sample| edit::stretch(l, r, factor, tune);
                self.store_edit(out, "Stretch", Some((&lane, (0, true))), None);
            }
            S::SliceMode(m) => {
                self.sampling.slice_mode = m;
                self.compute_slices();
            }
            S::SplitCount(i) => {
                self.sampling.split = i;
                self.compute_slices();
            }
            S::FindCount(i) => {
                self.sampling.find = i.min(layout::sampling::FIND_COUNTS.len() - 1);
                self.compute_slices();
            }
            S::RandomCount(i) => {
                self.sampling.random = i.min(layout::sampling::FIND_COUNTS.len() - 1);
                self.compute_slices();
            }
            S::Reroll => {
                self.sampling.seed = self.sampling.seed.wrapping_add(1);
                self.compute_slices();
            }
            S::KeepSlice(i) => self.keep_slice(i),
            S::Catch(free) => self.catch_slice(free),
            S::KeyShift(i) => self.key_shift(i),
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
            S::PlaySliceAt(f) => {
                // To the next slice, or the end of what is being sliced.
                let s = &self.sampling.slices;
                if let Some(i) = s.iter().rposition(|&a| a <= f) {
                    let selection = self.sampling.selection.take();
                    let end = self.edit_range().map_or(self.sampling.len(), |r| r.1);
                    self.sampling.selection = selection;
                    let (a, b) = (s[i], s.get(i + 1).copied().unwrap_or(end));
                    if b > a {
                        self.sampling_call(deckard_lua_host::state::SamplingCall::Prehear(a, b));
                    }
                }
            }
            S::RemoveSliceAt(f) => {
                // The slice containing `f` joins the previous one.
                if let Some(i) = self.sampling.slices.iter().rposition(|&s| s <= f).filter(|&i| i > 0) {
                    let gone = self.sampling.slices.remove(i);
                    self.sampling.kept.retain(|k| *k != gone);
                }
            }
            S::MoveSlice(i, f) => {
                let s = &mut self.sampling.slices;
                if i > 0 && i < s.len() {
                    let lo = s[i - 1] + 1;
                    let hi = s.get(i + 1).map_or(usize::MAX, |n| n - 1);
                    let was = std::mem::replace(&mut s[i], f.clamp(lo, hi));
                    // (Kept, it stays kept where it goes.)
                    let now = s[i];
                    if let Some(k) = self.sampling.kept.iter_mut().find(|k| **k == was) {
                        *k = now;
                    }
                }
            }
            S::ClearSlices => {
                self.sampling.slices.clear();
                self.sampling.kept.clear();
            }
            S::PatternMode(m) => self.sampling.pattern_mode = m,
            S::ApplyTo(k) => self.sampling.apply_to = k.min(1),
            S::NoteOrder(on) => self.sampling.shuffle = on,
            S::ExportSlices => self.export_slices(),
            S::SliceToEnd(on) => self.sampling.slice_to_end = Some(on),
            S::ApplySlices => self.apply_slices(),
            S::FocusZone(i) => self.zone = i,
            S::ZoneMap(on) => self.sampling.zone_map = on,
            S::ZoneList(on) => self.sampling.zone_list = on,
            S::AddZones => self.add_zones(),
            S::Meta(meta) => {
                let Some(doc) = self.doc.clone() else { return };
                let p = doc.borrow().project().clone();
                let Some((zi, _, _, false)) = layout::sampling::edited(&p, (self.group, self.sound, self.zone)) else { return };
                if let Err(e) = doc.borrow_mut().set_zone_meta(self.group, self.sound, zi, &meta, "Zone Tempo") {
                    self.status = format!("can't change the zone: {e}");
                }
            }
            S::Tap(t) => {
                let taps = &mut self.sampling.taps;
                if taps.last().is_some_and(|l| t - l > beat::TAP_GAP || t < *l) {
                    taps.clear();
                }
                taps.push(t);
                let Some(bpm) = beat::tap_bpm(taps) else { return };
                let Some(doc) = self.doc.clone() else { return };
                let p = doc.borrow().project().clone();
                let Some((zi, zone, _, false)) = layout::sampling::edited(&p, (self.group, self.sound, self.zone)) else { return };
                let (Some(meta), Some(s)) = (zone.meta, self.sampling.sample.as_ref()) else { return };
                let meta = beat::with_bpm(&meta, bpm, s.frames.len() as f64 / s.rate.max(1) as f64);
                // The taps of one count are one undo step.
                let label = "Tap Tempo";
                let mut d = doc.borrow_mut();
                if self.sampling.taps.len() > 2 && d.undo_label() == Some(label) {
                    d.undo();
                }
                if let Err(e) = d.set_zone_meta(self.group, self.sound, zi, &meta, label) {
                    self.status = format!("can't change the zone: {e}");
                }
            }
            S::Warp(w) => self.warp_act(w),
            S::Reanalyze => self.reanalyze(),
            S::ToProject(bpm) => {
                let Some(doc) = self.doc.clone() else { return };
                if let Err(e) = doc.borrow_mut().set_tempo(bpm) {
                    self.status = format!("can't set the tempo: {e}");
                }
            }
            S::PlayMode(mode) => {
                let Some(doc) = self.doc.clone() else { return };
                let p = doc.borrow().project().clone();
                let Some((zi, zone, _, false)) = layout::sampling::edited(&p, (self.group, self.sound, self.zone)) else { return };
                let Some(meta) = zone.meta else { return };
                let meta = deckard_model::new_sampler::ZoneMeta { mode, ..meta };
                if let Err(e) = doc.borrow_mut().set_zone_meta(self.group, self.sound, zi, &meta, "Play Mode") {
                    self.status = format!("can't change how the zone plays: {e}");
                }
            }
            S::Plays(plays) => self.set_plays(plays),
            S::Motion(motion) => self.set_motion(motion),
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
                RecordAct::Start => {
                    if self.session.active() && self.recorder.view.source == 2 {
                        self.recorder.view.error =
                            Some("Can't record from an internal source while a session recording is running. Stop it first.".into());
                    } else {
                        self.start_recording();
                    }
                }
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
                RecordAct::Set(layout::sampling::RecParam::Layer, v) if !self.recorder.busy() => {
                    self.recorder.view.choose_layer((self.group, self.sound), v)
                }
                RecordAct::Set(p, v) if !self.recorder.busy() => self.recorder.view.set(p, v),
                RecordAct::Threshold(t) if !self.recorder.busy() => self.recorder.view.threshold = t.clamp(-60.0, 0.0),
                RecordAct::MidiTo(m) if !self.recorder.busy() => self.recorder.view.auto.midi_to = m,
                RecordAct::ScanMidi => self.recorder.scan_midi(),
                RecordAct::Set(..) | RecordAct::Threshold(_) | RecordAct::MidiTo(_) => {}
                RecordAct::FocusTake(i) => {
                    self.recorder.view.focused = Some(i);
                    self.recorder.view.layer_choice = None;
                    let (g, s) = (self.group, self.sound);
                    // In Layers the take picked is what EDIT works on: its layer, if it
                    // is on one; else the take itself, until it has one.
                    let on = self.doc.as_ref().and_then(|d| {
                        let d = d.borrow();
                        let take = self.recorder.view.takes.get(i)?;
                        layers::can_layer(d.project(), g, s).then(|| layers::take_layer(d.project(), g, s, take))
                    });
                    match on {
                        Some(Some(lane)) => (self.zone, self.take_shown) = (lane, None),
                        Some(None) => self.take_shown = Some((g, s)),
                        None => self.take_shown = None,
                    }
                    // A Sampler Preview with a sample keeps it: a take clicked is only
                    // focused, and its menu puts it on a layer. [DECKARD's choice]
                    let kept = self.doc.as_ref().is_some_and(|d| !layers::layers_in(d.borrow().project(), g, s).is_empty());
                    if let Some(p) = self.recorder.view.takes.get(i).cloned().filter(|_| !kept) {
                        self.place_take(g, s, &p);
                    }
                }
                RecordAct::ApplyLayer => self.apply_take_layer(),
                RecordAct::TakeToLayer(i, lane) => {
                    self.recorder.view.focused = Some(i);
                    self.recorder.view.layer_choice = None;
                    if let Some(p) = self.recorder.view.takes.get(i).cloned() {
                        let (g, s) = (self.group, self.sound);
                        self.take_to_layer(g, s, &p, lane);
                    }
                }
                RecordAct::DeleteTake(i) => {
                    if i < self.recorder.view.takes.len() {
                        let path = self.recorder.view.takes.remove(i);
                        self.recorder.view.take_waves.remove(&path);
                        self.unload_take(&path);
                    }
                    self.recorder.view.focused = None;
                    self.recorder.view.layer_choice = None;
                }
            },
        }
    }

    /// The focused New Sampler zone's tempo and root again, heard in the part selected
    /// (else the part it plays): a stretch of a song may have a tempo and a key of its
    /// own. Its root stays if set by hand; its warp markers go (they were the old tempo's).
    fn reanalyze(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let Some(s) = self.sampling.sample.clone() else { return };
        let p = doc.borrow().project().clone();
        let Some((zi, zone, _, false)) = layout::sampling::edited(&p, (self.group, self.sound, self.zone)) else { return };
        let Some(meta) = zone.meta else { return };
        let (a, b) = self.sampling.selection.or_else(|| self.play_range_of_focused()).unwrap_or((0, s.frames.len()));
        let (a, b) = (a.min(s.frames.len()), b.min(s.frames.len()));
        if b <= a || s.rate == 0 {
            return;
        }
        let mut found = deckard_analysis::analyse(&s.frames[a..b], s.rate, "");
        // Its bass lane, if it has one, settles a key that's a close call.
        if let Some(l) = self.sampling.lanes[BASS_LANE].as_ref().filter(|l| l.sample.frames.len() >= b) {
            found.root = deckard_analysis::root_by_bass(&s.frames[a..b], s.rate, Some(&l.sample.frames[a..b]));
        }
        let seconds = s.frames.len() as f64 / f64::from(s.rate);
        let mut m = meta.clone();
        if let Some(t) = found.tempo {
            m = deckard_model::new_sampler::ZoneMeta {
                bpm_source: deckard_model::new_sampler::BpmSource::Audio,
                grid_offset: (a + t.offset) as u64,
                warps: Vec::new(),
                ..beat::with_bpm(&m, t.bpm, seconds)
            };
        }
        if let Some(r) = found.root.filter(|_| !m.root_locked) {
            (m.root, m.cents, m.root_guess, m.key_minor) = (Some(r.note), r.cents, r.guess, r.minor);
        }
        self.status = match (found.tempo, found.root) {
            (None, None) => "Reanalyze: no tempo or root heard there".into(),
            (t, r) => format!(
                "Reanalyze: {}{}",
                t.map_or("no tempo".into(), |t| format!("{:.2} BPM", t.bpm)),
                r.map_or(String::new(), |r| format!(", root {}{}", analysis::note_name(r.note), if r.guess { "?" } else { "" }))
            ),
        };
        if m != meta
            && let Err(e) = doc.borrow_mut().set_zone_meta(self.group, self.sound, zi, &m, "Reanalyze")
        {
            self.status = format!("can't change the zone: {e}");
        }
    }

    /// The focused zone's root and/or tempo as stem lane `lane` alone has them (the part
    /// selected, else the part the zone plays): a root set by hand stays. One undo step.
    fn hear_lane(&mut self, lane: usize, root: bool, tempo: bool) {
        let Some(doc) = self.doc.clone() else { return };
        let name = deckard_model::new_sampler::LANE_NAMES.get(lane).copied().unwrap_or("stem");
        let Some(l) = self.sampling.lanes.get(lane).and_then(Option::as_ref).map(|l| l.sample.clone()) else {
            self.status = format!("no {name} stem here");
            return;
        };
        let p = doc.borrow().project().clone();
        let Some((zi, zone, _, false)) = layout::sampling::edited(&p, (self.group, self.sound, self.zone)) else { return };
        let Some(meta) = zone.meta else { return };
        let (a, b) = self.sampling.selection.or_else(|| self.play_range_of_focused()).unwrap_or((0, l.frames.len()));
        let (a, b) = (a.min(l.frames.len()), b.min(l.frames.len()));
        if b <= a || l.rate == 0 {
            return;
        }
        let part = &l.frames[a..b];
        let mut m = meta.clone();
        let mut said = Vec::new();
        if tempo {
            match deckard_analysis::tempo(part, l.rate, None) {
                Some(t) => {
                    let seconds = l.frames.len() as f64 / f64::from(l.rate);
                    m = deckard_model::new_sampler::ZoneMeta {
                        bpm_source: deckard_model::new_sampler::BpmSource::Audio,
                        grid_offset: (a + t.offset) as u64,
                        warps: Vec::new(),
                        ..beat::with_bpm(&m, t.bpm, seconds)
                    };
                    said.push(format!("{:.2} BPM", t.bpm));
                }
                None => said.push("no tempo heard".into()),
            }
        }
        if root {
            match deckard_analysis::root(part, l.rate) {
                Some(_) if m.root_locked => said.push("the root is set by hand: it stays".into()),
                Some(r) => {
                    (m.root, m.cents, m.root_guess, m.key_minor) = (Some(r.note), r.cents, r.guess, r.minor);
                    said.push(format!("root {}{}", analysis::note_name(r.note), if r.guess { "?" } else { "" }));
                }
                None => said.push("no root heard".into()),
            }
        }
        self.status = format!("From the {name} stem: {}", said.join(", "));
        if m != meta
            && let Err(e) = doc.borrow_mut().set_zone_meta(self.group, self.sound, zi, &m, "Analyse Stem")
        {
            self.status = format!("can't change the zone: {e}");
        }
    }

    /// Slice positions for the current mode (Manual keeps the ones set by hand).
    fn compute_slices(&mut self) {
        use deckard_engine::edit;
        use layout::sampling::{FIND_COUNTS, GRID_DIVS, SPLIT_COUNTS};
        let Some(s) = self.sampling.sample.clone() else { return };
        let r = self.slice_range().unwrap_or((0, s.frames.len()));
        // Kept slices are Find's and Random's, and only where slicing still reaches.
        if !matches!(self.sampling.slice_mode, 4 | 5) {
            self.sampling.kept.clear();
        }
        self.sampling.kept.retain(|k| (r.0..r.1).contains(k));
        // Split and Grid go from the playhead on while the sample plays, or from where a
        // selection starts (a sliced sample's is its focused slice: not that).
        let selection = self.sampling.selection.filter(|_| self.sampling.selected && self.chops().is_none());
        // (PLAY's playhead: not a slice pad's, nor the song's notes.)
        let playhead = self.sampling.playhead.filter(|_| self.audition.is_some());
        let from = chop::slicer_range(r, playhead, selection);
        let st = &self.sampling;
        let slices = match st.slice_mode {
            0 => edit::split_positions(from, SPLIT_COUNTS[st.split]),
            1 => {
                // A New Sampler zone with a tempo: on its own beats. Else at the song's.
                let map = self.doc.as_ref().and_then(|d| {
                    let d = d.borrow();
                    let snd = d.project().groups.get(self.group)?.sounds.get(self.sound)?;
                    snd.new_sampler?;
                    let m = snd.sampler.as_ref()?.zones.get(self.zone)?.meta.as_ref()?;
                    deckard_model::new_sampler::BeatMap::of(m, s.rate)
                });
                match map {
                    Some(map) => beat::grid_slices(&map, from, GRID_DIVS[st.grid].0),
                    None => {
                        let tempo = self.doc.as_ref().map_or(120.0, |d| d.borrow().project().tempo);
                        edit::grid_positions(s.rate, from, tempo, GRID_DIVS[st.grid].0)
                    }
                }
            }
            2 => edit::detect_positions(&s, r, st.sensitivity, st.zero_x),
            4 => {
                // Where things start: on the sample's own bars (or beats, of a short one)
                // when the zone has a tempo, else on its hits.
                let count = FIND_COUNTS[st.find.min(FIND_COUNTS.len() - 1)];
                let lines = self.find_lines(s.rate, r, count);
                let mut found = deckard_analysis::find_samples_keeping(&s.frames, s.rate, r, &lines, count, &st.kept);
                if found.is_empty() {
                    found.push(r.0);
                }
                found
            }
            5 => {
                // Any of the places Find weighs, the kept ones staying.
                let count = FIND_COUNTS[st.random.min(FIND_COUNTS.len() - 1)];
                let mut picked = chop::random_picks(&self.slice_candidates(&s, r, count), &st.kept, count, st.seed, s.rate as usize / 25);
                if picked.is_empty() {
                    picked.push(r.0);
                }
                picked
            }
            _ => {
                if st.slices.is_empty() {
                    vec![r.0]
                } else {
                    return;
                }
            }
        };
        self.sampling.slices = slices;
    }

    /// The focused sound's zones of the focused zone's file, when there's more than one
    /// (the sample was sliced): the frames they play between them, their starts in
    /// order, and each start's zone.
    fn chops(&self) -> Option<((usize, usize), Vec<usize>, Vec<usize>)> {
        let doc = self.doc.as_ref()?.borrow();
        let zones = &doc.project().groups.get(self.group)?.sounds.get(self.sound)?.sampler.as_ref()?.zones;
        let file = &zones.get(self.zone)?.sample;
        let len = self.sampling.len();
        let mut of: Vec<(usize, usize, usize)> = zones
            .iter()
            .enumerate()
            .filter(|(_, z)| &z.sample == file && z.play_range.1 > z.play_range.0)
            .map(|(i, z)| (z.play_range.0.max(0) as usize, (z.play_range.1 as usize + 1).min(len), i))
            .collect();
        of.sort_unstable();
        of.dedup_by_key(|z| z.0);
        (of.len() > 1).then(|| {
            let span = (of[0].0, of.iter().map(|z| z.1).max().unwrap_or(len));
            (span, of.iter().map(|z| z.0).collect(), of.iter().map(|z| z.2).collect())
        })
    }

    /// The frames slicing works on: all a sliced sample's slices play, else the focused
    /// zone's play range.
    fn slice_range(&self) -> Option<(usize, usize)> {
        self.chops().map(|c| c.0).or_else(|| self.play_range_of_focused())
    }

    fn play_range_of_focused(&self) -> Option<(usize, usize)> {
        let doc = self.doc.as_ref()?.borrow();
        let z = doc.project().groups.get(self.group)?.sounds.get(self.sound)?.sampler.as_ref()?.zones.get(self.zone)?.clone();
        let len = self.sampling.len();
        Some(if z.play_range.1 > z.play_range.0 { (z.play_range.0 as usize, (z.play_range.1 as usize + 1).min(len)) } else { (0, len) })
    }

    /// What a note does with the focused New Sampler slice. A one-shot that would play on
    /// to the sample's end stops at the next slice instead (its E marker moves from there);
    /// a loop with no frame of its own takes the slice (its start to the next one's); held
    /// again, a slice cut short at the next one plays on to the end as it did.
    fn set_plays(&mut self, plays: deckard_model::new_sampler::Plays) {
        use deckard_model::new_sampler::{Plays, ZoneMeta};
        use mxprj::decode::Value;
        let Some(doc) = self.doc.clone() else { return };
        let p = doc.borrow().project().clone();
        let (g, s) = (self.group, self.sound);
        let Some((zi, zone, _, false)) = layout::sampling::edited(&p, (g, s, self.zone)) else { return };
        let Some(meta) = zone.meta.clone() else { return };
        let Some(zones) = p.groups.get(g).and_then(|x| x.sounds.get(s)).and_then(|x| x.sampler.as_ref()).map(|x| &x.zones) else { return };
        let len = self.sampling.len() as i64;
        if len < 2 {
            return;
        }
        let (lo, hi) = zone.play_range;
        let (a, b) = if hi > lo { (lo.max(0), hi.min(len - 1)) } else { (0, len - 1) };
        // Where this slice ends: the next one's start, or the sample's end.
        let next = zones
            .iter()
            .filter(|z| z.sample == zone.sample && z.play_range.1 > z.play_range.0)
            .map(|z| z.play_range.0)
            .filter(|&x| x > a)
            .min();
        let slice_end = next.map_or(len - 1, |n| n - 1).max(a + 1);
        let mut set: Vec<(&str, f64)> = Vec::new();
        // No longer a one-shot, it plays on past the next slice again.
        let mut b = b;
        if plays != Plays::Once && meta.once && b == slice_end {
            b = len - 1;
            set.push(("play_range/high", b as f64));
        }
        match plays {
            Plays::Held => set.push(("loop_active", 0.0)),
            Plays::Once => {
                set.push(("loop_active", 0.0));
                if b >= len - 1 {
                    set.push(("play_range/high", slice_end as f64));
                }
            }
            Plays::Loop => {
                let (la, lb) = zone.loop_range;
                // A frame of its own: inside what it plays, and not the whole sample's.
                let own = lb > la && la >= a && lb <= b && !(la <= 0 && lb >= len - 1 && slice_end < len - 1);
                if !own {
                    set.push(("loop_range/low", a as f64));
                    set.push(("loop_range/high", slice_end.min(b) as f64));
                }
                set.push(("loop_active", 1.0));
            }
        }
        let meta = ZoneMeta { once: plays == Plays::Once, ..meta };
        let label = "Slice Plays";
        let done = doc.borrow_mut().transaction(label, |d| {
            for (field, v) in &set {
                d.set(label, deckard_doc::Target::Zone(g, s, zi), field, Value::Float(*v))?;
            }
            d.set_zone_meta(g, s, zi, &meta, label)
        });
        if let Err(e) = done {
            self.status = format!("can't change how the slice plays: {e}");
        }
    }

    /// APPLY: one zone per slice on the same sample, keys from C3 (`SLICE_KEY`, where a pad plays); with
    /// "Create/Replace Pattern", a note per slice at its time in the focused pattern.
    fn apply_slices(&mut self) {
        /// The first slice's key: C3, the key a pad plays.
        const SLICE_KEY: usize = 60;
        let Some(doc) = self.doc.clone() else { return };
        let Some(s) = self.sampling.sample.clone() else { return };
        let end = self.slice_range().map_or(s.frames.len(), |r| r.1);
        let slices = deckard_engine::edit::slices_from(&self.sampling.slices, end);
        if slices.len() < 2 {
            return;
        }
        if self.sampling.apply_to == 1 {
            return self.apply_slices_to_pads(&s, &slices, end);
        }
        let zi = self.zone;
        // Sliced before: a slice still where it was keeps its zone's settings (its lanes'
        // mutes, its tune), a new or moved one takes the focused zone's.
        let was = self.chops().map(|c| (c.1, c.2)).unwrap_or_default();
        let zone_of = |a: usize| was.0.iter().position(|&s| s == a).map_or(zi, |k| was.1[k]);
        let (g, snd) = (self.group, self.sound);
        // Start points: each slice plays on to the end, for as long as its pad is held.
        let new_sampler = layers::own(doc.borrow().project(), g, snd);
        let to_end = self.sampling.slices_to_end(new_sampler);
        // (Before the zones change: a Sync zone's notes go by its own tempo.)
        let ticks_per_frame = self.slice_ticks_per_frame(doc.borrow().project(), s.rate);
        let shuffle_seed = self.shuffle_seed();
        let specs: Vec<deckard_doc::ZoneSpec> = slices
            .iter()
            .enumerate()
            .map(|(i, &(a, b))| {
                let b = if to_end { end } else { b };
                let key = (SLICE_KEY + i).min(127) as f64;
                deckard_doc::ZoneSpec {
                    from: deckard_doc::ZoneFrom::Zone(zone_of(a)),
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
        let done = doc.borrow_mut().transaction("Apply Slices", |d| {
            d.set_zones(g, snd, &specs, "Apply Slices")?;
            // A one-shot would play on to the end whatever the pad does: the notes end it.
            let one_shot = d.project().groups[g].sounds[snd].sampler.as_ref().is_some_and(|x| x.envelope_type == 0);
            if to_end && one_shot {
                d.set("Apply Slices", deckard_doc::Target::Sampler(g, snd), "envelope_type", mxprj::decode::Value::Float(2.0))?;
                d.set("Apply Slices", deckard_doc::Target::Sampler(g, snd), "volume_envelope/release", mxprj::decode::Value::Float(0.1))?;
            }
            Ok(())
        });
        if let Err(e) = done {
            self.status = format!("can't apply the slices: {e}");
            return;
        }
        self.zone = 0;
        self.sampling.slices.clear();
        self.sampling.kept.clear();
        self.sampling.tab = 3;
        if self.sampling.pattern_mode < 2 {
            // A note per slice, at the slice's time at the song tempo.
            let mut d = doc.borrow_mut();
            let p = d.project().clone();
            let pi = self.pattern;
            let Some(pat) = p.groups.get(g).and_then(|gr| gr.patterns.get(pi)) else { return };
            let a0 = slices[0].0;
            // (Shuffled: a note's time is its own, the slice it plays another's.)
            let order = chop::note_order(slices.len(), self.sampling.shuffle, shuffle_seed);
            let add: Vec<Note> = slices
                .iter()
                .zip(&order)
                .map(|(&(a, _), &k)| Note {
                    ratchet: Default::default(),
                    expr: None,
                    sound: snd,
                    position: pat.start + ((a - a0) as f64 * ticks_per_frame) as i64,
                    length: (((slices[k].1 - slices[k].0) as f64 * ticks_per_frame) as i64).max(1),
                    pitch: (SLICE_KEY + k).min(127) as i64,
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
        let Some(files) = rfd::FileDialog::new().add_filter("Audio", &deckard_engine::sample::EXTENSIONS).pick_files() else { return };
        // (In Layers what comes in is a layer.)
        if layers::can_layer(doc.borrow().project(), self.group, self.sound) {
            let at = layers::layers_in(doc.borrow().project(), self.group, self.sound).len();
            return self.layer_files(at, &files);
        }
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
        egui::Window::new("Note Repeat / Arp")
            .open(&mut open)
            .resizable(false)
            .max_height(crate::layout::dialog_max_height(ctx))
            .max_width(crate::layout::dialog_max_width(ctx))
            .vscroll(true)
            .show(ctx, |ui| {
                crate::layout::dialog_style(ui);
                ui.horizontal(|ui| {
                    ui.toggle_value(&mut on, "On")
                        .on_hover_text("Held pads (and ⌘K's keys) repeat; the Arp with the pads in keyboard mode");
                    ui.toggle_value(&mut lock, "Lock").on_hover_text("Stays on outside the MK3's NOTE REPEAT page");
                    ui.separator();
                    let shown = view.knobs.iter().find(|k| k.key == "arp/preset").map_or(0, |k| k.value as usize);
                    // Presets 1–4 (item 0 is none).
                    for (i, name) in view
                        .lists
                        .iter()
                        .find(|(k, _)| k == "arp/preset")
                        .map(|x| x.1.clone())
                        .unwrap_or_default()
                        .iter()
                        .enumerate()
                        .skip(1)
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
                                let slider =
                                    egui::Slider::new(&mut v, k.min..=k.max).step_by(k.step).custom_formatter(|_, _| k.text.clone());
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
        // (Maschine's scripts have no LAYERS: they show ZONE for it.)
        let app_tab = self.sampling.tab.min(3);
        if self.hw_sampling_tab.is_some_and(|t| t != hw_tab) {
            self.sampling_act(layout::sampling::SamplingAct::Tab(hw_tab));
        } else if app_tab != hw_tab {
            tab = Some(app_tab);
        }
        self.hw_sampling_tab = Some(self.sampling.tab.min(3));
        let at = (self.group, self.sound, self.zone);
        let pages = self.doc.as_ref().map(|d| {
            let d = d.borrow();
            let record = record_knobs::knobs(&self.recorder.view, &self.settings, d.project(), (self.group, self.sound), false, None);
            (
                sampling_hw::pages(record, &self.sampling, d.project(), at, false),
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
            for (t, pages) in k.tabs.into_iter().take(4).enumerate() {
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
                if a == R::Sampler {
                    let mode = deckard_model::new_sampler::SamplerMode::ALL[at.min(2)];
                    return self.ask_sampler_mode(mode, true);
                }
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
                    R::Sampler => return,
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
                // (A take on no layer: all of it, not the layer's range behind it.)
                let whole = self.shown_take().map(|_| (0, len));
                let (a, b) = self.sampling.selection.or(whole).unwrap_or_else(|| self.play_range_of_focused().unwrap_or((0, len)));
                let v = value.round().max(0.0) as usize;
                let (a, b) = if end { (a, v.max(a + 1)) } else { (v.min(b.saturating_sub(1)), b) };
                SamplingAct::Select(Some((a, b.min(len))))
            }
            Some(Apply::SliceMode) => SamplingAct::SliceMode(sampling_hw::app_slice_mode(value)),
            Some(Apply::SplitCount) => SamplingAct::SplitCount(at),
            Some(Apply::FindCount) => SamplingAct::FindCount(at),
            Some(Apply::RandomCount) => SamplingAct::RandomCount(at),
            Some(Apply::GridDiv) => SamplingAct::GridDiv(at),
            Some(Apply::Sensitivity) => SamplingAct::Sensitivity(value.clamp(0.0, 1.0)),
            Some(Apply::ZeroX) => SamplingAct::ZeroX(value >= 0.5),
            Some(Apply::PatternMode) => SamplingAct::PatternMode(at),
            Some(Apply::ApplyTo) => SamplingAct::ApplyTo(at),
            Some(Apply::NoteOrder) => SamplingAct::NoteOrder(value >= 0.5),
            Some(Apply::LaneGain(k)) => {
                let gain = if value <= -60.0 { 0.0 } else { 10f64.powf(value.min(6.0) / 20.0) };
                SamplingAct::Lane(k, layout::sampling::LaneAct::Gain(gain))
            }
            Some(Apply::Layer(k)) => {
                use layout::sampling::LayerAct as L;
                use sampling_hw::{LayerKnob as K, time_at};
                let lane = self.zone;
                SamplingAct::Layer(
                    lane,
                    match k {
                        K::Pan => L::Pan(value),
                        K::Tune => L::Tune(value),
                        K::Root => L::Root(value.round() as i64),
                        K::Attack => L::Attack(time_at(at)),
                        K::Decay => L::Decay(time_at(at)),
                        K::Sustain => L::Sustain(value),
                        K::Release => L::Release(time_at(at)),
                        K::Window(point) => L::Window(point, value),
                    },
                )
            }
            Some(Apply::Morph(k)) => {
                use deckard_model::layers::{Fade, Shape};
                use layout::sampling::MorphAct as M;
                use sampling_hw::MorphKnob as K;
                SamplingAct::Morph(match k {
                    K::On => M::On(value >= 0.5),
                    K::Position => M::Position(value),
                    K::Rate => match sampling_hw::rate_at(at) {
                        (true, bars) => M::Bars(bars),
                        (false, seconds) => M::Seconds(seconds),
                    },
                    K::Shape => M::Shape(if value >= 0.5 { Shape::Back } else { Shape::Round }),
                    K::Depth => M::Depth(value),
                    K::Reset => M::Reset(value >= 0.5),
                    K::Fade => M::Fade(if value >= 0.5 { Fade::Power } else { Fade::Linear }),
                })
            }
            Some(Apply::WarpGrid) => SamplingAct::Warp(layout::sampling::WarpAct::Grid(at)),
            Some(Apply::WarpStrength) => SamplingAct::Warp(layout::sampling::WarpAct::Strength(value.clamp(0.0, 1.0))),
            Some(Apply::LoopLength) => {
                let Some(doc) = self.doc.clone() else { return };
                let Some((_, z, t, false)) = layout::sampling::edited(doc.borrow().project(), (self.group, self.sound, self.zone)) else {
                    return;
                };
                let Some(rate) = self.sampling.sample.as_ref().map(|s| s.rate) else { return };
                let (unit, step, _) = sampling_hw::loop_unit(z.meta.as_ref().and_then(|m| m.bpm), rate);
                // On the step's lines: a loop of an odd length takes the nearest.
                let n = ((value / step).round().max(1.0) * step * unit).round() as i64;
                let last = if z.play_range.1 > z.play_range.0 { z.play_range.1 } else { self.sampling.len() as i64 - 1 };
                let end = (z.loop_range.0 + n - 1).clamp(z.loop_range.0 + 1, last.max(z.loop_range.0 + 1));
                layout::apply_acts(self, vec![layout::Act::SetParam(t, "loop_range/high", end as f64)]);
                return;
            }
            Some(Apply::Plays) => SamplingAct::Plays(deckard_model::new_sampler::Plays::ALL[at.min(2)]),
            Some(Apply::Motion) => SamplingAct::Motion(deckard_model::new_sampler::Motion::ALL[at.min(2)]),
            Some(
                a @ (Apply::PlayMode
                | Apply::Bpm
                | Apply::Beats
                | Apply::Root
                | Apply::PlaysAt
                | Apply::StartGrid
                | Apply::Speed
                | Apply::Reverse
                | Apply::Tail),
            ) => {
                use deckard_model::new_sampler::PlayMode;
                let Some(doc) = self.doc.clone() else { return };
                let Some((_, z, t, false)) = layout::sampling::edited(doc.borrow().project(), (self.group, self.sound, self.zone)) else {
                    return;
                };
                let Some(m) = z.meta else { return };
                let seconds = self.sampling.sample.as_ref().map_or(0.0, |s| s.frames.len() as f64 / s.rate.max(1) as f64);
                match a {
                    Apply::PlayMode => {
                        let mode = [PlayMode::Repitch, PlayMode::Sync, PlayMode::Keys][at.min(2)];
                        if mode != PlayMode::Repitch && m.bpm.is_none() {
                            return;
                        }
                        SamplingAct::PlayMode(mode)
                    }
                    Apply::Bpm => SamplingAct::Meta(beat::with_bpm(&m, value, seconds)),
                    Apply::Beats => match beat::with_beats(&m, sampling_hw::beats_turned(m.beats, value), seconds) {
                        Some(m) => SamplingAct::Meta(m),
                        None => return,
                    },
                    Apply::Root => SamplingAct::Meta(beat::with_root(&m, at.min(127) as u8)),
                    Apply::Speed => {
                        SamplingAct::Meta(deckard_model::new_sampler::ZoneMeta { speed: (value.round() / 100.0).clamp(0.25, 4.0), ..m })
                    }
                    Apply::Reverse => SamplingAct::Meta(deckard_model::new_sampler::ZoneMeta { reverse: value >= 0.5, ..m }),
                    Apply::Tail => {
                        use deckard_model::new_sampler::{TAIL_MAX, ZoneMeta};
                        SamplingAct::Meta(ZoneMeta { tail: ((value / 10.0).round() * 10.0).clamp(0.0, TAIL_MAX), ..m })
                    }
                    Apply::StartGrid => {
                        use deckard_model::new_sampler::{StartGrid, ZoneMeta};
                        SamplingAct::Meta(ZoneMeta { start: StartGrid::ALL[at.min(StartGrid::ALL.len() - 1)], ..m })
                    }
                    _ => {
                        let Some(r) = m.root else { return };
                        let tune = beat::tune_for(r, m.cents, at.min(127) as i64).clamp(-36.0, 36.0);
                        layout::apply_acts(self, vec![layout::Act::SetParam(t, "tune", tune)]);
                        return;
                    }
                }
            }
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
                    self.sampling.kept.clear();
                }
                return;
            }
            C::RemoveSlice(i) => {
                if i > 0 && i < self.sampling.slices.len() {
                    self.sampling.slices.remove(i);
                    self.sampling.slice_mode = 3;
                    self.sampling.kept.clear();
                }
                return;
            }
            C::ClearSlices => S::ClearSlices,
            C::CatchSlice(free) => S::Catch(free),
            C::KeepSlice(i) => S::KeepSlice(i),
            C::KeyShift(i) => S::KeyShift(i),
            C::Reroll => S::Reroll,
            C::MoveSlice(i, f) => S::MoveSlice(i, f),
            C::ApplySlices => S::ApplySlices,
            C::RemoveZone(i) => S::RemoveZone(i),
            C::FocusZone(i) => S::FocusZone(i),
            C::FocusLayer(k) => S::Layer(k, layout::sampling::LayerAct::Focus),
            C::LayerMute(k) => S::Layer(k, layout::sampling::LayerAct::Mute),
            C::LayerSolo(k) => S::Layer(k, layout::sampling::LayerAct::Solo),
            C::RemoveLayer(k) => S::Layer(k, layout::sampling::LayerAct::Remove),
            C::ApplyTakeLayer => S::Record(layout::sampling::RecordAct::ApplyLayer),
            C::SamplerMode(m) => S::Sampler(deckard_model::new_sampler::SamplerMode::ALL[usize::from(m).min(2)]),
            C::SamplerAnswer(ok) => S::SamplerAnswer(ok),
            C::LayerLevel(v) => S::Layer(self.zone, layout::sampling::LayerAct::Level(v)),
            C::SpreadLayers => S::Morph(layout::sampling::MorphAct::Spread),
            C::LaneMute(k) => S::Lane(k, layout::sampling::LaneAct::Mute),
            C::LaneSolo(k) => S::Lane(k, layout::sampling::LaneAct::Solo),
            C::Warp(k) => {
                use layout::sampling::WarpAct as W;
                S::Warp([W::Detect, W::Quantize, W::Clear][k.min(2)])
            }
            C::Convert => {
                let Some(doc) = self.doc.clone() else { return };
                let to_new = doc
                    .borrow()
                    .project()
                    .groups
                    .get(self.group)
                    .and_then(|x| x.sounds.get(self.sound))
                    .is_some_and(|x| x.new_sampler.is_none());
                self.convert_instrument(to_new);
                return;
            }
            C::Reanalyze => {
                self.reanalyze();
                return;
            }
            C::LaneHear(lane, root, tempo) => {
                self.hear_lane(lane, root, tempo);
                return;
            }
            C::MatchKey => {
                let Some(doc) = self.doc.clone() else { return };
                let Some((_, z, t, false)) = layout::sampling::edited(doc.borrow().project(), (self.group, self.sound, self.zone)) else {
                    return;
                };
                let Some((r, cents)) = z.meta.and_then(|m| Some((m.root?, m.cents))) else { return };
                let tune = beat::match_tune(r, cents, self.scale.0).clamp(-36.0, 36.0);
                layout::apply_acts(self, vec![layout::Act::SetParam(t, "tune", tune)]);
                return;
            }
            C::ScaleBpm(_) | C::ToProject => {
                let Some(doc) = self.doc.clone() else { return };
                let Some((_, z, _, false)) = layout::sampling::edited(doc.borrow().project(), (self.group, self.sound, self.zone)) else {
                    return;
                };
                let Some((m, bpm)) = z.meta.and_then(|m| Some((m.clone(), m.bpm?))) else { return };
                match c {
                    C::ScaleBpm(up) => {
                        let seconds = self.sampling.sample.as_ref().map_or(0.0, |s| s.frames.len() as f64 / s.rate.max(1) as f64);
                        S::Meta(beat::with_bpm(&m, if up { bpm * 2.0 } else { bpm / 2.0 }, seconds))
                    }
                    _ => S::ToProject(bpm),
                }
            }
            // A slice's pad: hear the slice (C63).
            C::Prehear(a, b) => {
                if let Some(part) = self.sampling_part(a, b) {
                    self.ensure_engine();
                    // As loud as the sound is in the mix: its level, its group's, the master's
                    // (the prehear itself goes round all three).
                    let gain = self.doc.as_ref().map_or(1.0, |d| {
                        let d = d.borrow();
                        let p = d.project();
                        let g = p.groups.get(self.group);
                        let s = g.and_then(|g| g.sounds.get(self.sound));
                        deckard_model::level_gain(p.master_level) * g.map_or(1.0, |g| g.level) * s.map_or(1.0, |s| s.level)
                    });
                    if let Some(Ok(out)) = &self.output {
                        out.prehear(Some(std::sync::Arc::new(part)), gain as f32);
                        self.prehear_from = Some((std::time::Instant::now(), a, b.min(self.sampling.len())));
                    }
                }
                return;
            }
        };
        self.sampling_act(act);
    }

    /// Arm a session recording of the master output (pick the file and open the Recorder
    /// window, metering but not yet writing — Play begins it), or stop/cancel a live one.
    /// Refuses when the Sampling recorder already holds the engine tap (mutual exclusion).
    fn toggle_session_record(&mut self) {
        if self.session.active() {
            self.stop_session_record();
            return;
        }
        let Some(rate) = self.session_record_rate() else { return };
        let project = self.path.as_ref().and_then(|p| p.file_stem()).map(|n| n.to_string_lossy().into_owned());
        let default = session_record::session_filename(project.as_deref(), &session_record::session_stamp());
        let mut dialog = rfd::FileDialog::new().add_filter("WAV", &["wav"]).set_file_name(default);
        if let Some(dir) = &self.settings.session_dir {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else { return };
        self.arm_session_record(path, rate);
    }

    /// The rate a session recording would be made at, if one can start now: not while
    /// Sampling records an internal source (it holds the engine's one tap), and only with
    /// an engine to tap. Says why not.
    fn session_record_rate(&mut self) -> Option<u32> {
        if self.recorder.busy() && self.recorder.is_internal_active() {
            self.error = Some("Can't start a session recording while Sampling is recording from an internal source. Stop it first.".into());
            return None;
        }
        // The tap is the engine's: without one (nothing played yet) it would be dropped, and
        // the whole recording would be silence.
        self.ensure_engine();
        match &self.output {
            Some(Ok(o)) if o.has_engine() => Some(o.rate),
            Some(Ok(_)) => {
                self.error = Some("Nothing to record a session from: open a project first.".into());
                None
            }
            _ => {
                self.error = Some("No audio output to record from.".into());
                None
            }
        }
    }

    /// Arm a session recording to `path`, at the output's `rate`.
    fn arm_session_record(&mut self, path: PathBuf, rate: u32) {
        let reserve = self.settings.session_reserve_mb as u64 * 1024 * 1024;
        let free = session_record::free_bytes(path.parent().unwrap_or(&path)).unwrap_or(u64::MAX);
        if !session_record::can_start(free, reserve) {
            self.error = Some("Not enough free disk space to start a session recording.".into());
            return;
        }
        let output = match &self.output {
            Some(Ok(o)) => Some(o),
            _ => None,
        };
        // Sampling only metering an internal source makes way: its meter lets the tap go,
        // and takes it up again when the session ends.
        if self.recorder.is_internal_active() {
            self.recorder.cancel(output);
        }
        // Arm: attach the master tap now so the window meters live, but write nothing until Play.
        let buffer = self.session.attach_tap();
        self.session.arm(path.clone(), rate, reserve);
        if let Some(o) = output {
            o.set_tap(Some(deckard_engine::Tap { source: deckard_engine::TapSource::Master, buffer }));
        }
        self.settings.session_dir = path.parent().map(Path::to_path_buf);
        self.settings.save();
        self.session_window_open = true;
        // Surface the time-available estimate once (the OS save dialog can't carry it); the
        // window then shows it live once recording starts.
        let left = session_record::seconds_available(free, reserve, rate);
        self.status = format!("Session armed — press Play ({})", session_window::format_time_left(left));
    }

    /// Resume/begin recording, or pause it, from the window's Play/Pause control.
    fn toggle_session_play(&mut self) {
        match self.session.phase() {
            session_record::SessionPhase::Armed | session_record::SessionPhase::Paused => {
                self.session.play(session_record::started_label());
            }
            session_record::SessionPhase::Recording => self.session.pause(),
            session_record::SessionPhase::Idle => {}
        }
    }

    /// Stop and finalize a session recording (or cancel an armed one), releasing the tap and
    /// closing the window.
    fn stop_session_record(&mut self) {
        self.session.stop();
        if let Some(Ok(o)) = &self.output {
            o.set_tap(None);
        }
        self.session_window_open = false;
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
                self.recorder.view.layer_choice = None;
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
                        self.take_into(g, s, &path, sample.frames.len());
                        // In Layers, a take that waits for its layer is what EDIT works
                        // on meanwhile (as one picked in the list).
                        let waits = self.doc.as_ref().is_some_and(|d| {
                            let d = d.borrow();
                            layers::can_layer(d.project(), g, s) && layers::take_layer(d.project(), g, s, &path).is_none()
                        });
                        if waits && (g, s) == (self.group, self.sound) {
                            self.take_shown = Some((g, s));
                        }
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
        let note = Note {
            ratchet: Default::default(),
            expr: None,
            sound: s,
            position: pat.start,
            length: ticks,
            pitch: 60,
            velocity: 100,
            probability: 100,
        };
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
        // (A sound in Layers has no key-mapped zones: the notes go to the next free sound.)
        let has_sampler = doc.borrow().project().groups.get(g).and_then(|x| x.sounds.get(from)).is_some_and(|x| x.sampler.is_some())
            && !layers::can_layer(doc.borrow().project(), g, from);
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

    fn forget_group_presets(&mut self, g: usize) {
        use deckard_doc::FileItem;
        self.saved.retain(|item, _| !matches!(item, FileItem::Group(group) | FileItem::Sound(group, _) if *group == g));
    }

    // Loads and resets retain the document slot's ID. Undo or redo can
    // change which preset occupies that ID.
    fn forget_replaced_presets(&mut self, label: Option<&str>) {
        use deckard_doc::FileItem;
        match label {
            Some("Load Group" | "Paste Group" | "Clear Group" | "Clear rig group") => {
                self.saved.retain(|item, _| !matches!(item, FileItem::Group(_) | FileItem::Sound(..)));
            }
            Some("Load Sound" | "Cut Sound" | "Paste Sound" | "Paste Sounds" | "Reset Sound" | "Reset Sounds") => {
                self.saved.retain(|item, _| !matches!(item, FileItem::Sound(..)));
            }
            _ => {}
        }
    }

    /// Remember an editable preset only after its load succeeds.
    fn remember_preset_source(
        &mut self,
        what: deckard_doc::FileItem,
        item: &deckard_library::Item,
        info: Option<mxprj::sound_info::SoundInfo>,
    ) {
        self.saved.remove(&what);
        let Some(info) = info else { return };
        let id = self.doc.as_ref().and_then(|d| d.borrow().file_item_id(what));
        if id.is_some() {
            self.saved.insert(what, Saved { id, path: item.path.clone(), info, samples: false, preview: false });
        }
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
            preview: !self.preview_declined,
            confirm_overwrite: false,
            error: None,
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
            dlg.error = Some("It needs a name.".into());
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
        let saved = Saved { id, path, info, samples: dlg.samples, preview: dlg.preview };
        if matches!(what, deckard_doc::FileItem::Sound(..)) {
            self.preview_declined = !saved.preview;
        }
        if self.write_item(what, saved) {
            self.save_dialog = None;
        } else if let Some(d) = &mut self.save_dialog {
            // The status line is easy to miss behind the dialog.
            d.error = Some(self.status.clone());
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
                if s.preview
                    && let deckard_doc::FileItem::Sound(g, n) = what
                {
                    self.start_preview(g, n, &s.path);
                }
                self.saved.insert(what, s);
                true
            }
            Err(e) => {
                self.status = format!("can't save {}: {e}", s.path.display());
                false
            }
        }
    }

    /// Render sound (`g`, `n`)'s audio preview beside its saved file `file`, for the
    /// browser to prehear; the engine is built on the main thread (so plug-ins can load)
    /// and rendered on a worker thread so saving doesn't wait for it.
    fn start_preview(&mut self, g: usize, n: usize, file: &Path) {
        self.store_plugin_states();
        let Some(doc) = &self.doc else { return };
        let (p, path, rate) = (doc.borrow().project().clone(), self.path.clone(), self.render_rate());
        let mut cache = SampleCache::default();
        cache.library_dirs = self.samples.library_dirs.clone();
        let engine = deckard_engine::export::preview_engine(&p, path.as_deref(), (g, n), rate, &mut cache);
        let out = browser::preview_path(file);
        // Nobody waits for it: the file appearing is the result.
        drop(export::Offline::start("Preview", move |_, cancel| {
            let audio = deckard_engine::export::render_preview_engine(engine, rate, cancel).ok_or("cancelled")?;
            browser::write_preview(&out, &audio, rate)
        }));
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
            B::User(u) => q.user = u,
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
                self.stop_prehear();
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
                    let key = item.favorite_key();
                    b.set_favorite(&key, on);
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
            B::StopPrehear => {
                self.stop_prehear();
                return;
            }
            B::PrehearFile(path) => {
                b.file_selected = Some(path.clone());
                if let Some(item) = browser::item_of(&path) {
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
                self.stop_prehear();
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

    /// Stop whatever the browser is prehearing.
    fn stop_prehear(&mut self) {
        self.prehear_generation = self.prehear_generation.wrapping_add(1);
        self.prehear_task = None;
        if let Some(Ok(out)) = &self.output {
            out.prehear(None, 0.0);
        }
    }

    fn poll_prehear(&mut self) {
        let Some((generation, receiver)) = self.prehear_task.take() else { return };
        match receiver.try_recv() {
            Ok(result) if generation == self.prehear_generation => match result {
                Ok(sample) => {
                    self.open_output_now();
                    if let Some(Ok(out)) = &self.output {
                        out.prehear(Some(std::sync::Arc::new(sample)), self.browser.prehear_gain);
                    }
                }
                Err(e) => self.status = e,
            },
            Ok(_) => {}
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                self.prehear_task = Some((generation, receiver));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
        }
    }

    /// The MK3 left its Browse page: what it was prehearing stops.
    fn stop_prehear_after_browse(&mut self) {
        let browsing = self.surface.page() == "browse";
        if self.surface_was_browsing && !browsing {
            self.stop_prehear();
        }
        self.surface_was_browsing = browsing;
    }

    pub(crate) fn prehear_item(&mut self, item: &deckard_library::Item) {
        self.prehear_from = None;
        let Some(file) = browser::prehear_file(item) else {
            self.stop_prehear();
            return;
        };
        self.stop_prehear();
        self.open_output_now();
        let generation = self.prehear_generation;
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = deckard_engine::sample::Sample::load(&file).map_err(|e| format!("can't prehear {}: {e}", file.display()));
            let _ = sender.send(result);
        });
        self.prehear_task = Some((generation, receiver));
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
                if let Some(res) =
                    deckard_plugins::nks::native_preset_file(path, item.uuid.as_deref(), item.kind == Kind::Effect, &self.catalog.entries)
                {
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
                if matches!(item.kind, Kind::Sound) {
                    self.remember_preset_source(deckard_doc::FileItem::Sound(g, s), item, preset_load::editable_preset_info(item, &bytes));
                }
                if matches!(item.kind, Kind::Group) {
                    self.forget_group_presets(g);
                    self.remember_preset_source(deckard_doc::FileItem::Group(g), item, preset_load::editable_preset_info(item, &bytes));
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
                    // (In Layers what comes in is a layer.)
                    if layers::can_layer(d.project(), g, s) {
                        layers::make_layered(d, g, s, "Load Sample")?;
                    }
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
        // A sound in Layers takes them as layers more.
        let layers = layers::layers_in(doc.borrow().project(), g, s).len();
        if layers::can_layer(doc.borrow().project(), g, s) {
            self.sound = s;
            self.layer_files(layers, files);
            return;
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
            // A sound with layers keeps them: the take is one more.
            (2, _) => match self.doc.as_ref().map_or(0, |d| layers::layers_in(d.borrow().project(), g, s).len()) {
                0 => self.take_into(g, s, path, smp.frames.len()),
                have => self.take_to_layer(g, s, path, have),
            },
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
            if let (2, Some(p)) = (self.prop_set, self.sound_groove_page(chain)) {
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
        if let (2, true, deckard_doc::Chain::Sound(g, s), Some(doc)) = (self.prop_set, self.channel, chain, &self.doc) {
            doc.borrow_mut().set_groove_page(g, s, self.prop_page.min(2));
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
        let (loop_start, loop_end) = self.song_loop.unwrap_or((0, 0));
        let mut t = format!(
            "song {}\ntab {tab}\nmixer {}\nmixer_sounds {}\npiano_roll {}\nprops {}\nsong_loop {} {} {}\n",
            self.song_view as u8,
            self.mixer as u8,
            self.mixer_sounds as u8,
            self.piano_roll as u8,
            self.prop_set,
            self.looping as u8,
            loop_start,
            loop_end,
        );
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
                Some("song_loop") => {
                    self.looping = flag(1).unwrap_or(false);
                    if let (Some(start), Some(end)) =
                        (w.get(2).and_then(|v| v.parse::<i64>().ok()), w.get(3).and_then(|v| v.parse::<i64>().ok()))
                        && end > start
                    {
                        self.song_loop = Some((start.max(0), end));
                    }
                }
                Some("mixer") => self.mixer = flag(1).unwrap_or(self.mixer),
                Some("mixer_sounds") => self.mixer_sounds = flag(1).unwrap_or(self.mixer_sounds),
                Some("piano_roll") => self.piano_roll = flag(1).unwrap_or(self.piano_roll),
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

    /// The Groove page sound `chain` was left on (kept with the sound), if it was.
    fn sound_groove_page(&self, chain: deckard_doc::Chain) -> Option<usize> {
        let deckard_doc::Chain::Sound(g, s) = chain else { return None };
        self.doc.as_ref()?.borrow().groove_page(g, s)
    }

    /// The page property set `set` of the chain shown was left on (0 if never shown).
    pub(crate) fn kept_prop_page(&self, set: usize) -> usize {
        let chain = self.chain();
        if let (2, Some(p)) = (set, self.sound_groove_page(chain)) {
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
                Ok(g) if Some(g.id.cid) == cid => (view_pages(&g, &saved, self.surface.mapping_pages(&g.name).as_deref()), sidechain(&g)),
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
                        (view_pages(&g, &saved, self.surface.mapping_pages(&g.name).as_deref()), sidechain(&g))
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
    pub(crate) fn midi_input(&self, group: usize) -> Option<std::sync::Arc<deckard_engine::midi_input::Info>> {
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
        let count = doc.borrow().project().groups.get(self.group).map_or(0, |gr| gr.sounds.len());
        let mut d = doc.borrow_mut();
        let done = match r {
            layout::RouteSet::Sound(g, s, route) => {
                if let Some(sounds) = self.selected_sounds_for(g, s, count) {
                    d.transaction("Sound Routing", |d| {
                        for &si in &sounds {
                            let self_route = matches!(
                                &route,
                                deckard_doc::SoundRoute::Output(deckard_model::AudioDest::Sound(dg, ds))
                                    | deckard_doc::SoundRoute::Aux(_, deckard_model::AudioDest::Sound(dg, ds))
                                    if (*dg, *ds) == (g, si)
                            );
                            if !self_route {
                                d.set_sound_route(g, si, route.clone())?;
                            }
                        }
                        Ok(())
                    })
                } else {
                    d.set_sound_route(g, s, route)
                }
            }
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
        let settings: Vec<(String, f64)> =
            GROOVE_FIELDS.iter().filter_map(|f| doc.borrow().groove_setting(g, s, f).map(|v| (f.to_string(), v))).collect();
        let get = |f: &str| settings.iter().find(|(k, _)| k == f).map(|(_, v)| *v);
        // The scale selected (the pattern editor's, which is the controller's Keyboard
        // scale too; Chromatic is none): Generate keeps to it.
        let (root, bank, kind) = self.scale;
        let in_scale = ((bank, kind) != (0, 0)).then_some(move |p: i64| deckard_lua_host::api::scales::in_scale(bank, kind, root, p));
        let in_scale: Option<&dyn Fn(i64) -> bool> = in_scale.as_ref().map(|f| f as &dyn Fn(i64) -> bool);
        // What the sound has there now, so Apply can say when it changed nothing (a
        // Humanize at 100–127 and no time shift does exactly that, as Maschine's does).
        let sound_notes = |d: &deckard_doc::Document| -> Vec<deckard_model::Note> {
            d.project()
                .groups
                .get(g)
                .and_then(|x| x.patterns.get(p))
                .map_or(Vec::new(), |pat| pat.notes.iter().filter(|n| n.sound == s).copied().collect())
        };
        let before = sound_notes(&doc.borrow());
        let done = match b {
            // The page's one action: Humanize varies what is there, Random and Euclid run
            // the generator — the same code the Jam's Apply asks for.
            layout::GrooveButton::ApplyVariation => doc.borrow_mut().apply_groove_page(g, p, s, grid, polyphonic, in_scale, &get),
            layout::GrooveButton::Generate => doc.borrow_mut().generate(g, p, s, grid, polyphonic, in_scale, &get),
        };
        match done {
            Err(e) => self.status = format!("can't do that: {e}"),
            Ok(()) if matches!(b, layout::GrooveButton::ApplyVariation) && sound_notes(&doc.borrow()) == before => {
                self.status = "Apply changed nothing: mind the mode, the ranges and the time shift".into();
            }
            Ok(()) => {}
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
        let settings: Vec<(String, f64)> =
            GROOVE_FIELDS.iter().filter_map(|f| doc.borrow().groove_setting(g, s, f).map(|v| (f.to_string(), v))).collect();
        let get = |f: &str| settings.iter().find(|(k, _)| k == f).map(|(_, v)| *v);
        let grid = self.grid;
        let polyphonic = self.piano_roll;
        // The scale Random keeps to (the controller's Keyboard scale too; Chromatic is none).
        let (root, bank, kind) = self.scale;
        let in_scale = ((bank, kind) != (0, 0)).then_some(move |p: i64| deckard_lua_host::api::scales::in_scale(bank, kind, root, p));
        let in_scale: Option<&dyn Fn(i64) -> bool> = in_scale.as_ref().map(|f| f as &dyn Fn(i64) -> bool);
        if let Err(e) = doc.borrow_mut().apply_groove_page_from(g, p, s, grid, polyphonic, in_scale, &get, &base) {
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
            if cmd(egui::Key::A) {
                self.select_all_sounds();
            }
            if !self.sound_selection.is_empty() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.select_no_sounds();
            }
            let arrow = |m: egui::Modifiers, k: egui::Key| ctx.input_mut(|i| i.consume_key(m, k));
            if arrow(egui::Modifiers::SHIFT, egui::Key::ArrowUp) {
                if s > 0 {
                    acts.push(Act::ClickSound { sound: s - 1, shift: true, toggle: false });
                }
            } else if arrow(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                if s > 0 {
                    acts.push(Act::ClickSound { sound: s - 1, shift: false, toggle: false });
                }
            } else if arrow(egui::Modifiers::SHIFT, egui::Key::ArrowDown) {
                if s + 1 < self.group_len() {
                    acts.push(Act::ClickSound { sound: s + 1, shift: true, toggle: false });
                }
            } else if arrow(egui::Modifiers::NONE, egui::Key::ArrowDown) && s + 1 < self.group_len() {
                acts.push(Act::ClickSound { sound: s + 1, shift: false, toggle: false });
            }
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
        // Stopped with the playhead put in the pattern: ⌘V pastes there.
        let mode = if song_view { Mode::Arrangement } else { Mode::Scene(scene) };
        let head = self.parked.filter(|h| self.placed_head && self.playing.is_none() && h.mode == mode);
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
            let plen = layout::pattern_len(p, g, pi).max(1);
            let at = match (at, head) {
                (clipboard::PasteAt::Keys, Some(h)) => clipboard::PasteAt::Playhead(parked_tick(p, h, plen)),
                (at, _) => at,
            };
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
        self.doc.as_ref()?.borrow().groove_setting(g, s, field)
    }

    /// Ask the background watcher to scan MIDI ports again (e.g. for virtual ports).
    pub(crate) fn refresh_midi_ports(&self) {
        if let Some(w) = &self.midi_port_watch {
            w.request_refresh();
        }
    }

    /// The MIDI ports, kept current by the background watcher (or probed once if no
    /// watcher was started).
    fn midi_ports(&mut self) -> &(Vec<String>, Vec<String>) {
        if let Some(w) = &self.midi_port_watch {
            if let Some(ports) = w.try_take() {
                self.midi_ports = ports;
            }
        } else if self.midi_ports_at.is_none() {
            self.midi_ports_at = Some(std::time::Instant::now());
            self.midi_ports = midi_ports::probe();
        }
        &self.midi_ports
    }

    fn copy_plugin(&mut self, chain: deckard_doc::Chain, at: usize) {
        let Some(doc) = self.doc.clone() else { return };
        let key = self.plugin_slot_key(chain, at);
        self.store_plugin_states_of(|slot| *slot == key);
        match doc.borrow().copy_slot(chain, at) {
            Ok(copy) => {
                self.plugin_clipboard = Some(copy);
                self.status = "Copied plug-in".into();
            }
            Err(e) => self.status = format!("Can't copy plug-in: {e}"),
        }
    }

    fn paste_plugin(&mut self, chain: deckard_doc::Chain, over: Option<usize>) {
        let Some(doc) = self.doc.clone() else { return };
        self.store_plugin_states();
        let Some(copy) = &self.plugin_clipboard else { return };
        let pasted = match over {
            Some(at) => doc.borrow_mut().paste_over_instrument(chain, at, copy).map(|()| at),
            None => doc.borrow_mut().paste_slot(chain, copy),
        };
        match pasted {
            Ok(at) => {
                self.editors.retain(|(c, ..)| *c != chain);
                self.plugin_views.clear();
                layout::apply_acts(self, vec![layout::Act::FocusChainSlot(chain, at)]);
            }
            Err(e) => self.status = format!("Can't paste plug-in: {e}"),
        }
    }

    /// Move slot `from` of the focused chain to `to`; it stays shown.
    fn move_slot(&mut self, from: usize, to: usize) {
        self.move_slot_in(self.chain(), from, to);
    }

    /// Move slot `from` of chain `c` to `to`; if `c` is focused, the moved slot stays shown.
    fn move_slot_in(&mut self, c: deckard_doc::Chain, from: usize, to: usize) {
        self.move_slot_between(c, from, c, to);
    }

    /// Move slot `from_at` of chain `from` to `to_at` of chain `to`; if `to` is focused,
    /// the moved slot stays shown.
    fn move_slot_between(&mut self, from: deckard_doc::Chain, from_at: usize, to: deckard_doc::Chain, to_at: usize) {
        let Some(doc) = self.doc.clone() else { return };
        // Plug-in states edited in their windows first (instances follow positions),
        // and both chains' windows close.
        self.store_plugin_states();
        self.editors.retain(|(ec, ..)| *ec != from && *ec != to);
        match doc.borrow_mut().move_slot_between(from, from_at, to, to_at) {
            Ok(()) => {
                if self.chain() == to {
                    self.slot = Some(to_at);
                    self.slot_of = Some(to);
                    self.plugin_page = 0;
                }
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
        let mut out = vec![
            internal("Sampler", true, AddWhat::Sampler),
            internal("Sampler Preview", true, AddWhat::NewSampler),
            internal("Audio", true, AddWhat::Audio),
        ];
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

    /// The focused New Sampler zone's warp markers (roadmap 1e): one undo step each.
    fn warp_act(&mut self, w: layout::sampling::WarpAct) {
        use deckard_model::new_sampler::{Warp, quantize, warps_at};
        use layout::sampling::WarpAct as W;
        match w {
            W::Strength(v) => return self.sampling.warp_strength = v,
            W::Grid(k) => return self.sampling.warp_grid = k.min(beat::WARP_GRIDS.len() - 1),
            _ => {}
        }
        let Some(doc) = self.doc.clone() else { return };
        let Some((zi, zone, _, false)) = layout::sampling::edited(doc.borrow().project(), (self.group, self.sound, self.zone)) else {
            return;
        };
        let (Some(meta), Some(sample)) = (zone.meta.clone(), self.sampling.sample.clone()) else { return };
        let mut warps = meta.warps.clone();
        let label = match w {
            W::Detect => {
                let len = sample.frames.len();
                let (lo, hi) = if zone.play_range.1 > zone.play_range.0 {
                    (zone.play_range.0 as usize, zone.play_range.1 as usize + 1)
                } else {
                    (0, len)
                };
                // Every hit (detection merges those closer than 50 ms).
                let hits = deckard_engine::edit::detect_positions(&sample, (lo, hi.min(len)), 1.0, false);
                warps = warps_at(&meta, sample.rate, &hits);
                "Warp Markers"
            }
            W::Quantize => {
                let (div, _) = beat::WARP_GRIDS[self.sampling.warp_grid.min(beat::WARP_GRIDS.len() - 1)];
                warps = quantize(&warps, div, self.sampling.warp_strength);
                "Quantize Audio"
            }
            W::Clear => {
                warps.clear();
                "Clear Warp Markers"
            }
            W::Add(frame) => {
                let Some(map) = deckard_model::new_sampler::BeatMap::of(&meta, sample.rate) else { return };
                let at = warps.partition_point(|x| x.frame < frame as u64);
                if warps.get(at).is_some_and(|x| x.frame == frame as u64) {
                    return;
                }
                warps.insert(at, Warp { frame: frame as u64, beat: map.beat_at(frame as f64) });
                "Add Warp Marker"
            }
            W::Remove(i) => {
                if i >= warps.len() {
                    return;
                }
                warps.remove(i);
                "Remove Warp Marker"
            }
            W::Move(i, beat) => {
                // Between its neighbours' beats.
                let lo = i.checked_sub(1).and_then(|k| warps.get(k)).map_or(f64::MIN, |x| x.beat + 1e-3);
                let hi = warps.get(i + 1).map_or(f64::MAX, |x| x.beat - 1e-3);
                let Some(x) = warps.get_mut(i) else { return };
                x.beat = beat.clamp(lo, hi.max(lo));
                "Move Warp Marker"
            }
            W::Strength(_) | W::Grid(_) => return,
        };
        let meta = deckard_model::new_sampler::ZoneMeta { warps, ..meta };
        if let Err(e) = doc.borrow_mut().set_zone_meta(self.group, self.sound, zi, &meta, label) {
            self.status = format!("can't change the warp markers: {e}");
        }
    }

    /// The focused sound's instrument made a New Sampler (`to_new`: a Sampler keeps its
    /// zones; an Audio plug-in's takes become zones in Sync at their tempos), or a New
    /// Sampler made a plain Sampler (roadmap 1f). One undo step.
    fn convert_instrument(&mut self, to_new: bool) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, s) = (self.group, self.sound);
        let Some(snd) = doc.borrow().project().groups.get(g).and_then(|x| x.sounds.get(s)).cloned() else { return };
        let result = if !to_new {
            let synced = snd
                .sampler
                .iter()
                .flat_map(|x| &x.zones)
                .any(|z| z.meta.as_ref().is_some_and(|m| m.mode != deckard_model::new_sampler::PlayMode::Repitch));
            let r = doc.borrow_mut().new_sampler_to_sampler(g, s);
            if r.is_ok() && synced {
                self.status = "converted to a Sampler: its loops play at their own speed now (no Sync or Keys)".into();
            }
            r
        } else if let Some(a) = snd.audio.as_ref().filter(|_| snd.instrument == "AudioModule") {
            let mut takes = Vec::new();
            for t in &a.takes {
                let Some(file) = self.samples.path_of(&t.sample, self.path.as_deref()) else {
                    self.status = format!("can't convert: {} is missing", t.sample.path);
                    return;
                };
                let audio = self.samples.sample(&t.sample, self.path.as_deref());
                let play = (t.play_range.1 > t.play_range.0).then_some(t.play_range);
                let bpm = audio.filter(|_| t.beats > 0.0).map(|x| {
                    let frames = play.map_or(x.frames.len(), |(lo, hi)| (hi - lo).max(1) as usize);
                    t.tempo(frames, x.rate)
                });
                takes.push(deckard_doc::TakeZone { file, play, bpm, beats: (t.beats > 0.0).then_some(t.beats) });
            }
            doc.borrow_mut().audio_to_new_sampler(g, s, &takes, a.tune, a.mode == deckard_model::AudioMode::Gate)
        } else if snd.sampler.is_some() && snd.new_sampler.is_none() {
            // (In Original: it plays as it did.)
            let n =
                deckard_model::new_sampler::NewSampler { mode: deckard_model::new_sampler::SamplerMode::Original, ..Default::default() };
            doc.borrow_mut().transaction("Convert to Sampler Preview", |d| d.set_new_sampler(g, s, Some(&n)))
        } else {
            return;
        };
        if let Err(e) = result {
            self.status = format!("can't convert it: {e}");
        }
    }

    fn is_instrument(what: &layout::AddWhat) -> bool {
        match what {
            layout::AddWhat::Sampler | layout::AddWhat::NewSampler | layout::AddWhat::Audio | layout::AddWhat::Instrument(_) => true,
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
            layout::AddWhat::Sampler | layout::AddWhat::NewSampler => deckard_doc::NewModule::Sampler,
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
        let label = if matches!(what, layout::AddWhat::NewSampler) { "Add Sampler Preview" } else { label };
        let added = doc.borrow_mut().transaction(label, |d| {
            d.add_slot_in(c, at, &module)?;
            if let (layout::AddWhat::NewSampler, deckard_doc::Chain::Sound(g, s)) = (&what, c) {
                d.set_new_sampler(g, s, Some(&deckard_model::new_sampler::NewSampler::default()))?;
            }
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

    /// Ensure a stopped live engine is running on the audio output so plug-ins, buses,
    /// worker threads and direct controller-thread pad hits are warm before the first Play.
    pub(crate) fn ensure_idle_engine(&mut self) {
        if self.screenshot.is_some() || self.playing.is_some() {
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        #[cfg(not(test))]
        if self.output.is_none() {
            self.output = Some(open_output(&self.settings, &mut self.output_fallback));
        }
        let Some(Ok(out)) = &self.output else { return };
        if out.has_engine() {
            return;
        }
        self.remember_libraries();
        let d = doc.borrow();
        let Some(Ok(out)) = &self.output else { return };
        let (mut engine, report) = Engine::new(d.project(), self.path.as_deref(), out.rate, Mode::Arrangement, &mut self.samples);
        engine.sequencing = false;
        out.play(engine);
        self.seen_revision = d.revision();
        self.built = Some((out.plays(), d.project().clone(), report.chains));
    }

    /// Load every plug-in instrument of the project (at the output's rate), so playing
    /// and editing never wait for one.
    fn load_plugins(&mut self) {
        let wanted = self.wanted_plugins();
        if wanted.is_empty() {
            self.ensure_idle_engine();
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
        self.ensure_idle_engine();
    }

    /// Write the state of plug-ins edited in their windows into the document.
    pub(crate) fn store_plugin_states(&mut self) {
        self.store_plugin_states_of(|_| true);
    }

    /// [`App::store_plugin_states`] for the instances `which` picks. Saving a
    /// plug-in's state can make that instance skip audio until the lock is released.
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
        self.hits.clone_from(&played);
        self.loop_lit.clear();
        self.held_pitches.clear();
        if let Some(Ok(out)) = &self.output {
            self.lit.extend(out.played_sounds());
            self.hits.extend(out.played_notes());
            self.held_pitches = out.held_pitches();
            for (g, mask) in out.loop_playing().into_iter().enumerate() {
                let mut m = mask;
                while m != 0 {
                    let s = m.trailing_zeros() as usize;
                    self.loop_lit.insert((g, s));
                    m &= m - 1;
                }
            }
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
        self.poll_prehear();
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
        self.frame_surface_view = None;
        self.mk3_sync(ctx, &mut requests, true);
        self.jam_sync(ctx, true);
        self.frame_surface_view = None;
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
        // A session recording is finalized here (the app leaves without running destructors,
        // and the file would be short of what was written since its last flush).
        self.session.stop();
        self.midi_port_watch = None;
        self.midi_devices.close_all();
        if let Some(m) = &self.mk3 {
            m.shut_down();
        }
        if let Some(j) = &self.jam {
            j.shut_down();
        }
        // The app leaves without running destructors: an ASIO driver is let go of here.
        if deckard_engine::devices::driver() == deckard_engine::devices::AudioDriver::Asio {
            self.close_output();
            deckard_engine::devices::shut_down();
        }
    }

    /// While the window is covered or minimized eframe runs no `ui`, only this: keep the
    /// controller (scripts, LEDs, screens) and MIDI going, as `frame` does when shown.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.finish_quit(ctx) {
            return;
        }
        if self.boot.is_some() {
            self.boot_hidden(ctx);
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
        if std::mem::take(&mut self.drag_release)
            && let Some(pos) = ctx.input(|i| i.pointer.latest_pos())
        {
            let button = egui::PointerButton::Primary;
            raw.events.push(egui::Event::PointerButton { pos, button, pressed: false, modifiers: Default::default() });
        }
        #[cfg(not(test))]
        {
            static VSYNC: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            static ACCESSKIT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if !*VSYNC.get_or_init(|| std::env::var_os("DECKARD_VSYNC").is_some()) {
                // With AutoNoVsync, present() does not block for 16.67 ms; egui's default
                // predicted_dt (1/60s) would subtract 16.67 ms from request_repaint_after(16ms)
                // and spin the UI thread without pacing.
                raw.predicted_dt = 0.002;
            }
            if !*ACCESSKIT.get_or_init(|| std::env::var_os("DECKARD_ACCESSKIT").is_some()) {
                // Remote-desktop / shell WM_GETOBJECT probes otherwise enable full AccessKit
                // widget-tree building and diffing on every 60 Hz frame.
                ctx.disable_accesskit();
            }
        }
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
        // The recorder takes up its audio while armed, recording, or metering (buffer holds ~340 ms);
        // a session recording must keep rendering too, or its silence-pad grows between
        // polls and the meters freeze.
        if self.recorder.busy() || self.recorder.is_metering() || self.session.active() {
            ctx.request_repaint_after(BACKGROUND_PACE);
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
        ctx.data_mut(|d| d.remove::<Vec<egui::Rect>>(egui::Id::new(layout::sampling::layers::LANE_RECTS)));
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

        self.follow_engine(ctx, BACKGROUND_PACE);
        if !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            // SHIFT+Space on the Sampling page: its sample's PLAY / STOP.
            let zone_key = (self.sampling_open && ctx.input(|i| i.modifiers.shift)).then(|| self.audition_key()).flatten();
            match zone_key {
                Some(pitch) => self.audition_toggle(pitch),
                None => self.toggle_play(),
            }
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
            ctx.request_repaint_after(BACKGROUND_PACE);
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
        if self.held.is_some() && !ctx.input(|i| i.pointer.primary_down()) {
            self.release_held();
        }
        let before = (self.group, self.sound, self.scene, self.section, self.song_view);
        // Hardware controllers and MIDI run before plug-in editor timers and UI panels so
        // mutes, pattern switches and transport commands reach the audio thread at once.
        self.sync_notes_played();
        if !self.lit.is_empty() || !self.hits.is_empty() {
            // Unlight the flashed numbers on time.
            ui.ctx().request_repaint_after(deckard_engine::output::FLASH);
        } else if !self.loop_lit.is_empty() {
            ui.ctx().request_repaint_after(BACKGROUND_PACE);
        }
        let mut hw_requests = Vec::new();
        self.frame_surface_view = None;
        self.mk3_sync(ui.ctx(), &mut hw_requests, false);
        self.jam_sync(ui.ctx(), false);
        self.frame_surface_view = None;
        if !hw_requests.is_empty() {
            self.handle(hw_requests);
        }
        self.follow_edits();
        self.midi_sync(ui.ctx());
        self.midi_devices_sync(ui.ctx());
        self.follow_edits();
        self.sync_clocks(ui.ctx());
        self.input_sync();

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
        self.follow_edits();
        self.keep_session_soon();
        if self.session_due.is_some() {
            ui.ctx().request_repaint_after(SESSION_DELAY);
        }
        // The focused group plays live (pads, keys); the others may render ahead.
        if let Some(Ok(out)) = &self.output {
            out.set_focus(Some(self.group));
        }
        self.update_meters(ctx, BACKGROUND_PACE);
        self.poll_prehear();
        if self.prehear_task.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }

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
        let mut requests = Vec::new();
        if let Some(c) = &mut self.controller {
            if self.show_controller {
                egui::Panel::right("controller")
                    .resizable(false)
                    .exact_size(layout::CONTROLLER_WIDTH)
                    .frame(egui::Frame::new().fill(layout::s00()).inner_margin(8.0))
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
        self.handle(requests);
        self.follow_edits();
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
        // The Jam's strip is its own window: the page its script published, shown and hidden
        // by that page's own lifetime (never by a mode), over whatever view is showing, and
        // never in front of the mouse. No page draws no window.
        let shown = self.jam_shown();
        let strip = if self.settings.show_overlay { shown.as_ref().map(|(page, rows)| (page, rows.as_slice())) } else { None };
        let saved = self.settings.jam_strip_pos.map(|(x, y)| egui::Vec2::new(x, y));
        if let Some(offset) = crate::jam_window::show(ctx, strip, saved) {
            // Where the window manager put it, which is where the user has it now.
            self.settings.jam_strip_pos = Some((offset.x, offset.y));
            #[cfg(not(test))]
            self.settings.save();
        }
        layout::body(self, ui);
        if self.jam_shot
            && let Some((page, rows)) = &shown
        {
            // The window's own drawing, at its own size, over the app: `--screenshot` captures
            // it and the run crops the corner back out. (A second viewport is not in the main
            // one's picture, and Screen Recording is not always granted.)
            let width = crate::jam_strip::width_for(ui.max_rect().width());
            let size = crate::jam_window::window_size(width);
            let at = egui::Rect::from_min_size(egui::pos2(8.0, 8.0), size);
            let (name, counter) = (crate::jam_strip::name(page), crate::jam_strip::counter(page));
            egui::Area::new(egui::Id::new("jam strip preview")).fixed_pos(at.min).order(egui::Order::Foreground).show(ctx, |ui| {
                ui.painter().rect_filled(at, 0.0, egui::Color32::BLACK);
                ui.scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
                    crate::jam_window::paint(ui, &name, counter.as_deref(), rows, width, false);
                });
            });
        }
        self.update_slot_view();
        self.follow_edits();
        if !self.dropped_samples.is_empty() {
            let files = std::mem::take(&mut self.dropped_samples);
            // On a lane of the LAYERS tab: a layer there.
            let lanes =
                ctx.data(|d| d.get_temp::<Vec<egui::Rect>>(egui::Id::new(layout::sampling::layers::LANE_RECTS))).unwrap_or_default();
            let lane = self
                .file_drag
                .filter(|_| self.sampling_open && self.sampling.tab == 4)
                .and_then(|p| lanes.iter().position(|r| r.contains(p)));
            if let Some(lane) = lane {
                self.layer_files(lane, &files);
            } else {
                let rows =
                    ctx.data(|d| d.get_temp::<Vec<(egui::Rect, (usize, usize))>>(egui::Id::new(layout::DROP_ROWS))).unwrap_or_default();
                let at = self.file_drag.and_then(|p| rows.iter().find(|(r, _)| r.contains(p)).map(|&(_, gs)| gs));
                self.drop_samples(at.unwrap_or((self.group, self.sound)), &files);
            }
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
        if self.session_window_open {
            match session_window::show(ctx, &self.session.view, self.session_window_pin.on()) {
                session_window::WindowAction::TogglePlay => self.toggle_session_play(),
                session_window::WindowAction::Stop => self.stop_session_record(),
                session_window::WindowAction::Hide => self.session_window_open = false,
                session_window::WindowAction::TogglePin => self.session_window_pin = self.session_window_pin.toggled(),
                session_window::WindowAction::None => {}
            }
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
mod tests;

/// The MK3's LEDs as the scripts have set them: colour LEDs from their palette colour
/// (or RGB), the others from their level.
/// A controller theme's name as shown: `solarized_dark` → "Solarized Dark".
fn theme_name(t: &str) -> String {
    if t == "lcd" {
        return "LCD".to_string();
    }
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
    for theme in deckard_surface::theme_file::stock() {
        if let Some(text) = deckard_surface::theme_file::source(theme.id) {
            std::fs::write(defaults.join(format!("themes/{}.lua", theme.id)), text)?;
        }
    }
    std::fs::write(
        dir.join("README.txt"),
        "DECKARD's controller scripts: one file per page (docs/controller-api.md in DECKARD's source).\n\n\
         defaults/ holds DECKARD's own scripts, refreshed each time this folder is opened.\n\
         To change one, copy it from defaults/ to the same place here (defaults/pages/control.lua\n\
         to pages/control.lua) and edit it: DECKARD loads it as you save. Delete it to go back.\n\
         A new theme in themes/ shows in Preferences > Hardware > Hardware theme.\n\
         Preferences > Hardware can also edit colours and import or export a theme file.\n\
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

/// Where the parked playhead `h` is in a pattern `plen` ticks long, from its start (as
/// [`App::seek_pattern`] put it there): in Song view, from its section's start.
fn parked_tick(p: &deckard_model::Project, h: Playhead, plen: i64) -> i64 {
    let pos = h.position.max(0.0) as i64;
    let from = match h.mode {
        Mode::Arrangement => {
            let mut start = 0i64;
            for sec in &p.sections {
                if pos < start + sec.length {
                    break;
                }
                start += sec.length;
            }
            start
        }
        _ => 0,
    };
    (pos - from).max(0).rem_euclid(plen.max(1))
}
