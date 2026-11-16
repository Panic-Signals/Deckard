//! Contain Win32 hardware SEH faults (`0xc0000005` access violations, etc.) inside
//! third-party C++ VST3 DLLs so a buggy plug-in logs a diagnostic instead of killing
//! a live performance.

use std::ffi::c_void;
use std::sync::OnceLock;

pub type FaultLogger = fn(&str);

static LOGGER: OnceLock<FaultLogger> = OnceLock::new();

/// Register a callback invoked whenever a hosted plug-in raises a hardware fault.
pub fn set_fault_logger(logger: FaultLogger) {
    let _ = LOGGER.set(logger);
}

#[cfg(all(windows, target_env = "msvc"))]
unsafe extern "C" {
    fn deckard_seh_call(f: unsafe extern "C" fn(*mut c_void), ctx: *mut c_void, fault_rip: *mut usize) -> u32;
    #[cfg(test)]
    fn deckard_seh_test_null_read();
}

/// Run `op` across a plug-in FFI boundary. `op` must not panic in Rust; it should only
/// invoke raw COM / C entry points. On Windows MSVC, hardware faults (`0xc0000005`, etc.)
/// are caught via SEH, logged with module + RVA, and returned as `Err`.
#[cfg(all(windows, target_env = "msvc"))]
pub(crate) fn guard<T, F: FnOnce() -> T>(plugin: &str, what: &str, op: F) -> Result<T, String> {
    struct Ctx<F, T> {
        op: Option<F>,
        out: Option<T>,
    }
    unsafe extern "C" fn run<F: FnOnce() -> T, T>(raw: *mut c_void) {
        let ctx = unsafe { &mut *(raw as *mut Ctx<F, T>) };
        if let Some(op) = ctx.op.take() {
            ctx.out = Some(op());
        }
    }
    let mut ctx = Ctx { op: Some(op), out: None };
    let mut rip: usize = 0;
    let code = unsafe { deckard_seh_call(run::<F, T>, (&mut ctx as *mut Ctx<F, T>).cast(), &mut rip) };
    if code == 0
        && let Some(out) = ctx.out
    {
        return Ok(out);
    }
    let at = describe_address(rip);
    let msg = format!("{plugin} faulted in {what} ({code:#010x} at {at})");
    if let Some(log) = LOGGER.get() {
        log(&msg);
    } else {
        eprintln!("[deckard-plugins] {msg}");
    }
    Err(msg)
}

#[cfg(not(all(windows, target_env = "msvc")))]
pub(crate) fn guard<T, F: FnOnce() -> T>(_plugin: &str, _what: &str, op: F) -> Result<T, String> {
    Ok(op())
}

#[cfg(all(windows, target_env = "msvc"))]
fn describe_address(rip: usize) -> String {
    if rip == 0 {
        return "0x0".into();
    }
    use windows_sys::Win32::Foundation::HMODULE;
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleExW,
    };
    let mut module: HMODULE = std::ptr::null_mut();
    let ok = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            rip as *const u16,
            &mut module,
        )
    };
    if ok != 0 && !module.is_null() {
        let base = module as usize;
        let rva = rip.saturating_sub(base);
        let mut buf = [0u16; 512];
        let len = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if len > 0 && len < buf.len() {
            let full = String::from_utf16_lossy(&buf[..len]);
            let file = std::path::Path::new(&full).file_name().and_then(|n| n.to_str()).unwrap_or(&full);
            return format!("{file}+{rva:#x} ({rip:#x})");
        }
        return format!("{base:#x}+{rva:#x}");
    }
    format!("{rip:#x}")
}

#[cfg(all(test, windows, target_env = "msvc"))]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static CAPTURED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn record_for_test(msg: &str) {
        CAPTURED.lock().unwrap().push(msg.to_string());
    }

    #[test]
    fn seh_guard_catches_native_access_violation_and_reports_module_offset() {
        set_fault_logger(record_for_test);
        assert_eq!(guard("Healthy", "process", || 42).unwrap(), 42);

        let err = guard("Reaktor 6", "setState", || unsafe { deckard_seh_test_null_read() }).unwrap_err();
        assert!(err.contains("Reaktor 6 faulted in setState"), "{err}");
        assert!(err.contains("0xc0000005"), "{err}");
        assert!(err.contains("+0x"), "expected module+RVA in {err}");
        if LOGGER.get() == Some(&(record_for_test as FaultLogger)) {
            assert!(CAPTURED.lock().unwrap().iter().any(|m| m.contains("0xc0000005")));
        }
    }
}
