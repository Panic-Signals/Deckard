//! Time-stretching and pitch-shifting for the Audio plug-in's Stretch and Formant
//! engines (docs/audio-plugin.md): Signalsmith Stretch 1.3.1 (MIT, Signalsmith Audio;
//! `vendor/`, with its licences), compiled from its header and driven through
//! `src/shim.cpp`. Streaming: feed input frames, get output frames; a different count
//! of each stretches time, the transpose factor moves pitch independently, and the
//! formant factor (compensating the transpose) keeps a voice's character.

use std::ffi::{c_int, c_void};

unsafe extern "C" {
    fn deckard_stretch_new(channels: c_int, sample_rate: f32, cheaper: c_int) -> *mut c_void;
    fn deckard_stretch_free(s: *mut c_void);
    fn deckard_stretch_reset(s: *mut c_void);
    fn deckard_stretch_input_latency(s: *mut c_void) -> c_int;
    fn deckard_stretch_output_latency(s: *mut c_void) -> c_int;
    fn deckard_stretch_transpose(s: *mut c_void, factor: f32);
    fn deckard_stretch_formant(s: *mut c_void, factor: f32, compensate: c_int);
    fn deckard_stretch_seek(s: *mut c_void, inputs: *const *const f32, frames: c_int, rate: f64);
    fn deckard_stretch_process(s: *mut c_void, inputs: *const *const f32, in_frames: c_int, outputs: *const *mut f32, out_frames: c_int);
}

/// A stereo stretcher.
pub struct Stretch {
    raw: *mut c_void,
}

// The instance is used by one thread at a time (owned, `&mut` to process).
unsafe impl Send for Stretch {}

impl Stretch {
    /// At Signalsmith's default quality for `sample_rate`; `cheaper` trades some for
    /// less CPU.
    pub fn new(sample_rate: f32, cheaper: bool) -> Stretch {
        // SAFETY: the shim allocates an instance, freed in Drop.
        Stretch { raw: unsafe { deckard_stretch_new(2, sample_rate, c_int::from(cheaper)) } }
    }

    pub fn reset(&mut self) {
        // SAFETY: `raw` is a live instance.
        unsafe { deckard_stretch_reset(self.raw) }
    }

    /// Input frames to feed ahead of the processing time, and output frames it comes
    /// out behind it.
    pub fn latency(&self) -> (usize, usize) {
        // SAFETY: `raw` is a live instance.
        unsafe { (deckard_stretch_input_latency(self.raw).max(0) as usize, deckard_stretch_output_latency(self.raw).max(0) as usize) }
    }

    /// Pitch, as a frequency factor (2: an octave up).
    pub fn set_transpose(&mut self, factor: f32) {
        // SAFETY: `raw` is a live instance.
        unsafe { deckard_stretch_transpose(self.raw, factor) }
    }

    /// Formants, as a frequency factor; `compensate`: relative to the transpose (1 with
    /// it keeps them where they were).
    pub fn set_formant(&mut self, factor: f32, compensate: bool) {
        // SAFETY: `raw` is a live instance.
        unsafe { deckard_stretch_formant(self.raw, factor, c_int::from(compensate)) }
    }

    /// Pre-roll: input before where it starts (ideally a block and an interval of it),
    /// without moving the stream on; `rate` the speed it will play at (input per
    /// output frame).
    pub fn seek(&mut self, left: &[f32], right: &[f32], rate: f64) {
        let n = left.len().min(right.len());
        let ins = [left.as_ptr(), right.as_ptr()];
        // SAFETY: two channels of `n` readable frames.
        unsafe { deckard_stretch_seek(self.raw, ins.as_ptr(), n as c_int, rate) }
    }

    /// Stretch `left`/`right` (as much input as there is) into all of `out_l`/`out_r`.
    pub fn process(&mut self, left: &[f32], right: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let n_in = left.len().min(right.len());
        let n_out = out_l.len().min(out_r.len());
        let ins = [left.as_ptr(), right.as_ptr()];
        let outs = [out_l.as_mut_ptr(), out_r.as_mut_ptr()];
        // SAFETY: two channels of `n_in` readable and `n_out` writable frames.
        unsafe { deckard_stretch_process(self.raw, ins.as_ptr(), n_in as c_int, outs.as_ptr(), n_out as c_int) }
    }
}

impl Drop for Stretch {
    fn drop(&mut self) {
        // SAFETY: made by `deckard_stretch_new`, freed once.
        unsafe { deckard_stretch_free(self.raw) }
    }
}

#[cfg(test)]
mod tests {
    use super::Stretch;

    /// Rising zero crossings per second of `x` at `rate`: its frequency, roughly.
    fn freq(x: &[f32], rate: f32) -> f32 {
        let n = x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        n as f32 * rate / x.len() as f32
    }

    /// Half speed keeps a tone's pitch; transposed an octave, it doubles.
    #[test]
    fn stretching_keeps_pitch_and_transposing_moves_it() {
        let rate = 44_100.0;
        let tone: Vec<f32> = (0..rate as usize * 2).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / rate).sin() * 0.5).collect();
        let run = |transpose: f32| {
            let mut s = Stretch::new(rate, false);
            s.set_transpose(transpose);
            let (mut l, mut r) = (vec![0.0; tone.len() * 2], vec![0.0; tone.len() * 2]);
            // Half speed: every input block becomes twice as much output.
            for (k, chunk) in tone.chunks(512).enumerate() {
                let o = k * 1024;
                let (ol, or) = (&mut l[o..o + chunk.len() * 2], &mut r[o..o + chunk.len() * 2]);
                s.process(chunk, chunk, ol, or);
            }
            let (_, out) = s.latency();
            let steady = &l[out + 8_000..l.len() - 8_000];
            (steady.len(), freq(steady, rate))
        };
        let (n, f) = run(1.0);
        assert!(n > 150_000, "twice as long");
        assert!((f - 440.0).abs() < 8.0, "pitch kept: {f}");
        let (_, f) = run(2.0);
        assert!((f - 880.0).abs() < 16.0, "an octave up: {f}");
    }
}
