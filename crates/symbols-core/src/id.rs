//! Debug-id derivation and normalization — the three-way key that lets one
//! column of mixed-platform frames resolve against one cache.
//!
//! A frame carries the **build-id of the module it executed in**; the matching
//! debug file carries the **same** identifier. Resolve is a key lookup, never a
//! filename guess. The key is the format-normalized **debug-id**:
//!
//! | Format | Source identifier | debug-id derivation |
//! | --- | --- | --- |
//! | ELF | GNU build-id note (`NT_GNU_BUILD_ID`) | first 16 bytes as a little-endian GUID + age 0 |
//! | Mach-O | `LC_UUID` (16-byte UUID) | the UUID + age 0 (no byte-swap) |
//! | PE/PDB | CodeView GUID + age | GUID (network byte order) + age |
//!
//! The raw per-format build-id hex is retained alongside the normalized id so a
//! `debuginfod` lookup (which keys on the *full* build-id) still works, and so a
//! caller can join on either identifier.

use debugid::DebugId;
use object::read::Object;
use object::{Architecture, FileKind};
use uuid::Uuid;

use crate::errors::{ErrorKind, SymError, SymResult};
use crate::frame::Format;

/// The normalized identity of a debug module: its format, arch, normalized
/// debug-id, and the raw identifiers needed for cross-keying and remote fetch.
#[derive(Debug, Clone)]
pub struct Identity {
    /// Container format.
    pub format: Format,
    /// CPU architecture string (e.g. `x86_64`, `aarch64`).
    pub arch: String,
    /// The normalized cache key.
    pub debug_id: DebugId,
    /// Raw per-format build-id hex (ELF GNU build-id / Mach-O UUID / PDB GUID).
    /// Retained for `debuginfod` (keys on the full build-id) and for joining.
    pub raw_build_id: Option<String>,
    /// Code-file identifier (PE timestamp+size, ELF build-id) for code lookup.
    pub code_id: Option<String>,
    /// The image's preferred load base.
    pub image_base: u64,
}

impl Identity {
    /// The canonical (lowercase breakpad) debug-id string used as the LRU /
    /// manifest key.
    pub fn debug_id_str(&self) -> String {
        self.debug_id.breakpad().to_string().to_ascii_lowercase()
    }

    /// Every lookup token under which a module with this identity should be
    /// indexed: the normalized debug-id (breakpad), the bare UUID hex, and the
    /// raw per-format build-id hex. All are canonicalized (lowercase, no
    /// separators) so a frame keyed by any of them resolves.
    pub fn aliases(&self) -> Vec<String> {
        let mut out = vec![
            canonical_token(&self.debug_id_str()),
            canonical_token(&self.debug_id.uuid().simple().to_string()),
        ];
        if let Some(b) = &self.raw_build_id {
            out.push(canonical_token(b));
        }
        out.sort();
        out.dedup();
        out
    }
}

/// Canonicalize an identifier string for keying: lowercase, drop ASCII
/// whitespace and `-`/`{`/`}` separators. A frame's `build_id` and a module's
/// derived ids are compared only after passing through this, so a GUID written
/// as `e4c1f2-...` or `{E4C1F2...}` still matches the raw hex form.
pub fn canonical_token(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_ascii_whitespace() && !matches!(c, '-' | '{' | '}'))
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Map an `object` architecture onto a stable lowercase name.
pub fn arch_name(arch: Architecture) -> String {
    let s = match arch {
        Architecture::X86_64 | Architecture::X86_64_X32 => "x86_64",
        Architecture::I386 => "x86",
        Architecture::Aarch64 | Architecture::Aarch64_Ilp32 => "aarch64",
        Architecture::Arm => "arm",
        Architecture::PowerPc => "ppc",
        Architecture::PowerPc64 => "ppc64",
        Architecture::Riscv32 => "riscv32",
        Architecture::Riscv64 => "riscv64",
        Architecture::Mips => "mips",
        Architecture::Mips64 => "mips64",
        Architecture::S390x => "s390x",
        Architecture::LoongArch64 => "loongarch64",
        Architecture::Wasm32 => "wasm32",
        Architecture::Unknown => "unknown",
        other => return format!("{other:?}").to_ascii_lowercase(),
    };
    s.to_string()
}

/// Derive the [`Identity`] of an ELF / Mach-O / PE image from its bytes.
///
/// Returns [`ErrorKind::BadMagic`] for an unrecognized container and
/// [`ErrorKind::UnsupportedFormat`] for a recognized container we cannot key
/// (e.g. a fat archive). Never panics on malformed input.
pub fn identity_from_object(data: &[u8]) -> SymResult<Identity> {
    let kind = FileKind::parse(data)
        .map_err(|e| SymError::new(ErrorKind::BadMagic, format!("file kind: {e}")))?;
    let format = match kind {
        FileKind::Elf32 | FileKind::Elf64 => Format::Elf,
        FileKind::MachO32 | FileKind::MachO64 => Format::MachO,
        FileKind::Pe32 | FileKind::Pe64 => Format::Pe,
        other => {
            return Err(SymError::new(
                ErrorKind::UnsupportedFormat,
                format!("unsupported container {other:?}"),
            ))
        }
    };
    let obj = object::File::parse(data)
        .map_err(|e| SymError::new(ErrorKind::Truncated, format!("parse object: {e}")))?;

    let arch = arch_name(obj.architecture());
    let image_base = obj.relative_address_base();

    let (debug_id, raw_build_id, code_id) =
        match format {
            Format::Elf => {
                let build =
                    obj.build_id().ok().flatten().ok_or_else(|| {
                        SymError::new(ErrorKind::BadBuildId, "no GNU build-id note")
                    })?;
                let raw = hex_encode(build);
                // First 16 bytes read as a little-endian GUID + age 0 (the Sentry/
                // Breakpad ELF convention). Short notes are zero-padded to 16.
                let mut guid = [0u8; 16];
                let n = build.len().min(16);
                guid[..n].copy_from_slice(&build[..n]);
                let debug_id = DebugId::from_guid_age(&guid, 0)
                    .map_err(|_| SymError::new(ErrorKind::BadBuildId, "bad build-id guid"))?;
                (debug_id, Some(raw.clone()), Some(raw))
            }
            Format::MachO => {
                let uuid = obj
                    .mach_uuid()
                    .ok()
                    .flatten()
                    .ok_or_else(|| SymError::new(ErrorKind::BadBuildId, "no LC_UUID"))?;
                let raw = hex_encode(&uuid);
                // Mach-O UUIDs are already network-order — no byte-swap.
                let debug_id = DebugId::from_uuid(Uuid::from_bytes(uuid));
                (debug_id, Some(raw.clone()), Some(raw))
            }
            Format::Pe => {
                let cv = obj.pdb_info().ok().flatten().ok_or_else(|| {
                    SymError::new(ErrorKind::BadBuildId, "no CodeView debug info")
                })?;
                let guid = cv.guid();
                let age = cv.age();
                let debug_id = DebugId::from_guid_age(&guid, age)
                    .map_err(|_| SymError::new(ErrorKind::BadBuildId, "bad PE guid"))?;
                (debug_id, Some(hex_encode(&guid)), None)
            }
            _ => unreachable!(),
        };

    Ok(Identity {
        format,
        arch,
        debug_id,
        raw_build_id,
        code_id,
        image_base,
    })
}

/// Lowercase hex-encode a byte slice (no separators).
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_strips_separators() {
        assert_eq!(canonical_token("E4-C1 f2"), "e4c1f2");
        assert_eq!(canonical_token("{ABCDEF}"), "abcdef");
    }

    #[test]
    fn hex_roundtrips() {
        assert_eq!(hex_encode(&[0x00, 0xff, 0x1a]), "00ff1a");
    }

    #[test]
    fn elf_build_id_to_guid_byteswaps_first_fields() {
        // A 20-byte build-id; first 16 bytes form the GUID (LE fields swapped).
        let build: Vec<u8> = (1u8..=20).collect();
        let mut guid = [0u8; 16];
        guid.copy_from_slice(&build[..16]);
        let id = DebugId::from_guid_age(&guid, 0).unwrap();
        // Data1 (bytes 0..4) is byte-swapped: 01020304 -> 04030201.
        let s = id.breakpad().to_string();
        assert!(s.starts_with("04030201"), "got {s}");
    }

    #[test]
    fn bad_magic_is_clean_error() {
        let err = identity_from_object(b"not an object file at all").unwrap_err();
        assert_eq!(err.kind, ErrorKind::BadMagic);
    }

    #[test]
    fn truncated_elf_does_not_panic() {
        let mut data = vec![0x7f, b'E', b'L', b'F', 2, 1, 1, 0];
        data.extend_from_slice(&[0u8; 8]);
        let _ = identity_from_object(&data); // must not panic
    }
}
