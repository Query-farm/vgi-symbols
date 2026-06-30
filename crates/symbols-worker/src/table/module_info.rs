//! `module_info(blob)` / `module_info(path)` — inspect a candidate debug file
//! without resolving (and without paying the parse-into-cache cost): is this a
//! debug file, what is its debug-id, and does it actually have line info?

use std::sync::Arc;

use arrow_array::builder::{BooleanBuilder, Int64Builder, StringBuilder, UInt64Builder};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use symbols_core::frame::ModuleInfo;
use vgi::table_function::{TableFunction, TableProducer};
use vgi::{ArgSpec, BindParams, BindResponse, FunctionMetadata, ProcessParams};
use vgi_rpc::{OutputCollector, Result, RpcError};

use crate::state::with_state;
use crate::util::const_blob;

/// The fixed `module_info` output schema.
fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("format", DataType::Utf8, false),
        Field::new("arch", DataType::Utf8, false),
        Field::new("build_id", DataType::Utf8, true),
        Field::new("debug_id", DataType::Utf8, true),
        Field::new("code_id", DataType::Utf8, true),
        Field::new("has_dwarf", DataType::Boolean, false),
        Field::new("has_pdb", DataType::Boolean, false),
        Field::new("has_line_table", DataType::Boolean, false),
        Field::new("symbol_count", DataType::Int64, false),
        Field::new("file_count", DataType::Int64, false),
        Field::new("byte_size", DataType::Int64, false),
        Field::new("image_base", DataType::UInt64, false),
    ]))
}

fn columns_md() -> (String, String, String, String) {
    (
        "Inspect a debug file (passed inline as a BLOB or by file path) without resolving any \
         frames: returns one row describing the container `format` ('ELF' / 'MachO' / 'PE' / \
         'PDB' / 'dSYM' / 'Breakpad'), the CPU `arch`, the raw per-format `build_id` hex and the \
         normalized `debug_id` cache key (so you can check whether this file matches the frames you \
         have), the `code_id`, the `has_dwarf` / `has_pdb` / `has_line_table` capability flags, the \
         `symbol_count` / `file_count` / `byte_size`, and the image's preferred `image_base`. The \
         triage tool: 'does this file actually have line info, and does its debug-id match?' \
         answered without paying the parse-into-cache cost."
            .to_string(),
        "Inspect a debug file by BLOB or path without resolving: one row with format, arch, \
         build_id, debug_id, code_id, has_dwarf/has_pdb/has_line_table, symbol_count, file_count, \
         byte_size, image_base."
            .to_string(),
        "module_info, inspect, triage, debug file, ELF, MachO, PE, PDB, dSYM, build_id, debug_id, \
         has line table, symbol count"
            .to_string(),
        "Returns one row:\n\n\
         | column | type | description |\n\
         |---|---|---|\n\
         | `format` | VARCHAR | ELF / MachO / PE / PDB / dSYM / Breakpad. |\n\
         | `arch` | VARCHAR | CPU architecture. |\n\
         | `build_id` | VARCHAR | Raw per-format identifier (hex). |\n\
         | `debug_id` | VARCHAR | Normalized cache key. |\n\
         | `code_id` | VARCHAR | Code-file identifier. |\n\
         | `has_dwarf` / `has_pdb` / `has_line_table` | BOOLEAN | Capability flags. |\n\
         | `symbol_count` / `file_count` / `byte_size` | BIGINT | Counts + size. |\n\
         | `image_base` | UBIGINT | Image's preferred load base. |"
            .to_string(),
    )
}

/// One producer that emits a single `module_info` row (or an empty result if the
/// file is not a recognizable debug file).
struct InfoProducer {
    schema: SchemaRef,
    info: Option<ModuleInfo>,
    done: bool,
}

impl TableProducer for InfoProducer {
    fn next_batch(&mut self, _out: &mut OutputCollector) -> Result<Option<RecordBatch>> {
        if self.done {
            return Ok(None);
        }
        self.done = true;
        let mut format = StringBuilder::new();
        let mut arch = StringBuilder::new();
        let mut build_id = StringBuilder::new();
        let mut debug_id = StringBuilder::new();
        let mut code_id = StringBuilder::new();
        let mut has_dwarf = BooleanBuilder::new();
        let mut has_pdb = BooleanBuilder::new();
        let mut has_line = BooleanBuilder::new();
        let mut symbol_count = Int64Builder::new();
        let mut file_count = Int64Builder::new();
        let mut byte_size = Int64Builder::new();
        let mut image_base = UInt64Builder::new();

        if let Some(info) = &self.info {
            format.append_value(info.format.as_str());
            arch.append_value(&info.arch);
            append_opt(&mut build_id, info.build_id.as_deref());
            append_opt(&mut debug_id, info.debug_id.as_deref());
            append_opt(&mut code_id, info.code_id.as_deref());
            has_dwarf.append_value(info.has_dwarf);
            has_pdb.append_value(info.has_pdb);
            has_line.append_value(info.has_line_table);
            symbol_count.append_value(info.symbol_count);
            file_count.append_value(info.file_count);
            byte_size.append_value(info.byte_size);
            image_base.append_value(info.image_base);
        }

        let cols: Vec<ArrayRef> = vec![
            Arc::new(format.finish()),
            Arc::new(arch.finish()),
            Arc::new(build_id.finish()),
            Arc::new(debug_id.finish()),
            Arc::new(code_id.finish()),
            Arc::new(has_dwarf.finish()),
            Arc::new(has_pdb.finish()),
            Arc::new(has_line.finish()),
            Arc::new(symbol_count.finish()),
            Arc::new(file_count.finish()),
            Arc::new(byte_size.finish()),
            Arc::new(image_base.finish()),
        ];
        Ok(Some(
            RecordBatch::try_new(self.schema.clone(), cols)
                .map_err(|e| RpcError::runtime_error(e.to_string()))?,
        ))
    }
}

fn append_opt(b: &mut StringBuilder, v: Option<&str>) {
    match v {
        Some(s) => b.append_value(s),
        None => b.append_null(),
    }
}

/// `module_info(blob BLOB)`.
pub struct ModuleInfoBlob;

impl TableFunction for ModuleInfoBlob {
    fn name(&self) -> &str {
        "module_info"
    }
    fn metadata(&self) -> FunctionMetadata {
        let (llm, md, kw, cols) = columns_md();
        let mut tags = crate::meta::object_tags("Module Info (BLOB)", &llm, &md, &kw);
        tags.push(("vgi.result_columns_md".into(), cols));
        // Inline-BLOB overload: in practice pass the file's bytes, e.g.
        // `module_info(read_blob('/srv/debug/libssl.so.debug'))`. The runnable
        // example uses a short BLOB literal so it is self-contained; bytes that
        // are not a recognizable debug file yield zero rows (no error).
        tags.push((
            "vgi.executable_examples".into(),
            r#"[{"description":"Triage debug-file bytes passed inline as a BLOB (returns one row with format / debug_id / has_line_table, or zero rows if the bytes are not a recognizable debug file). In practice read a real file: `module_info(read_blob('/srv/debug/libssl.so.debug'))`.","sql":"SELECT * FROM symbols.main.module_info('\\x7fELF'::BLOB)"}]"#
                .into(),
        ));
        FunctionMetadata {
            description: "Inspect a debug file given inline as a BLOB (format, debug-id, line \
                          info) without resolving"
                .into(),
            examples: Vec::new(),
            tags,
            ..Default::default()
        }
    }
    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![ArgSpec::const_arg(
            "blob",
            0,
            "blob",
            "The debug file bytes to inspect (e.g. `read_blob('/path/libfoo.so.debug')`). Parsed \
             header-only — no resolve, no cache insertion.",
        )]
    }
    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse {
            output_schema: schema(),
            opaque_data: Vec::new(),
        })
    }
    fn producer(&self, params: &ProcessParams) -> Result<Box<dyn TableProducer>> {
        let blob = const_blob(&params.arguments, 0)
            .ok_or_else(|| RpcError::value_error("module_info: a BLOB argument is required"))?;
        let info = with_state(|state| state.module_info_blob(blob)).ok();
        Ok(Box::new(InfoProducer {
            schema: params.output_schema.clone(),
            info,
            done: false,
        }))
    }
}

/// `module_info(path VARCHAR)`.
pub struct ModuleInfoPath;

impl TableFunction for ModuleInfoPath {
    fn name(&self) -> &str {
        "module_info"
    }
    fn metadata(&self) -> FunctionMetadata {
        let (llm, md, kw, cols) = columns_md();
        let mut tags = crate::meta::object_tags("Module Info (path)", &llm, &md, &kw);
        tags.push(("vgi.result_columns_md".into(), cols));
        // Path overload: returns one row describing the file, or zero rows if the
        // path is missing or not a recognizable debug file (never an error). The
        // example uses a placeholder path so it runs standalone (zero rows).
        tags.push((
            "vgi.executable_examples".into(),
            r#"[{"description":"Triage a debug file by filesystem path (one row with format / debug_id / has_line_table, or zero rows if the path is missing or not a debug file). Point it at a real symbol file on the worker host, e.g. '/srv/debug/libssl.so.debug'.","sql":"SELECT * FROM symbols.main.module_info('/srv/debug/libssl.so.debug')"}]"#
                .into(),
        ));
        FunctionMetadata {
            description: "Inspect a debug file by path (format, debug-id, line info) without \
                          resolving"
                .into(),
            examples: Vec::new(),
            tags,
            ..Default::default()
        }
    }
    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![ArgSpec::const_arg(
            "path",
            0,
            "varchar",
            "Filesystem path of the debug file to inspect. Parsed header-only — no resolve, no \
             cache insertion.",
        )]
    }
    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse {
            output_schema: schema(),
            opaque_data: Vec::new(),
        })
    }
    fn producer(&self, params: &ProcessParams) -> Result<Box<dyn TableProducer>> {
        let path = params
            .arguments
            .const_str(0)
            .ok_or_else(|| RpcError::value_error("module_info: a path argument is required"))?;
        let info = with_state(|state| state.module_info_path(&path)).ok();
        Ok(Box::new(InfoProducer {
            schema: params.output_schema.clone(),
            info,
            done: false,
        }))
    }
}
