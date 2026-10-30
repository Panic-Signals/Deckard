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

#[cfg(test)]
mod tests {
    use super::tempo_in_name;

    #[test]
    fn tempos_from_names() {
        assert_eq!(tempo_in_name("Bass[94] Amber.wav"), Some(94.0));
        assert_eq!(tempo_in_name("drums_174bpm_break.wav"), Some(174.0));
        assert_eq!(tempo_in_name("Kick [Hard].wav"), None);
    }
}
