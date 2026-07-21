//! Catalog + schema metadata (description, provenance, discovery tags) surfaced
//! to DuckDB and the `vgi-lint` metadata-quality linter. The function objects
//! themselves are served from the registered scalars / table functions; this
//! adds the catalog/schema-level comments, `source_url`, and tags.

use vgi::catalog::{CatSchema, CatView, CatalogModel};

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
                 per-row status, never a panic or a dropped row."
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
                // Self-contained analyst tasks graded against the live (freshly
                // attached, cold) worker. Design rules that keep them sound:
                //  * value-returning SCALARS (`demangle`, `function_name`) carry no
                //    natural output-column name, so set `ignore_column_names`
                //    (compare values only);
                //  * deterministic-on-a-cold-worker projections: an unresolvable
                //    build-id ('deadbeef') always yields status 'not_found' /
                //    NULL / an empty inline chain regardless of registered sources,
                //    and a missing path / bogus source id yield zero rows / false;
                //  * STATEFUL surfaces are graded by a `check_sql` sanity assertion
                //    rather than an exact count: `cache_status` grows once any
                //    address is resolved, and `add_source` returns a fresh
                //    monotonic id and mutates process-global state, so exact-value
                //    grading would flake. `check_sql` (grader-only) also gives the
                //    static VGI520 coverage those objects need;
                //  * `register_dir_source` mutates state (it calls `add_source`),
                //    so it is declared LAST — the earlier `sources` count stays
                //    deterministic under the sequential (`--ai-concurrency 1`) run.
                r#"[
  {"name":"demangle_cpp_symbol","prompt":"Using the worker's demangle function, turn the mangled Itanium C++ linkage name '_ZN3foo3barEv' into its readable name (it should read 'foo::bar'). Return only the single demangled string.","reference_sql":"SELECT symbols.main.demangle('_ZN3foo3barEv')","ignore_column_names":true},
  {"name":"demangle_passthrough","prompt":"The demangle function returns any name that is not actually mangled unchanged. Show what it returns for the plain name 'main'.","reference_sql":"SELECT symbols.main.demangle('main')","ignore_column_names":true},
  {"name":"symbolicate_cold_status","prompt":"This worker has no symbol sources registered yet, so nothing can be resolved. Symbolicate the frame with build_id 'deadbeef' and module-relative address 4096 and report just the resolution status of the result (it will be 'not_found' because no matching debug file is available).","reference_sql":"SELECT (symbols.main.symbolicate('deadbeef', 4096)).status","ignore_column_names":true},
  {"name":"function_name_cold","prompt":"With no symbol sources registered, resolving any address yields no name. Resolve the innermost function name for build_id 'deadbeef' at module-relative address 4096 — the result should be NULL.","reference_sql":"SELECT symbols.main.function_name('deadbeef', 4096)","ignore_column_names":true},
  {"name":"inline_frames_empty_cold","prompt":"The inline_frames function returns just the inlined call chain (a list) at a frame. With no symbol sources registered, the chain for build_id 'deadbeef' at module-relative address 4096 is empty. Report the length of that list (it should be 0).","reference_sql":"SELECT length(symbols.main.inline_frames('deadbeef', 4096))","ignore_column_names":true},
  {"name":"module_info_missing_path","prompt":"Use the module_info table function to inspect the debug file at the path '/no/such/file.debug'. Because that path does not exist, module_info returns no rows. Report how many rows it returns (it should be 0).","reference_sql":"SELECT count(*) FROM symbols.main.module_info('/no/such/file.debug')","ignore_column_names":true},
  {"name":"local_source_kinds","prompt":"The worker exposes a browsable reference table of the symbol-source kinds add_source accepts. Using it, count how many source kinds are local (they do not cross the trust boundary, i.e. egress is false). Return the count.","reference_sql":"SELECT count(*) FROM symbols.main.source_kinds WHERE egress = false","ignore_column_names":true},
  {"name":"sources_empty_cold","prompt":"How many symbol sources are currently registered on this freshly attached worker? List them with the sources function and return the count (it should be 0 until add_source is called).","reference_sql":"SELECT count(*) FROM symbols.main.sources()","ignore_column_names":true},
  {"name":"cache_status_queryable","prompt":"This worker keeps a persistent, build-id-keyed debug-info cache. Using the cache_status table function, confirm the cache is queryable and report how many debug modules are currently cached (it may well be zero on a freshly attached worker).","check_sql":"SELECT count(*) >= 0 FROM symbols.main.cache_status()"},
  {"name":"cache_evict_cold","prompt":"Clear the worker's entire resident debug-info cache using cache_evict (with no argument) and report how many bytes were freed. On a cold worker with nothing resident this is 0.","reference_sql":"SELECT bytes_freed FROM symbols.main.cache_evict()","ignore_column_names":true},
  {"name":"drop_absent_source","prompt":"Try to drop a symbol source whose id is 'no_such_source'. Since no source with that id was registered, report whether one was actually removed (it should be false).","reference_sql":"SELECT dropped FROM symbols.main.drop_source('no_such_source')","ignore_column_names":true},
  {"name":"register_dir_source","prompt":"Register a local, zero-egress directory symbol source pointing at the directory '/srv/debug' so the worker can find debug files there. Use the add_source function with the appropriate kind and locator argument, then confirm a source is now registered.","check_sql":"SELECT (SELECT source_id FROM symbols.main.add_source('dir', path => '/srv/debug')) IS NOT NULL AND (SELECT count(*) FROM symbols.main.sources()) > 0"}
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
        // Publish the running build version as catalog metadata (VGI328) — an
        // agent reads it from vgi_catalogs() without spending a query, and it
        // cannot drift from the binary the way a parameterless version() scalar
        // could.
        implementation_version: Some(symbols_core::version().to_string()),
        schemas: vec![CatSchema {
            name: "main".to_string(),
            comment: Some(
                "Symbolication functions: symbolicate frames, demangle names, inspect modules, \
                 manage the debug-info cache and symbol sources."
                    .to_string(),
            ),
            tags: vec![
                ("vgi.title".to_string(), "Symbols — main".to_string()),
                (
                    "vgi.keywords".to_string(),
                    crate::meta::keywords_json(
                        "symbolicate, function_name, inline_frames, demangle, module_info, \
                         cache_status, cache_evict, add_source, sources, drop_source, DWARF, PDB, \
                         build-id",
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
  {"name":"Sources","description":"Register, audit, and remove the symbol sources debug files are drawn from."}
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
                     symbol sources it draws debug files from."
                        .to_string(),
                ),
                (
                    "vgi.doc_md".to_string(),
                    "## `symbols.main`\n\
                     \n\
                     Native symbolication as a SQL surface: turn module-relative `(build_id, \
                     address)` frames into function names, source locations, and inlined call \
                     chains by parsing DWARF and PDB debug info — the way `addr2line` and \
                     `llvm-symbolizer` do, but as a JOIN.\n\
                     \n\
                     Qualify calls as `symbols.main.<function>(...)` — the catalog name matches \
                     the ATTACH name. Resolution is backed by a persistent, build-id-keyed \
                     debug-info cache, so each module is parsed once and reused across queries.\n\
                     \n\
                     Debug files are drawn from registered **symbol sources**: local directories \
                     are zero-egress, while remote sources are opt-in."
                        .to_string(),
                ),
                (
                    "vgi.example_queries".to_string(),
                    // Described-example JSON list (VGI515). Addresses are decimal,
                    // module-relative virtual addresses. Bulk symbolication maps the
                    // scalar `symbolicate` / `function_name` over a frame column and
                    // UNNESTs the `inlined` list for inline-expanded rows — the
                    // working vectorized path under the current DuckDB + vgi binder.
                    r#"[
  {"description":"Register a local, zero-egress directory of debug files and capture its source_id.","sql":"SELECT source_id FROM symbols.main.add_source('dir', path => '/srv/debug')"},
  {"description":"Symbolicate one module-relative frame to its function name and resolution status.","sql":"SELECT (symbols.main.symbolicate('e4c1f2b9', 303600)).function AS function, (symbols.main.symbolicate('e4c1f2b9', 303600)).status AS status"},
  {"description":"Bulk-symbolicate a frame column: resolve each (build_id, address) to its innermost function name for a flamegraph or GROUP BY key.","sql":"SELECT f.build_id, f.address, symbols.main.function_name(f.build_id, f.address) AS function FROM stack_frames f"},
  {"description":"Demangle a raw Itanium C++ linkage name to its readable form.","sql":"SELECT symbols.main.demangle('_ZN3foo3barEv') AS name"},
  {"description":"Triage a debug file by path, reading just its format and debug-id (zero rows if the path is missing).","sql":"SELECT format, debug_id FROM symbols.main.module_info('/srv/debug/libssl.so.debug')"},
  {"description":"Watch the build-id-keyed cache: the hottest resident debug modules first.","sql":"SELECT debug_id, name, bytes_resident FROM symbols.main.cache_status() ORDER BY bytes_resident DESC"}
]"#
                        .to_string(),
                ),
            ],
            // A browsable, credential-free reference view (VGI146): it lets an
            // agent SELECT the worker's own vocabulary — the five symbol-source
            // kinds, whether each is local or remote, whether using it egresses,
            // and which `add_source` argument locates it — before it has to
            // construct an `add_source(...)` call. Backed by a literal VALUES list
            // so it scans instantly with no filesystem, network, or credential.
            views: vec![CatView {
                name: "source_kinds".to_string(),
                definition: "SELECT * FROM (VALUES \
                    ('dir', 'local', false, 'path', 'A local directory of debug files, scanned \
                     for matching build-ids; zero egress.'), \
                    ('glob', 'local', false, 'path', 'A recursive filesystem glob of debug files, \
                     e.g. /builds/**/*.{debug,pdb,dSYM}; zero egress.'), \
                    ('debuginfod', 'remote', true, 'url', 'An elfutils debuginfod server, queried \
                     by build-id; opt-in and disabled by default.'), \
                    ('s3', 'remote', true, 'bucket', 'An S3 bucket of debug files; opt-in, \
                     disabled by default, credentials via a named secret.'), \
                    ('http', 'remote', true, 'url', 'An HTTP(S) base URL of debug files addressed \
                     by build-id; opt-in and disabled by default.') \
                    ) AS t(kind, locality, egress, locator_arg, description)"
                    .to_string(),
                comment: Some(
                    "Reference registry of the five symbol-source kinds add_source accepts, with \
                     each kind's locality, egress posture, and locator argument."
                        .to_string(),
                ),
                tags: vec![
                    (
                        "vgi.title".to_string(),
                        "Symbol Source Kinds".to_string(),
                    ),
                    ("vgi.category".to_string(), "Sources".to_string()),
                    ("domain".to_string(), "observability".to_string()),
                    (
                        "vgi.keywords".to_string(),
                        crate::meta::keywords_json(
                            "source kinds, add_source, dir, glob, debuginfod, s3, http, egress, \
                             data residency, locator, reference, vocabulary",
                        ),
                    ),
                    (
                        "vgi.doc_llm".to_string(),
                        "A static lookup table of the five symbol-source kinds the `add_source` \
                         function accepts, so an agent can discover valid inputs before \
                         registering a source. One row per kind with: `kind` (dir / glob / \
                         debuginfod / s3 / http), `locality` ('local' for zero-egress filesystem \
                         sources, 'remote' for opt-in network sources), `egress` (true when using \
                         the kind crosses the trust boundary), `locator_arg` (which `add_source` \
                         named argument locates it — `path`, `url`, or `bucket`), and a one-line \
                         `description`. Filter `egress = false` for the air-gap-safe local kinds, \
                         or read `locator_arg` to learn which argument a given kind needs."
                            .to_string(),
                    ),
                    (
                        "vgi.doc_md".to_string(),
                        "## `source_kinds`\n\nA browsable, credential-free registry of the five \
                         symbol-source kinds `add_source` accepts — read it to discover valid \
                         inputs before registering a source.\n\nEach row is one kind:\n\n- \
                         **kind** — `dir`, `glob`, `debuginfod`, `s3`, or `http`.\n- **locality** \
                         — `local` (zero-egress filesystem) or `remote` (opt-in network).\n- \
                         **egress** — true when using the kind crosses the trust boundary.\n- \
                         **locator_arg** — the `add_source` named argument that locates it \
                         (`path` / `url` / `bucket`).\n- **description** — a one-line \
                         explanation.\n\nFilter `egress = false` for the local kinds, or read \
                         `locator_arg` to learn which argument a kind needs."
                            .to_string(),
                    ),
                    (
                        "vgi.example_queries".to_string(),
                        "[{\"description\":\"List the local, zero-egress source kinds and the \
                         add_source argument that locates each.\",\"sql\":\"SELECT kind, \
                         locator_arg FROM symbols.main.source_kinds WHERE egress = false ORDER BY \
                         kind\"},{\"description\":\"Show every source kind with whether \
                         registering it crosses the trust boundary.\",\"sql\":\"SELECT kind, \
                         locality, egress FROM symbols.main.source_kinds ORDER BY egress, \
                         kind\"}]"
                            .to_string(),
                    ),
                ],
                column_comments: vec![
                    (
                        "kind".to_string(),
                        "The source kind token passed as add_source's first argument: dir / glob \
                         / debuginfod / s3 / http."
                            .to_string(),
                    ),
                    (
                        "locality".to_string(),
                        "'local' for zero-egress filesystem sources, 'remote' for opt-in network \
                         sources."
                            .to_string(),
                    ),
                    (
                        "egress".to_string(),
                        "True when using the kind crosses the trust boundary (a remote source)."
                            .to_string(),
                    ),
                    (
                        "locator_arg".to_string(),
                        "The add_source named argument that locates the source: path, url, or \
                         bucket."
                            .to_string(),
                    ),
                    (
                        "description".to_string(),
                        "A one-line explanation of the source kind.".to_string(),
                    ),
                ],
            }],
            macros: Vec::new(),
            tables: Vec::new(),
        }],
        ..Default::default()
    }
}
