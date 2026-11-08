//! Reorder intact Group objects. Routing, scenes and clips refer to their object
//! identities; only the browser's positional handles and LFO triggers need moving.
use crate::{Document, Op};
use mxprj::decode::Value;

impl Document {
    /// Stable identities in slot order, for consumers keeping session-only state.
    pub fn group_ids(&self) -> Vec<u64> {
        self.addresses
            .groups
            .iter()
            .map(|g| match self.tree.at(&g.group) {
                Some(Value::Obj(o)) => o.oid.unwrap_or(0),
                _ => 0,
            })
            .collect()
    }

    /// Trade two complete Groups in one undoable edit. Names and custom colours
    /// travel with their content. No objects are copied or re-identified.
    pub fn swap_groups(&mut self, a: usize, b: usize) -> Result<(), String> {
        let n = self.project().groups.len();
        if a >= n || b >= n {
            return Err("no such group".into());
        }
        if a == b {
            return Ok(());
        }
        let song = self.addresses.song.clone().ok_or("no song")?;
        let path = self.field_path(&song, &["groups", "objects", "items"])?;
        let mut items = self.items(&path)?;
        items.swap(a, b);
        let qb = self.field_path(&song, &["quick_browse"])?;
        let mut handles = self.tree.at(&qb).cloned().ok_or("no browser handles")?;
        if let Value::Obj(o) = &mut handles {
            let ia = o.fields.iter().position(|(k, _)| k == &format!("group_{a}"));
            let ib = o.fields.iter().position(|(k, _)| k == &format!("group_{b}"));
            if let (Some(ia), Some(ib)) = (ia, ib) {
                let va = o.fields[ia].1.clone();
                o.fields[ia].1 = o.fields[ib].1.clone();
                o.fields[ib].1 = va;
            }
        }
        self.transaction("Move Group", |d| {
            d.apply("Move Group", vec![Op::Set { path, value: Value::List(items) }, Op::Set { path: qb, value: handles }])?;
            d.fix_lfo_triggers(|g, s| {
                Some((
                    if g == a {
                        b
                    } else if g == b {
                        a
                    } else {
                        g
                    },
                    s,
                ))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Target, channel::SoundRoute};
    use deckard_model::{AudioDest, AudioSource, Color, ScSource, lfo::Lfo};
    #[test]
    fn group_move_keeps_objects_routes_scenes_clips_and_undo() {
        let mut d = Document::new_project().unwrap();
        d.add_group().unwrap();
        d.add_group().unwrap();
        d.rename(Target::Group(0), "Kick").unwrap();
        d.set_color(&[Target::Group(0)], Color::Palette(3)).unwrap();
        let pattern = d.add_pattern_in_slot(0, 0, 0, "Beat").unwrap();
        d.assign_pattern(0, 0, Some(pattern)).unwrap();
        d.convert_to_clip(0, pattern, 0, 3840).unwrap();
        d.set_sound_route(2, 0, SoundRoute::Output(AudioDest::Group(0))).unwrap();
        d.set_sound_route(2, 1, SoundRoute::AudioIn(AudioSource::Sound(0, 2))).unwrap();
        d.set_sound_route(2, 2, SoundRoute::Aux(0, AudioDest::Sound(1, 0))).unwrap();
        d.set_group_route(1, None, AudioDest::Group(0)).unwrap();
        d.set_sidechain(2, 0, 1, Some(ScSource::Sound(0, 3))).unwrap();
        d.set_master_output(true, AudioDest::Group(0)).unwrap();
        d.set_lfo(Lfo { trigger: Some((0, 2)), ..Lfo::new(1) }).unwrap();
        let original = d.project().clone();
        let ids = d.group_ids();
        d.swap_groups(0, 1).unwrap();
        let p = d.project();
        assert_eq!(p.groups[1].name, "Kick");
        assert_eq!(p.groups[1].color, Color::Palette(3));
        assert_eq!(p.groups[1].patterns, original.groups[0].patterns);
        assert_eq!(p.groups[1].clips, original.groups[0].clips);
        assert!(p.scenes[0].patterns.contains(&(1, pattern)));
        assert_eq!(p.groups[2].sounds[0].channel.dest, AudioDest::Group(1));
        assert_eq!(p.groups[2].sounds[1].channel.audio_in, AudioSource::Sound(1, 2));
        assert_eq!(p.groups[2].sounds[2].channel.aux_dest[0], AudioDest::Sound(0, 0));
        assert_eq!(p.groups[0].channel.dest, AudioDest::Group(1));
        assert_eq!(p.groups[2].sounds[0].channel.sidechain, [(1, ScSource::Sound(1, 3))]);
        assert_eq!(p.cue_output, AudioDest::Group(1));
        assert_eq!(p.lfos[0].trigger, Some((1, 2)));
        assert_eq!(d.group_ids(), [ids[1], ids[0], ids[2]]);
        assert_eq!(d.undo_label(), Some("Move Group"));
        let saved = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(saved.project(), d.project());
        assert!(d.undo());
        assert_eq!(d.project(), &original);
        assert!(d.redo());
        assert_eq!(d.group_ids(), [ids[1], ids[0], ids[2]]);
        d.swap_groups(0, 1).unwrap();
        assert_eq!(d.project(), &original);
    }
    #[test]
    fn group_move_rejects_missing_slots_and_same_slot_is_no_edit() {
        let mut d = Document::new_project().unwrap();
        let revision = d.revision();
        assert!(d.swap_groups(0, 1).is_err());
        d.swap_groups(0, 0).unwrap();
        assert_eq!(d.revision(), revision);
    }
}
