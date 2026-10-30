//! The Snare's Chrome engine (`NI::DSP::WhippySDCore` in Maschine): DECKARD's own design, the
//! shared modal snare of [`super::modal`] with Chrome's profile.

use super::modal::{Kind, ModalSnare};

pub(crate) struct Whippy;

impl Whippy {
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(sr: f32) -> ModalSnare {
        ModalSnare::new(Kind::Chrome, sr)
    }
}
