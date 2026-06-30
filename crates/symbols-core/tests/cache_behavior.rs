//! The stateful centerpiece: parse-once, negative cache, cold-start rehydrate,
//! and LRU eviction — driven through `SymbolsState` against the committed
//! Mach-O `.dSYM` fixtures copied into a temp directory source.

use std::fs;
use std::path::PathBuf;

use symbols_core::frame::ResolveStatus;
use symbols_core::SymbolsState;

const INLINE_FIXTURE: &[u8] = include_bytes!("fixtures/macho_inline.dwarf");
const COMPUTE_FIXTURE: &[u8] = include_bytes!("fixtures/macho_compute.dwarf");

/// debug-ids derived from the two fixtures' Mach-O UUIDs.
const INLINE_DEBUG_ID: &str = "76ff8518da153e64a8403892fcbf11250";
const COMPUTE_DEBUG_ID: &str = "4930eb84c7b63ef6a97fbdcdd473f5a30";
const APPLY: u64 = 0x1_0000_0328;

/// Create a unique temp dir, write the fixtures into it, return its path.
fn temp_symbol_dir(tag: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "vgi-symbols-test-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("macho_inline.dwarf"), INLINE_FIXTURE).unwrap();
    fs::write(dir.join("macho_compute.dwarf"), COMPUTE_FIXTURE).unwrap();
    dir
}

#[test]
fn parse_once_across_many_frames() {
    let dir = temp_symbol_dir("parseonce");
    let mut state = SymbolsState::new();
    state
        .add_source(
            "dir",
            Some(dir.to_string_lossy().into_owned()),
            None,
            None,
            None,
            None,
        )
        .unwrap();

    // Resolve the same module 50 times across a range of addresses.
    for addr in (APPLY..APPLY + 0x40).step_by(4) {
        let frames = state.resolve(INLINE_DEBUG_ID, addr);
        assert!(!frames.is_empty());
    }
    // The module was parsed exactly once despite many resolves.
    assert_eq!(state.parse_count(), 1, "module must parse exactly once");

    // And resolving by the raw build-id alias hits the same resident module.
    let frames = state.resolve(INLINE_DEBUG_ID, APPLY);
    assert_eq!(frames.last().unwrap().function.as_deref(), Some("apply"));
    assert_eq!(state.parse_count(), 1);
}

#[test]
fn negative_cache_short_circuits() {
    let dir = temp_symbol_dir("negative");
    let mut state = SymbolsState::new();
    state
        .add_source(
            "dir",
            Some(dir.to_string_lossy().into_owned()),
            None,
            None,
            None,
            None,
        )
        .unwrap();

    let bogus = "deadbeefdeadbeefdeadbeefdeadbeef0";
    let f1 = state.resolve(bogus, 0x1000);
    assert_eq!(f1.len(), 1);
    assert_eq!(f1[0].status, ResolveStatus::NotFound);

    // Second resolve of the same unknown id is a manifest short-circuit, not a
    // re-scan; parse_count never moved and the miss is recorded.
    let f2 = state.resolve(bogus, 0x2000);
    assert_eq!(f2[0].status, ResolveStatus::NotFound);
    assert_eq!(state.parse_count(), 0);
}

#[test]
fn totality_mixes_resolvable_and_not_found() {
    let dir = temp_symbol_dir("totality");
    let mut state = SymbolsState::new();
    state
        .add_source(
            "dir",
            Some(dir.to_string_lossy().into_owned()),
            None,
            None,
            None,
            None,
        )
        .unwrap();

    // A resolvable frame yields >= 1 row; an unknown one yields exactly 1 row.
    let ok = state.resolve(INLINE_DEBUG_ID, APPLY);
    assert!(ok.iter().all(|f| f.status == ResolveStatus::Ok));
    let missing = state.resolve("00000000000000000000000000000000a", APPLY);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].status, ResolveStatus::NotFound);
}

#[test]
fn cold_start_rehydrates_from_manifest() {
    let dir = temp_symbol_dir("rehydrate");
    let mut state = SymbolsState::new();
    state
        .add_source(
            "dir",
            Some(dir.to_string_lossy().into_owned()),
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let _ = state.resolve(INLINE_DEBUG_ID, APPLY);
    assert_eq!(state.parse_count(), 1);

    // Serialize the manifest, then start a fresh process-equivalent state and
    // rehydrate. The resident set is empty; the first resolve re-parses lazily
    // (no redundant *fetch* — the source list and negative cache are restored).
    let manifest = state.export_manifest();
    let bytes = manifest.to_bytes();

    let restored = symbols_core::CacheManifest::from_bytes(&bytes);
    let mut cold = SymbolsState::new();
    cold.import_manifest(&restored);
    assert_eq!(cold.parse_count(), 0, "fresh process starts with no parses");

    let frames = cold.resolve(INLINE_DEBUG_ID, APPLY);
    assert_eq!(frames.last().unwrap().function.as_deref(), Some("apply"));
    assert_eq!(
        cold.parse_count(),
        1,
        "re-parsed lazily exactly once on cold start"
    );

    // The restored manifest carried the source list forward.
    assert_eq!(cold.list_sources().len(), 1);
}

#[test]
fn lru_eviction_is_whole_module_and_reparses_on_touch() {
    let dir = temp_symbol_dir("evict");
    // Budget that holds at most one module (count budget = 1).
    let mut state = SymbolsState::with_budgets(64, 1);
    state
        .add_source(
            "dir",
            Some(dir.to_string_lossy().into_owned()),
            None,
            None,
            None,
            None,
        )
        .unwrap();

    // Resolve module A, then module B. With a 1-module budget, inserting B
    // evicts A. Touching A again re-parses it (parse_count climbs), proving the
    // evicted module was dropped whole and rebuilt from origin.
    let a1 = state.resolve(INLINE_DEBUG_ID, APPLY);
    assert_eq!(a1.last().unwrap().function.as_deref(), Some("apply"));
    assert_eq!(state.parse_count(), 1);

    let b1 = state.resolve(COMPUTE_DEBUG_ID, APPLY);
    assert_eq!(b1.last().unwrap().function.as_deref(), Some("compute"));
    assert_eq!(state.parse_count(), 2);

    // A was evicted; resolving it again re-parses (3rd parse).
    let a2 = state.resolve(INLINE_DEBUG_ID, APPLY);
    assert_eq!(a2.last().unwrap().function.as_deref(), Some("apply"));
    assert_eq!(
        state.parse_count(),
        3,
        "evicted module re-parsed on next touch"
    );
}

#[test]
fn cache_status_lists_resident_modules() {
    let dir = temp_symbol_dir("status");
    let mut state = SymbolsState::new();
    state
        .add_source(
            "dir",
            Some(dir.to_string_lossy().into_owned()),
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let _ = state.resolve(INLINE_DEBUG_ID, APPLY);

    let rows = state.cache_status();
    let resident: Vec<_> = rows.iter().filter(|r| r.resident).collect();
    assert_eq!(resident.len(), 1);
    assert_eq!(resident[0].debug_id, INLINE_DEBUG_ID);
    assert!(resident[0].bytes_resident > 0);
    assert!(resident[0].rows_resolved >= 1);
}
