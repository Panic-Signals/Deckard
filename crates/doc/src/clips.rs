//! Clips (MK3 manual, "Working with Clips"): a group's clips are unique patterns of its
//! own (the group's `patterns`, an object vector of `EventPattern`s) placed on its lane
//! in the Song view (the group's `sequence`: a `ClipEvent` for each, its position and
//! length, pointing at its pattern). A clip plays in place of the group's pattern in
//! the section.
//!
//! The note-editing functions take a clip's pattern as pattern [`CLIP`] + its index in
//! `Group::clip_patterns`.

use crate::notes::{Item, events_with, items_of};
use crate::{Document, Op};
use deckard_model::{MidiEvent, Note};
use mxprj::decode::{Obj, Path, Value};

/// Pattern indices from here name a group's clip patterns (`CLIP + k`), for
/// [`Document::edit_notes`] and the other note edits.
pub const CLIP: usize = 1 << 24;

/// What a new clip's pattern is copied from.
#[derive(Clone, Copy)]
enum From {
    Pattern(usize),
    Clip(usize),
}

impl Document {
    /// Where group `g`'s clip patterns (their items) and clip lane are.
    fn clip_paths(&self, g: usize) -> Result<(Path, Path), String> {
        let ga = self.addresses.groups.get(g).ok_or("no such group")?;
        let items =
            self.field_path(&ga.group, &["patterns", "objects", "items"]).map_err(|_| "the group has no clip list (an old project?)")?;
        Ok((items, self.field_path(&ga.group, &["sequence"])?))
    }

    /// Convert pattern `pi` of group `g` into a clip at `position`, `length` ticks long
    /// (⌘. / the pattern's Convert to Clip): a copy of it, notes and all, unique to the
    /// clip. One undo step; returns the clip's index.
    pub fn convert_to_clip(&mut self, g: usize, pi: usize, position: i64, length: i64) -> Result<usize, String> {
        let mut k = 0;
        self.transaction("Convert to Clip", |d| {
            k = d.new_clip(g, From::Pattern(pi), position, length)?;
            Ok(())
        })?;
        Ok(k)
    }

    /// Section ▸ Convert to Clip(s): each pattern the section's scene plays becomes a
    /// clip over the section. One undo step.
    pub fn convert_section_to_clips(&mut self, section: usize) -> Result<(), String> {
        let sec = *self.project.sections.get(section).ok_or("no such section")?;
        let start = self.project.section_start(section);
        let patterns = sec.scene.and_then(|s| self.project.scenes.get(s)).map(|s| s.patterns.clone()).unwrap_or_default();
        if patterns.is_empty() {
            return Err("the section plays no patterns".into());
        }
        self.transaction("Convert to Clips", |d| {
            for (g, pi) in patterns {
                d.new_clip(g, From::Pattern(pi), start, sec.length)?;
            }
            Ok(())
        })
    }

    /// A copy of clip `c` right after it. One undo step; returns the copy's index.
    pub fn duplicate_clip(&mut self, g: usize, c: usize) -> Result<usize, String> {
        let clip = *self.project.groups.get(g).and_then(|x| x.clips.get(c)).ok_or("no such clip")?;
        let mut k = 0;
        self.transaction("Duplicate Clip", |d| {
            k = d.new_clip(g, From::Clip(clip.pattern), clip.position + clip.length, clip.length)?;
            Ok(())
        })?;
        Ok(k)
    }

    /// Double clip `c`: its pattern twice as long, the notes repeated, and the clip too.
    pub fn double_clip(&mut self, g: usize, c: usize) -> Result<(), String> {
        let clip = *self.project.groups.get(g).and_then(|x| x.clips.get(c)).ok_or("no such clip")?;
        let pat = self.project.groups[g].clip_patterns.get(clip.pattern).ok_or("no such clip")?.clone();
        let len = pat.length.max(1);
        let notes: Vec<Note> = pat.notes.iter().map(|n| Note { position: n.position + len, ..*n }).collect();
        let midi: Vec<MidiEvent> = pat.midi.iter().map(|m| MidiEvent { position: m.position + len, ..*m }).collect();
        self.transaction("Double Clip", |d| {
            d.set_pattern_length(g, CLIP + clip.pattern, len * 2)?;
            let ops = d.event_ops(g, CLIP + clip.pattern, &[], &notes, &[], &midi)?;
            if !ops.is_empty() {
                d.apply("Double Clip", ops)?;
            }
            d.set_clip(g, c, clip.position, clip.length * 2)
        })
    }

    /// Move clip `c` to `position` and make it `length` long.
    pub fn set_clip(&mut self, g: usize, c: usize, position: i64, length: i64) -> Result<(), String> {
        let (events_path, mut events, mut items, i) = self.clip_event(g, c)?;
        let Value::Obj(te) = &mut items[i].event else { return Err("bad clip event".into()) };
        *te.get_mut("position").ok_or("clip without position")? = Value::Int(position.max(0));
        let Some(Value::Obj(ev)) = te.get_mut("event") else { return Err("clip without event".into()) };
        *ev.find_mut("length").ok_or("clip without length")? = Value::Int(length.max(1));
        items.sort_by_key(Item::position);
        events = events_with(&events, &items);
        self.apply("Move Clip", vec![Op::Set { path: events_path, value: Value::Obj(events) }])
    }

    /// Delete clip `c` and its pattern (its notes too). One undo step.
    pub fn delete_clip(&mut self, g: usize, c: usize) -> Result<(), String> {
        let clip = *self.project.groups.get(g).and_then(|x| x.clips.get(c)).ok_or("no such clip")?;
        let pat = self.project.groups[g].clip_patterns.get(clip.pattern).ok_or("no such clip")?.clone();
        self.transaction("Delete Clip", |d| {
            let ops = d.event_ops(g, CLIP + clip.pattern, &pat.notes, &[], &pat.midi, &[])?;
            if !ops.is_empty() {
                d.apply("Delete Clip", ops)?;
            }
            let (events_path, events, mut items, i) = d.clip_event(g, c)?;
            items.remove(i);
            let seq = events_path[..events_path.len() - 1].to_vec();
            let mut ops = vec![Op::Set { path: events_path, value: Value::Obj(events_with(&events, &items)) }];
            ops.push(Op::Set { path: d.field_path(&seq, &["current"])?, value: Value::Null });
            d.apply("Delete Clip", ops)?;
            let (list, _) = d.clip_paths(g)?;
            let mut pats = d.items(&list)?;
            pats.remove(clip.pattern);
            let vector = list[..list.len() - 2].to_vec();
            let mut ops = vec![Op::Set { path: list, value: Value::List(pats) }];
            if let Ok(current) = d.field_path(&vector, &["current"]) {
                ops.push(Op::Set { path: current, value: Value::Null });
            }
            d.apply("Delete Clip", ops)
        })
    }

    /// Clip `c`'s event in its group's lane: the events' path, the events, their items
    /// and its index among them.
    fn clip_event(&self, g: usize, c: usize) -> Result<(Path, Obj, Vec<Item>, usize), String> {
        let clip = *self.project.groups.get(g).and_then(|x| x.clips.get(c)).ok_or("no such clip")?;
        let pat_path = self.addresses.groups[g].clip_patterns.get(clip.pattern).ok_or("no such clip")?;
        let oid = self.oid_at(pat_path)?;
        let (_, seq) = self.clip_paths(g)?;
        let events_path = self.field_path(&seq, &["events"])?;
        let Some(Value::Obj(events)) = self.tree.at(&events_path).cloned() else { return Err("lane without events".into()) };
        let items = items_of(&events)?;
        let paths = self.object_paths();
        let i = items
            .iter()
            .position(|it| self.event_obj(&it.event, &paths).is_some_and(|e| e.get("pattern") == Some(&Value::Ref(oid))))
            .ok_or("clip event not found")?;
        Ok((events_path, events, items, i))
    }

    /// A new clip at `position`, `length` long, its pattern a copy of `from` (with its
    /// notes); returns its index in the group's clips.
    fn new_clip(&mut self, g: usize, from: From, position: i64, length: i64) -> Result<usize, String> {
        let grp = self.project.groups.get(g).ok_or("no such group")?;
        let (pat, template) = match from {
            From::Pattern(pi) => {
                let pat = grp.patterns.get(pi).ok_or("no such pattern")?.clone();
                let items = self.items(&self.pattern_items_path(g)?)?;
                // A pattern entry is (id, shared_ptr); the clip list holds the shared_ptr.
                let item = match items.get(pi) {
                    Some(Value::Obj(e)) => e.get("second").cloned().ok_or("bad pattern entry")?,
                    _ => return Err("no such pattern".into()),
                };
                (pat, item)
            }
            From::Clip(k) => {
                let pat = grp.clip_patterns.get(k).ok_or("no such clip")?.clone();
                let (list, _) = self.clip_paths(g)?;
                (pat, self.items(&list)?.get(k).cloned().ok_or("no such clip")?)
            }
        };
        let (list, seq) = self.clip_paths(g)?;
        let mut item = self.fresh_copy(&template);
        let oid = {
            let Value::Obj(sp) = &mut item else { return Err("bad pattern".into()) };
            let Some(Value::Obj(px)) = sp.get_mut("px") else { return Err("bad pattern".into()) };
            unshare_sequences(px)?;
            // Named as Maschine names new clips.
            let name = format!("Clip {}", self.project.groups[g].clip_patterns.len() + 1);
            if let Some(Value::Obj(n)) = px.get_mut("name") {
                for k in ["value", "t0"] {
                    if let Some(v @ Value::Str(_)) = n.find_mut(k) {
                        *v = Value::Str(name.clone());
                    }
                }
            }
            px.oid.ok_or("pattern without id")?
        };
        let mut pats = self.items(&list)?;
        let k = pats.len();
        pats.push(item);
        self.apply("Convert to Clip", vec![Op::Set { path: list, value: Value::List(pats) }])?;
        let ops = self.event_ops(g, CLIP + k, &[], &pat.notes, &[], &pat.midi)?;
        if !ops.is_empty() {
            self.apply("Convert to Clip", ops)?;
        }
        let events_path = self.field_path(&seq, &["events"])?;
        let Some(Value::Obj(events)) = self.tree.at(&events_path).cloned() else { return Err("lane without events".into()) };
        let mut items = items_of(&events)?;
        let event = self.build(|b| b.clip_event(position.max(0), length.max(1), oid))?;
        items.push(Item { event, node: self.new_node()? });
        items.sort_by_key(Item::position);
        self.apply("Convert to Clip", vec![Op::Set { path: events_path, value: Value::Obj(events_with(&events, &items)) }])?;
        let clips = &self.project.groups[g].clips;
        clips.iter().position(|c| c.pattern == k).ok_or_else(|| "the clip didn't land".into())
    }
}

/// A copied pattern's sound sequences cleared (each sound's sequence lives in the sound
/// and would be shared); its notes are added afresh.
fn unshare_sequences(pat: &mut Obj) -> Result<(), String> {
    let Some(Value::Obj(ss)) = pat.get_mut("sound_sequences") else { return Ok(()) };
    let Some(Value::Obj(objs)) = ss.get_mut("objects") else { return Ok(()) };
    let Some(Value::List(entries)) = objs.get_mut("items") else { return Ok(()) };
    for e in entries {
        let Value::Obj(o) = e else { return Err("bad pattern entry".into()) };
        let Some(Value::Obj(r)) = o.get_mut("reference") else { return Err("a pattern entry without a reference".into()) };
        let Some(Value::Obj(sr)) = r.get_mut("px") else { return Err("a sequence reference defined elsewhere".into()) };
        if let Some(seq) = sr.get_mut("sequence") {
            *seq = Value::Null;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;
    use deckard_model::{Clip, Note, PPQ};

    fn note(sound: usize, position: i64, pitch: i64) -> Note {
        Note { expr: None, sound, position, length: PPQ / 2, pitch, velocity: 100, probability: 100 }
    }

    /// A new project: pattern 1 of group 1 with notes on two sounds, in scene 1, and
    /// two sections of it.
    fn doc() -> Document {
        let mut d = Document::new_project().unwrap();
        if d.project().groups[0].patterns.is_empty() {
            d.add_pattern(0, usize::MAX, "Pattern 1").unwrap();
        }
        d.edit_notes("Notes", 0, 0, &[], &[note(0, 0, 36), note(1, PPQ, 38)]).unwrap();
        d.assign_pattern(0, 0, Some(0)).unwrap();
        d.add_section(0).unwrap();
        d.add_section(0).unwrap();
        d
    }

    /// Convert to Clip: a copy of the pattern placed at the section, its notes its own;
    /// saved and opened again it's there; undo takes it away.
    #[test]
    fn a_pattern_becomes_a_clip() {
        let mut d = doc();
        let bar = d.project().bar_length();
        let pattern = d.project().groups[0].patterns[0].clone();
        assert_eq!(d.convert_to_clip(0, 0, bar, bar).unwrap(), 0);
        let look = |d: &Document| {
            let g = &d.project().groups[0];
            assert_eq!(g.clips, [Clip { position: bar, length: bar, pattern: 0 }]);
            assert_eq!(g.clip_patterns[0].notes, pattern.notes);
            assert_eq!(g.clip_patterns[0].length, pattern.length);
            assert_eq!(g.clip_patterns[0].name, "Clip 1");
        };
        look(&d);
        assert_eq!(d.undo_labels().first().copied(), Some("Convert to Clip"));
        // Its notes are its own.
        d.edit_notes("Notes", 0, CLIP, &[note(0, 0, 36)], &[note(0, 0, 40)]).unwrap();
        assert_eq!(d.project().groups[0].patterns[0].notes, pattern.notes, "the pattern stays");
        assert_eq!(d.project().groups[0].clip_patterns[0].notes[0].pitch, 40);
        d.undo();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let path = dir.join("clips.mxprj");
        d.save_as(&path).unwrap();
        look(&Document::open(&path).unwrap());
        d.undo();
        assert!(d.project().groups[0].clips.is_empty() && d.project().groups[0].clip_patterns.is_empty());
    }

    /// Section ▸ Convert to Clip(s): each pattern of the section's scene, a clip over the
    /// section.
    #[test]
    fn a_section_becomes_clips() {
        let mut d = doc();
        let bar = d.project().bar_length();
        d.convert_section_to_clips(1).unwrap();
        assert_eq!(d.project().groups[0].clips, [Clip { position: bar, length: bar, pattern: 0 }]);
        assert_eq!(d.undo_labels().first().copied(), Some("Convert to Clips"));
    }

    /// Moving, resizing, doubling, duplicating and deleting a clip.
    #[test]
    fn editing_clips() {
        let mut d = doc();
        let bar = d.project().bar_length();
        d.convert_to_clip(0, 0, 0, bar).unwrap();
        d.set_clip(0, 0, 2 * bar, 3 * bar).unwrap();
        assert_eq!(d.project().groups[0].clips, [Clip { position: 2 * bar, length: 3 * bar, pattern: 0 }]);
        assert_eq!(d.duplicate_clip(0, 0).unwrap(), 1);
        let g = &d.project().groups[0];
        assert_eq!(g.clips[1], Clip { position: 5 * bar, length: 3 * bar, pattern: 1 });
        assert_eq!(g.clip_patterns[1].notes, g.clip_patterns[0].notes, "a copy, notes and all");
        d.double_clip(0, 1).unwrap();
        let g = &d.project().groups[0];
        assert_eq!(g.clips[1].length, 6 * bar);
        assert_eq!(g.clip_patterns[1].length, 2 * bar);
        assert_eq!(g.clip_patterns[1].notes.len(), 4, "its notes repeated");
        d.delete_clip(0, 0).unwrap();
        let g = &d.project().groups[0];
        assert_eq!(g.clips, [Clip { position: 5 * bar, length: 6 * bar, pattern: 0 }]);
        assert_eq!(g.clip_patterns.len(), 1);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("clips-edit.mxprj");
        d.save_as(&path).unwrap();
        let again = Document::open(&path).unwrap();
        assert_eq!(again.project().groups[0].clips, d.project().groups[0].clips);
    }

    /// In a project Maschine saved with clips (read from it, written to a copy).
    #[test]
    fn clips_in_a_maschine_project() {
        let Ok(mut d) = Document::open(std::path::Path::new("/Users/Shared/Mother Board Library/Projects/Aardvark.mxprj")) else { return };
        let before = d.project().groups[0].clips.clone();
        let sec = (0..d.project().sections.len())
            .find(|&s| d.project().sections[s].scene.is_some_and(|x| !d.project().scenes[x].patterns.is_empty()))
            .unwrap();
        d.convert_section_to_clips(sec).unwrap();
        d.delete_clip(0, 0).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("clips-aardvark.mxprj");
        d.save_as(&path).unwrap();
        let again = Document::open(&path).unwrap();
        for (a, b) in again.project().groups.iter().zip(&d.project().groups) {
            assert_eq!(a.clips, b.clips);
            assert_eq!(a.clip_patterns.len(), b.clip_patterns.len());
        }
        assert_ne!(again.project().groups[0].clips, before);
    }
}
