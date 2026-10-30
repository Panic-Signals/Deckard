//! Worker threads that render sounds alongside the audio thread (as Maschine's
//! `ni::ae::ThreadedGraphExecutor` does, docs/maschine-audio-engine.md).
//!
//! [`RenderPool::run`] hands out the jobs of one batch (job `i` for `i < n`): the
//! calling thread and any awake workers claim them one at a time, and `run` returns
//! when every job has finished. It never allocates or takes a lock, so the audio thread
//! can call it; if no worker wakes in time the caller does every job itself, so the
//! result never depends on how many threads helped.
//!
//! Workers run at real-time priority, and on macOS join the audio device's workgroup
//! (Apple's way of telling the scheduler they share the audio deadline).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::thread::{JoinHandle, Thread};

type Task<'a> = &'a (dyn Fn(usize) + Sync);

/// The batch being worked on, packed in one word so claiming a job is a single
/// compare-and-swap: generation (24 bits), job count (20), next job (20).
fn pack(generation: u64, n: u64, next: u64) -> u64 {
    (generation & 0xff_ffff) << 40 | (n & 0xf_ffff) << 20 | (next & 0xf_ffff)
}

fn unpack(s: u64) -> (u64, u64, u64) {
    (s >> 40, (s >> 20) & 0xf_ffff, s & 0xf_ffff)
}

struct Shared {
    state: AtomicU64,
    /// Jobs of the current batch finished.
    done: AtomicUsize,
    /// The current batch's task (a `*const Task` on the caller's stack, valid until
    /// its `run` returns).
    task: AtomicPtr<()>,
    quit: AtomicBool,
    /// Per worker: asleep (needs waking for a batch).
    asleep: Vec<AtomicBool>,
    /// Jobs finished by workers (rather than the caller), for diagnostics.
    helped: AtomicU64,
    /// The audio device's workgroup (retained; 0 until found, or if there is none). Looked
    /// up on a thread of its own: Core Audio's first answer in a process can take a minute,
    /// and the workers mustn't wait for it before they help.
    workgroup: AtomicUsize,
}

impl Drop for Shared {
    fn drop(&mut self) {
        rt::release(self.workgroup.load(Ordering::Acquire));
    }
}

impl Shared {
    /// Claim the next job of batch `generation`, if any is left.
    fn claim(&self, generation: u64) -> Option<usize> {
        let mut s = self.state.load(Ordering::Acquire);
        loop {
            let (g, n, next) = unpack(s);
            if g != generation & 0xff_ffff || next >= n {
                return None;
            }
            match self.state.compare_exchange_weak(s, pack(g, n, next + 1), Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(next as usize),
                Err(now) => s = now,
            }
        }
    }

    /// Do jobs of batch `generation` until none are left; how many were done.
    fn work(&self, generation: u64) -> usize {
        let mut count = 0;
        while let Some(i) = self.claim(generation) {
            // A claimed job keeps its batch's `run` waiting, so the task is alive.
            let task = unsafe { *(self.task.load(Ordering::Acquire) as *const Task) };
            task(i);
            self.done.fetch_add(1, Ordering::AcqRel);
            count += 1;
        }
        count
    }
}

/// Worker threads for rendering; see the module docs.
pub struct RenderPool {
    shared: Arc<Shared>,
    threads: Vec<Thread>,
    handles: Vec<JoinHandle<()>>,
    generation: AtomicU64,
}

/// How long a worker keeps looking for the next batch before it sleeps: batches come
/// in bursts (one per event within a block).
const LINGER: std::time::Duration = std::time::Duration::from_micros(50);

impl RenderPool {
    /// `workers` threads, given real-time priority for blocks of `period_frames` at
    /// `sample_rate`.
    pub fn new(workers: usize, period_frames: u32, sample_rate: u32) -> RenderPool {
        let shared = Arc::new(Shared {
            state: AtomicU64::new(0),
            done: AtomicUsize::new(0),
            task: AtomicPtr::new(std::ptr::null_mut()),
            quit: AtomicBool::new(false),
            asleep: (0..workers).map(|_| AtomicBool::new(false)).collect(),
            helped: AtomicU64::new(0),
            workgroup: AtomicUsize::new(0),
        });
        {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("DECKARD workgroup".into())
                .spawn(move || shared.workgroup.store(rt::audio_workgroup(), Ordering::Release))
                .expect("workgroup thread");
        }
        let mut handles = Vec::new();
        for w in 0..workers {
            let shared = shared.clone();
            let h = std::thread::Builder::new()
                .name(format!("DECKARD render {}", w + 1))
                .spawn(move || {
                    rt::promote(period_frames, sample_rate);
                    // Joined once the workgroup is known (tried once).
                    let (mut joined, mut tried) = (None, false);
                    let mut seen = 0u64;
                    while !shared.quit.load(Ordering::Acquire) {
                        if !tried {
                            let wg = shared.workgroup.load(Ordering::Acquire);
                            if wg != 0 {
                                (joined, tried) = (rt::join(wg), true);
                            }
                        }
                        let (g, ..) = unpack(shared.state.load(Ordering::Acquire));
                        if g != seen {
                            seen = g;
                            let n = shared.work(g);
                            shared.helped.fetch_add(n as u64, Ordering::Relaxed);
                            continue;
                        }
                        // Linger for the next batch, then sleep until woken.
                        let start = std::time::Instant::now();
                        let mut woke = false;
                        while start.elapsed() < LINGER {
                            if unpack(shared.state.load(Ordering::Acquire)).0 != seen || shared.quit.load(Ordering::Acquire) {
                                woke = true;
                                break;
                            }
                            std::hint::spin_loop();
                        }
                        if !woke {
                            shared.asleep[w].store(true, Ordering::Release);
                            // Re-check after saying so (a batch may have started).
                            if unpack(shared.state.load(Ordering::Acquire)).0 == seen && !shared.quit.load(Ordering::Acquire) {
                                std::thread::park();
                            }
                            shared.asleep[w].store(false, Ordering::Release);
                        }
                    }
                    drop(joined);
                })
                .expect("render thread");
            handles.push(h);
        }
        let threads = handles.iter().map(|h| h.thread().clone()).collect();
        RenderPool { shared, threads, handles, generation: AtomicU64::new(0) }
    }

    /// A pool sized for this machine: the audio workgroup's suggested parallelism, else
    /// the cores less one (for the audio thread), at most 8.
    pub fn for_machine(period_frames: u32, sample_rate: u32) -> RenderPool {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let workers = rt::workgroup_parallelism().unwrap_or(cores).saturating_sub(1).clamp(0, 8);
        RenderPool::new(workers, period_frames, sample_rate)
    }

    pub fn workers(&self) -> usize {
        self.threads.len()
    }

    /// Jobs finished by worker threads so far.
    pub fn helped(&self) -> u64 {
        self.shared.helped.load(Ordering::Relaxed)
    }

    /// Run `task(i)` for every `i < n` (on this thread and the workers) and return when
    /// all are done. Not to be called from two threads at once.
    pub fn run(&self, n: usize, task: Task<'_>) {
        if n == 0 {
            return;
        }
        if n == 1 || self.threads.is_empty() {
            (0..n).for_each(task);
            return;
        }
        let generation = (self.generation.fetch_add(1, Ordering::Relaxed) + 1) & 0xff_ffff;
        let holder: Task<'_> = task;
        self.shared.task.store(&holder as *const Task as *mut (), Ordering::Release);
        self.shared.done.store(0, Ordering::Release);
        self.shared.state.store(pack(generation, n as u64, 0), Ordering::Release);
        // Wake as many sleeping workers as there are other jobs.
        let mut want = n - 1;
        for (w, t) in self.threads.iter().enumerate() {
            if want == 0 {
                break;
            }
            if self.shared.asleep[w].swap(false, Ordering::AcqRel) {
                t.unpark();
            }
            want -= 1;
        }
        self.shared.work(generation);
        // Wait for jobs other threads claimed.
        while self.shared.done.load(Ordering::Acquire) < n {
            std::hint::spin_loop();
        }
        self.shared.task.store(std::ptr::null_mut(), Ordering::Release);
    }
}

impl Drop for RenderPool {
    fn drop(&mut self) {
        self.shared.quit.store(true, Ordering::Release);
        for t in &self.threads {
            t.unpark();
        }
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
}

/// Give the calling thread real-time (time-constraint) scheduling for blocks of
/// `period_frames` at `sample_rate`, as the render workers have.
pub fn promote_current(period_frames: u32, sample_rate: u32) {
    rt::promote(period_frames, sample_rate);
}

/// Real-time scheduling for the workers.
#[cfg(target_os = "macos")]
mod rt {
    use std::ffi::c_void;

    #[repr(C)]
    struct TimeConstraint {
        period: u32,
        computation: u32,
        constraint: u32,
        preemptible: i32,
    }

    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }

    const THREAD_TIME_CONSTRAINT_POLICY: u32 = 2;

    unsafe extern "C" {
        fn mach_thread_self() -> u32;
        fn thread_policy_set(thread: u32, flavor: u32, info: *const TimeConstraint, count: u32) -> i32;
        fn mach_timebase_info(info: *mut Timebase) -> i32;
    }

    /// Time-constraint scheduling, as Core Audio gives its own I/O thread: blocks of
    /// `frames` at `rate`, needing up to half of each.
    pub fn promote(frames: u32, rate: u32) {
        let mut tb = Timebase { numer: 1, denom: 1 };
        unsafe { mach_timebase_info(&mut tb) };
        let ns = frames as f64 / rate.max(1) as f64 * 1e9;
        let ticks = |ns: f64| (ns * tb.denom as f64 / tb.numer.max(1) as f64) as u32;
        let policy = TimeConstraint { period: ticks(ns), computation: ticks(ns * 0.5), constraint: ticks(ns), preemptible: 1 };
        unsafe { thread_policy_set(mach_thread_self(), THREAD_TIME_CONSTRAINT_POLICY, &policy, 4) };
    }

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    const fn fourcc(s: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*s)
    }

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyData(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
            data: *mut c_void,
        ) -> i32;
    }

    unsafe extern "C" {
        fn os_workgroup_join(wg: *mut c_void, token: *mut JoinToken) -> i32;
        fn os_workgroup_leave(wg: *mut c_void, token: *mut JoinToken);
        fn os_workgroup_max_parallel_threads(wg: *mut c_void, attr: *const c_void) -> i32;
        fn os_release(object: *mut c_void);
        fn os_retain(object: *mut c_void) -> *mut c_void;
    }

    /// `os_workgroup_join_token_s`.
    #[repr(C)]
    pub struct JoinToken {
        sig: u32,
        opaque: [u8; 36],
    }

    /// The default output device's I/O workgroup (retained), if it has one.
    fn device_workgroup() -> Option<*mut c_void> {
        const SYSTEM: u32 = 1;
        let global = fourcc(b"glob");
        let mut device = 0u32;
        let mut size = 4u32;
        let addr = Address { selector: fourcc(b"dOut"), scope: global, element: 0 };
        let r =
            unsafe { AudioObjectGetPropertyData(SYSTEM, &addr, 0, std::ptr::null(), &mut size, &mut device as *mut u32 as *mut c_void) };
        if r != 0 || device == 0 {
            return None;
        }
        let mut wg: *mut c_void = std::ptr::null_mut();
        let mut size = std::mem::size_of::<*mut c_void>() as u32;
        let addr = Address { selector: fourcc(b"oswg"), scope: global, element: 0 };
        let r = unsafe {
            AudioObjectGetPropertyData(device, &addr, 0, std::ptr::null(), &mut size, &mut wg as *mut *mut c_void as *mut c_void)
        };
        (r == 0 && !wg.is_null()).then_some(wg)
    }

    /// Membership of the audio workgroup, left when dropped.
    pub struct Joined {
        wg: *mut c_void,
        token: JoinToken,
    }

    impl Drop for Joined {
        fn drop(&mut self) {
            unsafe {
                os_workgroup_leave(self.wg, &mut self.token);
                os_release(self.wg);
            }
        }
    }

    /// The output device's audio workgroup (retained, as an address; 0: none).
    pub fn audio_workgroup() -> usize {
        device_workgroup().map_or(0, |wg| wg as usize)
    }

    /// Release a workgroup from [`audio_workgroup`] (0: nothing).
    pub fn release(wg: usize) {
        if wg != 0 {
            unsafe { os_release(wg as *mut c_void) };
        }
    }

    /// Join workgroup `wg` (from [`audio_workgroup`]) on this thread.
    pub fn join(wg: usize) -> Option<Joined> {
        let wg = unsafe { os_retain(wg as *mut c_void) };
        let mut token = JoinToken { sig: 0, opaque: [0; 36] };
        if unsafe { os_workgroup_join(wg, &mut token) } != 0 {
            unsafe { os_release(wg) };
            return None;
        }
        Some(Joined { wg, token })
    }

    /// How many threads the audio workgroup suggests working in parallel.
    pub fn workgroup_parallelism() -> Option<usize> {
        let wg = device_workgroup()?;
        let n = unsafe { os_workgroup_max_parallel_threads(wg, std::ptr::null()) };
        unsafe { os_release(wg) };
        (n > 0).then_some(n as usize)
    }
}

#[cfg(not(target_os = "macos"))]
mod rt {
    pub fn promote(_frames: u32, _rate: u32) {}
    pub fn audio_workgroup() -> usize {
        0
    }
    pub fn release(_wg: usize) {}
    pub fn join(_wg: usize) -> Option<()> {
        None
    }
    pub fn workgroup_parallelism() -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every job of every batch runs exactly once, whatever the batch sizes and however
    /// the workers wake.
    #[test]
    fn every_job_runs_once() {
        let pool = RenderPool::new(4, 256, 48_000);
        let counts: Vec<AtomicUsize> = (0..64).map(|_| AtomicUsize::new(0)).collect();
        let mut expected = vec![0usize; 64];
        for round in 0..3_000 {
            let n = 1 + round % 64;
            pool.run(n, &|i| {
                counts[i].fetch_add(1, Ordering::Relaxed);
                // Work the size of a plug-in's block (unevenly spread): a few µs to ~30.
                let until = std::time::Instant::now() + std::time::Duration::from_micros(2 + (i as u64 * 7) % 30);
                while std::time::Instant::now() < until {
                    std::hint::spin_loop();
                }
            });
            for e in expected.iter_mut().take(n) {
                *e += 1;
            }
            if round % 997 == 0 {
                // Let the workers fall asleep now and then.
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        }
        let got: Vec<usize> = counts.iter().map(|c| c.load(Ordering::Relaxed)).collect();
        assert_eq!(got, expected);
        // The workers take part: on a busy machine the caller can finish such short jobs
        // before a worker wakes, so give them long ones until one helps (with every core
        // busy, as when all the engine's tests run, that can take most of a second).
        let start = std::time::Instant::now();
        while pool.helped() == 0 && start.elapsed() < std::time::Duration::from_secs(10) {
            pool.run(64, &|_| std::thread::sleep(std::time::Duration::from_millis(1)));
        }
        assert!(pool.helped() > 0, "the workers took part");
    }

    #[test]
    fn packing_round_trips() {
        assert_eq!(unpack(pack(0xab_cdef, 999, 3)), (0xab_cdef, 999, 3));
    }
}
