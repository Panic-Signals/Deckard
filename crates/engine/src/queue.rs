//! A bounded lock-free queue (Dmitry Vyukov's MPMC ring): any number of threads push
//! and pop without locks or allocation once made, so the audio thread can use it.

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Cell<T> {
    seq: AtomicUsize,
    value: UnsafeCell<MaybeUninit<T>>,
}

pub struct Queue<T> {
    cells: Box<[Cell<T>]>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
}

// Values move between threads through the cells; each cell is owned by one thread at a
// time (its sequence number says whose turn it is).
unsafe impl<T: Send> Send for Queue<T> {}
unsafe impl<T: Send> Sync for Queue<T> {}

impl<T> Queue<T> {
    /// A queue holding up to `capacity` values (rounded up to a power of two).
    pub fn new(capacity: usize) -> Queue<T> {
        let n = capacity.max(2).next_power_of_two();
        let cells = (0..n).map(|i| Cell { seq: AtomicUsize::new(i), value: UnsafeCell::new(MaybeUninit::uninit()) }).collect();
        Queue { cells, mask: n - 1, head: AtomicUsize::new(0), tail: AtomicUsize::new(0) }
    }

    /// Add `value`, or give it back if the queue is full.
    pub fn push(&self, value: T) -> Result<(), T> {
        let mut pos = self.tail.load(Ordering::Relaxed);
        loop {
            let cell = &self.cells[pos & self.mask];
            let seq = cell.seq.load(Ordering::Acquire);
            let diff = seq as isize - pos as isize;
            if diff == 0 {
                match self.tail.compare_exchange_weak(pos, pos + 1, Ordering::Relaxed, Ordering::Relaxed) {
                    Ok(_) => {
                        unsafe { (*cell.value.get()).write(value) };
                        cell.seq.store(pos + 1, Ordering::Release);
                        return Ok(());
                    }
                    Err(now) => pos = now,
                }
            } else if diff < 0 {
                return Err(value);
            } else {
                pos = self.tail.load(Ordering::Relaxed);
            }
        }
    }

    /// Whether it holds nothing (a moment's view: other threads may push or pop).
    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Acquire) >= self.tail.load(Ordering::Acquire)
    }

    /// Take the oldest value, if any.
    pub fn pop(&self) -> Option<T> {
        let mut pos = self.head.load(Ordering::Relaxed);
        loop {
            let cell = &self.cells[pos & self.mask];
            let seq = cell.seq.load(Ordering::Acquire);
            let diff = seq as isize - (pos + 1) as isize;
            if diff == 0 {
                match self.head.compare_exchange_weak(pos, pos + 1, Ordering::Relaxed, Ordering::Relaxed) {
                    Ok(_) => {
                        let value = unsafe { (*cell.value.get()).assume_init_read() };
                        cell.seq.store(pos + self.mask + 1, Ordering::Release);
                        return Some(value);
                    }
                    Err(now) => pos = now,
                }
            } else if diff < 0 {
                return None;
            } else {
                pos = self.head.load(Ordering::Relaxed);
            }
        }
    }
}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        while self.pop().is_some() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn fifo_and_full() {
        let q = Queue::new(4);
        for i in 0..4 {
            q.push(i).unwrap();
        }
        assert_eq!(q.push(9), Err(9), "full");
        assert_eq!((q.pop(), q.pop()), (Some(0), Some(1)));
        q.push(4).unwrap();
        assert_eq!([q.pop(), q.pop(), q.pop(), q.pop()], [Some(2), Some(3), Some(4), None]);
    }

    /// Four threads push 50 000 values each while one pops: every value arrives once,
    /// each thread's in order.
    #[test]
    fn many_producers() {
        let q = Arc::new(Queue::new(256));
        let producers: Vec<_> = (0..4u64)
            .map(|t| {
                let q = q.clone();
                std::thread::spawn(move || {
                    for i in 0..50_000u64 {
                        let mut v = t << 32 | i;
                        while let Err(back) = q.push(v) {
                            v = back;
                            std::hint::spin_loop();
                        }
                    }
                })
            })
            .collect();
        let mut next = [0u64; 4];
        let mut got = 0;
        while got < 200_000 {
            if let Some(v) = q.pop() {
                let (t, i) = ((v >> 32) as usize, v & 0xffff_ffff);
                assert_eq!(i, next[t], "thread {t} in order");
                next[t] += 1;
                got += 1;
            }
        }
        producers.into_iter().for_each(|p| p.join().unwrap());
        assert_eq!(next, [50_000; 4]);
        assert!(q.pop().is_none());
    }

    #[test]
    fn values_left_are_dropped() {
        let marker = Arc::new(());
        let q = Queue::new(8);
        q.push(marker.clone()).unwrap();
        q.push(marker.clone()).unwrap();
        drop(q);
        assert_eq!(Arc::strong_count(&marker), 1);
    }
}
