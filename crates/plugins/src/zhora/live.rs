//! Tune-while-playing: a slow trim that follows the voice's drift.

use super::pitch::Pitch;
use super::tune::{cents, note_hz, usable};

/// A note is only read this long after it starts, so the last note's audio is gone.
const SETTLE_MS: f32 = 150.0;
/// A reading further than this from the note is not about it.
const GATE_CENTS: f32 = 50.0;
/// The most the trim moves, in cents per second.
const MAX_RATE: f32 = 2.0;
/// The trim never goes past this, in cents.
const LIMIT_CENTS: f32 = 30.0;
/// Time constant of the follower.
const TAU_MS: f32 = 1000.0;
/// Longest stretch one reading stands for.
const MAX_DT_MS: f32 = 500.0;
/// The trim counts as locked this long after its last good reading.
const LOCK_MS: f32 = 500.0;

/// Correction added to the note, in semitones, so the voice stays in tune as it drifts.
pub struct Trim {
    cents: f32,
    /// Time since a reading was last used.
    idle_ms: f32,
    since_good_ms: f32,
    last_since_note_ms: f32,
}

impl Trim {
    pub fn new() -> Trim {
        Trim { cents: 0.0, idle_ms: 0.0, since_good_ms: LOCK_MS, last_since_note_ms: 0.0 }
    }

    /// Advances by `dt_ms` and returns the trim in semitones.
    ///
    /// `note` is the note asked for, bend included. `table_hz` is the pitch the table believes
    /// it is sending for it (trim included), so the reading's error against it is the voice's
    /// own drift. `reading` should be `Some` only for a new detection.
    pub fn update(&mut self, dt_ms: f32, note: f32, table_hz: f32, since_note_ms: f32, reading: Option<Pitch>) -> f32 {
        let dt = if dt_ms.is_finite() { dt_ms.max(0.0) } else { 0.0 };
        self.idle_ms = (self.idle_ms + dt).min(1e6);
        self.since_good_ms = (self.since_good_ms + dt).min(LOCK_MS);
        // A new note starts over; what was locked on belonged to the last one.
        if since_note_ms < self.last_since_note_ms {
            self.since_good_ms = LOCK_MS;
        }
        self.last_since_note_ms = since_note_ms;

        let inputs_ok = note.is_finite() && table_hz.is_finite() && table_hz > 0.0 && since_note_ms >= SETTLE_MS;
        let heard = usable(reading).filter(|p| inputs_ok && cents(p.hz, note_hz(note)).abs() <= GATE_CENTS);
        if let Some(p) = heard {
            let span = self.idle_ms.min(MAX_DT_MS);
            let target = (-cents(p.hz, table_hz)).clamp(-LIMIT_CENTS, LIMIT_CENTS);
            let follow = (target - self.cents) * (1.0 - (-span / TAU_MS).exp());
            let limit = MAX_RATE * span / 1000.0;
            self.cents = (self.cents + follow.clamp(-limit, limit)).clamp(-LIMIT_CENTS, LIMIT_CENTS);
            self.idle_ms = 0.0;
            self.since_good_ms = 0.0;
        }
        self.cents / 100.0
    }

    /// True while recent readings have been clear and about the note being played.
    pub fn locked(&self) -> bool {
        self.since_good_ms < LOCK_MS
    }
}

impl Default for Trim {
    fn default() -> Self {
        Trim::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: f32 = 57.0;

    /// Plays `NOTE` for `secs` against a voice that is `dev(t)` cents off the table. Returns
    /// the trim in cents after each 10 ms block, and the trim itself.
    fn play(secs: f32, hop_ms: f32, dev: impl Fn(f32) -> f32) -> (Vec<f32>, Trim) {
        let mut trim = Trim::new();
        let mut history = Vec::new();
        let mut semis = 0.0;
        let (mut t_ms, mut next_read) = (0.0f32, hop_ms);
        while t_ms < secs * 1000.0 {
            t_ms += 10.0;
            // The table sends the note plus the trim and believes it gets exactly that.
            let table_hz = note_hz(NOTE + semis);
            let reading = (t_ms >= next_read).then(|| {
                next_read += hop_ms;
                Pitch { hz: table_hz * (dev(t_ms / 1000.0) / 1200.0).exp2(), clarity: 0.95 }
            });
            semis = trim.update(10.0, NOTE, table_hz, t_ms, reading);
            history.push(semis * 100.0);
        }
        (history, trim)
    }

    #[test]
    fn live_trims_slow_drift_to_within_5_cents() {
        // 20 cents of drift over 30 s.
        let dev = |t: f32| 20.0 * (t / 30.0).min(1.0);
        let (trims, trim) = play(40.0, 50.0, dev);
        for (i, c) in trims.iter().enumerate().skip(500) {
            let residual = dev(i as f32 * 0.01 + 0.01) + c;
            assert!(residual.abs() < 5.0, "at {} s: {residual}", i / 100);
        }
        assert!(trim.locked());
    }

    #[test]
    fn live_ignores_vibrato() {
        let (trims, _) = play(5.0, 50.0, |t| 30.0 * (std::f32::consts::TAU * 6.0 * t).sin());
        let most = trims.iter().fold(0.0f32, |m, c| m.max(c.abs()));
        assert!(most < 3.0, "trim moved {most} cents");
    }

    #[test]
    fn live_holds_with_no_clear_pitch() {
        let (_, mut trim) = play(5.0, 50.0, |_| 10.0);
        let held = trim.update(10.0, NOTE, note_hz(NOTE), 6000.0, None) * 100.0;
        assert!(held < -3.0, "never moved: {held}");
        assert!(trim.locked());
        let mut now = 0.0;
        // No pitch, then a muddy one.
        for i in 0..100 {
            now = trim.update(10.0, NOTE, note_hz(NOTE), 6000.0 + i as f32 * 10.0, None);
        }
        assert!(!trim.locked());
        assert_eq!(now * 100.0, held);
        for i in 0..100 {
            let muddy = Some(Pitch { hz: note_hz(NOTE) * 1.01, clarity: 0.5 });
            now = trim.update(10.0, NOTE, note_hz(NOTE), 7000.0 + i as f32 * 10.0, muddy);
        }
        assert!(!trim.locked());
        assert_eq!(now * 100.0, held);
    }

    #[test]
    fn live_ignores_a_reading_from_the_last_note() {
        let mut trim = Trim::new();
        let far = Some(Pitch { hz: note_hz(NOTE) * (200.0f32 / 1200.0).exp2(), clarity: 0.95 });
        for since in [20.0, 100.0, 149.0, 300.0, 5000.0] {
            for _ in 0..100 {
                assert_eq!(trim.update(10.0, NOTE, note_hz(NOTE), since, far), 0.0, "since {since}");
            }
        }
        assert!(!trim.locked());
        // Even 40 cents away is the last note's if it was read too early.
        let near = Some(Pitch { hz: note_hz(NOTE) * (40.0f32 / 1200.0).exp2(), clarity: 0.95 });
        assert_eq!(trim.update(10.0, NOTE, note_hz(NOTE), 100.0, near), 0.0);
        assert_ne!(trim.update(10.0, NOTE, note_hz(NOTE), 200.0, near), 0.0);
    }

    #[test]
    fn live_is_limited_to_30_cents() {
        let (trims, _) = play(40.0, 50.0, |_| -45.0);
        assert!(trims.iter().all(|c| *c <= 30.0 + 1e-3));
        assert!((trims.last().unwrap() - 30.0).abs() < 0.1);
        // And it gets there slowly: at most 2 cents per second.
        assert!(trims[500] <= 10.5, "{} cents after 5 s", trims[500]);
    }

    #[test]
    fn live_does_not_chase_a_clamped_note() {
        // The table ends at note 80; the note asked for is 100, and the voice sounds the end.
        let end_hz = note_hz(80.0);
        let mut trim = Trim::new();
        for i in 0..1000 {
            let reading = Some(Pitch { hz: end_hz * 1.003, clarity: 0.95 });
            assert_eq!(trim.update(10.0, 100.0, end_hz, 200.0 + i as f32 * 10.0, reading), 0.0);
        }
        assert!(!trim.locked());
    }

    #[test]
    fn live_survives_nonsense_inputs() {
        let mut trim = Trim::new();
        let p = Some(Pitch { hz: 440.0, clarity: 0.95 });
        for (note, hz) in [(f32::NAN, 440.0), (69.0, f32::NAN), (69.0, 0.0), (f32::INFINITY, 440.0)] {
            assert_eq!(trim.update(f32::NAN, note, hz, 500.0, p), 0.0);
        }
    }
}
