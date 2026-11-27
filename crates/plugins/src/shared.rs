//! The UI can hold an instance while creating its editor. Renderers skip that instance
//! while it is busy, but releases must still reach it once it can process again.

use crate::{Params, Plugin};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, LockResult, Mutex, MutexGuard, TryLockResult};
use vst3::ComPtr;
use vst3::Steinberg::Vst::IEditController;

pub type Shared = Arc<SharedPlugin>;

/// UI-thread parameter/controller handle stored alongside the audio mutex so UI reads
/// and writes never contend with the audio thread's `try_audio`.
struct UiParams(Params);

// SAFETY: `UiParams` is only handed out on the main thread (`crate::on_main_thread()`)
// and its `Drop` runs before `SharedPlugin`'s `Plugin` (or is forgotten if faulted).
unsafe impl Send for UiParams {}
unsafe impl Sync for UiParams {}

/// A plug-in and the MIDI state that must survive a missed render block.
pub struct SharedPlugin {
    ui_params: Option<UiParams>,
    faulted: Option<Arc<AtomicBool>>,
    plugin: Mutex<Plugin>,
    offs: [AtomicU64; 2],
    stop: AtomicBool,
    controls: [AtomicU16; 130],
    controls_pending: AtomicBool,
    pending: AtomicBool,
}

impl Drop for SharedPlugin {
    fn drop(&mut self) {
        if let Some(params) = self.ui_params.take()
            && self.faulted.as_ref().is_some_and(|f| f.load(Ordering::Relaxed))
        {
            std::mem::forget(params);
        }
    }
}

impl Plugin {
    pub fn shared(self) -> Shared {
        let ui_params = self.params().map(UiParams);
        let faulted = self.faulted_flag();
        Arc::new(SharedPlugin {
            ui_params,
            faulted,
            plugin: Mutex::new(self),
            offs: Default::default(),
            stop: AtomicBool::new(false),
            controls: std::array::from_fn(|_| AtomicU16::new(0)),
            controls_pending: AtomicBool::new(false),
            pending: AtomicBool::new(false),
        })
    }
}

impl SharedPlugin {
    pub fn lock(&self) -> LockResult<MutexGuard<'_, Plugin>> {
        self.plugin.lock()
    }

    pub fn try_lock(&self) -> TryLockResult<MutexGuard<'_, Plugin>> {
        self.plugin.try_lock()
    }

    /// A handle to read and change a VST3's or Audio Unit's parameters without locking
    /// the audio instance. Main thread only; `None` for DECKARD's own plug-ins.
    pub fn params(&self) -> Option<Params> {
        if !crate::on_main_thread() || self.faulted.as_ref().is_some_and(|f| f.load(Ordering::Relaxed)) {
            return None;
        }
        self.ui_params.as_ref().map(|p| p.0.clone())
    }

    /// The VST3 edit controller, without locking the audio instance. Main thread only.
    pub fn controller(&self) -> Option<ComPtr<IEditController>> {
        self.params().and_then(|p| p.controller())
    }

    /// Whether a VST3's state was marked dirty since `take_dirty`, without locking the
    /// audio instance (`None` for native/AU plug-ins).
    pub fn is_dirty(&self) -> Option<bool> {
        if self.faulted.as_ref().is_some_and(|f| f.load(Ordering::Relaxed)) {
            return Some(false);
        }
        self.ui_params.as_ref().and_then(|p| p.0.is_dirty())
    }

    /// Acquire the instance without waiting, applying deferred releases and controls
    /// before accepting a fresh event or processing new audio.
    pub fn try_audio(&self) -> TryLockResult<MutexGuard<'_, Plugin>> {
        let mut p = self.try_lock()?;
        if !self.pending.load(Ordering::Acquire) || !self.pending.swap(false, Ordering::AcqRel) {
            return Ok(p);
        }
        if self.stop.swap(false, Ordering::AcqRel) {
            p.all_notes_off();
        }
        for (i, word) in self.offs.iter().enumerate() {
            let mut bits = word.swap(0, Ordering::AcqRel);
            while bits != 0 {
                let pitch = bits.trailing_zeros() as usize + 64 * i;
                p.note_off(pitch as i64);
                bits &= bits - 1;
            }
        }
        if self.controls_pending.swap(false, Ordering::AcqRel) {
            for (number, control) in self.controls.iter().enumerate() {
                if let Some(value) = control.swap(0, Ordering::AcqRel).checked_sub(1) {
                    apply_control(&mut p, number, value);
                }
            }
        }
        Ok(p)
    }

    pub fn audio_note_off(&self, pitch: i64) {
        if let Ok(mut p) = self.try_audio() {
            p.note_off(pitch);
        } else {
            let pitch = pitch.clamp(0, 127) as usize;
            self.offs[pitch / 64].fetch_or(1 << (pitch % 64), Ordering::Release);
            self.pending.store(true, Ordering::Release);
        }
    }

    pub fn audio_all_notes_off(&self) {
        if let Ok(mut p) = self.try_audio() {
            p.all_notes_off();
        } else {
            // Stop resets sustain, pressure and bend. Discard earlier values of these
            // controls, so they cannot undo the reset when rendering resumes.
            for number in [64, 128, 129] {
                self.controls[number].store(0, Ordering::Relaxed);
            }
            self.stop.store(true, Ordering::Release);
            self.pending.store(true, Ordering::Release);
        }
    }

    /// Keep the most recent value of each MIDI control while the instance is busy.
    /// 128 is channel pressure and 129 is pitch bend.
    pub fn audio_control(&self, number: usize, value: u16) {
        if let Ok(mut p) = self.try_audio() {
            apply_control(&mut p, number, value);
        } else if let Some(control) = self.controls.get(number) {
            control.store(value.min(16383) + 1, Ordering::Relaxed);
            self.controls_pending.store(true, Ordering::Release);
            self.pending.store(true, Ordering::Release);
        }
    }
}

fn apply_control(p: &mut Plugin, number: usize, value: u16) {
    match number {
        128 => p.channel_pressure(value.min(127) as u8),
        129 => p.pitch_bend(value),
        cc @ 0..=127 => p.control_change(cc as u8, value.min(127) as u8),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_plugin_exposes_lock_free_accessors_without_locking_instance() {
        let class = &crate::tuner::CLASS;
        let id = crate::PluginId::parse(&class.plugin_id(), class.name).unwrap();
        let native = Plugin::native(class, id, class.name, 48_000.0).shared();
        let _audio = native.lock().unwrap();
        assert!(native.params().is_none());
        assert!(native.controller().is_none());
        assert!(native.is_dirty().is_none());
    }
}
