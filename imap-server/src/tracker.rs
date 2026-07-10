use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};

use imap_core::types::Flag;
use tokio::sync::mpsc;

/// Updates queued by the mailbox tracker for sessions to consume.
#[derive(Debug, Clone)]
pub enum TrackerUpdate {
    /// Messages were expunged. Value is the sequence number.
    Expunge(u32),
    /// Number of messages changed.
    NumMessages(u32),
    /// Mailbox flags changed.
    MailboxFlags(Vec<Flag>),
    /// A message's flags changed.
    MessageFlags {
        seq: u32,
        uid: u32,
        flags: Vec<Flag>,
    },
}

/// Per-session tracker that receives updates from the mailbox tracker.
///
/// When the IDLE loop is active, `push()` signals through the `mpsc` channel
/// so the handler can wake up, drain `pending`, and write untagged responses
/// to the client (EXISTS, EXPUNGE, FETCH FLAGS, etc.).
///
/// The channel is created in `new()` with capacity 64. The receiver is
/// extracted via `take_receiver()` when IDLE starts; if a previous receiver
/// was dropped (prior IDLE ended), `take_receiver()` creates a fresh channel.
pub struct SessionTracker {
    pending: Vec<TrackerUpdate>,
    /// Sender half of the notification channel. `push()` calls `try_send`
    /// best-effort — if the channel is full or closed, the signal is silently
    /// dropped (the update is still in `pending` for the next poll).
    notify_tx: mpsc::Sender<()>,
    /// Receiver half, wrapped in Option so `take_receiver()` can extract it.
    /// None after the first IDLE takes it; `take_receiver()` recreates if needed.
    notify_rx: Option<mpsc::Receiver<()>>,
}

impl Default for SessionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionTracker {
    pub fn new() -> Self {
        let (notify_tx, notify_rx) = mpsc::channel(64);
        Self {
            pending: Vec::new(),
            notify_tx,
            notify_rx: Some(notify_rx),
        }
    }

    /// Drain all pending updates.
    pub fn drain(&mut self) -> Vec<TrackerUpdate> {
        std::mem::take(&mut self.pending)
    }

    /// Returns true if there are pending updates.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    fn push(&mut self, update: TrackerUpdate) {
        self.pending.push(update);
        // Best-effort signal to the IDLE handler. Ignore if the channel
        // is full (the handler will drain on the next tick) or closed
        // (the IDLE session ended and the receiver was dropped).
        self.notify_tx.try_send(()).ok();
    }

    /// Take the notification receiver for IDLE processing.
    ///
    /// Returns the current receiver if available. If a previous receiver
    /// was already taken (and dropped), creates a fresh channel so that
    /// subsequent `push()` calls still have a valid target.
    pub fn take_receiver(&mut self) -> mpsc::Receiver<()> {
        match self.notify_rx.take() {
            Some(rx) => rx,
            None => {
                // Previous receiver was taken and dropped. Create a fresh
                // channel so push() can continue signaling.
                let (tx, rx) = mpsc::channel(64);
                self.notify_tx = tx;
                rx
            }
        }
    }
}

/// Tracks mailbox state changes and distributes them to connected sessions.
#[derive(Debug)]
pub struct MailboxTracker {
    num_messages: AtomicU32,
    sessions: Mutex<Vec<Weak<Mutex<SessionTracker>>>>,
}

impl MailboxTracker {
    pub fn new(num_messages: u32) -> Arc<Self> {
        Arc::new(Self {
            num_messages: AtomicU32::new(num_messages),
            sessions: Mutex::new(Vec::new()),
        })
    }

    /// Register a session tracker to receive updates.
    pub fn register(self: &Arc<Self>, session: Weak<Mutex<SessionTracker>>) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.push(session);
        // Clean up dead weak refs
        sessions.retain(|w| w.strong_count() > 0);
    }

    /// Queue an expunge update to all sessions except the source.
    pub fn queue_expunge(
        &self,
        seq: u32,
        source: Option<&Arc<Mutex<SessionTracker>>>,
    ) {
        let update = TrackerUpdate::Expunge(seq);
        self.broadcast(update, source);
    }

    /// Queue a num_messages update to all sessions except the source.
    pub fn queue_num_messages(
        &self,
        num: u32,
        source: Option<&Arc<Mutex<SessionTracker>>>,
    ) {
        self.num_messages.store(num, Ordering::SeqCst);
        let update = TrackerUpdate::NumMessages(num);
        self.broadcast(update, source);
    }

    /// Queue mailbox flags update to all sessions except the source.
    pub fn queue_mailbox_flags(
        &self,
        flags: Vec<Flag>,
        source: Option<&Arc<Mutex<SessionTracker>>>,
    ) {
        let update = TrackerUpdate::MailboxFlags(flags);
        self.broadcast(update, source);
    }

    /// Queue message flags update to all sessions except the source.
    pub fn queue_message_flags(
        &self,
        seq: u32,
        uid: u32,
        flags: Vec<Flag>,
        source: Option<&Arc<Mutex<SessionTracker>>>,
    ) {
        let update = TrackerUpdate::MessageFlags { seq, uid, flags };
        self.broadcast(update, source);
    }

    /// Get current number of messages.
    pub fn num_messages(&self) -> u32 {
        self.num_messages.load(Ordering::SeqCst)
    }

    fn broadcast(
        &self,
        update: TrackerUpdate,
        source: Option<&Arc<Mutex<SessionTracker>>>,
    ) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|w| w.strong_count() > 0);
        for weak in sessions.iter() {
            if let Some(session) = weak.upgrade() {
                // Skip the source session
                if let Some(src) = source {
                    if Arc::ptr_eq(&session, src) {
                        continue;
                    }
                }
                let mut tracker = session.lock().unwrap();
                tracker.push(update.clone());
            }
        }
    }
}
