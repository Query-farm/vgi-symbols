//! The resident, build-id-keyed debug-info cache — the moat.
//!
//! Two halves interlock: a bounded **resident LRU** of parsed modules (the
//! rebuildable RAM artifact) and the serializable **manifest** (the durable
//! index + negative cache; see [`manifest`]). The resident set is bounded by two
//! budgets, whichever binds first: `cache_max_bytes` (default 4 GiB) and
//! `cache_max_modules` (default 256). Eviction is whole-module LRU by last-use;
//! an evicted module keeps its [`ManifestEntry`] so a later address re-parses
//! from `origin` rather than re-discovering it.
//!
//! Modules are stored **owned** and accessed only under the worker's global lock,
//! so a [`ParsedModule`] needs to be `Send` but not `Sync`.

pub mod manifest;

use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::time::{SystemTime, UNIX_EPOCH};

use lru::LruCache;

use crate::id::{canonical_token, Identity};
use crate::module::{Limits, ParsedModule};

pub use manifest::{CacheManifest, ManifestEntry, Origin, SourceSpec};

/// Default resident byte budget (4 GiB of parsed modules).
pub const DEFAULT_MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Default resident module-count budget.
pub const DEFAULT_MAX_MODULES: usize = 256;

/// Wall-clock epoch seconds (monotone enough for LRU bookkeeping).
fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A resident parsed module plus its live bookkeeping.
struct Resident {
    module: ParsedModule,
    bytes: u64,
    rows_resolved: u64,
    last_used_epoch: u64,
}

/// One row of `cache_status` (resident or manifest-only).
#[derive(Debug, Clone)]
pub struct StatusRow {
    /// Normalized cache key.
    pub debug_id: String,
    /// Debug file display name.
    pub name: String,
    /// Container format string.
    pub format: String,
    /// CPU architecture.
    pub arch: String,
    /// Resident byte footprint (0 for evicted/manifest-only).
    pub bytes_resident: i64,
    /// Cumulative addresses resolved against the module.
    pub rows_resolved: i64,
    /// Last-used epoch seconds.
    pub last_used_epoch: i64,
    /// Provenance label.
    pub origin: String,
    /// Whether the module is currently resident in RAM.
    pub resident: bool,
}

/// The resident cache + durable index.
pub struct SymbolCache {
    resident: LruCache<String, Resident>,
    /// token (canonical id alias) → debug_id; persists across eviction.
    aliases: HashMap<String, String>,
    /// debug_id → manifest entry; persists across eviction.
    entries: HashMap<String, ManifestEntry>,
    /// Canonical tokens proven missing (negative cache).
    negative: HashSet<String>,
    max_bytes: u64,
    resident_bytes: u64,
    /// Count of actual module parses (the parse-once invariant probe).
    parse_count: u64,
}

impl SymbolCache {
    /// Build an empty cache with the given budgets.
    pub fn new(max_bytes: u64, max_modules: usize) -> SymbolCache {
        let cap = NonZeroUsize::new(max_modules.max(1)).unwrap();
        SymbolCache {
            resident: LruCache::new(cap),
            aliases: HashMap::new(),
            entries: HashMap::new(),
            negative: HashSet::new(),
            max_bytes,
            resident_bytes: 0,
            parse_count: 0,
        }
    }

    /// Rehydrate the durable index (aliases / entries / negative cache) from a
    /// manifest. The resident set stays empty — modules are re-parsed lazily.
    pub fn rehydrate(&mut self, manifest: &CacheManifest) {
        for e in &manifest.entries {
            let token = canonical_token(&e.debug_id);
            self.aliases.insert(token.clone(), e.debug_id.clone());
            if e.miss {
                self.negative.insert(token);
            }
            self.entries.insert(e.debug_id.clone(), e.clone());
        }
    }

    /// Project the durable state into a manifest for hand-off across the batch
    /// boundary. Resident bookkeeping (rows/last_used) is folded back in first.
    pub fn to_manifest(&mut self, sources: Vec<SourceSpec>) -> CacheManifest {
        self.sync_entries();
        let mut entries: Vec<ManifestEntry> = self.entries.values().cloned().collect();
        entries.sort_by(|a, b| a.debug_id.cmp(&b.debug_id));
        CacheManifest { sources, entries }
    }

    /// Fold live resident stats into the persistent entries.
    fn sync_entries(&mut self) {
        for (debug_id, r) in self.resident.iter() {
            if let Some(e) = self.entries.get_mut(debug_id) {
                e.rows_resolved = r.rows_resolved;
                e.last_used_epoch = r.last_used_epoch;
            }
        }
    }

    /// Number of actual module parses performed (test probe for parse-once).
    pub fn parse_count(&self) -> u64 {
        self.parse_count
    }

    /// Resolve `debug_id` for an input token, if the alias is known.
    fn resolve_debug_id(&self, token: &str) -> Option<String> {
        let canon = canonical_token(token);
        self.aliases.get(&canon).cloned()
    }

    /// Whether `token` is a known negative-cache hit (proven missing).
    pub fn is_known_miss(&self, token: &str) -> bool {
        self.negative.contains(&canonical_token(token))
    }

    /// Record that `token` (a build-id we searched for) has no symbols, so a
    /// later batch short-circuits to `not_found` instead of re-scanning sources.
    pub fn record_miss(&mut self, token: &str) {
        let canon = canonical_token(token);
        self.aliases.entry(canon.clone()).or_insert(canon.clone());
        self.negative.insert(canon.clone());
        self.entries.entry(canon.clone()).or_insert(ManifestEntry {
            debug_id: canon,
            origin: None,
            format: None,
            arch: None,
            name: None,
            fetched_bytes: 0,
            rows_resolved: 0,
            last_used_epoch: now_epoch(),
            miss: true,
        });
    }

    /// Whether a module for `token` is currently resident.
    pub fn is_resident(&self, token: &str) -> bool {
        self.resolve_debug_id(token)
            .map(|id| self.resident.contains(&id))
            .unwrap_or(false)
    }

    /// Resolve `addr` against the resident module for `token`, promoting it in
    /// the LRU and bumping its row counter. Returns `None` if not resident.
    pub fn resolve(
        &mut self,
        token: &str,
        addr: u64,
        limits: &Limits,
    ) -> Option<Vec<crate::frame::ResolvedFrame>> {
        let debug_id = self.resolve_debug_id(token)?;
        let r = self.resident.get_mut(&debug_id)?;
        let frames = r.module.resolve(addr, limits);
        r.rows_resolved += 1;
        r.last_used_epoch = now_epoch();
        Some(frames)
    }

    /// The innermost function name for `token`/`addr` if resident.
    pub fn function_name(&mut self, token: &str, addr: u64) -> Option<Option<String>> {
        let debug_id = self.resolve_debug_id(token)?;
        let r = self.resident.get_mut(&debug_id)?;
        r.rows_resolved += 1;
        r.last_used_epoch = now_epoch();
        Some(r.module.function_name(addr))
    }

    /// Insert a freshly-parsed module under `origin`, registering all of its id
    /// aliases and a manifest entry, then evict to budget. Increments the parse
    /// counter. Returns the canonical debug-id it was stored under.
    pub fn insert(&mut self, module: ParsedModule, origin: Origin) -> String {
        self.parse_count += 1;
        let identity: &Identity = module.identity();
        let debug_id = identity.debug_id_str();
        let bytes = module.resident_bytes();
        let info = module.info();

        for alias in identity.aliases() {
            self.aliases.insert(alias, debug_id.clone());
        }
        self.negative.remove(&canonical_token(&debug_id));

        let now = now_epoch();
        self.entries.insert(
            debug_id.clone(),
            ManifestEntry {
                debug_id: debug_id.clone(),
                origin: Some(origin),
                format: Some(info.format.as_str().to_string()),
                arch: Some(info.arch.clone()),
                name: Some(module.name().to_string()),
                fetched_bytes: bytes,
                rows_resolved: 0,
                last_used_epoch: now,
                miss: false,
            },
        );

        if let Some(old) = self.resident.put(
            debug_id.clone(),
            Resident {
                module,
                bytes,
                rows_resolved: 0,
                last_used_epoch: now,
            },
        ) {
            self.resident_bytes = self.resident_bytes.saturating_sub(old.bytes);
        }
        self.resident_bytes += bytes;
        self.evict_to_budget();
        debug_id
    }

    /// Evict whole modules (LRU) until both budgets are satisfied.
    fn evict_to_budget(&mut self) {
        while self.resident_bytes > self.max_bytes && self.resident.len() > 1 {
            if let Some((id, r)) = self.resident.pop_lru() {
                self.resident_bytes = self.resident_bytes.saturating_sub(r.bytes);
                self.fold_back(&id, &r);
            } else {
                break;
            }
        }
    }

    /// Persist an evicted module's live stats back into its manifest entry.
    fn fold_back(&mut self, id: &str, r: &Resident) {
        if let Some(e) = self.entries.get_mut(id) {
            e.rows_resolved = r.rows_resolved;
            e.last_used_epoch = r.last_used_epoch;
        }
    }

    /// Force-evict one debug-id (or, with `None`, the whole resident set, keeping
    /// the manifest). Returns bytes freed.
    pub fn evict(&mut self, token: Option<&str>) -> i64 {
        match token {
            Some(t) => {
                let Some(debug_id) = self.resolve_debug_id(t) else {
                    return 0;
                };
                if let Some(r) = self.resident.pop(&debug_id) {
                    self.resident_bytes = self.resident_bytes.saturating_sub(r.bytes);
                    self.fold_back(&debug_id, &r);
                    return r.bytes as i64;
                }
                0
            }
            None => {
                let mut freed = 0i64;
                while let Some((id, r)) = self.resident.pop_lru() {
                    freed += r.bytes as i64;
                    self.fold_back(&id, &r);
                }
                self.resident_bytes = 0;
                freed
            }
        }
    }

    /// All known modules — resident first (with live stats), then manifest-only
    /// (evicted or cold) entries — for `cache_status`.
    pub fn status(&self) -> Vec<StatusRow> {
        let mut rows = Vec::new();
        for (debug_id, r) in self.resident.iter() {
            let info = r.module.info();
            rows.push(StatusRow {
                debug_id: debug_id.clone(),
                name: r.module.name().to_string(),
                format: info.format.as_str().to_string(),
                arch: info.arch,
                bytes_resident: r.bytes as i64,
                rows_resolved: r.rows_resolved as i64,
                last_used_epoch: r.last_used_epoch as i64,
                origin: self
                    .entries
                    .get(debug_id)
                    .and_then(|e| e.origin.as_ref().map(|o| o.label()))
                    .unwrap_or_default(),
                resident: true,
            });
        }
        for (debug_id, e) in &self.entries {
            if self.resident.contains(debug_id) {
                continue;
            }
            rows.push(StatusRow {
                debug_id: debug_id.clone(),
                name: e.name.clone().unwrap_or_default(),
                format: e.format.clone().unwrap_or_default(),
                arch: e.arch.clone().unwrap_or_default(),
                bytes_resident: 0,
                rows_resolved: e.rows_resolved as i64,
                last_used_epoch: e.last_used_epoch as i64,
                origin: e.origin.as_ref().map(|o| o.label()).unwrap_or_else(|| {
                    if e.miss {
                        "not_found".into()
                    } else {
                        String::new()
                    }
                }),
                resident: false,
            });
        }
        rows
    }
}
