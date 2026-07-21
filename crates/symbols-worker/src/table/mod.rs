//! Table and config functions exposed by the symbols worker.

mod cache;
mod module_info;
mod sources;

use vgi::Worker;

/// Register every table / config function on the worker.
pub fn register(worker: &mut Worker) {
    // Module inspection (two overloads: BLOB and path).
    worker.register_table(module_info::ModuleInfoBlob);
    worker.register_table(module_info::ModuleInfoPath);

    // Cache observability + control.
    worker.register_table(cache::CacheStatus);
    worker.register_table(cache::CacheEvict);

    // Symbol-source config.
    worker.register_table(sources::AddSource);
    worker.register_table(sources::ListSources);
    worker.register_table(sources::DropSource);
}
