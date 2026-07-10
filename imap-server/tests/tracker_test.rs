//! Tracker unit tests.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use imap_core::types::Flag;
use imap_server::tracker::*;

#[test]
fn test_session_tracker_new() {
    let mut tracker = SessionTracker::new();
    assert!(!tracker.has_pending());
    assert!(tracker.drain().is_empty());
}

#[test]
fn test_session_tracker_drain_clears() {
    let mailbox = MailboxTracker::new(10);
    let session = Arc::new(Mutex::new(SessionTracker::new()));
    mailbox.register(Arc::downgrade(&session));

    mailbox.queue_expunge(5, None);

    {
        let mut s = session.lock().unwrap();
        assert!(s.has_pending());
        let updates = s.drain();
        assert_eq!(updates.len(), 1);
        assert!(!s.has_pending());
        assert!(s.drain().is_empty());
    }
}

#[test]
fn test_mailbox_tracker_broadcast() {
    let tracker = MailboxTracker::new(10);
    let session1 = Arc::new(Mutex::new(SessionTracker::new()));
    let session2 = Arc::new(Mutex::new(SessionTracker::new()));

    tracker.register(Arc::downgrade(&session1));
    tracker.register(Arc::downgrade(&session2));

    // Broadcast to all (no source)
    tracker.queue_expunge(5, None);

    let updates1 = session1.lock().unwrap().drain();
    let updates2 = session2.lock().unwrap().drain();
    assert_eq!(updates1.len(), 1);
    assert_eq!(updates2.len(), 1);
}

#[test]
fn test_mailbox_tracker_skip_source() {
    let tracker = MailboxTracker::new(10);
    let session1 = Arc::new(Mutex::new(SessionTracker::new()));
    let session2 = Arc::new(Mutex::new(SessionTracker::new()));

    tracker.register(Arc::downgrade(&session1));
    tracker.register(Arc::downgrade(&session2));

    // Broadcast skipping session1
    tracker.queue_num_messages(15, Some(&session1));

    let updates1 = session1.lock().unwrap().drain();
    let updates2 = session2.lock().unwrap().drain();
    assert_eq!(updates1.len(), 0); // skipped
    assert_eq!(updates2.len(), 1); // received
}

#[test]
fn test_mailbox_tracker_num_messages() {
    let tracker = MailboxTracker::new(10);
    assert_eq!(tracker.num_messages(), 10);

    tracker.queue_num_messages(20, None);
    assert_eq!(tracker.num_messages(), 20);
}

#[test]
fn test_tracker_update_types() {
    let tracker = MailboxTracker::new(5);
    let session = Arc::new(Mutex::new(SessionTracker::new()));
    tracker.register(Arc::downgrade(&session));

    tracker.queue_expunge(3, None);
    tracker.queue_num_messages(4, None);
    tracker.queue_mailbox_flags(vec![Flag::seen()], None);
    tracker.queue_message_flags(1, 100, vec![Flag::flagged()], None);

    let updates = session.lock().unwrap().drain();
    assert_eq!(updates.len(), 4);

    match &updates[0] {
        TrackerUpdate::Expunge(seq) => assert_eq!(*seq, 3),
        _ => panic!("expected Expunge"),
    }
    match &updates[1] {
        TrackerUpdate::NumMessages(n) => assert_eq!(*n, 4),
        _ => panic!("expected NumMessages"),
    }
    match &updates[2] {
        TrackerUpdate::MailboxFlags(flags) => {
            assert_eq!(flags.len(), 1);
            assert_eq!(flags[0], Flag::seen());
        }
        _ => panic!("expected MailboxFlags"),
    }
    match &updates[3] {
        TrackerUpdate::MessageFlags { seq, uid, flags } => {
            assert_eq!(*seq, 1);
            assert_eq!(*uid, 100);
            assert_eq!(flags.len(), 1);
            assert_eq!(flags[0], Flag::flagged());
        }
        _ => panic!("expected MessageFlags"),
    }
}

#[test]
fn test_mailbox_tracker_dead_session_cleanup() {
    let tracker = MailboxTracker::new(10);
    let session = Arc::new(Mutex::new(SessionTracker::new()));
    tracker.register(Arc::downgrade(&session));

    // Drop the session
    drop(session);

    // Should not panic — dead sessions are cleaned up
    tracker.queue_expunge(1, None);
}

#[test]
fn test_mailbox_tracker_multiple_expunges() {
    let tracker = MailboxTracker::new(10);
    let session = Arc::new(Mutex::new(SessionTracker::new()));
    tracker.register(Arc::downgrade(&session));

    tracker.queue_expunge(3, None);
    tracker.queue_expunge(7, None);
    tracker.queue_expunge(1, None);

    let updates = session.lock().unwrap().drain();
    assert_eq!(updates.len(), 3);

    match &updates[0] {
        TrackerUpdate::Expunge(seq) => assert_eq!(*seq, 3),
        _ => panic!("expected Expunge"),
    }
    match &updates[1] {
        TrackerUpdate::Expunge(seq) => assert_eq!(*seq, 7),
        _ => panic!("expected Expunge"),
    }
    match &updates[2] {
        TrackerUpdate::Expunge(seq) => assert_eq!(*seq, 1),
        _ => panic!("expected Expunge"),
    }
}

#[tokio::test]
async fn test_session_tracker_notification_channel() {
    let mailbox = MailboxTracker::new(10);
    let session = Arc::new(Mutex::new(SessionTracker::new()));
    mailbox.register(Arc::downgrade(&session));

    let mut rx = session.lock().unwrap().take_receiver();

    // Queue an update — should signal through the channel
    mailbox.queue_expunge(5, None);

    // The receiver should get a notification
    let result = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
    assert!(result.is_ok(), "should receive notification within timeout");
    assert!(result.unwrap().is_some(), "channel should not be closed");

    // The pending vec should also have the update
    let updates = session.lock().unwrap().drain();
    assert_eq!(updates.len(), 1);
    match &updates[0] {
        TrackerUpdate::Expunge(seq) => assert_eq!(*seq, 5),
        _ => panic!("expected Expunge"),
    }
}

#[tokio::test]
async fn test_session_tracker_take_receiver_twice() {
    let mailbox = MailboxTracker::new(10);
    let session = Arc::new(Mutex::new(SessionTracker::new()));
    mailbox.register(Arc::downgrade(&session));

    let mut rx1 = session.lock().unwrap().take_receiver();

    mailbox.queue_expunge(1, None);

    // First receiver gets the notification
    let result = tokio::time::timeout(Duration::from_millis(100), rx1.recv()).await;
    assert!(result.is_ok(), "first receiver should get notification");
    session.lock().unwrap().drain();

    // Drop the first receiver (simulates end of IDLE)
    drop(rx1);

    let mut rx2 = session.lock().unwrap().take_receiver();

    mailbox.queue_expunge(2, None);

    // Second receiver should also get notifications
    let result = tokio::time::timeout(Duration::from_millis(100), rx2.recv()).await;
    assert!(result.is_ok(), "second receiver should get notification");

    let updates = session.lock().unwrap().drain();
    assert_eq!(updates.len(), 1);
    match &updates[0] {
        TrackerUpdate::Expunge(seq) => assert_eq!(*seq, 2),
        _ => panic!("expected Expunge"),
    }
}

#[test]
fn test_session_tracker_push_signals_best_effort() {
    // When no receiver is taken, push() still succeeds (signal is ignored)
    let mailbox = MailboxTracker::new(5);
    let session = Arc::new(Mutex::new(SessionTracker::new()));
    mailbox.register(Arc::downgrade(&session));

    mailbox.queue_num_messages(10, None);

    // Updates are queued even without a receiver
    let updates = session.lock().unwrap().drain();
    assert_eq!(updates.len(), 1);
}

#[tokio::test]
async fn test_session_tracker_channel_capacity() {
    let mailbox = MailboxTracker::new(0);
    let session = Arc::new(Mutex::new(SessionTracker::new()));
    mailbox.register(Arc::downgrade(&session));

    let mut rx = session.lock().unwrap().take_receiver();

    // Send more updates than channel capacity (64).
    // push() uses try_send which ignores Full errors — no panic.
    for i in 0..100 {
        mailbox.queue_expunge(i, None);
    }

    // All updates are still in the pending vec
    let updates = session.lock().unwrap().drain();
    assert_eq!(updates.len(), 100);

    // Channel received at most 64 signals (its capacity); excess were dropped
    let mut signal_count = 0;
    while rx.try_recv().is_ok() {
        signal_count += 1;
    }
    assert!(signal_count <= 64, "channel capacity is 64, got {signal_count}");
}
