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
    pub focus_slice: Option<usize>,
    pub playhead: Option<usize>,
    /// The Sampler's zones (keys, velocities; inclusive) and the focused one.
    pub zones: Vec<((i64, i64), (i64, i64))>,
    pub focus_zone: usize,
    /// The sound's colour (palette index), if it has one.
    pub color: Option<u32>,
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
    MoveSlice(usize, usize),
    /// Make the slices zones of the focused sound.
    ApplySlices,
    /// Remove zone `.0`; focus zone `.0`.
    RemoveZone(usize),
    FocusZone(usize),
    /// Play frames (start, end) of the focused sample.
    Prehear(usize, usize),
}

/// A page of the Sampling page's knobs (up to four; knobs 5 and 6 zoom and scroll).
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
