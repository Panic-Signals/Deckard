//! Note Repeat and the arpeggiator (MK3 manual, "Note Repeat and Arp"): pads held go
//! in, notes come out on the rate's grid. Note Repeat retriggers every pad held; the
//! Arp plays the held pitches one at a time (or together, Chord) in its order, over
//! its octaves, shaped by retrigger, repeat, offset, inversion and a rhythm with
//! rests. Times are ticks of a clock that runs on (the engine's), its steps aligned to
//! the song's when it plays.

use deckard_model::{Arp, ArpType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    /// Pads in group mode.
    NoteRepeat,
    /// Pads in keyboard or chord mode.
    Arp,
}

/// A note it plays: when (clock ticks), what, and how long (ticks).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fired {
    pub at: f64,
    pub group: usize,
    pub sound: usize,
    pub pitch: i64,
    pub velocity: i64,
    pub length: i64,
}

/// The rhythms (SEQUENCE 1–8): steps on (a note) or off (a rest). Maschine's own aren't
/// documented; these are DECKARD's.
pub const SEQUENCES: [&[bool]; 8] = [
    &[true, true, true, false],
    &[true, false, true, true],
    &[true, true, false, true],
    &[true, false, false, true, false, false, true, false],
    &[true, true, true, false, true, true, false, true],
    &[true, false, true, false, true, true, false, true],
    &[true, true, false, false],
    &[true, false, true, true, false, true, true, false],
];

#[derive(Debug, Clone, Copy, PartialEq)]
struct Held {
    group: usize,
    sound: usize,
    pitch: i64,
    velocity: i64,
}

pub struct Repeater {
    pub arp: Arp,
    pub mode: RepeatMode,
    /// What plays, in the order pressed; what is physically down.
    held: Vec<Held>,
    down: Vec<(usize, usize, i64)>,
    /// With HOLD: everything was let go, so the next press starts a new chord.
    latched: bool,
    next_at: Option<f64>,
    step: usize,
    note_i: usize,
    seq_i: usize,
}

impl Repeater {
    pub fn new(arp: Arp, mode: RepeatMode) -> Repeater {
        Repeater { arp, mode, held: Vec::new(), down: Vec::new(), latched: false, next_at: None, step: 0, note_i: 0, seq_i: 0 }
    }

    /// Whether anything plays.
    pub fn active(&self) -> bool {
        !self.held.is_empty()
    }

    pub fn press(&mut self, group: usize, sound: usize, pitch: i64, velocity: i64) {
        if self.latched {
            self.held.clear();
            self.latched = false;
        }
        self.down.retain(|d| *d != (group, sound, pitch));
        self.down.push((group, sound, pitch));
        if !self.held.iter().any(|h| (h.group, h.sound, h.pitch) == (group, sound, pitch)) {
            self.held.push(Held { group, sound, pitch, velocity });
        }
    }

    /// A held pad or key pressed harder or softer: its notes play at `velocity` from
    /// now on (as Maschine's pad pressure does).
    pub fn pressure(&mut self, group: usize, sound: usize, pitch: i64, velocity: i64) {
        for h in self.held.iter_mut().filter(|h| (h.group, h.sound, h.pitch) == (group, sound, pitch)) {
            h.velocity = velocity.clamp(1, 127);
        }
    }

    pub fn release(&mut self, group: usize, sound: usize, pitch: i64) {
        self.down.retain(|d| *d != (group, sound, pitch));
        if self.arp.hold {
            self.latched = self.down.is_empty();
        } else {
            self.held.retain(|h| (h.group, h.sound, h.pitch) != (group, sound, pitch));
        }
    }

    /// Let go of everything (HOLD off, the mode left).
    pub fn clear(&mut self) {
        self.held.clear();
        self.down.clear();
        self.latched = false;
    }

    /// The notes due from clock tick `now` for `span` ticks; `song`: where the song is
    /// at `now` (its steps align to the rate's grid), `None` when it's stopped.
    pub fn due(&mut self, now: f64, span: f64, song: Option<f64>) -> Vec<Fired> {
        let mut out = Vec::new();
        self.due_into(now, span, song, &mut out);
        out
    }

    /// [`Repeater::due`] into `out` (cleared first; the audio thread keeps one).
    pub fn due_into(&mut self, now: f64, span: f64, song: Option<f64>, out: &mut Vec<Fired>) {
        out.clear();
        if self.held.is_empty() {
            self.next_at = None;
            self.step = 0;
            self.note_i = 0;
            self.seq_i = 0;
            return;
        }
        let rate = self.arp.rate.max(1) as f64;
        let mut next = self.next_at.unwrap_or_else(|| match song {
            Some(pos) => {
                let phase = pos.rem_euclid(rate);
                if phase < 1e-6 || rate - phase < 1e-6 { now } else { now + rate - phase }
            }
            None => now,
        });
        while next < now + span - 1e-9 {
            self.fire(next, out);
            next += rate;
        }
        self.next_at = Some(next);
    }

    fn fire(&mut self, at: f64, out: &mut Vec<Fired>) {
        let length = ((self.arp.rate as f64 * self.arp.gate).round() as i64).max(1);
        let vel = |v: i64, dynamic: f64| ((v as f64 * dynamic).round() as i64).clamp(1, 127);
        match self.mode {
            RepeatMode::NoteRepeat => {
                for h in &self.held {
                    out.push(Fired {
                        at,
                        group: h.group,
                        sound: h.sound,
                        pitch: h.pitch,
                        velocity: vel(h.velocity, self.arp.dynamic),
                        length,
                    });
                }
            }
            RepeatMode::Arp => {
                let a = &self.arp;
                if a.retrigger > 0 && self.step > 0 && self.step % a.retrigger as usize == 0 {
                    self.note_i = 0;
                    self.seq_i = 0;
                }
                self.step += 1;
                let on = match SEQUENCES.get((a.sequence - 1).max(-1) as usize).filter(|_| a.sequence > 0) {
                    Some(s) => s[self.seq_i % s.len()],
                    None => true,
                };
                self.seq_i += 1;
                if !on {
                    return;
                }
                let cycle = self.cycle();
                if cycle.is_empty() {
                    return;
                }
                if a.kind == ArpType::Chord {
                    for h in cycle.iter().take(self.chord_size()) {
                        out.push(Fired { at, velocity: vel(h.velocity, a.dynamic), length, ..fired(h) });
                    }
                    self.note_i += 1;
                    return;
                }
                let n = a.repeat.max(1) as usize;
                let idx = ((self.note_i / n) as i64 % cycle.len() as i64 + a.offset).rem_euclid(cycle.len() as i64) as usize;
                let h = cycle[idx];
                out.push(Fired { at, velocity: vel(h.velocity, a.dynamic), length, ..fired(&h) });
                self.note_i += 1;
            }
        }
    }

    /// How many pitches a chord plays (the held ones in range).
    fn chord_size(&self) -> usize {
        self.held.iter().filter(|h| (self.arp.min_key..=self.arp.max_key).contains(&h.pitch)).count()
    }

    /// The arpeggio's cycle: the held pitches in range, in its order, over its
    /// octaves, with its inversions after.
    fn cycle(&self) -> Vec<Held> {
        let a = &self.arp;
        let mut base: Vec<Held> = self.held.iter().filter(|h| (a.min_key..=a.max_key).contains(&h.pitch)).copied().collect();
        if a.kind != ArpType::OrderPlayed {
            base.sort_by_key(|h| h.pitch);
        }
        let up = |h: &Held, o: i64| Held { pitch: (h.pitch + 12 * o).min(127), ..*h };
        let mut cycle: Vec<Held> = (0..a.octaves.max(1)).flat_map(|o| base.iter().map(move |h| up(h, o))).collect();
        let len = base.len();
        for i in 1..=a.inversion.max(0) as usize {
            if len == 0 {
                break;
            }
            let r = i % len;
            cycle.extend(base[r..].iter().copied().chain(base[..r].iter().map(|h| up(h, 1))));
        }
        match a.kind {
            ArpType::Down => cycle.reverse(),
            ArpType::UpDown if cycle.len() > 2 => {
                let back: Vec<Held> = cycle[1..cycle.len() - 1].iter().rev().copied().collect();
                cycle.extend(back);
            }
            _ => {}
        }
        cycle
    }
}

fn fired(h: &Held) -> Fired {
    Fired { at: 0.0, group: h.group, sound: h.sound, pitch: h.pitch, velocity: h.velocity, length: 1 }
}

#[cfg(test)]
mod tests {
    use super::{Fired, RepeatMode, Repeater};
    use deckard_model::{Arp, ArpType};

    fn arp(kind: ArpType) -> Arp {
        Arp { kind, rate: 240, ..Arp::default() }
    }

    /// The pitches fired from `from` to `to` (ticks), in order.
    fn pitches(r: &mut Repeater, from: f64, to: f64) -> Vec<i64> {
        r.due(from, to - from, Some(from)).into_iter().map(|f| f.pitch).collect()
    }

    /// Pressing a held pad harder or softer sets what its repeats play at.
    #[test]
    fn pressure_changes_repeat_velocity() {
        let mut r = Repeater::new(Arp { rate: 240, gate: 0.5, ..Arp::default() }, RepeatMode::NoteRepeat);
        r.press(0, 3, 60, 100);
        assert!(r.due(0.0, 240.0, Some(0.0)).iter().all(|f| f.velocity == 100));
        r.pressure(0, 3, 60, 38);
        let fired = r.due(240.0, 480.0, Some(240.0));
        assert!(!fired.is_empty() && fired.iter().all(|f| f.velocity == 38), "{fired:?}");
        // Another pad isn't touched.
        r.pressure(0, 4, 60, 10);
        assert!(r.due(720.0, 240.0, Some(720.0)).iter().all(|f| f.velocity == 38));
    }

    #[test]
    fn note_repeat_repeats_each_held_pad_at_the_rate() {
        let mut r = Repeater::new(Arp { rate: 240, gate: 0.5, ..Arp::default() }, RepeatMode::NoteRepeat);
        r.press(0, 3, 60, 100);
        let fired: Vec<Fired> = r.due(0.0, 960.0, Some(0.0));
        assert_eq!(fired.iter().map(|f| f.at).collect::<Vec<_>>(), [0.0, 240.0, 480.0, 720.0]);
        assert!(fired.iter().all(|f| (f.group, f.sound, f.pitch, f.velocity, f.length) == (0, 3, 60, 100, 120)));
        // Two pads: both each time.
        r.press(0, 5, 60, 80);
        assert_eq!(r.due(960.0, 240.0, Some(960.0)).len(), 2);
        // Released: nothing more.
        r.release(0, 3, 60);
        r.release(0, 5, 60);
        assert!(r.due(1200.0, 960.0, Some(1200.0)).is_empty());
    }

    #[test]
    fn the_first_note_waits_for_the_next_step_of_the_song() {
        let mut r = Repeater::new(arp(ArpType::Up), RepeatMode::NoteRepeat);
        r.press(0, 0, 60, 100);
        // Pressed at song tick 100: the next 1/16 is 240.
        let fired = r.due(1000.0, 400.0, Some(100.0));
        assert_eq!(fired.iter().map(|f| f.at).collect::<Vec<_>>(), [1140.0, 1380.0]);
    }

    #[test]
    fn arp_orders() {
        let held = |r: &mut Repeater| {
            for p in [64, 60, 67] {
                r.press(0, 0, p, 100);
            }
        };
        let mut up = Repeater::new(arp(ArpType::Up), RepeatMode::Arp);
        held(&mut up);
        assert_eq!(pitches(&mut up, 0.0, 240.0 * 6.0), [60, 64, 67, 60, 64, 67]);
        let mut down = Repeater::new(arp(ArpType::Down), RepeatMode::Arp);
        held(&mut down);
        assert_eq!(pitches(&mut down, 0.0, 240.0 * 3.0), [67, 64, 60]);
        let mut ud = Repeater::new(arp(ArpType::UpDown), RepeatMode::Arp);
        held(&mut ud);
        assert_eq!(pitches(&mut ud, 0.0, 240.0 * 5.0), [60, 64, 67, 64, 60]);
        let mut played = Repeater::new(arp(ArpType::OrderPlayed), RepeatMode::Arp);
        held(&mut played);
        assert_eq!(pitches(&mut played, 0.0, 240.0 * 3.0), [64, 60, 67]);
        let mut chord = Repeater::new(arp(ArpType::Chord), RepeatMode::Arp);
        held(&mut chord);
        assert_eq!(pitches(&mut chord, 0.0, 240.0 * 2.0), [60, 64, 67, 60, 64, 67]);
        // Octaves: the cycle again an octave up.
        let mut two = Repeater::new(Arp { octaves: 2, ..arp(ArpType::Up) }, RepeatMode::Arp);
        held(&mut two);
        assert_eq!(pitches(&mut two, 0.0, 240.0 * 6.0), [60, 64, 67, 72, 76, 79]);
        // Keys outside min–max don't feed it.
        let mut range = Repeater::new(Arp { min_key: 61, max_key: 66, ..arp(ArpType::Up) }, RepeatMode::Arp);
        held(&mut range);
        assert_eq!(pitches(&mut range, 0.0, 240.0 * 2.0), [64, 64]);
    }

    /// Page 2 as the manual's examples have it.
    #[test]
    fn arp_retrigger_repeat_offset() {
        let five = |r: &mut Repeater| {
            for p in [60, 62, 64, 65, 67] {
                r.press(0, 0, p, 100);
            }
        };
        let mut re = Repeater::new(Arp { retrigger: 3, ..arp(ArpType::Up) }, RepeatMode::Arp);
        five(&mut re);
        assert_eq!(pitches(&mut re, 0.0, 240.0 * 6.0), [60, 62, 64, 60, 62, 64]);
        let mut rep = Repeater::new(Arp { repeat: 3, ..arp(ArpType::Up) }, RepeatMode::Arp);
        rep.press(0, 0, 60, 100);
        rep.press(0, 0, 62, 100);
        assert_eq!(pitches(&mut rep, 0.0, 240.0 * 6.0), [60, 60, 60, 62, 62, 62]);
        let mut off = Repeater::new(Arp { offset: 1, ..arp(ArpType::Up) }, RepeatMode::Arp);
        five(&mut off);
        assert_eq!(pitches(&mut off, 0.0, 240.0 * 5.0), [62, 64, 65, 67, 60]);
        let mut back = Repeater::new(Arp { offset: -1, ..arp(ArpType::Up) }, RepeatMode::Arp);
        five(&mut back);
        assert_eq!(pitches(&mut back, 0.0, 240.0 * 2.0), [67, 60]);
    }

    #[test]
    fn dynamic_scales_velocity_and_hold_latches() {
        let mut r = Repeater::new(Arp { dynamic: 0.5, hold: true, ..arp(ArpType::Up) }, RepeatMode::Arp);
        r.press(0, 0, 60, 100);
        r.release(0, 0, 60);
        let fired = r.due(0.0, 480.0, Some(0.0));
        assert_eq!(fired.iter().map(|f| f.velocity).collect::<Vec<_>>(), [50, 50], "held on after release");
        // A new press after all were released starts a new chord.
        r.press(0, 0, 72, 100);
        assert_eq!(r.due(480.0, 240.0, Some(480.0))[0].pitch, 72);
    }

    #[test]
    fn a_sequence_rests_on_its_steps() {
        let mut r = Repeater::new(Arp { sequence: 1, ..arp(ArpType::Up) }, RepeatMode::Arp);
        r.press(0, 0, 60, 100);
        let steps = super::SEQUENCES[0];
        let fired = r.due(0.0, 240.0 * steps.len() as f64, Some(0.0));
        assert_eq!(fired.len(), steps.iter().filter(|&&on| on).count());
    }
}
