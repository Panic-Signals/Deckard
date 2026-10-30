//! Maschine's Hi-hat (docs/synth-hihat.md): `DATA::Hihat` →
//! `RT::DrumSynthWrapper<DSP::HihatProcessor>`, one monophonic drum voice with four
//! engines in three DSP cores: Silver and Circuit (`Hihat606Core`, [`c606`]), Memory
//! (`IFFTCymbalCore`) and Hybrid (`HybridHihatCore`).
//!
//! The wrapper runs the active core in blocks of at most 16 samples on a fixed grid,
//! with a control tick every 128 samples; a note switches the core when the engine
//! changed, re-sends the shared parameters and starts the voice. The cores follow the
//! arm64 code operation by operation so the output matches Maschine bit for bit.

mod c606;
mod env;
mod hybrid;
mod memory;
mod res;

use super::Synth;
use c606::Core606;
use memory::Memory;

/// The stored parameters, by field (crates/mxprj names) with the defaults of a new
/// Hi-hat (`DATA::Hihat::Hihat()`).
const FIELDS: [(&str, f64); 18] = [
    ("mode", 0.0),
    ("tune", 0.5),
    ("decay", 0.65),
    ("color", 0.715),
    ("impact", 0.8),
    ("gate", 0.0),
    ("velocity_scaling", 1.0),
    ("hihat606_sat", 0.19),
    ("hihat606_noise", 0.1),
    ("hihat606_seed", 0.2),
    ("hihat606_random", 0.25),
    ("cymbal_strike", 0.75),
    ("cymbal_sample", 0.0),
    ("cymbal_distress", 0.0),
    ("hybrid_metallic", 0.0),
    ("hybrid_bend", 0.5),
    ("hybrid_rattle", 0.5),
    ("hybrid_character", 0.25),
];

/// Wrapper block grid: the control rate and the longest `processAudio` call.
const CONTROL: u32 = 128;
const CHUNK: u32 = 16;
/// Maschine renders a sound in blocks of 256 samples.
const BLOCK: u32 = 256;

/// The cores (`HihatProcessor` +0x9c).
const SILVER_CIRCUIT: u32 = 0;
const MEMORY: u32 = 1;
const HYBRID: u32 = 2;

/// Events queued for the next `process` (the wrapper handles them at its start).
#[derive(Clone, Copy)]
enum Ev {
    On(u32, f32),
    Off,
}

pub struct Hihat {
    /// The stored values as last applied (`FIELDS` order).
    vals: [f64; 18],
    applied: bool,
    /// `HihatProcessor` +0x80: mode, tune, decay, gate, color, impact, velocity scaling.
    shared: [f32; 7],
    active: u32,  // +0x9c
    pending: u32, // +0xa0
    c606: Core606,
    mem: Memory,
    hyb: hybrid::Hybrid,
    ctl: u32,
    sub: u32,
    blk: u32,
    blk_start: usize,
    silent: bool,
    events: [Option<Ev>; 32],
    n_events: usize,
}

impl Hihat {
    pub fn new(sr: f64) -> Hihat {
        let sr = sr as f32;
        let mut h = Hihat {
            vals: [0.0; 18],
            applied: false,
            shared: [0.0; 7],
            active: SILVER_CIRCUIT,
            pending: SILVER_CIRCUIT,
            c606: Core606::new(sr),
            mem: Memory::new(sr),
            hyb: hybrid::Hybrid::new(sr),
            ctl: 0,
            sub: 0,
            blk: 0,
            blk_start: 0,
            silent: true,
            events: [None; 32],
            n_events: 0,
        };
        // DSPCoreWrapper::resume → onReset.
        h.c606.reset();
        h.mem.reset();
        h
    }

    /// `HihatProcessor::setParameter(i, v)` (a shared parameter).
    fn set_shared(&mut self, i: usize, v: f32) {
        self.shared[i] = v;
        match i {
            0 => {
                let m = v as i32;
                self.pending = if m == 3 { HYBRID } else { (m == 2) as u32 };
                if self.active == MEMORY && m != 2 {
                    self.mem.choke();
                }
                if self.active == HYBRID && m != 3 {
                    self.hyb.choke();
                }
                if self.active == SILVER_CIRCUIT {
                    self.c606.choke();
                    if self.pending == SILVER_CIRCUIT {
                        let s = if m != 0 { 0.0 } else { 1.0 };
                        self.c606.p[4] = s;
                        self.c606.silver_next = s > 0.0;
                    }
                }
            }
            1 if self.active == SILVER_CIRCUIT => self.c606.set_param(0, v),
            2 if self.active == SILVER_CIRCUIT => self.c606.set_param(1, v),
            3 if self.active == SILVER_CIRCUIT => self.c606.set_param(8, v),
            4 if self.active == SILVER_CIRCUIT => self.c606.set_param(2, v),
            1 if self.active == MEMORY => self.mem.set_param(0, v),
            2 if self.active == MEMORY => self.mem.set_param(1, v),
            3 if self.active == MEMORY => self.mem.set_param(6, v),
            4 if self.active == MEMORY => self.mem.set_param(4, v),
            5 if self.active == MEMORY => self.mem.set_param(2, v),
            // Hybrid: tune, decay, color, impact (its own numbering, see `hybrid`).
            1 => self.hyb.p[0] = v,
            2 => self.hyb.p[1] = v,
            4 => self.hyb.p[2] = v,
            5 => self.hyb.p[3] = v,
            6 => self.hyb.p[8] = v,
            _ => {}
        }
    }

    /// A field changed: route it as Maschine's `connectParameters` does.
    fn apply(&mut self, field: usize, v: f64) {
        let f = v as f32;
        match field {
            0 => self.set_shared(0, f),
            1 => self.set_shared(1, f),
            2 => self.set_shared(2, f),
            3 => self.set_shared(4, f),
            4 => self.set_shared(5, f),
            5 => self.set_shared(3, if v > 0.5 { 1.0 } else { 0.0 }),
            6 => self.set_shared(6, f),
            7 => self.c606.set_param(3, f),
            8 => self.c606.set_param(5, f),
            9 => self.c606.set_param(6, f),
            10 => self.c606.set_param(7, f),
            11 => self.mem.set_param(2, f),
            12 => self.mem.set_param(3, f),
            13 => self.mem.set_param(5, f),
            14 => self.hyb.p[4] = f,
            15 => self.hyb.p[5] = f,
            16 => self.hyb.p[6] = f,
            17 => self.hyb.p[7] = f,
            _ => {}
        }
    }

    /// `HihatProcessor::setNoteOn(v, note)`.
    fn start_note(&mut self, note: u32, v: f32) {
        if self.active != self.pending {
            self.active = self.pending;
            if self.active == SILVER_CIRCUIT {
                let s = if self.shared[0] as u32 != 0 { 0.0 } else { 1.0 };
                self.c606.set_param(4, s);
            }
        }
        // updateGlobalCoreParameters.
        if self.active == SILVER_CIRCUIT {
            let s = self.shared;
            self.c606.set_param(0, s[1]);
            self.c606.set_param(1, s[2]);
            self.c606.set_param(8, s[3]);
            self.c606.set_param(2, s[4]);
            // Velocity scaling: (1 − |V|·(V ≥ 0 ? 1 − v : v))³.
            let vs = s[6];
            let p = if vs < 0.0 { v } else { 1.0 - v };
            let a = p.mul_add(-vs.abs(), 1.0);
            self.c606.note_on(a * (a * a), note);
        } else if self.active == MEMORY {
            let s = self.shared;
            self.mem.set_param(0, s[1]);
            self.mem.set_param(1, s[2]);
            self.mem.set_param(6, s[3]);
            self.mem.set_param(4, s[4]);
            self.mem.set_param(2, s[5]);
            let vs = s[6];
            let p = if vs < 0.0 { v } else { 1.0 - v };
            let a = p.mul_add(-vs.abs(), 1.0);
            self.mem.note_on(a * (a * a), note);
        } else {
            let s = self.shared;
            for (k, i) in [(0, 1), (1, 2), (2, 4), (3, 5), (8, 6)] {
                self.hyb.p[k] = s[i];
            }
            self.hyb.note_on(v, note);
        }
    }

    fn choke(&mut self) {
        match self.active {
            SILVER_CIRCUIT => self.c606.choke(),
            MEMORY => self.mem.choke(),
            _ => self.hyb.choke(),
        }
    }

    fn handle_events(&mut self) {
        let mut started = false;
        for i in 0..self.n_events {
            match self.events[i].take() {
                Some(Ev::On(n, v)) => {
                    // One note-on per timestamp: later ones at the same time are dropped.
                    if !started {
                        self.start_note(n, v);
                        started = true;
                    }
                }
                Some(Ev::Off) => match self.active {
                    SILVER_CIRCUIT => self.c606.note_off(),
                    MEMORY => self.mem.note_off(),
                    _ => {}
                },
                None => {}
            }
        }
        self.n_events = 0;
    }

    fn push(&mut self, e: Ev) {
        if self.n_events < self.events.len() {
            self.events[self.n_events] = Some(e);
            self.n_events += 1;
        }
    }

    fn core_active(&self) -> bool {
        match self.active {
            SILVER_CIRCUIT => self.c606.active(),
            MEMORY => self.mem.active(),
            _ => self.hyb.active(),
        }
    }
}

impl Synth for Hihat {
    fn set(&mut self, params: &[(String, f64)]) {
        for (i, (name, default)) in FIELDS.iter().enumerate() {
            let v = super::param(params, name, *default);
            if !self.applied || v != self.vals[i] {
                self.vals[i] = v;
                self.apply(i, v);
            }
        }
        self.applied = true;
    }

    fn note_on(&mut self, note: i64, velocity: i64) {
        let v = (velocity.clamp(0, 127) as f32) * f32::from_bits(0x3c01_0204); // 1/127
        self.push(Ev::On(note.clamp(0, 127) as u32, v));
    }

    fn note_off(&mut self, _note: i64) {
        self.push(Ev::Off);
    }

    // MIDI controllers, pitch bend and CC 120 never reach the module in Maschine (the
    // wrapper's CC 120 choke is unreachable from a sound): the default `control` ignores
    // them.

    fn stop(&mut self) {
        self.choke();
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.handle_events();
        let n = l.len();
        // A block that began in an earlier call can only be cleared from here on.
        self.blk_start = 0;
        let mut pos = 0;
        while pos < n {
            if self.blk == 0 {
                // A new block of Maschine's audio engine.
                self.blk = BLOCK;
                self.blk_start = pos;
            }
            if self.ctl == 0 {
                match self.active {
                    SILVER_CIRCUIT => self.c606.control(),
                    MEMORY => self.mem.control(),
                    _ => {}
                }
                self.ctl = CONTROL;
            }
            if self.sub == 0 {
                self.sub = CHUNK;
            }
            let k = (n - pos).min(self.ctl as usize).min(self.sub as usize).min(self.blk as usize);
            if self.silent {
                // The block was marked silent by a voice that ended: clearing it before
                // writing wipes what this block already holds (AlignedAudioBlock).
                l[self.blk_start..pos].fill(0.0);
            }
            let out = &mut l[pos..pos + k];
            out.fill(0.0);
            match self.active {
                SILVER_CIRCUIT => {
                    self.c606.process(out);
                }
                MEMORY => self.mem.process(out),
                _ => self.hyb.process(out),
            }
            self.silent = !self.core_active();
            self.ctl -= k as u32;
            self.sub -= k as u32;
            self.blk -= k as u32;
            pos += k;
        }
        r.copy_from_slice(l);
    }

    fn voices(&self) -> usize {
        usize::from(self.core_active() || self.n_events > 0)
    }
}

/// What a new Hi-hat holds: Maschine's defaults (stored values, by field).
pub fn defaults() -> Vec<(&'static str, f64)> {
    vec![
        ("mode", 0.0),
        ("tune", 0.5),
        ("decay", 0.65),
        ("color", 0.715),
        ("impact", 0.8),
        ("gate", 0.0),
        ("velocity_scaling", 1.0),
        ("hihat606_sat", 0.19),
        ("hihat606_noise", 0.1),
        ("hihat606_seed", 0.2),
        ("hihat606_random", 0.25),
        ("cymbal_strike", 0.75),
        ("cymbal_sample", 0.0),
        ("cymbal_distress", 0.0),
        ("hybrid_metallic", 0.0),
        ("hybrid_bend", 0.5),
        ("hybrid_rattle", 0.5),
        ("hybrid_character", 0.25),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(extra: &[(&str, f64)]) -> Vec<(String, f64)> {
        let mut p: Vec<(String, f64)> = FIELDS.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        for (k, v) in extra {
            p.iter_mut().find(|(n, _)| n == k).unwrap().1 = *v;
        }
        p
    }

    /// Run `n` samples in blocks of `block`; returns the left channel.
    fn run(h: &mut Hihat, n: usize, block: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n);
        let (mut l, mut r) = (vec![0f32; block], vec![0f32; block]);
        while out.len() < n {
            let k = block.min(n - out.len());
            h.process(&mut l[..k], &mut r[..k]);
            assert_eq!(l[..k], r[..k]);
            out.extend_from_slice(&l[..k]);
        }
        out
    }

    fn make(extra: &[(&str, f64)], sr: f64) -> Hihat {
        let mut h = Hihat::new(sr);
        h.set(&params(extra));
        h
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    #[test]
    fn defaults_sound_then_return_to_silence() {
        for mode in [0.0, 1.0] {
            let mut h = make(&[("mode", mode)], 48_000.0);
            assert_eq!(h.voices(), 0);
            h.note_on(60, 100);
            let a = run(&mut h, 4800, 512);
            assert!(rms(&a) > 1e-3, "mode {mode}: {}", rms(&a));
            assert!(h.voices() > 0);
            h.note_off(60);
            let b = run(&mut h, 48_000 * 4, 512);
            assert!(b.iter().all(|v| v.is_finite()));
            assert_eq!(h.voices(), 0, "mode {mode}");
            let c = run(&mut h, 2048, 512);
            assert!(c.iter().all(|v| *v == 0.0));
        }
    }

    #[test]
    fn every_engine_renders_finite_and_block_size_does_not_matter() {
        for mode in [0.0, 1.0, 2.0, 3.0] {
            for &(f, v) in &[("decay", 1.0), ("hihat606_noise", 1.0), ("cymbal_distress", 1.0), ("gate", 1.0), ("color", 0.0)] {
                let p = [("mode", mode), (f, v)];
                let mut a = make(&p, 44_100.0);
                let mut b = make(&p, 44_100.0);
                a.note_on(48, 127);
                b.note_on(48, 127);
                let x = run(&mut a, 20_000, 512);
                let y = run(&mut b, 20_000, 37);
                assert!(x.iter().all(|s| s.is_finite() && s.abs() < 4.0));
                if mode < 2.0 {
                    assert_eq!(x, y, "mode {mode} {f}");
                    assert!(rms(&x) > 1e-4, "mode {mode} {f}");
                }
            }
        }
    }

    #[test]
    fn memory_plays_when_installed() {
        if res::cymbal(0).is_none() {
            return;
        }
        let mut h = make(&[("mode", 2.0)], 48_000.0);
        h.note_on(60, 100);
        assert!(rms(&run(&mut h, 9600, 256)) > 1e-3);
        run(&mut h, 48_000 * 6, 512);
        assert_eq!(h.voices(), 0);
    }

    #[test]
    fn set_mid_note_keeps_the_voice_and_glides() {
        let mut h = make(&[("mode", 0.0), ("decay", 1.0)], 48_000.0);
        h.note_on(60, 100);
        let a = run(&mut h, 4000, 256);
        // Re-sending the same values changes nothing.
        let mut g = make(&[("mode", 0.0), ("decay", 1.0)], 48_000.0);
        g.note_on(60, 100);
        run(&mut g, 4000, 256);
        g.set(&params(&[("mode", 0.0), ("decay", 1.0)]));
        assert_eq!(run(&mut g, 4000, 256), run(&mut h, 4000, 256));
        // A big Saturate change ramps over 20 ms instead of jumping, keeping the voice.
        h.set(&params(&[("mode", 0.0), ("decay", 1.0), ("hihat606_sat", 1.0)]));
        assert!(h.c606.cur[4] < h.c606.tgt[4]);
        let b = run(&mut h, 256, 256);
        assert!(h.voices() > 0 && a.iter().chain(&b).all(|v| v.is_finite()));
        run(&mut h, 2048, 256);
        assert_eq!(h.c606.cur[4], h.c606.tgt[4]);
    }

    #[test]
    fn velocity_scales_and_pitch_follows() {
        let level = |vel, note| {
            let mut h = make(&[("mode", 0.0)], 48_000.0);
            h.note_on(note, vel);
            rms(&run(&mut h, 4800, 512))
        };
        assert!(level(127, 60) > level(30, 60) * 2.0);
        let mut lo = make(&[("mode", 0.0), ("hihat606_noise", 0.0)], 48_000.0);
        let mut hi = make(&[("mode", 0.0), ("hihat606_noise", 0.0)], 48_000.0);
        lo.note_on(48, 100);
        hi.note_on(72, 100);
        assert_ne!(run(&mut lo, 2000, 512), run(&mut hi, 2000, 512));
    }

    #[test]
    fn stop_chokes_quickly() {
        let mut h = make(&[("mode", 0.0), ("decay", 1.0)], 48_000.0);
        h.note_on(60, 100);
        run(&mut h, 2000, 512);
        h.stop();
        run(&mut h, 4800, 512);
        assert_eq!(h.voices(), 0);
    }
}
