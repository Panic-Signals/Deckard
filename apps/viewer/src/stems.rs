//! EDIT ▸ STEMS (Maschine 3 manual, "Audio Editing Functions"): the focused zone's sample
//! separated into drums, bass, other and vocals, in the background, one at a time. The
//! work is `deckard_stems` (Demucs v4); this is the job the app keeps: where it came
//! from, its progress, cancelling it, and its result.
//!
//! The model is installed with DECKARD (`scripts/install-demucs.sh`, `.ps1` on
//! Windows). If it isn't there yet the job runs that installer first — the copy shipped
//! beside the program (Windows, Linux), in the app's Resources (macOS), or the
//! repository's when run from it.

use deckard_engine::sample::Sample;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};

/// Separating, or still installing the model first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Installing,
    Separating,
}

pub struct Job {
    /// The zone whose sample it is (group, sound, zone), and the sample's name.
    pub source: (usize, usize, usize),
    pub name: String,
    pub rate: u32,
    /// The source's length in beats, when it's an Audio plug-in's take (the stems keep
    /// it).
    pub beats: Option<f64>,
    progress: Arc<AtomicU32>,
    installing: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    done: mpsc::Receiver<Result<Vec<deckard_stems::Stem>, String>>,
}

impl Job {
    /// Start separating `sample`, with the model in `settings`' `models`.
    pub fn start(sample: Arc<Sample>, name: String, source: (usize, usize, usize), settings: PathBuf) -> Job {
        let progress = Arc::new(AtomicU32::new(0));
        let installing = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, done) = mpsc::channel();
        let rate = sample.rate;
        let (p, inst, c) = (progress.clone(), installing.clone(), cancel.clone());
        std::thread::Builder::new()
            .name("stems".into())
            .spawn(move || {
                let _ = tx.send(run(&sample, &settings, &p, &inst, &c));
            })
            .expect("a thread for the stems");
        Job { source, name, rate, beats: None, progress, installing, cancel, done }
    }

    /// How far it is, 0..1.
    pub fn progress(&self) -> f32 {
        f32::from_bits(self.progress.load(Ordering::Relaxed))
    }

    pub fn phase(&self) -> Phase {
        if self.installing.load(Ordering::Relaxed) { Phase::Installing } else { Phase::Separating }
    }

    /// Stop it (between chunks of the work; the result is then an error).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The stems, once it's done.
    pub fn poll(&self) -> Option<Result<Vec<deckard_stems::Stem>, String>> {
        match self.done.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("the stem separation stopped".into())),
        }
    }
}

fn run(
    sample: &Sample,
    settings: &Path,
    progress: &AtomicU32,
    installing: &AtomicBool,
    cancel: &AtomicBool,
) -> Result<Vec<deckard_stems::Stem>, String> {
    let models = settings.join("models");
    let model = match deckard_stems::load(&models) {
        Ok(m) => m,
        Err(_) => {
            installing.store(true, Ordering::Relaxed);
            install(settings)?;
            installing.store(false, Ordering::Relaxed);
            deckard_stems::load(&models)?
        }
    };
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    let report = |p: f32| progress.store(p.to_bits(), Ordering::Relaxed);
    // The GPU, else (it isn't there, or fails) the CPU.
    let gpu = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        deckard_stems::separate(&model, &sample.frames, sample.rate, deckard_stems::Device::Gpu, report, cancel)
    }));
    match gpu {
        Ok(Ok(stems)) => Ok(stems),
        Ok(Err(e)) if cancel.load(Ordering::Relaxed) => Err(e),
        _ => deckard_stems::separate(&model, &sample.frames, sample.rate, deckard_stems::Device::Cpu, report, cancel),
    }
}

/// Run the model's installer (into `settings`, as the app reads it).
/// The model is installed (in `settings`' `models`).
pub fn installed(settings: &Path) -> bool {
    settings.join("models").join(deckard_stems::MODEL_FILE).is_file()
}

/// The model's size, as the offer to install it says.
pub const MODEL_MB: u32 = 84;

/// Install the model into `settings`' `models` (a download; takes a while).
pub fn install(settings: &Path) -> Result<(), String> {
    let script = installer().ok_or("the stem separation model isn't installed, and its installer isn't with DECKARD")?;
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("powershell");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&script);
        c
    } else {
        let mut c = std::process::Command::new("bash");
        c.arg(&script);
        c
    };
    let out = cmd
        .env("DECKARD_SETTINGS", settings)
        .env("DECKARD_INSTALL_DEMUCS", "1")
        .output()
        .map_err(|e| format!("{}: {e}", script.display()))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("installing the stem separation model failed: {}", err.lines().last().unwrap_or("")))
    }
}

/// The installer: beside the program, in the app bundle's Resources, in the package's
/// share folder (/usr/share/deckard, from /usr/bin), or the repository's (a build run
/// from it).
fn installer() -> Option<PathBuf> {
    let name = if cfg!(windows) { "install-demucs.ps1" } else { "install-demucs.sh" };
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [
        dir.join(name),
        dir.join("../Resources").join(name),
        dir.join("../share/deckard").join(name),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts").join(name),
    ]
    .into_iter()
    .find(|p| p.is_file())
}
