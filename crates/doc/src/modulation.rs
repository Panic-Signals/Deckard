//! Modulation (docs/superpowers/plans/2026-10-22-phase2c-modulation.md): a channel's
//! lanes are its `modulators` (`ObjectVector<Modulator>`, each `target` a pointer to
//! the parameter), and the points are `ModulationEvent<T>`s (`parameter` a pointer to
//! the same parameter) in the pattern's sequences: a sound's in its own sequence, the
//! group's in the pattern's `sequence`. A v0 event holds its value (`constant`); a v1
//! event a `Modulation` (`a = [0, 1, 0, 0]`, `b = [value, 0, 0, 0]` for a plain point).
//! An archive has one version per class: points are written in the version the project
//! has, else Maschine 3's.

use crate::automation::pointee;
use crate::notes::{Edits, Item, events_with, items_of};
use crate::{Chain, Document, Op};
use deckard_model::ModTarget;
use mxprj::decode::{Obj, Path, Value};
use std::collections::HashSet;

const MODULATOR: &str = "NI::MASCHINE::DATA::Modulator";

impl Document {
    /// Write the points (position, value) of `target`'s modulation in pattern `p` of
    /// chain `c`'s group, replacing its points from `span.0` to `span.1` (inclusive),
    /// and add its lane if it has none, as one undo step. The master's parameters
    /// aren't modulated.
    pub fn set_modulation(
        &mut self,
        c: Chain,
        target: &ModTarget,
        p: usize,
        span: (i64, i64),
        points: &[(i64, f64)],
    ) -> Result<(), String> {
        let param = self.mod_param(c, target)?;
        let mut ops = self.lane_ops(c, param.0)?;
        ops.extend(self.point_ops(c, &param, p, span, points)?);
        self.apply("Record Modulation", ops)
    }

    /// Remove `target`'s modulation: its lane and its points in every pattern, as one
    /// undo step.
    pub fn clear_modulation(&mut self, c: Chain, target: &ModTarget) -> Result<(), String> {
        let (oid, _) = self.mod_param(c, target)?;
        // Nothing is skipped: no path starts with this.
        let ops = self.drop_modulation_ops(&HashSet::from([oid]), &vec![u32::MAX], false)?;
        if ops.is_empty() {
            return Ok(());
        }
        self.apply("Remove Modulation", ops)
    }

    /// Remove every modulation point of chain `c`'s channel in pattern `p` (its lanes
    /// stay), as one undo step.
    pub fn clear_pattern_modulation(&mut self, c: Chain, p: usize) -> Result<(), String> {
        let paths = self.object_paths();
        let seq_path = match c {
            Chain::Sound(g, s) => match self.sequence_of(g, p, s, &paths)?.2 {
                Some(sp) => sp,
                None => return Ok(()),
            },
            Chain::Group(g) => {
                let pattern = self.addresses.groups.get(g).and_then(|ga| ga.patterns.get(p)).ok_or("no such pattern")?.clone();
                let sp = self.field_path(&pattern, &["sequence"])?;
                match self.tree.at(&sp) {
                    Some(Value::Obj(_)) => sp,
                    Some(Value::Ref(id)) => paths.get(id).cloned().ok_or("sequence not found")?,
                    _ => return Ok(()),
                }
            }
            Chain::Master => return Err("the master's parameters are not modulated".into()),
        };
        let events_path = self.field_path(&seq_path, &["events"])?;
        let Some(Value::Obj(events)) = self.tree.at(&events_path).cloned() else { return Err("sequence without events".into()) };
        let mut items = items_of(&events)?;
        let n = items.len();
        items.retain(|it| !self.event_obj(&it.event, &paths).is_some_and(|ev| ev.type_name.contains("ModulationEvent")));
        if items.len() == n {
            return Ok(());
        }
        let mut ops = vec![Op::Set { path: events_path, value: Value::Obj(events_with(&events, &items)) }];
        let current = self.obj_by_path(&seq_path).and_then(|s| pointee(s.get("current")));
        if current.is_some_and(|c| !items.iter().any(|it| self.event_obj(&it.event, &paths).and_then(|e| e.oid) == Some(c))) {
            ops.push(Op::Set { path: self.field_path(&seq_path, &["current"])?, value: Value::Null });
        }
        self.apply("Clear Modulation", ops)
    }

    /// The channel (sound or group) of chain `c`.
    fn channel_path(&self, c: Chain) -> Result<Path, String> {
        match c {
            Chain::Sound(g, s) => {
                self.addresses.groups.get(g).and_then(|ga| ga.sounds.get(s)).cloned().ok_or_else(|| "no such sound".into())
            }
            Chain::Group(g) => Ok(self.addresses.groups.get(g).ok_or("no such group")?.group.clone()),
            Chain::Master => Err("the master's parameters are not modulated".into()),
        }
    }

    /// `target`'s parameter: its object id and class.
    fn mod_param(&self, c: Chain, target: &ModTarget) -> Result<(u64, String), String> {
        let channel = self.channel_path(c)?;
        let mut o: &Obj = match target.slot {
            Some(i) => {
                let items = self.field_path(&self.chain_of(c)?, &["slots", "objects", "items"])?;
                let Some(Value::List(list)) = self.tree.at(&items) else { return Err("chain without slots".into()) };
                crate::chain::module_of(list.get(i).ok_or("no such slot")?).ok_or("the slot's module isn't stored there")?
            }
            None => self.obj_by_path(&channel).ok_or("no such channel")?,
        };
        for part in target.field.split('/') {
            match o.find(part) {
                Some(Value::Obj(f)) => o = f,
                _ => return Err(format!("no parameter {}", target.field)),
            }
        }
        Ok((o.oid.ok_or("parameter without id")?, o.type_name.clone()))
    }

    /// Ops adding a lane for parameter `param` to chain `c`'s channel, if it has none.
    fn lane_ops(&mut self, c: Chain, param: u64) -> Result<Vec<Op>, String> {
        let v = self.field_path(&self.channel_path(c)?, &["modulators"])?;
        let items_path = self.field_path(&v, &["objects", "items"])?;
        let current_path = self.field_path(&v, &["current"])?;
        let mut items = self.items(&items_path)?;
        let lane_of = |i: &Value| match i {
            Value::Obj(w) => match w.get("px") {
                Some(Value::Obj(m)) => pointee(m.get("target")),
                _ => None,
            },
            _ => None,
        };
        if items.iter().any(|i| lane_of(i) == Some(param)) {
            return Ok(Vec::new());
        }
        let mut item = self.build(|b| b.item(MODULATOR))?;
        let Value::Obj(w) = &mut item else { unreachable!() };
        let Some(Value::Obj(m)) = w.get_mut("px") else { return Err("modulator item without its object".into()) };
        *m.get_mut("target").ok_or("modulator without target")? = Value::Ref(param);
        let oid = m.oid.ok_or("modulator without id")?;
        items.push(item);
        let mut ops = vec![Op::Set { path: items_path, value: Value::List(items) }];
        if matches!(self.tree.at(&current_path), Some(Value::Null)) {
            ops.push(Op::Set { path: current_path, value: Value::Ref(oid) });
        }
        Ok(ops)
    }

    /// Ops writing the points of parameter `param` (id, class) in pattern `p`.
    fn point_ops(&mut self, c: Chain, param: &(u64, String), p: usize, span: (i64, i64), points: &[(i64, f64)]) -> Result<Vec<Op>, String> {
        let paths = self.object_paths();
        let mut new = Vec::new();
        for &(position, value) in points {
            let event = self.build(|b| b.mod_event(&param.1, param.0, position, value))?;
            new.push(Item { event, node: self.new_node()? });
        }
        new.sort_by_key(Item::position);
        let seq_path = match c {
            Chain::Sound(g, s) => {
                let (entries_path, entry, seq) = self.sequence_of(g, p, s, &paths)?;
                match seq {
                    Some(sp) => sp,
                    None => return self.create_sequence_with(g, s, &entries_path, entry, &Edits::default(), new, &paths),
                }
            }
            Chain::Group(g) => {
                let pattern = self.addresses.groups.get(g).and_then(|ga| ga.patterns.get(p)).ok_or("no such pattern")?.clone();
                let sp = self.field_path(&pattern, &["sequence"])?;
                match self.tree.at(&sp) {
                    Some(Value::Obj(_)) => sp,
                    Some(Value::Ref(id)) => paths.get(id).cloned().ok_or("sequence not found")?,
                    _ => return Err("the pattern has no sequence".into()),
                }
            }
            Chain::Master => return Err("the master's parameters are not modulated".into()),
        };
        let events_path = self.field_path(&seq_path, &["events"])?;
        let Some(Value::Obj(events)) = self.tree.at(&events_path).cloned() else { return Err("sequence without events".into()) };
        let mut items = items_of(&events)?;
        let mut removed = Vec::new();
        items.retain(|it| {
            let Some(ev) = self.event_obj(&it.event, &paths) else { return true };
            let hit = ev.type_name.contains("ModulationEvent")
                && pointee(ev.get("parameter")) == Some(param.0)
                && (span.0..=span.1).contains(&it.position());
            if hit {
                removed.extend(ev.oid);
            }
            !hit
        });
        for it in new {
            let at = items.iter().position(|i| i.position() > it.position()).unwrap_or(items.len());
            items.insert(at, it);
        }
        let mut ops = vec![Op::Set { path: events_path, value: Value::Obj(events_with(&events, &items)) }];
        // The sequence's `current` event, if it was removed.
        if let Some(Value::Ref(cur)) = self.obj_by_path(&seq_path).and_then(|s| s.get("current"))
            && removed.contains(cur)
        {
            ops.push(Op::Set { path: self.field_path(&seq_path, &["current"])?, value: Value::Null });
        }
        Ok(ops)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Chain, Document, NewModule};
    use deckard_model::{ModPoint, ModTarget, Project};

    fn cutoff() -> ModTarget {
        ModTarget { slot: Some(0), field: "cutoff".into() }
    }

    /// A new project with a pattern, a Filter on sound 1 and one on the group.
    fn project() -> Document {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "Pattern 1").unwrap();
        d.add_slot(0, 0, 0, &NewModule::Effect("Filter")).unwrap();
        d.add_slot_in(Chain::Group(0), 0, &NewModule::Effect("Filter")).unwrap();
        d
    }

    fn points(p: &Project, sound: Option<usize>) -> Vec<(i64, f64)> {
        p.groups[0].patterns[0]
            .mods
            .iter()
            .filter(|m: &&ModPoint| m.sound == sound && m.field == "cutoff")
            .map(|m| (m.position, (m.value * 1000.0).round() / 1000.0))
            .collect()
    }

    #[test]
    fn points_are_written_with_their_lane_and_read_back() {
        let mut d = project();
        d.set_modulation(Chain::Sound(0, 0), &cutoff(), 0, (0, 960), &[(0, 0.1), (480, -0.2)]).unwrap();
        assert_eq!(points(d.project(), Some(0)), [(0, 0.1), (480, -0.2)]);
        assert_eq!(d.project().groups[0].sounds[0].modulators, [cutoff()]);
        // Saved and opened again (Maschine 3's v1 points in a new project).
        let again = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(points(again.project(), Some(0)), [(0, 0.1), (480, -0.2)]);
        let mut versions = Vec::new();
        again.root().walk_paths(&mut |o, _| {
            if o.type_name.contains("ModulationEvent") {
                versions.push(o.version);
            }
        });
        assert_eq!(versions, [1, 1]);
        // One undo step: the lane and the points.
        d.undo();
        assert!(points(d.project(), Some(0)).is_empty());
        assert!(d.project().groups[0].sounds[0].modulators.is_empty());
    }

    #[test]
    fn writing_again_replaces_the_span() {
        let mut d = project();
        let c = Chain::Sound(0, 0);
        d.set_modulation(c, &cutoff(), 0, (0, 960), &[(0, 0.1), (480, 0.2), (960, 0.3)]).unwrap();
        d.set_modulation(c, &cutoff(), 0, (400, 1000), &[(500, 0.5)]).unwrap();
        assert_eq!(points(d.project(), Some(0)), [(0, 0.1), (500, 0.5)]);
        assert_eq!(d.project().groups[0].sounds[0].modulators.len(), 1, "one lane");
    }

    #[test]
    fn the_groups_points_are_the_patterns_own() {
        let mut d = project();
        d.set_modulation(Chain::Group(0), &cutoff(), 0, (0, 0), &[(240, 0.25)]).unwrap();
        assert_eq!(points(d.project(), None), [(240, 0.25)]);
        assert_eq!(d.project().groups[0].modulators, [cutoff()]);
        let again = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(points(again.project(), None), [(240, 0.25)]);
    }

    #[test]
    fn clearing_removes_the_lane_and_every_point() {
        let mut d = project();
        let c = Chain::Sound(0, 0);
        d.set_modulation(c, &cutoff(), 0, (0, 960), &[(0, 0.1), (480, 0.2)]).unwrap();
        d.clear_modulation(c, &cutoff()).unwrap();
        assert!(points(d.project(), Some(0)).is_empty());
        assert!(d.project().groups[0].sounds[0].modulators.is_empty());
    }

    #[test]
    fn the_masters_parameters_are_not_modulated() {
        let mut d = project();
        d.add_slot_in(Chain::Master, 0, &NewModule::Effect("Filter")).unwrap();
        assert!(d.set_modulation(Chain::Master, &cutoff(), 0, (0, 0), &[(0, 0.1)]).is_err());
    }

    /// A project with Maschine 2's points gets v0 points.
    #[test]
    fn points_keep_the_projects_version() {
        let path = std::path::Path::new("/Users/Shared/Solar Breeze Library/Projects/Atma.mxprj");
        let Ok(mut d) = Document::open(path) else { return };
        let (s, target) = d.project().groups[0]
            .sounds
            .iter()
            .enumerate()
            .find_map(|(s, snd)| snd.slots.iter().position(|x| !x.params.is_empty()).map(|i| (s, i)).map(|(s, i)| (s, (i, snd))))
            .map(|(s, (i, snd))| (s, ModTarget { slot: Some(i), field: snd.slots[i].params[0].0.clone() }))
            .expect("a sound with an effect");
        d.set_modulation(Chain::Sound(0, s), &target, 0, (0, 0), &[(0, 0.5)]).unwrap();
        let again = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        let p = &again.project().groups[0].patterns[0];
        assert!(p.mods.iter().any(|m| m.sound == Some(s) && m.field == target.field && m.position == 0 && (m.value - 0.5).abs() < 1e-6));
        let mut versions = std::collections::BTreeSet::new();
        again.root().walk_paths(&mut |o, _| {
            if o.type_name.contains("ModulationEvent") {
                versions.insert(o.version);
            }
        });
        assert_eq!(versions.into_iter().collect::<Vec<_>>(), [0]);
    }
}
