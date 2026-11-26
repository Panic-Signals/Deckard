//! A note's ratchet: the note played as several quick hits inside its own length, their
//! velocities flat, falling toward a floor or rising to a ceiling. Maschine has no place
//! for it; DECKARD keeps it in its own block in the project file
//! (`docs/deckard-block.md`).

/// The field of a note's `TimedEvent` that holds its [`Ratchet`] ([`Ratchet::to_int`]) in
/// DECKARD's tree; never written to Maschine's archive.
pub const FIELD: &str = "deckard:ratchet";

/// The most hits a note plays.
pub const MOST: u8 = 16;

/// How the hits' velocities run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Shape {
    /// Every hit at the note's velocity.
    #[default]
    Flat,
    /// From the note's velocity toward the floor, which the note's end would reach.
    Down,
    /// From the note's velocity to the ceiling, which the last hit plays.
    Up,
}

impl Shape {
    pub const ALL: [Shape; 3] = [Shape::Flat, Shape::Down, Shape::Up];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Flat => "Flat",
            Shape::Down => "Down",
            Shape::Up => "Up",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ratchet {
    /// How many hits, 1–16. 1 is an ordinary note.
    pub repeats: u8,
    pub shape: Shape,
    /// [`Shape::Down`]'s target velocity, 0–127.
    pub floor: u8,
    /// [`Shape::Up`]'s target velocity, 0–127.
    pub ceiling: u8,
}

impl Default for Ratchet {
    fn default() -> Self {
        Ratchet { repeats: 1, shape: Shape::Flat, floor: 0, ceiling: 127 }
    }
}

/// One hit of a note: where it starts after the note's start, how long it lasts (ticks),
/// and how hard it plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub offset: i64,
    pub length: i64,
    pub velocity: i64,
}

impl Ratchet {
    /// An ordinary note: one hit.
    pub fn is_plain(self) -> bool {
        self.repeats <= 1
    }

    /// How many ticks a note carries into the next pattern pass. Only hits
    /// whose starts are inside the pattern play.
    pub fn loop_tail(self, position: i64, length: i64, span: std::ops::Range<i64>) -> i64 {
        if !span.contains(&position) || length <= span.end - position {
            return 0;
        }
        let left = span.end - position;
        let end = if self.is_plain() {
            length
        } else {
            self.hits(length, 100).into_iter().rev().find(|hit| hit.offset < left).map_or(0, |hit| hit.offset + hit.length)
        };
        (end - left).clamp(0, span.end - span.start)
    }

    /// The hits of a note `length` ticks long at `velocity`: the length split evenly (a
    /// note too short to give each a tick plays a hit a tick), the first at the note's
    /// velocity.
    pub fn hits(self, length: i64, velocity: i64) -> Vec<Hit> {
        let n = i64::from(self.repeats.clamp(1, MOST)).min(length.max(1));
        let v = velocity as f64;
        (0..n)
            .map(|k| {
                let offset = k * length / n;
                let end = if k + 1 == n { length } else { (k + 1) * length / n };
                let to = match self.shape {
                    Shape::Flat => v,
                    Shape::Down => v + (f64::from(self.floor).min(v) - v) * k as f64 / n as f64,
                    Shape::Up if n > 1 => v + (f64::from(self.ceiling).max(v) - v) * k as f64 / (n - 1) as f64,
                    Shape::Up => v,
                };
                let velocity = if k == 0 { velocity } else { (to.round() as i64).clamp(1, 127) };
                Hit { offset, length: end - offset, velocity }
            })
            .collect()
    }

    /// The four values in one number: repeats, then shape, floor and ceiling a byte each.
    pub fn to_int(self) -> i64 {
        let shape = Shape::ALL.iter().position(|&s| s == self.shape).unwrap_or(0) as i64;
        i64::from(self.repeats) | shape << 8 | i64::from(self.floor) << 16 | i64::from(self.ceiling) << 24
    }

    /// [`Ratchet::to_int`] back, each value kept in its range.
    pub fn from_int(v: i64) -> Ratchet {
        let byte = |shift: u32| ((v >> shift) & 0xff) as u8;
        Ratchet {
            repeats: byte(0).clamp(1, MOST),
            shape: Shape::ALL[usize::from(byte(8)).min(2)],
            floor: byte(16).min(127),
            ceiling: byte(24).min(127),
        }
    }

    /// The velocity the shape heads for: Down's floor, Up's ceiling.
    pub fn target(self) -> Option<u8> {
        match self.shape {
            Shape::Flat => None,
            Shape::Down => Some(self.floor),
            Shape::Up => Some(self.ceiling),
        }
    }

    /// With the shape's target `v` (Flat has none, and is unchanged).
    pub fn with_target(self, v: u8) -> Ratchet {
        let v = v.min(127);
        match self.shape {
            Shape::Flat => self,
            Shape::Down => Ratchet { floor: v, ..self },
            Shape::Up => Ratchet { ceiling: v, ..self },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(repeats: u8, shape: Shape, floor: u8, ceiling: u8) -> Ratchet {
        Ratchet { repeats, shape, floor, ceiling }
    }
    fn vels(h: &[Hit]) -> Vec<i64> {
        h.iter().map(|h| h.velocity).collect()
    }

    #[test]
    fn a_plain_note_is_one_hit_of_itself() {
        assert_eq!(Ratchet::default().hits(240, 100), [Hit { offset: 0, length: 240, velocity: 100 }]);
    }

    #[test]
    fn the_hits_split_the_length_evenly_and_add_up_to_it() {
        let h = r(4, Shape::Flat, 0, 127).hits(960, 90);
        assert_eq!(h.iter().map(|h| (h.offset, h.length)).collect::<Vec<_>>(), [(0, 240), (240, 240), (480, 240), (720, 240)]);
        let h = r(3, Shape::Flat, 0, 127).hits(100, 90);
        assert_eq!(h.iter().map(|h| h.length).sum::<i64>(), 100);
        assert_eq!(h[2].offset + h[2].length, 100);
    }

    #[test]
    fn down_falls_toward_the_floor_and_up_ends_on_the_ceiling() {
        assert_eq!(vels(&r(4, Shape::Down, 0, 127).hits(960, 100)), [100, 75, 50, 25]);
        assert_eq!(vels(&r(4, Shape::Down, 60, 127).hits(960, 100)), [100, 90, 80, 70]);
        assert_eq!(vels(&r(4, Shape::Up, 0, 127).hits(960, 67)), [67, 87, 107, 127]);
        assert_eq!(vels(&r(3, Shape::Up, 0, 100).hits(960, 40)), [40, 70, 100]);
    }

    #[test]
    fn a_target_on_the_wrong_side_is_flat_and_no_hit_is_silent() {
        assert_eq!(vels(&r(3, Shape::Down, 120, 127).hits(960, 100)), [100, 100, 100]);
        assert_eq!(vels(&r(3, Shape::Up, 0, 20).hits(960, 100)), [100, 100, 100]);
        assert!(vels(&r(16, Shape::Down, 0, 127).hits(960, 2)).iter().all(|&v| (1..=127).contains(&v)));
    }

    #[test]
    fn a_note_shorter_than_its_repeats_plays_a_hit_a_tick() {
        let h = r(16, Shape::Up, 0, 127).hits(3, 27);
        assert_eq!(h.iter().map(|h| (h.offset, h.length)).collect::<Vec<_>>(), [(0, 1), (1, 1), (2, 1)]);
        assert_eq!(vels(&h), [27, 77, 127]);
        assert_eq!(r(4, Shape::Flat, 0, 127).hits(0, 90).len(), 1);
    }

    #[test]
    fn it_packs_into_one_number_and_back() {
        let x = r(7, Shape::Up, 12, 99);
        assert_eq!(Ratchet::from_int(x.to_int()), x);
        assert_eq!(Ratchet::from_int(0).repeats, 1);
        assert_eq!(Ratchet::from_int(200 | 9 << 8).repeats, 16);
        assert_eq!(
            (x.target(), x.with_target(50).ceiling, r(2, Shape::Down, 5, 9).with_target(50).floor, r(2, Shape::Flat, 5, 9).with_target(50)),
            (Some(99), 50, 50, r(2, Shape::Flat, 5, 9))
        );
    }
}
