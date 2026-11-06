//! A plug-in's extra outputs appearing as mixer strips the first time they are heard
//! (docs/superpowers/specs/2026-11-05-plugin-extra-outputs-design.md): each new strip
//! takes the next free connected output, else plays into its parent.

use deckard_engine::Heard;
use deckard_model::extra_out::{ExtraOut, Owner, plugin_at};
use deckard_model::{AudioDest, Project};

/// The most output pairs looked at when finding a free one: the engine's.
const MAX_OUTPUTS: usize = deckard_engine::OUTPUTS;

/// The lowest connected external output pair no channel plays into (not the master or
/// cue output, a sound's or group's destination or aux send, an extra-output strip, or
/// one in `taken`, handed out earlier in the same batch); `Parent` when none is free.
pub fn next_free_output(p: &Project, connected: impl Fn(usize) -> bool, taken: &[usize]) -> AudioDest {
    let mut used: Vec<i64> = vec![];
    let mut note = |d: AudioDest| {
        if let AudioDest::Ext(n) = d {
            used.push(n);
        }
    };
    note(p.master_output);
    note(p.cue_output);
    for g in &p.groups {
        note(g.channel.dest);
        g.channel.aux_dest.into_iter().for_each(&mut note);
        for s in &g.sounds {
            note(s.channel.dest);
            s.channel.aux_dest.into_iter().for_each(&mut note);
        }
    }
    p.extra_outs.iter().for_each(|e| note(e.dest));
    let free = |n: usize| connected(n) && !taken.contains(&n) && !used.contains(&(n as i64));
    (0..MAX_OUTPUTS).find(|&n| free(n)).map_or(AudioDest::Parent, |n| AudioDest::Ext(n as i64))
}

/// A strip for each newly heard (owner, bus) with none yet, in the order heard: level
/// unity, centred, on the next free output. `name` gives (owner, bus)'s display name.
pub fn strips_for(p: &Project, heard: &[Heard], name: impl Fn(Owner, usize) -> String, connected: impl Fn(usize) -> bool) -> Vec<ExtraOut> {
    let mut made: Vec<ExtraOut> = Vec::new();
    let mut taken: Vec<usize> = Vec::new();
    for h in heard {
        let has = |e: &ExtraOut| e.owner == h.owner && e.bus == h.bus;
        if p.extra_outs.iter().chain(&made).any(has) {
            continue;
        }
        let Some(pr) = plugin_at(p, h.owner) else { continue };
        let dest = next_free_output(p, &connected, &taken);
        if let AudioDest::Ext(n) = dest {
            taken.push(n as usize);
        }
        made.push(ExtraOut { owner: h.owner, bus: h.bus, plugin: pr.id.clone(), name: name(h.owner, h.bus), level: 1.0, pan: 0.0, muted: false, dest });
    }
    made
}

/// The plug-in host's slot for a strip owner.
pub fn slot_of(o: Owner) -> deckard_plugins::Slot {
    use deckard_plugins::Slot;
    match o {
        Owner::Sound(group, sound) => Slot::Sound { group, sound },
        Owner::Chain(group, sound, slot) => Slot::Chain { group, sound, slot },
        Owner::Group(group, slot) => Slot::Group { group, slot },
        Owner::Master(slot) => Slot::Master { slot },
    }
}

impl crate::App {
    /// Turn what the engine heard since the last frame into strips: one undoable edit
    /// for all of them. Names come from the live plug-in when it isn't busy, else
    /// "Out N".
    pub(crate) fn add_heard_strips(&mut self) {
        let heard = match &self.output {
            Some(Ok(out)) => out.take_heard(),
            _ => return,
        };
        if heard.is_empty() {
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        let fresh = {
            let d = doc.borrow();
            let name = |o: Owner, bus: usize| {
                self.samples
                    .plugins
                    .existing(slot_of(o))
                    .and_then(|pl| pl.try_lock().ok().map(|p| p.output_name(bus)))
                    .unwrap_or_else(|| format!("Out {}", bus + 1))
            };
            strips_for(d.project(), &heard, name, |n| self.settings.output_pair_on(n))
        };
        if !fresh.is_empty() {
            crate::layout::apply_acts(self, vec![crate::layout::Act::AddExtraOuts(fresh)]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_model::PluginRef;

    fn project_with_plugin() -> Project {
        let mut p = Project::default();
        let mut g = deckard_model::Group::default();
        let mut s = deckard_model::Sound::default();
        s.plugin = Some(PluginRef { id: "synth".into(), ..PluginRef::default() });
        g.sounds.push(s);
        p.groups.push(g);
        p
    }

    fn name(_: Owner, bus: usize) -> String {
        format!("Out {}", bus + 1)
    }

    #[test]
    fn next_free_skips_the_master_and_used_outputs() {
        let mut p = Project::default();
        p.master_output = AudioDest::Ext(0);
        p.cue_output = AudioDest::Ext(0);
        let mut e = ExtraOut { owner: Owner::Sound(0, 0), bus: 1, plugin: "x".into(), name: "".into(), level: 1.0, pan: 0.0, muted: false, dest: AudioDest::Ext(1) };
        p.extra_outs.push(e.clone());
        assert_eq!(next_free_output(&p, |_| true, &[]), AudioDest::Ext(2));
        assert_eq!(next_free_output(&p, |n| n != 2, &[]), AudioDest::Ext(3));
        assert_eq!(next_free_output(&p, |_| true, &[2]), AudioDest::Ext(3));
        e.dest = AudioDest::Master;
        p.extra_outs = vec![e];
        assert_eq!(next_free_output(&p, |_| true, &[]), AudioDest::Ext(1));
    }

    #[test]
    fn next_free_falls_back_to_parent_when_none_free() {
        let mut p = Project::default();
        p.master_output = AudioDest::Ext(0);
        assert_eq!(next_free_output(&p, |n| n < 2, &[1]), AudioDest::Parent);
    }

    #[test]
    fn next_free_never_goes_past_the_engines_outputs() {
        let p = Project::default();
        let taken: Vec<usize> = (0..deckard_engine::OUTPUTS).collect();
        assert_eq!(next_free_output(&p, |_| true, &taken), AudioDest::Parent);
    }

    #[test]
    fn two_buses_heard_together_get_distinct_outputs_in_mixer_order() {
        let p = project_with_plugin();
        let o = Owner::Sound(0, 0);
        let made = strips_for(&p, &[Heard { owner: o, bus: 1 }, Heard { owner: o, bus: 2 }], name, |_| true);
        assert_eq!(made.len(), 2);
        assert_eq!((made[0].bus, made[0].dest), (1, AudioDest::Ext(1)));
        assert_eq!((made[1].bus, made[1].dest), (2, AudioDest::Ext(2)));
        assert_eq!((made[0].level, made[0].pan, made[0].muted, made[0].plugin.as_str()), (1.0, 0.0, false, "synth"));
    }

    #[test]
    fn a_bus_heard_twice_makes_one_strip() {
        let mut p = project_with_plugin();
        let h = Heard { owner: Owner::Sound(0, 0), bus: 1 };
        assert_eq!(strips_for(&p, &[h, h], name, |_| true).len(), 1);
        p.extra_outs = strips_for(&p, &[h], name, |_| true);
        assert!(strips_for(&p, &[h], name, |_| true).is_empty());
    }
}
