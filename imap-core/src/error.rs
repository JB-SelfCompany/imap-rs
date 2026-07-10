use std::fmt;
use std::error::Error;
use std::io;

/// IMAP protocol error — maps to tagged NO/BAD responses.
#[derive(Debug)]
pub enum ImapError {
    /// Tagged NO response (operation rejected).
    No { code: Option<String>, text: String },
    /// Tagged BAD response (protocol/syntax error).
    Bad { code: Option<String>, text: String },
    /// Internal error → * NO [SERVERBUG]
    Internal(Box<dyn Error + Send + Sync>),
    /// Close the connection.
    Closed,
}

impl ImapError {
    pub fn no(text: impl Into<String>) -> Self {
        Self::No { code: None, text: text.into() }
    }
    pub fn no_code(code: impl Into<String>, text: impl Into<String>) -> Self {
        Self::No { code: Some(code.into()), text: text.into() }
    }
    pub fn bad(text: impl Into<String>) -> Self {
        Self::Bad { code: None, text: text.into() }
    }
    pub fn bad_code(code: impl Into<String>, text: impl Into<String>) -> Self {
        Self::Bad { code: Some(code.into()), text: text.into() }
    }
    pub fn client_bug(text: impl Into<String>) -> Self {
        Self::Bad { code: Some("CLIENTBUG".into()), text: text.into() }
    }
}

impl fmt::Display for ImapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::No { text, .. } => write!(f, "NO {text}"),
            Self::Bad { text, .. } => write!(f, "BAD {text}"),
            Self::Internal(e) => write!(f, "internal: {e}"),
            Self::Closed => write!(f, "connection closed"),
        }
    }
}

impl Error for ImapError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Internal(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<io::Error> for ImapError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            Self::Closed
        } else {
            Self::Internal(Box::new(e))
        }
    }
}

/// Result alias for IMAP operations.
pub type ImapResult<T> = Result<T, ImapError>;
