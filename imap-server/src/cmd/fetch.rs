use imap_core::error::{ImapError, ImapResult};
use imap_core::fetch::{
    parse_fetch_att, BodySection, FetchAtt, FetchOptions, SectionSpecifier,
};
use imap_core::types::SeqSet;

use crate::conn::Conn;

/// Helper: write a parenthesized flag list manually (avoids closure capture issues).
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

/// Handle FETCH and UID FETCH commands.
/// Syntax: FETCH seq-set attrs / UID FETCH uid-set attrs
pub async fn handle_fetch(conn: &mut Conn, tag: &str, uid: bool) -> ImapResult<()> {
    conn.require_selected()?;

    // Read sequence set
    let seq_str = conn.decoder.read_num_set_str().await?;
    let seq_set = SeqSet::parse(&seq_str).map_err(|e| ImapError::bad(e))?;
    conn.decoder.expect_sp().await?;

    // Read fetch attributes
    let options = parse_fetch_args(conn).await?;

    conn.decoder.expect_crlf().await?;

    let session = conn
        .session
        .as_mut()
        .ok_or_else(|| ImapError::bad("No session"))?;
    let messages = session.fetch(uid, &seq_set, &options).await?;

    // Write fetch responses
    for msg in messages {
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
            .await;

        let mut has_item = false;

        // Write flags
        if options.flags {
            if has_item {
                conn.encoder.sp().await;
            }
            let flag_strs: Vec<String> = msg.flags.iter().map(|f| f.0.clone()).collect();
            conn.encoder.atom("FLAGS").await.sp().await;
            write_flag_list(conn, &flag_strs).await;
            has_item = true;
        }

        // Write UID
        if options.uid || uid {
            if has_item {
                conn.encoder.sp().await;
            }
            conn.encoder
                .atom("UID")
                .await
                .sp()
                .await
                .number(msg.uid)
                .await;
            has_item = true;
        }

        // Write INTERNALDATE
        if options.internal_date {
            if has_item {
                conn.encoder.sp().await;
            }
            conn.encoder
                .atom("INTERNALDATE")
                .await
                .sp()
                .await
                .quoted(&msg.internal_date)
                .await;
            has_item = true;
        }

        // Write RFC822.SIZE
        if options.rfc822_size {
            if has_item {
                conn.encoder.sp().await;
            }
            conn.encoder
                .atom("RFC822.SIZE")
                .await
                .sp()
                .await
                .number(msg.rfc822_size)
                .await;
            has_item = true;
        }

        // Write body sections (BODY[] etc.)
        for (bi, bs) in options.body_sections.iter().enumerate() {
            if bi > 0 || has_item {
                conn.encoder.sp().await;
            }
            write_body_section_response(conn, bs, &msg.body).await;
            has_item = true;
        }

        // Close the fetch response
        conn.encoder
            .special(b')')
            .await
            .crlf()
            .await
            .map_err(|e| ImapError::Internal(Box::new(e)))?;
    }

    conn.write_ok(tag, "FETCH completed").await;
    Ok(())
}

/// Write a single body section response.
async fn write_body_section_response(conn: &mut Conn, bs: &BodySection, body: &[u8]) {
    let len = body.len();

    // RFC 3501 §7.4.2: the FETCH *response* data item is always `BODY[<section>]`.
    // `.PEEK` is a request-side modifier only (it suppresses \Seen); the
    // response label is always BODY regardless of peek. Echoing `BODY.PEEK`
    // here made strict clients (async-imap / Delta Chat) misparse the item.
    let _ = bs.peek;
    conn.encoder.atom("BODY").await;
    conn.encoder.special(b'[').await;

    if !bs.part.is_empty() {
        let parts: Vec<String> = bs.part.iter().map(|p| p.to_string()).collect();
        conn.encoder.atom(&parts.join(".")).await;
    }

    let spec = bs.specifier.to_wire();
    if !spec.is_empty() {
        if !bs.part.is_empty() {
            conn.encoder.special(b'.').await;
        }
        conn.encoder.atom(&spec).await;
    }

    conn.encoder.special(b']').await;

    if let Some((offset, size)) = bs.partial {
        conn.encoder
            .special(b'<')
            .await
            .number64(offset as i64)
            .await
            .special(b'.')
            .await
            .number64(size as i64)
            .await
            .special(b'>')
            .await;
    }

    conn.encoder.sp().await;
    let literal_header = format!("{{{len}}}\r\n");
    conn.encoder
        .write_raw(literal_header.as_bytes())
        .await;
    conn.encoder.write_raw(body).await;
}

/// Parse FETCH arguments from the decoder.
async fn parse_fetch_args(conn: &mut Conn) -> Result<FetchOptions, ImapError> {
    let mut options = FetchOptions::default();

    // Peek at the next byte: if '(', it's a list of attrs
    let is_list = {
        let b = conn.decoder.read_byte().await?;
        if b == b'(' {
            true
        } else {
            conn.decoder.unread_byte(b);
            false
        }
    };

    if is_list {
        loop {
            // Read fetch attribute name. Must stop at '[' so that
            // `BODY.PEEK[<section>]` is split into keyword + section.
            let atom = conn.decoder.read_fetch_att_name().await?;
            apply_fetch_att(&atom, conn, &mut options).await?;

            // Check if next byte is ')'
            let b = conn.decoder.read_byte().await?;
            if b == b')' {
                break;
            }
            // Must be SP
            if b != b' ' {
                conn.decoder.unread_byte(b);
                break;
            }
        }
    } else {
        let atom = conn.decoder.read_fetch_att_name().await?;
        apply_fetch_att(&atom, conn, &mut options).await?;
    }

    Ok(options)
}

/// Apply a single FETCH attribute name to the options.
async fn apply_fetch_att(
    name: &str,
    conn: &mut Conn,
    options: &mut FetchOptions,
) -> ImapResult<()> {
    match name.to_ascii_uppercase().as_str() {
        "ALL" => {
            options.flags = true;
            options.internal_date = true;
            options.rfc822_size = true;
            options.envelope = true;
        }
        "FAST" => {
            options.flags = true;
            options.internal_date = true;
            options.rfc822_size = true;
        }
        "FULL" => {
            options.flags = true;
            options.internal_date = true;
            options.rfc822_size = true;
            options.envelope = true;
            options.body_structure = true;
        }
        "FLAGS" => options.flags = true,
        "UID" => options.uid = true,
        "INTERNALDATE" => options.internal_date = true,
        "RFC822.SIZE" => options.rfc822_size = true,
        "ENVELOPE" => options.envelope = true,
        "BODYSTRUCTURE" => {
            options.body_structure = true;
            options.body_structure_extended = true;
        }
        "RFC822" | "RFC822.HEADER" | "RFC822.TEXT" => {
            // Add a default body section
            options.body_sections.push(BodySection::default());
        }
        "BODY" => {
            // Check if '[' follows
            let b = conn.decoder.read_byte().await?;
            if b == b'[' {
                let mut bs = read_body_section(conn).await?;
                bs.partial = read_partial_opt(conn).await?;
                options.body_sections.push(bs);
            } else {
                conn.decoder.unread_byte(b);
                // BODY without [] = non-extended BodyStructure
                options.body_structure = true;
            }
        }
        "BODY.PEEK" => {
            // Expect '['
            let b = conn.decoder.read_byte().await?;
            if b == b'[' {
                let mut bs = read_body_section(conn).await?;
                bs.peek = true;
                bs.partial = read_partial_opt(conn).await?;
                options.body_sections.push(bs);
            } else {
                conn.decoder.unread_byte(b);
            }
        }
        _ => {
            // Try parse_fetch_att for unrecognized ones
            match parse_fetch_att(name, "") {
                Ok(FetchAtt::BodySection(bs)) => {
                    options.body_sections.push(bs);
                }
                _ => {} // ignore unknown
            }
        }
    }

    Ok(())
}

/// Read an optional partial specifier `<offset.size>` that follows a body
/// section's closing `]` (RFC 3501 §6.4.5: `BODY [section] <<origin octet>>`).
/// The partial lives OUTSIDE the brackets; `read_body_section` stops at `]`.
async fn read_partial_opt(conn: &mut Conn) -> Result<Option<(u64, u64)>, ImapError> {
    if conn.decoder.peek_byte().await? != b'<' {
        return Ok(None);
    }
    conn.decoder.read_byte().await?; // consume '<'
    let offset = conn.decoder.read_number().await? as u64;
    if conn.decoder.read_byte().await? != b'.' {
        return Err(ImapError::bad("expected '.' in partial"));
    }
    let size = conn.decoder.read_number().await? as u64;
    if conn.decoder.read_byte().await? != b'>' {
        return Err(ImapError::bad("expected '>' closing partial"));
    }
    Ok(Some((offset, size)))
}

/// Read a body section specifier starting after '['.
async fn read_body_section(conn: &mut Conn) -> Result<BodySection, ImapError> {
    let mut bs = BodySection::default();

    // Read until ']'
    let mut spec = String::new();
    loop {
        let b = conn.decoder.read_byte().await?;
        if b == b']' {
            break;
        }
        spec.push(b as char);
    }

    if !spec.is_empty() {
        // Parse part numbers and specifier
        let spec_upper = spec.to_ascii_uppercase();
        if let Some(dot_pos) = spec_upper.find(|c: char| !c.is_ascii_digit() && c != '.') {
            let part_str = &spec[..dot_pos];
            let spec_str = &spec[dot_pos..];

            // Parse part numbers
            for p in part_str.split('.') {
                if let Ok(n) = p.parse::<u32>() {
                    bs.part.push(n);
                }
            }

            // Parse specifier
            let spec_trimmed = spec_str.trim_start_matches('.').to_ascii_uppercase();
            bs.specifier = match spec_trimmed.as_str() {
                "HEADER" => SectionSpecifier::Header,
                "TEXT" => SectionSpecifier::Text,
                "MIME" => SectionSpecifier::Mime,
                s if s.starts_with("HEADER.FIELDS ") => {
                    let fields_str = s["HEADER.FIELDS ".len()..].trim();
                    let fields = fields_str
                        .trim_start_matches('(')
                        .trim_end_matches(')')
                        .split_whitespace()
                        .map(|f| f.to_string())
                        .collect();
                    SectionSpecifier::HeaderFields(fields)
                }
                s if s.starts_with("HEADER.FIELDS.NOT ") => {
                    let fields_str = s["HEADER.FIELDS.NOT ".len()..].trim();
                    let fields = fields_str
                        .trim_start_matches('(')
                        .trim_end_matches(')')
                        .split_whitespace()
                        .map(|f| f.to_string())
                        .collect();
                    SectionSpecifier::HeaderFieldsNot(fields)
                }
                _ => SectionSpecifier::None,
            };
        } else {
            // All digits — part numbers with no specifier
            for p in spec.split('.') {
                if let Ok(n) = p.parse::<u32>() {
                    bs.part.push(n);
                }
            }
        }
    }

    Ok(bs)
}