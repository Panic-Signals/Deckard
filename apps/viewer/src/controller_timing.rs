//! Keep the controllers' 2 ms HID polls from rounding to the Windows scheduler tick.
//! Acquired only while a controller is open; every successful request is released.
pub(crate) struct Polling {
    #[cfg(windows)]
    active: bool,
}
impl Polling {
    pub fn start() -> Self {
        #[cfg(windows)]
        {
            // SAFETY: the multimedia timer API takes only a millisecond period.
            Self { active: unsafe { timeBeginPeriod(1) } == 0 }
        }
        #[cfg(not(windows))]
        Self {}
    }
}
impl Drop for Polling {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.active {
            // SAFETY: balances this guard's successful timeBeginPeriod(1).
            unsafe { timeEndPeriod(1) };
        }
    }
}
#[cfg(windows)]
#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
}
