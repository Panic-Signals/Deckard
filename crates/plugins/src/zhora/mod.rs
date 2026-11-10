//! Zhora: plays a modular synth voice over CV and keeps it in tune.

pub mod live;
pub mod pitch;
pub mod table;
pub mod tune;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use self::live::Trim;
use self::pitch::{Pitch, WINDOW, detect};
use self::table::Table;
use self::tune::{Outcome, Run, Step, note_hz};
use crate::native::{Class, Engine, Page, Param, Readout, TuneState};
use crate::replicant::{Env, EnvSettings, env_time, glide_time, step, time_text};
use crate::{MAX_BLOCK, Transport};

pub static CLASS: Class =
    Class { cid: *b"DECKARD:Zhora001", name: "Zhora", instrument: true, params: PARAMS, pages: PAGES, make: |sr| Box::new(Zhora::new(sr)) };

/// Whether `id` (a saved `plugin_id`) is Zhora.
pub fn is_zhora(id: &str) -> bool {
    id == CLASS.plugin_id()
}

// Parameter ids (saved in projects: never renumber).
const PITCH_OUT: u32 = 1;
const GATE_OUT: u32 = 2;
const CLOCK_OUT: u32 = 3;
const ENV_OUT: u32 = 4;
const VEL_OUT: u32 = 5;
const MOD_OUT: u32 = 6;
const ATTACK: u32 = 10;
const DECAY: u32 = 11;
const SUSTAIN: u32 = 12;
const RELEASE: u32 = 13;
const DIVISION: u32 = 20;
const CLOCK_LEVEL: u32 = 21;
const GATE_LEVEL: u32 = 22;
const ENV_LEVEL: u32 = 23;
const VEL_LEVEL: u32 = 24;
const MOD_LEVEL: u32 = 25;
const INPUT: u32 = 30;
const HOLD_NOTE: u32 = 31;
const LIVE: u32 = 32;
const GLIDE: u32 = 33;
const HOLD: u32 = 40;
const TUNE: u32 = 41;
const CHECK: u32 = 42;

const fn p(id: u32, name: &'static str, short: &'static str, default: f64, steps: i32) -> Param {
    Param { id, name, short, default, steps }
}

/// Steps of an output picker.
const OUT_STEPS: usize = 32;
/// Steps of the input picker.
const INPUT_STEPS: usize = 16;
/// The hold note's default: MIDI 69, which DECKARD names A3 (it calls note 60 C3).
const HOLD_DEFAULT: f64 = 69.0 / 127.0;
/// The clock divisions, and their lengths in quarter notes.
const DIVISIONS: [(&str, f64); 8] = [
    ("1/32", 1.0 / 8.0),
    ("1/16 T", 1.0 / 6.0),
    ("1/16", 1.0 / 4.0),
    ("1/8 T", 1.0 / 3.0),
    ("1/8", 1.0 / 2.0),
    ("1/4 T", 2.0 / 3.0),
    ("1/4", 1.0),
    ("1/2", 2.0),
];
const DIVISION_STEPS: usize = DIVISIONS.len() - 1;
const DEFAULT_DIVISION: f64 = 2.0 / DIVISION_STEPS as f64;

static PARAMS: &[Param] = &[
    p(PITCH_OUT, "Pitch Output", "PITCH", 0.0, OUT_STEPS as i32),
    p(GATE_OUT, "Gate Output", "GATE", 0.0, OUT_STEPS as i32),
    p(CLOCK_OUT, "Clock Output", "CLOCK", 0.0, OUT_STEPS as i32),
    p(ENV_OUT, "Envelope Output", "ENV", 0.0, OUT_STEPS as i32),
    p(VEL_OUT, "Velocity Output", "VEL OUT", 0.0, OUT_STEPS as i32),
    p(MOD_OUT, "Mod Wheel Output", "MOD OUT", 0.0, OUT_STEPS as i32),
    p(ATTACK, "Attack", "ATTACK", 0.1, 0),
    p(DECAY, "Decay", "DECAY", 0.5, 0),
    p(SUSTAIN, "Sustain", "SUSTAIN", 0.7, 0),
    p(RELEASE, "Release", "RELEASE", 0.4, 0),
    p(DIVISION, "Clock Division", "DIVISION", DEFAULT_DIVISION, DIVISION_STEPS as i32),
    p(CLOCK_LEVEL, "Clock Level", "CLK LVL", 0.5, 0),
    p(GATE_LEVEL, "Gate Level", "GATE LVL", 0.5, 0),
    p(ENV_LEVEL, "Envelope Level", "ENV LVL", 0.5, 0),
    p(VEL_LEVEL, "Velocity Level", "VEL LVL", 0.5, 0),
    p(MOD_LEVEL, "Mod Wheel Level", "MOD LVL", 0.5, 0),
    p(INPUT, "Input", "INPUT", 0.0, INPUT_STEPS as i32),
    p(HOLD_NOTE, "Hold Note", "HOLD NOTE", HOLD_DEFAULT, 127),
    p(LIVE, "Tune While Playing", "LIVE", 0.0, 1),
    p(GLIDE, "Glide", "GLIDE", 0.0, 0),
    p(HOLD, "Hold", "HOLD", 0.0, 1),
    p(TUNE, "Tune", "TUNE", 0.0, 1),
    p(CHECK, "Check", "CHECK", 0.0, 1),
];

static PAGES: &[Page] = &[
    Page {
        name: "OUTS",
        knobs: &[
            (PITCH_OUT, "Outputs"),
            (GATE_OUT, ""),
            (CLOCK_OUT, ""),
            (ENV_OUT, ""),
            (INPUT, "Tuner"),
            (HOLD_NOTE, ""),
            (LIVE, ""),
            (GLIDE, ""),
        ],
    },
    Page {
        name: "ENV",
        knobs: &[
            (ATTACK, "Envelope"),
            (DECAY, ""),
            (SUSTAIN, ""),
            (RELEASE, ""),
            (INPUT, "Tuner"),
            (HOLD_NOTE, ""),
            (LIVE, ""),
            (GLIDE, ""),
        ],
    },
    Page {
        name: "LEVELS",
        knobs: &[
            (DIVISION, "Clock and Levels"),
            (CLOCK_LEVEL, ""),
            (GATE_LEVEL, ""),
            (ENV_LEVEL, ""),
            (INPUT, "Tuner"),
            (HOLD_NOTE, ""),
            (LIVE, ""),
            (GLIDE, ""),
        ],
    },
    Page {
        name: "MOD",
        knobs: &[
            (VEL_OUT, "Velocity and Mod Wheel"),
            (VEL_LEVEL, ""),
            (MOD_OUT, ""),
            (MOD_LEVEL, ""),
            (INPUT, "Tuner"),
            (HOLD_NOTE, ""),
            (LIVE, ""),
            (GLIDE, ""),
        ],
    },
];

// The signals, in `Engine::cv` order.
const PITCH: usize = 0;
const GATE: usize = 1;
const CLOCK: usize = 2;
const ENV: usize = 3;
const VEL: usize = 4;
const MOD: usize = 5;
const SIGNALS: usize = 6;

/// Frames between detections.
const HOP: usize = 1024;
/// Gate drop that makes the modular retrigger on an overlapping note.
const RETRIGGER_S: f64 = 0.002;
/// Length of a clock pulse.
const CLOCK_PULSE_S: f64 = 0.005;
/// Time the mod wheel takes to reach a new value.
const MOD_RAMP_S: f64 = 0.064;
/// Semitones of a full pitch bend.
const BEND_SEMIS: f64 = 2.0;
/// Input level under which there is no pitch (about -60 dBFS).
const MIN_RMS: f32 = 0.001;
/// The detector never sees a rate over this.
const MAX_DETECT_RATE: f32 = 60_000.0;
/// A Run fails after this long with no input arriving.
const NO_INPUT_S: f64 = 2.0;
/// The tuner stops showing a pitch this long after the input stops.
const STALE_S: f64 = 0.25;

const NO_INPUT: &str = "Choose an input first";
const NO_SIGNAL: &str = "No signal on the input";
const NOT_TUNED: &str = "Tune first";
const BUSY: &str = "Tuner is busy";
const TUNER_STOPPED: &str = "The tuner has stopped";

/// The channel of an output picker's value: 0 is None, n is channel n - 1.
fn channel(v: f64) -> Option<usize> {
    step(v, OUT_STEPS).checked_sub(1)
}

/// `note`'s name in DECKARD's naming (MIDI note 60 is C3).
fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[note as usize % 12], note as i32 / 12 - 2)
}

/// What the worker tells `process`, without blocking either of them.
#[derive(Default)]
struct Shared {
    /// The running Tune or Check's output value (`f32` bits).
    output: AtomicU32,
    /// The last detection: hz bits over clarity bits, 0 for none.
    reading: AtomicU64,
    /// Counts detections, heard or not.
    seq: AtomicU32,
}

impl Shared {
    fn publish(&self, reading: Option<Pitch>) {
        let bits = reading.map_or(0, |p| ((p.hz.to_bits() as u64) << 32) | p.clarity.to_bits() as u64);
        self.reading.store(bits, Ordering::Relaxed);
        self.seq.fetch_add(1, Ordering::Release);
    }

    fn reading(&self) -> Option<Pitch> {
        let bits = self.reading.load(Ordering::Relaxed);
        (bits != 0).then(|| Pitch { hz: f32::from_bits((bits >> 32) as u32), clarity: f32::from_bits(bits as u32) })
    }
}

/// How the last Tune or Check ended.
#[derive(Default)]
struct Report {
    id: u32,
    /// A new table (taken by `process`), or the one it replaced on its way back to be freed.
    table: Option<Arc<Table>>,
    /// The worst error of a Check.
    worst_cents: Option<f32>,
    off: Vec<u8>,
    message: Option<&'static str>,
}

enum Cmd {
    Tune(u32),
    Check(u32, Arc<Table>),
    Cancel,
    /// Only here to be freed off the audio thread.
    Retire(Report),
}

/// The analysis thread. It owns the Run, since stepping one allocates, and ends when the
/// engine drops its end of `blocks`.
fn worker(sample_rate: f32, blocks: Receiver<[f32; HOP]>, cmds: Receiver<Cmd>, done: SyncSender<Report>, shared: Arc<Shared>) {
    // Above 60 kHz, whole samples are averaged down so the window still spans low notes.
    let mut factor = 1;
    let mut rate = sample_rate;
    while rate > MAX_DETECT_RATE {
        factor *= 2;
        rate *= 0.5;
    }
    let hop_ms = HOP as f32 * 1000.0 / sample_rate;
    let shift = HOP / factor;
    let mut window = vec![0f32; WINDOW];
    let mut have = 0;
    let mut run: Option<(u32, Run)> = None;
    while let Ok(block) = blocks.recv() {
        let mut started = false;
        for cmd in cmds.try_iter() {
            match cmd {
                Cmd::Tune(id) => {
                    run = Some((id, Run::tune()));
                    started = true;
                }
                Cmd::Check(id, table) => {
                    run = Some((id, Run::check((*table).clone())));
                    started = true;
                }
                Cmd::Cancel => run = None,
                Cmd::Retire(report) => drop(report),
            }
        }
        window.copy_within(shift.., 0);
        for (slot, chunk) in window[WINDOW - shift..].iter_mut().zip(block.chunks_exact(factor)) {
            *slot = chunk.iter().sum::<f32>() / factor as f32;
        }
        have = (have + shift).min(WINDOW);
        let rms = (block.iter().map(|s| s * s).sum::<f32>() / HOP as f32).sqrt();
        // Written so that a NaN gives no pitch.
        let reading = if have == WINDOW && rms >= MIN_RMS { detect(&window, rate) } else { None };
        shared.publish(reading);
        if let Some((id, r)) = &mut run {
            // The hop that brought the order was played before it: no time has passed yet.
            let id = *id;
            let result = if started { r.step(0.0, None) } else { r.step(hop_ms, reading) };
            match result {
                Step::Output(v) => shared.output.store(v.to_bits(), Ordering::Relaxed),
                Step::Done(outcome) => {
                    run = None;
                    let report = report_of(id, outcome);
                    if done.send(report).is_err() {
                        return;
                    }
                }
            }
        }
    }
}

/// What the rest of the instrument needs to know of `outcome`.
fn report_of(id: u32, outcome: Outcome) -> Report {
    match outcome {
        Outcome::Tuned { mut table, off } => {
            table.tuned_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            Report { id, table: Some(Arc::new(table)), worst_cents: None, off, message: None }
        }
        Outcome::Checked { worst_cents, off } => Report { id, table: None, worst_cents: Some(worst_cents), off, message: None },
        Outcome::Failed(message) => Report { id, table: None, worst_cents: None, off: Vec::new(), message: Some(message) },
    }
}

#[derive(Clone, Copy)]
struct Running {
    id: u32,
    tuning: bool,
}

pub struct Zhora {
    sr: f64,
    shared: Arc<Shared>,
    blocks: SyncSender<[f32; HOP]>,
    cmds: SyncSender<Cmd>,
    finished: Receiver<Report>,
    /// The input going toward the next hop, and how much of it there is.
    block: [f32; HOP],
    filled: usize,
    /// Frames of `process` since `input` was last called.
    since_input: u64,
    seen_seq: u32,
    table: Option<Arc<Table>>,
    report: Report,
    running: Option<Running>,
    next_id: u32,
    /// `process` has run, so a rising Hold, Tune or Check is a press and not a saved value.
    started: bool,
    hold_value: f64,
    tune_value: f64,
    check_value: f64,
    holding: bool,
    hold_note: u8,

    outs: [Option<usize>; SIGNALS],
    input_pair: Option<usize>,
    env_values: [f64; 4],
    env_settings: EnvSettings,
    env: Env,
    division: usize,
    clock_level: f32,
    gate_level: f32,
    env_level: f32,
    vel_level: f32,
    mod_level: f32,
    glide_frames: u32,
    live: bool,

    /// Held notes, oldest first.
    stack: Vec<u8>,
    /// The pitch now, in notes, sliding to `target`.
    note: f64,
    target: f64,
    glide_step: f64,
    glide_left: u32,
    played: bool,
    since_note_ms: f32,
    bend: f64,
    velocity: f32,
    mod_wheel: f32,
    /// The smoothed mod wheel output, and its ramp.
    mod_now: f64,
    mod_target: f64,
    mod_step: f64,
    mod_left: u32,
    /// Frames the gate stays low for a retrigger.
    gap_left: u32,
    gate_was_high: bool,
    /// No note has moved the pitch since the last reset: it holds its value.
    frozen: bool,
    last_pitch: f32,
    trim: Trim,
    trim_semis: f64,
    clamped: bool,

    cv: [Vec<f32>; SIGNALS],
    frames: usize,
    /// A retired report the queue had no room for, sent again on the next `process`.
    retiring: Option<Cmd>,
    /// The worker has ended, so no Run can finish.
    worker_gone: bool,
    /// Hold or a Run had the outputs in the last `process`.
    was_open: bool,
}

impl Zhora {
    pub fn new(sr: f64) -> Zhora {
        let (blocks, block_rx) = sync_channel(16);
        let (cmds, cmd_rx) = sync_channel(16);
        let (done, finished) = sync_channel(4);
        let shared = Arc::new(Shared::default());
        let for_worker = shared.clone();
        let rate = sr as f32;
        std::thread::Builder::new()
            .name("zhora tuner".into())
            .spawn(move || worker(rate, block_rx, cmd_rx, done, for_worker))
            .expect("start Zhora's tuner thread");
        let mut z = Zhora {
            sr,
            shared,
            blocks,
            cmds,
            finished,
            block: [0.0; HOP],
            filled: 0,
            since_input: 0,
            seen_seq: 0,
            table: None,
            report: Report::default(),
            running: None,
            next_id: 0,
            started: false,
            hold_value: 0.0,
            tune_value: 0.0,
            check_value: 0.0,
            holding: false,
            hold_note: 69,
            outs: [None; SIGNALS],
            input_pair: None,
            env_values: [0.1, 0.5, 0.7, 0.4],
            env_settings: EnvSettings::default(),
            env: Env::default(),
            division: 2,
            clock_level: 0.5,
            gate_level: 0.5,
            env_level: 0.5,
            vel_level: 0.5,
            mod_level: 0.5,
            glide_frames: 0,
            live: false,
            stack: Vec::with_capacity(128),
            note: 60.0,
            target: 60.0,
            glide_step: 0.0,
            glide_left: 0,
            played: false,
            since_note_ms: 0.0,
            bend: 0.0,
            velocity: 0.0,
            mod_wheel: 0.0,
            mod_now: 0.0,
            mod_target: 0.0,
            mod_step: 0.0,
            mod_left: 0,
            gap_left: 0,
            gate_was_high: false,
            frozen: true,
            last_pitch: 0.0,
            trim: Trim::new(),
            trim_semis: 0.0,
            clamped: false,
            cv: std::array::from_fn(|_| vec![0.0; MAX_BLOCK]),
            frames: 0,
            retiring: None,
            worker_gone: false,
            was_open: false,
        };
        z.update_env();
        z
    }

    fn update_env(&mut self) {
        let [a, d, s, r] = self.env_values;
        self.env_settings = EnvSettings::new(a, d, s, r, self.sr);
    }

    /// The output value for `note`, and whether it had to be clamped.
    fn value_for(&self, note: f64) -> (f32, bool) {
        match &self.table {
            Some(t) => t.value_for(note as f32),
            None => (Table::nominal(note as f32).clamp(-1.0, 1.0), false),
        }
    }

    /// Whether Tune, Check or Hold has the outputs, so notes don't.
    fn open(&self) -> bool {
        self.holding || self.running.is_some()
    }

    fn go_to(&mut self, note: f64) {
        self.target = note;
        self.since_note_ms = 0.0;
        if self.played && self.glide_frames > 0 && note != self.note {
            self.glide_left = self.glide_frames;
            self.glide_step = (note - self.note) / self.glide_frames as f64;
        } else {
            self.note = note;
            self.glide_left = 0;
        }
        self.played = true;
    }

    fn advance_glide(&mut self) {
        if self.glide_left == 0 {
            return;
        }
        self.glide_left -= 1;
        self.note += self.glide_step;
        if self.glide_left == 0 {
            self.note = self.target;
            // A glide's last moments are not a settled note.
            self.since_note_ms = 0.0;
        }
    }

    fn press(&mut self, tuning: bool) {
        if self.running.is_some() {
            return;
        }
        self.report.message = None;
        if self.input_pair.is_none() {
            self.report.message = Some(NO_INPUT);
            return;
        }
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        let cmd = if tuning {
            Cmd::Tune(id)
        } else {
            let Some(table) = self.table.clone() else {
                self.report.message = Some(NOT_TUNED);
                return;
            };
            Cmd::Check(id, table)
        };
        // Until the worker takes over, the pitch stays where it was.
        self.shared.output.store(self.last_pitch.to_bits(), Ordering::Relaxed);
        if self.cmds.try_send(cmd).is_err() {
            self.report.message = Some(BUSY);
            return;
        }
        self.running = Some(Running { id, tuning });
        self.since_input = 0;
    }

    /// Takes the worker's finished Run, if it is the one still wanted. What it replaces goes
    /// back to the worker; while the queue is full it waits in `retiring`, never freed here.
    fn take_finished(&mut self) {
        loop {
            if let Some(cmd) = self.retiring.take()
                && let Err(TrySendError::Full(cmd) | TrySendError::Disconnected(cmd)) = self.cmds.try_send(cmd)
            {
                self.retiring = Some(cmd);
                return;
            }
            let Ok(mut report) = self.finished.try_recv() else { return };
            if self.running.is_some_and(|r| r.id == report.id) {
                self.running = None;
                // The table this one replaces goes back in `report` to be freed.
                let replaced = report.table.take().and_then(|table| {
                    self.trim = Trim::new();
                    self.trim_semis = 0.0;
                    self.table.replace(table)
                });
                report = std::mem::replace(&mut self.report, report);
                report.table = replaced;
            }
            self.retiring = Some(Cmd::Retire(report));
        }
    }

    fn give_up(&mut self, message: &'static str) {
        self.running = None;
        let _ = self.cmds.try_send(Cmd::Cancel);
        self.report.message = Some(message);
    }

    /// A reading from a detection `process` has not seen yet.
    fn fresh_reading(&mut self) -> Option<Pitch> {
        let seq = self.shared.seq.load(Ordering::Acquire);
        if seq == self.seen_seq {
            return None;
        }
        self.seen_seq = seq;
        self.shared.reading()
    }

    /// One block's worth of tune-while-playing. `reading` is a new detection or `None`.
    fn update_trim(&mut self, dt_ms: f32, reading: Option<Pitch>) {
        let asked = self.note + self.bend * BEND_SEMIS;
        let trimmed = asked + self.trim_semis;
        // A note the table had to clamp isn't sounding what was asked; nothing to learn.
        let reading = reading
            .filter(|_| !self.frozen && !self.stack.is_empty() && self.gap_left == 0 && self.table.is_some() && !self.value_for(trimmed).1);
        self.trim_semis = self.trim.update(dt_ms, asked as f32, note_hz(trimmed as f32), self.since_note_ms, reading) as f64;
    }
}

impl Engine for Zhora {
    fn set_param(&mut self, id: u32, value: f64) {
        let v32 = value as f32;
        match id {
            PITCH_OUT..=MOD_OUT => self.outs[id as usize - 1] = channel(value),
            ATTACK..=RELEASE => {
                self.env_values[(id - ATTACK) as usize] = value;
                self.update_env();
            }
            DIVISION => self.division = step(value, DIVISION_STEPS),
            CLOCK_LEVEL => self.clock_level = v32,
            GATE_LEVEL => self.gate_level = v32,
            ENV_LEVEL => self.env_level = v32,
            VEL_LEVEL => self.vel_level = v32,
            MOD_LEVEL => self.mod_level = v32,
            INPUT => self.input_pair = step(value, INPUT_STEPS).checked_sub(1),
            HOLD_NOTE => self.hold_note = step(value, 127) as u8,
            LIVE => {
                self.live = value > 0.5;
                if !self.live {
                    self.trim = Trim::new();
                    self.trim_semis = 0.0;
                }
            }
            GLIDE => self.glide_frames = if value <= 0.0 { 0 } else { (glide_time(value) * self.sr).round() as u32 },
            HOLD => {
                let was = std::mem::replace(&mut self.hold_value, value);
                if value <= 0.5 {
                    self.holding = false;
                } else if self.started && was <= 0.5 {
                    self.holding = true;
                    self.report.message = None;
                }
            }
            TUNE => {
                let was = std::mem::replace(&mut self.tune_value, value);
                if self.started && was <= 0.5 && value > 0.5 {
                    self.press(true);
                }
            }
            CHECK => {
                let was = std::mem::replace(&mut self.check_value, value);
                if self.started && was <= 0.5 && value > 0.5 {
                    self.press(false);
                }
            }
            _ => {}
        }
    }

    fn text(&self, id: u32, v: f64) -> String {
        match id {
            PITCH_OUT..=MOD_OUT => match channel(v) {
                Some(c) => format!("Out {} {}", c / 2 + 1, if c % 2 == 0 { "L" } else { "R" }),
                None => "None".into(),
            },
            ATTACK | DECAY | RELEASE => time_text(env_time(v)),
            DIVISION => DIVISIONS[step(v, DIVISION_STEPS)].0.into(),
            INPUT => match step(v, INPUT_STEPS) {
                0 => "None".into(),
                n => format!("In {n} L+R"),
            },
            HOLD_NOTE => note_name(step(v, 127) as u8),
            LIVE | HOLD | TUNE | CHECK => ["Off", "On"][step(v, 1)].into(),
            GLIDE => {
                if v <= 0.0 {
                    "Off".into()
                } else {
                    time_text(glide_time(v))
                }
            }
            _ => format!("{:.0} %", v * 100.0),
        }
    }

    fn note_on(&mut self, pitch: u8, velocity: u8) {
        if self.open() {
            return;
        }
        if self.gate_was_high {
            self.gap_left = (RETRIGGER_S * self.sr).round() as u32;
        }
        self.stack.retain(|&n| n != pitch);
        self.stack.push(pitch);
        self.velocity = velocity as f32 / 127.0;
        self.frozen = false;
        self.go_to(pitch as f64);
        self.env.gate(true);
    }

    fn note_off(&mut self, pitch: u8) {
        let Some(i) = self.stack.iter().position(|&n| n == pitch) else { return };
        let newest = i + 1 == self.stack.len();
        self.stack.remove(i);
        match self.stack.last() {
            None => self.env.gate(false),
            Some(&note) if newest => self.go_to(note as f64),
            Some(_) => {}
        }
    }

    fn control_change(&mut self, number: u8, value: u8) {
        if number == 1 {
            self.mod_wheel = value as f32 / 127.0;
        }
    }

    fn pitch_bend(&mut self, bend: f32) {
        self.bend = bend as f64;
    }

    /// Gate and envelope drop; pitch, velocity and mod wheel hold their last values.
    /// The gate's last rendered frame stays as it was, so a note in the same span still retriggers.
    fn reset(&mut self) {
        self.stack.clear();
        self.env = Env::default();
        self.gap_left = 0;
        self.frozen = true;
        self.clamped = false;
    }

    /// Fills the CV for `left`'s length of frames; the sound itself is the modular's.
    fn process(&mut self, left: &mut [f32], _right: &mut [f32], transport: &Transport) {
        let n = left.len().min(MAX_BLOCK);
        self.take_finished();
        if self.running.is_some() && self.worker_gone {
            self.give_up(TUNER_STOPPED);
        }
        if self.running.is_some() && self.since_input as f64 >= NO_INPUT_S * self.sr {
            self.give_up(NO_SIGNAL);
        }
        self.since_input = self.since_input.saturating_add(n as u64);

        let reading = self.fresh_reading();
        let holding = self.holding;
        let running = self.running.is_some();
        let run_value = running.then(|| f32::from_bits(self.shared.output.load(Ordering::Relaxed)));
        let hold_value = holding.then(|| self.value_for(self.hold_note as f64).0);
        let open = holding || running;
        if self.was_open && !open {
            // What was heard belonged to the hold note or the Run, not a settled note.
            self.since_note_ms = 0.0;
        }
        self.was_open = open;
        if !open {
            let dt_ms = n as f32 * 1000.0 / self.sr as f32;
            if self.glide_left == 0 {
                self.since_note_ms += dt_ms;
            }
            if self.live {
                self.update_trim(dt_ms, reading);
            }
        }

        let mod_goal = self.mod_wheel as f64 * self.mod_level as f64;
        if mod_goal != self.mod_target {
            self.mod_target = mod_goal;
            self.mod_left = (MOD_RAMP_S * self.sr).round().max(1.0) as u32;
            self.mod_step = (mod_goal - self.mod_now) / self.mod_left as f64;
        }

        let beat_per_frame = transport.tempo.max(0.0) / 60.0 / self.sr;
        let division = DIVISIONS[self.division.min(DIVISION_STEPS)].1;
        let period = if beat_per_frame > 0.0 { division / beat_per_frame } else { f64::MAX };
        let pulse = (CLOCK_PULSE_S * self.sr).min(period * 0.5);
        let velocity = self.velocity * self.vel_level;
        let live_trim = if self.live { self.trim_semis } else { 0.0 };

        for i in 0..n {
            let pitch = if let Some(v) = run_value {
                v
            } else if let Some(v) = hold_value {
                v
            } else if self.frozen {
                self.last_pitch
            } else {
                self.advance_glide();
                self.value_for(self.note + self.bend * BEND_SEMIS + live_trim).0
            };
            let gate = open || (!self.stack.is_empty() && self.gap_left == 0);
            self.gap_left = self.gap_left.saturating_sub(1);
            let env = if open { 1.0 } else { self.env.next(&self.env_settings) as f32 };
            let clock = transport.playing && {
                let position = (transport.quarters + i as f64 * beat_per_frame).rem_euclid(division);
                beat_per_frame > 0.0 && position / beat_per_frame < pulse - 1e-6
            };
            if self.mod_left > 0 {
                self.mod_left -= 1;
                self.mod_now = if self.mod_left == 0 { self.mod_target } else { self.mod_now + self.mod_step };
            }

            self.cv[PITCH][i] = pitch;
            self.cv[GATE][i] = if gate { self.gate_level } else { 0.0 };
            self.cv[CLOCK][i] = if clock { self.clock_level } else { 0.0 };
            self.cv[ENV][i] = env * self.env_level;
            self.cv[VEL][i] = velocity;
            self.cv[MOD][i] = self.mod_now as f32;
            self.last_pitch = pitch;
            self.gate_was_high = gate;
        }
        self.frames = n;
        self.clamped = !open && !self.frozen && !self.stack.is_empty() && self.value_for(self.note + self.bend * BEND_SEMIS).1;
        self.started = true;
    }

    fn save(&self) -> Vec<u8> {
        self.table.as_ref().map_or_else(Vec::new, |t| t.to_bytes())
    }

    fn load(&mut self, data: &[u8]) {
        if self.running.take().is_some() {
            let _ = self.cmds.try_send(Cmd::Cancel);
        }
        self.table = Table::from_bytes(data).map(Arc::new);
        self.report = Report::default();
        self.trim = Trim::new();
        self.trim_semis = 0.0;
        // The saved values about to arrive are not presses.
        self.started = false;
    }

    fn cv(&self, signal: usize, frames: usize) -> Option<(usize, &[f32])> {
        let channel = (*self.outs.get(signal)?)?;
        Some((channel, &self.cv[signal][..frames.min(self.frames)]))
    }

    fn input(&mut self, left: &[f32], right: &[f32]) {
        self.since_input = 0;
        for (&l, &r) in left.iter().zip(right) {
            self.block[self.filled] = (l + r) * 0.5;
            self.filled += 1;
            if self.filled == HOP {
                // A full queue means the analysis is behind, and this hop is lost.
                if let Err(TrySendError::Disconnected(_)) = self.blocks.try_send(self.block) {
                    self.worker_gone = true;
                }
                self.filled = 0;
            }
        }
    }

    fn input_pair(&self) -> Option<usize> {
        self.input_pair
    }

    fn readout(&self) -> Option<Readout> {
        let state = match self.running {
            Some(r) if r.tuning => TuneState::Tuning,
            Some(_) => TuneState::Checking,
            None if self.table.is_some() => TuneState::Idle,
            None => TuneState::NotTuned,
        };
        let heard = (self.input_pair.is_some() && (self.since_input as f64) < STALE_S * self.sr)
            .then(|| self.shared.reading())
            .flatten()
            .map(|p| {
                let exact = 69.0 + 12.0 * (p.hz / 440.0).log2();
                let note = exact.round().clamp(0.0, 127.0);
                (note as u8, (exact - note) * 100.0)
            });
        let table = self.table.as_deref();
        Some(Readout {
            note: heard,
            state,
            worst_cents: self.report.worst_cents.or(table.map(|t| t.worst_cents)),
            tuned_at: table.map(|t| t.tuned_at),
            range: table.map(Table::range),
            live_locked: self.live.then(|| self.trim.locked()),
            clamped: self.clamped,
            message: self.report.message,
            off: self.report.off.clone(),
            holding: self.holding,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::table::Table;
    use super::*;
    use crate::Transport;
    use crate::native::{Engine, TuneState};
    use std::time::{Duration, Instant};

    const SR: f64 = 48_000.0;
    const BLOCK: usize = 480;

    fn stopped() -> Transport {
        Transport { tempo: 120.0, playing: false, quarters: 0.0, numerator: 4, denominator: 4 }
    }

    /// What `Instance::new` does before the first `process`.
    fn defaults(e: &mut dyn Engine) {
        for p in PARAMS {
            e.set_param(p.id, p.default);
        }
    }

    fn engine() -> Box<dyn Engine> {
        let mut e = (CLASS.make)(SR);
        defaults(e.as_mut());
        e
    }

    /// Every signal to its own channel, 0..6.
    fn route(e: &mut dyn Engine) {
        for (i, id) in [PITCH_OUT, GATE_OUT, CLOCK_OUT, ENV_OUT, VEL_OUT, MOD_OUT].into_iter().enumerate() {
            e.set_param(id, (i + 1) as f64 / 32.0);
        }
    }

    fn routed() -> Box<dyn Engine> {
        let mut e = engine();
        route(e.as_mut());
        e
    }

    fn run(e: &mut dyn Engine, frames: usize, t: &Transport) {
        let (mut l, mut r) = (vec![0f32; frames], vec![0f32; frames]);
        e.process(&mut l, &mut r, t);
    }

    /// Runs one block and returns signal `signal` of it.
    fn block(e: &mut dyn Engine, signal: usize, frames: usize, t: &Transport) -> Vec<f32> {
        run(e, frames, t);
        e.cv(signal, frames).unwrap().1.to_vec()
    }

    /// A voice that follows 1 V per octave, bowed so a table is not a straight line.
    fn bowed() -> Table {
        let points = (0..=24)
            .map(|i| {
                let v = i as f32 / 24.0 - 0.5;
                (v, (440f32).log2() + v * 10.0 + 0.04 * (v * 9.0).sin())
            })
            .collect();
        Table { points, tuned_at: 1234, worst_cents: 1.5 }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    fn nominal(note: f32) -> f32 {
        Table::nominal(note)
    }

    #[test]
    fn note_on_sets_pitch_gate_velocity_and_the_envelope_fires() {
        let mut e = routed();
        let t = stopped();
        e.note_on(60, 127);
        run(e.as_mut(), BLOCK, &t);
        assert!(e.cv(PITCH, BLOCK).unwrap().1.iter().all(|&v| near(v, nominal(60.0))));
        assert!(e.cv(GATE, BLOCK).unwrap().1.iter().all(|&v| near(v, 0.5)));
        assert!(e.cv(VEL, BLOCK).unwrap().1.iter().all(|&v| near(v, 0.5)));
        let env = e.cv(ENV, BLOCK).unwrap().1;
        assert!(env[0] > 0.0 && env[BLOCK - 1] > env[0], "the envelope rises");
        e.note_off(60);
        let gate = block(e.as_mut(), GATE, BLOCK, &t);
        assert!(gate.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn newest_note_wins_and_lifting_it_returns_to_the_held_note() {
        let mut e = routed();
        let t = stopped();
        e.note_on(60, 100);
        run(e.as_mut(), BLOCK, &t);
        e.note_on(64, 100);
        run(e.as_mut(), BLOCK, &t);
        assert!(near(e.cv(PITCH, BLOCK).unwrap().1[BLOCK - 1], nominal(64.0)));
        e.note_off(64);
        let gate = block(e.as_mut(), GATE, BLOCK, &t);
        assert!(near(e.cv(PITCH, BLOCK).unwrap().1[0], nominal(60.0)));
        assert!(gate.iter().all(|&v| near(v, 0.5)), "going back to a held note does not retrigger");
    }

    #[test]
    fn an_overlapping_note_drops_the_gate_briefly() {
        let mut e = routed();
        let t = stopped();
        e.note_on(60, 100);
        run(e.as_mut(), BLOCK, &t);
        e.note_on(64, 100);
        let gate = block(e.as_mut(), GATE, BLOCK, &t);
        assert!(gate[..96].iter().all(|&v| v == 0.0), "low for 2 ms");
        assert!(gate[96..].iter().all(|&v| near(v, 0.5)), "then high");
        assert!(near(e.cv(PITCH, BLOCK).unwrap().1[0], nominal(64.0)), "pitch moves before the gate rises");
    }

    #[test]
    fn a_note_landing_as_the_last_one_ends_still_retriggers() {
        let mut e = routed();
        let t = stopped();
        e.note_on(60, 100);
        run(e.as_mut(), BLOCK, &t);
        e.note_off(60);
        e.note_on(64, 100);
        let gate = block(e.as_mut(), GATE, BLOCK, &t);
        assert!(gate[..96].iter().all(|&v| v == 0.0));
        assert!(near(gate[96], 0.5));
    }

    #[test]
    fn glide_slides_in_notes_so_each_point_is_in_tune() {
        let table = bowed();
        let mut e = routed();
        e.load(&table.to_bytes());
        e.set_param(GLIDE, 0.5);
        let t = stopped();
        e.note_on(48, 100);
        run(e.as_mut(), BLOCK, &t);
        e.note_on(72, 100);
        let frames = (glide_time(0.5) * SR).round() as usize;
        let mut values = Vec::new();
        for _ in 0..(frames + 2 * BLOCK) / BLOCK {
            values.extend(block(e.as_mut(), PITCH, BLOCK, &t));
        }
        // Each value is the table's value for a note, and those notes move by equal steps.
        let note_of = |v: f32| {
            let (mut lo, mut hi) = (47.0f32, 73.0f32);
            for _ in 0..40 {
                let mid = (lo + hi) * 0.5;
                if table.value_for(mid).0 < v {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        let notes: Vec<f32> = values[..frames].iter().map(|&v| note_of(v)).collect();
        for (i, n) in notes.iter().enumerate() {
            let want = 48.0 + 24.0 * (i + 1) as f32 / frames as f32;
            assert!((n - want).abs() < 0.02, "frame {i}: note {n}, wanted {want}");
        }
        assert!(values.windows(2).all(|w| w[1] >= w[0]));
        assert!(near(*values.last().unwrap(), table.value_for(72.0).0));
    }

    #[test]
    fn mod_wheel_is_smoothed() {
        let mut e = routed();
        let t = stopped();
        run(e.as_mut(), BLOCK, &t);
        e.control_change(1, 127);
        let mut values = Vec::new();
        for _ in 0..16 {
            values.extend(block(e.as_mut(), MOD, BLOCK, &t));
        }
        let jump = 0.5f32;
        assert!(values.windows(48).all(|w| (w[47] - w[0]).abs() <= jump / 64.0 * 1.001), "no step over 1/64 of the jump per ms");
        assert!(near(*values.last().unwrap(), jump), "it arrives");
    }

    #[test]
    fn a_none_output_returns_no_cv() {
        let mut e = engine();
        e.set_param(PITCH_OUT, 3.0 / 32.0);
        e.note_on(60, 100);
        run(e.as_mut(), BLOCK, &stopped());
        assert_eq!(e.cv(PITCH, BLOCK).unwrap().0, 2);
        assert_eq!(e.cv(PITCH, BLOCK).unwrap().1.len(), BLOCK);
        for signal in [GATE, CLOCK, ENV, VEL, MOD] {
            assert!(e.cv(signal, BLOCK).is_none());
        }
        assert!(e.cv(6, BLOCK).is_none());
    }

    #[test]
    fn the_clock_pulses_at_the_division_only_while_playing() {
        let mut e = routed();
        let mut t = Transport { playing: true, ..stopped() };
        let mut values = Vec::new();
        // 120 BPM: a 1/16 is 6000 frames.
        for b in 0..40 {
            t.quarters = b as f64 * 1024.0 / SR * 2.0;
            values.extend(block(e.as_mut(), CLOCK, 1024, &t));
        }
        let rises: Vec<usize> = (1..values.len()).filter(|&i| values[i] > 0.0 && values[i - 1] == 0.0).collect();
        assert!(values[0] > 0.0, "the first pulse is at the start");
        assert_eq!(rises.len(), 6, "{rises:?}");
        assert!(rises.iter().all(|&r| r % 6000 <= 1 || r % 6000 >= 5999), "{rises:?}");
        let width = values[..3000].iter().filter(|&&v| v > 0.0).count();
        assert!((239..=241).contains(&width), "5 ms pulse, got {width} frames");
        // The pulse at 6000 crosses the block boundary at 6144.
        let width = values[5900..6400].iter().filter(|&&v| v > 0.0).count();
        assert!((239..=241).contains(&width), "a pulse across blocks, got {width} frames");
        assert!(values.iter().all(|&v| v == 0.0 || near(v, 0.5)));
        t.playing = false;
        t.quarters = 0.0;
        let idle = block(e.as_mut(), CLOCK, 1024, &t);
        assert!(idle.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn levels_scale_gate_clock_env_velocity_and_mod() {
        let mut e = routed();
        e.set_param(GATE_LEVEL, 0.25);
        e.set_param(CLOCK_LEVEL, 0.75);
        e.set_param(ENV_LEVEL, 1.0);
        e.set_param(VEL_LEVEL, 0.2);
        e.set_param(MOD_LEVEL, 0.1);
        e.set_param(ATTACK, 0.0);
        e.set_param(SUSTAIN, 1.0);
        e.control_change(1, 127);
        e.note_on(60, 127);
        let t = Transport { playing: true, ..stopped() };
        for _ in 0..16 {
            run(e.as_mut(), BLOCK, &stopped());
        }
        run(e.as_mut(), BLOCK, &t);
        let at = |e: &dyn Engine, s: usize, i: usize| e.cv(s, BLOCK).unwrap().1[i];
        assert!(near(at(e.as_ref(), GATE, 0), 0.25));
        assert!(near(at(e.as_ref(), CLOCK, 0), 0.75));
        assert!(near(at(e.as_ref(), ENV, 0), 1.0));
        assert!(near(at(e.as_ref(), VEL, 0), 0.2));
        assert!(near(at(e.as_ref(), MOD, BLOCK - 1), 0.1));
    }

    #[test]
    fn hold_sends_the_hold_note_with_gate_and_envelope_open_ignoring_glide() {
        let mut e = routed();
        let t = stopped();
        e.set_param(GLIDE, 1.0);
        e.note_on(48, 100);
        run(e.as_mut(), BLOCK, &t);
        e.note_off(48);
        e.set_param(HOLD, 1.0);
        run(e.as_mut(), BLOCK, &t);
        assert!(e.cv(PITCH, BLOCK).unwrap().1.iter().all(|&v| near(v, nominal(69.0))));
        assert!(e.cv(GATE, BLOCK).unwrap().1.iter().all(|&v| near(v, 0.5)));
        assert!(e.cv(ENV, BLOCK).unwrap().1.iter().all(|&v| near(v, 0.5)));
        e.set_param(HOLD, 0.0);
        run(e.as_mut(), BLOCK, &t);
        assert!(e.cv(GATE, BLOCK).unwrap().1.iter().all(|&v| v == 0.0), "lets go");
    }

    #[test]
    fn hold_note_defaults_to_a3_in_deckards_naming() {
        let e = engine();
        let id = |p: u32| PARAMS.iter().find(|q| q.id == p).unwrap().default;
        assert_eq!(e.text(HOLD_NOTE, id(HOLD_NOTE)), "A3");
    }

    #[test]
    fn saved_actions_do_nothing_before_the_first_process() {
        let mut e = routed();
        e.set_param(INPUT, 1.0 / 16.0);
        e.set_param(TUNE, 1.0);
        e.set_param(HOLD, 1.0);
        run(e.as_mut(), BLOCK, &stopped());
        assert_eq!(e.readout().unwrap().state, TuneState::NotTuned);
        assert!(e.cv(GATE, BLOCK).unwrap().1.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn tune_runs_while_stopped_and_opens_gate_and_envelope() {
        let mut e = routed();
        let t = stopped();
        e.set_param(INPUT, 1.0 / 16.0);
        run(e.as_mut(), BLOCK, &t);
        e.set_param(TUNE, 1.0);
        run(e.as_mut(), BLOCK, &t);
        assert_eq!(e.readout().unwrap().state, TuneState::Tuning);
        assert!(e.cv(GATE, BLOCK).unwrap().1.iter().all(|&v| near(v, 0.5)));
        assert!(e.cv(ENV, BLOCK).unwrap().1.iter().all(|&v| near(v, 0.5)));
        // Notes are ignored meanwhile.
        e.note_on(90, 100);
        run(e.as_mut(), BLOCK, &t);
        assert!(!near(e.cv(PITCH, BLOCK).unwrap().1[0], nominal(90.0)));
    }

    #[test]
    fn tune_with_no_input_fails_at_once_and_keeps_the_table() {
        let mut e = routed();
        let t = stopped();
        e.load(&bowed().to_bytes());
        run(e.as_mut(), BLOCK, &t);
        e.set_param(TUNE, 1.0);
        let r = e.readout().unwrap();
        assert_eq!(r.state, TuneState::Idle);
        assert!(r.message.is_some());
        assert_eq!(e.save(), bowed().to_bytes());
    }

    #[test]
    fn check_with_no_table_says_so() {
        let mut e = routed();
        e.set_param(INPUT, 1.0 / 16.0);
        run(e.as_mut(), BLOCK, &stopped());
        e.set_param(CHECK, 1.0);
        let r = e.readout().unwrap();
        assert_eq!(r.state, TuneState::NotTuned);
        assert!(r.message.is_some());
    }

    #[test]
    fn no_input_arriving_fails_the_run_after_two_seconds() {
        let mut e = routed();
        let t = stopped();
        e.load(&bowed().to_bytes());
        e.set_param(INPUT, 1.0 / 16.0);
        run(e.as_mut(), BLOCK, &t);
        e.set_param(CHECK, 1.0);
        for _ in 0..190 {
            run(e.as_mut(), BLOCK, &t);
            assert_eq!(e.readout().unwrap().state, TuneState::Checking);
        }
        for _ in 0..12 {
            run(e.as_mut(), BLOCK, &t);
        }
        let r = e.readout().unwrap();
        assert_eq!(r.state, TuneState::Idle);
        assert_eq!(r.message, Some("No signal on the input"));
        assert_eq!(e.save(), bowed().to_bytes(), "the table stays");
    }

    #[test]
    fn reset_drops_gate_and_envelope_and_holds_pitch_velocity_and_mod() {
        let mut e = routed();
        let t = stopped();
        e.pitch_bend(0.5);
        e.control_change(1, 64);
        e.note_on(72, 64);
        for _ in 0..20 {
            run(e.as_mut(), BLOCK, &t);
        }
        let last = |e: &dyn Engine, s| e.cv(s, BLOCK).unwrap().1[BLOCK - 1];
        let (pitch, vel, modw) = (last(e.as_ref(), PITCH), last(e.as_ref(), VEL), last(e.as_ref(), MOD));
        // What `Instance::all_notes_off` does.
        e.pitch_bend(0.0);
        e.reset();
        run(e.as_mut(), BLOCK, &t);
        assert!(near(last(e.as_ref(), PITCH), pitch));
        assert!(near(last(e.as_ref(), VEL), vel));
        assert!(near(last(e.as_ref(), MOD), modw));
        assert!(e.cv(GATE, BLOCK).unwrap().1.iter().all(|&v| v == 0.0));
        assert!(e.cv(ENV, BLOCK).unwrap().1.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn pitch_bend_shifts_the_pitch() {
        let mut e = routed();
        let t = stopped();
        e.note_on(60, 100);
        e.pitch_bend(1.0);
        let pitch = block(e.as_mut(), PITCH, BLOCK, &t);
        assert!(near(pitch[0], nominal(62.0)));
        e.pitch_bend(-0.5);
        let pitch = block(e.as_mut(), PITCH, BLOCK, &t);
        assert!(near(pitch[0], nominal(59.0)));
    }

    #[test]
    fn pitch_gets_nominal_scaling_and_says_not_tuned_before_a_tune() {
        let mut e = routed();
        e.note_on(72, 100);
        let pitch = block(e.as_mut(), PITCH, BLOCK, &stopped());
        assert!(near(pitch[0], 0.1));
        let r = e.readout().unwrap();
        assert_eq!(r.state, TuneState::NotTuned);
        assert_eq!(r.tuned_at, None);
        assert_eq!(r.range, None);
    }

    #[test]
    fn a_note_past_the_table_clamps_and_is_flagged() {
        // Trimmed so note 127 lies past its top.
        let table = Table { points: bowed().points[6..=18].to_vec(), ..bowed() };
        let mut e = routed();
        e.load(&table.to_bytes());
        e.note_on(127, 100);
        let pitch = block(e.as_mut(), PITCH, BLOCK, &stopped());
        assert!(near(pitch[0], table.points.last().unwrap().0));
        assert!(e.readout().unwrap().clamped);
    }

    #[test]
    fn the_table_is_saved_and_loaded() {
        let table = bowed();
        let mut e = routed();
        assert!(e.save().is_empty());
        e.load(&table.to_bytes());
        assert_eq!(e.save(), table.to_bytes());
        let r = e.readout().unwrap();
        assert_eq!(r.state, TuneState::Idle);
        assert_eq!(r.tuned_at, Some(1234));
        assert_eq!(r.worst_cents, Some(1.5));
        assert_eq!(r.range, Some(table.range()));
        e.load(b"junk");
        assert_eq!(e.readout().unwrap().state, TuneState::NotTuned);
        assert!(e.save().is_empty());
    }

    #[test]
    fn output_param_text_reads_like_out_2_l() {
        let e = engine();
        assert_eq!(e.text(PITCH_OUT, 0.0), "None");
        assert_eq!(e.text(PITCH_OUT, 3.0 / 32.0), "Out 2 L");
        assert_eq!(e.text(GATE_OUT, 4.0 / 32.0), "Out 2 R");
        assert_eq!(e.text(GATE_OUT, 1.0), "Out 16 R", "the last position is the last channel");
        assert!(PARAMS.iter().filter(|p| p.id <= MOD_OUT).all(|p| p.steps == 32));
        assert_eq!(e.text(INPUT, 0.0), "None");
        assert_eq!(e.text(INPUT, 2.0 / 16.0), "In 2 L+R");
        assert_eq!(e.text(DIVISION, 2.0 / 7.0), "1/16");
    }

    #[test]
    fn process_writes_only_to_preallocated_buffers_in_a_held_note() {
        // The crate has no allocation counter: what `process` writes to must not move or grow.
        let mut z = Zhora::new(SR);
        defaults(&mut z);
        route(&mut z);
        let spots: Vec<_> = z.cv.iter().map(|b| (b.as_ptr(), b.capacity())).collect();
        assert!(spots.iter().all(|&(_, c)| c >= crate::MAX_BLOCK));
        assert!(z.stack.capacity() >= 128);
        let t = Transport { playing: true, ..stopped() };
        z.note_on(60, 100);
        for _ in 0..50 {
            run(&mut z, crate::MAX_BLOCK, &t);
        }
        assert_eq!(z.cv.iter().map(|b| (b.as_ptr(), b.capacity())).collect::<Vec<_>>(), spots);
    }

    #[test]
    fn class_id_pages_and_is_zhora() {
        assert_eq!(&CLASS.cid, b"DECKARD:Zhora001");
        assert!(crate::native::find(b"DECKARD:Zhora001").is_some());
        assert!(is_zhora(&CLASS.plugin_id()));
        assert!(!is_zhora("DECKARD:VST3:INSTR:00"));
        let page = |n: usize| CLASS.pages[n].knobs.iter().map(|k| k.0).collect::<Vec<_>>();
        assert_eq!(page(0), [1, 2, 3, 4, 30, 31, 32, 33]);
        assert_eq!(page(1), [10, 11, 12, 13, 30, 31, 32, 33]);
        assert_eq!(page(2), [20, 21, 22, 23, 30, 31, 32, 33]);
        assert_eq!(page(3), [5, 24, 6, 25, 30, 31, 32, 33]);
    }

    #[test]
    fn input_pair_follows_the_input_param() {
        let mut e = engine();
        assert_eq!(e.input_pair(), None);
        e.set_param(INPUT, 3.0 / 16.0);
        assert_eq!(e.input_pair(), Some(2));
    }

    #[test]
    fn a_note_in_the_same_span_as_a_reset_still_gets_its_gap() {
        let mut e = routed();
        let t = stopped();
        e.note_on(60, 100);
        run(e.as_mut(), BLOCK, &t);
        e.reset();
        e.note_on(64, 100);
        let gate = block(e.as_mut(), GATE, BLOCK, &t);
        assert!(gate[..96].iter().all(|&v| v == 0.0), "the gate was high when the reset landed");
        assert!(near(gate[96], 0.5));
    }

    #[test]
    fn readout_says_when_hold_is_holding() {
        let mut e = routed();
        e.set_param(HOLD, 1.0);
        run(e.as_mut(), BLOCK, &stopped());
        assert!(!e.readout().unwrap().holding, "a saved value is not a press");
        e.set_param(HOLD, 0.0);
        e.set_param(HOLD, 1.0);
        assert!(e.readout().unwrap().holding);
        e.set_param(HOLD, 0.0);
        assert!(!e.readout().unwrap().holding);
    }

    #[test]
    fn a_dead_worker_ends_the_run_and_keeps_the_table() {
        let mut z = Zhora::new(SR);
        defaults(&mut z);
        z.load(&bowed().to_bytes());
        z.set_param(INPUT, 1.0 / 16.0);
        let t = stopped();
        run(&mut z, BLOCK, &t);
        z.set_param(CHECK, 1.0);
        // The worker's end of the input is gone.
        let (gone, rx) = sync_channel(1);
        drop(rx);
        z.blocks = gone;
        z.input(&[0.1; HOP], &[0.1; HOP]);
        run(&mut z, BLOCK, &t);
        let r = z.readout().unwrap();
        assert_eq!(r.state, TuneState::Idle);
        assert!(r.message.is_some());
        assert_eq!(z.save(), bowed().to_bytes());
    }

    #[test]
    fn a_retired_report_waits_when_the_queue_is_full() {
        let mut z = Zhora::new(SR);
        defaults(&mut z);
        z.load(&bowed().to_bytes());
        z.set_param(INPUT, 1.0 / 16.0);
        run(&mut z, BLOCK, &stopped());
        z.set_param(CHECK, 1.0);
        let id = z.running.unwrap().id;
        // A full queue, and a finished Check waiting.
        let (full, cmd_rx) = sync_channel(1);
        full.try_send(Cmd::Cancel).unwrap();
        z.cmds = full;
        let (done, finished) = sync_channel(4);
        z.finished = finished;
        done.send(Report { id, worst_cents: Some(2.0), off: vec![60], ..Report::default() }).unwrap();
        done.send(Report { id: id + 7, off: vec![61], ..Report::default() }).unwrap();
        run(&mut z, BLOCK, &stopped());
        assert_eq!(z.readout().unwrap().worst_cents, Some(2.0));
        assert!(z.retiring.is_some(), "kept, not freed");
        // Room again: it goes, and so does the next.
        assert!(matches!(cmd_rx.try_recv(), Ok(Cmd::Cancel)));
        run(&mut z, BLOCK, &stopped());
        assert!(matches!(cmd_rx.try_recv(), Ok(Cmd::Retire(_))));
        run(&mut z, BLOCK, &stopped());
        assert!(matches!(cmd_rx.try_recv(), Ok(Cmd::Retire(r)) if r.off == [61]));
        assert!(z.retiring.is_none());
    }

    #[test]
    fn live_waits_for_a_settled_note_after_hold() {
        let table = bowed();
        let mut z = Zhora::new(SR);
        defaults(&mut z);
        route(&mut z);
        z.load(&table.to_bytes());
        z.set_param(LIVE, 1.0);
        let t = stopped();
        z.note_on(69, 100);
        for _ in 0..100 {
            run(&mut z, 1024, &t);
        }
        z.set_param(HOLD, 1.0);
        for _ in 0..10 {
            run(&mut z, 1024, &t);
        }
        z.set_param(HOLD, 0.0);
        // The first reading after Hold is of the hold note's tail.
        z.shared.publish(Some(Pitch { hz: 440.0 * (20.0f32 / 1200.0).exp2(), clarity: 0.99 }));
        run(&mut z, 1024, &t);
        assert_eq!(z.trim_semis, 0.0);
    }

    /// Live must trim the whole drift, which it can only do if it is told what the table
    /// believes it is sending with the trim included.
    #[test]
    fn live_trims_all_of_a_drift_on_top_of_the_table() {
        let points: Vec<(f32, f32)> =
            (0..=2).map(|i| (i as f32 * 0.25 - 0.25, (super::tune::note_hz(30.0 + 30.0 * i as f32)).log2())).collect();
        let table = Table { points, tuned_at: 1, worst_cents: 0.0 };
        let voice = |v: f32| super::tune::note_hz(60.0 + v * 120.0) * (10.0f32 / 1200.0).exp2();
        let mut z = Zhora::new(SR);
        defaults(&mut z);
        route(&mut z);
        z.load(&table.to_bytes());
        z.set_param(LIVE, 1.0);
        let t = stopped();
        z.note_on(69, 100);
        for _ in 0..1600 {
            run(&mut z, 1024, &t);
            let v = z.cv(PITCH, 1024).unwrap().1[1023];
            z.shared.publish(Some(Pitch { hz: voice(v), clarity: 0.99 }));
        }
        let v = z.cv(PITCH, 1024).unwrap().1[1023];
        let off = super::tune::cents(voice(v), 440.0);
        assert!(off.abs() < 1.0, "the voice is {off} cents off");
        assert_eq!(z.readout().unwrap().live_locked, Some(true));
    }

    /// Feeds the real worker a simulated voice, one hop at a time, waiting for each to be
    /// analysed so none is lost whatever the machine's speed.
    fn feed_hop(z: &mut Zhora, samples: &[f32], hops: &mut u32, deadline: Instant) {
        z.input(samples, samples);
        *hops = hops.wrapping_add(1);
        while z.shared.seq.load(Ordering::Acquire) != *hops {
            assert!(Instant::now() < deadline, "the worker fell silent");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn the_tuner_hears_the_input_through_the_worker() {
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut z = Zhora::new(SR);
        defaults(&mut z);
        z.set_param(INPUT, 1.0 / 16.0);
        let t = stopped();
        let mut hops = 0;
        let mut phase = 0f32;
        for _ in 0..8 {
            let mut b = [0f32; 1024];
            for s in &mut b {
                *s = 0.5 * (phase * std::f32::consts::TAU).sin();
                phase = (phase + 440.0 / SR as f32).fract();
            }
            run(&mut z, 1024, &t);
            feed_hop(&mut z, &b, &mut hops, deadline);
        }
        run(&mut z, 16, &t);
        let (note, cents) = z.readout().unwrap().note.expect("a pitch");
        assert_eq!(note, 69);
        assert!(cents.abs() < 1.0);
        // Under the level gate: no pitch.
        for _ in 0..8 {
            let mut b = [0f32; 1024];
            for s in &mut b {
                *s = 0.0005 * (phase * std::f32::consts::TAU).sin();
                phase = (phase + 440.0 / SR as f32).fract();
            }
            run(&mut z, 1024, &t);
            feed_hop(&mut z, &b, &mut hops, deadline);
        }
        run(&mut z, 16, &t);
        assert_eq!(z.readout().unwrap().note, None);
    }

    #[test]
    fn a_tune_through_the_worker_builds_a_table_while_stopped() {
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut z = Zhora::new(SR);
        defaults(&mut z);
        route(&mut z);
        z.set_param(INPUT, 1.0 / 16.0);
        let t = stopped();
        run(&mut z, 1024, &t);
        z.set_param(TUNE, 1.0);
        let (mut hops, mut phase) = (0, 0f64);
        loop {
            run(&mut z, 1024, &t);
            if z.readout().unwrap().state != TuneState::Tuning {
                break;
            }
            let v = z.cv(PITCH, 1024).unwrap().1[1023] as f64;
            // A voice that speaks over two octaves either side of C3 and is silent past that.
            let hz = (v.abs() <= 0.2 + 1e-4).then(|| 261.63 * 2f64.powf(v * 10.0));
            let mut b = [0f32; 1024];
            if let Some(hz) = hz {
                for s in &mut b {
                    *s = (0.5 * (phase * std::f64::consts::TAU).sin()) as f32;
                    phase = (phase + hz / SR).fract();
                }
            }
            feed_hop(&mut z, &b, &mut hops, deadline);
            assert!(Instant::now() < deadline, "the tune never finished");
        }
        let r = z.readout().unwrap();
        assert_eq!(r.message, None);
        assert_eq!(r.state, TuneState::Idle);
        assert!(r.worst_cents.is_some_and(|w| w < 5.0), "{:?}", r.worst_cents);
        assert!(r.tuned_at.is_some_and(|at| at > 1_000_000_000), "stamped");
        assert!(r.range.is_some_and(|(lo, hi)| lo <= 40 && hi >= 80), "{:?}", r.range);
        assert!(r.off.is_empty());
        // The table it made plays in tune.
        z.note_on(60, 100);
        let pitch = block(&mut z, PITCH, 256, &t);
        let hz = 261.63 * 2f64.powf(pitch[255] as f64 * 10.0);
        assert!(1200.0 * (hz / 261.6256).log2().abs() < 5.0);
    }
}
