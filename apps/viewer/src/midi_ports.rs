//! Background MIDI port enumeration driven by OS PnP events and explicit refreshes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use eframe::egui;

const PNP_COALESCE: Duration = Duration::from_millis(150);

/// Probe all current WinMM / OS MIDI inputs and outputs.
pub fn probe() -> (Vec<String>, Vec<String>) {
    let ins =
        midir::MidiInput::new("DECKARD ports").map(|m| m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect()).unwrap_or_default();
    let outs =
        midir::MidiOutput::new("DECKARD ports").map(|m| m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect()).unwrap_or_default();
    (ins, outs)
}

#[derive(Default)]
struct Wake {
    stop: bool,
    scan: bool,
    pnp_deadline: Option<Instant>,
    latest: Option<(Vec<String>, Vec<String>)>,
    ctx: Option<egui::Context>,
}

struct Shared {
    wake: Mutex<Wake>,
    cv: Condvar,
    dirty: AtomicBool,
    has_ctx: AtomicBool,
}

impl Shared {
    fn on_pnp(&self) {
        if let Ok(mut st) = self.wake.lock() {
            if !st.stop {
                st.pnp_deadline.get_or_insert_with(|| Instant::now() + PNP_COALESCE);
                self.cv.notify_all();
            }
        }
    }
}

/// Asynchronous watcher that scans MIDI ports off the UI thread.
pub struct Watcher {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Watcher {
    /// Start the background port scanner and do an initial probe immediately.
    pub fn start() -> Self {
        let shared = Arc::new(Shared {
            wake: Mutex::new(Wake { scan: true, ..Default::default() }),
            cv: Condvar::new(),
            dirty: AtomicBool::new(false),
            has_ctx: AtomicBool::new(false),
        });
        let bg = Arc::clone(&shared);
        let thread = thread::Builder::new().name("DECKARD MIDI ports".into()).spawn(move || run(bg)).ok();
        Self { shared, thread }
    }

    /// Give the watcher the UI context so finished scans wake the window.
    pub fn set_context(&self, ctx: &egui::Context) {
        if self.shared.has_ctx.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut st) = self.shared.wake.lock() else { return };
        if st.ctx.is_none() {
            st.ctx = Some(ctx.clone());
            self.shared.has_ctx.store(true, Ordering::Release);
            if self.shared.dirty.load(Ordering::Acquire) {
                ctx.request_repaint();
            }
        }
    }

    /// Ask the worker thread for a fresh scan (e.g. for virtual ports that emit no PnP event).
    pub fn request_refresh(&self) {
        if let Ok(mut st) = self.shared.wake.lock() {
            st.scan = true;
            self.shared.cv.notify_all();
        }
    }

    /// Take the newest scanned `(inputs, outputs)` snapshot if one arrived since the last call.
    pub fn try_take(&self) -> Option<(Vec<String>, Vec<String>)> {
        if !self.shared.dirty.load(Ordering::Acquire) {
            return None;
        }
        let Ok(mut st) = self.shared.wake.lock() else { return None };
        self.shared.dirty.store(false, Ordering::Release);
        st.latest.take()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if let Ok(mut st) = self.shared.wake.lock() {
            st.stop = true;
            self.shared.cv.notify_all();
        }
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

fn run(shared: Arc<Shared>) {
    // Unregistered explicitly before `shared` drops; `CM_Unregister_Notification`
    // waits for any running callback, keeping the context pointer valid.
    let pnp = pnp::register(&shared);
    let mut last: Option<(Vec<String>, Vec<String>)> = None;

    loop {
        let should_scan = {
            let Ok(mut st) = shared.wake.lock() else { break };
            loop {
                if st.stop {
                    drop(st);
                    drop(pnp);
                    return;
                }
                if st.scan {
                    st.scan = false;
                    st.pnp_deadline = None;
                    break true;
                }
                if let Some(deadline) = st.pnp_deadline {
                    let now = Instant::now();
                    if now >= deadline {
                        st.pnp_deadline = None;
                        break true;
                    }
                    let (guard, _) = match shared.cv.wait_timeout(st, deadline - now) {
                        Ok(r) => r,
                        Err(_) => {
                            drop(pnp);
                            return;
                        }
                    };
                    st = guard;
                } else {
                    st = match shared.cv.wait(st) {
                        Ok(g) => g,
                        Err(_) => {
                            drop(pnp);
                            return;
                        }
                    };
                }
            }
        };

        if !should_scan {
            continue;
        }
        let ports = probe();
        let changed = last.as_ref() != Some(&ports);
        last = Some(ports.clone());
        let ctx = {
            let Ok(mut st) = shared.wake.lock() else { break };
            if st.stop {
                break;
            }
            if changed {
                st.latest = Some(ports);
                shared.dirty.store(true, Ordering::Release);
                st.ctx.clone()
            } else {
                None
            }
        };
        if let Some(ctx) = ctx {
            ctx.request_repaint();
        }
    }
    drop(pnp);
}

#[cfg(windows)]
mod pnp {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::sync::Arc;

    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_NOTIFY_ACTION, CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL, CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL, CM_NOTIFY_EVENT_DATA,
        CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_0, CM_NOTIFY_FILTER_0_2, CM_NOTIFY_FILTER_FLAG_ALL_INTERFACE_CLASSES,
        CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CM_Register_Notification, CM_Unregister_Notification, CR_SUCCESS, HCMNOTIFICATION,
    };

    use super::Shared;

    pub struct Handle(HCMNOTIFICATION);

    unsafe extern "system" fn callback(
        _hnotify: HCMNOTIFICATION,
        context: *const c_void,
        action: CM_NOTIFY_ACTION,
        _event_data: *const CM_NOTIFY_EVENT_DATA,
        _event_data_size: u32,
    ) -> u32 {
        if (action == CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL || action == CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL) && !context.is_null() {
            let shared = unsafe { &*(context as *const Shared) };
            shared.on_pnp();
        }
        0
    }

    pub fn register(shared: &Arc<Shared>) -> Option<Handle> {
        let mut filter: CM_NOTIFY_FILTER = unsafe { zeroed() };
        filter.cbSize = size_of::<CM_NOTIFY_FILTER>() as u32;
        filter.Flags = CM_NOTIFY_FILTER_FLAG_ALL_INTERFACE_CLASSES;
        filter.FilterType = CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE;
        filter.u = CM_NOTIFY_FILTER_0 { DeviceInterface: CM_NOTIFY_FILTER_0_2 { ClassGuid: unsafe { zeroed() } } };
        let mut handle: HCMNOTIFICATION = unsafe { zeroed() };
        let rc = unsafe { CM_Register_Notification(&filter, Arc::as_ptr(shared).cast(), Some(callback), &mut handle) };
        (rc == CR_SUCCESS).then_some(Handle(handle))
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CM_Unregister_Notification(self.0);
            }
        }
    }
}

#[cfg(not(windows))]
mod pnp {
    use std::sync::Arc;

    pub struct Handle;

    pub fn register(_: &Arc<super::Shared>) -> Option<Handle> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_does_not_panic() {
        let _ = probe();
    }

    #[test]
    fn watcher_delivers_initial_ports_and_stops_cleanly() {
        let watcher = Watcher::start();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut got = None;
        while Instant::now() < deadline {
            if let Some(p) = watcher.try_take() {
                got = Some(p);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(got.is_some(), "expected initial MIDI port scan within 2s");
        watcher.request_refresh();
        drop(watcher);
    }
}
