//! The UI can hold an instance while creating its editor. Renderers wait a moment for a
//! busy instance, then skip it, but releases must still reach it once it can process again.

use crate::Plugin;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, LockResult, Mutex, MutexGuard, TryLockResult};

pub type Shared = Arc<SharedPlugin>;

/// How long the audio thread waits for an instance held elsewhere before it gives the
/// block up: well over what reading or setting a value takes, well under a block.
pub const AUDIO_WAIT: std::time::Duration = std::time::Duration::from_millis(1);

/// A plug-in and the MIDI state that must survive a missed render block.
pub struct SharedPlugin {
    plugin: Mutex<Plugin>,
    offs: [AtomicU64; 2],
    stop: AtomicBool,
    controls: [AtomicU16; 130],
    controls_pending: AtomicBool,
    pending: AtomicBool,
}

impl Plugin {
    pub fn shared(self) -> Shared {
        Arc::new(SharedPlugin {
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

    /// Acquire the instance without waiting, applying deferred releases and controls
    /// before accepting a fresh event or processing new audio.
    ///
    /// An instance held elsewhere is waited for, [`AUDIO_WAIT`] at most: the UI holds one
    /// for an instant at a time (to read a value, to set one), and giving its block up
    /// leaves an effect dry for that block, which clicks (a reverb's tail cut and back).
    /// One held longer (its editor opening) is given up on, so the audio never stalls.
    pub fn try_audio(&self) -> TryLockResult<MutexGuard<'_, Plugin>> {
        self.try_audio_within(AUDIO_WAIT)
    }

    /// [`SharedPlugin::try_audio`], waiting `wait` at most for an instance held elsewhere.
    pub fn try_audio_within(&self, wait: std::time::Duration) -> TryLockResult<MutexGuard<'_, Plugin>> {
        let mut p = match self.try_lock() {
            Ok(p) => p,
            Err(std::sync::TryLockError::WouldBlock) => self.wait_for_audio(wait)?,
            Err(e) => return Err(e),
        };
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

    /// Keep trying a held instance until `wait` is up. Spinning, not sleeping: the wait
    /// is far shorter than a scheduler's turn.
    fn wait_for_audio(&self, wait: std::time::Duration) -> TryLockResult<MutexGuard<'_, Plugin>> {
        let until = std::time::Instant::now() + wait;
        loop {
            for _ in 0..64 {
                std::hint::spin_loop();
            }
            match self.try_lock() {
                Err(std::sync::TryLockError::WouldBlock) if std::time::Instant::now() < until => {}
                other => return other,
            }
        }
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
