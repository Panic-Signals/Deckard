//! The event loop a VST3 plug-in expects from a Linux host (`Linux::IRunLoop`). Linux
//! has no system event loop, so a plug-in registers the file descriptors it reads (its
//! own X connection) and the timers it wants with the host, which serves them.
//! [`RunLoop::poll`] does that once; the editor calls it every frame, from outside any
//! call into the plug-in.

use std::cell::RefCell;
use std::time::{Duration, Instant};
use vst3::Steinberg::Linux::*;
use vst3::{ComPtr, ComRef};

struct Timer {
    handler: ComPtr<ITimerHandler>,
    every: Duration,
    due: Instant,
}

/// The handlers and timers a plug-in has registered.
#[derive(Default)]
pub struct RunLoop {
    handlers: RefCell<Vec<(ComPtr<IEventHandler>, i32)>>,
    timers: RefCell<Vec<Timer>>,
}

impl RunLoop {
    /// Call `handler` whenever `fd` is readable. False for a null handler.
    ///
    /// # Safety
    /// `handler` is null or a live `IEventHandler`.
    pub unsafe fn register_handler(&self, handler: *mut IEventHandler, fd: i32) -> bool {
        let Some(h) = (unsafe { ComRef::from_raw(handler) }) else { return false };
        self.handlers.borrow_mut().push((h.to_com_ptr(), fd));
        true
    }

    /// False if `handler` wasn't registered.
    pub fn unregister_handler(&self, handler: *mut IEventHandler) -> bool {
        let mut hs = self.handlers.borrow_mut();
        let n = hs.len();
        hs.retain(|(h, _)| h.as_ptr() != handler);
        hs.len() != n
    }

    /// Call `handler` every `ms` milliseconds (at least one), starting one interval
    /// from now. False for a null handler.
    ///
    /// # Safety
    /// `handler` is null or a live `ITimerHandler`.
    pub unsafe fn register_timer(&self, handler: *mut ITimerHandler, ms: u64) -> bool {
        let Some(h) = (unsafe { ComRef::from_raw(handler) }) else { return false };
        let every = Duration::from_millis(ms.max(1));
        self.timers.borrow_mut().push(Timer { handler: h.to_com_ptr(), every, due: Instant::now() + every });
        true
    }

    /// False if `handler` wasn't registered.
    pub fn unregister_timer(&self, handler: *mut ITimerHandler) -> bool {
        let mut ts = self.timers.borrow_mut();
        let n = ts.len();
        ts.retain(|t| t.handler.as_ptr() != handler);
        ts.len() != n
    }

    /// Serve the plug-in once: every registered descriptor that is readable now, then
    /// every timer that is due. Handlers may register or unregister from inside their
    /// callbacks; one unregistered by an earlier callback in the same poll isn't called.
    pub fn poll(&self) {
        let handlers = self.handlers.borrow().clone();
        if !handlers.is_empty() {
            let mut fds: Vec<libc::pollfd> =
                handlers.iter().map(|(_, fd)| libc::pollfd { fd: *fd, events: libc::POLLIN | libc::POLLPRI, revents: 0 }).collect();
            let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 0) };
            if ready > 0 {
                for (pfd, (h, fd)) in fds.iter().zip(&handlers) {
                    if pfd.revents != 0 && self.has_handler(h.as_ptr()) {
                        unsafe { h.onFDIsSet(*fd) };
                    }
                }
            }
        }
        let now = Instant::now();
        let due: Vec<ComPtr<ITimerHandler>> = self
            .timers
            .borrow_mut()
            .iter_mut()
            .filter(|t| t.due <= now)
            .map(|t| {
                t.due = now + t.every;
                t.handler.clone()
            })
            .collect();
        for h in due {
            if self.timers.borrow().iter().any(|t| t.handler.as_ptr() == h.as_ptr()) {
                unsafe { h.onTimer() };
            }
        }
    }

    fn has_handler(&self, handler: *mut IEventHandler) -> bool {
        self.handlers.borrow().iter().any(|(h, _)| h.as_ptr() == handler)
    }

    /// Forget every handler and timer (the editor is gone).
    pub fn clear(&self) {
        self.handlers.borrow_mut().clear();
        self.timers.borrow_mut().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use vst3::{Class, ComWrapper};

    /// Counts the calls it gets.
    struct Counter(Arc<AtomicU32>);

    impl Class for Counter {
        type Interfaces = (IEventHandler, ITimerHandler);
    }

    impl IEventHandlerTrait for Counter {
        unsafe fn onFDIsSet(&self, _fd: FileDescriptor) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl ITimerHandlerTrait for Counter {
        unsafe fn onTimer(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn counter() -> (ComWrapper<Counter>, Arc<AtomicU32>) {
        let n = Arc::new(AtomicU32::new(0));
        (ComWrapper::new(Counter(n.clone())), n)
    }

    #[test]
    fn a_timer_fires_once_per_interval_after_the_first() {
        let rl = RunLoop::default();
        let (c, n) = counter();
        let h = c.to_com_ptr::<ITimerHandler>().unwrap();
        assert!(unsafe { rl.register_timer(h.as_ptr(), 20) });
        rl.poll();
        assert_eq!(n.load(Ordering::SeqCst), 0, "not before its first interval");
        std::thread::sleep(Duration::from_millis(25));
        rl.poll();
        rl.poll();
        assert_eq!(n.load(Ordering::SeqCst), 1, "once when due, not again until the next interval");
        std::thread::sleep(Duration::from_millis(25));
        rl.poll();
        assert_eq!(n.load(Ordering::SeqCst), 2);
        assert!(rl.unregister_timer(h.as_ptr()));
        assert!(!rl.unregister_timer(h.as_ptr()), "already gone");
        std::thread::sleep(Duration::from_millis(25));
        rl.poll();
        assert_eq!(n.load(Ordering::SeqCst), 2, "nothing after unregistering");
    }

    #[test]
    fn a_handler_is_called_while_its_descriptor_is_readable() {
        let rl = RunLoop::default();
        let (c, n) = counter();
        let h = c.to_com_ptr::<IEventHandler>().unwrap();
        let mut pipe = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
        assert!(unsafe { rl.register_handler(h.as_ptr(), pipe[0]) });
        rl.poll();
        assert_eq!(n.load(Ordering::SeqCst), 0, "nothing to read");
        assert_eq!(unsafe { libc::write(pipe[1], b"x".as_ptr().cast(), 1) }, 1);
        rl.poll();
        assert_eq!(n.load(Ordering::SeqCst), 1);
        rl.poll();
        assert_eq!(n.load(Ordering::SeqCst), 2, "still readable: the handler is the one that reads");
        assert!(rl.unregister_handler(h.as_ptr()));
        rl.poll();
        assert_eq!(n.load(Ordering::SeqCst), 2, "nothing after unregistering");
        unsafe {
            libc::close(pipe[0]);
            libc::close(pipe[1]);
        }
    }

    #[test]
    fn null_handlers_are_refused_and_clear_forgets_everything() {
        let rl = RunLoop::default();
        assert!(!unsafe { rl.register_handler(std::ptr::null_mut(), 0) });
        assert!(!unsafe { rl.register_timer(std::ptr::null_mut(), 1) });
        let (c, n) = counter();
        unsafe { rl.register_timer(c.to_com_ptr::<ITimerHandler>().unwrap().as_ptr(), 1) };
        rl.clear();
        std::thread::sleep(Duration::from_millis(5));
        rl.poll();
        assert_eq!(n.load(Ordering::SeqCst), 0);
    }
}
