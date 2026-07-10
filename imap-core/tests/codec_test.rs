//! Codec tests for imap-core.

use imap_core::codec::*;

#[tokio::test]
async fn test_read_atom() {
    let data = b"hello world\r\n";
    let mut dec = AsyncDecoder::new(&data[..]);
    let atom = dec.read_atom().await.unwrap();
    assert_eq!(atom, "hello");
}

#[tokio::test]
async fn test_read_number() {
    let data = b"12345 ";
    let mut dec = AsyncDecoder::new(&data[..]);
    let num = dec.read_number().await.unwrap();
    assert_eq!(num, 12345);
}

#[tokio::test]
async fn test_read_quoted() {
    let data = b"\"hello world\"";
    let mut dec = AsyncDecoder::new(&data[..]);
    let s = dec.read_quoted().await.unwrap();
    assert_eq!(s, "hello world");
}

#[tokio::test]
async fn test_read_quoted_escape() {
    let data = b"\"hello \\\"world\\\"\"";
    let mut dec = AsyncDecoder::new(&data[..]);
    let s = dec.read_quoted().await.unwrap();
    assert_eq!(s, "hello \"world\"");
}

#[tokio::test]
async fn test_read_nstring_nil() {
    let data = b"NIL ";
    let mut dec = AsyncDecoder::new(&data[..]);
    let s = dec.read_nstring().await.unwrap();
    assert_eq!(s, None);
}

#[tokio::test]
async fn test_read_nstring_value() {
    let data = b"\"hello\" ";
    let mut dec = AsyncDecoder::new(&data[..]);
    let s = dec.read_nstring().await.unwrap();
    assert_eq!(s, Some("hello".to_string()));
}

#[tokio::test]
async fn test_read_astring_atom() {
    let data = b"INBOX ";
    let mut dec = AsyncDecoder::new(&data[..]);
    let s = dec.read_astring().await.unwrap();
    assert_eq!(s, "INBOX");
}

#[tokio::test]
async fn test_read_astring_quoted() {
    let data = b"\"Sent Mail\" ";
    let mut dec = AsyncDecoder::new(&data[..]);
    let s = dec.read_astring().await.unwrap();
    assert_eq!(s, "Sent Mail");
}

#[tokio::test]
async fn test_read_num_set() {
    let data = b"1:5,7,10:* ";
    let mut dec = AsyncDecoder::new(&data[..]);
    let s = dec.read_num_set_str().await.unwrap();
    assert_eq!(s, "1:5,7,10:*");
}

#[tokio::test]
async fn test_read_astring_empty_quoted() {
    let data = b"\"\" ";
    let mut dec = AsyncDecoder::new(tokio::io::BufReader::new(&data[..]));
    let s = dec.read_astring().await.unwrap();
    assert_eq!(s, "");
}

#[tokio::test]
async fn test_full_list_command_parse() {
    // Simulate: a2 LIST "" *\r\n
    let data = b"a2 LIST \"\" *\r\n";
    let mut dec = AsyncDecoder::new(tokio::io::BufReader::new(&data[..]));

    // read_command: read tag, SP, verb
    let cmd = dec.read_command().await.unwrap().unwrap();
    eprintln!("tag={} verb={}", cmd.tag, cmd.verb);
    assert_eq!(cmd.tag, "a2");
    assert_eq!(cmd.verb, "LIST");

    // LIST handler: read_astring for reference
    let reference = dec.read_astring().await.unwrap();
    eprintln!("reference={:?}", reference);
    assert_eq!(reference, "");

    // expect_sp
    dec.expect_sp().await.unwrap();
    eprintln!("sp ok");

    // read_list_mailbox for pattern (supports wildcards * and %)
    let pattern = dec.read_list_mailbox().await.unwrap();
    eprintln!("pattern={:?}", pattern);
    assert_eq!(pattern, "*");

    // expect_crlf
    dec.expect_crlf().await.unwrap();
    eprintln!("crlf ok");
}

#[tokio::test]
async fn test_read_text() {
    let data = b"Hello World\r\n";
    let mut dec = AsyncDecoder::new(&data[..]);
    let s = dec.read_text().await.unwrap();
    assert_eq!(s, "Hello World");
}

#[tokio::test]
async fn test_read_number64() {
    let data = b"-12345 ";
    let mut dec = AsyncDecoder::new(&data[..]);
    let num = dec.read_number64().await.unwrap();
    assert_eq!(num, -12345);
}

#[tokio::test]
async fn test_encoder_atom() {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(&mut buf);
        enc.atom("HELLO").await;
        enc.sp().await;
        enc.atom("WORLD").await;
        enc.crlf().await.unwrap();
    }
    assert_eq!(buf, b"HELLO WORLD\r\n");
}

#[tokio::test]
async fn test_encoder_quoted() {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(&mut buf);
        enc.quoted("hello world").await;
        enc.crlf().await.unwrap();
    }
    assert_eq!(buf, b"\"hello world\"\r\n");
}

#[tokio::test]
async fn test_encoder_number() {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(&mut buf);
        enc.number(42).await;
        enc.crlf().await.unwrap();
    }
    assert_eq!(buf, b"42\r\n");
}

#[tokio::test]
async fn test_encoder_nil() {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(&mut buf);
        enc.nil().await;
        enc.crlf().await.unwrap();
    }
    assert_eq!(buf, b"NIL\r\n");
}

#[tokio::test]
async fn test_encoder_nstring_nil() {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(&mut buf);
        enc.nstring("").await;
        enc.crlf().await.unwrap();
    }
    assert_eq!(buf, b"NIL\r\n");
}

#[tokio::test]
async fn test_encoder_nstring_value() {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(&mut buf);
        enc.nstring("hello").await;
        enc.crlf().await.unwrap();
    }
    assert_eq!(buf, b"\"hello\"\r\n");
}

#[tokio::test]
async fn test_encoder_flag() {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(&mut buf);
        enc.flag("\\Seen").await;
        enc.crlf().await.unwrap();
    }
    assert_eq!(buf, b"\\Seen\r\n");
}

#[tokio::test]
async fn test_encoder_text() {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(&mut buf);
        enc.text("Hello World").await;
        enc.crlf().await.unwrap();
    }
    assert_eq!(buf, b"Hello World\r\n");
}

#[tokio::test]
async fn test_encoder_special() {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(&mut buf);
        enc.special(b'[').await;
        enc.atom("TEST").await;
        enc.special(b']').await;
        enc.crlf().await.unwrap();
    }
    assert_eq!(buf, b"[TEST]\r\n");
}

#[tokio::test]
async fn test_encoder_write_status() {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(&mut buf);
        enc.write_status("a1", "OK", None, "completed").await.unwrap();
    }
    assert_eq!(buf, b"a1 OK completed\r\n");
}

#[tokio::test]
async fn test_encoder_write_status_with_code() {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(&mut buf);
        enc.write_status("*", "OK", Some("READ-WRITE"), "SELECT completed")
            .await
            .unwrap();
    }
    assert_eq!(buf, b"* OK [READ-WRITE] SELECT completed\r\n");
}

#[tokio::test]
async fn test_encoder_write_continuation() {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(&mut buf);
        enc.write_continuation("Ready for literal").await.unwrap();
    }
    assert_eq!(buf, b"+ Ready for literal\r\n");
}

#[tokio::test]
async fn test_encoder_list() {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(&mut buf);
        enc.list(2, |_i| async move {}).await;
        enc.crlf().await.unwrap();
    }
    // list(2) with no-op closure produces "( )" — one SP between items
    assert_eq!(buf, b"( )\r\n");
}

#[tokio::test]
async fn test_is_atom_char() {
    assert!(is_atom_char(b'a'));
    assert!(is_atom_char(b'Z'));
    assert!(is_atom_char(b'0'));
    assert!(is_atom_char(b'-'));
    assert!(!is_atom_char(b'('));
    assert!(!is_atom_char(b')'));
    assert!(!is_atom_char(b' '));
    assert!(!is_atom_char(b'{'));
    assert!(!is_atom_char(b'%'));
    assert!(!is_atom_char(b'*'));
    assert!(!is_atom_char(b'"'));
    assert!(!is_atom_char(b'\\'));
}

#[tokio::test]
async fn test_read_command() {
    let data = b"a1 LOGIN user pass\r\n";
    let mut dec = AsyncDecoder::new(&data[..]);
    let cmd = dec.read_command().await.unwrap().unwrap();
    assert_eq!(cmd.tag, "a1");
    assert_eq!(cmd.verb, "LOGIN");
    assert!(!cmd.uid);
}

#[tokio::test]
async fn test_read_command_uid() {
    let data = b"a1 UID FETCH 1:5 FLAGS\r\n";
    let mut dec = AsyncDecoder::new(&data[..]);
    let cmd = dec.read_command().await.unwrap().unwrap();
    assert_eq!(cmd.tag, "a1");
    assert_eq!(cmd.verb, "FETCH");
    assert!(cmd.uid);
}

// ── Regression: Delta Chat prefetch parse ───────────────────────────
// Delta Chat sends: UID FETCH <set> (UID INTERNALDATE RFC822.SIZE
//   BODY.PEEK[HEADER.FIELDS (MESSAGE-ID DATE FROM ...)])
// The old read_atom() treated '[' as an atom char, so it greedily read
// `BODY.PEEK[HEADER.FIELDS` as one token and then failed with
// "expected atom" on the '(' of the field list — which desynchronized
// the stream and killed the IMAP session.

#[tokio::test]
async fn test_read_fetch_att_name_stops_at_bracket() {
    let data = b"BODY.PEEK[HEADER.FIELDS (MESSAGE-ID DATE)]";
    let mut dec = AsyncDecoder::new(&data[..]);
    // Keyword must stop at '[', leaving the section for the handler.
    let name = dec.read_fetch_att_name().await.unwrap();
    assert_eq!(name, "BODY.PEEK");
    let next = dec.read_byte().await.unwrap();
    assert_eq!(next, b'[', "section bracket must remain in the stream");
}

#[tokio::test]
async fn test_read_fetch_att_name_plain_keyword() {
    // Mimics parse_fetch_args: read keyword, then the handler consumes the
    // SP separator before the next keyword is read.
    let data = b"UID INTERNALDATE ";
    let mut dec = AsyncDecoder::new(&data[..]);
    assert_eq!(dec.read_fetch_att_name().await.unwrap(), "UID");
    dec.read_byte().await.unwrap(); // consume SP separator
    assert_eq!(dec.read_fetch_att_name().await.unwrap(), "INTERNALDATE");
}

#[tokio::test]
async fn test_full_deltachat_prefetch_parse() {
    // Exactly the shape Delta Chat sends (session.rs PREFETCH_FLAGS),
    // after read_command has consumed `a3 UID FETCH <set>`.
    let data = b"(UID INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER.FIELDS (MESSAGE-ID DATE FROM IN-REPLY-TO REFERENCES CHAT-VERSION )])\r\n";
    let mut dec = AsyncDecoder::new(&data[..]);

    // opening '(' of the attribute list
    assert_eq!(dec.read_byte().await.unwrap(), b'(');

    let attrs = ["UID", "INTERNALDATE", "RFC822.SIZE", "BODY.PEEK"];
    for expected in attrs {
        let name = dec.read_fetch_att_name().await.unwrap();
        assert_eq!(name, expected);
        // consume the SP separator (or the '[' that follows BODY.PEEK)
        let sep = dec.read_byte().await.unwrap();
        if expected == "BODY.PEEK" {
            assert_eq!(sep, b'[');
            // consume the section up to and including ']'
            loop {
                let b = dec.read_byte().await.unwrap();
                if b == b']' {
                    break;
                }
            }
        } else {
            assert_eq!(sep, b' ');
        }
    }

    // closing ')' of the attribute list, then CRLF
    assert_eq!(dec.read_byte().await.unwrap(), b')');
    dec.expect_crlf().await.unwrap();
}

// ── Regression: STORE/APPEND backslash flag parse ───────────────────
// `STORE +FLAGS (\Seen)` — read_atom() cannot read `\Seen` because
// '\' is excluded from atom-chars.

#[tokio::test]
async fn test_read_flag_backslash() {
    let data = b"\\Seen \\Recent ";
    let mut dec = AsyncDecoder::new(&data[..]);
    assert_eq!(dec.read_flag().await.unwrap(), "\\Seen");
    dec.read_byte().await.unwrap(); // consume SP
    assert_eq!(dec.read_flag().await.unwrap(), "\\Recent");
}

#[tokio::test]
async fn test_read_flag_keyword() {
    let data = b"$MDNSent Junk ";
    let mut dec = AsyncDecoder::new(&data[..]);
    assert_eq!(dec.read_flag().await.unwrap(), "$MDNSent");
    dec.read_byte().await.unwrap(); // consume SP
    assert_eq!(dec.read_flag().await.unwrap(), "Junk");
}

// ── Regression: per-command size budget must reset ──────────────────
// read_bytes accumulates every byte and max_size is meant to cap a SINGLE
// command, not the connection. Without a reset in read_command, a long
// session crosses the cap and every read_byte then fails — killing IDLE.

#[tokio::test]
async fn test_read_bytes_resets_per_command() {
    // Three commands whose cumulative bytes exceed a tiny per-command cap.
    // Each "aN NOOP\r\n" cycle reads ~9 bytes; cap = 12 fits one command
    // but not two cumulatively, so the 2nd command only parses if read_bytes
    // was reset at the start of read_command.
    let data = b"a1 NOOP\r\na2 NOOP\r\na3 NOOP\r\n";
    let mut dec = AsyncDecoder::new(&data[..]);
    dec.set_max_size(12);

    let c1 = dec.read_command().await.unwrap().unwrap();
    assert_eq!(c1.tag, "a1");
    dec.expect_crlf().await.unwrap();

    // Without the reset, cumulative bytes (>12) make this fail with
    // "max command size exceeded".
    let c2 = dec.read_command().await.unwrap().unwrap();
    assert_eq!(c2.tag, "a2");
    dec.expect_crlf().await.unwrap();

    let c3 = dec.read_command().await.unwrap().unwrap();
    assert_eq!(c3.tag, "a3");
}
