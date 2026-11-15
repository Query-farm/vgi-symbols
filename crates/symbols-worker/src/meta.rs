//! Shared helpers for the per-object discovery/description metadata that the
//! `vgi-lint` strict profile expects on **every** function and table.
//!
//! Each function/table surfaces these in its `FunctionMetadata.tags`:
//! - `vgi.title` (VGI124)            — human-friendly display name
//! - `vgi.doc_llm` (VGI112)          — concise prose aimed at LLMs
//! - `vgi.doc_md` (VGI113)           — short Markdown description
//! - `vgi.keywords` (VGI126/VGI138)  — a JSON array of search terms/synonyms
//!
//! Per-object `vgi.source_url` is intentionally NOT emitted here: `vgi.source_url`
//! belongs on the catalog object only (VGI139). The catalog's `source_url` field
//! already points at the repo.

/// Encode comma-separated keywords as the JSON array of strings that
/// `vgi.keywords` requires (VGI138). Each term is trimmed and empty terms are
/// dropped; the result is e.g. `["fixed-width","unpack","copybook"]`.
pub fn keywords_json(keywords: &str) -> String {
    let items: Vec<String> = keywords
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        // JSON-escape each keyword (covers quotes/backslashes) by emitting a
        // one-element array and stripping the surrounding brackets.
        .map(|k| {
            let escaped = k.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{escaped}\"")
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Build the four standard per-object discovery/description tags
/// (`vgi.title`, `vgi.doc_llm`, `vgi.doc_md`, `vgi.keywords`).
pub fn object_tags(
    title: &str,
    description_llm: &str,
    description_md: &str,
    keywords: &str,
) -> Vec<(String, String)> {
    vec![
        ("vgi.title".to_string(), title.to_string()),
        ("vgi.doc_llm".to_string(), description_llm.to_string()),
        ("vgi.doc_md".to_string(), description_md.to_string()),
        ("vgi.keywords".to_string(), keywords_json(keywords)),
    ]
}

/// Join pre-encoded `{"description":…,"sql":…}` JSON objects into the JSON list
/// a `vgi.example_queries` tag carries, so a composed example is written once
/// (a constant below) and shared by the schema-level list and the per-object
/// list of the object that drives it.
pub fn example_list(items: &[&str]) -> String {
    format!("[\n  {}\n]", items.join(",\n  "))
}

/// Composed example (`source_kinds` view + `sources()`): the local, zero-egress
/// source kinds not registered yet, with the `add_source` argument each needs.
pub const EX_UNREGISTERED_LOCAL_KINDS: &str = r#"{"description":"Find the local, zero-egress source kinds that are not registered yet (source_kinds anti-joined to sources()), with the add_source argument each one needs; returns dir and glob on a fresh worker, and only glob once a dir source is added.","sql":"SELECT k.kind, k.locator_arg FROM symbols.main.source_kinds AS k WHERE k.egress = false AND k.kind NOT IN (SELECT s.kind FROM symbols.main.sources() AS s) ORDER BY k.kind"}"#;

/// Examples read the committed Mach-O dSYM fixtures from a URL pinned to a
/// release tag that carries them (VGI516: no placeholder local paths), e.g.
/// `https://raw.githubusercontent.com/Query-farm/vgi-symbols/v0.4.2/crates/symbols-core/tests/fixtures/macho_inline.dwarf`
/// (debug_id `76ff8518da153e64a8403892fcbf11250`; `macho_compute.dwarf` is
/// `4930eb84c7b63ef6a97fbdcdd473f5a30`).
///
/// Composed executable example (`module_info` on a downloaded debug file joined
/// to the debug-ids a crash report lists): confirm a debug file belongs to the
/// build that crashed by matching debug-ids. A table function takes only
/// constant arguments, so the bytes go through a session variable first.
pub const EX_MATCH_DEBUG_FILE: &str = r#"{"description":"Check that a downloaded debug file belongs to the build that crashed: compare its module_info debug_id with the debug-ids a crash report lists. One verdict row per crash module: matches is true for the build the file was produced from and false for any other build.","sql":[{"description":"Fetch the debug file's bytes (a committed Mach-O dSYM fixture) into a session variable.","sql":"SET VARIABLE symbols_debug_file = (SELECT content FROM read_blob('https://raw.githubusercontent.com/Query-farm/vgi-symbols/v0.4.2/crates/symbols-core/tests/fixtures/macho_inline.dwarf'))"},{"description":"Match it against the crash report's module debug-ids.","sql":"SELECT crash.crash_module, coalesce(d.debug_id = crash.debug_id, false) AS matches, d.debug_id AS debug_file_debug_id, d.format, d.arch, d.has_line_table FROM (VALUES ('compute', '4930eb84c7b63ef6a97fbdcdd473f5a30'), ('inline', '76ff8518da153e64a8403892fcbf11250')) AS crash(crash_module, debug_id) LEFT JOIN symbols.main.module_info(getvariable('symbols_debug_file')) AS d ON true ORDER BY crash.crash_module","expected_result":[{"crash_module":"compute","matches":false,"debug_file_debug_id":"76ff8518da153e64a8403892fcbf11250","format":"MachO","arch":"aarch64","has_line_table":true},{"crash_module":"inline","matches":true,"debug_file_debug_id":"76ff8518da153e64a8403892fcbf11250","format":"MachO","arch":"aarch64","has_line_table":true}]}]}"#;
