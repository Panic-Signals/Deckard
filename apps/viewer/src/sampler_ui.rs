//! The Sampler's pages in the SOUND tab: Maschine 3's Main and FX pages (SamplerEditor.qml)
//! plus the modulation, LFO and velocity/mod wheel parameters (the hardware's pages),
//! and the sound's output. Values are shown in Maschine's units (docs/sampler-dsp.md).

use crate::layout::{Act, Edit, Knob};
use deckard_doc::Target;
use deckard_engine::sampler as dsp;
use deckard_model::{Sampler, Zone};
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};

/// Maschine's Sampler pages (`Sampler::s_aPageNames`; docs/sampler-pages.md).
pub const PAGES: [&str; 6] = ["Voice Settings / Engine", "Pitch / Envelope", "FX / Filter", "Modulation", "LFO", "Velocity / Modwheel"];

/// A page: eight knob slots (empty ones `None`) and the section headings that start at
/// a slot.
pub struct Page {
    pub slots: [Option<Knob>; 8],
    pub sections: Vec<(usize, &'static str)>,
}

/// A zone's sample, reduced to (min, max) per column for drawing.
pub struct Waveform {
    /// The zone it is for: (group, sound, zone, sample path).
    pub key: (usize, usize, usize, String),
    /// Of the two channels mixed.
    pub peaks: Vec<(f32, f32)>,
    /// Of each channel (left, right), as the plug-in strip draws them.
    pub channels: [Vec<(f32, f32)>; 2],
    /// Frames in the sample, and its rate.
    pub len: usize,
    pub rate: u32,
}

impl Waveform {
    pub fn new(key: (usize, usize, usize, String), frames: &[[f32; 2]], rate: u32) -> Waveform {
        const COLUMNS: usize = 1024;
        let len = frames.len();
        let reduce = |of: &dyn Fn(&[f32; 2]) -> f32| -> Vec<(f32, f32)> {
            (0..COLUMNS)
                .map(|c| {
                    let (a, b) = (c * len / COLUMNS, ((c + 1) * len / COLUMNS).max(c * len / COLUMNS + 1).min(len));
                    frames[a.min(len.saturating_sub(1))..b].iter().fold((0f32, 0f32), |(lo, hi), f| {
                        let m = of(f);
                        (lo.min(m), hi.max(m))
                    })
                })
                .collect()
        };
        let peaks = reduce(&|f| (f[0] + f[1]) * 0.5);
        let channels = [reduce(&|f| f[0]), reduce(&|f| f[1])];
        Waveform { key, peaks, channels, len, rate }
    }
}

const POLY: [&str; 8] = ["Legato", "1", "2", "4", "8", "16", "32", "64"];
const MODEL: [&str; 7] = ["Standard", "MP60", "S1200", "S1200 L", "S1200 LM", "S1200 HM", "S1200 H"];
const ENVELOPE: [&str; 3] = ["One-shot", "AHD", "ADSR"];
const FILTER: [&str; 5] = ["Off", "LP2", "BP2", "HP2", "EQ"];
const LFO_TYPE: [&str; 5] = ["Sine", "Tri", "Rect", "Saw", "Random"];
const LFO_SYNC: [&str; 3] = ["Free", "Retrig", "Lock"];

fn ms(v: f64) -> String {
    if v < 1000.0 { format!("{v:.1} ms") } else { format!("{:.2} s", v / 1000.0) }
}

fn hz(v: f64) -> String {
    if v < 1000.0 { format!("{v:.0} Hz") } else { format!("{:.1} kHz", v / 1000.0) }
}

fn percent(v: f64) -> String {
    format!("{:.0} %", v * 100.0)
}

fn amount(v: f64) -> String {
    format!("{:+.0} %", v * 100.0)
}

/// Sampler page `page` of sound `s` in group `g`, as Maschine lays it out
/// (docs/sampler-pages.md): which parameter is in each of the eight slots depends on
/// the polyphony, envelope type and filter mode.
pub fn page(smp: &Sampler, g: usize, s: usize, page: usize) -> Page {
    let t = Target::Sampler(g, s);
    let value = |path: &str| smp.get(path);
    let knob = |label: &'static str, path: &'static str, min: f64, max: f64, text: String| -> Option<Knob> {
        let v = smp.get(path);
        let default = deckard_model::SAMPLER_PARAMS.iter().find(|(k, _)| *k == path).map_or(0.0, |(_, d)| *d);
        Some((label.into(), text, ((v - min) / (max - min)) as f32, Some(Edit::Param(t, path, min, max, default))))
    };
    // Choices are selectors (the value between up and down arrows), as in Maschine.
    let choice = |label: &'static str, path: &'static str, options: &'static [&'static str]| -> Option<Knob> {
        let i = (value(path).max(0.0) as usize).min(options.len() - 1);
        let default = deckard_model::SAMPLER_PARAMS.iter().find(|(k, _)| *k == path).map_or(0.0, |(_, d)| *d);
        let n = options.len();
        Some((label.into(), options[i].to_string(), i as f32 / (n - 1) as f32, Some(Edit::Choice(t, path, options, i, default))))
    };
    let env_ms = |e: &str| {
        let (a, h, d) = dsp::envelope_ms(value(&format!("{e}/attack")), value(&format!("{e}/hold")), value(&format!("{e}/decay")));
        (a, h, d, dsp::envelope_ms(0.0, 0.0, value(&format!("{e}/release"))).2)
    };
    let env_type = value("envelope_type") as i64;
    let mut k: [Option<Knob>; 8] = Default::default();
    let sections: Vec<(usize, &'static str)> = match page {
        0 => {
            k[0] = choice("POLYPHONY", "polyphony", &POLY);
            if value("polyphony") as i64 == 0 {
                let gl = value("glide");
                k[2] = knob("GLIDE", "glide", 0.0, 1.0, format!("{:.2} s", gl * gl));
            }
            let pb = value("pitchbend");
            k[3] = knob("PITCHBEND", "pitchbend", 0.0, 1.0, format!("±{:.1} st", pb * pb * 12.0));
            k[4] = choice("MODE", "mode", &MODEL);
            vec![(0, "Voice Settings"), (4, "Engine")]
        }
        1 => {
            k[0] = knob("TUNE", "tune", -1.0, 1.0, format!("{:+.2}", value("tune") * 36.0));
            k[1] = knob("START", "start", 0.0, 1.0, percent(value("start")));
            k[2] = Some((
                "REVERSE".into(),
                if smp.reverse { "On" } else { "Off" }.into(),
                smp.reverse as i32 as f32,
                Some(Edit::Toggle(t, "reverse", smp.reverse)),
            ));
            k[3] = choice("TYPE", "envelope_type", &ENVELOPE);
            let (a, h, d, r) = env_ms("volume_envelope");
            let sustain = percent(value("volume_envelope/sustain").powi(3));
            match env_type {
                1 => {
                    k[4] = knob("ATTACK", "volume_envelope/attack", 0.0, 1.0, ms(a));
                    k[5] = knob("HOLD", "volume_envelope/hold", 0.0, 1.0, ms(h));
                    k[6] = knob("DECAY", "volume_envelope/decay", 0.0, 1.0, ms(d));
                }
                2 => {
                    k[4] = knob("ATTACK", "volume_envelope/attack", 0.0, 1.0, ms(a));
                    k[5] = knob("DECAY", "volume_envelope/decay", 0.0, 1.0, ms(d));
                    k[6] = knob("SUSTAIN", "volume_envelope/sustain", 0.0, 1.0, sustain);
                    k[7] = knob("RELEASE", "volume_envelope/release", 0.0, 1.0, ms(r));
                }
                _ => {}
            }
            vec![(0, "Pitch / Gate"), (4, "Amplitude Envelope")]
        }
        2 => {
            k[0] = knob("COMP", "comp", 0.0, 1.0, percent(value("comp")));
            k[1] = knob("DRIVE", "drive", 0.0, 1.0, percent(value("drive")));
            k[2] = knob("SR", "sample_rate", 0.0, 1.0, dsp::lofi_rate_hz(value("sample_rate"), 44_100.0).map_or("Off".into(), hz));
            k[3] = knob("BITS", "bits", 0.0, 1.0, dsp::lofi_bits(value("bits")).map_or("Off".into(), |b| format!("{b:.1}")));
            k[4] = choice("FILTER", "filter", &FILTER);
            let mode = value("filter") as i64;
            match mode {
                1..=3 => {
                    k[5] = knob("CUTOFF", "cutoff", 0.0, 1.0, hz(dsp::cutoff_hz(value("cutoff"), mode)));
                    if mode != 2 {
                        k[6] = knob("RESONANCE", "resonance", 0.0, 1.0, format!("{:.1} %", value("resonance") * 100.0));
                    }
                }
                4 => {
                    k[5] = knob("FREQUENCY", "eq_frequency", 0.0, 1.0, hz(dsp::eq_hz(value("eq_frequency"))));
                    let bw = 2f64.powf(value("eq_width") * 8.0 / 3.0 + 1.0 / 3.0);
                    k[6] = knob("WIDTH", "eq_width", 0.0, 1.0, format!("{:.2} oct", bw.log2()));
                    k[7] = knob("GAIN", "eq_gain", -1.0, 1.0, format!("{:+.1} dB", value("eq_gain") * 18.0));
                }
                _ => {}
            }
            vec![(0, "FX"), (4, "Filter")]
        }
        3 => {
            let (a, h, d, r) = env_ms("mod_envelope");
            let sustain = percent(value("mod_envelope/sustain").powi(3));
            k[0] = knob("ATTACK", "mod_envelope/attack", 0.0, 1.0, ms(a));
            if env_type == 2 {
                k[1] = knob("DECAY", "mod_envelope/decay", 0.0, 1.0, ms(d));
                k[2] = knob("SUSTAIN", "mod_envelope/sustain", 0.0, 1.0, sustain);
                k[3] = knob("RELEASE", "mod_envelope/release", 0.0, 1.0, ms(r));
            } else {
                k[1] = knob("HOLD", "mod_envelope/hold", 0.0, 1.0, ms(h));
                k[2] = knob("DECAY", "mod_envelope/decay", 0.0, 1.0, ms(d));
            }
            for (i, (label, path)) in [
                ("PITCH", "mod_envelope/pitch"),
                ("CUTOFF", "mod_envelope/cutoff"),
                ("DRIVE", "mod_envelope/drive"),
                ("PAN", "mod_envelope/pan"),
            ]
            .into_iter()
            .enumerate()
            {
                k[4 + i] = knob(label, path, -1.0, 1.0, amount(value(path)));
            }
            vec![(0, "Modulation Envelope"), (4, "Destination")]
        }
        4 => {
            k[0] = choice("TYPE", "lfo/type", &LFO_TYPE);
            let sync = value("lfo/sync_mode") as i64;
            let (free, (num, den)) = dsp::lfo_rate(value("lfo/speed"));
            k[1] = knob("SPEED", "lfo/speed", 0.0, 1.0, if sync == 0 { format!("{free:.2} Hz") } else { format!("{num}/{den}") });
            k[2] = knob("PHASE", "lfo/phase", -0.5, 0.5, format!("{:.0}°", value("lfo/phase") * 360.0));
            k[3] = choice("SYNC", "lfo/sync_mode", &LFO_SYNC);
            for (i, (label, path)) in
                [("PITCH", "lfo/pitch_amount"), ("CUTOFF", "lfo/cutoff_amount"), ("DRIVE", "lfo/drive_amount"), ("PAN", "lfo/pan_amount")]
                    .into_iter()
                    .enumerate()
            {
                k[4 + i] = knob(label, path, -1.0, 1.0, amount(value(path)));
            }
            vec![(0, "LFO"), (4, "Destination")]
        }
        _ => {
            for (i, (label, path)) in [
                ("START", "velocity_start"),
                ("DECAY", "velocity_decay"),
                ("CUTOFF", "velocity_cutoff"),
                ("VOLUME", "velocity_volume"),
                ("START", "modwheel_start"),
                ("CUTOFF", "modwheel_cutoff"),
            ]
            .into_iter()
            .enumerate()
            {
                k[i] = knob(label, path, -1.0, 1.0, amount(value(path)));
            }
            k[6] = knob("LFO DEPTH", "modwheel_lfo", 0.0, 1.0, percent(value("modwheel_lfo")));
            k[7] = knob("PAN", "modwheel_pan", -1.0, 1.0, amount(value("modwheel_pan")));
            vec![(0, "Velocity Destination"), (4, "Modwheel Destination")]
        }
    };
    Page { slots: k, sections }
}

/// The zone's loop switch, drawn at the waveform's top right; a click toggles it.
pub fn loop_switch(ui: &Ui, rect: Rect, zone: &Zone, target: Target, acts: &mut Vec<Act>) {
    let r = Rect::from_min_size(Pos2::new(rect.right() - 46.0, rect.top() + 4.0), Vec2::new(42.0, 16.0));
    let resp = ui
        .interact(r, ui.id().with("sampler-loop"), Sense::click())
        .on_hover_text("Loop the zone (looping plays only with an AHD or ADSR envelope)");
    let p = ui.painter();
    let on = zone.loop_active;
    p.rect_filled(
        r,
        2.0,
        if on { Color32::from_rgb(0x2a, 0x5a, 0x2a) } else { Color32::from_gray(if resp.hovered() { 0x3c } else { 0x2a }) },
    );
    p.text(
        r.center(),
        Align2::CENTER_CENTER,
        "LOOP",
        egui::FontId::proportional(10.0),
        if on { Color32::WHITE } else { Color32::from_white_alpha(160) },
    );
    if resp.clicked() {
        acts.push(Act::SetParam(target, "loop_active", if on { 0.0 } else { 1.0 }));
    }
}

/// The focused zone's waveform with its play range and loop, and the playhead
/// (`playhead`: the frame playing); drag a marker to move it.
#[allow(clippy::too_many_arguments)]
pub fn waveform(
    ui: &Ui,
    rect: Rect,
    wf: &Waveform,
    zone: &Zone,
    target: Target,
    color: Color32,
    playhead: Option<usize>,
    acts: &mut Vec<Act>,
) {
    ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-waveform"), rect));
    let painter = &ui.painter_at(rect.expand(1.0));
    painter.rect_filled(rect, 2.0, Color32::from_gray(0x12));
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0, Color32::from_gray(0x2a)), egui::StrokeKind::Inside);
    let len = wf.len.max(1) as f32;
    let x_of = |f: i64| rect.left() + rect.width() * (f as f32 / len).clamp(0.0, 1.0);
    let (lo, hi) = if zone.play_range.1 > zone.play_range.0 { zone.play_range } else { (0, wf.len as i64 - 1) };
    let mid = rect.center().y;
    let h = rect.height() * 0.45;
    // The stored peaks, merged into one column per pixel.
    let n = wf.peaks.len();
    let px = rect.width().max(1.0) as usize;
    let cols: Vec<WaveCol> = (0..px)
        .filter_map(|c| {
            let (a, b) = (c * n / px, ((c + 1) * n / px).max(c * n / px + 1).min(n));
            if a >= n {
                return None;
            }
            let (mn, mx) = wf.peaks[a..b].iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.0), hi.max(p.1)));
            let frame = (c as f32 / px as f32 * len) as i64;
            let inside = (lo..=hi).contains(&frame);
            Some(WaveCol {
                x: rect.left() + c as f32 + 0.5,
                top: mid - mx.clamp(-1.0, 1.0) * h,
                bottom: mid - mn.clamp(-1.0, 1.0) * h,
                color: if inside { color } else { color.gamma_multiply(0.3) },
            })
        })
        .collect();
    paint_wave(painter, &cols);
    if zone.loop_active && zone.loop_range.1 > zone.loop_range.0 {
        let r = Rect::from_min_max(Pos2::new(x_of(zone.loop_range.0), rect.top()), Pos2::new(x_of(zone.loop_range.1), rect.bottom()));
        painter.rect_filled(r, 0.0, Color32::from_white_alpha(18));
        for x in [r.left(), r.right()] {
            painter.vline(x, rect.y_range(), Stroke::new(1.0, Color32::from_rgb(0x53, 0xff, 0x53)));
        }
    }
    for x in [x_of(lo), x_of(hi)] {
        painter.vline(x, rect.y_range(), Stroke::new(1.5, Color32::WHITE));
    }
    if let Some(f) = playhead {
        painter.vline(x_of(f as i64), rect.shrink(1.0).y_range(), Stroke::new(1.0, Color32::from_rgb(0xff, 0xd8, 0x40)));
        ui.ctx().request_repaint();
    }
    // Drag the nearest marker.
    let resp = ui
        .interact(rect, ui.id().with("sampler-waveform"), Sense::drag())
        .on_hover_text("Drag the play start/end (white) or loop (green) markers");
    let id = ui.id().with("sampler-waveform-marker");
    if resp.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin())
    {
        let mut marks = vec![("play_range/low", x_of(lo)), ("play_range/high", x_of(hi))];
        if zone.loop_active {
            marks.push(("loop_range/low", x_of(zone.loop_range.0)));
            marks.push(("loop_range/high", x_of(zone.loop_range.1)));
        }
        let near = marks.into_iter().min_by(|a, b| (a.1 - p.x).abs().total_cmp(&(b.1 - p.x).abs())).filter(|m| (m.1 - p.x).abs() < 12.0);
        ui.data_mut(|d| d.insert_temp::<Option<&'static str>>(id, near.map(|m| m.0)));
    }
    if resp.dragged()
        && let Some(Some(field)) = ui.data(|d| d.get_temp::<Option<&'static str>>(id))
        && let Some(p) = resp.interact_pointer_pos()
    {
        let frame = (((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * (len - 1.0)).round() as f64;
        acts.push(Act::SetParam(target, field, frame));
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
}

/// One column of a drawn waveform: its x, the top and bottom of its envelope (screen y)
/// and its colour.
#[derive(Clone, Copy)]
pub struct WaveCol {
    pub x: f32,
    pub top: f32,
    pub bottom: f32,
    pub color: Color32,
}

/// Catmull-Rom spline through `p1` (t = 0) and `p2` (t = 1): a smooth curve that passes
/// through every point.
pub fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let (t2, t3) = (t * t, t * t * t);
    0.5 * (2.0 * p1 + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
}

/// Points between the columns, on Catmull-Rom curves through the columns' tops and bottoms
/// (about one every half pixel; none added where columns are closer than that).
fn curve(cols: &[WaveCol], top: &[f32], bottom: &[f32]) -> Vec<WaveCol> {
    let n = cols.len();
    let spacing = (cols[n - 1].x - cols[0].x).abs() / (n - 1) as f32;
    let sub = ((spacing * 2.0).ceil() as usize).clamp(1, 64);
    let at = |v: &[f32], i: isize| v[i.clamp(0, n as isize - 1) as usize];
    let mut out = Vec::with_capacity(n * sub);
    for i in 0..n - 1 {
        let ii = i as isize;
        for s in 0..sub {
            let t = s as f32 / sub as f32;
            let y = |v: &[f32]| catmull_rom(at(v, ii - 1), at(v, ii), at(v, ii + 1), at(v, ii + 2), t);
            out.push(WaveCol {
                x: cols[i].x + (cols[i + 1].x - cols[i].x) * t,
                top: y(top),
                bottom: y(bottom),
                color: if t < 0.5 { cols[i].color } else { cols[i + 1].color },
            });
        }
    }
    out.push(WaveCol { top: top[n - 1], bottom: bottom[n - 1], ..cols[n - 1] });
    out
}

/// Paint a waveform envelope as one filled shape: the outline is lightly smoothed and
/// drawn as curves through the columns, with anti-aliased top and bottom edges.
pub fn paint_wave(painter: &egui::Painter, cols: &[WaveCol]) {
    if cols.len() < 2 {
        return;
    }
    // Smooth the outline ([1 2 1] / 4), so dense audio doesn't draw a ragged edge.
    let n = cols.len();
    let smooth = |f: fn(&WaveCol) -> f32| -> Vec<f32> {
        (0..n).map(|i| (f(&cols[i.saturating_sub(1)]) + 2.0 * f(&cols[i]) + f(&cols[(i + 1).min(n - 1)])) * 0.25).collect()
    };
    let pts = curve(cols, &smooth(|c| c.top), &smooth(|c| c.bottom));
    let mut mesh = egui::Mesh::default();
    for (i, c) in pts.iter().enumerate() {
        let (mut top, mut bottom) = (c.top, c.bottom);
        // At least a pixel thick: a quiet part stays visible.
        if bottom - top < 1.2 {
            let mid = (top + bottom) * 0.5;
            (top, bottom) = (mid - 0.6, mid + 0.6);
        }
        // Feathered edges: transparent half a pixel outside, full colour half a pixel in.
        let inner = ((top + 0.5).min((top + bottom) * 0.5), (bottom - 0.5).max((top + bottom) * 0.5));
        for (y, col) in [(top - 0.5, Color32::TRANSPARENT), (inner.0, c.color), (inner.1, c.color), (bottom + 0.5, Color32::TRANSPARENT)] {
            mesh.colored_vertex(Pos2::new(c.x, y), col);
        }
        if i > 0 {
            let (p, q) = ((i as u32 - 1) * 4, i as u32 * 4);
            for k in 0..3 {
                mesh.add_triangle(p + k, p + k + 1, q + k);
                mesh.add_triangle(q + k, p + k + 1, q + k + 1);
            }
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Paint the signal itself (one column per frame, its `top` the value) as an anti-aliased
/// line, curved between the frames when they are far enough apart.
pub fn paint_signal(painter: &egui::Painter, cols: &[WaveCol], width: f32) {
    if cols.len() < 2 {
        return;
    }
    let ys: Vec<f32> = cols.iter().map(|c| c.top).collect();
    let pts = curve(cols, &ys, &ys);
    // One line per run of a colour, overlapping by a point so the runs join.
    let mut run: Vec<Pos2> = Vec::new();
    for (i, c) in pts.iter().enumerate() {
        run.push(Pos2::new(c.x, c.top));
        let last = i + 1 == pts.len();
        if last || pts[i + 1].color != c.color {
            painter.add(egui::Shape::line(std::mem::take(&mut run), Stroke::new(width, c.color)));
            run.push(Pos2::new(c.x, c.top));
        }
    }
}
