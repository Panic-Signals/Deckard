//! Tune and Check: drive the voice with known output values and compare what it sounds.
//!
//! A `Run` holds no clock. The caller feeds it elapsed time and the detector's latest reading
//! on every audio block, and plays whatever output value it returns.

use super::pitch::Pitch;
use super::table::Table;

/// What the detector heard this block: `None` for no clear pitch (or no new analysis yet).
pub type Reading = Option<Pitch>;

/// Time the voice gets to settle on a new output value before anything is read.
const SETTLE_MS: f32 = 100.0;
/// Time after that in which readings are taken.
const READ_MS: f32 = 50.0;
/// Sweep steps from the centre to either end (each is `1 / STEPS` of the output range).
const STEPS: u32 = 25;
/// The least clarity a reading may have.
const MIN_CLARITY: f32 = 0.85;
/// Smallest rise, in log2 hz (one cent), that counts as the pitch still going up.
const MIN_RISE: f32 = 1.0 / 1200.0;
/// Fewest good points that make a table.
const MIN_POINTS: usize = 5;
/// A fresh table with more notes than this off or unheard when verified is thrown away.
const MAX_OFF: usize = 2;
/// Where the straight line between table points is trusted no further, in cents of estimated error.
const MAX_BEND_CENTS: f32 = 4.0;
/// Notes replayed to verify a table.
const VERIFY_NOTES: usize = 8;
/// A verified note is in tune within this many cents.
const TARGET_CENTS: f32 = 5.0;

/// The frequency of MIDI `note` in Hz.
pub(super) fn note_hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) / 12.0).exp2()
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

    /// Drops the points from where the response bends too sharply for a straight line between
    /// points to be trusted (a soft knee at the top or bottom of the range) to that end.
    fn cut_at_bends(&mut self) {
        let slope = |a: (f32, f32), b: (f32, f32)| (b.1 - a.1) / (b.0 - a.0);
        let bend = |p: &[(f32, f32)]| 1200.0 * (slope(p[1], p[2]) - slope(p[0], p[1])).abs() * (p[2].0 - p[0].0) / 16.0;
        let sharp = (1..self.points.len().saturating_sub(1)).filter(|&i| bend(&self.points[i - 1..=i + 1]) > MAX_BEND_CENTS);
        let (above, below): (Vec<usize>, Vec<usize>) = sharp.partition(|&i| self.points[i].0 > 0.0);
        let first_above = above.first().copied().unwrap_or(self.points.len());
        let last_below = below.last().map(|&i| i + 1).unwrap_or(0);
        self.points.truncate(first_above);
        self.points.drain(..last_below.min(self.points.len()));
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

impl Verify {
    fn new(table: Table, tuning: bool) -> Verify {
        let (lo, hi) = table.range();
        let mut notes: Vec<u8> = Vec::new();
        if table.points.len() >= 2 && lo <= hi {
            for i in 0..VERIFY_NOTES {
                let note = lo + (((hi - lo) as f32) * i as f32 / (VERIFY_NOTES - 1) as f32).round() as u8;
                if notes.last() != Some(&note) {
                    notes.push(note);
                }
            }
        }
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
        if off.len() > MAX_OFF {
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
pub struct Run {
    phase: Phase,
    output: f32,
    elapsed: f32,
    /// The last usable reading of the current read phase.
    last: Option<Pitch>,
}

impl Run {
    /// Sweeps the whole output range, builds a table from it and verifies it.
    pub fn tune() -> Run {
        Run {
            phase: Phase::Sweep(Sweep { k: 0, up: Side::new(), down: Side::new(), points: Vec::new() }),
            output: 0.0,
            elapsed: 0.0,
            last: None,
        }
    }

    /// Replays notes through `table` and reports how far off they sound.
    pub fn check(table: Table) -> Run {
        let verify = Verify::new(table, false);
        let output = if verify.notes.is_empty() { 0.0 } else { verify.output() };
        Run { phase: Phase::Verify(verify), output, elapsed: 0.0, last: None }
    }

    /// Advances by `dt_ms` with the detector's newest reading (`None` on most blocks, as
    /// detections come less often than blocks). The last reading in each read phase decides.
    pub fn step(&mut self, dt_ms: f32, reading: Reading) -> Step {
        match &self.phase {
            Phase::Finished => return Step::Done(Outcome::Failed("run is over")),
            Phase::Verify(v) if v.notes.is_empty() => {
                self.phase = Phase::Finished;
                return Step::Done(Outcome::Failed("table has no range"));
            }
            _ => {}
        }
        // A long block still has to sit through settling, one read phase at a time.
        self.elapsed += dt_ms.clamp(0.0, READ_MS);
        if self.elapsed < SETTLE_MS {
            return Step::Output(self.output);
        }
        // The newest analysis has the least audio from before the voice settled in it.
        if let Some(p) = usable(reading) {
            self.last = Some(p);
        }
        if self.elapsed < SETTLE_MS + READ_MS {
            return Step::Output(self.output);
        }
        let measured = self.last.take();
        self.elapsed = 0.0;
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

        /// What the detector would say: nothing below 20 Hz.
        fn read(&self, v: f32, t_s: f32) -> Reading {
            let hz = self.hz(v, t_s);
            (hz >= 20.0 && t_s * 1000.0 < self.lose_after_ms && v >= self.silent_below).then_some(Pitch { hz, clarity: 0.95 })
        }
    }

    /// Feeds a run from a `Vco` in 5 ms blocks, with a detection every `hop_ms`. A detection
    /// taken less than `stale_ms` after the output changed still hears the previous output.
    /// Returns the outcome and the simulated milliseconds it took.
    fn drive(mut run: Run, vco: &Vco, hop_ms: f32, stale_ms: f32) -> (Outcome, f32) {
        let dt = 5.0;
        let (mut out, mut prev_out, mut age, mut t, mut next_read) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, hop_ms);
        loop {
            t += dt;
            age += dt;
            let reading = if t >= next_read {
                next_read += hop_ms;
                vco.read(if age < stale_ms { prev_out } else { out }, t / 1000.0)
            } else {
                None
            };
            match run.step(dt, reading) {
                Step::Output(v) => {
                    if v != out {
                        prev_out = out;
                        out = v;
                        age = 0.0;
                    }
                }
                Step::Done(o) => return (o, t),
            }
            assert!(t < 120_000.0, "run never finished");
        }
    }

    fn tuned(vco: &Vco) -> Table {
        match drive(Run::tune(), vco, 25.0, 0.0).0 {
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
        let mut run = Run::tune();
        let mut t = 0.0;
        let outcome = loop {
            t += 5.0;
            if let Step::Done(o) = run.step(5.0, None) {
                break o;
            }
            assert!(t < 2000.0, "still running at {t} ms");
        };
        assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
        assert!(matches!(run.step(5.0, None), Step::Done(Outcome::Failed(_))));
    }

    #[test]
    fn tune_fails_on_noise() {
        let mut run = Run::tune();
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
        let (outcome, _) = drive(Run::tune(), &vco, 40.0, 0.0);
        assert!(matches!(outcome, Outcome::Tuned { .. }), "{outcome:?}");
    }

    #[test]
    fn tune_uses_the_last_reading_of_the_phase() {
        let vco = Vco::new();
        // Detections at 100 ms into a step still hear the last output; those at 125 and 150 don't.
        let (outcome, _) = drive(Run::tune(), &vco, 25.0, 110.0);
        let Outcome::Tuned { table, .. } = outcome else { panic!("{outcome:?}") };
        let (lo, hi) = table.range();
        assert!(table.worst_cents < 5.0 && hi - lo > 50);
    }

    #[test]
    fn check_reports_the_worst_note_and_names_those_over_target() {
        let vco = Vco::new();
        let table = tuned(&vco);
        let (outcome, _) = drive(Run::check(table.clone()), &vco, 25.0, 0.0);
        let Outcome::Checked { worst_cents, off } = outcome else { panic!("{outcome:?}") };
        assert!(worst_cents < 5.0 && off.is_empty(), "{worst_cents} {off:?}");

        // The voice now runs a little wide: error grows with the output value.
        let wide = Vco { per_unit_octaves: 10.05, ..Vco::new() };
        let (outcome, _) = drive(Run::check(table.clone()), &wide, 25.0, 0.0);
        let Outcome::Checked { worst_cents, off } = outcome else { panic!("{outcome:?}") };
        assert!(!off.is_empty() && off.len() < VERIFY_NOTES, "{off:?}");
        for &n in &off {
            assert!(error_cents(&wide, &table, n).abs() > 5.0, "note {n}");
        }
        let worst = off.iter().map(|&n| error_cents(&wide, &table, n).abs()).fold(0.0, f32::max);
        assert!((worst - worst_cents).abs() < 0.01, "{worst} vs {worst_cents}");
        assert!(worst_cents > 20.0);
    }

    /// Simulated ms a plain tune takes, and how many of them are the verify.
    fn tune_ms(vco: &Vco) -> f32 {
        drive(Run::tune(), vco, 25.0, 0.0).1
    }

    const VERIFY_MS: f32 = 8.0 * 150.0;

    #[test]
    fn tune_fails_when_the_vco_goes_wrong_during_verify() {
        let plain = Vco::new();
        let sweep_end = tune_ms(&plain) - VERIFY_MS;
        let vco = Vco { after_ms: sweep_end + 1.0, late_octaves: 0.1, late_above_v: 0.0, ..Vco::new() };
        let (outcome, _) = drive(Run::tune(), &vco, 25.0, 0.0);
        assert_eq!(outcome, Outcome::Failed("tuned table did not hold up"));
    }

    #[test]
    fn tune_fails_when_the_signal_is_lost_during_verify() {
        let sweep_end = tune_ms(&Vco::new()) - VERIFY_MS;
        let vco = Vco { lose_after_ms: sweep_end + 1.0, ..Vco::new() };
        let (outcome, _) = drive(Run::tune(), &vco, 25.0, 0.0);
        assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
    }

    #[test]
    fn tune_names_a_bad_top_note_but_keeps_the_table() {
        let table = tuned(&Vco::new());
        let hi = table.range().1;
        let above = table.value_for(hi as f32).0 - 0.005;
        let sweep_end = tune_ms(&Vco::new()) - VERIFY_MS;
        let vco = Vco { after_ms: sweep_end + 1.0, late_octaves: 0.03, late_above_v: above, ..Vco::new() };
        let (outcome, _) = drive(Run::tune(), &vco, 25.0, 0.0);
        let Outcome::Tuned { table: kept, off } = outcome else { panic!("{outcome:?}") };
        assert_eq!(off, vec![hi]);
        // The worst of the notes heard includes the bad one: 0.03 octaves is 36 cents.
        assert!((kept.worst_cents - 36.0).abs() < 1.0, "{}", kept.worst_cents);
    }

    #[test]
    fn tune_fails_on_noise_for_many_seeds() {
        for seed in 0..250u32 {
            let mut run = Run::tune();
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

    /// Either a failure, or a table all of whose notes are in tune but those named in `off`.
    fn assert_honest(vco: &Vco) {
        match drive(Run::tune(), vco, 25.0, 0.0).0 {
            Outcome::Failed(_) => {}
            Outcome::Tuned { table, off } => {
                let (lo, hi) = table.range();
                for note in lo..=hi {
                    let e = error_cents(vco, &table, note);
                    assert!(e.abs() <= 5.0 || off.contains(&note), "note {note}: {e} cents, off {off:?}");
                }
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_soft_saturating_vco_gives_failure_or_an_honest_table() {
        assert_honest(&Vco { soft: true, ceiling_hz: 20000.0, ..Vco::new() });
    }

    #[test]
    fn a_non_monotonic_vco_gives_failure_or_an_honest_table() {
        assert_honest(&Vco { fold: true, ..Vco::new() });
    }

    #[test]
    fn a_vco_silent_at_the_centre_still_tunes_its_upper_side() {
        // Silent below output 0.05, so the centre step is not heard but +0.08 and up is.
        let vco = Vco { silent_below: 0.05, ..Vco::new() };
        let outcome = drive(Run::tune(), &vco, 25.0, 0.0).0;
        let Outcome::Tuned { table, .. } = outcome else { panic!("{outcome:?}") };
        assert!(table.points.iter().all(|p| p.0 >= 0.05) && table.points.len() >= 5);
    }

    #[test]
    fn one_huge_block_does_not_skip_settling() {
        let mut run = Run::tune();
        let heard = Some(Pitch { hz: 440.0, clarity: 0.95 });
        assert_eq!(run.step(1e9, heard), Step::Output(0.0));
        assert_eq!(run.step(f32::INFINITY, heard), Step::Output(0.0));
        // Only now (100 ms in) is a reading taken, and the next step comes at 150 ms.
        assert_eq!(run.step(50.0, heard), Step::Output(0.04));
    }

    #[test]
    fn check_of_an_empty_table_fails() {
        let table = Table { points: vec![], tuned_at: 0, worst_cents: 0.0 };
        assert!(matches!(Run::check(table).step(5.0, None), Step::Done(Outcome::Failed(_))));
    }
}
