//! The tuning table: what output value makes the voice sound each pitch.

/// log2 of 440 Hz.
const LOG2_A4: f32 = 8.781_359;
const MAGIC: &[u8; 4] = b"ZTB1";

/// Measured response of the voice. Points are `(output value, log2 hz)`, ascending in both.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub points: Vec<(f32, f32)>,
    /// When it was tuned, in seconds since the Unix epoch.
    pub tuned_at: u64,
    /// The worst error left after tuning, in cents.
    pub worst_cents: f32,
}

fn log2_hz(note: f32) -> f32 {
    LOG2_A4 + (note - 69.0) / 12.0
}

impl Table {
    /// The output value for MIDI `note` if the voice followed 1 V per octave exactly.
    pub fn nominal(note: f32) -> f32 {
        (note - 60.0) / 120.0
    }

    /// The output value that sounds `note`, and whether the note lay outside the table (and
    /// so was clamped to the nearest whole note inside it).
    pub fn value_for(&self, note: f32) -> (f32, bool) {
        if !note.is_finite() {
            return (Self::nominal(60.0), true);
        }
        let (Some(&first), Some(&last)) = (self.points.first(), self.points.last()) else {
            return (Self::nominal(note).clamp(-1.0, 1.0), true);
        };
        if self.points.len() < 2 {
            return (first.0, true);
        }
        let mut l = log2_hz(note);
        let outside = l < first.1 || l > last.1;
        if outside {
            // The nearest tuned note, unless no whole note lies inside the table at all.
            let (lo, hi) = self.range();
            l = match (l < first.1, lo <= hi) {
                (_, false) => l,
                (true, true) => log2_hz(lo as f32),
                (false, true) => log2_hz(hi as f32),
            };
        }
        if l <= first.1 {
            return (first.0, outside);
        }
        if l >= last.1 {
            return (last.0, outside);
        }
        let i = self.points.partition_point(|p| p.1 <= l).clamp(1, self.points.len() - 1);
        let (lo, hi) = (self.points[i - 1], self.points[i]);
        let span = hi.1 - lo.1;
        let t = if span > 0.0 { (l - lo.1) / span } else { 0.0 };
        (lo.0 + t * (hi.0 - lo.0), outside)
    }

    /// The lowest and highest whole MIDI notes the table covers.
    pub fn range(&self) -> (u8, u8) {
        let (Some(first), Some(last)) = (self.points.first(), self.points.last()) else {
            return (0, 0);
        };
        let note = |l: f32| 69.0 + 12.0 * (l - LOG2_A4);
        // Tolerance so a point measured at exactly a note's pitch counts for it.
        let lo = (note(first.1) - 1e-3).ceil().clamp(0.0, 127.0) as u8;
        let hi = (note(last.1) + 1e-3).floor().clamp(0.0, 127.0) as u8;
        (lo, hi)
    }

    /// Little-endian bytes: magic, `tuned_at`, `worst_cents`, point count, then the points.
    /// Nothing in them depends on the sample rate.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(20 + self.points.len() * 8);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.tuned_at.to_le_bytes());
        out.extend_from_slice(&self.worst_cents.to_le_bytes());
        out.extend_from_slice(&(self.points.len() as u32).to_le_bytes());
        for &(v, l) in &self.points {
            out.extend_from_slice(&v.to_le_bytes());
            out.extend_from_slice(&l.to_le_bytes());
        }
        out
    }

    /// The table `to_bytes` wrote, or `None` for anything else.
    pub fn from_bytes(bytes: &[u8]) -> Option<Table> {
        if bytes.len() < 20 || &bytes[..4] != MAGIC {
            return None;
        }
        let tuned_at = u64::from_le_bytes(bytes[4..12].try_into().ok()?);
        let worst_cents = f32::from_le_bytes(bytes[12..16].try_into().ok()?);
        let count = u32::from_le_bytes(bytes[16..20].try_into().ok()?) as usize;
        if bytes.len() != count.checked_mul(8)?.checked_add(20)? || !worst_cents.is_finite() {
            return None;
        }
        let points: Vec<(f32, f32)> = bytes[20..]
            .chunks_exact(8)
            .map(|c| (f32::from_le_bytes(c[..4].try_into().unwrap()), f32::from_le_bytes(c[4..].try_into().unwrap())))
            .collect();
        let sane = points.len() >= 2
            && points.iter().all(|p| p.0.is_finite() && p.1.is_finite())
            && points.windows(2).all(|w| w[0].0 <= w[1].0 && w[0].1 <= w[1].1);
        sane.then_some(Table { points, tuned_at, worst_cents })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bowed response: log2 hz as a function of output value.
    fn response(v: f32) -> f32 {
        let x = v - 0.5;
        LOG2_A4 + (x * 10.0) / 1.0 * 0.5 + 0.08 * (v * 3.0).sin() - 0.2
    }

    fn bowed() -> Table {
        let points = (0..=80).map(|i| i as f32 / 80.0).map(|v| (v, response(v))).collect();
        Table { points, tuned_at: 7, worst_cents: 0.5 }
    }

    #[test]
    fn value_for_inverts_measured_points_within_a_cent() {
        let t = bowed();
        let (lo, hi) = t.range();
        assert!(hi - lo > 40);
        for note in lo..=hi {
            let (v, clamped) = t.value_for(note as f32);
            assert!(!clamped);
            let cents = 1200.0 * (response(v) - log2_hz(note as f32));
            assert!(cents.abs() < 1.0, "note {note}: {cents} cents");
        }
    }

    /// Past either end, a note plays the nearest whole note the table covers, flagged.
    #[test]
    fn notes_outside_the_table_clamp_to_the_nearest_tuned_note_and_flag() {
        let t = bowed();
        let (lo, hi) = t.range();
        assert_eq!(t.value_for(200.0), (t.value_for(hi as f32).0, true));
        assert_eq!(t.value_for(hi as f32 + 1.0), (t.value_for(hi as f32).0, true));
        assert_eq!(t.value_for(-20.0), (t.value_for(lo as f32).0, true));
        assert!(!t.value_for(hi as f32).1 && !t.value_for(lo as f32).1);
        // Not the table's ends, which lie between notes.
        assert!(t.value_for(hi as f32).0 < 1.0 && t.value_for(lo as f32).0 > 0.0);
        assert_eq!(Table::nominal(60.0), 0.0);
    }

    #[test]
    fn value_for_never_panics_on_odd_tables_or_notes() {
        let t = bowed();
        assert_eq!(t.value_for(f32::NAN), (Table::nominal(60.0), true));
        assert_eq!(t.value_for(f32::INFINITY), (Table::nominal(60.0), true));
        let one = Table { points: vec![(0.25, 8.0)], tuned_at: 0, worst_cents: 0.0 };
        assert_eq!(one.value_for(60.0), (0.25, true));
        assert_eq!(one.value_for(f32::NAN), (Table::nominal(60.0), true));
        let none = Table { points: vec![], tuned_at: 0, worst_cents: 0.0 };
        assert_eq!(none.value_for(72.0), (Table::nominal(72.0), true));
        assert_eq!(none.value_for(f32::NEG_INFINITY), (Table::nominal(60.0), true));
        assert_eq!(none.value_for(1e30), (1.0, true));
    }

    #[test]
    fn a_saved_table_with_fewer_than_two_points_reads_none() {
        for points in [vec![], vec![(0.5, 8.0)]] {
            let t = Table { points, tuned_at: 1, worst_cents: 0.0 };
            assert_eq!(Table::from_bytes(&t.to_bytes()), None);
        }
    }

    #[test]
    fn table_round_trips_as_bytes_and_bad_bytes_read_none() {
        let t = bowed();
        let bytes = t.to_bytes();
        assert_eq!(Table::from_bytes(&bytes), Some(t));
        assert_eq!(Table::from_bytes(&[]), None);
        assert_eq!(Table::from_bytes(&bytes[..bytes.len() - 1]), None);
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert_eq!(Table::from_bytes(&bad), None);
        let mut nan = bytes.clone();
        nan[20..24].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(Table::from_bytes(&nan), None);
    }
}
