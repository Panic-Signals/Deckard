//! Maschine's Snare (docs/synth-snare.md): `DATA::Snare` → `RT::DrumSynthWrapper<DSP::SnareProcessor>`.
//!
//! The processor owns one core per engine family and plays only the active one:
//!
//! | engine (mode) | core | file |
//! |---|---|---|
//! | Volt (0), Bit (1) | `ClassicSnareCore` | `snare/classic.rs` |
//! | Pow (2), Clap (8) | `ClapCore` | `snare/clap.rs` |
//! | Sharp (3) | `DiscoSDCore` | `snare/disco.rs` |
//! | Airy (4) | `AirySDCore` | `snare/airy.rs` |
//! | Vintage (5) | `OldieSDCore` | `snare/oldie.rs` |
//! | Chrome (6) | `WhippySDCore` | `snare/whippy.rs` |
//! | Iron (7) | `OrangeSDCore` | `snare/orange.rs` |
//! | Breaker (9) | `BreakerSDCore` | `snare/breaker.rs` |
//!
//! The wrapper runs the active core's control-rate step every 128 samples and renders
//! in pieces of at most 16 samples (both counters free-run from the start). A new
//! engine takes over at the next note-on (spec §2). It is one mono-timbral voice: a
//! note-on retriggers the core.

mod airy;
mod breaker;
mod clap;
mod classic;
mod disco;
mod modal;
mod oldie;
mod orange;
mod whippy;

use super::Synth;
use deckard_model::MidiKind;

/// One engine family's DSP core (`NI::DSP::*Core`), as `SnareProcessor` drives it.
pub(crate) trait Core: Send {
    /// `setParameter(idx, v)`: the core's own parameter numbering (spec §2.3).
    fn set_parameter(&mut self, idx: u32, v: f32);
    /// `setNoteOn(velocity, note)`. `velocity` is what the processor passes (spec §2.4).
    fn note_on(&mut self, velocity: f32, note: u32);
    /// `setNoteOff(note)`.
    fn note_off(&mut self, note: u32);
    /// `onChoke()` (MIDI CC 120, transport stop).
    fn choke(&mut self);
    /// What `setParamMode` does to the playing core when the engine changes away from
    /// it (Classic/Clap: `onChoke`, also on a change within the core; modal cores:
    /// their switch fade, state 1 → 2).
    fn leave(&mut self);
    /// `processControlRate()`, every 128 samples.
    fn control_rate(&mut self);
    /// `processAudio` for `l.len()` (≤ 16) samples, overwriting `l`/`r`.
    fn process(&mut self, l: &mut [f32], r: &mut [f32]);
    /// `hasOutputSignal()`.
    fn has_output(&self) -> bool;
}

/// The engines (the `mode` enum), in stored order.
pub const ENGINES: [&str; 10] = ["Volt", "Bit", "Pow", "Sharp", "Airy", "Vintage", "Chrome", "Iron", "Clap", "Breaker"];

/// The core that plays engine `mode` (`SnareProcessor::setParamMode`, +0xb4).
fn core_of(mode: usize) -> usize {
    match mode {
        2 | 8 => 1,
        3 => 6,
        4 => 5,
        5 => 3,
        6 => 4,
        7 => 2,
        9 => 7,
        _ => 0,
    }
}

/// The processor's shared parameters (`setParameter` index → field, stored range).
const SHARED: [(&str, f32, f32); 12] = [
    ("mode", 0.0, 9.0),
    ("tune", 0.0, 1.0),
    ("decay", 0.0, 1.0),
    ("osc_mode", 0.0, 1.0),
    ("skin_tune", 0.0, 1.0),
    ("impact", 0.0, 1.0),
    ("body", 0.0, 1.0),
    ("spectra_mode", 0.0, 1.0),
    ("spectra_tune", 0.0, 1.0),
    ("spectra_amount", 0.0, 1.0),
    ("spectra_decay", 0.0, 1.0),
    ("velocity", -1.0, 1.0),
];

/// Engine parameters (`setCoreParameter(core, idx, v)`): field, core, index.
const CORE_PARAMS: [(&str, usize, u32); 10] = [
    ("noise_color", 0, 6),
    ("noise_amount", 0, 7),
    ("punch", 0, 4),
    ("osc_mix", 0, 3),
    ("grit", 0, 5),
    ("gate", 0, 8),
    ("bend", 1, 4),
    ("room", 1, 3),
    ("focus", 1, 6),
    ("noise", 6, 5),
];

/// Defaults of a new Snare (`DATA::Snare::Snare()`), by field, in connect order.
pub const DEFAULTS: [(&str, f64); 22] = [
    ("mode", 0.0),
    ("tune", 0.5),
    ("decay", 0.5),
    ("skin_tune", 0.5),
    ("impact", 0.75),
    ("body", 0.75),
    ("spectra_mode", 0.0),
    ("spectra_tune", 0.5),
    ("spectra_amount", 0.75),
    ("spectra_decay", 0.4),
    ("velocity", 1.0),
    ("osc_mode", 0.0),
    ("noise_color", 0.5),
    ("noise_amount", 0.25),
    ("punch", 0.25),
    ("osc_mix", 0.5),
    ("grit", 0.3),
    ("gate", 0.0),
    ("bend", 0.5),
    ("room", 0.5),
    ("focus", 0.5),
    ("noise", 0.4),
];

pub struct Snare {
    cores: [Box<dyn Core>; 8],
    /// `SnareProcessor` +0x80: the shared parameters as last set (index as [`SHARED`]).
    shared: [f32; 12],
    /// +0xb0: the core that plays; +0xb4: the one the engine asks for.
    active: usize,
    pending: usize,
    /// The stored values last applied, by [`DEFAULTS`] index (NaN: never).
    applied: [f64; 22],
    /// Wrapper counters: samples to the next control-rate step, and left in this piece.
    ctrl_left: usize,
    piece_left: usize,
    /// Where the current 128-frame block began in this `process` call, and the block's
    /// silence flag (`AlignedAudioBlock` +0x10).
    block_start: usize,
    silent: bool,
    /// Events for the next `process`, applied at its start.
    events: [Event; 64],
    n_events: usize,
}

#[derive(Clone, Copy)]
enum Event {
    On(u32, f32),
    Off(u32),
    Choke,
}

impl Snare {
    pub fn new(sr: f64) -> Snare {
        let sr = sr as f32;
        let mut s = Snare {
            cores: [
                Box::new(classic::Classic::new(sr)),
                Box::new(clap::Clap::new(sr)),
                Box::new(orange::Orange::new(sr)),
                Box::new(oldie::Oldie::new(sr)),
                Box::new(whippy::Whippy::new(sr)),
                Box::new(airy::Airy::new(sr)),
                Box::new(disco::Disco::new(sr)),
                Box::new(breaker::Breaker::new(sr)),
            ],
            shared: [0.0; 12],
            active: 0,
            pending: 0,
            applied: [f64::NAN; 22],
            ctrl_left: 0,
            piece_left: 0,
            block_start: 0,
            silent: false,
            events: [Event::Choke; 64],
            n_events: 0,
        };
        s.set(&[]);
        s
    }

    /// `SnareProcessor::setParameter(idx, v)`: stored, and sent to the playing core.
    fn set_shared(&mut self, idx: usize, v: f32) {
        self.shared[idx] = v;
        if idx == 0 {
            self.set_mode(v);
        } else if idx == 3 {
            // `setParamAnalogSnareMode`: Osc Mode reaches the Classic core only while Volt
            // plays.
            if self.shared[0] as i32 == 0 && self.active == 0 && matches!(v as i32, 0 | 1) {
                self.cores[0].set_parameter(2, (v as i32) as f32);
            }
        } else if let Some(ci) = core_index(self.active, idx) {
            self.cores[self.active].set_parameter(ci, v);
        }
    }

    /// `SnareProcessor::setParamMode`: the new engine's core waits for the next note;
    /// the playing core is choked or faded when the engine leaves it.
    fn set_mode(&mut self, v: f32) {
        let mode = v as i32;
        let mode = if (0..=9).contains(&mode) { mode as usize } else { 0 };
        let core = core_of(mode);
        self.pending = core;
        let active = self.active;
        if active <= 1 {
            self.cores[active].leave();
            if core == active {
                let m = self.core_mode(mode);
                self.cores[active].set_parameter(if active == 0 { 2 } else { 1 }, m);
            }
        } else if core != active {
            self.cores[active].leave();
        }
    }

    /// `modeValueToCoreModeValue`: Classic 0 Volt Tonal, 1 Volt Punchy, 2 Bit; Clap 0
    /// Pow, 1 Clap.
    fn core_mode(&self, mode: usize) -> f32 {
        match mode {
            0 => {
                if self.shared[3] as u32 == 1 {
                    1.0
                } else {
                    0.0
                }
            }
            1 => 2.0,
            8 => 1.0,
            _ => 0.0,
        }
    }

    /// `updateGlobalCoreParameters`: the shared values into the playing core.
    fn update_global(&mut self) {
        let a = self.active;
        for idx in 1..12 {
            if let Some(ci) = core_index(a, idx) {
                self.cores[a].set_parameter(ci, self.shared[idx]);
                // Clap takes decay as 5 and then 2.
                if a == 1 && idx == 2 {
                    self.cores[a].set_parameter(2, self.shared[idx]);
                }
            }
        }
    }

    /// `SnareProcessor::setNoteOn(velocity, note)`.
    fn start_note(&mut self, vel: f32, note: u32) {
        if self.pending != self.active {
            self.active = self.pending;
            if self.active <= 1 {
                let mode = (self.shared[0] as u32 as usize).min(9);
                let m = self.core_mode(mode);
                self.cores[self.active].set_parameter(if self.active == 0 { 2 } else { 1 }, m);
            }
        }
        self.update_global();
        let a = self.active;
        let v = if a <= 1 {
            // Classic/Clap: 1 − |s|·(s < 0 ? v : 1 − v), cubed (s: Velocity).
            let s = self.shared[11];
            let w = if s < 0.0 { vel } else { 1.0 - vel };
            let x = (-w).mul_add(s.abs(), 1.0);
            x * (x * x)
        } else {
            vel
        };
        self.cores[a].note_on(v, note);
    }
}

/// The core parameter that shared parameter `idx` drives in `core`
/// (`updateGlobalCoreParameters`, spec §2.3).
fn core_index(core: usize, idx: usize) -> Option<u32> {
    // idx: 1 tune, 2 decay, 3 osc mode, 4 skin tune, 5 impact, 6 body, 7 spectra mode,
    // 8 spectra tune, 9 spectra amount, 10 spectra decay, 11 velocity.
    let t: [i8; 12] = match core {
        0 => [-1, 0, 1, -1, -1, -1, -1, -1, -1, -1, -1, -1],
        1 => [-1, 0, 5, -1, -1, -1, -1, -1, -1, -1, -1, -1],
        2 => [-1, 1, 2, -1, 0, 3, 6, 7, -1, 5, 4, 8],
        3..=5 => [-1, 1, 2, -1, 0, 3, -1, 7, 6, 4, 5, 8],
        6 => [-1, 1, 2, -1, 0, 3, 4, -1, -1, -1, -1, 6],
        _ => [-1, 0, 1, -1, 2, 3, -1, 7, 4, 6, 5, 8],
    };
    let i = t[idx];
    (i >= 0).then_some(i as u32)
}

impl Synth for Snare {
    fn set(&mut self, params: &[(String, f64)]) {
        // Connect order (`DATA::Snare::connectParameters`): mode, the shared ones, then
        // the engine ones. Only changed values are sent, as Maschine does.
        for (i, &(field, default)) in DEFAULTS.iter().enumerate() {
            let v = super::param(params, field, default);
            if self.applied[i] == v {
                continue;
            }
            self.applied[i] = v;
            if let Some(idx) = SHARED.iter().position(|(k, ..)| *k == field) {
                let (_, lo, hi) = SHARED[idx];
                // Enums and Velocity are bound without `getNormalized` (raw value).
                let x = if matches!(idx, 0 | 3 | 7 | 11) {
                    v as f32
                } else {
                    // `NumericParameter::getNormalized`.
                    ((v as f32) - lo) / (hi - lo)
                };
                self.set_shared(idx, x);
            } else if let Some(&(_, core, idx)) = CORE_PARAMS.iter().find(|(k, ..)| *k == field) {
                self.cores[core].set_parameter(idx, v as f32);
            }
        }
    }

    fn note_on(&mut self, note: i64, velocity: i64) {
        if self.n_events < self.events.len() {
            let v = velocity.clamp(0, 127) as f32 * 0.007874016;
            self.events[self.n_events] = Event::On(note.clamp(0, 127) as u32, v);
            self.n_events += 1;
        }
    }

    fn note_off(&mut self, note: i64) {
        if self.n_events < self.events.len() {
            self.events[self.n_events] = Event::Off(note.clamp(0, 127) as u32);
            self.n_events += 1;
        }
    }

    fn control(&mut self, kind: MidiKind) {
        // The wrapper would choke on CC 120, but Maschine does not pass it to the drum
        // synth from a host (measured: a render with CC 120 equals one without).
        let _ = kind;
    }

    fn stop(&mut self) {
        self.cores[self.active].choke();
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        // Events land at the start; only the first note-on at one position plays
        // (`DrumSynthWrapper::process`).
        let mut started = false;
        for i in 0..self.n_events {
            match self.events[i] {
                Event::On(n, v) => {
                    if !started {
                        started = true;
                        self.start_note(v, n);
                    }
                }
                Event::Off(n) => self.cores[self.active].note_off(n),
                Event::Choke => self.cores[self.active].choke(),
            }
        }
        self.n_events = 0;
        let n = l.len().min(r.len());
        self.block_start = 0;
        let mut at = 0;
        while at < n {
            if self.ctrl_left == 0 {
                self.cores[self.active].control_rate();
                self.ctrl_left = 128;
                // Maschine calls the wrapper per 128-frame engine block, each with its
                // own buffer.
                self.block_start = at;
                self.silent = false;
            }
            if self.piece_left == 0 {
                self.piece_left = 16;
            }
            // `DrumSynthWrapper::processAudio`: after a piece without output signal the
            // block is flagged silent, and the next piece clears the whole block buffer
            // first (so the block where the sound ends comes out as zeros).
            if self.silent {
                l[self.block_start..at].fill(0.0);
                r[self.block_start..at].fill(0.0);
                self.silent = false;
            }
            let k = (n - at).min(self.ctrl_left).min(self.piece_left);
            self.cores[self.active].process(&mut l[at..at + k], &mut r[at..at + k]);
            self.silent = !self.cores[self.active].has_output();
            self.ctrl_left -= k;
            self.piece_left -= k;
            at += k;
        }
    }

    fn voices(&self) -> usize {
        usize::from(self.cores[self.active].has_output())
    }
}

/// What a new Snare holds: [`DEFAULTS`].
pub fn defaults() -> Vec<(&'static str, f64)> {
    DEFAULTS.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(s: &mut Snare, n: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            s.process(a, b);
        }
        (l, r)
    }

    #[test]
    fn every_engine_renders_finite() {
        for (mode, name) in ENGINES.iter().enumerate() {
            let mut s = Snare::new(48_000.0);
            s.set(&[("mode".into(), mode as f64)]);
            s.note_on(60, 100);
            let (l, r) = render(&mut s, 48_000);
            assert!(l.iter().chain(&r).all(|x| x.is_finite()), "{name}");
        }
    }
}
