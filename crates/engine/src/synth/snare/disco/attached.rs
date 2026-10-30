//! `NI::DSP::AttachedBodiesCore<AM on, Noise 1, Crispel off, PitchEnv on>` at
//! `DiscoSDCore` +0x42080. Not ported yet: silent.

pub(super) struct Attached {}

impl Attached {
    pub(super) fn new() -> Attached {
        Attached {}
    }
    pub(super) fn set_sample_rate(&mut self, _sr: f32, _block: i32) {}
    /// The `DiscoSDCore::onReset` lines for +0x42080..0x42668.
    pub(super) fn reset(&mut self) {}
    pub(super) fn note_on(&mut self, _vel: f32) {}
    pub(super) fn process(&mut self) -> f32 {
        0.0
    }
    /// +0x421a8 != 0 || +0x42228 != 0.
    pub(super) fn active(&self) -> bool {
        false
    }
    pub(super) fn set_crispel_decay2(&mut self, _v: f32) {}
    pub(super) fn set_noise_decay2(&mut self, _v: f32) {}
    /// +0x4265c = velocity target 7 (caller checked ±100).
    pub(super) fn set_out7(&mut self, _v: f32) {}
    /// `DiscoSDCore::setNoteOn`'s inline delay: +0x42544 = ms, +0x42558 = samples.
    pub(super) fn set_delay_from_attack(&mut self, _ms: f32) {}
}
