use std::fmt;
use std::collections::BTreeSet;

/// IMAP message flag (e.g. `\Seen`, `\Answered`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Flag(pub String);

impl Flag {
    pub fn seen() -> Self { Self("\\Seen".into()) }
    pub fn answered() -> Self { Self("\\Answered".into()) }
    pub fn flagged() -> Self { Self("\\Flagged".into()) }
    pub fn deleted() -> Self { Self("\\Deleted".into()) }
    pub fn draft() -> Self { Self("\\Draft".into()) }
    pub fn recent() -> Self { Self("\\Recent".into()) }
}

impl fmt::Display for Flag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// UID — 32-bit unsigned, 0 is invalid per RFC 3501.
pub type UID = u32;

/// UID set — reuses SeqSet semantics. 0 represents *.
pub type UIDSet = SeqSet;

/// Sequence set: sorted non-overlapping `(start, end)` ranges.
/// `u32::MAX` represents `*` (infinity).
#[derive(Debug, Clone, Default)]
pub struct SeqSet(pub Vec<(u32, u32)>);

impl SeqSet {
    pub fn is_empty(&self) -> bool { self.0.is_empty() }

    /// Parse from wire format: "1", "1:5", "1:*", "1,3,5", "1:3,7:*"
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut ranges = Vec::new();
        for part in s.split(',') {
            let part = part.trim();
            if part.is_empty() { continue; }
            if let Some((a, b)) = part.split_once(':') {
                let start = parse_seq_num(a)?;
                let end = parse_seq_num(b)?;
                if start == 0 || end == 0 || start > end {
                    return Err(format!("invalid range: {part}"));
                }
                ranges.push((start, end));
            } else {
                let n = parse_seq_num(part)?;
                if n == 0 { return Err(format!("invalid seq: {part}")); }
                ranges.push((n, n));
            }
        }
        if ranges.is_empty() {
            return Err("empty sequence set".into());
        }
        Ok(Self(ranges))
    }

    /// Iterate all concrete numbers in range [1..max] — skips `*`.
    pub fn iter_numbers(&self, max: u32) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().flat_while(move |&(s, e)| {
            let end = if e == u32::MAX { max } else { e.min(max) };
            if s > max { None } else { Some(s..=end) }
        })
    }

    /// Wire format: "1:5,7,10:*"
    pub fn to_wire(&self) -> String {
        let parts: Vec<String> = self.0.iter().map(|&(s, e)| {
            if s == e {
                seq_to_str(s)
            } else {
                format!("{}:{}", seq_to_str(s), seq_to_str(e))
            }
        }).collect();
        parts.join(",")
    }

    pub fn contains(&self, n: u32) -> bool {
        self.0.iter().any(|&(s, e)| n >= s && n <= e)
    }

    pub fn add_num(&mut self, n: u32) {
        self.add_range(n, n);
    }

    pub fn add_range(&mut self, start: u32, end: u32) {
        if start == 0 || end == 0 || start > end {
            return;
        }
        let mut new_ranges = Vec::new();
        let mut merged = false;
        for &(s, e) in &self.0 {
            if merged {
                new_ranges.push((s, e));
                continue;
            }
            if start <= e + 1 && end + 1 >= s {
                new_ranges.push((start.min(s), end.max(e)));
                merged = true;
            } else if e < start {
                new_ranges.push((s, e));
            } else {
                new_ranges.push((start, end));
                new_ranges.push((s, e));
                merged = true;
            }
        }
        if !merged {
            new_ranges.push((start, end));
        }
        self.0 = new_ranges;
    }

    pub fn add_set(&mut self, other: &SeqSet) {
        for &(s, e) in &other.0 {
            self.add_range(s, e);
        }
    }

    pub fn dynamic(&self) -> bool {
        self.0.last().map_or(false, |&(_, e)| e == u32::MAX)
    }

    pub fn nums(&self) -> Option<Vec<u32>> {
        let mut nums = Vec::new();
        for &(s, e) in &self.0 {
            if e == u32::MAX {
                return None;
            }
            for n in s..=e {
                nums.push(n);
            }
        }
        Some(nums)
    }
}

fn seq_to_str(n: u32) -> String {
    if n == u32::MAX { "*".into() } else { n.to_string() }
}

fn parse_seq_num(s: &str) -> Result<u32, String> {
    if s == "*" { return Ok(u32::MAX); }
    s.parse::<u32>().map_err(|_| format!("invalid number: {s}"))
}

// ponytail: simple flat_while via stdlib
trait FlatWhile: Iterator {
    fn flat_while<U, F>(self, f: F) -> FlatWhileIter<Self, U, F>
    where
        Self: Sized,
        U: IntoIterator,
        F: FnMut(Self::Item) -> Option<U>,
    {
        FlatWhileIter { iter: self, f, current: None }
    }
}
impl<I: Iterator> FlatWhile for I {}

struct FlatWhileIter<I, U: IntoIterator, F> {
    iter: I,
    f: F,
    current: Option<U::IntoIter>,
}
impl<I, U, F> Iterator for FlatWhileIter<I, U, F>
where
    I: Iterator,
    U: IntoIterator,
    F: FnMut(I::Item) -> Option<U>,
{
    type Item = U::Item;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ref mut inner) = self.current {
                if let Some(item) = inner.next() {
                    return Some(item);
                }
            }
            let item = self.iter.next()?;
            match (self.f)(item) {
                Some(u) => self.current = Some(u.into_iter()),
                None => return None,
            }
        }
    }
}

/// IMAP address (RFC 3501).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Address {
    pub name: String,
    pub mailbox: String,
    pub host: String,
}

impl Address {
    pub fn addr(&self) -> String {
        if self.mailbox.is_empty() || self.host.is_empty() {
            String::new()
        } else {
            format!("{}@{}", self.mailbox, self.host)
        }
    }
    pub fn is_group_start(&self) -> bool {
        self.host.is_empty() && !self.mailbox.is_empty()
    }
    pub fn is_group_end(&self) -> bool {
        self.host.is_empty() && self.mailbox.is_empty()
    }
}

/// IMAP envelope structure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Envelope {
    pub date: String,
    pub subject: String,
    pub from: Vec<Address>,
    pub sender: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub in_reply_to: Vec<String>,
    pub message_id: String,
}

/// Mailbox attribute (RFC 9051 section 7.3.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MailboxAttr(pub String);

impl MailboxAttr {
    pub const NON_EXISTENT: &'static str = "\\NonExistent";
    pub const NO_INFERIORS: &'static str = "\\Noinferiors";
    pub const NO_SELECT: &'static str = "\\Noselect";
    pub const HAS_CHILDREN: &'static str = "\\HasChildren";
    pub const HAS_NO_CHILDREN: &'static str = "\\HasNoChildren";
    pub const MARKED: &'static str = "\\Marked";
    pub const UNMARKED: &'static str = "\\Unmarked";
    pub const SUBSCRIBED: &'static str = "\\Subscribed";
    pub const REMOTE: &'static str = "\\Remote";
    pub const ALL: &'static str = "\\All";
    pub const ARCHIVE: &'static str = "\\Archive";
    pub const DRAFTS: &'static str = "\\Drafts";
    pub const FLAGGED: &'static str = "\\Flagged";
    pub const JUNK: &'static str = "\\Junk";
    pub const SENT: &'static str = "\\Sent";
    pub const TRASH: &'static str = "\\Trash";
    pub const IMPORTANT: &'static str = "\\Important";
}

impl fmt::Display for MailboxAttr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// IMAP capability string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cap(pub String);

impl Cap {
    pub fn imap4rev1() -> Self { Self("IMAP4rev1".into()) }
    pub fn imap4rev2() -> Self { Self("IMAP4rev2".into()) }
    pub fn starttls() -> Self { Self("STARTTLS".into()) }
    pub fn login_disabled() -> Self { Self("LOGINDISABLED".into()) }
    pub fn idle() -> Self { Self("IDLE".into()) }
    pub fn move_cap() -> Self { Self("MOVE".into()) }
    pub fn namespace() -> Self { Self("NAMESPACE".into()) }
    pub fn literal_plus() -> Self { Self("LITERAL+".into()) }
    pub fn enable() -> Self { Self("ENABLE".into()) }
    pub fn uidplus() -> Self { Self("UIDPLUS".into()) }
    pub fn unselect() -> Self { Self("UNSELECT".into()) }
    pub fn esearch() -> Self { Self("ESEARCH".into()) }
    pub fn searchres() -> Self { Self("SEARCHRES".into()) }
    pub fn sasl_ir() -> Self { Self("SASL-IR".into()) }
    pub fn list_extended() -> Self { Self("LIST-EXTENDED".into()) }
    pub fn list_status() -> Self { Self("LIST-STATUS".into()) }
    pub fn literal_minus() -> Self { Self("LITERAL-".into()) }
    pub fn status_size() -> Self { Self("STATUS=SIZE".into()) }
    pub fn children() -> Self { Self("CHILDREN".into()) }
    pub fn acl() -> Self { Self("ACL".into()) }
    pub fn append_limit() -> Self { Self("APPENDLIMIT".into()) }
    pub fn binary() -> Self { Self("BINARY".into()) }
    pub fn condstore() -> Self { Self("CONDSTORE".into()) }
    pub fn qresync() -> Self { Self("QRESYNC".into()) }
    pub fn create_special_use() -> Self { Self("CREATE-SPECIAL-USE".into()) }
    pub fn metadata() -> Self { Self("METADATA".into()) }
    pub fn multi_append() -> Self { Self("MULTIAPPEND".into()) }
    pub fn notify() -> Self { Self("NOTIFY".into()) }
    pub fn quota() -> Self { Self("QUOTA".into()) }
    pub fn sort() -> Self { Self("SORT".into()) }
    pub fn special_use() -> Self { Self("SPECIAL-USE".into()) }
    pub fn unauthenticate() -> Self { Self("UNAUTHENTICATE".into()) }
    pub fn id() -> Self { Self("ID".into()) }
    pub fn utf8_accept() -> Self { Self("UTF8=ACCEPT".into()) }
    pub fn within() -> Self { Self("WITHIN".into()) }
    pub fn uid_only() -> Self { Self("UIDONLY".into()) }
    pub fn auth(mechanism: &str) -> Self { Self(format!("AUTH={mechanism}")) }
}

impl fmt::Display for Cap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub type CapSet = BTreeSet<Cap>;

/// Connection state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    NotAuthenticated,
    Authenticated,
    Selected,
    Logout,
}
