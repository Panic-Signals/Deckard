//! DECKARD's own MK3 controller layer (docs/superpowers/specs/
//! 2026-10-25-controller-layer-design.md): the MK3 working with no Maschine install.
//!
//! The behaviour is DECKARD's own Lua scripts, one file per page (`lua/`, shipped in the
//! binary), which a user folder can replace file by file. This crate runs them: MK3
//! events and a [`View`] of the app go in; [`Action`]s for the app, the LEDs, the pads'
//! setup and both screens come out. The scripts' API is `docs/controller-api.md`.
//! Nothing here is from NI's controller scripts.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use deckard_mk3::Event;
use deckard_mk3::leds::Leds;
use mlua::{Function, Lua, LuaOptions, StdLib, Table, Value};

mod api;
pub mod device;
mod leds;
mod names;
mod screen;
mod sources;

pub use sources::SHIPPED;

/// What the layer shows of the app, filled by the app each frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct View {
    pub project: String,
    pub tempo: f64,
    /// The song's swing, 0–100 %.
    pub swing: f64,
    /// The master level as stored (0..1, 0.75 unity).
    pub master_volume: f64,
    /// The master level in dB (−∞ silent).
    pub master_db: f64,
    pub playing: bool,
    pub recording: bool,
    pub looping: bool,
    pub metronome: bool,
    pub auto_write: bool,
    /// When the Smart Strip shows the play position with no strip mode on: 0 never, 1
    /// while recording, 2 while playing (HARDWARE's SHOW POSITION).
    pub strip_position: i64,
    /// Touch Auto-write: a knob touched with Auto-write on writes its modulation.
    pub touch_auto_write: bool,
    pub fixed_velocity: bool,
    /// The velocity the pads play with Fixed Velocity on (0–127).
    pub velocity: i64,
    /// Keyboard and Chords mode's scale: its bank (0 Main … 7 Minor) and its place in
    /// the bank (0..15), as `deckard_model::scale_tables` lists them.
    pub scale_bank: i64,
    pub scale_kind: i64,
    /// The user chords: 16 sets of 12, each the semitones above the root it plays
    /// (none: an empty chord).
    pub user_chords: Vec<Vec<Vec<i64>>>,
    /// Follow: the software's views follow the playhead.
    pub follow: bool,
    /// The grids the Grid page (SHIFT + FOLLOW) sets.
    pub grid: GridView,
    /// The projects opened lately, most recent first (their names).
    pub recent: Vec<String>,
    /// What the Settings page (SETTINGS) shows.
    pub settings: SettingsView,
    /// Lock's snapshots and settings.
    pub lock: LockView,
    /// The project's LFOs (by pad), and the pad the software panel and the MK3 show.
    pub lfos: Vec<LfoView>,
    pub lfo_focus: usize,
    /// The mod matrix page's slots: every route in the project (by group, sound and the
    /// route's place on the sound), then the slot being made or an empty one.
    pub mod_slots: Vec<ModSlotView>,
    /// Note Repeat's and the Arp's settings (NOTE REPEAT).
    pub note_repeat: RepeatView,
    /// Ticks (960 a quarter) into what plays, while playing.
    pub position: Option<i64>,
    /// How far through its pattern the focused group is (0..1), while playing.
    pub progress: Option<f64>,
    pub numerator: i64,
    pub denominator: i64,
    pub groups: Vec<GroupView>,
    /// The focused scene's name and palette colour.
    pub scene_name: String,
    pub scene_color: usize,
    /// The scenes, and the focused one (from 0).
    pub scenes: Vec<SceneView>,
    pub scene: usize,
    /// The scene playing (in the Ideas view, while playing), and the one queued next.
    pub playing_scene: Option<usize>,
    pub queued_scene: Option<usize>,
    /// ARRANGE shows the Song view (the sections), not the Ideas view.
    pub song_view: bool,
    /// The arrangement's sections, and the focused one (from 0).
    pub sections: Vec<SectionView>,
    pub section: Option<usize>,
    /// The focused group's focused pattern (from 0): where it starts and how long it is
    /// (ticks), its notes, and where playback is in it.
    pub pattern: usize,
    pub pattern_start: i64,
    pub pattern_length: i64,
    /// Its length grows as it's recorded into (none chosen).
    pub pattern_auto: bool,
    pub notes: Vec<NoteView>,
    pub pattern_position: Option<i64>,
    pub group: usize,
    pub sound: usize,
    pub control: ControlView,
    /// The library as BROWSE shows it.
    pub browser: BrowserView,
    /// What the Sampling page (SAMPLE) shows.
    pub sampling: SamplingView,
}

/// The app's sampler and recorder as the Sampling page's tabs show them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SamplingView {
    /// The tab: 0 RECORD, 1 EDIT, 2 SLICE, 3 ZONE.
    pub tab: usize,
    /// Each tab's knob pages (SLICE's knobs are its slicer's: `slice`).
    pub pages: [Vec<KnobPage>; 4],
    pub slice: Vec<SamplingKnob>,
    /// The focused zone's sample, if it has one.
    pub sample: Option<SampleView>,
    pub record: RecordState,
    /// How many recordings have been taken, and the one focused.
    pub takes: usize,
    pub take: Option<usize>,
    /// A stem separation running.
    pub stems: Option<StemsRun>,
}

/// The library as BROWSE shows it: the content kind (from 0: Projects, Groups,
/// Sounds, Instruments, Effects, Loops, One-shots), user content (else factory), the
/// products and the one chosen, the three attributes (Type, Sub-type, Character: the
/// values and the one chosen), the results (name, a favourite) and the one focused.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserView {
    pub kind: usize,
    pub user: bool,
    pub products: Vec<String>,
    pub product: Option<usize>,
    pub attributes: [(Vec<String>, Option<usize>); 3],
    pub results: Vec<(String, bool)>,
    pub focus: usize,
    /// A search is running.
    pub busy: bool,
    /// Only favourites are shown.
    pub favorites: bool,
}

/// What BROWSE asks of the library.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Browse {
    Kind(usize),
    User(bool),
    Product(Option<usize>),
    /// Attribute `.0` (0 Type, 1 Sub-type, 2 Character) to value `.1` (`None`: all).
    Attribute(usize, Option<usize>),
    /// Move the result focus by this much.
    Focus(i64),
    /// Load the focused result.
    Load,
    Favorites,
    /// Make the focused result a favourite, or not.
    SetFavorite,
    /// Prehear the focused result.
    Prehear,
}

/// The song's grids, as Maschine's Grid page lists them (each value its place in the
/// list, from 0).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GridView {
    /// The Perform Grid scene and section changes wait for: 0 Off, then 1 Bar, 1/2,
    /// 1/4, 1/8, 1/16 and Section.
    pub perform: i64,
    /// The Arrange Grid (1 Bar, 1/2, 1/4, 1/8, 1/16), on or off, and Quick lengths.
    pub arrange: i64,
    pub arrange_snap: bool,
    pub quick: bool,
    /// The Step Grid (1 Bar .. 1/128, then the triplets), on or off.
    pub step: i64,
    pub step_snap: bool,
    /// The Step Grid's step (ticks).
    pub step_ticks: i64,
    /// The Nudge Grid: the step over 1, 2, 4, 8 or 16.
    pub nudge: i64,
}

/// An LFO as the page shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LfoView {
    /// Its pad, 1-16.
    pub pad: usize,
    pub shape: String,
    pub sync: bool,
    /// The rate as written: "1/4", or "2.50 Hz".
    pub rate: String,
    /// Degrees, 0-360.
    pub phase: f64,
    /// 0..1.
    pub level: f64,
    /// Steps per cycle; 0 is Off.
    pub quantize: i64,
    /// "Off", or "A1 / S3" (group A1, as Maschine names them, sound 3).
    pub trigger: String,
    /// Beats, 0-16.
    pub fade: f64,
    /// Muted: its routes leave their targets alone.
    pub muted: bool,
    /// Envelope mode (docs/superpowers/specs/2026-10-29-lfo-envelope-design.md): its
    /// trigger's notes play an ADSR.
    pub env: bool,
    /// The envelope's times as written: "12 ms", "1.20 s".
    pub attack: String,
    pub decay: String,
    pub release: String,
    /// 0..1.
    pub sustain: f64,
    /// Each new note restarts the attack.
    pub retrigger: bool,
    /// "LFO 3", or "ENV 3" in Envelope mode.
    pub name: String,
    /// Its output over two cycles (240 samples); an envelope's ADSR.
    pub points: Vec<f32>,
    /// Where an envelope's picture shows the note off (0..1 of its width).
    pub note_off: Option<f32>,
}

/// A mod matrix slot as its column shows it: Source, Group ("A1 – name", "A1" unnamed), Sound
/// ("S3 – name"), Module, Destination and Depth ("+50%"), "—" where unset. `midi`: a CC
/// route (its Group "MIDI"), whose rows 3–5 are Port, Channel ("1"–"16") and CC ("0"–"119").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModSlotView {
    pub rows: [String; 6],
    pub midi: bool,
}

/// Lock (LOCK, SHIFT + LOCK).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LockView {
    /// LOCK lit: a snapshot taken, to recall at the next press.
    pub active: bool,
    /// The slots holding a snapshot, and the focused one (from 0; 16 a bank).
    pub slots: Vec<usize>,
    pub focus: Option<usize>,
    /// The bank shown (from 0) and how many there are (1-4).
    pub bank: usize,
    pub banks: usize,
    /// Morph on; its mode (0 Travel, 1 Target), Travel's time (1/2 Bar, 1 Bar, 2, 4, 8,
    /// 16 Bars) and Target's grid (1/8, 1/4, 1/2, Bar), as places in those lists.
    pub morph: bool,
    pub mode: usize,
    pub time: usize,
    pub grid: usize,
}

/// Note Repeat and the Arp (NOTE REPEAT).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RepeatView {
    /// Each setting, by the name [`Action::Repeat`] sets it by: `type`, `rate`,
    /// `rate_unit`, `sequence`, `octaves`, `dynamic`, `gate`, `retrigger`, `repeat`,
    /// `offset`, `inversion`, `min_key`, `max_key`.
    pub settings: Vec<RepeatSetting>,
    /// The four presets' names (display buttons 5-8), and the one chosen (from 0).
    pub presets: Vec<String>,
    pub preset: usize,
    /// HOLD: the notes keep repeating once let go.
    pub hold: bool,
    /// LOCK: on off the page too; and on now.
    pub locked: bool,
    pub active: bool,
}

/// A Note Repeat setting: its knob's name, its value shown and as a number, its range
/// and step, and how far a knob turns for a step.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RepeatSetting {
    pub key: String,
    pub name: String,
    pub text: String,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub turn: f64,
}

/// The app's settings as the Settings page shows them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettingsView {
    /// Each setting's value as shown, by the name [`Action::Setting`] steps it by
    /// (`count_in` "1 Bar", `buffer_size` "256", `in_route` 3 as `in_route_3` …).
    pub values: std::collections::BTreeMap<String, String>,
    /// The MIDI ports, and whether each is switched on.
    pub midi_inputs: Vec<(String, bool)>,
    pub midi_outputs: Vec<(String, bool)>,
    /// The lines beside AUDIO's knobs: the device's status, its latencies.
    pub audio_status: Vec<String>,
    /// Link joined, and the other applications in its session.
    pub link: bool,
    pub link_peers: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupView {
    pub name: String,
    pub rgb: [u8; 3],
    /// Its palette index (0..17, 16 white), as the lights show it.
    pub color: usize,
    pub muted: bool,
    /// It has a pattern in the scene playing (while playing).
    pub playing: bool,
    /// The Mixer: its level (dB, −∞ silent), pan (−1..1), meters (left, right;
    /// amplitude), headroom (its loudest since reset; amplitude) and plug-ins.
    pub mix: MixView,
    /// Its Perform FX, if it has one: the strip on it (touched), and where (0..1).
    pub perform: Option<(bool, f64)>,
    pub sounds: Vec<SoundView>,
    /// It has no colour of its own.
    pub default_color: bool,
    /// Its patterns, and the one the focused scene plays (from 0).
    pub patterns: Vec<PatternView>,
    pub scene_pattern: Option<usize>,
    /// How far through its pattern in the scene the song is (0..1), while it plays.
    pub progress: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SoundView {
    pub name: String,
    pub rgb: [u8; 3],
    /// Its palette index (0..17, 16 white), as the lights show it.
    pub color: usize,
    pub muted: bool,
    /// It has something in it: a plug-in, a name, notes or a MIDI output.
    pub used: bool,
    /// One of its notes is sounding.
    pub playing: bool,
    /// The Mixer: its level (dB, −∞ silent), pan (−1..1), meters (left, right;
    /// amplitude), headroom (its loudest since reset; amplitude) and plug-ins.
    pub mix: MixView,
    /// Pad mode's Choke and Link: group 0 none, else 1–8; mode 0 Send, 1 Receive.
    pub choke_group: i64,
    pub choke_mode: i64,
    pub link_group: i64,
    pub link_mode: i64,
    /// Pad mode's base key (MIDI note; 60 C3).
    pub base_key: i64,
    /// It has no colour of its own (it takes its group's).
    pub default_color: bool,
    /// Its Pad Touch (`deckard_model::pad_touch`): mode 0 Velocity, 1 Poly AT,
    /// 2 Aftertouch, 3 CC; the CC number; curve, min and max in percent.
    pub touch_mode: i64,
    pub touch_cc: i64,
    pub touch_curve: i64,
    pub touch_min: i64,
    pub touch_max: i64,
    /// PROB LOCK: its notes' chances rolled once and kept.
    pub prob_locked: bool,
}

/// A scene: its name and palette colour.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneView {
    pub name: String,
    pub color: usize,
    /// Its colour as RGB, for the lights.
    pub rgb: [u8; 3],
    /// Its colour is the default (none chosen).
    pub default_color: bool,
    /// The pattern each group plays in it (from 0), by group.
    pub patterns: Vec<Option<usize>>,
}

/// A section of the arrangement: the scene it plays (from 0), where it starts and how
/// long it is (ticks); its length follows its scene's (Auto).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SectionView {
    pub scene: Option<usize>,
    pub start: i64,
    pub length: i64,
    pub auto: bool,
}

/// A pattern: its name, palette colour and length in bars.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PatternView {
    pub name: String,
    pub color: usize,
    /// Its colour as RGB, for the lights.
    pub rgb: [u8; 3],
    /// Its colour is the default (its group's).
    pub default_color: bool,
    pub bars: f64,
}

/// A note of the focused pattern: its sound (from 0), where it starts and how long it
/// is (ticks), its pitch (MIDI), velocity (1–127) and probability (0–100 %), and
/// whether it's selected.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NoteView {
    pub sound: usize,
    pub position: i64,
    pub length: i64,
    pub pitch: i64,
    pub velocity: i64,
    pub probability: i64,
    pub selected: bool,
}

/// A channel as the Mixer shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MixView {
    pub level: f64,
    pub pan: f64,
    pub meter: (f32, f32),
    pub headroom: f32,
    /// Its plug-ins in order: (an instrument, on).
    pub plugins: Vec<(bool, bool)>,
}

/// The app's Control area: what the eight knobs turn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ControlView {
    /// 0 master, 1 group, 2 sound.
    pub level: usize,
    /// Plug-ins shown (else the channel's properties).
    pub modules_visible: bool,
    pub page_group: usize,
    pub page_groups: usize,
    /// The focused slot's name, if any.
    pub slot: Option<String>,
    /// How many slots the chain has, and which is focused (from 0).
    pub slot_count: usize,
    pub slot_index: Option<usize>,
    /// The chain's slots, in order.
    pub chain: Vec<SlotView>,
    pub pages: Vec<String>,
    pub page: usize,
    /// Eight positions; `None` where the page has no knob.
    pub knobs: Vec<Option<KnobView>>,
    /// What the plug-in browser (the Module page) lists, in order (only while it
    /// shows).
    pub modules: Vec<ModuleView>,
}

/// One instrument, effect or plug-in the plug-in browser can load.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModuleView {
    pub name: String,
    pub instrument: bool,
    /// "Internal" (Maschine's and DECKARD's own), "VST3", "Audio Units", "VST".
    pub format: String,
    /// The maker ("Native Instruments" counts as Maschine's own, NI).
    pub vendor: String,
}

/// A slot of the focused chain.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SlotView {
    pub name: String,
    /// An instrument (else an effect).
    pub instrument: bool,
    /// Switched on (not bypassed).
    pub active: bool,
    /// A hosted plug-in (it has an editor window).
    pub plugin: bool,
    /// DECKARD's picture of it ([`Surface::add_picture`]'s key), for one of its own.
    pub picture: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct KnobView {
    pub name: String,
    /// The section of the plug-in it's in ("Voice Settings").
    pub section: String,
    pub text: String,
    /// 0..1.
    pub value: f64,
    /// Its movements can be written as modulation (a continuous knob of a sound's or
    /// group's plug-in).
    pub modulatable: bool,
    /// On the Step Mod page: its modulation in the step shown (a deviation, "+25%"),
    /// if the step has one.
    pub step_mod: Option<String>,
    /// With Auto-write on, its modulation where the song is, as Maschine shows it while
    /// writing ("+25%", "0%"); `None` when off or not modulatable.
    pub auto_mod: Option<String>,
    /// A macro controls its parameter (the macro icon at the top right of its name).
    pub in_macro: bool,
}

pub use deckard_model::sampling_view::{KnobPage, RecordPress, RecordState, SampleView, SamplingCall, SamplingKnob, StemsRun};

/// What [`Action::Color`] colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorOf {
    Pattern,
    Scene,
    Group,
    Sound,
}

/// What the layer asks of the app.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Play,
    Stop,
    Record {
        on: bool,
        count_in: bool,
    },
    Restart,
    Loop(bool),
    Metronome(bool),
    AutoWrite(bool),
    /// Set the tempo (BPM), the swing (0–100 %) or the master level (0..1).
    Tempo(f64),
    Swing(f64),
    Volume(f64),
    /// The master level in dB (at most +10).
    VolumeDb(f64),
    FocusGroup(usize),
    /// Focus sound `.0` of the focused group.
    FocusSound(usize),
    /// Mute or unmute group `.0` (or sound `.0` of the focused group); solo it.
    MuteGroup(usize),
    SoloGroup(usize),
    MuteSound(usize),
    SoloSound(usize),
    /// Erase sound `.0`'s notes from the focused pattern.
    EraseSound(usize),
    /// PROB LOCK of the focused sound on or off.
    ProbLock(bool),
    /// Sound `.0` of the focused group removed: an empty sound in its place (SHIFT +
    /// ERASE + its pad).
    ResetSound(usize),
    /// Turn Control knob `knob` (0..8) by `inc` (a fraction of a turn).
    Turn {
        knob: usize,
        inc: f64,
        fine: bool,
    },
    /// The next (true) or previous page.
    Page(bool),
    /// Show the master (0), group (1) or sound (2) chain.
    Level(usize),
    /// Focus the slot `.0` places on.
    Slot(i64),
    ModulesVisible(bool),
    PageGroup(usize),
    FixedVelocity(bool),
    KeyboardMode(bool),
    /// Set one of the focused sound's Pad mode settings: `choke_group`, `link_group`
    /// (0 none, 1–8), `choke_mode`, `link_mode` (0 Send, 1 Receive).
    SoundSetting(&'static str, i64),
    /// Set group `group`'s level (or its sound `sound`'s; both from 0) to `db` (−∞ for
    /// silence), or its pan (−1..1).
    ChannelLevel {
        group: usize,
        sound: Option<usize>,
        db: f64,
    },
    ChannelPan {
        group: usize,
        sound: Option<usize>,
        pan: f64,
    },
    /// Focus scene `.0` (while a scene plays, it plays next).
    FocusScene(usize),
    /// Launch scene `.0`: as clicking it in the Ideas view (focused; while a scene
    /// plays, it starts when the playing one ends).
    LaunchScene(usize),
    /// The focused group plays its pattern `.0` in the focused scene, focused.
    PickPattern(usize),
    /// Group `group` plays pattern `pattern` in the focused scene, or none (both from 0).
    ScenePattern {
        group: usize,
        pattern: Option<usize>,
    },
    /// The focused pattern from tick `start` for `length` ticks; or a length chosen for
    /// it (no longer Auto).
    PatternSpan {
        start: i64,
        length: i64,
    },
    PatternLength(i64),
    /// Colour the focused pattern, scene, group or sound: a palette colour (0–15), or
    /// `None` for the default.
    Color {
        of: ColorOf,
        color: Option<u8>,
    },
    /// The velocity the pads play with Fixed Velocity on (0–127).
    VelocityValue(i64),
    /// Keyboard and Chords mode's scale: the root (MIDI note), bank and kind (from 0).
    Scale {
        root: i64,
        bank: i64,
        kind: i64,
    },
    /// One of the focused sound's Pad Touch settings (`mode`, `cc`, `curve`, `min`,
    /// `max`), as [`SoundView`]'s `touch_` fields.
    PadTouch(&'static str, i64),
    /// Follow playback on or off.
    Follow(bool),
    /// A FILE page command: `new`, `save` (the project), `save_copy`, `export_audio`,
    /// `save_group` / `save_sound` (the focused one), or `load` recent project `n`
    /// (from 0, as [`View::recent`]).
    File(&'static str, usize),
    /// One of the song's grids (`perform`, `arrange`, `arrange_snap`, `quick`, `step`,
    /// `step_snap`, `nudge`), as [`GridView`]'s fields (on / off as 1 / 0).
    Grid(&'static str, i64),
    /// A step up or down (`step` ±1; on / off for a switch) of one of the app's
    /// settings (the Settings page): `count_in`, `numerator`, `denominator`, `quantize`,
    /// `device`, `buffer_size`, `in_route` / `out_route` (channel `index`, from 0),
    /// `midi_sync`, `clock_offset`, `default_input`, `midi_in` / `midi_out` (port
    /// `index`), `link`, `link_start_stop`, `sensitivity`, `display_brightness`, `metronome_volume`, `metronome_auto`.
    Setting(&'static str, usize, i64),
    /// Set Macro (SHIFT + MACRO, a knob touched): knob `knob`'s parameter (from 0)
    /// becomes a macro of the master (`level` 0), the focused group (1) or sound (2), or
    /// stops being one.
    SetMacro {
        knob: usize,
        level: usize,
    },
    /// Lock: `button` (LOCK), `pad` (pad `.1` of the bank shown, from 0: recall or
    /// take), `update`, `delete` (the focused snapshot), `delete_slot` (slot `.1`),
    /// `bank` (1 the next, 0 the previous), or a setting: `morph` (1 on), `morph_mode`,
    /// `morph_time`, `morph_grid` (places in their lists).
    Lock(&'static str, usize),
    /// An LFO edit: (what, pad, value). `what` is "focus", "create", "remove", "mute",
    /// "env" (Envelope mode), "retrigger" (each toggles), "shape" (1-5), "sync" (0/1),
    /// "trigger_focused" (0 Off, 1 the focused sound), or a knob turned `value` steps:
    /// "rate", "phase", "level", "quantize", "trigger", "fade", "attack", "decay",
    /// "sustain", "release".
    Lfo(&'static str, usize, f64),
    /// A mod matrix slot's row (0 Source, 1 Group, 2 Sound, 3 Module, 4 Destination,
    /// 5 Depth) turned some steps: (slot, row, steps). 0 steps erases the slot.
    ModSlot(usize, usize, i64),
    /// The mod matrix page closed: the slot being made is dropped (no route changes).
    ModSlotsClosed,
    /// Note Repeat: setting `.0` ([`RepeatSetting::key`]) to `.1`; `hold` and `locked`
    /// (1 on), `preset` (1-4).
    Repeat(&'static str, f64),
    /// The Smart Strip: `pitch` (bend, -1..1, 0 the middle) or `mod` (the mod wheel,
    /// 0..1) of the focused sound; the focused group's Perform FX touched
    /// (`perform_touch`), moved (`perform`) or let go (`perform_release`) at `.1` (0..1).
    Strip(&'static str, f64),
    /// The Smart Strip's NOTES: a note of the focused group's sound `sound` (from 0)
    /// starts (`on`) or stops.
    StripNote {
        sound: usize,
        pitch: i64,
        velocity: i64,
        on: bool,
    },
    /// SHIFT + PERFORM: the focused group's Perform FX shown (one loaded if it has
    /// none).
    PerformFx,
    /// An edit of the project: `undo` / `redo` (a whole take, as the recording made
    /// it), `undo_step` / `redo_step` (one edit), `copy` / `paste` (the selected notes),
    /// `clear_modulation` (the focused pattern's: the focused sound's in Keyboard mode).
    Edit(&'static str),
    /// The focused sound's channel properties, Groove page `.0` (0 Swing, 1 Variation,
    /// 2 Generate).
    GroovePage(usize),
    /// The Events page's edits of the selected notes (`.1` a step up or down):
    /// `nudge` / `nudge_fine` (by the Nudge Grid, or the finest step), `pitch`,
    /// `velocity`, `length` / `length_fine` (by the Step Grid, or the finest step),
    /// `probability`; of the selection or, with none, the notes shown: `nudge_shown` /
    /// `nudge_shown_fine`, `transpose` (`.1` semitones), `quantize` / `quantize_half`;
    /// `clear_all` (the group's notes; `.1` 1: the focused sound's); the selection:
    /// `select_all`, `select_none`, `select_sound` (sound `.1`'s notes, from 0, in or out
    /// of it, focused), `select_pitch` (the focused sound's notes of pitch `.1`),
    /// `erase_pitch` (delete those), `select_step` (the focused sound's next or previous
    /// note alone).
    Events(&'static str, i64),
    /// Step mode: a step of the focused sound at tick `at`, `length` long: with no note
    /// starting in it, a new one there at the sound's base key and `velocity`
    /// (selected); else its notes go.
    StepToggle {
        at: i64,
        length: i64,
        velocity: i64,
    },
    /// The Step Mod page shows the modulation of the step at `.0` (`.1` long), or is
    /// gone (`None`).
    StepModAt(Option<(i64, i64)>),
    /// The Step Mod page: knob `knob` (0..8) turned by `inc` sets the modulation of the
    /// steps held (`pads`, bit 0 the first pad's; pad `n` the step at `from + n × length`).
    StepMod {
        from: i64,
        length: i64,
        pads: u16,
        knob: usize,
        inc: f64,
    },
    /// Touch Auto-write: Control knob `.0` (0..8) is held, Auto-write on, so its value
    /// is written as modulation where the song is (as often as the pages tick).
    TouchWrite(usize),
    /// The Sampling page shown (the focused sound's sample loaded into it), or gone.
    Sampling(bool),
    /// Its tab: 0 RECORD, 1 EDIT, 2 SLICE, 3 ZONE.
    SamplingTab(usize),
    /// Knob `knob` (from 0) of the tab's page `page` set to `value` (on SLICE, knob
    /// `knob` of [`SamplingView::slice`]).
    SamplingKnob {
        page: usize,
        knob: usize,
        value: f64,
    },
    /// What a Sampling page button asks of the sample, its slices or zones.
    SamplingCall(SamplingCall),
    /// RECORD's START / STOP, or the recording cancelled.
    RecordSample(RecordPress),
    /// RECORD's take `index` (from 0) focused (loaded into the focused sound), or
    /// removed from the list.
    Take {
        index: usize,
        delete: bool,
    },
    /// The focused sound's notes starting from `start` to `end` (ticks) with pitches
    /// from `low` to `high` become the selection.
    EventsRange {
        start: i64,
        end: i64,
        low: i64,
        high: i64,
    },
    /// Add `note` (semitones above the root) to user chord `slot` of set `set` (both
    /// from 0), or take it out if it's there.
    UserChordNote {
        set: usize,
        slot: usize,
        note: i64,
    },
    /// Start (`on`) or end hearing a user chord played from `root`.
    UserChordPreview {
        set: usize,
        slot: usize,
        root: i64,
        on: bool,
    },
    /// Swap a user chord with the one before or after (`next`) it in its set.
    UserChordMove {
        set: usize,
        slot: usize,
        next: bool,
    },
    UserChordDelete {
        set: usize,
        slot: usize,
    },
    /// A new empty group after the last, focused.
    AddGroup,
    /// Group `.0` deleted (the last: reset), SHIFT + ERASE + its button.
    DeleteGroup(usize),
    /// The Pattern page's edits of the focused group's focused pattern: `create` (a new
    /// empty one, in the scene), `double`, `duplicate`, `delete`.
    PatternEdit(&'static str),
    /// The Scene page's edits of the focused scene: `create` (a new one after the
    /// last), `insert` (a new one after it), `duplicate`, `delete`, `clear` (no group
    /// plays a pattern in it), `append` (a section playing it at the end of the
    /// arrangement).
    SceneEdit(&'static str),
    /// ARRANGE shows the Song view (else the Ideas view).
    SongView(bool),
    /// Focus section `.0` (while the song plays, it plays next).
    FocusSection(usize),
    /// The Song view's edits of the focused section: `create` (a new one after it),
    /// `duplicate`, `delete`.
    SectionEdit(&'static str),
    /// The focused section swaps places with the next (`true`) or the previous one.
    SectionMove(bool),
    /// The focused section plays the next (`true`) or the previous scene.
    SectionScene(bool),
    /// The focused section's length (ticks); no longer Auto.
    SectionLength(i64),
    Browse(Browse),
    /// The Plug-in page's SHIFT layer, on the focused slot: `edit` (its plug-in's
    /// window), `move_left` / `move_right` (swap it with the slot before or after),
    /// `bypass` (on or off), `remove`.
    SlotEdit(&'static str),
    /// The plug-in browser's LOAD: entry `.0` of [`ControlView::modules`] (from 0) in
    /// place of the focused slot, or into the "+"; `None` takes the focused slot out.
    LoadModule(Option<usize>),
}

/// Where the pads play.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PadSetup {
    /// Hits play sounds (not while SELECT, MUTE, SOLO or ERASE is held).
    pub play: bool,
    /// Keyboard mode: every pad plays this sound at its own pitch.
    pub keys: Option<(usize, [i64; 16])>,
    /// Chords mode: every pad plays its chord's notes instead (128: none).
    pub chords: Option<[[i64; 16]; 16]>,
}

/// The page shown when the scripts start.
pub const FIRST_PAGE: &str = "control";

/// The theme the screens draw with unless another is picked (`themes/<name>.lua`).
pub const THEME: &str = "maschine";

/// How often the user folder is checked for edited scripts.
const RELOAD_EVERY: Duration = Duration::from_millis(500);

/// What the scripts and the engine share while a handler runs.
pub(crate) struct Shared {
    pub view: View,
    /// Seconds since the surface started.
    pub now: f64,
    /// Buttons held (the scripts' names: `SHIFT`, `PLAY`…), and pads held (0..16).
    pub held: HashSet<String>,
    pub pads: [bool; 16],
    pub actions: Vec<Action>,
    pub page: String,
    pub leds: Leds,
}

/// The loaded scripts.
struct Run {
    lua: Lua,
    shared: Rc<RefCell<Shared>>,
}

pub struct Surface {
    sources: sources::Sources,
    /// The theme's name (`themes/<name>.lua`).
    theme: String,
    run: Option<Run>,
    /// The last script error, shown in the app and on the screens.
    error: RefCell<Option<String>>,
    epoch: Instant,
    checked: Option<Instant>,
    /// The last picture sent, and how many were sent.
    picture: Option<Vec<[u8; 4]>>,
    frames: u64,
    /// Pictures the scripts can draw ([`Surface::add_picture`]).
    pictures: screen::Pictures,
}

impl Default for Surface {
    fn default() -> Surface {
        Surface::new()
    }
}

impl Surface {
    /// The shipped scripts only.
    pub fn new() -> Surface {
        Surface::with_user_dir(None)
    }

    /// The shipped scripts, with the files in `user` (if any) in place of theirs.
    pub fn with_user_dir(user: Option<PathBuf>) -> Surface {
        let mut s = Surface {
            sources: sources::Sources::new(user),
            theme: THEME.to_string(),
            run: None,
            error: RefCell::new(None),
            epoch: Instant::now(),
            checked: None,
            picture: None,
            frames: 0,
            pictures: Default::default(),
        };
        s.load();
        s
    }

    /// Draw with theme `name` (`themes/<name>.lua`); a theme that doesn't load leaves
    /// the last one.
    pub fn set_theme(&mut self, name: &str) {
        if name == self.theme {
            return;
        }
        let old = std::mem::replace(&mut self.theme, name.to_string());
        if !self.load() {
            self.theme = old;
        }
    }

    pub fn theme(&self) -> &str {
        &self.theme
    }

    /// The themes there are: DECKARD's, and the user's own.
    pub fn themes(&self) -> Vec<String> {
        self.sources.list("themes")
    }

    /// The last script error, if there is one.
    pub fn error(&self) -> Option<String> {
        self.error.borrow().clone()
    }

    /// The page shown.
    pub fn page(&self) -> String {
        self.run.as_ref().map_or(FIRST_PAGE.to_string(), |r| r.shared.borrow().page.clone())
    }

    fn set_error(&self, e: impl std::fmt::Display) {
        *self.error.borrow_mut() = Some(e.to_string().lines().next().unwrap_or_default().to_string());
    }

    /// Load the scripts; on an error the last working ones stay.
    fn load(&mut self) -> bool {
        let page = self.run.as_ref().map_or(FIRST_PAGE.to_string(), |r| r.shared.borrow().page.clone());
        match start(&self.sources, page, &self.theme) {
            Ok(run) => {
                self.run = Some(run);
                *self.error.borrow_mut() = None;
                self.picture = None;
                true
            }
            Err(e) => {
                self.set_error(e);
                false
            }
        }
    }

    /// Whether a button or pad is held (the pages may be timing it: `tick` wants
    /// calling often).
    pub fn holding(&self) -> bool {
        self.run.as_ref().is_some_and(|r| {
            let sh = r.shared.borrow();
            !sh.held.is_empty() || sh.pads.iter().any(|&p| p)
        })
    }

    /// Whether the theme moves by itself (a `chase` border): the screens want drawing
    /// often, even with nothing happening.
    pub fn animates(&self) -> bool {
        self.run.as_ref().is_some_and(|r| {
            r.lua.named_registry_value::<Table>("deckard.theme").and_then(|t| t.get::<Value>("chase")).is_ok_and(|c| !c.is_nil())
        })
    }

    /// The pages' timer: `tick()` for the page shown, then `global.lua` (as for events),
    /// as often as the app updates.
    pub fn tick(&mut self, v: &View, now: Instant) -> Vec<Action> {
        let r = self.with(v, Some(now), |run| {
            self.dispatch(run, "tick", ());
        });
        r.map(|(_, a)| a).unwrap_or_default()
    }

    /// Load the scripts again if the user's files changed (checked at most twice a
    /// second). Whether they were reloaded.
    pub fn poll(&mut self, now: Instant) -> bool {
        if self.checked.is_some_and(|t| now.duration_since(t) < RELOAD_EVERY) {
            return false;
        }
        self.checked = Some(now);
        self.reload()
    }

    /// Load the scripts again if the user's files changed.
    pub fn reload(&mut self) -> bool {
        self.sources.changed() && self.load()
    }

    /// Set what the scripts see, run `f`, and take the actions they asked for.
    fn with<R>(&self, v: &View, now: Option<Instant>, f: impl FnOnce(&Run) -> R) -> Option<(R, Vec<Action>)> {
        let run = self.run.as_ref()?;
        {
            let mut sh = run.shared.borrow_mut();
            sh.view = v.clone();
            if let Some(now) = now {
                sh.now = now.saturating_duration_since(self.epoch).as_secs_f64();
            }
        }
        let r = f(run);
        let actions = std::mem::take(&mut run.shared.borrow_mut().actions);
        Some((r, actions))
    }

    /// The shown page's script, then `global.lua`: each handler that exists gets the
    /// call until one returns `true`. Whether one did.
    fn dispatch(&self, run: &Run, handler: &str, args: impl mlua::IntoLuaMulti + Clone) -> bool {
        for t in self.tables(run) {
            let Ok(Some(f)) = t.get::<Option<Function>>(handler) else { continue };
            match f.call::<Value>(args.clone()) {
                Ok(Value::Boolean(true)) => return true,
                Ok(_) => {}
                Err(e) => {
                    self.set_error(e);
                    return true;
                }
            }
        }
        false
    }

    /// `global.lua`'s handler alone, if it has it: whether it returned `true`.
    fn call_global(&self, run: &Run, handler: &str, args: impl mlua::IntoLuaMulti) -> bool {
        let Ok(g) = run.lua.named_registry_value::<Table>("deckard.global") else { return false };
        let Ok(Some(f)) = g.get::<Option<Function>>(handler) else { return false };
        match f.call::<Value>(args) {
            Ok(v) => matches!(v, Value::Boolean(true)),
            Err(e) => {
                self.set_error(e);
                true
            }
        }
    }

    /// The shown page's table (if it loads) and the global one, in that order.
    fn tables(&self, run: &Run) -> Vec<Table> {
        let page = run.shared.borrow().page.clone();
        let mut out = Vec::new();
        match page_table(&run.lua, &page) {
            Ok(Some(t)) => out.push(t),
            Ok(None) => {}
            Err(e) => self.set_error(e),
        }
        if let Ok(g) = run.lua.named_registry_value::<Table>("deckard.global") {
            out.push(g);
        }
        out
    }

    /// The page shown and the pads' mode ("pad", "keyboard", "chords", "step"), kept
    /// with a saved project (BUGS #6).
    pub fn screen(&self) -> Option<(String, String)> {
        let run = self.run.as_ref()?;
        let require: Function = run.lua.globals().get("require").ok()?;
        let mode: String = require.call::<Table>("lib.pads").ok()?.get("mode").ok()?;
        Some((self.page(), mode))
    }

    /// Come back to a saved project's [`Surface::screen`]: `global.lua`'s `restore`.
    pub fn restore(&mut self, page: &str, mode: &str, v: &View, now: Instant) -> Vec<Action> {
        let r = self.with(v, Some(now), |run| {
            self.call_global(run, "restore", (page, mode));
        });
        r.map(|(_, a)| a).unwrap_or_default()
    }

    /// One MK3 event, at `now`: what the app should do.
    pub fn event(&mut self, ev: &Event, v: &View, now: Instant) -> Vec<Action> {
        let r = self.with(v, Some(now), |run| match *ev {
            Event::Button(hw, down) => {
                let name = names::button(hw);
                {
                    let mut sh = run.shared.borrow_mut();
                    if down {
                        sh.held.insert(name.clone());
                    } else {
                        sh.held.remove(&name);
                    }
                }
                self.dispatch(run, "button", (name, down));
            }
            Event::PadOn(pad, velocity) if (1..=16).contains(&pad) => {
                run.shared.borrow_mut().pads[pad - 1] = true;
                // (With SHIFT held, global.lua's `shift_pad` first: the shortcuts printed
                // on the pads, before any page, as Maschine's.)
                let shift = run.shared.borrow().held.contains("SHIFT");
                if !(shift && self.call_global(run, "shift_pad", (pad, velocity))) {
                    self.dispatch(run, "pad", (pad, velocity));
                }
            }
            Event::PadOff(pad) if (1..=16).contains(&pad) => {
                run.shared.borrow_mut().pads[pad - 1] = false;
                self.dispatch(run, "pad_off", pad);
            }
            Event::Knob(k, amount) if k < 8 => {
                self.dispatch(run, "knob", (k + 1, amount));
            }
            Event::Wheel(steps) => {
                if !self.call_global(run, "wheel_mode", steps) {
                    self.dispatch(run, "wheel", steps);
                }
            }
            // (Fingers from 1; "down", "move" or "up".)
            Event::Touch(finger, kind, pos) => {
                let kind = ["down", "move", "up"].get(kind as usize).copied().unwrap_or("move");
                self.dispatch(run, "touch", (finger + 1, kind, pos));
            }
            _ => {}
        });
        r.map(|(_, a)| a).unwrap_or_default()
    }

    /// Where the pads play now: the page's `pads()`, else the global one's; with
    /// neither, every pad plays its sound.
    pub fn pads(&self, v: &View) -> PadSetup {
        let all = PadSetup { play: true, keys: None, chords: None };
        let r = self.with(v, None, |run| {
            for t in self.tables(run) {
                let Ok(Some(f)) = t.get::<Option<Function>>("pads") else { continue };
                match f.call::<Option<Table>>(()) {
                    Ok(Some(p)) => return pad_setup(&p).unwrap_or(all),
                    Ok(None) => {}
                    Err(e) => {
                        self.set_error(e);
                        return all;
                    }
                }
            }
            all
        });
        let mut setup = r.map_or(all, |(p, _)| p);
        // (With SHIFT held the pads run the shortcuts printed under them, never playing:
        // nothing is heard or recorded.)
        if self.run.as_ref().is_some_and(|run| run.shared.borrow().held.contains("SHIFT")) {
            setup.play = false;
        }
        setup
    }

    /// Both LED reports: buttons at their faint backlight, the rest dark, then what
    /// `global.lua`'s `leds()` and the page's set.
    pub fn leds(&self, v: &View) -> Leds {
        let r = self.with(v, None, |run| {
            run.shared.borrow_mut().leds = leds::blank();
            for t in self.tables(run).into_iter().rev() {
                let Ok(Some(f)) = t.get::<Option<Function>>("leds") else { continue };
                if let Err(e) = f.call::<()>(()) {
                    self.set_error(e);
                }
            }
            run.shared.borrow().leds.clone()
        });
        r.map_or_else(leds::blank, |(l, _)| l)
    }

    /// Both screens, if they changed since the last call: a count that goes up with each
    /// new picture, and the picture (960 × 272 RGBA, the left screen first).
    pub fn screens(&mut self, v: &View) -> Option<(u64, Vec<[u8; 4]>)> {
        let px = self.draw(v);
        if self.picture.as_ref() == Some(&px) {
            return None;
        }
        self.picture = Some(px.clone());
        self.frames += 1;
        Some((self.frames, px))
    }

    /// Both screens as they are now, changed or not (960 × 272 RGBA).
    pub fn picture(&self, v: &View) -> Vec<[u8; 4]> {
        self.draw(v)
    }

    /// The text both screens show now, in the order drawn (for tests).
    #[cfg(test)]
    pub fn screen_text(&self, v: &View) -> Vec<String> {
        self.draw_picture(v).texts
    }

    /// `global.lua`'s `draw(screen, theme)`, then the page's; a script error over them.
    fn draw(&self, v: &View) -> Vec<[u8; 4]> {
        self.draw_picture(v).px
    }

    /// Give the scripts picture `key` to draw (`screen:image`): a plug-in's thumbnail
    /// (`thumb`) or its logo, RGBA, `w` × `h`.
    pub fn add_picture(&mut self, key: &str, thumb: bool, w: usize, h: usize, px: Vec<[u8; 4]>) {
        if px.len() == w * h {
            self.pictures.borrow_mut().insert((key.to_string(), thumb), screen::Image { w, h, px });
        }
    }

    fn draw_picture(&self, v: &View) -> screen::Picture {
        let r = self.with(v, None, |run| -> mlua::Result<screen::Picture> {
            run.lua.set_app_data(self.pictures.clone());
            let theme: Table = run.lua.named_registry_value("deckard.theme")?;
            let colors: Table = theme.get("colors")?;
            let ud = run.lua.create_userdata(screen::Picture::new(screen::colour(&run.lua, &colors.get::<Value>("background")?)?))?;
            for t in self.tables(run).into_iter().rev() {
                let Ok(Some(f)) = t.get::<Option<Function>>("draw") else { continue };
                if let Err(e) = f.call::<()>((ud.clone(), theme.clone())) {
                    self.set_error(e);
                }
            }
            ud.take::<screen::Picture>()
        });
        let mut p = match r {
            Some((Ok(p), _)) => p,
            Some((Err(e), _)) => {
                self.set_error(e);
                screen::Picture::new([0, 0, 0])
            }
            None => screen::Picture::new([0, 0, 0]),
        };
        if let Some(e) = self.error() {
            p.error(&e);
        }
        p
    }

    /// Run a Lua expression in the scripts' state (for tests).
    #[cfg(test)]
    fn eval<T: mlua::FromLua>(&self, code: &str) -> mlua::Result<T> {
        self.run.as_ref().expect("scripts loaded").lua.load(code).eval()
    }
}

/// A fresh Lua state with the API, `require` and the scripts loaded.
/// How deep themes may build on each other (`base = "…"`).
const THEME_BASES: usize = 4;

/// Theme `name`: what it leaves out of `colors`, `styles` (each style's fields too)
/// and the rest comes from its `base` theme, if it names one.
fn load_theme(lua: &Lua, sources: &sources::Sources, name: &str, depth: usize) -> mlua::Result<Table> {
    let theme: Table = sources::run(lua, sources, &format!("themes/{name}.lua"))?;
    let Some(base) = theme.get::<Option<String>>("base")? else { return Ok(theme) };
    if depth >= THEME_BASES {
        return Err(mlua::Error::runtime(format!("theme \"{name}\": its bases go too deep (a loop?)")));
    }
    let base = load_theme(lua, sources, &base, depth + 1)?;
    for pair in theme.pairs::<Value, Value>() {
        let (k, v) = pair?;
        let key = match &k {
            Value::String(s) => s.to_str()?.to_string(),
            _ => String::new(),
        };
        let merged = match (key.as_str(), &v, base.get::<Value>(k.clone())?) {
            ("colors", Value::Table(t), Value::Table(b)) => merge(lua, &b, t, false)?,
            ("styles", Value::Table(t), Value::Table(b)) => merge(lua, &b, t, true)?,
            _ => v,
        };
        base.set(k, merged)?;
    }
    Ok(base)
}

/// `over`'s entries over `base`'s (a copy); `deep`: tables in both merge too.
fn merge(lua: &Lua, base: &Table, over: &Table, deep: bool) -> mlua::Result<Value> {
    let out = lua.create_table()?;
    for pair in base.pairs::<Value, Value>() {
        let (k, v) = pair?;
        out.set(k, v)?;
    }
    for pair in over.pairs::<Value, Value>() {
        let (k, v) = pair?;
        let v = match (deep, &v, out.get::<Value>(k.clone())?) {
            (true, Value::Table(t), Value::Table(b)) => merge(lua, &b, t, false)?,
            _ => v,
        };
        out.set(k, v)?;
    }
    Ok(Value::Table(out))
}

fn start(sources: &sources::Sources, page: String, theme: &str) -> mlua::Result<Run> {
    // No file, process or OS access: the scripts only get the `deckard` API.
    let lua = Lua::new_with(StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH, LuaOptions::default())?;
    let shared = Rc::new(RefCell::new(Shared {
        view: View::default(),
        now: 0.0,
        held: HashSet::new(),
        pads: [false; 16],
        actions: Vec::new(),
        page,
        leds: leds::blank(),
    }));
    api::install(&lua, &shared)?;
    sources::install_require(&lua, sources.clone())?;
    let global: Table = sources::run(&lua, sources, "global.lua")?;
    lua.set_named_registry_value("deckard.global", global)?;
    let theme = load_theme(&lua, sources, theme, 0)?;
    lua.set_named_registry_value("deckard.theme", theme)?;
    lua.set_named_registry_value("deckard.pages", lua.create_table()?)?;
    // The first page loads now, so its errors show at once.
    let first = shared.borrow().page.clone();
    page_table(&lua, &first)?;
    Ok(Run { lua, shared })
}

/// Page `name`'s table (`pages/<name>.lua`), loaded once and the one
/// `require("pages.<name>")` gives the scripts; `None` if there's no such page.
fn page_table(lua: &Lua, name: &str) -> mlua::Result<Option<Table>> {
    let pages: Table = lua.named_registry_value("deckard.pages")?;
    if let Some(t) = pages.get::<Option<Table>>(name)? {
        return Ok(Some(t));
    }
    let sources: sources::Sources = lua.app_data_ref::<sources::Sources>().map(|s| s.clone()).unwrap_or_default();
    if sources.get(&format!("pages/{name}.lua")).is_none() {
        return Ok(None);
    }
    let require: mlua::Function = lua.globals().get("require")?;
    let t: Table = require.call(format!("pages.{name}"))?;
    pages.set(name, t.clone())?;
    Ok(Some(t))
}

/// `{ play = bool, keys = { sound = n, notes = {16 pitches} } }` as a [`PadSetup`].
fn pad_setup(t: &Table) -> mlua::Result<PadSetup> {
    let play = t.get::<Option<bool>>("play")?.unwrap_or(true);
    let keys = match t.get::<Option<Table>>("keys")? {
        Some(k) => {
            let sound = k.get::<usize>("sound")?.max(1) - 1;
            let notes: Table = k.get("notes")?;
            let mut out = [128i64; 16];
            for (i, n) in out.iter_mut().enumerate() {
                *n = notes.get::<Option<i64>>(i + 1)?.map_or(128, |n| n.clamp(0, 128));
            }
            Some((sound, out))
        }
        None => None,
    };
    let chords = match t.get::<Option<Table>>("keys")?.map(|k| k.get::<Option<Table>>("chords")).transpose()?.flatten() {
        Some(c) => {
            let mut out = [[128i64; 16]; 16];
            for (i, pad) in out.iter_mut().enumerate() {
                let Some(notes) = c.get::<Option<Table>>(i + 1)? else { continue };
                for (k, n) in pad.iter_mut().enumerate() {
                    *n = notes.get::<Option<i64>>(k + 1)?.map_or(128, |n| n.clamp(0, 128));
                }
            }
            Some(out)
        }
        None => None,
    };
    Ok(PadSetup { play, keys, chords })
}

#[cfg(test)]
mod chord_tests;
#[cfg(test)]
mod device_tests;
#[cfg(test)]
mod launchpad_tests;
#[cfg(test)]
mod sampling_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_support;
