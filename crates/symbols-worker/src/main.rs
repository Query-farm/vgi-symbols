// Copyright 2026 Query Farm LLC - https://query.farm

//! The `symbols` VGI worker.
//!
//! A standalone binary DuckDB launches and talks to over Apache Arrow IPC. It
//! brings native (DWARF/PDB) symbolication to SQL under the catalog `symbols`,
//! schema `main`, as a bulk JOIN over a column of `(build_id, address)` frames
//! backed by a persistent, build-id-keyed debug-info cache:
//!
//! - `symbols.main.symbolicate(build_id, address)` — one frame → STRUCT (inline chain collapsed)
//! - `symbols.main.resolve(build_id, address)` — LATERAL: inline-expanded rows (the JOIN surface)
//! - `symbols.main.resolve_batch(frames)` — a whole LIST of frames in one pass
//! - `symbols.main.function_name` / `inline_frames` / `demangle` — scalar conveniences
//! - `symbols.main.module_info` / `cache_status` / `cache_evict` — inspect modules + cache
//! - `symbols.add_source` / `list_sources` / `drop_source` — where debug files come from

mod catalog;
mod meta;
mod scalar;
mod schema;
mod state;
mod table;
mod util;

use vgi::Worker;

fn main() {
    // Logs MUST go to stderr — stdout is the Arrow-IPC channel.
    let _ = env_logger::Builder::from_env(env_logger::Env::default().filter_or("VGI_LOG", "info"))
        .format_timestamp_millis()
        .try_init();

    // The catalog name DuckDB sees in `ATTACH 'symbols' (TYPE vgi, …)`. Default
    // to `symbols`, but honor an explicit override so a test harness can rename.
    if std::env::var_os("VGI_WORKER_CATALOG_NAME").is_none() {
        std::env::set_var("VGI_WORKER_CATALOG_NAME", "symbols");
    }
    let catalog_name =
        std::env::var("VGI_WORKER_CATALOG_NAME").unwrap_or_else(|_| "symbols".to_string());

    let mut worker = Worker::new();
    scalar::register(&mut worker);
    table::register(&mut worker);
    worker.set_catalog(catalog::catalog_metadata(&catalog_name));
    worker.run();
}
