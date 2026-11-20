//! A single reader taps an effect's main stereo input and output. The audio
//! thread fills a preallocated ring and publishes overlapping windows.
use std::sync::{Arc, Mutex};

pub const FRAMES: usize = 4096;
const HOP: usize = 1024;
type Pair = [[f32; 4]];
type Mailbox = Arc<Mutex<(Box<Pair>, bool)>>;

fn buffer() -> Box<Pair> {
    vec![[0.0; 4]; FRAMES].into_boxed_slice()
}

pub struct Reader {
    mailbox: Mailbox,
    window: Box<Pair>,
}

pub struct Tap {
    mailbox: Mailbox,
    window: Box<Pair>,
    cursor: usize,
    until_publish: usize,
}

pub fn channel() -> (Tap, Reader) {
    let mailbox = Arc::new(Mutex::new((buffer(), false)));
    (Tap { mailbox: mailbox.clone(), window: buffer(), cursor: 0, until_publish: FRAMES }, Reader { mailbox, window: buffer() })
}

impl Reader {
    /// The newest complete window, in left/right input, left/right output order.
    pub fn receive(&mut self) -> Option<&[[f32; 4]]> {
        let mut ready = self.mailbox.try_lock().ok()?;
        if !ready.1 {
            return None;
        }
        std::mem::swap(&mut self.window, &mut ready.0);
        ready.1 = false;
        Some(&self.window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishes_the_latest_complete_stereo_input_and_output_window() {
        let (mut tap, mut reader) = channel();
        let push = |tap: &mut Tap, n: usize, x: f32| {
            tap.push((&vec![x; n], &vec![x + 1.0; n]), (&vec![x + 2.0; n], &vec![x + 3.0; n]));
        };
        push(&mut tap, FRAMES - 1, 1.0);
        assert!(reader.receive().is_none());
        push(&mut tap, 2, 2.0);
        let first = reader.receive().unwrap();
        assert_eq!(first[0], [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(first[FRAMES - 1], [2.0, 3.0, 4.0, 5.0]);
        push(&mut tap, HOP - 2, 3.0);
        assert!(reader.receive().is_none());
        push(&mut tap, 1, 4.0);
        let next = reader.receive().unwrap();
        assert_eq!(next[0], [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(next[FRAMES - HOP - 1], [2.0, 3.0, 4.0, 5.0]);
        assert_eq!(next[FRAMES - HOP], [2.0, 3.0, 4.0, 5.0]);
        assert_eq!(next[FRAMES - 1], [4.0, 5.0, 6.0, 7.0]);
        push(&mut tap, FRAMES, 5.0);
        assert!(reader.receive().unwrap().iter().all(|f| *f == [5.0, 6.0, 7.0, 8.0]));
        assert!(reader.receive().is_none());
    }
}

impl Tap {
    pub fn has_reader(&self) -> bool {
        Arc::strong_count(&self.mailbox) > 1
    }

    pub fn push(&mut self, (il, ir): (&[f32], &[f32]), (ol, or): (&[f32], &[f32])) {
        // The reader drops its reference when focus leaves this effect.
        if !self.has_reader() {
            return;
        }
        let n = il.len().min(ir.len()).min(ol.len()).min(or.len());
        for i in 0..n {
            self.window[self.cursor] = [il[i], ir[i], ol[i], or[i]];
            self.cursor = (self.cursor + 1) % FRAMES;
            self.until_publish -= 1;
            if self.until_publish == 0 {
                if let Ok(mut ready) = self.mailbox.try_lock() {
                    let (tail, head) = ready.0.split_at_mut(FRAMES - self.cursor);
                    tail.copy_from_slice(&self.window[self.cursor..]);
                    head.copy_from_slice(&self.window[..self.cursor]);
                    ready.1 = true;
                }
                self.until_publish = HOP;
            }
        }
    }
}
