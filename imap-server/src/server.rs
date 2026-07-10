use std::collections::BTreeSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, ToSocketAddrs};
use tracing::{error, info};

use imap_core::types::{Cap, CapSet};

use crate::backend::Backend;
use crate::conn::Conn;

/// Server configuration options.
pub struct Options {
    /// Supported capabilities. If empty, only IMAP4rev1 is advertised.
    pub caps: CapSet,
    /// TLS configuration for STARTTLS. If None, STARTTLS is disabled.
    pub tls_config: Option<Arc<rustls::ServerConfig>>,
    /// Allow authentication without TLS.
    pub insecure_auth: bool,
    /// Writer for debug output (raw protocol data). If None, no debug output.
    pub debug_writer: Option<Box<dyn io::Write + Send + Sync>>,
}

impl Default for Options {
    fn default() -> Self {
        let mut caps = BTreeSet::new();
        caps.insert(Cap::imap4rev1());
        Self {
            caps,
            tls_config: None,
            insecure_auth: false,
            debug_writer: None,
        }
    }
}

/// An IMAP server that accepts connections and spawns handler tasks.
pub struct Server {
    backend: Arc<dyn Backend>,
    options: Options,
    shutdown: Arc<AtomicBool>,
}

impl Server {
    /// Create a new IMAP server with the given backend and options.
    pub fn new(backend: impl Backend, options: Options) -> Self {
        Server {
            backend: Arc::new(backend),
            options,
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Create a server with default options.
    pub fn with_defaults(backend: impl Backend) -> Self {
        Self::new(backend, Options::default())
    }

    /// Gracefully shut down the server. Stops accepting new connections.
    pub fn close(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Check if the server is shutting down.
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// Listen on a TCP address and serve plain (non-TLS) connections.
    pub async fn listen<A: ToSocketAddrs>(&self, addr: A) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(addr).await?;
        info!("IMAP server listening on {}", listener.local_addr()?);

        loop {
            if self.is_shutdown() {
                info!("Server shutting down");
                break;
            }

            let (stream, peer) = listener.accept().await?;
            let backend = self.backend.clone();
            let peer_str = peer.to_string();

            let insecure_auth = self.options.insecure_auth;
            tokio::spawn(async move {
                let (reader, writer) = tokio::io::split(stream);
                let mut conn = Conn::new(reader, writer, backend, peer_str, insecure_auth);
                conn.serve().await;
            });
        }
        Ok(())
    }

    /// Listen on a TCP address and serve TLS connections (implicit TLS, port 993).
    pub async fn listen_tls<A: ToSocketAddrs>(
        &self,
        addr: A,
        tls_config: Arc<rustls::ServerConfig>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(addr).await?;
        info!("IMAP TLS server listening on {}", listener.local_addr()?);

        loop {
            if self.is_shutdown() {
                info!("Server shutting down");
                break;
            }
            let (stream, peer) = listener.accept().await?;
            let backend = self.backend.clone();
            let tls_config = tls_config.clone();
            let peer_str = peer.to_string();

            tokio::spawn(async move {
                let tls_acceptor = tokio_rustls::TlsAcceptor::from(tls_config);
                let tls_stream = match tls_acceptor.accept(stream).await {
                    Ok(s) => s,
                    Err(e) => {
                        error!("TLS handshake failed from {peer_str}: {e}");
                        return;
                    }
                };

                let (reader, writer) = tokio::io::split(tls_stream);
                // TLS connections always allow auth regardless of insecure_auth
                let mut conn = Conn::new(reader, writer, backend, peer_str, true);
                conn.serve().await;
            });
        }
        Ok(())
    }

    /// Serve a single pre-established connection (e.g., for testing).
    pub async fn serve_conn<R, W>(
        backend: Arc<dyn Backend>,
        reader: R,
        writer: W,
        peer_addr: String,
        insecure_auth: bool,
    ) where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let mut conn = Conn::new(reader, writer, backend, peer_addr, insecure_auth);
        conn.serve().await;
    }
}