//! Spectrum capture for the focused internal EQ.
use std::sync::Mutex;

use deckard_model::extra_out::Owner;
use deckard_plugins::{MAX_BLOCK, spectrum};

#[derive(Default)]
pub struct EqSpectrum(Mutex<Option<Watch>>);

struct Watch {
    owner: Owner,
    tap: spectrum::Tap,
    input_l: Vec<f32>,
    input_r: Vec<f32>,
}

impl EqSpectrum {
    /// Select one slot from the UI. Dropping the reader stops capture.
    pub fn watch(&self, owner: Owner) -> spectrum::Reader {
        let (tap, reader) = spectrum::channel();
        let watch = Watch { owner, tap, input_l: vec![0.0; MAX_BLOCK], input_r: vec![0.0; MAX_BLOCK] };
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(watch);
        reader
    }

    pub(crate) fn process(&self, owner: Owner, l: &mut [f32], r: &mut [f32], run: impl FnOnce(&mut [f32], &mut [f32])) {
        // The audio thread never waits on the UI or another chain.
        if let Ok(mut watch) = self.0.try_lock()
            && let Some(watch) = watch.as_mut().filter(|w| w.owner == owner && w.tap.has_reader())
        {
            let il = &mut watch.input_l[..l.len()];
            let ir = &mut watch.input_r[..r.len()];
            il.copy_from_slice(l);
            ir.copy_from_slice(r);
            run(l, r);
            watch.tap.push((il, ir), (l, r));
            return;
        }
        run(l, r);
    }
}
