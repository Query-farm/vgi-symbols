//! Untrusted-input hardening: property tests that the parsers and resolver
//! **never panic** on arbitrary or truncated bytes. Debug files are
//! attacker-influenced in the workloads this worker targets, so a malformed
//! ELF/Mach-O/PDB must produce a clean error or empty result, never a crash.
//!
//! This mirrors the `cargo-fuzz` zero-panic gate (see `fuzz/`); proptest gives a
//! fast, deterministic, in-CI version that also seeds from the real fixtures so
//! truncations of valid containers are exercised, not just random noise.

use std::sync::Once;

use proptest::prelude::*;

use symbols_core::demangle::{demangle, DemangleLang};
use symbols_core::id::identity_from_object;
use symbols_core::module::{probe_identity, Limits, ParsedModule};

const INLINE_FIXTURE: &[u8] = include_bytes!("fixtures/macho_inline.dwarf");

/// The core *catches* panics raised deep in the DWARF/PDB readers and converts
/// them to clean error rows; that conversion still fires the global panic hook
/// (noisy on stderr). Install a no-op hook in this fuzz binary so the (expected,
/// contained) panics don't spam CI logs. proptest reports genuine failures via
/// its own capture, so real regressions are still visible.
static QUIET: Once = Once::new();
fn quiet_hook() {
    QUIET.call_once(|| std::panic::set_hook(Box::new(|_| {})));
}

/// Drive every parse/resolve entry point on `data`; the harness fails only if a
/// panic *escapes* the core's containment (proptest turns that into a failure).
fn exercise(data: &[u8]) {
    quiet_hook();
    let limits = Limits::default();
    let _ = identity_from_object(data);
    let _ = probe_identity(data);
    if let Ok(module) = ParsedModule::parse(data.to_vec(), "fuzz".to_string(), &limits) {
        // Resolve a spread of addresses, including edge values.
        for addr in [0u64, 1, 0x328, 0x1_0000_0328, u64::MAX, u64::MAX / 2] {
            let frames = module.resolve(addr, &limits);
            // The resolver is total: always at least one row, never zero.
            assert!(!frames.is_empty());
        }
        let _ = module.function_name(0x1_0000_0328);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// Arbitrary bytes never panic any parser.
    #[test]
    fn arbitrary_bytes_never_panic(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        exercise(&data);
    }

    /// Bytes that start with a real container magic but are otherwise garbage.
    #[test]
    fn magic_prefixed_garbage_never_panics(
        tail in proptest::collection::vec(any::<u8>(), 0..2048),
        which in 0u8..4,
    ) {
        let mut data: Vec<u8> = match which {
            0 => vec![0x7f, b'E', b'L', b'F', 2, 1, 1, 0],          // ELF64 LE
            1 => vec![0xcf, 0xfa, 0xed, 0xfe],                      // Mach-O 64
            2 => b"Microsoft C/C++ MSF 7.00\r\n\x1a\x44\x53\x00\x00\x00".to_vec(), // PDB
            _ => vec![b'M', b'Z'],                                  // PE
        };
        data.extend_from_slice(&tail);
        exercise(&data);
    }

    /// Truncations of a *valid* Mach-O dSYM never panic (every prefix length).
    #[test]
    fn truncated_valid_fixture_never_panics(len in 0usize..INLINE_FIXTURE.len()) {
        exercise(&INLINE_FIXTURE[..len]);
    }

    /// A single flipped byte in the valid fixture never panics.
    #[test]
    fn bitflipped_fixture_never_panics(pos in 0usize..INLINE_FIXTURE.len(), xor in 1u8..=255) {
        let mut data = INLINE_FIXTURE.to_vec();
        data[pos] ^= xor;
        exercise(&data);
    }

    /// Demangling arbitrary strings under every language never panics.
    #[test]
    fn demangle_never_panics(s in ".{0,256}") {
        for lang in [DemangleLang::Auto, DemangleLang::Cpp, DemangleLang::Rust, DemangleLang::Msvc, DemangleLang::Swift] {
            let _ = demangle(&s, lang);
        }
    }
}
