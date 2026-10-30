//! Real-time output through the default audio device (feature `output`).
//!
//! The audio thread owns the playing [`Engine`]: nothing else touches it, and it never
//! waits for another thread. Other threads (the UI, MIDI input, the recorder) send it
//! [`Command`]s through a lock-free queue; it applies them at the start of each block.
//! What it replaces (an old engine, timelines, …) goes back through another queue to
//! be freed off the audio thread (by [`Output::collect`], which most calls run). What
//! the UI shows comes from [`crate::Telemetry`], published after each block.
//!
//! Live events (pads, keys, MIDI) carry the time they happened. In sample-accurate
//! timing (the default) each is played exactly one block after it happened, at its
//! sample: a constant delay of one buffer and no jitter, where applying events at the
//! start of the next block (Maschine's way) moves them by up to a buffer.

use crate::queue::Queue;
use crate::{Engine, Live, Mode, Tap, Timeline, Timelines};
use cpal::traits::{DeviceTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How the audio callback has been keeping up (since the output started).
#[derive(Debug, Default)]
pub struct Stats {
    pub callbacks: AtomicU64,
    /// Blocks played as silence because the engine couldn't be had (kept for the
    /// summary's format; with the audio thread owning the engine it stays 0).
    pub missed: AtomicU64,
    /// Blocks that took longer to compute than they last (the device ran dry).
    pub late: AtomicU64,
    /// The longest block, as a share of its duration (per mille).
    pub peak_load: AtomicU64,
    /// Total compute time and audio time (µs), for the average load.
    pub busy_us: AtomicU64,
    pub audio_us: AtomicU64,
    /// Commands that found the queue full (the sender retried or gave up).
    pub queue_full: AtomicU64,
    /// Underruns the device reported ("A buffer underrun or overrun occurred"): also
    /// at start and restart, when the device itself takes a moment.
    pub device_errors: AtomicU64,
}

impl Stats {
    /// "callbacks, missed, late, peak %, average %".
    pub fn summary(&self) -> String {
        let l = |a: &AtomicU64| a.load(Ordering::Relaxed);
        let avg = if l(&self.audio_us) > 0 { l(&self.busy_us) as f64 / l(&self.audio_us) as f64 * 100.0 } else { 0.0 };
        format!(
            "{} blocks, {} missed (engine busy), {} late (over time), peak load {:.0} %, average {avg:.1} %, {} reported by the device",
            l(&self.callbacks),
            l(&self.missed),
            l(&self.late),
            l(&self.peak_load) as f64 / 10.0,
            l(&self.device_errors)
        )
    }
}

/// An edit for the playing engine: the rebuilt engine and the timelines it may need.
/// After it is applied it holds what was replaced.
pub struct AdoptMsg {
    engine: Option<Engine>,
    timelines: Timelines,
}

/// What other threads ask of the audio thread.
pub enum Command {
    /// Play this engine (made at the output's rate), replacing any other.
    Play(Box<Engine>),
    /// Take up an edit, keeping the position and what sounds.
    Adopt(Box<AdoptMsg>),
    /// Switch to a mode (with its timeline) when the current pass ends.
    Queue(Mode, Box<Timeline>),
    /// At tick `.0`, continue from tick `.1` (`None` cancels).
    JumpAt(Option<(i64, i64)>),
    Seek(i64),
    /// Loop the timeline at its end (else stop there).
    Looping(bool),
    /// Click on the beats (or not).
    Metronome(bool),
    /// Record live notes (REC) or stop.
    Record(bool),
    /// Note Repeat / the Arp on (with its settings) or off.
    Repeat(Option<(deckard_model::Arp, crate::repeat::RepeatMode)>),
    /// The metronome's volume (0..1), whether it clicks while recording, and the note
    /// value it clicks on (0: the song's beats).
    MetronomeSettings {
        volume: f32,
        auto: bool,
        ticks: u32,
    },
    /// Prehear a sample (`None` stops), at a gain.
    Prehear(Option<Arc<crate::sample::Sample>>, f32),
    Stop,
    /// Something played live, and when it happened.
    Live(Instant, Live),
    SetTap(Option<Tap>),
    SetMidi(Option<std::sync::mpsc::SyncSender<crate::midi::MidiOut>>),
    /// Output ports set to MPE (name → member bend range).
    SetMpeOut(std::collections::BTreeMap<String, u8>),
    SetLiveIn(Option<crate::LiveIn>),
    /// Run on the engine (a query; the box comes back to be freed).
    Run(Box<dyn FnMut(&mut Engine) + Send>),
    /// The group pads and keys play: its sounds aren't rendered ahead.
    Focus(Option<usize>),
    /// Something the engine's players sent back, to be freed here.
    Junk(crate::Junk),
    /// Follow this clock (Link, incoming MIDI clock), or none.
    Clock(Option<Box<dyn crate::sync::Clock>>),
    /// Send MIDI clock (or not).
    ClockOut(Option<crate::sync::ClockOut>),
}

/// The sending side of an output's command queue (any thread may hold one).
#[derive(Clone)]
pub struct Sender {
    commands: Arc<Queue<Command>>,
    garbage: Arc<Queue<Command>>,
    /// Commands sent, and whether an engine was sent to play.
    sent: Arc<AtomicU64>,
    has_engine: Arc<AtomicBool>,
    stats: Arc<Stats>,
    played: Arc<Mutex<Played>>,
}

/// How long a sound shows as played after a note that isn't held (a pad hit).
pub const FLASH: Duration = Duration::from_millis(120);

/// Live notes sent, so the UI can light the sounds they play: the pitches held per
/// (group, sound), and when a note last started.
#[derive(Default)]
struct Played {
    held: std::collections::HashMap<(usize, usize), Vec<i64>>,
    last: std::collections::HashMap<(usize, usize), Instant>,
}

impl Sender {
    /// Queue `cmd` for the audio thread. If the queue is full (the audio thread has
    /// stopped taking them), waits a little, freeing what came back meanwhile.
    pub fn send(&self, cmd: Command) -> bool {
        let mut cmd = cmd;
        for _ in 0..200 {
            match self.commands.push(cmd) {
                Ok(()) => {
                    self.sent.fetch_add(1, Ordering::Release);
                    return true;
                }
                Err(back) => {
                    self.stats.queue_full.fetch_add(1, Ordering::Relaxed);
                    cmd = back;
                    self.collect();
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
        false
    }

    /// Play something live now (its time is taken here).
    pub fn live(&self, event: Live) -> bool {
        self.note_played(&event);
        self.send(Command::Live(Instant::now(), event))
    }

    /// Note a live note for [`Sender::played_sounds`] (also for notes played some other
    /// way, such as a fresh engine's first note).
    pub fn note_played(&self, event: &Live) {
        let Ok(mut p) = self.played.lock() else { return };
        match *event {
            Live::NoteOn { group, sound, pitch, length, .. } => {
                p.last.insert((group, sound), Instant::now());
                if length == i64::MAX {
                    p.held.entry((group, sound)).or_default().push(pitch);
                }
            }
            Live::NoteIn { group, sound, pitch, velocity, .. } => {
                p.last.insert((group, sound), Instant::now());
                if velocity > 0 {
                    p.held.entry((group, sound)).or_default().push(pitch);
                }
            }
            Live::NoteRelease { group, sound, pitch } => {
                if let Some(h) = p.held.get_mut(&(group, sound)) {
                    h.retain(|&q| q != pitch);
                    if h.is_empty() {
                        p.held.remove(&(group, sound));
                    }
                }
            }
            Live::ControlIn { .. } | Live::Expression { .. } | Live::RepeatPressure { .. } => {}
        }
    }

    /// The (group, sound)s live notes play now: held, or started within [`FLASH`].
    pub fn played_sounds(&self) -> std::collections::HashSet<(usize, usize)> {
        let Ok(mut p) = self.played.lock() else { return Default::default() };
        let now = Instant::now();
        p.last.retain(|_, t| now.duration_since(*t) < FLASH);
        p.held.keys().chain(p.last.keys()).copied().collect()
    }

    /// Whether an engine was sent to play.
    pub fn has_engine(&self) -> bool {
        self.has_engine.load(Ordering::Acquire)
    }

    /// Free what the audio thread sent back.
    pub fn collect(&self) {
        while let Some(g) = self.garbage.pop() {
            drop(g);
        }
    }
}

pub struct Output {
    /// Device sample rate; create engines at this rate.
    pub rate: u32,
    pub device: String,
    /// Frames per audio callback, as last seen.
    block: Arc<std::sync::atomic::AtomicUsize>,
    /// Audio to play through as it comes (interleaved stereo; an input being
    /// monitored while recording).
    monitor: Arc<Mutex<Vec<f32>>>,
    /// Given to each engine played (and to the playing one when set).
    midi_tx: Mutex<Option<std::sync::mpsc::SyncSender<crate::midi::MidiOut>>>,
    /// Output ports set to MPE, for each engine played.
    mpe_out: Mutex<std::collections::BTreeMap<String, u8>>,
    live_in: Mutex<Option<crate::LiveIn>>,
    /// MIDI clock sent, for each engine played.
    clock_out: Mutex<Option<crate::sync::ClockOut>>,
    pub stats: Arc<Stats>,
    /// What the audio thread last published of the engine (meters, playhead).
    telemetry: Arc<Mutex<crate::Telemetry>>,
    sender: Sender,
    /// Live events placed at their sample (else at the start of the next block).
    sample_accurate: Arc<AtomicBool>,
    /// Threads that render sounds alongside the audio thread.
    pool: Option<Arc<crate::pool::RenderPool>>,
    /// Render sounds that can't get live input this many frames ahead (0: off).
    ahead_frames: std::sync::atomic::AtomicU64,
    /// The group pads and keys play (`usize::MAX`: none), for engines played later.
    focus: std::sync::atomic::AtomicUsize,
    /// Engines played so far.
    plays: AtomicU64,
    /// [`Output::set_routing`], as the audio thread reads it (-1: not connected).
    routing: Arc<[AtomicI32; OUT_CHANNELS]>,
    channels: usize,
    /// Effects' meters for their panels (the Compressor's).
    fx_meters: Arc<crate::meters::FxMeters>,
    /// The routes' travels, for their knobs.
    route_travels: Arc<crate::meters::RouteTravels>,
    _stream: cpal::Stream,
}

/// Peak (left, right) of each sound, `[group][sound]`.
pub type Meters = Vec<Vec<(f32, f32)>>;

/// Buffer sizes offered (frames): smaller is quicker to respond, larger leaves more
/// room for heavy projects and plug-ins' occasional slow blocks.
pub const BUFFER_SIZES: [u32; 5] = [64, 128, 256, 512, 1024];

/// The buffer size used unless another is asked for.
pub const DEFAULT_BUFFER: u32 = 256;

/// Commands waiting at once (pads, MIDI, edits): far more than a block brings.
const QUEUE_ROOM: usize = 4096;

/// Apply `cmd` to the audio thread's engine; what to free elsewhere comes back.
/// `start`: when this block's callback began; `period`: its length.
fn apply(
    engine: &mut Option<Box<Engine>>,
    cmd: Command,
    start: Instant,
    period: Duration,
    rate: f64,
    frames: usize,
    exact: bool,
) -> Option<Command> {
    match cmd {
        Command::Play(mut new) => {
            if let Some(old) = engine.as_mut() {
                // Its players stop now (they may share plug-ins with the new engine).
                old.halt_ahead();
                // An internal recording goes on from the new engine; the old one's
                // plug-in notes end (the new one may play the same instances).
                if let Some(t) = old.tap().cloned() {
                    new.continue_tap(t, old.tap_frames());
                }
                old.release_plugins();
                // Its notes out to MIDI end (and MPE channels reset): the new one starts afresh.
                old.end_midi();
                old.release_routes();
                new.take_unroutes(old);
            }
            engine.replace(new).map(Command::Play)
        }
        Command::Adopt(mut msg) => {
            if let (Some(e), Some(fresh)) = (engine.as_mut(), msg.engine.take()) {
                msg.engine = Some(e.adopt_with(fresh, &mut msg.timelines));
            }
            Some(Command::Adopt(msg))
        }
        Command::Queue(mode, mut t) => {
            if let Some(e) = engine.as_mut() {
                e.queue_timeline(mode, &mut t);
            }
            Some(Command::Queue(mode, t))
        }
        Command::JumpAt(j) => {
            if let Some(e) = engine.as_mut() {
                e.jump_at(j);
            }
            None
        }
        Command::Seek(to) => {
            if let Some(e) = engine.as_mut() {
                e.seek(to);
            }
            None
        }
        Command::Looping(on) => {
            if let Some(e) = engine.as_mut() {
                e.looping = on;
            }
            None
        }
        Command::MetronomeSettings { volume, auto, ticks } => {
            if let Some(e) = engine.as_mut() {
                e.set_metronome_volume(volume);
                e.metronome_auto = auto;
                e.metronome_ticks = ticks;
            }
            None
        }
        Command::Repeat(r) => {
            if let Some(e) = engine.as_mut() {
                e.set_repeat(r);
            }
            None
        }
        Command::Record(on) => {
            if let Some(e) = engine.as_mut() {
                e.set_recording(on);
            }
            None
        }
        Command::Metronome(on) => {
            if let Some(e) = engine.as_mut() {
                e.metronome = on;
            }
            None
        }
        Command::Prehear(sample, gain) => {
            let e = engine.as_mut()?;
            Some(Command::Prehear(e.prehear(sample, gain), 0.0))
        }
        Command::Stop => {
            if let Some(e) = engine.as_mut() {
                e.stop();
            }
            None
        }
        Command::Live(at, event) => {
            if let Some(e) = engine.as_mut() {
                // One block after it happened, at its sample (within this block).
                let offset = if exact {
                    let due = at + period;
                    let ahead = due.checked_duration_since(start).unwrap_or_default();
                    ((ahead.as_secs_f64() * rate) as u64).min(frames.saturating_sub(1) as u64)
                } else {
                    0
                };
                e.schedule(offset, event);
            }
            None
        }
        Command::SetTap(tap) => {
            let e = engine.as_mut()?;
            let old = e.tap().cloned();
            e.set_tap(tap);
            Some(Command::SetTap(old))
        }
        Command::SetMidi(tx) => {
            let e = engine.as_mut()?;
            Some(Command::SetMidi(std::mem::replace(&mut e.midi_tx, tx)))
        }
        Command::SetMpeOut(ports) => {
            engine.as_mut()?.set_mpe_out(ports);
            None
        }
        Command::SetLiveIn(live) => {
            let e = engine.as_mut()?;
            Some(Command::SetLiveIn(std::mem::replace(&mut e.live_in, live)))
        }
        Command::Run(mut f) => {
            if let Some(e) = engine.as_mut() {
                f(e);
            }
            Some(Command::Run(f))
        }
        Command::Focus(group) => {
            if let Some(e) = engine.as_mut() {
                e.set_focus(group);
            }
            None
        }
        Command::Junk(j) => Some(Command::Junk(j)),
        // (The callback keeps the clock; see `Output::with_device`.)
        Command::Clock(c) => Some(Command::Clock(c)),
        // (Without an engine it goes back as it came, to be freed elsewhere.)
        Command::ClockOut(c) => match engine.as_mut() {
            Some(e) => Some(Command::ClockOut(e.set_clock_out(c))),
            None => Some(Command::ClockOut(c)),
        },
    }
}

/// Before a block: tell the engine when it is heard (`ahead` from now), and have it
/// follow the clock, if any, there.
fn before_block(engine: &mut Engine, clock: &mut Option<Box<dyn crate::sync::Clock>>, now: Instant, ahead: Duration) {
    engine.heard_at(now + ahead);
    if let Some(c) = clock.as_mut()
        && let Some(f) = c.follow(ahead, engine.quantum())
    {
        engine.follow(&f);
    }
}

/// DECKARD's output channels: Out 1 L … Out 16 R.
pub const OUT_CHANNELS: usize = crate::OUTPUTS * 2;

/// Out 1 L and R on the interface's first two channels; the rest not connected.
pub fn default_route(i: usize) -> i32 {
    if i < 2 { i as i32 } else { -1 }
}

pub use crate::devices::{default_output, device_name, output_devices};

impl Output {
    pub fn new() -> Result<Output, String> {
        Output::with_options(DEFAULT_BUFFER, true)
    }

    /// Open the default device asking for buffers of `frames` (the device may give
    /// another size; [`Output::block_frames`] tells), rendering sounds on several
    /// cores if `multicore`.
    pub fn with_options(frames: u32, multicore: bool) -> Result<Output, String> {
        Output::with_device(None, frames, multicore)
    }

    /// [`Output::with_options`] on the output device called `device` (see
    /// [`output_devices`]); `None`: the system's default.
    pub fn with_device(device: Option<&str>, frames: u32, multicore: bool) -> Result<Output, String> {
        let (device, config) = crate::devices::open_output(device)?;
        if config.sample_format() != cpal::SampleFormat::F32 {
            return Err(format!("output sample format {:?} is not supported (need f32)", config.sample_format()));
        }
        let rate = config.sample_rate();
        let channels = config.channels() as usize;
        let block = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let monitor: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let stats = Arc::new(Stats::default());
        let telemetry: Arc<Mutex<crate::Telemetry>> = Arc::default();
        let sender = Sender {
            commands: Arc::new(Queue::new(QUEUE_ROOM)),
            garbage: Arc::new(Queue::new(QUEUE_ROOM)),
            sent: Arc::default(),
            has_engine: Arc::default(),
            stats: stats.clone(),
            played: Arc::default(),
        };
        let sample_accurate = Arc::new(AtomicBool::new(true));
        let routing: Arc<[AtomicI32; OUT_CHANNELS]> = Arc::new(std::array::from_fn(|i| AtomicI32::new(default_route(i))));
        // Small buffers keep pads and keys immediate (256 frames is ~5.3 ms at 48 kHz);
        // fall back to the device's default if the size is refused.
        let wanted = match config.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => Some(frames.clamp(*min, *max)),
            cpal::SupportedBufferSize::Unknown => None,
        };
        let build = |size: cpal::BufferSize| {
            let block = block.clone();
            let monitor = monitor.clone();
            let stats = stats.clone();
            let err_stats = stats.clone();
            let telemetry = telemetry.clone();
            let (commands, garbage) = (sender.commands.clone(), sender.garbage.clone());
            let exact = sample_accurate.clone();
            let routing = routing.clone();
            // Out 1–16 (other than the master's), as the engine fills them.
            let mut ext = vec![vec![0f32; 16_384]; crate::OUTPUTS];
            let mut stream_config: cpal::StreamConfig = config.into();
            stream_config.buffer_size = size;
            let mut stereo = vec![0f32; 16_384];
            let mut engine: Option<Box<Engine>> = None;
            let mut clock: Option<Box<dyn crate::sync::Clock>> = None;
            let mut applied = 0u64;
            device.build_output_stream(
                stream_config,
                move |out: &mut [f32], info: &cpal::OutputCallbackInfo| {
                    let started = Instant::now();
                    let frames = out.len() / channels;
                    block.store(frames, std::sync::atomic::Ordering::Relaxed);
                    if stereo.len() < frames * 2 {
                        stereo.resize(frames * 2, 0.0); // only for unusually large host blocks
                    }
                    let buf = &mut stereo[..frames * 2];
                    buf.fill(0.0);
                    // What other threads asked for since the last block.
                    let period = Duration::from_secs_f64(frames as f64 / rate as f64);
                    let exact = exact.load(Ordering::Relaxed);
                    while let Some(cmd) = commands.pop() {
                        applied += 1;
                        let cmd = match cmd {
                            Command::Clock(c) => Command::Clock(std::mem::replace(&mut clock, c)),
                            cmd => cmd,
                        };
                        if let Some(back) = apply(&mut engine, cmd, started, period, rate as f64, frames, exact)
                            && let Err(back) = garbage.push(back)
                        {
                            // Nobody is collecting: free it here rather than leak.
                            drop(back);
                        }
                    }
                    let ext_len = (frames * 2).min(16_384);
                    for b in &mut ext {
                        b[..ext_len].fill(0.0);
                    }
                    let master_bus = engine.as_ref().map_or(Some(0), |e| e.master_bus());
                    if let Some(engine) = engine.as_mut() {
                        // When this block is heard: as the device says, else a block on.
                        let ts = info.timestamp();
                        let ahead = ts.playback.duration_since(ts.callback);
                        before_block(engine, &mut clock, started, if ahead.is_zero() { period } else { ahead });
                        engine.process_outputs(buf, &mut ext);
                        // What its players sent back is freed off this thread.
                        while let Some(j) = engine.take_junk() {
                            if let Err(back) = garbage.push(Command::Junk(j)) {
                                drop(back);
                                break;
                            }
                        }
                    }
                    // Never waits: if the UI is reading, the next block publishes
                    // (peaks keep adding up meanwhile).
                    if let Ok(mut t) = telemetry.try_lock() {
                        match engine.as_mut() {
                            Some(e) => e.publish(&mut t),
                            None => t.finished = true,
                        }
                        t.has_engine = engine.is_some();
                        t.actions = applied;
                    }
                    if let Ok(mut m) = monitor.try_lock()
                        && !m.is_empty()
                    {
                        let n = m.len().min(buf.len());
                        for (o, v) in buf.iter_mut().zip(m.drain(..n)) {
                            *o = (*o + v).clamp(-4.0, 4.0);
                        }
                        // Don't let it fall behind (more than ~0.1 s).
                        if m.len() > 9_600 {
                            let extra = m.len() - 9_600;
                            m.drain(..extra);
                        }
                    }
                    // Each of DECKARD's output channels (Out 1 L … Out 16 R) onto the
                    // interface channel it's routed to.
                    out.fill(0.0);
                    for (i, to) in routing.iter().enumerate() {
                        let to = to.load(Ordering::Relaxed);
                        if to < 0 || to as usize >= channels {
                            continue;
                        }
                        let (bus, side) = (i / 2, i % 2);
                        let src: &[f32] = if master_bus == Some(bus) { buf } else { &ext[bus][..ext_len] };
                        for (f, frame) in out.chunks_mut(channels).enumerate() {
                            if let Some(v) = src.get(f * 2 + side) {
                                frame[to as usize] += *v;
                            }
                        }
                    }
                    let busy = started.elapsed().as_micros() as u64;
                    let audio = frames as u64 * 1_000_000 / rate as u64;
                    stats.callbacks.fetch_add(1, Ordering::Relaxed);
                    stats.busy_us.fetch_add(busy, Ordering::Relaxed);
                    stats.audio_us.fetch_add(audio, Ordering::Relaxed);
                    if busy > audio {
                        stats.late.fetch_add(1, Ordering::Relaxed);
                    }
                    if let Some(load) = (busy * 1000).checked_div(audio) {
                        stats.peak_load.fetch_max(load, Ordering::Relaxed);
                    }
                },
                move |e| {
                    err_stats.device_errors.fetch_add(1, Ordering::Relaxed);
                    eprintln!("audio error: {e} (after {} blocks)", err_stats.callbacks.load(Ordering::Relaxed));
                },
                None,
            )
        };
        let stream = match wanted.map(|n| build(cpal::BufferSize::Fixed(n))) {
            Some(Ok(s)) => s,
            _ => build(cpal::BufferSize::Default).map_err(|e| e.to_string())?,
        };
        stream.play().map_err(|e| e.to_string())?;
        Ok(Output {
            rate,
            device: device_name(&device),
            block,
            monitor,
            midi_tx: Mutex::new(None),
            mpe_out: Mutex::default(),
            live_in: Mutex::new(None),
            clock_out: Mutex::new(None),
            stats,
            telemetry,
            sender,
            sample_accurate,
            pool: multicore.then(|| Arc::new(crate::pool::RenderPool::for_machine(frames, rate))),
            ahead_frames: std::sync::atomic::AtomicU64::new(0),
            focus: std::sync::atomic::AtomicUsize::new(usize::MAX),
            plays: AtomicU64::new(0),
            fx_meters: Arc::default(),
            route_travels: Arc::default(),
            routing,
            channels,
            _stream: stream,
        })
    }

    /// Preferences > Audio > Routing > Output: the interface channel (from 0) each of
    /// DECKARD's output channels (Out 1 L, Out 1 R, … Out 16 R) plays on, or none.
    pub fn set_routing(&self, routing: &[Option<usize>]) {
        for (i, r) in self.routing.iter().enumerate() {
            let to = routing.get(i).copied().unwrap_or(None).map_or(-1, |c| c as i32);
            r.store(to, Ordering::Relaxed);
        }
    }

    /// The interface's output channels.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// A handle other threads use to send commands (MIDI input).
    pub fn sender(&self) -> Sender {
        self.sender.clone()
    }

    /// Free what the audio thread sent back (old engines, timelines).
    pub fn collect(&self) {
        self.sender.collect();
    }

    /// Start playing `engine` (created at [`Output::rate`]), replacing any other.
    pub fn play(&self, engine: Engine) {
        let mut engine = engine;
        engine.midi_tx = self.midi_tx.lock().unwrap().clone();
        engine.set_mpe_out(self.mpe_out.lock().unwrap().clone());
        engine.live_in = self.live_in.lock().unwrap().clone();
        engine.set_clock_out(self.clock_out.lock().unwrap().clone());
        engine.pool = self.pool.clone();
        engine.set_fx_meters(&self.fx_meters);
        engine.set_route_travels(&self.route_travels);
        // Sounds that can't get live input render ahead, on a thread of their own.
        let ahead = self.ahead_frames.load(Ordering::Relaxed);
        if ahead > 0 {
            let focus = self.focus.load(Ordering::Relaxed);
            engine.set_focus((focus != usize::MAX).then_some(focus));
            let workers = self.pool.as_ref().map_or(0, |p| p.workers() / 2);
            let block = self.block_frames().max(64) as u32;
            engine.render_ahead(ahead, workers, block, false);
        }
        self.sender.collect();
        if self.sender.send(Command::Play(Box::new(engine))) {
            self.sender.has_engine.store(true, Ordering::Release);
            self.plays.fetch_add(1, Ordering::AcqRel);
        }
    }

    /// The meters of the effect in chain slot `slot` of sound (`g`, `s`) since the last
    /// call: (input level, gain reduction in dB, output level).
    pub fn fx_meters(&self, g: usize, s: usize, slot: usize) -> (f32, f32, f32) {
        self.fx_meters.take(g, s, slot)
    }

    /// Where route `k` of sound (`g`, `s`) has its knob now (0..1), as the playing
    /// engine last published; `None` when it isn't driving it, or the engine's route
    /// there isn't the one keyed `key` ([`crate::meters::route_key`]) yet.
    pub fn route_travel(&self, g: usize, s: usize, k: usize, key: u32) -> Option<f32> {
        self.route_travels.get(g, s, k, key)
    }

    /// How many engines have been played (an edit is built against the playing one).
    pub fn plays(&self) -> u64 {
        self.plays.load(Ordering::Acquire)
    }

    /// Commands sent so far (compare with [`crate::Telemetry::actions`]).
    pub fn sent(&self) -> u64 {
        self.sender.sent.load(Ordering::Acquire)
    }

    /// Whether an engine has been sent to play (live notes need one).
    pub fn has_engine(&self) -> bool {
        self.sender.has_engine()
    }

    /// Hand the playing engine an edit: `fresh`, built from `project` at the mode
    /// it plays. The timelines it may need are made here, for the mode playing and the
    /// one queued as last published (if playback moved on meanwhile, the engine says
    /// so in [`crate::Telemetry::stale`] and the edit should be sent again).
    pub fn adopt(&self, fresh: Engine, project: &deckard_model::Project) {
        let mut fresh = fresh;
        fresh.set_fx_meters(&self.fx_meters);
        fresh.set_route_travels(&self.route_travels);
        let (mode, queued) = self.telemetry(|t| (t.mode, t.queued));
        let mut modes: Vec<Mode> = mode.into_iter().collect();
        modes.extend(queued);
        // (None needed when the edit keeps the playing timelines.)
        let timelines = if fresh.keeps_timelines() { Timelines::default() } else { Timelines::of(project, &modes) };
        self.sender.collect();
        self.sender.send(Command::Adopt(Box::new(AdoptMsg { engine: Some(fresh), timelines })));
    }

    /// Switch to `mode` when the current pass ends (the playing mode: cancel a switch).
    pub fn queue(&self, project: &deckard_model::Project, mode: Mode) {
        self.sender.send(Command::Queue(mode, Box::new(crate::timeline(project, mode))));
    }

    /// When playback reaches tick `.0`, continue from `.1` (`None` cancels).
    pub fn jump_at(&self, jump: Option<(i64, i64)>) {
        self.sender.send(Command::JumpAt(jump));
    }

    /// Continue from tick `to` now.
    pub fn seek(&self, to: i64) {
        self.sender.send(Command::Seek(to));
    }

    /// Loop what plays at its end (else stop there).
    pub fn set_looping(&self, on: bool) {
        self.sender.send(Command::Looping(on));
    }

    /// Note Repeat / the Arp on (with its settings) or off.
    pub fn set_repeat(&self, r: Option<(deckard_model::Arp, crate::repeat::RepeatMode)>) {
        self.sender.send(Command::Repeat(r));
    }

    /// Record live notes (REC) or stop.
    pub fn set_recording(&self, on: bool) {
        self.sender.send(Command::Record(on));
    }

    /// The live notes recorded since last asked (see [`crate::Recorded`]).
    pub fn take_recorded(&self) -> Vec<crate::Recorded> {
        self.telemetry(|t| std::mem::take(&mut t.recorded))
    }

    /// The per-note expression recorded since last asked (see [`crate::RecordedExpr`]).
    pub fn take_recorded_expr(&self) -> Vec<crate::RecordedExpr> {
        self.telemetry(|t| std::mem::take(&mut t.recorded_expr))
    }

    /// The channel-wide controllers recorded since last asked.
    pub fn take_recorded_controls(&self) -> Vec<crate::RecordedControl> {
        self.telemetry(|t| std::mem::take(&mut t.recorded_controls))
    }

    /// The metronome's volume (0..1), whether it clicks while recording (AUTOENABLE),
    /// and the note value it clicks on (TIMESIGNATURE; 0: the song's beats).
    pub fn set_metronome_settings(&self, volume: f32, auto: bool, ticks: u32) {
        self.sender.send(Command::MetronomeSettings { volume, auto, ticks });
    }

    /// Click on the beats of what plays (or not).
    pub fn set_metronome(&self, on: bool) {
        self.sender.send(Command::Metronome(on));
    }

    /// Prehear `sample` (the browser's preview; `None` stops it) at `gain`.
    pub fn prehear(&self, sample: Option<Arc<crate::sample::Sample>>, gain: f32) {
        self.sender.send(Command::Prehear(sample, gain));
    }

    /// Play something live now (a pad, a key, a controller).
    pub fn live(&self, event: Live) {
        self.sender.live(event);
    }

    /// See [`Sender::note_played`].
    pub fn note_played(&self, event: &Live) {
        self.sender.note_played(event);
    }

    /// See [`Sender::played_sounds`].
    pub fn played_sounds(&self) -> std::collections::HashSet<(usize, usize)> {
        self.sender.played_sounds()
    }

    /// Copy audio from inside the engine to `tap` (or stop, with `None`).
    pub fn set_tap(&self, tap: Option<Tap>) {
        self.sender.send(Command::SetTap(tap));
    }

    /// Render sounds that can't get live input `seconds` ahead (0: off), for engines
    /// played from now on.
    pub fn set_render_ahead(&self, seconds: f64) {
        self.ahead_frames.store((seconds * self.rate as f64).round() as u64, Ordering::Relaxed);
    }

    /// The group pads and keys play now (its sounds play live).
    pub fn set_focus(&self, group: Option<usize>) {
        let v = group.unwrap_or(usize::MAX);
        if self.focus.swap(v, Ordering::Relaxed) != v {
            self.sender.send(Command::Focus(group));
        }
    }

    /// Place live events at their sample, one block after they happened (else at the
    /// start of the next block).
    pub fn set_sample_accurate(&self, on: bool) {
        self.sample_accurate.store(on, Ordering::Relaxed);
    }

    /// Send the engines' MIDI for output ports to `tx` (this one's and later ones).
    pub fn set_midi(&self, tx: std::sync::mpsc::SyncSender<crate::midi::MidiOut>) {
        *self.midi_tx.lock().unwrap() = Some(tx.clone());
        self.sender.send(Command::SetMidi(Some(tx)));
    }

    /// Send MPE to these output ports (name → member bend range; others normal), from
    /// the playing engine on; nothing if unchanged.
    pub fn set_mpe_out(&self, ports: &std::collections::BTreeMap<String, u8>) {
        let mut m = self.mpe_out.lock().unwrap();
        if *m != *ports {
            *m = ports.clone();
            self.sender.send(Command::SetMpeOut(ports.clone()));
        }
    }

    /// Follow `clock` (Link, incoming MIDI clock) from the next block on, or none.
    pub fn set_clock(&self, clock: Option<Box<dyn crate::sync::Clock>>) {
        self.sender.send(Command::Clock(clock));
    }

    /// Send MIDI clock (this engine and later ones), or not.
    pub fn set_clock_out(&self, out: Option<crate::sync::ClockOut>) {
        let mut cur = self.clock_out.lock().unwrap();
        if *cur != out {
            *cur = out.clone();
            self.sender.send(Command::ClockOut(out));
        }
    }

    /// Where the interface input comes from (this engine's and later ones').
    pub fn set_live_in(&self, live: crate::LiveIn) {
        *self.live_in.lock().unwrap() = Some(live.clone());
        self.sender.send(Command::SetLiveIn(Some(live)));
    }

    /// Where to put audio to be heard as it comes (see [`Output::monitor`]).
    pub fn monitor(&self) -> Arc<Mutex<Vec<f32>>> {
        self.monitor.clone()
    }

    /// Fade out and stop.
    pub fn stop(&self) {
        self.sender.send(Command::Stop);
    }

    /// Worker threads rendering alongside the audio thread, and the jobs they did.
    pub fn render_threads(&self) -> Option<(usize, u64)> {
        self.pool.as_ref().map(|p| (p.workers(), p.helped()))
    }

    /// Frames per audio callback (0 until the first callback).
    pub fn block_frames(&self) -> usize {
        self.block.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Read what the audio thread last published (meters, playhead, zones). Also frees
    /// what it sent back.
    pub fn telemetry<R>(&self, f: impl FnOnce(&mut crate::Telemetry) -> R) -> R {
        self.sender.collect();
        f(&mut self.telemetry.lock().unwrap())
    }

    /// The peaks of each sound (`[group][sound]`) and of the master since the last
    /// call (reset).
    pub fn take_meters(&self) -> (Meters, (f32, f32)) {
        self.telemetry(|t| {
            let meters = t.meters.clone();
            for row in &mut t.meters {
                row.fill((0.0, 0.0));
            }
            (meters, std::mem::take(&mut t.master_peak))
        })
    }

    /// Where zone `zone` of a sound plays, as last published; it is watched from now
    /// on (the last few zones asked about are).
    pub fn zone_position(&self, group: usize, sound: usize, zone: usize) -> Option<usize> {
        let key = (group, sound, zone);
        self.telemetry(|t| match t.watch.iter().position(|w| *w == key) {
            Some(i) => t.zones.get(i).copied().flatten(),
            None => {
                if t.watch.len() >= 8 {
                    t.watch.remove(0);
                    t.zones.remove(0);
                }
                t.watch.push(key);
                // Sized here, so the audio thread doesn't allocate.
                t.zones.resize(t.watch.len(), None);
                None
            }
        })
    }

    /// Run `f` on the playing engine at its next block, without waiting (it goes
    /// back to be freed here). For decisions that need the engine's state at that
    /// moment.
    pub fn run(&self, f: impl FnMut(&mut Engine) + Send + 'static) {
        self.sender.send(Command::Run(Box::new(f)));
    }

    /// Run `f` on the playing engine (on the audio thread, at its next block) and
    /// return what it returns; `None` if no engine plays or the audio thread doesn't
    /// answer within a second. For queries and tests: it waits for the audio thread.
    pub fn call<R: Send + 'static>(&self, f: impl FnOnce(&mut Engine) -> R + Send + 'static) -> Option<R> {
        if !self.has_engine() {
            return None;
        }
        let (tx, rx) = std::sync::mpsc::sync_channel::<R>(1);
        let mut f = Some(f);
        let run = move |e: &mut Engine| {
            if let Some(f) = f.take() {
                let _ = tx.try_send(f(e));
            }
        };
        if !self.sender.send(Command::Run(Box::new(run))) {
            return None;
        }
        let r = rx.recv_timeout(Duration::from_secs(1)).ok();
        self.sender.collect();
        r
    }

    /// Whether the published state has caught up with everything sent to the engine
    /// (a scene queued, a seek): until then it shows how things were before.
    pub fn is_current(&self) -> bool {
        let sent = self.sender.sent.load(Ordering::Acquire);
        self.telemetry(|t| t.actions >= sent)
    }

    /// Whether anything is still sounding.
    pub fn is_playing(&self) -> bool {
        self.has_engine() && self.telemetry(|t| t.has_engine && !t.finished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{Clock, ClockOut, Follow};

    /// A new engine played over one with an MPE note sounding (Restart, Play): the old
    /// note ends and its channel goes neutral, so nothing sticks.
    #[test]
    fn playing_a_new_engine_ends_the_old_ones_midi() {
        let mut p = crate::tests::project();
        p.groups[0].sounds[0].channel.midi_out = deckard_model::MidiDest::Port("Synth".into());
        let (tx, rx) = std::sync::mpsc::sync_channel(4096);
        let mut old = crate::tests::click_engine_for(&p, 1000);
        old.midi_tx = Some(tx.clone());
        old.set_mpe_out([("Synth".to_string(), 48)].into());
        let mut out = vec![0f32; 2];
        old.process(&mut out);
        let on: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
        assert!(on.contains(&vec![0x91, 60, 127]), "{on:?}");
        let mut engine = Some(Box::new(old));
        let new = Box::new(crate::tests::click_engine_for(&p, 1000));
        apply(&mut engine, Command::Play(new), Instant::now(), Duration::from_millis(10), 1000.0, 1, false);
        let got: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
        assert_eq!(got, vec![vec![0x81, 60, 0], vec![0xe1, 0, 64], vec![0xb1, 74, 64], vec![0xd1, 0]]);
    }

    /// A clock always at 90 BPM, 1 beat into the bar, that notes what it was asked.
    struct Fixed(Arc<Mutex<Vec<(Duration, f64)>>>);

    impl Clock for Fixed {
        fn follow(&mut self, ahead: Duration, quantum: f64) -> Option<Follow> {
            self.0.lock().unwrap().push((ahead, quantum));
            Some(Follow { tempo: 90.0, beat: 1.0, absolute: false })
        }
    }

    fn engine() -> Box<Engine> {
        Box::new(crate::tests::click_engine_for(&crate::tests::project(), 48_000))
    }

    /// Each block, the clock is asked where it is when the block is heard (on the song's
    /// bar), and the engine follows it.
    #[test]
    fn the_engine_follows_the_clock_each_block() {
        let asked = Arc::default();
        let mut clock: Option<Box<dyn Clock>> = Some(Box::new(Fixed(Arc::clone(&asked))));
        let mut e = engine();
        before_block(&mut e, &mut clock, Instant::now(), Duration::from_millis(5));
        assert_eq!(e.tempo(), 90.0);
        assert_eq!(*asked.lock().unwrap(), [(Duration::from_millis(5), 4.0)]);
    }

    /// Clock-out settings reach the engine; the old ones come back to be freed.
    #[test]
    fn clock_out_reaches_the_engine() {
        let mut e = Some(engine());
        let out = ClockOut { ports: vec!["A".into()], early: Duration::ZERO };
        let now = Instant::now();
        let back = apply(&mut e, Command::ClockOut(Some(out.clone())), now, Duration::ZERO, 48_000.0, 256, true);
        assert!(matches!(back, Some(Command::ClockOut(None))));
        let back = apply(&mut e, Command::ClockOut(None), now, Duration::ZERO, 48_000.0, 256, true);
        assert!(matches!(back, Some(Command::ClockOut(Some(o))) if o == out));
    }
}
