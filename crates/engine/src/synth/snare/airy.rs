//! The Snare's Airy engine (`NI::DSP::AirySDCore` in Maschine): DECKARD's own design, the
//! shared modal snare of [`super::modal`] with Airy's profile.

use super::modal::{Kind, ModalSnare};

pub(crate) struct Airy;

impl Airy {
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(sr: f32) -> ModalSnare {
        ModalSnare::new(Kind::Airy, sr)
    }
}
