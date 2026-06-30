//! The resolved-row and module-info data shapes produced by symbolication.
//!
//! These are plain data: the worker crate maps them onto Arrow columns. One
//! machine address can correspond to several source-level functions when the
//! compiler inlined callees into the caller, so resolution emits **one
//! [`ResolvedFrame`] per (input frame × inline level)**, ordered innermost-first
//! with the physical frame last (highest `inline_depth`).

use crate::errors::ErrorKind;

/// The outcome of resolving one address against the cache.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ResolveStatus {
    /// Resolved to at least a function (line may still be absent).
    Ok,
    /// No module matched the build-id (or a negative-cache hit).
    #[default]
    NotFound,
    /// The address fell inside the module but had no line-table coverage.
    NoLine,
    /// The module that this frame needed was malformed.
    Error(ErrorKind),
}

impl ResolveStatus {
    /// The lowercase token emitted in the `status` column.
    pub fn as_status(&self) -> String {
        match self {
            ResolveStatus::Ok => "ok".to_string(),
            ResolveStatus::NotFound => "not_found".to_string(),
            ResolveStatus::NoLine => "no_line".to_string(),
            ResolveStatus::Error(k) => format!("error:{}", k.as_str()),
        }
    }
}

/// One resolved source-level frame for an input address (a physical frame or a
/// synthesized inline frame). `build_id`/`address` are echoed by the worker from
/// the input row and are not stored here.
#[derive(Debug, Clone, Default)]
pub struct ResolvedFrame {
    /// 0 = innermost inlined function … N = the real (physical) frame.
    pub inline_depth: i32,
    /// True for synthesized inline frames, false for the physical frame.
    pub is_inline: bool,
    /// Demangled function name (None if unknown).
    pub function: Option<String>,
    /// The mangled/linkage name as stored (None if unknown).
    pub function_raw: Option<String>,
    /// Source file path from the line table (None if unknown).
    pub file: Option<String>,
    /// 1-based source line (None if unknown).
    pub line: Option<u32>,
    /// Source column if the line program recorded one.
    pub column: Option<u32>,
    /// Debug file name that answered (None for unresolved frames).
    pub module: Option<String>,
    /// Normalized cache key (debug-id) that answered.
    pub debug_id: Option<String>,
    /// Per-frame resolution status.
    pub status: ResolveStatus,
}

impl ResolvedFrame {
    /// Build the single "no module matched" row for an address: one row,
    /// `status='not_found'`, all symbol columns empty. Resolve never drops a
    /// frame and never errors the scan, so an unsymbolicated frame is a row.
    pub fn not_found() -> ResolvedFrame {
        ResolvedFrame {
            inline_depth: 0,
            is_inline: false,
            status: ResolveStatus::NotFound,
            ..Default::default()
        }
    }

    /// Build the single error row for an address whose module was malformed.
    pub fn error(kind: ErrorKind, debug_id: Option<String>) -> ResolvedFrame {
        ResolvedFrame {
            inline_depth: 0,
            is_inline: false,
            debug_id,
            status: ResolveStatus::Error(kind),
            ..Default::default()
        }
    }
}

/// The container format of a debug file (the `format` column of `module_info`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// ELF (Linux/BSD) with DWARF.
    Elf,
    /// Mach-O (Apple) with DWARF.
    MachO,
    /// `.dSYM` bundle (Mach-O DWARF companion).
    DSym,
    /// PE/COFF (Windows executable/library).
    Pe,
    /// Windows PDB.
    Pdb,
    /// Breakpad text symbol file.
    Breakpad,
}

impl Format {
    /// The string emitted in the `format` column.
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Elf => "ELF",
            Format::MachO => "MachO",
            Format::DSym => "dSYM",
            Format::Pe => "PE",
            Format::Pdb => "PDB",
            Format::Breakpad => "Breakpad",
        }
    }
}

/// The triage view of a candidate debug file (one row of `module_info`).
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    /// Container format.
    pub format: Format,
    /// CPU architecture, e.g. `x86_64`, `aarch64`.
    pub arch: String,
    /// Raw per-format identifier (hex): ELF GNU build-id, Mach-O UUID, PDB GUID.
    pub build_id: Option<String>,
    /// Normalized cache key (debug-id).
    pub debug_id: Option<String>,
    /// Code-file identifier (PE timestamp+size / ELF build-id) for code lookup.
    pub code_id: Option<String>,
    /// Whether the file carries DWARF debug info.
    pub has_dwarf: bool,
    /// Whether the file is/has a PDB.
    pub has_pdb: bool,
    /// Whether a usable line table is present.
    pub has_line_table: bool,
    /// Number of function symbols.
    pub symbol_count: i64,
    /// Number of source files referenced.
    pub file_count: i64,
    /// Size of the debug file in bytes.
    pub byte_size: i64,
    /// The image's preferred load base (helps callers compute module-relative
    /// addresses; see the address-model contract).
    pub image_base: u64,
}
