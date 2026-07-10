use imap_core::error::{ImapError, ImapResult};
use imap_core::store::{StoreFlags, StoreOp};
use imap_core::types::SeqSet;

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

/// Handle STORE and UID STORE commands.
pub async fn handle_store(conn: &mut Conn, tag: &str, uid: bool) -> ImapResult<()> {
    conn.require_selected()?;

    // Read sequence set
    let seq_str = conn.decoder.read_num_set_str().await?;
    let seq_set = SeqSet::parse(&seq_str).map_err(|e| ImapError::bad(e))?;
    conn.decoder.expect_sp().await?;

    // Read store operation name
    let op_name = conn.decoder.read_atom().await?;

    // Handle optional .SILENT suffix
    let (op, _full_op) = parse_store_op(conn, &op_name).await?;

    // Read flag list
    let flags = read_flag_list(conn).await?;

    conn.decoder.expect_crlf().await?;

    let store_flags = StoreFlags { op, flags };

    let session = conn
        .session
        .as_mut()
        .ok_or_else(|| ImapError::bad("No session"))?;
    let results = session.store(uid, &seq_set, &store_flags).await?;

    // Write unsolicited FETCH responses for non-silent stores
    if !op.is_silent() {
        for msg in results {
            let flag_strs: Vec<String> = msg.flags.iter().map(|f| f.0.clone()).collect();
            conn.encoder
                .atom("*")
                .await
                .sp()
                .await
                .number(msg.seq)
                .await
                .sp()
                .await
                .atom("FETCH")
                .await
                .sp()
                .await
                .special(b'(')
                .await
                .atom("FLAGS")
                .await
                .sp()
                .await;
            write_flag_list(conn, &flag_strs).await;
            conn.encoder
                .special(b')')
                .await
                .crlf()
                .await
                .map_err(|e| ImapError::Internal(Box::new(e)))?;
        }
    }

    conn.write_ok(tag, "STORE completed").await;
    Ok(())
}

/// Parse the store operation name, detecting .SILENT suffix.
async fn parse_store_op(conn: &mut Conn, name: &str) -> Result<(StoreOp, String), ImapError> {
    let upper = name.to_ascii_uppercase();
    let (base, _silent_suffix) = if upper == "FLAGS" || upper == "+FLAGS" || upper == "-FLAGS" {
        (upper, false)
    } else {
        return Err(ImapError::bad(format!("unknown STORE op: {name}")));
    };

    // Check if next char is '.'
    let b = conn.decoder.read_byte().await?;
    if b == b'.' {
        let suffix = conn.decoder.read_atom().await?;
        if suffix.eq_ignore_ascii_case("SILENT") {
            let full = format!("{base}.SILENT");
            let op = StoreOp::parse(&full).map_err(|e| ImapError::bad(e))?;
            // Expect SP after the op
            conn.decoder.expect_sp().await?;
            return Ok((op, full));
        }
        return Err(ImapError::bad("expected SILENT after '.'"));
    } else if b == b' ' {
        // SP follows — normal case
        let op = StoreOp::parse(&base).map_err(|e| ImapError::bad(e))?;
        return Ok((op, base));
    } else {
        // Unexpected character
        conn.decoder.unread_byte(b);
        let op = StoreOp::parse(&base).map_err(|e| ImapError::bad(e))?;
        return Ok((op, base));
    }
}

/// Read a flag list: `(\Seen \Answered)` or a single flag.
async fn read_flag_list(conn: &mut Conn) -> Result<Vec<String>, ImapError> {
    let mut flags = Vec::new();

    let b = conn.decoder.read_byte().await?;
    if b == b'(' {
        loop {
            let b = conn.decoder.read_byte().await?;
            match b {
                b')' => break,
                b' ' => continue,
                _ => {
                    conn.decoder.unread_byte(b);
                    let flag = conn.decoder.read_flag().await?;
                    flags.push(flag);
                }
            }
        }
    } else {
        conn.decoder.unread_byte(b);
        let flag = conn.decoder.read_flag().await?;
        flags.push(flag);
    }

    Ok(flags)
}