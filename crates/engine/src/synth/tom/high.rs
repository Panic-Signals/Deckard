//! Tom's High engine: `DSP::HighTomCore` (docs/synth-tom.md §5), run as the translated
//! core in [`super::modal`].

use super::Engine;
use super::modal::Core;

pub struct High {
    c: Core,
}

impl High {
    pub fn new() -> High {
        High { c: Core::high() }
    }
}

impl Engine for High {
    fn set_sample_rate(&mut self, sr: f32) {
        self.c.set_sample_rate(sr);
    }
    fn reset(&mut self) {
        self.c.reset();
    }
    /// `TomProcessor::setParam*` → `HighTomCore::setParameter`: tune 0, decay 1,
    /// tension 2, impact 3, velocity scaling 4 (Bend is not used).
    fn set_shared(&mut self, idx: u32, v: f32, _shared: &[f32; 7]) {
        match idx {
            1 => self.c.set_parameter(0, v),
            2 => self.c.set_parameter(1, v),
            4 => self.c.set_parameter(3, v),
            5 => self.c.set_parameter(2, v),
            6 => self.c.set_parameter(4, v),
            _ => {}
        }
    }
    fn update_global(&mut self, s: &[f32; 7]) {
        self.c.set_parameter(0, s[1]);
        self.c.set_parameter(1, s[2]);
        self.c.set_parameter(3, s[4]);
        self.c.set_parameter(2, s[5]);
        self.c.set_parameter(4, s[6]);
    }
    fn set_core(&mut self, _idx: u32, _v: f32) {}
    fn note_on(&mut self, vel: f32, note: u32, _shared: &[f32; 7]) {
        self.c.note_on(vel, note);
    }
    fn note_off(&mut self, note: u32) {
        self.c.note_off(note);
    }
    fn choke(&mut self) {
        self.c.choke();
    }
    fn control_rate(&mut self) {
        self.c.control_rate();
    }
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.c.process(l, r);
    }
    fn silent(&self) -> bool {
        !self.c.output()
    }
}
