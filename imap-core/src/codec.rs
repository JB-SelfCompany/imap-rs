use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use crate::error::{ImapError, ImapResult};

// ── Atom helpers ────────────────────────────────────────────────────

pub fn is_atom_char(ch: u8) -> bool {
    !matches!(ch, b'(' | b')' | b'{' | b' ' | b'%' | b'*' | b'"' | b'\\' | b']')
        && !ch.is_ascii_control()
}

fn is_astring_char(ch: u8) -> bool {
    is_atom_char(ch) || ch == b']'
}

/// LIST-wildcard char: atom-char + `*` + `%` + `]` (resp-specials).
fn is_list_char(ch: u8) -> bool {
    ch == b'*' || ch == b'%' || is_astring_char(ch)
}

fn is_num_set_char(ch: u8) -> bool {
    ch == b'*' || is_atom_char(ch)
}

// ── ParsedCommand ───────────────────────────────────────────────────

/// A partially-parsed command (tag + verb + uid flag). Args are read by each handler.
#[derive(Debug)]
pub struct ParsedCommand {
    pub tag: String,
    pub verb: String,
    pub uid: bool,
}

// ── AsyncDecoder ────────────────────────────────────────────────────

/// Reads IMAP commands from an async reader using read_exact + pushback byte.
pub struct AsyncDecoder<R> {
    inner: BufReader<R>,
    err: Option<ImapError>,
    literal_remaining: u64,
    read_bytes: u64,
    max_size: u64,
    pushed_back: Option<u8>,
}

impl<R: tokio::io::AsyncRead + Unpin> AsyncDecoder<R> {
    pub fn new(reader: R) -> Self {
        Self {
            inner: BufReader::new(reader),
            err: None,
            literal_remaining: 0,
            read_bytes: 0,
            max_size: 50 * 1024,
            pushed_back: None,
        }
    }

    pub fn set_max_size(&mut self, max: u64) { self.max_size = max; }

    pub fn take_err(&mut self) -> Option<ImapError> { self.err.take() }

    pub async fn read_byte(&mut self) -> ImapResult<u8> {
        if let Some(b) = self.pushed_back.take() {
            return Ok(b);
        }
        if self.max_size > 0 && self.read_bytes > self.max_size {
            return Err(ImapError::bad("max command size exceeded"));
        }
        if self.literal_remaining > 0 {
            return Err(ImapError::Internal("cannot read command while literal open".into()));
        }
        let mut buf = [0u8; 1];
        self.inner.read_exact(&mut buf).await?;
        self.read_bytes += 1;
        Ok(buf[0])
    }

    pub fn unread_byte(&mut self, b: u8) {
        self.pushed_back = Some(b);
    }

    async fn accept_byte(&mut self, want: u8) -> ImapResult<bool> {
        let b = self.read_byte().await?;
        if b == want { Ok(true) } else { self.unread_byte(b); Ok(false) }
    }

    pub async fn expect_crlf(&mut self) -> ImapResult<()> {
        self.accept_byte(b' ').await?; // skip trailing SP
        self.accept_byte(b'\r').await?; // optional CR
        if !self.accept_byte(b'\n').await? {
            return Err(ImapError::bad("expected CRLF"));
        }
        Ok(())
    }

    pub async fn discard_line(&mut self) {
        loop {
            match self.read_byte().await {
                Ok(b'\n') | Err(_) => return,
                _ => {}
            }
        }
    }

    pub async fn read_atom(&mut self) -> ImapResult<String> {
        let mut s = String::new();
        loop {
            let b = self.read_byte().await?;
            if !is_atom_char(b) {
                self.unread_byte(b);
                break;
            }
            s.push(b as char);
        }
        if s.is_empty() { Err(ImapError::bad("expected atom")) }
        else { Ok(s) }
    }

    /// Read a FETCH attribute keyword.
    ///
    /// Identical to `read_atom` except it also stops at `[`, which begins the
    /// optional `[section]` of a `BODY`/`BODY.PEEK` fetch-att (RFC 3501 §6.4.5).
    ///
    /// Without this, `read_atom` greedily consumes `BODY.PEEK[HEADER.FIELDS`
    /// as one token (because `[` is a valid atom-char per RFC), then the parser
    /// chokes on the `(` of the field list with "expected atom" — exactly the
    /// bug that killed Delta Chat's IMAP sessions.
    pub async fn read_fetch_att_name(&mut self) -> ImapResult<String> {
        let mut s = String::new();
        loop {
            let b = self.read_byte().await?;
            if !is_atom_char(b) || b == b'[' {
                self.unread_byte(b);
                break;
            }
            s.push(b as char);
        }
        if s.is_empty() { Err(ImapError::bad("expected fetch attribute")) }
        else { Ok(s) }
    }

    /// Read an IMAP flag token: a keyword atom (`Junk`, `$MDNSent`) or a
    /// backslash-prefixed system/extension flag (`\Seen`, `\Recent`, `\$Label1`).
    ///
    /// `read_atom` cannot read these because `\` is a quoted-special and is
    /// excluded from atom-chars — so flag lists like `STORE +FLAGS (\Seen)`
    /// would otherwise fail with "expected atom".
    pub async fn read_flag(&mut self) -> ImapResult<String> {
        let mut s = String::new();
        let first = self.peek_byte().await?;
        if first == b'\\' {
            self.read_byte().await?; // consume the leading backslash
            s.push('\\');
        }
        loop {
            let b = self.read_byte().await?;
            if !is_atom_char(b) {
                self.unread_byte(b);
                break;
            }
            s.push(b as char);
        }
        if s.is_empty() || s == "\\" {
            Err(ImapError::bad("expected flag"))
        } else {
            Ok(s)
        }
    }

    pub async fn expect_sp(&mut self) -> ImapResult<()> {
        let b = self.read_byte().await?;
        if b == b' ' {
            let next = self.read_byte().await?;
            self.unread_byte(next);
            if next == b'\r' || next == b'\n' {
                return Err(ImapError::bad("expected SP"));
            }
            Ok(())
        } else if b == b'(' {
            // SP optional before parenthesized list (go-imap compat)
            self.unread_byte(b);
            Ok(())
        } else {
            Err(ImapError::bad("expected SP"))
        }
    }

    pub async fn read_number(&mut self) -> ImapResult<u32> {
        let mut s = String::new();
        loop {
            let b = self.read_byte().await?;
            if b.is_ascii_digit() {
                s.push(b as char);
            } else {
                self.unread_byte(b);
                break;
            }
        }
        if s.is_empty() { return Err(ImapError::bad("expected number")); }
        s.parse::<u32>().map_err(|_| ImapError::bad("number overflow"))
    }

    pub async fn read_number64(&mut self) -> ImapResult<i64> {
        let mut s = String::new();
        loop {
            let b = self.read_byte().await?;
            if b.is_ascii_digit() || (s.is_empty() && b == b'-') {
                s.push(b as char);
            } else {
                self.unread_byte(b);
                break;
            }
        }
        if s.is_empty() { return Err(ImapError::bad("expected number64")); }
        s.parse::<i64>().map_err(|_| ImapError::bad("number64 overflow"))
    }

    pub async fn read_quoted(&mut self) -> ImapResult<String> {
        if !self.accept_byte(b'"').await? {
            return Err(ImapError::bad("expected '\"'"));
        }
        let mut s = String::new();
        loop {
            let b = self.read_byte().await?;
            if b == b'"' { break; }
            if b == b'\\' {
                let esc = self.read_byte().await?;
                s.push(esc as char);
            } else {
                s.push(b as char);
            }
        }
        Ok(s)
    }

    pub async fn read_string(&mut self) -> ImapResult<String> {
        if self.peek_byte().await? == b'"' {
            return self.read_quoted().await;
        }
        let bytes = self.read_literal_data().await?;
        String::from_utf8(bytes).map_err(|_| ImapError::bad("literal not valid UTF-8"))
    }

    pub async fn peek_byte(&mut self) -> ImapResult<u8> {
        let b = self.read_byte().await?;
        self.unread_byte(b);
        Ok(b)
    }

    /// Read astring: atom or string (quoted/literal).
    pub async fn read_astring(&mut self) -> ImapResult<String> {
        let b = self.peek_byte().await?;
        if b == b'"' || b == b'{' {
            return self.read_string().await;
        }
        let mut s = String::new();
        loop {
            let b = self.read_byte().await?;
            if !is_astring_char(b) {
                self.unread_byte(b);
                break;
            }
            s.push(b as char);
        }
        if s.is_empty() { Err(ImapError::bad("expected astring")) }
        else { Ok(s) }
    }

    /// Read a LIST mailbox pattern: like astring but also accepts `*` and `%` wildcards.
    pub async fn read_list_mailbox(&mut self) -> ImapResult<String> {
        let b = self.peek_byte().await?;
        if b == b'"' || b == b'{' {
            return self.read_string().await;
        }
        let mut s = String::new();
        loop {
            let b = self.read_byte().await?;
            if !is_list_char(b) {
                self.unread_byte(b);
                break;
            }
            s.push(b as char);
        }
        if s.is_empty() { Err(ImapError::bad("expected list-mailbox")) }
        else { Ok(s) }
    }

    /// Read nstring: NIL → None, else string → Some.
    pub async fn read_nstring(&mut self) -> ImapResult<Option<String>> {
        let b = self.peek_byte().await?;
        if b == b'N' || b == b'n' {
            let atom = self.read_atom().await?;
            if atom.eq_ignore_ascii_case("NIL") { return Ok(None); }
            return Err(ImapError::bad("expected NIL or string"));
        }
        self.read_string().await.map(Some)
    }

    /// Read a parenthesized list, calling `f` for each item.
    pub async fn read_list<F, Fut>(&mut self, mut f: F) -> ImapResult<bool>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = ImapResult<()>>,
    {
        if !self.accept_byte(b'(').await? { return Ok(false); }
        if self.accept_byte(b')').await? { return Ok(true); }
        loop {
            f().await?;
            if self.accept_byte(b')').await? { return Ok(true); }
            self.expect_sp().await?;
        }
    }

    /// Read literal: `{N}\r\n<data>` or `{N+}\r\n<data>`.
    /// Returns (data, is_non_sync).
    pub async fn read_literal(&mut self) -> ImapResult<(Vec<u8>, bool)> {
        if !self.accept_byte(b'{').await? {
            return Err(ImapError::bad("expected '{'"));
        }
        let size = self.read_number64().await? as u64;
        let non_sync = self.accept_byte(b'+').await?;
        let cb = self.read_byte().await?;
        if cb != b'}' { return Err(ImapError::bad("expected '}'")); }
        let cr = self.read_byte().await?;
        let lf = self.read_byte().await?;
        if cr != b'\r' || lf != b'\n' { return Err(ImapError::bad("expected CRLF after literal")); }

        let mut data = vec![0u8; size as usize];
        self.inner.read_exact(&mut data).await?;
        self.read_bytes += size;
        Ok((data, non_sync))
    }

    pub async fn read_literal_data(&mut self) -> ImapResult<Vec<u8>> {
        let (data, _) = self.read_literal().await?;
        Ok(data)
    }

    /// Read a sequence set string (run of num-set chars).
    pub async fn read_num_set_str(&mut self) -> ImapResult<String> {
        let mut s = String::new();
        loop {
            let b = self.read_byte().await?;
            if !is_num_set_char(b) {
                self.unread_byte(b);
                break;
            }
            s.push(b as char);
        }
        if s.is_empty() { Err(ImapError::bad("expected sequence-set")) }
        else { Ok(s) }
    }

    /// Read text until CRLF.
    pub async fn read_text(&mut self) -> ImapResult<String> {
        let mut s = String::new();
        loop {
            let b = self.read_byte().await?;
            if b == b'\r' || b == b'\n' {
                self.unread_byte(b);
                break;
            }
            s.push(b as char);
        }
        Ok(s)
    }

    /// Read command: `tag SP verb [SP args...] CRLF`.
    /// Handles UID prefix.
    pub async fn read_command(&mut self) -> ImapResult<Option<ParsedCommand>> {
        // Reset the per-command size budget. `read_bytes` accumulates every
        // byte read (including args/literals read by handlers after this
        // returns), and `max_size` is meant to cap a SINGLE command — not the
        // whole connection lifetime. Without this reset, a long-lived session
        // (Delta Chat IDLE + periodic poll) crosses 50 KiB of cumulative
        // command input after a few hundred commands, after which EVERY
        // read_byte returns "max command size exceeded" and the serve loop
        // spins forever in the resync arm.
        self.read_bytes = 0;
        // Skip empty lines
        loop {
            let b = self.read_byte().await?;
            if b == b'\r' || b == b'\n' { continue; }
            self.unread_byte(b);
            break;
        }

        let tag = self.read_atom().await?;
        self.expect_sp().await?;
        let mut verb = self.read_atom().await?;
        verb.make_ascii_uppercase();

        let uid = if verb == "UID" {
            self.expect_sp().await?;
            let mut sub = self.read_atom().await?;
            sub.make_ascii_uppercase();
            verb = sub;
            true
        } else {
            false
        };

        // Consume the space between verb and arguments (if present).
        // read_atom() unreads the delimiter after the verb, leaving a leading
        // space before the first argument.  Handlers expect arguments to begin
        // immediately, so we absorb the SP here.
        let b = self.peek_byte().await?;
        if b == b' ' {
            self.read_byte().await?;
        }

        Ok(Some(ParsedCommand { tag, verb, uid }))
    }
}

// ── Encoder ─────────────────────────────────────────────────────────

/// Writes IMAP responses to an async writer.
pub struct Encoder<W> {
    inner: BufWriter<W>,
    err: Option<io::Error>,
}

impl<W: tokio::io::AsyncWrite + Unpin> Encoder<W> {
    pub fn new(writer: W) -> Self {
        Self { inner: BufWriter::new(writer), err: None }
    }

    fn set_err(&mut self, e: io::Error) {
        if self.err.is_none() { self.err = Some(e); }
    }

    pub async fn write_raw(&mut self, data: &[u8]) -> &mut Self {
        if self.err.is_some() { return self; }
        if let Err(e) = self.inner.write_all(data).await {
            self.set_err(e);
        }
        self
    }

    pub async fn atom(&mut self, s: &str) -> &mut Self {
        self.write_raw(s.as_bytes()).await
    }

    pub async fn sp(&mut self) -> &mut Self {
        self.write_raw(b" ").await
    }

    pub async fn special(&mut self, ch: u8) -> &mut Self {
        self.write_raw(&[ch]).await
    }

    pub async fn quoted(&mut self, s: &str) -> &mut Self {
        self.write_raw(b"\"").await;
        for &b in s.as_bytes() {
            if b == b'"' || b == b'\\' { self.write_raw(&[b'\\']).await; }
            self.write_raw(&[b]).await;
        }
        self.write_raw(b"\"").await
    }

    pub async fn string(&mut self, s: &str) -> &mut Self {
        if valid_quoted(s) {
            self.quoted(s).await
        } else {
            self.literal(s.as_bytes()).await
        }
    }

    pub async fn nstring(&mut self, s: &str) -> &mut Self {
        if s.is_empty() { self.nil().await } else { self.string(s).await }
    }

    pub async fn number(&mut self, n: u32) -> &mut Self {
        self.write_raw(n.to_string().as_bytes()).await
    }

    pub async fn number64(&mut self, n: i64) -> &mut Self {
        self.write_raw(n.to_string().as_bytes()).await
    }

    pub async fn nil(&mut self) -> &mut Self {
        self.write_raw(b"NIL").await
    }

    pub async fn text(&mut self, s: &str) -> &mut Self {
        self.write_raw(s.as_bytes()).await
    }

    pub async fn flag(&mut self, f: &str) -> &mut Self {
        self.write_raw(f.as_bytes()).await
    }

    pub async fn crlf(&mut self) -> io::Result<()> {
        self.write_raw(b"\r\n").await;
        if let Some(e) = self.err.take() { return Err(e); }
        self.inner.flush().await
    }

    pub async fn list<F, Fut>(&mut self, n: usize, mut f: F) -> &mut Self
    where
        F: FnMut(usize) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        self.special(b'(').await;
        for i in 0..n {
            if i > 0 { self.sp().await; }
            f(i).await;
        }
        self.special(b')').await
    }

    /// Write literal `{N}\r\n<data>` (server-side, always synchronizing).
    pub async fn literal(&mut self, data: &[u8]) -> &mut Self {
        let len = data.len();
        self.write_raw(format!("{{{len}}}\r\n").as_bytes()).await;
        self.write_raw(data).await
    }

    /// Write status response: `[tag] TYPE [CODE] text\r\n`
    pub async fn write_status(
        &mut self, tag: &str, typ: &str, code: Option<&str>, text: &str,
    ) -> io::Result<()> {
        let t = if tag.is_empty() { "*" } else { tag };
        self.atom(t).await.sp().await.atom(typ).await;
        if let Some(c) = code {
            self.sp().await.special(b'[').await.atom(c).await.special(b']').await;
        }
        self.sp().await.text(text).await.crlf().await
    }

    /// Write capability status response.
    pub async fn write_capability_status(
        &mut self, tag: &str, typ: &str, caps: &[String], text: &str,
    ) -> io::Result<()> {
        let t = if tag.is_empty() { "*" } else { tag };
        self.atom(t).await.sp().await.atom(typ).await.sp().await
            .special(b'[').await.atom("CAPABILITY").await;
        for c in caps {
            self.sp().await.atom(c).await;
        }
        self.special(b']').await.sp().await.text(text).await.crlf().await
    }

    /// Write continuation request: `+ text\r\n`
    pub async fn write_continuation(&mut self, text: &str) -> io::Result<()> {
        self.atom("+").await.sp().await.text(text).await.crlf().await
    }
}

fn valid_quoted(s: &str) -> bool {
    if s.len() > 4096 { return false; }
    for &b in s.as_bytes() {
        if matches!(b, 0 | b'\r' | b'\n') { return false; }
        if b > 127 { return false; }
    }
    true
}
