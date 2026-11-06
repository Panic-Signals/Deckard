//! A plug-in's extra outputs (docs/superpowers/specs/2026-11-05-plugin-extra-outputs-design.md):
//! each output pair beyond a plug-in's first that has played gets a mixer strip with its
//! own level, pan, mute and destination. DECKARD's own: kept on the project as
//! [`crate::Project::extra_outs`] and saved in the DECKARD block as [`to_text`]'s text.

use crate::{AudioDest, PluginRef, Project};

/// The field on the project's root object holding [`to_text`]'s text (DECKARD's own, so
/// undo carries it and it's saved only in the DECKARD block).
pub const FIELD: &str = "deckard:extra_outs";

/// Which plug-in slot a strip belongs to. Mirrors `deckard_plugins::Slot`, and indexes
/// the same way: `Sound` is a sound's instrument (`Sound::plugin`); `Chain`'s slot is the
/// position in the sound's `slots` (instrument first, so effects start at 1); `Group`'s
/// slot is the position in the group's `slots`; `Master`'s in `master_slots`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    /// (group, sound)
    Sound(usize, usize),
    /// (group, sound, slot)
    Chain(usize, usize, usize),
    /// (group, slot)
    Group(usize, usize),
    /// (slot)
    Master(usize),
}

/// What a strip takes from its pair: both sides (`Stereo`), their sum (`Mono`), or one
/// side (`Left`, `Right`: the halves of a split strip, each a mono strip of its own).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Channels {
    #[default]
    Stereo,
    Mono,
    Left,
    Right,
}

impl Channels {
    fn text(self) -> &'static str {
        match self {
            Channels::Stereo => "stereo",
            Channels::Mono => "mono",
            Channels::Left => "left",
            Channels::Right => "right",
        }
    }

    fn from(t: &str) -> Option<Channels> {
        [Channels::Stereo, Channels::Mono, Channels::Left, Channels::Right].into_iter().find(|c| c.text() == t)
    }

    /// One half of a split strip.
    pub fn is_half(self) -> bool {
        matches!(self, Channels::Left | Channels::Right)
    }
}

/// One extra output: output pair `bus` of the plug-in `plugin` (its `PluginRef::id`) in
/// `owner`'s slot, which the plug-in calls `name` (see [`strip_label`]). `level` is linear
/// (1.0 = 0 dB), `pan` -1..1.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtraOut {
    pub owner: Owner,
    pub bus: usize,
    pub plugin: String,
    pub name: String,
    pub level: f64,
    pub pan: f64,
    pub muted: bool,
    pub dest: AudioDest,
    pub channels: Channels,
}

/// Split strip `i` (stereo or mono) into two mono strips in its place, `Left` then
/// `Right`, each with its settings. Anything else: nothing.
pub fn split(v: &mut Vec<ExtraOut>, i: usize) {
    let Some(x) = v.get_mut(i).filter(|x| !x.channels.is_half()) else { return };
    x.channels = Channels::Left;
    let right = ExtraOut { channels: Channels::Right, ..x.clone() };
    v.insert(i + 1, right);
}

/// Join split half `i` with its partner (the other half of the same owner, pair and
/// plug-in) into one stereo strip with the `Left` half's settings, where that half was;
/// a half whose partner is missing becomes stereo with its own. Anything else: nothing.
pub fn join(v: &mut Vec<ExtraOut>, i: usize) {
    let Some(x) = v.get(i).filter(|x| x.channels.is_half()) else { return };
    let other = if x.channels == Channels::Left { Channels::Right } else { Channels::Left };
    let partner = v.iter().position(|y| y.owner == x.owner && y.bus == x.bus && y.plugin == x.plugin && y.channels == other);
    let keep = match partner {
        Some(p) => {
            let keep = if x.channels == Channels::Left { i } else { p };
            let drop = if keep == i { p } else { i };
            v.remove(drop);
            if drop < keep { keep - 1 } else { keep }
        }
        None => i,
    };
    v[keep].channels = Channels::Stereo;
}

/// A strip's label: "Out N" for pair `bus` (from 0), then the plug-in's name for it
/// (`plugin_bus_name`) when that says more than a number would, then "L" or "R" for a
/// split half. Generic names, whose every word is a number, a side or one of a few words
/// for an output ("Aux 1+2", "Output 3/4", "Stereo Out 2", "Main Out"...), are left out, so no
/// strip is ever labelled "Aux", which in Maschine means the send effects.
pub fn strip_label(bus: usize, plugin_bus_name: &str, ch: Channels) -> String {
    const GENERIC: [&str; 10] = ["aux", "out", "output", "outs", "bus", "st", "stereo", "main", "ch", "channel"];
    // Its words, lower case, without digits; a lone "l" or "r" (a side) says nothing.
    let lower = plugin_bus_name.to_lowercase();
    let generic = lower
        .split(|c: char| c.is_whitespace() || "./-_:#()+&,".contains(c))
        .map(|w| w.chars().filter(|c| !c.is_ascii_digit()).collect::<String>())
        .filter(|w| !w.is_empty() && w != "l" && w != "r")
        .all(|w| GENERIC.contains(&w.as_str()));
    let mut s = format!("Out {}", bus + 1);
    if !generic {
        s.push(' ');
        s.push_str(plugin_bus_name.trim());
    }
    match ch {
        Channels::Left => s.push_str(" L"),
        Channels::Right => s.push_str(" R"),
        Channels::Stereo | Channels::Mono => {}
    }
    s
}

/// The plug-in `o`'s slot holds, if any.
pub fn plugin_at(p: &Project, o: Owner) -> Option<&PluginRef> {
    match o {
        Owner::Sound(g, s) => p.groups.get(g)?.sounds.get(s)?.plugin.as_ref(),
        Owner::Chain(g, s, k) => p.groups.get(g)?.sounds.get(s)?.slots.get(k)?.plugin.as_ref(),
        Owner::Group(g, k) => p.groups.get(g)?.slots.get(k)?.plugin.as_ref(),
        Owner::Master(k) => p.master_slots.get(k)?.plugin.as_ref(),
    }
}

/// Drop the strips whose slot now holds a different plug-in, or none. A strip stays
/// while the slot names the same plug-in id, even if that plug-in failed to load (the
/// model can't see that).
pub fn prune(p: &mut Project) {
    let outs = std::mem::take(&mut p.extra_outs);
    p.extra_outs = outs.into_iter().filter(|e| plugin_at(p, e.owner).is_some_and(|pr| pr.id == e.plugin)).collect();
}

fn owner_text(o: Owner) -> String {
    match o {
        Owner::Sound(g, s) => format!("sound:{g}:{s}"),
        Owner::Chain(g, s, k) => format!("chain:{g}:{s}:{k}"),
        Owner::Group(g, k) => format!("group:{g}:{k}"),
        Owner::Master(k) => format!("master:{k}"),
    }
}

fn owner_from(t: &str) -> Option<Owner> {
    let mut w = t.split(':');
    let kind = w.next()?;
    let n: Vec<usize> = w.map(|v| v.parse().ok()).collect::<Option<_>>()?;
    match (kind, n.as_slice()) {
        ("sound", &[g, s]) => Some(Owner::Sound(g, s)),
        ("chain", &[g, s, k]) => Some(Owner::Chain(g, s, k)),
        ("group", &[g, k]) => Some(Owner::Group(g, k)),
        ("master", &[k]) => Some(Owner::Master(k)),
        _ => None,
    }
}

fn dest_text(d: AudioDest) -> String {
    match d {
        AudioDest::None => "none".into(),
        AudioDest::Parent => "parent".into(),
        AudioDest::Master => "master".into(),
        AudioDest::Ext(n) => format!("ext:{n}"),
        AudioDest::Group(g) => format!("group:{g}"),
        AudioDest::Sound(g, s) => format!("sound:{g}:{s}"),
    }
}

fn dest_from(t: &str) -> Option<AudioDest> {
    let mut w = t.split(':');
    let kind = w.next()?;
    let n: Vec<&str> = w.collect();
    match (kind, n.as_slice()) {
        ("none", []) => Some(AudioDest::None),
        ("parent", []) => Some(AudioDest::Parent),
        ("master", []) => Some(AudioDest::Master),
        ("ext", [n]) => Some(AudioDest::Ext(n.parse().ok()?)),
        ("group", [g]) => Some(AudioDest::Group(g.parse().ok()?)),
        ("sound", [g, s]) => Some(AudioDest::Sound(g.parse().ok()?, s.parse().ok()?)),
        _ => None,
    }
}

/// Text that has no whitespace: `%`, whitespace, control and non-ASCII bytes as `%XX`.
fn escape(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b == b'%' || b <= b' ' || b >= 0x7f {
            o.push_str(&format!("%{b:02X}"));
        } else {
            o.push(b as char);
        }
    }
    o
}

fn unescape(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut o = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = s.get(i + 1..i + 3)?;
            o.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            o.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(o).ok()
}

/// The strips as text, one per line:
/// `owner bus level pan dest [muted] [ch=mono|left|right] plugin=ID name=NAME` (ID and
/// NAME with whitespace and `%` written as `%XX`; no `ch=` for a stereo strip, so text
/// from before strips had channels reads the same).
pub fn to_text(v: &[ExtraOut]) -> String {
    let mut s = String::new();
    for e in v {
        s.push_str(&format!(
            "{} {} {} {} {}{}{} plugin={} name={}\n",
            owner_text(e.owner),
            e.bus,
            e.level,
            e.pan,
            dest_text(e.dest),
            if e.muted { " muted" } else { "" },
            if e.channels == Channels::Stereo { String::new() } else { format!(" ch={}", e.channels.text()) },
            escape(&e.plugin),
            escape(&e.name),
        ));
    }
    s
}

/// Strips from [`to_text`]'s text. Lines that don't read are skipped, and unknown
/// `key=` words (and `ch=` values) ignored.
pub fn from_text(s: &str) -> Vec<ExtraOut> {
    s.lines().filter_map(line).collect()
}

fn line(l: &str) -> Option<ExtraOut> {
    let mut w = l.split_whitespace();
    let owner = owner_from(w.next()?)?;
    let bus: usize = w.next()?.parse().ok()?;
    let level: f64 = w.next()?.parse().ok().filter(|v: &f64| v.is_finite())?;
    let pan: f64 = w.next()?.parse().ok().filter(|v: &f64| v.is_finite())?;
    let dest = dest_from(w.next()?)?;
    let mut e =
        ExtraOut { owner, bus, plugin: String::new(), name: String::new(), level, pan, muted: false, dest, channels: Channels::Stereo };
    for word in w {
        if word == "muted" {
            e.muted = true;
        } else if let Some(v) = word.strip_prefix("ch=") {
            // (One this version doesn't know: stereo.)
            e.channels = Channels::from(v).unwrap_or_default();
        } else if let Some(v) = word.strip_prefix("plugin=") {
            e.plugin = unescape(v)?;
        } else if let Some(v) = word.strip_prefix("name=") {
            e.name = unescape(v)?;
        }
    }
    Some(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Group, SlotInfo, Sound};

    fn out(owner: Owner, plugin: &str, dest: AudioDest) -> ExtraOut {
        ExtraOut {
            owner,
            bus: 1,
            plugin: plugin.into(),
            name: "Out 2".into(),
            level: 0.5,
            pan: -0.25,
            muted: false,
            dest,
            channels: Channels::Stereo,
        }
    }

    fn pr(id: &str) -> Option<PluginRef> {
        Some(PluginRef { id: id.into(), ..Default::default() })
    }

    fn slot(id: &str) -> SlotInfo {
        SlotInfo { module: "PluginHost".into(), plugin: pr(id), ..Default::default() }
    }

    /// A sound with instrument "inst" and an effect "fx" at slot 1; a group effect "gfx"
    /// at slot 0; a master effect "mfx" at slot 0.
    fn project() -> Project {
        let sound = Sound { plugin: pr("inst"), slots: vec![slot("inst"), slot("fx")], ..Default::default() };
        Project {
            groups: vec![Group { sounds: vec![sound], slots: vec![slot("gfx")], ..Default::default() }],
            master_slots: vec![slot("mfx")],
            ..Default::default()
        }
    }

    #[test]
    fn extra_outs_round_trip_as_text() {
        let owners = [Owner::Sound(0, 3), Owner::Chain(1, 2, 3), Owner::Group(4, 5), Owner::Master(6)];
        let dests = [AudioDest::None, AudioDest::Parent, AudioDest::Master, AudioDest::Ext(2), AudioDest::Group(3), AudioDest::Sound(1, 7)];
        let mut v = Vec::new();
        for (i, o) in owners.into_iter().enumerate() {
            for (j, d) in dests.into_iter().enumerate() {
                let mut e = out(o, "Native Instruments:VST3:INSTR:56 53%é", d);
                e.bus = i + j + 1;
                e.name = format!("Kick & 100% — {i} {j}");
                e.level = 0.1 + j as f64 / 3.0;
                e.muted = (i + j) % 2 == 0;
                v.push(e);
            }
        }
        let text = to_text(&v);
        assert_eq!(text.lines().count(), 24);
        assert_eq!(from_text(&text), v);
        assert_eq!(to_text(&[]), "");
    }

    #[test]
    fn prune_drops_strips_of_a_replaced_or_removed_plugin() {
        let mut p = project();
        p.extra_outs = vec![
            out(Owner::Sound(0, 0), "inst", AudioDest::Parent),
            out(Owner::Chain(0, 0, 1), "fx", AudioDest::Parent),
            out(Owner::Group(0, 0), "gfx", AudioDest::Master),
            out(Owner::Master(0), "mfx", AudioDest::Master),
        ];
        p.groups[0].sounds[0].plugin = pr("other"); // replaced
        p.groups[0].sounds[0].slots.truncate(1); // fx removed
        p.groups[0].slots.clear(); // removed
        p.master_slots[0].plugin = None; // no plug-in
        prune(&mut p);
        assert!(p.extra_outs.is_empty());
        // A missing sound or group goes too.
        let mut p = project();
        p.extra_outs = vec![out(Owner::Sound(3, 0), "inst", AudioDest::Parent)];
        prune(&mut p);
        assert!(p.extra_outs.is_empty());
    }

    #[test]
    fn prune_keeps_strips_whose_plugin_id_is_unchanged() {
        let mut p = project();
        p.extra_outs = vec![
            out(Owner::Sound(0, 0), "inst", AudioDest::Parent),
            out(Owner::Chain(0, 0, 1), "fx", AudioDest::Parent),
            out(Owner::Group(0, 0), "gfx", AudioDest::Master),
            out(Owner::Master(0), "mfx", AudioDest::Master),
            out(Owner::Master(0), "mfx", AudioDest::Ext(1)),
        ];
        let before = p.extra_outs.clone();
        prune(&mut p);
        assert_eq!(p.extra_outs, before);
        assert_eq!(plugin_at(&p, Owner::Chain(0, 0, 1)).map(|r| r.id.as_str()), Some("fx"));
        assert!(plugin_at(&p, Owner::Master(1)).is_none());
    }

    #[test]
    fn bad_lines_are_skipped() {
        let text = "garbage\n\
                    sound:0:1 1 0.5 0 parent plugin=a name=B\n\
                    sound:0 1 0.5 0 parent plugin=a name=B\n\
                    sound:0:1 x 0.5 0 parent plugin=a name=B\n\
                    sound:0:1 1 nan 0 parent plugin=a name=B\n\
                    sound:0:1 1 0.5 0 nowhere plugin=a name=B\n\
                    master:2 3 1 0.5 ext:1 muted future=9 plugin=m%20x name=C\n\
                    \n";
        let v = from_text(text);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].owner, Owner::Sound(0, 1));
        assert_eq!(v[1].owner, Owner::Master(2));
        assert_eq!(v[1].plugin, "m x");
        assert!(v[1].muted);
        assert_eq!(v[1].dest, AudioDest::Ext(1));
    }

    #[test]
    fn channels_round_trip_and_default_to_stereo() {
        // Old text, with no `ch=`: stereo.
        let v = from_text("sound:0:1 1 0.5 0 parent plugin=a name=B\n");
        assert_eq!(v[0].channels, Channels::Stereo);
        // All-stereo strips write today's text.
        let e = out(Owner::Sound(0, 1), "a", AudioDest::Parent);
        assert_eq!(to_text(&[e.clone()]), "sound:0:1 1 0.5 -0.25 parent plugin=a name=Out%202\n");
        let mut v = Vec::new();
        for ch in [Channels::Stereo, Channels::Mono, Channels::Left, Channels::Right] {
            v.push(ExtraOut { channels: ch, ..e.clone() });
        }
        let text = to_text(&v);
        assert!(text.contains(" ch=mono ") && text.contains(" ch=left ") && text.contains(" ch=right "), "{text}");
        assert!(!text.contains("ch=stereo"), "{text}");
        assert_eq!(from_text(&text), v);
    }

    #[test]
    fn split_makes_left_then_right_with_the_same_settings() {
        let a = out(Owner::Group(0, 0), "a", AudioDest::Ext(3));
        let mut b = out(Owner::Sound(0, 0), "b", AudioDest::Master);
        b.channels = Channels::Mono;
        b.muted = true;
        let c = out(Owner::Master(0), "c", AudioDest::Ext(1));
        let mut v = vec![a.clone(), b.clone(), c.clone()];
        split(&mut v, 1);
        assert_eq!(v.len(), 4);
        assert_eq!(v[0], a);
        assert_eq!(v[1], ExtraOut { channels: Channels::Left, ..b.clone() });
        assert_eq!(v[2], ExtraOut { channels: Channels::Right, ..b.clone() });
        assert_eq!(v[3], c);
        // A half isn't split again; out of range does nothing.
        let before = v.clone();
        split(&mut v, 1);
        split(&mut v, 9);
        assert_eq!(v, before);
    }

    #[test]
    fn join_keeps_the_left_settings_and_drops_the_partner() {
        let a = out(Owner::Group(0, 0), "a", AudioDest::Ext(3));
        let b = out(Owner::Sound(0, 0), "b", AudioDest::Master);
        let mut v = vec![a.clone(), b.clone()];
        split(&mut v, 1);
        v[1].level = 0.25;
        v[1].pan = -1.0;
        v[2].level = 2.0;
        v[2].pan = 1.0;
        v[2].muted = true;
        v[2].dest = AudioDest::Ext(5);
        let left = v[1].clone();
        // Joined from the right half: the left half's settings stay.
        let mut w = v.clone();
        join(&mut w, 2);
        assert_eq!(w, vec![a.clone(), ExtraOut { channels: Channels::Stereo, ..left.clone() }]);
        join(&mut v, 1);
        assert_eq!(v, w);
        // A half whose partner is gone becomes stereo with its own settings.
        let mut v = vec![a.clone(), ExtraOut { channels: Channels::Right, ..b.clone() }];
        join(&mut v, 1);
        assert_eq!(v, vec![a.clone(), b.clone()]);
        // Not a half: nothing.
        join(&mut v, 0);
        assert_eq!(v, vec![a, b]);
    }

    #[test]
    fn strip_label_hides_generic_names() {
        for n in
            ["Aux 1", "Out 3", "Output 3/4", "St. 3/4", "", "Aux 1+2", "Aux L/R", "Out 3+4", "Stereo Out 2", "Main Out", "Out 3 & 4, L"]
        {
            assert_eq!(strip_label(2, n, Channels::Stereo), "Out 3", "{n:?}");
        }
        assert_eq!(strip_label(2, "Kick", Channels::Stereo), "Out 3 Kick");
        assert_eq!(strip_label(2, "Snare Top", Channels::Stereo), "Out 3 Snare Top");
        assert_eq!(strip_label(2, "Hats 1", Channels::Stereo), "Out 3 Hats 1");
        assert_eq!(strip_label(2, "Kick L/R", Channels::Stereo), "Out 3 Kick L/R");
        assert_eq!(strip_label(2, "Kick", Channels::Mono), "Out 3 Kick");
        assert_eq!(strip_label(2, "Out 3", Channels::Left), "Out 3 L");
        assert_eq!(strip_label(2, "Kick", Channels::Right), "Out 3 Kick R");
    }
}
