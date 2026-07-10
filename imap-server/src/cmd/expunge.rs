use imap_core::error::{ImapError, ImapResult};
use imap_core::types::SeqSet;

use crate::conn::Conn;

/// Handle EXPUNGE command.
/// Syntax: EXPUNGE
pub async fn handle_expunge(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_selected()?;

    conn.decoder.expect_crlf().await?;

    let session = conn.session.as_mut().ok_or_else(|| ImapError::bad("No session"))?;
    let expunged = session.expunge(None).await?;

    // Write unsolicited EXPUNGE responses
    for seq in &expunged {
        conn.encoder
            .atom("*")
            .await
            .sp()
            .await
            .number(*seq)
            .await
            .sp()
            .await
            .atom("EXPUNGE")
            .await
            .crlf()
            .await
            .map_err(|e| ImapError::Internal(Box::new(e)))?;
    }

    conn.write_ok(tag, "EXPUNGE completed").await;
    Ok(())
}

/// Handle UID EXPUNGE command.
/// Syntax: UID EXPUNGE uid-set
pub async fn handle_uid_expunge(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_selected()?;

    let seq_str = conn.decoder.read_num_set_str().await?;
    let uid_set = SeqSet::parse(&seq_str).map_err(|e| ImapError::bad(e))?;
    conn.decoder.expect_crlf().await?;

    let session = conn.session.as_mut().ok_or_else(|| ImapError::bad("No session"))?;
    let expunged = session.expunge(Some(&uid_set)).await?;

    for seq in &expunged {
        conn.encoder
            .atom("*")
            .await
            .sp()
            .await
            .number(*seq)
            .await
            .sp()
            .await
            .atom("EXPUNGE")
            .await
            .crlf()
            .await
            .map_err(|e| ImapError::Internal(Box::new(e)))?;
    }

    conn.write_ok(tag, "UID EXPUNGE completed").await;
    Ok(())
}