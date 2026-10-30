//! Edits from the context menus of sounds, groups and patterns
//! (docs/context-menus.md): colour, a sound's copy / paste / reset, and a pattern's
//! clear / double / duplicate / delete.

use super::{Document, Op, Target};
use deckard_model::{Color, MidiEvent, Note};
use mxprj::decode::{Path, Value};

/// A copied sound: the sound object as stored, its name, and its notes and MIDI
/// events in each pattern of its group (by pattern index).
#[derive(Debug, Clone)]
pub struct SoundClip {
    pub name: String,
    sound: Value,
    events: Vec<(usize, Vec<Note>, Vec<MidiEvent>)>,
}

impl SoundClip {
    /// The clip copied from document `from`, for pasting in `to` (its notes'
    /// expression carried over).
    pub fn carried(mut self, from: &Document, to: &mut Document) -> SoundClip {
        for (_, notes, _) in &mut self.events {
            for n in notes.iter_mut() {
                *n = to.carry_note(from, *n);
            }
        }
        self
    }
}

impl Document {
    /// Colour sounds, groups, patterns, scenes or sections (a palette slot, or the
    /// default colour), as one undo step.
    pub fn set_color(&mut self, targets: &[Target], color: Color) -> Result<(), String> {
        let mut ops = Vec::new();
        for &t in targets {
            let owner = self.owner(t).ok_or("no such object")?;
            let Some(Value::Obj(o)) = self.tree.at(&owner) else { return Err("no such object".into()) };
            let cp = o.find_path("color").ok_or("it has no colour")?;
            let base = [owner.as_slice(), &cp].concat();
            let Some(Value::Obj(c)) = self.tree.at(&base) else { return Err("bad colour".into()) };
            let (index, default) = match color {
                Color::Palette(i) => (i as i64, 0),
                Color::Default => (c.find("color").and_then(Value::as_int).unwrap_or(0), 1),
            };
            for (field, v) in [("color", index), ("use_default_color", default)] {
                if let Some(fp) = c.find_path(field) {
                    ops.push(Op::Set { path: [base.as_slice(), &fp].concat(), value: Value::Int(v) });
                }
            }
        }
        self.apply("Color", ops)
    }

    /// Copy sound `s` of group `g` (with its notes in each of the group's patterns).
    pub fn copy_sound(&self, g: usize, s: usize) -> Result<SoundClip, String> {
        let path = self.owner(Target::Sound(g, s)).ok_or("no such sound")?;
        let sound = self.tree.at(&path).cloned().ok_or("no such sound")?;
        let group = &self.project.groups[g];
        let events = group
            .patterns
            .iter()
            .enumerate()
            .map(|(pi, p)| {
                let notes: Vec<Note> = p.notes.iter().filter(|n| n.sound == s).copied().collect();
                let midi: Vec<MidiEvent> = p.midi.iter().filter(|m| m.sound == s).copied().collect();
                (pi, notes, midi)
            })
            .filter(|(_, n, m)| !n.is_empty() || !m.is_empty())
            .collect();
        Ok(SoundClip { name: group.sounds[s].name.clone(), sound, events })
    }

    /// Paste copied sounds into slot `s` of group `g` and the slots after it (those
    /// past the group's last slot are left out): each replaces the slot's sound, and
    /// its notes replace the slot's in the patterns with the same index. One undo step.
    pub fn paste_sounds(&mut self, g: usize, s: usize, clips: &[SoundClip]) -> Result<(), String> {
        let n = self.project.groups.get(g).ok_or("no such group")?.sounds.len();
        let label = if clips.len() > 1 { "Paste Sounds" } else { "Paste Sound" };
        self.transaction(label, |d| {
            for (k, clip) in clips.iter().enumerate().take(n.saturating_sub(s)) {
                d.replace_sound(g, s + k, clip)?;
            }
            Ok(())
        })
    }

    fn replace_sound(&mut self, g: usize, s: usize, clip: &SoundClip) -> Result<(), String> {
        let path = self.owner(Target::Sound(g, s)).ok_or("no such sound")?;
        let Some(Value::Obj(old)) = self.tree.at(&path).cloned() else { return Err("no such sound".into()) };
        let Value::Obj(mut new) = self.fresh_copy(&clip.sound) else { return Err("bad sound".into()) };
        // The slot keeps its identity (patterns refer to it) and its own sequences.
        new.oid = old.oid;
        new.export = old.export.clone();
        if let (Some(i), Some(seq)) = (new.fields.iter().position(|(f, _)| f == "sequences"), old.get("sequences")) {
            new.fields[i].1 = seq.clone();
        }
        self.apply("Paste Sound", vec![Op::Set { path, value: Value::Obj(new) }])?;
        // Its notes: the clip's replace the slot's, pattern by pattern.
        let patterns = self.project.groups[g].patterns.clone();
        for (pi, p) in patterns.iter().enumerate() {
            let remove: Vec<Note> = p.notes.iter().filter(|x| x.sound == s).copied().collect();
            let remove_midi: Vec<MidiEvent> = p.midi.iter().filter(|x| x.sound == s).copied().collect();
            let (add, add_midi) = clip
                .events
                .iter()
                .find(|e| e.0 == pi)
                .map(|e| {
                    (
                        e.1.iter().map(|x| Note { sound: s, ..*x }).collect::<Vec<_>>(),
                        e.2.iter().map(|x| MidiEvent { sound: s, ..*x }).collect::<Vec<_>>(),
                    )
                })
                .unwrap_or_default();
            if remove.is_empty() && add.is_empty() && remove_midi.is_empty() && add_midi.is_empty() {
                continue;
            }
            let ops = self.event_ops(g, pi, &remove, &add, &remove_midi, &add_midi)?;
            if !ops.is_empty() {
                self.apply("Paste Sound", ops)?;
            }
        }
        Ok(())
    }

    /// Empty sound `s` of group `g`, as Maschine's Reset: no plug-ins, level, pan, mute
    /// and cue back to their defaults, named "Sound N", the default colour, and no
    /// notes. One undo step.
    pub fn reset_sound(&mut self, g: usize, s: usize) -> Result<(), String> {
        self.transaction("Reset Sound", |d| {
            let t = Target::Sound(g, s);
            for slot in (0..d.project.groups[g].sounds[s].slots.len()).rev() {
                d.remove_slot(g, s, slot)?;
            }
            d.set_level(t, 1.0)?;
            d.set_pan(t, 0.0)?;
            d.set_mute(t, false)?;
            d.set_cue(t, false)?;
            d.rename(t, &format!("Sound {}", s + 1))?;
            d.set_color(&[t], Color::Default)?;
            d.clear_sound_events(g, s)?;
            d.fix_lfo_triggers(|x, y| ((x, y) != (g, s)).then_some((x, y)))
        })
    }

    /// Remove sound `s`'s notes and MIDI events from every pattern of group `g`.
    pub fn clear_sound_events(&mut self, g: usize, s: usize) -> Result<(), String> {
        let patterns = self.project.groups.get(g).ok_or("no such group")?.patterns.clone();
        for (pi, p) in patterns.iter().enumerate() {
            let notes: Vec<Note> = p.notes.iter().filter(|x| x.sound == s).copied().collect();
            let midi: Vec<MidiEvent> = p.midi.iter().filter(|x| x.sound == s).copied().collect();
            if notes.is_empty() && midi.is_empty() {
                continue;
            }
            let ops = self.event_ops(g, pi, &notes, &[], &midi, &[])?;
            if !ops.is_empty() {
                self.apply("Clear Events", ops)?;
            }
        }
        Ok(())
    }

    /// Remove every note and MIDI event from pattern `p` of group `g` (it stays).
    pub fn clear_pattern(&mut self, g: usize, p: usize) -> Result<(), String> {
        let pat = self.project.groups.get(g).and_then(|x| x.patterns.get(p)).ok_or("no such pattern")?.clone();
        let ops = self.event_ops(g, p, &pat.notes, &[], &pat.midi, &[])?;
        if ops.is_empty() {
            return Ok(());
        }
        self.apply("Clear Pattern", ops)
    }

    /// Double pattern `p`: twice as long, its notes repeated in the second half.
    pub fn double_pattern(&mut self, g: usize, p: usize) -> Result<(), String> {
        let pat = self.project.groups.get(g).and_then(|x| x.patterns.get(p)).ok_or("no such pattern")?.clone();
        let len = pat.length.max(1);
        let notes: Vec<Note> = pat.notes.iter().map(|n| Note { position: n.position + len, ..*n }).collect();
        let midi: Vec<MidiEvent> = pat.midi.iter().map(|m| MidiEvent { position: m.position + len, ..*m }).collect();
        self.transaction("Double Pattern", |d| {
            d.set_pattern_length(g, p, len * 2)?;
            let ops = d.event_ops(g, p, &[], &notes, &[], &midi)?;
            if !ops.is_empty() {
                d.apply("Double Pattern", ops)?;
            }
            Ok(())
        })
    }

    /// A copy of pattern `p` (its notes too) right after it; returns its index.
    pub fn duplicate_pattern(&mut self, g: usize, p: usize) -> Result<usize, String> {
        let pat = self.project.groups.get(g).and_then(|x| x.patterns.get(p)).ok_or("no such pattern")?.clone();
        let mut at = 0;
        self.transaction("Duplicate Pattern", |d| {
            at = d.insert_pattern(g, p, p + 1, &pat.name)?;
            let start = d.project.groups[g].patterns[at].start;
            let shift = |pos: i64| pos - pat.start + start;
            let notes: Vec<Note> = pat.notes.iter().map(|n| Note { position: shift(n.position), ..*n }).collect();
            let midi: Vec<MidiEvent> = pat.midi.iter().map(|m| MidiEvent { position: shift(m.position), ..*m }).collect();
            let ops = d.event_ops(g, at, &[], &notes, &[], &midi)?;
            if !ops.is_empty() {
                d.apply("Duplicate Pattern", ops)?;
            }
            Ok(())
        })?;
        Ok(at)
    }

    /// Delete pattern `p` of group `g`: scenes that played it play nothing for the
    /// group, and the patterns after it move up.
    pub fn delete_pattern(&mut self, g: usize, p: usize) -> Result<(), String> {
        let scenes: Vec<usize> =
            self.project.scenes.iter().enumerate().filter(|(_, sc)| sc.patterns.contains(&(g, p))).map(|(i, _)| i).collect();
        self.transaction("Delete Pattern", |d| {
            for sc in scenes {
                d.assign_pattern(sc, g, None)?;
            }
            d.clear_pattern(g, p)?;
            d.remove_pattern_entry(g, p)
        })
    }

    /// The path of group `g`'s pattern list (`patterns_by_id` items).
    pub(crate) fn pattern_items_path(&self, g: usize) -> Result<Path, String> {
        let ga = self.addresses.groups.get(g).ok_or("no such group")?.clone();
        self.field_path(&ga.group, &["patterns_by_id", "objects", "items"])
    }
}

/// The id (`first`) of a `patterns_by_id` entry.
pub(crate) fn entry_id(e: &Value) -> Option<i64> {
    match e {
        Value::Obj(o) => match o.get("first") {
            Some(Value::Int(i)) => Some(*i),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn set_entry_id(e: &mut Value, id: i64) {
    if let Value::Obj(o) = e
        && let Some(f) = o.get_mut("first")
    {
        *f = Value::Int(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    fn open() -> Option<Document> {
        Document::open(std::path::Path::new("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj")).ok()
    }

    /// Save to a temporary file and read it back.
    fn reload(d: &Document) -> Document {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("p.mxprj");
        std::fs::write(&file, d.to_bytes().unwrap()).unwrap();
        Document::open(&file).unwrap()
    }

    fn consistent(d: &Document) {
        let (p, a) = deckard_model::Project::read(d.root());
        assert!(p == *d.project(), "patched model differs from a rebuild");
        assert!(a == *d.addresses());
    }

    #[test]
    fn color_copy_paste_and_reset_sounds() {
        let Some(mut d) = open() else { return };
        let before = d.project().clone();
        d.set_color(&[Target::Sound(0, 1), Target::Group(2)], Color::Palette(9)).unwrap();
        assert_eq!(d.project().groups[0].sounds[1].color, Color::Palette(9));
        assert_eq!(d.project().groups[2].color, Color::Palette(9));
        d.set_color(&[Target::Sound(0, 1)], Color::Default).unwrap();
        assert_eq!(d.project().groups[0].sounds[1].color, Color::Default);

        // Copy sound 1 (Snares' first sound) over sound 0, and sound 0 of group 1 over
        // the empty sound 5 of group 0 (a slot later in the file than its source).
        let snare = d.copy_sound(1, 0).unwrap();
        let kick = d.copy_sound(0, 0).unwrap();
        assert!(!snare.events.is_empty());
        d.paste_sounds(0, 5, std::slice::from_ref(&snare)).unwrap();
        d.paste_sounds(1, 3, std::slice::from_ref(&kick)).unwrap();
        consistent(&d);
        let check = |d: &Document| {
            let p = d.project();
            assert_eq!(p.groups[0].sounds[5].name, before.groups[1].sounds[0].name);
            assert_eq!(p.groups[0].sounds[5].slots, before.groups[1].sounds[0].slots);
            assert_eq!(p.groups[1].sounds[3].sampler, before.groups[0].sounds[0].sampler);
            // The notes came along, in the patterns with the same index.
            for (pi, pat) in before.groups[1].patterns.iter().enumerate().take(p.groups[0].patterns.len()) {
                let want: Vec<(i64, i64)> = pat.notes.iter().filter(|n| n.sound == 0).map(|n| (n.position - pat.start, n.pitch)).collect();
                let tp = &p.groups[0].patterns[pi];
                let got: Vec<(i64, i64)> = tp.notes.iter().filter(|n| n.sound == 5).map(|n| (n.position - tp.start, n.pitch)).collect();
                assert_eq!(got, want, "pattern {pi}");
            }
            // The originals are unchanged.
            assert_eq!(p.groups[1].sounds[0], before.groups[1].sounds[0]);
            assert_eq!(
                p.groups[1].patterns[0].notes.iter().filter(|n| n.sound == 0).count(),
                before.groups[1].patterns[0].notes.iter().filter(|n| n.sound == 0).count()
            );
        };
        check(&d);
        check(&reload(&d));

        // Reset: empty, default name and colour, no notes; one undo step.
        let steps = d.undo_labels().len();
        let pasted = d.project().clone();
        d.reset_sound(0, 0).unwrap();
        assert_eq!(d.undo_labels().len(), steps + 1);
        let s = &d.project().groups[0].sounds[0];
        assert!(s.slots.is_empty() && s.sampler.is_none());
        assert_eq!((s.name.as_str(), s.color, s.level, s.muted), ("Sound 1", Color::Default, 1.0, false));
        assert!(d.project().groups[0].patterns.iter().all(|p| p.notes.iter().all(|n| n.sound != 0)));
        consistent(&d);
        let back = reload(&d);
        assert!(back.project().groups[0].sounds[0].slots.is_empty());
        assert!(d.undo());
        assert_eq!(d.project().groups[0], pasted.groups[0]);
    }

    #[test]
    fn clear_double_duplicate_and_delete_patterns() {
        let Some(mut d) = open() else { return };
        let before = d.project().clone();
        let g = 0;
        let n = before.groups[g].patterns.len();
        let p0 = before.groups[g].patterns[0].clone();
        assert!(!p0.notes.is_empty());

        d.double_pattern(g, 0).unwrap();
        let p = &d.project().groups[g].patterns[0];
        assert_eq!((p.length, p.notes.len()), (p0.length * 2, p0.notes.len() * 2));
        assert!(d.undo());
        assert_eq!(d.project().groups[g].patterns[0], p0);

        let at = d.duplicate_pattern(g, 0).unwrap();
        assert_eq!(at, 1);
        let q = d.project().groups[g].clone();
        assert_eq!(q.patterns.len(), n + 1);
        assert_eq!(q.patterns[1].length, p0.length);
        let rel = |p: &deckard_model::Pattern| p.notes.iter().map(|x| (x.sound, x.position - p.start, x.pitch)).collect::<Vec<_>>();
        assert_eq!(rel(&q.patterns[1]), rel(&p0));
        assert_eq!(q.patterns[2], before.groups[g].patterns[1], "the next pattern moved on one");
        consistent(&d);
        assert_eq!(reload(&d).project().groups[g].patterns.len(), n + 1);

        d.clear_pattern(g, 1).unwrap();
        assert!(d.project().groups[g].patterns[1].notes.is_empty());
        assert_eq!(d.project().groups[g].patterns[0].notes, p0.notes);

        // Delete a pattern a scene plays: those scenes play nothing for the group, and
        // the patterns after it move up.
        let dp = before.scenes.iter().flat_map(|s| s.patterns.iter()).find(|x| x.0 == g).unwrap().1;
        let dp_now = if dp >= 1 { dp + 1 } else { dp };
        let scenes_now = d.project().scenes.clone();
        let users: Vec<usize> = scenes_now.iter().enumerate().filter(|(_, s)| s.patterns.contains(&(g, dp_now))).map(|(i, _)| i).collect();
        assert!(!users.is_empty());
        let after = d.project().groups[g].patterns.get(dp_now + 1).cloned();
        d.delete_pattern(g, dp_now).unwrap();
        assert_eq!(d.project().groups[g].patterns.len(), n);
        for &sc in &users {
            assert!(!d.project().scenes[sc].patterns.iter().any(|x| x.0 == g), "scene {sc} plays nothing for the group");
        }
        assert_eq!(d.project().groups[g].patterns.get(dp_now).cloned(), after);
        consistent(&d);
        let back = reload(&d);
        assert_eq!(back.project().groups[g].patterns.len(), n);
        assert_eq!(back.project().scenes, d.project().scenes);
        assert!(d.undo());
        assert_eq!(d.project().groups[g].patterns.len(), n + 1);
        assert_eq!(d.project().scenes, scenes_now);
    }
}
