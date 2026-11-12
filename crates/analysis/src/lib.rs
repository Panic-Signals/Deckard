//! What a sample is: its tempo and its root note, for the sampler's Sync and Keys modes
//! (docs/superpowers/plans/2026-11-11-sampler-roadmap.md, Phase 1a).
//!
//! Pure functions over a sample's frames: no files, no threads. They take a while on long
//! samples, so callers run them off the audio thread and keep the answers.

mod onset;
pub mod root;
pub mod tempo;
#[cfg(test)]
mod test_signals;

// ROOT_EXPORTS
pub use tempo::{Tempo, TempoSource, tempo};

/// The frames mixed to mono.
pub(crate) fn mono(frames: &[[f32; 2]]) -> Vec<f32> {
    frames.iter().map(|f| 0.5 * (f[0] + f[1])).collect()
}
