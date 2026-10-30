//! Rendering sounds ahead of the audio thread (docs/engine-design.md, "Render ahead").
//!
//! A sound that can't receive live input right now (not in the focused group, no MIDI
//! or audio input, no side-chain) doesn't need to be computed at the audio deadline:
//! its notes come from the timeline, which is known in advance. Such a sound becomes a
//! [`Player`]: it owns the sound's Sampler, plug-in and effects, and a background
//! thread ([`Driver`]) keeps it a few blocks ahead of the audio thread, writing its
//! output (after its effects, before its level) into a [`Ring`]. The audio thread
//! mixes the ring with the sound's level, pan and mute, meters and taps, as it does
//! for sounds it renders itself; a plug-in that takes longer than a block now and then
//! no longer costs a dropout.
//!
//! The engine stays in charge of the sequence: each block it looks a little ahead
//! (`Engine::foresee`) and sends the players their notes early, stamped with their
//! output frame ([`Msg::At`]). Players move between ahead and live at a frame boundary
//! ([`Msg::Install`], [`Msg::Return`]), so nothing plays twice or goes missing.

use crate::queue::Queue;
use crate::sampler::{Context, SamplerRt};
use crate::{FxChain, InstrumentRt, JOB_FRAMES, PPQ, SoundBufs, render_sound};
use deckard_model::MidiKind;
use deckard_plugins::Transport;
use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// A sound's playing parts, moved between the audio thread and a player.
pub(crate) struct Parts {
    pub smp: Option<SamplerRt>,
    pub inst: Option<InstrumentRt>,
    pub chain: Option<FxChain>,
    /// Where the player renders before copying into its ring.
    pub scratch: SoundBufs,
    /// For an edit: keep the playing Sampler, plug-in, effect chain (instead of these).
    pub keep: [bool; 3],
    /// For an edit: the kept Sampler's new settings, the kept chain's effects' new
    /// settings (slot, parameters), the kept plug-in's gain and mute.
    pub settings: Option<Box<crate::sampler::Settings>>,
    pub params: Vec<(usize, Vec<(String, f64)>)>,
    pub inst_gain: ((f32, f32), bool),
}

impl Parts {
    pub fn empty() -> Parts {
        Parts {
            smp: None,
            inst: None,
            chain: None,
            scratch: SoundBufs::new((1.0, 1.0)),
            keep: [false; 3],
            settings: None,
            params: Vec::new(),
            inst_gain: ((1.0, 1.0), false),
        }
    }
}

/// Something for a player to do at a frame.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Ev {
    /// A note (`gate`: frames until its note-off; none: until released).
    NoteOn {
        id: u64,
        pitch: i64,
        velocity: i64,
        gate: Option<u64>,
        beats: f64,
    },
    /// A note held live ends.
    Release {
        pitch: i64,
    },
    Control(MidiKind),
    /// The Audio plug-in's loop: play take `.0` from song beat `.1` on, or stop (none).
    Cue(Option<usize>, f64),
    /// Where the song is from this frame on (for tempo-synced effects and plug-ins).
    Position {
        frame: u64,
        playing: bool,
        tempo: f64,
        numerator: i32,
        denominator: i32,
    },
    /// Stop: voices fade, plug-ins' notes end.
    Stop,
}

pub(crate) enum Msg {
    /// `Ev` at output frame `.0`; `.2`: it came live (not from the timeline).
    At(u64, Ev, bool),
    /// Play from output frame `at`, with these parts and note-offs pending
    /// (frame, note id, pitch).
    Install { at: u64, parts: Box<Parts>, offs: Vec<(u64, u64, i64)> },
    /// An edit, from output frame `.0` on: these fresh parts take over the playing
    /// ones' state.
    Adopt(u64, Box<Parts>),
    /// Render up to output frame `.0` (no further), then hand the parts back: the
    /// audio thread plays the sound from there.
    Return(u64),
}

/// What came back from players, for the output to free off the audio thread.
pub struct Junk(#[allow(dead_code)] pub(crate) JunkKind);

#[allow(dead_code)] // held only to be dropped
pub(crate) enum JunkKind {
    Parts(Box<Parts>),
    Offs(Vec<(u64, u64, i64)>),
    Live(Vec<(u64, Ev)>),
}

/// What players send back to the audio thread.
pub(crate) enum Back {
    /// The parts of sound (`g`, `s`), rendered up to frame `at` (the audio thread plays
    /// them from there), with their pending note-offs and live events not yet played.
    Returned { g: usize, s: usize, at: u64, parts: Box<Parts>, offs: Vec<(u64, u64, i64)>, live: Vec<(u64, Ev)> },
    /// An emptied box (an edit's parts came in it), to move parts in again.
    Spare { g: usize, s: usize, parts: Box<Parts> },
}

/// A sound's output rendered ahead: the Sampler's and the plug-in's (interleaved
/// stereo), by absolute output frame. One writer (its player), one reader (the audio
/// thread).
pub(crate) struct Ring {
    cap: u64,
    smp: UnsafeCell<Vec<f32>>,
    inst: UnsafeCell<Vec<f32>>,
    /// Frames before this are written.
    written: AtomicU64,
    /// Frames before this are read (may be overwritten).
    read: AtomicU64,
}

// The writer only writes frames the reader has finished with, and the reader only
// reads frames the writer has published (`written`, `read`).
unsafe impl Sync for Ring {}
unsafe impl Send for Ring {}

impl Ring {
    pub fn new(frames: usize) -> Ring {
        let cap = frames.next_power_of_two();
        Ring {
            cap: cap as u64,
            smp: UnsafeCell::new(vec![0.0; cap * 2]),
            inst: UnsafeCell::new(vec![0.0; cap * 2]),
            written: AtomicU64::new(0),
            read: AtomicU64::new(0),
        }
    }

    /// Frames written so far (absolute).
    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Acquire)
    }

    /// Restart at frame `at` (both ends): the reader calls it before a player starts
    /// there.
    pub fn restart(&self, at: u64) {
        self.read.store(at, Ordering::Release);
        self.written.store(at, Ordering::Release);
    }

    /// Frames the writer may write from `pos` without overtaking the reader.
    fn room(&self, pos: u64) -> u64 {
        self.cap.saturating_sub(pos.saturating_sub(self.read.load(Ordering::Acquire)))
    }

    /// Write `n` frames at `pos` (the writer; within [`Ring::room`]), then publish them.
    fn write(&self, pos: u64, smp: &[f32], inst: &[f32], n: usize) {
        for (dst, src) in [(&self.smp, smp), (&self.inst, inst)] {
            let buf = unsafe { &mut *dst.get() };
            for f in 0..n {
                let i = ((pos + f as u64) & (self.cap - 1)) as usize * 2;
                buf[i] = src[f * 2];
                buf[i + 1] = src[f * 2 + 1];
            }
        }
        self.written.store(pos + n as u64, Ordering::Release);
    }

    /// The reader is done with frames before `upto` (also those it skipped).
    pub fn read_to(&self, upto: u64) {
        self.read.fetch_max(upto, Ordering::AcqRel);
    }

    /// Copy frames `pos..pos + n` (published) into `smp` and `inst`, and let the writer
    /// have them (the reader).
    pub fn read_into(&self, pos: u64, n: usize, smp: &mut [f32], inst: &mut [f32]) {
        for (src, dst) in [(&self.smp, smp), (&self.inst, inst)] {
            let buf = unsafe { &*src.get() };
            for f in 0..n {
                let i = ((pos + f as u64) & (self.cap - 1)) as usize * 2;
                dst[f * 2] = buf[i];
                dst[f * 2 + 1] = buf[i + 1];
            }
        }
        self.read.store(pos + n as u64, Ordering::Release);
    }
}

/// One sound rendered ahead (owned by the driver's thread).
pub(crate) struct Player {
    g: usize,
    s: usize,
    parts: Option<Box<Parts>>,
    pub inbox: Arc<Queue<Msg>>,
    /// Events received, not yet due: (frame, event, came live).
    pending: Vec<(u64, Ev, bool)>,
    /// Note-offs to play: (frame, note id, pitch).
    offs: Vec<(u64, u64, i64)>,
    /// Samplers of the sound replaced by an edit, ringing out.
    retired: Vec<SamplerRt>,
    pub ring: Arc<Ring>,
    /// Rendered up to this output frame.
    pos: u64,
    /// The song position from frame `.0`: (frame, song frame, playing, tempo, num, den).
    position: (u64, u64, bool, f64, i32, i32),
    pub voices: Arc<AtomicUsize>,
    sr: f64,
    tmp: Vec<f32>,
    /// Asked back at this frame.
    return_at: Option<u64>,
    /// Edits waiting for their frame.
    edits: Vec<(u64, Box<Parts>)>,
}

/// Events a player holds at once.
const PENDING_ROOM: usize = 4096;

impl Player {
    pub fn new(g: usize, s: usize, sr: f64, ring_frames: usize) -> Player {
        Player {
            g,
            s,
            parts: None,
            inbox: Arc::new(Queue::new(PENDING_ROOM)),
            pending: Vec::with_capacity(PENDING_ROOM),
            offs: Vec::with_capacity(512),
            retired: Vec::with_capacity(8),
            ring: Arc::new(Ring::new(ring_frames)),
            pos: 0,
            position: (0, 0, false, 120.0, 4, 4),
            voices: Arc::default(),
            sr,
            tmp: vec![0.0; JOB_FRAMES * 2],
            return_at: None,
            edits: Vec::with_capacity(16),
        }
    }

    fn take_messages(&mut self, back: &Queue<Back>) {
        while let Some(m) = self.inbox.pop() {
            match m {
                Msg::At(at, ev, live) => {
                    if self.pending.len() < self.pending.capacity() {
                        self.pending.push((at, ev, live));
                    } else {
                        self.apply(ev);
                    }
                }
                Msg::Install { at, parts, offs } => {
                    self.parts = Some(parts);
                    self.pos = at;
                    self.offs = offs;
                    // What was sent for it from now on (its notes and position came
                    // just before).
                    self.pending.retain(|p| p.0 >= at);
                }
                Msg::Adopt(at, fresh) => {
                    if self.edits.len() < self.edits.capacity() {
                        self.edits.push((at, fresh));
                    } else {
                        self.adopt(fresh, back);
                    }
                }
                Msg::Return(at) => {
                    self.return_at = Some(at);
                    self.hand_back(back);
                }
            }
        }
    }

    /// Take an edit: fresh parts take over the playing ones' state (as the engine
    /// adopts one), or keep them where the edit didn't touch them.
    fn adopt(&mut self, mut fresh: Box<Parts>, back: &Queue<Back>) {
        match self.parts.as_mut() {
            Some(old) => {
                // Parts the edit didn't touch stay (with its small changes).
                if fresh.keep[0] {
                    fresh.smp = old.smp.take();
                    if let (Some(smp), Some(st)) = (fresh.smp.as_mut(), fresh.settings.as_mut()) {
                        smp.swap_settings(st);
                    }
                }
                if fresh.keep[1] {
                    fresh.inst = old.inst.take();
                    if let Some(i) = fresh.inst.as_mut() {
                        (i.gain, i.muted) = fresh.inst_gain;
                    }
                }
                if fresh.keep[2] {
                    fresh.chain = old.chain.take();
                    let mut params = std::mem::take(&mut fresh.params);
                    if let Some(c) = fresh.chain.as_mut() {
                        c.set_params(&mut params);
                    }
                }
                // As the engine adopts an edit: voices carry over, effects keep
                // their state; a Sampler that went away rings out.
                match (fresh.smp.as_mut(), old.smp.take()) {
                    (Some(new), Some(o)) => new.adopt(o),
                    (None, Some(mut o)) if o.voices() > 0 && self.retired.len() < self.retired.capacity() => {
                        o.fade_all();
                        self.retired.push(o);
                    }
                    _ => {}
                }
                if let (Some(new), Some(o)) = (fresh.chain.as_mut(), old.chain.as_mut()) {
                    new.adopt(o);
                }
                std::mem::swap(&mut fresh.scratch, &mut old.scratch);
                std::mem::swap(old, &mut fresh);
                // `fresh` now holds what was replaced: freed here (off the
                // audio thread), and the box goes back for the next edit.
                (fresh.smp, fresh.inst, fresh.chain) = (None, None, None);
                (fresh.keep, fresh.settings) = ([false; 3], None);
                fresh.params.clear();
                let _ = back.push(Back::Spare { g: self.g, s: self.s, parts: fresh });
            }
            None => drop(fresh),
        }
    }

    /// Asked back and rendered up to the frame asked: hand the parts back.
    fn hand_back(&mut self, back: &Queue<Back>) {
        let Some(at) = self.return_at else { return };
        if self.pos < at && self.parts.is_some() {
            return;
        }
        self.return_at = None;
        // Edits still waiting take effect now (the sound plays on with them).
        while !self.edits.is_empty() {
            let (_, fresh) = self.edits.remove(0);
            self.adopt(fresh, back);
        }
        if let Some(parts) = self.parts.take() {
            // Live input not played yet goes back with it (timeline events the audio
            // thread plays itself).
            let live = self.pending.iter().filter(|p| p.2).map(|p| (p.0, p.1)).collect();
            self.pending.clear();
            let offs = std::mem::replace(&mut self.offs, Vec::with_capacity(512));
            let _ = back.push(Back::Returned { g: self.g, s: self.s, at: self.pos, parts, offs, live });
        }
    }

    fn apply(&mut self, ev: Ev) {
        let pos = self.pos;
        let Some(p) = self.parts.as_mut() else { return };
        match ev {
            Ev::NoteOn { id, pitch, velocity, gate, beats } => {
                if let Some(inst) = &p.inst {
                    if inst.muted {
                        return;
                    }
                    if let Ok(mut pl) = inst.plugin.lock() {
                        pl.note_on(pitch, velocity);
                    }
                    self.offs.retain(|o| o.2 != pitch);
                    if let Some(g) = gate {
                        self.offs.push((pos + g, 0, pitch));
                    }
                } else if let Some(smp) = p.smp.as_mut() {
                    smp.note_on(id, pitch, velocity.clamp(0, 127), &Context { tempo: self.position.3, beats });
                    if let Some(g) = gate {
                        self.offs.push((pos + g, id, pitch));
                    }
                }
            }
            Ev::Release { pitch } => {
                if let Some(smp) = p.smp.as_mut() {
                    smp.release_note(pitch);
                }
                if let Some(inst) = &p.inst
                    && let Ok(mut pl) = inst.plugin.lock()
                {
                    pl.note_off(pitch);
                }
                self.offs.retain(|o| o.2 != pitch);
            }
            Ev::Cue(take, origin) => {
                if let Some(smp) = p.smp.as_mut() {
                    smp.cue(take.map(|t| (t, origin)));
                }
            }
            Ev::Control(kind) => {
                if let Some(smp) = p.smp.as_mut() {
                    smp.control(kind);
                }
                if let Some(inst) = &p.inst
                    && let Ok(mut pl) = inst.plugin.lock()
                {
                    match kind {
                        MidiKind::Controller { number, value } => pl.control_change(number, value),
                        MidiKind::PitchBend(v) => pl.pitch_bend(v),
                        // Plug-ins take no channel pressure yet (part 2 of the expression work).
                        MidiKind::ChannelPressure(_) => {}
                    }
                }
            }
            Ev::Position { .. } => {}
            Ev::Stop => {
                if let Some(smp) = p.smp.as_mut() {
                    smp.fade_all();
                }
                if let Some(inst) = &p.inst
                    && let Ok(mut pl) = inst.plugin.lock()
                {
                    pl.all_notes_off();
                }
                self.offs.clear();
            }
        }
    }

    /// Play what is due at the current frame.
    fn fire(&mut self) {
        let pos = self.pos;
        // Events in the order they came.
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].0 <= pos {
                let (at, ev, _) = self.pending.remove(i);
                if let Ev::Position { frame, playing, tempo, numerator, denominator } = ev {
                    if at >= self.position.0 {
                        self.position = (at, frame, playing, tempo, numerator, denominator);
                    }
                } else {
                    self.apply(ev);
                }
            } else {
                i += 1;
            }
        }
        if self.offs.iter().any(|o| o.0 <= pos) {
            let Some(p) = self.parts.as_mut() else { return };
            self.offs.retain(|&(at, id, pitch)| {
                if at > pos {
                    return true;
                }
                if let Some(smp) = p.smp.as_mut() {
                    smp.note_off(id, pitch);
                } else if let Some(inst) = &p.inst
                    && let Ok(mut pl) = inst.plugin.lock()
                {
                    pl.note_off(pitch);
                }
                false
            });
        }
    }

    /// Render ahead up to output frame `target` (as far as the ring has room).
    fn render_to(&mut self, target: u64, back: &Queue<Back>) {
        // Asked back: exactly to that frame.
        let target = self.return_at.unwrap_or(target);
        while self.parts.is_some() && self.pos < target {
            let room = self.ring.room(self.pos);
            if room == 0 {
                break;
            }
            // Edits whose frame has come.
            while let Some(i) = self.edits.iter().position(|e| e.0 <= self.pos) {
                let (_, fresh) = self.edits.remove(i);
                self.adopt(fresh, back);
            }
            if self.parts.is_none() {
                break;
            }
            self.fire();
            let pos = self.pos;
            let mut end = target.min(pos + room.min(JOB_FRAMES as u64));
            for &(at, _) in &self.edits {
                if at > pos {
                    end = end.min(at);
                }
            }
            for &(at, ..) in &self.pending {
                if at > pos {
                    end = end.min(at);
                }
            }
            for &(at, ..) in &self.offs {
                if at > pos {
                    end = end.min(at);
                }
            }
            let n = (end - pos) as usize;
            let (start, song, playing, tempo, num, den) = self.position;
            let song_frame = if playing { song + (pos - start.min(pos)) } else { song };
            let spt = self.sr * 60.0 / (tempo * PPQ as f64);
            let base = Transport { tempo, playing, quarters: song_frame as f64 / spt / PPQ as f64, numerator: num, denominator: den };
            let p = self.parts.as_mut().unwrap();
            render_sound(p.smp.as_mut(), p.inst.as_ref(), p.chain.as_mut(), &mut p.scratch, (n, 0), song_frame, &base, spt, &[]);
            let b = &mut p.scratch;
            if !b.smp_out {
                b.smp[..n * 2].fill(0.0);
            }
            if !b.inst_out {
                b.inst[..n * 2].fill(0.0);
            }
            // Samplers an edit replaced ring out in the Sampler's output.
            if !self.retired.is_empty() {
                self.tmp[..n * 2].fill(0.0);
                for r in &mut self.retired {
                    r.render(&mut self.tmp[..n * 2]);
                }
                self.retired.retain(|r| r.voices() > 0);
                for (o, v) in b.smp[..n * 2].iter_mut().zip(&self.tmp[..n * 2]) {
                    *o += v;
                }
            }
            self.ring.write(pos, &b.smp, &b.inst, n);
            self.pos = end;
            let voices = p.smp.as_ref().map_or(0, SamplerRt::voices) + self.retired.iter().map(SamplerRt::voices).sum::<usize>();
            self.voices.store(voices, Ordering::Relaxed);
        }
    }
}

/// What the audio thread shares with the driver.
pub(crate) struct Shared {
    /// The audio thread has played up to this output frame.
    pub now: AtomicU64,
    /// How far ahead of `now` players render (frames).
    pub lookahead: AtomicU64,
    pub quit: AtomicBool,
    pub back: Queue<Back>,
}

/// The thread that keeps players ahead.
pub(crate) struct Driver {
    pub shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
    handle: std::thread::Thread,
}

/// A raw pointer to a player, for the render pool (each job has its own player).
#[derive(Clone, Copy)]
struct PlayerPtr(*mut Player);
unsafe impl Send for PlayerPtr {}
unsafe impl Sync for PlayerPtr {}

impl Driver {
    /// Start a driver for `players`, rendering `lookahead` frames ahead, with `workers`
    /// threads to render players side by side.
    pub fn start(players: Vec<Player>, lookahead: u64, workers: usize, block: u32, rate: u32) -> Driver {
        let shared = Arc::new(Shared {
            now: AtomicU64::new(0),
            lookahead: AtomicU64::new(lookahead),
            quit: AtomicBool::new(false),
            back: Queue::new(1024),
        });
        let sh = shared.clone();
        let thread = std::thread::Builder::new()
            .name("DECKARD render ahead".into())
            .spawn(move || {
                crate::pool::promote_current(block, rate);
                let pool = (workers > 0).then(|| crate::pool::RenderPool::new(workers, block, rate));
                let mut players = players;
                let mut jobs: Vec<PlayerPtr> = Vec::with_capacity(players.len());
                while !sh.quit.load(Ordering::Acquire) {
                    let target = sh.now.load(Ordering::Acquire) + sh.lookahead.load(Ordering::Relaxed);
                    jobs.clear();
                    for p in &mut players {
                        p.take_messages(&sh.back);
                        let goal = p.return_at.unwrap_or(target);
                        if p.parts.is_some() && p.pos < goal && p.ring.room(p.pos) > 0 {
                            jobs.push(PlayerPtr(p as *mut Player));
                        }
                    }
                    if jobs.is_empty() {
                        std::thread::park_timeout(std::time::Duration::from_micros(500));
                        continue;
                    }
                    let js = &jobs;
                    let task = |i: usize| {
                        // Each job has its own player.
                        let ptr = js[i].0;
                        let p = unsafe { &mut *ptr };
                        p.render_to(target, &sh.back);
                    };
                    match &pool {
                        Some(pool) => pool.run(jobs.len(), &task),
                        None => (0..jobs.len()).for_each(task),
                    }
                    for p in &mut players {
                        p.hand_back(&sh.back);
                    }
                }
                // Replaced: notes the players' plug-ins hold end (the next engine may
                // play the same instances).
                for p in &players {
                    if let Some(inst) = p.parts.as_ref().and_then(|x| x.inst.as_ref())
                        && let Ok(mut pl) = inst.plugin.lock()
                    {
                        pl.all_notes_off();
                    }
                }
            })
            .expect("render-ahead thread");
        let handle = thread.thread().clone();
        Driver { shared, thread: Some(thread), handle }
    }

    /// The audio thread has played up to `now`: keep going.
    pub fn advance(&self, now: u64) {
        self.shared.now.store(now, Ordering::Release);
        self.handle.unpark();
    }

    pub fn wake(&self) {
        self.handle.unpark();
    }

    /// Stop rendering (the thread ends soon; it is joined when dropped).
    pub fn halt(&self) {
        self.shared.quit.store(true, Ordering::Release);
        self.handle.unpark();
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        self.halt();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
