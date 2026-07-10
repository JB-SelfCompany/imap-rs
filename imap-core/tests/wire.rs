//! Wire roundtrip tests for imap-core types.

use imap_core::types::*;
use imap_core::response::*;
use imap_core::fetch::*;
use imap_core::store::*;
use imap_core::search::*;
use imap_core::select::*;
use imap_core::bodystructure::*;

#[test]
fn test_seq_set_parse_and_wire() {
    let cases = vec![
        ("1", "1"),
        ("1:5", "1:5"),
        ("1,3,5", "1,3,5"),
        ("1:3,7:10", "1:3,7:10"),
    ];
    for (input, expected) in cases {
        let set = SeqSet::parse(input).unwrap();
        assert_eq!(set.to_wire(), expected);
    }
}

#[test]
fn test_seq_set_contains() {
    let set = SeqSet::parse("1:5,10:15").unwrap();
    assert!(set.contains(1));
    assert!(set.contains(5));
    assert!(set.contains(10));
    assert!(!set.contains(6));
    assert!(!set.contains(0));
}

#[test]
fn test_seq_set_add_range() {
    let mut set = SeqSet::parse("1:3").unwrap();
    set.add_range(5, 7);
    assert!(set.contains(1));
    assert!(set.contains(5));
    assert_eq!(set.to_wire(), "1:3,5:7");
}

#[test]
fn test_seq_set_add_num() {
    let mut set = SeqSet::default();
    set.add_num(5);
    set.add_num(3);
    set.add_num(7);
    assert_eq!(set.to_wire(), "3,5,7");
}

#[test]
fn test_seq_set_dynamic() {
    let set = SeqSet::parse("1:5").unwrap();
    assert!(!set.dynamic());
}

#[test]
fn test_seq_set_empty_is_err() {
    assert!(SeqSet::parse("").is_err());
}

#[test]
fn test_seq_set_nums() {
    let set = SeqSet::parse("1:3,7").unwrap();
    let nums = set.nums().unwrap();
    assert_eq!(nums, vec![1, 2, 3, 7]);
}

#[test]
fn test_flag_display() {
    assert_eq!(Flag::seen().to_string(), "\\Seen");
    assert_eq!(Flag::deleted().to_string(), "\\Deleted");
    assert_eq!(Flag::answered().to_string(), "\\Answered");
    assert_eq!(Flag::flagged().to_string(), "\\Flagged");
    assert_eq!(Flag::draft().to_string(), "\\Draft");
    assert_eq!(Flag::recent().to_string(), "\\Recent");
}

#[test]
fn test_cap_display() {
    assert_eq!(Cap::imap4rev1().to_string(), "IMAP4rev1");
    assert_eq!(Cap::starttls().to_string(), "STARTTLS");
    assert_eq!(Cap::idle().to_string(), "IDLE");
    assert_eq!(Cap::move_cap().to_string(), "MOVE");
    assert_eq!(Cap::namespace().to_string(), "NAMESPACE");
    assert_eq!(Cap::literal_plus().to_string(), "LITERAL+");
}

#[test]
fn test_cap_auth() {
    assert_eq!(Cap::auth("PLAIN").to_string(), "AUTH=PLAIN");
    assert_eq!(Cap::auth("LOGIN").to_string(), "AUTH=LOGIN");
}

#[test]
fn test_response_code_constants() {
    assert_eq!(ResponseCode::ALERT, "ALERT");
    assert_eq!(ResponseCode::AUTHENTICATIONFAILED, "AUTHENTICATIONFAILED");
    assert_eq!(ResponseCode::PRIVACYREQUIRED, "PRIVACYREQUIRED");
    assert_eq!(ResponseCode::CLOSED, "CLOSED");
    assert_eq!(ResponseCode::READONLY, "READ-ONLY");
    assert_eq!(ResponseCode::READWRITE, "READ-WRITE");
}

#[test]
fn test_section_specifier_wire() {
    assert_eq!(SectionSpecifier::None.to_wire(), "");
    assert_eq!(SectionSpecifier::Header.to_wire(), "HEADER");
    assert_eq!(SectionSpecifier::Text.to_wire(), "TEXT");
    assert_eq!(SectionSpecifier::Mime.to_wire(), "MIME");
    assert_eq!(
        SectionSpecifier::HeaderFields(vec!["From".into(), "To".into()]).to_wire(),
        "HEADER.FIELDS (From To)"
    );
    assert_eq!(
        SectionSpecifier::HeaderFieldsNot(vec!["Bcc".into()]).to_wire(),
        "HEADER.FIELDS.NOT (Bcc)"
    );
}

#[test]
fn test_store_op_parse() {
    assert_eq!(StoreOp::parse("+FLAGS").unwrap(), StoreOp::Add);
    assert_eq!(StoreOp::parse("-FLAGS").unwrap(), StoreOp::Remove);
    assert_eq!(StoreOp::parse("FLAGS").unwrap(), StoreOp::Replace);
    assert_eq!(StoreOp::parse("+FLAGS.SILENT").unwrap(), StoreOp::AddSilent);
    assert_eq!(StoreOp::parse("-FLAGS.SILENT").unwrap(), StoreOp::RemoveSilent);
    assert_eq!(StoreOp::parse("FLAGS.SILENT").unwrap(), StoreOp::ReplaceSilent);
    assert!(StoreOp::parse("INVALID").is_err());
}

#[test]
fn test_store_op_is_silent() {
    assert!(!StoreOp::Add.is_silent());
    assert!(!StoreOp::Remove.is_silent());
    assert!(!StoreOp::Replace.is_silent());
    assert!(StoreOp::AddSilent.is_silent());
    assert!(StoreOp::RemoveSilent.is_silent());
    assert!(StoreOp::ReplaceSilent.is_silent());
}

#[test]
fn test_mailbox_attr_constants() {
    assert_eq!(MailboxAttr::HAS_CHILDREN, "\\HasChildren");
    assert_eq!(MailboxAttr::HAS_NO_CHILDREN, "\\HasNoChildren");
    assert_eq!(MailboxAttr::SENT, "\\Sent");
    assert_eq!(MailboxAttr::TRASH, "\\Trash");
    assert_eq!(MailboxAttr::DRAFTS, "\\Drafts");
    assert_eq!(MailboxAttr::FLAGGED, "\\Flagged");
    assert_eq!(MailboxAttr::JUNK, "\\Junk");
    assert_eq!(MailboxAttr::ARCHIVE, "\\Archive");
    assert_eq!(MailboxAttr::ALL, "\\All");
}

#[test]
fn test_mailbox_attr_display() {
    let attr = MailboxAttr("\\HasChildren".into());
    assert_eq!(attr.to_string(), "\\HasChildren");
}

#[test]
fn test_envelope_default() {
    let env = Envelope::default();
    assert!(env.from.is_empty());
    assert!(env.to.is_empty());
    assert!(env.cc.is_empty());
    assert!(env.bcc.is_empty());
    assert!(env.subject.is_empty());
    assert!(env.date.is_empty());
    assert!(env.message_id.is_empty());
}

#[test]
fn test_address_methods() {
    let addr = Address {
        name: "Test".into(),
        mailbox: "user".into(),
        host: "example.com".into(),
    };
    assert_eq!(addr.addr(), "user@example.com");
    assert!(!addr.is_group_start());
    assert!(!addr.is_group_end());

    let group_start = Address {
        name: "".into(),
        mailbox: "group".into(),
        host: "".into(),
    };
    assert!(group_start.is_group_start());
    assert!(!group_start.is_group_end());
    assert_eq!(group_start.addr(), ""); // empty host -> empty addr

    let group_end = Address {
        name: "".into(),
        mailbox: "".into(),
        host: "".into(),
    };
    assert!(!group_end.is_group_start());
    assert!(group_end.is_group_end());
}

#[test]
fn test_conn_state() {
    assert_eq!(ConnState::NotAuthenticated, ConnState::NotAuthenticated);
    assert_ne!(ConnState::NotAuthenticated, ConnState::Authenticated);
    assert_ne!(ConnState::Authenticated, ConnState::Selected);
    assert_ne!(ConnState::Selected, ConnState::Logout);
}

#[test]
fn test_status_response_constructors() {
    let ok = StatusResponse::ok("All good");
    assert_eq!(ok.typ, StatusResponseType::Ok);
    assert_eq!(ok.text, "All good");
    assert!(ok.code.is_none());

    let no = StatusResponse::no("Denied");
    assert_eq!(no.typ, StatusResponseType::No);

    let bad = StatusResponse::bad("Syntax error");
    assert_eq!(bad.typ, StatusResponseType::Bad);

    let bye = StatusResponse::bye("Goodbye");
    assert_eq!(bye.typ, StatusResponseType::Bye);
}

#[test]
fn test_status_response_type_display() {
    assert_eq!(StatusResponseType::Ok.to_string(), "OK");
    assert_eq!(StatusResponseType::No.to_string(), "NO");
    assert_eq!(StatusResponseType::Bad.to_string(), "BAD");
    assert_eq!(StatusResponseType::PreAuth.to_string(), "PREAUTH");
    assert_eq!(StatusResponseType::Bye.to_string(), "BYE");
}

#[test]
fn test_response_code_display() {
    let code = ResponseCode("ALERT".into());
    assert_eq!(code.to_string(), "ALERT");
}

#[test]
fn test_search_criteria_default() {
    let c = SearchCriteria::default();
    assert!(!c.all);
    assert!(c.from.is_none());
    assert!(c.to.is_none());
    assert!(c.subject.is_none());
    assert!(c.not.is_none());
    assert!(c.or.is_none());
    assert!(c.and.is_empty());
}

#[test]
fn test_search_data_default() {
    let d = SearchData::default();
    assert!(d.all.is_none());
    assert!(d.min.is_none());
    assert!(d.max.is_none());
    assert_eq!(d.count, 0);
}

#[test]
fn test_status_data_default() {
    let d = StatusData::default();
    assert!(d.messages.is_none());
    assert!(d.recent.is_none());
    assert!(d.uid_next.is_none());
    assert!(d.uid_validity.is_none());
    assert!(d.unseen.is_none());
    assert!(d.size.is_none());
}

#[test]
fn test_fetch_options_default() {
    let f = FetchOptions::default();
    assert!(!f.uid);
    assert!(!f.flags);
    assert!(!f.envelope);
    assert!(f.body_sections.is_empty());
}

#[test]
fn test_body_structure_media_type() {
    let single = BodyStructure::SinglePart {
        typ: "text".into(),
        subtype: "plain".into(),
        params: Default::default(),
        id: String::new(),
        description: String::new(),
        encoding: "7bit".into(),
        size: 100,
        message_rfc822: None,
        text: None,
        extended: None,
    };
    assert_eq!(single.media_type(), "text/plain");

    let multi = BodyStructure::MultiPart {
        children: vec![],
        subtype: "mixed".into(),
        extended: None,
    };
    assert_eq!(multi.media_type(), "multipart/mixed");
}

#[test]
fn test_child_info() {
    let info = ChildInfo {
        has_children: true,
        has_no_children: false,
    };
    assert!(info.has_children);
    assert!(!info.has_no_children);
}

#[test]
fn test_namespace_descriptor() {
    let ns = NamespaceDescriptor {
        prefix: "".into(),
        delimiter: "/".into(),
    };
    assert_eq!(ns.prefix, "");
    assert_eq!(ns.delimiter, "/");
}
