//! Windows PDB resolution via `pdb` + `pdb-addr2line`.
//!
//! The whole PDB is read into memory (an owned `Cursor<Vec<u8>>` source) so the
//! parsed [`ContextPdbData`] is `'static` and `Send` and can live resident in
//! the shared cache. PDB addresses are **RVAs** (module-relative), which matches
//! this worker's address-model contract directly.

use std::io::Cursor;

use debugid::DebugId;
use pdb_addr2line::pdb::{MachineType, PDB};
use pdb_addr2line::ContextPdbData;

use crate::errors::{ErrorKind, SymError, SymResult};
use crate::frame::{Format, ModuleInfo, ResolveStatus, ResolvedFrame};
use crate::id::{hex_encode, Identity};

use super::Limits;

/// A parsed PDB held resident in the cache.
pub struct PdbModule {
    data: ContextPdbData<'static, 'static, Cursor<Vec<u8>>>,
    identity: Identity,
    name: String,
    byte_size: i64,
    symbol_count: i64,
}

impl PdbModule {
    /// Parse a PDB (its whole bytes) into a resident module.
    pub fn parse(data: Vec<u8>, name: String, _limits: &Limits) -> SymResult<PdbModule> {
        let byte_size = data.len() as i64;
        let mut pdb = PDB::open(Cursor::new(data))
            .map_err(|e| SymError::new(ErrorKind::BadMagic, format!("pdb open: {e}")))?;

        // Identity: GUID + age → normalized debug-id. The pdb crate already
        // returns the GUID as a network-order Uuid, so no further byte-swap.
        let info = pdb
            .pdb_information()
            .map_err(|e| SymError::new(ErrorKind::BadBuildId, format!("pdb info: {e}")))?;
        let guid = info.guid;
        let age = info.age;
        let debug_id = DebugId::from_parts(guid, age);
        let raw_build_id = Some(hex_encode(guid.as_bytes()));

        let arch = pdb
            .debug_information()
            .ok()
            .and_then(|dbi| dbi.machine_type().ok())
            .map(machine_arch)
            .unwrap_or_else(|| "unknown".to_string());

        let identity = Identity {
            format: Format::Pdb,
            arch,
            debug_id,
            raw_build_id,
            code_id: None,
            image_base: 0,
        };

        // Build the resolve data (parse-once); count functions via a temporary
        // context, which we then drop — the context is rebuilt per resolve from
        // this same parsed data (cheap relative to the initial stream parse).
        let data = ContextPdbData::try_from_pdb(pdb)
            .map_err(|e| SymError::new(ErrorKind::Truncated, format!("pdb data: {e}")))?;
        let symbol_count = data
            .make_context()
            .ok()
            .map(|c| c.function_count() as i64)
            .unwrap_or(0);

        Ok(PdbModule {
            data,
            identity,
            name,
            byte_size,
            symbol_count,
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

    /// The innermost function name only (fast path).
    pub fn function_name(&self, addr: u64) -> Option<String> {
        let probe = u32::try_from(addr).ok()?;
        let ctx = self.data.make_context().ok()?;
        let frames = ctx.find_frames(probe).ok()??;
        frames.frames.first().and_then(|f| f.function.clone())
    }

    /// Resolve `addr` (an RVA) into inline-expanded frames, innermost-first.
    pub fn resolve(&self, addr: u64, limits: &Limits) -> Vec<ResolvedFrame> {
        let ctx = match self.data.make_context() {
            Ok(c) => c,
            Err(_) => return vec![self.error_frame(ErrorKind::CorruptLineProgram)],
        };
        let probe = match u32::try_from(addr) {
            Ok(p) => p,
            // PDB RVAs are 32-bit; an out-of-range address simply isn't covered.
            Err(_) => return vec![self.blank_frame(ResolveStatus::NoLine)],
        };
        let func = match ctx.find_frames(probe) {
            Ok(Some(f)) => f,
            Ok(None) => return vec![self.blank_frame(ResolveStatus::NoLine)],
            Err(_) => return vec![self.error_frame(ErrorKind::CorruptInlineTree)],
        };

        let n = func.frames.len().min(limits.max_inline_depth);
        if n == 0 {
            return vec![self.blank_frame(ResolveStatus::NoLine)];
        }
        let mut out = Vec::with_capacity(n);
        for (i, frame) in func.frames.iter().take(n).enumerate() {
            let mut f = self.blank_frame(ResolveStatus::Ok);
            f.inline_depth = i as i32;
            f.is_inline = i + 1 < n;
            f.function = frame.function.clone();
            f.function_raw = frame.function.clone();
            f.file = frame.file.as_ref().map(|c| c.to_string());
            f.line = frame.line;
            f.column = None;
            out.push(f);
        }
        out
    }

    /// A frame pre-stamped with this module's name + debug-id and a status.
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

    fn error_frame(&self, kind: ErrorKind) -> ResolvedFrame {
        self.blank_frame(ResolveStatus::Error(kind))
    }

    /// The triage view (for `module_info`).
    pub fn info(&self) -> ModuleInfo {
        ModuleInfo {
            format: Format::Pdb,
            arch: self.identity.arch.clone(),
            build_id: self.identity.raw_build_id.clone(),
            debug_id: Some(self.identity.debug_id_str()),
            code_id: self.identity.code_id.clone(),
            has_dwarf: false,
            has_pdb: true,
            has_line_table: true,
            symbol_count: self.symbol_count,
            file_count: 0,
            byte_size: self.byte_size,
            image_base: 0,
        }
    }
}

/// Cheaply derive a PDB's [`Identity`] (GUID + age + arch) without building the
/// full resolve context — used to index a directory of symbol files by debug-id.
pub fn pdb_identity(data: &[u8]) -> SymResult<Identity> {
    let mut pdb = PDB::open(Cursor::new(data.to_vec()))
        .map_err(|e| SymError::new(ErrorKind::BadMagic, format!("pdb open: {e}")))?;
    let info = pdb
        .pdb_information()
        .map_err(|e| SymError::new(ErrorKind::BadBuildId, format!("pdb info: {e}")))?;
    let debug_id = DebugId::from_parts(info.guid, info.age);
    let raw_build_id = Some(hex_encode(info.guid.as_bytes()));
    let arch = pdb
        .debug_information()
        .ok()
        .and_then(|dbi| dbi.machine_type().ok())
        .map(machine_arch)
        .unwrap_or_else(|| "unknown".to_string());
    Ok(Identity {
        format: Format::Pdb,
        arch,
        debug_id,
        raw_build_id,
        code_id: None,
        image_base: 0,
    })
}

/// Map a PDB machine type onto a stable lowercase arch name.
fn machine_arch(m: MachineType) -> String {
    match m {
        MachineType::Amd64 => "x86_64".to_string(),
        MachineType::X86 => "x86".to_string(),
        MachineType::Arm64 => "aarch64".to_string(),
        MachineType::Arm | MachineType::ArmNT => "arm".to_string(),
        other => format!("{other:?}").to_ascii_lowercase(),
    }
}
