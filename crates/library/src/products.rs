//! NI product-category metadata, read-only. Never opens a controller script or writes NI data.
use std::{collections::BTreeMap, path::Path};
pub fn categories(database: &Path, kind: crate::Kind) -> Result<BTreeMap<String, Vec<String>>, String> {
    use rusqlite::{Connection, OpenFlags};
    let table = match kind {
        crate::Kind::Instrument => "Instruments",
        crate::Kind::Effect => "FX",
        crate::Kind::Group => "Groups",
        crate::Kind::Loop => "Loops",
        crate::Kind::OneShot => "Oneshots",
        crate::Kind::Sound => "Sounds",
        crate::Kind::Project => "Projects",
    };
    let db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
    let mut stmt = db
        .prepare(&format!(
            "SELECT i.Item,c.CategoryName FROM {table} i JOIN Categories c ON c.CategoryId=i.CategoryId ORDER BY c.seqidx,i.seqidx"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(|e| e.to_string())?;
    let mut out = BTreeMap::<String, Vec<String>>::new();
    for row in rows {
        let (product, category) = row.map_err(|e| e.to_string())?;
        out.entry(category).or_default().push(product);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remaining_category_metadata_is_read_only_and_kind_specific() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("categories.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE Categories(CategoryId INTEGER,CategoryName TEXT,seqidx INTEGER); CREATE TABLE Instruments(Item TEXT,CategoryId INTEGER,seqidx INTEGER); CREATE TABLE FX(Item TEXT,CategoryId INTEGER,seqidx INTEGER); INSERT INTO Categories VALUES(1,'Sampled Instruments',1); INSERT INTO Instruments VALUES('M1 Le',1,1); INSERT INTO FX VALUES('FX',1,1);").unwrap();
        drop(db);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(categories(&path, crate::Kind::Instrument).unwrap()["Sampled Instruments"], ["M1 Le"]);
        assert_eq!(categories(&path, crate::Kind::Effect).unwrap()["Sampled Instruments"], ["FX"]);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(categories(&temp.path().join("absent.db"), crate::Kind::Instrument).is_err());
        assert!(!temp.path().join("absent.db").exists());
    }
    #[test]
    fn remaining_category_and_vendor_filters_compose_with_favorites() {
        let item = crate::Item { product: Some("M1 Le".into()), vendor: "KORG".into(), favorite: true, ..Default::default() };
        let mut q = crate::Query { products: vec!["m1 le".into()], vendor: Some("korg".into()), favorites: true, ..Default::default() };
        assert!(q.admits(&item));
        q.vendor = Some("NI".into());
        assert!(!q.admits(&item));
        q.vendor = None;
        q.products = vec!["Kontakt".into()];
        assert!(!q.admits(&item));
    }
}
