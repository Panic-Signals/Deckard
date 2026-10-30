//! Maschine's Kick (docs/synth-kick.md): `DATA::Kick` → `RT::DrumSynthWrapper<DSP::KickProcessor>`.
//! The processor holds one core per engine (Sub, Tronic, Dusty, Grit, Rasper, Snappy,
//! Bold, Maple, Push) and plays the one the Engine selector chose at the last note-on.
//! Each core is a monophonic one-shot: a new note restarts it, note-offs are ignored
//! (except by Grit). The wrapper calls `processControlRate` every 128 samples and
//! `processAudio` in chunks of at most 16.

mod bd;
mod dusty;
mod env;
mod grit;
mod simd;
mod smooth;
mod sub;
mod tronic;

use super::{Synth, param};
use deckard_model::MidiKind;

/// The control-rate block (`DrumSynthWrapper`: 0x80).
const CONTROL: u32 = 128;
/// The audio chunk (`DrumSynthWrapper`: 0x10).
const CHUNK: u32 = 16;
/// The sample rate of an `onSetSampleRate` call before the real one (−1: none). Tronic's
/// click counter reads it (a Maschine quirk).
const PREV_SR: f32 = 44100.0;
/// `1/127` as the wrapper multiplies the MIDI velocity (`0x3c010204`).
const VEL_SCALE: f32 = f32::from_bits(0x3c01_0204);

/// The Engine selector (`engine`), in order.
pub const ENGINES: [&str; 9] = ["Sub", "Tronic", "Dusty", "Grit", "Rasper", "Snappy", "Bold", "Maple", "Push"];

/// A parameter: field, default, and where it goes.
struct Field {
    name: &'static str,
    default: f32,
    to: To,
}

#[derive(Clone, Copy)]
enum To {
    /// `KickProcessor::setParameter(i)`: 0 mode, 1 tune, 2 decay, 3 punch, 4 bend,
    /// 5 impact, 6 velocity.
    Shared(usize),
    /// `setCoreParameter(engine, i)`.
    Core(usize, usize),
}

/// Fields in `DATA::Kick::connectParameters` order (0x1019c8fbc), which is the order
/// Maschine applies them in.
const FIELDS: [Field; 31] = [
    Field { name: "engine", default: 0.0, to: To::Shared(0) },
    Field { name: "tune", default: 0.5, to: To::Shared(1) },
    Field { name: "decay", default: 0.5, to: To::Shared(2) },
    Field { name: "punch", default: 1.0, to: To::Shared(3) },
    Field { name: "bend", default: f32::from_bits(0x3d8b_4396), to: To::Shared(4) },
    Field { name: "impact", default: 0.75, to: To::Shared(5) },
    Field { name: "velocity", default: 1.0, to: To::Shared(6) },
    Field { name: "sub_kick_p_env_decay", default: f32::from_bits(0x3e99_999a), to: To::Core(0, 3) },
    Field { name: "sub_kick_thin_attack", default: 0.0, to: To::Core(0, 4) },
    Field { name: "sub_kick_color", default: 0.5, to: To::Core(0, 6) },
    Field { name: "sub_kick_attack_amount", default: 0.5, to: To::Core(0, 5) },
    Field { name: "clean_bd_drive_gain", default: f32::from_bits(0x3f33_3333), to: To::Core(1, 4) },
    Field { name: "clean_bd_drive_tone", default: 0.25, to: To::Core(1, 5) },
    Field { name: "clean_bd_bias", default: 0.5, to: To::Core(1, 6) },
    Field { name: "clean_bd_drive_dw", default: 0.0, to: To::Core(1, 7) },
    Field { name: "dusty_kick_filter", default: f32::from_bits(0x3ecc_cccd), to: To::Core(2, 2) },
    Field { name: "dusty_kick_noise", default: f32::from_bits(0x3dcc_cccd), to: To::Core(2, 4) },
    Field { name: "grit_kick_gate", default: 0.0, to: To::Core(3, 4) },
    Field { name: "grit_kick_aero_grind", default: f32::from_bits(0x3ee6_6666), to: To::Core(3, 5) },
    Field { name: "grit_kick_aero_amount", default: f32::from_bits(0x3e4c_cccd), to: To::Core(3, 6) },
    Field { name: "acoustic_supporter_tune_skin", default: 0.5, to: To::Core(7, 0) },
    Field { name: "acoustic_supporter_room_size", default: 0.5, to: To::Core(7, 4) },
    Field { name: "acoustic_supporter_room_amount", default: 0.75, to: To::Core(7, 5) },
    Field { name: "acoustic_supporter_room_character", default: 0.0, to: To::Core(7, 6) },
    Field { name: "medio_snap", default: 0.5, to: To::Core(5, 3) },
    Field { name: "medio_punch_amount", default: 1.0, to: To::Core(5, 4) },
    Field { name: "medio_punch_decay", default: 0.5, to: To::Core(5, 5) },
    Field { name: "medio_punch_mode", default: 0.0, to: To::Core(5, 6) },
    Field { name: "jungle_crispness_mode", default: 0.0, to: To::Core(4, 6) },
    Field { name: "jungle_crispness_decay", default: f32::from_bits(0x3eae_147b), to: To::Core(4, 4) },
    Field { name: "jungle_crispness_amount", default: 0.5, to: To::Core(4, 5) },
];

pub struct Kick {
    /// `KickProcessor` +0x80…: the shared parameters as last set.
    shared: [f32; 7],
    /// +0x9c: the engine playing; +0xa0 the engine for the next note.
    mode: usize,
    pending: usize,
    sub: sub::SubKick,
    tronic: tronic::Tronic,
    dusty: dusty::Dusty,
    grit: grit::Grit,
    /// Rasper, Snappy, Bold, Maple, Push (engines 4…8).
    bd: [bd::Bd; 5],
    /// The values applied last (by `FIELDS` index): only changes are passed on.
    applied: [Option<f32>; 31],
    /// Samples to the next control-rate call and to the next chunk end.
    ctrl_left: u32,
    chunk_left: u32,
    /// Whether the last chunk sounded.
    sounding: bool,
    /// Note events since the last `process` (they land at its start).
    events: [Event; MAX_EVENTS],
    n_events: usize,
}

/// The modal core's `setParameter` index for shared parameter `i` (1 tune, 2 decay,
/// 3 punch, 5 impact, 6 velocity) under engine `mode` (4…8), as `KickProcessor`'s
/// setters and `updateGlobalCoreParameters` route them; `None` where the core has none.
fn bd_index(mode: usize, i: usize) -> Option<usize> {
    match (mode, i) {
        // Rasper (JungleBDCore), Bold (SubSnappyBDCore), Push (PusherBDCore).
        (4 | 6 | 8, 1) => Some(0),
        (4 | 6 | 8, 2) => Some(1),
        (4 | 6 | 8, 5) => Some(2),
        (4 | 6 | 8, 3) => Some(3),
        (4, 6) => Some(7),
        (6 | 8, 6) => Some(4),
        // Snappy (MedioBDCore).
        (5, 1) => Some(0),
        (5, 2) => Some(1),
        (5, 5) => Some(2),
        (5, 6) => Some(7),
        // Maple (AcousticSupporterBDCore): Skin Tune is its parameter 0.
        (7, 1) => Some(1),
        (7, 2) => Some(2),
        (7, 5) => Some(3),
        (7, 6) => Some(7),
        _ => None,
    }
}

/// A queued note event.
#[derive(Clone, Copy, Debug)]
enum Event {
    On(u32, f32),
    Off,
}

/// Note events kept per `process` call (more are dropped).
const MAX_EVENTS: usize = 32;

impl Kick {
    pub fn new(sr: f64) -> Kick {
        let sr = sr as f32;
        Kick {
            shared: [0.0; 7],
            mode: 0,
            pending: 0,
            sub: sub::SubKick::new(sr),
            tronic: tronic::Tronic::new(sr, PREV_SR),
            dusty: dusty::Dusty::new(sr),
            grit: grit::Grit::new(sr),
            bd: [
                bd::Bd::new(bd::Kind::Jungle, sr),
                bd::Bd::new(bd::Kind::Medio, sr),
                bd::Bd::new(bd::Kind::SubSnappy, sr),
                bd::Bd::new(bd::Kind::Acoustic, sr),
                bd::Bd::new(bd::Kind::Pusher, sr),
            ],
            applied: [None; 31],
            ctrl_left: 0,
            chunk_left: 0,
            sounding: false,
            events: [Event::Off; MAX_EVENTS],
            n_events: 0,
        }
    }

    /// `KickProcessor::setParameter(i, v)` (0x1019f4b58).
    fn set_shared(&mut self, i: usize, v: f32) {
        self.shared[i] = v;
        match i {
            0 => {
                self.pending = (v as i32).clamp(0, 8) as usize;
                if self.mode != self.pending {
                    self.release(self.mode);
                }
            }
            _ => self.core_shared(i, v),
        }
    }

    /// A shared parameter reaching the playing engine (`setParamTune`, `setParamDecay`,
    /// `setParamPunch`, `setParamBend`, `setParamImpact`).
    fn core_shared(&mut self, i: usize, v: f32) {
        match (self.mode, i) {
            (0, 1) => self.sub.set(0, v),
            (0, 2) => self.sub.set(1, v),
            (0, 4) => self.sub.set(2, v),
            (1, 1) => self.tronic.set(0, v),
            (1, 2) => self.tronic.set(1, v),
            (1, 4) => self.tronic.set(2, v),
            (1, 5) => self.tronic.set(3, v),
            (2, 1) => self.dusty.set(0, v),
            (2, 2) => self.dusty.set(1, v),
            (2, 5) => self.dusty.set_impact(v),
            (3, 1) => self.grit.set(0, v),
            (3, 2) => self.grit.set(1, v),
            (3, 4) => self.grit.set(2, v),
            (3, 5) => self.grit.set(3, v),
            (4..=8, _) => {
                if let Some(k) = bd_index(self.mode, i) {
                    self.bd[self.mode - 4].set(k, v);
                }
            }
            _ => {}
        }
    }

    /// `updateGlobalCoreParameters()` (0x1019f570c): the shared values into the engine
    /// about to play.
    fn update_global(&mut self) {
        let s = self.shared;
        match self.mode {
            0 => {
                self.sub.set(0, s[1]);
                self.sub.set(1, s[2]);
                self.sub.set(2, s[4]);
            }
            1 => {
                self.tronic.set(0, s[1]);
                self.tronic.set(1, s[2]);
                self.tronic.set(2, s[4]);
                self.tronic.set(3, s[5]);
            }
            2 => {
                self.dusty.set(0, s[1]);
                self.dusty.set(1, s[2]);
                self.dusty.set_impact(s[5]);
            }
            3 => {
                self.grit.set(0, s[1]);
                self.grit.set(1, s[2]);
                self.grit.set(2, s[4]);
                self.grit.set(3, s[5]);
            }
            m @ 4..=8 => {
                // Tune, decay, impact, punch (where the core has it), then velocity.
                for i in [1, 2, 5, 3, 6] {
                    if let Some(k) = bd_index(m, i) {
                        self.bd[m - 4].set(k, s[i]);
                    }
                }
            }
            _ => {}
        }
    }

    /// Fade out an engine's gate (on an engine switch or a choke).
    fn release(&mut self, mode: usize) {
        match mode {
            0 => self.sub.choke(),
            1 => self.tronic.choke(),
            2 => self.dusty.choke(),
            3 => self.grit.choke(),
            4..=8 => self.bd[mode - 4].choke(),
            _ => {}
        }
    }

    fn set_core(&mut self, engine: usize, i: usize, v: f32) {
        match engine {
            0 => self.sub.set(i, v),
            1 => self.tronic.set(i, v),
            2 => self.dusty.set(i, v),
            3 => self.grit.set(i, v),
            4..=8 => self.bd[engine - 4].set(i, v),
            _ => {}
        }
    }

    /// `hasOutputSignal()`.
    fn active(&self) -> bool {
        match self.mode {
            0 => self.sub.active(),
            1 => self.tronic.active(),
            2 => self.dusty.active(),
            3 => self.grit.active(),
            4..=8 => self.bd[self.mode - 4].active(),
            _ => false,
        }
    }

    fn control_rate(&mut self) {
        match self.mode {
            0 => self.sub.control(),
            1 => self.tronic.control(),
            2 => self.dusty.control(),
            3 => self.grit.control(),
            4..=8 => self.bd[self.mode - 4].control(),
            _ => {}
        }
    }

    /// `processAudio` for one chunk; false when it wrote nothing.
    fn render(&mut self, l: &mut [f32], r: &mut [f32]) -> bool {
        match self.mode {
            0 => self.sub.process(l, r),
            1 => self.tronic.process(l, r),
            2 => self.dusty.process(l, r),
            3 => self.grit.process(l, r),
            4..=8 => self.bd[self.mode - 4].process(l, r),
            _ => false,
        }
    }
}

impl Synth for Kick {
    fn set(&mut self, params: &[(String, f64)]) {
        for (k, f) in FIELDS.iter().enumerate() {
            let v = param(params, f.name, f.default as f64) as f32;
            if self.applied[k] == Some(v) {
                continue;
            }
            self.applied[k] = Some(v);
            match f.to {
                To::Shared(i) => self.set_shared(i, v),
                To::Core(e, i) => self.set_core(e, i, v),
            }
        }
    }

    fn note_on(&mut self, note: i64, velocity: i64) {
        if self.n_events < MAX_EVENTS {
            let v = velocity.clamp(0, 127) as f32 * VEL_SCALE;
            self.events[self.n_events] = Event::On(note.clamp(0, 127) as u32, v);
            self.n_events += 1;
        }
    }

    fn note_off(&mut self, _note: i64) {
        if self.n_events < MAX_EVENTS {
            self.events[self.n_events] = Event::Off;
            self.n_events += 1;
        }
    }

    fn control(&mut self, kind: MidiKind) {
        // Maschine does not pass All Sound Off (CC 120) from the host to the drum synth:
        // a render with it equals one without (measured), so controllers do nothing.
        let _ = kind;
    }

    fn stop(&mut self) {
        self.release(self.mode);
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.take_events();
        self.render_blocks(l, r);
    }

    fn voices(&self) -> usize {
        let queued = self.events[..self.n_events].iter().any(|e| matches!(e, Event::On(..)));
        usize::from(self.sounding || self.active() || queued)
    }
}

impl Kick {
    /// The queued events, in order: only the first note-on plays (`DrumSynthWrapper`
    /// takes one note per time stamp); note-offs reach Grit only (its Gate mode).
    fn take_events(&mut self) {
        let mut started = false;
        for i in 0..self.n_events {
            match self.events[i] {
                Event::On(note, v) => {
                    if !started {
                        started = true;
                        self.start_note(note, v);
                    }
                }
                Event::Off => {
                    if self.mode == 3 {
                        self.grit.note_off();
                    }
                }
            }
        }
        self.n_events = 0;
    }

    /// `KickProcessor::setNoteOn(velocity, note)` (0x1019f5af4).
    fn start_note(&mut self, note: u32, v: f32) {
        self.mode = self.pending;
        self.update_global();
        if self.mode >= 4 {
            // The modal cores take the raw velocity (their tables shape it).
            self.bd[self.mode - 4].note_on(v, note);
            return;
        }
        // The Velocity parameter bends the curve: v³ at +1, flat at 0, (1−v)³ at −1.
        let s = self.shared[6];
        let x = if s < 0.0 { v } else { 1.0 - v };
        let g = (-x).mul_add(s.abs(), 1.0);
        let g = g * (g * g);
        match self.mode {
            0 => self.sub.note_on(g, note),
            1 => self.tronic.note_on(g, note),
            2 => self.dusty.note_on(g, note),
            3 => self.grit.note_on(g, note),
            _ => {}
        }
    }

    /// Run the wrapper's 128-sample control blocks and 16-sample chunks.
    fn render_blocks(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n = l.len();
        let mut at = 0;
        // Where the current 128-sample block began in this call. Maschine runs the
        // wrapper on 128-sample buffers: once a chunk ends silent, the wrapper marks
        // the buffer silent, which clears all of it.
        let mut block_start = 0;
        while at < n {
            if self.ctrl_left == 0 {
                self.control_rate();
                self.ctrl_left = CONTROL;
                block_start = at;
            }
            if self.chunk_left == 0 {
                self.chunk_left = CHUNK;
            }
            let k = (self.ctrl_left.min(self.chunk_left) as usize).min(n - at);
            if !self.render(&mut l[at..at + k], &mut r[at..at + k]) {
                l[at..at + k].fill(0.0);
                r[at..at + k].fill(0.0);
            }
            if !self.active() {
                l[block_start..at + k].fill(0.0);
                r[block_start..at + k].fill(0.0);
            }
            self.ctrl_left -= k as u32;
            self.chunk_left -= k as u32;
            at += k;
        }
        self.sounding = self.active();
    }
}

/// What a new Kick holds: Maschine's defaults (stored values, by field).
pub fn defaults() -> Vec<(&'static str, f64)> {
    FIELDS.iter().map(|f| (f.name, f.default as f64)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 48_000.0;

    fn params(engine: usize, extra: &[(&str, f64)]) -> Vec<(String, f64)> {
        let mut p = vec![("engine".to_string(), engine as f64)];
        p.extend(extra.iter().map(|(k, v)| (k.to_string(), *v)));
        p
    }

    /// Render `n` frames in host buffers of 512.
    fn render(k: &mut Kick, n: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n);
        let (mut l, mut r) = (vec![0f32; 512], vec![0f32; 512]);
        while out.len() < n {
            let m = (n - out.len()).min(512);
            k.process(&mut l[..m], &mut r[..m]);
            assert_eq!(l[..m], r[..m], "the Kick is mono");
            out.extend_from_slice(&l[..m]);
        }
        out
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0, |a, v| a.max(v.abs()))
    }

    /// The engines DECKARD plays (Rasper…Push are not ported yet: see the doc).
    const PORTED: [usize; 4] = [0, 1, 2, 3];

    #[test]
    fn defaults_sound() {
        let mut k = Kick::new(SR);
        k.set(&[]);
        k.note_on(36, 100);
        let y = render(&mut k, 24_000);
        assert!(peak(&y) > 0.1, "peak {}", peak(&y));
        assert!(y.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn every_ported_engine_renders() {
        for e in PORTED {
            let mut k = Kick::new(SR);
            k.set(&params(e, &[]));
            k.note_on(48, 127);
            let y = render(&mut k, 24_000);
            assert!(peak(&y) > 0.05, "{}: peak {}", ENGINES[e], peak(&y));
        }
    }

    /// DECKARD's own modal engines (Rasper…Push) sound at a sane level and end.
    #[test]
    fn modal_engines_sound_and_end() {
        for (e, name) in ENGINES.iter().enumerate().skip(4) {
            let mut k = Kick::new(SR);
            k.set(&params(e, &[]));
            k.note_on(48, 127);
            let y = render(&mut k, 4096);
            let p = peak(&y);
            assert!(p > 0.05 && p < 2.0 && y.iter().all(|v| v.is_finite()), "{name}: peak {p}");
            let _ = render(&mut k, 48_000 * 4);
            assert_eq!(k.voices(), 0, "{name} ends");
        }
    }

    #[test]
    fn silence_after_release() {
        for e in PORTED {
            for gate in [0.0, 1.0] {
                let mut k = Kick::new(SR);
                k.set(&params(e, &[("decay", 1.0), ("grit_kick_gate", gate)]));
                k.note_on(36, 127);
                let _ = render(&mut k, 4800);
                k.note_off(36);
                let mut tail = Vec::new();
                for _ in 0..40 {
                    tail = render(&mut k, 24_000);
                    if k.voices() == 0 {
                        break;
                    }
                }
                assert_eq!(k.voices(), 0, "{} gate {gate} still sounds", ENGINES[e]);
                assert_eq!(peak(&tail[tail.len() - 128..]), 0.0, "{}", ENGINES[e]);
                // Silent from then on, whatever the host does.
                assert_eq!(peak(&render(&mut k, 4096)), 0.0);
            }
        }
    }

    #[test]
    fn no_nan_or_runaway_at_extremes() {
        let corners: [&[(&str, f64)]; 3] = [
            &[("tune", 0.0), ("decay", 0.0), ("bend", 0.0), ("impact", 0.0), ("punch", 0.0), ("velocity", -1.0)],
            &[
                ("tune", 1.0),
                ("decay", 1.0),
                ("bend", 1.0),
                ("impact", 1.0),
                ("punch", 1.0),
                ("velocity", 1.0),
                ("sub_kick_thin_attack", 2.0),
                ("sub_kick_attack_amount", 1.0),
                ("sub_kick_color", 1.0),
                ("clean_bd_drive_gain", 1.0),
                ("clean_bd_drive_tone", 1.0),
                ("clean_bd_bias", 1.0),
                ("clean_bd_drive_dw", 1.0),
                ("dusty_kick_filter", 1.0),
                ("dusty_kick_noise", 1.0),
                ("grit_kick_aero_grind", 1.0),
                ("grit_kick_aero_amount", 1.0),
            ],
            &[("sub_kick_thin_attack", 1.0), ("grit_kick_gate", 1.0), ("grit_kick_aero_grind", 0.0)],
        ];
        // Grit at the loud corner peaks near 21 in Maschine too (measured): the bound only
        // catches runaways.
        for e in PORTED {
            for c in corners {
                let mut k = Kick::new(SR);
                k.set(&params(e, c));
                // Fast retriggers across the range, then a long tail.
                for (i, n) in [0, 127, 36, 90, 12].into_iter().enumerate() {
                    k.note_on(n, 1 + 31 * i as i64);
                    let y = render(&mut k, 700);
                    assert!(y.iter().all(|v| v.is_finite() && v.abs() < 64.0), "{} {c:?}: {}", ENGINES[e], peak(&y));
                }
                let y = render(&mut k, 10 * 48_000);
                let bad = y.iter().position(|v| !(v.is_finite() && v.abs() < 64.0));
                assert!(bad.is_none(), "{} {c:?}: at {bad:?} {:?} peak {}", ENGINES[e], bad.map(|i| y[i]), peak(&y));
                // No denormal residue once the voice is over.
                if k.voices() == 0 {
                    assert!(y[y.len() - 4096..].iter().all(|v| *v == 0.0), "{} {c:?}", ENGINES[e]);
                }
            }
        }
    }

    #[test]
    fn set_mid_note_does_not_click_or_restart() {
        // (engine, a parameter the change moves)
        let moves: [(usize, &str, f64); 8] = [
            (0, "tune", 0.9),
            (0, "sub_kick_color", 1.0),
            (1, "decay", 0.1),
            (1, "clean_bd_drive_gain", 0.0),
            (2, "tune", 0.1),
            (2, "dusty_kick_filter", 1.0),
            (3, "tune", 1.0),
            (3, "grit_kick_aero_amount", 1.0),
        ];
        let at = 2400;
        for (e, field, v) in moves {
            let base = params(e, &[("decay", 0.8)]);
            let mut a = Kick::new(SR);
            a.set(&base);
            a.note_on(40, 110);
            let ya = render(&mut a, 9600);
            let mut b = Kick::new(SR);
            b.set(&base);
            b.note_on(40, 110);
            let mut yb = render(&mut b, at);
            let mut moved = base.clone();
            moved.push((field.to_string(), v));
            b.set(&moved);
            yb.extend(render(&mut b, 9600 - at));
            assert_eq!(ya[..at], yb[..at]);
            // The change ramps in: the first samples after it stay close to the
            // unchanged note (no step), and the note keeps sounding (no restart).
            let step = (at..at + 8).map(|i| (ya[i] - yb[i]).abs()).fold(0.0, f32::max);
            assert!(step < 0.05, "{} {field}: step {step}", ENGINES[e]);
            assert!(peak(&yb[at..at + 2400]) > 0.01, "{} {field}", ENGINES[e]);
            let restart = peak(&yb[at..at + 480]) > 1.5 * peak(&ya[at - 480..at]) + 0.05;
            assert!(!restart, "{} {field}: restarted", ENGINES[e]);
        }
    }

    #[test]
    fn set_is_idempotent() {
        // Re-sending the same values changes nothing (the host re-sends on any edit).
        let p = params(3, &[("tune", 0.3)]);
        let mut a = Kick::new(SR);
        a.set(&p);
        a.note_on(50, 90);
        let ya = render(&mut a, 4800);
        let mut b = Kick::new(SR);
        b.set(&p);
        b.note_on(50, 90);
        let mut yb = render(&mut b, 1000);
        b.set(&p);
        yb.extend(render(&mut b, 3800));
        assert_eq!(ya, yb);
    }

    #[test]
    fn stop_fades_out_and_cc120_does_not() {
        for e in PORTED {
            let mut k = Kick::new(SR);
            k.set(&params(e, &[("decay", 1.0)]));
            k.note_on(36, 127);
            let a = render(&mut k, 2400);
            k.stop();
            let _ = render(&mut k, 48_000);
            let y = render(&mut k, 4800);
            assert!(peak(&y) < 1e-3, "{}: {}", ENGINES[e], peak(&y));
            // CC 120 from the host is not passed on (as measured in Maschine).
            let mut k2 = Kick::new(SR);
            k2.set(&params(e, &[("decay", 1.0)]));
            k2.note_on(36, 127);
            let b = render(&mut k2, 2400);
            k2.control(MidiKind::Controller { number: 120, value: 0 });
            assert_eq!(a, b);
            let mut k3 = Kick::new(SR);
            k3.set(&params(e, &[("decay", 1.0)]));
            k3.note_on(36, 127);
            let _ = render(&mut k3, 2400);
            assert_eq!(render(&mut k2, 4800), render(&mut k3, 4800));
        }
    }

    #[test]
    fn first_note_on_of_a_block_plays() {
        let mut a = Kick::new(SR);
        a.set(&params(0, &[]));
        a.note_on(30, 80);
        let ya = render(&mut a, 4800);
        let mut b = Kick::new(SR);
        b.set(&params(0, &[]));
        b.note_on(30, 80);
        b.note_on(90, 127);
        assert_eq!(ya, render(&mut b, 4800));
    }
}
