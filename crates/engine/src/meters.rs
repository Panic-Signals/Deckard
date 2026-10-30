//! Meters of effects for their panels (the Compressor's input, gain reduction and
//! output), and the routes' travels for their knobs, shared between the threads that
//! render sounds and the UI without locks.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Groups, sounds per group and slots per chain that have room on the board (others
/// aren't metered).
const GROUPS: usize = 64;
const SOUNDS: usize = 16;
const SLOTS: usize = 16;

/// Peaks since the UI last read them, per effect slot: input and output level (linear)
/// and gain reduction (dB, positive). Each is an `f32` kept as bits: for positive values
/// the bits order as the values do, so `fetch_max` keeps the peak.
pub struct FxMeters {
    cells: Box<[[AtomicU32; 3]]>,
}

impl Default for FxMeters {
    fn default() -> FxMeters {
        FxMeters { cells: (0..GROUPS * SOUNDS * SLOTS).map(|_| [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)]).collect() }
    }
}

fn index(g: usize, s: usize, slot: usize) -> Option<usize> {
    (g < GROUPS && s < SOUNDS && slot < SLOTS).then(|| (g * SOUNDS + s) * SLOTS + slot)
}

impl FxMeters {
    /// What an effect did over a block (the audio side).
    pub fn record(&self, g: usize, s: usize, slot: usize, input: f32, reduction_db: f32, output: f32) {
        let Some(i) = index(g, s, slot) else { return };
        for (cell, v) in self.cells[i].iter().zip([input, reduction_db, output]) {
            if v.is_finite() && v > 0.0 {
                cell.fetch_max(v.to_bits(), Ordering::Relaxed);
            }
        }
    }

    /// The peaks since the last call: (input, gain reduction in dB, output) (the UI).
    pub fn take(&self, g: usize, s: usize, slot: usize) -> (f32, f32, f32) {
        let Some(i) = index(g, s, slot) else { return (0.0, 0.0, 0.0) };
        let [a, b, c] = &self.cells[i];
        let f = |x: &AtomicU32| f32::from_bits(x.swap(0, Ordering::Relaxed));
        (f(a), f(b), f(c))
    }
}

/// Routes per sound that have room on [`RouteTravels`] (others' knobs don't move).
pub const ROUTES: usize = 32;

/// Each route's travel now (0..1), by group, sound and its place in the sound's routes:
/// the route's [`route_key`] (high 32 bits) and the travel as `f32` bits (`NaN`: not
/// driving its knob), in one atomic so they're read together.
pub struct RouteTravels {
    cells: Box<[AtomicU64]>,
}

impl Default for RouteTravels {
    fn default() -> RouteTravels {
        RouteTravels { cells: (0..GROUPS * SOUNDS * ROUTES).map(|_| AtomicU64::new(u64::from(f32::NAN.to_bits()))).collect() }
    }
}

fn route_index(g: usize, s: usize, k: usize) -> Option<usize> {
    (g < GROUPS && s < SOUNDS && k < ROUTES).then(|| (g * SOUNDS + s) * ROUTES + k)
}

/// Which route a travel is (its slot and parameter), so a knob doesn't read another's
/// while an edit reaches the engine.
pub fn route_key(r: &deckard_model::mod_matrix::ModRoute) -> u32 {
    use deckard_model::mod_matrix::ModParam;
    // FNV-1a.
    let mut h: u32 = 0x811c_9dc5;
    let mut eat = |b: &[u8]| {
        for x in b {
            h = (h ^ u32::from(*x)).wrapping_mul(0x0100_0193);
        }
    };
    eat(&(r.slot as u64).to_le_bytes());
    match &r.param {
        ModParam::Field(f) => {
            eat(b"f");
            eat(f.as_bytes());
        }
        ModParam::Plugin(id) => {
            eat(b"p");
            eat(&id.to_le_bytes());
        }
    }
    h
}

impl RouteTravels {
    /// Route `k` of sound (`g`, `s`), keyed `key`, is at `travel` (`NaN`: not driving)
    /// (the audio side).
    pub fn set(&self, g: usize, s: usize, k: usize, key: u32, travel: f32) {
        if let Some(i) = route_index(g, s, k) {
            self.cells[i].store((u64::from(key) << 32) | u64::from(travel.to_bits()), Ordering::Relaxed);
        }
    }

    /// Route `k` of sound (`g`, `s`)'s travel, if it drives its knob and is the route
    /// keyed `key` (the UI).
    pub fn get(&self, g: usize, s: usize, k: usize, key: u32) -> Option<f32> {
        let x = self.cells[route_index(g, s, k)?].load(Ordering::Relaxed);
        let t = f32::from_bits(x as u32);
        ((x >> 32) as u32 == key && !t.is_nan()).then_some(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_hold_until_read() {
        let m = FxMeters::default();
        m.record(1, 2, 3, 0.5, 6.0, 0.25);
        m.record(1, 2, 3, 0.25, 9.0, 0.5);
        assert_eq!(m.take(1, 2, 3), (0.5, 9.0, 0.5));
        assert_eq!(m.take(1, 2, 3), (0.0, 0.0, 0.0), "read: reset");
        m.record(99, 0, 0, 1.0, 1.0, 1.0);
        assert_eq!(m.take(99, 0, 0), (0.0, 0.0, 0.0), "off the board");
    }
}
