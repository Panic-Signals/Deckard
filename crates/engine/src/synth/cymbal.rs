//! Maschine's Cymbal (docs/synth-cymbal.md): `DATA::Cymbal` →
//! `RT::DrumSynthWrapper<DSP::CymbalProcessor>`, two engines — Crash
//! (`DSP::CrashCymbalCore`) and Ride (`DSP::RideCymbalCore`) — built from NI's
//! modal-drum kit:
//!
//! - [`attack`]: `AttackCore` (oscillator and noise burst of the strike);
//! - [`body`]: `CymbalBodyCore` (exciter, 512-partial modal bank, air noise);
//! - [`attached`]: `AttachedBodiesCore` (recorded noises through crispel filters);
//! - [`fdn`]: `FdbckDelayNetwork` (15 lines, circulant feedback);
//! - [`rtmod`]: `RealTimeModulation` (knobs and velocity → DSP targets);
//! - [`crash`], [`ride`]: the cores (preset, parameter routing, mixing, output shelves).
//!
//! The wrapper runs the active core in stretches cut at its control period (128
//! samples, `processControlRate`) and modulation period (16, `processModulationRate`),
//! the counters running on across calls, so any block length gives the same output.
//! The noise recordings two layers play come from the user's Maschine install
//! ([`res`]); without one those layers are silent.

#![allow(dead_code)]

mod attached;
mod attack;
mod body;
mod crash;
mod fdn;
mod memo;
mod metal;
mod res;
mod ride;
mod rtmod;

use super::Synth;
use crate::fx::param;
use crash::Crash;
use ride::Ride;

/// `processControlRate` period (`DrumSynthWrapper::processAudio`).
const CONTROL: usize = 128;
/// `processModulationRate` period.
const MODULATION: usize = 16;

/// The stored fields and their defaults (`Cymbal::Cymbal()`), in `connectParameters`
/// order.
const FIELDS: [(&str, f64); 13] = [
    ("engine", 0.0),
    ("tune", 0.300_000_011_920_929),
    ("decay", 0.699_999_988_079_071),
    ("impact", 0.699_999_988_079_071),
    ("width", 0.5),
    ("velocity", 1.0),
    ("density", 1.0),
    ("tone", 0.5),
    ("crash_mode", 0.0),
    ("edge", 0.5),
    ("bell", 0.0),
    ("character", 0.200_000_002_980_232_24),
    ("tail", 1.0),
];

/// `NI::DSP::CymbalProcessor` and its wrapper.
pub struct Cymbal {
    crash: Box<metal::Metal>,
    ride: Box<metal::Metal>,
    /// +0x80: the shared parameters (mode, tune, decay, impact, width, velocity scale).
    shared: [f32; 6],
    /// +0x98: the last note's velocity (0..1).
    velocity: f32,
    /// +0x9c: `(1 − |scale|·(1 − v))³` (`v` instead of `1 − v` for a negative scale).
    vel_gain: f32,
    /// +0xa0: the engine playing.
    mode: u32,
    /// +0xa4: the engine chosen (takes over at the next note-on).
    pending: u32,
    /// The stored values last applied, by [`FIELDS`] index.
    applied: [Option<f32>; 13],
    /// Samples to the next control / modulation tick.
    to_control: usize,
    to_modulation: usize,
    sr: f32,
    started: bool,
}

impl Cymbal {
    pub fn new(sr: f64) -> Cymbal {
        let mut c = Cymbal {
            crash: Crash::new(),
            ride: Ride::new(),
            shared: [0.0; 6],
            velocity: 0.0,
            vel_gain: 0.0,
            mode: 0,
            pending: 0,
            applied: [None; 13],
            to_control: 0,
            to_modulation: 0,
            sr: sr as f32,
            started: false,
        };
        // DSPCoreWrapper(): onInit, onSetSampleRate(44100, 128).
        c.crash.on_set_sample_rate(44_100.0, CONTROL as i32);
        c.ride.on_set_sample_rate(44_100.0, CONTROL as i32);
        c
    }

    /// `CymbalProcessor::setParameter(i, v)` (0x1e3b6a8).
    fn set_shared(&mut self, i: usize, v: f32) {
        self.shared[i] = v;
        match i {
            0 => {
                let m = v as u32;
                self.pending = m;
                if self.mode != m {
                    match self.mode {
                        0 => self.crash.on_choke(),
                        1 => self.ride.on_choke(),
                        _ => {}
                    }
                }
            }
            1 => self.core_param(0, 0, v),
            2 => self.core_param(1, 1, v),
            3 => self.core_param(4, 4, v),
            4 => self.core_param(5, 7, v),
            _ => self.core_param(7, 8, v),
        }
    }

    /// A shared parameter for the current core (its Crash and Ride index).
    fn core_param(&mut self, crash: u32, ride: u32, v: f32) {
        match self.mode {
            0 => self.crash.set_parameter(crash, v),
            1 => self.ride.set_parameter(ride, v),
            _ => {}
        }
    }

    /// `CymbalProcessor::updateGlobalCoreParameters()` (0x1e3bcf4).
    fn update_global(&mut self) {
        let p = self.shared;
        match self.mode {
            0 => {
                for (i, v) in [(0, p[1]), (1, p[2]), (4, p[3]), (5, p[4]), (7, p[5])] {
                    self.crash.set_parameter(i, v);
                }
            }
            1 => {
                for (i, v) in [(0, p[1]), (1, p[2]), (4, p[3]), (7, p[4]), (8, p[5])] {
                    self.ride.set_parameter(i, v);
                }
            }
            _ => {}
        }
    }

    /// Field `i` of [`FIELDS`] at stored value `v` (`connectParameters`' routes).
    fn apply(&mut self, i: usize, v: f32) {
        match i {
            0..=5 => self.set_shared(i, v),
            6 => self.crash.set_parameter(2, v),
            7 => self.crash.set_parameter(3, v),
            8 => self.crash.set_parameter(6, v as u32 as f32),
            9 => self.ride.set_parameter(2, v),
            10 => self.ride.set_parameter(3, v),
            11 => self.ride.set_parameter(5, v),
            _ => self.ride.set_parameter(6, v),
        }
    }
}

impl Synth for Cymbal {
    fn set(&mut self, params: &[(String, f64)]) {
        for (i, (field, default)) in FIELDS.iter().enumerate() {
            let v = param(params, field, *default) as f32;
            if self.applied[i] != Some(v) {
                self.applied[i] = Some(v);
                self.apply(i, v);
            }
        }
        if !self.started {
            // onSamplerateChanged(sr), then resume() = onReset.
            self.started = true;
            self.crash.on_set_sample_rate(self.sr, CONTROL as i32);
            self.ride.on_set_sample_rate(self.sr, CONTROL as i32);
            self.crash.on_reset();
            self.ride.on_reset();
        }
    }

    /// `CymbalProcessor::setNoteOn(v, key)` (0x1e3be4c), `v = velocity·f32(1/127)`.
    /// Velocity 0 is a note-off (nothing happens).
    fn note_on(&mut self, note: i64, velocity: i64) {
        if velocity <= 0 {
            return;
        }
        let v = velocity.min(127) as f32 * f32::from_bits(0x3c01_0204);
        self.mode = self.pending;
        let s = self.shared[5];
        let x = if s < 0.0 { v } else { 1.0 - v };
        let g = (-x).mul_add(s.abs(), 1.0);
        self.velocity = v;
        self.vel_gain = g * (g * g);
        self.update_global();
        let key = note.clamp(0, 127) as u32;
        match self.mode {
            0 => self.crash.set_note_on(v, key),
            1 => self.ride.set_note_on(v, key),
            _ => {}
        }
    }

    /// `setNoteOff` does nothing in either core.
    fn note_off(&mut self, _note: i64) {}

    /// All sound off (`onChoke`, also MIDI CC 120): the playing core fades out.
    fn stop(&mut self) {
        match self.mode {
            0 => self.crash.on_choke(),
            1 => self.ride.on_choke(),
            _ => {}
        }
    }

    /// `DrumSynthWrapper::processAudio` (0x1e2ac64).
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n = l.len();
        let mut at = 0;
        while at < n {
            if self.to_control == 0 {
                match self.mode {
                    0 => self.crash.process_control_rate(),
                    1 => self.ride.process_control_rate(),
                    _ => {}
                }
                self.to_control = CONTROL;
            }
            if self.to_modulation == 0 {
                match self.mode {
                    0 => self.crash.process_modulation_rate(),
                    1 => self.ride.process_modulation_rate(),
                    _ => {}
                }
                self.to_modulation = MODULATION;
            }
            let k = (n - at).min(self.to_control).min(self.to_modulation);
            let (bl, br) = (&mut l[at..at + k], &mut r[at..at + k]);
            let wrote = match self.mode {
                0 => self.crash.process_audio(bl, br),
                1 => self.ride.process_audio(bl, br),
                _ => false,
            };
            if !wrote {
                bl.fill(0.0);
                br.fill(0.0);
            }
            self.to_control -= k;
            self.to_modulation -= k;
            at += k;
        }
    }

    fn voices(&self) -> usize {
        let on = match self.mode {
            0 => self.crash.has_output_signal(),
            1 => self.ride.has_output_signal(),
            _ => false,
        };
        usize::from(on)
    }
}

/// What a new Cymbal holds: Maschine's defaults (stored values, by field).
pub fn defaults() -> Vec<(&'static str, f64)> {
    vec![
        ("engine", 0.0),
        ("tune", 0.3),
        ("decay", 0.7),
        ("impact", 0.7),
        ("width", 0.5),
        ("velocity", 1.0),
        ("density", 1.0),
        ("tone", 0.5),
        ("crash_mode", 0.0),
        ("edge", 0.5),
        ("bell", 0.0),
        ("character", 0.2),
        ("tail", 1.0),
    ]
}
