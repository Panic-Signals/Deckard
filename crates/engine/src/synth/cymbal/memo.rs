//! `NI::DSP::UTIL::MemoEnvelope`: the attack–hold–decay–release envelope of Maschine's
//! modal drums (docs/synth-cymbal.md §5.1). It runs in `f64`; times are pitches on
//! Maschine's exp table (`E(x)`, [`crate::fx::exp_e`]), so a time parameter `t` gives a
//! per-sample step `E((60 − t)·9.965784 + 1200)/sr`.
//!
//! The owners inline its per-sample step; [`MemoEnvelope::process`] is that step, as
//! `ParallelNoise::processSample` and `CymbalBodyCore::processSample` run it.

use crate::fx::exp_e;

/// `log2(10)·3`: semitones → exp-table steps, as Maschine writes it (`0x411f73da`).
pub(super) const K: f32 = f32::from_bits(0x411f_73da);
/// Envelopes and amplitudes end below this (`1e-4` as `f64` from `f32`).
pub(super) const FLOOR: f64 = 9.999_999_747_378_752e-5;

#[derive(Clone, Debug, Default)]
pub(super) struct MemoEnvelope {
    /// +0x00: 0 idle, 1 attack, 2 hold, 3 decay, 4 release.
    pub state: u32,
    /// +0x04: attack time parameter (its step is `E((60 − a)·K + 1200)/sr`).
    pub attack: f32,
    /// +0x08: hold time parameter (`(E(h·K + 602.05) − 0.001)·sr` samples).
    pub hold: f32,
    /// +0x0c: decay time parameter.
    pub decay: f32,
    /// +0x10: release time parameter (a multiplier `1 − E(…)/sr` per sample).
    pub release: f32,
    /// +0x18: attack step per unit level.
    pub attack_step: f64,
    /// +0x20: hold length in samples.
    pub hold_len: f64,
    /// +0x28: decay step per unit level.
    pub decay_step: f64,
    /// +0x30: sustain level as a fraction of the peak.
    pub sustain: f64,
    /// +0x38: release multiplier.
    pub release_mul: f64,
    /// +0x40: attack step of the current note (`attack_step·peak`).
    pub inc: f64,
    /// +0x48: decay step of the current note (`(sustain·peak − peak)·decay_step`).
    pub dec: f64,
    /// +0x50: sustain level of the current note.
    pub sus: f64,
    /// +0x58: the note's peak as `f32` (the hold stage's output).
    pub peak32: f32,
    /// +0x60: the note's peak.
    pub peak: f64,
    /// +0x68: the current level.
    pub cur: f64,
    /// +0x70: samples held so far.
    pub count: u32,
    /// +0x74: sample rate.
    pub sr: f32,
    /// +0x78: `1/sr`.
    pub inv_sr: f64,
}

impl MemoEnvelope {
    /// `setSampleRate(sr)` (0x1cd4150): the steps from the time parameters.
    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr;
        let inv = 1.0 / f64::from(sr);
        self.inv_sr = inv;
        self.attack_step = inv * f64::from(exp_e((60.0 - self.attack).mul_add(K, 1200.0)));
        let h = exp_e(self.hold.mul_add(K, f32::from_bits(0x4416_8363)));
        self.hold_len = f64::from((h + -0.001) * sr);
        self.decay_step = inv * f64::from(exp_e((60.0 - self.decay).mul_add(K, 1200.0)));
        let r = inv * f64::from(exp_e((60.0 - self.release).mul_add(K, 1200.0)));
        self.release_mul = if r < 1.0 { 1.0 - r } else { 0.0 };
    }

    /// `setNoteOn(v)` (0x1cd460c): start (or restart, soft-limited against the current
    /// level) at peak `v` (0..1; other values are ignored).
    pub fn set_note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        let old = self.state;
        if old == 0 {
            self.start(v);
            self.state = 1;
            return;
        }
        let cur = self.cur;
        let c = cur as f32;
        let mut v = c + v;
        let room = 1.0 - c;
        if v >= room {
            // Soft knee above `1 − cur`: `room + x·(1 − x/(4·span))`, x ≤ 2·span.
            let span = {
                let s = 1.0 - room;
                if s < 1e-12 { f32::from_bits(0x2b8c_bccc) } else { s }
            };
            let mut x = v - room;
            if x > span + span {
                x = span + span;
            }
            v = x.mul_add((x * -0.25) / span + 1.0, room);
        }
        self.start(v);
        let peak = self.peak;
        let mut s = old;
        match old {
            4 => {
                if cur > peak {
                    return;
                }
                self.state = 1;
                if cur >= peak {
                    self.state = 2;
                }
                return;
            }
            3 => {
                if cur <= peak {
                    self.state = 1;
                    if cur >= peak {
                        self.state = 2;
                    }
                    return;
                }
                if self.sus < self.cur {
                    return;
                }
                self.state = 4;
                return;
            }
            2 => {
                s = if self.sustain == 1.0 { 4 } else { 3 };
                if cur <= peak {
                    s = 1;
                }
                self.state = s;
                self.count = 0;
            }
            _ => {}
        }
        match s {
            0 => self.state = 1,
            1 => {
                if self.cur >= peak {
                    self.state = 2;
                }
            }
            2 => {
                if self.hold_len <= f64::from(self.count) {
                    self.state = 3;
                }
                self.count = 0;
            }
            3 => {
                if self.sus >= self.cur {
                    self.state = 4;
                }
            }
            _ => {}
        }
    }

    fn start(&mut self, v: f32) {
        self.peak32 = v;
        let p = f64::from(v);
        self.peak = p;
        self.inc = self.attack_step * p;
        let sus = self.sustain * p;
        self.dec = (sus - p) * self.decay_step;
        self.sus = sus;
    }

    /// One sample of the envelope (inlined in its owners), as `f32`.
    #[inline]
    pub fn process(&mut self) -> f32 {
        let d = match self.state {
            0 => return 0.0,
            1 => {
                let d = self.inc + self.cur;
                self.cur = d;
                if d >= self.peak {
                    self.cur = self.peak;
                    self.state = 2;
                    self.peak
                } else {
                    d
                }
            }
            2 => {
                self.count += 1;
                if self.hold_len <= f64::from(self.count) {
                    self.state = 3;
                    self.count = 0;
                }
                return self.peak32;
            }
            3 => {
                let d = self.dec + self.cur;
                self.cur = d;
                let d = if self.sus >= d {
                    self.cur = self.sus;
                    self.state = 4;
                    self.sus
                } else {
                    d
                };
                self.floor(d)
            }
            4 => {
                let d = self.release_mul * self.cur;
                self.cur = d;
                self.floor(d)
            }
            _ => self.cur,
        };
        d as f32
    }

    #[inline]
    fn floor(&mut self, d: f64) -> f64 {
        if d < FLOOR {
            self.state = 0;
            self.cur = 0.0;
            0.0
        } else {
            d
        }
    }
}
