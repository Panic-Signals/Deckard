//! Typed view of a Maschine project: songs → groups → sounds, patterns with notes,
//! scenes and sections.
//!
//! Built from the generic tree that `mxprj` decodes. Cross-links in the archive are
//! object-id back-references; [`Index`] resolves them. Missing or unexpected data
//! becomes a default plus an entry in [`Project::warnings`] rather than an error, so a
//! partly understood file still loads.

pub mod chords;
pub mod expression;
pub mod lfo;
pub mod mod_matrix;
pub mod pad_touch;
pub mod sampling_view;
pub mod scale_tables;
pub mod scales;

pub use expression::{Curve, Dim, ExprId, ExprKind, Expression, Point};
use mxprj::decode::{Obj, Path, Value};
use std::collections::{BTreeMap, HashMap};

/// Ticks per quarter note.
pub const PPQ: i64 = 960;

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub name: String,
    pub tempo: f64,
    pub numerator: i64,
    pub denominator: i64,
    pub groups: Vec<Group>,
    pub scenes: Vec<Scene>,
    pub sections: Vec<Section>,
    /// The master groove.
    pub swing: Swing,
    /// The master channel: level (linear) and pan.
    pub master_level: f64,
    pub master_pan: f64,
    /// The master's plug-in chain (its effects).
    pub master_slots: Vec<SlotInfo>,
    /// Note Repeat / Arp (one for the project).
    pub arp: Arp,
    /// Note Repeat's four presets (the NOTE REPEAT page's buttons 5–8): (rate, unit),
    /// rate an index of 1 Bar, 1/2 … 1/128 and unit 0 Normal, 1 Triplet, 2 Dotted.
    pub arp_presets: Vec<(i64, i64)>,
    /// The Mixer shows the focused group's sounds (else the groups).
    pub mixer_sounds: bool,
    /// Where the master and the cue bus go, and the cue bus's level and pan.
    pub master_output: AudioDest,
    pub cue_output: AudioDest,
    pub cue_level: f64,
    pub cue_pan: f64,
    /// Lock: the song's snapshots and their settings.
    pub lock: Lock,
    /// The project's LFOs (DECKARD's; [`lfo`]), by pad.
    pub lfos: Vec<lfo::Lfo>,
    /// The project's routes from LFOs to MIDI CCs (DECKARD's; [`mod_matrix::CcRoute`]).
    pub cc_routes: Vec<mod_matrix::CcRoute>,
    pub warnings: Vec<String>,
    /// Per-note expression, by [`ExprId`] (a note's `expr`): the document's table, for
    /// the engine. Empty when read from a file (the document fills it).
    pub expressions: Vec<std::sync::Arc<Expression>>,
}

/// Lock (docs/superpowers/plans/2026-10-22-phase6-lock.md): which of the 64 slots (bank
/// × 16 + pad) hold a snapshot, the focused one, and the morph settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Lock {
    pub slots: Vec<usize>,
    pub focus: Option<usize>,
    pub morph: bool,
    /// 0 Travel, 1 Target.
    pub mode: usize,
    /// Travel's length: 1/2 Bar, 1 Bar, 2, 4, 8, 16 Bars.
    pub time: usize,
    /// Target's: 1/8, 1/4, 1/2, Bar.
    pub grid: usize,
    pub focus_bank: usize,
    /// 1–4.
    pub banks: usize,
    /// MIDI Lock Change (Edit ▸ MIDI Change): 0 None, 1 MIDI notes, 2 program changes;
    /// from which input and channel (1–16).
    pub midi_trigger: usize,
    pub midi_source: String,
    pub midi_channel: i64,
}

impl Default for Lock {
    fn default() -> Lock {
        Lock {
            slots: Vec::new(),
            focus: None,
            morph: false,
            mode: 0,
            time: 1,
            grid: 3,
            focus_bank: 0,
            banks: 1,
            midi_trigger: 0,
            midi_source: "None".into(),
            midi_channel: 1,
        }
    }
}

/// Lock's choices, as Maschine lists them.
pub const LOCK_MODES: [&str; 2] = ["Travel", "Target"];
pub const LOCK_TIMES: [&str; 6] = ["1/2 Bar", "1 Bar", "2 Bars", "4 Bars", "8 Bars", "16 Bars"];
pub const LOCK_GRIDS: [&str; 4] = ["1/8", "1/4", "1/2", "Bar"];
/// MIDI Change's triggers.
pub const MIDI_TRIGGERS: [&str; 3] = ["None", "MIDI Note", "Program Change"];

impl Lock {
    fn read(ix: &Index, m: &Obj) -> Lock {
        let int = |f: &str, d: i64| param_i64(m, f).unwrap_or(d).max(0) as usize;
        let mut l = Lock {
            morph: param_i64(m, "morph").unwrap_or(0) != 0,
            mode: int("morph_mode", 0).min(1),
            time: int("morph_time", 1).min(5),
            grid: int("morph_grid", 3).min(3),
            focus_bank: int("focus_bank", 0).min(3),
            banks: int("banks", 1).clamp(1, 4),
            ..Lock::default()
        };
        let Some(map) = m.get("snapshots").and_then(|v| ix.obj(v)) else { return l };
        let current = map.get("current").and_then(|v| ix.obj(v)).map(|o| o as *const Obj);
        let items = map.get("objects").and_then(|v| ix.obj(v)).and_then(|o| o.get("items"));
        if let Some(Value::List(items)) = items {
            for it in items {
                let Value::Obj(pair) = it else { continue };
                let Some(slot) = pair.get("first").and_then(Value::as_int) else { continue };
                // (`obj` sees through the shared_ptr.)
                let snap = pair.get("second").and_then(|v| ix.obj(v));
                let Some(snap) = snap else { continue };
                let slot = slot.max(0) as usize;
                l.slots.push(slot);
                if current == Some(snap as *const Obj) {
                    l.focus = Some(slot);
                }
            }
        }
        l.slots.sort_unstable();
        l
    }
}

/// Where a channel's audio goes (`AudioOutputParameterValue`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AudioDest {
    #[default]
    None,
    /// The channel above: a sound's group, a group's master.
    Parent,
    Master,
    /// External stereo output `n` (0 = Ext. 1).
    Ext(i64),
    Group(usize),
    Sound(usize, usize),
}

/// What a sound takes in as audio (`AudioInputParameterValue`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AudioSource {
    #[default]
    None,
    /// External input `n`: below the number of stereo inputs a pair (`In n+1 L+R`),
    /// then the mono inputs in turn (`In 1 L`, `In 1 R`, …).
    Ext(i64),
    Sound(usize, usize),
    Master,
    Group(usize),
}

/// What feeds an effect's side-chain input (`SideChainInputParameterValue`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScSource {
    Group(usize),
    Sound(usize, usize),
}

/// Where a sound sends MIDI (`EventOutputParameterValue`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum MidiDest {
    #[default]
    None,
    /// A MIDI output port, by name ("Host" in a plug-in).
    Port(String),
    /// Sound `n` of the same group (a multitimbral plug-in).
    Sound(usize),
}

/// An aux send.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Aux {
    pub dest: AudioDest,
    /// Stored level (0.75 = 0 dB).
    pub level: f64,
    /// Before the channel's level and pan (else after).
    pub pre: bool,
}

/// A sound's channel properties (Input, Output) beyond level, pan and mute.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundChannel {
    /// MIDI input port name: "Default", "Focus", "None", "All" or a port.
    pub midi_source: String,
    /// 0 = all channels, else 1–16.
    pub midi_channel: i64,
    pub midi_thru: bool,
    pub audio_in: AudioSource,
    pub input_gain: f64,
    pub dest: AudioDest,
    pub audio_mute: bool,
    pub aux_dest: [AudioDest; 2],
    pub aux_pre: [bool; 2],
    pub midi_out: MidiDest,
    /// 0–15 (channel 1–16).
    pub midi_out_channel: i64,
    /// Semitones added to the notes sent.
    pub transpose: i64,
    /// The note the sound plays at its original pitch (60 = C3).
    pub base_key: i64,
    /// Pad mode's Choke and Link groups: 0 = None, else 1–8; modes 0 = Send, 1 =
    /// Receive. A Send sound's note mutes (choke) or also plays (link) the group's other
    /// sounds in its choke or link group.
    pub choke_group: i64,
    pub choke_mode: i64,
    pub link_group: i64,
    pub link_mode: i64,
    /// Side-chain sources of the effects in its chain: (slot, source).
    pub sidechain: Vec<(usize, ScSource)>,
}

/// A group's channel properties: MIDI input and routing.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupChannel {
    /// 0 = Off, 1 = Drumkit, 2 = Manual.
    pub key_mode: i64,
    pub midi_source: String,
    pub midi_channel: i64,
    /// Sound 1 plays from this note, sound 2 from the next, … (Manual).
    pub start_note: i64,
    pub midi_thru: bool,
    pub dest: AudioDest,
    pub aux_dest: [AudioDest; 2],
    pub aux_pre: [bool; 2],
    /// Its MIDI files hold each sound on its own channel (0) or note (1).
    pub midi_batch: i64,
}

impl Default for SoundChannel {
    /// As Maschine makes a sound: MIDI from the Default source on all channels with
    /// Thru, out to its group, MIDI out off.
    fn default() -> SoundChannel {
        SoundChannel {
            midi_source: "Default".into(),
            midi_channel: 0,
            midi_thru: true,
            audio_in: AudioSource::None,
            input_gain: 0.75,
            dest: AudioDest::Parent,
            audio_mute: false,
            aux_dest: [AudioDest::None; 2],
            aux_pre: [false; 2],
            midi_out: MidiDest::None,
            midi_out_channel: 0,
            transpose: 0,
            base_key: 60,
            choke_group: 0,
            choke_mode: 0,
            link_group: 0,
            link_mode: 0,
            sidechain: Vec::new(),
        }
    }
}

impl Default for GroupChannel {
    /// A new group: MIDI input off, out to the master.
    fn default() -> GroupChannel {
        GroupChannel {
            key_mode: 0,
            midi_source: "None".into(),
            midi_channel: 0,
            start_note: 36,
            midi_thru: false,
            dest: AudioDest::Parent,
            aux_dest: [AudioDest::None; 2],
            aux_pre: [false; 2],
            midi_batch: 0,
        }
    }
}

impl Default for Project {
    /// An empty project: 120 BPM in 4/4, the master at 0 dB.
    fn default() -> Project {
        Project {
            name: String::new(),
            tempo: 120.0,
            numerator: 4,
            denominator: 4,
            groups: Vec::new(),
            scenes: Vec::new(),
            sections: Vec::new(),
            swing: Swing::default(),
            master_level: 0.75,
            master_pan: 0.0,
            master_slots: Vec::new(),
            arp: Arp::default(),
            arp_presets: ARP_PRESETS.to_vec(),
            mixer_sounds: false,
            master_output: AudioDest::Ext(0),
            cue_output: AudioDest::Ext(0),
            cue_level: 0.75,
            cue_pan: 0.0,
            lock: Lock::default(),
            lfos: Vec::new(),
            cc_routes: Vec::new(),
            warnings: Vec::new(),
            expressions: Vec::new(),
        }
    }
}

/// Groove ("Swing" page of a channel's Groove properties).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Swing {
    /// 0–100 %.
    pub amount: f64,
    /// Index into Maschine's swing cycles: 1/8, 3/16, 1/4, 3/8, 1/2, 9/16, 1, 2 (of a
    /// whole note); 0 (1/8, swinging 16ths) is the default.
    pub cycle: i64,
    /// Events come early instead of late.
    pub invert: bool,
}

/// The gain of a stored level (`Level`, `Gain`, aux `Level`: 0..1, 0.75 = unity, 1 =
/// +10 dB), as Maschine's level converter maps it.
pub fn level_gain(level: f64) -> f64 {
    let x = level.max(0.0);
    2.3948107 * x.powi(5) + 0.767467 * x * x
}

/// A stored level in dB (−∞ at 0).
pub fn level_db(level: f64) -> f64 {
    20.0 * level_gain(level).log10()
}

/// The stored level for `db` (the inverse of [`level_db`]), within 0..1.
pub fn level_from_db(db: f64) -> f64 {
    if !db.is_finite() || db < -200.0 {
        return 0.0;
    }
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if level_db(mid) < db { lo = mid } else { hi = mid }
    }
    (lo + hi) / 2.0
}

/// Cycle lengths (`Swing::s_aSwingLengths`, in 1/24 quarter notes) and how strongly the
/// amount acts at each (`s_aSwingComp`), from Maschine.
pub const SWING_CYCLES: [(&str, f64, f64); 8] = [
    ("1/8", 12.0, 1.0),
    ("3/16", 18.0, 0.5),
    ("1/4", 24.0, 0.5),
    ("6/16", 36.0, 0.25),
    ("1/2", 48.0, 0.25),
    ("9/16", 56.0, 0.160_714),
    ("1/1", 96.0, 0.125),
    ("2/1", 192.0, 0.0625),
];

impl Swing {
    pub fn is_off(&self) -> bool {
        self.amount == 0.0
    }

    /// Where a straight position (in quarter notes) plays with this groove: Maschine's
    /// `Swing::calcToSwingPos`. Each cycle's midpoint moves to
    /// `0.5 + 0.45·comp·(|a| + a²)/2` (a = amount, negated when inverted); the halves
    /// stretch to fit.
    pub fn apply(&self, beats: f64) -> f64 {
        let (_, len, comp) = SWING_CYCLES[self.cycle.clamp(0, 7) as usize];
        let a = (self.amount / 100.0).clamp(-1.0, 1.0) * if self.invert { -1.0 } else { 1.0 };
        let curve = (a.abs() + a * a) * 0.5;
        let k = if a > 0.0 { -curve } else { curve };
        let mid = comp * k * -0.45 + 0.5;
        let units = beats * 24.0;
        let start = len * (units / len).floor();
        let f = (units - start) / len;
        let g = if f < 0.5 { f * 2.0 * mid } else { mid + 2.0 * (1.0 - mid) * (f - 0.5) };
        (start + len * g) / 24.0
    }
}

fn swing_of(o: &Obj) -> Swing {
    Swing {
        amount: param_f64(o, "swing_amount").unwrap_or(0.0),
        cycle: param_i64(o, "swing_cycle").unwrap_or(0),
        invert: param_i64(o, "swing_invert").unwrap_or(0) != 0,
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Group {
    pub name: String,
    pub color: Color,
    /// Linear amplitude (1.0 = 0 dB).
    pub level: f64,
    pub pan: f64,
    pub muted: bool,
    /// Sent to the cue (headphones) bus.
    pub cue: bool,
    /// The group's plug-in chain (its effects).
    pub slots: Vec<SlotInfo>,
    /// Aux 1 and 2 send levels (linear).
    pub aux: [f64; 2],
    pub sounds: Vec<Sound>,
    pub patterns: Vec<Pattern>,
    pub swing: Swing,
    pub channel: GroupChannel,
    /// The group's modulated parameters (its Control Lane's modulation lanes).
    pub modulators: Vec<ModTarget>,
    /// Clips (the Song view): each clip's own pattern (`patterns`, an object vector of
    /// `EventPattern`s), and where the clips play (the group's `sequence` of
    /// `ClipEvent`s), in time order.
    pub clip_patterns: Vec<Pattern>,
    pub clips: Vec<Clip>,
}

/// A clip on a group's lane in the Song view: from `position` for `length` ticks (from
/// the song's start) it plays clip pattern `pattern` (repeated to fill it), in place of
/// the group's pattern in the section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Clip {
    pub position: i64,
    pub length: i64,
    pub pattern: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sound {
    pub name: String,
    pub color: Color,
    /// Linear amplitude (1.0 = 0 dB).
    pub level: f64,
    /// -1 (left) .. 1 (right).
    pub pan: f64,
    pub muted: bool,
    /// Sent to the cue (headphones) bus.
    pub cue: bool,
    /// Aux 1 and 2 send levels (linear).
    pub aux: [f64; 2],
    /// Short type name of the first module in the plug-in chain, e.g. "Sampler",
    /// "Kick" (drum synth), "PluginHost"; empty if the chain is empty.
    pub instrument: String,
    /// The first Sampler in the sound's plug-in chain, if any.
    pub sampler: Option<Sampler>,
    /// The Audio plug-in, when it's the sound's (docs/audio-plugin.md).
    pub audio: Option<AudioLoop>,
    /// The sound's instrument, when it is a hosted plug-in (VST/VST3).
    pub plugin: Option<PluginRef>,
    pub swing: Swing,
    /// The sound's plug-in chain: every slot, in order (instrument first, then effects).
    pub slots: Vec<SlotInfo>,
    /// The lanes listed under the sound's notes (`midi_automators`), in order, and
    /// which one is shown.
    pub lanes: Vec<LaneRef>,
    pub lane: Option<usize>,
    pub channel: SoundChannel,
    /// The sound's modulated parameters (its Control Lane's modulation lanes).
    pub modulators: Vec<ModTarget>,
    /// What pressing a held MK3 pad does for it (DECKARD's; [`pad_touch`]).
    pub pad_touch: pad_touch::PadTouch,
    /// Its modulation routes (DECKARD's; [`mod_matrix`]).
    pub mod_routes: Vec<mod_matrix::ModRoute>,
    /// PROB LOCK: its notes' chances rolled once and kept (DECKARD's [`PROB_LOCK`]
    /// field; Maschine's sound has its own, not in its saved projects).
    pub probability_locked: bool,
}

/// The sound object's field that has PROB LOCK on (`Int(1)`; absent when off).
pub const PROB_LOCK: &str = "deckard:prob_lock";

/// A lane of the pattern editor, as saved (`NI::MASCHINE::DATA::MidiAutomator`):
/// `kind` is a MIDI event type (4 = control change, `number` the controller; 3 =
/// pitch bend), or 2 / 8 for the note lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LaneRef {
    pub kind: i64,
    pub number: i64,
}

/// One slot of a plug-in chain.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SlotInfo {
    /// The module's class without its namespace: "Sampler", "PluginHost", "Compressor", …
    pub module: String,
    /// What the slot shows: the plug-in's name, or the module's.
    pub name: String,
    /// Off = bypassed.
    pub active: bool,
    /// The plug-in, for a `PluginHost` slot.
    pub plugin: Option<PluginRef>,
    /// An internal module's parameters (stored values, by field name).
    pub params: Vec<(String, f64)>,
}

impl SlotInfo {
    /// Parameter `field` of an internal module, as stored.
    pub fn param(&self, field: &str) -> Option<f64> {
        self.params.iter().find(|(k, _)| k == field).map(|(_, v)| *v)
    }
}

/// A hosted plug-in (a `PluginHost` module): which plug-in, and its saved state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginRef {
    /// Maschine's id, e.g. `"Native Instruments:VST3:INSTR:5653…"`.
    pub id: String,
    pub name: String,
    /// The saved state, as stored (`[u32 1][u32 1][u32 n][n bytes]`).
    pub chunk: Vec<u8>,
    /// Parameter pages saved with it (eight knobs each; empty when Maschine maps them
    /// automatically).
    pub pages: Vec<PluginPage>,
}

/// A page of a plug-in's parameters, as saved.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginPage {
    pub name: String,
    /// Up to eight knobs: the plug-in's parameter id, a name given in Maschine (empty:
    /// the plug-in's), the section heading starting at it.
    pub params: Vec<PluginKnob>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginKnob {
    pub id: u32,
    pub name: String,
    pub section: String,
}

/// Settings of Maschine's built-in Sampler module that playback uses.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sampler {
    pub zones: Vec<Zone>,
    /// Every parameter as stored, by its path from the Sampler (see
    /// [`SAMPLER_PARAMS`]); missing ones are absent.
    pub params: BTreeMap<String, f64>,
    /// Semitones.
    pub tune: f64,
    pub reverse: bool,
    /// 0 = one-shot (sample plays to its end), otherwise an envelope gated by the note.
    pub envelope_type: i64,
    /// Volume envelope, normalized 0..1 as stored.
    pub attack: f64,
    pub hold: f64,
    pub decay: f64,
    pub sustain: f64,
    pub release: f64,
    /// How much velocity scales volume (0..1).
    pub velocity_to_volume: f64,
}

/// The Sampler's parameters, as paths from the Sampler module (`a/b` is field `b` of
/// its object `a`), in the order stored, with Maschine's defaults
/// (docs/sampler-dsp.md §1).
pub const SAMPLER_PARAMS: &[(&str, f64)] = &[
    ("editor_tab", 0.0),
    ("polyphony", 4.0),
    ("glide", 0.0),
    ("pitchbend", 0.5),
    ("mode", 0.0),
    ("envelope_type", 0.0),
    ("reverse", 0.0),
    ("tune", 0.0),
    ("start", 0.0),
    ("comp", 0.0),
    ("drive", 0.0),
    ("sample_rate", 0.0),
    ("bits", 0.0),
    ("filter", 0.0),
    ("cutoff", 0.5),
    ("resonance", 0.0),
    ("eq_frequency", 0.5),
    ("eq_width", 0.25),
    ("eq_gain", 0.0),
    ("velocity_start", 0.0),
    ("velocity_decay", 0.0),
    ("velocity_cutoff", 0.0),
    ("velocity_volume", 1.0),
    ("modwheel_start", 0.0),
    ("modwheel_cutoff", 0.0),
    ("modwheel_lfo", 0.0),
    ("modwheel_pan", 0.0),
    ("volume_envelope/attack", 0.0),
    ("volume_envelope/hold", 0.5),
    ("volume_envelope/decay", 0.5),
    ("volume_envelope/sustain", 1.0),
    ("volume_envelope/release", 0.5),
    ("mod_envelope/attack", 0.0),
    ("mod_envelope/hold", 0.5),
    ("mod_envelope/decay", 0.5),
    ("mod_envelope/sustain", 1.0),
    ("mod_envelope/release", 0.5),
    ("mod_envelope/pitch", 0.0),
    ("mod_envelope/cutoff", 0.0),
    ("mod_envelope/drive", 0.0),
    ("mod_envelope/pan", 0.0),
    ("lfo/type", 0.0),
    ("lfo/speed", 0.5),
    ("lfo/phase", 0.0),
    ("lfo/sync_mode", 0.0),
    ("lfo/pitch_amount", 0.0),
    ("lfo/cutoff_amount", 0.0),
    ("lfo/drive_amount", 0.0),
    ("lfo/pan_amount", 0.0),
];

impl Sampler {
    /// A parameter's stored value, or Maschine's default if the file has none.
    pub fn get(&self, path: &str) -> f64 {
        self.params.get(path).copied().unwrap_or_else(|| SAMPLER_PARAMS.iter().find(|(k, _)| *k == path).map_or(0.0, |(_, d)| *d))
    }

    /// Change a parameter, keeping the fields derived from it up to date.
    pub fn set(&mut self, path: &str, value: f64) {
        self.params.insert(path.to_string(), value);
        self.derive();
    }

    fn derive(&mut self) {
        self.tune = self.get("tune");
        self.reverse = self.get("reverse") != 0.0;
        self.envelope_type = self.get("envelope_type") as i64;
        self.attack = self.get("volume_envelope/attack");
        self.hold = self.get("volume_envelope/hold");
        self.decay = self.get("volume_envelope/decay");
        self.sustain = self.get("volume_envelope/sustain");
        self.release = self.get("volume_envelope/release");
        self.velocity_to_volume = self.get("velocity_volume");
    }
}

/// One sample mapping of a Sampler.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Zone {
    pub sample: SampleRef,
    pub root_key: i64,
    /// Inclusive MIDI note range.
    pub key_range: (i64, i64),
    /// Inclusive velocity range.
    pub velocity_range: (i64, i64),
    /// Linear amplitude.
    pub gain: f64,
    pub pan: f64,
    /// Semitones.
    pub tune: f64,
    /// Sample frames played (start, end).
    pub play_range: (i64, i64),
    pub loop_range: (i64, i64),
    pub loop_active: bool,
    /// Loop crossfade, in sample frames.
    pub x_fade: i64,
    /// Zone fade-in and fade-out (`attack`, `decay`), in sample frames.
    pub fade_in: i64,
    pub fade_out: i64,
}

impl Zone {
    /// Change a parameter (`play_range/low`, `loop_active`, ...) as [`Sampler::set`].
    /// False for ones the model doesn't keep.
    pub fn set(&mut self, path: &str, v: f64) -> bool {
        let i = v as i64;
        match path {
            "play_range/low" => self.play_range.0 = i,
            "play_range/high" => self.play_range.1 = i,
            "loop_range/low" => self.loop_range.0 = i,
            "loop_range/high" => self.loop_range.1 = i,
            "loop_active" => self.loop_active = i != 0,
            "x_fade" => self.x_fade = i,
            "attack" => self.fade_in = i,
            "decay" => self.fade_out = i,
            "root_key" => self.root_key = i,
            "key_range/low" => self.key_range.0 = i,
            "key_range/high" => self.key_range.1 = i,
            "velocity_range/low" => self.velocity_range.0 = i,
            "velocity_range/high" => self.velocity_range.1 = i,
            "gain" => self.gain = v,
            "pan" => self.pan = v,
            "tune" => self.tune = v,
            _ => return false,
        }
        true
    }
}

/// Where a sample file lives, as stored (`NI::MASCOMMON::FileLocation`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SampleRef {
    /// 2 = relative to a content library (all files seen so far).
    pub kind: i64,
    pub path: String,
    /// Library root on the machine that saved the file (may be another OS).
    pub library_root: String,
    /// Library product name, e.g. "Maschine 2 Factory Library".
    pub library: String,
}

/// A sound's Audio plug-in: its takes (the Audio Pool) and settings
/// (docs/audio-plugin.md).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioLoop {
    pub takes: Vec<Take>,
    /// The take in focus (the Source Tempo and Length shown are its).
    pub focused: usize,
    pub mode: AudioMode,
    /// Gate mode's fade, normalized (0.1222 new).
    pub fade: f64,
    /// Semitones (−36…36).
    pub tune: f64,
    /// The pitch wheel's range, normalized (0.5 new).
    pub pitchbend: f64,
    pub engine: AudioEngine,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AudioMode {
    /// Plays with the pattern, without notes.
    #[default]
    Loop,
    /// Heard during notes, which pitch it.
    Gate,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AudioEngine {
    /// Speed follows the tempo, pitch with it.
    Repitch,
    /// Tempo and pitch independent.
    #[default]
    Stretch,
    /// As Stretch, keeping the formants.
    Formant,
}

/// One take: its sample, its length in beats (the Source Length; the Source Tempo
/// follows from the sample's frames) and the frames it plays.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Take {
    pub sample: SampleRef,
    pub beats: f64,
    /// Frames played (low, high; high inclusive, as a zone's); (0, 0): the whole file.
    pub play_range: (i64, i64),
}

impl Take {
    /// The source tempo (BPM) of a take `frames` long at `rate` Hz.
    pub fn tempo(&self, frames: usize, rate: u32) -> f64 {
        if frames == 0 || self.beats <= 0.0 { 120.0 } else { self.beats * 60.0 * f64::from(rate) / frames as f64 }
    }
}

/// A sound's audio on in a pattern (its `AudioEvent`): the take it plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioCue {
    pub sound: usize,
    pub take: usize,
}

/// A modulated parameter (a lane of a channel's `modulators`): a parameter of the
/// module in chain slot `slot` (`field` as [`SlotInfo::params`] names it), or with no
/// slot, a property of the channel (sound or group) itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModTarget {
    pub slot: Option<usize>,
    pub field: String,
}

/// A modulation point (`ModulationEvent`) of the sound `sound` (`None`: the group):
/// from `position` (ticks) the parameter deviates by `value` (knobs: a fraction of its
/// range, −1..1) or is `value` (selectors, buttons). Points are joined by lines
/// (docs/superpowers/plans/2026-10-22-phase2c-modulation.md).
#[derive(Debug, Clone, PartialEq)]
pub struct ModPoint {
    pub sound: Option<usize>,
    pub slot: Option<usize>,
    pub field: String,
    pub position: i64,
    pub value: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pattern {
    pub name: String,
    pub color: Color,
    /// Ticks.
    pub start: i64,
    /// Ticks.
    pub length: i64,
    pub notes: Vec<Note>,
    /// Sounds whose Audio plug-in plays in the pattern, and the take each plays.
    pub audio: Vec<AudioCue>,
    /// MIDI controller and pitch-bend events recorded with the notes (e.g. a sustain
    /// pedal), sorted by position.
    pub midi: Vec<MidiEvent>,
    /// Modulation points, by sound, slot, field and position.
    pub mods: Vec<ModPoint>,
}

/// A recorded MIDI event that is not a note (`NI::MASCHINE::DATA::MidiEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiEvent {
    pub sound: usize,
    /// Ticks, like a note's position.
    pub position: i64,
    pub kind: MidiKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiKind {
    /// Control change (stored as type 4): controller number and value 0..127. 64 is
    /// the sustain pedal, 1 the mod wheel.
    Controller { number: u8, value: u8 },
    /// Pitch bend (type 3): 0..16383, 8192 = centre.
    PitchBend(u16),
    /// Channel pressure (type 6, Maschine's "Aftertouch"): 0..127.
    ChannelPressure(u8),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Note {
    /// Index into the group's `sounds`.
    pub sound: usize,
    /// Ticks from pattern start.
    pub position: i64,
    /// Ticks.
    pub length: i64,
    pub pitch: i64,
    pub velocity: i64,
    /// Chance of playing, 0–100 % (Maschine 3; older projects always play: 100).
    pub probability: i64,
    /// Its expression (MPE), in the document's table; DECKARD only.
    pub expr: Option<ExprId>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub name: String,
    pub color: Color,
    /// (group index, pattern index) for each group that plays a pattern in this scene.
    pub patterns: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Section {
    pub scene: Option<usize>,
    /// Ticks.
    pub length: i64,
    /// The length follows the scene's (its longest pattern).
    pub auto: bool,
}

/// The arpeggiator's order (Maschine's TYPE).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ArpType {
    #[default]
    Up,
    Down,
    UpDown,
    OrderPlayed,
    Chord,
}

impl ArpType {
    pub const ALL: [ArpType; 5] = [ArpType::Up, ArpType::Down, ArpType::UpDown, ArpType::OrderPlayed, ArpType::Chord];

    pub fn name(self) -> &'static str {
        match self {
            ArpType::Up => "Up",
            ArpType::Down => "Down",
            ArpType::UpDown => "Up & Down",
            ArpType::OrderPlayed => "Order Played",
            ArpType::Chord => "Chord",
        }
    }
}

/// Note Repeat / Arp (MK3 manual, "Note Repeat and Arp"; the song's `arpeggiator`):
/// one for the project.
#[derive(Debug, Clone, PartialEq)]
pub struct Arp {
    pub kind: ArpType,
    /// Ticks between notes (240: 1/16).
    pub rate: i64,
    /// 0: off; 1–8: a rhythm with rests.
    pub sequence: i64,
    /// How many octaves (1–8).
    pub octaves: i64,
    /// Velocity factor (0.01–2).
    pub dynamic: f64,
    /// Note length as a factor of the rate (0–2).
    pub gate: f64,
    pub hold: bool,
    pub retrigger: i64,
    pub repeat: i64,
    /// Steps shifted (−8…8).
    pub offset: i64,
    pub inversion: i64,
    pub min_key: i64,
    pub max_key: i64,
}

impl Default for Arp {
    fn default() -> Arp {
        Arp {
            kind: ArpType::Up,
            rate: PPQ / 4,
            sequence: 0,
            octaves: 1,
            dynamic: 1.0,
            gate: 1.0,
            hold: false,
            retrigger: 0,
            repeat: 0,
            offset: 0,
            inversion: 0,
            min_key: 0,
            max_key: 127,
        }
    }
}

impl Arp {
    fn read(a: &Obj) -> Arp {
        let d = Arp::default();
        let i = |f: &str, def: i64| param_i64(a, f).unwrap_or(def);
        Arp {
            kind: ArpType::ALL.get(i("type", 0) as usize).copied().unwrap_or_default(),
            rate: i("pc0", d.rate).max(1),
            sequence: i("sequence", 0),
            octaves: i("octaves", 0) + 1,
            dynamic: param_f64(a, "dynamic").unwrap_or(1.0),
            gate: param_f64(a, "gate").unwrap_or(1.0),
            hold: i("hold", 0) != 0,
            retrigger: i("retrigger", 0),
            repeat: i("repeat", 0),
            offset: i("offset", 8) - 8,
            inversion: i("inversion", 0),
            min_key: i("min_key", 0),
            max_key: i("max_key", 127),
        }
    }
}

/// Note Repeat's presets in a new project, as Maschine 3 makes them: 1/8, 1/16,
/// 1/16 Triplet, 1/32 ([`Project::arp_presets`]).
pub const ARP_PRESETS: [(i64, i64); 4] = [(3, 0), (4, 0), (4, 1), (5, 0)];

/// A palette slot (0–19; the app rejects larger values), or the default color.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Color {
    #[default]
    Default,
    Palette(u8),
}

/// Where each part of the project lives in the decoded tree (paths from the root
/// object), for editing it: parameters are found by name below these objects.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Addresses<P = Path> {
    /// The song (tempo, signature).
    pub song: Option<P>,
    pub groups: Vec<GroupAddresses<P>>,
    pub scenes: Vec<P>,
    pub sections: Vec<P>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupAddresses<P = Path> {
    pub group: P,
    /// Where the group's level, pan and mute live (the group, or its legacy strip).
    pub strip: P,
    pub sounds: Vec<P>,
    pub patterns: Vec<P>,
    /// The clips' patterns (`Group::clip_patterns`).
    pub clip_patterns: Vec<P>,
}

impl Addresses<*const Obj> {
    /// Paths of the objects, found in one walk of `root`.
    fn resolve(self, root: &Obj) -> Addresses {
        let mut want: HashMap<*const Obj, Path> = HashMap::new();
        let mut add = |p: &*const Obj| {
            want.insert(*p, Path::new());
        };
        self.song.iter().for_each(&mut add);
        for g in &self.groups {
            add(&g.group);
            add(&g.strip);
            g.sounds.iter().chain(&g.patterns).chain(&g.clip_patterns).for_each(&mut add);
        }
        self.scenes.iter().chain(&self.sections).for_each(&mut add);
        let mut found = std::collections::HashSet::new();
        root.walk_paths(&mut |o, path| {
            let k = o as *const Obj;
            if let Some(slot) = want.get_mut(&k)
                && found.insert(k)
            {
                *slot = path.to_vec();
            }
        });
        let path = |p: &*const Obj| want.get(p).cloned().unwrap_or_default();
        Addresses {
            song: self.song.as_ref().map(path),
            groups: self
                .groups
                .iter()
                .map(|g| GroupAddresses {
                    group: path(&g.group),
                    strip: path(&g.strip),
                    sounds: g.sounds.iter().map(path).collect(),
                    patterns: g.patterns.iter().map(path).collect(),
                    clip_patterns: g.clip_patterns.iter().map(path).collect(),
                })
                .collect(),
            scenes: self.scenes.iter().map(path).collect(),
            sections: self.sections.iter().map(path).collect(),
        }
    }
}

/// Object id → object, for resolving back-references.
pub struct Index<'a> {
    /// Object ids count up from 0 in an archive, so a vector indexed by id.
    by_oid: Vec<Option<&'a Obj>>,
}

impl<'a> Index<'a> {
    pub fn of(root: &'a Obj) -> Self {
        let mut ix = Index { by_oid: Vec::new() };
        root.walk_paths(&mut |o, _| ix.add(o));
        ix
    }

    pub fn new(root: &'a Value) -> Self {
        let mut ix = Index { by_oid: Vec::new() };
        root.walk(&mut |o| ix.add(o));
        ix
    }

    fn add(&mut self, o: &'a Obj) {
        let Some(oid) = o.oid else { return };
        let Ok(i) = usize::try_from(oid) else { return };
        if i >= self.by_oid.len() {
            self.by_oid.resize(i + 1, None);
        }
        self.by_oid[i].get_or_insert(o);
    }

    /// The object a value is or refers to, looking through `shared_ptr`/`scoped_ptr`
    /// wrappers (`px`) and `sp` (weak_ptr) holders.
    pub fn obj(&self, v: &'a Value) -> Option<&'a Obj> {
        let o = match v {
            Value::Obj(o) => o,
            Value::Ref(oid) => self.by_oid.get(usize::try_from(*oid).ok()?).copied().flatten()?,
            _ => return None,
        };
        match o.get("px").or_else(|| o.get("sp")) {
            Some(inner) if o.type_name.starts_with("std::") || o.type_name.starts_with("boost::") => self.obj(inner),
            _ => Some(o),
        }
    }

    /// Elements of an `ObjectVector<T>` or `ObjectMap<K, T>` field (map values for maps).
    pub fn elements(&self, container: Option<&'a Value>) -> Vec<&'a Obj> {
        let Some(c) = container.and_then(|v| self.obj(v)) else { return Vec::new() };
        let Some(Value::Obj(objects)) = c.get("objects") else { return Vec::new() };
        let Some(Value::List(items)) = objects.get("items") else { return Vec::new() };
        items
            .iter()
            .filter_map(|it| match it {
                // map entries: pair { first, second }
                Value::Obj(p) if p.get("second").is_some() => self.obj(p.get("second")?),
                other => self.obj(other),
            })
            .collect()
    }
}

/// `field.value` of a parameter object (searching base classes).
fn param<'v>(o: &'v Obj, field: &str) -> Option<&'v Value> {
    match o.find(field)? {
        Value::Obj(p) => p.find("value"),
        _ => None,
    }
}

fn param_f64(o: &Obj, field: &str) -> Option<f64> {
    match param(o, field)? {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f64),
        _ => None,
    }
}

fn param_i64(o: &Obj, field: &str) -> Option<i64> {
    param(o, field)?.as_int()
}

fn param_str(o: &Obj, field: &str) -> String {
    match param(o, field) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn color(o: &Obj) -> Color {
    let Some(Value::Obj(c)) = o.find("color") else { return Color::Default };
    let default = c.find("use_default_color").and_then(Value::as_int).unwrap_or(0) != 0;
    match c.find("color").and_then(Value::as_int) {
        Some(i) if !default && (0..20).contains(&i) => Color::Palette(i as u8),
        _ => Color::Default,
    }
}

impl Project {
    /// Build from a decoded `Project` archive root.
    pub fn from_decoded(root: Obj) -> Project {
        Project::read(&root).0
    }

    /// Build from a decoded `Project` archive root, with where each part lives in it.
    pub fn read(r: &Obj) -> (Project, Addresses) {
        let ix = Index::of(r);
        let path_of = |o: &Obj| o as *const Obj;
        let mut addr: Addresses<*const Obj> = Addresses::default();
        let mut p = Project { name: param_str(r, "name"), ..Default::default() };
        if let Some(Value::Str(t)) = r.get(lfo::FIELD) {
            p.lfos = lfo::from_text(t);
        }
        if let Some(Value::Str(t)) = r.get(mod_matrix::CC_FIELD) {
            p.cc_routes = mod_matrix::cc_from_text(t);
        }
        let songs = ix.elements(r.get("songs"));
        let Some(song) = songs.first() else {
            p.warnings.push("project has no song".into());
            return (p, Addresses::default());
        };
        addr.song = Some(path_of(song));
        if songs.len() > 1 {
            p.warnings.push(format!("{} songs; showing the first", songs.len()));
        }
        p.tempo = param_f64(song, "tempo").unwrap_or(120.0);
        p.swing = swing_of(song);
        p.master_level = param_f64(song, "level").unwrap_or(0.75);
        p.master_pan = param_f64(song, "pan").unwrap_or(0.0);
        p.master_slots = chain_slots(&ix, song);
        if let Some(a) = song.find("arpeggiator").and_then(|v| ix.obj(v)) {
            p.arp = Arp::read(a);
        }
        if let Some(Value::List(items)) = song.find("arp_presets").and_then(|v| ix.obj(v)).and_then(|o| o.get("items")) {
            let int = |o: &Obj, f: &str| match o.get(f) {
                Some(Value::Int(i)) => *i,
                _ => 0,
            };
            p.arp_presets = items.iter().filter_map(|v| ix.obj(v)).map(|o| (int(o, "i00"), int(o, "i08"))).collect();
        }
        p.mixer_sounds = param_i64(song, "sounds_visible_mixer").unwrap_or(0) != 0;
        if let Some(m) = song.find("parameter_snapshots").and_then(|v| ix.obj(v)) {
            p.lock = Lock::read(&ix, m);
        }
        if let Some(r) = song.find("snapshot_recall").and_then(|v| ix.obj(v)) {
            // Maschine 3: the trigger; before: on/off and a kind (0 notes, 1 programs).
            p.lock.midi_trigger = match param_i64(r, "trigger") {
                Some(t) => t.clamp(0, 2) as usize,
                None if param_i64(r, "legacy_bool").unwrap_or(0) != 0 => {
                    (param_i64(r, "legacy_enum").unwrap_or(0).clamp(0, 1) + 1) as usize
                }
                None => 0,
            };
            if let Some(Value::Str(s)) = param(r, "input") {
                p.lock.midi_source = s.clone();
            }
            p.lock.midi_channel = param_i64(r, "midi_channel").unwrap_or(1).clamp(1, 16);
        }
        p.numerator = param_i64(song, "numerator").unwrap_or(4);
        // Stored as a power of two (2 -> quarter notes).
        p.denominator = param_i64(song, "denominator").filter(|e| (0..6).contains(e)).map_or(4, |e| 1 << e);

        // Where routing pointers lead: groups and sounds by object.
        let mut targets = Targets::default();
        for (gi, g) in ix.elements(song.get("groups")).into_iter().enumerate() {
            targets.groups.insert(g as *const Obj, gi);
            for (si, s) in ix.elements(g.get("sounds")).into_iter().enumerate() {
                targets.sounds.insert(s as *const Obj, (gi, si));
            }
        }
        p.master_output = param(song, "master_output").and_then(|v| ix.obj(v)).map_or(AudioDest::Ext(0), |v| audio_dest(&ix, v, &targets));
        p.cue_output = param(song, "cue_output").and_then(|v| ix.obj(v)).map_or(AudioDest::Ext(0), |v| audio_dest(&ix, v, &targets));
        p.cue_level = param_f64(song, "cue_level").unwrap_or(0.75);
        p.cue_pan = param_f64(song, "cue_pan").unwrap_or(0.0);

        // Groups, with oid -> index maps for resolving scene/pattern references.
        let mut group_ix: HashMap<*const Obj, usize> = HashMap::new();
        let mut pattern_ix: HashMap<*const Obj, (usize, usize)> = HashMap::new();
        for (gi, g) in ix.elements(song.get("groups")).into_iter().enumerate() {
            group_ix.insert(g as *const Obj, gi);
            // Groups before v13 keep their channel strip in a legacy block.
            let strip = match g.get("track_parameters_legacy") {
                Some(Value::Obj(t)) => t,
                _ => g,
            };
            let mut group = Group {
                name: param_str(g, "name"),
                color: color(g),
                level: param_f64(strip, "level").unwrap_or(0.75),
                pan: param_f64(strip, "pan").unwrap_or(0.0),
                muted: param_i64(strip, "mute").unwrap_or(0) != 0,
                cue: param_i64(strip, "cue_enabled").unwrap_or(0) != 0,
                slots: chain_slots(&ix, g),
                aux: [param_f64(strip, "aux1_level").unwrap_or(0.75), param_f64(strip, "aux2_level").unwrap_or(0.75)],
                swing: swing_of(g),
                ..Default::default()
            };
            let sounds = ix.elements(g.get("sounds"));
            let sound_ix: HashMap<*const Obj, usize> = sounds.iter().enumerate().map(|(i, s)| (*s as *const Obj, i)).collect();
            let take_ix: HashMap<*const Obj, usize> =
                sounds.iter().flat_map(|s| takes_of(&ix, s).into_iter().enumerate().map(|(k, t)| (t as *const Obj, k))).collect();
            group.channel = group_channel(&ix, g, strip, &targets);
            // Maschine restores each sound's routing (and some switches) from the
            // group's sound configs, which win over the sound's own values.
            let configs: Vec<&Obj> = match g.get("sound_configs") {
                Some(Value::Obj(v)) => match v.get("items") {
                    Some(Value::List(items)) => items.iter().filter_map(|i| ix.obj(i)).collect(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            group.sounds = sounds.iter().enumerate().map(|(i, s)| sound(&ix, s, configs.get(i).copied(), &targets)).collect();
            let mut params = ModParams::new();
            channel_params(&ix, g, None, &mut params);
            for (i, s) in sounds.iter().enumerate() {
                channel_params(&ix, s, Some(i), &mut params);
            }
            group.modulators = modulators_of(&ix, g, &params);
            for (snd, s) in group.sounds.iter_mut().zip(&sounds) {
                snd.modulators = modulators_of(&ix, s, &params);
            }
            let mut ga = GroupAddresses {
                group: path_of(g),
                strip: path_of(strip),
                sounds: sounds.iter().map(|s| path_of(s)).collect(),
                patterns: Vec::new(),
                clip_patterns: Vec::new(),
            };
            for (pi, pat) in ix.elements(g.get("patterns_by_id")).into_iter().enumerate() {
                pattern_ix.insert(pat as *const Obj, (gi, pi));
                group.patterns.push(pattern(&ix, pat, &sound_ix, &take_ix, &params, &mut p.warnings));
                ga.patterns.push(path_of(pat));
            }
            // Clips: their patterns, and the clip events that place them.
            let clip_pats = ix.elements(g.get("patterns"));
            let clip_ix: HashMap<*const Obj, usize> = clip_pats.iter().enumerate().map(|(i, c)| (*c as *const Obj, i)).collect();
            for pat in &clip_pats {
                group.clip_patterns.push(pattern(&ix, pat, &sound_ix, &take_ix, &params, &mut p.warnings));
                ga.clip_patterns.push(path_of(pat));
            }
            if let Some(seq) = g.get("sequence").and_then(|v| ix.obj(v)) {
                for (position, ev) in timed_events(&ix, seq) {
                    let Some(&pattern) = ev.get("pattern").and_then(|v| ix.obj(v)).and_then(|c| clip_ix.get(&(c as *const Obj))) else {
                        continue;
                    };
                    let length = ev.find("length").and_then(Value::as_int).unwrap_or(0);
                    group.clips.push(Clip { position, length, pattern });
                }
            }
            group.clips.sort_by_key(|c| c.position);
            p.groups.push(group);
            addr.groups.push(ga);
        }

        let scenes = ix.elements(song.get("scenes"));
        let scene_ix: HashMap<*const Obj, usize> = scenes.iter().enumerate().map(|(i, s)| (*s as *const Obj, i)).collect();
        addr.scenes = scenes.iter().map(|s| path_of(s)).collect();
        for s in &scenes {
            let mut scene = Scene { name: param_str(s, "name"), color: color(s), ..Default::default() };
            for entry in map_entries(s.get("patterns")) {
                let (Some(gv), Some(rv)) = (entry.get("first"), entry.get("second")) else { continue };
                let Some(pref) = ix.obj(rv) else { continue };
                let Some(pat) = pref.get("pattern").and_then(|v| ix.obj(v)) else { continue }; // empty slot
                let group = ix.obj(gv).and_then(|g| group_ix.get(&(g as *const Obj)));
                match (group, pattern_ix.get(&(pat as *const Obj))) {
                    (Some(_), Some(&(pg, pi))) => scene.patterns.push((pg, pi)),
                    _ => p.warnings.push(format!("scene {:?}: unresolved pattern reference", scene.name)),
                }
            }
            scene.patterns.sort_unstable();
            p.scenes.push(scene);
        }

        for s in ix.elements(song.get("sections")) {
            addr.sections.push(path_of(s));
            let scene = s.get("scene").and_then(|v| ix.obj(v)).and_then(|sc| scene_ix.get(&(sc as *const Obj)).copied());
            let mut length = param_i64(s, "length").unwrap_or(0);
            // Auto-length sections last as long as their scene's longest pattern.
            let auto = param_i64(s, "auto_length").unwrap_or(0) != 0 || length <= 0;
            if auto {
                length = scene.map_or(0, |i| p.scene_length(i));
            }
            p.sections.push(Section { scene, length, auto });
        }
        (p, addr.resolve(r))
    }

    /// Length of a scene: its longest pattern, or one bar if it has none.
    pub fn scene_length(&self, scene: usize) -> i64 {
        let longest = self.scenes[scene].patterns.iter().map(|&(g, pi)| self.groups[g].patterns[pi].length).max();
        longest.filter(|&l| l > 0).unwrap_or_else(|| self.bar_length())
    }

    /// Ticks per bar.
    pub fn bar_length(&self) -> i64 {
        PPQ * 4 * self.numerator / self.denominator.max(1)
    }

    /// Tick at which section `i` starts in the arrangement.
    pub fn section_start(&self, i: usize) -> i64 {
        self.sections.iter().take(i).map(|s| s.length).sum()
    }

    /// The section playing at `tick`.
    pub fn section_at(&self, tick: i64) -> Option<usize> {
        let mut end = 0;
        self.sections.iter().position(|s| {
            end += s.length;
            tick < end
        })
    }

    /// Length of the arrangement in ticks.
    pub fn arrangement_length(&self) -> i64 {
        self.sections.iter().map(|s| s.length).sum()
    }
}

/// Groups and sounds by object, for routing values that point at them.
#[derive(Default)]
struct Targets {
    groups: HashMap<*const Obj, usize>,
    sounds: HashMap<*const Obj, (usize, usize)>,
}

fn int_field(o: &Obj, k: &str) -> Option<i64> {
    o.get(k).and_then(Value::as_int)
}

/// An `AudioOutputParameterValue`.
fn audio_dest(ix: &Index, v: &Obj, t: &Targets) -> AudioDest {
    let target = |k: &str| v.get(k).and_then(|x| ix.obj(x)).map(|o| o as *const Obj);
    match int_field(v, "type").unwrap_or(0) {
        1 => AudioDest::Parent,
        2 => AudioDest::Master,
        3 => AudioDest::Ext(int_field(v, "index").unwrap_or(0).max(0)),
        4 => target("group").and_then(|g| t.groups.get(&g)).map_or(AudioDest::None, |&g| AudioDest::Group(g)),
        5 => target("sound").and_then(|s| t.sounds.get(&s)).map_or(AudioDest::None, |&(g, s)| AudioDest::Sound(g, s)),
        _ => AudioDest::None,
    }
}

/// An `AudioInputParameterValue`.
fn audio_source(ix: &Index, v: &Obj, t: &Targets) -> AudioSource {
    let target = |k: &str| v.get(k).and_then(|x| ix.obj(x)).map(|o| o as *const Obj);
    match int_field(v, "type").unwrap_or(0) {
        1 => AudioSource::Ext(int_field(v, "index").unwrap_or(0).max(0)),
        2 => target("sound").and_then(|s| t.sounds.get(&s)).map_or(AudioSource::None, |&(g, s)| AudioSource::Sound(g, s)),
        3 => AudioSource::Master,
        4 => target("group").and_then(|g| t.groups.get(&g)).map_or(AudioSource::None, |&g| AudioSource::Group(g)),
        _ => AudioSource::None,
    }
}

/// A `map<slot, SideChainInputParameterValue>`: the sources that are set.
fn sidechain_map(ix: &Index, v: Option<&Value>, t: &Targets) -> Vec<(usize, ScSource)> {
    let Some(Value::Obj(m)) = v else { return Vec::new() };
    let Some(Value::List(items)) = m.get("items") else { return Vec::new() };
    let mut out = Vec::new();
    for item in items {
        let Some(pair) = ix.obj(item) else { continue };
        let Some(slot) = pair.get("first").and_then(Value::as_int) else { continue };
        let Some(val) = pair.get("second").and_then(|x| ix.obj(x)) else { continue };
        let target = |k: &str| val.get(k).and_then(|x| ix.obj(x)).map(|o| o as *const Obj);
        let src = match int_field(val, "type").unwrap_or(0) {
            1 => target("group").and_then(|g| t.groups.get(&g)).map(|&g| ScSource::Group(g)),
            2 => target("sound").and_then(|s| t.sounds.get(&s)).map(|&(g, s)| ScSource::Sound(g, s)),
            _ => None,
        };
        if let Some(src) = src {
            out.push((slot.max(0) as usize, src));
        }
    }
    out
}

/// An `EventOutputParameterValue`.
fn midi_dest(ix: &Index, v: &Obj, t: &Targets) -> MidiDest {
    match int_field(v, "type").unwrap_or(0) {
        1 => match v.get("port") {
            Some(Value::Str(p)) if p != "None" && !p.is_empty() => MidiDest::Port(p.clone()),
            _ => MidiDest::None,
        },
        2 => v
            .get("sound")
            .and_then(|x| ix.obj(x))
            .and_then(|o| t.sounds.get(&(o as *const Obj)))
            .map_or(MidiDest::None, |&(_, s)| MidiDest::Sound(s)),
        _ => MidiDest::None,
    }
}

fn group_channel(ix: &Index, g: &Obj, strip: &Obj, t: &Targets) -> GroupChannel {
    let dest = |k: &str, default: AudioDest| param(strip, k).and_then(|v| ix.obj(v)).map_or(default, |v| audio_dest(ix, v, t));
    GroupChannel {
        key_mode: param_i64(g, "key_mode").unwrap_or(0),
        midi_source: match param(g, "event_input") {
            Some(Value::Str(s)) => s.clone(),
            _ => "None".into(),
        },
        midi_channel: param_i64(g, "midi_channel").unwrap_or(0),
        start_note: param_i64(g, "start_note").unwrap_or(36),
        midi_thru: param_i64(g, "midi_thru").unwrap_or(0) != 0,
        dest: dest("audio_output", AudioDest::Parent),
        aux_dest: [dest("aux1_output", AudioDest::None), dest("aux2_output", AudioDest::None)],
        aux_pre: [param_i64(strip, "aux1_order").unwrap_or(0) == 1, param_i64(strip, "aux2_order").unwrap_or(0) == 1],
        midi_batch: param_i64(g, "midi_batch_export").unwrap_or(0),
    }
}

fn sound(ix: &Index, s: &Obj, config: Option<&Obj>, t: &Targets) -> Sound {
    let mut snd = sound_own(ix, s);
    let c = &mut snd.channel;
    c.midi_channel = param_i64(s, "midi_channel").unwrap_or(0);
    c.midi_thru = param_i64(s, "midi_thru").unwrap_or(1) != 0;
    c.input_gain = param_f64(s, "input_gain").unwrap_or(0.75);
    c.audio_mute = param_i64(s, "audio_mute").unwrap_or(0) != 0;
    c.aux_pre = [param_i64(s, "aux1_order").unwrap_or(0) == 1, param_i64(s, "aux2_order").unwrap_or(0) == 1];
    c.midi_out_channel = param_i64(s, "event_output_channel").unwrap_or(0);
    c.transpose = param_i64(s, "transpose").unwrap_or(0);
    c.base_key = param_i64(s, "base_key").unwrap_or(60);
    for (field, v) in [
        ("choke_group", &mut c.choke_group),
        ("choke_mode", &mut c.choke_mode),
        ("link_group", &mut c.link_group),
        ("link_mode", &mut c.link_mode),
    ] {
        *v = param_i64(s, field).unwrap_or(0);
    }
    c.midi_source = "Default".into();
    c.dest = AudioDest::Parent;
    let Some(cfg) = config else { return snd };
    let int = |k: &str| int_field(cfg, k);
    let float = |k: &str| match cfg.get(k) {
        Some(Value::Float(f)) => Some(*f),
        _ => None,
    };
    let obj = |k: &str| cfg.get(k).and_then(|v| ix.obj(v));
    if let Some(Value::Str(m)) = cfg.get("midi_source") {
        c.midi_source = m.clone();
    }
    c.midi_channel = int("midi_channel").unwrap_or(c.midi_channel);
    c.midi_thru = int("midi_thru").map_or(c.midi_thru, |v| v != 0);
    c.audio_in = obj("audio_input").map_or(AudioSource::None, |v| audio_source(ix, v, t));
    c.input_gain = float("input_gain").unwrap_or(c.input_gain);
    c.dest = obj("audio_output").map_or(AudioDest::Parent, |v| audio_dest(ix, v, t));
    c.audio_mute = int("audio_mute").map_or(c.audio_mute, |v| v != 0);
    c.aux_dest = [
        obj("aux1_output").map_or(AudioDest::None, |v| audio_dest(ix, v, t)),
        obj("aux2_output").map_or(AudioDest::None, |v| audio_dest(ix, v, t)),
    ];
    c.aux_pre = [int("aux1_order").map_or(c.aux_pre[0], |v| v == 1), int("aux2_order").map_or(c.aux_pre[1], |v| v == 1)];
    c.midi_out = obj("event_output").map_or(MidiDest::None, |v| midi_dest(ix, v, t));
    c.midi_out_channel = int("midi_out_channel").unwrap_or(c.midi_out_channel);
    c.transpose = int("transpose").unwrap_or(c.transpose);
    for (field, v) in [
        ("choke_group", &mut c.choke_group),
        ("choke_mode", &mut c.choke_mode),
        ("link_group", &mut c.link_group),
        ("link_mode", &mut c.link_mode),
    ] {
        *v = int(field).unwrap_or(*v);
    }
    c.sidechain = sidechain_map(ix, cfg.get("sidechain_inputs"), t);
    snd.muted = int("mute").map_or(snd.muted, |v| v != 0);
    snd.cue = int("cue_enabled").map_or(snd.cue, |v| v != 0);
    snd.aux = [float("aux1_level").unwrap_or(snd.aux[0]), float("aux2_level").unwrap_or(snd.aux[1])];
    snd
}

fn sound_own(ix: &Index, s: &Obj) -> Sound {
    let modules = chain_modules(ix, s);
    let slots = chain_slots(ix, s);
    let (lanes, lane) = lanes_of(ix, s);
    Sound {
        instrument: modules.first().map_or(String::new(), |m| m.type_name.rsplit("::").next().unwrap_or("").to_string()),
        name: param_str(s, "name"),
        color: color(s),
        level: param_f64(s, "level").unwrap_or(0.75),
        pan: param_f64(s, "pan").unwrap_or(0.0),
        muted: param_i64(s, "mute").unwrap_or(0) != 0,
        cue: param_i64(s, "cue_enabled").unwrap_or(0) != 0,
        aux: [param_f64(s, "aux1_level").unwrap_or(0.75), param_f64(s, "aux2_level").unwrap_or(0.75)],
        // (An effect plug-in first is an effect: the sound has no instrument.)
        plugin: modules
            .first()
            .filter(|m| m.type_name == "NI::MASCHINE::DATA::PluginHost")
            .map(|m| plugin_ref(m))
            .filter(|p| !(p.id.contains(":FX:") || p.id.contains(":Fx:"))),
        audio: modules.iter().find(|m| m.type_name == AUDIO_MODULE).map(|m| audio_loop(ix, m)),
        sampler: modules.into_iter().find(|m| m.type_name == "NI::MASCOMMON::DATA::Sampler").map(|m| sampler(ix, m)),
        swing: swing_of(s),
        slots,
        lanes: lanes.iter().map(|&(_, l)| l).collect(),
        lane,
        channel: SoundChannel::default(),
        modulators: Vec::new(),
        pad_touch: pad_touch::PadTouch::from_values(pad_touch::FIELDS.map(|f| s.get(f).and_then(Value::as_int))),
        mod_routes: match s.get(mod_matrix::FIELD) {
            Some(Value::Str(t)) => mod_matrix::from_text(t),
            _ => Vec::new(),
        },
        probability_locked: s.get(PROB_LOCK).and_then(Value::as_int).is_some_and(|v| v != 0),
    }
}

/// The sound's `midi_automators` (each with its object, to find the current one).
fn lanes_of<'a>(ix: &Index<'a>, s: &'a Obj) -> (Vec<(&'a Obj, LaneRef)>, Option<usize>) {
    let Some(Value::Obj(v)) = s.find("midi_automators") else { return (Vec::new(), None) };
    let items = match v.get("objects").and_then(|o| ix.obj(o)).and_then(|o| o.get("items")) {
        Some(Value::List(items)) => items.iter().filter_map(|i| ix.obj(i)).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let lanes: Vec<(&Obj, LaneRef)> = items
        .into_iter()
        .map(|a| {
            let int = |k: &str| match a.get(k) {
                Some(Value::Obj(p)) => p.find("value").and_then(Value::as_int).unwrap_or(0),
                _ => 0,
            };
            (a, LaneRef { kind: int("mode"), number: int("controller") })
        })
        .collect();
    let current = v.get("current").and_then(|c| ix.obj(c));
    let lane = current.and_then(|c| lanes.iter().position(|(a, _)| std::ptr::eq(*a, c)));
    (lanes, lane)
}

fn plugin_ref(m: &Obj) -> PluginRef {
    let text = |k: &str| match m.find(k) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let chunk = match m.find("chunk") {
        Some(Value::Obj(c)) => match c.find("size_data") {
            Some(Value::Bytes(b)) => b.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    PluginRef { id: text("plugin_id"), name: text("plugin_name"), chunk, pages: plugin_pages(m) }
}

/// A `PluginHost`'s saved parameter pages (`parameters.unknown_48`).
fn plugin_pages(m: &Obj) -> Vec<PluginPage> {
    let Some(Value::Obj(params)) = m.get("parameters") else { return Vec::new() };
    let Some(Value::Obj(ov)) = params.get("unknown_48") else { return Vec::new() };
    let Some(Value::Obj(objs)) = ov.get("objects") else { return Vec::new() };
    let Some(Value::List(items)) = objs.get("items") else { return Vec::new() };
    let str_of = |v: Option<&Value>| match v {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    };
    items
        .iter()
        .filter_map(|it| match it {
            Value::Obj(ptr) => match ptr.get("px") {
                Some(Value::Obj(page)) => Some(page),
                _ => None,
            },
            _ => None,
        })
        .map(|page| {
            let name = match page.find("name") {
                Some(Value::Obj(n)) => str_of(n.find("value")),
                _ => String::new(),
            };
            let knobs = match page.get("parameters") {
                Some(Value::Obj(arr)) => arr
                    .fields
                    .iter()
                    .filter(|(n, _)| n == "item")
                    .filter_map(|(_, v)| match v {
                        Value::Obj(ptr) => match ptr.get("px") {
                            Some(Value::Obj(p)) => Some(PluginKnob {
                                id: p.get("index").and_then(Value::as_int).unwrap_or(0) as u32,
                                name: str_of(p.get("custom_name")),
                                section: str_of(p.get("section_name")),
                            }),
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            PluginPage { name, params: knobs }
        })
        .collect()
}

/// Every slot of an object's plug-in chain.
/// The numeric parameters directly in a module object (not its base classes), by
/// field name: `value` of each `MaschineNumericParameter` / `EnumParameter`.
fn module_params(m: &Obj) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> =
        m.fields.iter().filter(|(n, _)| !n.starts_with('<')).filter_map(|(n, _)| param_f64(m, n).map(|v| (n.clone(), v))).collect();
    // The Side-Chain Input page's settings: `side_chain_filter/gain`, …
    if let Some(Value::Obj(sc)) = m.get("side_chain_filter") {
        for (n, _) in &sc.fields {
            if let Some(v) = param_f64(sc, n) {
                out.push((format!("side_chain_filter/{n}"), v));
            }
        }
    }
    out
}

fn chain_slots(ix: &Index, o: &Obj) -> Vec<SlotInfo> {
    let Some(Value::Obj(chain)) = o.find("chain") else { return Vec::new() };
    ix.elements(chain.get("slots"))
        .into_iter()
        .map(|slot| {
            let m = slot.get("module").and_then(|m| ix.obj(m));
            let module = m.map_or(String::new(), |m| m.type_name.rsplit("::").next().unwrap_or("").to_string());
            let name = match m.and_then(|m| m.find("plugin_name")) {
                Some(Value::Str(n)) if !n.is_empty() => n.clone(),
                _ => module_label(&module).to_string(),
            };
            let plugin = m.filter(|m| m.type_name == "NI::MASCHINE::DATA::PluginHost").map(plugin_ref);
            // An internal module's settings: its numeric parameters, by field.
            let params = match m {
                Some(m) if plugin.is_none() && module != "Sampler" => module_params(m),
                _ => Vec::new(),
            };
            SlotInfo { module, name, active: param_i64(slot, "active").unwrap_or(1) != 0, plugin, params }
        })
        .collect()
}

/// How Maschine names its own modules in a slot.
impl SlotInfo {
    /// A sound source (an instrument: the Sampler, Audio, a drum synth or an
    /// instrument plug-in), which Maschine keeps in a chain's first slot; else an
    /// effect.
    pub fn is_instrument(&self) -> bool {
        const SOURCES: [&str; 10] = ["Sampler", "AudioModule", "Kick", "Snare", "Hihat", "Tom", "Perc", "Cymbal", "BassSynth", "PolySynth"];
        match &self.plugin {
            Some(p) => !(p.id.contains(":FX:") || p.id.contains(":Fx:")),
            None => SOURCES.contains(&self.module.as_str()),
        }
    }
}

/// The side-chain input's filter at its defaults (no filter; Gain 1.0, as Maschine makes it).
const SIDE_CHAIN_DEFAULTS: [(&str, f64); 4] = [
    ("side_chain_filter/gain", 1.0),
    ("side_chain_filter/filter", 0.0),
    ("side_chain_filter/center_freq", 0.5),
    ("side_chain_filter/width", 0.5),
];

/// Maschine's internal instruments (docs/synth-*.md), in the "+" menu's order (a new
/// one's parameters come from `deckard_engine::synth::defaults`).
pub const INTERNAL_INSTRUMENTS: [&str; 8] = ["Kick", "Snare", "Hihat", "Tom", "Perc", "Cymbal", "BassSynth", "PolySynth"];

/// Stutter's quantized loop lengths, longest first (the strip's start): name, quarter
/// notes.
pub const STUTTER_LENGTHS: [(&str, f64); 6] = [("1/2", 2.0), ("1/4", 1.0), ("1/8", 0.5), ("1/16", 0.25), ("1/32", 0.125), ("1/64", 0.0625)];

/// The step of [`STUTTER_LENGTHS`] a Length position (0..1) picks, on Maschine's curve
/// (docs/fx-perform-stutter.md §2): the steps change at about 0.078, 0.245, 0.427, 0.630
/// and 0.865.
pub fn stutter_length_step(x: f64) -> usize {
    let x = x as f32;
    (((-x.abs()).mul_add(0.3, 1.3) * x * 5.0 + 0.5) as i32).clamp(0, 5) as usize
}

/// Stutter's Length without Quantize (0..1), in seconds at `bpm`: relative to the tempo,
/// about 707 ms down to 22 ms at 120 BPM.
pub fn stutter_seconds(x: f64, bpm: f64) -> f64 {
    let x = x as f32;
    let f = ((-x.abs()).mul_add(-0.3, 0.7) * x * 5.0 + 0.5).exp2();
    f64::from(240.0 / ((f + f) * bpm as f32))
}

/// Stutter's Length as Maschine shows it: the note value with Quantize ("1/16"); without,
/// the loop's time at `bpm` ("707.1 ms", "1.4 s").
pub fn stutter_length_text(x: f64, quantize: bool, bpm: f64) -> String {
    if quantize {
        return STUTTER_LENGTHS[stutter_length_step(x)].0.to_string();
    }
    let ms = (stutter_seconds(x, bpm) as f32 * 1000.0).abs();
    if ms < 0.005 {
        "0.00 ms".into()
    } else if ms < 1.0 {
        format!("{ms:.2} ms")
    } else if ms < 1000.0 {
        format!("{ms:.1} ms")
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
}

/// The last effect of each group of Maschine's "+" › Internal menu (a separator follows).
pub const EFFECT_GROUP_ENDS: [&str; 5] = ["Maximizer", "Filter", "Phaser", "PlateReverb", "Resochord"];

/// Maschine's internal effects, in the "+" menu's order and groups ([`EFFECT_GROUP_ENDS`]):
/// (class, the parameters of a new one (stored values, by field name; docs/fx-*.md)).
/// Maschine's Perform FX (a group's first module, docs/superpowers/plans/
/// 2026-10-22-phase5-perform-fx.md): its parameters at Maschine 3's defaults.
pub const PERFORM_FX: (&str, &[(&str, f64)]) = (
    "PerformanceFX",
    &[
        ("mode", 0.0),
        ("touchstrip_pos", 0.5),
        ("touchstrip_on_off", 0.0),
        ("filter_type", 0.0),
        ("filter_min_freq", 0.0),
        ("filter_max_freq", 1.0),
        ("filter_resonance", 1.0 / 3.0),
        ("filter_saturation", 0.3),
        ("filter_out_level", 0.74),
        ("flanger_decay", 0.65),
        ("flanger_stereo", 1.0),
        ("flanger_tone", 0.0),
        ("flanger_saturation", 0.7),
        ("flanger_wet_level", 0.45),
        ("flanger_polarity", 0.0),
        ("ring_spread", 0.3),
        ("ring_rev_mix", 0.5),
        ("ring_rev_time", 0.1),
        ("ring_mix_mode", 0.0),
        ("ring_dry_wet", 0.35),
        ("ring_glide", 0.0),
        ("burst_time", 0.5),
        ("burst_feedback", 0.95),
        ("burst_time_mode", 1.0),
        ("burst_ts_assign", 0.0),
        ("burst_filter", 0.35),
        ("burst_feed", 0.3),
        ("burst_mix_mode", 0.0),
        ("burst_wet_level", 0.6),
        ("reso_resonance", 0.45),
        ("reso_feedback", 0.75),
        ("reso_time", 0.25),
        ("reso_saturation", 0.5),
        ("reso_pump", 0.5),
        ("reso_wet_level", 0.35),
        ("reso_mix_mode", 0.0),
        ("stutter_ts_assign", 0.0),
        ("stutter_length", 0.4),
        ("stutter_pitch", 0.5),
        ("stutter_direction", 2.0),
        ("stutter_quantize", 1.0),
        ("stutter_gate", 1.0),
        ("stutter_retrigger", 1.0),
        ("scratcher_delay_time", 0.55),
        ("scratcher_feedback", 0.7),
        ("scratcher_freq_shift", 0.5),
        ("scratcher_wet_level", 0.5),
        ("tremolo_trem_vibr", 0.3),
        ("tremolo_vibr_mode", 0.0),
        ("tremolo_attack_release", 0.5),
        ("tremolo_stereo", 0.4),
        ("tremolo_depth_min", 0.1),
        ("tremolo_depth_max", 0.4),
        ("tremolo_rate_min", 0.6),
        ("tremolo_rate_max", 0.4),
    ],
);

pub const INTERNAL_EFFECTS: &[(&str, &[(&str, f64)])] = &[
    (
        "Compressor",
        &[
            ("mode", 0.0),
            ("threshold", 1.0 / 6.0),
            ("knee", 0.5),
            ("amount", 0.75),
            ("attack", 0.314_980_262),
            ("release", 0.5),
            ("gain", 0.916_666_7),
            SIDE_CHAIN_DEFAULTS[0],
            SIDE_CHAIN_DEFAULTS[1],
            SIDE_CHAIN_DEFAULTS[2],
            SIDE_CHAIN_DEFAULTS[3],
        ],
    ),
    (
        "Gate",
        &[
            ("threshold", 0.562_341_3),
            ("attack", 0.0),
            ("hold", 0.5),
            ("release", 0.25),
            ("mix", 1.0),
            SIDE_CHAIN_DEFAULTS[0],
            SIDE_CHAIN_DEFAULTS[1],
            SIDE_CHAIN_DEFAULTS[2],
            SIDE_CHAIN_DEFAULTS[3],
        ],
    ),
    ("TransientDesigner", &[("input_gain", 0.0), ("attack", 0.0), ("sustain", 0.0), ("limit", 1.0)]),
    (
        "Limiter",
        &[
            ("mode", 1.0),
            ("threshold", 0.0),
            ("release", 0.0),
            ("ceiling", 1.0),
            SIDE_CHAIN_DEFAULTS[0],
            SIDE_CHAIN_DEFAULTS[1],
            SIDE_CHAIN_DEFAULTS[2],
            SIDE_CHAIN_DEFAULTS[3],
        ],
    ),
    (
        "Maximizer",
        &[
            ("amount", 0.5),
            ("curve", 0.5),
            ("turbo", 0.0),
            SIDE_CHAIN_DEFAULTS[0],
            SIDE_CHAIN_DEFAULTS[1],
            SIDE_CHAIN_DEFAULTS[2],
            SIDE_CHAIN_DEFAULTS[3],
        ],
    ),
    (
        "EQ",
        &[
            ("low_freq", 0.420_214),
            ("low_gain", 0.0),
            ("low_mid_freq", 0.460_128),
            ("low_mid_gain", 0.0),
            ("low_mid_width", 0.5),
            ("high_mid_freq", 0.650_41),
            ("high_mid_gain", 0.0),
            ("high_mid_width", 0.5),
            ("high_freq", 0.660_119),
            ("high_gain", 0.0),
            ("output_gain", 0.0),
        ],
    ),
    (
        "Filter",
        &[
            ("mode", 0.0),
            ("cutoff", 0.549_053_7),
            ("resonance", 0.5),
            ("amount", 0.0),
            ("source", 0.0),
            ("speed", 0.5),
            ("lfo_shape", 0.5),
            ("phase", 0.0),
            ("decay", 0.5),
            ("smooth", 0.5),
            ("env_shape", 0.0),
            SIDE_CHAIN_DEFAULTS[0],
            SIDE_CHAIN_DEFAULTS[1],
            SIDE_CHAIN_DEFAULTS[2],
            SIDE_CHAIN_DEFAULTS[3],
        ],
    ),
    ("Chorus", &[("rate", 0.5), ("amount", 0.5), ("mix", 0.5)]),
    (
        "Flanger",
        &[
            ("frequency", 0.5),
            ("source", 0.0),
            ("amount", 0.75),
            ("speed", 0.5),
            ("shape", 0.5),
            ("stereo", 0.5),
            ("feedback", 0.5),
            ("invert", 0.0),
            ("mix", 0.5),
        ],
    ),
    ("FM", &[("rate", 0.25), ("split", 0.75), ("contour", 0.75), ("amount", 0.75)]),
    ("FreqShifter", &[("coarse", 0.5), ("fine", 0.0), ("feedback", 0.75), ("stereo", 0.75), ("invert", 1.0), ("mix", 0.75)]),
    (
        "Phaser",
        &[
            ("frequency", 0.5),
            ("amount", 0.5),
            ("source", 1.0),
            ("speed", 0.5),
            ("shape", 0.5),
            ("stereo", 0.5),
            ("feedback", 0.5),
            ("eight_pole", 0.0),
            ("mix", 0.5),
        ],
    ),
    ("Iceverb", &[("color", 0.75), ("ice", 0.75), ("size", 0.5), ("mix", 0.5)]),
    ("Metaverb", &[("size", 0.5), ("low", 0.5), ("high", 0.5), ("pan", 0.0), ("mix", 0.5)]),
    ("Reflex", &[("color", 0.5), ("smooth", 0.75), ("size", 0.25), ("mix", 0.5)]),
    ("Reverb", &[("room", 0.0), ("size", 0.5), ("low", 0.5), ("high", 0.5), ("pan", 0.0), ("stereo", 0.0), ("mix", 0.5), ("freeze", 0.0)]),
    (
        "PlateReverb",
        &[
            ("mode", 0.0),
            ("wet_gain", 3.5),
            ("mix", 0.5),
            ("room_rev_time", 0.2),
            ("room_size", 0.2),
            ("room_damp", 0.5),
            ("room_mod", 0.2),
            ("room_diff", 0.8),
            ("room_pre_delay", 0.0),
            ("room_high_cut", 0.0),
            ("room_low_shelf", 0.0),
            ("hall_rev_time", 0.4),
            ("hall_size", 0.1),
            ("hall_damp", 0.5),
            ("hall_mod", 0.4),
            ("hall_diff", 0.9),
            ("hall_pre_delay", 0.0),
            ("hall_high_cut", 0.0),
            ("hall_low_shelf", 0.0),
            ("plate_decay", 0.3),
            ("plate_low_shelf", 0.0),
            ("plate_high_damp", 0.0),
            ("plate_pre_delay", 0.0),
        ],
    ),
    (
        "BeatDelay",
        &[
            ("time", 3.0),
            ("offset", 0.0),
            ("feedback", 0.5),
            ("crossover", 0.25),
            ("color", 0.5),
            ("split", 0.25),
            ("stereo", 0.0),
            ("mix", 0.5),
            ("unit", 0.0),
        ],
    ),
    (
        "GrainDelay",
        &[
            ("pitch", 0.0),
            ("size", 0.25),
            ("jitter", 0.0),
            ("reverse", 0.0),
            ("space", 0.0),
            ("density", 0.25),
            ("modulation", 0.0),
            ("mix", 0.75),
            ("stereo", 0.75),
        ],
    ),
    ("GrainStretch", &[("on", 0.0), ("stretch", 0.0), ("loop_length", 8.0), ("pitch", 0.5), ("link", 0.0), ("size", 0.25), ("mix", 1.0)]),
    (
        "Resochord",
        &[
            ("style", 0.0),
            ("mode", 1.0),
            ("chord", 1.0),
            ("spread", 0.5),
            ("tune", 0.0),
            ("brightness", 0.75),
            ("feedback", 0.75),
            ("decay", 0.5),
            ("mix", 0.75),
        ],
    ),
    (
        "MullhollandDrive",
        &[
            ("mode", 1.0),
            ("analog_saturation", 0.25),
            ("analog_bass", 1.0),
            ("analog_tone", 1.0),
            ("analog_focus", 0.0),
            ("analog_definition", 0.5),
            ("analog_mix", 1.0),
            ("drive", 0.25),
            ("color", 0.25),
            ("feedback", 0.25),
            ("tone", 0.75),
            ("tone_mod", 0.0),
            ("gate", 1.0),
            ("release", 0.5),
            ("mix", 0.5),
        ],
    ),
    ("LoFi", &[("sample_rate", 0.129_756_79), ("bits", 0.185), ("smooth", 1.0), ("stereo", 1.0), ("mix", 1.0)]),
    (
        "Saturator",
        &[
            ("mode", 0.0),
            ("classic_input", 0.25),
            ("classic_drive", 0.5),
            ("classic_contour", 0.75),
            ("tape_input", 0.2),
            ("tape_drive", 0.34),
            ("tape_contour", 1.0),
            ("tube_drive", 0.25),
            ("tube_charge", 2.0),
            ("tube_bass", 0.8),
            ("tube_treble", 0.8),
            ("tube_gain", 0.0),
            ("tube_overload", 0.0),
            ("tube_eq_bypass", 0.0),
        ],
    ),
];

pub fn module_label(class: &str) -> &str {
    match class {
        "AudioModule" => "Audio",
        "Perc" => "Percussion",
        "Hihat" => "Hi-hat",
        "BassSynth" => "Bass Synth",
        "PolySynth" => "Poly Synth",
        "PlateReverb" => "Reverb",
        "Reverb" => "Legacy Reverb",
        "Iceverb" => "Ice",
        "TransientDesigner" => "Transient Master",
        "CabinetEmulation" => "Cabinet",
        "MullhollandDrive" => "Distortion",
        "LoFi" => "Lo-Fi",
        "BeatDelay" => "Beat Delay",
        "GrainDelay" => "Grain Delay",
        "GrainStretch" => "Grain Stretch",
        "FreqShifter" => "Freq Shifter",
        "PluginHost" => "Plug-in",
        "PerformanceFX" => "Perform FX",
        other => other,
    }
}

/// Modules of the active slots in an object's plug-in chain, in order.
fn chain_modules<'a>(ix: &Index<'a>, o: &'a Obj) -> Vec<&'a Obj> {
    let Some(Value::Obj(chain)) = o.find("chain") else { return Vec::new() };
    ix.elements(chain.get("slots"))
        .into_iter()
        .filter(|slot| param_i64(slot, "active").unwrap_or(1) != 0)
        .filter_map(|slot| slot.get("module").and_then(|m| ix.obj(m)))
        .collect()
}

fn sampler(ix: &Index, m: &Obj) -> Sampler {
    let mut params = BTreeMap::new();
    for (path, _) in SAMPLER_PARAMS {
        let (obj, field) = match path.split_once('/') {
            Some((o, f)) => match m.get(o) {
                Some(Value::Obj(o)) => (o, f),
                _ => continue,
            },
            None => (m, *path),
        };
        // Only the Sampler's own fields (and its bases'), never a zone's.
        if let Some(v) = obj.find_path(field).and_then(|fp| match obj.at(&fp) {
            Some(Value::Obj(p)) => p.find("value").and_then(|v| match v {
                Value::Float(f) => Some(*f),
                Value::Int(i) => Some(*i as f64),
                _ => None,
            }),
            _ => None,
        }) {
            params.insert(path.to_string(), v);
        }
    }
    let mut s = Sampler { zones: ix.elements(m.get("zones")).into_iter().map(zone).collect(), params, ..Sampler::default() };
    s.derive();
    s
}

const AUDIO_MODULE: &str = "NI::MASCOMMON::DATA::AudioModule";

fn audio_loop(ix: &Index, m: &Obj) -> AudioLoop {
    let takes = ix.elements(m.get("takes"));
    let current = match m.get("takes") {
        Some(Value::Obj(t)) => t.get("current").and_then(|c| ix.obj(c)),
        _ => None,
    };
    AudioLoop {
        focused: current.and_then(|c| takes.iter().position(|t| std::ptr::eq(*t, c))).unwrap_or(0),
        takes: takes.iter().map(|t| take(t)).collect(),
        mode: if param_i64(m, "mode").unwrap_or(0) == 1 { AudioMode::Gate } else { AudioMode::Loop },
        fade: param_f64(m, "fade").unwrap_or(0.1222),
        tune: param_f64(m, "tune").unwrap_or(0.0),
        pitchbend: param_f64(m, "pitchbend").unwrap_or(0.5),
        engine: match param_i64(m, "engine").unwrap_or(1) {
            0 => AudioEngine::Repitch,
            2 => AudioEngine::Formant,
            _ => AudioEngine::Stretch,
        },
    }
}

fn take(t: &Obj) -> Take {
    Take { sample: sample_ref(t), beats: param_f64(t, "length").unwrap_or(0.0), play_range: range(t, "play_range", (0, 0)) }
}

/// The takes of a sound's Audio module, as objects (what `AudioEvent`s point at).
fn takes_of<'a>(ix: &Index<'a>, s: &'a Obj) -> Vec<&'a Obj> {
    chain_modules(ix, s).into_iter().find(|m| m.type_name == AUDIO_MODULE).map_or_else(Vec::new, |m| ix.elements(m.get("takes")))
}

/// (low, high) of a `RangeParameter` field.
fn range(o: &Obj, field: &str, default: (i64, i64)) -> (i64, i64) {
    let Some(Value::Obj(r)) = o.find(field) else { return default };
    let end = |k: &str| match r.get(k) {
        Some(Value::Obj(p)) => p.find("value").and_then(Value::as_int),
        _ => None,
    };
    (end("low").unwrap_or(default.0), end("high").unwrap_or(default.1))
}

/// The sample a zone's or take's `file` names.
fn sample_ref(z: &Obj) -> SampleRef {
    let mut sample = SampleRef::default();
    if let Some(Value::Obj(f)) = z.get("file") {
        sample.kind = f.get("kind").and_then(Value::as_int).unwrap_or(0);
        if let Some(Value::Str(p)) = f.get("path") {
            sample.path = p.clone();
        }
        if let Some(Value::Obj(a)) = f.get("alias") {
            if let Some(Value::Str(r)) = a.get("root") {
                sample.library_root = r.clone();
            }
            if let Some(Value::Str(n)) = a.get("library") {
                sample.library = n.clone();
            }
        }
    }
    sample
}

fn zone(z: &Obj) -> Zone {
    let sample = sample_ref(z);
    Zone {
        sample,
        root_key: param_i64(z, "root_key").unwrap_or(60),
        key_range: range(z, "key_range", (0, 127)),
        velocity_range: range(z, "velocity_range", (0, 127)),
        gain: param_f64(z, "gain").unwrap_or(1.0),
        pan: param_f64(z, "pan").unwrap_or(0.0),
        tune: param_f64(z, "tune").unwrap_or(0.0),
        play_range: range(z, "play_range", (0, 0)),
        loop_range: range(z, "loop_range", (0, 0)),
        loop_active: param_i64(z, "loop_active").unwrap_or(0) != 0,
        x_fade: param_i64(z, "x_fade").unwrap_or(0),
        fade_in: param_i64(z, "attack").unwrap_or(0),
        fade_out: param_i64(z, "decay").unwrap_or(0),
    }
}

/// Entries (`pair` objects) of an `ObjectMap` field.
fn map_entries(container: Option<&Value>) -> Vec<&Obj> {
    let Some(Value::Obj(c)) = container else { return Vec::new() };
    let Some(Value::Obj(objects)) = c.get("objects") else { return Vec::new() };
    let Some(Value::List(items)) = objects.get("items") else { return Vec::new() };
    items.iter().filter_map(|v| if let Value::Obj(o) = v { Some(o) } else { None }).collect()
}

fn pattern(
    ix: &Index,
    pat: &Obj,
    sound_ix: &HashMap<*const Obj, usize>,
    take_ix: &HashMap<*const Obj, usize>,
    params: &ModParams,
    warnings: &mut Vec<String>,
) -> Pattern {
    let mut out = Pattern {
        name: param_str(pat, "name"),
        color: color(pat),
        start: param_i64(pat, "start").unwrap_or(0),
        length: param_i64(pat, "length").unwrap_or(0),
        notes: Vec::new(),
        audio: Vec::new(),
        midi: Vec::new(),
        mods: Vec::new(),
    };
    // The group's own points (its parameters'): the pattern's sequence.
    if let Some(seq) = pat.get("sequence").and_then(|v| ix.obj(v)) {
        for (position, ev) in timed_events(ix, seq) {
            out.mods.extend(mod_point(ix, ev, position, params));
        }
    }
    for entry in map_entries(pat.get("sound_sequences")) {
        let sound = entry.get("sound").and_then(|v| ix.obj(v)).and_then(|s| sound_ix.get(&(s as *const Obj)).copied());
        let seq = entry.get("reference").and_then(|v| ix.obj(v)).and_then(|r| r.get("sequence")).and_then(|v| ix.obj(v));
        // A null sequence means this sound has no notes in the pattern.
        let Some(seq) = seq else { continue };
        let Some(sound) = sound else {
            warnings.push(format!("pattern {:?}: sequence for an unknown sound", out.name));
            continue;
        };
        let Some(Value::Obj(events)) = seq.get("events") else { continue };
        let Some(Value::Obj(items)) = events.get("items") else { continue };
        for (_, v) in &items.fields {
            let Value::Obj(te) = v else { continue };
            if te.type_name != "NI::MASCOMMON::DATA::TimedEvent" {
                continue;
            }
            let Some(ev) = te.get("event").and_then(|e| ix.obj(e)) else { continue };
            let position = te.get("position").and_then(Value::as_int).unwrap_or(0);
            // The sound's Audio plug-in plays here (this take).
            if ev.type_name == "NI::MASCHINE::DATA::AudioEvent" {
                if let Some(&take) = ev.get("take").and_then(|t| ix.obj(t)).and_then(|t| take_ix.get(&(t as *const Obj))) {
                    out.audio.push(AudioCue { sound, take });
                }
                continue;
            }
            if ev.type_name == "NI::MASCHINE::DATA::MidiEvent" {
                let int = |k: &str| ev.get(k).and_then(Value::as_int).unwrap_or(0);
                let kind = match int("i14") {
                    4 => Some(MidiKind::Controller { number: int("i18").clamp(0, 127) as u8, value: int("i20").clamp(0, 127) as u8 }),
                    3 => Some(MidiKind::PitchBend(int("i20").clamp(0, 16383) as u16)),
                    6 => Some(MidiKind::ChannelPressure(int("i20").clamp(0, 127) as u8)),
                    _ => None,
                };
                if let Some(kind) = kind {
                    out.midi.push(MidiEvent { sound, position, kind });
                }
                continue;
            }
            if ev.type_name.contains("ModulationEvent") {
                out.mods.extend(mod_point(ix, ev, position, params));
                continue;
            }
            if !ev.type_name.ends_with("NoteEvent") {
                continue;
            }
            let int = |k: &str| ev.find(k).and_then(Value::as_int).unwrap_or(0);
            out.notes.push(Note {
                expr: te.get(expression::EXPR_FIELD).and_then(Value::as_int).map(|i| ExprId(i as u32)),
                sound,
                position: te.get("position").and_then(Value::as_int).unwrap_or(0),
                length: int("length"),
                pitch: int("note"),
                velocity: int("velocity"),
                // v0 of the probability policy only had a mute flag.
                probability: ev
                    .find("probability")
                    .and_then(Value::as_int)
                    .or_else(|| ev.find("muted").and_then(Value::as_int).map(|m| if m != 0 { 0 } else { 100 }))
                    .unwrap_or(100),
            });
        }
    }
    out.notes.sort_by_key(|n| (n.position, n.sound, n.pitch));
    out.midi.sort_by_key(|m| (m.position, m.sound));
    out.mods.sort_by(|a, b| (a.sound, a.slot, &a.field, a.position).cmp(&(b.sound, b.slot, &b.field, b.position)));
    out
}

/// A sequence's events: (position, event).
fn timed_events<'a>(ix: &Index<'a>, seq: &'a Obj) -> Vec<(i64, &'a Obj)> {
    let Some(Value::Obj(events)) = seq.get("events") else { return Vec::new() };
    let Some(Value::Obj(items)) = events.get("items") else { return Vec::new() };
    items
        .fields
        .iter()
        .filter_map(|(_, v)| match v {
            Value::Obj(te) if te.type_name == "NI::MASCOMMON::DATA::TimedEvent" => {
                Some((te.get("position").and_then(Value::as_int).unwrap_or(0), te.get("event").and_then(|e| ix.obj(e))?))
            }
            _ => None,
        })
        .collect()
}

/// The modulated parameters of a group's channels, by the parameter object: the
/// channel (sound index, `None` for the group) and the parameter.
pub(crate) type ModParams = HashMap<*const Obj, (Option<usize>, ModTarget)>;

/// A `ModulationEvent` as a point: v0 holds the value (`constant`), v1 a
/// `Modulation` whose `b[0]` is it.
fn mod_point(ix: &Index, ev: &Obj, position: i64, params: &ModParams) -> Option<ModPoint> {
    let param = ev.get("parameter").and_then(|v| ix.obj(v))?;
    let (sound, target) = params.get(&(param as *const Obj))?;
    let value = match ev.get("constant") {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f64,
        _ => match ev.find("modulation") {
            Some(Value::Obj(m)) => match m.get("b") {
                Some(Value::Obj(b)) => match b.get("v0") {
                    Some(Value::Float(f)) => *f,
                    _ => 0.0,
                },
                _ => 0.0,
            },
            _ => 0.0,
        },
    };
    Some(ModPoint { sound: *sound, slot: target.slot, field: target.field.clone(), position, value })
}

/// Whether `o` is a parameter (a modulation target's class).
fn is_parameter(o: &Obj) -> bool {
    o.type_name.starts_with("NI::MASCOMMON::DATA::MaschineNumericParameter") || o.type_name == "NI::MASCOMMON::DATA::EnumParameter"
}

/// The parameters of channel `ch` (a sound or group): its modules' (by slot; a
/// parameter in a module's part as `part/field`) and its own.
fn channel_params(ix: &Index, ch: &Obj, sound: Option<usize>, out: &mut ModParams) {
    fn walk(o: &Obj, prefix: &str, slot: Option<usize>, sound: Option<usize>, depth: usize, out: &mut ModParams) {
        for (n, v) in &o.fields {
            let Value::Obj(f) = v else { continue };
            if n.starts_with('<') {
                // A base class: its fields are the object's own.
                walk(f, prefix, slot, sound, depth, out);
            } else if is_parameter(f) {
                out.insert(f as *const Obj, (sound, ModTarget { slot, field: format!("{prefix}{n}") }));
            } else if depth > 0 && f.oid.is_none() {
                walk(f, &format!("{prefix}{n}/"), slot, sound, depth - 1, out);
            }
        }
    }
    if let Some(Value::Obj(chain)) = ch.find("chain") {
        for (i, slot) in ix.elements(chain.get("slots")).into_iter().enumerate() {
            if let Some(m) = slot.get("module").and_then(|m| ix.obj(m)) {
                walk(m, "", Some(i), sound, 2, out);
            }
        }
    }
    walk(ch, "", None, sound, 0, out);
}

/// A channel's modulated parameters (`modulators`), in order.
fn modulators_of(ix: &Index, ch: &Obj, params: &ModParams) -> Vec<ModTarget> {
    ix.elements(ch.find("modulators"))
        .into_iter()
        .filter_map(|m| m.get("target").and_then(|t| ix.obj(t)))
        .filter_map(|t| params.get(&(t as *const Obj)).map(|(_, target)| target.clone()))
        .collect()
}

#[cfg(test)]
mod swing_tests {
    use super::Swing;

    #[test]
    fn off_and_full_swing() {
        let off = Swing::default();
        assert_eq!(off.apply(1.25), 1.25);
        // 100 % at the default 1/8 cycle: the off-beat 16th (the middle of the
        // cycle) moves to 95 % of it.
        let full = Swing { amount: 100.0, ..Swing::default() };
        let cycle = 0.5; // a 1/8 note, in beats
        assert!((full.apply(0.25) - cycle * 0.95).abs() < 1e-9, "{}", full.apply(0.25));
        assert_eq!(full.apply(0.0), 0.0);
        assert_eq!(full.apply(0.5), 0.5, "cycle starts stay put");
        let inverted = Swing { invert: true, ..full };
        assert!(inverted.apply(0.25) < 0.25, "inverted swing plays early");
        let half = Swing { amount: 50.0, ..Swing::default() };
        assert!((half.apply(0.25) - cycle * (0.5 + 0.45 * 0.375)).abs() < 1e-9);
    }
}
