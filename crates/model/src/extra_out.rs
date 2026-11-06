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

/// One extra output: output pair `bus` of the plug-in `plugin` (its `PluginRef::id`) in
/// `owner`'s slot, called `name`. `level` is linear (1.0 = 0 dB), `pan` -1..1.
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
/// `owner bus level pan dest [muted] plugin=ID name=NAME` (ID and NAME with whitespace
/// and `%` written as `%XX`).
pub fn to_text(v: &[ExtraOut]) -> String {
    let mut s = String::new();
    for e in v {
        s.push_str(&format!(
            "{} {} {} {} {}{} plugin={} name={}\n",
            owner_text(e.owner),
            e.bus,
            e.level,
            e.pan,
            dest_text(e.dest),
            if e.muted { " muted" } else { "" },
            escape(&e.plugin),
            escape(&e.name),
        ));
    }
    s
}

/// Strips from [`to_text`]'s text. Lines that don't read are skipped, and unknown
/// `key=` words ignored.
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
    let mut e = ExtraOut { owner, bus, plugin: String::new(), name: String::new(), level, pan, muted: false, dest };
    for word in w {
        if word == "muted" {
            e.muted = true;
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
        ExtraOut { owner, bus: 1, plugin: plugin.into(), name: "Out 2".into(), level: 0.5, pan: -0.25, muted: false, dest }
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
        let dests = [
            AudioDest::None,
            AudioDest::Parent,
            AudioDest::Master,
            AudioDest::Ext(2),
            AudioDest::Group(3),
            AudioDest::Sound(1, 7),
        ];
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
}
