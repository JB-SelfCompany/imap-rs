use imap_core::error::{ImapError, ImapResult};
use imap_core::search::{SearchCriteria, SearchOptions};
use imap_core::types::SeqSet;

use crate::conn::Conn;

/// Handle SEARCH and UID SEARCH commands.
pub async fn handle_search(conn: &mut Conn, tag: &str, uid: bool) -> ImapResult<()> {
    conn.require_selected()?;

    // Parse optional RETURN options
    let mut options = SearchOptions::default();
    let mut extended = false;

    // Check for RETURN (...) before the search criteria
    let b = conn.decoder.read_byte().await?;
    if b == b' ' {
        // Could be RETURN or start of criteria
        let next = conn.decoder.read_byte().await?;
        conn.decoder.unread_byte(next);
        if next == b'R' || next == b'r' {
            // Try to read RETURN
            let atom = conn.decoder.read_atom().await?;
            if atom.eq_ignore_ascii_case("RETURN") {
                conn.decoder.expect_sp().await?;
                parse_search_return_opts(conn, &mut options).await?;
                conn.decoder.expect_sp().await?;
                extended = true;
            } else {
                // Not RETURN, unread and treat as criteria
                conn.decoder.unread_byte(b);
                for ch in atom.as_bytes().iter().rev() {
                    conn.decoder.unread_byte(*ch);
                }
            }
        } else {
            conn.decoder.unread_byte(b);
        }
    } else {
        conn.decoder.unread_byte(b);
    }

    let mut criteria = SearchCriteria::default();
    parse_search_criteria(conn, &mut criteria).await?;
    conn.decoder.expect_crlf().await?;

    let session = conn
        .session
        .as_mut()
        .ok_or_else(|| ImapError::bad("No session"))?;
    let ids = session.search(uid, &criteria).await?;

    // If no return option specified, default to ALL
    if !options.return_min && !options.return_max && !options.return_all && !options.return_count {
        options.return_all = true;
    }

    // Check if we should use ESEARCH format
    let use_esearch = extended || conn.enabled.contains(&imap_core::types::Cap::esearch());

    if use_esearch {
        write_esearch(conn, tag, &ids, &options, uid).await?;
    } else {
        // Classic SEARCH response
        conn.encoder.atom("*").await.sp().await.atom("SEARCH").await;
        for id in &ids {
            conn.encoder.sp().await.number(*id).await;
        }
        conn.encoder
            .crlf()
            .await
            .map_err(|e| ImapError::Internal(Box::new(e)))?;
    }

    conn.write_ok(tag, "SEARCH completed").await;
    Ok(())
}

/// Write ESEARCH response.
async fn write_esearch(
    conn: &mut Conn,
    tag: &str,
    ids: &[u32],
    options: &SearchOptions,
    uid: bool,
) -> Result<(), ImapError> {
    conn.encoder.atom("*").await.sp().await.atom("ESEARCH").await;

    // Tag
    if !tag.is_empty() {
        conn.encoder
            .sp()
            .await
            .special(b'(')
            .await
            .atom("TAG")
            .await
            .sp()
            .await
            .string(tag)
            .await
            .special(b')')
            .await;
    }

    if uid {
        conn.encoder.sp().await.atom("UID").await;
    }

    // ALL
    if options.return_all && !ids.is_empty() {
        let set = build_seqset(ids);
        conn.encoder.sp().await.atom("ALL").await.sp().await.atom(&set.to_wire()).await;
    }

    // MIN
    if options.return_min {
        if let Some(&min) = ids.iter().min() {
            conn.encoder.sp().await.atom("MIN").await.sp().await.number(min).await;
        }
    }

    // MAX
    if options.return_max {
        if let Some(&max) = ids.iter().max() {
            conn.encoder.sp().await.atom("MAX").await.sp().await.number(max).await;
        }
    }

    // COUNT
    if options.return_count {
        conn.encoder.sp().await.atom("COUNT").await.sp().await.number(ids.len() as u32).await;
    }

    conn.encoder
        .crlf()
        .await
        .map_err(|e| ImapError::Internal(Box::new(e)))?;

    Ok(())
}

/// Build a SeqSet from a list of IDs.
fn build_seqset(ids: &[u32]) -> SeqSet {
    let mut set = SeqSet::default();
    for &id in ids {
        set.add_num(id);
    }
    set
}

/// Parse SEARCH RETURN options: (MIN MAX ALL COUNT SAVE)
async fn parse_search_return_opts(
    conn: &mut Conn,
    options: &mut SearchOptions,
) -> ImapResult<()> {
    // Read parenthesized list
    let b = conn.decoder.read_byte().await?;
    if b != b'(' {
        conn.decoder.unread_byte(b);
        return Ok(());
    }

    loop {
        let b = conn.decoder.read_byte().await?;
        if b == b')' {
            break;
        }
        if b == b' ' {
            continue;
        }
        conn.decoder.unread_byte(b);
        let atom = conn.decoder.read_atom().await?;
        match atom.to_ascii_uppercase().as_str() {
            "MIN" => options.return_min = true,
            "MAX" => options.return_max = true,
            "ALL" => options.return_all = true,
            "COUNT" => options.return_count = true,
            "SAVE" => options.return_save = true,
            _ => {} // ignore unknown
        }
    }
    Ok(())
}

/// Parse search criteria from the decoder.
/// Non-recursive version using explicit stack for NOT/OR keywords.
async fn parse_search_criteria(
    conn: &mut Conn,
    criteria: &mut SearchCriteria,
) -> ImapResult<()> {
    loop {
        // Peek: if next is CRLF, break
        let b = conn.decoder.read_byte().await?;
        match b {
            b'\r' | b'\n' => {
                conn.decoder.unread_byte(b);
                break;
            }
            b' ' => continue,
            _ => conn.decoder.unread_byte(b),
        }

        let key = conn.decoder.read_atom().await?;
        let key_upper = key.to_ascii_uppercase();

        match key_upper.as_str() {
            "ALL" => criteria.all = true,
            "NEW" => criteria.new = Some(true),
            "OLD" => criteria.old = Some(true),
            "RECENT" => criteria.recent = Some(true),
            "SEEN" => criteria.seen = Some(true),
            "UNSEEN" => criteria.unseen = true,
            "ANSWERED" => criteria.answered = Some(true),
            "UNANSWERED" => criteria.unanswered = true,
            "DELETED" => criteria.deleted = Some(true),
            "UNDELETED" => criteria.deleted = Some(false),
            "DRAFT" => criteria.draft = Some(true),
            "UNDRAFT" => criteria.undraft = true,
            "FLAGGED" => criteria.flagged = Some(true),
            "UNFLAGGED" => criteria.unflagged = true,
            "FROM" | "TO" | "SUBJECT" | "BODY" | "TEXT" => {
                conn.decoder.expect_sp().await?;
                let val = conn.decoder.read_astring().await?;
                match key_upper.as_str() {
                    "FROM" => criteria.from = Some(val),
                    "TO" => criteria.to = Some(val),
                    "SUBJECT" => criteria.subject = Some(val),
                    "BODY" => criteria.body = Some(val),
                    "TEXT" => criteria.text = Some(val),
                    _ => unreachable!(),
                }
            }
            "SMALLER" => {
                conn.decoder.expect_sp().await?;
                criteria.smaller = Some(conn.decoder.read_number64().await? as u64);
            }
            "LARGER" => {
                conn.decoder.expect_sp().await?;
                criteria.larger = Some(conn.decoder.read_number64().await? as u64);
            }
            "BEFORE" | "SINCE" | "ON" => {
                conn.decoder.expect_sp().await?;
                let date_str = conn.decoder.read_astring().await?;
                match key_upper.as_str() {
                    "BEFORE" => criteria.before = Some(date_str),
                    "SINCE" => criteria.since = Some(date_str),
                    _ => {}
                }
            }
            "UID" | "SEQNO" => {
                conn.decoder.expect_sp().await?;
                let seq_str = conn.decoder.read_num_set_str().await?;
                match key_upper.as_str() {
                    "UID" => criteria.uid_set = Some(seq_str),
                    "SEQNO" => criteria.seq_set = Some(seq_str),
                    _ => unreachable!(),
                }
            }
            "NOT" => {
                let mut sub = SearchCriteria::default();
                // Box::pin the recursive call to avoid infinite future size
                Box::pin(parse_search_criteria(conn, &mut sub)).await?;
                criteria.not = Some(Box::new(sub));
            }
            "OR" => {
                let mut a = SearchCriteria::default();
                let mut b = SearchCriteria::default();
                Box::pin(parse_search_criteria(conn, &mut a)).await?;
                Box::pin(parse_search_criteria(conn, &mut b)).await?;
                criteria.or = Some((Box::new(a), Box::new(b)));
            }
            "KEYWORD" => {
                conn.decoder.expect_sp().await?;
                criteria.keyword = Some(conn.decoder.read_atom().await?);
            }
            "UNKEYWORD" => {
                conn.decoder.expect_sp().await?;
                criteria.unkeyword = Some(conn.decoder.read_atom().await?);
            }
            _ => {
                // Unknown key — ignore
            }
        }
    }

    Ok(())
}