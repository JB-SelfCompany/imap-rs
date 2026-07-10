use std::collections::HashMap;
use crate::types::Envelope;

/// Body structure disposition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BodyStructureDisposition {
    pub value: String,
    pub params: HashMap<String, String>,
}

/// Extended data for single-part body structures.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SinglePartExt {
    pub disposition: Option<BodyStructureDisposition>,
    pub language: Vec<String>,
    pub location: String,
}

/// Extended data for multi-part body structures.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MultiPartExt {
    pub params: HashMap<String, String>,
    pub disposition: Option<BodyStructureDisposition>,
    pub language: Vec<String>,
    pub location: String,
}

/// Metadata specific to message/rfc822 parts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BodyStructureMessageRFC822 {
    pub envelope: Option<Envelope>,
    pub body_structure: Option<Box<BodyStructure>>,
    pub num_lines: i64,
}

/// Metadata specific to text/* parts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BodyStructureText {
    pub num_lines: i64,
}

/// IMAP body structure (RFC 3501 BODYSTRUCTURE).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyStructure {
    SinglePart {
        typ: String,
        subtype: String,
        params: HashMap<String, String>,
        id: String,
        description: String,
        encoding: String,
        size: u32,
        message_rfc822: Option<BodyStructureMessageRFC822>,
        text: Option<BodyStructureText>,
        extended: Option<SinglePartExt>,
    },
    MultiPart {
        children: Vec<BodyStructure>,
        subtype: String,
        extended: Option<MultiPartExt>,
    },
}

impl BodyStructure {
    pub fn media_type(&self) -> String {
        match self {
            Self::SinglePart { typ, subtype, .. } => {
                format!("{}/{}", typ.to_lowercase(), subtype.to_lowercase())
            }
            Self::MultiPart { subtype, .. } => {
                format!("multipart/{}", subtype.to_lowercase())
            }
        }
    }
}
