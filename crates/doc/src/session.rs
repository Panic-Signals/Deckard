//! Opaque session state participates in the same history as document edits.
//! Owners keep the payload; the document only holds its token, never in the file.
use crate::{Document, Op};

impl Document {
    pub fn session_value(&self, key: &str) -> Option<u64> {
        self.session_values.get(key).copied()
    }

    /// Establish the current external state before recording its next change.
    /// Existing history keeps its own tokens, so later manual edits cannot alter redo.
    pub fn init_session_value(&mut self, key: &str, value: u64) {
        self.session_values.insert(key.to_string(), value);
    }

    pub fn set_session_value(&mut self, label: &str, key: &str, value: u64) -> Result<(), String> {
        if self.session_value(key) == Some(value) {
            return Ok(());
        }
        self.apply(label, vec![Op::Session { key: key.to_string(), value: Some(value) }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_history_groups_with_native_edits_and_rolls_back() {
        let mut d = Document::new_project().unwrap();
        let old = d.project().groups[0].sounds[0].level;
        d.init_session_value("hosted-lock", 1);
        d.transaction("Recall", |d| {
            d.set_session_value("Recall", "hosted-lock", 2)?;
            d.set("Level", crate::Target::Sound(0, 0), "level", mxprj::decode::Value::Float(0.3))
        })
        .unwrap();
        assert!(d.undo());
        assert_eq!(d.session_value("hosted-lock"), Some(1));
        assert_eq!(d.project().groups[0].sounds[0].level, old);
        assert!(d.redo());
        assert_eq!(d.session_value("hosted-lock"), Some(2));
        assert!(
            d.transaction("Fail", |d| {
                d.set_session_value("Recall", "hosted-lock", 3)?;
                Err::<(), _>("failed".into())
            })
            .is_err()
        );
        assert_eq!(d.session_value("hosted-lock"), Some(2));
        assert_eq!(d.undo_label(), Some("Recall"));
    }

    #[test]
    fn session_state_never_enters_the_tree_and_each_recall_keeps_its_redo() {
        let mut d = Document::new_project().unwrap();
        let tree = d.root().clone();
        d.init_session_value("hosted-lock", 10);
        d.transaction("Recall", |d| d.set_session_value("Recall", "hosted-lock", 11)).unwrap();
        d.init_session_value("hosted-lock", 12); // intervening manual knob edit
        d.transaction("Recall", |d| d.set_session_value("Recall", "hosted-lock", 13)).unwrap();
        assert!(d.undo());
        assert_eq!(d.session_value("hosted-lock"), Some(12));
        assert!(d.undo());
        assert_eq!(d.session_value("hosted-lock"), Some(10));
        assert!(d.redo());
        assert_eq!(d.session_value("hosted-lock"), Some(11));
        assert!(d.redo());
        assert_eq!(d.session_value("hosted-lock"), Some(13));
        assert_eq!(d.root(), &tree);
    }
}
