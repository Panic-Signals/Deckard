//! `NI::DSP::BodyTableCore<Exciter double, UseAir on, FeedbackDamping off>` at
//! `DiscoSDCore` +0xa00. Not ported yet: silent.

pub(super) struct Body {}

impl Body {
    pub(super) fn new() -> Box<Body> {
        Box::new(Body {})
    }
    /// `BodyCore::onSetSampleRate`.
    pub(super) fn set_sample_rate(&mut self, _sr: f32, _block: i32) {}
    /// `BodyCore::onReset` (+ the `DiscoSDCore::onReset` lines for +0xa00..0x42080).
    pub(super) fn reset(&mut self) {}
    /// `BodyTableCore::setNoteOn`.
    pub(super) fn note_on(&mut self, _vel: f32) {}
    /// `BodyCore::processSample`: (left, right).
    pub(super) fn process(&mut self) -> (f32, f32) {
        (0.0, 0.0)
    }
    /// `BodyTableCore::processControlRate` (inline at the end of `DiscoSDCore::processControlRate`).
    pub(super) fn control_rate(&mut self) {}
    /// +0x41735.
    pub(super) fn active(&self) -> bool {
        false
    }
    /// `setModulationTunningAmnt` (caller checked −1..1): +0x41b10, flag +0x41734.
    pub(super) fn set_tuning_amount(&mut self, _v: f32) {}
    /// +0x41b08 = semitones (caller checked ±48); `updatePitch` if partials (+0x416dc).
    pub(super) fn set_tune(&mut self, _semi: f32) {}
    /// `setModulationDecMaster`.
    pub(super) fn set_dec_master(&mut self, _v: f32) {}
    /// `setModulationOnePartialMask_Plus_DecayMaster(a, type 0, b)`.
    pub(super) fn set_partial_mask_decay(&mut self, _a: f32, _b: f32) {}
    /// The modal synth's "recalc" flags (partials +0x14, synth +0x35) if voices exist.
    pub(super) fn mark_dirty(&mut self) {}
    /// `setDelayFromAttack(ms)` (caller checked ms ≥ 0).
    pub(super) fn set_delay_from_attack(&mut self, _ms: f32) {}
    /// `setAttackPitch(+0x41730)`.
    pub(super) fn set_attack_pitch(&mut self) {}
    /// `setAttcakEnvDecay(ms)` (caller checked ms ≥ 0).
    pub(super) fn set_attack_env_decay(&mut self, _ms: f32) {}
}
