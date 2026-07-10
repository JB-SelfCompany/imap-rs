use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{debug, warn};

use imap_core::codec::{AsyncDecoder, Encoder};
use imap_core::error::{ImapError, ImapResult};
use imap_core::types::{Cap, CapSet, ConnState};

use crate::backend::Backend;
use crate::cmd;

/// Timeout constants matching Go implementation.
pub const CMD_READ_TIMEOUT: Duration = Duration::from_secs(30);
pub const IDLE_READ_TIMEOUT: Duration = Duration::from_secs(35 * 60);
pub const RESP_WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Type-erased reader half.
type BoxedRead = Box<dyn AsyncRead + Send + Unpin>;
/// Type-erased writer half.
type BoxedWrite = Box<dyn AsyncWrite + Send + Unpin>;

/// An IMAP connection — reads commands, dispatches to handlers, writes responses.
pub struct Conn {
    pub(crate) decoder: AsyncDecoder<BoxedRead>,
    pub(crate) encoder: Encoder<BoxedWrite>,
    pub(crate) state: ConnState,
    pub(crate) backend: Arc<dyn Backend>,
    pub(crate) session: Option<Box<dyn crate::backend::UserSession>>,
    pub(crate) enabled: CapSet,
    pub(crate) peer_addr: String,
    pub(crate) insecure_auth: bool,
}

impl Conn {
    /// Create a new IMAP connection from split read/write halves.
    pub fn new<R, W>(reader: R, writer: W, backend: Arc<dyn Backend>, peer_addr: String, insecure_auth: bool) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        Conn {
            decoder: AsyncDecoder::new(Box::new(reader) as BoxedRead),
            encoder: Encoder::new(Box::new(writer) as BoxedWrite),
            state: ConnState::NotAuthenticated,
            backend,
            session: None,
            enabled: BTreeSet::new(),
            peer_addr,
            insecure_auth,
        }
    }

    /// Run the IMAP serve loop.
    pub async fn serve(&mut self) {
        // Send greeting
        if let Err(e) = self.write_greeting().await {
            warn!("failed to send greeting: {e}");
            return;
        }

        loop {
            if self.state == ConnState::Logout {
                break;
            }

            let cmd = match self.decoder.read_command().await {
                Ok(Some(cmd)) => cmd,
                Ok(None) => {
                    debug!("decoder returned None (EOF)");
                    break;
                }
                Err(ImapError::Closed) => {
                    debug!("connection closed by peer");
                    let _ = self.write_bye("Connection closed").await;
                    break;
                }
                Err(ImapError::Bad { text, .. }) => {
                    // Resync SILENTLY. Two cases reach here: (a) a handler
                    // bailed mid-parse — dispatch() already sent a tagged BAD
                    // for that command, and the leftover bytes are now failing
                    // read_command(); (b) the client sent a line with no valid
                    // tag/verb. In (a) we must NOT emit another response: an
                    // untagged `* BAD` here would double-respond and re-trip
                    // the client's reconnect path. In (b) there is no tag to
                    // attach a response to. So discard to the next CRLF and
                    // keep the session alive (go-imap resyncs the same way).
                    warn!("read error (BAD): {text}; discarding to CRLF, resyncing");
                    self.decoder.discard_line().await;
                    continue;
                }
                Err(e) => {
                    warn!("read error: {e}");
                    let _ = self
                        .write_status_bye("BAD", "Internal server error")
                        .await;
                    break;
                }
            };

            // Dispatch — returns Err(Closed) to signal connection teardown.
            // All other errors are handled inside dispatch (tagged NO/BAD responses).
            if let Err(ImapError::Closed) = cmd::dispatch(self, cmd).await {
                break;
            }
        }
    }

    /// Send the initial greeting.
    async fn write_greeting(&mut self) -> Result<(), std::io::Error> {
        let caps = self.available_caps();
        let caps_str: Vec<String> = caps.iter().map(|c| c.0.clone()).collect();
        self.encoder
            .write_capability_status("", "OK", &caps_str, "IMAP server ready")
            .await
    }

    /// Write a BYE response and transition to Logout state.
    pub(crate) async fn write_bye(&mut self, text: &str) {
        let _ = self.encoder.write_status("*", "BYE", None, text).await;
        self.state = ConnState::Logout;
    }

    /// Write a status response (tagged or untagged).
    pub(crate) async fn write_status(&mut self, tag: &str, typ: &str, code: Option<&str>, text: &str) {
        let _ = self.encoder.write_status(tag, typ, code, text).await;
    }

    /// Write a status response, then set state to Logout on error.
    async fn write_status_bye(&mut self, typ: &str, text: &str) {
        let _ = self.encoder.write_status("", typ, None, text).await;
        self.state = ConnState::Logout;
    }

    /// Write a tagged OK response.
    pub(crate) async fn write_ok(&mut self, tag: &str, text: &str) {
        let _ = self.encoder.write_status(tag, "OK", None, text).await;
    }

    /// Write a tagged NO response.
    pub(crate) async fn write_no(&mut self, tag: &str, text: &str) {
        let _ = self.encoder.write_status(tag, "NO", None, text).await;
    }

    /// Write a continuation request.
    pub(crate) async fn write_continuation(&mut self, text: &str) -> Result<(), std::io::Error> {
        self.encoder.write_continuation(text).await
    }

    /// Get the set of available capabilities (base + enabled extensions).
    pub(crate) fn available_caps(&self) -> CapSet {
        let mut caps: CapSet = BTreeSet::new();
        caps.insert(Cap::imap4rev1());
        caps.insert(Cap::literal_plus());
        caps.insert(Cap::idle());
        caps.insert(Cap::move_cap());
        caps.insert(Cap::namespace());
        caps.insert(Cap::uidplus());
        caps.insert(Cap::enable());
        caps.insert(Cap::unselect());
        caps.insert(Cap::children());
        caps.insert(Cap::sasl_ir());
        caps.insert(Cap::id());

        // STARTTLS only before authentication
        if self.backend.tls_config().is_some() && self.state == ConnState::NotAuthenticated {
            caps.insert(Cap::starttls());
        }

        // LOGINDISABLED when can't auth
        if !self.can_auth() && self.state == ConnState::NotAuthenticated {
            caps.insert(Cap::login_disabled());
        }

        // Enabled caps
        for c in &self.enabled {
            caps.insert(c.clone());
        }
        caps
    }


    /// Require Authenticated (or Selected) state.
    pub(crate) fn require_auth(&self) -> ImapResult<()> {
        match self.state {
            ConnState::Authenticated | ConnState::Selected => Ok(()),
            _ => Err(ImapError::bad("Not authenticated")),
        }
    }

    /// Require Selected state.
    pub(crate) fn require_selected(&self) -> ImapResult<()> {
        match self.state {
            ConnState::Selected => Ok(()),
            ConnState::Authenticated => Err(ImapError::bad("No mailbox selected")),
            _ => Err(ImapError::bad("Not authenticated")),
        }
    }
}

impl fmt::Debug for Conn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Conn")
            .field("state", &self.state)
            .field("peer_addr", &self.peer_addr)
            .field(
                "session",
                &if self.session.is_some() {
                    "Some"
                } else {
                    "None"
                },
            )
            .finish()
    }
}