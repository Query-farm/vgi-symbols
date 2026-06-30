#![no_main]
//! Zero-panic gate: resolving addresses against an arbitrary (possibly
//! malformed) module must never panic — the core catches deep library panics.
use libfuzzer_sys::fuzz_target;
use symbols_core::module::{Limits, ParsedModule};

fuzz_target!(|data: &[u8]| {
    let limits = Limits::default();
    if let Ok(module) = ParsedModule::parse(data.to_vec(), "fuzz".to_string(), &limits) {
        for addr in [0u64, 0x1000, 0x1_0000_0328, u64::MAX] {
            let frames = module.resolve(addr, &limits);
            assert!(!frames.is_empty());
        }
        let _ = module.function_name(0x1_0000_0328);
    }
});
