//! The browser's library (docs/superpowers/specs/2026-10-22-browser-design.md): what
//! Maschine's LIBRARY tab lists — projects, groups, sounds, instrument and effect
//! presets, loops and one-shots — filtered by Type and Character, Factory or User, by
//! product, by text and by favourites.
//!
//! Two sources: NI's own index (`komplete.db3`), read and never written ([`ni`]), and
//! DECKARD's ([`user`]) of Maschine's user folders and what DECKARD saves.

pub mod names;
pub mod ni;
#[doc(hidden)]
pub mod testing;
pub mod user;

use std::path::PathBuf;

/// A section of the LIBRARY tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Kind {
    Project,
    Group,
    Sound,
    Instrument,
    Effect,
    Loop,
    #[default]
    OneShot,
}

impl Kind {
    pub const ALL: [Kind; 7] = [Kind::Project, Kind::Group, Kind::Sound, Kind::Instrument, Kind::Effect, Kind::Loop, Kind::OneShot];

    /// The name NI's index gives the section (`p_sound_info_<name>_1`).
    pub fn table(self) -> &'static str {
        match self {
            Kind::Project => "Project",
            Kind::Group => "Group",
            Kind::Sound => "Sound",
            Kind::Instrument => "Instrument",
            Kind::Effect => "Effect",
            Kind::Loop => "Loop",
            Kind::OneShot => "Oneshot",
        }
    }

    /// The section of a Maschine file by its extension (samples are a Loop or a
    /// One-shot by their name: [`Kind::of_file`]).
    pub fn of_extension(ext: &str) -> Option<Kind> {
        match ext.to_ascii_lowercase().as_str() {
            "mxprj" => Some(Kind::Project),
            "mxgrp" => Some(Kind::Group),
            "mxsnd" => Some(Kind::Sound),
            "mxinst" => Some(Kind::Instrument),
            "mxfx" => Some(Kind::Effect),
            _ => None,
        }
    }

    /// The section of any file the browser lists: Maschine files by extension, samples
    /// a Loop when their name gives a tempo, else a One-shot.
    pub fn of_file(path: &std::path::Path) -> Option<Kind> {
        let ext = path.extension()?.to_str()?;
        if let Some(k) = Kind::of_extension(ext) {
            return Some(k);
        }
        let sample = ["wav", "aif", "aiff", "aifc"].iter().any(|e| ext.eq_ignore_ascii_case(e));
        let name = path.file_stem()?.to_string_lossy();
        sample.then(|| if names::tempo_in_name(&name).is_some() { Kind::Loop } else { Kind::OneShot })
    }
}

/// One thing the library lists.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Item {
    pub kind: Kind,
    pub user: bool,
    pub name: String,
    pub path: PathBuf,
    /// The product it came with ("Deep Matter"); none for the user's own.
    pub product: Option<String>,
    /// Library, Bank, Sub-bank.
    pub bank: Vec<String>,
    /// Each type's levels, the deepest of each only.
    pub types: Vec<Vec<String>>,
    pub characters: Vec<String>,
    pub tempo: Option<f64>,
    pub author: String,
    pub favorite: bool,
}

/// What the browser asks for.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Query {
    pub kind: Kind,
    /// User content (true) or the factory's.
    pub user: bool,
    pub product: Option<String>,
    pub bank: Vec<String>,
    /// The chosen type's levels (none: all).
    pub types: Vec<String>,
    /// All of these (AND).
    pub characters: Vec<String>,
    pub text: String,
    pub favorites: bool,
}

/// What a query found, and what to narrow it by.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Results {
    /// By name, ignoring case.
    pub items: Vec<Item>,
    /// The chosen type's next level (the top level when none is chosen), with how many
    /// of the items have each.
    pub types: Vec<(String, usize)>,
    pub characters: Vec<(String, usize)>,
    /// The products this section has (whatever product is chosen).
    pub products: Vec<String>,
}

impl Query {
    /// Whether `item` passes the bank, type, character, text and favourite filters
    /// (the section, user and product are the source's to apply).
    pub fn admits(&self, item: &Item) -> bool {
        let bank = item.bank.starts_with(&self.bank);
        let typed = self.types.is_empty() || item.types.iter().any(|t| t.starts_with(&self.types));
        let characters = self.characters.iter().all(|c| item.characters.contains(c));
        let text = self.text.is_empty() || item.name.to_lowercase().contains(&self.text.to_lowercase());
        bank && typed && characters && text && (!self.favorites || item.favorite)
    }
}

impl Results {
    /// Results of `items` (already filtered): sorted, with the facets counted.
    pub fn of(mut items: Vec<Item>, q: &Query, products: Vec<String>) -> Results {
        items.sort_by_key(|i| i.name.to_lowercase());
        let depth = q.types.len();
        let mut types = std::collections::BTreeMap::<String, usize>::new();
        let mut characters = std::collections::BTreeMap::<String, usize>::new();
        for i in &items {
            let next: std::collections::BTreeSet<&String> =
                i.types.iter().filter(|t| t.starts_with(&q.types)).filter_map(|t| t.get(depth)).collect();
            for n in next {
                *types.entry(n.clone()).or_default() += 1;
            }
            for c in &i.characters {
                *characters.entry(c.clone()).or_default() += 1;
            }
        }
        Results { items, types: types.into_iter().collect(), characters: characters.into_iter().collect(), products }
    }
}

/// The library the browser shows: NI's index (when there) and DECKARD's, merged.
pub struct Library {
    ni: Option<ni::NiIndex>,
    own: user::UserIndex,
    /// Why NI's index isn't there, if it should be.
    pub ni_error: Option<String>,
}

impl Library {
    /// NI's index at `ni` (read-only), DECKARD's at `own` (created if new).
    pub fn new(ni: Option<PathBuf>, own: &std::path::Path) -> Result<Library, String> {
        let (ni, ni_error) = match ni.map(|p| ni::NiIndex::open(&p)) {
            Some(Ok(n)) => (Some(n), None),
            Some(Err(e)) => (None, Some(e)),
            None => (None, None),
        };
        Ok(Library { ni, own: user::UserIndex::open(own)?, ni_error })
    }

    /// Both sources' results, one entry per file (DECKARD's when both list it), the
    /// favourites marked. A source that fails (NI's busy) is left out and its error
    /// kept in `ni_error`.
    pub fn query(&mut self, q: &Query) -> Results {
        let open = Query { favorites: false, ..q.clone() };
        let own = self.own.query(&open).unwrap_or_default();
        let theirs = match &self.ni {
            Some(ni) => match ni.query(&open) {
                Ok(r) => {
                    self.ni_error = None;
                    r
                }
                Err(e) => {
                    self.ni_error = Some(e);
                    Results::default()
                }
            },
            None => Results::default(),
        };
        let favorites = self.own.favorites();
        let mine: std::collections::HashSet<&PathBuf> = own.items.iter().map(|i| &i.path).collect();
        let mut items: Vec<Item> = theirs.items.iter().filter(|i| !mine.contains(&i.path)).cloned().collect();
        items.extend(own.items.iter().cloned());
        for i in &mut items {
            i.favorite = favorites.contains(&i.path);
        }
        items.retain(|i| !q.favorites || i.favorite);
        let products: std::collections::BTreeSet<String> = theirs.products.into_iter().chain(own.products).collect();
        Results::of(items, q, products.into_iter().collect())
    }

    /// Re-read DECKARD's folders: Maschine's user folders and `extra` (library folders).
    pub fn rescan(&mut self, extra: &[user::Root]) -> Result<usize, String> {
        let mut roots = user::default_roots();
        roots.extend(extra.iter().cloned());
        self.own.scan(&roots)
    }

    /// Bring DECKARD's index up to date with `roots`.
    pub fn scan(&mut self, roots: &[user::Root]) -> Result<usize, String> {
        self.own.scan(roots)
    }

    /// Drop what DECKARD's index has under `dir`.
    pub fn forget(&mut self, dir: &std::path::Path) {
        let _ = self.own.forget(dir);
    }

    /// A file DECKARD just saved, listed under User at once.
    pub fn add_saved(&mut self, path: &std::path::Path) {
        let root = user::Root { path: path.parent().map(|p| p.to_path_buf()).unwrap_or_default(), factory: false, product: None };
        let _ = self.own.add_file(path, &root);
    }

    /// Put `item` in DECKARD's index as it is.
    pub fn add_entry(&mut self, item: Item) {
        let _ = self.own.add_entry(&item, 0);
    }

    pub fn set_favorite(&mut self, path: &std::path::Path, on: bool) {
        let _ = self.own.set_favorite(path, on);
    }
}
