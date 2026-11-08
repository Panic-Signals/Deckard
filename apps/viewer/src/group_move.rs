//! Keep positional runtime state with document Group identities, including undo.
use crate::App;
#[derive(Default)]
pub(crate) struct State {
    generation: u64,
    ids: Vec<u64>,
    pub pending: Vec<(usize, usize)>,
}
impl App {
    pub(crate) fn move_group(&mut self, from: usize, to: usize) -> Result<(), String> {
        let Some(doc) = self.doc.clone() else { return Ok(()) };
        let n = doc.borrow().project().groups.len();
        if from >= n || to >= n || from.abs_diff(to) != 1 {
            return Ok(());
        }
        self.end_mod_take();
        self.follow_group_moves();
        doc.borrow_mut().swap_groups(from, to)?;
        self.follow_group_moves();
        Ok(())
    }
    pub(crate) fn follow_group_moves(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let ids = doc.borrow().group_ids();
        if self.group_moves.generation != self.doc_generation
            || self.group_moves.ids.len() != ids.len()
            || ids.iter().any(|id| !self.group_moves.ids.contains(id))
        {
            self.group_moves = State { generation: self.doc_generation, ids, ..Default::default() };
            return;
        }
        for a in 0..ids.len() {
            if self.group_moves.ids[a] == ids[a] {
                continue;
            }
            let Some(b) = self.group_moves.ids.iter().position(|id| *id == ids[a]) else { continue };
            self.samples.plugins.swap_groups(a, b);
            if let Some(Ok(out)) = &self.output {
                out.swap_group_inputs(a, b);
            }
            if self.held.is_some_and(|(g, _, _)| g == a || g == b) {
                self.held = None;
            }
            self.group = remap(self.group, a, b);
            if self.meters.len() > a.max(b) {
                self.meters.swap(a, b);
            }
            self.group_moves.ids.swap(a, b);
            self.group_moves.pending.push((a, b));
            // A full edit uses the moved instances and the new routing graph.
            self.built = None;
        }
    }
}
fn remap(g: usize, a: usize, b: usize) -> usize {
    if g == a {
        b
    } else if g == b {
        a
    } else {
        g
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn group_move_follows_focus_and_undo_and_stops_at_the_ends() {
        let mut app = App::default();
        app.new_project();
        let doc = app.doc.clone().unwrap();
        doc.borrow_mut().add_group().unwrap();
        app.follow_group_moves();
        app.move_group(0, 1).unwrap();
        assert_eq!(app.group, 1);
        assert_eq!(doc.borrow().undo_label(), Some("Move Group"));
        assert!(doc.borrow_mut().undo());
        app.follow_group_moves();
        assert_eq!(app.group, 0);
        let revision = doc.borrow().revision();
        app.move_group(0, usize::MAX).unwrap();
        app.move_group(1, 2).unwrap();
        assert_eq!(doc.borrow().revision(), revision);
    }
}
