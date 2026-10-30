//! The Snare's Breaker engine (`NI::DSP::BreakerSDCore` in Maschine): DECKARD's own design, the
//! shared modal snare of [`super::modal`] with Breaker's profile.

use super::modal::{Kind, ModalSnare};

pub(crate) struct Breaker;

impl Breaker {
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(sr: f32) -> ModalSnare {
        ModalSnare::new(Kind::Breaker, sr)
    }
}
