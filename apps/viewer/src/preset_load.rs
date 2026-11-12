//! Group LOAD acknowledges immediately; parsing/edit preparation does not own the UI thread.
use super::*;
use deckard_library::Item;
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(crate) struct GroupLoad {
    doc: deckard_doc::Shared,
    pub(super) group: usize,
    id: Option<u64>,
    pub(super) item: Item,
    routing: bool,
    patterns: bool,
    pub(super) ready: Option<Receiver<Result<deckard_doc::PreparedEdit, String>>>,
}

/// A sound/preset/effect load waiting for the controller screens to show "loading".
pub(crate) struct ItemLoad {
    pub(super) item: Item,
    target: Option<(usize, usize)>,
    frames: u8,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn fixture() -> (App, tempfile::TempDir, Item) {
        let dir = tempfile::tempdir().unwrap();
        let mut source = Document::new_project().unwrap();
        source.rename(deckard_doc::Target::Group(0), "Replacement Kit").unwrap();
        let bytes = source.file_bytes(deckard_doc::FileItem::Group(0), &mxprj::sound_info::SoundInfo::default()).unwrap();
        let path = dir.path().join("Replacement Kit.mxgrp");
        std::fs::write(&path, bytes).unwrap();
        let mut d = Document::new_project().unwrap();
        d.add_group().unwrap();
        d.rename(deckard_doc::Target::Group(0), "Playing kit").unwrap();
        let app = App { doc: Some(std::rc::Rc::new(std::cell::RefCell::new(d))), ..Default::default() };
        let item = Item { kind: deckard_library::Kind::Group, name: "Replacement Kit".into(), path, ..Default::default() };
        (app, dir, item)
    }

    fn finish(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.group_load.is_some() {
            app.poll_group_load();
            assert!(Instant::now() < deadline, "Group preparation timed out: {}", app.status);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn group_acknowledges_before_preparation_and_keeps_its_destination_after_focus_moves() {
        let (mut app, _dir, item) = fixture();
        let before = app.doc.as_ref().unwrap().borrow().revision();
        app.queue_group_load(&item, 0).unwrap();
        assert!(app.group_load.as_ref().unwrap().ready.is_none(), "LOAD does no file parsing or snapshot work");
        assert_eq!(app.doc.as_ref().unwrap().borrow().revision(), before);
        assert!(app.surface_view().groups[0].name.starts_with("Loading "));
        app.group = 1;
        finish(&mut app);
        assert_eq!(app.group, 1);
        assert_eq!(app.doc.as_ref().unwrap().borrow().project().groups[0].name, item.name);
        assert_ne!(app.doc.as_ref().unwrap().borrow().project().groups[1].name, item.name);
    }

    #[test]
    fn sound_load_shows_the_overlay_for_two_frames_before_it_loads() {
        let (mut app, _dir, mut item) = fixture();
        item.kind = deckard_library::Kind::Sound;
        item.name = "Pad".into();
        app.queue_item_load(&item, None).unwrap();
        assert_eq!(app.surface_view().loading.as_deref(), Some("Pad"));
        app.poll_item_load();
        assert!(app.item_load.is_some(), "the overlay gets a frame before the load starts");
        app.poll_item_load();
        assert!(app.item_load.is_none());
        assert_eq!(app.surface_view().loading, None);
    }

    #[test]
    fn invalid_group_preserves_the_playing_kit_and_history() {
        let (mut app, _dir, item) = fixture();
        std::fs::write(&item.path, b"bad group").unwrap();
        let before = app.doc.as_ref().unwrap().borrow().project().clone();
        app.queue_group_load(&item, 0).unwrap();
        finish(&mut app);
        assert_eq!(app.doc.as_ref().unwrap().borrow().project(), &before);
        assert!(!app.status.ends_with(" loaded"));
    }

    #[test]
    fn intervening_edits_survive_group_completion() {
        let (mut app, _dir, item) = fixture();
        app.queue_group_load(&item, 0).unwrap();
        app.poll_group_load();
        app.doc.as_ref().unwrap().borrow_mut().rename(deckard_doc::Target::Group(1), "Edited while loading").unwrap();
        finish(&mut app);
        let d = app.doc.as_ref().unwrap().borrow();
        assert_eq!(d.project().groups[0].name, item.name);
        assert_eq!(d.project().groups[1].name, "Edited while loading");
    }

    #[test]
    fn prepared_group_is_one_undo_step_and_keeps_earlier_history() {
        let (mut app, _dir, item) = fixture();
        app.doc.as_ref().unwrap().borrow_mut().set_tempo(135.).unwrap();
        app.queue_group_load(&item, 0).unwrap();
        finish(&mut app);
        let mut d = app.doc.as_ref().unwrap().borrow_mut();
        assert_eq!(d.undo_label(), Some("Load Group"));
        assert!(d.undo());
        assert_eq!(d.project().groups[0].name, "Playing kit");
        assert_eq!(d.project().tempo, 135.);
        assert!(d.undo());
        assert_eq!(d.project().tempo, 120.);
    }
}

impl App {
    /// Load a library item after the screens have had two frames to show the overlay.
    /// Projects and a load already waiting go straight through.
    pub(crate) fn queue_item_load(&mut self, item: &Item, target: Option<(usize, usize)>) -> Result<(), String> {
        if self.item_load.is_some() || matches!(item.kind, deckard_library::Kind::Project) {
            return self.load_item(item, target);
        }
        if !item.path.is_file() {
            return Err(format!("{} isn't there", item.path.display()));
        }
        let target = target.or(Some((self.group, self.sound)));
        self.item_load = Some(ItemLoad { item: item.clone(), target, frames: 2 });
        self.status = format!("Loading {}…", item.name);
        Ok(())
    }

    pub(crate) fn poll_item_load(&mut self) {
        let Some(mut job) = self.item_load.take() else { return };
        job.frames = job.frames.saturating_sub(1);
        if job.frames > 0 {
            self.item_load = Some(job);
            return;
        }
        if let Err(e) = self.load_item(&job.item, job.target) {
            self.status = e;
        }
    }

    pub(crate) fn queue_group_load(&mut self, item: &Item, group: usize) -> Result<(), String> {
        if self.group_load.is_some() {
            return Err("A Group is already loading".into());
        }
        if !item.path.is_file() {
            return Err(format!("{} isn't there", item.path.display()));
        }
        let doc = self.doc.clone().ok_or("no project")?;
        let id = doc.borrow().object_id(deckard_doc::Target::Group(group));
        if id.is_none() {
            return Err("no group".into());
        }
        self.group_load = Some(GroupLoad {
            doc,
            group,
            id,
            item: item.clone(),
            routing: self.browser.routing,
            patterns: self.browser.patterns,
            ready: None,
        });
        self.status = format!("Loading {}…", item.name);
        Ok(())
    }

    pub(crate) fn poll_group_load(&mut self) {
        let Some(mut job) = self.group_load.take() else { return };
        if self.doc.as_ref().is_none_or(|d| !std::rc::Rc::ptr_eq(d, &job.doc)) {
            return;
        }
        if job.doc.borrow().object_id(deckard_doc::Target::Group(job.group)) != job.id {
            self.status = "Group load cancelled: its destination moved or was removed".into();
            return;
        }
        let Some(ready) = &job.ready else {
            // A separate frame can submit the acknowledged controller screen before
            // taking a snapshot. Undo history is deliberately excluded from the snapshot.
            let snapshot = job.doc.borrow().edit_snapshot();
            let (tx, rx) = mpsc::sync_channel(1);
            let (item, g, routing, patterns) = (job.item.clone(), job.group, job.routing, job.patterns);
            match std::thread::Builder::new().name("deckard-group-load".into()).spawn(move || {
                let result = std::fs::read(&item.path)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| snapshot.prepare("Load Group", |d| d.load_group_file_with(g, &bytes, routing, patterns)));
                let _ = tx.send(result);
            }) {
                Ok(_) => {
                    job.ready = Some(rx);
                    self.group_load = Some(job);
                }
                Err(e) => self.status = format!("Cannot load Group: {e}"),
            }
            return;
        };
        let prepared = match ready.try_recv() {
            Err(TryRecvError::Empty) => {
                self.group_load = Some(job);
                return;
            }
            Err(TryRecvError::Disconnected) => {
                self.status = "Group preparation failed".into();
                return;
            }
            Ok(Err(e)) => {
                self.status = format!("{}: {e}", job.item.name);
                return;
            }
            Ok(Ok(edit)) => edit,
        };
        let applied = job.doc.borrow_mut().apply_prepared(prepared).is_ok();
        if !applied {
            // Preserve edits made while preparing (including recording). Use the
            // existing addressed loader against the latest document rather than
            // replacing it with a stale snapshot or retrying forever during REC.
            let before = (self.browser.routing, self.browser.patterns);
            (self.browser.routing, self.browser.patterns) = (job.routing, job.patterns);
            let result = self.load_item(&job.item, Some((job.group, 0)));
            (self.browser.routing, self.browser.patterns) = before;
            if let Err(e) = result {
                self.status = e;
            }
            return;
        }
        self.editors.retain(|&(c, ..)| !matches!(c, deckard_doc::Chain::Group(g) | deckard_doc::Chain::Sound(g, _) if g == job.group));
        if self.group == job.group {
            self.slot_of = None;
            self.plugin_page = 0;
            self.plugin_views.clear();
        }
        if self.output.is_some() {
            self.load_plugins();
        }
        self.status = format!("{} loaded", job.item.name);
    }
}
