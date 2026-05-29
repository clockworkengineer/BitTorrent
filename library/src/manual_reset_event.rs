use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct ManualResetEvent {
    inner: Mutex<bool>,
    condvar: Condvar,
}

impl ManualResetEvent {
    pub fn new(initial_state: bool) -> Self {
        ManualResetEvent {
            inner: Mutex::new(initial_state),
            condvar: Condvar::new(),
        }
    }

    pub fn set(&self) {
        let mut guard = self.inner.lock().unwrap();
        *guard = true;
        self.condvar.notify_all();
    }

    pub fn reset(&self) {
        let mut guard = self.inner.lock().unwrap();
        *guard = false;
    }

    pub fn wait_one(&self, timeout_ms: u64) -> bool {
        if timeout_ms == 0 {
            let guard = self.inner.lock().unwrap();
            *guard
        } else {
            let mut guard = self.inner.lock().unwrap();
            let deadline = Instant::now() + Duration::from_millis(timeout_ms);
            while !*guard {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                let remaining = deadline - now;
                let (g, _) = self.condvar.wait_timeout(guard, remaining).unwrap();
                guard = g;
            }
            *guard
        }
    }
}
