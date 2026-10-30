//! A sound's modulation routes ([`deckard_model::mod_matrix`]): a DECKARD-only text field
//! on the sound's object, so copying, pasting and undo carry them, saved in the DECKARD
//! block. Its chain's slots changing keep each route on its module.

use super::{Chain, Document, Op, Target};
use deckard_model::mod_matrix::{FIELD, ModRoute, to_text};
use mxprj::decode::Value;

impl Document {
    /// Give sound (`g`, `s`) these routes, one undo step (none: the field goes). Quick
    /// changes to a route (a slider dragged) make one step; adding or removing one is a
    /// step of its own.
    pub fn set_mod_routes(&mut self, g: usize, s: usize, routes: Vec<ModRoute>) -> Result<(), String> {
        let had = self.project.groups.get(g).and_then(|x| x.sounds.get(s)).map_or(0, |x| x.mod_routes.len());
        let ops = self.mod_route_ops(g, s, &routes)?;
        let label = match routes.len().cmp(&had) {
            std::cmp::Ordering::Greater => "Add Modulation",
            std::cmp::Ordering::Less => "Remove Modulation",
            std::cmp::Ordering::Equal => "Modulation",
        };
        if ops.is_empty() { Ok(()) } else { self.apply(label, ops) }
    }

    /// The ops that give sound (`g`, `s`) these routes (none if it has them).
    pub(crate) fn mod_route_ops(&self, g: usize, s: usize, routes: &[ModRoute]) -> Result<Vec<Op>, String> {
        let path = self.owner(Target::Sound(g, s)).ok_or_else(|| format!("no sound {} in group {}", s + 1, g + 1))?;
        let Some(Value::Obj(o)) = self.tree.at(&path) else { return Err("a sound that isn't an object".into()) };
        let want = (!routes.is_empty()).then(|| Value::Str(to_text(routes)));
        Ok(if o.get(FIELD) != want.as_ref() { vec![Op::SetField { path, field: FIELD.to_string(), value: want }] } else { Vec::new() })
    }

    /// The op moving sound chain `c`'s routes to their slots' new places (`remap`: a
    /// slot's new index, `None` when it's gone, and its routes with it); none for other
    /// chains or when nothing moves.
    pub(crate) fn mod_routes_rekey(&self, c: Chain, remap: impl Fn(usize) -> Option<usize>) -> Option<Op> {
        let Chain::Sound(g, s) = c else { return None };
        let routes = &self.project.groups.get(g)?.sounds.get(s)?.mod_routes;
        let moved: Vec<ModRoute> = routes.iter().filter_map(|r| remap(r.slot).map(|slot| ModRoute { slot, ..r.clone() })).collect();
        if moved == *routes {
            return None;
        }
        self.mod_route_ops(g, s, &moved).ok()?.pop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::NewModule;
    use deckard_model::mod_matrix::{ModParam, ModSource};

    fn routes(d: &Document) -> Vec<ModRoute> {
        d.project().groups[0].sounds[0].mod_routes.clone()
    }

    fn cutoff(slot: usize) -> ModRoute {
        ModRoute { low: 0.1, high: 0.9, ..ModRoute::new(ModSource::Pressure, slot, "Sampler", ModParam::Field("cutoff".into())) }
    }

    /// A new project's first sound with a Sampler (slot 0) and an effect (slot 1).
    fn doc() -> Document {
        let mut d = Document::new_project().unwrap();
        if d.project().groups[0].sounds[0].slots.is_empty() {
            d.add_slot(0, 0, 0, &NewModule::Sampler).unwrap();
        }
        let n = d.project().groups[0].sounds[0].slots.len();
        d.add_slot(0, 0, n, &NewModule::Effect("Filter")).unwrap();
        d
    }

    #[test]
    fn routes_are_set_and_undone() {
        let mut d = doc();
        d.set_mod_routes(0, 0, vec![cutoff(0)]).unwrap();
        assert_eq!(routes(&d), vec![cutoff(0)]);
        assert!(d.undo());
        assert!(routes(&d).is_empty());
        assert!(d.redo());
        assert_eq!(routes(&d), vec![cutoff(0)]);
    }

    /// Adding, changing and removing a route, quickly: each undoes on its own.
    #[test]
    fn adding_and_removing_are_steps_of_their_own() {
        let mut d = doc();
        d.set_mod_routes(0, 0, vec![cutoff(0)]).unwrap();
        let slide = ModRoute { source: ModSource::Slide, ..cutoff(0) };
        d.set_mod_routes(0, 0, vec![slide.clone()]).unwrap();
        d.set_mod_routes(0, 0, Vec::new()).unwrap();
        assert!(d.undo());
        assert_eq!(routes(&d), vec![slide]);
        assert!(d.undo());
        assert_eq!(routes(&d), vec![cutoff(0)]);
    }

    #[test]
    fn no_routes_no_field() {
        let mut d = doc();
        d.set_mod_routes(0, 0, vec![cutoff(0)]).unwrap();
        d.set_mod_routes(0, 0, Vec::new()).unwrap();
        let path = d.owner(Target::Sound(0, 0)).unwrap();
        let Some(Value::Obj(o)) = d.tree.at(&path) else { panic!() };
        assert!(o.get(FIELD).is_none());
    }

    #[test]
    fn adding_a_slot_before_moves_its_routes() {
        let mut d = doc();
        d.set_mod_routes(0, 0, vec![cutoff(1)]).unwrap();
        d.add_slot(0, 0, 1, &NewModule::Effect("Filter")).unwrap();
        assert_eq!(routes(&d)[0].slot, 2);
        assert!(d.undo());
        assert_eq!(routes(&d)[0].slot, 1, "one undo step with the slot");
    }

    #[test]
    fn removing_a_slot_removes_its_routes() {
        let mut d = doc();
        d.set_mod_routes(0, 0, vec![cutoff(0), cutoff(1)]).unwrap();
        d.remove_slot(0, 0, 0).unwrap();
        assert_eq!(routes(&d), vec![cutoff(0)], "slot 1's route moved down; slot 0's went");
    }

    #[test]
    fn moving_a_slot_takes_its_routes() {
        let mut d = doc();
        let n = d.project().groups[0].sounds[0].slots.len();
        d.add_slot(0, 0, n, &NewModule::Effect("Filter")).unwrap();
        d.set_mod_routes(0, 0, vec![cutoff(n - 1)]).unwrap();
        d.move_slot(0, 0, n - 1, n).unwrap();
        assert_eq!(routes(&d)[0].slot, n);
    }

    #[test]
    fn routes_round_trip_through_a_save() {
        let mut d = doc();
        let plug = ModRoute::new(ModSource::Pitch, 1, "Filter", ModParam::Plugin(7));
        d.set_mod_routes(0, 0, vec![cutoff(0), plug.clone()]).unwrap();
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(routes(&back), vec![cutoff(0), plug]);
        assert!(!back.is_dirty(), "reading the block is no edit");
    }
}
