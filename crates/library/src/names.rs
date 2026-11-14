//! What a file's name says about it.

/// The tempo a file's name gives (40–300 BPM), if it gives one.
pub fn tempo_in_name(name: &str) -> Option<f64> {
    let ok = |t: f64| (40.0..=300.0).contains(&t).then_some(t);
    // "[94]", "[94.5]"
    for part in name.split('[').skip(1) {
        if let Some((n, _)) = part.split_once(']')
            && let Some(t) = n.trim().parse::<f64>().ok().and_then(ok)
        {
            return Some(t);
        }
    }
    // "94bpm", "94 BPM", "94_bpm"
    let lower = name.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("bpm").map(|i| i + from) {
        let digits: String =
            lower[..i].trim_end_matches([' ', '_', '-']).chars().rev().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        let digits: String = digits.chars().rev().collect();
        if let Some(t) = digits.parse::<f64>().ok().and_then(ok) {
            return Some(t);
        }
        from = i + 3;
    }
    None
}

/// The note a file's name gives: its pitch class (0 = C … 11 = B) and its octave if
/// written (DECKARD's naming: C3 is MIDI note 60). The note stands alone between
/// separators ("Piano F#2.wav", "Bass_Db1_soft.wav", "Pad E-1.aif"); a lone letter
/// without an octave counts only in upper case ("Lead C.wav", not "a.wav"). The last one
/// in the name wins.
pub fn note_in_name(name: &str) -> Option<(u8, Option<i8>)> {
    let stem = match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    };
    let mut found = None;
    for token in stem.split([' ', '_', '.', '[', ']', '(', ')']) {
        // "E-1" is a note with a negative octave; "Keys-Bb2-Long" is three tokens.
        if let Some(n) = note_token(token) {
            found = Some(n);
        } else if let Some(n) = token.split('-').filter_map(note_token).next_back() {
            found = Some(n);
        }
    }
    found
}

/// One token as a note, if it is one.
fn note_token(t: &str) -> Option<(u8, Option<i8>)> {
    let mut chars = t.chars();
    let letter = chars.next()?;
    let class: i32 = match letter.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let mut rest: &str = chars.as_str();
    let mut shift = 0;
    for (sign, accs) in [(1, ["#", "♯"]), (-1, ["b", "♭"])] {
        for a in accs {
            if let Some(r) = rest.strip_prefix(a) {
                shift = sign;
                rest = r;
            }
        }
        if shift != 0 {
            break;
        }
    }
    let octave = if rest.is_empty() {
        if !letter.is_ascii_uppercase() {
            return None;
        }
        None
    } else {
        let o: i8 = rest.parse().ok().filter(|o| (-2..=8).contains(o) && !rest.starts_with('+'))?;
        Some(o)
    };
    Some(((class + shift).rem_euclid(12) as u8, octave))
}

#[cfg(test)]
mod tests {
    use super::{note_in_name, tempo_in_name};

    #[test]
    fn tempos_from_names() {
        assert_eq!(tempo_in_name("Bass[94] Amber.wav"), Some(94.0));
        assert_eq!(tempo_in_name("drums_174bpm_break.wav"), Some(174.0));
        assert_eq!(tempo_in_name("Kick [Hard].wav"), None);
    }

    #[test]
    fn notes_from_names() {
        assert_eq!(note_in_name("Piano F#2.wav"), Some((6, Some(2))));
        assert_eq!(note_in_name("Bass_Db1_soft.wav"), Some((1, Some(1))));
        assert_eq!(note_in_name("Lead C.wav"), Some((0, None)));
        assert_eq!(note_in_name("Kick 01.wav"), None);
        assert_eq!(note_in_name("Bass Amber.wav"), None);
        assert_eq!(note_in_name("Pad E-1.aif"), Some((4, Some(-1))));
        assert_eq!(note_in_name("Cello a3.wav"), Some((9, Some(3))));
        assert_eq!(note_in_name("drums.wav"), None);
        assert_eq!(note_in_name("Synth [120] G3.wav"), Some((7, Some(3))));
        assert_eq!(note_in_name("a.wav"), None);
        assert_eq!(note_in_name("Keys-Bb2-Long.wav"), Some((10, Some(2))));
    }
}
