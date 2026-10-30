//! Finding installed plug-ins: VST3 classes (from a bundle's `moduleinfo.json`, or by
//! loading it, which callers should do in a separate process), VST2 bundles, and Audio
//! Units (from the system's component registry).

use crate::module::{self, Module};
use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};
use vst3::ComPtr;
use vst3::Steinberg::*;

/// The plug-in formats Maschine hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Vst3,
    Vst2,
    AudioUnit,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Vst3 => "VST3",
            Kind::Vst2 => "VST",
            Kind::AudioUnit => "AU",
        }
    }
}

/// One installed plug-in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    pub kind: Kind,
    pub vendor: String,
    pub name: String,
    /// An instrument (else an effect).
    pub instrument: bool,
    /// The bundle (VST3/VST2) or "type subtype manufacturer" four-char codes (AU).
    pub location: String,
    /// The VST3 class id, upper-case hex (VST3 only).
    pub cid: String,
}

impl Entry {
    /// Maschine's `plugin_id` for this plug-in (VST3: `"<vendor>:VST3:INSTR|FX:<cid>"`).
    pub fn maschine_id(&self) -> Option<String> {
        let role = if self.instrument { "INSTR" } else { "FX" };
        match self.kind {
            Kind::Vst3 => Some(format!("{}:VST3:{role}:{}", self.vendor, self.cid)),
            _ => None,
        }
    }

    /// Whether this build can host it (VST3 for now).
    pub fn hostable(&self) -> bool {
        self.kind == Kind::Vst3
    }

    /// One line of the scan cache (tab-separated).
    pub fn to_line(&self) -> String {
        format!("{}\t{}\t{}\t{}\t{}\t{}", self.kind.label(), self.vendor, self.name, self.instrument as u8, self.location, self.cid)
    }

    pub fn from_line(line: &str) -> Option<Entry> {
        let f: Vec<&str> = line.split('\t').collect();
        let kind = match *f.first()? {
            "VST3" => Kind::Vst3,
            "VST" => Kind::Vst2,
            "AU" => Kind::AudioUnit,
            _ => return None,
        };
        Some(Entry {
            kind,
            vendor: f.get(1)?.to_string(),
            name: f.get(2)?.to_string(),
            instrument: *f.get(3)? == "1",
            location: f.get(4)?.to_string(),
            cid: f.get(5).unwrap_or(&"").to_string(),
        })
    }
}

fn text(b: &[c_char]) -> String {
    unsafe { CStr::from_ptr(b.as_ptr()) }.to_string_lossy().into_owned()
}

/// VST3 bundles in `dirs` (and one level of subfolders).
pub fn vst3_bundles(dirs: &[PathBuf]) -> Vec<PathBuf> {
    bundles_with(dirs, "vst3")
}

/// VST2 plug-ins in `dirs`: `.vst` bundles on macOS, `.so` files on Linux, `.dll`s on
/// Windows.
pub fn vst2_bundles(dirs: &[PathBuf]) -> Vec<PathBuf> {
    bundles_with(dirs, VST2_EXTENSION)
}

#[cfg(target_os = "linux")]
const VST2_EXTENSION: &str = "so";
#[cfg(windows)]
const VST2_EXTENSION: &str = "dll";
#[cfg(not(any(target_os = "linux", windows)))]
const VST2_EXTENSION: &str = "vst";

fn bundles_with(dirs: &[PathBuf], ext: &str) -> Vec<PathBuf> {
    let is = |p: &Path| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext));
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if is(&p) {
                out.push(p);
            } else if p.is_dir()
                && let Ok(sub) = std::fs::read_dir(&p)
            {
                out.extend(sub.flatten().map(|e| e.path()).filter(|p| is(p)));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A VST3 bundle's audio-processor classes from its `moduleinfo.json`, without loading
/// it. `None` if it has none.
pub fn vst3_from_moduleinfo(bundle: &Path) -> Option<Vec<Entry>> {
    let text = std::fs::read_to_string(bundle.join("Contents/Resources/moduleinfo.json")).ok()?;
    // moduleinfo.json allows trailing commas (JSON5-ish); drop them.
    let cleaned = text.replace(",\n}", "\n}").replace(",\n]", "\n]");
    let v: serde_json::Value = serde_json::from_str(&cleaned).ok()?;
    let factory_vendor = v.pointer("/Factory Info/Vendor").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let classes = v.get("Classes")?.as_array()?;
    Some(
        classes
            .iter()
            .filter(|c| c.get("Category").and_then(|x| x.as_str()) == Some("Audio Module Class"))
            .map(|c| {
                let subs: Vec<&str> = c
                    .get("Sub Categories")
                    .and_then(|x| x.as_array())
                    .map_or(Vec::new(), |a| a.iter().filter_map(|s| s.as_str()).collect());
                Entry {
                    kind: Kind::Vst3,
                    vendor: c
                        .get("Vendor")
                        .and_then(|x| x.as_str())
                        .filter(|s| !s.is_empty())
                        .map_or(factory_vendor.clone(), str::to_string),
                    name: c.get("Name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    instrument: subs.iter().any(|s| s.contains("Instrument")),
                    location: bundle.display().to_string(),
                    cid: c.get("CID").and_then(|x| x.as_str()).unwrap_or("").to_uppercase(),
                }
            })
            .collect(),
    )
}

/// A VST3 bundle's audio-processor classes, by loading it (do this in a helper process:
/// a plug-in can crash or hang while loading).
pub fn vst3_by_loading(bundle: &Path) -> Result<Vec<Entry>, String> {
    let m = Module::load(bundle)?;
    let mut out = Vec::new();
    let f2: Option<ComPtr<IPluginFactory2>> = m.factory.cast();
    let mut factory_info: PFactoryInfo = unsafe { std::mem::zeroed() };
    unsafe { m.factory.getFactoryInfo(&mut factory_info) };
    let factory_vendor = text(&factory_info.vendor);
    for i in 0..unsafe { m.factory.countClasses() } {
        let (cid, category, name, subs, vendor) = match &f2 {
            Some(f2) => {
                let mut info: PClassInfo2 = unsafe { std::mem::zeroed() };
                if unsafe { f2.getClassInfo2(i, &mut info) } != kResultOk {
                    continue;
                }
                (info.cid, text(&info.category), text(&info.name), text(&info.subCategories), text(&info.vendor))
            }
            None => {
                let mut info: PClassInfo = unsafe { std::mem::zeroed() };
                if unsafe { m.factory.getClassInfo(i, &mut info) } != kResultOk {
                    continue;
                }
                (info.cid, text(&info.category), text(&info.name), String::new(), String::new())
            }
        };
        if category != "Audio Module Class" {
            continue;
        }
        let cid: [u8; 16] = unsafe { std::mem::transmute::<TUID, [u8; 16]>(cid) };
        out.push(Entry {
            kind: Kind::Vst3,
            vendor: if vendor.is_empty() { factory_vendor.clone() } else { vendor },
            name,
            // Old factories have no sub-categories: count those as instruments.
            instrument: subs.contains("Instrument") || subs.is_empty(),
            location: bundle.display().to_string(),
            cid: cid.iter().map(|b| format!("{b:02X}")).collect(),
        });
    }
    Ok(out)
}

/// VST2 bundles as entries (name from the bundle; not hostable yet).
pub fn vst2_entries(dirs: &[PathBuf]) -> Vec<Entry> {
    vst2_bundles(dirs)
        .into_iter()
        .map(|b| Entry {
            kind: Kind::Vst2,
            vendor: String::new(),
            name: b.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned()),
            instrument: false,
            location: b.display().to_string(),
            cid: String::new(),
        })
        .collect()
}

/// The standard VST2 folders.
pub fn default_vst2_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut dirs = vec![PathBuf::from("/Library/Audio/Plug-Ins/VST")];
        if let Some(home) = std::env::var_os("HOME") {
            dirs.insert(0, PathBuf::from(home).join("Library/Audio/Plug-Ins/VST"));
        }
        dirs
    }
    #[cfg(target_os = "linux")]
    {
        // `$VST_PATH` (colon-separated) is the convention Linux hosts share.
        let mut dirs: Vec<PathBuf> = std::env::var_os("VST_PATH").map_or(Vec::new(), |p| std::env::split_paths(&p).collect());
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".vst"));
        }
        dirs.extend(["/usr/lib/vst", "/usr/local/lib/vst"].map(PathBuf::from));
        dirs.dedup();
        dirs
    }
    #[cfg(windows)]
    {
        let programs = std::env::var_os("ProgramFiles").map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from);
        let common = std::env::var_os("CommonProgramFiles").map_or_else(|| programs.join("Common Files"), PathBuf::from);
        vec![programs.join("VstPlugins"), programs.join(r"Steinberg\VstPlugins"), common.join("VST2")]
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    Vec::new()
}

pub use module::default_search_paths as default_vst3_paths;

/// Installed Audio Units (instruments, effects and MIDI-controlled effects).
#[cfg(target_os = "macos")]
pub fn audio_units() -> Vec<Entry> {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Desc {
        kind: u32,
        sub: u32,
        manufacturer: u32,
        flags: u32,
        mask: u32,
    }
    #[link(name = "AudioToolbox", kind = "framework")]
    unsafe extern "C" {
        fn AudioComponentFindNext(after: *mut c_void, desc: *const Desc) -> *mut c_void;
        fn AudioComponentGetDescription(c: *mut c_void, desc: *mut Desc) -> i32;
        fn AudioComponentCopyName(c: *mut c_void, name: *mut *const c_void) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringGetCString(s: *const c_void, buf: *mut c_char, len: isize, enc: u32) -> u8;
        fn CFRelease(cf: *const c_void);
    }
    let fourcc = |v: u32| String::from_utf8_lossy(&v.to_be_bytes()).into_owned();
    let mut out = Vec::new();
    for kind in [*b"aumu", *b"aufx", *b"aumf"] {
        let want = Desc { kind: u32::from_be_bytes(kind), ..Desc::default() };
        let mut c = std::ptr::null_mut();
        loop {
            c = unsafe { AudioComponentFindNext(c, &want) };
            if c.is_null() {
                break;
            }
            let mut d = Desc::default();
            unsafe { AudioComponentGetDescription(c, &mut d) };
            let mut name_ref = std::ptr::null();
            let mut full = String::new();
            if unsafe { AudioComponentCopyName(c, &mut name_ref) } == 0 && !name_ref.is_null() {
                let mut buf = [0 as c_char; 512];
                if unsafe { CFStringGetCString(name_ref, buf.as_mut_ptr(), buf.len() as isize, 0x0800_0100) } != 0 {
                    full = text(&buf);
                }
                unsafe { CFRelease(name_ref) };
            }
            // "Vendor: Name"
            let (vendor, name) = full.split_once(": ").map_or((String::new(), full.clone()), |(v, n)| (v.to_string(), n.to_string()));
            out.push(Entry {
                kind: Kind::AudioUnit,
                vendor,
                name,
                instrument: &kind == b"aumu",
                location: format!("{} {} {}", fourcc(d.kind), fourcc(d.sub), fourcc(d.manufacturer)),
                cid: String::new(),
            });
        }
    }
    out.sort();
    out
}

#[cfg(not(target_os = "macos"))]
pub fn audio_units() -> Vec<Entry> {
    Vec::new()
}
