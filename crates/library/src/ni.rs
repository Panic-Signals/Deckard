//! NI's library index: the SQLite file Maschine keeps of every product and user file
//! it has scanned (`komplete.db3`; the schema is in the design doc). DECKARD opens it
//! read-only and never writes it; Maschine may be using it (WAL), and when it is busy a
//! query fails rather than waits.
//!
//! Per section a table `p_sound_info_<Kind>_<1|2>` (1: user, 2: factory) lists the
//! items, with their bank chain (`entry1`–`entry3`); `k_sound_info` has the path and
//! tempo, `k_content_path` the product (`alias`, for factory content, `content_type`
//! 2), `k_category` (via `k_sound_info_category`) the Types, `k_mode` (via
//! `k_sound_info_mode`) the Characters.

use crate::{Item, Query, Results};
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct NiIndex {
    path: PathBuf,
    /// The connection, and whether it was opened immutable (no `-wal` then).
    db: std::cell::RefCell<(Connection, bool)>,
}

/// Where Maschine 3 keeps its index on this machine, if it's there.
pub fn default_path() -> Option<PathBuf> {
    let p = if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Native Instruments/Maschine 3/komplete.db3")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Native Instruments\\Maschine 3\\komplete.db3")
    } else {
        return None;
    };
    p.is_file().then_some(p)
}

impl NiIndex {
    /// Open it read-only (see [`connect`]).
    pub fn open(path: &Path) -> Result<NiIndex, String> {
        Ok(NiIndex { path: path.to_path_buf(), db: std::cell::RefCell::new(connect(path)?) })
    }

    pub fn query(&self, q: &Query) -> Result<Results, String> {
        // Maschine started since: read through its WAL from now on.
        if self.db.borrow().1 && wal_of(&self.path).exists() {
            *self.db.borrow_mut() = connect(&self.path)?;
        }
        let db = self.db.borrow();
        let db = &db.0;
        let table = format!("p_sound_info_{}_{}", q.kind.table(), if q.user { 1 } else { 2 });
        let err = |e: rusqlite::Error| format!("NI's library: {e}");
        // A section the index doesn't have is empty.
        let exists: bool = db
            .query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", [&table], |r| r.get::<_, i64>(0))
            .map_err(err)?
            > 0;
        if !exists {
            return Ok(Results::default());
        }

        let mut types: HashMap<i64, Vec<Vec<String>>> = HashMap::new();
        let sql = format!(
            "SELECT p.id, c.category, c.subcategory, c.subsubcategory FROM {table} p \
             JOIN k_sound_info_category sc ON sc.sound_info_id = p.id JOIN k_category c ON c.id = sc.category_id"
        );
        let mut st = db.prepare(&sql).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            let levels: Vec<String> =
                (1..4).filter_map(|i| r.get::<_, Option<String>>(i).ok().flatten()).take_while(|s| !s.is_empty()).collect();
            if !levels.is_empty() {
                types.entry(r.get(0).map_err(err)?).or_default().push(levels);
            }
        }
        let mut characters: HashMap<i64, Vec<String>> = HashMap::new();
        let sql = format!(
            "SELECT p.id, m.name FROM {table} p \
             JOIN k_sound_info_mode sm ON sm.sound_info_id = p.id JOIN k_mode m ON m.id = sm.mode_id"
        );
        let mut st = db.prepare(&sql).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            characters.entry(r.get(0).map_err(err)?).or_default().push(r.get(1).map_err(err)?);
        }

        let sql = format!(
            "SELECT p.id, k.name, k.file_name, k.tempo, coalesce(k.author, ''), p.entry1, p.entry2, p.entry3, \
             CASE WHEN c.content_type = 2 THEN c.alias END \
             FROM {table} p JOIN k_sound_info k ON k.id = p.id LEFT JOIN k_content_path c ON c.id = k.content_path_id \
             WHERE k.hidden_for = 0"
        );
        let mut st = db.prepare(&sql).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        let mut items = Vec::new();
        let mut products = std::collections::BTreeSet::new();
        while let Some(r) = rows.next().map_err(err)? {
            let id: i64 = r.get(0).map_err(err)?;
            let product: Option<String> = r.get(8).map_err(err)?;
            if let Some(p) = &product {
                products.insert(p.clone());
            }
            if q.product.is_some() && q.product != product {
                continue;
            }
            let name: String = r.get(1).map_err(err)?;
            let tempo = r.get::<_, Option<f64>>(3).map_err(err)?.filter(|t| *t > 0.0).or_else(|| crate::names::tempo_in_name(&name));
            let item = Item {
                kind: q.kind,
                user: q.user,
                path: PathBuf::from(r.get::<_, String>(2).map_err(err)?),
                product,
                bank: (5..8).filter_map(|i| r.get::<_, Option<String>>(i).ok().flatten()).filter(|s| !s.is_empty()).collect(),
                types: deepest(types.remove(&id).unwrap_or_default()),
                characters: characters.remove(&id).unwrap_or_default(),
                tempo,
                author: r.get(4).map_err(err)?,
                favorite: false,
                name,
            };
            if q.admits(&item) {
                items.push(item);
            }
        }
        Ok(Results::of(items, q, products.into_iter().collect()))
    }
}

/// Open it read-only. While Maschine has it open (its `-wal` is there) the read
/// shares Maschine's WAL; otherwise the file is complete as it is and is opened
/// immutable, so SQLite leaves no `-wal`/`-shm` of its own in NI's folder.
fn connect(path: &Path) -> Result<(Connection, bool), String> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
    let immutable = !wal_of(path).exists();
    let target = format!("file:{}{}", uri_path(path), if immutable { "?immutable=1" } else { "" });
    let db = Connection::open_with_flags(&target, flags).map_err(|e| format!("{}: {e}", path.display()))?;
    db.busy_timeout(Duration::from_millis(500)).map_err(|e| e.to_string())?;
    Ok((db, immutable))
}

fn wal_of(path: &Path) -> PathBuf {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    PathBuf::from(wal)
}

/// `path` for a `file:` URI: `%`, `?` and `#` escaped, `\` as `/` (Windows).
fn uri_path(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = s.replace('%', "%25").replace('?', "%3f").replace('#', "%23");
    if s.starts_with('/') { s } else { format!("/{s}") }
}

/// Only the deepest of each type (an item is tagged with "Drums" and "Drums › Kick").
fn deepest(all: Vec<Vec<String>>) -> Vec<Vec<String>> {
    all.iter().filter(|t| !all.iter().any(|u| u.len() > t.len() && u.starts_with(t))).cloned().collect()
}
