//! Reports to events: the JAM reports only on a change, the whole state each time.

use crate::map::{self, Control};

/// A strip finger's movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Touch {
    Down,
    Move,
    Up,
}

impl Touch {
    /// What the scripts call it.
    pub fn name(self) -> &'static str {
        match self {
            Touch::Down => "down",
            Touch::Move => "move",
            Touch::Up => "up",
        }
    }
}

/// What the controller did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// A button by its name in [`crate::map`].
    Button(&'static str, bool),
    /// A pad: column and row from 0, the top left first.
    Pad { column: u8, row: u8, down: bool },
    /// Strip `strip` (from 0), finger 0 or 1, at `pos` (0..1).
    Strip { strip: u8, finger: u8, kind: Touch, pos: f64 },
    /// The encoder turned this many clicks; + is clockwise.
    Encoder(i32),
}

/// Keeps the last state, to tell what changed.
#[derive(Debug, Default)]
pub struct Decoder {
    buttons: [u8; 15],
    encoder: Option<u8>,
    strips: [[u16; 2]; 8],
}

impl Decoder {
    /// One report, its id first; anything else is ignored.
    pub fn decode(&mut self, r: &[u8], out: &mut Vec<Event>) {
        match r.first() {
            Some(0x01) if r.len() >= 17 => self.buttons(r, out),
            Some(0x02) if r.len() >= 49 => self.strips(r, out),
            _ => {}
        }
    }

    fn buttons(&mut self, r: &[u8], out: &mut Vec<Event>) {
        let position = r[1] & 0x0f;
        if let Some(was) = self.encoder.replace(position)
            && was != position
        {
            // The shortest way round the 16 positions.
            let steps = (i32::from(position) - i32::from(was) + 8).rem_euclid(16) - 8;
            out.push(Event::Encoder(steps));
        }
        for (byte, (&now, was)) in r[2..17].iter().zip(self.buttons.iter_mut()).enumerate() {
            let changed = now ^ *was;
            *was = now;
            for bit in (0..8).filter(|b| changed & (1 << b) != 0) {
                let down = now & (1 << bit) != 0;
                match map::control(byte * 8 + bit) {
                    Control::Button(name) => out.push(Event::Button(name, down)),
                    Control::Pad(column, row) => out.push(Event::Pad { column, row, down }),
                    Control::Nothing => {}
                }
            }
        }
    }

    fn strips(&mut self, r: &[u8], out: &mut Vec<Event>) {
        // Each strip: a timestamp, then two fingers, each 0 while not touching.
        for (strip, (rec, was)) in r[1..49].chunks_exact(6).zip(self.strips.iter_mut()).enumerate() {
            for finger in 0..2 {
                let at = 2 + finger * 2;
                let now = u16::from_le_bytes([rec[at], rec[at + 1]]);
                let old = was[finger];
                was[finger] = now;
                let pos = |v: u16| (f64::from(v) / map::STRIP_RANGE).clamp(0.0, 1.0);
                let event = match (old, now) {
                    (0, 0) => continue,
                    (0, n) => (Touch::Down, pos(n)),
                    (o, 0) => (Touch::Up, pos(o)),
                    (o, n) if o != n => (Touch::Move, pos(n)),
                    _ => continue,
                };
                out.push(Event::Strip { strip: strip as u8, finger: finger as u8, kind: event.0, pos: event.1 });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::FIRST_PAD_BIT;

    fn buttons(encoder: u8, bits: &[usize]) -> Vec<u8> {
        let mut r = vec![0u8; 17];
        r[0] = 0x01;
        r[1] = encoder;
        for &b in bits {
            r[2 + b / 8] |= 1 << (b % 8);
        }
        r
    }

    fn strips(fingers: &[(usize, usize, u16)]) -> Vec<u8> {
        let mut r = vec![0u8; 49];
        r[0] = 0x02;
        for &(strip, finger, v) in fingers {
            let at = 1 + strip * 6 + 2 + finger * 2;
            r[at..at + 2].copy_from_slice(&v.to_le_bytes());
        }
        r
    }

    fn run(d: &mut Decoder, r: &[u8]) -> Vec<Event> {
        let mut out = Vec::new();
        d.decode(r, &mut out);
        out
    }

    #[test]
    fn a_press_and_a_release_are_named() {
        let mut d = Decoder::default();
        assert_eq!(run(&mut d, &buttons(0, &[0])), [Event::Button("SONG", true)]);
        assert_eq!(run(&mut d, &buttons(0, &[0])), [], "only changes");
        assert_eq!(run(&mut d, &buttons(0, &[])), [Event::Button("SONG", false)]);
    }

    #[test]
    fn pads_say_their_place() {
        let mut d = Decoder::default();
        let bit = FIRST_PAD_BIT + 8 + 3;
        assert_eq!(run(&mut d, &buttons(0, &[bit])), [Event::Pad { column: 3, row: 1, down: true }]);
        assert_eq!(run(&mut d, &buttons(0, &[])), [Event::Pad { column: 3, row: 1, down: false }]);
    }

    #[test]
    fn two_changes_in_one_report_are_both_seen() {
        let mut d = Decoder::default();
        assert_eq!(run(&mut d, &buttons(0, &[0, 1])), [Event::Button("SONG", true), Event::Button("SCENE_1", true)]);
    }

    /// The first report only sets where the encoder is; it wraps round its 16 places, and
    /// the byte's high nibble (unknown) isn't its.
    #[test]
    fn the_encoder_counts_clicks_the_short_way_round() {
        let mut d = Decoder::default();
        assert_eq!(run(&mut d, &buttons(5, &[])), []);
        assert_eq!(run(&mut d, &buttons(6, &[])), [Event::Encoder(1)]);
        assert_eq!(run(&mut d, &buttons(4, &[])), [Event::Encoder(-2)]);
        assert_eq!(run(&mut d, &buttons(0x34, &[])), [], "the high nibble alone");
        assert_eq!(run(&mut d, &buttons(15, &[])), [Event::Encoder(-5)]);
        assert_eq!(run(&mut d, &buttons(0, &[])), [Event::Encoder(1)], "15 → 0 goes up");
    }

    #[test]
    fn a_finger_goes_down_moves_and_comes_up() {
        let mut d = Decoder::default();
        let at = |v: f64| v / 1024.0;
        assert_eq!(run(&mut d, &strips(&[(2, 0, 512)])), [Event::Strip { strip: 2, finger: 0, kind: Touch::Down, pos: at(512.0) }]);
        assert_eq!(run(&mut d, &strips(&[(2, 0, 600)])), [Event::Strip { strip: 2, finger: 0, kind: Touch::Move, pos: at(600.0) }]);
        assert_eq!(run(&mut d, &strips(&[(2, 0, 600)])), [], "held still");
        assert_eq!(run(&mut d, &strips(&[])), [Event::Strip { strip: 2, finger: 0, kind: Touch::Up, pos: at(600.0) }]);
    }

    #[test]
    fn a_second_finger_is_its_own() {
        let mut d = Decoder::default();
        let e = run(&mut d, &strips(&[(0, 0, 100), (0, 1, 900)]));
        assert_eq!(e.len(), 2);
        assert!(matches!(e[1], Event::Strip { strip: 0, finger: 1, kind: Touch::Down, .. }));
    }

    /// A strip's timestamp changes on its own: that's no movement. (Resting strips were
    /// measured with it nonzero and the fingers 0.)
    #[test]
    fn the_timestamp_is_not_a_finger() {
        let mut d = Decoder::default();
        let mut r = strips(&[]);
        r[1] = 0x34;
        r[2] = 0x12;
        assert_eq!(run(&mut d, &r), []);
    }

    #[test]
    fn other_reports_and_short_ones_are_ignored() {
        let mut d = Decoder::default();
        assert_eq!(run(&mut d, &[0x40, 1, 2, 3]), []);
        assert_eq!(run(&mut d, &[0x01, 1, 2]), []);
        assert_eq!(run(&mut d, &[]), []);
    }
}
