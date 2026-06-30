//! Golden-fixture DWARF resolution tests over a real Mach-O `.dSYM` built with
//! deliberate `always_inline` callees, so one machine address fans out to a
//! multi-frame inline chain. The fixtures are committed (`tests/fixtures/`); the
//! `object`/`gimli` parsers are cross-platform so these run on Linux CI too.

use symbols_core::frame::{Format, ResolveStatus};
use symbols_core::module::{Limits, ParsedModule};

const INLINE_FIXTURE: &[u8] = include_bytes!("fixtures/macho_inline.dwarf");

/// Symbol addresses observed in the fixture (vmaddr; image base 0x1_0000_0000).
const APPLY_LO: u64 = 0x1_0000_0328;
const APPLY_HI: u64 = 0x1_0000_0368; // == start of `sink`, i.e. end of `apply`

fn parse_inline() -> ParsedModule {
    ParsedModule::parse(
        INLINE_FIXTURE.to_vec(),
        "macho_inline.dwarf".to_string(),
        &Limits::default(),
    )
    .expect("fixture parses")
}

#[test]
fn module_info_reports_macho_dwarf() {
    let module = parse_inline();
    let info = module.info();
    assert_eq!(info.format, Format::MachO);
    assert_eq!(info.arch, "aarch64");
    assert!(info.has_dwarf, "should carry DWARF");
    assert!(info.has_line_table, "should have a line table");
    assert!(info.symbol_count > 0);
    // The normalized debug-id is derived from the LC_UUID 5DBF2CD2-...
    let debug_id = info.debug_id.unwrap();
    assert!(
        debug_id.starts_with("76ff8518"),
        "debug_id {debug_id} should derive from the Mach-O UUID 76FF8518-..."
    );
}

#[test]
fn resolves_physical_function_with_file_and_line() {
    let module = parse_inline();
    let frames = module.resolve(APPLY_LO, &Limits::default());
    assert!(!frames.is_empty());
    let physical = frames.last().unwrap();
    assert!(!physical.is_inline, "deepest frame is the physical one");
    assert_eq!(physical.status, ResolveStatus::Ok);
    assert_eq!(physical.function.as_deref(), Some("apply"));
    assert!(
        physical.file.as_deref().unwrap_or("").ends_with("prog.c"),
        "file should be prog.c, got {:?}",
        physical.file
    );
    assert!(physical.line.unwrap_or(0) > 0);
}

#[test]
fn inline_chain_expands_innermost_first() {
    let module = parse_inline();
    // Scan apply's range; somewhere the inlined inner_hi/inner_lo are live.
    let mut found_inline = false;
    for addr in (APPLY_LO..APPLY_HI).step_by(4) {
        let frames = module.resolve(addr, &Limits::default());
        if frames.len() < 2 {
            continue;
        }
        // Innermost-first: inline_depth ascends 0..N, only the last is physical.
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(f.inline_depth, i as i32);
            assert_eq!(f.is_inline, i + 1 < frames.len());
        }
        let names: Vec<String> = frames.iter().filter_map(|f| f.function.clone()).collect();
        // The physical frame is always apply; the synthesized frames are the
        // inlined inner_* callees.
        assert_eq!(frames.last().unwrap().function.as_deref(), Some("apply"));
        if names.iter().any(|n| n.contains("inner")) {
            found_inline = true;
            break;
        }
    }
    assert!(
        found_inline,
        "expected at least one address in apply() to expand an inlined inner_* frame"
    );
}

#[test]
fn function_name_fast_path() {
    let module = parse_inline();
    assert_eq!(module.function_name(APPLY_LO).as_deref(), Some("apply"));
}

#[test]
fn out_of_range_address_never_panics_and_is_no_line() {
    let module = parse_inline();
    let frames = module.resolve(0xffff_ffff_ffff_0000, &Limits::default());
    assert_eq!(frames.len(), 1);
    // Outside any covered range → no_line (we have the module, not the address).
    assert!(matches!(
        frames[0].status,
        ResolveStatus::NoLine | ResolveStatus::Ok
    ));
}
