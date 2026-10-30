//! The Smart Strip of DECKARD's own controller layer: its bend and mod wheel for the
//! focused sound, its strummed notes and the focused group's Perform FX
//! (`crates/surface/lua/lib/strip.lua`).

use crate::App;
use deckard_engine::{Engine, Live, Mode};
use deckard_model::MidiKind;
use deckard_surface::Action;

impl App {
    /// What the strip plays live, if anything: a bend or the mod wheel of the focused
    /// sound (recorded with REC, as Maschine's), or a strummed note held till it stops.
    pub(crate) fn strip_live(&self, a: Action) -> Option<Live> {
        let (group, sound) = (self.group, self.sound);
        match a {
            Action::Strip("pitch", v) => Some(strip_control(group, sound, true, v)),
            Action::Strip("mod", v) => Some(strip_control(group, sound, false, v)),
            Action::StripNote { sound, pitch, velocity, on } => Some(strip_note(group, sound, pitch, velocity, on)),
            _ => None,
        }
    }

    /// A strip action ([`Action::Strip`], [`Action::StripNote`], [`Action::PerformFx`]).
    pub(crate) fn surface_strip(&mut self, a: Action) {
        let g = self.group;
        match a {
            Action::Strip("perform_touch", pos) => self.perform_strip(g, Some(true), pos),
            Action::Strip("perform", pos) => self.perform_strip(g, None, pos),
            Action::Strip("perform_release", pos) => self.perform_strip(g, Some(false), pos),
            Action::PerformFx => {
                // (One loaded if the group has none; then shown, as Maschine's.)
                if self.perform_slot(g).is_none()
                    && let Some(doc) = &self.doc
                    && let Err(e) = doc.borrow_mut().add_perform_fx(g)
                {
                    self.status = format!("can't add a Perform FX: {e}");
                    return;
                }
                self.control_request(deckard_lua_host::state::ControlRequest::ShowPerformFx(g));
            }
            a => {
                if let Some(ev) = self.strip_live(a) {
                    self.send_live(ev);
                }
            }
        }
    }

    /// Play `ev` now, starting the engine for a note or a control if none runs.
    pub(crate) fn send_live(&mut self, ev: Live) {
        let Some(doc) = self.doc.clone() else { return };
        let Ok(out) = self.output.get_or_insert_with(|| crate::open_output(&self.settings, &mut self.output_fallback)) else { return };
        if out.has_engine() {
            out.live(ev);
        } else if let Live::NoteOn { group, sound, pitch, velocity, .. } = ev {
            let d = doc.borrow();
            let (mut engine, _) = Engine::new(d.project(), self.path.as_deref(), out.rate, Mode::Arrangement, &mut self.samples);
            engine.sequencing = false;
            engine.note_hold(group, sound, pitch, velocity);
            out.play(engine);
            out.note_played(&ev);
        } else if matches!(ev, Live::ControlIn { .. }) {
            // A bend or the mod wheel with nothing playing (BUGS #13): an idle engine
            // takes it, and the notes played next have it.
            let rate = out.rate;
            let engine = self.idle_engine(rate);
            if let Some(Ok(out)) = &self.output {
                out.play(engine);
                out.live(ev);
            }
        }
    }
}

/// The strip's bend (`pitch`, -1..1) or mod wheel (0..1) of a sound, recorded with REC
/// as Maschine's.
pub(crate) fn strip_control(group: usize, sound: usize, pitch: bool, v: f64) -> Live {
    let kind = if pitch {
        MidiKind::PitchBend(((v.clamp(-1.0, 1.0) + 1.0) / 2.0 * 16383.0).round() as u16)
    } else {
        MidiKind::Controller { number: 1, value: (v.clamp(0.0, 1.0) * 127.0).round() as u8 }
    };
    Live::ControlIn { group, sound, kind, thru: false, record: true }
}

/// A strummed note (NOTES mode) started, held till it ends, or ended.
pub(crate) fn strip_note(group: usize, sound: usize, pitch: i64, velocity: i64, on: bool) -> Live {
    if on { Live::NoteOn { group, sound, pitch, velocity, length: i64::MAX } } else { Live::NoteRelease { group, sound, pitch } }
}
