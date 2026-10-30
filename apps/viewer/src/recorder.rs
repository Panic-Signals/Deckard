//! Sampling ▸ RECORD, as Maschine 3's recorder (docs/recording-software.md): the
//! source is an audio interface's input (Ext. Ster. / Ext. Mono, as routed in
//! Preferences ▸ Audio) or audio from inside the engine (Internal: the master or a
//! group; Auto: one sound). Modes:
//! - Detect: starts when the input passes the threshold (10 ms of pre-roll kept), or at
//!   once with the threshold OFF; Stop ends it.
//! - Sync: starts at the next bar while the song plays, for Length bars (FREE: until
//!   Stop, which ends it at the next bar).
//! - Loop: as Sync, on the focused pattern's length.
//! - Auto: plays each note × velocity of the Note and Velocity maps to an instrument
//!   (a sound here, or a MIDI port) and records each into its own file.
//!
//! All sources go through [`Recorder::poll`] (every frame), which reads what arrived,
//! so the timing logic is the same for each. The input stream (or the engine's tap) is
//! open only while armed.

use crate::layout::sampling::{AutoParams, RecStatus, RecorderView};
use cpal::traits::{DeviceTrait, StreamTrait};
use deckard_engine::output::Output;
use deckard_engine::sample::Sample;
use deckard_engine::{Tap, TapSource};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SOURCES: [&str; 3] = ["Ext. Ster.", "Ext. Mono", "Internal"];
pub const MODES: [&str; 4] = ["Detect", "Sync", "Loop", "Auto"];
pub const LENGTHS: [&str; 6] = ["1 Bar", "2 Bars", "4 Bars", "8 Bars", "16 Bars", "FREE"];
pub const TARGETS: [&str; 3] = ["Take", "Sound", "Pattern"];
/// Maschine's inputs: 16 stereo pairs (32 channels).
pub const INPUT_PAIRS: usize = 16;
/// The threshold at or below which Detect is OFF (starts at once).
pub const THRESHOLD_OFF: f64 = -59.99;

/// `In k L+R`.
pub fn stereo_input_name(i: usize) -> String {
    format!("In {} L+R", i + 1)
}

/// `In k L` / `In k R`.
pub fn mono_input_name(i: usize) -> String {
    format!("In {} {}", i / 2 + 1, if i.is_multiple_of(2) { "L" } else { "R" })
}

/// Where playback is, for Sync and Loop (ticks).
#[derive(Debug, Clone, Copy, Default)]
pub struct Clock {
    pub playing: bool,
    /// The position in the playing timeline, and the timeline's length (it loops).
    pub position: f64,
    pub length: f64,
    pub tempo: f64,
    /// Recording starts on multiples of this: a bar, or (Loop) the pattern length.
    pub grid: f64,
    pub bar: f64,
}

impl Clock {
    fn frames(&self, ticks: f64, rate: u32) -> f64 {
        ticks / deckard_model::PPQ as f64 * 60.0 / self.tempo.max(1.0) * rate as f64
    }

    fn ticks(&self, frames: usize, rate: u32) -> f64 {
        frames as f64 / rate.max(1) as f64 * self.tempo.max(1.0) / 60.0 * deckard_model::PPQ as f64
    }
}

/// One note of an Auto Sampler run: the note and velocity played, the keys and
/// velocities its zone covers, and its file.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoItem {
    pub note: u8,
    pub velocity: u8,
    pub keys: (u8, u8),
    pub vels: (u8, u8),
    pub path: PathBuf,
}

/// A note an Auto run recorded: its item, its length (frames) and the loop found in it.
pub type AutoTake = (AutoItem, usize, Option<(usize, usize)>);

/// A finished recording.
pub enum Finished {
    /// Detect, Sync or Loop: the take, its channels (1 for Ext. Mono), the mode, and its
    /// length in ticks at the song tempo.
    Take { sample: Sample, channels: u16, mode: usize, ticks: f64 },
    /// Auto: each recorded note (written already), its length in frames and the loop
    /// found in it.
    Auto { done: Vec<AutoTake> },
}

/// What the recorder needs to start.
pub struct StartWith<'a> {
    pub output: Option<&'a Output>,
    /// The audio interface (Preferences ▸ Audio), `None`: the default input; and which
    /// of its channels each of Maschine's 32 input channels reads.
    pub device: Option<String>,
    pub routing: &'a [Option<usize>],
    pub clock: Clock,
    /// Auto: the folder and name stem of its files.
    pub dir: PathBuf,
    pub stem: String,
}

/// An Auto run in progress.
struct AutoRun {
    items: Vec<AutoItem>,
    i: usize,
    /// The frame (of the source) at which the current note sounds, and when to send
    /// its note-off (MIDI).
    start: Option<u64>,
    note_off: Option<(Instant, u8)>,
    sample_frames: usize,
    note_on_frames: usize,
    params: AutoParams,
    done: Vec<AutoTake>,
}

struct Run {
    rate: u32,
    channels: u16,
    /// Frames that have arrived from the source, and the recent ones (pre-roll, and
    /// the frames before a bar line that was crossed between two polls).
    received: u64,
    ring: VecDeque<[f32; 2]>,
    /// The take, and where it ends (frames), if it has a length.
    frames: Vec<[f32; 2]>,
    limit: Option<usize>,
    /// Sync/Loop: the position counting loops of the timeline, the line to start at,
    /// the line to stop at (FREE, after Stop), and where the take started.
    abs: Option<f64>,
    last_pos: f64,
    arm_line: Option<f64>,
    stop_line: Option<f64>,
    started: f64,
    auto: Option<AutoRun>,
    /// Auto: the frames from `base` on.
    pending: Vec<[f32; 2]>,
    base: u64,
}

/// What the idle meter listens to: the source, its external inputs, the interface,
/// monitoring, and whether an engine runs (to tap, for the internal source).
type MeterKey = (usize, usize, usize, Option<usize>, Option<String>, bool, bool);

#[derive(Default)]
pub struct Recorder {
    pub view: RecorderView,
    incoming: Arc<Mutex<Vec<[f32; 2]>>>,
    stream: Option<cpal::Stream>,
    tap: Option<Arc<Mutex<Vec<f32>>>>,
    run: Option<Run>,
    midi: Option<midir::MidiOutputConnection>,
    /// The source the input meter listens to while nothing is recorded.
    meter_key: Option<MeterKey>,
}

enum Done {
    Take(Finished),
    /// Ended without anything to keep.
    Ended,
}

impl Recorder {
    pub fn new() -> Recorder {
        let mut r = Recorder::default();
        r.view.threshold = -12.0;
        r.view.mode = 1;
        r.view.auto = AutoParams::default();
        r.scan_midi();
        r
    }

    /// Look up the MIDI outputs (Auto ▸ MIDI To).
    pub fn scan_midi(&mut self) {
        self.view.midi_outs =
            midir::MidiOutput::new("DECKARD").map(|m| m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect()).unwrap_or_default();
    }

    /// Armed or recording: the parameters can't change.
    pub fn busy(&self) -> bool {
        self.run.is_some()
    }

    /// Whether START can be pressed: Auto needs something to play.
    pub fn can_start(&self) -> bool {
        let v = &self.view;
        v.mode != 3 || if v.source == 2 { v.auto_sound.is_some() } else { v.auto.midi_to.is_some() }
    }

    /// Meter the chosen source while nothing is recorded (`on`: the RECORD tab shows),
    /// so its level shows before recording; it follows changes of source. `on: false`
    /// lets the input go.
    pub fn meter(&mut self, on: bool, with: StartWith) {
        if self.run.is_some() {
            return;
        }
        let v = &self.view;
        let engine = with.output.is_some_and(Output::has_engine);
        let key = (v.source, v.ext_stereo, v.ext_mono, v.internal, with.device.clone(), v.monitor, engine);
        if on && self.meter_key.as_ref() == Some(&key) {
            return;
        }
        if self.meter_key.take().is_some() {
            self.close_with(with.output);
        }
        if !on {
            return;
        }
        // Kept even if it fails, so it isn't tried again every frame.
        self.meter_key = Some(key);
        if self.view.source == 2 {
            if let Some(o) = with.output.filter(|o| o.has_engine()) {
                let buffer = Arc::new(Mutex::new(Vec::new()));
                let source = self.view.internal.map_or(TapSource::Master, TapSource::Group);
                o.set_tap(Some(Tap { source, buffer: buffer.clone() }));
                self.tap = Some(buffer);
            }
        } else if let Err(e) = self.open_input(&with) {
            self.view.error = Some(e);
        }
    }

    /// Arm (Detect, Sync, Loop) or start (Detect with the threshold OFF, Auto).
    pub fn start(&mut self, with: StartWith) {
        if self.run.is_some() || !self.can_start() {
            return;
        }
        // The meter's input makes way (the take opens its own).
        if self.meter_key.take().is_some() {
            self.close_with(with.output);
        }
        self.view.error = None;
        let internal = self.view.source == 2;
        let rate = match (internal, with.output) {
            (true, Some(out)) => out.rate,
            (true, None) => {
                self.view.error = Some("no audio output to record from".into());
                return;
            }
            (false, _) => match self.open_input(&with) {
                Ok(rate) => rate,
                Err(e) => {
                    self.view.error = Some(e);
                    return;
                }
            },
        };
        if internal {
            let source = if self.view.mode == 3 {
                let (g, s) = self.view.auto_sound.unwrap_or_default();
                TapSource::Sound(g, s)
            } else {
                match self.view.internal {
                    Some(g) => TapSource::Group(g),
                    None => TapSource::Master,
                }
            };
            let buffer = Arc::new(Mutex::new(Vec::new()));
            match with.output.filter(|o| o.has_engine()) {
                Some(o) => o.set_tap(Some(Tap { source, buffer: buffer.clone() })),
                None => {
                    self.view.error = Some("nothing is playing to record from".into());
                    return;
                }
            }
            self.tap = Some(buffer);
        }
        let mut run = Run {
            rate,
            channels: if self.view.source == 1 { 1 } else { 2 },
            received: 0,
            ring: VecDeque::new(),
            frames: Vec::new(),
            limit: None,
            abs: None,
            last_pos: 0.0,
            arm_line: None,
            stop_line: None,
            started: 0.0,
            auto: None,
            pending: Vec::new(),
            base: 0,
        };
        self.view.rate = rate;
        self.view.recorded_frames = 0;
        self.view.preview.clear();
        self.view.status = RecStatus::Waiting;
        match self.view.mode {
            0 if self.view.threshold <= THRESHOLD_OFF => self.view.status = RecStatus::Recording,
            3 => {
                let p = self.view.auto.clone();
                let items = auto_items(&p, &with.dir, &with.stem);
                if items.is_empty() {
                    self.view.error = Some("no notes to sample (Key Lo/Hi)".into());
                    self.close_with(with.output);
                    return;
                }
                if !internal {
                    match connect_midi(p.midi_to.as_deref().unwrap_or_default()) {
                        Ok(c) => self.midi = Some(c),
                        Err(e) => {
                            self.view.error = Some(e);
                            self.close_with(with.output);
                            return;
                        }
                    }
                }
                let ms = |v: u32| (v as u64 * rate as u64 / 1000) as usize;
                run.auto = Some(AutoRun {
                    items,
                    i: 0,
                    start: None,
                    note_off: None,
                    sample_frames: ms(p.sample_ms),
                    note_on_frames: ms(p.note_on_ms),
                    params: p,
                    done: Vec::new(),
                });
                self.view.status = RecStatus::Recording;
            }
            _ => {}
        }
        self.run = Some(run);
        self.view.info = self.info(&with.clock);
    }

    /// Open the interface input for the chosen Ext. input; returns its rate.
    fn open_input(&mut self, with: &StartWith) -> Result<u32, String> {
        let (device, config) = deckard_engine::devices::open_input(with.device.as_deref())?;
        if config.sample_format() != cpal::SampleFormat::F32 {
            return Err(format!("input format {:?} is not supported", config.sample_format()));
        }
        let channels = config.channels() as usize;
        let route = |i: usize| with.routing.get(i).copied().unwrap_or(Some(i)).filter(|&c| c < channels);
        let (l, r) = if self.view.source == 1 {
            let c = route(self.view.ext_mono);
            (c, c)
        } else {
            (route(self.view.ext_stereo * 2), route(self.view.ext_stereo * 2 + 1))
        };
        if l.is_none() && r.is_none() {
            let name = if self.view.source == 1 { mono_input_name(self.view.ext_mono) } else { stereo_input_name(self.view.ext_stereo) };
            return Err(format!("{name} isn't connected to a channel of {} (Preferences > Audio > Routing)", device_name(&device)));
        }
        let incoming = self.incoming.clone();
        incoming.lock().unwrap().clear();
        let monitor = self.view.monitor.then(|| with.output.map(Output::monitor)).flatten();
        let rate = config.sample_rate();
        let stream = device
            .build_input_stream(
                config.into(),
                move |data: &[f32], _| {
                    let at = |f: &[f32], c: Option<usize>| c.map_or(0.0, |c| f[c]);
                    if let Ok(mut inc) = incoming.try_lock() {
                        inc.extend(data.chunks(channels).map(|f| [at(f, l), at(f, r)]));
                    }
                    if let Some(m) = &monitor
                        && let Ok(mut m) = m.try_lock()
                    {
                        for f in data.chunks(channels) {
                            m.push(at(f, l));
                            m.push(at(f, r));
                        }
                    }
                },
                |e| eprintln!("audio input error: {e}"),
                None,
            )
            .map_err(|e| format!("audio input: {e}"))?;
        stream.play().map_err(|e| format!("audio input: {e}"))?;
        self.stream = Some(stream);
        Ok(rate)
    }

    /// Stop: Detect ends now; Sync and Loop at the next bar line (FREE) or their end;
    /// Auto keeps the notes recorded so far.
    pub fn stop(&mut self, output: Option<&Output>, clock: Clock) -> Option<Finished> {
        let run = self.run.as_mut()?;
        match self.view.mode {
            1 | 2 if self.view.status == RecStatus::Recording => {
                if run.stop_line.is_none() {
                    let abs = run.abs.unwrap_or(clock.position);
                    let bar = clock.bar.max(1.0);
                    run.stop_line = Some(((abs / bar).floor() + 1.0) * bar);
                }
                self.view.info = self.info(&clock);
                None
            }
            1 | 2 => {
                self.cancel(output);
                None
            }
            3 => {
                let done = run.auto.as_mut().map(|a| std::mem::take(&mut a.done)).unwrap_or_default();
                self.cancel(output);
                (!done.is_empty()).then_some(Finished::Auto { done })
            }
            _ => {
                let finished = take(run, 0, &clock);
                self.close_with(output);
                match finished {
                    Done::Take(f) => Some(f),
                    Done::Ended => None,
                }
            }
        }
    }

    /// Throw away what is being recorded.
    pub fn cancel(&mut self, output: Option<&Output>) {
        if let (Some(conn), Some(a)) = (self.midi.as_mut(), self.run.as_ref().and_then(|r| r.auto.as_ref()))
            && let Some((_, n)) = a.note_off
        {
            let _ = conn.send(&[0x80 | (a.params.channel.clamp(1, 16) - 1), n, 0]);
        }
        self.close_with(output);
    }

    fn close_with(&mut self, output: Option<&Output>) {
        if self.tap.take().is_some()
            && let Some(o) = output
        {
            o.set_tap(None);
        }
        self.stream = None;
        self.run = None;
        self.midi = None;
        self.view.status = RecStatus::Idle;
        self.view.info.clear();
    }

    /// Take up what arrived (call every frame). Returns a finished recording.
    pub fn poll(&mut self, output: Option<&Output>, clock: Clock) -> Option<Finished> {
        let new: Vec<[f32; 2]> = if let Some(tap) = &self.tap {
            let raw = std::mem::take(&mut *tap.lock().unwrap());
            raw.as_chunks::<2>().0.to_vec()
        } else if self.stream.is_some() {
            std::mem::take(&mut *self.incoming.lock().unwrap())
        } else {
            self.view.meter = (self.view.meter.0 * 0.8, self.view.meter.1 * 0.8);
            return None;
        };
        let peak = new.iter().fold((0f32, 0f32), |p, f| (p.0.max(f[0].abs()), p.1.max(f[1].abs())));
        self.view.meter = ((self.view.meter.0 * 0.8).max(peak.0), (self.view.meter.1 * 0.8).max(peak.1));
        if self.run.is_none() {
            // Only metering.
            return None;
        }
        let done = match self.view.mode {
            3 => self.poll_auto(output, new),
            0 => self.poll_detect(new),
            _ => self.poll_sync(clock, new),
        };
        if let Some(run) = &self.run {
            self.view.recorded_frames = if run.auto.is_some() { run.pending.len() } else { run.frames.len() };
            let shown: Vec<[f32; 2]> = match self.view.status {
                RecStatus::Waiting => run.ring.iter().copied().collect(),
                _ if run.auto.is_some() => run.pending.clone(),
                _ => run.frames[run.frames.len().saturating_sub(run.rate as usize * 5)..].to_vec(),
            };
            self.view.preview = preview(&shown);
        }
        self.view.info = self.info(&clock);
        match done {
            Some(Done::Take(f)) => {
                self.close_with(output);
                Some(f)
            }
            Some(Done::Ended) => {
                self.close_with(output);
                None
            }
            None => None,
        }
    }

    fn poll_detect(&mut self, new: Vec<[f32; 2]>) -> Option<Done> {
        let run = self.run.as_mut()?;
        run.received += new.len() as u64;
        if self.view.status == RecStatus::Recording {
            run.frames.extend(new);
            return None;
        }
        let thr = 10f32.powf(self.view.threshold as f32 / 20.0);
        match new.iter().position(|f| f[0].abs().max(f[1].abs()) >= thr) {
            Some(i) => {
                // 10 ms of pre-roll.
                let pre = run.rate as usize / 100;
                let mut all: Vec<[f32; 2]> = run.ring.drain(..).collect();
                let at = all.len() + i;
                all.extend(new);
                run.frames = all[at.saturating_sub(pre)..].to_vec();
                self.view.status = RecStatus::Recording;
            }
            None => keep_recent(&mut run.ring, new, run.rate as usize / 100 + 1),
        }
        None
    }

    fn poll_sync(&mut self, clock: Clock, new: Vec<[f32; 2]>) -> Option<Done> {
        let mode = self.view.mode;
        let length = self.view.length;
        let run = self.run.as_mut()?;
        run.received += new.len() as u64;
        if !clock.playing {
            run.abs = None;
            // The take ends with the song.
            if self.view.status == RecStatus::Recording {
                run.frames.extend(new);
                return Some(take(run, mode, &clock));
            }
            keep_recent(&mut run.ring, new, run.rate as usize * 2);
            return None;
        }
        // The position, counting the timeline's loops.
        let abs = match run.abs {
            None => clock.position,
            Some(prev) => {
                let mut d = clock.position - run.last_pos;
                if d < 0.0 && clock.length > 0.0 {
                    d += clock.length;
                }
                prev + d.max(0.0)
            }
        };
        let first = run.abs.is_none();
        run.abs = Some(abs);
        run.last_pos = clock.position;
        if self.view.status == RecStatus::Waiting {
            let grid = clock.grid.max(1.0);
            if run.arm_line.is_none() {
                // Where the song was when these frames began (just started: its start).
                let before = if first { abs - clock.ticks(new.len(), run.rate) } else { abs };
                run.arm_line = Some((before / grid - 1e-6).ceil().max(0.0) * grid);
            }
            let line = run.arm_line.unwrap_or(0.0);
            let mut all: Vec<[f32; 2]> = run.ring.drain(..).collect();
            all.extend(new);
            if abs < line {
                keep_recent(&mut run.ring, all, run.rate as usize * 2);
                return None;
            }
            let over = (clock.frames(abs - line, run.rate).round() as usize).min(all.len());
            run.frames = all[all.len() - over..].to_vec();
            run.started = line;
            run.limit = (length < 5).then(|| clock.frames(clock.bar * (1u32 << length) as f64, run.rate).round() as usize);
            self.view.status = RecStatus::Recording;
        } else {
            run.frames.extend(new);
        }
        if let Some(stop) = run.stop_line {
            let n = clock.frames(stop - run.started, run.rate).round().max(1.0) as usize;
            run.limit = Some(run.limit.map_or(n, |l| l.min(n)));
        }
        if let Some(n) = run.limit
            && run.frames.len() >= n
        {
            run.frames.truncate(n);
            return Some(take(run, mode, &clock));
        }
        None
    }

    fn poll_auto(&mut self, output: Option<&Output>, new: Vec<[f32; 2]>) -> Option<Done> {
        let internal = self.view.source == 2;
        let target = self.view.auto_sound;
        let run = self.run.as_mut()?;
        let rate = run.rate;
        run.received += new.len() as u64;
        run.pending.extend(new);
        let a = run.auto.as_mut()?;
        let channel = a.params.channel.clamp(1, 16) - 1;
        // MIDI note-off when the note's time is up.
        if let Some((at, n)) = a.note_off
            && Instant::now() >= at
            && let Some(c) = self.midi.as_mut()
        {
            let _ = c.send(&[0x80 | channel, n, 0]);
            a.note_off = None;
        }
        let Some(start) = a.start else {
            // Play the next note; its recording starts where it sounds.
            let item = a.items[a.i].clone();
            let start = if internal {
                let (g, s) = target?;
                let seconds = a.params.note_on_ms as f64 / 1000.0;
                // On the audio thread, at the start of a block: the tap's frame count
                // there is where the note sounds.
                let (note, velocity) = (item.note as i64, item.velocity as i64);
                output?.call(move |e| {
                    let ticks = seconds * e.tempo() / 60.0 * deckard_model::PPQ as f64;
                    e.note_on(g, s, note, velocity, ticks.max(1.0) as i64);
                    e.tap_frames()
                })?
            } else {
                let c = self.midi.as_mut()?;
                if c.send(&[0x90 | channel, item.note, item.velocity.max(1)]).is_err() {
                    self.view.error = Some("can't send to the MIDI output".into());
                    return Some(Done::Ended);
                }
                a.note_off = Some((Instant::now() + Duration::from_millis(a.params.note_on_ms as u64), item.note));
                run.received
            };
            // Frames before the note aren't needed.
            let drop = (start.saturating_sub(run.base) as usize).min(run.pending.len());
            run.pending.drain(..drop);
            run.base += drop as u64;
            a.start = Some(start);
            return None;
        };
        let have = (run.base + run.pending.len() as u64).saturating_sub(start) as usize;
        if have < a.sample_frames {
            return None;
        }
        // This note is done: process and write it.
        let from = start.saturating_sub(run.base) as usize;
        let frames = run.pending[from..from + a.sample_frames].to_vec();
        run.pending.drain(..from + a.sample_frames);
        run.base = start + a.sample_frames as u64;
        let item = a.items[a.i].clone();
        let mut s = Sample { rate, frames };
        let mut held = a.note_on_frames.min(s.frames.len());
        if a.params.normalize {
            s = deckard_engine::edit::normalize(&s, (0, s.frames.len()));
        }
        if a.params.trim_silence {
            let lead = s.frames.iter().position(|f| f[0].abs().max(f[1].abs()) > 0.001).unwrap_or(0);
            s = deckard_engine::edit::trim_silence(&s);
            held = held.saturating_sub(lead).min(s.frames.len());
        }
        let lp = if a.params.find_loop { deckard_engine::edit::find_loop(&s, (0, held)) } else { None };
        if let Err(e) = deckard_engine::edit::write_wav_channels(&item.path, &s, run.channels) {
            self.view.error = Some(format!("can't save {}: {e}", item.path.display()));
            return Some(Done::Ended);
        }
        a.done.push((item, s.frames.len(), lp));
        a.i += 1;
        a.start = None;
        if a.i >= a.items.len() {
            let done = std::mem::take(&mut a.done);
            return Some(Done::Take(Finished::Auto { done }));
        }
        None
    }

    /// The info bar's status text, as Maschine's.
    fn info(&self, clock: &Clock) -> String {
        let Some(run) = &self.run else { return String::new() };
        if let Some(a) = &run.auto {
            let it = &a.items[a.i.min(a.items.len() - 1)];
            return format!("Recording {} at {} ({}/{})", note_name(it.note), it.velocity, a.i + 1, a.items.len());
        }
        match (self.view.status, self.view.mode) {
            (RecStatus::Waiting, 0) => "Waiting for input...".into(),
            (RecStatus::Waiting, _) if !clock.playing => "Waiting for next bar... (press Play)".into(),
            (RecStatus::Waiting, 2) if clock.grid > clock.bar => "Waiting for end of Pattern...".into(),
            (RecStatus::Waiting, _) => "Waiting for next bar...".into(),
            _ if run.stop_line.is_some() => "Recording... (stops at the next bar)".into(),
            _ => "Recording...".into(),
        }
    }
}

fn take(run: &mut Run, mode: usize, clock: &Clock) -> Done {
    let frames = std::mem::take(&mut run.frames);
    if frames.is_empty() {
        return Done::Ended;
    }
    Done::Take(Finished::Take {
        ticks: clock.ticks(frames.len(), run.rate),
        sample: Sample { rate: run.rate, frames },
        channels: run.channels,
        mode,
    })
}

/// Keep the last `n` frames of `ring` + `new` in `ring`.
fn keep_recent(ring: &mut VecDeque<[f32; 2]>, new: Vec<[f32; 2]>, n: usize) {
    ring.extend(new);
    while ring.len() > n {
        ring.pop_front();
    }
}

/// (min, max) of the mid signal in up to 400 columns.
fn preview(frames: &[[f32; 2]]) -> Vec<(f32, f32)> {
    let per = (frames.len() / 400).max(1);
    frames
        .chunks(per)
        .map(|c| {
            c.iter().fold((0f32, 0f32), |(lo, hi), f| {
                let m = (f[0] + f[1]) * 0.5;
                (lo.min(m), hi.max(m))
            })
        })
        .collect()
}

fn connect_midi(name: &str) -> Result<midir::MidiOutputConnection, String> {
    let out = midir::MidiOutput::new("DECKARD").map_err(|e| e.to_string())?;
    let port = out
        .ports()
        .into_iter()
        .find(|p| out.port_name(p).is_ok_and(|n| n == name))
        .ok_or_else(|| format!("the MIDI output \"{name}\" isn't there"))?;
    out.connect(&port, "DECKARD Auto Sampler").map_err(|e| e.to_string())
}

pub use deckard_engine::devices::{default_input, device_name, input_devices};

/// A note's name as Maschine shows it: C3 = 60, C-2 = 0.
pub fn note_name(n: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[n as usize % 12], n as i32 / 12 - 2)
}

/// Maschine's zones for a stride over lo..=hi (`getRangesWithRootFromProperties`):
/// (root, low, high) each, roughly centred on the root; Extend stretches the first
/// down to 0 and the last up to 127.
pub fn ranges_with_root(stride: u32, lo: u8, hi: u8, extend: bool) -> Vec<(u8, u8, u8)> {
    let (lo, hi) = (lo as i64, hi as i64);
    if stride == 0 || hi < lo || lo > 127 || hi > 127 {
        return Vec::new();
    }
    if lo == hi {
        return vec![(hi as u8, if extend { 0 } else { hi as u8 }, if extend { 127 } else { hi as u8 })];
    }
    let s = (stride as i64).min(hi - lo);
    let mut start = (lo - (s - 1) / 2).max(0);
    let mut root = lo;
    let mut out = Vec::new();
    loop {
        root = root.min(127);
        let up = (root + s / 2).min(127);
        out.push((root as u8, start.min(up) as u8, start.max(up) as u8));
        start = up + 1;
        root += s;
        if up >= hi {
            break;
        }
    }
    if extend {
        out[0].1 = 0;
        let n = out.len() - 1;
        out[n].2 = 127;
    }
    out
}

/// The notes of an Auto run, note by note (each note's velocity layers in turn), with
/// their files `<dir>/<stem> <Note> <velocity>.wav`.
pub fn auto_items(p: &AutoParams, dir: &Path, stem: &str) -> Vec<AutoItem> {
    let keys = ranges_with_root(p.key_stride, p.key_lo, p.key_hi, p.key_extend);
    let vels = ranges_with_root(p.vel_stride, p.vel_lo, p.vel_hi, p.vel_extend);
    let mut out = Vec::new();
    for k in &keys {
        for v in &vels {
            let base = format!("{stem} {} {}", note_name(k.0), v.0);
            let mut path = dir.join(format!("{base}.wav"));
            let mut n = 2;
            while path.exists() {
                path = dir.join(format!("{base} {n}.wav"));
                n += 1;
            }
            out.push(AutoItem { note: k.0, velocity: v.0, keys: (k.1, k.2), vels: (v.1, v.2), path });
        }
    }
    out
}

/// `YYYYMMDDTHHMMSS.wav` for now (local time), as Maschine names recordings.
pub fn take_name() -> String {
    format!("{}.wav", timestamp())
}

/// `YYYYMMDDTHHMMSS` for now, in local time.
pub fn timestamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let secs = secs + local_offset(secs);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// The local time zone's offset from UTC at `secs` (seconds).
#[cfg(unix)]
fn local_offset(secs: i64) -> i64 {
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: std::ffi::c_long,
        zone: *const std::ffi::c_char,
    }
    unsafe extern "C" {
        fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    }
    let mut tm = std::mem::MaybeUninit::<Tm>::zeroed();
    // SAFETY: localtime_r only writes the struct it is given.
    let ok = unsafe { !localtime_r(&secs, tm.as_mut_ptr()).is_null() };
    if ok { unsafe { tm.assume_init() }.gmtoff } else { 0 }
}

/// The same from the C runtime: the local time read back as if it were UTC.
#[cfg(windows)]
fn local_offset(secs: i64) -> i64 {
    #[repr(C)]
    #[derive(Default)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
    }
    unsafe extern "C" {
        fn _localtime64_s(out: *mut Tm, t: *const i64) -> i32;
        fn _mkgmtime64(tm: *mut Tm) -> i64;
    }
    let mut tm = Tm::default();
    // SAFETY: both only read and write the struct and time they are given.
    unsafe {
        if _localtime64_s(&mut tm, &secs) != 0 {
            return 0;
        }
        let local = _mkgmtime64(&mut tm);
        if local == -1 { 0 } else { local - secs }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::layout::sampling::RecParam;
    use deckard_engine::{Engine, Mode, SampleCache};

    /// The test project playing (or ready to play notes) through the real output, with
    /// the master turned down (taps before the master still hear it).
    fn playing(sequencing: bool) -> Option<(Output, deckard_model::Project)> {
        let out = Output::new().ok()?;
        let path = std::path::Path::new("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj");
        let doc = deckard_doc::Document::open(path).ok()?;
        let p = doc.project().clone();
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, Some(path), out.rate, Mode::Arrangement, &mut cache);
        e.sequencing = sequencing;
        e.master = 0.0;
        out.play(e);
        Some((out, p))
    }

    fn clock_of(out: &Output, p: &deckard_model::Project) -> Clock {
        let (playing, position, length) = out.call(|e| (e.sequencing, e.position(), e.timeline().length as f64)).unwrap();
        let bar = p.bar_length() as f64;
        Clock { playing, position, length, tempo: p.tempo, grid: bar, bar }
    }

    #[test]
    fn auto_samples_a_sound_inside_the_engine() {
        let Some((out, p)) = playing(false) else { return };
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("auto");
        let mut r = Recorder::new();
        for (k, v) in [
            (RecParam::Mode, 3),
            (RecParam::Source, 2),
            (RecParam::AutoSound, 0),
            (RecParam::SampleMs, 300),
            (RecParam::NoteOnMs, 100),
            (RecParam::KeyHi, 62),
            (RecParam::KeyLo, 60),
            (RecParam::KeyStride, 2),
        ] {
            r.view.set(k, v);
        }
        assert!(r.can_start());
        r.start(StartWith {
            output: Some(&out),
            device: None,
            routing: &[],
            clock: clock_of(&out, &p),
            dir: dir.clone(),
            stem: "Kick".into(),
        });
        assert_eq!(r.view.error, None);
        let t0 = Instant::now();
        let done = loop {
            std::thread::sleep(Duration::from_millis(15));
            if let Some(f) = r.poll(Some(&out), clock_of(&out, &p)) {
                break f;
            }
            assert!(t0.elapsed() < Duration::from_secs(5), "{}", r.view.info);
        };
        let Finished::Auto { done } = done else { panic!("not an Auto result") };
        assert_eq!(done.iter().map(|d| (d.0.note, d.0.keys)).collect::<Vec<_>>(), vec![(60, (0, 61)), (62, (62, 127))]);
        for (item, frames, _) in &done {
            assert_eq!(*frames, out.rate as usize * 3 / 10, "{}", item.path.display());
            let s = Sample::load(&item.path).unwrap();
            let peak = s.frames.iter().fold(0f32, |m, f| m.max(f[0].abs()));
            assert!(peak > 0.05, "{} is silent ({peak})", item.path.display());
            // The kick starts at once: its note was recorded from where it sounded.
            let first = s.frames.iter().position(|f| f[0].abs() > 0.01).unwrap();
            assert!(first < out.rate as usize / 100, "{}: sound starts at frame {first}", item.path.display());
        }
        assert!(done[0].0.path.ends_with("Kick C3 100.wav"));
    }

    /// The meter shows the source's level while nothing is recorded, and lets it go
    /// when asked to.
    #[test]
    fn the_meter_moves_before_recording() {
        let Some((out, p)) = playing(true) else { return };
        let mut r = Recorder::new();
        r.view.set(RecParam::Source, 2);
        // A group: the test turns the master down.
        r.view.set(RecParam::Internal, 0);
        let with = |out| StartWith {
            output: Some(out),
            device: None,
            routing: &[],
            clock: clock_of(out, &p),
            dir: PathBuf::new(),
            stem: String::new(),
        };
        let t0 = Instant::now();
        while r.view.meter.0 < 0.01 {
            r.meter(true, with(&out));
            std::thread::sleep(Duration::from_millis(15));
            assert!(r.poll(Some(&out), clock_of(&out, &p)).is_none(), "metering records nothing");
            assert!(t0.elapsed() < Duration::from_secs(5), "the meter stays down");
        }
        assert!(!r.busy());
        assert_eq!(r.view.status, RecStatus::Idle);
        r.meter(false, with(&out));
        assert!(r.tap.is_none(), "let go");
    }

    #[test]
    fn sync_records_a_bar_of_a_group() {
        let Some((out, p)) = playing(true) else { return };
        let mut r = Recorder::new();
        r.view.set(RecParam::Mode, 1);
        r.view.set(RecParam::Source, 2);
        r.view.set(RecParam::Internal, 0);
        r.view.set(RecParam::Length, 0);
        r.start(StartWith {
            output: Some(&out),
            device: None,
            routing: &[],
            clock: clock_of(&out, &p),
            dir: std::env::temp_dir(),
            stem: String::new(),
        });
        let t0 = Instant::now();
        let f = loop {
            std::thread::sleep(Duration::from_millis(15));
            if let Some(f) = r.poll(Some(&out), clock_of(&out, &p)) {
                break f;
            }
            assert!(t0.elapsed() < Duration::from_secs(8), "{}", r.view.info);
        };
        out.stop();
        let Finished::Take { sample, mode, ticks, .. } = f else { panic!("not a take") };
        let bar = p.bar_length() as f64 / deckard_model::PPQ as f64 * 60.0 / p.tempo * out.rate as f64;
        assert_eq!(mode, 1);
        assert!((sample.frames.len() as f64 - bar).abs() <= 1.0, "{} frames, a bar is {bar}", sample.frames.len());
        assert!((ticks - p.bar_length() as f64).abs() < 1.0);
        assert!(sample.frames.iter().any(|f| f[0].abs() > 0.01), "the song is heard");
    }

    #[test]
    fn detect_starts_at_the_threshold_with_pre_roll() {
        let mut r = Recorder::new();
        r.view.set(RecParam::Mode, 0);
        r.view.threshold = -12.0;
        // Fed through a tap buffer, as an internal source would be.
        let buffer = Arc::new(Mutex::new(Vec::new()));
        r.tap = Some(buffer.clone());
        r.view.status = RecStatus::Waiting;
        r.run = Some(Run {
            rate: 1000,
            channels: 2,
            received: 0,
            ring: VecDeque::new(),
            frames: Vec::new(),
            limit: None,
            abs: None,
            last_pos: 0.0,
            arm_line: None,
            stop_line: None,
            started: 0.0,
            auto: None,
            pending: Vec::new(),
            base: 0,
        });
        let clock = Clock { tempo: 120.0, ..Clock::default() };
        // Quiet (below -12 dB), then loud from frame 50 of the second block.
        let push = |v: &[f32]| buffer.lock().unwrap().extend(v.iter().flat_map(|x| [*x, *x]));
        push(&[0.1; 100]);
        assert!(r.poll(None, clock).is_none());
        assert_eq!(r.view.status, RecStatus::Waiting);
        let mut block = vec![0.1f32; 50];
        block.extend([0.5f32; 50]);
        push(&block);
        r.poll(None, clock);
        assert_eq!(r.view.status, RecStatus::Recording);
        let Some(Finished::Take { sample, mode, .. }) = r.stop(None, clock) else { panic!("a take") };
        assert_eq!(mode, 0);
        // 10 ms of pre-roll (10 frames at 1 kHz) and the 50 loud frames.
        assert_eq!(sample.frames.len(), 60);
        assert_eq!(sample.frames[9][0], 0.1);
        assert_eq!(sample.frames[10][0], 0.5);
        assert_eq!(r.view.status, RecStatus::Idle);
    }

    #[test]
    fn take_names() {
        let n = take_name();
        assert_eq!(n.len(), "20260101T000000.wav".len());
        assert!(n.ends_with(".wav") && n.as_bytes()[8] == b'T');
    }

    #[test]
    fn zones_as_maschine_lays_them_out() {
        // The spec's examples.
        assert_eq!(ranges_with_root(12, 36, 48, false), vec![(36, 31, 42), (48, 43, 54)]);
        assert_eq!(ranges_with_root(12, 36, 48, true), vec![(36, 0, 42), (48, 43, 127)]);
        assert_eq!(ranges_with_root(3, 60, 65, false).iter().map(|r| r.0).collect::<Vec<_>>(), vec![60, 63, 66]);
        assert_eq!(ranges_with_root(1, 60, 60, true), vec![(60, 0, 127)]);
        assert_eq!(ranges_with_root(1, 60, 60, false), vec![(60, 60, 60)]);
        // Neighbouring zones touch.
        let r = ranges_with_root(4, 36, 60, false);
        for w in r.windows(2) {
            assert_eq!(w[0].2 + 1, w[1].1);
        }
    }

    #[test]
    fn auto_items_go_note_by_note() {
        let p = AutoParams { key_lo: 60, key_hi: 62, key_stride: 2, vel_lo: 64, vel_hi: 127, vel_stride: 63, ..AutoParams::default() };
        let items = auto_items(&p, &std::env::temp_dir().join("deckard-no-such-dir"), "Synth");
        let got: Vec<(u8, u8)> = items.iter().map(|i| (i.note, i.velocity)).collect();
        assert_eq!(got, vec![(60, 64), (60, 127), (62, 64), (62, 127)]);
        assert!(items[0].path.ends_with("Synth C3 64.wav"));
        assert_eq!(note_name(0), "C-2");
        assert_eq!(note_name(127), "G8");
    }
}
