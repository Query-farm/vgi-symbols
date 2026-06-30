//! The ordered symbol-source registry: where debug files come from.
//!
//! Resolve order is the order sources were added; first debug-id match wins;
//! misses fall through to the next source and finally to the negative cache.
//! Local sources (`dir`, `glob`) are **zero-egress** and indexed by debug-id on
//! first use. Remote sources (`debuginfod`, `s3`, `http`) are **opt-in and off
//! by default**; in this build they register and surface in `list_sources` with
//! `egress=true` but are not fetched (the air-gap-safe default), and their
//! credentials would flow only through the SDK secret provider, never inline.

pub mod local;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::cache::manifest::{Origin, SourceSpec};
use crate::id::canonical_token;

/// How many files indexing will visit per local source before stopping (a
/// denial-of-service backstop against a hostile directory).
const MAX_INDEX_FILES: usize = 1_000_000;

/// A located debug file ready to parse: its bytes, display name, and provenance.
pub struct Located {
    /// The whole debug file bytes.
    pub data: Vec<u8>,
    /// Display name (basename) for the `module` column.
    pub name: String,
    /// Where it came from (for the manifest / `cache_status`).
    pub origin: Origin,
}

/// The ordered set of symbol sources plus the lazily-built local index.
#[derive(Default)]
pub struct SourceRegistry {
    sources: Vec<SourceSpec>,
    next_id: u64,
    /// token → (path, is_glob); built lazily from enabled local sources.
    index: HashMap<String, (PathBuf, bool)>,
    /// source_ids already indexed (so re-locating doesn't re-walk the disk).
    indexed: HashSet<String>,
}

impl SourceRegistry {
    /// An empty registry (no sources — the air-gapped default).
    pub fn new() -> SourceRegistry {
        SourceRegistry::default()
    }

    /// Restore the ordered source list from a manifest (cold start). The local
    /// index is rebuilt lazily on the next `locate`.
    pub fn restore(&mut self, specs: Vec<SourceSpec>) {
        self.sources = specs;
        self.index.clear();
        self.indexed.clear();
        // Keep next_id ahead of any restored id to avoid collisions.
        for s in &self.sources {
            if let Some(n) = s
                .source_id
                .strip_prefix("src")
                .and_then(|n| n.parse::<u64>().ok())
            {
                self.next_id = self.next_id.max(n + 1);
            }
        }
    }

    /// The current ordered source specs (for the manifest / `list_sources`).
    pub fn specs(&self) -> Vec<SourceSpec> {
        self.sources.clone()
    }

    /// Register a new source. Returns the assigned `source_id`. Validates that
    /// the kind's required locator (path / url / bucket) is present.
    pub fn add(
        &mut self,
        kind: &str,
        path: Option<String>,
        url: Option<String>,
        bucket: Option<String>,
        enabled: Option<bool>,
        secret: Option<String>,
    ) -> Result<String, String> {
        let kind = kind.trim().to_ascii_lowercase();
        let egress = matches!(kind.as_str(), "debuginfod" | "s3" | "http");
        match kind.as_str() {
            "dir" | "glob" => {
                if path.as_deref().unwrap_or("").is_empty() {
                    return Err(format!("source kind '{kind}' requires path =>"));
                }
            }
            "debuginfod" | "http" => {
                if url.as_deref().unwrap_or("").is_empty() {
                    return Err(format!("source kind '{kind}' requires url =>"));
                }
            }
            "s3" => {
                if bucket.as_deref().unwrap_or("").is_empty() {
                    return Err("source kind 's3' requires bucket =>".to_string());
                }
            }
            "inline" => {}
            other => return Err(format!("unknown source kind '{other}'")),
        }
        // Remote sources default disabled; local default enabled.
        let enabled = enabled.unwrap_or(!egress);
        let source_id = format!("src{}", self.next_id);
        self.next_id += 1;
        self.sources.push(SourceSpec {
            source_id: source_id.clone(),
            kind,
            path,
            url,
            bucket,
            enabled,
            egress,
            secret,
        });
        Ok(source_id)
    }

    /// Drop a source by id; returns whether one was removed.
    pub fn drop_source(&mut self, source_id: &str) -> bool {
        let before = self.sources.len();
        self.sources.retain(|s| s.source_id != source_id);
        let removed = self.sources.len() != before;
        if removed {
            // The index may have pointed into the dropped source; rebuild lazily.
            self.index.clear();
            self.indexed.clear();
        }
        removed
    }

    /// Ensure every enabled local source has been indexed.
    fn ensure_indexed(&mut self) {
        // Clone the specs we need so we don't borrow self while mutating index.
        let pending: Vec<SourceSpec> = self
            .sources
            .iter()
            .filter(|s| s.enabled && matches!(s.kind.as_str(), "dir" | "glob"))
            .filter(|s| !self.indexed.contains(&s.source_id))
            .cloned()
            .collect();
        for spec in pending {
            let Some(p) = spec.path.as_deref() else {
                self.indexed.insert(spec.source_id.clone());
                continue;
            };
            let is_glob = spec.kind == "glob";
            let entries = if is_glob {
                local::index_glob(p, MAX_INDEX_FILES)
            } else {
                local::index_dir(p, MAX_INDEX_FILES)
            };
            for ix in entries {
                for alias in ix.aliases {
                    // First match wins (source order); don't clobber an earlier
                    // source's entry.
                    self.index
                        .entry(alias)
                        .or_insert_with(|| (ix.path.clone(), is_glob));
                }
            }
            self.indexed.insert(spec.source_id);
        }
    }

    /// Locate a debug file for `token` (a build-id or debug-id). Tries local
    /// sources in order; returns the bytes + provenance of the first match, or
    /// `None` if no enabled source has it. Remote sources are not fetched in this
    /// build (the air-gap-safe default).
    pub fn locate(&mut self, token: &str) -> Option<Located> {
        self.ensure_indexed();
        let canon = canonical_token(token);
        let (path, is_glob) = self.index.get(&canon)?.clone();
        let data = std::fs::read(&path).ok()?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| canon.clone());
        let origin = if is_glob {
            Origin::Glob {
                matched_path: path.to_string_lossy().into_owned(),
            }
        } else {
            Origin::Dir {
                path: path.to_string_lossy().into_owned(),
            }
        };
        Some(Located { data, name, origin })
    }
}
