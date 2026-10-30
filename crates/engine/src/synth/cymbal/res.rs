//! The two noise recordings the Cymbal plays (docs/synth-cymbal.md §6). DECKARD has no
//! copy of them: both are recorded/authored audio compiled into Maschine as `WAVE`
//! resources, not data any Maschine code generates, so they can be neither shipped nor
//! regenerated. [`noises`] and [`pink`] are DECKARD's own stand-ins instead
//! (`synth::noise`): noise shaped like each recording.
//!
//! - WAVE 3, `ModalNoises_23x131072.wav` (`NoiseSampler::m_aSampleAudioBlock`, read by
//!   `AttachedBodiesCore`): 23 noises of 131072 samples one after the other (3014656
//!   samples, 6029430-byte RIFF; its LIST/INFO says "2011-11-07", "Sony Sound Forge 9.0").
//! - WAVE 4, `ModalPinkNoise_1x131072.wav` (`PinkSampler::s_aSampleAudioBlock`, read by
//!   `UTIL::Air` in the body): 131072 samples of pink noise (262188-byte RIFF).
//!
//! Both are mono 16-bit PCM at 44.1 kHz; Maschine decodes a sample `n` to
//! `max(n·f32(1/32767), −1)` (`loadSampleFromResources`, `SampleFormatConverter`).
//!
//! The oracle tests may feed decoded copies (raw little-endian `f32` files written by the
//! scratch harness from a local Maschine install) through `CYMBAL_RES_NOISES` /
//! `CYMBAL_RES_PINK`; nothing outside `cfg(test)` reads them.

/// Samples per noise in [`noises`] (and the length of [`pink`]).
pub(super) const NOISE_LEN: usize = 131_072;
/// The rate the recordings were made at (`s_fWAVFileSampleRate`).
pub(super) const WAV_RATE: f32 = 44_100.0;

/// Resource WAVE 3: 23 noises × [`NOISE_LEN`] samples (DECKARD's stand-in).
pub(super) fn noises() -> Option<&'static [f32]> {
    copy::get(0)
}

/// Resource WAVE 4: [`NOISE_LEN`] samples of pink noise (DECKARD's stand-in).
pub(super) fn pink() -> Option<&'static [f32]> {
    copy::get(1)
}

#[cfg(not(test))]
mod copy {
    pub(super) fn get(i: usize) -> Option<&'static [f32]> {
        Some(super::stand_in(i))
    }
}

/// DECKARD's own source `i` (0 the noises, 1 pink).
fn stand_in(i: usize) -> &'static [f32] {
    if i == 0 { crate::synth::noise::modal_noises() } else { crate::synth::noise::pink() }
}

#[cfg(test)]
mod copy {
    use std::sync::OnceLock;

    pub(super) fn get(i: usize) -> Option<&'static [f32]> {
        static R: [OnceLock<Option<Box<[f32]>>>; 2] = [OnceLock::new(), OnceLock::new()];
        let var = ["CYMBAL_RES_NOISES", "CYMBAL_RES_PINK"][i];
        R[i].get_or_init(|| {
            let b = std::fs::read(std::env::var_os(var)?).ok()?;
            Some(b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect())
        })
        .as_deref()
        .or_else(|| Some(super::stand_in(i)))
    }
}
