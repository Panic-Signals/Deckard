//! Finding sample files on this machine.
//!
//! Projects store library samples as (library name, path inside the library) plus
//! the library root on the machine that saved the file, which is often another
//! computer or OS. Native Instruments installers record each library's location in
//! `/Library/Preferences/com.native-instruments.<library>.plist` (`ContentDir`, an
//! HFS-style path); that is tried first.

use deckard_model::SampleRef;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Resolver {
    /// Directory of the project file (for relative paths and its own library).
    project_dir: Option<PathBuf>,
    roots: HashMap<String, Vec<PathBuf>>,
}

impl Resolver {
    pub fn new(project_path: Option<&Path>) -> Resolver {
        Resolver { project_dir: project_path.and_then(Path::parent).map(Path::to_path_buf), roots: HashMap::new() }
    }

    /// The local file for a sample reference, if one exists.
    pub fn resolve(&mut self, s: &SampleRef) -> Option<PathBuf> {
        if s.path.is_empty() {
            return None;
        }
        let path = s.path.replace('\\', "/");
        // Library-relative paths (kind 2) are sometimes written with leading slashes.
        let rel = if s.kind == 2 { path.trim_start_matches('/') } else { path.as_str() };
        let p = Path::new(rel);
        if p.is_absolute() {
            return p.exists().then(|| p.to_path_buf());
        }
        let mut candidates = self.library_roots(&s.library).clone();
        candidates.push(PathBuf::from(s.library_root.replace('\\', "/")));
        if let Some(dir) = &self.project_dir {
            // a project inside a library: <root>/Projects/x.mxprj
            if let Some(parent) = dir.parent() {
                candidates.push(parent.to_path_buf());
            }
            candidates.push(dir.clone());
        }
        candidates.into_iter().map(|root| root.join(p)).find(|c| c.is_file())
    }

    fn library_roots(&mut self, library: &str) -> &Vec<PathBuf> {
        self.roots.entry(library.to_string()).or_insert_with(|| library_roots(library))
    }
}

/// Plausible install locations of a content library.
fn library_roots(library: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if library.is_empty() {
        return out;
    }
    let plist = PathBuf::from(format!("/Library/Preferences/com.native-instruments.{library}.plist"));
    if let Ok(v) = plist::Value::from_file(&plist)
        && let Some(dir) = v.as_dictionary().and_then(|d| d.get("ContentDir")).and_then(|d| d.as_string())
    {
        out.extend(hfs_to_posix(dir));
    }
    // Default install folder: "<name> Library", where factory libraries drop "Factory".
    let short = library.trim_end_matches(" Library").replace(" Factory", "");
    out.push(PathBuf::from(format!("/Users/Shared/{short} Library")));
    out.push(PathBuf::from(format!("/Users/Shared/{library}")));
    out
}

/// `Macintosh HD:Users:Shared:X Library:` -> `/Users/Shared/X Library` (boot volume)
/// or `/Volumes/<vol>/...` (other volumes). Both are returned when unsure.
pub fn hfs_to_posix(hfs: &str) -> Vec<PathBuf> {
    let mut parts = hfs.trim_end_matches(':').split(':');
    let Some(volume) = parts.next() else { return Vec::new() };
    let rest: Vec<&str> = parts.collect();
    let rest = rest.join("/");
    vec![PathBuf::from(format!("/{rest}")), PathBuf::from(format!("/Volumes/{volume}/{rest}"))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hfs_paths() {
        assert_eq!(
            hfs_to_posix("Macintosh HD:Users:Shared:Maschine 2 Library:"),
            vec![PathBuf::from("/Users/Shared/Maschine 2 Library"), PathBuf::from("/Volumes/Macintosh HD/Users/Shared/Maschine 2 Library")]
        );
    }
}
