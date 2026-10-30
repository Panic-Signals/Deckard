//! Maschine's internal instruments (docs/synth-*.md): the drum synths (Kick, Snare,
//! Hi-hat, Tom, Percussion, Cymbal), Bass Synth and Poly Synth. A sound plays one in
//! its chain's first slot, where the Sampler would be.

pub mod bass_synth;
pub mod cymbal;
pub mod hihat;
pub mod kick;
pub(crate) mod modal_lite;
pub mod noise;
pub mod perc;
pub mod poly_synth;
pub mod snare;
pub mod tom;

use deckard_model::MidiKind;

/// The module classes [`make`] plays, as stored ("Kick", …).
pub const CLASSES: [&str; 8] = ["Kick", "Snare", "Hihat", "Tom", "Perc", "Cymbal", "BassSynth", "PolySynth"];

/// An internal instrument: notes in, stereo out.
pub trait Synth: Send {
    /// Take the module's parameters (stored values, by field name). Called when the
    /// instrument is made and again when they change, keeping its voices (no clicks).
    /// Called on the audio thread: no allocating.
    fn set(&mut self, params: &[(String, f64)]);
    /// Start a note (`velocity` 1..127) at the start of the next [`Synth::process`].
    fn note_on(&mut self, note: i64, velocity: i64);
    /// End a note.
    fn note_off(&mut self, note: i64);
    /// Mod wheel, sustain pedal, pitch bend, …
    fn control(&mut self, kind: MidiKind) {
        let _ = kind;
    }
    /// Silence every voice quickly (transport stop, all sound off).
    fn stop(&mut self);
    /// Render the next `l.len()` frames into `l`/`r` (the same length, any length),
    /// overwriting them.
    fn process(&mut self, l: &mut [f32], r: &mut [f32]);
    /// How many voices sound (0: [`Synth::process`] would give silence and may be
    /// skipped).
    fn voices(&self) -> usize;
    /// The song tempo (BPM), for tempo-synced modulation.
    fn set_tempo(&mut self, bpm: f64) {
        let _ = bpm;
    }
}

/// The instrument for a module class ("Kick", …) at sample rate `sr`, if it is one
/// DECKARD plays.
pub fn make(module: &str, params: &[(String, f64)], sr: f64) -> Option<Box<dyn Synth>> {
    let mut s: Box<dyn Synth> = match module {
        "Kick" => Box::new(kick::Kick::new(sr)),
        "Snare" => Box::new(snare::Snare::new(sr)),
        "Hihat" | "Tom" | "Cymbal"
            if {
                noise::warm();
                false
            } =>
        {
            unreachable!()
        }
        "Hihat" => Box::new(hihat::Hihat::new(sr)),
        "Tom" => Box::new(tom::Tom::new(sr)),
        "Perc" => Box::new(perc::Perc::new(sr)),
        "Cymbal" => Box::new(cymbal::Cymbal::new(sr)),
        "BassSynth" => Box::new(bass_synth::BassSynth::new(sr)),
        "PolySynth" => Box::new(poly_synth::PolySynth::new(sr)),
        _ => return None,
    };
    s.set(params);
    Some(s)
}

/// What a new instrument of module class `module` holds (Maschine's defaults, by
/// field), if DECKARD has it.
pub fn defaults(module: &str) -> Option<Vec<(&'static str, f64)>> {
    Some(match module {
        "Kick" => kick::defaults(),
        "Snare" => snare::defaults(),
        "Hihat" => hihat::defaults(),
        "Tom" => tom::defaults(),
        "Perc" => perc::defaults(),
        "Cymbal" => cymbal::defaults(),
        "BassSynth" => bass_synth::defaults(),
        "PolySynth" => poly_synth::defaults(),
        _ => return None,
    })
}

pub use crate::fx::param;

#[cfg(test)]
mod tests {
    use super::*;

    /// Every engine of every drum synth makes a sound.
    #[test]
    fn every_drum_engine_sounds() {
        let engines: [(&str, &str, usize); 6] = [
            ("Kick", "engine", 9),
            ("Snare", "mode", 10),
            ("Hihat", "mode", 4),
            ("Tom", "engine", 4),
            ("Perc", "mode", 3),
            ("Cymbal", "engine", 2),
        ];
        let mut silent = Vec::new();
        for (module, field, n) in engines {
            for e in 0..n {
                let mut params: Vec<(String, f64)> = defaults(module).unwrap().into_iter().map(|(k, v)| (k.to_string(), v)).collect();
                params.retain(|(k, _)| k != field);
                params.push((field.to_string(), e as f64));
                let mut s = make(module, &params, 48_000.0).unwrap();
                s.note_on(60, 110);
                let (mut l, mut r) = (vec![0f32; 512], vec![0f32; 512]);
                let mut peak = 0f32;
                for _ in 0..40 {
                    s.process(&mut l, &mut r);
                    peak = l.iter().chain(&r).fold(peak, |m, v| m.max(v.abs()));
                }
                if peak < 0.01 {
                    silent.push(format!("{module} {e}"));
                }
            }
        }
        assert!(silent.is_empty(), "silent: {silent:?}");
    }
}
