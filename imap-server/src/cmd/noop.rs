use imap_core::error::{ImapError, ImapResult};
use imap_core::types::{Cap, ConnState};

use crate::conn::Conn;

/// Handle NOOP command.
/// Syntax: NOOP
pub async fn handle_noop(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.decoder.expect_crlf().await?;

    if let Some(ref mut session) = conn.session {
        let _ = session.poll().await;
    }

    conn.write_ok(tag, "NOOP completed").await;
    Ok(())
}

/// Handle CHECK command — also polls for changes like NOOP.
pub async fn handle_check(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_selected()?;
    conn.decoder.expect_crlf().await?;

    if let Some(ref mut session) = conn.session {
        let _ = session.poll().await;
    }

    conn.write_ok(tag, "CHECK completed").await;
    Ok(())
}

/// Handle ENABLE command (RFC 5161).
/// Syntax: ENABLE capability [capability ...]
pub async fn handle_enable(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    match conn.state {
        ConnState::Authenticated | ConnState::Selected => {}
        _ => return Err(ImapError::bad("Not authenticated")),
    }

    let server_caps = conn.available_caps();
    let mut enabled: Vec<String> = Vec::new();

    loop {
        conn.decoder.expect_sp().await?;
        let cap = conn.decoder.read_atom().await?;
        let cap_upper = cap.to_ascii_uppercase();

        // Only enable caps the server actually advertises
        if server_caps.contains(&Cap(cap_upper.clone())) {
            enabled.push(cap_upper);
        }

        let b = conn.decoder.read_byte().await?;
        if b == b'\r' || b == b'\n' {
            conn.decoder.unread_byte(b);
            break;
        }
        conn.decoder.unread_byte(b);
    }
    conn.decoder.expect_crlf().await?;

    for c in &enabled {
        conn.enabled.insert(Cap(c.clone()));
    }

    if !enabled.is_empty() {
        conn.encoder.atom("*").await.sp().await.atom("ENABLED").await;
        for c in &enabled {
            conn.encoder.sp().await.atom(c).await;
        }
        conn.encoder
            .crlf()
            .await
            .map_err(|e| ImapError::Internal(Box::new(e)))?;
    }

    conn.write_ok(tag, "ENABLE completed").await;
    Ok(())
}

/// Handle NAMESPACE command (RFC 2342).
/// Syntax: NAMESPACE
pub async fn handle_namespace(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_auth()?;

    conn.decoder.expect_crlf().await?;

    let session = conn
        .session
        .as_mut()
        .ok_or_else(|| ImapError::bad("No session"))?;
    let data = session.namespace().await?;

    conn.encoder
        .atom("*")
        .await
        .sp()
        .await
        .atom("NAMESPACE")
        .await
        .sp()
        .await;

    // Personal namespaces
    write_namespace_list(conn, &data.personal).await;
    conn.encoder.sp().await;

    // Other users' namespaces
    write_namespace_list(conn, &data.other).await;
    conn.encoder.sp().await;

    // Shared namespaces
    write_namespace_list(conn, &data.shared).await;

    conn.encoder
        .crlf()
        .await
        .map_err(|e| ImapError::Internal(Box::new(e)))?;

    conn.write_ok(tag, "NAMESPACE completed").await;
    Ok(())
}

async fn write_namespace_list(
    conn: &mut Conn,
    namespaces: &[imap_core::select::NamespaceDescriptor],
) {
    if namespaces.is_empty() {
        conn.encoder.nil().await;
        return;
    }

    conn.encoder.special(b'(').await;
    for (i, ns) in namespaces.iter().enumerate() {
        if i > 0 {
            conn.encoder.sp().await;
        }
        conn.encoder
            .special(b'(')
            .await
            .quoted(&ns.prefix)
            .await
            .sp()
            .await
            .quoted(&ns.delimiter)
            .await
            .special(b')')
            .await;
    }
    conn.encoder.special(b')').await;
}