//! Engine-side state the scripts read and change through the API.

use deckard_model::Project;
pub use deckard_model::sampling_view::{KnobPage, RecordPress, RecordState, SampleView, SamplingCall, SamplingKnob, StemsRun};
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct State {
    /// Controller model acronym the app reports (`M3`, `MS1`, `MM3`, ...).
    pub controller: String,
    /// The open project, shared with the app (edits made here are undoable there).
    pub doc: Option<deckard_doc::Shared>,
    pub focus: Focus,
    pub transport: Transport,
    pub hw: Hardware,
    pub widgets: Widgets,
    pub objects: Objects,
    /// Parameter values set by scripts that aren't stored in the project model.
    pub params: std::collections::HashMap<usize, Prop>,
    /// Parameters written since the last tick (`Parameter:isChanged()`).
    pub changed_params: std::collections::HashSet<usize>,
    /// `print` output from scripts.
    pub log: Vec<String>,
    /// A scene launched from the controller since the app last took it.
    pub launched_scene: Option<usize>,
    /// Likewise a section (SCENE + pad in the Song view).
    pub launched_section: Option<usize>,
    /// PAD MODE's SHIFT + BOUNCE: bounce these sounds (group, sound) in place.
    pub bounce_request: Option<(usize, usize)>,
    /// The audio interface as the app has it (the Settings page's Audio tab), and what
    /// the page changed.
    pub audio: AudioView,
    pub audio_edits: Vec<AudioEdit>,
    /// The app shows the Ideas view (scenes) rather than the Song view (sections);
    /// the SCENE button picks scenes or sections accordingly.
    pub ideas_view: bool,
    /// The Sampling page's tab, 0 RECORD … 3 ZONE.
    pub sampling_tab: i64,
    /// The focused sound's recorder, as the app has it (`api::recorder`).
    pub record: RecordState,
    /// START / STOP or CANCEL pressed on the RECORD screen, for the app to act on.
    pub record_press: Option<RecordPress>,
    /// The Sampling page's knobs, as the app has them: each tab's pages (RECORD, EDIT,
    /// SLICE, ZONE; up to four knobs each, knobs 5 and 6 zoom and scroll) and the one
    /// shown, the SLICE tab's own knobs (its script picks them), the settings turned for
    /// the app to apply (key, new value), and how far each knob has turned toward its
    /// next step.
    pub sampling_pages: [Vec<KnobPage>; 4],
    pub sampling_page: [usize; 4],
    pub slice_knobs: Vec<SamplingKnob>,
    pub sampling_edits: Vec<(String, f64)>,
    pub sampling_turns: std::collections::HashMap<String, f64>,
    /// The focused zone's sample as the app shows it (`api::sampling`), and what the
    /// Sampling page asked of it.
    pub sample: Option<SampleView>,
    pub sampling_calls: Vec<SamplingCall>,
    /// The browser as the app has it (BROWSE), and what the page asked of it.
    pub browser: BrowserView,
    pub browse_calls: Vec<BrowseCall>,
    /// Note Repeat / Arp (NOTE REPEAT): its settings as knobs (keys `arp/<field>`), the
    /// lists of its choices, whether it plays and is locked, and what the page changed.
    pub arp: ArpView,
    pub arp_edits: Vec<(String, f64)>,
    /// Each sound's mod wheel as the strip last left it, by (group, sound).
    pub strip_mod: std::collections::HashMap<(usize, usize), f64>,
    /// In NOTES mode, what each finger on the strip holds: (pad, group, sound, pitches,
    /// velocity).
    pub strummed: [Option<(usize, usize, usize, Vec<i64>, i64)>; 2],
    /// Auto-write (modulation) is on, as the app has it; and what AUTO asked for.
    pub auto_write: bool,
    pub auto_write_set: Option<bool>,
    /// The stem separation the app is running, if any (EDIT ▸ STEMS).
    pub stems: Option<StemsRun>,
    /// Audio plug-in takes' peaks (min, max) across their played frames, by (group,
    /// sound, take), for the pattern editor's loops (`pattern_view`).
    pub loop_peaks: std::collections::HashMap<(usize, usize, usize), std::sync::Arc<Vec<(f32, f32)>>>,
    /// The scale engine's settings (Keyboard and Chords modes).
    pub scale: ScaleState,
    /// Chords mode's Auto position: the state chords lead from, and what each pad's hit
    /// would make of it (`Host::keyboard_chords`, moved on by `Host::pad_event`).
    pub chord_auto: (deckard_model::chords::Auto, [Option<deckard_model::chords::Auto>; 16]),
    /// Notes sounding now, for the pads' and group buttons' flashing.
    pub notes_on: NotesOn,
    /// Per group, (bars, how far through them in bars) in its pattern in the focused
    /// scene, as the app plays it (`set_group_progress`).
    pub group_progress: std::collections::HashMap<usize, (usize, f64)>,
    /// What the app's Control area shows, as the hardware's Control page reads it.
    pub control: Control,
    /// The Control area's level at the last controller timer (`isMixingLayerChanged`).
    pub level_seen: Option<usize>,
    /// What the scripts asked the Control area to do since the app last took them.
    pub control_requests: Vec<ControlRequest>,
    /// The view and focused scene and section at the last controller timer (the
    /// arranger state's `isViewChanged`/`isFocusChanged`).
    pub arranger_seen: Option<(bool, Option<usize>, Option<usize>)>,
    /// The focus at the last controller timer (`StateCache:isFocus…Changed`).
    pub focus_seen: Option<Focus>,
    /// The groups' and sounds' colours as the scripts last saw them.
    pub colors_seen: Vec<(ObjRef, deckard_model::Color)>,
    /// The chain (focused slot, slots) at the last controller timer: what changed since.
    pub chain_seen: Option<(Option<usize>, Vec<ControlSlot>)>,
    /// Selected notes (the controller's Step and Events pages, shared with the app's
    /// pattern editor): the pattern (group, pattern) they belong to and the notes.
    pub selection: Option<(usize, usize, Vec<deckard_model::Note>)>,
    /// The step grid in ticks (the pattern editor's Grid; a 16th until the app says).
    pub step_grid: i64,
    /// The Nudge Grid in ticks: what nudging moves notes by (half a step until the app
    /// says): the step grid over 2^`nudge_div`.
    pub nudge_grid: i64,
    /// The Nudge Grid's setting: 0 Step, 1 Step/2, … 4 Step/16.
    pub nudge_div: usize,
    /// The Grid page changed the step or nudge grid (the app takes them over).
    pub grid_changed: bool,
    /// LOCK is lit (the app's).
    pub lock_active: bool,
    /// Notes copied on the controller (SHIFT + pad 11), positions from their pattern's
    /// start.
    pub note_clipboard: Vec<deckard_model::Note>,
    /// The controller's pattern editor view (`api::pattern_editor`).
    pub editor: EditorView,
    /// The Song view's arranger on the displays: zoom and scroll.
    pub arranger: ArrangerView,
    /// When TAP was last pressed, for tap tempo (`api::hw::tap_tempo`).
    pub taps: Vec<std::time::Instant>,
    /// The plug-ins the app can load, for the controller's plug-in browser (set by the
    /// app; built-in ones first).
    pub plugins: Vec<PluginEntry>,
    /// The plug-in list changed since the scripts last ticked.
    pub plugins_changed: bool,
    /// The meters the Mixer page shows ([`crate::Host::set_levels`]).
    pub levels: Levels,
    /// Link and MIDI sync as the app has them, changed by the Settings ▸ MIDI page
    /// ([`crate::Host::set_sync`]).
    pub sync: SyncView,
}

/// Link and MIDI sync (Settings ▸ MIDI), in the scripts' terms.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncView {
    /// Link joined, Start Stop Sync, the peers, and the session's phase in the bar
    /// (0..1).
    pub link: bool,
    pub link_start_stop: bool,
    pub peers: u64,
    pub phase: f64,
    /// `NSA:getMidiSyncStatus()`: 0 Off, 1 Send Clock, 2 Receive Clock.
    pub midi_sync: i64,
    /// Clock Offset, sending: 0..=200 (shown as 0 … -200 ms).
    pub send_offset: i64,
    /// Clock Offset, receiving: 0..=200 (shown as -100 … +100 ms).
    pub receive_offset: i64,
    /// The MIDI ports, and whether each is switched on.
    pub inputs: Vec<(String, bool)>,
    pub outputs: Vec<(String, bool)>,
    /// What a sound whose MIDI input is Default takes: the focus (else nothing).
    pub default_focus: bool,
}

/// How the controller's pattern editor is zoomed and scrolled, and for which pattern.
#[derive(Debug, Default, Clone)]
pub struct EditorView {
    /// (group, pattern) the zoom and offset were set for.
    pub pattern: Option<(usize, usize)>,
    /// Magnification of the unzoomed view; 0 until the scripts zoom.
    pub zoom: f64,
    /// Ticks from the pattern's start.
    pub offset: i64,
    /// Sound lanes doubled in height.
    pub tall_lanes: bool,
    /// The step grid switched off (GRID): edits move by the smallest step.
    pub grid_off: bool,
    /// The piano roll's scroll (knob 8) for sound (group, sound): its lowest row in
    /// hundredths of a semitone, once turned (until then it centres on the notes).
    pub pitch_offset: Option<(usize, usize, f64)>,
}

/// How the controller's arranger (Song view) is zoomed and scrolled.
#[derive(Debug, Default, Clone, Copy)]
pub struct ArrangerView {
    /// Magnification of the unzoomed view; 0 until the scripts zoom (the whole song).
    pub zoom: f64,
    /// Ticks from the song's start.
    pub offset: i64,
    /// The focused section the view last scrolled to show.
    pub shown_section: Option<usize>,
}

/// What sounds now (`StateCache::isNoteOn`): the notes the sequencer plays, as the app
/// sets them, and what each held pad plays. A pad tapped from the screen stays lit for
/// [`NotesOn::AFTER_RELEASE`], so the tap shows.
#[derive(Debug, Default)]
pub struct NotesOn {
    /// (group, sound, pitch)
    pub played: std::collections::HashSet<(usize, usize, i64)>,
    /// Per pad: (group, sound, pitch) and when it was released.
    pub pads: [Option<((usize, usize, i64), Option<std::time::Instant>)>; 16],
}

impl NotesOn {
    pub const AFTER_RELEASE: std::time::Duration = std::time::Duration::from_millis(100);

    /// Every note sounding: those played, then the held pads' (with their pad, 0-based).
    pub fn all(&self) -> impl Iterator<Item = ((usize, usize, i64), Option<usize>)> + '_ {
        let held = self.pads.iter().enumerate().filter_map(|(i, p)| {
            let (note, released) = (*p)?;
            released.is_none_or(|t| t.elapsed() < Self::AFTER_RELEASE).then_some((note, Some(i)))
        });
        self.played.iter().map(|&n| (n, None)).chain(held)
    }
}

/// The scale engine's settings, as `ScaleEngine`'s parameters hold them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScaleState {
    /// The bank (0 Main … 7 Minor) and the scale in it (0..15).
    pub bank: i64,
    pub scale: i64,
    /// 0 Off, 1 Harmonizer, 2 Chord Set, 3 User Chord (`ChordMode`).
    pub chord_mode: i64,
    /// The Chords page's mode: 0 Harmonizer, 1 Chord Set, 2 User Chord.
    pub screen_mode: i64,
    /// The chord type kept for each kind of list: the Harmonizer's for the chromatic
    /// scale and for other scales, the chord set and the user chord set.
    pub chord_chromatic: i64,
    pub chord_scale: i64,
    pub chord_set: i64,
    pub user_set: i64,
    /// 0 Auto, 1..=8 -8..-1, 9 Root, 10..=17 +1..+8 (`ChordPosition`).
    pub position: i64,
}

impl Default for ScaleState {
    fn default() -> Self {
        // As a new project starts in Maschine: Main/Chromatic, Harmonizer 1-3-5.
        Self {
            bank: 0,
            scale: 0,
            chord_mode: 0,
            screen_mode: 0,
            chord_chromatic: 3,
            chord_scale: 3,
            chord_set: 0,
            user_set: 0,
            position: 9,
        }
    }
}

/// The app's Control area, published each frame: the focused chain's slots, the
/// focused slot's pages and the shown page's eight knobs.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Control {
    /// 0 master, 1 group, 2 sound (`NI.DATA.LEVEL_TAB_SONG/GROUP/SOUND`).
    pub level: usize,
    /// Plug-ins shown (else the channel's properties: `Workspace` ModulesVisible).
    pub modules_visible: bool,
    /// The channel properties' page group (Input, Output, Groove, Macro:
    /// [`MACRO_PAGE_GROUP`]), and how many there are.
    pub page_group: usize,
    pub page_groups: usize,
    pub slots: Vec<ControlSlot>,
    pub focus_slot: Option<usize>,
    pub pages: Vec<String>,
    pub page: usize,
    /// Eight positions; `None` where the page has no knob.
    pub knobs: Vec<Option<ControlKnob>>,
}

/// The channel properties' Macro set (the last of Input, Output, Groove, Macro).
pub const MACRO_PAGE_GROUP: usize = 3;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ControlSlot {
    pub name: String,
    pub plugin: bool,
    pub active: bool,
    /// An instrument (else an effect).
    pub instrument: bool,
}

/// A plug-in the app can load into a slot (the controller's plug-in browser lists them).
#[derive(Debug, Clone, PartialEq)]
pub struct PluginEntry {
    pub name: String,
    pub instrument: bool,
    pub format: PluginFormat,
    /// The maker ("Native Instruments" counts as Maschine's own vendor, `NI`).
    pub vendor: String,
}

/// Where a plug-in comes from: built in, or a format of external plug-ins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginFormat {
    Internal,
    AudioUnit,
    Vst3,
    Vst2,
}

impl PluginFormat {
    /// The browser's name for it (`ModuleHelper.INTERNAL`, `.AU`, `.VST3`, `.VST2`).
    pub fn label(self) -> &'static str {
        match self {
            PluginFormat::Internal => "Internal",
            PluginFormat::AudioUnit => "Audio Units",
            PluginFormat::Vst3 => "VST3",
            PluginFormat::Vst2 => "VST",
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ControlKnob {
    pub name: String,
    pub section: String,
    /// The knob's position, 0..1.
    pub value: f64,
    /// Its value as shown ("12.0 %", "1.20 kHz").
    pub text: String,
    /// The parameter it turns (its object id in the project), if a macro can control it.
    pub param: Option<u64>,
    /// It is modulated (the knob shows a modulation mark).
    pub modulated: bool,
}

/// What LOCK and the Snapshots page ask the app for (docs/superpowers/plans/
/// 2026-10-22-phase6-lock.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockOp {
    /// LOCK: take (or update) the focused snapshot, or recall it.
    Button,
    /// Pad `.0` (0–15) of the focused bank: recall its snapshot, or take one there.
    Pad(usize),
    /// UPDATE: take the focused one again.
    Update,
    /// DELETE: the focused one.
    Delete,
    /// Delete the one in slot `.0`.
    DeleteSlot(usize),
    /// The previous (false) or next bank (past the last: a new one).
    Bank(bool),
}

/// The audio interface, for the Settings page's Audio tab (`App:getNSA()`).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct AudioView {
    /// The audio system (Core Audio, WASAPI, ALSA…): DECKARD has one.
    pub driver: String,
    pub devices: Vec<String>,
    pub device: String,
    pub rate: u32,
    /// The buffer sizes offered, and the one in use (frames).
    pub buffers: Vec<u32>,
    pub buffer: u32,
    pub running: bool,
    /// The interface's channels (names), and which one each of DECKARD's 32 inputs and
    /// 32 outputs (In 1 L … In 16 R) is on (`None`: not connected).
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub input_map: Vec<Option<usize>>,
    pub output_map: Vec<Option<usize>>,
}

/// A change the Audio tab asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioEdit {
    Device(String),
    Buffer(u32),
    /// DECKARD's input (`true`) or output channel `.1` to interface channel `.2`
    /// (`None`: not connected).
    Map(bool, usize, Option<usize>),
}

/// A change to the Control area the scripts asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlRequest {
    /// Turn knob `pos` (0..8) by `inc` (a fraction of a turn); `fine` with SHIFT.
    Turn { pos: usize, inc: f64, fine: bool },
    /// Knob `pos` turned with ERASE held: remove its modulation.
    Erase { pos: usize },
    /// Show the focused sound's Groove page `.0` (0 Swing, 1 Variation, 2 Generate).
    GroovePage(usize),
    /// Show group `.0`'s Perform FX.
    ShowPerformFx(usize),
    /// Lock (LOCK, the Snapshots page).
    Lock(LockOp),
    /// The strip in PERFORM mode on group `group`'s Perform FX: touched (`Some(true)`),
    /// let go (`Some(false)`) or moved, at `pos` (0..1).
    PerformStrip { group: usize, touch: Option<bool>, pos: f64 },
    /// The strip in PITCH (`pitch`: a bend, -1..1) or MOD (the mod wheel, 0..1) mode on
    /// sound `sound` of group `group`.
    StripControl { group: usize, sound: usize, pitch: bool, value: f64 },
    /// The strip in NOTES mode: note `pitch` of sound `sound` of group `group` started
    /// (`on`, at `velocity`) or ended.
    StripNote { group: usize, sound: usize, pitch: i64, velocity: i64, on: bool },
    /// Show the previous (false) or next page.
    Page(bool),
    /// Focus the slot `delta` places on.
    Slot(i64),
    /// Show the master (0), group (1) or sound (2) chain.
    Level(usize),
    /// Switch slot `.0` on or off.
    Active(usize, bool),
    /// Remove slot `.0`.
    Remove(usize),
    /// Move slot `.0` to position `.1`.
    Move(usize, usize),
    /// Show plug-ins (true) or the channel's properties.
    ModulesVisible(bool),
    /// Show channel-property page group `.0`.
    PageGroup(usize),
    /// Set Macro (SHIFT + MACRO): make knob `pos`'s parameter a macro of the master (level
    /// 0), the focused group (1) or sound (2), or stop it being one.
    SetMacro { pos: usize, level: usize },
    /// Load plug-in `entry` (of `State::plugins`): into slot `replace` in its place, or
    /// with `None` after the focused slot (at the end when the "+" is focused).
    Load { entry: usize, replace: Option<usize> },
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Focus {
    pub group: usize,
    pub sound: usize,
    pub pattern: Option<usize>,
    pub scene: Option<usize>,
    pub section: Option<usize>,
}

/// What the app plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Playing {
    /// Pattern (group, index) on its own.
    Pattern(usize, usize),
    /// A scene of the Ideas view.
    Scene(usize),
    /// The arrangement (the Song view's sections).
    Arrangement,
}

/// Note Repeat / Arp as the NOTE REPEAT page shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArpView {
    pub knobs: Vec<SamplingKnob>,
    /// A choice's names, by knob key (`getAsString(i)`).
    pub lists: Vec<(String, Vec<String>)>,
    pub active: bool,
    pub locked: bool,
}

/// The browser as BROWSE shows it (the app's LIBRARY tab): the section (an index in
/// `deckard_library::Kind::ALL`), Factory or User, the products and the one chosen,
/// the three attributes (Type, Sub-type, Character: the values and the one chosen),
/// the results (name, favourite) and the one in focus.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserView {
    pub kind: usize,
    pub user: bool,
    pub products: Vec<String>,
    pub product: Option<usize>,
    pub attributes: [(Vec<String>, Option<usize>); 3],
    pub results: Vec<(String, bool)>,
    pub focus: usize,
    /// A query is running.
    pub busy: bool,
    pub favorites: bool,
}

/// What BROWSE asks of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum BrowseCall {
    Kind(usize),
    User(bool),
    Product(Option<usize>),
    /// Attribute `.0` (Type, Sub-type, Character) set to value `.1` (`None`: all).
    Attribute(usize, Option<usize>),
    /// Move the result focus by this much.
    Focus(i64),
    Load,
    ToggleFavorites,
    /// Prehear the focused result.
    Prehear,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Transport {
    pub playing: bool,
    /// Ticks, into what plays (`what`).
    pub position: i64,
    /// What plays, or played when STOP left it (`Host::set_playhead`).
    pub what: Option<Playing>,
    pub recording: bool,
    /// The recording asked for counts in first (SHIFT + REC).
    pub count_in: bool,
    pub looping: bool,
    pub metronome: bool,
    /// RESTART was pressed; cleared by whoever plays it (the app).
    pub restarted: bool,
}

/// What the scripts have told the hardware to show.
#[derive(Debug, Default)]
pub struct Hardware {
    pub leds: BTreeMap<i64, Led>,
    /// Bottom first.
    pub page_stack: Vec<i64>,
    /// Set when the stack changes; cleared after each controller timer tick.
    pub page_stack_changed: bool,
    /// Root widgets of the hardware display, by accessor (`OverlayRoot`, ...).
    pub display_roots: BTreeMap<String, usize>,
    pub pinned: BTreeMap<i64, bool>,
    pub pad_mode: i64,
    pub jog_wheel_mode: i64,
    pub encoder_mode: i64,
    /// Buttons held down now (`NI.HW.BUTTON_*` values).
    pub held: std::collections::BTreeSet<i64>,
    /// `NI.HW.SCREENBUTTON_MODE_*`: in SCENES (the Ideas view) the buttons above the
    /// displays pick scenes themselves.
    pub screen_button_mode: i64,
    /// Which eight scenes those buttons pick (`getSceneBank8`).
    pub scene_bank8: i64,
}

/// One LED as the scripts last set it.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Led {
    /// Brightness 0..1 (`setLEDLevel`).
    pub level: f64,
    /// Palette colour index and LED state (`setLEDColorLUT`), on controllers with a
    /// colour lookup table.
    pub lut: Option<(i64, i64)>,
    /// RGB, 0..127 per channel (`setLEDColor`).
    pub rgb: Option<(u8, u8, u8)>,
}

impl Led {
    /// LUT state 1 is `LEDHelper.LS_OFF` (the scripts' states are 1-based).
    pub fn is_lit(&self) -> bool {
        self.level > 0.0 || self.lut.is_some_and(|(_, state)| state > 1)
    }
}

impl State {
    /// Share the step grid (ticks) and the Nudge Grid (its setting) with the app: what
    /// the Grid page changed since the last call goes to the app, else the app's come
    /// here.
    pub fn sync_grid(&mut self, grid: &mut i64, nudge_div: &mut usize) {
        if std::mem::take(&mut self.grid_changed) {
            *grid = self.step_grid;
            *nudge_div = self.nudge_div;
        } else {
            self.step_grid = (*grid).max(1);
            self.nudge_div = (*nudge_div).min(4);
        }
        self.nudge_grid = (self.step_grid >> self.nudge_div).max(1);
    }

    pub fn new(controller: &str, doc: Option<deckard_doc::Shared>) -> State {
        // Like the app after loading: first group, sound and scene in focus, and the
        // group's pattern in that scene (else its first).
        let focus = {
            let d = doc.as_ref().map(|d| d.borrow());
            let project = d.as_ref().map(|d| d.project());
            let in_scene = project.and_then(|p| p.scenes.first()?.patterns.iter().find(|(g, _)| *g == 0).map(|&(_, pi)| pi));
            Focus {
                pattern: in_scene.or_else(|| project.and_then(|p| p.groups.first()).filter(|g| !g.patterns.is_empty()).map(|_| 0)),
                scene: project.filter(|p| !p.scenes.is_empty()).map(|_| 0),
                ..Default::default()
            }
        };
        // Until the app says otherwise: the group's chain, plug-ins shown.
        let control = Control { level: 1, modules_visible: true, page_groups: 4, knobs: vec![None; 8], ..Default::default() };
        State {
            controller: controller.to_string(),
            doc,
            focus,
            ideas_view: true,
            control,
            step_grid: deckard_model::PPQ / 4,
            nudge_grid: deckard_model::PPQ / 8,
            nudge_div: 1,
            grid_changed: false,
            lock_active: false,
            ..Default::default()
        }
    }

    /// The project as it is now.
    pub fn project(&self) -> Option<std::cell::Ref<'_, Project>> {
        Some(std::cell::Ref::map(self.doc.as_ref()?.borrow(), |d| d.project()))
    }
}

/// A script-set widget property.
#[derive(Debug, Clone, PartialEq)]
pub enum Prop {
    Nil,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(String),
    Widget(usize),
    /// Tables, functions and engine objects, kept as a description only.
    Other(String),
}

impl Prop {
    pub fn from_lua(v: &mlua::Value) -> Prop {
        match v {
            mlua::Value::Nil => Prop::Nil,
            mlua::Value::Boolean(b) => Prop::Bool(*b),
            mlua::Value::Integer(i) => Prop::Int(*i),
            mlua::Value::Number(n) => Prop::Num(*n),
            mlua::Value::String(s) => Prop::Str(s.to_string_lossy()),
            other => match crate::api::object_id(other) {
                Some(id) if crate::api::object_type(other).as_deref() == Some("Widget") => Prop::Widget(id),
                _ => Prop::Other(other.to_string().unwrap_or_else(|_| other.type_name().to_string())),
            },
        }
    }

    pub fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        Ok(match self {
            Prop::Nil => mlua::Value::Nil,
            Prop::Bool(b) => mlua::Value::Boolean(b),
            Prop::Int(i) => mlua::Value::Integer(i),
            Prop::Num(n) => mlua::Value::Number(n),
            Prop::Str(s) => mlua::Value::String(lua.create_string(s)?),
            Prop::Widget(id) => crate::api::new_object(lua, "Widget", id)?,
            Prop::Other(_) => mlua::Value::Nil,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct Widget {
    pub kind: String,
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub props: BTreeMap<String, Prop>,
}

/// Every widget the scripts created, by id.
#[derive(Debug, Default)]
pub struct Widgets {
    pub all: Vec<Widget>,
}

impl Widgets {
    pub fn insert(&mut self, parent: Option<usize>, kind: &str, name: &str) -> usize {
        let id = self.all.len();
        let parent = parent.filter(|p| *p < id);
        self.all.push(Widget { kind: kind.into(), name: name.into(), parent, ..Default::default() });
        if let Some(p) = parent {
            self.all[p].children.push(id);
        }
        id
    }

    pub fn get(&self, id: usize) -> Option<&Widget> {
        self.all.get(id)
    }

    /// Move `id` under `parent` (detaching it from its old parent).
    pub fn reparent(&mut self, id: usize, parent: usize, name: &str) {
        if id >= self.all.len() || parent >= self.all.len() || id == parent {
            return;
        }
        if let Some(old) = self.all[id].parent.take() {
            self.all[old].children.retain(|c| *c != id);
        }
        self.all[id].parent = Some(parent);
        if !name.is_empty() {
            self.all[id].name = name.to_string();
        }
        self.all[parent].children.push(id);
    }

    pub fn set(&mut self, id: usize, prop: &str, v: Prop) {
        if let Some(w) = self.all.get_mut(id) {
            w.props.insert(prop.to_string(), v);
        }
    }

    pub fn roots(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.all.len()).filter(|i| self.all[*i].parent.is_none())
    }

    /// Indented outline of the visible widgets under `root` that show text, for
    /// debugging what the scripts put on a display.
    pub fn outline(&self, root: usize) -> String {
        let mut out = String::new();
        self.outline_into(root, 0, &mut out);
        out
    }

    fn outline_into(&self, id: usize, depth: usize, out: &mut String) {
        let w = &self.all[id];
        if matches!(w.props.get("Visible"), Some(Prop::Bool(false))) {
            return;
        }
        let text = match (w.props.get("Text"), w.props.get("style")) {
            (Some(Prop::Str(t)), _) if !t.is_empty() => t.clone(),
            (_, Some(Prop::Str(t))) if !t.is_empty() && w.kind.contains("Label") => t.clone(),
            _ => String::new(),
        };
        let start = out.len();
        out.push_str(&format!("{}{} {}", "  ".repeat(depth), w.kind, w.name));
        if !text.is_empty() {
            out.push_str(&format!(" = {text:?}"));
        }
        out.push('\n');
        let before = out.len();
        for &c in &w.children {
            self.outline_into(c, depth + 1, out);
        }
        // Drop text-less leaves to keep the outline readable.
        if text.is_empty() && out.len() == before {
            out.truncate(start);
        }
    }
}

/// An engine object the scripts hold a handle to. Handles are interned so the same
/// object always gets the same id (and the same Lua table, so `==` works).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ObjRef {
    Song,
    Group(usize),
    Sound(usize, usize),
    Pattern(usize, usize),
    Scene(usize),
    Section(usize),
    Workspace,
    Metronome,
    Recorder,
    /// `App:getSamplingParameters()`, a tab's pages (`getPagesForTab(tab)`) and page
    /// `.1` of tab `.0`.
    SamplingParameters,
    KnobPages(usize),
    KnobPage(usize, usize),
    /// The focused sample (`getFocusSample`), its transaction sample, its slices, a
    /// Sampler's zone list and the focused zone, and the wave display's view
    /// (`SampleOwnerViewHelper.makeFocusView`).
    Sample,
    TransactionSample,
    Slices,
    Zones,
    Zone,
    WaveView,
    /// The sample a stem separation runs on, when it isn't the focused one.
    StemSource,
    // The browser (BROWSE): `App:getDatabaseFrontend()` and what hangs off it.
    DatabaseFrontend,
    BrowserController,
    BrowserModel,
    ProductModel,
    BankChainModel,
    AttributesModel,
    ResultListModel,
    DeferredSearch,
    /// `NI.DATA.getArpeggiator(App)`.
    Arpeggiator,
    PageStack,
    HardwareDisplay,
    StateCache,
    ObjectCache,
    List(ListKind),
    Param {
        owner: usize,
        name: String,
    },
    /// Slot `.0` of the Control area's chain, its module and the module's info.
    Slot(usize),
    Module(usize),
    ModuleInfo(usize),
    /// The focused chain (a mixing layer's `getChain()`).
    Chain,
    /// Entry `.0` of the plug-ins the app can load (`State::plugins`), as a module's info
    /// or a plug-in descriptor; `usize::MAX` is no module (`ID_NONE`).
    PluginEntry(usize),
    /// The knobs of the Control area's page (owner of their `Param`s, named "0".."7").
    ControlKnobs,
    /// Scales and chords (`NI.DATA.getScaleEngine`).
    ScaleEngine,
    /// `NHLController:getContext()`.
    Context,
    /// `Song:getArrangerState()`.
    ArrangerState,
    /// A sound's notes in a pattern (group, pattern, sound): `getSoundSequence`.
    Sequence(usize, usize, usize),
    /// Group `.0`'s Perform FX (`Group:getPerformanceFX()`).
    PerformFx(usize),
    /// The song's Lock snapshots (`Song:getParameterSnapshotsManager()`).
    LockManager,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListKind {
    Groups,
    Sounds(usize),
    Patterns(usize),
    Scenes,
    Sections,
    /// The Control area's chain slots.
    Slots,
}

impl ObjRef {
    /// Proxy type name, which selects the method table.
    pub fn type_name(&self) -> &'static str {
        match self {
            ObjRef::Song => "Song",
            ObjRef::Group(_) => "Group",
            ObjRef::Sound(..) => "Sound",
            ObjRef::Pattern(..) => "EventPattern",
            ObjRef::Scene(_) => "Scene",
            ObjRef::Section(_) => "Section",
            ObjRef::Workspace => "Workspace",
            ObjRef::Metronome => "Metronome",
            ObjRef::Recorder => "Recorder",
            ObjRef::SamplingParameters => "SamplingParameters",
            ObjRef::KnobPages(_) => "SamplingParameterPages",
            ObjRef::KnobPage(..) => "SamplingParameterPage",
            ObjRef::Sample => "Sample",
            ObjRef::TransactionSample => "TransactionSample",
            ObjRef::Slices => "SliceContainer",
            ObjRef::Zones => "ZoneVector",
            ObjRef::Zone => "Zone",
            ObjRef::WaveView => "SampleOwnerView",
            ObjRef::StemSource => "TransactionSample",
            ObjRef::DatabaseFrontend => "DatabaseFrontend",
            ObjRef::BrowserController => "BrowserController",
            ObjRef::BrowserModel => "BrowserModel",
            ObjRef::ProductModel => "ProductModel",
            ObjRef::BankChainModel => "BankChainModel",
            ObjRef::AttributesModel => "AttributesModel",
            ObjRef::ResultListModel => "ResultListModel",
            ObjRef::DeferredSearch => "DeferredSearch",
            ObjRef::Arpeggiator => "Arpeggiator",
            ObjRef::PageStack => "PageStack",
            ObjRef::HardwareDisplay => "HardwareDisplay",
            ObjRef::StateCache => "StateCache",
            ObjRef::ObjectCache => "ObjectCache",
            ObjRef::List(_) => "List",
            ObjRef::Param { .. } => "Parameter",
            ObjRef::Slot(_) => "Slot",
            ObjRef::Module(_) => "Module",
            ObjRef::ModuleInfo(_) | ObjRef::PluginEntry(_) => "ModuleInfo",
            ObjRef::Chain => "Chain",
            ObjRef::ControlKnobs => "ControlKnobs",
            ObjRef::ScaleEngine => "ScaleEngine",
            ObjRef::Context => "Context",
            ObjRef::ArrangerState => "ArrangerState",
            ObjRef::Sequence(..) => "Sequence",
            ObjRef::PerformFx(_) => "PerformanceFX",
            ObjRef::LockManager => "ParameterSnapshotsManager",
        }
    }
}

#[derive(Debug, Default)]
pub struct Objects {
    refs: Vec<ObjRef>,
    ids: std::collections::HashMap<ObjRef, usize>,
}

impl Objects {
    pub fn intern(&mut self, r: ObjRef) -> usize {
        if let Some(&id) = self.ids.get(&r) {
            return id;
        }
        let id = self.refs.len();
        self.refs.push(r.clone());
        self.ids.insert(r, id);
        id
    }

    pub fn get(&self, id: usize) -> Option<&ObjRef> {
        self.refs.get(id)
    }
}

/// What the app meters: each sound's level (left, right; amplitude), the master's, and
/// each channel's headroom (its loudest since reset) by (group, sound): (Some, None)
/// for a group, (None, None) for the master.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Levels {
    pub sounds: Vec<Vec<(f32, f32)>>,
    pub master: (f32, f32),
    pub headroom: std::collections::HashMap<(Option<usize>, Option<usize>), f32>,
}
