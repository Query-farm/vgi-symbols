//! DWARF resolution for ELF / Mach-O / `.dSYM` via `object` + `gimli` +
//! `addr2line`.
//!
//! Sections are loaded into `Arc`-backed gimli readers ([`gimli::EndianArcSlice`])
//! so the resulting [`addr2line::Context`] **owns** its data (no borrowed mmap)
//! and is therefore `Send + Sync` — it can live in the shared, resident cache
//! across worker threads. A flat, address-sorted symbol table is kept alongside
//! for the `function_name` fast path and as a fallback when an address has no
//! line-table coverage.

use std::borrow::Cow;
use std::sync::Arc;

use gimli::{EndianArcSlice, RunTimeEndian};
use object::read::{Object, ObjectSection, ObjectSymbol};
use object::SymbolKind;

use crate::demangle::try_demangle;
use crate::errors::{ErrorKind, SymError, SymResult};
use crate::frame::{ModuleInfo, ResolveStatus, ResolvedFrame};
use crate::id::{identity_from_object, Identity};

use super::Limits;

/// The owned gimli reader the DWARF context is built over.
type Reader = EndianArcSlice<RunTimeEndian>;

/// One raw frame from the DWARF reader: (raw function name, file, line, column).
type RawFrame = (Option<String>, Option<String>, Option<u32>, Option<u32>);

/// A parsed DWARF module held resident in the cache.
pub struct DwarfModule {
    ctx: addr2line::Context<Reader>,
    identity: Identity,
    name: String,
    /// Address-sorted `(start_address, demangled_name)` of function symbols.
    symbols: Vec<(u64, String)>,
    file_count: i64,
    has_line_table: bool,
    byte_size: i64,
}

impl DwarfModule {
    /// Parse a DWARF-bearing image (its whole bytes) into a resident module.
    pub fn parse(data: Vec<u8>, name: String, limits: &Limits) -> SymResult<DwarfModule> {
        let byte_size = data.len() as i64;
        let identity = identity_from_object(&data)?;
        let obj = object::File::parse(&*data)
            .map_err(|e| SymError::new(ErrorKind::Truncated, format!("parse: {e}")))?;

        let endian = if obj.is_little_endian() {
            RunTimeEndian::Little
        } else {
            RunTimeEndian::Big
        };

        // Load every DWARF section into an Arc-backed reader. A missing or
        // unreadable section becomes empty rather than an error, so partial
        // debug info still resolves what it can. Mach-O names DWARF sections
        // `__debug_info` (in segment `__DWARF`) where ELF uses `.debug_info`, so
        // we try both spellings.
        let load = |id: gimli::SectionId| -> Result<Reader, gimli::Error> {
            let name = id.name();
            let section = obj.section_by_name(name).or_else(|| {
                let macho = format!("__{}", name.trim_start_matches('.'));
                obj.section_by_name(&macho)
            });
            let bytes: Cow<[u8]> = match section {
                Some(s) => s.uncompressed_data().unwrap_or(Cow::Borrowed(&[])),
                None => Cow::Borrowed(&[]),
            };
            Ok(EndianArcSlice::new(Arc::from(bytes.as_ref()), endian))
        };
        let dwarf = gimli::Dwarf::load(load)
            .map_err(|e| SymError::new(ErrorKind::CorruptLineProgram, format!("dwarf: {e}")))?;

        // Best-effort stats (file count + line-table presence), bounded by the
        // unit cap so a hostile unit count cannot spin.
        let (file_count, has_line_table) = dwarf_stats(&dwarf, limits);

        // Address-sorted function symbol table (fast path + no-line fallback).
        let mut symbols: Vec<(u64, String)> = obj
            .symbols()
            .filter(|s| s.kind() == SymbolKind::Text)
            .filter_map(|s| {
                let name = s.name().ok()?;
                if name.is_empty() {
                    return None;
                }
                Some((s.address(), try_demangle(name)))
            })
            .collect();
        symbols.sort_by_key(|(a, _)| *a);
        symbols.dedup_by_key(|(a, _)| *a);

        let ctx = addr2line::Context::from_dwarf(dwarf)
            .map_err(|e| SymError::new(ErrorKind::CorruptLineProgram, format!("addr2line: {e}")))?;

        Ok(DwarfModule {
            ctx,
            identity,
            name,
            symbols,
            file_count,
            has_line_table,
            byte_size,
        })
    }

    /// This module's identity.
    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// The display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Approximate resident byte footprint.
    pub fn resident_bytes(&self) -> u64 {
        self.byte_size as u64
    }

    /// The symbol whose range starts at or before `addr` (nearest-below).
    fn symbol_for(&self, addr: u64) -> Option<&str> {
        let idx = match self.symbols.binary_search_by_key(&addr, |(a, _)| *a) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        self.symbols.get(idx).map(|(_, n)| n.as_str())
    }

    /// The innermost function name only (fast path).
    pub fn function_name(&self, addr: u64) -> Option<String> {
        // Prefer DWARF (gives the inlined-innermost name); fall back to symbols.
        if let Ok(mut iter) = self.ctx.find_frames(addr).skip_all_loads() {
            if let Ok(Some(frame)) = iter.next() {
                if let Some(f) = frame.function {
                    if let Ok(raw) = f.raw_name() {
                        return Some(try_demangle(&raw));
                    }
                }
            }
        }
        self.symbol_for(addr).map(|s| s.to_string())
    }

    /// Resolve `addr` into inline-expanded frames (innermost-first, physical
    /// frame last). Totally fault-isolated.
    pub fn resolve(&self, addr: u64, limits: &Limits) -> Vec<ResolvedFrame> {
        let mut frames: Vec<ResolvedFrame> = Vec::new();

        let iter = self.ctx.find_frames(addr).skip_all_loads();
        let mut iter = match iter {
            Ok(i) => i,
            Err(_) => {
                return vec![self.error_frame(ErrorKind::CorruptInlineTree)];
            }
        };

        // Collect raw frames (innermost-first), bounded by the inline-depth cap.
        let mut raw: Vec<RawFrame> = Vec::new();
        loop {
            if raw.len() >= limits.max_inline_depth {
                break;
            }
            match iter.next() {
                Ok(Some(frame)) => {
                    let func_raw = frame
                        .function
                        .as_ref()
                        .and_then(|f| f.raw_name().ok().map(|c| c.into_owned()));
                    let (file, line, column) = match frame.location {
                        Some(loc) => (loc.file.map(|s| s.to_string()), loc.line, loc.column),
                        None => (None, None, None),
                    };
                    raw.push((func_raw, file, line, column));
                }
                Ok(None) => break,
                Err(_) => {
                    // Corrupt inline tree mid-walk: keep what we have, mark error.
                    if raw.is_empty() {
                        return vec![self.error_frame(ErrorKind::CorruptInlineTree)];
                    }
                    break;
                }
            }
        }

        if raw.is_empty() {
            // No DWARF coverage. Try the symbol table; otherwise a clean no_line.
            let mut f = self.blank_frame(ResolveStatus::NoLine);
            if let Some(name) = self.symbol_for(addr) {
                f.function = Some(name.to_string());
                f.function_raw = Some(name.to_string());
            }
            return vec![f];
        }

        let n = raw.len();
        for (i, (func_raw, file, line, column)) in raw.into_iter().enumerate() {
            let mut f = self.blank_frame(ResolveStatus::Ok);
            f.inline_depth = i as i32;
            f.is_inline = i + 1 < n;
            f.function = func_raw.as_deref().map(try_demangle);
            f.function_raw = func_raw;
            f.file = file;
            f.line = line;
            f.column = column;
            frames.push(f);
        }
        frames
    }

    /// A frame pre-stamped with this module's name + debug-id and a status, with
    /// all symbol fields empty.
    fn blank_frame(&self, status: ResolveStatus) -> ResolvedFrame {
        ResolvedFrame {
            inline_depth: 0,
            is_inline: false,
            function: None,
            function_raw: None,
            file: None,
            line: None,
            column: None,
            module: Some(self.name.clone()),
            debug_id: Some(self.identity.debug_id_str()),
            status,
        }
    }

    /// A single error frame stamped with this module's debug-id.
    fn error_frame(&self, kind: ErrorKind) -> ResolvedFrame {
        self.blank_frame(ResolveStatus::Error(kind))
    }

    /// The triage view (for `module_info`).
    pub fn info(&self) -> ModuleInfo {
        // dSYM vs plain Mach-O is distinguished by the caller (path-based); the
        // module itself reports the container format from its identity.
        ModuleInfo {
            format: self.identity.format,
            arch: self.identity.arch.clone(),
            build_id: self.identity.raw_build_id.clone(),
            debug_id: Some(self.identity.debug_id_str()),
            code_id: self.identity.code_id.clone(),
            has_dwarf: self.has_line_table || self.file_count > 0,
            has_pdb: false,
            has_line_table: self.has_line_table,
            symbol_count: self.symbols.len() as i64,
            file_count: self.file_count,
            byte_size: self.byte_size,
            image_base: self.identity.image_base,
        }
    }
}

/// Walk the DWARF units (bounded) to count distinct source files and detect a
/// line table. Best-effort: any decode error stops the walk and returns what was
/// counted so far (never panics, never errors out).
fn dwarf_stats(dwarf: &gimli::Dwarf<Reader>, limits: &Limits) -> (i64, bool) {
    let mut file_count: i64 = 0;
    let mut has_line = false;
    let mut units = dwarf.units();
    let mut seen = 0usize;
    loop {
        if seen >= limits.max_units {
            break;
        }
        let header = match units.next() {
            Ok(Some(h)) => h,
            _ => break,
        };
        seen += 1;
        let unit = match dwarf.unit(header) {
            Ok(u) => u,
            Err(_) => continue,
        };
        if let Some(program) = unit.line_program.as_ref() {
            has_line = true;
            file_count += program.header().file_names().len() as i64;
        }
    }
    (file_count, has_line)
}
