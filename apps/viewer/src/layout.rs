//! Maschine's main window, drawn after the measurements in docs/ui-layout.md: sizes,
//! colours and the order of panels come from the app's skin style sheets; the drawing
//! is our own (no NI bitmaps or fonts).

use crate::remote::{AssignTab, RemoteAct};
use crate::{App, Playhead, modulation, sampler_ui};
use deckard_doc::Target;
use deckard_engine::Mode;
use deckard_model::{Color, LaneRef, MidiEvent, MidiKind, Note, PPQ, Project, SWING_CYCLES, Swing};
use eframe::egui::{self, Align2, Color32, FontId, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use mxprj::decode::Value;
use std::borrow::Cow;

mod assign;
pub(crate) mod channel;
pub(crate) mod expr_lane;
pub(crate) mod mod_routes;
pub use channel::{GrooveButton, Level, RouteSet};
pub mod browser;
mod lfo_panel;
mod mixer;
pub mod sampling;
pub mod save_dialog;
pub(crate) mod strip;
pub use mixer::{Channel, MixerSection, MixerSections};

// Surfaces (Definitions/Colors.txt).
pub const S00: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
const S05: Color32 = Color32::from_rgb(0x21, 0x21, 0x21);
const S01: Color32 = Color32::from_rgb(0x26, 0x26, 0x26);
const S02: Color32 = Color32::from_rgb(0x32, 0x32, 0x32);
const S03: Color32 = Color32::from_rgb(0x3C, 0x3C, 0x3C);
const FOCUS_ROW: Color32 = Color32::from_rgb(0x29, 0x29, 0x29);
const LANE: Color32 = Color32::from_rgb(0x12, 0x12, 0x12);
const PLAY_ON: Color32 = Color32::from_rgb(0xBD, 0xF1, 0x64);
const REC_ON: Color32 = Color32::from_rgb(0xE3, 0x42, 0x3F);

pub const CONTROLLER_WIDTH: f32 = 450.0;
const HEADER: f32 = 33.0;
const STRIP: f32 = 32.0;
const COLUMN: f32 = 163.0;
const ROW: f32 = 24.0;
const GAP: f32 = 2.0;
const IDEA_W: f32 = 99.0;
const TILE_H: f32 = 56.0;
const SCENE_ROW: f32 = 32.0;
const CHAIN_H: f32 = 140.0;
/// Height of the lane under the note area (velocity, probability or a MIDI controller).
const LANE_H: f32 = 145.0;
/// The Pattern Editor's zooming scroll bars: along the bottom (time) and the right
/// (pitch in the piano roll; a plain scroll bar for the drum grid's sound slots).
const ZOOM_BAR: f32 = 12.0;
/// Piano roll: keyboard width and pitch row height.
const KEYS_W: f32 = 46.0;
const PIANO_ROW: f32 = 12.0;

/// Maschine's 16 colours plus white (Colors.qml / palette 1).
pub(crate) const PALETTE: [[u8; 3]; 17] = [
    [0xFF, 0x42, 0x3E],
    [0xFF, 0x74, 0x42],
    [0xFC, 0x98, 0x24],
    [0xFF, 0xDD, 0x1A],
    [0xFF, 0xFF, 0x19],
    [0xA3, 0xFF, 0x18],
    [0x53, 0xFF, 0x53],
    [0x19, 0xEA, 0xA3],
    [0x19, 0xEA, 0xEB],
    [0x18, 0xCE, 0xFF],
    [0x4D, 0x94, 0xFF],
    [0x84, 0x72, 0xFF],
    [0xB6, 0x6E, 0xFF],
    [0xDF, 0x5F, 0xFF],
    [0xFF, 0x53, 0xF0],
    [0xFF, 0x48, 0x80],
    [0xFF, 0xFF, 0xFF],
];

fn white(a: u8) -> Color32 {
    Color32::from_white_alpha(a)
}

fn black(a: u8) -> Color32 {
    Color32::from_black_alpha(a)
}

fn with_alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

/// `c` darkened to `f` of its brightness.
fn dim(c: Color32, f: f32) -> Color32 {
    let m = |v: u8| (v as f32 * f) as u8;
    Color32::from_rgb(m(c.r()), m(c.g()), m(c.b()))
}

/// A colour slot; `Default` falls back to `default_slot` (our guess at Maschine's
/// default, two slots per index).
fn palette(c: Color, default_slot: usize) -> Color32 {
    let i = match c {
        Color::Palette(i) => i as usize,
        Color::Default => default_slot % 16,
    };
    let [r, g, b] = PALETTE[i.min(16)];
    Color32::from_rgb(r, g, b)
}

fn group_color(p: &Project, g: usize) -> Color32 {
    palette(p.groups[g].color, g * 2)
}

/// Group `g`'s colour, or its sound `s`'s, as a palette index (0..17, 16 white): what
/// the controller's lights show.
pub(crate) fn colour_index(p: &Project, g: usize, s: Option<usize>) -> usize {
    let slot = |c: Color, default: usize| match c {
        Color::Palette(i) => (i as usize).min(16),
        Color::Default => default % 16,
    };
    let group = slot(p.groups[g].color, g * 2);
    match s.and_then(|s| p.groups[g].sounds.get(s)) {
        Some(snd) if snd.color != Color::Default => slot(snd.color, 0),
        _ => group,
    }
}

/// Scene `s`'s colour as a palette index (0..16).
pub(crate) fn scene_index(p: &Project, s: usize) -> usize {
    match p.scenes[s].color {
        Color::Palette(i) => (i as usize).min(16),
        Color::Default => (s * 2) % 16,
    }
}

/// A pattern of group `g` in colour `c` as RGB (the default is its group's).
pub(crate) fn pattern_rgb(p: &Project, g: usize, c: Color) -> [u8; 3] {
    let c = inherit(c, group_color(p, g));
    [c.r(), c.g(), c.b()]
}

/// Group `g`'s colour, or its sound `s`'s, as RGB (for the controller's lights and
/// screens).
pub(crate) fn rgb(p: &Project, g: usize, s: Option<usize>) -> [u8; 3] {
    let c = match s {
        Some(s) => sound_color(p, g, s),
        None => group_color(p, g),
    };
    [c.r(), c.g(), c.b()]
}

fn inherit(c: Color, parent: Color32) -> Color32 {
    if c == Color::Default { parent } else { palette(c, 0) }
}

fn sound_color(p: &Project, g: usize, s: usize) -> Color32 {
    let parent = group_color(p, g);
    p.groups[g].sounds.get(s).map_or(parent, |x| inherit(x.color, parent))
}

fn pattern_color(p: &Project, g: usize, pi: usize) -> Color32 {
    let parent = group_color(p, g);
    let c = inherit(p.groups[g].patterns[pi].color, parent);
    if p.groups[g].muted { Color32::from_gray(0x80) } else { c }
}

fn scene_color(p: &Project, s: usize) -> Color32 {
    palette(p.scenes[s].color, s * 2)
}

/// Group ID as Maschine shows it: A1..H1, then A2.. for the next bank of eight.
pub fn group_id(g: usize) -> String {
    format!("{}{}", (b'A' + (g % 8) as u8) as char, g / 8 + 1)
}

/// The label on group `g`'s tab: its letter and bank (A1), as Maschine's.
fn group_tab(g: usize) -> String {
    group_id(g)
}

pub(crate) fn group_name(p: &Project, g: usize) -> String {
    let n = &p.groups[g].name;
    if n.is_empty() { format!("Group {}", group_id(g)) } else { n.clone() }
}

fn scene_name(p: &Project, s: usize) -> String {
    let n = &p.scenes[s].name;
    if n.is_empty() { format!("Scene {}", s + 1) } else { n.clone() }
}

fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Text clipped to `rect`, left-aligned at `indent`, vertically centred.
fn label(painter: &Painter, rect: Rect, indent: f32, text: &str, size: f32, color: Color32) {
    painter.with_clip_rect(rect.intersect(painter.clip_rect())).text(
        Pos2::new(rect.left() + indent, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font(size),
        color,
    );
}

/// Bar.beat.sixteenth, 1-based, as Maschine's position display.
fn position_string(p: &Project, ticks: f64) -> String {
    let t = ticks.max(0.0) as i64;
    let bar = p.bar_length().max(1);
    let beat = PPQ * 4 / p.denominator.max(1);
    format!("{}.{}.{}", t / bar + 1, t % bar / beat + 1, t % beat / (PPQ / 4) + 1)
}

pub fn pattern_len(p: &Project, g: usize, pi: usize) -> i64 {
    let l = p.groups[g].patterns[pi].length;
    if l > 0 { l } else { p.bar_length() }
}

/// Where pattern (g, pi) is in its loop, if it is playing.
fn pattern_position(p: &Project, head: Option<Playhead>, g: usize, pi: usize) -> Option<f64> {
    let head = head?;
    let plen = pattern_len(p, g, pi) as f64;
    match head.mode {
        Mode::Pattern(hg, hp) if (hg, hp) == (g, pi) => Some(head.position % plen),
        Mode::Scene(s) => p.scenes.get(s)?.patterns.contains(&(g, pi)).then(|| head.position % plen),
        Mode::Arrangement => {
            let mut start = 0.0;
            for sec in &p.sections {
                let end = start + sec.length as f64;
                if head.position < end {
                    let in_scene = sec.scene.and_then(|s| p.scenes.get(s)).is_some_and(|s| s.patterns.contains(&(g, pi)));
                    return in_scene.then(|| (head.position - start) % plen);
                }
                start = end;
            }
            None
        }
        _ => None,
    }
}

/// Group `g`'s progress through its pattern in scene `scene`: (bars, up to 8; how far
/// through them, in bars, 0 when not playing), or `None` when the scene has no pattern
/// for it.
pub fn group_progress(p: &Project, head: Option<Playhead>, scene: usize, g: usize) -> Option<(usize, f64)> {
    let &(_, pi) = p.scenes.get(scene)?.patterns.iter().find(|(pg, _)| *pg == g)?;
    let len = pattern_len(p, g, pi) as f64;
    let bars = (len / p.bar_length() as f64).ceil().clamp(1.0, 8.0) as usize;
    let played = pattern_position(p, head, g, pi).map_or(0.0, |t| t / len * bars as f64);
    Some((bars, played.min(bars as f64)))
}

/// Segments of a progress strip in `strip`, `gap` apart, filled up to `played` (bars):
/// the bar playing fills as it goes, so even a one-bar pattern moves.
fn progress_strip(painter: &Painter, strip: Rect, bars: usize, gap: f32, played: f64, filled: Color32, unfilled: Color32) {
    let w = (strip.width() - gap * (bars - 1) as f32) / bars as f32;
    for b in 0..bars {
        let seg = Rect::from_min_size(Pos2::new(strip.left() + b as f32 * (w + gap), strip.top()), Vec2::new(w, strip.height()));
        painter.rect_filled(seg, 0.0, unfilled);
        let part = (played - b as f64).clamp(0.0, 1.0) as f32;
        if part > 0.0 {
            painter.rect_filled(Rect::from_min_size(seg.min, Vec2::new(w * part, seg.height())), 0.0, filled);
        }
    }
}

/// The focused pattern's span on the editor's timeline, which starts at the pattern's
/// start (`x0`, `scale` px a tick): drag its left edge to move the start (the end
/// stays), its right edge to set the length, or between them to move both (the
/// pattern's position over its notes). Drags snap to the grid (not with SHIFT).
#[allow(clippy::too_many_arguments)]
fn pattern_span_handles(
    ui: &mut Ui,
    app: &App,
    painter: &Painter,
    tl: Rect,
    start: i64,
    len: i64,
    scale: f32,
    x0: f32,
    acts: &mut Vec<Act>,
) {
    #[derive(Clone, Copy)]
    enum Part {
        Start,
        End,
        Body,
    }
    let (g, pi) = (app.group, app.pattern);
    let end_x = x0 + len as f32 * scale;
    let parts = [
        (Part::Start, Rect::from_min_max(Pos2::new(x0 - 4.0, tl.top()), Pos2::new(x0 + 5.0, tl.bottom()))),
        (Part::End, Rect::from_min_max(Pos2::new(end_x - 5.0, tl.top()), Pos2::new(end_x + 4.0, tl.bottom()))),
        (Part::Body, Rect::from_min_max(Pos2::new(x0 + 5.0, tl.top()), Pos2::new(end_x - 5.0, tl.bottom()))),
    ];
    // The span as the drag began (the view moves under the pointer as it changes).
    let key = egui::Id::new(("pattern-span-drag", g, pi));
    for (part, r) in parts {
        if r.width() <= 0.0 {
            continue;
        }
        let resp = ui.interact(r, key.with(part as u8), Sense::drag());
        let hovered = resp.hovered() || resp.dragged();
        if hovered {
            ui.ctx().set_cursor_icon(match part {
                Part::Body => egui::CursorIcon::Grab,
                _ => egui::CursorIcon::ResizeHorizontal,
            });
        }
        if resp.drag_started() {
            ui.data_mut(|d| d.insert_temp(key, (start, len, scale)));
        }
        if resp.dragged()
            && let Some((s0, l0, sc)) = ui.data(|d| d.get_temp::<(i64, i64, f32)>(key))
            && let (Some(from), Some(now)) = ui.input(|i| (i.pointer.press_origin(), i.pointer.latest_pos()))
        {
            let free = ui.input(|i| i.modifiers.shift);
            let snap = |t: i64| if free { t } else { (t as f64 / app.grid.max(1) as f64).round() as i64 * app.grid.max(1) };
            let delta = ((now.x - from.x) / sc) as i64;
            let min_len = if free { PPQ / 4 } else { app.grid.max(PPQ / 4) };
            let (s, l) = match part {
                Part::Start => {
                    let s = snap(s0 + delta).max(0).min(s0 + l0 - min_len);
                    (s, s0 + l0 - s)
                }
                Part::End => (s0, snap(l0 + delta).max(min_len)),
                Part::Body => (snap(s0 + delta).max(0), l0),
            };
            if (s, l) != (start, len) {
                acts.push(Act::PatternSpan(g, pi, s, l));
            }
        }
        // The ends: a bright bar with a tab into the ruler.
        if !matches!(part, Part::Body) {
            let x = if matches!(part, Part::Start) { x0 } else { end_x };
            let ink = if hovered { Color32::WHITE } else { white(170) };
            painter.vline(x, tl.y_range(), Stroke::new(2.0, ink));
            let tab = if matches!(part, Part::Start) {
                Rect::from_min_size(Pos2::new(x, tl.top()), Vec2::new(5.0, 6.0))
            } else {
                Rect::from_min_size(Pos2::new(x - 5.0, tl.top()), Vec2::new(5.0, 6.0))
            };
            painter.rect_filled(tab, 0.0, ink);
        }
    }
}

/// Index of the section playing at `ticks`.
fn section_at(p: &Project, ticks: f64) -> Option<usize> {
    let mut start = 0.0;
    for (i, s) in p.sections.iter().enumerate() {
        start += s.length as f64;
        if ticks < start {
            return Some(i);
        }
    }
    None
}

/// The DECKARD wordmark at `left_center`: wide-set capitals cut by a line that runs past
/// both ends, the slice above the line lighter than the letters below it. Returns its
/// extent (letters only).
pub fn deckard_logo(painter: &Painter, left_center: Pos2) -> Rect {
    // The logo font, if one was given (logo.ttf in the settings folder).
    if painter.ctx().data(|d| d.get_temp::<bool>(egui::Id::new(LOGO_FONT))).unwrap_or(false) {
        let font = FontId::new(LOGO_SIZE, egui::FontFamily::Name(LOGO_FONT.into()));
        // All lowercase: the font's capital R (U+0052) is the whole "BLADE RUNNER" logo,
        // so the r is U+0072.
        const NAME: &str = "d\u{65}ck\u{61}\u{72}d";
        return painter.text(Pos2::new(left_center.x + 4.0, left_center.y), Align2::LEFT_CENTER, NAME, font, white(240));
    }
    let font = FontId::proportional(18.0);
    let spacing = 2.5;
    let galleys: Vec<_> = "DECKARD".chars().map(|c| painter.layout_no_wrap(c.to_string(), font.clone(), Color32::WHITE)).collect();
    let width: f32 = galleys.iter().map(|g| g.size().x).sum::<f32>() + spacing * (galleys.len() - 1) as f32;
    let h = galleys[0].size().y;
    let rect = Rect::from_min_size(Pos2::new(left_center.x + 4.0, left_center.y - h / 2.0), Vec2::new(width, h));
    // Cap height sits in the middle of the line box: the cut is a third of the way down
    // the capitals.
    let cap_top = rect.top() + h * 0.24;
    let cut = (cap_top + h * 0.25).round() + 0.5;
    let upper = Rect::from_min_max(Pos2::new(rect.left() - 10.0, rect.top()), Pos2::new(rect.right() + 10.0, cut - 1.0));
    let lower = Rect::from_min_max(Pos2::new(rect.left() - 10.0, cut + 1.0), Pos2::new(rect.right() + 10.0, rect.bottom()));
    for (clip, ink) in [(upper, white(150)), (lower, white(240))] {
        let p = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
        let mut x = rect.left();
        for (g, c) in galleys.iter().zip("DECKARD".chars()) {
            p.text(Pos2::new(x, rect.top()), Align2::LEFT_TOP, c, font.clone(), ink);
            x += g.size().x + spacing;
        }
    }
    painter.hline((rect.left() - 6.0)..=(rect.right() + 6.0), cut, Stroke::new(1.0, white(200)));
    rect
}

/// What clicks and drags do in the event area: Maschine's five mouse modes (software
/// manual p. 236). E, X, Y and Z switch Draw, Split, Join and Mute on and off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseMode {
    #[default]
    Select,
    Draw,
    /// A click deletes a note; in a lane, a click or a drag erases (BUGS #17).
    Erase,
    Split,
    Join,
    Mute,
}

/// The Nudge Grid: what ⌥← / ⌥→ (and the MK3's nudges) move notes by, the step grid
/// over 2^`.0` (Step, Step/2 … Step/16; Step/2 to start).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NudgeGrid(pub usize);

impl Default for NudgeGrid {
    fn default() -> NudgeGrid {
        NudgeGrid(1)
    }
}

impl NudgeGrid {
    pub fn ticks(self, grid: i64) -> i64 {
        (grid.max(1) >> self.0.min(4)).max(1)
    }
}

impl MouseMode {
    /// `mode`, or Select if it is the one on (its key pressed again).
    pub fn toggled(self, mode: MouseMode) -> MouseMode {
        if self == mode { MouseMode::Select } else { mode }
    }
}

/// What clicks in the panels ask for; applied after drawing.
pub enum Act {
    /// (For edits that do nothing.)
    Nothing,
    /// The browser's.
    Browse(crate::browser::BrowseAct),
    /// Switch slot `.1` of a chain on or off (the Mix view's group and master strips).
    ChainSlotActive(deckard_doc::Chain, usize, bool),
    /// Show slot `.1` of a chain in the chain area (its tab, its pages).
    FocusChainSlot(deckard_doc::Chain, usize),
    /// Save a sound, group or plug-in preset as a file.
    SaveItem(deckard_doc::FileItem, crate::SaveMode),
    /// Load a sound, group or preset file (chosen in a dialog) into it.
    OpenItem(deckard_doc::FileItem),
    /// Show the channel properties (`true`) or the plug-ins in the Control area.
    ChannelMode(bool),
    /// Show a set of channel properties (Input, Output, Groove, Macro), or a page of it.
    PropSet(usize),
    PropPage(usize),
    SetRoute(RouteSet),
    Groove(GrooveButton),
    Macro(MacroAct),
    /// The Assignment area: MIDI learn, host automation, macro slots.
    Remote(crate::remote::RemoteAct),
    LaunchScene(usize),
    LaunchSection(usize),
    FocusGroup(usize),
    FocusPattern(usize, usize),
    FocusSound(usize),
    SongView(bool),
    PianoRoll(bool),
    ChainTab(ChainTab),
    Play,
    /// REC on or off.
    Record,
    /// SHIFT + REC: count in, then play and record.
    RecordCountIn,
    /// Auto-write on or off.
    ToggleAutoWrite,
    /// A knob moved while writing modulation (to this position; the knob let go).
    Modulate(Edit, f32, bool),
    /// Remove a knob's modulation.
    ClearModulation(Edit),
    Restart,
    Stop,
    ToggleLoop,
    ToggleMetronome,
    /// LINK: join or leave the Ableton Link session.
    ToggleLink,
    ToggleKeys,
    ToggleController,
    SetTempo(f64),
    SetLevel(Target, f64),
    SetPan(Target, f64),
    ToggleMute(Target),
    Save,
    ToggleFollow,
    // Scene and arrangement edits.
    AssignPattern(usize, usize, Option<usize>),
    /// A new empty pattern in group `.1`, put in scene `.0` and focused.
    NewPattern(usize, usize),
    AddScene,
    /// A new empty group after the last one (the "+" after the groups), focused.
    AddGroup,
    InsertScene(usize),
    DuplicateScene(usize),
    DeleteScene(usize),
    RenameScene(usize, String),
    AddSection(usize),
    DuplicateSection(usize),
    DeleteSection(usize),
    MoveSection(usize, usize),
    MoveScene(usize, usize),
    SectionLength(usize, Option<i64>),
    SectionScene(usize, usize),
    /// Sound `.1`'s Audio plug-in on (its take in focus) or off in pattern `.2` of group
    /// `.0` (a double-click on its waveform).
    ToggleAudio(usize, usize, usize),
    /// Notes of group `g`'s pattern `p` given new expression (the Expression editor).
    SetExpressions {
        g: usize,
        p: usize,
        edits: Vec<(Note, deckard_model::Expression)>,
    },
    // Notes.
    EditNotes {
        g: usize,
        p: usize,
        label: &'static str,
        remove: Vec<Note>,
        add: Vec<Note>,
    },
    /// Split notes at a tick (each part gets its slice of the notes' expression).
    SplitNotes {
        g: usize,
        p: usize,
        targets: Vec<Note>,
        at: i64,
    },
    /// Join a note with the next on its key (see `note_tools::join`), their
    /// expression one after the other.
    JoinNotes {
        g: usize,
        p: usize,
        all: Vec<Note>,
        clicked: Note,
        selected: Vec<Note>,
    },
    SelectNotes(Vec<Note>),
    DrawMode(bool),
    Grid(i64),
    /// The scale notes are written in: (root note, bank, scale), shared with the
    /// controller's Keyboard mode.
    Scale(i64, i64, i64),
    /// Sound (group, sound)'s Pad Touch: what a held MK3 pad's pressure does for it.
    PadTouch(usize, usize, deckard_model::pad_touch::PadTouch),
    /// Sound (group, sound)'s modulation routes, the whole list (the MOD tab).
    ModRoutes(usize, usize, Vec<deckard_model::mod_matrix::ModRoute>),
    /// Play a pitch while the mouse button is held.
    HoldPitch(usize, usize, i64),
    StartRename(Target),
    Rename(Target, String),
    CancelRename,
    Quantize(i64),
    /// Put a Perform FX first in group `.0`'s chain, playing effect `.1` (its mode).
    AddPerformFx(usize, usize),
    /// Engage (or release) group `.0`'s Perform FX, as touching the strip.
    PerformEngage(usize, bool),
    /// The event area's mouse mode.
    SetMouse(MouseMode),
    /// Where the editing keys act.
    KeyFocus(KeyFocus),
    /// The Nudge Grid (Step / 2^n).
    NudgeGrid(usize),
    /// Copy the selected notes (or every note shown); `cut` takes them out too.
    CopyNotes {
        cut: bool,
    },
    PasteNotes(crate::clipboard::PasteAt),
    PatternLength(usize, usize, i64),
    /// Pattern `.1` of group `.0` back to an Auto length.
    PatternLengthAuto(usize, usize),
    /// A pattern's start and length (dragging its ends or its body on the timeline).
    PatternSpan(usize, usize, i64, i64),
    TimeSignature(i64, i64),
    SetSwing(Target, Swing),
    /// Set a Sampler or zone parameter (dragging a Sampler knob).
    SetParam(Target, &'static str, f64),
    SamplerPage(usize),
    /// Show a slot's pages (`None`: the sound's channel).
    SelectSlot(Option<usize>),
    AddSlot(AddWhat),
    RemoveSlot(usize),
    /// Move slot `.0` of the focused sound's chain to position `.1`.
    MoveSlot(usize, usize),
    SlotActive(usize, bool),
    OpenSlotEditor(usize),
    /// Set parameter `id` of the plug-in in slot `.0` (normalized).
    PluginParam(usize, u32, f64),
    /// [`Act::PluginParam`] of the plug-in in slot `.1` of chain `.0` (a macro's: any
    /// chain).
    PluginParamIn(deckard_doc::Chain, usize, u32, f64),
    PluginPage(usize),
    Preferences,
    /// The Sampling view.
    Sampling(sampling::SamplingAct),
    // The Mixer.
    ToggleMixer,
    /// The LFO panel, in the Mixer's place.
    ToggleLfos,
    /// Give the project this LFO (adding it, or replacing the one on its pad).
    SetLfo(deckard_model::lfo::Lfo),
    RemoveLfo(u8),
    /// Give the project these CC routes (the LFO panel's MIDI CC section).
    SetCcRoutes(Vec<deckard_model::mod_matrix::CcRoute>),
    FocusLfo(u8),
    /// Show or hide the Sampler pages' waveform.
    ToggleWaveform,
    // Context menus.
    SetColor(Vec<Target>, Color),
    CutSound(usize, usize),
    CopySound(usize, usize),
    PasteSound(usize, usize),
    ResetSound(usize, usize),
    CutGroup(usize),
    CopyGroup(usize),
    PasteGroup(usize),
    DeleteGroup(usize),
    ImportMidi(usize, usize),
    ExportMidi(usize, usize),
    ImportGroupMidi(usize),
    ExportGroupMidi(usize),
    /// Bounce sound `.1` of group `.0` in place (asks first).
    Bounce(usize, usize),
    /// ⌘.: the focused group's pattern in the picked section becomes a clip.
    ConvertToClip,
    /// Section ▸ Convert to Clip(s).
    ConvertSectionToClips(usize),
    /// Clip `.1` of group `.0` (the Song view's clips).
    Clip(usize, usize, ClipAct),
    /// Render pattern `.1` of group `.0` (the group, or the focused sound in Keyboard
    /// view): to a file in the Export Audio folder, or (`true`) into the group's first
    /// empty sound (Sample to Sound).
    PatternAudio(usize, usize, bool),
    ClearPattern(usize, usize),
    DoublePattern(usize, usize),
    DuplicatePattern(usize, usize),
    DeletePattern(usize, usize),
    MixerSounds(bool),
    MixerSection(MixerSection),
    Cue(Target, bool),
    /// Aux send `.1` (0, 1) level of a channel strip.
    SetAux(Target, usize, f64),
    /// Mute every other channel of the same level (again: unmute them all).
    Solo(Channel),
    ResetHeadroom(Channel),
    /// Bypass or enable slot `.2` of sound `.1` in group `.0`.
    SlotActiveOf(usize, usize, usize, bool),
    /// Focus sound `.1` of group `.0` and show slot `.2` in the SOUND tab (`usize::MAX`:
    /// its channel).
    FocusSlot(usize, usize, usize),
    /// Show this lane under the note area.
    Lane(Lane),
    LanesDialog(bool),
    AddLane(Lane),
    /// Take a MIDI lane off the list, deleting the focused sound's events in it.
    RemoveLane(Lane),
    /// Erase what lane `0` has from tick `1` to `2` (the focused sound's, in the
    /// focused pattern): its events, or its notes' expression there (BUGS #17).
    EraseSpan(Lane, i64, i64),
    /// A lane's span selected (Shift-drag), or none.
    LaneSpan(Option<(Lane, i64, i64)>),
    EditMidi {
        g: usize,
        p: usize,
        label: &'static str,
        remove: Vec<MidiEvent>,
        add: Vec<MidiEvent>,
    },
}

/// An edit of a channel's macros (`ch`: the channel's document target).
#[derive(Debug, Clone)]
pub enum MacroAct {
    /// Point macro `slot` of page `page` at a parameter.
    Assign {
        ch: Target,
        page: usize,
        slot: usize,
        t: Target,
        field: &'static str,
    },
    /// Point macro `slot` of page `page` at parameter `id` of the plug-in in slot
    /// `plugin_slot` of `chain` (its pages saved first).
    AssignPlugin {
        ch: Target,
        page: usize,
        slot: usize,
        chain: deckard_doc::Chain,
        plugin_slot: usize,
        id: u32,
    },
    /// Make a parameter a macro (the next free slot).
    Create {
        ch: Target,
        t: Target,
        field: &'static str,
    },
    /// Empty the macros of `ch` that control a parameter.
    Remove {
        ch: Target,
        t: Target,
        field: &'static str,
    },
    Reset {
        ch: Target,
        page: usize,
        slot: usize,
    },
    Section {
        ch: Target,
        page: usize,
        slot: usize,
        name: String,
    },
    DeletePage(Target, usize),
    RenamePage(Target, usize, String),
    Clear(Target),
    /// Make parameter `id` of the plug-in in slot `slot` of `chain` a macro of `ch`, or
    /// stop it being one.
    Plugin {
        ch: Target,
        chain: deckard_doc::Chain,
        slot: usize,
        id: u32,
    },
}

/// What's done to a clip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipAct {
    Select,
    /// Move it to this tick.
    Move(i64),
    /// Make it this long (ticks).
    Length(i64),
    Duplicate,
    Double,
    Delete,
}

/// What the "+" of a chain adds.
#[derive(Debug, Clone)]
pub enum AddWhat {
    Sampler,
    Audio,
    Plugin(deckard_plugins::scan::Entry),
    /// One of Maschine's effects (its class: "EQ", "BeatDelay", …).
    Effect(&'static str),
    /// One of Maschine's instruments (its class: "Kick", "BassSynth", …).
    Instrument(&'static str),
    /// A group's Perform FX in mode `0` (`module_ui::PERFORM_MODES`), first in its chain.
    PerformFx(usize),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ChainTab {
    Master,
    Group,
    #[default]
    Sound,
}

/// Carry out what the panels (or the controller's Control page) asked for.
pub fn apply_acts(app: &mut App, acts: Vec<Act>) {
    apply(app, acts)
}

fn apply(app: &mut App, acts: Vec<Act>) {
    for a in acts {
        app.macro_learn_saw(&a);
        match a {
            Act::LaunchScene(s) => app.launch_scene(s),
            Act::LaunchSection(s) => app.launch_section(s),
            Act::FocusGroup(g) => app.focus_group(g),
            Act::FocusPattern(g, p) => {
                if g != app.group {
                    app.sound = 0;
                }
                app.group = g;
                app.pattern = p;
            }
            Act::FocusSound(s) => app.sound = s,
            Act::SongView(v) => app.song_view = v,
            Act::PianoRoll(v) => app.piano_roll = v,
            Act::ChainTab(t) => app.chain_tab = t,
            Act::Play => app.toggle_play(),
            Act::Record => app.set_recording(!app.recording, false),
            // Off while recording; else on (armed already: started) with a count-in.
            Act::RecordCountIn => app.set_recording(!(app.recording && app.playing.is_some()), true),
            Act::PatternLengthAuto(g, p) => {
                if let Some(doc) = &app.doc {
                    doc.borrow_mut().set_length_auto(g, p, true);
                }
            }
            Act::ToggleAutoWrite => app.auto_write = !app.auto_write,
            Act::Modulate(e, a, done) => app.modulate(e, a, done),
            Act::ClearModulation(e) => app.clear_modulation(e),
            Act::Restart => app.restart(),
            Act::ToggleLoop => app.set_looping(!app.looping),
            Act::ToggleMetronome => app.set_metronome(!app.metronome),
            Act::ToggleLink => app.toggle_link(),
            Act::ToggleKeys => app.toggle_keys(),
            Act::Stop => {
                app.stop();
                app.sync_transport();
            }
            Act::ToggleController => app.show_controller = !app.show_controller,
            Act::Save => app.save(),
            Act::ToggleFollow => app.follow = !app.follow,
            Act::AssignPattern(..)
            | Act::NewPattern(..)
            | Act::AddScene
            | Act::AddGroup
            | Act::InsertScene(_)
            | Act::DuplicateScene(_)
            | Act::DeleteScene(_)
            | Act::RenameScene(..)
            | Act::AddSection(_)
            | Act::DuplicateSection(_)
            | Act::DeleteSection(_)
            | Act::MoveSection(..)
            | Act::MoveScene(..)
            | Act::SectionLength(..)
            | Act::SectionScene(..)
            | Act::EditNotes { .. }
            | Act::SetExpressions { .. }
            | Act::SplitNotes { .. }
            | Act::JoinNotes { .. }
            | Act::ToggleAudio(..)
            | Act::EditMidi { .. } => app.arrange(a),
            Act::Lane(l) => app.lane = l,
            Act::LanesDialog(open) => app.lanes_dialog = open,
            Act::AddLane(l) => {
                app.lane = l;
                if let (Some(doc), Some(r)) = (app.doc.clone(), l.to_ref())
                    && let Err(e) = doc.borrow_mut().add_lane(app.group, app.sound, r)
                {
                    app.status = format!("can't add the lane: {e}");
                }
            }
            Act::RemoveLane(l) => {
                if app.lane == l {
                    app.lane = Lane::Velocity;
                }
                // An Expression editor lane: what it shows goes from the sound's notes
                // in the pattern, one undo step (BUGS #18).
                if let (Some(doc), Some(dim)) = (app.doc.clone(), expr_lane::dim_of(l)) {
                    let mut d = doc.borrow_mut();
                    let edits: Vec<(Note, deckard_model::Expression)> = {
                        let p = d.project();
                        let pat = p.groups.get(app.group).and_then(|g| g.patterns.get(app.pattern));
                        pat.map_or(Vec::new(), |pt| {
                            pt.notes
                                .iter()
                                .filter(|n| n.sound == app.sound)
                                .filter_map(|n| {
                                    let mut e = expr_lane::expression_of(p, n);
                                    if !has_lane(&e, l) {
                                        return None;
                                    }
                                    match dim {
                                        Some(dim) => *e.curve_mut(dim) = None,
                                        None => e.release_velocity = None,
                                    }
                                    Some((*n, e))
                                })
                                .collect()
                        })
                    };
                    if !edits.is_empty()
                        && let Err(e) = d.set_expressions(app.group, app.pattern, &edits)
                    {
                        app.status = format!("can't remove the lane: {e}");
                    }
                    continue;
                }
                let (Some(doc), Some(r)) = (app.doc.clone(), l.to_ref()) else { continue };
                let mut d = doc.borrow_mut();
                let events: Vec<MidiEvent> =
                    d.project().groups.get(app.group).and_then(|g| g.patterns.get(app.pattern)).map_or(Vec::new(), |pt| {
                        pt.midi.iter().filter(|m| m.sound == app.sound && Lane::of(&m.kind) == l).copied().collect()
                    });
                if let Err(e) = d.remove_lane(app.group, app.pattern, app.sound, r, &events) {
                    app.status = format!("can't remove the lane: {e}");
                }
            }
            Act::EraseSpan(l, from, to) => {
                app.lane_span = None;
                erase_span(app, l, from.min(to), from.max(to));
            }
            Act::LaneSpan(span) => app.lane_span = span,
            Act::SelectNotes(n) => app.selected = n,
            Act::DrawMode(on) => app.mouse = if on { MouseMode::Draw } else { MouseMode::Select },
            Act::SetMouse(m) => app.mouse = m,
            Act::PerformEngage(g, on) => {
                let pos = app
                    .perform_slot(g)
                    .and_then(|i| app.doc.as_ref()?.borrow().project().groups[g].slots[i].param("touchstrip_pos"))
                    .unwrap_or(0.5);
                app.perform_strip(g, Some(on), pos);
            }
            Act::AddPerformFx(g, mode) => {
                let Some(doc) = app.doc.clone() else { continue };
                let added = doc.borrow_mut().transaction("Add Perform FX", |d| {
                    d.add_perform_fx(g)?;
                    d.set("Add Perform FX", Target::ChainModule(deckard_doc::Chain::Group(g), 0), "mode", Value::Float(mode as f64))
                });
                if let Err(e) = added {
                    app.status = format!("can't add Perform FX: {e}");
                    continue;
                }
                app.chain_tab = ChainTab::Group;
                app.slot_of = Some(deckard_doc::Chain::Group(g));
                app.slot = Some(0);
                app.channel = false;
            }
            Act::KeyFocus(f) => app.key_focus = f,
            Act::NudgeGrid(i) => app.nudge = NudgeGrid(i.min(4)),
            Act::CopyNotes { cut } => app.copy_notes(cut),
            Act::PasteNotes(at) => app.paste_notes(at),
            Act::Grid(t) => app.grid = t,
            Act::Scale(root, bank, scale) => {
                let from = app.scale.0;
                app.scale = (root, bank, scale);
                if root != from {
                    app.follow_root(from, root);
                }
                if let Some(Ok(c)) = &mut app.controller {
                    c.host.set_scale(root, bank, scale);
                }
            }
            Act::StartRename(t) => app.renaming = Some(t),
            Act::HoldPitch(g, snd, pitch) => app.hold_pitch(g, snd, pitch),
            Act::CancelRename => app.renaming = None,
            Act::Rename(t, name) => {
                app.renaming = None;
                if let Some(doc) = app.doc.clone()
                    && let Err(e) = doc.borrow_mut().rename(t, &name)
                {
                    app.status = format!("can't rename: {e}");
                }
            }
            Act::Quantize(_) | Act::PatternLength(..) | Act::PatternSpan(..) | Act::TimeSignature(..) | Act::SetSwing(..) => app.arrange(a),
            Act::SamplerPage(i) => app.sampler_page = i,
            Act::Nothing => {}
            Act::ChannelMode(on) => app.channel = on,
            Act::PropSet(i) => {
                app.channel = true;
                if app.prop_set != i {
                    app.prop_set = i;
                    app.prop_page = 0;
                }
            }
            Act::PropPage(i) => {
                // The "+" after the macro pages adds one.
                let ch = app.macro_channel();
                let count = match (&app.doc, ch) {
                    (Some(doc), Some(ch)) if app.prop_set == 3 => Some((doc.clone(), ch, doc.borrow().macro_pages(ch).len())),
                    _ => None,
                };
                match count {
                    Some((doc, ch, n)) if i >= n => match doc.borrow_mut().add_macro_page(ch) {
                        Ok(p) => app.prop_page = p,
                        Err(e) => app.status = format!("can't add a page: {e}"),
                    },
                    _ => app.prop_page = i,
                }
            }
            Act::SetRoute(r) => app.set_route(r),
            Act::Groove(b) => app.groove(b),
            Act::Remote(r) => app.remote_act(r),
            Act::Macro(MacroAct::Plugin { ch, chain, slot, id }) => app.toggle_plugin_macro(ch, chain, slot, id),
            Act::Macro(MacroAct::AssignPlugin { ch, page, slot, chain, plugin_slot, id }) => {
                app.assign_plugin_macro((ch, page, slot), chain, plugin_slot, id)
            }
            Act::Macro(m) => {
                let Some(doc) = app.doc.clone() else { continue };
                let mut d = doc.borrow_mut();
                let done = match m {
                    MacroAct::Assign { ch, page, slot, t, field } => d.set_macro(ch, page, slot, Some((t, field)), ""),
                    MacroAct::Create { ch, t, field } => d.add_macro(ch, t, field).map(|_| ()),
                    MacroAct::Remove { ch, t, field } => d.remove_macro_for(ch, t, field),
                    MacroAct::Reset { ch, page, slot } => d.set_macro(ch, page, slot, None, ""),
                    MacroAct::Section { ch, page, slot, name } => d.set_macro_section(ch, page, slot, &name),
                    MacroAct::DeletePage(ch, page) => d.remove_macro_page(ch, page).map(|_| {
                        app.prop_page = app.prop_page.min(page.saturating_sub(1));
                    }),
                    MacroAct::RenamePage(ch, page, name) => d.rename_macro_page(ch, page, &name),
                    MacroAct::Clear(ch) => d.clear_macros(ch).map(|_| app.prop_page = 0),
                    MacroAct::Plugin { .. } | MacroAct::AssignPlugin { .. } => Ok(()),
                };
                if let Err(e) = done {
                    app.status = format!("can't change the macros: {e}");
                }
            }
            Act::SelectSlot(i) => {
                app.channel = i.is_none();
                app.slot = i;
                app.plugin_page = 0;
                app.sampler_page = 0;
            }
            Act::AddSlot(what) => app.add_slot(what),
            Act::MoveSlot(from, to) => app.move_slot(from, to),
            Act::RemoveSlot(i) => {
                if let Some(doc) = app.doc.clone()
                    && let Err(e) = doc.borrow_mut().remove_slot_in(app.chain(), i)
                {
                    app.status = format!("can't remove it: {e}");
                }
                app.plugin_view = None;
            }
            Act::SlotActive(i, on) => {
                if let Some(doc) = app.doc.clone()
                    && let Err(e) = doc.borrow_mut().set_slot_active_in(app.chain(), i, on)
                {
                    app.status = format!("can't switch it: {e}");
                }
            }
            Act::OpenSlotEditor(i) => app.open_slot_editor_in(app.chain(), i),
            Act::PluginParam(i, id, v) => set_plugin_param(app, app.chain(), i, id, v),
            Act::PluginParamIn(c, i, id, v) => set_plugin_param(app, c, i, id, v),
            Act::PluginPage(n) => app.plugin_page = n,
            Act::Preferences => app.prefs = Some(app.prefs.unwrap_or(0)),
            Act::ToggleMixer => {
                app.mixer = !app.mixer;
                app.lfos &= !app.mixer;
            }
            Act::ToggleLfos => {
                app.lfos = !app.lfos;
                app.mixer &= !app.lfos;
            }
            Act::FocusLfo(pad) => app.lfo_focus = pad,
            Act::SetLfo(lfo) => {
                let Some(doc) = app.doc.clone() else { continue };
                if let Err(e) = doc.borrow_mut().set_lfo(lfo) {
                    app.status = format!("can't change that: {e}");
                }
            }
            Act::SetCcRoutes(routes) => {
                let Some(doc) = app.doc.clone() else { continue };
                if let Err(e) = doc.borrow_mut().set_cc_routes(routes) {
                    app.status = format!("can't change that: {e}");
                }
            }
            Act::RemoveLfo(pad) => {
                let Some(doc) = app.doc.clone() else { continue };
                if let Err(e) = doc.borrow_mut().remove_lfo(pad) {
                    app.status = format!("can't remove that: {e}");
                }
                let d = doc.borrow();
                let lfos = &d.project().lfos;
                if !lfos.iter().any(|l| l.pad == app.lfo_focus) {
                    app.lfo_focus = lfos.first().map_or(0, |l| l.pad);
                }
            }
            Act::Browse(b) => app.browse(b),
            Act::SaveItem(what, mode) => app.save_item(what, mode),
            Act::ChainSlotActive(c, i, on) => {
                if let Some(doc) = app.doc.clone()
                    && let Err(e) = doc.borrow_mut().set_slot_active_in(c, i, on)
                {
                    app.status = format!("can't switch it: {e}");
                }
            }
            Act::FocusChainSlot(c, i) => {
                app.chain_tab = match c {
                    deckard_doc::Chain::Master => ChainTab::Master,
                    deckard_doc::Chain::Group(g) => {
                        app.focus_group(g);
                        ChainTab::Group
                    }
                    deckard_doc::Chain::Sound(..) => ChainTab::Sound,
                };
                app.channel = false;
                app.slot = Some(i);
                app.slot_of = Some(app.chain());
                app.plugin_page = 0;
            }
            Act::OpenItem(what) => app.open_item(what),
            Act::Bounce(g, s) => app.request_bounce(g, s),
            Act::ConvertToClip => app.convert_focused_to_clip(),
            Act::ConvertSectionToClips(i) => app.convert_section_to_clips(i),
            Act::Clip(g, c, a) => app.clip_act(g, c, a),
            Act::PatternAudio(g, pi, to_sound) => app.pattern_audio(g, pi, to_sound),
            Act::ToggleWaveform => app.toggle_waveform(),
            Act::SetColor(..)
            | Act::CutSound(..)
            | Act::CopySound(..)
            | Act::PasteSound(..)
            | Act::ResetSound(..)
            | Act::CutGroup(..)
            | Act::CopyGroup(..)
            | Act::PasteGroup(..)
            | Act::DeleteGroup(..)
            | Act::ImportMidi(..)
            | Act::ExportMidi(..)
            | Act::ImportGroupMidi(..)
            | Act::ExportGroupMidi(..)
            | Act::ClearPattern(..)
            | Act::DoublePattern(..)
            | Act::DuplicatePattern(..)
            | Act::DeletePattern(..) => app.menu_act(a),
            Act::Sampling(a) => app.sampling_act(a),
            Act::MixerSounds(on) => app.mixer_sounds = on,
            Act::MixerSection(sec) => {
                let m = &mut app.mixer_sections;
                match sec {
                    MixerSection::Extended => m.extended = !m.extended,
                    MixerSection::Io => m.io = !m.io,
                    MixerSection::Plugins => m.plugins = !m.plugins,
                    MixerSection::Aux => m.aux = !m.aux,
                }
            }
            Act::Cue(t, on) => {
                if let Some(doc) = app.doc.clone()
                    && let Err(e) = doc.borrow_mut().set_cue(t, on)
                {
                    app.status = format!("can't cue: {e}");
                }
            }
            Act::Solo(ch) => app.solo(ch),
            Act::SetAux(t, k, v) => {
                if let Some(doc) = app.doc.clone()
                    && let Err(e) = doc.borrow_mut().set("Aux Send", t, if k == 0 { "aux1_level" } else { "aux2_level" }, Value::Float(v))
                {
                    app.status = format!("can't change the send: {e}");
                }
            }
            Act::ResetHeadroom(ch) => {
                app.headroom.remove(&ch);
            }
            Act::SlotActiveOf(g, s, i, on) => {
                if let Some(doc) = app.doc.clone()
                    && let Err(e) = doc.borrow_mut().set_slot_active(g, s, i, on)
                {
                    app.status = format!("can't switch it: {e}");
                }
            }
            Act::FocusSlot(g, s, i) => {
                if g != app.group {
                    app.focus_group(g);
                }
                app.sound = s;
                app.chain_tab = ChainTab::Sound;
                app.slot_of = Some(deckard_doc::Chain::Sound(g, s));
                app.slot = (i != usize::MAX).then_some(i);
                app.channel = i == usize::MAX;
                app.plugin_page = 0;
                app.sampler_page = 0;
            }
            // An Audio plug-in take's TEMPO: its length in beats over its frames.
            Act::SetParam(Target::Take(g, s, k), "tempo", bpm) => {
                let Some(doc) = app.doc.clone() else { continue };
                let Some((_, frames, rate)) = app.take_info.filter(|(at, ..)| *at == (g, s, k)) else { continue };
                let beats = bpm.clamp(20.0, 400.0) * frames as f64 / f64::from(rate.max(1)) / 60.0;
                if let Err(e) = doc.borrow_mut().set("Audio", Target::Take(g, s, k), "length", Value::Float(beats)) {
                    app.status = format!("can't change the tempo: {e}");
                }
            }
            Act::SetParam(t, field, v) => {
                let Some(doc) = app.doc.clone() else { continue };
                let label = match t {
                    Target::Take(..) => "Audio",
                    Target::Module(g, s, 0)
                        if doc.borrow().project().groups.get(g).and_then(|x| x.sounds.get(s)).is_some_and(|x| x.audio.is_some()) =>
                    {
                        "Audio"
                    }
                    Target::Zone(..) => "Zone",
                    Target::Sampler(..) => "Sampler",
                    Target::Module(..) => "Effect Parameter",
                    _ => "Channel Property",
                };
                // Variation and Generate settings of a project from before Maschine 3:
                // kept for the session.
                if let Target::Sound(g, s) = t
                    && field.starts_with("sequence_")
                    && !doc.borrow().has_groove_settings(g, s)
                {
                    const WHOLE: [&str; 9] = ["mode", "distribution", "division", "steps", "fills", "rotate", "note_count", "low", "high"];
                    let v = if WHOLE.iter().any(|w| field.ends_with(w)) { v.round() } else { v };
                    app.groove_settings.insert((g, s, field.to_string()), v);
                    app.live_groove(g, s, field);
                    continue;
                }
                if let Err(e) = doc.borrow_mut().set(label, t, field, Value::Float(v)) {
                    app.status = format!("can't change {field}: {e}");
                    continue;
                }
                if let Target::Sound(g, s) = t
                    && field.starts_with("sequence_")
                {
                    app.live_groove(g, s, field);
                }
                // A sound's MIDI controller (a macro or knob on it) goes out now.
                if let Target::Sound(g, s) = t
                    && let Some(n) = field.strip_prefix("midi_macro/parameters/p").and_then(|n| n.parse::<u8>().ok())
                    && n < 128
                    && let Some(Ok(out)) = &app.output
                {
                    let kind = deckard_model::MidiKind::Controller { number: n, value: v.round().clamp(0.0, 127.0) as u8 };
                    out.live(deckard_engine::Live::ControlIn { group: g, sound: s, kind, thru: true, record: false });
                }
            }
            Act::PadTouch(g, s, t) => {
                let Some(doc) = app.doc.clone() else { continue };
                if let Err(e) = doc.borrow_mut().set_pad_touch(g, s, t) {
                    app.status = format!("can't change that: {e}");
                }
            }
            Act::ModRoutes(g, s, routes) => {
                let Some(doc) = app.doc.clone() else { continue };
                if let Err(e) = doc.borrow_mut().set_mod_routes(g, s, routes) {
                    app.status = format!("can't change that: {e}");
                }
            }
            Act::SetTempo(_) | Act::SetLevel(..) | Act::SetPan(..) | Act::ToggleMute(_) => {
                let Some(doc) = app.doc.clone() else { continue };
                let mut d = doc.borrow_mut();
                let done = match a {
                    Act::SetTempo(bpm) => d.set_tempo(bpm),
                    Act::SetLevel(t, v) => d.set_level(t, v),
                    Act::SetPan(t, v) => d.set_pan(t, v),
                    Act::ToggleMute(t) => {
                        let p = d.project();
                        let muted = match t {
                            Target::Group(g) => p.groups[g].muted,
                            Target::Sound(g, s) => p.groups[g].sounds[s].muted,
                            _ => false,
                        };
                        d.set_mute(t, !muted)
                    }
                    _ => unreachable!(),
                };
                if let Err(e) = done {
                    app.status = format!("can't change that: {e}");
                }
            }
        }
    }
}

/// The family name of the logo font, and its size.
const LOGO_FONT: &str = "deckard-logo";
const LOGO_SIZE: f32 = 20.0;

/// The name the emoji font has among the app's fonts.
const EMOJI_FONT: &str = "deckard-emoji";

/// The logo font built in: Phil Steinschneider's freeware "Blade Runner Movie Font"
/// (v3.01, 2002), whose capital R is the film's whole logo.
const BUILT_IN_LOGO_FONT: &[u8] = include_bytes!("../assets/logo.ttf");

/// The logo font: `logo.ttf` in the settings folder if there is one, else the built-in
/// one.
pub fn logo_font() -> Vec<u8> {
    crate::menu::settings_dir().and_then(|d| std::fs::read(d.join("logo.ttf")).ok()).unwrap_or_else(|| BUILT_IN_LOGO_FONT.to_vec())
}

/// The app's fonts: egui's, the logo's, and the emoji font for names.
fn font_definitions() -> egui::FontDefinitions {
    let bytes = logo_font();
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(LOGO_FONT.into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
    fonts.families.insert(egui::FontFamily::Name(LOGO_FONT.into()), vec![LOGO_FONT.into()]);
    // Emoji in names: the full Noto Emoji (egui's has only the older ones), after
    // the text fonts and before egui's.
    fonts.font_data.insert(EMOJI_FONT.into(), std::sync::Arc::new(egui::FontData::from_static(deckard_nwl::draw::EMOJI_FONT)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        list.insert(1.min(list.len()), EMOJI_FONT.into());
    }
    fonts
}

pub fn style(ctx: &egui::Context) {
    {
        let fonts = font_definitions();
        ctx.set_fonts(fonts);
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(LOGO_FONT), true));
    }
    ctx.set_visuals(egui::Visuals::dark());
    ctx.all_styles_mut(|s| {
        use egui::TextStyle::*;
        s.text_styles.insert(Body, font(12.0));
        s.text_styles.insert(Button, font(12.0));
        s.text_styles.insert(Small, font(10.0));
        s.text_styles.insert(Heading, font(15.0));
        s.text_styles.insert(Monospace, FontId::monospace(12.0));
        s.visuals.panel_fill = Color32::BLACK;
        s.visuals.override_text_color = Some(white(245));
    });
}

/// A 32×24-style button drawing its own icon; `on` lights it in `on_color`.
fn icon_button(ui: &mut Ui, size: Vec2, on: bool, on_color: Color32, tip: &str, draw: impl FnOnce(&Painter, Rect, Color32)) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let color = if on {
        on_color
    } else if resp.hovered() {
        white(200)
    } else {
        white(143)
    };
    if resp.hovered() {
        ui.painter().rect_filled(rect.shrink(1.0), 3.0, white(20));
    }
    draw(ui.painter(), rect, color);
    resp.on_hover_text(tip)
}

fn triangle(p: &Painter, c: Pos2, r: f32, color: Color32) {
    p.add(egui::Shape::convex_polygon(
        vec![Pos2::new(c.x - r * 0.7, c.y - r), Pos2::new(c.x + r, c.y), Pos2::new(c.x - r * 0.7, c.y + r)],
        color,
        Stroke::NONE,
    ));
}

// ---------------------------------------------------------------------------------------
// Header

pub fn header(app: &mut App, ui: &mut Ui) {
    let mut acts = Vec::new();
    egui::Panel::top("header")
        .exact_size(HEADER)
        .frame(egui::Frame::new().fill(S00).inner_margin(egui::Margin { left: 6, right: 6, top: 0, bottom: 0 }))
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 0.0);
            let full = ui.max_rect();
            // Left: the menus, the browser, the Mixer, then save (undo and redo are in Edit).
            ui.horizontal_centered(|ui| {
                // The DECKARD wordmark and its menu arrow. As Maschine's, clicking it opens
                // the menus.
                let (r, resp) = ui.allocate_exact_size(Vec2::new(124.0, HEADER), Sense::click());
                let resp = resp.on_hover_text("Menus: File, Edit, View, Controller, Help");
                if resp.hovered() {
                    ui.painter().rect_filled(
                        Rect::from_min_size(r.min, Vec2::new(118.0, HEADER)).shrink2(Vec2::new(-4.0, 4.0)),
                        3.0,
                        white(16),
                    );
                }
                let t = deckard_logo(ui.painter(), r.left_center());
                let c = Pos2::new(t.right() + 9.0, r.center().y + 1.0);
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![c + Vec2::new(-4.0, -2.5), c + Vec2::new(4.0, -2.5), c + Vec2::new(0.0, 2.5)],
                    white(142),
                    Stroke::NONE,
                ));
                let menus = crate::menu::menus(app);
                let mut chosen = Vec::new();
                egui::Popup::menu(&resp).show(|ui| {
                    ui.set_min_width(150.0);
                    for (title, items) in &menus {
                        ui.menu_button(*title, |ui| crate::menu::show(ui, items, &mut chosen));
                    }
                });
                if !chosen.is_empty() {
                    ui.ctx().request_repaint();
                }
                app.menu_commands.extend(chosen);
                ui.add_space(2.0);
                // The browser (F4): a magnifier over a shelf.
                if icon_button(ui, Vec2::new(32.0, 24.0), app.browser.open, white(245), "Show or hide the browser (F4)", |p, r, c| {
                    for i in 0..3 {
                        let x = r.center().x - 7.0 + i as f32 * 4.0;
                        p.vline(x, (r.center().y - 6.0)..=(r.center().y + 6.0), Stroke::new(1.5, c));
                    }
                    p.circle_stroke(r.center() + Vec2::new(5.0, -1.0), 3.5, Stroke::new(1.2, c));
                    p.line_segment([r.center() + Vec2::new(7.5, 1.5), r.center() + Vec2::new(9.5, 4.5)], Stroke::new(1.5, c));
                })
                .clicked()
                {
                    acts.push(Act::Browse(crate::browser::BrowseAct::Toggle));
                }
                ui.add_space(2.0);
                if app.doc.is_some() {
                    let on = app.mixer;
                    if icon_button(ui, Vec2::new(32.0, 24.0), on, white(245), "Show or hide the Mixer", |p, r, c| {
                        // Three faders.
                        for i in 0..3 {
                            let x = r.center().x - 6.0 + i as f32 * 6.0;
                            p.vline(x, (r.center().y - 7.0)..=(r.center().y + 7.0), Stroke::new(1.0, c));
                            let y = r.center().y + [3.0, -3.0, 1.0][i];
                            p.rect_filled(Rect::from_center_size(Pos2::new(x, y), Vec2::new(4.0, 2.5)), 0.5, c);
                        }
                    })
                    .clicked()
                    {
                        acts.push(Act::ToggleMixer);
                    }
                    ui.add_space(4.0);
                    let on = app.lfos;
                    if icon_button(ui, Vec2::new(32.0, 24.0), on, white(245), "Show or hide the LFOs", |p, r, c| {
                        // A circle with a sine through it.
                        p.circle_stroke(r.center(), 8.0, Stroke::new(1.0, c));
                        let pts: Vec<Pos2> = (0..=16)
                            .map(|i| {
                                let t = i as f32 / 16.0;
                                Pos2::new(r.center().x - 8.0 + 16.0 * t, r.center().y - 4.0 * (t * std::f32::consts::TAU).sin())
                            })
                            .collect();
                        p.add(egui::Shape::line(pts, Stroke::new(1.0, c)));
                    })
                    .clicked()
                    {
                        acts.push(Act::ToggleLfos);
                    }
                    ui.add_space(4.0);
                }
                separator(ui);
                if let Some(doc) = app.doc.clone() {
                    let t = Vec2::new(32.0, 24.0);
                    ui.add_space(4.0);
                    let d = doc.borrow();
                    let dirty = d.is_dirty();
                    let save_tip =
                        if dirty { "Save (⌘S); the first save keeps the original as .mxprj.bak" } else { "No unsaved changes" };
                    if icon_button(ui, t, dirty, PLAY_ON, save_tip, |p, r, c| {
                        let b = Rect::from_center_size(r.center(), Vec2::splat(12.0));
                        p.rect_stroke(b, 1.0, Stroke::new(1.5, c), StrokeKind::Inside);
                        p.rect_filled(Rect::from_min_size(b.min + Vec2::new(3.0, 0.0), Vec2::new(6.0, 4.0)), 0.0, c);
                    })
                    .clicked()
                    {
                        acts.push(Act::Save);
                    }
                }
            });
            // The middle, centred as Maschine's: the transport and LINK, the metronome, the
            // display (BPM, SWING, SIG, the position and scene), then follow and the
            // computer keyboard.
            let t = Vec2::new(32.0, 24.0);
            let display = if app.doc.is_some() { 2.0 * DISPLAY_GAP + DISPLAY_SIZE.x } else { DISPLAY_GAP };
            let group = t.x * 6.0 + LINK_WIDTH + METRONOME_GAP + t.x + display + t.x * 2.0;
            let mid_rect = Rect::from_min_size(Pos2::new(full.center().x - group / 2.0, full.top()), Vec2::new(group, HEADER));
            let mut mid = ui.new_child(UiBuilder::new().max_rect(mid_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
            {
                let ui = &mut mid;
                ui.spacing_mut().item_spacing = Vec2::new(0.0, 0.0);
                let playing = app.playing.is_some();
                let restart = icon_button(ui, t, false, PLAY_ON, "Restart: play from the beginning", |p, r, c| {
                    p.rect_filled(Rect::from_center_size(r.center() - Vec2::new(6.0, 0.0), Vec2::new(2.0, 10.0)), 0.0, c);
                    triangle(p, r.center() + Vec2::new(2.0, 0.0), 5.0, c);
                });
                note_rect(ui, "restart", restart.rect);
                if restart.clicked() {
                    acts.push(Act::Restart);
                }
                // (While MIDI clock leads, it starts and stops the song: PLAY is greyed.)
                let receiving = app.receiving_clock();
                let play_tip = if receiving { "Play: the external MIDI clock starts and stops the song" } else { "Play (Space)" };
                let play = ui.add_enabled_ui(!receiving, |ui| {
                    icon_button(ui, t, playing, PLAY_ON, play_tip, |p, r, c| triangle(p, r.center(), 6.0, c))
                });
                note_rect(ui, "play", play.inner.rect);
                if play.inner.clicked() {
                    acts.push(Act::Play);
                }
                if icon_button(ui, t, false, PLAY_ON, "Stop", |p, r, c| {
                    p.rect_filled(Rect::from_center_size(r.center(), Vec2::splat(10.0)), 1.0, c);
                })
                .clicked()
                {
                    acts.push(Act::Stop);
                }
                let rec = icon_button(
                    ui,
                    t,
                    app.recording,
                    REC_ON,
                    "Record: notes played go into the patterns playing. Stopped, it arms (PLAY starts it); shift-click counts in first",
                    |p, r, c| {
                        p.circle_filled(r.center(), 5.5, c);
                    },
                );
                note_rect(ui, "record", rec.rect);
                if rec.clicked() {
                    acts.push(if ui.input(|i| i.modifiers.shift) { Act::RecordCountIn } else { Act::Record });
                }
                if icon_button(
                    ui,
                    t,
                    app.auto_write,
                    REC_ON,
                    "Auto-write (MK3: AUTO). Red: on, knob moves while the song plays are recorded as modulation, \
                     overwriting what's there while the knob is touched. Grey: off, knobs just change their value \
                     (a drag started on a knob's ring still records). Right-click a knob to remove its modulation.",
                    |p, r, c| {
                        // A bullseye: two rings and a dot.
                        let o = r.center();
                        p.circle_stroke(o, 6.5, Stroke::new(1.3, c));
                        p.circle_stroke(o, 3.8, Stroke::new(1.3, c));
                        p.circle_filled(o, 1.5, c);
                    },
                )
                .clicked()
                {
                    acts.push(Act::ToggleAutoWrite);
                }
                let loop_tip = "Loop: the song starts over at its end (scenes always loop)";
                let looping = icon_button(ui, t, app.looping, PLAY_ON, loop_tip, |p, r, c| {
                    // A loop: a rounded frame, an arrowhead on its top edge.
                    let b = Rect::from_center_size(r.center(), Vec2::new(14.0, 9.0));
                    p.rect_stroke(b, 3.0, Stroke::new(1.5, c), StrokeKind::Middle);
                    let tip = Pos2::new(b.center().x + 2.5, b.top());
                    p.add(egui::Shape::convex_polygon(
                        vec![tip + Vec2::new(0.0, -3.5), tip + Vec2::new(3.5, 0.0), tip + Vec2::new(0.0, 3.5)],
                        c,
                        Stroke::NONE,
                    ));
                });
                note_rect(ui, "loop", looping.rect);
                if looping.clicked() {
                    acts.push(Act::ToggleLoop);
                }
                let link = link_button(ui, app.link_view());
                note_rect(ui, "link", link.rect);
                if link.clicked() {
                    acts.push(Act::ToggleLink);
                }
                ui.add_space(METRONOME_GAP);
                let metronome = icon_button(ui, t, app.metronome, PLAY_ON, "Metronome (SHIFT + TAP)", |p, r, c| {
                    // A metronome: its body, and the pendulum leaning over.
                    let (b, w) = (r.center().y + 6.0, r.center().y - 7.0);
                    let body = [Pos2::new(r.center().x - 2.5, w), Pos2::new(r.center().x + 2.5, w)];
                    let s = Stroke::new(1.5, c);
                    p.line_segment([body[0], Pos2::new(r.center().x - 6.0, b)], s);
                    p.line_segment([body[1], Pos2::new(r.center().x + 6.0, b)], s);
                    p.line_segment([body[0], body[1]], s);
                    p.line_segment([Pos2::new(r.center().x - 6.0, b), Pos2::new(r.center().x + 6.0, b)], s);
                    p.line_segment([Pos2::new(r.center().x, b - 2.0), Pos2::new(r.center().x + 5.0, w + 1.0)], s);
                });
                note_rect(ui, "metronome", metronome.rect);
                if metronome.clicked() {
                    acts.push(Act::ToggleMetronome);
                }
                ui.add_space(DISPLAY_GAP);
                if let Some(doc) = app.doc.clone() {
                    display_box(ui, app, doc.borrow().project(), &mut acts);
                    ui.add_space(DISPLAY_GAP);
                }
                let tip = if app.follow { "Follow playback: on (the focus moves with the playing scene)" } else { "Follow playback: off" };
                let follow = icon_button(ui, t, app.follow, white(245), tip, |p, r, c| {
                    // An arrow on to dots: the view following the play position.
                    let y = r.center().y;
                    p.line_segment([Pos2::new(r.center().x - 8.0, y), Pos2::new(r.center().x + 1.0, y)], Stroke::new(1.5, c));
                    triangle(p, Pos2::new(r.center().x + 1.0, y), 4.0, c);
                    for i in 0..3 {
                        p.circle_filled(Pos2::new(r.center().x + 5.0 + i as f32 * 3.0, y), 1.0, c);
                    }
                });
                note_rect(ui, "follow", follow.rect);
                if follow.clicked() {
                    acts.push(Act::ToggleFollow);
                }
                let keys_tip = if app.keys.on {
                    format!("Computer keyboard: playing notes ({}) (⌘K)", app.keys.summary())
                } else {
                    "Computer keyboard: play notes with the letter keys (⌘K)".to_string()
                };
                let keys = icon_button(ui, t, app.keys.on, white(245), &keys_tip, |p, r, c| {
                    // A keyboard: its outline, two rows of keys and the space bar.
                    let b = Rect::from_center_size(r.center(), Vec2::new(18.0, 12.0));
                    p.rect_stroke(b, 2.0, Stroke::new(1.2, c), StrokeKind::Inside);
                    for row in 0..2 {
                        for k in 0..5 {
                            let at = Pos2::new(b.left() + 3.5 + k as f32 * 2.75, b.top() + 3.5 + row as f32 * 2.5);
                            p.rect_filled(Rect::from_center_size(at, Vec2::splat(1.4)), 0.0, c);
                        }
                    }
                    p.hline((b.left() + 5.0)..=(b.right() - 5.0), b.bottom() - 3.0, Stroke::new(1.2, c));
                });
                note_rect(ui, "keys", keys.rect);
                if keys.clicked() {
                    acts.push(Act::ToggleKeys);
                }
            }
            let middle_end = mid_rect.right();
            // Right, as Maschine's: the controller view, the master fader, the CPU meter.
            let r = Rect::from_min_max(Pos2::new(middle_end + 8.0, full.top()), full.right_bottom());
            let mut right = ui.new_child(UiBuilder::new().max_rect(r).layout(egui::Layout::right_to_left(egui::Align::Center)));
            {
                let ui = &mut right;
                ui.spacing_mut().item_spacing = Vec2::new(0.0, 0.0);
                let cpu = cpu_meter(ui, app.output.as_ref().is_some_and(|o| o.is_ok()).then_some(app.cpu_load));
                note_rect(ui, "cpu", cpu.rect);
                ui.add_space(10.0);
                if let Some(doc) = &app.doc {
                    let level = doc.borrow().project().master_level;
                    let master = master_fader(ui, level, app.master_meter, &mut acts);
                    note_rect(ui, "master", master.rect);
                    ui.add_space(10.0);
                }
                if app.controller.is_some() {
                    let on = app.show_controller;
                    if icon_button(ui, Vec2::new(32.0, 24.0), on, white(245), "Show or hide the controller", |p, r, c| {
                        for i in 0..4 {
                            let x = r.center().x - 7.5 + (i % 2) as f32 * 9.0;
                            let y = r.center().y - 7.5 + (i / 2) as f32 * 9.0;
                            p.rect_stroke(
                                Rect::from_min_size(Pos2::new(x, y), Vec2::splat(6.0)),
                                1.0,
                                Stroke::new(1.0, c),
                                StrokeKind::Inside,
                            );
                        }
                    })
                    .clicked()
                    {
                        acts.push(Act::ToggleController);
                    }
                    ui.add_space(8.0);
                }
                if let Some(e) = &app.error {
                    let path = app.path.as_deref().map(|p| p.display().to_string()).unwrap_or_default();
                    ui.colored_label(REC_ON, format!("{path}: {e}"));
                }
            }
        });
    apply(app, acts);
}

/// The header's LINK button's width.
const LINK_WIDTH: f32 = 44.0;

/// The header's display (BPM, SWING, SIG, position, scene), and its gap either side;
/// the gap before the metronome.
pub(crate) const DISPLAY_SIZE: Vec2 = Vec2::new(430.0, 27.0);
pub(crate) const DISPLAY_GAP: f32 = 6.0;
const METRONOME_GAP: f32 = 8.0;

/// The header's master fader's width.
pub(crate) const MASTER_FADER_W: f32 = 100.0;

/// The master level: the master meter's left and right, with a white line across them
/// at the level (the stored level along them, 0 dB at three quarters), which is what's
/// dragged (Shift: fine); double-click: 0 dB.
fn master_fader(ui: &mut Ui, level: f64, meter: (f32, f32), acts: &mut Vec<Act>) -> Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(MASTER_FADER_W, HEADER), Sense::click_and_drag());
    let resp = resp.on_hover_text(format!("Master: {} (drag; Shift: fine; double-click: 0 dB)", db(level)));
    let p = ui.painter();
    let bars = Rect::from_center_size(r.center(), Vec2::new(r.width(), 10.0));
    let x = |v: f64| bars.left() + bars.width() * v.clamp(0.0, 1.0) as f32;
    for (k, a) in [meter.0, meter.1].into_iter().enumerate() {
        let m = Rect::from_min_size(Pos2::new(bars.left(), bars.top() + k as f32 * 6.0), Vec2::new(bars.width(), 4.0));
        p.rect_filled(m, 1.0, white(28));
        if a > 1e-5 {
            let v = deckard_model::level_from_db(20.0 * f64::from(a).log10());
            let col = if a >= 1.0 { REC_ON } else { PLAY_ON };
            p.rect_filled(Rect::from_min_max(m.min, Pos2::new(x(v), m.bottom())), 1.0, col);
        }
    }
    // 0 dB: a tick under the meters.
    p.vline(x(0.75), (bars.bottom() + 2.0)..=(bars.bottom() + 4.0), Stroke::new(1.0, white(90)));
    // The level: a white line through both.
    let lit = resp.hovered() || resp.dragged();
    let line = Rect::from_center_size(Pos2::new(x(level), bars.center().y), Vec2::new(if lit { 3.0 } else { 2.0 }, bars.height() + 8.0));
    p.rect_filled(line, 1.0, if lit { Color32::WHITE } else { white(230) });
    if resp.double_clicked() {
        acts.push(Act::SetLevel(Target::Song, 0.75));
    } else if resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let dx = resp.drag_delta().x / if fine { 10.0 } else { 1.0 };
        if dx != 0.0 {
            acts.push(Act::SetLevel(Target::Song, (level + f64::from(dx / bars.width())).clamp(0.0, 1.0)));
        }
    }
    resp
}

/// The audio thread's load (0..1): its busy time over the audio's time between two
/// readings of (busy, audio) µs; `None` if no audio played between them.
pub(crate) fn cpu_load(prev: (u64, u64), now: (u64, u64)) -> Option<f32> {
    let audio = now.1.saturating_sub(prev.1);
    (audio > 0).then(|| (now.0.saturating_sub(prev.0) as f32 / audio as f32).min(1.0))
}

/// The CPU meter: the audio thread's load as a bar over "CPU" (`None`: no audio out).
fn cpu_meter(ui: &mut Ui, load: Option<f32>) -> Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(30.0, HEADER), Sense::hover());
    let tip = match load {
        Some(l) => format!("CPU: the audio is {:.0} % of the time it has to play in", l * 100.0),
        None => "CPU: no audio output".to_string(),
    };
    let resp = resp.on_hover_text(tip);
    let p = ui.painter();
    let bar = Rect::from_center_size(r.center() - Vec2::new(0.0, 5.0), Vec2::new(r.width() - 4.0, 5.0));
    p.rect_filled(bar, 1.0, white(28));
    if let Some(l) = load.filter(|l| *l > 0.0) {
        let col = if l > 0.9 {
            REC_ON
        } else if l > 0.7 {
            Color32::from_rgb(0xF0, 0xA0, 0x30)
        } else {
            white(200)
        };
        p.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * l.min(1.0), bar.height())), 1.0, col);
    }
    p.text(r.center() + Vec2::new(0.0, 6.0), Align2::CENTER_CENTER, "CPU", font(8.0), white(142));
    resp
}

/// The id under which the header keeps where it drew `name` (tests find it there).
/// A note with expression (per-note pressure, slide or pitch) is marked in the grid.
pub(crate) fn has_expression_mark(n: &Note) -> bool {
    n.expr.is_some()
}

pub(crate) fn header_rect(name: &str) -> egui::Id {
    egui::Id::new(("deckard-header", name))
}

fn note_rect(ui: &Ui, name: &str, rect: Rect) {
    ui.ctx().data_mut(|d| d.insert_temp(header_rect(name), rect));
}

/// LINK: lit while in the session (`view`: its peers, and the phase in the bar), with a
/// bar under the word filling over each bar of the session, as Maschine's does.
fn link_button(ui: &mut Ui, view: Option<(u64, f64)>) -> Response {
    let tip = match view {
        None => "Ableton Link: join a session with other applications on the network".to_string(),
        Some((0, _)) => "Ableton Link: in the session, alone so far".to_string(),
        Some((1, _)) => "Ableton Link: in a session with 1 other application".to_string(),
        Some((n, _)) => format!("Ableton Link: in a session with {n} other applications"),
    };
    icon_button(ui, Vec2::new(LINK_WIDTH, 24.0), view.is_some(), PLAY_ON, &tip, |p, r, c| {
        p.text(r.center() - Vec2::new(0.0, 2.0), Align2::CENTER_CENTER, "LINK", font(10.0), c);
        if let Some((_, phase)) = view {
            let bar = Rect::from_min_size(Pos2::new(r.left() + 7.0, r.bottom() - 6.0), Vec2::new(r.width() - 14.0, 2.0));
            p.rect_filled(bar, 0.0, white(40));
            p.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * phase.clamp(0.0, 1.0) as f32, 2.0)), 0.0, c);
        }
    })
}

fn separator(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(3.0, HEADER), Sense::hover());
    ui.painter().vline(r.center().x, r.y_range(), Stroke::new(1.0, Color32::BLACK));
}

/// The rounded display: tempo, signature, position and the scene name.
fn display_box(ui: &mut Ui, app: &App, p: &Project, acts: &mut Vec<Act>) {
    let head = app.shown_playhead();
    let pos = head.map_or(0.0, |h| h.position);
    let (scene, scene_col) = match head.map(|h| h.mode) {
        Some(Mode::Arrangement) => match section_at(p, pos).and_then(|i| p.sections[i].scene) {
            Some(s) if s < p.scenes.len() => (scene_name(p, s), scene_color(p, s)),
            _ => (String::new(), white(142)),
        },
        Some(Mode::Scene(s)) if s < p.scenes.len() => (scene_name(p, s), scene_color(p, s)),
        _ if app.scene < p.scenes.len() => (scene_name(p, app.scene), scene_color(p, app.scene)),
        _ => (String::new(), white(142)),
    };
    let (rect, _) = ui.allocate_exact_size(DISPLAY_SIZE, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 5.0, Color32::from_gray(10));
    // The cells, left to right, each with a divider after it.
    let mut x = rect.left() + 4.0;
    let mut cell = |w: f32| {
        let r = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(w, rect.height()));
        x += w;
        painter.vline(x, rect.y_range().shrink(5.0), Stroke::new(1.0, S03));
        r
    };
    let (bpm, swing, sig, position) = (cell(96.0), cell(100.0), cell(64.0), cell(66.0));
    let scene_cell = Rect::from_min_max(Pos2::new(x + 2.0, rect.top() + 2.0), Pos2::new(rect.right() - 4.0, rect.bottom() - 2.0));
    let y = rect.center().y;
    // A value, then its caption (as "120.00 BPM").
    let value = |r: Rect, v: &str, caption: &str| {
        let t = painter.text(Pos2::new(r.left() + 8.0, y), Align2::LEFT_CENTER, v, font(12.0), white(245));
        painter.text(Pos2::new(t.right() + 6.0, y), Align2::LEFT_CENTER, caption, font(10.0), white(142));
    };
    // A value dragged up and down (Shift: fine), double-click: its default.
    let drag = |ui: &mut Ui, r: Rect, name: &str, tip: &str| {
        note_rect(ui, name, r);
        let resp = ui.interact(r, ui.id().with(name), Sense::click_and_drag()).on_hover_text(tip);
        if resp.hovered() || resp.dragged() {
            painter.rect_filled(r.shrink(3.0), 3.0, white(16));
        }
        let fine = ui.input(|i| i.modifiers.shift);
        let dy = if resp.dragged() { -resp.drag_delta().y as f64 } else { 0.0 };
        (dy, fine, resp.double_clicked())
    };
    let (dy, fine, reset) = drag(ui, bpm, "bpm", "Drag to change the tempo (Shift: fine), double-click for 120");
    if reset {
        acts.push(Act::SetTempo(120.0));
    } else if dy != 0.0 {
        acts.push(Act::SetTempo(((p.tempo + dy * if fine { 0.01 } else { 0.25 }) * 100.0).round() / 100.0));
    }
    value(bpm, &format!("{:.2}", p.tempo), "BPM");
    let (dy, fine, reset) = drag(ui, swing, "swing", "The master swing: drag to change it (Shift: fine), double-click for 0 %");
    if reset {
        acts.push(Act::SetSwing(Target::Song, Swing { amount: 0.0, ..p.swing }));
    } else if dy != 0.0 {
        let amount = ((p.swing.amount + dy * if fine { 0.05 } else { 0.5 }) * 10.0).round() / 10.0;
        acts.push(Act::SetSwing(Target::Song, Swing { amount: amount.clamp(0.0, 100.0), ..p.swing }));
    }
    value(swing, &format!("{:.1} %", p.swing.amount), "SWING");
    // The time signature: a menu.
    note_rect(ui, "sig", sig);
    menu_at(ui, sig.shrink2(Vec2::new(4.0, 2.0)), format!("{} / {}  SIG", p.numerator, p.denominator), |ui| {
        time_signature_menu(ui, p, acts)
    });
    painter.text(Pos2::new(position.left() + 8.0, y), Align2::LEFT_CENTER, position_string(p, pos), font(12.0), white(245));
    label(&painter, scene_cell, 2.0, &scene, 12.0, scene_col);
}

/// A menu button in a rectangle of a custom-drawn panel.
fn menu_at(ui: &mut Ui, rect: Rect, text: String, add: impl FnOnce(&mut Ui)) -> Response {
    // A child placed at `rect` that takes no room in the parent's layout.
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().button_padding = Vec2::new(4.0, 1.0);
    let text = egui::RichText::new(text).size(12.0).color(Color32::WHITE);
    child.menu_button(text, add).response
}

/// [`menu_at`] for settings: it stays open while they're changed, until a click outside.
fn settings_menu_at(ui: &mut Ui, rect: Rect, text: String, add: impl FnOnce(&mut Ui)) -> Response {
    use egui::containers::menu::{MenuButton, MenuConfig};
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().button_padding = Vec2::new(4.0, 1.0);
    let text = egui::RichText::new(text).size(12.0).color(Color32::WHITE);
    let config = MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    MenuButton::new(text).config(config).ui(&mut child, add).0
}

/// PAD TOUCH (next to SCALE): what pressing a held MK3 pad does for the focused sound,
/// the settings the controller's Keyboard page has.
fn pad_touch_menu(ui: &mut Ui, at: Rect, g: usize, s: usize, sound: Option<&deckard_model::Sound>, acts: &mut Vec<Act>) {
    use deckard_model::pad_touch::{TouchMode, cc_label};
    let Some(sound) = sound else { return };
    let t = sound.pad_touch;
    let shown = match t.mode {
        TouchMode::Velocity => "PAD TOUCH".to_string(),
        TouchMode::Cc => format!("PAD: CC {}", cc_label(t.cc)),
        m => format!("PAD: {}", m.name().to_uppercase()),
    };
    settings_menu_at(ui, at, shown, |ui| {
        ui.label(egui::RichText::new(format!("Pressing a held MK3 pad, for {}", sound.name)).weak());
        for m in TouchMode::ALL {
            if ui.selectable_label(t.mode == m, m.name()).clicked() {
                acts.push(Act::PadTouch(g, s, deckard_model::pad_touch::PadTouch { mode: m, ..t }));
            }
        }
        ui.separator();
        let pressure = t.mode != TouchMode::Velocity;
        let mut n = t;
        egui::Grid::new("pad-touch").num_columns(2).show(ui, |ui| {
            ui.add_enabled_ui(t.mode == TouchMode::Cc, |ui| ui.label("CC number"));
            ui.add_enabled(
                t.mode == TouchMode::Cc,
                egui::DragValue::new(&mut n.cc).range(0..=127).custom_formatter(|v, _| cc_label(v as u8)),
            );
            ui.end_row();
            for (name, v) in [("Curve", &mut n.curve), ("Min", &mut n.min), ("Max", &mut n.max)] {
                ui.add_enabled_ui(pressure, |ui| ui.label(name));
                ui.add_enabled(pressure, egui::Slider::new(v, 0..=100).suffix(" %"));
                ui.end_row();
            }
        });
        // Min turned past Max takes Max with it, and Max turned below Min, Min.
        if n.max < n.min {
            if n.max != t.max { n.min = n.max } else { n.max = n.min }
        }
        if n != t {
            acts.push(Act::PadTouch(g, s, n));
        }
        ui.label(egui::RichText::new("The pressure modes play at Fixed Velocity; Min and Max are what's sent.").weak());
    });
}

fn time_signature_menu(ui: &mut Ui, p: &Project, acts: &mut Vec<Act>) {
    for (n, d) in [(2, 4), (3, 4), (4, 4), (5, 4), (6, 8), (7, 8), (9, 8), (12, 8)] {
        if ui.selectable_label((p.numerator, p.denominator) == (n, d), format!("{n} / {d}")).clicked() {
            acts.push(Act::TimeSignature(n, d));
            ui.close();
        }
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Beats");
        if ui.small_button("−").clicked() && p.numerator > 1 {
            acts.push(Act::TimeSignature(p.numerator - 1, p.denominator));
        }
        ui.label(p.numerator.to_string());
        if ui.small_button("+").clicked() && p.numerator < 32 {
            acts.push(Act::TimeSignature(p.numerator + 1, p.denominator));
        }
    });
    ui.horizontal(|ui| {
        ui.label("Note");
        for d in [2, 4, 8, 16] {
            if ui.selectable_label(p.denominator == d, format!("1/{d}")).clicked() {
                acts.push(Act::TimeSignature(p.numerator, d));
            }
        }
    });
}

// ---------------------------------------------------------------------------------------
// Body

pub fn body(app: &mut App, ui: &mut Ui) {
    egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK).inner_margin(3.0)).show(ui, |ui| {
        let Some(doc) = app.doc.clone() else {
            let r = ui.max_rect();
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "Drop a .mxprj file here", font(15.0), white(142));
            if let Some(e) = &app.error {
                ui.painter().text(r.center() + Vec2::new(0.0, 24.0), Align2::CENTER_CENTER, e, font(12.0), REC_ON);
            }
            return;
        };
        let d = doc.borrow();
        let project = d.project();
        // Keep the focus valid (e.g. from --group/--pattern or after opening a file).
        app.group = app.group.min(project.groups.len().saturating_sub(1));
        let n_patterns = project.groups.get(app.group).map_or(0, |g| g.patterns.len());
        app.pattern = app.pattern.min(n_patterns.saturating_sub(1));
        app.scene = app.scene.min(project.scenes.len().saturating_sub(1));

        let all = ui.max_rect();
        // Song panel heights for the small/medium/large window sizes (TopLevel.txt).
        let song_h = if all.height() > 850.0 {
            343.0
        } else if all.height() > 700.0 {
            255.0
        } else {
            167.0
        };
        let song = Rect::from_min_size(all.min, Vec2::new(all.width(), song_h));
        let chain_h = CHAIN_H + if app.remote.assign.is_some() { assign::ASSIGN_H } else { 0.0 };
        let chain = Rect::from_min_size(Pos2::new(all.left(), song.bottom() + GAP), Vec2::new(all.width(), chain_h));
        let group = Rect::from_min_max(Pos2::new(all.left(), chain.bottom() + GAP), all.max);
        // (Stopped: where STOP left it.)
        let head = app.shown_playhead();
        let mut acts = Vec::new();
        if app.mixer {
            // The Mixer replaces the work views; the chain area stays, below it.
            // The Mixer, and under it the focused sound's plug-ins as panels.
            let strip = Rect::from_min_max(Pos2::new(all.left(), all.bottom() - strip::STRIP_H), all.max);
            let mix = Rect::from_min_max(all.min, Pos2::new(all.right(), strip.top() - GAP));
            mixer::mixer(ui, app, &d, mix, &mut acts);
            strip::plugin_strip(ui, app, &d, strip, &mut acts);
            drop(d);
            apply(app, acts);
            return;
        }
        if app.lfos {
            // The LFO panel takes the Mixer's place, with the same strip below.
            let strip = Rect::from_min_max(Pos2::new(all.left(), all.bottom() - strip::STRIP_H), all.max);
            let panel = Rect::from_min_max(all.min, Pos2::new(all.right(), strip.top() - GAP));
            lfo_panel::lfo_panel(ui, app, &d, panel, &mut acts);
            strip::plugin_strip(ui, app, &d, strip, &mut acts);
            drop(d);
            apply(app, acts);
            return;
        }
        if app.song_view {
            song_view(ui, app, project, song, head, &mut acts);
        } else {
            ideas_view(ui, app, project, song, head, &mut acts);
        }
        chain_area(ui, app, &d, chain, &mut acts);
        group_panel(ui, app, project, group, head, &mut acts);
        drop(d);
        apply(app, acts);
    });
}

/// The 32-wide icon strip at the left of a panel; returns the clicked button.
/// A strip button: lit, tooltip, icon.
type StripButton<'a> = (bool, &'a str, fn(&Painter, Rect, Color32));

fn strip(ui: &mut Ui, rect: Rect, fill: Color32, top: f32, buttons: &[StripButton]) -> Option<usize> {
    ui.painter().rect_filled(rect, 0.0, fill);
    let mut clicked = None;
    for (i, (on, tip, draw)) in buttons.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(rect.left(), rect.top() + top + i as f32 * 28.0), Vec2::new(STRIP, 24.0));
        let resp = ui.interact(r, ui.id().with(("strip", rect.top() as i32, i)), Sense::click()).on_hover_text(*tip);
        if *on {
            ui.painter().rect_filled(r.shrink2(Vec2::new(3.0, 0.0)), 3.0, S03);
        } else if resp.hovered() {
            ui.painter().rect_filled(r.shrink2(Vec2::new(3.0, 0.0)), 3.0, white(20));
        }
        draw(ui.painter(), r, if *on { white(245) } else { white(143) });
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

fn ideas_icon(p: &Painter, r: Rect, c: Color32) {
    for i in 0..4 {
        let o = Vec2::new((i % 2) as f32 * 7.0 - 6.0, (i / 2) as f32 * 7.0 - 6.0);
        p.rect_filled(Rect::from_min_size(r.center() + o, Vec2::splat(5.0)), 1.0, c);
    }
}

fn song_icon(p: &Painter, r: Rect, c: Color32) {
    for (i, w) in [12.0, 8.0, 14.0].into_iter().enumerate() {
        p.rect_filled(Rect::from_min_size(r.center() + Vec2::new(-7.0, i as f32 * 5.0 - 7.0), Vec2::new(w, 3.0)), 1.0, c);
    }
}

fn grid_icon(p: &Painter, r: Rect, c: Color32) {
    for i in 0..9 {
        let o = Vec2::new((i % 3) as f32 * 5.0 - 6.0, (i / 3) as f32 * 5.0 - 6.0);
        p.rect_filled(Rect::from_min_size(r.center() + o, Vec2::splat(3.0)), 0.0, c);
    }
}

fn wave_icon(p: &Painter, r: Rect, c: Color32) {
    let pts: Vec<Pos2> =
        (0..=16).map(|i| r.center() + Vec2::new(i as f32 - 8.0, ((i as f32) * 1.3).sin() * (6.0 - (i as f32 - 8.0).abs() * 0.5))).collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.5, c)));
}

/// An eraser: a block tilted as the pencil, its end the colour, the rest outlined.
fn eraser_icon(p: &Painter, r: Rect, c: Color32) {
    let at = |x: f32, y: f32| r.center() + Vec2::new(x, y);
    let body = [at(-7.0, 3.0), at(1.0, -5.0), at(6.0, 0.0), at(-2.0, 8.0)];
    p.add(egui::Shape::closed_line(body.to_vec(), Stroke::new(1.5, c)));
    p.add(egui::Shape::convex_polygon(vec![at(-7.0, 3.0), at(-3.0, -1.0), at(2.0, 4.0), at(-2.0, 8.0)], c, Stroke::NONE));
}

fn pencil_icon(p: &Painter, r: Rect, c: Color32) {
    let (a, b) = (r.center() + Vec2::new(-6.0, 6.0), r.center() + Vec2::new(5.0, -5.0));
    p.line_segment([a, b], Stroke::new(3.0, c));
    p.circle_filled(a, 1.5, c);
}

fn piano_icon(p: &Painter, r: Rect, c: Color32) {
    let k = Rect::from_center_size(r.center(), Vec2::new(16.0, 12.0));
    p.rect_stroke(k, 1.0, Stroke::new(1.0, c), StrokeKind::Inside);
    for i in 1..4 {
        p.vline(k.left() + i as f32 * 4.0, k.y_range(), Stroke::new(1.0, c));
    }
    for i in [0.0, 1.0, 2.5] {
        p.rect_filled(Rect::from_min_size(Pos2::new(k.left() + 2.5 + i * 4.0, k.top()), Vec2::new(2.5, 7.0)), 0.0, c);
    }
}

fn view_switch(ui: &mut Ui, app: &App, rect: Rect, acts: &mut Vec<Act>) {
    let strip_rect = Rect::from_min_size(rect.min, Vec2::new(STRIP, rect.height()));
    match strip(ui, strip_rect, S05, 6.0, &[(!app.song_view, "Ideas view", ideas_icon), (app.song_view, "Song view", song_icon)]) {
        Some(0) => acts.push(Act::SongView(false)),
        Some(1) => acts.push(Act::SongView(true)),
        _ => {}
    }
}

/// Vertical scroll offset for a region, kept in egui memory and moved by the wheel
/// while hovered.
fn scroll(ui: &Ui, id: &str, region: Rect, content: f32) -> f32 {
    let id = ui.id().with(id);
    let mut off: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
    if ui.rect_contains_pointer(region) {
        off -= ui.input(|i| i.smooth_scroll_delta.y);
    }
    off = off.clamp(0.0, (content - region.height()).max(0.0));
    ui.data_mut(|d| d.insert_temp(id, off));
    off
}

/// A zooming scroll bar, as Maschine's Pattern Editor has: `win` (start, length) of
/// `total` is shown. Drag the bar along to scroll; across to zoom, centred where it was
/// grabbed; its ends to zoom with the other end held; double-click it to show it all.
/// Without `zoom` it only scrolls. The new window, if it changed.
#[allow(clippy::too_many_arguments)]
fn zoom_bar(
    ui: &mut Ui,
    painter: &Painter,
    id: egui::Id,
    track: Rect,
    horizontal: bool,
    total: f32,
    win: (f32, f32),
    min_len: f32,
    zoom: bool,
) -> Option<(f32, f32)> {
    let along = |p: Pos2| if horizontal { p.x } else { p.y };
    let across = |p: Pos2| if horizontal { p.y } else { p.x };
    let (t0, t1) = if horizontal { (track.left(), track.right()) } else { (track.top(), track.bottom()) };
    let per = (t1 - t0) / total.max(f32::EPSILON);
    let a = t0 + win.0 * per;
    let b = (a + win.1 * per).max(a + 12.0).min(t1);
    let span = |lo: f32, hi: f32| {
        if horizontal {
            Rect::from_min_max(Pos2::new(lo, track.top() + 2.0), Pos2::new(hi, track.bottom() - 2.0))
        } else {
            Rect::from_min_max(Pos2::new(track.left() + 2.0, lo), Pos2::new(track.right() - 2.0, hi))
        }
    };
    painter.rect_filled(track, 0.0, S00);
    let thumb = span(a, b);
    let h = if zoom { 6.0_f32.min((b - a) / 3.0) } else { 0.0 };
    #[derive(Clone, Copy, PartialEq)]
    enum Part {
        Body,
        Low,
        High,
    }
    let mut parts = vec![(Part::Body, span(a + h, b - h))];
    if zoom {
        parts.push((Part::Low, span(a, a + h)));
        parts.push((Part::High, span(b - h, b)));
    }
    let clamp = |(s, l): (f32, f32)| {
        let l = l.clamp(min_len.min(total), total);
        (s.clamp(0.0, total - l), l)
    };
    let mut out = None;
    let mut lit = false;
    for (part, r) in parts {
        let resp = ui.interact(r, id.with(part as u8), Sense::click_and_drag());
        lit |= resp.hovered() || resp.dragged();
        if resp.hovered() && part != Part::Body {
            ui.ctx().set_cursor_icon(if horizontal { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
        }
        if resp.drag_started() {
            ui.data_mut(|d| d.insert_temp(id, win));
        }
        if resp.double_clicked() && part == Part::Body && zoom {
            out = Some((0.0, total));
        }
        if resp.dragged()
            && let Some((s0, l0)) = ui.data(|d| d.get_temp::<(f32, f32)>(id))
            && let (Some(from), Some(now)) = ui.input(|i| (i.pointer.press_origin(), i.pointer.latest_pos()))
        {
            let d = (along(now) - along(from)) / per;
            out = Some(clamp(match part {
                Part::Body if zoom => {
                    // Up (left, on the vertical bar) zooms in round the content under where
                    // the bar was grabbed; along scrolls.
                    let l = (l0 * ((across(now) - across(from)) * 0.012).exp()).clamp(min_len.min(total), total);
                    let anchor = ((along(from) - t0) / per).clamp(s0, s0 + l0);
                    let rel = if l0 > 0.0 { (anchor - s0) / l0 } else { 0.0 };
                    (anchor - rel * l + d, l)
                }
                Part::Body => (s0 + d, l0),
                Part::Low => {
                    let s = (s0 + d).min(s0 + l0 - min_len);
                    (s, s0 + l0 - s)
                }
                Part::High => (s0, l0 + d),
            }));
        }
    }
    painter.rect_filled(thumb, 3.0, if lit { white(70) } else { white(40) });
    if zoom {
        for r in [span(a, a + h), span(b - h, b)] {
            painter.rect_filled(r.shrink(1.0), 2.0, if lit { white(150) } else { white(90) });
        }
    }
    out.filter(|w| *w != win)
}

/// Horizontal scroll offset, moved by horizontal wheel/trackpad motion.
fn hscroll(ui: &Ui, id: &str, region: Rect, content: f32) -> f32 {
    let id = ui.id().with(id);
    let mut off: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
    if ui.rect_contains_pointer(region) {
        off -= ui.input(|i| i.smooth_scroll_delta.x);
    }
    off = off.clamp(0.0, (content - region.width()).max(0.0));
    ui.data_mut(|d| d.insert_temp(id, off));
    off
}

// ---------------------------------------------------------------------------------------
// Ideas view (IdeaSpace.txt): scene tabs on top, a column of patterns per group, group
// tiles at the bottom.

fn ideas_view(ui: &mut Ui, app: &App, p: &Project, rect: Rect, head: Option<Playhead>, acts: &mut Vec<Act>) {
    view_switch(ui, app, rect, acts);
    let area = Rect::from_min_max(Pos2::new(rect.left() + STRIP + 1.0, rect.top()), rect.max);
    let painter = ui.painter_at(area);
    painter.rect_filled(area, 0.0, S00);
    let time = ui.input(|i| i.time);
    let playing_scene = head.and_then(|h| match h.mode {
        Mode::Scene(s) => Some(s),
        _ => None,
    });
    let queued_scene = head.and_then(|h| match h.queued {
        Some(Mode::Scene(s)) => Some(s),
        _ => None,
    });

    // Scene tabs.
    let tabs = Rect::from_min_size(area.min, Vec2::new(area.width(), SCENE_ROW));
    painter.rect_filled(tabs, 0.0, S05);
    let sx = hscroll(ui, "scene-tabs", tabs, p.scenes.len() as f32 * IDEA_W + 27.0);
    for s in 0..p.scenes.len() {
        let r = Rect::from_min_size(Pos2::new(tabs.left() + s as f32 * IDEA_W - sx, tabs.top()), Vec2::new(IDEA_W + 1.0, SCENE_ROW));
        if r.right() < tabs.left() || r.left() > tabs.right() {
            continue;
        }
        let resp = ui.interact(r.intersect(tabs), ui.id().with(("scene", s)), Sense::click_and_drag());
        ui.data_mut(|d| d.insert_temp(egui::Id::new(("deckard-scene-tab", s)), r));
        // Drag a tab to move the scene: a line shows where it goes.
        if (resp.dragged() || resp.drag_stopped())
            && let Some(pos) = ui.ctx().pointer_latest_pos()
        {
            let to = (((pos.x - tabs.left() + sx) / IDEA_W).floor().max(0.0) as usize).min(p.scenes.len() - 1);
            if resp.drag_stopped() {
                if to != s {
                    acts.push(Act::MoveScene(s, to));
                }
            } else if to != s {
                let x = tabs.left() + (if to > s { to + 1 } else { to }) as f32 * IDEA_W - sx;
                ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.id().with("scene-drop"))).vline(
                    x,
                    tabs.y_range(),
                    Stroke::new(2.0, Color32::WHITE),
                );
            }
        }
        let col = scene_color(p, s);
        let focused = s == app.scene;
        // A launched scene waiting for the playing one to end blinks.
        let blink = queued_scene == Some(s) && (time * 4.0).sin() < 0.0;
        let fill = if focused && !blink {
            col
        } else if resp.hovered() {
            S03
        } else {
            S01
        };
        let inner = r.shrink2(Vec2::new(0.5, 4.0));
        painter.rect_filled(inner, 2.0, fill);
        if queued_scene == Some(s) {
            painter.rect_stroke(inner, 2.0, Stroke::new(1.0, col), StrokeKind::Inside);
            ui.ctx().request_repaint();
        }
        let text = if focused && !blink { black(245) } else { col };
        let mut text_rect = inner;
        if playing_scene == Some(s) {
            text_rect.max.y -= 4.0;
            triangle(&painter, Pos2::new(inner.left() + 10.0, text_rect.center().y), 4.0, text);
            text_rect.min.x += 12.0;
            // How far through the scene: a segment per bar (up to 16), filled as it
            // plays, like the group tiles' pattern progress.
            if let Some(h) = head {
                let len = p.scene_length(s) as f64;
                let bars = (len / p.bar_length() as f64).ceil().clamp(1.0, 16.0) as usize;
                let played = (h.position % len) / len * bars as f64;
                let gap = if bars > 8 { 1.0 } else { 2.0 };
                let strip = Rect::from_min_size(Pos2::new(inner.left() + 8.0, inner.bottom() - 6.0), Vec2::new(inner.width() - 16.0, 3.0));
                let (filled, unfilled) = if focused && !blink { (black(245), black(61)) } else { (col, with_alpha(col, 64)) };
                progress_strip(&painter, strip, bars, gap, played, filled, unfilled);
                ui.ctx().request_repaint();
            }
        }
        if app.renaming == Some(Target::Scene(s)) {
            rename_editor(ui, inner.shrink2(Vec2::new(2.0, 1.0)).intersect(tabs), Target::Scene(s), &p.scenes[s].name, acts);
            continue;
        }
        label(&painter, text_rect.shrink2(Vec2::new(4.0, 0.0)), 2.0, &scene_name(p, s), 12.0, text);
        if resp.double_clicked() {
            acts.push(Act::StartRename(Target::Scene(s)));
        } else if resp.clicked() {
            acts.push(Act::LaunchScene(s));
        }
        let resp = resp.on_hover_text(
            "Click to select the scene (while a scene plays, it switches when the playing scene ends); double-click to rename; right-click for more",
        );
        resp.context_menu(|ui| scene_menu(ui, p, s, acts));
    }
    // Focus line.
    painter.rect_filled(Rect::from_min_size(Pos2::new(area.left(), tabs.bottom()), Vec2::new(area.width(), 3.0)), 0.0, S03);

    // Patterns, one column per group, and the group tiles.
    let tiles = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - TILE_H), area.max);
    ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-tiles"), tiles));
    let cells = Rect::from_min_max(Pos2::new(area.left(), tabs.bottom() + 4.0), Pos2::new(area.right(), tiles.top() - 1.0));
    painter.rect_filled(cells, 0.0, S05);
    painter.rect_filled(tiles, 0.0, S05);
    let columns = Rect::from_min_max(cells.min, tiles.max);
    let gx = hscroll(ui, "idea-columns", columns, p.groups.len() as f32 * IDEA_W + 27.0);
    let most = p.groups.iter().map(|g| g.patterns.len()).max().unwrap_or(0);
    let gy = scroll(ui, "idea-cells", cells, most as f32 * ROW + 8.0);
    let in_scene = |g: usize, pi: usize| p.scenes.get(app.scene).is_some_and(|s| s.patterns.contains(&(g, pi)));
    let cell_painter = painter.with_clip_rect(cells);
    for (g, group) in p.groups.iter().enumerate() {
        let x = cells.left() + g as f32 * IDEA_W - gx;
        if x + IDEA_W < cells.left() || x > cells.right() {
            continue;
        }
        let col_rect = Rect::from_min_max(Pos2::new(x, cells.top()), Pos2::new(x + IDEA_W, cells.bottom()));
        if g == app.group {
            cell_painter.rect_filled(col_rect, 0.0, white(10));
        }
        for pi in 0..group.patterns.len() {
            let r = Rect::from_min_size(Pos2::new(x, cells.top() + 4.0 + pi as f32 * ROW - gy), Vec2::new(IDEA_W - 1.0, ROW));
            if !r.intersects(cells) {
                continue;
            }
            let resp = ui.interact(r.intersect(cells), ui.id().with(("cell", g, pi)), Sense::click());
            let shape = r.shrink2(Vec2::new(3.0, 2.0));
            let col = pattern_color(p, g, pi);
            let assigned = in_scene(g, pi);
            // Patterns of the focused scene are lit; the group's other patterns are dark.
            cell_painter.rect_filled(shape, 3.0, if assigned { col } else { dim(col, 0.3) });
            if g == app.group && pi == app.pattern {
                cell_painter.rect_stroke(shape, 3.0, Stroke::new(2.0, white(235)), StrokeKind::Outside);
            } else if resp.hovered() {
                cell_painter.rect_stroke(shape, 3.0, Stroke::new(1.0, white(41)), StrokeKind::Outside);
            }
            if let Some(t) = pattern_position(p, head, g, pi) {
                // Playing: the part played so far is darker, and a playhead line.
                let f = (t / pattern_len(p, g, pi) as f64) as f32;
                let x = shape.left() + shape.width() * f;
                cell_painter.rect_filled(Rect::from_min_max(shape.min, Pos2::new(x, shape.bottom())), 3.0, black(70));
                cell_painter.vline(x, shape.y_range(), Stroke::new(2.0, Color32::WHITE));
            }
            let name = &group.patterns[pi].name;
            let ink = if assigned { black(245) } else { col };
            label(&cell_painter, shape.shrink2(Vec2::new(4.0, 0.0)), 3.0, if name.is_empty() { "Pattern" } else { name }, 12.0, ink);
            if app.renaming == Some(Target::Pattern(g, pi)) {
                rename_editor(ui, shape.shrink2(Vec2::new(2.0, 2.0)).intersect(cells), Target::Pattern(g, pi), name, acts);
                continue;
            }
            // A click puts the pattern in the focused scene (replacing the group's other
            // pattern there) and shows it in the pattern editor; a double-click on a
            // pattern that was in the scene takes it out. (The first click of a
            // double-click on a pattern that wasn't in puts it in; the second leaves it.)
            let just_in = egui::Id::new("deckard-just-assigned");
            if resp.double_clicked() {
                let added = ui.data(|d| d.get_temp::<(usize, usize)>(just_in)) == Some((g, pi));
                ui.data_mut(|d| d.remove::<(usize, usize)>(just_in));
                if assigned && !added && !p.scenes.is_empty() {
                    acts.push(Act::AssignPattern(app.scene, g, None));
                }
            } else if resp.clicked() {
                if !assigned {
                    // No scenes (all deleted): the click makes Scene 1 with it in.
                    let scene = if p.scenes.is_empty() { 0 } else { app.scene };
                    acts.push(Act::AssignPattern(scene, g, Some(pi)));
                    ui.data_mut(|d| d.insert_temp(just_in, (g, pi)));
                } else {
                    ui.data_mut(|d| d.remove::<(usize, usize)>(just_in));
                }
                acts.push(Act::FocusPattern(g, pi));
            }
            let tip = if assigned {
                "Click to show this pattern; double-click to take it out of the scene"
            } else {
                "Click to use this pattern in the scene and show it"
            };
            let resp = resp.on_hover_text(format!("{tip}; right-click for more"));
            if resp.secondary_clicked() {
                acts.push(Act::FocusPattern(g, pi));
            }
            resp.context_menu(|ui| pattern_menu(ui, p, g, pi, acts));
        }
        // Below the group's patterns: double-click for a new one.
        let free_top = cells.top() + 4.0 + group.patterns.len() as f32 * ROW - gy;
        let free = Rect::from_min_max(Pos2::new(x, free_top.max(cells.top())), Pos2::new(x + IDEA_W - 1.0, cells.bottom()));
        if free.height() > 0.0 {
            let resp = ui.interact(free, ui.id().with(("new-pattern", g)), Sense::click());
            if resp.hovered() {
                let slot = Rect::from_min_size(Pos2::new(x, free_top), Vec2::new(IDEA_W - 1.0, ROW)).shrink2(Vec2::new(3.0, 2.0));
                cell_painter.rect_stroke(slot, 3.0, Stroke::new(1.0, white(41)), StrokeKind::Inside);
            }
            if resp.on_hover_text("Double-click for a new pattern in this group").double_clicked() {
                acts.push(Act::NewPattern(app.scene, g));
            }
        }
        // Group tile.
        let tile = Rect::from_min_size(Pos2::new(x, tiles.top()), Vec2::new(IDEA_W - 1.0, TILE_H)).shrink(1.0);
        let resp = ui.interact(tile.intersect(tiles), ui.id().with(("tile", g)), Sense::click());
        note_rect(ui, &format!("group tile {g}"), tile);
        let gcol = if group.muted { Color32::from_gray(0x7D) } else { group_color(p, g) };
        let focused = g == app.group;
        let tp = painter.with_clip_rect(tiles);
        if focused {
            tp.rect_filled(tile, 2.0, gcol);
        } else if resp.hovered() {
            tp.rect_filled(tile, 2.0, white(20));
        }
        let ink = if focused { black(245) } else { gcol };
        let letter = Pos2::new(tile.center().x, tile.top() + 12.0);
        let id = group_tab(g);
        tp.text(letter, Align2::CENTER_CENTER, &id, FontId::proportional(16.0), ink);
        tp.text(letter + Vec2::new(0.4, 0.0), Align2::CENTER_CENTER, &id, FontId::proportional(16.0), ink);
        let name_rect = Rect::from_min_size(Pos2::new(tile.left() + 6.0, tile.top() + 20.0), Vec2::new(86.0, 20.0));
        let name_ink = if focused {
            black(245)
        } else if group.muted {
            white(142)
        } else {
            white(245)
        };
        if app.renaming == Some(Target::Group(g)) {
            rename_editor(ui, name_rect.intersect(tiles), Target::Group(g), &group.name, acts);
        } else {
            tp.with_clip_rect(name_rect.intersect(tiles)).text(
                name_rect.center(),
                Align2::CENTER_CENTER,
                group_name(p, g),
                font(12.0),
                name_ink,
            );
        }
        // Progress: the scene's pattern for this group, up to 8 segments.
        let strip = Rect::from_min_size(Pos2::new(tile.center().x - 43.0, tile.bottom() - 9.0), Vec2::new(86.0, 4.0));
        match group_progress(p, head, app.scene, g) {
            Some((bars, played)) => {
                let (filled, unfilled) = if focused { (black(245), black(61)) } else { (gcol, with_alpha(gcol, 64)) };
                progress_strip(&tp, strip, bars, 2.0, played, filled, unfilled);
            }
            None => {
                tp.rect_filled(strip, 0.0, if focused { black(61) } else { S01 });
            }
        }
        // A double-click mutes or unmutes the group (renaming is in its menu).
        if resp.double_clicked() {
            acts.push(Act::ToggleMute(Target::Group(g)));
        } else if resp.clicked() || resp.secondary_clicked() {
            acts.push(Act::FocusGroup(g));
        }
        resp.context_menu(|ui| group_menu(ui, app, p, g, acts));
    }
    // "+" after the last group: a new empty group (MK3 manual, "Creating Groups").
    let plus = Rect::from_min_size(Pos2::new(tiles.left() + p.groups.len() as f32 * IDEA_W - gx, tiles.top()), Vec2::new(27.0, TILE_H));
    if plus.intersects(tiles) {
        let resp = ui.interact(plus.intersect(tiles), ui.id().with("add-group"), Sense::click());
        if resp.hovered() {
            painter.rect_filled(plus.shrink2(Vec2::new(0.5, 4.0)).intersect(tiles), 2.0, S03);
        }
        painter.with_clip_rect(tiles).text(
            plus.center(),
            Align2::CENTER_CENTER,
            "+",
            font(16.0),
            white(if resp.hovered() { 245 } else { 142 }),
        );
        if resp.on_hover_text("New group").clicked() {
            acts.push(Act::AddGroup);
        }
    }
    // "+" after the last scene: a new empty scene.
    let plus = Rect::from_min_size(Pos2::new(tabs.left() + p.scenes.len() as f32 * IDEA_W - sx, tabs.top()), Vec2::new(27.0, SCENE_ROW));
    if plus.intersects(tabs) {
        let resp = ui.interact(plus.intersect(tabs), ui.id().with("add-scene"), Sense::click());
        if resp.hovered() {
            painter.rect_filled(plus.shrink2(Vec2::new(0.5, 4.0)), 2.0, S03);
        }
        painter.text(plus.center(), Align2::CENTER_CENTER, "+", font(16.0), white(if resp.hovered() { 245 } else { 142 }));
        if resp.on_hover_text("New scene").clicked() {
            acts.push(Act::AddScene);
        }
    }
}

/// Right-click menu of a scene (Ideas view tab).
fn scene_menu(ui: &mut Ui, p: &Project, s: usize, acts: &mut Vec<Act>) {
    let id = ui.id().with(("rename-scene", s));
    let mut name: String = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| p.scenes[s].name.clone());
    ui.horizontal(|ui| {
        ui.label("Name");
        let r = ui.text_edit_singleline(&mut name);
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            acts.push(Act::RenameScene(s, name.clone()));
            ui.data_mut(|d| d.remove::<String>(id));
            ui.close();
        }
    });
    ui.data_mut(|d| d.insert_temp(id, name));
    ui.separator();
    if ui.button("Duplicate").clicked() {
        acts.push(Act::DuplicateScene(s));
        ui.close();
    }
    if ui.button("Insert empty scene after").clicked() {
        acts.push(Act::InsertScene(s + 1));
        ui.close();
    }
    if ui.button("Append to arrangement").clicked() {
        acts.push(Act::AddSection(s));
        ui.close();
    }
    ui.separator();
    let last = p.scenes.len() <= 1;
    if ui
        .add_enabled(!last, egui::Button::new("Delete (and its sections)"))
        .on_disabled_hover_text("A project keeps at least one scene")
        .clicked()
    {
        acts.push(Act::DeleteScene(s));
        ui.close();
    }
}

/// Right-click menu of a section (Song view).
fn section_menu(ui: &mut Ui, p: &Project, i: usize, acts: &mut Vec<Act>) {
    let bar = p.bar_length().max(1);
    let len = p.sections[i].length;
    ui.menu_button("Scene", |ui| {
        for s in 0..p.scenes.len() {
            if ui.selectable_label(p.sections[i].scene == Some(s), scene_name(p, s)).clicked() {
                acts.push(Act::SectionScene(i, s));
                ui.close();
            }
        }
    });
    ui.menu_button(format!("Length ({} bars)", len as f64 / bar as f64), |ui| {
        if ui.button("Auto (the scene's length)").clicked() {
            acts.push(Act::SectionLength(i, None));
            ui.close();
        }
        for bars in [1, 2, 4, 8, 16, 32] {
            if ui.button(format!("{bars} bars")).clicked() {
                acts.push(Act::SectionLength(i, Some(bars * bar)));
                ui.close();
            }
        }
        ui.separator();
        if ui.button("+1 bar").clicked() {
            acts.push(Act::SectionLength(i, Some(len + bar)));
            ui.close();
        }
        if ui.add_enabled(len > bar, egui::Button::new("-1 bar")).clicked() {
            acts.push(Act::SectionLength(i, Some(len - bar)));
            ui.close();
        }
    });
    ui.separator();
    if ui.add_enabled(i > 0, egui::Button::new("Move left")).clicked() {
        acts.push(Act::MoveSection(i, i - 1));
        ui.close();
    }
    if ui.add_enabled(i + 1 < p.sections.len(), egui::Button::new("Move right")).clicked() {
        acts.push(Act::MoveSection(i, i + 1));
        ui.close();
    }
    if ui.button("Duplicate (with a copy of its scene)").clicked() {
        acts.push(Act::DuplicateSection(i));
        ui.close();
    }
    let plays = p.sections[i].scene.and_then(|s| p.scenes.get(s)).is_some_and(|s| !s.patterns.is_empty());
    if ui.add_enabled(plays, egui::Button::new("Convert to Clip(s)")).clicked() {
        acts.push(Act::ConvertSectionToClips(i));
        ui.close();
    }
    ui.separator();
    if ui.button("Delete").clicked() {
        acts.push(Act::DeleteSection(i));
        ui.close();
    }
}

// ---------------------------------------------------------------------------------------
// Song view (Arranger.txt): group list on the left; sections, timeline and tracks.

/// `c` halfway to white.
fn lighter(c: Color32) -> Color32 {
    let up = |v: u8| v + (255 - v) / 2;
    Color32::from_rgb(up(c.r()), up(c.g()), up(c.b()))
}

#[allow(clippy::too_many_arguments)]
fn group_row(
    painter: &Painter,
    rect: Rect,
    id: &str,
    name: &str,
    color: Option<Color32>,
    focused: bool,
    muted: bool,
    lit: bool,
    base: Color32,
    meter: (f32, f32),
) {
    painter.rect_filled(rect, 0.0, if focused { FOCUS_ROW } else { base });
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, black(90)));
    if let Some(c) = color {
        painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(4.0, rect.height() - 1.0)), 0.0, c);
    }
    let idr = Rect::from_min_size(Pos2::new(rect.left() + 4.0, rect.top()), Vec2::new(28.0, rect.height() - 1.0));
    // A note playing flashes the number in a lighter tint of the row's colour (light
    // grey where there is none).
    let id_ink = match (lit, focused, muted, color) {
        (true, ..) => {
            painter.rect_filled(idr, 0.0, lighter(color.unwrap_or(Color32::from_gray(0x9B))));
            S00
        }
        (_, true, _, Some(c)) => {
            painter.rect_filled(idr, 0.0, c);
            S00
        }
        (_, _, true, _) => Color32::from_gray(0x66),
        _ => white(164),
    };
    painter.text(idr.center(), Align2::CENTER_CENTER, id, font(12.0), id_ink);
    let name_ink = if muted {
        white(61)
    } else if focused {
        Color32::WHITE
    } else {
        white(142)
    };
    label(painter, name_rect(rect), 10.0, name, 12.0, name_ink);
    // The coloured box at the right: the channel's mute (grey when muted); the
    // stereo level meter next to it.
    if let Some(c) = color {
        let mb = mute_box(rect);
        painter.rect_filled(mb, 1.0, if muted { Color32::from_gray(0x55) } else { c });
        for (i, level) in [meter.0, meter.1].into_iter().enumerate() {
            let bar = Rect::from_min_max(
                Pos2::new(mb.left() - 9.0 + i as f32 * 3.0, mb.top()),
                Pos2::new(mb.left() - 7.0 + i as f32 * 3.0, mb.bottom()),
            );
            painter.rect_filled(bar, 0.0, white(14));
            // -48 dB .. 0 dB
            let db = if level > 0.0 { 20.0 * level.log10() } else { -96.0 };
            let f = ((db + 48.0) / 48.0).clamp(0.0, 1.0);
            if f > 0.0 {
                let lit = Rect::from_min_max(Pos2::new(bar.left(), bar.bottom() - bar.height() * f), bar.max);
                painter.rect_filled(lit, 0.0, if level >= 1.0 { Color32::from_rgb(0xE3, 0x42, 0x3F) } else { c });
            }
        }
    }
}

/// The primary button went down on this widget in this frame.
pub fn pressed(ui: &Ui, resp: &Response) -> bool {
    ui.input(|i| i.pointer.primary_pressed()) && resp.is_pointer_button_down_on()
}

/// Where a row's mute box sits (its right end).
fn mute_box(row: Rect) -> Rect {
    Rect::from_min_max(Pos2::new(row.right() - 14.0, row.top() + 5.0), Pos2::new(row.right() - 5.0, row.bottom() - 6.0))
}

/// Where a row's name sits (between the ID button and the mute box).
fn name_rect(row: Rect) -> Rect {
    Rect::from_min_max(Pos2::new(row.left() + 32.0, row.top()), Pos2::new(row.right() - 26.0, row.bottom()))
}

/// Clicks on a group or sound row: the mute box toggles mute, a click focuses
/// (`focus`), a double-click mutes a group or renames a sound in place (Enter keeps it,
/// Esc cancels), a right-click focuses and opens Maschine's menu for it.
#[allow(clippy::too_many_arguments)]
/// The sound rows drawn this frame and their sounds (group, sound): where files dragged
/// from the desktop land (`App::drop_samples`).
pub const DROP_ROWS: &str = "deckard-drop-rows";

fn row_input(ui: &mut Ui, app: &App, p: &Project, row: Rect, clip: Rect, target: Target, name: &str, focus: Act, acts: &mut Vec<Act>) {
    // A sound row takes samples dragged onto it: lit while they're over it.
    if let Target::Sound(g, s) = target {
        let r = row.intersect(clip);
        ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<Vec<(Rect, (usize, usize))>>(egui::Id::new(DROP_ROWS)).push((r, (g, s))));
        if app.file_drag.is_some_and(|at| r.contains(at)) {
            ui.painter().rect_stroke(r.shrink(1.0), 2.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
        }
    }
    let key = ui.id().with(("row", format!("{target:?}")));
    if let Target::Group(g) = target {
        note_rect(ui, &format!("group row {g}"), row);
    }
    // The row first, then the mute box on top of it (the last widget gets the click).
    let resp = ui.interact(row.intersect(clip), key, Sense::click());
    let mb = mute_box(row).expand(3.0).intersect(clip);
    let mute = ui.interact(mb, key.with("mute"), Sense::click()).on_hover_text("Mute (click again to unmute)");
    if mute.clicked() {
        acts.push(Act::ToggleMute(target));
        return;
    }
    if app.renaming == Some(target) {
        rename_editor(ui, name_rect(row).shrink2(Vec2::new(4.0, 3.0)).intersect(clip), target, name, acts);
        return;
    }
    if resp.clicked() || resp.secondary_clicked() || resp.double_clicked() {
        acts.push(Act::KeyFocus(if matches!(target, Target::Group(_)) { KeyFocus::Groups } else { KeyFocus::Sounds }));
    }
    if resp.double_clicked() {
        // A group's double-click mutes or unmutes it (renaming is in its menu).
        acts.push(if matches!(target, Target::Group(_)) { Act::ToggleMute(target) } else { Act::StartRename(target) });
    } else if resp.clicked() || resp.secondary_clicked() {
        acts.push(focus);
    }
    resp.context_menu(|ui| match target {
        Target::Sound(g, s) => sound_menu(ui, app, p, g, s, acts),
        Target::Group(g) => group_menu(ui, app, p, g, acts),
        _ => {}
    });
}

/// The event area's right-click menu (software manual p. 236 and on): the mouse
/// modes, Quantize, the clipboard, the Nudge Grid.
fn event_menu(ui: &mut Ui, app: &App, at: crate::clipboard::PasteAt, acts: &mut Vec<Act>) {
    ui.set_min_width(150.0);
    for (m, name) in [
        (MouseMode::Select, "Select"),
        (MouseMode::Draw, "Draw"),
        (MouseMode::Erase, "Erase"),
        (MouseMode::Split, "Split"),
        (MouseMode::Join, "Join"),
        (MouseMode::Mute, "Mute"),
    ] {
        if ui.add(egui::Button::new(name).selected(app.mouse == m)).clicked() {
            acts.push(Act::SetMouse(m));
            ui.close();
        }
    }
    ui.separator();
    item(ui, "Quantize", true, || Act::Quantize(100), acts);
    item(ui, "Quantize 50%", true, || Act::Quantize(50), acts);
    ui.separator();
    item(ui, "Cut", true, || Act::CopyNotes { cut: true }, acts);
    item(ui, "Copy", true, || Act::CopyNotes { cut: false }, acts);
    item(ui, "Paste", app.note_clip.is_some(), || Act::PasteNotes(at), acts);
    let selected = app.selected.clone();
    item(
        ui,
        "Delete",
        !selected.is_empty(),
        || Act::EditNotes { g: app.group, p: app.pattern, label: "Delete Notes", remove: selected, add: Vec::new() },
        acts,
    );
    ui.separator();
    ui.menu_button("Nudge Grid", |ui| {
        for (i, name) in deckard_lua_host::api::pattern_editor::NUDGE_GRIDS.iter().enumerate() {
            if ui.add(egui::Button::new(*name).selected(app.nudge.0 == i)).clicked() {
                acts.push(Act::NudgeGrid(i));
                ui.close();
            }
        }
    });
}

/// Which part of the window the editing keys (⌘X / ⌘C / ⌘V, Del, ⌘R) act on: the last
/// one clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyFocus {
    #[default]
    Events,
    Sounds,
    Groups,
    /// The focused macro in the Assignment area's Pages tab.
    Macros,
}

fn item(ui: &mut Ui, label: &str, enabled: bool, act: impl FnOnce() -> Act, acts: &mut Vec<Act>) {
    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
        acts.push(act());
        ui.close();
    }
}

/// Maschine's Color ▸ submenu: the 16 colours as a 4 × 4 palette (the current one
/// outlined), then Default.
fn color_menu(ui: &mut Ui, current: Color, targets: Vec<Target>, acts: &mut Vec<Act>) {
    ui.menu_button("Color", |ui| {
        for row in 0..4u8 {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for col in 0..4u8 {
                    let i = row * 4 + col;
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                    ui.painter().rect_filled(r, 2.0, palette(Color::Palette(i), 0));
                    if current == Color::Palette(i) {
                        ui.painter().rect_stroke(r, 2.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Outside);
                    } else if resp.hovered() {
                        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, white(200)), StrokeKind::Outside);
                    }
                    if resp.clicked() {
                        acts.push(Act::SetColor(targets.clone(), Color::Palette(i)));
                        ui.close();
                    }
                }
            });
        }
        if ui.selectable_label(current == Color::Default, "Default").clicked() {
            acts.push(Act::SetColor(targets.clone(), Color::Default));
            ui.close();
        }
    });
}

/// Open…, Save, Save As…, Save with Samples… of a sound's or group's menu.
fn file_items(ui: &mut Ui, app: &App, what: deckard_doc::FileItem, acts: &mut Vec<Act>) {
    use crate::SaveMode;
    item(ui, "Open...", true, || Act::OpenItem(what), acts);
    let saved = app.saved.contains_key(&what);
    item(ui, "Save", true, || Act::SaveItem(what, if saved { SaveMode::Save } else { SaveMode::SaveAs }), acts);
    item(ui, "Save As...", true, || Act::SaveItem(what, SaveMode::SaveAs), acts);
    item(ui, "Save with Samples...", true, || Act::SaveItem(what, SaveMode::WithSamples), acts);
}

/// A sound slot's menu, as Maschine's (docs/context-menus.md §1).
fn sound_menu(ui: &mut Ui, app: &App, p: &Project, g: usize, s: usize, acts: &mut Vec<Act>) {
    ui.set_min_width(170.0);
    let what = deckard_doc::FileItem::Sound(g, s);
    file_items(ui, app, what, acts);
    if !app.song_view {
        item(ui, "Bounce", app.bounce.running.is_none(), || Act::Bounce(g, s), acts);
    }
    ui.separator();
    item(ui, "Import MIDI...", true, || Act::ImportMidi(g, s), acts);
    item(ui, "Export MIDI...", true, || Act::ExportMidi(g, s), acts);
    ui.separator();
    let color = p.groups.get(g).and_then(|x| x.sounds.get(s)).map_or(Color::Default, |x| x.color);
    color_menu(ui, color, vec![Target::Sound(g, s)], acts);
    ui.separator();
    item(ui, "Cut", true, || Act::CutSound(g, s), acts);
    item(ui, "Copy", true, || Act::CopySound(g, s), acts);
    item(ui, "Paste", !app.sound_clipboard.is_empty(), || Act::PasteSound(g, s), acts);
    ui.separator();
    item(ui, "Rename", true, || Act::StartRename(Target::Sound(g, s)), acts);
    item(ui, "Reset", true, || Act::ResetSound(g, s), acts);
}

/// A group slot's menu (§2).
fn group_menu(ui: &mut Ui, app: &App, p: &Project, g: usize, acts: &mut Vec<Act>) {
    ui.set_min_width(170.0);
    let batch = p.groups.get(g).map_or(0, |x| x.channel.midi_batch) as usize;
    file_items(ui, app, deckard_doc::FileItem::Group(g), acts);
    ui.separator();
    item(ui, "Import MIDI...", true, || Act::ImportGroupMidi(g), acts);
    item(ui, "Export MIDI...", true, || Act::ExportGroupMidi(g), acts);
    // How the group's MIDI files hold its sounds: each on its own channel, or each on
    // its own note (from the group's start note).
    ui.menu_button("Group MIDI Batch Setup", |ui| {
        for (k, name) in ["Sounds to MIDI Channels", "Sounds to MIDI Notes"].into_iter().enumerate() {
            if ui.radio(batch == k, name).clicked() {
                acts.push(Act::SetParam(Target::Group(g), "midi_batch_export", k as f64));
                ui.close();
            }
        }
    });
    ui.separator();
    color_menu(ui, p.groups.get(g).map_or(Color::Default, |x| x.color), vec![Target::Group(g)], acts);
    ui.separator();
    item(ui, "Cut", true, || Act::CutGroup(g), acts);
    item(ui, "Copy", true, || Act::CopyGroup(g), acts);
    item(ui, "Paste", app.group_clipboard.is_some(), || Act::PasteGroup(g), acts);
    ui.separator();
    item(ui, "Rename", true, || Act::StartRename(Target::Group(g)), acts);
    item(ui, "Delete", true, || Act::DeleteGroup(g), acts);
}

/// A pattern's menu in the Ideas view (§3).
fn pattern_menu(ui: &mut Ui, p: &Project, g: usize, pi: usize, acts: &mut Vec<Act>) {
    ui.set_min_width(150.0);
    item(ui, "Clear", true, || Act::ClearPattern(g, pi), acts);
    ui.separator();
    item(ui, "Rename", true, || Act::StartRename(Target::Pattern(g, pi)), acts);
    let color = p.groups.get(g).and_then(|x| x.patterns.get(pi)).map_or(Color::Default, |x| x.color);
    color_menu(ui, color, vec![Target::Pattern(g, pi)], acts);
    ui.separator();
    item(ui, "Double", true, || Act::DoublePattern(g, pi), acts);
    item(ui, "Duplicate", true, || Act::DuplicatePattern(g, pi), acts);
    item(ui, "Delete", true, || Act::DeletePattern(g, pi), acts);
    ui.separator();
    item(ui, "Export Audio", true, || Act::PatternAudio(g, pi, false), acts);
    item(ui, "Sample to Sound", true, || Act::PatternAudio(g, pi, true), acts);
}

/// The in-place name editor over `r` (rows, scene tabs, group tiles, section
/// headers): Enter keeps the new name, Esc or clicking away cancels.
fn rename_editor(ui: &mut Ui, r: Rect, target: Target, name: &str, acts: &mut Vec<Act>) {
    let text_id = egui::Id::new("rename-text");
    let mut text: String = ui.data(|d| d.get_temp(text_id)).unwrap_or_else(|| name.to_string());
    let mut child = ui.new_child(UiBuilder::new().max_rect(r));
    let edit = child.add(egui::TextEdit::singleline(&mut text).desired_width(r.width()).font(egui::TextStyle::Body));
    if !edit.has_focus() && !edit.lost_focus() {
        edit.request_focus();
    }
    if edit.lost_focus() {
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            acts.push(Act::Rename(target, text.trim().to_string()));
        } else {
            acts.push(Act::CancelRename);
        }
        ui.data_mut(|d| d.remove::<String>(text_id));
    } else {
        ui.data_mut(|d| d.insert_temp(text_id, text));
    }
}

fn song_view(ui: &mut Ui, app: &App, p: &Project, rect: Rect, head: Option<Playhead>, acts: &mut Vec<Act>) {
    view_switch(ui, app, rect, acts);
    let col = Rect::from_min_size(Pos2::new(rect.left() + STRIP + 1.0, rect.top()), Vec2::new(COLUMN, rect.height()));
    let ed = Rect::from_min_max(Pos2::new(col.right() + 1.0, rect.top()), rect.max);
    let painter = ui.painter_at(rect);
    painter.rect_filled(col, 0.0, S05);
    painter.rect_filled(ed, 0.0, S05);

    // Column title: 32 + timeline 18.
    let title = Rect::from_min_size(col.min, Vec2::new(COLUMN, 32.0));
    label(&painter, title, 8.0, "Song", 15.0, white(245));
    let rows_top = col.top() + 50.0;
    let rows = Rect::from_min_max(Pos2::new(rect.left(), rows_top), rect.max);
    let y0 = scroll(ui, "song-rows", rows, (p.groups.len() + 1) as f32 * ROW);
    let row_painter = painter.with_clip_rect(Rect::from_min_max(Pos2::new(col.left(), rows_top), col.max));
    for g in 0..p.groups.len() {
        let r = Rect::from_min_size(Pos2::new(col.left(), rows_top + g as f32 * ROW - y0), Vec2::new(COLUMN, ROW));
        if !r.intersects(rows) {
            continue;
        }
        let gr = &p.groups[g];
        group_row(
            &row_painter,
            r,
            &group_id(g),
            &group_name(p, g),
            Some(group_color(p, g)),
            g == app.group,
            gr.muted,
            app.lit.iter().any(|&(lg, _)| lg == g),
            S05,
            app.meter(g, None),
        );
        row_input(ui, app, p, r, rows, Target::Group(g), &gr.name, Act::FocusGroup(g), acts);
    }
    // "+" after the last group: a new empty group.
    let plus = Rect::from_min_size(Pos2::new(col.left(), rows_top + p.groups.len() as f32 * ROW - y0), Vec2::new(COLUMN, ROW));
    if plus.intersects(rows) {
        let resp = ui.interact(plus.intersect(rows), ui.id().with("song-add-group"), Sense::click());
        if resp.hovered() {
            row_painter.rect_filled(plus.shrink(1.0), 2.0, S03);
        }
        row_painter.text(
            Pos2::new(plus.left() + 16.0, plus.center().y),
            Align2::CENTER_CENTER,
            "+",
            font(16.0),
            white(if resp.hovered() { 245 } else { 142 }),
        );
        if resp.on_hover_text("New group").clicked() {
            acts.push(Act::AddGroup);
        }
    }

    // Editor: sections bar (32), timeline (18), tracks.
    let bar = p.bar_length().max(1) as f32;
    let clips_end = p.groups.iter().flat_map(|g| &g.clips).map(|c| c.position + c.length).max().unwrap_or(0);
    let total = p.arrangement_length().max(clips_end) as f32 / bar;
    // Leave room for the "+" after the last section.
    let ppb = ((ed.width() - 40.0) / total.max(16.0)).max(12.0);
    let hx = hscroll(ui, "song-x", ed, total.max(16.0) * ppb);
    let x_of = |ticks: f64| ed.left() + ticks as f32 / bar * ppb - hx;
    let secs = Rect::from_min_size(ed.min, Vec2::new(ed.width(), 32.0));
    ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-sections"), secs));
    let timeline = Rect::from_min_size(Pos2::new(ed.left(), secs.bottom()), Vec2::new(ed.width(), 18.0));
    let tracks = Rect::from_min_max(Pos2::new(ed.left(), rows_top), ed.max);
    let ep = painter.with_clip_rect(ed);
    ep.hline(timeline.x_range(), timeline.top(), Stroke::new(1.0, Color32::BLACK));
    ep.hline(timeline.x_range(), timeline.bottom(), Stroke::new(1.0, Color32::BLACK));
    let end_x = x_of(p.arrangement_length() as f64);
    ep.rect_filled(Rect::from_min_max(Pos2::new(end_x, tracks.top()), tracks.max), 0.0, S00);
    // Bars and beats.
    let step = (40.0 / ppb).ceil().max(1.0) as i64;
    for b in 0..=(total.max(16.0) as i64) {
        let x = x_of(b as f64 * bar as f64);
        ep.vline(x, tracks.y_range(), Stroke::new(1.0, white(20)));
        if b % step == 0 {
            ep.vline(x, timeline.y_range(), Stroke::new(1.0, white(40)));
            ep.text(Pos2::new(x + 6.0, timeline.center().y - 1.0), Align2::LEFT_CENTER, (b + 1).to_string(), font(12.0), white(142));
        }
    }
    let tp = painter.with_clip_rect(tracks);
    for g in 0..p.groups.len() {
        let y = rows_top + g as f32 * ROW - y0;
        if g == app.group {
            tp.rect_filled(Rect::from_min_size(Pos2::new(ed.left(), y), Vec2::new(ed.width(), ROW)), 0.0, white(12));
        }
        tp.hline(ed.x_range(), y + ROW - 0.5, Stroke::new(1.0, white(10)));
    }
    if p.sections.is_empty() {
        label(&ep, secs, 8.0, "(no arrangement: use the Ideas view)", 12.0, white(142));
    }
    let playing_section = head.filter(|h| h.mode == Mode::Arrangement).and_then(|h| section_at(p, h.position));
    let queued = app.queued_section();
    let time = ui.input(|i| i.time);
    let mut start = 0i64;
    for (i, sec) in p.sections.iter().enumerate() {
        let (x0, x1) = (x_of(start as f64), x_of((start + sec.length) as f64));
        start += sec.length;
        if x1 < ed.left() || x0 > ed.right() {
            continue;
        }
        let r = Rect::from_min_max(Pos2::new(x0, secs.top() + 4.0), Pos2::new(x1, secs.bottom() - 4.0));
        let resp = ui.interact(r.intersect(secs), ui.id().with(("section", i)), Sense::click_and_drag());
        ui.data_mut(|d| d.insert_temp(egui::Id::new(("deckard-section", i)), r));
        // Drag the header to move the section; a line shows where it goes.
        if (resp.dragged() || resp.drag_stopped())
            && let Some(pos) = ui.ctx().pointer_latest_pos()
        {
            let mut acc = 0i64;
            let mut to = p.sections.len() - 1;
            for (k, sk) in p.sections.iter().enumerate() {
                if pos.x < x_of((acc + sk.length) as f64) {
                    to = k;
                    break;
                }
                acc += sk.length;
            }
            if pos.x < x_of(0.0) {
                to = 0;
            }
            if resp.drag_stopped() {
                if to != i {
                    acts.push(Act::MoveSection(i, to));
                }
            } else if to != i {
                let edge: i64 = p.sections.iter().take(if to > i { to + 1 } else { to }).map(|x| x.length).sum();
                ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.id().with("section-drop"))).vline(
                    x_of(edge as f64),
                    Rect::from_min_max(secs.min, tracks.max).y_range(),
                    Stroke::new(2.0, Color32::WHITE),
                );
            }
        }
        let scene = sec.scene.filter(|&s| s < p.scenes.len());
        // The picked section is focused; a picked section waiting for the playing one
        // to end blinks.
        let blink = queued == Some(i) && (time * 4.0).sin() < 0.0;
        let focused = (app.section == Some(i) || (app.section.is_none() && playing_section == Some(i))) && !blink;
        ep.rect_filled(
            r,
            0.0,
            if focused {
                Color32::from_rgb(0x46, 0x46, 0x46)
            } else if resp.hovered() {
                S03
            } else {
                S02
            },
        );
        if queued == Some(i) {
            ui.ctx().request_repaint();
        }
        if playing_section == Some(i) {
            triangle(&ep, Pos2::new(r.left() + 8.0, r.center().y), 3.5, white(245));
        }
        ep.vline(x1, r.y_range(), Stroke::new(1.0, Color32::BLACK));
        if let Some(s) = scene {
            if focused {
                ep.rect_filled(Rect::from_min_max(Pos2::new(x0, r.bottom() - 3.0), r.max), 0.0, scene_color(p, s));
            }
            let indent = if playing_section == Some(i) { 16.0 } else { 7.0 };
            // A double-click (which also picks the section) renames its scene.
            if app.renaming == Some(Target::Scene(s)) && app.section == Some(i) {
                let er = Rect::from_min_max(Pos2::new(r.left() + indent, r.top() + 1.0), Pos2::new(r.right() - 2.0, r.bottom() - 4.0));
                rename_editor(ui, er.intersect(secs), Target::Scene(s), &p.scenes[s].name, acts);
            } else {
                label(&ep, r, indent, &scene_name(p, s), 12.0, white(245));
            }
            for &(g, pi) in &p.scenes[s].patterns {
                let y = rows_top + g as f32 * ROW - y0;
                let plen = pattern_len(p, g, pi);
                let mut t = 0;
                while t < sec.length {
                    let c0 = x_of((start - sec.length + t) as f64);
                    let c1 = x_of((start - sec.length + (t + plen).min(sec.length)) as f64);
                    let clip = Rect::from_min_max(Pos2::new(c0 + 0.5, y + 2.0), Pos2::new(c1 - 0.5, y + ROW - 2.0));
                    // The pattern's repeats are dark, as the patterns not in the scene
                    // are in the Ideas view.
                    // Under clips, patterns are dimmed (clips are fully coloured).
                    let c = pattern_color(p, g, pi);
                    let c = if p.groups[g].clips.is_empty() { c } else { dim(c, 0.6) };
                    tp.rect_filled(clip, 3.0, if t == 0 { c } else { dim(c, 0.3) });
                    if t == 0 {
                        label(&tp, clip, 7.0, &p.groups[g].patterns[pi].name, 12.0, black(245));
                    }
                    let cr = ui.interact(clip.intersect(tracks), ui.id().with(("clip", i, g, t)), Sense::click());
                    if cr.clicked() {
                        acts.push(Act::FocusPattern(g, pi));
                        acts.push(Act::LaunchSection(i));
                    }
                    t += plen;
                }
            }
        }
        let resp = resp.on_hover_text(
            "Click to pick the section (while the song plays, it jumps there when the playing section ends); double-click to rename its scene; right-click for more",
        );
        if resp.double_clicked() {
            if let Some(s) = scene {
                acts.push(Act::StartRename(Target::Scene(s)));
            }
        } else if resp.clicked() {
            acts.push(Act::LaunchSection(i));
        }
        resp.context_menu(|ui| section_menu(ui, p, i, acts));
    }
    clip_lanes(ui, app, p, tracks, rows_top - y0, &x_of, ppb, acts);
    // Section ends, over the headers: drag to make a section longer or shorter.
    let mut start = 0i64;
    for (i, sec) in p.sections.iter().enumerate() {
        let (x0, x1) = (x_of(start as f64), x_of((start + sec.length) as f64));
        start += sec.length;
        if x1 < ed.left() - 4.0 || x0 > ed.right() {
            continue;
        }
        let r = Rect::from_min_max(Pos2::new(x0, secs.top() + 4.0), Pos2::new(x1, secs.bottom() - 4.0));
        let edge = Rect::from_min_max(Pos2::new(x1 - 4.0, r.top()), Pos2::new(x1 + 4.0, r.bottom()));
        let er = ui
            .interact(edge.intersect(secs), ui.id().with(("section-end", i)), Sense::drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        let er = if er.dragged() {
            er.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        } else {
            er.on_hover_text("Drag to change the section's length")
        };
        if (er.dragged() || er.drag_stopped())
            && let Some(pos) = ui.ctx().pointer_latest_pos()
        {
            let section_start = start - sec.length;
            let ticks_at = (pos.x - x_of(0.0)) / ppb * bar;
            let bars = (((ticks_at as f64 - section_start as f64) / bar as f64).round() as i64).max(1);
            let len = bars * bar as i64;
            if er.drag_stopped() {
                if len != sec.length {
                    acts.push(Act::SectionLength(i, Some(len)));
                }
            } else {
                // The new end, and the length, while dragging.
                let fg = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.id().with("section-len")));
                let nx = x_of((section_start + len) as f64);
                let ghost = Rect::from_min_max(Pos2::new(x0, secs.top() + 4.0), Pos2::new(nx, tracks.bottom()));
                fg.rect_stroke(ghost, 0.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
                fg.text(
                    Pos2::new(nx + 6.0, secs.center().y),
                    Align2::LEFT_CENTER,
                    format!("{bars} bar{}", if bars == 1 { "" } else { "s" }),
                    font(12.0),
                    Color32::WHITE,
                );
            }
        }
    }
    // "+" after the last section: a new section playing the focused scene.
    if !p.sections.is_empty() && !p.scenes.is_empty() {
        let plus = Rect::from_min_size(Pos2::new(end_x + 1.0, secs.top() + 4.0), Vec2::new(27.0, secs.height() - 8.0));
        let resp = ui.interact(plus.intersect(secs), ui.id().with("add-section"), Sense::click());
        ep.rect_filled(plus, 0.0, if resp.hovered() { S03 } else { S01 });
        ep.text(plus.center(), Align2::CENTER_CENTER, "+", font(16.0), white(if resp.hovered() { 245 } else { 142 }));
        if resp.on_hover_text(format!("New section playing {}", scene_name(p, app.scene.min(p.scenes.len() - 1)))).clicked() {
            acts.push(Act::AddSection(app.scene.min(p.scenes.len() - 1)));
        }
    }
    if let Some(h) = head.filter(|h| h.mode == Mode::Arrangement) {
        let x = x_of(h.position);
        ep.vline(x, Rect::from_min_max(timeline.min, ed.max).y_range(), Stroke::new(1.0, Color32::WHITE));
    }
}

/// The groups' clips over their rows (drawn over the patterns they replace), fully
/// coloured: click to select, drag to move (by bars), drag the end to resize,
/// right-click for Duplicate, Double, Delete.
#[allow(clippy::too_many_arguments)]
fn clip_lanes(ui: &mut Ui, app: &App, p: &Project, tracks: Rect, top: f32, x_of: &dyn Fn(f64) -> f32, ppb: f32, acts: &mut Vec<Act>) {
    let bar = p.bar_length().max(1);
    let tp = ui.painter_at(tracks);
    let snap = |ticks: f32| ((ticks / bar as f32).round() as i64 * bar).max(0);
    for (g, grp) in p.groups.iter().enumerate() {
        let y = top + g as f32 * ROW;
        for (c, clip) in grp.clips.iter().enumerate() {
            let r = Rect::from_min_max(
                Pos2::new(x_of(clip.position as f64) + 0.5, y + 2.0),
                Pos2::new(x_of((clip.position + clip.length) as f64) - 0.5, y + ROW - 2.0),
            );
            if !r.intersects(tracks) {
                continue;
            }
            let color = inherit(grp.clip_patterns.get(clip.pattern).map_or(Color::Default, |x| x.color), group_color(p, g));
            tp.rect_filled(r, 3.0, color);
            if app.clip == Some((g, c)) {
                tp.rect_stroke(r, 3.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
            }
            let name = grp.clip_patterns.get(clip.pattern).map_or("", |x| x.name.as_str());
            label(&tp, r, 7.0, if name.is_empty() { "Clip" } else { name }, 12.0, black(245));
            let body = ui.interact(r.intersect(tracks), ui.id().with(("song-clip", g, c)), Sense::click_and_drag());
            let edge = Rect::from_min_max(Pos2::new(r.right() - 4.0, r.top()), Pos2::new(r.right() + 4.0, r.bottom()));
            let er = ui
                .interact(edge.intersect(tracks), ui.id().with(("song-clip-end", g, c)), Sense::drag())
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            let fg = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.id().with("clip-ghost")));
            if body.drag_started() || body.clicked() {
                acts.push(Act::Clip(g, c, ClipAct::Select));
            }
            if (body.dragged() || body.drag_stopped())
                && let (Some(pos), Some(from)) = (ui.ctx().pointer_latest_pos(), ui.ctx().input(|i| i.pointer.press_origin()))
            {
                let to = snap(clip.position as f32 + (pos.x - from.x) / ppb * bar as f32);
                if body.drag_stopped() {
                    if to != clip.position {
                        acts.push(Act::Clip(g, c, ClipAct::Move(to)));
                    }
                } else {
                    fg.rect_stroke(
                        r.translate(Vec2::new(x_of(to as f64) - x_of(clip.position as f64), 0.0)),
                        3.0,
                        Stroke::new(2.0, Color32::WHITE),
                        StrokeKind::Inside,
                    );
                }
            }
            if (er.dragged() || er.drag_stopped())
                && let Some(pos) = ui.ctx().pointer_latest_pos()
            {
                let end = snap((pos.x - x_of(0.0)) / ppb * bar as f32).max(clip.position + bar);
                if er.drag_stopped() {
                    if end - clip.position != clip.length {
                        acts.push(Act::Clip(g, c, ClipAct::Length(end - clip.position)));
                    }
                } else {
                    fg.rect_stroke(
                        Rect::from_min_max(r.min, Pos2::new(x_of(end as f64), r.bottom())),
                        3.0,
                        Stroke::new(2.0, Color32::WHITE),
                        StrokeKind::Inside,
                    );
                }
            }
            body.on_hover_text("A clip: drag to move it, drag its end to resize, right-click for more").context_menu(|ui| {
                ui.set_min_width(150.0);
                item(ui, "Duplicate", true, || Act::Clip(g, c, ClipAct::Duplicate), acts);
                item(ui, "Double", true, || Act::Clip(g, c, ClipAct::Double), acts);
                ui.separator();
                item(ui, "Delete", true, || Act::Clip(g, c, ClipAct::Delete), acts);
            });
        }
    }
}

// ---------------------------------------------------------------------------------------
// Chain area (ParameterArea.txt): MASTER/GROUP/SOUND tabs, slots, and the parameters
// we know.

/// A knob; `bipolar` ones show their value from the centre (12 o'clock).
fn knob(painter: &Painter, rect: Rect, name: &str, value: &str, amount: f32, color: Color32, bipolar: bool) {
    painter.text(Pos2::new(rect.center().x, rect.top() + 8.0), Align2::CENTER_CENTER, name, font(10.0), white(142));
    let c = Pos2::new(rect.center().x, rect.top() + 40.0);
    let arc = |from: f32, to: f32, col: Color32, w: f32| {
        let n = 24;
        let pts: Vec<Pos2> = (0..=n)
            .map(|i| {
                let a = from + (to - from) * i as f32 / n as f32;
                c + Vec2::angled(a) * 16.0
            })
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(w, col)));
    };
    let start = std::f32::consts::PI * 0.75;
    let sweep = std::f32::consts::PI * 1.5;
    arc(start, start + sweep, white(40), 3.0);
    let a = amount.clamp(0.0, 1.0);
    if bipolar {
        if (a - 0.5).abs() > 0.005 {
            arc(start + sweep * 0.5, start + sweep * a, color, 3.0);
        }
    } else if a > 0.0 {
        arc(start, start + sweep * a, color, 3.0);
    }
    painter.text(Pos2::new(rect.center().x, rect.bottom() - 10.0), Align2::CENTER_CENTER, value, font(12.0), white(245));
}

fn db(level: f64) -> String {
    let d = deckard_model::level_db(level);
    if d.is_finite() && d > -120.0 { format!("{:.1} dB", d + 0.0).replace("-0.0", "0.0") } else { "-inf dB".into() }
}

pub(crate) fn pan(v: f64) -> String {
    let v = (v * 100.0).round() as i64;
    match v {
        0 => "C".into(),
        v if v < 0 => format!("{}L", -v),
        v => format!("{v}R"),
    }
}

/// A knob: name, value text, position 0..1, and what dragging it changes.
pub type Knob = (Cow<'static, str>, String, f32, Option<Edit>);

/// A knob that changes a document parameter.
#[derive(Clone, Copy)]
pub enum Edit {
    Level(Target),
    Pan(Target),
    /// One field of a groove (the rest as they are).
    Swing(Target, Swing, SwingField),
    /// A parameter (`Target::Sampler`/`Target::Zone` field) over min..max, with its
    /// default (double-click).
    Param(Target, &'static str, f64, f64, f64),
    /// [`Edit::Param`] on a log scale: the knob's travel is `lo·(hi/lo)^amount` (lo > 0).
    Log(Target, &'static str, f64, f64, f64),
    /// [`Edit::Param`] in whole steps (Resochord's Tune: −24..24).
    Int(Target, &'static str, f64, f64, f64),
    /// An on/off parameter, now on or off: a click switches it.
    Toggle(Target, &'static str, bool),
    /// One of the choices (their names), now `cur`, with its default: a selector with
    /// arrows; a click on the value lists the choices.
    Choice(Target, &'static str, &'static [&'static str], usize, f64),
    /// Parameter `id` of the plug-in in chain slot `.0`, with its default.
    Plugin(usize, u32, f64),
    /// A plug-in parameter that is a choice: slot, id, steps, the step it is at, and
    /// its default (the page holds the names).
    PluginChoice(usize, u32, i32, usize, f64),
    /// [`Edit::Plugin`] of the plug-in in slot `.1` of chain `.0`, not the chain shown
    /// (a macro on another channel's plug-in).
    PluginIn(deckard_doc::Chain, usize, u32, f64),
    /// A routing choice, now entry `.0` of its list (the page holds the list).
    Route(usize),
}

#[derive(Clone, Copy)]
pub enum SwingField {
    Amount,
    Cycle,
    Invert,
}

impl Edit {
    /// The action for knob position `amount` (0..1), using the knob's scale.
    pub(crate) fn to_act(self, amount: f32) -> Act {
        let a = amount as f64;
        match self {
            Edit::Level(t) => Act::SetLevel(t, a),
            Edit::Pan(t) => Act::SetPan(t, (a * 2.0 - 1.0 + 1e-9).clamp(-1.0, 1.0)),
            Edit::Swing(t, s, f) => Act::SetSwing(
                t,
                match f {
                    SwingField::Amount => Swing { amount: (a * 100.0).round(), ..s },
                    SwingField::Cycle => Swing { cycle: (a * 7.0).round() as i64, ..s },
                    SwingField::Invert => Swing { invert: a > 0.5, ..s },
                },
            ),
            Edit::Param(t, f, lo, hi, _) => Act::SetParam(t, f, lo + a * (hi - lo)),
            Edit::Log(t, f, lo, hi, _) => Act::SetParam(t, f, lo * (hi / lo).powf(a)),
            Edit::Int(t, f, lo, hi, _) => Act::SetParam(t, f, (lo + a * (hi - lo)).round()),
            Edit::Choice(t, f, options, _, _) => Act::SetParam(t, f, (a * (options.len() - 1) as f64).round()),
            Edit::Toggle(t, f, _) => Act::SetParam(t, f, if a > 0.5 { 1.0 } else { 0.0 }),
            Edit::Plugin(slot, id, _) => Act::PluginParam(slot, id, a),
            Edit::PluginIn(c, slot, id, _) => Act::PluginParamIn(c, slot, id, a),
            Edit::PluginChoice(slot, id, steps, ..) => Act::PluginParam(slot, id, (a * steps as f64).round() / steps as f64),
            Edit::Route(_) => Act::Nothing,
        }
    }

    /// What a click does (switches an on/off parameter).
    fn click(self) -> Option<Act> {
        match self {
            Edit::Toggle(t, f, on) => Some(Act::SetParam(t, f, if on { 0.0 } else { 1.0 })),
            _ => None,
        }
    }

    fn reset(self) -> Act {
        match self {
            Edit::Level(t) => Act::SetLevel(t, 0.75),
            Edit::Pan(t) => Act::SetPan(t, 0.0),
            Edit::Swing(t, s, f) => Act::SetSwing(
                t,
                match f {
                    SwingField::Amount => Swing { amount: 0.0, ..s },
                    SwingField::Cycle => Swing { cycle: 0, ..s },
                    SwingField::Invert => Swing { invert: false, ..s },
                },
            ),
            Edit::Param(t, f, _, _, default)
            | Edit::Log(t, f, _, _, default)
            | Edit::Int(t, f, _, _, default)
            | Edit::Choice(t, f, _, _, default) => Act::SetParam(t, f, default),
            Edit::Toggle(t, f, _) => Act::SetParam(t, f, 0.0),
            Edit::Plugin(slot, id, default) | Edit::PluginChoice(slot, id, _, _, default) => Act::PluginParam(slot, id, default),
            Edit::PluginIn(c, slot, id, default) => Act::PluginParamIn(c, slot, id, default),
            Edit::Route(_) => Act::Nothing,
        }
    }
}

/// One page of a module's knobs as the Control area shows them.
pub(crate) struct SlotPage {
    pub names: Vec<String>,
    pub page: usize,
    pub knobs: Vec<Option<Knob>>,
    pub sections: Vec<(usize, Cow<'static, str>)>,
    /// The act that shows page n.
    pub show: fn(usize) -> Act,
    /// The knobs that pick from a list (an effect's side-chain SOURCE): knob, the
    /// choices' names and what each sets.
    pub routes: Vec<(usize, Vec<String>, Vec<RouteSet>)>,
}

/// Every parameter a knob turns anywhere in the project ([`channel::every_target`]).
pub(crate) fn every_target(app: &App, doc: &deckard_doc::Document) -> Vec<channel::MacroTarget> {
    let groove = |g: usize, s: usize, f: &str| app.groove_setting(g, s, f);
    let ctx = channel::Ctx {
        doc,
        has_midi_macro: false,
        p: doc.project(),
        midi_ins: &app.midi_ports.0,
        midi_outs: &app.midi_ports.1,
        polyphonic: app.piano_roll,
        groove: &groove,
        io: &app.settings,
        plugin_knob: &|_| None,
    };
    channel::every_target(&ctx)
}

/// The channel page the Control area shows (CHANNEL: the focused sound's, group's or
/// the master's properties), for the controller: its page names, which is shown, and
/// the page (knobs and routing choices).
pub(crate) fn channel_page(app: &App, doc: &deckard_doc::Document) -> Option<(Vec<String>, usize, channel::Page)> {
    let p = doc.project();
    let level = match app.chain_tab {
        ChainTab::Master => Level::Master,
        ChainTab::Group => {
            p.groups.get(app.group)?;
            Level::Group(app.group)
        }
        ChainTab::Sound => {
            p.groups.get(app.group)?.sounds.get(app.sound)?;
            Level::Sound(app.group, app.sound)
        }
    };
    let groove = |g: usize, s: usize, f: &str| app.groove_setting(g, s, f);
    let on = |prefix: &str, v: &Vec<String>| -> Vec<String> {
        v.iter().filter(|n| !app.settings.midi_off.contains(&format!("{prefix}{n}"))).cloned().collect()
    };
    let (midi_ins, midi_outs) = (on("in:", &app.midi_ports.0), on("out:", &app.midi_ports.1));
    let ctx = channel::Ctx {
        doc,
        has_midi_macro: matches!(level, Level::Sound(g, s) if doc.param_oid(Target::Sound(g, s), "midi_macro/parameters/p0").is_some()),
        p,
        midi_ins: &midi_ins,
        midi_outs: &midi_outs,
        polyphonic: app.piano_roll,
        groove: &groove,
        io: &app.settings,
        plugin_knob: &|id| plugin_macro_knob(app, doc, id),
    };
    let set = app.prop_set.min(3);
    let macro_pages = if set == 3 { doc.macro_pages(channel::macro_channel(level)) } else { Vec::new() };
    let macros: Vec<String> = macro_pages.iter().enumerate().map(|(i, m)| channel::macro_page_name(m, i)).collect();
    let names = channel::page_names(level, set, &macros);
    let page = app.prop_page.min(names.len().saturating_sub(1));
    let pg = if set == 3 {
        let targets = channel::macro_targets(&ctx, level);
        channel::macro_page(&ctx, level, page, &targets).0
    } else {
        channel::page(&ctx, level, set, page)
    };
    Some((names, page, pg))
}

/// Set parameter `id` of the plug-in in slot `i` of chain `c` (normalized).
fn set_plugin_param(app: &mut App, chain: deckard_doc::Chain, i: usize, id: u32, v: f64) {
    let mut native = false;
    if let Ok(p) = app.slot_plugin_in(chain, i)
        && let Ok(mut g) = p.lock()
    {
        // A VST3 through its controller, not holding the instance while the plug-in
        // takes it in (the audio thread would wait).
        match g.params() {
            Some(h) => {
                drop(g);
                h.set(id, v);
            }
            None => {
                g.set_param(id, v);
                native = g.is_native();
            }
        }
    }
    // DECKARD's own plug-ins save into the project as they change (an undo step, like
    // Maschine's modules); hosted ones when the project is saved.
    if native {
        let key = app.plugin_slot_key(chain, i);
        app.store_plugin_states_of(|s| *s == key);
    }
    if let Some(view) = app.plugin_view.as_mut().filter(|v| v.key == (chain, i)) {
        view.page = usize::MAX;
        // (Shown until the plug-in reports it: the next turn goes on from it.)
        view.sent.retain(|(x, ..)| *x != id);
        view.sent.push((id, v, std::time::Instant::now()));
    }
}

/// The object id of the parameter a knob's edit changes (a plug-in's once its pages
/// are saved): what macros point at.
pub(crate) fn edit_param_oid(app: &App, d: &deckard_doc::Document, e: Edit) -> Option<u64> {
    match e {
        Edit::Plugin(i, id, _) | Edit::PluginChoice(i, id, ..) => d.plugin_param_oid(app.chain(), i, id),
        Edit::PluginIn(c, i, id, _) => d.plugin_param_oid(c, i, id),
        e => channel::knob_param(&e).and_then(|(t, f)| d.param_oid(t, f)),
    }
}

/// The knob of a macro controlling plug-in parameter object `id`: the parameter's name
/// and value, as on the plug-in's page (the plug-in must be running), from any chain.
fn plugin_macro_knob(app: &App, doc: &deckard_doc::Document, id: u64) -> Option<Knob> {
    let (chain, slot, param) = doc.plugin_param_at(id)?;
    plugin_knob(app, doc, chain, slot, param)
}

/// The knob of parameter `param` of the plug-in in slot `slot` of `chain`, as on its
/// page (the plug-in must be running).
fn plugin_knob(app: &App, doc: &deckard_doc::Document, chain: deckard_doc::Chain, slot: usize, param: u32) -> Option<Knob> {
    let plugin_id = doc.chain_slots(chain)?.get(slot)?.plugin.as_ref()?.id.clone();
    let p = app.samples.plugins.existing(app.plugin_slot_key(chain, slot))?;
    let info = {
        let mut cache = app.plugin_params.borrow_mut();
        if cache.get(&(chain, slot)).is_none_or(|(k, _)| *k != plugin_id) {
            let g = p.try_lock().ok()?;
            let mut list = g.parameter_list(false);
            // (Named as its page names them: a plug-in's own pages by the short names.)
            if !g.pages().is_empty() {
                list.iter_mut().filter(|i| !i.short.is_empty()).for_each(|i| i.title = i.short.clone());
            }
            cache.insert((chain, slot), (plugin_id, list));
        }
        cache.get(&(chain, slot))?.1.iter().find(|i| i.id == param)?.clone()
    };
    // (A VST3's value from its controller, not holding the instance: the audio thread
    // would wait.)
    let (value, text) = match p.try_lock() {
        Ok(g) => match g.params() {
            Some(h) => {
                drop(g);
                let v = h.value(param);
                (v, h.text(param, v))
            }
            None => {
                let v = g.param_value(param);
                (v, g.param_text(param, v))
            }
        },
        Err(_) => (info.default, String::new()),
    };
    let units = if info.units.is_empty() || text.ends_with(&info.units) { String::new() } else { format!(" {}", info.units) };
    let edit =
        if chain == app.chain() { Edit::Plugin(slot, param, info.default) } else { Edit::PluginIn(chain, slot, param, info.default) };
    Some((knob_label(&info.title).into(), format!("{text}{units}"), value as f32, Some(edit)))
}

/// The page the Control area shows for slot `i` of the focused sound (the Sampler's,
/// a plug-in's whose parameters are loaded, or one of Maschine's own modules), for the
/// controller's Control page.
pub(crate) fn slot_page(app: &App, p: &Project, i: usize) -> Option<SlotPage> {
    slot_page_as(app, p, i, false)
}

/// [`slot_page`] as shown: built-in knobs where their routes have them now (their
/// values, for turns and modulation, are [`slot_page`]'s).
pub(crate) fn live_slot_page(app: &App, p: &Project, i: usize) -> Option<SlotPage> {
    slot_page_as(app, p, i, true)
}

fn slot_page_as(app: &App, p: &Project, i: usize, live: bool) -> Option<SlotPage> {
    let chain = app.chain_slots();
    let slot = chain.get(i)?;
    // The Sampler's and the Audio plug-in's pages are a sound's.
    let none = deckard_model::Sound::default();
    let s = match app.chain() {
        deckard_doc::Chain::Sound(g, s) => p.groups.get(g)?.sounds.get(s)?,
        _ => &none,
    };
    if slot.module == "Sampler"
        && let Some(smp) = &s.sampler
    {
        let page = app.sampler_page.min(sampler_ui::PAGES.len() - 1);
        let smp = if live { app.live_sampler(app.group, app.sound, s, smp) } else { Cow::Borrowed(smp) };
        let pg = sampler_ui::page(&smp, app.group, app.sound, page);
        return Some(SlotPage {
            names: sampler_ui::PAGES.iter().map(|x| x.to_string()).collect(),
            page,
            knobs: pg.slots.into_iter().collect(),
            sections: pg.sections.into_iter().map(|(i, n)| (i, n.into())).collect(),
            show: Act::SamplerPage,
            routes: Vec::new(),
        });
    }
    if slot.module == "AudioModule"
        && let Some(a) = &s.audio
    {
        let pg = crate::audio_ui::page(app, a, app.group, app.sound, i);
        return Some(SlotPage {
            names: vec!["Main".into()],
            page: 0,
            knobs: pg.knobs,
            sections: pg.sections,
            show: Act::PluginPage,
            routes: Vec::new(),
        });
    }
    if let Some(pr) = &slot.plugin {
        let v = app.plugin_view.as_ref().filter(|v| v.key == (app.chain(), i) && v.error.is_none())?;
        let pages = v.pages.len().max(1);
        let page = app.plugin_page.min(pages - 1);
        let mut knobs = Vec::new();
        let mut sections = Vec::new();
        if let Some(pg) = v.pages.get(page) {
            for (k, (info, label, section)) in pg.knobs.iter().enumerate() {
                let (value, text) = v.values.get(k).cloned().unwrap_or((info.default, String::new()));
                let units = if info.units.is_empty() || text.ends_with(&info.units) { String::new() } else { format!(" {}", info.units) };
                let edit = match pg.choices.get(k).filter(|c| !c.is_empty()) {
                    Some(names) => {
                        let cur = (value * info.steps as f64).round() as usize;
                        Edit::PluginChoice(i, info.id, info.steps, cur.min(names.len() - 1), info.default)
                    }
                    None => Edit::Plugin(i, info.id, info.default),
                };
                knobs.push(Some((knob_label(label).into(), format!("{text}{units}"), value as f32, Some(edit))));
                if !section.is_empty() {
                    sections.push((k, section.clone().into()));
                }
            }
        }
        let names = v
            .pages
            .iter()
            .enumerate()
            .map(|(n, x)| if x.name.is_empty() { format!("{} {}", pr.name, n + 1) } else { x.name.clone() })
            .collect();
        return Some(SlotPage { names, page, knobs, sections, show: Act::PluginPage, routes: Vec::new() });
    }
    let names = crate::module_ui::pages(&slot.module)?;
    let page = app.plugin_page.min(names.len().saturating_sub(1));
    let slot = if live { app.live_slot((app.group, app.sound, i), s, slot) } else { Cow::Borrowed(slot) };
    let pg = crate::module_ui::page_for(&slot, app.module_target(i), page);
    let mut knobs: Vec<Option<Knob>> = pg.slots.into_iter().collect();
    let mut routes = Vec::new();
    // The Side-Chain Input page's SOURCE: what the effect listens to, as the Control
    // area shows it.
    if names.get(page) == Some(&"Side-Chain Input")
        && let Some((choices, sets, cur)) = channel::sidechain_choices(p, app.chain(), i)
        && let Some(k) = knobs.first_mut()
    {
        let shown = channel::abbrev(choices[cur].split(" - ").next().unwrap_or(""));
        *k = Some(("SOURCE".into(), shown, 0.0, Some(Edit::Route(cur))));
        routes.push((0, choices, sets));
    }
    Some(SlotPage {
        names: names.iter().map(|x| x.to_string()).collect(),
        page,
        knobs,
        sections: pg.sections.into_iter().map(|(i, n)| (i, n.into())).collect(),
        show: Act::PluginPage,
        routes,
    })
}

/// Every knob on every page of slot `i` of sound (`g`, `s`), as the Control area shows
/// them (a plug-in's only while it runs): what a route there can drive.
pub(crate) fn slot_knobs(app: &App, doc: &deckard_doc::Document, g: usize, s: usize, i: usize) -> Vec<Knob> {
    let p = doc.project();
    let Some(snd) = p.groups.get(g).and_then(|x| x.sounds.get(s)) else { return Vec::new() };
    let Some(slot) = snd.slots.get(i) else { return Vec::new() };
    let mut out: Vec<Knob> = Vec::new();
    if slot.module == "Sampler"
        && let Some(smp) = &snd.sampler
    {
        for page in 0..sampler_ui::PAGES.len() {
            out.extend(sampler_ui::page(smp, g, s, page).slots.into_iter().flatten());
        }
    } else if slot.module == "AudioModule"
        && let Some(a) = &snd.audio
    {
        out.extend(crate::audio_ui::page(app, a, g, s, i).knobs.into_iter().flatten());
    } else if slot.plugin.is_some() {
        let chain = deckard_doc::Chain::Sound(g, s);
        for pg in app.plugin_pages(chain, i) {
            out.extend(pg.knobs.iter().filter_map(|(id, _)| plugin_knob(app, doc, chain, i, *id)));
        }
    } else if let Some(names) = crate::module_ui::pages(&slot.module) {
        for page in 0..names.len() {
            out.extend(crate::module_ui::page_for(slot, Target::Module(g, s, i), page).slots.into_iter().flatten());
        }
    }
    out
}

/// The groove knobs (Maschine's Groove properties, Swing page): amount, cycle, invert.
fn swing_knobs(t: Target, s: Swing) -> Vec<Knob> {
    let cycle = s.cycle.clamp(0, 7) as usize;
    vec![
        ("SWING".into(), format!("{:.0} %", s.amount), (s.amount / 100.0) as f32, Some(Edit::Swing(t, s, SwingField::Amount))),
        ("CYCLE".into(), SWING_CYCLES[cycle].0.to_string(), cycle as f32 / 7.0, Some(Edit::Swing(t, s, SwingField::Cycle))),
        (
            "INVERT".into(),
            if s.invert { "On" } else { "Off" }.to_string(),
            if s.invert { 1.0 } else { 0.0 },
            Some(Edit::Swing(t, s, SwingField::Invert)),
        ),
    ]
}

fn chain_area(ui: &mut Ui, app: &App, doc: &deckard_doc::Document, rect: Rect, acts: &mut Vec<Act>) {
    let p = doc.project();
    let painter = ui.painter_at(rect);
    let strip_r = Rect::from_min_size(rect.min, Vec2::new(STRIP, rect.height()));
    painter.rect_filled(strip_r, 0.0, S05);
    // The far-left icons: channel properties (a knob) and plug-ins (a plug).
    for (k, (on, tip, draw)) in [
        (app.channel, "Channel properties", channel_icon as fn(&Painter, Rect, Color32)),
        (!app.channel, "Plug-ins", plug_icon as fn(&Painter, Rect, Color32)),
    ]
    .into_iter()
    .enumerate()
    {
        let b = Rect::from_min_size(Pos2::new(strip_r.left(), strip_r.top() + 5.0 + k as f32 * 27.0), Vec2::new(STRIP, 24.0));
        let resp = ui.interact(b, ui.id().with(("chain-mode", k)), Sense::click()).on_hover_text(tip);
        ui.data_mut(|d| d.insert_temp(egui::Id::new(("deckard-chain-mode", k)), b));
        let ink = if on {
            white(245)
        } else if resp.hovered() {
            white(164)
        } else {
            white(90)
        };
        draw(&painter, b, ink);
        if resp.clicked() {
            acts.push(Act::ChannelMode(k == 0));
        }
    }
    // At the bottom: the Assignment area's arrow (down: open it; up: close it).
    {
        let open = app.remote.assign.is_some();
        let b = Rect::from_min_size(Pos2::new(strip_r.left(), strip_r.bottom() - 26.0), Vec2::new(STRIP, 22.0));
        let resp = ui.interact(b, ui.id().with("assign-toggle"), Sense::click()).on_hover_text("Assignment area");
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Assignment area"));
        ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-assign-toggle"), b));
        let ink = if resp.hovered() || open { white(220) } else { white(120) };
        let (m, d) = (b.center(), if open { -1.0 } else { 1.0 });
        painter.add(egui::Shape::convex_polygon(
            vec![m + Vec2::new(-5.0, -2.5 * d), m + Vec2::new(5.0, -2.5 * d), m + Vec2::new(0.0, 3.0 * d)],
            ink,
            Stroke::NONE,
        ));
        if resp.clicked() {
            let tab = if open { None } else { Some(if app.channel && app.prop_set == 3 { AssignTab::Pages } else { AssignTab::Midi }) };
            acts.push(Act::Remote(RemoteAct::Assign(tab)));
        }
    }
    let col = Rect::from_min_size(Pos2::new(rect.left() + STRIP + 1.0, rect.top()), Vec2::new(COLUMN, rect.height()));
    let ed = Rect::from_min_max(Pos2::new(col.right() + 1.0, rect.top()), rect.max);
    painter.rect_filled(col, 0.0, S05);
    painter.rect_filled(ed, 0.0, S05);
    let assign_h = if app.remote.assign.is_some() { assign::ASSIGN_H } else { 0.0 };
    let group = p.groups.get(app.group);
    let sound = group.and_then(|g| g.sounds.get(app.sound));
    let gcol = if group.is_some() { group_color(p, app.group) } else { white(245) };

    // Tabs.
    let mut x = col.left() + 6.0;
    for (tab, name) in [(ChainTab::Master, "MASTER"), (ChainTab::Group, "GROUP"), (ChainTab::Sound, "SOUND")] {
        // Each tab as wide as its name and a little room, 8 apart.
        let w = painter.layout_no_wrap(name.to_string(), font(12.0), Color32::WHITE).size().x + 4.0;
        let r = Rect::from_min_size(Pos2::new(x, col.top()), Vec2::new(w, 24.0));
        let resp = ui.interact(r, ui.id().with(("tab", name)), Sense::click());
        let ink = if app.chain_tab == tab {
            white(245)
        } else if resp.hovered() {
            white(164)
        } else {
            white(142)
        };
        painter.text(r.center(), Align2::CENTER_CENTER, name, font(12.0), ink);
        if app.chain_tab == tab {
            painter.hline(r.shrink2(Vec2::new(4.0, 0.0)).x_range(), r.bottom() - 1.0, Stroke::new(2.0, white(245)));
        }
        if resp.clicked() {
            acts.push(Act::ChainTab(tab));
        }
        x += w + 8.0;
    }
    let title = match app.chain_tab {
        ChainTab::Master => "Master".to_string(),
        ChainTab::Group => group.map_or(String::new(), |_| group_name(p, app.group)),
        ChainTab::Sound => {
            sound.map_or(String::new(), |s| if s.name.is_empty() { format!("Sound {}", app.sound + 1) } else { s.name.clone() })
        }
    };
    let title_r = Rect::from_min_size(Pos2::new(col.left(), col.top() + 24.0), Vec2::new(COLUMN, 32.0));
    let on_sound = app.chain_tab == ChainTab::Sound && sound.is_some();
    label(&painter, title_r, 8.0, &title, 15.0, white(245));
    let level = match app.chain_tab {
        ChainTab::Master => Some(Level::Master),
        ChainTab::Group => group.map(|_| Level::Group(app.group)),
        ChainTab::Sound => sound.map(|_| Level::Sound(app.group, app.sound)),
    };
    let below_title = Rect::from_min_max(Pos2::new(col.left(), title_r.bottom() + 2.0), col.max - Vec2::new(0.0, assign_h));
    let chain_slots_owned = app.chain_slots();
    let slots: &[deckard_model::SlotInfo] = if level.is_some() { &chain_slots_owned } else { &[] };
    if app.channel {
        property_tiles(ui, app, &painter, below_title, level, gcol, acts);
    } else if level.is_some() {
        chain_slots(ui, app, &painter, below_title, slots, acts);
    } else {
        label(&painter, Rect::from_min_size(below_title.min, Vec2::new(COLUMN, 24.0)), 10.0, "No plug-ins here yet", 11.0, white(90));
    }
    // The sound whose Sampler or Audio plug-in pages show (none on the group's or the
    // master's chain).
    let no_sound = deckard_model::Sound::default();
    let shown_sound = if on_sound { sound } else { level.map(|_| &no_sound) };
    let selected = if app.channel { None } else { app.slot.and_then(|i| slots.get(i).map(|x| (i, x))) };

    // Parameters: up to eight knob slots (empty ones None), with section headings.
    let header = Rect::from_min_size(ed.min, Vec2::new(ed.width(), 32.0));
    let mut params: Vec<Option<Knob>> = Vec::new();
    // Knobs' edits start from their own values (set below where routes move them).
    mod_routes::set_bases(ui, Vec::new());
    let mut sections: Vec<(usize, Cow<'static, str>)> = Vec::new();
    let heading: String;
    let mut note: Option<String> = None;
    let mut knobs_x = ed.left() + 13.0;
    let mut tabs: Option<PageTabs> = None;
    let mut window_button = None;
    let mut waveform_button = false;
    let mut routes: Vec<(usize, Vec<String>, Vec<RouteSet>)> = Vec::new();
    // Knobs that are a plug-in's choices: (knob, the names of its values).
    let mut plugin_choices: Vec<(usize, Vec<String>)> = Vec::new();
    let mut groove_button = None;
    let mut hint: Option<String> = None;
    let groove = |g: usize, s: usize, f: &str| app.groove_setting(g, s, f);
    // The ports switched on in Preferences.
    let on = |prefix: &str, v: &Vec<String>| -> Vec<String> {
        v.iter().filter(|n| !app.settings.midi_off.contains(&format!("{prefix}{n}"))).cloned().collect()
    };
    let (midi_ins, midi_outs) = (on("in:", &app.midi_ports.0), on("out:", &app.midi_ports.1));
    let ctx = channel::Ctx {
        doc,
        has_midi_macro: matches!(level, Some(Level::Sound(g, s)) if doc.param_oid(Target::Sound(g, s), "midi_macro/parameters/p0").is_some()),
        p,
        midi_ins: &midi_ins,
        midi_outs: &midi_outs,
        polyphonic: app.piano_roll,
        groove: &groove,
        io: &app.settings,
        plugin_knob: &|id| plugin_macro_knob(app, doc, id),
    };
    // On the Macro page: the channel, the page, what each slot controls, and the targets.
    let mut macro_view: Option<MacroView> = None;
    let mut macro_tabs = false;
    if app.channel {
        let set = app.prop_set.min(3);
        heading = channel::SETS[set].to_string();
        if let Some(level) = level {
            let macro_pages = if set == 3 { doc.macro_pages(channel::macro_channel(level)) } else { Vec::new() };
            let macros: Vec<String> = macro_pages.iter().enumerate().map(|(i, m)| channel::macro_page_name(m, i)).collect();
            let mut names = channel::page_names(level, set, &macros);
            let page = app.prop_page.min(names.len().saturating_sub(1));
            if set == 3 {
                let targets = channel::macro_targets(&ctx, level);
                let (pg, which) = channel::macro_page(&ctx, level, page, &targets);
                params = pg.slots;
                sections = pg.sections;
                macro_view = Some((level, page, which, targets));
                names.push("+".into());
                macro_tabs = true;
            } else {
                let pg = channel::page(&ctx, level, set, page);
                params = pg.slots;
                sections = pg.sections;
                routes = pg.routes;
                groove_button = pg.button;
                note = pg.note;
            }
            if !names.is_empty() {
                tabs = Some((names, page, Act::PropPage));
            }
            if let Level::Sound(g, s) = level
                && set == 2
                && page > 0
                && !doc.has_groove_settings(g, s)
            {
                hint = Some("Kept for this session (this project's format has no place for them)".into());
            }
        }
    } else {
        match app.chain_tab {
            ChainTab::Master | ChainTab::Group | ChainTab::Sound => match (shown_sound, selected) {
                (Some(_), None) => {
                    heading = "Plug-ins".into();
                    note = Some("Pick a plug-in on the left, or add one with +".into());
                }
                (Some(s), Some((i, slot))) if slot.module == "Sampler" && s.sampler.is_some() => {
                    // Where its routes have its knobs now.
                    let (live, bases) = app.live_sampler_bases(app.group, app.sound, s, s.sampler.as_ref().unwrap());
                    mod_routes::set_bases(ui, bases);
                    let smp = &*live;
                    let page = app.sampler_page.min(sampler_ui::PAGES.len() - 1);
                    heading = format!("Sampler: {}", sampler_ui::PAGES[page]);
                    let pg = sampler_ui::page(smp, app.group, app.sound, page);
                    params = pg.slots.into_iter().collect();
                    sections = pg.sections.into_iter().map(|(i, n)| (i, n.into())).collect();
                    tabs = Some((sampler_ui::PAGES.iter().map(|x| x.to_string()).collect(), page, Act::SamplerPage));
                    let zi = app.zone.min(smp.zones.len().saturating_sub(1));
                    let wf = app.waveform.as_ref().filter(|w| (w.key.0, w.key.1, w.key.2) == (app.group, app.sound, zi));
                    waveform_button = true;
                    if let (Some(w), Some(z), true) = (wf, smp.zones.get(zi), app.settings.show_waveform) {
                        let r = Rect::from_min_size(
                            Pos2::new(ed.left() + 10.0, header.bottom() + 8.0),
                            Vec2::new(360.0, rect.bottom() - header.bottom() - 16.0),
                        );
                        let zt = Target::Zone(app.group, app.sound, zi);
                        sampler_ui::waveform(ui, r, w, z, zt, gcol, app.zone_playhead, acts);
                        sampler_ui::loop_switch(ui, r, z, zt, acts);
                        knobs_x = r.right() + 14.0;
                    }
                    if !slot.active {
                        note = Some("Bypassed".into());
                    }
                    let _ = i;
                }
                (Some(_), Some((i, slot))) if slot.plugin.is_some() => {
                    let pr = slot.plugin.as_ref().unwrap();
                    // DECKARD's own plug-ins have no window: their pages are the interface.
                    if !pr.id.starts_with(deckard_plugins::native::VENDOR) {
                        window_button = Some(i);
                    }
                    match app.plugin_view.as_ref().filter(|v| v.key == (app.chain(), i)) {
                        Some(v) if v.error.is_none() => {
                            let pages = v.pages.len().max(1);
                            let page = app.plugin_page.min(pages - 1);
                            let pname = v.pages.get(page).map(|p| p.name.clone()).filter(|n| !n.is_empty());
                            heading = match pname {
                                Some(n) => format!("{}: {n}", pr.name),
                                None => format!("{}: Page {} of {}", pr.name, page + 1, pages),
                            };
                            // Named pages (DECKARD's plug-ins name theirs) by name, else by number.
                            let names: Vec<String> = if !v.pages.is_empty() && v.pages.iter().all(|p| !p.name.is_empty()) {
                                v.pages.iter().map(|p| p.name.clone()).collect()
                            } else {
                                (1..=pages).map(|n| n.to_string()).collect()
                            };
                            tabs = Some((names, page, Act::PluginPage));
                            if let Some(pg) = v.pages.get(page) {
                                for (k, (info, label, section)) in pg.knobs.iter().enumerate() {
                                    let (value, text) = v.values.get(k).cloned().unwrap_or((info.default, String::new()));
                                    let name: Cow<'static, str> = knob_label(label).into();
                                    let units = if info.units.is_empty() || text.ends_with(&info.units) {
                                        String::new()
                                    } else {
                                        format!(" {}", info.units)
                                    };
                                    let names = pg.choices.get(k).filter(|c| !c.is_empty());
                                    let edit = match names {
                                        Some(names) => {
                                            plugin_choices.push((k, names.clone()));
                                            let cur = (value * info.steps as f64).round() as usize;
                                            Edit::PluginChoice(i, info.id, info.steps, cur.min(names.len() - 1), info.default)
                                        }
                                        None => Edit::Plugin(i, info.id, info.default),
                                    };
                                    params.push(Some((name, format!("{text}{units}"), value as f32, Some(edit))));
                                    if !section.is_empty() {
                                        sections.push((k, section.clone().into()));
                                    }
                                }
                            }
                            if v.pages.is_empty() {
                                note = Some("This plug-in has no parameters to show".into());
                            }
                        }
                        Some(v) => {
                            heading = format!("{}: Plug-in", pr.name);
                            note = v.error.clone();
                        }
                        None => heading = format!("{}: Plug-in", pr.name),
                    }
                }
                (Some(s), Some((i, slot))) if slot.module == "AudioModule" && s.audio.is_some() => {
                    heading = "Audio: Main".into();
                    let pg = crate::audio_ui::page(app, s.audio.as_ref().unwrap(), app.group, app.sound, i);
                    params = pg.knobs;
                    sections = pg.sections;
                    if !slot.active {
                        hint = Some("Bypassed".into());
                    }
                }
                (Some(s), Some((i, slot))) if crate::module_ui::pages(&slot.module).is_some() => {
                    // One of Maschine's own effects (or synths), where its routes have
                    // its knobs now.
                    let names = crate::module_ui::pages(&slot.module).unwrap_or(&[]);
                    let page = app.plugin_page.min(names.len().saturating_sub(1));
                    heading = format!("{}: {}", slot.name, names[page]);
                    let t = app.module_target(i);
                    let (live, bases) = app.live_slot_bases((app.group, app.sound, i), s, slot, t);
                    mod_routes::set_bases(ui, bases);
                    let pg = crate::module_ui::page_for(&live, t, page);
                    params = pg.slots.into_iter().collect();
                    if names.get(page) == Some(&"Side-Chain Input")
                        && let Some((names, sets, cur)) = channel::sidechain_choices(p, app.chain(), i)
                    {
                        // The Source: a list of what the effect can listen to.
                        let shown = channel::abbrev(names[cur].split(" - ").next().unwrap_or(""));
                        params[0] = Some(("SOURCE".into(), shown, 0.0, Some(Edit::Route(cur))));
                        routes.push((0, names, sets));
                    }
                    sections = pg.sections.into_iter().map(|(i, n)| (i, n.into())).collect();
                    if names.len() > 1 {
                        tabs = Some((names.iter().map(|x| x.to_string()).collect(), page, Act::PluginPage));
                    }
                    if !slot.active {
                        hint = Some("Bypassed".into());
                    }
                }
                (Some(_), Some((_, slot))) => {
                    heading = slot.name.clone();
                    note = Some(format!("{}'s parameters aren't available yet", slot.name));
                }
                _ => heading = "Main".into(),
            },
        }
    }
    label(&painter, header, 10.0, &heading, 12.0, white(245));
    if let Some(h) = &hint {
        painter.text(Pos2::new(header.right() - 10.0, header.center().y), Align2::RIGHT_CENTER, h, font(10.0), white(110));
    }
    painter.hline(header.x_range(), header.bottom(), Stroke::new(1.0, Color32::BLACK));
    let mut tabs_left = header.right();
    if let Some(i) = window_button {
        let b = Rect::from_min_size(Pos2::new(header.right() - 130.0, header.top() + 5.0), Vec2::new(120.0, 22.0));
        let resp = ui.interact(b, ui.id().with("plugin-window"), Sense::click());
        painter.rect_filled(b, 2.0, if resp.hovered() { S03 } else { S02 });
        painter.text(b.center(), Align2::CENTER_CENTER, "PLUG-IN WINDOW", font(11.0), white(if resp.hovered() { 245 } else { 164 }));
        if resp.clicked() {
            acts.push(Act::OpenSlotEditor(i));
        }
        tabs_left = b.left() - 8.0;
    }
    if waveform_button {
        // Show or hide the waveform (lit while shown).
        let b = Rect::from_min_size(Pos2::new(header.right() - 38.0, header.top() + 5.0), Vec2::new(28.0, 22.0));
        let on = app.settings.show_waveform;
        let resp = ui.interact(b, ui.id().with("waveform-toggle"), Sense::click()).on_hover_text(if on {
            "Hide the waveform"
        } else {
            "Show the waveform"
        });
        painter.rect_filled(b, 2.0, if resp.hovered() { S03 } else { S02 });
        wave_icon(&painter, b, if on { gcol } else { white(142) });
        if resp.clicked() {
            acts.push(Act::ToggleWaveform);
        }
        tabs_left = b.left() - 8.0;
    }
    if let Some((names, page, act)) = tabs {
        // Left-aligned, after the heading.
        let heading_w = painter.layout_no_wrap(heading.clone(), font(12.0), Color32::WHITE).size().x;
        let left = header.left() + 10.0 + heading_w + 24.0;
        let tab_resps = page_tabs(
            ui,
            &painter,
            Rect::from_min_max(Pos2::new(left, header.top()), Pos2::new(tabs_left, header.bottom())),
            &names,
            page,
            act,
            acts,
        );
        // Macro pages: right-click a page for Rename, Delete, Clear All.
        if macro_tabs && let Some((level, ..)) = &macro_view {
            let ch = channel::macro_channel(*level);
            for (i, resp) in tab_resps.iter().enumerate().take(names.len() - 1) {
                resp.context_menu(|ui| {
                    let id = ui.id().with(("macro-page-name", i));
                    let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| names[i].clone());
                    ui.label("Name");
                    let te = ui.text_edit_singleline(&mut text);
                    ui.data_mut(|d| d.insert_temp(id, text.clone()));
                    if te.lost_focus() && ui.input(|x| x.key_pressed(egui::Key::Enter)) {
                        acts.push(Act::Macro(MacroAct::RenamePage(ch, i, text)));
                        ui.data_mut(|d| d.remove::<String>(id));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Delete").on_hover_text("Delete this page with its macros").clicked() {
                        acts.push(Act::Macro(MacroAct::DeletePage(ch, i)));
                        ui.close();
                    }
                    if ui.button("Clear All").on_hover_text("Delete every macro and page").clicked() {
                        acts.push(Act::Macro(MacroAct::Clear(ch)));
                        ui.close();
                    }
                });
            }
        }
    }
    if let Some(n) = &note {
        painter.text(Pos2::new(knobs_x, header.bottom() + 20.0), Align2::LEFT_CENTER, n, font(12.0), white(142));
    }
    // Section headings: from their slot to the next section.
    let slot_x = |i: usize| knobs_x + i as f32 * 83.0;
    for (k, (i, name)) in sections.iter().enumerate() {
        let end = sections.get(k + 1).map_or(params.len().max(i + 1), |(j, _)| *j);
        let (x0, x1) = (slot_x(*i), slot_x(end) - 13.0);
        let y = header.bottom() + 9.0;
        let t = painter.text(Pos2::new(x0, y), Align2::LEFT_CENTER, name.as_ref(), font(10.0), gcol);
        if t.right() + 6.0 < x1 {
            painter.hline((t.right() + 6.0)..=x1, y, Stroke::new(1.0, white(40)));
        }
    }
    let knob_y = header.bottom() + if sections.is_empty() { 16.0 } else { 18.0 };
    // Where the knobs are, for tests that drive the window.
    let rects: Vec<(String, Rect)> = params
        .iter()
        .enumerate()
        .filter_map(|(i, k)| Some((k.as_ref()?.0.to_string(), Rect::from_min_size(Pos2::new(slot_x(i), knob_y), Vec2::new(70.0, 84.0)))))
        .collect();
    ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-knobs"), rects));
    if let Some(b) = groove_button {
        // After the eight slots: a divider, then the button.
        let x = slot_x(8) - 6.0;
        painter.vline(x, (knob_y + 6.0)..=(knob_y + 70.0), Stroke::new(1.0, S02));
        let br = Rect::from_min_size(Pos2::new(x + 9.0, knob_y + 30.0), Vec2::new(70.0, 24.0));
        let text = match b {
            GrooveButton::ApplyVariation => "Apply",
            GrooveButton::Generate => "Generate",
        };
        ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-groove-button"), br));
        let resp = ui.interact(br, ui.id().with("groove-button"), Sense::click()).on_hover_text(match b {
            GrooveButton::ApplyVariation => "Vary the focused sound's notes in the focused pattern",
            GrooveButton::Generate => "Make new notes for the focused sound in the focused pattern",
        });
        painter.rect_filled(br, 3.0, if resp.hovered() { S03 } else { S02 });
        painter.text(br.center(), Align2::CENTER_CENTER, text, font(11.0), white(if resp.hovered() { 245 } else { 164 }));
        if resp.clicked() {
            acts.push(Act::Groove(b));
        }
    }
    let knob_rects: Vec<Option<Rect>> = params
        .iter()
        .enumerate()
        .map(|(i, k)| k.as_ref().map(|_| Rect::from_min_size(Pos2::new(slot_x(i), knob_y), Vec2::new(70.0, 84.0))))
        .collect();
    // Where the knobs are, for tests to point at.
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(egui::Id::new("knob-rects"), knob_rects.clone()));
    let macro_targets = if macro_view.is_none() { doc.macro_targets() } else { Default::default() };
    for (i, k) in params.iter().enumerate() {
        let Some((name, value, amount, edit)) = k else { continue };
        let r = Rect::from_min_size(Pos2::new(slot_x(i), knob_y), Vec2::new(70.0, 84.0));
        // The name strip: right-click for the macro menu.
        let strip = Rect::from_min_size(r.min, Vec2::new(r.width(), 16.0));
        let strip_resp = ui.interact(strip, ui.id().with(("param-name", i)), Sense::click());
        if let Some((level, page, _, targets)) = &macro_view {
            let ch = channel::macro_channel(*level);
            let (page, slot) = (*page, i);
            let assigned = doc.macro_pages(ch).get(page).and_then(|p| p.macros.get(slot)).is_some_and(|m| m.target.is_some());
            strip_resp.context_menu(|ui| assign::macro_slot_menu(ui, app, (ch, page, slot), assigned, acts));
            if !assigned {
                // An empty macro: a click picks what it controls.
                let body = Rect::from_min_max(Pos2::new(r.left(), strip.bottom()), r.max);
                let resp = ui
                    .interact(body, ui.id().with(("macro-pick", i)), Sense::click())
                    .on_hover_text("Click to pick what this macro controls");
                if resp.hovered() {
                    painter.rect_filled(r, 4.0, white(10));
                }
                painter.text(Pos2::new(r.center().x, r.top() + 8.0), Align2::CENTER_CENTER, name.as_ref(), font(10.0), white(90));
                painter.circle_stroke(Pos2::new(r.center().x, r.top() + 42.0), 18.0, Stroke::new(1.0, white(30)));
                painter.text(
                    Pos2::new(r.center().x, r.top() + 42.0),
                    Align2::CENTER_CENTER,
                    "+",
                    font(14.0),
                    white(if resp.hovered() { 200 } else { 70 }),
                );
                egui::Popup::menu(&resp).show(|ui| macro_target_menu(ui, app, *level, (ch, page, slot), targets, acts));
                continue;
            }
        } else if let Some((t, f)) = edit.as_ref().and_then(channel::knob_param) {
            strip_resp.context_menu(|ui| param_macro_menu(ui, doc, t, f, acts));
        } else if let Some(Edit::Plugin(slot, id, _) | Edit::PluginChoice(slot, id, ..)) = *edit {
            strip_resp.context_menu(|ui| plugin_macro_menu(ui, doc, app.chain(), slot, id, acts));
        }
        // A parameter a macro controls: the macro icon at the top right.
        if macro_view.is_none() && edit.and_then(|e| edit_param_oid(app, doc, e)).is_some_and(|id| macro_targets.contains(&id)) {
            macro_icon(&painter, Pos2::new(r.right() - 7.0, r.top() + 8.0), white(142));
        }
        if let Some(Edit::Choice(t, f, options, cur, _)) = *edit {
            selector(ui, &painter, r, (i, name.as_ref(), value), options, cur, &|k| Act::SetParam(t, f, k as f64), gcol, acts);
            continue;
        }
        if let Some(Edit::PluginChoice(slot, id, steps, cur, _)) = *edit {
            if let Some((_, names)) = plugin_choices.iter().find(|(k, _)| *k == i) {
                let names: Vec<&str> = names.iter().map(String::as_str).collect();
                let set = move |k: usize| Act::PluginParam(slot, id, k as f64 / steps as f64);
                selector(ui, &painter, r, (i, name.as_ref(), value), &names, cur, &set, gcol, acts);
            }
            continue;
        }
        if let Some(Edit::Route(cur)) = *edit {
            if let Some((_, names, sets)) = routes.iter().find(|(slot, ..)| *slot == i) {
                let names: Vec<&str> = names.iter().map(String::as_str).collect();
                selector(ui, &painter, r, (i, name.as_ref(), value), &names, cur, &|k| Act::SetRoute(sets[k].clone()), gcol, acts);
            }
            continue;
        }
        if let Some(Edit::Toggle(t, f, on)) = *edit {
            // An on/off button, as Maschine's.
            painter.text(Pos2::new(r.center().x, r.top() + 8.0), Align2::CENTER_CENTER, name.as_ref(), font(10.0), white(142));
            let b = Rect::from_center_size(Pos2::new(r.center().x, r.top() + 42.0), Vec2::new(54.0, 22.0));
            let resp = ui.interact(b, ui.id().with(("toggle", i)), Sense::click());
            let fill = if on {
                gcol
            } else if resp.hovered() {
                S03
            } else {
                S02
            };
            painter.rect_filled(b, 3.0, fill);
            painter.text(
                b.center(),
                Align2::CENTER_CENTER,
                if on { "ON" } else { "OFF" },
                font(11.0),
                if on { Color32::BLACK } else { white(200) },
            );
            if resp.clicked() {
                // A Perform FX's Engage is the strip's touch.
                acts.push(match t {
                    Target::ChainModule(deckard_doc::Chain::Group(g), _) if f == "touchstrip_on_off" => Act::PerformEngage(g, !on),
                    _ => Act::SetParam(t, f, if on { 0.0 } else { 1.0 }),
                });
            }
            continue;
        }
        let mut amount = *amount;
        if let Some(edit) = edit {
            // Drag up/down (Shift: fine); double-click resets.
            let below = Rect::from_min_max(Pos2::new(r.left(), strip.bottom()), r.max);
            let resp = ui.interact(below, ui.id().with(("knob", i)), Sense::click_and_drag()).on_hover_text(
                "Drag to change (Shift: fine), double-click to reset; right-click to remove its modulation, the name for macros",
            );
            if resp.hovered() || resp.dragged() {
                painter.rect_filled(r, 4.0, white(10));
            }
            // Measured from where the drag started, so parameters that round (choices,
            // frames) still move with small steps.
            let fine = ui.input(|i| i.modifiers.shift);
            let start_id = ui.id().with(("knob-start", i));
            let ring = knob_drag_writes(app, *edit);
            // A knob a route moves shows where the route has it; edits start from its own value.
            let base = mod_routes::ui_edit_from(ui, edit, amount);
            if resp.drag_started() {
                ui.data_mut(|d| d.insert_temp(start_id, (base, 0f32)));
            }
            let dy = -resp.drag_delta().y / if fine { 1000.0 } else { 200.0 };
            if resp.dragged() && dy != 0.0 {
                let (from, moved) = ui.data(|d| d.get_temp::<(f32, f32)>(start_id)).unwrap_or((base, 0.0));
                let moved = moved + dy;
                ui.data_mut(|d| d.insert_temp(start_id, (from, moved)));
                let to = (from + moved).clamp(0.0, 1.0);
                if ring {
                    acts.push(Act::Modulate(*edit, to, false));
                } else {
                    amount = to;
                    acts.push(edit.to_act(amount));
                }
            }
            if resp.drag_stopped() && ring {
                let (from, moved) = ui.data(|d| d.get_temp::<(f32, f32)>(start_id)).unwrap_or((base, 0.0));
                acts.push(Act::Modulate(*edit, (from + moved).clamp(0.0, 1.0), true));
            }
            // ⌃-click (right-click) anywhere on the knob: its menu, to remove its
            // modulation.
            if modulation::mod_target(*edit).is_some() {
                let e = *edit;
                resp.context_menu(|ui| {
                    if ui.button("Remove Modulation").on_hover_text("Remove this knob's modulation from every pattern").clicked() {
                        acts.push(Act::ClearModulation(e));
                        ui.close();
                    }
                });
            }
            if resp.double_clicked() {
                acts.push(edit.reset());
            } else if resp.clicked()
                && let Some(a) = edit.click()
            {
                acts.push(a);
            }
        }
        let bipolar = matches!(edit, Some(Edit::Param(_, _, lo, _, _)) if *lo < 0.0) || matches!(edit, Some(Edit::Pan(_)));
        knob(&painter, r, name.as_ref(), value, amount, if edit.is_some() { gcol } else { white(142) }, bipolar);
        // A modulated knob: where the modulation has it now, a mark on the ring.
        if let Some(m) = edit.and_then(|e| app.mod_amount(e)) {
            let a = std::f32::consts::PI * (0.75 + 1.5 * m);
            let c = Pos2::new(r.center().x, r.top() + 40.0);
            painter.circle_filled(c + Vec2::angled(a) * 16.0, 3.0, Color32::from_rgb(0x9a, 0xd0, 0xff));
        }
    }
    if assign_h > 0.0 {
        let fields = assign::Fields {
            area: Rect::from_min_max(Pos2::new(ed.left(), rect.bottom() - assign_h), rect.max),
            tabs: Rect::from_min_max(Pos2::new(col.left(), rect.bottom() - assign_h), Pos2::new(col.right(), rect.bottom())),
            knobs: knob_rects,
            params: &params,
            macro_view: macro_view.as_ref(),
        };
        assign::area(ui, app, doc, &painter, fields, gcol, acts);
    }
}

/// A drag on knob `edit` is written as modulation: only with Auto-write on while the
/// song plays, and only where the knob can be modulated. (BUGS #44: a drag that began
/// on the knob's ring used to write it with Auto off, so turns got recorded "by
/// themselves"; Auto is the one switch, as on the MK3.)
pub(crate) fn knob_drag_writes(app: &App, edit: Edit) -> bool {
    modulation::mod_target(edit).is_some() && app.playing.is_some() && app.auto_write
}

/// The Macro page being shown: the channel, the page, what each slot controls (an
/// index into the targets), and the targets.
type MacroView = (Level, usize, Vec<Option<usize>>, Vec<channel::MacroTarget>);

/// The macro menu of a parameter's name: make it a macro of its channel or one
/// above, or stop it being one.
fn param_macro_menu(ui: &mut Ui, doc: &deckard_doc::Document, t: Target, f: &'static str, acts: &mut Vec<Act>) {
    let Some(level) = channel::param_level(t) else { return };
    let levels: Vec<(Level, &str)> = match level {
        Level::Sound(g, _) => vec![(level, "Sound"), (Level::Group(g), "Group"), (Level::Master, "Master")],
        Level::Group(_) => vec![(level, "Group"), (Level::Master, "Master")],
        Level::Master => vec![(level, "Master")],
    };
    let id = doc.param_oid(t, f);
    let mut any = false;
    for (l, name) in levels.iter().rev() {
        let ch = channel::macro_channel(*l);
        let has = id.is_some() && doc.macro_pages(ch).iter().any(|p| p.macros.iter().any(|m| m.target == id));
        if has {
            any = true;
            if ui.button(format!("Remove {name} Macro")).clicked() {
                acts.push(Act::Macro(MacroAct::Remove { ch, t, field: f }));
                ui.close();
            }
        } else if ui.add_enabled(id.is_some(), egui::Button::new(format!("Create {name} Macro"))).clicked() {
            acts.push(Act::Macro(MacroAct::Create { ch, t, field: f }));
            ui.close();
        }
    }
    let _ = any;
}

/// [`param_macro_menu`] for parameter `id` of the plug-in in slot `slot` of `chain`.
fn plugin_macro_menu(ui: &mut Ui, doc: &deckard_doc::Document, chain: deckard_doc::Chain, slot: usize, id: u32, acts: &mut Vec<Act>) {
    use deckard_doc::Chain;
    let levels: Vec<(Level, &str)> = match chain {
        Chain::Sound(g, s) => vec![(Level::Sound(g, s), "Sound"), (Level::Group(g), "Group"), (Level::Master, "Master")],
        Chain::Group(g) => vec![(Level::Group(g), "Group"), (Level::Master, "Master")],
        Chain::Master => vec![(Level::Master, "Master")],
    };
    let param = doc.plugin_param_oid(chain, slot, id);
    for (l, name) in levels.iter().rev() {
        let ch = channel::macro_channel(*l);
        let has = param.is_some() && doc.macro_pages(ch).iter().any(|p| p.macros.iter().any(|m| m.target == param));
        if ui.button(format!("{} {name} Macro", if has { "Remove" } else { "Create" })).clicked() {
            acts.push(Act::Macro(MacroAct::Plugin { ch, chain, slot, id }));
            ui.close();
        }
    }
}

/// A macro's target menu: the parameters of the channels it reaches, then their
/// plug-ins'. The pick points macro `slot` there.
pub(super) fn macro_target_menu(
    ui: &mut Ui,
    app: &App,
    level: Level,
    (ch, page, slot): crate::remote::MacroSlot,
    targets: &[channel::MacroTarget],
    acts: &mut Vec<Act>,
) {
    let plugins = app.plugin_targets(level);
    let extra: Vec<channel::MacroTarget> = plugins
        .iter()
        .map(|(path, _)| channel::MacroTarget { path: path.clone(), knob: (Cow::Borrowed(""), String::new(), 0.0, None), t: ch, field: "" })
        .collect();
    let refs: Vec<&channel::MacroTarget> = targets.iter().chain(&extra).collect();
    let Some(k) = target_picker(ui, &refs, 0) else { return };
    acts.push(Act::Macro(match k.checked_sub(targets.len()) {
        None => MacroAct::Assign { ch, page, slot, t: targets[k].t, field: targets[k].field },
        Some(j) => {
            let (chain, plugin_slot, id) = plugins[j].1;
            MacroAct::AssignPlugin { ch, page, slot, chain, plugin_slot, id }
        }
    }));
}

/// Menus of the parameters a macro can control, by where they are; returns the pick
/// (an index into the targets the list was made from).
fn target_picker(ui: &mut Ui, targets: &[&channel::MacroTarget], depth: usize) -> Option<usize> {
    let mut picked = None;
    let mut heads: Vec<&str> = Vec::new();
    for t in targets {
        if let Some(h) = t.path.get(depth)
            && !heads.contains(&h.as_str())
        {
            heads.push(h);
        }
    }
    for h in heads {
        let under: Vec<&channel::MacroTarget> = targets.iter().copied().filter(|t| t.path.get(depth).is_some_and(|x| x == h)).collect();
        if under.len() == 1 && under[0].path.len() == depth + 1 {
            if ui.button(h).clicked() {
                picked = Some(under[0] as *const channel::MacroTarget);
                ui.close();
            }
        } else {
            ui.menu_button(h, |ui| {
                if let Some(k) = target_picker(ui, &under, depth + 1) {
                    picked = Some(under[k] as *const channel::MacroTarget);
                }
            });
        }
    }
    picked.and_then(|p| targets.iter().position(|t| std::ptr::eq(*t, p)))
}

/// The Channel icon: a small knob.
fn channel_icon(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    p.circle_stroke(m, 7.0, Stroke::new(1.5, c));
    p.line_segment([m, m + Vec2::angled(-2.2) * 6.0], Stroke::new(1.5, c));
}

/// The Plug-in icon: a plug.
fn plug_icon(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    p.rect_filled(Rect::from_center_size(m + Vec2::new(1.0, 0.0), Vec2::new(9.0, 10.0)), 2.0, c);
    p.line_segment([m + Vec2::new(-7.0, 0.0), m + Vec2::new(-3.0, 0.0)], Stroke::new(1.5, c));
    for dy in [-2.5, 2.5] {
        p.line_segment([m + Vec2::new(5.0, dy), m + Vec2::new(8.5, dy)], Stroke::new(1.5, c));
    }
}

fn input_icon(p: &Painter, m: Pos2, c: Color32) {
    p.circle_stroke(m, 5.0, Stroke::new(1.2, c));
    p.line_segment([m + Vec2::new(-8.0, 0.0), m + Vec2::new(1.0, 0.0)], Stroke::new(1.2, c));
    p.line_segment([m + Vec2::new(-2.0, -3.0), m + Vec2::new(1.0, 0.0)], Stroke::new(1.2, c));
    p.line_segment([m + Vec2::new(-2.0, 3.0), m + Vec2::new(1.0, 0.0)], Stroke::new(1.2, c));
}

fn output_icon(p: &Painter, m: Pos2, c: Color32) {
    p.circle_stroke(m, 5.0, Stroke::new(1.2, c));
    p.line_segment([m + Vec2::new(-1.0, 0.0), m + Vec2::new(8.0, 0.0)], Stroke::new(1.2, c));
    p.line_segment([m + Vec2::new(5.0, -3.0), m + Vec2::new(8.0, 0.0)], Stroke::new(1.2, c));
    p.line_segment([m + Vec2::new(5.0, 3.0), m + Vec2::new(8.0, 0.0)], Stroke::new(1.2, c));
}

fn groove_icon(p: &Painter, m: Pos2, c: Color32) {
    p.line_segment([m + Vec2::new(-5.0, 0.0), m + Vec2::new(5.0, 0.0)], Stroke::new(1.2, c));
    p.line_segment([m + Vec2::new(0.0, -5.0), m + Vec2::new(0.0, 5.0)], Stroke::new(1.2, c));
}

fn macro_icon(p: &Painter, m: Pos2, c: Color32) {
    p.circle_stroke(m, 5.0, Stroke::new(1.2, c));
    p.line_segment([m, m + Vec2::angled(-0.8) * 4.5], Stroke::new(1.2, c));
}

/// The Channel Property selector: Input, Output (top row), Groove, Macro (bottom row).
#[allow(clippy::too_many_arguments)]
fn property_tiles(ui: &mut Ui, app: &App, painter: &Painter, area: Rect, level: Option<Level>, _color: Color32, acts: &mut Vec<Act>) {
    let icons: [fn(&Painter, Pos2, Color32); 4] = [input_icon, output_icon, groove_icon, macro_icon];
    let (w, h) = (75.0, 35.0);
    for (i, name) in channel::SETS.iter().enumerate() {
        let r = Rect::from_min_size(
            Pos2::new(area.left() + 4.0 + (i % 2) as f32 * (w + 1.0), area.top() + 6.0 + (i / 2) as f32 * (h + 7.0)),
            Vec2::new(w, h),
        );
        // The master has no input.
        let enabled = level.is_some() && !(i == 0 && level == Some(Level::Master));
        let resp = ui.interact(r, ui.id().with(("prop-set", i)), Sense::click());
        let on = app.prop_set == i && enabled;
        if on {
            painter.rect_filled(r, 3.0, S03);
        } else if resp.hovered() && enabled {
            painter.rect_filled(r, 3.0, S02);
        }
        let ink = if !enabled {
            white(50)
        } else if on {
            white(245)
        } else if resp.hovered() {
            white(200)
        } else {
            white(164)
        };
        icons[i](painter, Pos2::new(r.left() + 14.0, r.center().y), ink);
        painter.text(Pos2::new(r.left() + 27.0, r.center().y), Align2::LEFT_CENTER, *name, font(12.0), ink);
        if resp.clicked() && enabled {
            acts.push(Act::PropSet(i));
        }
    }
}

/// A choice parameter as Maschine shows it: its name, then the value between an up
/// arrow (previous choice) and a down arrow (next choice). A click on the value lists
/// the choices; dragging up or down steps through them too.
#[allow(clippy::too_many_arguments)]
fn selector(
    ui: &Ui,
    painter: &Painter,
    r: Rect,
    (i, name, value): (usize, &str, &str),
    options: &[&str],
    cur: usize,
    set: &dyn Fn(usize) -> Act,
    color: Color32,
    acts: &mut Vec<Act>,
) {
    painter.text(Pos2::new(r.center().x, r.top() + 8.0), Align2::CENTER_CENTER, name, font(10.0), white(142));
    let up = Rect::from_center_size(Pos2::new(r.center().x, r.top() + 27.0), Vec2::new(40.0, 14.0));
    let down = Rect::from_center_size(Pos2::new(r.center().x, r.top() + 57.0), Vec2::new(40.0, 14.0));
    let mid = Rect::from_center_size(Pos2::new(r.center().x, r.top() + 42.0), Vec2::new(70.0, 16.0));
    let arrow = |rect: Rect, pointing_up: bool, enabled: bool, id: &str| {
        let resp = ui.interact(rect, ui.id().with((id, i)), Sense::click());
        let c = if !enabled {
            white(30)
        } else if resp.hovered() {
            Color32::WHITE
        } else {
            color
        };
        let m = rect.center();
        let pts = if pointing_up {
            vec![m + Vec2::new(-5.0, 3.0), m + Vec2::new(5.0, 3.0), m + Vec2::new(0.0, -3.0)]
        } else {
            vec![m + Vec2::new(-5.0, -3.0), m + Vec2::new(5.0, -3.0), m + Vec2::new(0.0, 3.0)]
        };
        painter.add(egui::Shape::convex_polygon(pts, c, Stroke::NONE));
        enabled && resp.clicked()
    };
    let n = options.len();
    // (Not in the list: `cur` is past the end; the arrows go to the first entry.)
    let known = cur < n;
    if arrow(up, true, known && cur > 0, "choice-up") {
        acts.push(set(cur - 1));
    }
    if arrow(down, false, cur.wrapping_add(1) < n || !known, "choice-down") {
        acts.push(set(if known { cur + 1 } else { 0 }));
    }
    let resp = ui
        .interact(mid, ui.id().with(("choice-value", i)), Sense::click_and_drag())
        .on_hover_text("Click for the list, or drag up and down");
    if resp.hovered() || resp.dragged() {
        painter.rect_filled(mid, 2.0, white(10));
    }
    painter.text(mid.center(), Align2::CENTER_CENTER, value, font(12.0), white(245));
    if let Some(to) = choice_list(&resp, options.iter().copied(), cur)
        && to != cur
    {
        acts.push(set(to));
    }
    // Drag: one choice per 16 points.
    let id = ui.id().with(("choice-drag", i));
    if resp.drag_started() {
        ui.data_mut(|d| d.insert_temp(id, 0f32));
    }
    if resp.dragged() {
        let moved = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(0.0) - resp.drag_delta().y;
        ui.data_mut(|d| d.insert_temp(id, moved));
        let steps = (moved / 16.0).trunc() as i64;
        if steps != 0 {
            ui.data_mut(|d| d.insert_temp(id, moved - steps as f32 * 16.0));
            // Up the screen is the previous choice, as the up arrow.
            let from = if known { cur as i64 } else { 0 };
            let to = (from - steps).clamp(0, n as i64 - 1);
            if to != cur as i64 {
                acts.push(set(to as usize));
            }
        }
    }
}

/// A click on `resp` opens the list of `options` under it (the current one marked);
/// returns the one picked.
pub fn choice_list<'a>(resp: &egui::Response, options: impl IntoIterator<Item = &'a str>, cur: usize) -> Option<usize> {
    let mut picked = None;
    egui::Popup::menu(resp).show(|ui| {
        ui.set_min_width(resp.rect.width().max(110.0));
        for (k, o) in options.into_iter().enumerate() {
            let text = egui::RichText::new(o).size(12.0);
            let text = if k == cur { text.color(Color32::WHITE) } else { text };
            if ui.selectable_label(k == cur, text).clicked() {
                picked = Some(k);
            }
        }
    });
    picked
}

/// Page tabs to show: names, the page shown, and the action a click asks for.
type PageTabs = (Vec<String>, usize, fn(usize) -> Act);

/// A plug-in parameter's knob label: upper case, cut to fit.
fn knob_label(name: &str) -> String {
    let t: String = name.chars().take(12).collect();
    t.to_uppercase()
}

/// Page tabs from the left of `area` (the header, after its heading); those that
/// don't fit are left out.
fn page_tabs(
    ui: &Ui,
    painter: &Painter,
    area: Rect,
    names: &[String],
    page: usize,
    act: fn(usize) -> Act,
    acts: &mut Vec<Act>,
) -> Vec<Response> {
    let mut out = Vec::new();
    let mut x = area.left();
    for (i, name) in names.iter().enumerate() {
        let w = 14.0 + painter.layout_no_wrap(name.clone(), font(11.0), Color32::WHITE).size().x;
        if x + w > area.right() {
            break;
        }
        let r = Rect::from_min_max(Pos2::new(x, area.top() + 4.0), Pos2::new(x + w, area.bottom() - 4.0));
        let resp = ui.interact(r, ui.id().with(("page-tab", i)), Sense::click());
        let on = i == page;
        let ink = if on || resp.hovered() { Color32::WHITE } else { white(142) };
        painter.text(r.center(), Align2::CENTER_CENTER, name, font(11.0), ink);
        if on {
            painter.hline(r.shrink2(Vec2::new(4.0, 0.0)).x_range(), r.bottom(), Stroke::new(2.0, Color32::WHITE));
        }
        if resp.clicked() {
            acts.push(act(i));
        }
        out.push(resp);
        x += w + 2.0;
    }
    out
}

/// The sound's chain: one row per slot (click: its pages; right-click: bypass, remove,
/// plug-in window), then "+" to add an internal module or a plug-in.
fn chain_slots(ui: &mut Ui, app: &App, painter: &Painter, area: Rect, slots: &[deckard_model::SlotInfo], acts: &mut Vec<Act>) {
    const ROW: f32 = 21.0;
    let off = scroll(ui, "chain-slots", area, (slots.len() + 1) as f32 * ROW + 4.0);
    let p = painter.with_clip_rect(area);
    let (mut rows, mut icons) = (Vec::new(), Vec::new());
    for i in 0..=slots.len() {
        let r = Rect::from_min_size(Pos2::new(area.left() + 6.0, area.top() + i as f32 * ROW - off), Vec2::new(COLUMN - 12.0, ROW - 2.0));
        let vis = r.intersect(area);
        if vis.height() <= 0.0 {
            continue;
        }
        let resp = ui.interact(vis, ui.id().with(("chain-slot", i)), Sense::click_and_drag());
        rows.push(r);
        // An instrument stays first: effects move among themselves, after it.
        let first_free = usize::from(slots.first().is_some_and(|x| x.is_instrument()));
        match slots.get(i) {
            Some(slot) if !slot.is_instrument() && (resp.dragged() || resp.drag_stopped()) => {
                // Drag an effect to move it: a line shows where it goes.
                if let Some(pos) = ui.ctx().pointer_latest_pos() {
                    let to = (((pos.y - area.top() + off) / ROW).floor().max(0.0) as usize).clamp(first_free, slots.len() - 1);
                    if resp.drag_stopped() {
                        if to != i {
                            acts.push(Act::MoveSlot(i, to));
                        }
                    } else {
                        let y = area.top() + (if to > i { to + 1 } else { to }) as f32 * ROW - off - 1.0;
                        ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.id().with("slot-drop"))).hline(
                            r.x_range(),
                            y,
                            Stroke::new(2.0, Color32::WHITE),
                        );
                    }
                }
                p.rect_filled(r, 2.0, S03);
                slot_icon(&p, icon_rect(r).center(), slot.is_instrument(), white(245));
                p.text(Pos2::new(r.left() + NAME_AT, r.center().y), Align2::LEFT_CENTER, &slot.name, font(12.0), white(245));
            }
            Some(slot) => {
                let on = app.slot == Some(i);
                p.rect_filled(
                    r,
                    2.0,
                    if on {
                        S03
                    } else if resp.hovered() {
                        S02
                    } else {
                        S01
                    },
                );
                let ink = if !slot.active {
                    white(90)
                } else if on {
                    white(245)
                } else {
                    white(200)
                };
                p.text(Pos2::new(r.left() + NAME_AT, r.center().y), Align2::LEFT_CENTER, &slot.name, font(12.0), ink);
                // The instrument or effect icon bypasses the slot (BUGS #24), dim while bypassed.
                let ir = icon_rect(r);
                let icon = ui.interact(ir.intersect(area), ui.id().with(("chain-slot-icon", i)), Sense::click());
                let glyph = if !slot.active {
                    white(70)
                } else if icon.hovered() {
                    white(255)
                } else {
                    ink
                };
                slot_icon(&p, ir.center(), slot.is_instrument(), glyph);
                icons.push((ir, slot.is_instrument()));
                if icon.on_hover_text(if slot.active { "Bypass" } else { "Enable" }).clicked() {
                    acts.push(Act::SlotActive(i, !slot.active));
                }
                // On/off dot.
                p.circle_filled(Pos2::new(r.right() - 10.0, r.center().y), 3.0, if slot.active { white(200) } else { white(50) });
                if resp.clicked() {
                    acts.push(Act::SelectSlot(Some(i)));
                }
                let plugin = slot.plugin.is_some();
                let active = slot.active;
                let movable = !slot.is_instrument();
                let resp = if movable { resp.on_hover_text("Drag to move this effect") } else { resp };
                resp.context_menu(|ui| {
                    if plugin && ui.button("Open Plug-in Window").clicked() {
                        acts.push(Act::OpenSlotEditor(i));
                        ui.close();
                    }
                    if ui.add_enabled(movable && i > first_free, egui::Button::new("Move Up")).clicked() {
                        acts.push(Act::MoveSlot(i, i - 1));
                        ui.close();
                    }
                    if ui.add_enabled(movable && i + 1 < slots.len(), egui::Button::new("Move Down")).clicked() {
                        acts.push(Act::MoveSlot(i, i + 1));
                        ui.close();
                    }
                    if ui.button(if active { "Bypass" } else { "Enable" }).clicked() {
                        acts.push(Act::SlotActive(i, !active));
                        ui.close();
                    }
                    if ui.button("Remove").clicked() {
                        acts.push(Act::RemoveSlot(i));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Save Preset...").clicked() {
                        acts.push(Act::SaveItem(deckard_doc::FileItem::Slot(app.group, app.sound, i), crate::SaveMode::SaveAs));
                        ui.close();
                    }
                    if ui.button("Load Preset...").clicked() {
                        acts.push(Act::OpenItem(deckard_doc::FileItem::Slot(app.group, app.sound, i)));
                        ui.close();
                    }
                });
            }
            None => {
                p.rect_stroke(r, 2.0, Stroke::new(1.0, if resp.hovered() { white(120) } else { white(40) }), StrokeKind::Inside);
                p.text(r.center(), Align2::CENTER_CENTER, "+", font(15.0), if resp.hovered() { white(245) } else { white(142) });
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Add plug-in"));
                let resp = resp.on_hover_text("Add an instrument, effect or plug-in");
                // A sound has one instrument, in its first slot.
                let instruments = slots.is_empty();
                egui::Popup::menu(&resp).show(|ui| add_menu(ui, app, instruments, acts));
            }
        }
    }
    ui.data_mut(|d| {
        d.insert_temp(egui::Id::new("deckard-chain-rows"), rows);
        d.insert_temp(egui::Id::new("deckard-chain-icons"), icons);
    });
}

/// Where a slot row's name starts: after its icon.
const NAME_AT: f32 = 24.0;

/// A slot row's icon, at its left.
fn icon_rect(r: Rect) -> Rect {
    Rect::from_min_size(r.left_top() + Vec2::new(2.0, 0.0), Vec2::new(18.0, r.height()))
}

/// A slot's icon: a keyboard for an instrument, faders for an effect.
fn slot_icon(p: &Painter, m: Pos2, instrument: bool, c: Color32) {
    if instrument {
        let k = Rect::from_center_size(m, Vec2::new(12.0, 9.0));
        p.rect_stroke(k, 1.0, Stroke::new(1.0, c), StrokeKind::Inside);
        for i in 1..3 {
            p.vline(k.left() + i as f32 * 4.0, k.y_range(), Stroke::new(1.0, c));
        }
        for x in [3.0, 7.0] {
            p.rect_filled(Rect::from_min_size(Pos2::new(k.left() + x - 0.5, k.top()), Vec2::new(2.5, 5.0)), 0.0, c);
        }
    } else {
        for (i, y) in [2.0, -2.0, 1.0].into_iter().enumerate() {
            let x = m.x + (i as f32 - 1.0) * 4.0;
            p.vline(x, (m.y - 5.0)..=(m.y + 5.0), Stroke::new(1.0, c));
            p.rect_filled(Rect::from_center_size(Pos2::new(x, m.y + y), Vec2::new(3.0, 2.0)), 0.0, c);
        }
    }
}

/// A DECKARD plug-in as the "+" menu adds plug-ins.
pub(crate) fn native_entry(c: &deckard_plugins::native::Class) -> deckard_plugins::scan::Entry {
    deckard_plugins::scan::Entry {
        kind: deckard_plugins::scan::Kind::Vst3,
        vendor: deckard_plugins::native::VENDOR.into(),
        name: c.name.into(),
        instrument: c.instrument,
        location: String::new(),
        cid: c.cid.iter().map(|b| format!("{b:02X}")).collect(),
    }
}

const AFTER_FX: &str = "A sound has one instrument, in its first slot";

/// DECKARD's own plug-ins, instruments or effects.
fn native(instrument: bool) -> impl Iterator<Item = &'static deckard_plugins::native::Class> {
    deckard_plugins::native::CLASSES.iter().copied().filter(move |c| c.instrument == instrument)
}

/// "+" › Internal's instruments (a sound's first slot only): the Sampler and Audio,
/// Maschine's synths, DECKARD's own.
fn internal_instruments(ui: &mut Ui, instruments: bool, acts: &mut Vec<Act>) {
    for (name, what) in [("Sampler", AddWhat::Sampler), ("Audio", AddWhat::Audio)] {
        if ui.add_enabled(instruments, egui::Button::new(name)).on_disabled_hover_text(AFTER_FX).clicked() {
            acts.push(Act::AddSlot(what));
            ui.close();
        }
    }
    // Maschine's synths.
    ui.separator();
    for class in deckard_model::INTERNAL_INSTRUMENTS {
        let b = egui::Button::new(deckard_model::module_label(class));
        if ui.add_enabled(instruments, b).on_disabled_hover_text(AFTER_FX).clicked() {
            acts.push(Act::AddSlot(AddWhat::Instrument(class)));
            ui.close();
        }
    }
    // DECKARD's own.
    for c in native(true) {
        if ui.add_enabled(instruments, egui::Button::new(c.name)).on_disabled_hover_text(AFTER_FX).clicked() {
            acts.push(Act::AddSlot(AddWhat::Plugin(native_entry(c))));
            ui.close();
        }
    }
}

/// The "+" menu: Maschine's internal modules, then plug-ins by role and vendor.
fn add_menu(ui: &mut Ui, app: &App, instruments: bool, acts: &mut Vec<Act>) {
    ui.set_min_width(170.0);
    // A group's and the master's chains take effects only.
    let sound = matches!(app.chain(), deckard_doc::Chain::Sound(..));
    ui.menu_button("Internal", |ui| {
        if sound {
            internal_instruments(ui, instruments, acts);
            ui.separator();
        }
        for (class, _) in deckard_model::INTERNAL_EFFECTS {
            if ui.button(deckard_model::module_label(class)).clicked() {
                acts.push(Act::AddSlot(AddWhat::Effect(class)));
                ui.close();
            }
            if deckard_model::EFFECT_GROUP_ENDS.contains(class) {
                ui.separator();
            }
        }
        // DECKARD's own effects.
        for c in native(false) {
            if ui.button(c.name).clicked() {
                acts.push(Act::AddSlot(AddWhat::Plugin(native_entry(c))));
                ui.close();
            }
        }
        // A group's Perform FX: one, first in its chain (Maschine's last category).
        if let deckard_doc::Chain::Group(g) = app.chain() {
            ui.separator();
            let has = app.perform_slot(g).is_some();
            ui.add_enabled_ui(!has, |ui| {
                ui.menu_button("Perform FX", |ui| {
                    for (mode, name) in crate::module_ui::PERFORM_MODES.iter().enumerate() {
                        if ui.button(*name).clicked() {
                            acts.push(Act::AddPerformFx(g, mode));
                            ui.close();
                        }
                    }
                })
                .response
                .on_disabled_hover_text("A group has one Perform FX");
            });
        }
    });
    // Plug-ins by format, then vendor, as in Maschine.
    use deckard_plugins::scan::Kind;
    for (title, kind) in [("Audio Units", Kind::AudioUnit), ("VST3", Kind::Vst3), ("VST", Kind::Vst2)] {
        let mut entries: Vec<&deckard_plugins::scan::Entry> =
            app.catalog.entries.iter().filter(|e| e.kind == kind && app.settings.allows(e)).collect();
        if entries.is_empty() {
            continue;
        }
        entries.sort_by_key(|e| (e.vendor.to_lowercase(), e.name.to_lowercase()));
        ui.menu_button(title, |ui| {
            let vendor = |e: &deckard_plugins::scan::Entry| if e.vendor.is_empty() { "Other".to_string() } else { e.vendor.clone() };
            let mut vendors: Vec<String> = entries.iter().map(|e| vendor(e)).collect();
            vendors.dedup();
            for v in vendors {
                ui.menu_button(&v, |ui| {
                    for e in entries.iter().filter(|e| vendor(e) == v) {
                        let text = if e.instrument { e.name.clone() } else { format!("{}  (effect)", e.name) };
                        if e.instrument && !sound {
                            continue;
                        }
                        let allowed = instruments || !e.instrument;
                        let b = ui.add_enabled(e.hostable() && allowed, egui::Button::new(text));
                        let b = if !e.hostable() {
                            b.on_disabled_hover_text(format!("{} plug-ins can't be hosted yet", e.kind.label()))
                        } else if !allowed {
                            b.on_disabled_hover_text(AFTER_FX)
                        } else {
                            b
                        };
                        if b.clicked() {
                            acts.push(Act::AddSlot(AddWhat::Plugin((*e).clone())));
                            ui.close();
                        }
                    }
                });
            }
        });
    }
    if app.catalog.entries.is_empty() {
        ui.label(egui::RichText::new(if app.catalog.scanning() { "Scanning plug-ins…" } else { "No plug-ins found" }).weak());
    }
    ui.separator();
    if ui.button("Manage Plug-ins…").clicked() {
        acts.push(Act::Preferences);
        ui.close();
    }
}

// ---------------------------------------------------------------------------------------
// Group panel (GroupPanel.txt, PatternEditor.txt): sound list, pattern editor, velocity.

fn group_panel(ui: &mut Ui, app: &App, p: &Project, rect: Rect, head: Option<Playhead>, acts: &mut Vec<Act>) {
    let strip_rect = Rect::from_min_size(rect.min, Vec2::new(STRIP, rect.height()));
    match strip(
        ui,
        strip_rect,
        S00,
        7.0,
        &[
            (!app.piano_roll, "Drum grid", grid_icon),
            (app.piano_roll, "Piano roll (focused sound)", piano_icon),
            (app.draw_mode(), "Draw mode: click adds or removes notes, drag paints notes and lane values. Off: select mode.", pencil_icon),
            (
                app.mouse == MouseMode::Erase,
                "Erase mode: click a note to delete it; click or drag in a lane to erase its values there. Off: select mode.",
                eraser_icon,
            ),
            (app.sampling_open, "Sampling: record, edit, slice and map the sound's samples", wave_icon),
        ],
    ) {
        Some(0) => acts.push(Act::PianoRoll(false)),
        Some(1) => acts.push(Act::PianoRoll(true)),
        Some(2) => acts.push(Act::DrawMode(!app.draw_mode())),
        Some(3) => acts.push(Act::SetMouse(app.mouse.toggled(MouseMode::Erase))),
        Some(4) => acts.push(Act::Sampling(sampling::SamplingAct::Open(!app.sampling_open))),
        _ => {}
    }
    let col = Rect::from_min_size(Pos2::new(rect.left() + STRIP + 1.0, rect.top()), Vec2::new(COLUMN, rect.height()));
    let ed = Rect::from_min_max(Pos2::new(col.right() + 1.0, rect.top()), rect.max);
    let painter = ui.painter_at(rect);
    painter.rect_filled(col, 0.0, S00);
    painter.rect_filled(ed, 0.0, S00);
    let Some(group) = p.groups.get(app.group) else { return };
    let gcol = group_color(p, app.group);
    let pat = group.patterns.get(app.pattern);
    // The Sampling view replaces the pattern editor; the sound list stays.
    if app.sampling_open {
        label(&painter, Rect::from_min_size(col.min, Vec2::new(COLUMN, 32.0)), 8.0, &group_name(p, app.group), 15.0, gcol);
        let list = Rect::from_min_max(Pos2::new(col.left(), col.top() + 56.0), col.max);
        let lp = painter.with_clip_rect(list);
        let n = group.sounds.len().max(16);
        let y0 = scroll(ui, "sampling-sound-list", list, n as f32 * ROW);
        for r in 0..n {
            let rr = Rect::from_min_size(Pos2::new(col.left(), list.top() + r as f32 * ROW - y0), Vec2::new(COLUMN, ROW));
            if rr.bottom() < list.top() || rr.top() > list.bottom() {
                continue;
            }
            let sound = group.sounds.get(r);
            let has = sound.is_some_and(|s| !s.name.is_empty() || !s.instrument.is_empty());
            let name = sound.map_or("", |s| s.name.as_str());
            let color = has.then(|| sound_color(p, app.group, r));
            group_row(
                &lp,
                rr,
                &(r + 1).to_string(),
                name,
                color,
                r == app.sound,
                sound.is_some_and(|s| s.muted),
                app.lit.contains(&(app.group, r)),
                S00,
                app.meter(app.group, Some(r)),
            );
            if r < group.sounds.len() {
                row_input(ui, app, p, rr, list, Target::Sound(app.group, r), name, Act::FocusSound(r), acts);
            }
        }
        sampling::panel(ui, app, p, ed, acts);
        return;
    }

    // Titles: sound list 56 = editor header 38 + timeline 19 (−1).
    label(&painter, Rect::from_min_size(col.min, Vec2::new(COLUMN, 32.0)), 8.0, &group_name(p, app.group), 15.0, gcol);
    let header = Rect::from_min_size(ed.min, Vec2::new(ed.width(), 38.0));
    let bar = p.bar_length().max(1);
    match pat {
        Some(pt) => {
            let name = if pt.name.is_empty() { "Pattern" } else { &pt.name };
            let r = painter.text(Pos2::new(header.left() + 8.0, header.center().y), Align2::LEFT_CENTER, name, font(15.0), gcol);
            let mut x = r.right() + 24.0;
            let len = pattern_len(p, app.group, app.pattern);
            let beat = PPQ * 4 / p.denominator.max(1);
            let y = header.center().y;
            let r = painter.text(Pos2::new(x, y), Align2::LEFT_CENTER, "START", font(10.0), white(61));
            let r = painter.text(
                Pos2::new(r.right() + 6.0, y),
                Align2::LEFT_CENTER,
                position_string(p, pt.start as f64),
                font(12.0),
                Color32::WHITE,
            );
            x = r.right() + 20.0;
            // Length, grid and quantize: menus.
            let (g, pi) = (app.group, app.pattern);
            let r = painter.text(Pos2::new(x, y), Align2::LEFT_CENTER, "LENGTH", font(10.0), white(61));
            let at = Rect::from_min_size(Pos2::new(r.right() + 2.0, y - 10.0), Vec2::new(64.0, 20.0));
            // Auto (no length chosen): an empty pattern grows as it's recorded into.
            let auto = app.doc.as_ref().is_some_and(|d| d.borrow().length_is_auto(g, pi));
            let text = if auto && pt.notes.is_empty() {
                "Auto".to_string()
            } else {
                format!("{}:{}:{}", len / bar, len % bar / beat, len % beat / (PPQ / 4))
            };
            menu_at(ui, at, text, |ui| {
                if ui
                    .selectable_label(auto, "Auto")
                    .on_hover_text("Recorded into from empty, the pattern grows a bar at a time as it's played into")
                    .clicked()
                {
                    acts.push(Act::PatternLengthAuto(g, pi));
                    ui.close();
                }
                ui.separator();
                for bars in [1, 2, 4, 8, 16, 32] {
                    if ui.selectable_label(len == bars * bar, format!("{bars} bar{}", if bars > 1 { "s" } else { "" })).clicked() {
                        acts.push(Act::PatternLength(g, pi, bars * bar));
                        ui.close();
                    }
                }
                ui.separator();
                for (label, delta) in [("+1 bar", bar), ("−1 bar", -bar), ("+1 beat", beat), ("−1 beat", -beat)] {
                    if ui.add_enabled(len + delta >= beat, egui::Button::new(label)).clicked() {
                        acts.push(Act::PatternLength(g, pi, len + delta));
                    }
                }
            });
            x = at.right() + 16.0;
            let r = painter.text(Pos2::new(x, y), Align2::LEFT_CENTER, "GRID", font(10.0), white(61));
            let at = Rect::from_min_size(Pos2::new(r.right() + 2.0, y - 10.0), Vec2::new(52.0, 20.0));
            menu_at(ui, at, grid_name(app.grid), |ui| {
                for (name, ticks) in GRIDS {
                    if ui.selectable_label(app.grid == ticks, name).clicked() {
                        acts.push(Act::Grid(ticks));
                        ui.close();
                    }
                }
            });
            x = at.right() + 16.0;
            let at = Rect::from_min_size(Pos2::new(x, y - 10.0), Vec2::new(78.0, 20.0));
            let selected = app.selected.iter().filter(|n| pt.notes.contains(n)).count();
            let what = match selected {
                0 => format!("all {} notes", pt.notes.len()),
                1 => "the selected note".to_string(),
                n => format!("the {n} selected notes"),
            };
            // The scale (Maschine only sets it on the controller): root and scale, by bank.
            let scale_at = Rect::from_min_size(Pos2::new(at.right() + 16.0, y - 10.0), Vec2::new(150.0, 20.0));
            let (root, bank, kind) = app.scale;
            let chromatic = bank == 0 && kind == 0;
            let names = deckard_lua_host::api::scales::scale_names(bank);
            let shown = if chromatic {
                "SCALE".to_string()
            } else {
                format!("{} {}", NOTE_NAMES[root.rem_euclid(12) as usize], names[kind.clamp(0, 14) as usize])
            };
            let scale_drawn = menu_at(ui, scale_at, shown, |ui| {
                ui.label(egui::RichText::new("Keys outside the scale are marked in the piano roll").weak());
                ui.menu_button(format!("Root: {}", NOTE_NAMES[root.rem_euclid(12) as usize]), |ui| {
                    for (pc, name) in NOTE_NAMES.iter().enumerate() {
                        if ui.selectable_label(root.rem_euclid(12) == pc as i64, *name).clicked() {
                            acts.push(Act::Scale(root - root.rem_euclid(12) + pc as i64, bank, kind));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                for (b, bank_name) in deckard_lua_host::api::scales::banks().iter().enumerate() {
                    let b = b as i64;
                    let title = if b == bank { egui::RichText::new(*bank_name).strong() } else { egui::RichText::new(*bank_name) };
                    ui.menu_button(title, |ui| {
                        for (k, name) in deckard_lua_host::api::scales::scale_names(b).iter().enumerate() {
                            if ui.selectable_label(b == bank && k as i64 == kind, *name).clicked() {
                                acts.push(Act::Scale(root, b, k as i64));
                                ui.close();
                            }
                        }
                    });
                }
            });
            let pad_at = Rect::from_min_size(Pos2::new(scale_drawn.rect.right() + 16.0, y - 10.0), Vec2::new(130.0, 20.0));
            pad_touch_menu(ui, pad_at, app.group, app.sound, group.sounds.get(app.sound), acts);
            if selected > 0 {
                painter.text(
                    Pos2::new(pad_at.right() + 14.0, y),
                    Align2::LEFT_CENTER,
                    format!("{selected} selected  (Esc clears)"),
                    font(11.0),
                    Color32::WHITE,
                );
            }
            menu_at(ui, at, "QUANTIZE".into(), |ui| {
                ui.label(egui::RichText::new(format!("Snap {what} to the {} grid", grid_name(app.grid))).weak());
                for pct in [100, 75, 50] {
                    if ui.button(format!("{pct} %")).clicked() {
                        acts.push(Act::Quantize(pct));
                        ui.close();
                    }
                }
            });
        }
        None => label(&painter, header, 8.0, "No pattern", 12.0, white(142)),
    }
    let timeline = Rect::from_min_size(Pos2::new(ed.left(), header.bottom()), Vec2::new(ed.width(), 19.0));
    painter.rect_filled(timeline, 0.0, Color32::BLACK);
    // One lane at the bottom, showing what the list at its left picks: velocity,
    // probability or a MIDI controller, as Maschine 3 does.
    let lane = Rect::from_min_max(
        Pos2::new(ed.left(), rect.bottom() - ZOOM_BAR - LANE_H),
        Pos2::new(ed.right() - ZOOM_BAR, rect.bottom() - ZOOM_BAR - 1.0),
    );
    // In the piano roll a keyboard sits at the left of the note area; the sound list
    // stays where it is.
    let keys_w = if app.piano_roll { KEYS_W } else { 0.0 };
    let grid = Rect::from_min_max(Pos2::new(ed.left() + keys_w, timeline.bottom()), Pos2::new(ed.right() - ZOOM_BAR, lane.top() - 7.0));
    let keys = Rect::from_min_max(Pos2::new(ed.left(), grid.top()), Pos2::new(grid.left(), grid.bottom()));
    let list = Rect::from_min_max(Pos2::new(col.left(), grid.top()), Pos2::new(col.right(), grid.bottom()));
    painter.hline(rect.x_range(), lane.top() - 3.5, Stroke::new(1.0, Color32::BLACK));
    // Where the lane is, for tests that drive the window.
    ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-lane"), lane));
    ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-grid"), grid));

    // Time: the pattern's own, from its tick 0 to two bars past the next whole bar after
    // its end (room to drag the end into); the pattern plays from `start` for `plen`
    // (the span marked on the timeline).
    let (plen, start) = pat.map_or((bar, 0), |pt| (pattern_len(p, app.group, app.pattern), pt.start));
    let end = start + plen;
    let view = (((end + bar - 1) / bar).max(1) + 2) * bar;
    // What of it is shown (horizontal zoom): from `shown_from` for `shown` ticks, kept as
    // fractions of the view so it survives the pattern's length changing.
    let width = grid.width() - 8.0;
    let hkey = egui::Id::new(("pattern-hzoom", app.group, app.pattern));
    let (mut from_f, mut len_f) = ui.data(|d| d.get_temp::<(f32, f32)>(hkey)).unwrap_or((0.0, 1.0));
    let min_shown = (PPQ as f32 / view as f32).min(1.0);
    let over_time = ui.rect_contains_pointer(grid) || ui.rect_contains_pointer(timeline) || ui.rect_contains_pointer(lane);
    if over_time && let Some(px) = ui.input(|i| i.pointer.hover_pos()) {
        // ⌘/Ctrl + scroll (and pinch) zooms the time axis around the pointer; a sideways
        // scroll moves along it.
        let (zoom, dx) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta.x));
        let at = ((px.x - grid.left()) / width).clamp(0.0, 1.0);
        if zoom != 1.0 {
            let anchor = from_f + at * len_f;
            len_f = (len_f / zoom).clamp(min_shown, 1.0);
            from_f = anchor - at * len_f;
        }
        from_f -= dx / width * len_f;
        from_f = from_f.clamp(0.0, 1.0 - len_f);
    }
    let hbar = Rect::from_min_max(Pos2::new(grid.left(), rect.bottom() - ZOOM_BAR), Pos2::new(grid.right(), rect.bottom()));
    if let Some((f, l)) = zoom_bar(ui, &painter, hkey.with("bar"), hbar, true, 1.0, (from_f, len_f), min_shown, true) {
        (from_f, len_f) = (f, l);
    }
    // Follow playback: zoomed in, the view turns the page when the playhead leaves it.
    if app.follow
        && len_f < 1.0
        && let Some(t) = pat.and_then(|_| pattern_position(p, head, app.group, app.pattern))
    {
        let at = ((start as f64 + t) / view as f64) as f32;
        if at < from_f || at >= from_f + len_f {
            from_f = ((at / len_f).floor() * len_f).clamp(0.0, 1.0 - len_f);
        }
    }
    ui.data_mut(|d| d.insert_temp(hkey, (from_f, len_f)));
    let shown_from = from_f as f64 * view as f64;
    let scale = width / (len_f * view as f32);
    // (Where the time axis is, for tests that drive the window.)
    ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-grid-time"), (shown_from, scale)));
    let x_of = |t: f64| grid.left() + ((t - shown_from) as f32) * scale;
    // Ticks under an x (the inverse of `x_of`).
    let tick_at = |x: f32| ((x - grid.left()) / scale) as f64 + shown_from;
    let tl = Rect::from_min_max(Pos2::new(timeline.left(), timeline.top() + 1.0), timeline.max);
    painter.rect_filled(tl, 0.0, S00);
    let painter_all = painter.clone();
    let painter = painter.with_clip_rect(Rect::from_min_max(Pos2::new(grid.left(), rect.top()), Pos2::new(grid.right(), rect.bottom())));
    painter.rect_filled(Rect::from_min_max(Pos2::new(x_of(start as f64), tl.top()), Pos2::new(x_of(end as f64), tl.bottom())), 0.0, S01);
    let beat = PPQ * 4 / p.denominator.max(1);
    let step = app.grid.max(1);
    let bars_step = ((48.0 / (bar as f32 * scale)).ceil() as i64).max(1);
    let mut t = 0;
    while t <= view {
        let inside = (start..=end).contains(&t);
        let x = x_of(t as f64);
        if t % bar == 0 {
            painter.vline(x, tl.y_range(), Stroke::new(1.0, white(if inside { 92 } else { 40 })));
            if (t / bar) % bars_step == 0 && t < view {
                painter.text(
                    Pos2::new(x + 6.0, tl.center().y - 1.0),
                    Align2::LEFT_CENTER,
                    (t / bar + 1).to_string(),
                    font(12.0),
                    white(if inside { 142 } else { 61 }),
                );
            }
        } else if t % beat == 0 && beat as f32 * scale > 12.0 {
            painter.vline(
                x,
                Rect::from_min_max(Pos2::new(0.0, tl.top() + 10.0), tl.max).y_range(),
                Stroke::new(1.0, white(if inside { 40 } else { 20 })),
            );
        }
        t += step;
    }
    if pat.is_some() {
        pattern_span_handles(ui, app, &painter, tl, start, plen, scale, x_of(start as f64), acts);
    }

    // Rows of the note area: 16 sound slots (24 each), or in the piano roll every
    // pitch of the focused sound, top to bottom (it opens centred on its notes).
    let notes: Vec<&Note> = pat.map_or(Vec::new(), |pt| pt.notes.iter().collect());
    let n_sounds = group.sounds.len().max(16);
    let (rows, lo, hi) = if app.piano_roll { (128, 0i64, 127i64) } else { (n_sounds, 0, 0) };
    let vbar = Rect::from_min_max(Pos2::new(grid.right(), grid.top()), Pos2::new(grid.right() + ZOOM_BAR, grid.bottom()));
    let (row_h, y0) = if app.piano_roll {
        // The pitches shown (vertical zoom): from row `first` for `count` rows; opens at
        // 12 px a row, centred on the sound's notes.
        let vkey = egui::Id::new(("piano-zoom", app.group, app.sound));
        let (mut first, mut count) = ui.data(|d| d.get_temp::<(f32, f32)>(vkey)).unwrap_or_else(|| {
            let mine: Vec<i64> = notes.iter().filter(|n| n.sound == app.sound).map(|n| n.pitch).collect();
            let centre = if mine.is_empty() { 60 } else { (mine.iter().min().unwrap() + mine.iter().max().unwrap()) / 2 };
            let count = (grid.height() / PIANO_ROW).min(128.0);
            (((hi - centre) as f32 - count / 2.0).clamp(0.0, 128.0 - count), count)
        });
        count = count.clamp(8.0_f32.min(128.0), 128.0);
        if ui.rect_contains_pointer(grid) || ui.rect_contains_pointer(keys) {
            first -= ui.input(|i| i.smooth_scroll_delta.y) / (grid.height() / count);
        }
        first = first.clamp(0.0, 128.0 - count);
        if let Some(w) = zoom_bar(ui, &painter_all, vkey.with("bar"), vbar, false, 128.0, (first, count), 8.0, true) {
            (first, count) = w;
        }
        ui.data_mut(|d| d.insert_temp(vkey, (first, count)));
        let row_h = grid.height() / count;
        (row_h, first * row_h)
    } else {
        // The sound slots: a plain scroll bar.
        let y0 = scroll(ui, "pattern-rows", grid, rows as f32 * ROW);
        let total = rows as f32 * ROW;
        let y0 = match zoom_bar(
            ui,
            &painter_all,
            ui.id().with("pattern-rows-bar"),
            vbar,
            false,
            total,
            (y0, grid.height().min(total)),
            0.0,
            false,
        ) {
            Some((y, _)) => {
                ui.data_mut(|d| d.insert_temp(ui.id().with("pattern-rows"), y));
                y
            }
            None => y0,
        };
        (ROW, y0)
    };
    let row_y = |r: usize| grid.top() + r as f32 * row_h - y0;
    let gp = painter.with_clip_rect(grid);
    let lp = painter_all.with_clip_rect(list);
    // The sound list: always there. In the drum grid it scrolls with the rows; in the
    // piano roll on its own, and clicking a sound shows its notes.
    let list_y0 = if app.piano_roll { scroll(ui, "sound-list", list, n_sounds as f32 * ROW) } else { y0 };
    let painter = painter_all;
    for r in 0..n_sounds {
        let y = list.top() + r as f32 * ROW - list_y0;
        if y + ROW < list.top() || y > list.bottom() {
            continue;
        }
        let rr = Rect::from_min_size(Pos2::new(col.left(), y), Vec2::new(COLUMN, ROW));
        let sound = group.sounds.get(r);
        let has = sound.is_some_and(|s| !s.name.is_empty() || !s.instrument.is_empty());
        let name = sound.map_or("", |s| s.name.as_str());
        let color = has.then(|| sound_color(p, app.group, r));
        group_row(
            &lp,
            rr,
            &(r + 1).to_string(),
            name,
            color,
            r == app.sound,
            sound.is_some_and(|s| s.muted),
            app.lit.contains(&(app.group, r)),
            S00,
            app.meter(app.group, Some(r)),
        );
        if r < group.sounds.len() {
            row_input(ui, app, p, rr, list, Target::Sound(app.group, r), name, Act::FocusSound(r), acts);
            // Pressing the sound's number plays its root note while held.
            let num = Rect::from_min_size(Pos2::new(rr.left() + 4.0, rr.top()), Vec2::new(28.0, rr.height())).intersect(list);
            if ui.input(|i| i.pointer.primary_pressed() && i.pointer.press_origin().is_some_and(|o| num.contains(o))) {
                acts.push(Act::HoldPitch(app.group, r, app.scale.0));
            }
        }
    }
    let kp = painter.with_clip_rect(keys);
    for r in 0..rows {
        let y = row_y(r);
        if y + row_h < grid.top() || y > grid.bottom() {
            continue;
        }
        let band = Rect::from_min_size(Pos2::new(grid.left(), y), Vec2::new(grid.width(), row_h));
        if app.piano_roll {
            let pitch = hi - r as i64;
            let black_key = matches!(pitch.rem_euclid(12), 1 | 3 | 6 | 8 | 10);
            gp.rect_filled(band, 0.0, if black_key { S00 } else { S05 });
            if pitch.rem_euclid(12) == 0 {
                gp.hline(grid.x_range(), band.bottom() - 0.5, Stroke::new(1.0, white(24)));
            }
            // The keyboard: white and black keys, C labelled; a click plays the pitch.
            let key = Rect::from_min_size(Pos2::new(keys.left(), y), Vec2::new(keys.width(), row_h));
            kp.rect_filled(key, 0.0, if black_key { S00 } else { Color32::from_gray(0x9B) });
            // Out of the scale: red over the key and its row.
            let (root, bank, kind) = app.scale;
            if !deckard_lua_host::api::scales::in_scale(bank, kind, root, pitch) {
                kp.rect_filled(key, 0.0, Color32::from_rgba_unmultiplied(220, 40, 40, 110));
                gp.rect_filled(band, 0.0, Color32::from_rgba_unmultiplied(220, 40, 40, 28));
            }
            kp.hline(key.x_range(), key.bottom() - 0.5, Stroke::new(1.0, S00));
            if pitch.rem_euclid(12) == 0 {
                label(&kp, key, 4.0, &note_name(pitch), 9.0, Color32::BLACK);
            }
            // Sounds on press, not release, as an instrument should.
            if pressed(ui, &ui.interact(key.intersect(keys), ui.id().with(("key", pitch)), Sense::click())) {
                acts.push(Act::HoldPitch(app.group, app.sound, pitch));
            }
        } else {
            if r == app.sound {
                gp.rect_filled(band, 0.0, white(12));
            }
            let line = if (r + 1) % 4 == 0 { white(24) } else { white(6) };
            gp.hline(grid.x_range(), y + row_h - 0.5, Stroke::new(1.0, line));
        }
    }
    // Grid lines, inside and outside the pattern.
    let mut t = 0;
    while t <= view {
        let inside = (start..=end).contains(&t);
        let (c_in, c_out) = if t % bar == 0 {
            (0x8A, 0x4B)
        } else if t % beat == 0 {
            (0x48, 0x2A)
        } else {
            (0x21, 0x1C)
        };
        if step as f32 * scale > 3.0 || t % beat == 0 {
            gp.vline(x_of(t as f64), grid.y_range(), Stroke::new(1.0, Color32::from_gray(if inside { c_in } else { c_out })));
        }
        t += step;
    }
    // Outside the span: dimmed (notes there don't play).
    if start > 0 {
        gp.rect_filled(Rect::from_min_max(grid.min, Pos2::new(x_of(start as f64), grid.bottom())), 0.0, black(80));
    }
    if end < view {
        gp.rect_filled(Rect::from_min_max(Pos2::new(x_of(end as f64), grid.top()), grid.max), 0.0, black(80));
    }
    // Audio plug-ins' loops on their sounds' rows (in the piano roll, the focused
    // sound's behind its notes): from the pattern's start, a loop's length in beats at a
    // time, repeats darker (as Maschine shows them), dim where it's off in the pattern.
    // Placed by beats, they stay on the steps whatever the tempo.
    for (si, snd) in group.sounds.iter().enumerate() {
        if app.piano_roll && si != app.sound {
            continue;
        }
        let Some(a) = snd.audio.as_ref().filter(|_| snd.instrument == "AudioModule") else { continue };
        let cue = pat.and_then(|pt| pt.audio.iter().find(|c| c.sound == si));
        let k = cue.map_or(a.focused, |c| c.take);
        let (Some(take), Some(peaks)) = (a.takes.get(k), app.loop_peaks.get(&(app.group, si, k)).map(|x| &x.2)) else { continue };
        let band = if app.piano_roll {
            grid
        } else {
            let y = row_y(si);
            if y + row_h < grid.top() || y > grid.bottom() {
                continue;
            }
            Rect::from_min_size(Pos2::new(grid.left(), y + 1.0), Vec2::new(grid.width(), row_h - 2.0))
        };
        let loop_ticks = take.beats * PPQ as f64;
        if loop_ticks <= 0.0 || peaks.is_empty() {
            continue;
        }
        let colour = sound_color(p, app.group, si);
        let alpha = |rep: usize| -> u8 {
            match (cue.is_some(), app.piano_roll, rep) {
                (false, ..) => 40,
                (true, true, _) => 70,
                (true, false, 0) => 230,
                (true, false, _) => 120,
            }
        };
        let (mid, half) = (band.center().y, band.height() * 0.46);
        let (x0, x1) = (x_of(start as f64).max(grid.left()), x_of(end as f64).min(grid.right()));
        let mut x = x0.floor();
        while x < x1 {
            let t = tick_at(x + 0.5) - start as f64;
            if t >= 0.0 {
                let rep = (t / loop_ticks).floor() as usize;
                let f = (t / loop_ticks).fract();
                let i = ((f * peaks.len() as f64) as usize).min(peaks.len() - 1);
                let (lo, hi) = peaks[i];
                let c = Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), alpha(rep));
                gp.vline(x + 0.5, (mid - hi * half)..=(mid - lo * half).max(mid - hi * half + 1.0), Stroke::new(1.0, c));
            }
            x += 1.0;
        }
    }
    // ---- Note editing ---------------------------------------------------------------
    // Select mode: click selects (Shift adds), drag moves the selection, dragging on
    // empty space box-selects, double-click adds, Delete removes. Draw mode: click
    // adds or removes; dragging paints notes along the rows. A drag in progress lives
    // in egui memory and shows as a preview until the button is released.
    let drag_id = ui.id().with("note-drag");
    let mut drag: Option<NoteDrag> = ui.data(|d| d.get_temp(drag_id));
    // An Expression edit left behind (its lane gone, the button up) isn't shown or kept.
    if !expr_lane::keep_drag(&drag, app.lane, pat.is_some(), ui.input(|i| i.pointer.primary_down() || i.pointer.primary_released())) {
        drag = None;
    }
    let row_of = |n: &Note| -> Option<usize> {
        if app.piano_roll {
            (n.sound == app.sound && n.pitch >= lo && n.pitch <= hi).then(|| (hi - n.pitch) as usize)
        } else {
            Some(n.sound)
        }
    };
    let (top, bottom) = if app.piano_roll { (1.0, 1.0) } else { (2.0, 3.0) };
    let note_rect = |n: &Note| -> Option<Rect> {
        let y = row_y(row_of(n)?);
        let pos = n.position as f64;
        Some(Rect::from_min_max(
            Pos2::new(x_of(pos), y + top),
            Pos2::new(x_of(pos + n.length as f64).max(x_of(pos) + 3.0), y + row_h - bottom),
        ))
    };
    // The grid; Shift snaps a quarter as fine.
    let snap = if ui.input(|i| i.modifiers.shift) { (app.grid / 4).max(1) } else { app.grid.max(1) };
    let snapped = |x: f32| (tick_at(x) as i64).div_euclid(snap) * snap;
    let row_at = |y: f32| ((y - grid.top() + y0) / row_h).floor() as i64;
    // A new note at a grid position: drum rows keep the pitch the sound already uses
    // here (C3 by default); piano rows are pitches of the focused sound.
    let note_at = |pos: Pos2| -> Option<Note> {
        let row = row_at(pos.y);
        let (sound, pitch) = if app.piano_roll {
            (app.sound, hi - row)
        } else {
            let s = usize::try_from(row).ok().filter(|r| *r < group.sounds.len())?;
            (s, notes.iter().find(|n| n.sound == s).map_or(60, |n| n.pitch))
        };
        let rel = snapped(pos.x);
        (0..view).contains(&rel).then_some(Note {
            expr: None,
            sound,
            position: rel,
            length: app.grid.max(1),
            pitch,
            velocity: 100,
            probability: 100,
        })
    };
    let hit = |pos: Pos2| {
        notes.iter().rev().find_map(|n| note_rect(n).filter(|r| r.expand(1.0).contains(pos)).map(|r| (**n, pos.x > r.right() - 6.0)))
    };
    // Where dragged notes would go.
    let moved = |d: &NoteDrag| -> Vec<(Note, Note)> {
        match d {
            NoteDrag::Move { notes: ns, ticks, rows } => {
                let ok_rows = |n: &Note| {
                    if app.piano_roll {
                        (0..=127).contains(&(n.pitch - rows))
                    } else {
                        (0..group.sounds.len() as i64).contains(&(n.sound as i64 + rows))
                    }
                };
                let rows = if ns.iter().all(ok_rows) { *rows } else { 0 };
                ns.iter()
                    .map(|n| {
                        let mut m = *n;
                        m.position = (n.position + ticks).clamp(0, view - 1);
                        if app.piano_roll {
                            m.pitch = n.pitch - rows;
                        } else {
                            m.sound = (n.sound as i64 + rows) as usize;
                        }
                        (*n, m)
                    })
                    .collect()
            }
            NoteDrag::Resize { notes: ns, anchor, length } => {
                let delta = length - anchor.length;
                ns.iter().map(|n| (*n, Note { length: (n.length + delta).max(snap.min(n.length)), ..*n })).collect()
            }
            _ => Vec::new(),
        }
    };
    let moving: Vec<(Note, Note)> = drag.as_ref().map_or(Vec::new(), moved);
    // Notes: alpha 80..255 by velocity; selected notes outlined.
    // Selected notes (and previews) are lit and framed: a white wash, a dark inner
    // line and a white outer one, as Maschine marks a selection.
    let marks = std::cell::RefCell::new(Vec::new());
    let draw = |n: &Note, outline: Option<Color32>| {
        if let Some(r) = note_rect(n) {
            let a = 80 + (n.velocity.clamp(0, 127) * 175 / 127) as u8;
            // Muted notes: Maschine's grey.
            let c = if deckard_doc::note_tools::muted(n) { Color32::from_rgb(128, 128, 128) } else { sound_color(p, app.group, n.sound) };
            gp.rect_filled(r, 1.0, with_alpha(c, a));
            if let Some(c) = outline {
                gp.rect_filled(r, 1.0, white(90));
                gp.rect_stroke(r, 1.0, Stroke::new(1.0, S00), StrokeKind::Inside);
                gp.rect_stroke(r, 1.0, Stroke::new(2.0, c), StrokeKind::Outside);
            }
            // Its pitch curve, in the piano roll: a line over it, bent by as many rows.
            if app.piano_roll
                && n.expr.is_some()
                && let Some(c) = expr_lane::shown(p, &drag, n).pitch
            {
                let steps = ((r.width() / 3.0).ceil() as usize).clamp(1, 2000);
                let line: Vec<Pos2> = (0..=steps)
                    .map(|k| {
                        let f = k as f32 / steps as f32;
                        let t = (f * n.length.max(1) as f32) as i64;
                        Pos2::new(r.left() + r.width() * f, r.center().y - c.value_at(t) * row_h)
                    })
                    .collect();
                gp.add(egui::Shape::line(line, Stroke::new(1.5, white(200))));
            }
            // Expression played on it (MPE): a dot in its top-left corner.
            if has_expression_mark(n) && r.width() >= 6.0 && r.height() >= 5.0 {
                let dot = Rect::from_min_size(r.min + Vec2::splat(1.0), Vec2::splat(3.0));
                gp.rect_filled(dot, 1.5, white(178));
                marks.borrow_mut().push(dot);
            }
        }
    };
    for n in &notes {
        if !moving.iter().any(|(o, _)| o == *n) {
            draw(n, app.selected.contains(n).then_some(Color32::WHITE));
        }
        // Swing: where the note actually plays, if the groove moves it.
        let played = deckard_engine::swung(p, app.group, n.sound, n.position);
        if played != n.position
            && let Some(r) = note_rect(n)
        {
            let x = x_of(played as f64);
            gp.line_segment([Pos2::new(x, r.top() - 1.0), Pos2::new(x, r.bottom() + 1.0)], Stroke::new(2.0, Color32::WHITE));
            gp.line_segment([Pos2::new(r.left(), r.center().y), Pos2::new(x, r.center().y)], Stroke::new(1.0, white(160)));
        }
    }
    for (_, m) in &moving {
        draw(m, Some(Color32::WHITE));
    }
    match &drag {
        Some(NoteDrag::Paint { notes: ns }) => ns.iter().for_each(|n| draw(n, Some(Color32::WHITE))),
        Some(NoteDrag::Select { origin, to }) => {
            let r = Rect::from_two_pos(*origin, *to);
            gp.rect_filled(r, 0.0, white(16));
            gp.rect_stroke(r, 0.0, Stroke::new(1.0, white(120)), StrokeKind::Inside);
        }
        _ => {}
    }
    let marks = marks.into_inner();
    ui.ctx().data_mut(|d| d.insert_temp(header_rect("expression marks"), marks));
    if pat.is_some() {
        let resp = ui.interact(grid, ui.id().with("grid"), Sense::click_and_drag());
        let pointer = resp.interact_pointer_pos();
        // Preview on press: the note under the pointer, or the one a click will draw.
        if pressed(ui, &resp)
            && let Some(pos) = pointer
            && let Some(n) = hit(pos).map(|(n, _)| n).or_else(|| app.draw_mode().then(|| note_at(pos)).flatten())
        {
            acts.push(Act::HoldPitch(app.group, n.sound, n.pitch));
        }
        // Drags are measured from where the button went down (egui reports the drag
        // only after the pointer has moved a little).
        let press = ui.input(|i| i.pointer.press_origin());
        let shift = ui.input(|i| i.modifiers.shift);
        if resp.clicked() || resp.secondary_clicked() || resp.drag_started() {
            acts.push(Act::KeyFocus(KeyFocus::Events));
        }
        // Erase, Split, Join and Mute act on clicks only.
        let tool = matches!(app.mouse, MouseMode::Erase | MouseMode::Split | MouseMode::Join | MouseMode::Mute);
        // Where Split cuts: the grid line nearest the pointer (⌘: right under it).
        let cut_at = |x: f32| {
            let t = tick_at(x) as i64;
            if ui.input(|i| i.modifiers.command) { t } else { (t + app.grid.max(1) / 2).div_euclid(app.grid.max(1)) * app.grid.max(1) }
        };
        if app.mouse == MouseMode::Split
            && let Some(pos) = resp.hover_pos()
            && let Some((n, _)) = hit(pos)
            && let Some(r) = note_rect(&n)
        {
            let x = x_of(cut_at(pos.x) as f64);
            if x > r.left() && x < r.right() {
                gp.line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], Stroke::new(1.5, Color32::WHITE));
            }
        }
        if resp.drag_started()
            && !tool
            && let Some(at) = press.or(pointer)
        {
            drag = Some(match hit(at) {
                Some((n, true)) => {
                    // A selected note resizes the whole selection.
                    let ns = if app.selected.contains(&n) { app.selected.clone() } else { vec![n] };
                    NoteDrag::Resize { notes: ns, anchor: n, length: n.length }
                }
                Some((n, false)) => {
                    let group_move = !app.draw_mode() && app.selected.contains(&n);
                    if !app.draw_mode() && !group_move {
                        acts.push(Act::SelectNotes(vec![n]));
                    }
                    NoteDrag::Move { notes: if group_move { app.selected.clone() } else { vec![n] }, ticks: 0, rows: 0 }
                }
                None if app.draw_mode() => NoteDrag::Paint { notes: note_at(at).into_iter().collect() },
                None => NoteDrag::Select { origin: at, to: at },
            });
            ui.data_mut(|d| d.insert_temp(ui.id().with("note-drag-origin"), at));
        }
        let origin: Option<Pos2> = ui.data(|d| d.get_temp(ui.id().with("note-drag-origin")));
        if let (Some(d), Some(pos), Some(origin)) = (drag.as_mut(), pointer, origin)
            && resp.dragged()
        {
            match d {
                NoteDrag::Move { notes: ns, ticks, rows } => {
                    // Snap the grabbed note's new start to the grid; the rest follow.
                    let anchor = hit(origin).map_or(ns[0], |(n, _)| n);
                    let rel = anchor.position;
                    let target = (rel + ((pos.x - origin.x) / scale) as i64).div_euclid(snap) * snap;
                    *ticks = target - rel;
                    *rows = row_at(pos.y) - row_at(origin.y);
                }
                NoteDrag::Resize { anchor, length, .. } => {
                    let rel = anchor.position;
                    let end = (rel + anchor.length + ((pos.x - origin.x) / scale) as i64 + snap - 1).div_euclid(snap) * snap;
                    *length = (end - rel).max(snap);
                }
                NoteDrag::Paint { notes: ns } => {
                    if let Some(n) = note_at(pos)
                        && hit(pos).is_none()
                        && !ns.iter().any(|m| m.sound == n.sound && m.pitch == n.pitch && m.position == n.position)
                    {
                        ns.push(n);
                    }
                }
                NoteDrag::Select { to, .. } => *to = pos,
                NoteDrag::Lane { .. } | NoteDrag::Midi { .. } | NoteDrag::Expr(_) | NoteDrag::Span { .. } => {}
            }
        }
        if resp.drag_stopped() {
            match drag.take() {
                Some(d @ (NoteDrag::Move { .. } | NoteDrag::Resize { .. })) => {
                    let pairs: Vec<(Note, Note)> = moved(&d).into_iter().filter(|(a, b)| a != b).collect();
                    if !pairs.is_empty() {
                        let label = if matches!(d, NoteDrag::Resize { .. }) { "Resize Notes" } else { "Move Notes" };
                        let (remove, add): (Vec<Note>, Vec<Note>) = pairs.into_iter().unzip();
                        acts.push(Act::EditNotes { g: app.group, p: app.pattern, label, remove, add });
                    }
                }
                Some(NoteDrag::Paint { notes: ns }) if !ns.is_empty() => {
                    acts.push(Act::EditNotes { g: app.group, p: app.pattern, label: "Draw Notes", remove: Vec::new(), add: ns });
                }
                Some(NoteDrag::Select { origin, to }) => {
                    let r = Rect::from_two_pos(origin, to);
                    let mut sel: Vec<Note> =
                        notes.iter().filter(|n| note_rect(n).is_some_and(|nr| nr.intersects(r))).map(|n| **n).collect();
                    if shift {
                        let extra: Vec<Note> = app.selected.iter().filter(|n| !sel.contains(n)).copied().collect();
                        sel.extend(extra);
                    }
                    acts.push(Act::SelectNotes(sel));
                }
                _ => {}
            }
        }
        if resp.clicked()
            && tool
            && let Some(pos) = pointer
            && let Some((n, _)) = hit(pos)
        {
            let all: Vec<Note> = notes.iter().map(|n| **n).collect();
            let selected: Vec<Note> = app.selected.iter().filter(|m| all.contains(m)).copied().collect();
            let (g, p) = (app.group, app.pattern);
            match app.mouse {
                MouseMode::Erase => {
                    acts.push(Act::SelectNotes(Vec::new()));
                    acts.push(Act::EditNotes { g, p, label: "Delete Note", remove: vec![n], add: Vec::new() });
                }
                MouseMode::Split => {
                    let targets = if selected.contains(&n) { selected } else { vec![n] };
                    let at = cut_at(pos.x);
                    if !deckard_doc::note_tools::split(&targets, at).0.is_empty() {
                        acts.push(Act::SelectNotes(Vec::new()));
                        acts.push(Act::SplitNotes { g, p, targets, at });
                    }
                }
                MouseMode::Join => {
                    if !deckard_doc::note_tools::join(&all, n, &selected).0.is_empty() {
                        acts.push(Act::SelectNotes(Vec::new()));
                        acts.push(Act::JoinNotes { g, p, all, clicked: n, selected });
                    }
                }
                _ => {
                    let ((remove, add), label) = deckard_doc::note_tools::toggle_mute(n, &selected);
                    if !remove.is_empty() {
                        acts.push(Act::SelectNotes(Vec::new()));
                        acts.push(Act::EditNotes { g, p, label, remove, add });
                    }
                }
            }
        } else if resp.clicked()
            && !tool
            && let Some(pos) = pointer
        {
            match (hit(pos), app.draw_mode()) {
                (Some((n, _)), true) => {
                    acts.push(Act::EditNotes { g: app.group, p: app.pattern, label: "Delete Note", remove: vec![n], add: Vec::new() })
                }
                (Some((n, _)), false) => {
                    let mut sel = if shift { app.selected.clone() } else { Vec::new() };
                    match sel.iter().position(|m| *m == n) {
                        Some(i) if shift => {
                            sel.remove(i);
                        }
                        _ => sel.push(n),
                    }
                    acts.push(Act::SelectNotes(sel));
                    acts.push(Act::FocusSound(n.sound));
                }
                (None, true) => {
                    if let Some(n) = note_at(pos) {
                        acts.push(Act::EditNotes { g: app.group, p: app.pattern, label: "Add Note", remove: Vec::new(), add: vec![n] });
                        acts.push(Act::FocusSound(n.sound));
                    }
                }
                (None, false) => acts.push(Act::SelectNotes(Vec::new())),
            }
        }
        // Double-click: a note on an empty spot, or delete the note under the pointer; on
        // an Audio plug-in's row in Loop mode, its loop on or off in the pattern.
        let loop_row = |pos: Pos2| -> Option<usize> {
            let s = usize::try_from(row_at(pos.y)).ok()?;
            let snd = group.sounds.get(s)?;
            let a = snd.audio.as_ref().filter(|_| snd.instrument == "AudioModule" && !app.piano_roll)?;
            (a.mode == deckard_model::AudioMode::Loop && !a.takes.is_empty()).then_some(s)
        };
        if resp.double_clicked()
            && !app.draw_mode()
            && !tool
            && let Some(pos) = pointer
            && let Some(s) = loop_row(pos).filter(|_| hit(pos).is_none() && pat.is_some())
        {
            acts.push(Act::ToggleAudio(app.group, s, app.pattern));
        } else if resp.double_clicked()
            && !app.draw_mode()
            && !tool
            && let Some(pos) = pointer
        {
            match hit(pos) {
                Some((n, _)) => {
                    acts.push(Act::EditNotes { g: app.group, p: app.pattern, label: "Delete Note", remove: vec![n], add: Vec::new() })
                }
                None => {
                    if let Some(n) = note_at(pos) {
                        acts.push(Act::EditNotes { g: app.group, p: app.pattern, label: "Add Note", remove: Vec::new(), add: vec![n] });
                        acts.push(Act::FocusSound(n.sound));
                    }
                }
            }
        }
        // Right-click: the event area's menu. A note not in the selection becomes it;
        // in Group view the row's sound takes the focus (a paste goes there).
        let menu_id = ui.id().with("event-menu-at");
        if resp.secondary_clicked()
            && let Some(pos) = pointer
        {
            ui.data_mut(|d| d.insert_temp(menu_id, pos));
            match hit(pos) {
                Some((n, _)) if !app.selected.contains(&n) => {
                    acts.push(Act::SelectNotes(vec![n]));
                    acts.push(Act::FocusSound(n.sound));
                }
                Some(_) => {}
                None => {
                    if let Some(n) = note_at(pos).filter(|_| !app.piano_roll) {
                        acts.push(Act::FocusSound(n.sound));
                    }
                }
            }
        }
        let paste_at = ui.data(|d| d.get_temp::<Pos2>(menu_id)).map_or(crate::clipboard::PasteAt::Keys, |pos| {
            crate::clipboard::PasteAt::Mouse { tick: tick_at(pos.x) as i64, pitch: app.piano_roll.then(|| hi - row_at(pos.y)) }
        });
        resp.context_menu(|ui| event_menu(ui, app, paste_at, acts));
        // ⌘X / ⌘C / ⌘V (macOS sends them as Cut, Copy and Paste).
        if !ui.ctx().egui_wants_keyboard_input() && app.key_focus == KeyFocus::Events {
            let (cut, copy, paste) = ui.input_mut(|i| {
                let ev = |i: &egui::InputState, f: fn(&egui::Event) -> bool| i.events.iter().any(f);
                let cut = ev(i, |e| matches!(e, egui::Event::Cut)) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::X);
                let copy = ev(i, |e| matches!(e, egui::Event::Copy)) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::C);
                let paste = ev(i, |e| matches!(e, egui::Event::Paste(_))) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::V);
                (cut, copy, paste)
            });
            if cut || copy {
                acts.push(Act::CopyNotes { cut });
            }
            if paste {
                acts.push(Act::PasteNotes(crate::clipboard::PasteAt::Keys));
            }
        }
        // ⌘A selects the focused sound's notes; Esc clears the selection.
        if !ui.ctx().egui_wants_keyboard_input() {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
                acts.push(Act::SelectNotes(notes.iter().filter(|n| n.sound == app.sound).map(|n| **n).collect()));
            }
            if !app.selected.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                acts.push(Act::SelectNotes(Vec::new()));
            }
        }
        // ⌥ + arrows nudge by the Nudge Grid: the selection, or every note shown, coming
        // round at the pattern's ends (Maschine's Nudge).
        let (left, right, alt) = ui.input(|i| (i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::ArrowRight), i.modifiers.alt));
        if alt && (left || right) && !ui.ctx().egui_wants_keyboard_input() {
            let amount = app.nudge.ticks(app.grid);
            let shown: Vec<Note> = notes.iter().map(|n| **n).collect();
            let targets: Vec<Note> = app.selected.iter().filter(|n| shown.contains(n)).copied().collect();
            let targets = if targets.is_empty() { shown } else { targets };
            let (remove, add) = deckard_doc::note_tools::nudge(&targets, if left { -amount } else { amount }, start, end);
            if !remove.is_empty() {
                if !app.selected.is_empty() {
                    acts.push(Act::SelectNotes(add.clone()));
                }
                acts.push(Act::EditNotes { g: app.group, p: app.pattern, label: "Nudge Notes", remove, add });
            }
        }
        // Arrows alone move the selection by the step grid.
        if !app.selected.is_empty() && !alt && !ui.ctx().egui_wants_keyboard_input() {
            let delta = if left {
                -app.grid
            } else if right {
                app.grid
            } else {
                0
            };
            if delta != 0 {
                let moved: Vec<Note> =
                    app.selected.iter().map(|n| Note { position: (n.position + delta).clamp(start, start + view - 1), ..*n }).collect();
                if moved != app.selected {
                    acts.push(Act::EditNotes {
                        g: app.group,
                        p: app.pattern,
                        label: "Nudge Notes",
                        remove: app.selected.clone(),
                        add: moved,
                    });
                }
            }
        }
        if !app.selected.is_empty()
            && app.key_focus == KeyFocus::Events
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            acts.push(Act::EditNotes {
                g: app.group,
                p: app.pattern,
                label: "Delete Notes",
                remove: app.selected.clone(),
                add: Vec::new(),
            });
        }
        if resp.hovered() {
            let edge = ui.input(|i| i.pointer.hover_pos()).and_then(hit).is_some_and(|(_, e)| e);
            // Each mode's cursor: pencil (a crosshair here), cut line, plus, cross.
            let icon = match app.mouse {
                MouseMode::Split => egui::CursorIcon::Text,
                MouseMode::Join => egui::CursorIcon::Copy,
                MouseMode::Mute | MouseMode::Erase => egui::CursorIcon::NotAllowed,
                _ if edge => egui::CursorIcon::ResizeHorizontal,
                MouseMode::Draw => egui::CursorIcon::Crosshair,
                MouseMode::Select => egui::CursorIcon::Default,
            };
            ui.ctx().set_cursor_icon(icon);
        }
    }

    // ---- The lane: the focused sound's velocities, probabilities or MIDI events -----
    let bar_x = |n: &Note| x_of(n.position as f64);
    let scol = sound_color(p, app.group, app.sound);
    let midi: Vec<MidiEvent> = pat.map_or(Vec::new(), |pt| pt.midi.iter().filter(|m| m.sound == app.sound).copied().collect());
    let lanes = lane_list(app, p, &midi);
    // The list at the left picks the lane; "+" adds or removes MIDI controller lanes.
    // "+" stays in view at the bottom; the lanes scroll above it.
    const ITEM_H: f32 = 20.0;
    let plus = Rect::from_min_max(Pos2::new(col.left(), lane.bottom() - ITEM_H), Pos2::new(col.right(), lane.bottom()));
    let lane_col = Rect::from_min_max(Pos2::new(col.left(), lane.top()), Pos2::new(col.right(), plus.top()));
    let off = scroll(ui, "lane-list", lane_col, lanes.len() as f32 * ITEM_H + 8.0);
    let lp = painter.with_clip_rect(lane_col.union(plus));
    let mut items = Vec::new();
    for i in 0..=lanes.len() {
        let r = if i == lanes.len() {
            plus
        } else {
            Rect::from_min_size(Pos2::new(lane_col.left(), lane_col.top() + 4.0 + i as f32 * ITEM_H - off), Vec2::new(COLUMN, ITEM_H))
        };
        let vis = if i == lanes.len() { plus } else { r.intersect(lane_col) };
        if vis.height() <= 0.0 {
            continue;
        }
        let resp = ui.interact(vis, ui.id().with(("lane-item", i)), Sense::click());
        let y = r.center().y;
        match lanes.get(i) {
            Some(&l) => {
                let on = l == app.lane;
                if on {
                    lp.rect_filled(r.shrink2(Vec2::new(4.0, 1.0)), 2.0, S02);
                }
                let col = if on || resp.hovered() { Color32::WHITE } else { white(142) };
                lp.text(Pos2::new(r.left() + 8.0, y), Align2::LEFT_CENTER, l.name(), font(12.0), col);
                if resp.clicked() {
                    acts.push(Act::Lane(l));
                }
                items.push((l, r));
            }
            None => {
                let col = if app.lanes_dialog || resp.hovered() { Color32::WHITE } else { white(142) };
                lp.text(Pos2::new(r.left() + 8.0, y), Align2::LEFT_CENTER, "+", font(16.0), col);
                if resp.on_hover_text("Add or remove MIDI controller lanes").clicked() {
                    acts.push(Act::LanesDialog(!app.lanes_dialog));
                }
                items.push((Lane::Velocity, r));
            }
        }
    }
    // The list's rows (the last one is "+"), for tests that drive the window.
    ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-lane-list"), items));

    painter.rect_filled(lane, 0.0, LANE);
    let vp = painter.with_clip_rect(Rect::from_min_max(lane.min, Pos2::new(grid.right(), lane.bottom())));
    let lane_in = lane.shrink2(Vec2::new(0.0, 7.0));
    for f in [0.25, 0.5, 0.75] {
        vp.hline(lane.x_range(), lane_in.bottom() - lane_in.height() * f, Stroke::new(1.0, white(6)));
    }
    let mut t = 0;
    while t <= view {
        if t % beat == 0 {
            vp.vline(x_of(t as f64), lane.y_range(), Stroke::new(1.0, Color32::from_gray(if t % bar == 0 { 0x48 } else { 0x21 })));
        }
        t += beat;
    }
    // A span of a MIDI or expression lane (BUGS #17): Shift-drag selects it, Delete then
    // erases it; with the Erase tool a drag erases it at once, a click one step. The
    // lane's own editing waits meanwhile.
    let spans = app.lane.note_kind().is_none() && pat.is_some();
    let span_step = app.grid.max(1);
    let span_at = |x: f32| ((tick_at(x) as i64).clamp(start, end - 1) / span_step) * span_step;
    let span_of = |from: i64, to: i64| (from.min(to), from.max(to) + span_step);
    let spanning = spans
        && (matches!(&drag, Some(NoteDrag::Span { lane: l, .. }) if *l == app.lane)
            || app.mouse == MouseMode::Erase
            || ui.input(|i| i.modifiers.shift));
    if let Some(kind) = app.lane.note_kind() {
        'notes: {
            let lane_in = lane.shrink2(Vec2::new(0.0, 7.0));
            let max = kind.max();
            let value_at =
                |y: f32| (((lane_in.bottom() - y) / lane_in.height()) * max as f32).round().clamp(kind.min() as f32, max as f32) as i64;
            let editing: Vec<(Note, i64)> = match &drag {
                Some(NoteDrag::Lane { kind: k, values, .. }) if *k == kind => values.clone(),
                _ => Vec::new(),
            };
            let mine: Vec<Note> = notes.iter().filter(|n| n.sound == app.sound).map(|n| **n).collect();
            for n in &mine {
                let edited = editing.iter().find(|(m, _)| m == n).map(|(_, v)| *v);
                let v = edited.unwrap_or_else(|| kind.get(n));
                let x = bar_x(n);
                let h = lane_in.height() * v.clamp(0, max) as f32 / max as f32;
                let col = if edited.is_some() || app.selected.contains(n) { Color32::WHITE } else { scol };
                vp.rect_filled(Rect::from_min_max(Pos2::new(x, lane_in.bottom() - h), Pos2::new(x + 3.0, lane_in.bottom())), 0.0, col);
            }
            if pat.is_none() {
                break 'notes;
            }
            let tip = if app.draw_mode() { "Drag across to draw values" } else { "Drag a bar up or down" };
            let resp = ui.interact(lane, ui.id().with("note-lane"), Sense::click_and_drag()).on_hover_text(tip);
            let pointer = resp.interact_pointer_pos();
            let nearest = |x: f32| {
                mine.iter().copied().filter(|n| (bar_x(n) + 1.5 - x).abs() < 8.0).min_by_key(|n| (bar_x(n) + 1.5 - x).abs() as i64)
            };
            if resp.drag_started()
                && let Some(at) = ui.input(|i| i.pointer.press_origin()).or(pointer)
            {
                let grabbed = nearest(at.x);
                drag = Some(match (app.draw_mode(), grabbed) {
                    // A selected note's bar: the whole selection moves with it.
                    (false, Some(n)) if app.selected.contains(&n) && app.selected.len() > 1 => {
                        let base: Vec<(Note, i64)> = app.selected.iter().map(|m| (*m, kind.get(m))).collect();
                        NoteDrag::Lane { kind, values: base.clone(), base, anchor: Some((n, kind.get(&n))) }
                    }
                    (_, n) => NoteDrag::Lane {
                        kind,
                        values: n.map(|n| (n, value_at(at.y))).into_iter().collect(),
                        base: Vec::new(),
                        anchor: None,
                    },
                });
            }
            if let (Some(NoteDrag::Lane { kind: k, values, base, anchor }), Some(pos)) = (drag.as_mut(), pointer)
                && *k == kind
                && resp.dragged()
            {
                if let Some((_, from)) = anchor {
                    // Relative: every selected note changes by as much as the grabbed one.
                    let delta = value_at(pos.y) - *from;
                    *values = base.iter().map(|(m, b)| (*m, (b + delta).clamp(kind.min(), kind.max()))).collect();
                } else if app.draw_mode() {
                    // Every bar the pointer passes takes the value under it.
                    let x0 = pos.x - resp.drag_delta().x;
                    let (a, b) = (x0.min(pos.x) - 2.0, x0.max(pos.x) + 2.0);
                    for n in mine.iter().filter(|n| (a..=b).contains(&(bar_x(n) + 1.5))) {
                        match values.iter_mut().find(|(m, _)| m == n) {
                            Some((_, v)) => *v = value_at(pos.y),
                            None => values.push((*n, value_at(pos.y))),
                        }
                    }
                } else if let Some((_, v)) = values.first_mut() {
                    *v = value_at(pos.y);
                }
            }
            let mut commit = |values: Vec<(Note, i64)>| {
                let pairs: Vec<(Note, Note)> = values.into_iter().map(|(n, v)| (n, kind.with(n, v))).filter(|(a, b)| a != b).collect();
                if !pairs.is_empty() {
                    let (remove, add): (Vec<Note>, Vec<Note>) = pairs.into_iter().unzip();
                    acts.push(Act::EditNotes { g: app.group, p: app.pattern, label: kind.label(), remove, add });
                }
            };
            if resp.drag_stopped()
                && let Some(NoteDrag::Lane { kind: k, values, .. }) = drag.clone()
                && k == kind
            {
                drag = None;
                commit(values);
            }
            if resp.clicked()
                && let Some(pos) = pointer
                && let Some(n) = nearest(pos.x)
            {
                commit(vec![(n, value_at(pos.y))]);
            }
        }
    } else if expr_lane::dim_of(app.lane).is_some() {
        let mine: Vec<Note> = notes.iter().filter(|n| n.sound == app.sound).map(|n| **n).collect();
        let area = expr_lane::Area {
            lane,
            inner: lane.shrink2(Vec2::new(0.0, 7.0)),
            x_of: &|t| x_of(t),
            tick_at: &|x| tick_at(x),
            notes: &mine,
            editable: pat.is_some() && !spanning,
            color: scol,
        };
        expr_lane::lane(ui, app, p, &vp, area, &mut drag, acts);
    } else {
        // A MIDI controller (or pitch bend): each value holds until the next event.
        let l = app.lane;
        let max = l.max();
        let y_of = |v: i64| lane_in.bottom() - lane_in.height() * v as f32 / max as f32;
        let value_at = |y: f32| (((lane_in.bottom() - y) / lane_in.height()) * max as f32).round().clamp(0.0, max as f32) as i64;
        let step = app.grid.max(1);
        let step_at = |x: f32| ((tick_at(x) as i64).clamp(start, end - 1) / step) * step;
        let (painting, erasing) = match &drag {
            Some(NoteDrag::Midi { lane: dl, steps, erase }) if *dl == l => (steps.clone(), *erase),
            _ => (Vec::new(), false),
        };
        let in_steps = |steps: &[(i64, i64)], at: i64| steps.iter().any(|(t, _)| (*t..*t + step).contains(&at));
        let mut shown: Vec<(i64, i64, bool)> = midi
            .iter()
            .filter_map(|m| l.value(&m.kind).map(|v| (m.position, v, false)))
            .filter(|(t, _, _)| !in_steps(&painting, *t))
            .collect();
        if !erasing {
            shown.extend(painting.iter().map(|(t, v)| (*t, *v, true)));
        }
        shown.sort_by_key(|(t, _, _)| *t);
        let base = if l == Lane::PitchBend { y_of(8192) } else { lane_in.bottom() };
        if l == Lane::PitchBend {
            vp.hline(lane.x_range(), base, Stroke::new(1.0, white(40)));
        }
        let end_x = x_of(end as f64);
        for (i, (t, v, painted)) in shown.iter().enumerate() {
            let x = x_of(*t as f64);
            let x2 = shown.get(i + 1).map_or(end_x, |(t2, _, _)| x_of(*t2 as f64));
            let y = y_of(*v);
            vp.rect_filled(Rect::from_two_pos(Pos2::new(x, y), Pos2::new(x2, base)), 0.0, scol.gamma_multiply(0.25));
            vp.hline(x..=x2, y, Stroke::new(1.5, scol));
            // The jump from the previous value (or the lane's base).
            let prev = if i == 0 { base } else { y_of(shown[i - 1].1) };
            vp.vline(x, prev.min(y)..=prev.max(y), Stroke::new(1.5, scol));
            vp.rect_filled(
                Rect::from_center_size(Pos2::new(x + 1.0, y), Vec2::splat(5.0)),
                1.0,
                if *painted { Color32::WHITE } else { scol },
            );
        }
        if shown.is_empty() {
            vp.text(lane.center(), Align2::CENTER_CENTER, format!("No {} events: click or drag to draw", l.name()), font(11.0), white(61));
        }
        if pat.is_some() && !spanning {
            let resp = ui
                .interact(lane, ui.id().with("midi-lane"), Sense::click_and_drag())
                .on_hover_text("Click or drag to draw a value per grid step. Right-click or right-drag erases.");
            let pointer = resp.interact_pointer_pos();
            let set = |steps: &mut Vec<(i64, i64)>, t: i64, v: i64| match steps.iter_mut().find(|(s, _)| *s == t) {
                Some((_, old)) => *old = v,
                None => steps.push((t, v)),
            };
            if resp.drag_started()
                && let Some(at) = ui.input(|i| i.pointer.press_origin()).or(pointer)
            {
                let erase = ui.input(|i| i.pointer.secondary_down());
                drag = Some(NoteDrag::Midi { lane: l, erase, steps: vec![(step_at(at.x), value_at(at.y))] });
            }
            if let (Some(NoteDrag::Midi { lane: dl, steps, .. }), Some(pos)) = (drag.as_mut(), pointer)
                && *dl == l
                && resp.dragged()
            {
                // Every step the pointer passed takes the value under it.
                let x0 = pos.x - resp.drag_delta().x;
                let (a, b) = (step_at(x0.min(pos.x)), step_at(x0.max(pos.x)));
                let mut t = a;
                while t <= b {
                    set(steps, t, value_at(pos.y));
                    t += step;
                }
            }
            let mut commit = |steps: Vec<(i64, i64)>, erase: bool| {
                let remove: Vec<MidiEvent> =
                    midi.iter().filter(|m| Lane::of(&m.kind) == l && in_steps(&steps, m.position)).copied().collect();
                let add: Vec<MidiEvent> = if erase {
                    Vec::new()
                } else {
                    steps.iter().filter_map(|(t, v)| Some(MidiEvent { sound: app.sound, position: *t, kind: l.with(*v)? })).collect()
                };
                if remove != add && !(remove.is_empty() && add.is_empty()) {
                    let label = if erase { "Erase MIDI" } else { "Draw MIDI" };
                    acts.push(Act::EditMidi { g: app.group, p: app.pattern, label, remove, add });
                }
            };
            if resp.drag_stopped()
                && let Some(NoteDrag::Midi { lane: dl, steps, erase }) = drag.clone()
                && dl == l
            {
                drag = None;
                commit(steps, erase);
            }
            if let Some(pos) = pointer {
                if resp.clicked() {
                    commit(vec![(step_at(pos.x), value_at(pos.y))], false);
                } else if resp.secondary_clicked() {
                    commit(vec![(step_at(pos.x), 0)], true);
                }
            }
        }
    }

    if spans {
        let shown = match &drag {
            Some(NoteDrag::Span { lane: l, from, to, erase }) if *l == app.lane => Some((span_of(*from, *to), *erase)),
            _ => app.lane_span.filter(|(l, ..)| *l == app.lane).map(|(_, a, b)| ((a, b), false)),
        };
        if let Some(((a, b), erase)) = shown {
            let r = Rect::from_x_y_ranges(x_of(a as f64)..=x_of(b as f64), lane.y_range());
            vp.rect_filled(r, 0.0, if erase { Color32::from_rgba_unmultiplied(0xe0, 0x50, 0x50, 40) } else { white(24) });
            vp.rect_stroke(r, 0.0, Stroke::new(1.0, white(90)), egui::StrokeKind::Inside);
        }
    }
    if spanning {
        let erase = app.mouse == MouseMode::Erase;
        let resp = ui.interact(lane, ui.id().with("lane-span"), Sense::click_and_drag()).on_hover_text(if erase {
            "Click or drag to erase this lane's values there"
        } else {
            "Shift-drag to select a span; Delete erases it"
        });
        let pointer = resp.interact_pointer_pos();
        if resp.drag_started()
            && let Some(at) = ui.input(|i| i.pointer.press_origin()).or(pointer)
        {
            let t = span_at(at.x);
            drag = Some(NoteDrag::Span { lane: app.lane, from: t, to: t, erase });
            acts.push(Act::KeyFocus(KeyFocus::Events));
        }
        if let (Some(NoteDrag::Span { to, .. }), Some(pos)) = (drag.as_mut(), pointer)
            && resp.dragged()
        {
            *to = span_at(pos.x);
        }
        if resp.drag_stopped()
            && let Some(NoteDrag::Span { lane: l, from, to, erase }) = drag.clone()
        {
            drag = None;
            let (a, b) = span_of(from, to);
            acts.push(if erase { Act::EraseSpan(l, a, b) } else { Act::LaneSpan(Some((l, a, b))) });
        }
        if resp.clicked()
            && let Some(pos) = pointer
        {
            let t = span_at(pos.x);
            acts.push(if erase { Act::EraseSpan(app.lane, t, t + span_step) } else { Act::LaneSpan(None) });
        }
    }
    if let Some((l, a, b)) = app.lane_span
        && l == app.lane
        && app.key_focus == KeyFocus::Events
        && !ui.ctx().egui_wants_keyboard_input()
        && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
    {
        acts.push(Act::EraseSpan(l, a, b));
    }

    // The "+" dialog: MIDI controller lanes to add, and the ones shown to remove.
    if app.lanes_dialog {
        let mut open = true;
        let sound = p.groups[app.group].sounds.get(app.sound).map_or(String::new(), |s| s.name.clone());
        egui::Window::new("MIDI Lanes").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, Vec2::ZERO).open(&mut open).show(
            ui.ctx(),
            |ui| {
                ui.label(egui::RichText::new(format!("Lanes for {sound}")).weak());
                ui.add_space(4.0);
                let shown: Vec<Lane> =
                    lanes.iter().copied().filter(|l| l.note_kind().is_none() && expr_lane::dim_of(*l).is_none()).collect();
                if shown.is_empty() {
                    ui.label(egui::RichText::new("No MIDI lanes yet").weak());
                }
                for l in &shown {
                    ui.horizontal(|ui| {
                        ui.label(l.name());
                        let n = midi.iter().filter(|m| Lane::of(&m.kind) == *l).count();
                        let b = if n == 0 {
                            ui.button("Remove")
                        } else {
                            ui.button(format!("Remove ({n} event{})", if n == 1 { "" } else { "s" }))
                                .on_hover_text("Deletes this sound's events in this pattern (Undo brings them back)")
                        };
                        if b.clicked() {
                            acts.push(Act::RemoveLane(*l));
                        }
                    });
                }
                ui.separator();
                // The Expression editor's lanes: listed once the sound's notes have
                // expression; picked here to draw it from scratch.
                ui.label("Expression (per note)");
                ui.horizontal(|ui| {
                    for l in EXPR_LANES {
                        if ui.selectable_label(app.lane == l, l.name()).clicked() {
                            acts.push(Act::Lane(l));
                            acts.push(Act::LanesDialog(false));
                        }
                    }
                });
                // Those the notes have, to take off them (BUGS #18).
                let notes = pat.map_or(&[][..], |pt| &pt.notes[..]);
                let had = |l: Lane| notes.iter().any(|n| n.sound == app.sound && has_lane(&expr_lane::expression_of(p, n), l));
                for l in EXPR_LANES.into_iter().filter(|&l| had(l)) {
                    ui.horizontal(|ui| {
                        ui.label(l.name());
                        if ui
                            .button("Remove")
                            .on_hover_text("Clears it from this sound's notes in this pattern (Undo brings it back)")
                            .clicked()
                        {
                            acts.push(Act::RemoveLane(l));
                        }
                    });
                }
                ui.separator();
                ui.label("Add a lane");
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    for l in [Lane::PitchBend, Lane::ChannelPressure]
                        .into_iter()
                        .chain((0..=127).map(Lane::Controller))
                        .filter(|l| !lanes.contains(l))
                    {
                        if ui.button(l.name()).clicked() {
                            acts.push(Act::AddLane(l));
                        }
                    }
                });
            },
        );
        if !open {
            acts.push(Act::LanesDialog(false));
        }
    }
    ui.data_mut(|d| match drag {
        Some(nd) => {
            d.insert_temp(drag_id, nd);
        }
        None => d.remove::<NoteDrag>(drag_id),
    });

    // Playhead.
    if let Some(t) = pat.and_then(|_| pattern_position(p, head, app.group, app.pattern)) {
        let x = x_of(start as f64 + t);
        painter.vline(x, Rect::from_min_max(tl.min, rect.max).y_range(), Stroke::new(1.0, Color32::WHITE));
    }
}

/// Grid sizes, in ticks (960 per quarter note).
/// The Step Grid's values, as Maschine lists them.
pub const GRIDS: [(&str, i64); 15] = deckard_lua_host::api::pattern_editor::STEP_GRIDS;

fn grid_name(ticks: i64) -> String {
    GRIDS.iter().find(|(_, t)| *t == ticks).map_or(format!("{ticks} t"), |(n, _)| n.to_string())
}

/// A drag in the pattern editor or its lanes, until the button is released.
#[derive(Debug, Clone)]
enum NoteDrag {
    /// Notes moved by `ticks` and `rows` (sound rows, or pitches in the piano roll).
    Move { notes: Vec<Note>, ticks: i64, rows: i64 },
    /// `notes` (the grabbed note, or the selection it belongs to) resized by as
    /// much as the grabbed note `anchor`, whose new length is `length`.
    Resize { notes: Vec<Note>, anchor: Note, length: i64 },
    /// Draw mode: notes painted so far.
    Paint { notes: Vec<Note> },
    /// Box selection.
    Select { origin: Pos2, to: Pos2 },
    /// New velocity or probability values. When a selected note's bar is dragged, the
    /// whole selection moves with it: `base` holds their values before the drag and
    /// `anchor` the grabbed note's.
    Lane { kind: LaneKind, values: Vec<(Note, i64)>, base: Vec<(Note, i64)>, anchor: Option<(Note, i64)> },
    /// MIDI values drawn so far in a controller lane, one per grid step (position,
    /// value), or the steps being erased.
    Midi { lane: Lane, erase: bool, steps: Vec<(i64, i64)> },
    /// An Expression lane's edit.
    Expr(expr_lane::ExprDrag),
    /// A span of a MIDI or expression lane, from the step pressed on to the step
    /// under the pointer: erased (the Erase tool) or selected (Shift) on release.
    Span { lane: Lane, from: i64, to: i64, erase: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaneKind {
    Velocity,
    Probability,
}

impl LaneKind {
    fn label(self) -> &'static str {
        match self {
            LaneKind::Velocity => "Velocity",
            LaneKind::Probability => "Probability",
        }
    }

    fn get(self, n: &Note) -> i64 {
        match self {
            LaneKind::Velocity => n.velocity,
            LaneKind::Probability => n.probability,
        }
    }

    fn with(self, n: Note, v: i64) -> Note {
        match self {
            LaneKind::Velocity => Note { velocity: v, ..n },
            LaneKind::Probability => Note { probability: v, ..n },
        }
    }

    fn min(self) -> i64 {
        match self {
            LaneKind::Velocity => 1,
            LaneKind::Probability => 0,
        }
    }

    fn max(self) -> i64 {
        match self {
            LaneKind::Velocity => 127,
            LaneKind::Probability => 100,
        }
    }
}

/// What the lane under the note area shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lane {
    #[default]
    Velocity,
    Probability,
    Controller(u8),
    PitchBend,
    /// Channel pressure (Maschine's "Aftertouch" lane).
    ChannelPressure,
    /// The Expression editor: each note's curve of a dimension.
    Expr(deckard_model::Dim),
    /// The Expression editor: each note's release velocity.
    Release,
}

impl Lane {
    pub fn name(self) -> String {
        match self {
            Lane::Velocity => "Velocity".into(),
            Lane::Probability => "Probability".into(),
            Lane::PitchBend => "Pitch Bend".into(),
            Lane::ChannelPressure => "Aftertouch".into(),
            Lane::Expr(d) => d.name().into(),
            Lane::Release => "Release".into(),
            Lane::Controller(n) => match cc_name(n) {
                Some(name) => format!("{name} (CC {n})"),
                None => format!("CC {n}"),
            },
        }
    }

    fn note_kind(self) -> Option<LaneKind> {
        match self {
            Lane::Velocity => Some(LaneKind::Velocity),
            Lane::Probability => Some(LaneKind::Probability),
            _ => None,
        }
    }

    /// The lane a MIDI event belongs to.
    pub fn of(kind: &MidiKind) -> Lane {
        match kind {
            MidiKind::Controller { number, .. } => Lane::Controller(*number),
            MidiKind::PitchBend(_) => Lane::PitchBend,
            MidiKind::ChannelPressure(_) => Lane::ChannelPressure,
        }
    }

    fn max(self) -> i64 {
        if self == Lane::PitchBend { 16383 } else { 127 }
    }

    /// An event's value, if it belongs to this lane.
    fn value(self, kind: &MidiKind) -> Option<i64> {
        match (self, kind) {
            (Lane::Controller(n), MidiKind::Controller { number, value }) if n == *number => Some(*value as i64),
            (Lane::PitchBend, MidiKind::PitchBend(v)) => Some(*v as i64),
            (Lane::ChannelPressure, MidiKind::ChannelPressure(v)) => Some(*v as i64),
            _ => None,
        }
    }

    /// The lane a saved lane (`MidiAutomator`) is, if we know its kind.
    pub fn from_ref(r: LaneRef) -> Option<Lane> {
        match r.kind {
            2 => Some(Lane::Velocity),
            8 => Some(Lane::Probability),
            4 => Some(Lane::Controller(r.number.clamp(0, 127) as u8)),
            3 => Some(Lane::PitchBend),
            6 => Some(Lane::ChannelPressure),
            _ => None,
        }
    }

    pub fn to_ref(self) -> Option<LaneRef> {
        match self {
            Lane::Controller(n) => Some(LaneRef { kind: 4, number: n as i64 }),
            Lane::PitchBend => Some(LaneRef { kind: 3, number: -1 }),
            Lane::ChannelPressure => Some(LaneRef { kind: 6, number: -1 }),
            _ => None,
        }
    }

    /// An event of this lane with value `v`.
    fn with(self, v: i64) -> Option<MidiKind> {
        match self {
            Lane::Controller(number) => Some(MidiKind::Controller { number, value: v.clamp(0, 127) as u8 }),
            Lane::PitchBend => Some(MidiKind::PitchBend(v.clamp(0, 16383) as u16)),
            Lane::ChannelPressure => Some(MidiKind::ChannelPressure(v.clamp(0, 127) as u8)),
            _ => None,
        }
    }
}

/// The Expression editor's lanes, in the order listed.
const EXPR_LANES: [Lane; 4] =
    [Lane::Expr(deckard_model::Dim::Pitch), Lane::Expr(deckard_model::Dim::Slide), Lane::Expr(deckard_model::Dim::Pressure), Lane::Release];

/// [`Act::EraseSpan`]: lane `l`'s events from `from` to `to` (not including it) go, or
/// for an expression lane its curves' points there and the release velocity of the
/// notes starting there; one undo step.
fn erase_span(app: &mut App, l: Lane, from: i64, to: i64) {
    let Some(doc) = app.doc.clone() else { return };
    let mut d = doc.borrow_mut();
    let (g, pi, s) = (app.group, app.pattern, app.sound);
    let r = match expr_lane::dim_of(l) {
        None => {
            let remove: Vec<MidiEvent> = d.project().groups.get(g).and_then(|x| x.patterns.get(pi)).map_or(Vec::new(), |pt| {
                pt.midi.iter().filter(|m| m.sound == s && Lane::of(&m.kind) == l && (from..to).contains(&m.position)).copied().collect()
            });
            if remove.is_empty() { Ok(()) } else { d.edit_midi("Erase MIDI", g, pi, &remove, &[]) }
        }
        Some(dim) => {
            let edits: Vec<(Note, deckard_model::Expression)> = {
                let p = d.project();
                let notes = p.groups.get(g).and_then(|x| x.patterns.get(pi)).map_or(&[][..], |pt| &pt.notes[..]);
                notes
                    .iter()
                    .filter(|n| n.sound == s && n.expr.is_some())
                    .filter_map(|n| {
                        let mut e = expr_lane::expression_of(p, n);
                        match dim {
                            Some(dim) => {
                                let c = e.curve_mut(dim).as_mut()?;
                                let before = c.points.len();
                                c.points.retain(|pt| !(from..to).contains(&(n.position + pt.at)));
                                if c.points.len() == before {
                                    return None;
                                }
                                if c.points.is_empty() {
                                    *e.curve_mut(dim) = None;
                                }
                            }
                            None if e.release_velocity.is_some() && (from..to).contains(&n.position) => e.release_velocity = None,
                            None => return None,
                        }
                        Some((*n, e))
                    })
                    .collect()
            };
            if edits.is_empty() { Ok(()) } else { d.set_expressions(g, pi, &edits).map(|_| ()) }
        }
    };
    if let Err(e) = r {
        app.status = format!("can't erase: {e}");
    }
}

/// Whether expression `e` has what lane `l` shows.
fn has_lane(e: &deckard_model::Expression, l: Lane) -> bool {
    match expr_lane::dim_of(l) {
        Some(Some(d)) => e.curve(d).is_some(),
        Some(None) => e.release_velocity.is_some(),
        None => false,
    }
}

/// [`lane_list`] for the focused sound and pattern.
#[cfg(test)]
pub(crate) fn lane_list_of(app: &App) -> Vec<Lane> {
    let Some(doc) = &app.doc else { return Vec::new() };
    let d = doc.borrow();
    let p = d.project();
    let pat = p.groups.get(app.group).and_then(|g| g.patterns.get(app.pattern));
    let midi: Vec<MidiEvent> = pat.map_or(Vec::new(), |pt| pt.midi.iter().filter(|m| m.sound == app.sound).copied().collect());
    lane_list(app, p, &midi)
}

/// Names of the common MIDI controllers.
fn cc_name(n: u8) -> Option<&'static str> {
    Some(match n {
        1 => "Mod Wheel",
        2 => "Breath",
        4 => "Foot",
        5 => "Portamento Time",
        7 => "Volume",
        8 => "Balance",
        10 => "Pan",
        11 => "Expression",
        64 => "Sustain",
        65 => "Portamento",
        66 => "Sostenuto",
        67 => "Soft Pedal",
        71 => "Resonance",
        74 => "Cutoff",
        91 => "Reverb",
        93 => "Chorus",
        _ => return None,
    })
}

/// The lanes the list offers: velocity, probability, then the sound's saved MIDI
/// lanes in their order, and any others it has events in.
fn lane_list(app: &App, p: &Project, midi: &[MidiEvent]) -> Vec<Lane> {
    let saved = p.groups.get(app.group).and_then(|g| g.sounds.get(app.sound)).map_or(&[][..], |s| &s.lanes[..]);
    let mut out = vec![Lane::Velocity, Lane::Probability];
    // The Expression editor's lanes the sound's notes here have (BUGS #18).
    let pat = p.groups.get(app.group).and_then(|g| g.patterns.get(app.pattern));
    let exprs: Vec<deckard_model::Expression> = pat.map_or(Vec::new(), |pt| {
        pt.notes.iter().filter(|n| n.sound == app.sound && n.expr.is_some()).map(|n| expr_lane::expression_of(p, n)).collect()
    });
    out.extend(EXPR_LANES.into_iter().filter(|&l| exprs.iter().any(|e| has_lane(e, l))));
    let mut events: Vec<Lane> = midi.iter().map(|m| Lane::of(&m.kind)).collect();
    events.sort_unstable();
    for l in saved.iter().filter_map(|r| Lane::from_ref(*r)).chain(events).chain([app.lane]) {
        if !out.contains(&l) {
            out.push(l);
        }
    }
    out
}

/// Root notes as Maschine spells them for a scale (its mixed list).
const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];

/// MIDI note number to name, middle C (60) = C3 as in Maschine.
fn note_name(n: i64) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[n.rem_euclid(12) as usize], n.div_euclid(12) - 2)
}

#[cfg(test)]
mod lane_tests {
    use super::Lane;
    use deckard_model::MidiKind;

    /// Channel pressure is Maschine's Aftertouch lane (a `MidiAutomator` of mode 6).
    #[test]
    fn aftertouch_lane_maps_to_kind_6() {
        let l = Lane::of(&MidiKind::ChannelPressure(0));
        assert_eq!(l, Lane::ChannelPressure);
        assert_eq!(l.name(), "Aftertouch");
        assert_eq!(l.to_ref().map(|r| r.kind), Some(6));
        assert_eq!(l.to_ref().and_then(Lane::from_ref), Some(l));
        assert_eq!(l.with(200), Some(MidiKind::ChannelPressure(127)));
        assert_eq!(l.value(&MidiKind::ChannelPressure(40)), Some(40));
    }
}

#[cfg(test)]
mod emoji_tests {
    /// Names with emoji draw in the app's text: the full emoji font (which has the
    /// newer emoji egui's own lacks; see deckard-nwl's test) comes right after the text
    /// font, before egui's.
    #[test]
    fn the_apps_text_falls_back_to_the_emoji_font() {
        use eframe::egui::FontFamily;
        let f = super::font_definitions();
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            let list = &f.families[&family];
            assert_eq!(list[1], super::EMOJI_FONT, "{family:?}: {list:?}");
        }
        assert_eq!(&f.font_data[super::EMOJI_FONT].font[..], deckard_nwl::draw::EMOJI_FONT);
    }
}

#[cfg(test)]
mod group_tab_tests {
    /// A group's tab shows its bank with its letter, as Maschine's (A1–H1, then A2…),
    /// however many groups there are (BUGS.md #26).
    #[test]
    fn group_tabs_show_the_bank() {
        assert_eq!(super::group_tab(0), "A1");
        assert_eq!(super::group_tab(2), "C1");
        assert_eq!(super::group_tab(9), "B2");
    }
}
