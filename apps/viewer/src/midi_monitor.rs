//! The MIDI Monitor (View ▸ MIDI Monitor): what came in on each input (the MK3's pads
//! too) and what DECKARD sent to each output, as two running lists. It logs nothing
//! while its window is closed.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Rows each list keeps (the oldest go).
pub const KEEP: usize = 5000;

/// The source the MK3's pads show as.
pub const PADS: &str = "MK3 pads";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    In,
    Out,
}

/// What kind of event a row is (the type filter, and its colour).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Note,
    Control,
    Aftertouch,
    PitchBend,
    Mpe,
    Program,
    Other,
}

impl Kind {
    pub const ALL: [Kind; 7] = [Kind::Note, Kind::Control, Kind::Aftertouch, Kind::PitchBend, Kind::Mpe, Kind::Program, Kind::Other];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Note => "Notes",
            Kind::Control => "CC",
            Kind::Aftertouch => "Aftertouch",
            Kind::PitchBend => "Pitch bend",
            Kind::Mpe => "MPE",
            Kind::Program => "Program",
            Kind::Other => "Other",
        }
    }
}

/// One event as the monitor shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Seconds since the monitor started.
    pub at: f64,
    pub port: Arc<str>,
    /// 1–16 (`None`: not MIDI, a pad).
    pub ch: Option<u8>,
    pub kind: Kind,
    pub what: &'static str,
    pub data: String,
    pub value: String,
}

/// A pad event, as it played.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pad {
    On(i64),
    Off,
    /// Pressure setting Note Repeat's velocity (the Velocity pad mode), 1..=127.
    Pressure(u8),
    /// Poly aftertouch sent, 0..=127.
    PolyAt(u8),
    /// Channel aftertouch or a CC sent for the pad's sound.
    Control(deckard_model::MidiKind),
}

/// What a MIDI message shows as: (channel, kind, what, data, value). `mpe`: the input
/// is an MPE controller with this member bend range, and `held` the note last started
/// on the message's channel (for its member channels' expression).
pub fn describe(bytes: &[u8], mpe: Option<u8>, held: Option<u8>) -> Option<(Option<u8>, Kind, &'static str, String, String)> {
    let (&status, rest) = bytes.split_first()?;
    let b = |i: usize| rest.get(i).copied().unwrap_or(0);
    if status >= 0xf0 {
        let what = match status {
            0xf0 => "SysEx",
            0xf2 => "Song Position",
            0xfa => "Start",
            0xfb => "Continue",
            0xfc => "Stop",
            _ => "System",
        };
        return Some((None, Kind::Other, what, String::new(), String::new()));
    }
    let ch = status & 0x0f;
    let note = |n: u8| format!("{} ({n})", crate::keys::note_name(n as i64));
    // An MPE member channel's expression: the note held on it.
    let member = mpe.filter(|_| ch != 0);
    let held_note = || held.map(note).unwrap_or_default();
    let (kind, what, data, value) = match status & 0xf0 {
        0x90 if b(1) > 0 => (Kind::Note, "Note On", note(b(0)), b(1).to_string()),
        0x80 | 0x90 => (Kind::Note, "Note Off", note(b(0)), b(1).to_string()),
        0xa0 => (Kind::Aftertouch, "Poly AT", note(b(0)), b(1).to_string()),
        0xb0 if member.is_some() && b(0) == 74 => (Kind::Mpe, "MPE Slide", held_note(), b(1).to_string()),
        0xb0 => (Kind::Control, "CC", deckard_model::pad_touch::cc_label(b(0)), b(1).to_string()),
        0xc0 => (Kind::Program, "Program", String::new(), (b(0) as u16 + 1).to_string()),
        0xd0 if member.is_some() => (Kind::Mpe, "MPE Pressure", held_note(), b(0).to_string()),
        0xd0 => (Kind::Aftertouch, "Channel AT", String::new(), b(0).to_string()),
        _ => {
            let v = ((b(1) as i32) << 7 | b(0) as i32) - 8192;
            match member {
                Some(range) => (Kind::Mpe, "MPE Pitch", held_note(), format!("{:+.2} st", v as f32 / 8192.0 * range as f32)),
                None if v > 0 => (Kind::PitchBend, "Pitch Bend", String::new(), format!("+{v}")),
                None => (Kind::PitchBend, "Pitch Bend", String::new(), v.to_string()),
            }
        }
    };
    Some((Some(ch + 1), kind, what, data, value))
}

/// A pad event: (kind, what, data, value), `pitch` the note it plays.
pub fn describe_pad(pad: usize, pitch: Option<i64>, ev: Pad) -> (Kind, &'static str, String, String) {
    let data = match pitch {
        Some(p) => format!("Pad {pad} · {}", crate::keys::note_name(p)),
        None => format!("Pad {pad}"),
    };
    match ev {
        Pad::On(v) => (Kind::Note, "Note On", data, v.to_string()),
        Pad::Off => (Kind::Note, "Note Off", data, String::new()),
        Pad::Pressure(v) => (Kind::Aftertouch, "Pad pressure", data, v.to_string()),
        Pad::PolyAt(v) => (Kind::Aftertouch, "Pad Poly AT", data, v.to_string()),
        Pad::Control(deckard_model::MidiKind::ChannelPressure(v)) => (Kind::Aftertouch, "Pad AT", data, v.to_string()),
        Pad::Control(deckard_model::MidiKind::Controller { number, value }) => {
            (Kind::Control, "Pad CC", format!("{data} · {}", deckard_model::pad_touch::cc_label(number)), value.to_string())
        }
        Pad::Control(_) => (Kind::Other, "Pad", data, String::new()),
    }
}

/// The monitor's log, shared by the MIDI threads, the MK3's and the window.
#[derive(Clone, Default)]
pub struct Monitor {
    open: Arc<AtomicBool>,
    log: Arc<Mutex<Log>>,
}

#[derive(Default)]
struct Log {
    start: Option<Instant>,
    ins: VecDeque<Row>,
    outs: VecDeque<Row>,
    /// The note last started on each MPE input's channel.
    held: HashMap<(Arc<str>, u8), u8>,
    repaint: Option<eframe::egui::Context>,
}

impl Log {
    fn push(&mut self, dir: Dir, port: &Arc<str>, ch: Option<u8>, (kind, what, data, value): (Kind, &'static str, String, String)) {
        let at = self.start.get_or_insert_with(Instant::now).elapsed().as_secs_f64();
        let rows = match dir {
            Dir::In => &mut self.ins,
            Dir::Out => &mut self.outs,
        };
        if rows.len() >= KEEP {
            rows.pop_front();
        }
        rows.push_back(Row { at, port: port.clone(), ch, kind, what, data, value });
        if let Some(ctx) = &self.repaint {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
}

impl Monitor {
    /// The window is open (logging) or not.
    pub fn set_open(&self, open: bool) {
        self.open.store(open, Ordering::Relaxed);
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Relaxed)
    }

    pub fn set_context(&self, ctx: &eframe::egui::Context) {
        let mut l = self.log.lock().unwrap();
        if l.repaint.is_none() {
            l.repaint = Some(ctx.clone());
        }
    }

    /// A message that came in on, or went out to, `port` (`mpe`: an MPE input's member
    /// bend range). MIDI clock and active sensing aren't shown.
    pub fn midi(&self, dir: Dir, port: &Arc<str>, bytes: &[u8], mpe: Option<u8>) {
        if !self.is_open() || matches!(bytes.first(), Some(0xf8 | 0xfe) | None) {
            return;
        }
        let mut l = self.log.lock().unwrap();
        let ch = bytes[0] & 0x0f;
        let key = (port.clone(), ch);
        let held = l.held.get(&key).copied();
        let Some((ch, kind, what, data, value)) = describe(bytes, mpe, held) else { return };
        if mpe.is_some() && kind == Kind::Note {
            let n = bytes.get(1).copied().unwrap_or(0);
            if what == "Note On" {
                l.held.insert(key, n);
            } else if held == Some(n) {
                l.held.remove(&key);
            }
        }
        l.push(dir, port, ch, (kind, what, data, value));
    }

    /// A pad (1–16) played: on the input side, as [`PADS`].
    pub fn pad(&self, pad: usize, pitch: Option<i64>, ev: Pad) {
        if !self.is_open() {
            return;
        }
        let port: Arc<str> = PADS.into();
        self.log.lock().unwrap().push(Dir::In, &port, None, describe_pad(pad, pitch, ev));
    }

    /// One side's rows, oldest first.
    #[cfg(test)]
    pub fn rows(&self, dir: Dir) -> Vec<Row> {
        self.view(dir, |r| r.iter().cloned().collect())
    }

    /// Look at one side's rows (oldest first) without copying them.
    pub fn view<T>(&self, dir: Dir, f: impl FnOnce(&VecDeque<Row>) -> T) -> T {
        let l = self.log.lock().unwrap();
        f(match dir {
            Dir::In => &l.ins,
            Dir::Out => &l.outs,
        })
    }

    pub fn clear(&self) {
        let mut l = self.log.lock().unwrap();
        l.ins.clear();
        l.outs.clear();
    }
}

/// The window's choices: the type shown (`None` all), each side's port (`None` all), and
/// whether it's paused (logging nothing).
#[derive(Debug, Clone, Default)]
pub struct View {
    pub kind: Option<Kind>,
    pub port_in: Option<String>,
    pub port_out: Option<String>,
    pub paused: bool,
}

fn colour(k: Kind) -> eframe::egui::Color32 {
    use eframe::egui::Color32;
    match k {
        Kind::Note => Color32::from_rgb(155, 227, 143),
        Kind::Control => Color32::from_rgb(230, 210, 122),
        Kind::Aftertouch => Color32::from_rgb(232, 143, 216),
        Kind::PitchBend => Color32::from_rgb(143, 190, 232),
        Kind::Mpe => Color32::from_rgb(143, 224, 216),
        Kind::Program => Color32::from_rgb(232, 180, 143),
        Kind::Other => Color32::GRAY,
    }
}

/// The MIDI Monitor window: In and Out side by side, each with its source or
/// destination, a type filter, Pause and Clear. `open` goes false when it's closed.
pub fn window(ctx: &eframe::egui::Context, open: &mut bool, m: &Monitor, v: &mut View) {
    use eframe::egui;
    m.set_context(ctx);
    egui::Window::new("MIDI Monitor").open(open).resizable(true).default_size([980.0, 420.0]).show(ctx, |ui| {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("monitor-kind").selected_text(v.kind.map_or("All types", Kind::label)).show_ui(ui, |ui| {
                ui.selectable_value(&mut v.kind, None, "All types");
                for k in Kind::ALL {
                    ui.selectable_value(&mut v.kind, Some(k), k.label());
                }
            });
            let label = if v.paused { "▶ Resume" } else { "⏸ Pause" };
            ui.toggle_value(&mut v.paused, label);
            if ui.button("Clear").clicked() {
                m.clear();
            }
        });
        ui.separator();
        let half = (ui.available_width() - 12.0) / 2.0;
        ui.horizontal_top(|ui| {
            for (dir, title, all) in [(Dir::In, "IN", "All sources"), (Dir::Out, "OUT", "All destinations")] {
                ui.allocate_ui_with_layout(egui::vec2(half, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(half);
                    let pick = match dir {
                        Dir::In => &mut v.port_in,
                        Dir::Out => &mut v.port_out,
                    };
                    side(ui, m, dir, title, all, v.kind, pick);
                });
                if dir == Dir::In {
                    ui.separator();
                }
            }
        });
    });
}

/// One side: its port picker and its rows (newest at the bottom, following unless
/// scrolled up).
fn side(ui: &mut eframe::egui::Ui, m: &Monitor, dir: Dir, title: &str, all: &str, kind: Option<Kind>, pick: &mut Option<String>) {
    use eframe::egui;
    let mut ports: Vec<String> =
        m.view(dir, |rows| rows.iter().map(|r| r.port.to_string()).collect::<std::collections::BTreeSet<_>>()).into_iter().collect();
    if let Some(p) = pick.as_ref()
        && !ports.contains(p)
    {
        ports.push(p.clone());
    }
    ui.horizontal(|ui| {
        ui.strong(title);
        egui::ComboBox::from_id_salt(("monitor-port", title)).selected_text(pick.as_deref().unwrap_or(all)).show_ui(ui, |ui| {
            ui.selectable_value(pick, None, all);
            for p in &ports {
                ui.selectable_value(pick, Some(p.clone()), p);
            }
        });
    });
    let keep = |r: &Row| kind.is_none_or(|k| r.kind == k) && pick.as_deref().is_none_or(|p| *r.port == *p);
    let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
    let widths = [64.0, 0.0, 28.0, 96.0, 112.0, 80.0];
    let fixed: f32 = widths.iter().sum::<f32>() + 6.0 * ui.spacing().item_spacing.x;
    let port_w = (ui.available_width() - fixed).max(60.0);
    let cell = |ui: &mut egui::Ui, w: f32, t: egui::RichText| {
        ui.add_sized([w, row_h], egui::Label::new(t.monospace()).truncate());
    };
    let heads = ["Time", "Port", "Ch", "Type", "Data", "Value"];
    ui.horizontal(|ui| {
        for (i, h) in heads.iter().enumerate() {
            cell(ui, if i == 1 { port_w } else { widths[i] }, egui::RichText::new(*h).weak());
        }
    });
    let n = m.view(dir, |rows| rows.iter().filter(|r| keep(r)).count());
    egui::ScrollArea::vertical().id_salt(("monitor-rows", title)).stick_to_bottom(true).auto_shrink([false, false]).show_rows(
        ui,
        row_h,
        n,
        |ui, range| {
            let shown: Vec<Row> =
                m.view(dir, |rows| rows.iter().filter(|r| keep(r)).skip(range.start).take(range.len()).cloned().collect());
            for r in shown {
                ui.horizontal(|ui| {
                    cell(ui, widths[0], egui::RichText::new(format!("{:.3}", r.at)));
                    cell(ui, port_w, egui::RichText::new(&*r.port));
                    cell(ui, widths[2], egui::RichText::new(r.ch.map_or("–".into(), |c| c.to_string())));
                    cell(ui, widths[3], egui::RichText::new(r.what).color(colour(r.kind)));
                    cell(ui, widths[4], egui::RichText::new(&r.data));
                    cell(ui, widths[5], egui::RichText::new(&r.value));
                });
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(bytes: &[u8]) -> (Option<u8>, Kind, &'static str, String, String) {
        describe(bytes, None, None).unwrap()
    }

    #[test]
    fn messages_show_their_channel_type_and_values() {
        assert_eq!(d(&[0x91, 60, 96]), (Some(2), Kind::Note, "Note On", "C3 (60)".into(), "96".into()));
        // A note-on at velocity 0 is a note-off.
        assert_eq!(d(&[0x90, 61, 0]).2, "Note Off");
        assert_eq!(d(&[0x80, 61, 40]), (Some(1), Kind::Note, "Note Off", "C#3 (61)".into(), "40".into()));
        assert_eq!(d(&[0xa1, 60, 54]), (Some(2), Kind::Aftertouch, "Poly AT", "C3 (60)".into(), "54".into()));
        assert_eq!(d(&[0xd3, 70]), (Some(4), Kind::Aftertouch, "Channel AT", String::new(), "70".into()));
        assert_eq!(d(&[0xb0, 1, 64]), (Some(1), Kind::Control, "CC", "1 Mod".into(), "64".into()));
        assert_eq!(d(&[0xb0, 20, 5]).3, "20");
        assert_eq!(d(&[0xe0, 0, 0x40]), (Some(1), Kind::PitchBend, "Pitch Bend", String::new(), "0".into()));
        assert_eq!(d(&[0xe0, 0x7f, 0x7f]).4, "+8191");
        assert_eq!(d(&[0xe0, 0, 0]).4, "-8192");
        assert_eq!(d(&[0xc5, 9]), (Some(6), Kind::Program, "Program", String::new(), "10".into()));
        assert_eq!(d(&[0xfa]), (None, Kind::Other, "Start", String::new(), String::new()));
    }

    /// On an MPE input a member channel's bend, slide (CC 74) and pressure are the
    /// note held on it; the manager channel (1) is as any other input.
    #[test]
    fn mpe_member_expression_shows_its_note() {
        let m = |b: &[u8]| describe(b, Some(48), Some(64)).unwrap();
        assert_eq!(m(&[0xe2, 0, 0x50]), (Some(3), Kind::Mpe, "MPE Pitch", "E3 (64)".into(), "+12.00 st".into()));
        assert_eq!(m(&[0xb2, 74, 88]), (Some(3), Kind::Mpe, "MPE Slide", "E3 (64)".into(), "88".into()));
        assert_eq!(m(&[0xd2, 30]), (Some(3), Kind::Mpe, "MPE Pressure", "E3 (64)".into(), "30".into()));
        assert_eq!(m(&[0xb2, 1, 9]).1, Kind::Control);
        assert_eq!(m(&[0xe0, 0, 0x50]).1, Kind::PitchBend, "the manager channel");
        assert_eq!(describe(&[0xe2, 0, 0x50], Some(48), None).unwrap().3, "", "no note held");
    }

    #[test]
    fn pads_show_their_pad_and_note() {
        assert_eq!(describe_pad(5, Some(62), Pad::On(100)), (Kind::Note, "Note On", "Pad 5 · D3".into(), "100".into()));
        assert_eq!(describe_pad(5, Some(62), Pad::Off).1, "Note Off");
        assert_eq!(describe_pad(5, Some(62), Pad::Pressure(90)), (Kind::Aftertouch, "Pad pressure", "Pad 5 · D3".into(), "90".into()));
        assert_eq!(describe_pad(7, None, Pad::Off).2, "Pad 7");
    }

    /// What a pad's Pad Touch sends: poly aftertouch on its note, channel aftertouch,
    /// or its CC with the controller's name.
    #[test]
    fn pad_touch_shows_what_it_sends() {
        use deckard_model::MidiKind;
        assert_eq!(describe_pad(5, Some(62), Pad::PolyAt(64)), (Kind::Aftertouch, "Pad Poly AT", "Pad 5 · D3".into(), "64".into()));
        assert_eq!(describe_pad(5, Some(62), Pad::Control(MidiKind::ChannelPressure(30))).1, "Pad AT");
        let cc = describe_pad(5, Some(62), Pad::Control(MidiKind::Controller { number: 1, value: 99 }));
        assert_eq!(cc, (Kind::Control, "Pad CC", "Pad 5 · D3 · 1 Mod".into(), "99".into()));
    }

    /// Closed, it logs nothing; open, each side keeps its newest rows, the MK3's pads
    /// on the input side.
    #[test]
    fn the_monitor_logs_only_while_open() {
        let m = Monitor::default();
        let port: Arc<str> = "Launchpad".into();
        m.midi(Dir::In, &port, &[0x90, 60, 100], None);
        assert!(m.rows(Dir::In).is_empty());
        m.set_open(true);
        m.midi(Dir::In, &port, &[0x90, 60, 100], None);
        m.midi(Dir::Out, &"Hydrasynth".into(), &[0xa1, 60, 40], None);
        m.pad(3, Some(60), Pad::Pressure(12));
        // MIDI clock and active sensing would bury the rest.
        m.midi(Dir::In, &port, &[0xf8], None);
        m.midi(Dir::In, &port, &[0xfe], None);
        let ins = m.rows(Dir::In);
        assert_eq!(ins.iter().map(|r| (&*r.port, r.what)).collect::<Vec<_>>(), vec![("Launchpad", "Note On"), (PADS, "Pad pressure")]);
        assert_eq!(m.rows(Dir::Out).len(), 1);
        for _ in 0..KEEP + 10 {
            m.midi(Dir::Out, &"Hydrasynth".into(), &[0xb0, 1, 1], None);
        }
        assert_eq!(m.rows(Dir::Out).len(), KEEP);
        m.clear();
        assert!(m.rows(Dir::In).is_empty() && m.rows(Dir::Out).is_empty());
    }

    /// An MPE input's member expression finds the note last started on its channel.
    #[test]
    fn the_monitor_follows_mpe_notes() {
        let m = Monitor::default();
        m.set_open(true);
        let port: Arc<str> = "Seaboard".into();
        m.midi(Dir::In, &port, &[0x92, 64, 90], Some(48));
        m.midi(Dir::In, &port, &[0xd2, 30], Some(48));
        m.midi(Dir::In, &port, &[0x82, 64, 0], Some(48));
        m.midi(Dir::In, &port, &[0xd2, 31], Some(48));
        let rows = m.rows(Dir::In);
        assert_eq!(rows[1].data, "E3 (64)");
        assert_eq!(rows[3].data, "", "released");
    }

    /// View ▸ MIDI Monitor opens it, logging; what comes shows in its list, filtered by
    /// type; Pause stops the logging.
    #[test]
    fn the_monitor_window() {
        use egui_kittest::kittest::Queryable;
        let Some(mut h) = crate::tests::harness(false, false) else { return };
        let m = h.state().midi.monitor();
        assert!(!m.is_open(), "closed: logs nothing");
        h.state_mut().menu_commands.push(crate::menu::Command::MidiMonitor);
        h.run_steps(3);
        assert!(h.state().midi_monitor && m.is_open());
        m.midi(Dir::In, &"Launchpad".into(), &[0xa0, 60, 54], None);
        m.midi(Dir::Out, &"Hydrasynth".into(), &[0xb0, 7, 100], None);
        h.run_steps(3);
        assert!(h.query_by_label("Poly AT").is_some());
        assert!(h.query_by_label("7 Volume").is_some());
        h.state_mut().monitor_view.kind = Some(Kind::Aftertouch);
        h.run_steps(3);
        assert!(h.query_by_label("Poly AT").is_some());
        assert!(h.query_by_label("7 Volume").is_none(), "filtered out");
        h.get_by_label("⏸ Pause").click();
        h.run_steps(3);
        assert!(!m.is_open());
    }
}
