//! The Snare's Vintage engine (`NI::DSP::OldieSDCore` in Maschine): DECKARD's own design, the
//! shared modal snare of [`super::modal`] with Vintage's profile.

use super::modal::{Kind, ModalSnare};

pub(crate) struct Oldie;

impl Oldie {
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(sr: f32) -> ModalSnare {
        ModalSnare::new(Kind::Vintage, sr)
    }
}
