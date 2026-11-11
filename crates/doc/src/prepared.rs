//! Prepare an addressed edit off-thread without cloning undo history or moving Rc loaders.
use super::*;

pub struct EditSnapshot {
    tree: Value,
    project: Project,
    addresses: Addresses,
    file: Vec<u8>,
    path: Option<PathBuf>,
    revision: u64,
    next_oid: u64,
    chosen_lengths: std::collections::HashSet<u64>,
    expressions: Vec<std::sync::Arc<deckard_model::Expression>>,
    native_pressure: bool,
    screen: Option<SavedScreen>,
    session_values: std::collections::BTreeMap<String, u64>,
}

pub struct PreparedEdit {
    base_revision: u64,
    state: EditSnapshot,
    step: Option<Step>,
}

impl Document {
    /// A Send snapshot of the current edit state, excluding potentially large history.
    pub fn edit_snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            tree: self.tree.clone(),
            project: self.project.clone(),
            addresses: self.addresses.clone(),
            file: self.file.clone(),
            path: self.path.clone(),
            revision: self.revision,
            next_oid: self.next_oid,
            chosen_lengths: self.chosen_lengths.clone(),
            expressions: self.expressions.clone(),
            native_pressure: self.native_pressure,
            screen: self.screen.clone(),
            session_values: self.session_values.clone(),
        }
    }

    pub fn object_id(&self, target: Target) -> Option<u64> {
        let Value::Obj(o) = self.tree.at(&self.owner(target)?)? else { return None };
        o.oid
    }

    /// Commit only to the originating document at the captured revision. A caller can
    /// retry preparation against a fresh snapshot after intervening edits.
    pub fn apply_prepared(&mut self, prepared: PreparedEdit) -> Result<(), PreparedEdit> {
        if self.revision != prepared.base_revision {
            return Err(prepared);
        }
        let Some(step) = prepared.step else { return Ok(()) };
        let state = prepared.state;
        self.tree = state.tree;
        self.project = state.project;
        self.addresses = state.addresses;
        self.next_oid = state.next_oid;
        self.chosen_lengths = state.chosen_lengths;
        self.expressions = state.expressions;
        self.native_pressure = state.native_pressure;
        self.session_values = state.session_values;
        // Keep live file/save/screen metadata and all previous undo steps.
        self.undo.push(step);
        self.redo.clear();
        self.revision += 1;
        Ok(())
    }
}

impl EditSnapshot {
    pub fn prepare(self, label: &str, edit: impl FnOnce(&mut Document) -> Result<(), String>) -> Result<PreparedEdit, String> {
        let base_revision = self.revision;
        let mut d = Document {
            tree: self.tree,
            project: self.project,
            addresses: self.addresses,
            file: self.file,
            path: self.path,
            revision: self.revision,
            saved_revision: self.revision,
            next_oid: self.next_oid,
            chosen_lengths: self.chosen_lengths,
            expressions: self.expressions,
            native_pressure: self.native_pressure,
            screen: self.screen,
            session_values: self.session_values,
            // Built again when needed (see `notes.rs` and `Document::run`).
            event_node: None,
            note_event: None,
            event_sequence: None,
            references: None,
            types: mxprj::types::Types::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            take: None,
        };
        d.transaction(label, edit)?;
        let step = d.undo.pop();
        let state = EditSnapshot {
            tree: d.tree,
            project: d.project,
            addresses: d.addresses,
            file: d.file,
            path: d.path,
            revision: d.revision,
            next_oid: d.next_oid,
            chosen_lengths: d.chosen_lengths,
            expressions: d.expressions,
            native_pressure: d.native_pressure,
            screen: d.screen,
            session_values: d.session_values,
        };
        Ok(PreparedEdit { base_revision, state, step })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_edit_retains_undo_and_rejects_intervening_edits() {
        let mut d = Document::new_project().unwrap();
        d.set_tempo(130.).unwrap();
        let snapshot = d.edit_snapshot();
        let prepared =
            std::thread::spawn(move || snapshot.prepare("Prepared rename", |d| d.rename(Target::Group(0), "Kit"))).join().unwrap().unwrap();
        assert!(d.apply_prepared(prepared).is_ok());
        assert_eq!(d.project().groups[0].name, "Kit");
        assert!(d.undo());
        assert_ne!(d.project().groups[0].name, "Kit");
        assert!(d.undo());
        assert_eq!(d.project().tempo, 120.);
        let stale = d.edit_snapshot().prepare("Stale", |d| d.set_tempo(140.)).unwrap();
        d.set_tempo(135.).unwrap();
        assert!(d.apply_prepared(stale).is_err());
        assert_eq!(d.project().tempo, 135.);
    }

    /// A prepared edit that doesn't touch session state (a Group load) leaves it as it was:
    /// the hosted LOCK's token isn't lost on the way through the worker.
    #[test]
    fn session_state_survives_a_prepared_edit() {
        let mut d = Document::new_project().unwrap();
        d.init_session_value("hosted-lock", 7);
        let snapshot = d.edit_snapshot();
        let prepared =
            std::thread::spawn(move || snapshot.prepare("Load Group", |d| d.rename(Target::Group(0), "Kit"))).join().unwrap().unwrap();
        assert!(d.apply_prepared(prepared).is_ok());
        assert_eq!(d.project().groups[0].name, "Kit");
        assert_eq!(d.session_value("hosted-lock"), Some(7));
        assert!(d.undo());
        assert_eq!(d.session_value("hosted-lock"), Some(7));
    }

    /// A prepared edit can change session state too, as one undo step with its other edits.
    #[test]
    fn a_prepared_edit_changes_session_state_in_one_undo_step() {
        let mut d = Document::new_project().unwrap();
        d.init_session_value("hosted-lock", 1);
        let prepared = d
            .edit_snapshot()
            .prepare("Recall", |d| {
                d.set_session_value("Recall", "hosted-lock", 2)?;
                d.rename(Target::Group(0), "Recalled")
            })
            .unwrap();
        assert!(d.apply_prepared(prepared).is_ok());
        assert_eq!(d.session_value("hosted-lock"), Some(2));
        assert_eq!(d.undo_label(), Some("Recall"));
        assert!(d.undo());
        assert_eq!(d.session_value("hosted-lock"), Some(1));
        assert_ne!(d.project().groups[0].name, "Recalled");
        assert!(d.redo());
        assert_eq!(d.session_value("hosted-lock"), Some(2));
        assert_eq!(d.project().groups[0].name, "Recalled");
    }
}
