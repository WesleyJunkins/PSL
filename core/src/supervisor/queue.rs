//! A bounded frame queue with drop-oldest or blocking behaviour (docs/01 §Backpressure).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};

use crate::model::OnFull;
use crate::protocol::Frame;

pub struct DropQueue {
    inner: Mutex<Inner>,
    not_empty: Condvar,
    not_full: Condvar,
    cap: usize,
    on_full: OnFull,
    pub pushed: AtomicU64,
    pub dropped: AtomicU64,
}

struct Inner {
    q: VecDeque<Frame>,
    closed: bool,
}

impl DropQueue {
    pub fn new(cap: usize, on_full: OnFull) -> DropQueue {
        DropQueue {
            inner: Mutex::new(Inner {
                q: VecDeque::with_capacity(cap.min(1024)),
                closed: false,
            }),
            not_empty: Condvar::new(),
            not_full: Condvar::new(),
            cap: cap.max(1),
            on_full,
            pushed: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    pub fn push(&self, frame: Frame) {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return;
        }
        while g.q.len() >= self.cap {
            match self.on_full {
                OnFull::DropOldest => {
                    g.q.pop_front();
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                }
                OnFull::Block => {
                    g = self.not_full.wait(g).unwrap();
                    if g.closed {
                        return;
                    }
                }
            }
        }
        g.q.push_back(frame);
        self.pushed.fetch_add(1, Ordering::Relaxed);
        drop(g);
        self.not_empty.notify_one();
    }

    /// Blocks until a frame is available; `None` once closed and drained.
    pub fn pop(&self) -> Option<Frame> {
        let mut g = self.inner.lock().unwrap();
        loop {
            if let Some(f) = g.q.pop_front() {
                drop(g);
                self.not_full.notify_one();
                return Some(f);
            }
            if g.closed {
                return None;
            }
            g = self.not_empty.wait(g).unwrap();
        }
    }

    pub fn close(&self) {
        let mut g = self.inner.lock().unwrap();
        g.closed = true;
        drop(g);
        self.not_empty.notify_all();
        self.not_full.notify_all();
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().q.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::FrameType;

    #[test]
    fn drops_oldest() {
        let q = DropQueue::new(2, OnFull::DropOldest);
        for i in 0..5u32 {
            q.push(Frame::new(FrameType::Signal, 0, 0, i, vec![]));
        }
        assert_eq!(q.dropped.load(Ordering::Relaxed), 3);
        assert_eq!(q.pop().unwrap().header.seq, 3);
        assert_eq!(q.pop().unwrap().header.seq, 4);
        q.close();
        assert!(q.pop().is_none());
    }
}
