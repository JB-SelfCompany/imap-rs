use crate::types::SeqSet;

/// SEARCH criteria — simplified for the subset we need.
/// Full RFC 3501 SEARCH is complex; this covers what Thunderbird/DeltaChat use.
#[derive(Debug, Clone, Default)]
pub struct SearchCriteria {
    pub all: bool,
    pub uid_set: Option<String>,
    pub seq_set: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub text: Option<String>,
    pub before: Option<String>,
    pub since: Option<String>,
    pub smaller: Option<u64>,
    pub larger: Option<u64>,
    pub seen: Option<bool>,
    pub answered: Option<bool>,
    pub flagged: Option<bool>,
    pub deleted: Option<bool>,
    pub draft: Option<bool>,
    pub recent: Option<bool>,
    pub new: Option<bool>,
    pub old: Option<bool>,
    pub keyword: Option<String>,
    pub unkeyword: Option<String>,
    pub not: Option<Box<SearchCriteria>>,
    pub or: Option<(Box<SearchCriteria>, Box<SearchCriteria>)>,
    pub and: Vec<SearchCriteria>,
    pub unanswered: bool,
    pub undraft: bool,
    pub unflagged: bool,
    pub unkeyword_val: bool,
    pub unnew: bool,
    pub unold: bool,
    pub unrecent: bool,
    pub unseen: bool,
}

/// Options for SEARCH command.
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    pub return_min: bool,
    pub return_max: bool,
    pub return_all: bool,
    pub return_count: bool,
    pub return_save: bool,
}

/// Data returned by SEARCH.
#[derive(Debug, Clone, Default)]
pub struct SearchData {
    pub all: Option<SeqSet>,
    pub min: Option<u32>,
    pub max: Option<u32>,
    pub count: u32,
    pub modseq: Option<u64>,
}
