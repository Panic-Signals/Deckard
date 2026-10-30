//! LFOs on DECKARD's own controller layer: the project's LFOs as the page shows them,
//! and its edits ([`deckard_surface::Action::Lfo`]).

use crate::App;
use crate::layout::{self, Act};
use deckard_model::lfo::{ENV_MAX, ENV_MIN, Lfo, RATES, Shape, env_knob, env_time};
use deckard_surface::LfoView;

/// How far one knob step moves each setting.
const PHASE_STEP: f32 = 5.0;
const LEVEL_STEP: f32 = 0.01;
const FADE_STEP: f32 = 0.25;
/// A free rate goes up by this factor a step.
const HZ_STEP: f32 = 1.1;
/// An envelope time's knob moves this much a step (100 steps from 1 ms to 10 s).
const ENV_STEP: f32 = 0.01;
const SUSTAIN_STEP: f32 = 0.01;

/// An envelope time as written: "12 ms" under a second, else "1.20 s".
pub(crate) fn env_time_text(seconds: f32) -> String {
    if seconds < 0.9995 { format!("{:.0} ms", seconds * 1000.0) } else { format!("{seconds:.2} s") }
}

/// An LFO's trigger as written: "A1 / S3", "Off" (an envelope's: "None").
pub(crate) fn trigger_text(l: &Lfo) -> String {
    match l.trigger {
        Some((g, s)) => format!("{} / S{}", layout::group_id(g), s + 1),
        None if l.env => "None".to_string(),
        None => "Off".to_string(),
    }
}

impl App {
    /// The project's LFOs for the surface; their pictures only with `points`.
    pub(crate) fn lfo_views(&self, points: bool) -> Vec<LfoView> {
        let Some(doc) = &self.doc else { return Vec::new() };
        doc.borrow()
            .project()
            .lfos
            .iter()
            .map(|l| LfoView {
                pad: l.pad as usize,
                shape: l.shape.name().to_string(),
                sync: l.sync,
                rate: if l.sync { RATES[l.rate.min(RATES.len() - 1)].0.to_string() } else { format!("{:.2} Hz", l.hz) },
                phase: l.phase as f64,
                level: l.level as f64,
                quantize: l.quantize as i64,
                trigger: trigger_text(l),
                fade: l.fade as f64,
                muted: l.muted,
                env: l.env,
                attack: env_time_text(l.attack),
                decay: env_time_text(l.decay),
                release: env_time_text(l.release),
                sustain: l.sustain as f64,
                retrigger: l.retrigger,
                name: l.name(),
                points: if points { l.picture(240) } else { Vec::new() },
                note_off: l.picture_note_off(),
            })
            .collect()
    }

    /// An LFO edit from the surface.
    pub(crate) fn surface_lfo(&mut self, what: &str, pad: usize, value: f64) {
        let Ok(pad) = u8::try_from(pad) else { return };
        if !(1..=16).contains(&pad) {
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        let (found, sounds) = {
            let d = doc.borrow();
            let p = d.project();
            let sounds: Vec<(usize, usize)> =
                p.groups.iter().enumerate().flat_map(|(g, x)| (0..x.sounds.len()).map(move |s| (g, s))).collect();
            (p.lfos.iter().find(|l| l.pad == pad).cloned(), sounds)
        };
        match what {
            "focus" => {
                if found.is_some() {
                    layout::apply_acts(self, vec![Act::FocusLfo(pad)]);
                }
                return;
            }
            "create" => {
                if found.is_none() {
                    layout::apply_acts(self, vec![Act::SetLfo(Lfo::new(pad)), Act::FocusLfo(pad)]);
                } else {
                    layout::apply_acts(self, vec![Act::FocusLfo(pad)]);
                }
                return;
            }
            "remove" => {
                if found.is_some() {
                    layout::apply_acts(self, vec![Act::RemoveLfo(pad)]);
                }
                return;
            }
            _ => {}
        }
        let Some(mut l) = found else { return };
        let steps = value.round() as i64;
        let n = value as f32;
        let time = |t: f32| env_time(env_knob(t) + n * ENV_STEP).clamp(ENV_MIN, ENV_MAX);
        match what {
            "mute" => l.muted = !l.muted,
            // (An envelope needs a trigger: the focused sound, if it has none.)
            "env" => {
                l.env = !l.env;
                if l.env && l.trigger.is_none() {
                    l.trigger = Some((self.group, self.sound));
                }
            }
            "retrigger" => l.retrigger = !l.retrigger,
            "attack" => l.attack = time(l.attack),
            "decay" => l.decay = time(l.decay),
            "release" => l.release = time(l.release),
            "sustain" => l.sustain = round2(l.sustain + n * SUSTAIN_STEP).clamp(0.0, 1.0),
            // An envelope's trigger is never turned off.
            "trigger_focused" if l.env && value <= 0.5 => return,
            "shape" => match Shape::ALL.get((value as usize).wrapping_sub(1)) {
                Some(s) => l.shape = *s,
                None => return,
            },
            "sync" => l.sync = value > 0.5,
            "trigger_focused" => l.trigger = (value > 0.5).then_some((self.group, self.sound)),
            "rate" if l.sync => l.rate = (l.rate as i64 + steps).clamp(0, RATES.len() as i64 - 1) as usize,
            "rate" => l.hz = (l.hz * HZ_STEP.powf(n)).clamp(0.01, 20.0),
            "phase" => l.phase = (l.phase + n * PHASE_STEP).clamp(0.0, 360.0),
            "level" => l.level = round2(l.level + n * LEVEL_STEP).clamp(0.0, 1.0),
            "quantize" => {
                // Off, then 2 to 32.
                let at = if l.quantize == 0 { 1 } else { l.quantize as i64 };
                let to = (at + steps).clamp(1, 32);
                l.quantize = if to < 2 { 0 } else { to as u8 };
            }
            "trigger" if l.env => {
                // Each sound in turn, no Off.
                if sounds.is_empty() {
                    return;
                }
                let at = l.trigger.and_then(|t| sounds.iter().position(|s| *s == t)).map_or(-1, |i| i as i64);
                l.trigger = Some(sounds[(at + steps).clamp(0, sounds.len() as i64 - 1) as usize]);
            }
            "trigger" => {
                // Off, then each sound in turn.
                let at = l.trigger.and_then(|t| sounds.iter().position(|s| *s == t)).map_or(0, |i| i as i64 + 1);
                let to = (at + steps).clamp(0, sounds.len() as i64);
                l.trigger = (to > 0).then(|| sounds[to as usize - 1]);
            }
            "fade" => l.fade = (l.fade + n * FADE_STEP).clamp(0.0, 16.0),
            _ => return,
        }
        layout::apply_acts(self, vec![Act::SetLfo(l)]);
    }
}

fn round2(x: f32) -> f32 {
    (x * 100.0).round() / 100.0
}
