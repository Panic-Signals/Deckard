//! Tune and Check: drive the voice with known output values and compare what it sounds.
//!
//! A `Run` holds no clock. The caller feeds it elapsed time and the detector's latest reading
//! on every audio block, and plays whatever output value it returns.

use super::pitch::Pitch;
use super::table::Table;

/// What the detector heard this block: `None` for no clear pitch (or no new analysis yet).
pub type Reading = Option<Pitch>;

/// The least time the voice gets to settle on a new output value before anything is read.
const SETTLE_MS: f32 = 100.0;
/// After settling, how long a step waits for two readings that agree.
const READ_MS: f32 = 100.0;
/// Two readings in a row agree within this many cents, or neither counts.
const AGREE_CENTS: f32 = 2.0;
/// The longest delay, output to input, a run allows for.
pub const MAX_DELAY_MS: f32 = 500.0;
/// Settling allows this much over the delay and the analysis window.
const MARGIN_MS: f32 = 50.0;
/// A pitch this many cents from the last step's has been moved by the output.
const MOVED_CENTS: f32 = 50.0;
/// The most one block counts for, so a long one still sits through settling.
const MAX_DT_MS: f32 = 50.0;
/// Why a run fails when the voice does not follow the output in time.
pub const TOO_SLOW: &str = "Delay over 500 ms: try a smaller buffer size";
/// Sweep steps from the centre to either end (each is `1 / STEPS` of the output range).
const STEPS: u32 = 25;
/// The least clarity a reading may have.
const MIN_CLARITY: f32 = 0.85;
/// Smallest rise, in log2 hz (one cent), that counts as the pitch still going up.
const MIN_RISE: f32 = 1.0 / 1200.0;
/// Fewest good points that make a table.
const MIN_POINTS: usize = 5;
/// A fresh table with more than this share of its verify notes off or unheard is thrown
/// away (4 of 16; at least 1 of a short table's fewer).
const MAX_OFF_SHARE: usize = 4;
/// Where the straight line between table points is trusted no further, in cents of estimated error.
const MAX_BEND_CENTS: f32 = 4.0;
/// A slope under this fraction of the side's median has collapsed: the voice is saturating.
const COLLAPSE: f32 = 0.6;
/// How many points before the slope gives out a knee may begin.
const KNEE_REACH: usize = 3;
/// Notes replayed to verify or check a table.
const VERIFY_NOTES: usize = 16;
/// A verified note is in tune within this many cents.
const TARGET_CENTS: f32 = 5.0;

/// The frequency of MIDI `note` in Hz.
pub(super) fn note_hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

/// The MIDI note (fractional) of `l`, a pitch in log2 hz.
fn note_of(l: f32) -> f32 {
    69.0 + 12.0 * (l - 440f32.log2())
}

/// How far `hz` is above `reference_hz`, in cents.
pub(super) fn cents(hz: f32, reference_hz: f32) -> f32 {
    1200.0 * (hz / reference_hz).log2()
}

/// A reading the run may use: finite, positive and clear.
pub(super) fn usable(reading: Reading) -> Option<Pitch> {
    reading.filter(|p| p.hz.is_finite() && p.hz > 0.0 && p.clarity >= MIN_CLARITY)
}

/// What a run does next.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Hold the voice at this output value.
    Output(f32),
    Done(Outcome),
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// A verified table, with the verified notes that were over 5 cents or unheard.
    /// `tuned_at` is left at 0 for the caller to stamp.
    Tuned { table: Table, off: Vec<u8> },
    /// The worst error over the notes replayed, and those more than 5 cents out.
    Checked { worst_cents: f32, off: Vec<u8> },
    /// Nothing usable was measured. No table comes with it, so the previous one stays.
    Failed(&'static str),
}

/// One side of the sweep, going away from the centre.
struct Side {
    alive: bool,
    fails: u8,
    /// log2 hz and output value of the last good point.
    last: Option<(f32, f32)>,
}

impl Side {
    fn new() -> Side {
        Side { alive: true, fails: 0, last: None }
    }

    fn fail(&mut self) {
        self.fails += 1;
        if self.fails >= 2 {
            self.alive = false;
        }
    }
}

struct Sweep {
    /// Step being measured: 0 is the centre, odd steps go up, even steps go down.
    k: u32,
    up: Side,
    down: Side,
    points: Vec<(f32, f32)>,
}

impl Sweep {
    fn level(k: u32) -> u32 {
        k.div_ceil(2)
    }

    fn value(k: u32) -> f32 {
        let v = Self::level(k) as f32 / STEPS as f32;
        if k % 2 == 1 { v } else { -v }
    }

    fn record(&mut self, measured: Option<Pitch>) {
        let v = Self::value(self.k);
        let l = measured.map(|p| p.hz.log2());
        if self.k == 0 {
            // Silence at the centre says nothing about either side.
            if let Some(l) = l {
                self.points.push((v, l));
                self.up.last = Some((l, v));
                self.down.last = Some((l, v));
            }
            return;
        }
        let rising = self.k % 2 == 1;
        let side = if rising { &mut self.up } else { &mut self.down };
        let Some(l) = l else {
            side.fail();
            return;
        };
        let ok = match side.last {
            None => true,
            Some((prev, _)) if rising => l > prev + MIN_RISE,
            Some((prev, _)) => l < prev - MIN_RISE,
        };
        if ok {
            self.points.push((v, l));
            side.fails = 0;
            side.last = Some((l, v));
        } else {
            // The pitch stopped following the output: the last point may already sit on the
            // flat part, so leave it out. The centre always stays.
            side.alive = false;
            if let Some((_, last_v)) = side.last.filter(|&(_, v)| v != 0.0) {
                self.points.retain(|p| p.0 != last_v);
            }
        }
    }

    /// Moves to the next step on a side still going, and returns its output value.
    fn advance(&mut self) -> Option<f32> {
        let mut k = self.k + 1;
        while Self::level(k) <= STEPS {
            let side = if k % 2 == 1 { &self.up } else { &self.down };
            if side.alive {
                self.k = k;
                return Some(Self::value(k));
            }
            k += 1;
        }
        None
    }

    /// Drops the points from where the response has gone flat (a saturating top or bottom) to
    /// that end, but only where a straight line between points is also untrue. Curvature
    /// alone, with the slope healthy, never cuts.
    fn cut_at_bends(&mut self) {
        let slope = |a: (f32, f32), b: (f32, f32)| (b.1 - a.1) / (b.0 - a.0);
        let bend = |p: &[(f32, f32)]| 1200.0 * (slope(p[1], p[2]) - slope(p[0], p[1])).abs() * (p[2].0 - p[0].0) / 16.0;
        // The typical slope of a side: the median over its inner half, which saturation at
        // the far end cannot have dragged down.
        let median = |outward: Vec<(f32, f32)>| {
            let mut s: Vec<f32> = outward.windows(2).map(|w| slope(w[0], w[1]).abs()).collect();
            s.truncate(s.len().div_ceil(2));
            s.sort_by(|a, b| a.total_cmp(b));
            s.get(s.len() / 2).copied()
        };
        let centre = self.points.partition_point(|p| p.0 < 0.0);
        let below_outward: Vec<_> = self.points[..(centre + 1).min(self.points.len())].iter().rev().copied().collect();
        let (below_med, above_med) = (median(below_outward), median(self.points[centre..].to_vec()));
        let p = &self.points;
        let collapsed =
            |med: Option<f32>, beyond: (f32, f32), other: (f32, f32)| med.is_some_and(|m| slope(beyond, other).abs() < COLLAPSE * m);
        let sharp = |i: usize| i >= 1 && i + 1 < p.len() && bend(&p[i - 1..=i + 1]) > MAX_BEND_CENTS;
        let mut cut_above = p.len();
        let mut cut_below = 0;
        for i in 1..p.len().saturating_sub(1) {
            if !sharp(i) {
                continue;
            }
            // The knee starts at the first sharp point just before the slope gives out.
            if p[i].0 > 0.0 {
                if collapsed(above_med, p[i], p[i + 1]) {
                    cut_above = cut_above.min((i.saturating_sub(KNEE_REACH)..=i).find(|&j| sharp(j) && p[j].0 > 0.0).unwrap_or(i));
                }
            } else if p[i].0 < 0.0 && collapsed(below_med, p[i - 1], p[i]) {
                cut_below = cut_below.max((i..=i + KNEE_REACH).rev().find(|&j| sharp(j) && p[j].0 < 0.0).unwrap_or(i) + 1);
            }
        }
        self.points.truncate(cut_above);
        self.points.drain(..cut_below.min(self.points.len()));
    }

    fn into_table(mut self) -> Option<Table> {
        self.points.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.cut_at_bends();
        (self.points.len() >= MIN_POINTS).then_some(Table { points: self.points, tuned_at: 0, worst_cents: 0.0 })
    }
}

/// Replays notes through a table and measures how far each lands from its pitch.
struct Verify {
    table: Table,
    notes: Vec<u8>,
    errors: Vec<Option<f32>>,
    /// True when this verifies a fresh tune, false for a check of a saved table.
    tuning: bool,
}

/// The whole note a fraction `t` of the way (in pitch) from `table`'s point `gap` to the
/// next, kept within the table's range.
fn between(table: &Table, gap: usize, t: f32) -> u8 {
    let (lo, hi) = table.range();
    let (a, b) = (table.points[gap].1, table.points[gap + 1].1);
    note_of(a + t * (b - a)).round().clamp(lo as f32, hi as f32) as u8
}

/// The gaps between points that 16 notes spread over `table` fall in, shifted on by
/// `offset` (0 to 1) of the spacing between them.
fn gaps(table: &Table, offset: f32) -> impl Iterator<Item = usize> {
    let count = table.points.len().saturating_sub(1);
    (0..VERIFY_NOTES).map(move |k| (((k as f32 + offset) * count as f32 / VERIFY_NOTES as f32) as usize).min(count.saturating_sub(1)))
}

/// The notes a fresh table is verified on: the midpoints of 16 gaps between its points.
fn verify_notes(table: &Table) -> Vec<u8> {
    let mut notes: Vec<u8> = gaps(table, 0.25).map(|gap| between(table, gap, 0.5)).collect();
    notes.sort_unstable();
    notes.dedup();
    notes
}

/// The notes a saved table is checked on: between its points too, in the gaps between
/// those verified (or, where none is free, elsewhere in a gap), so a Check looks where the
/// Tune didn't.
fn check_notes(table: &Table) -> Vec<u8> {
    let verified = verify_notes(table);
    let mut notes: Vec<u8> = Vec::new();
    for gap in gaps(table, 0.75) {
        let free = [0.5, 0.25, 0.75].map(|t| between(table, gap, t)).into_iter().find(|n| !verified.contains(n) && !notes.contains(n));
        notes.extend(free);
    }
    notes.sort_unstable();
    notes.dedup();
    notes
}

impl Verify {
    fn new(table: Table, tuning: bool) -> Verify {
        let (lo, hi) = table.range();
        let notes = match table.points.len() >= 2 && lo <= hi {
            false => Vec::new(),
            true if tuning => verify_notes(&table),
            true => check_notes(&table),
        };
        Verify { table, notes, errors: Vec::new(), tuning }
    }

    fn output(&self) -> f32 {
        self.table.value_for(self.notes[self.errors.len()] as f32).0
    }

    fn record(&mut self, measured: Option<Pitch>) {
        let target = note_hz(self.notes[self.errors.len()] as f32);
        self.errors.push(measured.map(|p| cents(p.hz, target)));
    }

    fn finish(self) -> Outcome {
        let worst = self.errors.iter().flatten().fold(None, |w: Option<f32>, e| Some(w.map_or(e.abs(), |w| w.max(e.abs()))));
        let off: Vec<u8> =
            self.notes.iter().zip(&self.errors).filter(|(_, e)| e.is_none_or(|e| e.abs() > TARGET_CENTS)).map(|(&n, _)| n).collect();
        let Some(worst_cents) = worst else {
            return Outcome::Failed("no pitch heard while verifying");
        };
        if !self.tuning {
            return Outcome::Checked { worst_cents, off };
        }
        if off.len() * MAX_OFF_SHARE > self.notes.len() {
            return Outcome::Failed("tuned table did not hold up");
        }
        Outcome::Tuned { table: Table { worst_cents, ..self.table }, off }
    }
}

enum Phase {
    Sweep(Sweep),
    Verify(Verify),
    Finished,
}

/// A Tune or a Check in progress.
///
/// What the voice sounds lags what the run plays by the loop's delay: the output's latency,
/// the interface's, the input's and the analysis. The run measures it on its first move it
/// can hear (the time until the pitch heard leaves the last step's), and every step from
/// then settles for that, the analysis window and a margin. Until then a step allows for
/// twice the longest delay, so what it reads is its own even when the voice answers a
/// little too late, and the next step catches that rather than measuring a short delay
/// against a stale pitch.
pub struct Run {
    phase: Phase,
    output: f32,
    /// Time since the output moved to this step's value.
    elapsed: f32,
    /// The analysis window, in ms: a reading speaks for this much audio before it.
    window_ms: f32,
    /// The loop's delay, once measured.
    delay_ms: Option<f32>,
    /// What the last step settled on: the pitch heard until this step's arrives.
    before: Option<Pitch>,
    /// A reading that left `before`, while the delay is being measured.
    moved: Option<Pitch>,
    /// When `before`'s pitch was last heard, while the delay is being measured.
    still_at: Option<f32>,
    /// The last usable reading since settling, for the next to agree with.
    last: Option<Pitch>,
}

impl Run {
    fn new(phase: Phase, output: f32, window_ms: f32) -> Run {
        let window_ms = if window_ms.is_finite() { window_ms.clamp(0.0, 1000.0) } else { 0.0 };
        Run { phase, output, elapsed: 0.0, window_ms, delay_ms: None, before: None, moved: None, still_at: None, last: None }
    }

    /// Sweeps the whole output range, builds a table from it and verifies it. Each reading
    /// covers `window_ms` of audio.
    pub fn tune(window_ms: f32) -> Run {
        Run::new(Phase::Sweep(Sweep { k: 0, up: Side::new(), down: Side::new(), points: Vec::new() }), 0.0, window_ms)
    }

    /// Replays notes through `table` and reports how far off they sound.
    pub fn check(table: Table, window_ms: f32) -> Run {
        let verify = Verify::new(table, false);
        let output = if verify.notes.is_empty() { 0.0 } else { verify.output() };
        Run::new(Phase::Verify(verify), output, window_ms)
    }

    /// How long this step waits before it reads.
    fn settle_ms(&self) -> f32 {
        match self.delay_ms {
            Some(delay) => (delay + self.window_ms + MARGIN_MS).max(SETTLE_MS),
            None => 2.0 * MAX_DELAY_MS + self.window_ms + MARGIN_MS,
        }
    }

    /// Advances by `dt_ms` with the detector's newest reading (`None` on most blocks, as
    /// detections come less often than blocks). A step's reading is the second of two in a
    /// row, after settling, that agree within 2 cents.
    pub fn step(&mut self, dt_ms: f32, reading: Reading) -> Step {
        match &self.phase {
            Phase::Finished => return Step::Done(Outcome::Failed("run is over")),
            Phase::Verify(v) if v.notes.is_empty() => {
                self.phase = Phase::Finished;
                return Step::Done(Outcome::Failed("table has no range"));
            }
            _ => {}
        }
        self.elapsed += if dt_ms.is_nan() { 0.0 } else { dt_ms.clamp(0.0, MAX_DT_MS) };
        let heard = usable(reading);
        // Measuring the delay: this step's pitch has arrived when two readings in a row
        // have left the last step's.
        if self.delay_ms.is_none()
            && let Some(before) = self.before
        {
            match heard {
                Some(p) if cents(p.hz, before.hz).abs() > MOVED_CENTS => {
                    if self.moved.is_some() {
                        self.delay_ms = Some(self.elapsed);
                    }
                    self.moved = Some(p);
                }
                Some(_) => (self.moved, self.still_at) = (None, Some(self.elapsed)),
                None => {}
            }
            if self.delay_ms.is_none() && self.elapsed >= MAX_DELAY_MS + self.window_ms {
                // Still hearing the last step: the voice is too late. Hearing nothing: this
                // step is silent, and the delay waits for one that isn't.
                if self.still_at.is_some_and(|t| self.elapsed - t <= self.window_ms + MAX_DT_MS) {
                    self.phase = Phase::Finished;
                    return Step::Done(Outcome::Failed(TOO_SLOW));
                }
                return self.next(None);
            }
            if self.delay_ms.is_none() {
                return Step::Output(self.output);
            }
        }
        let settle = self.settle_ms();
        if self.elapsed < settle {
            return Step::Output(self.output);
        }
        if let Some(p) = heard {
            if self.last.is_some_and(|q| cents(p.hz, q.hz).abs() <= AGREE_CENTS) {
                return self.next(Some(p));
            }
            self.last = Some(p);
        }
        if self.elapsed >= settle + READ_MS {
            return self.next(None);
        }
        Step::Output(self.output)
    }

    /// Ends this step on `measured`, and moves to the next.
    fn next(&mut self, measured: Option<Pitch>) -> Step {
        (self.elapsed, self.before, self.moved, self.still_at, self.last) = (0.0, measured, None, None, None);
        match std::mem::replace(&mut self.phase, Phase::Finished) {
            Phase::Sweep(mut sweep) => {
                sweep.record(measured);
                match sweep.advance() {
                    Some(v) => {
                        self.output = v;
                        self.phase = Phase::Sweep(sweep);
                        Step::Output(v)
                    }
                    None => match sweep.into_table() {
                        Some(table) => self.verify(Verify::new(table, true)),
                        None => Step::Done(Outcome::Failed("could not hear the voice across its range")),
                    },
                }
            }
            Phase::Verify(mut verify) => {
                verify.record(measured);
                if verify.errors.len() < verify.notes.len() {
                    self.output = verify.output();
                    self.phase = Phase::Verify(verify);
                    Step::Output(self.output)
                } else {
                    Step::Done(verify.finish())
                }
            }
            Phase::Finished => Step::Done(Outcome::Failed("run is over")),
        }
    }

    fn verify(&mut self, verify: Verify) -> Step {
        if verify.notes.is_empty() {
            return Step::Done(Outcome::Failed("table has no range"));
        }
        self.output = verify.output();
        self.phase = Phase::Verify(verify);
        Step::Output(self.output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The analysis window the tests' runs allow for (4096 samples at 48 kHz).
    const W: f32 = 85.0;

    /// A simulated oscillator: output value in, `Pitch` out.
    struct Vco {
        base_hz: f32,
        per_unit_octaves: f32,
        /// Amplitude of a ripple in the response, in octaves.
        bow: f32,
        /// Octaves of drift per second.
        drift: f32,
        /// The voice cannot sound above this.
        ceiling_hz: f32,
        /// From this many ms on, outputs at or above `late_above_v` sound `late_octaves` higher.
        after_ms: f32,
        late_octaves: f32,
        late_above_v: f32,
        /// From this many ms on, nothing can be heard.
        lose_after_ms: f32,
        /// Above 0.3 the response flattens smoothly instead of staying straight.
        soft: bool,
        /// Above 0.3 the response turns round and falls.
        fold: bool,
        /// The output is silent (not heard) below this.
        silent_below: f32,
        /// Readings swing this many cents either side of the pitch, one way then the other.
        jitter_cents: f32,
        /// Before `bad_until_ms`, output `bad_v` is misread `bad_octaves` high.
        bad_v: f32,
        bad_octaves: f32,
        bad_until_ms: f32,
    }

    impl Vco {
        fn new() -> Vco {
            Vco {
                base_hz: note_hz(60.0),
                per_unit_octaves: 10.0,
                bow: 0.1,
                drift: 0.0,
                ceiling_hz: 4000.0,
                after_ms: f32::INFINITY,
                late_octaves: 0.0,
                late_above_v: -2.0,
                lose_after_ms: f32::INFINITY,
                soft: false,
                fold: false,
                silent_below: -2.0,
                jitter_cents: 0.0,
                bad_v: f32::NAN,
                bad_octaves: 0.0,
                bad_until_ms: 0.0,
            }
        }

        fn hz(&self, v: f32, t_s: f32) -> f32 {
            let mut octaves = self.per_unit_octaves * v + self.bow * (6.0 * v).sin() + self.drift * t_s;
            if v > 0.3 && self.soft {
                octaves = self.per_unit_octaves * 0.3 + 1.5 * (1.0 - (-(v - 0.3) * 6.67).exp());
            }
            if v > 0.3 && self.fold {
                octaves = self.per_unit_octaves * 0.3 - 6.0 * (v - 0.3);
            }
            if t_s * 1000.0 >= self.after_ms && v >= self.late_above_v {
                octaves += self.late_octaves;
            }
            (self.base_hz * octaves.exp2()).min(self.ceiling_hz)
        }

        /// What the detector would say at `t_s` of output `v`: nothing below 20 Hz.
        fn read(&self, v: f32, t_s: f32, nth: u32) -> Reading {
            let mut hz = self.hz(v, t_s);
            let sign = if nth % 2 == 0 { 1.0 } else { -1.0 };
            hz *= (sign * self.jitter_cents / 1200.0).exp2();
            if (v - self.bad_v).abs() < 1e-4 && t_s * 1000.0 < self.bad_until_ms {
                hz *= self.bad_octaves.exp2();
            }
            (hz >= 20.0 && t_s * 1000.0 < self.lose_after_ms && v >= self.silent_below).then_some(Pitch { hz, clarity: 0.95 })
        }
    }

    /// How a run went: its outcome, the simulated ms it took, and when it began to verify.
    struct Drove {
        outcome: Outcome,
        ms: f32,
        verify_at: Option<f32>,
    }

    /// Feeds a run from a `Vco` in 5 ms blocks, with a detection every `hop_ms`. The voice
    /// hears the output `delay_ms` late, and each detection is of the oldest audio in its
    /// window (`W` ms), so a step is heard only once its audio fills a window.
    fn drive_with(mut run: Run, vco: &Vco, hop_ms: f32, delay_ms: f32) -> Drove {
        let dt = 5.0;
        // When the output changed, and to what (the voice sounds the centre before the run).
        let mut history = vec![(f32::NEG_INFINITY, 0.0f32)];
        let (mut t, mut next_read, mut nth, mut verify_at) = (0.0f32, hop_ms, 0, None);
        loop {
            t += dt;
            let reading = if t >= next_read {
                next_read += hop_ms;
                nth += 1;
                let heard_at = t - delay_ms - W;
                let v = history.iter().rev().find(|c| c.0 <= heard_at).map_or(0.0, |c| c.1);
                vco.read(v, t / 1000.0, nth)
            } else {
                None
            };
            match run.step(dt, reading) {
                Step::Output(v) => {
                    if v != history.last().unwrap().1 {
                        history.push((t, v));
                    }
                }
                Step::Done(outcome) => return Drove { outcome, ms: t, verify_at },
            }
            if verify_at.is_none() && matches!(run.phase, Phase::Verify(_)) {
                verify_at = Some(t);
            }
            assert!(t < 300_000.0, "run never finished");
        }
    }

    fn drive(run: Run, vco: &Vco, hop_ms: f32, delay_ms: f32) -> (Outcome, f32) {
        let d = drive_with(run, vco, hop_ms, delay_ms);
        (d.outcome, d.ms)
    }

    fn tuned(vco: &Vco) -> Table {
        match drive(Run::tune(W), vco, 25.0, 0.0).0 {
            Outcome::Tuned { table, .. } => table,
            other => panic!("expected a table, got {other:?}"),
        }
    }

    fn error_cents(vco: &Vco, table: &Table, note: u8) -> f32 {
        cents(vco.hz(table.value_for(note as f32).0, 0.0), note_hz(note as f32))
    }

    #[test]
    fn tune_brings_every_note_within_5_cents_on_a_bowed_vco() {
        let vco = Vco::new();
        let table = tuned(&vco);
        // Points are as far apart as the sweep makes them.
        assert!(table.points.windows(2).all(|w| w[1].0 - w[0].0 > 0.039));
        let (lo, hi) = table.range();
        assert!(hi - lo > 50, "range {lo}..{hi}");
        for note in lo..=hi {
            let e = error_cents(&vco, &table, note);
            assert!(e.abs() < 5.0, "note {note}: {e} cents");
        }
        assert!(table.worst_cents < 5.0);
    }

    /// A short table has fewer verify notes; with all of them off it still fails.
    #[test]
    fn a_short_table_with_every_verify_note_off_fails() {
        // Heard only from the centre up to about 1.8 octaves: a handful of points.
        let short = Vco { silent_below: -0.001, ceiling_hz: note_hz(60.0) * 1.8f32.exp2(), ..Vco::new() };
        let dry = drive_with(Run::tune(W), &short, 25.0, 0.0);
        let Outcome::Tuned { table, .. } = &dry.outcome else { panic!("expected a table, got {:?}", dry.outcome) };
        assert!(table.points.len() < 17, "{} points", table.points.len());
        // The voice jumps 120 cents just as verifying starts.
        let wrong = Vco { after_ms: dry.verify_at.unwrap(), late_octaves: 0.1, ..short };
        match drive(Run::tune(W), &wrong, 25.0, 0.0).0 {
            Outcome::Failed(_) => {}
            other => panic!("a table with every verify note off was kept: {other:?}"),
        }
    }

    #[test]
    fn tune_finds_the_vcos_range_and_clamps_past_it() {
        let vco = Vco { ceiling_hz: 1000.0, ..Vco::new() };
        let table = tuned(&vco);
        let (lo, hi) = table.range();
        // 20 Hz is MIDI 15.4 and 1000 Hz is MIDI 83.2.
        assert!((15..=30).contains(&lo), "lo {lo}");
        assert!((70..=83).contains(&hi), "hi {hi}");
        for note in lo..=hi {
            assert!(error_cents(&vco, &table, note).abs() < 5.0, "note {note}");
        }
        assert!(table.value_for(hi as f32 + 24.0).1);
        assert!(table.value_for(lo as f32 - 12.0).1);
        assert!(!table.value_for(hi as f32).1);
    }

    #[test]
    fn tune_fails_on_no_signal_quickly_and_makes_no_table() {
        let mut run = Run::tune(W);
        let mut t = 0.0;
        let outcome = loop {
            t += 5.0;
            if let Step::Done(o) = run.step(5.0, None) {
                break o;
            }
            // Five steps, each allowing for twice the longest delay.
            assert!(t < 7000.0, "still running at {t} ms");
        };
        assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
        assert!(matches!(run.step(5.0, None), Step::Done(Outcome::Failed(_))));
    }

    #[test]
    fn tune_fails_on_noise() {
        let mut run = Run::tune(W);
        let mut seed = 99u32;
        let mut rand = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let mut t = 0.0;
        loop {
            t += 5.0;
            // A pitch of any value, every 25 ms.
            let reading = (t % 25.0 == 0.0).then(|| Pitch { hz: 60.0 + 3000.0 * rand(), clarity: 0.9 });
            if let Step::Done(o) = run.step(5.0, reading) {
                assert!(matches!(o, Outcome::Failed(_)), "{o:?}");
                break;
            }
            assert!(t < 60_000.0);
        }
    }

    #[test]
    fn tune_still_works_when_most_steps_have_no_reading() {
        let vco = Vco::new();
        // A detection every 40 ms against blocks of 5 ms.
        let (outcome, _) = drive(Run::tune(W), &vco, 40.0, 0.0);
        assert!(matches!(outcome, Outcome::Tuned { .. }), "{outcome:?}");
    }

    /// Either a failure, or a table all of whose notes are in tune but those named in `off`.
    fn assert_honest_after(vco: &Vco, delay_ms: f32) -> Outcome {
        let outcome = drive(Run::tune(W), vco, 25.0, delay_ms).0;
        match &outcome {
            Outcome::Failed(_) => {}
            Outcome::Tuned { table, off } => {
                let (lo, hi) = table.range();
                for note in lo..=hi {
                    let e = error_cents(vco, table, note);
                    assert!(e.abs() <= 5.0 || off.contains(&note), "delay {delay_ms}: note {note}: {e} cents, off {off:?}");
                }
            }
            other => panic!("{other:?}"),
        }
        outcome
    }

    fn assert_honest(vco: &Vco) {
        assert_honest_after(vco, 0.0);
    }

    /// The voice answers late (the interface's buffers, the input queue): every step waits
    /// for the delay it measured, so the table is true however late, up to half a second.
    #[test]
    fn tune_allows_for_the_loops_delay_up_to_half_a_second() {
        let vco = Vco::new();
        for delay in [0.0, 50.0, 100.0, 200.0, 400.0] {
            let outcome = assert_honest_after(&vco, delay);
            let Outcome::Tuned { table, .. } = outcome else { panic!("delay {delay}: {outcome:?}") };
            let (lo, hi) = table.range();
            assert!(hi - lo > 50, "delay {delay}: range {lo}..{hi}");
        }
        assert_eq!(drive(Run::tune(W), &vco, 25.0, 600.0).0, Outcome::Failed(TOO_SLOW));
    }

    #[test]
    fn check_allows_for_the_loops_delay_too() {
        let vco = Vco::new();
        let table = tuned(&vco);
        let (outcome, _) = drive(Run::check(table.clone(), W), &vco, 25.0, 300.0);
        let Outcome::Checked { worst_cents, off } = outcome else { panic!("{outcome:?}") };
        assert!(worst_cents < 5.0 && off.is_empty(), "{worst_cents} {off:?}");
        assert_eq!(drive(Run::check(table, W), &vco, 25.0, 600.0).0, Outcome::Failed(TOO_SLOW));
    }

    /// Readings that never settle within 2 cents of each other are not readings.
    #[test]
    fn a_reading_counts_only_when_two_in_a_row_agree() {
        let steady = Vco { jitter_cents: 0.5, ..Vco::new() };
        assert!(matches!(drive(Run::tune(W), &steady, 25.0, 0.0).0, Outcome::Tuned { .. }));
        let unsteady = Vco { jitter_cents: 1.5, ..Vco::new() };
        let outcome = drive(Run::tune(W), &unsteady, 25.0, 0.0).0;
        assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
    }

    #[test]
    fn check_reports_the_worst_note_and_names_those_over_target() {
        let vco = Vco::new();
        let table = tuned(&vco);
        let (outcome, _) = drive(Run::check(table.clone(), W), &vco, 25.0, 0.0);
        let Outcome::Checked { worst_cents, off } = outcome else { panic!("{outcome:?}") };
        assert!(worst_cents < 5.0 && off.is_empty(), "{worst_cents} {off:?}");

        // The voice now runs a little wide: error grows with the output value.
        let wide = Vco { per_unit_octaves: 10.05, ..Vco::new() };
        let (outcome, _) = drive(Run::check(table.clone(), W), &wide, 25.0, 0.0);
        let Outcome::Checked { worst_cents, off } = outcome else { panic!("{outcome:?}") };
        assert!(!off.is_empty() && off.len() < VERIFY_NOTES, "{off:?}");
        for &n in &off {
            assert!(error_cents(&wide, &table, n).abs() > 5.0, "note {n}");
        }
        let worst = off.iter().map(|&n| error_cents(&wide, &table, n).abs()).fold(0.0, f32::max);
        assert!((worst - worst_cents).abs() < 0.01, "{worst} vs {worst_cents}");
        assert!(worst_cents > 20.0);
    }

    /// Verify and Check play notes between the table's points, where its straight lines
    /// are least sure, and not the same ones.
    #[test]
    fn verify_and_check_play_different_notes_between_the_points() {
        let table = tuned(&Vco::new());
        let (verify, check) = (verify_notes(&table), check_notes(&table));
        assert!(verify.len() >= 12 && check.len() >= 12, "{verify:?} {check:?}");
        assert!(check.iter().all(|n| !verify.contains(n)), "{verify:?} {check:?}");
        let (lo, hi) = table.range();
        let at = |n: u8| (n as f32 - 69.0) / 12.0 + 440f32.log2();
        for &n in verify.iter().chain(&check) {
            assert!((lo..=hi).contains(&n));
            // A cent or more from every point.
            assert!(table.points.iter().all(|p| (at(n) - p.1).abs() * 1200.0 > 1.0), "note {n} sits on a point");
        }
    }

    /// One sweep point misread between two of the notes an evenly spread verify of eight
    /// would play: the notes either side of it are out, and the Tune either fails or names
    /// them, and so does a later Check of the table it would have made.
    #[test]
    fn a_bad_point_between_the_verify_notes_is_caught_by_tune_and_check() {
        let plain = Vco::new();
        let good = tuned(&plain);
        let verify_at = drive_with(Run::tune(W), &plain, 25.0, 0.0).verify_at.unwrap();
        // The point at output -0.2 (about MIDI 35), misread 30 cents sharp while sweeping.
        // The notes either side of it are out, up to 30 cents; eight notes spread evenly
        // (16, 29, 41, ...) would play none of them.
        let bad = Vco { bad_v: -0.2, bad_octaves: 0.025, bad_until_ms: verify_at, ..Vco::new() };
        match drive(Run::tune(W), &bad, 25.0, 0.0).0 {
            Outcome::Failed(_) => {}
            Outcome::Tuned { table, off } => {
                assert!(!off.is_empty(), "the bad point went unseen");
                for &n in &off {
                    assert!(error_cents(&plain, &table, n).abs() > 5.0, "note {n} named but in tune");
                }
            }
            other => panic!("{other:?}"),
        }
        // The table that point makes, checked on the true voice.
        let mut table = good.clone();
        let i = table.points.iter().position(|p| (p.0 + 0.2).abs() < 1e-4).unwrap();
        table.points[i].1 += 0.025;
        let (outcome, _) = drive(Run::check(table.clone(), W), &plain, 25.0, 0.0);
        let Outcome::Checked { off, .. } = outcome else { panic!("{outcome:?}") };
        assert!(!off.is_empty(), "the Check missed it");
        for &n in &off {
            assert!(error_cents(&plain, &table, n).abs() > 5.0, "note {n}");
        }
    }

    #[test]
    fn tune_fails_when_the_vco_goes_wrong_during_verify() {
        let sweep_end = drive_with(Run::tune(W), &Vco::new(), 25.0, 0.0).verify_at.unwrap();
        let vco = Vco { after_ms: sweep_end + 1.0, late_octaves: 0.1, late_above_v: 0.0, ..Vco::new() };
        let (outcome, _) = drive(Run::tune(W), &vco, 25.0, 0.0);
        assert_eq!(outcome, Outcome::Failed("tuned table did not hold up"));
    }

    #[test]
    fn tune_fails_when_the_signal_is_lost_during_verify() {
        let sweep_end = drive_with(Run::tune(W), &Vco::new(), 25.0, 0.0).verify_at.unwrap();
        let vco = Vco { lose_after_ms: sweep_end + 1.0, ..Vco::new() };
        let (outcome, _) = drive(Run::tune(W), &vco, 25.0, 0.0);
        assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
    }

    #[test]
    fn tune_names_a_bad_top_note_but_keeps_the_table() {
        let table = tuned(&Vco::new());
        let top = *verify_notes(&table).last().unwrap();
        let above = table.value_for(top as f32).0 - 0.005;
        let sweep_end = drive_with(Run::tune(W), &Vco::new(), 25.0, 0.0).verify_at.unwrap();
        let vco = Vco { after_ms: sweep_end + 1.0, late_octaves: 0.03, late_above_v: above, ..Vco::new() };
        let (outcome, _) = drive(Run::tune(W), &vco, 25.0, 0.0);
        let Outcome::Tuned { table: kept, off } = outcome else { panic!("{outcome:?}") };
        assert_eq!(off, vec![top]);
        // The worst of the notes heard includes the bad one: 0.03 octaves is 36 cents.
        assert!((kept.worst_cents - 36.0).abs() < 1.0, "{}", kept.worst_cents);
    }

    #[test]
    fn tune_fails_on_noise_for_many_seeds() {
        for seed in 0..250u32 {
            let mut run = Run::tune(W);
            let mut state = seed.wrapping_mul(2654435761).wrapping_add(7);
            let mut rand = move || {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 8) as f32 / (1u32 << 24) as f32
            };
            let mut t = 0.0;
            loop {
                t += 5.0;
                let reading = (t % 25.0 == 0.0).then(|| Pitch { hz: 60.0 + 3000.0 * rand(), clarity: 0.9 });
                if let Step::Done(o) = run.step(5.0, reading) {
                    assert!(matches!(o, Outcome::Failed(_)), "seed {seed}: {o:?}");
                    break;
                }
                assert!(t < 60_000.0);
            }
        }
    }

    #[test]
    fn a_soft_saturating_vco_gives_failure_or_an_honest_table() {
        assert_honest(&Vco { soft: true, ceiling_hz: 20000.0, ..Vco::new() });
    }

    #[test]
    fn a_curved_vco_keeps_its_whole_range() {
        let vco = Vco { bow: 0.5, ..Vco::new() };
        let Outcome::Tuned { table, off } = drive(Run::tune(W), &vco, 25.0, 0.0).0 else { panic!("did not tune") };
        let (lo, hi) = table.range();
        // The 20 Hz floor is note 15 and the 4 kHz ceiling note 95.
        assert!(lo <= 20 && hi >= 90, "range {lo}..{hi}");
        assert!(off.is_empty(), "{off:?}");
        for note in lo..=hi {
            assert!(error_cents(&vco, &table, note).abs() < 5.0, "note {note}");
        }
    }

    /// Blocks of 50 ms up to the one that moves on: the first step allows for twice the
    /// longest delay, then the window and the margin, then reads twice.
    fn blocks_to_read() -> usize {
        ((2.0 * MAX_DELAY_MS + W + MARGIN_MS) / 50.0).ceil() as usize + 1
    }

    #[test]
    fn a_nan_block_counts_as_no_time() {
        let mut run = Run::tune(W);
        let heard = Some(Pitch { hz: 440.0, clarity: 0.95 });
        assert_eq!(run.step(f32::NAN, heard), Step::Output(0.0));
        for _ in 1..blocks_to_read() {
            assert_eq!(run.step(50.0, heard), Step::Output(0.0));
        }
        assert_eq!(run.step(50.0, heard), Step::Output(0.04));
    }

    #[test]
    fn a_non_monotonic_vco_gives_failure_or_an_honest_table() {
        assert_honest(&Vco { fold: true, ..Vco::new() });
    }

    #[test]
    fn a_vco_silent_at_the_centre_still_tunes_its_upper_side() {
        // Silent below output 0.05, so the centre step is not heard but +0.08 and up is.
        let vco = Vco { silent_below: 0.05, ..Vco::new() };
        let outcome = drive(Run::tune(W), &vco, 25.0, 0.0).0;
        let Outcome::Tuned { table, .. } = outcome else { panic!("{outcome:?}") };
        assert!(table.points.iter().all(|p| p.0 >= 0.05) && table.points.len() >= 5);
    }

    #[test]
    fn one_huge_block_does_not_skip_settling() {
        let mut run = Run::tune(W);
        let heard = Some(Pitch { hz: 440.0, clarity: 0.95 });
        assert_eq!(run.step(1e9, heard), Step::Output(0.0));
        // It counted 50 ms, as does this.
        assert_eq!(run.step(f32::INFINITY, heard), Step::Output(0.0));
        for _ in 3..blocks_to_read() {
            assert_eq!(run.step(50.0, heard), Step::Output(0.0));
        }
        assert_eq!(run.step(50.0, heard), Step::Output(0.04));
    }

    #[test]
    fn check_of_an_empty_table_fails() {
        let table = Table { points: vec![], tuned_at: 0, worst_cents: 0.0 };
        assert!(matches!(Run::check(table, W).step(5.0, None), Step::Done(Outcome::Failed(_))));
    }
}
