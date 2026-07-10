//! Helper types for writing IMAP response data.
//!
//! These provide a streaming interface for backends to write FETCH, LIST,
//! EXPUNGE, and other responses without buffering entire result sets.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use imap_core::bodystructure::{BodyStructure, BodyStructureDisposition};
use imap_core::codec::Encoder;
use imap_core::fetch::BodySection;
use imap_core::select::ListData;
use imap_core::types::{Address, Envelope, Flag};

use crate::conn::Conn;
use crate::tracker::TrackerUpdate;

// ── FetchResponseWriter ──────────────────────────────────────────────

/// Writes FETCH responses for a single message.
///
/// The backend creates one of these per message in a FETCH result and
/// calls the individual item methods. The writer handles separator logic
/// (spaces between items) and the opening/closing parentheses.
pub struct FetchResponseWriter<'a> {
    conn: &'a mut Conn,
    has_item: bool,
}

impl<'a> FetchResponseWriter<'a> {
    pub(crate) fn new(conn: &'a mut Conn) -> Self {
        Self {
            conn,
            has_item: false,
        }
    }

    /// Write item separator (space between items).
    async fn sep(&mut self) {
        if self.has_item {
            self.conn.encoder.sp().await;
        }
        self.has_item = true;
    }

    // ── Basic items ──────────────────────────────────────────────────

    /// Write the UID.
    pub async fn uid(&mut self, uid: u32) {
        self.sep().await;
        self.conn
            .encoder
            .atom("UID")
            .await
            .sp()
            .await
            .number(uid)
            .await;
    }

    /// Write the flag list.
    pub async fn flags(&mut self, flags: &[Flag]) {
        self.sep().await;
        self.conn.encoder.atom("FLAGS").await.sp().await;
        write_flag_list_enc(&mut self.conn.encoder, flags).await;
    }

    /// Write the internal date.
    pub async fn internal_date(&mut self, date: &str) {
        self.sep().await;
        self.conn
            .encoder
            .atom("INTERNALDATE")
            .await
            .sp()
            .await
            .quoted(date)
            .await;
    }

    /// Write RFC822.SIZE.
    pub async fn rfc822_size(&mut self, size: u32) {
        self.sep().await;
        self.conn
            .encoder
            .atom("RFC822.SIZE")
            .await
            .sp()
            .await
            .number(size)
            .await;
    }

    // ── Structure items ──────────────────────────────────────────────

    /// Write ENVELOPE response.
    pub async fn envelope(&mut self, env: &Envelope) {
        self.sep().await;
        self.conn.encoder.atom("ENVELOPE").await.sp().await;
        write_envelope(&mut self.conn.encoder, env).await;
    }

    /// Write BODYSTRUCTURE response.
    ///
    /// When `extended` is true, writes `BODYSTRUCTURE` (extended form).
    /// When false, writes `BODY` (non-extended form per RFC 3501).
    pub async fn body_structure(&mut self, bs: &BodyStructure, extended: bool) {
        self.sep().await;
        if extended {
            self.conn.encoder.atom("BODYSTRUCTURE").await.sp().await;
        } else {
            self.conn.encoder.atom("BODY").await.sp().await;
        }
        write_body_structure(&mut self.conn.encoder, bs).await;
    }

    /// Write MODSEQ response (CONDSTORE extension).
    pub async fn modseq(&mut self, modseq: u64) {
        self.sep().await;
        self.conn
            .encoder
            .atom("MODSEQ")
            .await
            .sp()
            .await
            .special(b'(')
            .await
            .number64(modseq as i64)
            .await
            .special(b')')
            .await;
    }

    // ── Body / Binary section items ──────────────────────────────────

    /// Write BODY[] with literal data.
    pub async fn body(&mut self, data: &[u8]) {
        self.sep().await;
        let len = data.len();
        self.conn.encoder.atom("BODY[]").await.sp().await;
        self.conn
            .encoder
            .write_raw(format!("{{{len}}}\r\n").as_bytes())
            .await;
        self.conn.encoder.write_raw(data).await;
    }

    /// Write body section with raw data (for BODY[], BODY[HEADER], etc.).
    ///
    /// This is the general form that handles part paths, specifiers,
    /// peek mode, and partial fetches.
    pub async fn body_section(&mut self, section: &BodySection, data: &[u8]) {
        self.sep().await;
        if section.peek {
            self.conn.encoder.atom("BODY.PEEK").await;
        } else {
            self.conn.encoder.atom("BODY").await;
        }
        self.conn.encoder.special(b'[').await;

        if !section.part.is_empty() {
            let parts: Vec<String> = section.part.iter().map(|p| p.to_string()).collect();
            self.conn.encoder.atom(&parts.join(".")).await;
        }

        let spec = section.specifier.to_wire();
        if !spec.is_empty() {
            if !section.part.is_empty() {
                self.conn.encoder.special(b'.').await;
            }
            self.conn.encoder.atom(&spec).await;
        }

        self.conn.encoder.special(b']').await;

        if let Some((offset, size)) = section.partial {
            self.conn
                .encoder
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

        self.conn.encoder.sp().await;
        let len = data.len();
        self.conn
            .encoder
            .write_raw(format!("{{{len}}}\r\n").as_bytes())
            .await;
        self.conn.encoder.write_raw(data).await;
    }

    /// Write BINARY section response.
    ///
    /// Note: The actual base64 encoding is left to the backend. The `data`
    /// parameter should already be base64-encoded. The `~{len}` literal
    /// prefix is written per RFC 3516.
    pub async fn binary_section(&mut self, part: &[u32], data: &[u8]) {
        self.sep().await;
        let part_str: Vec<String> = part.iter().map(|p| p.to_string()).collect();
        self.conn
            .encoder
            .atom("BINARY")
            .await
            .special(b'[')
            .await
            .atom(&part_str.join("."))
            .await
            .special(b']')
            .await
            .sp()
            .await;
        let len = data.len();
        self.conn
            .encoder
            .write_raw(format!("~{{{len}}}\r\n").as_bytes())
            .await;
        self.conn.encoder.write_raw(data).await;
    }

    /// Write BINARY.SIZE response.
    pub async fn binary_section_size(&mut self, part: &[u32], size: u32) {
        self.sep().await;
        let part_str: Vec<String> = part.iter().map(|p| p.to_string()).collect();
        self.conn
            .encoder
            .atom("BINARY.SIZE")
            .await
            .special(b'[')
            .await
            .atom(&part_str.join("."))
            .await
            .special(b']')
            .await
            .sp()
            .await
            .number(size)
            .await;
    }

    // ── Lifecycle ────────────────────────────────────────────────────

    /// Close the FETCH response (write ")" CRLF).
    pub async fn close(&mut self) -> Result<(), std::io::Error> {
        self.conn.encoder.special(b')').await.crlf().await
    }
}

// ── ListWriter ───────────────────────────────────────────────────────

/// Writes LIST or LSUB responses.
///
/// Created by the command handler for LIST/LSUB commands. The backend
/// calls `write_list` for each matching mailbox.
pub struct ListWriter<'a> {
    conn: &'a mut Conn,
    lsub: bool,
}

impl<'a> ListWriter<'a> {
    pub(crate) fn new(conn: &'a mut Conn, lsub: bool) -> Self {
        Self { conn, lsub }
    }

    /// Write a LIST (or LSUB) response.
    pub async fn write_list(&mut self, data: &ListData) -> Result<(), std::io::Error> {
        let verb = if self.lsub { "LSUB" } else { "LIST" };
        self.conn
            .encoder
            .atom("*")
            .await
            .sp()
            .await
            .atom(verb)
            .await
            .sp()
            .await;

        // Attributes
        self.conn.encoder.special(b'(').await;
        for (i, attr) in data.attrs.iter().enumerate() {
            if i > 0 {
                self.conn.encoder.sp().await;
            }
            self.conn.encoder.atom(&attr.0).await;
        }
        self.conn.encoder.special(b')').await.sp().await;

        // Delimiter
        if data.delimiter.is_empty() {
            self.conn.encoder.nil().await;
        } else {
            self.conn.encoder.quoted(&data.delimiter).await;
        }
        self.conn.encoder.sp().await;

        // Name -- INBOX is always uppercase per convention
        if data.name.eq_ignore_ascii_case("INBOX") {
            self.conn.encoder.atom("INBOX").await;
        } else {
            self.conn.encoder.string(&data.name).await;
        }

        self.conn.encoder.crlf().await
    }
}

// ── MoveWriter ───────────────────────────────────────────────────────

/// Writes MOVE/COPY responses.
///
/// Used for COPY and MOVE commands. The COPYUID response code is written
/// as part of the tagged OK response by the command handler, not here.
pub struct MoveWriter<'a> {
    conn: &'a mut Conn,
}

impl<'a> MoveWriter<'a> {
    pub(crate) fn new(conn: &'a mut Conn) -> Self {
        Self { conn }
    }

    /// Write COPYUID response.
    ///
    /// Note: COPYUID is typically written as part of the tagged OK response
    /// (e.g., `OK [COPYUID ...]`), not as a separate untagged response.
    /// This method is a placeholder for future implementation when the
    /// command handler integration is added.
    pub async fn write_copy_uid(
        &mut self,
        uid_validity: u32,
        source_uids: &[u32],
        dest_uids: &[u32],
    ) -> Result<(), std::io::Error> {
        // COPYUID is written as part of the tagged OK response, not here.
        // Suppress unused parameter warnings.
        let _ = (uid_validity, source_uids, dest_uids);
        Ok(())
    }
}

// ── ExpungeWriter ────────────────────────────────────────────────────

/// Writes unsolicited EXPUNGE responses.
pub struct ExpungeWriter<'a> {
    conn: &'a mut Conn,
}

impl<'a> ExpungeWriter<'a> {
    pub(crate) fn new(conn: &'a mut Conn) -> Self {
        Self { conn }
    }

    /// Write an EXPUNGE response for a sequence number.
    pub async fn write_expunge(&mut self, seq: u32) -> Result<(), std::io::Error> {
        self.conn
            .encoder
            .atom("*")
            .await
            .sp()
            .await
            .number(seq)
            .await
            .sp()
            .await
            .atom("EXPUNGE")
            .await
            .crlf()
            .await
    }
}

// ── UpdateWriter ─────────────────────────────────────────────────────

/// Writes unsolicited update responses (expunge, exists, flags, etc.)
pub struct UpdateWriter<'a> {
    conn: &'a mut Conn,
}

impl<'a> UpdateWriter<'a> {
    pub(crate) fn new(conn: &'a mut Conn) -> Self {
        Self { conn }
    }

    /// Write a single TrackerUpdate to the connection.
    pub async fn write_update(&mut self, update: &TrackerUpdate) -> Result<(), std::io::Error> {
        match update {
            TrackerUpdate::Expunge(seq) => self.write_expunge(*seq).await,
            TrackerUpdate::NumMessages(n) => self.write_num_messages(*n).await,
            TrackerUpdate::MailboxFlags(flags) => self.write_mailbox_flags(flags).await,
            TrackerUpdate::MessageFlags { seq, uid: _, flags } => {
                self.write_message_flags(*seq, flags).await
            }
        }
    }

    /// Write EXPUNGE response.
    pub async fn write_expunge(&mut self, seq: u32) -> Result<(), std::io::Error> {
        self.conn
            .encoder
            .atom("*")
            .await
            .sp()
            .await
            .number(seq)
            .await
            .sp()
            .await
            .atom("EXPUNGE")
            .await
            .crlf()
            .await
    }

    /// Write EXISTS response.
    pub async fn write_num_messages(&mut self, n: u32) -> Result<(), std::io::Error> {
        self.conn
            .encoder
            .atom("*")
            .await
            .sp()
            .await
            .number(n)
            .await
            .sp()
            .await
            .atom("EXISTS")
            .await
            .crlf()
            .await
    }

    /// Write FLAGS response for the mailbox.
    pub async fn write_mailbox_flags(&mut self, flags: &[Flag]) -> Result<(), std::io::Error> {
        self.conn.encoder.atom("*").await.sp().await.atom("FLAGS").await.sp().await;
        write_flag_list_enc(&mut self.conn.encoder, flags).await;
        self.conn.encoder.crlf().await
    }

    /// Write FETCH FLAGS response for a message.
    pub async fn write_message_flags(
        &mut self,
        seq: u32,
        flags: &[Flag],
    ) -> Result<(), std::io::Error> {
        self.conn
            .encoder
            .atom("*")
            .await
            .sp()
            .await
            .number(seq)
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
        write_flag_list_enc(&mut self.conn.encoder, flags).await;
        self.conn.encoder.special(b')').await.crlf().await
    }
}

// ── Helper functions ─────────────────────────────────────────────────

/// Write a parenthesized flag list from `&[Flag]` using the Encoder directly.
/// Does NOT write CRLF -- the caller continues the chain.
async fn write_flag_list_enc<W: tokio::io::AsyncWrite + Unpin>(
    encoder: &mut Encoder<W>,
    flags: &[Flag],
) {
    encoder.special(b'(').await;
    for (i, f) in flags.iter().enumerate() {
        if i > 0 {
            encoder.sp().await;
        }
        encoder.flag(&f.0).await;
    }
    encoder.special(b')').await;
}

/// Write an ENVELOPE structure to the encoder (RFC 3501 section 7.4.2).
async fn write_envelope<W: tokio::io::AsyncWrite + Unpin>(
    encoder: &mut Encoder<W>,
    env: &Envelope,
) {
    encoder.special(b'(').await;
    // date, subject
    encoder.nstring(&env.date).await.sp().await;
    encoder.nstring(&env.subject).await;
    // Address lists: from, sender, reply-to, to, cc, bcc
    write_address_list(encoder, &env.from).await;
    write_address_list(encoder, &env.sender).await;
    write_address_list(encoder, &env.reply_to).await;
    write_address_list(encoder, &env.to).await;
    write_address_list(encoder, &env.cc).await;
    write_address_list(encoder, &env.bcc).await;
    encoder.sp().await;
    // In-Reply-To: join multiple IDs with space, or NIL if empty
    if env.in_reply_to.is_empty() {
        encoder.nil().await;
    } else {
        encoder.nstring(&env.in_reply_to.join(" ")).await;
    }
    encoder.sp().await;
    // Message-ID
    encoder.nstring(&env.message_id).await;
    encoder.special(b')').await;
}

/// Write an address list (parenthesized list of addresses, or NIL if empty).
async fn write_address_list<W: tokio::io::AsyncWrite + Unpin>(
    encoder: &mut Encoder<W>,
    addrs: &[Address],
) {
    encoder.sp().await;
    if addrs.is_empty() {
        encoder.nil().await;
        return;
    }
    encoder.special(b'(').await;
    for (i, addr) in addrs.iter().enumerate() {
        if i > 0 {
            encoder.sp().await;
        }
        encoder.special(b'(').await;
        encoder.nstring(&addr.name).await.sp().await;
        encoder.nil().await.sp().await; // route (NIL per RFC 3501)
        encoder.nstring(&addr.mailbox).await.sp().await;
        encoder.nstring(&addr.host).await;
        encoder.special(b')').await;
    }
    encoder.special(b')').await;
}

/// Write a BODYSTRUCTURE to the encoder (RFC 3501 section 7.4.2).
/// Returns a boxed future to handle recursion (Rust async fn size is infinite otherwise).
fn write_body_structure<'a, W: tokio::io::AsyncWrite + Unpin + Send + 'a>(
    encoder: &'a mut Encoder<W>,
    bs: &'a BodyStructure,
) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
    match bs {
        BodyStructure::SinglePart {
            typ,
            subtype,
            params,
            id,
            description,
            encoding,
            size,
            message_rfc822,
            text,
            extended,
        } => {
            encoder.special(b'(').await;
            encoder.string(typ).await.sp().await;
            encoder.string(subtype).await.sp().await;
            write_fld_param(encoder, params).await;
            encoder.sp().await;
            encoder.nstring(id).await.sp().await;
            encoder.nstring(description).await.sp().await;
            encoder.string(encoding).await.sp().await;
            encoder.number(*size).await;

            // Type-specific fields
            if typ.eq_ignore_ascii_case("message") && subtype.eq_ignore_ascii_case("rfc822") {
                if let Some(msg) = message_rfc822 {
                    encoder.sp().await;
                    if let Some(ref env) = msg.envelope {
                        write_envelope(encoder, env).await;
                    } else {
                        encoder.nil().await;
                    }
                    encoder.sp().await;
                    if let Some(ref body) = msg.body_structure {
                        write_body_structure(encoder, body).await;
                    } else {
                        encoder.nil().await;
                    }
                    encoder
                        .sp()
                        .await
                        .number64(msg.num_lines)
                        .await;
                }
            } else if typ.eq_ignore_ascii_case("text") {
                if let Some(ref t) = text {
                    encoder.sp().await.number64(t.num_lines).await;
                }
            }

            // Extended data (only present in BODYSTRUCTURE, not BODY)
            if let Some(ref ext) = extended {
                encoder.sp().await;
                if let Some(ref disp) = ext.disposition {
                    write_fld_dsp(encoder, disp).await;
                } else {
                    encoder.nil().await;
                }
                encoder.sp().await;
                write_fld_lang(encoder, &ext.language).await;
                encoder.sp().await.nstring(&ext.location).await;
            }
            encoder.special(b')').await;
        }
        BodyStructure::MultiPart {
            children,
            subtype,
            extended,
        } => {
            encoder.special(b'(').await;
            for child in children {
                write_body_structure(encoder, child).await;
                encoder.sp().await;
            }
            encoder.string(subtype).await;

            // Extended data
            if let Some(ref ext) = extended {
                encoder.sp().await;
                write_fld_param(encoder, &ext.params).await;
                encoder.sp().await;
                if let Some(ref disp) = ext.disposition {
                    write_fld_dsp(encoder, disp).await;
                } else {
                    encoder.nil().await;
                }
                encoder.sp().await;
                write_fld_lang(encoder, &ext.language).await;
                encoder.sp().await.nstring(&ext.location).await;
            }
            encoder.special(b')').await;
        }
    }
    })
}

/// Write a body-fld-param (parenthesized parameter list).
async fn write_fld_param<W: tokio::io::AsyncWrite + Unpin>(
    encoder: &mut Encoder<W>,
    params: &HashMap<String, String>,
) {
    if params.is_empty() {
        encoder.nil().await;
        return;
    }
    encoder.special(b'(').await;
    let mut first = true;
    for (k, v) in params {
        if !first {
            encoder.sp().await;
        }
        first = false;
        encoder.string(k).await.sp().await.string(v).await;
    }
    encoder.special(b')').await;
}

/// Write a body-fld-disposition (parenthesized disposition).
async fn write_fld_dsp<W: tokio::io::AsyncWrite + Unpin>(
    encoder: &mut Encoder<W>,
    disp: &BodyStructureDisposition,
) {
    encoder.special(b'(').await;
    encoder.string(&disp.value).await.sp().await;
    write_fld_param(encoder, &disp.params).await;
    encoder.special(b')').await;
}

/// Write a body-fld-lang (language list or single string).
async fn write_fld_lang<W: tokio::io::AsyncWrite + Unpin>(
    encoder: &mut Encoder<W>,
    lang: &[String],
) {
    if lang.is_empty() {
        encoder.nil().await;
    } else if lang.len() == 1 {
        encoder.string(&lang[0]).await;
    } else {
        encoder.special(b'(').await;
        for (i, l) in lang.iter().enumerate() {
            if i > 0 {
                encoder.sp().await;
            }
            encoder.string(l).await;
        }
        encoder.special(b')').await;
    }
}
