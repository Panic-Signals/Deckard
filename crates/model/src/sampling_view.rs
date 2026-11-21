//! What the controller's Sampling page shows of the app's sampler and recorder, and
//! what it asks of them: shared by the scripts route (`deckard-lua-host`) and DECKARD's
//! own layer (`deckard-surface`).

/// The focused zone's sample, for the Sampling page's EDIT, SLICE and ZONE screens.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SampleView {
    pub name: String,
    pub path: String,
    pub frames: usize,
    pub rate: u32,
    /// Peaks (min, max) per column: across the whole sample, and across `view`.
    pub overview: Vec<(f32, f32)>,
    pub detail: Vec<(f32, f32)>,
    /// The frames shown (start, end).
    pub view: (f64, f64),
    /// The zone's play range and loop (start, end; end exclusive), the selection.
    pub play: (usize, usize),
    pub looped: Option<(usize, usize)>,
    pub selection: Option<(usize, usize)>,
    /// Slice starts (frames), and the slice in focus (kept by the scripts).
    pub slices: Vec<usize>,
    /// The slices kept through re-rolls (Find, Random), by index.
    pub kept: Vec<usize>,
    pub focus_slice: Option<usize>,
    pub playhead: Option<usize>,
    /// The Sampler's zones (keys, velocities; inclusive) and the focused one.
    pub zones: Vec<((i64, i64), (i64, i64))>,
    pub focus_zone: usize,
    /// The sound's colour (palette index), if it has one.
    pub color: Option<u32>,
    /// An Audio plug-in's take (RECORD and EDIT only), not a Sampler zone.
    pub take: bool,
    /// The zone's stem lanes (a New Sampler's), in their order.
    pub lanes: Vec<LaneView>,
    /// A layered New Sampler's layers in their order (else none), the one focused, and
    /// its morph.
    pub layers: Vec<LayerView>,
    pub layer: usize,
    pub morph: Option<MorphView>,
}

/// A layer of the focused sound, for the controller's LAYERS screens: its sample's
/// name, whether it's heard (not muted, nor left out by another's solo), its level as
/// stored (0..1) and as shown, its window on the morph and its envelope (attack, decay,
/// sustain, release), and its sample's peaks.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LayerView {
    pub name: String,
    pub mute: bool,
    pub solo: bool,
    pub heard: bool,
    pub gain: f64,
    pub level: String,
    pub window: [f64; 4],
    pub envelope: [f64; 4],
    pub detail: Vec<(f32, f32)>,
}

/// The morph of a sound's layers: whether it runs, where it is now, and its shape
/// (round, else there and back) and fade (equal power, else linear).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct MorphView {
    pub on: bool,
    pub position: f64,
    pub round: bool,
    pub power: bool,
}

/// A stem lane of the focused zone, for the controller: which (0 Drums, 1 Vocals, 2
/// Bass, 3 Leads), whether it's heard (not muted, nor left out by another's solo), and
/// its peaks over the frames shown.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LaneView {
    pub lane: usize,
    pub name: String,
    pub mute: bool,
    pub solo: bool,
    pub heard: bool,
    pub detail: Vec<(f32, f32)>,
}

/// A stem separation running in the app: whether it's the focused zone's sample, and
/// how far it is (0..1).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct StemsRun {
    pub focused: bool,
    pub progress: f32,
}

/// What the Sampling page asks of the app (`NI.DATA.FocusSampleAccess`,
/// `TransactionSampleAccess`, `SamplerAccess`, …).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SamplingCall {
    /// An EDIT function: `SamplingHelper.EditFunc…` (1 Truncate … 11 Duplicate).
    Edit(usize),
    /// AUTO LOOP: find a loop in the focused zone (`ZoneAccess.findAndSetLoopPoints`).
    AutoLoop,
    /// STEMS: separate the focused zone's sample (`executeStemSeparation`), or stop the
    /// separation running (`cancelStemSeparation`).
    Stems,
    CancelStems,
    /// Show these frames (knobs 5 and 6).
    View(f64, f64),
    /// Split slice `.0` in two; remove its start (joining it to the one before); remove
    /// them all; move slice `.0`'s start to frame `.1`.
    SplitSlice(usize),
    RemoveSlice(usize),
    ClearSlices,
    /// A slice where the sample is playing (`true`: exactly there, not on its nearest
    /// 1/8 note).
    CatchSlice(bool),
    /// Keep slice `.0` through re-rolls, or no longer.
    KeepSlice(usize),
    /// Slice `.0` onto the group's next empty sound, over all its keys.
    KeyShift(usize),
    /// Random picks again.
    Reroll,
    MoveSlice(usize, usize),
    /// Make the slices zones of the focused sound.
    ApplySlices,
    /// Remove zone `.0`; focus zone `.0`.
    RemoveZone(usize),
    FocusZone(usize),
    /// Play frames (start, end) of the focused sample.
    Prehear(usize, usize),
    /// A New Sampler zone's tempo doubled or halved (true: doubled); the song's tempo
    /// set to it.
    ScaleBpm(bool),
    ToProject,
    /// Its Tune set so its root is the nearest note of the song's key.
    MatchKey,
    /// Its tempo and root heard again (the part selected, else the part it plays).
    Reanalyze,
    /// Stem lane `.0` (from 0, as [`LaneView::lane`]) muted or not; soloed or not.
    LaneMute(usize),
    LaneSolo(usize),
    /// Its root (`.1`) and/or tempo (`.2`) as stem lane `.0` alone has them.
    LaneHear(usize, bool, bool),
    /// Its warp markers: 0 put on its hits, 1 quantized, 2 cleared.
    Warp(usize),
    /// The sound's Sampler made a New Sampler, or back.
    Convert,
    /// Layer `.0` focused; muted or not; soloed or not; removed. The focused layer's
    /// level (a zone's gain, 0..1). Every layer's window spread evenly again.
    FocusLayer(usize),
    LayerMute(usize),
    LayerSolo(usize),
    RemoveLayer(usize),
    LayerLevel(f64),
    SpreadLayers,
}

/// A page of the Sampling page's knobs (up to four, knobs 5 and 6 zooming and scrolling;
/// up to six on LAYERS).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct KnobPage {
    pub name: String,
    pub knobs: Vec<SamplingKnob>,
}

/// One setting on a Sampling page knob.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SamplingKnob {
    /// What the app knows the setting by.
    pub key: String,
    pub name: String,
    /// Its value as shown ("Ext. Ster.", "-12.0 dB").
    pub text: String,
    /// The value (a choice's index), its range, how far a step moves it, and how much of
    /// a turn a step takes.
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub turn: f64,
}

/// What the RECORD screen asks of the app's recorder
/// (`NI.DATA.RecorderAccess.toggleRecording` / `requestCancelRecording`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordPress {
    /// START when idle, STOP while waiting or recording.
    Toggle,
    Cancel,
}

/// The app's recorder, for the Sampling page's RECORD screen.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct RecordState {
    /// Input levels (left, right; linear), falling back after peaks.
    pub levels: (f32, f32),
    /// `NI.DATA.MODE_DETECT` (0), SYNC, LOOP, AUTO.
    pub mode: i64,
    /// Detect's threshold, dB (−60 … 0).
    pub threshold: f64,
    pub waiting: bool,
    pub recording: bool,
}
