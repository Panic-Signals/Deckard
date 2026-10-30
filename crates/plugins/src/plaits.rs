//! Plaits: Mutable Instruments' macro-oscillator as a DECKARD instrument. The sound is
//! the module's own code (`deckard-mi`, Émilie Gillet's MIT-licensed DSP); this is the
//! instrument around it:
//! - its 24 engines and knobs (Harmonics, Timbre, Morph) and its low-pass gate (Decay,
//!   Colour), as on the module;
//! - a voice per note (up to 8, each a Plaits), the module's TRIG held while the key
//!   is (a gate, as the jack takes: the six-op engines play their envelopes for as long
//!   as it's high);
//! - Envelope: LPG (the module's gate, decaying with Decay) or Gate (a held note, its
//!   LEVEL patched with an envelope that releases with Release; the gate then closes
//!   over its Decay tail, as on the module);
//! - Out/Aux: between the module's two outputs (each engine's variant on AUX).
//!
//! Plaits runs at 48 kHz, tuned for the module's 47 872 Hz clock (corrected here); at
//! other rates its output is resampled.

use crate::Transport;
use crate::native::{Class, Engine, Page, Param};
use deckard_mi::{BLOCK, Modulations, Patch, Plaits};

pub static CLASS: Class = Class {
    cid: *b"DECKARD:Plaits01",
    name: "Plaits",
    instrument: true,
    params: PARAMS,
    pages: PAGES,
    make: |sr| Box::new(PlaitsSynth::new(sr)),
};

// Parameter ids (saved in projects: never renumber).
const ENGINE: u32 = 1;
const HARMONICS: u32 = 2;
const TIMBRE: u32 = 3;
const MORPH: u32 = 4;
const DECAY: u32 = 5;
const COLOUR: u32 = 6;
const LEVEL: u32 = 7;
const OUT_AUX: u32 = 8;
const ENVELOPE: u32 = 10;
const RELEASE: u32 = 11;
const VOICES: u32 = 12;
const VELOCITY: u32 = 13;

/// The engines, in the module's order: the second bank (green), then the first (red).
pub const ENGINES: [&str; 24] = [
    "VA VCF",
    "Phase Dist",
    "6-Op FM 1",
    "6-Op FM 2",
    "6-Op FM 3",
    "Wave Terrain",
    "String Mach",
    "Chiptune",
    "Virtual Analog",
    "Waveshaping",
    "2-Op FM",
    "Formant",
    "Additive",
    "Wavetable",
    "Chords",
    "Speech",
    "Swarm",
    "Noise",
    "Particle",
    "String",
    "Modal",
    "Bass Drum",
    "Snare Drum",
    "Hi-Hat",
];

/// What HARMONICS, TIMBRE and MORPH do in each engine, as the module's manual names
/// them.
const LABELS: [[&str; 3]; 24] = [
    ["RESONANCE", "CUTOFF", "SHAPE"],
    ["RATIO", "AMOUNT", "ASYMMETRY"],
    ["PRESET", "MOD LEVEL", "STRETCH"],
    ["PRESET", "MOD LEVEL", "STRETCH"],
    ["PRESET", "MOD LEVEL", "STRETCH"],
    ["TERRAIN", "RADIUS", "OFFSET"],
    ["CHORD", "CHORUS", "WAVEFORM"],
    ["CHORD", "ARPEGGIO", "PW / SYNC"],
    ["DETUNE", "SQUARE", "SAW"],
    ["WAVEFORM", "FOLD", "ASYMMETRY"],
    ["RATIO", "INDEX", "FEEDBACK"],
    ["RATIO", "FORMANT", "WIDTH"],
    ["BUMPS", "HARMONIC", "SHAPE"],
    ["BANK", "ROW", "COLUMN"],
    ["CHORD", "INVERSION", "WAVEFORM"],
    ["SYNTHESIS", "SPECIES", "PHONEME"],
    ["SPREAD", "DENSITY", "OVERLAP"],
    ["FILTER", "CLOCK", "RESONANCE"],
    ["SPREAD", "DENSITY", "FILTER"],
    ["MATERIAL", "BRIGHTNESS", "DECAY"],
    ["MATERIAL", "BRIGHTNESS", "DECAY"],
    ["ATTACK", "BRIGHTNESS", "DECAY"],
    ["NOISE", "MODES", "DECAY"],
    ["METAL", "HIGH-PASS", "DECAY"],
];

/// Engines with their own envelope: the low-pass gate (DECAY, COLOUR) is bypassed.
fn enveloped(engine: usize) -> bool {
    matches!(engine, 2..=4 | 19..=23)
}

/// Silent blocks (of 0.5 ms) after which a let-go voice is free: 50 ms.
const QUIET_BLOCKS: u32 = 100;

/// Voices a choice of the Voices knob gives.
const VOICE_COUNTS: [usize; 4] = [1, 2, 4, 8];
const MAX_VOICES: usize = 8;

const fn p(id: u32, name: &'static str, short: &'static str, default: f64, steps: i32) -> Param {
    Param { id, name, short, default, steps }
}

static PARAMS: &[Param] = &[
    p(ENGINE, "Engine", "ENGINE", 8.0 / 23.0, 23),
    p(HARMONICS, "Harmonics", "HARMONICS", 0.5, 0),
    p(TIMBRE, "Timbre", "TIMBRE", 0.5, 0),
    p(MORPH, "Morph", "MORPH", 0.5, 0),
    p(DECAY, "Decay", "DECAY", 0.5, 0),
    p(COLOUR, "LPG Colour", "COLOUR", 0.5, 0),
    p(LEVEL, "Level", "LEVEL", 0.7, 0),
    p(OUT_AUX, "Out / Aux", "OUT/AUX", 0.0, 0),
    p(ENVELOPE, "Envelope", "ENVELOPE", 0.0, 1),
    p(RELEASE, "Release", "RELEASE", 0.3, 0),
    p(VOICES, "Voices", "VOICES", 2.0 / 3.0, 3),
    p(VELOCITY, "Velocity", "VELOCITY", 0.5, 0),
];

static PAGES: &[Page] = &[
    Page {
        name: "Plaits",
        knobs: &[
            (ENGINE, "Model"),
            (HARMONICS, ""),
            (TIMBRE, ""),
            (MORPH, ""),
            (DECAY, "Low-Pass Gate"),
            (COLOUR, ""),
            (LEVEL, "Output"),
            (OUT_AUX, ""),
        ],
    },
    Page { name: "Voice", knobs: &[(ENVELOPE, "Voice"), (RELEASE, ""), (VOICES, ""), (VELOCITY, "")] },
];

fn step(v: f64, steps: usize) -> usize {
    ((v * steps as f64).round() as usize).min(steps)
}

/// How much the module's 47 872 Hz clock flattens its tuning at 48 kHz, in semitones.
fn tuning() -> f32 {
    (12.0 * (48_000.0f64 / 47_872.34).log2()) as f32
}

/// One voice: a Plaits and the note it plays.
struct Voice {
    plaits: Plaits,
    note: Option<u8>,
    gate: bool,
    velocity: f32,
    /// The Gate envelope's level, and when it started (for stealing the oldest).
    env: f32,
    age: u64,
    /// Blocks it has been silent since its key was let go (free again after a while).
    quiet: u32,
}

pub struct PlaitsSynth {
    sr: f64,
    voices: Vec<Voice>,
    patch: Patch,
    level: f32,
    out_aux: f32,
    gate_mode: bool,
    release: f32,
    count: usize,
    velocity: f32,
    bend: f32,
    clock: u64,
    /// Plaits' output at 48 kHz not yet resampled, and where the next output sample
    /// falls between its frames (`at` from `held[1]`).
    held: [f32; 4],
    at: f64,
    block: [f32; BLOCK],
    block_len: usize,
    block_pos: usize,
}

impl PlaitsSynth {
    pub fn new(sr: f64) -> PlaitsSynth {
        PlaitsSynth {
            sr,
            voices: (0..MAX_VOICES)
                .map(|_| Voice { plaits: Plaits::new(), note: None, gate: false, velocity: 1.0, env: 0.0, age: 0, quiet: 0 })
                .collect(),
            patch: Patch {
                note: 60.0,
                harmonics: 0.5,
                timbre: 0.5,
                morph: 0.5,
                engine: 8,
                decay: 0.5,
                lpg_colour: 0.5,
                ..Default::default()
            },
            level: 0.7,
            out_aux: 0.0,
            gate_mode: false,
            release: 0.3,
            count: 4,
            velocity: 0.5,
            bend: 0.0,
            clock: 0,
            held: [0.0; 4],
            at: 0.0,
            block: [0.0; BLOCK],
            block_len: 0,
            block_pos: 0,
        }
    }

    /// The next block of Plaits at 48 kHz: every sounding voice, mixed.
    fn render_block(&mut self) {
        let n = BLOCK;
        self.block = [0.0; BLOCK];
        let (mut out, mut aux) = ([0f32; BLOCK], [0f32; BLOCK]);
        // The Gate envelope: up in ~2 ms, down over Release (to 4 s).
        let attack = 1.0 - (-(n as f32) / (0.002 * 48_000.0)).exp();
        let release = 1.0 - (-(n as f32) / ((0.01 + 4.0 * self.release * self.release) * 48_000.0)).exp();
        for v in self.voices.iter_mut().take(self.count) {
            let Some(note) = v.note else { continue };
            v.env += ((if v.gate { 1.0 } else { 0.0 }) - v.env) * if v.gate { attack } else { release };
            let patch = Patch { note: note as f32 + self.bend * 2.0 - tuning(), ..self.patch };
            let mods = Modulations {
                trigger: if v.gate { 1.0 } else { 0.0 },
                trigger_patched: 1,
                level: v.env,
                level_patched: i32::from(self.gate_mode),
                ..Default::default()
            };
            v.plaits.render(&patch, &mods, &mut out, &mut aux);
            let gain = self.level * v.velocity;
            let mut loudest = 0f32;
            for i in 0..n {
                let x = out[i] * (1.0 - self.out_aux) + aux[i] * self.out_aux;
                loudest = loudest.max(x.abs());
                self.block[i] += gain * x;
            }
            // Let go and silent a while (its gate's tail over): free again.
            v.quiet = if !v.gate && loudest < 1e-4 { v.quiet + 1 } else { 0 };
            if v.quiet > QUIET_BLOCKS {
                v.note = None;
            }
        }
        self.block_len = n;
        self.block_pos = 0;
    }

    /// The next sample at 48 kHz.
    fn next_48k(&mut self) -> f32 {
        if self.block_pos >= self.block_len {
            self.render_block();
        }
        let s = self.block[self.block_pos];
        self.block_pos += 1;
        s
    }
}

impl Engine for PlaitsSynth {
    fn set_param(&mut self, id: u32, v: f64) {
        let f = v as f32;
        match id {
            ENGINE => self.patch.engine = step(v, 23) as i32,
            HARMONICS => self.patch.harmonics = f,
            TIMBRE => self.patch.timbre = f,
            MORPH => self.patch.morph = f,
            DECAY => self.patch.decay = f,
            COLOUR => self.patch.lpg_colour = f,
            LEVEL => self.level = f,
            OUT_AUX => self.out_aux = f,
            ENVELOPE => self.gate_mode = step(v, 1) == 1,
            RELEASE => self.release = f,
            VOICES => self.count = VOICE_COUNTS[step(v, 3)],
            VELOCITY => self.velocity = f,
            _ => {}
        }
    }

    fn text(&self, id: u32, v: f64) -> String {
        match id {
            ENGINE => ENGINES[step(v, 23)].into(),
            ENVELOPE => ["LPG", "Gate"][step(v, 1)].into(),
            VOICES => VOICE_COUNTS[step(v, 3)].to_string(),
            OUT_AUX => match (v * 100.0).round() as i64 {
                0 => "Out".into(),
                100 => "Aux".into(),
                n => format!("{} / {n}", 100 - n),
            },
            RELEASE => format!("{:.0} ms", (0.01 + 4.0 * v * v) * 1000.0),
            DECAY | COLOUR if enveloped(self.patch.engine as usize) => "unused".into(),
            _ => format!("{:.0} %", v * 100.0),
        }
    }

    fn label(&self, id: u32) -> Option<&'static str> {
        let names = LABELS.get(self.patch.engine as usize)?;
        match id {
            HARMONICS => Some(names[0]),
            TIMBRE => Some(names[1]),
            MORPH => Some(names[2]),
            _ => None,
        }
    }

    fn note_on(&mut self, pitch: u8, velocity: u8) {
        self.clock += 1;
        let count = self.count;
        // The voice already on this note, else a free one, else the oldest.
        let i = (0..count)
            .find(|&i| self.voices[i].note == Some(pitch))
            .or_else(|| (0..count).find(|&i| self.voices[i].note.is_none()))
            .unwrap_or_else(|| (0..count).min_by_key(|&i| self.voices[i].age).unwrap_or(0));
        let vel = velocity as f32 / 127.0;
        let v = &mut self.voices[i];
        v.note = Some(pitch);
        v.gate = true;
        v.quiet = 0;
        v.velocity = 1.0 - self.velocity + self.velocity * vel;
        v.age = self.clock;
    }

    fn note_off(&mut self, pitch: u8) {
        for v in self.voices.iter_mut().filter(|v| v.note == Some(pitch)) {
            v.gate = false;
        }
    }

    fn pitch_bend(&mut self, bend: f32) {
        self.bend = bend;
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            v.plaits.reset();
            (v.note, v.gate, v.env) = (None, false, 0.0);
        }
        self.block_len = 0;
        self.held = [0.0; 4];
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], _transport: &Transport) {
        let step = deckard_mi::SAMPLE_RATE / self.sr;
        for i in 0..left.len().min(right.len()) {
            let s = if (step - 1.0).abs() < 1e-9 {
                self.next_48k()
            } else {
                // Cubic (Hermite) between the 48 kHz frames around this output sample.
                while self.at >= 1.0 {
                    self.at -= 1.0;
                    let next = self.next_48k();
                    self.held = [self.held[1], self.held[2], self.held[3], next];
                }
                let [y0, y1, y2, y3] = self.held;
                let t = self.at as f32;
                let c1 = 0.5 * (y2 - y0);
                let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
                let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
                self.at += step;
                ((c3 * t + c2) * t + c1) * t + y1
            };
            left[i] = s;
            right[i] = s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(sr: f64, engine: f64, gate: bool) -> Vec<f32> {
        let mut s = PlaitsSynth::new(sr);
        s.set_param(ENGINE, engine);
        s.set_param(ENVELOPE, if gate { 1.0 } else { 0.0 });
        // A short release (about 20 ms) and gate tail, to hear it end.
        s.set_param(RELEASE, 0.05);
        s.set_param(DECAY, 0.1);
        s.note_on(57, 127);
        let mut out = Vec::new();
        let t = Transport::default();
        for k in 0..80 {
            if k == 20 {
                s.note_off(57);
            }
            let (mut l, mut r) = (vec![0f32; 512], vec![0f32; 512]);
            s.process(&mut l, &mut r, &t);
            out.extend(l);
        }
        out
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0f32, |m, v| m.max(v.abs()))
    }

    /// A held note sounds at 48 kHz and at 44.1 kHz (resampled), for an engine of each
    /// bank; in Gate mode it holds while the key does and fades after.
    #[test]
    fn notes_sound_and_gate_holds() {
        for sr in [48_000.0, 44_100.0] {
            for engine in [0.0, 8.0 / 23.0, 20.0 / 23.0] {
                let x = play(sr, engine, false);
                assert!(peak(&x) > 0.01 && peak(&x) < 1.5, "{sr} engine {engine}: {}", peak(&x));
            }
        }
        let x = play(48_000.0, 8.0 / 23.0, true);
        let held = peak(&x[512 * 15..512 * 20]);
        let after = peak(&x[512 * 70..]);
        assert!(held > 0.05, "held: {held}");
        assert!(after < held * 0.1, "released: {after} after {held}");
    }

    /// The VA engine's A 220 (MIDI 57) plays at 220 Hz: the module's clock corrected.
    #[test]
    fn it_is_in_tune() {
        let x = play(48_000.0, 8.0 / 23.0, true);
        let part = &x[512 * 5..512 * 19];
        let crossings = part.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f64;
        let hz = crossings / (part.len() as f64 / 48_000.0);
        assert!((hz - 220.0).abs() < 3.0, "{hz} Hz");
    }

    /// HARMONICS, TIMBRE and MORPH are named for the engine; the gate's knobs read
    /// unused where the engine has its own envelope.
    #[test]
    fn knobs_are_named_for_the_engine() {
        let mut s = PlaitsSynth::new(48_000.0);
        s.set_param(ENGINE, 8.0 / 23.0);
        assert_eq!([HARMONICS, TIMBRE, MORPH].map(|id| s.label(id)), [Some("DETUNE"), Some("SQUARE"), Some("SAW")]);
        assert_eq!(s.label(LEVEL), None);
        assert_eq!(s.text(DECAY, 0.5), "50 %");
        s.set_param(ENGINE, 21.0 / 23.0);
        assert_eq!(s.label(MORPH), Some("DECAY"));
        assert_eq!(s.text(DECAY, 0.5), "unused");
        assert_eq!(s.text(COLOUR, 0.5), "unused");
    }
}
