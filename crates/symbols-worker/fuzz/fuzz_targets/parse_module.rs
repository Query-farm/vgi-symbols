#![no_main]
//! Zero-panic gate: parsing arbitrary bytes as a debug module must never panic.
use libfuzzer_sys::fuzz_target;
use symbols_core::module::{Limits, ParsedModule};

fuzz_target!(|data: &[u8]| {
    if let Ok(module) = ParsedModule::parse(data.to_vec(), "fuzz".to_string(), &Limits::default()) {
        let _ = module.info();
    }
});
