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
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct NiIndex {
    path: PathBuf,
    /// The connection, and whether it was opened immutable (no `-wal` then).
    db: std::cell::RefCell<(Connection, bool)>,
    /// Cached items for factory content paths marked pending rescan (`state = 2`) with no
    /// indexed `k_sound_info` rows yet.
    unindexed: std::cell::RefCell<Option<Vec<Item>>>,
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

/// The favourites NI's products share, beside the products' own folders
/// (`Shared/favorites.db3`): a table of ids, which the index's items carry.
fn favorites_of(index: &Path) -> Option<PathBuf> {
    Some(index.parent()?.parent()?.join("Shared").join("favorites.db3"))
}

/// Maschine's favourites, read afresh (it may have changed them). None without the file.
fn favorite_ids(index: &Path) -> HashSet<String> {
    let read = || -> Option<HashSet<String>> {
        let file = favorites_of(index).filter(|f| f.is_file())?;
        let (db, _) = connect(&file).ok()?;
        let mut st = db.prepare("SELECT id FROM favorites").ok()?;
        let ids = st.query_map([], |r| r.get::<_, String>(0)).ok()?.flatten().collect();
        Some(ids)
    };
    read().unwrap_or_default()
}

/// Up to a few `.nksf` / `.nksfx` / `.nfm8` / `.nmsv` preset files for `plugin` (one
/// per bank/folder), from NI's library index if present.
pub fn nks_presets_for(plugin: &str) -> Vec<PathBuf> {
    let Some(path) = default_path() else { return Vec::new() };
    let Ok((db, _)) = connect(&path) else { return Vec::new() };
    let mut out = Vec::new();
    for table in ["p_sound_info_Instrument_1", "p_sound_info_Instrument_2", "p_sound_info_Effect_1", "p_sound_info_Effect_2"] {
        let sql = format!(
            "SELECT k.file_name FROM {table} p JOIN k_sound_info k ON k.id = p.id \
             WHERE (lower(p.entry1) = lower(?1) OR lower(p.entry1) LIKE lower(?1) || ' %') \
             AND lower(k.file_ext) IN ('nksf', 'nksfx', 'nfm8', 'nmsv') \
             GROUP BY coalesce(p.entry2, ''), coalesce(k.sub_path, '') \
             ORDER BY CASE WHEN p.entry2 IS NULL OR p.entry2 = 'Factory' THEN 0 ELSE 1 END, k.id \
             LIMIT 10"
        );
        let Ok(mut st) = db.prepare(&sql) else { continue };
        let Ok(mut rows) = st.query([plugin]) else { continue };
        while let Ok(Some(r)) = rows.next() {
            if let Ok(file) = r.get::<_, String>(0) {
                let p = PathBuf::from(file);
                if p.is_file() && !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }
    out
}

impl NiIndex {
    /// Open it read-only (see [`connect`]).
    pub fn open(path: &Path) -> Result<NiIndex, String> {
        Ok(NiIndex { path: path.to_path_buf(), db: std::cell::RefCell::new(connect(path)?), unindexed: std::cell::RefCell::new(None) })
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

        let has_vendor = db
            .prepare("PRAGMA table_info(k_sound_info)")
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map([], |r| r.get::<_, String>(1)).ok().map(|rows| rows.filter_map(Result::ok).any(|name| name == "vendor"))
            })
            .unwrap_or(false);
        let vendor = if has_vendor { "coalesce(k.vendor, '')" } else { "''" };
        // A product Maschine has marked as gone (state 4) keeps its rows, but its files aren't
        // there to load. An older index may have no state.
        let has_state: bool = db
            .query_row("SELECT count(*) FROM pragma_table_info('k_content_path') WHERE name = 'state'", [], |r| r.get::<_, i64>(0))
            .map_err(err)?
            > 0;
        let installed = if has_state { "AND coalesce(c.state, 0) <> 4" } else { "" };
        let favorites = favorite_ids(&self.path);
        // The section's table has the id (`k_sound_info`'s may be empty); an older index may not.
        let has_id: bool = db
            .query_row("SELECT count(*) FROM pragma_table_info(?1) WHERE name = 'favorite_id'", [&table], |r| r.get::<_, i64>(0))
            .map_err(err)?
            > 0;
        let favorite_id = if has_id { "p.favorite_id" } else { "NULL" };
        let sql = format!(
            "SELECT p.id, k.name, k.file_name, k.tempo, coalesce(k.author, ''), p.entry1, p.entry2, p.entry3, \
             COALESCE(NULLIF(p.entry1, ''), CASE WHEN c.content_type = 2 THEN c.alias END), {favorite_id}, {vendor} \
             FROM {table} p JOIN k_sound_info k ON k.id = p.id LEFT JOIN k_content_path c ON c.id = k.content_path_id \
             WHERE k.hidden_for = 0 {installed}"
        );
        let mut st = db.prepare(&sql).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        let mut items = Vec::new();
        let mut products = std::collections::BTreeSet::new();
        while let Some(r) = rows.next().map_err(err)? {
            let id: i64 = r.get(0).map_err(err)?;
            let product: Option<String> = r.get(8).map_err(err)?;
            let vendor_str: String = r.get(10).unwrap_or_default();
            let bank: Vec<String> = (5..8).filter_map(|i| r.get::<_, Option<String>>(i).ok().flatten()).filter(|s| !s.is_empty()).collect();
            let matches_facet = (q.products.is_empty()
                || product.as_ref().is_some_and(|p| q.products.iter().any(|want| want.eq_ignore_ascii_case(p)))
                || bank.first().is_some_and(|p| q.products.iter().any(|want| want.eq_ignore_ascii_case(p))))
                && q.vendor.as_ref().is_none_or(|v| v.eq_ignore_ascii_case(&vendor_str));
            if matches_facet && let Some(p) = &product {
                products.insert(p.clone());
            }
            if q.product.is_some() && q.product != product {
                continue;
            }
            let name: String = r.get(1).map_err(err)?;
            let tempo = r.get::<_, Option<f64>>(3).map_err(err)?.filter(|t| *t > 0.0).or_else(|| crate::names::tempo_in_name(&name));
            let item = Item {
                vendor: vendor_str,
                kind: q.kind,
                user: q.user,
                path: PathBuf::from(r.get::<_, String>(2).map_err(err)?),
                product,
                bank,
                types: deepest(types.remove(&id).unwrap_or_default()),
                characters: characters.remove(&id).unwrap_or_default(),
                tempo,
                author: r.get(4).map_err(err)?,
                favorite: r.get::<_, Option<String>>(9).map_err(err)?.is_some_and(|id| favorites.contains(&id)),
                name,
            };
            if q.admits(&item) {
                items.push(item);
            }
        }
        if !q.user && has_state {
            if self.unindexed.borrow().is_none() {
                *self.unindexed.borrow_mut() = Some(scan_unindexed_content_paths(db));
            }
            if let Some(extra) = self.unindexed.borrow().as_ref() {
                for item in extra.iter().filter(|i| i.kind == q.kind) {
                    let matches_facet = (q.products.is_empty()
                        || item.product.as_ref().is_some_and(|p| q.products.iter().any(|want| want.eq_ignore_ascii_case(p)))
                        || item.bank.first().is_some_and(|p| q.products.iter().any(|want| want.eq_ignore_ascii_case(p))))
                        && q.vendor.as_ref().is_none_or(|v| v.eq_ignore_ascii_case(&item.vendor));
                    if matches_facet && let Some(p) = &item.product {
                        products.insert(p.clone());
                    }
                    if (q.product.is_none() || q.product == item.product) && q.admits(item) {
                        items.push(item.clone());
                    }
                }
            }
        }
        Ok(Results::of(items, q, products.into_iter().collect()))
    }
}

fn normalize_content_alias(alias: &str) -> String {
    for suffix in [" Pro Factory Library", " Factory Library", " Factory Content", " Factory Presets", " Factory"] {
        if let Some(stripped) = alias.strip_suffix(suffix)
            && !stripped.is_empty()
        {
            return stripped.to_string();
        }
    }
    alias.to_string()
}

fn is_fx_product(product: &str) -> bool {
    matches!(
        product,
        "Filter MS-20" | "Guitar Rig 6" | "Infiltrator" | "OVox" | "Phasis" | "Raum" | "Replika XT" | "Solid Bus Comp" | "Transit"
    )
}

fn scan_unindexed_content_paths(db: &Connection) -> Vec<Item> {
    let sql = "SELECT c.alias, c.path FROM k_content_path c \
               WHERE c.content_type = 2 AND c.state = 2 AND coalesce(c.alias, '') <> '' \
               AND NOT EXISTS (SELECT 1 FROM k_sound_info k WHERE k.content_path_id = c.id AND k.hidden_for = 0)";
    let Ok(mut st) = db.prepare(sql) else { return Vec::new() };
    let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (alias, dir) in rows.flatten() {
        if alias.eq_ignore_ascii_case("Freelance Soundlabs") {
            continue;
        }
        let root = PathBuf::from(&dir);
        if !root.is_dir() {
            continue;
        }
        let product = normalize_content_alias(&alias);
        let fx_default = is_fx_product(&product);
        walk_unindexed_dir(&root, &product, fx_default, &mut out);
    }
    out
}

fn walk_unindexed_dir(dir: &Path, product: &str, fx_default: bool, out: &mut Vec<Item>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.eq_ignore_ascii_case("Snapshots2")
                || name.eq_ignore_ascii_case("Samples")
                || name.eq_ignore_ascii_case("Documentation")
                || name.eq_ignore_ascii_case("Pictures")
                || name.eq_ignore_ascii_case("Resources")
            {
                continue;
            }
            walk_unindexed_dir(&path, product, fx_default, out);
        } else if ft.is_file() {
            let Some(ext) = path.extension().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase()) else { continue };
            let kind = match ext.as_str() {
                "mxprj" => crate::Kind::Project,
                "mxgrp" => crate::Kind::Group,
                "mxsnd" => crate::Kind::Sound,
                "mxfx" | "nksfx" => crate::Kind::Effect,
                "mxinst" | "nksn" | "nki" | "nmsv" | "nfm8" | "nrkt" | "rkplr" | "ens" => crate::Kind::Instrument,
                "nksf" => {
                    if fx_default {
                        crate::Kind::Effect
                    } else {
                        crate::Kind::Instrument
                    }
                }
                _ => continue,
            };
            let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
            let tempo = crate::names::tempo_in_name(&stem);
            out.push(Item {
                vendor: "Native Instruments".into(),
                kind,
                user: false,
                name: stem,
                path,
                product: Some(product.to_string()),
                bank: vec![product.to_string()],
                types: Vec::new(),
                characters: Vec::new(),
                tempo,
                author: String::new(),
                favorite: false,
            });
        }
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
