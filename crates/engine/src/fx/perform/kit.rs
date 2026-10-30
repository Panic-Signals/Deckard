//! The parts the Perform FX share: Maschine's linear parameter glide, its double
//! one-pole de-zipper, the tan series its filters take coefficients by, `__exp10`; and
//! the strip's own one-pole glide.

/// `__exp10` from the system math library (10^x in double), which Maschine's Perform FX
/// call.
#[cfg(target_os = "macos")]
pub fn exp10(x: f64) -> f64 {
    unsafe extern "C" {
        fn __exp10(x: f64) -> f64;
    }
    // SAFETY: a pure libm function.
    unsafe { __exp10(x) }
}

#[cfg(not(target_os = "macos"))]
pub fn exp10(x: f64) -> f64 {
    10f64.powf(x)
}

/// Maschine's linear parameter glide (the Perform FX cores' smoothers): set a target
/// and the value moves to it in equal steps over `len` samples, landing on it exactly.
/// Set while suspended (before the first sample), it's there at once.
#[derive(Debug, Clone, Copy, Default)]
pub struct Glide {
    pub cur: f32,
    pub target: f32,
    step: f32,
    lenf: f32,
    n: i32,
    count: i32,
    ramping: bool,
}

impl Glide {
    /// A glide over `sr`·0.001·20 samples (20 ms).
    pub fn new(sr: f32) -> Glide {
        let lenf = sr * 0.001 * 20.0;
        Glide { lenf, n: lenf as i32, ..Glide::default() }
    }

    pub fn set(&mut self, t: f32) {
        self.target = t;
        if self.cur != t {
            self.step = (t - self.cur) / self.lenf;
            self.count = self.n;
            self.ramping = true;
        }
    }

    /// At the target now (if it was moving).
    pub fn land(&mut self) {
        if self.ramping {
            self.cur = self.target;
            self.step = 0.0;
            self.count = 0;
            self.ramping = false;
        }
    }

    /// One sample on.
    pub fn tick(&mut self) -> f32 {
        if (self.target - self.cur).abs() > 0.0 {
            let c = self.count;
            self.count = c - 1;
            if c >= 2 {
                self.cur += self.step;
                return self.cur;
            }
        }
        self.land();
        self.cur
    }
}

/// `tan(x)` by its series to x⁵, as Maschine's filters take their coefficient (`f32`).
pub fn tan5(w: f32) -> f32 {
    let x2 = w * w;
    let x3 = w * x2;
    let x5 = x2 * x3;
    let g = x3.mul_add(f32::from_bits(0x3eaa_aaab), w);
    x5.mul_add(f32::from_bits(0x3e08_8889), g)
}

/// The same in double.
pub fn tan5d(w: f64) -> f64 {
    let x2 = w * w;
    let x3 = w * x2;
    let x5 = x2 * x3;
    let g = x3.mul_add(f64::from_bits(0x3fd5_5555_5555_554f), w);
    x5.mul_add(f64::from_bits(0x3fc1_1111_1111_1111), g)
}

/// A one-pole low-pass in double (trapezoidal), Maschine's parameter de-zipper: `g` from
/// [`tan5d`]`(π·fc/sr)`, settled at `x` by [`Tpt::settle`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Tpt {
    pub s: f64,
    g: f64,
    h: f64,
}

impl Tpt {
    pub fn new(fc: f64, inv_sr: f32) -> Tpt {
        let g = tan5d(fc * std::f64::consts::PI * f64::from(inv_sr));
        Tpt { s: 0.0, g, h: 1.0 / (g + 1.0) }
    }

    pub fn settle(&mut self, x: f32) {
        self.s = f64::from(x) * 0.5;
    }

    pub fn tick(&mut self, x: f32) -> f32 {
        let x = f64::from(x);
        let v = self.h * (-self.g).mul_add(x, self.s * -2.0) + x;
        let y = self.s.mul_add(2.0, self.g * v);
        self.s = self.g.mul_add(v, self.s);
        y as f32
    }
}

/// The per-sample step of a one-pole glide that covers most of the way in `ms`.
pub fn coef(ms: f32, sr: f32) -> f32 {
    if ms <= 0.0 { 1.0 } else { 1.0 - (-1.0 / (ms * 0.001 * sr)).exp() }
}

/// A value gliding to its target.
#[derive(Debug, Clone, Copy)]
pub struct Ramp {
    pub v: f32,
    pub target: f32,
    step: f32,
}

impl Ramp {
    pub fn new(v: f32, ms: f32, sr: f32) -> Ramp {
        Ramp { v, target: v, step: coef(ms, sr) }
    }

    pub fn next(&mut self) -> f32 {
        self.v += (self.target - self.v) * self.step;
        if (self.target - self.v).abs() < 1e-5 {
            self.v = self.target;
        }
        self.v
    }
}
