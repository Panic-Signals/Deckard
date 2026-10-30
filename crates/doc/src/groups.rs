//! Adding groups to the song (MK3 manual, "Creating Groups"): a new empty group of
//! 16 empty sounds goes at the end of the group list, named after its place (Group A1–H1
//! in the first bank, A2–H2 in the second, …) with the default colour, and becomes the
//! focused group.
//!
//! The song keeps one browser handle per group (`quick_browse`, `group_0`…), so one is
//! added with it; each scene gets an empty slot for it (playing nothing), as a new
//! project's scene has for its group.

use super::arrange::px;
use super::build::{GROUP, set_text};
use super::channel::SoundRoute;
use super::{Document, Op, Target};
use deckard_model::{AudioDest, AudioSource, ScSource};
use mxprj::decode::{Obj, Value};

impl Document {
    /// A new empty group at the end of the group list; returns its index.
    pub fn add_group(&mut self) -> Result<usize, String> {
        let song = self.addresses.song.clone().ok_or("the project has no song")?;
        let items_path = self.field_path(&song, &["groups", "objects", "items"])?;
        let current_path = self.field_path(&song, &["groups", "current"])?;
        let qb_path = self.field_path(&song, &["quick_browse"])?;
        let mut items = self.items(&items_path)?;
        let n = items.len();
        let name = format!("Group {}{}", (b'A' + (n % 8) as u8) as char, n / 8 + 1);

        let (item, oid, handle) = self.build(|b| {
            let mut g = b.group(16)?;
            set_text(&mut g, "name", &name);
            let oid = g.oid.ok_or("group without id")?;
            let handle = b.handles("group_", 1)?;
            Ok((b.shared(GROUP, g), oid, handle))
        })?;
        items.push(item);

        // One handle per group: `group_0` alone is stored as itself.
        let qb = match self.tree.at(&qb_path) {
            Some(Value::Obj(o)) if o.type_name.is_empty() => {
                let mut o = o.clone();
                o.fields.push((format!("group_{n}"), handle));
                Value::Obj(o)
            }
            Some(one) => {
                Value::Obj(Obj { fields: vec![("group_0".into(), one.clone()), (format!("group_{n}"), handle)], ..Obj::default() })
            }
            None => return Err("song without quick_browse".into()),
        };

        let mut ops = vec![
            Op::Set { path: items_path, value: Value::List(items) },
            Op::Set { path: current_path, value: Value::Ref(oid) },
            Op::Set { path: qb_path, value: qb },
        ];
        // An empty slot for it in every scene.
        let scenes_path = self.field_path(&song, &["scenes", "objects", "items"])?;
        for k in 0..self.items(&scenes_path)?.len() {
            let mut item = [scenes_path.as_slice(), &[k as u32]].concat();
            let Some(Value::Obj(holder)) = self.tree.at(&item) else { continue };
            item.extend(holder.find_path("px").ok_or("scene item without px")?);
            let entries_path = self.field_path(&item, &["patterns", "objects", "items"])?;
            let mut entries = self.items(&entries_path)?;
            entries.push(self.build(|b| b.scene_entry(oid))?);
            ops.push(Op::Set { path: entries_path, value: Value::List(entries) });
        }
        self.apply("Add Group", ops)?;
        Ok(n)
    }

    /// Delete group `g` (one undo step), as Maschine's SHIFT + ERASE + group button or
    /// the group's Delete: its scene slots go with it, and what was routed to it or took
    /// its audio goes back to its default. The last group is reset instead (a new empty
    /// one in its place): a project keeps one.
    pub fn delete_group(&mut self, g: usize) -> Result<(), String> {
        let n = self.project().groups.len();
        if g >= n {
            return Err("no such group".into());
        }
        self.transaction("Delete Group", |d| {
            if n == 1 {
                d.add_group()?;
                d.remove_group(0)?;
                // (Unnamed, as a new project's: shown by its place.)
                d.rename(Target::Group(0), "")?;
                return d.fix_lfo_triggers(|_, _| None);
            }
            d.unroute_group(g)?;
            d.remove_group(g)?;
            d.fix_lfo_triggers(|x, s| match x.cmp(&g) {
                std::cmp::Ordering::Less => Some((x, s)),
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some((x - 1, s)),
            })
        })
    }

    /// Point everything routed to group `g` (or its sounds) elsewhere: outputs to their
    /// parent, inputs and sends to none, side-chains off, the master and cue to Ext. 1.
    fn unroute_group(&mut self, g: usize) -> Result<(), String> {
        let p = self.project().clone();
        let dest = |d: AudioDest| matches!(d, AudioDest::Group(x) | AudioDest::Sound(x, _) if x == g);
        for (x, group) in p.groups.iter().enumerate().filter(|(x, _)| *x != g) {
            for (s, sound) in group.sounds.iter().enumerate() {
                let c = &sound.channel;
                if dest(c.dest) {
                    self.set_sound_route(x, s, SoundRoute::Output(AudioDest::Parent))?;
                }
                for (a, d) in c.aux_dest.iter().enumerate() {
                    if dest(*d) {
                        self.set_sound_route(x, s, SoundRoute::Aux(a, AudioDest::None))?;
                    }
                }
                if matches!(c.audio_in, AudioSource::Group(y) | AudioSource::Sound(y, _) if y == g) {
                    self.set_sound_route(x, s, SoundRoute::AudioIn(AudioSource::None))?;
                }
                for (slot, src) in &c.sidechain {
                    if matches!(src, ScSource::Group(y) | ScSource::Sound(y, _) if *y == g) {
                        self.set_sidechain(x, s, *slot, None)?;
                    }
                }
            }
            if dest(group.channel.dest) {
                self.set_group_route(x, None, AudioDest::Parent)?;
            }
            for (a, d) in group.channel.aux_dest.iter().enumerate() {
                if dest(*d) {
                    self.set_group_route(x, Some(a), AudioDest::None)?;
                }
            }
        }
        for (cue, d) in [(false, p.master_output), (true, p.cue_output)] {
            if dest(d) {
                self.set_master_output(cue, AudioDest::Ext(0))?;
            }
        }
        Ok(())
    }

    /// Take group `g` out of the group list, the song's browser handles and the scenes.
    fn remove_group(&mut self, g: usize) -> Result<(), String> {
        let song = self.addresses.song.clone().ok_or("the project has no song")?;
        let items_path = self.field_path(&song, &["groups", "objects", "items"])?;
        let current_path = self.field_path(&song, &["groups", "current"])?;
        let qb_path = self.field_path(&song, &["quick_browse"])?;
        let mut items = self.items(&items_path)?;
        let oid = items.get(g).and_then(px).and_then(|o| o.oid).ok_or("group without id")?;
        items.remove(g);

        // The handles `group_0`… follow the groups (one alone is stored as itself).
        let qb = match self.tree.at(&qb_path) {
            Some(Value::Obj(o)) if o.type_name.is_empty() => {
                let mut handles: Vec<Value> = o.fields.iter().filter(|(k, _)| k.starts_with("group_")).map(|(_, v)| v.clone()).collect();
                if g < handles.len() {
                    handles.remove(g);
                }
                match handles.len() {
                    1 => handles.remove(0),
                    _ => Value::Obj(Obj {
                        fields: handles.into_iter().enumerate().map(|(k, v)| (format!("group_{k}"), v)).collect(),
                        ..o.clone()
                    }),
                }
            }
            Some(one) => one.clone(),
            None => return Err("song without quick_browse".into()),
        };

        let mut ops = vec![Op::Set { path: items_path, value: Value::List(items.clone()) }, Op::Set { path: qb_path, value: qb }];
        ops.extend(self.fix_current(&current_path, &items, Some(g.min(items.len().saturating_sub(1)))));
        let scenes_path = self.field_path(&song, &["scenes", "objects", "items"])?;
        for k in 0..self.items(&scenes_path)?.len() {
            let mut item = [scenes_path.as_slice(), &[k as u32]].concat();
            let Some(Value::Obj(holder)) = self.tree.at(&item) else { continue };
            item.extend(holder.find_path("px").ok_or("scene item without px")?);
            let entries_path = self.field_path(&item, &["patterns", "objects", "items"])?;
            let mut entries = self.items(&entries_path)?;
            entries.retain(|e| !matches!(e, Value::Obj(o) if matches!(o.get("first"), Some(Value::Ref(r)) if *r == oid) || o.get("first").and_then(|f| match f { Value::Obj(f) => f.oid, _ => None }) == Some(oid)));
            ops.push(Op::Set { path: entries_path, value: Value::List(entries) });
        }
        self.apply("Delete Group", ops)
    }
}

#[cfg(test)]
mod tests {
    use crate::Document;

    /// A new group comes last, named for its place, with 16 empty sounds; the project
    /// saves and opens again with it.
    #[test]
    fn groups_are_added_at_the_end_and_saved() {
        let mut d = Document::new_project().unwrap();
        for k in 1..=9 {
            assert_eq!(d.add_group().unwrap(), k);
        }
        let names: Vec<String> = d.project().groups.iter().map(|g| g.name.clone()).collect();
        assert_eq!(names[1..3], ["Group B1", "Group C1"]);
        assert_eq!(names[8..], ["Group A2", "Group B2"]);
        assert!(d.project().groups.iter().all(|g| g.sounds.len() == 16));
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let path = dir.join("groups.mxprj");
        d.save_as(&path).unwrap();
        let again = Document::open(&path).unwrap();
        assert_eq!(again.project().groups.len(), 10);
        assert_eq!(again.project().groups[9].name, "Group B2");
    }

    /// Deleting a group takes it, its scene slots and anything routed to it away; the
    /// project opens again. The last group is reset instead (a project keeps one).
    #[test]
    fn a_group_is_deleted_or_the_last_reset() {
        use crate::channel::SoundRoute;
        use deckard_model::{AudioDest, AudioSource, ScSource};
        let mut d = Document::new_project().unwrap();
        d.add_group().unwrap();
        d.add_group().unwrap();
        d.rename(crate::Target::Group(2), "Keep").unwrap();
        d.set_sound_route(2, 0, SoundRoute::Output(AudioDest::Group(1))).unwrap();
        d.set_sound_route(2, 1, SoundRoute::AudioIn(AudioSource::Sound(1, 3))).unwrap();
        d.set_sidechain(0, 0, 1, Some(ScSource::Group(1))).unwrap();
        d.set_master_output(true, AudioDest::Group(1)).unwrap();
        d.delete_group(1).unwrap();
        let p = d.project();
        assert_eq!(p.groups.len(), 2);
        assert_eq!(p.groups[1].name, "Keep");
        assert!(p.scenes.iter().all(|s| s.patterns.iter().all(|&(g, _)| g < 2)));
        assert_eq!(p.groups[1].sounds[0].channel.dest, AudioDest::Parent);
        assert_eq!(p.groups[1].sounds[1].channel.audio_in, AudioSource::None);
        assert!(p.groups[0].sounds[0].channel.sidechain.is_empty());
        assert_eq!(p.cue_output, AudioDest::Ext(0));
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let path = dir.join("deleted.mxprj");
        d.save_as(&path).unwrap();
        let again = Document::open(&path).unwrap();
        assert_eq!(again.project().groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["", "Keep"]);
        // One undo step.
        d.undo();
        assert_eq!(d.project().groups.len(), 3);
        d.redo();
        d.delete_group(0).unwrap();
        d.delete_group(0).unwrap();
        assert_eq!(d.project().groups.len(), 1);
        assert_eq!(d.project().groups[0].name, "", "reset, unnamed");
        d.save_as(&path).unwrap();
        assert_eq!(Document::open(&path).unwrap().project().groups.len(), 1);
    }

    /// The same in a project saved by Maschine (when there's one to read): it opens
    /// again with the new group and its scenes intact.
    #[test]
    fn a_group_is_added_to_a_maschine_project() {
        let src = std::env::var_os("DECKARD_TEST_PROJECT").map_or_else(
            || std::path::PathBuf::from("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj"),
            std::path::PathBuf::from,
        );
        let Ok(mut d) = Document::open(&src) else { return };
        let (n, scenes) = (d.project().groups.len(), d.project().scenes.clone());
        assert_eq!(d.add_group().unwrap(), n);
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let path = dir.join("copy.mxprj");
        d.save_as(&path).unwrap();
        let again = Document::open(&path).unwrap();
        assert_eq!(again.project().groups.len(), n + 1);
        assert_eq!(
            again.project().scenes.iter().map(|s| s.patterns.clone()).collect::<Vec<_>>(),
            scenes.iter().map(|s| s.patterns.clone()).collect::<Vec<_>>()
        );
    }

    /// Deleting the first, a middle and the last group of a project saved by Maschine (when there's one to read): it
    /// opens again without it, its scenes otherwise intact.
    #[test]
    fn a_group_is_deleted_from_a_maschine_project() {
        let src = std::env::var_os("DECKARD_TEST_PROJECT").map_or_else(
            || std::path::PathBuf::from("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj"),
            std::path::PathBuf::from,
        );
        let Ok(d) = Document::open(&src) else { return };
        let n = d.project().groups.len();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut picks = vec![0, n / 2, n.saturating_sub(1)];
        picks.dedup();
        for g in picks {
            let mut d = Document::open(&src).unwrap();
            let names: Vec<String> = d.project().groups.iter().map(|x| x.name.clone()).collect();
            d.delete_group(g).unwrap();
            let path = dir.join(format!("copy{g}.mxprj"));
            d.save_as(&path).unwrap();
            let again = Document::open(&path).unwrap();
            let left: Vec<String> = again.project().groups.iter().map(|x| x.name.clone()).collect();
            let mut want = names.clone();
            if n > 1 {
                want.remove(g);
            }
            assert_eq!(left.len(), want.len(), "group {g}");
            if n > 1 {
                assert_eq!(left, want, "group {g}");
            }
        }
    }
}
