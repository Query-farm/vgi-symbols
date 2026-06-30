//! Parsed debug modules: the rebuildable, in-RAM compute artifact.
//!
//! A [`ParsedModule`] is the expensive thing the cache exists to amortize: a
//! release `.debug`/PDB decoded into address-range → (function, file, line) maps
//! and an inline-subroutine tree. Resolving one address against it is cheap (a
//! binary search plus an inline-tree walk); parsing it is not. The cache keeps
//! the hot modules resident so a column of millions of frames pays O(modules ×
//! parse + frames × lookup), not O(frames × parse).
//!
//! Parsing is **fault-isolated and bounded** ([`Limits`]): a malformed or hostile
//! debug file yields a clean [`crate::errors::SymError`], never a panic, hang, or
//! OOM (see the hardening notes in the crate root).

mod dwarf;
mod pdb;

pub use dwarf::DwarfModule;
pub use pdb::PdbModule;

use crate::errors::{ErrorKind, SymError, SymResult};
use crate::frame::{ModuleInfo, ResolvedFrame};
use crate::id::Identity;

/// Bounded-allocation / bounded-recursion caps applied during parse and resolve
/// so a hostile debug file cannot OOM or stack-overflow the worker.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Hard cap on the debug file size accepted for parsing.
    pub max_parse_bytes: u64,
    /// Cap on the inline-subroutine chain length emitted for one address.
    pub max_inline_depth: usize,
    /// Cap on the number of compilation units walked when computing stats.
    pub max_units: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            // 2 GiB: large enough for a real `libchrome.so.debug`, small enough
            // that a single hostile module cannot blow a multi-GiB cache budget.
            max_parse_bytes: 2 * 1024 * 1024 * 1024,
            max_inline_depth: 256,
            max_units: 200_000,
        }
    }
}

/// The MSF 7.00 magic that opens every modern PDB.
const PDB_MAGIC: &[u8] = b"Microsoft C/C++ MSF 7.00\r\n\x1a\x44\x53\x00\x00\x00";

/// A parsed, resident debug module — either DWARF (ELF / Mach-O / dSYM) or PDB.
/// The variants are boxed because a `PdbModule` (which owns the whole parsed PDB)
/// is much larger than a `DwarfModule`.
pub enum ParsedModule {
    /// DWARF-backed (ELF, Mach-O, `.dSYM`).
    Dwarf(Box<DwarfModule>),
    /// PDB-backed (Windows).
    Pdb(Box<PdbModule>),
}

impl ParsedModule {
    /// Parse `data` (a whole debug file already in memory) into a resident
    /// module, dispatching on the container magic. `name` is the display name
    /// (the debug file's basename) surfaced in the `module` column.
    ///
    /// The whole parse is wrapped in [`std::panic::catch_unwind`]: the underlying
    /// DWARF/PDB libraries can `panic` (e.g. an arithmetic overflow) on
    /// adversarial input, and this worker's contract is that untrusted bytes
    /// yield a clean [`SymError`], never a crash.
    pub fn parse(data: Vec<u8>, name: String, limits: &Limits) -> SymResult<ParsedModule> {
        if data.len() as u64 > limits.max_parse_bytes {
            return Err(SymError::new(
                ErrorKind::AllocCap,
                format!("debug file {} bytes exceeds parse cap", data.len()),
            ));
        }
        let is_pdb = data.starts_with(PDB_MAGIC);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if is_pdb {
                PdbModule::parse(data, name, limits).map(|m| ParsedModule::Pdb(Box::new(m)))
            } else {
                DwarfModule::parse(data, name, limits).map(|m| ParsedModule::Dwarf(Box::new(m)))
            }
        }));
        match result {
            Ok(r) => r,
            Err(_) => Err(SymError::new(
                ErrorKind::CorruptLineProgram,
                "debug-info parser panicked on malformed input",
            )),
        }
    }

    /// Resolve a **module-relative** address to its inline-expanded frames,
    /// innermost-first with the physical frame last. Never panics; a malformed
    /// region (including a panic raised deep in the DWARF/PDB reader) yields a
    /// single `error:<kind>` frame, leaving every other frame unaffected.
    pub fn resolve(&self, address: u64, limits: &Limits) -> Vec<ResolvedFrame> {
        let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match self {
            ParsedModule::Dwarf(m) => m.resolve(address, limits),
            ParsedModule::Pdb(m) => m.resolve(address, limits),
        }));
        guarded.unwrap_or_else(|_| {
            vec![ResolvedFrame::error(
                ErrorKind::CorruptLineProgram,
                Some(self.identity().debug_id_str()),
            )]
        })
    }

    /// The innermost function name only (the fast path for `function_name`).
    pub fn function_name(&self, address: u64) -> Option<String> {
        let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match self {
            ParsedModule::Dwarf(m) => m.function_name(address),
            ParsedModule::Pdb(m) => m.function_name(address),
        }));
        guarded.unwrap_or(None)
    }

    /// This module's normalized identity.
    pub fn identity(&self) -> &Identity {
        match self {
            ParsedModule::Dwarf(m) => m.identity(),
            ParsedModule::Pdb(m) => m.identity(),
        }
    }

    /// The display name (debug file basename).
    pub fn name(&self) -> &str {
        match self {
            ParsedModule::Dwarf(m) => m.name(),
            ParsedModule::Pdb(m) => m.name(),
        }
    }

    /// Approximate resident byte footprint, for the LRU byte budget.
    pub fn resident_bytes(&self) -> u64 {
        match self {
            ParsedModule::Dwarf(m) => m.resident_bytes(),
            ParsedModule::Pdb(m) => m.resident_bytes(),
        }
    }

    /// The full triage view of this module (for `module_info`).
    pub fn info(&self) -> ModuleInfo {
        match self {
            ParsedModule::Dwarf(m) => m.info(),
            ParsedModule::Pdb(m) => m.info(),
        }
    }
}

/// Cheaply derive a debug file's normalized [`Identity`] from its bytes,
/// dispatching on the container magic — used to index a directory/glob of symbol
/// files by debug-id without paying the full resolve-artifact parse.
pub fn probe_identity(data: &[u8]) -> SymResult<Identity> {
    if data.starts_with(PDB_MAGIC) {
        return pdb::pdb_identity(data);
    }
    crate::id::identity_from_object(data)
}

/// Inspect a debug file's identity + capabilities **without** building the full
/// resident resolve artifact where possible — the `module_info` triage path.
/// Falls back to a full parse for stats that require it. Always total.
pub fn inspect(data: Vec<u8>, name: String, limits: &Limits) -> SymResult<ModuleInfo> {
    let module = ParsedModule::parse(data, name, limits)?;
    Ok(module.info())
}
