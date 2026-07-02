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
                 addr2line / llvm-symbolizer do, but exposed as a bulk SQL JOIN over a column of \
                 (build_id, address) frames and backed by a persistent, build-id-keyed debug-info \
                 cache so each debug module is parsed once even across millions of frames. Reach \
                 for it to turn stack traces, CPU/heap profiles, and crash dumps (minidump / \
                 pprof / perf) into human-readable functions and source locations in-engine, \
                 without shelling out to addr2line per frame. Key concepts: addresses are \
                 MODULE-RELATIVE (the caller subtracts the load base before querying); a debug \
                 module is keyed by its normalized build-id / debug-id, so the same symbols serve \
                 every frame from that module; local symbol sources are zero-egress while remote \
                 ones (debuginfod / s3 / http) are opt-in and disabled by default; and resolution \
                 is total — an unresolved address or a malformed, untrusted debug file yields a \
                 per-row status, never a panic or a dropped row. Discover the individual functions \
                 by listing the schema."
                    .to_string(),
            ),
            (
                "vgi.doc_md".to_string(),
                "# symbols\n\nNative **symbolication** as a SQL JOIN: turn a column of \
                 `(build_id, address)` stack frames into `function` + `file` + `line` plus the \
                 **inlined call chain**, by parsing DWARF (ELF / Mach-O / dSYM) and Windows PDB — \
                 the same resolution as `addr2line` / `llvm-symbolizer`, but over millions of \
                 frames in-engine and backed by a **persistent, build-id-keyed debug-info cache** \
                 so each debug module is parsed exactly once.\n\nReach for it to make stack \
                 traces, CPU/heap profiles, and crash dumps readable directly in SQL. Addresses \
                 are **module-relative** (the caller subtracts the load base); local symbol \
                 sources are zero-egress while remote ones (`debuginfod` / `s3` / `http`) are \
                 opt-in and off by default; and resolution is total — a missing or malformed \
                 module yields a per-row status, never a dropped row. The upstream feeders are \
                 `vgi-minidump`, `vgi-pprof`, and `vgi-perf`."
                    .to_string(),
            ),
            (
                "vgi.agent_test_tasks".to_string(),
                // Self-contained analyst tasks, each graded by result-equality
                // against a freshly attached worker (Tier-1 reference grading is
                // strict on column names + values + order). Design rules that keep
                // them sound:
                //  * avoid MUTATING, non-idempotent surfaces — `add_source` returns
                //    a fresh monotonic source_id every call, so the grader's
                //    reference run and the analyst run would never match;
                //  * value-returning SCALARS (`demangle`, `symbols_version`) carry
                //    no natural output-column name, so set `ignore_column_names`
                //    (compare values only) — otherwise the analyst's un-aliased
                //    column drifts from the reference;
                //  * table tasks select the functions' REAL column names and are
                //    deterministic on a cold worker (`sources` / `cache_status`
                //    return zero rows; non-debug bytes yield zero `module_info`
                //    rows), so strict grading holds.
                r#"[
  {"name":"demangle_cpp_symbol","prompt":"Using the worker's demangle function, turn the mangled Itanium C++ linkage name '_ZN3foo3barEv' into its readable name (it should read 'foo::bar'). Return only the single demangled string.","reference_sql":"SELECT symbols.main.demangle('_ZN3foo3barEv')","ignore_column_names":true},
  {"name":"demangle_passthrough","prompt":"The demangle function returns any name that is not actually mangled unchanged. Show what it returns for the plain name 'main'.","reference_sql":"SELECT symbols.main.demangle('main')","ignore_column_names":true},
  {"name":"symbolicate_cold_status","prompt":"This worker has no symbol sources registered yet, so nothing can be resolved. Symbolicate the frame with build_id 'deadbeef' and module-relative address 4096 and report just the resolution status of the result (it will be 'not_found' because no matching debug file is available).","reference_sql":"SELECT (symbols.main.symbolicate('deadbeef', 4096)).status","ignore_column_names":true},
  {"name":"function_name_cold","prompt":"With no symbol sources registered, resolving any address yields no name. Resolve the innermost function name for build_id 'deadbeef' at module-relative address 4096 — the result should be NULL.","reference_sql":"SELECT symbols.main.function_name('deadbeef', 4096)","ignore_column_names":true},
  {"name":"worker_version","prompt":"What version string does the symbols worker report for itself?","reference_sql":"SELECT symbols.main.symbols_version()","ignore_column_names":true}
]"#
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
                         sources, drop_source, DWARF, PDB, build-id",
                    ),
                ),
                ("domain".to_string(), "observability".to_string()),
                ("category".to_string(), "symbolication".to_string()),
                ("topic".to_string(), "native-debug-info".to_string()),
                (
                    "vgi.categories".to_string(),
                    r#"[
  {"name":"Resolution","description":"Turn module-relative (build_id, address) frames into functions, source locations, and inline call chains — one at a time or in bulk."},
  {"name":"Demangling","description":"Turn raw mangled linkage names into readable function names, independent of any module."},
  {"name":"Modules","description":"Inspect a candidate debug file's identity and capabilities without resolving or caching it."},
  {"name":"Cache","description":"Observe and control the persistent, build-id-keyed debug-info cache."},
  {"name":"Sources","description":"Register, audit, and remove the symbol sources debug files are drawn from."},
  {"name":"Diagnostics","description":"Introspect the running worker, such as its build version."}
]"#
                        .to_string(),
                ),
                (
                    "vgi.doc_llm".to_string(),
                    "The single schema for the `symbols` worker; the catalog name matches the \
                     ATTACH name, so qualify calls as `symbols.main.<function>(...)`. It groups \
                     the symbolication surface — turning module-relative `(build_id, address)` \
                     frames into functions, source locations, and inline call chains — together \
                     with the persistent debug-info cache that backs it and the registry of \
                     symbol sources it draws debug files from. List the schema to discover the \
                     individual functions and their categories."
                        .to_string(),
                ),
                (
                    "vgi.doc_md".to_string(),
                    "The single schema for the `symbols` worker — qualify calls as \
                     `symbols.main.<function>(...)`. It groups the symbolication functions, the \
                     build-id-keyed debug-info cache that backs them, and the registry of symbol \
                     sources debug files are drawn from. Local sources are zero-egress; remote \
                     ones are opt-in. List the schema to discover the available functions."
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
