//! Maschine's internal effects (docs/fx-*.md), as units of a sound's effect chain.

pub mod beat_delay;
pub mod cabinet;
pub mod chorus;
pub mod compressor;
pub mod distortion;
pub mod eq;
pub mod filter;
pub mod flanger;
pub mod fm;
pub mod freq_shifter;
pub mod gate;
pub mod grain_delay;
pub mod grain_stretch;
pub mod iceverb;
pub mod limiter;
pub mod lofi;
pub mod maximizer;
pub mod metaverb;
pub mod perform;
pub mod phaser;
pub mod plate_reverb;
pub mod reflex;
pub mod resochord;
pub mod reverb;
pub mod saturator;
pub mod sidechain;
pub mod transient_master;

/// An internal effect, processing a block in place.
pub trait Effect: Send {
    /// Take the module's parameters (stored values, by field name). Called when the
    /// effect is made and again when they change, keeping its state (no clicks).
    fn set(&mut self, params: &[(String, f64)]);
    /// Process `l`/`r` (the same length) in place.
    fn process(&mut self, l: &mut [f32], r: &mut [f32]);

    /// [`Effect::process`] with a side-chain input (left, right; the same length), when
    /// the effect's Side-Chain Input has a source. Effects without one ignore it.
    fn process_sc(&mut self, l: &mut [f32], r: &mut [f32], sc: Option<(&[f32], &[f32])>) {
        let _ = sc;
        self.process(l, r)
    }

    /// The song tempo (BPM), for tempo-synced effects. Called when made and when it
    /// changes.
    fn set_tempo(&mut self, bpm: f64) {
        let _ = bpm;
    }

    /// The song position (quarters) for the next block while the transport plays,
    /// `None` when stopped: synced modulation follows it. As in Maschine, the position
    /// is the host block's start and holds until the next call: every one of the
    /// effect's own 128-sample blocks within the host block sees the same position
    /// ([`song_position`]).
    fn set_transport(&mut self, quarters: Option<f64>) {
        let _ = quarters;
    }

    /// Whether its panel shows meters (input, gain reduction, output): the chain then
    /// measures them around it.
    fn metered(&self) -> bool {
        false
    }

    /// How much it is reducing the gain now (linear, 1 = none), for its GR meter.
    fn gain_reduction(&self) -> f32 {
        1.0
    }
}

/// The song position an effect's processor sees (`ProcessInfo` +0x30) for the host's
/// position `quarters`: Maschine's clock counts 240000 ticks per quarter, so the host
/// position arrives rounded to the nearest tick (docs/fx-filter.md §4).
pub(crate) fn song_position(quarters: f64) -> f64 {
    (quarters * 240_000.0).round() / 240_000.0
}

/// `NI::AB::ExpTable` (docs/fx-eq.md §2.1): `(val, slope)`, `val[i] = 2^(i/60 − 20)`.
fn exp_table() -> &'static [(f32, f32); 2401] {
    static T: std::sync::OnceLock<Box<[(f32, f32); 2401]>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = Box::new([(0f32, 0f32); 2401]);
        t[0].0 = f32::from_bits(0x3580_0000);
        for (i, e) in t.iter_mut().enumerate().skip(1) {
            e.0 = (i as f64 / 60.0 + -20.0).exp2() as f32;
        }
        for i in 0..2400 {
            t[i].1 = t[i + 1].0 - t[i].0;
        }
        t
    })
}

/// The exp-table lookup `E(x) ≈ 2^((x − 1200)/60)`, shared by Maschine's effects.
pub(crate) fn exp_e(x: f32) -> f32 {
    let t = exp_table();
    let n = x as i32; // fcvtzs
    if n >= 2400 {
        t[2400].0
    } else if n < 0 {
        0.0
    } else {
        let (v, s) = t[n as usize];
        (x - n as f32).mul_add(s, v)
    }
}

/// A parameter by field name, or `default`.
pub fn param(params: &[(String, f64)], field: &str, default: f64) -> f64 {
    params.iter().find(|(k, _)| k == field).map_or(default, |(_, v)| *v)
}

/// The effect for a module class ("EQ", "Compressor", …) at sample rate `sr`, if it
/// is one DECKARD renders.
pub fn make(module: &str, params: &[(String, f64)], sr: f64) -> Option<Box<dyn Effect>> {
    let mut fx: Box<dyn Effect> = match module {
        "Limiter" => Box::new(limiter::Limiter::new(sr)),
        "EQ" => Box::new(eq::Eq::new(sr)),
        "Compressor" => Box::new(compressor::Compressor::new(sr)),
        "BeatDelay" => Box::new(beat_delay::BeatDelay::new(sr)),
        "Chorus" => Box::new(chorus::Chorus::new(sr)),
        "Filter" => Box::new(filter::Filter::new(sr)),
        "Maximizer" => Box::new(maximizer::Maximizer::new(sr)),
        "Saturator" => Box::new(saturator::Saturator::new(sr)),
        "Reverb" => Box::new(reverb::Reverb::new(sr)),
        "PlateReverb" => Box::new(plate_reverb::PlateReverb::new(sr)),
        "Metaverb" => Box::new(metaverb::Metaverb::new(sr)),
        "Iceverb" => Box::new(iceverb::Iceverb::new(sr)),
        "Reflex" => Box::new(reflex::Reflex::new(sr)),
        "Gate" => Box::new(gate::Gate::new(sr)),
        "TransientDesigner" => Box::new(transient_master::TransientMaster::new(sr)),
        "Flanger" => Box::new(flanger::Flanger::new(sr)),
        "Phaser" => Box::new(phaser::Phaser::new(sr)),
        "FreqShifter" => Box::new(freq_shifter::FreqShifter::new(sr)),
        "FM" => Box::new(fm::Fm::new(sr)),
        "LoFi" => Box::new(lofi::LoFi::new(sr)),
        // Cabinet needs NI's impulse responses (from a Maschine install): left off so
        // DECKARD runs standalone. A project's Cabinet passes its input through.
        "MullhollandDrive" => Box::new(distortion::Distortion::new(sr)),
        "GrainDelay" => Box::new(grain_delay::GrainDelay::new(sr)),
        "GrainStretch" => Box::new(grain_stretch::GrainStretch::new(sr)),
        "Resochord" => Box::new(resochord::Resochord::new(sr)),
        "PerformanceFX" => Box::new(perform::PerformFx::new(sr)),
        _ => return None,
    };
    fx.set(params);
    Some(fx)
}
