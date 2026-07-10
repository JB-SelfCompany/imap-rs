/// STORE flag operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOp {
    /// +FLAGS — add flags
    Add,
    /// -FLAGS — remove flags
    Remove,
    /// FLAGS — replace flags
    Replace,
    /// +FLAGS.SILENT — add flags, no FETCH response
    AddSilent,
    /// -FLAGS.SILENT — remove flags, no FETCH response
    RemoveSilent,
    /// FLAGS.SILENT — replace flags, no FETCH response
    ReplaceSilent,
}

impl StoreOp {
    pub fn is_silent(&self) -> bool {
        matches!(self, Self::AddSilent | Self::RemoveSilent | Self::ReplaceSilent)
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_uppercase().as_str() {
            "+FLAGS" => Ok(Self::Add),
            "-FLAGS" => Ok(Self::Remove),
            "FLAGS" => Ok(Self::Replace),
            "+FLAGS.SILENT" => Ok(Self::AddSilent),
            "-FLAGS.SILENT" => Ok(Self::RemoveSilent),
            "FLAGS.SILENT" => Ok(Self::ReplaceSilent),
            _ => Err(format!("unknown STORE operation: {s}")),
        }
    }
}

/// Parsed STORE command data.
#[derive(Debug, Clone)]
pub struct StoreFlags {
    pub op: StoreOp,
    pub flags: Vec<String>,
}

/// Options for STORE command.
#[derive(Debug, Clone, Default)]
pub struct StoreOptions {
    pub unchanged_since: Option<u64>,
}
