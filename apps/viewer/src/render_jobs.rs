//! The New Sampler's Rendered stretch engine, run: the renders engines ask for
//! ([`deckard_engine::SampleCache::wanted_renders`]) made on a thread of their own,
//! nearest the song's needs first (docs/superpowers/plans/2026-11-12-sync-playback.md).

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use deckard_engine::render::RenderJob;
use deckard_engine::sample::Sample;

/// The render thread, started on the first job.
#[derive(Default)]
pub struct Renders {
    send: Option<Sender<RenderJob>>,
    done: Option<Receiver<(RenderJob, Arc<Sample>)>>,
}

impl Renders {
    /// Make `job`'s render.
    pub fn want(&mut self, job: RenderJob) {
        if self.send.is_none() {
            let (send, jobs) = channel::<RenderJob>();
            let (tell, done) = channel();
            let started = std::thread::Builder::new().name("renders".into()).spawn(move || {
                for job in jobs {
                    let r = job.run();
                    if tell.send((job, r)).is_err() {
                        return;
                    }
                }
            });
            if started.is_err() {
                return;
            }
            (self.send, self.done) = (Some(send), Some(done));
        }
        if let Some(s) = &self.send {
            let _ = s.send(job);
        }
    }

    /// The renders made since the last call.
    pub fn poll(&mut self) -> Vec<(RenderJob, Arc<Sample>)> {
        self.done.as_ref().map_or_else(Vec::new, |d| d.try_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_what_it_is_asked() {
        let src = Arc::new(Sample { rate: 48_000, frames: vec![[0.1; 2]; 4800] });
        let mut r = Renders::default();
        r.want(RenderJob::new(&src, (0, 4800), 100.0, 120.0, 0.0));
        let mut got = Vec::new();
        for _ in 0..500 {
            got.extend(r.poll());
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1.frames.len(), 4000);
    }
}
