//! Playing in time with others: following an outside clock (Ableton Link, incoming
//! MIDI clock) and sending MIDI clock.
//!
//! The output asks its [`Clock`] each block where the clock will be when the block is
//! heard, and the engine follows ([`Engine::follow`]): it takes the clock's tempo
//! (keeping the song's position: only the sequencer's clock moves, so nothing clicks)
//! and lines the song's beats up with the clock's, within the bar (Link) or exactly
//! (MIDI clock, whose beats count from the song's start). A song started while
//! following Link waits for the clock's next bar.

use crate::{Engine, PPQ};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where an outside clock is when the block about to be rendered is heard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Follow {
    /// BPM.
    pub tempo: f64,
    /// Beats (quarter notes): Link's on the song's bar (the quantum); MIDI clock's from
    /// the song's start.
    pub beat: f64,
    /// The beat is the song's position (MIDI clock), not only its phase in the bar.
    pub absolute: bool,
}

/// An outside clock the audio thread follows: asked once per block, without blocking
/// or allocating.
pub trait Clock: Send {
    /// Where the clock is `ahead` from now, on a bar of `quantum` beats; `None` while it
    /// isn't running (no MIDI clock coming in).
    fn follow(&mut self, ahead: Duration, quantum: f64) -> Option<Follow>;
}

/// Sending MIDI clock: to these ports, this much ahead of the audio (to make up for
/// what the receiving devices take).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClockOut {
    pub ports: Vec<Arc<str>>,
    pub early: Duration,
}

/// MIDI clock ticks per beat.
const CLOCKS_PER_BEAT: i64 = 24;
/// Song ticks per MIDI clock.
const TICKS_PER_CLOCK: i64 = PPQ / CLOCKS_PER_BEAT;

/// Corrections smaller than this many ticks move the sequencer's clock; larger ones
/// seek (a sixteenth).
const NUDGE_TICKS: f64 = PPQ as f64 / 4.0;

/// The engine's part in following and sending clock.
#[derive(Debug, Default)]
pub struct Sync {
    /// A start waits for the clock to say where it falls; how late (beats) it may come
    /// and still start at once.
    pending_start: bool,
    late: f64,
    /// Frames still to wait (silent) before the count-in or the song.
    pub(crate) waiting: u64,
    clock_out: Option<ClockOut>,
    /// The song's ticks since the clock sent started (they keep counting across loops
    /// and jumps); `None` while no clock was started.
    clock_pos: Option<f64>,
    /// When the current block is heard, and the engine's time at its start.
    heard: Option<(Instant, u64)>,
}

impl Sync {
    /// The song is waiting to start in time with the clock.
    pub fn waits(&self) -> bool {
        self.pending_start || self.waiting > 0
    }
}

impl Engine {
    /// Beats (quarter notes) in the song's bar: the quantum Link lines bars up on.
    pub fn quantum(&self) -> f64 {
        self.numerator.max(1) as f64 * 4.0 / self.denominator.max(1) as f64
    }

    /// Play at `bpm` from here (the song stays where it is).
    pub fn set_tempo(&mut self, bpm: f64) {
        if bpm.is_nan() || bpm <= 0.0 || bpm == self.tempo {
            return;
        }
        let ticks = self.position();
        self.tempo = bpm;
        self.frame = (ticks * self.samples_per_tick()).round().max(0.0) as u64;
        for u in self.fx.iter_mut().flatten().flatten().flat_map(|c| c.units.iter_mut()) {
            if let crate::FxUnit::Internal { fx, .. } = &mut u.fx {
                fx.set_tempo(bpm);
            }
        }
    }

    /// Start in time with the clock followed: the song waits until the clock's next
    /// follow says where (then the count-in, if any, and the song).
    pub fn sync_start(&mut self) {
        self.sync_start_within(0.0);
    }

    /// [`Engine::sync_start`] for a start the clock already made (Link's peers started
    /// it): up to `late` beats after its bar, the song starts at once, as far in as the
    /// clock is.
    pub fn sync_start_within(&mut self, late: f64) {
        self.sync.pending_start = true;
        self.sync.late = late;
        self.sequencing = false;
    }

    /// Follow a clock for the block about to be rendered (see the module's comment).
    pub fn follow(&mut self, f: &Follow) {
        self.set_tempo(f.tempo);
        let spt = self.samples_per_tick();
        let q = self.quantum() * PPQ as f64;
        let len = self.timeline.length as f64;
        let wrap = |t: f64| if self.looping && len > 0.0 { t.rem_euclid(len) } else { t.max(0.0) };
        let clock = f.beat * PPQ as f64;
        if std::mem::take(&mut self.sync.pending_start) {
            if f.absolute {
                self.seek(wrap(clock).round() as i64);
                self.sequencing = self.counting == 0;
            } else {
                // The song's start falls on the clock's beat with the same place in the
                // bar: wait for it.
                let pos = self.position();
                let wait = (pos - clock).rem_euclid(q);
                let past = q - wait;
                if past <= self.sync.late * PPQ as f64 {
                    if past > 1e-6 {
                        self.seek(wrap(pos + past).round() as i64);
                    }
                    self.sync.waiting = 0;
                } else {
                    self.sync.waiting = (wait * spt).round() as u64;
                }
                self.sequencing = self.sync.waiting == 0 && self.counting == 0;
            }
            return;
        }
        if !self.sequencing || self.sync.waiting > 0 || self.counting > 0 {
            return;
        }
        // How far the song is ahead of the clock (ticks; negative: behind).
        let pos = self.position();
        let ahead = if f.absolute {
            let d = pos - wrap(clock);
            if self.looping && len > 0.0 { (d + len / 2.0).rem_euclid(len) - len / 2.0 } else { d }
        } else {
            (pos - clock + q / 2.0).rem_euclid(q) - q / 2.0
        };
        if (ahead * spt).abs() < 1.0 {
            return;
        }
        if ahead.abs() < NUDGE_TICKS {
            self.frame = (self.frame as f64 - ahead * spt).round().max(0.0) as u64;
        } else {
            self.seek(wrap(pos - ahead).round() as i64);
        }
    }

    /// When the block about to be rendered will be heard (MIDI clock is timed by it).
    pub fn heard_at(&mut self, at: Instant) {
        self.sync.heard = Some((at, self.time));
    }

    /// Send MIDI clock (or stop sending it); the old setting comes back (to be freed
    /// off the audio thread).
    pub fn set_clock_out(&mut self, out: Option<ClockOut>) -> Option<ClockOut> {
        if out.is_none() {
            self.clock_stop();
        }
        std::mem::replace(&mut self.sync.clock_out, out)
    }

    /// When engine time `time` is heard, less the clock's lead; `None` offline.
    fn clock_due(&self, time: u64) -> Option<Instant> {
        let (at, t0) = self.sync.heard?;
        let early = self.sync.clock_out.as_ref().map_or(Duration::ZERO, |c| c.early);
        let after = Duration::from_secs_f64(time.saturating_sub(t0) as f64 / self.sr);
        Some((at + after).checked_sub(early).unwrap_or(at))
    }

    fn send_clock(&self, bytes: [u8; 3], len: u8, at: Option<Instant>) {
        let (Some(tx), Some(out)) = (&self.midi_tx, &self.sync.clock_out) else { return };
        for port in &out.ports {
            let _ = tx.try_send(crate::midi::MidiOut { port: port.clone(), bytes, len, at });
        }
    }

    /// The MIDI clock for the next `frames` of the song: its start (song position, then
    /// Start or Continue), a clock every 24th of a beat, Stop at its end.
    pub(crate) fn clock_span(&mut self, frames: u64) {
        if self.sync.clock_out.is_none() {
            return;
        }
        if !self.sequencing {
            if !self.sync.waits() && self.counting == 0 {
                self.clock_stop();
            }
            return;
        }
        let at = self.clock_due(self.time);
        let start = match self.sync.clock_pos {
            Some(p) => p,
            None => {
                let pos = self.position();
                let sixteenths = (pos / (PPQ / 4) as f64).round().clamp(0.0, 16383.0) as u16;
                self.send_clock([0xf2, (sixteenths & 0x7f) as u8, (sixteenths >> 7) as u8], 3, at);
                self.send_clock([if sixteenths == 0 { 0xfa } else { 0xfb }, 0, 0], 1, at);
                // Clocks from the song position (rounded to its 16th).
                sixteenths as f64 * (PPQ / 4) as f64
            }
        };
        let spt = self.samples_per_tick();
        let end = start + frames as f64 / spt;
        let mut k = (start / TICKS_PER_CLOCK as f64).ceil() as i64;
        while ((k * TICKS_PER_CLOCK) as f64) < end {
            let offset = ((k * TICKS_PER_CLOCK) as f64 - start) * spt;
            let at = self.clock_due(self.time + offset.round() as u64);
            self.send_clock([0xf8, 0, 0], 1, at);
            k += 1;
        }
        self.sync.clock_pos = Some(end);
    }

    /// Stop, if the clock sent was started.
    pub(crate) fn clock_stop(&mut self) {
        if self.sync.clock_pos.take().is_some() {
            // (After the clocks already on their way.)
            self.send_clock([0xfc, 0, 0], 1, self.clock_due(self.time));
        }
    }
}

/// MIDI's real-time and song position messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMsg {
    Clock,
    Start,
    Continue,
    Stop,
    /// Song Position Pointer: 16ths from the song's start.
    SongPosition(u16),
}

impl SyncMsg {
    pub fn parse(b: &[u8]) -> Option<SyncMsg> {
        match b {
            [0xf8, ..] => Some(SyncMsg::Clock),
            [0xfa, ..] => Some(SyncMsg::Start),
            [0xfb, ..] => Some(SyncMsg::Continue),
            [0xfc, ..] => Some(SyncMsg::Stop),
            [0xf2, lsb, msb, ..] => Some(SyncMsg::SongPosition((*msb as u16) << 7 | *lsb as u16)),
            _ => None,
        }
    }
}

/// What incoming MIDI clock asks of the transport: start (from the top), continue (from
/// the song position, in beats), or stop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transport {
    Start(f64),
    Continue(f64),
    Stop,
}

/// Follows incoming MIDI clock: fed by the MIDI input's thread, read by the audio
/// thread (never waiting: a block that finds it busy goes on as it was) and the app.
#[derive(Debug, Default)]
pub struct ClockFollower {
    state: Mutex<Follower>,
}

#[derive(Debug, Default)]
struct Follower {
    /// Start or Continue came, and no Stop since.
    running: bool,
    /// Where the next clock falls (beats from the song's start).
    next_beat: f64,
    /// The last clock: when, and its beat (while running).
    last: Option<Instant>,
    last_beat: Option<f64>,
    /// Seconds between clocks, smoothed, and clocks since that didn't fit it.
    interval: Option<f64>,
    outliers: u8,
    /// Seconds the song plays after the clock (negative: before it).
    offset: f64,
    transport: Option<Transport>,
}

impl ClockFollower {
    /// A message from the clock's port, and when it came.
    pub fn message(&self, msg: SyncMsg, at: Instant) {
        let Ok(mut f) = self.state.lock() else { return };
        match msg {
            SyncMsg::Clock => {
                if let Some(prev) = f.last {
                    let dt = at.saturating_duration_since(prev).as_secs_f64();
                    match f.interval {
                        Some(i) if dt > i * 0.7 && dt < i * 1.3 => {
                            f.interval = Some(i + (dt - i) * 0.1);
                            f.outliers = 0;
                        }
                        // A gap (a clock lost, Start) is let go; a new tempo is taken
                        // once a few clocks agree on it.
                        Some(_) if f.outliers < 3 => f.outliers += 1,
                        _ if dt > 0.0 && dt < 0.25 => (f.interval, f.outliers) = (Some(dt), 0),
                        _ => {}
                    }
                }
                f.last = Some(at);
                if f.running {
                    f.last_beat = Some(f.next_beat);
                    f.next_beat += 1.0 / CLOCKS_PER_BEAT as f64;
                }
            }
            SyncMsg::Start => {
                (f.running, f.next_beat, f.last_beat) = (true, 0.0, None);
                f.transport = Some(Transport::Start(0.0));
            }
            SyncMsg::Continue => {
                (f.running, f.last_beat) = (true, None);
                f.transport = Some(Transport::Continue(f.next_beat));
            }
            SyncMsg::Stop => {
                (f.running, f.last_beat) = (false, None);
                f.transport = Some(Transport::Stop);
            }
            SyncMsg::SongPosition(n) => {
                if !f.running {
                    f.next_beat = n as f64 / 4.0;
                }
            }
        }
    }

    /// What the clock last asked of the transport, once.
    pub fn take_transport(&self) -> Option<Transport> {
        self.state.lock().ok()?.transport.take()
    }

    /// The clock's tempo, once clocks have come.
    pub fn tempo(&self) -> Option<f64> {
        let f = self.state.lock().ok()?;
        f.interval.map(|i| 60.0 / (i * CLOCKS_PER_BEAT as f64))
    }

    /// Play `seconds` after the clock (negative: before it).
    pub fn set_offset(&self, seconds: f64) {
        if let Ok(mut f) = self.state.lock() {
            f.offset = seconds;
        }
    }

    /// Where the song should be at `when`: from the last clock, at most a clock and a
    /// half on. `None` until the clock runs (and its first clock came).
    pub fn at(&self, when: Instant) -> Option<Follow> {
        let f = self.state.lock().ok()?;
        Self::follow_at(&f, when)
    }

    fn follow_at(f: &Follower, when: Instant) -> Option<Follow> {
        let (true, Some(last), Some(beat), Some(interval)) = (f.running, f.last, f.last_beat, f.interval) else { return None };
        let since =
            if when >= last { when.duration_since(last).as_secs_f64() } else { -last.duration_since(when).as_secs_f64() } - f.offset;
        let clocks = (since / interval).clamp(-1.5, 1.5);
        Some(Follow { tempo: 60.0 / (interval * CLOCKS_PER_BEAT as f64), beat: beat + clocks / CLOCKS_PER_BEAT as f64, absolute: true })
    }

    /// The audio thread's [`Clock`] for this follower.
    pub fn clock(self: &Arc<Self>) -> Box<dyn Clock> {
        Box::new(FollowerClock(self.clone()))
    }
}

struct FollowerClock(Arc<ClockFollower>);

impl Clock for FollowerClock {
    fn follow(&mut self, ahead: Duration, _quantum: f64) -> Option<Follow> {
        let f = self.0.state.try_lock().ok()?;
        ClockFollower::follow_at(&f, Instant::now() + ahead)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{click_engine_for, project};
    use crate::{Engine, PPQ};
    use std::time::{Duration, Instant};

    fn onsets(e: &mut Engine, frames: usize) -> Vec<usize> {
        let mut out = vec![0f32; 2 * frames];
        e.process(&mut out);
        (0..frames).filter(|&i| out[i * 2] > 0.5).collect()
    }

    /// A new tempo keeps the song where it is: the next beats come at the new tempo
    /// from there.
    #[test]
    fn set_tempo_keeps_the_position() {
        // 120 BPM at 1 kHz: a beat is 500 frames.
        let mut e = click_engine_for(&project(), 1000);
        onsets(&mut e, 1250);
        e.set_tempo(240.0);
        assert_eq!(e.position(), 2.5 * PPQ as f64);
        assert_eq!(onsets(&mut e, 1000), [125, 375, 625, 875]);
    }

    /// Following a clock (Link): its tempo, and the song's beat lined up with the
    /// clock's within the bar.
    #[test]
    fn follow_takes_the_tempo_and_the_phase() {
        let mut e = click_engine_for(&project(), 1000);
        onsets(&mut e, 1000);
        // The song is at beat 2; the clock at beat 6.25 (1.25 bars on: 0.25 of a beat
        // further in its bar), at 100 BPM.
        e.follow(&Follow { tempo: 100.0, beat: 6.25, absolute: false });
        assert_eq!(e.tempo(), 100.0);
        assert!((e.position() / PPQ as f64 - 2.25).abs() < 1e-3, "{}", e.position());
        // A small drift is taken up too (the song a little behind).
        onsets(&mut e, 300);
        let at = e.position() / PPQ as f64;
        e.follow(&Follow { tempo: 100.0, beat: at + 4.0 + 0.01, absolute: false });
        assert!((e.position() / PPQ as f64 - (at + 0.01)).abs() < 1e-3);
        // At 100 BPM a beat is 600 frames: moved to beat 2.75, beat 3 comes 150 frames on.
        let mut e = click_engine_for(&project(), 1000);
        onsets(&mut e, 1000);
        e.follow(&Follow { tempo: 100.0, beat: 6.75, absolute: false });
        assert_eq!(onsets(&mut e, 700), [150]);
    }

    /// Starting in time with a clock (Link): the song waits, silent, for the clock's
    /// next bar, then plays from where it starts.
    #[test]
    fn a_synced_start_waits_for_the_next_bar() {
        let mut e = click_engine_for(&project(), 1000);
        e.sync_start();
        // The clock is 2.5 beats into its bar: the song starts 1.5 beats (750 frames)
        // later.
        e.follow(&Follow { tempo: 120.0, beat: 10.5, absolute: false });
        assert!(!e.finished());
        assert_eq!(onsets(&mut e, 1500), [750, 1250]);
        // Started on beat 1 of a bar.
        let mut e = click_engine_for(&project(), 1000);
        e.sync_start();
        e.follow(&Follow { tempo: 120.0, beat: 8.0, absolute: false });
        assert_eq!(onsets(&mut e, 600), [0, 500]);
        // From the second beat of the song: it waits for the clock's second beat.
        let mut e = click_engine_for(&project(), 1000);
        e.seek(PPQ);
        e.sync_start();
        e.follow(&Follow { tempo: 120.0, beat: 0.5, absolute: false });
        assert_eq!(onsets(&mut e, 600), [250]);
    }

    /// A start the clock made a moment ago (Link's peers started; DECKARD heard a frame
    /// later): within its leeway the song starts at once, as far in as it would be.
    #[test]
    fn a_late_synced_start_catches_up() {
        let mut e = click_engine_for(&project(), 1000);
        e.sync_start_within(0.5);
        e.follow(&Follow { tempo: 120.0, beat: 8.1, absolute: false });
        assert!((e.position() / PPQ as f64 - 0.1).abs() < 1e-3, "{}", e.position());
        assert_eq!(onsets(&mut e, 600), [450]);
        // Later than that: the next bar.
        let mut e = click_engine_for(&project(), 1000);
        e.sync_start_within(0.5);
        e.follow(&Follow { tempo: 120.0, beat: 8.6, absolute: false });
        assert_eq!(e.position(), 0.0);
        assert_eq!(onsets(&mut e, 1800), [1700]);
    }

    /// Following MIDI clock: the clock's beat is the song's position.
    #[test]
    fn an_absolute_clock_sets_the_song_position() {
        let mut e = click_engine_for(&project(), 1000);
        e.sync_start();
        e.follow(&Follow { tempo: 120.0, beat: 1.9, absolute: true });
        assert!((e.position() / PPQ as f64 - 1.9).abs() < 1e-3);
        assert_eq!(onsets(&mut e, 600), [50, 550]);
        // Drifting: pulled back to the clock.
        e.follow(&Follow { tempo: 120.0, beat: 3.0, absolute: true });
        assert!((e.position() / PPQ as f64 - 3.0).abs() < 1e-3);
    }

    fn clock_engine() -> (Engine, std::sync::mpsc::Receiver<crate::midi::MidiOut>) {
        let mut e = click_engine_for(&project(), 1200);
        let (tx, rx) = std::sync::mpsc::sync_channel(1024);
        e.midi_tx = Some(tx);
        e.set_clock_out(Some(ClockOut { ports: vec!["Drums".into()], early: Duration::ZERO }));
        (e, rx)
    }

    /// Sending MIDI clock: the song position and Start when the song starts from the
    /// top, then a clock every 24th of a beat, Stop when it stops; each stamped with
    /// when it's heard.
    #[test]
    fn clock_out_from_the_top() {
        let (mut e, rx) = clock_engine();
        let heard = Instant::now();
        e.heard_at(heard);
        // 120 BPM at 1.2 kHz: a beat is 600 frames, a clock 25.
        onsets(&mut e, 600);
        let got: Vec<_> = rx.try_iter().collect();
        assert_eq!(got[0].bytes(), [0xf2, 0, 0], "the song position: the top");
        assert_eq!(got[1].bytes(), [0xfa], "Start");
        let clocks: Vec<_> = got[2..].iter().collect();
        assert_eq!(clocks.len(), 24);
        assert!(clocks.iter().all(|m| m.bytes() == [0xf8] && &*m.port == "Drums"));
        let due = |m: &crate::midi::MidiOut| m.at.unwrap().duration_since(heard).as_secs_f64();
        assert!((due(clocks[3]) - 3.0 * 25.0 / 1200.0).abs() < 1e-6);
        e.stop();
        assert_eq!(rx.try_iter().map(|m| m.bytes().to_vec()).collect::<Vec<_>>(), [vec![0xfc]]);
    }

    /// From the middle of the song: its position (in 16ths) and Continue; the clock
    /// offset sends it all that much earlier.
    #[test]
    fn clock_out_from_the_middle_and_early() {
        let (mut e, rx) = clock_engine();
        e.set_clock_out(Some(ClockOut { ports: vec!["Drums".into()], early: Duration::from_millis(10) }));
        e.seek(2 * PPQ + 240);
        let heard = Instant::now() + Duration::from_secs(1);
        e.heard_at(heard);
        onsets(&mut e, 100);
        let got: Vec<_> = rx.try_iter().collect();
        assert_eq!(got[0].bytes(), [0xf2, 9, 0], "2¼ beats: the 9th 16th");
        assert_eq!(got[1].bytes(), [0xfb], "Continue");
        assert_eq!(got[1].at, Some(heard - Duration::from_millis(10)));
        assert_eq!(got.len(), 2 + 4);
    }

    /// Following incoming MIDI clock: tempo from the clocks' spacing, the beat from
    /// how many came since Start (or the song position), stops with Stop.
    #[test]
    fn clock_follower() {
        let f = ClockFollower::default();
        let t0 = Instant::now();
        let tick = Duration::from_secs_f64(60.0 / 120.0 / 24.0);
        // Clocks before Start: the tempo, but nothing plays.
        for k in 0..24 {
            f.message(SyncMsg::Clock, t0 + tick * k);
        }
        assert!((f.tempo().unwrap() - 120.0).abs() < 0.01);
        assert_eq!(f.at(t0 + tick * 24), None);
        let t1 = t0 + tick * 24;
        f.message(SyncMsg::Start, t1);
        assert_eq!(f.take_transport(), Some(Transport::Start(0.0)));
        // Started: the first clock after Start is the song's first beat.
        for k in 0..48 {
            f.message(SyncMsg::Clock, t1 + tick * (k + 1));
        }
        let last = t1 + tick * 48;
        let now = f.at(last + tick / 2).unwrap();
        assert!(now.absolute);
        assert!((now.beat - (47.5 / 24.0)).abs() < 1e-4, "{}", now.beat);
        assert!((now.tempo - 120.0).abs() < 0.01);
        // Not further than a clock and a half past the last one.
        assert!((f.at(last + tick * 10).unwrap().beat - (48.5 / 24.0)).abs() < 1e-4);
        f.message(SyncMsg::Stop, last + tick);
        assert_eq!(f.take_transport(), Some(Transport::Stop));
        assert_eq!(f.at(last + tick), None);
        // Continue from where the song position says: the 16th 16th is beat 4.
        f.message(SyncMsg::SongPosition(16), last + tick * 2);
        f.message(SyncMsg::Continue, last + tick * 3);
        assert_eq!(f.take_transport(), Some(Transport::Continue(4.0)));
        f.message(SyncMsg::Clock, last + tick * 4);
        assert!((f.at(last + tick * 4).unwrap().beat - 4.0).abs() < 1e-6);
        // The receive offset: the song plays that much later than the clock.
        f.set_offset(tick.as_secs_f64() / 2.0);
        assert!((f.at(last + tick * 4).unwrap().beat - (4.0 - 0.5 / 24.0)).abs() < 1e-4);
        // A new tempo (100 BPM) is taken after a few clocks.
        let slow = Duration::from_secs_f64(60.0 / 100.0 / 24.0);
        for k in 1..=48 {
            f.message(SyncMsg::Clock, last + tick * 4 + slow * k);
        }
        assert!((f.tempo().unwrap() - 100.0).abs() < 0.2, "{:?}", f.tempo());
    }

    #[test]
    fn sync_messages_parse() {
        assert_eq!(SyncMsg::parse(&[0xf8]), Some(SyncMsg::Clock));
        assert_eq!(SyncMsg::parse(&[0xfa]), Some(SyncMsg::Start));
        assert_eq!(SyncMsg::parse(&[0xfb]), Some(SyncMsg::Continue));
        assert_eq!(SyncMsg::parse(&[0xfc]), Some(SyncMsg::Stop));
        assert_eq!(SyncMsg::parse(&[0xf2, 0x05, 0x01]), Some(SyncMsg::SongPosition(133)));
        assert_eq!(SyncMsg::parse(&[0x90, 60, 100]), None);
    }
}
