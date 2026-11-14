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

/// Composed example (`module_info` on a binary joined to `module_info` on a
/// candidate debug file): confirm a separate debug file belongs to a shipped,
/// stripped binary by matching their debug-ids.
pub const EX_MATCH_DEBUG_FILE: &str = r#"{"description":"Check that a separate debug file belongs to a shipped binary: inspect both with module_info and join on debug_id. One row (the debug file's format, arch and has_line_table) when the build-ids match; zero rows when they differ or either path is not an object file.","sql":"SELECT b.debug_id, d.format, d.arch, d.has_line_table FROM symbols.main.module_info('/srv/bin/myapp') AS b JOIN symbols.main.module_info('/srv/debug/myapp.debug') AS d ON d.debug_id = b.debug_id ORDER BY b.debug_id"}"#;
