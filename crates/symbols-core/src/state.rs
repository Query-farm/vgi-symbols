//! The worker's shared, stateful core: the ordered source registry + the
//! resident debug-info cache, with the per-frame resolve flow that interlocks
//! them. The worker crate wraps one `SymbolsState` in a process-global mutex;
//! every scalar/table function drives it through this surface.
//!
//! The resolve flow mirrors `vgi-cdc`'s "rehydrate state → do work → hand state
//! back" loop, but the state is the cache manifest:
//!
//! 1. resident hit → resolve in RAM;
//! 2. negative-cache hit → short-circuit to `not_found`;
//! 3. else locate via the source list, parse under the caps, insert into the
//!    LRU, then resolve.

use crate::cache::manifest::CacheManifest;
use crate::cache::manifest::SourceSpec;
use crate::cache::{StatusRow, SymbolCache, DEFAULT_MAX_BYTES, DEFAULT_MAX_MODULES};
use crate::errors::SymResult;
use crate::frame::{ModuleInfo, ResolvedFrame};
use crate::module::{inspect, Limits, ParsedModule};
use crate::source::SourceRegistry;

/// The full worker state: sources + cache + parse caps.
pub struct SymbolsState {
    sources: SourceRegistry,
    cache: SymbolCache,
    limits: Limits,
}

impl Default for SymbolsState {
    fn default() -> Self {
        SymbolsState::new()
    }
}

impl SymbolsState {
    /// A fresh state with default budgets (4 GiB / 256 modules) and no sources.
    pub fn new() -> SymbolsState {
        SymbolsState {
            sources: SourceRegistry::new(),
            cache: SymbolCache::new(DEFAULT_MAX_BYTES, DEFAULT_MAX_MODULES),
            limits: Limits::default(),
        }
    }

    /// A state with explicit cache budgets (used by eviction tests).
    pub fn with_budgets(max_bytes: u64, max_modules: usize) -> SymbolsState {
        SymbolsState {
            sources: SourceRegistry::new(),
            cache: SymbolCache::new(max_bytes, max_modules),
            limits: Limits::default(),
        }
    }

    // ---- config surface -------------------------------------------------

    /// Register a symbol source; returns the assigned `source_id`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_source(
        &mut self,
        kind: &str,
        path: Option<String>,
        url: Option<String>,
        bucket: Option<String>,
        enabled: Option<bool>,
        secret: Option<String>,
    ) -> Result<String, String> {
        self.sources.add(kind, path, url, bucket, enabled, secret)
    }

    /// The ordered source list (for `list_sources`).
    pub fn list_sources(&self) -> Vec<SourceSpec> {
        self.sources.specs()
    }

    /// Drop a source by id.
    pub fn drop_source(&mut self, source_id: &str) -> bool {
        self.sources.drop_source(source_id)
    }

    // ---- resolve surface ------------------------------------------------

    /// Ensure a module for `token` is resident, parsing it from the sources on a
    /// miss. Returns `Ok(true)` if a module is resident afterward, `Ok(false)`
    /// if no source has it (recorded as a negative-cache miss), or an error row
    /// kind if the located file was malformed.
    fn ensure_resident(&mut self, token: &str) -> Result<bool, crate::errors::ErrorKind> {
        if self.cache.is_resident(token) {
            return Ok(true);
        }
        if self.cache.is_known_miss(token) {
            return Ok(false);
        }
        match self.sources.locate(token) {
            Some(located) => match ParsedModule::parse(located.data, located.name, &self.limits) {
                Ok(module) => {
                    self.cache.insert(module, located.origin);
                    Ok(true)
                }
                Err(e) => Err(e.kind),
            },
            None => {
                self.cache.record_miss(token);
                Ok(false)
            }
        }
    }

    /// Resolve one `(build_id, address)` frame into inline-expanded rows. Always
    /// returns at least one row; never panics, never errors the scan.
    pub fn resolve(&mut self, build_id: &str, address: u64) -> Vec<ResolvedFrame> {
        if let Some(frames) = self.cache.resolve(build_id, address, &self.limits) {
            return frames;
        }
        match self.ensure_resident(build_id) {
            Ok(true) => self
                .cache
                .resolve(build_id, address, &self.limits)
                .unwrap_or_else(|| vec![ResolvedFrame::not_found()]),
            Ok(false) => vec![ResolvedFrame::not_found()],
            Err(kind) => vec![ResolvedFrame::error(kind, None)],
        }
    }

    /// The innermost function name only (fast path). `None` if not found.
    pub fn function_name(&mut self, build_id: &str, address: u64) -> Option<String> {
        if let Some(name) = self.cache.function_name(build_id, address) {
            return name;
        }
        match self.ensure_resident(build_id) {
            Ok(true) => self.cache.function_name(build_id, address).flatten(),
            _ => None,
        }
    }

    // ---- module_info ----------------------------------------------------

    /// Inspect a debug file given inline as a BLOB (no source/cache touched).
    pub fn module_info_blob(&self, data: Vec<u8>) -> SymResult<ModuleInfo> {
        inspect(data, "<blob>".to_string(), &self.limits)
    }

    /// Inspect a debug file at `path`.
    pub fn module_info_path(&self, path: &str) -> SymResult<ModuleInfo> {
        let data = std::fs::read(path).map_err(|e| {
            crate::errors::SymError::new(
                crate::errors::ErrorKind::Truncated,
                format!("read {path}: {e}"),
            )
        })?;
        let name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        inspect(data, name, &self.limits)
    }

    // ---- cache observability + control ---------------------------------

    /// The resident + manifest-only cache rows (for `cache_status`).
    pub fn cache_status(&self) -> Vec<StatusRow> {
        self.cache.status()
    }

    /// Force-evict one debug-id (or the whole resident set with `None`).
    pub fn cache_evict(&mut self, token: Option<&str>) -> i64 {
        self.cache.evict(token)
    }

    /// Module parses performed so far (the parse-once test probe).
    pub fn parse_count(&self) -> u64 {
        self.cache.parse_count()
    }

    // ---- externalized state (the manifest) -----------------------------

    /// Project the durable state into a serializable manifest (sources + index +
    /// negative cache) for hand-off across the scan-batch boundary.
    pub fn export_manifest(&mut self) -> CacheManifest {
        let sources = self.sources.specs();
        self.cache.to_manifest(sources)
    }

    /// Rehydrate the durable state from a manifest at cold start. The resident
    /// parsed-module set stays empty and is rebuilt lazily from `origin`.
    pub fn import_manifest(&mut self, manifest: &CacheManifest) {
        self.sources.restore(manifest.sources.clone());
        self.cache.rehydrate(manifest);
    }
}
