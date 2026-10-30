//! Maschine's Poly Synth (docs/synth-poly-synth.md): `NI::MASCHINE::DATA::PolySynth` →
//! `RT::PolySynthProcessor` → `ni::p5::P5`, a Prophet-5-style polysynth: two
//! band-limited oscillators (A: saw + pulse, hard sync; B: saw + triangle + pulse,
//! Low and KBD switches), noise, a 4-pole LP/BP/HP filter with its own ADSR, an
//! amplifier ADSR, a global LFO with sample & hold, wheel modulation, poly mod
//! (Osc B and the filter envelope onto Freq A, PW A and the filter), up to 16 voices
//! with oldest-released voice reuse and stealing, unison with glide and detune.
//!
//! The engine (`poly_synth/*`) is ported from the arm64 code operation by operation;
//! this file is the glue `PolySynth::connectParameters` provides: each stored
//! parameter normalized and handed to `P5::setParameterValue`, in the order Maschine
//! connects them.

// The engine keeps Maschine's operand order (`a = b + a`, `!(x > y)` for NaN cases),
// which these lints would rewrite.
#[allow(clippy::assign_op_pattern, clippy::neg_cmp_op_on_partial_ord, clippy::manual_range_contains)]
mod engine;
#[allow(clippy::assign_op_pattern, clippy::neg_cmp_op_on_partial_ord, clippy::manual_range_contains)]
mod params;
#[allow(
    clippy::assign_op_pattern,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::manual_range_contains,
    clippy::needless_range_loop,
    clippy::needless_late_init
)]
mod render;
#[allow(clippy::assign_op_pattern, clippy::manual_range_contains)]
mod voices;

use self::engine::P5;
use self::params::id;
use super::Synth;
use deckard_model::MidiKind;

/// How a stored field becomes the engine's normalized value.
#[derive(Clone, Copy)]
enum Kind {
    /// `(v − min) · (1/(max − min))` (the scale in `f64`, as the parameter object
    /// stores it).
    Range(f64, f64),
    /// Filter Mode enum (3 options): index / 2.
    Mode3,
    /// LFO Mode enum: 0 or 1.
    Mode2,
    /// Voices enum: index + 1 voices (resets every voice when it changes).
    Voices,
    /// Voice Mode enum: index · 0.5.
    VoiceMode,
}

/// One Maschine parameter: (mxprj field, `ParameterId`, kind, default stored value),
/// in `connectParameters` order (`$_0` … `$_60`).
const PARAMS: [(&str, u32, Kind, f64); 61] = [
    ("amp_attack", id::A_ATTACK, Kind::Range(0.0, 1.0), 0.0),
    ("amp_decay", id::A_DECAY, Kind::Range(0.0, 1.0), 0.0),
    ("amp_sustain", id::A_SUSTAIN, Kind::Range(0.0, 1.0), 1.0),
    ("amp_release", id::A_RELEASE, Kind::Range(0.0, 1.0), 0.0),
    ("hold", id::HOLD, Kind::Range(0.0, 1.0), 0.0),
    ("filter_cutoff", id::CUTOFF, Kind::Range(0.0, 1.0), 1.0),
    ("filter_resonance", id::RESONANCE, Kind::Range(0.0, 1.0), 0.0),
    ("filter_env_amount", id::ENV_AMOUNT, Kind::Range(0.0, 1.0), 0.0),
    ("filter_kbd", id::KBD, Kind::Range(0.0, 1.0), 0.0),
    ("filter_invert", id::INVERT, Kind::Range(0.0, 1.0), 0.0),
    ("filter_attack", id::F_ATTACK, Kind::Range(0.0, 1.0), 0.0),
    ("filter_decay", id::F_DECAY, Kind::Range(0.0, 1.0), 0.0),
    ("filter_sustain", id::F_SUSTAIN, Kind::Range(0.0, 1.0), 1.0),
    ("filter_release", id::F_RELEASE, Kind::Range(0.0, 1.0), 0.0),
    ("filter_mode", id::FILTER_MODE, Kind::Mode3, 0.0),
    ("release", id::RELEASE, Kind::Range(0.0, 1.0), 1.0),
    ("velocity", id::VELOCITY, Kind::Range(0.0, 1.0), 0.0),
    ("glide", id::GLIDE, Kind::Range(0.0, 1.0), 0.0),
    ("voices", 0, Kind::Voices, 5.0),
    ("voice_mode", id::VOICE_MODE, Kind::VoiceMode, 0.0),
    ("detune", id::DETUNE, Kind::Range(0.0, 1.0), 0.0),
    ("tune", id::MASTER_TUNE, Kind::Range(-1.0, 1.0), 0.0),
    ("volume", id::VOLUME, Kind::Range(0.0, 1.0), 1.0),
    ("pitchbend_range", id::BEND_RANGE, Kind::Range(0.0, 24.0), 2.0),
    ("filter_version", id::FILTER_VERSION, Kind::Range(0.0, 1.0), 1.0),
    ("osc_version", id::OSC_VERSION, Kind::Range(0.0, 1.0), 1.0),
    ("pm_filter_env", id::PM_ENV, Kind::Range(0.0, 1.0), 0.0),
    ("pm_osc_b", id::PM_OSC_B, Kind::Range(0.0, 1.0), 0.0),
    ("pm_freq_a", id::PM_FREQ_A, Kind::Range(0.0, 1.0), 0.0),
    ("pm_pw_a", id::PM_PW_A, Kind::Range(0.0, 1.0), 0.0),
    ("pm_filter", id::PM_FILTER, Kind::Range(0.0, 1.0), 0.0),
    ("lfo_mode", id::LFO_MODE, Kind::Mode2, 0.0),
    ("lfo_rate", id::LFO_RATE, Kind::Range(0.0, 1.0), 0.0),
    ("lfo_saw", id::LFO_SAW, Kind::Range(0.0, 1.0), 0.0),
    ("lfo_triangle", id::LFO_TRI, Kind::Range(0.0, 1.0), 1.0),
    ("lfo_pulse", id::LFO_PULSE, Kind::Range(0.0, 1.0), 0.0),
    ("lfo_voice_trig", id::VOICE_TRIG, Kind::Range(0.0, 1.0), 0.0),
    ("mixer_osc_a", id::MIX_A, Kind::Range(0.0, 1.0), 1.0),
    ("mixer_osc_b", id::MIX_B, Kind::Range(0.0, 1.0), 0.0),
    ("mixer_noise", id::NOISE, Kind::Range(0.0, 1.0), 0.0),
    ("mixer_external", id::EXTERNAL, Kind::Range(0.0, 1.0), 0.0),
    ("osc_a_tune", id::TUNE_A, Kind::Range(-24.0, 24.0), 0.0),
    ("osc_a_saw", id::SAW_A, Kind::Range(0.0, 1.0), 1.0),
    ("osc_a_pulse", id::PULSE_A, Kind::Range(0.0, 1.0), 0.0),
    ("osc_a_pulse_width", id::PW_A, Kind::Range(0.0, 1.0), 0.5),
    ("osc_a_sync", id::SYNC, Kind::Range(0.0, 1.0), 0.0),
    ("osc_b_tune", id::TUNE_B, Kind::Range(-24.0, 24.0), 0.0),
    ("osc_b_fine", id::FINE_B, Kind::Range(0.0, 1.0), 0.0),
    ("osc_b_saw", id::SAW_B, Kind::Range(0.0, 1.0), 1.0),
    ("osc_b_triangle", id::TRI_B, Kind::Range(0.0, 1.0), 0.0),
    ("osc_b_pulse", id::PULSE_B, Kind::Range(0.0, 1.0), 0.0),
    ("osc_b_pulse_width", id::PW_B, Kind::Range(0.0, 1.0), 0.5),
    ("osc_b_low", id::LOW_B, Kind::Range(0.0, 1.0), 0.0),
    ("osc_b_kbd", id::KBD_B, Kind::Range(0.0, 1.0), 1.0),
    ("mod_lfo_noise", id::LFO_NOISE, Kind::Range(0.0, 1.0), 0.0),
    ("mod_amount", id::MOD_AMOUNT, Kind::Range(0.0, 1.0), 0.0),
    ("mod_freq_a", id::WM_FREQ_A, Kind::Range(0.0, 1.0), 0.0),
    ("mod_freq_b", id::WM_FREQ_B, Kind::Range(0.0, 1.0), 0.0),
    ("mod_pw_a", id::WM_PW_A, Kind::Range(0.0, 1.0), 0.0),
    ("mod_pw_b", id::WM_PW_B, Kind::Range(0.0, 1.0), 0.0),
    ("mod_filter", id::WM_FILTER, Kind::Range(0.0, 1.0), 0.0),
];

pub struct PolySynth {
    p5: Box<P5>,
    /// The stored value each parameter last had (None before the first `set`).
    current: [Option<f64>; 61],
    tempo: f32,
    mono: Vec<f32>,
    /// The host rate, applied after the first parameters (Maschine connects the
    /// parameters while the engine still runs at its constructor's 44.1 kHz).
    sr: f32,
}

impl PolySynth {
    pub fn new(sr: f64) -> PolySynth {
        PolySynth { p5: P5::new(44100.0), current: [None; 61], tempo: 120.0, mono: vec![0.0; 1024], sr: sr as f32 }
    }

    /// One parameter's lambda: normalize as Maschine does and hand it to the engine.
    fn apply(&mut self, k: usize, value: f64) {
        let (_, pid, kind, _) = PARAMS[k];
        let p = &mut self.p5;
        match kind {
            Kind::Range(min, max) => {
                let v = ((1.0 / (max - min)) * (value as f32 - min as f32) as f64) as f32;
                p.set_parameter_value(pid, v);
            }
            Kind::Mode3 => {
                let i = value.max(0.0) as u64;
                p.set_parameter_value(pid, (0.5 * i as f64) as f32);
            }
            Kind::Mode2 => p.set_parameter_value(pid, if value as i64 == 0 { 0.0 } else { 1.0 }),
            Kind::VoiceMode => {
                let i = (value as i32).clamp(0, 2);
                p.set_parameter_value(pid, i as f32 * 0.5);
            }
            Kind::Voices => {
                let n = ((value as i64).max(0) as u32 + 1).min(32);
                if n != p.voices {
                    p.voices = n;
                    p.sustain = false;
                    p.all_sound_off();
                }
            }
        }
    }
}

impl Synth for PolySynth {
    fn set(&mut self, params: &[(String, f64)]) {
        let first = self.current[0].is_none();
        for (k, &(field, _, _, default)) in PARAMS.iter().enumerate() {
            let v = params.iter().find(|(f, _)| f == field).map_or(default, |(_, v)| *v);
            if self.current[k] != Some(v) {
                self.current[k] = Some(v);
                self.apply(k, v);
            }
        }
        if first {
            self.p5.set_sample_rate(self.sr);
        }
    }

    fn note_on(&mut self, note: i64, velocity: i64) {
        self.p5.note_on(note.clamp(0, 127) as u8, velocity.clamp(0, 127) as u8);
    }

    fn note_off(&mut self, note: i64) {
        self.p5.note_off(note.clamp(0, 127) as u8);
    }

    fn control(&mut self, kind: MidiKind) {
        match kind {
            MidiKind::Controller { number: 1, value } => self.p5.mod_wheel(value.min(127)),
            MidiKind::Controller { number: 64, value } => self.p5.sustain_pedal(value >= 64),
            MidiKind::Controller { number: 120, .. } => self.p5.all_sound_off(),
            MidiKind::PitchBend(v) => self.p5.pitch_bend(v),
            _ => {}
        }
    }

    fn stop(&mut self) {
        self.p5.all_sound_off();
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.p5.lfo_synced != 0 {
            self.p5.lfo_rate = self.tempo * 0.016_666_668 * self.p5.lfo_mult;
        }
        let n = l.len();
        if self.mono.len() < n {
            // Only when a host sends blocks longer than 1024 frames.
            self.mono.resize(n, 0.0);
        }
        let m = &mut self.mono[..n];
        self.p5.render(m);
        l.copy_from_slice(m);
        r.copy_from_slice(m);
    }

    fn voices(&self) -> usize {
        let n = (self.p5.voices as usize).min(engine::SLOTS);
        self.p5.v[..n].iter().filter(|v| v.active != 0 || v.velocity > 0.0).count()
    }

    fn set_tempo(&mut self, bpm: f64) {
        self.tempo = bpm as f32;
    }
}

/// What a new Poly Synth holds: Maschine's defaults (stored values, by field).
pub fn defaults() -> Vec<(&'static str, f64)> {
    // (The editor tab and the two version flags aren't stored.)
    PARAMS
        .iter()
        .filter(|(field, ..)| !matches!(*field, "editor_tab" | "filter_version" | "osc_version"))
        .map(|&(field, _, _, default)| (field, default))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(s: &mut PolySynth, frames: usize) -> Vec<f32> {
        let mut out = Vec::new();
        let mut done = 0;
        while done < frames {
            let n = 300.min(frames - done);
            let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
            s.process(&mut l, &mut r);
            assert_eq!(l, r);
            out.extend_from_slice(&l);
            done += n;
        }
        out
    }

    #[test]
    fn defaults_sound_and_end() {
        let mut s = PolySynth::new(44100.0);
        s.set(&[]);
        render(&mut s, 4410);
        s.note_on(60, 100);
        let a = render(&mut s, 22050);
        assert!(a.iter().all(|x| x.is_finite()));
        assert!(a.iter().any(|x| x.abs() > 0.01), "silent");
        s.note_off(60);
        let b = render(&mut s, 44100);
        assert!(b.iter().all(|x| x.is_finite()));
        assert_eq!(s.voices(), 0);
        let c = render(&mut s, 512);
        assert!(c.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn polyphony_and_stealing() {
        let mut s = PolySynth::new(48000.0);
        s.set(&[("voices".into(), 3.0)]); // 4 voices
        for n in 0..6 {
            s.note_on(48 + n, 90);
        }
        render(&mut s, 2000);
        assert_eq!(s.voices(), 4);
    }

    #[test]
    fn every_switch_renders() {
        for (f, v) in [
            ("filter_mode", 1.0),
            ("filter_mode", 2.0),
            ("voice_mode", 1.0),
            ("voice_mode", 2.0),
            ("osc_a_sync", 1.0),
            ("osc_a_pulse", 1.0),
            ("osc_b_triangle", 1.0),
            ("osc_b_pulse", 1.0),
            ("osc_b_low", 1.0),
            ("mixer_noise", 1.0),
            ("filter_resonance", 1.0),
            ("detune", 1.0),
            ("lfo_voice_trig", 1.0),
            ("pm_freq_a", 1.0),
        ] {
            let mut s = PolySynth::new(44100.0);
            s.set(&[(f.into(), v), ("mixer_osc_b".into(), 0.7), ("mod_amount".into(), 0.5), ("mod_filter".into(), 1.0)]);
            s.note_on(57, 127);
            s.note_on(64, 64);
            let a = render(&mut s, 8000);
            assert!(a.iter().all(|x| x.is_finite() && x.abs() < 50.0), "{f}");
            assert!(a.iter().any(|x| x.abs() > 1e-4), "{f} silent");
        }
    }

    #[test]
    fn set_mid_note_keeps_voice() {
        let mut s = PolySynth::new(44100.0);
        s.set(&[]);
        s.note_on(60, 100);
        let a = render(&mut s, 4000);
        s.set(&[("filter_cutoff".into(), 0.5)]);
        let b = render(&mut s, 4000);
        assert_eq!(s.voices(), 1);
        let jump = (b[0] - a[a.len() - 1]).abs();
        assert!(jump < 0.2, "click {jump}");
    }

    #[test]
    fn no_denormals_in_long_tail() {
        let mut s = PolySynth::new(44100.0);
        s.set(&[("amp_release".into(), 1.0)]);
        s.note_on(40, 100);
        render(&mut s, 1000);
        s.note_off(40);
        let t = render(&mut s, 44100 * 4);
        assert!(t.iter().all(|x| x.is_finite() && (*x == 0.0 || x.abs() > 1e-38)));
    }
}
