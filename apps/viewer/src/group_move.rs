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
            let chain = |c| match c {
                deckard_doc::Chain::Group(g) => deckard_doc::Chain::Group(remap(g, a, b)),
                deckard_doc::Chain::Sound(g, s) => deckard_doc::Chain::Sound(remap(g, a, b), s),
                c => c,
            };
            for (c, _, _) in &mut self.editors {
                *c = chain(*c);
            }
            let cached = self.plugin_params.get_mut();
            *cached = std::mem::take(cached).into_iter().map(|((c, s), v)| ((chain(c), s), v)).collect();
            self.groove_settings =
                std::mem::take(&mut self.groove_settings).into_iter().map(|((g, s, f), v)| ((remap(g, a, b), s, f), v)).collect();
            self.fx_meter_view =
                std::mem::take(&mut self.fx_meter_view).into_iter().map(|((g, s, i), v)| ((remap(g, a, b), s, i), v)).collect();
            let channel = |c| match c {
                crate::layout::Channel::Group(g) => crate::layout::Channel::Group(remap(g, a, b)),
                crate::layout::Channel::Sound(g, s) => crate::layout::Channel::Sound(remap(g, a, b), s),
                c => c,
            };
            self.headroom = std::mem::take(&mut self.headroom).into_iter().map(|(c, v)| (channel(c), v)).collect();
            self.peaks = std::mem::take(&mut self.peaks).into_iter().map(|(c, v)| (channel(c), v)).collect();
            self.lit = std::mem::take(&mut self.lit).into_iter().map(|(g, s)| (remap(g, a, b), s)).collect();
            self.clip = self.clip.map(|(g, c)| (remap(g, a, b), c));
            self.zone_of = self.zone_of.map(|(g, s)| (remap(g, a, b), s));
            self.lane_of = self.lane_of.map(|(g, s)| (remap(g, a, b), s));

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
            if self.output.as_ref().and_then(|o| o.as_ref().ok()).is_some_and(|o| o.has_engine()) {
                self.group_moves.pending.push((a, b));
            }
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
    fn group_move_cached_controls_follow_their_music() {
        let mut app = App::default();
        app.new_project();
        let doc = app.doc.clone().unwrap();
        doc.borrow_mut().add_group().unwrap();
        app.groove_settings.insert((0, 0, "test".into()), 0.75);
        app.headroom.insert(crate::layout::Channel::Group(0), 0.4);
        app.zone_of = Some((0, 2));
        app.lit.insert((0, 2));
        app.move_group(0, 1).unwrap();
        assert_eq!(app.groove_settings[&(1, 0, "test".into())], 0.75);
        assert_eq!(app.headroom[&crate::layout::Channel::Group(1)], 0.4);
        assert_eq!(app.zone_of, Some((1, 2)));
        assert!(app.lit.contains(&(1, 2)));
        assert!(doc.borrow_mut().undo());
        app.follow_group_moves();
        assert_eq!(app.zone_of, Some((0, 2)));
    }
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
