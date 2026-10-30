//! The drum cores' parameter smoother (`processControlRate`, e.g. 0x1018d5a30 and the
//! shared ramp at 0x1019685bc): `N` control values ramp linearly to their targets over
//! ~20 ms (`round(0.02·sr/128)` control blocks), one step per sample.

#[derive(Clone, Debug)]
pub struct Smooth<const N: usize> {
    pub cur: [f32; N],
    pub step: [f32; N],
    pub target: [f32; N],
    /// Targets changed since the last control block.
    pub dirty: bool,
    /// A ramp is running (the `processAudioImpl<true, …>` variants).
    pub ramping: bool,
    /// Control blocks left in the ramp; its length; 1/(its length in samples).
    left: i32,
    ramp: i32,
    ramp_inv: f32,
    /// 0 idle, 1 changed, 2 ramp started, 3 ramping, 4 ended.
    pub state: u32,
}

impl<const N: usize> Smooth<N> {
    pub fn new() -> Self {
        Smooth { cur: [0.0; N], step: [0.0; N], target: [0.0; N], dirty: true, ramping: false, left: 0, ramp: 1, ramp_inv: 1.0, state: 1 }
    }

    /// The ramp length for `sr` and the control block.
    pub fn set_rate(&mut self, sr: f32, block: i32) {
        let r = ((sr * 0.02) / block as f32 + 0.5) as i32;
        self.ramp = if r > 1 { r } else { 1 };
        self.ramp_inv = 1.0 / (self.ramp * block) as f32;
    }

    /// A target changed.
    pub fn touch(&mut self) {
        self.dirty = true;
        self.state = 1;
    }

    /// Advance the current values one sample (while ramping).
    #[inline]
    pub fn tick(&mut self) {
        for i in 0..N {
            self.cur[i] += self.step[i];
        }
    }

    /// Whether value `i` should be re-derived this sample: always on the first chunk
    /// after a note, else only while it ramps.
    #[inline]
    pub fn check(&self, first: bool, i: usize) -> bool {
        first || (self.ramping && self.step[i] != 0.0)
    }

    /// `processControlRate()`.
    pub fn control(&mut self) {
        if !self.dirty {
            self.left -= 1;
            if self.left == 0 {
                self.cur = self.target;
                self.step = [0.0; N];
                self.ramping = false;
                self.state = 4;
            } else if self.state == 2 {
                self.state = 3;
            } else if self.state == 4 {
                self.state = 0;
            }
            return;
        }
        self.dirty = false;
        self.state = 2;
        let mut any = false;
        for i in 0..N {
            let d = (self.target[i] - self.cur[i]) * self.ramp_inv;
            self.step[i] = d;
            if d * d < 1e-15 {
                self.step[i] = 0.0;
                self.cur[i] = self.target[i];
            } else {
                any = true;
            }
        }
        if any {
            self.left = self.ramp;
            self.ramping = true;
        } else {
            self.state = 4;
        }
    }
}
