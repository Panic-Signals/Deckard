//! Pads blink as the sequencer starts each note, as for a drum hit, however long the
//! note is held: every note that started since the last look lights for
//! [`FLASH`], so a note between two looks still shows, and each blink is as long at any
//! tempo.

use deckard_engine::output::FLASH;
use deckard_engine::{Mode, Timeline};
use deckard_model::PPQ;
use std::collections::HashSet;
use std::time::Instant;

/// (group, sound, pitch).
pub type Played = (usize, usize, i64);

/// Further than this between looks is a jump (a restart, a new scene), not playing on.
const JUMP: i64 = PPQ;

#[derive(Default)]
pub struct Blinks {
    /// The mode and position of the last look.
    last: Option<(Mode, i64)>,
    on: Vec<(Played, Instant)>,
}

impl Blinks {
    /// What blinks now, playing `t` in `mode` at `pos` (in its loop).
    pub fn update(&mut self, t: &Timeline, mode: Mode, pos: i64, now: Instant) -> HashSet<Played> {
        let len = t.length.max(1);
        let from = match self.last {
            Some((m, last)) if m == mode && (pos - last).rem_euclid(len) <= JUMP => Some(last),
            _ => None,
        };
        // Notes starting after `from` up to `pos`, round the loop's end if it came round.
        let started = |tick: i64| match from {
            Some(f) if f <= pos => f < tick && tick <= pos,
            Some(f) => f < tick || tick <= pos,
            None => pos - PPQ / 16 < tick && tick <= pos,
        };
        for e in t.events.iter().filter(|e| e.midi.is_none() && started(e.tick)) {
            let k = (e.group, e.sound, e.pitch);
            self.on.retain(|(p, _)| *p != k);
            self.on.push((k, now));
        }
        self.last = Some((mode, pos));
        self.lit(now)
    }

    /// Stopped: what still blinks; playing again starts afresh.
    pub fn stopped(&mut self, now: Instant) -> HashSet<Played> {
        self.last = None;
        self.lit(now)
    }

    fn lit(&mut self, now: Instant) -> HashSet<Played> {
        self.on.retain(|(_, at)| now.duration_since(*at) < FLASH);
        self.on.iter().map(|(p, _)| *p).collect()
    }
}
