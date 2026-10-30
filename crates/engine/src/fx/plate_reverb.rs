//! Maschine's Reverb module (`NI::MASCHINE::DATA::PlateReverb`, docs/fx-plate-reverb.md):
//! three algorithms behind one Mode selector, as `RT::PlateReverbProcessor` runs them.
//! Room and Hall are two instances of `NI::DSP::GaloisReverb` (a 32-line feedback delay
//! network, [`galois`]); Plate is `NI::DSP::PlateReverb` ([`plate`]). Only the selected
//! engine runs; all three always take their parameters. The engines' control-rate updates
//! run every 128 samples on one shared counter.

use super::Effect;

mod galois;
pub(crate) mod plate;

use galois::Galois;
use plate::Plate;

/// An `f32` constant as Maschine stores it.
pub(crate) const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// The processor's control-rate period (samples).
const BLOCK: usize = 128;

/// `__exp10f` from the system math library, which Maschine calls (10^x).
#[cfg(target_os = "macos")]
pub(crate) fn exp10f(x: f32) -> f32 {
    unsafe extern "C" {
        fn __exp10f(x: f32) -> f32;
    }
    // SAFETY: a pure libm function.
    unsafe { __exp10f(x) }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn exp10f(x: f32) -> f32 {
    10f32.powf(x)
}

/// `APPROXUTIL::sin_rx5`'s polynomial in `x²`: `sin x ≈ x·sin_poly(x²)`.
pub(crate) fn sin_poly(x2: f32) -> f32 {
    const S: [u32; 6] = [0xb2cd4ca8, 0x3638b94f, 0xb9500c03, 0x3c088885, 0xbe2aaac1, 0x3f800000];
    let mut p = x2 * c(S[0]);
    for &k in &S[1..5] {
        p = x2 * (c(k) + p);
    }
    c(S[5]) + p
}

/// `APPROXUTIL::cos_rx5`'s polynomial: `cos x ≈ cos_poly(x²)`.
pub(crate) fn cos_poly(x2: f32) -> f32 {
    const C: [u32; 6] = [0xb48bdad9, 0x37cfb5a1, 0xbab609bd, 0x3d2aaa99, 0xbefff5e8, 0x3f800000];
    let mut p = x2 * c(C[0]);
    for &k in &C[1..5] {
        p = x2 * (c(k) + p);
    }
    c(C[5]) + p
}

/// A module field's stored value, if the project has it.
fn get(params: &[(String, f64)], field: &str) -> Option<f32> {
    params.iter().find(|(k, _)| k == field).map(|(_, v)| *v as f32)
}

/// Room and Hall fields, in `GaloisReverb` parameter order 0..7, with their defaults
/// (`DATA::PlateReverb::PlateReverb()`).
const ROOM: [(&str, f32); 8] = [
    ("room_rev_time", 0.2),
    ("room_size", 0.2),
    ("room_damp", 0.5),
    ("room_mod", 0.2),
    ("room_diff", 0.8),
    ("room_pre_delay", 0.0),
    ("room_high_cut", 0.0),
    ("room_low_shelf", 0.0),
];
const HALL: [(&str, f32); 8] = [
    ("hall_rev_time", 0.4),
    ("hall_size", 0.1),
    ("hall_damp", 0.5),
    ("hall_mod", 0.4),
    ("hall_diff", 0.9),
    ("hall_pre_delay", 0.0),
    ("hall_high_cut", 0.0),
    ("hall_low_shelf", 0.0),
];
/// Plate fields and their `PlateReverb::setParameter` indices.
const PLATE: [(&str, u32); 5] =
    [("plate_pre_delay", 0), ("plate_decay", 1), ("plate_low_shelf", 9), ("plate_high_damp", 10), ("wet_gain", 8)];

pub struct PlateReverb {
    sr: f32,
    plate: Box<Plate>,
    room: Box<Galois>,
    hall: Box<Galois>,
    /// 0 Room, 1 Hall, 2 Plate.
    mode: usize,
    /// Samples to the next control-rate update.
    countdown: usize,
    started: bool,
    /// The values last sent, to send only changes after the start.
    sent: Vec<(String, f32)>,
}

impl PlateReverb {
    pub fn new(sr: f64) -> Self {
        PlateReverb {
            sr: sr as f32,
            plate: Plate::new(),
            room: Galois::new(false),
            hall: Galois::new(true),
            mode: 0,
            countdown: 0,
            started: false,
            sent: Vec::new(),
        }
    }

    /// `PlateReverbProcessor::setMode`: the newly chosen engine is cleared.
    fn set_mode(&mut self, mode: usize) {
        self.mode = mode;
        match mode {
            0 => self.room.clear_for_mode(),
            1 => self.hall.clear_for_mode(),
            2 => self.plate.reset(),
            _ => {}
        }
    }

    /// `PlateReverbProcessor::processParameter` (or `setMode`) for one field.
    fn send(&mut self, field: &str, v: f32) {
        if let Some(i) = ROOM.iter().position(|(f, _)| *f == field) {
            self.room.set_parameter(i as u32, v);
        } else if let Some(i) = HALL.iter().position(|(f, _)| *f == field) {
            self.hall.set_parameter(i as u32, v);
        } else if let Some(&(_, i)) = PLATE.iter().find(|(f, _)| *f == field) {
            self.plate.set_parameter(i, v);
        } else if field == "mix" {
            self.room.set_parameter(8, v);
            self.hall.set_parameter(8, v);
            self.plate.set_parameter(7, v);
        } else if field == "mode" {
            self.set_mode((v.max(0.0) as usize).min(2));
        }
    }
}

impl Effect for PlateReverb {
    fn set(&mut self, params: &[(String, f64)]) {
        // Version-0 projects (Plate only) have no Mode and no Wet Gain: Maschine loads
        // them as Plate with Wet Gain 1.
        let v0 = get(params, "mode").is_none();
        let mut vals: Vec<(String, f32)> = Vec::new();
        let mut put = |f: &str, d: f32| vals.push((f.to_string(), get(params, f).unwrap_or(d)));
        // `DATA::PlateReverb::connectParameters` order.
        put("mode", if v0 { 2.0 } else { 0.0 });
        put("plate_pre_delay", 0.0);
        put("plate_decay", 0.3);
        put("mix", 0.5);
        put("plate_low_shelf", 0.0);
        put("plate_high_damp", 0.0);
        put("wet_gain", if v0 { 1.0 } else { 3.5 });
        for (f, d) in ROOM.iter().chain(HALL.iter()) {
            put(f, *d);
        }
        if !self.started {
            self.started = true;
            for (f, v) in &vals {
                self.send(f, *v);
            }
            // onSamplerateChanged(sr), then resume().
            self.room.set_sample_rate(self.sr);
            self.hall.set_sample_rate(self.sr);
            self.plate.set_sample_rate(self.sr, BLOCK as i32);
            self.plate.reset();
            self.hall.reset();
            self.room.reset();
            self.room.resume();
            self.hall.resume();
            self.plate.resume();
        } else {
            for (f, v) in &vals {
                let old = self.sent.iter().find(|(k, _)| k == f).map(|(_, x)| *x);
                if old != Some(*v) {
                    self.send(f, *v);
                }
            }
        }
        self.sent = vals;
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let mut i = 0;
        while i < l.len() {
            if self.countdown == 0 {
                match self.mode {
                    0 => self.room.control(),
                    1 => self.hall.control(),
                    2 => self.plate.control(),
                    _ => {}
                }
                self.countdown = BLOCK;
            }
            let n = self.countdown.min(l.len() - i);
            let (a, b) = (&mut l[i..i + n], &mut r[i..i + n]);
            match self.mode {
                0 => self.room.process(a, b),
                1 => self.hall.process(a, b),
                2 => self.plate.process(a, b),
                _ => {}
            }
            self.countdown -= n;
            i += n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(kv: &[(&str, f64)]) -> Vec<(String, f64)> {
        kv.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect()
    }

    fn run(fx: &mut PlateReverb, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut a, mut b) = (l.to_vec(), r.to_vec());
        for (x, y) in a.chunks_mut(100).zip(b.chunks_mut(100)) {
            fx.process(x, y);
        }
        (a, b)
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn every_mode_rings_after_the_input_stops() {
        for mode in 0..3 {
            let mut fx = PlateReverb::new(48000.0);
            fx.set(&p(&[("mode", mode as f64), ("mix", 1.0)]));
            let mut l = noise(4800, 1);
            let mut r = noise(4800, 2);
            l.resize(48000, 0.0);
            r.resize(48000, 0.0);
            let (a, b) = run(&mut fx, &l, &r);
            assert!(a.iter().chain(&b).all(|v| v.is_finite()), "mode {mode}");
            let tail = rms(&a[9600..19200]) + rms(&b[9600..19200]);
            assert!(tail > 1e-3, "mode {mode}: tail {tail}");
            assert!(rms(&a[40000..]) < rms(&a[9600..19200]), "mode {mode} decays");
        }
    }

    #[test]
    fn room_and_hall_at_mix_zero_are_the_identity() {
        for mode in 0..2 {
            let mut fx = PlateReverb::new(44100.0);
            fx.set(&p(&[("mode", mode as f64), ("mix", 0.0)]));
            let l = noise(5000, 3);
            let r = noise(5000, 4);
            let (a, b) = run(&mut fx, &l, &r);
            assert_eq!(a, l);
            assert_eq!(b, r);
        }
    }

    #[test]
    fn silence_stays_silent() {
        for mode in 0..3 {
            let mut fx = PlateReverb::new(44100.0);
            fx.set(&p(&[("mode", mode as f64)]));
            let z = vec![0.0f32; 20000];
            let (a, b) = run(&mut fx, &z, &z);
            assert!(a.iter().chain(&b).all(|v| *v == 0.0), "mode {mode}");
        }
    }

    #[test]
    fn set_keeps_state_and_resending_changes_nothing() {
        for mode in 0..3 {
            let prm = p(&[("mode", mode as f64), ("mix", 0.6)]);
            let mut a = PlateReverb::new(48000.0);
            let mut b = PlateReverb::new(48000.0);
            a.set(&prm);
            b.set(&prm);
            let l = noise(9600, 5);
            let r = noise(9600, 6);
            let (a1, _) = run(&mut a, &l[..4800], &r[..4800]);
            let (b1, _) = run(&mut b, &l[..4800], &r[..4800]);
            b.set(&prm);
            let (a2, _) = run(&mut a, &l[4800..], &r[4800..]);
            let (b2, _) = run(&mut b, &l[4800..], &r[4800..]);
            assert_eq!(a1, b1);
            assert_eq!(a2, b2, "mode {mode}");
        }
    }

    #[test]
    fn parameter_changes_ramp_without_blowing_up() {
        for mode in 0..3 {
            let mut fx = PlateReverb::new(48000.0);
            fx.set(&p(&[("mode", mode as f64)]));
            let l = noise(48000, 7);
            let r = noise(48000, 8);
            let (_, _) = run(&mut fx, &l[..12800], &r[..12800]);
            fx.set(&p(&[
                ("mode", mode as f64),
                ("room_size", 0.9),
                ("hall_diff", 0.1),
                ("plate_decay", 1.0),
                ("mix", 0.9),
                ("room_high_cut", 1.0),
            ]));
            let (a, b) = run(&mut fx, &l[12800..], &r[12800..]);
            assert!(a.iter().chain(&b).all(|v| v.is_finite() && v.abs() < 10.0), "mode {mode}");
        }
    }

    #[test]
    fn version_zero_projects_are_plate() {
        let mut fx = PlateReverb::new(44100.0);
        fx.set(&p(&[("plate_decay", 0.5), ("mix", 0.5)]));
        assert_eq!(fx.mode, 2);
    }

    #[test]
    fn approximations_are_close() {
        for i in 0..20 {
            let x = i as f32 * 0.08;
            assert!((x * sin_poly(x * x) - x.sin()).abs() < 1e-3, "sin {x}");
            assert!((cos_poly(x * x) - x.cos()).abs() < 1e-3, "cos {x}");
        }
        assert!((exp10f(0.3) - 10f32.powf(0.3)).abs() < 1e-5);
    }
}
