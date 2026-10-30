//! `P5::setParameterValue` (0x101aa6ed4) and the helpers it shares with the event
//! code: what each normalized parameter does to the engine (docs/synth-poly-synth.md
//! §3.2). Every multiply-add below is an `fmadd` in Maschine and is `mul_add` here.

use super::engine::{P5, PARAM_TYPE, SLOTS, clip_step};

/// `ParameterId`s (the order of `P5`'s parameter table; Maschine's knobs map onto them
/// in `PolySynth::connectParameters`).
#[allow(dead_code)]
pub(crate) mod id {
    pub const PM_ENV: u32 = 0x00;
    pub const PM_OSC_B: u32 = 0x01;
    pub const PM_FREQ_A: u32 = 0x02;
    pub const PM_PW_A: u32 = 0x03;
    pub const PM_FILTER: u32 = 0x04;
    pub const TUNE_A: u32 = 0x05;
    pub const SAW_A: u32 = 0x06;
    pub const PULSE_A: u32 = 0x07;
    pub const PW_A: u32 = 0x08;
    pub const SYNC: u32 = 0x09;
    pub const MIX_A: u32 = 0x0a;
    pub const MIX_B: u32 = 0x0b;
    pub const NOISE: u32 = 0x0c;
    pub const EXTERNAL: u32 = 0x0d;
    pub const CUTOFF: u32 = 0x0e;
    pub const RESONANCE: u32 = 0x0f;
    pub const ENV_AMOUNT: u32 = 0x10;
    pub const KBD: u32 = 0x11;
    pub const F_ATTACK: u32 = 0x12;
    pub const F_DECAY: u32 = 0x13;
    pub const F_SUSTAIN: u32 = 0x14;
    pub const F_RELEASE: u32 = 0x15;
    pub const LFO_MODE: u32 = 0x16;
    pub const LFO_RATE: u32 = 0x17;
    pub const LFO_SAW: u32 = 0x18;
    pub const LFO_TRI: u32 = 0x19;
    pub const LFO_PULSE: u32 = 0x1a;
    pub const TUNE_B: u32 = 0x1b;
    pub const FINE_B: u32 = 0x1c;
    pub const SAW_B: u32 = 0x1d;
    pub const TRI_B: u32 = 0x1e;
    pub const PULSE_B: u32 = 0x1f;
    pub const PW_B: u32 = 0x20;
    pub const LOW_B: u32 = 0x21;
    pub const KBD_B: u32 = 0x22;
    pub const MOD_AMOUNT: u32 = 0x23;
    pub const LFO_NOISE: u32 = 0x24;
    pub const WM_FREQ_A: u32 = 0x25;
    pub const WM_FREQ_B: u32 = 0x26;
    pub const WM_PW_A: u32 = 0x27;
    pub const WM_PW_B: u32 = 0x28;
    pub const WM_FILTER: u32 = 0x29;
    pub const GLIDE: u32 = 0x2a;
    pub const VOICE_MODE: u32 = 0x2b;
    pub const A_ATTACK: u32 = 0x2c;
    pub const A_DECAY: u32 = 0x2d;
    pub const A_SUSTAIN: u32 = 0x2e;
    pub const A_RELEASE: u32 = 0x2f;
    pub const RELEASE: u32 = 0x30;
    pub const VELOCITY: u32 = 0x31;
    pub const VOICE_TRIG: u32 = 0x32;
    pub const HOLD: u32 = 0x33;
    pub const FILTER_MODE: u32 = 0x34;
    pub const INVERT: u32 = 0x35;
    pub const DETUNE: u32 = 0x36;
    pub const MASTER_TUNE: u32 = 0x37;
    pub const VOLUME: u32 = 0x38;
    pub const WHEEL: u32 = 0x39;
    pub const BEND: u32 = 0x3a;
    pub const BEND_RANGE: u32 = 0x3b;
    pub const FILTER_VERSION: u32 = 0x3c;
    pub const OSC_VERSION: u32 = 0x3d;
}

/// `x` unless `|x| < 1e-12` (`f32` compare), as `smoothValue` drops tiny steps.
#[inline]
fn tiny0(x: f32) -> f32 {
    if x.abs() < 1e-12 { 0.0 } else { x }
}

/// 1 − `x·inv_sr`, or 0 when that would go negative (decay/release factor).
#[inline]
fn factor(x: f32) -> f32 {
    if x > 1.0 { 0.0 } else { 1.0 - x }
}

impl P5 {
    /// Quantize `v` the way the parameter's type does (switches to 0/1, three-way to
    /// 0/0.5/1), as `setParameterValue` and the Maschine glue do before using it.
    pub fn quantize(id: u32, v: f32) -> f32 {
        match PARAM_TYPE.get(id as usize).copied().unwrap_or(1) {
            0 => {
                if v < 0.5 {
                    0.0
                } else {
                    1.0
                }
            }
            2 => {
                let d = v as f64;
                if d >= 0.666_66 {
                    1.0
                } else if d < 0.333_33 {
                    0.0
                } else {
                    0.5
                }
            }
            _ => v,
        }
    }

    /// `P5::setParameterValue(id, v)`.
    pub fn set_parameter_value(&mut self, pid: u32, value: f32) {
        if pid == 0x3e {
            return;
        }
        let v = P5::quantize(pid, value);
        if let Some(p) = self.program.get_mut(pid as usize) {
            *p = clamp01(v);
        }
        if pid > 0x3b {
            return;
        }
        match pid {
            id::PM_ENV => {
                let x = v.mul_add(4.7, 0.4);
                self.pm_env = x.mul_add(x, -0.16);
            }
            id::PM_OSC_B => self.pm_osc_b = v * 3.09,
            id::PM_FREQ_A => self.pm_freq_a = if v == 0.0 { 0.0 } else { 0.4 },
            id::PM_PW_A => self.pm_pw_a = if v == 0.0 { 0.0 } else { -1.0 },
            id::PM_FILTER => self.pm_cut = if v == 0.0 { 0.0 } else { 3.8 },
            id::TUNE_A => {
                // In f64: ⌊v·47.625 + 0.4⌋ − 24 + 12.
                let n = (v as f64).mul_add(47.625, 0.4) as i32;
                self.semis_a = (n - 24) as f32 + 12.0;
                for i in 0..SLOTS {
                    self.retune_a(i);
                }
            }
            id::SAW_A => {
                self.saw_on_a = if v == 0.0 { 0.0 } else { 1.0 };
                self.saw_a = self.mix_a * self.saw_on_a;
            }
            id::PULSE_A => self.pulse_on_a = (v != 0.0) as i32,
            id::PW_A => {
                self.pw_a_target = v.mul_add(2.0, -1.0);
                self.smooth_value();
            }
            id::SYNC => self.sync = v as i32,
            id::MIX_A => {
                self.mix_a_target = v;
                self.smooth_value();
            }
            id::MIX_B => {
                self.mix_b_target = v + v;
                self.smooth_value();
            }
            id::NOISE => self.noise_mix = v * 1.75,
            id::EXTERNAL => self.ext_mix = v * 25.0,
            id::CUTOFF => {
                self.cutoff_semis = (v * 120.0) as i32 as f32 + -39.0;
                self.smooth_value();
            }
            id::RESONANCE => {
                self.reso = v;
                self.set_damping(reso_curve(v));
            }
            id::ENV_AMOUNT => {
                self.env_amt_target = v * (self.invert * 192.0);
                self.smooth_value();
            }
            id::KBD => {
                self.kbd_target = v;
                self.smooth_value();
            }
            id::F_ATTACK => self.flt_attack = self.env_time(v, 90.7, -5.0) * self.inv_sr,
            id::F_DECAY => {
                self.flt_decay = factor(self.env_time(v, 85.0, -3.0) * self.inv_sr);
                let c = self.flt_decay;
                for voice in self.v.iter_mut() {
                    if voice.flt.stage == 1 {
                        voice.flt.coef = c;
                    }
                }
            }
            id::F_SUSTAIN => {
                let s = v * 0.98;
                self.flt_sustain = s;
                for voice in self.v.iter_mut() {
                    if voice.flt.stage == 1 {
                        voice.flt.floor = s * voice.flt.target;
                        voice.flt.rem = voice.flt.value - voice.flt.floor;
                    }
                }
            }
            id::F_RELEASE => {
                self.flt_release_raw = v.mul_add(92.7, -10.0);
                self.update_flt_release();
            }
            id::LFO_MODE => {
                if v <= 0.5 && self.lfo_synced != 0 {
                    self.lfo_rate = self.lfo_mult + self.lfo_mult;
                }
                self.lfo_synced = (v > 0.5) as i32;
            }
            id::LFO_RATE => {
                let x = v.mul_add(9.725, -5.5);
                let fl = x.floor();
                let m = (1i32.wrapping_shl((fl as i32).wrapping_add(6) as u32)) as f32 * 0.015625;
                let frac = x - fl as i32 as f32;
                self.lfo_mult = if frac as f64 > 0.58 { m * 1.5 } else { m };
                if self.lfo_synced == 0 {
                    self.lfo_rate = self.lookup(v.mul_add(116.7, -92.7).mul_add(5.0, 1381.8816));
                }
            }
            id::LFO_SAW => self.lfo_shape(0, v != 0.0),
            id::LFO_TRI => self.lfo_shape(1, v != 0.0),
            id::LFO_PULSE => self.lfo_shape(2, v != 0.0),
            id::TUNE_B => {
                let raw = v * 47.625;
                self.tune_b_raw = raw;
                self.semis_b = self.semis_from(raw);
                self.retune_b_all();
            }
            id::FINE_B => {
                self.fine_b = v.mul_add(0.935, 0.015);
                self.retune_b_all();
            }
            id::SAW_B => self.saw_b = if v == 0.0 { 0.0 } else { 0.5 },
            id::TRI_B => self.tri_on_b = (v != 0.0) as i32,
            id::PULSE_B => self.pulse_on_b = (v != 0.0) as i32,
            id::PW_B => {
                self.pw_b_target = v.mul_add(2.0, -1.0);
                self.smooth_value();
            }
            id::LOW_B => {
                self.low_b = if v != 0.0 { -91.3 } else { 0.0 };
                self.retune_b_all();
            }
            id::KBD_B => {
                self.kbd_b = if v != 0.0 { 1.0 } else { 0.0 };
                self.semis_b = self.semis_from(self.tune_b_raw);
                self.retune_b_all();
            }
            id::MOD_AMOUNT => {
                self.mod_amount_target = v;
                self.smooth_value();
            }
            id::LFO_NOISE => self.lfo_noise = v,
            id::WM_FREQ_A => self.wm_freq_a = if v == 0.0 { 0.0 } else { 0.4 },
            id::WM_FREQ_B => self.wm_freq_b = if v == 0.0 { 0.0 } else { 0.4 },
            id::WM_PW_A => self.wm_pw_a = if v == 0.0 { 0.0 } else { 1.0 },
            id::WM_PW_B => self.wm_pw_b = if v == 0.0 { 0.0 } else { 1.0 },
            id::WM_FILTER => self.wm_cut = if v == 0.0 { 0.0 } else { 60.0 },
            id::GLIDE => {
                self.glide_rate = self.lookup(v.mul_add(-72.0, 80.0).mul_add(9.965784, 1200.0));
                if self.unison != 0 {
                    self.glide_step = self.glide_step_for(self.glide_target);
                }
            }
            id::VOICE_MODE => self.set_voice_mode(v),
            id::A_ATTACK => self.amp_attack = self.env_time(v, 90.7, -5.0) * self.inv_sr,
            id::A_DECAY => {
                self.amp_decay = factor(self.env_time(v, 85.0, -3.0) * self.inv_sr);
                let c = self.amp_decay;
                for voice in self.v.iter_mut() {
                    if voice.amp.stage == 1 {
                        voice.amp.coef = c;
                    }
                }
            }
            id::A_SUSTAIN => {
                let s = v * 0.95;
                self.amp_sustain = s;
                for voice in self.v.iter_mut() {
                    if voice.amp.stage == 1 {
                        voice.amp.floor = (voice.amp.target + 0.02).mul_add(s, -0.02);
                        voice.amp.rem = voice.amp.value - voice.amp.floor;
                    }
                }
            }
            id::A_RELEASE => {
                self.amp_release_raw = v.mul_add(92.0, -10.0);
                self.update_amp_release();
            }
            id::RELEASE => {
                self.release_on = if v != 0.0 { 1.0 } else { 0.0 };
                self.update_amp_release();
                self.update_flt_release();
            }
            id::VELOCITY => {
                (self.vel_amp, self.vel_flt) = if v == 0.0 { (0.0, 0.0) } else { (0.8, 0.55) };
            }
            id::VOICE_TRIG => self.voice_trig = v as i32,
            id::HOLD => {
                self.hold = v as i32;
                if self.hold == 0 {
                    self.sustained_notes_off();
                } else if self.notes[..self.voices as usize].iter().any(|&n| n > 0) {
                    self.sustain = true;
                } else {
                    self.latch_note();
                }
            }
            id::FILTER_MODE => {
                let (mode, sign) = if v < 0.33 {
                    (0, 1.0)
                } else if v < 0.66 {
                    (1, -1.0)
                } else {
                    (2, 1.0)
                };
                self.mode = mode;
                self.fb_sign = sign;
                self.set_damping(reso_curve(self.reso));
            }
            id::INVERT => {
                self.invert = if v != 0.0 { -1.0 } else { 1.0 };
                let a = self.env_amt_target;
                let mag = if a > 0.0 { a } else { -a };
                self.env_amt_target = mag * self.invert;
                self.smooth_value();
            }
            id::DETUNE => self.set_detune(v),
            id::MASTER_TUNE => {
                self.master = v.mul_add(1.4, -0.7);
                self.tune = self.master + self.bend;
                for i in 0..SLOTS {
                    self.retune_a(i);
                    self.retune_b(i);
                }
            }
            id::VOLUME => {
                self.vol_target = v * 0.224;
                self.smoother();
            }
            id::WHEEL => {
                let t = v * 5.0;
                self.wheel_target = t;
                let step = (t - self.wheel) / (self.sr * 0.02);
                if step.abs() < 1e-5 {
                    self.wheel = t;
                    self.wheel_step = 0.0;
                } else {
                    self.wheel_step = step;
                }
            }
            id::BEND => {
                self.bend_value = v;
                self.retarget_bend();
            }
            id::BEND_RANGE => {
                self.bend_range = v.mul_add(24.0, 0.5) as i32 as f32;
                self.retarget_bend();
            }
            _ => {}
        }
    }

    /// Load a whole program (`P5::setProgram`): every parameter in id order, then the
    /// bend wheel centred.
    pub fn set_program(&mut self, values: &[f32; 62], voices: u32) {
        self.voices = voices;
        self.program = *values;
        for pid in 0..0x3e {
            self.set_parameter_value(pid, self.program[pid as usize]);
        }
        self.reset_bend();
    }

    /// Osc B semitones from its knob (×47.625) and the KBD switch: the knob spans 4
    /// octaves with KBD on, 8 without, and the base moves from 23 to −12.
    fn semis_from(&self, raw: f32) -> f32 {
        let k = self.kbd_b;
        k.mul_add(-35.0, 23.0) + ((raw + 0.4) * (2.0 - k)) as i32 as f32
    }

    /// Voice `i`'s Osc A pitch offset and frequency.
    pub(crate) fn retune_a(&mut self, i: usize) {
        let off = (self.semis_a + self.v[i].det_a) + self.tune;
        self.v[i].off_a = off;
        self.v[i].freq_a = self.hz(self.v[i].pitch + off);
    }

    /// Voice `i`'s Osc B pitch offset and frequency (KBD scales how the note moves it).
    pub(crate) fn retune_b(&mut self, i: usize) {
        let off = (((self.semis_b + self.low_b) + self.fine_b) + self.v[i].det_b) + self.tune;
        self.v[i].off_b = off;
        self.v[i].freq_b = self.hz(self.kbd_b.mul_add(self.v[i].pitch, off));
    }

    fn retune_b_all(&mut self) {
        for i in 0..SLOTS {
            self.retune_b(i);
        }
    }

    /// Both oscillator frequencies of voice `i` from its pitch (offsets unchanged).
    pub(crate) fn refresh_freqs(&mut self, i: usize) {
        let p = self.v[i].pitch;
        self.v[i].freq_a = self.hz(p + self.v[i].off_a);
        self.v[i].freq_b = self.hz(self.kbd_b.mul_add(p, self.v[i].off_b));
    }

    /// Resonance curve → both stages' damping (`2 − 2g`, `2 − 2·0.8g`), then smoothing.
    fn set_damping(&mut self, g: f32) {
        self.damp1 = g.mul_add(-2.0, 2.0);
        self.damp2 = ((g as f64 * 0.8) as f32).mul_add(-2.0, 2.0);
        self.smooth_value();
    }

    fn update_amp_release(&mut self) {
        let x = self.lookup((self.amp_release_raw * self.release_on).mul_add(-9.965784, 1797.947));
        self.amp_release = factor(x * self.inv_sr);
        let c = self.amp_release;
        for voice in self.v.iter_mut() {
            if voice.amp.stage == 2 {
                voice.amp.coef = c;
            }
        }
    }

    fn update_flt_release(&mut self) {
        let x = self.lookup((self.flt_release_raw * self.release_on).mul_add(-9.965784, 1797.947));
        self.flt_release = factor(x * self.inv_sr);
        let c = self.flt_release;
        for voice in self.v.iter_mut() {
            if voice.flt.stage == 2 {
                voice.flt.coef = c;
            }
        }
    }

    /// LFO shape switches: one or two shapes add; all three turn into sample & hold of
    /// the noise; none gives a constant 0.4.
    fn lfo_shape(&mut self, which: usize, on: bool) {
        fn level(p: &mut P5, k: usize) -> &mut f32 {
            match k {
                0 => &mut p.lfo_saw,
                1 => &mut p.lfo_tri,
                _ => &mut p.lfo_pulse,
            }
        }
        let (a, b) = ((which + 1) % 3, (which + 2) % 3);
        if on {
            self.lfo_none = 0.0;
            if self.lfo_sh_on != 0.0 {
                return;
            }
            *level(self, which) = 0.4;
            if *level(self, a) != 0.0 && *level(self, b) != 0.0 {
                self.lfo_sh_on = 1.0;
                self.lfo_saw = 0.0;
                self.lfo_tri = 0.0;
                self.lfo_pulse = 0.0;
            }
        } else {
            *level(self, which) = 0.0;
            if self.lfo_sh_on != 0.0 {
                self.lfo_sh_on = 0.0;
                *level(self, a) = 0.4;
                *level(self, b) = 0.4;
                return;
            }
            if *level(self, a) == 0.0 && *level(self, b) == 0.0 {
                self.lfo_none = 0.4;
            }
        }
    }

    /// Unison glide step toward `target` (0 when there).
    pub(crate) fn glide_step_for(&self, target: f32) -> f32 {
        if target == self.glide {
            0.0
        } else if target <= self.glide {
            self.inv_sr * -self.glide_rate
        } else {
            self.glide_rate * self.inv_sr
        }
    }

    /// Voice Mode (`ParameterId` 0x2b): < 0.33 poly, < 0.66 unison, else unison with
    /// legato.
    fn set_voice_mode(&mut self, v: f32) {
        self.unison = (v >= 0.33) as i32;
        if v < 0.33 {
            self.glide_step = 0.0;
            for i in 0..SLOTS {
                if self.v[i].note >= 0.0 {
                    let p = note_pitch(self.v[i].note as i32);
                    self.v[i].pitch = p;
                    self.v[i].freq_a = self.hz(p + self.v[i].off_a);
                    self.v[i].freq_b = self.hz(self.kbd_b.mul_add(p, self.v[i].off_b));
                }
                if self.v[i].velocity <= 0.0 {
                    self.release_voice(i);
                }
            }
            self.uni_held = 0;
        } else {
            self.uni_retrig = (v < 0.66) as i32;
            let target = note_pitch(self.uni_note as i32);
            self.glide_target = target;
            if target == self.glide {
                self.glide_step = 0.0;
                for i in 0..self.voices as usize {
                    self.v[i].pitch = target + self.v[i].spread;
                    self.refresh_freqs(i);
                }
            } else {
                self.glide_step = self.glide_step_for(target);
            }
            if self.hold != 0 {
                self.sustained_notes_off();
                self.latch_note();
            }
        }
    }

    /// Both envelopes of voice `i` into release.
    pub(crate) fn release_voice(&mut self, i: usize) {
        let (ar, fr) = (self.amp_release, self.flt_release);
        let v = &mut self.v[i];
        v.amp.stage = 2;
        v.amp.coef = ar;
        v.amp.floor = -0.02;
        v.amp.rem = v.amp.value + 0.02;
        v.flt.stage = 2;
        v.flt.coef = fr;
        v.flt.floor = 0.0;
        v.flt.rem = v.flt.value;
    }

    /// Hold switched on with no key down: play note 60 (velocity 64) and latch it.
    fn latch_note(&mut self) {
        self.note_on(60, 64);
        self.sustain = true;
        self.note_off(60);
    }

    /// Detune: re-rolls every voice's random offsets from a fixed seed (so the same
    /// amount always gives the same voices), and with Detune at 0 spreads unison
    /// voices evenly over ±0.035 semitones.
    fn set_detune(&mut self, v: f32) {
        self.detune = if v > 0.0 { self.lookup((v * 5.0).mul_add(86.5617, 1200.0)) * 0.19205 } else { 0.0 };
        let d = self.detune;
        let mut s: u32 = 1_234_567_890;
        let rnd = |s: &mut u32| -> f32 {
            *s = xorshift(*s);
            ((*s & 0xffff) as f32).mul_add(3.0518e-05, -1.0)
        };
        for i in 0..SLOTS {
            let a = rnd(&mut s) * (d * 0.01);
            self.v[i].det_a = a;
            self.v[i].off_a = (self.semis_a + a) + self.tune;
            let b = rnd(&mut s) * (d * 0.01);
            self.v[i].det_b = b;
            self.v[i].off_b = (((self.semis_b + self.low_b) + self.fine_b) + b) + self.tune;
            self.v[i].det_cut = rnd(&mut s) * (d * 0.5);
            self.smooth_value();
            self.v[i].drift = (d * 0.1).mul_add(rnd(&mut s), 1.0);
            let r = rnd(&mut s);
            self.v[i].spread = if d == 0.0 {
                let k = if i & 1 == 1 { ((i + 1) >> 1) as i32 } else { -((i >> 1) as i32) };
                (0.07 / self.voices as f32) * k as f32
            } else {
                r * (d * 0.035)
            };
            self.update_pitch();
        }
    }

    /// `P5::updatePitch`: every voice's pitch and oscillator frequencies (poly: from
    /// its note; unison: toward the unison note with glide).
    pub(crate) fn update_pitch(&mut self) {
        if self.unison == 0 {
            for i in 0..SLOTS {
                if self.v[i].note >= 0.0 {
                    self.v[i].pitch = note_pitch(self.v[i].note as i32);
                }
                self.refresh_freqs(i);
            }
        } else {
            let target = note_pitch(self.uni_note as i32);
            self.glide_target = target;
            if target == self.glide {
                self.glide_step = 0.0;
                for i in 0..self.voices as usize {
                    self.v[i].pitch = target + self.v[i].spread;
                    self.refresh_freqs(i);
                }
            } else {
                self.glide_step = self.glide_step_for(target);
            }
        }
    }

    /// `P5::smoothValue`: restart the linear smoothing of the mixer, pulse widths,
    /// cutoff, envelope amount, keyboard tracking and modulation amount.
    pub(crate) fn smooth_value(&mut self) {
        self.smooth_left = self.smooth_len;
        let k = 1.0 / self.smooth_len as f32;
        self.mix_a_step = tiny0(k * (self.mix_a_target - self.mix_a));
        self.mix_b_step = tiny0(k * (self.mix_b_target - self.mix_b));
        self.pw_a_step = tiny0(k * (self.pw_a_target - self.pw_a));
        self.pw_b_step = tiny0(k * (self.pw_b_target - self.pw_b));
        let target = self.reso.mul_add(4.5, (1.0 - self.kbd_target).mul_add(35.0, self.cutoff_semis));
        self.cutoff_step = tiny0(k * (target - self.cutoff));
        self.env_amt_step = tiny0(k * (self.env_amt_target - self.env_amt));
        self.kbd_step = tiny0(k * (self.kbd_target - self.kbd));
        self.mod_amount_step = tiny0(k * (self.mod_amount_target - self.mod_amount));
    }

    /// `P5::smoother`: volume ramps to its target over ⌊0.02·sr⌋ samples.
    pub(crate) fn smoother(&mut self) {
        let n = (self.sr as f64 * 0.02) as i32;
        self.vol_left = n;
        self.vol_step = clip_step((self.vol_target - self.vol) * (1.0 / n as f32));
    }

    /// Pitch bend target from the wheel and the range; the bend glides there over
    /// 0.02·sr samples.
    pub(crate) fn retarget_bend(&mut self) {
        let t = (self.bend_range + self.bend_range) * (self.bend_value + -0.5);
        self.bend_target = t;
        let step = (t - self.bend) / (self.sr * 0.02);
        if step.abs() < 1e-5 {
            self.bend = t;
            self.bend_step = 0.0;
        } else {
            self.bend_step = step;
        }
    }

    /// The bend wheel back to centre (0.5).
    pub(crate) fn reset_bend(&mut self) {
        self.program[0x3a] = 0.5;
        self.bend_value = 0.5;
        self.retarget_bend();
    }
}

/// The resonance knob → the filter's feedback curve (piecewise linear).
fn reso_curve(r: f32) -> f32 {
    if r < 0.25 {
        r * 2.32
    } else if r < 0.5 {
        r.mul_add(0.52, 0.45)
    } else if r < 0.75 {
        r.mul_add(0.92, 0.25)
    } else {
        r.mul_add(0.036, 0.913)
    }
}

/// Clamp to 0..1 the way `fminnm`/`fmaxnm` do (NaN → 1).
#[inline]
pub(crate) fn clamp01(v: f32) -> f32 {
    if v.is_nan() { 1.0 } else { v.clamp(0.0, 1.0) }
}

/// The note → pitch table (P5 + 0x7294, the identity on 0..127).
#[inline]
pub(crate) fn note_pitch(n: i32) -> f32 {
    if (0..128).contains(&n) { n as f32 } else { 0.0 }
}

/// The shift-register step the noise and Detune use:
/// `x' = (x >> 1) | (((x >> 13) ^ (x >> 5) ^ x) << 31)`.
#[inline]
pub(crate) fn xorshift(x: u32) -> u32 {
    let t = (x >> 13) ^ (x >> 5) ^ x;
    (t << 31) | (x >> 1)
}
