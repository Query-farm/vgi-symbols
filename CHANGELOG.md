# Changelog

All notable changes to `vgi-symbols` are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/), and the project follows
[Semantic Versioning](https://semver.org/).

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
    `add_source` / `list_sources` / `drop_source`.
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
  source surfaces it in `list_sources` but does not fetch. Remote fetch is the
  next increment.
