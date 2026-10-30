//! Link and MIDI clock in the app (docs/superpowers/plans/2026-10-23-phase9-link-
//! midi-clock.md): the Link instance and the MIDI clock follower, and each frame what
//! goes between them and the song — the tempo both ways, starting and stopping (Start
//! Stop Sync, an external clock's Start / Continue / Stop), the clock the audio output
//! follows, the MIDI clock it sends, and what the MK3's Settings ▸ MIDI page shows.

use crate::settings::MidiSync;
use deckard_engine::sync::{Clock, ClockFollower, ClockOut, Follow, Transport};
use deckard_model::PPQ;
use eframe::egui;
use std::sync::Arc;
use std::time::Duration;

/// How late (beats) a start the Link session made may be heard and still start the
/// song at once, caught up (else it waits for the next bar).
const LATE_START: f64 = 0.5;

/// The smallest change of an outside tempo that changes the song's: Link's is exact;
/// MIDI clock's wavers (it's measured).
const LINK_STEP: f64 = 0.005;
const MIDI_STEP: f64 = 0.05;

#[derive(Default)]
pub struct Sync {
    /// Made when Link is first switched on (it goes on the network only then).
    link: Option<deckard_link::Link>,
    /// Follows incoming MIDI clock (fed by the MIDI inputs while receiving).
    pub follower: Arc<ClockFollower>,
    /// The clock handed to the audio output, and which output (its address).
    handed: Option<(Source, usize)>,
    /// The tempo last agreed with the outside clock.
    agreed: Option<f64>,
    /// The Link session's transport and ours, as last seen (Start Stop Sync).
    seen: Option<(bool, bool)>,
    /// How late (beats) the start being made may be and still start at once.
    late: f64,
    /// Link is a session of one that never goes on the network (tests).
    pub local_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Link,
    Midi,
}

impl Sync {
    /// The Link instance, once made.
    pub fn link(&self) -> Option<&deckard_link::Link> {
        self.link.as_ref()
    }
}

/// Link's session as the audio thread follows it.
struct LinkClock(deckard_link::AudioClock);

impl Clock for LinkClock {
    fn follow(&mut self, ahead: Duration, quantum: f64) -> Option<Follow> {
        let m = self.0.at(ahead.as_micros() as i64, quantum);
        Some(Follow { tempo: m.tempo, beat: m.beat, absolute: false })
    }
}

impl crate::App {
    /// LINK (the header's button, Preferences, the MK3): join or leave the session.
    /// Receiving MIDI clock stops: Link takes priority.
    pub fn toggle_link(&mut self) {
        self.settings.link = !self.settings.link;
        if self.settings.link && self.settings.midi_sync == MidiSync::Receive {
            self.settings.midi_sync = MidiSync::Off;
        }
        self.settings.save();
    }

    /// MIDI clock comes in and leads (Receive Clock, Link off).
    pub fn receiving_clock(&self) -> bool {
        self.settings.midi_sync == MidiSync::Receive && !self.settings.link
    }

    /// Link as the header shows it: joined, peers, and the phase in the bar (0..1).
    pub fn link_view(&self) -> Option<(u64, f64)> {
        let link = self.sync.link.as_ref().filter(|_| self.settings.link)?;
        let q = self.quantum();
        Some((link.peers(), link.phase(q) / q))
    }

    /// The song's bar in beats: Link's quantum.
    fn quantum(&self) -> f64 {
        self.doc.as_ref().map_or(4.0, |d| {
            let p = d.borrow();
            let p = p.project();
            p.numerator.max(1) as f64 * 4.0 / p.denominator.max(1) as f64
        })
    }

    /// A song about to play starts in time with the clock followed, if any: how late
    /// it may start (see `Engine::sync_start_within`).
    pub fn sync_start(&self) -> Option<f64> {
        (self.settings.link || self.receiving_clock()).then_some(self.sync.late)
    }

    /// The MIDI clock to send: Send Clock, to the outputs switched on.
    pub fn clock_out(&mut self) -> Option<ClockOut> {
        if self.settings.midi_sync != MidiSync::Send {
            return None;
        }
        let outs = self.midi_ports().1.clone();
        // (Not a MIDI controller's own port: the Launchpad takes clock on its MIDI port.)
        let ports = outs
            .into_iter()
            .filter(|n| !self.settings.midi_off.contains(&format!("out:{n}")) && !self.midi_devices.exclusive(n))
            .map(|n| n.as_str().into())
            .collect();
        Some(ClockOut { ports, early: Duration::from_millis(self.settings.clock_send_offset_ms as u64) })
    }

    /// Each frame: Link, the clock in and out, the tempo, the transport, the MK3.
    pub fn sync_clocks(&mut self, ctx: &egui::Context) {
        self.sync_link();
        let receiving = self.receiving_clock();
        self.midi.set_clock_follower(receiving.then(|| self.sync.follower.clone()));
        self.sync.follower.set_offset(self.settings.clock_receive_offset_ms as f64 / 1000.0);
        self.follow_tempo(receiving);
        if receiving {
            self.clock_transport();
        }
        self.hand_clock(receiving);
        let out = self.clock_out();
        if let Some(Ok(o)) = &self.output {
            o.set_clock_out(out);
        }
        self.share_sync();
        if self.settings.link {
            // The phase bar moves; the session may start.
            ctx.request_repaint_after(Duration::from_millis(33));
        } else if receiving {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// Link on or off as set; Start Stop Sync both ways.
    fn sync_link(&mut self) {
        if !self.settings.link {
            if let Some(l) = &self.sync.link {
                l.enable(false);
            }
            self.sync.seen = None;
            return;
        }
        let tempo = self.doc.as_ref().map_or(120.0, |d| d.borrow().project().tempo);
        let local = self.sync.local_only;
        let link =
            self.sync.link.get_or_insert_with(|| if local { deckard_link::Link::local(tempo) } else { deckard_link::Link::new(tempo) });
        if !link.is_enabled() {
            // Alone, the session takes the song's tempo; joining one, the song its.
            link.set_tempo(tempo);
            link.enable(true);
            self.sync.agreed = None;
        }
        link.set_start_stop_sync(self.settings.link_start_stop);
        if !self.settings.link_start_stop {
            self.sync.seen = None;
            return;
        }
        let session = link.is_playing();
        let ours = self.playing.is_some();
        match self.sync.seen {
            Some((s0, _)) if session != s0 => {
                // The session started or stopped.
                if session && !ours {
                    self.play_synced(LATE_START);
                } else if !session && ours {
                    self.stop();
                }
                self.sync_transport();
            }
            Some((_, o0)) if ours != o0 && ours != session => link.set_playing(ours),
            _ => {}
        }
        let session = self.sync.link.as_ref().is_some_and(|l| l.is_playing());
        self.sync.seen = Some((session, self.playing.is_some()));
    }

    /// PLAY, starting in time with the clock (and catching up when up to `late` beats
    /// behind it).
    fn play_synced(&mut self, late: f64) {
        self.sync.late = late;
        self.transport_play();
        self.sync.late = 0.0;
    }

    /// The song's tempo and the outside clock's: a change here goes to Link; Link's or
    /// the MIDI clock's becomes the song's (not an edit: no undo step, nothing to save).
    fn follow_tempo(&mut self, receiving: bool) {
        let Some(doc) = self.doc.clone() else { return };
        let ours = doc.borrow().project().tempo;
        let theirs = match (&self.sync.link, receiving) {
            (Some(l), false) if self.settings.link => Some((l.tempo(), LINK_STEP)),
            (_, true) => self.sync.follower.tempo().map(|t| (t, MIDI_STEP)),
            _ => None,
        };
        let Some((theirs, step)) = theirs else {
            self.sync.agreed = None;
            return;
        };
        if self.sync.agreed.is_some_and(|a| a != ours) && !receiving {
            // Changed here (the header, the MK3, undo): the session takes it.
            if let Some(l) = &self.sync.link {
                l.set_tempo(ours);
            }
            self.sync.agreed = Some(ours);
        } else if (theirs - ours).abs() >= step {
            let bpm = (theirs * 100.0).round() / 100.0;
            let value = mxprj::decode::Value::Float(bpm.clamp(5.0, 999.0));
            if doc.borrow_mut().set_initial(deckard_doc::Target::Song, "tempo", value).is_ok() {
                self.sync.agreed = Some(doc.borrow().project().tempo);
            }
        } else {
            self.sync.agreed = Some(ours);
        }
    }

    /// The external clock's Start, Continue and Stop.
    fn clock_transport(&mut self) {
        let Some(t) = self.sync.follower.take_transport() else { return };
        let mode = if self.song_view { deckard_engine::Mode::Arrangement } else { deckard_engine::Mode::Scene(self.scene) };
        match t {
            Transport::Start(beat) | Transport::Continue(beat) => {
                // (No count-in: the clock says where the song is.)
                self.count_in_next = 0;
                self.play_from(mode, (beat * PPQ as f64).round() as i64);
            }
            Transport::Stop => self.stop(),
        }
        self.sync_transport();
    }

    /// Give the audio output the clock to follow (Link's session, the MIDI clock), or
    /// none; again when the output was opened anew.
    fn hand_clock(&mut self, receiving: bool) {
        let Some(Ok(out)) = &self.output else { return };
        let id = out as *const _ as usize;
        let want = if self.settings.link && self.sync.link.is_some() {
            Some(Source::Link)
        } else if receiving {
            Some(Source::Midi)
        } else {
            None
        };
        let key = want.map(|s| (s, id));
        if self.sync.handed == key {
            return;
        }
        let had = self.sync.handed.is_some_and(|(_, o)| o == id);
        self.sync.handed = key;
        match want {
            Some(Source::Link) => {
                let clock = self.sync.link.as_ref().map(|l| l.audio_clock());
                out.set_clock(clock.map(|c| Box::new(LinkClock(c)) as Box<dyn Clock>));
            }
            Some(Source::Midi) => out.set_clock(Some(self.sync.follower.clock())),
            None if had => out.set_clock(None),
            None => {}
        }
    }

    /// Preferences ▸ MIDI's Sync (Mode, Clock Offset) and Link (Enabled, Start Stop
    /// Sync), as Maschine has them.
    pub fn sync_preferences(&mut self, ui: &mut egui::Ui) {
        let link = self.settings.link;
        ui.label(egui::RichText::new("Sync").strong());
        egui::Grid::new("midi-sync").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            let s = &mut self.settings;
            ui.label("Mode");
            let name = |m: MidiSync| match m {
                MidiSync::Off => "Off",
                MidiSync::Send => "Send Clock",
                MidiSync::Receive => "Receive Clock",
            };
            egui::ComboBox::from_id_salt("midi-sync-mode").selected_text(name(s.midi_sync)).show_ui(ui, |ui| {
                for m in [MidiSync::Off, MidiSync::Send, MidiSync::Receive] {
                    let r = ui.add_enabled_ui(m != MidiSync::Receive || !link, |ui| ui.selectable_value(&mut s.midi_sync, m, name(m)));
                    r.response.on_disabled_hover_text("Not while Link is on: Link takes priority");
                }
            });
            ui.end_row();
            match s.midi_sync {
                MidiSync::Off => {}
                MidiSync::Send => {
                    ui.label("Clock Offset");
                    let mut ms = -(s.clock_send_offset_ms as i32);
                    ui.add(egui::Slider::new(&mut ms, -200..=0).suffix(" ms"))
                        .on_hover_text("Clock sent this much early, for devices that answer late");
                    s.clock_send_offset_ms = (-ms).clamp(0, 200) as u32;
                    ui.end_row();
                }
                MidiSync::Receive => {
                    ui.label("Clock Offset");
                    ui.add(egui::Slider::new(&mut s.clock_receive_offset_ms, -100..=100).suffix(" ms"))
                        .on_hover_text("The song plays this much after the clock (before it, negative)");
                    ui.end_row();
                }
            }
            ui.label("");
            ui.label(
                egui::RichText::new(match s.midi_sync {
                    MidiSync::Off => "No MIDI clock is sent or followed.",
                    MidiSync::Send => "MIDI clock, Start, Stop and Continue go to the outputs switched on below.",
                    MidiSync::Receive => "The inputs switched on below lead: their clock sets the tempo and starts and stops the song.",
                })
                .weak(),
            );
            ui.end_row();
        });
        ui.separator();
        ui.label(egui::RichText::new("Link").strong());
        let mut on = link;
        if ui.checkbox(&mut on, "Enabled").on_hover_text("The LINK button in the header").changed() {
            self.toggle_link();
        }
        ui.checkbox(&mut self.settings.link_start_stop, "Start Stop Sync")
            .on_hover_text("Start and stop with the other applications in the session that do too");
        let status = if !deckard_link::AVAILABLE {
            "Ableton Link isn't built into this version of DECKARD.".to_string()
        } else {
            match self.sync.link().filter(|_| self.settings.link).map(|l| l.peers()) {
                None => "Not in a session.".to_string(),
                Some(0) => "In the session, alone so far.".to_string(),
                Some(1) => "In a session with 1 other application.".to_string(),
                Some(n) => format!("In a session with {n} other applications."),
            }
        };
        ui.label(egui::RichText::new(status).weak());
        ui.separator();
    }

    /// The MK3's Settings ▸ MIDI page: what the app has now.
    fn share_sync(&mut self) {
        let (ins, outs) = self.midi_ports().clone();
        let view = |prefix: &str, ports: &[String]| -> Vec<(String, bool)> {
            ports.iter().map(|n| (n.clone(), !self.settings.midi_off.contains(&format!("{prefix}{n}")))).collect()
        };
        let (peers, phase) = self.link_view().unwrap_or((0, 0.0));
        let s = &self.settings;
        let v = deckard_lua_host::state::SyncView {
            link: s.link,
            link_start_stop: s.link_start_stop,
            peers,
            phase,
            midi_sync: match s.midi_sync {
                MidiSync::Off => 0,
                MidiSync::Send => 1,
                MidiSync::Receive => 2,
            },
            send_offset: s.clock_send_offset_ms as i64,
            receive_offset: s.clock_receive_offset_ms as i64 + 100,
            inputs: view("in:", &ins),
            outputs: view("out:", &outs),
            default_focus: s.sound_midi_focus,
        };
        if let Some(Ok(c)) = &mut self.controller {
            c.set_sync(v);
        }
    }

    /// The MK3's Settings ▸ MIDI page changed Link or MIDI sync: the app's settings.
    pub fn sync_from_controller(&mut self) {
        let Some(Ok(c)) = &self.controller else { return };
        let v = c.sync_view();
        let before = self.settings.clone();
        let s = &mut self.settings;
        if v.link != s.link {
            self.toggle_link();
        }
        let s = &mut self.settings;
        s.link_start_stop = v.link_start_stop;
        s.midi_sync = match v.midi_sync {
            1 => MidiSync::Send,
            2 if !s.link => MidiSync::Receive,
            _ => MidiSync::Off,
        };
        s.clock_send_offset_ms = v.send_offset.clamp(0, 200) as u32;
        s.clock_receive_offset_ms = (v.receive_offset - 100).clamp(-100, 100) as i32;
        s.sound_midi_focus = v.default_focus;
        for (prefix, ports) in [("in:", &v.inputs), ("out:", &v.outputs)] {
            for (name, on) in ports {
                let key = format!("{prefix}{name}");
                if *on {
                    s.midi_off.remove(&key);
                } else {
                    s.midi_off.insert(key);
                }
            }
        }
        if self.settings != before {
            self.settings.save();
        }
    }
}
