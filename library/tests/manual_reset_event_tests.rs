use bittorrent_rs::manual_reset_event::ManualResetEvent;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[test]
fn test_mre_initial_state() {
    let mre_false = ManualResetEvent::new(false);
    assert!(!mre_false.wait_one(0));

    let mre_true = ManualResetEvent::new(true);
    assert!(mre_true.wait_one(0));
}

#[test]
fn test_mre_set_reset() {
    let mre = ManualResetEvent::new(false);
    assert!(!mre.wait_one(0));

    mre.set();
    assert!(mre.wait_one(0));
    assert!(mre.wait_one(10));

    mre.reset();
    assert!(!mre.wait_one(0));
}

#[test]
fn test_mre_threaded_signal() {
    let mre = Arc::new(ManualResetEvent::new(false));
    let mre_clone = mre.clone();

    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        mre_clone.set();
    });

    assert!(mre.wait_one(1000));
    handle.join().unwrap();
}

#[test]
fn test_mre_timeout() {
    let mre = ManualResetEvent::new(false);
    assert!(!mre.wait_one(20));
}

#[test]
fn test_mre_multiple_waiters() {
    let mre = Arc::new(ManualResetEvent::new(false));
    let mut handles = Vec::new();

    for _ in 0..5 {
        let mre_clone = mre.clone();
        handles.push(thread::spawn(move || {
            mre_clone.wait_one(2000)
        }));
    }

    // Give the threads time to start and block
    thread::sleep(Duration::from_millis(50));
    mre.set();

    for handle in handles {
        let success = handle.join().unwrap();
        assert!(success, "Waiting thread failed to unblock on signal");
    }
}
