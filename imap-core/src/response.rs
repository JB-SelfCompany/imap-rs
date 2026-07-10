use std::fmt;

/// Response type: OK, NO, BAD, PREAUTH, BYE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusResponseType {
    Ok,
    No,
    Bad,
    PreAuth,
    Bye,
}

impl fmt::Display for StatusResponseType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ok => "OK",
            Self::No => "NO",
            Self::Bad => "BAD",
            Self::PreAuth => "PREAUTH",
            Self::Bye => "BYE",
        })
    }
}

/// Response code like [ALREADYEXISTS], [READ-WRITE], etc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseCode(pub String);

impl ResponseCode {
    pub const ALERT: &'static str = "ALERT";
    pub const ALREADYEXISTS: &'static str = "ALREADYEXISTS";
    pub const AUTHENTICATIONFAILED: &'static str = "AUTHENTICATIONFAILED";
    pub const CANNOT: &'static str = "CANNOT";
    pub const CLIENTBUG: &'static str = "CLIENTBUG";
    pub const CONTACTADMIN: &'static str = "CONTACTADMIN";
    pub const CORRUPTION: &'static str = "CORRUPTION";
    pub const EXPIRED: &'static str = "EXPIRED";
    pub const PRIVACYREQUIRED: &'static str = "PRIVACYREQUIRED";
    pub const READONLY: &'static str = "READ-ONLY";
    pub const READWRITE: &'static str = "READ-WRITE";
    pub const SERVERBUG: &'static str = "SERVERBUG";
    pub const TOOBIG: &'static str = "TOOBIG";
    pub const UIDNEXT: &'static str = "UIDNEXT";
    pub const UIDVALIDITY: &'static str = "UIDVALIDITY";
    pub const UNSEEN: &'static str = "UNSEEN";
    pub const TRYCREATE: &'static str = "TRYCREATE";
    pub const BADCHARSET: &'static str = "BADCHARSET";
    pub const HASCHILDREN: &'static str = "HASCHILDREN";
    pub const INUSE: &'static str = "INUSE";
    pub const LIMIT: &'static str = "LIMIT";
    pub const NONEXISTENT: &'static str = "NONEXISTENT";
    pub const NOPERM: &'static str = "NOPERM";
    pub const OVERQUOTA: &'static str = "OVERQUOTA";
    pub const PARSE: &'static str = "PARSE";
    pub const UNAVAILABLE: &'static str = "UNAVAILABLE";
    pub const UNKNOWNCTE: &'static str = "UNKNOWN-CTE";
    pub const NOPRIVATE: &'static str = "NOPRIVATE";
    pub const TOOMANY: &'static str = "TOOMANY";
    pub const AUTHORIZATIONFAILED: &'static str = "AUTHORIZATIONFAILED";
    pub const CLOSED: &'static str = "CLOSED";
}

impl fmt::Display for ResponseCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Tagged or untagged status response.
#[derive(Debug, Clone)]
pub struct StatusResponse {
    pub typ: StatusResponseType,
    pub code: Option<ResponseCode>,
    pub text: String,
}

impl StatusResponse {
    pub fn ok(text: impl Into<String>) -> Self {
        Self { typ: StatusResponseType::Ok, code: None, text: text.into() }
    }
    pub fn no(text: impl Into<String>) -> Self {
        Self { typ: StatusResponseType::No, code: None, text: text.into() }
    }
    pub fn bad(text: impl Into<String>) -> Self {
        Self { typ: StatusResponseType::Bad, code: None, text: text.into() }
    }
    pub fn bye(text: impl Into<String>) -> Self {
        Self { typ: StatusResponseType::Bye, code: None, text: text.into() }
    }
}
