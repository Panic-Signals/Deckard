//! `NI::DSP::UTIL::MemoEnvelope` (0x80 bytes): attack / hold / decay / release with a
//! "memory" of its level, so a retrigger continues from where it is. [exact]
//!
//! Times are in Maschine's log units: a rate is `E((60 − t)·9.965784 + 1200) / sr`
//! (`E` = `NI::AB::ExpTable`, i.e. `10^((60 − t)/20) / sr` per sample), a hold is
//! `(E(h·9.965784 + 602.0529) − 0.001)·sr` samples (`10^(h/20)` ms − 1 ms).

use crate::fx::exp_e;

/// `t · 9.965784` (20·log10 → table units), as Maschine writes it.
pub(crate) const DB: f32 = f32::from_bits(0x411f_73da);
/// The hold offset 602.0529 (−0.001 s base).
const HOLD0: f32 = f32::from_bits(0x4416_8363);
/// −0.001.
pub(crate) const MS: f32 = f32::from_bits(0xba83_126f);
/// 1e-4 as f32, compared in double: the level below which the envelope stops.
pub(crate) const FLOOR: f64 = 9.999_999_747_378_752e-5;

/// `E((60 − t)·9.965784 + 1200)`: a rate per second.
pub(crate) fn rate(t: f32) -> f32 {
    exp_e((60.0 - t).mul_add(DB, 1200.0))
}

/// `1 − E(..)/sr`, or 0 when that is ≤ 0: a release multiplier.
pub(crate) fn release_mul(inv_sr: f64, t: f32) -> f64 {
    let d = inv_sr * f64::from(rate(t));
    if d < 1.0 { 1.0 - d } else { 0.0 }
}

/// `(E(h·9.965784 + 602.05) − 0.001)·sr`: a hold length in samples.
pub(crate) fn hold_len(sr: f32, h: f32) -> f64 {
    f64::from((exp_e(h.mul_add(DB, HOLD0)) + MS) * sr)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Env {
    /// +0x00: 0 idle, 1 attack, 2 hold, 3 decay, 4 release.
    pub state: u32,
    /// +0x04 attack, +0x08 hold, +0x0c decay, +0x10 release (log units).
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    pub release: f32,
    /// +0x18: attack rate (per sample, times the level at note-on).
    pub attack_rate: f64,
    /// +0x20: hold length (samples).
    pub hold_len: f64,
    /// +0x28: decay rate.
    pub decay_rate: f64,
    /// +0x30: sustain factor (1 = no decay stage).
    pub sustain: f64,
    /// +0x38: release multiplier.
    pub release_mul: f64,
    /// +0x40 attack step, +0x48 decay step, +0x50 sustain level.
    pub attack_step: f64,
    pub decay_step: f64,
    pub sustain_level: f64,
    /// +0x58 / +0x60: the peak level (f32 / f64).
    pub level_f: f32,
    pub level: f64,
    /// +0x68: the current value.
    pub cur: f64,
    /// +0x70: hold counter.
    pub count: u32,
    /// +0x74 / +0x78.
    pub sr: f32,
    pub inv_sr: f64,
}

impl Env {
    /// Constructor defaults (sr 1, nothing set).
    pub(crate) const fn new() -> Env {
        Env {
            state: 0,
            attack: 0.0,
            hold: 0.0,
            decay: 0.0,
            release: 0.0,
            attack_rate: 0.0,
            hold_len: 0.0,
            decay_rate: 0.0,
            sustain: 0.0,
            release_mul: 0.0,
            attack_step: 0.0,
            decay_step: 0.0,
            sustain_level: 0.0,
            level_f: 0.0,
            level: 0.0,
            cur: 0.0,
            count: 0,
            sr: 1.0,
            inv_sr: 1.0,
        }
    }

    /// `MemoEnvelope::setSampleRate`: the four times again (not the sustain).
    pub(crate) fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr;
        let inv = 1.0 / f64::from(sr);
        self.inv_sr = inv;
        self.attack_rate = inv * f64::from(rate(self.attack));
        self.hold_len = hold_len(sr, self.hold);
        self.decay_rate = inv * f64::from(rate(self.decay));
        self.release_mul = release_mul(inv, self.release);
    }

    /// `MemoEnvelope::setNoteOn(level)`: starts, or retriggers from where it is.
    pub(crate) fn note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        if self.state == 0 {
            self.set_level(v);
            self.state = 1;
            return;
        }
        let cur = self.cur;
        let c = cur as f32;
        let mut l = c + v;
        let top = 1.0 - c;
        if l >= top {
            // Soft knee above 1 − cur.
            let room = (1.0 - top).max(f32::from_bits(0x2b8c_bccc));
            let x = (l - top).min(room + room);
            let k = (x * -0.25) / room + 1.0;
            l = x.mul_add(k, top);
        }
        self.set_level(l);
        let level = self.level;
        match self.state {
            4 => {
                if cur > level {
                    return;
                }
                self.state = 1;
                if cur < level {
                    return;
                }
                self.state = 2;
            }
            3 => {
                if cur <= level {
                    self.state = 1;
                    if cur < level {
                        return;
                    }
                    self.state = 2;
                } else {
                    if self.sustain_level < self.cur {
                        return;
                    }
                    self.state = 4;
                }
            }
            2 => {
                let mut s = if self.sustain == 1.0 { 4 } else { 3 };
                if cur <= level {
                    s = 1;
                }
                self.state = s;
                self.count = 0;
                match s {
                    1 => {
                        if cur < level {
                            return;
                        }
                        self.state = 2;
                    }
                    3 => {
                        if self.sustain_level < self.cur {
                            return;
                        }
                        self.state = 4;
                    }
                    _ => {}
                }
            }
            1 => {
                if cur < level {
                    return;
                }
                self.state = 2;
            }
            _ => {}
        }
    }

    fn set_level(&mut self, l: f32) {
        self.level_f = l;
        let d = f64::from(l);
        self.level = d;
        self.attack_step = self.attack_rate * d;
        let s = self.sustain * d;
        self.decay_step = (s - d) * self.decay_rate;
        self.sustain_level = s;
    }

    /// One sample (the inlined `MemoEnvelope` step).
    #[inline]
    pub(crate) fn tick(&mut self) -> f32 {
        match self.state {
            0 => 0.0,
            1 => {
                let c = self.attack_step + self.cur;
                self.cur = c;
                if c >= self.level {
                    self.cur = self.level;
                    self.state = 2;
                }
                self.cur as f32
            }
            2 => {
                self.count = self.count.wrapping_add(1);
                if self.hold_len <= f64::from(self.count) {
                    self.state = 3;
                    self.count = 0;
                }
                self.level_f
            }
            3 => {
                let c = self.decay_step + self.cur;
                self.cur = c;
                if self.sustain_level >= c {
                    self.cur = self.sustain_level;
                    self.state = 4;
                }
                self.floor()
            }
            4 => {
                self.cur *= self.release_mul;
                self.floor()
            }
            _ => self.cur as f32,
        }
    }

    #[inline]
    fn floor(&mut self) -> f32 {
        if self.cur < FLOOR {
            self.state = 0;
            self.cur = 0.0;
            0.0
        } else {
            self.cur as f32
        }
    }
}
