//! DECKARD's own index, `library.db3` in its settings folder: Maschine's user folders
//! (and library folders added in Preferences), scanned, and every file DECKARD saves,
//! added as it's saved. Maschine files carry their tags ([`mxprj::sound_info`]);
//! samples are listed untagged, a Loop when their name gives a tempo.

use crate::{Item, Kind, Query, Results};
use rusqlite::{Connection, params};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A folder to index.
#[derive(Debug, Clone, PartialEq)]
pub struct Root {
    pub path: PathBuf,
    /// Listed under Factory (a library folder) rather than User.
    pub factory: bool,
    /// The product its items show (a library folder's).
    pub product: Option<String>,
}

/// Maschine's user folders on this machine: Maschine 3's, and Maschine 2's when there.
pub fn default_roots() -> Vec<Root> {
    let Some(docs) = documents() else { return Vec::new() };
    ["Maschine 3", "Maschine 2"]
        .iter()
        .map(|m| docs.join("Native Instruments").join(m))
        .filter(|p| p.is_dir())
        .map(|path| Root { path, factory: false, product: None })
        .collect()
}

/// The user's Documents folder.
pub fn documents() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    Some(PathBuf::from(home).join("Documents"))
}

pub struct UserIndex {
    db: Connection,
}

/// Levels of a type are joined by US, types by RS.
const LEVEL: char = '\u{1f}';
const TYPE: char = '\u{1e}';

impl UserIndex {
    pub fn open(db: &Path) -> Result<UserIndex, String> {
        let err = |e: rusqlite::Error| format!("{}: {e}", db.display());
        let db = Connection::open(db).map_err(err)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS items (path TEXT PRIMARY KEY, kind INTEGER NOT NULL, user INTEGER NOT NULL,
                name TEXT NOT NULL, product TEXT, bank TEXT NOT NULL, types TEXT NOT NULL, characters TEXT NOT NULL,
                tempo REAL, author TEXT NOT NULL, mtime INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS favorites (path TEXT PRIMARY KEY);",
        )
        .map_err(|e| e.to_string())?;
        Ok(UserIndex { db })
    }

    /// Bring the index up to date with `roots`: new and changed files read, files no
    /// longer there dropped. Returns how many items the roots hold.
    pub fn scan(&mut self, roots: &[Root]) -> Result<usize, String> {
        let mut seen = HashSet::new();
        for root in roots {
            let mut files = Vec::new();
            walk(&root.path, &mut files, 0);
            for f in files {
                let mtime = mtime(&f);
                let known: Option<i64> = self.db.query_row("SELECT mtime FROM items WHERE path = ?1", [path_text(&f)], |r| r.get(0)).ok();
                if known != Some(mtime) {
                    self.add_file(&f, root)?;
                }
                seen.insert(path_text(&f));
            }
        }
        // Drop what's gone from under the roots.
        let mut gone = Vec::new();
        {
            let mut st = self.db.prepare("SELECT path FROM items").map_err(|e| e.to_string())?;
            let paths = st.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
            for p in paths.flatten() {
                let under = roots.iter().any(|r| Path::new(&p).starts_with(&r.path));
                if under && !seen.contains(&p) {
                    gone.push(p);
                }
            }
        }
        for p in gone {
            self.db.execute("DELETE FROM items WHERE path = ?1", [p]).map_err(|e| e.to_string())?;
        }
        Ok(seen.len())
    }

    /// Drop everything under `dir` (a library folder no longer listed).
    pub fn forget(&mut self, dir: &Path) -> Result<(), String> {
        let mut st = self.db.prepare("SELECT path FROM items").map_err(|e| e.to_string())?;
        let gone: Vec<String> = st
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .flatten()
            .filter(|p| Path::new(p).starts_with(dir))
            .collect();
        drop(st);
        for p in gone {
            self.db.execute("DELETE FROM items WHERE path = ?1", [p]).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Read one file into the index (one DECKARD just saved, or a scan's).
    pub fn add_file(&mut self, path: &Path, root: &Root) -> Result<(), String> {
        let Some(kind) = Kind::of_file(path) else { return Ok(()) };
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut item =
            Item { kind, user: !root.factory, name: stem, path: path.to_path_buf(), product: root.product.clone(), ..Item::default() };
        if Kind::of_extension(path.extension().and_then(|e| e.to_str()).unwrap_or("")).is_some() {
            if let Some(info) = std::fs::read(path).ok().and_then(|b| mxprj::Project::parse(&b).ok().and_then(|p| p.sound_info())) {
                if !info.name.is_empty() {
                    item.name = info.name.clone();
                }
                item.types = info.types();
                item.characters = info.characters();
                item.bank = info.bank.clone();
                item.author = info.author.clone();
                item.tempo = info.prop("\\@tempo").and_then(|t| t.parse().ok()).filter(|t: &f64| *t > 0.0);
            }
        } else {
            item.tempo = crate::names::tempo_in_name(&item.name);
        }
        self.add_entry(&item, mtime(path))
    }

    /// Put `item` in the index as it is.
    pub fn add_entry(&mut self, item: &Item, mtime: i64) -> Result<(), String> {
        let types: Vec<String> = item.types.iter().map(|t| t.join(&LEVEL.to_string())).collect();
        self.db
            .execute(
                "INSERT OR REPLACE INTO items VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    path_text(&item.path),
                    kind_index(item.kind),
                    item.user,
                    item.name,
                    item.product,
                    item.bank.join(&LEVEL.to_string()),
                    types.join(&TYPE.to_string()),
                    item.characters.join(&LEVEL.to_string()),
                    item.tempo,
                    item.author,
                    mtime
                ],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn query(&self, q: &Query) -> Result<Results, String> {
        let favorites = self.favorites();
        let mut st = self
            .db
            .prepare("SELECT path, name, product, bank, types, characters, tempo, author FROM items WHERE kind = ?1 AND user = ?2")
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map(params![kind_index(q.kind), q.user], |r| {
                let split = |s: String| s.split(LEVEL).filter(|x| !x.is_empty()).map(str::to_string).collect::<Vec<_>>();
                let path = PathBuf::from(r.get::<_, String>(0)?);
                Ok(Item {
                    kind: q.kind,
                    user: q.user,
                    favorite: favorites.contains(&path),
                    path,
                    name: r.get(1)?,
                    product: r.get(2)?,
                    bank: split(r.get(3)?),
                    types: r.get::<_, String>(4)?.split(TYPE).map(|t| split(t.to_string())).filter(|t| !t.is_empty()).collect(),
                    characters: split(r.get(5)?),
                    tempo: r.get(6)?,
                    author: r.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut products = std::collections::BTreeSet::new();
        let mut items = Vec::new();
        for item in rows.flatten() {
            if let Some(p) = &item.product {
                products.insert(p.clone());
            }
            if (q.product.is_none() || q.product == item.product) && q.admits(&item) {
                items.push(item);
            }
        }
        Ok(Results::of(items, q, products.into_iter().collect()))
    }

    pub fn set_favorite(&mut self, path: &Path, on: bool) -> Result<(), String> {
        let sql = if on { "INSERT OR IGNORE INTO favorites VALUES (?1)" } else { "DELETE FROM favorites WHERE path = ?1" };
        self.db.execute(sql, [path_text(path)]).map(|_| ()).map_err(|e| e.to_string())
    }

    pub fn favorites(&self) -> HashSet<PathBuf> {
        let Ok(mut st) = self.db.prepare("SELECT path FROM favorites") else { return HashSet::new() };
        st.query_map([], |r| r.get::<_, String>(0)).map(|rows| rows.flatten().map(PathBuf::from).collect()).unwrap_or_default()
    }
}

fn kind_index(k: Kind) -> i64 {
    Kind::ALL.iter().position(|x| *x == k).unwrap_or(0) as i64
}

fn path_text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn mtime(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

/// The files under `dir` the library lists (hidden folders and Maschine's `.previews`
/// skipped).
fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        match e.file_type() {
            Ok(t) if t.is_dir() && depth < 8 => walk(&p, out, depth + 1),
            Ok(t) if t.is_file() && Kind::of_file(&p).is_some() => out.push(p),
            _ => {}
        }
    }
}
