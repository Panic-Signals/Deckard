//! Meters of effects for their panels (the Compressor's input, gain reduction and
//! output), and the routes' travels for their knobs, shared between the threads that
//! render sounds and the UI without locks; and which plug-ins' extra outputs made sound.

use deckard_model::extra_out::Owner;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

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

/// A plug-in slot's place on [`HeardBoard`]: per sound, its instrument then its chain's
/// slots; then each group's slots; then the master's.
const SOUND_CELLS: usize = SLOTS + 1;
const GROUP_BASE: usize = GROUPS * SOUNDS * SOUND_CELLS;
const MASTER_BASE: usize = GROUP_BASE + GROUPS * SLOTS;

fn heard_index(o: Owner) -> Option<usize> {
    match o {
        Owner::Sound(g, s) => (g < GROUPS && s < SOUNDS).then(|| (g * SOUNDS + s) * SOUND_CELLS),
        Owner::Chain(g, s, k) => (g < GROUPS && s < SOUNDS && k < SLOTS).then(|| (g * SOUNDS + s) * SOUND_CELLS + 1 + k),
        Owner::Group(g, k) => (g < GROUPS && k < SLOTS).then(|| GROUP_BASE + g * SLOTS + k),
        Owner::Master(k) => (k < SLOTS).then_some(MASTER_BASE + k),
    }
}

/// Which of plug-ins' extra output pairs (buses 1 and up, as bits) made sound since the
/// engine last looked, per plug-in slot, shared between the threads that render sounds
/// and the audio thread without locks. `groups` (bit per group) and `master` say where
/// to look, so a look costs nothing when nothing was heard.
pub struct HeardBoard {
    cells: Box<[AtomicU64]>,
    groups: AtomicU64,
    master: AtomicBool,
}

impl Default for HeardBoard {
    fn default() -> HeardBoard {
        HeardBoard {
            cells: (0..MASTER_BASE + SLOTS).map(|_| AtomicU64::new(0)).collect(),
            groups: AtomicU64::new(0),
            master: AtomicBool::new(false),
        }
    }
}

impl HeardBoard {
    /// Output pair `bus` of the plug-in in `o`'s slot made sound (the rendering side).
    /// Slots off the board, or buses past 63, aren't kept.
    pub fn hit(&self, o: Owner, bus: usize) {
        let Some(i) = heard_index(o).filter(|_| bus < 64) else { return };
        self.cells[i].fetch_or(1 << bus, Ordering::Relaxed);
        match o {
            Owner::Master(_) => self.master.store(true, Ordering::Release),
            Owner::Sound(g, ..) | Owner::Chain(g, ..) | Owner::Group(g, _) => {
                self.groups.fetch_or(1 << g, Ordering::Release);
            }
        }
    }

    /// Each (slot, bus) heard since the last call, once, in mixer order: by group, its
    /// own slots, then its sounds (each one's instrument, then its chain's slots); the
    /// master's last. No allocating (the audio thread calls it).
    pub fn take(&self, mut f: impl FnMut(Owner, usize)) {
        let mut each = |i: usize, o: Owner| {
            let mut bits = self.cells[i].swap(0, Ordering::Relaxed);
            while bits != 0 {
                f(o, bits.trailing_zeros() as usize);
                bits &= bits - 1;
            }
        };
        let mut groups = self.groups.swap(0, Ordering::Acquire);
        while groups != 0 {
            let g = groups.trailing_zeros() as usize;
            groups &= groups - 1;
            for k in 0..SLOTS {
                each(GROUP_BASE + g * SLOTS + k, Owner::Group(g, k));
            }
            for s in 0..SOUNDS {
                each((g * SOUNDS + s) * SOUND_CELLS, Owner::Sound(g, s));
                for k in 0..SLOTS {
                    each((g * SOUNDS + s) * SOUND_CELLS + 1 + k, Owner::Chain(g, s, k));
                }
            }
        }
        if self.master.swap(false, Ordering::Acquire) {
            for k in 0..SLOTS {
                each(MASTER_BASE + k, Owner::Master(k));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heard_buses_come_once_in_mixer_order() {
        let b = HeardBoard::default();
        b.hit(Owner::Master(0), 1);
        b.hit(Owner::Group(1, 0), 2);
        b.hit(Owner::Chain(1, 0, 1), 3);
        b.hit(Owner::Sound(1, 0), 1);
        b.hit(Owner::Sound(1, 0), 1);
        b.hit(Owner::Sound(0, 2), 2);
        b.hit(Owner::Sound(99, 0), 1);
        let mut v = Vec::new();
        b.take(|o, bus| v.push((o, bus)));
        assert_eq!(
            v,
            [(Owner::Sound(0, 2), 2), (Owner::Group(1, 0), 2), (Owner::Sound(1, 0), 1), (Owner::Chain(1, 0, 1), 3), (Owner::Master(0), 1)]
        );
        v.clear();
        b.take(|o, bus| v.push((o, bus)));
        assert!(v.is_empty(), "taken");
    }

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
