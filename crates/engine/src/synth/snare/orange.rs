//! The Snare's Iron engine (`NI::DSP::OrangeSDCore` in Maschine): DECKARD's own design, the
//! shared modal snare of [`super::modal`] with Iron's profile.

use super::modal::{Kind, ModalSnare};

pub(crate) struct Orange;

impl Orange {
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(sr: f32) -> ModalSnare {
        ModalSnare::new(Kind::Iron, sr)
    }
}
