//! `NI::DSP::CymbalBodyCore<Exciter0, UseAir0, FeedbackDamping1>` (docs/synth-cymbal.md §5).
//! Unfinished: only the modal bank is built; `air`, `exciter` and the body core itself are
//! not ported yet (air.rs/exciter.rs/src.rs on disk are partial and not compiled).
#![allow(dead_code)]

#[macro_use]
mod img;
pub(super) mod modal;
