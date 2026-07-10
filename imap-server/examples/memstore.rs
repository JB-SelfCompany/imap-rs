//! In-memory IMAP backend for testing purposes.
//!
//! Usage: cargo run --example memstore [port]
//! Default port: 11443.
//! Login with any username/password.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use imap_core::error::{ImapError, ImapResult};
use imap_core::fetch::{BodySection, FetchOptions, SectionSpecifier};
use imap_core::search::SearchCriteria;
use imap_core::select::{AppendData, CopyData, ListData, SelectData, StatusData};
use imap_core::store::StoreFlags;
use imap_core::types::{Flag, MailboxAttr, SeqSet};

use imap_server::backend::{Backend, ConnInfo, FetchedMessage, StoredMessage, UserSession};
use imap_server::tracker::MailboxTracker;
use imap_server::Server;

// ── Date helpers ────────────────────────────────────────────────────────

fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn epoch_days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let days_in_month = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &dim in &days_in_month {
        if days < dim {
            break;
        }
        days -= dim;
        month += 1;
    }
    (year, month, days + 1)
}

fn now_as_imap_date() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days_since_epoch = now / 86400;
    let (year, month, day) = epoch_days_to_ymd(days_since_epoch);
    let seconds_of_day = now % 86400;
    let hour = seconds_of_day / 3600;
    let minute = (seconds_of_day % 3600) / 60;
    let second = seconds_of_day % 60;
    let month_names = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{:02}-{}-{} {:02}:{:02}:{:02} +0000",
        day,
        month_names[(month - 1) as usize],
        year,
        hour,
        minute,
        second
    )
}

// ── Body section filtering ──────────────────────────────────────────────

fn filter_body_section(body: &[u8], section: &BodySection) -> Vec<u8> {
    if section.part.is_empty() && matches!(section.specifier, SectionSpecifier::None) {
        return body.to_vec();
    }

    match &section.specifier {
        SectionSpecifier::Header => {
            if let Some(pos) = find_header_end(body) {
                body[..pos].to_vec()
            } else {
                body.to_vec()
            }
        }
        SectionSpecifier::Text => {
            if let Some(pos) = find_header_end(body) {
                body[pos..].to_vec()
            } else {
                Vec::new()
            }
        }
        SectionSpecifier::HeaderFields(fields) => {
            let headers = if let Some(pos) = find_header_end(body) {
                &body[..pos]
            } else {
                body
            };
            let mut result = Vec::new();
            for line in headers.split(|&b| b == b'\n') {
                let line = if line.last() == Some(&b'\r') {
                    &line[..line.len() - 1]
                } else {
                    line
                };
                let line_str = String::from_utf8_lossy(line);
                for field in fields {
                    if line_str.to_lowercase().starts_with(&field.to_lowercase()) {
                        result.extend_from_slice(line);
                        result.extend_from_slice(b"\r\n");
                        break;
                    }
                }
            }
            result.extend_from_slice(b"\r\n");
            result
        }
        SectionSpecifier::HeaderFieldsNot(fields) => {
            let headers = if let Some(pos) = find_header_end(body) {
                &body[..pos]
            } else {
                body
            };
            let mut result = Vec::new();
            for line in headers.split(|&b| b == b'\n') {
                let line = if line.last() == Some(&b'\r') {
                    &line[..line.len() - 1]
                } else {
                    line
                };
                let line_str = String::from_utf8_lossy(line);
                let mut excluded = false;
                for field in fields {
                    if line_str.to_lowercase().starts_with(&field.to_lowercase()) {
                        excluded = true;
                        break;
                    }
                }
                if !excluded {
                    result.extend_from_slice(line);
                    result.extend_from_slice(b"\r\n");
                }
            }
            result.extend_from_slice(b"\r\n");
            result
        }
        SectionSpecifier::Mime | SectionSpecifier::None => body.to_vec(),
    }
}

fn find_header_end(body: &[u8]) -> Option<usize> {
    // Look for \r\n\r\n
    for i in 0..body.len().saturating_sub(3) {
        if body[i] == b'\r'
            && body[i + 1] == b'\n'
            && body[i + 2] == b'\r'
            && body[i + 3] == b'\n'
        {
            return Some(i + 4);
        }
    }
    None
}

// ── Search matching ─────────────────────────────────────────────────────

fn match_criteria(msg: &MemMessage, seq: u32, criteria: &SearchCriteria) -> bool {
    let mut matched = criteria.all;

    // UID set
    if let Some(ref set) = criteria.uid_set {
        if let Ok(seq_set) = SeqSet::parse(set) {
            matched = matched && seq_set.contains(msg.uid);
        }
    }

    // Sequence set
    if let Some(ref set) = criteria.seq_set {
        if let Ok(seq_set) = SeqSet::parse(set) {
            matched = matched && seq_set.contains(seq);
        }
    }

    // Flag-based criteria
    if criteria.unseen {
        matched = matched && !msg.flags.iter().any(|f| f.0 == "\\Seen");
    }
    if let Some(true) = criteria.seen {
        matched = matched && msg.flags.iter().any(|f| f.0 == "\\Seen");
    }
    if let Some(true) = criteria.answered {
        matched = matched && msg.flags.iter().any(|f| f.0 == "\\Answered");
    }
    if let Some(true) = criteria.flagged {
        matched = matched && msg.flags.iter().any(|f| f.0 == "\\Flagged");
    }
    if let Some(true) = criteria.deleted {
        matched = matched && msg.flags.iter().any(|f| f.0 == "\\Deleted");
    }
    if let Some(true) = criteria.draft {
        matched = matched && msg.flags.iter().any(|f| f.0 == "\\Draft");
    }
    if let Some(true) = criteria.recent {
        matched = matched && msg.flags.iter().any(|f| f.0 == "\\Recent");
    }
    if criteria.unanswered {
        matched = matched && !msg.flags.iter().any(|f| f.0 == "\\Answered");
    }
    if criteria.unflagged {
        matched = matched && !msg.flags.iter().any(|f| f.0 == "\\Flagged");
    }
    if criteria.undraft {
        matched = matched && !msg.flags.iter().any(|f| f.0 == "\\Draft");
    }

    // Text search
    let body_str = String::from_utf8_lossy(&msg.body);
    if let Some(ref from) = criteria.from {
        matched = matched && body_str.to_lowercase().contains(&from.to_lowercase());
    }
    if let Some(ref to) = criteria.to {
        matched = matched && body_str.to_lowercase().contains(&to.to_lowercase());
    }
    if let Some(ref subject) = criteria.subject {
        matched = matched && body_str.to_lowercase().contains(&subject.to_lowercase());
    }
    if let Some(ref body_text) = criteria.body {
        matched = matched && body_str.to_lowercase().contains(&body_text.to_lowercase());
    }
    if let Some(ref text) = criteria.text {
        matched = matched && body_str.to_lowercase().contains(&text.to_lowercase());
    }

    // Size criteria
    if let Some(smaller) = criteria.smaller {
        matched = matched && (msg.body.len() as u64) < smaller;
    }
    if let Some(larger) = criteria.larger {
        matched = matched && (msg.body.len() as u64) > larger;
    }

    // NOT
    if let Some(ref not) = criteria.not {
        matched = matched && !match_criteria(msg, seq, not);
    }

    // OR
    if let Some((ref a, ref b)) = criteria.or {
        matched = matched && (match_criteria(msg, seq, a) || match_criteria(msg, seq, b));
    }

    // AND
    for sub in &criteria.and {
        matched = matched && match_criteria(msg, seq, sub);
    }

    matched
}

// ── In-memory mail state ────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct MemMessage {
    uid: u32,
    flags: Vec<Flag>,
    internal_date: String,
    body: Vec<u8>,
}

#[derive(Debug, Clone)]
struct MemMailbox {
    messages: Vec<MemMessage>,
    uid_next: u32,
    uid_validity: u32,
    tracker: Arc<MailboxTracker>,
}

impl MemMailbox {
    fn new() -> Self {
        Self {
            messages: Vec::new(),
            uid_next: 1,
            uid_validity: 1,
            tracker: MailboxTracker::new(0),
        }
    }
}

// ── Session ─────────────────────────────────────────────────────────────

struct MemSession {
    mailboxes: HashMap<String, MemMailbox>,
    selected: Option<String>,
    uid_counter: Arc<AtomicU32>,
}

#[async_trait]
impl UserSession for MemSession {
    async fn list(&mut self, _reference: &str, _pattern: &str) -> Result<Vec<ListData>, ImapError> {
        let mut results = Vec::new();
        for name in self.mailboxes.keys() {
            results.push(ListData {
                attrs: vec![MailboxAttr("\\HasNoChildren".to_string())],
                delimiter: "/".to_string(),
                name: name.clone(),
                child_info: None,
                old_name: None,
                status: None,
            });
        }
        if results.is_empty() {
            results.push(ListData {
                attrs: vec![MailboxAttr("\\HasNoChildren".to_string())],
                delimiter: "/".to_string(),
                name: "INBOX".to_string(),
                child_info: None,
                old_name: None,
                status: None,
            });
        }
        Ok(results)
    }

    async fn subscribe(&mut self, _mailbox: &str) -> ImapResult<()> {
        Ok(())
    }
    async fn unsubscribe(&mut self, _mailbox: &str) -> ImapResult<()> {
        Ok(())
    }

    async fn create(&mut self, mailbox: &str) -> ImapResult<()> {
        self.mailboxes
            .entry(mailbox.to_string())
            .or_insert_with(MemMailbox::new);
        Ok(())
    }

    async fn delete(&mut self, mailbox: &str) -> ImapResult<()> {
        self.mailboxes.remove(mailbox);
        Ok(())
    }

    async fn rename(&mut self, from: &str, to: &str) -> ImapResult<()> {
        if let Some(mb) = self.mailboxes.remove(from) {
            self.mailboxes.insert(to.to_string(), mb);
        }
        Ok(())
    }

    async fn status(&mut self, mailbox: &str) -> Result<StatusData, ImapError> {
        match self.mailboxes.get(mailbox) {
            Some(mb) => Ok(StatusData {
                messages: Some(mb.messages.len() as u32),
                recent: Some(0),
                uid_next: Some(mb.uid_next),
                uid_validity: Some(mb.uid_validity),
                unseen: Some(
                    mb.messages
                        .iter()
                        .filter(|m| !m.flags.iter().any(|f| f.0 == "\\Seen"))
                        .count() as u32,
                ),
                size: None,
                deleted: Some(
                    mb.messages
                        .iter()
                        .filter(|m| m.flags.iter().any(|f| f.0 == "\\Deleted"))
                        .count() as u32,
                ),
                highest_mod_seq: None,
            }),
            None => Err(ImapError::no("Mailbox does not exist")),
        }
    }

    async fn select(
        &mut self,
        mailbox: &str,
        _read_only: bool,
    ) -> Result<SelectData, ImapError> {
        match self.mailboxes.get(mailbox) {
            Some(mb) => {
                let unseen_count = mb
                    .messages
                    .iter()
                    .filter(|m| !m.flags.iter().any(|f| f.0 == "\\Seen"))
                    .count() as u32;
                let first_unseen_seq = mb
                    .messages
                    .iter()
                    .enumerate()
                    .find(|(_, m)| !m.flags.iter().any(|f| f.0 == "\\Seen"))
                    .map(|(i, _)| (i + 1) as u32);
                self.selected = Some(mailbox.to_string());
                Ok(SelectData {
                    flags: vec![
                        Flag::seen(),
                        Flag::answered(),
                        Flag::flagged(),
                        Flag::deleted(),
                        Flag::draft(),
                        Flag::recent(),
                    ],
                    exists: mb.messages.len() as u32,
                    recent: 0,
                    unseen: unseen_count,
                    uid_validity: mb.uid_validity,
                    uid_next: mb.uid_next,
                    permanent_flags: vec![
                        Flag::seen(),
                        Flag::answered(),
                        Flag::flagged(),
                        Flag::deleted(),
                        Flag::draft(),
                    ],
                    read_only: _read_only,
                    first_unseen_seq_num: first_unseen_seq,
                    list: None,
                    highest_mod_seq: None,
                })
            }
            None => Err(ImapError::no("Mailbox does not exist")),
        }
    }

    async fn close(&mut self) -> ImapResult<()> {
        self.selected = None;
        Ok(())
    }

    async fn fetch(
        &mut self,
        _uid: bool,
        _seq_set: &SeqSet,
        _options: &FetchOptions,
    ) -> Result<Vec<FetchedMessage>, ImapError> {
        let mailbox = self
            .selected
            .as_ref()
            .ok_or_else(|| ImapError::bad("No mailbox selected"))?;
        let mb = self
            .mailboxes
            .get(mailbox)
            .ok_or_else(|| ImapError::no("Mailbox deleted"))?;

        let mut results = Vec::new();
        let ids: Vec<(u32, &MemMessage)> = if _uid {
            mb.messages
                .iter()
                .filter(|m| _seq_set.contains(m.uid))
                .map(|m| (m.uid, m))
                .collect()
        } else {
            mb.messages
                .iter()
                .enumerate()
                .filter(|(i, _)| _seq_set.contains((*i as u32) + 1))
                .map(|(i, m)| ((i as u32) + 1, m))
                .collect()
        };

        for (seq, msg) in ids {
            // Filter body sections if requested
            let body = if _options.body_sections.is_empty() {
                msg.body.clone()
            } else {
                let mut filtered = Vec::new();
                for section in &_options.body_sections {
                    filtered = filter_body_section(&msg.body, section);
                }
                filtered
            };

            results.push(FetchedMessage {
                seq,
                uid: msg.uid,
                flags: msg.flags.clone(),
                internal_date: msg.internal_date.clone(),
                rfc822_size: msg.body.len() as u32,
                body,
            });
        }
        Ok(results)
    }

    async fn store(
        &mut self,
        _uid: bool,
        _seq_set: &SeqSet,
        flags: &StoreFlags,
    ) -> Result<Vec<StoredMessage>, ImapError> {
        let mailbox = self
            .selected
            .as_ref()
            .ok_or_else(|| ImapError::bad("No mailbox selected"))?;
        let mb = self
            .mailboxes
            .get_mut(mailbox)
            .ok_or_else(|| ImapError::no("Mailbox deleted"))?;

        let req_flags: Vec<Flag> = flags.flags.iter().map(|s| Flag(s.clone())).collect();
        let mut results = Vec::new();

        for (i, msg) in mb.messages.iter_mut().enumerate() {
            let seq = (i + 1) as u32;
            let matches = if _uid {
                _seq_set.contains(msg.uid)
            } else {
                _seq_set.contains(seq)
            };
            if !matches {
                continue;
            }

            match flags.op {
                imap_core::store::StoreOp::Add | imap_core::store::StoreOp::AddSilent => {
                    for f in &req_flags {
                        if !msg.flags.contains(f) {
                            msg.flags.push(f.clone());
                        }
                    }
                }
                imap_core::store::StoreOp::Remove | imap_core::store::StoreOp::RemoveSilent => {
                    msg.flags.retain(|f| !req_flags.contains(f));
                }
                imap_core::store::StoreOp::Replace | imap_core::store::StoreOp::ReplaceSilent => {
                    msg.flags = req_flags.clone();
                }
            }
            results.push(StoredMessage {
                seq,
                uid: msg.uid,
                flags: msg.flags.clone(),
            });
        }
        Ok(results)
    }

    async fn search(
        &mut self,
        _uid: bool,
        criteria: &SearchCriteria,
    ) -> Result<Vec<u32>, ImapError> {
        let mailbox = self
            .selected
            .as_ref()
            .ok_or_else(|| ImapError::bad("No mailbox selected"))?;
        let mb = self
            .mailboxes
            .get(mailbox)
            .ok_or_else(|| ImapError::no("Mailbox deleted"))?;

        let mut results = Vec::new();
        for (i, msg) in mb.messages.iter().enumerate() {
            let seq = (i + 1) as u32;
            let id = if _uid { msg.uid } else { seq };
            if match_criteria(msg, seq, criteria) {
                results.push(id);
            }
        }
        Ok(results)
    }

    async fn copy(
        &mut self,
        _uid: bool,
        _seq_set: &SeqSet,
        dest: &str,
    ) -> Result<CopyData, ImapError> {
        let src_name = self
            .selected
            .as_ref()
            .ok_or_else(|| ImapError::bad("No mailbox selected"))?;

        // Clone source data first to avoid double borrow
        let src_msgs: Vec<MemMessage> = {
            let src = self
                .mailboxes
                .get(src_name)
                .ok_or_else(|| ImapError::no("Source mailbox deleted"))?;
            if _uid {
                src.messages
                    .iter()
                    .filter(|m| _seq_set.contains(m.uid))
                    .cloned()
                    .collect()
            } else {
                src.messages
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| _seq_set.contains((*i as u32) + 1))
                    .map(|(_, m)| m.clone())
                    .collect()
            }
        };

        let dest_mb = self
            .mailboxes
            .get_mut(dest)
            .ok_or_else(|| ImapError::no("Destination mailbox does not exist"))?;

        let mut source_uids = Vec::new();
        let mut dest_uids = Vec::new();

        for mut msg in src_msgs {
            let new_uid = self.uid_counter.fetch_add(1, Ordering::SeqCst);
            source_uids.push(msg.uid);
            msg.uid = new_uid;
            dest_uids.push(new_uid);
            dest_mb.messages.push(msg);
        }

        Ok(CopyData {
            uid_validity: dest_mb.uid_validity,
            source_uids,
            dest_uids,
        })
    }

    async fn expunge(&mut self, _uid_set: Option<&SeqSet>) -> Result<Vec<u32>, ImapError> {
        let mailbox = self
            .selected
            .clone()
            .ok_or_else(|| ImapError::bad("No mailbox selected"))?;
        let mb = self
            .mailboxes
            .get_mut(&mailbox)
            .ok_or_else(|| ImapError::no("Mailbox deleted"))?;

        let mut expunged = Vec::new();
        let mut i = 0;
        mb.messages.retain(|msg| {
            i += 1;
            let is_deleted = msg.flags.iter().any(|f| f.0 == "\\Deleted");
            let should_remove = if let Some(uid_set) = _uid_set {
                is_deleted && uid_set.contains(msg.uid)
            } else {
                is_deleted
            };
            if should_remove {
                expunged.push(i);
            }
            !should_remove
        });

        Ok(expunged)
    }

    async fn append(
        &mut self,
        mailbox: &str,
        data: Vec<u8>,
        flags: Option<Vec<Flag>>,
        _date: Option<String>,
    ) -> Result<AppendData, ImapError> {
        let mb = self
            .mailboxes
            .get_mut(mailbox)
            .ok_or_else(|| ImapError::no("Mailbox does not exist"))?;

        let uid = mb.uid_next;
        mb.uid_next += 1;

        let msg = MemMessage {
            uid,
            flags: flags.unwrap_or_default(),
            internal_date: _date.unwrap_or_else(now_as_imap_date),
            body: data,
        };
        mb.messages.push(msg);

        Ok(AppendData {
            uid_validity: Some(mb.uid_validity),
            uid: Some(uid),
        })
    }
}

// ── Backend ─────────────────────────────────────────────────────────────

struct MemBackend {
    mailboxes: Arc<Mutex<HashMap<String, MemMailbox>>>,
    uid_counter: Arc<AtomicU32>,
}

#[async_trait]
impl Backend for MemBackend {
    async fn login(
        &self,
        _conn: &ConnInfo,
        _username: &str,
        _password: &str,
    ) -> Result<Box<dyn UserSession>, ImapError> {
        let mailboxes = self.mailboxes.lock().await.clone();
        Ok(Box::new(MemSession {
            mailboxes,
            selected: None,
            uid_counter: self.uid_counter.clone(),
        }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(11443);

    let mut mailboxes = HashMap::new();
    let mut inbox = MemMailbox::new();

    for i in 1..=5u32 {
        inbox.messages.push(MemMessage {
            uid: i,
            flags: if i == 1 { vec![] } else { vec![Flag::seen()] },
            internal_date: format!("{:02}-Jun-2025 12:00:00 +0000", i),
            body: format!(
                "From: test{i}@example.com\r\nSubject: Test Message {i}\r\n\r\nThis is test message {i}.\r\n"
            )
            .into_bytes(),
        });
        inbox.uid_next = i + 1;
    }
    mailboxes.insert("INBOX".to_string(), inbox);

    let backend = MemBackend {
        mailboxes: Arc::new(Mutex::new(mailboxes)),
        uid_counter: Arc::new(AtomicU32::new(100)),
    };

    let server = Server::with_defaults(backend);
    println!("Starting in-memory IMAP server on 127.0.0.1:{port}");
    println!("Login with any username/password");
    server.listen(("127.0.0.1", port)).await
}
