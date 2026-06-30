//! Catalog + schema metadata (description, provenance, discovery tags) surfaced
//! to DuckDB and the `vgi-lint` metadata-quality linter. The function objects
//! themselves are served from the registered scalars / table functions; this
//! adds the catalog/schema-level comments, `source_url`, and tags.

use vgi::catalog::{CatSchema, CatalogModel};

const REPO: &str = "https://github.com/Query-farm/vgi-symbols";

/// Build the catalog model for the given attach (catalog) name.
pub fn catalog_metadata(name: &str) -> CatalogModel {
    CatalogModel {
        name: name.to_string(),
        comment: Some(
            "Native (DWARF/PDB) symbolication as a bulk SQL JOIN over (build_id, address) frames, \
             backed by a persistent build-id-keyed debug-info cache."
                .to_string(),
        ),
        tags: vec![
            (
                "vgi.title".to_string(),
                "Native Symbolication (DWARF / PDB)".to_string(),
            ),
            (
                "vgi.keywords".to_string(),
                crate::meta::keywords_json(
                    "symbolication, symbolicate, addr2line, llvm-symbolizer, DWARF, PDB, debug \
                     info, build-id, debug-id, stack frame, inline frames, demangle, minidump, \
                     pprof, perf, profiling, crash, DFIR, function name, source line, debuginfod, \
                     symbol cache",
                ),
            ),
            (
                "vgi.doc_llm".to_string(),
                "Resolve raw instruction addresses (return addresses captured in stack frames) \
                 into function name + source file + line plus the inlined call chain, by parsing \
                 native debug info (DWARF for ELF/Mach-O/dSYM, Windows PDB) — exactly as \
                 addr2line / llvm-symbolizer do, but exposed as a SQL JOIN over a column of \
                 (build_id, address) frames and backed by a persistent, build-id-keyed debug-info \
                 cache so a fleet of millions of frames symbolicates in-engine with each debug \
                 module parsed once. Scalars: symbolicate (one frame → STRUCT with inline chain), \
                 function_name (innermost name only), inline_frames (the inline chain), demangle. \
                 Table functions: resolve (LATERAL, fans one address to its inline-expanded rows), \
                 resolve_batch (a whole LIST of frames in one pass), module_info (inspect a debug \
                 file), cache_status / cache_evict (the cache state), and add_source / \
                 list_sources / drop_source (where debug files come from — local dirs/globs are \
                 zero-egress; debuginfod/s3/http are opt-in). Addresses are module-relative (the \
                 caller subtracts the load base). Untrusted debug files never crash the query: a \
                 malformed module yields a per-row error status, never a panic."
                    .to_string(),
            ),
            (
                "vgi.doc_md".to_string(),
                "# symbols\n\nNative **symbolication** as a SQL JOIN: resolve a column of \
                 `(build_id, address)` stack frames into `function` + `file` + `line` plus the \
                 **inlined call chain**, by parsing DWARF (ELF / Mach-O / dSYM) and Windows PDB — \
                 the same resolution as `addr2line` / `llvm-symbolizer`, but over millions of \
                 frames in-engine and backed by a **persistent, build-id-keyed debug-info cache** \
                 so each debug module is parsed exactly once.\n\n**Scalars:** `symbolicate` (one \
                 frame → STRUCT with the inline chain), `function_name` (innermost name only), \
                 `inline_frames` (the inline chain), `demangle`.\n\n**Table functions:** `resolve` \
                 (LATERAL — the bulk JOIN surface), `resolve_batch` (a whole frame list in one \
                 pass), `module_info` (inspect a debug file), `cache_status` / `cache_evict` (the \
                 cache state), and `add_source` / `list_sources` / `drop_source` (symbol sources — \
                 local dirs/globs are zero-egress; `debuginfod`/`s3`/`http` are opt-in). Addresses \
                 are **module-relative** (the caller subtracts the load base). The feeders are \
                 `vgi-minidump`, `vgi-pprof`, and `vgi-perf`."
                    .to_string(),
            ),
            ("vgi.author".to_string(), "Query.Farm".to_string()),
            (
                "vgi.copyright".to_string(),
                "Copyright 2026 Query Farm LLC - https://query.farm".to_string(),
            ),
            ("vgi.license".to_string(), "MIT".to_string()),
            ("vgi.support_contact".to_string(), format!("{REPO}/issues")),
            (
                "vgi.support_policy_url".to_string(),
                format!("{REPO}/blob/main/README.md"),
            ),
        ],
        source_url: Some(REPO.to_string()),
        schemas: vec![CatSchema {
            name: "main".to_string(),
            comment: Some(
                "Symbolication functions: resolve/symbolicate frames, demangle, inspect modules, \
                 manage the debug-info cache and symbol sources."
                    .to_string(),
            ),
            tags: vec![
                ("vgi.title".to_string(), "Symbols — main".to_string()),
                (
                    "vgi.keywords".to_string(),
                    crate::meta::keywords_json(
                        "symbolicate, resolve, resolve_batch, function_name, inline_frames, \
                         demangle, module_info, cache_status, cache_evict, add_source, \
                         list_sources, drop_source, DWARF, PDB, build-id",
                    ),
                ),
                ("domain".to_string(), "observability".to_string()),
                ("category".to_string(), "symbolication".to_string()),
                ("topic".to_string(), "native-debug-info".to_string()),
                (
                    "vgi.doc_llm".to_string(),
                    "The single schema for the `symbols` worker — the catalog name matches the \
                     ATTACH name, so qualify calls as `symbols.main.<fn>(...)`. It holds the \
                     symbolication functions: `symbolicate` (→ STRUCT), `function_name` (→ \
                     VARCHAR), `inline_frames` (→ LIST<STRUCT>), and `demangle` (→ VARCHAR) \
                     scalars; the `resolve` (LATERAL) and `resolve_batch` table functions that \
                     emit inline-expanded rows; `module_info` (inspect a debug file by BLOB or \
                     path); `cache_status` / `cache_evict` for the build-id-keyed cache; and \
                     `add_source` / `list_sources` / `drop_source` for symbol sources."
                        .to_string(),
                ),
                (
                    "vgi.doc_md".to_string(),
                    "The single (and only) schema for the `symbols` worker — qualify calls as \
                     `symbols.main.<fn>(...)`. It holds the `symbolicate` / `function_name` / \
                     `inline_frames` / `demangle` scalars, the `resolve` (LATERAL) and \
                     `resolve_batch` table functions, `module_info`, the `cache_status` / \
                     `cache_evict` cache surface, and the `add_source` / `list_sources` / \
                     `drop_source` symbol-source config."
                        .to_string(),
                ),
                (
                    "vgi.example_queries".to_string(),
                    // Illustrative usage (not auto-executed). Addresses are decimal,
                    // module-relative virtual addresses. Note: the current DuckDB +
                    // vgi binder accepts only literal table-function params, so the
                    // bulk JOIN uses the scalar `symbolicate` over a frame column
                    // (UNNEST `inlined` for inline-expanded rows) or `resolve_batch`
                    // over an aggregated LIST — the `LATERAL symbols.main.resolve(...)`
                    // per-row form lights up once the binder gains lateral support.
                    "SELECT source_id FROM symbols.main.add_source('dir', path => '/srv/debug');\n\
                     SELECT symbols.main.symbolicate('e4c1f2b9', 303600) AS frame;\n\
                     SELECT f.build_id, f.address, symbols.main.function_name(f.build_id, \
                     f.address) AS function FROM stack_frames f;\n\
                     SELECT * FROM symbols.main.resolve_batch((SELECT list({build_id: build_id, \
                     address: address}) FROM stack_frames));\n\
                     SELECT symbols.main.demangle('_ZN3foo3barEv');\n\
                     SELECT * FROM symbols.main.module_info('/srv/debug/libssl.so.debug');\n\
                     SELECT debug_id, name, bytes_resident FROM symbols.main.cache_status() ORDER \
                     BY bytes_resident DESC;"
                        .to_string(),
                ),
            ],
            views: Vec::new(),
            macros: Vec::new(),
            tables: Vec::new(),
        }],
        ..Default::default()
    }
}
