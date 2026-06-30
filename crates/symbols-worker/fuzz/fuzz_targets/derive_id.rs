#![no_main]
//! Zero-panic gate: debug-id derivation must never panic on arbitrary bytes.
use libfuzzer_sys::fuzz_target;
use symbols_core::id::identity_from_object;
use symbols_core::module::probe_identity;

fuzz_target!(|data: &[u8]| {
    let _ = identity_from_object(data);
    let _ = probe_identity(data);
});
