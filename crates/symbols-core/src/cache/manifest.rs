//! The serializable cache **manifest** — the externalized/durable state.
//!
//! Like `vgi-cdc` carries one `applied_lsn` and nothing live, vgi-symbols
//! carries this serde-serializable manifest between scan batches — never the
//! parsed in-memory module tree (which holds mmaps / borrowed buffers and is not
//! serializable). The manifest is plain data: which debug-ids are *known*, where
//! each came from (so a warm remote-fetched file is re-located without
//! re-querying a remote source), and the **negative cache** (a build-id we
//! already proved we have no symbols for → don't re-scan every source for it).
//!
//! What is **not** serialized: the [`crate::module::ParsedModule`] itself. Those
//! are reconstructible from `origin` and rebuilt lazily on the first lookup of
//! that debug-id in a fresh process. Durable *index* in the manifest;
//! rebuildable *compute artifact* in RAM.

use serde::{Deserialize, Serialize};

/// Where a debug file for a given debug-id was located. Used to re-locate a
/// module on cold start (or after LRU eviction) without re-discovering it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    /// Found under a registered directory source.
    Dir {
        /// Absolute path of the debug file.
        path: String,
    },
    /// Matched by a registered glob source.
    Glob {
        /// The concrete path the glob matched.
        matched_path: String,
    },
    /// Fetched from a debuginfod server (egress).
    Debuginfod {
        /// The server base URL.
        url: String,
    },
    /// Fetched from an S3 symbol store (egress).
    S3 {
        /// Bucket name.
        bucket: String,
        /// Object key.
        key: String,
    },
    /// Fetched over HTTP (egress).
    Http {
        /// The object URL.
        url: String,
    },
    /// Supplied inline as a BLOB column (no provenance path).
    Inline,
}

impl Origin {
    /// A short human label for the `origin` column of `cache_status`.
    pub fn label(&self) -> String {
        match self {
            Origin::Dir { path } => format!("dir:{path}"),
            Origin::Glob { matched_path } => format!("glob:{matched_path}"),
            Origin::Debuginfod { url } => format!("debuginfod:{url}"),
            Origin::S3 { bucket, key } => format!("s3:{bucket}/{key}"),
            Origin::Http { url } => format!("http:{url}"),
            Origin::Inline => "inline".to_string(),
        }
    }
}

/// A registered symbol source (serialized so a fresh process restores config).
/// No secrets are stored inline — only the *name* of the SDK secret to resolve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpec {
    /// Opaque id returned by `add_source`.
    pub source_id: String,
    /// `dir` | `glob` | `inline` | `debuginfod` | `s3` | `http`.
    pub kind: String,
    /// Filesystem path / glob, for local sources.
    pub path: Option<String>,
    /// Base URL, for remote sources.
    pub url: Option<String>,
    /// Bucket, for S3 sources.
    pub bucket: Option<String>,
    /// Whether the source is enabled (remote sources default off).
    pub enabled: bool,
    /// Whether using the source egresses the trust boundary.
    pub egress: bool,
    /// Name of the SDK secret carrying credentials, if any (never the secret).
    pub secret: Option<String>,
}

impl SourceSpec {
    /// The human location string for `list_sources`.
    pub fn location(&self) -> String {
        self.path
            .clone()
            .or_else(|| self.url.clone())
            .or_else(|| self.bucket.clone())
            .unwrap_or_default()
    }
}

/// One manifest entry — one debug-id ever resolved (or proven missing) this
/// session. Monotone: entries are only added or refreshed, never lost, so batch
/// redelivery is idempotent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// The normalized cache key.
    pub debug_id: String,
    /// Where it came from (None until located).
    pub origin: Option<Origin>,
    /// Container format string (e.g. `ELF`, `PDB`).
    pub format: Option<String>,
    /// CPU architecture.
    pub arch: Option<String>,
    /// The debug file's display name.
    pub name: Option<String>,
    /// Bytes read for this module.
    pub fetched_bytes: u64,
    /// Cumulative addresses resolved against this module.
    pub rows_resolved: u64,
    /// Last-used wall-clock epoch seconds.
    pub last_used_epoch: u64,
    /// Negative-cache marker: a build-id we looked for and could NOT find.
    pub miss: bool,
}

/// The whole serializable manifest: ordered sources + per-debug-id entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheManifest {
    /// Ordered sources (resolve order = insertion order).
    pub sources: Vec<SourceSpec>,
    /// One entry per debug-id (or alias) ever seen.
    pub entries: Vec<ManifestEntry>,
}

impl CacheManifest {
    /// Serialize the manifest to JSON bytes (the scan-state blob).
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Restore a manifest from scan-state bytes; an empty/corrupt blob restores
    /// the empty manifest (degrade to a cold start, never panic).
    pub fn from_bytes(bytes: &[u8]) -> CacheManifest {
        if bytes.is_empty() {
            return CacheManifest::default();
        }
        serde_json::from_slice(bytes).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_roundtrips() {
        let m = CacheManifest {
            sources: vec![SourceSpec {
                source_id: "s1".into(),
                kind: "dir".into(),
                path: Some("/srv/debug".into()),
                url: None,
                bucket: None,
                enabled: true,
                egress: false,
                secret: None,
            }],
            entries: vec![ManifestEntry {
                debug_id: "abc".into(),
                origin: Some(Origin::Dir {
                    path: "/srv/debug/x".into(),
                }),
                format: Some("ELF".into()),
                arch: Some("x86_64".into()),
                name: Some("x".into()),
                fetched_bytes: 10,
                rows_resolved: 3,
                last_used_epoch: 99,
                miss: false,
            }],
        };
        let bytes = m.to_bytes();
        let back = CacheManifest::from_bytes(&bytes);
        assert_eq!(back.sources.len(), 1);
        assert_eq!(back.entries.len(), 1);
        assert_eq!(back.entries[0].debug_id, "abc");
    }

    #[test]
    fn corrupt_bytes_degrade_to_empty() {
        let m = CacheManifest::from_bytes(b"{not json");
        assert!(m.sources.is_empty() && m.entries.is_empty());
    }
}
