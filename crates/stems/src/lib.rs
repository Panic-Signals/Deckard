//! Stem separation, as Maschine's EDIT ▸ STEMS: a stereo sample into four stems — drums
//! (and percussion), bass, other and vocals — with Meta's Demucs v4 model (`htdemucs`)
//! run by demucs-rs (`demucs-core`), on the GPU when there is one.
//!
//! The model's weights (`htdemucs.safetensors`, 84 MB) are installed with DECKARD
//! (`scripts/install-demucs.sh`) into its settings folder's `models`; [`load`] checks
//! them against [`MODEL_SHA256`].

use burn::prelude::Backend;
use demucs_core::listener::{ForwardEvent, ForwardListener};
use demucs_core::{Demucs, ModelOptions};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// The model's file, as Hugging Face's `set-soft/audio_separation` serves it, and its
/// SHA-256 (as published there).
pub const MODEL_FILE: &str = "htdemucs.safetensors";
pub const MODEL_URL: &str = "https://huggingface.co/set-soft/audio_separation/resolve/main/Demucs/htdemucs.safetensors";
pub const MODEL_SHA256: &str = "8193504cdfb3943adaf039b8acb524a46e87ebf232c383ac7a32c80a6578423e";

/// The stems, in Maschine's order (its new group's first four sounds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Drums,
    Bass,
    Other,
    Vocals,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Drums, Kind::Bass, Kind::Other, Kind::Vocals];

    /// As Maschine names its stems: the sample's name followed by this.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Drums => "Drums",
            Kind::Bass => "Bass",
            Kind::Other => "Other",
            Kind::Vocals => "Vocal",
        }
    }
}

/// One stem: stereo frames at the sample's rate.
pub struct Stem {
    pub kind: Kind,
    pub frames: Vec<[f32; 2]>,
}

/// The installed model's bytes, if they're there and intact.
pub fn load(dir: &Path) -> Result<Vec<u8>, String> {
    let path = dir.join(MODEL_FILE);
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("the stem separation model isn't installed ({}: {e}); run scripts/install-demucs.sh", path.display()))?;
    let sum: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if sum != MODEL_SHA256 {
        return Err(format!("{} isn't the expected model; run scripts/install-demucs.sh again", path.display()));
    }
    Ok(bytes)
}

/// Where it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Gpu,
    Cpu,
}

/// Progress (0..1, as each chunk finishes) and cancelling (checked between chunks).
struct Listener<'a, F: FnMut(f32)> {
    progress: F,
    cancel: &'a AtomicBool,
}

impl<F: FnMut(f32)> ForwardListener for Listener<'_, F> {
    fn on_event(&mut self, event: ForwardEvent) {
        if let ForwardEvent::ChunkDone { index, total } = event {
            (self.progress)((index + 1) as f32 / total.max(1) as f32);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Separate stereo `frames` at `rate` into the four stems (at the same rate and length).
/// `progress` hears how far it is; setting `cancel` stops it between chunks (an error).
pub fn separate(
    model: &[u8],
    frames: &[[f32; 2]],
    rate: u32,
    device: Device,
    progress: impl FnMut(f32),
    cancel: &AtomicBool,
) -> Result<Vec<Stem>, String> {
    match device {
        Device::Gpu => run::<burn::backend::Wgpu>(model, frames, rate, Default::default(), progress, cancel),
        Device::Cpu => run::<burn::backend::NdArray<f32>>(model, frames, rate, Default::default(), progress, cancel),
    }
}

fn run<B: Backend>(
    model: &[u8],
    frames: &[[f32; 2]],
    rate: u32,
    device: B::Device,
    progress: impl FnMut(f32),
    cancel: &AtomicBool,
) -> Result<Vec<Stem>, String> {
    let demucs = Demucs::<B>::from_bytes(ModelOptions::FourStem, model, device).map_err(|e| e.to_string())?;
    let left: Vec<f32> = frames.iter().map(|f| f[0]).collect();
    let right: Vec<f32> = frames.iter().map(|f| f[1]).collect();
    let mut listener = Listener { progress, cancel };
    let stems = pollster::block_on(demucs.separate_with_listener(&left, &right, rate, &mut listener)).map_err(|e| e.to_string())?;
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    // Audio shorter than the model's segment is one piece, without chunks to report.
    (listener.progress)(1.0);
    let mut out: Vec<Stem> = stems
        .into_iter()
        .filter_map(|s| {
            let kind = match s.id {
                demucs_core::model::metadata::StemId::Drums => Kind::Drums,
                demucs_core::model::metadata::StemId::Bass => Kind::Bass,
                demucs_core::model::metadata::StemId::Other => Kind::Other,
                demucs_core::model::metadata::StemId::Vocals => Kind::Vocals,
                _ => return None,
            };
            // Back at the sample's rate, frame for frame.
            let n = frames.len();
            let at = |v: &[f32], i: usize| v.get(i).copied().unwrap_or(0.0);
            let frames = (0..n).map(|i| [at(&s.left, i), at(&s.right, i)]).collect();
            Some(Stem { kind, frames })
        })
        .collect();
    out.sort_by_key(|s| Kind::ALL.iter().position(|k| *k == s.kind));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A missing or damaged model is refused, pointing at its installer.
    #[test]
    fn only_the_published_model_loads() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert!(load(&dir).unwrap_err().contains("install-demucs"));
        std::fs::write(dir.join(MODEL_FILE), b"not the model").unwrap();
        assert!(load(&dir).unwrap_err().contains("expected model"));
    }

    /// With the model installed (`DECKARD_STEMS_MODEL`: its folder): a short mix of
    /// kick-like hits and a bass tone comes back as the four stems in Maschine's order,
    /// adding up to the mix; cancelled, it stops.
    #[test]
    fn a_mix_separates_into_four_stems() {
        let Some(dir) = std::env::var_os("DECKARD_STEMS_MODEL") else { return };
        let model = load(Path::new(&dir)).unwrap();
        let rate = 44_100;
        let frames: Vec<[f32; 2]> = (0..rate * 3)
            .map(|i| {
                let t = i as f32 / rate as f32;
                let beat = t % 0.5;
                let kick = (-beat * 30.0).exp() * (std::f32::consts::TAU * (50.0 + 100.0 * (-beat * 40.0).exp()) * beat).sin();
                let bass = 0.3 * (std::f32::consts::TAU * 55.0 * t).sin();
                [0.6 * kick + bass, 0.6 * kick + bass]
            })
            .collect();
        let mut last = 0.0;
        let stems = separate(&model, &frames, rate as u32, Device::Gpu, |p| last = p, &AtomicBool::new(false)).unwrap();
        assert_eq!(stems.iter().map(|s| s.kind).collect::<Vec<_>>(), Kind::ALL);
        assert!(stems.iter().all(|s| s.frames.len() == frames.len()));
        assert_eq!(last, 1.0, "progress to the end");
        let (mut mix, mut rest) = (0.0f64, 0.0f64);
        for (i, f) in frames.iter().enumerate() {
            let sum: f32 = stems.iter().map(|s| s.frames[i][0]).sum();
            mix += f64::from(f[0] * f[0]);
            rest += f64::from((f[0] - sum).powi(2));
        }
        assert!(10.0 * (mix / rest).log10() > 20.0, "the stems add up to the mix");
        assert!(separate(&model, &frames, rate as u32, Device::Gpu, |_| {}, &AtomicBool::new(true)).is_err(), "cancelled");
    }
}
