//! ASIO without the Steinberg SDK: transit's native backend (the vendored `transit-audio-asio`
//! crate; see its NOTICE.md), behind what [`crate::devices`] and
//! [`crate::output`] need of an ASIO driver. Built when the `asio-native` feature is on and the
//! `asio` one (cpal over the real SDK) isn't: the `native_asio` cfg of `build.rs`.
//!
//! A driver has one set of buffers for both directions, and they must be made before it
//! starts. So the output opens the driver's input with it, and keeps it: what reads the input
//! later ([`attach_input`]) takes its blocks from the [`Tap`] that input feeds, rather than
//! opening the driver again.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use transit_audio_asio::{AsioBackend, AsioEndpoint};
use transit_audio_backend as ab;
use transit_audio_backend::{Backend, Stream};

/// What the driver's callbacks tell the app, off its audio thread's way.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fault {
    /// The driver reset, changed its rate or buffer size, stopped calling or went away: open it
    /// afresh.
    Reset,
    Other,
}

/// A block of the output to render: interleaved `f32`, and how long before it is heard.
pub type Render = Box<dyn FnMut(&mut [f32], Duration) + Send>;
/// A block of the input: interleaved `f32` frames of [`input_format`]'s channels.
pub type InputBlock = Box<dyn FnMut(&[f32]) + Send>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn backend() -> Result<AsioBackend, String> {
    AsioBackend::shared().map_err(|e| e.to_string())
}

/// What reads the driver's input: each gets every block. Changed only off the audio thread, which
/// never waits for it (a block goes unread rather than wait).
#[derive(Default)]
struct Tap {
    sinks: Mutex<Vec<(u64, InputBlock)>>,
    next: AtomicU64,
}

/// The driver open now: its input's tap, and what that input is.
struct Current {
    tap: Arc<Tap>,
    name: String,
    rate: u32,
    channels: usize,
}

static CURRENT: Mutex<Option<Current>> = Mutex::new(None);

/// The ASIO drivers installed, with their channels. The first time, with no driver playing, it
/// looks at each in turn (as the SDK path does, so the channels show); after that it lists what
/// was found, and [`rescan`] looks again. A driver playing stops the others loading, so the
/// listing then loads nothing.
pub fn list(inputs: bool) -> Vec<(String, usize)> {
    let Ok(backend) = backend() else { return Vec::new() };
    let direction = if inputs { ab::Direction::Input } else { ab::Direction::Output };
    let names = backend.driver_names().unwrap_or_default();
    if lock(&CURRENT).is_none() && names.iter().any(|n| backend.probed(n).is_none()) {
        let _ = backend.probe_all();
    }
    names
        .into_iter()
        .map(|name| {
            let channels = match backend.probed(&name) {
                Some(Ok(info)) => info.channels(direction) as usize,
                _ => 0,
            };
            (name, channels)
        })
        .collect()
}

/// Look at the drivers (load each in turn), or only the open one, in place, while one plays
/// (the others won't load then, and what was found of them stays).
pub fn rescan() {
    let Ok(backend) = backend() else { return };
    let open = lock(&CURRENT).as_ref().map(|c| c.name.clone());
    match open {
        Some(name) => {
            let _ = backend.probe_open(&name);
        }
        None => {
            let _ = backend.probe_all();
        }
    }
}

/// Let the driver go, at the end of the process, after the streams are closed.
pub fn shut_down() {
    AsioBackend::shut_down_shared();
}

/// A driver picked and its first format: what [`Planned::open`] opens.
pub struct Planned {
    backend: AsioBackend,
    plan: ab::Plan,
    pub name: String,
    pub rate: u32,
    pub channels: usize,
}

/// Choose how to open the driver called `name`: at its own rate, on all its output channels. It
/// loads the driver to ask it (and unloads it).
pub fn plan(name: Option<&str>) -> Result<Planned, String> {
    let name = name.ok_or("no ASIO driver picked (Preferences ▸ Audio)")?;
    let backend = backend()?;
    let wish = ab::OutputWish { device: Some(name.to_owned()), rates: Vec::new(), channels: 2, period: None };
    let plan = backend.plan_render(&wish)?;
    let first = plan.attempts.first().ok_or_else(|| format!("the ASIO driver \"{name}\" offers no format"))?;
    let (rate, channels) = (first.format.rate, first.format.channels as usize);
    if channels == 0 {
        return Err(format!("the ASIO driver \"{name}\" has no output channels"));
    }
    Ok(Planned { backend, plan, name: name.to_owned(), rate, channels })
}

impl Planned {
    /// `frames` within what the driver takes (it may have said nothing).
    fn buffer(&self, frames: u32) -> Option<u32> {
        let info = &self.plan.endpoint.handle::<AsioEndpoint>()?.info;
        let (min, max) = (u32::try_from(info.buffer.min).ok()?, u32::try_from(info.buffer.max).ok()?);
        (min > 0 && max >= min).then(|| frames.clamp(min, max))
    }

    /// Open the driver, its input and its output together, and start them. `render` makes the
    /// output's callback (once for each try: the buffer asked for, then the driver's own).
    pub fn open(self, frames: u32, mut render: impl FnMut() -> Render, fault: Arc<dyn Fn(Fault) + Send + Sync>) -> Result<Session, String> {
        // Small buffers keep pads and keys immediate; the driver's own if it refuses the size.
        let mut periods: Vec<ab::Period> = self.buffer(frames).map(ab::Period::Frames).into_iter().collect();
        periods.push(ab::Period::Default);
        let mut last = String::new();
        for period in periods {
            match self.open_with(period, render(), &fault) {
                Ok(session) => return Ok(session),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn open_with(&self, period: ab::Period, mut render: Render, fault: &Arc<dyn Fn(Fault) + Send + Sync>) -> Result<Session, String> {
        let endpoint = &self.plan.endpoint;
        let mut request = self.plan.attempts[0];
        request.period = period;
        let errors = || -> ab::ErrorCallback {
            let fault = fault.clone();
            Box::new(move |e| fault(if e == ab::StreamError::Other { Fault::Other } else { Fault::Reset }))
        };
        let tap = Arc::new(Tap::default());
        // The input first, as the driver makes both sets of buffers at once.
        let mut capture: Option<Box<dyn Stream>> = None;
        let mut in_channels = 0;
        if self.backend.capture_with_render(endpoint) && endpoint.inputs > 0 {
            let opened =
                self.backend.capture_format(endpoint, Some(request.format.rate)).map_err(ab::OpenError::Other).and_then(|format| {
                    let tap = tap.clone();
                    let on_block: ab::CaptureCallback = Box::new(move |block| {
                        if let Ok(mut sinks) = tap.sinks.try_lock()
                            && !sinks.is_empty()
                        {
                            block.chunks(&mut |_, samples| {
                                for (_, sink) in sinks.iter_mut() {
                                    sink(samples);
                                }
                            });
                        }
                    });
                    self.backend.open_capture(endpoint, &ab::OpenRequest { format, ..request }, on_block, errors())
                });
            match opened {
                Ok((stream, achieved)) => {
                    in_channels = achieved.format.channels as usize;
                    capture = Some(stream);
                }
                // Playing matters more: without it, nothing can read the input.
                Err(e) => eprintln!("ASIO input of \"{}\" not opened: {e}", self.name),
            }
        }
        let latency = Arc::new(AtomicU64::new(0));
        let ahead = latency.clone();
        let on_block: ab::RenderCallback = Box::new(move |block| {
            let ahead = Duration::from_nanos(ahead.load(Ordering::Relaxed));
            block.render(&mut |out| render(out, ahead));
        });
        let (stream, achieved) = self.backend.open_render(endpoint, &request, on_block, errors()).map_err(|e| e.to_string())?;
        if let Some(l) = achieved.latency.or_else(|| stream.latency()) {
            latency.store(u64::try_from(l.as_nanos()).unwrap_or(0), Ordering::Relaxed);
        }
        if let Some(capture) = &capture {
            let _ = capture.start();
        }
        stream.start().map_err(|e| e.to_string())?;
        *lock(&CURRENT) = Some(Current { tap: tap.clone(), name: self.name.clone(), rate: achieved.format.rate, channels: in_channels });
        Ok(Session { tap, _render: stream, _capture: capture })
    }
}

/// A driver playing. Dropping it stops the driver and lets it go, which can take a moment:
/// off the audio thread.
pub struct Session {
    tap: Arc<Tap>,
    // (Held to play, and dropped in this order: the output, then the input.)
    _render: Box<dyn Stream>,
    _capture: Option<Box<dyn Stream>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        lock(&self.tap.sinks).clear();
        let mut current = lock(&CURRENT);
        if current.as_ref().is_some_and(|c| Arc::ptr_eq(&c.tap, &self.tap)) {
            *current = None;
        }
    }
}

/// The input of the driver playing: (its name, rate, channels). `name` is the one asked for,
/// if any: the input is the output's driver and no other.
pub fn input_format(name: Option<&str>) -> Result<(String, u32, usize), String> {
    let current = lock(&CURRENT);
    let current = current.as_ref().ok_or("the ASIO driver isn't playing yet, and its input opens with its output")?;
    if name.is_some_and(|n| n != current.name) {
        return Err(format!("ASIO plays through \"{}\": its input is the one to use", current.name));
    }
    if current.channels == 0 {
        return Err(format!("the ASIO driver \"{}\" has no input", current.name));
    }
    Ok((current.name.clone(), current.rate, current.channels))
}

/// Read the input of the driver playing, until the guard drops.
pub fn attach_input(name: Option<&str>, on_block: InputBlock) -> Result<InputGuard, String> {
    input_format(name)?;
    let current = lock(&CURRENT);
    let current = current.as_ref().ok_or("the ASIO driver has stopped")?;
    let id = current.tap.next.fetch_add(1, Ordering::Relaxed);
    lock(&current.tap.sinks).push((id, on_block));
    Ok(InputGuard { tap: current.tap.clone(), id })
}

/// What reads the input of [`attach_input`]; dropping it stops that.
pub struct InputGuard {
    tap: Arc<Tap>,
    id: u64,
}

impl Drop for InputGuard {
    fn drop(&mut self) {
        lock(&self.tap.sinks).retain(|(id, _)| *id != self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test touches the process's one current driver.
    #[test]
    fn what_reads_the_input_gets_its_blocks_until_it_lets_go() {
        assert!(input_format(None).is_err(), "no driver is playing");
        let tap = Arc::new(Tap::default());
        *lock(&CURRENT) = Some(Current { tap: tap.clone(), name: "Interface".into(), rate: 48_000, channels: 2 });
        assert_eq!(input_format(Some("Interface")).unwrap(), ("Interface".to_string(), 48_000, 2));
        assert!(input_format(Some("Another")).is_err(), "the input is the output's driver and no other");
        let heard = Arc::new(Mutex::new(Vec::<f32>::new()));
        let sink = heard.clone();
        let guard = attach_input(None, Box::new(move |block| lock(&sink).extend_from_slice(block))).unwrap();
        let feed = |block: &[f32]| {
            for (_, sink) in lock(&tap.sinks).iter_mut() {
                sink(block);
            }
        };
        feed(&[0.5, -0.5]);
        assert_eq!(*lock(&heard), [0.5, -0.5]);
        drop(guard);
        feed(&[1.0, 1.0]);
        assert_eq!(lock(&heard).len(), 2, "nothing arrives after the guard drops");
        *lock(&CURRENT) = None;
    }
}
