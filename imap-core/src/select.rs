use crate::types::{Flag, MailboxAttr};

/// Data returned by SELECT/EXAMINE.
#[derive(Debug, Clone)]
pub struct SelectData {
    pub flags: Vec<Flag>,
    pub exists: u32,
    pub recent: u32,
    pub unseen: u32,
    pub uid_validity: u32,
    pub uid_next: u32,
    pub permanent_flags: Vec<Flag>,
    pub read_only: bool,
    pub first_unseen_seq_num: Option<u32>,
    pub list: Option<ListData>,
    pub highest_mod_seq: Option<u64>,
}

/// Data returned by STATUS.
#[derive(Debug, Clone, Default)]
pub struct StatusData {
    pub messages: Option<u32>,
    pub recent: Option<u32>,
    pub uid_next: Option<u32>,
    pub uid_validity: Option<u32>,
    pub unseen: Option<u32>,
    pub size: Option<u64>,
    pub deleted: Option<u32>,
    pub highest_mod_seq: Option<u64>,
}

/// Data returned by APPEND.
#[derive(Debug, Clone)]
pub struct AppendData {
    pub uid_validity: Option<u32>,
    pub uid: Option<u32>,
}

/// Data returned by COPY.
#[derive(Debug, Clone)]
pub struct CopyData {
    pub uid_validity: u32,
    pub source_uids: Vec<u32>,
    pub dest_uids: Vec<u32>,
}

/// Data returned by LIST.
#[derive(Debug, Clone)]
pub struct ListData {
    pub attrs: Vec<MailboxAttr>,
    pub delimiter: String,
    pub name: String,
    pub child_info: Option<ChildInfo>,
    pub old_name: Option<String>,
    pub status: Option<StatusData>,
}

#[derive(Debug, Clone)]
pub struct ChildInfo {
    pub has_children: bool,
    pub has_no_children: bool,
}

/// Data returned by NAMESPACE.
#[derive(Debug, Clone)]
pub struct NamespaceData {
    pub personal: Vec<NamespaceDescriptor>,
    pub other: Vec<NamespaceDescriptor>,
    pub shared: Vec<NamespaceDescriptor>,
}

#[derive(Debug, Clone)]
pub struct NamespaceDescriptor {
    pub prefix: String,
    pub delimiter: String,
}

/// Options for SELECT command.
#[derive(Debug, Clone, Default)]
pub struct SelectOptions {
    pub read_only: bool,
    pub cond_store: bool,
}

/// Options for STATUS command.
#[derive(Debug, Clone, Default)]
pub struct StatusOptions {
    pub messages: bool,
    pub recent: bool,
    pub uid_next: bool,
    pub uid_validity: bool,
    pub unseen: bool,
    pub size: bool,
    pub deleted: bool,
    pub highest_mod_seq: bool,
}

/// Options for LIST command.
#[derive(Debug, Clone, Default)]
pub struct ListOptions {
    pub select_subscribed: bool,
    pub select_remote: bool,
    pub select_recursion: bool,
    pub return_subscribed: bool,
    pub return_children: bool,
    pub return_status: Option<StatusOptions>,
}

/// Options for CREATE command.
#[derive(Debug, Clone, Default)]
pub struct CreateOptions {
    pub special_use: Vec<MailboxAttr>,
}

/// Options for RENAME command.
#[derive(Debug, Clone, Default)]
pub struct RenameOptions {}
