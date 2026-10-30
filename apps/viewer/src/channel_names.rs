//! The names an audio interface gives its channels (BUGS #9): "Mic 1", "Line 3", … as
//! the interface's driver reports them. Core Audio has them on macOS (each channel's
//! element name); elsewhere, and for a channel the driver leaves unnamed, there's none.

/// Channel `c`'s label in the routing list: its number, and the interface's name for
/// it if it has one, else the port and the number ("3: Mic 3", "3: Input 3").
pub fn interface_label(c: usize, port: &str, names: &[Option<String>]) -> String {
    match names.get(c).cloned().flatten() {
        Some(name) => format!("{}: {name}", c + 1),
        None => format!("{}: {port} {}", c + 1, c + 1),
    }
}

/// The names device `device` gives its first `channels` input (or output) channels.
#[cfg(target_os = "macos")]
pub fn channel_names(device: &str, input: bool, channels: usize) -> Vec<Option<String>> {
    mac::device(device).map_or_else(Vec::new, |id| (0..channels).map(|c| mac::element_name(id, input, c as u32 + 1)).collect())
}

#[cfg(not(target_os = "macos"))]
pub fn channel_names(_device: &str, _input: bool, _channels: usize) -> Vec<Option<String>> {
    Vec::new()
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_void};
    use std::ptr::null;

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    const fn code(s: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*s)
    }

    const SYSTEM: u32 = 1;
    const DEVICES: u32 = code(b"dev#");
    const DEVICE_NAME: u32 = code(b"lnam");
    const ELEMENT_NAME: u32 = code(b"lchn");
    const GLOBAL: u32 = code(b"glob");
    const INPUT: u32 = code(b"inpt");
    const OUTPUT: u32 = code(b"outp");
    const UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyDataSize(id: u32, a: *const Address, qs: u32, q: *const c_void, size: *mut u32) -> i32;
        fn AudioObjectGetPropertyData(id: u32, a: *const Address, qs: u32, q: *const c_void, size: *mut u32, data: *mut c_void) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringGetCString(s: *const c_void, buf: *mut c_char, len: isize, encoding: u32) -> bool;
        fn CFRelease(s: *const c_void);
    }

    /// A CFString property of object `id`, if it has one and it isn't empty.
    fn string(id: u32, a: Address) -> Option<String> {
        let mut s: *const c_void = null();
        let mut size = size_of::<*const c_void>() as u32;
        // SAFETY: the property is a CFStringRef the call hands us (+1); it's released below.
        let status = unsafe { AudioObjectGetPropertyData(id, &a, 0, null(), &mut size, (&raw mut s).cast()) };
        if status != 0 || s.is_null() {
            return None;
        }
        let mut buf = [0 as c_char; 256];
        // SAFETY: `s` is the CFString above; `buf` is as long as said.
        let ok = unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) };
        unsafe { CFRelease(s) };
        let name = ok.then(|| unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().trim().to_string())?;
        (!name.is_empty()).then_some(name)
    }

    /// The device named `name` (as cpal names it).
    pub fn device(name: &str) -> Option<u32> {
        let a = Address { selector: DEVICES, scope: GLOBAL, element: 0 };
        let mut size = 0;
        // SAFETY: the system object's device list, read into a buffer of the size asked.
        if unsafe { AudioObjectGetPropertyDataSize(SYSTEM, &a, 0, null(), &mut size) } != 0 {
            return None;
        }
        let mut ids = vec![0u32; size as usize / size_of::<u32>()];
        if unsafe { AudioObjectGetPropertyData(SYSTEM, &a, 0, null(), &mut size, ids.as_mut_ptr().cast()) } != 0 {
            return None;
        }
        ids.truncate(size as usize / size_of::<u32>());
        ids.into_iter().find(|&id| {
            [OUTPUT, INPUT, GLOBAL]
                .iter()
                .any(|&scope| string(id, Address { selector: DEVICE_NAME, scope, element: 0 }).as_deref() == Some(name))
        })
    }

    /// Channel `element`'s (1-based) name.
    pub fn element_name(id: u32, input: bool, element: u32) -> Option<String> {
        string(id, Address { selector: ELEMENT_NAME, scope: if input { INPUT } else { OUTPUT }, element })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_channel_shows_its_name() {
        let names = [Some("Mic 1".to_string()), None];
        assert_eq!(interface_label(0, "Input", &names), "1: Mic 1");
        assert_eq!(interface_label(1, "Input", &names), "2: Input 2", "unnamed");
        assert_eq!(interface_label(5, "Output", &names), "6: Output 6", "past the names");
    }

    /// Asking a device that isn't there gives no names; asking the default input gives
    /// one per channel asked (named or not). Ignored: the system's audio takes up to a
    /// minute to start in a new process.
    #[test]
    #[ignore = "starts the system's audio"]
    fn names_of_the_devices() {
        assert!(channel_names("no such interface", true, 2).is_empty());
        if let Some(name) = crate::recorder::default_input() {
            let names = channel_names(&name, true, 2);
            assert!(names.is_empty() || names.len() == 2, "{name}: {names:?}");
        }
    }
}
