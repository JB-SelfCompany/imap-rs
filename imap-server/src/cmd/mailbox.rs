use imap_core::error::{ImapError, ImapResult};

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

/// Handle CREATE command.
pub async fn handle_create(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_auth()?;

    let mailbox = conn.decoder.read_astring().await?;
    conn.decoder.expect_crlf().await?;

    let session = conn
        .session
        .as_mut()
        .ok_or_else(|| ImapError::bad("No session"))?;
    session.create(&mailbox).await?;

    conn.write_ok(tag, "CREATE completed").await;
    Ok(())
}

/// Handle DELETE command.
pub async fn handle_delete(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_auth()?;

    let mailbox = conn.decoder.read_astring().await?;
    conn.decoder.expect_crlf().await?;

    let session = conn
        .session
        .as_mut()
        .ok_or_else(|| ImapError::bad("No session"))?;
    session.delete(&mailbox).await?;

    conn.write_ok(tag, "DELETE completed").await;
    Ok(())
}

/// Handle RENAME command.
pub async fn handle_rename(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_auth()?;

    let from = conn.decoder.read_astring().await?;
    conn.decoder.expect_sp().await?;
    let to = conn.decoder.read_astring().await?;
    conn.decoder.expect_crlf().await?;

    let session = conn
        .session
        .as_mut()
        .ok_or_else(|| ImapError::bad("No session"))?;
    session.rename(&from, &to).await?;

    conn.write_ok(tag, "RENAME completed").await;
    Ok(())
}

/// Handle SUBSCRIBE command.
pub async fn handle_subscribe(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_auth()?;

    let mailbox = conn.decoder.read_astring().await?;
    conn.decoder.expect_crlf().await?;

    let session = conn
        .session
        .as_mut()
        .ok_or_else(|| ImapError::bad("No session"))?;
    session.subscribe(&mailbox).await?;

    conn.write_ok(tag, "SUBSCRIBE completed").await;
    Ok(())
}

/// Handle UNSUBSCRIBE command.
pub async fn handle_unsubscribe(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_auth()?;

    let mailbox = conn.decoder.read_astring().await?;
    conn.decoder.expect_crlf().await?;

    let session = conn
        .session
        .as_mut()
        .ok_or_else(|| ImapError::bad("No session"))?;
    session.unsubscribe(&mailbox).await?;

    conn.write_ok(tag, "UNSUBSCRIBE completed").await;
    Ok(())
}

/// Handle LIST command.
pub async fn handle_list(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_auth()?;

    let reference = conn.decoder.read_astring().await?;
    conn.decoder.expect_sp().await?;
    let pattern = conn.decoder.read_list_mailbox().await?;
    conn.decoder.expect_crlf().await?;

    let session = conn
        .session
        .as_mut()
        .ok_or_else(|| ImapError::bad("No session"))?;
    let mailboxes = session.list(&reference, &pattern).await?;

    for mb in &mailboxes {
        let flag_strs: Vec<String> = mb.attrs.iter().map(|a| a.to_string()).collect();
        conn.encoder
            .atom("*")
            .await
            .sp()
            .await
            .atom("LIST")
            .await
            .sp()
            .await;
        write_flag_list(conn, &flag_strs).await;
        conn.encoder.sp().await;

        // Delimiter
        if mb.delimiter.is_empty() {
            conn.encoder.nil().await;
        } else {
            conn.encoder.quoted(&mb.delimiter).await;
        }

        conn.encoder.sp().await;

        // Name
        if mb.name.eq_ignore_ascii_case("INBOX") {
            conn.encoder.atom("INBOX").await;
        } else {
            conn.encoder.quoted(&mb.name).await;
        }

        conn.encoder
            .crlf()
            .await
            .map_err(|e| ImapError::Internal(Box::new(e)))?;
    }

    conn.write_ok(tag, "LIST completed").await;
    Ok(())
}

/// Handle LSUB command.
pub async fn handle_lsub(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_auth()?;

    let reference = conn.decoder.read_astring().await?;
    conn.decoder.expect_sp().await?;
    let pattern = conn.decoder.read_list_mailbox().await?;
    conn.decoder.expect_crlf().await?;

    let session = conn
        .session
        .as_mut()
        .ok_or_else(|| ImapError::bad("No session"))?;
    let mailboxes = session.list(&reference, &pattern).await?;

    for mb in &mailboxes {
        let flag_strs: Vec<String> = mb.attrs.iter().map(|a| a.to_string()).collect();
        conn.encoder
            .atom("*")
            .await
            .sp()
            .await
            .atom("LSUB")
            .await
            .sp()
            .await;
        write_flag_list(conn, &flag_strs).await;
        conn.encoder.sp().await;

        if mb.delimiter.is_empty() {
            conn.encoder.nil().await;
        } else {
            conn.encoder.quoted(&mb.delimiter).await;
        }

        conn.encoder.sp().await;

        if mb.name.eq_ignore_ascii_case("INBOX") {
            conn.encoder.atom("INBOX").await;
        } else {
            conn.encoder.quoted(&mb.name).await;
        }

        conn.encoder
            .crlf()
            .await
            .map_err(|e| ImapError::Internal(Box::new(e)))?;
    }

    conn.write_ok(tag, "LSUB completed").await;
    Ok(())
}

/// Handle STATUS command.
/// Syntax: STATUS mailbox (items)
pub async fn handle_status(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_auth()?;

    let mailbox = conn.decoder.read_astring().await?;
    conn.decoder.expect_sp().await?;

    // Consume status items (parenthesized list)
    let _items = read_status_items(conn).await?;
    conn.decoder.expect_crlf().await?;

    let session = conn
        .session
        .as_mut()
        .ok_or_else(|| ImapError::bad("No session"))?;
    let data = session.status(&mailbox).await?;

    // Write STATUS response
    conn.encoder
        .atom("*")
        .await
        .sp()
        .await
        .atom("STATUS")
        .await
        .sp()
        .await;

    if mailbox.eq_ignore_ascii_case("INBOX") {
        conn.encoder.atom("INBOX").await;
    } else {
        conn.encoder.quoted(&mailbox).await;
    }

    conn.encoder.sp().await.special(b'(').await;

    let mut has_item = false;
    if let Some(n) = data.messages {
        if has_item {
            conn.encoder.sp().await;
        }
        conn.encoder.atom("MESSAGES").await.sp().await.number(n).await;
        has_item = true;
    }
    if let Some(n) = data.recent {
        if has_item {
            conn.encoder.sp().await;
        }
        conn.encoder.atom("RECENT").await.sp().await.number(n).await;
        has_item = true;
    }
    if let Some(n) = data.uid_next {
        if has_item {
            conn.encoder.sp().await;
        }
        conn.encoder.atom("UIDNEXT").await.sp().await.number(n).await;
        has_item = true;
    }
    if let Some(n) = data.uid_validity {
        if has_item {
            conn.encoder.sp().await;
        }
        conn.encoder
            .atom("UIDVALIDITY")
            .await
            .sp()
            .await
            .number(n)
            .await;
        has_item = true;
    }
    if let Some(n) = data.unseen {
        if has_item {
            conn.encoder.sp().await;
        }
        conn.encoder.atom("UNSEEN").await.sp().await.number(n).await;
    }

    conn.encoder
        .special(b')')
        .await
        .crlf()
        .await
        .map_err(|e| ImapError::Internal(Box::new(e)))?;

    conn.write_ok(tag, "STATUS completed").await;
    Ok(())
}

/// Read status items list (parenthesized). Just consumes them.
async fn read_status_items(conn: &mut Conn) -> Result<Vec<String>, ImapError> {
    let mut items = Vec::new();
    let b = conn.decoder.read_byte().await?;
    if b != b'(' {
        conn.decoder.unread_byte(b);
        return Ok(items);
    }
    loop {
        let b = conn.decoder.read_byte().await?;
        match b {
            b')' => break,
            b' ' => continue,
            _ => {
                conn.decoder.unread_byte(b);
                let item = conn.decoder.read_atom().await?;
                items.push(item.to_ascii_uppercase());
            }
        }
    }
    Ok(items)
}