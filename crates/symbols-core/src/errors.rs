//! Malformed-input error kinds for the symbolication core.
//!
//! Debug files are **attacker-influenced** in the workloads this worker targets
//! (a crash corpus or malware sample's own symbols, a hostile minidump's
//! embedded modules). Parsing and resolution therefore never `panic`, never
//! abort the query, and never OOM: every failure is reduced to one of these
//! bounded [`ErrorKind`] values, which surface per-row as `status='error:<kind>'`
//! (see [`crate::frame::ResolveStatus`]). A malformed module poisons only the
//! frames that needed it — every other module and frame is unaffected.

use std::fmt;

/// A bounded classification of why a debug file could not be parsed or used.
///
/// The string form (`as_str`) is what callers see appended to `error:` in the
/// resolved-row `status` column. The set is closed on purpose: a fuzzer asserts
/// that no input can produce anything outside it (and that nothing panics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The file ended before a structure it declared could be read.
    Truncated,
    /// The container magic / signature was not a format we recognize.
    BadMagic,
    /// A build-id / debug-id field was missing or malformed.
    BadBuildId,
    /// The DWARF/PDB line program could not be decoded.
    CorruptLineProgram,
    /// The inline-subroutine tree was cyclic or otherwise unreadable.
    CorruptInlineTree,
    /// A recognized container with no format we can symbolicate.
    UnsupportedFormat,
    /// A declared nesting depth exceeded the configured cap.
    NestingLimit,
    /// A parse would have allocated past the configured byte cap.
    AllocCap,
}

impl ErrorKind {
    /// The stable lowercase token used in the `status` column after `error:`.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Truncated => "truncated",
            ErrorKind::BadMagic => "bad-magic",
            ErrorKind::BadBuildId => "bad-build-id",
            ErrorKind::CorruptLineProgram => "corrupt-line-program",
            ErrorKind::CorruptInlineTree => "corrupt-inline-tree",
            ErrorKind::UnsupportedFormat => "unsupported-format",
            ErrorKind::NestingLimit => "nesting-limit",
            ErrorKind::AllocCap => "alloc-cap",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A parse/resolution error: a bounded [`ErrorKind`] plus a human detail string.
#[derive(Debug, Clone)]
pub struct SymError {
    /// The bounded classification (drives the `status` column).
    pub kind: ErrorKind,
    /// Free-form context for logs (never attacker-controlled bytes verbatim).
    pub detail: String,
}

impl SymError {
    /// Build a [`SymError`] from a kind and detail message.
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> Self {
        SymError {
            kind,
            detail: detail.into(),
        }
    }

    /// The `status` value a frame needing this module should carry.
    pub fn status(&self) -> String {
        format!("error:{}", self.kind.as_str())
    }
}

impl fmt::Display for SymError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.detail)
    }
}

impl std::error::Error for SymError {}

/// A `Result` specialized to [`SymError`].
pub type SymResult<T> = std::result::Result<T, SymError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_strings_are_stable() {
        assert_eq!(ErrorKind::Truncated.as_str(), "truncated");
        assert_eq!(
            SymError::new(ErrorKind::AllocCap, "x").status(),
            "error:alloc-cap"
        );
    }
}
