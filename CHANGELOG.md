# Changelog

All notable changes to `vgi-symbols` are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/), and the project follows
[Semantic Versioning](https://semver.org/).

## 0.4.4 (2026-11-16)

- VGI Rust SDK `vgi` 0.51.0 → 0.51.1 (`vgi-rpc` stays 0.32.4).
- SQL tests now run through the starter's `ci/run-integration.sh` over stdio, launch and http; `make check` runs everything CI runs.
- CI image test installs the pinned `haybarn-unittest`; new `make lint-explain` metadata gate (vgi-lint at fail-on=info).

## [0.4.3] — 2026-11-15

### Fixed
- Examples no longer read placeholder local paths (`/srv/debug/libssl.so.debug`,
  `/srv/bin/myapp`, `/srv/debug/myapp.debug`) that exist on no user's machine
  (vgi-lint VGI516). `module_info` examples now read the committed Mach-O dSYM
  fixture from a URL pinned to `v0.4.2` and return real rows: the BLOB overload
  fetches it with `read_blob` into a session variable, the path overload first
  writes it to the current directory with `COPY ... (FORMAT blob)`, and the
  debug-id match example compares the downloaded file's `debug_id` with a crash
  report's module debug-ids (one `true`, one `false` verdict). These are
  multi-step `vgi.executable_examples` with `expected_result`, so the two
  single-statement `module_info` entries left the schema-level example list.
- `module_info` docs no longer suggest `module_info(read_blob(...))`, which
  cannot work (`read_blob` is a table function and table-function arguments
  must be constants); they show the `SET VARIABLE` / `getvariable` form.
- README examples 6 and 9 use the fixture and the variable form.

## [0.4.2] — 2026-11-15

### Fixed
- `test/sql/compose.test` no longer depends on running before other test files in a
  shared worker process: it checks that both local source kinds are offered, then
  registers `dir` and checks only `glob` is left. The v0.4.1 release build failed on it.

## [0.4.1] — 2026-11-14

### Changed
- Docs link to upstream sources: the catalog docs now link the DWARF standard, the
  LLVM PDB format docs, debuginfod, and the `addr2line` / `gimli` / `pdb-addr2line`
  crates (vgi-lint VGI183).

### Added
- Composed example queries that combine two of the worker's objects:
  `source_kinds` anti-joined to `sources()` (local kinds still to register) and
  `module_info` on a binary compared with `module_info` on its debug file (one
  verdict row: do the build-ids match). Each is in the schema-level list and on the driving object's
  examples; covered by `test/sql/compose.test`. README gains the debug-file match.

## [0.4.0] — 2026-11-12

### Changed
- Ported to VGI protocol 3.3 (`vgi` 0.50.0 -> 0.51.0, `vgi-rpc` 0.32.3 -> 0.32.4).
  The current haybarn `vgi` extension speaks 3.3 and rejects 3.2 workers. No
  SQL-visible change: every function stays registered as a public table function.

## [0.3.0] and earlier

### Changed
- Catalog metadata satisfies vgi-lint 2.2: the catalog `vgi.title` is now the short
  name "Native Symbolication" (VGI147), the catalog carries a one-sentence
  `vgi.subtitle` (VGI148/VGI149), and the `main` schema title is
  "Symbolication Functions" instead of the headline-style "Symbols — main".

### Security
- Build `vgi` without its default `sqlite` feature. vgi 0.50.0 pins rusqlite 0.39,
  whose bundled SQLite (libsqlite3-sys 0.37) is affected by RUSTSEC-2026-0459
  (CVE-2026-119375). The feature only backs the SDK's cross-process state store,
  which this worker does not use; the SDK now falls back to its `fs` store.

### Changed
- Ported to VGI protocol 3.2 (`vgi` 0.49.2 -> 0.50.0). The haybarn `vgi` extension
  matches protocol major+minor exactly, so 3.1 workers are rejected by the current build.
- Ported to VGI protocol 3.1 (`vgi` 0.49.2, `vgi-rpc` 0.32.3, arrow 59). Requires
  haybarn 2.0 and its community `vgi` extension; haybarn 1.5 can no longer attach this worker.
- SQL examples and tests use the `lambda x: ...` syntax; DuckDB in haybarn 2.0 rejects
  the deprecated `x -> ...` arrow lambda.

## [0.1.0] — initial

### Added
- **Native symbolication as a SQL surface.** Resolve `(build_id, address)` stack
  frames into `function` + `file` + `line` plus the **inlined call chain**, by
  parsing DWARF (ELF / Mach-O / dSYM) and Windows PDB — the same resolution as
  `addr2line` / `llvm-symbolizer`.
  - Scalars: `symbolicate` (one frame → STRUCT with the inline chain), `function_name`
    (innermost name only), `inline_frames` (the inline chain), `demangle`
    (Itanium C++ / Rust legacy+v0 / MSVC / Swift).
  - Table functions: `resolve` (inline-expanded rows), `resolve_batch` (a whole
    `LIST` of frames in one pass, with `frame_idx`), `module_info` (inspect a
    debug file by BLOB or path), `cache_status` / `cache_evict`, and
    `add_source` / `sources` / `drop_source`.
- **The build-id-keyed debug-info cache (the moat).** A process-resident,
  size-bounded (LRU by bytes + module count) cache keyed by the format-normalized
  **debug-id** (ELF GNU build-id → GUID+age, Mach-O UUID, PDB GUID+age), so a
  column of millions of frames parses each debug module **exactly once**. A
  serde-serializable **manifest** (source list + per-debug-id index + negative
  cache) is the externalized state that survives the batch boundary and drives
  cold start; the resident parsed modules are the rebuildable RAM artifact.
- **Local-first, air-gap-safe symbol sources.** `dir` and `glob` sources are
  zero-egress and indexed by debug-id on first use. `debuginfod` / `s3` / `http`
  register with `egress=true` and are **disabled by default** (opt-in only;
  credentials via the SDK secret provider). The default posture performs no
  network egress.
- **Untrusted-input hardening.** Every parse and resolve is fault-isolated and
  bounded (alloc / inline-depth / unit caps) and wrapped in `catch_unwind`, so a
  malformed or hostile ELF / Mach-O / PDB yields a per-row `status='error:<kind>'`
  — never a panic, hang, or OOM. A `cargo-fuzz` zero-panic gate and a proptest
  no-panic suite cover the parser + id paths.

### Notes / known limitations
- The current DuckDB + `vgi` community-extension binding only allows **literal**
  parameters to table functions, so the LATERAL column form
  `… , LATERAL symbols.resolve(f.build_id, f.address)` is not yet supported by
  the engine. The supported bulk patterns are the scalar `symbolicate` /
  `function_name` over a frame column (optionally `UNNEST`-ing the inline list)
  and `resolve_batch` over an aggregated `LIST`. `resolve` works as a
  single-frame table form with literal arguments.
- `debuginfod` / `s3` / `http` remote fetch is **registered and egress-gated but
  not performed** in this release (local-first by design); enabling a remote
  source surfaces it in `sources()` but does not fetch. Remote fetch is the
  next increment.
