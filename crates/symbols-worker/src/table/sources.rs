//! Symbol-source config: `add_source(...)`, `sources()`, `drop_source(id)`.
//! Invoked via `CALL symbols.add_source('dir', path => '/srv/debug')` or `SELECT
//! * FROM symbols.sources()`. Local sources are zero-egress; remote sources
//! (`debuginfod`/`s3`/`http`) register with `egress=true` and default disabled.

use std::sync::Arc;

use arrow_array::builder::{BooleanBuilder, StringBuilder};
use arrow_array::{ArrayRef, BooleanArray, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use vgi::table_function::{TableFunction, TableProducer};
use vgi::{ArgSpec, BindParams, BindResponse, FunctionMetadata, ProcessParams};
use vgi_rpc::{OutputCollector, Result, RpcError};

use crate::state::with_state;

/// `add_source(kind, path :=, url :=, bucket :=, enabled :=, secret :=)`.
pub struct AddSource;

impl TableFunction for AddSource {
    fn name(&self) -> &str {
        "add_source"
    }
    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Add Symbol Source",
            "Register where debug files live so `symbolicate` can find them, returning \
             the assigned `source_id`. Sources are tried in the order added; the first debug-id \
             match wins. Local, zero-egress kinds: `dir` (`path =>` a directory of symbol files) \
             and `glob` (`path =>` a recursive glob like '/builds/**/*.{debug,pdb,dSYM}'). Remote, \
             egress kinds (opt-in, default disabled): `debuginfod` (`url =>`), `http` (`url =>`), \
             and `s3` (`bucket =>`) — pass `enabled => true` to turn one on; credentials come from \
             the SDK secret provider via `secret =>`, never inline. The default posture is \
             air-gap-safe: with no remote source enabled there is zero network egress.",
            "Register a symbol source and get its source_id. `add_source('dir', path => '/srv/debug')` \
             or `add_source('glob', path => '/b/**/*.debug')`; remote kinds (debuginfod/s3/http) are \
             opt-in (`enabled => true`) with secrets via `secret =>`.",
            "add_source, symbol source, dir, glob, debuginfod, s3, http, debug files, egress, \
             secret, data residency",
        );
        tags.push(("vgi.category".into(), "Sources".into()));
        tags.push((
            "vgi.result_columns_schema".into(),
            r#"[
  {"name": "source_id", "type": "VARCHAR", "description": "The id assigned to the new source (e.g. src0). Pass it to drop_source to remove it."}
]"#
                .into(),
        ));
        tags.push((
            "vgi.executable_examples".into(),
            r#"[{"description":"Register a local, zero-egress directory of symbol files and capture its assigned source_id (use `add_source('glob', path => '/builds/**/*.{debug,pdb}')` for a recursive glob, or a remote kind like 'debuginfod'/'s3'/'http' with `enabled => true`).","sql":"SELECT source_id FROM symbols.main.add_source('dir', path => '/srv/debug')"}]"#
                .into(),
        ));
        FunctionMetadata {
            description: "Register a symbol source (dir/glob/debuginfod/s3/http); returns its \
                          source_id"
                .into(),
            examples: Vec::new(),
            tags,
            ..Default::default()
        }
    }
    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![
            ArgSpec::const_arg(
                "kind",
                0,
                "varchar",
                "Source kind: 'dir' / 'glob' (local, zero-egress) or 'debuginfod' / 's3' / 'http' \
                 (remote, opt-in egress).",
            ),
            ArgSpec::const_arg(
                "path",
                -1,
                "varchar",
                "Filesystem path (for `dir`) or recursive glob (for `glob`).",
            ),
            ArgSpec::const_arg(
                "url",
                -1,
                "varchar",
                "Base URL for a `debuginfod` or `http` source.",
            ),
            ArgSpec::const_arg("bucket", -1, "varchar", "Bucket name for an `s3` source."),
            ArgSpec::const_arg(
                "enabled",
                -1,
                "boolean",
                "Whether the source is active. Local sources default true; remote sources default \
                 false (must be explicitly enabled to allow egress).",
            )
            .with_choices([true, false]),
            ArgSpec::const_arg(
                "secret",
                -1,
                "varchar",
                "Name of the SDK secret carrying credentials for a remote source (never the \
                 secret value itself).",
            ),
        ]
    }
    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse {
            output_schema: Arc::new(Schema::new(vec![Field::new(
                "source_id",
                DataType::Utf8,
                false,
            )])),
            opaque_data: Vec::new(),
        })
    }
    fn producer(&self, params: &ProcessParams) -> Result<Box<dyn TableProducer>> {
        let args = &params.arguments;
        let kind = args
            .const_str(0)
            .ok_or_else(|| RpcError::value_error("add_source: kind is required"))?;
        let path = args.named_str("path");
        let url = args.named_str("url");
        let bucket = args.named_str("bucket");
        let enabled = args.named_bool("enabled");
        let secret = args.named_str("secret");
        let source_id =
            with_state(|state| state.add_source(&kind, path, url, bucket, enabled, secret))
                .map_err(RpcError::value_error)?;
        Ok(Box::new(OneString {
            schema: params.output_schema.clone(),
            value: Some(source_id),
        }))
    }
}

/// `sources()` — list the registered symbol sources.
pub struct ListSources;

impl TableFunction for ListSources {
    fn name(&self) -> &str {
        "sources"
    }
    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Symbol Sources",
            "List the registered symbol sources in resolve order: `source_id`, `kind`, `location` \
             (the path / url / bucket), `enabled`, and `egress` (whether using the source crosses \
             the trust boundary). Use it to audit data residency — which sources are local \
             (`egress=false`) versus remote, and which are enabled.",
            "List registered symbol sources: source_id, kind, location, enabled, egress (in \
             resolve order).",
            "sources, list sources, audit, data residency, egress, enabled",
        );
        tags.push(("vgi.category".into(), "Sources".into()));
        tags.push((
            "vgi.result_columns_schema".into(),
            r#"[
  {"name": "source_id", "type": "VARCHAR", "description": "Opaque id from add_source."},
  {"name": "kind", "type": "VARCHAR", "description": "Source kind: dir / glob / debuginfod / s3 / http."},
  {"name": "location", "type": "VARCHAR", "description": "The source's locator: path / url / bucket."},
  {"name": "enabled", "type": "BOOLEAN", "description": "True if the source is active."},
  {"name": "egress", "type": "BOOLEAN", "description": "True if using the source crosses the trust boundary (remote)."}
]"#
                .into(),
        ));
        tags.push((
            "vgi.executable_examples".into(),
            r#"[{"description":"Audit the registered symbol sources in resolve order, including whether each is enabled and whether using it crosses the trust boundary (egress). Returns no rows until a source is registered with add_source; pass enabled_only => true to list only active sources.","sql":"SELECT source_id, kind, location, enabled, egress FROM symbols.main.sources()"}]"#
                .into(),
        ));
        FunctionMetadata {
            description: "List the registered symbol sources (source_id, kind, location, enabled, \
                          egress)"
                .into(),
            examples: Vec::new(),
            tags,
            ..Default::default()
        }
    }
    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![ArgSpec::const_arg(
            "enabled_only",
            -1,
            "boolean",
            "When true, return only sources that are currently enabled (active). Defaults to false \
             (every registered source, enabled or not).",
        )
        .with_choices([true, false])]
    }
    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse {
            output_schema: Arc::new(Schema::new(vec![
                Field::new("source_id", DataType::Utf8, false),
                Field::new("kind", DataType::Utf8, false),
                Field::new("location", DataType::Utf8, false),
                Field::new("enabled", DataType::Boolean, false),
                Field::new("egress", DataType::Boolean, false),
            ])),
            opaque_data: Vec::new(),
        })
    }
    fn producer(&self, params: &ProcessParams) -> Result<Box<dyn TableProducer>> {
        let enabled_only = params.arguments.named_bool("enabled_only").unwrap_or(false);
        let mut specs = with_state(|state| state.list_sources());
        if enabled_only {
            specs.retain(|s| s.enabled);
        }
        Ok(Box::new(SourcesProducer {
            schema: params.output_schema.clone(),
            specs: Some(specs),
        }))
    }
}

struct SourcesProducer {
    schema: SchemaRef,
    specs: Option<Vec<symbols_core::SourceSpec>>,
}

impl TableProducer for SourcesProducer {
    fn next_batch(&mut self, _out: &mut OutputCollector) -> Result<Option<RecordBatch>> {
        let Some(specs) = self.specs.take() else {
            return Ok(None);
        };
        let mut id = StringBuilder::new();
        let mut kind = StringBuilder::new();
        let mut location = StringBuilder::new();
        let mut enabled = BooleanBuilder::new();
        let mut egress = BooleanBuilder::new();
        for s in &specs {
            id.append_value(&s.source_id);
            kind.append_value(&s.kind);
            location.append_value(s.location());
            enabled.append_value(s.enabled);
            egress.append_value(s.egress);
        }
        let cols: Vec<ArrayRef> = vec![
            Arc::new(id.finish()),
            Arc::new(kind.finish()),
            Arc::new(location.finish()),
            Arc::new(enabled.finish()),
            Arc::new(egress.finish()),
        ];
        Ok(Some(
            RecordBatch::try_new(self.schema.clone(), cols)
                .map_err(|e| RpcError::runtime_error(e.to_string()))?,
        ))
    }
}

/// `drop_source(source_id)`.
pub struct DropSource;

impl TableFunction for DropSource {
    fn name(&self) -> &str {
        "drop_source"
    }
    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Drop Symbol Source",
            "Remove a previously-registered symbol source by its `source_id`, returning whether a \
             source was actually removed. The local file index is rebuilt lazily on the next \
             resolve.",
            "Drop a symbol source by id: `drop_source('src0')`. Returns `dropped` (whether one was \
             removed).",
            "drop_source, remove source, source_id, deregister",
        );
        tags.push(("vgi.category".into(), "Sources".into()));
        tags.push((
            "vgi.result_columns_schema".into(),
            r#"[
  {"name": "dropped", "type": "BOOLEAN", "description": "True if a source with that source_id existed and was removed; false if no such source was registered."}
]"#
                .into(),
        ));
        // Described example carried as `vgi.example_queries` (the native examples
        // carrier drops descriptions → VGI515).
        tags.push((
            "vgi.example_queries".into(),
            r#"[{"description":"Deregister the symbol source with id 'src0'; dropped is false if no such source was registered.","sql":"SELECT dropped FROM symbols.main.drop_source('src0')"}]"#
                .into(),
        ));
        FunctionMetadata {
            description: "Remove a registered symbol source by id; returns whether one was removed"
                .into(),
            examples: Vec::new(),
            tags,
            ..Default::default()
        }
    }
    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![ArgSpec::const_arg(
            "source_id",
            0,
            "varchar",
            "The `source_id` returned by `add_source` to remove.",
        )]
    }
    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse {
            output_schema: Arc::new(Schema::new(vec![Field::new(
                "dropped",
                DataType::Boolean,
                false,
            )])),
            opaque_data: Vec::new(),
        })
    }
    fn producer(&self, params: &ProcessParams) -> Result<Box<dyn TableProducer>> {
        let id = params
            .arguments
            .const_str(0)
            .ok_or_else(|| RpcError::value_error("drop_source: source_id is required"))?;
        let dropped = with_state(|state| state.drop_source(&id));
        Ok(Box::new(OneBool {
            schema: params.output_schema.clone(),
            value: Some(dropped),
        }))
    }
}

/// Emits a single one-column VARCHAR row.
struct OneString {
    schema: SchemaRef,
    value: Option<String>,
}
impl TableProducer for OneString {
    fn next_batch(&mut self, _out: &mut OutputCollector) -> Result<Option<RecordBatch>> {
        let Some(v) = self.value.take() else {
            return Ok(None);
        };
        let col: ArrayRef = Arc::new(StringArray::from(vec![v]));
        Ok(Some(
            RecordBatch::try_new(self.schema.clone(), vec![col])
                .map_err(|e| RpcError::runtime_error(e.to_string()))?,
        ))
    }
}

/// Emits a single one-column BOOLEAN row.
struct OneBool {
    schema: SchemaRef,
    value: Option<bool>,
}
impl TableProducer for OneBool {
    fn next_batch(&mut self, _out: &mut OutputCollector) -> Result<Option<RecordBatch>> {
        let Some(v) = self.value.take() else {
            return Ok(None);
        };
        let col: ArrayRef = Arc::new(BooleanArray::from(vec![v]));
        Ok(Some(
            RecordBatch::try_new(self.schema.clone(), vec![col])
                .map_err(|e| RpcError::runtime_error(e.to_string()))?,
        ))
    }
}
