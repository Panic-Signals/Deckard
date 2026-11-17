//! Which pictures sit over which knobs: the plug-in mappings spec's `generic` template
//! ("Templates", "Laying out pictures on any page"), for any page of knobs.
//!
//! Common runs of adjacent knob names get a picture spanning those knobs, in the
//! picture band directly above them:
//!
//! | Knobs, adjacent, in one section | Picture |
//! |---|---|
//! | a frequency, a gain and a Q | `band` |
//! | a cutoff next to a resonance (a drive beside them joins) | `filter` |
//! | attack, decay, sustain, release in order, or attack and decay, or attack and release | `envelope` |
//! | a threshold next to a ratio (a knee, a makeup beside them join) | `transfer` |
//! | an LFO's rate next to its depth or shape | `lfo` |
//!
//! Values in real units come from each knob's own text ("1.20 kHz", "-6.0 dB",
//! "380 ms"). A knob whose text doesn't give the unit a picture needs leaves that
//! picture out: nothing is guessed. Levels without units (a resonance, a sustain in
//! percent, a depth) come from the knob's position.
//!
//! A span is two to four knob cells on one screen, never across the gap between knobs
//! 4 and 5. A run split over the gap gets its picture on the screen holding its first
//! input, if that screen has room; the knob on the other screen still lights it on
//! touch. Picked spans don't overlap: more knobs bound win, then the leftmost.

/// A knob as the page shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Knob {
    pub name: String,
    pub section: String,
    pub text: String,
    /// 0..1.
    pub value: f32,
}

/// An input a picture is drawn from.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Num(f32),
    Text(String),
}

/// A picture over knobs `first .. first + cells` (0..8, all on one screen).
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub first: usize,
    pub cells: usize,
    pub kind: &'static str,
    pub inputs: Vec<(&'static str, Input)>,
    /// Each bound knob and the input it moves (for touch focus).
    pub knobs: Vec<(usize, &'static str)>,
}

impl Span {
    /// The input lit by touching knob `k`, if this picture draws it.
    pub fn focus_of(&self, k: usize) -> Option<&'static str> {
        self.knobs.iter().find(|(i, _)| *i == k).map(|(_, n)| *n)
    }
}

/// A number and the word after it: "1.20 kHz" -> (1.2, "khz"), "-inf dB" -> (-inf, "db"),
/// "4.0:1" -> (4.0, ":1").
pub fn number_unit(text: &str) -> Option<(f32, String)> {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("-inf") {
        return Some((f32::NEG_INFINITY, rest.trim().to_string()));
    }
    let end = t
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || ((c == '-' || c == '+') && i == 0)))
        .map_or(t.len(), |(i, _)| i);
    let v: f32 = t[..end].parse().ok()?;
    Some((v, t[end..].trim().to_ascii_lowercase()))
}

/// A frequency in Hz from text in Hz or kHz.
pub fn hz(text: &str) -> Option<f32> {
    let (v, u) = number_unit(text)?;
    match u.as_str() {
        "hz" => Some(v),
        "khz" | "k" => Some(v * 1000.0),
        _ => None,
    }
}

/// A time in ms from text in ms or s.
pub fn ms(text: &str) -> Option<f32> {
    let (v, u) = number_unit(text)?;
    match u.as_str() {
        "ms" => Some(v),
        "s" | "sec" => Some(v * 1000.0),
        "us" | "µs" => Some(v / 1000.0),
        _ => None,
    }
}

/// A level in dB from text in dB.
pub fn db(text: &str) -> Option<f32> {
    let (v, u) = number_unit(text)?;
    (u == "db").then_some(v.max(-200.0))
}

/// A ratio from "4.0:1", "4:1", "4.0", or infinite ("inf", "∞").
pub fn ratio(text: &str) -> Option<f32> {
    let t = text.trim().to_ascii_lowercase();
    if t.starts_with("inf") || t.starts_with('∞') {
        return Some(100.0);
    }
    let (v, u) = number_unit(&t)?;
    (u.is_empty() || u.starts_with(':')).then_some(v)
}

/// A 0..1 level: from percent or dB text, else the knob's position.
fn level(k: &Knob) -> f32 {
    match number_unit(&k.text) {
        Some((v, u)) if u == "%" => (v / 100.0).clamp(0.0, 1.0),
        Some((v, u)) if u == "db" => 10f32.powf(v / 20.0).clamp(0.0, 1.0),
        _ => k.value.clamp(0.0, 1.0),
    }
}

/// The role a knob's name plays, if it's one the patterns know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Cutoff,
    Reso,
    Drive,
    Freq,
    Gain,
    Q,
    Attack,
    Decay,
    Sustain,
    Release,
    Threshold,
    Ratio,
    Knee,
    Makeup,
    Rate,
    Depth,
    Shape,
}

fn role(name: &str) -> Option<Role> {
    let n = name.trim().to_ascii_lowercase();
    // (The last word decides: "Filter Cutoff", "Amp Attack", "LFO Rate".)
    let last = n.rsplit([' ', '_', '-', '.']).find(|w| !w.is_empty()).unwrap_or(&n);
    let any = |words: &[&str]| words.contains(&n.as_str()) || words.contains(&last);
    Some(if any(&["cutoff", "cut", "cutoff freq"]) {
        Role::Cutoff
    } else if any(&["resonance", "reso", "res", "emphasis", "peak"]) {
        Role::Reso
    } else if any(&["drive", "overdrive"]) {
        Role::Drive
    } else if any(&["freq", "frequency"]) {
        Role::Freq
    } else if any(&["gain", "boost"]) {
        Role::Gain
    } else if any(&["q", "bandwidth", "bw", "width"]) && n != "stereo width" {
        Role::Q
    } else if any(&["attack", "att", "atk"]) {
        Role::Attack
    } else if any(&["decay", "dec"]) {
        Role::Decay
    } else if any(&["sustain", "sus"]) {
        Role::Sustain
    } else if any(&["release", "rel"]) {
        Role::Release
    } else if any(&["threshold", "thresh", "thr"]) {
        Role::Threshold
    } else if any(&["ratio"]) {
        Role::Ratio
    } else if any(&["knee"]) {
        Role::Knee
    } else if any(&["makeup", "make-up", "make up", "output"]) {
        Role::Makeup
    } else if any(&["rate", "speed"]) {
        Role::Rate
    } else if any(&["depth", "amount", "amt"]) {
        Role::Depth
    } else if any(&["shape", "wave", "waveform"]) {
        Role::Shape
    } else {
        return None;
    })
}

/// A run found on the page: its picture, inputs and knobs.
struct Found {
    kind: &'static str,
    min_cells: usize,
    inputs: Vec<(&'static str, Input)>,
    knobs: Vec<(usize, &'static str)>,
}

/// The knobs at `start..` whose roles are `want` in order, all present, adjacent and in
/// one section.
fn run(knobs: &[Option<Knob>; 8], roles: &[Option<Role>; 8], start: usize, want: &[Role]) -> bool {
    if start + want.len() > 8 {
        return false;
    }
    let section = knobs[start].as_ref().map(|k| k.section.as_str());
    want.iter().enumerate().all(|(i, r)| roles[start + i] == Some(*r) && knobs[start + i].as_ref().map(|k| k.section.as_str()) == section)
}

fn knob(knobs: &[Option<Knob>; 8], i: usize) -> &Knob {
    knobs[i].as_ref().expect("a knob with a role")
}

/// Every run of knobs a pattern recognises.
fn find(knobs: &[Option<Knob>; 8]) -> Vec<Found> {
    let roles: [Option<Role>; 8] = std::array::from_fn(|i| knobs[i].as_ref().and_then(|k| role(&k.name)));
    let same_section = |a: usize, b: usize| knobs[a].as_ref().map(|k| &k.section) == knobs[b].as_ref().map(|k| &k.section);
    let mut out = Vec::new();
    for s in 0..8 {
        // A band: frequency, gain and Q.
        if run(knobs, &roles, s, &[Role::Freq, Role::Gain, Role::Q])
            && let (Some(f), Some(g)) = (hz(&knob(knobs, s).text), db(&knob(knobs, s + 1).text))
        {
            let q = number_unit(&knob(knobs, s + 2).text).map_or(0.707, |(v, _)| v.clamp(0.1, 40.0));
            out.push(Found {
                kind: "band",
                min_cells: 2,
                inputs: vec![("freq", Input::Num(f)), ("gain", Input::Num(g)), ("q", Input::Num(q))],
                knobs: vec![(s, "freq"), (s + 1, "gain"), (s + 2, "q")],
            });
        }
        // A filter: a cutoff (or a frequency) and a resonance, either way round, and a
        // drive beside them.
        for (c, r) in [(s, s + 1), (s + 1, s)] {
            if s + 1 >= 8 || !same_section(s, s + 1) {
                continue;
            }
            let cut = matches!(roles[c], Some(Role::Cutoff | Role::Freq));
            if !cut || roles[r] != Some(Role::Reso) {
                continue;
            }
            let Some(f) = hz(&knob(knobs, c).text) else { continue };
            let mut inputs = vec![("cutoff", Input::Num(f)), ("resonance", Input::Num(knob(knobs, r).value))];
            let mut bound = vec![(c, "cutoff"), (r, "resonance")];
            let after = s + 2;
            if after < 8 && roles[after] == Some(Role::Drive) && same_section(s, after) {
                if let Some(d) = db(&knob(knobs, after).text) {
                    inputs.push(("drive", Input::Num(d)));
                }
                bound.push((after, "drive"));
            }
            out.push(Found { kind: "filter", min_cells: 2, inputs, knobs: bound });
        }
        // An envelope: ADSR, AD or AR.
        for (want, names) in [
            (&[Role::Attack, Role::Decay, Role::Sustain, Role::Release][..], &["attack", "decay", "sustain", "release"][..]),
            (&[Role::Attack, Role::Decay][..], &["attack", "decay"][..]),
            (&[Role::Attack, Role::Release][..], &["attack", "release"][..]),
        ] {
            if !run(knobs, &roles, s, want) {
                continue;
            }
            let mut inputs = Vec::new();
            let mut ok = true;
            for (i, name) in names.iter().enumerate() {
                let k = knob(knobs, s + i);
                if *name == "sustain" {
                    inputs.push(("sustain", Input::Num(level(k))));
                } else if let Some(t) = ms(&k.text) {
                    inputs.push((*name, Input::Num(t)));
                } else {
                    ok = false;
                }
            }
            if !ok {
                continue;
            }
            if want.len() == 2 && want[1] == Role::Release {
                // (An AR envelope: no decay, held at full until released.)
                inputs.push(("decay", Input::Num(0.0)));
                inputs.push(("sustain", Input::Num(1.0)));
            }
            let bound = names.iter().enumerate().map(|(i, n)| (s + i, *n)).collect();
            out.push(Found { kind: "envelope", min_cells: 2, inputs, knobs: bound });
            break;
        }
        // A compressor: threshold and ratio, then a knee and a makeup beside them.
        if run(knobs, &roles, s, &[Role::Threshold, Role::Ratio])
            && let (Some(t), Some(r)) = (db(&knob(knobs, s).text), ratio(&knob(knobs, s + 1).text))
        {
            let mut inputs = vec![("threshold", Input::Num(t)), ("ratio", Input::Num(r.max(1.0)))];
            let mut bound = vec![(s, "threshold"), (s + 1, "ratio")];
            for i in s + 2..(s + 4).min(8) {
                let name = match roles[i] {
                    Some(Role::Knee) if same_section(s, i) => "knee",
                    Some(Role::Makeup) if same_section(s, i) => "makeup",
                    _ => break,
                };
                if let Some(v) = db(&knob(knobs, i).text) {
                    inputs.push((name, Input::Num(v)));
                }
                bound.push((i, name));
            }
            out.push(Found { kind: "transfer", min_cells: 2, inputs, knobs: bound });
        }
        // An LFO: its rate next to its depth or shape.
        if s + 1 < 8 && roles[s] == Some(Role::Rate) && matches!(roles[s + 1], Some(Role::Depth | Role::Shape)) && same_section(s, s + 1) {
            let rate = knob(knobs, s);
            let mut inputs = vec![("rate_text", Input::Text(rate.text.clone()))];
            let other = knob(knobs, s + 1);
            if roles[s + 1] == Some(Role::Depth) {
                inputs.push(("depth", Input::Num(level(other))));
                out.push(Found { kind: "lfo", min_cells: 2, inputs, knobs: vec![(s, "rate"), (s + 1, "depth")] });
            } else {
                inputs.push(("shape", Input::Text(other.text.to_ascii_lowercase())));
                out.push(Found { kind: "lfo", min_cells: 2, inputs, knobs: vec![(s, "rate"), (s + 1, "shape")] });
            }
        }
    }
    out
}

/// The span a run's picture takes: its knobs on the screen of its first input,
/// widened to `min_cells` inside that screen. `None` if that screen has no room.
fn span_of(f: &Found) -> Option<(usize, usize)> {
    let first = f.knobs[0].0;
    let screen = first / 4;
    let on: Vec<usize> = f.knobs.iter().map(|(k, _)| *k).filter(|k| k / 4 == screen).collect();
    let (mut lo, mut hi) = (*on.iter().min()?, *on.iter().max()?);
    while hi - lo + 1 < f.min_cells {
        if hi + 1 < screen * 4 + 4 {
            hi += 1;
        } else if lo > screen * 4 {
            lo -= 1;
        } else {
            return None;
        }
    }
    Some((lo, hi - lo + 1))
}

/// The pictures for a page of eight knobs (`None` where the page has no knob).
pub fn layout(knobs: &[Option<Knob>; 8]) -> Vec<Span> {
    let mut found: Vec<(Found, usize, usize)> = find(knobs).into_iter().filter_map(|f| span_of(&f).map(|(a, n)| (f, a, n))).collect();
    // More knobs bound first, then the leftmost.
    found.sort_by_key(|(f, first, _)| (std::cmp::Reverse(f.knobs.len()), *first));
    let mut taken = [false; 8];
    let mut used = [false; 8];
    let mut out = Vec::new();
    for (f, first, cells) in found {
        if (first..first + cells).any(|k| taken[k]) || f.knobs.iter().any(|(k, _)| used[*k]) {
            continue;
        }
        for k in first..first + cells {
            taken[k] = true;
        }
        for (k, _) in &f.knobs {
            used[*k] = true;
        }
        out.push(Span { first, cells, kind: f.kind, inputs: f.inputs, knobs: f.knobs });
    }
    out.sort_by_key(|s| s.first);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(knobs: &[(&str, &str)]) -> [Option<Knob>; 8] {
        std::array::from_fn(|i| {
            knobs.get(i).filter(|(n, _)| !n.is_empty()).map(|(n, t)| Knob {
                name: n.to_string(),
                section: String::new(),
                text: t.to_string(),
                value: 0.5,
            })
        })
    }

    #[test]
    fn units_come_from_the_knobs_text() {
        assert_eq!(hz("1.20 kHz"), Some(1200.0));
        assert_eq!(hz("80 Hz"), Some(80.0));
        assert_eq!(hz("42 %"), None);
        assert_eq!(ms("380 ms"), Some(380.0));
        assert_eq!(ms("1.2 s"), Some(1200.0));
        assert_eq!(db("-6.0 dB"), Some(-6.0));
        assert_eq!(db("-inf dB"), Some(-200.0));
        assert_eq!(ratio("4.0:1"), Some(4.0));
        assert_eq!(ratio("2.5"), Some(2.5));
        assert_eq!(ratio("Inf:1"), Some(100.0));
    }

    #[test]
    fn a_cutoff_beside_a_resonance_gets_a_filter() {
        let spans = layout(&page(&[("Cutoff", "1.20 kHz"), ("Reso", "42 %"), ("Drive", "3.0 dB"), ("Mode", "LP24")]));
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].kind, spans[0].first, spans[0].cells), ("filter", 0, 3));
        assert_eq!(spans[0].focus_of(1), Some("resonance"));
        assert_eq!(spans[0].focus_of(3), None);
    }

    #[test]
    fn a_page_mixing_filter_and_envelope_gets_both() {
        let spans = layout(&page(&[("Cutoff", "1.20 kHz"), ("Resonance", "42 %"), ("Attack", "4 ms"), ("Release", "380 ms")]));
        let kinds: Vec<_> = spans.iter().map(|s| (s.kind, s.first, s.cells)).collect();
        assert_eq!(kinds, [("filter", 0, 2), ("envelope", 2, 2)]);
    }

    #[test]
    fn nothing_is_guessed() {
        assert!(layout(&page(&[("Cutoff", "64 %"), ("Resonance", "42 %")])).is_empty(), "a cutoff without Hz");
        assert!(layout(&page(&[("Attack", "12"), ("Decay", "30")])).is_empty(), "times without ms");
    }

    #[test]
    fn a_pair_across_the_gap_goes_to_the_cutoffs_screen() {
        let spans = layout(&page(&[("", ""), ("", ""), ("", ""), ("Cutoff", "1.20 kHz"), ("Reso", "42 %")]));
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].first, spans[0].cells), (2, 2), "knobs 3-4, on the left screen");
        assert_eq!(spans[0].focus_of(4), Some("resonance"), "the knob over the gap still lights it");
    }

    #[test]
    fn adsr_wins_over_ad_and_sections_keep_envelopes_apart() {
        let spans = layout(&page(&[("Attack", "4 ms"), ("Decay", "200 ms"), ("Sustain", "60 %"), ("Release", "1.2 s")]));
        assert_eq!((spans[0].kind, spans[0].cells), ("envelope", 4));
        let mut p = page(&[("Attack", "4 ms"), ("Decay", "200 ms")]);
        p[1].as_mut().unwrap().section = "Amp Env".into();
        assert!(layout(&p).is_empty());
    }

    #[test]
    fn a_band_and_a_compressor() {
        let spans = layout(&page(&[
            ("Freq", "800 Hz"),
            ("Gain", "-3.0 dB"),
            ("Q", "1.4"),
            ("", ""),
            ("Threshold", "-18 dB"),
            ("Ratio", "4.0:1"),
            ("Knee", "6 dB"),
            ("Makeup", "2.0 dB"),
        ]));
        let kinds: Vec<_> = spans.iter().map(|s| (s.kind, s.first, s.cells)).collect();
        assert_eq!(kinds, [("band", 0, 3), ("transfer", 4, 4)]);
    }
}
