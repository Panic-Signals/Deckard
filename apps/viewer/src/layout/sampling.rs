//! The Sampling view (docs/sampling-layout.md): RECORD, EDIT, SLICE and ZONE tabs over
//! the focused sound's Sampler, in place of the pattern editor. A stereo (or mixed)
//! waveform with the zone's play range (S/E markers), loop frame, selection, slices
//! and the playhead; zoom with the wheel or the bar under it, scroll with the bar.

use super::*;
use deckard_engine::sample::Sample;
use std::sync::Arc;

pub const TABS: [&str; 4] = ["RECORD", "EDIT", "SLICE", "ZONE"];

/// The fewest frames the waveform zooms in to.
const MIN_VIEW: f64 = 64.0;
const EDIT_BUTTONS: [(&str, EditOp); 14] = [
    ("TRUNCATE", EditOp::Truncate),
    ("NORM.", EditOp::Normalize),
    ("REVERSE", EditOp::Reverse),
    ("FADE IN", EditOp::FadeIn),
    ("FADE OUT", EditOp::FadeOut),
    ("CORRECT", EditOp::Dc),
    ("SILENCE", EditOp::Silence),
    ("CUT", EditOp::Cut),
    ("COPY", EditOp::Copy),
    ("PASTE", EditOp::Paste),
    ("DUPL.", EditOp::Duplicate),
    ("STRETCH", EditOp::Stretch),
    ("STEMS", EditOp::Stems),
    ("AUTO LOOP", EditOp::AutoLoop),
];
pub const SLICE_MODES: [&str; 4] = ["Split", "Grid", "Detect", "Manual"];
pub const SPLIT_COUNTS: [usize; 4] = [4, 8, 16, 32];
pub const GRID_DIVS: [(u32, &str); 4] = [(4, "1/4"), (8, "1/8"), (16, "1/16"), (32, "1/32")];
pub const PATTERN_MODES: [&str; 3] = ["Create Pattern", "Replace Pattern", "No Pattern"];

/// An edit of the selection (or, without one, the play range).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOp {
    Truncate,
    Normalize,
    Reverse,
    FadeIn,
    FadeOut,
    Dc,
    Silence,
    Cut,
    Copy,
    Paste,
    Duplicate,
    Stretch,
    /// Find a loop in the play range that joins smoothly, and switch it on.
    AutoLoop,
    /// Separate the sample into drums, bass, other and vocals, into a new group (or stop
    /// the separation running).
    Stems,
}

/// Time-stretch settings (Edit ▸ STRETCH).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stretch {
    /// Beat (tempo to tempo) or Free (speed).
    pub beat: bool,
    pub tune: f64,
    pub src_bpm: f64,
    pub new_bpm: f64,
    /// Percent.
    pub speed: f64,
}

impl Stretch {
    /// New length / old.
    pub fn factor(&self) -> f64 {
        if self.beat { self.src_bpm / self.new_bpm.max(1.0) } else { 100.0 / self.speed.max(10.0) }
    }
}

/// The view's state, kept between frames.
pub struct SamplingState {
    pub tab: usize,
    /// Two channels (else mixed).
    pub stereo: bool,
    /// Visible frames [start, end), of the sample shown (`None`: all of it).
    pub view: Option<(f64, f64)>,
    pub selection: Option<(usize, usize)>,
    pub clipboard: Option<Sample>,
    /// Slice starts (frames), sorted.
    pub slices: Vec<usize>,
    pub slice_mode: usize,
    pub split: usize,
    pub grid: usize,
    pub sensitivity: f64,
    pub zero_x: bool,
    /// The SLICE tool removes instead of adding.
    pub remove_tool: bool,
    pub pattern_mode: usize,
    /// Stretch settings while the stretch bar is open.
    pub stretch: Option<Stretch>,
    /// Zone tab: the map (else the focused zone's sample) and the zone list.
    pub zone_map: bool,
    pub zone_list: bool,
    /// The focused zone's sample and its peaks.
    pub sample: Option<Arc<Sample>>,
    pub sample_key: Option<(usize, usize, usize, String)>,
    peaks: Vec<[(f32, f32); 2]>,
    /// Where the focused zone plays (frames).
    pub playhead: Option<usize>,
}

impl Default for SamplingState {
    fn default() -> SamplingState {
        SamplingState {
            tab: 1,
            stereo: true,
            view: None,
            selection: None,
            clipboard: None,
            slices: Vec::new(),
            slice_mode: 0,
            split: 1,
            grid: 2,
            sensitivity: 0.5,
            zero_x: true,
            remove_tool: false,
            pattern_mode: 2,
            stretch: None,
            zone_map: true,
            zone_list: true,
            sample: None,
            sample_key: None,
            peaks: Vec::new(),
            playhead: None,
        }
    }
}

/// Frames per peak block.
const BLOCK: usize = 256;

impl SamplingState {
    /// Show `sample` (for `key`), computing its peak blocks.
    pub fn set_sample(&mut self, key: (usize, usize, usize, String), sample: Arc<Sample>) {
        let changed = self.sample_key.as_ref().is_none_or(|k| (k.0, k.1, k.2) != (key.0, key.1, key.2));
        self.peaks = sample
            .frames
            .chunks(BLOCK)
            .map(|c| {
                let mut p = [(f32::MAX, f32::MIN); 2];
                for f in c {
                    for ch in 0..2 {
                        p[ch] = (p[ch].0.min(f[ch]), p[ch].1.max(f[ch]));
                    }
                }
                p
            })
            .collect();
        if changed {
            self.view = None;
            self.selection = None;
            self.slices.clear();
        }
        self.sample = Some(sample);
        self.sample_key = Some(key);
    }

    pub fn len(&self) -> usize {
        self.sample.as_ref().map_or(0, |s| s.frames.len())
    }

    pub(crate) fn view(&self) -> (f64, f64) {
        let len = self.len().max(1) as f64;
        self.view.map_or((0.0, len), |(a, b)| (a.max(0.0), b.min(len).max(a + 16.0)))
    }

    /// (min, max) per channel over frames [a, b).
    pub(crate) fn column(&self, a: usize, b: usize) -> [(f32, f32); 2] {
        let Some(s) = &self.sample else { return [(0.0, 0.0); 2] };
        let b = b.min(s.frames.len());
        if a >= b {
            return [(0.0, 0.0); 2];
        }
        let mut p = [(f32::MAX, f32::MIN); 2];
        if b - a >= BLOCK * 2 {
            for blk in &self.peaks[a / BLOCK..b.div_ceil(BLOCK).min(self.peaks.len())] {
                for ch in 0..2 {
                    p[ch] = (p[ch].0.min(blk[ch].0), p[ch].1.max(blk[ch].1));
                }
            }
        } else {
            for f in &s.frames[a..b] {
                for ch in 0..2 {
                    p[ch] = (p[ch].0.min(f[ch]), p[ch].1.max(f[ch]));
                }
            }
        }
        p
    }
}

/// What the Sampling view asks for.
#[derive(Debug, Clone)]
pub enum SamplingAct {
    Open(bool),
    Tab(usize),
    Stereo,
    View(Option<(f64, f64)>),
    Select(Option<(usize, usize)>),
    Edit(EditOp),
    StretchOpen(bool),
    SetStretch(Stretch),
    StretchApply,
    SliceMode(usize),
    SplitCount(usize),
    GridDiv(usize),
    Sensitivity(f64),
    ZeroX(bool),
    RemoveTool(bool),
    SliceAt(usize),
    RemoveSliceAt(usize),
    MoveSlice(usize, usize),
    ClearSlices,
    ApplySlices,
    PatternMode(usize),
    FocusZone(usize),
    AddZones,
    RemoveZone(usize),
    MapAsDrumKit,
    ZoneMap(bool),
    ZoneList(bool),
    Record(RecordAct),
}

/// Recording controls.
#[derive(Debug, Clone, PartialEq)]
pub enum RecordAct {
    Start,
    Stop,
    Cancel,
    Set(RecParam, i64),
    Threshold(f64),
    /// Auto ▸ MIDI To: a MIDI output (by name), or none.
    MidiTo(Option<String>),
    /// Look up the MIDI outputs again (the MIDI To list opened).
    ScanMidi,
    FocusTake(usize),
    DeleteTake(usize),
}

fn sact(a: SamplingAct) -> Act {
    Act::Sampling(a)
}

pub(crate) fn frames_text(f: usize, rate: u32) -> String {
    let s = f as f64 / rate.max(1) as f64;
    if s < 60.0 { format!("{s:.3} s") } else { format!("{}:{:06.3}", (s / 60.0) as u64, s % 60.0) }
}

/// A recorded take being dragged from RECORD's list (its file), to drop on a sound.
pub struct DraggedTake(pub std::path::PathBuf);

/// What the Sampling view works on in sound (`g`, `s`): its Sampler's zone `zi`, or, for
/// an Audio plug-in, its take `zi` as a zone (its sample and play range, no loop); each
/// with the index (kept in range), where its fields are set, and whether it's a take.
pub fn edited(p: &Project, (g, s, zi): (usize, usize, usize)) -> Option<(usize, deckard_model::Zone, Target, bool)> {
    let snd = p.groups.get(g)?.sounds.get(s)?;
    if let Some(a) = snd.audio.as_ref().filter(|_| snd.instrument == "AudioModule") {
        let k = zi.min(a.takes.len().checked_sub(1)?);
        let t = &a.takes[k];
        let z = deckard_model::Zone { sample: t.sample.clone(), play_range: t.play_range, ..Default::default() };
        return Some((k, z, Target::Take(g, s, k), true));
    }
    let smp = snd.sampler.as_ref()?;
    let k = zi.min(smp.zones.len().checked_sub(1)?);
    Some((k, smp.zones[k].clone(), Target::Zone(g, s, k), false))
}

/// The Sampling view in `rect` (the pattern editor's place).
pub fn panel(ui: &mut Ui, app: &App, p: &Project, rect: Rect, acts: &mut Vec<Act>) {
    let st = &app.sampling;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::BLACK);
    let gcol = sound_color(p, app.group, app.sound);
    let edited = edited(p, (app.group, app.sound, app.zone));
    let (zi, zone, zt, take) = match &edited {
        Some((zi, z, t, take)) => (*zi, Some(z), *t, *take),
        None => (0, None, Target::Zone(app.group, app.sound, 0), false),
    };
    // An Audio plug-in's take: RECORD and EDIT only.
    let tab = if take { st.tab.min(1) } else { st.tab };

    // Header: the tabs (only RECORD until the sound has a sample).
    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 37.0));
    painter.rect_filled(header, 0.0, S00);
    let mut x = header.left() + 12.0;
    for (i, name) in TABS.iter().enumerate() {
        if i > 0 && zone.is_none() || i > 1 && take {
            break;
        }
        let w = 14.0 + name.len() as f32 * 7.5;
        let r = Rect::from_min_size(Pos2::new(x, header.top() + 8.0), Vec2::new(w, 22.0));
        let resp = ui.interact(r, ui.id().with(("samp-tab", i)), Sense::click());
        let on = tab == i;
        painter.text(
            r.center(),
            Align2::CENTER_CENTER,
            *name,
            font(12.0),
            if on {
                white(245)
            } else if resp.hovered() {
                white(164)
            } else {
                white(142)
            },
        );
        if on {
            painter.hline(r.shrink2(Vec2::new(4.0, 0.0)).x_range(), r.bottom(), Stroke::new(2.0, Color32::WHITE));
        }
        if resp.clicked() {
            acts.push(sact(SamplingAct::Tab(i)));
        }
        x += w + 10.0;
    }
    if tab == 3 && zone.is_some() {
        for (k, (label, on, act)) in
            [("LIST", st.zone_list, SamplingAct::ZoneList(!st.zone_list)), ("SAMPLE", !st.zone_map, SamplingAct::ZoneMap(!st.zone_map))]
                .into_iter()
                .enumerate()
        {
            let r = Rect::from_min_size(Pos2::new(header.right() - 140.0 + k as f32 * 66.0, header.top() + 9.0), Vec2::new(60.0, 20.0));
            let resp = ui.interact(r, ui.id().with(("zone-hdr", k)), Sense::click());
            painter.rect_filled(
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
            painter.text(r.center(), Align2::CENTER_CENTER, label, font(10.0), if on { white(245) } else { white(142) });
            if resp.clicked() {
                acts.push(sact(act));
            }
        }
    }
    // Info bar: play, name, duration, stereo/mono.
    let info = Rect::from_min_size(Pos2::new(rect.left(), header.bottom() + 1.0), Vec2::new(rect.width(), 16.0));
    painter.rect_filled(info, 0.0, S00);
    if let (Some(z), Some(s)) = (zone, &st.sample) {
        let play = Rect::from_min_size(Pos2::new(info.left() + 4.0, info.top()), Vec2::splat(16.0));
        let resp = ui.interact(play, ui.id().with("samp-play"), Sense::click_and_drag()).on_hover_text("Hold to play the sample");
        painter.rect_filled(play, 2.0, gcol);
        triangle(&painter, play.center(), 4.0, Color32::BLACK);
        if resp.is_pointer_button_down_on() && !app.held.is_some_and(|h| h == (app.group, app.sound, z.root_key)) {
            acts.push(Act::HoldPitch(app.group, app.sound, z.root_key));
        }
        let name = z.sample.path.rsplit(['/', '\\']).next().unwrap_or("");
        let t = painter.text(Pos2::new(play.right() + 6.0, info.center().y), Align2::LEFT_CENTER, name, font(12.0), gcol);
        painter.text(
            Pos2::new(t.right() + 12.0, info.center().y),
            Align2::LEFT_CENTER,
            frames_text(s.frames.len(), s.rate),
            font(12.0),
            white(142),
        );
        let tog = Rect::from_min_size(Pos2::new(info.right() - 60.0, info.top()), Vec2::new(50.0, 16.0));
        let resp = ui.interact(tog, ui.id().with("samp-stereo"), Sense::click()).on_hover_text("Show the two channels, or their mix");
        painter.text(
            tog.center(),
            Align2::CENTER_CENTER,
            if st.stereo { "STEREO" } else { "MONO" },
            font(10.0),
            if resp.hovered() { white(245) } else { white(142) },
        );
        if resp.clicked() {
            acts.push(sact(SamplingAct::Stereo));
        }
    } else {
        painter.text(
            Pos2::new(info.left() + 8.0, info.center().y),
            Align2::LEFT_CENTER,
            "No sample: record one, or load one into the sound's Sampler",
            font(11.0),
            white(142),
        );
    }
    let body = Rect::from_min_max(Pos2::new(rect.left(), info.bottom() + 1.0), rect.max);
    match tab {
        0 => record_tab(ui, app, p, body, gcol, acts),
        1 => edit_tab(ui, app, body, zi, zone, zt, take, gcol, acts),
        2 => slice_tab(ui, app, body, zi, zone, gcol, acts),
        _ => zone_tab(ui, app, p, body, zi, gcol, acts),
    }
}

/// Timeline, waveform and zoom bar in `area` (for the Edit, Slice and Zone ▸ Sample
/// views). Returns the waveform's rect and the frame→x mapping's view.
#[allow(clippy::too_many_arguments)]
fn wave(
    ui: &mut Ui,
    app: &App,
    area: Rect,
    zone: &deckard_model::Zone,
    zt: Target,
    gcol: Color32,
    slices: bool,
    acts: &mut Vec<Act>,
) -> Rect {
    let st = &app.sampling;
    let painter = ui.painter_at(area);
    let Some(sample) = &st.sample else { return area };
    let len = sample.frames.len().max(1);
    let timeline = Rect::from_min_size(area.min, Vec2::new(area.width(), 14.0));
    let bar = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - 16.0), area.max);
    let w = Rect::from_min_max(Pos2::new(area.left(), timeline.bottom() + 1.0), Pos2::new(area.right(), bar.top() - 1.0));
    ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-sample-wave"), w));
    let (v0, v1) = st.view();
    let fx = |f: f64| w.left() + ((f - v0) / (v1 - v0)) as f32 * w.width();
    let xf = |x: f32| (v0 + ((x - w.left()) / w.width()) as f64 * (v1 - v0)).clamp(0.0, len as f64);
    painter.rect_filled(timeline, 0.0, S00);
    painter.rect_filled(w, 0.0, S00);
    // Timeline: seconds.
    let secs = (v1 - v0) / sample.rate.max(1) as f64;
    let step =
        [0.001, 0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0].into_iter().find(|s| secs / s <= 10.0).unwrap_or(60.0);
    let mut t = (v0 / sample.rate as f64 / step).ceil() * step;
    while t * (sample.rate as f64) < v1 {
        let x = fx(t * sample.rate as f64);
        painter.vline(x, (timeline.bottom() - 8.0)..=timeline.bottom(), Stroke::new(1.0, white(92)));
        painter.text(
            Pos2::new(x + 3.0, timeline.top() + 1.0),
            Align2::LEFT_TOP,
            format!("{t:.3}").trim_end_matches('0').trim_end_matches('.').to_string(),
            font(9.0),
            white(92),
        );
        t += step;
    }
    // Play range, loop, selection.
    let (lo, hi) =
        if zone.play_range.1 > zone.play_range.0 { (zone.play_range.0 as f64, zone.play_range.1 as f64 + 1.0) } else { (0.0, len as f64) };
    if let Some((a, b)) = st.selection {
        painter.rect_filled(Rect::from_x_y_ranges(fx(a as f64)..=fx(b as f64), w.y_range()), 0.0, S02);
    }
    let looped = zone.loop_active && zone.loop_range.1 > zone.loop_range.0;
    let (la, lb) = (zone.loop_range.0 as f64, zone.loop_range.1 as f64 + 1.0);
    if looped {
        painter.rect_filled(Rect::from_x_y_ranges(fx(la)..=fx(lb), w.y_range()), 0.0, gcol.gamma_multiply(0.18));
    }
    // The waveform: per column, (min, max) per channel.
    let chans: Vec<Rect> = if st.stereo {
        let h = (w.height() - 1.0) / 2.0;
        vec![
            Rect::from_min_size(w.min, Vec2::new(w.width(), h)),
            Rect::from_min_size(Pos2::new(w.left(), w.top() + h + 1.0), Vec2::new(w.width(), h)),
        ]
    } else {
        vec![w]
    };
    let cols = w.width().max(1.0) as usize;
    let colour = |f: usize| {
        let inside = (f as f64) >= lo && (f as f64) < hi;
        if inside || st.selection.is_some_and(|(s0, s1)| f >= s0 && f < s1) { gcol } else { S03 }
    };
    let level = |p: [(f32, f32); 2], k: usize| if st.stereo { p[k] } else { ((p[0].0 + p[1].0) * 0.5, (p[0].1 + p[1].1) * 0.5) };
    let y_of = |cr: &Rect, v: f32| cr.center().y - v.clamp(-1.0, 1.0) * cr.height() * 0.48;
    let mut outlines: Vec<Vec<crate::sampler_ui::WaveCol>> = vec![Vec::with_capacity(cols); chans.len()];
    // Up to a few frames per pixel the signal's shape shows: draw it as a line through
    // every frame (curved between them when zoomed in). Further out, its envelope.
    let as_line = (v1 - v0) <= cols as f64 * 8.0;
    if as_line {
        let first = (v0.floor() as usize).saturating_sub(2);
        let last = ((v1.ceil() as usize) + 2).min(len - 1);
        for f in first..=last {
            let fr = sample.frames[f];
            let p = [(fr[0], fr[0]), (fr[1], fr[1])];
            for (k, cr) in chans.iter().enumerate() {
                let y = y_of(cr, level(p, k).0);
                outlines[k].push(crate::sampler_ui::WaveCol { x: fx(f as f64), top: y, bottom: y, color: colour(f) });
            }
        }
    } else {
        for c in 0..cols {
            let a = xf(w.left() + c as f32) as usize;
            let b = (xf(w.left() + c as f32 + 1.0) as usize).max(a + 1);
            if a >= len {
                break;
            }
            // One frame more, so neighbouring columns overlap and the shape stays joined.
            let p = st.column(a, b + 1);
            for (k, cr) in chans.iter().enumerate() {
                let (mn, mx) = level(p, k);
                outlines[k].push(crate::sampler_ui::WaveCol {
                    x: w.left() + c as f32 + 0.5,
                    top: y_of(cr, mx),
                    bottom: y_of(cr, mn),
                    color: colour(a),
                });
            }
        }
    }
    for (k, cr) in chans.iter().enumerate() {
        let p = painter.with_clip_rect(*cr);
        if as_line {
            crate::sampler_ui::paint_signal(&p, &outlines[k], 1.2);
        } else {
            crate::sampler_ui::paint_wave(&p, &outlines[k]);
        }
    }
    for cr in &chans {
        painter.hline(w.x_range(), cr.center().y, Stroke::new(1.0, Color32::from_rgb(30, 30, 30)));
    }
    // Slice markers.
    if slices {
        for (i, &s) in st.slices.iter().enumerate() {
            let x = fx(s as f64);
            painter.vline(x, w.y_range(), Stroke::new(1.0, white(164)));
            painter.text(Pos2::new(x + 3.0, w.top() + 2.0), Align2::LEFT_TOP, (i + 1).to_string(), font(9.0), white(142));
        }
    }
    // Loop frame and S/E markers.
    if looped {
        let r = Rect::from_x_y_ranges(fx(la)..=fx(lb), w.y_range());
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
        let tab = Rect::from_min_size(r.min, Vec2::new(r.width(), 12.0));
        painter.rect_filled(tab, 0.0, Color32::WHITE);
        painter.text(tab.left_center() + Vec2::new(4.0, 0.0), Align2::LEFT_CENTER, "LOOP", font(9.0), Color32::BLACK);
    }
    // The S and E flags at the top of their lines (grabbed with them).
    let flag_s = Rect::from_min_size(Pos2::new(fx(lo), w.top()), Vec2::splat(14.0));
    let flag_e = Rect::from_min_size(Pos2::new(fx(hi) - 14.0, w.top()), Vec2::splat(14.0));
    for (f, label, flag) in [(lo, "S", flag_s), (hi, "E", flag_e)] {
        let x = fx(f);
        painter.vline(x, w.y_range(), Stroke::new(3.0, Color32::WHITE));
        painter.rect_filled(flag, 0.0, Color32::WHITE);
        painter.text(flag.center(), Align2::CENTER_CENTER, label, font(10.0), Color32::BLACK);
    }
    // Playhead.
    if let Some(ph) = st.playhead {
        painter.vline(fx(ph as f64), w.y_range(), Stroke::new(1.0, Color32::from_white_alpha(198)));
    }
    // Interaction: drag a marker (S/E, loop edges or body, a slice), else select (or,
    // on the Slice tab, add/remove slices).
    let resp = ui.interact(w, ui.id().with("sample-wave"), Sense::click_and_drag());
    let drag_id = ui.id().with("sample-wave-drag");
    #[derive(Clone, Copy)]
    enum Grab {
        Start,
        End,
        LoopStart,
        LoopEnd,
        Loop(f64),
        Slice(usize),
        Select(f64),
    }
    // What's under the pointer to drag: the S/E flags or lines, the loop's edges or its
    // tab, a slice; elsewhere a selection.
    let grab_at = |pos: Pos2| {
        let near = |f: f64| (fx(f) - pos.x).abs() < 6.0;
        if flag_s.expand(1.0).contains(pos) {
            Grab::Start
        } else if flag_e.expand(1.0).contains(pos) {
            Grab::End
        } else if near(lo) {
            Grab::Start
        } else if near(hi) {
            Grab::End
        } else if looped && near(la) {
            Grab::LoopStart
        } else if looped && near(lb) {
            Grab::LoopEnd
        } else if looped && pos.y < w.top() + 12.0 && pos.x > fx(la) && pos.x < fx(lb) {
            Grab::Loop(xf(pos.x) - la)
        } else if slices && let Some(i) = st.slices.iter().position(|&s| (fx(s as f64) - pos.x).abs() < 4.0).filter(|&i| i > 0) {
            Grab::Slice(i)
        } else {
            Grab::Select(xf(pos.x))
        }
    };
    let cursor = |g: Grab| match g {
        Grab::Start | Grab::End | Grab::LoopStart | Grab::LoopEnd | Grab::Slice(_) => Some(egui::CursorIcon::ResizeHorizontal),
        Grab::Loop(_) => Some(egui::CursorIcon::Grab),
        Grab::Select(_) => None,
    };
    if resp.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
    {
        ui.data_mut(|d| d.insert_temp(drag_id, Some(grab_at(pos))));
    }
    // The pointer shows what can be dragged (and keeps showing it while dragging).
    let held = if resp.dragged() { ui.data(|d| d.get_temp::<Option<Grab>>(drag_id)).flatten() } else { None };
    let shown = match (held, resp.hover_pos()) {
        (Some(Grab::Loop(_)), _) => Some(egui::CursorIcon::Grabbing),
        (Some(g), _) => cursor(g),
        (None, Some(pos)) => cursor(grab_at(pos)),
        _ => None,
    };
    if let Some(c) = shown {
        ui.ctx().set_cursor_icon(c);
    }
    if resp.dragged()
        && let (Some(Some(grab)), Some(pos)) = (ui.data(|d| d.get_temp::<Option<Grab>>(drag_id)), resp.interact_pointer_pos())
    {
        let f = xf(pos.x).round();
        let last = len as f64 - 1.0;
        match grab {
            Grab::Start => acts.push(Act::SetParam(zt, "play_range/low", f.min(hi - 2.0).max(0.0))),
            Grab::End => acts.push(Act::SetParam(zt, "play_range/high", (f - 1.0).max(lo + 1.0).min(last))),
            Grab::LoopStart => acts.push(Act::SetParam(zt, "loop_range/low", f.clamp(lo, lb - 2.0))),
            Grab::LoopEnd => acts.push(Act::SetParam(zt, "loop_range/high", (f - 1.0).clamp(la + 1.0, hi - 1.0))),
            Grab::Loop(off) => {
                let n = lb - la;
                let a = (f - off).clamp(lo, hi - n);
                acts.push(Act::SetParam(zt, "loop_range/low", a));
                acts.push(Act::SetParam(zt, "loop_range/high", a + n - 1.0));
            }
            Grab::Slice(i) => acts.push(sact(SamplingAct::MoveSlice(i, f as usize))),
            Grab::Select(from) => {
                let (a, b) = (from.min(f) as usize, from.max(f) as usize);
                acts.push(sact(SamplingAct::Select((b > a).then_some((a, b)))));
            }
        }
    }
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let f = xf(pos.x) as usize;
        if slices {
            acts.push(sact(if st.remove_tool { SamplingAct::RemoveSliceAt(f) } else { SamplingAct::SliceAt(f) }));
        } else {
            acts.push(sact(SamplingAct::Select(None)));
        }
    }
    if resp.double_clicked() && !slices {
        acts.push(sact(SamplingAct::Select(Some((lo as usize, hi as usize)))));
    }
    // Zoom with the wheel (around the pointer), scroll with a horizontal swipe.
    if resp.hovered() {
        let (zoom, pan) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
        let mut z = zoom;
        if pan.y.abs() > pan.x.abs() {
            z *= (pan.y * 0.01).exp();
        }
        if (z - 1.0).abs() > 1e-4 || pan.x.abs() > 0.5 {
            let at = ui.input(|i| i.pointer.hover_pos()).map_or(w.center().x, |p| p.x);
            let pivot = xf(at);
            let span = ((v1 - v0) / z as f64).clamp(MIN_VIEW, len as f64);
            let mut a = pivot - (pivot - v0) * span / (v1 - v0);
            a -= pan.x as f64 * span / w.width() as f64;
            let a = a.clamp(0.0, len as f64 - span);
            acts.push(sact(SamplingAct::View(Some((a, a + span)))));
        }
    }
    // The Pattern Editor's zooming scroll bar over the visible part of the sample: along
    // scrolls, up and down zoom round where it was grabbed, its ends zoom with the other
    // end held, double-click shows it all.
    let total = len as f64;
    let shown = ((v0 / total) as f32, ((v1 - v0) / total) as f32);
    if let Some((from, part)) =
        zoom_bar(ui, &painter, ui.id().with("sample-zoombar"), bar, true, 1.0, shown, (MIN_VIEW / total) as f32, true)
    {
        let view = (part < 1.0).then(|| {
            let a = from as f64 * total;
            (a, a + part as f64 * total)
        });
        acts.push(sact(SamplingAct::View(view)));
    }
    w
}

/// A footer group: a box with its title and (label, value, drag) fields; returns the
/// next x.
fn group_box(ui: &Ui, painter: &Painter, x: f32, y: f32, title: &str, fields: &[Field], acts: &mut Vec<Act>) -> f32 {
    // Each field as wide as its value (up to a limit; longer values are cut with "…").
    let widths: Vec<f32> = fields
        .iter()
        .map(|f| {
            let w = |t: &str, size: f32| painter.layout_no_wrap(t.to_string(), font(size), Color32::WHITE).size().x;
            let arrow = if f.choice.is_some() { 12.0 } else { 0.0 };
            (w(&f.value, 12.0).max(w(f.label, 9.0)) + 8.0 + arrow).clamp(70.0, FIELD_MAX_W)
        })
        .collect();
    let inner: f32 = widths.iter().map(|w| w + 4.0).sum();
    let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new((inner + 4.0).max(80.0), 54.0));
    painter.rect_filled(r, 2.0, S01);
    painter.text(Pos2::new(r.left() + 6.0, r.top() + 9.0), Align2::LEFT_CENTER, title, font(10.0), white(142));
    let mut fx = r.left() + 4.0;
    for (i, f) in fields.iter().enumerate() {
        let fr = Rect::from_min_size(Pos2::new(fx, r.top() + 18.0), Vec2::new(widths[i], 34.0));
        fx += widths[i] + 4.0;
        painter.text(Pos2::new(fr.left() + 2.0, fr.top() + 6.0), Align2::LEFT_CENTER, f.label, font(9.0), white(110));
        let value = elide(painter, &f.value, 12.0, fr.width() - 4.0 - if f.choice.is_some() { 10.0 } else { 0.0 });
        let mut resp = ui.interact(fr, ui.id().with(("field", title, i)), Sense::click_and_drag());
        resp = if value != f.value { resp.on_hover_text(format!("{}\n{}", f.value, f.tip)) } else { resp.on_hover_text(f.tip) };
        ui.data_mut(|d| d.insert_temp(egui::Id::new(("deckard-sample-field", title.to_string(), f.label.to_string())), fr));
        if let Some(what) = f.typed {
            value_entry::gestures(ui, &resp, what, &f.value, acts);
        }
        if resp.hovered() || resp.dragged() {
            painter.rect_filled(fr, 2.0, white(8));
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        let hot = resp.hovered() || resp.dragged();
        painter.text(
            Pos2::new(fr.left() + 2.0, fr.top() + 22.0),
            Align2::LEFT_CENTER,
            &value,
            font(12.0),
            if hot { Color32::WHITE } else { white(220) },
        );
        if f.choice.is_some() {
            // A small down arrow: a click lists the choices.
            let c = Pos2::new(fr.right() - 6.0, fr.top() + 22.0);
            painter.add(egui::Shape::convex_polygon(
                vec![c + Vec2::new(-3.5, -2.0), c + Vec2::new(3.5, -2.0), c + Vec2::new(0.0, 2.5)],
                if hot { Color32::WHITE } else { white(142) },
                Stroke::NONE,
            ));
        }
        if resp.dragged() {
            let fine = ui.input(|i| i.modifiers.shift);
            let d = -resp.drag_delta().y / if fine { 10.0 } else { 1.0 };
            let id = ui.id().with(("field-acc", title, i));
            let acc = ui.data(|dd| dd.get_temp::<f32>(id)).unwrap_or(0.0) + d;
            let steps = (acc / 4.0).trunc();
            ui.data_mut(|dd| dd.insert_temp(id, acc - steps * 4.0));
            if steps != 0.0 {
                acts.push((f.step)(steps as f64));
            }
        }
        if let Some((options, cur, pick)) = &f.choice {
            if resp.clicked()
                && let Some(c) = &f.click
            {
                acts.push(c());
            }
            if let Some(k) = crate::layout::choice_list(&resp, options.iter().map(String::as_str), *cur)
                && k != *cur
            {
                acts.push(pick(k));
            }
        } else if resp.clicked()
            && let Some(c) = &f.click
        {
            acts.push(c());
        }
    }
    r.right() + 5.0
}

/// The widest a footer field gets.
const FIELD_MAX_W: f32 = 200.0;

/// `text` cut with "…" to fit `width` at `size`.
fn elide(painter: &Painter, text: &str, size: f32, width: f32) -> String {
    let fits = |t: &str| painter.layout_no_wrap(t.to_string(), font(size), Color32::WHITE).size().x <= width;
    if fits(text) {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let t: String = chars[..mid].iter().collect::<String>() + "…";
        if fits(&t) { lo = mid } else { hi = mid - 1 }
    }
    chars[..lo].iter().collect::<String>().trim_end().to_string() + "…"
}

/// A choice: its options, the current one, and what picking one does.
type Choice<'a> = (Vec<String>, usize, Box<dyn Fn(usize) -> Act + 'a>);

/// A footer field: label, value text, what `n` drag steps do, what a click does, and
/// for a choice its list (a click opens it).
struct Field<'a> {
    label: &'a str,
    value: String,
    tip: &'a str,
    step: Box<dyn Fn(f64) -> Act + 'a>,
    click: Option<Box<dyn Fn() -> Act + 'a>>,
    choice: Option<Choice<'a>>,
    /// A double-click types it (value_entry).
    typed: Option<value_entry::What>,
}

fn field<'a>(label: &'a str, value: String, step: impl Fn(f64) -> Act + 'a) -> Field<'a> {
    Field { label, value, tip: "Drag up or down (Shift: fine)", step: Box::new(step), click: None, choice: None, typed: None }
}

/// A position in the sample, in samples: dragged, or typed with a double-click. Shift
/// moves it a sample at a time.
fn frame_field<'a>(
    label: &'a str,
    at: i64,
    (lo, hi): (i64, i64),
    per_step: f64,
    what: value_entry::What,
    set: impl Fn(f64) -> Act + 'a,
) -> Field<'a> {
    let (lo, hi) = (lo as f64, (hi.max(lo)) as f64);
    let mut f = field(label, at.to_string(), move |n| set((at as f64 + n * per_step).clamp(lo, hi).round()));
    f.tip = "Drag up or down (Shift: sample by sample), or double-click to type";
    f.typed = Some(what);
    f
}

/// A zone's position in samples, set through its parameter `path`.
fn zone_frame<'a>(label: &'a str, t: Target, path: &'static str, at: i64, range: (i64, i64), default: i64, per_step: f64) -> Field<'a> {
    let what = value_entry::What::Edit(Edit::Int(t, path, range.0 as f64, range.1.max(range.0) as f64, default as f64));
    frame_field(label, at, range, per_step, what, move |v| Act::SetParam(t, path, v))
}

/// The Edit and Zone tabs' PLAY RANGE and LOOP fields, in samples.
fn play_fields<'a>(t: Target, z: &deckard_model::Zone, last: i64, per_step: f64) -> [Field<'a>; 2] {
    let (lo, hi) = shown_play(z, last);
    [
        zone_frame("START", t, "play_range/low", lo, (0, hi - 1), 0, per_step),
        zone_frame("END", t, "play_range/high", hi, (lo + 1, last), last, per_step),
    ]
}

fn loop_fields<'a>(t: Target, z: &deckard_model::Zone, last: i64, per_step: f64) -> [Field<'a>; 4] {
    let (la, lb) = z.loop_range;
    let on = z.loop_active;
    [
        toggle_field("ACTIVE", on, move || Act::SetParam(t, "loop_active", if on { 0.0 } else { 1.0 })),
        zone_frame("START", t, "loop_range/low", la, (0, lb - 1), 0, per_step),
        zone_frame("END", t, "loop_range/high", lb, (la + 1, last), last, per_step),
        zone_frame("XFADE", t, "x_fade", z.x_fade, (0, lb - la), 0, per_step),
    ]
}

/// A zone's play range, both ends in: all of it when it has none.
fn shown_play(z: &deckard_model::Zone, last: i64) -> (i64, i64) {
    if z.play_range.1 > z.play_range.0 { z.play_range } else { (0, last) }
}

/// What the Edit tab selects when a sample comes up: what was saved, else the play range
/// (half open, as the selection is). A saved one that no longer fits the sample is passed
/// over.
pub(crate) fn first_selection(z: &deckard_model::Zone, frames: usize) -> Option<(usize, usize)> {
    let last = frames.checked_sub(1)? as i64;
    let (a, b) = z.selection_range;
    let (a, b) = if 0 <= a && a < b && b <= last { (a, b) } else { shown_play(z, last) };
    let (a, b) = (a.clamp(0, last), b.clamp(0, last));
    (a < b).then_some((a as usize, b as usize + 1))
}

/// One of `options` (now `cur`): a click lists them, dragging steps through them.
fn choice_field<'a>(label: &'a str, options: Vec<String>, cur: usize, pick: impl Fn(usize) -> Act + 'a) -> Field<'a> {
    let pick = std::rc::Rc::new(pick);
    let (p, n) = (pick.clone(), options.len());
    Field {
        label,
        value: options.get(cur).cloned().unwrap_or_default(),
        tip: "Click for the list, or drag up and down",
        step: Box::new(move |d| p((cur as f64 + d).clamp(0.0, n.saturating_sub(1) as f64) as usize)),
        click: None,
        choice: Some((options, cur, Box::new(move |k| pick(k)))),
        typed: None,
    }
}

fn toggle_field<'a>(label: &'a str, on: bool, act: impl Fn() -> Act + 'a) -> Field<'a> {
    let a = std::rc::Rc::new(act);
    let b = a.clone();
    Field {
        label,
        value: if on { "On".into() } else { "Off".into() },
        tip: "Click to switch",
        step: Box::new(move |_| b()),
        click: Some(Box::new(move || a())),
        choice: None,
        typed: None,
    }
}

/// A toolbar button (icon area with its label); returns whether it was clicked.
fn tool_button(ui: &Ui, painter: &Painter, r: Rect, label: &str, enabled: bool, on: bool, id: &str) -> bool {
    ui.data_mut(|d| d.insert_temp(egui::Id::new(("deckard-tool", id.to_string())), r));
    let resp = ui.interact(r, ui.id().with(("tool", id)), Sense::click());
    let fill = if on {
        S03
    } else if resp.hovered() && enabled {
        S02
    } else {
        S01
    };
    painter.rect_filled(r, 3.0, fill);
    painter.text(
        r.center(),
        Align2::CENTER_CENTER,
        label,
        font(10.0),
        if !enabled {
            white(50)
        } else if resp.hovered() || on {
            white(245)
        } else {
            white(164)
        },
    );
    enabled && resp.clicked()
}

#[allow(clippy::too_many_arguments)]
/// The Edit tab: the focused zone's sample, or (`take`) an Audio plug-in's take, whose
/// fields are `zt`'s (a take has a play range but no loop).
#[allow(clippy::too_many_arguments)]
fn edit_tab(
    ui: &mut Ui,
    app: &App,
    body: Rect,
    zi: usize,
    zone: Option<&deckard_model::Zone>,
    zt: Target,
    take: bool,
    gcol: Color32,
    acts: &mut Vec<Act>,
) {
    let Some(zone) = zone else { return };
    let st = &app.sampling;
    let painter = ui.painter_at(body);
    let tools_h = 120.0;
    let area = Rect::from_min_max(body.min, Pos2::new(body.right(), body.bottom() - tools_h));
    wave(ui, app, area, zone, zt, gcol, false, acts);
    let tools = Rect::from_min_max(Pos2::new(body.left(), area.bottom() + 1.0), body.max);
    painter.rect_filled(tools, 0.0, S00);
    let has_sel = st.selection.is_some();
    let y = tools.top() + 6.0;
    if let Some(sx) = st.stretch {
        // The stretch bar: APPLY / CANCEL and the settings.
        let mut x = tools.left() + 12.0;
        if tool_button(
            ui,
            &painter,
            Rect::from_min_size(Pos2::new(x, y), Vec2::new(62.0, 40.0)),
            "APPLY",
            sx.new_bpm >= sx.src_bpm / 10.0,
            false,
            "stretch-apply",
        ) {
            acts.push(sact(SamplingAct::StretchApply));
        }
        x += 70.0;
        if tool_button(ui, &painter, Rect::from_min_size(Pos2::new(x, y), Vec2::new(62.0, 40.0)), "CANCEL", true, false, "stretch-cancel") {
            acts.push(sact(SamplingAct::StretchOpen(false)));
        }
        x += 80.0;
        let mut fields = vec![
            field("TUNE", format!("{:+.2}", sx.tune), move |n| {
                sact(SamplingAct::SetStretch(Stretch { tune: (sx.tune + n * 0.01).clamp(-24.0, 24.0), ..sx }))
            }),
            toggle_field("MODE BEAT", sx.beat, move || sact(SamplingAct::SetStretch(Stretch { beat: !sx.beat, ..sx }))),
        ];
        if sx.beat {
            fields.push(field("SRC BPM", format!("{:.2}", sx.src_bpm), move |n| {
                sact(SamplingAct::SetStretch(Stretch { src_bpm: (sx.src_bpm + n * 0.1).clamp(20.0, 400.0), ..sx }))
            }));
            fields.push(field("NEW BPM", format!("{:.2}", sx.new_bpm), move |n| {
                sact(SamplingAct::SetStretch(Stretch { new_bpm: (sx.new_bpm + n * 0.1).clamp(2.0, 400.0), ..sx }))
            }));
        } else {
            fields.push(field("SPEED", format!("{:.0} %", sx.speed), move |n| {
                sact(SamplingAct::SetStretch(Stretch { speed: (sx.speed + n).clamp(10.0, 1000.0), ..sx }))
            }));
        }
        group_box(ui, &painter, x, y - 2.0, "STRETCH", &fields, acts);
        return;
    }
    let mut x = tools.left() + 12.0;
    for (label, op) in EDIT_BUTTONS {
        // STEMS shows how far a separation of this sample is (a click stops it); one at
        // a time, so it's off for the others meanwhile.
        let stems = app.stems.as_ref().map(|j| (j.source == (app.group, app.sound, zi), j.progress(), j.phase()));
        let enabled = match op {
            EditOp::Paste => st.clipboard.is_some(),
            EditOp::Cut | EditOp::Copy | EditOp::Duplicate | EditOp::Silence => has_sel,
            EditOp::Stems => stems.is_none_or(|(here, ..)| here),
            _ => true,
        };
        let text = match (op, stems) {
            (EditOp::Stems, Some((true, _, crate::stems::Phase::Installing))) => "MODEL…".to_string(),
            (EditOp::Stems, Some((true, p, _))) => format!("{:.0} %", p * 100.0),
            _ => label.to_string(),
        };
        let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(58.0, 40.0));
        if tool_button(ui, &painter, r, &text, enabled, false, label) {
            acts.push(sact(if op == EditOp::Stretch { SamplingAct::StretchOpen(true) } else { SamplingAct::Edit(op) }));
        }
        x += 62.0;
        if matches!(op, EditOp::Silence | EditOp::Duplicate) {
            x += 8.0;
        }
    }
    // Footer: play range, selection and loop, always there, in samples.
    let last = st.len().max(1) as i64 - 1;
    let per_step = frame_step(ui, last);
    let fy = y + 48.0;
    let mut x = tools.left() + 12.0;
    x = group_box(ui, &painter, x, fy, "PLAY RANGE", &play_fields(zt, zone, last, per_step), acts);
    x = group_box(ui, &painter, x, fy, "SELECTION RANGE", &selection_fields(st.selection, shown_play(zone, last), last, per_step), acts);
    if !take {
        group_box(ui, &painter, x, fy, "LOOP", &loop_fields(zt, zone, last, per_step), acts);
    }
}

/// How many samples a drag step of a footer's position moves: Shift, one.
fn frame_step(ui: &Ui, last: i64) -> f64 {
    if ui.input(|i| i.modifiers.shift) { 1.0 } else { ((last + 1) as f64 / 2000.0).max(1.0) }
}

/// SELECTION RANGE, both ends in as the others read; with nothing selected "-", and moving
/// an end selects from the play range.
fn selection_fields<'a>(sel: Option<(usize, usize)>, play: (i64, i64), last: i64, per_step: f64) -> [Field<'a>; 2] {
    let (a, b) = sel.map_or(play, |(a, b)| (a as i64, b as i64 - 1));
    let select = |a: i64, b: i64| sact(SamplingAct::Select(Some((a.max(0) as usize, (b + 1).max(a + 1) as usize))));
    let start =
        frame_field("START", a, (0, b - 1), per_step, value_entry::What::Selection(false, a, b, last), move |v| select(v as i64, b));
    let end = frame_field("END", b, (a + 1, last), per_step, value_entry::What::Selection(true, a, b, last), move |v| select(a, v as i64));
    let shown = |mut f: Field<'a>| {
        if sel.is_none() {
            f.value = "-".into();
        }
        f
    };
    [shown(start), shown(end)]
}

#[allow(clippy::too_many_arguments)]
fn slice_tab(ui: &mut Ui, app: &App, body: Rect, zi: usize, zone: Option<&deckard_model::Zone>, gcol: Color32, acts: &mut Vec<Act>) {
    let Some(zone) = zone else { return };
    let st = &app.sampling;
    let painter = ui.painter_at(body);
    let area = Rect::from_min_max(body.min, Pos2::new(body.right(), body.bottom() - 120.0));
    wave(ui, app, area, zone, Target::Zone(app.group, app.sound, zi), gcol, true, acts);
    let tools = Rect::from_min_max(Pos2::new(body.left(), area.bottom() + 1.0), body.max);
    painter.rect_filled(tools, 0.0, S00);
    let y = tools.top() + 6.0;
    let mut x = tools.left() + 12.0;
    let manual = st.slice_mode == 3;
    if tool_button(
        ui,
        &painter,
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(58.0, 40.0)),
        "SLICE",
        manual,
        manual && !st.remove_tool,
        "slice",
    ) {
        acts.push(sact(SamplingAct::RemoveTool(false)));
    }
    x += 62.0 + 15.0;
    if tool_button(
        ui,
        &painter,
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(58.0, 40.0)),
        "REMOVE",
        manual,
        manual && st.remove_tool,
        "remove",
    ) {
        acts.push(sact(SamplingAct::RemoveTool(true)));
    }
    x += 62.0 + 15.0;
    if tool_button(
        ui,
        &painter,
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(70.0, 40.0)),
        "DELETE ALL",
        !st.slices.is_empty(),
        false,
        "clear",
    ) {
        acts.push(sact(SamplingAct::ClearSlices));
    }
    let apply = Rect::from_min_size(Pos2::new(tools.right() - 80.0, y), Vec2::new(68.0, 40.0));
    if tool_button(ui, &painter, apply, "APPLY", st.slices.len() > 1, false, "apply") {
        acts.push(sact(SamplingAct::ApplySlices));
    }
    painter.text(
        Pos2::new(apply.left() - 10.0, apply.center().y),
        Align2::RIGHT_CENTER,
        format!("{} slices, on keys from C1", st.slices.len()),
        font(11.0),
        white(142),
    );
    // Footer.
    let fy = y + 48.0;
    let mode = st.slice_mode;
    let mut fields = vec![choice_field("MODE", SLICE_MODES.map(String::from).to_vec(), mode, |k| sact(SamplingAct::SliceMode(k)))];
    match mode {
        0 => {
            let i = st.split;
            fields.push(choice_field("SLICES", SPLIT_COUNTS.map(|c| c.to_string()).to_vec(), i, |k| sact(SamplingAct::SplitCount(k))));
        }
        1 => {
            let i = st.grid;
            fields.push(choice_field("SLICES", GRID_DIVS.map(|d| d.1.to_string()).to_vec(), i, |k| sact(SamplingAct::GridDiv(k))));
        }
        2 => {
            let s = st.sensitivity;
            fields.push(field("SENSITIVITY", format!("{:.0} %", s * 100.0), move |n| {
                sact(SamplingAct::Sensitivity((s + n * 0.01).clamp(0.0, 1.0)))
            }));
            let z = st.zero_x;
            fields.push(toggle_field("ZERO-X", z, move || sact(SamplingAct::ZeroX(!z))));
        }
        _ => {}
    }
    let nx = group_box(ui, &painter, tools.left() + 12.0, fy, "SLICE", &fields, acts);
    let pm = st.pattern_mode;
    group_box(
        ui,
        &painter,
        nx,
        fy,
        "APPLY",
        &[choice_field("PATTERN", PATTERN_MODES.map(String::from).to_vec(), pm, |k| sact(SamplingAct::PatternMode(k)))],
        acts,
    );
}

fn zone_tab(ui: &mut Ui, app: &App, p: &Project, body: Rect, zi: usize, gcol: Color32, acts: &mut Vec<Act>) {
    let st = &app.sampling;
    let painter = ui.painter_at(body);
    let Some(smp) = p.groups.get(app.group).and_then(|g| g.sounds.get(app.sound)).and_then(|s| s.sampler.as_ref()) else { return };
    let footer_h = 64.0;
    let main = Rect::from_min_max(body.min, Pos2::new(body.right(), body.bottom() - footer_h));
    // Zone list.
    let mut left = main.left();
    if st.zone_list {
        let list = Rect::from_min_size(main.min, Vec2::new(160.0, main.height()));
        painter.rect_filled(list, 0.0, S00);
        let lp = painter.with_clip_rect(list);
        for i in 0..=smp.zones.len() {
            let r =
                Rect::from_min_size(Pos2::new(list.left() + 5.0, list.top() + 5.0 + i as f32 * 25.0), Vec2::new(list.width() - 10.0, 24.0));
            if r.bottom() > list.bottom() {
                break;
            }
            let resp = ui.interact(r, ui.id().with(("zone-row", i)), Sense::click());
            match smp.zones.get(i) {
                Some(z) => {
                    let name = z.sample.path.rsplit(['/', '\\']).next().unwrap_or("").to_string();
                    let on = i == zi;
                    if on {
                        lp.rect_filled(r, 2.0, S02);
                    }
                    lp.text(
                        Pos2::new(r.left() + 6.0, r.center().y),
                        Align2::LEFT_CENTER,
                        &name,
                        font(11.0),
                        if on {
                            gcol
                        } else if resp.hovered() {
                            white(164)
                        } else {
                            white(142)
                        },
                    );
                    if resp.clicked() {
                        acts.push(sact(SamplingAct::FocusZone(i)));
                    }
                    resp.menu_on_right_click(|ui| {
                        if ui.button("Delete").clicked() {
                            acts.push(sact(SamplingAct::RemoveZone(i)));
                            ui.close();
                        }
                        if ui.button("Map as Drum Kit").clicked() {
                            acts.push(sact(SamplingAct::MapAsDrumKit));
                            ui.close();
                        }
                    });
                }
                None => {
                    lp.rect_stroke(r, 2.0, Stroke::new(1.0, if resp.hovered() { white(120) } else { white(40) }), StrokeKind::Inside);
                    super::plus_glyph(&lp, r.center(), 14.0, white(142));
                    if resp.on_hover_text("Add samples as zones").clicked() {
                        acts.push(sact(SamplingAct::AddZones));
                    }
                }
            }
        }
        left = list.right() + 3.0;
    }
    let view = Rect::from_min_max(Pos2::new(left, main.top()), main.max);
    if st.zone_map {
        zone_map(ui, app, smp, view, zi, gcol, acts);
    } else if let Some(z) = smp.zones.get(zi) {
        wave(ui, app, view, z, Target::Zone(app.group, app.sound, zi), gcol, false, acts);
    }
    // Footer: the focused zone's parameters.
    let Some(z) = smp.zones.get(zi) else { return };
    let footer = Rect::from_min_max(Pos2::new(body.left(), main.bottom() + 1.0), body.max);
    painter.rect_filled(footer, 0.0, S00);
    let zt = Target::Zone(app.group, app.sound, zi);
    let last = st.len().max(1) as i64 - 1;
    let per_step = frame_step(ui, last);
    let key = |k: i64, n: f64| (k as f64 + n).clamp(0.0, 127.0);
    let (kl, kh, vl, vh, root) = (z.key_range.0, z.key_range.1, z.velocity_range.0, z.velocity_range.1, z.root_key);
    let (tune, gain, pan_v) = (z.tune, z.gain, z.pan);
    let y = footer.top() + 4.0;
    let mut x = footer.left() + 8.0;
    x = group_box(ui, &painter, x, y, "PLAY RANGE", &play_fields(zt, z, last, per_step), acts);
    x = group_box(ui, &painter, x, y, "LOOP", &loop_fields(zt, z, last, per_step), acts);
    x = group_box(
        ui,
        &painter,
        x,
        y,
        "TUNE / MIX",
        &[
            field("TUNE", format!("{tune:+.2}"), move |n| Act::SetParam(zt, "tune", (tune + n * 0.01).clamp(-36.0, 36.0))),
            field("GAIN", format!("{:.1} dB", 20.0 * ((2.3948107 * gain.powi(5) + 0.767467 * gain * gain).max(1e-9)).log10()), move |n| {
                Act::SetParam(zt, "gain", (gain + n * 0.005).clamp(0.0, 1.0))
            }),
            field("PAN", pan(pan_v), move |n| Act::SetParam(zt, "pan", (pan_v + n * 0.01).clamp(-1.0, 1.0))),
            field("ROOT", note_name(root), move |n| Act::SetParam(zt, "root_key", key(root, n))),
        ],
        acts,
    );
    group_box(
        ui,
        &painter,
        x,
        y,
        "MAP",
        &[
            field("KEY LO", note_name(kl), move |n| Act::SetParam(zt, "key_range/low", key(kl, n).min(kh as f64))),
            field("KEY HI", note_name(kh), move |n| Act::SetParam(zt, "key_range/high", key(kh, n).max(kl as f64))),
            field("VEL LO", vl.to_string(), move |n| Act::SetParam(zt, "velocity_range/low", key(vl, n).min(vh as f64))),
            field("VEL HI", vh.to_string(), move |n| Act::SetParam(zt, "velocity_range/high", key(vh, n).max(vl as f64))),
        ],
        acts,
    );
}

/// The zone map: keys across (C-2…G8), velocity up; zones as rectangles. Drag a zone's
/// edges to change its key or velocity range, its middle to move it.
fn zone_map(ui: &mut Ui, app: &App, smp: &deckard_model::Sampler, view: Rect, zi: usize, gcol: Color32, acts: &mut Vec<Act>) {
    let painter = ui.painter_at(view);
    let keyboard = Rect::from_min_max(Pos2::new(view.left(), view.bottom() - 24.0), view.max);
    let grid = Rect::from_min_max(view.min, Pos2::new(view.right(), keyboard.top() - 1.0));
    ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-zone-grid"), grid));
    let kw = grid.width() / 128.0;
    let kx = |k: f32| grid.left() + k * kw;
    let vy = |v: f32| grid.bottom() - v / 128.0 * grid.height();
    for k in 0..128 {
        let black = matches!(k % 12, 1 | 3 | 6 | 8 | 10);
        painter.rect_filled(
            Rect::from_x_y_ranges(kx(k as f32)..=kx(k as f32 + 1.0), grid.y_range()),
            0.0,
            if black { Color32::from_rgb(47, 47, 47) } else { Color32::from_rgb(53, 53, 53) },
        );
        if k % 12 == 0 {
            painter.vline(kx(k as f32), grid.y_range(), Stroke::new(1.0, Color32::BLACK));
        }
        // Keyboard.
        let kr = Rect::from_x_y_ranges(kx(k as f32)..=kx(k as f32 + 1.0), keyboard.y_range());
        painter.rect_filled(kr, 0.0, if black { Color32::from_rgb(28, 28, 28) } else { Color32::from_rgb(134, 134, 134) });
    }
    if let Some(z) = smp.zones.get(zi) {
        painter.rect_filled(Rect::from_x_y_ranges(kx(z.root_key as f32)..=kx(z.root_key as f32 + 1.0), keyboard.y_range()), 0.0, gcol);
    }
    for v in [32, 64, 96] {
        painter.hline(grid.x_range(), vy(v as f32), Stroke::new(1.0, Color32::from_rgb(38, 38, 38)));
    }
    for (i, z) in smp.zones.iter().enumerate() {
        let r = Rect::from_x_y_ranges(
            kx(z.key_range.0 as f32)..=kx(z.key_range.1 as f32 + 1.0),
            vy(z.velocity_range.1 as f32 + 1.0)..=vy(z.velocity_range.0 as f32),
        );
        let focused = i == zi;
        painter.rect_filled(
            r,
            0.0,
            if focused { gcol.gamma_multiply(0.47) } else { Color32::from_rgba_unmultiplied(0x84, 0x84, 0x84, 32) },
        );
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, if focused { Color32::WHITE } else { white(102) }), StrokeKind::Inside);
        let name = z.sample.path.rsplit(['/', '\\']).next().unwrap_or("");
        painter.with_clip_rect(r).text(r.left_top() + Vec2::new(4.0, 3.0), Align2::LEFT_TOP, name, font(10.0), Color32::WHITE);
    }
    // Interaction.
    let resp = ui.interact(grid, ui.id().with("zone-map"), Sense::click_and_drag());
    let key_at = |x: f32| ((x - grid.left()) / kw).floor().clamp(0.0, 127.0);
    let vel_at = |y: f32| ((grid.bottom() - y) / grid.height() * 128.0).floor().clamp(0.0, 127.0);
    let hit = |pos: Pos2| -> Option<usize> {
        let k = key_at(pos.x) as i64;
        let v = vel_at(pos.y) as i64;
        let mut cands: Vec<usize> = smp
            .zones
            .iter()
            .enumerate()
            .filter(|(_, z)| (z.key_range.0..=z.key_range.1).contains(&k) && (z.velocity_range.0..=z.velocity_range.1).contains(&v))
            .map(|(i, _)| i)
            .collect();
        // Prefer the focused zone, then the last drawn.
        if cands.contains(&zi) { Some(zi) } else { cands.pop() }
    };
    let drag_id = ui.id().with("zone-map-drag");
    #[derive(Clone, Copy)]
    enum Edge {
        Left,
        Right,
        Top,
        Bottom,
        Move(f32, f32),
    }
    // Which of zone `i`'s edges `pos` is on, else its body.
    let edge_at = |i: usize, pos: Pos2| {
        let z = &smp.zones[i];
        let r = Rect::from_x_y_ranges(
            kx(z.key_range.0 as f32)..=kx(z.key_range.1 as f32 + 1.0),
            vy(z.velocity_range.1 as f32 + 1.0)..=vy(z.velocity_range.0 as f32),
        );
        if (pos.x - r.left()).abs() < 5.0 {
            Edge::Left
        } else if (pos.x - r.right()).abs() < 5.0 {
            Edge::Right
        } else if (pos.y - r.top()).abs() < 5.0 {
            Edge::Top
        } else if (pos.y - r.bottom()).abs() < 5.0 {
            Edge::Bottom
        } else {
            Edge::Move(key_at(pos.x), vel_at(pos.y))
        }
    };
    // The pointer shows what dragging there does.
    let held = if resp.dragged() {
        ui.data(|d| d.get_temp::<Option<(usize, Edge, (i64, i64), (i64, i64), i64)>>(drag_id)).flatten().map(|h| h.1)
    } else {
        None
    };
    let under = held.or_else(|| resp.hover_pos().and_then(|pos| hit(pos).map(|i| edge_at(i, pos))));
    if let Some(e) = under {
        ui.ctx().set_cursor_icon(match e {
            Edge::Left | Edge::Right => egui::CursorIcon::ResizeHorizontal,
            Edge::Top | Edge::Bottom => egui::CursorIcon::ResizeVertical,
            Edge::Move(..) if held.is_some() => egui::CursorIcon::Grabbing,
            Edge::Move(..) => egui::CursorIcon::Grab,
        });
    }
    if resp.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
        && let Some(i) = hit(pos)
    {
        let z = &smp.zones[i];
        let e = edge_at(i, pos);
        ui.data_mut(|d| d.insert_temp(drag_id, Some((i, e, z.key_range, z.velocity_range, z.root_key))));
        if i != zi {
            acts.push(sact(SamplingAct::FocusZone(i)));
        }
    }
    if resp.dragged()
        && let (Some(Some((i, e, kr, vr, root))), Some(pos)) =
            (ui.data(|d| d.get_temp::<Option<(usize, Edge, (i64, i64), (i64, i64), i64)>>(drag_id)), resp.interact_pointer_pos())
    {
        let t = Target::Zone(app.group, app.sound, i);
        let (k, v) = (key_at(pos.x) as i64, vel_at(pos.y) as i64);
        match e {
            Edge::Left => acts.push(Act::SetParam(t, "key_range/low", k.min(kr.1) as f64)),
            Edge::Right => acts.push(Act::SetParam(t, "key_range/high", k.max(kr.0) as f64)),
            Edge::Top => acts.push(Act::SetParam(t, "velocity_range/high", v.max(vr.0) as f64)),
            Edge::Bottom => acts.push(Act::SetParam(t, "velocity_range/low", v.min(vr.1) as f64)),
            Edge::Move(k0, v0) => {
                let dk = (k - k0 as i64).clamp(-kr.0, 127 - kr.1);
                let dv = (v - v0 as i64).clamp(-vr.0, 127 - vr.1);
                acts.push(Act::SetParam(t, "key_range/low", (kr.0 + dk) as f64));
                acts.push(Act::SetParam(t, "key_range/high", (kr.1 + dk) as f64));
                acts.push(Act::SetParam(t, "velocity_range/low", (vr.0 + dv) as f64));
                acts.push(Act::SetParam(t, "velocity_range/high", (vr.1 + dv) as f64));
                acts.push(Act::SetParam(t, "root_key", (root + dk).clamp(0, 127) as f64));
            }
        }
    }
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(i) = hit(pos)
    {
        acts.push(sact(SamplingAct::FocusZone(i)));
    }
    // The keyboard: click a key to set the focused zone's root key.
    let kresp = ui.interact(keyboard, ui.id().with("zone-keys"), Sense::click()).on_hover_text("Click: the focused zone's root key");
    if kresp.clicked()
        && let Some(pos) = kresp.interact_pointer_pos()
    {
        acts.push(Act::SetParam(Target::Zone(app.group, app.sound, zi), "root_key", key_at(pos.x) as f64));
    }
}

/// RECORD: input meter and threshold, START/STOP and CANCEL, the takes; the footer
/// with Maschine's parameter groups (Recording, Input, and for Auto the Note Map,
/// Velocity Map and Batch Process; Monitor for external sources).
fn record_tab(ui: &mut Ui, app: &App, p: &Project, body: Rect, gcol: Color32, acts: &mut Vec<Act>) {
    use crate::recorder as r;
    let rec = &app.recorder.view;
    let busy = rec.status != RecStatus::Idle;
    let painter = ui.painter_at(body);
    painter.rect_filled(body, 0.0, S00);
    let top = Rect::from_min_size(body.min + Vec2::new(15.0, 10.0), Vec2::new(body.width() - 30.0, 70.0));
    // Input meters (L, R) with the threshold slider (Detect, while idle).
    let meter = Rect::from_min_size(top.min, Vec2::new(200.0, 30.0));
    for k in 0..2 {
        let m = Rect::from_min_size(Pos2::new(meter.left(), meter.top() + 6.0 + k as f32 * 10.0), Vec2::new(meter.width(), 6.0));
        painter.rect_filled(m, 0.0, Color32::BLACK);
        let lv = if k == 0 { rec.meter.0 } else { rec.meter.1 };
        let db = if lv > 0.0 { 20.0 * lv.log10() } else { -60.0 };
        let fill = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
        painter.rect_filled(
            Rect::from_min_size(m.min, Vec2::new(m.width() * fill, m.height())),
            0.0,
            if lv >= 1.0 { REC_ON } else { gcol },
        );
    }
    let detect = rec.mode == 0;
    if detect {
        let thr_x = meter.left() + meter.width() * ((rec.threshold + 60.0) / 60.0).clamp(0.0, 1.0) as f32;
        painter.vline(thr_x, meter.y_range(), Stroke::new(2.0, if busy { white(90) } else { Color32::WHITE }));
        let tr = ui
            .interact(meter, ui.id().with("rec-threshold"), Sense::click_and_drag())
            .on_hover_text("Threshold: recording starts when the input passes it (double-click: -12 dB)");
        if !busy && (tr.hovered() || tr.dragged()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if !busy && tr.dragged() {
            let d = tr.drag_delta().x as f64 / meter.width() as f64 * 60.0;
            acts.push(sact(SamplingAct::Record(RecordAct::Threshold((rec.threshold + d).clamp(-60.0, 0.0)))));
        }
        if !busy && tr.double_clicked() {
            acts.push(sact(SamplingAct::Record(RecordAct::Threshold(-12.0))));
        }
        painter.text(
            Pos2::new(meter.left(), meter.bottom() + 8.0),
            Align2::LEFT_CENTER,
            format!("THRESHOLD {}", threshold_text(rec.threshold)),
            font(9.0),
            white(110),
        );
    }
    // START / WAITING / STOP and CANCEL.
    let bx = meter.right() + 20.0;
    let start = Rect::from_min_size(Pos2::new(bx, top.top()), Vec2::new(64.0, 26.0));
    let label = match rec.status {
        RecStatus::Idle => "START",
        RecStatus::Waiting => "WAITING",
        RecStatus::Recording => "STOP",
    };
    let can = busy || app.recorder.can_start();
    let resp = ui.interact(start, ui.id().with("rec-start"), Sense::click());
    let resp = if can {
        resp
    } else {
        resp.on_hover_text(if rec.source == 2 { "Pick the sound to sample (Input)" } else { "Pick the MIDI output to play (MIDI To)" })
    };
    painter.rect_filled(
        start,
        3.0,
        if rec.status == RecStatus::Recording {
            REC_ON
        } else if resp.hovered() && can {
            S03
        } else {
            S02
        },
    );
    painter.text(start.center(), Align2::CENTER_CENTER, label, font(11.0), if can { white(245) } else { white(80) });
    if can && resp.clicked() {
        acts.push(sact(SamplingAct::Record(if rec.status == RecStatus::Idle { RecordAct::Start } else { RecordAct::Stop })));
    }
    let cancel = Rect::from_min_size(Pos2::new(bx, start.bottom() + 6.0), Vec2::new(64.0, 22.0));
    if tool_button(ui, &painter, cancel, "CANCEL", busy, false, "rec-cancel") {
        acts.push(sact(SamplingAct::Record(RecordAct::Cancel)));
    }
    if let Some(e) = &rec.error {
        painter.text(Pos2::new(bx + 80.0, start.center().y), Align2::LEFT_CENTER, e, font(11.0), REC_ON);
    } else if !rec.info.is_empty() {
        let secs = if rec.status == RecStatus::Recording && rec.mode != 3 {
            format!("  {:.1} s", rec.recorded_frames as f64 / rec.rate.max(1) as f64)
        } else {
            String::new()
        };
        painter.text(Pos2::new(bx + 80.0, start.center().y), Align2::LEFT_CENTER, format!("{}{secs}", rec.info), font(11.0), white(245));
    } else if rec.takes.is_empty() {
        painter.text(
            Pos2::new(bx + 80.0, start.center().y),
            Align2::LEFT_CENTER,
            "Press START to record your input source",
            font(11.0),
            white(110),
        );
    }
    // Takes.
    let mut tx = bx + 80.0;
    let ty = top.top() + 34.0;
    for (i, take) in rec.takes.iter().enumerate() {
        let rr = Rect::from_min_size(Pos2::new(tx, ty), Vec2::new(79.0, 40.0));
        if rr.right() > top.right() {
            break;
        }
        let resp = ui
            .interact(rr, ui.id().with(("take", i)), Sense::click_and_drag())
            .on_hover_text("Click to load it into the focused sound; drag it onto a sound");
        if resp.drag_started() {
            resp.dnd_set_drag_payload(DraggedTake(take.clone()));
        }
        painter.rect_filled(rr, 2.0, Color32::from_rgb(32, 32, 32));
        painter.rect_stroke(rr, 2.0, Stroke::new(1.0, if rec.focused == Some(i) { gcol } else { S03 }), StrokeKind::Inside);
        // Its waveform, the name over it.
        if let Some(wave) = rec.take_waves.get(take).filter(|w| !w.is_empty()) {
            let inner = rr.shrink2(Vec2::new(3.0, 4.0));
            let mid = inner.center().y + 4.0;
            let half = (inner.height() - 8.0) / 2.0;
            let cols = inner.width() as usize;
            for c in 0..cols {
                let (lo, hi) = wave[(c * wave.len() / cols.max(1)).min(wave.len() - 1)];
                let x = inner.left() + c as f32 + 0.5;
                painter.vline(
                    x,
                    (mid - hi.clamp(-1.0, 1.0) * half)..=(mid - lo.clamp(-1.0, 1.0) * half + 1.0),
                    Stroke::new(1.0, gcol.gamma_multiply(0.8)),
                );
            }
        }
        let name = take.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        painter.with_clip_rect(rr).text(rr.left_top() + Vec2::new(4.0, 2.0), Align2::LEFT_TOP, name, font(9.0), white(200));
        let del = Rect::from_min_size(Pos2::new(rr.right() - 16.0, rr.bottom() - 16.0), Vec2::splat(14.0));
        let dr = ui.interact(del, ui.id().with(("take-del", i)), Sense::click()).on_hover_text("Delete the take (its file stays)");
        painter.text(del.center(), Align2::CENTER_CENTER, "×", font(12.0), if dr.hovered() { white(245) } else { white(110) });
        if dr.clicked() {
            acts.push(sact(SamplingAct::Record(RecordAct::DeleteTake(i))));
        } else if resp.clicked() {
            acts.push(sact(SamplingAct::Record(RecordAct::FocusTake(i))));
        }
        tx += 85.0;
    }
    // The waveform of what is being recorded (the last few seconds).
    let w = Rect::from_min_max(Pos2::new(body.left(), top.bottom() + 8.0), Pos2::new(body.right(), body.bottom() - 66.0));
    painter.rect_filled(w, 0.0, Color32::BLACK);
    let n = rec.preview.len();
    if n > 0 {
        // Inputs can go past full scale: clamp, and keep everything inside the area.
        let wp = painter.with_clip_rect(w);
        let mid = w.center().y;
        let half = w.height() * 0.48;
        let cols: Vec<_> = rec
            .preview
            .iter()
            .enumerate()
            .map(|(c, (mn, mx))| crate::sampler_ui::WaveCol {
                x: w.left() + c as f32 * w.width() / n as f32,
                top: mid - mx.clamp(-1.0, 1.0) * half,
                bottom: mid - mn.clamp(-1.0, 1.0) * half,
                color: gcol,
            })
            .collect();
        crate::sampler_ui::paint_wave(&wp, &cols);
    }
    // Footer: Maschine's parameter groups.
    let footer = Rect::from_min_max(Pos2::new(body.left(), w.bottom() + 1.0), body.max);
    let set = |param: RecParam| move |v: usize| sact(SamplingAct::Record(RecordAct::Set(param, v as i64)));
    let step = |param: RecParam, cur: i64, by: i64, lo: i64, hi: i64| {
        move |n: f64| sact(SamplingAct::Record(RecordAct::Set(param, (cur + n as i64 * by).clamp(lo, hi))))
    };
    let toggle = |label: &'static str, param: RecParam, on: bool| {
        toggle_field(label, on, move || sact(SamplingAct::Record(RecordAct::Set(param, !on as i64))))
    };
    let names = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let a = &rec.auto;
    let auto = rec.mode == 3;
    let external = rec.source < 2;
    // Recording.
    let mut fields = vec![choice_field("MODE", names(&r::MODES), rec.mode, set(RecParam::Mode))];
    match rec.mode {
        0 => {
            let t = rec.threshold;
            fields.push(Field {
                tip: "Drag up or down (Shift: fine); double-click the meter's slider for -12 dB",
                ..field("THRESHOLD", threshold_text(t), move |n| {
                    sact(SamplingAct::Record(RecordAct::Threshold((t + n * 0.5).clamp(-60.0, 0.0))))
                })
            });
        }
        1 => fields.push(choice_field("LENGTH", names(&r::LENGTHS), rec.length, set(RecParam::Length))),
        2 => {
            fields.push(choice_field("LENGTH", names(&r::LENGTHS), rec.length, set(RecParam::Length)));
            fields.push(choice_field("TARGET", names(&r::TARGETS), rec.target, set(RecParam::Target)));
        }
        _ => {
            fields.push(field("SAMPLE", ms_text(a.sample_ms), step(RecParam::SampleMs, a.sample_ms as i64, 100, 100, 30_000)));
            fields.push(field("NOTE ON", ms_text(a.note_on_ms), step(RecParam::NoteOnMs, a.note_on_ms as i64, 100, 100, 30_000)));
        }
    }
    let y = footer.top() + 6.0;
    let mut nx = group_box(ui, &painter, footer.left() + 8.0, y, "RECORDING", &fields, acts);
    // Input.
    let mut fields = vec![choice_field("SOURCE", names(&r::SOURCES), rec.source, set(RecParam::Source))];
    match rec.source {
        // The connected inputs (and the one picked), by their aliases.
        0 | 1 => {
            let io = &app.settings;
            let stereo = rec.source == 0;
            let (cur, n) = if stereo { (rec.ext_stereo, r::INPUT_PAIRS) } else { (rec.ext_mono, r::INPUT_PAIRS * 2) };
            let shown: Vec<usize> = (0..n).filter(|&i| i == cur || if stereo { io.input_pair_on(i) } else { io.input_on(i) }).collect();
            let names = shown.iter().map(|&i| if stereo { io.input_pair_name(i) } else { io.input_name(i) }).collect();
            let at = shown.iter().position(|&i| i == cur).unwrap_or(0);
            let pick = set(if stereo { RecParam::ExtStereo } else { RecParam::ExtMono });
            fields.push(choice_field("INPUT", names, at, move |k| pick(shown.get(k).copied().unwrap_or(cur))));
        }
        _ if auto => {
            // A sound to play and record: those with an instrument or a Sampler.
            let mut opts = vec![("None".to_string(), -1i64)];
            for (g, gr) in p.groups.iter().enumerate() {
                for (s, snd) in gr.sounds.iter().enumerate() {
                    if !snd.slots.is_empty() || snd.sampler.is_some() {
                        opts.push((format!("{}-{} {}", crate::layout::group_id(g), s + 1, snd.name), (g * 16 + s) as i64));
                    }
                }
            }
            let cur = rec.auto_sound.map_or(0, |(g, s)| opts.iter().position(|o| o.1 == (g * 16 + s) as i64).unwrap_or(0));
            let values: Vec<i64> = opts.iter().map(|o| o.1).collect();
            fields.push(choice_field("INPUT", opts.into_iter().map(|o| o.0).collect(), cur, move |k| {
                sact(SamplingAct::Record(RecordAct::Set(RecParam::AutoSound, values[k])))
            }));
        }
        _ => {
            let (opts, sources): (Vec<String>, Vec<deckard_engine::TapSource>) = internal_inputs(&p.groups).into_iter().unzip();
            let cur = sources.iter().position(|&s| s == rec.internal).unwrap_or(0);
            fields.push(choice_field("INPUT", opts, cur, move |k| {
                sact(SamplingAct::Record(RecordAct::Set(RecParam::Internal, internal_value(sources[k]))))
            }));
        }
    }
    if auto && external {
        let mut opts = vec!["None".to_string()];
        opts.extend(rec.midi_outs.iter().cloned());
        let cur = a.midi_to.as_ref().and_then(|m| opts.iter().position(|o| o == m)).unwrap_or(0);
        let ports = opts.clone();
        let mut f =
            choice_field("MIDI TO", opts, cur, move |k| sact(SamplingAct::Record(RecordAct::MidiTo((k > 0).then(|| ports[k].clone())))));
        f.click = Some(Box::new(|| sact(SamplingAct::Record(RecordAct::ScanMidi))));
        fields.push(f);
        fields.push(field("CHANNEL", a.channel.to_string(), step(RecParam::Channel, a.channel as i64, 1, 1, 16)));
    }
    nx = group_box(ui, &painter, nx, y, "INPUT", &fields, acts);
    if auto {
        let key = |n: u8| crate::recorder::note_name(n);
        let fields = [
            field("STRIDE", a.key_stride.to_string(), step(RecParam::KeyStride, a.key_stride as i64, 1, 1, 127)),
            field("KEY LO", key(a.key_lo), step(RecParam::KeyLo, a.key_lo as i64, 1, 0, 127)),
            field("KEY HI", key(a.key_hi), step(RecParam::KeyHi, a.key_hi as i64, 1, 0, 127)),
            toggle("EXTEND", RecParam::KeyExtend, a.key_extend),
        ];
        nx = group_box(ui, &painter, nx, y, "NOTE MAP", &fields, acts);
        let fields = [
            field("STRIDE", a.vel_stride.to_string(), step(RecParam::VelStride, a.vel_stride as i64, 1, 1, 127)),
            field("VEL LO", a.vel_lo.to_string(), step(RecParam::VelLo, a.vel_lo as i64, 1, 0, 127)),
            field("VEL HI", a.vel_hi.to_string(), step(RecParam::VelHi, a.vel_hi as i64, 1, 0, 127)),
            toggle("EXTEND", RecParam::VelExtend, a.vel_extend),
        ];
        nx = group_box(ui, &painter, nx, y, "VELOCITY MAP", &fields, acts);
        let fields = [
            toggle("FIND LOOP", RecParam::FindLoop, a.find_loop),
            toggle("TRIM SILENCE", RecParam::TrimSilence, a.trim_silence),
            toggle("NORMALIZE", RecParam::Normalize, a.normalize),
        ];
        nx = group_box(ui, &painter, nx, y, "BATCH PROCESS", &fields, acts);
    }
    if external {
        group_box(ui, &painter, nx, y, "MONITOR", &[toggle("MONITOR", RecParam::Monitor, rec.monitor)], acts);
    }
}

pub(crate) fn threshold_text(db: f64) -> String {
    if db <= crate::recorder::THRESHOLD_OFF { "OFF".into() } else { format!("{db:.1} dB") }
}

pub(crate) fn ms_text(ms: u32) -> String {
    format!("{ms} ms")
}

/// Recording state the RECORD tab shows (kept by the app's recorder).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecStatus {
    #[default]
    Idle,
    /// Armed: Detect waits for the threshold, Sync and Loop for the next bar or pattern.
    Waiting,
    Recording,
}

/// A recorder parameter set from the footer (values as integers; switches 0/1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecParam {
    Source,
    ExtStereo,
    ExtMono,
    /// -1: the master, else a group.
    Internal,
    /// -1: none, else group × 16 + sound.
    AutoSound,
    Mode,
    Length,
    Target,
    Monitor,
    Channel,
    NoteOnMs,
    SampleMs,
    KeyStride,
    KeyLo,
    KeyHi,
    KeyExtend,
    VelStride,
    VelLo,
    VelHi,
    VelExtend,
    FindLoop,
    TrimSilence,
    Normalize,
}

/// The Auto Sampler's settings (Maschine's defaults; not saved with the project).
#[derive(Debug, Clone, PartialEq)]
pub struct AutoParams {
    /// The MIDI output played (Ext. sources), and its channel (1–16).
    pub midi_to: Option<String>,
    pub channel: u8,
    /// How long each note is held, and how long each is recorded (ms; Note On ≤ Sample).
    pub note_on_ms: u32,
    pub sample_ms: u32,
    pub key_stride: u32,
    pub key_lo: u8,
    pub key_hi: u8,
    pub key_extend: bool,
    pub vel_stride: u32,
    pub vel_lo: u8,
    pub vel_hi: u8,
    pub vel_extend: bool,
    pub find_loop: bool,
    pub trim_silence: bool,
    pub normalize: bool,
}

impl Default for AutoParams {
    fn default() -> AutoParams {
        AutoParams {
            midi_to: None,
            channel: 1,
            note_on_ms: 2000,
            sample_ms: 5000,
            key_stride: 1,
            key_lo: 60,
            key_hi: 60,
            key_extend: true,
            vel_stride: 1,
            vel_lo: 100,
            vel_hi: 100,
            vel_extend: true,
            find_loop: false,
            trim_silence: false,
            normalize: false,
        }
    }
}

/// Internal ▸ INPUT: the master, then each group followed by its sounds that play
/// something. (Maschine lists the master and groups only; a sound is DECKARD's own, so
/// one sound can be recorded without soloing it.)
pub fn internal_inputs(groups: &[deckard_model::Group]) -> Vec<(String, deckard_engine::TapSource)> {
    use deckard_engine::TapSource;
    let mut out = vec![("Master".to_string(), TapSource::Master)];
    for (g, gr) in groups.iter().enumerate() {
        let id = crate::layout::group_id(g);
        out.push((format!("{id} - {}", gr.name), TapSource::Group(g)));
        for (s, snd) in gr.sounds.iter().enumerate() {
            if !snd.slots.is_empty() || snd.sampler.is_some() {
                out.push((format!("{id} {} {}", s + 1, snd.name).trim_end().to_string(), TapSource::Sound(g, s)));
            }
        }
    }
    out
}

/// An internal input as `RecParam::Internal`'s value: -1 the master, a group's index,
/// or 1000 + 16 × group + sound.
pub fn internal_value(t: deckard_engine::TapSource) -> i64 {
    use deckard_engine::TapSource;
    match t {
        TapSource::Master => -1,
        TapSource::Group(g) => g as i64,
        TapSource::Sound(g, s) => 1000 + (g * 16 + s) as i64,
    }
}

fn internal_from(v: i64) -> deckard_engine::TapSource {
    use deckard_engine::TapSource;
    match v {
        ..0 => TapSource::Master,
        0..1000 => TapSource::Group(v as usize),
        _ => TapSource::Sound((v - 1000) as usize / 16, (v - 1000) as usize % 16),
    }
}

/// What the RECORD tab shows, and the recorder's settings.
#[derive(Default)]
pub struct RecorderView {
    pub status: RecStatus,
    /// Ext. Ster., Ext. Mono or Internal, and each one's input: a stereo pair (0–15),
    /// a channel (0–31), the master, a group or a sound; Auto + Internal: the sound.
    pub source: usize,
    pub ext_stereo: usize,
    pub ext_mono: usize,
    pub internal: deckard_engine::TapSource,
    pub auto_sound: Option<(usize, usize)>,
    /// Detect, Sync, Loop or Auto.
    pub mode: usize,
    pub threshold: f64,
    /// 1, 2, 4, 8 or 16 bars, or FREE; Loop's target: Take, Sound or Pattern.
    pub length: usize,
    pub target: usize,
    pub monitor: bool,
    pub auto: AutoParams,
    /// MIDI outputs there are (Auto ▸ MIDI To).
    pub midi_outs: Vec<String>,
    pub meter: (f32, f32),
    pub recorded_frames: usize,
    pub rate: u32,
    pub preview: Vec<(f32, f32)>,
    pub takes: Vec<std::path::PathBuf>,
    /// Each take's mini waveform (`recorder::mini_wave`), by file.
    pub take_waves: std::collections::HashMap<std::path::PathBuf, std::sync::Arc<Vec<(f32, f32)>>>,
    pub focused: Option<usize>,
    /// The info bar's status ("Waiting for next bar...").
    pub info: String,
    pub error: Option<String>,
}

impl RecorderView {
    /// Set a parameter, keeping Maschine's rules: Note On ≤ Sample, Lo ≤ Hi (moving the
    /// other), strides within 1..=max(1, Hi − Lo).
    pub fn set(&mut self, p: RecParam, v: i64) {
        let a = &mut self.auto;
        let u8v = v.clamp(0, 127) as u8;
        match p {
            RecParam::Source => self.source = v.clamp(0, 2) as usize,
            RecParam::ExtStereo => self.ext_stereo = v.clamp(0, 15) as usize,
            RecParam::ExtMono => self.ext_mono = v.clamp(0, 31) as usize,
            RecParam::Internal => self.internal = internal_from(v),
            RecParam::AutoSound => self.auto_sound = (v >= 0).then_some((v as usize / 16, v as usize % 16)),
            RecParam::Mode => self.mode = v.clamp(0, 3) as usize,
            RecParam::Length => self.length = v.clamp(0, 5) as usize,
            RecParam::Target => self.target = v.clamp(0, 2) as usize,
            RecParam::Monitor => self.monitor = v != 0,
            RecParam::Channel => a.channel = v.clamp(1, 16) as u8,
            RecParam::NoteOnMs => {
                a.note_on_ms = v.clamp(100, 30_000) as u32;
                a.sample_ms = a.sample_ms.max(a.note_on_ms);
            }
            RecParam::SampleMs => {
                a.sample_ms = v.clamp(100, 30_000) as u32;
                a.note_on_ms = a.note_on_ms.min(a.sample_ms);
            }
            RecParam::KeyStride => a.key_stride = v.max(1) as u32,
            RecParam::KeyLo => {
                a.key_lo = u8v;
                a.key_hi = a.key_hi.max(u8v);
            }
            RecParam::KeyHi => {
                a.key_hi = u8v;
                a.key_lo = a.key_lo.min(u8v);
            }
            RecParam::KeyExtend => a.key_extend = v != 0,
            RecParam::VelStride => a.vel_stride = v.max(1) as u32,
            RecParam::VelLo => {
                a.vel_lo = u8v;
                a.vel_hi = a.vel_hi.max(u8v);
            }
            RecParam::VelHi => {
                a.vel_hi = u8v;
                a.vel_lo = a.vel_lo.min(u8v);
            }
            RecParam::VelExtend => a.vel_extend = v != 0,
            RecParam::FindLoop => a.find_loop = v != 0,
            RecParam::TrimSilence => a.trim_silence = v != 0,
            RecParam::Normalize => a.normalize = v != 0,
        }
        a.key_stride = a.key_stride.clamp(1, (a.key_hi - a.key_lo).max(1) as u32);
        a.vel_stride = a.vel_stride.clamp(1, (a.vel_hi - a.vel_lo).max(1) as u32);
    }
}

#[cfg(test)]
mod range_tests {
    use super::*;

    /// A sample comes up selected: what was saved, else its play range; half open, as the
    /// selection is.
    #[test]
    fn a_sample_comes_up_selected() {
        let mut z = deckard_model::Zone { play_range: (100, 199), ..Default::default() };
        assert_eq!(first_selection(&z, 1000), Some((100, 200)), "nothing saved: the play range");
        z.selection_range = (10, 20);
        assert_eq!(first_selection(&z, 1000), Some((10, 21)), "as saved");
        z.selection_range = (10, 5000);
        assert_eq!(first_selection(&z, 1000), Some((100, 200)), "past the end of the sample");
        z.play_range = (0, 0);
        assert_eq!(first_selection(&z, 1000), Some((0, 1000)), "no play range: all of it");
        assert_eq!(first_selection(&z, 0), None, "nothing to select");
        assert_eq!(first_selection(&z, 1), None);
    }
}

/// The zone to show for a note played: `current` if the note plays it (zones can overlap,
/// and the one in view shouldn't flicker away), else the first it plays; none if it plays
/// no zone.
pub fn zone_for_note(zones: &[deckard_model::Zone], current: usize, pitch: i64, velocity: i64) -> Option<usize> {
    let plays = |z: &deckard_model::Zone| {
        (z.key_range.0..=z.key_range.1).contains(&pitch) && (z.velocity_range.0..=z.velocity_range.1).contains(&velocity)
    };
    if zones.get(current).is_some_and(plays) {
        return Some(current);
    }
    zones.iter().position(plays)
}

#[cfg(test)]
mod zone_for_note_tests {
    use super::zone_for_note;
    use deckard_model::Zone;

    #[test]
    fn the_zone_a_note_lands_in_is_shown() {
        let zone = |keys, vels| Zone { key_range: keys, velocity_range: vels, ..Default::default() };
        let zones = [zone((0, 59), (0, 127)), zone((60, 71), (0, 63)), zone((60, 71), (64, 127)), zone((60, 127), (0, 127))];
        assert_eq!(zone_for_note(&zones, 0, 30, 100), Some(0), "stays");
        assert_eq!(zone_for_note(&zones, 0, 65, 30), Some(1), "by key and velocity");
        assert_eq!(zone_for_note(&zones, 0, 65, 100), Some(2));
        assert_eq!(zone_for_note(&zones, 0, 80, 100), Some(3));
        assert_eq!(zone_for_note(&zones, 3, 65, 100), Some(3), "overlapping: the one in view stays");
        assert_eq!(zone_for_note(&zones[..3], 1, 100, 100), None, "no zone there: nothing changes");
        assert_eq!(zone_for_note(&[], 0, 60, 100), None);
    }
}

#[cfg(test)]
mod internal_input_tests {
    use super::*;
    use deckard_engine::TapSource;
    use deckard_model::{Group, SlotInfo, Sound};

    /// Internal ▸ INPUT lists the master, each group, and each of a group's sounds that
    /// plays something (empty ones can't be heard).
    #[test]
    fn internal_input_lists_sounds_under_their_groups() {
        let sound = |name: &str, used: bool| Sound {
            name: name.into(),
            slots: if used { vec![SlotInfo { module: "Sampler".into(), ..Default::default() }] } else { vec![] },
            ..Default::default()
        };
        let groups = vec![
            Group { name: "Drums".into(), sounds: vec![sound("Kick", true), sound("", false), sound("Snare", true)], ..Default::default() },
            Group { name: "Bass".into(), sounds: vec![sound("Sub", true)], ..Default::default() },
        ];
        let got = internal_inputs(&groups);
        let labels: Vec<&str> = got.iter().map(|o| o.0.as_str()).collect();
        assert_eq!(labels, ["Master", "A1 - Drums", "A1 1 Kick", "A1 3 Snare", "B1 - Bass", "B1 1 Sub"]);
        let sources: Vec<TapSource> = got.iter().map(|o| o.1).collect();
        assert_eq!(
            sources,
            [
                TapSource::Master,
                TapSource::Group(0),
                TapSource::Sound(0, 0),
                TapSource::Sound(0, 2),
                TapSource::Group(1),
                TapSource::Sound(1, 0)
            ]
        );
        // Each goes through the knob's value and back.
        for s in sources {
            let mut v = RecorderView::default();
            v.set(RecParam::Internal, internal_value(s));
            assert_eq!(v.internal, s);
        }
        assert_eq!(RecorderView::default().internal, TapSource::Master, "the master by default");
    }
}
