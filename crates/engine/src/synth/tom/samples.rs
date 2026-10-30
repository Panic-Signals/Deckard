//! The two noise recordings Floor and High play (docs/synth-tom.md §6). DECKARD has no
//! copy of them: they are recorded audio compiled into Maschine as `WAVE` resources, not
//! data any Maschine code generates, so they can be neither shipped nor regenerated.
//! Without them [`pink`] and [`modal_noises`] are DECKARD's own stand-ins
//! (`synth::noise`: noise shaped like each recording; [`original`] false).
//!
//! - WAVE 3, "ModalNoises_23x131072.wav" (`NoiseSampler`, read by the
//!   `AttachedBodiesCore` of both Floor and High, which use only noise 12): 23 noises of
//!   131072 samples one after the other (3014656 samples; 6029438-byte file, RIFF size
//!   6029430; LIST/INFO dated 2011-11-07).
//! - WAVE 4, "ModalPinkNoise_1x131072.wav" (`PinkSampler::s_aSampleAudioBlock`, read by
//!   High's `UTIL::Air`): 131072 samples of pink noise (262188-byte file).
//!
//! Both are mono 16-bit PCM at 44.1 kHz; Maschine decodes a sample `n` to
//! `max(n·f32(1/32767), −1)` (`loadSampleFromResources`, `SampleFormatConverter`).
//!
//! Oracle tests may feed decoded copies (raw little-endian `f32` files written by the
//! scratch harness from a local Maschine install) through `TOM_RES_NOISES` /
//! `TOM_RES_PINK`; nothing outside `cfg(test)` reads them.

/// Frames per noise.
#[cfg(test)]
pub(super) const FRAMES: usize = 131072;
/// Noises in "ModalNoises".
#[cfg(test)]
pub(super) const NOISES: usize = 23;
/// The recordings' sample rate (`PinkSampler::s_fWAVFileSampleRate`).
pub(super) const RATE: f32 = 44100.0;

/// "ModalPinkNoise_1x131072.wav": 131072 frames at 44.1 kHz (or DECKARD's stand-in).
pub(super) fn pink() -> &'static [f32] {
    copy::get(1).unwrap_or_else(crate::synth::noise::pink)
}

/// "ModalNoises_23x131072.wav": 23 noises of 131072 frames (or DECKARD's stand-in).
#[cfg(test)]
pub(super) fn modal_noises() -> &'static [f32] {
    copy::get(0).unwrap_or_else(crate::synth::noise::modal_noises)
}

/// Whether the recordings are present (only in oracle tests).
#[allow(dead_code)]
pub(super) fn original() -> bool {
    copy::get(0).is_some() && copy::get(1).is_some()
}

#[cfg(not(test))]
mod copy {
    pub(super) fn get(_: usize) -> Option<&'static [f32]> {
        None
    }
}

#[cfg(test)]
mod copy {
    use std::sync::OnceLock;

    pub(super) fn get(i: usize) -> Option<&'static [f32]> {
        static R: [OnceLock<Option<Box<[f32]>>>; 2] = [OnceLock::new(), OnceLock::new()];
        let var = ["TOM_RES_NOISES", "TOM_RES_PINK"][i];
        R[i].get_or_init(|| {
            let b = std::fs::read(std::env::var_os(var)?).ok()?;
            Some(b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect())
        })
        .as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_have_the_right_size() {
        assert_eq!(pink().len(), FRAMES);
        assert_eq!(modal_noises().len(), FRAMES * NOISES);
        assert!(pink().iter().chain(modal_noises()).all(|x| x.is_finite() && x.abs() <= 1.0));
    }
}
