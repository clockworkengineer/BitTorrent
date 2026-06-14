//! Manual reset event synchronization primitive
//!
//! A synchronization primitive that allows threads to wait until a signal is set
//! manually. Similar to Win32's ManualResetEvent or .NET's ManualResetEvent.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// A synchronization event that, when signaled, must be reset manually.
#[derive(Debug)]
pub struct ManualResetEvent {
    inner: Mutex<bool>,
    condvar: Condvar,
}

impl ManualResetEvent {
    /// Creates a new `ManualResetEvent` with the specified initial state.
    pub fn new(initial_state: bool) -> Self {
        ManualResetEvent {
            inner: Mutex::new(initial_state),
            condvar: Condvar::new(),
        }
    }

    /// Sets the state of the event to signaled, allowing one or more waiting threads to proceed.
    pub fn set(&self) {
        let mut guard = self.inner.lock().unwrap();
        *guard = true;
        self.condvar.notify_all();
    }

    /// Resets the state of the event to non-signaled, causing threads that call `wait_one` to block.
    pub fn reset(&self) {
        let mut guard = self.inner.lock().unwrap();
        *guard = false;
    }

    /// Blocks the current thread until the current `ManualResetEvent` receives a signal,
    /// or until the specified timeout (in milliseconds) expires.
    /// If `timeout_ms` is 0, it performs a non-blocking check of the signal state.
    /// Returns `true` if the event is signaled; otherwise, `false`.
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
