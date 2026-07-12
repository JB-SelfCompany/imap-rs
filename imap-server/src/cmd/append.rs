use imap_core::error::{ImapError, ImapResult};
use imap_core::types::Flag;

use crate::conn::Conn;

/// Handle APPEND command.
/// Syntax: APPEND mailbox [flags] [date] literal
pub async fn handle_append(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_auth()?;

    // Read mailbox name
    let mailbox = conn.decoder.read_astring().await?;

    // Read optional flags
    conn.decoder.expect_sp().await?;
    let flags = if let Ok(b'(') = conn.decoder.peek_byte().await {
        Some(read_flag_list(conn).await?)
    } else {
        None
    };

    // Read optional date and/or literal
    // After possible flags, we may have SP then date-string or literal.
    // Just check for literal directly
    let date;
    let data;
    match conn.decoder.peek_byte().await {
        Ok(b'{') => {
            // Literal directly (no date)
            date = None;
            let (bytes, _) = conn.decoder.read_literal().await?;
            data = bytes;
        }
        Ok(b'"') => {
            // Date string
            date = Some(conn.decoder.read_string().await?);
            // SP then literal
            conn.decoder.expect_sp().await?;
            let (bytes, _) = conn.decoder.read_literal().await?;
            data = bytes;
        }
        Ok(b' ') => {
            // SP then date or literal
            let _ = conn.decoder.read_byte().await; // consume SP
            match conn.decoder.peek_byte().await {
                Ok(b'{') => {
                    date = None;
                    let (bytes, _) = conn.decoder.read_literal().await?;
                    data = bytes;
                }
                Ok(b'"') => {
                    date = Some(conn.decoder.read_string().await?);
                    conn.decoder.expect_sp().await?;
                    let (bytes, _) = conn.decoder.read_literal().await?;
                    data = bytes;
                }
                Ok(_) => {
                    let (bytes, _) = conn.decoder.read_literal().await?;
                    data = bytes;
                    date = None;
                }
                Err(e) => return Err(e),
            }
        }
        _ => {
            let (bytes, _) = conn.decoder.read_literal().await?;
            data = bytes;
            date = None;
        }
    }

    let session = conn
        .session
        .as_mut()
        .ok_or_else(|| ImapError::bad("No session"))?;
    let result = session.append(&mailbox, data, flags, date).await?;

    // Write APPENDUID response
    if let (Some(uid_val), Some(uid)) = (result.uid_validity, result.uid) {
        conn.encoder
            .write_status(
                tag,
                "OK",
                Some(&format!("APPENDUID {uid_val} {uid}")),
                "APPEND completed",
            )
            .await
            .map_err(|e| ImapError::Internal(Box::new(e)))?;
    } else {
        conn.write_ok(tag, "APPEND completed").await;
    }

    Ok(())
}

/// Read a parenthesized flag list.
async fn read_flag_list(conn: &mut Conn) -> Result<Vec<Flag>, ImapError> {
    let mut flags = Vec::new();

    let b = conn.decoder.read_byte().await?;
    if b != b'(' {
        conn.decoder.unread_byte(b);
        let flag = conn.decoder.read_flag().await?;
        flags.push(Flag(flag));
        return Ok(flags);
    }

    loop {
        let b = conn.decoder.read_byte().await?;
        match b {
            b')' => break,
            b' ' => continue,
            _ => {
                conn.decoder.unread_byte(b);
                let flag = conn.decoder.read_flag().await?;
                flags.push(Flag(flag));
            }
        }
    }

    // Consume trailing SP (if present)
    match conn.decoder.read_byte().await {
        Ok(b' ') => {
            // Check if next is a date or literal
            let next = conn.decoder.read_byte().await;
            match next {
                Ok(b @ (b'"' | b'{')) => {
                    conn.decoder.unread_byte(b);
                }
                Ok(b) => conn.decoder.unread_byte(b),
                Err(_) => {}
            }
        }
        Ok(b) => conn.decoder.unread_byte(b),
        Err(_) => {}
    }

    Ok(flags)
}