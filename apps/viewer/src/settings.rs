//! Preferences (plug-in formats and folders, plug-ins switched off) and the catalogue
//! of installed plug-ins, as in Maschine's Preferences > Plug-ins (Manager, Locations).

use deckard_engine::devices::AudioDriver;
use deckard_plugins::scan::{self, Entry, Kind};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

/// Maschine's Preferences > Plug-ins: which formats to use, the folders searched for
/// VST plug-ins (Location), and plug-ins switched off (Manager). Audio Units are found
/// through the system, not folders.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub use_vst3: bool,
    pub use_vst2: bool,
    pub use_au: bool,
    /// Rescan when the app starts.
    pub scan_on_start: bool,
    /// Linux: run `yabridgectl sync` before each rescan, so Windows plug-ins installed
    /// into Wine are bridged.
    pub bridge_windows: bool,
    /// Folders searched for plug-ins besides the platform's standard ones
    /// ([`Settings::scan_dirs`]), added in Preferences.
    pub dirs: Vec<PathBuf>,
    /// Plug-ins switched off in the Manager ([`key`]).
    pub disabled: BTreeSet<String>,
    /// The audio interface recorded from with Source Ext. Stereo / Ext. Mono
    /// (Preferences > Audio); `None`: the system's default input.
    pub input_device: Option<String>,
    /// Preferences > Audio > Routing > Input: the interface channel each of Maschine's
    /// 32 input channels (In 1 L … In 16 R) reads, or not connected (then it isn't
    /// offered anywhere).
    pub input_routing: Vec<Option<usize>>,
    /// In 1's and In 2's levels (as a fader's: 0..1, 0.75 = unity), DECKARD's own on the
    /// way in (a JAM's IN1 and IN2).
    pub input_levels: [f64; 2],
    /// The output device (Preferences > Audio); `None`: the system's default output.
    pub output_device: Option<String>,
    /// Routing > Output: the interface channel each of the 32 output channels (Out 1 L
    /// … Out 16 R) plays on, or not connected (then it isn't offered anywhere).
    pub output_routing: Vec<Option<usize>>,
    /// The names given to the input and output channels in Routing (empty: their own),
    /// shown wherever they're offered.
    pub input_aliases: Vec<String>,
    pub output_aliases: Vec<String>,
    /// Each device's routing and aliases, as last set ("in"/"out", device name; "" for
    /// the system's default): picking a device again brings its own back.
    pub routings: BTreeMap<(String, String), Routing>,
    /// The audio system asked for (Preferences > Audio; what's in use is
    /// [`deckard_engine::devices::driver`]), and the ASIO driver, which serves input and
    /// output together. `input_device` and `output_device` are the system driver's.
    pub audio_driver: AudioDriver,
    pub asio_device: Option<String>,
    /// The Sampler pages show the focused zone's waveform (Maschine doesn't).
    pub show_waveform: bool,
    /// The browser's FILES tab: folders pinned to its top.
    pub favorite_folders: Vec<PathBuf>,
    /// The browser's `+ ROUTING` and `+ PATTERNS`: loading a group brings its routing
    /// and its patterns (both on by default; kept between sessions).
    pub load_group_routing: bool,
    pub load_group_patterns: bool,
    /// Preferences ▸ General: bars counted in before recording from a stop (0: none),
    /// and recorded notes snapped to the grid.
    pub count_in_bars: u32,
    pub input_quantize: bool,
    /// Ask GitHub for a newer release at launch (default on).
    pub check_updates: bool,
    /// The release that ran last: one newer at launch was just updated to, and offers
    /// its release notes.
    pub last_version: String,
    /// Preferences ▸ General: the icon the window, the taskbar and the Dock show.
    pub app_icon: crate::icon::Style,
    /// Preferences ▸ Library: folders of sound packs listed under Factory.
    pub library_folders: Vec<PathBuf>,
    /// MIDI ports switched off (Preferences > MIDI): `in:<name>` and `out:<name>`.
    pub midi_off: BTreeSet<String>,
    /// MIDI inputs that are MPE controllers: port name → member pitch-bend range (±
    /// semitones).
    pub mpe: std::collections::BTreeMap<String, u8>,
    /// MIDI outputs to MPE synths: port name → member pitch-bend range (± semitones).
    pub mpe_out: std::collections::BTreeMap<String, u8>,
    /// MIDI controllers' drum channels (1–16), by device id: their notes on it from C1
    /// play the focused group's pads.
    pub drum_channels: std::collections::BTreeMap<String, u8>,
    /// Their drum tracks (from 1), by device id: a print from it plays the pads too.
    pub drum_tracks: std::collections::BTreeMap<String, u8>,
    /// Send MPE synths their set-up (MPE on, the bend ranges) when a port is set to MPE
    /// or connects.
    pub mpe_setup: bool,
    /// What a sound whose MIDI Source is "Default" takes (Preferences > Default):
    /// the focus follows it (else nothing).
    pub sound_midi_focus: bool,
    /// Audio buffer size in frames (Preferences > Audio).
    pub buffer_frames: u32,
    /// The stem separation model was offered at start-up (installed, or declined):
    /// not again.
    pub stems_offered: bool,
    /// Render sounds on several cores (Preferences > Audio).
    pub multicore: bool,
    /// Live input (pads, keys, MIDI) placed at its sample, one buffer after it came
    /// (else at the start of the next buffer) (Preferences > Audio).
    pub exact_timing: bool,
    /// Render sounds that can't get live input this far ahead (ms; 0, the default: off,
    /// every sound plays live as in Maschine)
    /// (Preferences > Audio).
    pub render_ahead_ms: u32,
    /// The MK3 displays' backlight, 0..=100 (Maschine's Hardware settings > Display).
    pub display_brightness: u8,
    /// DECKARD drives a Maschine JAM plugged in (Preferences > MIDI).
    pub jam_enabled: bool,
    /// MIDI recorded into another group makes it the focused group (Preferences > MIDI).
    pub focus_follows: bool,
    /// A JAM touchstrip moves a value by the distance the finger travels instead of setting it
    /// to where the finger lands (Preferences > Hardware).
    pub relative_strips: bool,
    /// How hard the MK3's pads have to be pressed, 0..=100 (Preferences > Hardware): 0
    /// takes the pressure as it comes; higher, light presses play louder.
    pub pad_sensitivity: u8,
    /// The metronome's volume, 0..=100 %, and whether it clicks while recording though
    /// it's off (the Settings page's VOLUME and AUTOENABLE).
    pub metronome_volume: u8,
    pub metronome_auto: bool,
    /// The metronome's tick, a note value (4: quarter notes, as Maschine's TIMESIGNATURE
    /// "1/4").
    pub metronome_ticks: u32,
    /// GENERAL's Project defaults: open the last project at start (else a new one,
    /// unless it had unsaved changes), and keep it as it's edited (the session, for a
    /// crash), not only on quitting.
    pub reload_last: bool,
    pub auto_backup: bool,
    /// The project File ▸ New starts from (a template: its groups, sounds, plug-ins,
    /// tempo and routing); `None`, or gone: an empty project.
    pub default_project: Option<PathBuf>,
    /// Scene / Section: duplicating a scene copies its patterns too (Maschine's "Scn &
    /// Ptn"), and a duplicated section plays the same scene (LINK WHEN DUPL).
    pub duplicate_patterns: bool,
    pub link_duplicates: bool,
    /// Pattern: new patterns' length (bars, beats, sixteenths), and whether an empty
    /// pattern grows as it's recorded into (GROW).
    pub pattern_length: (u32, u32, u32),
    pub pattern_grow: bool,
    /// The Settings page's HARDWARE: how the pads' presses become velocities (-3 Soft 3
    /// … 0 Linear … 3 Hard 3), the one-colour lights' brightness (0..=100 %), when the
    /// Smart Strip shows the play position with no strip mode on (0 Off, 1 while
    /// recording, 2 On), and whether touching a knob writes its modulation while
    /// Auto-write is on (Touch Auto-write).
    pub velocity_scaling: i8,
    pub led_brightness: u8,
    pub strip_position: u8,
    pub touch_auto_write: bool,
    /// How tall the patterns and scenes above the pattern editor were dragged; `None`:
    /// as tall as the window's size has them.
    pub song_panel_h: Option<f32>,
    /// Which controller layer runs the MK3 (Preferences > Hardware > Controller).
    pub controller: ControllerChoice,
    /// The MK3 screens' theme under DECKARD's own layer (Preferences > Hardware).
    pub controller_theme: String,
    /// Ableton Link (the header's LINK): joined, and starting and stopping with the
    /// session (Start Stop Sync).
    pub link: bool,
    pub link_start_stop: bool,
    /// Preferences > MIDI > Sync: MIDI clock off, sent or followed.
    pub midi_sync: MidiSync,
    /// Clock Offset: MIDI clock sent this early (0..=200 ms), and the song played this
    /// much after the clock received (-100..=100 ms).
    pub clock_send_offset_ms: u32,
    pub clock_receive_offset_ms: i32,
}

/// Preferences > MIDI > Sync > Mode (Maschine's `NSA:getMidiSyncStatus`, 0–2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MidiSync {
    #[default]
    Off,
    /// Send Clock: to the MIDI outputs switched on.
    Send,
    /// Receive Clock: from the MIDI inputs switched on (not while Link is on).
    Receive,
}

/// Which layer runs the MK3: Maschine's controller scripts (from the user's own
/// install) or DECKARD's own (docs/superpowers/specs/2026-10-25-controller-layer-design.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ControllerChoice {
    /// The scripts when they're found, else DECKARD's.
    Automatic,
    /// (A new install's.)
    #[default]
    Deckard,
    Scripts,
}

/// A new install's theme for DECKARD's own layer.
pub const CONTROLLER_THEME: &str = "palenight";

/// The metronome's ticks offered (note values: 1/1 … 1/32).
pub const METRONOME_TICKS: [u32; 6] = [1, 2, 4, 8, 16, 32];

/// Input and output channels: In/Out 1 L … 16 R.
pub const CHANNELS: usize = 32;

/// A device's routing: each channel's interface channel (or not connected), and its
/// alias (empty: none).
#[derive(Debug, Clone, PartialEq)]
pub struct Routing {
    pub routes: Vec<Option<usize>>,
    pub aliases: Vec<String>,
}

impl Default for Routing {
    fn default() -> Routing {
        Routing { routes: default_routing(), aliases: vec![String::new(); CHANNELS] }
    }
}

/// In 1 and Out 1 on the interface's first two channels, the rest not connected.
pub fn default_routing() -> Vec<Option<usize>> {
    (0..CHANNELS).map(|i| (i < 2).then_some(i)).collect()
}

/// A mono channel's own name: "In 1 L", "Out 16 R".
pub fn channel_name(kind: &str, i: usize) -> String {
    format!("{kind} {} {}", i / 2 + 1, if i % 2 == 0 { "L" } else { "R" })
}

impl Settings {
    /// Input channel `i` (In 1 L = 0) as shown: its alias, else its own name.
    pub fn input_name(&self, i: usize) -> String {
        alias_or(&self.input_aliases, i, || channel_name("In", i))
    }

    /// The stereo input `pair` (In 1 L+R = 0) as shown: its channels' names.
    pub fn input_pair_name(&self, pair: usize) -> String {
        pair_name(&self.input_aliases, pair, || format!("In {} L+R", pair + 1), |i| self.input_name(i))
    }

    /// Output channel `i` (Out 1 L = 0) as shown.
    pub fn output_name(&self, i: usize) -> String {
        alias_or(&self.output_aliases, i, || channel_name("Out", i))
    }

    /// External output `pair` (Ext. 1 = 0) as shown.
    pub fn output_pair_name(&self, pair: usize) -> String {
        pair_name(&self.output_aliases, pair, || format!("Ext. {}", pair + 1), |i| self.output_name(i))
    }

    /// Whether input channel `i` is connected (else it isn't offered).
    pub fn input_on(&self, i: usize) -> bool {
        self.input_routing.get(i).is_some_and(Option::is_some)
    }

    /// Whether the stereo input `pair` is offered: either side connected.
    pub fn input_pair_on(&self, pair: usize) -> bool {
        self.input_on(pair * 2) || self.input_on(pair * 2 + 1)
    }

    /// Whether external output `pair` is offered: either side connected.
    pub fn output_pair_on(&self, pair: usize) -> bool {
        (0..2).any(|k| self.output_routing.get(pair * 2 + k).is_some_and(Option::is_some))
    }
}

fn alias_or(aliases: &[String], i: usize, own: impl FnOnce() -> String) -> String {
    match aliases.get(i).map(|a| a.trim()) {
        Some(a) if !a.is_empty() => a.to_string(),
        _ => own(),
    }
}

/// A stereo pair's name: its own while neither channel has an alias; one alias when
/// both have it; else both channels' names.
fn pair_name(aliases: &[String], pair: usize, own: impl FnOnce() -> String, name: impl Fn(usize) -> String) -> String {
    let alias = |i: usize| aliases.get(i).map_or("", |a| a.trim());
    let (l, r) = (alias(pair * 2), alias(pair * 2 + 1));
    match (l.is_empty(), r.is_empty()) {
        (true, true) => own(),
        (false, false) if l == r => l.to_string(),
        _ => format!("{} / {}", name(pair * 2), name(pair * 2 + 1)),
    }
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            use_vst3: true,
            use_vst2: true,
            use_au: true,
            favorite_folders: Vec::new(),
            load_group_routing: true,
            load_group_patterns: true,
            library_folders: Vec::new(),
            count_in_bars: 1,
            input_quantize: false,
            check_updates: true,
            last_version: String::new(),
            app_icon: crate::icon::Style::default(),
            scan_on_start: false,
            bridge_windows: true,
            dirs: Vec::new(),
            disabled: BTreeSet::new(),
            input_device: None,
            input_routing: default_routing(),
            input_levels: [0.75; 2],
            output_device: None,
            output_routing: default_routing(),
            input_aliases: vec![String::new(); CHANNELS],
            output_aliases: vec![String::new(); CHANNELS],
            routings: BTreeMap::new(),
            audio_driver: AudioDriver::default(),
            asio_device: None,
            show_waveform: true,
            midi_off: BTreeSet::new(),
            mpe: Default::default(),
            mpe_out: Default::default(),
            drum_channels: Default::default(),
            drum_tracks: Default::default(),
            mpe_setup: true,
            sound_midi_focus: true,
            buffer_frames: deckard_engine::output::DEFAULT_BUFFER,
            stems_offered: false,
            multicore: true,
            exact_timing: true,
            render_ahead_ms: 0,
            display_brightness: 100,
            jam_enabled: true,
            pad_sensitivity: 50,
            metronome_volume: 100,
            focus_follows: true,
            relative_strips: false,
            metronome_auto: false,
            metronome_ticks: 4,
            reload_last: true,
            default_project: None,
            auto_backup: true,
            duplicate_patterns: false,
            link_duplicates: false,
            pattern_length: (1, 0, 0),
            pattern_grow: true,
            velocity_scaling: 0,
            led_brightness: 100,
            song_panel_h: None,
            strip_position: 2,
            touch_auto_write: false,
            controller: ControllerChoice::default(),
            controller_theme: CONTROLLER_THEME.to_string(),
            link: false,
            link_start_stop: false,
            midi_sync: MidiSync::Off,
            clock_send_offset_ms: 0,
            clock_receive_offset_ms: 0,
        }
    }
}

/// What identifies a plug-in in the settings.
pub fn key(e: &Entry) -> String {
    format!("{}|{}|{}", e.kind.label(), e.location, e.cid)
}

fn file(name: &str) -> Option<PathBuf> {
    Some(crate::menu::settings_dir()?.join(name))
}

impl Settings {
    /// The saved settings (lines of `key = value`), or the defaults.
    pub fn load() -> Settings {
        file("settings.txt").and_then(|f| std::fs::read_to_string(f).ok()).map_or_else(Settings::default, |t| Settings::parse(&t))
    }

    /// The platform's standard plug-in folders (VST3, then VST), always scanned.
    pub fn standard_dirs() -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = scan::default_vst3_paths().into_iter().chain(scan::default_vst2_paths()).collect();
        let mut seen = std::collections::HashSet::new();
        dirs.retain(|d| seen.insert(d.clone()));
        dirs
    }

    /// Every folder scanned: the standard ones, then the user's additions.
    pub fn scan_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Settings::standard_dirs();
        for d in &self.dirs {
            if !dirs.contains(d) {
                dirs.push(d.clone());
            }
        }
        dirs
    }

    /// Settings from their saved text ([`Settings::to_text`]).
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        let mut dirs = Vec::new();
        // Input routes saved before devices kept their own (`<channel>:<route>`, only
        // those not straight through).
        let mut legacy: Option<Vec<Option<usize>>> = None;
        for line in text.lines() {
            let Some((k, v)) = line.split_once(" = ") else { continue };
            match k {
                "use_vst3" => s.use_vst3 = v == "true",
                "use_vst2" => s.use_vst2 = v == "true",
                "use_au" => s.use_au = v == "true",
                "scan_on_start" => s.scan_on_start = v == "true",
                "bridge_windows" => s.bridge_windows = v == "true",
                "show_waveform" => s.show_waveform = v == "true",
                "dir" => dirs.push(PathBuf::from(v)),
                "favorite_folder" => s.favorite_folders.push(PathBuf::from(v)),
                "load_group_routing" => s.load_group_routing = v != "false",
                "load_group_patterns" => s.load_group_patterns = v != "false",
                "library_folder" => s.library_folders.push(PathBuf::from(v)),
                "count_in_bars" => s.count_in_bars = v.parse().unwrap_or(1),
                "input_quantize" => s.input_quantize = v == "true",
                "check_updates" => s.check_updates = v != "false",
                "last_version" => s.last_version = v.to_string(),
                "app_icon" => s.app_icon = crate::icon::Style::from_key(v).unwrap_or_default(),
                "input_device" => s.input_device = Some(v.to_string()),
                "input_level_1" => s.input_levels[0] = v.parse().unwrap_or(0.75),
                "input_level_2" => s.input_levels[1] = v.parse().unwrap_or(0.75),
                "output_device" => s.output_device = Some(v.to_string()),
                "audio_driver" => s.audio_driver = if v == "asio" { AudioDriver::Asio } else { AudioDriver::System },
                "asio_device" => s.asio_device = Some(v.to_string()),
                // A device's routing: `<device>\t<channel>:<route or alias>`.
                "input_route" if !v.contains('\t') => {
                    if let Some((i, x)) = v.split_once(':')
                        && let Ok(i) = i.parse::<usize>()
                        && i < CHANNELS
                    {
                        legacy.get_or_insert_with(|| (0..CHANNELS).map(Some).collect())[i] = x.parse::<usize>().ok();
                    }
                }
                "input_route" | "output_route" | "input_alias" | "output_alias" => {
                    let (device, rest) = v.split_once('\t').unwrap_or(("", v));
                    if let Some((i, x)) = rest.split_once(':')
                        && let Ok(i) = i.parse::<usize>()
                        && i < CHANNELS
                    {
                        let side = if k.starts_with("input") { "in" } else { "out" };
                        let r = s.routings.entry((side.to_string(), device.to_string())).or_default();
                        if k.ends_with("route") {
                            r.routes[i] = x.parse::<usize>().ok();
                        } else {
                            r.aliases[i] = x.to_string();
                        }
                    }
                }
                "disabled" => {
                    s.disabled.insert(v.to_string());
                }
                "midi_off" => {
                    s.midi_off.insert(v.to_string());
                }
                "mpe" => {
                    if let Some((range, port)) = v.split_once(' ')
                        && let Ok(range) = range.parse::<u8>()
                    {
                        s.mpe.insert(port.to_string(), range.clamp(1, 96));
                    }
                }
                "mpe_out" => {
                    if let Some((range, port)) = v.split_once(' ')
                        && let Ok(range) = range.parse::<u8>()
                    {
                        s.mpe_out.insert(port.to_string(), range.clamp(1, 96));
                    }
                }
                "mpe_setup" => s.mpe_setup = v != "false",
                "sound_midi_default" => s.sound_midi_focus = v != "none",
                "drum_track" => {
                    if let Some((n, id)) = v.split_once(' ')
                        && let Ok(n) = n.parse::<u8>()
                    {
                        s.drum_tracks.insert(id.to_string(), n.clamp(1, 16));
                    }
                }
                "drum_channel" => {
                    if let Some((ch, id)) = v.split_once(' ')
                        && let Ok(ch) = ch.parse::<u8>()
                    {
                        s.drum_channels.insert(id.to_string(), ch.clamp(1, 16));
                    }
                }
                "multicore" => s.multicore = v != "false",
                "link" => s.link = v == "true",
                "link_start_stop" => s.link_start_stop = v == "true",
                "midi_sync" => {
                    s.midi_sync = match v {
                        "send" => MidiSync::Send,
                        "receive" => MidiSync::Receive,
                        _ => MidiSync::Off,
                    }
                }
                "clock_send_offset_ms" => s.clock_send_offset_ms = v.parse::<u32>().map_or(0, |n| n.min(200)),
                "clock_receive_offset_ms" => s.clock_receive_offset_ms = v.parse::<i32>().map_or(0, |n| n.clamp(-100, 100)),
                "exact_timing" => s.exact_timing = v != "false",
                "controller_theme" if !v.is_empty() => s.controller_theme = v.to_string(),
                "controller" => {
                    s.controller = match v {
                        "automatic" => ControllerChoice::Automatic,
                        "scripts" => ControllerChoice::Scripts,
                        _ => ControllerChoice::Deckard,
                    }
                }
                "metronome_volume" => {
                    if let Ok(n) = v.parse::<u8>() {
                        s.metronome_volume = n.min(100);
                    }
                }
                "focus_follows" => s.focus_follows = v != "false",
                "relative_strips" => s.relative_strips = v == "true",
                "metronome_auto" => s.metronome_auto = v == "true",
                "metronome_ticks" => {
                    if let Ok(n) = v.parse::<u32>()
                        && METRONOME_TICKS.contains(&n)
                    {
                        s.metronome_ticks = n;
                    }
                }
                "reload_last" => s.reload_last = v == "true",
                "stems_offered" => s.stems_offered = v == "true",
                "default_project" if !v.is_empty() => s.default_project = Some(PathBuf::from(v)),
                "auto_backup" => s.auto_backup = v == "true",
                "duplicate_patterns" => s.duplicate_patterns = v == "true",
                "link_duplicates" => s.link_duplicates = v == "true",
                // `<bars>:<beats>:<sixteenths>`.
                "pattern_length" => {
                    let n: Vec<u32> = v.split(':').filter_map(|n| n.parse().ok()).collect();
                    if let [bars, beats, sixteenths] = n[..] {
                        s.pattern_length = (bars, beats.min(3), sixteenths.min(3));
                    }
                }
                "pattern_grow" => s.pattern_grow = v == "true",
                "velocity_scaling" => {
                    if let Ok(n) = v.parse::<i8>() {
                        s.velocity_scaling = n.clamp(-3, 3);
                    }
                }
                "song_panel_h" => s.song_panel_h = v.parse::<f32>().ok().filter(|h| h.is_finite() && *h > 0.0),
                "led_brightness" => {
                    if let Ok(n) = v.parse::<u8>() {
                        s.led_brightness = n.min(100);
                    }
                }
                "strip_position" => {
                    if let Ok(n) = v.parse::<u8>() {
                        s.strip_position = n.min(2);
                    }
                }
                "touch_auto_write" => s.touch_auto_write = v == "true",
                "pad_sensitivity" => {
                    if let Ok(n) = v.parse::<u8>() {
                        s.pad_sensitivity = n.min(100);
                    }
                }
                "display_brightness" => {
                    if let Ok(n) = v.parse::<u8>() {
                        s.display_brightness = n.min(100);
                    }
                }
                "jam_enabled" => s.jam_enabled = v != "false",
                "render_ahead_ms" => {
                    if let Ok(n) = v.parse::<u32>() {
                        s.render_ahead_ms = n.min(200);
                    }
                }
                "buffer_frames" => {
                    if let Ok(n) = v.parse::<u32>()
                        && deckard_engine::output::BUFFER_SIZES.contains(&n)
                    {
                        s.buffer_frames = n;
                    }
                }
                _ => {}
            }
        }
        // Settings from before listed the standard folders too (and a settings file
        // carried over from another platform lists that platform's): only the others
        // are the user's additions. The standard ones are always scanned.
        let standard = Settings::standard_dirs();
        s.dirs = dirs.into_iter().filter(|d| !standard.contains(d)).collect();
        s.dirs.dedup();
        // Settings from before: everything straight through, but for what they list.
        let has_routing = s.routings.keys().any(|(side, _)| side == "in");
        if !has_routing && (legacy.is_some() || text.contains("input_device = ")) {
            let routes = legacy.unwrap_or_else(|| (0..CHANNELS).map(Some).collect());
            let key = s.device_key("in");
            s.routings.insert(key, Routing { routes, ..Routing::default() });
        }
        // The devices in use, with their routings.
        s.load_routing("in");
        s.load_routing("out");
        s
    }

    /// Pick the input device: its own routing comes back (the one left is kept).
    pub fn set_input_device(&mut self, device: Option<String>) {
        self.keep_routing("in");
        self.input_device = device;
        self.load_routing("in");
    }

    /// Pick the output device, as [`Settings::set_input_device`].
    pub fn set_output_device(&mut self, device: Option<String>) {
        self.keep_routing("out");
        self.output_device = device;
        self.load_routing("out");
    }

    /// Ask for another audio system: its devices' routings come back.
    pub fn set_audio_driver(&mut self, driver: AudioDriver) {
        self.keep_routing("in");
        self.keep_routing("out");
        self.audio_driver = driver;
        self.load_routing("in");
        self.load_routing("out");
    }

    /// Pick the ASIO driver, as [`Settings::set_input_device`].
    pub fn set_asio_device(&mut self, device: Option<String>) {
        self.keep_routing("in");
        self.keep_routing("out");
        self.asio_device = device;
        self.load_routing("in");
        self.load_routing("out");
    }

    /// The input device of the driver in use (it can differ from the one asked for).
    pub fn input_device_in_use(&self) -> Option<String> {
        match deckard_engine::devices::driver() {
            AudioDriver::Asio => self.asio_device.clone(),
            AudioDriver::System => self.input_device.clone(),
        }
    }

    /// The output device of the driver in use.
    pub fn output_device_in_use(&self) -> Option<String> {
        match deckard_engine::devices::driver() {
            AudioDriver::Asio => self.asio_device.clone(),
            AudioDriver::System => self.output_device.clone(),
        }
    }

    fn device_key(&self, side: &str) -> (String, String) {
        let d = match self.audio_driver {
            AudioDriver::Asio => &self.asio_device,
            AudioDriver::System if side == "in" => &self.input_device,
            AudioDriver::System => &self.output_device,
        };
        (side.to_string(), d.clone().unwrap_or_default())
    }

    /// Remember the device in use's routing.
    fn keep_routing(&mut self, side: &str) {
        let r = if side == "in" {
            Routing { routes: self.input_routing.clone(), aliases: self.input_aliases.clone() }
        } else {
            Routing { routes: self.output_routing.clone(), aliases: self.output_aliases.clone() }
        };
        self.routings.insert(self.device_key(side), r);
    }

    /// Take up the device in use's routing (the defaults for one not set up yet).
    fn load_routing(&mut self, side: &str) {
        let r = self.routings.get(&self.device_key(side)).cloned().unwrap_or_default();
        if side == "in" {
            (self.input_routing, self.input_aliases) = (r.routes, r.aliases);
        } else {
            (self.output_routing, self.output_aliases) = (r.routes, r.aliases);
        }
    }

    pub fn save(&self) {
        let Some(f) = file("settings.txt") else { return };
        let out = self.to_text();
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(f, out);
    }

    /// The settings as saved: lines of `key = value`.
    pub fn to_text(&self) -> String {
        let mut out = format!(
            "use_vst3 = {}\nuse_vst2 = {}\nuse_au = {}\nscan_on_start = {}\nshow_waveform = {}\n",
            self.use_vst3, self.use_vst2, self.use_au, self.scan_on_start, self.show_waveform
        );
        if !self.bridge_windows {
            out += "bridge_windows = false\n";
        }
        for d in &self.dirs {
            out += &format!("dir = {}\n", d.display());
        }
        for d in &self.disabled {
            out += &format!("disabled = {d}\n");
        }
        for m in &self.midi_off {
            out += &format!("midi_off = {m}\n");
        }
        for (port, range) in &self.mpe {
            out += &format!("mpe = {range} {port}\n");
        }
        for (port, range) in &self.mpe_out {
            out += &format!("mpe_out = {range} {port}\n");
        }
        for (id, ch) in &self.drum_channels {
            out += &format!("drum_channel = {ch} {id}\n");
        }
        for (id, n) in &self.drum_tracks {
            out += &format!("drum_track = {n} {id}\n");
        }
        if !self.mpe_setup {
            out += "mpe_setup = false\n";
        }
        for d in &self.favorite_folders {
            out += &format!("favorite_folder = {}\n", d.display());
        }
        if !self.load_group_routing {
            out += "load_group_routing = false\n";
        }
        if !self.load_group_patterns {
            out += "load_group_patterns = false\n";
        }
        out += &format!("count_in_bars = {}\ninput_quantize = {}\n", self.count_in_bars, self.input_quantize);
        if !self.check_updates {
            out += "check_updates = false\n";
        }
        if !self.last_version.is_empty() {
            out += &format!("last_version = {}\n", self.last_version);
        }
        if self.app_icon != crate::icon::Style::default() {
            out += &format!("app_icon = {}\n", self.app_icon.key());
        }
        for d in &self.library_folders {
            out += &format!("library_folder = {}\n", d.display());
        }
        if !self.sound_midi_focus {
            out += "sound_midi_default = none\n";
        }
        out += &format!(
            "buffer_frames = {}\nmulticore = {}\nexact_timing = {}\nrender_ahead_ms = {}\ndisplay_brightness = {}\npad_sensitivity = {}\n",
            self.buffer_frames, self.multicore, self.exact_timing, self.render_ahead_ms, self.display_brightness, self.pad_sensitivity
        );
        out += &format!("jam_enabled = {}\n", self.jam_enabled);
        out += &format!("focus_follows = {}\n", self.focus_follows);
        out += &format!("relative_strips = {}\n", self.relative_strips);
        out += &format!(
            "metronome_volume = {}\nmetronome_auto = {}\nmetronome_ticks = {}\n",
            self.metronome_volume, self.metronome_auto, self.metronome_ticks
        );
        let (bars, beats, sixteenths) = self.pattern_length;
        out += &format!(
            "reload_last = {}\nauto_backup = {}\nduplicate_patterns = {}\nlink_duplicates = {}\npattern_length = {bars}:{beats}:{sixteenths}\npattern_grow = {}\n",
            self.reload_last, self.auto_backup, self.duplicate_patterns, self.link_duplicates, self.pattern_grow
        );
        if let Some(p) = &self.default_project {
            out += &format!("default_project = {}\n", p.display());
        }
        if self.stems_offered {
            out += "stems_offered = true\n";
        }
        out += &format!("input_level_1 = {}\ninput_level_2 = {}\n", self.input_levels[0], self.input_levels[1]);
        out += &format!(
            "velocity_scaling = {}\nled_brightness = {}\nstrip_position = {}\ntouch_auto_write = {}\n",
            self.velocity_scaling, self.led_brightness, self.strip_position, self.touch_auto_write
        );
        if let Some(h) = self.song_panel_h {
            out += &format!("song_panel_h = {h}\n");
        }
        match self.controller {
            ControllerChoice::Automatic => out += "controller = automatic\n",
            ControllerChoice::Deckard => out += "controller = deckard\n",
            ControllerChoice::Scripts => out += "controller = scripts\n",
        }
        if self.controller_theme != CONTROLLER_THEME {
            out += &format!("controller_theme = {}\n", self.controller_theme);
        }
        let sync = match self.midi_sync {
            MidiSync::Off => "off",
            MidiSync::Send => "send",
            MidiSync::Receive => "receive",
        };
        out += &format!(
            "link = {}\nlink_start_stop = {}\nmidi_sync = {sync}\nclock_send_offset_ms = {}\nclock_receive_offset_ms = {}\n",
            self.link, self.link_start_stop, self.clock_send_offset_ms, self.clock_receive_offset_ms
        );
        if let Some(d) = &self.input_device {
            out += &format!("input_device = {d}\n");
        }
        if let Some(d) = &self.output_device {
            out += &format!("output_device = {d}\n");
        }
        if self.audio_driver == AudioDriver::Asio {
            out += "audio_driver = asio\n";
        }
        if let Some(d) = &self.asio_device {
            out += &format!("asio_device = {d}\n");
        }
        // Every device's routing, every route (so a later change of the defaults
        // leaves them as they were).
        let mut all = self.clone();
        all.keep_routing("in");
        all.keep_routing("out");
        for ((side, device), r) in &all.routings {
            let side = if side == "in" { "input" } else { "output" };
            for (i, ch) in r.routes.iter().enumerate() {
                out += &format!("{side}_route = {device}\t{i}:{}\n", ch.map_or("none".to_string(), |c| c.to_string()));
            }
            for (i, a) in r.aliases.iter().enumerate().filter(|(_, a)| !a.trim().is_empty()) {
                out += &format!("{side}_alias = {device}\t{i}:{}\n", a.trim());
            }
        }
        out
    }

    /// Whether `e` may be offered (its format is on, and it isn't switched off).
    pub fn allows(&self, e: &Entry) -> bool {
        let format_on = match e.kind {
            Kind::Vst3 => self.use_vst3,
            Kind::Vst2 => self.use_vst2,
            Kind::AudioUnit => self.use_au,
        };
        format_on && !self.disabled.contains(&key(e))
    }
}

/// Installed plug-ins, from the last scan (kept in `plugins.txt`), and a scan running.
#[derive(Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    scanning: Option<Receiver<ScanMsg>>,
    pub status: String,
}

enum ScanMsg {
    Progress(String),
    Done(Vec<Entry>, Vec<(String, u64)>),
}

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

impl Catalog {
    /// The catalogue from the last scan; scans if there was none.
    pub fn load(settings: &Settings) -> Catalog {
        let mut c = Catalog::default();
        if let Some(text) = file("plugins.txt").and_then(|f| std::fs::read_to_string(f).ok()) {
            c.entries = text.lines().filter(|l| !l.starts_with('#')).filter_map(Entry::from_line).collect();
        }
        if c.entries.is_empty() || settings.scan_on_start {
            c.rescan(settings);
        }
        c
    }

    pub fn scanning(&self) -> bool {
        self.scanning.is_some()
    }

    /// Scan again, in the background. VST3 bundles without a `moduleinfo.json` are
    /// loaded by a helper process (this program with `--scan-vst3 <bundle>`), so a
    /// plug-in that crashes can't take the app down; bundles that didn't change since
    /// the last scan are taken from it.
    pub fn rescan(&mut self, settings: &Settings) {
        if self.scanning.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let s = settings.clone();
        let old = std::fs::read_to_string(file("plugins.txt").unwrap_or_default()).unwrap_or_default();
        std::thread::spawn(move || {
            // Linux: Windows plug-ins installed into Wine are bridged first (yabridge
            // makes a bundle per plug-in in ~/.vst3/yabridge).
            if cfg!(target_os = "linux") && s.bridge_windows && crate::wine::sync_wanted() {
                let _ = tx.send(ScanMsg::Progress("Syncing Windows plug-ins (yabridge)…".into()));
                let _ = crate::wine::sync();
            }
            // Bundles scanned before, with their mtime, and their entries.
            let mut known: Vec<(String, u64, Vec<Entry>)> = Vec::new();
            for line in old.lines() {
                if let Some(rest) = line.strip_prefix("#bundle\t") {
                    let (p, t) = rest.rsplit_once('\t').unwrap_or((rest, "0"));
                    known.push((p.to_string(), t.parse().unwrap_or(0), Vec::new()));
                } else if let (Some(e), Some(last)) = (Entry::from_line(line), known.last_mut()) {
                    last.2.push(e);
                }
            }
            let mut entries = Vec::new();
            let mut bundles = Vec::new();
            let dirs = s.scan_dirs();
            for b in scan::vst3_bundles(&dirs) {
                let key = b.display().to_string();
                let t = mtime(&b);
                let _ = tx.send(ScanMsg::Progress(format!(
                    "Scanning {}",
                    b.file_name().map_or(key.clone(), |n| n.to_string_lossy().into_owned())
                )));
                let found = if let Some(e) = unchanged_scan(&known, &key, t) {
                    e.clone()
                } else if let Some(e) = scan::vst3_from_moduleinfo(&b) {
                    e
                } else {
                    scan_in_helper(&b)
                };
                bundles.push((key, t));
                entries.push(found);
            }
            let mut all: Vec<Entry> = entries.concat();
            all.extend(scan::vst2_entries(&dirs));
            all.extend(scan::audio_units());
            let _ = tx.send(ScanMsg::Done(all, bundles));
        });
        self.scanning = Some(rx);
        self.status = "Scanning plug-ins…".into();
    }

    /// Take up scan progress (call every frame).
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.scanning else { return false };
        let mut changed = false;
        while let Ok(m) = rx.try_recv() {
            match m {
                ScanMsg::Progress(p) => self.status = p,
                ScanMsg::Done(entries, bundles) => {
                    // The cache: per VST3 bundle, its mtime and entries; then the rest.
                    let mut out = String::new();
                    for (b, t) in &bundles {
                        out += &format!("#bundle\t{b}\t{t}\n");
                        for e in entries.iter().filter(|e| e.kind == Kind::Vst3 && e.location == *b) {
                            out += &(e.to_line() + "\n");
                        }
                    }
                    out += "#other\t\t0\n";
                    for e in entries.iter().filter(|e| e.kind != Kind::Vst3) {
                        out += &(e.to_line() + "\n");
                    }
                    if let Some(f) = file("plugins.txt") {
                        if let Some(dir) = f.parent() {
                            let _ = std::fs::create_dir_all(dir);
                        }
                        let _ = std::fs::write(f, out);
                    }
                    self.status = format!("{} plug-ins found", entries.len());
                    self.entries = entries;
                    self.scanning = None;
                    changed = true;
                    break;
                }
            }
        }
        changed
    }
}

/// What the last scan found in bundle `key`, if it hasn't changed since. A scan that found
/// nothing doesn't count: the helper was killed or crashed (a plug-in slow to start, say),
/// and the bundle is scanned again rather than left out for good.
fn unchanged_scan<'a>(known: &'a [(String, u64, Vec<Entry>)], key: &str, mtime: u64) -> Option<&'a Vec<Entry>> {
    known.iter().find(|(p, t, e)| p == key && *t == mtime && !e.is_empty()).map(|(_, _, e)| e)
}

/// Run `--scan-vst3 <bundle>` in a helper process (20 s at most; 90 s for a yabridge
/// bundle, which starts Wine). What it printed before it hung or crashed still counts.
fn scan_in_helper(bundle: &Path) -> Vec<Entry> {
    let limit = if bundle.to_string_lossy().contains("yabridge") { 90 } else { 20 };
    let Ok(exe) = std::env::current_exe() else { return Vec::new() };
    let Ok(mut child) = std::process::Command::new(exe)
        .arg("--scan-vst3")
        .arg(bundle)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed().as_secs() < limit => std::thread::sleep(std::time::Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    let mut text = String::new();
    if let Some(mut out) = child.stdout.take() {
        use std::io::Read;
        let _ = out.read_to_string(&mut text);
    }
    text.lines().filter_map(Entry::from_line).collect()
}

/// `--scan-vst3 <bundle>`: print the bundle's classes, one [`Entry`] line each.
pub fn scan_vst3_main(bundle: &Path) -> i32 {
    match scan::vst3_by_loading(bundle) {
        // Not println!: if the app quit meanwhile, the pipe is closed and println!
        // would panic, and a panic leaves through the plug-ins' exit-time destructors.
        Ok(entries) => {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            let ok = entries.iter().all(|e| writeln!(out, "{}", e.to_line()).is_ok()) && out.flush().is_ok();
            if ok { 0 } else { 1 }
        }
        Err(e) => {
            use std::io::Write;
            let _ = writeln!(std::io::stderr(), "{e}");
            1
        }
    }
}

#[cfg(test)]
mod routing_tests {
    use super::*;

    /// Each interface keeps its routing: A set up, then B, then A again brings A's
    /// back; the saved text keeps both and the device last picked.
    #[test]
    fn each_device_keeps_its_routing() {
        let mut s = Settings::default();
        assert_eq!(s.output_routing[..3], [Some(0), Some(1), None], "Out 1 on channels 1-2, the rest not connected");
        s.set_output_device(Some("A".into()));
        s.output_routing[2] = Some(4);
        s.output_aliases[2] = "Monitors".into();
        s.set_output_device(Some("B".into()));
        assert_eq!(s.output_routing, default_routing(), "B starts from the defaults");
        s.output_routing[0] = None;
        s.set_output_device(Some("A".into()));
        assert_eq!((s.output_routing[2], s.output_aliases[2].as_str()), (Some(4), "Monitors"), "A's own");
        s.set_input_device(Some("Mic box".into()));
        s.input_aliases[1] = "Vocal".into();
        let back = Settings::parse(&s.to_text());
        assert_eq!(back.output_device.as_deref(), Some("A"), "the last device picked");
        assert_eq!((back.output_routing[2], back.output_aliases[2].as_str()), (Some(4), "Monitors"));
        assert_eq!(back.input_name(1), "Vocal");
        let mut back = back;
        back.set_output_device(Some("B".into()));
        assert_eq!(back.output_routing[0], None, "B's own, saved too");
    }

    /// The driver asked for and the ASIO driver picked come back; settings that never
    /// asked for ASIO don't mention it.
    #[test]
    fn audio_driver_round_trips() {
        let text = Settings::default().to_text();
        assert!(!text.contains("audio_driver") && !text.contains("asio_device"), "{text}");
        assert_eq!(Settings::parse(&text).audio_driver, AudioDriver::System);
        let mut s = Settings::default();
        s.set_output_device(Some("Speakers (BEHRINGER UMC 204HD 192k)".into()));
        s.set_audio_driver(AudioDriver::Asio);
        s.set_asio_device(Some("UMC ASIO Driver".into()));
        let back = Settings::parse(&s.to_text());
        assert_eq!(back.audio_driver, AudioDriver::Asio);
        assert_eq!(back.asio_device.as_deref(), Some("UMC ASIO Driver"));
        assert_eq!(back.output_device.as_deref(), Some("Speakers (BEHRINGER UMC 204HD 192k)"), "the system driver's device is kept");
        // Back to the system's: ASIO's device is kept for next time, and isn't asked for.
        let mut s = back;
        s.set_audio_driver(AudioDriver::System);
        let back = Settings::parse(&s.to_text());
        assert_eq!(back.audio_driver, AudioDriver::System);
        assert_eq!(back.asio_device.as_deref(), Some("UMC ASIO Driver"));
    }

    /// Each driver's devices keep their routing, and the ASIO driver has one routing for
    /// each side, saved like any other device's.
    #[test]
    fn asio_keeps_its_own_routing() {
        let mut s = Settings::default();
        s.set_output_device(Some("Speakers".into()));
        s.output_routing[0] = Some(1);
        s.set_audio_driver(AudioDriver::Asio);
        s.set_asio_device(Some("UMC ASIO Driver".into()));
        assert_eq!(s.output_routing, default_routing(), "the ASIO driver starts from the defaults");
        s.output_routing[0] = Some(2);
        s.input_routing[0] = Some(3);
        let mut back = Settings::parse(&s.to_text());
        assert_eq!((back.output_routing[0], back.input_routing[0]), (Some(2), Some(3)), "the ASIO driver's own");
        back.set_audio_driver(AudioDriver::System);
        assert_eq!(back.output_routing[0], Some(1), "the system driver's device's own");
        back.set_audio_driver(AudioDriver::Asio);
        assert_eq!(back.output_routing[0], Some(2));
    }

    #[test]
    fn group_load_options_round_trip_and_default_on() {
        let s = Settings::parse("");
        assert!(s.load_group_routing && s.load_group_patterns);
        assert!(!s.to_text().contains("load_group"), "defaults aren't written");
        for (routing, patterns) in [(false, true), (true, false), (false, false)] {
            let s = Settings { load_group_routing: routing, load_group_patterns: patterns, ..Settings::default() };
            let back = Settings::parse(&s.to_text());
            assert_eq!((back.load_group_routing, back.load_group_patterns), (routing, patterns));
        }
    }

    #[test]
    fn controller_setting_round_trips() {
        // A new install: DECKARD's own layer, in Palenight.
        assert_eq!(Settings::default().controller, ControllerChoice::Deckard);
        assert_eq!(Settings::parse("").controller, ControllerChoice::Deckard);
        assert_eq!(Settings::parse("").controller_theme, "palenight");
        for c in [ControllerChoice::Automatic, ControllerChoice::Deckard, ControllerChoice::Scripts] {
            let s = Settings { controller: c, ..Settings::default() };
            assert_eq!(Settings::parse(&s.to_text()).controller, c);
        }
        for theme in ["solarized_dark", "maschine", "palenight"] {
            let s = Settings { controller_theme: theme.into(), ..Settings::default() };
            assert_eq!(Settings::parse(&s.to_text()).controller_theme, theme);
        }
    }

    /// Settings saved before devices kept their own routing: the input device's routes
    /// are as they were (straight through, but for the ones listed).
    #[test]
    fn old_input_routes_are_kept() {
        let s = Settings::parse("input_device = Box\ninput_route = 3:none\n");
        assert_eq!(s.input_device.as_deref(), Some("Box"));
        assert_eq!(s.input_routing[..5], [Some(0), Some(1), Some(2), None, Some(4)]);
        assert_eq!(s.output_routing, default_routing());
    }

    /// Names: an alias, else the channel's own; a pair is its alias when both sides
    /// share it, else both names. Not connected: not offered.
    #[test]
    fn aliases_name_channels_and_pairs() {
        let mut s = Settings::default();
        assert_eq!((s.input_name(3), s.input_pair_name(1), s.output_pair_name(1)), ("In 2 R".into(), "In 2 L+R".into(), "Ext. 2".into()));
        s.input_aliases[2] = "Synth L".into();
        assert_eq!(s.input_pair_name(1), "Synth L / In 2 R");
        s.output_aliases[0] = "Main".into();
        s.output_aliases[1] = "Main".into();
        assert_eq!(s.output_pair_name(0), "Main");
        assert!(s.input_pair_on(0) && !s.input_pair_on(1) && s.output_pair_on(0) && !s.output_pair_on(1));
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::{Entry, Kind, unchanged_scan};

    #[test]
    fn a_scan_that_found_nothing_is_tried_again() {
        let entry =
            Entry { kind: Kind::Vst3, vendor: "V".into(), name: "P".into(), instrument: true, location: "a.vst3".into(), cid: "00".into() };
        let known = vec![("a.vst3".to_string(), 5, vec![entry.clone()]), ("failed.vst3".to_string(), 5, Vec::new())];
        assert_eq!(unchanged_scan(&known, "a.vst3", 5), Some(&vec![entry]));
        assert_eq!(unchanged_scan(&known, "a.vst3", 6), None, "the bundle changed");
        assert_eq!(unchanged_scan(&known, "failed.vst3", 5), None);
        assert_eq!(unchanged_scan(&known, "new.vst3", 5), None);
    }
}

#[cfg(test)]
mod recording_tests {
    #[test]
    fn recording_preferences_are_kept() {
        let mut s = super::Settings::default();
        assert_eq!((s.count_in_bars, s.input_quantize), (1, false));
        s.count_in_bars = 4;
        s.input_quantize = true;
        let back = super::Settings::parse(&s.to_text());
        assert_eq!((back.count_in_bars, back.input_quantize), (4, true));
    }
}

#[cfg(test)]
mod sync_tests {
    use super::{MidiSync, Settings};

    /// The standard folders are scanned whatever the settings say (a settings file
    /// from an older build, or from another platform, listed its own); only folders
    /// added in Preferences are saved.
    #[test]
    fn standard_folders_are_always_scanned_and_only_additions_saved() {
        let standard = Settings::standard_dirs();
        assert!(!standard.is_empty());
        assert!(Settings::default().dirs.is_empty());
        assert_eq!(Settings::default().scan_dirs(), standard);
        let foreign = std::path::PathBuf::from("/Library/Audio/Plug-Ins/VST3 from a Mac");
        let text = format!("dir = {}\ndir = {}\ndir = {}\n", standard[0].display(), foreign.display(), foreign.display());
        let s = Settings::parse(&text);
        assert_eq!(s.dirs, vec![foreign.clone()], "the standard folder isn't an addition; the addition once");
        let scanned = s.scan_dirs();
        assert_eq!(&scanned[..standard.len()], &standard[..]);
        assert_eq!(scanned.last(), Some(&foreign));
        let saved = s.to_text();
        assert!(saved.contains("dir = /Library/Audio/Plug-Ins/VST3 from a Mac\n"));
        assert_eq!(saved.matches("dir = ").count(), 1, "only the addition is saved:\n{saved}");
        assert!(!saved.contains("dirs = none"));
        assert_eq!(Settings::parse(&saved).dirs, s.dirs);
        // An old file saying there are no folders at all: the standard ones still.
        assert_eq!(Settings::parse("dirs = none\n").scan_dirs(), standard);
    }

    /// The default project (Preferences ▸ General, BUGS #10): none unless set.
    #[test]
    fn app_icon_round_trips() {
        use crate::icon::Style;
        assert_eq!(Settings::parse("").app_icon, Style::Television);
        let s = Settings { app_icon: Style::Letter, ..Settings::default() };
        assert_eq!(Settings::parse(&s.to_text()).app_icon, Style::Letter);
        assert_eq!(Settings::parse("app_icon = gone").app_icon, Style::Television, "an unknown one: the default");
    }

    #[test]
    fn last_version_round_trips() {
        assert_eq!(Settings::parse("").last_version, "");
        let s = Settings { last_version: "0.2.0".into(), ..Settings::default() };
        assert_eq!(Settings::parse(&s.to_text()).last_version, "0.2.0");
    }

    #[test]
    fn default_project_round_trips() {
        assert_eq!(Settings::default().default_project, None);
        let s = Settings::parse("default_project = /Users/me/Template Song.mxprj\n");
        assert_eq!(s.default_project.as_deref(), Some(std::path::Path::new("/Users/me/Template Song.mxprj")));
        assert_eq!(Settings::parse(&s.to_text()).default_project, s.default_project);
        assert_eq!(Settings::parse(&Settings::default().to_text()).default_project, None);
    }

    /// The divider above the pattern editor stays where it was dragged; never dragged, the
    /// window's size decides.
    #[test]
    fn song_panel_height_round_trips() {
        assert_eq!(Settings::parse(&Settings::default().to_text()).song_panel_h, None);
        let s = Settings { song_panel_h: Some(412.5), ..Settings::default() };
        assert_eq!(Settings::parse(&s.to_text()).song_panel_h, Some(412.5));
        assert_eq!(Settings::parse("song_panel_h = nonsense\n").song_panel_h, None);
        assert_eq!(Settings::parse("song_panel_h = inf\n").song_panel_h, None);
    }

    /// Render ahead (Preferences ▸ Audio): off unless chosen (every sound plays live,
    /// as in Maschine); a choice is kept.
    #[test]
    fn render_ahead_is_off_by_default() {
        assert_eq!(Settings::default().render_ahead_ms, 0);
        assert_eq!(Settings::parse("").render_ahead_ms, 0);
        let s = Settings::parse("render_ahead_ms = 20\n");
        assert_eq!(Settings::parse(&s.to_text()).render_ahead_ms, 20);
    }

    /// MPE inputs (Preferences ▸ MIDI): each port's member bend range.
    #[test]
    fn mpe_setting_round_trips() {
        let s = Settings::parse("mpe = 48 Hydrasynth Explorer\nmpe = 24 Seaboard\n");
        assert_eq!(s.mpe.get("Hydrasynth Explorer"), Some(&48));
        assert_eq!(s.mpe.get("Seaboard"), Some(&24));
        let back = Settings::parse(&s.to_text());
        assert_eq!(back.mpe, s.mpe);
        assert!(Settings::default().mpe.is_empty(), "off by default");
    }

    /// MPE outputs (Preferences ▸ MIDI): each port's member bend range; DECKARD sets
    /// MPE synths up unless switched off.
    #[test]
    fn drum_tracks_round_trip() {
        let s = Settings::parse("drum_track = 1 launchpad_pro_mk3\n");
        assert_eq!(s.drum_tracks.get("launchpad_pro_mk3"), Some(&1));
        assert_eq!(Settings::parse(&s.to_text()).drum_tracks, s.drum_tracks);
    }

    #[test]
    fn drum_channels_round_trip() {
        assert!(Settings::default().drum_channels.is_empty(), "off");
        let s = Settings::parse("drum_channel = 10 launchpad_pro_mk3\ndrum_channel = 99 other\n");
        assert_eq!(s.drum_channels.get("launchpad_pro_mk3"), Some(&10));
        assert_eq!(s.drum_channels.get("other"), Some(&16), "clamped");
        assert_eq!(Settings::parse(&s.to_text()).drum_channels, s.drum_channels);
    }

    #[test]
    fn mpe_out_settings_round_trip() {
        let d = Settings::default();
        assert!(d.mpe_out.is_empty() && d.mpe_setup, "normal ports, set-up on");
        let s = Settings::parse("mpe_out = 48 Hydrasynth Explorer\nmpe_setup = false\n");
        assert_eq!(s.mpe_out.get("Hydrasynth Explorer"), Some(&48));
        assert!(!s.mpe_setup);
        let back = Settings::parse(&s.to_text());
        assert_eq!((back.mpe_out, back.mpe_setup), (s.mpe_out, false));
        assert!(Settings::parse(&d.to_text()).mpe_setup);
    }

    /// Link and MIDI sync (Preferences ▸ MIDI): off by default, kept as set; offsets
    /// stay in Maschine's ranges.
    #[test]
    fn sync_preferences_are_kept() {
        let mut s = Settings::default();
        assert_eq!((s.link, s.link_start_stop, s.midi_sync), (false, false, MidiSync::Off));
        assert_eq!((s.clock_send_offset_ms, s.clock_receive_offset_ms), (0, 0));
        s.link = true;
        s.link_start_stop = true;
        s.midi_sync = MidiSync::Send;
        s.clock_send_offset_ms = 12;
        s.clock_receive_offset_ms = -30;
        let back = Settings::parse(&s.to_text());
        assert_eq!((back.link, back.link_start_stop, back.midi_sync), (true, true, MidiSync::Send));
        assert_eq!((back.clock_send_offset_ms, back.clock_receive_offset_ms), (12, -30));
        let s = Settings::parse("midi_sync = receive\nclock_send_offset_ms = 900\nclock_receive_offset_ms = -500\n");
        assert_eq!((s.midi_sync, s.clock_send_offset_ms, s.clock_receive_offset_ms), (MidiSync::Receive, 200, -100));
    }
}

#[cfg(test)]
mod wine_tests {
    use super::Settings;

    /// Syncing Windows plug-ins before a rescan (Linux) is on unless switched off, and
    #[test]
    fn the_focus_follows_unless_switched_off() {
        let mut s = Settings::default();
        assert!(s.focus_follows);
        assert!(Settings::parse("").focus_follows, "a settings file from before");
        s.focus_follows = false;
        assert!(!Settings::parse(&s.to_text()).focus_follows);
    }

    #[test]
    fn touchstrips_are_absolute_unless_switched_on() {
        let mut s = Settings::default();
        assert!(!s.relative_strips);
        assert!(!Settings::parse("").relative_strips, "a settings file from before");
        s.relative_strips = true;
        assert!(Settings::parse(&s.to_text()).relative_strips);
    }

    /// only "off" is written, so other systems' settings files don't change.
    #[test]
    fn bridging_windows_plugins_is_on_unless_switched_off() {
        let mut s = Settings::default();
        assert!(s.bridge_windows);
        assert!(!s.to_text().contains("bridge_windows"));
        s.bridge_windows = false;
        assert!(s.to_text().contains("bridge_windows = false\n"));
        assert!(!Settings::parse(&s.to_text()).bridge_windows);
        assert!(Settings::parse("").bridge_windows);
    }
}
