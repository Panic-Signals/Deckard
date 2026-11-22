//! Group LOAD acknowledges immediately; parsing/edit preparation does not own the UI thread.
use super::*;
use deckard_library::Item;
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(crate) struct PreparedGroup {
    edit: deckard_doc::PreparedEdit,
    source_info: Option<mxprj::sound_info::SoundInfo>,
    samples: deckard_engine::PreloadedSamples,
    natives: Vec<(deckard_plugins::Slot, deckard_model::PluginRef, deckard_plugins::Shared)>,
}

pub(super) fn editable_preset_info(item: &Item, bytes: &[u8]) -> Option<mxprj::sound_info::SoundInfo> {
    if !item.user || std::fs::metadata(&item.path).ok()?.permissions().readonly() {
        return None;
    }
    let mut info = mxprj::Project::parse(bytes).ok()?.sound_info().unwrap_or_default();
    if info.name.is_empty() {
        info.name = item.name.clone();
    }
    Some(info)
}

pub(crate) struct GroupLoad {
    doc: deckard_doc::Shared,
    pub(super) group: usize,
    id: Option<u64>,
    pub(super) item: Item,
    routing: bool,
    patterns: bool,
    pub(super) ready: Option<Receiver<Result<PreparedGroup, String>>>,
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
    fn saving_a_loaded_user_group_replaces_its_exact_source() {
        for fallback in [false, true] {
            use deckard_doc::{FileItem, Target};
            let (mut app, dir, mut item) = fixture();
            item.user = true;
            let alternate = dir.path().join("other").join(item.path.file_name().unwrap());
            std::fs::create_dir(alternate.parent().unwrap()).unwrap();
            std::fs::copy(&item.path, &alternate).unwrap();
            let original = std::fs::read(&alternate).unwrap();
            let d = app.doc.clone().unwrap();
            app.queue_group_load(&item, 0).unwrap();
            if fallback {
                app.poll_group_load();
                d.borrow_mut().rename(Target::Group(1), "Edited while loading").unwrap();
            }
            finish(&mut app);
            d.borrow_mut().set_level(Target::Sound(0, 0), 0.659).unwrap();
            app.save_item(FileItem::Group(0), SaveMode::Save);
            assert!(app.status.starts_with("saved "), "{}", app.status);
            assert!(app.save_dialog.is_none());
            assert_eq!(std::fs::read(&alternate).unwrap(), original);
            let mut reopened = Document::new_project().unwrap();
            reopened.load_group_file(0, &std::fs::read(&item.path).unwrap()).unwrap();
            assert!((reopened.project().groups[0].sounds[0].level - 0.659).abs() < 1e-6);

            item.user = false;
            item.path = alternate;
            app.queue_group_load(&item, 0).unwrap();
            finish(&mut app);
            app.save_item(FileItem::Group(0), SaveMode::Save);
            assert!(app.save_dialog.is_some(), "factory presets use the Save dialog");
            assert_eq!(std::fs::read(&item.path).unwrap(), original);
        }
    }

    #[test]
    fn saving_a_loaded_user_sound_replaces_its_exact_source() {
        use deckard_doc::{FileItem, Target};
        let (mut app, dir, _) = fixture();
        let what = FileItem::Sound(0, 0);
        let d = app.doc.clone().unwrap();
        let mut info = mxprj::sound_info::SoundInfo::default();
        info.author = "Preset author".into();
        let original = d.borrow().file_bytes(what, &info).unwrap();
        let path = dir.path().join("Kick.mxsnd");
        let other = dir.path().join("other").join("Kick.mxsnd");
        std::fs::create_dir(other.parent().unwrap()).unwrap();
        for file in [&path, &other] {
            std::fs::write(file, &original).unwrap();
        }
        let mut item = crate::browser::item_of(&path).unwrap();
        app.queue_item_load(&item, Some((0, 0))).unwrap();
        app.poll_item_load();
        app.poll_item_load();
        d.borrow_mut().set_level(Target::Sound(0, 0), 0.659).unwrap();
        app.save_item(what, SaveMode::Save);
        assert!(app.status.starts_with("saved "), "{}", app.status);
        assert!(app.save_dialog.is_none());
        assert_eq!(std::fs::read(&other).unwrap(), original);
        let bytes = std::fs::read(&path).unwrap();
        let mut reopened = Document::new_project().unwrap();
        reopened.load_sound_file(0, 0, &bytes).unwrap();
        assert!((reopened.project().groups[0].sounds[0].level - 0.659).abs() < 1e-6);
        assert_eq!(mxprj::Project::parse(&bytes).unwrap().sound_info().unwrap().author, "Preset author");

        item.user = false;
        item.path = other;
        app.load_item(&item, Some((0, 0))).unwrap();
        app.save_item(what, SaveMode::Save);
        assert!(app.save_dialog.is_some(), "factory presets use the Save dialog");
        assert_eq!(std::fs::read(&item.path).unwrap(), original);
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

    #[test]
    fn group_load_preloads_all_sound_samples_off_the_ui_thread() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = Document::new_project().unwrap();
        source.rename(deckard_doc::Target::Group(0), "Multi-Pad Kit").unwrap();
        let mut wavs = Vec::new();
        for s in 0..8 {
            let wav = dir.path().join(format!("sound_{s}.wav"));
            deckard_engine::edit::write_wav(&wav, &deckard_engine::sample::Sample { rate: 48_000, frames: vec![[0.25; 2]; 256] }).unwrap();
            source.sampler_only(0, s).unwrap();
            source
                .set_zones(
                    0,
                    s,
                    &[deckard_doc::ZoneSpec { from: deckard_doc::ZoneFrom::Template, file: Some(wav.clone()), set: Vec::new() }],
                    "Zone",
                )
                .unwrap();
            wavs.push(wav);
        }
        let bytes = source.file_bytes(deckard_doc::FileItem::Group(0), &mxprj::sound_info::SoundInfo::default()).unwrap();
        let path = dir.path().join("Multi-Pad Kit.mxgrp");
        std::fs::write(&path, bytes).unwrap();

        let d = Document::new_project().unwrap();
        let mut app = App { doc: Some(std::rc::Rc::new(std::cell::RefCell::new(d))), ..Default::default() };
        let item = Item { kind: deckard_library::Kind::Group, name: "Multi-Pad Kit".into(), path, ..Default::default() };
        app.queue_group_load(&item, 0).unwrap();
        finish(&mut app);
        for wav in &wavs {
            assert!(app.samples.has_cached(wav), "sample {} should be decoded before follow_edits runs", wav.display());
        }
    }
}

fn prepare_group_natives(
    group: usize,
    grp: &deckard_model::Group,
    rate: f64,
) -> Vec<(deckard_plugins::Slot, deckard_model::PluginRef, deckard_plugins::Shared)> {
    let mut wanted: Vec<(deckard_plugins::Slot, deckard_model::PluginRef)> = Vec::new();
    for (s, snd) in grp.sounds.iter().enumerate() {
        if let Some(p) = &snd.plugin {
            wanted.push((deckard_plugins::Slot::Sound { group, sound: s }, p.clone()));
        }
        for (i, x) in snd.slots.iter().enumerate().filter(|(_, x)| !x.is_instrument()) {
            if let Some(p) = x.plugin.as_ref().filter(|p| deckard_engine::is_effect(&p.id)) {
                wanted.push((deckard_plugins::Slot::Chain { group, sound: s, slot: i }, p.clone()));
            }
        }
    }
    for (i, x) in grp.slots.iter().enumerate() {
        if let Some(p) = x.plugin.as_ref().filter(|p| deckard_engine::is_effect(&p.id)) {
            wanted.push((deckard_plugins::Slot::Group { group, slot: i }, p.clone()));
        }
    }
    if wanted.is_empty() {
        return Vec::new();
    }
    if wanted.len() == 1 {
        let (slot, p) = wanted.into_iter().next().unwrap();
        return match deckard_plugins::Host::prepare_native(&p.id, &p.name, &p.chunk, rate) {
            Some(Ok(shared)) => vec![(slot, p, shared)],
            _ => Vec::new(),
        };
    }
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8).min(wanted.len());
    let cursor = std::sync::atomic::AtomicUsize::new(0);
    let mut indexed = Vec::new();
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            handles.push(scope.spawn(|| {
                let mut local = Vec::new();
                loop {
                    let i = cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((slot, p)) = wanted.get(i) else { break };
                    if let Some(Ok(shared)) = deckard_plugins::Host::prepare_native(&p.id, &p.name, &p.chunk, rate) {
                        local.push((i, *slot, p.clone(), shared));
                    }
                }
                local
            }));
        }
        for h in handles {
            if let Ok(part) = h.join() {
                indexed.extend(part);
            }
        }
    });
    indexed.sort_by_key(|(i, ..)| *i);
    indexed.into_iter().map(|(_, slot, p, shared)| (slot, p, shared)).collect()
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
            let sample_snap = self.samples.known_sample_state();
            let project_path = self.path.clone();
            let rate = self.output.as_ref().and_then(|r| r.as_ref().ok()).map(|o| o.rate as f64);
            let (tx, rx) = mpsc::sync_channel(1);
            let (item, g, routing, patterns) = (job.item.clone(), job.group, job.routing, job.patterns);
            match std::thread::Builder::new().name("deckard-group-load".into()).spawn(move || {
                let result = std::fs::read(&item.path)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| {
                        snapshot
                            .prepare("Load Group", |d| d.load_group_file_with(g, &bytes, routing, patterns))
                            .map(|edit| (edit, editable_preset_info(&item, &bytes)))
                    })
                    .map(|(edit, source_info)| {
                        let (samples, natives) = match edit.project().groups.get(g) {
                            Some(grp) => std::thread::scope(|s| {
                                let samples_h = s.spawn(|| {
                                    deckard_engine::SampleCache::preload_group_samples(grp, project_path.as_deref(), &sample_snap)
                                });
                                let natives = rate.map_or_else(Vec::new, |r| prepare_group_natives(g, grp, r));
                                let samples = samples_h.join().unwrap_or_default();
                                (samples, natives)
                            }),
                            None => (deckard_engine::PreloadedSamples::default(), Vec::new()),
                        };
                        PreparedGroup { edit, source_info, samples, natives }
                    });
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
            Ok(Ok(prepared)) => prepared,
        };
        self.samples.ingest_preloaded(prepared.samples);
        let applied = job.doc.borrow_mut().apply_prepared(prepared.edit).is_ok();
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
        self.forget_group_presets(job.group);
        self.remember_preset_source(deckard_doc::FileItem::Group(job.group), &job.item, prepared.source_info);
        self.editors.retain(|&(c, ..)| !matches!(c, deckard_doc::Chain::Group(g) | deckard_doc::Chain::Sound(g, _) if g == job.group));
        if self.group == job.group {
            self.slot_of = None;
            self.plugin_page = 0;
            self.plugin_views.clear();
        }
        if let Some(Ok(out)) = &self.output {
            let rate = out.rate as f64;
            for (slot, p, shared) in prepared.natives {
                self.samples.plugins.adopt_prepared(slot, &p.id, &p.name, &p.chunk, rate, shared);
            }
        }
        if self.output.is_some() {
            self.load_plugins();
        }
        self.status = format!("{} loaded", job.item.name);
    }
}
