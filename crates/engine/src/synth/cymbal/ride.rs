//! The Cymbal's Ride engine: DECKARD's own design on
//! [`modal_lite`](crate::synth::modal_lite), tuned against Maschine's renders (not a
//! port; docs/synth-cymbal.md §5). See [`super::metal`].

use super::metal::{Kind, Metal};

pub(super) struct Ride;

impl Ride {
    pub(super) fn new() -> Box<Metal> {
        Box::new(Metal::new(Kind::Ride))
    }
}
