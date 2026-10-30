//! Loading a `.vst3` bundle and listing its classes.

use std::ffi::{CStr, c_char, c_void};
use std::path::{Path, PathBuf};
use vst3::ComPtr;
use vst3::Steinberg::*;

/// A loaded VST3 module (kept loaded for the life of the process).
pub struct Module {
    pub path: PathBuf,
    pub factory: ComPtr<IPluginFactory>,
    pub classes: Vec<ClassInfo>,
}

// The factory is only used on the thread that creates instances (the main thread).
unsafe impl Send for Module {}
unsafe impl Sync for Module {}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub cid: [u8; 16],
    pub category: String,
    pub name: String,
}

impl Module {
    pub fn load(path: &Path) -> Result<Module, String> {
        let factory = unsafe { platform::load(path)? };
        let factory = unsafe { ComPtr::from_raw(factory) }.ok_or("the plug-in has no factory")?;
        let mut classes = Vec::new();
        for i in 0..unsafe { factory.countClasses() } {
            let mut info: PClassInfo = unsafe { std::mem::zeroed() };
            if unsafe { factory.getClassInfo(i, &mut info) } != kResultOk {
                continue;
            }
            let text = |b: &[c_char]| unsafe { CStr::from_ptr(b.as_ptr()) }.to_string_lossy().into_owned();
            classes.push(ClassInfo {
                cid: unsafe { std::mem::transmute::<TUID, [u8; 16]>(info.cid) },
                category: text(&info.category),
                name: text(&info.name),
            });
        }
        Ok(Module { path: path.to_path_buf(), factory, classes })
    }

    pub fn has(&self, cid: &[u8; 16]) -> bool {
        self.classes.iter().any(|c| &c.cid == cid)
    }
}

static SEARCH_PATHS: std::sync::Mutex<Option<Vec<PathBuf>>> = std::sync::Mutex::new(None);

/// The standard VST3 folders: the user's, then the system's.
pub fn default_search_paths() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/Library/Audio/Plug-Ins/VST3")];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.insert(0, PathBuf::from(home).join("Library/Audio/Plug-Ins/VST3"));
    }
    dirs
}

/// Where VST3 plug-ins are looked for ([`set_search_paths`], else the standard folders).
pub fn search_paths() -> Vec<PathBuf> {
    SEARCH_PATHS.lock().ok().and_then(|p| p.clone()).unwrap_or_else(default_search_paths)
}

/// Look for VST3 plug-ins in `dirs` (the user's settings).
pub fn set_search_paths(dirs: Vec<PathBuf>) {
    if let Ok(mut p) = SEARCH_PATHS.lock() {
        *p = Some(dirs);
    }
}

/// Installed `.vst3` bundles (one level of subfolders is searched too).
pub fn bundles() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in search_paths() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "vst3") {
                out.push(p);
            } else if p.is_dir()
                && let Ok(sub) = std::fs::read_dir(&p)
            {
                out.extend(sub.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "vst3")));
            }
        }
    }
    out.sort();
    out
}

/// Class ids a bundle declares in its `moduleinfo.json`, if it has one (read without
/// loading the plug-in).
pub fn declared_classes(bundle: &Path) -> Option<Vec<[u8; 16]>> {
    let text = std::fs::read_to_string(bundle.join("Contents/Resources/moduleinfo.json")).ok()?;
    let mut out = Vec::new();
    for (i, _) in text.match_indices("\"CID\"") {
        let rest = &text[i + 5..];
        let start = rest.find('"')? + 1;
        let hex = rest.get(start..start + 32)?;
        let mut cid = [0u8; 16];
        for (k, b) in cid.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[2 * k..2 * k + 2], 16).ok()?;
        }
        out.push(cid);
    }
    Some(out)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    type CFTypeRef = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(alloc: CFTypeRef, buf: *const u8, len: isize, is_dir: u8) -> CFTypeRef;
        fn CFBundleCreate(alloc: CFTypeRef, url: CFTypeRef) -> CFTypeRef;
        fn CFBundleLoadExecutable(bundle: CFTypeRef) -> u8;
        fn CFBundleGetFunctionPointerForName(bundle: CFTypeRef, name: CFTypeRef) -> *const c_void;
        fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const c_char, encoding: u32) -> CFTypeRef;
        fn CFRelease(cf: CFTypeRef);
    }

    const UTF8: u32 = 0x0800_0100;

    unsafe fn symbol(bundle: CFTypeRef, name: &CStr) -> *const c_void {
        unsafe {
            let s = CFStringCreateWithCString(std::ptr::null(), name.as_ptr(), UTF8);
            let f = CFBundleGetFunctionPointerForName(bundle, s);
            CFRelease(s);
            f
        }
    }

    /// Load the bundle, run its `bundleEntry` and return its factory.
    pub unsafe fn load(path: &Path) -> Result<*mut IPluginFactory, String> {
        use std::os::unix::ffi::OsStrExt;
        let bytes = path.as_os_str().as_bytes();
        unsafe {
            let url = CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize, 1);
            if url.is_null() {
                return Err("bad plug-in path".into());
            }
            let bundle = CFBundleCreate(std::ptr::null(), url);
            CFRelease(url);
            if bundle.is_null() {
                return Err("not a bundle".into());
            }
            // The bundle stays loaded (and referenced) for the life of the process.
            if CFBundleLoadExecutable(bundle) == 0 {
                return Err("the plug-in's code could not be loaded".into());
            }
            let entry = symbol(bundle, c"bundleEntry");
            if !entry.is_null() {
                let entry: unsafe extern "C" fn(CFTypeRef) -> bool = std::mem::transmute(entry);
                if !entry(bundle) {
                    return Err("the plug-in's bundleEntry failed".into());
                }
            }
            let get = symbol(bundle, c"GetPluginFactory");
            if get.is_null() {
                return Err("the plug-in has no GetPluginFactory".into());
            }
            let get: unsafe extern "C" fn() -> *mut IPluginFactory = std::mem::transmute(get);
            Ok(get())
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;

    pub unsafe fn load(_path: &Path) -> Result<*mut IPluginFactory, String> {
        Err("plug-in hosting is only implemented on macOS".into())
    }
}
