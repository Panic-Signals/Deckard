//! The project's CV routes ([`deckard_model::mod_matrix::CvRoute`]): LFOs sent out as
//! control voltages on audio outputs, a DECKARD-only text field on the project's root
//! object, so undo carries them, saved in the DECKARD block.

use super::{Document, Op};
use deckard_model::mod_matrix::{CV_FIELD, CvRoute, cv_to_text};
use mxprj::decode::Value;

impl Document {
    /// Give the project these CV routes, one undo step (labelled as
    /// [`Self::set_mod_routes`]: quick edits of a route merge; adding or removing one is
    /// a step of its own).
    pub fn set_cv_routes(&mut self, routes: Vec<CvRoute>) -> Result<(), String> {
        let label = match routes.len().cmp(&self.project.cv_routes.len()) {
            std::cmp::Ordering::Greater => "Add Modulation",
            std::cmp::Ordering::Less => "Remove Modulation",
            std::cmp::Ordering::Equal => "Modulation",
        };
        let ops = self.cv_route_ops(&routes);
        if ops.is_empty() {
            return Ok(());
        }
        self.apply(label, ops)?;
        if label != "Modulation" {
            // (Nothing merges into it: adding another route next is another step.)
            if let Some(last) = self.undo.last_mut() {
                last.at = std::time::Instant::now() - super::MERGE_WINDOW;
            }
        }
        Ok(())
    }

    /// The ops that give the project these CV routes (none if it has them; no field if
    /// none).
    pub(crate) fn cv_route_ops(&self, routes: &[CvRoute]) -> Vec<Op> {
        let want = (!routes.is_empty()).then(|| Value::Str(cv_to_text(routes)));
        let Some(Value::Obj(o)) = self.tree.at(&[]) else { return Vec::new() };
        if o.get(CV_FIELD) == want.as_ref() {
            return Vec::new();
        }
        vec![Op::SetField { path: Vec::new(), field: CV_FIELD.to_string(), value: want }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_model::mod_matrix::ModSource;

    fn cv(n: u8) -> CvRoute {
        CvRoute { high: 0.5, ..CvRoute::new(ModSource::Lfo(1), n) }
    }

    #[test]
    fn cv_routes_are_set_undone_and_saved() {
        let mut d = Document::new_project().unwrap();
        assert!(d.project().cv_routes.is_empty());
        d.set_cv_routes(vec![cv(2)]).unwrap();
        assert_eq!(d.project().cv_routes, vec![cv(2)]);
        d.set_cv_routes(vec![cv(2), cv(3)]).unwrap();
        assert!(d.undo());
        assert_eq!(d.project().cv_routes, vec![cv(2)]);
        assert!(d.undo());
        assert!(d.project().cv_routes.is_empty());
        assert!(d.redo());
        assert!(d.redo());
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(back.project().cv_routes, vec![cv(2), cv(3)], "kept in the DECKARD block");
        assert!(!back.is_dirty());
    }

    #[test]
    fn maschine_does_not_see_the_field() {
        let mut d = Document::new_project().unwrap();
        d.set_cv_routes(vec![cv(3)]).unwrap();
        let Some(Value::Obj(root)) = d.tree.at(&[]) else { panic!("no root") };
        assert!(root.get(CV_FIELD).is_some(), "the document has the field");
        let saved = d.to_bytes().unwrap();
        let archive = mxprj::Project::parse(&saved).unwrap().decode(&mxprj::types::Types::new()).unwrap();
        let mut found = false;
        archive.walk_paths(&mut |o, _| found |= o.fields.iter().any(|(f, _)| f.starts_with("deckard:")));
        assert!(!found);
    }
}
