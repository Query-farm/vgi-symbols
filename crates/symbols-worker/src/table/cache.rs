//! Cache observability + control: `cache_status()` (what is parsed and resident
//! right now — the stateful part) and `cache_evict(debug_id := NULL)`.

use std::sync::Arc;

use arrow_array::builder::{
    BooleanBuilder, Int64Builder, StringBuilder, TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, Int64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use vgi::table_function::{TableFunction, TableProducer};
use vgi::{ArgSpec, BindParams, BindResponse, FunctionMetadata, ProcessParams};
use vgi_rpc::{OutputCollector, Result, RpcError};

use crate::state::with_state;

const UTC: &str = "UTC";

fn status_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("debug_id", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("format", DataType::Utf8, false),
        Field::new("arch", DataType::Utf8, false),
        Field::new("bytes_resident", DataType::Int64, false),
        Field::new("rows_resolved", DataType::Int64, false),
        Field::new(
            "last_used",
            DataType::Timestamp(TimeUnit::Microsecond, Some(UTC.into())),
            true,
        ),
        Field::new("origin", DataType::Utf8, true),
        Field::new("resident", DataType::Boolean, false),
    ]))
}

/// `cache_status()` — resident + manifest-only cache rows.
pub struct CacheStatus;

impl TableFunction for CacheStatus {
    fn name(&self) -> &str {
        "cache_status"
    }
    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Cache Status",
            "Report what debug modules are parsed and resident right now (the stateful heart of \
             this worker): one row per known debug-id — both modules resident in RAM and \
             manifest-only entries (evicted or proven-missing). Columns: `debug_id`, `name` (debug \
             file), `format`, `arch`, `bytes_resident` (0 if not resident), `rows_resolved` \
             (cumulative addresses resolved against it), `last_used`, `origin` (where it came from), \
             and `resident`. Order by `bytes_resident DESC` to see the hot modules. Use it to watch \
             the parse-once / LRU behavior of the cache.",
            "List the resident + known debug modules: debug_id, name, format, arch, \
             bytes_resident, rows_resolved, last_used, origin, resident. The cache-observability \
             surface.",
            "cache_status, cache, resident, parsed modules, observability, bytes_resident, \
             rows_resolved, debug_id, LRU",
        );
        tags.push(("vgi.category".into(), "Cache".into()));
        tags.push((
            "vgi.result_columns_schema".into(),
            // Static result schema (VGI307/VGI321), in `status_schema()` column
            // order so it matches DESCRIBE (VGI910). `last_used` is a
            // timezone-aware timestamp (TIMESTAMP WITH TIME ZONE).
            r#"[
  {"name": "debug_id", "type": "VARCHAR", "description": "Normalized debug-id cache key."},
  {"name": "name", "type": "VARCHAR", "description": "Debug file name."},
  {"name": "format", "type": "VARCHAR", "description": "Container format (ELF / MachO / PE / PDB / …)."},
  {"name": "arch", "type": "VARCHAR", "description": "CPU architecture."},
  {"name": "bytes_resident", "type": "BIGINT", "description": "Resident footprint in bytes (0 if evicted / manifest-only)."},
  {"name": "rows_resolved", "type": "BIGINT", "description": "Cumulative addresses resolved against this module."},
  {"name": "last_used", "type": "TIMESTAMPTZ", "description": "Timestamp of last use, or NULL if never used."},
  {"name": "origin", "type": "VARCHAR", "description": "Provenance the module is (re)parsed from (dir / glob / …), or NULL."},
  {"name": "resident", "type": "BOOLEAN", "description": "True if the parsed module is currently resident in RAM."}
]"#
                .into(),
        ));
        tags.push((
            "vgi.executable_examples".into(),
            r#"[{"description":"List the debug modules the worker has parsed and the manifest knows about, hottest first. Returns no rows on a cold worker (nothing parsed yet); pass resident_only => true to skip evicted/manifest-only entries.","sql":"SELECT debug_id, name, bytes_resident, rows_resolved FROM symbols.main.cache_status() ORDER BY bytes_resident DESC"}]"#
                .into(),
        ));
        FunctionMetadata {
            description:
                "Report the parsed/resident debug modules (the build-id-keyed cache state)".into(),
            examples: Vec::new(),
            tags,
            ..Default::default()
        }
    }
    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![ArgSpec::const_arg(
            "resident_only",
            -1,
            "boolean",
            "When true, return only modules currently resident in RAM and skip evicted / \
             manifest-only (and proven-missing) entries. Defaults to false (every known debug-id).",
        )
        .with_choices([true, false])]
    }
    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse {
            output_schema: status_schema(),
            opaque_data: Vec::new(),
        })
    }
    fn producer(&self, params: &ProcessParams) -> Result<Box<dyn TableProducer>> {
        let resident_only = params
            .arguments
            .named_bool("resident_only")
            .unwrap_or(false);
        let mut rows = with_state(|state| state.cache_status());
        if resident_only {
            rows.retain(|r| r.resident);
        }
        Ok(Box::new(StatusProducer {
            schema: params.output_schema.clone(),
            rows: Some(rows),
        }))
    }
}

struct StatusProducer {
    schema: SchemaRef,
    rows: Option<Vec<symbols_core::StatusRow>>,
}

impl TableProducer for StatusProducer {
    fn next_batch(&mut self, _out: &mut OutputCollector) -> Result<Option<RecordBatch>> {
        let Some(rows) = self.rows.take() else {
            return Ok(None);
        };
        let mut debug_id = StringBuilder::new();
        let mut name = StringBuilder::new();
        let mut format = StringBuilder::new();
        let mut arch = StringBuilder::new();
        let mut bytes_resident = Int64Builder::new();
        let mut rows_resolved = Int64Builder::new();
        let mut last_used = TimestampMicrosecondBuilder::new().with_timezone(UTC);
        let mut origin = StringBuilder::new();
        let mut resident = BooleanBuilder::new();

        for r in &rows {
            debug_id.append_value(&r.debug_id);
            name.append_value(&r.name);
            format.append_value(&r.format);
            arch.append_value(&r.arch);
            bytes_resident.append_value(r.bytes_resident);
            rows_resolved.append_value(r.rows_resolved);
            if r.last_used_epoch > 0 {
                last_used.append_value(r.last_used_epoch.saturating_mul(1_000_000));
            } else {
                last_used.append_null();
            }
            if r.origin.is_empty() {
                origin.append_null();
            } else {
                origin.append_value(&r.origin);
            }
            resident.append_value(r.resident);
        }

        let cols: Vec<ArrayRef> = vec![
            Arc::new(debug_id.finish()),
            Arc::new(name.finish()),
            Arc::new(format.finish()),
            Arc::new(arch.finish()),
            Arc::new(bytes_resident.finish()),
            Arc::new(rows_resolved.finish()),
            Arc::new(last_used.finish()),
            Arc::new(origin.finish()),
            Arc::new(resident.finish()),
        ];
        Ok(Some(
            RecordBatch::try_new(self.schema.clone(), cols)
                .map_err(|e| RpcError::runtime_error(e.to_string()))?,
        ))
    }
}

/// `cache_evict(debug_id := NULL)` — force-evict one module (or the whole
/// resident set), keeping the manifest. Returns bytes freed.
pub struct CacheEvict;

impl TableFunction for CacheEvict {
    fn name(&self) -> &str {
        "cache_evict"
    }
    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Cache Evict",
            "Force whole-module eviction from the resident debug-info cache, returning the number \
             of bytes freed. With `debug_id =>` set, evict just that module (e.g. after its symbol \
             file was updated on disk); with no argument, clear the entire resident set. Either way \
             the manifest entry is kept, so a later address re-parses the module from its origin \
             rather than re-discovering it. Use it to reclaim RAM or to pick up an updated symbol \
             file.",
            "Evict from the resident cache and return bytes freed: `cache_evict(debug_id => '…')` \
             evicts one module, `cache_evict()` clears the resident set (manifest kept).",
            "cache_evict, evict, LRU, reclaim, resident, debug_id, refresh symbols",
        );
        tags.push(("vgi.category".into(), "Cache".into()));
        tags.push((
            "vgi.result_columns_schema".into(),
            r#"[
  {"name": "bytes_freed", "type": "BIGINT", "description": "Resident bytes reclaimed by the eviction (0 if the module was not resident, or the cache was already empty)."}
]"#
                .into(),
        ));
        // Described example carried as `vgi.example_queries` (the native examples
        // carrier drops descriptions → VGI515).
        tags.push((
            "vgi.example_queries".into(),
            r#"[{"description":"Clear the entire resident cache (keeping the manifest) and report the bytes reclaimed; 0 on a cold worker with nothing resident.","sql":"SELECT bytes_freed FROM symbols.main.cache_evict()"}]"#
                .into(),
        ));
        FunctionMetadata {
            description: "Force-evict one module (or the whole resident set) from the cache; \
                          returns bytes freed"
                .into(),
            examples: Vec::new(),
            tags,
            ..Default::default()
        }
    }
    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![ArgSpec::const_arg(
            "debug_id",
            -1,
            "varchar",
            "The normalized debug-id (or build-id alias) to evict. Omit to clear the whole \
             resident set. The manifest entry is kept either way.",
        )]
    }
    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse {
            output_schema: Arc::new(Schema::new(vec![Field::new(
                "bytes_freed",
                DataType::Int64,
                false,
            )])),
            opaque_data: Vec::new(),
        })
    }
    fn producer(&self, params: &ProcessParams) -> Result<Box<dyn TableProducer>> {
        let token = params.arguments.const_str(0);
        let freed = with_state(|state| state.cache_evict(token.as_deref()));
        Ok(Box::new(OneI64 {
            schema: params.output_schema.clone(),
            value: Some(freed),
        }))
    }
}

/// Emits a single one-column BIGINT row, then nothing.
struct OneI64 {
    schema: SchemaRef,
    value: Option<i64>,
}

impl TableProducer for OneI64 {
    fn next_batch(&mut self, _out: &mut OutputCollector) -> Result<Option<RecordBatch>> {
        let Some(v) = self.value.take() else {
            return Ok(None);
        };
        let col: ArrayRef = Arc::new(Int64Array::from(vec![v]));
        Ok(Some(
            RecordBatch::try_new(self.schema.clone(), vec![col])
                .map_err(|e| RpcError::runtime_error(e.to_string()))?,
        ))
    }
}
