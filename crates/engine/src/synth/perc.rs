//! Maschine's Percussion drum synth (docs/synth-perc.md): `DATA::Perc` →
//! `RT::DrumSynthWrapper<DSP::PercProcessor>`, which plays one of three cores: Fractal
//! (`PercFBMatrixCore`), Kettle (`KettlePercCore`) and Shaker (`ShakerCore`). The engine
//! (`mode`) switches at the next note. One voice: a note restarts it.
//!
//! The wrapper runs the cores in blocks of at most 16 samples and calls their control
//! rate every 128 samples (counters kept across calls); notes land at the start of a
//! block. Note-off reaches only the Shaker; controllers and pitch bend don't reach it.

mod dsp;
mod fractal;
mod kettle;
mod shaker;

use super::{Synth, param};
use deckard_model::MidiKind;

/// Maschine's engine block (the wrapper's output buffer).
const BLOCK: usize = 128;

/// Fields of `DATA::Perc` (crates/mxprj names) and their defaults.
pub const FIELDS: [(&str, f64); 29] = [
    ("mode", 0.0),
    ("tune", 0.5),
    ("decay", 0.5),
    ("impact", 0.6),
    ("velocity", 1.0),
    ("shaker_grain", 0.5),
    ("shaker_filter", 0.7),
    ("shaker_env_mode", 1.0),
    ("shaker_attack", 0.4),
    ("shaker_hold", 0.0),
    ("shaker_release", 0.35),
    ("shaker_accent", 0.7),
    ("shaker_fill", 1.0),
    ("shaker_rate", 4.0),
    ("shaker_swing", 0.0),
    ("shaker_twist", 0.0),
    ("shaker_length", 3.0),
    ("shaker_offset", 0.0),
    ("shaker_sync", 1.0),
    ("fractal_freq_a", 0.5),
    ("fractal_freq_b", 0.4997),
    ("fractal_freq_c", 0.2906),
    ("fractal_fm", 0.3436),
    ("fractal_am", 0.1857),
    ("fractal_tune_hold", 0.0),
    ("fractal_keytrack", 0.0),
    ("fractal_glide", 0.0),
    ("kettle_puff", 0.25),
    ("kettle_damp", 0.0),
];

/// Core parameters: field → (core, index) as `DATA::Perc::connectParameters` binds them.
const CORE: [(&str, u32, u32); 24] = [
    ("shaker_grain", 2, 1),
    ("shaker_filter", 2, 2),
    ("shaker_env_mode", 2, 3),
    ("shaker_attack", 2, 4),
    ("shaker_hold", 2, 5),
    ("shaker_release", 2, 6),
    ("shaker_accent", 2, 7),
    ("shaker_fill", 2, 9),
    ("shaker_rate", 2, 10),
    ("shaker_swing", 2, 11),
    ("shaker_twist", 2, 12),
    ("shaker_length", 2, 13),
    ("shaker_offset", 2, 14),
    ("shaker_sync", 2, 16),
    ("fractal_freq_a", 0, 5),
    ("fractal_freq_b", 0, 6),
    ("fractal_freq_c", 0, 7),
    ("fractal_fm", 0, 8),
    ("fractal_am", 0, 9),
    ("fractal_tune_hold", 0, 0),
    ("fractal_keytrack", 0, 1),
    ("fractal_glide", 0, 4),
    ("kettle_puff", 1, 3),
    ("kettle_damp", 1, 4),
];

/// The Percussion instrument (`DrumSynthWrapper<PercProcessor>`).
pub struct Perc {
    sr: f32,
    fresh: bool,
    /// Shared parameters as last set: mode, tune, decay, impact, velocity.
    shared: [f32; 5],
    /// Values as last applied (unchanged ones are skipped in `set`).
    last: [f32; 29],
    /// The engine playing (`+0x10c`) and the one the next note uses (`+0x110`).
    mode: u32,
    next_mode: u32,
    fractal: fractal::Fractal,
    kettle: kettle::Kettle,
    shaker: shaker::Shaker,
    /// `PercProcessor` copies of the Shaker parameters (+0x94 + 4·idx).
    shaker_raw: [f32; 17],
    amp: f32,
    tempo: f64,
    /// Samples left to the next control-rate tick and to the end of the 16-sample block.
    ctl: usize,
    sub: usize,
    /// Position in the 128-frame audio block Maschine hands the wrapper (its
    /// `AlignedAudioBlock`), and the block's "silent" flag; frames since the start, and at
    /// the start of this `process` call (the host block's song position).
    pos: usize,
    clock: usize,
    call_clock: usize,
    silent: bool,
    pending: Option<(i64, i64)>,
}

impl Perc {
    pub fn new(sr: f64) -> Perc {
        dsp::exp_table();
        let mut p = Perc {
            sr: sr as f32,
            fresh: true,
            shared: [0.0; 5],
            last: [f32::NAN; 29],
            mode: 0,
            next_mode: 0,
            fractal: fractal::Fractal::new(),
            kettle: kettle::Kettle::new(),
            shaker: shaker::Shaker::new(),
            shaker_raw: [0.0; 17],
            amp: 0.0,
            tempo: 120.0,
            ctl: 0,
            sub: 0,
            pos: 0,
            clock: 0,
            call_clock: 0,
            silent: true,
            pending: None,
        };
        // DSPCoreWrapper's constructor: onInit, onSetSampleRate(44100, 128).
        p.shaker.set_sample_rate(44100.0, 128);
        p.fractal.set_sample_rate(44100.0, 128);
        p.kettle.set_sample_rate(44100.0, 128);
        p
    }

    /// The wrapper's sample-rate change and `resume`, after the module's parameters
    /// arrived.
    fn start(&mut self) {
        self.fresh = false;
        self.shaker.set_sample_rate(self.sr, 128);
        self.fractal.set_sample_rate(self.sr, 128);
        self.kettle.set_sample_rate(self.sr, 128);
        self.shaker.reset();
        self.fractal.reset();
        self.kettle.reset();
    }

    /// `PercProcessor::setParameter`: shared parameters go to the engine playing.
    fn set_shared(&mut self, idx: usize, v: f32) {
        self.shared[idx] = v;
        match idx {
            0 => {
                let new = v as i32 as u32;
                self.next_mode = new;
                if self.mode != new {
                    self.choke();
                }
            }
            1 if self.mode == 0 => self.fractal.set_param(2, v),
            1 if self.mode == 1 => self.kettle.set_param(0, v),
            2 if self.mode == 1 => self.kettle.set_param(1, v),
            3 if self.mode == 1 => self.kettle.set_param(2, v),
            4 if self.mode == 1 => self.kettle.set_param(5, v),
            1 if self.mode == 2 => self.shaker.set_param(0, v),
            2 if self.mode == 0 => self.fractal.set_param(3, v),
            3 if self.mode == 0 => self.fractal.set_param(10, v),
            _ => {}
        }
    }

    /// `PercProcessor::setCoreParameter`: every core keeps its own parameters.
    fn set_core(&mut self, core: u32, idx: u32, v: f32) {
        match core {
            0 => self.fractal.set_param(idx, v),
            1 => self.kettle.set_param(idx, v),
            2 => self.set_shaker(idx, v),
            _ => {}
        }
    }

    /// The Shaker branch of `setCoreParameter`: Grain, Filter and Release follow the
    /// note's velocity (except in Performer), Fill and Length are shifted to −1..1 and
    /// 1..8, Offset is split into steps and a fraction.
    fn set_shaker(&mut self, idx: u32, v: f32) {
        self.shaker_raw[idx as usize] = v;
        let vel = self.shared[4];
        let am1 = self.amp + -1.0;
        if self.shaker.env_mode() == 2 {
            match idx {
                6 => {
                    self.shaker.set_param(8, v);
                    self.shaker.set_param(6, v);
                }
                9 => self.shaker.set_param(9, -1.0 + v),
                13 => self.shaker.set_param(13, 1.0 + v),
                _ => self.set_shaker_direct(idx, v),
            }
            return;
        }
        let s = &mut self.shaker;
        match idx {
            1 => s.set_param(1, dsp_clamp(am1.mul_add(-vel, v))),
            2 => s.set_param(2, dsp_clamp(am1.mul_add(vel * f32::from_bits(0x3e71_de6a), v))),
            6 => {
                let r = dsp_clamp(am1.mul_add(vel * f32::from_bits(0x3ed5_a858), v));
                s.set_param(8, r);
                s.set_param(6, r);
            }
            9 => s.set_param(9, -1.0 + v),
            13 => s.set_param(13, 1.0 + v),
            _ => self.set_shaker_direct(idx, v),
        }
    }

    fn set_shaker_direct(&mut self, idx: u32, v: f32) {
        self.shaker.set_param(idx, v);
        if idx == 14 {
            let x = v * 4.0;
            let h = if x < 0.0 { -0.5 } else { 0.5 };
            let n = (h + x as f64) as i32 as f32;
            self.shaker.set_param(14, n);
            self.shaker.set_param(15, x - n);
        } else if idx == 3 {
            self.shaker.choke();
            self.modulate();
        }
    }

    /// `updateModulatedParameters` (Shaker): Grain, Filter, Release from velocity.
    fn modulate(&mut self) {
        if self.mode != 2 {
            return;
        }
        let r = &self.shaker_raw;
        let (g, f, rel) = (r[1], r[2], r[6]);
        let s = &mut self.shaker;
        if s.env_mode() == 2 {
            s.set_param(1, g);
            s.set_param(2, f);
            s.set_param(6, rel);
            return;
        }
        let vel = self.shared[4];
        let am1 = self.amp + -1.0;
        s.set_param(1, dsp_clamp((-am1).mul_add(vel, g)));
        s.set_param(2, dsp_clamp(am1.mul_add(vel * f32::from_bits(0x3e71_de6a), f)));
        s.set_param(6, dsp_clamp(am1.mul_add(vel * f32::from_bits(0x3ed5_a858), rel)));
    }

    fn choke(&mut self) {
        match self.mode {
            0 => self.fractal.choke(),
            1 => self.kettle.choke(),
            2 => self.shaker.choke(),
            _ => {}
        }
    }

    /// `PercProcessor::setNoteOn`: switch engine, re-apply the shared parameters, and
    /// start the note.
    fn note(&mut self, key: i64, vel: i64) {
        let v = vel.clamp(0, 127) as f32 * f32::from_bits(0x3c01_0204);
        self.mode = self.next_mode;
        // Velocity scaling: (1 − x·|s|)³ with x = v (s < 0) or 1 − v.
        let s = self.shared[4];
        let x = if s < 0.0 { v } else { 1.0 - v };
        let a = (-x).mul_add(s.abs(), 1.0);
        let amp = a * (a * a);
        self.amp = amp;
        if self.mode == 2 {
            self.shaker.set_param(0, self.shared[1]);
            self.modulate();
            self.shaker.note_on(amp, key.clamp(0, 127) as u32);
        }
        if self.mode == 0 {
            let [_, tune, decay, impact, _] = self.shared;
            self.fractal.set_param(2, tune);
            self.fractal.set_param(3, decay);
            self.fractal.set_param(10, impact);
            self.fractal.note_on(amp, key.clamp(0, 127) as u32);
        }
        if self.mode == 1 {
            // `updateGlobalCoreParameters`, then the core's note-on with the raw velocity.
            let [_, tune, decay, impact, vel] = self.shared;
            self.kettle.set_param(0, tune);
            self.kettle.set_param(1, decay);
            self.kettle.set_param(2, impact);
            self.kettle.set_param(5, vel);
            self.kettle.note_on(v, key.clamp(0, 127) as u32);
        }
    }

    fn transport(&mut self) {
        if self.mode == 2 {
            self.shaker.set_tempo(self.tempo as f32 as f64);
            // The host's song position at the start of its block, in quarters; DECKARD's
            // runs on from the first block (stopped), like maschine-ref's.
            let q = self.call_clock as f64 / self.sr as f64 * self.tempo / 60.0;
            self.shaker.position(q, false);
        }
    }

    fn core_has_output(&self) -> bool {
        match self.mode {
            0 => self.fractal.has_output(),
            1 => self.kettle.has_output(),
            2 => self.shaker.has_output(),
            _ => false,
        }
    }
}

impl Synth for Perc {
    fn set(&mut self, params: &[(String, f64)]) {
        for (i, (name, def)) in FIELDS.iter().enumerate() {
            let v = param(params, name, *def) as f32;
            if v.to_bits() == self.last[i].to_bits() {
                continue;
            }
            self.last[i] = v;
            if i < 5 {
                self.set_shared(i, v);
            } else if let Some(&(_, core, idx)) = CORE.iter().find(|c| c.0 == *name) {
                self.set_core(core, idx, v);
            }
        }
        if self.fresh {
            self.start();
        }
    }

    fn note_on(&mut self, note: i64, velocity: i64) {
        // The wrapper takes one note-on per time stamp.
        if self.pending.is_none() {
            self.pending = Some((note, velocity));
        }
    }

    fn note_off(&mut self, _note: i64) {
        if self.mode == 2 {
            self.shaker.note_off();
        }
    }

    fn set_tempo(&mut self, bpm: f64) {
        self.tempo = bpm;
    }

    /// Maschine delivers no controller or pitch bend to the module (null-tested: a
    /// CC 120 and a bend leave the output bit-identical), so none has an effect. The
    /// wrapper itself would choke on CC 120 if one arrived.
    fn control(&mut self, _kind: MidiKind) {}

    fn stop(&mut self) {
        self.choke();
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.fresh {
            self.start();
        }
        // The host's 128-frame blocks start with `PercProcessor::onBPMChanged` and
        // `onPositionChanged` (which reach the Shaker only while it plays), then events.
        self.call_clock = self.clock;
        let mut host = self.ctl == 0;
        if host {
            self.transport();
        }
        if let Some((k, v)) = self.pending.take() {
            self.note(k, v);
        }
        let n = l.len().min(r.len());
        let mut at = 0;
        while at < n {
            if self.ctl == 0 {
                if !host {
                    self.transport();
                }
                host = false;
                match self.mode {
                    0 => self.fractal.control(),
                    1 => self.kettle.control(),
                    2 => self.shaker.control(),
                    _ => {}
                }
                self.ctl = 128;
            }
            if self.sub == 0 {
                self.sub = 16;
            }
            let k = (n - at).min(self.ctl).min(self.sub).min(BLOCK - self.pos);
            // A chunk after one that ended silent clears the engine block so far.
            if self.silent {
                let from = at.saturating_sub(self.pos);
                l[from..at].fill(0.0);
                r[from..at].fill(0.0);
                self.silent = false;
            }
            let (bl, br) = (&mut l[at..at + k], &mut r[at..at + k]);
            match self.mode {
                0 => self.fractal.process(bl, br),
                1 => self.kettle.process(bl, br),
                2 => self.shaker.process(bl, br),
                _ => {
                    bl.fill(0.0);
                    br.fill(0.0);
                }
            }
            let quiet = !self.core_has_output();
            self.ctl -= k;
            self.sub -= k;
            self.pos += k;
            self.clock += k;
            at += k;
            self.silent = quiet;
            if self.pos == BLOCK {
                // A block whose last chunk ended without output is silence as a whole.
                if quiet {
                    let from = at.saturating_sub(BLOCK);
                    l[from..at].fill(0.0);
                    r[from..at].fill(0.0);
                }
                self.pos = 0;
                self.silent = true;
            }
        }
    }

    fn voices(&self) -> usize {
        (self.core_has_output() || self.pending.is_some()) as usize
    }
}

fn dsp_clamp(x: f32) -> f32 {
    dsp::clamp01(x)
}

/// What a new Percussion holds: Maschine's defaults (stored values, by field).
pub fn defaults() -> Vec<(&'static str, f64)> {
    vec![
        ("mode", 0.0),
        ("tune", 0.5),
        ("decay", 0.5),
        ("impact", 0.6),
        ("velocity", 1.0),
        ("shaker_grain", 0.5),
        ("shaker_filter", 0.7),
        ("shaker_env_mode", 1.0),
        ("shaker_attack", 0.4),
        ("shaker_hold", 0.0),
        ("shaker_release", 0.35),
        ("shaker_accent", 0.7),
        ("shaker_fill", 1.0),
        ("shaker_rate", 4.0),
        ("shaker_swing", 0.0),
        ("shaker_twist", 0.0),
        ("shaker_length", 3.0),
        ("shaker_offset", 0.0),
        ("shaker_sync", 1.0),
        ("fractal_freq_a", 0.5),
        ("fractal_freq_b", 0.4997),
        ("fractal_freq_c", 0.2906),
        ("fractal_fm", 0.3436),
        ("fractal_am", 0.1857),
        ("fractal_tune_hold", 0.0),
        ("fractal_keytrack", 0.0),
        ("fractal_glide", 0.0),
        ("kettle_puff", 0.25),
        ("kettle_damp", 0.0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(mode: f64) -> Vec<(String, f64)> {
        let mut p: Vec<(String, f64)> = FIELDS.iter().map(|(n, d)| (n.to_string(), *d)).collect();
        p[0].1 = mode;
        p
    }

    fn render(s: &mut Perc, n: usize) -> Vec<f32> {
        let mut out = Vec::new();
        let (mut l, mut r) = (vec![0f32; 512], vec![0f32; 512]);
        let mut left = n;
        while left > 0 {
            let k = left.min(512);
            s.process(&mut l[..k], &mut r[..k]);
            out.extend_from_slice(&l[..k]);
            left -= k;
        }
        out
    }

    /// The engines (Engine, Shaker Env Mode) and their names.
    /// The engines DECKARD plays (Kettle isn't ported yet).
    const ENGINES: [(f64, f64, &str); 4] = [(0.0, 1.0, "Fractal"), (2.0, 0.0, "Machine"), (2.0, 1.0, "Realistic"), (2.0, 2.0, "Performer")];

    fn engine(mode: f64, env: f64) -> Vec<(String, f64)> {
        let mut p = params(mode);
        p.iter_mut().find(|(n, _)| n == "shaker_env_mode").unwrap().1 = env;
        p
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0f32, |a, v| a.max(v.abs()))
    }

    #[test]
    fn every_engine_sounds_and_stops() {
        for (mode, env, name) in ENGINES {
            let mut s = Perc::new(48000.0);
            s.set(&engine(mode, env));
            assert_eq!(s.voices(), 0, "{name}");
            s.note_on(60, 100);
            assert_eq!(s.voices(), 1, "{name}");
            let out = render(&mut s, 24000);
            assert!(peak(&out) > 0.01, "{name}: peak {}", peak(&out));
            assert!(out.iter().all(|x| x.is_finite()), "{name}");
            s.note_off(60);
            let tail = render(&mut s, 48000 * 20);
            assert!(tail.iter().all(|x| x.is_finite()), "{name}");
            assert_eq!(s.voices(), 0, "{name}");
            let last = render(&mut s, 1024);
            assert!(last.iter().all(|x| *x == 0.0), "{name}");
        }
    }

    #[test]
    fn extremes_stay_finite() {
        for (mode, env, name) in ENGINES {
            for v in [0.0, 1.0] {
                let mut p = engine(mode, env);
                for (n, x) in p.iter_mut() {
                    if !matches!(n.as_str(), "mode" | "shaker_env_mode") {
                        *x = if n == "velocity" || n == "shaker_offset" || n == "shaker_swing" || n == "shaker_twist" {
                            2.0 * v - 1.0
                        } else if n == "shaker_fill" {
                            2.0 * v
                        } else if n == "shaker_rate" || n == "shaker_length" {
                            6.0 * v
                        } else {
                            v
                        };
                    }
                }
                for (key, vel) in [(0, 1), (127, 127), (60, 64)] {
                    let mut s = Perc::new(44100.0);
                    s.set(&p);
                    s.note_on(key, vel);
                    let out = render(&mut s, 44100);
                    assert!(out.iter().all(|x| x.is_finite() && x.abs() < 8.0), "{name} {v} {key}");
                }
            }
        }
    }

    #[test]
    fn set_mid_note_keeps_playing_without_clicks() {
        let jump = |x: &[f32]| x.windows(2).fold(0f32, |a, w| a.max((w[1] - w[0]).abs()));
        for (mode, env, name) in ENGINES {
            let p = engine(mode, env);
            // Every continuous parameter moves a little; the twin keeps them. (The Kettle
            // retunes its modal bank in one step at the next control tick, as Maschine
            // does, so its Tune is left out.)
            let q: Vec<(String, f64)> = p
                .iter()
                .map(|(n, v)| {
                    let fixed = matches!(
                        n.as_str(),
                        "mode"
                            | "shaker_env_mode"
                            | "shaker_fill"
                            | "shaker_rate"
                            | "shaker_length"
                            | "shaker_sync"
                            | "fractal_keytrack"
                            | "fractal_tune_hold"
                    ) || (mode == 1.0 && n == "tune");
                    (n.clone(), if fixed { *v } else { (v + 0.05).min(1.0) })
                })
                .collect();
            let (mut s, mut twin) = (Perc::new(48000.0), Perc::new(48000.0));
            for x in [&mut s, &mut twin] {
                x.set(&p);
                x.note_on(48, 127);
                render(x, 2400);
            }
            assert_eq!(s.voices(), 1, "{name}");
            s.set(&q);
            let (a, b) = (render(&mut s, 2400), render(&mut twin, 2400));
            assert!(peak(&a[..256]) > 0.0, "{name}: stopped");
            let (j, jt) = (jump(&a[..256]), jump(&b[..256]));
            assert!(j <= 1.5 * jt + 1e-3, "{name}: jump {j} vs {jt}");
        }
    }

    #[test]
    fn mode_changes_at_the_next_note() {
        let mut s = Perc::new(48000.0);
        s.set(&engine(0.0, 1.0));
        s.note_on(60, 100);
        render(&mut s, 4800);
        // Switching the engine chokes the playing one; the next note plays the new one.
        s.set(&engine(2.0, 1.0));
        render(&mut s, 48000);
        assert_eq!(s.voices(), 0);
        s.note_on(60, 100);
        assert!(peak(&render(&mut s, 4800)) > 0.01);
        assert_eq!(s.mode, 2);
    }

    #[test]
    fn fractal_defaults_sound_and_stop() {
        let mut s = Perc::new(48000.0);
        s.set(&params(0.0));
        assert_eq!(s.voices(), 0);
        s.note_on(60, 100);
        let out = render(&mut s, 48000);
        let peak = out.iter().fold(0f32, |a, x| a.max(x.abs()));
        assert!(peak > 0.01, "peak {peak}");
        assert!(out.iter().all(|x| x.is_finite()));
        let tail = render(&mut s, 48000 * 10);
        assert!(tail.iter().all(|x| x.is_finite()));
        assert_eq!(s.voices(), 0);
        let last = render(&mut s, 1024);
        assert!(last.iter().all(|x| *x == 0.0));
    }
}
