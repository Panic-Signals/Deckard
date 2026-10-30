//! Per-note expression in the document. A note's [`ExprId`] lives in the tree as a
//! DECKARD-only field of its `TimedEvent` ([`EXPR_FIELD`]), so every edit, copy and
//! undo carries it with the note; the [`Expression`]s sit in an append-only table, so an
//! id never changes meaning. The field never reaches Maschine's archive: saving strips
//! it (and the curves go in the DECKARD block, `docs/deckard-block.md`).

use super::Document;
use deckard_model::expression::EXPR_FIELD;
use deckard_model::{ExprId, Expression, Note};
use mxprj::decode::Value;
use std::sync::Arc;

impl Document {
    /// Keep `e` in the table: its id, for a note's `expr`.
    pub fn add_expression(&mut self, e: Expression) -> ExprId {
        let e = Arc::new(e);
        self.expressions.push(e.clone());
        self.project.expressions.push(e);
        ExprId(self.expressions.len() as u32 - 1)
    }

    pub fn expression(&self, id: ExprId) -> Option<&Expression> {
        self.expressions.get(id.0 as usize).map(|e| &**e)
    }

    /// Note `n` of document `from`, for pasting here: its expression added to this
    /// document's table (an id is only good in its own document).
    pub fn carry_note(&mut self, from: &Document, n: Note) -> Note {
        let expr = n.expr.and_then(|id| from.expression(id)).cloned().map(|e| self.add_expression(e));
        Note { expr, ..n }
    }

    /// Notes of group `g`'s pattern `p` given new expression (the Expression editor): each
    /// gets a new id (none for an empty one), all in one undo step. The notes as they
    /// are now, in `edits`' order.
    pub fn set_expressions(&mut self, g: usize, p: usize, edits: &[(Note, Expression)]) -> Result<Vec<Note>, String> {
        let (mut remove, mut add) = (Vec::new(), Vec::new());
        for (n, e) in edits {
            let expr = (!e.is_empty()).then(|| self.add_expression(e.clone()));
            remove.push(*n);
            add.push(Note { expr, ..*n });
        }
        self.edit_notes("Edit Expression", g, p, &remove, &add)?;
        // Each edit is a step of its own: the next (on any note of the sound) doesn't merge.
        if let Some(last) = self.undo.last_mut().filter(|s| s.label == "Edit Expression") {
            last.at -= crate::MERGE_WINDOW * 2;
        }
        Ok(add)
    }

    /// Split [`crate::note_tools::split`]'s way, each part getting its slice of the
    /// curves.
    pub fn split_notes(&mut self, targets: &[Note], at: i64) -> crate::note_tools::Change {
        let (cut, mut made) = crate::note_tools::split(targets, at);
        for (n, parts) in cut.iter().zip(made.chunks_mut(2)) {
            let Some(e) = n.expr.and_then(|id| self.expression(id)).cloned() else { continue };
            let mid = at - n.position;
            parts[0].expr = Some(self.add_expression(e.slice(0, mid)));
            parts[1].expr = Some(self.add_expression(e.slice(mid, n.length)));
        }
        (cut, made)
    }

    /// Join [`crate::note_tools::join`]'s way, the joined note's curves the two notes'
    /// one after the other.
    pub fn join_notes(&mut self, all: &[Note], clicked: Note, selected: &[Note]) -> crate::note_tools::Change {
        let (gone, mut made) = crate::note_tools::join(all, clicked, selected);
        for (pair, n) in gone.chunks(2).zip(made.iter_mut()) {
            let [a, b] = [pair[0], pair[1]];
            let (ea, eb) = (a.expr.and_then(|i| self.expression(i)), b.expr.and_then(|i| self.expression(i)));
            if ea.is_none() && eb.is_none() {
                n.expr = None;
                continue;
            }
            let empty = Expression::default();
            let joined = Expression::join(ea.unwrap_or(&empty), b.position - a.position, eb.unwrap_or(&empty));
            n.expr = Some(self.add_expression(joined));
        }
        (gone, made)
    }
}

/// A note's `TimedEvent` with its expression id set (or cleared).
pub(crate) fn set_expr_field(te: &mut mxprj::decode::Obj, expr: Option<ExprId>) {
    te.fields.retain(|(f, _)| f != EXPR_FIELD);
    if let Some(id) = expr {
        te.fields.push((EXPR_FIELD.into(), Value::Int(id.0 as i64)));
    }
}

/// Take every DECKARD-only field out of `v` (before it's written as Maschine's archive).
pub(crate) fn strip_deckard_fields(v: &mut Value) {
    v.walk_mut(&mut |o| o.fields.retain(|(f, _)| !f.starts_with("deckard:")));
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_model::{Curve, Point};

    fn ramp(len: i64) -> Expression {
        let points = vec![Point { at: 0, value: 0.0, curve: 0.0 }, Point { at: len, value: 1.0, curve: 0.0 }];
        Expression { pressure: Some(Curve { points }), ..Expression::default() }
    }

    /// A new project: group 0's pattern 0 holding one note with a pressure ramp.
    fn doc() -> (Document, Note) {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "Pattern 1").unwrap();
        let id = d.add_expression(ramp(960));
        let n = Note { sound: 0, position: 0, length: 960, pitch: 60, velocity: 100, probability: 100, expr: Some(id) };
        d.edit_notes("t", 0, 0, &[], &[n]).unwrap();
        (d, n)
    }

    /// The project carries the document's expression table (for the engine): by id,
    /// through edits and undo; a new project has none.
    #[test]
    fn project_carries_the_expression_table() {
        assert!(Document::new_project().unwrap().project().expressions.is_empty());
        let (mut d, n) = doc();
        let id = n.expr.unwrap();
        assert_eq!(*d.project().expressions[id.0 as usize], ramp(960));
        d.edit_notes("t", 0, 0, &[n], &[Note { pitch: 62, ..n }]).unwrap();
        assert!(d.undo());
        assert_eq!(*d.project().expressions[id.0 as usize], ramp(960));
        let id2 = d.add_expression(ramp(480));
        assert_eq!(*d.project().expressions[id2.0 as usize], ramp(480));
    }

    fn notes(d: &Document, p: usize) -> Vec<Note> {
        d.project().groups[0].patterns[p].notes.clone()
    }

    #[test]
    fn expression_survives_move_transpose_quantize() {
        let (mut d, n) = doc();
        assert_eq!(notes(&d, 0), vec![n], "read back with its expr");
        let moved = Note { position: 480, pitch: 62, ..n };
        d.edit_notes("Move", 0, 0, &[n], &[moved]).unwrap();
        assert_eq!(notes(&d, 0), vec![moved]);
        assert_eq!(d.expression(moved.expr.unwrap()), Some(&ramp(960)));
    }

    #[test]
    fn expression_survives_copy_and_duplicate() {
        let (mut d, n) = doc();
        let copy = Note { position: 1920, ..n };
        d.edit_notes("Paste", 0, 0, &[], &[copy]).unwrap();
        let dup = d.duplicate_pattern(0, 0).unwrap();
        assert!(notes(&d, dup).iter().all(|m| m.expr == n.expr), "duplicate: {:?}", notes(&d, dup));
        d.double_pattern(0, 0).unwrap();
        assert!(notes(&d, 0).len() == 4 && notes(&d, 0).iter().all(|m| m.expr == n.expr));
        let clip = d.copy_sound(0, 0).unwrap();
        d.paste_sounds(0, 1, &[clip]).unwrap();
        let pasted: Vec<Note> = notes(&d, 0).into_iter().filter(|m| m.sound == 1).collect();
        assert!(!pasted.is_empty() && pasted.iter().all(|m| m.expr == n.expr), "pasted sound: {pasted:?}");
        let k = d.convert_to_clip(0, 0, 0, 3840).unwrap();
        let clip_notes = &d.project().groups[0].clip_patterns[k].notes;
        assert!(!clip_notes.is_empty() && clip_notes.iter().all(|m| m.expr == n.expr), "clip: {clip_notes:?}");
    }

    #[test]
    fn split_slices_and_join_joins() {
        let (mut d, n) = doc();
        let (gone, made) = d.split_notes(&[n], 480);
        d.edit_notes("Split", 0, 0, &gone, &made).unwrap();
        let halves = notes(&d, 0);
        assert_eq!(halves.len(), 2);
        let curve = |d: &Document, m: &Note| d.expression(m.expr.unwrap()).unwrap().pressure.clone().unwrap();
        assert_eq!((curve(&d, &halves[0]).value_at(0), curve(&d, &halves[0]).value_at(480)), (0.0, 0.5));
        assert_eq!((curve(&d, &halves[1]).value_at(0), curve(&d, &halves[1]).value_at(480)), (0.5, 1.0));
        let (gone, made) = d.join_notes(&halves, halves[0], &[]);
        d.edit_notes("Join", 0, 0, &gone, &made).unwrap();
        let joined = notes(&d, 0);
        assert_eq!(joined.len(), 1);
        let (c, orig) = (curve(&d, &joined[0]), ramp(960).pressure.unwrap());
        for t in (0..=960).step_by(120) {
            assert!((c.value_at(t) - orig.value_at(t)).abs() < 1e-6, "at {t}");
        }
    }

    #[test]
    fn delete_drops_and_undo_restores() {
        let (mut d, n) = doc();
        d.edit_notes("Delete", 0, 0, &[n], &[]).unwrap();
        assert!(notes(&d, 0).is_empty());
        assert!(d.undo());
        assert_eq!(notes(&d, 0), vec![n]);
    }

    #[test]
    fn deckard_fields_never_reach_the_archive() {
        let (d, _) = doc();
        // Maschine's archive as Maschine reads it (not reopened: that takes the block in).
        let saved = d.to_bytes().unwrap();
        let archive = mxprj::Project::parse(&saved).unwrap().decode(&mxprj::types::Types::new()).unwrap();
        let (mut found, mut notes) = (false, 0);
        archive.walk_paths(&mut |o, _| {
            found |= o.fields.iter().any(|(f, _)| f.starts_with("deckard:"));
            notes += (o.type_name == "NI::MASCOMMON::DATA::TimedEvent") as usize;
        });
        assert!(notes > 0 && !found);
    }

    /// Final review #1: a group (or sound) whose notes have expression saves as a file.
    #[test]
    fn a_group_with_expression_saves_as_a_file() {
        let (d, _) = doc();
        let info = mxprj::sound_info::SoundInfo::default();
        d.file_bytes(crate::FileItem::Group(0), &info).unwrap();
        d.file_bytes(crate::FileItem::Sound(0, 0), &info).unwrap();
    }

    /// Final review #3: a note copied in one project and pasted in another keeps its
    /// curves (its id is the other table's).
    #[test]
    fn a_note_carried_to_another_project_keeps_its_curves() {
        let (from, n) = doc();
        let mut to = Document::new_project().unwrap();
        let other = to.add_expression(Expression { release_velocity: Some(9), ..Expression::default() });
        assert_eq!(Some(other), n.expr, "the same id means another curve there");
        let carried = to.carry_note(&from, n);
        assert_eq!(to.expression(carried.expr.unwrap()), Some(&ramp(960)));
        assert_eq!(to.carry_note(&from, Note { expr: None, ..n }).expr, None);
    }

    #[test]
    fn saving_without_expression_is_byte_identical() {
        let (mut d, n) = doc();
        let plain = Note { expr: None, ..n };
        d.edit_notes("Plain", 0, 0, &[n], &[plain]).unwrap();
        let once = d.to_bytes().unwrap();
        let again = Document::from_bytes(once.clone(), None).unwrap().to_bytes().unwrap();
        assert_eq!(once, again);
    }

    fn bent_ramp() -> Expression {
        let mut e = ramp(960);
        e.pressure.as_mut().unwrap().bend(0, 0.5);
        e.slide = Some(Curve { points: vec![Point { at: 100, value: 0.25, curve: 0.0 }] });
        e
    }

    #[test]
    fn an_edit_is_one_undo_step() {
        let (mut d, n) = doc();
        d.set_expressions(0, 0, &[(n, bent_ramp())]).unwrap();
        let now = notes(&d, 0);
        assert_eq!(now.len(), 1);
        assert_ne!(now[0].expr, n.expr, "a new id: the old one keeps meaning the old curves");
        assert_eq!(d.expression(now[0].expr.unwrap()), Some(&bent_ramp()));
        assert!(d.undo());
        assert_eq!(notes(&d, 0), vec![n]);
        assert_eq!(d.expression(n.expr.unwrap()), Some(&ramp(960)));
    }

    #[test]
    fn clearing_everything_clears_the_id() {
        let (mut d, n) = doc();
        d.set_expressions(0, 0, &[(n, Expression::default())]).unwrap();
        assert_eq!(notes(&d, 0), vec![Note { expr: None, ..n }]);
    }

    #[test]
    fn edits_round_trip_through_a_save() {
        let (mut d, n) = doc();
        d.set_expressions(0, 0, &[(n, bent_ramp())]).unwrap();
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        let m = notes(&back, 0)[0];
        assert_eq!(back.expression(m.expr.unwrap()), Some(&bent_ramp()));
    }

    /// Final review #3: two edits in a row, on two notes of one sound, are two undo steps.
    #[test]
    fn quick_edits_are_steps_of_their_own() {
        let (mut d, n) = doc();
        let m = Note { position: 1920, expr: None, ..n };
        d.edit_notes("t", 0, 0, &[], &[m]).unwrap();
        d.set_expressions(0, 0, &[(n, bent_ramp())]).unwrap();
        d.set_expressions(0, 0, &[(m, ramp(960))]).unwrap();
        assert!(d.undo());
        let now = notes(&d, 0);
        assert!(
            now.iter().any(|x| x.position == 0 && d.expression(x.expr.unwrap()) == Some(&bent_ramp())),
            "the first edit stays: {now:?}"
        );
    }
}
