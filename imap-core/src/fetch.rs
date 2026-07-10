/// FETCH command options — what the client wants back.
#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    pub uid: bool,
    pub flags: bool,
    pub internal_date: bool,
    pub rfc822_size: bool,
    pub envelope: bool,
    pub body_structure: bool,
    pub body_sections: Vec<BodySection>,
    pub binary_sections: Vec<BinarySection>,
    pub binary_section_sizes: Vec<BinarySectionSize>,
    pub mod_seq: bool,
    pub changed_since: Option<u64>,
    pub body_structure_extended: bool,
}

/// BINARY section fetch item.
#[derive(Debug, Clone, Default)]
pub struct BinarySection {
    pub part: Vec<u32>,
    pub partial: Option<(u64, u64)>,
    pub peek: bool,
}

/// BINARY.SIZE section fetch item.
#[derive(Debug, Clone, Default)]
pub struct BinarySectionSize {
    pub part: Vec<u32>,
}

/// A BODY[section] or BODY.PEEK[section] request.
#[derive(Debug, Clone, Default)]
pub struct BodySection {
    /// Part path, e.g. `[1, 2]` for `BODY[1.2.TEXT]`.
    pub part: Vec<u32>,
    /// Section specifier: HEADER, TEXT, MIME, HEADER.FIELDS, etc.
    pub specifier: SectionSpecifier,
    /// Peek mode — don't set \Seen.
    pub peek: bool,
    /// Partial fetch: `<offset.size>`.
    pub partial: Option<(u64, u64)>,
}

#[derive(Debug, Clone, Default)]
pub enum SectionSpecifier {
    #[default]
    None,
    Header,
    HeaderFields(Vec<String>),
    HeaderFieldsNot(Vec<String>),
    Text,
    Mime,
}

impl SectionSpecifier {
    pub fn to_wire(&self) -> String {
        match self {
            Self::None => String::new(),
            Self::Header => "HEADER".into(),
            Self::HeaderFields(fields) => {
                format!("HEADER.FIELDS ({})", fields.join(" "))
            }
            Self::HeaderFieldsNot(fields) => {
                format!("HEADER.FIELDS.NOT ({})", fields.join(" "))
            }
            Self::Text => "TEXT".into(),
            Self::Mime => "MIME".into(),
        }
    }
}

/// Parse FETCH attribute name from wire (e.g. "FLAGS", "BODY[]", "BODY.PEEK[HEADER]").
pub fn parse_fetch_att(
    name: &str, rest: &str,
) -> Result<FetchAtt, String> {
    let upper = name.to_ascii_uppercase();
    match upper.as_str() {
        "ALL" => Ok(FetchAtt::Macro(FetchMacro::All)),
        "FAST" => Ok(FetchAtt::Macro(FetchMacro::Fast)),
        "FULL" => Ok(FetchAtt::Macro(FetchMacro::Full)),
        "FLAGS" => Ok(FetchAtt::Flags),
        "UID" => Ok(FetchAtt::Uid),
        "INTERNALDATE" => Ok(FetchAtt::InternalDate),
        "RFC822.SIZE" => Ok(FetchAtt::Rfc822Size),
        "ENVELOPE" => Ok(FetchAtt::Envelope),
        "BODYSTRUCTURE" => Ok(FetchAtt::BodyStructure),
        "BODY" => {
            if rest.starts_with('[') {
                parse_body_section(rest, false)
            } else {
                Ok(FetchAtt::BodyStructure) // BODY without [] = non-extended body structure
            }
        }
        "BODY.PEEK" => parse_body_section(rest, true),
        "RFC822" => Ok(FetchAtt::Rfc822),
        "RFC822.HEADER" => Ok(FetchAtt::Rfc822Header),
        "RFC822.TEXT" => Ok(FetchAtt::Rfc822Text),
        _ => Err(format!("unknown FETCH item: {name}")),
    }
}

#[derive(Debug)]
pub enum FetchAtt {
    Macro(FetchMacro),
    Flags,
    Uid,
    InternalDate,
    Rfc822Size,
    Envelope,
    BodyStructure,
    Rfc822,
    Rfc822Header,
    Rfc822Text,
    BodySection(BodySection),
}

#[derive(Debug)]
pub enum FetchMacro {
    All,
    Fast,
    Full,
}

fn parse_body_section(spec: &str, peek: bool) -> Result<FetchAtt, String> {
    // spec starts with '[' already consumed or is the full "[...]"
    let inner = spec.trim_start_matches('[').trim_end_matches(']');
    let mut section = BodySection { peek, ..Default::default() };

    if inner.is_empty() {
        return Ok(FetchAtt::BodySection(section));
    }

    // Parse part.number.specifier
    let (part_str, spec_str) = if let Some(dot) = inner.find(|c: char| !c.is_ascii_digit() && c != '.') {
        (&inner[..dot], &inner[dot..])
    } else {
        (inner, "")
    };

    // Parse part numbers
    if !part_str.is_empty() {
        for p in part_str.split('.') {
            if !p.is_empty() {
                section.part.push(p.parse().map_err(|_| format!("bad part: {p}"))?);
            }
        }
    }

    // Parse specifier
    let spec_upper = spec_str.trim_start_matches('.').to_ascii_uppercase();
    section.specifier = match spec_upper.as_str() {
        "" | "NONE" => SectionSpecifier::None,
        "HEADER" => SectionSpecifier::Header,
        "TEXT" => SectionSpecifier::Text,
        "MIME" => SectionSpecifier::Mime,
        s if s.starts_with("HEADER.FIELDS ") => {
            let fields = parse_field_list(&s["HEADER.FIELDS ".len()..])?;
            SectionSpecifier::HeaderFields(fields)
        }
        s if s.starts_with("HEADER.FIELDS.NOT ") => {
            let fields = parse_field_list(&s["HEADER.FIELDS.NOT ".len()..])?;
            SectionSpecifier::HeaderFieldsNot(fields)
        }
        _ => return Err(format!("bad section specifier: {spec_str}")),
    };

    Ok(FetchAtt::BodySection(section))
}

fn parse_field_list(s: &str) -> Result<Vec<String>, String> {
    let inner = s.trim().trim_start_matches('(').trim_end_matches(')');
    Ok(inner.split_whitespace().map(|f| f.to_string()).collect())
}
