//! Keep DECKARD running at full speed in the background. macOS slows an app that isn't
//! in front (App Nap, timer coalescing), and the controller is ticked from the UI loop's
//! timed repaints: without this the MK3's lights, meters and screens go stale until the
//! window is focused again (CONTROLLER-BUGS C46). Held for as long as the app runs, as
//! audio apps do. [`StayAwake`] also keeps the computer from sleeping while it plays.

/// The activity held; let go when dropped.
#[cfg(target_os = "macos")]
pub struct Activity(objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2::runtime::NSObjectProtocol>>);

#[cfg(not(target_os = "macos"))]
pub struct Activity;

/// Start keeping the app running: user-initiated and latency-critical (no App Nap, no
/// timer coalescing), the system still free to sleep when idle.
#[cfg(target_os = "macos")]
pub fn begin() -> Activity {
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
    let options = NSActivityOptions::UserInitiatedAllowingIdleSystemSleep | NSActivityOptions::LatencyCritical;
    let reason = NSString::from_str("DECKARD drives a controller and plays audio");
    Activity(NSProcessInfo::processInfo().beginActivityWithOptions_reason(options, &reason))
}

#[cfg(not(target_os = "macos"))]
pub fn begin() -> Activity {
    Activity
}

#[cfg(target_os = "macos")]
impl Drop for Activity {
    fn drop(&mut self) {
        // SAFETY: the token is the one `beginActivityWithOptions_reason` gave.
        unsafe { objc2_foundation::NSProcessInfo::processInfo().endActivity(&self.0) };
    }
}

impl std::fmt::Debug for Activity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Activity")
    }
}

/// Held while the song plays or something records: the computer doesn't sleep, nor its
/// screen dim, for want of mouse or keyboard (a set is played on the controllers, which
/// the system doesn't count). The system's own audio keeps it awake by itself, but an
/// ASIO driver doesn't. Let go when dropped.
#[cfg(target_os = "macos")]
pub struct Awake(Activity);

#[cfg(windows)]
pub struct Awake(windows_sys::Win32::Foundation::HANDLE);

#[cfg(not(any(target_os = "macos", windows)))]
pub struct Awake;

/// The reason the system shows for it (`powercfg /requests`, `pmset -g assertions`).
const AWAKE_REASON: &str = "DECKARD is playing";

/// Keep the computer and its screen awake until the returned guard is dropped. `None`
/// if the system refused.
#[cfg(target_os = "macos")]
pub fn stay_awake() -> Option<Awake> {
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
    let options = NSActivityOptions::IdleSystemSleepDisabled | NSActivityOptions::IdleDisplaySleepDisabled;
    let reason = NSString::from_str(AWAKE_REASON);
    Some(Awake(Activity(NSProcessInfo::processInfo().beginActivityWithOptions_reason(options, &reason))))
}

#[cfg(windows)]
pub fn stay_awake() -> Option<Awake> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Power::{PowerCreateRequest, PowerRequestDisplayRequired, PowerRequestSystemRequired, PowerSetRequest};
    use windows_sys::Win32::System::Threading::{POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0};
    let mut reason: Vec<u16> = AWAKE_REASON.encode_utf16().chain([0]).collect();
    let context = REASON_CONTEXT {
        Version: 0, // POWER_REQUEST_CONTEXT_VERSION
        Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
        Reason: REASON_CONTEXT_0 { SimpleReasonString: reason.as_mut_ptr() },
    };
    // SAFETY: `context` and the string it points at outlive the call, which copies them.
    let request = unsafe { PowerCreateRequest(&context) };
    if request.is_null() || request == INVALID_HANDLE_VALUE {
        return None;
    }
    // SAFETY: `request` is the live handle just made.
    let set = unsafe { PowerSetRequest(request, PowerRequestSystemRequired) != 0 };
    if !set {
        // SAFETY: as above; nothing else holds it.
        unsafe { CloseHandle(request) };
        return None;
    }
    // The screen too, if it can (it's not worth giving up the rest for).
    // SAFETY: as above.
    unsafe { PowerSetRequest(request, PowerRequestDisplayRequired) };
    Some(Awake(request))
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn stay_awake() -> Option<Awake> {
    Some(Awake)
}

#[cfg(windows)]
impl Drop for Awake {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Power::{PowerClearRequest, PowerRequestDisplayRequired, PowerRequestSystemRequired};
        // SAFETY: the handle `stay_awake` made, closed only here.
        unsafe {
            PowerClearRequest(self.0, PowerRequestSystemRequired);
            PowerClearRequest(self.0, PowerRequestDisplayRequired);
            CloseHandle(self.0);
        }
    }
}

impl std::fmt::Debug for Awake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Awake")
    }
}

/// What keeps the computer awake: held exactly while `wanted`, taken and let go as that
/// changes (a refusal isn't asked again until it's let go and wanted anew).
#[derive(Debug, Default)]
pub struct StayAwake {
    held: Option<Option<Awake>>,
}

impl StayAwake {
    pub fn want(&mut self, wanted: bool) {
        if wanted != self.held.is_some() {
            self.held = wanted.then(stay_awake);
        }
    }

    /// Held, and the system agreed.
    pub fn is_awake(&self) -> bool {
        matches!(self.held, Some(Some(_)))
    }
}

#[cfg(test)]
mod tests {
    /// Begun and ended without fault, twice over (an activity that lets idle sleep happen
    /// leaves no power assertion to look for, so the stall itself needs the controller).
    #[cfg(target_os = "macos")]
    #[test]
    fn the_activity_begins_and_ends() {
        for _ in 0..2 {
            drop(super::begin());
        }
    }

    /// Taken while wanted and let go after, again and again (each one is the system's).
    #[test]
    fn staying_awake_follows_what_is_wanted() {
        let mut s = super::StayAwake::default();
        assert!(!s.is_awake());
        for _ in 0..3 {
            s.want(true);
            s.want(true);
            assert!(s.is_awake(), "the system refused to stay awake");
            s.want(false);
            assert!(!s.is_awake());
        }
    }

    /// Windows lists it among the power requests while held, and not after.
    #[cfg(windows)]
    #[test]
    fn windows_lists_the_request_while_held() {
        let listed = || {
            std::process::Command::new("powercfg")
                .arg("/requests")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains(super::AWAKE_REASON))
        };
        let guard = super::stay_awake().expect("power request");
        // powercfg needs an elevated prompt; without one there's nothing to compare.
        let Ok(held) = listed() else { return };
        if !held && !std::process::Command::new("powercfg").arg("/requests").output().is_ok_and(|o| o.status.success()) {
            return;
        }
        assert!(held, "powercfg /requests doesn't list the request");
        drop(guard);
        assert!(!listed().unwrap_or(false), "still listed after it was let go");
    }
}
