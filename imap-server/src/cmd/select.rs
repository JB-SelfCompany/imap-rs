use imap_core::error::{ImapError, ImapResult};
use imap_core::types::ConnState;

use crate::conn::Conn;

/// Write a parenthesized flag list.
async fn write_flag_list(conn: &mut Conn, flags: &[String]) {
    conn.encoder.special(b'(').await;
    for (i, f) in flags.iter().enumerate() {
        if i > 0 {
            conn.encoder.sp().await;
        }
        conn.encoder.flag(f).await;
    }
    conn.encoder.special(b')').await;
}

/// Handle SELECT and EXAMINE commands.
pub async fn handle_select(conn: &mut Conn, tag: &str, read_only: bool) -> ImapResult<()> {
    conn.require_auth()?;

    let mailbox = conn.decoder.read_astring().await?;
    conn.decoder.expect_crlf().await?;

    // Close previous mailbox if any
    if conn.state == ConnState::Selected {
        if let Some(ref mut session) = conn.session {
            session.close().await?;
        }
        conn.state = ConnState::Authenticated;
        let _ = conn
            .encoder
            .write_status("*", "OK", Some("CLOSED"), "Previous mailbox closed")
            .await;
    }

    let session = conn
        .session
        .as_mut()
        .ok_or_else(|| ImapError::bad("No session"))?;
    let data = session.select(&mailbox, read_only).await?;

    // * N EXISTS
    conn.encoder
        .atom("*")
        .await
        .sp()
        .await
        .number(data.exists)
        .await
        .sp()
        .await
        .atom("EXISTS")
        .await
        .crlf()
        .await
        .map_err(|e| ImapError::Internal(Box::new(e)))?;

    // * N RECENT
    conn.encoder
        .atom("*")
        .await
        .sp()
        .await
        .number(data.recent)
        .await
        .sp()
        .await
        .atom("RECENT")
        .await
        .crlf()
        .await
        .map_err(|e| ImapError::Internal(Box::new(e)))?;

    // * OK [UNSEEN N]
    if data.unseen > 0 {
        conn.encoder
            .atom("*")
            .await
            .sp()
            .await
            .atom("OK")
            .await
            .sp()
            .await
            .special(b'[')
            .await
            .atom("UNSEEN")
            .await
            .sp()
            .await
            .number(data.unseen)
            .await
            .special(b']')
            .await
            .sp()
            .await
            .text("Message(s) unseen")
            .await
            .crlf()
            .await
            .map_err(|e| ImapError::Internal(Box::new(e)))?;
    }

    // * OK [UIDVALIDITY N]
    conn.encoder
        .atom("*")
        .await
        .sp()
        .await
        .atom("OK")
        .await
        .sp()
        .await
        .special(b'[')
        .await
        .atom("UIDVALIDITY")
        .await
        .sp()
        .await
        .number(data.uid_validity)
        .await
        .special(b']')
        .await
        .sp()
        .await
        .text("UIDs valid")
        .await
        .crlf()
        .await
        .map_err(|e| ImapError::Internal(Box::new(e)))?;

    // * OK [UIDNEXT N]
    conn.encoder
        .atom("*")
        .await
        .sp()
        .await
        .atom("OK")
        .await
        .sp()
        .await
        .special(b'[')
        .await
        .atom("UIDNEXT")
        .await
        .sp()
        .await
        .number(data.uid_next)
        .await
        .special(b']')
        .await
        .sp()
        .await
        .text("Predicted next UID")
        .await
        .crlf()
        .await
        .map_err(|e| ImapError::Internal(Box::new(e)))?;

    // * FLAGS (list)
    let flag_strs: Vec<String> = data.flags.iter().map(|f| f.0.clone()).collect();
    conn.encoder.atom("*").await.sp().await.atom("FLAGS").await.sp().await;
    write_flag_list(conn, &flag_strs).await;
    conn.encoder
        .crlf()
        .await
        .map_err(|e| ImapError::Internal(Box::new(e)))?;

    // * OK [PERMANENTFLAGS (list)]
    let perm_strs: Vec<String> = data.permanent_flags.iter().map(|f| f.0.clone()).collect();
    conn.encoder
        .atom("*")
        .await
        .sp()
        .await
        .atom("OK")
        .await
        .sp()
        .await
        .special(b'[')
        .await
        .atom("PERMANENTFLAGS")
        .await
        .sp()
        .await;
    write_flag_list(conn, &perm_strs).await;
    conn.encoder
        .special(b']')
        .await
        .sp()
        .await
        .text("Permanent flags")
        .await
        .crlf()
        .await
        .map_err(|e| ImapError::Internal(Box::new(e)))?;

    conn.state = ConnState::Selected;

    let code = if read_only {
        "READ-ONLY"
    } else {
        "READ-WRITE"
    };
    let cmd_name = if read_only { "EXAMINE" } else { "SELECT" };
    conn.encoder
        .write_status(
            tag,
            "OK",
            Some(code),
            &format!("{cmd_name} completed"),
        )
        .await
        .map_err(|e| ImapError::Internal(Box::new(e)))?;

    Ok(())
}

/// Handle CLOSE command — auto-expunges \Deleted messages.
pub async fn handle_close(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_selected()?;

    if let Some(ref mut session) = conn.session {
        // CLOSE auto-expunges messages with \Deleted flag
        let _ = session.expunge(None).await;
        session.close().await?;
    }
    conn.state = ConnState::Authenticated;
    // CLOSE/UNSELECT don't send a tagged response per RFC 3501/9051
    // But many implementations do. We send OK for compatibility.
    conn.write_ok(tag, "CLOSE completed").await;
    Ok(())
}

/// Handle UNSELECT command — does NOT auto-expunge.
pub async fn handle_unselect(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_selected()?;

    if let Some(ref mut session) = conn.session {
        session.close().await?;
    }
    conn.state = ConnState::Authenticated;
    conn.write_ok(tag, "UNSELECT completed").await;
    Ok(())
}