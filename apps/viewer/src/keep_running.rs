//! Keep DECKARD running at full speed in the background. macOS slows an app that isn't
//! in front (App Nap, timer coalescing), and the controller is ticked from the UI loop's
//! timed repaints: without this the MK3's lights, meters and screens go stale until the
//! window is focused again (CONTROLLER-BUGS C46). Held for as long as the app runs, as
//! audio apps do.

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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    /// Begun and ended without fault, twice over (an activity that lets idle sleep happen
    /// leaves no power assertion to look for, so the stall itself needs the controller).
    #[test]
    fn the_activity_begins_and_ends() {
        for _ in 0..2 {
            drop(super::begin());
        }
    }
}
