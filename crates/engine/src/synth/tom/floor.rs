//! Tom's Floor engine: `DSP::FloorTomCore` (docs/synth-tom.md §5), run as the translated
//! core in [`super::modal`].

use super::Engine;
use super::modal::Core;

pub struct Floor {
    c: Core,
}

impl Floor {
    pub fn new() -> Floor {
        Floor { c: Core::floor() }
    }
}

impl Engine for Floor {
    fn set_sample_rate(&mut self, sr: f32) {
        self.c.set_sample_rate(sr);
    }
    fn reset(&mut self) {
        self.c.reset();
    }
    /// `TomProcessor::setParam*` → `FloorTomCore::setParameter`: tune 1, decay 4,
    /// tension 2, impact 3, velocity scaling 7 (Bend is not used).
    fn set_shared(&mut self, idx: u32, v: f32, _shared: &[f32; 7]) {
        match idx {
            1 => self.c.set_parameter(1, v),
            2 => self.c.set_parameter(4, v),
            4 => self.c.set_parameter(3, v),
            5 => self.c.set_parameter(2, v),
            6 => self.c.set_parameter(7, v),
            _ => {}
        }
    }
    fn update_global(&mut self, s: &[f32; 7]) {
        self.c.set_parameter(1, s[1]);
        self.c.set_parameter(4, s[2]);
        self.c.set_parameter(3, s[4]);
        self.c.set_parameter(2, s[5]);
        self.c.set_parameter(7, s[6]);
    }
    /// Flex 0, Air 5, Skin 6 (`setCoreParameter(2, idx)`).
    fn set_core(&mut self, idx: u32, v: f32) {
        self.c.set_parameter(idx, v);
    }
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
