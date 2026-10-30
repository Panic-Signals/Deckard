//! Mutable Instruments' module DSP, built from Émilie Gillet's open source C++ (MIT; see
//! `vendor/LICENSE`): the code itself, so DECKARD's Plaits sounds as the module does.
//!
//! - [`Plaits`]: one voice of the macro-oscillator, all 24 engines, with its low-pass
//!   gate. It runs at the module's 48 kHz in blocks of up to [`BLOCK`] frames.

use std::ffi::c_void;

/// Mutable Instruments' copyright and licence (MIT), for the About box: it goes with
/// every copy of DECKARD.
pub const LICENSE: &str = include_str!("../vendor/LICENSE");

/// The most frames a [`Plaits::render`] takes.
pub const BLOCK: usize = 24;

/// The rate Plaits runs at.
pub const SAMPLE_RATE: f64 = 48_000.0;

/// What the module's knobs set (`plaits::Patch`): `note` in semitones (MIDI numbers),
/// the rest 0..1 but `engine` (0..23) and the attenuverters (-1..1).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Patch {
    pub note: f32,
    pub harmonics: f32,
    pub timbre: f32,
    pub morph: f32,
    pub frequency_modulation_amount: f32,
    pub timbre_modulation_amount: f32,
    pub morph_modulation_amount: f32,
    pub engine: i32,
    pub decay: f32,
    pub lpg_colour: f32,
}

/// What its jacks carry (`plaits::Modulations`): `trigger` and `level` (0..1) with
/// whether they're patched; the others as the module's CVs.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Modulations {
    pub engine: f32,
    pub note: f32,
    pub frequency: f32,
    pub harmonics: f32,
    pub timbre: f32,
    pub morph: f32,
    pub trigger: f32,
    pub level: f32,
    pub frequency_patched: i32,
    pub timbre_patched: i32,
    pub morph_patched: i32,
    pub trigger_patched: i32,
    pub level_patched: i32,
}

unsafe extern "C" {
    fn dk_plaits_new() -> *mut c_void;
    fn dk_plaits_free(p: *mut c_void);
    fn dk_plaits_reset(p: *mut c_void);
    fn dk_plaits_render(p: *mut c_void, patch: *const Patch, mods: *const Modulations, out: *mut f32, aux: *mut f32, size: usize);
}

/// One Plaits voice.
pub struct Plaits(*mut c_void);

// The voice is plain memory, used from one thread at a time.
unsafe impl Send for Plaits {}

impl Plaits {
    pub fn new() -> Plaits {
        let p = unsafe { dk_plaits_new() };
        assert!(!p.is_null(), "out of memory for a Plaits voice");
        Plaits(p)
    }

    /// Back to as made: silent, every engine's state cleared. Doesn't allocate.
    pub fn reset(&mut self) {
        unsafe { dk_plaits_reset(self.0) }
    }

    /// Render `out.len()` frames (at most [`BLOCK`]; `aux` as long) of the main and aux
    /// outputs, -1..1.
    pub fn render(&mut self, patch: &Patch, mods: &Modulations, out: &mut [f32], aux: &mut [f32]) {
        let n = out.len().min(aux.len()).min(BLOCK);
        unsafe { dk_plaits_render(self.0, patch, mods, out.as_mut_ptr(), aux.as_mut_ptr(), n) }
    }
}

impl Default for Plaits {
    fn default() -> Plaits {
        Plaits::new()
    }
}

impl Drop for Plaits {
    fn drop(&mut self) {
        unsafe { dk_plaits_free(self.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every engine makes sound with its trigger held (a gate, as the TRIG jack takes),
    /// and none goes past full scale.
    #[test]
    fn every_engine_sounds() {
        for engine in 0..24 {
            let mut v = Plaits::new();
            let patch =
                Patch { note: 48.0, harmonics: 0.5, timbre: 0.5, morph: 0.5, engine, decay: 0.5, lpg_colour: 0.5, ..Default::default() };
            let mut peak = 0f32;
            for block in 0..400 {
                let mods = Modulations {
                    trigger: if (10..200).contains(&block) { 1.0 } else { 0.0 },
                    trigger_patched: 1,
                    level: 1.0,
                    ..Default::default()
                };
                let (mut out, mut aux) = ([0f32; BLOCK], [0f32; BLOCK]);
                v.render(&patch, &mods, &mut out, &mut aux);
                peak = out.iter().chain(&aux).fold(peak, |m, x| m.max(x.abs()));
            }
            assert!(peak > 0.01 && peak <= 1.0, "engine {engine}: peak {peak}");
        }
    }
}
