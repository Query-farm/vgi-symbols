<p align="center">
  <img src="https://raw.githubusercontent.com/Query-farm/vgi-rust/main/docs/vgi-logo.png" alt="VGI" height="80">
</p>

# vgi-symbols

**Native symbolication as a SQL JOIN** — resolve a column of `(build_id, address)`
stack frames into `function` + `file` + `line` plus the **inlined call chain**,
by parsing native debug info (DWARF for ELF / Mach-O / dSYM, Windows PDB) exactly
as `addr2line` / `llvm-symbolizer` do, backed by a **persistent, build-id-keyed
debug-info cache** so a fleet of millions of frames symbolicates in-engine with
each debug module parsed **exactly once** — no external symbolication service, no
per-row reparse, no shipping crash data to a SaaS.

A [VGI](https://query.farm) worker for DuckDB. This is the **resolve backend** for
the crash/profiling wave: [`vgi-minidump`](https://query.farm),
[`vgi-pprof`](https://query.farm), and [`vgi-perf`](https://query.farm) extract
`(build_id, address)` frames and feed them here.

> **License:** Query Farm Source-Available (a stateful compute asset — the cache
> is the moat). All *dependencies* are permissive (MIT / Apache-2.0; zero
> copyleft). See [`LICENSE`](LICENSE).

## Why a cache is the product

Resolving one address is cheap *once the module is parsed* — a binary search in a
sorted line table plus a walk of the inline tree. Parsing the module is **not**: a
release `libchrome.so.debug` or a game-engine PDB is hundreds of MB of DWARF/PDB.
A naive `addr2line`-per-row re-parses that for **every** frame; across a profiling
fleet that is the entire cost. The cache turns `O(frames × parse)` into
`O(modules × parse + frames × lookup)`. **That is the worker.**

The cache is keyed by the format-normalized **debug-id** (ELF GNU build-id →
GUID+age, Mach-O `LC_UUID`, PDB GUID+age), so one column of mixed-platform frames
resolves against one cache. It is size-bounded (LRU by bytes + module count), with
a **negative cache** for build-ids proven missing, and a serde-serializable
**manifest** (source index + provenance + negative cache) that survives the batch
boundary and drives cold start.

## Quick start

```sql
INSTALL vgi FROM community;
LOAD vgi;
ATTACH 'symbols' AS symbols (TYPE vgi, LOCATION '/path/to/symbols-worker');

-- Point the worker at where debug files live (LOCAL by default; zero egress).
CALL symbols.add_source('dir',  path => '/srv/debug');
CALL symbols.add_source('glob', path => '/builds/**/*.{debug,pdb,dSYM}');

-- Resolve a single (build_id, address) frame to function/file/line + inline chain.
SELECT symbols.main.symbolicate('76ff8518da153e64a8403892fcbf11250', 4294968116) AS frame;
--   → {function: 'apply', file: '…/prog.c', line: 16,
--      inlined: [{function:'inner_lo', …}, {function:'inner_hi', …}], status: 'ok', …}
```

## SQL surface

```sql
-- 1. Resolve one frame to a STRUCT with the inline chain collapsed (scalar).
SELECT symbols.main.symbolicate(build_id, address) FROM crash_top_frames;

-- 2. Just a label (the fast path for a flamegraph leaf / GROUP BY key).
SELECT symbols.main.function_name(top.build_id, top.address) AS crash_site, count(*) AS hits
FROM   crash_top_frames top
GROUP  BY crash_site ORDER BY hits DESC;          -- "which function are we crashing in"

-- 3. THE WORKLOAD: bulk-symbolicate a frame table, fanning each address out to
--    its inline chain via the scalar struct + UNNEST (innermost-first).
SELECT f.thread_id, f.frame_idx, u.depth, u.fr.function, u.fr.file, u.fr.line
FROM   stack_frames f,
       UNNEST(list_transform(
         symbols.main.symbolicate(f.build_id, f.address).inlined,
         (x, i) -> {'depth': i, 'fr': x})) AS t(u)
ORDER  BY f.thread_id, f.frame_idx, u.depth;

-- 4. Just the inline chain at an address.
SELECT symbols.main.inline_frames(build_id, address) FROM frames;

-- 5. Demangle a raw symbol (Itanium C++ / Rust legacy+v0 / MSVC / Swift).
SELECT symbols.main.demangle('_ZN3foo3barEv');     -- → 'foo::bar'

-- 6. Inspect a module before resolving: is it usable, and does its debug-id match?
SELECT * FROM symbols.main.module_info('/srv/debug/libssl.so.debug');
--   → format MachO/ELF/PE/PDB, build_id, debug_id, arch, has_dwarf, has_line_table, symbol_count

-- 7. Cache observability (the stateful part) — what is parsed and resident now.
SELECT debug_id, name, format, bytes_resident, rows_resolved, last_used
FROM   symbols.main.cache_status() ORDER BY bytes_resident DESC;

-- 8. Audit where symbols come from (and whether any source egresses).
SELECT * FROM symbols.main.sources();
```

### Functions

| Area | Signature | Kind |
| --- | --- | --- |
| Resolve one frame | `symbolicate(build_id VARCHAR, address UBIGINT) → STRUCT(function, file, line, inlined LIST<STRUCT(function,file,line)>, module, debug_id, status)` | scalar |
| Names only (fast path) | `function_name(build_id, address) → VARCHAR` | scalar |
| Inline chain only | `inline_frames(build_id, address) → LIST<STRUCT(function, file, line)>` | scalar |
| Demangle | `demangle(mangled VARCHAR [, lang]) → VARCHAR` | scalar |
| Resolve a column (rows) | `resolve(build_id, address) → TABLE(...)` | table-in-out † |
| Resolve a list in one pass | `resolve_batch(LIST<STRUCT(build_id, address)>) → TABLE(frame_idx, ...)` | table-in-out † |
| Inspect a debug file | `module_info(blob BLOB)` / `module_info(path VARCHAR) → TABLE(...)` | table |
| Sources | `add_source(kind [, path=>, url=>, bucket=>, enabled=>, secret=>])`, `sources()`, `drop_source(id)` | table |
| Cache | `cache_status() → TABLE(...)`, `cache_evict(debug_id := NULL) → BIGINT` | table |

The resolved-row shape (`resolve` / `resolve_batch`) emits **one row per (input
frame × inline level)**, ordered innermost-first with the physical frame last:
`build_id, address, inline_depth, is_inline, function, function_raw, file, line,
column, module, debug_id, status`. A frame with no matching module yields **one**
row, `status='not_found'`, all symbol columns NULL — resolution **never drops a
frame and never errors the scan**.

> † **Engine note.** The current DuckDB + `vgi` community-extension binding only
> allows **literal** parameters to table functions, so the headline
> `… , LATERAL symbols.resolve(f.build_id, f.address)` form (per-row column
> params) is not yet accepted by the engine binder. The working bulk patterns are
> the scalar `symbolicate` / `function_name` over a frame column (with `UNNEST` of
> the inline list for inline-expanded rows, query 3 above). `resolve` /
> `resolve_batch` are shipped (correct table-in-out implementations) and light up
> when the binding gains lateral/streaming-argument support.

### Address model (caller contract)

`address` is the **module-relative virtual address** (within the module image,
before ASLR slide / load bias). The caller — or the upstream
`vgi-minidump`/`vgi-pprof`/`vgi-perf` frame extractor, which owns the module map —
**subtracts the module base first**. `module_info` reports the image's preferred
base to help.

### Symbol sources (local-first, remote opt-in)

| Source | Config | Egress | Notes |
| --- | --- | --- | --- |
| `dir` | `path => '/srv/debug'` | none | indexed by debug-id on first use |
| `glob` | `path => '/builds/**/*.{debug,pdb,dSYM}'` | none | recursive; `{a,b}` brace alternation supported |
| `debuginfod` / `s3` / `http` | `url=>` / `bucket=>`, `enabled => true`, `secret =>` | **network** | opt-in, **off by default**; credentials via the SDK secret provider |

The default posture is **air-gap-safe**: with no remote source enabled there is
zero network egress. (Remote *fetch* is registered and egress-gated but not
performed in this release — local-first by design; see the [CHANGELOG](CHANGELOG.md).)

## Hardening (untrusted debug files)

Debug files are attacker-influenced (a crash corpus / malware sample's own
symbols). A malformed ELF/PDB produces a clean per-row error, **never** a crash,
hang, or OOM:

- **Per-row fault isolation** — a malformed module yields `status='error:<kind>'`
  for the frames that needed it (`kind ∈ {truncated, bad-magic, bad-build-id,
  corrupt-line-program, corrupt-inline-tree, unsupported-format, nesting-limit,
  alloc-cap}`); every other module and frame is unaffected.
- **Bounded allocation + recursion** — caps on parse size, inline-tree depth, and
  unit count; a hostile blob can't OOM or stack-overflow.
- **`catch_unwind` containment** — the underlying DWARF/PDB libraries can `panic`
  on adversarial input (e.g. an arithmetic overflow); that panic is caught and
  converted to a clean error row.
- **`cargo-fuzz` zero-panic gate** + a proptest no-panic suite on the ELF/Mach-O/
  PE/PDB and build-id paths (CI).
- **Negative cache** prevents amplification — a missing build-id is recorded once
  and short-circuits.

## Build & test

```sh
cargo test --workspace                          # unit + golden DWARF + cache + proptest no-panic
cargo clippy --all-targets -- -D warnings
cargo build --release                           # → target/release/symbols-worker
./run_tests.sh                                  # haybarn SQLLogic e2e
```

The end-to-end suite needs the haybarn tooling (one-time):

```sh
uv tool install haybarn-unittest
echo "INSTALL vgi FROM community;" | uvx haybarn-cli
```

See [`CLAUDE.md`](CLAUDE.md) for architecture, the cache internals, and how to
regenerate the golden Mach-O dSYM fixtures.

## License

Query Farm Source-Available — see [`LICENSE`](LICENSE). Copyright 2026 Query Farm
LLC — https://query.farm
