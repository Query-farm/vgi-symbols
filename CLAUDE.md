# CLAUDE.md

Guidance for working in this repository.

## What this is

`vgi-symbols` is a **VGI worker** (a standalone binary DuckDB launches and talks
to over Apache Arrow IPC, `ATTACH 'symbols' (TYPE vgi, LOCATION '…')`) that does
**native symbolication**: it resolves a raw instruction address into the
function name + source file + line, plus the **inlined call chain**, by parsing
DWARF (ELF / Mach-O / dSYM) and Windows PDB — exactly as `addr2line` /
`llvm-symbolizer` do, but exposed as a **SQL JOIN over a column of
`(build_id, address)` frames** and backed by a **persistent, build-id-keyed
debug-info cache**. Functions live under catalog `symbols`, schema `main`.

This is a strategic **compute moat**: any-language symbolication as a SQL surface
+ a hard-stateful, build-id-keyed debug-info cache (no external symbolication
service, no per-row reparse). It is the resolve backend for the crash/profiling
wave: `vgi-minidump`, `vgi-pprof`, and `vgi-perf` feed `(build_id, address)` into
it. Keep scope tight to that — resist feature sprawl (unwinding, minidump
parsing, load-map/ASLR resolution, a symbol-store UI are all out of scope).

Built on the published VGI Rust SDK (`vgi = "0.9.5"` from crates.io), arrow 59.
Modeled on `../vgi-fixedformat`. The repo builds standalone — no local SDK
checkout. **Worker license: Query Farm Source-Available** (a stateful compute
asset, not a pure-permissive utility) — all *dependencies* are MIT/Apache-2.0.

## SQL surface

```sql
INSTALL vgi FROM community; LOAD vgi;
ATTACH 'symbols' AS symbols (TYPE vgi, LOCATION '/path/to/symbols-worker');

CALL symbols.add_source('dir', path => '/srv/debug');     -- local, zero-egress
SELECT symbols.main.symbolicate('<build_id>', <addr>);    -- → STRUCT(fn,file,line,inlined,…)
SELECT * FROM symbols.resolve_batch((SELECT list({build_id, address}) FROM frames));
```

- `symbolicate(build_id VARCHAR, address UBIGINT) -> STRUCT(function, file, line, inlined LIST<STRUCT(function,file,line)>, module, debug_id, status)` — one frame, inline chain collapsed.
- `function_name(build_id, address) -> VARCHAR` — innermost name only (fast path).
- `inline_frames(build_id, address) -> LIST<STRUCT(function,file,line)>` — the inline chain.
- `demangle(mangled [, lang]) -> VARCHAR` — Itanium C++ / Rust legacy+v0 / MSVC / Swift (`lang` positional: auto/cpp/rust/msvc/swift).
- `resolve(build_id, address) -> TABLE(...)` — inline-expanded rows; **single-frame table form with literal args** (see the binder note below).
- `resolve_batch(frames LIST<STRUCT(build_id, address)>) -> TABLE(frame_idx, ...)` — a whole list in one pass (the bulk vectorized path).
- `module_info(blob BLOB)` / `module_info(path VARCHAR) -> TABLE(...)` — inspect a debug file without resolving.
- `add_source(kind [, path =>, url =>, bucket =>, enabled =>, secret =>])`, `list_sources()`, `drop_source(source_id)`.
- `cache_status() -> TABLE(...)`, `cache_evict(debug_id := NULL) -> BIGINT`.
- `symbols_version()`.

### Address model (caller contract — loud)

`address` is the **module-relative virtual address** (within the module image,
before ASLR slide / load bias). The caller — or the upstream
`vgi-minidump`/`vgi-pprof`/`vgi-perf` frame extractor, which owns the module map —
subtracts the module base first. Folding load-map handling in here is scope
creep. `module_info` reports the image's preferred base to help.

### Binder note (important)

The current DuckDB + `vgi` community extension only allows **literal** parameters
to table functions, so the spec's `… , LATERAL symbols.resolve(f.build_id,
f.address)` (per-row column params) is **rejected by the engine binder**. The
working bulk patterns are:
- scalar `symbolicate` / `function_name` over a frame column (and `UNNEST` of the
  `inlined` list for inline-expanded rows), and
- `resolve_batch` over an aggregated `LIST` (subquery or literal).
`resolve` itself works as a single-frame table form with literal args. The
`resolve` table-in-out is kept (it is correct and will light up if the binding
gains lateral support).

## The debug-info cache (the heart — `crates/symbols-core/src/cache/`)

- **Key = format-normalized debug-id** (`id.rs`): ELF GNU build-id → first 16
  bytes as a little-endian GUID + age 0; Mach-O `LC_UUID` (no swap); PDB GUID +
  age. The raw per-format build-id hex is retained for cross-keying/`debuginfod`.
  A frame keyed by *either* identifier resolves (alias map).
- **Resident LRU** (`cache/mod.rs`): bounded by `cache_max_bytes` (default 4 GiB)
  and `cache_max_modules` (default 256), whichever binds first; whole-module
  eviction by last-use. An evicted module keeps its manifest entry → re-parses
  from `origin`, never re-discovered.
- **Serializable manifest** (`cache/manifest.rs`): the externalized/durable state
  — source list + per-debug-id index (origin/format/arch/stats) + **negative
  cache** (a build-id proven missing → short-circuit). Plain serde data only;
  **never** a `ParsedModule`/mmap/handle in state (the live-artifact-in-state
  trap). The resident parsed modules are rebuilt lazily from `origin` on cold
  start. `state.rs` exports/imports it; the live cache is the process-global
  `Mutex<SymbolsState>` in `crates/symbols-worker/src/state.rs`.
- **Parse-once invariant**: `resolve` groups by debug-id and parses each module
  once; `cache_status.rows_resolved` rises while the parse counter does not. This
  is the moat — proved by `crates/symbols-core/tests/cache_behavior.rs`.

## Layout

- `crates/symbols-core` — pure compute, **no Arrow/VGI deps**. `id.rs` (debug-id
  derivation), `module/{dwarf,pdb}.rs` (resolution via `object`+`gimli`+`addr2line`
  / `pdb-addr2line`), `module/mod.rs` (bounded, `catch_unwind`-guarded parse +
  resolve), `cache/` (LRU + manifest), `source/` (dir/glob index, egress-gated
  remote), `demangle.rs`, `frame.rs`, `errors.rs`, `state.rs`. All correctness is
  unit-tested here.
- `crates/symbols-worker` — thin Arrow/VGI adapter: `scalar/`, `table/`,
  `schema.rs` (the resolved-row schema + STRUCT/LIST builders), `state.rs`
  (process-global cache), `catalog.rs`, `meta.rs`, `util.rs`. `main.rs` registers
  everything and calls `Worker::run()`.
- `crates/symbols-worker/fuzz` — `cargo-fuzz` targets (zero-panic gate).
- `test/sql/*.test` — haybarn SQLLogic e2e; `test/symbols/` holds the committed
  Mach-O dSYM fixtures.

## Build & test

```sh
cargo test --workspace            # core unit + golden DWARF + cache + proptest no-panic
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
cargo build --release             # build the worker
./run_tests.sh                    # haybarn SQLLogic e2e
cd crates/symbols-worker/fuzz && cargo +nightly fuzz run parse_module   # zero-panic gate
```

E2E needs the haybarn tooling (one-time):
```sh
uv tool install haybarn-unittest
echo "INSTALL vgi FROM community;" | uvx haybarn-cli
```
`run_tests.sh` builds the worker and points `VGI_SYMBOLS_WORKER` at the binary
and `VGI_SYMBOLS_DIR` at `test/symbols`.

### Regenerating the golden fixtures

`crates/symbols-core/tests/fixtures/*.dwarf` are real Mach-O `.dSYM` DWARF files
built with deliberate `always_inline` callees (so one address fans out to an
inline chain). Rebuild on macOS from the committed `prog.c` / `prog2.c`:
```sh
clang -g -O2 -c prog.c -o prog.o && clang prog.o -o prog && dsymutil prog -o prog.dSYM
cp prog.dSYM/Contents/Resources/DWARF/prog tests/fixtures/macho_inline.dwarf
```
Keep the `.o` so dsymutil can find the debug map (else the dSYM is empty). The
debug-id constants in the tests come from the new `LC_UUID`.

## Conventions / gotchas

- All algorithms go in `symbols-core` with unit tests; the worker is a thin
  adapter.
- Logs go to **stderr** — stdout is the Arrow-IPC channel.
- The catalog name must match the ATTACH name; `main.rs` defaults
  `VGI_WORKER_CATALOG_NAME` to `symbols`.
- VGI arg-type tokens are `uint64`/`varchar`/`blob`/… (NOT `ubigint`); an
  unknown token silently binds to NULL.
- The DWARF/PDB libraries can **panic** on adversarial input; `ParsedModule`
  wraps parse + resolve in `catch_unwind` → clean `error:<kind>` rows.
- Addresses are module-relative (caller subtracts the base).
- PDB types come from the `pdb2` fork re-exported as `pdb_addr2line::pdb` (the
  original `pdb` crate is a *different* type and must not be mixed in).
