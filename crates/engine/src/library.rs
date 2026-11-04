//! Finding sample files on this machine.
//!
//! Projects store library samples as (library name, path inside the library) plus
//! the library root on the machine that saved the file, which is often another
//! computer or OS. Native Instruments installers record each library's location in
//! `/Library/Preferences/com.native-instruments.<library>.plist` (`ContentDir`, an
//! HFS-style path) or, on Windows, `HKLM\SOFTWARE\Native Instruments\<library>`
//! (`ContentDir`); that is tried first.

use deckard_model::SampleRef;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Resolver {
    /// Directory of the project file (for relative paths and its own library).
    project_dir: Option<PathBuf>,
    /// Folders the user added (Preferences ▸ Library), searched like installed libraries.
    extra: Vec<PathBuf>,
    roots: HashMap<String, Vec<PathBuf>>,
}

impl Resolver {
    pub fn new(project_path: Option<&Path>) -> Resolver {
        Resolver { project_dir: project_path.and_then(Path::parent).map(Path::to_path_buf), extra: Vec::new(), roots: HashMap::new() }
    }

    /// Also look in `dirs` (a library folder added in Preferences, an expansion's folder).
    pub fn set_extra(&mut self, dirs: &[PathBuf]) {
        self.extra = dirs.to_vec();
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
        candidates.extend(self.extra.iter().cloned());
        candidates.push(PathBuf::from(s.library_root.replace('\\', "/")));
        // The root recorded in the file is the machine that authored it
        // (`/Volumes/…/Aquarius Earth Library`). Its folder name is the library
        // installed here, which the product name ("Maschine 2.0 Library") is not.
        if let Some(name) = saved_root_name(&s.library_root)
            && let Some(docs) = public_documents()
        {
            candidates.push(docs.join(name));
        }
        if let Some(dir) = &self.project_dir {
            // a project inside a library: <root>/Projects/x.mxprj
            if let Some(parent) = dir.parent() {
                candidates.push(parent.to_path_buf());
            }
            candidates.push(dir.clone());
        }
        let file = p.to_path_buf();
        if let Some(found) = candidates.into_iter().map(|root| root.join(&file)).find(|c| c.is_file()) {
            return Some(found);
        }
        // Every expansion group names its library "Maschine 2.0 Library", so the
        // product key never matches. The relative path lives in one installed ContentDir.
        installed_content_dirs().iter().map(|root| root.join(&file)).find(|c| c.is_file())
    }

    fn library_roots(&mut self, library: &str) -> &Vec<PathBuf> {
        self.roots.entry(library.to_string()).or_insert_with(|| library_roots(library))
    }
}

/// The library folder named at the end of a saved root
/// (`/Volumes/ohne titel 1/+Expansions/Aquarius Earth Library` → `Aquarius Earth Library`).
fn saved_root_name(library_root: &str) -> Option<String> {
    let root = library_root.replace('\\', "/");
    let name = root.trim_matches('/').rsplit('/').next()?.trim();
    (!name.is_empty() && !name.contains(':')).then(|| name.to_string())
}

/// Names a library is registered under. A group file says "Elastic Thump Library";
/// Native Instruments' installer records the key "Elastic Thump".
fn library_key_names(library: &str) -> Vec<String> {
    let mut names = vec![library.to_string()];
    let short = library.trim_end_matches(" Library");
    if short != library {
        names.push(short.to_string());
    }
    let factory = short.trim_end_matches(" Factory");
    if factory != short {
        names.push(factory.to_string());
    }
    names
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
    // Windows: HKLM\SOFTWARE\Native Instruments\<library>\ContentDir.
    #[cfg(windows)]
    out.extend(windows_content_dirs(library));
    // Default install folder: "<name> Library", where factory libraries drop "Factory".
    let short = library.trim_end_matches(" Library").replace(" Factory", "");
    out.push(PathBuf::from(format!("/Users/Shared/{short} Library")));
    out.push(PathBuf::from(format!("/Users/Shared/{library}")));
    if let Some(public) = public_documents() {
        out.push(public.join(format!("{short} Library")));
        out.push(public.join(library));
    }
    out
}

/// `C:\Users\Public\Documents`, where Native Instruments installs expansion libraries.
fn public_documents() -> Option<PathBuf> {
    let public = std::env::var_os("PUBLIC")?;
    Some(PathBuf::from(public).join("Documents"))
}

/// Every Native Instruments content library installed on this machine (`ContentDir`).
fn installed_content_dirs() -> &'static Vec<PathBuf> {
    use std::sync::OnceLock;
    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRS.get_or_init(read_installed_content_dirs)
}

#[cfg(windows)]
fn read_installed_content_dirs() -> Vec<PathBuf> {
    const HKLM: isize = 0x80000002u32 as i32 as isize;
    let mut out = Vec::new();
    for prefix in ["SOFTWARE\\Native Instruments", "SOFTWARE\\WOW6432Node\\Native Instruments"] {
        for name in registry_subkeys(HKLM, prefix) {
            if let Some(dir) = registry_sz(HKLM, &format!("{prefix}\\{name}"), "ContentDir") {
                let path = PathBuf::from(dir);
                if path.is_dir() && !out.contains(&path) {
                    out.push(path);
                }
            }
        }
    }
    out
}

#[cfg(not(windows))]
fn read_installed_content_dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// Subkey names of `subkey`.
#[cfg(windows)]
fn registry_subkeys(root: isize, subkey: &str) -> Vec<String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    unsafe extern "system" {
        fn RegOpenKeyExW(key: isize, sub: *const u16, options: u32, sam: u32, result: *mut isize) -> i32;
        fn RegEnumKeyExW(
            key: isize,
            index: u32,
            name: *mut u16,
            name_len: *mut u32,
            reserved: *mut u32,
            class: *mut u16,
            class_len: *mut u32,
            time: *mut u8,
        ) -> i32;
        fn RegCloseKey(key: isize) -> i32;
    }
    const KEY_READ: u32 = 0x20019;
    let wide = |s: &str| OsStr::new(s).encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let mut key = 0isize;
    if unsafe { RegOpenKeyExW(root, wide(subkey).as_ptr(), 0, KEY_READ, &mut key) } != 0 {
        return Vec::new();
    }
    let mut names = Vec::new();
    let mut index = 0u32;
    loop {
        let mut buf = [0u16; 256];
        let mut len = buf.len() as u32;
        // SAFETY: `buf` holds `len` wide characters; 259 is ERROR_NO_MORE_ITEMS.
        let rc = unsafe {
            RegEnumKeyExW(
                key,
                index,
                buf.as_mut_ptr(),
                &mut len,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if rc != 0 {
            break;
        }
        names.push(std::ffi::OsString::from_wide(&buf[..len as usize]).to_string_lossy().into_owned());
        index += 1;
    }
    unsafe { RegCloseKey(key) };
    names
}

/// ContentDir from the Native Instruments registry keys for `library`.
#[cfg(windows)]
fn windows_content_dirs(library: &str) -> Vec<PathBuf> {
    const HKLM: isize = 0x80000002u32 as i32 as isize;
    let mut out = Vec::new();
    for name in library_key_names(library) {
        for prefix in ["SOFTWARE\\Native Instruments", "SOFTWARE\\WOW6432Node\\Native Instruments"] {
            let sub = format!("{prefix}\\{name}");
            for value in ["ContentDir", "ContentDir64", "InstallDir"] {
                if let Some(dir) = registry_sz(HKLM, &sub, value) {
                    let path = PathBuf::from(dir);
                    if path.is_dir() {
                        out.push(path);
                    }
                }
            }
        }
    }
    out
}

/// A registry string value, if the key exists.
#[cfg(windows)]
fn registry_sz(root: isize, subkey: &str, value: &str) -> Option<String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    unsafe extern "system" {
        fn RegOpenKeyExW(key: isize, sub: *const u16, options: u32, sam: u32, result: *mut isize) -> i32;
        fn RegQueryValueExW(key: isize, name: *const u16, reserved: *mut u32, typ: *mut u32, data: *mut u8, len: *mut u32) -> i32;
        fn RegCloseKey(key: isize) -> i32;
    }
    const KEY_READ: u32 = 0x20019;
    let wide = |s: &str| OsStr::new(s).encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let mut key = 0isize;
    // SAFETY: the subkey is a null-terminated wide string; the key is closed below.
    if unsafe { RegOpenKeyExW(root, wide(subkey).as_ptr(), 0, KEY_READ, &mut key) } != 0 {
        return None;
    }
    let name = wide(value);
    let mut typ = 0u32;
    let mut len = 0u32;
    // SAFETY: a size query (null buffer) against the open key.
    let sized = unsafe { RegQueryValueExW(key, name.as_ptr(), std::ptr::null_mut(), &mut typ, std::ptr::null_mut(), &mut len) };
    if sized != 0 {
        unsafe { RegCloseKey(key) };
        return None;
    }
    let mut buf = vec![0u16; len as usize / 2 + 1];
    let mut len2 = (buf.len() * 2) as u32;
    // SAFETY: `buf` is writable for `len2` bytes.
    let rc = unsafe { RegQueryValueExW(key, name.as_ptr(), std::ptr::null_mut(), &mut typ, buf.as_mut_ptr() as *mut u8, &mut len2) };
    unsafe { RegCloseKey(key) };
    if rc != 0 {
        return None;
    }
    let n = (len2 as usize / 2).min(buf.len());
    let text = std::ffi::OsString::from_wide(&buf[..n]).to_string_lossy().trim_end_matches('\0').to_string();
    (!text.is_empty()).then_some(text)
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
    fn a_library_name_matches_its_registry_key() {
        assert_eq!(library_key_names("Elastic Thump Library"), ["Elastic Thump Library", "Elastic Thump"]);
        assert_eq!(library_key_names("Massive X Factory Library"), ["Massive X Factory Library", "Massive X Factory", "Massive X"]);
    }

    #[test]
    fn an_expansion_root_is_the_folder_it_was_saved_from() {
        assert_eq!(saved_root_name("/Volumes/ohne titel 1/+Expansions/Aquarius Earth Library"), Some("Aquarius Earth Library".into()));
    }

    /// Aquarius Earth (and the other expansions) store samples under their own
    /// ContentDir, while the group file calls the library "Maschine 2.0 Library"
    /// and records the Mac volume the expansion was authored on.
    #[test]
    fn an_expansion_group_sample_resolves_from_its_content_dir() {
        let wav = PathBuf::from(r"C:\Users\Public\Documents\Aquarius Earth Library\Samples\Drums\Kick\Kick BabyMayka 1.wav");
        if !wav.is_file() {
            return;
        }
        let sample = deckard_model::SampleRef {
            kind: 2,
            path: "Samples/Drums/Kick/Kick BabyMayka 1.wav".into(),
            library_root: "/Volumes/ohne titel 1/+Expansions/Aquarius Earth Library".into(),
            library: "Maschine 2.0 Library".into(),
        };
        assert_eq!(Resolver::new(None).resolve(&sample).as_deref(), Some(wav.as_path()));
    }

    #[test]
    fn hfs_paths() {
        assert_eq!(
            hfs_to_posix("Macintosh HD:Users:Shared:Maschine 2 Library:"),
            vec![PathBuf::from("/Users/Shared/Maschine 2 Library"), PathBuf::from("/Volumes/Macintosh HD/Users/Shared/Maschine 2 Library")]
        );
    }
}
