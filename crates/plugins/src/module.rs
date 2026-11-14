//! Loading a `.vst3` bundle and listing its classes.

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::ffi::c_void;
use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};
use vst3::Steinberg::*;
use vst3::{ComPtr, ComWrapper};

/// A loaded VST3 module (kept loaded for the life of the process).
pub struct Module {
    pub path: PathBuf,
    pub factory: ComPtr<IPluginFactory>,
    pub classes: Vec<ClassInfo>,
    _host: ComWrapper<crate::com::HostApp>,
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

/// Held while a module loads. A plug-in's entry points (`InitDll`, `bundleEntry`) and
/// first factory calls assume one thread: the app loads on its main thread only, but
/// each test has a host of its own on its own thread, and two of them loading the same
/// plug-in at once crashed inside it (an access violation in its `InitDll`).
static LOADING: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Module {
    pub fn load(path: &Path) -> Result<Module, String> {
        let _one_at_a_time = LOADING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let factory = unsafe { platform::load(path)? };
        let factory = unsafe { ComPtr::from_raw(factory) }.ok_or("the plug-in has no factory")?;
        let host = ComWrapper::new(crate::com::HostApp);
        if let Some(f3) = factory.cast::<IPluginFactory3>() {
            let ctx = host.as_com_ref::<FUnknown>().unwrap().as_ptr();
            unsafe { f3.setHostContext(ctx) };
        }
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
        Ok(Module { path: path.to_path_buf(), factory, classes, _host: host })
    }

    pub fn has(&self, cid: &[u8; 16]) -> bool {
        self.class_id(cid).is_some()
    }

    /// Class `cid` as this module's factory knows it, in whichever byte order it's in.
    pub fn class_id(&self, cid: &[u8; 16]) -> Option<[u8; 16]> {
        self.classes.iter().map(|c| c.cid).find(|c| same_class(c, cid))
    }
}

/// Whether two class ids are the same class. A plug-in built for Windows keeps the first
/// eight bytes of its ids in COM's order while a `moduleinfo.json`, and a project saved on
/// another system, have them in the standard one: the same plug-in reads differently.
pub fn same_class(a: &[u8; 16], b: &[u8; 16]) -> bool {
    a == b || com_order(a) == *b
}

/// A class id with its first eight bytes in COM's order, or back (it's its own inverse).
pub(crate) fn com_order(cid: &[u8; 16]) -> [u8; 16] {
    let mut out = *cid;
    out[..4].reverse();
    out.swap(4, 5);
    out.swap(6, 7);
    out
}

static SEARCH_PATHS: std::sync::Mutex<Option<Vec<PathBuf>>> = std::sync::Mutex::new(None);

/// The standard VST3 folders: the user's, then the system's (Steinberg's list for each
/// system).
pub fn default_search_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut dirs = vec![PathBuf::from("/Library/Audio/Plug-Ins/VST3")];
        if let Some(home) = std::env::var_os("HOME") {
            dirs.insert(0, PathBuf::from(home).join("Library/Audio/Plug-Ins/VST3"));
        }
        dirs
    }
    #[cfg(target_os = "linux")]
    {
        let mut dirs = Vec::new();
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".vst3"));
        }
        dirs.extend(["/usr/lib64/vst3", "/usr/lib/vst3", "/usr/local/lib64/vst3", "/usr/local/lib/vst3"].map(PathBuf::from));
        dirs
    }
    #[cfg(windows)]
    {
        let mut dirs = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join(r"Programs\Common\VST3"));
        }
        let common = std::env::var_os("CommonProgramFiles").map_or_else(|| PathBuf::from(r"C:\Program Files\Common Files"), PathBuf::from);
        dirs.push(common.join("VST3"));
        dirs
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    Vec::new()
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

/// Installed `.vst3` bundles in [`search_paths`] and their subfolders.
pub fn bundles() -> Vec<PathBuf> {
    crate::scan::vst3_bundles(&search_paths())
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

/// Linux: a bundle is a folder holding `Contents/<machine>-linux/<name>.so`, where
/// `<machine>` is `uname -m` (as Steinberg's own loader does). The library exports
/// `ModuleEntry(handle)`, `ModuleExit()` and `GetPluginFactory()`.
#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use libloading::os::unix::{Library, RTLD_LAZY, RTLD_LOCAL};

    fn machine() -> String {
        let mut u: libc::utsname = unsafe { std::mem::zeroed() };
        if unsafe { libc::uname(&mut u) } != 0 {
            return std::env::consts::ARCH.to_string();
        }
        unsafe { CStr::from_ptr(u.machine.as_ptr()) }.to_string_lossy().into_owned()
    }

    /// The shared library inside the bundle.
    pub fn library_path(bundle: &Path) -> Result<PathBuf, String> {
        let name = bundle.file_stem().ok_or("bad plug-in path")?;
        let so = bundle.join("Contents").join(format!("{}-linux", machine())).join(name).with_extension("so");
        if !so.is_file() {
            return Err(format!("not a plug-in for this machine (no {})", so.display()));
        }
        Ok(so)
    }

    /// Load the bundle's library, run its `ModuleEntry` and return its factory. The
    /// library stays loaded for the life of the process.
    pub unsafe fn load(path: &Path) -> Result<*mut IPluginFactory, String> {
        let so = if path.is_dir() { library_path(path)? } else { path.to_path_buf() };
        // Lazy binding, as the SDK's loader: some plug-ins reference symbols they never
        // call on this system.
        let lib = unsafe { Library::open(Some(&so), RTLD_LAZY | RTLD_LOCAL) }
            .map_err(|e| format!("the plug-in's code could not be loaded: {e}"))?;
        unsafe {
            let get = *lib
                .get::<unsafe extern "C" fn() -> *mut IPluginFactory>(b"GetPluginFactory\0")
                .map_err(|_| "the plug-in has no GetPluginFactory".to_string())?;
            let entry = lib.get::<unsafe extern "C" fn(*mut c_void) -> bool>(b"ModuleEntry\0").ok().map(|f| *f);
            // Never unloaded (`into_raw` is also the handle `ModuleEntry` is owed).
            let handle = lib.into_raw();
            if let Some(entry) = entry
                && !entry(handle)
            {
                return Err("the plug-in's ModuleEntry failed".into());
            }
            Ok(get())
        }
    }
}

/// Windows: a bundle is a folder holding `Contents/<arch>-win/<name>.vst3`, a DLL; an
/// older plug-in is that DLL on its own. It exports `GetPluginFactory()` and may export
/// `InitDll()` and `ExitDll()`.
#[cfg(windows)]
mod platform {
    use super::*;
    use libloading::os::windows::Library;

    /// The DLL inside the bundle.
    pub fn library_path(bundle: &Path) -> Result<PathBuf, String> {
        let arch = match std::env::consts::ARCH {
            "x86_64" => "x86_64-win",
            "x86" => "x86-win",
            "aarch64" => "arm64-win",
            other => other,
        };
        let name = bundle.file_name().ok_or("bad plug-in path")?;
        let dll = bundle.join("Contents").join(arch).join(name);
        if !dll.is_file() {
            return Err(format!("not a plug-in for this machine (no {})", dll.display()));
        }
        Ok(dll)
    }

    /// Load the bundle's DLL, run its `InitDll` and return its factory. The DLL stays
    /// loaded for the life of the process.
    pub unsafe fn load(path: &Path) -> Result<*mut IPluginFactory, String> {
        let dll = if path.is_dir() { library_path(path)? } else { path.to_path_buf() };
        let lib = unsafe { Library::new(&dll) }.map_err(|e| format!("the plug-in's code could not be loaded: {e}"))?;
        unsafe {
            let get = *lib
                .get::<unsafe extern "system" fn() -> *mut IPluginFactory>(b"GetPluginFactory\0")
                .map_err(|_| "the plug-in has no GetPluginFactory".to_string())?;
            if let Ok(init) = lib.get::<unsafe extern "system" fn() -> bool>(b"InitDll\0")
                && !init()
            {
                return Err("the plug-in's InitDll failed".into());
            }
            let _ = lib.into_raw();
            Ok(get())
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod platform {
    use super::*;

    pub unsafe fn load(_path: &Path) -> Result<*mut IPluginFactory, String> {
        Err("plug-in hosting is not implemented on this system".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STANDARD: [u8; 16] = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 1, 2, 3, 4, 5, 6, 7, 8];
    const COM: [u8; 16] = [0x78, 0x56, 0x34, 0x12, 0xBC, 0x9A, 0xF0, 0xDE, 1, 2, 3, 4, 5, 6, 7, 8];

    /// A class is the same class in either byte order, as on Windows.
    #[test]
    fn a_class_id_matches_in_either_byte_order() {
        assert_eq!(com_order(&STANDARD), COM);
        assert_eq!(com_order(&COM), STANDARD);
        assert!(same_class(&STANDARD, &STANDARD));
        assert!(same_class(&STANDARD, &COM));
        assert!(same_class(&COM, &STANDARD));
        let mut other = COM;
        other[15] ^= 1;
        assert!(!same_class(&STANDARD, &other), "another class");
    }

    /// A module finds its class in either order, and answers with its own.
    #[test]
    fn a_module_answers_with_its_own_class_id() {
        let Some(bundle) = bundles().into_iter().find(|b| b.file_stem().is_some_and(|s| s == "Dexed")) else {
            eprintln!("no Dexed here; skipping");
            return;
        };
        let m = Module::load(&bundle).unwrap();
        let own = m.classes.iter().find(|c| c.category == "Audio Module Class").unwrap().cid;
        assert_eq!(m.class_id(&own), Some(own));
        assert_eq!(m.class_id(&com_order(&own)), Some(own));
        let mut other = own;
        other[15] ^= 1;
        assert_eq!(m.class_id(&other), None);
    }

    #[test]
    fn bundles_matches_the_scanners_vst3_bundle_list() {
        assert_eq!(bundles(), crate::scan::vst3_bundles(&search_paths()));
    }
}
