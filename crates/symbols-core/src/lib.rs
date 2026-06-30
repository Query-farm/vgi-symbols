// Copyright 2026 Query Farm LLC - https://query.farm

//! Pure-compute native-symbolication core for the `vgi-symbols` VGI worker.
//!
//! This crate carries all of the symbolication logic with **no Arrow / VGI
//! dependencies**, so the worker crate stays a thin adapter and the engineering
//! that matters — debug-id derivation, DWARF/PDB resolution, and the
//! build-id-keyed debug-info cache — is unit-testable directly.
//!
//! # What it does
//!
//! Resolve a raw instruction address (a return address captured in a stack
//! frame) into the human-meaningful **function name + source file + line**, plus
//! the **inlined call chain** at that address, by parsing native debug info —
//! DWARF (ELF / Mach-O / dSYM) and Windows PDB — exactly as `addr2line` /
//! `llvm-symbolizer` do, but designed to back a **bulk SQL JOIN over a column of
//! `(build_id, address)` frames** through a **persistent, build-id-keyed cache**
//! ([`SymbolsState`]). A fleet of millions of frames symbolicates with each
//! debug module parsed exactly once.
//!
//! # Hardening
//!
//! Debug files are untrusted input. Every parse and resolve is fault-isolated
//! and bounded: a malformed ELF/Mach-O/PDB yields a clean
//! [`errors::ErrorKind`] (surfaced per-row as `status='error:<kind>'`), never a
//! panic, hang, or OOM. See [`module::Limits`].

pub mod cache;
pub mod demangle;
pub mod errors;
pub mod frame;
pub mod id;
pub mod module;
pub mod source;
pub mod state;

pub use cache::manifest::{CacheManifest, ManifestEntry, Origin, SourceSpec};
pub use cache::StatusRow;
pub use demangle::{demangle, DemangleLang};
pub use errors::{ErrorKind, SymError, SymResult};
pub use frame::{Format, ModuleInfo, ResolveStatus, ResolvedFrame};
pub use module::{Limits, ParsedModule};
pub use state::SymbolsState;

/// The crate version (also the worker's advertised version).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
