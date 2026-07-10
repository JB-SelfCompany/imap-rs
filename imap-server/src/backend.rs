use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use rustls::ServerConfig;

use imap_core::error::ImapError;
use imap_core::fetch::FetchOptions;
use imap_core::search::SearchCriteria;
use imap_core::select::{AppendData, CopyData, ListData, NamespaceData, SelectData, StatusData};
use imap_core::store::StoreFlags;
use imap_core::types::{Flag, SeqSet};

use crate::tracker::MailboxTracker;

/// Connection metadata provided to the backend on login.
#[derive(Debug, Clone)]
pub struct ConnInfo {
    pub peer_addr: Option<SocketAddr>,
}

/// UID set — reuses SeqSet (which uses u32::MAX for `*`).
pub type UIDSet = SeqSet;

/// Data for one fetched message, returned by UserSession::fetch().
#[derive(Debug, Clone)]
pub struct FetchedMessage {
    pub seq: u32,
    pub uid: u32,
    pub flags: Vec<Flag>,
    pub internal_date: String,
    pub rfc822_size: u32,
    pub body: Vec<u8>,
}

/// Data returned by STORE for one message.
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub seq: u32,
    pub uid: u32,
    pub flags: Vec<Flag>,
}

/// Backend trait — the session factory.
#[async_trait]
pub trait Backend: Send + Sync + 'static {
    /// Create a new UserSession for an authenticated user.
    /// Return Err(ImapError::No { .. }) to reject login.
    async fn login(
        &self,
        conn: &ConnInfo,
        username: &str,
        password: &str,
    ) -> Result<Box<dyn UserSession>, ImapError>;

    /// Optional TLS configuration for STARTTLS.
    /// Return None to disable STARTTLS.
    fn tls_config(&self) -> Option<Arc<ServerConfig>> {
        None
    }

    /// Create a mailbox tracker for the given mailbox.
    fn create_tracker(&self, _mailbox: &str, num_messages: u32) -> Arc<MailboxTracker> {
        MailboxTracker::new(num_messages)
    }
}

/// User session — one authenticated IMAP connection.
#[async_trait]
pub trait UserSession: Send + 'static {
    // ── Authenticated state ──────────────────────────────────────────

    /// List mailboxes matching the given pattern.
    async fn list(&mut self, reference: &str, pattern: &str) -> Result<Vec<ListData>, ImapError>;

    /// Subscribe to a mailbox.
    async fn subscribe(&mut self, mailbox: &str) -> Result<(), ImapError>;

    /// Unsubscribe from a mailbox.
    async fn unsubscribe(&mut self, mailbox: &str) -> Result<(), ImapError>;

    /// Create a mailbox.
    async fn create(&mut self, mailbox: &str) -> Result<(), ImapError>;

    /// Delete a mailbox.
    async fn delete(&mut self, mailbox: &str) -> Result<(), ImapError>;

    /// Rename a mailbox.
    async fn rename(&mut self, from: &str, to: &str) -> Result<(), ImapError>;

    /// Get mailbox status.
    async fn status(&mut self, mailbox: &str) -> Result<StatusData, ImapError>;

    /// Select a mailbox for access.
    async fn select(&mut self, mailbox: &str, read_only: bool) -> Result<SelectData, ImapError>;

    /// Close the currently selected mailbox (no expunge).
    async fn close(&mut self) -> Result<(), ImapError>;

    // ── Selected state ───────────────────────────────────────────────

    /// Fetch messages matching the given sequence set.
    async fn fetch(
        &mut self,
        uid: bool,
        seq_set: &SeqSet,
        options: &FetchOptions,
    ) -> Result<Vec<FetchedMessage>, ImapError>;

    /// Store flags on messages.
    async fn store(
        &mut self,
        uid: bool,
        seq_set: &SeqSet,
        flags: &StoreFlags,
    ) -> Result<Vec<StoredMessage>, ImapError>;

    /// Search messages matching criteria.
    async fn search(
        &mut self,
        uid: bool,
        criteria: &SearchCriteria,
    ) -> Result<Vec<u32>, ImapError>;

    /// Copy messages to another mailbox.
    async fn copy(
        &mut self,
        uid: bool,
        seq_set: &SeqSet,
        dest: &str,
    ) -> Result<CopyData, ImapError>;

    /// Expunge messages with \Deleted flag set.
    async fn expunge(&mut self, uid_set: Option<&UIDSet>) -> Result<Vec<u32>, ImapError>;

    /// Append a message to a mailbox.
    async fn append(
        &mut self,
        mailbox: &str,
        data: Vec<u8>,
        flags: Option<Vec<Flag>>,
        date: Option<String>,
    ) -> Result<AppendData, ImapError>;

    // ── Extensions with defaults ──────────────────────────────────────

    /// MOVE command (RFC 6851). Default: COPY + expunge source.
    async fn move_messages(
        &mut self,
        uid: bool,
        seq_set: &SeqSet,
        dest: &str,
    ) -> Result<CopyData, ImapError> {
        let data = self.copy(uid, seq_set, dest).await?;
        // Build a UID set from the source UIDs and expunge
        if !data.source_uids.is_empty() {
            let source_set = seq_set_from_uids(&data.source_uids);
            self.expunge(Some(&source_set)).await?;
        }
        Ok(data)
    }

    /// IDLE command (RFC 2177). Default: wait on stop signal.
    async fn idle(
        &mut self,
        stop: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<(), ImapError> {
        let _ = stop.await;
        Ok(())
    }

    /// NAMESPACE command (RFC 2342). Default: empty namespaces.
    async fn namespace(&mut self) -> Result<NamespaceData, ImapError> {
        Ok(NamespaceData {
            personal: vec![],
            other: vec![],
            shared: vec![],
        })
    }

    /// Poll for changes (called before tagged OK response).
    async fn poll(&mut self) -> Result<(), ImapError> {
        Ok(())
    }

    /// Current message count of the SELECTED mailbox, for IDLE `EXISTS` polling.
    ///
    /// The IDLE loop calls this each tick; when the returned count grows it emits
    /// an untagged `* N EXISTS` (RFC 2177) so clients (e.g. DeltaChat) learn about
    /// new mail without reconnecting. Default `None` disables IDLE push — suitable
    /// for demo backends that don't track a live count.
    async fn current_message_count(&mut self) -> Option<u32> {
        None
    }
}

/// Build a SeqSet from a list of UIDs.
fn seq_set_from_uids(uids: &[u32]) -> SeqSet {
    if uids.is_empty() {
        return SeqSet::default();
    }
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    let mut ranges = Vec::new();
    let mut start = sorted[0];
    let mut end = sorted[0];
    for &u in sorted[1..].iter() {
        if u == end + 1 {
            end = u;
        } else {
            ranges.push((start, end));
            start = u;
            end = u;
        }
    }
    ranges.push((start, end));
    SeqSet(ranges)
}

impl fmt::Display for ConnInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.peer_addr {
            Some(addr) => write!(f, "{addr}"),
            None => write!(f, "(unknown)"),
        }
    }
}