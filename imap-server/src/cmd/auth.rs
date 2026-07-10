use tracing::info;

use imap_core::error::{ImapError, ImapResult};
use imap_core::response::ResponseCode;
use imap_core::types::ConnState;

use crate::backend::ConnInfo;
use crate::conn::Conn;

/// Handle CAPABILITY command.
/// Syntax: CAPABILITY
/// Response: * CAPABILITY cap1 cap2 ... (NOT * OK [CAPABILITY ...])
pub async fn handle_capability(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    let caps: Vec<String> = conn.available_caps().iter().map(|c| c.to_string()).collect();
    // RFC 3501 §6.1.1: untagged response is * CAPABILITY, not * OK [CAPABILITY ...]
    // The * OK [CAPABILITY ...] format is only for greeting and LOGIN response codes.
    conn.encoder.atom("*").await.sp().await.atom("CAPABILITY").await;
    for c in &caps {
        conn.encoder.sp().await.atom(c).await;
    }
    conn.encoder.crlf().await.map_err(|e| ImapError::Internal(Box::new(e)))?;
    conn.write_ok(tag, "CAPABILITY completed").await;
    Ok(())
}

/// Handle ID command (RFC 2971).
/// Syntax: ID ("key" "value" ...)|NIL
/// Response: * ID ("name" "yggmail") + tagged OK
pub async fn handle_id(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    // Manually consume client ID params: parenthesized list or NIL
    let b = conn.decoder.read_byte().await?;
    if b == b'(' {
        // Consume everything inside parens byte-by-byte
        loop {
            let rb = conn.decoder.read_byte().await?;
            if rb == b')' { break; }
        }
    } else {
        // NIL or other atom — already read first byte, consume rest
        conn.decoder.unread_byte(b);
        let _ = conn.decoder.read_atom().await?;
    }
    conn.decoder.expect_crlf().await?;

    // Respond with server ID
    conn.encoder
        .atom("*").await.sp().await.atom("ID").await.sp().await
        .special(b'(').await
        .quoted("name").await.sp().await.quoted("yggmail").await
        .special(b')').await
        .crlf().await
        .map_err(|e| ImapError::Internal(Box::new(e)))?;

    conn.write_ok(tag, "ID completed").await;
    Ok(())
}

/// Handle LOGIN command.
/// Syntax: LOGIN username password
pub async fn handle_login(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    if conn.state != ConnState::NotAuthenticated {
        return Err(ImapError::bad("Already authenticated"));
    }

    // Privacy check: require TLS unless insecure_auth is allowed
    if !conn.can_auth() {
        return Err(ImapError::no_code(
            ResponseCode::PRIVACYREQUIRED,
            "TLS is required to authenticate",
        ));
    }

    let username = conn.decoder.read_astring().await?;
    conn.decoder.expect_sp().await?;
    let password = conn.decoder.read_astring().await?;
    conn.decoder.expect_crlf().await?;

    let cinfo = ConnInfo {
        peer_addr: None,
    };

    match conn.backend.login(&cinfo, &username, &password).await {
        Ok(session) => {
            conn.session = Some(session);
            conn.state = ConnState::Authenticated;
            info!("User {username} authenticated from {}", conn.peer_addr);
            // Write OK with CAPABILITY as per Go implementation
            let caps: Vec<String> = conn.available_caps().iter().map(|c| c.to_string()).collect();
            conn.encoder
                .write_capability_status(tag, "OK", &caps, "Logged in")
                .await
                .map_err(|e| ImapError::Internal(Box::new(e)))?;
            Ok(())
        }
        Err(ImapError::No { .. }) => {
            Err(ImapError::no_code(
                ResponseCode::AUTHENTICATIONFAILED,
                "Authentication failed",
            ))
        }
        Err(e) => Err(e),
    }
}

/// Handle LOGOUT command.
/// Syntax: LOGOUT
pub async fn handle_logout(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.write_bye("Logging out").await;
    conn.write_ok(tag, "LOGOUT completed").await;
    Ok(())
}

/// Handle STARTTLS command.
/// Syntax: STARTTLS
pub async fn handle_starttls(conn: &mut Conn, _tag: &str) -> ImapResult<()> {
    if conn.state != ConnState::NotAuthenticated {
        return Err(ImapError::bad("STARTTLS not allowed in this state"));
    }

    let _tls_config = conn.backend.tls_config().ok_or_else(|| {
        ImapError::bad("STARTTLS not available")
    })?;

    conn.write_continuation("Ready to start TLS")
        .await
        .map_err(|e| ImapError::Internal(Box::new(e)))?;

    // TODO: Implement STARTTLS stream upgrade (requires extracting raw stream from type-erased Box)
    Err(ImapError::bad("STARTTLS not implemented in this version"))
}

/// Handle AUTHENTICATE command (RFC 4959 SASL-IR for PLAIN).
pub async fn handle_authenticate(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    if conn.state != ConnState::NotAuthenticated {
        return Err(ImapError::bad("Already authenticated"));
    }

    if !conn.can_auth() {
        return Err(ImapError::no_code(
            ResponseCode::PRIVACYREQUIRED,
            "TLS is required to authenticate",
        ));
    }

    let mechanism = conn.decoder.read_atom().await?;

    if !mechanism.eq_ignore_ascii_case("PLAIN") {
        // Consume rest of line before rejecting
        let _ = conn.decoder.read_text().await;
        let _ = conn.decoder.expect_crlf().await;
        return Err(ImapError::no(format!("SASL mechanism {mechanism} not supported")));
    }

    // RFC 4959 SASL-IR: client may send initial response on same line
    // AUTHENTICATE PLAIN <base64>\r\n  (SASL-IR)
    // AUTHENTICATE PLAIN\r\n           (no IR, needs continuation)
    let line = match conn.decoder.peek_byte().await {
        Ok(b'\r' | b'\n') | Err(_) => {
            // No inline response — send continuation prompt
            conn.decoder.expect_crlf().await?;
            conn.write_continuation("")
                .await
                .map_err(|e| ImapError::Internal(Box::new(e)))?;
            let l = conn.decoder.read_text().await?;
            conn.decoder.expect_crlf().await?;
            l
        }
        Ok(_) => {
            // SASL-IR: initial response follows on same line after space
            let _ = conn.decoder.read_byte().await?; // consume SP
            let l = conn.decoder.read_text().await?;
            conn.decoder.expect_crlf().await?;
            l
        }
    };

    // Handle cancellation
    if line == "*" {
        return Err(ImapError::bad("AUTHENTICATE cancelled"));
    }

    // Decode base64: format is \x00username\x00password (or identity\x00username\x00password)
    let decoded = base64_decode(&line).map_err(|_| ImapError::bad("Malformed SASL response"))?;
    let parts: Vec<&[u8]> = decoded.split(|&b| b == 0).collect();
    if parts.len() < 3 {
        return Err(ImapError::bad("Malformed SASL PLAIN response"));
    }

    let identity = std::str::from_utf8(parts[0]).unwrap_or("");
    let username = std::str::from_utf8(parts[1]).map_err(|_| ImapError::bad("Invalid UTF-8 in username"))?;
    let password = std::str::from_utf8(parts[2]).map_err(|_| ImapError::bad("Invalid UTF-8 in password"))?;

    // If identity is provided, it must match username
    if !identity.is_empty() && identity != username {
        return Err(ImapError::no_code(
            ResponseCode::AUTHORIZATIONFAILED,
            "SASL identity not supported",
        ));
    }

    let cinfo = ConnInfo { peer_addr: None };
    match conn.backend.login(&cinfo, username, password).await {
        Ok(session) => {
            conn.session = Some(session);
            conn.state = ConnState::Authenticated;
            info!("User {username} authenticated via AUTHENTICATE from {}", conn.peer_addr);
            let caps: Vec<String> = conn.available_caps().iter().map(|c| c.to_string()).collect();
            conn.encoder
                .write_capability_status(tag, "OK", &caps, &format!("{mechanism} authentication successful"))
                .await
                .map_err(|e| ImapError::Internal(Box::new(e)))?;
            Ok(())
        }
        Err(ImapError::No { .. }) => {
            Err(ImapError::no_code(
                ResponseCode::AUTHENTICATIONFAILED,
                "Authentication failed",
            ))
        }
        Err(e) => Err(e),
    }
}

/// Handle UNAUTHENTICATE command (RFC 8437).
pub async fn handle_unauthenticate(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    if conn.state != ConnState::Authenticated {
        return Err(ImapError::bad("Not authenticated"));
    }

    // Check if backend supports unauthenticate
    // For now, just clear the session and reset state
    conn.session = None;
    conn.state = ConnState::NotAuthenticated;
    conn.enabled.clear();

    conn.write_ok(tag, "UNAUTHENTICATE completed").await;
    Ok(())
}

/// Minimal base64 decode (standard alphabet, no padding required).
fn base64_decode(s: &str) -> Result<Vec<u8>, ()> {
    const TABLE: [i8; 256] = {
        let mut t = [-1i8; 256];
        let mut i = 0u8;
        while i < 26 { t[b'A' as usize + i as usize] = i as i8; i += 1; }
        i = 0;
        while i < 26 { t[b'a' as usize + i as usize] = (26 + i) as i8; i += 1; }
        i = 0;
        while i < 10 { t[b'0' as usize + i as usize] = (52 + i) as i8; i += 1; }
        t[b'+' as usize] = 62;
        t[b'/' as usize] = 63;
        t
    };

    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &b in bytes {
        if b == b'=' { break; }
        let val = TABLE[b as usize];
        if val < 0 { return Err(()); }
        buf = (buf << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

impl Conn {
    /// Check whether authentication is possible on this connection.
    /// Returns true if insecure_auth is enabled (plain TCP) or TLS is established.
    /// When STARTTLS is implemented, add: `|| self.is_tls`
    pub(crate) fn can_auth(&self) -> bool {
        self.insecure_auth
    }
}