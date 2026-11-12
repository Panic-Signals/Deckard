//! The browser (docs/superpowers/specs/2026-10-22-browser-design.md): its state, and
//! the library it shows, kept on a thread of its own so the window never waits on a
//! query (NI's index holds tens of thousands of items). The newest query wins; results
//! of older ones are dropped.

use deckard_library::user::Root;
use deckard_library::{Item, Library, Query, Results};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// The browser's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Library,
    Files,
}

enum Job {
    Query(u64, Query),
    Favorite(PathBuf, bool),
    Saved(PathBuf),
    Entry(deckard_library::Item),
    Scan(Vec<Root>),
    Forget(PathBuf),
}

/// A query's answer: its number, the results, and why NI's index isn't there.
type Answer = (u64, Results, Option<String>);

pub struct Browser {
    pub open: bool,
    pub tab: Tab,
    pub query: Query,
    pub results: Results,
    pub selected: Option<usize>,
    /// Prehear a sample when it's selected.
    pub prehear_auto: bool,
    pub prehear_gain: f32,
    /// Load a group with its sound routing (`+ ROUTING`).
    pub routing: bool,
    /// Load a group with its patterns (`+ PATTERNS`).
    pub patterns: bool,
    /// Why NI's index can't be read, if it can't.
    pub error: Option<String>,
    /// The top-level types and the chosen type's sub-types, as last listed (the
    /// controller's Type and Sub-type lists stay while one is chosen).
    top_types: Vec<String>,
    sub_types: Vec<String>,
    /// The FILES tab's folder, what's in it, and the file chosen.
    pub files_dir: PathBuf,
    pub files: Vec<Entry>,
    pub file_selected: Option<PathBuf>,
    worker: Option<(mpsc::Sender<Job>, mpsc::Receiver<Answer>)>,
    /// Moves on with every change (what the controller is shown follows it).
    rev: u64,
    asked: u64,
    shown: u64,
}

impl Default for Browser {
    fn default() -> Browser {
        Browser {
            open: false,
            tab: Tab::Library,
            query: Query::default(),
            results: Results::default(),
            selected: None,
            prehear_auto: true,
            prehear_gain: 0.8,
            routing: true,
            patterns: true,
            error: None,
            top_types: Vec::new(),
            sub_types: Vec::new(),
            files_dir: music_dir(),
            files: Vec::new(),
            file_selected: None,
            worker: None,
            rev: 1,
            asked: 0,
            shown: 0,
        }
    }
}

/// The user's Music folder, else their home.
fn music_dir() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default();
    let music = home.join("Music");
    if music.is_dir() { music } else { home }
}

impl Browser {
    /// Start the library: NI's index at `ni` (read-only), DECKARD's at `own`, the
    /// folders `roots` scanned into it; then the current query.
    pub fn start(&mut self, ni: Option<PathBuf>, own: PathBuf, roots: Vec<Root>) {
        let (jobs, rx) = mpsc::channel::<Job>();
        let (tx, answers) = mpsc::channel::<Answer>();
        std::thread::Builder::new()
            .name("library".into())
            .spawn(move || {
                let mut lib = match Library::new(ni, &own) {
                    Ok(l) => l,
                    Err(e) => {
                        // Answer every query with the error.
                        while let Ok(job) = rx.recv() {
                            if let Job::Query(n, _) = job
                                && tx.send((n, Results::default(), Some(e.clone()))).is_err()
                            {
                                return;
                            }
                        }
                        return;
                    }
                };
                while let Ok(mut job) = rx.recv() {
                    // Only the newest of the queries waiting.
                    while let Job::Query(..) = job {
                        match rx.try_recv() {
                            Ok(next @ Job::Query(..)) => job = next,
                            Ok(other) => {
                                run(&mut lib, other);
                            }
                            Err(_) => break,
                        }
                    }
                    if let Job::Query(n, q) = job {
                        let r = lib.query(&q);
                        if tx.send((n, r, lib.ni_error.clone())).is_err() {
                            return;
                        }
                    } else {
                        run(&mut lib, job);
                    }
                }
            })
            .expect("a thread for the library");
        self.worker = Some((jobs, answers));
        self.send(Job::Scan(roots));
        self.refresh();
    }

    pub fn started(&self) -> bool {
        self.worker.is_some()
    }

    fn send(&mut self, job: Job) {
        if let Some((jobs, _)) = &self.worker {
            let _ = jobs.send(job);
        }
    }

    /// Ask for `q` (the results come in through [`Browser::poll`]).
    pub fn set_query(&mut self, q: Query) {
        self.query = q;
        self.refresh();
    }

    /// Run the current query again.
    pub fn refresh(&mut self) {
        self.asked += 1;
        let q = self.query.clone();
        let n = self.asked;
        self.send(Job::Query(n, q));
    }

    pub fn set_favorite(&mut self, path: &Path, on: bool) {
        self.send(Job::Favorite(path.to_path_buf(), on));
        self.refresh();
    }

    /// A file just saved: listed at once.
    pub fn add_saved(&mut self, path: &Path) {
        self.send(Job::Saved(path.to_path_buf()));
        self.refresh();
    }

    /// A file DECKARD just saved, listed under User at once as `item` describes it
    /// (for files whose tags DECKARD's index can't read itself, such as NKS presets).
    pub fn add_item(&mut self, item: deckard_library::Item) {
        self.send(Job::Entry(item));
        self.refresh();
    }

    /// Scan `roots` again (Maschine's user folders, library folders).
    pub fn rescan(&mut self, roots: Vec<Root>) {
        self.send(Job::Scan(roots));
        self.refresh();
    }

    /// Drop what's listed from under `dir` (a library folder taken out).
    pub fn forget(&mut self, dir: &Path) {
        self.send(Job::Forget(dir.to_path_buf()));
    }

    /// Take up results that came in; true when they changed.
    pub fn poll(&mut self) -> bool {
        let Some((_, answers)) = &self.worker else { return false };
        let mut newest = None;
        while let Ok(a) = answers.try_recv() {
            if a.0 == self.asked {
                newest = Some(a);
            }
        }
        let Some((n, results, error)) = newest else { return false };
        self.rev += 1;
        // An old filter's results can omit the selected preset and send focus
        // back to the first row. Only the current query may replace the list.
        self.shown = n;
        let keep = self.selected.and_then(|i| self.results.items.get(i)).map(|i| i.path.clone());
        match self.query.types.len() {
            0 => self.top_types = results.types.iter().map(|t| t.0.clone()).collect(),
            1 => self.sub_types = results.types.iter().map(|t| t.0.clone()).collect(),
            _ => {}
        }
        self.results = results;
        self.selected = keep.and_then(|p| self.results.items.iter().position(|i| i.path == p));
        self.error = error;
        true
    }

    /// Moves on with every change.
    pub fn revision(&self) -> u64 {
        self.rev
    }

    /// Something changed (a choice, the focus).
    pub fn touch(&mut self) {
        self.rev += 1;
    }

    /// Whether results are still to come.
    pub fn waiting(&self) -> bool {
        self.worker.is_some() && self.shown < self.asked
    }

    /// Wait for the newest query's results (tests).
    #[cfg(test)]
    pub fn wait(&mut self) {
        let t = std::time::Instant::now();
        while self.waiting() && t.elapsed() < std::time::Duration::from_secs(10) {
            if !self.poll() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    /// The library as the controller's BROWSE shows it.
    pub fn view(&self) -> deckard_lua_host::state::BrowserView {
        use deckard_library::Kind;
        let q = &self.query;
        let pick = |list: &[String], chosen: Option<&String>| chosen.and_then(|c| list.iter().position(|x| x == c));
        let subs = if q.types.is_empty() { Vec::new() } else { self.sub_types.clone() };
        let characters: Vec<String> = self.results.characters.iter().map(|c| c.0.clone()).collect();
        deckard_lua_host::state::BrowserView {
            kind: Kind::ALL.iter().position(|k| *k == q.kind).unwrap_or(0),
            user: q.user,
            product: q.product.as_ref().and_then(|p| self.results.products.iter().position(|x| x == p)),
            products: self.results.products.clone(),
            attributes: [
                (self.top_types.clone(), pick(&self.top_types, q.types.first())),
                (subs.clone(), pick(&subs, q.types.get(1))),
                (characters.clone(), pick(&characters, q.characters.first())),
            ],
            results: self.results.items.iter().map(|i| (i.name.clone(), i.favorite)).collect(),
            focus: self.selected.unwrap_or(0),
            busy: self.waiting(),
            favorites: q.favorites,
            routing: self.routing,
            patterns: self.patterns,
        }
    }

    /// What the controller's BROWSE asked for, as the panel would ask it.
    pub fn from_controller(&self, call: deckard_lua_host::state::BrowseCall) -> Option<BrowseAct> {
        use deckard_lua_host::state::BrowseCall as C;
        let q = &self.query;
        let focus = self.selected.unwrap_or(0);
        Some(match call {
            C::Kind(k) => BrowseAct::Kind(*deckard_library::Kind::ALL.get(k)?),
            C::User(u) => BrowseAct::User(u),
            C::Product(p) => BrowseAct::Product(p.and_then(|p| self.results.products.get(p).cloned())),
            C::Attribute(0, i) => BrowseAct::Types(i.and_then(|i| self.top_types.get(i).cloned()).into_iter().collect()),
            C::Attribute(1, i) => {
                let mut t: Vec<String> = q.types.first().cloned().into_iter().collect();
                t.extend(i.and_then(|i| self.sub_types.get(i).cloned()));
                BrowseAct::Types(t)
            }
            C::Attribute(_, i) => {
                BrowseAct::Characters(i.and_then(|i| self.results.characters.get(i).map(|c| c.0.clone())).into_iter().collect())
            }
            C::Focus(by) => BrowseAct::Focus((focus as i64 + by).max(0) as usize),
            C::Load => BrowseAct::Load(focus, None),
            C::ToggleFavorites => BrowseAct::Favorites(!q.favorites),
            C::Prehear => BrowseAct::Prehear(focus),
            C::Routing(on) => BrowseAct::Routing(on),
            C::Patterns(on) => BrowseAct::Patterns(on),
        })
    }

    pub fn selected_item(&self) -> Option<&Item> {
        self.selected.and_then(|i| self.results.items.get(i))
    }
}

fn run(lib: &mut Library, job: Job) {
    match job {
        Job::Favorite(p, on) => lib.set_favorite(&p, on),
        Job::Saved(p) => lib.add_saved(&p),
        Job::Entry(item) => lib.add_entry(item),
        Job::Scan(roots) => {
            let _ = lib.scan(&roots);
        }
        Job::Forget(dir) => lib.forget(&dir),
        Job::Query(..) => {}
    }
}

/// What the browser asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum BrowseAct {
    /// Show or hide the panel.
    Toggle,
    Tab(Tab),
    Kind(deckard_library::Kind),
    User(bool),
    Favorites(bool),
    Product(Option<String>),
    /// The chosen type's levels.
    Types(Vec<String>),
    /// The characters chosen (all of them).
    Characters(Vec<String>),
    /// Focus result `.0`, without prehearing it (the controller moves it).
    Focus(usize),
    /// A character chosen (true) or not.
    Character(String, bool),
    Text(String),
    Select(usize),
    /// Load result `.0` into a sound (group, sound), else into the focus.
    Load(usize, Option<(usize, usize)>),
    Favorite(usize, bool),
    Prehear(usize),
    PrehearAuto(bool),
    Gain(f32),
    /// Load groups with (`true`) or without (`false`) their sound routing (`+ ROUTING`).
    Routing(bool),
    /// Load groups with (`true`) or without (`false`) their patterns (`+ PATTERNS`).
    Patterns(bool),
    /// FILES: show this folder.
    Folder(PathBuf),
    /// FILES: the folder above.
    Up,
    /// FILES: pin a folder (true) or unpin it.
    PinFolder(PathBuf, bool),
    /// FILES: a file chosen (prehears it).
    SelectFile(PathBuf),
    /// FILES: load a file into a sound (group, sound), else into the focus.
    LoadFile(PathBuf, Option<(usize, usize)>),
    /// Preferences ▸ Library: a folder of sound packs added (true) or taken out.
    LibraryFolder(PathBuf, bool),
    /// Scan the library folders again (a newly installed expansion).
    Rescan,
}

/// Whether `item` has something to prehear: a sample, or a preset's preview.
pub fn can_prehear(item: &Item) -> bool {
    prehear_file(item).is_some()
}

/// The file to prehear for `item`: a sample itself; for a sound, group, instrument or
/// effect, the preview Maschine keeps beside it (`.previews/<file name>.ogg`, or a `.wav`
/// preview in `.previews/`) when there is one.
pub fn prehear_file(item: &Item) -> Option<PathBuf> {
    use deckard_library::Kind;
    match item.kind {
        Kind::Loop | Kind::OneShot => Some(item.path.clone()),
        Kind::Sound | Kind::Group | Kind::Instrument | Kind::Effect => {
            let previews = item.path.parent()?.join(".previews");
            let name = item.path.file_name()?.to_string_lossy();
            let ogg = previews.join(format!("{name}.ogg"));
            if ogg.is_file() {
                return Some(ogg);
            }
            let wav_full = previews.join(format!("{name}.wav"));
            if wav_full.is_file() {
                return Some(wav_full);
            }
            let stem = item.path.file_stem()?.to_string_lossy();
            let wav_stem = previews.join(format!("{stem}.wav"));
            wav_stem.is_file().then_some(wav_stem)
        }
        Kind::Project => None,
    }
}

/// A FILES row: a folder, or a file the browser can load.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    pub dir: bool,
}

/// What `dir` holds that FILES shows: its folders, then the files DECKARD loads
/// (Maschine's, and samples), each by name ignoring case; hidden ones left out.
pub fn list_folder(dir: &Path) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Entry> = rd
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| {
            let path = e.path();
            let dir = e.file_type().ok()?.is_dir();
            (dir || deckard_library::Kind::of_file(&path).is_some()).then_some(Entry { path, dir })
        })
        .collect();
    out.sort_by_key(|e| (!e.dir, e.path.file_name().map(|n| n.to_string_lossy().to_lowercase())));
    out
}

/// A file as a library item (its kind from its extension or name).
pub fn item_of(path: &Path) -> Option<Item> {
    let kind = deckard_library::Kind::of_file(path)?;
    let name = path.file_stem()?.to_string_lossy().into_owned();
    Some(Item { kind, user: true, name, path: path.to_path_buf(), ..Item::default() })
}

#[cfg(test)]
mod query_order_tests {
    use super::*;
    #[test]
    fn an_old_query_cannot_send_the_second_preset_back_to_the_first() {
        let items: Vec<_> = ["First", "Second", "Third"]
            .into_iter()
            .map(|name| Item {
                path: PathBuf::from(format!("{name}.nksf")),
                name: name.into(),
                kind: deckard_library::Kind::Instrument,
                ..Default::default()
            })
            .collect();
        let (jobs, _) = mpsc::channel();
        let (answers, received) = mpsc::channel();
        let mut browser = Browser {
            worker: Some((jobs, received)),
            asked: 2,
            selected: Some(1),
            results: Results { items: items.clone(), ..Default::default() },
            ..Default::default()
        };
        answers.send((1, Results { items: vec![items[0].clone()], ..Default::default() }, None)).unwrap();
        assert!(!browser.poll());
        assert_eq!(browser.selected_item().unwrap().name, "Second");
        assert!(browser.waiting());
        answers.send((2, Results { items, ..Default::default() }, None)).unwrap();
        assert!(browser.poll());
        assert_eq!(browser.selected_item().unwrap().name, "Second");
        assert!(!browser.waiting());
    }
}

#[cfg(test)]
mod current_query_tests {
    use super::*;
    #[test]
    fn stale_results_after_current_results_in_the_same_drain_are_ignored() {
        let (jobs, _) = mpsc::channel();
        let (answers, received) = mpsc::channel();
        let mut browser = Browser { worker: Some((jobs, received)), asked: 2, ..Default::default() };
        let current =
            Item { path: "current.nksf".into(), name: "Current".into(), kind: deckard_library::Kind::Instrument, ..Default::default() };
        answers.send((2, Results { items: vec![current], ..Default::default() }, None)).unwrap();
        answers.send((1, Results::default(), Some("old failure".into()))).unwrap();
        assert!(browser.poll());
        assert_eq!(browser.results.items[0].name, "Current");
        assert!(browser.error.is_none());
        assert!(!browser.waiting());
    }

    #[test]
    fn prehear_file_resolves_ogg_and_wav_previews_for_instruments_effects_groups_and_sounds() {
        use deckard_library::Kind;
        let dir = std::env::temp_dir().join(format!("deckard_prehear_test_{}", std::process::id()));
        let previews = dir.join(".previews");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&previews).unwrap();

        let inst_ogg = previews.join("Lead.nksf.ogg");
        let fx_full_wav = previews.join("Chorus.nksfx.wav");
        let fx_wav = previews.join("Dub.wav");
        let grp_ogg = previews.join("Kit.mxgrp.ogg");
        let snd_ogg = previews.join("Kick.mxsnd.ogg");
        for p in [&inst_ogg, &fx_full_wav, &fx_wav, &grp_ogg, &snd_ogg] {
            std::fs::write(p, b"test").unwrap();
        }

        let item = |kind, file: &str| Item { kind, path: dir.join(file), name: file.into(), ..Default::default() };
        assert!(can_prehear(&item(Kind::Loop, "Break.wav")));
        assert_eq!(prehear_file(&item(Kind::Instrument, "Lead.nksf")), Some(inst_ogg));
        assert_eq!(prehear_file(&item(Kind::Effect, "Chorus.nksfx")), Some(fx_full_wav));
        assert_eq!(prehear_file(&item(Kind::Effect, "Dub.nksfx")), Some(fx_wav));
        assert_eq!(prehear_file(&item(Kind::Group, "Kit.mxgrp")), Some(grp_ogg));
        assert_eq!(prehear_file(&item(Kind::Sound, "Kick.mxsnd")), Some(snd_ogg));
        assert!(!can_prehear(&item(Kind::Instrument, "Missing.nksf")));
        assert!(!can_prehear(&item(Kind::Project, "Song.mxprj")));
        let mut app = crate::App::default();
        app.prehear_item(&item(Kind::Instrument, "Missing.nksf"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
