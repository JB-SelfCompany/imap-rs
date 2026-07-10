use imap_core::error::{ImapError, ImapResult};
use imap_core::types::SeqSet;

use crate::conn::Conn;

/// Handle COPY and UID COPY commands.
/// Syntax: COPY seq-set mailbox / UID COPY uid-set mailbox
pub async fn handle_copy(conn: &mut Conn, tag: &str, uid: bool) -> ImapResult<()> {
    conn.require_selected()?;

    let seq_str = conn.decoder.read_num_set_str().await?;
    let seq_set = SeqSet::parse(&seq_str).map_err(|e| ImapError::bad(e))?;
    conn.decoder.expect_sp().await?;
    let dest = conn.decoder.read_astring().await?;
    conn.decoder.expect_crlf().await?;

    let session = conn.session.as_mut().ok_or_else(|| ImapError::bad("No session"))?;
    let data = session.copy(uid, &seq_set, &dest).await?;

    // [COPYUID] response code
    if !data.source_uids.is_empty() && !data.dest_uids.is_empty() {
        let src_str: Vec<String> = data.source_uids.iter().map(|u| u.to_string()).collect();
        let dest_str: Vec<String> = data.dest_uids.iter().map(|u| u.to_string()).collect();
        conn.encoder
            .write_status(
                tag,
                "OK",
                Some(&format!(
                    "COPYUID {} ({}) ({})",
                    data.uid_validity,
                    src_str.join(" "),
                    dest_str.join(" ")
                )),
                "COPY completed",
            )
            .await
            .map_err(|e| ImapError::Internal(Box::new(e)))?;
    } else {
        conn.write_ok(tag, "COPY completed").await;
    }

    Ok(())
}

/// Handle MOVE and UID MOVE commands.
/// Syntax: MOVE seq-set mailbox / UID MOVE uid-set mailbox
pub async fn handle_move(conn: &mut Conn, tag: &str, uid: bool) -> ImapResult<()> {
    conn.require_selected()?;

    let seq_str = conn.decoder.read_num_set_str().await?;
    let seq_set = SeqSet::parse(&seq_str).map_err(|e| ImapError::bad(e))?;
    conn.decoder.expect_sp().await?;
    let dest = conn.decoder.read_astring().await?;
    conn.decoder.expect_crlf().await?;

    let session = conn.session.as_mut().ok_or_else(|| ImapError::bad("No session"))?;
    let data = session.move_messages(uid, &seq_set, &dest).await?;

    if !data.source_uids.is_empty() && !data.dest_uids.is_empty() {
        let src_str: Vec<String> = data.source_uids.iter().map(|u| u.to_string()).collect();
        let dest_str: Vec<String> = data.dest_uids.iter().map(|u| u.to_string()).collect();
        conn.encoder
            .write_status(
                tag,
                "OK",
                Some(&format!(
                    "COPYUID {} ({}) ({})",
                    data.uid_validity,
                    src_str.join(" "),
                    dest_str.join(" ")
                )),
                "MOVE completed",
            )
            .await
            .map_err(|e| ImapError::Internal(Box::new(e)))?;
    } else {
        conn.write_ok(tag, "MOVE completed").await;
    }

    Ok(())
}