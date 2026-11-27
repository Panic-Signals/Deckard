//! A tap on a plug-in's audio, for something on the UI thread to look at (a scope, a
//! spectrum): the audio thread copies the plug-in's main stereo input and output into a
//! ring made beforehand and, every [`HOP`] frames, publishes the newest [`FRAMES`] of them
//! to a single [`Reader`]. Nothing allocates or waits on the audio thread: a publish the
//! reader is busy with is skipped, and with no reader left the tap does nothing.
use std::sync::{Arc, Mutex};

/// Frames in a window the reader gets.
pub const FRAMES: usize = 4096;
/// Frames between windows.
const HOP: usize = 1024;
/// Frames a live scope shows (about 21 ms at 48 kHz).
const SCOPE_WINDOW: usize = 1024;

/// A window: per frame, input left and right, then output left and right.
type Window = Box<[[f32; 4]]>;
type Mailbox = Arc<Mutex<(Window, bool)>>;

fn window() -> Window {
    vec![[0.0; 4]; FRAMES].into_boxed_slice()
}

/// The audio thread's end, held by the plug-in ([`crate::Plugin::set_tap`]).
pub struct Tap {
    mailbox: Mailbox,
    ring: Window,
    cursor: usize,
    until_publish: usize,
}

/// The UI's end.
pub struct Reader {
    mailbox: Mailbox,
    window: Window,
}

/// A tap and its reader. Make it off the audio thread.
pub fn channel() -> (Tap, Reader) {
    let mailbox = Arc::new(Mutex::new((window(), false)));
    (Tap { mailbox: mailbox.clone(), ring: window(), cursor: 0, until_publish: FRAMES }, Reader { mailbox, window: window() })
}

impl Tap {
    /// Whether anyone still reads it: the reader is dropped when nothing shows it.
    pub fn has_reader(&self) -> bool {
        Arc::strong_count(&self.mailbox) > 1
    }

    /// An effect's input (left, right) and output.
    pub fn push(&mut self, (il, ir): (&[f32], &[f32]), (ol, or): (&[f32], &[f32])) {
        let n = il.len().min(ir.len()).min(ol.len()).min(or.len());
        self.push_with(n, |i| [il[i], ir[i], ol[i], or[i]]);
    }

    /// An instrument's output, with silence for the input it doesn't have.
    pub fn push_output(&mut self, (ol, or): (&[f32], &[f32])) {
        self.push_with(ol.len().min(or.len()), |i| [0.0, 0.0, ol[i], or[i]]);
    }

    fn push_with(&mut self, n: usize, frame: impl Fn(usize) -> [f32; 4]) {
        if !self.has_reader() {
            return;
        }
        for i in 0..n {
            self.ring[self.cursor] = frame(i);
            self.cursor = (self.cursor + 1) % FRAMES;
            self.until_publish -= 1;
            if self.until_publish == 0 {
                if let Ok(mut ready) = self.mailbox.try_lock() {
                    let (tail, head) = ready.0.split_at_mut(FRAMES - self.cursor);
                    tail.copy_from_slice(&self.ring[self.cursor..]);
                    head.copy_from_slice(&self.ring[..self.cursor]);
                    ready.1 = true;
                }
                self.until_publish = HOP;
            }
        }
    }
}

impl Reader {
    /// The newest window published since the last call, oldest frame first: input left
    /// and right, output left and right. `None` when nothing new has come.
    pub fn receive(&mut self) -> Option<&[[f32; 4]]> {
        let mut ready = self.mailbox.try_lock().ok()?;
        if !ready.1 {
            return None;
        }
        std::mem::swap(&mut self.window, &mut ready.0);
        ready.1 = false;
        Some(&self.window)
    }

    /// The newest output as a scope draws it, `columns` wide: a slice of the output
    /// (both channels mixed), from a rising zero crossing so a steady tone stands still,
    /// at its own level. No envelope: a live signal has no single note to follow.
    /// `None` when nothing new has come.
    pub fn scope(&mut self, columns: usize) -> Option<crate::native::Scope> {
        let w = self.receive()?;
        let mono: Vec<f32> = w.iter().map(|f| (f[2] + f[3]) * 0.5).collect();
        // (The trigger is looked for where a whole slice still follows it.)
        let room = mono.len().saturating_sub(SCOPE_WINDOW);
        let at = (1..=room).find(|&i| mono[i - 1] < 0.0 && mono[i] >= 0.0).unwrap_or(room);
        let slice = &mono[at..(at + SCOPE_WINDOW).min(mono.len())];
        let wave = (0..columns)
            .map(|c| {
                let a = c * slice.len() / columns;
                let b = ((c + 1) * slice.len() / columns).max(a + 1).min(slice.len());
                let (lo, hi) = slice[a..b]
                    .iter()
                    .filter(|x| x.is_finite())
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &x| (lo.min(x), hi.max(x)));
                if lo > hi { (0.0, 0.0) } else { (lo.clamp(-1.0, 1.0), hi.clamp(-1.0, 1.0)) }
            })
            .collect();
        Some(crate::native::Scope { wave, envelope: Vec::new() })
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

    #[test]
    fn an_instruments_output_comes_with_a_silent_input() {
        let (mut tap, mut reader) = channel();
        tap.push_output((&vec![0.5; FRAMES], &vec![-0.5; FRAMES]));
        assert!(reader.receive().unwrap().iter().all(|f| *f == [0.0, 0.0, 0.5, -0.5]));
    }

    #[test]
    fn with_its_reader_gone_a_tap_does_nothing() {
        let (mut tap, reader) = channel();
        drop(reader);
        assert!(!tap.has_reader());
        tap.push_output((&vec![0.5; FRAMES], &vec![0.5; FRAMES]));
        assert!(!tap.mailbox.lock().unwrap().1, "nothing published");
    }

    /// A steady sine's scope: triggered on its rising zero crossing, so it starts at the
    /// middle going up; at its own level (0.5), not the preview's gain.
    #[test]
    fn a_live_scope_is_triggered_and_at_the_signals_level() {
        let (mut tap, mut reader) = channel();
        let sine: Vec<f32> = (0..FRAMES).map(|i| 0.5 * (i as f32 * 0.05 + 1.0).sin()).collect();
        tap.push_output((&sine, &sine));
        let s = reader.scope(64).unwrap();
        assert_eq!(s.wave.len(), 64);
        assert!(s.envelope.is_empty());
        let (lo0, hi0) = s.wave[0];
        assert!(lo0.abs() < 0.05 && hi0 > 0.0, "starts at the middle, rising: {:?}", s.wave[0]);
        let top = s.wave.iter().fold(0f32, |m, &(_, hi)| m.max(hi));
        assert!((top - 0.5).abs() < 0.02, "at its own level: {top}");
        assert!(reader.scope(64).is_none(), "nothing new");
    }

    /// Any plug-in feeds its tap as it plays (here an effect, the Tuner, which passes its
    /// input through), and stops when the tap is taken off.
    #[test]
    fn a_plugin_feeds_its_tap_as_it_plays() {
        let class = &crate::tuner::CLASS;
        let id = crate::PluginId::parse(&class.plugin_id(), class.name).unwrap();
        let mut p = crate::Plugin::native(class, id, "Tuner", 48_000.0);
        let (tap, mut reader) = channel();
        p.set_tap(Some(tap));
        let t = crate::Transport::default();
        let input = vec![0.25f32; crate::MAX_BLOCK];
        for _ in 0..FRAMES.div_ceil(crate::MAX_BLOCK) {
            p.process_input(crate::MAX_BLOCK, &t, Some((&input, &input)));
        }
        let w = reader.receive().expect("a window");
        assert!(w.iter().all(|f| *f == [0.25, 0.25, 0.25, 0.25]), "its input, and its output (passed through)");
        p.set_tap(None);
        for _ in 0..FRAMES.div_ceil(crate::MAX_BLOCK) {
            p.process_input(crate::MAX_BLOCK, &t, Some((&input, &input)));
        }
        assert!(reader.receive().is_none(), "taken off: nothing more");
    }
}
